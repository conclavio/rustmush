//! The `mush-economy` records (E2a) — instruments, accounts, and the immutable
//! double-entry journal transaction (doc 08 §6/§7). Plain serializable structs; all
//! persistence + boot is E2b.
//!
//! Balances and line deltas are [`Amount`] (checked minor units), not raw `i64`, so
//! the type system forbids unchecked money arithmetic everywhere (R-R3). `Amount`
//! serializes transparently as the `i64` the design specifies, so the persisted doc
//! shape still matches doc 08.

use mush_core::ObjId;
use mush_org::{OrgScopeId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::ids::{AccountId, InstrumentId, TransactionId};

// ---- instruments ----------------------------------------------------------

/// A registered instrument (currency/commodity/…). Authoritative values are signed
/// `i64` minor units at `scale` decimal places; there is **no implicit exchange**
/// between instruments — an exchange is a multi-instrument transaction that balances
/// each instrument independently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentDef {
    /// The canonical instrument code (its id).
    pub id: InstrumentId,
    /// The ruleset that owns this instrument.
    pub ruleset: SmolStr,
    /// Human code, same as `id` unless a ruleset aliases it.
    pub code: SmolStr,
    /// What kind of value this is.
    pub kind: InstrumentKind,
    /// Decimal minor-unit scale (0 = indivisible).
    pub scale: u8,
    /// Whether ordinary transfers between non-system accounts are allowed.
    pub transferable: bool,
    /// The default negative-balance rule for accounts of this instrument.
    pub negative_policy: NegativePolicy,
    /// Lifecycle.
    pub status: InstrumentStatus,
    /// Staleness stamp.
    pub revision: u64,
}

/// What an instrument represents.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstrumentKind {
    /// A double-entry currency (nuyen, C-bills).
    Currency,
    /// A fungible commodity (fuel, ammo, steel).
    Commodity,
    /// A requisition/allowance instrument.
    Requisition,
    /// A non-transferable entitlement.
    Entitlement,
}

/// Instrument lifecycle. A non-active instrument cannot be posted against.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstrumentStatus {
    /// Normal.
    Active,
    /// Suspended (no new postings).
    Suspended,
    /// Retired (id never reused).
    Retired,
}

/// The instrument-level default for whether accounts may hold a negative balance.
/// The per-account [`CreditPolicy`] is the actually-enforced gate; this records the
/// instrument's stance (a currency normally `SystemOnly`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NegativePolicy {
    /// No account may go negative.
    Forbid,
    /// Only registered system source/sink accounts may go negative.
    SystemOnly,
    /// Any account with a permitting credit policy may go negative.
    Allow,
}

// ---- accounts -------------------------------------------------------------

/// One account: a balance in exactly one instrument, owned by a principal, optionally
/// bound to an org scope. `posted_balance` and `held_amount` are materialized
/// projections re-derivable from the journal — never a competing truth (E2b boot
/// re-derives and freezes on drift). Available funds = `posted_balance − held_amount`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Identity (never reused).
    pub id: AccountId,
    /// Who owns it.
    pub owner: PrincipalId,
    /// The single instrument this account holds.
    pub instrument: InstrumentId,
    /// The org scope this account binds to (the divisional-wallet vertical is E3;
    /// E2 records the binding and authorizes against it).
    pub scope: Option<OrgScopeId>,
    /// Display name.
    pub name: SmolStr,
    /// Lifecycle.
    pub status: AccountStatus,
    /// Materialized posted balance (checked, journal-derived).
    pub posted_balance: Amount,
    /// Materialized sum of active holds (always `Amount::ZERO` in E2 — holds are E4).
    pub held_amount: Amount,
    /// Whether/how far this account may go negative.
    pub credit: CreditPolicy,
    /// The last journal sequence folded into this projection — the tail-loss marker
    /// (E2b boot freezes if this exceeds the journal's restored max seq).
    pub last_journal_seq: u64,
    /// Staleness stamp (optimistic-concurrency for economic data).
    pub revision: u64,
}

impl Account {
    /// Available funds = posted − held (checked). `None` only on an impossible
    /// projection (held exceeding posted past `i64`), which validation prevents.
    #[must_use]
    pub fn available(&self) -> Option<Amount> {
        self.posted_balance.checked_sub(self.held_amount)
    }
}

/// Account lifecycle. Only [`AccountStatus::Active`] may be posted against.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountStatus {
    /// Normal.
    Active,
    /// Frozen by boot drift or staff action — no postings until cleared.
    Frozen,
    /// Draining toward closure.
    Closing,
    /// Closed (zero balance, no holds/obligations).
    Closed,
}

/// Whether an account may hold a negative posted balance, and how far. A normal
/// account forbids it; a registered system source/sink permits it unbounded (the one
/// unbounded-balance locus — R-R4). A finite `floor` bounds an overdraft.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditPolicy {
    /// Whether the posted balance may go below zero at all.
    pub allow_negative: bool,
    /// The most-negative allowed balance (`None` = unbounded, system source/sink only).
    pub floor: Option<Amount>,
}

impl CreditPolicy {
    /// The default: no negative balance.
    #[must_use]
    pub const fn forbid() -> CreditPolicy {
        CreditPolicy { allow_negative: false, floor: Some(Amount::ZERO) }
    }
    /// An unbounded-negative policy (system source/sink only).
    #[must_use]
    pub const fn unbounded() -> CreditPolicy {
        CreditPolicy { allow_negative: true, floor: None }
    }
    /// Whether `balance` is within this policy.
    #[must_use]
    pub fn permits(&self, balance: Amount) -> bool {
        if !balance.is_negative() {
            return true;
        }
        if !self.allow_negative {
            return false;
        }
        match self.floor {
            None => true,
            Some(floor) => balance >= floor,
        }
    }
}

// ---- the journal transaction ---------------------------------------------

/// One immutable double-entry transaction — the journal payload. Posted once, never
/// mutated; a correction is a NEW transaction, and a reversal a NEW transaction with
/// inverse lines and an explicit `reverses` back-reference (invariant 7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicTransaction {
    /// Stable id (never reused).
    pub id: TransactionId,
    /// Monotonic journal sequence.
    pub journal_seq: u64,
    /// The idempotency key that authorized this exact post.
    pub idempotency: IdempotencyKey,
    /// Why it happened.
    pub cause: EconomicCause,
    /// Who posted it.
    pub actor: PrincipalId,
    /// Wall-clock post time (unix seconds).
    pub posted_at: i64,
    /// Free-form human description.
    pub description: String,
    /// The balanced lines (≥2, per-instrument zero-sum).
    pub lines: Vec<JournalLine>,
    /// The transaction this one reverses, if any.
    pub reverses: Option<TransactionId>,
    /// Recorded visibility (NOT enforced until E4 — R-D2).
    pub visibility: EconomicVisibility,
    /// The canonical plan hash that produced this post (idempotency + approval link).
    pub plan_hash: [u8; 32],
}

/// One line of a double-entry transaction: a signed delta to one account, in that
/// account's instrument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalLine {
    /// The account moved.
    pub account: AccountId,
    /// The instrument (must equal the account's).
    pub instrument: InstrumentId,
    /// The signed delta (checked minor units).
    pub delta: Amount,
    /// Per-line memo.
    pub memo: SmolStr,
}

/// Why a transaction was posted — keeps the journal independent of the crates that
/// cause economic activity (scenes, canon, sim, jobs) via an opaque [`CausalRef`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomicCause {
    /// A player command with a dispatch serial.
    Command {
        /// The command dispatch serial.
        serial: u64,
    },
    /// An internal economy record (contract/obligation/work-order) without widening
    /// this enum per record kind.
    Internal {
        /// The internal record kind.
        kind: SmolStr,
        /// The internal record id.
        id: u64,
    },
    /// An external correlated reference (validated by its provider).
    External(CausalRef),
    /// A staff action with a reason.
    Staff {
        /// The staff reason.
        reason: String,
    },
    /// A ruleset-attributed cause.
    Ruleset {
        /// The ruleset namespace.
        namespace: SmolStr,
        /// A ruleset correlation token.
        correlation: SmolStr,
    },
}

/// An opaque external reference correlating a transaction with a scene/canon/sim/job
/// without `mush-economy` depending on those crates. The provider validates it; the
/// journal preserves the attribution even if the external record is later archived.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalRef {
    /// The provider name (`"scene"`, `"canon"`, `"sim"`, `"jobs"`).
    pub provider: SmolStr,
    /// The referenced record kind.
    pub kind: SmolStr,
    /// The referenced record id.
    pub id: SmolStr,
    /// The referenced record revision, if versioned.
    pub revision: Option<u64>,
}

/// Recorded transaction visibility. **Recorded, not enforced** in E2 — all E2 reads
/// are staff-gated and the disclosure filter lands in E4 (R-D2).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomicVisibility {
    /// Only the transacting parties (and staff).
    #[default]
    Parties,
    /// Staff only.
    Staff,
    /// Anyone.
    Public,
}

// ---- idempotency ----------------------------------------------------------

/// A caller-supplied idempotency key: repeating it with the SAME plan returns the
/// original result; reusing it with a DIFFERENT plan is always an error + audit
/// (never silently re-posted).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey {
    /// The key namespace (`"transfer"`, `"econ.reverse"`, …).
    pub namespace: SmolStr,
    /// The key within the namespace.
    pub key: SmolStr,
}

impl IdempotencyKey {
    /// Construct a key.
    #[must_use]
    pub fn new(namespace: impl Into<SmolStr>, key: impl Into<SmolStr>) -> IdempotencyKey {
        IdempotencyKey { namespace: namespace.into(), key: key.into() }
    }
    /// The deterministic reversal key for a target transaction (R-B4): namespace
    /// `econ.reverse`, key = the target id. A second reverse of the same tx therefore
    /// resolves to the same key → `AlreadyPosted`.
    #[must_use]
    pub fn reversal_of(target: TransactionId) -> IdempotencyKey {
        IdempotencyKey::new(SmolStr::new_static("econ.reverse"), SmolStr::new(target.0.to_string()))
    }
}

/// The persisted idempotency record — a cache the E2b boot re-derives from the
/// journal (never boot authority). Maps a key to the transaction it produced + the
/// plan hash it was authorized with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdempotencyRecord {
    /// The key.
    pub idempotency: IdempotencyKey,
    /// The plan hash it was posted with (a re-use with a different hash is refused).
    pub plan_hash: [u8; 32],
    /// The transaction it produced.
    pub transaction: TransactionId,
    /// When (unix seconds).
    pub posted_at: i64,
}

/// Convenience: an `ObjId` as an object principal (the common actor form).
#[must_use]
pub fn object_principal(obj: ObjId) -> PrincipalId {
    PrincipalId::Object(obj)
}
