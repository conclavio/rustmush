//! The `mush-economy` records (E2a) — instruments, accounts, and the immutable
//! double-entry journal transaction (doc 08 §6/§7). Plain serializable structs; all
//! persistence + boot is E2b.
//!
//! Balances and line deltas are [`Amount`] (checked minor units), not raw `i64`, so
//! the type system forbids unchecked money arithmetic everywhere (R-R3). `Amount`
//! serializes transparently as the `i64` the design specifies, so the persisted doc
//! shape still matches doc 08.

use mush_core::ObjId;
use mush_org::{ApprovalPolicy, OrgScopeId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::ids::{AccountId, ApprovalRequestId, InstrumentId, TransactionId, VaultId};

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
    /// Per-account high-value escalation (E3a, D-E3-2): a net debit exceeding
    /// `threshold` requires an [`ApprovalRequest`] bound to the plan hash. `None`
    /// (the default, so every E2 account doc still deserializes) means no escalation.
    #[serde(default)]
    pub approval: Option<ApprovalRule>,
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
    /// The title/assignment/encumbrance/maintenance changes this transaction carried
    /// (E3a). **Journal-authority**: an `asset:{obj}` doc is a projection replay
    /// rebuilds, so a companion-doc drop can never desync title from the money that
    /// bought it (D-E3-1). `#[serde(default)]` so every E2 entry still deserializes.
    #[serde(default)]
    pub asset_effects: Vec<AssetEffect>,
    /// The custody movements this transaction carried (E3a). Audit-only: the world
    /// move itself is performed by the E3b module through the custody provider.
    #[serde(default)]
    pub custody_effects: Vec<CustodyEffect>,
    /// The approval request this post consumed, if a per-account [`ApprovalRule`]
    /// fired (E3a).
    #[serde(default)]
    pub consumed_approval: Option<ApprovalRequestId>,
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

// ---- assets (E3a) ---------------------------------------------------------

/// A titled economic asset — the four-axis separation of doc 08 §9.1 (title /
/// custody / operation / maintenance), keyed by the world object it describes
/// (`ObjId` is never recycled, so the asset needs no id of its own).
///
/// Every field above the latch is **journal authority**: an [`AssetEffect`] carries
/// the full post-state of exactly these, so boot replay rebuilds the asset from the
/// journal alone and a dropped `asset:{obj}` doc can never lose a paid-for title
/// (D-E3-1). The `frozen` latch below is **doc authority** — never inside an effect,
/// ignored by replay and by drift comparison — exactly the `Account.status` split
/// (E2b boot compares `posted_balance`, never `status`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicAsset {
    /// The world object this asset titles.
    pub object: ObjId,
    /// Who holds title.
    pub title_owner: PrincipalId,
    /// Org compartment for an org-owned asset — the policy anchor, exactly as
    /// `Account.scope`. Invariant 18: `Some(scope)` ⇒
    /// `title_owner == PrincipalId::Organization(scope.org)`.
    pub scope: Option<OrgScopeId>,
    /// Who currently has it assigned (a relationship with the title owner, cleared
    /// on transfer — invariant 13).
    pub assignment: Option<AssetAssignment>,
    /// The account upkeep debits (invariant 14); cleared on transfer.
    pub maintenance_account: Option<AccountId>,
    /// The opaque pre-registry catalog reference (the §9.4 `CatalogId` precursor).
    pub catalog: Option<CatalogRef>,
    /// Claims against the asset (≤ [`crate::MAX_ENCUMBRANCES_PER_ASSET`]).
    pub encumbrances: Vec<Encumbrance>,
    /// The transaction that first titled / last transferred it while money moved
    /// (invariant 22 — the §9.4 provenance seed).
    pub acquisition: Option<AcquisitionRecord>,
    /// Journaled lifecycle.
    pub lifecycle: AssetLifecycle,
    /// The journal sequence of the last effect folded into this projection.
    pub last_journal_seq: u64,
    /// Staleness stamp (bumped once per effect).
    pub revision: u64,
    /// **Doc-authority latch** — never carried in an effect. A frozen asset takes no
    /// asset op (the destroy/recovery backstop, E3b).
    #[serde(default)]
    pub frozen: bool,
    /// Why it was frozen.
    #[serde(default)]
    pub frozen_reason: Option<SmolStr>,
}

/// Who has an asset assigned, and for what — the assignment axis, distinct from
/// title, custody, and operation (D5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetAssignment {
    /// The assignee.
    pub assigned_to: PrincipalId,
    /// Free-text purpose (≤ [`crate::MAX_REASON_BYTES`]).
    pub purpose: SmolStr,
    /// Optional expiry (unix seconds; recorded, not enforced by the pure engine).
    pub valid_until: Option<i64>,
}

/// A claim against an asset. Serde-**tagged** so E4's `Lease`/`Collateral` (which
/// need a `ContractId`) can be added without changing the persisted shape of the two
/// E3 variants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Encumbrance {
    /// A priced claim by a creditor. **Staff-only to create in E3** (invariant 19):
    /// without a `ContractId` an unpriced lien is a hostage primitive.
    Lien {
        /// Who holds the claim.
        creditor: PrincipalId,
        /// How much.
        amount: Amount,
        /// In which instrument.
        instrument: InstrumentId,
    },
    /// A recorded dispute over title.
    Disputed {
        /// Who disputes.
        claimant: PrincipalId,
        /// Why (≤ [`crate::MAX_REASON_BYTES`]).
        reason: String,
    },
}

/// Journaled asset lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetLifecycle {
    /// Normal; the only state an asset op may touch.
    Active,
    /// Written off (destroyed/lost/scrapped). Terminal in E3.
    WrittenOff,
}

/// An opaque ruleset catalog reference — the pre-registry precursor of doc 08
/// §9.4's `CatalogId` (E6/E8 build the registry; `mush-economy` never interprets it).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CatalogRef {
    /// The owning ruleset.
    pub ruleset: SmolStr,
    /// The catalog code within that ruleset.
    pub code: SmolStr,
}

/// How an asset was acquired — stamped when a title op rides a transaction that also
/// moved money (invariant 22).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquisitionRecord {
    /// The transaction that paid for it.
    pub transaction: TransactionId,
    /// When (unix seconds).
    pub at: i64,
}

// ---- vaults (E3a) ---------------------------------------------------------

/// A custody container: a named, policy-bearing wrapper around a world backing
/// object (doc 08 §9.3). Custody is **not** title (D5) — depositing an object into a
/// vault moves no title, and titling an object grants no custody standing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vault {
    /// Identity (never reused).
    pub id: VaultId,
    /// Who owns the vault.
    pub owner: PrincipalId,
    /// Org compartment (invariant 18 applies exactly as for accounts and assets).
    pub scope: Option<OrgScopeId>,
    /// Display name.
    pub name: SmolStr,
    /// What physically holds the contents (one vault per backing).
    pub backing: CustodyRef,
    /// Doc-authority lifecycle.
    pub status: VaultStatus,
    /// Staleness stamp.
    pub revision: u64,
}

/// Vault lifecycle (doc authority, like `AccountStatus`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VaultStatus {
    /// Normal; the only state a custody op may touch.
    Active,
    /// Frozen by staff / recovery — no deposits or withdrawals.
    Frozen,
    /// Closed.
    Closed,
}

/// What backs a vault. `Ord` so it can key the one-vault-per-backing index.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CustodyRef {
    /// An ordinary container object (the built-in provider, E3b1).
    Container(ObjId),
    /// A `mush-sim` spatial interior (E3c).
    SpatialInterior(ObjId),
    /// A provider-registered backing with no single world object.
    Registered {
        /// The registering provider.
        provider: SmolStr,
        /// The provider's id for it.
        id: u64,
    },
}

impl CustodyRef {
    /// The world object backing this vault, if it has one. `Registered` backings
    /// have none — the pure engine therefore cannot name a `to`/`from` location for
    /// them and refuses their custody ops fail-closed (`BackingInvalid`).
    #[must_use]
    pub fn object(&self) -> Option<ObjId> {
        match self {
            CustodyRef::Container(o) | CustodyRef::SpatialInterior(o) => Some(*o),
            CustodyRef::Registered { .. } => None,
        }
    }
}

// ---- approvals (E3a) ------------------------------------------------------

/// Per-account high-value escalation (D-E3-2). A net debit **strictly exceeding**
/// `threshold` requires a satisfied [`ApprovalRequest`] bound to the plan hash.
///
/// [`ApprovalPolicy::OneAuthorizedActor`] is refused (it can never gate anything —
/// the one authorized actor is the poster); clear the rule instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRule {
    /// The escalation threshold, in the account's instrument.
    pub threshold: Amount,
    /// The policy that must be satisfied above it.
    pub policy: ApprovalPolicy,
}

/// A collected approval for one exact plan hash. There is deliberately **no
/// `Satisfied` status**: whether the recorded approvers satisfy the policy is
/// re-derived LIVE in [`crate::validate`] (an approver who lost the capability stops
/// counting — the `mush-org` delegation discipline).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Identity (never reused).
    pub id: ApprovalRequestId,
    /// The exact plan this approves. Approvals authorize an **intent, not a state**:
    /// `expected_*_revisions` are outside the hash.
    pub plan_hash: [u8; 32],
    /// The accounts whose rules fired.
    pub accounts: Vec<AccountId>,
    /// Who asked (never counts as an approver — four-eyes).
    pub requested_by: PrincipalId,
    /// The recorded approvers (≤ [`crate::MAX_APPROVERS_PER_REQUEST`]).
    pub approvals: Vec<Approval>,
    /// The full audit trail, approvals and denials alike (same cap).
    pub decisions: Vec<Decision>,
    /// When it was created (unix seconds).
    pub created_at: i64,
    /// When it stops counting (`created_at + APPROVAL_TTL_SECS`).
    pub expires_at: i64,
    /// Lifecycle.
    pub status: ApprovalStatus,
    /// Staleness stamp.
    pub revision: u64,
}

/// One recorded approval.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    /// Who approved.
    pub approver: ObjId,
    /// When (unix seconds).
    pub at: i64,
}

/// One recorded decision — the audit trail (a denial is kept, not just dropped).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Who decided.
    pub actor: ObjId,
    /// When (unix seconds).
    pub at: i64,
    /// What they decided.
    pub verdict: Verdict,
}

/// Approve or deny.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// In favour.
    Approved,
    /// Against.
    Denied,
}

/// Approval-request lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalStatus {
    /// Live — the only status [`crate::validate`] will count.
    Pending,
    /// Spent by a post (at most once).
    Consumed {
        /// The transaction that consumed it.
        transaction: TransactionId,
    },
    /// Withdrawn by the requester or staff.
    Cancelled,
    /// Aged out past `expires_at`.
    Expired,
}

// ---- journal effects (E3a) ------------------------------------------------

/// One asset change carried by a transaction. `post` is the **full journal-authority
/// post-state** of the asset after this effect — `frozen`/`frozen_reason` are always
/// `false`/`None` inside an effect and are ignored by replay and drift comparison.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetEffect {
    /// Which asset.
    pub object: ObjId,
    /// What kind of change (audit + display; the state is `post`).
    pub kind: AssetEffectKind,
    /// The complete asset after the change.
    pub post: EconomicAsset,
}

/// What an [`AssetEffect`] did.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetEffectKind {
    /// A title record was created.
    Created,
    /// Title moved to another principal.
    TitleTransferred,
    /// The asset was assigned.
    Assigned,
    /// The assignment was cleared.
    Unassigned,
    /// A claim was recorded.
    Encumbered,
    /// A claim was released.
    Released,
    /// The maintenance payer changed.
    MaintenanceSet,
    /// The asset was written off.
    WrittenOff,
}

/// One custody movement carried by a transaction — **audit only**: the world move is
/// performed by the E3b module through the custody provider, in the same terminal
/// bundle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustodyEffect {
    /// The object that moved.
    pub object: ObjId,
    /// The vault involved.
    pub vault: VaultId,
    /// Which way.
    pub direction: CustodyDirection,
    /// Where it came from, when known to the recorder.
    pub from: Option<ObjId>,
    /// Where it went.
    pub to: ObjId,
}

/// Which way a [`CustodyEffect`] moved.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CustodyDirection {
    /// Into the vault.
    Deposit,
    /// Out of the vault.
    Withdraw,
}

/// Convenience: an `ObjId` as an object principal (the common actor form).
#[must_use]
pub fn object_principal(obj: ObjId) -> PrincipalId {
    PrincipalId::Object(obj)
}
