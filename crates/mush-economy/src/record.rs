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
use crate::clock::EconomicDeadline;
use crate::ids::{
    AccountId, ApprovalRequestId, ContractId, EscalationId, HoldId, InstrumentId, ObligationId,
    TransactionId, VaultId,
};

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
    /// How a player-facing render writes an amount of this instrument (E6a step 1,
    /// E2-C16 / 08g §12.1 item 4): `¥1000`, `12.50€`. `None` (the default, so every
    /// E2 instrument doc still deserializes) renders the bare decimal.
    #[serde(default)]
    pub display: Option<InstrumentDisplay>,
    /// Staleness stamp.
    pub revision: u64,
}

/// The largest symbol an [`InstrumentDisplay`] carries (the crate's bounded-string
/// discipline; `¥`, `€`, `nuyen` and `¢` all fit).
pub const MAX_SYMBOL_BYTES: usize = 8;

/// Where an instrument's symbol goes relative to the digits.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolPosition {
    /// `¥1000` — the sign, when any, comes before the symbol: `-¥5`.
    Prefix,
    /// `1000¥`.
    Suffix,
}

/// How an instrument renders (E6a step 1, D-E6-1's `¥`): a bounded symbol and its
/// position. Loading re-checks the bound — an empty symbol, one over
/// [`MAX_SYMBOL_BYTES`], or one carrying whitespace or a control character refuses
/// to deserialize — so a hand-written document cannot put a paragraph in front of
/// every balance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "InstrumentDisplayWire")]
pub struct InstrumentDisplay {
    /// The symbol (1..=[`MAX_SYMBOL_BYTES`] bytes, no whitespace).
    pub symbol: SmolStr,
    /// Before or after the digits.
    pub position: SymbolPosition,
}

#[derive(Deserialize)]
struct InstrumentDisplayWire {
    symbol: SmolStr,
    position: SymbolPosition,
}

impl InstrumentDisplay {
    /// Build a display, refusing an empty, oversized, or whitespace-bearing symbol.
    #[must_use]
    pub fn new(symbol: &str, position: SymbolPosition) -> Option<InstrumentDisplay> {
        if symbol.is_empty()
            || symbol.len() > MAX_SYMBOL_BYTES
            || symbol.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return None;
        }
        Some(InstrumentDisplay {
            symbol: SmolStr::new(symbol),
            position,
        })
    }
}

impl TryFrom<InstrumentDisplayWire> for InstrumentDisplay {
    type Error = String;

    fn try_from(w: InstrumentDisplayWire) -> Result<InstrumentDisplay, String> {
        InstrumentDisplay::new(w.symbol.as_str(), w.position).ok_or_else(|| {
            format!("an instrument symbol is 1..={MAX_SYMBOL_BYTES} bytes with no whitespace")
        })
    }
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
    /// Materialized sum of the account's `Held` holds (E4a). A **projection**, like
    /// `posted_balance`: `apply` maintains it from the transaction's hold effects and
    /// E4b1's boot re-derives it as Σ of the account's `Held` holds.
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
    /// The journal sequence of the last **hold effect** touching this account (E4a,
    /// review point D11) — the drift anchor for `held_amount`, deliberately distinct
    /// from `last_journal_seq`, which moves only for money-line deltas. A
    /// `Place`/`Release`/`Expire` is an effect-only entry, so without its own anchor
    /// a `held_amount` projection could never be compared at the sequence it was
    /// derived at. `#[serde(default)]` so every E2/E3 account doc still deserializes.
    #[serde(default)]
    pub last_hold_seq: u64,
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
        CreditPolicy {
            allow_negative: false,
            floor: Some(Amount::ZERO),
        }
    }
    /// An unbounded-negative policy (system source/sink only).
    #[must_use]
    pub const fn unbounded() -> CreditPolicy {
        CreditPolicy {
            allow_negative: true,
            floor: None,
        }
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
    /// The hold placements/captures/releases/expiries this transaction carried
    /// (E4a). **Journal authority** exactly like [`EconomicTransaction::asset_effects`]
    /// (D-E4-1): each effect carries the full post-state of the hold, so replay
    /// rebuilds every reservation — and `Account.held_amount` — from the journal
    /// alone, and a dropped `hold:{id}` projection can never strand escrow.
    /// `#[serde(default)]` so every E2/E3 entry still deserializes.
    #[serde(default)]
    pub hold_effects: Vec<HoldEffect>,
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
        /// The command's dispatch serial: the world's **per-boot** monotonic
        /// dispatch correlation (`World::next_correlation_id`, the same counter the
        /// NPC path draws), so two commands in one boot never share one (H4,
        /// E2-D24; D-H-10). It restarts at boot — pair it with `posted_at` to order
        /// across boots. Pre-H4 entries carry `0`.
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
    External(ExternalRef),
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

/// An opaque external reference correlating a transaction — or a contract's `Fact`
/// condition — with a scene/canon/sim/job, without `mush-economy` depending on those
/// crates. The provider validates it; the journal preserves the attribution even if
/// the external record is later archived.
///
/// **E4a rename (doc 09 conformance, §12).** This was E2's `CausalRef`; doc 09 C6
/// uses that name for the *kernel's* causal reference (`root: CommitToken`, `parent`,
/// `ordinal`), which this is not. The serde field shape is **byte-identical** —
/// `provider`/`kind`/`id`/`revision` — so every stored journal entry still
/// deserializes, and [`CausalRef`] survives as an alias for one release.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ExternalRef {
    /// The provider name (`"scene"`, `"canon"`, `"sim"`, `"jobs"`).
    pub provider: SmolStr,
    /// The referenced record kind.
    pub kind: SmolStr,
    /// The referenced record id.
    pub id: SmolStr,
    /// The referenced record revision, if versioned.
    pub revision: Option<u64>,
}

/// The E2 name of [`ExternalRef`], kept for one release so a caller written against
/// E2/E3 keeps compiling (§12).
pub type CausalRef = ExternalRef;

/// The largest audience list one contract may name (doc 10 T6, E4b2).
pub const MAX_AUDIENCE: usize = 32;

/// Recorded transaction visibility. **Recorded, not enforced** in E2 — all E2 reads
/// are staff-gated and the disclosure filter lands in E4 (R-D2).
///
/// **E4b2 adds [`EconomicVisibility::Audience`]** (doc 10 §15 / T6): a declared
/// operation is contract-shaped but not necessarily world-visible, and the class
/// that matters first is "visible to the parties *and* to everyone entitled to
/// defend" — an audience list, not a public broadcast. The three shipped variants
/// keep their serde bytes exactly (a unit variant still serializes as `"Parties"`),
/// so every stored journal entry, plan hash and E2/E3/E4a document still
/// round-trips, and both digest pins stay green.
///
/// The type is no longer `Copy`: an audience carries a list. Everywhere the E2/E3
/// code copied it, it now clones — a per-post allocation on a value that is written
/// into the journal entry anyway.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomicVisibility {
    /// Only the transacting parties (and staff).
    #[default]
    Parties,
    /// Staff only.
    Staff,
    /// Anyone.
    Public,
    /// The parties **and** a named audience (doc 10 T6's "eligible defenders" /
    /// "participating organizations" classes). Canonicalized by principal key
    /// inside [`crate::consent_hash`], so two orderings of one audience are one
    /// consent surface.
    Audience {
        /// Who else may see it (≤ [`MAX_AUDIENCE`]).
        members: Vec<PrincipalId>,
        /// Whether the audience's **notice** names the issuer (E4b2 verify fold, A7).
        ///
        /// Default `false`, and it lives inside the consent surface rather than
        /// beside it because it is a disclosure the parties agree to: doc 10 T6's
        /// class is "notice of the window, not the plan", and who is running an
        /// operation is exactly the thing a defender learning about it must not get
        /// for free. An issuer who *wants* to be named says so, before anyone accepts.
        #[serde(default)]
        announce_issuer: bool,
    },
}

impl EconomicVisibility {
    /// The named audience, or an empty slice.
    #[must_use]
    pub fn members(&self) -> &[PrincipalId] {
        match self {
            EconomicVisibility::Audience { members, .. } => members,
            _ => &[],
        }
    }
    /// Whether an audience notice names the issuer (A7). Always `false` outside an
    /// [`EconomicVisibility::Audience`], which has no notice render.
    #[must_use]
    pub fn announce_issuer(&self) -> bool {
        matches!(
            self,
            EconomicVisibility::Audience {
                announce_issuer: true,
                ..
            }
        )
    }
    /// Whether this visibility is the one E4c's canon bridge may publish (D-E4-12:
    /// only a `Public` contract, whose every party accepted that disclosure, ever
    /// reaches canon — an audience list is emphatically **not** public).
    #[must_use]
    pub fn is_public(&self) -> bool {
        matches!(self, EconomicVisibility::Public)
    }
    /// A short label for a listing line.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            EconomicVisibility::Parties => "parties",
            EconomicVisibility::Staff => "staff",
            EconomicVisibility::Public => "public",
            EconomicVisibility::Audience { .. } => "audience",
        }
    }
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
        IdempotencyKey {
            namespace: namespace.into(),
            key: key.into(),
        }
    }
    /// The deterministic reversal key for a target transaction (R-B4): namespace
    /// `econ.reverse`, key = the target id. A second reverse of the same tx therefore
    /// resolves to the same key → `AlreadyPosted`.
    #[must_use]
    pub fn reversal_of(target: TransactionId) -> IdempotencyKey {
        IdempotencyKey::new(
            SmolStr::new_static("econ.reverse"),
            SmolStr::new(target.0.to_string()),
        )
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
#[serde(try_from = "EconomicAssetWire")]
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
    /// The **typed theft report** (E6b, D-E6-18, 08e answer 14).
    ///
    /// Journal authority like every field above the latch: it is carried in the
    /// [`AssetEffect`] post-state, so replay rebuilds it. It sits *beside*
    /// [`Encumbrance::Disputed`] rather than inside it, because insurance and
    /// salvage consume the plain `Disputed` claim (08f §7 item 5) and adding a
    /// visibility field to the claim would have made every consumer read it.
    #[serde(default)]
    pub stolen: Option<StolenReport>,
    /// **Doc-authority latch** — never carried in an effect. A frozen asset takes no
    /// asset op (the destroy/recovery backstop, E3b).
    #[serde(default)]
    pub frozen: bool,
    /// Why it was frozen.
    #[serde(default)]
    pub frozen_reason: Option<SmolStr>,
}

/// The persisted wire of [`EconomicAsset`] (E6b independent verify fold, D8).
///
/// The record was raw `Deserialize`, so **every bound the write path enforces was
/// enforced on the write path only**: `MAX_REASON_BYTES` on a theft report's reason
/// and on a `Disputed` claim's, and `MAX_ENCUMBRANCES_PER_ASSET` on the claim list.
/// A document edited on disk, restored from an older build, or written by a future
/// schema could therefore carry an unbounded reason straight into `@asset/show`,
/// every notice over it and the `AssetEffect` post-state the journal then replays.
/// The cut's rule is that every persisted type is a `TryFrom` wire carrying the write
/// path's own caps; this is that type for assets.
///
/// A document that fails these bounds does not load, and boot says so and continues:
/// every field above the latch is journal authority, so the projection is rebuilt
/// from the journal and comes back **frozen** with reason `doc-lost` — the same
/// treatment a vanished document already gets, and the reason it is safe to be
/// non-fatal.
#[derive(Deserialize)]
struct EconomicAssetWire {
    object: ObjId,
    title_owner: PrincipalId,
    #[serde(default)]
    scope: Option<OrgScopeId>,
    #[serde(default)]
    assignment: Option<AssetAssignment>,
    #[serde(default)]
    maintenance_account: Option<AccountId>,
    #[serde(default)]
    catalog: Option<CatalogRef>,
    #[serde(default)]
    encumbrances: Vec<Encumbrance>,
    #[serde(default)]
    acquisition: Option<AcquisitionRecord>,
    lifecycle: AssetLifecycle,
    #[serde(default)]
    last_journal_seq: u64,
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    stolen: Option<StolenReport>,
    #[serde(default)]
    frozen: bool,
    #[serde(default)]
    frozen_reason: Option<SmolStr>,
}

impl TryFrom<EconomicAssetWire> for EconomicAsset {
    type Error = &'static str;

    fn try_from(w: EconomicAssetWire) -> Result<EconomicAsset, &'static str> {
        if w.encumbrances.len() > crate::MAX_ENCUMBRANCES_PER_ASSET {
            return Err("an asset carries more claims than the cap allows");
        }
        for e in &w.encumbrances {
            if let Encumbrance::Disputed { reason, .. } = e
                && reason.len() > crate::MAX_REASON_BYTES
            {
                return Err("a disputed claim's reason is too long");
            }
        }
        if w.stolen
            .as_ref()
            .is_some_and(|r| r.reason.len() > crate::MAX_REASON_BYTES)
        {
            return Err("a theft report's reason is too long");
        }
        if w.assignment
            .as_ref()
            .is_some_and(|a| a.purpose.len() > crate::MAX_REASON_BYTES)
        {
            return Err("an assignment purpose is too long");
        }
        Ok(EconomicAsset {
            object: w.object,
            title_owner: w.title_owner,
            scope: w.scope,
            assignment: w.assignment,
            maintenance_account: w.maintenance_account,
            catalog: w.catalog,
            encumbrances: w.encumbrances,
            acquisition: w.acquisition,
            lifecycle: w.lifecycle,
            last_journal_seq: w.last_journal_seq,
            revision: w.revision,
            stolen: w.stolen,
            frozen: w.frozen,
            frozen_reason: w.frozen_reason,
        })
    }
}

// ---- E6b: B2 — the typed theft report -------------------------------------

/// Who may be told an asset was reported stolen (08e answer 14).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StolenVisibility {
    /// Anyone who can see the asset at all.
    Public,
    /// Only someone with view standing over the asset (`may_view_asset`).
    Restricted,
    /// Only the reporter and staff — invisible to the thief even when they hold
    /// the object, which is the whole point of a covert claim.
    Covert,
}

impl StolenVisibility {
    /// Parse the staff/player-typed word.
    #[must_use]
    pub fn parse(s: &str) -> Option<StolenVisibility> {
        match s.trim().to_ascii_lowercase().as_str() {
            "public" => Some(StolenVisibility::Public),
            "restricted" | "restrict" => Some(StolenVisibility::Restricted),
            "covert" | "secret" => Some(StolenVisibility::Covert),
            _ => None,
        }
    }
    /// The staff-facing label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            StolenVisibility::Public => "public",
            StolenVisibility::Restricted => "restricted",
            StolenVisibility::Covert => "covert",
        }
    }
}

/// A recorded theft report — bounded, journal-authority, and **evidence, never
/// guilt** (08e answers 13 and 14): a later purchase generates an
/// [`AcquisitionRecord`] and does not clear this.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StolenReport {
    /// Who filed it (the title owner, an `asset.insure` insurer, or staff).
    pub by: PrincipalId,
    /// Who may be told.
    pub visibility: StolenVisibility,
    /// Why (≤ [`crate::MAX_REASON_BYTES`]).
    pub reason: String,
    /// When (wall clock, unix seconds).
    pub at: i64,
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
    /// A lease held by `lessor` under a contract (E4). Placed at contract
    /// activation from the source party's acceptance, released at settlement or
    /// refund — never staff-minted, because the contract IS the consent (D-E4-8).
    Lease {
        /// Who leases it out.
        lessor: PrincipalId,
        /// The contract that placed it.
        contract: ContractId,
    },
    /// Collateral pledged under a contract (E4). Same lifecycle as
    /// [`Encumbrance::Lease`]: contract-placed, contract-released.
    Collateral {
        /// The contract that placed it.
        contract: ContractId,
    },
}

/// Journaled asset lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetLifecycle {
    /// Normal; the only state an asset op may touch.
    Active,
    /// Written off (destroyed/lost/scrapped). Terminal in E3.
    WrittenOff,
    /// **Abandoned** by its owner (E7 cut 3; doc 10 T3, §15; 08f §1.1's "in E7
    /// with the mechanical recipe written out").
    ///
    /// A **third, non-terminal** state, and deliberately not a null owner:
    /// `title_owner` is a non-optional `PrincipalId` with no `none` arm, so
    /// 10b §3.3's "a unilateral `AssetEffect` setting `title_owner` to none" is
    /// unbuildable as drafted. The owner is **retained as the last owner** so
    /// provenance survives — T3: *"captured units keep title and provenance
    /// until legally transferred, abandoned or written off"* — and **custody is
    /// unchanged**, because abandoning a thing is not moving it.
    ///
    /// An abandoned asset **authorizes nobody and takes no asset op but its
    /// title**: `apply_asset_op`'s live-asset gate admits `TransferTitle` (the
    /// salvage claim that turns possession into title, which stays an ordinary
    /// E4b2 contract with a `Before` deadline and a named adjudicator) and
    /// refuses everything else. It is not `WrittenOff`: the thing still exists,
    /// which is why the destroy gate still holds it.
    Abandoned,
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
    /// The **rolling budget** beside the threshold (E6a step 1, D-E6-21 / DL-10):
    /// a debit that would take the account's spend over the trailing `period_secs`
    /// past `total` needs the same approval an over-threshold debit does. `None`
    /// (the default, so every E3 account doc still deserializes) keeps the
    /// threshold-only behaviour; there is deliberately no default window.
    #[serde(default)]
    pub window: Option<ApprovalWindow>,
}

/// The longest trailing period an [`ApprovalWindow`] may declare (90 days). It is
/// what bounds the RAM-only per-account debit tail the window is computed from: an
/// entry older than this is dropped at index time, so the tail is O(entries in the
/// widest window) and never O(journal).
pub const MAX_APPROVAL_WINDOW_SECS: i64 = 90 * 86_400;

/// A rolling spending budget on one account (E6a step 1, D-E6-21): the account's
/// debits **plus placed holds** over the trailing `period_secs` (wall clock), with a
/// refund never re-opening the window, must not exceed `total` — or the post needs
/// approval on the threshold's own path.
///
/// It closes the ≤-threshold drip against an org treasury (08f review M3): a
/// per-transaction threshold is not a budget, and an officer with `economy.account.post`
/// could otherwise fund runs of ≤ threshold each, forever. Loading re-checks the
/// bounds (a `period_secs` of 0 or past [`MAX_APPROVAL_WINDOW_SECS`], or a negative
/// `total`, refuses to deserialize), so a hand-written document cannot declare a
/// window the engine cannot compute.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ApprovalWindowWire")]
pub struct ApprovalWindow {
    /// The most the account may spend inside one trailing period, in its instrument.
    pub total: Amount,
    /// The trailing period, in wall-clock seconds (`1..=MAX_APPROVAL_WINDOW_SECS`).
    pub period_secs: i64,
}

#[derive(Deserialize)]
struct ApprovalWindowWire {
    total: Amount,
    period_secs: i64,
}

impl ApprovalWindow {
    /// Build a window, refusing a negative total or a period outside
    /// `1..=`[`MAX_APPROVAL_WINDOW_SECS`].
    #[must_use]
    pub fn new(total: Amount, period_secs: i64) -> Option<ApprovalWindow> {
        if total.is_negative() || period_secs <= 0 || period_secs > MAX_APPROVAL_WINDOW_SECS {
            return None;
        }
        Some(ApprovalWindow { total, period_secs })
    }
}

impl TryFrom<ApprovalWindowWire> for ApprovalWindow {
    type Error = String;

    fn try_from(w: ApprovalWindowWire) -> Result<ApprovalWindow, String> {
        ApprovalWindow::new(w.total, w.period_secs).ok_or_else(|| {
            format!(
                "an approval window needs a non-negative total and a period of \
                 1..={MAX_APPROVAL_WINDOW_SECS} seconds"
            )
        })
    }
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
    /// The asset was **abandoned** by its owner (E7 cut 3): lifecycle moves to
    /// [`AssetLifecycle::Abandoned`], `title_owner` is retained as the last
    /// owner, and custody is untouched.
    Abandoned,
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
    /// The asset was **reported stolen** (E6b): a `Disputed` claim plus the
    /// bounded [`StolenReport`] the visibility lives on.
    StolenReported,
    /// The **catalogue reference** was set or cleared (E6b independent verify fold,
    /// M7). Title does not move and no claim moves; only what the thing is declared
    /// to be, and the revision that stamps it.
    CatalogSet,
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

// ---- holds (E4a) ----------------------------------------------------------

/// What a hold reserves value *for*.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoldPurpose {
    /// A contract's escrow (placed when a party accepts).
    Escrow {
        /// The contract.
        contract: ContractId,
    },
    /// A ruleset reservation (a work order, a production run, a quote).
    Reservation {
        /// The reserving namespace.
        kind: SmolStr,
        /// The reserving record's id in that namespace.
        id: u64,
    },
    /// A staff/player hold with a stated reason.
    Manual {
        /// Why (≤ [`crate::MAX_REASON_BYTES`]).
        reason: SmolStr,
    },
}

/// A hold's lifecycle — doc 09 C8's `ReservationRecord` states, adopted verbatim
/// (§12) so the K4 migration is a rename of nothing at all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoldStatus {
    /// Live: counted in `Account.held_amount`, subtracted from available funds.
    Held,
    /// Spent by a transaction. `amount` is what that transaction actually took
    /// (≤ the hold's `amount`); the remainder was released in the same effect, so a
    /// capture is exactly attributable per hold (review point A1).
    Consumed {
        /// The transaction that consumed it.
        transaction: TransactionId,
        /// How much it took.
        amount: Amount,
    },
    /// Released without being spent.
    Released,
    /// Aged out past its deadline (the sweep).
    Expired,
}

impl HoldStatus {
    /// Whether this hold still reserves value.
    #[must_use]
    pub fn is_held(&self) -> bool {
        matches!(self, HoldStatus::Held)
    }
}

/// One reservation against an account — doc 09 C8's `ReservationRecord` in the
/// economy's vocabulary (§12).
///
/// **Journal authority** (D-E4-1): every placement, capture, release and expiry is a
/// [`HoldEffect`] inside a journal transaction carrying the full post-state, so
/// replay rebuilds every hold and every `held_amount` from the journal alone. The
/// `hold:{id}` doc is a projection; a missing one heals silently because a hold
/// carries no recovery latch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    /// Identity (never reused; floored from the journal, never from surviving docs).
    pub id: HoldId,
    /// The account whose funds are reserved.
    pub account: AccountId,
    /// How much (> 0).
    pub amount: Amount,
    /// Who the reservation is *for*, if anyone — a beneficiary may release their own
    /// claim (invariant 25).
    pub beneficiary: Option<PrincipalId>,
    /// What it reserves for.
    pub purpose: HoldPurpose,
    /// Who placed it.
    pub placed_by: PrincipalId,
    /// The **placing plan's idempotency key** — doc 09 C8's `holder` (§12). Two
    /// retries of one placement therefore name one holder, and an audit can trace a
    /// reservation back to the exact intent that made it.
    ///
    /// **It is an attribution label, never authority.** The key is caller-chosen:
    /// `post` is public, so anyone who may post at all may name any key that is not
    /// already taken. Nothing in the engine, in `ContractAuth`, or in the sweep may
    /// branch on it — who may capture, release or expire a hold is
    /// [`crate::AuthEval::authorize_hold`]'s question and nobody else's (E4a verify
    /// fold, finding 8).
    pub holder: IdempotencyKey,
    /// When it lapses, on a named clock (`None` = never).
    pub expires: Option<EconomicDeadline>,
    /// Lifecycle.
    pub status: HoldStatus,
    /// The journal sequence of the last effect folded into this projection.
    pub last_journal_seq: u64,
    /// Staleness stamp (bumped once per effect).
    pub revision: u64,
}

/// One hold change carried by a transaction. `hold` is the **full post-state**, the
/// same journal-authority discipline as [`AssetEffect`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldEffect {
    /// What kind of change (audit + display; the state is `hold`).
    pub kind: HoldEffectKind,
    /// The complete hold after the change.
    pub hold: Hold,
}

/// What a [`HoldEffect`] did.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum HoldEffectKind {
    /// A reservation was placed.
    Placed,
    /// A reservation was (partly) spent; the remainder was released with it.
    Captured,
    /// A reservation was released unspent.
    Released,
    /// A reservation aged out past its deadline.
    Expired,
}

// ---- compact economic events (E4a, D-E4-13) -------------------------------

/// A payload-free [`crate::ContractState`] discriminant — what an event may say.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContractStateKind {
    /// Being drafted.
    Draft,
    /// Offered to its parties.
    Offered,
    /// Every party accepted.
    Active,
    /// Every condition satisfied; settlement pending.
    ReadyToSettle,
    /// Settled.
    Settled,
    /// Disputed.
    Disputed,
    /// Cancelled.
    Cancelled,
    /// Expired.
    Expired,
}

/// A payload-free [`crate::ObligationStatus`] discriminant — what an event may say.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ObligationStatusKind {
    /// Settling on schedule.
    Active,
    /// A period was missed; within the grace allowance.
    Grace,
    /// Past the grace allowance.
    Delinquent,
    /// Out of the due pool pending a human decision.
    Suspended,
    /// Finished (a one-shot that settled).
    Satisfied,
    /// Cancelled.
    Cancelled,
}

/// What the economy announces to observers (§6.3).
///
/// **Ids and kinds only** (D-E4-13, tighter than §12.4's sketch): no amounts, no
/// parties, no `cause` text — a staff reason string riding an event would leak
/// through every observer that logs it. An observer that needs detail asks the
/// economy through the disclosure filter, under the viewer's own authority.
///
/// The pre-C6 form of doc 09's `SemanticEvent`; it migrates when commit tokens exist
/// (§12).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomicEvent {
    /// A transaction posted.
    TransactionPosted {
        /// Which one.
        id: TransactionId,
    },
    /// A hold changed state.
    HoldChanged {
        /// Which hold.
        id: HoldId,
        /// What happened to it.
        kind: HoldEffectKind,
    },
    /// A contract changed state.
    ContractChanged {
        /// Which contract.
        id: ContractId,
        /// Its new state, kind only.
        state: ContractStateKind,
        /// The transaction the transition **landed on**, when it had one (E4b2's
        /// terminal-transition rule: a terminal transition is a post under the
        /// contract's deterministic key followed by the state write).
        ///
        /// This is the event's **de-duplication key**, and it is here for one
        /// reason: a settlement whose state write is lost is finished later — by
        /// the sweep's retry, whose post resolves to `AlreadyPosted`, or by boot's
        /// `finished_transition`, which reads the same key out of the idempotency
        /// index. Both produce the transition a second time. Keyed on the landed
        /// transaction id, an observer that publishes (the canon bridge) can tell
        /// "this settlement" from "this settlement again"; keyed on the contract
        /// id and state alone it could not.
        #[serde(default)]
        transaction: Option<TransactionId>,
    },
    /// An obligation changed status.
    ObligationChanged {
        /// Which obligation.
        id: ObligationId,
        /// Its new status, kind only.
        status_kind: ObligationStatusKind,
    },
    /// An asset's title record changed.
    AssetTitleChanged {
        /// Which object.
        object: ObjId,
        /// Its new revision.
        revision: u64,
    },
    /// The ladder **opened, reopened, or re-notified** a level-2/3 record (E6a
    /// step 1, DL-05): the staff-channel fan-out E4b1 deviation 5 left to the
    /// server's observer glue. Emitted on every step of the ladder's own re-notify
    /// schedule (open → 1 h → daily; a reopen restarts it), so an observer that
    /// pages staff follows that schedule and dedupes on the record's step — never a
    /// storm. Ids and kinds only, as every event: the diagnosis is read off the
    /// record under the viewer's own standing.
    EscalationOpened {
        /// Which record.
        id: EscalationId,
        /// What it is about.
        subject: crate::escalation::EscalationSubject,
        /// 2 = page, 3 = the whole-economy stop's companion.
        level: u8,
    },
}
