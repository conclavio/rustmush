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
use mush_core::{Ctx, ObjId, ObjKind, Role, World, perm};
use mush_proto::{OutputEvent, RichText};

pub use data::{
    KIND_TIMESTOP, NS, PauseReason, SceneMode, SceneRecord, SceneStatus, SceneUid, ScenesState,
    scene_key,
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

/// Announce a system line to everyone in `room` (shared by the observer and,
/// via `commands`, the command handlers).
pub(crate) fn announce_scene(ctx: &mut Ctx<'_>, room: ObjId, text: impl Into<String>) {
    ctx.out.to_room(
        room,
        Vec::new(),
        OutputEvent::Sys(RichText::tag("scene", text.into())),
    );
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

    for (key, value) in data.kv {
        if key == META_KEY {
            next_from_meta = value
                .get("next_scene_uid")
                .and_then(serde_json::Value::as_u64);
        } else if let Some(id) = key.strip_prefix("scene:") {
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("scenes: bad scene key {key:?}")))?;
            let mut rec = serde_json::from_value::<SceneRecord>(value)
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
        } else {
            tracing::warn!(%key, "scenes: unknown persisted key ignored");
        }
    }

    let next_scene_uid = next_from_meta
        .unwrap_or_else(|| scenes.keys().last().map_or(0, |id| id.0 + 1))
        .max(scenes.keys().last().map_or(0, |id| id.0 + 1));

    let st = ScenesState { scenes, next_scene_uid };
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
