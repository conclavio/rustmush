//! The pure posting engine (E2a, extended in E3a) — [`validate`], [`apply`], and
//! [`plan_reversal`].
//!
//! [`validate`] answers *may this plan post, against this immutable snapshot, for
//! this actor?* over the §7.1 invariants (E2a's nine plus E3a's 10–22), and returns a
//! [`Prepared`] that [`apply`] folds **infallibly** (validation already proved it
//! against the pre-state — the M32 `unit::apply` shape). It is pure and world-free;
//! authorization is injected as an [`AuthEval`] and world custody as a [`CustodyEval`]
//! so `mush-economy` never depends on the org or sim modules.
//!
//! The boundedness caps (R-B5) are validate invariants, not an afterthought: an
//! oversized entry would build a journal chunk past `MAX_SAFE_CHUNK_BYTES` and latch
//! the E0 poison-freeze at APPLY time — so "infallible apply" would be false. Every
//! accepted plan provably serializes below [`MAX_ENTRY_BYTES`], effects included.
//!
//! E3a widens a transaction from "money only" to "money **and/or** title and custody
//! in one durable bundle". Three properties keep that safe:
//!
//! - **One authority, live.** Every money line AND every asset/custody op is
//!   authorized for the ONE `plan.actor`, at post time (D-E3-4). Counterparty consent
//!   is E4.
//! - **Journal-carried effects.** An [`AssetEffect`] holds the FULL post-state of the
//!   asset, so replay rebuilds title from the journal alone and a dropped projection
//!   doc can never desync title from the money that bought it (D-E3-1).
//! - **Fail-closed seams.** Every new [`AuthEval`] method defaults to `false` and
//!   [`CustodyEval`] has a `NoCustody` fake that refuses everything, so a caller that
//!   has not yet wired a provider simply cannot move custody.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use mush_org::{ApprovalPolicy, OrgScopeId, PrincipalId};

use crate::amount::Amount;
use crate::clock::Clocks;
use crate::contract::AttestorSet;
use crate::hash::{asset_op_key, custody_op_key, hold_op_key, plan_hash};
use crate::ids::{AccountId, ApprovalRequestId, HoldId, InstrumentId, TransactionId, VaultId};
use crate::plan::{
    ApprovalNeed, AssetOp, CustodyOp, EconomyError, EconomyPlan, HoldOp, PlanResult, Prepared,
};
use crate::record::{
    Account, AccountStatus, AcquisitionRecord, ApprovalRule, AssetAssignment, AssetEffect,
    AssetEffectKind, AssetLifecycle, CustodyDirection, CustodyEffect, EconomicAsset, EconomicCause,
    EconomicTransaction, Encumbrance, ExternalRef, Hold, HoldEffect, HoldEffectKind, HoldStatus,
    IdempotencyRecord, InstrumentStatus, JournalLine, Vault, VaultStatus,
};
use crate::snapshot::EconomySnapshot;

// ---- boundedness caps (R-B5) ----------------------------------------------

/// Maximum journal lines in one transaction. Also caps `expected_revisions` (G5: an
/// unbounded revision vector is a cheap way to make validation expensive).
pub const MAX_LINES_PER_TX: usize = 256;
/// Maximum bytes in a line memo.
pub const MAX_MEMO_BYTES: usize = 512;
/// Maximum bytes in a transaction description.
pub const MAX_DESCRIPTION_BYTES: usize = 4096;
/// Maximum serialized bytes of one journal entry. Far below the ledger's 8 MiB
/// `MAX_SAFE_CHUNK_BYTES` poison threshold, so no accepted plan can build a chunk
/// the store rejects — a fresh chunk holding one accepted entry stays well under.
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;
/// Maximum asset ops in one plan. Also caps `expected_asset_revisions`.
pub const MAX_ASSET_OPS_PER_PLAN: usize = 32;
/// Maximum custody ops in one plan (at most one per object, so also the object cap).
pub const MAX_CUSTODY_OPS_PER_PLAN: usize = 32;
/// Maximum claims one asset may carry. Every later effect re-serializes the full
/// post-state, so this is a real entry-size input, not just tidiness.
pub const MAX_ENCUMBRANCES_PER_ASSET: usize = 16;
/// Maximum bytes of any free-text reason/purpose that ends up inside an effect.
pub const MAX_REASON_BYTES: usize = 512;
/// Maximum approvers counted on one request (G1).
pub const MAX_APPROVERS_PER_REQUEST: usize = 8;
/// How long an approval request stays live (real seconds).
pub const APPROVAL_TTL_SECS: i64 = 86_400;
/// Maximum LIVE (pending, unexpired) approval requests one requester may hold
/// open at once (E3b2 §4.6, G1).
pub const MAX_PENDING_PER_REQUESTER: usize = 4;
/// Maximum LIVE approval requests naming one account (E3b2 §4.6, G1).
pub const MAX_PENDING_PER_ACCOUNT: usize = 16;
/// How long past `expires_at` a dead (`Expired`/`Cancelled`) request doc is kept
/// before boot garbage-collects it — long enough to be an audit trail, bounded
/// enough not to be a landfill.
pub const APPROVAL_GC_AFTER_SECS: i64 = 7 * APPROVAL_TTL_SECS;
/// Maximum hold ops in one plan (§9). Also caps `expected_hold_revisions`.
pub const MAX_HOLD_OPS_PER_PLAN: usize = 32;
/// Maximum `Held` holds one account may carry at once (§9, invariant 26). Every
/// later validation walks the account's holds, so this is a real cost bound.
pub const MAX_ACTIVE_HOLDS_PER_ACCOUNT: usize = 64;
/// Maximum hold expiries one economy sweep may post (§9; E4b1 enforces).
pub const MAX_HOLD_EXPIRIES_PER_SWEEP: usize = 64;
/// Maximum obligations one economy sweep may touch (§9; E4b1 enforces).
pub const MAX_OBLIGATIONS_PER_SWEEP: usize = 64;
/// Maximum posts one economy sweep may make (§9; E4b1 enforces).
pub const MAX_POSTS_PER_SWEEP: usize = 128;
/// How long a terminal (`Released`/`Expired`/`Consumed`) hold doc is kept before
/// GC (§9); its history stays in the journal forever.
pub const HOLD_GC_AFTER_SECS: i64 = 7 * 86_400;
/// The window in which a repeated `Overload` forgiveness escalates (§9, §4.6).
pub const ESCALATION_WINDOW_SECS: i64 = 7 * 86_400;
/// How often an unacknowledged level-2 escalation re-notifies (§9, §4.6).
pub const ESCALATION_RENOTIFY_SECS: i64 = 3_600;

/// The economy capability vocabulary (§4.8) — every capability the economy asks
/// `mush-org` about. Wiring freezes the vocabulary; this is the authoritative list
/// the module and its commands check against, so a typo'd grant is visible.
pub const CAPABILITIES: &[&str] = &[
    "economy.account.post",
    "economy.account.view",
    "economy.account.approve",
    "asset.title.transfer",
    "asset.assign",
    "vault.view",
    "vault.deposit",
    "vault.withdraw",
    // E4b2 (§5): an org contract is drafted, accepted, attested and disputed by
    // whoever holds these on the contract's own compartment. `contract.accept` is
    // deliberately NOT a spending right — an org member needs it *and* debit
    // authority on the account the consideration names (D-E4-4).
    "contract.draft",
    "contract.accept",
    "contract.attest",
    "contract.dispute",
    // E6b (§2 item 7, D-E6-18, §11 q10): an organization holding this on an asset's
    // compartment is "an authorized insurer" and may file `AssetOp::ReportStolen`.
    // It is deliberately NOT `asset.title.transfer` — an insurer files claims; it
    // does not move title — and wiring freezes the vocabulary, so a typo'd grant is
    // visible here.
    "asset.insure",
];

// ---- the authorization seam -----------------------------------------------

/// Resolves *may this actor do this, now?* — re-evaluated LIVE at every post (R-D4),
/// never a captured revision. `mush-economy` defines the seam; E2b/E3b bridge it to
/// `mush_org::authorize_in_world`. A system source/sink line is the module's own to
/// authorize; a player line checks the actor against the account's scope.
///
/// Every method except the E2 money one **defaults to `false`**: a foreign impl
/// written against E2 keeps compiling and simply cannot move title, custody, or
/// maintenance, and cannot make an approver count.
pub trait AuthEval {
    /// `true` if `actor` may apply `delta` to `account`; `false` denies (mapped to
    /// [`EconomyError::Unauthorized`]).
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool;

    /// `true` if `actor` may perform `op` on `asset` (`None` for `Title`, which
    /// creates one). The seam owns the policy: owner / `asset.title.transfer` on the
    /// asset's scope / staff, with `Title` and `Encumber` staff-only in E3
    /// (invariants 19–20) and `Release(Lien)` additionally open to its creditor.
    fn authorize_asset(
        &self,
        _actor: &PrincipalId,
        _asset: Option<&EconomicAsset>,
        _op: &AssetOp,
    ) -> bool {
        false
    }
    /// `true` if `actor` may put title INTO `to`/`to_scope` — the destination half of
    /// D-E3-12 (title carries liabilities, so receiving is not free the way a money
    /// credit is). The seam also resolves the org identity of `to_scope`, which the
    /// pure engine cannot see.
    fn authorize_title_receipt(
        &self,
        _actor: &PrincipalId,
        _to: &PrincipalId,
        _to_scope: Option<OrgScopeId>,
    ) -> bool {
        false
    }
    /// `true` if `actor` may perform `op` against `vault` (`vault.deposit` /
    /// `vault.withdraw` on the vault's scope, its owner, or staff).
    fn authorize_vault(&self, _actor: &PrincipalId, _vault: &Vault, _op: &CustodyOp) -> bool {
        false
    }
    /// `true` if `actor` may point maintenance at `account` — implemented as "may
    /// post from it" (invariant 14; a zero-delta probe through [`AuthEval::authorize`]
    /// is NOT enough, because crediting is free).
    fn authorize_maintenance(&self, _actor: &PrincipalId, _account: &Account) -> bool {
        false
    }
    /// `true` if `approver` currently qualifies under `policy` for `account` — re-run
    /// LIVE at post (A5: approvers frozen at approve time are a fail-open). Holds
    /// `economy.account.approve` on the account's scope; `Threshold.from_role` also
    /// requires that role via `OrgMembership`; `NamedSet` requires set membership.
    fn authorize_approver(
        &self,
        _approver: ObjId,
        _account: &Account,
        _policy: &ApprovalPolicy,
    ) -> bool {
        false
    }
    /// `true` if `actor` may perform `op` on `hold` (E4a, invariant 25).
    ///
    /// The seam owns the policy: a `Release` is the placer's, the beneficiary's
    /// (releasing their own claim), or staff's; an `Expire` is the sweep's; a
    /// `Capture` is the beneficiary's, which for escrow means the contract's own
    /// acceptances ([`crate::ContractAuth`]).
    ///
    /// **Every** hold op asks this, `Capture` included: plain debit authority on the
    /// account is emphatically NOT enough (E4a verify fold, finding 1). A reservation
    /// exists precisely to stand between the account's owner and the money, so
    /// letting the owner's own wallet rights spend it would make escrow a promise
    /// only the payer could break.
    ///
    /// Defaults to `false` like every other E3+ method, so an impl written against
    /// E3 keeps compiling and simply cannot move a reservation.
    fn authorize_hold(&self, _actor: &PrincipalId, _hold: &Hold, _op: &HoldOp) -> bool {
        false
    }
    /// `true` if this authority's posts are **exempt from the rolling
    /// [`crate::ApprovalWindow`]** (E6a step 1 verify fold, M1).
    ///
    /// The window is a *discretionary-spend* control: it asks "has this account
    /// spent more than its officers agreed it could, without anybody deciding?".
    /// A standing obligation's period post is not discretionary — the debit was
    /// consented to when the obligation was created, and the sweep cannot collect
    /// an approval — so a windowed treasury with a full window would have had its
    /// rent *suspended* and a level-2 page opened, with the creditor unpaid. Only
    /// the sweep's own `ObligationAuth` answers `true`; the **threshold** clause is
    /// untouched (an obligation over the per-transaction threshold still escalates,
    /// exactly as it did), and the post still counts toward the account's window for
    /// every later discretionary spend.
    ///
    /// It hangs off the authority rather than off the plan's `cause` deliberately: a
    /// cause is a caller-settable string, and a bypass any caller could stamp on a
    /// plan is not a bypass, it is a hole.
    fn window_exempt(&self) -> bool {
        false
    }
    /// `true` if `actor` currently qualifies as an attestor for `set` — re-run LIVE
    /// at settlement, exactly like [`AuthEval::authorize_approver`] (E3's A5 rule):
    /// a `Capability`/`Staff` attestor who has since lost the capability stops
    /// counting, so an attestation is a record of a claim, not a captured credential.
    fn authorize_attestor(&self, _actor: &PrincipalId, _set: &AttestorSet) -> bool {
        false
    }
    /// `true` if this authority may take `object` out of `vault` **while a live
    /// contract's `DepositInto` still locks it** (E4b2 verify fold, M2+M3).
    ///
    /// The default is `false`, and `crate::api::WorldAuth` keeps it: no command, staff
    /// included, opens a custody lock. The only `true` comes from
    /// `ContractAuth::unwinding`, for a deposit that contract itself recorded — which
    /// is what lets an unwind post its refund *before* the state change, so a refused
    /// refund leaves a live, retryable contract instead of a terminal one holding
    /// somebody's money.
    fn authorize_custody_unlock(&self, _object: ObjId, _vault: VaultId) -> bool {
        false
    }
    /// `true` if this authority may send `object` **home into the backing of a
    /// vault** rather than to a world location (E6b, DL-03).
    ///
    /// Invariant 15's destination rule and A2's laundering guard both say entering
    /// a vault is a `Deposit`, never a withdrawal's side effect. The one exception
    /// is an unwind returning a deposit to a depositor who is not a world object at
    /// all — an organization or a system principal — because there is no world
    /// object to hand it to and the alternative is stranding somebody's property in
    /// the counterparty's vault forever.
    ///
    /// It is **authority-shaped, never cause- or key-shaped** (08c §5.1): the
    /// default is `false`, `crate::api::WorldAuth` keeps it, so no `@vault/withdraw`
    /// can reach it; the only `true` comes from `ContractAuth::unwinding`, for a
    /// deposit that contract recorded, to the backing of an `Active` vault that
    /// deposit's own depositor owns.
    fn authorize_vault_home(&self, _object: ObjId, _to: ObjId) -> bool {
        false
    }
}

/// An [`AuthEval`] that authorizes everything — for tests and trusted internal posts.
///
/// **E3a note:** this now authorizes *title* as well as money — asset ops, title
/// receipt, vault ops, maintenance, and every approver. Never hand one to a path that
/// takes player input.
pub struct AllowAll;
impl AuthEval for AllowAll {
    fn authorize(&self, _: &PrincipalId, _: &Account, _: Amount) -> bool {
        true
    }
    fn authorize_asset(&self, _: &PrincipalId, _: Option<&EconomicAsset>, _: &AssetOp) -> bool {
        true
    }
    fn authorize_title_receipt(
        &self,
        _: &PrincipalId,
        _: &PrincipalId,
        _: Option<OrgScopeId>,
    ) -> bool {
        true
    }
    fn authorize_vault(&self, _: &PrincipalId, _: &Vault, _: &CustodyOp) -> bool {
        true
    }
    fn authorize_maintenance(&self, _: &PrincipalId, _: &Account) -> bool {
        true
    }
    fn authorize_approver(&self, _: ObjId, _: &Account, _: &ApprovalPolicy) -> bool {
        true
    }
    fn authorize_hold(&self, _: &PrincipalId, _: &Hold, _: &HoldOp) -> bool {
        true
    }
    fn authorize_attestor(&self, _: &PrincipalId, _: &AttestorSet) -> bool {
        true
    }
}

// ---- the custody seam -----------------------------------------------------

/// Resolves the WORLD facts a custody op needs — is the backing a usable container,
/// does the vault hold this object, does the actor have standing over it, is the
/// destination legal. `mush-economy` is world-free, so every one of these is asked of
/// the embedder (E3b1's container provider, E3c's `mush-sim` interiors).
///
/// Standing is deliberately **not** "owns it": carrying it (`Obj.location == actor`
/// exactly, never an ancestor walk) or being co-located with it as title owner /
/// assignee / MUSH controller / staff (invariant 15).
pub trait CustodyEval {
    /// Whether the vault's backing is a live, usable custody container.
    fn backing_valid(&self, vault: &Vault) -> bool;
    /// Whether the vault (transitively, depth-capped) holds `obj`.
    fn holds(&self, vault: &Vault, obj: ObjId) -> bool;
    /// Whether `actor` has custody standing over `obj` to deposit it.
    fn may_deposit(&self, obj: ObjId, actor: &PrincipalId) -> bool;
    /// Whether `to` is a legal withdrawal destination for `actor` (the actor or the
    /// actor's current location; live, not `GOING`).
    fn destination_ok(&self, actor: &PrincipalId, to: ObjId) -> bool;
}

/// A [`CustodyEval`] that refuses everything — the fail-closed default for callers
/// with no custody provider wired (E3a's own `post` seam, and the reversal path,
/// which never touches custody).
pub struct NoCustody;
impl CustodyEval for NoCustody {
    fn backing_valid(&self, _: &Vault) -> bool {
        false
    }
    fn holds(&self, _: &Vault, _: ObjId) -> bool {
        false
    }
    fn may_deposit(&self, _: ObjId, _: &PrincipalId) -> bool {
        false
    }
    fn destination_ok(&self, _: &PrincipalId, _: ObjId) -> bool {
        false
    }
}

// ---- the external-fact seam (E4a) -----------------------------------------

/// Resolves a contract's [`crate::TypedCondition::Fact`] against the provider that
/// owns it (E4c wires `mush-jobs`; nothing implements it in E4a).
///
/// Three answers, and only one of them settles money: `Some(true)` satisfied,
/// `Some(false)` known-unsatisfied, `None` **unknown**. Unknown is unsatisfied — a
/// contract can never settle on a fact nobody can answer, which is why the default
/// impl answers `None` and [`NoFacts`] exists at all (D-E4-4).
pub trait FactEval {
    /// Resolve within the exact contract the pure engine is evaluating. Legacy
    /// pure evaluators keep their existing behavior; live restricted providers
    /// must authenticate this scope against retained world truth.
    fn resolve_in_contract(
        &self,
        _contract: &crate::EconomicContract,
        fact: &ExternalRef,
    ) -> Option<bool> {
        self.resolve(fact)
    }

    /// Resolve one fact.
    fn resolve(&self, _fact: &ExternalRef) -> Option<bool> {
        None
    }
}

/// A [`FactEval`] that knows nothing — the fail-closed default until E4c wires a
/// provider.
pub struct NoFacts;
impl FactEval for NoFacts {}

/// Everything [`validate`] needs beyond the plan and the snapshot: the three live
/// seams and the two clocks (approval expiry, hold expiry, and every schedule are
/// time questions the pure engine cannot answer on its own).
pub struct ValidateEnv<'a> {
    /// Live authorization.
    pub auth: &'a dyn AuthEval,
    /// Live world custody.
    pub custody: &'a dyn CustodyEval,
    /// Live external facts (E4a; [`NoFacts`] until E4c).
    pub facts: &'a dyn FactEval,
    /// Wall-clock unix seconds.
    pub now: i64,
    /// The world's IC clock, unix seconds (`World::game_now_unix`) — the second
    /// instant [`crate::EconomicClock`] resolves against (E4a).
    pub ic_now: i64,
    /// The next [`HoldId`] the module's allocator will mint. `validate` assigns hold
    /// ids deterministically from it in canonical op order, so `apply` stays
    /// infallible and the module advances its allocator by the number of
    /// [`crate::HoldEffect`]s with kind `Placed`. It is **not** hashed: two
    /// validations of one plan at different allocator states are the same plan.
    pub next_hold_id: u64,
    /// What each account this plan debits (or places a hold on) has **already
    /// spent inside its [`crate::ApprovalWindow`]** — the trailing-period sum of its
    /// negative lines plus placed holds, a refund never subtracted (E6a step 1,
    /// D-E6-21). The pure snapshot carries no journal, so the module computes it
    /// from its RAM-only per-account tail and hands it in; an account absent here is
    /// read as having spent nothing, which is exactly right for one with no window.
    /// Not hashed, like the allocator floor: it is a fact about the world at post
    /// time, not about the plan.
    pub spent: BTreeMap<AccountId, Amount>,
    /// Doc 13 (W-D1, W-D9): how the plan's idempotency key is decided — the
    /// store's gate (the RAM window, the cold cache, the key filter) on every
    /// live path; [`MapOnlyKeys`] where a rig validates against a bare snapshot.
    pub keys: &'a dyn KeyGate,
}

impl ValidateEnv<'_> {
    /// The clock resolver for this environment.
    #[must_use]
    pub fn clocks(&self) -> Clocks {
        Clocks::new(self.now, self.ic_now)
    }
}

/// What the gate says about a plan's idempotency key (doc 13 §2.4).
#[derive(Clone, Copy, Debug)]
pub enum KeyVerdict<'a> {
    /// The key was posted before: its record.
    Known(&'a crate::record::IdempotencyRecord),
    /// The key was never posted: certain.
    Fresh,
    /// The key may have been posted before the RAM window and the store has to
    /// be asked — [`crate::EconomyError::RetryShortly`].
    Unknown,
}

/// The idempotency gate the planner asks (doc 13 §2.4, W-D1).
pub trait KeyGate {
    /// The verdict for `key` against `snapshot`'s window.
    fn verdict<'s>(
        &'s self,
        snapshot: &'s EconomySnapshot,
        key: &crate::record::IdempotencyKey,
    ) -> KeyVerdict<'s>;
}

/// The gate before the window exists: the snapshot's map is every key ever
/// posted, so a miss is `Fresh`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MapOnlyKeys;

impl KeyGate for MapOnlyKeys {
    fn verdict<'s>(
        &'s self,
        snapshot: &'s EconomySnapshot,
        key: &crate::record::IdempotencyKey,
    ) -> KeyVerdict<'s> {
        match snapshot.idempotency(key) {
            Some(record) => KeyVerdict::Known(record),
            None => KeyVerdict::Fresh,
        }
    }
}

// ---- validate -------------------------------------------------------------

/// The non-`Post` outcomes of the inner validation, so `NeedsApproval` (a plan that
/// is well-formed and authorized but unapproved) never collapses into a refusal.
enum Fail {
    Refused(EconomyError),
    NeedsApproval(ApprovalNeed),
}
impl From<EconomyError> for Fail {
    fn from(e: EconomyError) -> Fail {
        Fail::Refused(e)
    }
}

/// Validate `plan` against `snapshot` for its actor. Pure + total; mutates nothing.
#[must_use]
pub fn validate(
    plan: &EconomyPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(plan, snapshot, env, PlanGuard::Legacy)
}

/// Only the private closed purchase adapter can construct this proof. Public
/// post/approval paths always select Legacy, including exact-key retries.
pub(crate) fn validate_purchase(
    proof: &crate::managed_purchase::ClosedPurchasePlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(proof.plan(), snapshot, env, PlanGuard::Purchase(proof))
}

/// Separate private renewal proof. Existing purchase and Legacy routes do not
/// accept a renewal namespace or infer authority from a managed contract.
pub(crate) fn validate_renewal(
    proof: &crate::managed_renewal::ClosedRenewalPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(proof.plan(), snapshot, env, PlanGuard::Renewal(proof))
}
pub(crate) fn validate_house_renewal(
    proof: &crate::managed_house_renewal::ClosedHouseRenewalPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(proof.plan(), snapshot, env, PlanGuard::HouseRenewal(proof))
}
/// Separate closed fee proof; public legacy posting cannot occupy its history.
pub(crate) fn validate_compatible_update(
    proof: &crate::registry_filing::update::ClosedUpdatePlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(
        proof.plan(),
        snapshot,
        env,
        PlanGuard::CompatibleUpdate(proof),
    )
}
/// Only the private native adapter may authorize a managed charter mutation.
pub(crate) fn validate_charter_transfer(
    proof: &crate::registry_filing::charter_transfer::ClosedCharterTransferPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(
        proof.plan(),
        snapshot,
        env,
        PlanGuard::CharterTransfer(proof),
    )
}
pub(crate) fn validate_charter_pending(
    proof: &crate::managed_charter_transfer::ClosedCharterPendingPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(
        proof.plan(),
        snapshot,
        env,
        PlanGuard::CharterPending(proof),
    )
}
pub(crate) fn validate_paid_resale(
    proof: &crate::managed_paid_resale::ClosedPaidResalePlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(proof.plan(), snapshot, env, PlanGuard::PaidResale(proof))
}
pub(crate) fn validate_core_integration(
    proof: &crate::core_integration::ClosedCorePlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(
        proof.plan(),
        snapshot,
        env,
        PlanGuard::CoreIntegration(proof),
    )
}
pub(crate) fn validate_core_refit(
    proof: &crate::core_refit::ClosedRefitPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> PlanResult {
    validate_guarded(proof.plan(), snapshot, env, PlanGuard::CoreRefit(proof))
}
#[derive(Clone, Copy)]
enum PlanGuard<'a> {
    PaidResale(&'a crate::managed_paid_resale::ClosedPaidResalePlan),
    Legacy,
    CharterTransfer(&'a crate::registry_filing::charter_transfer::ClosedCharterTransferPlan),
    CharterPending(&'a crate::managed_charter_transfer::ClosedCharterPendingPlan),
    CoreIntegration(&'a crate::core_integration::ClosedCorePlan),
    CoreRefit(&'a crate::core_refit::ClosedRefitPlan),
    Purchase(&'a crate::managed_purchase::ClosedPurchasePlan),
    Renewal(&'a crate::managed_renewal::ClosedRenewalPlan),
    HouseRenewal(&'a crate::managed_house_renewal::ClosedHouseRenewalPlan),
    CompatibleUpdate(&'a crate::registry_filing::update::ClosedUpdatePlan),
}
impl PlanGuard<'_> {
    fn managed_mutations(
        self,
        plan: &EconomyPlan,
        snapshot: &EconomySnapshot,
    ) -> Result<(), EconomyError> {
        match self {
            Self::CharterTransfer(proof) => proof.check(plan, snapshot),
            Self::CharterPending(proof) => proof.check(plan, snapshot),
            _ => crate::managed_title::require_unmanaged_mutations(plan, snapshot),
        }
    }
    fn check(self, plan: &EconomyPlan, snapshot: &EconomySnapshot) -> Result<(), EconomyError> {
        match self {
            Self::Legacy => crate::contract::managed::require_legacy_plan(plan, snapshot),
            Self::PaidResale(proof) => proof.check(plan, snapshot),
            Self::Purchase(proof) => proof.check(plan, snapshot),
            Self::CoreIntegration(proof) => proof.check(plan, snapshot),
            Self::CoreRefit(proof) => proof.check(plan, snapshot),
            Self::Renewal(proof) => proof.check(plan, snapshot),
            Self::HouseRenewal(proof) => proof.check(plan, snapshot),
            Self::CompatibleUpdate(proof) => proof.check(plan, snapshot),
            Self::CharterTransfer(proof) => proof.check(plan, snapshot),
            Self::CharterPending(proof) => proof.check(plan, snapshot),
        }
    }
}
fn validate_guarded(
    plan: &EconomyPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
    guard: PlanGuard<'_>,
) -> PlanResult {
    if let Err(error) = guard.check(plan, snapshot) {
        return PlanResult::Refused(error);
    }
    if let Err(error) = guard.managed_mutations(plan, snapshot) {
        return PlanResult::Refused(error);
    }
    let hash = plan_hash(plan);

    // Idempotency FIRST: an exact repeat short-circuits to the original id; a reuse
    // with a different plan is an error (never a silent re-post); a key the gate
    // cannot decide from RAM is a transient (doc 13, W-D1).
    match env.keys.verdict(snapshot, &plan.idempotency) {
        KeyVerdict::Known(rec) => {
            if rec.plan_hash == hash {
                return PlanResult::AlreadyPosted(rec.transaction);
            }
            return PlanResult::Refused(EconomyError::IdempotencyConflict);
        }
        KeyVerdict::Unknown => return PlanResult::Refused(EconomyError::RetryShortly),
        KeyVerdict::Fresh => {}
    }

    match validate_inner(plan, snapshot, env, hash, false, guard) {
        Ok(prepared) => PlanResult::Post(Box::new(prepared)),
        Err(Fail::Refused(e)) => PlanResult::Refused(e),
        Err(Fail::NeedsApproval(need)) => PlanResult::NeedsApproval(Box::new(need)),
    }
}

fn validate_inner(
    plan: &EconomyPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
    hash: [u8; 32],
    skip_approval: bool,
    guard: PlanGuard<'_>,
) -> Result<Prepared, Fail> {
    // Boundedness caps (R-B5) — cheap, first, so a huge plan is refused before work.
    caps(plan)?;
    guard.check(plan, snapshot)?;
    guard.managed_mutations(plan, snapshot)?;

    // Expected-revision staleness on ECONOMIC data (authorization is live, below).
    for e in &plan.expected_revisions {
        let cur = snapshot.account(e.account).map(|a| a.revision);
        if cur != Some(e.revision) {
            return Err(EconomyError::Stale {
                account: e.account,
                expected: e.revision,
                found: cur,
            }
            .into());
        }
    }
    // Invariant 16: the same guard on TITLE data.
    for e in &plan.expected_asset_revisions {
        let cur = snapshot.asset(e.object).map(|a| a.revision);
        if cur != Some(e.revision) {
            return Err(EconomyError::StaleAsset {
                object: e.object,
                expected: e.revision,
                found: cur,
            }
            .into());
        }
    }
    // Invariant 26: and on RESERVATION data — the exit criterion's stale revisions.
    for e in &plan.expected_hold_revisions {
        let cur = snapshot.hold(e.hold).map(|h| h.revision);
        if cur != Some(e.revision) {
            return Err(EconomyError::StaleHold {
                hold: e.hold,
                expected: e.revision,
                found: cur,
            }
            .into());
        }
    }

    // Resolve every line: account active, instrument active + matching, delta non-zero.
    // Build the canonical journal lines and the net per-account + per-instrument sums.
    let mut lines: Vec<JournalLine> = Vec::with_capacity(plan.requested_lines.len());
    let mut per_account: BTreeMap<AccountId, i128> = BTreeMap::new();
    // The GROSS debit per account — the sum of its NEGATIVE lines only. Authorization
    // is asked about this as well as the net, because a net of zero (or of a credit)
    // hides real debits behind lines that pay them back (below).
    let mut gross_debit: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut per_instrument: BTreeMap<InstrumentId, i128> = BTreeMap::new();
    let mut non_zero = 0usize;

    for (i, rl) in plan.requested_lines.iter().enumerate() {
        if rl.delta.is_zero() {
            return Err(EconomyError::ZeroLine(i).into());
        }
        non_zero += 1;
        let account = snapshot
            .account(rl.account)
            .ok_or(EconomyError::NoSuchAccount(rl.account))?;
        if account.status != AccountStatus::Active {
            return Err(EconomyError::InactiveAccount(rl.account).into());
        }
        let instrument = snapshot
            .instrument(&account.instrument)
            .ok_or_else(|| EconomyError::NoSuchInstrument(account.instrument.clone()))?;
        if instrument.status != InstrumentStatus::Active {
            return Err(EconomyError::InactiveInstrument(account.instrument.clone()).into());
        }
        *per_account.entry(rl.account).or_insert(0) += rl.delta.widen();
        if rl.delta.is_negative() {
            *gross_debit.entry(rl.account).or_insert(0) += rl.delta.widen();
        }
        *per_instrument
            .entry(account.instrument.clone())
            .or_insert(0) += rl.delta.widen();
        lines.push(JournalLine {
            account: rl.account,
            instrument: account.instrument.clone(),
            delta: rl.delta,
            memo: rl.memo.clone(),
        });
    }

    // Invariant 10 (extends invariant 1): a transaction needs ≥2 non-zero money
    // lines, OR ≥1 asset/custody effect. When money lines exist at all, invariants
    // 1–4 hold unchanged — one lonely line is still refused.
    let has_ops =
        !plan.asset_ops.is_empty() || !plan.custody_ops.is_empty() || !plan.hold_ops.is_empty();
    if non_zero < 2 && !(non_zero == 0 && has_ops) {
        return Err(EconomyError::TooFewLines.into());
    }
    // Invariant 3: each instrument balances to exactly zero.
    for (instrument, sum) in &per_instrument {
        if *sum != 0 {
            return Err(EconomyError::NotBalanced(instrument.clone()).into());
        }
    }

    // Invariants 23-26: the reservations. Resolved BEFORE the credit checks, because
    // what a debit is checked against is now `posted - held`, and this plan's own
    // placements count while its own captures release what they consume (D-E4-9).
    let holds = resolve_hold_ops(plan, snapshot, env)?;

    // Invariant 23's other half (E4a verify fold, finding 1): a `Capture` SPENDS
    // reserved value, so this plan must actually debit the hold's own account by at
    // least what it captures there. Without this an effect-only capture would
    // silently destroy a reservation (freeing the payer's funds and leaving the
    // beneficiary with nothing), and a capture on X paired with a debit on Y would
    // let a `ContractAuth`-shaped authority spend one account's escrow to move a
    // different account's money.
    for (&account_id, &captured) in &holds.captured {
        let gross = gross_debit.get(&account_id).copied().unwrap_or(0);
        let debited = gross.checked_neg().ok_or(EconomyError::Overflow)?;
        if debited < captured {
            let hold = holds
                .capture_hold
                .get(&account_id)
                .copied()
                .expect("a captured account always records its first hold");
            return Err(EconomyError::CaptureUnbacked(hold).into());
        }
    }

    // Invariant 4 + credit policy: fold each net delta into the account's balance
    // with checked arithmetic and refuse a credit-policy breach.
    let mut balance_deltas: BTreeMap<AccountId, Amount> = BTreeMap::new();
    for (&account_id, &net128) in &per_account {
        let net = Amount::from_widened(net128).ok_or(EconomyError::Overflow)?;
        let account = snapshot.account(account_id).expect("resolved above");
        let new_balance = account
            .posted_balance
            .checked_add(net)
            .ok_or(EconomyError::Overflow)?;
        if !account.credit.permits(new_balance) {
            return Err(EconomyError::CreditViolation(account_id).into());
        }
        balance_deltas.insert(account_id, net);
    }

    // Invariant 23: **available** balance. Every account this plan debits, and every
    // account it places a hold on, must still satisfy its credit policy at
    // `posted' - held'`. The posted check above stays exactly as E3 wrote it, so a
    // plain overdraft is still `CreditViolation` - `InsufficientAvailable` means the
    // money IS there and is spoken for, which is a different thing to tell a payer.
    //
    // Every touched account's `held'` is narrowed to `i64` here even when its policy
    // is not checked, so `apply`'s fold is provably infallible (a release only ever
    // lowers `held`, but the arithmetic is proved rather than assumed).
    let mut new_held: BTreeMap<AccountId, Amount> = BTreeMap::new();
    for (&account_id, &delta) in &holds.held_deltas {
        let account = snapshot
            .account(account_id)
            .ok_or(EconomyError::NoSuchAccount(account_id))?;
        let widened = account
            .held_amount
            .widen()
            .checked_add(delta)
            .ok_or(EconomyError::Overflow)?;
        new_held.insert(
            account_id,
            Amount::from_widened(widened).ok_or(EconomyError::Overflow)?,
        );
    }
    let mut available_checked: BTreeSet<AccountId> = gross_debit.keys().copied().collect();
    available_checked.extend(holds.place_accounts.iter().copied());
    for account_id in available_checked {
        let account = snapshot
            .account(account_id)
            .ok_or(EconomyError::NoSuchAccount(account_id))?;
        let net = balance_deltas
            .get(&account_id)
            .copied()
            .unwrap_or(Amount::ZERO);
        let posted = account
            .posted_balance
            .checked_add(net)
            .ok_or(EconomyError::Overflow)?;
        let held = new_held
            .get(&account_id)
            .copied()
            .unwrap_or(account.held_amount);
        let available = posted.checked_sub(held).ok_or(EconomyError::Overflow)?;
        if !account.credit.permits(available) {
            return Err(EconomyError::InsufficientAvailable(account_id).into());
        }
        // Invariant 26: a hold cannot promise money that is not there, so a
        // PLACEMENT additionally needs non-negative available funds - a credit line
        // is a licence to overdraw, not a licence to reserve headroom.
        if holds.place_accounts.contains(&account_id) && available.is_negative() {
            return Err(EconomyError::InsufficientAvailable(account_id).into());
        }
    }

    // Authorization — re-evaluated LIVE per touched account (R-D4), on the account's
    // GROSS debit as well as its net.
    //
    // Asking only about the net is a bypass: receiving value is free, and a zero
    // delta is not negative, so a line PAIR like `[-1, +1]` on one account nets to
    // zero, reads as a credit, and needs no authority at all — any actor could post
    // journal entries against any account they could name, bumping its revision
    // without bound (and, on a scoped wallet, doing it entirely outside the org's
    // capability policy). The net still drives balances, the credit policy, and the
    // approval rules; the DEBIT question is asked about every account some line
    // actually debits.
    //
    // E4a verify fold, finding 1: what this plan CAPTURES on an account is already
    // authorized debit authority. The placement proved debit authority at the time
    // (invariant 25), and `authorize_hold` has just proved this actor may spend that
    // exact reservation - so only the UNCOVERED part of the debit needs live
    // authority. That is what a hold is for: a settlement whose whole debit is
    // covered by escrow must post without the payer being asked again, or a party
    // could break an agreement they already funded simply by losing their wallet
    // rights. A plan that captures nothing has `covered == 0` and this loop is
    // byte-for-byte E3's.
    for (&account_id, &net) in &balance_deltas {
        let account = snapshot.account(account_id).expect("resolved above");
        let covered = holds.captured.get(&account_id).copied().unwrap_or(0);
        let net_probe = net
            .widen()
            .checked_add(covered)
            .ok_or(EconomyError::Overflow)?;
        // A fully covered net (`net_probe == 0`) asks nothing at all: probing it as
        // a zero delta would ask the CREDIT question ("is this account a payee?") of
        // an account that is only paying.
        if covered == 0 || net_probe != 0 {
            let probe = Amount::from_widened(net_probe).ok_or(EconomyError::Overflow)?;
            if !env.auth.authorize(&plan.actor, account, probe) {
                return Err(EconomyError::Unauthorized.into());
            }
        }
        let Some(&debit128) = gross_debit.get(&account_id) else {
            continue;
        };
        let uncovered = debit128
            .checked_add(covered)
            .ok_or(EconomyError::Overflow)?;
        if uncovered >= 0 {
            continue;
        }
        let debit = Amount::from_widened(uncovered).ok_or(EconomyError::Overflow)?;
        if uncovered != net_probe && !env.auth.authorize(&plan.actor, account, debit) {
            return Err(EconomyError::Unauthorized.into());
        }
    }

    // Invariants 11–14 + 18–20: title. Invariant 15: custody.
    let asset_effects = resolve_asset_ops(plan, snapshot, env)?;
    let custody_effects = resolve_custody_ops(plan, snapshot, env)?;

    // Invariant 17: approval. NOT evaluated on the reversal path — a staff reversal
    // is not a new spend, and an ApprovalRule must never be a shield against
    // clawback (A7).
    //
    // D-E4-11: a placement is a debit-authority plan, so it counts toward the
    // account's rule exactly like a spend - otherwise reserving a fortune and
    // capturing it later would route around the escalation entirely.
    let consumed_approval = if skip_approval {
        None
    } else {
        let mut approval_deltas = balance_deltas.clone();
        for (&account_id, &placed) in &holds.placed {
            let entry = approval_deltas.entry(account_id).or_insert(Amount::ZERO);
            let amount = Amount::from_widened(placed).ok_or(EconomyError::Overflow)?;
            *entry = entry.checked_sub(amount).ok_or(EconomyError::Overflow)?;
        }
        // ...and, symmetrically, what this plan CAPTURES was already escalated when
        // it was placed (E4b2). A contract's escrow goes through the account's rule
        // at `@contract/accept`, exactly as D-E4-11 requires; asking again at
        // settlement would demand a second approval for the same money — and the
        // sweep, which settles a `Before`/`After` contract, has nobody to collect it
        // from, so an approval-ruled payer could fund a contract that could then
        // never settle. Same reasoning as the authorization pass above, which already
        // excuses the captured part of the debit.
        for (&account_id, &captured) in &holds.captured {
            let entry = approval_deltas.entry(account_id).or_insert(Amount::ZERO);
            let amount = Amount::from_widened(captured).ok_or(EconomyError::Overflow)?;
            *entry = entry.checked_add(amount).ok_or(EconomyError::Overflow)?;
        }
        evaluate_approvals(snapshot, env, hash, &approval_deltas)?
    };

    // Canonical line order (matches the hash's total key) so the persisted entry is
    // deterministic regardless of request order.
    lines.sort_by(|a, b| {
        (a.account.0, a.delta.minor(), a.memo.as_str()).cmp(&(
            b.account.0,
            b.delta.minor(),
            b.memo.as_str(),
        ))
    });

    let prepared = Prepared {
        idempotency: plan.idempotency.clone(),
        plan_hash: hash,
        actor: plan.actor.clone(),
        cause: plan.cause.clone(),
        description: plan.description.clone(),
        visibility: plan.visibility.clone(),
        lines,
        // A normal post never reverses; only `reverse` sets this, from a real tx.
        reverses: None,
        balance_deltas,
        // Invariant 22: provenance is stamped only when the same transaction moved
        // money (a free re-titling is not an acquisition).
        stamp_acquisition: non_zero > 0,
        asset_effects,
        custody_effects,
        hold_effects: holds.effects,
        consumed_approval,
    };

    // Boundedness: the serialized entry must fit MAX_ENTRY_BYTES so apply can never
    // build a poison-sized chunk (R-B5). Measure the real entry shape — effects
    // included (invariant 21); they are by far the largest part of an E3 entry.
    let probe = entry_probe(&prepared);
    let size = serde_json::to_vec(&probe)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if size > MAX_ENTRY_BYTES {
        return Err(EconomyError::TooLarge("serialized entry exceeds the per-entry cap").into());
    }

    Ok(prepared)
}

/// The cheap first block: every count/length cap, before any lookup work.
fn caps(plan: &EconomyPlan) -> Result<(), EconomyError> {
    if plan.requested_lines.len() > MAX_LINES_PER_TX {
        return Err(EconomyError::TooLarge("too many lines"));
    }
    if plan.expected_revisions.len() > MAX_LINES_PER_TX {
        return Err(EconomyError::TooLarge("too many expected revisions"));
    }
    if plan.description.len() > MAX_DESCRIPTION_BYTES {
        return Err(EconomyError::TooLarge("description too long"));
    }
    for l in &plan.requested_lines {
        if l.memo.len() > MAX_MEMO_BYTES {
            return Err(EconomyError::TooLarge("memo too long"));
        }
    }
    if plan.asset_ops.len() > MAX_ASSET_OPS_PER_PLAN {
        return Err(EconomyError::TooLarge("too many asset ops"));
    }
    if plan.custody_ops.len() > MAX_CUSTODY_OPS_PER_PLAN {
        return Err(EconomyError::TooLarge("too many custody ops"));
    }
    if plan.expected_asset_revisions.len() > MAX_ASSET_OPS_PER_PLAN {
        return Err(EconomyError::TooLarge("too many expected asset revisions"));
    }
    if plan.hold_ops.len() > MAX_HOLD_OPS_PER_PLAN {
        return Err(EconomyError::TooLarge("too many hold ops"));
    }
    if plan.expected_hold_revisions.len() > MAX_HOLD_OPS_PER_PLAN {
        return Err(EconomyError::TooLarge("too many expected hold revisions"));
    }
    for op in &plan.hold_ops {
        if let HoldOp::Place {
            purpose: crate::record::HoldPurpose::Manual { reason },
            ..
        } = op
            && reason.len() > MAX_REASON_BYTES
        {
            return Err(EconomyError::TooLarge("hold reason too long"));
        }
    }
    // Invariant 21: every free-text field that lands inside an effect is capped, and
    // the full post-state re-serializes them on each later effect.
    for op in &plan.asset_ops {
        match op {
            AssetOp::WriteOff { reason, .. } if reason.len() > MAX_REASON_BYTES => {
                return Err(EconomyError::TooLarge("write-off reason too long"));
            }
            AssetOp::Assign { purpose, .. } if purpose.len() > MAX_REASON_BYTES => {
                return Err(EconomyError::TooLarge("assignment purpose too long"));
            }
            AssetOp::Encumber { encumbrance, .. } | AssetOp::Release { encumbrance, .. } => {
                if let Encumbrance::Disputed { reason, .. } = encumbrance
                    && reason.len() > MAX_REASON_BYTES
                {
                    return Err(EconomyError::TooLarge("dispute reason too long"));
                }
            }
            // E6b (B2): the report's reason lands in an effect twice over — in the
            // `Disputed` claim and in the `StolenReport` — so invariant 21 caps it
            // exactly like every other free-text field.
            AssetOp::ReportStolen { reason, .. } if reason.len() > MAX_REASON_BYTES => {
                return Err(EconomyError::TooLarge("theft report reason too long"));
            }
            // E6b independent verify fold (M7): the catalogue reference is a stored,
            // rendered pair, bounded in the pure engine and not only at the command
            // that parses it — `AssetOp` is a `pub` alphabet and the next producer
            // will not go through `parse_catalog`.
            AssetOp::SetCatalog {
                catalog: Some(c), ..
            } if c.ruleset.is_empty()
                || c.code.is_empty()
                || c.ruleset.len() > crate::commands::MAX_CATALOG_PART_BYTES
                || c.code.len() > crate::commands::MAX_CATALOG_PART_BYTES =>
            {
                return Err(EconomyError::TooLarge(
                    "a catalogue reference half is empty or too long",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

// ---- invariants 11–14, 18–20: the title ops -------------------------------

/// Invariant 18's structural half: a scoped account/asset/vault must be owned by an
/// organization. WHICH organization (`scope.org == owner`) needs the org state, so it
/// is resolved live by the seam ([`AuthEval::authorize_title_receipt`] /
/// `authorize_asset`), which is the only party that can see it.
fn scope_owner_coherent(scope: Option<OrgScopeId>, owner: &PrincipalId) -> bool {
    scope.is_none() || matches!(owner, PrincipalId::Organization(_))
}

fn resolve_asset_ops(
    plan: &EconomyPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> Result<Vec<AssetEffect>, EconomyError> {
    if plan.asset_ops.is_empty() {
        return Ok(Vec::new());
    }
    // Canonical order — the SAME total order the plan hash uses, so the applied order
    // is the hashed order and two plans differing only in op order are one plan.
    let mut keyed: Vec<(_, &AssetOp)> = plan
        .asset_ops
        .iter()
        .map(|op| (asset_op_key(op), op))
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    // An exactly-duplicated op is a conflicting plan, not a no-op (adjacent after the
    // sort, since the key includes the op's canonical bytes).
    for w in keyed.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(EconomyError::DuplicateAssetOp(w[0].1.object()));
        }
    }

    // Invariant 11: per object, at most one title-changing op, at most one
    // assignment-changing op, at most one maintenance op. (Release/Encumber may
    // repeat — invariant 12 needs to clear several claims in one plan.)
    let mut title_changing: BTreeMap<ObjId, usize> = BTreeMap::new();
    let mut assignment: BTreeMap<ObjId, usize> = BTreeMap::new();
    let mut maintenance: BTreeMap<ObjId, usize> = BTreeMap::new();
    for (_, op) in &keyed {
        let obj = op.object();
        if op.is_title_changing() {
            *title_changing.entry(obj).or_insert(0) += 1;
        }
        if matches!(op, AssetOp::Assign { .. } | AssetOp::Unassign { .. }) {
            *assignment.entry(obj).or_insert(0) += 1;
        }
        if matches!(op, AssetOp::SetMaintenance { .. }) {
            *maintenance.entry(obj).or_insert(0) += 1;
        }
    }
    for counts in [&title_changing, &assignment, &maintenance] {
        if let Some((obj, _)) = counts.iter().find(|(_, n)| **n > 1) {
            return Err(EconomyError::DuplicateAssetOp(*obj));
        }
    }

    let actor = &plan.actor;
    let mut work: BTreeMap<ObjId, EconomicAsset> = BTreeMap::new();
    let mut effects: Vec<AssetEffect> = Vec::with_capacity(keyed.len());

    for (_, op) in keyed {
        let object = op.object();
        let effect_kind = match op {
            AssetOp::Title {
                owner,
                scope,
                catalog,
                ..
            } => {
                if work.contains_key(&object) {
                    return Err(EconomyError::AssetExists(object));
                }
                // E4c verify fold, E7: a **written-off** title may be re-issued.
                // Write-off is how a title ends, and an object whose title ended is
                // an object with no live title — the state `Title` is for. Refusing
                // it made a re-issue an idempotency hit that reported success and
                // minted nothing, so a ruleset handing out the same flat twice
                // handed it out once. Anything with a LIVE title is refused exactly
                // as before, and so is one still carrying claims: a lien outlives the
                // write-off it was recorded against, and a fresh title over it would
                // silently drop somebody's security.
                //
                // E6c B4(b) — **E3-C36** (08g §3 #2, filed "before E6c — titles every
                // purchase"). The arm above admits a re-issue over a `WrittenOff`
                // asset with no encumbrances and **never consulted `a.frozen`**: the
                // frozen check sits in the non-`Title` arm below, so a title could be
                // minted straight over a frozen record. That is reachable without any
                // staff act — boot's principal-gone sweep freezes an asset whose owner
                // has vanished, and a written-off one is exactly the shape it can
                // catch — and it would hand a fresh, unfrozen title to whoever asked
                // for the object next, discarding the reason the freeze was placed.
                // A frozen record is refused here; staff `@asset/unfreeze` first. The
                // mitigation that carries `prev.frozen` onto the new generation stays
                // where it is, for every path this one does not cover.
                let previous = match snapshot.asset(object) {
                    None => None,
                    Some(a) if a.frozen => return Err(EconomyError::AssetFrozen(object)),
                    Some(a)
                        if a.lifecycle == AssetLifecycle::WrittenOff
                            && a.encumbrances.is_empty() =>
                    {
                        Some(a.revision)
                    }
                    Some(_) => return Err(EconomyError::AssetExists(object)),
                };
                if !scope_owner_coherent(*scope, owner) {
                    return Err(EconomyError::ScopeOwnerMismatch);
                }
                // Invariant 20: creating a title is staff-only in E3 (the seam says so).
                if !env.auth.authorize_asset(actor, None, op) {
                    return Err(EconomyError::Unauthorized);
                }
                work.insert(
                    object,
                    EconomicAsset {
                        object,
                        title_owner: owner.clone(),
                        scope: *scope,
                        assignment: None,
                        maintenance_account: None,
                        catalog: catalog.clone(),
                        encumbrances: Vec::new(),
                        acquisition: None,
                        stolen: None,
                        lifecycle: AssetLifecycle::Active,
                        last_journal_seq: 0,
                        // A re-issue keeps counting from the record it replaces, so an
                        // expected-revision guard written against the old title cannot
                        // match the new one by accident.
                        revision: previous.map_or(1, |r| r.saturating_add(1)),
                        frozen: false,
                        frozen_reason: None,
                    },
                );
                AssetEffectKind::Created
            }
            _ => {
                // Invariant 11: the asset must exist, be Active, and not be frozen.
                let mut post = match work.get(&object) {
                    Some(a) => a.clone(),
                    None => {
                        let a = snapshot
                            .asset(object)
                            .ok_or(EconomyError::NoSuchAsset(object))?;
                        if a.frozen {
                            return Err(EconomyError::AssetFrozen(object));
                        }
                        a.clone()
                    }
                };
                // E7 cut 3: an **abandoned** asset authorizes nobody and
                // takes no asset op but its title. `TransferTitle` stays open
                // because that is exactly the salvage claim that turns
                // possession into title (an ordinary E4b2 contract with a
                // `Before` deadline and a named adjudicator); everything else
                // — assign, encumber, maintenance, write-off, re-catalogue —
                // is refused, and the refusal is the same `AssetNotActive` a
                // written-off asset answers, so the two are indistinguishable
                // to anyone who could not already see the record.
                let title_only = matches!(post.lifecycle, AssetLifecycle::Abandoned)
                    && matches!(op, AssetOp::TransferTitle { .. });
                if post.lifecycle != AssetLifecycle::Active && !title_only {
                    return Err(EconomyError::AssetNotActive(object));
                }
                if !env.auth.authorize_asset(actor, Some(&post), op) {
                    return Err(EconomyError::Unauthorized);
                }
                let kind = apply_asset_op(op, &mut post, snapshot, env, actor)?;
                post.revision += 1;
                work.insert(object, post);
                kind
            }
        };
        let mut post = work.get(&object).expect("just inserted").clone();
        // The doc-authority latch is NEVER carried in an effect (D-E3-1).
        post.frozen = false;
        post.frozen_reason = None;
        effects.push(AssetEffect {
            object,
            kind: effect_kind,
            post,
        });
    }
    Ok(effects)
}

/// Fold one non-`Title` op into the working post-state, proving its invariants.
/// `post` arrives as the pre-state and leaves as the post-state (its `revision` is
/// bumped by the caller, once per effect).
fn apply_asset_op(
    op: &AssetOp,
    post: &mut EconomicAsset,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
    actor: &PrincipalId,
) -> Result<AssetEffectKind, EconomyError> {
    match op {
        AssetOp::Title { .. } => unreachable!("Title is handled by the caller"),
        AssetOp::TransferTitle {
            object,
            to,
            to_scope,
        } => {
            // Invariant 12: no claim may ride along. Releases sort first, so this
            // sees the post-release state.
            if !post.encumbrances.is_empty() {
                return Err(EconomyError::Encumbered(*object));
            }
            // Invariant 18 (A4: an attacker-chosen `to_scope` must not smuggle an
            // asset into an org compartment).
            if !scope_owner_coherent(*to_scope, to) {
                return Err(EconomyError::ScopeOwnerMismatch);
            }
            // Invariant 20 / D-E3-12: BOTH sides. `authorize_asset` covered the
            // source; the destination is its own question because title carries
            // liabilities a money credit does not.
            if !env.auth.authorize_title_receipt(actor, to, *to_scope) {
                return Err(EconomyError::Unauthorized);
            }
            post.title_owner = to.clone();
            post.scope = *to_scope;
            // Invariant 13: both are relationships with the OLD owner.
            post.assignment = None;
            post.maintenance_account = None;
            Ok(AssetEffectKind::TitleTransferred)
        }
        AssetOp::Assign {
            object,
            to,
            purpose,
            valid_until,
        } => {
            // Invariant 13: assignment is a relationship with someone else.
            if *to == post.title_owner {
                return Err(EconomyError::AssignmentToOwner(*object));
            }
            post.assignment = Some(AssetAssignment {
                assigned_to: to.clone(),
                purpose: purpose.clone(),
                valid_until: *valid_until,
            });
            Ok(AssetEffectKind::Assigned)
        }
        AssetOp::Unassign { .. } => {
            post.assignment = None;
            Ok(AssetEffectKind::Unassigned)
        }
        AssetOp::Encumber { encumbrance, .. } => {
            if post.encumbrances.len() >= MAX_ENCUMBRANCES_PER_ASSET {
                return Err(EconomyError::TooLarge("too many encumbrances on one asset"));
            }
            // Invariant 19 (staff-only) is the seam's answer to `authorize_asset`,
            // already asked by the caller — the pure engine just records the claim.
            post.encumbrances.push(encumbrance.clone());
            Ok(AssetEffectKind::Encumbered)
        }
        AssetOp::Release {
            object,
            encumbrance,
        } => {
            let at = post
                .encumbrances
                .iter()
                .position(|e| e == encumbrance)
                .ok_or(EconomyError::NoSuchEncumbrance(*object))?;
            post.encumbrances.remove(at);
            // E6b verify fold (R1): **the report goes with its claim.** `record.rs`
            // said clearing a theft report was `@asset/release` of its `Disputed`
            // claim, and it was not: nothing anywhere wrote `stolen = None`, so a
            // recovered asset carried the report for ever, kept rendering it to its
            // reporter, and refused a *genuine* second theft with `AlreadyClaimed`.
            // The claim and the report are one act, so they end as one act.
            if let Encumbrance::Disputed { claimant, reason } = encumbrance
                && post
                    .stolen
                    .as_ref()
                    .is_some_and(|r| &r.by == claimant && &r.reason == reason)
            {
                post.stolen = None;
            }
            Ok(AssetEffectKind::Released)
        }
        AssetOp::SetMaintenance { account, .. } => {
            if let Some(id) = account {
                let acct = snapshot
                    .account(*id)
                    .ok_or(EconomyError::NoSuchAccount(*id))?;
                if acct.status != AccountStatus::Active {
                    return Err(EconomyError::InactiveAccount(*id));
                }
                // Invariant 14: the payer must belong to the asset's owner or share
                // its compartment...
                let same_owner = acct.owner == post.title_owner;
                let same_scope = acct.scope.is_some() && acct.scope == post.scope;
                if !(same_owner || same_scope) {
                    return Err(EconomyError::ScopeOwnerMismatch);
                }
                // ...and the actor must actually be able to DEBIT it (crediting is
                // free, so a zero-delta probe through `authorize` proves nothing).
                if !env.auth.authorize_maintenance(actor, acct) {
                    return Err(EconomyError::Unauthorized);
                }
            }
            post.maintenance_account = *account;
            Ok(AssetEffectKind::MaintenanceSet)
        }
        AssetOp::Abandon { object } => {
            // Invariant 12 again, for the same reason as `WriteOff`:
            // abandoning an encumbered asset would strand the claim, and a
            // lender's collateral is not something an owner may walk away
            // from unilaterally.
            if !post.encumbrances.is_empty() {
                return Err(EconomyError::Encumbered(*object));
            }
            // `title_owner` is RETAINED — provenance survives (T3) — and
            // custody is untouched, because abandoning a thing is not moving
            // it. There is no null owner to set: `PrincipalId` has no such
            // arm, which is why 10b §3.3's drafted effect was unbuildable.
            post.lifecycle = AssetLifecycle::Abandoned;
            Ok(AssetEffectKind::Abandoned)
        }
        AssetOp::WriteOff { object, .. } => {
            // Invariant 12 again: writing off an encumbered asset would strand the
            // claim.
            if !post.encumbrances.is_empty() {
                return Err(EconomyError::Encumbered(*object));
            }
            post.lifecycle = AssetLifecycle::WrittenOff;
            Ok(AssetEffectKind::WrittenOff)
        }
        // ---- E6b: B2 — the typed theft report ----
        AssetOp::ReportStolen {
            object,
            claimant,
            visibility,
            reason,
        } => {
            // Written once: a standing report is not re-filed, because a second
            // filing would re-stamp `at` (and could widen the visibility of a
            // covert claim). Clearing one is `@asset/release` of its `Disputed`
            // claim, which is staff's — and which now clears the report with it
            // (E6b verify fold, R1).
            if post.stolen.is_some() {
                return Err(EconomyError::AlreadyClaimed(*object));
            }
            // E6b verify fold (M3): **not while a live contract is about to move
            // it.** Before E6b `Encumber` was staff-only (invariant 19), so no party
            // could put a claim on an asset; relaxing it for the title owner handed
            // them a settlement veto — accept the escrow, file a covert report, and
            // the `TransferTitle` refuses with `Encumbered` while the buyer cannot
            // even see why. A theft report is evidence, never a way out of a deal
            // you already agreed to.
            if snapshot.contracts().any(|c| {
                c.state.is_live()
                    && c.consent.terms.deliverables.iter().any(|d| match d {
                        crate::contract::Deliverable::TitleTransfer { object: o, .. }
                        | crate::contract::Deliverable::Collateral { object: o, .. } => o == object,
                        crate::contract::Deliverable::DepositInto { .. }
                        | crate::contract::Deliverable::Obligation { .. } => false,
                    })
            }) {
                return Err(EconomyError::Encumbered(*object));
            }
            if post.encumbrances.len() >= MAX_ENCUMBRANCES_PER_ASSET {
                return Err(EconomyError::TooLarge("too many encumbrances on one asset"));
            }
            let claim = Encumbrance::Disputed {
                claimant: claimant.clone(),
                reason: reason.clone(),
            };
            // The plain claim is what insurance and salvage consume (08f §7 item
            // 5); an identical one already standing is the caller's own duplicate.
            if post.encumbrances.contains(&claim) {
                return Err(EconomyError::AlreadyClaimed(*object));
            }
            post.encumbrances.push(claim);
            post.stolen = Some(crate::record::StolenReport {
                by: claimant.clone(),
                visibility: *visibility,
                reason: reason.clone(),
                at: env.now,
            });
            Ok(AssetEffectKind::StolenReported)
        }
        // ---- E6b independent verify fold (M7): catalogue an existing title ----
        AssetOp::SetCatalog { object, catalog } => {
            // A written-off title is not a thing any more; re-cataloguing one would
            // be describing a record instead of an object.
            if post.lifecycle != AssetLifecycle::Active {
                return Err(EconomyError::NoSuchAsset(*object));
            }
            if !env.auth.authorize_asset(actor, Some(post), op) {
                return Err(EconomyError::Unauthorized);
            }
            post.catalog = catalog.clone();
            Ok(AssetEffectKind::CatalogSet)
        }
    }
}

// ---- invariants 23-26: the reservations (E4a) -----------------------------

/// Everything the hold ops of one plan resolve to.
struct HoldResolution {
    /// The journal effects, in canonical order, each carrying the full post-state.
    effects: Vec<HoldEffect>,
    /// The signed change to each account's `held_amount` (widened).
    held_deltas: BTreeMap<AccountId, i128>,
    /// The accounts this plan places a hold on (invariant 23's stricter half).
    place_accounts: BTreeSet<AccountId>,
    /// How much this plan reserves on each account - the input D-E4-11 feeds to the
    /// account's `ApprovalRule` alongside its net debit.
    placed: BTreeMap<AccountId, i128>,
    /// How much this plan CAPTURES from each account's holds (positive). Two things
    /// read it (E4a verify fold, finding 1): the money lines must actually debit at
    /// least this much on the same account, and that much of the debit needs no
    /// further live authority - the placement already proved it.
    captured: BTreeMap<AccountId, i128>,
    /// The first hold captured on each account, so an unbacked capture can name one.
    capture_hold: BTreeMap<AccountId, HoldId>,
}

/// Resolve `plan.hold_ops` into journal effects, proving invariants 23-26.
///
/// Every op is authorized LIVE and per-hold, and every state change carries the FULL
/// post-state (D-E4-1), so replay rebuilds both the reservation and the account
/// projection from the journal alone. Hold ids are minted deterministically from
/// `env.next_hold_id` in canonical order, which is why `apply` never has to allocate.
fn resolve_hold_ops(
    plan: &EconomyPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> Result<HoldResolution, EconomyError> {
    let mut out = HoldResolution {
        effects: Vec::new(),
        held_deltas: BTreeMap::new(),
        place_accounts: BTreeSet::new(),
        placed: BTreeMap::new(),
        captured: BTreeMap::new(),
        capture_hold: BTreeMap::new(),
    };
    if plan.hold_ops.is_empty() {
        return Ok(out);
    }
    // The SAME total order the plan hash uses, so the applied order is the hashed
    // order and two plans differing only in op order are one plan (invariant 27).
    let mut keyed: Vec<(_, &HoldOp)> = plan
        .hold_ops
        .iter()
        .map(|op| (hold_op_key(op), op))
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    // At most ONE op per hold: two would race each other inside one bundle, and the
    // second would fold a stale pre-state.
    let mut seen: BTreeSet<HoldId> = BTreeSet::new();
    for (_, op) in &keyed {
        if let Some(id) = op.hold()
            && !seen.insert(id)
        {
            return Err(EconomyError::DuplicateHoldOp(id));
        }
    }

    let actor = &plan.actor;
    let clocks = env.clocks();
    let mut minted = 0u64;
    let mut placed_count: BTreeMap<AccountId, usize> = BTreeMap::new();

    for (_, op) in keyed {
        let effect = match op {
            HoldOp::Place {
                account,
                amount,
                beneficiary,
                purpose,
                expires,
            } => {
                if amount.is_negative() || amount.is_zero() {
                    return Err(EconomyError::HoldAmountNotPositive);
                }
                let acct = snapshot
                    .account(*account)
                    .ok_or(EconomyError::NoSuchAccount(*account))?;
                if acct.status != AccountStatus::Active {
                    return Err(EconomyError::InactiveAccount(*account));
                }
                // Invariant 23: `CreditPolicy::permits` is vacuous on an unbounded
                // floor, so a reservation there would reserve nothing at all - the
                // account would still fund every debit that ignored it.
                if acct.credit.floor.is_none() {
                    return Err(EconomyError::HoldOnUnboundedAccount(*account));
                }
                let mine = placed_count.entry(*account).or_insert(0);
                *mine += 1;
                if snapshot.active_holds_on(*account) + *mine > MAX_ACTIVE_HOLDS_PER_ACCOUNT {
                    return Err(EconomyError::TooLarge(
                        "too many active holds on one account",
                    ));
                }
                // Invariant 25: a reservation is a PROMISE TO PAY, so it needs debit
                // authority - asked as the negative probe, because crediting is free.
                let probe = amount.checked_neg().ok_or(EconomyError::Overflow)?;
                if !env.auth.authorize(actor, acct, probe) {
                    return Err(EconomyError::Unauthorized);
                }
                let id = HoldId(
                    env.next_hold_id
                        .checked_add(minted)
                        .ok_or(EconomyError::Overflow)?,
                );
                // The allocator floor is the CALLER's (`ValidateEnv::next_hold_id`),
                // and a stale one silently aims a placement at a hold that is still
                // live: `apply` would overwrite it, its reservation would vanish
                // from `held_amount`, and the escrow it was holding would become
                // spendable (E4a verify fold, finding 4). Refuse rather than mint.
                if snapshot.hold(id).is_some() {
                    return Err(EconomyError::HoldIdReused(id));
                }
                minted += 1;
                *out.held_deltas.entry(*account).or_insert(0) += amount.widen();
                *out.placed.entry(*account).or_insert(0) += amount.widen();
                out.place_accounts.insert(*account);
                HoldEffect {
                    kind: HoldEffectKind::Placed,
                    hold: Hold {
                        id,
                        account: *account,
                        amount: *amount,
                        beneficiary: beneficiary.clone(),
                        purpose: purpose.clone(),
                        placed_by: actor.clone(),
                        // doc 09 C8's `holder`: the placing plan's key, so two
                        // retries of one placement name one holder.
                        holder: plan.idempotency.clone(),
                        expires: *expires,
                        status: HoldStatus::Held,
                        last_journal_seq: 0,
                        revision: 1,
                    },
                }
            }
            HoldOp::Capture { hold, amount } => {
                let (live, _) = live_hold(snapshot, *hold)?;
                // Invariant 26: an explicit amount, never "whatever is left" - that
                // is what makes two captures on one account attributable per hold.
                if amount.is_negative() || amount.is_zero() || *amount > live.amount {
                    return Err(EconomyError::CaptureExceedsHold(*hold));
                }
                // Invariant 25 (E4a verify fold, finding 1): the hold seam is
                // MANDATORY, with no fallback to plain debit authority on the
                // account. A hold stands between an account's owner and their own
                // money on someone else's behalf; if the owner's wallet rights could
                // spend it, escrow would be a promise only the payer could break and
                // `ContractAuth`'s careful per-hold ceiling would be decoration.
                if !env.auth.authorize_hold(actor, live, op) {
                    return Err(EconomyError::Unauthorized);
                }
                // The WHOLE hold leaves `held`: the captured part becomes a real
                // debit line and the remainder is released by this same effect.
                *out.held_deltas.entry(live.account).or_insert(0) -= live.amount.widen();
                *out.captured.entry(live.account).or_insert(0) += amount.widen();
                out.capture_hold.entry(live.account).or_insert(*hold);
                let mut post = live.clone();
                // `apply` stamps the real transaction id (the acquisition pattern).
                post.status = HoldStatus::Consumed {
                    transaction: TransactionId(0),
                    amount: *amount,
                };
                post.revision += 1;
                HoldEffect {
                    kind: HoldEffectKind::Captured,
                    hold: post,
                }
            }
            HoldOp::Release { hold } => {
                let (live, _) = live_hold(snapshot, *hold)?;
                if !env.auth.authorize_hold(actor, live, op) {
                    return Err(EconomyError::Unauthorized);
                }
                *out.held_deltas.entry(live.account).or_insert(0) -= live.amount.widen();
                let mut post = live.clone();
                post.status = HoldStatus::Released;
                post.revision += 1;
                HoldEffect {
                    kind: HoldEffectKind::Released,
                    hold: post,
                }
            }
            HoldOp::Expire { hold } => {
                let (live, _) = live_hold(snapshot, *hold)?;
                // Invariant 24: expiry is the SWEEP's act. Without the cause gate any
                // caller who could name a hold could age it out early and free the
                // funds it was reserving.
                match &plan.cause {
                    EconomicCause::Internal { kind, .. } if kind == "hold.expire" => {}
                    _ => return Err(EconomyError::HoldExpireNotSweep),
                }
                let deadline = live.expires.ok_or(EconomyError::HoldNotExpired(*hold))?;
                // A `Manual` deadline resolves to `i64::MIN`, i.e. never passed: a
                // manually-clocked hold is released by hand, never aged out by a
                // sweep reading someone else's clock.
                if !clocks.passed(&deadline, snapshot, i64::MIN) {
                    return Err(EconomyError::HoldNotExpired(*hold));
                }
                if !env.auth.authorize_hold(actor, live, op) {
                    return Err(EconomyError::Unauthorized);
                }
                *out.held_deltas.entry(live.account).or_insert(0) -= live.amount.widen();
                let mut post = live.clone();
                post.status = HoldStatus::Expired;
                post.revision += 1;
                HoldEffect {
                    kind: HoldEffectKind::Expired,
                    hold: post,
                }
            }
        };
        out.effects.push(effect);
    }
    Ok(out)
}

/// Fetch a hold that is still `Held`, and prove its account exists **and is
/// `Active`** (so `apply`'s projection fold is infallible). Invariant 24's shared
/// precondition.
///
/// The account check is not tidiness: `held_amount` is a projection on the account
/// record, so every hold op writes to it. A frozen or closed account is exactly the
/// state a staffer freezes *to stop things moving*, and every money line already
/// refuses there (`validate`'s line loop) — an effect-only release or expiry would
/// otherwise be the one way to keep changing a frozen account's numbers (E4a verify
/// fold, finding 7).
fn live_hold(snapshot: &EconomySnapshot, id: HoldId) -> Result<(&Hold, &Account), EconomyError> {
    let hold = snapshot.hold(id).ok_or(EconomyError::NoSuchHold(id))?;
    if !hold.status.is_held() {
        return Err(EconomyError::HoldNotHeld(id));
    }
    let account = snapshot
        .account(hold.account)
        .ok_or(EconomyError::NoSuchAccount(hold.account))?;
    if account.status != AccountStatus::Active {
        return Err(EconomyError::InactiveAccount(hold.account));
    }
    Ok((hold, account))
}

// ---- invariant 15: custody ------------------------------------------------

fn resolve_custody_ops(
    plan: &EconomyPlan,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> Result<Vec<CustodyEffect>, EconomyError> {
    if plan.custody_ops.is_empty() {
        return Ok(Vec::new());
    }
    let mut keyed: Vec<(_, &CustodyOp)> = plan
        .custody_ops
        .iter()
        .map(|op| (custody_op_key(op), op))
        .collect();
    keyed.sort_by_key(|(k, _)| *k);
    // At most one custody op per object per plan — two would race each other inside
    // one bundle (and make the hash's (object, vault, kind) order non-total).
    let mut seen: BTreeSet<ObjId> = BTreeSet::new();
    for (_, op) in &keyed {
        if !seen.insert(op.object()) {
            return Err(EconomyError::DuplicateCustodyOp(op.object()));
        }
    }

    let actor = &plan.actor;
    let mut effects = Vec::with_capacity(keyed.len());
    for (_, op) in keyed {
        let vault_id = op.vault();
        let vault = snapshot
            .vault(vault_id)
            .ok_or(EconomyError::NoSuchVault(vault_id))?;
        if vault.status != VaultStatus::Active {
            return Err(EconomyError::VaultNotActive(vault_id));
        }
        if !env.custody.backing_valid(vault) {
            return Err(EconomyError::BackingInvalid(vault_id));
        }
        // The vault's own policy (vault.deposit / vault.withdraw), live.
        if !env.auth.authorize_vault(actor, vault, op) {
            return Err(EconomyError::Unauthorized);
        }
        // A backing with no world object cannot name a from/to for the audit trail;
        // refuse rather than invent one (E3b's providers back vaults with objects).
        let backing = vault
            .backing
            .object()
            .ok_or(EconomyError::BackingInvalid(vault_id))?;

        let effect = match op {
            CustodyOp::Deposit { object, .. } => {
                // A vault's own backing is not cargo (A2, the mirror of the
                // withdrawal rule below): nesting backing B inside vault A puts
                // everything vault B holds under vault A's withdraw policy, so
                // A's holders could extract B's contents without ever touching B.
                if snapshot.is_vault_backing(*object) {
                    return Err(EconomyError::BackingNotDepositable(*object));
                }
                if !env.custody.may_deposit(*object, actor) {
                    return Err(EconomyError::NoCustodyStanding(*object));
                }
                CustodyEffect {
                    object: *object,
                    vault: vault_id,
                    direction: CustodyDirection::Deposit,
                    // The pure engine does not resolve world locations; the live
                    // module fills the origin in when it performs the move.
                    from: None,
                    to: backing,
                }
            }
            CustodyOp::Withdraw { object, to, .. } => {
                if !env.custody.holds(vault, *object) {
                    return Err(EconomyError::NotInVault(*object));
                }
                // A live contract's `DepositInto` condition custody-LOCKS the object
                // it names (§3.5): while the contract is `Active`/`ReadyToSettle`
                // **no plan whatsoever** may withdraw it, or a seller could sell the
                // deposit out from under the buyer between attestation and
                // settlement.
                //
                // E4a verify fold, finding 2: there is no exception, not even for the
                // owning contract. Both halves of the old one were forgeable from
                // outside — `cause` is caller-settable through the public `post`, and
                // a contract's two idempotency keys are `contract/{id}/settle` and
                // `contract/{id}/refund`, derived from an id anyone who can see the
                // contract can read. The real unlock is the STATE change: settlement
                // and cancellation both move the contract out of the live states
                // first, which lifts the lock, and E4b2's refund then returns the
                // object to its depositor with a `ContractAuth`-authorized `Withdraw`
                // in the same step.
                //
                // E4b2 verify fold, M2+M3: the lock now has exactly **one** key, and
                // it is not a state and not a cause — it is the authority itself
                // (`AuthEval::authorize_custody_unlock`, `false` by default and for
                // `WorldAuth`). A `ContractAuth::unwinding` opens it for a deposit
                // that contract recorded, so the refund can post BEFORE the terminal
                // state write and a refused refund leaves a live contract to retry
                // rather than a terminal one holding the goods.
                if snapshot.custody_lock(*object, vault_id).is_some()
                    && !env.auth.authorize_custody_unlock(*object, vault_id)
                {
                    return Err(EconomyError::CustodyLocked(*object));
                }
                // A2: an unconstrained destination is a laundering primitive. The
                // seam checks live/`GOING`/reachability; the snapshot check below is
                // the one the pure engine owns — entering a vault is a Deposit, with
                // its own standing check, never a withdrawal's side effect.
                //
                // E6b (DL-03): the one exception, and it is **authority-shaped**.
                // An organization or system depositor has no world object to hand
                // the thing back to, so an unwind returning *their* recorded deposit
                // may name the backing of a vault they own. Only
                // `ContractAuth::unwinding` ever answers `true`; `WorldAuth` never
                // does, so no command can reach this path.
                if env.auth.authorize_vault_home(*object, *to) {
                    let home = snapshot
                        .vault_by_backing(&crate::record::CustodyRef::Container(*to))
                        .or_else(|| {
                            snapshot
                                .vault_by_backing(&crate::record::CustodyRef::SpatialInterior(*to))
                        })
                        .ok_or(EconomyError::BadDestination(*to))?;
                    if home.status != crate::record::VaultStatus::Active
                        || !env.custody.backing_valid(home)
                    {
                        return Err(EconomyError::BadDestination(*to));
                    }
                } else {
                    if snapshot.is_vault_backing(*to) {
                        return Err(EconomyError::BadDestination(*to));
                    }
                    if !env.custody.destination_ok(actor, *to) {
                        return Err(EconomyError::BadDestination(*to));
                    }
                }
                CustodyEffect {
                    object: *object,
                    vault: vault_id,
                    direction: CustodyDirection::Withdraw,
                    from: Some(backing),
                    to: *to,
                }
            }
        };
        effects.push(effect);
    }
    Ok(effects)
}

// ---- invariant 17: approval -----------------------------------------------

/// How many approvals a policy demands. `OneAuthorizedActor` is inert (the one
/// authorized actor is the poster) and is reported separately.
fn required_approvals(policy: &ApprovalPolicy) -> Option<usize> {
    match policy {
        ApprovalPolicy::OneAuthorizedActor => None,
        ApprovalPolicy::Threshold { approvals, .. }
        | ApprovalPolicy::NamedSet { approvals, .. } => Some(usize::from(*approvals)),
    }
}

fn evaluate_approvals(
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
    hash: [u8; 32],
    balance_deltas: &BTreeMap<AccountId, Amount>,
) -> Result<Option<ApprovalRequestId>, Fail> {
    // Which accounts' rules fire? A rule escalates on the NET DEBIT: an account that
    // nets a credit, or nets a debit at or below the threshold, is unaffected. A
    // per-op limit is not a budget (A12) — two approved plans both post.
    //
    // E6a step 1 (D-E6-21): a rule with a `window` ALSO escalates when this debit
    // would take the account's trailing-period spend (`env.spent`, placements
    // included, refunds never subtracted) past the window's total. Same path, same
    // request, same approvers: the budget is the threshold's own escalation applied
    // to a sum instead of a single line.
    let mut firing: Vec<(AccountId, &ApprovalRule)> = Vec::new();
    for (&account_id, &net) in balance_deltas {
        let account = snapshot.account(account_id).expect("resolved above");
        let Some(rule) = account.approval.as_ref() else {
            continue;
        };
        if !net.is_negative() {
            continue;
        }
        // `i64::MIN` has no positive counterpart; treat it as exceeding any threshold
        // rather than wrapping into a credit.
        let debit = net.checked_neg();
        let exceeds = debit.is_none_or(|debit| debit > rule.threshold);
        // E6a step 1 verify fold (M1): the window is a discretionary-spend control,
        // and an authority that says its posts are not discretionary — the sweep's
        // obligation settlement — skips the window clause entirely. The threshold
        // clause above still applies to it.
        let over_window = !env.auth.window_exempt()
            && rule.window.as_ref().is_some_and(|window| {
                let spent = env.spent.get(&account_id).copied().unwrap_or(Amount::ZERO);
                // An unrepresentable sum is over any window: fail closed.
                debit
                    .and_then(|d| spent.checked_add(d))
                    .is_none_or(|sum| sum > window.total)
            });
        if exceeds || over_window {
            firing.push((account_id, rule));
        }
    }
    if firing.is_empty() {
        return Ok(None);
    }
    // A11: `OneAuthorizedActor` can never gate anything, so a rule carrying it is
    // refused at set time; if one is somehow stored, fail CLOSED rather than let a
    // high-value debit through unapproved.
    let mut required: Vec<(AccountId, &ApprovalRule, usize)> = Vec::with_capacity(firing.len());
    for (account_id, rule) in firing {
        let Some(n) = required_approvals(&rule.policy) else {
            return Err(EconomyError::ApprovalRequired.into());
        };
        required.push((account_id, rule, n));
    }
    let accounts: Vec<AccountId> = required.iter().map(|(a, _, _)| *a).collect();
    if required.iter().all(|(_, _, n)| *n == 0) {
        return Ok(None);
    }
    let need = || {
        Fail::NeedsApproval(ApprovalNeed {
            plan_hash: hash,
            accounts: accounts.clone(),
        })
    };

    // One LIVE request per hash (A1). An expired or missing one is `NeedsApproval`,
    // not a refusal — the remedy is a fresh request, and the plan is otherwise fine.
    let Some(request) = snapshot.live_approval_for_hash(&hash) else {
        return Err(need());
    };
    if env.now >= request.expires_at {
        return Err(need());
    }
    for (account_id, rule, n) in &required {
        if *n == 0 {
            continue;
        }
        if !request.accounts.contains(account_id) {
            return Err(need());
        }
        let account = snapshot.account(*account_id).expect("resolved above");
        // A5: re-qualify every recorded approver LIVE. An approver who has since lost
        // the capability, the role, or their place in the named set stops counting —
        // the approval is not a captured credential.
        let mut qualified: BTreeSet<ObjId> = BTreeSet::new();
        for approval in request.approvals.iter().take(MAX_APPROVERS_PER_REQUEST) {
            // Four-eyes: the requester never approves their own request.
            if request.requested_by == PrincipalId::Object(approval.approver) {
                continue;
            }
            if env
                .auth
                .authorize_approver(approval.approver, account, &rule.policy)
            {
                qualified.insert(approval.approver);
            }
        }
        if qualified.len() < *n {
            return Err(need());
        }
    }
    Ok(Some(request.id))
}

// ---- the entry probe ------------------------------------------------------

/// Build the exact `EconomicTransaction` shape that would be journaled, purely to
/// measure its serialized size in validate.
///
/// The three values only `apply` knows — the transaction id, the journal sequence,
/// and the acquisition stamp — are filled with their WIDEST form here, so the probe
/// is an upper bound on the real entry rather than an optimistic sample (invariant
/// 21: an accepted plan must provably fit, not usually fit).
fn entry_probe(p: &Prepared) -> EconomicTransaction {
    let asset_effects: Vec<AssetEffect> = p
        .asset_effects
        .iter()
        .map(|e| {
            let mut e = e.clone();
            e.post.last_journal_seq = u64::MAX;
            e.post.acquisition = Some(AcquisitionRecord {
                transaction: TransactionId(u64::MAX),
                at: i64::MAX,
            });
            e
        })
        .collect();
    let hold_effects: Vec<HoldEffect> = p
        .hold_effects
        .iter()
        .map(|e| {
            let mut e = e.clone();
            e.hold.last_journal_seq = u64::MAX;
            if let HoldStatus::Consumed { transaction, .. } = &mut e.hold.status {
                *transaction = TransactionId(u64::MAX);
            }
            e
        })
        .collect();
    EconomicTransaction {
        id: TransactionId(u64::MAX),
        journal_seq: u64::MAX,
        idempotency: p.idempotency.clone(),
        cause: p.cause.clone(),
        actor: p.actor.clone(),
        posted_at: i64::MAX,
        description: p.description.clone(),
        lines: p.lines.clone(),
        reverses: p.reverses,
        visibility: p.visibility.clone(),
        plan_hash: p.plan_hash,
        asset_effects,
        custody_effects: p.custody_effects.clone(),
        consumed_approval: p.consumed_approval,
        hold_effects,
    }
}

// ---- apply ----------------------------------------------------------------

/// Apply a validated [`Prepared`] against `snapshot`, returning **what changes**
/// — the touched records as their post-images ([`Delta`]) — and the immutable
/// transaction. The module supplies the allocated `txid`, monotonic
/// `journal_seq`, and `posted_at`. Infallible: validation proved every checked op.
///
/// Doc 13 W1-F1: this used to clone the whole snapshot per post — every account,
/// title and reservation and the entire idempotency map — to build the post-image
/// in. The delta is O(touched records); the post path applies it to the live
/// snapshot at install, and the pure [`apply`] applies it to a clone for the rigs.
#[must_use]
pub fn apply_delta(
    snapshot: &EconomySnapshot,
    prepared: &Prepared,
    txid: TransactionId,
    journal_seq: u64,
    posted_at: i64,
) -> (Delta, EconomicTransaction) {
    let mut next = DeltaBuilder::new(snapshot);

    // Fold the reservation effects first: each carries the FULL post-state, and the
    // one thing only `apply` knows - which transaction consumed a capture - is
    // stamped here, exactly as the acquisition record is.
    let mut held_deltas: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut hold_effects = Vec::with_capacity(prepared.hold_effects.len());
    for effect in &prepared.hold_effects {
        let mut effect = effect.clone();
        effect.hold.last_journal_seq = journal_seq;
        if let HoldStatus::Consumed { transaction, .. } = &mut effect.hold.status {
            *transaction = txid;
        }
        let signed = if effect.kind == HoldEffectKind::Placed {
            effect.hold.amount.widen()
        } else {
            // A capture, a release and an expiry all take the WHOLE reservation off
            // `held`; what a capture spent is recorded inside its own status.
            -effect.hold.amount.widen()
        };
        *held_deltas.entry(effect.hold.account).or_insert(0) += signed;
        next.put_hold(effect.hold.clone());
        hold_effects.push(effect);
    }

    // One revision bump per touched account, whichever projection moved: an
    // effect-only placement moves `held_amount`/`last_hold_seq` and deliberately
    // leaves `last_journal_seq` alone (D11 - that is why the anchor is separate).
    let touched: BTreeSet<AccountId> = prepared
        .balance_deltas
        .keys()
        .chain(held_deltas.keys())
        .copied()
        .collect();
    for account_id in touched {
        let account = next
            .account_mut(account_id)
            .expect("validated account exists");
        if let Some(&net) = prepared.balance_deltas.get(&account_id) {
            account.posted_balance = account
                .posted_balance
                .checked_add(net)
                .expect("validate proved no overflow");
            account.last_journal_seq = journal_seq;
        }
        if let Some(&delta) = held_deltas.get(&account_id) {
            let widened = account.held_amount.widen() + delta;
            account.held_amount =
                Amount::from_widened(widened).expect("validate proved the held projection fits");
            account.last_hold_seq = journal_seq;
        }
        account.revision += 1;
    }

    // Fold the title effects. Each carries the FULL post-state; the two things only
    // `apply` knows — the journal sequence and the transaction id — are stamped here,
    // and the doc-authority freeze latch is carried across from the live record
    // rather than being taken from the (always-zeroed) effect.
    let mut asset_effects = Vec::with_capacity(prepared.asset_effects.len());
    for effect in &prepared.asset_effects {
        let mut effect = effect.clone();
        effect.post.last_journal_seq = journal_seq;
        effect.post.frozen = false;
        effect.post.frozen_reason = None;
        if prepared.stamp_acquisition
            && matches!(
                effect.kind,
                AssetEffectKind::Created | AssetEffectKind::TitleTransferred
            )
        {
            effect.post.acquisition = Some(AcquisitionRecord {
                transaction: txid,
                at: posted_at,
            });
        }
        let mut live = effect.post.clone();
        if let Some(prev) = next.asset(effect.object) {
            live.frozen = prev.frozen;
            live.frozen_reason = prev.frozen_reason.clone();
        }
        next.put_asset(live);
        asset_effects.push(effect);
    }

    let tx = EconomicTransaction {
        id: txid,
        journal_seq,
        idempotency: prepared.idempotency.clone(),
        cause: prepared.cause.clone(),
        actor: prepared.actor.clone(),
        posted_at,
        description: prepared.description.clone(),
        lines: prepared.lines.clone(),
        reverses: prepared.reverses,
        visibility: prepared.visibility.clone(),
        plan_hash: prepared.plan_hash,
        asset_effects,
        // Custody effects are audit-only here: the world move itself is performed by
        // the E3b module through the custody provider, in the same terminal bundle.
        custody_effects: prepared.custody_effects.clone(),
        consumed_approval: prepared.consumed_approval,
        hold_effects,
    };
    next.put_idempotency(IdempotencyRecord {
        idempotency: prepared.idempotency.clone(),
        plan_hash: prepared.plan_hash,
        transaction: txid,
        posted_at,
    });
    if let Some(target) = prepared.reverses {
        next.mark_reversed(target);
    }
    // At most once: a consumed request can never satisfy a second plan.
    if let Some(request) = prepared.consumed_approval {
        next.consume_approval(request, txid);
    }
    (next.finish(), tx)
}

/// `apply_delta` applied to a **clone** of `snapshot`: the pure form the rigs
/// and the boot-time probes use. The post path never takes this — it applies the
/// delta to the live snapshot at install (W1-F1).
#[must_use]
pub fn apply(
    snapshot: &EconomySnapshot,
    prepared: &Prepared,
    txid: TransactionId,
    journal_seq: u64,
    posted_at: i64,
) -> (EconomySnapshot, EconomicTransaction) {
    let (delta, tx) = apply_delta(snapshot, prepared, txid, journal_seq, posted_at);
    let mut next = snapshot.clone();
    delta.apply_to(&mut next);
    (next, tx)
}

/// What one post changes (doc 13 W1-F1): the touched records as their
/// post-images, in the order [`apply_delta`] produced them, plus the three
/// index facts a post records. Applied whole to a snapshot by
/// [`Delta::apply_to`]; never a clone of anything it does not touch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Delta {
    /// Every reservation the post placed, captured, released or expired.
    pub holds: Vec<crate::record::Hold>,
    /// Every account whose balance or held projection moved.
    pub accounts: Vec<crate::record::Account>,
    /// Every title the post created, transferred or otherwise changed.
    pub assets: Vec<crate::record::EconomicAsset>,
    /// The post's idempotency record.
    pub idempotency: Option<crate::record::IdempotencyRecord>,
    /// The transaction this post reverses, if any.
    pub reversed: Option<TransactionId>,
    /// The approval request this post consumed, if any, and by which transaction.
    pub consumed_approval: Option<(crate::ids::ApprovalRequestId, TransactionId)>,
}

impl Delta {
    /// Apply to `snapshot`, in the order the records were produced.
    pub fn apply_to(self, snapshot: &mut EconomySnapshot) {
        for hold in self.holds {
            snapshot.put_hold(hold);
        }
        for account in self.accounts {
            snapshot.put_account(account);
        }
        for asset in self.assets {
            snapshot.put_asset(asset);
        }
        if let Some(record) = self.idempotency {
            snapshot.put_idempotency(record);
        }
        if let Some(target) = self.reversed {
            snapshot.mark_reversed(target);
        }
        if let Some((request, transaction)) = self.consumed_approval {
            snapshot.consume_approval(request, transaction);
        }
    }
}

/// The post-image under construction: reads fall through to the base snapshot
/// until a record is touched, then to its copy here — the same six calls
/// `apply_delta` made against a cloned snapshot, answered without the clone.
struct DeltaBuilder<'a> {
    base: &'a EconomySnapshot,
    holds: BTreeMap<crate::ids::HoldId, crate::record::Hold>,
    accounts: BTreeMap<AccountId, crate::record::Account>,
    assets: BTreeMap<mush_core::ObjId, crate::record::EconomicAsset>,
    idempotency: Option<crate::record::IdempotencyRecord>,
    reversed: Option<TransactionId>,
    consumed_approval: Option<(crate::ids::ApprovalRequestId, TransactionId)>,
}

impl<'a> DeltaBuilder<'a> {
    fn new(base: &'a EconomySnapshot) -> DeltaBuilder<'a> {
        DeltaBuilder {
            base,
            holds: BTreeMap::new(),
            accounts: BTreeMap::new(),
            assets: BTreeMap::new(),
            idempotency: None,
            reversed: None,
            consumed_approval: None,
        }
    }

    fn put_hold(&mut self, hold: crate::record::Hold) {
        self.holds.insert(hold.id, hold);
    }

    /// The account's post-image so far — copied from the base on first touch.
    fn account_mut(&mut self, id: AccountId) -> Option<&mut crate::record::Account> {
        if !self.accounts.contains_key(&id) {
            let account = self.base.account(id)?.clone();
            self.accounts.insert(id, account);
        }
        self.accounts.get_mut(&id)
    }

    fn asset(&self, object: mush_core::ObjId) -> Option<&crate::record::EconomicAsset> {
        self.assets.get(&object).or_else(|| self.base.asset(object))
    }

    fn put_asset(&mut self, asset: crate::record::EconomicAsset) {
        self.assets.insert(asset.object, asset);
    }

    fn put_idempotency(&mut self, record: crate::record::IdempotencyRecord) {
        self.idempotency = Some(record);
    }

    fn mark_reversed(&mut self, target: TransactionId) {
        self.reversed = Some(target);
    }

    fn consume_approval(
        &mut self,
        request: crate::ids::ApprovalRequestId,
        transaction: TransactionId,
    ) {
        self.consumed_approval = Some((request, transaction));
    }

    fn finish(self) -> Delta {
        Delta {
            holds: self.holds.into_values().collect(),
            accounts: self.accounts.into_values().collect(),
            assets: self.assets.into_values().collect(),
            idempotency: self.idempotency,
            reversed: self.reversed,
            consumed_approval: self.consumed_approval,
        }
    }
}

// ---- reversal -------------------------------------------------------------

/// Validate the reversal of `original` — a dedicated path so the `reverses`
/// back-reference is set ONLY by the engine from a **real transaction the caller
/// supplied** (E2b fetches it from the journal by id), never a forgeable plan field.
///
/// It builds the inverse lines, uses the deterministic idempotency key
/// `econ.reverse/{id}`, and validates the inverse post against the CURRENT snapshot
/// (live balances, credit policy, authorization). A repeat resolves to
/// [`PlanResult::AlreadyPosted`]; the at-most-once [`EconomySnapshot::is_reversed`]
/// guard is the backstop. A reversal may itself be reversed — a distinct new
/// transaction with its own deterministic key.
///
/// **E3a.** It keeps its E2 signature: money is all it reverses, so it builds an
/// in-crate [`NoCustody`] and takes `now` from the original's `posted_at` (nothing on
/// this path reads the clock — invariant 17 is skipped). Two E3 rules apply:
///
/// - A target carrying asset/custody effects is **refused** ([`EconomyError::HasEffects`]):
///   money-only compensation would pay the buyer back and leave them the title (C5).
///   Compensate with an explicit `@asset/transfer`; there is no `/force` in E3.
/// - Invariant 17 is **skipped** (A7): a staff reversal is not a new spend, so an
///   `ApprovalRule` must never become a shield against clawback.
#[must_use]
pub fn plan_reversal(
    original: &EconomicTransaction,
    actor: PrincipalId,
    cause: crate::record::EconomicCause,
    snapshot: &EconomySnapshot,
    auth: &dyn AuthEval,
    keys: &dyn KeyGate,
) -> PlanResult {
    if let Err(error) = crate::contract::managed::require_legacy_transaction(original, snapshot) {
        return PlanResult::Refused(error);
    }
    // Invariant 28 extends E3's rule to reservations: money-only compensation would
    // pay the payer back and leave the hold `Consumed`, so the payee's credit
    // vanishes while the reservation still says it was spent. A capture is unwound by
    // a contract refund or an attributed compensating plan, never by a reversal.
    if !original.asset_effects.is_empty()
        || !original.custody_effects.is_empty()
        || !original.hold_effects.is_empty()
    {
        return PlanResult::Refused(EconomyError::HasEffects);
    }
    // Inverse lines; `i64::MIN` has no negation (cannot arise from a balanced posted
    // set in practice, but refused rather than wrapped).
    let mut lines = Vec::with_capacity(original.lines.len());
    for l in &original.lines {
        let Some(neg) = l.delta.checked_neg() else {
            return PlanResult::Refused(EconomyError::Overflow);
        };
        lines.push(crate::plan::RequestedLine {
            account: l.account,
            delta: neg,
            memo: l.memo.clone(),
        });
    }
    let plan = EconomyPlan {
        idempotency: crate::record::IdempotencyKey::reversal_of(original.id),
        actor,
        description: format!("reversal of transaction {}", original.id),
        cause,
        visibility: original.visibility.clone(),
        requested_lines: lines,
        expected_revisions: Vec::new(),
        asset_ops: Vec::new(),
        custody_ops: Vec::new(),
        expected_asset_revisions: Vec::new(),
        hold_ops: Vec::new(),
        expected_hold_revisions: Vec::new(),
    };
    // The reversal's identity is the TARGET, not the caller's cause/actor — so a
    // re-reverse with a different staff reason still resolves to AlreadyPosted.
    let hash = crate::hash::reversal_hash(original.id);

    // A clean retry of the SAME reversal returns the original reversal id; a key
    // the gate cannot decide from RAM is a transient (doc 13, W-D1).
    match keys.verdict(snapshot, &plan.idempotency) {
        KeyVerdict::Known(rec) => {
            if rec.plan_hash == hash {
                return PlanResult::AlreadyPosted(rec.transaction);
            }
            return PlanResult::Refused(EconomyError::IdempotencyConflict);
        }
        KeyVerdict::Unknown => return PlanResult::Refused(EconomyError::RetryShortly),
        KeyVerdict::Fresh => {}
    }
    // At-most-once backstop: refuse if the target is already reversed by any path.
    if snapshot.is_reversed(original.id) {
        return PlanResult::Refused(EconomyError::AlreadyReversed(original.id));
    }
    let env = ValidateEnv {
        auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: original.posted_at,
        ic_now: original.posted_at,
        next_hold_id: 0,
        // Invariant 17 is skipped on this path, so no window is ever consulted.
        spent: BTreeMap::new(),
        keys,
    };
    match validate_inner(&plan, snapshot, &env, hash, true, PlanGuard::Legacy) {
        Ok(mut prepared) => {
            prepared.reverses = Some(original.id);
            PlanResult::Post(Box::new(prepared))
        }
        Err(Fail::Refused(e)) => PlanResult::Refused(e),
        // Unreachable: `skip_approval` is set on this path.
        Err(Fail::NeedsApproval(_)) => PlanResult::Refused(EconomyError::ApprovalRequired),
    }
}
