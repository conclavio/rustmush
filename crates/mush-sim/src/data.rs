//! Durable records of the spatial model (design 06 §5–§6) and their
//! persistence: one JSON document per record under the `sim` namespace.
//!
//! The placement graph is the truth; everything in [`Derived`] is a
//! rebuildable projection of it and is never persisted.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Deferred, Ledger, ModuleOp, ModuleWrite, ObjId, Outbox, LIMBO};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The module namespace (`mod:sim:*`).
pub const NS: &str = "sim";
/// The allocator doc.
pub const META_KEY: &str = "meta";
/// The only driver cut 1 accepts: a passive domain that holds placements and
/// never ticks (design §6.2 `DomainClock::Passive`).
pub const PASSIVE_DRIVER: &str = "sim.passive";
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

/// A compartment/berth name within an interior space (lowercased).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SlotId(SmolStr);

/// A mount point name on a host (lowercased).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MountId(SmolStr);

macro_rules! name_id {
    ($t:ident) => {
        impl $t {
            /// A non-empty, trimmed, lowercased name.
            #[must_use]
            pub fn new(s: &str) -> Option<$t> {
                let s = s.trim().to_lowercase();
                if s.is_empty() { None } else { Some($t(SmolStr::new(s))) }
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

// ----------------------------------------------------------------------
// Records
// ----------------------------------------------------------------------

/// An opaque, schema-tagged value whose interpretation belongs to a domain
/// driver (design §5.1): a hex pose, a berth transform, whatever the ruleset
/// needs. The passive driver stores it verbatim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RulesValue {
    /// The schema id (`"tactics.hex"`, …).
    pub schema: SmolStr,
    /// Schema version.
    pub version: u32,
    /// The payload.
    pub data: serde_json::Value,
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
pub struct DomainRecord {
    /// Durable id.
    pub id: DomainId,
    /// The object carrying `sim.domain`.
    pub host: ObjId,
    /// The driver id (cut 1: [`PASSIVE_DRIVER`] only).
    pub driver: SmolStr,
    /// Driver data schema version.
    pub schema_version: u32,
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
}

/// The module's state slot: the durable records, the allocator, the recovery
/// audit ledger, and the derived indexes.
#[derive(Clone, Debug)]
pub struct SpatialState {
    /// Domains by id.
    pub domains: BTreeMap<DomainId, DomainRecord>,
    /// Placements by object.
    pub placements: BTreeMap<ObjId, PlacementRecord>,
    /// Interior spaces by object.
    pub spaces: BTreeMap<ObjId, InteriorSpaceRecord>,
    /// Mount definitions by host.
    pub mounts: BTreeMap<ObjId, HostMounts>,
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
            spaces: BTreeMap::new(),
            mounts: BTreeMap::new(),
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
            && self.spaces == other.spaces
            && self.mounts == other.mounts
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
    pub fn alloc_domain_id(&mut self) -> DomainId {
        let id = DomainId(self.next_domain_id);
        self.next_domain_id += 1;
        id
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
        self.placements.get(&obj).is_some_and(|p| p.parent.is_placed())
    }

    /// Whether `obj` is quarantined.
    #[must_use]
    pub fn is_quarantined(&self, obj: ObjId) -> bool {
        self.placements
            .get(&obj)
            .and_then(|p| p.unplaced.as_ref())
            .is_some_and(|u| u.quarantined)
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
        static EMPTY: std::sync::LazyLock<BTreeSet<ObjId>> = std::sync::LazyLock::new(BTreeSet::new);
        self.derived.children.get(&obj).unwrap_or(&EMPTY)
    }

    /// `obj` plus every transitive structural descendant, in BFS order.
    #[must_use]
    pub fn subtree(&self, obj: ObjId) -> Vec<ObjId> {
        let mut out = vec![obj];
        let mut i = 0;
        while i < out.len() {
            let cur = out[i];
            for &c in self.children_of(cur) {
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
            for &c in self.children_of(cur) {
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
                SpatialParent::Interior { slot: s, .. } => slot.is_none_or(|want| s.as_ref() == Some(want)),
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

/// The default recovery room when none is configured.
pub const DEFAULT_RECOVERY_ROOM: ObjId = LIMBO;
