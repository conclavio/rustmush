//! Durable records of the spatial model (design 06 §5–§6) and their
//! persistence: one JSON document per record under the `sim` namespace.
//!
//! The placement graph is the truth; everything in [`Derived`] is a
//! rebuildable projection of it and is never persisted.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use mush_core::{
    ChannelId, ClockRef, ClockStamp, Deferred, DefinitionDescriptor, DefinitionId, DefinitionKind,
    DefinitionRef, DefinitionRegistryBuilder, DefinitionValidationContext, LIMBO, Ledger, ModuleOp,
    ModuleWrite, ObjId, Outbox, OwnerId, RulesValueError, World,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

pub use mush_core::RulesValue;

/// The module namespace (`mod:sim:*`).
pub const NS: &str = "sim";
/// The allocator doc.
pub const META_KEY: &str = "meta";
/// The only driver cut 1 accepts: a passive domain that holds placements and
/// never ticks (design §6.2 `DomainClock::Passive`).
pub const PASSIVE_DRIVER: &str = "sim.passive";
const PASSIVE_DEFINITION_KIND: &str = "schema";
const PASSIVE_DEFINITION_ID: &str = "passive";
const PASSIVE_DEFINITION_VERSION: u32 = 1;
static PASSIVE_DEFINITION_REF: OnceLock<DefinitionRef> = OnceLock::new();
/// Bound on the placement/anchor ancestor walk (design §5.2 #2).
pub const MAX_SPATIAL_DEPTH: usize = 8;
/// The recovery audit ledger's log id (design §12.1 "audited").
pub const RECOVERY_LOG: &str = "recovery";

// ----------------------------------------------------------------------
// Ids
// ----------------------------------------------------------------------

/// A durable domain id — monotonic, never reused (design D2).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DomainId(pub u64);

impl std::fmt::Display for DomainId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A persisted monotonic counter cannot advance without wrapping.
///
/// Callers must either refuse the requested change before mutating anything,
/// or (only for a terminal safety transition) deliberately retain the maximum
/// value and make the record permanently non-reactivatable.
#[derive(Copy, Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpatialCounterError {
    /// `u64::MAX` is reserved as the exhausted domain-id cursor because there
    /// is no representable cursor after allocating it.
    #[error("the domain id allocator is exhausted")]
    DomainIdExhausted,
    /// A domain record cannot represent another revision.
    #[error("domain #{0} has exhausted its revision counter")]
    DomainRevisionExhausted(DomainId),
    /// A placement record cannot represent another revision.
    #[error("{0} has exhausted its placement revision counter")]
    PlacementRevisionExhausted(ObjId),
    /// A control host cannot represent another lease/control epoch.
    #[error("{0} has exhausted its control generation")]
    ControlGenerationExhausted(ObjId),
}

pub(crate) fn checked_domain_revision(
    domain: DomainId,
    current: u64,
) -> Result<u64, SpatialCounterError> {
    current
        .checked_add(1)
        .ok_or(SpatialCounterError::DomainRevisionExhausted(domain))
}

pub(crate) fn checked_placement_revision(
    object: ObjId,
    current: u64,
) -> Result<u64, SpatialCounterError> {
    current
        .checked_add(1)
        .ok_or(SpatialCounterError::PlacementRevisionExhausted(object))
}

/// A compartment/berth name within an interior space (lowercased).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SlotId(SmolStr);

/// A mount point name on a host (lowercased).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MountId(SmolStr);

/// A control-station name on a host (lowercased).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StationId(SmolStr);

macro_rules! name_id {
    ($t:ident) => {
        impl $t {
            /// A non-empty, trimmed, lowercased name.
            #[must_use]
            pub fn new(s: &str) -> Option<$t> {
                let s = s.trim().to_lowercase();
                if s.is_empty() {
                    None
                } else {
                    Some($t(SmolStr::new(s)))
                }
            }
            /// The name.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl std::fmt::Display for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
name_id!(SlotId);
name_id!(MountId);
name_id!(StationId);

// ----------------------------------------------------------------------
// Records
// ----------------------------------------------------------------------

/// The immutable definition registered for the passive driver's opaque JSON
/// values. The registrar supplies the `sim` owner; this descriptor supplies
/// the owner-local `schema:passive@1` identity and its canonical payload.
pub(crate) fn passive_definition_descriptor() -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new(PASSIVE_DEFINITION_KIND)
            .expect("the built-in passive definition kind is canonical"),
        DefinitionId::new(PASSIVE_DEFINITION_ID)
            .expect("the built-in passive definition id is canonical"),
        PASSIVE_DEFINITION_VERSION,
        serde_json::json!({ "instance": "opaque_json" }),
    )
    .expect("the built-in passive definition has a non-zero version")
    .as_current()
}

/// Provider-owned semantic validation for the `sim:schema:*` catalog. This is
/// deliberately stricter than accepting arbitrary canonical JSON: the only
/// current schema is the exact passive-driver descriptor, with no external
/// dependencies.
pub(crate) fn validate_schema_definition(
    context: &DefinitionValidationContext<'_>,
) -> Result<(), String> {
    let entry = context.definition();
    let reference = entry.reference();
    if reference.key().id.as_str() != PASSIVE_DEFINITION_ID
        || reference.version() != PASSIVE_DEFINITION_VERSION
        || entry.payload() != &serde_json::json!({ "instance": "opaque_json" })
        || !entry.references().is_empty()
        || !entry.is_current()
    {
        return Err("unknown or malformed passive schema definition".into());
    }
    Ok(())
}

/// The exact shared identity of [`PASSIVE_DRIVER`]. This deliberately builds
/// the same owner-stamped descriptor used during module composition; legacy
/// dotted schema names are never parsed to infer an owner.
#[must_use]
pub fn passive_definition_ref() -> DefinitionRef {
    PASSIVE_DEFINITION_REF
        .get_or_init(|| {
            DefinitionRegistryBuilder::new()
                .register(
                    OwnerId::new(NS).expect("the sim module namespace is canonical"),
                    passive_definition_descriptor(),
                )
                .expect("the built-in passive definition is valid")
        })
        .clone()
}

/// Wrap passive-driver data in the shared, exact-reference rules value.
pub fn passive_rules_value(data: serde_json::Value) -> Result<RulesValue, RulesValueError> {
    RulesValue::new(passive_definition_ref(), data)
}

/// Domain lifecycle (design §6.1; `Retired` per the cut-1 review: a retired
/// domain keeps its record for stable history and accepts no placements).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DomainStatus {
    /// Accepting placements.
    Active,
    /// Held; no new placements (ticking is Phase C).
    Paused,
    /// Retired: empty of references, kept for history; no placements.
    Retired,
}

// ---- E7c8: B1 — the pulse cadence ----

/// Ground/atmosphere pulse period (§7, inside S2A's 1–2 s band).
pub const GROUND_PULSE_MS: u32 = 2_000;
/// Space pulse period (§7, inside S2A's 5–10 s band).
pub const SPACE_PULSE_MS: u32 = 6_000;
/// The shortest pulse period a `Pulse` record may carry.
pub const MIN_PULSE_PERIOD_MS: u32 = 1_000;
/// The longest pulse period a `Pulse` record may carry.
pub const MAX_PULSE_PERIOD_MS: u32 = 60_000;

/// Why a [`DomainClock`] refused to be constructed or to re-enter.
///
/// Every arm is re-run by [`DomainClockWire`]'s `TryFrom`, so a stored clock
/// that could never have been written is refused **on load** rather than
/// silently widening a cap (§4 rule 1).
#[derive(Copy, Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DomainClockError {
    /// The pulse period lies outside `MIN_PULSE_PERIOD_MS..=MAX_PULSE_PERIOD_MS`.
    #[error("a domain pulse period must be {MIN_PULSE_PERIOD_MS}..={MAX_PULSE_PERIOD_MS} ms")]
    PeriodOutOfRange,
    /// Generation zero is reserved by `ClockRef` and can never be read.
    #[error("domain clock generation zero is reserved")]
    ZeroGeneration,
    /// The pulse's own epoch is stamped on the world IC timeline and nothing
    /// else — the tick counter is derived from it.
    #[error("a domain pulse epoch must be stamped on ClockRef::WorldIc")]
    NotWorldIc,
}

/// The domain clock mode (design §6.2). Cut 1 created only `Passive`;
/// **E7 cut 8 adds `Pulse`** (10b §3.8 contradiction (ii)).
///
/// `Pulse` is a *cadence*, never an authority: it says how fast the domain's
/// own `ClockRef::DomainTick` scalar advances and which generation is live. The
/// scalar itself is a pure function of the world IC clock
/// ([`DomainClock::tick_at`]), so two runs and a restart agree, which is what
/// the whole determinism lens rests on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "mode")]
pub enum DomainClock {
    /// Holds placements; never ticks.
    Passive,
    /// Resolves one deterministic domain-tick commit every `period_ms`.
    Pulse {
        /// The cadence, in milliseconds of world IC time.
        period_ms: u32,
        /// The live domain-clock generation; `ClockRef::DomainTick` names it and
        /// a read under any other generation is `StaleGeneration`.
        generation: u64,
        /// The world-IC stamp tick 0 is measured from.
        started_at: ClockStamp,
    },
}

/// The strict wire form of [`DomainClock`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum DomainClockWire {
    /// Holds placements; never ticks.
    Passive,
    /// The pulse cadence.
    Pulse {
        /// The cadence, in milliseconds of world IC time.
        period_ms: u32,
        /// The live domain-clock generation.
        generation: u64,
        /// The world-IC stamp tick 0 is measured from.
        started_at: ClockStamp,
    },
}

impl DomainClock {
    /// The one `Pulse` constructor.
    ///
    /// # Errors
    ///
    /// [`DomainClockError`] for a period outside the band, generation zero, or
    /// an epoch that is not on the world IC timeline.
    pub fn pulse(
        period_ms: u32,
        generation: u64,
        started_at: ClockStamp,
    ) -> Result<DomainClock, DomainClockError> {
        if !(MIN_PULSE_PERIOD_MS..=MAX_PULSE_PERIOD_MS).contains(&period_ms) {
            return Err(DomainClockError::PeriodOutOfRange);
        }
        if generation == 0 {
            return Err(DomainClockError::ZeroGeneration);
        }
        if !matches!(started_at.clock, ClockRef::WorldIc { .. }) {
            return Err(DomainClockError::NotWorldIc);
        }
        Ok(DomainClock::Pulse {
            period_ms,
            generation,
            started_at,
        })
    }

    /// Whether this domain resolves a tactical commit at all.
    #[must_use]
    pub const fn is_pulsing(&self) -> bool {
        matches!(self, DomainClock::Pulse { .. })
    }

    /// The live generation, or `None` for a passive domain.
    #[must_use]
    pub const fn generation(&self) -> Option<u64> {
        match self {
            DomainClock::Passive => None,
            DomainClock::Pulse { generation, .. } => Some(*generation),
        }
    }

    /// The cadence in whole IC seconds (at least one).
    #[must_use]
    pub const fn period_secs(&self) -> Option<i64> {
        match self {
            DomainClock::Passive => None,
            DomainClock::Pulse { period_ms, .. } => {
                let secs = *period_ms as i64 / 1_000;
                Some(if secs < 1 { 1 } else { secs })
            }
        }
    }

    /// **The one tick function** — the domain-tick scalar at a world IC stamp.
    ///
    /// Both the composition-registered `DomainClockProvider` and every command
    /// that needs "what tick is it?" call THIS, so a reader and the kernel can
    /// never disagree about which tick a submission belongs to.
    ///
    /// Returns `None` for a passive domain, for a stamp on a different world-IC
    /// generation than the pulse's epoch (a rate change re-anchors rather than
    /// silently re-scaling), or for a stamp before the epoch.
    #[must_use]
    pub fn tick_at(&self, world_ic: &ClockStamp) -> Option<i64> {
        let DomainClock::Pulse {
            started_at,
            period_ms,
            ..
        } = self
        else {
            return None;
        };
        if started_at.clock != world_ic.clock {
            return None;
        }
        let elapsed = world_ic.at.checked_sub(started_at.at)?;
        if elapsed < 0 {
            return None;
        }
        let period = {
            let secs = *period_ms as i64 / 1_000;
            if secs < 1 { 1 } else { secs }
        };
        elapsed.checked_div(period)
    }
}

impl From<DomainClock> for DomainClockWire {
    fn from(value: DomainClock) -> DomainClockWire {
        match value {
            DomainClock::Passive => DomainClockWire::Passive,
            DomainClock::Pulse {
                period_ms,
                generation,
                started_at,
            } => DomainClockWire::Pulse {
                period_ms,
                generation,
                started_at,
            },
        }
    }
}

impl TryFrom<DomainClockWire> for DomainClock {
    type Error = DomainClockError;

    fn try_from(value: DomainClockWire) -> Result<DomainClock, DomainClockError> {
        match value {
            DomainClockWire::Passive => Ok(DomainClock::Passive),
            DomainClockWire::Pulse {
                period_ms,
                generation,
                started_at,
            } => DomainClock::pulse(period_ms, generation, started_at),
        }
    }
}

impl<'de> Deserialize<'de> for DomainClock {
    fn deserialize<D>(deserializer: D) -> Result<DomainClock, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DomainClockWire::deserialize(deserializer)?;
        DomainClock::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// A simulation domain (design §6.1). `presentation_location` is where every
/// directly placed participant's `Obj.location` projects (design D1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainRecord {
    /// Durable id.
    pub id: DomainId,
    /// The object carrying `sim.domain`.
    pub host: ObjId,
    /// Exact immutable driver definition (cut 1: [`PASSIVE_DRIVER`] only).
    ///
    /// The legacy `"sim.passive"` plus `schema_version` pair is accepted only
    /// by the domain-record boot adapter; newly authored records always carry
    /// this complete owner/kind/id/version/digest identity.
    pub driver: DefinitionRef,
    /// Lifecycle.
    pub status: DomainStatus,
    /// Clock mode.
    pub clock: DomainClock,
    /// The room directly placed participants project into.
    pub presentation_location: ObjId,
    /// Bumped on every change.
    pub revision: u64,
    /// The window over this domain's ground, when a scene or staff opened one
    /// (doc 14 §2.4; S4). A domain with a window projects its direct participants
    /// through the registered [`RoomResolver`] and lets them walk its exits.
    #[serde(default)]
    pub window: Option<WindowRec>,
}

/// A metric pose on a domain's surface (doc 14 §2.4, D-14-4): region metres, `z`
/// per P2 (the surface's height at `(x, y)` plus the declared height above it), a
/// facing in degrees clockwise from north, and the driver-private posture — the
/// only opaque part.
///
/// The wire form is `{ x, y, z, facing_deg, posture }`; a bare rules value (the
/// cut-1 opaque pose) still decodes, as the posture of a pose at the origin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "PoseWire")]
pub struct Pose {
    /// East, region metres.
    pub x: f64,
    /// North, region metres.
    pub y: f64,
    /// Height against the region datum, metres.
    pub z: f64,
    /// Facing, degrees clockwise from north, `0..360`.
    pub facing_deg: u16,
    /// The driver's posture (opaque).
    pub posture: RulesValue,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PoseWire {
    Metric {
        x: f64,
        y: f64,
        z: f64,
        #[serde(default)]
        facing_deg: u16,
        posture: RulesValue,
    },
    Legacy(RulesValue),
}

impl From<PoseWire> for Pose {
    fn from(wire: PoseWire) -> Pose {
        match wire {
            PoseWire::Metric {
                x,
                y,
                z,
                facing_deg,
                posture,
            } => Pose {
                x,
                y,
                z,
                facing_deg: facing_deg % 360,
                posture,
            },
            PoseWire::Legacy(posture) => Pose::passive(posture),
        }
    }
}

impl Pose {
    /// A pose at the origin: the shape every cut-1 placement has.
    #[must_use]
    pub fn passive(posture: RulesValue) -> Pose {
        Pose {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            facing_deg: 0,
            posture,
        }
    }

    /// A pose at `(x, y, z)` facing north.
    #[must_use]
    pub fn at(x: f64, y: f64, z: f64, posture: RulesValue) -> Pose {
        Pose {
            x,
            y,
            z,
            facing_deg: 0,
            posture,
        }
    }

    /// Whether every coordinate is a finite number and the facing is a bearing.
    ///
    /// # Errors
    ///
    /// A message naming what is wrong.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.x.is_finite() && self.y.is_finite() && self.z.is_finite()) {
            return Err("a pose's coordinates must be finite".to_string());
        }
        if self.facing_deg >= 360 {
            return Err("a pose's facing is 0..360 degrees".to_string());
        }
        Ok(())
    }
}

/// A tactical window over a domain's ground (doc 14 §2.4): the frame of the
/// composite the atlas keeps — region, surfaces, extent and cell size — and what
/// that composite was made from. Integer centimetres, so the domain record stays
/// `Eq`; the record keys are the atlas's, in their display form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowRec {
    /// The region record the window lies in.
    pub region: SmolStr,
    /// The surfaces it composites, the ground first.
    pub surfaces: Vec<WindowSurface>,
    /// The extent, region centimetres.
    pub bbox: WindowBox,
    /// The cell size, centimetres.
    pub cell_cm: u16,
    /// The tile revisions the composite pinned.
    #[serde(default)]
    pub composited_from: Vec<WindowPin>,
    /// The feature revision the composite saw.
    #[serde(default)]
    pub features_seen: u64,
    /// Who opened it.
    pub opened_by: WindowOpener,
    /// Bumped by every re-composite.
    pub revision: u64,
}

/// A surface a window composites.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WindowSurface {
    /// The region's ground.
    Ground,
    /// A structure's floor.
    Floor {
        /// The structure record, display form.
        structure: SmolStr,
        /// The floor index.
        index: i16,
    },
}

/// An axis-aligned extent, region centimetres, `x0 < x1` and `y0 < y1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowBox {
    /// West edge.
    pub x0: i64,
    /// South edge.
    pub y0: i64,
    /// East edge.
    pub x1: i64,
    /// North edge.
    pub y1: i64,
}

/// One tile revision a composite pinned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowPin {
    /// The tile record, display form.
    pub tile: SmolStr,
    /// Its revision.
    pub revision: u64,
}

/// Who opened a window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WindowOpener {
    /// A scene: `system` names the scene system (`core` for engine scenes,
    /// `scenes` for the scenes module's records) and `id` the scene in it.
    Scene {
        /// The scene system.
        system: SmolStr,
        /// The scene.
        id: u64,
    },
    /// A staffer.
    Staff {
        /// Who.
        by: ObjId,
    },
}

/// The room a metric pose is in (doc 14 §3.4's resolver), answered by the
/// module that owns the windows' composites.
pub type ResolveFn = Arc<dyn Fn(&World, DomainId, &Pose) -> Option<ObjId> + Send + Sync>;

/// The resolver slot: a domain with a window projects its direct participants
/// through it (design D1's deferred `PresentationResolver`), else to its
/// presentation room. Registered once, by the atlas, at its boot.
#[derive(Clone, Default)]
pub struct RoomResolver {
    /// The resolver, when one is composed.
    pub resolve: Option<ResolveFn>,
}

impl std::fmt::Debug for RoomResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomResolver")
            .field("resolve", &self.resolve.is_some())
            .finish()
    }
}

/// The metric between two things on one window (doc 14 §5.2; S5): the
/// range in cells and whether the line of sight between them is blocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricGeometry {
    /// Chebyshev distance in the window's cells.
    pub range_cells: u32,
    /// Whether sight between them is blocked on the window.
    pub occluded: bool,
}

/// The metric answer for two objects, `None` when they do not share a window
/// with a composite; answered by the module that owns the windows.
pub type GeometryFn = Arc<dyn Fn(&World, ObjId, ObjId) -> Option<MetricGeometry> + Send + Sync>;

/// The geometry slot: `mush-propagation` takes its `Geometry { range,
/// occluded }` from it when both ends are on one window, and keeps its
/// graph answer otherwise. Registered once, by the atlas, at its boot.
#[derive(Clone, Default)]
pub struct WindowGeometry {
    /// The answerer, when one is composed.
    pub geometry: Option<GeometryFn>,
}

impl std::fmt::Debug for WindowGeometry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowGeometry")
            .field("geometry", &self.geometry.is_some())
            .finish()
    }
}

/// The one authoritative spatial parent of an object (design §5.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SpatialParent {
    /// On a domain, at a driver-owned pose.
    Direct {
        /// The domain.
        domain: DomainId,
        /// The metric pose (doc 14 §2.4); its posture is the driver's.
        pose: Pose,
    },
    /// Inside an interior space, optionally in a named compartment.
    Interior {
        /// The space object (a room carrying `sim.interior`).
        space: ObjId,
        /// The compartment, if any.
        slot: Option<SlotId>,
    },
    /// Mounted on a host at a named mount point.
    Attached {
        /// The host object.
        host: ObjId,
        /// The mount.
        mount: MountId,
        /// Local transform (opaque), if any.
        local_pose: Option<RulesValue>,
    },
    /// A spatial entity that is currently nowhere (staff-unplaced or
    /// quarantined). Persisted explicitly: "no record" means "not a spatial
    /// entity at all".
    Unplaced,
}

impl SpatialParent {
    /// Whether this parent places the object on/under something.
    #[must_use]
    pub fn is_placed(&self) -> bool {
        !matches!(self, SpatialParent::Unplaced)
    }
}

/// Why and from where an object became unplaced (the durable recovery record
/// of design D4 item 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnplacedInfo {
    /// Human reason.
    pub reason: String,
    /// Who did it (`None` for the system: boot validation, destroy backstop).
    pub by: Option<ObjId>,
    /// Unix seconds.
    pub at: i64,
    /// The parent it had before.
    pub previous: Option<SpatialParent>,
    /// Quarantined (system-forced, held in the recovery room; movement gates
    /// veto it) vs. staff-unplaced (a plain object again).
    #[serde(default)]
    pub quarantined: bool,
}

/// A placement record (design §5.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlacementRecord {
    /// The placed object.
    pub object: ObjId,
    /// Its one parent.
    pub parent: SpatialParent,
    /// Bumped on every change.
    pub revision: u64,
    /// Set while `parent` is `Unplaced`.
    #[serde(default)]
    pub unplaced: Option<UnplacedInfo>,
}

/// Boot-only hold metadata for an authoritative placement document that could
/// not be decoded or whose exact rules references could not be resolved.
///
/// The authoritative `placement:*` value is deliberately left untouched in
/// the store until a later repair/migration workflow can preserve it.  This
/// record keeps the associated object gated and mechanically inactive without
/// pretending that the corrupt document was successfully interpreted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldPlacement {
    /// The authoritative key that remains unchanged.
    pub key: SmolStr,
    /// Bounded parser/validation diagnostic; never the raw document.
    pub reason: String,
}

/// Where an interior space sits (design §5.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SpaceAnchor {
    /// A static interior at a fixed pose in a domain (a hangar).
    FixedDomain {
        /// The domain.
        domain: DomainId,
        /// The anchor pose (opaque), if any.
        anchor_pose: Option<RulesValue>,
    },
    /// Rides a moving object (a dropship bay).
    MovingObject {
        /// The carrier.
        host: ObjId,
    },
    /// Not anchored: occupants have no effective domain.
    Unanchored,
}

/// Generic capacity accounting (design D3 shape). Cut 1 enforces only
/// `maximum_items`; `accepts`/`limits` are stored for later so the doc shape
/// never changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacitySpec {
    /// Maximum occupants/attachments (`None` = unbounded).
    pub maximum_items: Option<u32>,
    /// Accepted load tags (unused in cut 1).
    #[serde(default)]
    pub accepts: Vec<SmolStr>,
    /// Per-measure limits (unused in cut 1).
    #[serde(default)]
    pub limits: BTreeMap<SmolStr, i64>,
}

/// An egress definition (design §5.4). Stored in cut 1; used by cut 2's
/// egress transitions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EgressSpec {
    /// Human label ("bay doors").
    pub label: String,
    /// The pose the egress emits at (opaque), if any.
    pub to_pose: Option<RulesValue>,
}

/// An interior space (design §5.4). In cut 1 the object must be a Room.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteriorSpaceRecord {
    /// The space object (carries `sim.interior`).
    pub object: ObjId,
    /// Where it sits.
    pub anchor: SpaceAnchor,
    /// Named compartments with capacity.
    #[serde(default)]
    pub compartments: BTreeMap<SlotId, CapacitySpec>,
    /// Member rooms of a multi-room interior (reserved for cut 2's boundary
    /// rule; the space object itself is always a member).
    #[serde(default)]
    pub rooms: BTreeSet<ObjId>,
    /// Egresses by id.
    #[serde(default)]
    pub egresses: BTreeMap<SmolStr, EgressSpec>,
}

/// One mount point on a host (design §5.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountSpec {
    /// Capacity.
    pub capacity: CapacitySpec,
    /// Whether attached units keep an independent tactical projection
    /// (design §5.3 `DerivedExposed`) or are `Suppressed`.
    pub exposed: bool,
}

/// A host's mount points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostMounts {
    /// The host.
    pub host: ObjId,
    /// Mounts by id.
    pub mounts: BTreeMap<MountId, MountSpec>,
}

/// A control station on a host (design §5.6). Cut 2 stations are CONTROL
/// positions only (pilot/gunner): an assignment grants the unit as a command
/// subject. Passengers and cargo are ordinary interior slots, never stations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationSpec {
    /// The role (`"pilot"`, `"gunner"`, …) — a DESCRIPTIVE label; the engine no
    /// longer derives anything from it (priority/channels are explicit data).
    pub role: SmolStr,
    /// Control channels this station GRANTS its holder — the capability that
    /// authorizes the unit's channelled commands. Explicit (empty until
    /// authored); the engine only does set-membership, names are ruleset doctrine.
    #[serde(default)]
    pub channels: BTreeSet<ChannelId>,
    /// Dispatch priority (higher wins among an actor's links). Authoring seeds a
    /// role default when omitted; the ENGINE no longer knows pilot/gunner.
    #[serde(default)]
    pub priority: u16,
    /// The backing cockpit room, if any (reserved: cut 2 does not auto-embark).
    pub space: Option<ObjId>,
    /// Opaque ruleset policy (unused in cut 2).
    pub policy: RulesValue,
}

/// A host's control stations, keyed by id (`stations:<host>`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostStations {
    /// The host unit.
    pub host: ObjId,
    /// Stations by id.
    pub stations: BTreeMap<StationId, StationSpec>,
}

/// Who holds a control station (design §5.6). One actor per station in cut 2.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationAssignment {
    /// The occupying actor.
    pub actor: ObjId,
    /// Unix seconds.
    pub assigned_at: i64,
    /// **E7c10 B1 (10b §3.10 OPEN 2) — the seat's narrowed channel set.**
    ///
    /// `None` (the default, and every pre-cut-10 document) means "the whole of
    /// [`StationSpec::channels`]" — the cut-2 behaviour verbatim. `Some(set)`
    /// is intersected with the station's own set by `links::desired_links`, the
    /// ONE production `ChannelGrant` projection, so a proxy seat granted a
    /// narrower set is limited **exactly**: an action whose `required_channels`
    /// fall outside it never projects an offer at all
    /// (`select_offer_channel_grant` finds no grant carrying the channel).
    ///
    /// This is **one extra intersection term inside the one projection** — not a
    /// tenth `ChannelGrant` consumer, not a second projection, and not a second
    /// focus. It is why `assignment:<unit>` becomes legality-gating and §4.1's
    /// cut-10 row records the explicit rule-1 exemption (`legacy-post-image`,
    /// future family `sim.station_assignment`, migrating at E7k beside
    /// placement — the same synchronous `SpatialState` lane).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted: Option<BTreeSet<ChannelId>>,
}

/// Load-refusing cap on one unit's `HostAssignments.seats` (10b §3.10
/// contradiction (vii); cut 5's §7 gap (ix)). A stored map over it is **refused
/// on load, never truncated** — the document is dropped loudly and its host
/// marked, exactly as an unparseable assignment doc already is.
pub const MAX_SEATS_PER_UNIT: usize = 64;

/// Load-refusing cap on one seat's narrowed [`StationAssignment::granted`] set.
/// A station's own `channels` set is authored data bounded by the pack; the
/// narrowed set is a subset of it, so this is a defence-in-depth wire bound.
pub const MAX_GRANTED_CHANNELS: usize = 32;

impl StationAssignment {
    /// The channels this seat actually grants over `spec` — the station's own
    /// set intersected with the seat's narrowing, if any. The single place the
    /// narrowing is computed; `links::desired_links` is its one production
    /// caller.
    #[must_use]
    pub fn granted_channels(&self, spec: &StationSpec) -> BTreeSet<ChannelId> {
        match &self.granted {
            None => spec.channels.clone(),
            Some(granted) => spec.channels.intersection(granted).cloned().collect(),
        }
    }

    /// Re-run the wire invariants for one seat (§4 rule 1). Bounded, so a
    /// stored narrowed set over [`MAX_GRANTED_CHANNELS`] refuses on load.
    pub fn check(&self) -> Result<(), String> {
        if let Some(granted) = &self.granted
            && granted.len() > MAX_GRANTED_CHANNELS
        {
            return Err(format!(
                "station assignment grants {} channels, over the cap of {MAX_GRANTED_CHANNELS}",
                granted.len()
            ));
        }
        Ok(())
    }
}

impl HostAssignments {
    /// Re-run the wire invariants for a whole host's seats (§4 rule 1): the
    /// seat census against [`MAX_SEATS_PER_UNIT`] and every seat's own
    /// narrowing. Called by the boot decode, which drops-and-marks on `Err`.
    pub fn check(&self) -> Result<(), String> {
        if self.seats.len() > MAX_SEATS_PER_UNIT {
            return Err(format!(
                "host {} carries {} seats, over the cap of {MAX_SEATS_PER_UNIT}",
                self.host,
                self.seats.len()
            ));
        }
        for seat in self.seats.values() {
            seat.check()?;
        }
        Ok(())
    }
}

/// A host's current station assignments (`assignment:<host>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostAssignments {
    /// The host unit.
    pub host: ObjId,
    /// Station → who holds it.
    pub seats: BTreeMap<StationId, StationAssignment>,
}

// (Control-role priority is no longer an engine function: `StationSpec.priority`
// is authored data — the @spatial/station default seeds pilot=2/gunner=1.)

/// Derived tactical presence (design §5.3). Computed, never stored.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    /// The driver moves/senses/targets it directly.
    Independent,
    /// Inherits a host transform but stays visible and targetable.
    DerivedExposed,
    /// Local to the domain, no independent tactical projection.
    Suppressed,
}

// ----------------------------------------------------------------------
// State
// ----------------------------------------------------------------------

/// Rebuildable indexes over the records (design §5.3). Never persisted;
/// recomputed in full as the last step of every transition and of boot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Derived {
    /// Host object → its (single, non-retired) domain.
    pub domain_by_host: BTreeMap<ObjId, DomainId>,
    /// Structural children: space → occupants, host → attachments.
    pub children: BTreeMap<ObjId, BTreeSet<ObjId>>,
    /// Domain → directly placed objects.
    pub domain_direct: BTreeMap<DomainId, BTreeSet<ObjId>>,
    /// Domain → every object whose effective domain it is.
    pub local_objects: BTreeMap<DomainId, BTreeSet<ObjId>>,
    /// Domain → local objects with an independent or exposed presence.
    pub projected: BTreeMap<DomainId, BTreeSet<ObjId>>,
    /// Room → domains presenting into it.
    pub presentation_of: BTreeMap<ObjId, BTreeSet<DomainId>>,
    /// Host → spaces anchored on it (`MovingObject`).
    pub anchored_spaces: BTreeMap<ObjId, BTreeSet<ObjId>>,
    /// Domain → spaces fixed in it.
    pub spaces_in_domain: BTreeMap<DomainId, BTreeSet<ObjId>>,
    /// Actor → the units it holds a control station on.
    pub assignments_by_actor: BTreeMap<ObjId, BTreeSet<ObjId>>,
    /// Actor → the exact `(unit, station)` seats it holds (B3): `+stations` and
    /// the fallback re-source iterate THIS, so their scan cost is bounded by the
    /// actor's own billets, never a walk of every unit's whole `seats` map.
    pub seats_by_actor: BTreeMap<ObjId, BTreeSet<(ObjId, StationId)>>,
}

/// The module's state slot: the durable records, the allocator, the recovery
/// audit ledger, and the derived indexes.
#[derive(Clone, Debug)]
pub struct SpatialState {
    /// Domains by id.
    pub domains: BTreeMap<DomainId, DomainRecord>,
    /// Placements by object.
    pub placements: BTreeMap<ObjId, PlacementRecord>,
    /// Unreadable authoritative placements associated with a canonical object
    /// key. These are mechanically inactive and never rewritten at boot.
    pub held_placements: BTreeMap<ObjId, HeldPlacement>,
    /// Unreadable/non-canonical placement keys that cannot safely be associated
    /// with an object. Retained only as bounded in-memory diagnostics; the raw
    /// authoritative values remain in the store.
    pub unbound_placement_docs: BTreeMap<SmolStr, String>,
    /// **Assignment documents the boot REFUSED, held rather than dropped** (E7
    /// cut 10 independent verify fold, B4).
    ///
    /// [`MAX_SEATS_PER_UNIT`] was a load refusal with no write-side counterpart,
    /// and unlike `placement:` (which goes to `held_placements`) and unlike the
    /// campaign lane (`CampaignStore::hold`) the over-cap assignment document was
    /// only `tracing::error!`ed - so it was held NOWHERE, and the very next
    /// ordinary `@spatial/assign` rebuilt the document from the empty RAM map and
    /// overwrote a 65-seat document with a one-seat one. §4.1's "a stored set over
    /// a cap refuses on load and is HELD, never truncated" was false of exactly
    /// this document. A host held here refuses every seat write until staff repair
    /// it, so an ordinary write can never clobber it.
    pub held_assignments: BTreeMap<ObjId, HeldPlacement>,
    /// Interior spaces by object.
    pub spaces: BTreeMap<ObjId, InteriorSpaceRecord>,
    /// Mount definitions by host.
    pub mounts: BTreeMap<ObjId, HostMounts>,
    /// Control-station definitions by host.
    pub stations: BTreeMap<ObjId, HostStations>,
    /// Station assignments by host.
    pub assignments: BTreeMap<ObjId, HostAssignments>,
    /// Per-unit control lease epoch (`control_generation`): bumped whenever a
    /// unit's control surface changes, so a future queued human/NPC/LLM intent
    /// captured `(subject, gen)` can detect it was issued under stale control.
    /// A SECONDARY guard behind the execution-time channel/link re-check.
    pub control_gen: BTreeMap<ObjId, u64>,
    /// Domain id allocator resume point.
    pub next_domain_id: u64,
    /// Where quarantined/unplaced objects are projected (design D4 item 3).
    pub recovery_room: ObjId,
    /// Where a carrier root that is **under way** is projected (E7 cut 6 fold,
    /// D2). Same shape as [`SpatialState::recovery_room`] and the same default
    /// ([`DEFAULT_IN_TRANSIT_ROOM`], the engine's own nowhere-room), but a
    /// different meaning: a *stopped* journey quarantines its root into
    /// `recovery_room`, while a root that is still moving projects here, so a
    /// departed ship actually leaves the room it departed from and staff can
    /// tell "in passage" from "wrecked" at a glance.
    pub in_transit_room: ObjId,
    /// The audit ledger of staff spatial operations and system recoveries.
    pub recovery: Ledger,
    /// Rebuilt indexes.
    pub derived: Derived,
}

impl Default for SpatialState {
    fn default() -> Self {
        SpatialState {
            domains: BTreeMap::new(),
            placements: BTreeMap::new(),
            held_placements: BTreeMap::new(),
            unbound_placement_docs: BTreeMap::new(),
            held_assignments: BTreeMap::new(),
            spaces: BTreeMap::new(),
            mounts: BTreeMap::new(),
            stations: BTreeMap::new(),
            assignments: BTreeMap::new(),
            control_gen: BTreeMap::new(),
            next_domain_id: 0,
            recovery_room: DEFAULT_RECOVERY_ROOM,
            in_transit_room: DEFAULT_IN_TRANSIT_ROOM,
            recovery: Ledger::default(),
            derived: Derived::default(),
        }
    }
}

impl PartialEq for SpatialState {
    /// Everything except the audit ledger (its chunk state is incidental to
    /// the graph); used by the reboot-equality tests.
    fn eq(&self, other: &Self) -> bool {
        self.domains == other.domains
            && self.placements == other.placements
            && self.held_placements == other.held_placements
            && self.unbound_placement_docs == other.unbound_placement_docs
            && self.held_assignments == other.held_assignments
            && self.spaces == other.spaces
            && self.mounts == other.mounts
            && self.stations == other.stations
            && self.assignments == other.assignments
            && self.control_gen == other.control_gen
            && self.next_domain_id == other.next_domain_id
            && self.recovery_room == other.recovery_room
            && self.in_transit_room == other.in_transit_room
            && self.derived == other.derived
    }
}

impl SpatialState {
    /// A fresh state projecting recoveries into `recovery_room`.
    #[must_use]
    pub fn new(recovery_room: ObjId) -> SpatialState {
        SpatialState {
            recovery_room,
            ..SpatialState::default()
        }
    }

    /// Allocate the next domain id (callers persist the meta doc).
    ///
    /// `u64::MAX` is an exhausted sentinel rather than an allocatable id: a
    /// persisted allocator must always retain a representable resume cursor.
    /// Refusal leaves the cursor unchanged.
    pub fn alloc_domain_id(&mut self) -> Result<DomainId, SpatialCounterError> {
        let id = DomainId(self.next_domain_id);
        let next = self
            .next_domain_id
            .checked_add(1)
            .ok_or(SpatialCounterError::DomainIdExhausted)?;
        self.next_domain_id = next;
        Ok(id)
    }

    /// Compute the next control epoch without mutating the state.
    pub fn next_control_generation(&self, unit: ObjId) -> Result<u64, SpatialCounterError> {
        self.control_generation(unit)
            .checked_add(1)
            .ok_or(SpatialCounterError::ControlGenerationExhausted(unit))
    }

    /// Advance a unit's control epoch (a control change); returns the new value.
    /// Callers persist it via [`persist_control_gen`]. Monotonic across reboot.
    /// Refusal leaves the current epoch unchanged.
    pub fn bump_control_gen(&mut self, unit: ObjId) -> Result<u64, SpatialCounterError> {
        let next = self.next_control_generation(unit)?;
        self.apply_control_generation(unit, next);
        Ok(next)
    }

    /// Commit a value returned by [`SpatialState::next_control_generation`]
    /// after the caller has completed all other fallible preflight.
    pub(crate) fn apply_control_generation(&mut self, unit: ObjId, next: u64) {
        debug_assert_eq!(
            self.control_generation(unit).checked_add(1),
            Some(next),
            "prepared control generation must still be current"
        );
        self.control_gen.insert(unit, next);
    }

    /// The current control epoch for `unit` (0 if never changed). A queued intent
    /// captures this at issue and compares with `==` at execution.
    #[must_use]
    pub fn control_generation(&self, unit: ObjId) -> u64 {
        self.control_gen.get(&unit).copied().unwrap_or(0)
    }

    /// The placement parent of `obj` (`Unplaced` when absent or unplaced).
    #[must_use]
    pub fn parent_of(&self, obj: ObjId) -> SpatialParent {
        self.placements
            .get(&obj)
            .map_or(SpatialParent::Unplaced, |p| p.parent.clone())
    }

    /// Whether `obj` is currently placed (Direct/Interior/Attached).
    #[must_use]
    pub fn is_placed(&self, obj: ObjId) -> bool {
        self.placements
            .get(&obj)
            .is_some_and(|p| p.parent.is_placed())
    }

    /// Whether `obj` is quarantined.
    #[must_use]
    pub fn is_quarantined(&self, obj: ObjId) -> bool {
        self.held_placements.contains_key(&obj)
            || self
                .placements
                .get(&obj)
                .and_then(|p| p.unplaced.as_ref())
                .is_some_and(|u| u.quarantined)
    }

    /// Why an unreadable authoritative placement is held, if one exists.
    #[must_use]
    pub fn held_placement(&self, obj: ObjId) -> Option<&HeldPlacement> {
        self.held_placements.get(&obj)
    }

    /// Why a refused `assignment:<host>` document is held, if one is (B4).
    /// **A held host refuses every seat write** until staff repair it.
    #[must_use]
    pub fn held_assignment(&self, host: ObjId) -> Option<&HeldPlacement> {
        self.held_assignments.get(&host)
    }

    /// Whether `obj` is bound into the simulation in ANY way: placed,
    /// quarantined, or acting as infrastructure (a domain host, an interior
    /// space, a mount host, or a station host). A route traveler must be bound
    /// in NONE of these (the EITHER-journey-OR-sim-placed invariant). This is
    /// the single predicate `route_ineligible` (create) and `install_journeys`
    /// (boot) share, so the two never disagree — note `is_placed` checks the
    /// record's PARENT, so a lingering `Unplaced` record is correctly NOT bound.
    #[must_use]
    pub fn is_sim_bound(&self, obj: ObjId) -> bool {
        self.is_placed(obj)
            || self.is_quarantined(obj)
            || self.domain_of_host(obj).is_some()
            || self.spaces.contains_key(&obj)
            || self.mounts.contains_key(&obj)
            || self.stations.contains_key(&obj)
    }

    /// The domain a host object carries, if any (non-retired).
    #[must_use]
    pub fn domain_of_host(&self, host: ObjId) -> Option<&DomainRecord> {
        self.derived
            .domain_by_host
            .get(&host)
            .and_then(|id| self.domains.get(id))
    }

    /// Structural children of `obj` (occupants of a space, attachments on a
    /// host), from the derived index.
    #[must_use]
    pub fn children_of(&self, obj: ObjId) -> &BTreeSet<ObjId> {
        static EMPTY: std::sync::LazyLock<BTreeSet<ObjId>> =
            std::sync::LazyLock::new(BTreeSet::new);
        self.derived.children.get(&obj).unwrap_or(&EMPTY)
    }

    /// Every DIRECT structural descendant of `obj` — the complete inverse of
    /// `graph::next_ancestor`, not just the placement-`children` index: the
    /// Interior occupants + Attached units (`children_of`), plus — if `obj`
    /// hosts a domain — that domain's Direct participants and its FixedDomain
    /// spaces, plus any spaces riding `obj` (MovingObject). `subtree` /
    /// `subtree_height` walk THIS, so they match the ancestor relation the
    /// acyclicity/depth checks use; `children_of` stays the placement-only
    /// dependents query.
    #[must_use]
    pub fn descendant_edges(&self, obj: ObjId) -> BTreeSet<ObjId> {
        let mut out: BTreeSet<ObjId> = self.children_of(obj).iter().copied().collect();
        if let Some(&d) = self.derived.domain_by_host.get(&obj) {
            if let Some(directs) = self.derived.domain_direct.get(&d) {
                out.extend(directs.iter().copied());
            }
            if let Some(spaces) = self.derived.spaces_in_domain.get(&d) {
                out.extend(spaces.iter().copied());
            }
        }
        if let Some(spaces) = self.derived.anchored_spaces.get(&obj) {
            out.extend(spaces.iter().copied());
        }
        out
    }

    /// `obj` plus every transitive structural descendant, in BFS order.
    ///
    /// The membership test is a `BTreeSet` index rather than a linear scan of
    /// the result, so the walk is linear in the records it visits (E7 cut 6
    /// fold, C8: the old `Vec::contains` form was quadratic).
    #[must_use]
    pub fn subtree(&self, obj: ObjId) -> Vec<ObjId> {
        self.walk_subtree(obj, usize::MAX)
    }

    /// The BFS of [`SpatialState::subtree`], stopping as soon as `limit`
    /// records have been **visited**. The one walker the bounded and unbounded
    /// forms share, so they can never disagree about what a subtree is.
    fn walk_subtree(&self, obj: ObjId, limit: usize) -> Vec<ObjId> {
        if limit == 0 {
            return Vec::new();
        }
        let mut out = vec![obj];
        let mut seen: BTreeSet<ObjId> = BTreeSet::from([obj]);
        let mut i = 0;
        while i < out.len() && out.len() < limit {
            let cur = out[i];
            for c in self.descendant_edges(cur) {
                if seen.insert(c) {
                    out.push(c);
                    if out.len() >= limit {
                        break;
                    }
                }
            }
            i += 1;
        }
        out
    }

    /// The subtree of `obj` when it fits in `max` records, or `Err(count)` with
    /// `count == max + 1` when it does not.
    ///
    /// **Bounded on records VISITED** (D-LBK-5): the walk stops the moment it
    /// has seen one record too many, so a pathological graph costs the cap, not
    /// the graph. The caller refuses — never truncates (§4 rule 1).
    ///
    /// # Errors
    ///
    /// `Err(max + 1)` when the subtree is larger than `max`.
    pub fn subtree_bounded(&self, obj: ObjId, max: usize) -> Result<Vec<ObjId>, usize> {
        let walked = self.walk_subtree(obj, max.saturating_add(1));
        if walked.len() > max {
            Err(walked.len())
        } else {
            Ok(walked)
        }
    }

    /// The same bounded walk for the callers that legitimately work on a
    /// prefix (the arrival's onboard scene floors, the link refresh): at most
    /// `max` records visited, and the walk stops there.
    #[must_use]
    pub fn subtree_truncated(&self, obj: ObjId, max: usize) -> Vec<ObjId> {
        self.walk_subtree(obj, max)
    }

    /// The height of `obj`'s structural subtree: 0 with no children, else
    /// 1 + the deepest child's height. Guarded against a malformed cycle and
    /// bounded by [`MAX_SPATIAL_DEPTH`].
    #[must_use]
    pub fn subtree_height(&self, obj: ObjId) -> usize {
        let mut seen = BTreeSet::from([obj]);
        let mut stack = vec![(obj, 0usize)];
        let mut max = 0usize;
        while let Some((cur, depth)) = stack.pop() {
            max = max.max(depth);
            if depth >= MAX_SPATIAL_DEPTH {
                continue;
            }
            for c in self.descendant_edges(cur) {
                if seen.insert(c) {
                    stack.push((c, depth + 1));
                }
            }
        }
        max
    }

    /// Objects currently placed `Interior` in `space` (optionally in `slot`).
    #[must_use]
    pub fn occupants(&self, space: ObjId, slot: Option<&SlotId>) -> usize {
        self.children_of(space)
            .iter()
            .filter(|&&o| match self.parent_of(o) {
                SpatialParent::Interior { slot: s, .. } => {
                    slot.is_none_or(|want| s.as_ref() == Some(want))
                }
                _ => false,
            })
            .count()
    }

    /// Objects currently attached to `host` at `mount`.
    #[must_use]
    pub fn attachments(&self, host: ObjId, mount: &MountId) -> usize {
        self.children_of(host)
            .iter()
            .filter(|&&o| matches!(self.parent_of(o), SpatialParent::Attached { mount: ref m, .. } if m == mount))
            .count()
    }
}

// ----------------------------------------------------------------------
// Persistence
// ----------------------------------------------------------------------

/// Key of a domain doc.
#[must_use]
pub fn domain_key(id: DomainId) -> SmolStr {
    SmolStr::new(format!("domain:{}", id.0))
}
/// Key of a placement doc.
#[must_use]
pub fn placement_key(obj: ObjId) -> SmolStr {
    SmolStr::new(format!("placement:{}", obj.0))
}
/// Key of an interior-space doc.
#[must_use]
pub fn space_key(obj: ObjId) -> SmolStr {
    SmolStr::new(format!("space:{}", obj.0))
}
/// Key of a mounts doc.
#[must_use]
pub fn mounts_key(host: ObjId) -> SmolStr {
    SmolStr::new(format!("mounts:{}", host.0))
}
/// Key of a stations doc.
#[must_use]
pub fn stations_key(host: ObjId) -> SmolStr {
    SmolStr::new(format!("stations:{}", host.0))
}
/// Key of an assignments doc.
#[must_use]
pub fn assignment_key(host: ObjId) -> SmolStr {
    SmolStr::new(format!("assignment:{}", host.0))
}

pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

fn json_set(out: &mut Outbox, key: SmolStr, value: &impl Serialize) {
    let value = serde_json::to_value(value).expect("sim doc serializes");
    push_op(out, ModuleWrite::JsonSet { key, value });
}

pub(crate) fn persist_domain(out: &mut Outbox, d: &DomainRecord) {
    json_set(out, domain_key(d.id), d);
}
pub(crate) fn persist_placement(out: &mut Outbox, p: &PlacementRecord) {
    json_set(out, placement_key(p.object), p);
}
pub(crate) fn persist_space(out: &mut Outbox, s: &InteriorSpaceRecord) {
    json_set(out, space_key(s.object), s);
}
pub(crate) fn persist_mounts(out: &mut Outbox, m: &HostMounts) {
    if m.mounts.is_empty() {
        persist_del(out, mounts_key(m.host));
    } else {
        json_set(out, mounts_key(m.host), m);
    }
}
pub(crate) fn persist_stations(out: &mut Outbox, s: &HostStations) {
    if s.stations.is_empty() {
        persist_del(out, stations_key(s.host));
    } else {
        json_set(out, stations_key(s.host), s);
    }
}
pub(crate) fn persist_assignments(out: &mut Outbox, a: &HostAssignments) {
    if a.seats.is_empty() {
        persist_del(out, assignment_key(a.host));
    } else {
        json_set(out, assignment_key(a.host), a);
    }
}
pub(crate) fn persist_meta(out: &mut Outbox, next_domain_id: u64) {
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: serde_json::json!({ "next_domain_id": next_domain_id }),
        },
    );
}
pub(crate) fn persist_del(out: &mut Outbox, key: SmolStr) {
    push_op(out, ModuleWrite::Del { key });
}

/// The doc key of a unit's control epoch (`cgen:<unit>`).
#[must_use]
pub fn cgen_key(unit: ObjId) -> SmolStr {
    SmolStr::new(format!("cgen:{}", unit.0))
}
pub(crate) fn persist_control_gen(out: &mut Outbox, unit: ObjId, generation: u64) {
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: cgen_key(unit),
            value: serde_json::json!(generation),
        },
    );
}

/// The default recovery room when none is configured.
pub const DEFAULT_RECOVERY_ROOM: ObjId = LIMBO;

/// The default in-transit room when none is configured (E7 cut 6 fold, D2).
/// [`LIMBO`] is the engine's own nowhere-room: nothing walks in and nothing
/// there is takeable, which is exactly what an under-way hull needs.
pub const DEFAULT_IN_TRANSIT_ROOM: ObjId = LIMBO;

#[cfg(test)]
mod tests {
    use super::{DomainId, SpatialCounterError, SpatialState};
    use mush_core::ObjId;

    #[test]
    fn domain_allocator_exhaustion_is_explicit_and_nonmutating() {
        let mut state = SpatialState {
            next_domain_id: u64::MAX - 1,
            ..SpatialState::default()
        };

        assert_eq!(state.alloc_domain_id(), Ok(DomainId(u64::MAX - 1)));
        assert_eq!(state.next_domain_id, u64::MAX);
        let before = state.clone();
        assert_eq!(
            state.alloc_domain_id(),
            Err(SpatialCounterError::DomainIdExhausted)
        );
        assert_eq!(state, before, "exhausted allocation changed state");
    }

    #[test]
    fn control_generation_exhaustion_is_explicit_and_nonmutating() {
        let unit = ObjId(42);
        let mut state = SpatialState::default();
        state.control_gen.insert(unit, u64::MAX - 1);

        assert_eq!(state.bump_control_gen(unit), Ok(u64::MAX));
        let before = state.clone();
        assert_eq!(
            state.bump_control_gen(unit),
            Err(SpatialCounterError::ControlGenerationExhausted(unit))
        );
        assert_eq!(state, before, "exhausted control bump changed state");
    }
}
