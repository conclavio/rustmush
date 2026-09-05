//! # Journeys — the mobility / route-interception primitive (design 07 §9)
//!
//! A **plain object** (a WoD black car, a courier) follows an ordered path of
//! ordinary rooms over a `Coarse` sweep: it **never teleports**, it is
//! **legible** to anyone co-present as it passes, and a third party can
//! **intercept** it. This is an independent module clock that relocates plain
//! objects room-to-room — it is NOT `DomainClock` (still Passive-only) and is
//! NOT a placement-graph citizen (it never touches `SpatialState`).
//!
//! Time is in **seconds** (the only clock a tick can read is `world.now()`,
//! unix seconds). A leg's `travel_s` is both its travel time and the dwell in
//! the room it reaches — the window in which a co-present observer can waylay.
//!
//! The load-bearing invariant (doc 07 §9), scoped to LIVE advancement: while
//! the server runs at wall-clock pace a passage visits and narrates every room
//! in order (never a skip, never instantaneous relocation). A catch-up after
//! downtime is not a skipped *opportunity* — no live observer existed — so the
//! sweep advances the cursor to the object's correct current position in ONE
//! move and suppresses the stale intervening narration. That single-move design
//! also bounds a catch-up to one relocation per journey per sweep (no storm).

use std::collections::BTreeMap;

use mush_core::{
    Ctx, Deferred, Flags, ModuleOp, ModuleWrite, ObjId, WorldEvent, output::name_span,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The sweep cadence (seconds). A journey's own absolute due-times are the
/// authority; this only sets how often we check and the interception latency.
pub const SWEEP_SECS: u64 = 1;
/// The registered tick tag.
pub const SWEEP_TAG: &str = "route.sweep";
/// An arrival whose due-time is within this many seconds of now is LIVE
/// (narrated); older arrivals are catch-up (silent).
const LIVE_GRACE_S: i64 = 2;
/// After a resume, the journey is immune to re-interception this long
/// (anti-grief: the owner gets it moving before it can be re-waylaid).
const RESUME_GRACE_S: i64 = 5;
/// Default per-leg travel time when a route token gives no explicit seconds.
pub const DEFAULT_TRAVEL_S: u32 = 30;

/// One leg of a route: travel `travel_s` seconds, then arrive at room `to`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leg {
    /// The room reached at the end of this leg.
    pub to: ObjId,
    /// Seconds to traverse it (>= 1; also the dwell/interception window).
    pub travel_s: u32,
}

/// A journey's lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JourneyStatus {
    /// Advancing on the sweep.
    Traveling,
    /// Held by an interception (or staff); resume shifts the schedule.
    Paused,
    /// Reached the last leg; the record is being retired.
    Arrived,
    /// Ended early (destroyed traveler/room, staff abort); being retired.
    Aborted,
}

/// A scheduled route a plain object is following (`journey:<traveler>`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Journey {
    /// The moving object.
    pub traveler: ObjId,
    /// The ordered legs.
    pub legs: Vec<Leg>,
    /// Legs already reached (`0..=legs.len()`); the traveler is at
    /// `legs[cursor-1].to` (or its origin when 0).
    pub cursor: usize,
    /// Absolute start (unix seconds); `due(k) = started_at + sum(travel_s[0..=k])`.
    pub started_at: i64,
    /// Lifecycle.
    pub status: JourneyStatus,
    /// When it was paused (to shift the schedule on resume).
    pub paused_at: Option<i64>,
    /// No interception before this instant (unix seconds).
    pub grace_until: i64,
    /// The final destination (`legs.last().to`) — reserved for a typed
    /// `KnownPlaceRef` later; never revealed to observers in cut 1.
    pub destination: ObjId,
    /// Who authorized the route (staff).
    pub by: ObjId,
}

impl Journey {
    /// The absolute due-time to have reached `legs[k]`.
    #[must_use]
    fn due(&self, k: usize) -> i64 {
        self.started_at
            + self.legs[..=k]
                .iter()
                .map(|l| i64::from(l.travel_s))
                .sum::<i64>()
    }
    /// The absolute due-time of the next unreached leg, if any.
    #[must_use]
    pub fn next_due(&self) -> Option<i64> {
        (self.cursor < self.legs.len()).then(|| self.due(self.cursor))
    }
}

/// The module's journey slot. Never persisted as a whole (each journey is one
/// doc); rebuilt at boot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RouteState {
    /// Journeys by traveler.
    pub journeys: BTreeMap<ObjId, Journey>,
}

impl RouteState {
    /// Whether `obj` is on a live (Traveling/Paused) journey.
    #[must_use]
    pub fn is_traveling(&self, obj: ObjId) -> bool {
        self.journeys
            .get(&obj)
            .is_some_and(|j| matches!(j.status, JourneyStatus::Traveling | JourneyStatus::Paused))
    }
}

// ---- E7 cut 1: the route DEFINITION (G1, D-E7-29, 10b §3.1) -----------
//
// `Leg`/`Journey`/`RouteState` above are the *runtime*: an in-flight passage and
// where it has got to. What the tree had no type for at all is the **definition** a
// passage is planned from — 10b's G1, re-confirmed by symbol at `a1934a7`.
//
// **Why the type is here and not in `ruleset-tacsim`** (10b §3.1 correction (a),
// OPEN 2). 10b §3.1 and §4.1 put `RouteDef` in cut 1's ruleset crate and §3.6 makes
// `mush-sim` (cut 6) its consumer. `mush-sim -> {core, economy, proto}` and every
// ruleset depends on `mush-sim`, so `mush-sim -> ruleset-tacsim` is a cycle and the
// cut as drafted is unbuildable. The split is `mush-tactics/src/mobility.rs`'s
// stated pattern instead — **the crate owns the state machine, the ruleset provides
// the data**: the type, its caps and its one pure evaluator live here, adding **no
// crate edge** (`CadenceId` already reaches this crate through `mush-economy`), and
// the corridor's actual routes are authored in `ruleset-tacsim`'s rules pack.
//
// Cut 1 changes nothing above this line: `Leg`, `Journey`, `RouteState` and
// `route_create` are untouched, and cut 6 is what makes journeys consume a
// `RouteDef`.

/// The most legs one [`RouteDef`] may declare.
///
/// Re-checked by the `TryFrom` wire, so a stored or pack-authored definition past it
/// is refused on load rather than truncated (10b §4 rule 1).
pub const MAX_LEGS: usize = 32;

/// The declared cadence a route's alignment band is measured on.
///
/// Re-exported so a ruleset can name the field without taking an economy edge of its
/// own; `mush-sim` already depends on `mush-economy` for [`crate::custody`].
pub use mush_economy::CadenceId;

/// One leg of a route **definition** — the planned counterpart of [`Leg`].
///
/// `base_s` is the unladen traversal time at nominal thrust (1000 permille). The
/// laden, drive-adjusted time is [`route_duration`]'s to compute; nothing here is a
/// wall-clock or IC instant, because a definition that carried one could not be
/// pinned (T1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "LegDefWire")]
pub struct LegDef {
    /// The place reached at the end of this leg.
    to: ObjId,
    /// Unladen seconds at nominal thrust; at least one.
    base_s: u32,
}

/// The strict wire form of [`LegDef`].
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegDefWire {
    /// The place reached at the end of this leg.
    pub to: ObjId,
    /// Unladen seconds at nominal thrust.
    pub base_s: u32,
}

impl LegDef {
    /// The one constructor, and therefore the one place the invariants live.
    ///
    /// # Errors
    ///
    /// [`RouteDefError::ZeroLeg`] when `base_s` is zero — a leg that takes no time
    /// is an instantaneous relocation, which is exactly what design 07 §9's route
    /// invariant forbids.
    pub fn new(to: ObjId, base_s: u32) -> Result<LegDef, RouteDefError> {
        if base_s == 0 {
            return Err(RouteDefError::ZeroLeg);
        }
        Ok(LegDef { to, base_s })
    }

    /// The place reached at the end of this leg.
    #[must_use]
    pub fn to(&self) -> ObjId {
        self.to
    }

    /// Unladen seconds at nominal thrust.
    #[must_use]
    pub fn base_s(&self) -> u32 {
        self.base_s
    }
}

impl From<LegDef> for LegDefWire {
    fn from(value: LegDef) -> LegDefWire {
        LegDefWire {
            to: value.to,
            base_s: value.base_s,
        }
    }
}

impl TryFrom<LegDefWire> for LegDef {
    type Error = RouteDefError;

    fn try_from(value: LegDefWire) -> Result<LegDef, RouteDefError> {
        LegDef::new(value.to, value.base_s)
    }
}

impl<'de> Deserialize<'de> for LegDef {
    fn deserialize<D>(deserializer: D) -> Result<LegDef, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = LegDefWire::deserialize(deserializer)?;
        LegDef::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// A pack-authored route definition — the thing T1's frozen route assumptions rest
/// on (G1, D-E7-29).
///
/// Immutable once published: a change publishes a new pack version that coexists,
/// and a pinned consumer keeps the old one (Q18A). `alignment_band` is `None` until
/// a campaign cadence is declared for the body pair; cut 6 binds it, and
/// [`route_duration`] does not read it — a band is a *departure window*, not a
/// duration term.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "RouteDefWire")]
pub struct RouteDef {
    id: mush_core::CanonicalId,
    endpoints: (ObjId, ObjId),
    legs: Vec<LegDef>,
    alignment_band: Option<CadenceId>,
    drive_class: mush_core::CanonicalId,
    propellant: u64,
}

/// The strict wire form of [`RouteDef`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteDefWire {
    /// Stable definition id, unique within its pack.
    pub id: mush_core::CanonicalId,
    /// Origin and destination.
    pub endpoints: (ObjId, ObjId),
    /// The ordered legs; `1..=MAX_LEGS` of them.
    pub legs: Vec<LegDefWire>,
    /// The declared cadence the body-pair alignment band is measured on (cut 6).
    #[serde(default)]
    pub alignment_band: Option<CadenceId>,
    /// Which drive class may fly it.
    pub drive_class: mush_core::CanonicalId,
    /// Propellant the plan consumes, in the drive class's own units.
    pub propellant: u64,
}

impl RouteDef {
    /// The one constructor, and therefore the one place the invariants live.
    ///
    /// # Errors
    ///
    /// [`RouteDefError::NoLegs`] for an empty leg list, [`RouteDefError::TooManyLegs`]
    /// past [`MAX_LEGS`], [`RouteDefError::EndpointMismatch`] when the last leg does
    /// not arrive at the declared destination, and [`RouteDefError::DegenerateRoute`]
    /// when origin and destination are the same place.
    pub fn new(
        id: mush_core::CanonicalId,
        endpoints: (ObjId, ObjId),
        legs: Vec<LegDef>,
        alignment_band: Option<CadenceId>,
        drive_class: mush_core::CanonicalId,
        propellant: u64,
    ) -> Result<RouteDef, RouteDefError> {
        if legs.is_empty() {
            return Err(RouteDefError::NoLegs);
        }
        if legs.len() > MAX_LEGS {
            return Err(RouteDefError::TooManyLegs {
                actual: legs.len(),
                max: MAX_LEGS,
            });
        }
        if endpoints.0 == endpoints.1 {
            return Err(RouteDefError::DegenerateRoute);
        }
        // The definition's own coherence: whatever the legs are, following all of
        // them has to arrive where the route says it goes.
        let last = legs.last().map(LegDef::to);
        if last != Some(endpoints.1) {
            return Err(RouteDefError::EndpointMismatch);
        }
        Ok(RouteDef {
            id,
            endpoints,
            legs,
            alignment_band,
            drive_class,
            propellant,
        })
    }

    /// Stable definition id.
    #[must_use]
    pub fn id(&self) -> &mush_core::CanonicalId {
        &self.id
    }

    /// Origin and destination.
    #[must_use]
    pub fn endpoints(&self) -> (ObjId, ObjId) {
        self.endpoints
    }

    /// The ordered legs.
    #[must_use]
    pub fn legs(&self) -> &[LegDef] {
        &self.legs
    }

    /// The declared cadence the alignment band is measured on, once cut 6 binds it.
    #[must_use]
    pub fn alignment_band(&self) -> Option<&CadenceId> {
        self.alignment_band.as_ref()
    }

    /// Which drive class may fly it.
    #[must_use]
    pub fn drive_class(&self) -> &mush_core::CanonicalId {
        &self.drive_class
    }

    /// Propellant the plan consumes.
    #[must_use]
    pub fn propellant(&self) -> u64 {
        self.propellant
    }
}

impl From<RouteDef> for RouteDefWire {
    fn from(value: RouteDef) -> RouteDefWire {
        RouteDefWire {
            id: value.id,
            endpoints: value.endpoints,
            legs: value.legs.into_iter().map(LegDefWire::from).collect(),
            alignment_band: value.alignment_band,
            drive_class: value.drive_class,
            propellant: value.propellant,
        }
    }
}

impl TryFrom<RouteDefWire> for RouteDef {
    type Error = RouteDefError;

    fn try_from(value: RouteDefWire) -> Result<RouteDef, RouteDefError> {
        // The cap is re-checked before the legs are built, so a stored definition
        // with a million legs is refused rather than materialized and then rejected.
        if value.legs.len() > MAX_LEGS {
            return Err(RouteDefError::TooManyLegs {
                actual: value.legs.len(),
                max: MAX_LEGS,
            });
        }
        let mut legs = Vec::with_capacity(value.legs.len());
        for leg in value.legs {
            legs.push(LegDef::try_from(leg)?);
        }
        RouteDef::new(
            value.id,
            value.endpoints,
            legs,
            value.alignment_band,
            value.drive_class,
            value.propellant,
        )
    }
}

impl<'de> Deserialize<'de> for RouteDef {
    fn deserialize<D>(deserializer: D) -> Result<RouteDef, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RouteDefWire::deserialize(deserializer)?;
        RouteDef::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// A drive class's performance, as a rules pack declares it.
///
/// Deliberately three integers: [`route_duration`] must be a **pure function** with
/// no rounding that depends on evaluation order, so every term is checked integer
/// arithmetic over these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "DriveProfileWire")]
pub struct DriveProfile {
    class: mush_core::CanonicalId,
    thrust_permille: u32,
    payload_capacity: u64,
}

/// The strict wire form of [`DriveProfile`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveProfileWire {
    /// The drive class this profile describes.
    pub class: mush_core::CanonicalId,
    /// Thrust as permille of nominal; 1000 is nominal, 500 is half as fast.
    pub thrust_permille: u32,
    /// The greatest payload the drive may carry, in the pack's own units.
    pub payload_capacity: u64,
}

impl DriveProfile {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`RouteDefError::ZeroThrust`] and [`RouteDefError::ZeroCapacity`]; both would
    /// otherwise be a division by zero inside [`route_duration`], and a definition
    /// that can only be evaluated by dividing by zero must not exist.
    pub fn new(
        class: mush_core::CanonicalId,
        thrust_permille: u32,
        payload_capacity: u64,
    ) -> Result<DriveProfile, RouteDefError> {
        if thrust_permille == 0 {
            return Err(RouteDefError::ZeroThrust);
        }
        if payload_capacity == 0 {
            return Err(RouteDefError::ZeroCapacity);
        }
        Ok(DriveProfile {
            class,
            thrust_permille,
            payload_capacity,
        })
    }

    /// The drive class this profile describes.
    #[must_use]
    pub fn class(&self) -> &mush_core::CanonicalId {
        &self.class
    }

    /// Thrust as permille of nominal.
    #[must_use]
    pub fn thrust_permille(&self) -> u32 {
        self.thrust_permille
    }

    /// The greatest payload the drive may carry.
    #[must_use]
    pub fn payload_capacity(&self) -> u64 {
        self.payload_capacity
    }
}

impl From<DriveProfile> for DriveProfileWire {
    fn from(value: DriveProfile) -> DriveProfileWire {
        DriveProfileWire {
            class: value.class,
            thrust_permille: value.thrust_permille,
            payload_capacity: value.payload_capacity,
        }
    }
}

impl TryFrom<DriveProfileWire> for DriveProfile {
    type Error = RouteDefError;

    fn try_from(value: DriveProfileWire) -> Result<DriveProfile, RouteDefError> {
        DriveProfile::new(value.class, value.thrust_permille, value.payload_capacity)
    }
}

impl<'de> Deserialize<'de> for DriveProfile {
    fn deserialize<D>(deserializer: D) -> Result<DriveProfile, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DriveProfileWire::deserialize(deserializer)?;
        DriveProfile::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// Why a route definition, or an evaluation over one, refuses.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RouteDefError {
    /// A route with no legs is not a route.
    #[error("a route definition needs at least one leg")]
    NoLegs,
    /// Past [`MAX_LEGS`].
    #[error("a route definition declares {actual} legs; the maximum is {max}")]
    TooManyLegs {
        /// How many were declared.
        actual: usize,
        /// [`MAX_LEGS`].
        max: usize,
    },
    /// A leg that takes no time is an instantaneous relocation.
    #[error("a route leg needs a positive traversal time")]
    ZeroLeg,
    /// The legs do not arrive where the route says it goes.
    #[error("a route definition's last leg does not reach its declared destination")]
    EndpointMismatch,
    /// Origin and destination are the same place.
    #[error("a route definition's origin and destination are the same place")]
    DegenerateRoute,
    /// A drive with no thrust.
    #[error("a drive profile needs positive thrust")]
    ZeroThrust,
    /// A drive that can carry nothing.
    #[error("a drive profile needs positive payload capacity")]
    ZeroCapacity,
    /// The drive offered is not the one the route declares.
    #[error("route {route} is flown by drive class {expected}, not {actual}")]
    UnknownDriveClass {
        /// The route being evaluated.
        route: String,
        /// The class the route declares.
        expected: String,
        /// The class offered.
        actual: String,
    },
    /// More payload than the drive may carry.
    #[error("payload {payload} is past this drive's capacity of {capacity}")]
    Overloaded {
        /// The payload offered.
        payload: u64,
        /// The drive's capacity.
        capacity: u64,
    },
    /// The arithmetic left `i64`.
    #[error("route duration arithmetic overflowed")]
    Overflow,
}

/// **The pure evaluator** — the only thing T1's frozen route assumptions rest on.
///
/// Duration is a function of the definition, the drive and the payload and of
/// nothing else: no clock is read, no world is touched, no randomness is drawn, so
/// two evaluations of one triple always agree and a published hauling contract's
/// route assumptions cannot shift underneath it (T1, D-E7-24).
///
/// Per leg: `base_s` is scaled up by how far the drive falls short of nominal thrust
/// (`* 1000 / thrust_permille`), then by how loaded it is (`* (capacity + payload) /
/// capacity`), each step checked. Integer throughout and in a fixed order, so the
/// result is bit-identical on every platform.
///
/// # Errors
///
/// [`RouteDefError::UnknownDriveClass`] when the drive is not the route's,
/// [`RouteDefError::Overloaded`] past the drive's capacity, and
/// [`RouteDefError::Overflow`] if any term leaves `i64`.
pub fn route_duration(
    route: &RouteDef,
    drive: &DriveProfile,
    payload: u64,
) -> Result<i64, RouteDefError> {
    if drive.class != route.drive_class {
        return Err(RouteDefError::UnknownDriveClass {
            route: route.id.to_string(),
            expected: route.drive_class.to_string(),
            actual: drive.class.to_string(),
        });
    }
    if payload > drive.payload_capacity {
        return Err(RouteDefError::Overloaded {
            payload,
            capacity: drive.payload_capacity,
        });
    }
    let thrust = i64::from(drive.thrust_permille);
    let capacity = i64::try_from(drive.payload_capacity).map_err(|_| RouteDefError::Overflow)?;
    let laden = i64::try_from(payload)
        .ok()
        .and_then(|p| capacity.checked_add(p))
        .ok_or(RouteDefError::Overflow)?;
    let mut total: i64 = 0;
    for leg in &route.legs {
        let base = i64::from(leg.base_s);
        let scaled = base
            .checked_mul(1_000)
            .ok_or(RouteDefError::Overflow)?
            .checked_div(thrust)
            .ok_or(RouteDefError::Overflow)?;
        let loaded = scaled
            .checked_mul(laden)
            .ok_or(RouteDefError::Overflow)?
            .checked_div(capacity)
            .ok_or(RouteDefError::Overflow)?;
        total = total.checked_add(loaded).ok_or(RouteDefError::Overflow)?;
    }
    Ok(total)
}

// ---- persistence ------------------------------------------------------

/// The doc key of a journey.
#[must_use]
pub fn journey_key(traveler: ObjId) -> SmolStr {
    SmolStr::new(format!("journey:{}", traveler.0))
}

pub(crate) fn persist_journey(out: &mut mush_core::Outbox, j: &Journey) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static("sim"),
        op: ModuleWrite::JsonSet {
            key: journey_key(j.traveler),
            value: serde_json::to_value(j).expect("journey serializes"),
        },
    }));
}
pub(crate) fn persist_journey_del(out: &mut mush_core::Outbox, traveler: ObjId) {
    persist_key_del(out, journey_key(traveler));
}
/// Delete a raw journey doc key (used at boot to purge an unparseable doc so it
/// does not re-error on every subsequent boot).
pub(crate) fn persist_key_del(out: &mut mush_core::Outbox, key: SmolStr) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static("sim"),
        op: ModuleWrite::Del { key },
    }));
}

// ---- the gate reason --------------------------------------------------

/// Why a traveling object may not be moved/carried by ordinary means (it moves
/// only via the sweep, an interception, or staff `@route/abort`).
#[must_use]
pub fn traveling_reason(rs: &RouteState, w: &mush_core::World, obj: ObjId) -> Option<String> {
    rs.is_traveling(obj).then(|| {
        format!(
            "{} is en route; it moves only along its route (staff @route/abort to stop it).",
            mush_core::output::display_name(w, obj)
        )
    })
}

// ---- narrative helpers (narrative_emit is crate-private in core) -------

fn emit(text: RichText) -> OutputEvent {
    OutputEvent::Narrative {
        kind: NarrativeKind::Emit,
        actor: None,
        text,
    }
}

// ---- the sweep --------------------------------------------------------

/// The `Coarse` tick: advance every traveling journey to its correct current
/// position, narrating only near-present (live) arrivals.
pub(crate) fn sweep(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    let now = ctx.w.now();
    let mut rs = std::mem::take(
        ctx.w
            .module_state_mut::<RouteState>()
            .expect("route state slot installed at registration"),
    );
    let mut retire: Vec<ObjId> = Vec::new();
    let mut touched: Vec<ObjId> = Vec::new();

    for (&traveler, j) in rs.journeys.iter_mut() {
        if j.status != JourneyStatus::Traveling {
            continue;
        }
        match j.next_due() {
            None => {
                j.status = JourneyStatus::Arrived;
                retire.push(traveler);
                continue;
            }
            Some(nd) if nd > now => continue, // not due yet
            Some(_) => {}
        }
        // The traveler must still exist.
        let from = match ctx.w.get(traveler) {
            Some(o) if !o.flags.contains(Flags::GOING) => o.location,
            _ => {
                j.status = JourneyStatus::Aborted;
                retire.push(traveler);
                continue;
            }
        };
        // Advance the cursor to the correct current position (one relocation,
        // even across a downtime backlog — no per-leg storm).
        while j.cursor < j.legs.len() && j.due(j.cursor) <= now {
            j.cursor += 1;
        }
        let reached = j.cursor - 1;
        let dest = j.legs[reached].to;
        if ctx.w.move_to(traveler, dest).is_err() {
            // A leg room vanished/became unenterable: end the journey.
            j.status = JourneyStatus::Aborted;
            retire.push(traveler);
            touched.push(traveler);
            continue;
        }
        // Narrate only a LIVE arrival (near its due-time); a stale catch-up
        // relocation is silent (no live observer existed).
        if now - j.due(reached) <= LIVE_GRACE_S {
            if let Some(fr) = from {
                ctx.out.to_room_from(
                    fr,
                    vec![traveler],
                    traveler,
                    emit(
                        RichText::new()
                            .span(name_span(ctx.w, traveler))
                            .text(" departs."),
                    ),
                );
            }
            ctx.out.to_room_from(
                dest,
                vec![traveler],
                traveler,
                emit(
                    RichText::new()
                        .span(name_span(ctx.w, traveler))
                        .text(" arrives."),
                ),
            );
        }
        ctx.emit_world_event(&WorldEvent::Moved {
            who: traveler,
            from,
            to: dest,
        });
        if j.cursor == j.legs.len() {
            j.status = JourneyStatus::Arrived;
            retire.push(traveler);
        }
        touched.push(traveler);
    }

    // One persist per touched journey; delete terminal ones.
    for t in &touched {
        if let Some(j) = rs.journeys.get(t) {
            persist_journey(ctx.out, j);
        }
    }
    for t in retire {
        rs.journeys.remove(&t);
        persist_journey_del(ctx.out, t);
    }
    *ctx.w
        .module_state_mut::<RouteState>()
        .expect("route state slot installed at registration") = rs;
}

// ---- lifecycle: a destroyed traveler drops its journey ----------------

pub(crate) fn on_traveler_destroyed(ctx: &mut Ctx<'_>, obj: ObjId) {
    let removed = ctx
        .w
        .module_state_mut::<RouteState>()
        .expect("route state slot installed at registration")
        .journeys
        .remove(&obj)
        .is_some();
    if removed {
        persist_journey_del(ctx.out, obj);
    }
}

// ---- interception / resume (called by commands) -----------------------

/// Pause `traveler`'s journey (an interception). Returns the room it was
/// stopped in on success.
pub(crate) fn pause(
    rs: &mut RouteState,
    traveler: ObjId,
    now: i64,
    enforce_grace: bool,
) -> Result<(), String> {
    let j = rs
        .journeys
        .get_mut(&traveler)
        .ok_or("It is not going anywhere.")?;
    if j.status != JourneyStatus::Traveling {
        return Err("It is already stopped.".into());
    }
    if enforce_grace && now < j.grace_until {
        return Err("It slips past before you can act — try again in a moment.".into());
    }
    j.status = JourneyStatus::Paused;
    j.paused_at = Some(now);
    Ok(())
}

/// Resume a paused journey, shifting its schedule forward by the pause length
/// so the remaining legs keep their relative timing, with a re-interception
/// grace.
pub(crate) fn resume(rs: &mut RouteState, traveler: ObjId, now: i64) -> Result<(), String> {
    let j = rs
        .journeys
        .get_mut(&traveler)
        .ok_or("It is not going anywhere.")?;
    if j.status != JourneyStatus::Paused {
        return Err("It is not paused.".into());
    }
    if let Some(p) = j.paused_at.take() {
        j.started_at += now - p;
    }
    j.status = JourneyStatus::Traveling;
    j.grace_until = now + RESUME_GRACE_S;
    Ok(())
}

// ---- E7 cut 1: the route-definition unit tests ------------------------

#[cfg(test)]
mod route_def_tests {
    use super::*;
    use mush_core::CanonicalId;

    fn cid(s: &str) -> CanonicalId {
        CanonicalId::new(s).expect("a well-formed canonical id")
    }

    fn corridor() -> RouteDef {
        RouteDef::new(
            cid("ceres_phobos"),
            (ObjId(10), ObjId(12)),
            vec![
                LegDef::new(ObjId(11), 86_400).expect("a positive leg"),
                LegDef::new(ObjId(12), 172_800).expect("a positive leg"),
            ],
            None,
            cid("fusion_torch"),
            1_200,
        )
        .expect("a coherent route")
    }

    fn torch(capacity: u64) -> DriveProfile {
        DriveProfile::new(cid("fusion_torch"), 1_000, capacity).expect("a usable drive")
    }

    /// A route's duration is a pure function of definition, drive and payload, and two
    /// evaluations agree (10b §3.1's *Routes* tests; T1).
    #[test]
    fn e7c1_routes_duration_is_pure_and_two_evaluations_agree() {
        let route = corridor();
        let drive = torch(1_000);
        let first = route_duration(&route, &drive, 250).expect("evaluates");
        let second = route_duration(&route, &drive, 250).expect("evaluates");
        assert_eq!(first, second);
        // Unladen is exactly the sum of the legs at nominal thrust.
        assert_eq!(
            route_duration(&route, &drive, 0).expect("evaluates"),
            86_400 + 172_800
        );
        // Payload lengthens it linearly, and a slower drive lengthens it further; both
        // are integer and order-fixed, so the numbers are exact.
        assert_eq!(
            route_duration(&route, &drive, 1_000).expect("evaluates"),
            2 * (86_400 + 172_800)
        );
        let half = DriveProfile::new(cid("fusion_torch"), 500, 1_000).expect("a usable drive");
        assert_eq!(
            route_duration(&route, &half, 0).expect("evaluates"),
            2 * (86_400 + 172_800)
        );
    }

    /// An unknown drive class refuses, and so does more payload than the drive carries.
    #[test]
    fn e7c1_routes_an_unknown_drive_class_refuses() {
        let route = corridor();
        let wrong = DriveProfile::new(cid("chemical"), 1_000, 1_000).expect("a usable drive");
        assert!(matches!(
            route_duration(&route, &wrong, 0),
            Err(RouteDefError::UnknownDriveClass { .. })
        ));
        assert!(matches!(
            route_duration(&route, &torch(100), 101),
            Err(RouteDefError::Overloaded {
                payload: 101,
                capacity: 100
            })
        ));
        // A drive that could only be evaluated by dividing by zero does not exist.
        assert_eq!(
            DriveProfile::new(cid("fusion_torch"), 0, 1),
            Err(RouteDefError::ZeroThrust)
        );
        assert_eq!(
            DriveProfile::new(cid("fusion_torch"), 1, 0),
            Err(RouteDefError::ZeroCapacity)
        );
    }

    /// **The wire re-runs `MAX_LEGS`** — and every other constructor invariant — so a
    /// stored or pack-authored definition past a cap is refused on load rather than
    /// truncated (10b §4 rule 1).
    #[test]
    fn e7c1_routes_the_wire_re_runs_max_legs_and_every_invariant() {
        let good = serde_json::to_string(&corridor()).expect("serializes");
        let round_trip: RouteDef = serde_json::from_str(&good).expect("re-enters");
        assert_eq!(round_trip, corridor());

        let legs: Vec<String> = (0..=MAX_LEGS)
            .map(|i| format!("{{\"to\":{},\"base_s\":1}}", 100 + i))
            .collect();
        let too_many = format!(
            "{{\"id\":\"long\",\"endpoints\":[10,{}],\"legs\":[{}],\"drive_class\":\"t\",\
             \"propellant\":0}}",
            100 + MAX_LEGS,
            legs.join(",")
        );
        let refused = serde_json::from_str::<RouteDef>(&too_many).expect_err("past MAX_LEGS");
        assert!(refused.to_string().contains("maximum"), "{refused}");

        // A leg that takes no time, a route that arrives somewhere else, a route with
        // no legs, and a route that goes nowhere: each refused by the constructor the
        // wire runs.
        for (text, needle) in [
            (
                "{\"id\":\"z\",\"endpoints\":[10,11],\"legs\":[{\"to\":11,\"base_s\":0}],\
                 \"drive_class\":\"t\",\"propellant\":0}",
                "positive traversal",
            ),
            (
                "{\"id\":\"z\",\"endpoints\":[10,12],\"legs\":[{\"to\":11,\"base_s\":5}],\
                 \"drive_class\":\"t\",\"propellant\":0}",
                "declared destination",
            ),
            (
                "{\"id\":\"z\",\"endpoints\":[10,11],\"legs\":[],\"drive_class\":\"t\",\
                 \"propellant\":0}",
                "at least one leg",
            ),
            (
                "{\"id\":\"z\",\"endpoints\":[10,10],\"legs\":[{\"to\":10,\"base_s\":5}],\
                 \"drive_class\":\"t\",\"propellant\":0}",
                "same place",
            ),
            (
                "{\"id\":\"z\",\"endpoints\":[10,11],\"legs\":[{\"to\":11,\"base_s\":5}],\
                 \"drive_class\":\"t\",\"propellant\":0,\"nonsense\":1}",
                "unknown field",
            ),
        ] {
            let refused = serde_json::from_str::<RouteDef>(text).expect_err(text);
            assert!(refused.to_string().contains(needle), "{text}: {refused}");
        }
    }

    /// Cut 1 adds a definition and changes no runtime type: `Leg`, `Journey` and
    /// `RouteState` are untouched, and a `RouteDef` is not a `Journey`.
    #[test]
    fn e7c1_routes_the_runtime_types_are_untouched() {
        let leg = Leg {
            to: ObjId(11),
            travel_s: DEFAULT_TRAVEL_S,
        };
        let journey = Journey {
            traveler: ObjId(1),
            legs: vec![leg.clone()],
            cursor: 0,
            started_at: 0,
            status: JourneyStatus::Traveling,
            paused_at: None,
            grace_until: 0,
            destination: ObjId(11),
            by: ObjId(1),
        };
        assert_eq!(journey.next_due(), Some(i64::from(DEFAULT_TRAVEL_S)));
        let mut state = RouteState::default();
        state.journeys.insert(ObjId(1), journey);
        assert!(state.is_traveling(ObjId(1)));
        // The definition names the same places without becoming one of them.
        assert_eq!(corridor().legs().len(), 2);
        assert_eq!(corridor().endpoints(), (ObjId(10), ObjId(12)));
    }
}
