//! Doc 20 cut SM.6 — the industry seams a game layer composes at the root
//! (§3.4, D-20-5, D-20-12; 10b §3.3's cut-3 handoffs).
//!
//! Every trait here is **declared** in this crate and **implemented** by the
//! composition root — `crate::workorder::RecipeAuthority`'s shape, and for the
//! same reason: this crate is neutral and has no edge to the game that works a
//! surface block. Unwired, each seam is fail-closed or a no-op:
//!
//! * no [`SystemOpener`]: `DepositOpen` stays staff-only, as before;
//! * no [`ExternalDraws`]: a deposit with `external` set is quarantined
//!   `ExternalUnwired` at every sweep (nothing drains a block pool behind the
//!   provider's back, and nobody can work it);
//! * no [`JurisdictionAuthority`]: the default [`ActsForJurisdiction`], which a
//!   deposit with no recorded jurisdiction never matches;
//! * no [`TenureAuthority`] (cut SM.10): `industry_claim_tenure` registers and
//!   offers nothing.

use std::sync::{Arc, OnceLock};

use mush_core::{ActionTxn, ClockStamp, Engine, ModuleCtx, ObjId};
use mush_economy::ContractId;
use mush_inventory::EndpointRef;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::families::IndustryFamilies;
use crate::record::{ClaimId, CurvePoint, OreDeposit, OreDepositId};

/// The composition root's cell for this crate's handles
/// (`mush_inventory::InventoryHandleCell`'s shape): filled by
/// `IndustryModule::register` right after the families are built, read by
/// providers composed before it registers (a [`SystemOpener`] composed onto this
/// module) or after it (the game's port tick and its site-work provider). It
/// carries a **capability**, not authority: writing an industry family still
/// needs this owner's transaction or a delegation
/// (`IndustryModule::with_state_delegates`).
pub type IndustryHandleCell = Arc<OnceLock<IndustryFamilies>>;

/// One server-authored opening a system opener may make (doc 20 §3.4,
/// D-20-5): everything `OreDeposit::new` needs, plus the provider that works
/// the body and the issuer whose jurisdiction it lies in. The deposit's id is
/// derived from `receipt` alone (`derive_system_deposit_id`), so a second
/// opening of one receipt refuses whoever submits it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemOpening {
    /// The replay guard and id source: `1..=96` bytes of `[a-z0-9_./-]`.
    pub receipt: SmolStr,
    /// The place the body sits at.
    pub place: ObjId,
    /// What comes out of it.
    pub commodity: mush_core::CanonicalId,
    /// Its grade (cut 2's `Grade`, checked at the prepare).
    pub grade: u8,
    /// Its accessibility, `0..=100`.
    pub accessibility: u8,
    /// The recorded grade band a survey copies verbatim.
    pub grade_class: String,
    /// The recorded reserve band a survey copies verbatim.
    pub reserve_class: String,
    /// The depletion curve.
    pub curve: Vec<CurvePoint>,
    /// The endpoint its pool lives at.
    pub endpoint: EndpointRef,
    /// The reserve credited to that pool. **Staff-only**: never in an outcome.
    pub reserve: u64,
    /// The provider that works it (`OreDeposit::external`).
    pub external: SmolStr,
    /// The issuer whose jurisdiction it lies in (`OreDeposit::jurisdiction`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<PrincipalId>,
}

/// A composed admission of a non-staff actor to `DepositOpen` (doc 20 §3.4): the
/// ONE opening `actor` makes next, server-authored, in the opener's own order;
/// `None` when `actor` is no opener or has nothing left to open. Called from
/// `project_offer` only (an `&Engine` and no transaction). One a call, because
/// the opener acts once a fire and a whole site's openings in one offer could
/// pass `MAX_ACTION_PARAMETER_BYTES` (reviewer F5).
pub trait SystemOpener: Send + Sync {
    /// The next opening `actor` may make, if any.
    fn next_opening(&self, engine: &Engine, actor: ObjId) -> Option<SystemOpening>;
}

/// The composed answer to "how much has the external provider drawn from this
/// deposit" (doc 20 §3.4; 10b §3.3 handoff). The sweep checks
/// `initial_reserve − pool == drawn` for every deposit whose `external` is set,
/// and quarantines the deposit's whole block on a mismatch. `None` = cannot
/// answer (fail closed: quarantine).
///
/// It takes a [`ModuleCtx`], not a `&World` (cut SM.6 D11): a typed family is
/// readable only through a module context, owner- or delegation-gated, and the
/// provider's own record of what it drew is one.
pub trait ExternalDraws: Send + Sync {
    /// What the external provider has drawn from `deposit`'s pool, in units.
    fn drawn(&self, ctx: &ModuleCtx<'_>, deposit: &OreDeposit) -> Option<u64>;
}

/// The contest offer's jurisdiction row (doc 20 §9.6, D-20-12): the claims,
/// beyond those whose issuer `actor` acts for, whose contest state `actor` may
/// move. Composed at the root; the default is [`ActsForJurisdiction`]. SM.10
/// composes its own to carry the Registrar's rule inside a player's own renewal
/// ("carried inside the player's own acts"). Bounded like
/// `provider::administrable_claims`: at most `MAX_DEPOSIT_CHOICES`, sorted.
pub trait JurisdictionAuthority: Send + Sync {
    /// The claims `actor` may move the contest state of by jurisdiction.
    fn claims(&self, engine: &Engine, families: &IndustryFamilies, actor: ObjId) -> Vec<ClaimId>;
}

/// The default row: every claim over a `Deposit` whose `jurisdiction()` `actor`
/// `acts_for` — so an issuer may suspend a foreign claim at its own sites and
/// nowhere else (`provider::jurisdiction_claims`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ActsForJurisdiction;

impl JurisdictionAuthority for ActsForJurisdiction {
    fn claims(&self, engine: &Engine, families: &IndustryFamilies, actor: ObjId) -> Vec<ClaimId> {
        crate::provider::jurisdiction_claims(engine, families, actor)
    }
}

// ---- doc 20 cut SM.10: the composed tenure authority (§9, D-20-12) ----

/// Most claims one [`TenureAct`] list may name: a surface block holds at most
/// four opened deposits (one per ore family; tacsim's
/// `surface::opening::MAX_DEPOSITS_PER_BLOCK`), so a block-wide act names at
/// most four claims. A longer list refuses `InvalidRequest`.
pub const MAX_TENURE_CLAIMS: usize = 4;

/// Most bytes [`TenureAct::summary`] may carry.
pub const MAX_TENURE_SUMMARY_BYTES: usize = 200;

/// Most bytes [`TenureAct::game`] may encode to.
pub const MAX_TENURE_GAME_BYTES: usize = 1024;

/// One claim at the revision the act was planned against (doc 20 §9; cut
/// SM.10). The prepare re-reads it under that checked revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimRev {
    /// The claim.
    pub claim: ClaimId,
    /// Its revision when the act was planned.
    pub revision: u64,
}

/// **One server-authored tenure act over ONE block place** (doc 20 §9,
/// D-20-12; cut SM.10), in [`SystemOpening`]'s shape: the composed
/// [`TenureAuthority`] vouches for exactly this row, the offer carries it as a
/// fixed parameter `merge_parameters` will not let a submission overwrite, and
/// its `receipt` is the offer's one `Choice`. Every list may be empty (a block
/// with no opened deposit; reviewer F4) and none may pass
/// [`MAX_TENURE_CLAIMS`]. The `i64` stamps are IC seconds on the offer's own
/// `now` clock: the prepare builds `ClockStamp { clock: now.clock, at }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TenureAct {
    /// The replay guard: `1..=96` bytes of `[a-z0-9_./-]`
    /// ([`is_receipt`](crate::families::is_receipt)); the offer's one `Choice`.
    pub receipt: SmolStr,
    /// The actor the row was projected for (re-checked at the prepare).
    pub actor: ObjId,
    /// The block place every named claim and deposit must lie in.
    pub place: ObjId,
    /// What the act does.
    pub kind: TenureKind,
    /// The outcome's summary, at most [`MAX_TENURE_SUMMARY_BYTES`]; never a
    /// count, a stamp or a reserve.
    pub summary: String,
    /// The composing game's own payload for [`TenureAuthority::stage`], at
    /// most [`MAX_TENURE_GAME_BYTES`] encoded. Opaque to this crate.
    pub game: serde_json::Value,
}

/// What a [`TenureAct`] does (doc 20 §9.2–§9.6; cut SM.10).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TenureKind {
    /// File claims over `file` and take over the lapsed `take_over`, one term,
    /// one holder, one discovery stamp. `reopen`: the block's last claim lapsed
    /// (a take-over) — the place's tenure is held once
    /// ([`IndustryFamilies::tenure_hold`]) even when both lists are empty.
    Stake {
        /// The issuer every filed claim is issued by (the deposits' jurisdiction).
        issuer: PrincipalId,
        /// The holder.
        holder: PrincipalId,
        /// The term's end, IC seconds.
        term_at: i64,
        /// The holder's discovery stamp, IC seconds.
        discovered_at: i64,
        /// Whether the block's last claim lapsed (the stake re-opens it).
        reopen: bool,
        /// The deposits to file a new claim over.
        file: Vec<OreDepositId>,
        /// The lapsed claims to take over.
        take_over: Vec<ClaimRev>,
    },
    /// Renew every named claim in place to `term_at`; suspend `suspend` under
    /// `by` (the Registrar's rule inside the player's own renewal, §9.6).
    Renew {
        /// The holder's claims.
        claims: Vec<ClaimRev>,
        /// The new term's end, IC seconds.
        term_at: i64,
        /// Another issuer's paper claims over the block to suspend.
        suspend: Vec<ClaimRev>,
        /// The jurisdiction suspending them; required when `suspend` is not empty.
        by: Option<PrincipalId>,
    },
    /// Lapse the place for `holder` at `at`, cooling to `cooling_until`, and
    /// every named claim. A `cooling_until` no later than `at` is no cooling
    /// term at all: no row is written
    /// ([`TenureRecord::on_lapse`](crate::record::TenureRecord::on_lapse)).
    Lapse {
        /// The lapsing holder.
        holder: PrincipalId,
        /// The holder's claims over the block.
        claims: Vec<ClaimRev>,
        /// The lapse instant, IC seconds.
        at: i64,
        /// The cooling term's end, IC seconds; at or before `at`, none.
        cooling_until: i64,
    },
    /// The contest settled for `to`: the place held once, every named claim
    /// transferred with the challenger's discovery stamp and the new term the
    /// challenger paid for ([`IndustryFamilies::transfer_tenure_termed`]).
    Contest {
        /// The holder's claims over the block.
        claims: Vec<ClaimRev>,
        /// The challenger.
        to: PrincipalId,
        /// The challenger's heard stamp, IC seconds.
        discovered_at: i64,
        /// The new term's end, IC seconds.
        term_at: i64,
    },
    /// The clerk's seeding: one claim per deposit for an org, optionally filed
    /// `Contested { contract }` (a paper).
    Seed {
        /// The issuer.
        issuer: PrincipalId,
        /// The holder (the org).
        holder: PrincipalId,
        /// The term's end, IC seconds.
        term_at: i64,
        /// The deposits to file over.
        deposits: Vec<OreDepositId>,
        /// The paper's adjudication contract, when the seed is a paper.
        contest: Option<ContractId>,
    },
}

impl TenureKind {
    /// The kind's wire word (`stake`, `renew`, `lapse`, `contest`, `seed`).
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Stake { .. } => "stake",
            Self::Renew { .. } => "renew",
            Self::Lapse { .. } => "lapse",
            Self::Contest { .. } => "contest",
            Self::Seed { .. } => "seed",
        }
    }
}

/// The block's tenure as the bundle leaves it, handed to
/// [`TenureAuthority::stage`] so the composing game never reads an industry
/// family: the epoch and `since` before the act, and the epoch after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TenureOutcome {
    /// The block's epoch before the act.
    pub epoch_before: u32,
    /// When that epoch began (IC seconds); `None` for epoch 0.
    pub since_before: Option<i64>,
    /// The block's epoch after the act.
    pub epoch_after: u32,
}

/// **The composed tenure authority** (doc 20 §9, D-20-12; cut SM.10), in
/// [`SystemOpener`]'s shape: the ONE server-authored [`TenureAct`] an actor
/// may make now, and the composing game's own record writes staged in the same
/// bundle. Unwired, `industry_claim_tenure` registers and offers nothing.
pub trait TenureAuthority: Send + Sync {
    /// The ONE act `actor` may make now, or `None` (no offer). Called from
    /// `project_offer` only.
    fn next_tenure(&self, engine: &Engine, actor: ObjId) -> Option<TenureAct>;

    /// Stage the composing game's OWN record writes for `act` in the same
    /// bundle (its own families only; never an industry family). Called last in
    /// the prepare, after every industry write. An `Err` refuses the whole act
    /// and nothing is written.
    ///
    /// # Errors
    /// The refusal line, shown to the actor.
    fn stage(
        &self,
        txn: &mut ActionTxn<'_>,
        act: &TenureAct,
        outcome: &TenureOutcome,
        now: &ClockStamp,
    ) -> Result<(), String>;
}
