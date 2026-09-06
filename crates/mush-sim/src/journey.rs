//! # Carrier journeys — E7 cut 6 (10b §3.6), RESOLUTION A
//!
//! A **carrier root** (a ship, a dropship, a hauler) follows a [`RouteDef`]'s
//! ordered legs on the world's **IC** clock, and **everything hosted on it moves
//! with it**: anchored interiors ([`SpaceAnchor::MovingObject`]), their
//! occupants, attached mounts and craft, cargo and crewed stations. There is
//! **no second placement topology** and no per-aboard record (C16A): the subtree
//! is the pre-existing [`SpatialState::subtree`] and the only thing a journey
//! ever moves is the **root's own** placement record.
//!
//! ## Why a Coarse tick and not a schedule source (RESOLUTION A, 2026-09-06)
//!
//! The first design made the advance a `reg.schedule_source` that emitted a
//! `WorldEvent` a synchronous observer reacted to. It cannot work on this tree:
//! a `schedule_source`-fired action runs inside `prepare_authoritative` with no
//! `&World`, no `Ctx` and no `Outbox`, and writes only typed `state_family`
//! records through an `ActionTxn`; `WorldEvent` is emitted only from the `Ctx`
//! dispatch layer; and placement is a legacy `reg.state` slot that
//! `prepare_authoritative` cannot reach. There is no world-side seam on a
//! scheduled authoritative commit.
//!
//! So this is the **pull** form of the same rule — the housing assessment
//! tick's shape (`mush-housing/src/tick.rs`, "a pull, not a subscription") and
//! the economy sweep's budget:
//!
//! - one `Coarse` [`reg.tick`](mush_core::Registrar::tick), [`TICK_TAG`];
//! - each fire reads [`mush_core::World::world_ic_now`] and advances every live
//!   journey whose IC position has passed its next leg boundary;
//! - each advance is **one atomic tick batch**: the journey document write and
//!   the synchronous world mutation land in the same `Outbox`, so a crash either
//!   applied the whole arrival or none of it;
//! - the **leg cursor is the idempotency key**. Re-running the pass over the
//!   same documents — which is exactly what a reboot does — is a no-op past the
//!   point already applied. That *is* B3's crash-gap healing: there is no
//!   separate "boot reconcile" pass, only [`advance_due`] running at boot.
//!
//! ## What is bounded
//!
//! [`MAX_JOURNEYS_VISITED_PER_FIRE`] / [`MAX_JOURNEYS_WORKED_PER_FIRE`] over a
//! rotation cursor (D-LBK-2/-5, housing's budget), and
//! [`MAX_CARRIER_SUBTREE`] on the records one carrier move visits — §7 lists
//! `MAX_ACTIVE_DOMAINS`, `QUIET_BOUNDARY` and `MAX_LEGS` but **no per-carrier
//! subtree cap**, and this is that gap closed.
//!
//! ## Exhaustive by construction
//!
//! Every match here is exhaustive and there is no `?`, no `unwrap`, no `expect`
//! and no panicking index after the first irreversible mutation: a tick handler
//! runs in the world task, so a panic here takes the world down. Arithmetic that
//! could leave `i64` is checked and **pages** (E5a S2) instead of stalling.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{
    CanonicalId, ClockRef, ClockStamp, Ctx, Flags, ObjId, Outbox, SceneId, World, WorldEvent,
    output::name_span,
};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::data::{DomainId, SpatialParent, SpatialState, push_op};
use crate::route::{MAX_LEGS, RouteDef, RouteDefError, RouteDefWire};

/// The registered tick tag.
pub const TICK_TAG: &str = "sim.journey";
/// The tick cadence in seconds. A journey's own absolute IC due-times are the
/// authority; this only sets how often they are checked.
pub const TICK_SECS: u64 = 1;

/// The most records one carrier move may visit (10b §3.6's recorded §7 gap:
/// there is no per-carrier occupant/subtree cap, only `MAX_SPATIAL_DEPTH` 8 on
/// depth). One window's worth; past it the move is refused rather than making an
/// unbounded arrival bundle.
pub const MAX_CARRIER_SUBTREE: usize = 256;
/// How many journeys one tick fire may touch at all.
pub const MAX_JOURNEYS_VISITED_PER_FIRE: usize = 64;
/// How many journeys one tick fire may actually advance.
pub const MAX_JOURNEYS_WORKED_PER_FIRE: usize = 16;
/// The cap on a journey's recorded anomaly diagnosis (a bound, like every other
/// stored string in the crate).
pub const MAX_JOURNEY_ANOMALY_BYTES: usize = 240;
/// How many carrier journeys one world may hold at once — the census §7 never
/// declared (E7 cut 6 fold, B8), in the `MAX_STATION_FALLBACKS` shape: the
/// `cjourney:` key space and the tick's per-fire walk are both bounded by it,
/// and the `MAX_CARRIER_JOURNEYS + 1`-th `@journey/depart` is refused rather
/// than admitted and then truncated.
pub const MAX_CARRIER_JOURNEYS: usize = 512;

/// A durable journey id — monotonic within a world, never reused.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JourneyId(pub u64);

impl std::fmt::Display for JourneyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a carrier journey has got to (10b §3.6; contradiction (iii): no
/// `resume`, no `Pulse` — `Promoted` is only the marker cut 8's authority sets).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state")]
pub enum JourneyState {
    /// Under way. The root is deliberately **not** directly placed in a live
    /// domain; its subtree is, and rides with it.
    Cruising,
    /// Under way inside a promoted (tactical) domain. Cut 8's authority sets
    /// this; cut 6 only reads it, and treats it as under way.
    Promoted {
        /// The domain the carrier was promoted into.
        domain: DomainId,
    },
    /// Under way on a re-planned route (`@journey/divert`), naming it.
    Diverted {
        /// The route the journey was diverted onto.
        to: CanonicalId,
    },
    /// Terminal: the carrier reached the route's destination.
    Arrived,
    /// Terminal: taken. Cut 9 owns capture; the arm exists so a stored record
    /// written by a later cut re-enters here rather than being refused.
    Captured {
        /// Who took it.
        by: ObjId,
    },
    /// Terminal: the journey stopped and a human must decide (the record's
    /// `anomaly` carries the diagnosis and the recommended command).
    Failed,
}

impl JourneyState {
    /// Whether the journey is still moving (or held tactically): the states in
    /// which the carrier-root subtree is **sealed** and the tick advances it.
    #[must_use]
    pub fn is_under_way(&self) -> bool {
        match self {
            JourneyState::Cruising
            | JourneyState::Promoted { .. }
            | JourneyState::Diverted { .. } => true,
            JourneyState::Arrived | JourneyState::Captured { .. } | JourneyState::Failed => false,
        }
    }
}

/// Why a [`CarrierJourney`] refused to be constructed or to re-enter from its
/// wire.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CarrierJourneyError {
    /// The route itself refused (`MAX_LEGS`, zero leg, endpoint mismatch, …).
    #[error(transparent)]
    Route(#[from] RouteDefError),
    /// The stored cursor names a leg the route does not have.
    #[error("journey cursor {cursor} is past the route's {legs} leg(s)")]
    CursorPastEnd {
        /// The stored cursor.
        cursor: u32,
        /// How many legs the route declares.
        legs: usize,
    },
    /// The departure and last-advance stamps name different clock generations.
    #[error("a journey's departure and last-advance stamps must share one clock generation")]
    ClockMismatch,
    /// A journey is clocked on the world IC timeline and nothing else.
    #[error("a carrier journey must be clocked on ClockRef::WorldIc")]
    NotWorldIc,
    /// The destination is not a place a carrier can come to rest.
    #[error("a carrier journey needs a placed destination")]
    UnplacedDestination,
    /// The recorded diagnosis is over [`MAX_JOURNEY_ANOMALY_BYTES`].
    #[error("a journey anomaly note is too long")]
    AnomalyTooLong,
}

/// A carrier's journey along a [`RouteDef`] — the **exempt `sim.journey`**
/// workflow envelope (§4.1): a bounded whole-document legacy-post-image record
/// at `cjourney:<root>`, with a strict [`TryFrom`] wire that re-runs every
/// constructor invariant — including the leg cap, which is **[`RouteDef`]'s**
/// and is enforced twice inside `RouteDefWire::try_from` (E7 cut 6 fold, B7:
/// this record carries no independent bound of its own) — so a stored set past
/// a cap is **refused on load, never truncated**, and since the fold never
/// deleted either (R3: boot HOLDS a refused document rather than dropping the
/// only record of where an unplaced ship is) (§4 rule 1).
///
/// Legality derives from **placement**, never from [`JourneyState`] (OPEN 4):
/// the state says what the journey is doing, the placement graph says where
/// everything is.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(into = "CarrierJourneyWire")]
pub struct CarrierJourney {
    journey: JourneyId,
    root: ObjId,
    route: RouteDef,
    destination: SpatialParent,
    departed: ClockStamp,
    advanced_at: ClockStamp,
    cursor: u32,
    state: JourneyState,
    anomaly: Option<String>,
    applied_revision: u64,
}

/// The strict wire form of [`CarrierJourney`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarrierJourneyWire {
    /// Durable id.
    pub journey: JourneyId,
    /// The carrier root.
    pub root: ObjId,
    /// The route being flown.
    pub route: RouteDefWire,
    /// Where the root comes to rest at arrival.
    pub destination: SpatialParent,
    /// The WorldIc stamp the journey departed at.
    pub departed: ClockStamp,
    /// The WorldIc position of the last applied advance.
    pub advanced_at: ClockStamp,
    /// Legs already applied (the idempotent cursor), `0..=route.legs().len()`.
    pub cursor: u32,
    /// Where it has got to.
    pub state: JourneyState,
    /// A paged diagnosis, if one stands.
    #[serde(default)]
    pub anomaly: Option<String>,
    /// The root's placement revision the arrival was applied at (E7 cut 6
    /// fold, D3). `#[serde(default)]` so every stored document still re-enters
    /// — an old document reads 0, which scopes its repair claim to nothing and
    /// therefore fails closed.
    #[serde(default)]
    pub applied_revision: u64,
}

impl CarrierJourney {
    /// The one constructor, and therefore the one place the invariants live.
    ///
    /// # Errors
    ///
    /// [`CarrierJourneyError::CursorPastEnd`] for a cursor past the route's
    /// legs, [`CarrierJourneyError::ClockMismatch`] when the two stamps name
    /// different generations, [`CarrierJourneyError::NotWorldIc`] for any clock
    /// but [`ClockRef::WorldIc`], [`CarrierJourneyError::UnplacedDestination`]
    /// for a destination a carrier cannot rest at, and
    /// [`CarrierJourneyError::AnomalyTooLong`] past
    /// [`MAX_JOURNEY_ANOMALY_BYTES`].
    // Nine fields, nine arguments: this is the ONE place a journey's invariants
    // live (§4 rule 1), and every field is re-checked here and by the `TryFrom`
    // wire. Grouping them into a builder would give the invariants a second
    // entry point, which is exactly what rule 1 forbids.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        journey: JourneyId,
        root: ObjId,
        route: RouteDef,
        destination: SpatialParent,
        departed: ClockStamp,
        advanced_at: ClockStamp,
        cursor: u32,
        state: JourneyState,
        anomaly: Option<String>,
    ) -> Result<CarrierJourney, CarrierJourneyError> {
        if !matches!(departed.clock, ClockRef::WorldIc { .. }) {
            return Err(CarrierJourneyError::NotWorldIc);
        }
        if departed.clock != advanced_at.clock {
            return Err(CarrierJourneyError::ClockMismatch);
        }
        // The leg cap is `RouteDef`'s and it is enforced TWICE inside
        // `RouteDefWire::try_from` — once before any `LegDef` is built and once
        // by `RouteDef::new`, the type's only constructor, whose fields are
        // private. No `RouteDef` value can therefore exist whose `legs()`
        // exceeds `MAX_LEGS`, so the arm this record used to carry was
        // unreachable (E7 cut 6 fold, B7). Stated as a `debug_assert!` so it
        // cannot be mistaken for a live bound the journey record owns.
        debug_assert!(
            route.legs().len() <= MAX_LEGS,
            "RouteDef is the one owner of the leg cap"
        );
        if usize::try_from(cursor).unwrap_or(usize::MAX) > route.legs().len() {
            return Err(CarrierJourneyError::CursorPastEnd {
                cursor,
                legs: route.legs().len(),
            });
        }
        if !destination.is_placed() {
            return Err(CarrierJourneyError::UnplacedDestination);
        }
        if anomaly
            .as_ref()
            .is_some_and(|a| a.len() > MAX_JOURNEY_ANOMALY_BYTES)
        {
            return Err(CarrierJourneyError::AnomalyTooLong);
        }
        Ok(CarrierJourney {
            journey,
            root,
            route,
            destination,
            departed,
            advanced_at,
            cursor,
            state,
            anomaly,
            applied_revision: 0,
        })
    }

    /// Scope this record's arrival-repair claim to a placement revision
    /// (E7 cut 6 fold, D3). Deliberately **not** a tenth constructor argument:
    /// it carries no invariant of its own — it is a cursor into the placement
    /// lane — and `new` stays the one place a journey's invariants live.
    #[must_use]
    pub fn with_applied_revision(mut self, revision: u64) -> CarrierJourney {
        self.applied_revision = revision;
        self
    }

    /// Durable id.
    #[must_use]
    pub fn id(&self) -> JourneyId {
        self.journey
    }
    /// The carrier root.
    #[must_use]
    pub fn root(&self) -> ObjId {
        self.root
    }
    /// The route being flown.
    #[must_use]
    pub fn route(&self) -> &RouteDef {
        &self.route
    }
    /// Where the root comes to rest at arrival.
    #[must_use]
    pub fn destination(&self) -> &SpatialParent {
        &self.destination
    }
    /// The WorldIc stamp it departed at.
    #[must_use]
    pub fn departed(&self) -> &ClockStamp {
        &self.departed
    }
    /// The WorldIc position of the last applied advance.
    #[must_use]
    pub fn advanced_at(&self) -> &ClockStamp {
        &self.advanced_at
    }
    /// Legs already applied — the idempotent cursor.
    #[must_use]
    pub fn cursor(&self) -> u32 {
        self.cursor
    }
    /// Where it has got to.
    #[must_use]
    pub fn state(&self) -> &JourneyState {
        &self.state
    }
    /// The paged diagnosis, if one stands.
    #[must_use]
    pub fn anomaly(&self) -> Option<&str> {
        self.anomaly.as_deref()
    }

    /// The root placement revision this record's arrival was applied at.
    ///
    /// **The repair claim's scope** (E7 cut 6 fold, D3). `step` re-applies an
    /// `Arrived` journey's move only while the root's live placement revision
    /// is *at or below* this: a rolled-back placement document (the s3 crash
    /// gap, whose revision is the old one) still heals, while a legitimate
    /// later placement bumps the revision past it and ends the claim, so an
    /// arrived journey stops dragging its carrier back forever.
    #[must_use]
    pub fn applied_revision(&self) -> u64 {
        self.applied_revision
    }

    /// The absolute WorldIc position by which leg `k` (0-based) must have been
    /// reached. `None` when `k` is past the route or the sum leaves `i64` — an
    /// overflow **pages**, it never stalls (E5a S2).
    #[must_use]
    pub fn leg_due_at(&self, k: usize) -> Option<i64> {
        let legs = self.route.legs();
        if k >= legs.len() {
            return None;
        }
        let mut at = self.departed.at;
        for leg in &legs[..=k] {
            at = at.checked_add(i64::from(leg.base_s()))?;
        }
        Some(at)
    }

    /// The absolute WorldIc position of arrival (the last leg's boundary).
    #[must_use]
    pub fn arrival_at(&self) -> Option<i64> {
        self.leg_due_at(self.route.legs().len().checked_sub(1)?)
    }

    /// The greatest cursor whose whole prefix of legs is due at `ic_now`.
    /// `None` when a boundary's arithmetic overflowed.
    fn due_cursor(&self, ic_now: i64) -> Option<u32> {
        let mut reached = self.cursor;
        let total = self.route.legs().len();
        let mut k = usize::try_from(self.cursor).unwrap_or(usize::MAX);
        while k < total {
            let due = self.leg_due_at(k)?;
            if due > ic_now {
                break;
            }
            reached = reached.checked_add(1)?;
            k = k.checked_add(1)?;
        }
        Some(reached)
    }
}

impl From<CarrierJourney> for CarrierJourneyWire {
    fn from(value: CarrierJourney) -> CarrierJourneyWire {
        CarrierJourneyWire {
            journey: value.journey,
            root: value.root,
            route: RouteDefWire::from(value.route),
            destination: value.destination,
            departed: value.departed,
            advanced_at: value.advanced_at,
            cursor: value.cursor,
            state: value.state,
            anomaly: value.anomaly,
            applied_revision: value.applied_revision,
        }
    }
}

impl TryFrom<CarrierJourneyWire> for CarrierJourney {
    type Error = CarrierJourneyError;

    fn try_from(value: CarrierJourneyWire) -> Result<CarrierJourney, CarrierJourneyError> {
        // The route's own wire re-runs MAX_LEGS before it builds any `LegDef`,
        // so a stored definition with a million legs is refused before the legs
        // are materialized. (Serde has already built the `Vec<LegDefWire>` by
        // then — the saving is the `LegDef` construction, not the parse.)
        let route = RouteDef::try_from(value.route)?;
        let applied_revision = value.applied_revision;
        CarrierJourney::new(
            value.journey,
            value.root,
            route,
            value.destination,
            value.departed,
            value.advanced_at,
            value.cursor,
            value.state,
            value.anomaly,
        )
        .map(|j| j.with_applied_revision(applied_revision))
    }
}

impl<'de> Deserialize<'de> for CarrierJourney {
    fn deserialize<D>(deserializer: D) -> Result<CarrierJourney, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CarrierJourneyWire::deserialize(deserializer)?;
        CarrierJourney::try_from(wire).map_err(serde::de::Error::custom)
    }
}

// ----------------------------------------------------------------------
// The state slot
// ----------------------------------------------------------------------

/// The module's carrier-journey slot: one record per carrier root, the id
/// allocator, and the tick's rotation cursor. Never persisted as a whole (each
/// journey is one document); rebuilt at boot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JourneyBook {
    /// Journeys by carrier root.
    pub journeys: BTreeMap<ObjId, CarrierJourney>,
    /// The id allocator resume point.
    pub next_id: u64,
    /// The tick's rotation cursor (D-LBK-2): the next root id to start at.
    pub cursor: u32,
    /// `cjourney:` documents the strict wire refused at boot, by key, with the
    /// reason. **Held, never deleted** (E7 cut 6 fold, R3): bounded in-memory
    /// diagnostics exactly like `SpatialState::unbound_placement_docs`, while
    /// the authoritative value stays untouched in the store.
    pub held: BTreeMap<SmolStr, String>,
}

impl JourneyBook {
    /// The journey a carrier root is on, if any.
    #[must_use]
    pub fn get(&self, root: ObjId) -> Option<&CarrierJourney> {
        self.journeys.get(&root)
    }

    /// Whether `root` is a carrier root that is currently under way.
    #[must_use]
    pub fn under_way(&self, root: ObjId) -> bool {
        self.journeys
            .get(&root)
            .is_some_and(|j| j.state.is_under_way())
    }

    /// Allocate the next journey id. `None` when the allocator is exhausted
    /// (a persisted monotonic counter must always retain a resume cursor).
    pub fn alloc_id(&mut self) -> Option<JourneyId> {
        let id = JourneyId(self.next_id);
        self.next_id = self.next_id.checked_add(1)?;
        Some(id)
    }
}

/// The under-way carrier root `obj` rides, if any: `obj` itself, or the nearest
/// ancestor that is one. Bounded by [`crate::data::MAX_SPATIAL_DEPTH`] through
/// [`crate::graph::ancestors`]; a malformed graph answers `None` and leaves the
/// caller's own cycle check to surface it.
#[must_use]
pub fn under_way_root_of(st: &SpatialState, book: &JourneyBook, obj: ObjId) -> Option<ObjId> {
    if book.under_way(obj) {
        return Some(obj);
    }
    let up = crate::graph::ancestors(st, obj).ok()?;
    up.into_iter().find(|&a| book.under_way(a))
}

/// Why a carrier with a journey record may not be moved or carried by ordinary
/// means — the [`crate::PlacementGate`](crate) half of the carrier-root
/// invariant.
///
/// **Phrased over record presence and unplacedness, never over a second
/// "is it moving?" predicate** (E7 cut 6 fold, D1). [`JourneyState::is_under_way`]
/// stays the ONE predicate the seal, the move gate, the links and the tick
/// share; this gate is deliberately *wider* than it: any journey record whose
/// root is not placed is covered, with per-state wording. That is what closes
/// C4 by construction — a `Failed` or `Captured` journey (and any terminal arm
/// a later cut adds) leaves an unplaced root that is still gated, so no
/// bystander can `get` a dropship whose passage merely stopped.
#[must_use]
pub fn journeying_reason(book: &JourneyBook, w: &World, obj: ObjId) -> Option<String> {
    let journey = book.get(obj)?;
    if w.module_state::<SpatialState>()
        .is_some_and(|st| st.is_placed(obj))
    {
        return None;
    }
    let name = mush_core::output::display_name(w, obj);
    Some(match journey.state {
        JourneyState::Cruising | JourneyState::Promoted { .. } | JourneyState::Diverted { .. } => {
            format!(
                "{name} is under way; it moves only along its journey (staff @journey/divert to re-plan it)."
            )
        }
        JourneyState::Arrived => format!(
            "{name}'s passage ended but it is not placed anywhere; staff @spatial/place/domain it."
        ),
        JourneyState::Captured { .. } => format!(
            "{name} was taken and is not placed anywhere; staff @journey/divert it (or @spatial/place/domain to recover it by hand)."
        ),
        JourneyState::Failed => format!(
            "{name}'s journey stopped and it is not placed anywhere; staff @journey/divert it (or @spatial/place/domain to recover it by hand)."
        ),
    })
}

// ----------------------------------------------------------------------
// Persistence
// ----------------------------------------------------------------------

/// The doc key of a carrier journey. Deliberately **not** `journey:` — that
/// prefix is the legacy `@route` runtime's (`route::journey_key`), which cut 6
/// leaves untouched (10b §3.6 contradiction (i)).
#[must_use]
pub fn carrier_journey_key(root: ObjId) -> SmolStr {
    SmolStr::new(format!("cjourney:{}", root.0))
}

/// The doc key of the journey allocator.
pub const JOURNEY_META_KEY: &str = "journey_meta";

/// Queue the journey document. Returns whether it was queued: a record that
/// will not serialize **pages** rather than panicking, because this runs in the
/// world task from the tick and from boot (E7 cut 6 fold, R12 — the module's
/// own no-panic contract, restated as code rather than as a comment).
pub(crate) fn persist_journey(out: &mut Outbox, j: &CarrierJourney) -> bool {
    match serde_json::to_value(j) {
        Ok(value) => {
            push_op(
                out,
                mush_core::ModuleWrite::JsonSet {
                    key: carrier_journey_key(j.root),
                    value,
                },
            );
            true
        }
        Err(error) => {
            tracing::error!(
                root = %j.root,
                journey = %j.journey,
                %error,
                "sim: a carrier journey would not serialize; the document was NOT written.                  Recommended: @journey/show the carrier and re-issue @journey/divert"
            );
            false
        }
    }
}

pub(crate) fn persist_journey_del(out: &mut Outbox, root: ObjId) {
    push_op(
        out,
        mush_core::ModuleWrite::Del {
            key: carrier_journey_key(root),
        },
    );
}

/// The allocator document — and, since the fold (R11), the tick's rotation
/// cursor, so a restart resumes the rotation where it left off instead of
/// re-walking the lowest root ids and starving the tail of the book.
pub(crate) fn persist_journey_meta(out: &mut Outbox, next_id: u64, rotation_cursor: u32) {
    push_op(
        out,
        mush_core::ModuleWrite::JsonSet {
            key: SmolStr::new_static(JOURNEY_META_KEY),
            value: serde_json::json!({
                "next_journey_id": next_id,
                "rotation_cursor": rotation_cursor,
            }),
        },
    );
}

// ----------------------------------------------------------------------
// Boot
// ----------------------------------------------------------------------

/// What boot hydrated out of the `sim` kv for the journey lane.
pub(crate) struct HydratedJourneys {
    /// The records that re-entered through their `TryFrom` wire.
    pub journeys: Vec<CarrierJourney>,
    /// The allocator resume point.
    pub next_id: u64,
    /// The tick rotation cursor (E7 cut 6 fold, R11).
    pub rotation_cursor: u32,
    /// `(key, why)` for every `cjourney:` document the strict wire refused.
    /// **Held, never deleted** (E7 cut 6 fold, R3).
    pub held: Vec<(SmolStr, String)>,
}

/// Pull `cjourney:*` / `journey_meta` documents out of the boot kv (so
/// `load_boot` does not warn on them) and re-enter each through its `TryFrom`
/// wire. A corrupt document is skipped **loudly** and its key returned so the
/// caller can HOLD it; the raw value is never rewritten and never deleted
/// (§4 rule 2, and E7 cut 6 fold R3 — an under-way carrier is deliberately not
/// placed, so this document is the only record of where its whole crewed
/// subtree is).
pub(crate) fn extract_carrier_journeys(data: &mut mush_core::ModuleBootData) -> HydratedJourneys {
    let mut out = Vec::new();
    let mut held: Vec<(SmolStr, String)> = Vec::new();
    let mut next_id = 0u64;
    let mut rotation_cursor = 0u32;
    data.kv.retain(|(key, value)| {
        if key.as_str() == JOURNEY_META_KEY {
            match value.get("next_journey_id").and_then(serde_json::Value::as_u64) {
                Some(n) => next_id = n,
                None => {
                    tracing::error!(%key, "sim: corrupt journey allocator doc ignored");
                }
            }
            // A document written before the fold carries no cursor; 0 is the
            // pre-fold behaviour (start at the head of the book), so an older
            // document is read exactly as it always was.
            rotation_cursor = value
                .get("rotation_cursor")
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0);
            false
        } else if key.starts_with("cjourney:") {
            match serde_json::from_value::<CarrierJourney>(value.clone()) {
                Ok(j) if carrier_journey_key(j.root) == *key => out.push(j),
                Ok(j) => {
                    tracing::error!(%key, root = %j.root, "sim: carrier-journey doc key/root mismatch HELD");
                    held.push((
                        key.clone(),
                        format!("the key names a different root than the body ({})", j.root),
                    ));
                }
                Err(error) => {
                    tracing::error!(%key, %error, "sim: unreadable carrier-journey doc HELD");
                    held.push((key.clone(), error.to_string()));
                }
            }
            false
        } else {
            true
        }
    });
    HydratedJourneys {
        journeys: out,
        next_id,
        rotation_cursor,
        held,
    }
}

/// Install the hydrated journeys, then run the **same idempotent advance** the
/// tick runs. That single call is B3: a journey whose durable cursor says it
/// arrived but whose carrier root is not at the destination is repaired here,
/// and a journey whose cursor is already current is a no-op. There is no
/// separate reconcile pass to disagree with the tick.
pub(crate) fn install_carrier_journeys(ctx: &mut Ctx<'_>, hydrated: HydratedJourneys) {
    let HydratedJourneys {
        journeys,
        next_id,
        rotation_cursor,
        held,
    } = hydrated;
    let mut book = JourneyBook::default();
    let mut discovered: u64 = next_id;
    for j in journeys {
        let live = ctx
            .w
            .get(j.root)
            .is_some_and(|o| !o.flags.contains(Flags::GOING));
        if !live {
            // The ONE legitimate delete: the object the key names is gone, so
            // the record can strand nothing (the mirror of `on_root_destroyed`).
            tracing::warn!(root = %j.root, "sim: carrier journey dropped at boot (root gone)");
            persist_journey_del(ctx.out, j.root);
            continue;
        }
        discovered = discovered.max(j.journey.0.saturating_add(1));
        book.journeys.insert(j.root, j);
    }
    book.next_id = discovered;
    book.cursor = rotation_cursor;
    for (key, why) in &held {
        book.held.insert(key.clone(), why.clone());
    }
    if let Some(slot) = ctx.w.module_state_mut::<JourneyBook>() {
        *slot = book;
    }
    // **A refused journey document is HELD, never deleted** (E7 cut 6 fold, R3).
    // The raw value stays in the store exactly as `boot.rs` holds an unreadable
    // placement, the key is recorded in `JourneyBook::held`, and the affected
    // root — whose whole crewed subtree is otherwise silently nowhere — is
    // quarantined so it is findable, with the recovery command filled in.
    for (key, why) in held {
        let root = key
            .as_str()
            .strip_prefix("cjourney:")
            .and_then(|n| n.parse::<u32>().ok())
            .map(ObjId)
            .filter(|r| ctx.w.get(*r).is_some());
        match root {
            Some(root) => {
                let unplaced = ctx
                    .w
                    .module_state::<SpatialState>()
                    .is_some_and(|st| !st.is_placed(root));
                if unplaced
                    && let Err(error) = crate::transition::set_carrier_root_quarantine(
                        ctx,
                        root,
                        true,
                        "its carrier-journey document was refused at boot and is held",
                    )
                {
                    tracing::error!(%key, %root, %error, "sim: a held journey's root could not be quarantined");
                }
                tracing::error!(
                    %key,
                    %root,
                    reason = %why,
                    recommended = %format!("@spatial/place/domain #{}=<domain>", root.0),
                    "sim: a carrier-journey document was refused and is HELD (never deleted); its \
                     carrier is quarantined until staff place it by hand"
                );
            }
            None => tracing::error!(
                %key,
                reason = %why,
                "sim: a carrier-journey document was refused and is HELD (never deleted); its key \
                 names no live object"
            ),
        }
    }
    // **A hydrated `Cruising` record whose root is PLACED is incoherent**
    // (E7 cut 6 fold, C10). Nothing in cut 6 constructs it, but nothing repairs
    // it either: `step` only ever heals the `Arrived` direction, so a
    // hand-edited or migrated document would leave a ship sitting in a hangar
    // permanently sealed (nothing boards or leaves) and permanently un-movable
    // (the gate refuses), with no anomaly and no page. Re-unplace the root
    // through the ONE checked carrier move so record and graph agree; a refusal
    // stops that journey and pages, and nothing else.
    let incoherent: Vec<ObjId> = {
        let st = ctx.w.module_state::<SpatialState>();
        match ctx.w.module_state::<JourneyBook>() {
            Some(book) => book
                .journeys
                .values()
                .filter(|j| j.state.is_under_way() && st.is_some_and(|st| st.is_placed(j.root)))
                .map(|j| j.root)
                .collect(),
            None => Vec::new(),
        }
    };
    for root in incoherent {
        tracing::error!(
            %root,
            "sim: a carrier journey says under way while its root is placed; re-unplacing the \
             root so the record and the placement graph agree"
        );
        if let Err(error) = crate::transition::move_carrier_subtree(ctx, root, None)
            && let Some(journey) = ctx
                .w
                .module_state::<JourneyBook>()
                .and_then(|b| b.get(root).cloned())
        {
            fail(
                ctx,
                journey,
                &format!(
                    "the journey says under way but its carrier is placed, and it could not be re-unplaced ({error})"
                ),
            );
        }
    }
    advance_due(ctx, Source::Boot);
}

// ----------------------------------------------------------------------
// The tick
// ----------------------------------------------------------------------

/// Who is running the advance. The durable truth — placement, record, onboard
/// floors — is identical either way; only the **audience** differs, because
/// `Ctx::emit_world_event` dispatches inline to observers and the modules that
/// subscribe to `Moved` (script, npc, weather) are composed *after* `mush-sim`
/// and have not hydrated when `on_world_loaded` runs (E7 cut 6 fold, R9).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The registered `Coarse` tick: a live audience.
    Tick,
    /// `install_carrier_journeys`, inside `on_world_loaded`: no audience, and
    /// no un-hydrated observer may be woken.
    Boot,
}

/// The registered `Coarse` tick.
pub(crate) fn tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    advance_due(ctx, Source::Tick);
}

/// What one journey cost this fire.
enum Step {
    /// Nothing due (or the journey is terminal and coherent).
    Skipped,
    /// The cursor moved, a carrier was moved, or an anomaly was recorded.
    Worked,
}

/// Whether an `Arrived` record still holds a live repair claim on its root
/// (E7 cut 6 fold, D3/C3): the carrier is not where the record says, AND the
/// root's live placement revision has not moved past the one the arrival was
/// applied at. A rolled-back placement document (older revision) still heals; a
/// legitimate later placement (newer revision) ends the claim for good.
fn heal_pending(st: &SpatialState, j: &CarrierJourney) -> bool {
    j.state == JourneyState::Arrived
        && !same_place(&st.parent_of(j.root), &j.destination)
        && st
            .placements
            .get(&j.root)
            .is_some_and(|p| p.revision <= j.applied_revision)
}

/// Whether a terminal journey has left its root loose in the graph — neither
/// placed nor quarantined, i.e. exactly C4's un-gated dropship. `fail` closes
/// this at the source (D1); this is the tick's backstop for a terminal record a
/// later cut writes directly.
fn loose_terminal(st: &SpatialState, j: &CarrierJourney) -> bool {
    !j.state.is_under_way() && !st.is_placed(j.root) && !st.is_quarantined(j.root)
}

/// Whether this fire has anything at all to do for `j`. **Filter before take**
/// (E7 cut 6 fold, R8): a terminal, coherent record costs no `visited` budget,
/// so a world with a long passage history still detects a live boundary on the
/// first fire instead of `ceil(N/64)` seconds later.
fn needs_a_slice(st: Option<&SpatialState>, j: &CarrierJourney) -> bool {
    if j.state.is_under_way() {
        return true;
    }
    st.is_some_and(|st| heal_pending(st, j) || loose_terminal(st, j))
}

/// Advance every live journey whose IC position has passed its next boundary,
/// and repair any arrived journey whose carrier is not where it says it is.
///
/// **Idempotent**: re-running this over the same documents applies nothing past
/// what is already applied, which is what makes a reboot (and the tick's own
/// re-fire) safe with no crash gap.
pub(crate) fn advance_due(ctx: &mut Ctx<'_>, source: Source) {
    let now = ctx.w.world_ic_now();
    // The rotation: start at the first root at or past the cursor, wrap once, so
    // a fire that is cut off by its budget resumes with whoever was skipped
    // rather than at the top again (D-LBK-2). The range walk allocates at most
    // `MAX_JOURNEYS_VISITED_PER_FIRE` entries however large the book is (B8).
    let order: Vec<ObjId> = {
        let st = ctx.w.module_state::<SpatialState>();
        let Some(book) = ctx.w.module_state::<JourneyBook>() else {
            return;
        };
        if book.journeys.is_empty() {
            return;
        }
        let start = ObjId(book.cursor);
        book.journeys
            .range(start..)
            .chain(book.journeys.range(..start))
            .filter(|(_, j)| needs_a_slice(st, j))
            .map(|(root, _)| *root)
            .take(MAX_JOURNEYS_VISITED_PER_FIRE)
            .collect()
    };
    if order.is_empty() {
        return;
    }

    let mut worked = 0usize;
    let mut last: Option<ObjId> = None;
    for root in &order {
        if worked >= MAX_JOURNEYS_WORKED_PER_FIRE {
            break;
        }
        last = Some(*root);
        // One LEG BOUNDARY per slice (E7 cut 6 fold, R13): a long downtime is
        // caught up boundary by boundary, each observed exactly once and each
        // charging one `worked` slot, so a future cut can hang an interception
        // or an alignment-band advance on a boundary without the catch-up
        // silently collapsing past it. `MAX_LEGS + 1` bounds the inner loop.
        for _ in 0..=MAX_LEGS {
            if worked >= MAX_JOURNEYS_WORKED_PER_FIRE {
                break;
            }
            match step(ctx, *root, &now, source) {
                Step::Skipped => break,
                Step::Worked => worked = worked.saturating_add(1),
            }
        }
    }
    let (next_id, cursor) = match ctx.w.module_state_mut::<JourneyBook>() {
        Some(book) => {
            let moved = last.map(|l| l.0.saturating_add(1));
            let changed = moved.is_some_and(|c| c != book.cursor);
            if let Some(c) = moved {
                book.cursor = c;
            }
            if !changed {
                return;
            }
            (book.next_id, book.cursor)
        }
        None => return,
    };
    // The rotation cursor rides the allocator document that already exists, so
    // a restart resumes the rotation instead of re-starving the tail (R11).
    persist_journey_meta(ctx.out, next_id, cursor);
    #[cfg(debug_assertions)]
    {
        let Some(st) = ctx.w.module_state::<SpatialState>() else {
            return;
        };
        let Some(book) = ctx.w.module_state::<JourneyBook>() else {
            return;
        };
        for root in &order {
            if let Some(j) = book.get(*root) {
                debug_assert!(
                    j.state.is_under_way() || st.is_placed(*root) || st.is_quarantined(*root),
                    "no journey record is terminal while its root is unplaced-and-unquarantined \
                     (root {root}, state {:?})",
                    j.state
                );
            }
        }
    }
}

/// One journey's share of one slice: **at most one leg boundary**.
fn step(ctx: &mut Ctx<'_>, root: ObjId, now: &ClockStamp, source: Source) -> Step {
    let Some(journey) = ctx
        .w
        .module_state::<JourneyBook>()
        .and_then(|b| b.get(root).cloned())
    else {
        return Step::Skipped;
    };

    // Terminal states: the only things left are B3's crash-gap heal — an
    // ARRIVED journey whose carrier never got where the record says, and whose
    // repair claim is still live (D3) — and D1's backstop, a terminal record
    // that left its root loose in the graph.
    if !journey.state.is_under_way() {
        let (heal, loose) = match ctx.w.module_state::<SpatialState>() {
            Some(st) => (heal_pending(st, &journey), loose_terminal(st, &journey)),
            None => (false, false),
        };
        if heal {
            tracing::warn!(
                root = %root,
                journey = %journey.journey,
                "sim: arrived carrier journey found off its destination; re-applying the move"
            );
            return apply_arrival(ctx, journey, now, source);
        }
        if loose {
            // A terminal journey never leaves a live crewed hull takeable by a
            // bystander: it is held where staff can find it (D1).
            return hold_stopped_root(ctx, root);
        }
        return Step::Skipped;
    }

    // A world rate-generation change must never pause a journey (D-E7-9): it
    // re-anchors, preserving the IC duration still to run, and keeps going.
    if journey.departed.clock != now.clock {
        return reanchor(ctx, journey, now);
    }

    let Some(due) = journey.due_cursor(now.at) else {
        return fail(
            ctx,
            journey,
            "a leg boundary overflowed i64; the journey is stopped, not silently stalled",
        );
    };
    if due <= journey.cursor {
        return Step::Skipped;
    }
    // ONE boundary (R13): the catch-up after a downtime observes every leg it
    // crossed, one slice each, instead of collapsing them into a single jump.
    let Some(target) = journey.cursor.checked_add(1) else {
        return fail(
            ctx,
            journey,
            "the leg cursor overflowed; the journey is stopped, not silently stalled",
        );
    };
    let total = u32::try_from(journey.route.legs().len()).unwrap_or(u32::MAX);
    let mut advanced = journey;
    advanced.cursor = target;
    advanced.advanced_at = now.clone();
    if target >= total {
        // Arrival: the whole batch — the journey document, the carrier-root
        // subtree move and the onboard scene floors — lands in one Outbox.
        return apply_arrival(ctx, advanced, now, source);
    }
    // An intermediate boundary changes no placement: a cruising root is not
    // placed anywhere, so only the durable cursor moves.
    persist_journey(ctx.out, &advanced);
    if let Some(book) = ctx.w.module_state_mut::<JourneyBook>() {
        book.journeys.insert(root, advanced);
    }
    Step::Worked
}

/// Latch a stopped journey's root into the recovery room (E7 cut 6 fold, D1),
/// and re-derive the crew's links against the settled truth (C5).
fn hold_stopped_root(ctx: &mut Ctx<'_>, root: ObjId) -> Step {
    match crate::transition::set_carrier_root_quarantine(
        ctx,
        root,
        true,
        "its carrier journey stopped; staff must place it (@journey/divert or @spatial/place/domain)",
    ) {
        Ok(()) => {
            refresh_carrier_links(ctx, root);
            Step::Worked
        }
        Err(error) => {
            tracing::error!(
                %root,
                %error,
                "sim: a stopped carrier journey's root could not be held. Recommended: \
                 @spatial/place/domain the carrier by hand"
            );
            Step::Skipped
        }
    }
}

/// Whether the carrier root's placement already matches the journey's recorded
/// destination, compared on **structural identity** (domain / space+slot /
/// host+mount) rather than on an opaque pose.
fn carrier_is_at_destination(ctx: &Ctx<'_>, journey: &CarrierJourney) -> bool {
    ctx.w
        .module_state::<SpatialState>()
        .is_some_and(|st| same_place(&st.parent_of(journey.root), &journey.destination))
}

/// Structural equality of two placements, ignoring opaque poses.
#[must_use]
pub fn same_place(a: &SpatialParent, b: &SpatialParent) -> bool {
    match (a, b) {
        (SpatialParent::Direct { domain: x, .. }, SpatialParent::Direct { domain: y, .. }) => {
            x == y
        }
        (
            SpatialParent::Interior { space: x, slot: sx },
            SpatialParent::Interior { space: y, slot: sy },
        ) => x == y && sx == sy,
        (
            SpatialParent::Attached {
                host: x, mount: mx, ..
            },
            SpatialParent::Attached {
                host: y, mount: my, ..
            },
        ) => x == y && mx == my,
        (SpatialParent::Unplaced, SpatialParent::Unplaced) => true,
        _ => false,
    }
}

/// Re-anchor a journey onto the current world-IC generation, preserving the IC
/// duration still to run. Journeys **never pause** (D-E7-9) — deliberately the
/// opposite of production's default — so a rate change re-bases the departure
/// stamp instead of stranding a stamp nobody may compare.
fn reanchor(ctx: &mut Ctx<'_>, journey: CarrierJourney, now: &ClockStamp) -> Step {
    // **Relabel, do not re-base** (E7 cut 6 fold, R2). The world IC axis is
    // CONTINUOUS across a generation — `WorldTimeline::next_rate_epoch` sets
    // `ic_anchor = current.ic_at(real_anchor)` — so a stamp's `at` is directly
    // comparable across the two generations and only the generation *label*
    // changes. Keeping `departed.at` exactly as it is therefore preserves every
    // boundary and the arrival stamp to the second; the old arithmetic restarted
    // the in-flight leg, so a rate change EXTENDED a journey and a bump cadence
    // shorter than a leg deferred arrival without limit. No sum, so no overflow
    // arm: the two `fail()` paths that existed only for it are gone.
    let stamp = ClockStamp {
        clock: now.clock.clone(),
        at: journey.departed.at,
    };
    let root = journey.root;
    let mut rebased = journey;
    rebased.departed = stamp;
    rebased.advanced_at = now.clone();
    tracing::info!(
        root = %root,
        journey = %rebased.journey,
        "sim: carrier journey re-anchored onto a new world-IC generation (it never pauses)"
    );
    persist_journey(ctx.out, &rebased);
    if let Some(book) = ctx.w.module_state_mut::<JourneyBook>() {
        book.journeys.insert(root, rebased);
    }
    Step::Worked
}

/// Stop **only this journey**, record the diagnosis with the recommended
/// command already filled in, and page (08c §4.6: resolve programmatically,
/// page for a genuine decision, stop only the subject).
fn fail(ctx: &mut Ctx<'_>, journey: CarrierJourney, why: &str) -> Step {
    let root = journey.root;
    let mut stopped = journey;
    stopped.state = JourneyState::Failed;
    stopped.anomaly = Some(bounded_anomaly(format!(
        "{why}. Recommended: @journey/divert #{} =<route id> <place>… (or @spatial/place/domain #{} =<domain> to recover it by hand).",
        root.0, root.0
    )));
    tracing::error!(
        root = %root,
        journey = %stopped.journey,
        anomaly = stopped.anomaly.as_deref().unwrap_or_default(),
        "sim: carrier journey stopped and paged"
    );
    persist_journey(ctx.out, &stopped);
    if let Some(book) = ctx.w.module_state_mut::<JourneyBook>() {
        book.journeys.insert(root, stopped);
    }
    // **A stopped ship is never loose** (E7 cut 6 fold, D1): the root is held
    // in the recovery room, so `is_quarantined` answers, `placed_or_quarantined`
    // covers it and no bystander standing where it departed from can pick a
    // crewed dropship up. Then the links are re-derived against that settled
    // truth (C5) — every writer of a `controls_are_live` input refreshes, which
    // is what stops a stranded crew holding a grant the projection has dropped.
    let held = !ctx
        .w
        .module_state::<SpatialState>()
        .is_some_and(|st| st.is_placed(root));
    if held
        && let Err(error) = crate::transition::set_carrier_root_quarantine(
            ctx,
            root,
            true,
            "its carrier journey stopped; staff must place it (@journey/divert or @spatial/place/domain)",
        )
    {
        tracing::error!(
            %root,
            %error,
            "sim: a stopped carrier journey's root could not be held; it is unplaced and \
             ungated. Recommended: @spatial/place/domain the carrier by hand"
        );
    }
    refresh_carrier_links(ctx, root);
    Step::Worked
}

fn bounded_anomaly(mut text: String) -> String {
    if text.len() > MAX_JOURNEY_ANOMALY_BYTES {
        let mut end = MAX_JOURNEY_ANOMALY_BYTES;
        while end > 0 && !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        text.truncate(end);
    }
    text
}

/// Commit an arrival: the ONE checked carrier move, then the durable record,
/// then the typed arrival event, then each onboard scene's IC floor. In that
/// order (S6A), and a scene **cannot delay** it — the floor raise is the last,
/// non-fatal step and a refusal pages instead of holding the journey.
fn apply_arrival(
    ctx: &mut Ctx<'_>,
    journey: CarrierJourney,
    now: &ClockStamp,
    source: Source,
) -> Step {
    let root = journey.root;
    let destination = journey.destination.clone();
    // Idempotent: an arrival already applied moves nothing and refuses nothing.
    let already = carrier_is_at_destination(ctx, &journey);
    // Read where it came FROM before the move, so the `Left` half of the event
    // pairs with the `Joined` (E7 cut 6 fold, R9(2)): `from: None` left scenes
    // recording arrivals with no matching departure and never fired
    // `mush-script`'s `on_leave`.
    let from = ctx.w.get(root).and_then(|o| o.location);
    if !already
        && let Err(error) =
            crate::transition::move_carrier_subtree(ctx, root, Some(destination.clone()))
    {
        return fail(
            ctx,
            journey,
            &format!("the carrier could not be placed at its destination ({error})"),
        );
    }
    let arrival_at = journey.arrival_at().unwrap_or(now.at);
    let applied_revision = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|st| st.placements.get(&root))
        .map_or(0, |p| p.revision);
    let mut arrived = journey;
    arrived.state = JourneyState::Arrived;
    arrived.advanced_at = now.clone();
    arrived.anomaly = None;
    // D3: the repair claim is scoped to the revision this arrival landed at, so
    // a legitimate later placement of the carrier ends it instead of being
    // reverted — whole subtree included — on the next 1 s fire, forever.
    arrived.applied_revision = applied_revision;
    persist_journey(ctx.out, &arrived);
    if let Some(book) = ctx.w.module_state_mut::<JourneyBook>() {
        book.journeys.insert(root, arrived);
    }
    // Bump then refresh: the record now says Arrived and the root is placed
    // again, so the crew's grants are re-derived against the settled truth.
    refresh_carrier_links(ctx, root);
    // **The post-move steps are NOT inside the `already` short-circuit**
    // (E7 cut 6 fold, R6). A replayed arrival — a journey document older than
    // its placement document — used to skip the event, the narrative AND the
    // onboard IC-floor raise entirely, and nothing else ever raised it. The
    // floor raise is monotonic, so running it again is a HIT; only the two
    // audience-facing halves are conditional.
    if !already {
        // The typed arrival event, on the ONE `Moved` world event the tree
        // already has (cut 6 adds no `WorldEvent` variant — RESOLUTION A).
        // Never from boot: `emit_world_event` dispatches INLINE to observers,
        // and script/npc/weather are composed after `mush-sim` and have not
        // hydrated when `on_world_loaded` runs (R9(1)).
        if let Some(to) = ctx.w.get(root).and_then(|o| o.location) {
            if source == Source::Tick {
                ctx.emit_world_event(&WorldEvent::Moved {
                    who: root,
                    from,
                    to,
                });
            }
            ctx.out.to_room_from(
                to,
                vec![root],
                root,
                OutputEvent::Narrative {
                    kind: NarrativeKind::Emit,
                    actor: None,
                    text: RichText::new()
                        .span(name_span(ctx.w, root))
                        .text(" arrives, and the passage ends."),
                },
            );
        }
    }
    raise_onboard_scene_floors(ctx, root, arrival_at);
    Step::Worked
}

/// Raise every onboard scene's IC floor to the arrival stamp through E6b's one
/// seam, [`mush_scenes::SceneRecord::raise_ic_floor_on_arrival`] (bounded by
/// `MAX_IC_FLOOR_LEAD_SECS`, refused after the seal, monotonic, and it never
/// touches `clock_generation`). A refusal is **paged with its numbers** and
/// never holds the journey (Q6A/S6A).
fn raise_onboard_scene_floors(ctx: &mut Ctx<'_>, root: ObjId, at: i64) {
    let world_now = ctx.w.world_ic_now().at;
    let rooms: Vec<ObjId> = {
        let Some(st) = ctx.w.module_state::<SpatialState>() else {
            return;
        };
        let mut set: BTreeSet<ObjId> = BTreeSet::new();
        for o in st.subtree_truncated(root, MAX_CARRIER_SUBTREE) {
            if let Some(space) = st.spaces.get(&o) {
                set.insert(space.object);
                set.extend(space.rooms.iter().copied());
            }
        }
        set.into_iter().collect()
    };
    if rooms.is_empty() {
        return;
    }
    let Some(scenes) = ctx.w.module_state::<mush_scenes::ScenesState>() else {
        // No scenes module in this composition: nothing to raise.
        return;
    };
    let targets: Vec<mush_scenes::SceneUid> = rooms
        .iter()
        .filter_map(|&room| scenes.active_scene_at(room).map(|s| s.id))
        .collect();
    let mut raised: Vec<mush_scenes::SceneRecord> = Vec::new();
    let mut refusals: Vec<(mush_scenes::SceneUid, mush_scenes::ArrivalFloorRefusal)> = Vec::new();
    if let Some(scenes) = ctx.w.module_state_mut::<mush_scenes::ScenesState>() {
        for uid in targets {
            let Some(record) = scenes.scenes.get_mut(&uid) else {
                continue;
            };
            match record.raise_ic_floor_on_arrival(at, world_now) {
                Ok(_) => raised.push(record.clone()),
                Err(refusal) => refusals.push((uid, refusal)),
            }
        }
    }
    for record in &raised {
        mush_scenes::persist_scene_record(ctx.out, record);
    }
    for (uid, refusal) in refusals {
        // **A repeated raise is a HIT, not a page** (E7 cut 6 fold, R10). The
        // floor is monotonic, so re-running an arrival at the same stamp is
        // refused `NotForward` with a floor already at or past it — that is the
        // repair working, not a decision a human owes anyone. Only the two arms
        // that genuinely need a person page: the stamp is impossible
        // (`AheadOfWorld`) or the scene's IC time is canon (`AlreadySealed`).
        if let mush_scenes::ArrivalFloorRefusal::NotForward { at: refused, floor } = refusal
            && floor >= refused
        {
            tracing::debug!(
                root = %root,
                scene = uid.0,
                floor,
                arrival_at = at,
                "sim: an onboard scene's IC floor is already at or past the arrival stamp (a hit)"
            );
            continue;
        }
        tracing::error!(
            root = %root,
            scene = uid.0,
            ?refusal,
            arrival_at = at,
            "sim: an onboard scene refused the arrival IC-floor raise; the journey is NOT held. \
             Recommended: @scene/icfloor <scene>=<stamp> once staff have decided the stamp"
        );
    }
}

// ----------------------------------------------------------------------
// Lifecycle
// ----------------------------------------------------------------------

/// Re-derive the control links of every actor crewing anything in `root`'s
/// subtree — the **refresh** half of cut 5's bump-then-refresh discipline, run
/// *after* the durable truth (the placement and the journey record) has changed,
/// so `links::controls_are_live` sees the state it is meant to answer for.
///
/// Departure needs this on top of the refresh `move_carrier_subtree` already
/// does: the move happens before the journey record exists, so at that instant
/// the root is neither placed nor under way and every grant is correctly
/// dropped — this is the call that gives the crew the helm back one line later.
pub(crate) fn refresh_carrier_links(ctx: &mut Ctx<'_>, root: ObjId) {
    let subtree: Vec<ObjId> = ctx
        .w
        .module_state::<SpatialState>()
        .map(|st| st.subtree_truncated(root, MAX_CARRIER_SUBTREE))
        .unwrap_or_default();
    for unit in subtree {
        crate::links::refresh_unit_actors(ctx.w, unit);
    }
}

/// A destroyed carrier root drops its journey (the mirror of
/// `route::on_traveler_destroyed`).
pub(crate) fn on_root_destroyed(ctx: &mut Ctx<'_>, obj: ObjId) {
    let removed = ctx
        .w
        .module_state_mut::<JourneyBook>()
        .is_some_and(|b| b.journeys.remove(&obj).is_some());
    if removed {
        persist_journey_del(ctx.out, obj);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::passive_rules_value;
    use crate::route::LegDef;

    fn cid(s: &str) -> CanonicalId {
        CanonicalId::new(s).expect("a well-formed canonical id")
    }

    fn stamp(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at,
        }
    }

    fn corridor() -> RouteDef {
        RouteDef::new(
            cid("ceres_phobos"),
            (ObjId(10), ObjId(12)),
            vec![
                LegDef::new(ObjId(11), 100).expect("a positive leg"),
                LegDef::new(ObjId(12), 200).expect("a positive leg"),
            ],
            None,
            cid("fusion_torch"),
            0,
        )
        .expect("a coherent route")
    }

    fn dest() -> SpatialParent {
        SpatialParent::Direct {
            domain: DomainId(0),
            pose: passive_rules_value(serde_json::json!({})).expect("a canonical pose"),
        }
    }

    fn journey(cursor: u32, state: JourneyState) -> CarrierJourney {
        CarrierJourney::new(
            JourneyId(0),
            ObjId(7),
            corridor(),
            dest(),
            stamp(1_000),
            stamp(1_000),
            cursor,
            state,
            None,
        )
        .expect("a coherent journey")
    }

    /// The leg cursor IS the idempotency key: boundaries are absolute WorldIc
    /// positions off `departed`, and a position before the next boundary
    /// advances nothing.
    #[test]
    fn e7c6_b2_the_leg_cursor_is_the_idempotency_key() {
        let j = journey(0, JourneyState::Cruising);
        assert_eq!(j.leg_due_at(0), Some(1_100));
        assert_eq!(j.leg_due_at(1), Some(1_300));
        assert_eq!(j.leg_due_at(2), None);
        assert_eq!(j.arrival_at(), Some(1_300));
        assert_eq!(j.due_cursor(1_099), Some(0));
        assert_eq!(j.due_cursor(1_100), Some(1));
        assert_eq!(j.due_cursor(1_299), Some(1));
        assert_eq!(j.due_cursor(1_300), Some(2));
        assert_eq!(j.due_cursor(9_999), Some(2), "never past the last leg");
        // Already at the end: nothing further is due, however far the clock ran.
        let done = journey(2, JourneyState::Arrived);
        assert_eq!(done.due_cursor(9_999), Some(2));
    }

    /// A `due` sum that leaves `i64` yields `None` so the caller PAGES rather
    /// than stalling silently (E5a S2).
    #[test]
    fn e7c6_b2_a_leg_boundary_overflow_is_visible_not_silent() {
        let j = CarrierJourney::new(
            JourneyId(1),
            ObjId(7),
            corridor(),
            dest(),
            stamp(i64::MAX - 10),
            stamp(i64::MAX - 10),
            0,
            JourneyState::Cruising,
            None,
        )
        .expect("a coherent journey");
        assert_eq!(j.leg_due_at(0), None);
        assert_eq!(j.due_cursor(i64::MAX), None);
    }

    /// The wire re-runs every constructor invariant, and `RouteDefWire` re-runs
    /// the leg cap it owns, so a stored record past a cap is refused on load
    /// rather than truncated (§4 rule 1).
    #[test]
    fn e7c6_b2_the_wire_re_runs_max_legs_and_every_invariant() {
        let good = serde_json::to_string(&journey(1, JourneyState::Cruising)).expect("serializes");
        let round_trip: CarrierJourney = serde_json::from_str(&good).expect("re-enters");
        assert_eq!(round_trip, journey(1, JourneyState::Cruising));

        // A cursor past the route's legs.
        assert!(matches!(
            CarrierJourney::new(
                JourneyId(0),
                ObjId(7),
                corridor(),
                dest(),
                stamp(0),
                stamp(0),
                3,
                JourneyState::Cruising,
                None,
            ),
            Err(CarrierJourneyError::CursorPastEnd { cursor: 3, legs: 2 })
        ));
        // Two stamps on different generations.
        assert_eq!(
            CarrierJourney::new(
                JourneyId(0),
                ObjId(7),
                corridor(),
                dest(),
                stamp(0),
                ClockStamp {
                    clock: ClockRef::WorldIc { generation: 2 },
                    at: 0
                },
                0,
                JourneyState::Cruising,
                None,
            ),
            Err(CarrierJourneyError::ClockMismatch)
        );
        // Any clock but the world IC timeline.
        assert_eq!(
            CarrierJourney::new(
                JourneyId(0),
                ObjId(7),
                corridor(),
                dest(),
                ClockStamp {
                    clock: ClockRef::Utc,
                    at: 0
                },
                ClockStamp {
                    clock: ClockRef::Utc,
                    at: 0
                },
                0,
                JourneyState::Cruising,
                None,
            ),
            Err(CarrierJourneyError::NotWorldIc)
        );
        // A destination a carrier cannot come to rest at.
        assert_eq!(
            CarrierJourney::new(
                JourneyId(0),
                ObjId(7),
                corridor(),
                SpatialParent::Unplaced,
                stamp(0),
                stamp(0),
                0,
                JourneyState::Cruising,
                None,
            ),
            Err(CarrierJourneyError::UnplacedDestination)
        );
        // An over-long diagnosis.
        assert_eq!(
            CarrierJourney::new(
                JourneyId(0),
                ObjId(7),
                corridor(),
                dest(),
                stamp(0),
                stamp(0),
                0,
                JourneyState::Cruising,
                Some("x".repeat(MAX_JOURNEY_ANOMALY_BYTES + 1)),
            ),
            Err(CarrierJourneyError::AnomalyTooLong)
        );
        // A stored route past MAX_LEGS is refused by the wire, never truncated.
        let mut doc = serde_json::to_value(journey(0, JourneyState::Cruising)).expect("serializes");
        let legs: Vec<serde_json::Value> = (0..=MAX_LEGS)
            .map(|i| serde_json::json!({ "to": 100 + i, "base_s": 1 }))
            .collect();
        doc["route"]["legs"] = serde_json::Value::Array(legs);
        doc["route"]["endpoints"] = serde_json::json!([10, 100 + MAX_LEGS]);
        let refused = serde_json::from_value::<CarrierJourney>(doc).expect_err("past MAX_LEGS");
        assert!(refused.to_string().contains("maximum"), "{refused}");
    }

    /// `is_under_way` is the ONE predicate the seal, the gate and the tick share,
    /// so they can never disagree about what "moving" means.
    #[test]
    fn e7c6_b3_under_way_is_one_predicate() {
        assert!(JourneyState::Cruising.is_under_way());
        assert!(
            JourneyState::Promoted {
                domain: DomainId(1)
            }
            .is_under_way()
        );
        assert!(
            JourneyState::Diverted {
                to: cid("other_route")
            }
            .is_under_way()
        );
        assert!(!JourneyState::Arrived.is_under_way());
        assert!(!JourneyState::Captured { by: ObjId(3) }.is_under_way());
        assert!(!JourneyState::Failed.is_under_way());
    }

    /// Structural placement equality ignores an opaque pose, so a re-derived
    /// arrival never looks "wrong" because a driver stamped a different pose.
    #[test]
    fn e7c6_b3_same_place_compares_identity_not_pose() {
        let a = SpatialParent::Direct {
            domain: DomainId(0),
            pose: passive_rules_value(serde_json::json!({ "x": 1 })).expect("pose"),
        };
        let b = SpatialParent::Direct {
            domain: DomainId(0),
            pose: passive_rules_value(serde_json::json!({ "x": 2 })).expect("pose"),
        };
        assert!(same_place(&a, &b));
        assert!(!same_place(
            &a,
            &SpatialParent::Direct {
                domain: DomainId(1),
                pose: passive_rules_value(serde_json::json!({})).expect("pose"),
            }
        ));
        assert!(!same_place(&a, &SpatialParent::Unplaced));
    }
}
