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
//! | `lock` | dropped | for a house default, only on a ship she may answer; her lock, for a mind sealed aboard |
//! | `fire` | dropped | for a house default, only at a ship she may answer; dropped for a mind sealed aboard, beside which the house default's own shots land (doc 47 unit CO.2) |
//! | `arm` | dropped | dropped |
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
//! # The posture her player left on her
//!
//! Doc 45 unit AP.9 (Q-45-23, Q-45-24; Q-45-7, Q-45-8, Q-45-12). A ship carries the one
//! posture every unit carries, on her own record (`crate::posture::record`), and the seat
//! reads it there and nowhere else. What the seat holds a mind to, whichever source thinks:
//!
//! | | Outside a bubble | Inside a bubble |
//! |---|---|---|
//! | `RETURN` | the return passage, as above | she heaves to when stopped; the house default may answer a ship whose fire has arrived on her (doc 47 unit CO.2, R-47-3), and she is on her passage again when the bubble lets her go |
//! | `HOLD` | she holds where she is: a `speed(n)` is read as a `hold()` | the same helm and the same answer as `RETURN` |
//! | `GUARD` | as `HOLD` | as `HOLD`, and the house default may also answer a ship whose fire has arrived on another unit of her player in the bubble |
//!
//! **One gate.** What she may answer is `crate::arena::absent::permits`, the ground's rule,
//! asked by the applier at a lock and at a shot with the facts [`standing`] reads off the
//! bubble ([`super::applier::Answers`]): never first, never at a party her party only marks
//! or holds hostile, and nothing that mines or trades (the seat has no such act).
//!
//! **Holding station.** A hull adrift or at a cluster stays where she is. A hull on a
//! passage keeps her passage (Q-44-45): her plan is her player's own act, and the seat
//! never thinks for a hull on a corridor. Inside a bubble her drive is cut and she keeps the
//! way she had; the one burn the seat lets through is the one that heaves her to, under
//! every posture, and when the bubble lets her go she is on her passage again.
//!
//! **She complies with customs** (`complying_with`). While a picket's or an interdictor's
//! stop on her stands, and after it has cleared her, the ship that stopped her is not one she
//! answers, whatever arrived from it. A hull that ran is past that.
//!
//! **A hull that cannot defend herself** ([`flown`]) is read as on the ground
//! (`crate::arena::absent::flown_as`): on `GUARD` with nothing to fire she is flown as
//! `HOLD`; on `GUARD` with her drive destroyed or nothing left to fire she is flown as
//! `RETURN`. Her record is not rewritten.
//!
//! **A mind sealed aboard** (doc 44 unit MW.13) keeps her helm and her sensors under every
//! posture and has no fire channel. **The house default answers fire beside it** (doc 47 unit
//! CO.2, R-47-2): the think host runs every source the seat tries, so the house default's
//! think on the same step is already there; its fire channel alone lands
//! ([`super::applier::apply_answer`]), fired at its ship without moving her lock, so the
//! sealed mind's helm, lock and emission posture stand. The sealed mind reads the bubble as
//! her sensors hold it; the house default reads the view narrowed to what she may answer.
//! (Until CO.2 nothing answered fire for her: AP.9's default 1, which the owner ruled.)
//!
//! **Whose she is** ([`player_of`]): whoever commands her by charter or title
//! (`crate::port::coop::helm_holder`), if that is a player. Her player's other hulls in the
//! bubble are the units a `GUARD` answers for. When she stands guard and a ship has fired on
//! one of them, the seat marks that ship's party hostile in her own party's book
//! (`collect_in_bubble`): each carrier is a party of its own in a bubble, so without the
//! mark the bubble would refuse the shot the gate lets through.
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

use super::applier::{Answers, BurnRule, FireRule, Seat};
use super::bubble::{HullStatus, LocalSpaceBubble, ShipSectionId, SpaceShipState};
use super::picket::{CARRIER_PARTY, HEAVE_TO_REL_VEL_MMPS};
use super::thinks::{Answer, Ask, Due, Owed, Owner, Source, ThinkHost, Thinker};
use super::tick::{ActiveInterdictionBubble, SpaceBubbleBook, SpaceLinkdeadState};
use crate::arena::intents::{IntentKind, MotorGoal};
use crate::arena::mind::{MindView, ThinkResult};
use crate::arena::perception::{DriveOutcome, Receiver as FrameReceiver};
use crate::arena::state::{Designator, MatchMode, Power};
use crate::arena::step::FrameMotor;
use crate::minds::content::{LINKDEAD_RETURN_ROLE, linkdead_return_digest};
use crate::posture::Posture;
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
    /// She holds where she is because her player left her on `HOLD` or `GUARD` (doc 45
    /// unit AP.9).
    HoldingStation,
    /// She has answered a ship that fired on her, or on another unit of her player (doc 45
    /// unit AP.9).
    ReturningFire,
}

/// The hull's line when she holds station on her posture: `{posture}`.
pub const BACKUP_HOLDS_STATION: &str =
    "[SPACE] Backup mind: holding station where she is; her posture is {posture}.";
/// The hull's line the first time she answers fire in a bubble.
pub const BACKUP_RETURNS_FIRE: &str = "[SPACE] Backup mind: returning fire.";

/// **Whose ship `hull` is** (the module head): whoever commands her by charter or title, if
/// that is a player. `None` for a cast hull, a hull laid up and a hull a non-player commands.
#[must_use]
pub fn player_of(w: &World, hull: ObjId) -> Option<ObjId> {
    crate::port::coop::helm_holder(w, hull).filter(|who| {
        w.get(*who)
            .is_some_and(|object| object.kind == mush_core::ObjKind::Player)
    })
}

/// Whether `ship` carries anything she can fire: a beam, a railgun or a tube.
fn armed(ship: &SpaceShipState) -> bool {
    !(ship.lasers.is_empty() && ship.railguns.is_empty() && ship.torpedo_tubes.is_empty())
}

/// **Whether `ship` can still defend herself** (Q-45-9, as `crate::arena::absent` reads it
/// of a unit on the ground): she flies, her drive section stands, and she has a beam, or a
/// railgun or a tube with a round left.
fn can_defend(ship: &SpaceShipState) -> bool {
    ship.status == HullStatus::Operational
        && ship
            .sections
            .get(&ShipSectionId::Drive)
            .is_none_or(|drive| drive.internals > 0)
        && (!ship.lasers.is_empty()
            || ship.railguns.iter().any(|mount| mount.rounds_remaining > 0)
            || ship
                .torpedo_tubes
                .iter()
                .any(|tube| tube.magazine_remaining > 0))
}

/// **The posture a hull the seat flies in a bubble is flown on** (the module head, "a hull
/// that cannot defend herself"): `record`, the word on her own record, read by the ground's
/// rule over what `ship` carries and can still do.
#[must_use]
pub fn flown(record: Posture, ship: &SpaceShipState) -> Posture {
    crate::arena::absent::flown_as(record, true, armed(ship), can_defend(ship))
}

/// **The ship `ship_id` complies with** (the module head, "she complies with customs"): the
/// ship that holds the stops in this bubble, while its stop on her stands or has cleared
/// her. `None` when nobody has stopped her, and when she ran.
fn complying_with<'a>(situation: &'a super::view::Situation, ship_id: &str) -> Option<&'a str> {
    use super::view::Stop;
    match situation.stop_of(ship_id) {
        Stop::Hailed | Stop::HoveTo | Stop::Cleared => situation.stopper.as_deref(),
        Stop::None | Stop::Ran => None,
    }
}

/// **What a hull flown on `flown` may answer** (the module head, "one gate"): the facts the
/// absent player's rule reads, off the bubble's record of whose fire has arrived on whom.
/// `kin` is her player's other units in the bubble, by ship id; `complying` the ship she
/// complies with. Under every posture she may answer a ship whose fire has arrived on her
/// (doc 47 unit CO.2, R-47-3: `RETURN` too); on `GUARD` also one whose fire has arrived on
/// another unit of her player.
///
/// Her player's own units and the ship she complies with are never among them, so neither
/// set can let her fire on one.
#[must_use]
pub fn answers_of(
    bubble: &LocalSpaceBubble,
    ship_id: &str,
    flown: Posture,
    kin: &[String],
    complying: Option<&str>,
) -> Answers {
    let answerable = |by: &&String| {
        by.as_str() != ship_id && Some(by.as_str()) != complying && !kin.contains(*by)
    };
    let arrived_on = |target: &str| -> std::collections::BTreeSet<String> {
        bubble
            .fired_on_by(target)
            .into_iter()
            .flatten()
            .filter(answerable)
            .cloned()
            .collect()
    };
    let guards = flown == Posture::Guard;
    Answers {
        fired_on_by: arrived_on(ship_id),
        guards,
        fired_on_kin: if guards {
            kin.iter().flat_map(|unit| arrived_on(unit)).collect()
        } else {
            std::collections::BTreeSet::new()
        },
    }
}

/// **How the seat stands `hull` in her bubble** ([`standing`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    /// The posture she is flown on ([`flown`]).
    pub posture: Posture,
    /// What she may answer, under every posture (doc 47 unit CO.2).
    pub answers: Answers,
}

/// **How the seat stands `hull`, as the bubble is now** (the module head): the posture off
/// her own record, read over what she carries; her player's other hulls in the bubble; the
/// ship she complies with; and so what she may answer.
#[must_use]
pub fn standing(
    w: &World,
    active: &ActiveInterdictionBubble,
    hull: ObjId,
    ship_id: &str,
    situation: &super::view::Situation,
) -> Standing {
    let record = crate::posture::record::of(w, hull);
    let posture = active
        .bubble
        .ships
        .get(ship_id)
        .map_or(Posture::Return, |ship| flown(record, ship));
    let kin: Vec<String> = match player_of(w, hull) {
        Some(player) => active
            .all_carrier_hulls()
            .into_iter()
            .filter(|(other, _, _, _)| *other != hull && player_of(w, *other) == Some(player))
            .map(|(_, other_ship, _, _)| other_ship)
            .collect(),
        None => Vec::new(),
    };
    let answers = answers_of(
        &active.bubble,
        ship_id,
        posture,
        &kin,
        complying_with(situation, ship_id),
    );
    Standing { posture, answers }
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

/// **What the seat does outside a bubble with what a think asked, by her posture** (the
/// module head, "holding station"; doc 45 unit AP.9): on `HOLD` and `GUARD` a passage asked
/// for is a hold. On `RETURN` the ask is the mind's own.
const fn held_to(ask: Asks, posture: Posture) -> Asks {
    match ask {
        Asks::MakeForPort if !posture.is_return() => Asks::Hold,
        ask => ask,
    }
}

/// **Whether the seat lets the mind that thought answer fire** (the module head, "a mind
/// sealed aboard"; doc 45 unit AP.9, doc 47 unit CO.2): the house default answers under the
/// gate, under every posture. A mind sealed aboard has no fire channel (doc 44 unit MW.13;
/// Q-44-26), under every posture: beside it the house default's own think answers
/// ([`super::applier::apply_answer`]).
const fn fire_rule(sealed: bool) -> FireRule {
    if sealed {
        FireRule::Never
    } else {
        FireRule::Answer
    }
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
    // Doc 45 unit AP.9 (the module head, "holding station"): a hull her player left on
    // `HOLD` or `GUARD` stays where she is. The seat reads a passage asked for as a hold,
    // so no source can take her home against her player's word.
    let posture = crate::posture::record::of(ctx.w, hull);
    let ask = held_to(thought.ask, posture);
    match ask {
        Asks::Nothing => {}
        Asks::Hold if !posture.is_return() => {
            if state.backup_act != BackupAct::HoldingStation {
                state.backup_act = BackupAct::HoldingStation;
                line = Some(BACKUP_HOLDS_STATION.replace("{posture}", posture.label()));
            }
            state.next_think_at_wall_s = wall_now_s.saturating_add(BACKUP_RETURN_RETRY_WALL_S);
        }
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
    /// What the bubble answered to her mind's commands: her own shots, since doc 45 unit
    /// AP.9. They go with the step's events.
    pub(crate) events: Vec<super::bubble::SpaceCombatEvent>,
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
    active: &mut ActiveInterdictionBubble,
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
    // Doc 45 units AP.3 and AP.9: the posture her player left on her, off her own record
    // and as she is flown on it, so her mind reads in a bubble the word it reads outside
    // one; and the ships she may answer, which are the only ones the house default's view
    // shows hostile (doc 47 unit CO.2: under every posture, `RETURN` too).
    let stood = standing(w, active, hull, ship_id, &situation);
    if stood.answers.guards {
        stand_guard(&mut active.bubble, ship_id, &stood.answers);
    }
    situation.postures.insert(ship_id.to_owned(), stood.posture);
    let sensors = situation.clone();
    situation
        .answerable
        .insert(ship_id.to_owned(), stood.answers.all());
    let store = w.module_state::<TacsimStore>();
    let sources = sources(w, hull);
    // Doc 47 unit CO.2 (R-47-2): a mind sealed aboard keeps her sensors, so it reads the
    // bubble as her sensors hold it, not narrowed to what she may answer: it has no fire
    // channel, and the narrowing is the fire channel's. The house default beside it, whose
    // shots are the ones landed, reads the narrowed view. Both views number the contacts
    // off the one book, so one map names the ships of both.
    let (first, rest) = match sources.split_first() {
        Some((Source::Sealed(_), rest)) => (&sources[..1], rest),
        _ => (&sources[..0], &sources[..]),
    };
    let mut ask = None;
    if !first.is_empty() {
        ask = super::picket::collect_bubble_think_on(
            host,
            store,
            &active.bubble,
            Thinker::Backup,
            ship_id,
            hull,
            first,
            state.mind_think_seq,
            &state.mind_memory,
            &mut state.mind_contacts,
            &sensors,
        );
    }
    let house = super::picket::collect_bubble_think_on(
        host,
        store,
        &active.bubble,
        Thinker::Backup,
        ship_id,
        hull,
        rest,
        state.mind_think_seq,
        &state.mind_memory,
        &mut state.mind_contacts,
        &situation,
    );
    match (ask, house) {
        (Some(mut sealed), Some(house)) => {
            sealed.jobs.extend(house.jobs);
            Some(sealed)
        }
        (sealed, house) => sealed.or(house),
    }
}

/// **A hull that stands guard takes up her player's quarrel** (the module head, "whose she
/// is"): the party of each ship whose fire has arrived on another unit of her player is
/// marked hostile in her own party's book, if it does not hold it hostile already. Only her
/// party's book moves. The mark is the host's, like a customs clearance and a stand-down:
/// it is made on the bubble and not through the tape.
fn stand_guard(bubble: &mut LocalSpaceBubble, ship_id: &str, answers: &Answers) {
    use crate::arena::party::{Mark, Relation};
    let Some(own) = bubble.ships.get(ship_id).map(|ship| ship.party) else {
        return;
    };
    let theirs: Vec<_> = answers
        .fired_on_kin
        .iter()
        .filter_map(|by| bubble.ships.get(by).map(|ship| ship.party))
        .filter(|party| *party != own && bubble.party_relation(own, *party) != Relation::Hostile)
        .collect();
    for party in theirs {
        // Her own book alone: `LocalSpaceBubble::set_party_relation` would mark both.
        bubble.party_relations.mark(own, party, Some(Mark::Hostile));
    }
}

/// **Land `hull`'s think at the head of her bubble's step** (doc 35 §11 `B-5`; doc 44 units
/// MW.9 and MW.11): hand what the mind asked to the one applier, seated [`Seat::ABSENT`],
/// before the bubble steps `dt_wall_ms`. The seat's rules are read off the bubble as it
/// stands now, not as the think saw it. The seat adds nothing to her flying: a burn that
/// heaves her to is the mind's own. An answer for a think her record has moved past, or
/// with no clean result, changes nothing.
pub(crate) fn land_in_bubble(
    w: &World,
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
    // Doc 45 unit AP.9 (the module head, "one gate"): what her posture lets her answer, as
    // the bubble stands now. Only the house default asks: a mind sealed aboard has her
    // sensors and no fire channel under every posture.
    let stood = standing(w, active, state.hull, ship_id, &situation);
    let fire = fire_rule(sealed.is_some());
    let mut answered = Vec::new();
    let applied = super::applier::apply_intents_answering(
        &mut active.bubble,
        &mut active.tape,
        ship_id,
        Seat {
            burn,
            fire,
            // Doc 44 unit MW.13 (Q-44-26): her own sealed mind has her sensors.
            lock: sealed.is_some(),
            ..Seat::ABSENT
        },
        Some(&stood.answers),
        &answer.contacts,
        &thought.intents,
        &mut answered,
    );
    // Doc 47 unit CO.2 (R-47-2): beside a mind sealed aboard, the house default answers fire.
    // Its think ran on the same view; only its fire channel lands, and her lock, her helm
    // and her emission posture stay the sealed mind's.
    let answered_beside = match (sealed, answer.house.as_ref()) {
        (Some(_), Some(house)) => super::applier::apply_answer(
            &mut active.bubble,
            &mut active.tape,
            ship_id,
            &stood.answers,
            &answer.contacts,
            &house.intents,
            &mut answered,
        ),
        _ => super::applier::Applied::default(),
    };
    let applied = super::applier::Applied {
        fired: applied.fired || answered_beside.fired,
        ..applied
    };
    flown.events = answered;

    // What the seat does with the world in hand, and what the crew are told.
    let hove_to = active.bubble.ships.get(ship_id).is_some_and(|ship| {
        stopper_vel.is_some_and(|stopper_vel| {
            super::picket::relative_speed_3d_mmps(ship.vel_mmps, stopper_vel)
                < HEAVE_TO_REL_VEL_MMPS
        })
    });
    if applied.fired && state.backup_act != BackupAct::ReturningFire {
        state.backup_act = BackupAct::ReturningFire;
        flown.line = Some(BACKUP_RETURNS_FIRE);
    }
    if applied.asked_hold && customs_holds && hove_to {
        flown.submit = true;
    } else if applied.asked_burn
        && stop.holds()
        && !matches!(
            state.backup_act,
            BackupAct::HeavingTo | BackupAct::ReturningFire
        )
    {
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

    use super::super::applier::Answers;
    use super::super::bubble::{LocalSpaceBubble, ShipSectionId, SpaceShipState, admitted_party};
    use super::super::picket::{BELT_RAIDER_PARTY, CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY};
    use super::super::view::{Situation, Stop};
    use super::{answers_of, complying_with, flown, stand_guard};
    use crate::arena::party::{Mark, Relation};
    use std::collections::BTreeSet;

    fn set(ships: &[&str]) -> BTreeSet<String> {
        ships.iter().map(|ship| (*ship).to_owned()).collect()
    }

    /// Her, another hull of her player, a stranger, a raider and the customs corvette, each
    /// carrier a party of its own as the tick admits them.
    fn five_ships() -> LocalSpaceBubble {
        let mut bubble = LocalSpaceBubble::new("bubble.test.ap9", "domain.5", "Test", 1_000);
        for (id, party) in [
            ("her", CARRIER_PARTY),
            ("kin", admitted_party(3)),
            ("stranger", admitted_party(4)),
        ] {
            bubble.insert_ship(SpaceShipState::new_dray(
                id,
                id,
                party,
                [0, 0, 0],
                [0, 0, 0],
            ));
        }
        bubble.insert_ship(SpaceShipState::new_corvette(
            "raider",
            "Raider",
            BELT_RAIDER_PARTY,
            [9_000, 0, 0],
            [0, 0, 0],
        ));
        bubble.insert_ship(SpaceShipState::new_corvette(
            "customs",
            "Customs",
            CUSTOMS_CORVETTE_PARTY,
            [-9_000, 0, 0],
            [0, 0, 0],
        ));
        bubble
    }

    fn arrived(bubble: &mut LocalSpaceBubble, shooter: &str, target: &str) {
        bubble
            .fired_on
            .entry(target.to_owned())
            .or_default()
            .insert(shooter.to_owned());
    }

    /// **Doc 45 unit AP.9, as doc 47 unit CO.2 rules it (R-47-3): what a hull may answer is
    /// read off the bubble's record, by her posture.** `RETURN` and `HOLD` answer a ship whose
    /// fire has arrived on her. `GUARD` also answers one whose fire has arrived on another
    /// unit of her player. None answers a ship that fired only on a stranger, a unit of her
    /// own player, or the ship she complies with. (Until CO.2, `RETURN` answered nobody.)
    #[test]
    fn co2_what_a_hull_may_answer_is_read_off_the_bubbles_record_by_her_posture() {
        let mut bubble = five_ships();
        arrived(&mut bubble, "raider", "her");
        arrived(&mut bubble, "customs", "her");
        arrived(&mut bubble, "kin", "her");
        arrived(&mut bubble, "stranger", "kin");
        arrived(&mut bubble, "customs", "kin");
        arrived(&mut bubble, "raider", "stranger");
        let kin = ["kin".to_owned()];

        for posture in [Posture::Return, Posture::Hold] {
            assert_eq!(
                answers_of(&bubble, "her", posture, &kin, None),
                Answers {
                    fired_on_by: set(&["raider", "customs"]),
                    guards: false,
                    fired_on_kin: BTreeSet::new(),
                },
                "{posture}: her own attackers, and never her player's other hull"
            );
        }
        let guard = answers_of(&bubble, "her", Posture::Guard, &kin, None);
        assert_eq!(guard.fired_on_by, set(&["raider", "customs"]));
        assert!(guard.guards);
        assert_eq!(
            guard.fired_on_kin,
            set(&["stranger", "customs"]),
            "whoever fired on her player's other hull"
        );
        assert!(guard.permits("stranger") && guard.permits("raider"));
        assert!(!guard.permits("kin"), "never her player's own unit");
        for posture in [Posture::Return, Posture::Hold] {
            let answers = answers_of(&bubble, "her", posture, &kin, None);
            assert!(
                !answers.permits("stranger"),
                "{posture}: only a guard answers for her player's other hull"
            );
        }

        // A hull with no other unit of her player in the bubble: the raider that fired on a
        // stranger beside her is nobody she may answer, under any word.
        let mut alone = five_ships();
        arrived(&mut alone, "raider", "stranger");
        for posture in [Posture::Return, Posture::Hold, Posture::Guard] {
            let answers = answers_of(&alone, "her", posture, &[], None);
            assert!(
                !answers.permits("raider"),
                "{posture}: a stranger's quarrel"
            );
            assert!(answers.all().is_empty());
        }

        // The ship she complies with is not one she answers, whatever arrived from it.
        for posture in [Posture::Return, Posture::Hold, Posture::Guard] {
            let answers = answers_of(&bubble, "her", posture, &kin, Some("customs"));
            assert!(!answers.permits("customs"), "{posture}");
            assert!(answers.permits("raider"), "{posture}");
        }
    }

    /// **Doc 45 unit AP.9: she complies with the ship that stops her** while its stop stands
    /// and once it has cleared her, and with nobody when none stands or she ran.
    #[test]
    fn ap9_she_complies_with_the_ship_whose_stop_stands_or_cleared_her() {
        let mut situation = Situation::open(1_000);
        situation.stopper = Some("customs".to_owned());
        assert_eq!(complying_with(&situation, "her"), None, "no stop on her");
        for (stop, complies) in [
            (Stop::Hailed, true),
            (Stop::HoveTo, true),
            (Stop::Cleared, true),
            (Stop::Ran, false),
        ] {
            situation.stops.insert("her".to_owned(), stop);
            assert_eq!(
                complying_with(&situation, "her"),
                complies.then_some("customs"),
                "{stop:?}"
            );
            assert_eq!(complying_with(&situation, "kin"), None, "her stop is hers");
        }
    }

    /// **Doc 45 unit AP.9: a hull that cannot defend herself** is read as a unit on the
    /// ground is. On `GUARD` with nothing to fire she is flown as `HOLD`; with her drive
    /// destroyed, or adrift as a derelict, as `RETURN`. `HOLD` and `RETURN` are never read
    /// as anything else.
    #[test]
    fn ap9_a_guard_that_cannot_defend_herself_is_flown_as_on_the_ground() {
        let armed = SpaceShipState::new_dray("her", "Her", CARRIER_PARTY, [0; 3], [0; 3]);
        assert!(!armed.lasers.is_empty(), "a Dray carries a beam");
        assert_eq!(flown(Posture::Guard, &armed), Posture::Guard);

        let mut unarmed = armed.clone();
        unarmed.lasers.clear();
        unarmed.railguns.clear();
        unarmed.torpedo_tubes.clear();
        assert_eq!(flown(Posture::Guard, &unarmed), Posture::Hold);

        let mut crippled = armed.clone();
        if let Some(drive) = crippled.sections.get_mut(&ShipSectionId::Drive) {
            drive.internals = 0;
        }
        assert_eq!(flown(Posture::Guard, &crippled), Posture::Return);

        for ship in [&armed, &unarmed, &crippled] {
            assert_eq!(flown(Posture::Hold, ship), Posture::Hold);
            assert_eq!(flown(Posture::Return, ship), Posture::Return);
        }
    }

    /// **Doc 45 unit AP.9: a hull that stands guard takes up her player's quarrel, and
    /// nobody else's.** The party of a ship that fired on her player's other hull is marked
    /// hostile in her own party's book. No other book moves, and no party that fired on
    /// nobody of hers is marked.
    #[test]
    fn ap9_standing_guard_marks_only_the_party_that_fired_on_her_players_unit() {
        let mut bubble = five_ships();
        let stranger = admitted_party(4);
        let kin = admitted_party(3);
        assert_eq!(
            bubble.party_relation(CARRIER_PARTY, stranger),
            Relation::Neutral
        );
        let answers = Answers {
            fired_on_by: BTreeSet::new(),
            guards: true,
            fired_on_kin: set(&["stranger"]),
        };
        stand_guard(&mut bubble, "her", &answers);
        assert_eq!(
            bubble.party_relations.mark_of(CARRIER_PARTY, stranger),
            Some(Mark::Hostile)
        );
        assert_eq!(
            bubble.party_relation(stranger, CARRIER_PARTY),
            Relation::Neutral
        );
        assert_eq!(bubble.party_relation(kin, stranger), Relation::Neutral);
        assert_eq!(
            bubble
                .party_relations
                .mark_of(CARRIER_PARTY, BELT_RAIDER_PARTY),
            None,
            "the raider fired on nobody of hers"
        );
        assert_eq!(
            bubble
                .party_relations
                .mark_of(CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY),
            None
        );
    }

    /// **Doc 45 unit AP.9: the bubble records whose fire arrives on whom, by ship, and keeps
    /// it through a snapshot.** A beam's shot is an arrival; a ship that has fired on nobody
    /// is on nobody's record; the record moves the bubble's hash and reads back from the
    /// document a restart loads.
    #[test]
    fn ap9_the_bubble_records_whose_fire_arrived_on_whom_and_a_snapshot_keeps_it() {
        let mut bubble = five_ships();
        let quiet = bubble.state_hash();
        assert!(bubble.fired_on.is_empty());
        assert_eq!(
            serde_json::to_value(&bubble)
                .ok()
                .and_then(|doc| doc.get("fired_on").cloned()),
            None,
            "a bubble nobody has fired in spends no byte on the record"
        );
        // A shot the bubble refuses arrives on nobody: the stranger holds her neutral.
        assert!(bubble.fire_offensive_lasers("stranger", "her").is_empty());
        assert!(bubble.fired_on.is_empty());
        bubble.set_party_relation(admitted_party(4), CARRIER_PARTY, Some(Mark::Hostile));
        let hits = bubble.fire_offensive_lasers("stranger", "her");
        assert!(!hits.is_empty(), "the beam lands");
        assert_eq!(bubble.fired_on_by("her"), Some(&set(&["stranger"])));
        assert_eq!(bubble.fired_on_by("stranger"), None);
        assert_eq!(bubble.fired_on_by("kin"), None);
        assert_ne!(bubble.state_hash(), quiet);

        let text = serde_json::to_string(&bubble.snapshot()).expect("the snapshot is written");
        let back: super::super::bubble::BubbleSnapshot =
            serde_json::from_str(&text).expect("and read");
        let restored = LocalSpaceBubble::from_snapshot(back);
        assert_eq!(restored.fired_on_by("her"), Some(&set(&["stranger"])));
        assert_eq!(restored.state_hash(), bubble.state_hash());
    }

    /// **Doc 45 unit AP.9: outside a bubble the seat holds a hull to her posture, whatever
    /// her mind asks.** A passage asked for on `HOLD` or `GUARD` is a hold; on `RETURN` it is
    /// a passage. A hold is a hold under every word, and nothing is nothing.
    #[test]
    fn ap9_a_passage_asked_for_on_hold_or_guard_is_a_hold() {
        use super::{Asks, held_to};
        assert_eq!(
            held_to(Asks::MakeForPort, Posture::Return),
            Asks::MakeForPort
        );
        for posture in [Posture::Hold, Posture::Guard] {
            assert_eq!(held_to(Asks::MakeForPort, posture), Asks::Hold);
        }
        for posture in Posture::ALL {
            assert_eq!(held_to(Asks::Hold, posture), Asks::Hold);
            assert_eq!(held_to(Asks::Nothing, posture), Asks::Nothing);
        }
    }

    /// **Doc 45 unit AP.9: only the house default answers fire.** A mind sealed aboard is
    /// given no fire rule but `Never`; the house default's seat answers under the gate,
    /// under every posture (doc 47 unit CO.2). Beside a sealed mind the house default's own
    /// think answers (`space::tick::thinks_tests::co2_*`).
    #[test]
    fn ap9_a_mind_sealed_aboard_is_never_given_the_answering_seat() {
        use super::super::applier::FireRule;
        use super::fire_rule;
        assert_eq!(fire_rule(false), FireRule::Answer);
        assert_eq!(fire_rule(true), FireRule::Never);
    }

    /// **Doc 45 unit AP.9: a slug and a torpedo arrive when their flight ends on her.** The
    /// raider's railgun round is on her record once its flight is over, struck or dodged;
    /// its torpedoes once they strike or her point defence stops them. Nothing is on the
    /// record while they are still in flight.
    #[test]
    fn ap9_a_slug_and_a_torpedo_are_on_her_record_when_their_flight_ends() {
        let rigged = || {
            let mut bubble = five_ships();
            bubble.set_party_relation(BELT_RAIDER_PARTY, CARRIER_PARTY, Some(Mark::Hostile));
            bubble
        };
        let mut bubble = rigged();
        assert!(
            !bubble.fire_railgun("raider", "her").is_empty(),
            "the raider carries a railgun in range"
        );
        assert_eq!(
            bubble.fired_on_by("her"),
            None,
            "a round in flight has not arrived"
        );
        for _ in 0..400 {
            if bubble.railgun_slugs.is_empty() {
                break;
            }
            let _ = bubble.step_wall_ms_at(250, 1_000);
        }
        assert!(bubble.railgun_slugs.is_empty());
        assert_eq!(bubble.fired_on_by("her"), Some(&set(&["raider"])));

        let mut bubble = rigged();
        assert!(
            !bubble.fire_torpedo_salvo("raider", "her", 2).is_empty(),
            "the raider carries a tube"
        );
        assert_eq!(bubble.fired_on_by("her"), None);
        for _ in 0..2_000 {
            if bubble.torpedoes.is_empty() {
                break;
            }
            let _ = bubble.step_wall_ms_at(250, 1_000);
        }
        assert!(bubble.torpedoes.is_empty());
        assert_eq!(bubble.fired_on_by("her"), Some(&set(&["raider"])));
    }

    /// **Doc 45 unit AP.9 (Q-45-16): nothing an attacker reads carries the record.** The
    /// holotank and the cockpit feeds of the ship that fired, and of a bystander, are the
    /// same documents with the record of arrivals written as without it. The posture is
    /// not on the bubble at all.
    #[test]
    fn ap9_nothing_another_ship_reads_carries_the_record_of_arrivals() {
        let mut bubble = five_ships();
        let read = |bubble: &LocalSpaceBubble| {
            ["raider", "stranger", "customs"].map(|observer| {
                (
                    bubble.holotank_feed(observer),
                    bubble.cockpit_feed(observer),
                )
            })
        };
        let before = read(&bubble);
        arrived(&mut bubble, "raider", "her");
        arrived(&mut bubble, "her", "raider");
        arrived(&mut bubble, "raider", "stranger");
        assert_eq!(read(&bubble), before);
    }
}
