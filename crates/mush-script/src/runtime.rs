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

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt::{self, Write as _};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rhai::packages::{
    ArithmeticPackage, BasicArrayPackage, BasicIteratorPackage, BasicMapPackage,
    BasicStringPackage, LanguageCorePackage, LogicPackage, Package,
};
use rhai::plugin::*;
use rhai::{
    AST, Array, Dynamic, Engine as RhaiEngine, EvalAltResult, FLOAT, FuncRegistration, INT,
    ImmutableString, Position, Scope,
};
use std::cell::{Cell, RefCell};

use crate::guards;

use mush_core::commands::is_ignoring;
use mush_core::lock::{LockType, MAX_LOCK_DEPTH, eval_lock};
use mush_core::matchit::{MatchErr, is_nearby, match_object_visible};
use mush_core::module::{HookPoint, HookReg, perceives_via};
use mush_core::{
    Attr, AttrFlags, AttrValue, Deferred, Flags, Obj, ObjId, ObjKind, Outbox, Powers, Registry,
    Role, ScheduledKind, World, match_object, obj_ref, perm,
};
use mush_core::{DetachedTypedState, TypedStateLens};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use smol_str::SmolStr;

// ----------------------------------------------------------------------
// Config
// ----------------------------------------------------------------------

/// Minimum delay (seconds) accepted by `mush::schedule` (HIGH-18: prevents
/// 0-second self-rearming timer spin loops).
pub const MIN_TIMER_DELAY_SECS: i64 = 1;

/// Maximum delay (seconds) accepted by `mush::schedule` (matches `@wait`).
const MAX_TIMER_DELAY_SECS: i64 = 365 * 24 * 60 * 60;

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
    /// Maximum `script:*` attributes allowed on a single object (HIGH-18).
    pub max_scripts_per_object: usize,
    /// Maximum `emit`/`pemit` events one cascade may append to `sess.out` (HIGH-18).
    pub max_outbox_events: usize,
    /// Maximum combined UTF-8 bytes of `emit`/`pemit` text one cascade may
    /// append to `sess.out` (HIGH-18).
    pub max_outbox_bytes: usize,
    /// Maximum pending follow-up triggers held in `sess.queue` at once (HIGH-18).
    pub max_queue_len: usize,
    /// Maximum `mush::schedule` timers one cascade may enqueue (HIGH-18).
    pub max_schedules_per_cascade: usize,
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
            max_scripts_per_object: 64,
            max_outbox_events: 64,
            max_outbox_bytes: 64 * 1024,
            max_queue_len: 64,
            max_schedules_per_cascade: 16,
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
    /// Total `engine.compile` invocations performed (telemetry & coalescing tests).
    compile_count: u64,
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
            compile_count: 0,
        }
    }

    /// Total number of Rhai script compilations performed since boot.
    #[must_use]
    pub fn compile_count(&self) -> u64 {
        self.compile_count
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
        /// The matched verb (shared zero-copy across listeners).
        cmd: Arc<str>,
        /// The argument text after the verb (shared zero-copy across listeners).
        arg: Arc<str>,
    },
    /// `script:hear:*` — the spoken `text` and its `speaker`.
    Hear {
        /// The spoken text (shared zero-copy across listeners).
        text: Arc<str>,
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
    /// Number of `emit`/`pemit` events pushed by scripts during this cascade.
    out_events: usize,
    /// Combined UTF-8 bytes of `emit`/`pemit` text pushed during this cascade.
    out_bytes: usize,
    /// Number of `mush::schedule` timers deferred during this cascade.
    scheduled: usize,
    /// Coalesced `script:*` attribute edits pending compilation at the end of
    /// the current invocation (HIGH-18: avoids O(K²) recompilation).
    dirty_scripts: BTreeMap<ObjId, BTreeSet<SmolStr>>,
}

thread_local! {
    /// The active cascade's owned handle (see the module docs). `Some` only
    /// while [`drive`] runs; host functions borrow it transiently.
    static SESSION: RefCell<Option<ScriptSession>> = const { RefCell::new(None) };
    /// The currently-executing invocation's monotonic budget, read by `on_progress`.
    static BUDGET: Cell<Budget> = const { Cell::new(Budget::NONE) };
    /// First fatal sandbox refusal (`sleep`, `parse_json`) raised during the
    /// current invocation, latched so a callback's `ErrorInFunctionCall`
    /// wrapper + `try`/`catch` cannot swallow it.
    static FATAL_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Per-invocation monotonic budget state for `on_progress` (CRIT-8).
///
/// Tracks operations independently of Rhai's `GlobalRuntimeState::num_operations`
/// (which resets when a package method runs a callback on a cloned state) and
/// latches any overrun so `try`/`catch` around a callback cannot clear it.
#[derive(Clone, Copy)]
struct Budget {
    op_limit: u64,
    ops: u64,
    deadline: Option<Instant>,
    overrun: Option<&'static str>,
}

impl Budget {
    const NONE: Budget = Budget {
        op_limit: u64::MAX,
        ops: 0,
        deadline: None,
        overrun: None,
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
        FATAL_ERROR.with(|f| *f.borrow_mut() = None);
        guards::limit_begin();
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
    let max_queue = real_world
        .module_state::<ScriptState>()
        .map_or(ScriptConfig::default().max_queue_len, |st| {
            st.cfg.max_queue_len
        });
    let mut initial = initial;
    initial.truncate(max_queue.max(1));
    SESSION.with(|s| {
        *s.borrow_mut() = Some(ScriptSession {
            world: real_world,
            out: real_out,
            queue: initial,
            total: 0,
            current: None,
            out_events: 0,
            out_bytes: 0,
            scheduled: 0,
            dirty_scripts: BTreeMap::new(),
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

/// Whether `owner` holds the minimum role (`Role::Builder`) required to own,
/// compile, and run `script:*` hooks (CRIT-5 defense-in-depth).
fn owner_may_script(w: &World, owner: ObjId) -> bool {
    perm::has_role(w, owner, Role::Builder)
}

/// Resolve the executing authority for a `script:*` attribute on `obj`
/// (EXT-P1-1).
///
/// Both `obj.owner` and `attr.owner` must hold at least [`Role::Builder`], and
/// `attr.owner` must currently control `obj` (so revoking zone delegation or
/// transferring ownership immediately disables foreign scripts). Execution
/// authority is bound to `attr.owner` — never elevating a lower-role or peer
/// author to `obj.owner`'s role, powers, or unrelated objects. Only when
/// `attr.owner` is `Wizard`+ and strictly outranks `obj.owner` (e.g. staff
/// editing or `@chown`ing a builder's object) is authority capped down to
/// `obj.owner`, because `Wizard`+ strictly dominates every script-reachable
/// permission of a lower role.
fn effective_script_owner(w: &World, obj: &Obj, attr: &Attr) -> Option<ObjId> {
    let obj_owner = obj.owner;
    let attr_owner = attr.owner;
    if !owner_may_script(w, obj_owner) || !owner_may_script(w, attr_owner) {
        return None;
    }
    if !perm::controls(w, attr_owner, obj.id) {
        return None;
    }
    let obj_role = perm::role_of(w, obj_owner);
    let attr_role = perm::role_of(w, attr_owner);
    if attr_role >= Role::Wizard && obj_role < attr_role {
        Some(obj_owner)
    } else {
        Some(attr_owner)
    }
}

/// Run one trigger's script, then handle any error (strikes / HALTED).
fn run_one(trigger: &Trigger, cfg: ScriptConfig) {
    // Fetch engine + AST + effective script authority and set the frame; skip
    // HALTED/errored/gone objects or any script whose author/owner is
    // unauthorized.
    let prepared = SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let sess = b.as_mut()?;
        let obj = sess.world.get(trigger.self_obj)?;
        if obj.flags.contains(Flags::HALTED) || obj.flags.contains(Flags::GOING) {
            return None;
        }
        let attr = obj.attrs.get(trigger.script_key.as_str())?;
        let owner = effective_script_owner(&sess.world, obj, attr)?;
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
    fatal_begin();
    guards::limit_begin();
    // No SESSION borrow is held across this call — host functions reborrow.
    // Defense in depth: budget overruns already surface as a catchable `Err`,
    // but a panic in any host function or in the interpreter would otherwise
    // unwind out of the world task and kill the entire game loop. The
    // session's real world lives in the thread-local (the `SessionGuard`
    // restores it as the cascade unwinds) and no RefCell borrow is held across
    // this call, so catching keeps the loop alive and downgrades a panic to a
    // single failed script.
    let mut result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.run_ast_with_scope(&mut scope, &ast)
    }))
    .unwrap_or_else(|_| Err(err("script aborted (internal panic)")));
    let limit_hit = guards::limit_take();
    let overrun_reason = budget_tripped();
    let fatal_msg = fatal_take();
    clear_budget();
    SESSION.with(|s| {
        if let Some(sess) = s.borrow_mut().as_mut() {
            sess.current = None;
            flush_dirty_scripts(sess);
        }
    });

    // Reconcile latched sandbox faults even if a script caught a callback's
    // wrapped `ErrorInFunctionCall` in `try`/`catch` and returned cleanly (or
    // threw a different catchable error afterward).
    if limit_hit {
        result = Err(guards::depth_exceeded_err());
    } else if let Some(reason) = overrun_reason
        && result.is_ok()
    {
        result = Err(Box::new(EvalAltResult::ErrorTerminated(
            Dynamic::from(reason),
            Position::NONE,
        )));
    } else if let Some(msg) = fatal_msg {
        result = Err(fatal_error_unlatched(&msg));
    }
    let tripped = limit_hit || overrun_reason.is_some();

    match result {
        Err(e) => handle_error(trigger, &e, tripped, cfg),
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
            scope.push_constant("cmd", ImmutableString::from(cmd.as_ref()));
            scope.push_constant("arg", ImmutableString::from(arg.as_ref()));
        }
        Payload::Hear { text, speaker } => {
            scope.push_constant("text", ImmutableString::from(text.as_ref()));
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
///
/// Inspects both the innermost error (unwrapping callback `ErrorInFunctionCall`
/// wrappers) and the thread-local latched overrun flag so callback-wrapped
/// overruns always accrue strikes and halt runaway scripts (CRIT-8).
fn is_budget_overrun(err: &EvalAltResult, tripped: bool) -> bool {
    tripped
        || matches!(
            err.unwrap_inner(),
            EvalAltResult::ErrorTerminated(..)          // op / wall-clock abort
                | EvalAltResult::ErrorTooManyOperations(..) // global op ceiling
                | EvalAltResult::ErrorTooManyVariables(..)  // variable cap
                | EvalAltResult::ErrorTooManyModules(..)    // module cap
                | EvalAltResult::ErrorStackOverflow(..)     // call-level cap
                | EvalAltResult::ErrorDataTooLarge(..) // string/array/map/depth cap
        )
}

/// Format an error message up to a bounded character budget so thrown large or
/// nested values cannot flood the outbox or deeply recurse during `Display`.
fn bounded_error_text(err: &EvalAltResult) -> String {
    const MAX_ERR_CHARS: usize = 512;
    struct Bounded {
        text: String,
        chars: usize,
    }
    impl fmt::Write for Bounded {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            for c in s.chars() {
                if self.chars == MAX_ERR_CHARS {
                    return Err(fmt::Error);
                }
                self.text.push(if c.is_control() { ' ' } else { c });
                self.chars += 1;
            }
            Ok(())
        }
    }
    let mut out = Bounded {
        text: String::new(),
        chars: 0,
    };
    let _cut = write!(out, "{err}");
    out.text
}

/// Notify the owner of a failed script; on a budget overrun, accrue a strike
/// and auto-`HALTED` the object after `strikes_to_halt`.
fn handle_error(trigger: &Trigger, err: &EvalAltResult, tripped: bool, cfg: ScriptConfig) {
    let overrun = is_budget_overrun(err, tripped);
    let err_text = bounded_error_text(err);
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return };
        let obj = trigger.self_obj;
        let owner = sess.world.get(obj).map_or(obj, |o| {
            o.attrs
                .get(trigger.script_key.as_str())
                .and_then(|a| effective_script_owner(&sess.world, o, a))
                .unwrap_or(o.owner)
        });
        sess.out.error_on_commit(
            owner,
            format!(
                "Script {} on {} failed: {}",
                trigger.script_key, obj, err_text
            ),
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
// Budget & fatal-refusal helpers
// ----------------------------------------------------------------------

fn set_budget(cfg: &ScriptConfig) {
    let deadline = Instant::now() + Duration::from_millis(cfg.wall_clock_ms.max(1));
    BUDGET.with(|b| {
        b.set(Budget {
            op_limit: cfg.max_operations,
            ops: 0,
            deadline: Some(deadline),
            overrun: None,
        });
    });
}

fn clear_budget() {
    BUDGET.with(|b| b.set(Budget::NONE));
}

/// Whether the active invocation's op or wall-clock budget has tripped (latches
/// newly detected overruns so later checks see the same cause).
pub(crate) fn budget_tripped() -> Option<&'static str> {
    if guards::limit_tripped() {
        return Some("container limit exceeded");
    }
    BUDGET.with(|b| {
        let mut bud = b.get();
        if let Some(reason) = bud.overrun {
            return Some(reason);
        }
        if bud.ops > bud.op_limit {
            bud.overrun = Some("operation budget exceeded");
            b.set(bud);
            return bud.overrun;
        }
        if let Some(deadline) = bud.deadline
            && Instant::now() >= deadline
        {
            bud.overrun = Some("time budget exceeded");
            b.set(bud);
            return bud.overrun;
        }
        None
    })
}

/// Latch a resource/budget overrun so `try`/`catch` cannot swallow it and
/// `handle_error` accrues a strike (HIGH-18).
fn latch_budget_overrun(reason: &'static str) -> Box<EvalAltResult> {
    BUDGET.with(|b| {
        let mut bud = b.get();
        if bud.overrun.is_none() {
            bud.overrun = Some(reason);
            b.set(bud);
        }
    });
    Box::new(EvalAltResult::ErrorDataTooLarge(
        reason.to_string(),
        Position::NONE,
    ))
}

/// Uncatchable sandbox refusal payload inside `EvalAltResult::ErrorSystem`.
#[derive(Clone, Copy, Debug)]
struct SandboxRefusal;

impl fmt::Display for SandboxRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("refused")
    }
}

impl std::error::Error for SandboxRefusal {}

fn fatal_begin() {
    FATAL_ERROR.with(|f| *f.borrow_mut() = None);
}

fn fatal_take() -> Option<String> {
    FATAL_ERROR.with(|f| f.borrow_mut().take())
}

fn fatal_error_unlatched(msg: &str) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorSystem(
        msg.to_owned(),
        Box::new(SandboxRefusal),
    ))
}

fn fatal_error(msg: &str) -> Box<EvalAltResult> {
    FATAL_ERROR.with(|f| {
        let mut slot = f.borrow_mut();
        if slot.is_none() {
            *slot = Some(msg.to_owned());
        }
    });
    fatal_error_unlatched(msg)
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

/// Build the shared, sandboxed engine from `Engine::new_raw()` with an
/// explicit minimal safe package allowlist (CRIT-5), disabled dynamic/module
/// symbols, fatal `sleep`/`parse_json` shadows, monotonic progress budget
/// (CRIT-8), container/comparator guards (CRIT-6, CRIT-7, HIGH-16), silent
/// `on_print`/`on_debug`, and only the `mush` capability API.
fn build_engine(cfg: &ScriptConfig) -> RhaiEngine {
    let mut engine = RhaiEngine::new_raw();
    // Explicit minimal safe package allowlist (mirrors `mush-mind`): excludes
    // `MoreStringPackage` (OOM `replace`, infinite-loop `pad_with_string`),
    // `BasicTimePackage`, `BasicBlobPackage`, `BasicFnPackage`, and
    // `BasicMathPackage`.
    engine.register_global_module(LanguageCorePackage::new().as_shared_module());
    engine.register_global_module(ArithmeticPackage::new().as_shared_module());
    engine.register_global_module(LogicPackage::new().as_shared_module());
    engine.register_global_module(BasicIteratorPackage::new().as_shared_module());
    engine.register_global_module(BasicArrayPackage::new().as_shared_module());
    engine.register_global_module(BasicMapPackage::new().as_shared_module());
    engine.register_global_module(BasicStringPackage::new().as_shared_module());

    // Safe, non-growing string helpers builders rely on (e.g. `text.contains`
    // in `script:hear:*`) without exposing `MoreStringPackage`'s `replace` or
    // `pad_with_string`.
    register_safe_string_helpers(&mut engine);

    // A high global ceiling; the monotonic thread-local `on_progress` cap trips first.
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
    engine.set_max_modules(0);
    // Route `+=` through our depth-guarded shadows instead of built-in fast
    // operators that bypass registered `Array +=` overrides.
    engine.set_fast_operators(false);

    // No filesystem/network module loading, no dynamic eval, no Fn/call/curry,
    // and no `this` (scripts use `mush::me()`; disabling `this` also closes
    // `loop { this = [this]; }` container nesting that bypasses `on_var`).
    engine.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());
    for symbol in ["eval", "import", "export", "Fn", "call", "curry", "this"] {
        engine.disable_symbol(symbol);
    }

    // Never allow scripts to inject lines into server stdout.
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});

    // Shadow `sleep` and `parse_json` with fatal, latched refusals.
    engine.register_fn("sleep", |_: INT| -> Result<(), Box<EvalAltResult>> {
        Err(fatal_error("sleep is disabled in mush-script"))
    });
    engine.register_fn("sleep", |_: FLOAT| -> Result<(), Box<EvalAltResult>> {
        Err(fatal_error("sleep is disabled in mush-script"))
    });
    engine.register_fn(
        "parse_json",
        |_: &str| -> Result<Dynamic, Box<EvalAltResult>> {
            Err(fatal_error("parse_json is disabled in mush-script"))
        },
    );

    // Container padding, nesting depth, formatting/equality, and fallible
    // comparator sort/dedup shadows (CRIT-6, CRIT-7, HIGH-16).
    guards::register(&mut engine);

    // Monotonic op counter + wall-clock deadline + latched overrun (CRIT-8):
    // ignores Rhai's `_reported` counter, which resets inside package callbacks.
    engine.on_progress(|_reported| {
        if guards::limit_tripped() {
            return Some(Dynamic::from("container limit exceeded"));
        }
        BUDGET.with(|b| {
            let mut bud = b.get();
            if let Some(reason) = bud.overrun {
                return Some(Dynamic::from(reason));
            }
            bud.ops = bud.ops.saturating_add(1);
            if bud.ops > bud.op_limit {
                bud.overrun = Some("operation budget exceeded");
                b.set(bud);
                return Some(Dynamic::from("operation budget exceeded"));
            }
            // Sample the wall clock cheaply (every 1024 monotonic ops).
            if (bud.ops & 0x3FF) == 0
                && let Some(deadline) = bud.deadline
                && Instant::now() >= deadline
            {
                bud.overrun = Some("time budget exceeded");
                b.set(bud);
                return Some(Dynamic::from("time budget exceeded"));
            }
            b.set(bud);
            None
        })
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

/// Safe, non-growing string helpers registered explicitly in place of
/// `MoreStringPackage`.
fn register_safe_string_helpers(engine: &mut RhaiEngine) {
    FuncRegistration::new("contains")
        .in_global_namespace()
        .with_purity(true)
        .register_into_engine(engine, |s: &str, needle: &str| s.contains(needle));
    FuncRegistration::new("starts_with")
        .in_global_namespace()
        .with_purity(true)
        .register_into_engine(engine, |s: &str, prefix: &str| s.starts_with(prefix));
    FuncRegistration::new("ends_with")
        .in_global_namespace()
        .with_purity(true)
        .register_into_engine(engine, |s: &str, suffix: &str| s.ends_with(suffix));
    FuncRegistration::new("len")
        .in_global_namespace()
        .with_purity(true)
        .register_into_engine(engine, |s: &str| {
            INT::try_from(s.chars().count()).unwrap_or(INT::MAX)
        });
    FuncRegistration::new("is_empty")
        .in_global_namespace()
        .with_purity(true)
        .register_into_engine(engine, str::is_empty);
    FuncRegistration::new("trim")
        .in_global_namespace()
        .with_purity(false)
        .register_into_engine(engine, |s: &mut ImmutableString| {
            let trimmed = s.trim();
            if trimmed.len() != s.len() {
                *s = trimmed.into();
            }
        });
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

    /// An object's display name (`"nothing"` if it is gone, concealed, or out
    /// of reach).
    #[rhai_fn(name = "name")]
    pub fn name(obj: ObjId) -> String {
        super::name_impl(obj)
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

    /// Emit a narrative line with no name prefix to `target`'s room, and let
    /// the room's listeners (`script:hear:*`) react (bounded by the cascade
    /// cap). The line travels as a spoof attributed to the script's owner,
    /// so a NOSPOOF observer or staff still sees who is behind it.
    #[rhai_fn(name = "emit", return_raw)]
    pub fn emit(target: ObjId, text: &str) -> Result<(), Box<EvalAltResult>> {
        super::emit_impl(target, text)
    }

    /// Send a private line to one player/NPC, attributed to the script's
    /// owner the same way `emit` is. Honors locality, `HAVEN`, `@lock/page`,
    /// and `@ignore` (silently dropped when muted).
    #[rhai_fn(name = "pemit", return_raw)]
    pub fn pemit(player: ObjId, text: &str) -> Result<(), Box<EvalAltResult>> {
        super::pemit_impl(player, text)
    }

    /// Move `obj` to `dest` under the owner's authority (teleport rules +
    /// `MoveAttempt` vetoes); fires `script:leave`/`script:enter`.
    #[rhai_fn(name = "move", return_raw)]
    pub fn move_obj(obj: ObjId, dest: ObjId) -> Result<(), Box<EvalAltResult>> {
        super::move_impl(obj, dest)
    }

    /// Resolve a name to an object from the scripted object's viewpoint,
    /// enforcing owner visibility and locality (HIGH-17: prevents global
    /// `#id`/`*name` and local `DARK` oracles).
    /// (Named `matchit`, not `match`: `match` is a Rhai reserved word.)
    #[rhai_fn(name = "matchit", return_raw)]
    pub fn match_obj(query: &str) -> Result<ObjId, Box<EvalAltResult>> {
        let fr = super::frame(|f| *f)?;
        let q = query.trim();
        if q.is_empty() {
            return Err(super::err(MatchErr::NoMatch.to_string()));
        }
        super::with_session(|sess| {
            let w = &sess.world;
            if q.eq_ignore_ascii_case("me") || q.eq_ignore_ascii_case("here") {
                return match_object(w, fr.self_obj, q)
                    .ok()
                    .filter(|&id| w.get(id).is_some_and(|o| !o.flags.contains(Flags::GOING)))
                    .ok_or_else(|| super::err(MatchErr::NoMatch.to_string()));
            }
            if q.starts_with('#') || q.starts_with('*') {
                return match_object(w, fr.self_obj, q)
                    .ok()
                    .filter(|&id| super::script_can_observe(w, &fr, id))
                    .ok_or_else(|| super::err(MatchErr::NoMatch.to_string()));
            }
            match_object_visible(w, fr.self_obj, q, |id| {
                super::local_match_visible(w, &fr, id)
            })
            .map_err(|e| super::err(e.to_string()))
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
///
/// The lens is [`DetachedTypedState::ScriptSnapshot`]: this runs off the
/// world-task loop, where there is no live authoritative store to read, and
/// the hook slice itself is a boot snapshot. A hook that needs typed state
/// abstains here — deliberately, and by a name that says so.
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
        && !perceives_via(
            &st.perceive_hooks,
            w,
            owner,
            obj,
            TypedStateLens::Detached(DetachedTypedState::ScriptSnapshot),
        )
    {
        return false;
    }
    true
}

/// Whether `obj` is physically within reach of the running script's object:
/// nearby `self_obj` (self, contents, location, siblings, exits), an exit of
/// `self_obj` itself when it is a room, or a visible member of a nearby
/// open container (matching `contents_impl`).
fn script_is_local(w: &World, fr: &Frame, obj: ObjId) -> bool {
    if is_nearby(w, fr.self_obj, obj) || w.exits_of(fr.self_obj).contains(&obj) {
        return true;
    }
    if let Some(container) = w.get(obj).and_then(|o| o.location)
        && is_nearby(w, fr.self_obj, container)
        && let Some(c) = w.get(container)
    {
        let privileged = perm::has_power(w, fr.owner, Powers::SEE_HIDDEN)
            || perm::controls(w, fr.owner, container);
        let visible_container = privileged
            || (c.kind == ObjKind::Thing
                && !c.flags.contains(Flags::DARK)
                && !c.flags.contains(Flags::OPAQUE));
        if visible_container {
            return true;
        }
    }
    false
}

/// Whether the running script may observe `obj`: the owner must be able to
/// perceive it ([`owner_can_see`]) AND `obj` must either be local to
/// `self_obj` (not concealed inside an uncontrolled `DARK` room) or examinable
/// by `owner` (HIGH-17).
fn script_can_observe(w: &World, fr: &Frame, obj: ObjId) -> bool {
    if !owner_can_see(w, fr.owner, obj) {
        return false;
    }
    if perm::can_examine(w, fr.owner, obj) {
        return true;
    }
    if !script_is_local(w, fr, obj) {
        return false;
    }
    // An uncarried non-room occupant inside a DARK room the owner neither
    // controls nor sees through is concealed, matching `room_view`.
    let carried = obj == fr.self_obj || w.contents_of(fr.self_obj).contains(&obj);
    if !carried
        && w.get(obj).is_some_and(|o| o.kind != ObjKind::Room)
        && !perm::has_power(w, fr.owner, Powers::SEE_HIDDEN)
        && let Some(loc) = w.get(fr.self_obj).and_then(|o| o.location)
        && w.get(loc).is_some_and(|r| r.flags.contains(Flags::DARK))
        && !perm::controls(w, fr.owner, loc)
    {
        return false;
    }
    true
}

/// Visibility predicate for local `mush::matchit` candidate pools (HIGH-17):
/// hides `DARK`/concealed objects and occupants of an uncontrolled `DARK` room
/// before exact/prefix/ordinal matching runs.
fn local_match_visible(w: &World, fr: &Frame, id: ObjId) -> bool {
    if !owner_can_see(w, fr.owner, id) {
        return false;
    }
    let carried = id == fr.self_obj || w.contents_of(fr.self_obj).contains(&id);
    if !carried
        && !perm::has_power(w, fr.owner, Powers::SEE_HIDDEN)
        && !perm::controls(w, fr.owner, id)
        && let Some(loc) = w.get(fr.self_obj).and_then(|o| o.location)
        && w.get(loc).is_some_and(|r| r.flags.contains(Flags::DARK))
        && !perm::controls(w, fr.owner, loc)
    {
        return false;
    }
    true
}

/// `name(obj)` with owner visibility and locality gating (HIGH-17).
/// Returns `"nothing"` for gone, concealed, or distant uncontrolled objects.
fn name_impl(obj: ObjId) -> String {
    with_session(|sess| {
        let Some(fr) = sess.current else {
            return "nothing".to_string();
        };
        let w = &sess.world;
        let allowed = script_can_observe(w, &fr, obj)
            || (obj == fr.caller && owner_can_see(w, fr.owner, obj));
        if !allowed {
            return "nothing".to_string();
        }
        mush_core::output::display_name(w, obj)
    })
    .unwrap_or_default()
}

/// `loc(obj)` with visibility and locality gating: a hidden or distant
/// uncontrolled object reports "no location", the same error as a genuinely
/// location-less object, so existence is not leaked.
fn loc_visible(obj: ObjId) -> Result<ObjId, Box<EvalAltResult>> {
    let visible = with_session(|sess| {
        let Some(fr) = sess.current else {
            return false;
        };
        obj == fr.self_obj || script_can_observe(&sess.world, &fr, obj)
    });
    match visible {
        Some(true) => loc_of(obj),
        _ => Err(err(format!("{obj} has no location"))),
    }
}

/// `contents(obj)` with the look layer's DARK/OPAQUE and locality gating.
fn contents_impl(container: ObjId) -> Array {
    with_session(|sess| {
        let w = &sess.world;
        let Some(fr) = sess.current else {
            return Array::new();
        };
        let (owner, viewpoint) = (fr.owner, fr.self_obj);
        let Some(c) = w.get(container) else {
            return Array::new();
        };
        if c.flags.contains(Flags::GOING) {
            return Array::new();
        }
        let privileged =
            perm::has_power(w, owner, Powers::SEE_HIDDEN) || perm::controls(w, owner, container);
        // OPAQUE hides contents only from a viewpoint *outside* the container;
        // the scripted object's own location is the physical viewpoint.
        let inside = w.get(viewpoint).and_then(|o| o.location) == Some(container);
        let reachable =
            inside || script_is_local(w, &fr, container) || perm::can_examine(w, owner, container);
        if !reachable {
            return Array::new();
        }
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

/// `has_flag(obj, flag)` with visibility and locality gating (HIGH-17).
fn has_flag_impl(obj: ObjId, flag: &str) -> bool {
    with_session(|sess| {
        let Some(fr) = sess.current else {
            return false;
        };
        if !script_can_observe(&sess.world, &fr, obj) {
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
        let Some(fr) = sess.current else {
            return String::new();
        };
        let w = &sess.world;
        let owner = fr.owner;
        if !script_can_observe(w, &fr, obj) {
            return String::new();
        }
        let attr_trimmed = attr.trim();
        if attr_trimmed.is_empty() {
            return String::new();
        }
        // Module-owned namespaces (e.g. `script:*`, `wod:*`) are never readable
        // on objects the script owner cannot examine.
        if let Some((ns, _)) = attr_trimmed.split_once(':') {
            let owned = w.module_state::<ScriptState>().is_some_and(|st| {
                st.owned_namespaces
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(ns))
            });
            if owned && !perm::can_examine(w, owner, obj) {
                return String::new();
            }
        }
        let Some(a) = w.attr(obj, attr_trimmed) else {
            return String::new();
        };
        // Match `examine_cmd` per-flag confidentiality rules (HIGH-17):
        // - WIZARD_ONLY: Wizard+ only (even on objects the builder owns).
        // - SELF_ONLY: the object itself (`owner == obj`) or Wizard+ only.
        // - HIDDEN: requires `can_examine`.
        let owner_is_wizard = perm::has_role(w, owner, Role::Wizard);
        if a.flags.contains(AttrFlags::WIZARD_ONLY) && !owner_is_wizard {
            return String::new();
        }
        if a.flags.contains(AttrFlags::SELF_ONLY) && owner != obj && !owner_is_wizard {
            return String::new();
        }
        if a.flags.contains(AttrFlags::HIDDEN) && !perm::can_examine(w, owner, obj) {
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
        let is_wizard = perm::has_role(&sess.world, owner, Role::Wizard);
        if let Some((ns, _)) = attr.split_once(':') {
            let owned = sess.world.module_state::<ScriptState>().is_some_and(|st| {
                st.owned_namespaces
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(ns))
            });
            if owned {
                let min_role = if ns.eq_ignore_ascii_case("script") {
                    Role::Builder
                } else {
                    Role::Wizard
                };
                if !perm::has_role(&sess.world, owner, min_role) {
                    return Err(err(format!(
                        "The {ns}: attribute namespace is module-owned."
                    )));
                }
            }
        }
        // Executable `script:*` attributes cannot be written or mutated on
        // objects owned by a higher-role principal (EXT-P1-1).
        if perm::is_script_attr(attr) && !perm::can_write_script_attr(&sess.world, owner, obj) {
            return Err(err("Permission denied."));
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
        if existing_flags.is_some_and(|f| f.contains(AttrFlags::WIZARD_ONLY)) && !is_wizard {
            return Err(err("Permission denied."));
        }
        if existing.is_some_and(|(f, setter)| {
            f.contains(AttrFlags::LOCKED) && setter != owner && !is_wizard
        }) {
            return Err(err("That attribute is locked."));
        }
        // CORE-M1 + HIGH-18: enforce per-object attribute count & byte quotas
        // for non-Wizards so scripts cannot bypass `@set` limits.
        if !is_wizard
            && let Some(o) = sess.world.get(obj)
            && let Err(e) = o.attrs.check_default_limits(attr, val.len())
        {
            return Err(err(e.to_string()));
        }
        // HIGH-18: cap per-object `script:*` attribute count so one object
        // cannot accumulate unbounded ASTs or compilation work.
        let folded_script_key = if perm::is_script_attr(attr) {
            let max_scripts = sess
                .world
                .module_state::<ScriptState>()
                .map_or(ScriptConfig::default().max_scripts_per_object, |st| {
                    st.cfg.max_scripts_per_object
                });
            let existing_count = sess
                .world
                .get(obj)
                .map_or(0, |o| o.attrs.iter_namespace("script").count());
            if existing.is_none() && existing_count >= max_scripts {
                return Err(err("Too many script attributes on that object."));
            }
            Some(SmolStr::from(attr.to_lowercase()))
        } else {
            None
        };
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
        // HIGH-18: coalesce `script:*` recompilation until this invocation
        // completes, avoiding O(K²) recompilation on batch edits while still
        // making authored scripts live before the next trigger or command.
        if let Some(key) = folded_script_key {
            sess.dirty_scripts.entry(obj).or_default().insert(key);
        }
        Ok(())
    })
}

/// Charge one `emit`/`pemit` event and its text bytes against the cascade
/// outbox budget (HIGH-18).
fn charge_output(sess: &mut ScriptSession, text_bytes: usize) -> Result<(), Box<EvalAltResult>> {
    let cfg = sess
        .world
        .module_state::<ScriptState>()
        .map_or_else(ScriptConfig::default, |st| st.cfg);
    let next_events = sess.out_events.saturating_add(1);
    let next_bytes = sess.out_bytes.saturating_add(text_bytes);
    if next_events > cfg.max_outbox_events || next_bytes > cfg.max_outbox_bytes {
        return Err(latch_budget_overrun("script output budget exceeded"));
    }
    sess.out_events = next_events;
    sess.out_bytes = next_bytes;
    Ok(())
}

/// Remaining slots available in `sess.queue`, bounded by both `max_queue_len`
/// and the cascade's remaining invocation budget (HIGH-18).
fn remaining_queue_space(sess: &ScriptSession) -> usize {
    let cfg = sess
        .world
        .module_state::<ScriptState>()
        .map_or_else(ScriptConfig::default, |st| st.cfg);
    let remaining_invocations = cfg.max_total_invocations.saturating_sub(sess.total) as usize;
    let cap = cfg.max_queue_len.min(remaining_invocations);
    cap.saturating_sub(sess.queue.len())
}

fn emit_impl(target: ObjId, text: &str) -> Result<(), Box<EvalAltResult>> {
    with_session_mut(|sess| {
        let fr = sess.current.ok_or_else(|| err("no frame"))?;
        let (self_obj, owner, depth) = (fr.self_obj, fr.owner, fr.depth);
        let is_self_or_here =
            target == self_obj || sess.world.get(self_obj).and_then(|o| o.location) == Some(target);
        if !is_self_or_here && !owner_can_see(&sess.world, owner, target) {
            return Err(err("Cannot emit there."));
        }
        let Some(room) = room_of(&sess.world, target) else {
            return Err(err("Cannot emit there."));
        };
        // HIGH-17: scripts may emit to their own room or a room their owner
        // controls (which includes Wizard+ anywhere).
        let self_room = room_of(&sess.world, self_obj);
        if Some(room) != self_room && !perm::controls(&sess.world, owner, room) {
            return Err(err("Permission denied."));
        }
        // Honor the room's `@lock/speech` for non-Wizards.
        if !perm::has_role(&sess.world, owner, Role::Wizard)
            && let Some(lock) = sess
                .world
                .get(room)
                .and_then(|r| r.locks.get(&LockType::Speech))
            && !eval_lock(&sess.world, lock, owner, MAX_LOCK_DEPTH)
        {
            return Err(err("You can't speak here."));
        }
        charge_output(sess, text.len())?;
        // Attributed to the owner, as a spoof: the room fan-out shows the
        // source to NOSPOOF listeners and staff, and bare text to others.
        let actor = Some(obj_ref(&sess.world, owner));
        sess.out.to_room(
            room,
            Vec::new(),
            OutputEvent::Narrative {
                kind: NarrativeKind::Spoof,
                actor,
                text: RichText::plain(text),
            },
        );
        let max_depth = sess
            .world
            .module_state::<ScriptState>()
            .map_or(ScriptConfig::default().max_depth, |st| st.cfg.max_depth);
        let next_depth = depth.saturating_add(1);
        let space = remaining_queue_space(sess);
        if next_depth <= max_depth && space > 0 {
            let hears = collect_hear_triggers(&sess.world, room, self_obj, text, next_depth, space);
            sess.queue.extend(hears);
        }
        Ok(())
    })
}

fn pemit_impl(player: ObjId, text: &str) -> Result<(), Box<EvalAltResult>> {
    with_session_mut(|sess| {
        let fr = sess.current.ok_or_else(|| err("no frame"))?;
        let owner = fr.owner;
        let target_is_actor = sess.world.get(player).is_some_and(|o| o.kind.is_actor());
        let target_visible = target_is_actor && owner_can_see(&sess.world, owner, player);
        let target_reachable = (script_is_local(&sess.world, &fr, player)
            && script_can_observe(&sess.world, &fr, player))
            || player == fr.caller
            || perm::controls(&sess.world, owner, player);
        if !target_visible || !target_reachable {
            return Err(err("Cannot pemit that target."));
        }
        // HIGH-17: enforce HAVEN and `@lock/page` when owner does not control
        // the recipient (self and Wizard+ bypass via `controls`).
        if !perm::controls(&sess.world, owner, player)
            && let Some(t) = sess.world.get(player)
        {
            let refused = t.flags.contains(Flags::HAVEN)
                || t.locks
                    .get(&LockType::Page)
                    .is_some_and(|l| !eval_lock(&sess.world, l, owner, MAX_LOCK_DEPTH));
            if refused {
                return Err(err("That player does not wish to be disturbed."));
            }
        }
        charge_output(sess, text.len())?;
        // HIGH-17: `Audience::Actor` does not filter `@ignore` in the world
        // task, so check `is_ignoring` explicitly and drop silently (no oracle).
        if is_ignoring(&sess.world, player, owner) {
            return Ok(());
        }
        let actor = Some(obj_ref(&sess.world, owner));
        sess.out.to_actor(
            player,
            OutputEvent::Narrative {
                kind: NarrativeKind::Spoof,
                actor,
                text: RichText::plain(text),
            },
        );
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
        let max_depth = sess
            .world
            .module_state::<ScriptState>()
            .map_or(ScriptConfig::default().max_depth, |st| st.cfg.max_depth);
        let next_depth = depth.saturating_add(1);
        if next_depth <= max_depth {
            if let Some(from) = from {
                let space = remaining_queue_space(sess);
                if space > 0 {
                    let mut leave = Vec::new();
                    collect_move_triggers(
                        &mut leave,
                        &sess.world,
                        from,
                        obj,
                        false,
                        next_depth,
                        space,
                    );
                    sess.queue.extend(leave);
                }
            }
            let space = remaining_queue_space(sess);
            if space > 0 {
                let mut enter = Vec::new();
                collect_move_triggers(&mut enter, &sess.world, dest, obj, true, next_depth, space);
                sess.queue.extend(enter);
            }
        }
        Ok(())
    })
}

fn schedule_impl(secs: i64, timer: &str) -> Result<(), Box<EvalAltResult>> {
    with_session_mut(|sess| {
        if secs < MIN_TIMER_DELAY_SECS {
            return Err(err(format!(
                "timer delay must be at least {MIN_TIMER_DELAY_SECS} second"
            )));
        }
        if secs > MAX_TIMER_DELAY_SECS {
            return Err(err("timer delay is too long"));
        }
        let name = timer.trim();
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Err(err("timer name must be one non-empty word"));
        }
        let max_schedules = sess
            .world
            .module_state::<ScriptState>()
            .map_or(ScriptConfig::default().max_schedules_per_cascade, |st| {
                st.cfg.max_schedules_per_cascade
            });
        if sess.scheduled >= max_schedules {
            return Err(latch_budget_overrun("script schedule budget exceeded"));
        }
        let self_obj = sess
            .current
            .map(|f| f.self_obj)
            .ok_or_else(|| err("no frame"))?;
        sess.scheduled = sess.scheduled.saturating_add(1);
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

/// Maximum triggers an originating world event may collect into a fresh
/// cascade queue, bounded by both `max_queue_len` and `max_total_invocations`
/// (EXT-P2-8).
pub(crate) fn initial_trigger_cap(w: &World) -> usize {
    let cfg = w
        .module_state::<ScriptState>()
        .map_or_else(ScriptConfig::default, |st| st.cfg);
    cfg.max_queue_len.min(cfg.max_total_invocations as usize)
}

/// Whether `obj` has a runnable (compiled, non-errored, not HALTED/GONE,
/// authorized) script under the folded `key`.
pub(crate) fn has_runnable(w: &World, obj: ObjId, key: &str) -> bool {
    let Some(st) = w.module_state::<ScriptState>() else {
        return false;
    };
    has_runnable_with_state(w, st, obj, key)
}

fn has_runnable_with_state(w: &World, st: &ScriptState, obj: ObjId, key: &str) -> bool {
    let Some(os) = st.cache.get(&obj) else {
        return false;
    };
    if !os.scripts.contains_key(key) {
        return false;
    }
    let Some(o) = w.get(obj) else {
        return false;
    };
    if o.flags.contains(Flags::HALTED) || o.flags.contains(Flags::GOING) {
        return false;
    }
    o.attrs
        .get(key)
        .is_some_and(|a| effective_script_owner(w, o, a).is_some())
}

/// The objects a hear / move / command search scans: a room and its contents
/// (the room itself first, then its contents), yielded lazily without cloning
/// the room's contents vec (EXT-P2-8).
fn room_scope(w: &World, room: ObjId) -> impl Iterator<Item = ObjId> + '_ {
    std::iter::once(room).chain(w.contents_of(room).iter().copied())
}

/// Hearers in `room` (room + contents, minus the speaker) with a runnable
/// `script:hear[:*]`, stopping immediately at `limit` triggers and sharing
/// `text` via one `Arc<str>` across all collected triggers (EXT-P2-8).
pub(crate) fn collect_hear_triggers(
    w: &World,
    room: ObjId,
    speaker: ObjId,
    text: &str,
    depth: u32,
    limit: usize,
) -> Vec<Trigger> {
    if limit == 0 {
        return Vec::new();
    }
    let Some(st) = w.module_state::<ScriptState>() else {
        return Vec::new();
    };
    if st.cache.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut shared_text: Option<Arc<str>> = None;
    for cand in room_scope(w, room) {
        if out.len() >= limit {
            break;
        }
        if cand == speaker {
            continue;
        }
        let Some(os) = st.cache.get(&cand) else {
            continue;
        };
        let Some(o) = w.get(cand) else {
            continue;
        };
        if o.flags.contains(Flags::HALTED) || o.flags.contains(Flags::GOING) {
            continue;
        }
        for key in os.scripts.keys() {
            if out.len() >= limit {
                break;
            }
            let k = key.as_str();
            if k != "script:hear" && !k.starts_with("script:hear:") {
                continue;
            }
            if !o
                .attrs
                .get(k)
                .is_some_and(|a| effective_script_owner(w, o, a).is_some())
            {
                continue;
            }
            let text_arc = Arc::clone(shared_text.get_or_insert_with(|| Arc::from(text)));
            out.push(Trigger {
                self_obj: cand,
                caller: speaker,
                script_key: key.clone(),
                depth,
                payload: Payload::Hear {
                    text: text_arc,
                    speaker,
                },
            });
        }
    }
    out
}

/// Enter/leave reactors in `room` (room + contents, minus the mover),
/// appending at most `limit` triggers (EXT-P2-8).
pub(crate) fn collect_move_triggers(
    out: &mut Vec<Trigger>,
    w: &World,
    room: ObjId,
    mover: ObjId,
    entering: bool,
    depth: u32,
    limit: usize,
) {
    if limit == 0 {
        return;
    }
    let Some(st) = w.module_state::<ScriptState>() else {
        return;
    };
    if st.cache.is_empty() {
        return;
    }
    let key = if entering {
        "script:enter"
    } else {
        "script:leave"
    };
    let target_len = out.len().saturating_add(limit);
    for cand in room_scope(w, room) {
        if out.len() >= target_len {
            break;
        }
        if cand == mover {
            continue;
        }
        if has_runnable_with_state(w, st, cand, key) {
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
/// classic scope: the actor, their inventory, their room, and its contents,
/// stopping immediately at `limit` triggers and sharing `verb`/`arg` via
/// `Arc<str>` (EXT-P2-8).
pub(crate) fn collect_command_triggers(
    w: &World,
    actor: ObjId,
    verb: &str,
    arg: &str,
    limit: usize,
) -> Vec<Trigger> {
    if limit == 0 {
        return Vec::new();
    }
    let Some(st) = w.module_state::<ScriptState>() else {
        return Vec::new();
    };
    if st.cache.is_empty() {
        return Vec::new();
    }
    let key = SmolStr::new(format!("script:command:{}", verb.to_lowercase()));
    let loc = w.get(actor).and_then(|o| o.location);
    let scope = std::iter::once(actor)
        .chain(w.contents_of(actor).iter().copied())
        .chain(loc.into_iter().flat_map(|r| room_scope(w, r)));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut shared_cmd: Option<Arc<str>> = None;
    let mut shared_arg: Option<Arc<str>> = None;
    for cand in scope {
        if out.len() >= limit {
            break;
        }
        if !has_runnable_with_state(w, st, cand, &key) {
            continue;
        }
        if !seen.insert(cand) {
            continue;
        }
        let cmd = Arc::clone(shared_cmd.get_or_insert_with(|| Arc::from(verb)));
        let arg = Arc::clone(shared_arg.get_or_insert_with(|| Arc::from(arg)));
        out.push(Trigger {
            self_obj: cand,
            caller: actor,
            script_key: key.clone(),
            depth: 0,
            payload: Payload::Command { cmd, arg },
        });
    }
    out
}

/// Flush all coalesced `script:*` attribute edits recorded during the just-ended
/// invocation, compiling each modified key at most once (HIGH-18).
fn flush_dirty_scripts(sess: &mut ScriptSession) {
    if sess.dirty_scripts.is_empty() {
        return;
    }
    let dirty = std::mem::take(&mut sess.dirty_scripts);
    for (obj, keys) in dirty {
        recompile_script_attrs(&mut sess.world, &mut sess.out, obj, keys, false);
    }
}

/// Incrementally recompile a single changed `script:*` attribute on `obj`
/// (from external `@set`), avoiding O(K²) full-object recompilation (HIGH-18).
pub(crate) fn recompile_script_attr(w: &mut World, out: &mut Outbox, obj: ObjId, attr: &str) {
    let mut keys = BTreeSet::new();
    keys.insert(SmolStr::from(attr.trim().to_lowercase()));
    recompile_script_attrs(w, out, obj, keys, true);
}

/// Incrementally compile a batch of folded `script:*` keys on `obj`.
fn recompile_script_attrs(
    w: &mut World,
    out: &mut Outbox,
    obj: ObjId,
    keys: BTreeSet<SmolStr>,
    clear_strikes: bool,
) {
    let Some((engine, max_scripts)) = w
        .module_state::<ScriptState>()
        .map(|st| (Arc::clone(&st.engine), st.cfg.max_scripts_per_object))
    else {
        return;
    };
    let Some(o) = w.get(obj) else {
        return;
    };
    if !owner_may_script(w, o.owner) {
        if let Some(st) = w.module_state_mut::<ScriptState>() {
            st.cache.remove(&obj);
            st.strikes.remove(&obj);
        }
        return;
    }
    enum Update {
        Remove,
        Compiled { ast: Arc<AST>, notify: ObjId },
        Error { notify: ObjId, msg: String },
    }
    let mut updates: Vec<(SmolStr, Update)> = Vec::with_capacity(keys.len());
    let mut compiled_n = 0u64;
    for key in keys {
        match o.attrs.get(&key) {
            None => updates.push((key, Update::Remove)),
            Some(a) => {
                let Some(notify) = effective_script_owner(w, o, a) else {
                    updates.push((key, Update::Remove));
                    continue;
                };
                match a.as_text() {
                    None => updates.push((
                        key,
                        Update::Error {
                            notify,
                            msg: "script attribute must be text".into(),
                        },
                    )),
                    Some(src) => {
                        compiled_n = compiled_n.saturating_add(1);
                        match engine.compile(src) {
                            Ok(ast) => match guards::validate_source(src) {
                                Ok(()) => updates.push((
                                    key,
                                    Update::Compiled {
                                        ast: Arc::new(ast),
                                        notify,
                                    },
                                )),
                                Err(msg) => updates.push((key, Update::Error { notify, msg })),
                            },
                            Err(e) => updates.push((
                                key,
                                Update::Error {
                                    notify,
                                    msg: e.to_string(),
                                },
                            )),
                        }
                    }
                }
            }
        }
    }

    let mut errors: Vec<(ObjId, SmolStr, String)> = Vec::new();
    if let Some(st) = w.module_state_mut::<ScriptState>() {
        st.compile_count = st.compile_count.saturating_add(compiled_n);
        for (key, upd) in updates {
            match upd {
                Update::Remove => {
                    if let Some(os) = st.cache.get_mut(&obj) {
                        os.scripts.remove(&key);
                        if os.scripts.is_empty() {
                            st.cache.remove(&obj);
                        }
                    }
                }
                Update::Compiled { ast, notify } => {
                    let os = st.cache.entry(obj).or_default();
                    if !os.scripts.contains_key(&key) && os.scripts.len() >= max_scripts {
                        errors.push((
                            notify,
                            key,
                            format!("object exceeded maximum of {max_scripts} scripts"),
                        ));
                    } else {
                        os.scripts.insert(key, ast);
                    }
                }
                Update::Error { notify, msg } => {
                    if let Some(os) = st.cache.get_mut(&obj) {
                        os.scripts.remove(&key);
                        if os.scripts.is_empty() {
                            st.cache.remove(&obj);
                        }
                    }
                    errors.push((notify, key, msg));
                }
            }
        }
        if clear_strikes {
            st.strikes.remove(&obj);
        }
    }
    for (notify, key, msg) in errors {
        out.error_on_commit(
            notify,
            format!("Script {key} on {obj} failed to compile: {msg}"),
        );
    }
}

/// (Re)compile every authorized `script:*` attribute on `obj`, cache the
/// results, clear its strikes, and notify the effective owner of any compile
/// errors. Removes the cache entry when the object has no authorized scripts
/// or its owner is below `Role::Builder` (CRIT-5 + EXT-P1-1 defense-in-depth).
/// Enforces `max_scripts_per_object` (HIGH-18).
pub(crate) fn recompile_object(w: &mut World, out: &mut Outbox, obj: ObjId) {
    let Some((engine, max_scripts)) = w
        .module_state::<ScriptState>()
        .map(|st| (Arc::clone(&st.engine), st.cfg.max_scripts_per_object))
    else {
        return;
    };
    let Some(o) = w.get(obj) else {
        return;
    };
    if !owner_may_script(w, o.owner) {
        if let Some(st) = w.module_state_mut::<ScriptState>() {
            st.cache.remove(&obj);
            st.strikes.remove(&obj);
        }
        return;
    }
    let sources: Vec<(SmolStr, ObjId, Option<String>)> = o
        .attrs
        .iter_namespace("script")
        .filter_map(|(k, a)| {
            let notify = effective_script_owner(w, o, a)?;
            Some((
                SmolStr::new(k.folded()),
                notify,
                a.as_text().map(str::to_string),
            ))
        })
        .collect();

    if sources.is_empty() {
        if let Some(st) = w.module_state_mut::<ScriptState>() {
            st.cache.remove(&obj);
            st.strikes.remove(&obj);
        }
        return;
    }

    let mut compiled: HashMap<SmolStr, Arc<AST>> = HashMap::new();
    let mut errors: Vec<(ObjId, SmolStr, String)> = Vec::new();
    let mut compiled_n = 0u64;
    for (key, notify, text) in sources {
        if compiled.len() >= max_scripts {
            errors.push((
                notify,
                key,
                format!("object exceeded maximum of {max_scripts} scripts"),
            ));
            continue;
        }
        match text {
            None => errors.push((notify, key, "script attribute must be text".to_string())),
            Some(src) => {
                compiled_n = compiled_n.saturating_add(1);
                match engine.compile(&src) {
                    Ok(ast) => match guards::validate_source(&src) {
                        Ok(()) => {
                            compiled.insert(key, Arc::new(ast));
                        }
                        Err(msg) => errors.push((notify, key, msg)),
                    },
                    Err(e) => errors.push((notify, key, e.to_string())),
                }
            }
        }
    }

    if let Some(st) = w.module_state_mut::<ScriptState>() {
        st.compile_count = st.compile_count.saturating_add(compiled_n);
        if compiled.is_empty() {
            st.cache.remove(&obj);
        } else {
            st.cache.insert(obj, ObjScripts { scripts: compiled });
        }
        st.strikes.remove(&obj);
    }
    for (notify, key, msg) in errors {
        out.error_on_commit(
            notify,
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

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{GOD, LIMBO};

    fn world_with_script_state(cfg: ScriptConfig) -> (World, ObjId) {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        w.set_flag(nick, Flags::BUILDER, true).unwrap();
        let engine = mush_core::engine::Engine::with_modules(
            w,
            Registry::new(),
            vec![Box::new(crate::ScriptModule::with_config(cfg))],
        )
        .unwrap();
        (engine.world, nick)
    }

    fn attach_script(w: &mut World, obj: ObjId, owner: ObjId, key: &str) {
        w.set_attr(obj, key, Attr::text("let x = 1;", owner))
            .unwrap();
        let mut out = Outbox::new();
        recompile_script_attr(w, &mut out, obj, key);
    }

    #[test]
    fn collect_hear_triggers_respects_limit_and_shares_text_arc() {
        let (mut w, nick) = world_with_script_state(ScriptConfig::default());
        for i in 0..100 {
            let ear = w
                .create(ObjKind::Thing, &format!("ear{i}"), nick, Some(LIMBO))
                .unwrap();
            attach_script(&mut w, ear, nick, "script:hear:a");
            attach_script(&mut w, ear, nick, "script:hear:b");
        }

        // Limit 0 collects nothing.
        assert!(collect_hear_triggers(&w, LIMBO, nick, "hello", 0, 0).is_empty());

        // Strict cap stops collection at exactly `limit` (not 200).
        let triggers = collect_hear_triggers(&w, LIMBO, nick, "shared payload", 0, 7);
        assert_eq!(triggers.len(), 7);

        // Every collected trigger shares the exact same `Arc<str>` allocation.
        let first = match &triggers[0].payload {
            Payload::Hear { text, .. } => text,
            _ => panic!("expected Hear payload"),
        };
        assert_eq!(first.as_ref(), "shared payload");
        for t in &triggers[1..] {
            match &t.payload {
                Payload::Hear { text, .. } => assert!(Arc::ptr_eq(first, text)),
                _ => panic!("expected Hear payload"),
            }
        }
    }

    #[test]
    fn collect_move_triggers_respects_limit() {
        let (mut w, nick) = world_with_script_state(ScriptConfig::default());
        for i in 0..80 {
            let watcher = w
                .create(ObjKind::Thing, &format!("watch{i}"), nick, Some(LIMBO))
                .unwrap();
            attach_script(&mut w, watcher, nick, "script:enter");
        }

        let mut out = Vec::new();
        collect_move_triggers(&mut out, &w, LIMBO, nick, true, 0, 0);
        assert!(out.is_empty());

        collect_move_triggers(&mut out, &w, LIMBO, nick, true, 0, 5);
        assert_eq!(out.len(), 5);

        // Subsequent append with remaining budget of 3 caps total at 8.
        collect_move_triggers(&mut out, &w, LIMBO, nick, true, 0, 3);
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn collect_command_triggers_respects_limit_dedupes_and_shares_arcs() {
        let (mut w, nick) = world_with_script_state(ScriptConfig::default());
        attach_script(&mut w, nick, nick, "script:command:ping");
        for i in 0..50 {
            let gadget = w
                .create(ObjKind::Thing, &format!("g{i}"), nick, Some(LIMBO))
                .unwrap();
            attach_script(&mut w, gadget, nick, "script:command:ping");
        }

        assert!(collect_command_triggers(&w, nick, "ping", "arg", 0).is_empty());

        let triggers = collect_command_triggers(&w, nick, "ping", "some args", 6);
        assert_eq!(triggers.len(), 6);
        // `nick` appears both as actor and in LIMBO contents; must appear once.
        let nick_count = triggers.iter().filter(|t| t.self_obj == nick).count();
        assert_eq!(nick_count, 1);

        let (first_cmd, first_arg) = match &triggers[0].payload {
            Payload::Command { cmd, arg } => (cmd, arg),
            _ => panic!("expected Command payload"),
        };
        assert_eq!(first_cmd.as_ref(), "ping");
        assert_eq!(first_arg.as_ref(), "some args");
        for t in &triggers[1..] {
            match &t.payload {
                Payload::Command { cmd, arg } => {
                    assert!(Arc::ptr_eq(first_cmd, cmd));
                    assert!(Arc::ptr_eq(first_arg, arg));
                }
                _ => panic!("expected Command payload"),
            }
        }
    }
}
