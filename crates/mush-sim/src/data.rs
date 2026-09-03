//! Durable records of the spatial model (design 06 §5–§6) and their
//! persistence: one JSON document per record under the `sim` namespace.
//!
//! The placement graph is the truth; everything in [`Derived`] is a
//! rebuildable projection of it and is never persisted.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use mush_core::{
    ChannelId, Deferred, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, DefinitionValidationContext, LIMBO, Ledger, ModuleOp, ModuleWrite,
    ObjId, Outbox, OwnerId, RulesValueError,
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

/// The domain clock mode (design §6.2). Cut 1 creates only `Passive`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum DomainClock {
    /// Holds placements; never ticks.
    Passive,
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
}

/// The one authoritative spatial parent of an object (design §5.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SpatialParent {
    /// On a domain, at a driver-owned pose.
    Direct {
        /// The domain.
        domain: DomainId,
        /// The pose (opaque).
        pose: RulesValue,
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
            spaces: BTreeMap::new(),
            mounts: BTreeMap::new(),
            stations: BTreeMap::new(),
            assignments: BTreeMap::new(),
            control_gen: BTreeMap::new(),
            next_domain_id: 0,
            recovery_room: DEFAULT_RECOVERY_ROOM,
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
            && self.spaces == other.spaces
            && self.mounts == other.mounts
            && self.stations == other.stations
            && self.assignments == other.assignments
            && self.control_gen == other.control_gen
            && self.next_domain_id == other.next_domain_id
            && self.recovery_room == other.recovery_room
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
    #[must_use]
    pub fn subtree(&self, obj: ObjId) -> Vec<ObjId> {
        let mut out = vec![obj];
        let mut i = 0;
        while i < out.len() {
            let cur = out[i];
            for c in self.descendant_edges(cur) {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
            i += 1;
        }
        out
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
