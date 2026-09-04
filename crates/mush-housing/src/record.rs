//! The housing record model (E5a, 08d §3.1) and its caps (§9).
//!
//! Everything here is a plain, serde-round-trippable record with **no world access
//! and no game vocabulary** (D-E5-5). A [`SupportLevel`] is an opaque `0..=5`, not a
//! dot; a [`PropertyTier`] is an opaque `0..=5` physical class, not haven quality; a
//! [`SupportSource`] is an attribution label, not a rules object. `ruleset-wod`
//! interprets sheets and submits [`SupportPlan`]s through housing's own entry
//! ([`crate::submit_plan`]); housing never reads a sheet.
//!
//! **Caps are enforced, never asserted.** Every bound in 08d §9 is checked by a
//! fallible constructor or a fallible mutator returning [`crate::HousingError`] —
//! there is no `assert!`, no `unwrap`, and no panicking index on record data
//! anywhere in this crate, because a corrupt persisted document must fail a
//! transition, not the server.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use mush_economy::{Amount, CadenceId, ContractId, EscalationId, ObligationId};
use mush_org::{OrgId, OrgScopeId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::error::HousingError;
use crate::ids::{PropertyId, SceneRef, TenancyId};

// ---------------------------------------------------------------------------
// Caps (08d §9)
// ---------------------------------------------------------------------------

/// The most rooms one property may hold (§9).
pub const MAX_ROOMS_PER_PROPERTY: usize = 16;
/// The most approved features one property may carry (§9).
pub const MAX_FEATURES: usize = 16;
/// The most tenants one tenancy may name (§9).
pub const MAX_TENANTS: usize = 8;
/// The most tenant-managed guests one tenancy may name (§9).
pub const MAX_GUESTS: usize = 16;
/// The bounded audit tail one tenancy keeps (§9). The module's own persistence and
/// the jobs/escalation records are the real history; this is the summary
/// `@tenancy/show` prints.
pub const MAX_TENANCY_HISTORY: usize = 32;

/// The largest [`SupportLevel`] and [`PropertyTier`] (§3.1: both are `0..=5`).
pub const MAX_LEVEL: u8 = 5;

/// The largest property name a record stores.
///
/// Not in §9 — the crate-wide "every stored string is bounded" discipline
/// `mush-economy` applies to `MAX_REASON_BYTES`/`MAX_CONTRACT_NAME_BYTES`, applied
/// here so a persisted document can never carry an unbounded blob into a render.
pub const MAX_PROPERTY_NAME_BYTES: usize = 64;
/// The largest feature key a record stores (see [`MAX_PROPERTY_NAME_BYTES`]).
pub const MAX_FEATURE_KEY_BYTES: usize = 32;
/// The largest free-text reason a [`Resolution`] carries (see
/// [`MAX_PROPERTY_NAME_BYTES`]).
pub const MAX_REASON_BYTES: usize = 256;
/// The most attribution sources one [`SupportPlan`] lists (see
/// [`MAX_PROPERTY_NAME_BYTES`]).
pub const MAX_PLAN_SOURCES: usize = 8;

// ---------------------------------------------------------------------------
// Opaque scalars
// ---------------------------------------------------------------------------

/// A property's physical class and upkeep band, `0..=5` — **not** haven quality
/// (D-E5-7) and not a game statistic. Housing compares it to a [`SupportLevel`] and
/// nothing else.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PropertyTier(u8);

impl PropertyTier {
    /// Build a tier, **clamping** to [`MAX_LEVEL`] (the `GracePolicy::new`
    /// discipline: a stored record can never carry a band the comparison cannot
    /// express).
    #[must_use]
    pub fn new(v: u8) -> PropertyTier {
        PropertyTier(v.min(MAX_LEVEL))
    }
    /// The band, clamped on **read** as well: a document hand-written (or written by
    /// a later build) above the ceiling must not resurrect an unreachable tier.
    #[must_use]
    pub fn get(self) -> u8 {
        self.0.min(MAX_LEVEL)
    }
}

/// What the ruleset asserts about one resident's means, `0..=5` — deliberately
/// **opaque** (D-E5-5). Housing does not know it came from Resources, or from dots at
/// all; `ruleset-wod` derives it and housing only ever compares it to a
/// [`PropertyTier`].
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SupportLevel(u8);

impl SupportLevel {
    /// Build a level, **clamping** to [`MAX_LEVEL`].
    #[must_use]
    pub fn new(v: u8) -> SupportLevel {
        SupportLevel(v.min(MAX_LEVEL))
    }
    /// The level, clamped on read as well as at construction.
    #[must_use]
    pub fn get(self) -> u8 {
        self.0.min(MAX_LEVEL)
    }
}

// ---------------------------------------------------------------------------
// Property
// ---------------------------------------------------------------------------

/// One approved feature of a property (a workshop, a security door): a key the
/// ruleset or the module interprets, plus who approved it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Feature {
    /// The feature key (≤ [`MAX_FEATURE_KEY_BYTES`]).
    pub key: SmolStr,
    /// The staffer who approved it.
    pub approved_by: ObjId,
}

/// A property's lifecycle. There is no "destroyed": a property whose rooms died is
/// `Withdrawn` with an escalation (§4), never silently gone — every tenancy on it
/// stays exactly as it was, because D-E5-1 forbids a home disappearing on its own.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum PropertyStatus {
    /// Built and available; no live tenancy.
    #[default]
    Vacant,
    /// A live tenancy occupies it (the state the access hook and the destroy gate
    /// answer for).
    Let,
    /// Out of service — withdrawn by staff, or its rooms went missing at boot.
    Withdrawn,
}

/// A residence: an anchor room plus the ordinary rooms that make it up (D-E5-12).
///
/// `anchor` is the primary room — the `EconomicAsset` title record lives on it, and
/// it is the `Obj.home` every tenant is pointed at (§3.3).
///
/// **Loading re-checks every invariant** [`Property::new`] enforces
/// (`#[serde(try_from)]` through a private `PropertyWire`, the `mush-core` `DefinitionRef` /
/// `mush-tactics` `TacticalUnit` pattern): a document is exactly as trustworthy as the
/// disk it came off, and a plain derive let a hand-written or half-migrated one mint a
/// 56-room property, or one whose anchor is not among its rooms — a title room outside
/// the property, and a `Let` room the access hook never sees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PropertyWire")]
pub struct Property {
    /// Identity (never reused).
    pub id: PropertyId,
    /// Display name (≤ [`MAX_PROPERTY_NAME_BYTES`]).
    pub name: SmolStr,
    /// The primary room; the title record lives here and tenants are homed on it.
    pub anchor: ObjId,
    /// Every room of the property, `anchor` included; ≤ [`MAX_ROOMS_PER_PROPERTY`].
    pub rooms: BTreeSet<ObjId>,
    /// The residence's physical class and upkeep band (**not** haven quality).
    pub tier: PropertyTier,
    /// Who lets it: a named landlord organization, or `System(wod.housing)`.
    pub landlord: PrincipalId,
    /// The org compartment that manages it (D-E3-5); `None` for a system landlord.
    #[serde(default)]
    pub landlord_scope: Option<OrgScopeId>,
    /// Exact-mode rent per cadence period; `None` in abstract mode, which posts no
    /// pretend rent (owner answer 6).
    #[serde(default)]
    pub upkeep: Option<Amount>,
    /// Approved features; ≤ [`MAX_FEATURES`].
    #[serde(default)]
    pub features: Vec<Feature>,
    /// Lifecycle.
    #[serde(default)]
    pub status: PropertyStatus,
    /// Staleness stamp.
    pub revision: u64,
}

impl Property {
    /// Build a property, enforcing every §9 cap and the anchor rule.
    ///
    /// Fallible rather than clamping: silently dropping a room the builder listed
    /// would leave a room of a `Let` property outside the access hook's answer — a
    /// fail-**open** hole, which is exactly what this cut exists to prevent.
    pub fn new(
        id: PropertyId,
        name: impl AsRef<str>,
        anchor: ObjId,
        mut rooms: BTreeSet<ObjId>,
        tier: PropertyTier,
        landlord: PrincipalId,
    ) -> Result<Property, HousingError> {
        let name = name.as_ref();
        if name.trim().is_empty() {
            return Err(HousingError::Invalid("a property needs a name"));
        }
        if name.len() > MAX_PROPERTY_NAME_BYTES {
            return Err(HousingError::TooLarge("property name"));
        }
        rooms.insert(anchor);
        if rooms.len() > MAX_ROOMS_PER_PROPERTY {
            return Err(HousingError::TooLarge("rooms per property"));
        }
        Ok(Property {
            id,
            name: SmolStr::new(name),
            anchor,
            rooms,
            tier,
            landlord,
            landlord_scope: None,
            upkeep: None,
            features: Vec::new(),
            status: PropertyStatus::Vacant,
            revision: 1,
        })
    }

    /// Add a room, refusing past [`MAX_ROOMS_PER_PROPERTY`]. Adding a room the
    /// property already holds is a no-op that does not consume budget.
    pub fn add_room(&self, room: ObjId) -> Result<Property, HousingError> {
        if self.rooms.contains(&room) {
            return Ok(self.clone());
        }
        if self.rooms.len() >= MAX_ROOMS_PER_PROPERTY {
            return Err(HousingError::TooLarge("rooms per property"));
        }
        let mut next = self.clone();
        next.rooms.insert(room);
        next.revision = next.revision.saturating_add(1);
        Ok(next)
    }

    /// Remove a room. The **anchor** may never be removed: the title record and every
    /// tenant's `Obj.home` point at it, so a property without its anchor is a
    /// tenancy homed on nothing.
    ///
    /// Refused entirely while the property is [`PropertyStatus::Let`]: a room dropped
    /// from a let property is a room somebody is living in that the access predicate
    /// stops answering for — the tenants' own rooms silently leave their tenancy while
    /// the tenancy is still live. Withdraw the property, or end the tenancy, first.
    pub fn remove_room(&self, room: ObjId) -> Result<Property, HousingError> {
        if room == self.anchor {
            return Err(HousingError::Invalid("the anchor room cannot be removed"));
        }
        if self.status == PropertyStatus::Let {
            return Err(HousingError::Invalid("a let property's rooms cannot be removed"));
        }
        let mut next = self.clone();
        if next.rooms.remove(&room) {
            next.revision = next.revision.saturating_add(1);
        }
        Ok(next)
    }

    /// Add an approved feature, enforcing [`MAX_FEATURES`] and the key bound.
    pub fn add_feature(
        &self,
        key: impl AsRef<str>,
        approved_by: ObjId,
    ) -> Result<Property, HousingError> {
        let key = key.as_ref().trim();
        if key.is_empty() {
            return Err(HousingError::Invalid("a feature needs a key"));
        }
        if key.len() > MAX_FEATURE_KEY_BYTES {
            return Err(HousingError::TooLarge("feature key"));
        }
        if self.features.iter().any(|f| f.key == key) {
            return Ok(self.clone());
        }
        if self.features.len() >= MAX_FEATURES {
            return Err(HousingError::TooLarge("features per property"));
        }
        let mut next = self.clone();
        next.features.push(Feature { key: SmolStr::new(key), approved_by });
        next.revision = next.revision.saturating_add(1);
        Ok(next)
    }
}

/// The persisted shape of a [`Property`], deserialized **through**
/// [`Property::new`]'s checks.
///
/// Deriving `Deserialize` straight onto [`Property`] made the constructor advisory: a
/// document could carry more rooms than [`MAX_ROOMS_PER_PROPERTY`], an empty or
/// oversized name, duplicate feature keys, or an anchor that is not one of its own
/// rooms — every one of them a fail-**open** hole, because a room outside `rooms` is a
/// room of a `Let` property the access predicate is never asked about. A structurally
/// broken document now fails to load instead.
#[derive(Deserialize)]
struct PropertyWire {
    id: PropertyId,
    name: SmolStr,
    anchor: ObjId,
    rooms: BTreeSet<ObjId>,
    tier: PropertyTier,
    landlord: PrincipalId,
    #[serde(default)]
    landlord_scope: Option<OrgScopeId>,
    #[serde(default)]
    upkeep: Option<Amount>,
    #[serde(default)]
    features: Vec<Feature>,
    #[serde(default)]
    status: PropertyStatus,
    revision: u64,
}

impl TryFrom<PropertyWire> for Property {
    type Error = HousingError;

    fn try_from(w: PropertyWire) -> Result<Property, HousingError> {
        // The anchor is **checked** here rather than inserted. `Property::new` adds it
        // to the set a builder passed, which is a convenience at build time; doing the
        // same on load would silently repair a document whose title room is outside
        // its own property — exactly the corruption worth failing on.
        if !w.rooms.contains(&w.anchor) {
            return Err(HousingError::Invalid("the anchor must be one of the property's rooms"));
        }
        let mut p = Property::new(w.id, w.name.as_str(), w.anchor, w.rooms, w.tier, w.landlord)?;
        if w.features.len() > MAX_FEATURES {
            return Err(HousingError::TooLarge("features per property"));
        }
        let mut seen: BTreeSet<SmolStr> = BTreeSet::new();
        for f in &w.features {
            if f.key.trim().is_empty() {
                return Err(HousingError::Invalid("a feature needs a key"));
            }
            if f.key.len() > MAX_FEATURE_KEY_BYTES {
                return Err(HousingError::TooLarge("feature key"));
            }
            if !seen.insert(f.key.clone()) {
                return Err(HousingError::Invalid("a feature key appears twice"));
            }
        }
        p.landlord_scope = w.landlord_scope;
        p.upkeep = w.upkeep;
        p.features = w.features;
        p.status = w.status;
        p.revision = w.revision;
        Ok(p)
    }
}

// ---------------------------------------------------------------------------
// Access terms
// ---------------------------------------------------------------------------

/// When an inspection term admits its holder. There is no implied landlord entry and
/// no "delinquency unlocks the door" (D-E5-4): legal authority, economic title and
/// physical access stay separate, and inspection is an **explicit** policy term
/// (owner answer 9).
///
/// Deliberately **not** `Default`. It defaulted to [`InspectionWhen::Always`], the
/// widest term of the two, so anything that reached for a default — a form, a partial
/// document, a `..Default::default()` struct update in a later cut — minted a standing
/// right of entry nobody wrote down. A term that does not say *when* is not a term;
/// every construction names one.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InspectionWhen {
    /// Whenever the tenancy is live.
    Always,
    /// Only while the tenancy is `Delinquent` or `EvictionPending`.
    WhileDelinquent,
}

/// One explicit inspection right.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectionTerm {
    /// Who holds it (an object principal, or an org/system principal E5b resolves —
    /// see [`crate::may_enter_with`]).
    pub who: PrincipalId,
    /// When it admits them.
    pub when: InspectionWhen,
}

/// The tenancy's access policy. Empty by default — a tenancy grants nobody but its
/// tenants and their guests until somebody writes a term down.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessTerms {
    /// The one explicit inspection right, if any.
    #[serde(default)]
    pub inspection: Option<InspectionTerm>,
}

// ---------------------------------------------------------------------------
// Funding
// ---------------------------------------------------------------------------

/// Where a tenancy's periodic assessment comes from (D-E5-2).
///
/// `Abstract` is the default deployment: a per-resident [`SupportPlan`] asserted by
/// the ruleset, no money posted anywhere. `Exact` mirrors a real E4 obligation under
/// a real E4 contract — housing stores the ids and never touches the money.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Funding {
    /// Abstract support: what each resident's ruleset asserts, keyed by resident.
    Abstract {
        /// One plan per resident; ≤ [`MAX_TENANTS`]. A plan is keyed by the resident
        /// it describes, so one resident's submission can never overwrite another's
        /// (D-E5-13).
        #[serde(default)]
        plans: BTreeMap<ObjId, SupportPlan>,
    },
    /// Exact money: an E4 contract and the obligation that settles it each period.
    Exact {
        /// The E4 contract.
        contract: ContractId,
        /// The E4 obligation whose `ObligationChanged` events housing mirrors.
        obligation: ObligationId,
    },
}

impl Default for Funding {
    fn default() -> Funding {
        Funding::Abstract { plans: BTreeMap::new() }
    }
}

impl Funding {
    /// The abstract plans, or an empty map for an exact tenancy.
    #[must_use]
    pub fn plans(&self) -> Option<&BTreeMap<ObjId, SupportPlan>> {
        match self {
            Funding::Abstract { plans } => Some(plans),
            Funding::Exact { .. } => None,
        }
    }
}

/// One attribution label behind a resident's means — **display and attribution
/// only** (§3.1). Housing never reasons about these; they exist so `+home` and a
/// story job can say *why* somebody can afford where they live.
///
/// Not `Ord`: `mush_org::PrincipalId` is deliberately not ordered, and a label list
/// has no meaningful order anyway.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportSource {
    /// A job of some kind. The full career profile is `ruleset-wod`'s
    /// (`CareerProfile`); housing keeps only the opaque kind.
    Career {
        /// An opaque ruleset-defined kind (≤ [`MAX_FEATURE_KEY_BYTES`]).
        kind: SmolStr,
    },
    /// Somebody funds them.
    Patron {
        /// Who.
        who: PrincipalId,
    },
    /// Old money.
    Inheritance,
    /// Income nobody declares.
    Criminal,
    /// An organization's support.
    Domain {
        /// Which org.
        org: OrgId,
    },
    /// **Reserved** (D-E5-10). Retainers, ghouls and Herd are explicitly out of E5
    /// (08d §1.1); this variant exists so the wire format does not change when
    /// dependents land, and nothing in E5 constructs or interprets it.
    Dependent,
}

/// What the ruleset asserts about ONE resident's means, per cadence period.
///
/// Only [`crate::submit_plan`] writes one, called by the ruleset's evaluator and
/// never by a player command (D-E5-5), and a plan must name a **current tenant**.
///
/// A plan **expires**. `asserted_period` is what [`crate::assess_at`] measures its age
/// against, so an evaluator that stops running (a crashed ruleset, a character nobody
/// re-evaluates) stops supporting the lease after [`crate::PLAN_MAX_AGE_PERIODS`]
/// instead of carrying it forever on an answer nobody has restated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportPlan {
    /// Whose means this describes.
    pub resident: ObjId,
    /// The asserted level, opaque `0..=5`.
    pub level: SupportLevel,
    /// Attribution only; ≤ [`MAX_PLAN_SOURCES`].
    #[serde(default)]
    pub sources: Vec<SupportSource>,
    /// The cadence position this was asserted at — the raw stamp `@tenancy/show`
    /// prints, on the tenancy's own clock (it may be negative, and it may jump when a
    /// cadence is re-pointed).
    pub asserted_at: i64,
    /// The cadence **period ordinal** this was asserted in — the monotonic count E5b
    /// keeps, and the only thing freshness is measured in.
    ///
    /// Separate from `asserted_at` on purpose: staleness must survive a re-pointed
    /// clock or a restored backup, which is exactly when the raw position lies.
    /// `#[serde(default)]` (0), so a document written before this field existed reads
    /// as "asserted at the beginning of time" — which [`crate::assess_at`] ages out,
    /// the fail-closed direction.
    #[serde(default)]
    pub asserted_period: u64,
    /// Staleness stamp.
    pub revision: u64,
}

impl SupportPlan {
    /// Build a plan, enforcing [`MAX_PLAN_SOURCES`].
    pub fn new(
        resident: ObjId,
        level: SupportLevel,
        sources: Vec<SupportSource>,
        asserted_at: i64,
        asserted_period: u64,
    ) -> Result<SupportPlan, HousingError> {
        if sources.len() > MAX_PLAN_SOURCES {
            return Err(HousingError::TooLarge("plan sources"));
        }
        Ok(SupportPlan { resident, level, sources, asserted_at, asserted_period, revision: 1 })
    }
}

// ---------------------------------------------------------------------------
// Tenancy
// ---------------------------------------------------------------------------

/// The bounded story window opened when automatic processing reaches
/// `EvictionPending` (§3.2). It is bounded in **notices**, not in duration: an
/// offline player's home persists indefinitely (owner answer 8).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryWindow {
    /// The cadence position the window opened at.
    pub opened: i64,
    /// The `wod-story` job E5b opened for the ST, once it has one.
    #[serde(default)]
    pub job: Option<u64>,
    /// The scene the ST optionally started (opaque — see [`SceneRef`]).
    #[serde(default)]
    pub scene: Option<SceneRef>,
    /// How many **re**-notices have gone out; ≤ [`crate::MAX_WINDOW_NOTICES`]. The
    /// notice that accompanies the transition itself is not counted here.
    #[serde(default)]
    pub notified: u8,
    /// The level-2 escalation, once the quiet gap has elapsed (§4.6).
    #[serde(default)]
    pub escalation: Option<EscalationId>,
    /// The cadence position of the last notice — seeded to `opened`, so the first
    /// re-notice lands one [`crate::WINDOW_RENOTIFY`] later and never on the same
    /// period as the transition notice.
    pub last_notice: i64,
    /// The cadence position the window **closed** at, set only on the copy parked in
    /// [`Tenancy::last_window`] when the tenancy recovered (`None` while the window is
    /// the live one). It is what the relapse hysteresis
    /// ([`crate::RELAPSE_WINDOW_PERIODS`]) measures from.
    #[serde(default)]
    pub closed: Option<i64>,
}

impl StoryWindow {
    /// A window opening at `now`.
    #[must_use]
    pub fn opened_at(now: i64) -> StoryWindow {
        StoryWindow {
            opened: now,
            job: None,
            scene: None,
            notified: 0,
            escalation: None,
            last_notice: now,
            closed: None,
        }
    }
}

/// Why a tenancy ended. **Every** `Ended` carries one (D-E5-1): the enum variant
/// itself takes a `Resolution`, so there is no way to spell an ended tenancy that
/// nobody decided to end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// The tenant moved out (`+home/leave`).
    TenantLeft,
    /// Staff ended it after a scene (`@tenancy/resolve <id>=end`).
    StaffEnded {
        /// Who.
        by: ObjId,
        /// Why (≤ [`MAX_REASON_BYTES`]).
        reason: SmolStr,
    },
    /// Staff deliberately waived it (`@tenancy/resolve <id>=waive`).
    Waived {
        /// Who.
        by: ObjId,
    },
    /// A contract settlement closed it.
    Settled,
}

impl Resolution {
    /// A staff resolution with its reason **bounded** at [`MAX_REASON_BYTES`].
    pub fn staff_ended(by: ObjId, reason: impl AsRef<str>) -> Result<Resolution, HousingError> {
        let reason = reason.as_ref();
        if reason.len() > MAX_REASON_BYTES {
            return Err(HousingError::TooLarge("resolution reason"));
        }
        Ok(Resolution::StaffEnded { by, reason: SmolStr::new(reason) })
    }
}

/// The payload-free discriminant of a [`TenancyStatus`] — what a history entry and
/// an E5b event carry (the `ObligationStatusKind` pattern).
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum TenancyStatusKind {
    /// Offered, not yet accepted by both sides.
    #[default]
    Pending,
    /// Live and supported.
    Active,
    /// A period was missed.
    Grace,
    /// The grace allowance is spent.
    Delinquent,
    /// The story window is open.
    EvictionPending,
    /// Ended, by an explicit act.
    Ended,
}

impl TenancyStatusKind {
    /// A short human word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            TenancyStatusKind::Pending => "pending",
            TenancyStatusKind::Active => "active",
            TenancyStatusKind::Grace => "grace",
            TenancyStatusKind::Delinquent => "delinquent",
            TenancyStatusKind::EvictionPending => "eviction pending",
            TenancyStatusKind::Ended => "ended",
        }
    }
}

/// A tenancy's lifecycle (§3.2).
///
/// The one shape rule of this cut: **`Ended` carries a [`Resolution`]**, so the type
/// system, not a code review, is what guarantees D-E5-1's "a home never disappears on
/// its own". Automatic processing stops at `EvictionPending`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TenancyStatus {
    /// Offered; awaiting the landlord's and every tenant's acceptance.
    #[default]
    Pending,
    /// Live and supported.
    Active,
    /// A period was missed; the allowance is not yet spent.
    Grace {
        /// The cadence position the run started at.
        since: i64,
    },
    /// Past the grace allowance. A delinquent tenant **still enters their home**
    /// (D-E5-4).
    Delinquent {
        /// The cadence position delinquency started at.
        since: i64,
    },
    /// The story window is open. Nothing further happens automatically.
    EvictionPending {
        /// The bounded window.
        window: StoryWindow,
    },
    /// Ended by an explicit act, and only by one.
    Ended {
        /// Which act.
        how: Resolution,
    },
}

impl TenancyStatus {
    /// The payload-free discriminant.
    #[must_use]
    pub fn kind(&self) -> TenancyStatusKind {
        match self {
            TenancyStatus::Pending => TenancyStatusKind::Pending,
            TenancyStatus::Active => TenancyStatusKind::Active,
            TenancyStatus::Grace { .. } => TenancyStatusKind::Grace,
            TenancyStatus::Delinquent { .. } => TenancyStatusKind::Delinquent,
            TenancyStatus::EvictionPending { .. } => TenancyStatusKind::EvictionPending,
            TenancyStatus::Ended { .. } => TenancyStatusKind::Ended,
        }
    }
    /// Whether this tenancy is **live** — accepted and not ended.
    ///
    /// This is the set that grants access (§3.3). `Pending` is not live (nobody has
    /// agreed yet) and `Ended` is not live (ending revokes access); everything
    /// between is, *including* `Delinquent` and `EvictionPending`, because pressure is
    /// strain, status and a story hook, never a locked door.
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            TenancyStatus::Active
                | TenancyStatus::Grace { .. }
                | TenancyStatus::Delinquent { .. }
                | TenancyStatus::EvictionPending { .. }
        )
    }
    /// Whether this tenancy has ended.
    #[must_use]
    pub fn is_ended(&self) -> bool {
        matches!(self, TenancyStatus::Ended { .. })
    }
    /// Whether the tenancy is in the failure ladder's "behind" states — the ones a
    /// `WhileDelinquent` inspection term admits.
    #[must_use]
    pub fn is_behind(&self) -> bool {
        matches!(
            self,
            TenancyStatus::Delinquent { .. } | TenancyStatus::EvictionPending { .. }
        )
    }
    /// The cadence position the current failure state started at, when it has one.
    #[must_use]
    pub fn since(&self) -> Option<i64> {
        match self {
            TenancyStatus::Grace { since } | TenancyStatus::Delinquent { since } => Some(*since),
            TenancyStatus::EvictionPending { window } => Some(window.opened),
            _ => None,
        }
    }
}

/// One entry of the bounded audit tail (≤ [`MAX_TENANCY_HISTORY`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TenancyEvent {
    /// Both sides accepted; the tenancy went live.
    Accepted {
        /// When (cadence position).
        at: i64,
    },
    /// The ladder moved.
    StatusChanged {
        /// From.
        from: TenancyStatusKind,
        /// To.
        to: TenancyStatusKind,
        /// When.
        at: i64,
    },
    /// The ruleset asserted a resident's means.
    PlanSubmitted {
        /// Whose.
        resident: ObjId,
        /// What.
        level: SupportLevel,
        /// When.
        at: i64,
    },
    /// A story-window re-notice went out.
    Notified {
        /// How many re-notices have now gone out.
        count: u8,
        /// When.
        at: i64,
    },
    /// The level-2 escalation opened.
    Escalated {
        /// The record.
        id: EscalationId,
        /// When.
        at: i64,
    },
    /// A tenant left the roster (a move-out, or boot reconciliation of a destroyed
    /// object).
    TenantRemoved {
        /// Who.
        who: ObjId,
        /// When.
        at: i64,
    },
    /// The tenancy ended, by this act.
    Ended {
        /// Which act.
        how: Resolution,
        /// When.
        at: i64,
    },
}

/// Who lives where, on which clock, under what terms.
///
/// **Loading re-checks [`Tenancy::new`]'s invariants** (`#[serde(try_from)]` through
/// a private `TenancyWire`): a persisted tenancy with no tenants would be a `Let` property
/// nobody may enter and nobody may end, and one whose guest list overlaps its roster,
/// or whose plan map is filed under the wrong resident, is a fail-open answer to
/// "who lives here".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TenancyWire")]
pub struct Tenancy {
    /// Identity (never reused).
    pub id: TenancyId,
    /// The property.
    pub property: PropertyId,
    /// The tenants; ≤ [`MAX_TENANTS`].
    pub tenants: BTreeSet<ObjId>,
    /// Tenant-managed guests; ≤ [`MAX_GUESTS`].
    #[serde(default)]
    pub guests: BTreeSet<ObjId>,
    /// The access policy (explicit terms only).
    #[serde(default)]
    pub terms: AccessTerms,
    /// Abstract plans or an exact obligation.
    #[serde(default)]
    pub funding: Funding,
    /// The E4 cadence clock this tenancy is assessed on (D-E5-14). Every `now` in
    /// this crate is a position on **this** clock — never wall time, never an IC:OOC
    /// ratio, never a calendar.
    pub cadence: CadenceId,
    /// The objects whose core `Obj.home` housing pointed into this property (§3.3,
    /// D-E5-11).
    #[serde(default)]
    pub homed: BTreeSet<ObjId>,
    /// Lifecycle.
    #[serde(default)]
    pub status: TenancyStatus,
    /// The bounded audit tail.
    #[serde(default)]
    pub history: Vec<TenancyEvent>,
    /// The story window that closed when the tenancy last recovered, stamped with the
    /// period it closed at (§3.2, [`crate::period_supported`]).
    ///
    /// Recovery used to drop the window and everything in it. Its `job` is the
    /// `wod-story` job E5b must resolve "recovered" and its `escalation` is the level-2
    /// record E5b must acknowledge — losing both left an open job and an open
    /// escalation pointing at a tenancy that is fine. It is also what the relapse
    /// hysteresis reads ([`crate::RELAPSE_WINDOW_PERIODS`]).
    #[serde(default)]
    pub last_window: Option<StoryWindow>,
    /// Staleness stamp.
    pub revision: u64,
}

impl Tenancy {
    /// Offer a tenancy: `Pending` until both sides accept ([`crate::accept`]).
    ///
    /// Enforces [`MAX_TENANTS`] and refuses an empty roster — a tenancy with no
    /// tenants would be a `Let` property nobody may enter and nobody may end.
    pub fn new(
        id: TenancyId,
        property: PropertyId,
        tenants: BTreeSet<ObjId>,
        cadence: CadenceId,
    ) -> Result<Tenancy, HousingError> {
        if tenants.is_empty() {
            return Err(HousingError::Invalid("a tenancy needs at least one tenant"));
        }
        if tenants.len() > MAX_TENANTS {
            return Err(HousingError::TooLarge("tenants per tenancy"));
        }
        Ok(Tenancy {
            id,
            property,
            tenants,
            guests: BTreeSet::new(),
            terms: AccessTerms::default(),
            funding: Funding::default(),
            cadence,
            homed: BTreeSet::new(),
            status: TenancyStatus::Pending,
            history: Vec::new(),
            last_window: None,
            revision: 1,
        })
    }

    /// Push one entry onto the bounded tail, dropping the oldest past
    /// [`MAX_TENANCY_HISTORY`] (the `ObligationEvent` FIFO).
    pub(crate) fn push_event(&mut self, event: TenancyEvent) {
        while self.history.len() >= MAX_TENANCY_HISTORY {
            self.history.remove(0);
        }
        self.history.push(event);
    }

    /// Whether `who` is a tenant.
    #[must_use]
    pub fn is_tenant(&self, who: ObjId) -> bool {
        self.tenants.contains(&who)
    }
    /// Whether `who` is a tenant-managed guest.
    #[must_use]
    pub fn is_guest(&self, who: ObjId) -> bool {
        self.guests.contains(&who)
    }

    /// Add a tenant-managed guest, enforcing [`MAX_GUESTS`].
    ///
    /// Refused once the tenancy has **ended**: an ended tenancy grants nothing, and
    /// letting somebody add a guest to it would be a standing invitation to a home
    /// they no longer hold.
    pub fn add_guest(&self, who: ObjId) -> Result<Tenancy, HousingError> {
        if self.status.is_ended() {
            return Err(HousingError::AlreadyEnded(self.id));
        }
        if self.guests.contains(&who) {
            return Ok(self.clone());
        }
        if self.is_tenant(who) {
            return Err(HousingError::Invalid("a tenant is not their own guest"));
        }
        if self.guests.len() >= MAX_GUESTS {
            return Err(HousingError::TooLarge("guests per tenancy"));
        }
        let mut next = self.clone();
        next.guests.insert(who);
        next.revision = next.revision.saturating_add(1);
        Ok(next)
    }

    /// Remove a guest. Always allowed, in every status — revoking access is never
    /// the dangerous direction.
    #[must_use]
    pub fn remove_guest(&self, who: ObjId) -> Tenancy {
        let mut next = self.clone();
        if next.guests.remove(&who) {
            next.revision = next.revision.saturating_add(1);
        }
        next
    }

    /// Remove a tenant from the roster (a boot reconciliation of a destroyed object,
    /// or one co-tenant leaving a shared lease).
    ///
    /// The **last** tenant may not be removed this way: that would end the tenancy
    /// without a [`Resolution`], which D-E5-1 forbids. Use [`crate::resolve`].
    ///
    /// Refused on an ended tenancy ([`HousingError::AlreadyEnded`], as
    /// [`Tenancy::add_guest`] is): the roster of an ended tenancy is the record of who
    /// lived there, and editing it after the fact rewrites what happened.
    ///
    /// **`homed` is deliberately left alone.** It is not a copy of the roster; it is
    /// the list of objects whose core `Obj.home` housing pointed at this property, and
    /// that pointer outlives the roster entry — dropping the id here is how a
    /// co-tenant keeps a live `Obj.home` into a property they no longer rent, and
    /// disperses *into* it when a room is destroyed (D-E5-11). It clears when the
    /// tenancy ends, through [`crate::to_unhome_on_end`], whose carve-out already
    /// skips anybody since re-homed somewhere else.
    pub fn remove_tenant(&self, who: ObjId, now: i64) -> Result<Tenancy, HousingError> {
        if self.status.is_ended() {
            return Err(HousingError::AlreadyEnded(self.id));
        }
        if !self.tenants.contains(&who) {
            return Ok(self.clone());
        }
        if self.tenants.len() <= 1 {
            return Err(HousingError::Invalid(
                "the last tenant leaves through an explicit resolution",
            ));
        }
        let mut next = self.clone();
        next.tenants.remove(&who);
        if let Funding::Abstract { plans } = &mut next.funding {
            plans.remove(&who);
        }
        next.push_event(TenancyEvent::TenantRemoved { who, at: now });
        next.revision = next.revision.saturating_add(1);
        Ok(next)
    }
}

/// The persisted shape of a [`Tenancy`], deserialized **through** [`Tenancy::new`]'s
/// checks (the [`PropertyWire`] rationale, applied to the record that answers "who
/// lives here").
#[derive(Deserialize)]
struct TenancyWire {
    id: TenancyId,
    property: PropertyId,
    tenants: BTreeSet<ObjId>,
    #[serde(default)]
    guests: BTreeSet<ObjId>,
    #[serde(default)]
    terms: AccessTerms,
    #[serde(default)]
    funding: Funding,
    cadence: CadenceId,
    #[serde(default)]
    homed: BTreeSet<ObjId>,
    #[serde(default)]
    status: TenancyStatus,
    #[serde(default)]
    history: Vec<TenancyEvent>,
    #[serde(default)]
    last_window: Option<StoryWindow>,
    revision: u64,
}

impl TryFrom<TenancyWire> for Tenancy {
    type Error = HousingError;

    fn try_from(w: TenancyWire) -> Result<Tenancy, HousingError> {
        let mut t = Tenancy::new(w.id, w.property, w.tenants, w.cadence)?;
        if w.guests.len() > MAX_GUESTS {
            return Err(HousingError::TooLarge("guests per tenancy"));
        }
        if w.guests.iter().any(|g| t.tenants.contains(g)) {
            return Err(HousingError::Invalid("a tenant is not their own guest"));
        }
        if w.history.len() > MAX_TENANCY_HISTORY {
            return Err(HousingError::TooLarge("tenancy history"));
        }
        if let Funding::Abstract { plans } = &w.funding {
            if plans.len() > MAX_TENANTS {
                return Err(HousingError::TooLarge("plans per tenancy"));
            }
            for (who, p) in plans {
                // The map key *is* the D-E5-13 guarantee that one resident's plan can
                // never overwrite another's; a document filed under somebody else is a
                // plan attributed to a resident who never asserted it.
                if p.resident != *who {
                    return Err(HousingError::Invalid("a plan is filed under another resident"));
                }
                if p.sources.len() > MAX_PLAN_SOURCES {
                    return Err(HousingError::TooLarge("plan sources"));
                }
            }
        }
        t.guests = w.guests;
        t.terms = w.terms;
        t.funding = w.funding;
        t.homed = w.homed;
        t.status = w.status;
        t.history = w.history;
        t.last_window = w.last_window;
        t.revision = w.revision;
        Ok(t)
    }
}

/// Record one resident's asserted means on an **abstract** tenancy (§3.1).
///
/// Three rules, all of them enforced here rather than trusted to the caller:
///
/// - the plan must name a **current tenant** — a plan for a stranger is
///   [`HousingError::NotATenant`], so a ruleset bug cannot support a lease with
///   somebody else's income;
/// - it is keyed by `plan.resident`, so it can only ever replace **that resident's**
///   previous plan (D-E5-13 — a co-tenant can neither raise nor drag down another's);
/// - an exact tenancy takes no plans at all ([`HousingError::WrongFunding`]): its
///   assessment is the E4 obligation's, and a plan there would be a second, silent
///   answer to the same question.
///
/// **A restatement is not news.** The ruleset's evaluator asserts the same level from
/// the same sources every period for a character whose circumstances have not changed,
/// and writing that down each time bumped a revision, wrote a store document and
/// pushed a `PlanSubmitted` entry — thirty-two of which are the whole bounded tail, so
/// a stable tenancy's audit history was a wall of identical plans with the acceptance,
/// the ladder and the escalation evicted out of it. So:
///
/// | the submission | the record |
/// |---|---|
/// | same `level` and `sources`, same `asserted_period` | a **pure no-op** — the same tenancy back, no revision, no event |
/// | same `level` and `sources`, a later period | the stored plan's freshness stamp moves (so [`crate::assess_at`] still sees a live answer); a revision, no event |
/// | a different `level` or `sources` | replaced, a revision, and a `PlanSubmitted` entry — the only submissions the tail keeps |
pub fn submit_plan(t: &Tenancy, plan: SupportPlan) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    if plan.sources.len() > MAX_PLAN_SOURCES {
        return Err(HousingError::TooLarge("plan sources"));
    }
    if !t.is_tenant(plan.resident) {
        return Err(HousingError::NotATenant(plan.resident));
    }
    let Some(plans) = t.funding.plans() else {
        return Err(HousingError::WrongFunding);
    };
    let previous = plans.get(&plan.resident);
    let restated =
        matches!(previous, Some(p) if p.level == plan.level && p.sources == plan.sources);
    if let Some(p) = previous
        && restated
        && p.asserted_period == plan.asserted_period
    {
        return Ok(t.clone());
    }
    if previous.is_none() && plans.len() >= MAX_TENANTS {
        return Err(HousingError::TooLarge("plans per tenancy"));
    }
    let (resident, level, at) = (plan.resident, plan.level, plan.asserted_at);
    let entry = match previous {
        Some(p) if restated => {
            let mut fresh = p.clone();
            fresh.asserted_at = plan.asserted_at;
            fresh.asserted_period = plan.asserted_period;
            fresh.revision = fresh.revision.saturating_add(1);
            fresh
        }
        _ => plan,
    };
    let mut next = t.clone();
    match &mut next.funding {
        Funding::Abstract { plans } => {
            plans.insert(resident, entry);
        }
        // Unreachable — `funding.plans()` above already refused an exact tenancy — and
        // kept anyway so the match cannot silently acquire a fall-through later.
        Funding::Exact { .. } => return Err(HousingError::WrongFunding),
    }
    if !restated {
        next.push_event(TenancyEvent::PlanSubmitted { resident, level, at });
    }
    next.revision = next.revision.saturating_add(1);
    Ok(next)
}
