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
use crate::clock::EconomicDeadline;
use crate::ids::{
    AccountId, ApprovalRequestId, ContractId, HoldId, InstrumentId, ObligationId, TransactionId,
    VaultId,
};
use crate::record::{
    AssetEffect, CatalogRef, CustodyEffect, EconomicCause, EconomicVisibility, Encumbrance,
    HoldEffect, HoldPurpose, IdempotencyKey, JournalLine,
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

/// An optimistic-concurrency expectation on a **hold** (E4a, invariant 26) — the
/// [`AccountRevision`] shape for reservation data, and the exit criterion's "stale
/// revisions" guard: a second attest or a racing capture built against a hold that
/// has since moved is refused rather than applied to a different reservation.
/// Excluded from the plan hash like every other expectation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HoldRevision {
    /// The hold.
    pub hold: HoldId,
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
    /// **Report the asset stolen** (E6b, D-E6-18, 08e answer 14).
    ///
    /// The one relaxation of invariant 19 (`Encumber` is staff-only): the title
    /// owner, an organization holding `asset.insure` on the asset's compartment,
    /// or staff may file. The effect is a plain [`Encumbrance::Disputed`] — so
    /// insurance and salvage keep consuming exactly what they consume today —
    /// **plus** a bounded [`crate::record::StolenReport`] stamped on the asset,
    /// which is where the public/restricted/covert visibility lives.
    ///
    /// Title never moves: a fence buys custody, and a later purchase generates
    /// evidence, not guilt (D-E6-10).
    ReportStolen {
        /// The asset.
        object: ObjId,
        /// Who is claiming it was stolen.
        claimant: PrincipalId,
        /// Who may be told.
        visibility: crate::record::StolenVisibility,
        /// Why (≤ [`crate::MAX_REASON_BYTES`]).
        reason: String,
    },
    /// **Set (or clear) the catalogue reference** on a live title (E6b independent
    /// verify fold, M7).
    ///
    /// `AssetOp::Title` carries a `catalog`, and nothing else ever set one: a title
    /// created before a catalogue existed — which is every title minted before E6b —
    /// could never acquire one, so every catalogue-keyed rule (the fence baseline
    /// first) was permanently unreachable for it, and `Title` refuses an object that
    /// already has a live record. This is the missing verb, and it is deliberately
    /// **not** a title change: the owner does not move, the encumbrances do not move,
    /// and the acquisition does not move.
    ///
    /// It bumps the asset's `revision` like every other asset op, which is what makes
    /// a re-catalogue lapse an outstanding quote **deterministically** rather than
    /// silently re-pricing it (D-E6-13).
    SetCatalog {
        /// The asset.
        object: ObjId,
        /// The new reference, or `None` to clear it.
        catalog: Option<crate::record::CatalogRef>,
    },
    /// **Abandon** the asset (E7 cut 3; doc 10 T3, §15; 08f §1.1's "in E7 with
    /// the mechanical recipe written out").
    ///
    /// A **direct attributed asset action** that need not masquerade as a
    /// bilateral contract, next to the claim seizure that needs the same
    /// unilateral title effect. It moves `lifecycle` to
    /// [`AssetLifecycle::Abandoned`](crate::record::AssetLifecycle::Abandoned),
    /// **retains `title_owner`** as the last owner so provenance survives
    /// (T3), and **touches custody not at all**. The salvage claim that turns
    /// possession into title stays an ordinary E4b2 contract with a `Before`
    /// contest deadline and a named adjudicator; cut 3 writes none.
    Abandon {
        /// The titled object.
        object: ObjId,
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
            | AssetOp::WriteOff { object, .. }
            | AssetOp::ReportStolen { object, .. }
            | AssetOp::SetCatalog { object, .. }
            | AssetOp::Abandon { object } => *object,
        }
    }
    /// The canonical op ordinal — the second key of the plan-hash total order, and
    /// the order ops are applied in within one object (releases first, then the
    /// title change, then the relationship ops, write-off last).
    ///
    /// **Never renumber an existing ordinal.** It is the second key of the plan
    /// hash's total order, so every stored idempotency record for a title or
    /// custody post resolves through it; E6b's `ReportStolen` therefore takes the
    /// next free value, 8, and the two hard-coded digest pins prove nothing moved.
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
            AssetOp::ReportStolen { .. } => 8,
            AssetOp::SetCatalog { .. } => 9,
            // E7 cut 3 takes the next free value, 10, and renumbers nothing:
            // the ordinal is the second key of the plan-hash total order, and
            // the two hard-coded digest pins prove nothing moved. It sorts
            // **after** `WriteOff` because both are terminal statements about
            // the title and neither may precede the other in one plan.
            AssetOp::Abandon { .. } => 10,
        }
    }
    /// Whether this op changes who holds title (`Title` / `TransferTitle` /
    /// `WriteOff` are mutually exclusive per object, invariant 11).
    #[must_use]
    pub fn is_title_changing(&self) -> bool {
        matches!(
            self,
            AssetOp::Title { .. }
                | AssetOp::TransferTitle { .. }
                | AssetOp::WriteOff { .. }
                | AssetOp::Abandon { .. }
        )
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

/// One requested change to a reservation (E4a, §3.2). A hold op is an **effect**,
/// not a money line: a plan carrying only hold ops is the effect-only shape
/// invariant 10 admits, and `Account.held_amount` is the projection it folds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum HoldOp {
    /// Reserve `amount` on `account`. Needs **debit authority** — a reservation is a
    /// promise to pay (invariant 25) — and is subject to the account's
    /// [`crate::ApprovalRule`] like any other debit-authority plan (D-E4-11).
    Place {
        /// The account to reserve against.
        account: AccountId,
        /// How much (> 0, ≤ the account's available funds at placement).
        amount: Amount,
        /// Who the reservation is for, if anyone.
        beneficiary: Option<PrincipalId>,
        /// What it reserves for.
        purpose: HoldPurpose,
        /// When it lapses, on a named clock.
        expires: Option<EconomicDeadline>,
    },
    /// Spend (part of) a hold. `amount` is **explicit** and ≤ `hold.amount`; the
    /// uncaptured remainder is released by the same effect, so several holds on one
    /// account can be captured in one plan and each is attributed exactly (A1).
    Capture {
        /// The hold to spend.
        hold: HoldId,
        /// How much of it (> 0, ≤ `hold.amount`).
        amount: Amount,
    },
    /// Release a hold unspent (the placer, the beneficiary, or staff).
    Release {
        /// The hold.
        hold: HoldId,
    },
    /// Age a hold out past its deadline. **Sweep-only**: legal only with cause
    /// `Internal { kind: "hold.expire" }` and a deadline that has passed on its own
    /// clock (invariant 24).
    Expire {
        /// The hold.
        hold: HoldId,
    },
}

impl HoldOp {
    /// The hold this op targets, or `None` for a [`HoldOp::Place`] (which mints one).
    #[must_use]
    pub fn hold(&self) -> Option<HoldId> {
        match self {
            HoldOp::Place { .. } => None,
            HoldOp::Capture { hold, .. } | HoldOp::Release { hold } | HoldOp::Expire { hold } => {
                Some(*hold)
            }
        }
    }
    /// The **subject id** — the first key of the total order (invariant 27): the
    /// hold for an op that names one, the account for a `Place`, which does not yet
    /// have a hold id to be ordered by.
    #[must_use]
    pub fn subject(&self) -> u64 {
        match self {
            HoldOp::Place { account, .. } => account.0,
            HoldOp::Capture { hold, .. } | HoldOp::Release { hold } | HoldOp::Expire { hold } => {
                hold.0
            }
        }
    }
    /// The canonical op ordinal — the **second** key of the total order, which only
    /// breaks ties between ops on the same subject id. It does **not** put every
    /// `Place` first: a `Place` is keyed by its account and a capture by its hold,
    /// and those two id spaces are unrelated. What the order guarantees is that it
    /// is total and deterministic, so the applied order is the hashed order.
    #[must_use]
    pub fn ordinal(&self) -> u8 {
        match self {
            HoldOp::Place { .. } => 0,
            HoldOp::Capture { .. } => 1,
            HoldOp::Release { .. } => 2,
            HoldOp::Expire { .. } => 3,
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

/// A validated-then-applied economic plan. Doc 08's `contract_ops`/`world_ops`
/// vectors are still absent (a contract acts by *building* a plan — §3.5's
/// `settlement_plan` — rather than by carrying contract ops into one); `hold_ops`
/// arrived in E4a without changing the persisted transaction shape.
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
    /// Reservation changes (≤ [`crate::MAX_HOLD_OPS_PER_PLAN`]; ≤ 1 per hold).
    pub hold_ops: Vec<HoldOp>,
    /// Optimistic staleness expectations on holds (≤
    /// [`crate::MAX_HOLD_OPS_PER_PLAN`]).
    pub expected_hold_revisions: Vec<HoldRevision>,
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
            hold_ops: Vec::new(),
            expected_hold_revisions: Vec::new(),
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
    /// The reservation changes, in canonical order, each carrying the full
    /// post-state with its allocated [`HoldId`]. `apply` stamps `last_journal_seq`
    /// and — for a capture — the consuming transaction id.
    pub hold_effects: Vec<HoldEffect>,
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

/// One economic resource a refusal names — the input to the command layer's
/// existence-hiding renderer (§4.5, D-E3-6).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NamedResource {
    /// An account.
    Account(AccountId),
    /// A **title record**, keyed by the object it titles.
    Asset(ObjId),
    /// A vault.
    Vault(VaultId),
    /// A hold (E4a).
    Hold(HoldId),
    /// An obligation (E4a).
    Obligation(ObligationId),
    /// A contract (E4a).
    Contract(ContractId),
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
    /// [`crate::register_instrument`] was handed an id that is already registered
    /// (look-back fold, E2-C18). Registration is create-only: replacing a live
    /// definition would let a second call flip `status`, `scale` or the negative
    /// policy under every account of that instrument.
    #[error("instrument {0} is already registered")]
    InstrumentExists(InstrumentId),
    /// A hold or release on an obligation that is `Cancelled` or `Satisfied`
    /// (look-back fold, D-LBK-1): a finished obligation is nobody's to suspend.
    #[error("obligation {0} is finished")]
    ObligationTerminal(ObligationId),
    /// The deltas for an instrument do not sum to zero (invariant 3).
    #[error("instrument {0} is not balanced")]
    NotBalanced(InstrumentId),
    /// Checked arithmetic overflowed (invariant 4).
    #[error("arithmetic overflow")]
    Overflow,
    /// A faucet-funded obligation's per-period amount is above the configured mint
    /// cap (E4b1 verify fold, M1). Refused at creation and re-checked at **every**
    /// settlement, so a record edited above the cap afterwards stops being paid.
    #[error("that would mint more than the configured cap allows for account {0}")]
    MintCapExceeded(AccountId),
    /// The authoritative journal could not accept or install the exact prepared
    /// transaction append. No economic post may be confirmed in this state.
    #[error("the authoritative economy journal is unavailable")]
    JournalUnavailable,
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
    /// `ReportStolen` on an asset that already carries a standing theft report, or
    /// whose identical `Disputed` claim already stands (E6b, B2). A report is
    /// written **once**: re-filing would re-stamp its clock and could widen a
    /// covert claim, and clearing one is staff's `@asset/release`.
    #[error("asset {0} already carries that claim")]
    AlreadyClaimed(ObjId),
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
    /// `GOING`, or is (or sits inside) a vault backing (entering one is a deposit).
    #[error("object {0} is not a valid destination")]
    BadDestination(ObjId),
    /// A `Deposit` named an object that is itself the backing of a vault. Nesting a
    /// vault's backing inside another vault puts the inner vault's contents under
    /// the OUTER vault's withdraw policy — a custody-laundering primitive.
    #[error("object {0} backs a vault and cannot be put inside another")]
    BackingNotDepositable(ObjId),
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
    /// A custody provider refused (or could not be routed to) — the E3b1 world
    /// half of a custody op. Raised only AFTER `validate` returned `Post` and
    /// **before** the store is touched, so nothing has moved and nothing posted.
    #[error("custody refused: {0}")]
    Custody(String),
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

    // ---- E4a: holds, obligations, contracts ------------------------------
    /// The posted balance is fine but the money is **reserved**: `posted − held`
    /// would breach the account's credit policy (invariant 23). Distinct from
    /// [`EconomyError::CreditViolation`], which is the posted-balance breach, so a
    /// payer can tell "you do not have it" from "you have it, and it is spoken for".
    #[error("account {0} has insufficient available funds")]
    InsufficientAvailable(AccountId),
    /// A referenced hold does not exist.
    #[error("no such hold: {0}")]
    NoSuchHold(HoldId),
    /// A `Place` asked to reserve a non-positive amount.
    #[error("a hold amount must be positive")]
    HoldAmountNotPositive,
    /// A capture/release/expiry named a hold that is no longer `Held` (invariant 24).
    #[error("hold {0} is not held")]
    HoldNotHeld(HoldId),
    /// `Place` on an account whose `credit.floor` is `None` (invariant 23):
    /// `CreditPolicy::permits` is vacuous there, so a reservation on an
    /// unbounded-credit account reserves nothing at all.
    #[error("account {0} has unbounded credit and cannot carry a hold")]
    HoldOnUnboundedAccount(AccountId),
    /// A capture asked for more than the hold reserves, or for a non-positive
    /// amount (invariant 26).
    #[error("hold {0} cannot cover that capture")]
    CaptureExceedsHold(HoldId),
    /// `Expire` on a hold whose deadline has not passed on its own clock — or which
    /// has no deadline at all (invariant 24).
    #[error("hold {0} has not expired")]
    HoldNotExpired(HoldId),
    /// `Expire` outside the sweep's `Internal {{ kind: "hold.expire" }}` cause
    /// (invariant 24): expiry is the scheduler's act, not a player verb.
    #[error("a hold expiry must be posted by the economy sweep")]
    HoldExpireNotSweep,
    /// Two hold ops on one hold in one plan (invariant 26) — they would race each
    /// other inside one bundle.
    #[error("more than one hold op on hold {0}")]
    DuplicateHoldOp(HoldId),
    /// A `Capture` this plan's money lines do not back: the gross debit on the
    /// hold's account is smaller than what the plan captures there (E4a verify
    /// fold, finding 1). A capture SPENDS reserved value, so a plan that captures
    /// without debiting — or that captures on one account and debits another —
    /// would consume the reservation while the money went somewhere the hold never
    /// promised.
    #[error("hold {0}'s capture is not backed by a debit on its own account")]
    CaptureUnbacked(HoldId),
    /// A `Place` minted an id the snapshot already knows (E4a verify fold, finding
    /// 4): the caller's allocator floor is stale, and applying it would overwrite a
    /// live reservation with a brand-new one. Never silently re-mint.
    #[error("hold id {0} is already in use")]
    HoldIdReused(HoldId),
    /// An expected-hold-revision guard did not match current reservation state.
    #[error("stale plan: hold {hold} expected revision {expected}, found {found:?}")]
    StaleHold {
        /// The hold.
        hold: HoldId,
        /// The expected revision.
        expected: u64,
        /// The current revision, if the hold exists.
        found: Option<u64>,
    },
    /// A withdrawal named an object a live contract's `DepositInto` condition holds
    /// in that vault (§3.5): only that contract's settlement or refund may take it
    /// out, or a seller could sell the deposit out from under the buyer.
    #[error("object {0} is held by a live contract")]
    CustodyLocked(ObjId),
    /// A referenced contract does not exist.
    #[error("no such contract: {0}")]
    NoSuchContract(ContractId),
    /// The contract is not in a state that admits this operation.
    #[error("contract {0} is not in a state that allows that")]
    ContractState(ContractId),
    /// An acceptance no longer matches the contract's consent surface: something in
    /// the surface changed after `Offer` (D-E4-4).
    #[error("contract {0}'s consent surface has changed")]
    ConsentMismatch(ContractId),
    /// A referenced obligation does not exist.
    #[error("no such obligation: {0}")]
    NoSuchObligation(ObligationId),
}

impl EconomyError {
    /// Every account / title record / vault this refusal NAMES.
    ///
    /// The command layer's existence-hiding renderer (§4.5, D-E3-6) collapses a
    /// refusal to the matching "No such …" text when **any** of these is a resource
    /// the actor may not view — not merely when the COMMAND's own subject is. An
    /// error naming a different resource than the verb's subject (`@econ/pay
    /// <mine>=<someone else's>` answering about the *destination*) is exactly the
    /// cross-resource disclosure this closes.
    ///
    /// Custody refusals ([`EconomyError::NoCustodyStanding`],
    /// [`EconomyError::NotInVault`], [`EconomyError::BadDestination`],
    /// [`EconomyError::BackingNotDepositable`], [`EconomyError::DuplicateCustodyOp`])
    /// name a **world object**, not a title record — an object the actor is looking
    /// at in the room discloses nothing, and the vault they named is the subject the
    /// renderer already hides behind — so they are deliberately not listed here.
    #[must_use]
    pub fn named_resources(&self) -> Vec<NamedResource> {
        match self {
            EconomyError::NoSuchAccount(a)
            | EconomyError::InactiveAccount(a)
            | EconomyError::CreditViolation(a) => vec![NamedResource::Account(*a)],
            EconomyError::Stale { account, .. } => vec![NamedResource::Account(*account)],
            EconomyError::NoSuchAsset(o)
            | EconomyError::AssetExists(o)
            | EconomyError::AssetNotActive(o)
            | EconomyError::AssetFrozen(o)
            | EconomyError::Encumbered(o)
            | EconomyError::NoSuchEncumbrance(o)
            | EconomyError::AssignmentToOwner(o)
            | EconomyError::DuplicateAssetOp(o) => vec![NamedResource::Asset(*o)],
            EconomyError::StaleAsset { object, .. } => vec![NamedResource::Asset(*object)],
            EconomyError::NoSuchVault(v)
            | EconomyError::VaultNotActive(v)
            | EconomyError::BackingInvalid(v) => vec![NamedResource::Vault(*v)],
            EconomyError::InsufficientAvailable(a)
            | EconomyError::HoldOnUnboundedAccount(a)
            | EconomyError::MintCapExceeded(a) => {
                vec![NamedResource::Account(*a)]
            }
            EconomyError::NoSuchHold(h)
            | EconomyError::HoldNotHeld(h)
            | EconomyError::CaptureExceedsHold(h)
            | EconomyError::HoldNotExpired(h)
            | EconomyError::DuplicateHoldOp(h)
            | EconomyError::CaptureUnbacked(h)
            | EconomyError::HoldIdReused(h) => vec![NamedResource::Hold(*h)],
            EconomyError::StaleHold { hold, .. } => vec![NamedResource::Hold(*hold)],
            EconomyError::NoSuchContract(c)
            | EconomyError::ContractState(c)
            | EconomyError::ConsentMismatch(c) => vec![NamedResource::Contract(*c)],
            EconomyError::NoSuchObligation(o) | EconomyError::ObligationTerminal(o) => {
                vec![NamedResource::Obligation(*o)]
            }
            _ => Vec::new(),
        }
    }

    /// Whether a refusal that names **no** resource is nonetheless a policy answer
    /// *about the resources the caller named* — so the renderer must also hide
    /// behind those (the account `@asset/maintenance` pointed at, the account
    /// `@econ/pay` paid into). A cap or a shape complaint
    /// ([`EconomyError::TooLarge`], [`EconomyError::TooFewLines`]) is about the
    /// PLAN, not about any resource, and must keep rendering its own text — hiding
    /// it would tell a payer their own operational error is "no such account".
    #[must_use]
    pub fn concerns_named_subjects(&self) -> bool {
        matches!(
            self,
            EconomyError::Unauthorized | EconomyError::ScopeOwnerMismatch
        )
    }
}
