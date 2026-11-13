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
//! 1. the mind her holder sealed aboard (doc 44 unit MW.13; `crate::port::ship_mind`), while
//!    its seal still fits her core and her holder: a filed mind under the Roll's claim, on
//!    its own class row and with its own rule program;
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
//! | `lock` | dropped | dropped for a house default; her lock, for a mind sealed aboard |
//! | `fire`, `arm` | dropped | dropped |
//!
//! A mind sealed aboard sits in the same seat as the house default, with one difference
//! (doc 44 unit MW.13; Q-44-26, "helm and sensors"): its sensors' act, a lock, is let
//! through, by its own `lock` intent or by a `LOCK NEAREST` row of its rule program
//! ([`super::applier::program_lock`]). It has no fire channel, so a lock of its is no half
//! of a shot. Its burns are held to the rule every source is held to.
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
use super::thinks::{Answer, Ask, Due, Owed, Owner, Source, ThinkHost, Thinker};
use super::tick::{ActiveInterdictionBubble, SpaceBubbleBook, SpaceLinkdeadState};
use crate::arena::intents::{IntentKind, MotorGoal};
use crate::arena::mind::{MindView, ThinkResult};
use crate::arena::perception::{DriveOutcome, Receiver as FrameReceiver};
use crate::arena::state::{Designator, MatchMode, Power};
use crate::arena::step::FrameMotor;
use crate::minds::content::{LINKDEAD_RETURN_ROLE, linkdead_return_digest};
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
enum Asks {
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
    ask: Asks,
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

/// **The sources the seat tries for `hull`, first choice first** (doc 35 §11 `B-3`): the
/// mind her holder sealed aboard, the store's house default, and the compiled house default.
/// One think is collected on each ([`super::thinks::Ask::jobs`]) and the first that answers
/// without a fault is hers; until doc 44 unit MW.11 they were tried one after another on the
/// world thread.
///
/// **The sealed mind is the Roll's claim** (doc 44 unit MW.13): `crate::port::ship_mind`'s
/// reader answers it only while the seal still fits the hull's core and its dispatcher still
/// has authority over the hull. Until that unit this read the hull's `tacsim:mind_seal`
/// attribute, which only a cast hull or a drone ever carried, and which a salvaged hull kept;
/// the seat no longer reads it.
fn sources(w: &World, hull: ObjId) -> Vec<Source> {
    let store = w.module_state::<TacsimStore>();
    let house = store.and_then(|store| store.house_doctrine(LINKDEAD_RETURN_ROLE));
    let compiled = linkdead_return_digest().0;
    let mut tried: Vec<Source> = Vec::new();
    if let Some(mind) = crate::port::ship_mind::thinker(w, hull) {
        tried.push(Source::Sealed(mind));
    }
    for digest in [house, Some(compiled)].into_iter().flatten() {
        if !tried.iter().any(|source| source.digest() == digest) {
            tried.push(Source::Doctrine(digest));
        }
    }
    tried
}

/// Read a clean think into the seat's terms and keep its memory and its last trace word.
fn read(result: ThinkResult, state: &mut SpaceLinkdeadState) -> Thought {
    state.mind_think_seq = state.mind_think_seq.saturating_add(1);
    if state.mind_memory != result.memory {
        state.mind_memory = result.memory;
    }
    if let Some(word) = result.trace.last()
        && state.mind_last_trace.as_deref() != Some(word.as_str())
    {
        state.mind_last_trace = Some(word.clone());
    }
    let mut ask = Asks::Nothing;
    for intent in &result.intents {
        match intent {
            IntentKind::Goal(MotorGoal::Hold) => ask = Asks::Hold,
            IntentKind::Speed { cms } => {
                ask = if *cms > 0 {
                    Asks::MakeForPort
                } else {
                    Asks::Hold
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
fn tier1_view(
    hull: ObjId,
    wall_now_s: i64,
    no_route: bool,
    posture: crate::posture::Posture,
) -> MindView {
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
        posture,
    }
}

/// Where a hull the backup mind has is, as the seat sorts her.
enum Whereabouts {
    /// In an interdiction bubble: she is flown on the bubble's own step ([`land_in_bubble`]).
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
    if let Some(host) = super::thinks::host_of(ctx.w) {
        // A hull whose record is gone (her crew is back, she is lost) leaves no think behind.
        let flown: std::collections::BTreeSet<ObjId> =
            held.iter().map(|state| state.hull).collect();
        host.keep_hulls(|owner| match owner {
            Owner::Adrift(hull) => flown.contains(&hull),
            Owner::Bubble(_) | Owner::Cluster(_) => true,
        });
    }
    for mut state in held {
        let hull = state.hull;
        let here = whereabouts(ctx.w, hull);
        if !matches!(here, Whereabouts::Adrift)
            && let Some(host) = super::thinks::host_of(ctx.w)
        {
            // Doc 44 unit MW.11: a think collected while she was adrift is not hers any more.
            host.discard(Owner::Adrift(hull));
        }
        match here {
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

/// The mark of a hull's thinks outside a bubble: the wall second her crew's link dropped, so
/// a think from an earlier loss of link is never taken for this one's.
fn adrift_mark(state: &SpaceLinkdeadState) -> super::thinks::Mark {
    (state.grace_started_wall_s, 0)
}

/// **A hull in free flight or at a cluster** (doc 35 §11; doc 44 unit MW.11): the seat acts
/// on the think she owes, if it has answered, and collects her next. Nothing here waits for
/// a worker: with her think still out on a server's host she is left as she is until a later
/// tick.
fn step_adrift(ctx: &mut Ctx<'_>, state: SpaceLinkdeadState) {
    let Some(host) = super::thinks::host_of(ctx.w) else {
        return;
    };
    let Some(state) = land_adrift(ctx, &host, state) else {
        return;
    };
    collect_adrift(ctx, &host, &state);
}

/// Collect one think for a hull outside every bubble, when she is due one.
fn collect_adrift(ctx: &Ctx<'_>, host: &ThinkHost, state: &SpaceLinkdeadState) {
    let hull = state.hull;
    let wall_now_s = ctx.w.now();
    if wall_now_s < state.next_think_at_wall_s {
        return;
    }
    let no_route = wall_now_s < state.return_retry_at_wall_s;
    let view = Arc::new(tier1_view(
        hull,
        wall_now_s,
        no_route,
        // Doc 45 unit AP.3: the posture her player left on her, off her own record.
        crate::posture::record::of(ctx.w, hull),
    ));
    let store = ctx.w.module_state::<TacsimStore>();
    let (jobs, sealed) = host.jobs_on(
        store,
        hull,
        &sources(ctx.w, hull),
        state.mind_think_seq,
        &view,
        &state.mind_memory,
    );
    let ask = Ask {
        who: Thinker::Backup,
        unit: hull,
        think_seq: state.mind_think_seq,
        contacts: std::collections::BTreeMap::new(),
        jobs,
        sealed,
    };
    let _ = host.submit(Owner::Adrift(hull), adrift_mark(state), vec![ask]);
}

/// **Act on the think a hull outside every bubble owes** (doc 35 §11). Answers her record
/// as it now stands, for the next think to be collected on; `None` when her think has not
/// answered (a server's host holds her), or she is no longer the seat's to fly adrift.
fn land_adrift(
    ctx: &mut Ctx<'_>,
    host: &ThinkHost,
    mut state: SpaceLinkdeadState,
) -> Option<SpaceLinkdeadState> {
    let hull = state.hull;
    let owner = Owner::Adrift(hull);
    let mark = adrift_mark(&state);
    if !host.in_flight(owner, mark) {
        return Some(state);
    }
    let answer = match host.settle(owner, mark, Due::default()) {
        Owed::Held => return None,
        Owed::Nothing => return Some(state),
        Owed::Ready { mut answers, .. } => answers.pop(),
    };
    let Some(result) = answer
        .filter(|answer| answer.think_seq == state.mind_think_seq)
        .and_then(|answer| answer.result)
    else {
        tracing::warn!(
            hull = hull.0,
            "tacsim: a link-loss backup mind did not answer; she is thought for again"
        );
        return Some(state);
    };
    let wall_now_s = ctx.w.now();
    let no_route = wall_now_s < state.return_retry_at_wall_s;
    let thought = read(result, &mut state);
    let mut line = None;
    match thought.ask {
        Asks::Nothing => {}
        Asks::Hold => {
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
        Asks::MakeForPort if no_route => {}
        Asks::MakeForPort => {
            // The engine holds one submission an actor: a passage engaged behind another
            // would be flown on a debit that was turned away. Wait a tick instead.
            let payer = super::commands::resolve_hull_captain_aboard(ctx.w, hull, ctx.actor);
            if ctx.authoritative_pending(payer) || ctx.authoritative_queue_full() {
                keep(ctx, hull, state);
                return still_adrift(ctx, hull);
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
    still_adrift(ctx, hull)
}

/// `hull`'s record as the book now holds it, if the seat still flies her outside a bubble.
fn still_adrift(ctx: &Ctx<'_>, hull: ObjId) -> Option<SpaceLinkdeadState> {
    if !matches!(whereabouts(ctx.w, hull), Whereabouts::Adrift) {
        return None;
    }
    ctx.w
        .module_state::<SpaceBubbleBook>()
        .and_then(|book| book.linkdead_hulls.get(&hull))
        .filter(|state| state.backup_has_helm())
        .cloned()
}

/// What [`land_in_bubble`] leaves for [`settle_bubble_backup`], which has the world.
#[derive(Debug, Default)]
pub(crate) struct BubbleBackup {
    /// She is hove to under a customs hail: submit her to inspection.
    submit: bool,
    /// The crew's line for a change of act.
    line: Option<&'static str>,
}

/// **The seat's own upkeep of a hull it flies in a bubble** (doc 35 §11 `B-5`), at the head
/// of each of the bubble's steps: her helm is live under the seat, so the linkdead drive cut
/// is lifted; her point defence stays on AUTO.
pub(crate) fn seat_in_bubble(active: &mut ActiveInterdictionBubble, ship_id: &str) {
    if let Some(ship) = active.bubble.ships.get_mut(ship_id)
        && ship.status == HullStatus::Operational
    {
        ship.crew_linkdead = false;
        ship.pdc_auto = true;
    }
}

/// **Collect `hull`'s think on the step her bubble has just taken** (doc 35 §11 `B-5`; doc 44
/// units MW.9 and MW.11): on the view every space mind reads, with `step_ms` as the step the
/// bubble flies before she thinks again, once for each source the seat tries. The answer
/// lands at the head of the bubble's next step ([`land_in_bubble`]).
pub(crate) fn collect_in_bubble(
    w: &World,
    host: &ThinkHost,
    active: &ActiveInterdictionBubble,
    hull: ObjId,
    ship_id: &str,
    state: &mut SpaceLinkdeadState,
    step_ms: u32,
) -> Option<Ask> {
    if active
        .bubble
        .ships
        .get(ship_id)
        .is_none_or(|ship| ship.status != HullStatus::Operational)
    {
        return None;
    }
    let mut situation = active.situation(step_ms);
    // Doc 45 unit AP.3: the posture her player left on her, off her own record, so her mind
    // reads in a bubble the word it reads outside one.
    situation
        .postures
        .insert(ship_id.to_owned(), crate::posture::record::of(w, hull));
    super::picket::collect_bubble_think_on(
        host,
        w.module_state::<TacsimStore>(),
        &active.bubble,
        Thinker::Backup,
        ship_id,
        hull,
        &sources(w, hull),
        state.mind_think_seq,
        &state.mind_memory,
        &mut state.mind_contacts,
        &situation,
    )
}

/// **Land `hull`'s think at the head of her bubble's step** (doc 35 §11 `B-5`; doc 44 units
/// MW.9 and MW.11): hand what the mind asked to the one applier, seated [`Seat::ABSENT`],
/// before the bubble steps `dt_wall_ms`. The seat's rules are read off the bubble as it
/// stands now, not as the think saw it. The seat adds nothing to her flying: a burn that
/// heaves her to is the mind's own. An answer for a think her record has moved past, or
/// with no clean result, changes nothing.
pub(crate) fn land_in_bubble(
    active: &mut ActiveInterdictionBubble,
    ship_id: &str,
    state: &mut SpaceLinkdeadState,
    answer: Answer,
    dt_wall_ms: u32,
) -> BubbleBackup {
    let mut flown = BubbleBackup::default();
    if answer.think_seq != state.mind_think_seq
        || active
            .bubble
            .ships
            .get(ship_id)
            .is_none_or(|ship| ship.status != HullStatus::Operational)
    {
        return flown;
    }
    let Some(result) = answer.result else {
        tracing::warn!(
            hull = state.hull.0,
            "tacsim: a link-loss backup mind did not answer; she is thought for again"
        );
        return flown;
    };
    // Doc 44 unit MW.13: a mind sealed aboard brings its rule program. A `LOCK NEAREST` row
    // of it that holds asks a lock ahead of the think's own intents, so a `lock` the script
    // asks this think is the one that stands.
    let sealed = answer.sealed.as_ref();
    let rule_lock = sealed.and_then(|mind| {
        super::applier::program_lock(
            mind.program(),
            result.rule_sample.as_ref(),
            &active.bubble,
            ship_id,
            &answer.contacts,
        )
    });
    let mut thought = read(result, state);
    if let Some(contact) = rule_lock {
        thought.intents.insert(0, IntentKind::Lock { contact });
    }
    let situation = active.situation(dt_wall_ms);
    let stop = situation.stop_of(ship_id);
    let customs_holds = active.picket.is_some() && stop.holds();
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
            // Doc 44 unit MW.13 (Q-44-26): her own sealed mind has her sensors.
            lock: sealed.is_some(),
            ..Seat::ABSENT
        },
        &answer.contacts,
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

#[cfg(test)]
mod tests {
    use mush_core::ObjId;

    use super::tier1_view;
    use crate::posture::Posture;

    /// Doc 45 unit AP.3: a hull outside every bubble reads the posture its seat hands the
    /// view, which the seat reads off the hull's own record.
    #[test]
    fn a_hull_outside_every_bubble_reads_the_posture_it_is_given() {
        for posture in Posture::ALL {
            let view = tier1_view(ObjId(7), 100, false, posture);
            assert_eq!(view.posture, posture);
            assert!(view.space.is_none() && view.contacts.is_empty());
        }
    }
}
