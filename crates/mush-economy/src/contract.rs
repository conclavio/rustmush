//! Economic contracts — the pure consent carrier (E4a, §3.5, D-E4-4).
//!
//! E3 plans are single-authority: one actor, one live authorization, no counterparty
//! (D-E3-4). A sale needs the buyer's debit **and** the seller's title in one bundle,
//! and neither can be captured as a stored credential. A contract is how E4 answers
//! that without inventing one:
//!
//! - **The consent surface is one hashed blob.** Parties, terms, conditions,
//!   deadline, visibility and scope are hashed together into [`EconomicContract::consent_hash`],
//!   frozen at `Offer`. An [`Acceptance`] records the hash it bound, so changing
//!   *anything* in the surface afterwards invalidates every acceptance — a new
//!   contract is drafted instead (review point B5).
//! - **The acceptances ARE the settlement authority.** [`ContractAuth`] is an
//!   [`crate::AuthEval`] derived from the live record that authorizes exactly the
//!   captures (per hold, per amount), the title transfers (**both** sides —
//!   D-E3-12 holds), and the collateral releases the surface describes, and nothing
//!   else. No authority is stored; it is re-derived at settlement.
//! - **Conditions are re-verified live.** [`reverify`] re-checks the deposit, the
//!   title, the capability attestor and the external fact *at settlement*, so a
//!   seller who withdrew the goods or lost their title between attestation and
//!   settlement moves nothing.
//!
//! Everything here is pure: [`settlement_plan`] and [`refund_plan`] *build*
//! [`crate::EconomyPlan`]s, which the ordinary `validate`/`apply` path then proves.
//! A contract never posts by itself and never bypasses an invariant.

use std::collections::BTreeMap;

use mush_core::ObjId;
use mush_org::{CapabilityId, OrgScopeId, PrincipalId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::clock::{Clocks, EconomicClock, EconomicDeadline, EconomicSchedule};
use crate::engine::{MAX_ASSET_OPS_PER_PLAN, MAX_LINES_PER_TX, ValidateEnv};
use crate::ids::{AccountId, ContractId, HoldId, InstrumentId, TransactionId, VaultId};
use crate::obligation::GracePolicy;
use crate::plan::{AssetOp, EconomyPlan, HoldOp, HoldRevision, RequestedLine};
use crate::record::{
    AccountStatus, AssetLifecycle, ContractStateKind, EconomicCause, EconomicVisibility,
    Encumbrance, ExternalRef, IdempotencyKey,
};
use crate::snapshot::EconomySnapshot;

// ---- caps (§9) ------------------------------------------------------------

/// Maximum parties on one contract (§9).
pub const MAX_PARTIES: usize = 16;
/// Maximum conditions on one contract (§9).
pub const MAX_CONDITIONS: usize = 16;
/// Maximum deliverables on one contract (§9).
pub const MAX_DELIVERABLES: usize = 16;
/// Maximum payout shares in one split (§9).
pub const MAX_SHARES: usize = 32;
/// Maximum non-terminal contracts one issuer may hold open (§9; E4b2 enforces).
pub const MAX_OPEN_CONTRACTS_PER_ISSUER: usize = 16;
/// Maximum live contracts one party may be named on (§9; E4b2 enforces).
pub const MAX_LIVE_CONTRACTS_PER_PARTY: usize = 32;
/// How many settlement attempts a `ReadyToSettle` contract gets before it is
/// `Disputed` and escalated (§9).
pub const MAX_SETTLE_ATTEMPTS: u8 = 8;
/// How long a `Draft` survives before GC (§9).
pub const DRAFT_TTL_SECS: i64 = 7 * 86_400;
/// How long a `Disputed` contract waits for unanimous party agreement before a
/// level-2 escalation (§9).
pub const DISPUTE_TTL_SECS: i64 = 3 * 86_400;
/// Permille denominator — a `Share::Permille` is out of this.
pub const PERMILLE: u32 = 1000;

// ---- the consent surface --------------------------------------------------

/// A share of the settled total.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Share {
    /// A fixed amount, taken first.
    Exact(Amount),
    /// A permille (‰) of what remains after every [`Share::Exact`], **floored**.
    Permille(u32),
}

/// One payee of the deterministic split.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutShare {
    /// Who gets paid.
    pub to: AccountId,
    /// How much.
    pub share: Share,
}

/// What one party puts into escrow when they accept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consideration {
    /// Which party pays.
    pub from: PrincipalId,
    /// From which account (they need debit authority on it **independently** of
    /// their right to accept — D-E4-4).
    pub account: AccountId,
    /// How much (> 0).
    pub amount: Amount,
}

/// A template for the obligation a lease/retainer contract creates at activation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObligationTemplate {
    /// Who will owe.
    pub debtor: PrincipalId,
    /// Who will be owed.
    pub creditor: PrincipalId,
    /// The account debited each period.
    pub debit_account: AccountId,
    /// The account credited each period.
    pub credit_account: AccountId,
    /// How much per period.
    pub amount: Amount,
    /// When and how often.
    pub schedule: EconomicSchedule,
    /// The grace allowance.
    pub grace: GracePolicy,
    /// Whether a routine success notifies.
    pub notify_on_success: bool,
}

/// What a contract delivers besides money.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Deliverable {
    /// Title moves. **Both** parties must have accepted (D-E3-12: title carries
    /// liabilities a money credit does not).
    TitleTransfer {
        /// The object.
        object: ObjId,
        /// The current title owner.
        from: PrincipalId,
        /// The new title owner.
        to: PrincipalId,
    },
    /// The object must sit in the vault (delivered before settlement; the matching
    /// `DepositInto` condition custody-locks it while the contract is live).
    DepositInto {
        /// The object.
        object: ObjId,
        /// The vault.
        vault: VaultId,
    },
    /// The object is pledged: an [`Encumbrance::Collateral`] is placed at activation
    /// and released at settlement or refund.
    Collateral {
        /// The object.
        object: ObjId,
        /// Whose it is.
        from: PrincipalId,
    },
    /// An obligation is created at activation (a lease, a retainer) and cancelled if
    /// the contract is cancelled or expires.
    Obligation {
        /// What to create.
        template: Box<ObligationTemplate>,
    },
}

/// What happens when a contract reaches its deadline unsettled.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expiry {
    /// Release every escrow hold; nothing moves.
    Refund,
    /// The escrow pays out to one account.
    Forfeit {
        /// Who gets it.
        to: AccountId,
    },
    /// Freeze escrow and open a dispute.
    Dispute,
}

/// The money-and-goods half of the consent surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractTerms {
    /// What each party pays, escrowed at their accept.
    pub consideration: Vec<Consideration>,
    /// What is delivered (≤ [`MAX_DELIVERABLES`]).
    pub deliverables: Vec<Deliverable>,
    /// How the escrowed total is split (≤ [`MAX_SHARES`]).
    pub split: Vec<PayoutShare>,
    /// Where the floored remainder lands (scenario 17).
    pub remainder_to: AccountId,
    /// What an unsettled deadline does.
    pub on_expiry: Expiry,
}

/// Who may attest a condition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttestorSet {
    /// One named party. A party attestation **stands once given**.
    Party(PrincipalId),
    /// Anyone holding `capability` on `scope` — **re-qualified at settlement** like
    /// an approver (E3's A5 rule).
    Capability {
        /// The org compartment.
        scope: Option<OrgScopeId>,
        /// The capability.
        capability: CapabilityId,
    },
    /// Staff — re-qualified at settlement.
    Staff,
    /// A canon record of `kind` naming `source`. E5's owner answer 20: progress must
    /// cite a **specific** front/opportunity/operation, so `kind` alone is not a
    /// match key (§6.2).
    Canon {
        /// The canon event kind.
        kind: SmolStr,
        /// The specific source it must name.
        source: SmolStr,
    },
}

/// Who actually attested.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttestorRef {
    /// A principal (a party, a capability holder, staff).
    Principal(PrincipalId),
    /// A canon record.
    Canon {
        /// `canon:{id}`.
        canon: SmolStr,
    },
    /// An external fact provider's answer.
    Fact(ExternalRef),
    /// The clock itself (a `Before`/`After` condition).
    Clock,
}

/// One recorded satisfaction of one condition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    /// Who attested.
    pub by: AttestorRef,
    /// When, **on `clock`**.
    pub at: i64,
    /// The clock `at` is expressed on. Carried so an [`TypedCondition::After`] can
    /// refuse to compare two stamps from different clocks (doc 09 §2.5's rule) — the
    /// same reason [`EconomicDeadline`] exists.
    pub clock: EconomicClock,
    /// Free-form provenance (`canon:{id}`, a job id, a staff note).
    pub source: SmolStr,
}

/// A settlement condition. Only **durable facts** and clocks — never LLM output,
/// never narrative prose, and never a synchronous foreign event (§1.1, D-E4-4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypedCondition {
    /// Someone in the set attested.
    Attestation {
        /// Who may.
        by: AttestorSet,
    },
    /// The economy's own journaled custody effect put the object in the vault. While
    /// the contract is live the object is **custody-locked**.
    DepositInto {
        /// The object.
        object: ObjId,
        /// The vault.
        vault: VaultId,
    },
    /// Journal-authority title state.
    TitleHeld {
        /// The object.
        object: ObjId,
        /// Who must hold title.
        by: PrincipalId,
    },
    /// An external provider's fact (`Some(true)` only; `None` = unknown =
    /// unsatisfied). The provider seam is E4c; here it defaults to unknown.
    Fact {
        /// The reference.
        fact: ExternalRef,
    },
    /// Absolute time: satisfied until the deadline passes.
    Before {
        /// The deadline.
        deadline: EconomicDeadline,
    },
    /// Relative time: satisfied once `condition[i]` has been satisfied for `delay`
    /// on `clock` (E6's fence settlement delay).
    After {
        /// The index of the anchor condition.
        condition: usize,
        /// How long after it.
        delay: i64,
        /// On which clock.
        clock: EconomicClock,
    },
}

/// Everything an acceptance binds to — hashed as **one canonical blob** (B5).
///
/// `parties` is a `Vec` rather than a `BTreeSet` because `mush_org::PrincipalId`
/// is `Hash`-but-not-`Ord`; it is canonicalized by [`principal_key`] wherever order
/// could matter (the hash, the duplicate check), so the hash is order-independent
/// exactly as a set would be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentSurface {
    /// Everyone bound (≤ [`MAX_PARTIES`]; includes the issuer).
    pub parties: Vec<PrincipalId>,
    /// The money and goods.
    pub terms: ContractTerms,
    /// What must be true to settle (≤ [`MAX_CONDITIONS`]).
    pub conditions: Vec<TypedCondition>,
    /// When it lapses.
    pub deadline: Option<EconomicDeadline>,
    /// The disclosure every party consents to. Only a `Public` contract ever reaches
    /// canon (D-E4-12, E15).
    pub visibility: EconomicVisibility,
    /// The org compartment, if this is an org contract.
    pub scope: Option<OrgScopeId>,
}

/// One party's acceptance, bound to the exact consent surface they saw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acceptance {
    /// Who accepted.
    pub party: PrincipalId,
    /// When (real seconds).
    pub at: i64,
    /// The consent hash they bound. An acceptance whose hash no longer matches the
    /// contract's is **not** an acceptance.
    pub consent_hash: [u8; 32],
}

/// Contract lifecycle. Doc 08 §8.4's `Accepted` is folded into `Active` — the last
/// acceptance activates (D-E4-10).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractState {
    /// Being built; the consent surface is still mutable.
    Draft,
    /// Offered; the consent surface is frozen and acceptances are being collected.
    Offered,
    /// Every party accepted.
    Active,
    /// Every condition satisfied; settlement pending (or retrying).
    ReadyToSettle,
    /// Settled.
    Settled {
        /// The transaction that carried it.
        transaction: TransactionId,
    },
    /// Frozen pending resolution.
    Disputed,
    /// Cancelled.
    Cancelled,
    /// Lapsed past its deadline.
    Expired,
}

impl ContractState {
    /// The payload-free discriminant an [`crate::EconomicEvent`] may carry.
    #[must_use]
    pub fn kind(&self) -> ContractStateKind {
        match self {
            ContractState::Draft => ContractStateKind::Draft,
            ContractState::Offered => ContractStateKind::Offered,
            ContractState::Active => ContractStateKind::Active,
            ContractState::ReadyToSettle => ContractStateKind::ReadyToSettle,
            ContractState::Settled { .. } => ContractStateKind::Settled,
            ContractState::Disputed => ContractStateKind::Disputed,
            ContractState::Cancelled => ContractStateKind::Cancelled,
            ContractState::Expired => ContractStateKind::Expired,
        }
    }
    /// Whether the contract is live: its escrow is real and its `DepositInto`
    /// objects are custody-locked.
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self, ContractState::Active | ContractState::ReadyToSettle)
    }
    /// Whether nothing more will happen to it.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ContractState::Settled { .. } | ContractState::Cancelled | ContractState::Expired
        )
    }
}

/// A typed, consented, escrowed agreement between principals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicContract {
    /// Identity (never reused).
    pub id: ContractId,
    /// Who drafted it.
    pub issuer: PrincipalId,
    /// Everything acceptances bind to.
    pub consent: ConsentSurface,
    /// The hash of `consent`, **frozen at `Offer`**. It is deliberately stored
    /// rather than recomputed on read: a mutation after `Offer` is then *visible* as
    /// a mismatch instead of silently re-signing itself.
    pub consent_hash: [u8; 32],
    /// Who has accepted.
    pub acceptances: Vec<Acceptance>,
    /// Parallel to `consent.conditions`.
    pub satisfied: Vec<Option<Attestation>>,
    /// The escrow holds placed as parties accepted.
    pub escrow: Vec<HoldId>,
    /// Lifecycle.
    pub state: ContractState,
    /// How many settlement attempts have failed re-verification (§9's
    /// [`MAX_SETTLE_ATTEMPTS`]).
    #[serde(default)]
    pub settle_attempts: u8,
    /// Staleness stamp.
    pub revision: u64,
}

/// Why a contract operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    /// The contract is not in a state that admits this operation.
    #[error("contract {0} is not in a state that allows that")]
    State(ContractId),
    /// The stored `consent_hash` no longer matches the consent surface, or an
    /// acceptance bound a different one.
    #[error("the consent surface has changed since it was offered")]
    ConsentMismatch,
    /// A party named in the surface has not accepted.
    #[error("not every party has accepted")]
    NotAccepted,
    /// A cap was exceeded.
    #[error("contract exceeds a cap: {0}")]
    TooLarge(&'static str),
    /// The split cannot be paid out of the escrowed total.
    #[error("the split is not feasible")]
    SplitInfeasible,
    /// A referenced account does not exist or is not active.
    #[error("no usable account: {0}")]
    NoSuchAccount(AccountId),
    /// A referenced vault does not exist.
    #[error("no such vault: {0}")]
    NoSuchVault(VaultId),
    /// A referenced escrow hold does not exist.
    #[error("no such hold: {0}")]
    NoSuchHold(HoldId),
    /// An escrow hold is no longer `Held` — the escrow has been broken.
    #[error("hold {0} is no longer held")]
    HoldNotHeld(HoldId),
    /// Two accounts in the same contract hold different instruments; there is no
    /// implicit exchange in this economy.
    #[error("the contract mixes instruments")]
    InstrumentMismatch,
    /// A condition is not (or no longer) satisfied.
    #[error("condition {0} is not satisfied")]
    ConditionFailed(usize),
    /// A condition references another condition that does not exist (or itself).
    #[error("condition {0} references an impossible anchor")]
    BadConditionRef(usize),
    /// The contract is shaped in a way no single plan can carry.
    #[error("infeasible contract: {0}")]
    Infeasible(&'static str),
    /// Checked arithmetic overflowed.
    #[error("arithmetic overflow")]
    Overflow,
}

// ---- canonical ordering + the consent hash --------------------------------

/// A total order over `mush_org::PrincipalId`, which is `Hash` but not `Ord`.
/// Purely local: nothing persisted depends on it, only canonical ordering does.
#[must_use]
pub fn principal_key(p: &PrincipalId) -> (u8, u64, &str) {
    match p {
        PrincipalId::Object(o) => (0, u64::from(o.0), ""),
        PrincipalId::Organization(g) => (1, g.0, ""),
        PrincipalId::System(s) => (2, 0, s.as_str()),
    }
}

#[derive(Serialize)]
struct CanonicalConsent<'a> {
    parties: Vec<&'a PrincipalId>,
    terms: &'a ContractTerms,
    conditions: &'a [TypedCondition],
    deadline: &'a Option<EconomicDeadline>,
    visibility: EconomicVisibility,
    scope: &'a Option<OrgScopeId>,
}

/// The canonical SHA-256 of a consent surface — **everything** an acceptance binds
/// to (B5): parties, terms, conditions, deadline, visibility, and scope.
///
/// Parties are sorted by [`principal_key`] so the hash is order-independent; every
/// other vector's order *is* consented content (`conditions` are indexed by
/// [`TypedCondition::After`], and the split's order decides who is floored first).
/// Deterministic for the same reason [`crate::plan_hash`] is: field-ordered structs,
/// no floats, no maps.
#[must_use]
pub fn consent_hash(consent: &ConsentSurface) -> [u8; 32] {
    let mut parties: Vec<&PrincipalId> = consent.parties.iter().collect();
    parties.sort_by(|a, b| principal_key(a).cmp(&principal_key(b)));
    let canonical = CanonicalConsent {
        parties,
        terms: &consent.terms,
        conditions: &consent.conditions,
        deadline: &consent.deadline,
        visibility: consent.visibility,
        scope: &consent.scope,
    };
    let bytes = serde_json::to_vec(&canonical).expect("canonical consent serializes");
    Sha256::digest(&bytes).into()
}

/// The deterministic idempotency key for a contract's settlement:
/// `contract/{id}/settle` (§12's C8 list). Settling twice is `AlreadyPosted`.
#[must_use]
pub fn settle_key(id: ContractId) -> IdempotencyKey {
    IdempotencyKey::new(SmolStr::new_static("contract"), SmolStr::new(format!("{}/settle", id.0)))
}
/// The deterministic idempotency key for a contract's refund: `contract/{id}/refund`.
#[must_use]
pub fn refund_key(id: ContractId) -> IdempotencyKey {
    IdempotencyKey::new(SmolStr::new_static("contract"), SmolStr::new(format!("{}/refund", id.0)))
}

/// Whether `key` is one of contract `id`'s two deterministic keys.
///
/// This is the custody lock's second gate (§3.5). A plan's `cause` is caller-settable
/// — `post` is public — so the cause alone would let anyone who could name a live
/// contract unlock its deposit. Its deterministic keys cannot be borrowed silently:
/// a forger who uses one either collides with the real settlement (an idempotency
/// conflict, which doc 09 C8 says to treat as an integrity incident) or *is* it.
#[must_use]
pub fn is_contract_key(key: &IdempotencyKey, id: ContractId) -> bool {
    key == &settle_key(id) || key == &refund_key(id)
}

impl EconomicContract {
    /// Whether the stored `consent_hash` still describes the consent surface. A
    /// mutation after `Offer` makes this `false` and every acceptance void.
    #[must_use]
    pub fn consent_intact(&self) -> bool {
        consent_hash(&self.consent) == self.consent_hash
    }
    /// Whether `party` has an acceptance bound to the **current** consent hash.
    #[must_use]
    pub fn accepted_by(&self, party: &PrincipalId) -> bool {
        self.consent_intact()
            && self
                .acceptances
                .iter()
                .any(|a| &a.party == party && a.consent_hash == self.consent_hash)
    }
    /// Whether every party in the surface has accepted it as it stands.
    #[must_use]
    pub fn fully_accepted(&self) -> bool {
        self.consent_intact() && self.consent.parties.iter().all(|p| self.accepted_by(p))
    }
    /// The escrowed total per account, counting only holds still `Held`.
    #[must_use]
    pub fn escrowed_by_account(&self, snapshot: &EconomySnapshot) -> BTreeMap<AccountId, Amount> {
        let mut out: BTreeMap<AccountId, Amount> = BTreeMap::new();
        for id in &self.escrow {
            let Some(hold) = snapshot.hold(*id) else { continue };
            if !hold.status.is_held() {
                continue;
            }
            let slot = out.entry(hold.account).or_insert(Amount::ZERO);
            *slot = slot.checked_add(hold.amount).unwrap_or(*slot);
        }
        out
    }
}

// ---- the deterministic split (scenario 17) --------------------------------

/// Apply the split to `total`: [`Share::Exact`] first, then [`Share::Permille`] of
/// what remains — **floored** — and the leftover to `remainder_to`.
///
/// Returns `(account, amount)` pairs in declared order with the remainder last;
/// zero-valued payouts are dropped (a zero journal line is refused by invariant 2).
/// The floor is what makes the split deterministic: three equal thirds of 100 pay
/// 33/33/33 and the remaining 1 goes where the parties agreed it goes, rather than
/// depending on rounding mode or iteration order.
pub fn apply_split(total: Amount, terms: &ContractTerms) -> Result<Vec<(AccountId, Amount)>, ContractError> {
    let total_w = total.widen();
    if total_w < 0 {
        return Err(ContractError::SplitInfeasible);
    }
    let mut exact_sum: i128 = 0;
    let mut permille_sum: u64 = 0;
    for s in &terms.split {
        match s.share {
            Share::Exact(a) => {
                if a.is_negative() {
                    return Err(ContractError::SplitInfeasible);
                }
                exact_sum += a.widen();
            }
            Share::Permille(p) => permille_sum += u64::from(p),
        }
    }
    if exact_sum > total_w || permille_sum > u64::from(PERMILLE) {
        return Err(ContractError::SplitInfeasible);
    }
    let remaining = total_w - exact_sum;
    let mut out: Vec<(AccountId, Amount)> = Vec::with_capacity(terms.split.len() + 1);
    let mut assigned: i128 = exact_sum;
    for s in &terms.split {
        let amount = match s.share {
            Share::Exact(a) => a,
            Share::Permille(p) => {
                let v = remaining * i128::from(p) / i128::from(PERMILLE);
                assigned += v;
                Amount::from_widened(v).ok_or(ContractError::Overflow)?
            }
        };
        if !amount.is_zero() {
            out.push((s.to, amount));
        }
    }
    let remainder = Amount::from_widened(total_w - assigned).ok_or(ContractError::Overflow)?;
    if !remainder.is_zero() {
        out.push((terms.remainder_to, remainder));
    }
    Ok(out)
}

// ---- feasibility at Offer -------------------------------------------------

fn account_instrument(
    snapshot: &EconomySnapshot,
    id: AccountId,
    seen: &mut Option<InstrumentId>,
) -> Result<(), ContractError> {
    let account = snapshot.account(id).ok_or(ContractError::NoSuchAccount(id))?;
    if account.status != AccountStatus::Active {
        return Err(ContractError::NoSuchAccount(id));
    }
    match seen {
        Some(i) if *i != account.instrument => Err(ContractError::InstrumentMismatch),
        Some(_) => Ok(()),
        None => {
            *seen = Some(account.instrument.clone());
            Ok(())
        }
    }
}

/// Feasibility at `Offer` (§3.5): the caps, the split, one custody op per
/// settlement, and every referenced account/vault existing and coherent.
///
/// Checked **again** at settlement, because the world moves between the two.
pub fn check_offerable(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
) -> Result<(), ContractError> {
    let consent = &c.consent;
    if consent.parties.len() > MAX_PARTIES {
        return Err(ContractError::TooLarge("too many parties"));
    }
    if consent.conditions.len() > MAX_CONDITIONS {
        return Err(ContractError::TooLarge("too many conditions"));
    }
    if consent.terms.deliverables.len() > MAX_DELIVERABLES {
        return Err(ContractError::TooLarge("too many deliverables"));
    }
    if consent.terms.split.len() > MAX_SHARES {
        return Err(ContractError::TooLarge("too many shares"));
    }
    if c.satisfied.len() != consent.conditions.len() {
        return Err(ContractError::Infeasible("satisfaction vector does not match the conditions"));
    }
    if !consent.parties.iter().any(|p| p == &c.issuer) {
        return Err(ContractError::Infeasible("the issuer is not a party"));
    }
    // A duplicated party would let one acceptance stand for two.
    let mut keyed: Vec<(u8, u64, &str)> = consent.parties.iter().map(principal_key).collect();
    keyed.sort_unstable();
    if keyed.windows(2).any(|w| w[0] == w[1]) {
        return Err(ContractError::Infeasible("a party is named twice"));
    }

    // Every account exists, is active, and holds ONE instrument: there is no
    // implicit exchange in this economy, so a mixed-instrument contract could never
    // balance (invariant 3).
    let mut instrument: Option<InstrumentId> = None;
    let mut total: i128 = 0;
    for con in &consent.terms.consideration {
        if !con.amount.widen().gt(&0) {
            return Err(ContractError::Infeasible("a consideration must be positive"));
        }
        account_instrument(snapshot, con.account, &mut instrument)?;
        total += con.amount.widen();
    }
    for s in &consent.terms.split {
        account_instrument(snapshot, s.to, &mut instrument)?;
    }
    account_instrument(snapshot, consent.terms.remainder_to, &mut instrument)?;
    if let Expiry::Forfeit { to } = consent.terms.on_expiry {
        account_instrument(snapshot, to, &mut instrument)?;
    }
    apply_split(Amount::from_widened(total).ok_or(ContractError::Overflow)?, &consent.terms)?;

    // Deliverables must fit the ONE plan settlement builds (E3's per-plan caps), and
    // the module admits exactly one custody effect per plan.
    let mut title_ops = 0usize;
    let mut deposits = 0usize;
    for d in &consent.terms.deliverables {
        match d {
            Deliverable::TitleTransfer { .. } => title_ops += 1,
            Deliverable::Collateral { .. } => title_ops += 1,
            Deliverable::DepositInto { vault, .. } => {
                deposits += 1;
                snapshot.vault(*vault).ok_or(ContractError::NoSuchVault(*vault))?;
            }
            Deliverable::Obligation { template } => {
                account_instrument(snapshot, template.debit_account, &mut instrument)?;
                account_instrument(snapshot, template.credit_account, &mut instrument)?;
            }
        }
    }
    if title_ops > MAX_ASSET_OPS_PER_PLAN {
        return Err(ContractError::TooLarge("too many title ops for one settlement"));
    }
    if deposits > 1 {
        return Err(ContractError::Infeasible("one custody op per settlement"));
    }
    if consent.terms.split.len() + consent.terms.consideration.len() + 1 > MAX_LINES_PER_TX {
        return Err(ContractError::TooLarge("too many settlement lines"));
    }

    // Conditions must be resolvable: an `After` anchor must exist and never be
    // itself (a self-anchored condition can never become true).
    for (i, con) in consent.conditions.iter().enumerate() {
        if let TypedCondition::After { condition, .. } = con
            && (*condition >= consent.conditions.len() || *condition == i)
        {
            return Err(ContractError::BadConditionRef(i));
        }
        if let TypedCondition::DepositInto { vault, .. } = con {
            snapshot.vault(*vault).ok_or(ContractError::NoSuchVault(*vault))?;
        }
    }
    Ok(())
}

// ---- live re-verification -------------------------------------------------

/// Re-check every condition against the CURRENT world (D-E4-4).
///
/// This is the difference between "the seller said the goods are in the vault" and
/// "the goods are in the vault": an attestation is a durable record of a claim, and
/// settlement is the moment the claim has to still be true. A failure leaves the
/// contract `ReadyToSettle` for the sweep to retry — nothing moves.
pub fn reverify(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> Result<(), ContractError> {
    if !c.consent_intact() {
        return Err(ContractError::ConsentMismatch);
    }
    if c.satisfied.len() != c.consent.conditions.len() {
        return Err(ContractError::Infeasible("satisfaction vector does not match the conditions"));
    }
    let clocks = Clocks::new(env.now, env.ic_now);
    for (i, condition) in c.consent.conditions.iter().enumerate() {
        let ok = match condition {
            TypedCondition::Attestation { by } => match (&c.satisfied[i], by) {
                (Some(att), AttestorSet::Party(p)) => {
                    matches!(&att.by, AttestorRef::Principal(a) if a == p)
                }
                // A capability or staff attestor is re-qualified LIVE, exactly like
                // an approver: an attestor who lost the capability stops counting.
                (Some(att), set @ (AttestorSet::Capability { .. } | AttestorSet::Staff)) => {
                    matches!(&att.by, AttestorRef::Principal(a) if env.auth.authorize_attestor(a, set))
                }
                (Some(att), AttestorSet::Canon { .. }) => {
                    matches!(&att.by, AttestorRef::Canon { .. })
                }
                (None, _) => false,
            },
            TypedCondition::DepositInto { object, vault } => snapshot
                .vault(*vault)
                .is_some_and(|v| env.custody.holds(v, *object)),
            TypedCondition::TitleHeld { object, by } => snapshot.asset(*object).is_some_and(|a| {
                a.lifecycle == AssetLifecycle::Active && &a.title_owner == by
            }),
            // `None` (unknown) is unsatisfied, never a pass: the fact provider seam
            // is E4c and defaults to unknown, so a contract cannot settle on a fact
            // nobody can answer.
            TypedCondition::Fact { fact } => env.facts.resolve(fact) == Some(true),
            TypedCondition::Before { deadline } => clocks
                .now_on(deadline.clock, snapshot, 0)
                .is_some_and(|now| now <= deadline.at),
            TypedCondition::After { condition, delay, clock } => {
                match c.satisfied.get(*condition).and_then(|s| s.as_ref()) {
                    // Two stamps are comparable only on the SAME clock (doc 09 §2.5):
                    // an anchor stamped on wall time can never satisfy an IC delay.
                    Some(anchor) if anchor.clock == *clock => clocks
                        .now_on(*clock, snapshot, anchor.at)
                        .is_some_and(|now| {
                            i128::from(now) >= i128::from(anchor.at) + i128::from(*delay)
                        }),
                    _ => false,
                }
            }
        };
        if !ok {
            return Err(ContractError::ConditionFailed(i));
        }
    }
    Ok(())
}

/// Whether every condition is currently satisfied — the readiness question
/// [`reverify`] answers, as a predicate.
#[must_use]
pub fn conditions_met(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> bool {
    reverify(c, snapshot, env).is_ok()
}

// ---- the settlement and refund plans --------------------------------------

/// Build the settlement plan for a `ReadyToSettle` contract (§3.5).
///
/// Captures **every escrow hold with an explicit amount**, applies the deterministic
/// split, and carries the title ops and collateral releases the surface describes —
/// all as one plan, so the ordinary `validate`/`apply` path proves every invariant
/// and the whole settlement is one durable bundle or nothing. Cause
/// `Internal { kind: "contract", id }`, key `contract/{id}/settle`, so settling twice
/// is `AlreadyPosted` (scenario 16).
pub fn settlement_plan(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
) -> Result<EconomyPlan, ContractError> {
    if !matches!(c.state, ContractState::ReadyToSettle | ContractState::Active) {
        return Err(ContractError::State(c.id));
    }
    if !c.consent_intact() {
        return Err(ContractError::ConsentMismatch);
    }
    if !c.fully_accepted() {
        return Err(ContractError::NotAccepted);
    }
    check_offerable(c, snapshot)?;

    // Every escrow hold must still be live: a broken escrow is a dispute, never a
    // settlement that pays out less than the parties agreed.
    let mut hold_ops: Vec<HoldOp> = Vec::with_capacity(c.escrow.len());
    let mut expected_hold_revisions: Vec<HoldRevision> = Vec::with_capacity(c.escrow.len());
    let mut debits: Vec<RequestedLine> = Vec::with_capacity(c.escrow.len());
    let mut total: i128 = 0;
    for id in &c.escrow {
        let hold = snapshot.hold(*id).ok_or(ContractError::NoSuchHold(*id))?;
        if !hold.status.is_held() {
            return Err(ContractError::HoldNotHeld(*id));
        }
        total += hold.amount.widen();
        hold_ops.push(HoldOp::Capture { hold: *id, amount: hold.amount });
        expected_hold_revisions.push(HoldRevision { hold: *id, revision: hold.revision });
        let debit = hold.amount.checked_neg().ok_or(ContractError::Overflow)?;
        debits.push(RequestedLine {
            account: hold.account,
            delta: debit,
            memo: SmolStr::new(format!("contract {} escrow", c.id)),
        });
    }
    let total = Amount::from_widened(total).ok_or(ContractError::Overflow)?;
    let payouts = apply_split(total, &c.consent.terms)?;

    let mut lines = debits;
    for (account, amount) in payouts {
        lines.push(RequestedLine {
            account,
            delta: amount,
            memo: SmolStr::new(format!("contract {} payout", c.id)),
        });
    }

    // Deliverables. A `DepositInto` delivers BEFORE settlement (its condition
    // re-verifies it and its custody lock lifts when the contract leaves the live
    // states), and an `Obligation` template is created by E4b2 as a record, not as a
    // plan op — so only title moves and collateral releases ride the plan.
    let mut asset_ops: Vec<AssetOp> = Vec::new();
    for d in &c.consent.terms.deliverables {
        match d {
            Deliverable::TitleTransfer { object, to, .. } => {
                asset_ops.push(AssetOp::TransferTitle {
                    object: *object,
                    to: to.clone(),
                    to_scope: c.consent.scope.filter(|_| matches!(to, PrincipalId::Organization(_))),
                });
            }
            Deliverable::Collateral { object, .. } => {
                asset_ops.push(AssetOp::Release {
                    object: *object,
                    encumbrance: Encumbrance::Collateral { contract: c.id },
                });
            }
            Deliverable::DepositInto { .. } | Deliverable::Obligation { .. } => {}
        }
    }

    let mut plan = EconomyPlan::new(
        settle_key(c.id),
        c.issuer.clone(),
        EconomicCause::Internal { kind: SmolStr::new_static("contract"), id: c.id.0 },
        lines,
    );
    plan.description = format!("settlement of contract {}", c.id);
    plan.visibility = c.consent.visibility;
    plan.asset_ops = asset_ops;
    plan.hold_ops = hold_ops;
    plan.expected_hold_revisions = expected_hold_revisions;
    Ok(plan)
}

/// Build the refund plan: release every escrow hold and nothing else. Effect-only
/// (legal under invariant 10), key `contract/{id}/refund`.
#[must_use]
pub fn refund_plan(c: &EconomicContract) -> EconomyPlan {
    let mut plan = EconomyPlan::new(
        refund_key(c.id),
        c.issuer.clone(),
        EconomicCause::Internal { kind: SmolStr::new_static("contract"), id: c.id.0 },
        Vec::new(),
    );
    plan.description = format!("refund of contract {}", c.id);
    plan.visibility = c.consent.visibility;
    plan.hold_ops = c.escrow.iter().map(|hold| HoldOp::Release { hold: *hold }).collect();
    plan
}

// ---- ContractAuth ---------------------------------------------------------

/// The [`crate::AuthEval`] a contract's own acceptances constitute (D-E4-4).
///
/// It authorizes **exactly** the surface: each escrow capture up to that hold's
/// amount, each debit up to what is escrowed on that account, each credit only to a
/// declared payee, each title transfer whose source *and* destination party accepted
/// (D-E3-12), and each collateral release. Everything else — an extra line, an
/// unlisted object, a capture over the hold's amount, a vault op, an approver — is
/// `false`. No authority is stored anywhere: this value is built from the live record
/// at settlement and thrown away.
pub struct ContractAuth<'a> {
    contract: &'a EconomicContract,
    escrowed: BTreeMap<AccountId, Amount>,
}

impl<'a> ContractAuth<'a> {
    /// Derive the authority from a live contract and the current snapshot.
    #[must_use]
    pub fn new(contract: &'a EconomicContract, snapshot: &EconomySnapshot) -> ContractAuth<'a> {
        let escrowed = contract.escrowed_by_account(snapshot);
        ContractAuth { contract, escrowed }
    }
    fn is_payee(&self, account: AccountId) -> bool {
        self.contract.consent.terms.remainder_to == account
            || self.contract.consent.terms.split.iter().any(|s| s.to == account)
            || matches!(self.contract.consent.terms.on_expiry, Expiry::Forfeit { to } if to == account)
    }
}

impl crate::engine::AuthEval for ContractAuth<'_> {
    fn authorize(&self, _actor: &PrincipalId, account: &crate::record::Account, delta: Amount) -> bool {
        if !self.contract.fully_accepted() {
            return false;
        }
        if delta.is_negative() {
            // A debit may only take what this contract actually escrowed there.
            let Some(escrowed) = self.escrowed.get(&account.id) else { return false };
            return delta.checked_neg().is_some_and(|d| d <= *escrowed);
        }
        self.is_payee(account.id)
    }

    fn authorize_hold(&self, _actor: &PrincipalId, hold: &crate::record::Hold, op: &HoldOp) -> bool {
        if !self.contract.fully_accepted() || !self.contract.escrow.contains(&hold.id) {
            return false;
        }
        match op {
            HoldOp::Capture { hold: id, amount } => {
                *id == hold.id && !amount.is_negative() && !amount.is_zero() && *amount <= hold.amount
            }
            HoldOp::Release { hold: id } => *id == hold.id,
            HoldOp::Place { .. } | HoldOp::Expire { .. } => false,
        }
    }

    fn authorize_asset(
        &self,
        _actor: &PrincipalId,
        asset: Option<&crate::record::EconomicAsset>,
        op: &AssetOp,
    ) -> bool {
        if !self.contract.fully_accepted() {
            return false;
        }
        let Some(asset) = asset else { return false };
        match op {
            AssetOp::TransferTitle { object, to, .. } => {
                self.contract.consent.terms.deliverables.iter().any(|d| {
                    matches!(d, Deliverable::TitleTransfer { object: o, from, to: t }
                        if o == object
                            && t == to
                            && from == &asset.title_owner
                            // D-E3-12: BOTH sides accepted, or a title lands on
                            // someone who never agreed to its liabilities.
                            && self.contract.accepted_by(from)
                            && self.contract.accepted_by(t))
                })
            }
            // Placing and releasing a contract-backed claim are the same question:
            // is this MY claim, on an object my surface pledges, whose owner accepted?
            AssetOp::Encumber { object, encumbrance } | AssetOp::Release { object, encumbrance } => {
                let mine = match encumbrance {
                    Encumbrance::Collateral { contract } | Encumbrance::Lease { contract, .. } => {
                        *contract == self.contract.id
                    }
                    Encumbrance::Lien { .. } | Encumbrance::Disputed { .. } => false,
                };
                mine && self.contract.consent.terms.deliverables.iter().any(|d| {
                    matches!(d, Deliverable::Collateral { object: o, from }
                        if o == object && self.contract.accepted_by(from))
                })
            }
            _ => false,
        }
    }

    fn authorize_title_receipt(
        &self,
        _actor: &PrincipalId,
        to: &PrincipalId,
        _to_scope: Option<OrgScopeId>,
    ) -> bool {
        self.contract.fully_accepted()
            && self.contract.accepted_by(to)
            && self
                .contract
                .consent
                .terms
                .deliverables
                .iter()
                .any(|d| matches!(d, Deliverable::TitleTransfer { to: t, .. } if t == to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(split: Vec<PayoutShare>, remainder_to: AccountId) -> ContractTerms {
        ContractTerms {
            consideration: Vec::new(),
            deliverables: Vec::new(),
            split,
            remainder_to,
            on_expiry: Expiry::Refund,
        }
    }

    #[test]
    fn permille_shares_floor_and_the_remainder_is_deterministic() {
        let t = terms(
            vec![
                PayoutShare { to: AccountId(1), share: Share::Permille(333) },
                PayoutShare { to: AccountId(2), share: Share::Permille(333) },
                PayoutShare { to: AccountId(3), share: Share::Permille(333) },
            ],
            AccountId(9),
        );
        let out = apply_split(Amount::from_minor(100), &t).unwrap();
        assert_eq!(
            out,
            vec![
                (AccountId(1), Amount::from_minor(33)),
                (AccountId(2), Amount::from_minor(33)),
                (AccountId(3), Amount::from_minor(33)),
                (AccountId(9), Amount::from_minor(1)),
            ]
        );
    }

    #[test]
    fn exact_shares_come_first_and_an_oversized_one_is_infeasible() {
        let t = terms(
            vec![
                PayoutShare { to: AccountId(1), share: Share::Exact(Amount::from_minor(40)) },
                PayoutShare { to: AccountId(2), share: Share::Permille(500) },
            ],
            AccountId(9),
        );
        let out = apply_split(Amount::from_minor(100), &t).unwrap();
        assert_eq!(out, vec![
            (AccountId(1), Amount::from_minor(40)),
            (AccountId(2), Amount::from_minor(30)),
            (AccountId(9), Amount::from_minor(30)),
        ]);
        let bad = terms(
            vec![PayoutShare { to: AccountId(1), share: Share::Exact(Amount::from_minor(500)) }],
            AccountId(9),
        );
        assert_eq!(apply_split(Amount::from_minor(100), &bad), Err(ContractError::SplitInfeasible));
    }
}
