//! **A collector drone's sortie, flown by its mind** (doc 44 unit MW.12; Q-44-27).
//!
//! Until this unit a drone's mind thought and its drone was a timer: 120 IC seconds after
//! the launch the hold was credited four tonnes, whatever the mind had asked. Now a drone
//! flies. Each drone has a small bubble of its own ([`DroneFlight`]) that holds three ships:
//! the ship it left (`bay`), the rock it was sent to (`rock`) and itself (`drone`). Its mind
//! reads the one view every space mind reads ([`super::view::space_mind_view`]), its
//! intents go through the one applier ([`super::applier::apply_intents`]) onto the bubble's
//! tape, and its think goes through the one host ([`super::thinks::ThinkHost`]).
//!
//! # What the mind reads
//!
//! Nothing was added to what a mind's view exposes, so the mind ABI stands.
//!
//! * **The rock** is a contact that is not friendly: its range, bearing, elevation and
//!   velocity, and `reads_dead` once it bears no ore.
//! * **The ship it left** is the contact that is friendly.
//! * **The hold** is the drone's one mount row, its collector (`v.weapons[0]`): `rounds` is
//!   the lifts the hold still has room for, `rounds_max` the lifts a whole hold takes, and
//!   `ready` whether the collector has cycled and has room.
//!
//! # What the mind asks
//!
//! `lock(id)` on the rock or on the ship; `speed(n)` to burn toward its lock, or with
//! `helm(word)` along a direction; `hold()` to cut the drive; `fire(collector, slot)` to
//! lift a tonne from the rock it has locked, when it is in reach and slow enough against it
//! ([`super::bubble::LocalSpaceBubble::work_rock`]).
//!
//! # What it brings back
//!
//! What its hold holds when it lands, and nothing when it does not land. A drone lands by
//! flying back into its bay's reach, slowly ([`super::bubble::DRONE_BAY_RANGE_M`]). One that
//! is still out after [`DRONE_ENDURANCE_STEPS`] steps is reeled in with an empty hold. The
//! ore a drone carries is held against the rock's deposit for as long as it carries it, so
//! nobody else can draw the same tonnes; the deposit itself is debited once, by the draw the
//! landing files ([`super::mining`]). A tonne lifted and not brought home never left the
//! deposit.
//!
//! # Steps and thinks
//!
//! A hull's drones step together, one second of bubble time for each space tick that finds
//! a wall second gone ([`DRONE_STEP_WALL_MS`]): a tick that is late does not make a longer
//! step, so a sortie is the same count of steps on a fast box and a slow one. The step is
//! the bubble's rule (doc 44 unit MW.11): the thinks collected on the state the last step
//! left land at its head, and it is not taken until they have answered. A hull's drones
//! are one owner of thinks ([`Owner::Cluster`]) and their record says which step collected
//! the batch that is out ([`ActiveClusterMiningSession::drone_thinks_owed`]), so a session
//! read back after a restart collects the same thinks again.
//!
//! [`ActiveClusterMiningSession::drone_thinks_owed`]: super::mining::ActiveClusterMiningSession::drone_thinks_owed

use std::collections::BTreeMap;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use super::applier::{Seat, apply_intents};
use super::bubble::{
    BubbleInputCommand, BubbleInputTape, COLLECTOR_LIFT_KG, LocalSpaceBubble, SpaceShipState,
    admitted_party,
};
use super::mining::{ActiveClusterMiningSession, DroneSortieState};
use super::thinks::{Answer, Due, Owed, Owner, ThinkHost};
use super::tick::{MAX_BUBBLE_TAPE_ENTRIES, ThinksOwed};
use crate::arena::party::PartyId;
use crate::store::TacsimStore;
use crate::surface::excursion::ExcursionId;
use crate::surface::geology::OreFamily;

/// What a collector drone's hold takes, tonnes. Provisional: it is the four tonnes a sortie
/// was credited while a drone was a timer.
pub const DRONE_HOLD_T: u64 = 4;

/// How long one step of a drone's sortie is, wall milliseconds: the space tick's period.
pub const DRONE_STEP_WALL_MS: u32 = 1_000;

/// The steps a drone may be out before its bay reels it in with an empty hold. Provisional:
/// four minutes of flight, several times what the house doctrine takes for a sortie.
pub const DRONE_ENDURANCE_STEPS: u32 = 240;

/// How far from a rock a ship must stand to launch drones onto it, metres. A drone lands by
/// leaving its bay's reach and coming back into it; a ship that stands inside her drones'
/// reach of the rock would have them lift without ever leaving, and they could not land.
/// The standoff a cluster's approach leaves her at is 600 m.
pub const DRONE_LAUNCH_STANDOFF_M: i64 =
    super::bubble::COLLECTOR_REACH_M + super::bubble::DRONE_BAY_RANGE_M + 40;

/// How far the ship at `bay_pos_m` stands from the rock at `rock_pos_m`, metres, when that
/// is nearer than [`DRONE_LAUNCH_STANDOFF_M`]: a launch from there is refused.
#[must_use]
pub fn too_close_to_launch(bay_pos_m: [i64; 3], rock_pos_m: [i64; 3]) -> Option<i64> {
    let square = |axis: usize| -> i128 {
        let part = i128::from(bay_pos_m[axis]) - i128::from(rock_pos_m[axis]);
        part * part
    };
    let range_m = i64::try_from(super::weapons::isqrt_i128(
        square(0) + square(1) + square(2),
    ))
    .unwrap_or(i64::MAX);
    (range_m < DRONE_LAUNCH_STANDOFF_M).then_some(range_m)
}

/// The ship a drone left, by her id in its bubble.
pub const BAY_SHIP: &str = "bay";

/// The rock a drone was sent to, by its id in the drone's bubble.
pub const ROCK_SHIP: &str = "rock";

/// The drone, by its id in its own bubble.
pub const DRONE_SHIP: &str = "drone";

/// The drone's collector, as its mind's `fire(id, slot)` names it.
pub const COLLECTOR_ID: &str = "collector";

/// The party of a drone and of the ship it left.
pub const BAY_PARTY: PartyId = admitted_party(0);

/// The party of the rock: nobody's, so it reads as neutral to the drone.
pub const ROCK_PARTY: PartyId = admitted_party(1);

/// **One drone's sortie in flight** (the file head): its bubble, the tape its commands are
/// recorded on, and how many steps it has flown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroneFlight {
    /// The bay, the rock and the drone.
    pub bubble: LocalSpaceBubble,
    /// The commands since the tape was last begun. It is not persisted: a flight read back
    /// begins a new tape on the bubble it kept, as a tape that has grown long does.
    #[serde(skip)]
    pub tape: Option<BubbleInputTape>,
    /// The steps flown.
    pub steps: u32,
}

/// Where a drone's flight begins: the ship and the rock, in the frame that moves with the
/// rock, and what the rock bears.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch<'a> {
    /// The ship the drone leaves.
    pub hull: ObjId,
    /// Her name.
    pub hull_name: &'a str,
    /// The drone's bay slot.
    pub slot: u8,
    /// The cluster, for the bubble's site.
    pub cluster: &'a str,
    /// The rock's block letter.
    pub letter: char,
    /// Where the ship is, metres, in the cluster's frame.
    pub bay_pos_m: [i64; 3],
    /// How the ship moves against the rock, mm/s.
    pub bay_vel_mmps: [i64; 3],
    /// Where the rock is, metres, in the cluster's frame.
    pub rock_pos_m: [i64; 3],
    /// What the rock bears that this drone may lift, kilograms.
    pub ore_kg: u64,
    /// The world's IC second.
    pub ic_now_s: i64,
}

impl DroneFlight {
    /// A flight that begins in the bay (the file head).
    #[must_use]
    pub fn open(launch: &Launch<'_>) -> Self {
        let mut bubble = LocalSpaceBubble::new(
            format!("sortie.{}.{}", launch.hull.0, launch.slot),
            launch.cluster,
            "Collector sortie",
            launch.ic_now_s,
        );
        // Nobody looks out of a drone: its bubble carries no canopy.
        bubble.bodies.clear();
        bubble.insert_ship(SpaceShipState::new_drone_bay(
            BAY_SHIP,
            launch.hull_name,
            BAY_PARTY,
            launch.bay_pos_m,
            launch.bay_vel_mmps,
        ));
        bubble.insert_ship(SpaceShipState::new_rock(
            ROCK_SHIP,
            format!("Block {}", launch.letter),
            ROCK_PARTY,
            launch.rock_pos_m,
            [0, 0, 0],
            launch.ore_kg,
        ));
        let mut drone = SpaceShipState::new_collector_drone(
            DRONE_SHIP,
            format!("Collector {}", launch.slot),
            BAY_PARTY,
            BAY_SHIP,
            COLLECTOR_ID,
            u32::try_from(DRONE_HOLD_T.saturating_mul(u64::from(COLLECTOR_LIFT_KG)))
                .unwrap_or(u32::MAX),
            launch.bay_pos_m,
        );
        drone.vel_mmps = launch.bay_vel_mmps;
        bubble.insert_ship(drone);
        let tape = Some(BubbleInputTape::new(&bubble));
        Self {
            bubble,
            tape,
            steps: 0,
        }
    }

    /// The ore in the drone's hold, kilograms.
    #[must_use]
    pub fn hold_kg(&self) -> u64 {
        self.bubble
            .ships
            .get(DRONE_SHIP)
            .and_then(|drone| drone.collector.as_ref())
            .map_or(0, |kit| u64::from(kit.hold_kg))
    }

    /// Whether the drone is back aboard.
    #[must_use]
    pub fn landed(&self) -> bool {
        self.bubble
            .ships
            .get(DRONE_SHIP)
            .and_then(|drone| drone.collector.as_ref())
            .is_some_and(|kit| kit.landed)
    }

    /// Record `command` on the tape and apply it.
    fn command(&mut self, command: BubbleInputCommand) {
        let Self { bubble, tape, .. } = self;
        let tape = tape.get_or_insert_with(|| BubbleInputTape::new(bubble));
        let _ = tape.record_and_apply(bubble, command);
    }

    /// **Land one think's intents on the drone** (the file head), through the one applier.
    pub fn land(
        &mut self,
        seat: Seat,
        contacts: &BTreeMap<crate::arena::state::ContactId, String>,
        intents: &[crate::arena::intents::IntentKind],
    ) {
        if self.landed() {
            return;
        }
        let Self { bubble, tape, .. } = self;
        let tape = tape.get_or_insert_with(|| BubbleInputTape::new(bubble));
        let mut events = Vec::new();
        let _ = apply_intents(
            bubble,
            tape,
            DRONE_SHIP,
            seat,
            contacts,
            intents,
            &mut events,
        );
    }

    /// **The host's words for where the ship is and what the rock bears** (the file head),
    /// each on the tape when it has changed. They are told before the drone's think lands,
    /// so a lift it asks for is a lift of ore that is there.
    pub fn tell(&mut self, bay_pos_m: [i64; 3], bay_vel_mmps: [i64; 3], ore_kg: u64) {
        if self
            .bubble
            .ships
            .get(BAY_SHIP)
            .is_some_and(|bay| bay.pos_m != bay_pos_m || bay.vel_mmps != bay_vel_mmps)
        {
            self.command(BubbleInputCommand::PlaceShip {
                ship_id: BAY_SHIP.to_owned(),
                pos_m: bay_pos_m,
                vel_mmps: bay_vel_mmps,
            });
        }
        if self
            .bubble
            .ships
            .get(ROCK_SHIP)
            .is_some_and(|rock| rock.ore_kg != Some(ore_kg))
        {
            self.command(BubbleInputCommand::SetOre {
                ship_id: ROCK_SHIP.to_owned(),
                ore_kg,
            });
        }
    }

    /// **One step of the flight** (the file head), on the tape.
    pub fn fly(&mut self, due: Due) {
        self.command(BubbleInputCommand::StepWall {
            dt_wall_ms: due.dt_wall_ms,
            ic_now_s: due.ic_now_s,
        });
        self.steps = self.steps.saturating_add(1);
        // A tape that has grown long begins again on the bubble as it stands.
        if self
            .tape
            .as_ref()
            .is_some_and(|tape| tape.entries.len() > MAX_BUBBLE_TAPE_ENTRIES)
        {
            self.tape = Some(BubbleInputTape::new(&self.bubble));
        }
    }
}

/// **A sortie whose record is to leave the excursion book** (doc 44 unit MW.12): the drone
/// has landed, been reeled in, or been left behind by a ship that flew on. It is kept until
/// the book no longer holds the record, so a restart between the landing and the close
/// closes it still, and closes it once.
///
/// The record is staged by the launch's own action, which may land after the sortie is
/// over (a ship that flies on in the second she launched). So an entry whose record the
/// book does not hold yet is kept for [`SORTIE_CLOSE_WAIT_TICKS`] ticks before it is taken
/// for closed. A close that is filed and does not land is filed again, each time after
/// twice as long ([`SORTIE_CLOSE_RETRY_MAX_S`] at most).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortieClose {
    /// The sortie: the drone and the IC second it was launched.
    pub id: ExcursionId,
    /// The ticks that have found no record for the drone in the book.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub absent_ticks: u32,
    /// The closes filed for it so far.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub filed: u32,
    /// The wall second before which no further close is filed.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub retry_at_wall_s: i64,
}

impl SortieClose {
    /// The close owed for the sortie `id`, not yet tried.
    #[must_use]
    pub const fn of(id: ExcursionId) -> Self {
        Self {
            id,
            absent_ticks: 0,
            filed: 0,
            retry_at_wall_s: 0,
        }
    }
}

const fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

const fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

/// The ticks a close waits for a record the book does not hold yet. Provisional: half a
/// minute, many times what a launch's action takes to land.
pub const SORTIE_CLOSE_WAIT_TICKS: u32 = 30;

/// The longest a close that did not land waits before it is filed again, wall seconds.
pub const SORTIE_CLOSE_RETRY_MAX_S: i64 = 64;

/// How a drone's sortie ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// It flew back into its bay with this much in its hold, kilograms.
    Landed {
        /// The ore aboard.
        hold_kg: u64,
    },
    /// It was still out when its endurance ran down, and was reeled in empty.
    Reeled,
}

/// What one tick's pass over a hull's drones reads of the world.
pub(crate) struct DroneTick<'a> {
    /// Where the drones' minds think.
    pub host: &'a ThinkHost,
    /// Where a seal's source is looked up.
    pub store: Option<&'a TacsimStore>,
    /// The ship the drones left.
    pub hull: ObjId,
    /// The world's wall second.
    pub wall_now_s: i64,
    /// The world's IC second.
    pub ic_now_s: i64,
    /// What the drones' minds may do: [`Seat::DRONE`], less its collector while the ship's
    /// crew is away.
    pub seat: Seat,
    /// How each rock moves, mm/s in the cluster's frame, by block letter.
    pub rocks: &'a BTreeMap<char, [i64; 3]>,
    /// What each rock's deposit bears that this hull's drones may lift or carry, kilograms,
    /// by block letter and ore family: the deposit, less every draw already filed on it and
    /// less what other hulls' drones carry.
    pub free_kg: &'a BTreeMap<(char, OreFamily), u64>,
}

/// **The seat of the drones of a ship whose crew is `crew_away` or not** (the file head;
/// doc 44 units MW.7 and MW.12): [`Seat::DRONE`], and without its collector while the crew
/// is away. The absent player's rule decides (`crate::arena::absent::permits`, asked to
/// mine): space copies nothing of it.
#[must_use]
pub fn drone_seat(crew_away: bool) -> Seat {
    let nobody = std::collections::BTreeSet::<ObjId>::new();
    let may_work = crate::arena::absent::permits(
        &crate::arena::absent::Facts {
            absent: crew_away,
            holds_fire: false,
            fired_on_by: &nobody,
            guards: false,
            fired_on_kin: &nobody,
        },
        &crate::arena::absent::Ask::Mine,
    )
    .is_ok();
    Seat {
        work: may_work,
        ..Seat::DRONE
    }
}

/// The mark of a hull's drones' batch: the step that collected it.
const fn mark(step: u32) -> super::thinks::Mark {
    (0, step)
}

/// What every drone of `session` that works `key` carries, kilograms.
pub(crate) fn carried_kg(session: &ActiveClusterMiningSession, key: (char, OreFamily)) -> u64 {
    session
        .active_drones
        .iter()
        .filter(|drone| (drone.rock_letter, drone.family) == key)
        .filter_map(|drone| drone.flight.as_ref())
        .map(DroneFlight::hold_kg)
        .sum()
}

/// **Collect the next think of each drone of `session`** (doc 44 unit MW.11), on the state
/// its bubble stands in, for the batch `owed` names. `again` is the collection a session
/// read back makes of a batch it had out: it asks only the drones that batch asked, so a
/// drone launched after the batch was collected thinks no sooner for the restart.
fn collect(
    session: &mut ActiveClusterMiningSession,
    tick: &DroneTick<'_>,
    owed: ThinksOwed,
    again: bool,
) -> Vec<super::thinks::Ask> {
    session
        .active_drones
        .iter_mut()
        .filter(|drone| !again || drone.think_out)
        .filter_map(|drone| {
            let flight = drone.flight.as_ref()?;
            let ask = super::picket::collect_drone_think(
                tick.host,
                tick.store,
                drone.excursion_id.tv,
                drone.mind_seal.as_ref(),
                &flight.bubble,
                &mut drone.contacts,
                owed.step_ms,
                drone.mind_think_seq,
                &drone.mind_memory,
            );
            drone.think_out = ask.is_some();
            ask
        })
        .collect()
}

/// **The answers the step that is due lands**, and that step as it fell due; `None` while
/// the batch is still out on a host that holds late steps (doc 44 unit MW.11). A session
/// read back with thinks owed and none in flight collects them again first.
fn thinks_for_step(
    session: &mut ActiveClusterMiningSession,
    tick: &DroneTick<'_>,
    due: Due,
) -> Option<(Vec<Answer>, Due)> {
    let Some(owed) = session.drone_thinks_owed else {
        return Some((Vec::new(), due));
    };
    let owner = Owner::Cluster(tick.hull);
    if !tick.host.in_flight(owner, mark(owed.step)) {
        let asks = collect(session, tick, owed, true);
        if !tick.host.submit(owner, mark(owed.step), asks) {
            session.drone_thinks_owed = None;
            return Some((Vec::new(), due));
        }
    }
    match tick.host.settle(owner, mark(owed.step), due) {
        Owed::Held => None,
        Owed::Nothing => {
            session.drone_thinks_owed = None;
            Some((Vec::new(), due))
        }
        Owed::Ready { answers, due } => {
            session.drone_thinks_owed = None;
            Some((answers, due))
        }
    }
}

/// **Write a drone's landed think into its record and apply what it asked** (doc 44 units
/// MW.11 and MW.12): what it remembered and traced, the next sequence number, and its
/// intents through the one applier. An answer for a think the record has moved past is
/// dropped whole.
fn land_think(drone: &mut DroneSortieState, answer: &mut Option<Answer>, seat: Seat) {
    let Some(answer) = answer.take() else {
        return;
    };
    let Some(result) = answer.result else {
        return;
    };
    if answer.think_seq != drone.mind_think_seq {
        return;
    }
    drone.mind_think_seq = drone.mind_think_seq.saturating_add(1);
    drone.mind_memory = result.memory;
    if !result.trace.is_empty() {
        drone.mind_last_trace = result.trace;
    }
    if let Some(flight) = drone.flight.as_mut() {
        flight.land(seat, &answer.contacts, &result.intents);
    }
}

/// **One tick's step of `session`'s drones** (the file head). Answers each drone whose
/// sortie ended on this step, taken out of the session, with how it ended. Nothing is
/// answered, and nothing moves, when no wall second has gone since the last step or when
/// the step's thinks are still out.
pub(crate) fn fly_drones(
    session: &mut ActiveClusterMiningSession,
    tick: &DroneTick<'_>,
) -> Vec<(DroneSortieState, Ended)> {
    let owner = Owner::Cluster(tick.hull);
    if session.active_drones.is_empty() {
        if session.drone_thinks_owed.take().is_some() {
            tick.host.discard(owner);
        }
        return Vec::new();
    }
    let held = session
        .drone_thinks_owed
        .and_then(|owed| tick.host.held_step(owner, mark(owed.step)));
    let due = if let Some(held) = held {
        held
    } else {
        if session.drones_stepped_wall_s > 0 && tick.wall_now_s <= session.drones_stepped_wall_s {
            // No wall second has gone; or the clock stepped back, and the drones are
            // re-anchored on it.
            session.drones_stepped_wall_s = tick.wall_now_s;
            return Vec::new();
        }
        Due {
            dt_wall_ms: DRONE_STEP_WALL_MS,
            ic_now_s: tick.ic_now_s,
        }
    };
    let Some((answers, due)) = thinks_for_step(session, tick, due) else {
        return Vec::new();
    };
    session.drones_stepped_wall_s = tick.wall_now_s;
    for drone in &mut session.active_drones {
        drone.think_out = false;
    }

    // The answers land, drone by drone, each after the host has told its bubble where the
    // ship is and what the rock bears now: the deposit, less what every drone carries. So a
    // tonne one drone lifts is not there for the next, and two never lift the same one.
    let mut answers: BTreeMap<ObjId, Option<Answer>> = answers
        .into_iter()
        .map(|answer| (answer.unit, Some(answer)))
        .collect();
    for index in 0..session.active_drones.len() {
        let Some(key) = session
            .active_drones
            .get(index)
            .map(|drone| (drone.rock_letter, drone.family))
        else {
            continue;
        };
        let carried = carried_kg(session, key);
        let free = tick.free_kg.get(&key).copied().unwrap_or(0);
        let rock_vel_mmps = tick.rocks.get(&key.0).copied().unwrap_or_default();
        let bay_vel_mmps = [
            session.ship_vel_mmps[0].saturating_sub(rock_vel_mmps[0]),
            session.ship_vel_mmps[1].saturating_sub(rock_vel_mmps[1]),
            session.ship_vel_mmps[2].saturating_sub(rock_vel_mmps[2]),
        ];
        let bay_pos_m = session.ship_pos_m;
        let Some(drone) = session.active_drones.get_mut(index) else {
            continue;
        };
        if let Some(flight) = drone.flight.as_mut() {
            flight.tell(bay_pos_m, bay_vel_mmps, free.saturating_sub(carried));
        }
        if let Some(answer) = answers.get_mut(&drone.excursion_id.tv) {
            land_think(drone, answer, tick.seat);
        }
    }

    // The drones fly.
    for drone in &mut session.active_drones {
        if let Some(flight) = drone.flight.as_mut() {
            flight.fly(due);
        }
    }
    session.drone_steps = session.drone_steps.saturating_add(1);

    // The sorties that ended on this step leave the session.
    let mut ended = Vec::new();
    let mut flying = Vec::with_capacity(session.active_drones.len());
    for drone in std::mem::take(&mut session.active_drones) {
        let end = match drone.flight.as_ref() {
            // A record with no flight was written before drones flew: it is reeled in.
            None => Some(Ended::Reeled),
            Some(flight) if flight.landed() => Some(Ended::Landed {
                hold_kg: flight.hold_kg(),
            }),
            Some(flight) if flight.steps >= DRONE_ENDURANCE_STEPS => Some(Ended::Reeled),
            Some(_) => None,
        };
        match end {
            Some(end) => ended.push((drone, end)),
            None => flying.push(drone),
        }
    }
    session.active_drones = flying;

    // The next thinks are collected on the state the step left, and handed to the pool.
    let owed = ThinksOwed {
        step: session.drone_steps,
        step_ms: due.dt_wall_ms,
    };
    let asks = collect(session, tick, owed, false);
    session.drone_thinks_owed = tick
        .host
        .submit(owner, mark(owed.step), asks)
        .then_some(owed);
    ended
}

#[cfg(test)]
pub(crate) mod rig {
    //! A drone's sortie with no world: what the unit tests of the space modules fly.

    use mush_core::{CanonicalId, ObjId};
    use sha2::{Digest as _, Sha256};

    use super::{
        ActiveClusterMiningSession, DRONE_SHIP, DroneFlight, DroneSortieState, DroneTick, Ended,
        ExcursionId, Launch, OreFamily, Seat, ThinkHost, fly_drones,
    };
    use crate::minds::content::cast::CastActor;
    use crate::space::view::{ContactBook, Situation, space_mind_view};

    /// The ship the rig's drones leave.
    pub(crate) const HULL: ObjId = ObjId(900);

    /// Where she stands: the standoff an approach leaves her at, 600 m short of the rock.
    pub(crate) const BAY_AT: [i64; 3] = [400, 0, 0];

    /// Where the rock is.
    pub(crate) const ROCK_AT: [i64; 3] = [1_000, 0, 0];

    /// A flight as `+mine/drones` opens it, onto a rock that bears `ore_t` tonnes.
    pub(crate) fn flight(slot: u8, ore_t: u64) -> DroneFlight {
        DroneFlight::open(&Launch {
            hull: HULL,
            hull_name: "Salt Moth",
            slot,
            cluster: "ceres_halo",
            letter: 'A',
            bay_pos_m: BAY_AT,
            bay_vel_mmps: [0, 0, 0],
            rock_pos_m: ROCK_AT,
            ore_kg: ore_t.saturating_mul(1_000),
            ic_now_s: 1_000,
        })
    }

    /// The view a drone reads in its bay, before its first step.
    pub(crate) fn view(unit: ObjId) -> crate::arena::mind::MindView {
        let flight = flight(1, 8);
        space_mind_view(
            &flight.bubble,
            DRONE_SHIP,
            unit,
            &mut ContactBook::default(),
            &Situation::open(1_000),
        )
        .expect("a drone in its bay has a view")
        .0
    }

    /// The drone of bay slot `slot`, out on a rock of `ore_t` tonnes, sealed with the pack's
    /// doctrine or with the source `digest` names.
    pub(crate) fn drone(slot: u8, ore_t: u64, digest: Option<[u8; 32]>) -> DroneSortieState {
        let unit = ObjId(1_000 + u32::from(slot));
        let mind_seal = match digest {
            None => crate::space::picket::pack_cast_seal(CastActor::Drone, unit),
            Some(digest) => crate::space::picket::build_space_mind_seal(
                unit,
                crate::arena::mind::SortieId::ic(unit, 1_000),
                "drone_mining",
                digest,
            ),
        };
        assert!(mind_seal.is_some(), "the rig seals its drone");
        DroneSortieState {
            slot,
            excursion_id: ExcursionId {
                tv: unit,
                launch_ic: 1_000,
            },
            rock_letter: 'A',
            family: OreFamily::Metals,
            operator: ObjId(7),
            flight: Some(flight(slot, ore_t)),
            contacts: ContactBook::default(),
            think_out: false,
            mind_seal,
            mind_memory: Vec::new(),
            mind_think_seq: 0,
            mind_last_trace: Vec::new(),
        }
    }

    /// A cluster session whose ship stands matched off the rock with `drones` out.
    pub(crate) fn session(drones: Vec<DroneSortieState>) -> ActiveClusterMiningSession {
        let cluster = CanonicalId::new("ceres_halo").expect("a cluster id");
        let mut session = ActiveClusterMiningSession::new(HULL, cluster, 1_000);
        session.ship_pos_m = BAY_AT;
        session.ship_vel_mmps = [0, 0, 0];
        session.matched_rock = Some('A');
        session.active_drones = drones;
        session
    }

    /// Put `source` on `host` as a doctrine a seal can name, and answer its digest: what a
    /// revision staff published is to the store of a world.
    pub(crate) fn publish(host: &ThinkHost, source: &str) -> [u8; 32] {
        let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        let text = source.to_owned();
        let runner = crate::minds::sealed::scripted_runner(
            ObjId(1),
            &digest,
            &crate::config::ArenaConfig::default(),
            &|wanted| (*wanted == digest).then(|| text.clone()),
        )
        .expect("the doctrine compiles");
        host.install_runner(digest, runner);
        digest
    }

    /// One tick of the rig: the drones' step at the wall second `wall_now_s`. `free_t` is
    /// what the deposit bears that these drones may lift or carry.
    pub(crate) fn tick(
        session: &mut ActiveClusterMiningSession,
        host: &ThinkHost,
        wall_now_s: i64,
        free_t: u64,
        seat: Seat,
    ) -> Vec<(DroneSortieState, Ended)> {
        let rocks = std::collections::BTreeMap::from([('A', [0, 0, 0])]);
        let free_kg = std::collections::BTreeMap::from([(
            ('A', OreFamily::Metals),
            free_t.saturating_mul(1_000),
        )]);
        fly_drones(
            session,
            &DroneTick {
                host,
                store: None,
                hull: HULL,
                wall_now_s,
                ic_now_s: 1_000 + wall_now_s,
                seat,
                rocks: &rocks,
                free_kg: &free_kg,
            },
        )
    }

    /// Fly the drones of `session`, a wall second a tick from `from_wall_s`, until every
    /// sortie is over: each ending with the tick it ended on. What a drone lands with is
    /// drawn from the deposit, as the mining tick draws it.
    pub(crate) fn fly_home(
        session: &mut ActiveClusterMiningSession,
        host: &ThinkHost,
        from_wall_s: i64,
        mut free_t: u64,
        seat: Seat,
    ) -> Vec<(DroneSortieState, Ended, i64)> {
        let mut over = Vec::new();
        let mut wall_now_s = from_wall_s;
        while !session.active_drones.is_empty() {
            wall_now_s += 1;
            assert!(
                wall_now_s - from_wall_s < 2_000,
                "the drones never came home: {:?}",
                session
                    .active_drones
                    .iter()
                    .map(|drone| drone.mind_last_trace.clone())
                    .collect::<Vec<_>>()
            );
            for (drone, end) in tick(session, host, wall_now_s, free_t, seat) {
                if let Ended::Landed { hold_kg } = end {
                    free_t = free_t.saturating_sub(hold_kg / 1_000);
                }
                over.push((drone, end, wall_now_s));
            }
        }
        over
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::rig::{self, HULL};
    use super::*;
    use crate::minds::MindPool;
    use crate::minds::content::drone_mining_source;
    use crate::space::bubble::{
        COLLECTOR_CYCLE_MS, COLLECTOR_REACH_M, DRONE_BAY_RANGE_M, DRONE_PROPELLANT_KG,
    };
    use crate::space::thinks::test_support::{Gate, Gated};

    /// The house doctrine with the line that says when it has enough put to `lifts`.
    fn stops_at(lifts: u32) -> String {
        let house = drone_mining_source();
        let line = "    let enough = tool.rounds_max;\n";
        assert!(
            house.contains(line),
            "the house doctrine names what is enough"
        );
        house.replace(line, &format!("    let enough = {lifts};\n"))
    }

    fn rock_ore_kg(drone: &DroneSortieState) -> Option<u64> {
        drone
            .flight
            .as_ref()?
            .bubble
            .ships
            .get(ROCK_SHIP)
            .and_then(|rock| rock.ore_kg)
    }

    /// **Doc 44 unit MW.12 (Q-44-27): the house drone flies out, fills its hold and lands
    /// with it.** Nothing but its mind moves it: it leaves the bay, comes inside the
    /// reach of its collector, lifts four times, and comes home.
    #[test]
    fn mw12_the_house_drone_fills_its_hold_and_lands_with_it() {
        let host = ThinkHost::inline();
        let mut session = rig::session(vec![rig::drone(1, 120, None)]);
        let mut furthest_m = 0;
        let mut nearest_rock_m = i64::MAX;
        let mut wall = 0;
        let mut over = Vec::new();
        while over.is_empty() {
            wall += 1;
            assert!(wall < 400, "the drone never came home");
            over = rig::tick(&mut session, &host, wall, 120, Seat::DRONE);
            if let Some(drone) = session.active_drones.first() {
                let flight = drone.flight.as_ref().expect("a flight");
                let at = flight.bubble.ships[DRONE_SHIP].pos_m;
                furthest_m = furthest_m.max(at[0] - rig::BAY_AT[0]);
                nearest_rock_m = nearest_rock_m.min((rig::ROCK_AT[0] - at[0]).abs());
            }
        }
        let (drone, end) = over.pop().expect("one sortie");
        assert_eq!(end, Ended::Landed { hold_kg: 4_000 });
        assert!(
            furthest_m > DRONE_BAY_RANGE_M,
            "it left the bay: {furthest_m} m"
        );
        assert!(
            nearest_rock_m <= COLLECTOR_REACH_M,
            "it came inside the reach of its collector: {nearest_rock_m} m"
        );
        assert_eq!(
            rock_ore_kg(&drone),
            Some(116_000),
            "the rock bears what it left"
        );
        assert!(drone.mind_think_seq > 20, "its mind thought every step");
        assert_eq!(
            drone.mind_last_trace,
            vec!["DRONE_MINING_RETURN".to_owned()]
        );
        // The length of a sortie is a tuning figure; this holds it to the order it was built at.
        assert!((40..=110).contains(&wall), "the sortie took {wall} steps");
        assert!(session.active_drones.is_empty());
    }

    /// **The first done-when of doc 44 unit MW.12, at the unit: a drone whose mind is
    /// changed to stop short brings back less.** The same launch, the same rock, the same
    /// steps; the only thing that differs is the doctrine the seal names.
    #[test]
    fn mw12_a_doctrine_changed_to_stop_short_brings_back_less() {
        let host = ThinkHost::inline();
        let mut hauls = Vec::new();
        for lifts in [4_u32, 3, 2, 1] {
            let digest = rig::publish(&host, &stops_at(lifts));
            let mut session = rig::session(vec![rig::drone(1, 120, Some(digest))]);
            let over = rig::fly_home(&mut session, &host, 0, 120, Seat::DRONE);
            let (_, end, steps) = over.first().expect("one sortie");
            assert_eq!(
                *end,
                Ended::Landed {
                    hold_kg: u64::from(lifts) * 1_000
                },
                "a doctrine that goes home with {lifts} lifts"
            );
            hauls.push((lifts, *steps));
        }
        // And it is home sooner: a lift it does not wait for is time it does not spend.
        let steps: Vec<i64> = hauls.iter().map(|(_, steps)| *steps).collect();
        assert!(
            steps.windows(2).all(|pair| pair[0] > pair[1]),
            "fewer lifts, a shorter sortie: {hauls:?}"
        );
    }

    /// **A drone that does not come home brings nothing** (doc 44 unit MW.12). One whose
    /// mind never asks for anything sits in its bay until its endurance runs down. One that
    /// fills its hold and stays at the rock is reeled in too, and what it lifted is not
    /// landed: the yield is what the drone did, and it did not bring it back.
    #[test]
    fn mw12_a_drone_that_does_not_come_home_is_reeled_in_with_nothing() {
        let host = ThinkHost::inline();
        let idle = rig::publish(&host, "fn think(v, c) {\n    hold();\n}\n");
        let mut session = rig::session(vec![rig::drone(1, 120, Some(idle))]);
        let over = rig::fly_home(&mut session, &host, 0, 120, Seat::DRONE);
        let (drone, end, steps) = over.first().expect("one sortie");
        assert_eq!(*end, Ended::Reeled);
        assert_eq!(*steps, i64::from(DRONE_ENDURANCE_STEPS));
        assert_eq!(drone.flight.as_ref().map(DroneFlight::hold_kg), Some(0));

        let house = drone_mining_source();
        let goes = "    let goal = if this.home { ship } else { rock };\n";
        assert!(house.contains(goes));
        let stays = rig::publish(&host, &house.replace(goes, "    let goal = rock;\n"));
        let mut session = rig::session(vec![rig::drone(1, 120, Some(stays))]);
        let over = rig::fly_home(&mut session, &host, 0, 120, Seat::DRONE);
        let (drone, end, steps) = over.first().expect("one sortie");
        assert_eq!(*end, Ended::Reeled, "it never flew back");
        assert_eq!(*steps, i64::from(DRONE_ENDURANCE_STEPS));
        assert_eq!(
            drone.flight.as_ref().map(DroneFlight::hold_kg),
            Some(4_000),
            "it had lifted a whole hold, and landed none of it"
        );
    }

    /// **What leaves the rock is what reaches the holds** (doc 44 unit MW.12). Two drones
    /// work a rock that bears five tonnes. On every step what they carry and what has
    /// landed is never more than the five, and they land five between them, never six.
    #[test]
    fn mw12_two_drones_on_one_rock_never_lift_the_same_tonne() {
        let host = ThinkHost::inline();
        let mut session = rig::session(vec![rig::drone(1, 5, None), rig::drone(2, 5, None)]);
        let mut free_t = 5_u64;
        let mut landed_kg = 0_u64;
        let mut wall = 0;
        while !session.active_drones.is_empty() {
            wall += 1;
            assert!(wall < 600, "the drones never came home");
            for (_, end) in rig::tick(&mut session, &host, wall, free_t, Seat::DRONE) {
                let Ended::Landed { hold_kg } = end else {
                    panic!("a drone was reeled in");
                };
                landed_kg += hold_kg;
                free_t -= hold_kg / 1_000;
            }
            let carried = carried_kg(&session, ('A', OreFamily::Metals));
            assert!(
                carried + landed_kg <= 5_000,
                "step {wall}: {carried} kg carried and {landed_kg} kg landed of 5,000"
            );
            for drone in &session.active_drones {
                // No drone is shown ore that another carries or has landed: the rock it is
                // told of, and what is carried and landed, never come to more than the five.
                // (A drone told before a sister lifted in the same step is shown the tonne
                // that sister then took, until its next step.)
                let shown = rock_ore_kg(drone).expect("a rock");
                assert!(
                    shown + landed_kg <= 5_000,
                    "step {wall}: the rock shows {shown} kg with {landed_kg} kg landed"
                );
            }
        }
        assert_eq!(
            landed_kg, 5_000,
            "five tonnes left the rock and five landed"
        );
    }

    /// **A rock that is worked out sends the drone home with what it has** (doc 44 unit
    /// MW.12): the mind reads the rock, and the rock reads as done.
    #[test]
    fn mw12_a_rock_that_runs_out_sends_the_drone_home_with_what_it_has() {
        let host = ThinkHost::inline();
        let mut session = rig::session(vec![rig::drone(1, 2, None)]);
        let over = rig::fly_home(&mut session, &host, 0, 2, Seat::DRONE);
        let (drone, end, _) = over.first().expect("one sortie");
        assert_eq!(*end, Ended::Landed { hold_kg: 2_000 });
        assert_eq!(rock_ore_kg(drone), Some(0));
    }

    /// **A drone whose crew is away keeps its helm and loses its collector** (doc 44 units
    /// MW.7 and MW.12; `crate::arena::absent`): no mind mines for a player who is not
    /// there. It flies out, asks, is refused, and comes home empty.
    #[test]
    fn mw12_a_drone_whose_crew_is_away_lifts_nothing_and_comes_home() {
        assert_eq!(drone_seat(false), Seat::DRONE);
        let away = drone_seat(true);
        assert_eq!(
            away,
            Seat {
                work: false,
                ..Seat::DRONE
            },
            "the absent rule takes the collector and nothing else"
        );
        let host = ThinkHost::inline();
        let mut session = rig::session(vec![rig::drone(1, 120, None)]);
        let over = rig::fly_home(&mut session, &host, 0, 120, away);
        let (drone, end, _) = over.first().expect("one sortie");
        assert_eq!(*end, Ended::Landed { hold_kg: 0 });
        assert_eq!(rock_ore_kg(drone), Some(120_000));
    }

    /// **A replay of the tape of a sortie reproduces it, and runs no think** (doc 44 units
    /// MW.10, MW.11 and MW.12). Two drones work one rock, and the ship moves thirty metres
    /// while they are out. After every step each tape, replayed from where it was begun, is
    /// its bubble as it stands: where the drone is and how it moves, its hold, the ore the
    /// rock bears, where the ship is, the landing. The lifts, and the two words the host
    /// tells a bubble, are all on a tape that was replayed. The count of thinks the host
    /// keeps does not move for a replay.
    #[test]
    fn mw12_a_replay_of_the_tape_reproduces_the_sortie_and_runs_no_think() {
        let host = ThinkHost::inline();
        let mut session = rig::session(vec![rig::drone(1, 120, None), rig::drone(2, 120, None)]);
        let mut wall = 0;
        let mut replays = 0;
        let (mut lifts, mut ores, mut places) = (0, 0, 0);
        let mut landed_kg = 0;
        while !session.active_drones.is_empty() {
            wall += 1;
            assert!(wall < 400, "the drones never came home");
            if wall == 10 {
                session.ship_pos_m[1] += 30;
            }
            let over = rig::tick(&mut session, &host, wall, 120, Seat::DRONE);
            let thinks = host.stats().thinks;
            for drone in session
                .active_drones
                .iter()
                .chain(over.iter().map(|(drone, _)| drone))
            {
                let flight = drone.flight.as_ref().expect("a flight");
                let tape = flight.tape.as_ref().expect("a tape");
                let (replayed, _) = tape.replay();
                assert_eq!(replayed, flight.bubble, "step {wall}, slot {}", drone.slot);
                assert_eq!(replayed.state_hash(), flight.bubble.state_hash());
                replays += 1;
                for entry in &tape.entries {
                    match entry.command {
                        BubbleInputCommand::WorkRock { .. } => lifts += 1,
                        BubbleInputCommand::SetOre { .. } => ores += 1,
                        BubbleInputCommand::PlaceShip { .. } => places += 1,
                        _ => {}
                    }
                }
            }
            assert_eq!(host.stats().thinks, thinks, "a replay runs no think");
            for (drone, end) in &over {
                assert_eq!(*end, Ended::Landed { hold_kg: 4_000 });
                let replayed = drone
                    .flight
                    .as_ref()
                    .and_then(|flight| flight.tape.as_ref())
                    .map(|tape| tape.replay().0)
                    .expect("a tape");
                assert!(
                    replayed.ships[DRONE_SHIP]
                        .collector
                        .as_ref()
                        .is_some_and(|kit| kit.landed && kit.hold_kg == 4_000)
                );
                assert_eq!(replayed.ships[BAY_SHIP].pos_m[1], rig::BAY_AT[1] + 30);
                landed_kg += 4_000;
            }
        }
        assert_eq!(landed_kg, 8_000);
        assert!(replays > 80);
        assert!(lifts > 0, "a replayed tape held a lift");
        assert!(ores > 0, "a replayed tape held the word for the ore");
        assert!(places > 0, "a replayed tape held the word for the ship");
    }

    /// **A session read back with a sortie open ends as the one that was never stopped**
    /// (doc 44 units MW.11 and MW.12). The record goes through its stored form after any
    /// step and meets a host that has never seen it: the thinks it owed are collected
    /// again, and the drone lands on the same step with the same hold and the same memory.
    #[test]
    fn mw12_a_session_read_back_mid_sortie_ends_as_the_one_that_was_not() {
        let straight = {
            let host = ThinkHost::inline();
            let mut session = rig::session(vec![rig::drone(1, 120, None)]);
            rig::fly_home(&mut session, &host, 0, 120, Seat::DRONE)
        };
        let (drone, end, steps) = straight.first().expect("one sortie");
        assert_eq!(*end, Ended::Landed { hold_kg: 4_000 });
        for stop_at in [1_i64, 2, 9, 30, steps - 2] {
            let host = ThinkHost::inline();
            let mut session = rig::session(vec![rig::drone(1, 120, None)]);
            for wall in 1..=stop_at {
                let over = rig::tick(&mut session, &host, wall, 120, Seat::DRONE);
                assert!(over.is_empty());
            }
            assert!(session.drone_thinks_owed.is_some(), "a think is out");
            let stored = serde_json::to_string(&session).expect("the session is a document");
            let mut read_back: ActiveClusterMiningSession =
                serde_json::from_str(&stored).expect("and reads back");
            assert!(
                read_back.active_drones[0]
                    .flight
                    .as_ref()
                    .is_some_and(|flight| flight.tape.is_none())
            );
            let fresh = ThinkHost::inline();
            let over = rig::fly_home(&mut read_back, &fresh, stop_at, 120, Seat::DRONE);
            let (again, end_again, steps_again) = over.first().expect("one sortie");
            assert_eq!(
                (end_again, steps_again),
                (end, steps),
                "stopped at {stop_at}"
            );
            assert_eq!(again.mind_memory, drone.mind_memory);
            assert_eq!(again.mind_think_seq, drone.mind_think_seq);
            assert_eq!(
                again.flight.as_ref().map(|f| f.bubble.state_hash()),
                drone.flight.as_ref().map(|f| f.bubble.state_hash())
            );
        }
    }

    /// **The drones of a hull take one step for a wall second, of one second** (doc 44 unit
    /// MW.12): a tick in the same second takes none, and a tick that comes six seconds late
    /// takes one, not a step of six. A sortie is a count of steps, whatever the box.
    #[test]
    fn mw12_drones_take_one_step_a_wall_second_and_a_late_tick_is_not_a_long_step() {
        let host = ThinkHost::inline();
        let mut session = rig::session(vec![rig::drone(1, 120, None)]);
        let elapsed = |session: &ActiveClusterMiningSession| {
            let flight = session.active_drones[0].flight.as_ref().expect("a flight");
            (flight.steps, flight.bubble.wall_elapsed_ms)
        };
        let _ = rig::tick(&mut session, &host, 10, 120, Seat::DRONE);
        assert_eq!(elapsed(&session), (1, 1_000));
        let _ = rig::tick(&mut session, &host, 10, 120, Seat::DRONE);
        assert_eq!(elapsed(&session), (1, 1_000), "no wall second has gone");
        let _ = rig::tick(&mut session, &host, 16, 120, Seat::DRONE);
        assert_eq!(
            elapsed(&session),
            (2, 2_000),
            "six seconds late is one step"
        );
        let _ = rig::tick(&mut session, &host, 12, 120, Seat::DRONE);
        assert_eq!(
            elapsed(&session),
            (2, 2_000),
            "a clock that steps back re-anchors"
        );
        let _ = rig::tick(&mut session, &host, 13, 120, Seat::DRONE);
        assert_eq!(elapsed(&session), (3, 3_000));
    }

    /// **A step whose thinks are out holds the drones, and the thread goes on** (doc 44
    /// units MW.11 and MW.12; the held-step rule MW.11 left owed). On a host that holds
    /// late steps, with the worker parked, tick after tick takes no step and writes
    /// nothing; when the think answers the step is taken, and the sortie comes out as the
    /// one no step of which was ever held.
    #[test]
    fn mw12_a_step_whose_thinks_are_out_holds_the_drones() {
        let straight = {
            let host = ThinkHost::inline();
            let mut session = rig::session(vec![rig::drone(1, 120, None)]);
            rig::fly_home(&mut session, &host, 0, 120, Seat::DRONE)
        };
        let (drone, end, steps) = straight.first().expect("one sortie");

        let host = ThinkHost::on_pool(MindPool::new(1));
        host.hold_late_steps();
        let digest = crate::minds::content::drone_mining_digest().0;
        let view = Arc::new(rig::view(ObjId(1)));
        let _ = host.job(None, ObjId(1), digest, 0, &view, &[]);
        let inner = host
            .kept_runner(&digest)
            .expect("the doctrine has a runner");
        let gate = Arc::new(Gate::default());
        host.install_runner(
            digest,
            Arc::new(Gated {
                inner,
                gate: Arc::clone(&gate),
            }),
        );
        let wait_until = |what: &dyn Fn() -> bool| {
            let until = Instant::now() + Duration::from_secs(30);
            while !what() {
                assert!(Instant::now() < until, "the worker never got there");
                std::thread::sleep(Duration::from_millis(1));
            }
        };

        let mut session = rig::session(vec![rig::drone(1, 120, None)]);
        let mut wall = 0;
        let mut taken = 0_i64;
        let mut held_ticks = 0_u64;
        let mut thinks_parked = 0_u64;
        let mut over = Vec::new();
        while over.is_empty() {
            assert!(wall < 4_000, "the drone never came home");
            // The thinks of the step are out and the worker is parked: three ticks find the
            // step held.
            if let Some(owed) = session.drone_thinks_owed {
                thinks_parked += 1;
                wait_until(&|| gate.arrived() >= thinks_parked);
                let before = session.clone();
                for _ in 0..3 {
                    wall += 1;
                    let none = rig::tick(&mut session, &host, wall, 120, Seat::DRONE);
                    assert!(none.is_empty());
                    held_ticks += 1;
                }
                assert_eq!(
                    session, before,
                    "a held step moves nothing and writes nothing"
                );
                gate.open();
                wait_until(&|| host.is_answered(Owner::Cluster(HULL), mark(owed.step)));
                gate.close();
            }
            wall += 1;
            over = rig::tick(&mut session, &host, wall, 120, Seat::DRONE);
            taken += 1;
        }
        gate.open();
        let (held_drone, held_end) = over.first().expect("one sortie");
        assert_eq!(held_end, end);
        assert_eq!(taken, *steps, "the same count of steps");
        assert_eq!(held_drone.mind_memory, drone.mind_memory);
        assert_eq!(held_drone.mind_think_seq, drone.mind_think_seq);
        // Everything but the IC second its steps were stamped with, which is the world's
        // clock and ran on under the held steps (doc 44 unit MW.11).
        let flown = |drone: &DroneSortieState| {
            drone
                .flight
                .as_ref()
                .map(|flight| (flight.bubble.ships.clone(), flight.bubble.wall_elapsed_ms))
        };
        assert_eq!(flown(held_drone), flown(drone));
        assert!(host.stats().held >= held_ticks, "{:?}", host.stats());
    }

    /// **The rules of the bubble for a collector** (doc 44 unit MW.12): a lift needs the
    /// drone in reach and slow, the collector cycled and room in the hold; a drone lands
    /// only once it has been away, and only slowly.
    #[test]
    fn mw12_a_lift_needs_reach_and_a_cycled_collector_and_a_landing_needs_a_slow_return() {
        let mut flight = rig::flight(1, 3);
        let bubble = &mut flight.bubble;
        // In the bay, 600 m from the rock: out of reach.
        assert_eq!(bubble.work_rock(DRONE_SHIP, ROCK_SHIP), 0);
        let rock_at = bubble.ships[ROCK_SHIP].pos_m;
        let place = |bubble: &mut LocalSpaceBubble, off_m: i64, speed_mmps: i64| {
            let drone = bubble.ships.get_mut(DRONE_SHIP).expect("the drone");
            drone.pos_m = [rock_at[0] - off_m, rock_at[1], rock_at[2]];
            drone.vel_mmps = [speed_mmps, 0, 0];
        };
        place(bubble, COLLECTOR_REACH_M + 1, 0);
        assert_eq!(
            bubble.work_rock(DRONE_SHIP, ROCK_SHIP),
            0,
            "a metre out of reach"
        );
        place(bubble, COLLECTOR_REACH_M, 5_001);
        assert_eq!(
            bubble.work_rock(DRONE_SHIP, ROCK_SHIP),
            0,
            "too fast against it"
        );
        place(bubble, COLLECTOR_REACH_M, 5_000);
        assert_eq!(
            bubble.work_rock(DRONE_SHIP, BAY_SHIP),
            0,
            "the ship is not a rock"
        );
        assert_eq!(bubble.work_rock(DRONE_SHIP, ROCK_SHIP), 1_000);
        assert_eq!(
            bubble.work_rock(DRONE_SHIP, ROCK_SHIP),
            0,
            "the collector is cycling"
        );
        place(bubble, COLLECTOR_REACH_M, 0);
        let _ = bubble.step_wall_ms_at(COLLECTOR_CYCLE_MS - 250, 1_001);
        assert_eq!(
            bubble.work_rock(DRONE_SHIP, ROCK_SHIP),
            0,
            "a quarter second short"
        );
        let _ = bubble.step_wall_ms_at(250, 1_002);
        assert_eq!(bubble.work_rock(DRONE_SHIP, ROCK_SHIP), 1_000);
        let _ = bubble.step_wall_ms_at(COLLECTOR_CYCLE_MS, 1_005);
        assert_eq!(bubble.work_rock(DRONE_SHIP, ROCK_SHIP), 1_000);
        let _ = bubble.step_wall_ms_at(COLLECTOR_CYCLE_MS, 1_008);
        assert_eq!(
            bubble.work_rock(DRONE_SHIP, ROCK_SHIP),
            0,
            "the rock is worked out"
        );
        assert_eq!(bubble.ships[ROCK_SHIP].ore_kg, Some(0));
        let kit = bubble.ships[DRONE_SHIP].collector.clone().expect("a kit");
        assert_eq!((kit.hold_kg, kit.away, kit.landed), (3_000, true, false));

        // Back at the ship, fast: it flies through. Slow: it lands, and stays.
        let bay_at = bubble.ships[BAY_SHIP].pos_m;
        let home = |bubble: &mut LocalSpaceBubble, speed_mmps: i64| {
            let drone = bubble.ships.get_mut(DRONE_SHIP).expect("the drone");
            drone.pos_m = [bay_at[0] + 20, bay_at[1], bay_at[2]];
            drone.vel_mmps = [-speed_mmps, 0, 0];
        };
        home(bubble, 8_001);
        let _ = bubble.step_wall_ms_at(250, 1_009);
        assert!(
            !bubble.ships[DRONE_SHIP]
                .collector
                .as_ref()
                .is_some_and(|kit| kit.landed)
        );
        home(bubble, 8_000);
        let _ = bubble.step_wall_ms_at(250, 1_010);
        let drone = &bubble.ships[DRONE_SHIP];
        assert!(
            drone
                .collector
                .as_ref()
                .is_some_and(|kit| kit.landed && kit.hold_kg == 3_000)
        );
        assert_eq!((drone.pos_m, drone.vel_mmps), (bay_at, [0, 0, 0]));
        assert_eq!(
            bubble.work_rock(DRONE_SHIP, ROCK_SHIP),
            0,
            "a landed drone lifts nothing"
        );
        assert_eq!(flight.hold_kg(), 3_000);
        assert!(flight.landed());

        // A drone that has not left its bay does not land in it.
        let mut fresh = rig::flight(2, 3);
        let _ = fresh.bubble.step_wall_ms_at(1_000, 1_001);
        assert!(!fresh.landed());
        assert_eq!(
            fresh.bubble.ships[DRONE_SHIP].propellant_kg,
            DRONE_PROPELLANT_KG
        );
    }

    /// **A ship launches drones only from outside their reach of the rock** (doc 44 unit
    /// MW.12; the second reader's finding). A drone lands by leaving its bay's reach and
    /// coming back. From a ship that stands inside the collector's reach of the rock it
    /// would lift without leaving, and wait in a bay it could never land in until it was
    /// reeled in with nothing. So the launch is refused there; from the standoff, and from
    /// the 600 m an approach leaves her at, it is not, and the house drone lands its hold.
    #[test]
    fn mw12_a_ship_inside_her_drones_reach_of_the_rock_does_not_launch_them() {
        let rock = rig::ROCK_AT;
        let off = |metres: i64| [rock[0] - metres, rock[1], rock[2]];
        assert_eq!(too_close_to_launch(off(100), rock), Some(100));
        assert_eq!(
            too_close_to_launch(off(DRONE_LAUNCH_STANDOFF_M - 1), rock),
            Some(DRONE_LAUNCH_STANDOFF_M - 1)
        );
        assert_eq!(
            too_close_to_launch(off(DRONE_LAUNCH_STANDOFF_M), rock),
            None
        );
        assert_eq!(too_close_to_launch(rig::BAY_AT, rock), None);
        assert_eq!(
            too_close_to_launch([rock[0], rock[1] + 30, rock[2] - 40], rock),
            Some(50)
        );

        // From the nearest stand a launch allows, the drone still leaves its bay and lands.
        let host = ThinkHost::inline();
        let mut drone = rig::drone(1, 120, None);
        let stand = off(DRONE_LAUNCH_STANDOFF_M);
        if let Some(flight) = drone.flight.as_mut() {
            for ship in [BAY_SHIP, DRONE_SHIP] {
                if let Some(ship) = flight.bubble.ships.get_mut(ship) {
                    ship.pos_m = stand;
                }
            }
            flight.tape = None;
        }
        let mut session = rig::session(vec![drone]);
        session.ship_pos_m = stand;
        let over = rig::fly_home(&mut session, &host, 0, 120, Seat::DRONE);
        let (_, end, _) = over.first().expect("one sortie");
        assert_eq!(*end, Ended::Landed { hold_kg: 4_000 });
    }

    /// **A drone launched while a batch of thinks is out thinks no sooner for a restart**
    /// (doc 44 units MW.11 and MW.12; the second reader's finding). A session read back
    /// collects again the batch it had out, from the drones that batch asked. A drone
    /// launched since was not asked: with the restart or without it, its first think is
    /// collected by the next step.
    #[test]
    fn mw12_a_drone_launched_while_thinks_are_out_thinks_no_sooner_for_a_restart() {
        let fly = |restart: bool| {
            let host = ThinkHost::inline();
            let mut session = rig::session(vec![rig::drone(1, 120, None)]);
            for wall in 1..=5 {
                let over = rig::tick(&mut session, &host, wall, 120, Seat::DRONE);
                assert!(over.is_empty());
            }
            assert!(session.drone_thinks_owed.is_some(), "a batch is out");
            // The second drone is launched now, with that batch out.
            session.active_drones.push(rig::drone(2, 120, None));
            if restart {
                let stored = serde_json::to_string(&session).expect("a document");
                let mut read_back: ActiveClusterMiningSession =
                    serde_json::from_str(&stored).expect("it reads back");
                assert!(read_back.active_drones[0].think_out);
                assert!(!read_back.active_drones[1].think_out);
                let fresh = ThinkHost::inline();
                rig::fly_home(&mut read_back, &fresh, 5, 120, Seat::DRONE)
            } else {
                rig::fly_home(&mut session, &host, 5, 120, Seat::DRONE)
            }
        };
        let (straight, restarted) = (fly(false), fly(true));
        assert_eq!(straight.len(), 2);
        for ((drone, end, steps), (again, end_again, steps_again)) in
            straight.iter().zip(&restarted)
        {
            assert_eq!(drone.slot, again.slot);
            assert_eq!(
                (end, steps),
                (end_again, steps_again),
                "slot {}",
                drone.slot
            );
            assert_eq!(drone.mind_think_seq, again.mind_think_seq);
            assert_eq!(drone.mind_memory, again.mind_memory);
            assert_eq!(
                drone.flight.as_ref().map(|f| f.bubble.state_hash()),
                again.flight.as_ref().map(|f| f.bubble.state_hash())
            );
        }
    }
}
