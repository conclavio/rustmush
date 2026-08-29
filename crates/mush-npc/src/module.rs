//! The `npc` [`GameModule`]: world-event observers that build stimuli, the
//! coalescing/cooldown tick that fires one off-loop turn per ready NPC, the
//! [`AsyncCompletion`] that applies the returned typed actions (re-validated),
//! and the `@agent` staff surface.
//!
//! Everything the LLM produces re-enters here as a typed [`NpcAction`] and is
//! re-validated against current world state (treaty ruling 3). A `Move` through
//! a now-locked exit fails exactly as a player's would; nothing bypasses the
//! command layer, and there is no path to a raw command line.

use std::sync::Arc;

use mush_core::commands::movement::{find_exit, traverse_exit};
use mush_core::module::{AsyncCompletion, WorldEvent};
use mush_core::{
    ArgShape, Attr, CmdError, CommandSpec, Ctx, Deferred, Effect, GameModule, ModError,
    ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, ObjId, ObjKind, Registrar, Role, TickCadence,
    World, WorldEventKind, WorldObserver, obj_ref, perm,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use smol_str::SmolStr;

use crate::action::NpcAction;
use crate::memory::{MemoryRecord, record_key};
use crate::profile::{AgentProfile, NS, PROFILE_ATTR, SUMMARY_ATTR, THINKING_FEED};
use crate::runtime::NpcHostState;
use crate::snapshot::{
    AgentGrounding, AppliedAction, NoGrounding, PresentPlayer, RecalledMemory, SceneView,
    StimulusLine, TurnKind, TurnOutcome, TurnSnapshot,
};
use crate::stimulus::{SpeechKind, Stimulus};

/// The NPC agent module. Construct with an [`AgentGrounding`] (the driver wires
/// the loaded ruleset; [`NpcModule::new`] uses [`NoGrounding`] for
/// dialogue-only worlds and tests).
pub struct NpcModule {
    grounding: Arc<dyn AgentGrounding>,
}

impl NpcModule {
    /// A module with no ruleset grounding (dialogue only).
    #[must_use]
    pub fn new() -> NpcModule {
        NpcModule {
            grounding: Arc::new(NoGrounding),
        }
    }

    /// A module grounded by `grounding` (the driver's ruleset adapter).
    #[must_use]
    pub fn with_grounding(grounding: Arc<dyn AgentGrounding>) -> NpcModule {
        NpcModule { grounding }
    }
}

impl Default for NpcModule {
    fn default() -> NpcModule {
        NpcModule::new()
    }
}

impl GameModule for NpcModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.attr_namespace("agent")?;
        let panel = reg.panel_feed(THINKING_FEED)?;
        let state = NpcHostState::new(panel);
        let tick_period = state.config.tick;
        reg.state(state);

        // Speech + movement + presence observers build stimuli.
        for kind in [
            NarrativeKind::Say,
            NarrativeKind::Pose,
            NarrativeKind::SemiPose,
            NarrativeKind::Emit,
        ] {
            reg.subscribe(WorldEventKind::Said { kind }, Arc::new(NpcObserver));
        }
        reg.subscribe(WorldEventKind::Moved, Arc::new(NpcObserver));
        reg.subscribe(WorldEventKind::Connected, Arc::new(NpcObserver));
        reg.subscribe(WorldEventKind::Disconnected, Arc::new(NpcObserver));

        // The coalescing/cooldown driver.
        reg.tick(
            "agent",
            TickCadence::Coarse(tick_period),
            Arc::new(NpcTick {
                grounding: Arc::clone(&self.grounding),
            }),
        )?;

        // The typed-action re-entry point (holds the grounding so a UseAbility
        // executes on the world loop).
        reg.async_completion(Arc::new(NpcCompletion {
            grounding: Arc::clone(&self.grounding),
        }))?;

        // Staff surface. The handler is grounding-aware so `@agent/sheet` can
        // give an NPC a ruleset character sheet through the loaded ruleset.
        let grounding = Arc::clone(&self.grounding);
        reg.command(
            CommandSpec {
                name: "@agent".into(),
                shape: ArgShape::EqPair,
                switches: vec![
                    "create".into(),
                    "set".into(),
                    "sheet".into(),
                    "show".into(),
                    "poke".into(),
                    "enable".into(),
                    "disable".into(),
                ],
                min_role: Role::Builder,
                help: "@agent/create <name> -- create an NPC agent here (a first-class \
                       ObjKind::Npc) with a default Minor profile; give it a persona with \
                       @agent/set and (for rolls) a sheet with @agent/sheet.\n\
                       @agent/set <npc>/<field>=<value> -- edit the profile. Fields: \
                       persona, background, speech, goals, fallback, knows, does_not_know, \
                       tier (major|minor|extra), cooldown (seconds), daily_tokens, enabled. \
                       List fields (goals/fallback/knows/does_not_know) take \";\"-separated \
                       values; an empty value clears a field.\n\
                       @agent/sheet <npc>[=<template>] -- give it a ruleset sheet (default \
                       mortal) so it can roll; fill it in with @sheet/set afterwards.\n\
                       @agent/show <npc> (or bare @agent <npc>) -- profile + runtime status.\n\
                       @agent/enable <npc> / @agent/disable <npc> -- wake or park the agent.\n\
                       @agent/poke <npc>=<note> -- prod the agent with an out-of-band note.\n\
                       See also @llm/kill <npc> (per-NPC LLM disable) and help npc."
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| cmd_agent(grounding.as_ref(), ctx)),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        boot: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut store = crate::memory::LongTermStore::default();
        for (key, value) in boot.kv {
            if key.starts_with("mem:") {
                match serde_json::from_value::<MemoryRecord>(value) {
                    Ok(rec) => store.insert(rec),
                    Err(e) => tracing::error!(%key, error = %e, "npc: unparseable memory skipped"),
                }
            } else {
                tracing::warn!(%key, "npc: unknown persisted key ignored");
            }
        }
        if let Some(h) = ctx.state_mut::<NpcHostState>() {
            h.long_term = store;
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Observers: WorldEvent -> Stimulus
// ----------------------------------------------------------------------

struct NpcObserver;

impl WorldObserver for NpcObserver {
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        match ev {
            WorldEvent::Said {
                who,
                room,
                kind,
                text,
            } => on_said(ctx, *who, *room, *kind, text),
            WorldEvent::Moved { who, from, to } => on_moved(ctx, *who, *from, *to),
            WorldEvent::Connected { who } => on_presence(ctx, *who, true),
            WorldEvent::Disconnected { who } => on_presence(ctx, *who, false),
            _ => {}
        }
    }
}

fn on_said(ctx: &mut Ctx<'_>, who: ObjId, room: ObjId, kind: NarrativeKind, text: &str) {
    // No connected player in the room ⇒ no wakes, no tokens.
    if players_present(ctx.w, room).is_empty() {
        return;
    }
    let now = now_ms(ctx.w);
    let speaker = display_name(ctx.w, who);
    let line = format_speech(&speaker, kind, text);
    let sk = speech_kind(kind);
    // Loop-breaker (cost control): an NPC's own speech (re-emitted as a `Said`
    // event by `say_as_npc`) is recorded into other NPCs' memory buffers for
    // context, but it must **never wake** another NPC — otherwise two enabled
    // NPCs sharing a room ping-pong LLM turns forever while a single idle player
    // is merely present. Every NPC turn stays player-driven: only player-sourced
    // speech enqueues a waking stimulus.
    let speaker_is_npc = ctx.w.get(who).is_some_and(|o| o.kind == ObjKind::Npc);
    let targets = npcs_in(ctx.w, room);
    for npc in targets {
        if npc == who {
            continue;
        }
        let Some(profile) = read_profile(ctx.w, npc) else {
            continue;
        };
        if !profile.enabled {
            continue;
        }
        if speaker_is_npc {
            // Context only — no wake.
            record_self(ctx, npc, &line);
            continue;
        }
        let directed = mentions(ctx.w, npc, text);
        let stim = if directed {
            Stimulus::SpokenTo {
                by: who,
                text: text.to_string(),
                kind: sk,
            }
        } else {
            Stimulus::HeardInRoom {
                by: who,
                text: text.to_string(),
            }
        };
        remember_and_enqueue(ctx, npc, stim, &line, now);
    }
}

fn on_moved(ctx: &mut Ctx<'_>, who: ObjId, from: Option<ObjId>, to: ObjId) {
    if !is_connected_player(ctx.w, who) {
        return;
    }
    let now = now_ms(ctx.w);
    let name = display_name(ctx.w, who);
    // Arrival in `to`.
    for npc in npcs_in(ctx.w, to) {
        if npc == who || !enabled_npc(ctx.w, npc) {
            continue;
        }
        remember_and_enqueue(
            ctx,
            npc,
            Stimulus::PlayerArrived { who },
            &format!("{name} arrives."),
            now,
        );
    }
    // Departure from `from`.
    if let Some(from) = from {
        for npc in npcs_in(ctx.w, from) {
            if npc == who || !enabled_npc(ctx.w, npc) {
                continue;
            }
            remember_and_enqueue(
                ctx,
                npc,
                Stimulus::PlayerLeft { who },
                &format!("{name} leaves."),
                now,
            );
        }
    }
}

fn on_presence(ctx: &mut Ctx<'_>, who: ObjId, connected: bool) {
    let Some(room) = ctx.w.get(who).and_then(|o| o.location) else {
        return;
    };
    if ctx.w.get(who).is_none_or(|o| o.kind != ObjKind::Player) {
        return;
    }
    let now = now_ms(ctx.w);
    let name = display_name(ctx.w, who);
    let (stim_line, verb): (fn(ObjId) -> Stimulus, &str) = if connected {
        (|who| Stimulus::PlayerArrived { who }, "arrives")
    } else {
        (|who| Stimulus::PlayerLeft { who }, "leaves")
    };
    for npc in npcs_in(ctx.w, room) {
        if !enabled_npc(ctx.w, npc) {
            continue;
        }
        remember_and_enqueue(ctx, npc, stim_line(who), &format!("{name} {verb}."), now);
    }
}

/// Push a transcript line into the NPC's buffer (flagging overflow) and enqueue
/// the stimulus.
fn remember_and_enqueue(ctx: &mut Ctx<'_>, npc: ObjId, stim: Stimulus, line: &str, now: i64) {
    let ts = ctx.w.now();
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        let rt = h.runtime_mut(npc);
        rt.memory.push(ts, line);
        if rt.memory.is_overflowing() {
            rt.summarize_pending = true;
        }
        h.enqueue(npc, stim, now);
    }
}

// ----------------------------------------------------------------------
// Tick: fire ready NPCs
// ----------------------------------------------------------------------

struct NpcTick {
    grounding: Arc<dyn AgentGrounding>,
}

impl mush_core::module::TickHandler for NpcTick {
    fn tick(&self, ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::module::SceneId>) {
        let now = now_ms(ctx.w);
        let pending = match ctx.state::<NpcHostState>() {
            Some(h) => h.pending(),
            None => return,
        };
        for npc in pending {
            self.maybe_fire(ctx, npc, now);
        }
    }
}

impl NpcTick {
    fn maybe_fire(&self, ctx: &mut Ctx<'_>, npc: ObjId, now: i64) {
        let Some(profile) = read_profile(ctx.w, npc) else {
            drop_mailbox(ctx, npc);
            return;
        };
        if !profile.enabled {
            drop_mailbox(ctx, npc);
            return;
        }
        let Some(room) = ctx.w.get(npc).and_then(|o| o.location) else {
            return;
        };
        let present = players_present(ctx.w, room);

        // Decide the turn kind + readiness from the runtime.
        let (has_mailbox, summarize, ready) = {
            let Some(h) = ctx.state::<NpcHostState>() else {
                return;
            };
            let Some(rt) = h.runtime(npc) else {
                return;
            };
            let coalesce = h.config.coalesce;
            let has_mailbox = !rt.mailbox.is_empty();
            let window = rt.window_elapsed(now, coalesce);
            let cooldown = rt.cooldown_ok(now, profile.cooldown(), rt.has_direct());
            let ready = has_mailbox && window && cooldown;
            (has_mailbox, rt.summarize_pending, ready)
        };

        // No players present: never fire a paid *dialogue* turn into an empty
        // room. Drop the pending dialogue batch unconditionally (its stimuli are
        // stale — the players who caused them have left); only a cheap
        // background summarize may proceed, independent of occupancy. A pending
        // summarize must not smuggle a dialogue turn past this guard.
        if present.is_empty() {
            drop_mailbox(ctx, npc);
            if summarize {
                self.fire_summarize(ctx, npc, &profile, room, &present, now);
            }
            return;
        }

        if has_mailbox && ready {
            self.fire_dialogue(ctx, npc, &profile, room, &present, now);
        } else if summarize && !has_mailbox {
            self.fire_summarize(ctx, npc, &profile, room, &present, now);
        }
    }

    fn fire_dialogue(
        &self,
        ctx: &mut Ctx<'_>,
        npc: ObjId,
        profile: &AgentProfile,
        room: ObjId,
        present: &[ObjId],
        now: i64,
    ) {
        let correlation = ctx.w.next_correlation_id();
        let batch = match ctx.state_mut::<NpcHostState>() {
            Some(h) => h.begin_turn(npc, now, correlation),
            None => return,
        };
        let snap = build_dialogue_snapshot(
            ctx,
            self.grounding.as_ref(),
            npc,
            profile,
            room,
            present,
            &batch,
            correlation,
        );
        emit_thinking(ctx, npc, room);
        defer_turn(ctx, npc, correlation, &snap);
    }

    fn fire_summarize(
        &self,
        ctx: &mut Ctx<'_>,
        npc: ObjId,
        profile: &AgentProfile,
        room: ObjId,
        present: &[ObjId],
        now: i64,
    ) {
        let _ = now;
        let correlation = ctx.w.next_correlation_id();
        let fold = match ctx.state_mut::<NpcHostState>() {
            Some(h) => h.begin_summarize(npc, correlation),
            None => return,
        };
        if fold.is_empty() {
            // Nothing to fold after all; clear in-flight.
            if let Some(h) = ctx.state_mut::<NpcHostState>() {
                h.finish_turn(npc, now);
            }
            return;
        }
        let snap = build_summarize_snapshot(ctx, npc, profile, room, present, fold, correlation);
        defer_turn(ctx, npc, correlation, &snap);
    }
}

// ----------------------------------------------------------------------
// Async completion: apply typed actions, re-validated
// ----------------------------------------------------------------------

struct NpcCompletion {
    grounding: Arc<dyn AgentGrounding>,
}

impl AsyncCompletion for NpcCompletion {
    fn on_complete(&self, ctx: &mut Ctx<'_>, correlation: u64, payload: &str) {
        // Clear in-flight on the framework-supplied NPC id (`ctx.actor` is the
        // effect's actor — the real NPC) BEFORE decoding, so a malformed or
        // wrong-NPC payload can never wedge the agent in-flight forever. We do
        // not trust `outcome.npc()` for this: the undecodable-snapshot branch
        // returns `Nothing { npc: ObjId(0) }`, which would clear the wrong
        // runtime and leave the real NPC stuck.
        let npc = ctx.actor;
        let now = now_ms(ctx.w);
        if let Some(h) = ctx.state_mut::<NpcHostState>() {
            h.finish_turn(npc, now);
        }
        let outcome: TurnOutcome = match serde_json::from_str(payload) {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(correlation, error = %e, "npc: bad completion payload");
                return;
            }
        };
        // Re-validate against current state.
        if ctx.w.get(npc).is_none_or(|o| o.kind != ObjKind::Npc) {
            return;
        }
        let Some(room) = ctx.w.get(npc).and_then(|o| o.location) else {
            return;
        };

        match outcome {
            TurnOutcome::Nothing { .. } => {
                // Silent + already audited by the LlmService call.
            }
            TurnOutcome::Degraded { line, reason, .. } => {
                // Fallback lines are staff-authored, but hold them to the same
                // output-hygiene invariant as every other NPC-said line
                // (single line, no control/ANSI/bidi, length-capped).
                let line = crate::action::sanitize_line(&line, crate::action::SAY_CAP);
                if !line.is_empty() {
                    say_as_npc(ctx, npc, room, NarrativeKind::Say, &line);
                }
                notify_staff_once(ctx, npc, &reason);
            }
            TurnOutcome::Summary { summary, .. } => {
                let _ = ctx.w.set_attr(npc, SUMMARY_ATTR, Attr::text(&summary, npc));
                if let Some(h) = ctx.state_mut::<NpcHostState>() {
                    h.runtime_mut(npc).memory.summary = summary;
                }
            }
            TurnOutcome::Ok { actions, .. } => {
                for aa in actions {
                    apply_action(self.grounding.as_ref(), ctx, npc, room, &aa);
                }
            }
        }
    }
}

/// Apply one typed action, re-validated in the world task.
fn apply_action(
    grounding: &dyn AgentGrounding,
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    room: ObjId,
    aa: &AppliedAction,
) {
    match &aa.action {
        NpcAction::Say { text } => say_as_npc(ctx, npc, room, NarrativeKind::Say, text),
        NpcAction::Pose { text } => say_as_npc(ctx, npc, room, NarrativeKind::Pose, text),
        NpcAction::Move { exit } => {
            // Re-validated exactly like a player's move: a locked / vanished
            // exit simply fails.
            if let Some(e) = find_exit(ctx.w, npc, exit) {
                traverse_exit(ctx.reg, ctx.w, npc, e, ctx.out);
            }
            record_self(ctx, npc, &format!("(tries to leave via {exit})"));
        }
        NpcAction::LookAt { .. } => {
            // Resolved off-loop into a tool result; no world mutation on-loop.
        }
        NpcAction::Remember { subject, note } => {
            let now = ctx.w.now();
            let rec = match ctx.state_mut::<NpcHostState>() {
                Some(h) => Some(h.long_term.remember(npc, subject, note, now)),
                None => None,
            };
            if let Some(rec) = rec {
                persist_memory(ctx, &rec);
            }
        }
        NpcAction::UseAbility { name, input } => {
            // Executed on the world loop by the ruleset: it reads the NPC's own
            // sheet, rolls on the shared RNG + roll log, and emits the result
            // to the room itself. The model only requested it.
            match grounding.invoke_ability(ctx.w, ctx.out, npc, name, input.clone()) {
                Ok(outcome) => {
                    record_self(ctx, npc, &format!("(used {name}: {})", outcome.summary))
                }
                Err(e) => {
                    tracing::debug!(npc = npc.0, %name, error = %e, "npc ability refused");
                }
            }
        }
        NpcAction::DoNothing { .. } => {}
    }
}

// ----------------------------------------------------------------------
// @agent staff command
// ----------------------------------------------------------------------

fn cmd_agent(grounding: &dyn AgentGrounding, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // `create` runs before target resolution (the target does not exist yet).
    if ctx.switches.has("create") {
        return agent_create(ctx);
    }

    // Every other switch names an existing NPC. For `@agent/set <npc>/<field>`
    // the target is the path segment before the first `/`.
    let lhs = ctx.args.lhs().to_string();
    let target = lhs.split('/').next().unwrap_or("").trim();
    if target.is_empty() {
        return Err(CmdError::msg("Usage: @agent <npc>  (see help @agent)."));
    }
    let npc = mush_core::match_object(ctx.w, ctx.actor, target)?;
    if ctx.w.get(npc).is_none_or(|o| o.kind != ObjKind::Npc) {
        return Err(CmdError::msg("That is not an NPC."));
    }

    if ctx.switches.has("set") {
        return agent_set(ctx, npc, &lhs);
    }

    if ctx.switches.has("sheet") {
        return agent_sheet(grounding, ctx, npc);
    }

    if ctx.switches.has("poke") {
        let note = ctx
            .args
            .rhs()
            .map(str::to_string)
            .filter(|n| !n.trim().is_empty())
            .ok_or_else(|| CmdError::msg("Usage: @agent/poke <npc>=<note>."))?;
        let now = now_ms(ctx.w);
        let by = ctx.actor;
        if let Some(h) = ctx.state_mut::<NpcHostState>() {
            h.enqueue(npc, Stimulus::StaffPoke { by, note }, now);
        }
        ctx.out.info(ctx.actor, "Poked the agent.");
        return Ok(());
    }

    if ctx.switches.has("enable") || ctx.switches.has("disable") {
        let enable = ctx.switches.has("enable");
        let mut profile = read_profile_raw(ctx.w, npc).unwrap_or_default();
        profile.enabled = enable;
        store_profile(ctx, npc, &profile)?;
        ctx.out.info(
            ctx.actor,
            format!("Agent {}.", if enable { "enabled" } else { "disabled" }),
        );
        return Ok(());
    }

    // Default: show profile + status.
    let profile = read_profile_raw(ctx.w, npc).unwrap_or_default();
    let name = display_name(ctx.w, npc);
    ctx.out.info(
        ctx.actor,
        format!(
            "Agent {name} (#{}) — tier {:?}, {}.",
            npc.0,
            profile.tier,
            if profile.enabled {
                "enabled"
            } else {
                "disabled"
            }
        ),
    );
    if !profile.persona.trim().is_empty() {
        ctx.out
            .info(ctx.actor, format!("  Persona: {}", profile.persona));
    }
    if !profile.goals.is_empty() {
        ctx.out
            .info(ctx.actor, format!("  Goals: {}", profile.goals.join("; ")));
    }
    let status = ctx
        .state::<NpcHostState>()
        .and_then(|h| h.runtime(npc))
        .map(|rt| {
            format!(
                "  Runtime: {} pending, {}, {} buffered.",
                rt.mailbox.len(),
                if rt.in_flight { "in flight" } else { "idle" },
                rt.memory.events.len()
            )
        })
        .unwrap_or_else(|| "  Runtime: no activity yet.".to_string());
    ctx.out.info(ctx.actor, status);
    // Persisted memory (survives reboots): the rolling summary attribute + the
    // count of durable, subject-keyed memories reloaded into the long-term store.
    let summary = read_summary(ctx.w, npc);
    if !summary.trim().is_empty() {
        ctx.out
            .info(ctx.actor, format!("  Summary: {}", summary.trim()));
    }
    let mem_subjects = ctx
        .state::<NpcHostState>()
        .map_or(0, |h| h.long_term.subject_count(npc));
    if mem_subjects > 0 {
        let plural = if mem_subjects == 1 {
            "subject"
        } else {
            "subjects"
        };
        ctx.out.info(
            ctx.actor,
            format!("  Durable memories: {mem_subjects} {plural}."),
        );
    }
    Ok(())
}

/// `@agent/create <name>` — create a first-class NPC object in the staffer's
/// room and seed a default (Minor, enabled) profile. The NPC does nothing until
/// staff give it a persona (`@agent/set`) and, for rolls, a sheet
/// (`@agent/sheet`).
fn agent_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let name = ctx.args.lhs().trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg("Usage: @agent/create <name>."));
    }
    if name.contains('/') || name.contains('=') {
        return Err(CmdError::msg(
            "An NPC name can't contain '/' or '='. Usage: @agent/create <name>.",
        ));
    }
    let room = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or_else(|| CmdError::msg("You are nowhere; go somewhere before creating an NPC."))?;
    let npc = ctx
        .w
        .create(ObjKind::Npc, &name, ctx.actor, Some(room))
        .map_err(|e| CmdError::msg(format!("Could not create the NPC: {e}")))?;
    let profile = AgentProfile::default();
    store_profile(ctx, npc, &profile)?;
    ctx.out.info(
        ctx.actor,
        format!(
            "Created NPC agent {name} (#{}). Next: @agent/set {name}/persona=..., \
             @agent/sheet {name} for a rollable sheet.",
            npc.0
        ),
    );
    Ok(())
}

/// `@agent/set <npc>/<field>=<value>` — validated edit of the staff-authored
/// [`AgentProfile`]. List fields split on `;`; an empty value clears a field.
fn agent_set(ctx: &mut Ctx<'_>, npc: ObjId, lhs: &str) -> Result<(), CmdError> {
    let field = lhs
        .split_once('/')
        .map(|(_, f)| f.trim())
        .filter(|f| !f.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: @agent/set <npc>/<field>=<value>."))?;
    let value = ctx.args.rhs().unwrap_or("").trim().to_string();
    let mut profile = read_profile_raw(ctx.w, npc).unwrap_or_default();
    let field_l = field.to_ascii_lowercase();
    match field_l.as_str() {
        "persona" => profile.persona = value.clone(),
        "background" => profile.background = value.clone(),
        "speech" | "speech_style" => profile.speech_style = value.clone(),
        "goals" => profile.goals = split_list(&value),
        "fallback" | "fallback_lines" => profile.fallback_lines = split_list(&value),
        "knows" => profile.knowledge.knows = split_list(&value),
        "does_not_know" | "unknown" => profile.knowledge.does_not_know = split_list(&value),
        "tier" => profile.tier = parse_tier(&value)?,
        "cooldown" | "cooldown_s" => profile.cooldown_s = parse_opt_u32(&value)?,
        "daily_tokens" | "budget" => profile.daily_tokens = parse_opt_u64(&value)?,
        "enabled" => profile.enabled = parse_bool(&value)?,
        _ => {
            return Err(CmdError::msg(format!(
                "Unknown agent field {field:?}. Fields: persona, background, speech, goals, \
                 fallback, knows, does_not_know, tier, cooldown, daily_tokens, enabled."
            )));
        }
    }
    store_profile(ctx, npc, &profile)?;
    let name = display_name(ctx.w, npc);
    ctx.out.info(
        ctx.actor,
        if value.is_empty() && is_clearable(&field_l) {
            format!("Cleared {field} on {name}.")
        } else {
            format!("Set {field} on {name}.")
        },
    );
    Ok(())
}

/// `@agent/sheet <npc>[=<template>]` — give the NPC a ruleset character sheet
/// (default `mortal`) through the loaded ruleset's normal validated path, so
/// `wod_roll` (etc.) has real traits to read. Delegates to the driver's
/// grounding adapter; a dialogue-only world reports that no sheet ruleset is
/// loaded.
fn agent_sheet(
    grounding: &dyn AgentGrounding,
    ctx: &mut Ctx<'_>,
    npc: ObjId,
) -> Result<(), CmdError> {
    let template = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("mortal")
        .to_string();
    let by = ctx.actor;
    match grounding.init_sheet(ctx.w, npc, &template, by) {
        Ok(msg) => {
            let name = display_name(ctx.w, npc);
            ctx.out.info(
                ctx.actor,
                format!("{msg} Fill it in with @sheet/set {name}/<field>=<value>."),
            );
            Ok(())
        }
        Err(e) => Err(CmdError::msg(e)),
    }
}

/// Split a `;`- (or newline-) separated list value, trimming and dropping
/// empties. An empty input yields an empty list (clears the field).
fn split_list(value: &str) -> Vec<String> {
    value
        .split([';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether a field is a list/text field that an empty value clears (for the
/// confirmation wording only).
fn is_clearable(field: &str) -> bool {
    matches!(
        field,
        "persona"
            | "background"
            | "speech"
            | "speech_style"
            | "goals"
            | "fallback"
            | "fallback_lines"
            | "knows"
            | "does_not_know"
            | "unknown"
            | "cooldown"
            | "cooldown_s"
            | "daily_tokens"
            | "budget"
    )
}

fn parse_tier(value: &str) -> Result<crate::profile::NpcTier, CmdError> {
    use crate::profile::NpcTier;
    match value.trim().to_ascii_lowercase().as_str() {
        "major" => Ok(NpcTier::Major),
        "minor" | "" => Ok(NpcTier::Minor),
        "extra" => Ok(NpcTier::Extra),
        other => Err(CmdError::msg(format!(
            "Unknown tier {other:?}. Choose major, minor, or extra."
        ))),
    }
}

fn parse_bool(value: &str) -> Result<bool, CmdError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" | "enabled" => Ok(true),
        "false" | "off" | "no" | "0" | "disabled" | "" => Ok(false),
        other => Err(CmdError::msg(format!(
            "Expected a yes/no value for enabled, got {other:?}."
        ))),
    }
}

/// Parse an optional u32: empty / `none` / `default` / `0` clears the override.
fn parse_opt_u32(value: &str) -> Result<Option<u32>, CmdError> {
    let v = value.trim().to_ascii_lowercase();
    if v.is_empty() || v == "none" || v == "default" || v == "0" {
        return Ok(None);
    }
    v.parse::<u32>().map(Some).map_err(|_| {
        CmdError::msg(format!(
            "Expected a whole number of seconds, got {value:?}."
        ))
    })
}

/// Parse an optional u64: empty / `none` / `default` / `0` clears the override.
fn parse_opt_u64(value: &str) -> Result<Option<u64>, CmdError> {
    let v = value.trim().to_ascii_lowercase();
    if v.is_empty() || v == "none" || v == "default" || v == "0" {
        return Ok(None);
    }
    v.parse::<u64>()
        .map(Some)
        .map_err(|_| CmdError::msg(format!("Expected a whole number of tokens, got {value:?}.")))
}

// ----------------------------------------------------------------------
// Snapshot assembly
// ----------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn build_dialogue_snapshot(
    ctx: &Ctx<'_>,
    grounding: &dyn AgentGrounding,
    npc: ObjId,
    profile: &AgentProfile,
    room: ObjId,
    present: &[ObjId],
    batch: &[Stimulus],
    correlation: u64,
) -> TurnSnapshot {
    let scene = build_scene(ctx, npc, room, present);
    let agent_profile = grounding.profile(ctx.w, npc);
    let present_names: Vec<String> = present.iter().map(|&p| display_name(ctx.w, p)).collect();

    let (summary, transcript, recalled) = memory_context(ctx, npc, &present_names);
    let stimuli = batch.iter().map(|s| render_line(ctx.w, s)).collect();
    let actor = batch.iter().find_map(Stimulus::subject);

    TurnSnapshot {
        kind: TurnKind::Dialogue,
        npc,
        npc_name: display_name(ctx.w, npc),
        profile: profile.clone(),
        agent_profile,
        scene,
        summary,
        recalled,
        transcript,
        stimuli,
        actor,
        fold_events: Vec::new(),
        seed: correlation,
    }
}

fn build_summarize_snapshot(
    ctx: &Ctx<'_>,
    npc: ObjId,
    profile: &AgentProfile,
    room: ObjId,
    present: &[ObjId],
    fold: Vec<String>,
    correlation: u64,
) -> TurnSnapshot {
    let scene = build_scene(ctx, npc, room, present);
    let summary = read_summary(ctx.w, npc);
    TurnSnapshot {
        kind: TurnKind::Summarize,
        npc,
        npc_name: display_name(ctx.w, npc),
        profile: profile.clone(),
        agent_profile: None,
        scene,
        summary,
        recalled: Vec::new(),
        transcript: String::new(),
        stimuli: Vec::new(),
        actor: None,
        fold_events: fold,
        seed: correlation,
    }
}

fn build_scene(ctx: &Ctx<'_>, npc: ObjId, room: ObjId, present: &[ObjId]) -> SceneView {
    let room_name = display_name(ctx.w, room);
    let room_desc = attr_text(ctx.w, room, "short_desc")
        .or_else(|| attr_text(ctx.w, room, "desc"))
        .unwrap_or_default();
    let present = present
        .iter()
        .filter(|&&p| p != npc)
        .map(|&p| PresentPlayer {
            name: display_name(ctx.w, p),
            short_desc: attr_text(ctx.w, p, "short_desc").unwrap_or_default(),
        })
        .collect();
    let exits = ctx
        .w
        .exits_of(room)
        .iter()
        .filter_map(|&e| ctx.w.get(e).map(|o| primary(&o.name).to_string()))
        .collect();
    let weather = weather_digest(ctx, room);
    SceneView {
        room_name,
        room_desc,
        present,
        time: ctx.ic_datetime(),
        weather,
        exits,
    }
}

/// Read the summary + recent transcript + recalled memories for the prompt.
fn memory_context(
    ctx: &Ctx<'_>,
    npc: ObjId,
    present_names: &[String],
) -> (String, String, Vec<RecalledMemory>) {
    let attr_summary = read_summary(ctx.w, npc);
    let Some(h) = ctx.state::<NpcHostState>() else {
        return (attr_summary, String::new(), Vec::new());
    };
    let transcript = h
        .runtime(npc)
        .map(|rt| rt.memory.transcript(h.config.transcript_chars))
        .unwrap_or_default();
    let summary = h
        .runtime(npc)
        .map(|rt| rt.memory.summary.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(attr_summary);
    let recalled = h
        .long_term
        .recall(npc, present_names, h.config.recall_top_n)
        .into_iter()
        .map(|r| RecalledMemory {
            subject: r.subject,
            notes: r.notes,
        })
        .collect();
    (summary, transcript, recalled)
}

fn render_line(w: &World, s: &Stimulus) -> StimulusLine {
    match s {
        Stimulus::SpokenTo { by, text, kind } => StimulusLine {
            who: display_name(w, *by),
            text: text.clone(),
            directed: true,
            tag: speech_tag(*kind).to_string(),
        },
        Stimulus::HeardInRoom { by, text } => StimulusLine {
            who: display_name(w, *by),
            text: text.clone(),
            directed: false,
            tag: "say".to_string(),
        },
        Stimulus::PlayerArrived { who } => StimulusLine {
            who: display_name(w, *who),
            text: String::new(),
            directed: false,
            tag: "arrived".to_string(),
        },
        Stimulus::PlayerLeft { who } => StimulusLine {
            who: display_name(w, *who),
            text: String::new(),
            directed: false,
            tag: "left".to_string(),
        },
        Stimulus::AmbientTimer => StimulusLine {
            who: String::new(),
            text: String::new(),
            directed: false,
            tag: "ambient".to_string(),
        },
        Stimulus::GameEvent { digest } => StimulusLine {
            who: String::new(),
            text: digest.clone(),
            directed: false,
            tag: "event".to_string(),
        },
        Stimulus::StaffPoke { by, note } => StimulusLine {
            who: display_name(w, *by),
            text: note.clone(),
            directed: true,
            tag: "poke".to_string(),
        },
    }
}

// ----------------------------------------------------------------------
// Emission helpers (NPC speech through the normal formatter)
// ----------------------------------------------------------------------

/// Say/pose as the NPC: the normal `Narrative` event with the NPC's *true*
/// name (it cannot forge another speaker), plus the `Said` observer signal so
/// other agents / comsys logging hear it, plus a self-memory line.
fn say_as_npc(ctx: &mut Ctx<'_>, npc: ObjId, room: ObjId, kind: NarrativeKind, text: &str) {
    emit_room(ctx, room, kind, npc, text);
    ctx.emit_world_event(&WorldEvent::Said {
        who: npc,
        room,
        kind,
        text: text.to_string(),
    });
    let name = display_name(ctx.w, npc);
    record_self(ctx, npc, &format_speech(&name, kind, text));
}

fn emit_room(ctx: &mut Ctx<'_>, room: ObjId, kind: NarrativeKind, actor: ObjId, text: &str) {
    let ev = OutputEvent::Narrative {
        kind,
        actor: Some(obj_ref(ctx.w, actor)),
        text: RichText::plain(text),
    };
    ctx.out.to_room(room, Vec::new(), ev);
}

fn record_self(ctx: &mut Ctx<'_>, npc: ObjId, line: &str) {
    let ts = ctx.w.now();
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        let rt = h.runtime_mut(npc);
        rt.memory.push(ts, line);
        if rt.memory.is_overflowing() {
            rt.summarize_pending = true;
        }
    }
}

fn emit_thinking(ctx: &mut Ctx<'_>, npc: ObjId, room: ObjId) {
    let panel = ctx.state::<NpcHostState>().and_then(|h| h.thinking.clone());
    if let Some(panel) = panel {
        let data = serde_json::json!({
            "npc": npc.to_string(),
            "name": display_name(ctx.w, npc),
        });
        let aud = mush_core::Audience::Room {
            room,
            exclude: Vec::new(),
        };
        panel.keyframe_to(ctx, aud, data);
    }
}

fn notify_staff_once(ctx: &mut Ctx<'_>, npc: ObjId, reason: &str) {
    let already = ctx
        .state::<NpcHostState>()
        .and_then(|h| h.runtime(npc))
        .is_some_and(|rt| rt.degraded_notified);
    if already {
        return;
    }
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        h.runtime_mut(npc).degraded_notified = true;
    }
    let name = display_name(ctx.w, npc);
    let msg = format!(
        "NPC {name} (#{}) degraded to a fallback line: {reason}",
        npc.0
    );
    let staff: Vec<ObjId> = ctx
        .w
        .connected()
        .filter(|&a| perm::has_role(ctx.w, a, Role::Royalty))
        .collect();
    for s in staff {
        ctx.out.sys_to_actor(s, "npc", msg.clone());
    }
}

fn defer_turn(ctx: &mut Ctx<'_>, npc: ObjId, correlation: u64, snap: &TurnSnapshot) {
    let payload = match serde_json::to_string(snap) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(npc = npc.0, error = %e, "npc: unserializable snapshot; turn dropped");
            let nm = now_ms(ctx.w);
            if let Some(h) = ctx.state_mut::<NpcHostState>() {
                h.finish_turn(npc, nm);
            }
            return;
        }
    };
    ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
        module: SmolStr::new(NS),
        actor: npc,
        correlation,
        payload,
    }));
}

fn persist_memory(ctx: &mut Ctx<'_>, rec: &MemoryRecord) {
    let value = match serde_json::to_value(rec) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "npc: could not serialize memory record");
            return;
        }
    };
    ctx.persist(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::JsonSet {
            key: record_key(rec.npc, &rec.subject),
            value,
        },
    });
}

fn store_profile(ctx: &mut Ctx<'_>, npc: ObjId, profile: &AgentProfile) -> Result<(), CmdError> {
    let value = serde_json::to_value(profile)
        .map_err(|e| CmdError::msg(format!("Could not serialize the profile: {e}")))?;
    ctx.w
        .set_attr(npc, PROFILE_ATTR, Attr::json(value, ctx.actor))?;
    Ok(())
}

// ----------------------------------------------------------------------
// small world helpers
// ----------------------------------------------------------------------

/// Milliseconds derived from the world's (second-granularity) clock — the one
/// time base both the observers and the tick use.
fn now_ms(w: &World) -> i64 {
    w.now().saturating_mul(1000)
}

fn drop_mailbox(ctx: &mut Ctx<'_>, npc: ObjId) {
    if let Some(h) = ctx.state_mut::<NpcHostState>() {
        h.drop_mailbox(npc);
    }
}

fn read_profile(w: &World, npc: ObjId) -> Option<AgentProfile> {
    read_profile_raw(w, npc)
}

fn read_profile_raw(w: &World, npc: ObjId) -> Option<AgentProfile> {
    let v = w.attr(npc, PROFILE_ATTR)?.as_json()?;
    serde_json::from_value(v.clone()).ok()
}

fn read_summary(w: &World, npc: ObjId) -> String {
    w.attr(npc, SUMMARY_ATTR)
        .and_then(|a| a.as_text())
        .unwrap_or_default()
        .to_string()
}

fn enabled_npc(w: &World, npc: ObjId) -> bool {
    read_profile(w, npc).is_some_and(|p| p.enabled)
}

fn players_present(w: &World, room: ObjId) -> Vec<ObjId> {
    w.contents_of(room)
        .iter()
        .copied()
        .filter(|&o| w.get(o).is_some_and(|x| x.kind == ObjKind::Player) && w.is_connected(o))
        .collect()
}

fn is_connected_player(w: &World, who: ObjId) -> bool {
    w.get(who).is_some_and(|o| o.kind == ObjKind::Player) && w.is_connected(who)
}

fn npcs_in(w: &World, room: ObjId) -> Vec<ObjId> {
    w.contents_of(room)
        .iter()
        .copied()
        .filter(|&o| w.get(o).is_some_and(|x| x.kind == ObjKind::Npc))
        .collect()
}

/// Whether `text` addresses `npc` by (the first token of) its name.
fn mentions(w: &World, npc: ObjId, text: &str) -> bool {
    let Some(o) = w.get(npc) else { return false };
    let name = primary(&o.name).to_lowercase();
    if name.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|tok| tok == name)
        || o.aliases.iter().any(|a| {
            let a = a.to_lowercase();
            !a.is_empty()
                && lower
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|tok| tok == a)
        })
}

fn display_name(w: &World, id: ObjId) -> String {
    w.get(id)
        .map(|o| primary(&o.name).to_string())
        .unwrap_or_else(|| format!("#{}", id.0))
}

fn primary(name: &str) -> &str {
    name.split(';').next().unwrap_or(name).trim()
}

fn attr_text(w: &World, obj: ObjId, name: &str) -> Option<String> {
    w.attr(obj, name)
        .and_then(|a| a.as_text())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

fn weather_digest(ctx: &Ctx<'_>, room: ObjId) -> Option<String> {
    let (_m, provider) = ctx.reg.weather_query()?;
    let wx = provider.current(ctx.w, room)?;
    Some(wx.state.to_string())
}

fn speech_kind(kind: NarrativeKind) -> SpeechKind {
    match kind {
        NarrativeKind::Pose => SpeechKind::Pose,
        NarrativeKind::SemiPose => SpeechKind::SemiPose,
        NarrativeKind::Emit | NarrativeKind::Spoof => SpeechKind::Emit,
        _ => SpeechKind::Say,
    }
}

fn speech_tag(kind: SpeechKind) -> &'static str {
    match kind {
        SpeechKind::Say => "say",
        SpeechKind::Pose => "pose",
        SpeechKind::SemiPose => "semipose",
        SpeechKind::Emit => "emit",
    }
}

/// A script-formatted transcript line (matches the plain-text render shapes).
fn format_speech(name: &str, kind: NarrativeKind, text: &str) -> String {
    match kind {
        NarrativeKind::Say => format!("{name} says, \"{text}\""),
        NarrativeKind::Pose => format!("{name} {text}"),
        NarrativeKind::SemiPose => format!("{name}{text}"),
        _ => text.to_string(),
    }
}
