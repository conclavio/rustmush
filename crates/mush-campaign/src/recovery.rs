//! **E7r1: R3 — the recovery provider** (10b §15.1 item 5; doc 10 §12 Q7A, §13 T7,
//! §12 S3A, §7 item 7A *funded, never infinite*).
//!
//! A player who loses everything gets a loaner, on a lease, without a staff decision.
//! The whole subsystem is **records plus one predicate plus one arm on the tick that
//! already exists** — no new engine machinery, no new crate, no new edge.
//!
//! # Funded, never infinite — and it never mints
//!
//! The provider has **two** exhaustible resources and stops granting when either runs
//! out:
//!
//! * **Hulls.** A loaner is one of the provider principal's own accessible
//!   operational assets, chosen by the lowest object id so the choice is
//!   deterministic. When it holds none, the queue **holds** and pages level 1.
//! * **Money.** A grant pays the applicant an outfitting stipend of one period's
//!   upkeep out of `pool_account`, and it refuses to take that account below
//!   `pool_floor`. When the pool is dry the queue **holds** and pages level 1.
//!
//! **Nothing here mints.** There is no faucet, no `create_system_obligation`, no
//! `mint_origin` and no minting `AssetOp` anywhere in this crate — 10b §3.11's
//! replenishment row is *the provider's own production work orders (cut 4) and
//! salvage recovery, funded by its budget — **never a mint into the pool*** — and the
//! `e7c9_p6` source probe pins the absence. A deployment that wants to subsidize the
//! provider does it the ordinary way, with a staff-authored system obligation under
//! `[economy].mint_cap`, outside this crate entirely. The stipend is a two-line,
//! zero-sum post, so the ledger total is unmoved by a grant.
//!
//! # An empty pool is not a decision (Q7A(f))
//!
//! Exhaustion **queues and pages level 1**. Staff are never the gate: eligibility is
//! a deterministic predicate over recorded facts, re-evaluated **at grant time**, and
//! the arm that grants is a pull on cut 9's ONE `campaign.operations` Coarse tick with
//! its own [`MAX_RECOVERY_PER_FIRE`] budget so it cannot starve the settlement lane.
//!
//! # No second lease, encumbrance or title model
//!
//! The grant is an ordinary E4b2 lease through `mush_economy::offer_lease` — the same
//! call E5b's housing makes — so activation places `Encumbrance::Lease { lessor,
//! contract }` through the economy's own `contract_encumbrance`, the upkeep is the
//! contract's own `ObligationTemplate` on the campaign cadence, and `end_lease` takes
//! the claim back off. This crate constructs no `Encumbrance` of its own and calls
//! `create_obligation` never.

use std::collections::BTreeSet;

use mush_core::{ClockStamp, Ctx, ObjId, World};
use mush_economy::{
    AccountId, Amount, AssetLifecycle, EconomicClock, EconomicSchedule, EconomyStore, GracePolicy,
    ObligationTemplate,
};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The most applicants one world may queue. **Refused on load, never truncated**: a
/// silently dropped applicant is a player who was told they were in the queue and is
/// not.
pub const MAX_RECOVERY_QUEUE: usize = 256;

/// The most live applications one principal may hold, whatever the configuration
/// says. The block's own `applications_per_principal` may narrow it and never widen
/// it.
pub const MAX_RECOVERY_APPLICATIONS_PER_PRINCIPAL: usize = 1;

/// The most titled assets [`loss_is_covered`] visits looking for one wreck
/// (E7r2 P0b).
///
/// Bounded on records **visited** (D-LBK-5). Past it the predicate answers `false`,
/// which is the fail-closed direction: the cooldown applies. It is deliberately larger
/// than [`RecoveryConfig::max_operational_assets`] can usefully be, because an
/// applicant eligible at all holds few assets by construction.
pub const MAX_RECOVERY_LOSS_VISIT: usize = 64;

/// The recovery arm's **own** per-fire budget on cut 9's `campaign.operations` tick.
///
/// It is a fourth arm beside `observe_activations`, `expire_grants` and the operation
/// rotation, each independently budgeted, so the recovery lane can never starve the
/// settlement lane (10b §15's contradiction (iii)).
pub const MAX_RECOVERY_PER_FIRE: usize = 8;

/// **The recovery books' retention ring** (independent verify fold, B7): how many
/// **closed** applications and how many **ended** grants one world keeps.
///
/// 10b §15.1 row 5 said *"three bounded whole-document exempt records"*;
/// [`MAX_RECOVERY_QUEUE`] bounded only the queue, while `recovery_apps` and
/// `recovery_grants` were `BTreeMap`s with no cap, no ring and no eviction, hydrated
/// entry by entry at boot, and a free player verb (`@recovery/apply`, deliberately
/// not eligibility-gated) created one of each. This is cut 10's muster ring, one file
/// away, applied to the same shape: a `Del` of the oldest closed/ended record in the
/// same bundle that writes a new one, and a bounded boot hydration that drops and
/// **counts** the overflow. A live application or a live grant is never evicted —
/// eviction is by policy over records that carry no claim, never truncation of live
/// state.
pub const MAX_RETAINED_RECOVERY_RECORDS: usize = 64;

/// **How long an offered loaner waits for its `@contract/accept`** before the
/// campaign tick's fourth arm takes it back (independent verify fold, D7).
///
/// `try_grant` completes at `offer_lease`, which produces a contract the applicant
/// must still accept; until they do, no `Encumbrance::Lease` exists and the hull is
/// held out of the motor pool by the grant document alone. Nothing aged that: the
/// fourth arm walked only the **queue**, which the applicant has already left, so an
/// applicant who applied, was granted and never accepted held a hull out of the pool
/// **for ever** and was `LiveLease`-refused themselves. `SEAT_OFFER_TTL`'s shape, on
/// the same cadence.
pub const RECOVERY_ACCEPT_TTL_SECS: i64 = 24 * 60 * 60;

/// The per-player cooldown after a lease ends — 10b §7's `RECOVERY_COOLDOWN` of
/// **14 days**, cited rather than redefined. `[tacsim.recovery].cooldown_days`
/// overrides it per deployment and defaults to exactly this.
pub const RECOVERY_COOLDOWN_SECS: i64 = 14 * 24 * 60 * 60;

/// The queue document's key.
pub const RECOVERY_QUEUE_KEY: &str = "recovery_queue";

/// The document key one applicant's application lives at.
#[must_use]
pub fn recovery_application_key(applicant: ObjId) -> SmolStr {
    SmolStr::new(format!("recovery_app:{}", applicant.0))
}

/// The document key one applicant's grant lives at.
#[must_use]
pub fn recovery_grant_key(applicant: ObjId) -> SmolStr {
    SmolStr::new(format!("recovery_grant:{}", applicant.0))
}

/// What a recovery record refused on.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryRecordError {
    /// A stored set past its cap. Refused, never truncated.
    #[error("a {what} holds {actual} entries; the maximum is {max}")]
    TooMany {
        /// Which set.
        what: &'static str,
        /// How many were stored.
        actual: usize,
        /// The cap.
        max: usize,
    },
    /// A field that must not be empty was.
    #[error("{0} is empty")]
    Empty(&'static str),
    /// A field that must not be zero was.
    #[error("{0} is zero")]
    Zero(&'static str),
    /// The queue named one applicant twice.
    #[error("the recovery queue names an applicant twice")]
    Duplicate,
}

// ---- the configuration (E7r1 R4's block, typed for this crate) ------------

/// The recovery provider's policy, handed in at composition from
/// `[tacsim.recovery]` (**E7r1 R4**).
///
/// `mush-campaign` takes no `ruleset-tacsim` edge, so the block arrives as this
/// crate's own type through `CampaignModule::with_recovery`, exactly as formation
/// kinds arrive through `FormationAuthority` — the composition root does the mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryConfig {
    /// Whether the provider grants at all.
    pub enabled: bool,
    /// The provider's budget account. Its **owner** is the provider principal: the
    /// lessor of every grant, and the principal whose hulls the loaners come from.
    pub pool_account: AccountId,
    /// The balance the pool may not be granted below.
    pub pool_floor: i64,
    /// The loaner tier table, cheapest first.
    pub tiers: Vec<SmolStr>,
    /// Live applications one principal may hold.
    pub applications_per_principal: u32,
    /// The per-player cooldown after a lease ends, in seconds.
    pub cooldown_secs: i64,
    /// The eligibility threshold, over **accessible** operational assets.
    pub max_operational_assets: u32,
    /// The insurance policy word.
    pub insurance: SmolStr,
    /// The lease's upkeep per campaign cadence period — and the outfitting stipend
    /// one grant costs the pool.
    pub upkeep_per_period: i64,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        RecoveryConfig {
            enabled: false,
            pool_account: AccountId(0),
            pool_floor: 0,
            tiers: Vec::new(),
            applications_per_principal: 1,
            cooldown_secs: RECOVERY_COOLDOWN_SECS,
            max_operational_assets: 0,
            insurance: SmolStr::new_static("approved_operations"),
            upkeep_per_period: 0,
        }
    }
}

impl RecoveryConfig {
    /// The tier an application without an explicit one takes: the cheapest.
    #[must_use]
    pub fn default_tier(&self) -> Option<&SmolStr> {
        self.tiers.first()
    }

    /// Whether `tier` is one this deployment declares.
    #[must_use]
    pub fn declares(&self, tier: &str) -> bool {
        self.tiers.iter().any(|t| t == tier)
    }

    /// Whether losses inside the lease's restrictions are written off rather than
    /// becoming a debt (Q7A's insurance row).
    ///
    /// **It governs debt-versus-writeoff and nothing else** (independent verify fold,
    /// C6/B2; owner decision F4(i)(b)). It used to gate the whole cooldown branch of
    /// [`eligible`] and the whole of [`cooldown_until`], and `"approved_operations"`
    /// is the shipped default in all three defaulting sites — so on every default
    /// deployment `cooldown_days` was dead code that the host nonetheless *validated*
    /// at boot, `@recovery/status` rendered no cooldown, and apply → grant → return →
    /// apply drew a fresh stipend out of the pool every cycle. Q7A's *"waived when
    /// the loss was inside the restrictions"* is a **per-loss** fact, and nothing in
    /// this crate records or reads whether any loss was; implementing it as "waived
    /// by the policy word, for everybody, always" was the defect. The per-loss
    /// waiver (a `loss_covered` marker on the grant) waits for the loss path that
    /// would set it — 10b §15.3's pass-2 item.
    #[must_use]
    pub fn insures_approved_operations(&self) -> bool {
        self.insurance == "approved_operations"
    }
}

// ---- the three records ----------------------------------------------------

/// **The queue** — one document naming every waiting applicant in claim order.
///
/// Claim order is the record: *first-eligible, then longest-waiting, ties by claim
/// time* (Q7A). A generation change mid-queue reorders nothing, because nothing in
/// the order is a clock reading — it is the position in this list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(into = "RecoveryQueueWire")]
pub struct RecoveryQueue {
    waiting: Vec<u32>,
}

/// The strict wire form of [`RecoveryQueue`].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryQueueWire {
    /// The waiting applicants' raw object ids, in claim order.
    #[serde(default)]
    pub waiting: Vec<u32>,
}

impl RecoveryQueue {
    /// The one constructor, and therefore the one place the invariants live.
    ///
    /// # Errors
    ///
    /// [`RecoveryRecordError::TooMany`] past [`MAX_RECOVERY_QUEUE`] — **refused, not
    /// truncated** — and [`RecoveryRecordError::Duplicate`] for a repeated applicant,
    /// which would give one player two positions.
    pub fn new(waiting: Vec<u32>) -> Result<RecoveryQueue, RecoveryRecordError> {
        if waiting.len() > MAX_RECOVERY_QUEUE {
            return Err(RecoveryRecordError::TooMany {
                what: "recovery queue",
                actual: waiting.len(),
                max: MAX_RECOVERY_QUEUE,
            });
        }
        let unique: BTreeSet<u32> = waiting.iter().copied().collect();
        if unique.len() != waiting.len() {
            return Err(RecoveryRecordError::Duplicate);
        }
        if waiting.contains(&0) {
            return Err(RecoveryRecordError::Zero("a queued applicant"));
        }
        Ok(RecoveryQueue { waiting })
    }

    /// The waiting applicants, in claim order.
    #[must_use]
    pub fn waiting(&self) -> Vec<ObjId> {
        self.waiting.iter().copied().map(ObjId).collect()
    }

    /// How many are waiting.
    #[must_use]
    pub fn len(&self) -> usize {
        self.waiting.len()
    }

    /// Whether nobody is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    /// One applicant's own position, one-based. `None` when they are not queued.
    ///
    /// **Only ever rendered to that applicant** — `@recovery/status` discloses no
    /// other applicant's identity or position.
    #[must_use]
    pub fn position_of(&self, applicant: ObjId) -> Option<usize> {
        self.waiting
            .iter()
            .position(|id| *id == applicant.0)
            .map(|index| index.saturating_add(1))
    }

    /// Join the queue at the back.
    ///
    /// # Errors
    ///
    /// [`RecoveryRecordError::TooMany`] at [`MAX_RECOVERY_QUEUE`] — a full queue
    /// refuses out loud rather than dropping the newest applicant.
    pub fn joined(&self, applicant: ObjId) -> Result<RecoveryQueue, RecoveryRecordError> {
        if self.waiting.contains(&applicant.0) {
            return Ok(self.clone());
        }
        let mut waiting = self.waiting.clone();
        waiting.push(applicant.0);
        RecoveryQueue::new(waiting)
    }

    /// Leave the queue. A no-op for somebody who was not in it.
    #[must_use]
    pub fn left(&self, applicant: ObjId) -> RecoveryQueue {
        RecoveryQueue {
            waiting: self
                .waiting
                .iter()
                .copied()
                .filter(|id| *id != applicant.0)
                .collect(),
        }
    }
}

impl From<RecoveryQueue> for RecoveryQueueWire {
    fn from(value: RecoveryQueue) -> RecoveryQueueWire {
        RecoveryQueueWire {
            waiting: value.waiting,
        }
    }
}

impl TryFrom<RecoveryQueueWire> for RecoveryQueue {
    type Error = RecoveryRecordError;
    fn try_from(value: RecoveryQueueWire) -> Result<RecoveryQueue, RecoveryRecordError> {
        RecoveryQueue::new(value.waiting)
    }
}

impl<'de> Deserialize<'de> for RecoveryQueue {
    fn deserialize<D>(deserializer: D) -> Result<RecoveryQueue, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RecoveryQueueWire::deserialize(deserializer)?;
        RecoveryQueue::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// Where one application is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApplicationState {
    /// Waiting; re-evaluated at grant time, never at queue time.
    Queued,
    /// Granted, and the grant record names the contract.
    Granted,
    /// Withdrawn by the applicant, or closed when their lease ended.
    Closed,
}

/// **One applicant's application** (`recovery_app:{obj}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "RecoveryApplicationWire")]
pub struct RecoveryApplication {
    applicant: ObjId,
    claimed_at: i64,
    tier: SmolStr,
    state: ApplicationState,
}

/// The strict wire form of [`RecoveryApplication`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryApplicationWire {
    /// The applicant.
    pub applicant: ObjId,
    /// The IC second the claim was made — the queue's tie-break.
    pub claimed_at: i64,
    /// The tier applied for.
    pub tier: SmolStr,
    /// Lifecycle.
    pub state: ApplicationState,
}

impl RecoveryApplication {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`RecoveryRecordError::Zero`] for a zero applicant and
    /// [`RecoveryRecordError::Empty`] for a tier with no name.
    pub fn new(
        applicant: ObjId,
        claimed_at: i64,
        tier: SmolStr,
        state: ApplicationState,
    ) -> Result<RecoveryApplication, RecoveryRecordError> {
        if applicant.0 == 0 {
            return Err(RecoveryRecordError::Zero("an application's applicant"));
        }
        if tier.trim().is_empty() {
            return Err(RecoveryRecordError::Empty("an application's tier"));
        }
        Ok(RecoveryApplication {
            applicant,
            claimed_at,
            tier,
            state,
        })
    }

    /// The applicant.
    #[must_use]
    pub const fn applicant(&self) -> ObjId {
        self.applicant
    }
    /// When they claimed.
    #[must_use]
    pub const fn claimed_at(&self) -> i64 {
        self.claimed_at
    }
    /// The tier applied for.
    #[must_use]
    pub fn tier(&self) -> &str {
        self.tier.as_str()
    }
    /// Lifecycle.
    #[must_use]
    pub const fn state(&self) -> ApplicationState {
        self.state
    }

    /// The post-image of a state change.
    #[must_use]
    pub fn with_state(&self, state: ApplicationState) -> RecoveryApplication {
        let mut next = self.clone();
        next.state = state;
        next
    }
}

impl From<RecoveryApplication> for RecoveryApplicationWire {
    fn from(value: RecoveryApplication) -> RecoveryApplicationWire {
        RecoveryApplicationWire {
            applicant: value.applicant,
            claimed_at: value.claimed_at,
            tier: value.tier,
            state: value.state,
        }
    }
}

impl TryFrom<RecoveryApplicationWire> for RecoveryApplication {
    type Error = RecoveryRecordError;
    fn try_from(value: RecoveryApplicationWire) -> Result<Self, RecoveryRecordError> {
        RecoveryApplication::new(value.applicant, value.claimed_at, value.tier, value.state)
    }
}

impl<'de> Deserialize<'de> for RecoveryApplication {
    fn deserialize<D>(deserializer: D) -> Result<RecoveryApplication, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RecoveryApplicationWire::deserialize(deserializer)?;
        RecoveryApplication::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// **One granted loaner** (`recovery_grant:{obj}`).
///
/// It records the *contract* and never a second lease model: the claim on the hull is
/// `Encumbrance::Lease { lessor, contract }`, placed by the economy's own
/// `contract_encumbrance` at activation and taken off by `end_lease`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "RecoveryGrantWire")]
pub struct RecoveryGrant {
    applicant: ObjId,
    contract: u64,
    loaner: ObjId,
    tier: SmolStr,
    granted_at: i64,
    ended_at: Option<i64>,
    loss_covered: bool,
}

/// The strict wire form of [`RecoveryGrant`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryGrantWire {
    /// Who holds the loaner.
    pub applicant: ObjId,
    /// The E4b2 contract the lease IS.
    pub contract: u64,
    /// The hull.
    pub loaner: ObjId,
    /// The tier granted.
    pub tier: SmolStr,
    /// When (IC seconds).
    pub granted_at: i64,
    /// When the lease ended, if it has — the cooldown runs from here.
    #[serde(default)]
    pub ended_at: Option<i64>,
    /// **Whether the loss this grant answers was an approved-operation loss**
    /// (E7 residue pass 2, P0b).
    ///
    /// `#[serde(default)]` so every grant written before pass 2 re-enters as
    /// `false` — the fail-closed direction, in which the cooldown applies exactly
    /// as pass 1 left it.
    #[serde(default)]
    pub loss_covered: bool,
}

impl RecoveryGrant {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`RecoveryRecordError::Zero`] for a zero applicant or loaner, and
    /// [`RecoveryRecordError::Empty`] for a tier with no name. **A contract id of
    /// zero is legal** — the economy's contract allocator starts at zero, and a
    /// record that refused it would refuse the world's very first lease.
    pub fn new(
        applicant: ObjId,
        contract: u64,
        loaner: ObjId,
        tier: SmolStr,
        granted_at: i64,
        ended_at: Option<i64>,
    ) -> Result<RecoveryGrant, RecoveryRecordError> {
        Self::with_cover(
            applicant, contract, loaner, tier, granted_at, ended_at, false,
        )
    }

    /// ---- E7r2: P0b — the same constructor, carrying the insured-loss marker ----
    ///
    /// [`RecoveryGrant::new`] is this with `loss_covered: false`, which is the shape
    /// every call site that cannot attest a covered loss keeps and the shape every
    /// pre-pass-2 document re-enters with. There is **one** constructor and therefore
    /// one place the invariants live: `TryFrom<RecoveryGrantWire>` re-runs this, so a
    /// hand-edited document carrying the marker is validated exactly as a fresh grant
    /// is.
    ///
    /// # Errors
    ///
    /// As [`RecoveryGrant::new`].
    pub fn with_cover(
        applicant: ObjId,
        contract: u64,
        loaner: ObjId,
        tier: SmolStr,
        granted_at: i64,
        ended_at: Option<i64>,
        loss_covered: bool,
    ) -> Result<RecoveryGrant, RecoveryRecordError> {
        if applicant.0 == 0 {
            return Err(RecoveryRecordError::Zero("a grant's applicant"));
        }
        if loaner.0 == 0 {
            return Err(RecoveryRecordError::Zero("a grant's loaner"));
        }
        if tier.trim().is_empty() {
            return Err(RecoveryRecordError::Empty("a grant's tier"));
        }
        Ok(RecoveryGrant {
            applicant,
            contract,
            loaner,
            tier,
            granted_at,
            ended_at,
            loss_covered,
        })
    }

    /// Who holds the loaner.
    #[must_use]
    pub const fn applicant(&self) -> ObjId {
        self.applicant
    }
    /// The contract the lease is.
    #[must_use]
    pub const fn contract(&self) -> u64 {
        self.contract
    }
    /// The hull.
    #[must_use]
    pub const fn loaner(&self) -> ObjId {
        self.loaner
    }
    /// The tier.
    #[must_use]
    pub fn tier(&self) -> &str {
        self.tier.as_str()
    }
    /// When it was granted.
    #[must_use]
    pub const fn granted_at(&self) -> i64 {
        self.granted_at
    }
    /// When it ended, if it has.
    #[must_use]
    pub const fn ended_at(&self) -> Option<i64> {
        self.ended_at
    }
    /// Whether the lease is still live.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        self.ended_at.is_none()
    }

    /// **Whether the loss this grant answers was an approved-operation loss**
    /// (E7r2 P0b, from pass 1's fold F4(i)(a)).
    ///
    /// Q7A's *"waived when the loss was inside the restrictions"* is a **per-loss**
    /// fact. Pass 1 found the shipped tree implementing it as *"waived by the policy
    /// word, for everybody, always"* — which made `cooldown_days` dead on every
    /// default deployment and let apply → grant → return → apply farm one stipend per
    /// cycle out of a pool nothing in this crate can mint into. The fold's fix was to
    /// make the cooldown **always** apply until a per-loss marker existed; this is
    /// that marker, and it is the waiver's **only** gate.
    ///
    /// It is set by [`loss_is_covered`] at grant time — a deterministic predicate over
    /// recorded facts, never a staff opinion and never the config's `insurance` word,
    /// which still governs debt-versus-writeoff and nothing else.
    #[must_use]
    pub const fn loss_covered(&self) -> bool {
        self.loss_covered
    }

    /// The post-image of an ending.
    #[must_use]
    pub fn ended(&self, at: i64) -> RecoveryGrant {
        let mut next = self.clone();
        next.ended_at = Some(at);
        next
    }
}

impl From<RecoveryGrant> for RecoveryGrantWire {
    fn from(value: RecoveryGrant) -> RecoveryGrantWire {
        RecoveryGrantWire {
            applicant: value.applicant,
            contract: value.contract,
            loaner: value.loaner,
            tier: value.tier,
            granted_at: value.granted_at,
            ended_at: value.ended_at,
            loss_covered: value.loss_covered,
        }
    }
}

impl TryFrom<RecoveryGrantWire> for RecoveryGrant {
    type Error = RecoveryRecordError;
    fn try_from(value: RecoveryGrantWire) -> Result<Self, RecoveryRecordError> {
        RecoveryGrant::with_cover(
            value.applicant,
            value.contract,
            value.loaner,
            value.tier,
            value.granted_at,
            value.ended_at,
            value.loss_covered,
        )
    }
}

impl<'de> Deserialize<'de> for RecoveryGrant {
    fn deserialize<D>(deserializer: D) -> Result<RecoveryGrant, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RecoveryGrantWire::deserialize(deserializer)?;
        RecoveryGrant::try_from(wire).map_err(serde::de::Error::custom)
    }
}

// ---- the deterministic eligibility predicate ------------------------------

/// Why one applicant is not eligible — **typed**, so the renderer collapses every
/// arm into one sentence for a viewer and staff get the reason (08c §6.2's D1 shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryRefusal {
    /// The provider is not enabled on this server.
    NotEnabled,
    /// They already hold a live loaner.
    LiveLease,
    /// They are inside the cooldown after a lease ended.
    Cooldown {
        /// The IC second the cooldown ends.
        until: i64,
    },
    /// They still hold accessible operational assets.
    HasAssets {
        /// How many.
        count: u32,
        /// The threshold.
        threshold: u32,
    },
}

impl RecoveryRefusal {
    /// The one line an applicant sees, whatever the reason.
    #[must_use]
    pub const fn public(self) -> &'static str {
        match self {
            RecoveryRefusal::NotEnabled => "Recovery is not enabled on this server.",
            RecoveryRefusal::LiveLease => "You already hold a loaner on a live lease.",
            RecoveryRefusal::Cooldown { .. } => {
                "You are inside the recovery cooldown from your last loaner."
            }
            RecoveryRefusal::HasAssets { .. } => {
                "You still have operational assets you can reach and deploy."
            }
        }
    }
}

/// **How many accessible operational assets a principal holds** — the deterministic
/// input the whole predicate rests on (doc 10 S3A: *accessible* operational assets,
/// unencumbered, reachable and deployable, **rather than total title**, so a player
/// whose only hull is an unreachable wreck still qualifies).
///
/// Three recorded facts, each read from the crate that owns it and none of them
/// invented here:
///
/// * **unencumbered** — `mush_economy`: the asset is titled to them, `Active`, not
///   frozen and carries no claim (a leased or collateralized hull is not theirs to
///   fly);
/// * **reachable** — `mush_sim`: it is placed in the spatial graph and not
///   quarantined (a wreck in the recovery room is not reachable);
/// * **deployable** — **campaign-side**, from the formation records cuts 9 and 10
///   already own. It is deliberately **not** a `mush-tactics` read: `mush-campaign`
///   has no such edge and this pass adds none, and the formation membership *is* the
///   campaign's own answer to "can this be put in the field".
///
/// Pure over the state handed to it, so two evaluations of one world agree and the
/// answer is byte-identical across a restart.
#[must_use]
pub fn accessible_operational_assets(world: &World, who: ObjId) -> u32 {
    let principal = PrincipalId::Object(who);
    let Some(economy) = world.module_state::<EconomyStore>() else {
        return 0;
    };
    let mut count = 0_u32;
    for asset in economy.snapshot().assets() {
        if asset.title_owner != principal {
            continue;
        }
        if is_accessible_operational(world, asset.object) {
            count = count.saturating_add(1);
        }
    }
    count
}

/// **The per-asset half of [`accessible_operational_assets`], as one predicate**
/// (independent verify fold, C7) — everything except *whose* the hull is.
///
/// It exists because the eligibility side and the loaner-selection side had drifted
/// into two different notions of the same phrase: `accessible_operational_assets`
/// required unencumbered **and reachable and deployable**, while
/// [`next_loaner`]'s own filter checked only the first four conditions and its doc
/// comment claimed *"the provider's own lowest-id accessible operational asset"*
/// anyway. So a hull sitting unplaced in limbo, a quarantined wreck, or a hull in no
/// formation was a valid loaner for a player whose identical hull would have made
/// them **ineligible**. One predicate, two callers, and the two notions can no longer
/// disagree.
#[must_use]
pub fn is_accessible_operational(world: &World, object: ObjId) -> bool {
    let Some(economy) = world.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(asset) = economy.snapshot().asset(object) else {
        return false;
    };
    if asset.frozen
        || !matches!(asset.lifecycle, AssetLifecycle::Active)
        || !asset.encumbrances.is_empty()
    {
        return false;
    }
    // **reachable** — `mush_sim`: placed in the spatial graph and not quarantined.
    let reachable = world
        .module_state::<mush_sim::SpatialState>()
        .is_some_and(|st| st.is_placed(object) && !st.is_quarantined(object));
    reachable && is_deployable(world, object)
}

/// **Deployable, answered campaign-side** — the object is a member of a formation
/// this world records. Never a `mush-tactics` read (10b §15's OPEN 4 default): the
/// edge set is frozen and `cargo tree -p mush-campaign` is the proof.
#[must_use]
pub fn is_deployable(world: &World, object: ObjId) -> bool {
    let Some(store) = world.module_state::<crate::store::CampaignStore>() else {
        return false;
    };
    let unit = crate::record::UnitRef(object);
    store.formations().any(|node| node.members.contains(&unit))
}

/// **The predicate**, re-evaluated **at grant time and not at queue time** (Q7A).
///
/// Pure over the world handed to it: no clock is read beyond the `now` the caller
/// passes, no RNG is drawn, no staff opinion is consulted. `Ok(())` is eligible.
///
/// # Errors
///
/// [`RecoveryRefusal`], one typed code per reason.
pub fn eligible(
    world: &World,
    cfg: &RecoveryConfig,
    applicant: ObjId,
    grant: Option<&RecoveryGrant>,
    now: i64,
) -> Result<(), RecoveryRefusal> {
    if !cfg.enabled {
        return Err(RecoveryRefusal::NotEnabled);
    }
    if let Some(grant) = grant {
        if grant.is_live() {
            return Err(RecoveryRefusal::LiveLease);
        }
        // The cooldown runs from the END of the last lease and **always applies**
        // (independent verify fold, C6/B2; owner decision F4(i)(b)). The
        // `!cfg.insures_approved_operations() &&` conjunct that used to guard this
        // branch made `cooldown_days` dead on every shipped default, which is how
        // apply → grant → return → apply farmed one stipend per cycle out of a pool
        // nothing in this crate can mint into. `insurance` now governs
        // debt-versus-writeoff and nothing else; see
        // [`RecoveryConfig::insures_approved_operations`].
        //
        // **E7r2 P0b adds the one waiver, and it is per-loss.** A grant whose end
        // carries [`RecoveryGrant::loss_covered`] skips the cooldown — that is Q7A's
        // *"waived when the loss was inside the restrictions"*, implemented as the
        // per-loss fact it always was. The marker is written once, at grant time, by
        // [`loss_is_covered`]'s deterministic predicate; a grant that does not carry
        // it waits exactly as pass 1 left it, and a document written before pass 2
        // re-enters as `false`. The farm the fold closed stays closed, because a
        // second cycle can only be covered by a second *recorded* approved-operation
        // loss.
        if let Some(ended) = grant.ended_at()
            && !grant.loss_covered()
        {
            let until = ended.saturating_add(cfg.cooldown_secs);
            if now < until {
                return Err(RecoveryRefusal::Cooldown { until });
            }
        }
    }
    let count = accessible_operational_assets(world, applicant);
    if count > cfg.max_operational_assets {
        return Err(RecoveryRefusal::HasAssets {
            count,
            threshold: cfg.max_operational_assets,
        });
    }
    Ok(())
}

// ---- the provider's two exhaustible resources -----------------------------

/// The provider principal: the **owner of the pool account**. One value, derived in
/// one place, so the lessor, the loaner's owner and the payer can never disagree.
#[must_use]
pub fn provider(world: &World, cfg: &RecoveryConfig) -> Option<PrincipalId> {
    world
        .module_state::<EconomyStore>()?
        .snapshot()
        .account(cfg.pool_account)
        .map(|account| account.owner.clone())
}

/// The pool's posted balance, or `None` when the pool account does not resolve.
#[must_use]
pub fn pool_balance(world: &World, cfg: &RecoveryConfig) -> Option<i64> {
    Some(
        world
            .module_state::<EconomyStore>()?
            .snapshot()
            .account(cfg.pool_account)?
            .posted_balance
            .minor(),
    )
}

/// **Whether the pool can fund one more grant** without going below `pool_floor`.
///
/// The floor is what makes the budget *exhaustible*: at it, the provider stops
/// granting, the queue holds and a level-1 page goes out. It never goes negative
/// because it never tries — and if it did, the engine's own invariant 26 would refuse
/// the post before anything moved.
#[must_use]
pub fn pool_can_fund(world: &World, cfg: &RecoveryConfig) -> bool {
    let Some(balance) = pool_balance(world, cfg) else {
        return false;
    };
    balance.saturating_sub(cfg.upkeep_per_period) >= cfg.pool_floor
}

/// **The next loaner hull** — the provider's own lowest-id
/// [`is_accessible_operational`] asset that is not already out on a live grant.
///
/// Deterministic (the asset table is canonically ordered and the choice is the
/// minimum), so two evaluations agree and a restart picks the same hull. `None` is
/// an empty motor pool, which **queues and pages** exactly as a dry budget does.
///
/// **"Already out" is the MONEY fact** (independent verify fold, D3; owner decision
/// F6(a)): a live lease this provider offered on the hull, which is durable through
/// the economy's own `install_commit`. It used to be the live **grant document**, an
/// ordinary deferred campaign write, and that cut both ways: lose the write and the
/// next fire leased the same hull twice; lose the *close* write and the hull was out
/// of the motor pool and its applicant `LiveLease`-refused **for ever**, because the
/// document was the only thing anybody asked. R1 made the live hold found by purpose
/// the marker one crate over, for the same reason.
#[must_use]
pub fn next_loaner(world: &World, cfg: &RecoveryConfig) -> Option<ObjId> {
    let owner = provider(world, cfg)?;
    let economy = world.module_state::<EconomyStore>()?;
    economy
        .snapshot()
        .assets()
        .filter(|asset| asset.title_owner == owner)
        .map(|asset| asset.object)
        .filter(|object| {
            is_accessible_operational(world, *object)
                && live_lease_on(world, &owner, *object).is_none()
        })
        .min_by_key(|object| object.0)
}

/// **The live lease this provider holds on one hull, if any — the MONEY fact**
/// (independent verify fold, D3; owner decision F6(a)).
///
/// `mush_economy::end_lease`'s `owns_lease` shape, read rather than written: a
/// non-terminal contract issued by `lessor` that pledges exactly `premises` as the
/// lessor's collateral and creates an obligation. That is what `offer_lease` builds,
/// and it is durable through the economy's `install_commit` — unlike the three
/// campaign documents, which are ordinary deferred writes and can be lost on their
/// own.
#[must_use]
pub fn live_lease_on(
    world: &World,
    lessor: &PrincipalId,
    premises: ObjId,
) -> Option<mush_economy::ContractId> {
    let economy = world.module_state::<EconomyStore>()?;
    economy
        .snapshot()
        .contracts()
        .find(|contract| {
            !contract.state.is_terminal()
                && contract.issuer == *lessor
                && contract.consent.terms.deliverables.iter().any(|d| {
                    matches!(
                        d,
                        mush_economy::Deliverable::Collateral { object, from }
                            if *object == premises && from == lessor
                    )
                })
                && contract
                    .consent
                    .terms
                    .deliverables
                    .iter()
                    .any(|d| matches!(d, mush_economy::Deliverable::Obligation { .. }))
        })
        .map(|contract| contract.id)
}

/// **The live recovery lease one applicant already holds** — the authority on
/// *"already granted"*, read before any campaign document (D3; F6(a)).
///
/// A lost `recovery_grant:` document used to mean the next fire leased the applicant
/// a **second** hull and posted a **second** stipend, because the campaign mirror was
/// the only marker and `grant_key(applicant, now)` was scoped to the IC second.
/// R1 got this right one crate over by making the live hold found *by purpose* the
/// marker; this is the same move for a lease.
#[must_use]
pub fn live_recovery_lease(
    world: &World,
    cfg: &RecoveryConfig,
    applicant: ObjId,
) -> Option<(mush_economy::ContractId, ObjId)> {
    let lessor = provider(world, cfg)?;
    let tenant = PrincipalId::Object(applicant);
    let economy = world.module_state::<EconomyStore>()?;
    economy
        .snapshot()
        .contracts()
        .filter(|contract| {
            !contract.state.is_terminal()
                && contract.issuer == lessor
                && contract.consent.parties.contains(&tenant)
        })
        .find_map(|contract| {
            contract
                .consent
                .terms
                .deliverables
                .iter()
                .find_map(|d| match d {
                    mush_economy::Deliverable::Collateral { object, from } if *from == lessor => {
                        Some((contract.id, *object))
                    }
                    _ => None,
                })
        })
}

/// The upkeep obligation one grant's lease carries: the applicant pays the provider
/// `upkeep_per_period` on the **campaign cadence**, with the economy's own default
/// grace ladder. Built here and created by the economy at contract activation — this
/// crate calls `create_obligation` never.
#[must_use]
pub fn upkeep_template(
    cfg: &RecoveryConfig,
    provider: &PrincipalId,
    applicant: &PrincipalId,
    applicant_account: AccountId,
    now: i64,
) -> ObligationTemplate {
    ObligationTemplate {
        debtor: applicant.clone(),
        creditor: provider.clone(),
        debit_account: applicant_account,
        credit_account: cfg.pool_account,
        amount: Amount::from_minor(cfg.upkeep_per_period),
        schedule: EconomicSchedule::new(
            EconomicClock::RealUtc,
            now.saturating_add(i64::try_from(crate::tick::TICK_PERIOD_SECS).unwrap_or(15)),
            Some(i64::try_from(crate::tick::TICK_PERIOD_SECS).unwrap_or(15)),
            1,
        ),
        grace: GracePolicy::default(),
        notify_on_success: false,
    }
}

/// The IC second one applicant's cooldown ends, for `@recovery/status`.
///
/// Its early return on [`RecoveryConfig::insures_approved_operations`] was
/// [`eligible`]'s twin and went with it (independent verify fold, C6; owner decision
/// F4(i)(b)): a cooldown that bites must also be a cooldown a player can *see*.
#[must_use]
pub fn cooldown_until(cfg: &RecoveryConfig, grant: Option<&RecoveryGrant>) -> Option<i64> {
    // E7r2 P0b: one predicate, two readers. `@recovery/status` renders exactly the
    // cooldown `eligible` enforces, so a covered grant reads "no cooldown" and is in
    // fact refused nothing — the render and the gate cannot drift.
    grant
        .filter(|grant| !grant.loss_covered())
        .and_then(RecoveryGrant::ended_at)
        .map(|ended| ended.saturating_add(cfg.cooldown_secs))
}

/// ---- E7r2: P0b — **was this applicant's loss an approved-operation loss?** ----
///
/// The deterministic predicate the marker is written from, evaluated **once, at grant
/// time**, over recorded facts and nothing else (Q7A: staff are never the gate). It
/// answers true when the applicant has a hull that is **both**:
///
/// * a **wreck** — a carrier root whose journey reached a terminal loss state,
///   `JourneyState::{Failed, Captured { .. }}`, which is cut 6's own record and the
///   same fact E7 residue pass 2's salvage source reports; and
/// * an **approved operational asset** — a member of a formation this world records,
///   which is [`is_deployable`], the campaign-side deployability predicate
///   [`is_accessible_operational`] already uses. A hull that was never in a formation
///   was not lost on an operation this campaign approved.
///
/// Both halves are read from state this crate already reads — `mush_sim`'s
/// `JourneyBook` and this crate's own `CampaignStore` — so the edge set is unchanged
/// and `cargo tree -p mush-campaign` is byte-identical.
///
/// **Bounded**: the walk is over the applicant's own titled assets, capped at
/// [`MAX_RECOVERY_LOSS_VISIT`] records **visited** (D-LBK-5), and a census past the
/// cap answers `false` rather than paging — the fail-closed direction, in which the
/// cooldown applies.
#[must_use]
pub fn loss_is_covered(world: &World, applicant: ObjId) -> bool {
    let Some(economy) = world.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(book) = world.module_state::<mush_sim::JourneyBook>() else {
        return false;
    };
    let owner = PrincipalId::Object(applicant);
    economy
        .snapshot()
        .assets_of(&owner)
        .take(MAX_RECOVERY_LOSS_VISIT)
        .any(|asset| {
            book.get(asset.object).is_some_and(|journey| {
                matches!(
                    journey.state(),
                    mush_sim::JourneyState::Failed | mush_sim::JourneyState::Captured { .. }
                )
            }) && is_deployable(world, asset.object)
        })
}

/// The IC second a `ClockStamp` names, for the records above.
#[must_use]
pub fn ic_seconds(now: &ClockStamp) -> i64 {
    now.at
}

/// A read-only handle on the composed policy, so a command can reach it
/// synchronously through `World::module_state`.
#[derive(Clone, Debug, Default)]
pub struct RecoveryPolicy(RecoveryConfig);

impl RecoveryPolicy {
    /// Wrap the composed block.
    #[must_use]
    pub fn new(cfg: RecoveryConfig) -> RecoveryPolicy {
        RecoveryPolicy(cfg)
    }
    /// The policy.
    #[must_use]
    pub const fn config(&self) -> &RecoveryConfig {
        &self.0
    }
}

/// The composed policy, or the disabled default — **fail-closed**: an unwired
/// composition grants nothing and says so.
#[must_use]
pub fn policy(ctx: &Ctx<'_>) -> RecoveryConfig {
    ctx.w
        .module_state::<RecoveryPolicy>()
        .map(|p| p.config().clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e7r1_r3_the_caps_are_encoded_and_the_cooldown_is_cited() {
        assert_eq!(MAX_RECOVERY_QUEUE, 256);
        assert_eq!(MAX_RECOVERY_APPLICATIONS_PER_PRINCIPAL, 1);
        assert_eq!(MAX_RECOVERY_PER_FIRE, 8);
        // 10b §7's `RECOVERY_COOLDOWN` 14 d, in seconds.
        assert_eq!(RECOVERY_COOLDOWN_SECS, 14 * 24 * 60 * 60);
        // Its own budget, independent of the operation rotation's, so neither lane
        // starves the other.
        assert_eq!(crate::tick::MAX_WORKED_PER_FIRE, 16);
        const { assert!(MAX_RECOVERY_PER_FIRE < crate::tick::MAX_VISITED_PER_FIRE) }
    }

    #[test]
    fn e7r1_r3_the_queue_refuses_over_cap_and_never_truncates() {
        let full: Vec<u32> = (1..=u32::try_from(MAX_RECOVERY_QUEUE).unwrap()).collect();
        let queue = RecoveryQueue::new(full.clone()).expect("exactly the cap loads");
        assert_eq!(queue.len(), MAX_RECOVERY_QUEUE);
        let mut over = full;
        over.push(9_999);
        let error = RecoveryQueue::new(over).expect_err("one past it is refused");
        assert!(matches!(error, RecoveryRecordError::TooMany { .. }));
        // Refused, not truncated: the constructor returns no queue at all.
        assert!(queue.joined(ObjId(9_999)).is_err());
    }

    #[test]
    fn e7r1_r3_the_queue_refuses_a_duplicate_applicant() {
        assert!(RecoveryQueue::new(vec![3, 4, 3]).is_err());
        let queue = RecoveryQueue::new(vec![3, 4]).expect("fine");
        // Joining twice is idempotent, never a second position.
        let again = queue.joined(ObjId(3)).expect("idempotent");
        assert_eq!(again.waiting(), queue.waiting());
        assert_eq!(again.position_of(ObjId(3)), Some(1));
        assert_eq!(again.position_of(ObjId(4)), Some(2));
        assert_eq!(again.position_of(ObjId(5)), None);
    }

    #[test]
    fn e7r1_r3_every_record_re_enters_through_its_wire() {
        let queue = RecoveryQueue::new(vec![7, 8]).expect("queue");
        let back: RecoveryQueue =
            serde_json::from_value(serde_json::to_value(&queue).expect("encode")).expect("decode");
        assert_eq!(back, queue);

        let app = RecoveryApplication::new(
            ObjId(7),
            1_000,
            SmolStr::new_static("scout"),
            ApplicationState::Queued,
        )
        .expect("application");
        let back: RecoveryApplication =
            serde_json::from_value(serde_json::to_value(&app).expect("encode")).expect("decode");
        assert_eq!(back, app);

        let grant = RecoveryGrant::new(
            ObjId(7),
            3,
            ObjId(11),
            SmolStr::new_static("scout"),
            1_000,
            None,
        )
        .expect("grant");
        let back: RecoveryGrant =
            serde_json::from_value(serde_json::to_value(&grant).expect("encode")).expect("decode");
        assert_eq!(back, grant);
        assert!(grant.is_live());
        assert!(!grant.ended(2_000).is_live());

        // A forged document re-runs the constructor and is refused. **A contract of
        // zero is legal** — the economy's allocator starts there, and refusing it
        // would refuse the world's very first lease — so the forgery is a zero
        // loaner, which no grant can have.
        assert!(
            serde_json::from_value::<RecoveryGrant>(serde_json::json!({
                "applicant": 7, "contract": 3, "loaner": 0, "tier": "scout",
                "granted_at": 1, "ended_at": null
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<RecoveryGrant>(serde_json::json!({
                "applicant": 7, "contract": 0, "loaner": 11, "tier": "scout",
                "granted_at": 1, "ended_at": null
            }))
            .is_ok(),
            "contract #0 is the first lease a world offers"
        );
    }
}
