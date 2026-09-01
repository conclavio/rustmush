//! # mush-scenes
//!
//! Durable narrative scenes and the **timestop** driver (design:
//! `docs/scenes_factions_knowledge.md` §1).
//!
//! A scene is an explicit, durable envelope around play. Cut 1 ships the
//! timestop: `+timestop` promotes the actor's room into an **exclusive,
//! turn-based** scene that
//!
//! - freezes its members in place (a [`HookPoint::MoveAttempt`] veto);
//! - suppresses free-running NPC turns for its participants (via the core
//!   exclusive-scene index — NPCs act only when handed a scene turn);
//! - tracks initiative order, rounds, and a **scene-local IC clock** decoupled
//!   from the global (ratio-scaled) clock — a fight played over real days
//!   occupies seconds of IC time;
//! - has a durable id that never restarts and **survives reboots**,
//!   rehydrating as `Paused(ServerRestart)` so a facilitator resumes it
//!   deliberately (turns never auto-advance across a restart).
//!
//! ## How it hangs off the module API
//!
//! - **State** is one [`ScenesState`] slot; every mutation mirrors to
//!   `mod:scenes:*` docs (`scene:{uid}` per record, `meta` for the id
//!   allocator) — write-through with command atomicity.
//! - **Exclusivity** is published to core via
//!   [`World::set_exclusive_scene_member`](mush_core::World::set_exclusive_scene_member):
//!   the core index is what the movement gate and the NPC wake loop read, so
//!   neither depends on this crate's types.
//! - **Boot** rehydrates every non-closed scene as
//!   [`Paused`](data::SceneStatus::Paused)`(`[`ServerRestart`](data::PauseReason::ServerRestart)`)`
//!   and re-publishes its held members to the core index, so a restart keeps
//!   the room frozen until a facilitator resumes.
//!
//! What cut 1 does NOT yet do: route mechanical commands through the turn
//! (out-of-turn `+roll` is not rejected), record an immutable event log, or
//! roll ruleset initiative — those layer onto the same record next.

mod commands;
mod data;

use std::sync::Arc;

use mush_core::module::{
    EventHook, GameModule, HookPoint, ModError, ModuleBootData, ModuleCtx, Registrar, WorldEvent,
    WorldEventKind,
};
use mush_core::output::display_name;
use mush_core::{
    CommandGate, Ctx, ModuleOp, ObjId, ObjKind, Registry, Role, SceneClass, World, is_log_key, perm,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use smol_str::SmolStr;

pub use data::{
    KIND_TIMESTOP, NS, PauseReason, SceneEvent, SceneEventKind, SceneMode, SceneRecord, SceneStatus,
    SceneUid, ScenesState, scene_key,
};

use data::{persist_scene, META_KEY};

/// Whether `who` is scene-management staff (Royalty and up) — the same bar the
/// commands use, so a staffer who can drive a scene is also not frozen by it.
fn is_scene_staff(w: &World, who: ObjId) -> bool {
    perm::has_role(w, who, Role::Royalty)
}

/// The scenes game module. Hand one to `Engine::with_modules`.
pub struct ScenesModule;

impl ScenesModule {
    /// A fresh module instance.
    #[must_use]
    pub fn new() -> ScenesModule {
        ScenesModule
    }
}

impl Default for ScenesModule {
    fn default() -> Self {
        ScenesModule::new()
    }
}

impl GameModule for ScenesModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(ScenesState::default());
        reg.hook(HookPoint::MoveAttempt, Arc::new(TimestopMoveGate));
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        // The event log: IC narrative and room join/leave inside an active
        // scene are recorded to its ledger (lifecycle events are recorded by
        // the commands themselves). `Said` is subscribed per flavor.
        for kind in [
            NarrativeKind::Say,
            NarrativeKind::Pose,
            NarrativeKind::SemiPose,
            NarrativeKind::Emit,
            NarrativeKind::Spoof,
        ] {
            reg.subscribe(WorldEventKind::Said { kind }, Arc::new(on_said));
        }
        reg.subscribe(WorldEventKind::Moved, Arc::new(on_moved));
        // Mechanical resolutions (dice rolls) recorded into the scene log.
        reg.subscribe(WorldEventKind::Rolled, Arc::new(on_rolled));
        // Turn-gating: a mechanical command runs only on the actor's turn.
        reg.command_gate(Arc::new(TurnGate));
        commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        load_boot(ctx, data)
    }
}

/// The [`HookPoint::MoveAttempt`] veto: a member of a non-closed **exclusive**
/// scene cannot leave its anchor on their own. Staff (Royalty+) are never
/// frozen, matching the command-side staff bar, so they may come and go and
/// manage the scene. Everyone else is held until the scene ends, a facilitator
/// `+timestop/release`s them, or the scene is ended.
///
/// Note this fires on the **moved object's** membership: `@teleport X=…` moves
/// `X`, so a frozen non-staff `X` cannot be teleported out either — extraction
/// goes through `+timestop/release`, not a teleport (which is why staff have a
/// command-side recovery path independent of this gate).
struct TimestopMoveGate;

impl EventHook for TimestopMoveGate {
    fn move_attempt(
        &self,
        w: &mut World,
        actor: ObjId,
        _exit: Option<ObjId>,
        _dest: ObjId,
    ) -> Result<(), RichText> {
        if !w.in_exclusive_scene(actor) {
            return Ok(());
        }
        if is_scene_staff(w, actor) {
            return Ok(());
        }
        let scene_note = w
            .module_state::<ScenesState>()
            .and_then(|st| st.active_scene_of(actor))
            .map_or_else(
                || "a timestop".to_string(),
                |s| format!("a timestop (scene #{})", s.id),
            );
        Err(RichText::tag(
            "error",
            format!("You are caught in {scene_note}; you cannot leave until it ends."),
        ))
    }
}

/// `ObjDestroyed` observer: keep the freeze and the scene records consistent
/// when a frozen member is destroyed (staff purging an NPC or a chargen alt).
/// Core's `destroy` does not scrub the exclusive-scene index (scene owners must,
/// per its contract), so without this a dead dbref leaks in the index forever
/// and — if it was the facilitator — could strand every survivor with no way to
/// end the scene.
///
/// Policy: drop the object from every scene holding it and release its freeze;
/// if it was the **facilitator**, hand facilitation to the first surviving
/// participant (or close the scene if none remain); if removing it empties the
/// participants, close the scene. Closing releases all remaining members.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let obj = *obj;

    // 1. Snapshot the scenes holding the destroyed object (immutable read; no
    //    state borrow is held past this).
    let affected: Vec<SceneRecord> = match ctx.w.module_state::<ScenesState>() {
        Some(st) => st
            .scenes
            .values()
            .filter(|s| s.status.holds_members() && s.members().any(|m| m == obj))
            .cloned()
            .collect(),
        None => return,
    };
    if affected.is_empty() {
        return;
    }

    // 2. Decide each scene's new shape. This reads the world for kind checks
    //    (to hand facilitation to a *player*), which is why step 1 cloned out.
    enum Plan {
        /// Scene closed — release all these members.
        Close { record: SceneRecord, release_all: Vec<ObjId> },
        /// Object removed — release just it; `new_facilitator` set if handed on.
        Shrink { record: SceneRecord, new_facilitator: Option<ObjId> },
    }
    let mut plans: Vec<Plan> = Vec::new();
    for mut rec in affected {
        let was_facilitator = rec.facilitator == obj;
        rec.remove_member(obj);
        if rec.participants.is_empty() {
            rec.status = SceneStatus::Closed;
            rec.pause_reason = None;
            let release_all: Vec<ObjId> = rec.members().collect();
            plans.push(Plan::Close { record: rec, release_all });
        } else {
            let new_facilitator = if was_facilitator {
                // Prefer a player to inherit facilitation (an NPC could not
                // drive it); fall back to the first participant only if the
                // survivors are all NPCs (staff can still manage by room).
                let pick = rec
                    .participants
                    .iter()
                    .copied()
                    .find(|&p| ctx.w.get(p).is_some_and(|o| o.kind == ObjKind::Player))
                    .unwrap_or(rec.participants[0]);
                rec.facilitator = pick;
                Some(pick)
            } else {
                None
            };
            plans.push(Plan::Shrink { record: rec, new_facilitator });
        }
    }

    // 3. Apply: write the record back, adjust the freeze index, persist, tell
    //    the room.
    for plan in plans {
        match plan {
            Plan::Close { record, release_all } => {
                if let Some(st) = ctx.w.module_state_mut::<ScenesState>() {
                    st.scenes.insert(record.id, record.clone());
                }
                for m in release_all {
                    ctx.w.clear_exclusive_scene_member(m);
                }
                ctx.w.clear_exclusive_scene_member(obj);
                persist_scene(ctx.out, &record);
                announce_scene(
                    ctx,
                    record.anchor,
                    format!(
                        "Time resumes — the timestop (scene #{}) ends (its last participant is gone).",
                        record.id
                    ),
                );
                emit_scene_concluded(ctx, &record);
            }
            Plan::Shrink { record, new_facilitator } => {
                if let Some(st) = ctx.w.module_state_mut::<ScenesState>() {
                    st.scenes.insert(record.id, record.clone());
                }
                ctx.w.clear_exclusive_scene_member(obj);
                persist_scene(ctx.out, &record);
                if let Some(fac) = new_facilitator {
                    let name = display_name(ctx.w, fac);
                    announce_scene(
                        ctx,
                        record.anchor,
                        format!(
                            "The timestop's facilitator is gone; {name} now facilitates scene #{}.",
                            record.id
                        ),
                    );
                }
            }
        }
    }
}

/// Record a concluded scene as a durable **canon** event — the fact that a
/// scene occurred (its participants, room, and round span) becomes part of the
/// world chronicle, the seam richer consequences (deaths, obligations, faction
/// changes) will plug into. Restricted scope: the participants and staff know
/// it happened. A no-op if the canon module is not loaded.
pub(crate) fn emit_scene_concluded(ctx: &mut Ctx<'_>, rec: &SceneRecord) {
    // A scene whose last participant vanished (all destroyed) records nothing —
    // there is no living entity for it to concern, and "0 participants" is not a
    // meaningful chronicle line.
    if rec.participants.is_empty() {
        return;
    }
    let room_name = display_name(ctx.w, rec.anchor);
    let n = rec.participants.len();
    let summary = format!(
        "A scene concluded in {room_name} ({n} participant{}, {} round{}).",
        if n == 1 { "" } else { "s" },
        rec.round,
        if rec.round == 1 { "" } else { "s" }
    );
    let draft = mush_canon::CanonDraft {
        kind: SmolStr::new_static("scene.concluded"),
        summary,
        entities: rec.participants.clone(),
        scope: mush_canon::CanonScope::Restricted,
        ic_time: rec.scene_ic_time(),
        source: format!("scene:{}", rec.id),
        detail: serde_json::json!({
            "scene": rec.id.0,
            "rounds": rec.round,
            "anchor": rec.anchor.0,
        }),
    };
    let _ = mush_canon::commit(ctx, draft);
}

/// Announce a system line to everyone in `room` (shared by the observer and,
/// via `commands`, the command handlers).
pub(crate) fn announce_scene(ctx: &mut Ctx<'_>, room: ObjId, text: impl Into<String>) {
    ctx.out.to_room(
        room,
        Vec::new(),
        OutputEvent::Sys(RichText::tag("scene", text.into())),
    );
}

/// Append one event to a scene's immutable log and queue its persist op.
///
/// **Contract:** the event must already have happened — append only on the
/// success path. Lifecycle callers append after their mutation + persist; the
/// observers append for `Said`/`Moved` world events, which the dispatch layer
/// emits *terminally* (as the last act before a handler returns `Ok`), so an
/// observed event never rides a command that then fails. This matters because
/// the append advances in-RAM ledger state that is **not** rolled back on a
/// handler `Err` (only the deferred persist op is truncated) — a later
/// successful append's whole-chunk rewrite would otherwise resurrect a
/// phantom entry to disk. Any future command that emits `Said`/`Moved` must
/// keep that emit terminal.
pub(crate) fn record_scene_event(ctx: &mut Ctx<'_>, scene: SceneUid, ev: SceneEvent) {
    let value = serde_json::to_value(&ev).expect("scene event serializes");
    let op = {
        let Some(st) = ctx.w.module_state_mut::<ScenesState>() else {
            return;
        };
        let (_seq, op) = st.events.append(&scene.to_string(), value);
        op
    };
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    });
}

/// Whether a narrative flavor is IC (recorded) rather than OOC/private.
fn is_ic_narrative(kind: NarrativeKind) -> bool {
    matches!(
        kind,
        NarrativeKind::Say
            | NarrativeKind::Pose
            | NarrativeKind::SemiPose
            | NarrativeKind::Emit
            | NarrativeKind::Spoof
    )
}

/// A short label for a narrative flavor, for the event detail.
fn narr_label(kind: NarrativeKind) -> &'static str {
    match kind {
        NarrativeKind::Say => "say",
        NarrativeKind::Pose => "pose",
        NarrativeKind::SemiPose => "semipose",
        NarrativeKind::Emit => "emit",
        NarrativeKind::Spoof => "spoof",
        NarrativeKind::Page => "page",
        NarrativeKind::Ooc => "ooc",
        NarrativeKind::Whisper => "whisper",
    }
}

/// The `Active` scene anchored to `room`, cloned (releasing the state borrow so
/// the caller may then record into it).
fn active_scene_at(w: &World, room: ObjId) -> Option<SceneRecord> {
    w.module_state::<ScenesState>()
        .and_then(|st| st.active_scene_at(room))
        .filter(|s| s.status == SceneStatus::Active)
        .cloned()
}

/// `Said` observer: record IC narrative spoken in an active scene's anchor room
/// as a `Said` event. OOC / page / whisper are never recorded.
fn on_said(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::Said { who, room, kind, text } = ev else {
        return;
    };
    if !is_ic_narrative(*kind) {
        return;
    }
    let Some(s) = active_scene_at(ctx.w, *room) else {
        return;
    };
    let detail = serde_json::json!({ "narr": narr_label(*kind), "text": text });
    let event = SceneEvent::new(&s, ctx.w.now(), SceneEventKind::Said, Some(*who), detail);
    record_scene_event(ctx, s.id, event);
}

/// The turn gate: inside an **active turn-based** scene, a command declared
/// [`SceneClass::Turn`] (a mechanical action — a ruleset's `+roll`, an attack)
/// runs only for the participant whose turn it is. Everything else — speech,
/// poses, OOC, inspection, safety — is [`SceneClass::Free`] and always passes.
/// This is the general seam: rulesets tag their mechanical commands and need no
/// scene awareness; a paused scene blocks mechanical actions entirely.
struct TurnGate;

impl CommandGate for TurnGate {
    fn check(
        &self,
        w: &World,
        reg: &Registry,
        actor: ObjId,
        verb: &str,
        _line: &str,
    ) -> Result<(), String> {
        let Some(scene) = w
            .module_state::<ScenesState>()
            .and_then(|st| st.active_scene_of(actor))
        else {
            return Ok(());
        };
        if scene.mode != SceneMode::TurnBased {
            return Ok(());
        }
        // Resolve the verb exactly as dispatch will (lookup honors the prefix
        // rules, so an abbreviation can't slip a Turn command past the gate);
        // Free is the default for anything that doesn't resolve to a command.
        let class = reg
            .lookup(verb)
            .map_or(SceneClass::Free, |(spec, _)| spec.scene_class);
        if class != SceneClass::Turn {
            return Ok(());
        }
        if scene.status == SceneStatus::Paused {
            return Err(format!(
                "The timestop (scene #{}) is paused — mechanical actions wait for it to resume.",
                scene.id
            ));
        }
        if !scene.participants.contains(&actor) {
            // In the scene as an observer — no turn to take.
            return Err(
                "You are observing this scene, not participating — you have no turn to act on."
                    .to_string(),
            );
        }
        if scene.current_actor() != Some(actor) {
            let whose = scene
                .current_actor()
                .map_or_else(|| "no one".to_string(), |a| display_name(w, a));
            return Err(format!(
                "It is not your turn in the scene — it is {whose}'s (round {}). \
                 Wait, or ask the facilitator to advance the turn.",
                scene.round
            ));
        }
        Ok(())
    }
}

/// `Rolled` observer: record a mechanical resolution into an active scene's log.
fn on_rolled(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::Rolled { who, room, summary } = ev else {
        return;
    };
    let Some(s) = active_scene_at(ctx.w, *room) else {
        return;
    };
    let detail = serde_json::json!({ "summary": summary });
    let event = SceneEvent::new(&s, ctx.w.now(), SceneEventKind::Rolled, Some(*who), detail);
    record_scene_event(ctx, s.id, event);
}

/// `Moved` observer: record entries into / exits from an active scene's anchor
/// room (participants are frozen, so these are non-participants or staff coming
/// and going — context for the transcript).
fn on_moved(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::Moved { who, from, to } = ev else {
        return;
    };
    // A no-op move (e.g. `@teleport X=<the room X is already in>`) still emits
    // Moved with from == to; skip it so the anchor doesn't log a bogus
    // arrival-and-departure for the same actor.
    if *from == Some(*to) {
        return;
    }
    let now = ctx.w.now();
    if let Some(s) = active_scene_at(ctx.w, *to) {
        let event = SceneEvent::new(&s, now, SceneEventKind::Joined, Some(*who), serde_json::json!({}));
        record_scene_event(ctx, s.id, event);
    }
    if let Some(from) = *from
        && let Some(s) = active_scene_at(ctx.w, from)
    {
        let event = SceneEvent::new(&s, now, SceneEventKind::Left, Some(*who), serde_json::json!({}));
        record_scene_event(ctx, s.id, event);
    }
}

/// Rebuild [`ScenesState`] from persisted boot data. Every non-closed scene
/// rehydrates as `Paused(ServerRestart)` (turns never auto-advance across a
/// restart) with its exclusive members re-published to the core index, so the
/// room stays frozen until a facilitator resumes. A scene document that fails
/// to decode FAILS THE BOOT — dropping it would silently unfreeze its members
/// and lose the scene; corrupt data is an operational problem to fix.
fn load_boot(ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<(), ModError> {
    let mut scenes: std::collections::BTreeMap<SceneUid, SceneRecord> =
        std::collections::BTreeMap::new();
    let mut next_from_meta: Option<u64> = None;

    for (key, value) in &data.kv {
        if key == META_KEY {
            next_from_meta = value
                .get("next_scene_uid")
                .and_then(serde_json::Value::as_u64);
        } else if let Some(id) = key.strip_prefix("scene:") {
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("scenes: bad scene key {key:?}")))?;
            let mut rec = serde_json::from_value::<SceneRecord>(value.clone())
                .map_err(|e| ModError::msg(format!("scenes: bad scene doc {key:?}: {e}")))?;
            if rec.id != SceneUid(id) {
                return Err(ModError::msg(format!(
                    "scenes: doc {key:?} claims id {}",
                    rec.id
                )));
            }
            // Freeze rehydrated live scenes: a restart pauses them (turns never
            // auto-advance across a restart). A Closed scene stays history.
            if matches!(rec.status, SceneStatus::Active | SceneStatus::Paused) {
                rec.status = SceneStatus::Paused;
                rec.pause_reason = Some(PauseReason::ServerRestart);
            }
            scenes.insert(rec.id, rec);
        } else if is_log_key(key) {
            // Event-log chunk docs are rehydrated into the ledger below.
        } else {
            tracing::warn!(%key, "scenes: unknown persisted key ignored");
        }
    }

    let next_scene_uid = next_from_meta
        .unwrap_or_else(|| scenes.keys().last().map_or(0, |id| id.0 + 1))
        .max(scenes.keys().last().map_or(0, |id| id.0 + 1));

    // Rehydrate the event-log open tail — but only for scenes that still hold
    // members. A Closed scene's log is immutable history on disk that nothing
    // in RAM appends to or reads (`active_scene_of` excludes closed scenes), so
    // restoring its tail would be permanent dead RAM that grows with every
    // finished scene. (The deferred boot store-read seam makes even this
    // request-driven; for now, filter closed scenes out.)
    let live_uids: std::collections::BTreeSet<String> = scenes
        .values()
        .filter(|s| s.status.holds_members())
        .map(|s| s.id.0.to_string())
        .collect();
    let live_log_kv: Vec<(SmolStr, serde_json::Value)> = data
        .kv
        .iter()
        .filter(|(k, _)| {
            k.strip_prefix("log:")
                .and_then(|rest| rest.split(':').next())
                .is_some_and(|id| live_uids.contains(id))
        })
        .cloned()
        .collect();
    let mut events = mush_core::Ledger::new();
    events.restore_from_kv(&live_log_kv);
    let st = ScenesState {
        scenes,
        next_scene_uid,
        events,
    };
    // Re-persist any scene we paused for the restart, and re-publish every held
    // member to the core exclusive-scene index so the room stays frozen.
    let changed: Vec<SceneRecord> = st
        .scenes
        .values()
        .filter(|s| s.pause_reason == Some(PauseReason::ServerRestart))
        .cloned()
        .collect();
    let held = st.held_objects();

    *ctx.w
        .module_state_mut::<ScenesState>()
        .expect("scenes state slot installed at registration") = st;
    for obj in held {
        ctx.w.set_exclusive_scene_member(obj);
    }
    for s in &changed {
        persist_scene(ctx.out, s);
    }
    Ok(())
}
