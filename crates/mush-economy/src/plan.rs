//! The economy plan alphabet + the validation outcome (E2a).
//!
//! A caller builds an [`EconomyPlan`] (what should move, and why); the pure engine
//! ([`crate::validate`]) checks it against one immutable snapshot and returns a
//! [`PlanResult`]. On [`PlanResult::Post`] the [`Prepared`] carries everything E2b
//! needs to build the immutable transaction and fold the projections — the module
//! only supplies the allocated ids, sequence, and post time; the apply is infallible
//! because validation already proved it against the pre-state.

use std::collections::BTreeMap;

use mush_core::ObjId;
use mush_org::{OrgScopeId, PrincipalId};
use serde::Serialize;
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::ids::{AccountId, ApprovalRequestId, InstrumentId, TransactionId, VaultId};
use crate::record::{
    AssetEffect, CatalogRef, CustodyEffect, EconomicCause, EconomicVisibility, Encumbrance,
    IdempotencyKey, JournalLine,
};

/// One requested movement: a signed delta to an account. The instrument is derived
/// from the account during validation (the caller cannot pick a mismatched one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestedLine {
    /// The account to move.
    pub account: AccountId,
    /// The signed delta.
    pub delta: Amount,
    /// A per-line memo.
    pub memo: SmolStr,
}

/// An optimistic-concurrency expectation on **economic** data (a balance/quote/
/// account revision). Authorization is NOT captured here — it re-runs live at post
/// (R-D4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRevision {
    /// The account.
    pub account: AccountId,
    /// Its expected current revision.
    pub revision: u64,
}

/// An optimistic-concurrency expectation on an **asset** (E3a, invariant 16) — the
/// `AccountRevision` shape for title data. Excluded from the plan hash like
/// `expected_revisions`, so an approval still authorizes the plan at a later
/// revision (approvals authorize an intent, not a state).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AssetRevision {
    /// The asset (keyed by the object it titles).
    pub object: ObjId,
    /// Its expected current revision.
    pub revision: u64,
}

/// One requested change to a title record (E3a). Title, custody, operation, and
/// maintenance are four independent axes (D5): none of these ops moves an object in
/// the world, and no [`CustodyOp`] moves title.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum AssetOp {
    /// Create a title record for an untitled object. **Staff-only in E3**
    /// (invariant 20); a title-issuing ruleset principal is D-E3-13.
    Title {
        /// The object to title.
        object: ObjId,
        /// Who receives title.
        owner: PrincipalId,
        /// The org compartment (invariant 18).
        scope: Option<OrgScopeId>,
        /// The opaque catalog reference.
        catalog: Option<CatalogRef>,
    },
    /// Move title. Needs authority on BOTH sides (D-E3-12) and clears the
    /// assignment + maintenance payer (invariant 13).
    TransferTitle {
        /// The asset.
        object: ObjId,
        /// The new title owner.
        to: PrincipalId,
        /// The new org compartment (invariant 18).
        to_scope: Option<OrgScopeId>,
    },
    /// Assign the asset to someone other than its title owner (invariant 13).
    Assign {
        /// The asset.
        object: ObjId,
        /// The assignee.
        to: PrincipalId,
        /// Free-text purpose (≤ [`crate::MAX_REASON_BYTES`]).
        purpose: SmolStr,
        /// Optional expiry.
        valid_until: Option<i64>,
    },
    /// Clear the assignment.
    Unassign {
        /// The asset.
        object: ObjId,
    },
    /// Record a claim. **Staff-only in E3** (invariant 19).
    Encumber {
        /// The asset.
        object: ObjId,
        /// The claim.
        encumbrance: Encumbrance,
    },
    /// Release a claim — an **exact match** of a recorded encumbrance.
    Release {
        /// The asset.
        object: ObjId,
        /// The exact claim to remove.
        encumbrance: Encumbrance,
    },
    /// Point (or clear) the maintenance payer (invariant 14).
    SetMaintenance {
        /// The asset.
        object: ObjId,
        /// The paying account, or `None` to clear.
        account: Option<AccountId>,
    },
    /// Write the asset off. Terminal; refuses an encumbered asset (invariant 12).
    WriteOff {
        /// The asset.
        object: ObjId,
        /// Why (≤ [`crate::MAX_REASON_BYTES`]).
        reason: String,
    },
}

impl AssetOp {
    /// The object this op targets.
    #[must_use]
    pub fn object(&self) -> ObjId {
        match self {
            AssetOp::Title { object, .. }
            | AssetOp::TransferTitle { object, .. }
            | AssetOp::Assign { object, .. }
            | AssetOp::Unassign { object }
            | AssetOp::Encumber { object, .. }
            | AssetOp::Release { object, .. }
            | AssetOp::SetMaintenance { object, .. }
            | AssetOp::WriteOff { object, .. } => *object,
        }
    }
    /// The canonical op ordinal — the second key of the plan-hash total order, and
    /// the order ops are applied in within one object (releases first, then the
    /// title change, then the relationship ops, write-off last).
    #[must_use]
    pub fn ordinal(&self) -> u8 {
        match self {
            AssetOp::Release { .. } => 0,
            AssetOp::Title { .. } => 1,
            AssetOp::TransferTitle { .. } => 2,
            AssetOp::Encumber { .. } => 3,
            AssetOp::Assign { .. } => 4,
            AssetOp::Unassign { .. } => 5,
            AssetOp::SetMaintenance { .. } => 6,
            AssetOp::WriteOff { .. } => 7,
        }
    }
    /// Whether this op changes who holds title (`Title` / `TransferTitle` /
    /// `WriteOff` are mutually exclusive per object, invariant 11).
    #[must_use]
    pub fn is_title_changing(&self) -> bool {
        matches!(self, AssetOp::Title { .. } | AssetOp::TransferTitle { .. } | AssetOp::WriteOff { .. })
    }
}

/// One requested custody movement (E3a). Custody never touches title (D5).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
pub enum CustodyOp {
    /// Put an object into a vault. Requires custody standing on the object.
    Deposit {
        /// The object.
        object: ObjId,
        /// The vault.
        vault: VaultId,
    },
    /// Take an object out of a vault to `to` (constrained by invariant 15: the
    /// actor or the actor's location, live, not `GOING`, and never another vault's
    /// backing — entering a vault is a `Deposit`).
    Withdraw {
        /// The object.
        object: ObjId,
        /// The vault.
        vault: VaultId,
        /// Where it lands.
        to: ObjId,
    },
}

impl CustodyOp {
    /// The object this op moves.
    #[must_use]
    pub fn object(&self) -> ObjId {
        match self {
            CustodyOp::Deposit { object, .. } | CustodyOp::Withdraw { object, .. } => *object,
        }
    }
    /// The vault this op targets.
    #[must_use]
    pub fn vault(&self) -> VaultId {
        match self {
            CustodyOp::Deposit { vault, .. } | CustodyOp::Withdraw { vault, .. } => *vault,
        }
    }
    /// The canonical op ordinal (the third key of the plan-hash total order).
    #[must_use]
    pub fn ordinal(&self) -> u8 {
        match self {
            CustodyOp::Deposit { .. } => 0,
            CustodyOp::Withdraw { .. } => 1,
        }
    }
}

/// What a plan still needs before it can post: approvals bound to this exact hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalNeed {
    /// The plan hash an [`crate::ApprovalRequest`] must bind (E3b2 mints it).
    pub plan_hash: [u8; 32],
    /// The accounts whose [`crate::ApprovalRule`]s fired.
    pub accounts: Vec<AccountId>,
}

/// A validated-then-applied economic plan. The `hold_ops`/`contract_ops`/`world_ops`
/// vectors of doc 08 are forward-declared as **empty** (holds and contracts are E4)
/// and are simply absent from this struct until then; their arrival does not change
/// the persisted transaction shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EconomyPlan {
    /// The idempotency key (mandatory — repeat-safe posting).
    pub idempotency: IdempotencyKey,
    /// Who is posting.
    pub actor: PrincipalId,
    /// Human description.
    pub description: String,
    /// Why.
    pub cause: EconomicCause,
    /// Recorded visibility (not enforced in E2).
    pub visibility: EconomicVisibility,
    /// The requested movements (≥2 non-zero, per-instrument zero-sum).
    pub requested_lines: Vec<RequestedLine>,
    /// Optimistic staleness expectations on economic data (≤
    /// [`crate::MAX_LINES_PER_TX`]).
    pub expected_revisions: Vec<AccountRevision>,
    /// Title changes (≤ [`crate::MAX_ASSET_OPS_PER_PLAN`]).
    pub asset_ops: Vec<AssetOp>,
    /// Custody movements (≤ [`crate::MAX_CUSTODY_OPS_PER_PLAN`]; ≤ 1 per object).
    pub custody_ops: Vec<CustodyOp>,
    /// Optimistic staleness expectations on assets (≤
    /// [`crate::MAX_ASSET_OPS_PER_PLAN`]).
    pub expected_asset_revisions: Vec<AssetRevision>,
    // NOTE: a plan carries NO caller-settable `reverses` — a reversal's back-
    // reference is set ONLY by the engine ([`crate::reverse`]) from a real original
    // transaction, so it can never be forged onto an ordinary post (which would let
    // apply falsely latch an unrelated or not-yet-allocated transaction as reversed —
    // the E2a verify finding). Reversal is a dedicated validated path, not a field.
}

impl EconomyPlan {
    /// A minimal plan (no expected-revision guards).
    #[must_use]
    pub fn new(
        idempotency: IdempotencyKey,
        actor: PrincipalId,
        cause: EconomicCause,
        lines: Vec<RequestedLine>,
    ) -> EconomyPlan {
        EconomyPlan {
            idempotency,
            actor,
            description: String::new(),
            cause,
            visibility: EconomicVisibility::default(),
            requested_lines: lines,
            expected_revisions: Vec::new(),
            asset_ops: Vec::new(),
            custody_ops: Vec::new(),
            expected_asset_revisions: Vec::new(),
        }
    }
}

/// The outcome of validating a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanResult {
    /// Validated; the [`Prepared`] applies infallibly.
    Post(Box<Prepared>),
    /// This exact plan already posted under this idempotency key — the original id
    /// (no new transaction). E2b still ack-gates a HIT during the original's
    /// durability window (R-R2).
    AlreadyPosted(TransactionId),
    /// A per-account [`crate::ApprovalRule`] fired and is not (yet) satisfied by a
    /// live request for this hash — **not** a refusal: the plan is well-formed and
    /// authorized, it just needs approvals (invariant 17). E3b2 mints/serves the
    /// request; E3a callers surface [`EconomyError::ApprovalRequired`].
    NeedsApproval(Box<ApprovalNeed>),
    /// Refused; nothing mutates.
    Refused(EconomyError),
}

impl PlanResult {
    /// Whether the plan would post a new transaction.
    #[must_use]
    pub fn is_post(&self) -> bool {
        matches!(self, PlanResult::Post(_))
    }
}

/// A validated post, ready to apply. Carries the resolved, canonically-ordered lines
/// (instrument filled in), the plan hash, the net per-account deltas (checked), and
/// the attribution the module stamps onto the immutable transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    /// The idempotency key this post satisfies.
    pub idempotency: IdempotencyKey,
    /// The canonical plan hash (idempotency + approval link).
    pub plan_hash: [u8; 32],
    /// Who posted.
    pub actor: PrincipalId,
    /// Why.
    pub cause: EconomicCause,
    /// Description.
    pub description: String,
    /// Recorded visibility.
    pub visibility: EconomicVisibility,
    /// The resolved journal lines (instrument filled), sorted canonically.
    pub lines: Vec<JournalLine>,
    /// The transaction this reverses, if any (set only by the reversal path).
    pub reverses: Option<TransactionId>,
    /// Net delta to fold into each touched account's posted balance (checked).
    pub balance_deltas: BTreeMap<AccountId, Amount>,
    /// The asset changes, in canonical order, each carrying the full post-state
    /// (`frozen` fields zeroed). `apply` stamps `last_journal_seq` and — when
    /// `stamp_acquisition` — the acquisition record.
    pub asset_effects: Vec<AssetEffect>,
    /// The custody movements (audit; the world move is E3b's).
    pub custody_effects: Vec<CustodyEffect>,
    /// The approval request this post consumes, if any.
    pub consumed_approval: Option<ApprovalRequestId>,
    /// Invariant 22: this transaction carries ≥1 money line, so `apply` stamps
    /// `acquisition` on every `Created`/`TitleTransferred` effect.
    pub stamp_acquisition: bool,
}

/// Instrument mapped per touched account, so E2b can update `last_journal_seq` /
/// revision on exactly the affected accounts.
impl Prepared {
    /// The accounts this post touches (ascending).
    pub fn touched_accounts(&self) -> impl Iterator<Item = AccountId> + '_ {
        self.balance_deltas.keys().copied()
    }
}

/// Why a plan was refused. These carry precise detail for **trusted server callers
/// only** — E2 exposes no non-staff economic read (R-D2), so a disclosure filter is
/// not needed until E4.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EconomyError {
    /// Fewer than two non-zero lines (invariant 1).
    #[error("a transaction needs at least two non-zero lines")]
    TooFewLines,
    /// A line's delta is zero (a no-op line is not allowed).
    #[error("line {0} has a zero delta")]
    ZeroLine(usize),
    /// A referenced account does not exist.
    #[error("no such account: {0}")]
    NoSuchAccount(AccountId),
    /// A referenced transaction does not exist (reversal target).
    #[error("no such transaction: {0}")]
    NoSuchTransaction(TransactionId),
    /// A referenced account is not active.
    #[error("account {0} is not active")]
    InactiveAccount(AccountId),
    /// A referenced instrument does not exist.
    #[error("no such instrument: {0}")]
    NoSuchInstrument(InstrumentId),
    /// A referenced instrument is not active.
    #[error("instrument {0} is not active")]
    InactiveInstrument(InstrumentId),
    /// The deltas for an instrument do not sum to zero (invariant 3).
    #[error("instrument {0} is not balanced")]
    NotBalanced(InstrumentId),
    /// Checked arithmetic overflowed (invariant 4).
    #[error("arithmetic overflow")]
    Overflow,
    /// A projected balance would breach an account's credit policy.
    #[error("account {0} would breach its credit policy")]
    CreditViolation(AccountId),
    /// The actor is not authorized to move value on an account.
    #[error("unauthorized")]
    Unauthorized,
    /// The idempotency key was reused with a DIFFERENT plan (invariant/audit).
    #[error("idempotency key reused with a different plan")]
    IdempotencyConflict,
    /// The target of a reversal was already reversed (at-most-once, R-B4).
    #[error("transaction {0} was already reversed")]
    AlreadyReversed(TransactionId),
    /// An expected-revision guard did not match current economic state.
    #[error("stale plan: account {account} expected revision {expected}, found {found:?}")]
    Stale {
        /// The account.
        account: AccountId,
        /// The expected revision.
        expected: u64,
        /// The current revision, if the account exists.
        found: Option<u64>,
    },
    /// A boundedness cap was exceeded (R-B5) — refused at validate, so apply can
    /// never build a poison-sized chunk.
    #[error("plan exceeds a boundedness cap: {0}")]
    TooLarge(&'static str),
    /// The whole economy is frozen (boot found drift / a conservation break / a
    /// dropped durable write) — no posting until staff clear it.
    #[error("the economy is frozen")]
    EconomyFrozen,

    // ---- E3a: assets, vaults, approvals ----------------------------------
    /// A referenced asset has no title record.
    #[error("no title record for object {0}")]
    NoSuchAsset(ObjId),
    /// `Title` was asked for an object that already has a title record.
    #[error("object {0} is already titled")]
    AssetExists(ObjId),
    /// The asset is not `Active` (written off).
    #[error("asset {0} is not active")]
    AssetNotActive(ObjId),
    /// The asset carries the doc-authority freeze latch.
    #[error("asset {0} is frozen")]
    AssetFrozen(ObjId),
    /// A title change was asked of an asset carrying claims the plan does not
    /// release (invariant 12).
    #[error("asset {0} is encumbered")]
    Encumbered(ObjId),
    /// `Release` named a claim the asset does not carry (exact match required).
    #[error("asset {0} does not carry that encumbrance")]
    NoSuchEncumbrance(ObjId),
    /// `Assign` named the title owner (invariant 13 — assignment is a relationship
    /// with someone OTHER than the owner).
    #[error("asset {0} cannot be assigned to its own title owner")]
    AssignmentToOwner(ObjId),
    /// A referenced vault does not exist.
    #[error("no such vault: {0}")]
    NoSuchVault(VaultId),
    /// The vault is not `Active`.
    #[error("vault {0} is not active")]
    VaultNotActive(VaultId),
    /// `Withdraw` named an object the vault does not hold.
    #[error("object {0} is not in that vault")]
    NotInVault(ObjId),
    /// The actor has no custody standing on the object (not carrying it, not the
    /// owner/assignee/controller co-located with it, not staff).
    #[error("no custody standing over object {0}")]
    NoCustodyStanding(ObjId),
    /// A withdrawal destination is not the actor / the actor's location, is dead or
    /// `GOING`, or is itself a vault backing (entering one is a deposit).
    #[error("object {0} is not a valid destination")]
    BadDestination(ObjId),
    /// The vault's backing is not a usable custody container.
    #[error("vault {0} has an invalid backing")]
    BackingInvalid(VaultId),
    /// Two conflicting asset ops on one object in one plan (invariant 11).
    #[error("conflicting asset ops on object {0}")]
    DuplicateAssetOp(ObjId),
    /// More than one custody op on one object in one plan (invariant 15).
    #[error("more than one custody op on object {0}")]
    DuplicateCustodyOp(ObjId),
    /// Scope/owner coherence broken (invariant 18): a scoped account/asset/vault
    /// must be owned by that scope's organization, and an unscoped transfer target
    /// must carry no scope.
    #[error("scope and owner do not agree")]
    ScopeOwnerMismatch,
    /// A live approval request aged past its TTL. (Raised by the E3b approval
    /// commands; `validate` answers [`PlanResult::NeedsApproval`] for an expired
    /// request, because the remedy is a NEW request, not a refusal.)
    #[error("that approval request has expired")]
    ApprovalExpired,
    /// This plan needs approvals the caller has not collected. `validate` itself
    /// returns [`PlanResult::NeedsApproval`]; this is the error the E3a `post` seam
    /// surfaces until E3b2 mints the request.
    #[error("this plan requires approval")]
    ApprovalRequired,
    /// A reversal target carries asset/custody effects: money-only compensation
    /// would leave title moved and paid back (C5). Compensate with an explicit
    /// `@asset/transfer` instead — there is no `/force` in E3.
    #[error("that transaction carries asset or custody effects and cannot be reversed")]
    HasEffects,
    /// An expected-asset-revision guard did not match current title state.
    #[error("stale plan: asset {object} expected revision {expected}, found {found:?}")]
    StaleAsset {
        /// The asset.
        object: ObjId,
        /// The expected revision.
        expected: u64,
        /// The current revision, if the asset exists.
        found: Option<u64>,
    },
}
