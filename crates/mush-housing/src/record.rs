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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn remove_room(&self, room: ObjId) -> Result<Property, HousingError> {
        if room == self.anchor {
            return Err(HousingError::Invalid("the anchor room cannot be removed"));
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

// ---------------------------------------------------------------------------
// Access terms
// ---------------------------------------------------------------------------

/// When an inspection term admits its holder. There is no implied landlord entry and
/// no "delinquency unlocks the door" (D-E5-4): legal authority, economic title and
/// physical access stay separate, and inspection is an **explicit** policy term
/// (owner answer 9).
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum InspectionWhen {
    /// Whenever the tenancy is live.
    #[default]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportPlan {
    /// Whose means this describes.
    pub resident: ObjId,
    /// The asserted level, opaque `0..=5`.
    pub level: SupportLevel,
    /// Attribution only; ≤ [`MAX_PLAN_SOURCES`].
    #[serde(default)]
    pub sources: Vec<SupportSource>,
    /// The cadence position this was asserted at.
    pub asserted_at: i64,
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
    ) -> Result<SupportPlan, HousingError> {
        if sources.len() > MAX_PLAN_SOURCES {
            return Err(HousingError::TooLarge("plan sources"));
        }
        Ok(SupportPlan { resident, level, sources, asserted_at, revision: 1 })
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn remove_tenant(&self, who: ObjId, now: i64) -> Result<Tenancy, HousingError> {
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
        next.homed.remove(&who);
        if let Funding::Abstract { plans } = &mut next.funding {
            plans.remove(&who);
        }
        next.push_event(TenancyEvent::TenantRemoved { who, at: now });
        next.revision = next.revision.saturating_add(1);
        Ok(next)
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
    let mut next = t.clone();
    let (resident, level, at) = (plan.resident, plan.level, plan.asserted_at);
    match &mut next.funding {
        Funding::Abstract { plans } => {
            if !plans.contains_key(&resident) && plans.len() >= MAX_TENANTS {
                return Err(HousingError::TooLarge("plans per tenancy"));
            }
            plans.insert(resident, plan);
        }
        Funding::Exact { .. } => return Err(HousingError::WrongFunding),
    }
    next.push_event(TenancyEvent::PlanSubmitted { resident, level, at });
    next.revision = next.revision.saturating_add(1);
    Ok(next)
}
