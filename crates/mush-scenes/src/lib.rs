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
mod panel;

use std::sync::Arc;

use mush_core::module::{
    EventHook, GameModule, HookPoint, HookReadContext, ModError, ModuleBootData, ModuleCtx,
    Registrar, WorldEvent, WorldEventKind,
};
use mush_core::output::display_name;
use mush_core::{
    ClockProviderContext, ClockProviderFailure, CommandGate, Ctx, Migration, ModuleOp, ObjId,
    ObjKind, Registry, Role, SceneClass, SceneClockProvider, World, is_log_key, perm,
    scan_contiguity,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use smol_str::SmolStr;

pub use data::{
    KIND_TIMESTOP, MAX_IC_FLOOR_LEAD_SECS, MAX_OUTCOME_NAME_BYTES, MAX_OUTCOME_NOTE_BYTES,
    MAX_OUTCOME_SUBJECTS, MAX_PENDING_STORE_READS_PER_ACTOR, NS, PauseReason, PendingArchiveRead,
    SceneAdvanceError, SceneEvent, SceneEventKind, SceneMode, SceneOutcome, SceneRecord,
    SceneStatus, SceneUid, ScenesState, SealerGateSlot, scene_key,
};

/// The canon kind a **sealed scene outcome** is recorded under (E6b, 08f §4.1).
///
/// The composition root registers exactly this string with the economy's
/// `with_canon_outcome_kinds`, and nothing else: `scene.concluded` is lifecycle
/// any participant can produce and is deliberately never an outcome kind.
pub const KIND_SCENE_OUTCOME: &str = "scene.outcome";

use data::{META_KEY, persist_scene};

/// Whether `who` is scene-management staff (Royalty and up) — the same bar the
/// commands use, so a staffer who can drive a scene is also not frozen by it.
fn is_scene_staff(w: &World, who: ObjId) -> bool {
    perm::has_role(w, who, Role::Royalty)
}

/// The **sealer gate** (E6b independent verify fold, M4).
///
/// `@scene/facilitate <scene>=<who> run=<id>` had no party check of any kind, and
/// `mush-scenes` has no economy edge to make one with — so staff could confer the
/// seal of a run on one of that run's own payees, and the shipped acceptance test
/// did exactly that. The seal is a settlement authority; 08e answer 22's rule that
/// the authority ending a run never stands on one side of it applies to it as much
/// as to the adjudicator.
///
/// The gate is a **composition-frozen boxed closure**, supplied by the server from
/// the economy at wiring, so `mush-scenes -> {core, canon, proto}` is unchanged: the
/// run id stays opaque here, and the answer to "is this object a party to, or a role
/// holder of, that contract" comes from the crate that knows. `None` (the default,
/// and every composition that wires no economy) means the question cannot be asked
/// and conferral is unrestricted — which is exactly the pre-fold behaviour, on a
/// server that has no runs to protect.
pub type SealerGate = Arc<dyn Fn(&World, u64, ObjId) -> bool + Send + Sync>;

/// The scenes game module. Hand one to `Engine::with_modules`.
pub struct ScenesModule {
    sealer_gate: Option<SealerGate>,
}

impl ScenesModule {
    /// A fresh module instance, with no sealer gate (conferral is unrestricted).
    #[must_use]
    pub fn new() -> ScenesModule {
        ScenesModule { sealer_gate: None }
    }

    /// Freeze the **sealer gate** at wiring (E6b independent verify fold, M4).
    ///
    /// The predicate answers "is `who` a party to — or a named role holder of — run
    /// `run`?". `@scene/facilitate … run=<id>` refuses a sealer it answers `true`
    /// for, and `+scene/outcome` asks it again at seal time, because the roster of a
    /// contract moves between conferral and seal.
    #[must_use]
    pub fn with_sealer_gate(mut self, gate: SealerGate) -> ScenesModule {
        self.sealer_gate = Some(gate);
        self
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
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(ScenesState {
            sealer_gate: data::SealerGateSlot(self.sealer_gate.clone()),
            ..ScenesState::default()
        });
        reg.scene_clock_provider(DurableSceneClock)?;
        reg.migrations(
            NS,
            vec![
                Migration {
                    version: 1,
                    description: "initial durable scene records".into(),
                    apply: Arc::new(|_key, value| Ok(value)),
                },
                Migration {
                    version: 2,
                    description: "version scene clock anchors and turn cursors".into(),
                    apply: Arc::new(migrate_scene_clock_v2),
                },
                // E6b verify fold (R3). The three E6b fields are all
                // `#[serde(default)]`, so **forward** they need no migration at all
                // — an old document loads with `sealer: None`, `seal_run: None`,
                // `outcome: None`, `ic_floor: 0`, which is exactly the pre-E6b
                // behaviour. The stamp is for the other direction: without it, a
                // rollback to the pre-E6b binary boots happily on a v2 namespace,
                // drops the three fields it does not know, and the first
                // `persist_scene` permanently erases a **terminal** sealed outcome
                // the economy has already cited as evidence. The migration runner
                // refuses a namespace stamped ahead of the binary, so the version
                // alone buys that refusal; the `apply` has nothing to do.
                Migration {
                    version: 3,
                    description: "the conferred sealer, the sealed outcome and the IC floor".into(),
                    apply: Arc::new(|_key, value| Ok(value)),
                },
            ],
        )?;
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
        // The `scenes` panel feed (a keyframe to every member on each
        // lifecycle transition), its same-named query, and the room-band
        // glance under `RoomView.ambient["scene"]` (interface plan I1).
        let _ = reg.panel_feed(panel::FEED)?;
        reg.query_handler(panel::FEED, Arc::new(panel::ScenesQuery))?;
        reg.hook(HookPoint::RoomAmbient, Arc::new(panel::SceneAmbient));
        // H1: the archive — cold pages of a scene's log come back through the
        // module store-read seam and render here.
        reg.store_read_completion(Arc::new(commands::ScenesArchiveRead))?;
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

/// Adapter from the durable scene module state to the kernel's first-class
/// `SceneTurn` clock namespace.
struct DurableSceneClock;

impl SceneClockProvider for DurableSceneClock {
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        scene: u64,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure> {
        let state = context
            .module_state::<ScenesState>()
            .ok_or(ClockProviderFailure::Integrity)?;
        let record = state
            .scenes
            .get(&SceneUid(scene))
            .ok_or(ClockProviderFailure::Integrity)?;
        if record.mode != data::SceneMode::TurnBased || record.validate_clock().is_err() {
            return Err(ClockProviderFailure::Integrity);
        }
        if generation != record.clock_generation {
            return Err(ClockProviderFailure::StaleGeneration {
                requested: generation,
                current: record.clock_generation,
            });
        }
        Ok(record.turn)
    }
}

fn migrate_scene_clock_v2(
    key: &str,
    mut value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if !key.starts_with("scene:") {
        return Ok(value);
    }
    let record = value
        .as_object_mut()
        .ok_or_else(|| "scene document must be an object".to_owned())?;
    let anchor = record
        .get_mut("ic_started_at")
        .ok_or_else(|| "scene document is missing ic_started_at".to_owned())?;
    if let Some(at) = anchor.as_i64() {
        *anchor = serde_json::json!({
            "clock": { "kind": "world_ic", "generation": 1 },
            "at": at,
        });
    } else if !anchor.is_object() {
        return Err("scene ic_started_at has an invalid shape".to_owned());
    }
    record
        .entry("clock_generation")
        .or_insert_with(|| serde_json::json!(1));
    if !record.contains_key("turn") {
        // No SceneTurn stamp existed before v2. Establish its initial cursor
        // from the old current round/seat without claiming historical turns
        // across membership changes.
        let round = record
            .get("round")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "scene document has no valid round".to_owned())?;
        let turn_index = record
            .get("turn_index")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "scene document has no valid turn_index".to_owned())?;
        let participants = record
            .get("participants")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "scene document has no valid participants".to_owned())?;
        let seats = u64::try_from(participants.len())
            .map_err(|_| "scene participant count is not representable".to_owned())?;
        let prior_rounds = round.saturating_sub(1);
        let turn = prior_rounds
            .checked_mul(seats)
            .and_then(|base| base.checked_add(turn_index))
            .and_then(|value| i64::try_from(value).ok())
            .ok_or_else(|| "scene turn cursor overflowed during migration".to_owned())?;
        record.insert("turn".to_owned(), serde_json::json!(turn));
    }
    Ok(value)
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
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        _exit: Option<ObjId>,
        _dest: ObjId,
    ) -> Result<(), RichText> {
        let w = ctx.world();
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
        Close {
            record: SceneRecord,
            release_all: Vec<ObjId>,
        },
        /// Object removed — release just it; `new_facilitator` set if handed on.
        Shrink {
            record: SceneRecord,
            new_facilitator: Option<ObjId>,
        },
    }
    let mut plans: Vec<Plan> = Vec::new();
    for mut rec in affected {
        let was_facilitator = rec.facilitator == obj;
        rec.remove_member(obj);
        if rec.participants.is_empty() {
            rec.status = SceneStatus::Closed;
            rec.pause_reason = None;
            let release_all: Vec<ObjId> = rec.members().collect();
            plans.push(Plan::Close {
                record: rec,
                release_all,
            });
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
            plans.push(Plan::Shrink {
                record: rec,
                new_facilitator,
            });
        }
    }

    // 3. Apply: write the record back, adjust the freeze index, persist, tell
    //    the room.
    for plan in plans {
        match plan {
            Plan::Close {
                record,
                release_all,
            } => {
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
            Plan::Shrink {
                record,
                new_facilitator,
            } => {
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

// ---- E6b: B7 — the seal is canon, and only the producer gates it ----------

/// Record a **sealed scene outcome** as canon — a *second* `CanonDraft` beside
/// [`emit_scene_concluded`], never a change to it (08f §4.1, D-E6-7).
///
/// - `kind` is [`KIND_SCENE_OUTCOME`], the one kind a composition root registers
///   as a producer-declared outcome. `scene.concluded` is never registered.
/// - `source` is `run:{contract}` — a `scene:{uid}` is unknowable when the run
///   is offered, and the economy compares `source` as an **opaque string**.
/// - `entities` is `subjects ∪ {sealed_by}`, **named at seal time**, never
///   copied from `participants` (frozen at scene creation, so a runner who
///   joined mid-scene would silently fail the economy's party-intersection
///   gate).
/// - `ic_time` is `scene_ic_time()` **after** the seal raised `ic_floor` to the
///   world IC clock, so it is the world IC clock by construction rather than by
///   a second rule.
///
/// Who may seal is the **producer's** gate and nobody else's: `CanonRecorded`
/// carries `{canon_id, kind, entities, source}` and no `detail`, so the economy
/// checks kind, source and party intersection and *nothing else*.
pub(crate) fn emit_scene_outcome(
    ctx: &mut Ctx<'_>,
    rec: &SceneRecord,
    outcome: &data::SceneOutcome,
) {
    // E6b verify fold (M2/R12): the run is the one **staff bound to this scene**,
    // never free input at seal time. A scene bound to none seals for the chronicle
    // alone, under a `scene:{uid}` source no contract can ever name — a `scene:{uid}`
    // is unknowable when a run is offered, which is why `run:` exists at all.
    //
    // E6b verify fold (M5): and it names the **row**. `CanonRecorded` carries no
    // `detail`, so the economy matches on `(kind, source)` alone: with one source per
    // run, one seal filled every canon row's slot and the contract read
    // `Undecidable`; with a single row the row name the sealer typed decided nothing.
    let source = match rec.seal_run {
        Some(contract) => format!("run:{contract}#{}", outcome.name),
        None => format!("scene:{}", rec.id),
    };
    let mut entities = outcome.subjects.clone();
    if !entities.contains(&outcome.sealed_by) {
        entities.push(outcome.sealed_by);
    }
    let room_name = display_name(ctx.w, rec.anchor);
    // E6b independent verify fold (D3): **the summary does not name the contract.**
    // A `scene.outcome` record is `Restricted`, and `+chronicle` shows a `Restricted`
    // record to every entity it names — but the entities are the seal's *subjects*,
    // who need not be parties to the run. Interpolating `run:{id}#{row}` into the
    // line therefore told a subject which contract paid for the work, and which row
    // of its outcome table was selected, off a chronicle read. The source stays in
    // `detail` (staff) and in the `CanonRecorded` event (the economy's own match key),
    // which is where it is load-bearing; the chronicle line is prose.
    let summary = format!(
        "A sealed outcome in {room_name}: {} (scene #{}).",
        outcome.name, rec.id
    );
    let draft = mush_canon::CanonDraft {
        kind: SmolStr::new_static(KIND_SCENE_OUTCOME),
        summary,
        entities,
        scope: mush_canon::CanonScope::Restricted,
        ic_time: rec.scene_ic_time(),
        source,
        detail: serde_json::json!({
            "scene": rec.id.0,
            "outcome": outcome.name.as_str(),
            "sealed_by": outcome.sealed_by.0,
            "round": rec.round,
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
        let prepared = match st.events.prepare_append(&scene.to_string(), value) {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::error!(scene = %scene, %error, "scene: unsafe event-ledger append refused without ledger mutation");
                return;
            }
        };
        let (_seq, op) = st
            .events
            .apply_prepared(prepared)
            .expect("scene ledger cannot change during one exclusive state borrow");
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
    let WorldEvent::Said {
        who,
        room,
        kind,
        text,
    } = ev
    else {
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
        line: &str,
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
        // Resolve the verb exactly as dispatch will: the global table first
        // (lookup honors the prefix rules, so an abbreviation can't slip a Turn
        // command past the gate), then — only when that misses — the actor's
        // reachable behavior commands (a mech's FIRE is a Turn command supplied
        // by the object, never in the global table). They are ordinary
        // CommandSpecs carrying a scene_class: one turn engine, not two. No
        // match or an ambiguous match reads as Free (dispatch refuses or Huh?s
        // it anyway).
        // Exits and `home` resolve BEFORE the command table in dispatch and are
        // always Free — check them first so a Turn-class behavior verb that
        // happens to share an exit's name can never block a legal move.
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("home")
            || mush_core::commands::movement::find_exit(w, actor, trimmed).is_some()
        {
            return Ok(());
        }
        let class = match reg.lookup(verb) {
            Ok((spec, _)) => spec.scene_class,
            // An ambiguous global prefix is refused by dispatch before the
            // behavior stage, so it is Free here too.
            Err(mush_core::LookupErr::Ambiguous(_)) => SceneClass::Free,
            Err(mush_core::LookupErr::NotFound) => {
                match mush_core::resolve_behavior_command(reg, w, actor, verb) {
                    mush_core::BehaviorResolution::One { target, index } => reg
                        .behavior_spec(&target.behavior)
                        .and_then(|b| b.commands.get(index))
                        .map_or(SceneClass::Free, |c| c.spec.scene_class),
                    _ => SceneClass::Free,
                }
            }
        };
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
        let event = SceneEvent::new(
            &s,
            now,
            SceneEventKind::Joined,
            Some(*who),
            serde_json::json!({}),
        );
        record_scene_event(ctx, s.id, event);
    }
    if let Some(from) = *from
        && let Some(s) = active_scene_at(ctx.w, from)
    {
        let event = SceneEvent::new(
            &s,
            now,
            SceneEventKind::Left,
            Some(*who),
            serde_json::json!({}),
        );
        record_scene_event(ctx, s.id, event);
    }
}

/// Rebuild [`ScenesState`] from persisted boot data. Every non-closed scene
/// rehydrates as `Paused(ServerRestart)` (turns never auto-advance across a
/// restart) with its exclusive members re-published to the core index, so the
/// room stays frozen until a facilitator resumes. A scene document that fails
/// to decode FAILS THE BOOT — dropping it would silently unfreeze its members
/// and lose the scene; corrupt data is an operational problem to fix.
/// Drop an `outcome` field that will not decode, leaving the rest of the document
/// strict (E6b independent verify fold, R6). Returns the document and whether an
/// outcome was lost.
fn strip_unreadable_outcome(mut value: serde_json::Value) -> (serde_json::Value, bool) {
    let Some(map) = value.as_object_mut() else {
        return (value, false);
    };
    match map.get("outcome") {
        None | Some(serde_json::Value::Null) => (value, false),
        Some(raw) => {
            if serde_json::from_value::<data::SceneOutcome>(raw.clone()).is_ok() {
                return (value, false);
            }
            map.insert("outcome".to_owned(), serde_json::Value::Null);
            (value, true)
        }
    }
}

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
            // E6b independent verify fold (R6): **a corrupt sealed outcome does not
            // brick the boot.** `SceneOutcome` is a `TryFrom` wire, and its failure
            // failed the whole `SceneRecord`, which `load_boot` turned into a
            // `ModError` — so one over-long note in one persisted outcome (written by
            // an older build, a hand edit, or a future schema) was a server that
            // could not start, and the scene it belonged to was otherwise perfectly
            // readable. The outcome is decoded leniently instead: the scene loads
            // with `outcome: None`, its **sealer and its run binding are kept**, and
            // the loss is loud. The economy's write-once slots make a re-seal
            // harmless — a second canon record for a filled slot attests nothing —
            // so "re-seal it" is a real remedy, which is what makes this the right
            // trade. Every other field stays strict.
            let (value, lost_outcome) = strip_unreadable_outcome(value.clone());
            if lost_outcome {
                tracing::error!(
                    %key,
                    "scenes: a persisted scene outcome does not decode; the scene loads UNSEALED \
                     and its sealer may seal it again (+scene/outcome)"
                );
            }
            let mut rec = serde_json::from_value::<SceneRecord>(value)
                .map_err(|e| ModError::msg(format!("scenes: bad scene doc {key:?}: {e}")))?;
            if rec.id != SceneUid(id) {
                return Err(ModError::msg(format!(
                    "scenes: doc {key:?} claims id {}",
                    rec.id
                )));
            }
            rec.validate_clock().map_err(|error| {
                ModError::msg(format!(
                    "scenes: invalid clock in scene doc {key:?}: {error}"
                ))
            })?;
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

    // Validate every authoritative scene-history document before selecting the
    // live tails retained in RAM. Closed/missing-scene logs remain immutable on
    // disk, but corruption there is still an integrity breach and may not be
    // hidden by the memory optimization below.
    let all_log_kv: Vec<(SmolStr, serde_json::Value)> = data
        .kv
        .iter()
        .filter(|(key, _)| key.starts_with("log:"))
        .cloned()
        .collect();
    let breaches = scan_contiguity(&all_log_kv);
    if !breaches.is_empty() {
        return Err(ModError::msg(format!(
            "scene ledger history validation failed: {breaches:?}"
        )));
    }

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
    events
        .restore_from_kv(&live_log_kv)
        .map_err(|error| ModError::msg(format!("scene ledger restore failed: {error}")))?;
    // The gate is wiring, not boot data: it is installed at registration and must
    // survive `on_world_loaded` replacing the state slot wholesale.
    let sealer_gate = ctx
        .w
        .module_state::<ScenesState>()
        .map(|st| st.sealer_gate.clone())
        .unwrap_or_default();
    let st = ScenesState {
        // Pending archive reads are transient: a boot starts with none.
        pending_reads: Default::default(),
        sealer_gate,
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

#[cfg(test)]
mod clock_migration_tests {
    use super::*;

    #[test]
    fn v2_migration_versions_the_world_anchor_and_establishes_a_turn_cursor() {
        let migrated = migrate_scene_clock_v2(
            "scene:7",
            serde_json::json!({
                "ic_started_at": 1234,
                "round": 3,
                "turn_index": 1,
                "participants": [2, 3, 4],
            }),
        )
        .unwrap();
        assert_eq!(
            migrated["ic_started_at"],
            serde_json::json!({
                "clock": { "kind": "world_ic", "generation": 1 },
                "at": 1234,
            })
        );
        assert_eq!(migrated["clock_generation"], 1);
        assert_eq!(migrated["turn"], 7);
    }
}
