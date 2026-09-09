//! One travel system (S2, 14-spatial-implementation-plan §3.2; doc 14 §2.4, §3.3).
//!
//! A **journey** moves a traveler along a **way** — a corridor [`ConnectionRec`] with its
//! checkpoints, or a timed portal's implicit one-leg connection — on the world IC clock.
//! Three kinds of traveler ride the same record: a character on foot (or by car or
//! transit, which need no object), a plain object, and a **carrier**: a room-model
//! vehicle such as the studio tour bus, whose passengers keep the one location they
//! have — the vehicle's room — for the whole passage (P1), and whose doors open only
//! at the stop it stands at.
//!
//! ## The clock (P6, P7)
//!
//! A journey is stamped and advanced on `WorldIc`. Each leg's IC duration is fixed at
//! departure by [`plan_legs`]: the connection's `base_s` are IC seconds; real time is
//! IC over the campaign rate; the `[atlas].dwell_floor_s` floor holds a traveler at
//! every checkpoint long enough for a stake-out to act, and `[atlas].real_cap_s.<mode>`
//! caps the whole passage so a city crossing never eats a real hour. The **leg cursor is
//! the idempotency key**: the tick advances one boundary per slice, a catch-up after
//! downtime observes every boundary it crossed (a watched one still holds), moves the
//! traveler once and narrates nothing stale, and a world rate-generation change
//! **relabels** the stamps and moves no boundary (`journey.rs`'s fold R2, kept).
//!
//! ## Holds
//!
//! `+stakeout <checkpoint>` materializes a **patch** — one room made from the region's
//! background at the checkpoint's point — and registers a watch; a traveler reaching a
//! watched checkpoint is **held** there and both sides are told. `+intercept` holds a
//! named traveler when it reaches the interceptor, or a vehicle standing at the
//! interceptor's stop. Release shifts the schedule by the hold and grants
//! [`RESUME_GRACE_S`] against re-interception. Cancel, disconnect, a retired or
//! re-applied way, or a destroyed traveler leaves the traveler at its last committed
//! node; a restart resumes from the cursor.
//!
//! ## Knowledge (K)
//!
//! `+travel` lists what the character knows: public access points, plus the
//! per-character `known_places` set that grows by visiting. No rendering here names a
//! traveler's destination to anyone but the traveler and staff.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use mush_content::ContentState;
use mush_core::clock::{ClockRef, ClockStamp};
use mush_core::module::{EventHook, HookReadContext, Perceive};
use mush_core::output::{display_name, name_span, primary_name};
use mush_core::{Attr, Ctx, Flags, GOD, ObjId, ObjKind, World, WorldEvent};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::module::{AtlasState, del_doc, put_doc};
use crate::record::{
    ConnectionRec, ConnectionStatus, Direction, Endpoint, MAX_KNOWN_PLACES, MAX_LEGS, PortalRec,
    RecordKey, Records, TimePolicy, Visibility,
};

// ---------------------------------------------------------------------------
// Constants (plan §7)
// ---------------------------------------------------------------------------

/// The tick tag.
pub const TICK_TAG: &str = "atlas.travel";
/// The tick cadence, seconds. A journey's own due stamps are the authority; this
/// sets how often they are checked and the interception latency.
pub const TICK_SECS: u64 = 1;
/// The most live journeys one world holds.
pub const MAX_JOURNEYS: usize = 1024;
/// Journeys looked at per fire.
pub const MAX_JOURNEYS_VISITED_PER_FIRE: usize = 64;
/// Boundaries applied per fire.
pub const MAX_JOURNEYS_WORKED_PER_FIRE: usize = 16;
/// A boundary crossed within this many **real** seconds of its due stamp is live
/// (narrated); an older one is a catch-up and silent.
pub const LIVE_GRACE_S: i64 = 2;
/// After a release the journey cannot be re-held by a player for this many real
/// seconds (`route.rs`'s anti-grief grace, kept).
pub const RESUME_GRACE_S: i64 = 5;
/// The most bytes an anomaly note keeps.
pub const MAX_ANOMALY_BYTES: usize = 240;
/// The most timed-portal departures the hook queues between two ticks.
pub const MAX_PENDING_DEPARTURES: usize = 64;
/// The most watches (stake-outs) at once.
pub const MAX_WATCHES: usize = 256;
/// The default `[atlas].dwell_floor_s`.
pub const DEFAULT_DWELL_FLOOR_S: u32 = 30;
/// The default `[atlas].real_cap_s.<mode>` for `foot`, `car`, `transit` and `bus`.
pub const DEFAULT_REAL_CAP_S: u32 = 180;
/// The modes a character travels by with no object of their own (doc 14 §3.3).
pub const SELF_MODES: [&str; 3] = ["foot", "car", "transit"];
/// The modes `real_cap_s` defaults for.
pub const CAPPED_MODES: [&str; 4] = ["foot", "car", "transit", "bus"];
/// The most seconds a service vehicle dwells at a stop.
pub const MAX_DWELL_S: u32 = 86_400;

/// The persisted key of the allocator and rotation cursor.
pub const META_KEY: &str = "travel_meta";
/// Key prefixes of the travel documents (`mod:atlas:<prefix>:…`).
pub const KIND_JOURNEY: &str = "journey";
/// The known-places document of one character.
pub const KIND_KNOWN: &str = "known";
/// A way's transit room.
pub const KIND_TRANSIT: &str = "transit";
/// A checkpoint's patch room.
pub const KIND_PATCH: &str = "patch";
/// A connection's service (where its vehicle stands).
pub const KIND_SERVICE: &str = "service";

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// A journey's durable id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct JourneyId(pub u64);

impl std::fmt::Display for JourneyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "J{}", self.0)
    }
}

/// Who or what is travelling (doc 14 §2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "obj", rename_all = "snake_case")]
pub enum Traveler {
    /// A character, on foot or riding.
    Character(ObjId),
    /// A plain object.
    Object(ObjId),
    /// A room-model vehicle: the room everyone aboard is in.
    Carrier(ObjId),
}

impl Traveler {
    /// The object.
    #[must_use]
    pub fn obj(self) -> ObjId {
        match self {
            Traveler::Character(o) | Traveler::Object(o) | Traveler::Carrier(o) => o,
        }
    }

    /// Whether this traveler is a vehicle.
    #[must_use]
    pub fn is_carrier(self) -> bool {
        matches!(self, Traveler::Carrier(_))
    }
}

/// What a journey travels along.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "key", rename_all = "snake_case")]
pub enum Way {
    /// A corridor connection with its checkpoints.
    Connection(RecordKey),
    /// A timed portal: one leg, from its `from` room to its `to` room.
    Portal(RecordKey),
}

impl Way {
    /// The record key.
    #[must_use]
    pub fn key(&self) -> &RecordKey {
        match self {
            Way::Connection(k) | Way::Portal(k) => k,
        }
    }
}

/// Which way along the way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Heading {
    /// From the first endpoint to the second.
    Forward,
    /// From the second endpoint to the first.
    Reverse,
}

/// One node of a journey: its origin room, a checkpoint, or its destination room.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    /// A room.
    Room {
        /// The room.
        room: ObjId,
    },
    /// A checkpoint.
    Checkpoint {
        /// The checkpoint.
        key: RecordKey,
    },
}

/// Why a journey is held.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum HoldReason {
    /// A stake-out at the checkpoint.
    Watch {
        /// The watcher.
        by: ObjId,
    },
    /// `+intercept`.
    Intercept {
        /// The interceptor.
        by: ObjId,
    },
    /// `@atlas/journey hold`.
    Staff {
        /// Who.
        by: ObjId,
    },
}

impl HoldReason {
    /// Who holds.
    #[must_use]
    pub fn by(&self) -> ObjId {
        match self {
            HoldReason::Watch { by } | HoldReason::Intercept { by } | HoldReason::Staff { by } => {
                *by
            }
        }
    }
}

/// Where a journey has got to (doc 14 §2.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JourneyState {
    /// Advancing.
    UnderWay,
    /// Stopped at its current node.
    Held {
        /// Why.
        by: HoldReason,
        /// The IC stamp the hold began at.
        since: ClockStamp,
    },
    /// Terminal: reached its destination.
    Arrived,
    /// Terminal: ended early.
    Aborted,
}

impl JourneyState {
    /// Whether the record is still live.
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self, JourneyState::UnderWay | JourneyState::Held { .. })
    }
}

/// A journey (doc 14 §2.4 `JourneyRec`): a bounded whole document at
/// `mod:atlas:journey:<id>`, strict on the way in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JourneyRec {
    /// Durable id.
    pub id: JourneyId,
    /// Who travels.
    pub traveler: Traveler,
    /// Along what.
    pub way: Way,
    /// The travel mode.
    pub mode: String,
    /// Which way.
    pub heading: Heading,
    /// The nodes, origin first, destination last.
    pub nodes: Vec<Node>,
    /// The IC seconds of each leg (`nodes.len() - 1` of them), fixed at departure.
    pub legs_ic: Vec<u32>,
    /// The `WorldIc` stamp the journey departed at (shifted forward by holds).
    pub departed: ClockStamp,
    /// The `WorldIc` position of the last applied advance.
    pub advanced_at: ClockStamp,
    /// Legs already applied: the traveler is at `nodes[cursor]`.
    pub cursor: u32,
    /// Where it has got to.
    pub state: JourneyState,
    /// No player hold before this real unix second.
    #[serde(default)]
    pub grace_until: i64,
    /// The way's revision at departure; a way re-applied under way ends the journey.
    #[serde(default)]
    pub applied_revision: u64,
    /// A diagnosis, if one stands.
    #[serde(default)]
    pub anomaly: Option<String>,
}

impl JourneyRec {
    /// Re-check every invariant a stored record must hold.
    ///
    /// # Errors
    ///
    /// A message naming the field.
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.departed.clock, ClockRef::WorldIc { .. }) {
            return Err("a journey is stamped on WorldIc".to_string());
        }
        if self.departed.clock != self.advanced_at.clock {
            return Err("a journey's two stamps name one generation".to_string());
        }
        if self.nodes.len() < 2 || self.nodes.len() > MAX_LEGS + 1 {
            return Err(format!("a journey has 2 to {} nodes", MAX_LEGS + 1));
        }
        if self.legs_ic.len() + 1 != self.nodes.len() {
            return Err("a journey has one leg between each pair of nodes".to_string());
        }
        if usize::try_from(self.cursor).unwrap_or(usize::MAX) >= self.nodes.len() {
            return Err("the cursor is past the last node".to_string());
        }
        if self.mode.is_empty() || self.mode.len() > 32 {
            return Err("a mode is one word".to_string());
        }
        if self
            .anomaly
            .as_ref()
            .is_some_and(|a| a.len() > MAX_ANOMALY_BYTES)
        {
            return Err("the anomaly is too long".to_string());
        }
        if let JourneyState::Held { since, .. } = &self.state
            && since.clock != self.departed.clock
        {
            return Err("a hold is stamped on the journey's generation".to_string());
        }
        Ok(())
    }

    /// The absolute IC position at which node `k + 1` is reached (leg `k` done), or
    /// `None` past the end or on overflow.
    #[must_use]
    pub fn leg_due_at(&self, k: usize) -> Option<i64> {
        if k >= self.legs_ic.len() {
            return None;
        }
        let mut at = self.departed.at;
        for leg in self.legs_ic.iter().take(k + 1) {
            at = at.checked_add(i64::from(*leg))?;
        }
        Some(at)
    }

    /// The IC position of arrival.
    #[must_use]
    pub fn arrival_at(&self) -> Option<i64> {
        self.leg_due_at(self.legs_ic.len().checked_sub(1)?)
    }

    /// The IC position at which the next node is reached, if any.
    #[must_use]
    pub fn next_due(&self) -> Option<i64> {
        self.leg_due_at(usize::try_from(self.cursor).ok()?)
    }

    /// The node the traveler is at.
    #[must_use]
    pub fn here(&self) -> Option<&Node> {
        self.nodes.get(usize::try_from(self.cursor).ok()?)
    }

    /// The node ahead.
    #[must_use]
    pub fn ahead(&self) -> Option<&Node> {
        self.nodes
            .get(usize::try_from(self.cursor).ok()?.checked_add(1)?)
    }

    /// The origin room.
    #[must_use]
    pub fn origin(&self) -> Option<ObjId> {
        match self.nodes.first() {
            Some(Node::Room { room }) => Some(*room),
            _ => None,
        }
    }

    /// The destination room.
    #[must_use]
    pub fn destination(&self) -> Option<ObjId> {
        match self.nodes.last() {
            Some(Node::Room { room }) => Some(*room),
            _ => None,
        }
    }

    /// Whether the cursor stands at the last node.
    #[must_use]
    pub fn at_destination(&self) -> bool {
        usize::try_from(self.cursor).unwrap_or(usize::MAX) + 1 >= self.nodes.len()
    }
}

/// The known-places document of one character.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownDoc {
    /// The neighborhoods, in the order learnt.
    pub places: Vec<RecordKey>,
}

/// A room made for a way or a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomDoc {
    /// The room.
    pub room: ObjId,
    /// Doors a held vehicle has into it: `(vehicle room, exit out of the vehicle,
    /// exit back in)`.
    #[serde(default)]
    pub doors: Vec<(ObjId, ObjId, ObjId)>,
}

/// Which end of a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    /// The first endpoint.
    From,
    /// The second endpoint.
    To,
}

impl End {
    /// The other end.
    #[must_use]
    pub fn other(self) -> End {
        match self {
            End::From => End::To,
            End::To => End::From,
        }
    }
}

/// Where a connection's service vehicle stands (`mod:atlas:service:<pack>:<entry>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDoc {
    /// Which end it stands at (its next departure is toward the other).
    pub at: End,
    /// The real unix second it arrived there.
    pub since_real: i64,
}

/// The allocator and rotation cursor (`mod:atlas:travel_meta`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetaDoc {
    /// The next journey id.
    pub next_id: u64,
    /// The rotation cursor.
    pub rotation: u64,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// A stake-out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    /// The checkpoint watched.
    pub checkpoint: RecordKey,
    /// Where the watcher came from, to go back to.
    pub from: Option<ObjId>,
}

/// An `+intercept` on a traveler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intercept {
    /// The interceptor.
    pub by: ObjId,
    /// Where the hold lands.
    pub at: Node,
}

/// A service's live state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceState {
    /// The document.
    pub doc: ServiceDoc,
    /// Held at its stop by an interceptor.
    pub held_by: Option<ObjId>,
}

/// A timed-portal departure the hook queued for the tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingDeparture {
    /// Who.
    pub actor: ObjId,
    /// Through which exit.
    pub exit: ObjId,
}

/// The queue between the read-only hook and the tick.
pub type Pending = Arc<Mutex<Vec<PendingDeparture>>>;

fn lock(pending: &Pending) -> std::sync::MutexGuard<'_, Vec<PendingDeparture>> {
    match pending.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The travel half of the atlas's state slot.
#[derive(Clone, Debug, Default)]
pub struct TravelState {
    /// Live journeys.
    pub journeys: BTreeMap<JourneyId, JourneyRec>,
    /// The live journey of each traveler.
    pub by_traveler: BTreeMap<ObjId, JourneyId>,
    /// The allocator.
    pub next_id: u64,
    /// The rotation cursor.
    pub rotation: u64,
    /// Each way's transit room.
    pub transit: BTreeMap<RecordKey, RoomDoc>,
    /// Each materialized checkpoint's patch room.
    pub patches: BTreeMap<RecordKey, RoomDoc>,
    /// Stake-outs, by watcher.
    pub watches: BTreeMap<ObjId, Watch>,
    /// Intercepts, by traveler.
    pub intercepts: BTreeMap<ObjId, Intercept>,
    /// Known places, by character.
    pub known: BTreeMap<ObjId, Vec<RecordKey>>,
    /// Service vehicles, by connection.
    pub services: BTreeMap<RecordKey, ServiceState>,
    /// Travelers whose journey ended on the road: the way, the node they stopped at
    /// and the heading they had, so `+travel` can offer the ends. RAM only: after a
    /// restart a stranded traveler sets out from the near end.
    pub stranded: BTreeMap<ObjId, (Way, u32, Heading)>,
}

impl TravelState {
    /// The live journey of `obj`, if any.
    #[must_use]
    pub fn journey_of(&self, obj: ObjId) -> Option<&JourneyRec> {
        self.by_traveler
            .get(&obj)
            .and_then(|id| self.journeys.get(id))
    }

    /// Whether `room` is a way's transit room.
    #[must_use]
    pub fn is_transit(&self, room: ObjId) -> bool {
        self.transit.values().any(|d| d.room == room)
    }

    /// The checkpoint whose patch `room` is.
    #[must_use]
    pub fn patch_at(&self, room: ObjId) -> Option<&RecordKey> {
        self.patches
            .iter()
            .find(|(_, d)| d.room == room)
            .map(|(k, _)| k)
    }

    /// Whether `checkpoint` has a watcher.
    #[must_use]
    pub fn watched(&self, checkpoint: &RecordKey) -> Option<ObjId> {
        self.watches
            .iter()
            .find(|(_, w)| &w.checkpoint == checkpoint)
            .map(|(who, _)| *who)
    }

    /// Whether `who` knows `place`.
    #[must_use]
    pub fn knows(&self, who: ObjId, place: &RecordKey) -> bool {
        self.known
            .get(&who)
            .is_some_and(|places| places.contains(place))
    }
}

// ---------------------------------------------------------------------------
// The clock planner (pure)
// ---------------------------------------------------------------------------

/// The IC seconds of each leg, from the way's base IC seconds, the campaign rate,
/// the per-checkpoint dwell floor and the per-mode real cap (doc 14 §3.3).
///
/// Real time is IC over the rate. The cap scales every leg down proportionally when
/// the passage would run longer than `cap_s` real seconds; the floor then lifts every
/// leg **after the first** (the dwell at the checkpoint it leaves) to at least
/// `floor_s` real seconds, so a stake-out can act. A rate of zero (IC paused) plans as
/// a rate of one: nothing is due while it stays paused.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn plan_legs(base_ic: &[u32], rate: u32, floor_s: u32, cap_s: Option<u32>) -> Vec<u32> {
    let rate = f64::from(rate.max(1));
    let mut real: Vec<f64> = base_ic
        .iter()
        .map(|b| f64::from((*b).max(1)) / rate)
        .collect();
    let total: f64 = real.iter().sum();
    if let Some(cap) = cap_s
        && total > f64::from(cap)
        && total > 0.0
    {
        let scale = f64::from(cap) / total;
        for r in &mut real {
            *r *= scale;
        }
    }
    let floor = f64::from(floor_s);
    for (i, r) in real.iter_mut().enumerate() {
        if i > 0 && *r < floor {
            *r = floor;
        }
    }
    let mut legs: Vec<u32> = real
        .into_iter()
        .map(|r| {
            let ic = (r * rate).round();
            if ic < 1.0 {
                1
            } else if ic > f64::from(u32::MAX) {
                u32::MAX
            } else {
                // Bounded above and below just before; the cast cannot truncate.
                ic as u32
            }
        })
        .collect();
    // Rounding may carry the whole over the cap by a second or two: take those
    // back from the longest legs, never below one second or the floor.
    if let Some(cap) = cap_s {
        let floor_ic = (floor * rate).round();
        let floor_ic = if floor_ic > f64::from(u32::MAX) {
            u32::MAX
        } else {
            floor_ic as u32
        };
        let cap_ic = (f64::from(cap) * rate).round();
        let cap_ic = if cap_ic > 1.8e19 {
            u64::MAX
        } else {
            cap_ic as u64
        };
        let mut guard = 0usize;
        while legs.iter().map(|l| u64::from(*l)).sum::<u64>() > cap_ic && guard < 1_024 {
            guard += 1;
            let Some((i, _)) = legs
                .iter()
                .enumerate()
                .filter(|(i, l)| **l > 1 && (*i == 0 || **l > floor_ic))
                .max_by_key(|(_, l)| **l)
            else {
                break;
            };
            if let Some(l) = legs.get_mut(i) {
                *l -= 1;
            }
        }
    }
    legs
}

/// Metres per second for a mode, for a timed portal's implicit leg.
#[must_use]
pub fn mode_speed_mps(mode: &str) -> f64 {
    match mode {
        "car" => 12.0,
        "transit" => 10.0,
        "bus" | "ship" => 8.0,
        "shuttle" => 100.0,
        "torch" => 1_000.0,
        _ => 1.4,
    }
}

/// The base IC seconds of a timed portal by `mode`: the walk at the mode's speed plus
/// two seconds a metre of climb, at least one.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn portal_base_s(portal: &PortalRec, mode: &str) -> u32 {
    let walk = f64::from(portal.distance_m) / mode_speed_mps(mode);
    let climb = portal.dz_m.abs() * 2.0;
    let total = (walk + climb).ceil();
    if total < 1.0 {
        1
    } else if total > f64::from(u32::MAX) {
        u32::MAX
    } else {
        total as u32
    }
}

/// An exit's name as prose: its primary name without the `<HINT>`.
#[must_use]
pub fn exit_name(name: &str) -> String {
    primary_name(name)
        .split('<')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The current campaign rate: IC seconds per real second.
#[must_use]
pub fn rate_of(w: &World) -> u32 {
    w.world_timeline().current().rate
}

/// Real seconds until IC position `at`, at the current rate (never negative).
#[must_use]
pub fn real_until(w: &World, at: i64) -> i64 {
    let now = w.world_ic_now().at;
    let ic = at.saturating_sub(now).max(0);
    let rate = i64::from(rate_of(w).max(1));
    ic.saturating_add(rate - 1) / rate
}

fn about(secs: i64) -> String {
    if secs <= 5 {
        "moments".to_string()
    } else if secs < 90 {
        format!("about {secs} seconds")
    } else {
        let mins = (secs + 30) / 60;
        format!("about {mins} minute{}", if mins == 1 { "" } else { "s" })
    }
}

// ---------------------------------------------------------------------------
// Resolution: a way into nodes and base legs
// ---------------------------------------------------------------------------

/// A way resolved against the records: its nodes and base IC legs.
#[derive(Clone, Debug)]
pub struct Route {
    /// The name shown.
    pub name: String,
    /// The nodes.
    pub nodes: Vec<Node>,
    /// Base IC seconds per leg.
    pub base_ic: Vec<u32>,
    /// The way's revision.
    pub revision: u64,
}

/// The room a connection endpoint lands in: the applied neighborhood's room whose
/// footprint holds the access point, else the nearest by centre.
#[must_use]
pub fn access_room(records: &Records, ap: &crate::record::AccessPoint) -> Option<ObjId> {
    if let Some(room) = ap.room
        && records.roomgeo.contains_key(&room)
    {
        return Some(room);
    }
    let nb = records
        .neighborhoods
        .values()
        .find(|n| n.key.pack == ap.neighborhood)?;
    let p = ap.point.xy().to_cm().ok()?;
    let mut nearest: Option<(i64, ObjId)> = None;
    for geo in records.rooms_in(&nb.key) {
        let Ok(shape) = geo.footprint.shape() else {
            continue;
        };
        if shape.contains(p) {
            return Some(geo.room);
        }
        let b = shape.bbox();
        let cx = (b.min.x + b.max.x) / 2;
        let cy = (b.min.y + b.max.y) / 2;
        let d = (cx - p.x).abs().saturating_add((cy - p.y).abs());
        if nearest.is_none_or(|(best, _)| d < best) {
            nearest = Some((d, geo.room));
        }
    }
    nearest.map(|(_, room)| room)
}

/// Resolve a connection in `heading` into nodes and base legs.
///
/// # Errors
///
/// A message when an endpoint has no applied room, a leg names an unknown checkpoint,
/// or the legs do not end at the far endpoint.
pub fn resolve_connection(
    records: &Records,
    conn: &ConnectionRec,
    heading: Heading,
) -> Result<Route, String> {
    let from = access_room(records, &conn.endpoints.0)
        .ok_or_else(|| format!("{} has no room at its first end yet", conn.name))?;
    let to = access_room(records, &conn.endpoints.1)
        .ok_or_else(|| format!("{} has no room at its second end yet", conn.name))?;
    if conn.legs.is_empty() {
        return Err(format!("{} has no legs", conn.name));
    }
    let mut nodes = vec![Node::Room { room: from }];
    let mut base = Vec::with_capacity(conn.legs.len());
    let last = conn.legs.len() - 1;
    for (i, leg) in conn.legs.iter().enumerate() {
        if i == last {
            if leg.to != conn.key {
                return Err(format!(
                    "{}: the last leg ends at the far end (named by the connection's own key)",
                    conn.name
                ));
            }
            nodes.push(Node::Room { room: to });
        } else {
            if !records.checkpoints.contains_key(&leg.to) {
                return Err(format!("{}: a leg names an unknown checkpoint", conn.name));
            }
            nodes.push(Node::Checkpoint {
                key: leg.to.clone(),
            });
        }
        base.push(leg.base_s.max(1));
    }
    if heading == Heading::Reverse {
        nodes.reverse();
        base.reverse();
    }
    Ok(Route {
        name: conn.name.clone(),
        nodes,
        base_ic: base,
        revision: conn.revision,
    })
}

/// Resolve a timed portal through `exit` into its one-leg route.
///
/// # Errors
///
/// A message when the portal is instant or has no rooms at both ends.
pub fn resolve_portal(
    w: &World,
    portal: &PortalRec,
    exit: ObjId,
    mode: &str,
) -> Result<Route, String> {
    let (Endpoint::Room(a), Endpoint::Room(b)) = (portal.from, portal.to) else {
        return Err("that way leads outside the map".to_string());
    };
    let src = w.get(exit).and_then(|o| o.location);
    let (from, to) = if src == Some(b) { (b, a) } else { (a, b) };
    let name = w
        .get(exit)
        .map(|o| exit_name(&o.name))
        .unwrap_or_else(|| "the way".to_string());
    Ok(Route {
        name,
        nodes: vec![Node::Room { room: from }, Node::Room { room: to }],
        base_ic: vec![portal_base_s(portal, mode)],
        revision: portal.revision,
    })
}

/// The mode a character takes through a timed portal: the first of the portal's
/// modes they can use on their own.
#[must_use]
pub fn portal_mode(portal: &PortalRec) -> Option<String> {
    match &portal.time {
        TimePolicy::Timed { modes } => modes
            .iter()
            .find(|m| SELF_MODES.contains(&m.as_str()))
            .or_else(|| modes.first())
            .cloned(),
        TimePolicy::Instant => None,
    }
}

// ---------------------------------------------------------------------------
// Names and narration
// ---------------------------------------------------------------------------

/// A node's name as shown.
#[must_use]
pub fn node_name(records: &Records, w: &World, node: &Node) -> String {
    match node {
        Node::Room { room } => display_name(w, *room),
        Node::Checkpoint { key } => records
            .checkpoints
            .get(key)
            .map_or_else(|| "a checkpoint".to_string(), |c| c.name.clone()),
    }
}

fn emit(text: RichText) -> OutputEvent {
    OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: None,
        text,
    }
}

fn tell_room(ctx: &mut Ctx<'_>, room: ObjId, subject: ObjId, text: RichText) {
    ctx.out.to_room_from(room, Vec::new(), subject, emit(text));
}

fn tell(ctx: &mut Ctx<'_>, who: ObjId, text: impl Into<String>) {
    ctx.out.to_actor(
        who,
        OutputEvent::Sys(RichText::tag(crate::module::NS, text.into())),
    );
}

fn key_of(kind: &str, key: &RecordKey) -> SmolStr {
    crate::record::record_key(kind, key)
}

fn journey_key(id: JourneyId) -> SmolStr {
    SmolStr::new(format!("{KIND_JOURNEY}:{}", id.0))
}

fn known_key(who: ObjId) -> SmolStr {
    SmolStr::new(format!("{KIND_KNOWN}:{}", who.0))
}

fn persist(ctx: &mut Ctx<'_>, j: &JourneyRec) {
    put_doc(ctx, journey_key(j.id), j);
}

fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(st) = ctx.state::<AtlasState>() else {
        return;
    };
    let doc = MetaDoc {
        next_id: st.travel.next_id,
        rotation: st.travel.rotation,
    };
    put_doc(ctx, SmolStr::new_static(META_KEY), &doc);
}

fn state_of<'a>(ctx: &'a Ctx<'_>) -> Option<&'a AtlasState> {
    ctx.state::<AtlasState>()
}

fn state_mut<'a>(ctx: &'a mut Ctx<'_>) -> Option<&'a mut AtlasState> {
    ctx.state_mut::<AtlasState>()
}

/// The driver's name for a service vehicle's announcements: the pack's driver NPC if
/// the latch knows it, else "the driver".
fn driver_name(ctx: &Ctx<'_>, conn: &ConnectionRec) -> String {
    conn.service
        .as_ref()
        .and_then(|s| s.driver.as_ref())
        .and_then(|d| latched_object(ctx.w, d))
        .map_or_else(|| "The driver".to_string(), |o| display_name(ctx.w, o))
}

/// Resolve a pack entry reference through the content latch.
#[must_use]
pub fn latched_object(w: &World, key: &RecordKey) -> Option<ObjId> {
    let content = w.module_state::<ContentState>()?;
    let latch = content.latches.applied(&key.pack)?;
    let obj = latch.object_of(&key.entry)?;
    w.get(obj)
        .filter(|o| !o.flags.contains(Flags::GOING))
        .map(|o| o.id)
}

/// The vehicle room of a connection's service, if the latch knows it.
#[must_use]
pub fn service_vehicle(w: &World, conn: &ConnectionRec) -> Option<ObjId> {
    let service = conn.service.as_ref()?;
    latched_object(w, &service.vehicle)
        .filter(|o| w.get(*o).is_some_and(|o| o.kind == ObjKind::Room))
}

// ---------------------------------------------------------------------------
// Rooms the atlas makes: transit and patch
// ---------------------------------------------------------------------------

fn make_room(ctx: &mut Ctx<'_>, name: &str, desc: &str) -> Result<ObjId, String> {
    let room = ctx
        .w
        .create(ObjKind::Room, name, GOD, None)
        .map_err(|e| e.to_string())?;
    ctx.emit_world_event(&WorldEvent::ObjCreated {
        obj: room,
        kind: ObjKind::Room,
        creator: GOD,
    });
    ctx.w
        .set_attr(room, "desc", Attr::text(desc, GOD))
        .map_err(|e| e.to_string())?;
    Ok(room)
}

fn make_exit(ctx: &mut Ctx<'_>, src: ObjId, spec: &str, dest: ObjId) -> Result<ObjId, String> {
    let mut segs = spec.split(';').map(str::trim).filter(|s| !s.is_empty());
    let name = segs
        .next()
        .ok_or_else(|| "an exit needs a name".to_string())?;
    let exit = ctx
        .w
        .create(ObjKind::Exit, name, GOD, Some(src))
        .map_err(|e| e.to_string())?;
    for alias in segs {
        ctx.w.add_alias(exit, alias).map_err(|e| e.to_string())?;
    }
    ctx.w
        .link_exit(exit, Some(dest))
        .map_err(|e| e.to_string())?;
    ctx.emit_world_event(&WorldEvent::ObjCreated {
        obj: exit,
        kind: ObjKind::Exit,
        creator: GOD,
    });
    Ok(exit)
}

fn destroy_quietly(ctx: &mut Ctx<'_>, obj: ObjId) {
    if ctx.w.get(obj).is_none() {
        return;
    }
    if let Err(error) = ctx.w.destroy(obj) {
        tracing::warn!(%obj, %error, "atlas: a travel room or door would not go");
    } else {
        ctx.emit_world_event(&WorldEvent::ObjDestroyed { obj });
    }
}

/// The transit room of a way, made on first use.
///
/// # Errors
///
/// The world refusing the room.
pub fn transit_room(ctx: &mut Ctx<'_>, way: &Way, name: &str) -> Result<ObjId, String> {
    if let Some(doc) = state_of(ctx).and_then(|st| st.travel.transit.get(way.key()))
        && ctx.w.get(doc.room).is_some_and(|o| o.kind == ObjKind::Room)
    {
        return Ok(doc.room);
    }
    let (room_name, desc) = match way {
        Way::Connection(_) => (
            format!("{name}, under way"),
            format!(
                "The {name}, between stops: road, and the places it passes. \
                 +travel/status says where you have got to."
            ),
        ),
        Way::Portal(_) => (
            format!("{name}, on the way"),
            format!("On the way along {name}. +travel/status says how far there is to go."),
        ),
    };
    let room = make_room(ctx, &room_name, &desc)?;
    let doc = RoomDoc {
        room,
        doors: Vec::new(),
    };
    put_doc(ctx, key_of(KIND_TRANSIT, way.key()), &doc);
    if let Some(st) = state_mut(ctx) {
        st.travel.transit.insert(way.key().clone(), doc);
    }
    Ok(room)
}

/// The patch room at a checkpoint, made from the background on first use.
///
/// # Errors
///
/// An unknown checkpoint, or the world refusing the room.
pub fn patch_room(ctx: &mut Ctx<'_>, checkpoint: &RecordKey) -> Result<ObjId, String> {
    if let Some(doc) = state_of(ctx).and_then(|st| st.travel.patches.get(checkpoint))
        && ctx.w.get(doc.room).is_some_and(|o| o.kind == ObjKind::Room)
    {
        return Ok(doc.room);
    }
    let (name, desc) = {
        let st = state_of(ctx).ok_or_else(|| "the atlas is not composed".to_string())?;
        let cp = st
            .records
            .checkpoints
            .get(checkpoint)
            .ok_or_else(|| "no such checkpoint".to_string())?;
        let region = st.records.regions.get(&cp.region);
        let ground = region.and_then(|r| {
            let p = cp.point.xy().to_cm().ok()?;
            Some(crate::background::background(
                r,
                &crate::record::SurfaceRef::Ground,
                p.x,
                p.y,
            ))
        });
        let surface = ground.map_or_else(
            || "open".to_string(),
            |g| format!("{:?}", g.class).to_ascii_lowercase(),
        );
        let ways: Vec<String> = st
            .records
            .connections
            .values()
            .filter(|c| c.legs.iter().any(|l| &l.to == checkpoint))
            .map(|c| c.name.clone())
            .collect();
        let passes = if ways.is_empty() {
            String::new()
        } else {
            format!(" The {} passes here.", ways.join(" and the "))
        };
        (
            cp.name.clone(),
            format!(
                "{}: {surface} ground beside the road, and a place to wait.{passes} \
                 Whatever comes this way comes past you.",
                cp.name
            ),
        )
    };
    let room = make_room(ctx, &name, &desc)?;
    let doc = RoomDoc {
        room,
        doors: Vec::new(),
    };
    put_doc(ctx, key_of(KIND_PATCH, checkpoint), &doc);
    if let Some(st) = state_mut(ctx) {
        st.travel.patches.insert(checkpoint.clone(), doc);
    }
    Ok(room)
}

/// Who watches `checkpoint`: a registered stake-out, else any player standing in
/// its patch who is not themselves travelling — a person at the checkpoint is a
/// watcher whether or not they typed `+stakeout` (a passenger who stepped off a held
/// vehicle, say).
#[must_use]
pub fn watcher_at(st: &AtlasState, w: &World, checkpoint: &RecordKey) -> Option<ObjId> {
    watcher_at_except(st, w, checkpoint, None)
}

/// [`watcher_at`], not counting `except` (the watcher on their way out).
#[must_use]
pub fn watcher_at_except(
    st: &AtlasState,
    w: &World,
    checkpoint: &RecordKey,
    except: Option<ObjId>,
) -> Option<ObjId> {
    if let Some(who) = st
        .travel
        .watches
        .iter()
        .find(|(who, wt)| &wt.checkpoint == checkpoint && Some(**who) != except)
        .map(|(who, _)| *who)
    {
        return Some(who);
    }
    let patch = st.travel.patches.get(checkpoint)?.room;
    w.contents_of(patch).iter().copied().find(|o| {
        Some(*o) != except
            && w.get(*o).is_some_and(|o| {
                o.kind == ObjKind::Player
                    && !o.flags.contains(Flags::GOING)
                    && st.travel.journey_of(o.id).is_none()
            })
    })
}

/// Where someone leaving a patch on foot ends up: where they came from, else the
/// first end of a road through the checkpoint, else Limbo.
fn way_back(st: &AtlasState, w: &World, checkpoint: &RecordKey, from: Option<ObjId>) -> ObjId {
    if let Some(room) = from
        && w.get(room).is_some_and(|o| o.kind == ObjKind::Room)
    {
        return room;
    }
    st.records
        .connections
        .values()
        .filter(|c| c.legs.iter().any(|l| &l.to == checkpoint))
        .find_map(|c| access_room(&st.records, &c.endpoints.0))
        .unwrap_or(mush_core::LIMBO)
}

/// Retire a patch that nobody watches, nobody is held at and nothing stands in.
fn retire_patch_if_idle(ctx: &mut Ctx<'_>, checkpoint: &RecordKey) {
    let Some((room, doors)) = state_of(ctx).and_then(|st| {
        let doc = st.travel.patches.get(checkpoint)?;
        if watcher_at(st, ctx.w, checkpoint).is_some() {
            return None;
        }
        let held_here = st.travel.journeys.values().any(|j| {
            j.state.is_live()
                && matches!(j.here(), Some(Node::Checkpoint { key }) if key == checkpoint)
        });
        if held_here {
            return None;
        }
        let occupied = ctx.w.contents_of(doc.room).iter().any(|o| {
            ctx.w
                .get(*o)
                .is_some_and(|o| !o.flags.contains(Flags::GOING))
        });
        if occupied {
            return None;
        }
        Some((doc.room, doc.doors.clone()))
    }) else {
        return;
    };
    for (_, out, back) in doors {
        destroy_quietly(ctx, out);
        destroy_quietly(ctx, back);
    }
    destroy_quietly(ctx, room);
    del_doc(ctx, key_of(KIND_PATCH, checkpoint));
    if let Some(st) = state_mut(ctx) {
        st.travel.patches.remove(checkpoint);
    }
}

/// Open a held vehicle's doors into a patch.
fn open_doors(ctx: &mut Ctx<'_>, checkpoint: &RecordKey, vehicle: ObjId, patch: ObjId) {
    let already = state_of(ctx)
        .and_then(|st| st.travel.patches.get(checkpoint))
        .is_some_and(|d| d.doors.iter().any(|(v, _, _)| *v == vehicle));
    if already {
        return;
    }
    let cp_name = state_of(ctx)
        .and_then(|st| st.records.checkpoints.get(checkpoint))
        .map_or_else(|| "here".to_string(), |c| c.name.clone());
    let vehicle_name = display_name(ctx.w, vehicle);
    let out = make_exit(
        ctx,
        vehicle,
        &format!("Off at {cp_name} <OFF>;off;out"),
        patch,
    );
    let back = make_exit(
        ctx,
        patch,
        &format!("Board {vehicle_name} <BOARD>;board;in"),
        vehicle,
    );
    match (out, back) {
        (Ok(out), Ok(back)) => {
            if let Some(st) = state_mut(ctx)
                && let Some(doc) = st.travel.patches.get_mut(checkpoint)
            {
                doc.doors.push((vehicle, out, back));
                let doc = doc.clone();
                put_doc(ctx, key_of(KIND_PATCH, checkpoint), &doc);
            }
        }
        (out, back) => {
            for door in [out, back].into_iter().flatten() {
                destroy_quietly(ctx, door);
            }
            tracing::warn!(%vehicle, "atlas: a held vehicle's doors would not open");
        }
    }
}

/// Close a vehicle's doors into a patch.
fn close_doors(ctx: &mut Ctx<'_>, checkpoint: &RecordKey, vehicle: ObjId) {
    let doors: Vec<(ObjId, ObjId)> = state_of(ctx)
        .and_then(|st| st.travel.patches.get(checkpoint))
        .map(|d| {
            d.doors
                .iter()
                .filter(|(v, _, _)| *v == vehicle)
                .map(|(_, o, b)| (*o, *b))
                .collect()
        })
        .unwrap_or_default();
    if doors.is_empty() {
        return;
    }
    for (out, back) in doors {
        destroy_quietly(ctx, out);
        destroy_quietly(ctx, back);
    }
    if let Some(st) = state_mut(ctx)
        && let Some(doc) = st.travel.patches.get_mut(checkpoint)
    {
        doc.doors.retain(|(v, _, _)| *v != vehicle);
        let doc = doc.clone();
        put_doc(ctx, key_of(KIND_PATCH, checkpoint), &doc);
    }
}

// ---------------------------------------------------------------------------
// Where a vehicle stands
// ---------------------------------------------------------------------------

/// The room a service vehicle's doors open on right now: its stop, or the patch it is
/// held at; `None` while it is under way.
#[must_use]
pub fn vehicle_stop(st: &AtlasState, w: &World, vehicle: ObjId) -> Option<ObjId> {
    if let Some(j) = st.travel.journey_of(vehicle) {
        return match (&j.state, j.here()) {
            (JourneyState::Held { .. }, Some(Node::Checkpoint { key })) => {
                st.travel.patches.get(key).map(|d| d.room)
            }
            (JourneyState::Held { .. }, Some(Node::Room { room })) => Some(*room),
            _ => None,
        };
    }
    let (conn, service) = st.travel.services.iter().find_map(|(key, s)| {
        let conn = st.records.connections.get(key)?;
        (service_vehicle(w, conn) == Some(vehicle)).then_some((conn, s))
    })?;
    let end = match service.doc.at {
        End::From => &conn.endpoints.0,
        End::To => &conn.endpoints.1,
    };
    access_room(&st.records, end)
}

/// The connection a vehicle serves.
#[must_use]
pub fn vehicle_connection<'a>(
    st: &'a AtlasState,
    w: &World,
    vehicle: ObjId,
) -> Option<&'a ConnectionRec> {
    st.records
        .connections
        .values()
        .find(|c| service_vehicle(w, c) == Some(vehicle))
}

// ---------------------------------------------------------------------------
// Depart
// ---------------------------------------------------------------------------

/// Start a journey. The traveler must be free, the way in service, and (for a character
/// or object) the traveler in the way's first room. A character or object is moved into
/// the way's transit room; a vehicle's doors simply close.
///
/// # Errors
///
/// A message for the actor.
pub fn depart(
    ctx: &mut Ctx<'_>,
    traveler: Traveler,
    way: Way,
    heading: Heading,
    mode: &str,
    route: Route,
) -> Result<JourneyId, String> {
    let obj = traveler.obj();
    let live = ctx
        .w
        .get(obj)
        .is_some_and(|o| !o.flags.contains(Flags::GOING));
    if !live {
        return Err("There is nothing there to travel.".to_string());
    }
    let (floor, cap, count, id) = {
        let st = state_of(ctx).ok_or_else(|| "The atlas is not composed.".to_string())?;
        if st.travel.by_traveler.contains_key(&obj) {
            return Err(format!(
                "{} is already under way.",
                display_name(ctx.w, obj)
            ));
        }
        (
            st.limits.dwell_floor_s,
            st.limits.real_cap_s.get(mode).copied(),
            st.travel.journeys.len(),
            JourneyId(st.travel.next_id),
        )
    };
    if count >= MAX_JOURNEYS {
        return Err("Too many journeys are under way; try again later.".to_string());
    }
    let here = ctx.w.get(obj).and_then(|o| o.location);
    let origin = match route.nodes.first() {
        Some(Node::Room { room }) => *room,
        Some(Node::Checkpoint { key }) => {
            // Setting out again from the road: the traveler stands in the way's
            // transit room or at the checkpoint's patch.
            let ok = state_of(ctx).is_some_and(|st| {
                st.travel
                    .transit
                    .get(way.key())
                    .is_some_and(|d| Some(d.room) == here)
                    || st
                        .travel
                        .patches
                        .get(key)
                        .is_some_and(|d| Some(d.room) == here)
            });
            if !ok {
                return Err("You are not on that road.".to_string());
            }
            here.ok_or_else(|| "You are nowhere.".to_string())?
        }
        None => return Err("That way starts nowhere.".to_string()),
    };
    if !traveler.is_carrier() && here != Some(origin) {
        return Err(format!(
            "The {} leaves from {}.",
            route.name,
            display_name(ctx.w, origin)
        ));
    }
    let legs_ic = plan_legs(&route.base_ic, rate_of(ctx.w), floor, cap);
    let now = ctx.w.world_ic_now();
    let rec = JourneyRec {
        id,
        traveler,
        way: way.clone(),
        mode: mode.to_string(),
        heading,
        nodes: route.nodes,
        legs_ic,
        departed: now.clone(),
        advanced_at: now,
        cursor: 0,
        state: JourneyState::UnderWay,
        grace_until: 0,
        applied_revision: route.revision,
        anomaly: None,
    };
    rec.validate()?;
    let ahead = state_of(ctx)
        .and_then(|st| rec.ahead().map(|n| node_name(&st.records, ctx.w, n)))
        .unwrap_or_else(|| "the far end".to_string());
    // The projection first, so a refusal leaves no record.
    if !traveler.is_carrier() {
        let transit = transit_room(ctx, &way, &route.name)?;
        ctx.w
            .move_to(obj, transit)
            .map_err(|e| format!("You cannot set off: {e}"))?;
        let text = RichText::new()
            .span(name_span(ctx.w, obj))
            .text(format!(" sets off along the {}.", route.name));
        tell_room(ctx, origin, obj, text);
        ctx.emit_world_event(&WorldEvent::Moved {
            who: obj,
            from: Some(origin),
            to: transit,
        });
        let eta = rec
            .next_due()
            .map_or_else(|| "a while".to_string(), |at| about(real_until(ctx.w, at)));
        if traveler == Traveler::Character(obj) {
            tell(
                ctx,
                obj,
                format!(
                    "You set off along the {} by {mode}; {ahead} is {eta} off.",
                    route.name
                ),
            );
        }
    } else {
        announce_departure(ctx, &rec, origin);
    }
    persist(ctx, &rec);
    if let Some(st) = state_mut(ctx) {
        st.travel.next_id = st.travel.next_id.saturating_add(1);
        st.travel.by_traveler.insert(obj, id);
        st.travel.journeys.insert(id, rec);
        st.travel.stranded.remove(&obj);
    }
    persist_meta(ctx);
    Ok(id)
}

fn announce_departure(ctx: &mut Ctx<'_>, rec: &JourneyRec, origin: ObjId) {
    let vehicle = rec.traveler.obj();
    let (driver, ahead) = {
        let Some(st) = state_of(ctx) else { return };
        let driver = match &rec.way {
            Way::Connection(key) => st.records.connections.get(key).map(|c| driver_name(ctx, c)),
            Way::Portal(_) => None,
        }
        .unwrap_or_else(|| "The driver".to_string());
        let ahead = rec
            .ahead()
            .map(|n| node_name(&st.records, ctx.w, n))
            .unwrap_or_else(|| "the far end".to_string());
        (driver, ahead)
    };
    let name = display_name(ctx.w, vehicle);
    tell_room(
        ctx,
        vehicle,
        vehicle,
        RichText::new().text(format!(
            "{driver}, over the PA: \"{name} departing. Next stop, {ahead}.\" The doors close and the {name} pulls away."
        )),
    );
    tell_room(
        ctx,
        origin,
        vehicle,
        RichText::new()
            .span(name_span(ctx.w, vehicle))
            .text(" pulls away."),
    );
}

// ---------------------------------------------------------------------------
// The tick: advance, services, pending departures, patch sweep
// ---------------------------------------------------------------------------

/// The `Coarse` tick.
pub(crate) fn tick(ctx: &mut Ctx<'_>, pending: &Pending) {
    drain_pending(ctx, pending);
    advance_due(ctx);
    run_services(ctx);
    sweep_patches(ctx);
}

/// What one journey cost this fire.
enum Step {
    Skipped,
    Worked,
}

/// Advance every journey whose IC position has passed its next boundary: one boundary
/// per slice, worked from a rotation cursor, bounded per fire. Idempotent: re-running
/// it applies nothing past what is applied.
pub fn advance_due(ctx: &mut Ctx<'_>) {
    let now = ctx.w.world_ic_now();
    let order: Vec<JourneyId> = {
        let Some(st) = state_of(ctx) else { return };
        if st.travel.journeys.is_empty() {
            return;
        }
        let start = JourneyId(st.travel.rotation);
        st.travel
            .journeys
            .range(start..)
            .chain(st.travel.journeys.range(..start))
            .map(|(id, _)| *id)
            .take(MAX_JOURNEYS_VISITED_PER_FIRE)
            .collect()
    };
    let mut worked = 0usize;
    let mut last = None;
    for id in &order {
        if worked >= MAX_JOURNEYS_WORKED_PER_FIRE {
            break;
        }
        last = Some(*id);
        for _ in 0..=MAX_LEGS {
            if worked >= MAX_JOURNEYS_WORKED_PER_FIRE {
                break;
            }
            match step(ctx, *id, &now) {
                Step::Skipped => break,
                Step::Worked => worked = worked.saturating_add(1),
            }
        }
    }
    if let Some(last) = last {
        let moved = last.0.saturating_add(1);
        let changed = state_of(ctx).is_some_and(|st| st.travel.rotation != moved);
        if changed {
            if let Some(st) = state_mut(ctx) {
                st.travel.rotation = moved;
            }
            persist_meta(ctx);
        }
    }
}

fn step(ctx: &mut Ctx<'_>, id: JourneyId, now: &ClockStamp) -> Step {
    let Some(journey) = state_of(ctx).and_then(|st| st.travel.journeys.get(&id).cloned()) else {
        return Step::Skipped;
    };
    if !journey.state.is_live() {
        retire(ctx, id);
        return Step::Worked;
    }
    // A rate-generation change relabels; it never pauses (fold R2, kept).
    if journey.departed.clock != now.clock {
        let mut relabelled = journey;
        relabelled.departed = ClockStamp {
            clock: now.clock.clone(),
            at: relabelled.departed.at,
        };
        relabelled.advanced_at = now.clone();
        if let JourneyState::Held { since, .. } = &mut relabelled.state {
            *since = ClockStamp {
                clock: now.clock.clone(),
                at: since.at,
            };
        }
        tracing::info!(journey = %id, "atlas: journey re-anchored onto a new world-IC generation");
        persist(ctx, &relabelled);
        if let Some(st) = state_mut(ctx) {
            st.travel.journeys.insert(id, relabelled);
        }
        return Step::Worked;
    }
    if matches!(journey.state, JourneyState::Held { .. }) {
        return Step::Skipped;
    }
    // The traveler must still exist; the way must still be what it was.
    let gone = ctx
        .w
        .get(journey.traveler.obj())
        .is_none_or(|o| o.flags.contains(Flags::GOING));
    if gone {
        retire(ctx, id);
        return Step::Worked;
    }
    let revision = state_of(ctx).and_then(|st| match &journey.way {
        Way::Connection(key) => st
            .records
            .connections
            .get(key)
            .filter(|c| c.status == ConnectionStatus::Active)
            .map(|c| c.revision),
        Way::Portal(key) => st.records.portals.get(key).map(|p| p.revision),
    });
    match revision {
        Some(r) if r == journey.applied_revision => {}
        Some(_) => return abort(ctx, id, "the way changed under it", None),
        None => return abort(ctx, id, "the way was closed or retired", None),
    }
    let Some(due) = journey.next_due() else {
        return abort(ctx, id, "a leg boundary overflowed", None);
    };
    if due > now.at {
        return Step::Skipped;
    }
    let live = now.at.saturating_sub(due) <= LIVE_GRACE_S.saturating_mul(i64::from(rate_of(ctx.w)));
    let mut advanced = journey;
    advanced.cursor = advanced.cursor.saturating_add(1);
    advanced.advanced_at = now.clone();
    if advanced.at_destination() {
        return apply_arrival(ctx, advanced, live);
    }
    // An intermediate node: a checkpoint. Watched or intercepted, the traveler is held
    // there — in a catch-up too (P6: a stake-out always sees the traveler pass).
    let held_by = hold_reason_at(ctx, &advanced);
    if let Some(reason) = held_by {
        return apply_hold(ctx, advanced, reason, now);
    }
    // A pass: nothing moves; the patch (if any) and a vehicle's passengers hear of it
    // when it is live.
    if live {
        narrate_pass(ctx, &advanced);
    }
    persist(ctx, &advanced);
    if let Some(st) = state_mut(ctx) {
        st.travel.journeys.insert(id, advanced);
    }
    Step::Worked
}

/// Why the traveler is held at the node it just reached, if it is.
fn hold_reason_at(ctx: &Ctx<'_>, j: &JourneyRec) -> Option<HoldReason> {
    let st = state_of(ctx)?;
    let here = j.here()?;
    let obj = j.traveler.obj();
    let in_grace = ctx.w.now() < j.grace_until;
    if let Some(intercept) = st.travel.intercepts.get(&obj)
        && &intercept.at == here
        && !in_grace
    {
        return Some(HoldReason::Intercept { by: intercept.by });
    }
    if let Node::Checkpoint { key } = here
        && let Some(watcher) = watcher_at(st, ctx.w, key)
        && !in_grace
    {
        return Some(HoldReason::Watch { by: watcher });
    }
    None
}

fn narrate_pass(ctx: &mut Ctx<'_>, j: &JourneyRec) {
    let obj = j.traveler.obj();
    let Some(Node::Checkpoint { key }) = j.here().cloned() else {
        return;
    };
    let (patch, cp_name) = {
        let Some(st) = state_of(ctx) else { return };
        (
            st.travel.patches.get(&key).map(|d| d.room),
            st.records
                .checkpoints
                .get(&key)
                .map_or_else(|| "a checkpoint".to_string(), |c| c.name.clone()),
        )
    };
    if let Some(patch) = patch {
        let text = RichText::new()
            .span(name_span(ctx.w, obj))
            .text(" passes by without stopping.");
        tell_room(ctx, patch, obj, text);
    }
    if j.traveler.is_carrier() {
        let driver = state_of(ctx)
            .and_then(|st| match &j.way {
                Way::Connection(k) => st.records.connections.get(k).map(|c| driver_name(ctx, c)),
                Way::Portal(_) => None,
            })
            .unwrap_or_else(|| "The driver".to_string());
        tell_room(
            ctx,
            obj,
            obj,
            RichText::new().text(format!("{driver}, over the PA: \"{cp_name}.\"")),
        );
    } else if let Traveler::Character(who) = j.traveler {
        tell(ctx, who, format!("You pass {cp_name}."));
    }
}

fn apply_hold(
    ctx: &mut Ctx<'_>,
    journey: JourneyRec,
    reason: HoldReason,
    now: &ClockStamp,
) -> Step {
    let id = journey.id;
    let obj = journey.traveler.obj();
    let Some(here) = journey.here().cloned() else {
        return abort(ctx, id, "held nowhere", None);
    };
    let place = match &here {
        Node::Checkpoint { key } => match patch_room(ctx, key) {
            Ok(room) => room,
            Err(error) => return abort(ctx, id, &format!("no patch: {error}"), None),
        },
        Node::Room { room } => *room,
    };
    let held = JourneyRec {
        state: JourneyState::Held {
            by: reason.clone(),
            since: now.clone(),
        },
        ..journey
    };
    let holder = reason.by();
    let where_ = state_of(ctx)
        .map(|st| node_name(&st.records, ctx.w, &here))
        .unwrap_or_else(|| "here".to_string());
    if held.traveler.is_carrier() {
        if let Node::Checkpoint { key } = &here {
            open_doors(ctx, key, obj, place);
        }
        tell_room(
            ctx,
            obj,
            obj,
            RichText::new().text(format!(
                "The {} slows and stops at {where_}: {} is standing in the road. The doors open.",
                display_name(ctx.w, obj),
                display_name(ctx.w, holder)
            )),
        );
    } else {
        let from = ctx.w.get(obj).and_then(|o| o.location);
        if let Err(error) = ctx.w.move_to(obj, place) {
            return abort(
                ctx,
                id,
                &format!("could not be held at {where_}: {error}"),
                None,
            );
        }
        ctx.emit_world_event(&WorldEvent::Moved {
            who: obj,
            from,
            to: place,
        });
        if let Traveler::Character(who) = held.traveler {
            tell(
                ctx,
                who,
                format!("{} stops you at {where_}.", display_name(ctx.w, holder)),
            );
        }
    }
    let text = RichText::new()
        .span(name_span(ctx.w, holder))
        .text(" stops ")
        .span(name_span(ctx.w, obj))
        .text(format!(" at {where_}."));
    tell_room(ctx, place, holder, text);
    // A hold from a watch or intercept is spent once it lands.
    if let Some(st) = state_mut(ctx) {
        st.travel.intercepts.remove(&obj);
    }
    persist(ctx, &held);
    if let Some(st) = state_mut(ctx) {
        st.travel.journeys.insert(id, held);
    }
    Step::Worked
}

fn apply_arrival(ctx: &mut Ctx<'_>, journey: JourneyRec, live: bool) -> Step {
    let id = journey.id;
    let obj = journey.traveler.obj();
    let Some(dest) = journey.destination() else {
        return abort(ctx, id, "no destination room", None);
    };
    if ctx.w.get(dest).is_none_or(|o| o.kind != ObjKind::Room) {
        return abort(ctx, id, "the destination is gone", None);
    }
    if journey.traveler.is_carrier() {
        // The vehicle stands at the far end from now on; its doors there open by
        // themselves (the gate reads the service).
        let (conn_key, driver) = state_of(ctx)
            .and_then(|st| match &journey.way {
                Way::Connection(key) => st
                    .records
                    .connections
                    .get(key)
                    .map(|c| (Some(key.clone()), driver_name(ctx, c))),
                Way::Portal(_) => None,
            })
            .unwrap_or((None, "The driver".to_string()));
        if let Some(key) = conn_key {
            let at = match journey.heading {
                Heading::Forward => End::To,
                Heading::Reverse => End::From,
            };
            let now_real = ctx.w.now();
            set_service(ctx, &key, at, now_real);
        }
        if live {
            let name = display_name(ctx.w, obj);
            let stop = display_name(ctx.w, dest);
            tell_room(
                ctx,
                obj,
                obj,
                RichText::new().text(format!(
                    "{driver}, over the PA: \"{stop}. Mind the step.\" The {name} pulls in and the doors open."
                )),
            );
            tell_room(
                ctx,
                dest,
                obj,
                RichText::new()
                    .span(name_span(ctx.w, obj))
                    .text(" pulls in."),
            );
        }
    } else {
        let from = ctx.w.get(obj).and_then(|o| o.location);
        if let Err(error) = ctx.w.move_to(obj, dest) {
            return abort(ctx, id, &format!("could not arrive: {error}"), None);
        }
        if live {
            let text = RichText::new()
                .span(name_span(ctx.w, obj))
                .text(" arrives.");
            tell_room(ctx, dest, obj, text);
        }
        ctx.emit_world_event(&WorldEvent::Moved {
            who: obj,
            from,
            to: dest,
        });
        if let Traveler::Character(who) = journey.traveler {
            tell(
                ctx,
                who,
                format!("You arrive at {}.", display_name(ctx.w, dest)),
            );
        }
    }
    retire(ctx, id);
    Step::Worked
}

/// End a journey early, leaving the traveler at its last committed node (doc 14 §3.3).
fn abort(ctx: &mut Ctx<'_>, id: JourneyId, why: &str, by: Option<ObjId>) -> Step {
    let Some(journey) = state_of(ctx).and_then(|st| st.travel.journeys.get(&id).cloned()) else {
        return Step::Skipped;
    };
    let obj = journey.traveler.obj();
    let here = journey.here().cloned();
    let where_ = state_of(ctx)
        .and_then(|st| here.as_ref().map(|n| node_name(&st.records, ctx.w, n)))
        .unwrap_or_else(|| "here".to_string());
    tracing::info!(journey = %id, traveler = %obj, why, "atlas: journey ended early");
    if journey.traveler.is_carrier() {
        // A vehicle goes back to the stop it left; passengers are told.
        if let Some(Node::Checkpoint { key }) = &here {
            close_doors(ctx, key, obj);
        }
        if let Way::Connection(key) = &journey.way {
            let at = match journey.heading {
                Heading::Forward => End::From,
                Heading::Reverse => End::To,
            };
            let now_real = ctx.w.now();
            set_service(ctx, key, at, now_real);
        }
        let origin = journey.origin();
        let name = display_name(ctx.w, obj);
        tell_room(
            ctx,
            obj,
            obj,
            RichText::new().text(format!(
                "The {name} turns back ({why}) and pulls in where it started. The doors open."
            )),
        );
        if let Some(origin) = origin {
            tell_room(
                ctx,
                origin,
                obj,
                RichText::new()
                    .span(name_span(ctx.w, obj))
                    .text(" comes back and pulls in."),
            );
        }
    } else {
        // At the origin still (cursor 0): back into the origin room. Further on: the
        // traveler stays where it is — the transit room or the patch it is held at.
        let back = match here {
            Some(Node::Room { room }) if journey.cursor == 0 => Some(room),
            _ => None,
        };
        if let Some(room) = back
            && ctx.w.get(obj).and_then(|o| o.location) != Some(room)
        {
            let from = ctx.w.get(obj).and_then(|o| o.location);
            if ctx.w.move_to(obj, room).is_ok() {
                ctx.emit_world_event(&WorldEvent::Moved {
                    who: obj,
                    from,
                    to: room,
                });
                let text = RichText::new()
                    .span(name_span(ctx.w, obj))
                    .text(" turns back.");
                tell_room(ctx, room, obj, text);
            }
        }
        if journey.cursor > 0
            && let Some(st) = state_mut(ctx)
        {
            st.travel
                .stranded
                .insert(obj, (journey.way.clone(), journey.cursor, journey.heading));
        }
        if let Traveler::Character(who) = journey.traveler {
            let by = by
                .filter(|b| *b != who)
                .map(|b| format!(" by {}", display_name(ctx.w, b)))
                .unwrap_or_default();
            tell(
                ctx,
                who,
                format!("Your journey ends{by} at {where_} ({why})."),
            );
        }
    }
    retire(ctx, id);
    Step::Worked
}

/// Drop a journey's record and index.
fn retire(ctx: &mut Ctx<'_>, id: JourneyId) {
    let obj = state_of(ctx)
        .and_then(|st| st.travel.journeys.get(&id))
        .map(|j| j.traveler.obj());
    del_doc(ctx, journey_key(id));
    if let Some(st) = state_mut(ctx) {
        st.travel.journeys.remove(&id);
        if let Some(obj) = obj {
            if st.travel.by_traveler.get(&obj) == Some(&id) {
                st.travel.by_traveler.remove(&obj);
            }
            st.travel.intercepts.remove(&obj);
        }
    }
}

fn set_service(ctx: &mut Ctx<'_>, conn: &RecordKey, at: End, since_real: i64) {
    let doc = ServiceDoc { at, since_real };
    put_doc(ctx, key_of(KIND_SERVICE, conn), &doc);
    if let Some(st) = state_mut(ctx) {
        let held_by = st.travel.services.get(conn).and_then(|s| s.held_by);
        st.travel
            .services
            .insert(conn.clone(), ServiceState { doc, held_by });
    }
}

// ---------------------------------------------------------------------------
// Hold, release, cancel (commands and staff)
// ---------------------------------------------------------------------------

/// Hold a journey at its current node now (staff, or `+intercept` on a traveler in the
/// interceptor's room).
///
/// # Errors
///
/// A message for the actor.
pub fn hold_now(ctx: &mut Ctx<'_>, id: JourneyId, reason: HoldReason) -> Result<(), String> {
    let journey = state_of(ctx)
        .and_then(|st| st.travel.journeys.get(&id).cloned())
        .ok_or_else(|| "No such journey.".to_string())?;
    if matches!(journey.state, JourneyState::Held { .. }) {
        return Err("It is already stopped.".to_string());
    }
    if !matches!(reason, HoldReason::Staff { .. }) && ctx.w.now() < journey.grace_until {
        return Err("It slips past before you can act — try again in a moment.".to_string());
    }
    let now = ctx.w.world_ic_now();
    match apply_hold(ctx, journey, reason, &now) {
        Step::Worked => Ok(()),
        Step::Skipped => Err("It could not be stopped.".to_string()),
    }
}

/// Release a held journey: the schedule shifts by the hold, the traveler goes back on
/// the road, and no player may re-hold it for [`RESUME_GRACE_S`].
///
/// # Errors
///
/// A message for the actor.
pub fn release(ctx: &mut Ctx<'_>, id: JourneyId, by: Option<ObjId>) -> Result<(), String> {
    let journey = state_of(ctx)
        .and_then(|st| st.travel.journeys.get(&id).cloned())
        .ok_or_else(|| "No such journey.".to_string())?;
    let JourneyState::Held { since, .. } = &journey.state else {
        return Err("It is not stopped.".to_string());
    };
    let obj = journey.traveler.obj();
    let _ = since;
    let now = ctx.w.world_ic_now();
    // Re-base so the node the traveler stands at counts as reached NOW: every leg
    // still to run runs whole from the release, whether the hold began at the due
    // stamp or late in a catch-up.
    let done: i64 = journey
        .legs_ic
        .iter()
        .take(usize::try_from(journey.cursor).unwrap_or(0))
        .map(|l| i64::from(*l))
        .sum();
    let mut resumed = journey.clone();
    resumed.departed = ClockStamp {
        clock: now.clock.clone(),
        at: now.at.saturating_sub(done),
    };
    resumed.advanced_at = now;
    resumed.state = JourneyState::UnderWay;
    resumed.grace_until = ctx.w.now().saturating_add(RESUME_GRACE_S);
    let here = journey.here().cloned();
    let (where_, way_name) = state_of(ctx)
        .map(|st| {
            (
                here.as_ref()
                    .map(|n| node_name(&st.records, ctx.w, n))
                    .unwrap_or_else(|| "here".to_string()),
                match &journey.way {
                    Way::Connection(key) => st.records.connections.get(key).map(|c| c.name.clone()),
                    Way::Portal(_) => None,
                }
                .unwrap_or_else(|| "way".to_string()),
            )
        })
        .unwrap_or_else(|| ("here".to_string(), "way".to_string()));
    let place = ctx.w.get(obj).and_then(|o| o.location);
    if journey.traveler.is_carrier() {
        if let Some(Node::Checkpoint { key }) = &here {
            close_doors(ctx, key, obj);
        }
        let name = display_name(ctx.w, obj);
        tell_room(
            ctx,
            obj,
            obj,
            RichText::new().text(format!(
                "The doors close and the {name} pulls away from {where_}."
            )),
        );
        if let Some(Node::Checkpoint { key }) = &here
            && let Some(patch) = state_of(ctx)
                .and_then(|st| st.travel.patches.get(key))
                .map(|d| d.room)
        {
            tell_room(
                ctx,
                patch,
                obj,
                RichText::new()
                    .span(name_span(ctx.w, obj))
                    .text(" pulls away."),
            );
        }
    } else {
        let transit = transit_room(ctx, &journey.way, &way_name)?;
        if place != Some(transit) {
            ctx.w
                .move_to(obj, transit)
                .map_err(|e| format!("It cannot go on: {e}"))?;
            if let Some(from) = place {
                let text = RichText::new()
                    .span(name_span(ctx.w, obj))
                    .text(" goes on its way.");
                tell_room(ctx, from, obj, text);
            }
            ctx.emit_world_event(&WorldEvent::Moved {
                who: obj,
                from: place,
                to: transit,
            });
        }
        if let Traveler::Character(who) = journey.traveler {
            let by = by
                .map(|b| display_name(ctx.w, b))
                .unwrap_or_else(|| "Staff".to_string());
            tell(ctx, who, format!("{by} lets you go on from {where_}."));
        }
    }
    persist(ctx, &resumed);
    if let Some(st) = state_mut(ctx) {
        st.travel.journeys.insert(id, resumed);
    }
    if let Some(Node::Checkpoint { key }) = here {
        retire_patch_if_idle(ctx, &key);
    }
    Ok(())
}

/// End a journey early by request.
///
/// # Errors
///
/// A message for the actor.
pub fn cancel(
    ctx: &mut Ctx<'_>,
    id: JourneyId,
    why: &str,
    by: Option<ObjId>,
) -> Result<(), String> {
    let here = state_of(ctx)
        .and_then(|st| st.travel.journeys.get(&id))
        .map(|j| j.here().cloned());
    match abort(ctx, id, why, by) {
        Step::Worked => {
            if let Some(Some(Node::Checkpoint { key })) = here {
                retire_patch_if_idle(ctx, &key);
            }
            Ok(())
        }
        Step::Skipped => Err("No such journey.".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Services: the vehicle loop
// ---------------------------------------------------------------------------

fn run_services(ctx: &mut Ctx<'_>) {
    let now_real = ctx.w.now();
    let mut first_sight: Vec<RecordKey> = Vec::new();
    let due: Vec<(RecordKey, ObjId, Heading, String, Route)> = {
        let Some(st) = state_of(ctx) else { return };
        let mut due = Vec::new();
        for conn in st.records.connections.values() {
            let Some(service) = &conn.service else {
                continue;
            };
            if conn.status != ConnectionStatus::Active || conn.direction != Direction::Both {
                continue;
            }
            let Some(vehicle) = service_vehicle(ctx.w, conn) else {
                continue;
            };
            if st.travel.by_traveler.contains_key(&vehicle) {
                continue;
            }
            let Some(state) = st.travel.services.get(&conn.key) else {
                first_sight.push(conn.key.clone());
                continue;
            };
            if state.held_by.is_some()
                || now_real.saturating_sub(state.doc.since_real) < i64::from(service.dwell_s)
            {
                continue;
            }
            let heading = match state.doc.at {
                End::From => Heading::Forward,
                End::To => Heading::Reverse,
            };
            let Some(mode) = conn.modes.first().cloned() else {
                continue;
            };
            match resolve_connection(&st.records, conn, heading) {
                Ok(route) => due.push((conn.key.clone(), vehicle, heading, mode, route)),
                Err(error) => {
                    tracing::debug!(connection = %conn.name, %error, "atlas: service not ready");
                }
            }
        }
        due
    };
    for key in first_sight {
        // First sight of this service: its vehicle stands at the first end from now.
        set_service(ctx, &key, End::From, now_real);
    }
    for (key, vehicle, heading, mode, route) in due {
        if let Err(error) = depart(
            ctx,
            Traveler::Carrier(vehicle),
            Way::Connection(key.clone()),
            heading,
            &mode,
            route,
        ) {
            tracing::warn!(connection = %key, %error, "atlas: a service vehicle could not depart");
            // Try again after another dwell rather than every second.
            let at = if heading == Heading::Forward {
                End::From
            } else {
                End::To
            };
            set_service(ctx, &key, at, now_real);
        }
    }
}

// ---------------------------------------------------------------------------
// Timed portals: the hook queues, the tick departs
// ---------------------------------------------------------------------------

fn drain_pending(ctx: &mut Ctx<'_>, pending: &Pending) {
    let queued: Vec<PendingDeparture> = std::mem::take(&mut *lock(pending));
    for p in queued {
        let Some((portal, mode)) = state_of(ctx).and_then(|st| {
            let portal = st
                .records
                .portals
                .values()
                .find(|p_| p_.exits.contains(&p.exit))?
                .clone();
            let mode = portal_mode(&portal)?;
            Some((portal, mode))
        }) else {
            continue;
        };
        let src = ctx.w.get(p.exit).and_then(|o| o.location);
        if ctx.w.get(p.actor).and_then(|o| o.location) != src {
            continue;
        }
        let route = match resolve_portal(ctx.w, &portal, p.exit, &mode) {
            Ok(route) => route,
            Err(error) => {
                tell(ctx, p.actor, error);
                continue;
            }
        };
        let traveler = match ctx.w.get(p.actor).map(|o| o.kind) {
            Some(ObjKind::Player | ObjKind::Npc) => Traveler::Character(p.actor),
            _ => Traveler::Object(p.actor),
        };
        if let Err(error) = depart(
            ctx,
            traveler,
            Way::Portal(portal.key.clone()),
            Heading::Forward,
            &mode,
            route,
        ) {
            tell(ctx, p.actor, error);
        }
    }
}

// ---------------------------------------------------------------------------
// Patches
// ---------------------------------------------------------------------------

fn sweep_patches(ctx: &mut Ctx<'_>) {
    // Watchers who left their patch (walked, teleported, disconnected and swept) end
    // their watch; an idle patch goes.
    let stale: Vec<ObjId> = state_of(ctx)
        .map(|st| {
            st.travel
                .watches
                .iter()
                .filter(|(who, w)| {
                    let patch = st.travel.patches.get(&w.checkpoint).map(|d| d.room);
                    ctx.w.get(**who).and_then(|o| o.location) != patch
                        || ctx
                            .w
                            .get(**who)
                            .is_none_or(|o| o.flags.contains(Flags::GOING))
                })
                .map(|(who, _)| *who)
                .collect()
        })
        .unwrap_or_default();
    for who in stale {
        let _ = end_watch(ctx, who, false);
    }
    // A hold that a stake-out made lifts when nobody is left at the checkpoint,
    // whoever made it: a watcher who walked, teleported or was swept away.
    let unwatched: Vec<JourneyId> = state_of(ctx)
        .map(|st| {
            st.travel
                .journeys
                .values()
                .filter(|j| {
                    matches!(&j.state, JourneyState::Held { by: HoldReason::Watch { .. }, .. })
                        && matches!(j.here(), Some(Node::Checkpoint { key }) if watcher_at(st, ctx.w, key).is_none())
                })
                .map(|j| j.id)
                .collect()
        })
        .unwrap_or_default();
    for id in unwatched {
        if let Err(error) = release(ctx, id, None) {
            tracing::debug!(journey = %id, %error, "atlas: an unwatched hold stayed");
        }
    }
    let idle: Vec<RecordKey> = state_of(ctx)
        .map(|st| st.travel.patches.keys().cloned().collect())
        .unwrap_or_default();
    for key in idle {
        retire_patch_if_idle(ctx, &key);
    }
}

/// End `who`'s watch; `walk_back` moves them to where they came from.
///
/// # Errors
///
/// A message for the actor.
pub fn end_watch(ctx: &mut Ctx<'_>, who: ObjId, walk_back: bool) -> Result<(), String> {
    let registered = state_mut(ctx).and_then(|st| st.travel.watches.remove(&who));
    let watch = match registered {
        Some(w) => w,
        None => {
            // Standing in a patch without having staked it out: a passenger who
            // stepped off, or a watcher from before a restart.
            let here = ctx.w.get(who).and_then(|o| o.location);
            let checkpoint = state_of(ctx)
                .and_then(|st| here.and_then(|r| st.travel.patch_at(r)).cloned())
                .ok_or_else(|| "You are not staking anything out.".to_string())?;
            Watch {
                checkpoint,
                from: None,
            }
        }
    };
    let patch = state_of(ctx)
        .and_then(|st| st.travel.patches.get(&watch.checkpoint))
        .map(|d| d.room);
    // Travelers this watcher held, and no other watcher still holds, go on.
    let held: Vec<JourneyId> = state_of(ctx)
        .map(|st| {
            st.travel
                .journeys
                .values()
                .filter(|j| {
                    matches!(&j.state, JourneyState::Held { by: HoldReason::Watch { .. }, .. })
                        && matches!(j.here(), Some(Node::Checkpoint { key }) if *key == watch.checkpoint)
                        && watcher_at_except(st, ctx.w, &watch.checkpoint, Some(who)).is_none()
                })
                .map(|j| j.id)
                .collect()
        })
        .unwrap_or_default();
    for id in held {
        if let Err(error) = release(ctx, id, Some(who)) {
            tracing::debug!(journey = %id, %error, "atlas: a held traveler stayed after its watcher left");
        }
    }
    if walk_back
        && let Some(patch) = patch
        && ctx.w.get(who).and_then(|o| o.location) == Some(patch)
    {
        let back = state_of(ctx)
            .map(|st| way_back(st, ctx.w, &watch.checkpoint, watch.from))
            .unwrap_or(mush_core::LIMBO);
        if ctx.w.move_to(who, back).is_ok() {
            let text = RichText::new()
                .span(name_span(ctx.w, who))
                .text(" gives up the watch and heads back.");
            tell_room(ctx, patch, who, text);
            let text = RichText::new()
                .span(name_span(ctx.w, who))
                .text(" comes back from the road.");
            tell_room(ctx, back, who, text);
            ctx.emit_world_event(&WorldEvent::Moved {
                who,
                from: Some(patch),
                to: back,
            });
        }
    }
    retire_patch_if_idle(ctx, &watch.checkpoint);
    Ok(())
}

/// Begin a stake-out at `checkpoint`: the patch is made, the watcher moved there.
///
/// # Errors
///
/// A message for the actor.
pub fn begin_watch(ctx: &mut Ctx<'_>, who: ObjId, checkpoint: &RecordKey) -> Result<ObjId, String> {
    let count = state_of(ctx).map_or(0, |st| st.travel.watches.len());
    if count >= MAX_WATCHES {
        return Err("Too many stake-outs are running; try again later.".to_string());
    }
    if state_of(ctx).is_some_and(|st| st.travel.watches.contains_key(&who)) {
        return Err("You are already staking a checkpoint out (+stakeout/end first).".to_string());
    }
    let from = ctx.w.get(who).and_then(|o| o.location);
    let patch = patch_room(ctx, checkpoint)?;
    ctx.w
        .move_to(who, patch)
        .map_err(|e| format!("You cannot get there: {e}"))?;
    if let Some(from) = from {
        let cp = display_name(ctx.w, patch);
        let text = RichText::new()
            .span(name_span(ctx.w, who))
            .text(format!(" heads out to {cp}."));
        tell_room(ctx, from, who, text);
    }
    let text = RichText::new()
        .span(name_span(ctx.w, who))
        .text(" arrives and settles in to watch the road.");
    tell_room(ctx, patch, who, text);
    ctx.emit_world_event(&WorldEvent::Moved {
        who,
        from,
        to: patch,
    });
    if let Some(st) = state_mut(ctx) {
        st.travel.watches.insert(
            who,
            Watch {
                checkpoint: checkpoint.clone(),
                from,
            },
        );
    }
    Ok(patch)
}

// ---------------------------------------------------------------------------
// Known places
// ---------------------------------------------------------------------------

/// Record that `who` knows `place`; persists when it changed.
pub fn learn(ctx: &mut Ctx<'_>, who: ObjId, place: &RecordKey) {
    let changed = match state_mut(ctx) {
        Some(st) => {
            let places = st.travel.known.entry(who).or_default();
            if places.contains(place) {
                false
            } else {
                places.push(place.clone());
                while places.len() > MAX_KNOWN_PLACES {
                    places.remove(0);
                }
                true
            }
        }
        None => false,
    };
    if changed {
        let doc = KnownDoc {
            places: state_of(ctx)
                .and_then(|st| st.travel.known.get(&who).cloned())
                .unwrap_or_default(),
        };
        put_doc(ctx, known_key(who), &doc);
    }
}

// ---------------------------------------------------------------------------
// Observers: moves, disconnects, destroys
// ---------------------------------------------------------------------------

/// `Moved`: a character entering an authored room learns its neighborhood.
pub(crate) fn on_moved(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::Moved { who, to, .. } = ev else {
        return;
    };
    let is_player = ctx.w.get(*who).is_some_and(|o| o.kind == ObjKind::Player);
    if !is_player {
        return;
    }
    let place = state_of(ctx)
        .and_then(|st| st.records.roomgeo.get(to))
        .map(|g| g.neighborhood.clone());
    if let Some(place) = place {
        learn(ctx, *who, &place);
    }
}

/// `Disconnected`: a character's own journey ends where it is (doc 14 §3.3).
pub(crate) fn on_disconnected(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::Disconnected { who } = ev else {
        return;
    };
    let id = state_of(ctx).and_then(|st| {
        st.travel
            .journey_of(*who)
            .filter(|j| matches!(j.traveler, Traveler::Character(_)))
            .map(|j| j.id)
    });
    if let Some(id) = id {
        let _ = cancel(ctx, id, "you disconnected", None);
    }
    if state_of(ctx).is_some_and(|st| st.travel.watches.contains_key(who)) {
        let _ = end_watch(ctx, *who, false);
    }
}

/// `ObjDestroyed`: a destroyed traveler ends its journey; a destroyed travel room is
/// forgotten.
pub(crate) fn on_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let id = state_of(ctx).and_then(|st| st.travel.journey_of(*obj).map(|j| j.id));
    if let Some(id) = id {
        retire(ctx, id);
    }
    let transit: Vec<RecordKey> = state_of(ctx)
        .map(|st| {
            st.travel
                .transit
                .iter()
                .filter(|(_, d)| d.room == *obj)
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default();
    for key in transit {
        del_doc(ctx, key_of(KIND_TRANSIT, &key));
        if let Some(st) = state_mut(ctx) {
            st.travel.transit.remove(&key);
        }
    }
    let patches: Vec<RecordKey> = state_of(ctx)
        .map(|st| {
            st.travel
                .patches
                .iter()
                .filter(|(_, d)| d.room == *obj)
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default();
    for key in patches {
        del_doc(ctx, key_of(KIND_PATCH, &key));
        if let Some(st) = state_mut(ctx) {
            st.travel.patches.remove(&key);
            st.travel.watches.retain(|_, w| w.checkpoint != key);
        }
    }
    if let Some(st) = state_mut(ctx) {
        st.travel.watches.remove(obj);
        st.travel.intercepts.remove(obj);
        st.travel.intercepts.retain(|_, i| i.by != *obj);
    }
}

// ---------------------------------------------------------------------------
// Hooks: the movement gate, the transit look, the transit projection
// ---------------------------------------------------------------------------

/// The atlas's `MoveAttempt`, `LookRendered` and `Perceive` hook.
pub struct TravelHook {
    /// The queue to the tick.
    pub pending: Pending,
}

impl EventHook for TravelHook {
    fn move_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        let w = ctx.world();
        let Some(st) = w.module_state::<AtlasState>() else {
            return Ok(());
        };
        // A traveler under way or held moves only through the journey.
        if let Some(j) = st.travel.journey_of(actor)
            && !j.traveler.is_carrier()
        {
            return Err(RichText::tag(
                "error",
                "You are on your way; +travel/cancel to stop here, or wait to be let go.",
            ));
        }
        let here = w.get(actor).and_then(|o| o.location);
        if let Some(exit) = exit {
            // A vehicle's doors open only at its stop.
            if w.get(dest).is_some_and(|o| o.kind == ObjKind::Room)
                && vehicle_connection(st, w, dest).is_some()
                && vehicle_stop(st, w, dest) != here
            {
                return Err(RichText::tag(
                    "error",
                    format!("The {} is not here.", display_name(w, dest)),
                ));
            }
            if let Some(vehicle) = here
                && vehicle_connection(st, w, vehicle).is_some()
                && vehicle_stop(st, w, vehicle) != Some(dest)
            {
                return Err(RichText::tag(
                    "error",
                    format!(
                        "The {} is moving; wait for the next stop.",
                        display_name(w, vehicle)
                    ),
                ));
            }
            // A timed portal is a journey, not a step.
            if let Some(portal) = st
                .records
                .portals
                .values()
                .find(|p| p.exits.contains(&exit))
                && matches!(portal.time, TimePolicy::Timed { .. })
            {
                let Some(mode) = portal_mode(portal) else {
                    return Ok(());
                };
                let secs = i64::from(portal_base_s(portal, &mode));
                let rate = i64::from(rate_of(w).max(1));
                let real = secs.saturating_add(rate - 1) / rate;
                let name = w
                    .get(exit)
                    .map(|o| exit_name(&o.name))
                    .unwrap_or_else(|| "that way".to_string());
                let mut queue = lock(&self.pending);
                if queue.iter().any(|p| p.actor == actor) {
                    return Err(RichText::tag("error", "You are already setting off."));
                }
                if queue.len() >= MAX_PENDING_DEPARTURES {
                    return Err(RichText::tag(
                        "error",
                        "Too many people are setting off at once; try again in a moment.",
                    ));
                }
                queue.push(PendingDeparture { actor, exit });
                return Err(RichText::tag(
                    crate::module::NS,
                    format!(
                        "You set off along {name} by {mode}: it is {} away. (+travel/status \
                         says how far there is to go; +travel/cancel turns back.)",
                        about(real)
                    ),
                ));
            }
        }
        Ok(())
    }

    fn look_rendered(
        &self,
        ctx: &HookReadContext<'_>,
        looker: ObjId,
        room: ObjId,
    ) -> Vec<RichText> {
        let w = ctx.world();
        let Some(st) = w.module_state::<AtlasState>() else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        if let Some(j) = st.travel.journey_of(looker)
            && !j.traveler.is_carrier()
            && st.travel.is_transit(room)
        {
            lines.push(RichText::new().text(position_line(st, w, j)));
        }
        if let Some(j) = st.travel.journey_of(room)
            && j.traveler.is_carrier()
        {
            lines.push(RichText::new().text(position_line(st, w, j)));
        }
        lines
    }

    fn perceives(&self, ctx: &HookReadContext<'_>, observer: ObjId, subject: ObjId) -> Perceive {
        let w = ctx.world();
        let Some(st) = w.module_state::<AtlasState>() else {
            return Perceive::Abstain;
        };
        // Two travelers in one transit room at different points of the road do not see
        // each other: the room is a projection of progress, not a place (P1).
        let (Some(a), Some(b)) = (
            st.travel.journey_of(observer),
            st.travel.journey_of(subject),
        ) else {
            return Perceive::Abstain;
        };
        if a.traveler.is_carrier() || b.traveler.is_carrier() {
            return Perceive::Abstain;
        }
        let same_room = w.get(observer).and_then(|o| o.location).is_some_and(|r| {
            st.travel.is_transit(r) && w.get(subject).and_then(|o| o.location) == Some(r)
        });
        if same_room && (a.way != b.way || a.heading != b.heading || a.cursor != b.cursor) {
            return Perceive::Conceal;
        }
        Perceive::Abstain
    }
}

/// One line saying where a journey has got to — for its traveler and for staff.
#[must_use]
pub fn position_line(st: &AtlasState, w: &World, j: &JourneyRec) -> String {
    let name = match &j.way {
        Way::Connection(key) => st.records.connections.get(key).map(|c| c.name.clone()),
        Way::Portal(_) => None,
    }
    .unwrap_or_else(|| "the way".to_string());
    let here = j
        .here()
        .map(|n| node_name(&st.records, w, n))
        .unwrap_or_else(|| "nowhere".to_string());
    match (&j.state, j.ahead()) {
        (JourneyState::Held { by, .. }, _) => format!(
            "{} is stopped at {here} by {}.",
            display_name(w, j.traveler.obj()),
            display_name(w, by.by())
        ),
        (_, Some(ahead)) => {
            let eta = j
                .next_due()
                .map_or_else(|| "a while".to_string(), |at| about(real_until(w, at)));
            format!(
                "On the {name}, past {here}; {} is {eta} off.",
                node_name(&st.records, w, ahead)
            )
        }
        (_, None) => format!("At the end of the {name}."),
    }
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

/// Hydrate the travel documents into `travel`, dropping loudly what will not load or
/// no longer fits the world. Returns the keys to delete.
pub(crate) fn hydrate(
    kv: &[(SmolStr, serde_json::Value)],
    w: &World,
    records: &Records,
) -> (TravelState, Vec<SmolStr>) {
    let mut travel = TravelState::default();
    let mut dropped: Vec<SmolStr> = Vec::new();
    for (key, value) in kv {
        if key.as_str() == META_KEY {
            match serde_json::from_value::<MetaDoc>(value.clone()) {
                Ok(meta) => {
                    travel.next_id = meta.next_id;
                    travel.rotation = meta.rotation;
                }
                Err(error) => {
                    tracing::warn!(%key, %error, "atlas: travel meta would not load; reset");
                }
            }
            continue;
        }
        let Some((kind, rest)) = key.split_once(':') else {
            continue;
        };
        let outcome: Result<(), String> = match kind {
            KIND_JOURNEY => serde_json::from_value::<JourneyRec>(value.clone())
                .map_err(|e| e.to_string())
                .and_then(|j| {
                    j.validate()?;
                    if journey_key(j.id) != *key {
                        return Err("the document's key disagrees with its id".to_string());
                    }
                    let obj = j.traveler.obj();
                    let live = w.get(obj).is_some_and(|o| !o.flags.contains(Flags::GOING));
                    if !live {
                        return Err("its traveler is gone".to_string());
                    }
                    if !j.state.is_live() {
                        return Err("a terminal record".to_string());
                    }
                    if travel.by_traveler.contains_key(&obj) {
                        return Err("its traveler already has a journey".to_string());
                    }
                    let known = match &j.way {
                        Way::Connection(k) => records.connections.contains_key(k),
                        Way::Portal(k) => records.portals.contains_key(k),
                    };
                    if !known {
                        return Err("its way is gone".to_string());
                    }
                    travel.next_id = travel.next_id.max(j.id.0.saturating_add(1));
                    travel.by_traveler.insert(obj, j.id);
                    travel.journeys.insert(j.id, j);
                    Ok(())
                }),
            KIND_KNOWN => serde_json::from_value::<KnownDoc>(value.clone())
                .map_err(|e| e.to_string())
                .and_then(|doc| {
                    let who = rest.parse::<u32>().map(ObjId).map_err(|e| e.to_string())?;
                    let mut places: Vec<RecordKey> = Vec::new();
                    let mut seen = BTreeSet::new();
                    for p in doc.places.into_iter().take(MAX_KNOWN_PLACES) {
                        if seen.insert(p.clone()) {
                            places.push(p);
                        }
                    }
                    travel.known.insert(who, places);
                    Ok(())
                }),
            KIND_TRANSIT | KIND_PATCH => serde_json::from_value::<RoomDoc>(value.clone())
                .map_err(|e| e.to_string())
                .and_then(|doc| {
                    if w.get(doc.room).is_none_or(|o| o.kind != ObjKind::Room) {
                        return Err("its room is gone".to_string());
                    }
                    let record_key = parse_record_key(rest)?;
                    if kind == KIND_TRANSIT {
                        travel.transit.insert(record_key, doc);
                    } else {
                        travel.patches.insert(record_key, doc);
                    }
                    Ok(())
                }),
            KIND_SERVICE => serde_json::from_value::<ServiceDoc>(value.clone())
                .map_err(|e| e.to_string())
                .and_then(|doc| {
                    let record_key = parse_record_key(rest)?;
                    if !records.connections.contains_key(&record_key) {
                        return Err("its connection is gone".to_string());
                    }
                    travel
                        .services
                        .insert(record_key, ServiceState { doc, held_by: None });
                    Ok(())
                }),
            _ => continue,
        };
        if let Err(error) = outcome {
            tracing::warn!(%key, %error, "atlas: a travel document was dropped at boot");
            dropped.push(key.clone());
        }
    }
    // Watchers are whoever is standing in a patch: a watch is not persisted, the
    // person is.
    for (checkpoint, doc) in &travel.patches {
        for who in w.contents_of(doc.room) {
            if w.get(*who).is_some_and(|o| o.kind == ObjKind::Player)
                && !travel.by_traveler.contains_key(who)
            {
                travel.watches.insert(
                    *who,
                    Watch {
                        checkpoint: checkpoint.clone(),
                        from: None,
                    },
                );
            }
        }
    }
    (travel, dropped)
}

fn parse_record_key(rest: &str) -> Result<RecordKey, String> {
    let (pack, entry) = rest
        .split_once(':')
        .ok_or_else(|| "a record key is <pack>:<entry>".to_string())?;
    Ok(RecordKey::new(
        mush_core::CanonicalId::new(pack).map_err(|e| e.to_string())?,
        mush_core::CanonicalId::new(entry).map_err(|e| e.to_string())?,
    ))
}

/// Whether a record's kind prefix is one of the travel documents.
#[must_use]
pub fn is_travel_key(key: &str) -> bool {
    key == META_KEY
        || key.split_once(':').is_some_and(|(kind, _)| {
            matches!(
                kind,
                KIND_JOURNEY | KIND_KNOWN | KIND_TRANSIT | KIND_PATCH | KIND_SERVICE
            )
        })
}

// ---------------------------------------------------------------------------
// What `+travel` may list
// ---------------------------------------------------------------------------

/// One destination a character may set out for from where they stand.
#[derive(Clone, Debug)]
pub struct TravelOption {
    /// The connection.
    pub connection: RecordKey,
    /// Which way along it.
    pub heading: Heading,
    /// The far neighborhood.
    pub place: RecordKey,
    /// The far neighborhood's name.
    pub place_name: String,
    /// The connection's name.
    pub way_name: String,
    /// The modes the character may take.
    pub modes: Vec<String>,
    /// The room the way leaves from.
    pub leaves_from: ObjId,
    /// The route, forward-resolved.
    pub route: Route,
}

/// The destinations `who` may travel to from `room`: every active connection out of
/// the room's neighborhood whose far side is public or known, by a mode `who` has.
#[must_use]
pub fn options_from(st: &AtlasState, who: ObjId, room: ObjId, staff: bool) -> Vec<TravelOption> {
    let Some(nb) = st.records.neighborhood_of_room(room) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for conn in st.records.connections_in(&nb.region) {
        if conn.status != ConnectionStatus::Active {
            continue;
        }
        let (heading, far) = if conn.endpoints.0.neighborhood == nb.key.pack {
            (Heading::Forward, &conn.endpoints.1)
        } else if conn.endpoints.1.neighborhood == nb.key.pack {
            if conn.direction == Direction::OneWay {
                continue;
            }
            (Heading::Reverse, &conn.endpoints.0)
        } else {
            continue;
        };
        let Some(far_nb) = st
            .records
            .neighborhoods
            .values()
            .find(|n| n.key.pack == far.neighborhood)
        else {
            continue;
        };
        let visible =
            staff || conn.visibility == Visibility::Public || st.travel.knows(who, &far_nb.key);
        if !visible {
            continue;
        }
        let modes: Vec<String> = conn
            .modes
            .iter()
            .filter(|m| SELF_MODES.contains(&m.as_str()))
            .cloned()
            .collect();
        if modes.is_empty() {
            continue;
        }
        let Ok(route) = resolve_connection(&st.records, conn, heading) else {
            continue;
        };
        let Some(leaves_from) = route.nodes.first().and_then(|n| match n {
            Node::Room { room } => Some(*room),
            Node::Checkpoint { .. } => None,
        }) else {
            continue;
        };
        out.push(TravelOption {
            connection: conn.key.clone(),
            heading,
            place: far_nb.key.clone(),
            place_name: far_nb.name.clone(),
            way_name: conn.name.clone(),
            modes,
            leaves_from,
            route,
        });
    }
    out
}

/// The real seconds a route takes by `mode` at the current rate and limits.
#[must_use]
pub fn route_real_s(st: &AtlasState, w: &World, route: &Route, mode: &str) -> i64 {
    let legs = plan_legs(
        &route.base_ic,
        rate_of(w),
        st.limits.dwell_floor_s,
        st.limits.real_cap_s.get(mode).copied(),
    );
    let ic: i64 = legs.iter().map(|l| i64::from(*l)).sum();
    let rate = i64::from(rate_of(w).max(1));
    ic.saturating_add(rate - 1) / rate
}

/// `about …` for a duration in real seconds.
#[must_use]
pub fn about_real(secs: i64) -> String {
    about(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_legs_scales_to_the_cap_and_lifts_to_the_floor() {
        // Four hours of IC at rate 1 is 14 400 real seconds; the cap holds it to 180,
        // and every checkpoint dwell (legs after the first) is at least the floor.
        let legs = plan_legs(&[3_600, 3_600, 3_600, 3_600], 1, 30, Some(180));
        let total: u32 = legs.iter().sum();
        assert_eq!(legs.len(), 4);
        assert!(total <= 180, "{legs:?}");
        assert!(legs.iter().skip(1).all(|l| *l >= 30), "{legs:?}");
    }

    #[test]
    fn plan_legs_at_rate_four_arrives_sooner_but_never_inside_the_floor() {
        let slow = plan_legs(&[120, 180, 120], 1, 30, Some(180));
        let fast = plan_legs(&[120, 180, 120], 4, 30, Some(180));
        let real = |legs: &[u32], rate: u32| -> u32 { legs.iter().sum::<u32>().div_ceil(rate) };
        assert!(real(&fast, 4) < real(&slow, 1), "{fast:?} vs {slow:?}");
        // At rate 4 a 120 s leg is 30 real seconds: exactly the floor, never under it.
        assert!(fast.iter().skip(1).all(|l| *l >= 30 * 4), "{fast:?}");
    }

    #[test]
    fn plan_legs_never_yields_a_zero_leg() {
        assert!(plan_legs(&[0, 0], 0, 0, None).iter().all(|l| *l >= 1));
    }

    #[test]
    fn a_journey_record_re_checks_its_invariants() -> Result<(), String> {
        let stamp = ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 10,
        };
        let key = RecordKey::new(
            mush_core::CanonicalId::new("p").map_err(|e| e.to_string())?,
            mush_core::CanonicalId::new("e").map_err(|e| e.to_string())?,
        );
        let ok = JourneyRec {
            id: JourneyId(1),
            traveler: Traveler::Character(ObjId(5)),
            way: Way::Portal(key),
            mode: "foot".into(),
            heading: Heading::Forward,
            nodes: vec![Node::Room { room: ObjId(1) }, Node::Room { room: ObjId(2) }],
            legs_ic: vec![30],
            departed: stamp.clone(),
            advanced_at: stamp,
            cursor: 0,
            state: JourneyState::UnderWay,
            grace_until: 0,
            applied_revision: 1,
            anomaly: None,
        };
        ok.validate()?;
        assert_eq!(ok.arrival_at(), Some(40));
        let mut past = ok.clone();
        past.cursor = 2;
        assert!(past.validate().is_err());
        let mut legs = ok;
        legs.legs_ic.push(1);
        assert!(legs.validate().is_err());
        Ok(())
    }
}
