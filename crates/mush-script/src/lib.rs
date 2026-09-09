//! # mush-script
//!
//! Sandboxed builder scripting for rustmush, on **Rhai** (PLAN's Scripting
//! bullet + design 01 §8, on Rhai not Lua). Scripts are semi-trusted:
//! builders write them, so they must never (a) stall the game loop,
//! (b) exhaust memory, (c) bypass permissions, or (d) reach anything beyond
//! the exposed `mush` API. There is **no `mush::cmd`/`eval`** — scripts act
//! only through typed capabilities.
//!
//! ## Execution model
//!
//! Rhai runs **synchronously inside the world task**, fully budget-bounded,
//! so a script sees a consistent world and its effects order like any
//! command. See [`runtime`] for the sandbox, the budgets, and the safe
//! (no-`unsafe`) session bridge.
//!
//! ## How it hangs off the module API
//!
//! [`ScriptModule`] is a [`GameModule`] named `"script"`. On register it:
//!
//! - installs the one shared, sandboxed Rhai engine + the per-object AST
//!   cache in a [`runtime::ScriptState`] module-state slot;
//! - subscribes to [`WorldEventKind::AttrSet`] (recompile on `script:*`
//!   change — the additive core signal this milestone adds),
//!   [`WorldEventKind::ObjCreated`]/[`WorldEventKind::ObjDestroyed`] (cache
//!   upkeep), [`WorldEventKind::Moved`] (`on_enter`/`on_leave`), and every
//!   speech flavor of [`WorldEventKind::Said`] (`on_hear`);
//! - registers a post-command-table [`CommandFallback`] for object
//!   `$`-commands (`script:command:*` — a fallback, never a shadow of
//!   built-ins/exits, per treaty ruling 5), plus the `use` command
//!   (`on_use`) and the internal `@script/timer` command (`on_timer`).
//!
//! Scripts live in `script:<hook>[:<name>]` attributes:
//! `script:command:greet`, `script:enter`, `script:leave`,
//! `script:hear:<name>`, `script:use`, `script:timer:<name>`. On boot and on
//! any `script:*` change the owner's object is (re)compiled to cached ASTs;
//! a compile error disables just that script and notifies the owner.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

mod runtime;

use std::collections::VecDeque;
use std::sync::Arc;

use mush_core::module::{
    GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, WorldEvent, WorldEventKind,
};
use mush_core::{
    ArgShape, CmdError, CommandFallback, CommandSpec, Ctx, ObjId, Outbox, Role, World, match_object,
};
use mush_proto::NarrativeKind;
use smol_str::SmolStr;

pub use runtime::{ScriptConfig, ScriptState};
use runtime::{
    Trigger, collect_command_triggers, collect_hear_triggers, collect_move_triggers, drive,
    has_runnable, recompile_object, snapshot_registry,
};

/// The mush-script game module. Hand one to `Engine::with_modules`.
pub struct ScriptModule {
    cfg: ScriptConfig,
}

impl ScriptModule {
    /// A module with default sandbox budgets.
    #[must_use]
    pub fn new() -> ScriptModule {
        ScriptModule {
            cfg: ScriptConfig::default(),
        }
    }

    /// A module with custom sandbox budgets.
    #[must_use]
    pub fn with_config(cfg: ScriptConfig) -> ScriptModule {
        ScriptModule { cfg }
    }
}

impl Default for ScriptModule {
    fn default() -> ScriptModule {
        ScriptModule::new()
    }
}

impl GameModule for ScriptModule {
    fn name(&self) -> &'static str {
        "script"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // Object `$`-commands and `use` remain the explicitly declared script
        // compatibility lane; scripts cannot register authoritative providers.
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(ScriptState::new(self.cfg));

        reg.subscribe(WorldEventKind::AttrSet, Arc::new(on_attr_set));
        reg.subscribe(WorldEventKind::ObjCreated, Arc::new(on_obj_created));
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        reg.subscribe(WorldEventKind::Moved, Arc::new(on_moved));
        for kind in [
            NarrativeKind::Say,
            NarrativeKind::Pose,
            NarrativeKind::SemiPose,
            NarrativeKind::Emit,
        ] {
            reg.subscribe(WorldEventKind::Said { kind }, Arc::new(on_said));
        }

        reg.command_fallback(Arc::new(ScriptCommands))?;

        reg.command(
            CommandSpec {
                name: "use".into(),
                shape: ArgShape::Raw,
                min_role: Role::Player,
                help: "use <object>  --  Trigger an object's script:use script \
                       (if it has one). \"You can't use that.\" otherwise."
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(use_cmd),
        )?;
        reg.command(
            CommandSpec {
                name: "@script".into(),
                switches: vec!["timer".into()],
                shape: ArgShape::Raw,
                min_role: Role::Player,
                help: "@script/timer <name>  --  Internal: fire this object's \
                       script:timer:<name> script (armed by mush::schedule)."
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(script_cmd),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        snapshot_registry(ctx.w, ctx.reg);
        // Compile every object that carries script attributes.
        let ids: Vec<ObjId> = ctx
            .w
            .iter()
            .filter(|o| o.attrs.iter_namespace("script").next().is_some())
            .map(|o| o.id)
            .collect();
        for id in ids {
            recompile_object(ctx.w, ctx.out, id);
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Observers
// ----------------------------------------------------------------------

/// Recompile an object's scripts when one of its `script:*` attrs changes.
fn on_attr_set(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::AttrSet { obj, name, .. } = ev else {
        return;
    };
    if !name.to_ascii_lowercase().starts_with("script:") {
        return;
    }
    recompile_object(ctx.w, ctx.out, *obj);
}

/// Compile a freshly-created object that already carries scripts (e.g. an
/// `@clone` of a scripted object).
fn on_obj_created(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjCreated { obj, .. } = ev else {
        return;
    };
    let has_scripts = ctx
        .w
        .get(*obj)
        .is_some_and(|o| o.attrs.iter_namespace("script").next().is_some());
    if has_scripts {
        recompile_object(ctx.w, ctx.out, *obj);
    }
}

/// Drop a destroyed object's cache + strikes.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    if let Some(st) = ctx.w.module_state_mut::<ScriptState>() {
        st.cache.remove(obj);
        st.strikes.remove(obj);
    }
}

/// `on_hear`: run room listeners' `script:hear:*` scripts.
fn on_said(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::Said {
        who, room, text, ..
    } = ev
    else {
        return;
    };
    let triggers = collect_hear_triggers(ctx.w, *room, *who, text, 0);
    run(ctx, triggers);
}

/// `on_enter`/`on_leave`: run reactors in the vacated and entered rooms.
fn on_moved(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::Moved { who, from, to } = ev else {
        return;
    };
    let mut triggers = Vec::new();
    if let Some(from) = from {
        collect_move_triggers(&mut triggers, ctx.w, *from, *who, false, 0);
    }
    collect_move_triggers(&mut triggers, ctx.w, *to, *who, true, 0);
    run(ctx, triggers);
}

// ----------------------------------------------------------------------
// Object $-commands (the post-command-table fallback) + use/timer commands
// ----------------------------------------------------------------------

/// The `script:command:*` fallback (design §3.1 module fallback matcher).
struct ScriptCommands;

impl CommandFallback for ScriptCommands {
    fn try_command(&self, w: &mut World, actor: ObjId, line: &str, out: &mut Outbox) -> bool {
        let (verb, arg) = split_head(line);
        if verb.is_empty() {
            return false;
        }
        let triggers = collect_command_triggers(w, actor, verb, arg);
        if triggers.is_empty() {
            return false;
        }
        drive(w, out, triggers.into());
        true
    }
}

/// `use <object>`: fire the object's `script:use`.
fn use_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    if raw.is_empty() {
        return Err(CmdError::msg("Use what?"));
    }
    let target = match_object(ctx.w, ctx.actor, &raw)?;
    if !has_runnable(ctx.w, target, "script:use") {
        return Err(CmdError::msg("You can't use that."));
    }
    let trigger = Trigger {
        self_obj: target,
        caller: ctx.actor,
        script_key: SmolStr::new("script:use"),
        depth: 0,
        payload: runtime::Payload::Use { user: ctx.actor },
    };
    run(ctx, vec![trigger]);
    Ok(())
}

/// `@script/timer <name>`: fire the acting object's `script:timer:<name>`
/// (armed by `mush::schedule`; harmless if invoked directly).
fn script_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx.switches.has("timer") {
        return Err(CmdError::msg("Usage: @script/timer <name>."));
    }
    let name = ctx.args.raw().trim();
    if name.is_empty() {
        return Ok(());
    }
    let key = SmolStr::new(format!("script:timer:{name}"));
    if has_runnable(ctx.w, ctx.actor, &key) {
        let trigger = Trigger {
            self_obj: ctx.actor,
            caller: ctx.actor,
            script_key: key,
            depth: 0,
            payload: runtime::Payload::Timer,
        };
        run(ctx, vec![trigger]);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

/// Drive a cascade from a command/observer [`Ctx`].
fn run(ctx: &mut Ctx<'_>, triggers: Vec<Trigger>) {
    if triggers.is_empty() {
        return;
    }
    let queue: VecDeque<Trigger> = triggers.into();
    drive(ctx.w, ctx.out, queue);
}

/// Split off the first whitespace-delimited word.
fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}
