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
//! limits are kept, so no source can step past them. Inside a bubble the mind's intents go
//! through the one applier every space mind uses ([`super::applier::apply_intents`], doc 44
//! unit MW.9), seated [`Seat::ABSENT`]:
//!
//! | The mind asks | Outside a bubble | Inside a bubble |
//! |---|---|---|
//! | `speed(n)`, n above nought | the return passage, by the ruled order | while a stop holds her, a burn of that strength along its `helm(word)` if it takes her drift against the stopping ship off; any other burn is dropped and she coasts |
//! | `hold()` | she holds where she is | her drive is cut; hove to under a customs hail, she is submitted to inspection |
//! | `emcon(on)` | nothing | her emission posture, COMBAT or STANDARD |
//! | `lock`, `fire`, `arm` | dropped | dropped |
//!
//! The return is `execute_linkdead_return_journey` (`commands.rs`): the filed port, then
//! every open port nearest first, each through the planner with its tank and locker checks and
//! its debit. Nothing here draws from a vault; the seat has no verb that mines or trades.
//!
//! What the mind reads: inside a bubble, the view every space mind reads
//! ([`super::view::space_mind_view`]): her contacts, her own velocity and drive, and the stop
//! a picket or an interdictor has on her (`space(v).stop`; `v.held` is true while it holds
//! her). The burn that heaves her to is the mind's own, worked out from that view. Outside a
//! bubble, `v.motor.outcome == "no_route"` for [`BACKUP_RETURN_RETRY_WALL_S`] after every
//! open port was refused.

use std::sync::Arc;

use mush_core::{Ctx, ObjId, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::applier::{BurnRule, Seat};
use super::bubble::HullStatus;
use super::picket::{CARRIER_PARTY, HEAVE_TO_REL_VEL_MMPS};
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

/// One think, read: what it asks of the seat outside a bubble, and the intents themselves,
/// which inside a bubble go to the applier.
struct Thought {
    ask: Ask,
    intents: Vec<IntentKind>,
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
    let mut ask = Ask::Nothing;
    for intent in &result.intents {
        match intent {
            IntentKind::Goal(MotorGoal::Hold) => ask = Ask::Hold,
            IntentKind::Speed { cms } => {
                ask = if *cms > 0 {
                    Ask::MakeForPort
                } else {
                    Ask::Hold
                };
            }
            // `B-4`: outside a bubble the seat has two acts, a passage and a hold. Inside
            // one the applier's absent seat drops a lock, a shot and an arming whatever
            // source asked for them.
            _ => {}
        }
    }
    Thought {
        ask,
        intents: result.intents,
    }
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
        space: None,
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

/// Fly `hull` for one step of her bubble (doc 35 §11 `B-5`; doc 44 unit MW.9): think on the
/// view every space mind reads, with the `dt_wall_ms` the bubble is about to step as its
/// step, and hand what the mind asks to the one applier, seated [`Seat::ABSENT`]. Her helm is
/// live under the seat, so the linkdead drive cut is lifted; her point defence stays on AUTO.
/// The seat adds nothing to her flying: a burn that heaves her to is the mind's own.
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

    let situation = active.situation(dt_wall_ms);
    let stop = situation.stop_of(ship_id);
    let customs_holds = active.picket.is_some() && stop.holds();
    let Some((view, contacts)) = super::view::space_mind_view(
        &active.bubble,
        ship_id,
        hull,
        &mut state.mind_contacts,
        &situation,
    ) else {
        return flown;
    };
    let Some(thought) = think(w, hull, view, state) else {
        return flown;
    };
    // `B-4`: she never runs a picket under power. While a stop holds her the one burn the seat
    // lets her mind fly is one that takes her drift against the stopping ship off; with no
    // stop on her it flies none.
    let stopper_vel = situation
        .stopper
        .as_deref()
        .and_then(|stopper| active.bubble.ships.get(stopper))
        .map(|ship| ship.vel_mmps);
    let burn = match (stop.holds(), stopper_vel, active.bubble.ships.get(ship_id)) {
        (true, Some(theirs), Some(ship)) => BurnRule::Toward([
            theirs[0].saturating_sub(ship.vel_mmps[0]),
            theirs[1].saturating_sub(ship.vel_mmps[1]),
            theirs[2].saturating_sub(ship.vel_mmps[2]),
        ]),
        _ => BurnRule::Never,
    };
    let mut answered = Vec::new();
    let applied = super::applier::apply_intents(
        &mut active.bubble,
        &mut active.tape,
        ship_id,
        Seat {
            burn,
            ..Seat::ABSENT
        },
        &contacts,
        &thought.intents,
        &mut answered,
    );

    // What the seat does with the world in hand, and what the crew are told.
    let hove_to = active.bubble.ships.get(ship_id).is_some_and(|ship| {
        stopper_vel.is_some_and(|stopper_vel| {
            super::picket::relative_speed_3d_mmps(ship.vel_mmps, stopper_vel)
                < HEAVE_TO_REL_VEL_MMPS
        })
    });
    if applied.asked_hold && customs_holds && hove_to {
        flown.submit = true;
    } else if applied.asked_burn && stop.holds() && state.backup_act != BackupAct::HeavingTo {
        state.backup_act = BackupAct::HeavingTo;
        flown.line = Some(if customs_holds {
            "[SPACE] Backup mind: heaving to for the customs picket."
        } else {
            "[SPACE] Backup mind: heaving to."
        });
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
