//! The sandbox runtime: the shared Rhai engine, the per-owner AST cache, the
//! single-threaded session bridge, the budget-bounded cascade driver, and
//! the `mush` host API.
//!
//! # The bridge (why this is sound without `unsafe`)
//!
//! Rhai host functions are `'static` closures: they cannot borrow the
//! `&mut World` / `&mut Outbox` a command handler holds. The classic
//! solution is a thread-local raw pointer (`Cell<*mut Ctx>`), which needs
//! `unsafe`. This crate takes the **safe alternative the assignment asked
//! for**: for the duration of one cascade the driver *moves* the real
//! `World` and `Outbox` out of the caller's `&mut` (via [`std::mem::take`],
//! leaving a cheap `World::default()` / `Outbox::default()` placeholder
//! behind) into a thread-local [`ScriptSession`] owned handle. Host
//! functions reach them through `RefCell` borrows — fully checked, no raw
//! pointers, no `unsafe`. A [`SessionGuard`] moves them back on scope exit,
//! **including on panic**, so the caller's world is always restored.
//!
//! Safety rests on three invariants, all guaranteed by construction:
//!
//! 1. **Single-threaded, synchronous.** The whole cascade runs inside one
//!    `Engine::step` on the world task with no `.await`, so the thread-local
//!    never crosses a thread boundary while set.
//! 2. **No borrow held across an eval.** [`run_one`] clones the `Arc<Engine>`
//!    and `Arc<AST>` out of the session, *drops* the `RefCell` borrow, then
//!    evaluates; host functions each take a fresh transient borrow and drop
//!    it before returning. No two `&mut ScriptSession` are ever live at once
//!    (no aliasing).
//! 3. **No nested cascades.** Host functions never re-enter script execution
//!    or fire core world events — they push follow-up work onto the session
//!    queue, which the driver drains iteratively. So [`drive`] is never
//!    re-entrant and the session is `Some` only for one flat cascade.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rhai::plugin::*;
use rhai::{AST, Array, Dynamic, Engine as RhaiEngine, EvalAltResult, Position, Scope};
use std::cell::{Cell, RefCell};

use mush_core::module::{HookPoint, HookReg, perceives_via};
use mush_core::{
    Attr, AttrFlags, AttrValue, Deferred, Flags, ObjId, ObjKind, Outbox, Powers, Registry, Role,
    ScheduledKind, World, match_object, perm,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use smol_str::SmolStr;

// ----------------------------------------------------------------------
// Config
// ----------------------------------------------------------------------

/// Sandbox budgets (all configurable; defaults are the assignment's).
#[derive(Clone, Copy, Debug)]
pub struct ScriptConfig {
    /// Per-invocation operation cap enforced via `on_progress` (CPU budget).
    pub max_operations: u64,
    /// Per-invocation wall-clock backstop, milliseconds.
    pub wall_clock_ms: u64,
    /// Aggregate wall-clock backstop for one whole cascade (every invocation
    /// an originating event triggers, combined), milliseconds. Bounds the
    /// total synchronous world-task time a single event can consume, so a
    /// room full of honest-but-slow reflexes cannot chain into a multi-second
    /// stall (the per-invocation cap alone bounds only one script).
    pub cascade_wall_clock_ms: u64,
    /// Maximum Rhai call nesting (deep-recursion guard).
    pub max_call_levels: usize,
    /// Maximum length of any single string value (memory cap).
    pub max_string_size: usize,
    /// Maximum length of any single array (memory cap).
    pub max_array_size: usize,
    /// Maximum size of any single object map (memory cap).
    pub max_map_size: usize,
    /// Maximum expression nesting depth (parse-time guard).
    pub max_expr_depth: usize,
    /// Total script invocations allowed per originating world event.
    pub max_total_invocations: u32,
    /// Maximum cascade depth (A emits → B hears → …) per originating event.
    pub max_depth: u32,
    /// Consecutive budget overruns before an object is auto-`HALTED`.
    pub strikes_to_halt: u32,
}

impl Default for ScriptConfig {
    fn default() -> ScriptConfig {
        ScriptConfig {
            max_operations: 500_000,
            wall_clock_ms: 50,
            cascade_wall_clock_ms: 250,
            max_call_levels: 32,
            max_string_size: 64 * 1024,
            max_array_size: 4_096,
            max_map_size: 4_096,
            max_expr_depth: 64,
            max_total_invocations: 64,
            max_depth: 8,
            strikes_to_halt: 3,
        }
    }
}

// ----------------------------------------------------------------------
// Compiled cache + module state
// ----------------------------------------------------------------------

/// One object's successfully-compiled `script:*` attributes, keyed by folded
/// attr name (`"script:command:greet"`). A script that fails to compile is
/// **absent** here (its owner was notified at compile time) — absent means
/// not runnable, exactly like no script at all.
#[derive(Clone, Default)]
pub(crate) struct ObjScripts {
    pub(crate) scripts: HashMap<SmolStr, Arc<AST>>,
}

/// The mush-script module-state slot living on the [`World`]. In-RAM only —
/// scripts are attributes (core-persisted); ASTs and strikes are rebuilt on
/// boot.
#[derive(Clone)]
pub struct ScriptState {
    /// The single shared, sandbox-configured Rhai engine.
    engine: Arc<RhaiEngine>,
    /// Sandbox budgets.
    cfg: ScriptConfig,
    /// Per-object compiled scripts.
    pub(crate) cache: HashMap<ObjId, ObjScripts>,
    /// Consecutive budget-overrun strikes per object.
    pub(crate) strikes: HashMap<ObjId, u32>,
    /// Module-owned attr namespaces (snapshot at boot); `mush::set_attr`
    /// refuses to write these unless the owner is a Wizard.
    pub(crate) owned_namespaces: Vec<SmolStr>,
    /// `MoveAttempt` hooks (snapshot at boot) consulted by `mush::move`.
    pub(crate) move_hooks: Vec<HookReg>,
    /// `Perceive` hooks (snapshot at boot) consulted by [`owner_can_see`], so
    /// scripts cannot see an object the look/output layer conceals (M11).
    pub(crate) perceive_hooks: Vec<HookReg>,
    /// Dice PRNG state (xorshift64; deterministic across a boot).
    rng: u64,
}

impl ScriptState {
    /// A fresh state slot wrapping a built engine.
    pub(crate) fn new(cfg: ScriptConfig) -> ScriptState {
        ScriptState {
            engine: Arc::new(build_engine(&cfg)),
            cfg,
            cache: HashMap::new(),
            strikes: HashMap::new(),
            owned_namespaces: Vec::new(),
            move_hooks: Vec::new(),
            perceive_hooks: Vec::new(),
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }
}

// ----------------------------------------------------------------------
// Cascade primitives
// ----------------------------------------------------------------------

/// The identity a running script sees (`mush::me`/`owner`/`caller`).
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    /// The object the script lives on (`mush::me`).
    self_obj: ObjId,
    /// The authority every API call is checked against (`mush::owner`).
    owner: ObjId,
    /// Who triggered the script (`mush::caller`).
    caller: ObjId,
    /// This trigger's cascade depth (follow-up work is depth + 1).
    depth: u32,
}

/// The event payload a trigger injects into the script scope.
#[derive(Clone)]
pub(crate) enum Payload {
    /// `script:command:*` — `cmd` verb and `arg` remainder.
    Command {
        /// The matched verb.
        cmd: String,
        /// The argument text after the verb.
        arg: String,
    },
    /// `script:hear:*` — the spoken `text` and its `speaker`.
    Hear {
        /// The spoken text.
        text: String,
        /// The speaker.
        speaker: ObjId,
    },
    /// `script:enter` — the arriving `mover`.
    Enter {
        /// The arriving object.
        mover: ObjId,
    },
    /// `script:leave` — the departing `mover`.
    Leave {
        /// The departing object.
        mover: ObjId,
    },
    /// `script:use` — the `user`.
    Use {
        /// Who used the object.
        user: ObjId,
    },
    /// `script:timer:*` — no extra scope vars.
    Timer,
}

/// One queued script invocation.
#[derive(Clone)]
pub(crate) struct Trigger {
    /// The object whose script runs.
    pub(crate) self_obj: ObjId,
    /// Who triggered it.
    pub(crate) caller: ObjId,
    /// The folded script attribute name.
    pub(crate) script_key: SmolStr,
    /// Cascade depth (0 for the originating event).
    pub(crate) depth: u32,
    /// The event payload.
    pub(crate) payload: Payload,
}

/// The owned handle that holds the world for one cascade.
struct ScriptSession {
    world: World,
    out: Outbox,
    queue: VecDeque<Trigger>,
    total: u32,
    current: Option<Frame>,
}

thread_local! {
    /// The active cascade's owned handle (see the module docs). `Some` only
    /// while [`drive`] runs; host functions borrow it transiently.
    static SESSION: RefCell<Option<ScriptSession>> = const { RefCell::new(None) };
    /// The currently-executing invocation's budget, read by `on_progress`.
    static BUDGET: Cell<Budget> = const { Cell::new(Budget::NONE) };
}

/// Per-invocation budget snapshot for `on_progress`.
#[derive(Clone, Copy)]
struct Budget {
    op_limit: u64,
    deadline: Option<Instant>,
}

impl Budget {
    const NONE: Budget = Budget {
        op_limit: u64::MAX,
        deadline: None,
    };
}

/// Restores the caller's world/outbox on scope exit (incl. panic).
struct SessionGuard<'a> {
    world: &'a mut World,
    out: &'a mut Outbox,
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        if let Some(sess) = SESSION.with(|s| s.borrow_mut().take()) {
            *self.world = sess.world;
            *self.out = sess.out;
        }
        BUDGET.with(|b| b.set(Budget::NONE));
    }
}

/// Run a bounded script cascade: move `world`/`out` into a session, drain
/// `initial` (and everything the scripts enqueue) under the global cascade
/// caps, then restore. A no-op when `initial` is empty.
pub(crate) fn drive(world: &mut World, out: &mut Outbox, initial: VecDeque<Trigger>) {
    if initial.is_empty() {
        return;
    }
    debug_assert!(
        SESSION.with(|s| s.borrow().is_none()),
        "script cascades are never nested"
    );
    let real_world = std::mem::take(world);
    let real_out = std::mem::take(out);
    SESSION.with(|s| {
        *s.borrow_mut() = Some(ScriptSession {
            world: real_world,
            out: real_out,
            queue: initial,
            total: 0,
            current: None,
        });
    });
    // From here the real world lives in the thread-local; `world`/`out` are
    // placeholders moved into the guard, which restores them on drop.
    let _guard = SessionGuard { world, out };
    run_loop();
}

/// The cascade drain loop: pop, budget-check, run.
fn run_loop() {
    let Some(cfg) = with_state(|st| st.cfg) else {
        return;
    };
    // One deadline shared across every invocation this cascade runs, so the
    // aggregate synchronous time an originating event costs the world task is
    // bounded regardless of how many reflexes it chains (SEV-5): the
    // per-invocation cap alone would allow `max_total_invocations × wall_clock`.
    let cascade_deadline = Instant::now() + Duration::from_millis(cfg.cascade_wall_clock_ms.max(1));
    loop {
        // Pop the next runnable trigger, enforcing depth (drop) and total
        // (stop) caps.
        let next = SESSION.with(|s| {
            let mut b = s.borrow_mut();
            let sess = b.as_mut()?;
            loop {
                let t = sess.queue.pop_front()?;
                if t.depth > cfg.max_depth {
                    continue; // cascade too deep: drop this branch
                }
                if sess.total >= cfg.max_total_invocations {
                    tracing::warn!(
                        total = sess.total,
                        "script cascade hit its invocation cap; remaining triggers dropped"
                    );
                    sess.queue.clear();
                    return None;
                }
                sess.total += 1;
                return Some(t);
            }
        });
        let Some(trigger) = next else { break };
        // Aggregate wall-clock stop: abandon the rest of the cascade once the
        // shared deadline passes (the just-popped trigger still runs, bounded
        // by its own per-invocation cap, so worst case is deadline + one
        // wall_clock_ms).
        if Instant::now() >= cascade_deadline {
            tracing::warn!(
                "script cascade exceeded its aggregate wall-clock budget; remaining triggers dropped"
            );
            SESSION.with(|s| {
                if let Some(sess) = s.borrow_mut().as_mut() {
                    sess.queue.clear();
                }
            });
            break;
        }
        run_one(&trigger, cfg);
    }
}

/// Run one trigger's script, then handle any error (strikes / HALTED).
fn run_one(trigger: &Trigger, cfg: ScriptConfig) {
    // Fetch engine + AST + owner and set the frame; skip HALTED/errored/gone.
    let prepared = SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let sess = b.as_mut()?;
        let obj = sess.world.get(trigger.self_obj)?;
        if obj.flags.contains(Flags::HALTED) || obj.flags.contains(Flags::GOING) {
            return None;
        }
        let owner = obj.owner;
        let st = sess.world.module_state::<ScriptState>()?;
        let ast = Arc::clone(
            st.cache
                .get(&trigger.self_obj)?
                .scripts
                .get(&trigger.script_key)?,
        );
        let engine = Arc::clone(&st.engine);
        sess.current = Some(Frame {
            self_obj: trigger.self_obj,
            owner,
            caller: trigger.caller,
            depth: trigger.depth,
        });
        Some((engine, ast))
    });
    let Some((engine, ast)) = prepared else {
        return;
    };

    let mut scope = Scope::new();
    inject_scope(&mut scope, &trigger.payload);
    set_budget(&cfg);
    // No SESSION borrow is held across this call — host functions reborrow.
    // Defense in depth: budget overruns already surface as a catchable `Err`,
    // but a panic in any host function or in the interpreter would otherwise
    // unwind out of the world task and kill the entire game loop. The
    // session's real world lives in the thread-local (the `SessionGuard`
    // restores it as the cascade unwinds) and no RefCell borrow is held across
    // this call, so catching keeps the loop alive and downgrades a panic to a
    // single failed script.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.run_ast_with_scope(&mut scope, &ast)
    }))
    .unwrap_or_else(|_| Err(err("script aborted (internal panic)")));
    clear_budget();
    SESSION.with(|s| {
        if let Some(sess) = s.borrow_mut().as_mut() {
            sess.current = None;
        }
    });

    match result {
        Err(e) => handle_error(trigger, &e, cfg),
        // A clean run clears the object's accumulated overrun strikes, so
        // `strikes_to_halt` counts *consecutive* overruns — a successful run
        // in between resets the tally, matching the documented behavior and
        // keeping a third party from HALTing a sound script by inflating one
        // run's cost.
        Ok(_) => {
            SESSION.with(|s| {
                if let Some(sess) = s.borrow_mut().as_mut()
                    && let Some(st) = sess.world.module_state_mut::<ScriptState>()
                {
                    st.strikes.remove(&trigger.self_obj);
                }
            });
        }
    }
}

/// Push the trigger's payload into the script scope as constants.
fn inject_scope(scope: &mut Scope<'_>, payload: &Payload) {
    match payload {
        Payload::Command { cmd, arg } => {
            scope.push_constant("cmd", cmd.clone());
            scope.push_constant("arg", arg.clone());
        }
        Payload::Hear { text, speaker } => {
            scope.push_constant("text", text.clone());
            scope.push_constant("speaker", *speaker);
        }
        Payload::Enter { mover } | Payload::Leave { mover } => {
            scope.push_constant("mover", *mover);
        }
        Payload::Use { user } => {
            scope.push_constant("user", *user);
        }
        Payload::Timer => {}
    }
}

/// A budget/resource overrun (accrues a strike), vs. a plain script error.
fn is_budget_overrun(err: &EvalAltResult) -> bool {
    matches!(
        err,
        EvalAltResult::ErrorTerminated(..)          // op / wall-clock abort
            | EvalAltResult::ErrorTooManyOperations(..)  // global op ceiling
            | EvalAltResult::ErrorStackOverflow(..)      // call-level cap
            | EvalAltResult::ErrorDataTooLarge(..) // string/array/map cap
    )
}

/// Notify the owner of a failed script; on a budget overrun, accrue a strike
/// and auto-`HALTED` the object after `strikes_to_halt`.
fn handle_error(trigger: &Trigger, err: &EvalAltResult, cfg: ScriptConfig) {
    let overrun = is_budget_overrun(err);
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return };
        let obj = trigger.self_obj;
        let owner = sess.world.get(obj).map_or(obj, |o| o.owner);
        sess.out.error_on_commit(
            owner,
            format!("Script {} on {} failed: {}", trigger.script_key, obj, err),
        );
        if !overrun {
            return;
        }
        let strikes = {
            let st = sess
                .world
                .module_state_mut::<ScriptState>()
                .expect("script state slot");
            let n = st.strikes.entry(obj).or_insert(0);
            *n += 1;
            *n
        };
        if strikes >= cfg.strikes_to_halt {
            let _ = sess.world.set_flag(obj, Flags::HALTED, true);
            sess.out.error_on_commit(
                owner,
                format!(
                    "{obj} has been HALTED after {strikes} script budget overruns; \
                     fix the script and clear the HALTED flag to re-enable it."
                ),
            );
        }
    });
}

// ----------------------------------------------------------------------
// Budget helpers
// ----------------------------------------------------------------------

fn set_budget(cfg: &ScriptConfig) {
    let deadline = Instant::now() + Duration::from_millis(cfg.wall_clock_ms.max(1));
    BUDGET.with(|b| {
        b.set(Budget {
            op_limit: cfg.max_operations,
            deadline: Some(deadline),
        });
    });
}

fn clear_budget() {
    BUDGET.with(|b| b.set(Budget::NONE));
}

// ----------------------------------------------------------------------
// Session access helpers (used by host functions)
// ----------------------------------------------------------------------

/// Build a catchable Rhai runtime error from a message.
fn err(msg: impl Into<String>) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(
        Dynamic::from(msg.into()),
        Position::NONE,
    ))
}

/// Read the current frame; error if called outside a session.
fn frame<R>(f: impl FnOnce(&Frame) -> R) -> Result<R, Box<EvalAltResult>> {
    SESSION.with(|s| {
        let b = s.borrow();
        let sess = b.as_ref().ok_or_else(|| err("no active script session"))?;
        let fr = sess
            .current
            .as_ref()
            .ok_or_else(|| err("no active script frame"))?;
        Ok(f(fr))
    })
}

/// Mutable session access; error if called outside a session.
fn with_session_mut<R>(
    f: impl FnOnce(&mut ScriptSession) -> Result<R, Box<EvalAltResult>>,
) -> Result<R, Box<EvalAltResult>> {
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let sess = b.as_mut().ok_or_else(|| err("no active script session"))?;
        f(sess)
    })
}

/// Read-only session access; `None` outside a session.
fn with_session<R>(f: impl FnOnce(&ScriptSession) -> R) -> Option<R> {
    SESSION.with(|s| s.borrow().as_ref().map(f))
}

/// Read the (in-world) module state; `None` outside a session or before
/// registration.
fn with_state<R>(f: impl FnOnce(&ScriptState) -> R) -> Option<R> {
    with_session(|sess| sess.world.module_state::<ScriptState>().map(f)).flatten()
}

// ----------------------------------------------------------------------
// Engine construction + the `mush` host API
// ----------------------------------------------------------------------

/// Build the shared, sandboxed engine: budget knobs, no `eval`, no external
/// modules, and only the `mush` API registered.
fn build_engine(cfg: &ScriptConfig) -> RhaiEngine {
    let mut engine = RhaiEngine::new();
    // A high global ceiling; the per-invocation `on_progress` cap trips first.
    engine.set_max_operations(
        cfg.max_operations
            .saturating_mul(50)
            .max(cfg.max_operations),
    );
    engine.set_max_call_levels(cfg.max_call_levels);
    engine.set_max_string_size(cfg.max_string_size);
    engine.set_max_array_size(cfg.max_array_size);
    engine.set_max_map_size(cfg.max_map_size);
    engine.set_max_expr_depths(cfg.max_expr_depth, cfg.max_expr_depth);
    // No filesystem/network module loading, and no `eval`.
    engine.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());
    engine.disable_symbol("eval");

    engine.on_progress(|ops| {
        let bud = BUDGET.with(Cell::get);
        if ops > bud.op_limit {
            return Some(Dynamic::from("operation budget exceeded"));
        }
        // Sample the wall clock cheaply (every 1024 ops).
        if ops & 0x3FF == 0
            && let Some(deadline) = bud.deadline
            && Instant::now() >= deadline
        {
            return Some(Dynamic::from("time budget exceeded"));
        }
        None
    });

    // ObjId as a Rhai custom type.
    engine.register_type_with_name::<ObjId>("Obj");
    engine.register_fn("to_string", |o: ObjId| o.to_string());
    engine.register_fn("to_debug", |o: ObjId| format!("{o:?}"));
    engine.register_fn("==", |a: ObjId, b: ObjId| a == b);
    engine.register_fn("!=", |a: ObjId, b: ObjId| a != b);
    engine.register_fn("dbref", |o: ObjId| i64::from(o.0));

    engine.register_static_module("mush", exported_module!(mush_api).into());
    engine
}

/// The `mush` module — the only capability surface scripts get. Every call
/// is checked against the script **owner's** authority (classic
/// dollar-command semantics), never the triggering player's. There is no
/// `mush::cmd`/`eval`: scripts act only through these typed calls.
#[export_module]
mod mush_api {
    /// The scripted object (MUSH `me`).
    #[rhai_fn(name = "me", return_raw)]
    pub fn me() -> Result<ObjId, Box<EvalAltResult>> {
        super::frame(|f| f.self_obj)
    }

    /// The script owner (whose authority every call runs with).
    #[rhai_fn(name = "owner", return_raw)]
    pub fn owner() -> Result<ObjId, Box<EvalAltResult>> {
        super::frame(|f| f.owner)
    }

    /// Who triggered the script.
    #[rhai_fn(name = "caller", return_raw)]
    pub fn caller() -> Result<ObjId, Box<EvalAltResult>> {
        super::frame(|f| f.caller)
    }

    /// The scripted object's location.
    #[rhai_fn(name = "here", return_raw)]
    pub fn here() -> Result<ObjId, Box<EvalAltResult>> {
        let me = super::frame(|f| f.self_obj)?;
        super::loc_of(me)
    }

    /// An object's location. Errors if it has none — or if the owner cannot
    /// see the object (a `DARK` object it neither controls nor holds
    /// `SEE_HIDDEN` for reports "no location", so `loc` is not a scry tool).
    #[rhai_fn(name = "loc", return_raw)]
    pub fn loc(obj: ObjId) -> Result<ObjId, Box<EvalAltResult>> {
        super::loc_visible(obj)
    }

    /// An object's display name (`"nothing"` if it is gone).
    #[rhai_fn(name = "name")]
    pub fn name(obj: ObjId) -> String {
        super::with_session(|sess| mush_core::output::display_name(&sess.world, obj))
            .unwrap_or_default()
    }

    /// Read a text attribute (`""` if unset, not text, or unreadable).
    #[rhai_fn(name = "get_attr")]
    pub fn get_attr(obj: ObjId, attr: &str) -> String {
        super::get_attr_impl(obj, attr)
    }

    /// Set a text attribute. The owner must control `obj`, and module-owned
    /// namespaces are refused unless the owner is a Wizard.
    #[rhai_fn(name = "set_attr", return_raw)]
    pub fn set_attr(obj: ObjId, attr: &str, val: &str) -> Result<(), Box<EvalAltResult>> {
        super::set_attr_impl(obj, attr, val)
    }

    /// An object's contents as an array of `Obj`, honoring the same
    /// DARK/OPAQUE visibility the look/output layer enforces: the contents of
    /// a `DARK` container (or an `OPAQUE` one seen from outside) are hidden
    /// unless the owner controls it or holds `SEE_HIDDEN`, and individual
    /// hidden members are omitted — so `contents` is not a scry tool.
    #[rhai_fn(name = "contents")]
    pub fn contents(obj: ObjId) -> Array {
        super::contents_impl(obj)
    }

    /// Emit an unattributed narrative line to `target`'s room, and let the
    /// room's listeners (`script:hear:*`) react (bounded by the cascade cap).
    #[rhai_fn(name = "emit")]
    pub fn emit(target: ObjId, text: &str) {
        let _ = super::emit_impl(target, text);
    }

    /// Send a private line to one player/NPC.
    #[rhai_fn(name = "pemit")]
    pub fn pemit(player: ObjId, text: &str) {
        super::with_session_mut(|sess| {
            sess.out.to_actor(
                player,
                OutputEvent::Narrative {
                    kind: NarrativeKind::Emit,
                    actor: None,
                    text: RichText::plain(text),
                },
            );
            Ok(())
        })
        .ok();
    }

    /// Move `obj` to `dest` under the owner's authority (teleport rules +
    /// `MoveAttempt` vetoes); fires `script:leave`/`script:enter`.
    #[rhai_fn(name = "move", return_raw)]
    pub fn move_obj(obj: ObjId, dest: ObjId) -> Result<(), Box<EvalAltResult>> {
        super::move_impl(obj, dest)
    }

    /// Resolve a name to an object from the scripted object's viewpoint.
    /// (Named `matchit`, not `match`: `match` is a Rhai reserved word.)
    #[rhai_fn(name = "matchit", return_raw)]
    pub fn match_obj(query: &str) -> Result<ObjId, Box<EvalAltResult>> {
        let me = super::frame(|f| f.self_obj)?;
        super::with_session(|sess| {
            match_object(&sess.world, me, query).map_err(|e| super::err(e.to_string()))
        })
        .ok_or_else(|| super::err("no active script session"))?
    }

    /// Arm `script:timer:<name>` on the scripted object after `secs` seconds.
    ///
    /// **Not durable (M5):** the timer lives only in the in-RAM
    /// [`Scheduler`](mush_core) and is *not* persisted, so a pending timer is
    /// dropped on a server restart (the due-time sorted-set scheduler
    /// persistence in the PLAN is a later store feature). Scripts must treat
    /// scheduled timers as best-effort, not guaranteed to survive a reboot.
    #[rhai_fn(name = "schedule", return_raw)]
    pub fn schedule(secs: i64, timer: &str) -> Result<(), Box<EvalAltResult>> {
        super::schedule_impl(secs, timer)
    }

    /// Roll a simple dice expression (`"2d6"`, `"d20+3"`, `"4"`).
    #[rhai_fn(name = "dice", return_raw)]
    pub fn dice(expr: &str) -> Result<i64, Box<EvalAltResult>> {
        super::dice_impl(expr)
    }

    /// Whether `obj` has a core flag (by name) or a module marker flag.
    /// Returns `false` for an object the owner cannot see (a `DARK` object it
    /// neither controls nor holds `SEE_HIDDEN` for), so flag probes cannot be
    /// used to detect concealed objects.
    #[rhai_fn(name = "has_flag")]
    pub fn has_flag(obj: ObjId, flag: &str) -> bool {
        super::has_flag_impl(obj, flag)
    }
}

// ----------------------------------------------------------------------
// Host-function implementations (kept out of the export module so helpers
// stay non-exported)
// ----------------------------------------------------------------------

fn loc_of(obj: ObjId) -> Result<ObjId, Box<EvalAltResult>> {
    with_session(|sess| sess.world.get(obj).and_then(|o| o.location))
        .flatten()
        .ok_or_else(|| err(format!("{obj} has no location")))
}

/// Whether the script **owner** may perceive `obj` at all: `false` for a
/// `GOING` husk, or a `DARK` object the owner neither controls nor holds
/// `SEE_HIDDEN` for. Mirrors the per-object filter the look/output layer
/// applies (`output::room_view`), so scripts cannot see what a look cannot.
///
/// Also honors the M11 per-observer projection gate: the `Perceive` hooks
/// snapshotted at boot ([`ScriptState::perceive_hooks`]) are consulted
/// through the one shared [`perceives_via`] predicate, so a subject the
/// output layer conceals from `owner` is invisible to scripts too. A free
/// pass (no snapshotted hook) leaves script visibility exactly as before.
fn owner_can_see(w: &World, owner: ObjId, obj: ObjId) -> bool {
    let Some(o) = w.get(obj) else { return false };
    if o.flags.contains(Flags::GOING) {
        return false;
    }
    if o.flags.contains(Flags::DARK)
        && !perm::has_power(w, owner, Powers::SEE_HIDDEN)
        && !perm::controls(w, owner, obj)
    {
        return false;
    }
    if let Some(st) = w.module_state::<ScriptState>()
        && !perceives_via(&st.perceive_hooks, w, owner, obj)
    {
        return false;
    }
    true
}

/// `loc(obj)` with visibility gating: a hidden object reports "no location",
/// the same error as a genuinely location-less object, so existence is not
/// leaked. (`here` reads the scripted object's own location and stays ungated
/// — an owner always sees its own object.)
fn loc_visible(obj: ObjId) -> Result<ObjId, Box<EvalAltResult>> {
    let visible = with_session(|sess| {
        let owner = sess.current.map_or(obj, |f| f.owner);
        owner_can_see(&sess.world, owner, obj)
    });
    match visible {
        Some(true) => loc_of(obj),
        _ => Err(err(format!("{obj} has no location"))),
    }
}

/// `contents(obj)` with the look layer's DARK/OPAQUE gating.
fn contents_impl(container: ObjId) -> Array {
    with_session(|sess| {
        let w = &sess.world;
        let (owner, viewpoint) = sess
            .current
            .map_or((container, container), |f| (f.owner, f.self_obj));
        let Some(c) = w.get(container) else {
            return Array::new();
        };
        let privileged =
            perm::has_power(w, owner, Powers::SEE_HIDDEN) || perm::controls(w, owner, container);
        // OPAQUE hides contents only from a viewpoint *outside* the container;
        // the scripted object's own location is the physical viewpoint.
        let inside = w.get(viewpoint).and_then(|o| o.location) == Some(container);
        let hide_all = !privileged
            && (c.flags.contains(Flags::DARK) || (c.flags.contains(Flags::OPAQUE) && !inside));
        if hide_all {
            return Array::new();
        }
        w.contents_of(container)
            .iter()
            .copied()
            .filter(|&m| owner_can_see(w, owner, m))
            .map(Dynamic::from)
            .collect::<Array>()
    })
    .unwrap_or_default()
}

/// `has_flag(obj, flag)` with visibility gating (see [`owner_can_see`]).
fn has_flag_impl(obj: ObjId, flag: &str) -> bool {
    with_session(|sess| {
        let owner = sess.current.map_or(obj, |f| f.owner);
        if !owner_can_see(&sess.world, owner, obj) {
            return false;
        }
        let Some(o) = sess.world.get(obj) else {
            return false;
        };
        if let Some(f) = Flags::from_name(&flag.to_uppercase()) {
            o.flags.contains(f)
        } else {
            sess.world.has_marker(obj, flag)
        }
    })
    .unwrap_or(false)
}

fn get_attr_impl(obj: ObjId, attr: &str) -> String {
    with_session(|sess| {
        let owner = sess.current.map_or(obj, |f| f.owner);
        let Some(a) = sess.world.attr(obj, attr) else {
            return String::new();
        };
        // Restricted attrs — staff-only (`WIZARD_ONLY`), self-only, or
        // deliberately `HIDDEN` — require the owner to be able to examine
        // `obj`; `HIDDEN` matches canonical `examine`, which withholds hidden
        // attrs even from a controller without `/all`.
        let restricted = a
            .flags
            .intersects(AttrFlags::WIZARD_ONLY | AttrFlags::SELF_ONLY | AttrFlags::HIDDEN);
        if restricted && !perm::can_examine(&sess.world, owner, obj) {
            return String::new();
        }
        a.as_text().unwrap_or_default().to_string()
    })
    .unwrap_or_default()
}

fn set_attr_impl(obj: ObjId, attr: &str, val: &str) -> Result<(), Box<EvalAltResult>> {
    with_session_mut(|sess| {
        let owner = sess
            .current
            .map(|f| f.owner)
            .ok_or_else(|| err("no frame"))?;
        let attr = attr.trim();
        if attr.is_empty() {
            return Err(err("attribute name is empty"));
        }
        if !perm::can_modify(&sess.world, owner, obj) {
            return Err(err("Permission denied."));
        }
        if let Some((ns, _)) = attr.split_once(':') {
            let owned = sess.world.module_state::<ScriptState>().is_some_and(|st| {
                st.owned_namespaces
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(ns))
            });
            if owned && !perm::has_role(&sess.world, owner, Role::Wizard) {
                return Err(err(format!(
                    "The {ns}: attribute namespace is module-owned."
                )));
            }
        }
        // Replicate the two overwrite guards `@set` enforces (building.rs), so
        // a script cannot do through `mush::set_attr` what its owner could not
        // do by hand: a `WIZARD_ONLY` attr resists a non-Wizard, and a `LOCKED`
        // attr resists anyone but its setter (or staff).
        let existing = sess
            .world
            .get(obj)
            .and_then(|o| o.attrs.get(attr))
            .map(|a| (a.flags, a.owner));
        let existing_flags = existing.map(|(f, _)| f);
        if existing_flags.is_some_and(|f| f.contains(AttrFlags::WIZARD_ONLY))
            && !perm::has_role(&sess.world, owner, Role::Wizard)
        {
            return Err(err("Permission denied."));
        }
        if existing.is_some_and(|(f, setter)| {
            f.contains(AttrFlags::LOCKED)
                && setter != owner
                && !perm::has_role(&sess.world, owner, Role::Wizard)
        }) {
            return Err(err("That attribute is locked."));
        }
        // Carry the existing attr flags forward on rewrite (HIDDEN / LOCKED /
        // WIZARD_ONLY / NO_INHERIT) rather than silently resetting them to
        // empty — exactly as `@set` does.
        sess.world
            .set_attr(
                obj,
                attr,
                Attr {
                    value: AttrValue::Text(val.to_string()),
                    owner,
                    flags: existing_flags.unwrap_or_default(),
                },
            )
            .map_err(|e| err(e.to_string()))?;
        // Host functions never emit core `AttrSet` events (the re-entrancy
        // invariant), so `on_attr_set` does not recompile when a script authors
        // another script. Recompile inline here — safe and non-reentrant, we
        // hold `&mut world` directly — so a script written by a script is live
        // immediately, not only after the next `@set` or a reboot.
        if attr.to_ascii_lowercase().starts_with("script:") {
            recompile_object(&mut sess.world, &mut sess.out, obj);
        }
        Ok(())
    })
}

fn emit_impl(target: ObjId, text: &str) -> Result<(), Box<EvalAltResult>> {
    with_session_mut(|sess| {
        let (self_obj, depth) = sess
            .current
            .map(|f| (f.self_obj, f.depth))
            .ok_or_else(|| err("no frame"))?;
        let Some(room) = room_of(&sess.world, target) else {
            return Ok(());
        };
        sess.out.to_room(
            room,
            Vec::new(),
            OutputEvent::Narrative {
                kind: NarrativeKind::Emit,
                actor: None,
                text: RichText::plain(text),
            },
        );
        let hears = collect_hear_triggers(&sess.world, room, self_obj, text, depth + 1);
        sess.queue.extend(hears);
        Ok(())
    })
}

fn move_impl(obj: ObjId, dest: ObjId) -> Result<(), Box<EvalAltResult>> {
    with_session_mut(|sess| {
        let (owner, depth) = sess
            .current
            .map(|f| (f.owner, f.depth))
            .ok_or_else(|| err("no frame"))?;
        if !perm::can_teleport(&sess.world, owner, obj, dest) {
            return Err(err("Permission denied."));
        }
        let hooks = sess
            .world
            .module_state::<ScriptState>()
            .map(|st| st.move_hooks.clone())
            .unwrap_or_default();
        for h in &hooks {
            let hook_ctx = mush_core::HookReadContext::without_typed_state(&sess.world, &h.module);
            if let Err(reason) = h.hook.move_attempt(&hook_ctx, obj, None, dest) {
                return Err(err(reason.to_plain_text()));
            }
        }
        let from = sess.world.get(obj).and_then(|o| o.location);
        sess.world
            .move_to(obj, dest)
            .map_err(|e| err(e.to_string()))?;
        if let Some(from) = from {
            let mut leave = Vec::new();
            collect_move_triggers(&mut leave, &sess.world, from, obj, false, depth + 1);
            sess.queue.extend(leave);
        }
        let mut enter = Vec::new();
        collect_move_triggers(&mut enter, &sess.world, dest, obj, true, depth + 1);
        sess.queue.extend(enter);
        Ok(())
    })
}

fn schedule_impl(secs: i64, timer: &str) -> Result<(), Box<EvalAltResult>> {
    with_session_mut(|sess| {
        if secs < 0 {
            return Err(err("delay must be non-negative"));
        }
        let name = timer.trim();
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Err(err("timer name must be one non-empty word"));
        }
        let self_obj = sess
            .current
            .map(|f| f.self_obj)
            .ok_or_else(|| err("no frame"))?;
        sess.out.defer(Deferred::ScheduleIn {
            delay_ms: secs.saturating_mul(1000),
            kind: ScheduledKind::WaitCmd {
                actor: self_obj,
                line: format!("@script/timer {name}"),
            },
        });
        Ok(())
    })
}

fn dice_impl(expr: &str) -> Result<i64, Box<EvalAltResult>> {
    with_session_mut(|sess| {
        let st = sess
            .world
            .module_state_mut::<ScriptState>()
            .ok_or_else(|| err("no script state"))?;
        let mut seed = st.rng;
        let result = roll(expr, &mut seed).ok_or_else(|| err("bad dice expression"))?;
        st.rng = seed;
        Ok(result)
    })
}

/// The room an emit/hear targets: the object itself if it is a room,
/// otherwise the object's location.
fn room_of(w: &World, target: ObjId) -> Option<ObjId> {
    let o = w.get(target)?;
    if o.kind == ObjKind::Room {
        Some(target)
    } else {
        o.location
    }
}

// ----------------------------------------------------------------------
// Dice roller (tiny xorshift; mush-dice lands M6)
// ----------------------------------------------------------------------

fn xorshift64(s: &mut u64) -> u64 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *s = x;
    x
}

/// Parse and roll `NdS`, `NdS+M`, `NdS-M`, `dS` (N=1), or a bare integer.
/// Returns `None` on a malformed expression. Dice/side counts are bounded.
fn roll(expr: &str, seed: &mut u64) -> Option<i64> {
    let expr = expr.trim().to_lowercase();
    // Dice expressions are ASCII by construction. Rejecting non-ASCII up front
    // keeps the byte-index slicing below panic-free: a multi-byte first char
    // makes byte index 1 a non-char-boundary, and `expr[1..]` would panic
    // *unconditionally in every build profile* — a single reachable panic here
    // takes down the whole world task (there is no catch on the core path).
    if expr.is_empty() || !expr.is_ascii() {
        return None;
    }
    // Split off a trailing +/-M modifier (not the leading sign of a bare int).
    let (dice_part, modifier) = match expr[1..].find(['+', '-']).map(|i| i + 1) {
        Some(pos) => {
            let m: i64 = expr[pos..].parse().ok()?;
            (&expr[..pos], m)
        }
        None => (expr.as_str(), 0),
    };
    let dice_part = dice_part.trim();
    if let Some((n_str, s_str)) = dice_part.split_once('d') {
        let n: u32 = if n_str.is_empty() {
            1
        } else {
            n_str.parse().ok()?
        };
        let sides: u32 = s_str.parse().ok()?;
        if n == 0 || n > 1_000 || sides == 0 || sides > 1_000 {
            return None;
        }
        let mut total: i64 = 0;
        for _ in 0..n {
            total += 1 + i64::from((xorshift64(seed) % u64::from(sides)) as u32);
        }
        // `checked_add` so an extreme modifier ("1d1+9223372036854775807")
        // fails as a bad expression instead of overflow-panicking in a debug
        // build.
        total.checked_add(modifier)
    } else {
        // A bare integer.
        dice_part.parse::<i64>().ok()?.checked_add(modifier)
    }
}

// ----------------------------------------------------------------------
// Trigger collection + compilation (used by the module's observers)
// ----------------------------------------------------------------------

/// Whether `obj` has a runnable (compiled, non-errored, not HALTED/GONE)
/// script under the folded `key`.
pub(crate) fn has_runnable(w: &World, obj: ObjId, key: &str) -> bool {
    if w.get(obj)
        .is_none_or(|o| o.flags.contains(Flags::HALTED) || o.flags.contains(Flags::GOING))
    {
        return false;
    }
    w.module_state::<ScriptState>()
        .and_then(|st| st.cache.get(&obj))
        .is_some_and(|os| os.scripts.contains_key(key))
}

/// The objects an object-command / hear search scans: a room and its
/// contents (a room is checked itself, then everything inside it).
fn room_scope(w: &World, room: ObjId) -> Vec<ObjId> {
    let mut v = vec![room];
    v.extend(w.contents_of(room).iter().copied());
    v
}

/// Hearers in `room` (room + contents, minus the speaker) with a runnable
/// `script:hear[:*]`.
pub(crate) fn collect_hear_triggers(
    w: &World,
    room: ObjId,
    speaker: ObjId,
    text: &str,
    depth: u32,
) -> Vec<Trigger> {
    let mut out = Vec::new();
    for cand in room_scope(w, room) {
        if cand == speaker {
            continue;
        }
        for key in runnable_keys(w, cand, |k| {
            k == "script:hear" || k.starts_with("script:hear:")
        }) {
            out.push(Trigger {
                self_obj: cand,
                caller: speaker,
                script_key: key,
                depth,
                payload: Payload::Hear {
                    text: text.to_string(),
                    speaker,
                },
            });
        }
    }
    out
}

/// Enter/leave reactors in `room` (room + contents, minus the mover).
pub(crate) fn collect_move_triggers(
    out: &mut Vec<Trigger>,
    w: &World,
    room: ObjId,
    mover: ObjId,
    entering: bool,
    depth: u32,
) {
    let key = if entering {
        "script:enter"
    } else {
        "script:leave"
    };
    for cand in room_scope(w, room) {
        if cand == mover {
            continue;
        }
        if has_runnable(w, cand, key) {
            out.push(Trigger {
                self_obj: cand,
                caller: mover,
                script_key: SmolStr::new(key),
                depth,
                payload: if entering {
                    Payload::Enter { mover }
                } else {
                    Payload::Leave { mover }
                },
            });
        }
    }
}

/// Objects answering `verb` with a runnable `script:command:<verb>` in the
/// classic scope: the actor, their inventory, their room, and its contents.
pub(crate) fn collect_command_triggers(
    w: &World,
    actor: ObjId,
    verb: &str,
    arg: &str,
) -> Vec<Trigger> {
    let key = SmolStr::new(format!("script:command:{}", verb.to_lowercase()));
    let mut scope = vec![actor];
    scope.extend(w.contents_of(actor).iter().copied());
    if let Some(loc) = w.get(actor).and_then(|o| o.location) {
        scope.push(loc);
        scope.extend(w.contents_of(loc).iter().copied());
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for cand in scope {
        if !seen.insert(cand) {
            continue;
        }
        if has_runnable(w, cand, &key) {
            out.push(Trigger {
                self_obj: cand,
                caller: actor,
                script_key: key.clone(),
                depth: 0,
                payload: Payload::Command {
                    cmd: verb.to_string(),
                    arg: arg.to_string(),
                },
            });
        }
    }
    out
}

/// The runnable script keys on `obj` matching `pred`.
fn runnable_keys(w: &World, obj: ObjId, pred: impl Fn(&str) -> bool) -> Vec<SmolStr> {
    if w.get(obj)
        .is_none_or(|o| o.flags.contains(Flags::HALTED) || o.flags.contains(Flags::GOING))
    {
        return Vec::new();
    }
    w.module_state::<ScriptState>()
        .and_then(|st| st.cache.get(&obj))
        .map(|os| os.scripts.keys().filter(|k| pred(k)).cloned().collect())
        .unwrap_or_default()
}

/// (Re)compile every `script:*` attribute on `obj`, cache the results, clear
/// its strikes, and notify the owner of any compile errors. Removes the
/// cache entry when the object has no scripts.
pub(crate) fn recompile_object(w: &mut World, out: &mut Outbox, obj: ObjId) {
    let Some(engine) = w
        .module_state::<ScriptState>()
        .map(|st| Arc::clone(&st.engine))
    else {
        return;
    };
    let Some(o) = w.get(obj) else {
        return;
    };
    let owner = o.owner;
    let sources: Vec<(SmolStr, Option<String>)> = o
        .attrs
        .iter_namespace("script")
        .map(|(k, a)| (SmolStr::new(k.folded()), a.as_text().map(str::to_string)))
        .collect();

    if sources.is_empty() {
        if let Some(st) = w.module_state_mut::<ScriptState>() {
            st.cache.remove(&obj);
            st.strikes.remove(&obj);
        }
        return;
    }

    let mut compiled: HashMap<SmolStr, Arc<AST>> = HashMap::new();
    let mut errors: Vec<(SmolStr, String)> = Vec::new();
    for (key, text) in sources {
        match text {
            None => errors.push((key, "script attribute must be text".to_string())),
            Some(src) => match engine.compile(&src) {
                Ok(ast) => {
                    compiled.insert(key, Arc::new(ast));
                }
                Err(e) => errors.push((key, e.to_string())),
            },
        }
    }

    if let Some(st) = w.module_state_mut::<ScriptState>() {
        st.cache.insert(obj, ObjScripts { scripts: compiled });
        st.strikes.remove(&obj);
    }
    for (key, msg) in errors {
        out.error_on_commit(
            owner,
            format!("Script {key} on {obj} failed to compile: {msg}"),
        );
    }
}

/// Snapshot the registry data the off-loop session needs (module-owned attr
/// namespaces + `MoveAttempt`/`Perceive` hooks) into the state slot at boot.
pub(crate) fn snapshot_registry(w: &mut World, reg: &Registry) {
    let namespaces = reg.module_attr_namespaces();
    let move_hooks = reg.hooks(HookPoint::MoveAttempt).to_vec();
    let perceive_hooks = reg.hooks(HookPoint::Perceive).to_vec();
    if let Some(st) = w.module_state_mut::<ScriptState>() {
        st.owned_namespaces = namespaces;
        st.move_hooks = move_hooks;
        st.perceive_hooks = perceive_hooks;
    }
}
