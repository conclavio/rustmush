//! # Route definitions (E7 cut 1, 10b §3.1)
//!
//! The **definition** a carrier passage is planned from: [`RouteDef`], its [`LegDef`]s
//! with their hazard and interdiction bands, [`DriveProfile`], the one pure evaluator
//! [`route_duration`], and the [`RouteCatalog`] seam the composition root installs so
//! `@journey/depart` resolves an authored route id. The type lives here rather than
//! in `ruleset-tacsim` because every ruleset depends on `mush-sim` (the crate owns the
//! state machine, the ruleset provides the data).
//!
//! The legacy room-to-room **runtime** this file once carried (`Journey`, `Leg`, the
//! sweep, `@route` and `+intercept`) was retired by S2 of the spatial plan
//! (14-spatial-implementation-plan §3.2): travel for characters, objects and
//! room-model vehicles is `mush-atlas`'s one travel system now. The carrier journey
//! (`journey.rs`) stays until S7's TacSim fold.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

/// Default per-leg seconds when a `@journey/depart` token gives no explicit time.
pub const DEFAULT_TRAVEL_S: u32 = 30;

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
    /// **E7r1 R2** — the leg's authored hazard **band**, `0..=MAX_HAZARD_BAND`.
    hazard: u8,
    /// **E7r1 R2** — the interdiction **threshold**, `0..=MAX_HAZARD_BAND`.
    interdiction: u8,
}

/// The strict wire form of [`LegDef`].
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegDefWire {
    /// The place reached at the end of this leg.
    pub to: ObjId,
    /// Unladen seconds at nominal thrust.
    pub base_s: u32,
    /// The authored hazard band. `#[serde(default)]`, so every route stored before
    /// E7 residue pass 1 re-enters as an unhazardous leg rather than being refused —
    /// and `skip_serializing_if` (independent verify fold, B14), so an **unauthored**
    /// leg encodes byte-identically to the pre-pass form.
    ///
    /// 10b §15.1 row 7 claimed *"all three are `serde(default)` AND
    /// `skip_serializing_if`"*; that was true of the **pack** types (`PackLeg`,
    /// `RouteEntry`), whose skip is `tacsim.corridor@1`'s digest-load-bearing, and
    /// false of these two sim wires, which gained only the default. So every stored
    /// route document and every `cjourney:<root>` embedding a [`RouteDefWire`] began
    /// writing `hazard: 0`, `interdiction: 0` and `controller: null` where it had
    /// written nothing — no digest covers those documents, so nothing broke, but a
    /// rollback to the pre-pass build would have failed `deny_unknown_fields` on
    /// every one written since. The row's sentence is now simply true.
    #[serde(default, skip_serializing_if = "is_unbanded")]
    pub hazard: u8,
    /// The interdiction threshold; zero means *this leg is never interdicted*.
    #[serde(default, skip_serializing_if = "is_unbanded")]
    pub interdiction: u8,
}

/// An unauthored band, which serializes as absent (B14). The pack types' own
/// `is_unbanded`, restated here because `route.rs` may take no `ruleset-tacsim` edge.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_unbanded(band: &u8) -> bool {
    *band == 0
}

/// The widest hazard band and interdiction threshold a pack may author
/// (**E7r1 R2**). Bands are a small ordered vocabulary, not a scale: a pack that
/// wants finer resolution is authoring a damage model, which is post-E7 (§15.3
/// item 18). A stored or pack-authored leg past it is **refused on load, never
/// clamped**.
pub const MAX_HAZARD_BAND: u8 = 5;

impl LegDef {
    /// The one constructor, and therefore the one place the invariants live.
    ///
    /// # Errors
    ///
    /// [`RouteDefError::ZeroLeg`] when `base_s` is zero — a leg that takes no time
    /// is an instantaneous relocation, which is exactly what design 07 §9's route
    /// invariant forbids — and [`RouteDefError::HazardBand`] for a band or a
    /// threshold past [`MAX_HAZARD_BAND`].
    pub fn new(to: ObjId, base_s: u32) -> Result<LegDef, RouteDefError> {
        LegDef::with_hazard(to, base_s, 0, 0)
    }

    /// [`LegDef::new`] with the corridor's authored hazard band and interdiction
    /// threshold (**E7r1 R2**).
    ///
    /// # Errors
    ///
    /// [`RouteDefError::ZeroLeg`], or [`RouteDefError::HazardBand`] past
    /// [`MAX_HAZARD_BAND`].
    pub fn with_hazard(
        to: ObjId,
        base_s: u32,
        hazard: u8,
        interdiction: u8,
    ) -> Result<LegDef, RouteDefError> {
        if base_s == 0 {
            return Err(RouteDefError::ZeroLeg);
        }
        if hazard > MAX_HAZARD_BAND || interdiction > MAX_HAZARD_BAND {
            return Err(RouteDefError::HazardBand {
                max: MAX_HAZARD_BAND,
            });
        }
        Ok(LegDef {
            to,
            base_s,
            hazard,
            interdiction,
        })
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

    /// The authored hazard band (**E7r1 R2**).
    #[must_use]
    pub fn hazard(&self) -> u8 {
        self.hazard
    }

    /// The interdiction threshold; zero is *never interdicted* (**E7r1 R2**).
    #[must_use]
    pub fn interdiction(&self) -> u8 {
        self.interdiction
    }

    /// **Whether crossing this leg's boundary raises an interception.**
    ///
    /// A pure function of two authored numbers and of nothing else: no clock, no
    /// world, no draw. Two evaluations of one leg always agree, which is what lets
    /// the journey tick's boundary be idempotent under a replay (the leg cursor is
    /// the key, and this predicate cannot disagree with itself between fires).
    #[must_use]
    pub fn is_interdicted(&self) -> bool {
        self.interdiction > 0 && self.hazard >= self.interdiction
    }
}

impl From<LegDef> for LegDefWire {
    fn from(value: LegDef) -> LegDefWire {
        LegDefWire {
            to: value.to,
            base_s: value.base_s,
            hazard: value.hazard,
            interdiction: value.interdiction,
        }
    }
}

impl TryFrom<LegDefWire> for LegDef {
    type Error = RouteDefError;

    fn try_from(value: LegDefWire) -> Result<LegDef, RouteDefError> {
        LegDef::with_hazard(value.to, value.base_s, value.hazard, value.interdiction)
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
    /// **E7r1 R2** — who holds the corridor, as a pack-authored symbolic id.
    ///
    /// `None` is an uncontrolled corridor. It is deliberately a `CanonicalId` and
    /// not an `ObjId` or an `OrgScopeId`: a route definition is pinned by a
    /// published pack (T1), and a definition that carried a world id could not be
    /// pinned across a re-seed. Nothing in `mush-sim` resolves it — it is a name
    /// this crate renders and never interprets.
    controller: Option<mush_core::CanonicalId>,
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
    /// **E7r1 R2** — the corridor's controller, if any. `#[serde(default)]`, so
    /// every route stored before the residue pass re-enters uncontrolled, and
    /// `skip_serializing_if` (B14) so an uncontrolled route encodes no `controller`
    /// key at all rather than an explicit `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<mush_core::CanonicalId>,
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
        RouteDef::controlled(
            id,
            endpoints,
            legs,
            alignment_band,
            drive_class,
            propellant,
            None,
        )
    }

    /// [`RouteDef::new`] with the corridor's pack-authored controller
    /// (**E7r1 R2**).
    ///
    /// # Errors
    ///
    /// Exactly [`RouteDef::new`]'s.
    #[allow(clippy::too_many_arguments)]
    pub fn controlled(
        id: mush_core::CanonicalId,
        endpoints: (ObjId, ObjId),
        legs: Vec<LegDef>,
        alignment_band: Option<CadenceId>,
        drive_class: mush_core::CanonicalId,
        propellant: u64,
        controller: Option<mush_core::CanonicalId>,
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
            controller,
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

    /// The corridor's pack-authored controller, if any (**E7r1 R2**). A name this
    /// crate renders and never resolves.
    #[must_use]
    pub fn controller(&self) -> Option<&mush_core::CanonicalId> {
        self.controller.as_ref()
    }

    /// Whether any leg of this corridor is interdicted (**E7r1 R2**) — the one
    /// question `@corridor/rules` answers without disclosing a leg's own band.
    #[must_use]
    pub fn is_interdicted(&self) -> bool {
        self.legs.iter().any(LegDef::is_interdicted)
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
            controller: value.controller,
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
        RouteDef::controlled(
            value.id,
            value.endpoints,
            legs,
            value.alignment_band,
            value.drive_class,
            value.propellant,
            value.controller,
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

// ---- E7c7: B6-carried (cut 6 fold, B6 — the composition-installed route catalog) ----

/// The most bytes a route id may carry on the command line before it is
/// refused rather than hashed, matched or logged.
pub const MAX_ROUTE_ID_BYTES: usize = 64;

/// Resolve a typed route id against whatever authored the packs.
///
/// Cut 6's fold found `@journey/plan|depart <route id>` to be a **label only**:
/// `build_route` fabricated a fresh `RouteDef` at the `manual` drive class with
/// zero declared propellant and no alignment band, so `RouteDef::propellant()`,
/// `alignment_band()` and `route_duration` had no caller. The verify's proposed
/// fix — "resolve the id against cut 1's pack" — **cannot be an edge**:
/// `ruleset-tacsim` depends on `mush-sim`, so `mush-sim -> ruleset-tacsim` is a
/// cycle. It is therefore a composition seam in the
/// [`SimulationModule::with_accounts`](crate::SimulationModule::with_accounts) /
/// `IndustryModule::with_recipes` shape: the implementation lives at the
/// composition root (`mush_server::sim_routes`), reads the ruleset's own
/// published store, and hands back a `RouteDef` that has already re-run every
/// constructor invariant.
///
/// **Bounded and read-only.** One id in, at most one definition out; the
/// implementation performs no write, no allocation past the returned value and
/// no unbounded walk (the store's index is a `BTreeMap` lookup).
pub trait RouteCatalog: Send + Sync {
    /// The authored route this id names, or `None` when nothing authoritative
    /// answers for it — which is exactly "fall back to the manual fixture
    /// route", never "invent one".
    fn resolve(&self, world: &mush_core::World, id: &mush_core::CanonicalId) -> Option<RouteDef>;
}

/// A newtype over `Option<Arc<dyn RouteCatalog>>` so `SimulationModule` can carry
/// the composition-installed catalog into world state and command handlers can
/// reach it synchronously through `World::module_state::<RouteCatalogState>()`.
/// `None` is an unwired composition (a test rig, an old wiring) and every
/// consumer **falls back to the manual route**, never to a fabricated one.
#[derive(Clone, Default)]
pub struct RouteCatalogState(Option<std::sync::Arc<dyn RouteCatalog>>);

impl std::fmt::Debug for RouteCatalogState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouteCatalogState")
            .field("wired", &self.0.is_some())
            .finish()
    }
}

impl RouteCatalogState {
    /// The wired catalog.
    #[must_use]
    pub fn new(catalog: std::sync::Arc<dyn RouteCatalog>) -> Self {
        Self(Some(catalog))
    }

    /// No catalog: every lookup answers `None`.
    #[must_use]
    pub const fn unwired() -> Self {
        Self(None)
    }

    /// Whether a catalog is installed at all — the composition probe.
    #[must_use]
    pub const fn is_wired(&self) -> bool {
        self.0.is_some()
    }

    /// Resolve one id, or `None` when unwired or unknown.
    #[must_use]
    pub fn resolve(
        &self,
        world: &mush_core::World,
        id: &mush_core::CanonicalId,
    ) -> Option<RouteDef> {
        if id.as_str().len() > MAX_ROUTE_ID_BYTES {
            return None;
        }
        self.0.as_ref().and_then(|c| c.resolve(world, id))
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
    /// **E7r1 R2** — a hazard band or interdiction threshold past
    /// [`MAX_HAZARD_BAND`]. Refused on load, never clamped.
    #[error("a route leg's hazard band and interdiction threshold are at most {max}")]
    HazardBand {
        /// [`MAX_HAZARD_BAND`].
        max: u8,
    },
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
}
