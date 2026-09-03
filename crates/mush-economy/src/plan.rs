//! The economy plan alphabet + the validation outcome (E2a).
//!
//! A caller builds an [`EconomyPlan`] (what should move, and why); the pure engine
//! ([`crate::validate`]) checks it against one immutable snapshot and returns a
//! [`PlanResult`]. On [`PlanResult::Post`] the [`Prepared`] carries everything E2b
//! needs to build the immutable transaction and fold the projections — the module
//! only supplies the allocated ids, sequence, and post time; the apply is infallible
//! because validation already proved it against the pre-state.

use std::collections::BTreeMap;

use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::ids::{AccountId, InstrumentId, TransactionId};
use crate::record::{EconomicCause, EconomicVisibility, IdempotencyKey, JournalLine};

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

/// A validated-then-applied economic plan. The `hold_ops`/`asset_ops`/
/// `contract_ops`/`world_ops` vectors of doc 08 are forward-declared as **empty in
/// E2** (holds are E4, assets E3) and are simply absent from this struct until then;
/// their arrival does not change the persisted transaction shape.
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
    /// Optimistic staleness expectations on economic data.
    pub expected_revisions: Vec<AccountRevision>,
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
}
