//! **The link-loss backup mind** (doc 35 §11; Q-35d-2 as ruled on 2026-11-11).
//!
//! When every crew member aboard a hull has been without a link for
//! [`super::tick::SPACE_LINKDEAD_GRACE_WALL_S`], a mind takes her helm and tries to bring her
//! home. It keeps the helm until a crew member comes back or she docks.
//!
//! # The mind
//!
//! A Rhai doctrine like the picket's, thought once a space tick while there is something to
//! decide. Which source flies, first that resolves and thinks without a fault:
//!
//! 1. the mind sealed on the hull ([`super::picket::space_mind_seal_of`]), when the store
//!    holds its source;
//! 2. the house default the store names (`tacsim:doctrine:linkdead_return`, a pointer to a
//!    `tacsim:mindsrc:` document: [`house_doctrine_documents`]);
//! 3. the house default compiled in (`content/minds/linkdead_return.rhai`).
//!
//! # The seat
//!
//! The mind asks; the seat, which is this module, acts, and the seat is where the ruling's
//! limits are kept, so no source can step past them:
//!
//! | The mind asks | Outside a bubble | Inside a bubble |
//! |---|---|---|
//! | `speed(n)`, n above nought | the return passage, by the ruled order | nothing: she coasts |
//! | `hold()` | she holds where she is | she brakes to rest against the picket, and submits to a customs hail once hove to |
//! | `emcon(on)` | nothing | her emission posture |
//! | `lock`, `fire`, `arm` | dropped | dropped |
//!
//! The return is `execute_linkdead_return_journey` (`commands.rs`): the filed port, then
//! every open port nearest first, each through the planner with its tank and locker checks and
//! its debit. Nothing here draws from a vault; the seat has no verb that mines or trades.
//!
//! What the mind reads beyond its contacts: `v.held` while a customs picket holds her
//! uncleared, and `v.motor.outcome == "no_route"` for [`BACKUP_RETURN_RETRY_WALL_S`] after
//! every open port was refused.

use std::sync::Arc;

use mush_core::{Ctx, ObjId, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::bubble::{BubbleInputCommand, EmconPosture, HullStatus};
use super::picket::{CARRIER_PARTY, HEAVE_TO_REL_VEL_MMPS, PicketInspectionStatus};
use super::tick::{ActiveInterdictionBubble, SpaceBubbleBook, SpaceLinkdeadState};
use crate::arena::intents::{IntentKind, MotorGoal};
use crate::arena::mind::{MindView, ThinkResult};
use crate::arena::perception::{DriveOutcome, Receiver as FrameReceiver};
use crate::arena::state::{Designator, MatchMode, Power};
use crate::arena::step::FrameMotor;
use crate::minds::content::{LINKDEAD_RETURN_ROLE, linkdead_return_digest, source_lookup};
use crate::store::TacsimStore;

/// Real seconds the seat waits, after every open port's passage was refused, before it asks
/// the planner again (doc 35 §11 `B-6`). She drifts meanwhile, so the answer can change.
pub const BACKUP_RETURN_RETRY_WALL_S: i64 = 60;

/// Relative speed under which the seat stops braking: she is at rest against the picket.
const BRAKE_REST_MMPS: i64 = 500;

/// The hull's line when the grace runs out.
pub const BACKUP_TAKES_HELM: &str =
    "[SPACE] Linkdead grace window expired; the backup mind has the helm.";

/// What the backup seat last did for a hull (doc 35 §11). The crew hear each change once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupAct {
    /// Nothing yet.
    #[default]
    Standby,
    /// She is on a passage to a port.
    MakingForPort,
    /// She is braking to rest against a picket.
    HeavingTo,
    /// She has been submitted to customs; their ruling follows on the space tick.
    Inspected,
    /// She holds where she is because her mind asked it.
    Holding,
    /// No port is within her tank and stores: she holds and is a retrieval job.
    HoldingForRetrieval,
}

/// What one think asked of the seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    /// No helm intent.
    Nothing,
    /// `speed(n)`, n above nought.
    MakeForPort,
    /// `hold()`, or `speed(0)`.
    Hold,
}

/// One think, read: the helm ask and the emission posture, if the mind set one.
struct Thought {
    ask: Ask,
    emcon: Option<bool>,
}

/// The two documents an operator writes to replace the house default of `role` with `source`
/// (doc 35 §11 `B-3`): the source under its own digest, and the pointer that names it. Both
/// are read at boot.
#[must_use]
pub fn house_doctrine_documents(role: &str, source: &str) -> [(SmolStr, serde_json::Value); 2] {
    use sha2::{Digest as _, Sha256};
    let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
    let sha256 = crate::arena::digest_text(&digest);
    [
        (
            crate::store::mindsrc_key(&digest),
            serde_json::json!({
                "sha256": sha256,
                "abi": crate::minds::MIND_ABI,
                "bytes": source,
            }),
        ),
        (
            crate::store::doctrine_key(role),
            serde_json::json!({ "sha256": sha256 }),
        ),
    ]
}

/// Think once for `hull` on `view`, with the first of her sealed mind, the store's house
/// default and the compiled house default that thinks without a fault; write what it
/// remembered and traced into `state`.
fn think(
    w: &World,
    hull: ObjId,
    view: MindView,
    state: &mut SpaceLinkdeadState,
) -> Option<Thought> {
    let store = w.module_state::<TacsimStore>();
    let sealed = super::picket::space_mind_seal_of(w, hull)
        .map(|seal| seal.source_sha256)
        .filter(|digest| source_lookup(store, digest).is_some());
    let house = store.and_then(|store| store.house_doctrine(LINKDEAD_RETURN_ROLE));
    let compiled = linkdead_return_digest().0;
    let mut tried: Vec<[u8; 32]> = Vec::new();
    let think_seq = u64::try_from(w.now()).unwrap_or(0);
    let t_ms = view.t_ms;
    let view = Arc::new(view);
    for digest in [sealed, house, Some(compiled)].into_iter().flatten() {
        if tried.contains(&digest) {
            continue;
        }
        tried.push(digest);
        let result = super::picket::run_sealed_mind_on_pool(
            hull,
            None,
            digest,
            store,
            think_seq,
            t_ms,
            Arc::clone(&view),
            &state.mind_memory,
        );
        match result {
            Some(result) if result.fault.is_none() => return Some(read(result, state)),
            Some(result) => tracing::warn!(
                hull = hull.0,
                fault = ?result.fault,
                "tacsim: a link-loss backup mind faulted; the next source is tried"
            ),
            None => tracing::warn!(
                hull = hull.0,
                "tacsim: a link-loss backup mind did not run; the next source is tried"
            ),
        }
    }
    None
}

/// Read a clean think into the seat's terms and keep its memory and its last trace word.
fn read(result: ThinkResult, state: &mut SpaceLinkdeadState) -> Thought {
    if state.mind_memory != result.memory {
        state.mind_memory = result.memory;
    }
    if let Some(word) = result.trace.last()
        && state.mind_last_trace.as_deref() != Some(word.as_str())
    {
        state.mind_last_trace = Some(word.clone());
    }
    let mut thought = Thought {
        ask: Ask::Nothing,
        emcon: None,
    };
    for intent in &result.intents {
        match intent {
            IntentKind::Goal(MotorGoal::Hold) => thought.ask = Ask::Hold,
            IntentKind::Speed { cms } => {
                thought.ask = if *cms > 0 {
                    Ask::MakeForPort
                } else {
                    Ask::Hold
                };
            }
            IntentKind::Emcon { on } => thought.emcon = Some(*on),
            // `B-4`: the seat has no way to open fire, to mine or to trade. A lock, a shot
            // and an arming are dropped here whatever source asked for them.
            _ => {}
        }
    }
    thought
}

/// The view of a hull outside every bubble: no contacts, and the motor's `no_route` while the
/// planner's last refusal stands.
fn tier1_view(hull: ObjId, wall_now_s: i64, no_route: bool) -> MindView {
    MindView {
        unit: hull.0,
        callsign: Designator::Unit {
            party: CARRIER_PARTY,
            letter: 'A',
        },
        party: CARRIER_PARTY,
        cell: Some((8, 8)),
        mode: MatchMode::Skirmish,
        own_last_hit_ms: None,
        objective: None,
        motor: no_route.then_some(FrameMotor {
            requested_goal: (8, 8),
            outcome: Some(DriveOutcome::NoRoute),
        }),
        x_cm: 0,
        y_cm: 0,
        z_cm: 0,
        heading_mdeg: 0,
        speed_cms: 0,
        power: Power::Online,
        held: false,
        wreck: false,
        immobile: false,
        sight_m: 250_000,
        ecm: Some(false),
        receiver: Some(FrameReceiver {
            budget: 100,
            budget_max: 100,
            reach_permille: 1_000,
        }),
        heat: None,
        sections: Vec::new(),
        lock: None,
        weapons: Vec::new(),
        rules: Vec::new(),
        order: None,
        t_ms: wall_now_s.saturating_mul(1_000),
        notices: Vec::new(),
        contacts: Vec::new(),
        scan_tables: Vec::new(),
        odds_floor_permille: 0,
    }
}

/// Where a hull the backup mind has is, as the seat sorts her.
enum Whereabouts {
    /// In an interdiction bubble: [`fly_in_bubble`] flies her on the bubble's own step.
    Bubble,
    /// In free flight or at a cluster: the seat may engage a passage.
    Adrift,
    /// On a corridor journey: she is already flying to a port.
    Corridor,
    /// None of these: she has docked, or is no longer in space.
    Alongside,
}

fn whereabouts(w: &World, hull: ObjId) -> Whereabouts {
    let book = w.module_state::<SpaceBubbleBook>();
    if book.is_some_and(|book| book.active_for_hull(hull).is_some()) {
        Whereabouts::Bubble
    } else if book.is_some_and(|book| {
        book.free_flights.contains_key(&hull) || book.cluster_sessions.contains_key(&hull)
    }) {
        Whereabouts::Adrift
    } else if w
        .module_state::<mush_sim::JourneyBook>()
        .is_some_and(|journeys| journeys.get(hull).is_some())
    {
        Whereabouts::Corridor
    } else {
        Whereabouts::Alongside
    }
}

/// Write `state` back over `hull`'s record, if she still has one.
fn keep(ctx: &mut Ctx<'_>, hull: ObjId, state: SpaceLinkdeadState) {
    if let Some(slot) = ctx
        .w
        .module_state_mut::<SpaceBubbleBook>()
        .and_then(|book| book.linkdead_hulls.get_mut(&hull))
        && *slot != state
    {
        *slot = state;
    }
}

/// Step every hull a backup mind has that is outside a bubble (doc 35 §11): release one that
/// has docked, leave one on a corridor to her journey, and think for one adrift.
pub(crate) fn step_backup_minds(ctx: &mut Ctx<'_>) {
    if ctx.w.world_timeline().is_frozen() {
        return;
    }
    let held: Vec<SpaceLinkdeadState> = ctx
        .w
        .module_state::<SpaceBubbleBook>()
        .map(|book| {
            book.linkdead_hulls
                .values()
                .filter(|state| state.backup_has_helm())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for mut state in held {
        let hull = state.hull;
        match whereabouts(ctx.w, hull) {
            Whereabouts::Bubble => {}
            Whereabouts::Alongside => {
                // `B-1`: she is home, or no longer a hull under way (a derelict, say); the
                // seat's work is done either way.
                if let Some(book) = ctx.w.module_state_mut::<SpaceBubbleBook>() {
                    book.linkdead_hulls.remove(&hull);
                }
                if matches!(
                    crate::port::ship::berth_of(ctx.w, hull),
                    crate::port::ship::Berth::Docked { .. }
                ) {
                    crate::port::voyage::narrate_hull(
                        ctx,
                        hull,
                        "[SPACE] Backup mind: she is alongside; the helm is released.",
                    );
                }
            }
            Whereabouts::Corridor => {
                // `B-6`: a hull on a corridor is already flying to her filed port.
                if state.backup_act != BackupAct::MakingForPort {
                    state.backup_act = BackupAct::MakingForPort;
                    keep(ctx, hull, state);
                }
            }
            Whereabouts::Adrift => step_adrift(ctx, state),
        }
    }
}

/// One think for a hull in free flight or at a cluster, and the seat's act on it.
fn step_adrift(ctx: &mut Ctx<'_>, mut state: SpaceLinkdeadState) {
    let hull = state.hull;
    let wall_now_s = ctx.w.now();
    if wall_now_s < state.next_think_at_wall_s {
        return;
    }
    let no_route = wall_now_s < state.return_retry_at_wall_s;
    let Some(thought) = think(
        ctx.w,
        hull,
        tier1_view(hull, wall_now_s, no_route),
        &mut state,
    ) else {
        keep(ctx, hull, state);
        return;
    };
    let mut line = None;
    match thought.ask {
        Ask::Nothing => {}
        Ask::Hold => {
            if !matches!(
                state.backup_act,
                BackupAct::Holding | BackupAct::HoldingForRetrieval
            ) {
                state.backup_act = BackupAct::Holding;
                line = Some("[SPACE] Backup mind: holding where she is.".to_owned());
            }
            // A hull that holds is thought for again when the planner's wait is out, or in a
            // minute where there is none.
            state.next_think_at_wall_s = if no_route {
                state.return_retry_at_wall_s
            } else {
                wall_now_s.saturating_add(BACKUP_RETURN_RETRY_WALL_S)
            };
        }
        // Inside the wait the seat does not ask the planner again, whatever the mind asks.
        Ask::MakeForPort if no_route => {}
        Ask::MakeForPort => {
            // The engine holds one submission an actor: a passage engaged behind another
            // would be flown on a debit that was turned away. Wait a tick instead.
            let payer = super::commands::resolve_hull_captain_aboard(ctx.w, hull, ctx.actor);
            if ctx.authoritative_pending(payer) || ctx.authoritative_queue_full() {
                keep(ctx, hull, state);
                return;
            }
            let port = state
                .queued_return_port
                .clone()
                .or_else(|| super::commands::return_port_for_hull(ctx.w, hull))
                .unwrap_or_default();
            match super::commands::execute_linkdead_return_journey(ctx, hull, &port) {
                Ok(Some(engaged)) => {
                    state.backup_act = BackupAct::MakingForPort;
                    state.return_retry_at_wall_s = 0;
                    line = Some(format!(
                        "[SPACE] Backup mind: making for {}: burn {} t reaction mass, {} crew-days, transit {} IC days.",
                        engaged.port_label,
                        engaged.burn_t,
                        engaged.crew_days,
                        crate::port::voyage::ic_days(engaged.total_s)
                    ));
                }
                Ok(None) => {}
                Err(refusal) => {
                    state.return_retry_at_wall_s =
                        wall_now_s.saturating_add(BACKUP_RETURN_RETRY_WALL_S);
                    if state.backup_act != BackupAct::HoldingForRetrieval {
                        state.backup_act = BackupAct::HoldingForRetrieval;
                        line = Some(format!(
                            "[SPACE] Backup mind: no port is within her tank and stores ({refusal}); she holds on her life-support reserve and waits for a tow."
                        ));
                    }
                }
            }
        }
    }
    keep(ctx, hull, state);
    if let Some(line) = line {
        crate::port::voyage::narrate_hull(ctx, hull, &line);
    }
}

/// What [`fly_in_bubble`] leaves for [`settle_bubble_backup`], which has the world.
#[derive(Debug, Default)]
pub(crate) struct BubbleBackup {
    /// She is hove to under a customs hail: submit her to inspection.
    submit: bool,
    /// The crew's line for a change of act.
    line: Option<&'static str>,
}

/// Fly `hull` for one step of her bubble (doc 35 §11 `B-5`): think on what her sensors hold,
/// and on `hold()` brake her to rest against the picket over the `dt_wall_ms` the bubble is
/// about to step. Her helm is live under the seat, so the linkdead drive cut is lifted; her
/// point defence stays on AUTO.
pub(crate) fn fly_in_bubble(
    w: &World,
    active: &mut ActiveInterdictionBubble,
    hull: ObjId,
    ship_id: &str,
    state: &mut SpaceLinkdeadState,
    dt_wall_ms: u32,
) -> BubbleBackup {
    let mut flown = BubbleBackup::default();
    let Some(ship) = active.bubble.ships.get_mut(ship_id) else {
        return flown;
    };
    if ship.status != HullStatus::Operational {
        return flown;
    }
    ship.crew_linkdead = false;
    ship.pdc_auto = true;

    let customs_holds = active.picket.as_ref().is_some_and(|picket| {
        active
            .bubble
            .ships
            .get(&picket.corvette_ship_id)
            .is_some_and(|corvette| corvette.status == HullStatus::Operational)
            && matches!(
                picket.inspection_status_for(hull),
                PicketInspectionStatus::AwaitingHeaveTo
                    | PicketInspectionStatus::HoveToReadyForInspection
            )
    });
    let Some((mut view, _)) = super::picket::space_mind_view(&active.bubble, ship_id, hull) else {
        return flown;
    };
    view.held = customs_holds;
    let Some(thought) = think(w, hull, view, state) else {
        return flown;
    };

    if let Some(on) = thought.emcon {
        let posture = if on {
            EmconPosture::ActiveCombat
        } else {
            EmconPosture::Standard
        };
        if active
            .bubble
            .ships
            .get(ship_id)
            .is_some_and(|ship| ship.emcon != posture)
        {
            let _ = active.tape.record_and_apply(
                &mut active.bubble,
                BubbleInputCommand::SetEmcon {
                    ship_id: ship_id.to_owned(),
                    posture,
                },
            );
        }
    }

    // She heaves to against the picket's corvette, or the interdictor where there is none.
    let reference_id = active
        .picket
        .as_ref()
        .map_or(active.interdictor_ship_id.as_str(), |picket| {
            picket.corvette_ship_id.as_str()
        });
    let reference_vel = active
        .bubble
        .ships
        .get(reference_id)
        .map_or([0, 0, 0], |ship| ship.vel_mmps);
    let Some(ship) = active.bubble.ships.get(ship_id) else {
        return flown;
    };
    let burning = ship.thrust_mmps2 != [0, 0, 0] || ship.thrust_mps2 != [0, 0, 0];
    let rel_speed_mmps = super::picket::relative_speed_3d_mmps(ship.vel_mmps, reference_vel);
    let thrust_mmps2 = if thought.ask == Ask::Hold && rel_speed_mmps >= BRAKE_REST_MMPS {
        let max_mmps2 = if ship.max_accel_mmps2 > 0 {
            ship.max_accel_mmps2
        } else {
            ship.max_accel_mps2.saturating_mul(1_000)
        }
        .max(1);
        // The acceleration that brings her to rest over this step, up to what her drive gives.
        let want_mmps2 = rel_speed_mmps
            .saturating_mul(1_000)
            .checked_div(i64::from(dt_wall_ms.max(1)))
            .unwrap_or(max_mmps2)
            .clamp(1, max_mmps2);
        let against = |own: i64, other: i64| -> i64 {
            let rel = own.saturating_sub(other);
            let scaled = i128::from(rel) * i128::from(want_mmps2) / i128::from(rel_speed_mmps);
            i64::try_from(-scaled).unwrap_or(0)
        };
        let [vx, vy, vz] = ship.vel_mmps;
        let [rx, ry, rz] = reference_vel;
        [against(vx, rx), against(vy, ry), against(vz, rz)]
    } else {
        [0, 0, 0]
    };
    if thrust_mmps2 != [0, 0, 0] || burning {
        // A stop for customs is flown into the return reserve if it must be: a hull that
        // coasts past the picket's boundary has run it.
        let _ = active.tape.record_and_apply(
            &mut active.bubble,
            BubbleInputCommand::CommandBurnMmps2 {
                ship_id: ship_id.to_owned(),
                thrust_mmps2,
                force_into_reserve: true,
            },
        );
    }

    if thought.ask == Ask::Hold {
        if customs_holds && rel_speed_mmps < HEAVE_TO_REL_VEL_MMPS {
            flown.submit = true;
        } else if state.backup_act != BackupAct::HeavingTo && rel_speed_mmps >= BRAKE_REST_MMPS {
            state.backup_act = BackupAct::HeavingTo;
            flown.line = Some(if customs_holds {
                "[SPACE] Backup mind: heaving to for the customs picket."
            } else {
                "[SPACE] Backup mind: heaving to."
            });
        }
    }
    flown
}

/// Finish one hull's bubble step with the world in hand: say the seat's line, and submit a
/// hull that is hove to under a customs hail to inspection, as her captain (doc 35 §11 `B-5`).
pub(crate) fn settle_bubble_backup(ctx: &mut Ctx<'_>, hull: ObjId, flown: BubbleBackup) {
    if let Some(line) = flown.line {
        crate::port::voyage::narrate_hull(ctx, hull, line);
    }
    if !flown.submit {
        return;
    }
    // One ruling at a time (`SF.R26c`): an earlier one on her is still on its way to the store,
    // and the picket's verdict follows it on the space tick.
    if super::customs::ruling_pending(ctx.w, hull) {
        return;
    }
    // The ruling is one submitted action, and the engine holds one submission an actor: with
    // one of her captain's still in flight the seat waits a tick.
    let captain = super::commands::resolve_hull_captain_aboard(ctx.w, hull, ctx.actor);
    if ctx.authoritative_pending(captain) || ctx.authoritative_queue_full() {
        return;
    }
    let saved = ctx.actor;
    ctx.actor = captain;
    let ruled = super::commands::submit_hull_to_customs(ctx, hull);
    ctx.actor = saved;
    match ruled {
        Ok(()) => {
            if let Some(state) = ctx
                .w
                .module_state_mut::<SpaceBubbleBook>()
                .and_then(|book| book.linkdead_hulls.get_mut(&hull))
            {
                state.backup_act = BackupAct::Inspected;
            }
        }
        Err(refusal) => tracing::debug!(
            hull = hull.0,
            %refusal,
            "tacsim: the backup seat's customs submission was refused; it asks again next step"
        ),
    }
}
