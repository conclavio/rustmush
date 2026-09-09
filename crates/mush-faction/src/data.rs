//! The faction registry data model (design: scenes_factions_knowledge.md §3,
//! registry half) and its persistence keys.
//!
//! Live state is one [`FactionState`] in a typed module state slot on the
//! `World`; **every mutation mirrors to module ops** so the persisted
//! `mod:faction:*` namespace is always write-through current:
//!
//! | key                    | document                                     |
//! |------------------------|----------------------------------------------|
//! | `meta`                 | `{ "next_faction_id": u64, "next_region_id": u64 }` |
//! | `fac:{id}`             | one [`Faction`] (edges + resources embedded) |
//! | `member:{player}:{id}` | one [`FactionMembership`]                    |
//! | `region:{id}`          | one [`Region`]                               |
//! | `control:{region}`     | `Vec<ControlRecord>` for that region         |
//!
//! ## The three separations this model enforces (§3.3)
//!
//! - **Membership fact vs. knowledge of membership**: the registry is engine
//!   ground truth and drives *authorization only* (intel clearance, channel
//!   eligibility). Every membership carries a [`Visibility`] policy that every
//!   display surface must respect — the engine knows the infiltrator's real
//!   allegiance; the roster a player sees is a projection.
//! - **Structure vs. stance**: [`FactionEdge`]s are the small closed
//!   structural vocabulary (`ChapterOf`/`SubordinateTo`/`ContractedTo`/
//!   `FrontFor`), acyclic per kind, never auto-transitive. Rivalry/alliance
//!   stances are the future relationship matrix, not edges.
//! - **Control vs. ownership**: [`ControlRecord`]s attach [`ControlKind`]s
//!   (de jure / de facto / covert / economic) to abstract [`Region`]s — a
//!   region *may* reference zones, or nothing at all ("Media influence").
//!   In the registry phase control records are staff-set; the simulation
//!   phase moves changes behind front/clock resolution.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Deferred, ModuleOp, ModuleWrite, ObjId, Outbox};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The faction persistence namespace (`mod:faction:*`).
pub const NS: &str = "faction";

/// The kv key of the faction meta document (id allocator resume points).
pub const META_KEY: &str = "meta";

/// Bounded length of a faction's cause-stamped resource change log.
pub const RESOURCE_LOG_CAP: usize = 20;

/// Longest accepted faction / region name.
pub const MAX_NAME_LEN: usize = 40;

/// A faction id: monotonic per world, never recycled (allocator resume point
/// persists in the meta doc).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FactionId(pub u64);

impl std::fmt::Display for FactionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A region id: monotonic per world, never recycled.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegionId(pub u64);

impl std::fmt::Display for RegionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Who an NPC informant reports intel to — the generalized M12 principal.
/// A ghoul reports to its domitor ([`Principal::Character`] — personal bonds
/// stay personal); a gang lookout reports to the gang
/// ([`Principal::Faction`] — a clearance-gated shared pool).
///
/// Serde is untagged for backward compatibility: an old profile/intel doc's
/// bare dbref number (`42`) still deserializes as `Character(#42)`; faction
/// principals serialize as `{"faction": N}`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Principal {
    /// One character (the M12 original: domitor, employer, patron).
    Character(ObjId),
    /// A faction: intel lands in the faction's pool, readable by members
    /// whose level passes the faction's `intel_level`.
    Faction {
        /// The faction whose pool receives the intel.
        faction: FactionId,
    },
}

impl Principal {
    /// Convenience constructor for the faction variant.
    #[must_use]
    pub fn faction(id: FactionId) -> Principal {
        Principal::Faction { faction: id }
    }
}

/// Who may learn a fact the registry holds (a membership, an edge, a control
/// record). The engine always knows the truth; display surfaces project it
/// through this policy. Staff always pass.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Anyone may see it.
    #[default]
    Public,
    /// Members of the relevant faction see it.
    MembersKnow,
    /// Officers of the relevant faction see it.
    OfficersKnow,
    /// Staff only — the fact exists, the world has to discover it.
    Covert,
}

impl Visibility {
    /// Parse a staff-typed visibility name (accepts a few aliases).
    #[must_use]
    pub fn parse(s: &str) -> Option<Visibility> {
        match s.trim().to_ascii_lowercase().as_str() {
            "public" => Some(Visibility::Public),
            "members" | "members_know" | "membersknow" => Some(Visibility::MembersKnow),
            "officers" | "officers_know" | "officersknow" => Some(Visibility::OfficersKnow),
            "covert" | "secret" => Some(Visibility::Covert),
            _ => None,
        }
    }

    /// The staff-facing label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::MembersKnow => "members",
            Visibility::OfficersKnow => "officers",
            Visibility::Covert => "covert",
        }
    }
}

/// The small closed structural edge vocabulary (§3.2). Structural edges are
/// acyclic per kind and **never** confer transitive membership or implicit
/// permission inheritance.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Court of Chicago → Camarilla; garrison → House Davion. Implies
    /// representation and *default* stance inheritance.
    ChapterOf,
    /// Lance → company; coterie sworn to the Prince.
    SubordinateTo,
    /// Merc unit → House; runner crew → Johnson's corp. Time-boxed in the
    /// obligation system later; the edge records the standing relationship.
    ContractedTo,
    /// Setite temple → Followers of Set. Covert by default: the public node
    /// is real and joinable, the edge is the secret.
    FrontFor,
}

impl EdgeKind {
    /// Parse a staff-typed edge kind.
    #[must_use]
    pub fn parse(s: &str) -> Option<EdgeKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "chapter" | "chapter_of" | "chapterof" => Some(EdgeKind::ChapterOf),
            "subordinate" | "subordinate_to" | "subordinateto" => Some(EdgeKind::SubordinateTo),
            "contracted" | "contracted_to" | "contractedto" => Some(EdgeKind::ContractedTo),
            "front" | "front_for" | "frontfor" => Some(EdgeKind::FrontFor),
            _ => None,
        }
    }

    /// The default visibility for a new edge of this kind: `FrontFor` is
    /// covert by design; the rest are public.
    #[must_use]
    pub fn default_visibility(self) -> Visibility {
        match self {
            EdgeKind::FrontFor => Visibility::Covert,
            _ => Visibility::Public,
        }
    }

    /// The staff-facing label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            EdgeKind::ChapterOf => "chapter of",
            EdgeKind::SubordinateTo => "subordinate to",
            EdgeKind::ContractedTo => "contracted to",
            EdgeKind::FrontFor => "front for",
        }
    }
}

/// One outgoing structural edge, stored on the *from* faction's document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FactionEdge {
    /// The edge kind.
    pub kind: EdgeKind,
    /// The target faction.
    pub to: FactionId,
    /// Who may learn the edge exists.
    #[serde(default)]
    pub visibility: Visibility,
    /// Optional staff note ("expires with the Q3 contract").
    #[serde(default)]
    pub note: String,
}

/// One cause-stamped resource pool change (the bounded audit tail; the full
/// history belongs to the canon ledger once it exists).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceChange {
    /// The pool kind ("influence.police", "nuyen", "boon_credit").
    pub kind: SmolStr,
    /// Signed delta applied.
    pub delta: i64,
    /// Why ("staff: waterfront shakedown payout").
    pub reason: String,
    /// Who applied it.
    pub by: ObjId,
    /// When (unix seconds).
    pub ts: i64,
}

fn default_officer_level() -> u8 {
    2
}

/// One faction — the persisted `fac:{id}` document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Faction {
    /// Stable id.
    pub id: FactionId,
    /// Display name, unique case-insensitively.
    pub name: SmolStr,
    /// Alternate names resolving to this faction.
    #[serde(default)]
    pub aliases: Vec<SmolStr>,
    /// Description + goals prose.
    #[serde(default)]
    pub charter: String,
    /// A hidden faction (secret society) is listed only to members and staff;
    /// to everyone else it does not exist.
    #[serde(default)]
    pub hidden: bool,
    /// Membership `level >= officer_level` ⇒ officer (passes
    /// [`Visibility::OfficersKnow`], may see member levels).
    #[serde(default = "default_officer_level")]
    pub officer_level: u8,
    /// Membership `level >= intel_level` ⇒ may read the faction's intel pool
    /// via `+intel`. Default 0: every member reads it.
    #[serde(default)]
    pub intel_level: u8,
    /// Typed resource pools — never one influence scalar. Core owns the
    /// ledgered mutation; rulesets declare the vocabulary.
    ///
    /// These are **ratings**, not money: saturating, unaudited, and free of the
    /// double-entry discipline. A key that should be money is migrated out of
    /// here exactly once by `@faction/migrate` and recorded in [`Faction::migrated`]
    /// (E3d / D-E3-9).
    #[serde(default)]
    pub resources: BTreeMap<SmolStr, i64>,
    /// Bounded cause-stamped tail of resource changes (newest last).
    #[serde(default)]
    pub resource_log: Vec<ResourceChange>,
    /// The backing organization (E3d, D-E3-8): the faction's money identity, so
    /// value attributed to "the Camarilla" is owned by a real principal with
    /// scopes, grants, and a journal. `None` is the default and the norm —
    /// `@faction/create` never auto-binds; binding is a deliberate staff act
    /// (`@faction/org`).
    ///
    /// The link is validated at **every use** ([`backing_org`](crate::backing_org))
    /// and checked once more at boot: `FactionModule` loads after `OrgModule`
    /// (E3-D47), so a missing org or two factions naming one org is reported loudly
    /// at boot and never unbound. Either makes the link unusable — fail closed, both
    /// sides.
    #[serde(default)]
    pub org: Option<mush_org::OrgId>,
    /// Resource keys that are **money now**, not ratings (E3d, D-E3-9). The
    /// classification is explicit, static, and one-way: a key is money iff it is
    /// listed here, so registering an instrument can never retroactively turn a
    /// rating into something mintable. `@faction/resource` refuses these keys and
    /// points at `@econ`.
    #[serde(default)]
    pub migrated: BTreeSet<SmolStr>,
    /// Outgoing structural edges.
    #[serde(default)]
    pub edges: Vec<FactionEdge>,
    /// NPC(s) who personify the faction (the Prince, the Johnson's boss).
    /// A faction must be able to exist and act with zero PCs.
    #[serde(default)]
    pub principals: Vec<ObjId>,
    /// A bound comsys channel: **Public** members are auto-joined on add and
    /// removed on leave. Everyone else — MembersKnow included — is never
    /// auto-joined: a channel roster is world-readable via `@channel/who`
    /// (the see lock defaults open), so putting any quieter membership on it
    /// would publish exactly what the registry hides. The module also never
    /// removes a channel membership it did not create (a manual `addcom` is
    /// self-disclosure and stays).
    #[serde(default)]
    pub channel: Option<mush_comsys::ChannelId>,
    /// Opaque ruleset extension document (praxis/domain, turf income,
    /// contracts, TO&E) — owned by the ruleset, never interpreted here.
    #[serde(default)]
    pub ruleset_ext: serde_json::Value,
}

impl Faction {
    /// A fresh faction with defaults.
    #[must_use]
    pub fn new(id: FactionId, name: &str) -> Faction {
        Faction {
            id,
            name: SmolStr::new(name),
            aliases: Vec::new(),
            charter: String::new(),
            hidden: false,
            officer_level: default_officer_level(),
            intel_level: 0,
            resources: BTreeMap::new(),
            resource_log: Vec::new(),
            org: None,
            migrated: BTreeSet::new(),
            edges: Vec::new(),
            principals: Vec::new(),
            channel: None,
            ruleset_ext: serde_json::Value::Null,
        }
    }

    /// Apply one resource delta with its cause, keeping the log bounded.
    ///
    /// **Rating-only** (E3d): this is the saturating, unaudited arithmetic ratings
    /// want, and it *creates* the key if absent — which is exactly why the
    /// `@faction/migrate` path must never route through it. A migrated key removed
    /// from `resources` would be resurrected here at 0 and start looking like a
    /// second, parallel pool for money that already lives in the journal.
    pub fn apply_resource(&mut self, kind: &str, delta: i64, reason: &str, by: ObjId, ts: i64) {
        let kind = SmolStr::new(kind.to_ascii_lowercase());
        let slot = self.resources.entry(kind.clone()).or_insert(0);
        *slot = slot.saturating_add(delta);
        self.push_resource_log(ResourceChange {
            kind,
            delta,
            reason: reason.to_string(),
            by,
            ts,
        });
    }

    /// Append one change to the bounded audit tail, dropping the oldest entries
    /// past `RESOURCE_LOG_CAP`. The migration path (E3d) appends **directly**
    /// through here rather than through [`Faction::apply_resource`], which would
    /// re-create the pool key it just removed.
    pub fn push_resource_log(&mut self, change: ResourceChange) {
        self.resource_log.push(change);
        if self.resource_log.len() > RESOURCE_LOG_CAP {
            let overflow = self.resource_log.len() - RESOURCE_LOG_CAP;
            self.resource_log.drain(..overflow);
        }
    }
}

fn default_member_level() -> u8 {
    1
}

/// One character's membership of one faction — the persisted
/// `member:{player}:{faction}` document. **Ground truth**: used for
/// authorization; shown to others only through the [`Visibility`] policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FactionMembership {
    /// The member.
    pub character: ObjId,
    /// The faction.
    pub faction: FactionId,
    /// Cosmetic title ("Sheriff", "Harpy", "Face").
    #[serde(default)]
    pub title: SmolStr,
    /// Authorization scalar: officer / intel thresholds compare against it.
    /// Ruleset-named rank ladders layer on later; the engine needs only the
    /// number.
    #[serde(default = "default_member_level")]
    pub level: u8,
    /// Who may learn this membership exists.
    #[serde(default)]
    pub visibility: Visibility,
    /// When they joined (unix seconds).
    pub since: i64,
}

/// A territory region — deliberately **not** the room graph. A region may
/// reference zone master objects (street level), or nothing at all (a
/// non-spatial region like "Media influence in Chicago"); map-blob areas
/// arrive with the tactical subsystem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// Stable id.
    pub id: RegionId,
    /// Display name, unique case-insensitively.
    pub name: SmolStr,
    /// Staff note.
    #[serde(default)]
    pub note: String,
    /// Zone master objects this region covers (may be empty).
    #[serde(default)]
    pub zones: Vec<ObjId>,
    /// The atlas place this region stands on, if it stands anywhere (S2b, doc 14
    /// D-14-28): a neighborhood's or a region's pack id, or `<pack>:<entry>` for a
    /// locality. Never required — a non-spatial region has none.
    #[serde(default)]
    pub atlas: Option<String>,
}

/// How a faction holds a region (§3.5). A single owner field cannot express
/// the interesting states — the Prince still *claims* the waterfront
/// (`DeJure`) while the gang *holds* it (`DeFacto`) — so control is a set of
/// records per region, one per (faction, kind).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    /// Recognized claim (praxis, legal title, planetary charter).
    DeJure,
    /// Actual on-the-ground control.
    DeFacto,
    /// Control exercised secretly (the Nosferatu own the sewers).
    Covert,
    /// Economic grip (turf income, supply lines) without formal or physical
    /// control.
    Economic,
}

impl ControlKind {
    /// Parse a staff-typed control kind.
    #[must_use]
    pub fn parse(s: &str) -> Option<ControlKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "dejure" | "de_jure" | "jure" | "claim" => Some(ControlKind::DeJure),
            "defacto" | "de_facto" | "facto" | "held" => Some(ControlKind::DeFacto),
            "covert" => Some(ControlKind::Covert),
            "economic" | "econ" => Some(ControlKind::Economic),
            _ => None,
        }
    }

    /// The display label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ControlKind::DeJure => "de jure",
            ControlKind::DeFacto => "de facto",
            ControlKind::Covert => "covert",
            ControlKind::Economic => "economic",
        }
    }
}

fn default_strength() -> u8 {
    1
}

/// One control record: `faction` holds `region` in one [`ControlKind`].
/// Registry phase: staff-set only. Simulation phase: changes move behind
/// front/clock resolution, each mutation carrying its cause.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlRecord {
    /// The region held.
    pub region: RegionId,
    /// The holder.
    pub faction: FactionId,
    /// The kind of hold.
    pub kind: ControlKind,
    /// Coarse strength (1–5 by convention).
    #[serde(default = "default_strength")]
    pub strength: u8,
    /// Factions actively contesting this hold.
    #[serde(default)]
    pub contested_by: Vec<FactionId>,
    /// Who may learn this record exists (relative to the *holding* faction).
    #[serde(default)]
    pub visibility: Visibility,
    /// When the hold began (unix seconds).
    pub since: i64,
}

/// The live faction registry state, held in a module state slot on the
/// `World`. Mirrored write-through by the persisted `mod:faction:*` docs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FactionState {
    /// Every faction by id.
    pub factions: BTreeMap<FactionId, Faction>,
    /// Every membership, keyed `(character, faction)`.
    pub members: BTreeMap<(ObjId, FactionId), FactionMembership>,
    /// Every region by id.
    pub regions: BTreeMap<RegionId, Region>,
    /// Control records per region.
    pub control: BTreeMap<RegionId, Vec<ControlRecord>>,
    /// Faction id allocator resume point (persisted in the meta doc).
    pub next_faction_id: u64,
    /// Region id allocator resume point (persisted in the meta doc).
    pub next_region_id: u64,
}

impl FactionState {
    /// The faction with this (case-insensitive) name or alias.
    #[must_use]
    pub fn faction_by_name(&self, name: &str) -> Option<&Faction> {
        let name = name.trim();
        self.factions.values().find(|f| {
            f.name.eq_ignore_ascii_case(name)
                || f.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
        })
    }

    /// The region with this (case-insensitive) name.
    #[must_use]
    pub fn region_by_name(&self, name: &str) -> Option<&Region> {
        let name = name.trim();
        self.regions
            .values()
            .find(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// All of one character's memberships, in faction-id order.
    pub fn memberships_of(&self, who: ObjId) -> impl Iterator<Item = &FactionMembership> {
        self.members
            .range((who, FactionId(0))..=(who, FactionId(u64::MAX)))
            .map(|(_, m)| m)
    }

    /// All members of one faction (full scan; rosters are small).
    pub fn members_of(&self, faction: FactionId) -> impl Iterator<Item = &FactionMembership> {
        self.members
            .iter()
            .filter(move |((_, f), _)| *f == faction)
            .map(|(_, m)| m)
    }

    /// One membership record, if it exists.
    #[must_use]
    pub fn membership(&self, who: ObjId, faction: FactionId) -> Option<&FactionMembership> {
        self.members.get(&(who, faction))
    }

    /// Whether `who` is a member of `faction` (any visibility — truth).
    #[must_use]
    pub fn is_member(&self, who: ObjId, faction: FactionId) -> bool {
        self.members.contains_key(&(who, faction))
    }

    /// Whether `who` is an officer of `faction` (level ≥ `officer_level`).
    #[must_use]
    pub fn is_officer(&self, who: ObjId, faction: FactionId) -> bool {
        let Some(f) = self.factions.get(&faction) else {
            return false;
        };
        self.membership(who, faction)
            .is_some_and(|m| m.level >= f.officer_level)
    }

    /// Whether `who` may read `faction`'s intel pool: a member whose level
    /// passes the faction's `intel_level`.
    #[must_use]
    pub fn can_read_intel(&self, who: ObjId, faction: FactionId) -> bool {
        let Some(f) = self.factions.get(&faction) else {
            return false;
        };
        self.membership(who, faction)
            .is_some_and(|m| m.level >= f.intel_level)
    }

    /// Whether a fact scoped to `faction` with visibility `vis` is visible to
    /// `viewer`. Staff always pass; the truth stays in the registry.
    #[must_use]
    pub fn visibility_passes(
        &self,
        viewer: ObjId,
        staff: bool,
        faction: FactionId,
        vis: Visibility,
    ) -> bool {
        if staff {
            return true;
        }
        match vis {
            Visibility::Public => true,
            Visibility::MembersKnow => self.is_member(viewer, faction),
            Visibility::OfficersKnow => self.is_officer(viewer, faction),
            Visibility::Covert => false,
        }
    }

    /// Whether `viewer` may see membership `m`: always their own, else the
    /// visibility policy against `m.faction`.
    #[must_use]
    pub fn membership_visible(&self, viewer: ObjId, staff: bool, m: &FactionMembership) -> bool {
        viewer == m.character || self.visibility_passes(viewer, staff, m.faction, m.visibility)
    }

    /// Whether `viewer` may see faction `f` in listings: hidden factions
    /// exist only for members and staff.
    #[must_use]
    pub fn faction_listed(&self, viewer: ObjId, staff: bool, f: &Faction) -> bool {
        !f.hidden || staff || self.is_member(viewer, f.id)
    }

    /// Whether adding an edge `from --kind--> to` would create a cycle in
    /// that kind's directed graph (walks same-kind edges from `to`).
    #[must_use]
    pub fn would_cycle(&self, from: FactionId, kind: EdgeKind, to: FactionId) -> bool {
        if from == to {
            return true;
        }
        let mut stack = vec![to];
        let mut seen = vec![to];
        while let Some(cur) = stack.pop() {
            let Some(f) = self.factions.get(&cur) else {
                continue;
            };
            for e in f.edges.iter().filter(|e| e.kind == kind) {
                if e.to == from {
                    return true;
                }
                if !seen.contains(&e.to) {
                    seen.push(e.to);
                    stack.push(e.to);
                }
            }
        }
        false
    }

    /// Allocate the next faction id (callers persist the meta doc).
    pub fn alloc_faction_id(&mut self) -> FactionId {
        let id = FactionId(self.next_faction_id);
        self.next_faction_id += 1;
        id
    }

    /// Allocate the next region id (callers persist the meta doc).
    pub fn alloc_region_id(&mut self) -> RegionId {
        let id = RegionId(self.next_region_id);
        self.next_region_id += 1;
        id
    }
}

// ----------------------------------------------------------------------
// Persistence keys + op helpers
// ----------------------------------------------------------------------

/// The kv key of a faction document.
#[must_use]
pub fn fac_key(id: FactionId) -> SmolStr {
    SmolStr::new(format!("fac:{}", id.0))
}

/// The kv key of a membership document.
#[must_use]
pub fn member_key(who: ObjId, faction: FactionId) -> SmolStr {
    SmolStr::new(format!("member:{}:{}", who.0, faction.0))
}

/// The kv key of a region document.
#[must_use]
pub fn region_key(id: RegionId) -> SmolStr {
    SmolStr::new(format!("region:{}", id.0))
}

/// The kv key of a region's control record vec.
#[must_use]
pub fn control_key(id: RegionId) -> SmolStr {
    SmolStr::new(format!("control:{}", id.0))
}

/// Queue one faction write on the outbox's deferred side-channel.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Mirror a faction document (plain data; serialization cannot fail).
pub(crate) fn persist_faction(out: &mut Outbox, f: &Faction) {
    let value = serde_json::to_value(f).expect("faction doc serializes");
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: fac_key(f.id),
            value,
        },
    );
}

/// Mirror a membership document.
pub(crate) fn persist_membership(out: &mut Outbox, m: &FactionMembership) {
    let value = serde_json::to_value(m).expect("membership doc serializes");
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: member_key(m.character, m.faction),
            value,
        },
    );
}

/// Mirror a region document.
pub(crate) fn persist_region(out: &mut Outbox, r: &Region) {
    let value = serde_json::to_value(r).expect("region doc serializes");
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: region_key(r.id),
            value,
        },
    );
}

/// Mirror a region's control vec (empty vec deletes the doc).
pub(crate) fn persist_control(out: &mut Outbox, region: RegionId, records: &[ControlRecord]) {
    if records.is_empty() {
        persist_del(out, control_key(region));
        return;
    }
    let value = serde_json::to_value(records).expect("control records serialize");
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: control_key(region),
            value,
        },
    );
}

/// Mirror the meta document (the id allocator resume points).
pub(crate) fn persist_meta(out: &mut Outbox, next_faction_id: u64, next_region_id: u64) {
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: serde_json::json!({
                "next_faction_id": next_faction_id,
                "next_region_id": next_region_id,
            }),
        },
    );
}

/// Delete one persisted key.
pub(crate) fn persist_del(out: &mut Outbox, key: SmolStr) {
    push_op(out, ModuleWrite::Del { key });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(names: &[&str]) -> FactionState {
        let mut st = FactionState::default();
        for n in names {
            let id = st.alloc_faction_id();
            st.factions.insert(id, Faction::new(id, n));
        }
        st
    }

    fn join(st: &mut FactionState, who: ObjId, faction: FactionId, level: u8, vis: Visibility) {
        st.members.insert(
            (who, faction),
            FactionMembership {
                character: who,
                faction,
                title: SmolStr::default(),
                level,
                visibility: vis,
                since: 0,
            },
        );
    }

    #[test]
    fn principal_serde_is_backward_compatible() {
        // The M12 wire form: a bare dbref number is a character principal.
        let p: Principal = serde_json::from_str("42").unwrap();
        assert_eq!(p, Principal::Character(ObjId(42)));
        // The faction form is tagged by its field name.
        let p: Principal = serde_json::from_str(r#"{"faction": 3}"#).unwrap();
        assert_eq!(p, Principal::faction(FactionId(3)));
        // Round trips.
        let c = serde_json::to_string(&Principal::Character(ObjId(7))).unwrap();
        assert_eq!(c, "7");
        let f = serde_json::to_string(&Principal::faction(FactionId(9))).unwrap();
        assert_eq!(f, r#"{"faction":9}"#);
    }

    #[test]
    fn name_and_alias_resolution() {
        let mut st = state_with(&["Camarilla"]);
        let id = st.faction_by_name("camarilla").unwrap().id;
        st.factions.get_mut(&id).unwrap().aliases = vec![SmolStr::new("The Ivory Tower")];
        assert_eq!(st.faction_by_name("the ivory tower").unwrap().id, id);
        assert!(st.faction_by_name("anarchs").is_none());
    }

    #[test]
    fn officer_and_intel_thresholds() {
        let mut st = state_with(&["Undertow"]);
        let id = st.faction_by_name("Undertow").unwrap().id;
        st.factions.get_mut(&id).unwrap().intel_level = 2;
        join(&mut st, ObjId(10), id, 1, Visibility::Public);
        join(&mut st, ObjId(11), id, 2, Visibility::Public);
        assert!(st.is_member(ObjId(10), id));
        assert!(!st.is_officer(ObjId(10), id), "level 1 < officer 2");
        assert!(st.is_officer(ObjId(11), id));
        assert!(!st.can_read_intel(ObjId(10), id), "level 1 < intel 2");
        assert!(st.can_read_intel(ObjId(11), id));
        assert!(!st.can_read_intel(ObjId(99), id), "non-member never reads");
    }

    #[test]
    fn visibility_projection() {
        let mut st = state_with(&["Setites"]);
        let id = st.faction_by_name("Setites").unwrap().id;
        join(&mut st, ObjId(10), id, 3, Visibility::Public); // officer
        join(&mut st, ObjId(11), id, 1, Visibility::MembersKnow);
        join(&mut st, ObjId(12), id, 1, Visibility::Covert); // the infiltrator
        let covert = st.membership(ObjId(12), id).unwrap().clone();
        let members_know = st.membership(ObjId(11), id).unwrap().clone();
        // An outsider sees neither.
        assert!(!st.membership_visible(ObjId(99), false, &members_know));
        assert!(!st.membership_visible(ObjId(99), false, &covert));
        // A member sees MembersKnow but never Covert.
        assert!(st.membership_visible(ObjId(10), false, &members_know));
        assert!(!st.membership_visible(ObjId(10), false, &covert));
        // Staff see truth; the covert member always sees themselves.
        assert!(st.membership_visible(ObjId(99), true, &covert));
        assert!(st.membership_visible(ObjId(12), false, &covert));
    }

    #[test]
    fn hidden_faction_listing() {
        let mut st = state_with(&["Black Hand"]);
        let id = st.faction_by_name("Black Hand").unwrap().id;
        st.factions.get_mut(&id).unwrap().hidden = true;
        join(&mut st, ObjId(10), id, 1, Visibility::Covert);
        let f = st.factions.get(&id).unwrap().clone();
        assert!(
            !st.faction_listed(ObjId(99), false, &f),
            "outsider: no such faction"
        );
        assert!(st.faction_listed(ObjId(10), false, &f), "member sees it");
        assert!(st.faction_listed(ObjId(99), true, &f), "staff see it");
    }

    #[test]
    fn edge_cycles_are_detected_per_kind() {
        let mut st = state_with(&["A", "B", "C"]);
        let a = st.faction_by_name("A").unwrap().id;
        let b = st.faction_by_name("B").unwrap().id;
        let c = st.faction_by_name("C").unwrap().id;
        // A -> B -> C chain of ChapterOf.
        st.factions.get_mut(&a).unwrap().edges.push(FactionEdge {
            kind: EdgeKind::ChapterOf,
            to: b,
            visibility: Visibility::Public,
            note: String::new(),
        });
        st.factions.get_mut(&b).unwrap().edges.push(FactionEdge {
            kind: EdgeKind::ChapterOf,
            to: c,
            visibility: Visibility::Public,
            note: String::new(),
        });
        // Closing the loop is a cycle; self-edges always are.
        assert!(st.would_cycle(c, EdgeKind::ChapterOf, a));
        assert!(st.would_cycle(a, EdgeKind::ChapterOf, a));
        // The same closing edge in a DIFFERENT kind is not a cycle.
        assert!(!st.would_cycle(c, EdgeKind::ContractedTo, a));
        // A fresh edge deeper into the chain is fine.
        assert!(!st.would_cycle(a, EdgeKind::ChapterOf, c));
    }

    #[test]
    fn resource_pools_are_cause_stamped_and_bounded() {
        let mut f = Faction::new(FactionId(1), "Corp");
        for i in 0..(RESOURCE_LOG_CAP as i64 + 5) {
            f.apply_resource("Nuyen", 10, "payday", ObjId(1), i);
        }
        assert_eq!(
            f.resources.get("nuyen"),
            Some(&(10 * (RESOURCE_LOG_CAP as i64 + 5)))
        );
        assert_eq!(f.resource_log.len(), RESOURCE_LOG_CAP, "log stays bounded");
        assert_eq!(
            f.resource_log.last().unwrap().ts,
            RESOURCE_LOG_CAP as i64 + 4
        );
        // Kinds are case-normalized so "Nuyen" and "nuyen" are one pool.
        f.apply_resource("NUYEN", -10, "spent", ObjId(1), 99);
        assert_eq!(f.resources.len(), 1);
    }

    #[test]
    fn control_records_serde_round_trip() {
        let rec = ControlRecord {
            region: RegionId(1),
            faction: FactionId(2),
            kind: ControlKind::DeFacto,
            strength: 3,
            contested_by: vec![FactionId(4)],
            visibility: Visibility::Public,
            since: 1000,
        };
        let json = serde_json::to_string(&rec).unwrap();
        let back: ControlRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rec);
        // Sparse docs default sensibly.
        let sparse: ControlRecord =
            serde_json::from_str(r#"{"region":1,"faction":2,"kind":"de_jure","since":5}"#).unwrap();
        assert_eq!(sparse.kind, ControlKind::DeJure);
        assert_eq!(sparse.strength, 1);
        assert!(sparse.contested_by.is_empty());
    }

    #[test]
    fn parse_helpers_accept_staff_spellings() {
        assert_eq!(Visibility::parse("Members"), Some(Visibility::MembersKnow));
        assert_eq!(Visibility::parse("secret"), Some(Visibility::Covert));
        assert_eq!(EdgeKind::parse("chapter"), Some(EdgeKind::ChapterOf));
        assert_eq!(EdgeKind::parse("front_for"), Some(EdgeKind::FrontFor));
        assert_eq!(ControlKind::parse("de_facto"), Some(ControlKind::DeFacto));
        assert_eq!(ControlKind::parse("claim"), Some(ControlKind::DeJure));
        assert_eq!(EdgeKind::FrontFor.default_visibility(), Visibility::Covert);
        assert_eq!(EdgeKind::ChapterOf.default_visibility(), Visibility::Public);
    }
}
