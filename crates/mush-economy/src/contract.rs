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
use crate::plan::{
    AssetOp, CustodyOp, EconomyPlan, HoldOp, HoldRevision, NamedResource, RequestedLine,
};
use crate::record::{
    AccountStatus, AssetLifecycle, ContractStateKind, EconomicCause, EconomicVisibility,
    Encumbrance, ExternalRef, Hold, HoldPurpose, IdempotencyKey,
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
/// Maximum outcomes in one outcome table (§3.5's E6 extension, 08e answer 8). The
/// defaults are `failure`/`partial`/`success`, but the list is **data**, never
/// hard-coded.
pub const MAX_OUTCOMES: usize = 8;
/// Maximum agreements recorded on one dispute (one per party, §9's `MAX_PARTIES`).
pub const MAX_AGREEMENTS: usize = MAX_PARTIES;
/// Maximum bytes of a contract's human label.
pub const MAX_CONTRACT_NAME_BYTES: usize = 64;
/// Maximum bytes of **one outcome row's** name (E6a verify fold, D2).
///
/// A row name is frozen into the consent surface, copied into every projection a
/// ruleset builds over it, and broadcast by every notice render — so it is a stored
/// string, and this crate does not have uncapped stored strings. Enforced in
/// [`check_structure`], which is what `Offer` runs and what boot re-runs, so **no**
/// producer — command, seam, or persisted document — can freeze an oversized row.
pub const MAX_OUTCOME_NAME_BYTES: usize = 64;
/// How many times one contract may be amended (E4b2).
///
/// Each amendment refunds every acceptance and starts the collection again, and each
/// one advances the **generation** every deterministic key carries, so the bound is
/// really on how many key generations one contract id can own. Thirty-two is far more
/// renegotiation than any table wants and short of anything that could look like a
/// key-space attack.
pub const MAX_AMENDMENTS: u32 = 32;
/// Maximum deposits one contract records for return at refund. The module admits
/// **one** custody op per plan, so a refund can only ever return one object per
/// post; the cap is the same, stated where the record is bounded.
pub const MAX_DEPOSITS: usize = 1;
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
/// The **default** for how long a live contract may go with no condition progress at
/// all before the sweep pages a human (§9, E4c verify fold E9) — since E6a step 1
/// (DL-08) the live value is `[economy].stale_contract_age_secs`
/// ([`crate::EconomyConfig::stale_contract_age_secs`]), and a contract whose own
/// deadline has not lapsed is never paged for staleness.
///
/// A contract waiting forever was silent: escrow reserved, nobody told, no deadline
/// (a deadline is optional), and the only trace a `@contract/list` nobody runs. The
/// page is level 2 and deduped like every other, and it recommends
/// `@contract/resolve` — the command that can actually end it.
pub const STALE_CONTRACT_AGE_SECS: i64 = 7 * 86_400;
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

/// How one outcome treats an expense advance (08e answer 7/8).
///
/// **Declarative in E4b2**: it is frozen inside the consent hash, so the parties
/// agreed to it and E6a cannot change it after acceptance, but nothing here
/// synthesises a money line from it — an outcome's `payouts` must already say where
/// the advance ends up. E6a's vendor/expense flow is the consumer (§5.1's handoff).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpenseTreatment {
    /// No expense advance is part of this contract.
    #[default]
    None,
    /// The advance stays with whoever holds it, whatever this outcome pays.
    Retained,
    /// The advance comes back — this outcome's `payouts` must name the payer's
    /// account for it explicitly.
    Returned,
    /// Receipts decide, once attestations are mature enough to carry them (08e
    /// answer 7: "prefer a fixed expense advance first").
    Reimbursed,
}

/// One outcome of a data-driven outcome table (08e answers 7–8, §3.5).
///
/// E4a's single [`ContractTerms::split`] is the **degenerate one-outcome case**: a
/// contract with an empty `outcomes` vector settles exactly as it did in E4a. With
/// outcomes present, the conditions named by some outcome's `when` become *outcome*
/// conditions — the ones that decide *which* result happened, and which are
/// therefore mutually exclusive rather than all-required — while every other
/// condition stays a **gating** condition that must hold whatever the outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeTerms {
    /// The outcome's name (`success`, `partial`, `failure` are conventions, never
    /// hard-coded).
    pub name: SmolStr,
    /// The condition indexes that must ALL be satisfied for this outcome to be the
    /// one that pays (≤ [`MAX_CONDITIONS`]).
    pub when: Vec<usize>,
    /// Exactly what this outcome pays, with **no implicit remainder** (08e answer
    /// 7): the payouts must sum to the escrowed total exactly.
    pub payouts: Vec<PayoutShare>,
    /// How this outcome treats an expense advance (declarative — see
    /// [`ExpenseTreatment`]).
    #[serde(default)]
    pub expense_treatment: ExpenseTreatment,
    /// The indexes of the `deliverables` this outcome causes — a failed run does not
    /// move the title a successful one would. Deliverables this outcome does not
    /// name simply do not execute; `Collateral` releases are **not** listed here,
    /// because a pledge has to come back whatever happened.
    #[serde(default)]
    pub asset_consequences: Vec<usize>,
}

/// The money-and-goods half of the consent surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractTerms {
    /// What each party pays, escrowed at their accept.
    pub consideration: Vec<Consideration>,
    /// What is delivered (≤ [`MAX_DELIVERABLES`]).
    pub deliverables: Vec<Deliverable>,
    /// How the escrowed total is split when there is **one** result (≤
    /// [`MAX_SHARES`]). Ignored when `outcomes` is non-empty.
    pub split: Vec<PayoutShare>,
    /// Where the floored remainder lands (scenario 17). Ignored when `outcomes` is
    /// non-empty — an outcome has no implicit remainder.
    pub remainder_to: AccountId,
    /// What an unsettled deadline does.
    pub on_expiry: Expiry,
    /// The outcome table (≤ [`MAX_OUTCOMES`]). Empty is E4a's single split.
    #[serde(default)]
    pub outcomes: Vec<OutcomeTerms>,
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
        /// The canon event kind. It must be a **producer-declared outcome kind**
        /// ([`CanonOutcomeKinds`], E4c verify fold E1): a lifecycle record any
        /// participant can emit — `scene.concluded` — is not an outcome, and a payee
        /// who can produce one alone could manufacture their own settlement.
        kind: SmolStr,
        /// The specific source it must name. **Mandatory** since the E4c verify fold
        /// (E2): `None` was a wildcard over every record of that kind, which is the
        /// broad match E5's owner answer 20 exists to refuse. The field keeps its
        /// `Option` shape so records written before the fold still deserialize —
        /// [`check_structure`] refuses them.
        source: Option<SmolStr>,
    },
}

/// The wiring-frozen set of canon kinds that count as a **producer-declared
/// outcome** (E4c verify fold, E1; 08e answer 6).
///
/// `mush_canon` records *lifecycle* as readily as it records outcomes:
/// `scene.concluded` says a scene ended, and any participant can end a scene. A
/// contract whose settlement cites one is a contract its payee can settle alone. So
/// a canon attestor set names a kind from this table or it is refused at `Offer`,
/// and the table is composition-root data ([`crate::EconomyModule::with_canon_outcome_kinds`])
/// — never something a command, a document or a grant can add to. Since E6b the
/// server registers exactly one kind, `scene.outcome`, which `mush-scenes` writes
/// only from `+scene/outcome` under a **staff-conferred sealer** — never a
/// self-granted facilitator, which is what any player becomes by starting a scene
/// (M14, D-E6-7). `scene.concluded` is deliberately never registered.
pub type CanonOutcomeKinds = std::collections::BTreeSet<SmolStr>;

/// Who actually attested.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttestorRef {
    /// A principal (a party, a capability holder, staff).
    Principal(PrincipalId),
    /// A canon record.
    ///
    /// It carries the `kind` and `source` it was recorded under, not just its id,
    /// because [`reverify`] has to answer "does this attestation satisfy *this*
    /// [`AttestorSet::Canon`]?" and the pure engine cannot read the canon store to
    /// find out (E4a verify fold, finding 5). Without them any canon reference at
    /// all satisfied any canon condition — a bar brawl would have discharged a
    /// smuggling contract.
    Canon {
        /// `canon:{id}`.
        canon: SmolStr,
        /// The canon event kind the record carries.
        kind: SmolStr,
        /// The specific front/opportunity/operation it names.
        source: SmolStr,
    },
    /// An external fact provider's answer.
    Fact(ExternalRef),
    /// The clock itself (a `Before`/`After` condition).
    Clock,
    /// **Observed holding** — the stamp a sweep or an attest step writes the first
    /// time it finds a *deterministic* condition (`Fact`/`DepositInto`/`TitleHeld`/
    /// `Before`/`After`) true (E4c verify fold, E5).
    ///
    /// It is not evidence and confers no authority: the condition is re-evaluated
    /// live at settlement exactly as before. Its only job is to give an
    /// [`TypedCondition::After`] an anchor to count from, because before the fold a
    /// deterministic condition never filled its slot and "pay 60 s after the job is
    /// approved" could therefore never mature.
    Observed {
        /// The clock `Attestation::at` is expressed on.
        clock: EconomicClock,
    },
    /// A **staff resolution** of a disputed contract (`@contract/resolve <id>=settle`,
    /// E4c verify fold, E4b).
    ///
    /// It satisfies whatever attestor set the condition names, canon sets included,
    /// which is exactly why it is a separate variant and not a `Principal`: the
    /// override is visible on the document, carries the staffer and their reason
    /// forever, and can never be confused with an ordinary party or capability
    /// attestation. It is the level-2 answer to E4's edge-triggered evidence — a
    /// canon record that landed while the economy was frozen, a provider that was
    /// never wired — and it moves no deterministic condition: those are still
    /// re-verified at settlement and still refuse.
    StaffOverride {
        /// The staffer who resolved it.
        by: PrincipalId,
        /// The reason they gave.
        reason: SmolStr,
    },
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
    /// canon (D-E4-12, E15); an [`EconomicVisibility::Audience`] list is the T6
    /// class that discloses to eligible defenders without publishing.
    pub visibility: EconomicVisibility,
    /// The org compartment, if this is an org contract.
    pub scope: Option<OrgScopeId>,
    /// **The named adjudicator** (08e answer 6): the authority whose word settles a
    /// contested condition, named *before* acceptance so runners consent to it when
    /// they accept. When it is `Some`, it **replaces** every condition's own `by` as
    /// the attestor set that counts (`outcome_conditions`): an adjudicated condition
    /// is not something a party can discharge by asserting it, and the condition's
    /// own set is not consulted at all (look-back fold, the stale "as well as").
    ///
    /// It lives in the consent surface rather than beside the other roles because it
    /// is consented *authority*, not attribution: changing it after `Offer` must
    /// void every acceptance.
    #[serde(default)]
    pub adjudicator: Option<AttestorSet>,
    /// **The permitted evidence sources** (08e answer 6). Empty admits every
    /// condition kind; a non-empty list is exhaustive, and `check_offerable` refuses
    /// a condition of any other kind.
    #[serde(default)]
    pub evidence: Vec<EvidenceKind>,
}

/// A condition kind, as the consent surface's permitted-evidence list names it
/// (08e answer 6). Deliberately a separate enum from [`TypedCondition`]: the list
/// says which *kinds* of proof the parties agreed to, not which proofs exist.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EvidenceKind {
    /// An explicit attestation.
    Attestation,
    /// A journaled deposit into a named vault.
    DepositInto,
    /// Journal-authority title state.
    TitleHeld,
    /// An external provider's fact.
    Fact,
    /// An absolute deadline.
    Before,
    /// A delay after another condition.
    After,
}

impl TypedCondition {
    /// Which [`EvidenceKind`] this condition is.
    #[must_use]
    pub fn evidence_kind(&self) -> EvidenceKind {
        match self {
            TypedCondition::Attestation { .. } => EvidenceKind::Attestation,
            TypedCondition::DepositInto { .. } => EvidenceKind::DepositInto,
            TypedCondition::TitleHeld { .. } => EvidenceKind::TitleHeld,
            TypedCondition::Fact { .. } => EvidenceKind::Fact,
            TypedCondition::Before { .. } => EvidenceKind::Before,
            TypedCondition::After { .. } => EvidenceKind::After,
        }
    }
}

/// The **separate roles** a contract records (08e answer 22, §3.5).
///
/// "A Johnson is the visible representative, not necessarily the payer or author"
/// (08e answer 4). They may coincide — in the simple two-party sale they all are the
/// issuer — but the model must not assume it, so each is its own optional field and
/// none of them is authority: authority is the acceptance, the debit right on the
/// named account, and (for evidence) the adjudicator in the consent surface.
///
/// The fifth role, the **adjudicator**, lives in [`ConsentSurface`] instead, because
/// it is a power the parties consent to rather than a name they are told.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRoles {
    /// Who wrote it (an ST, a campaign system, a player).
    pub author: Option<PrincipalId>,
    /// **The accountable escrow principal** — whose account funds the consideration.
    pub payer: Option<PrincipalId>,
    /// The visible representative (the Johnson).
    pub representative: Option<PrincipalId>,
    /// Whoever is running the table for it.
    pub facilitator: Option<PrincipalId>,
}

/// One party's recorded position on how a dispute should end (level 0, §4.6):
/// unanimous agreement resolves it without staff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyAgreement {
    /// Who agreed.
    pub party: PrincipalId,
    /// What they agreed to.
    pub resolution: Resolution,
    /// When (wall clock).
    pub at: i64,
}

/// How a dispute ends.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// Settle: re-verify and pay out.
    Settle,
    /// Refund: release every escrow hold and return every deposit.
    Refund,
}

/// Who put an object into a vault under this contract's `DepositInto`, taken from
/// the **journal's** last-custody map (never from a claim).
///
/// It exists so a refund can put the object back where it came from: the custody
/// lock has no exception (E4a verify fold, finding 2), so the only way out for a
/// deposit is the contract's own unwind, and the unwind has to know whose it was.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositRecord {
    /// The deposited object.
    pub object: ObjId,
    /// The vault it went into.
    pub vault: VaultId,
    /// The principal whose post deposited it (the transaction's actor).
    pub deposited_by: PrincipalId,
    /// When (wall clock).
    pub at: i64,
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

/// Which terminal transition a contract is in the middle of (E4b2 verify fold,
/// M2+M3).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingKind {
    /// A cancellation (staff, the issuer, an agreed refund, an amendment).
    Cancel,
    /// A deadline that refunds.
    Expire,
    /// A deadline whose `on_expiry` is [`Expiry::Forfeit`].
    Forfeit,
}

impl PendingKind {
    /// The state this transition lands in once its plan posts.
    #[must_use]
    pub fn target(self) -> ContractState {
        match self {
            PendingKind::Cancel => ContractState::Cancelled,
            PendingKind::Expire | PendingKind::Forfeit => ContractState::Expired,
        }
    }
    /// The deterministic verb the transition's plan posts under.
    #[must_use]
    pub fn verb(self) -> &'static str {
        match self {
            PendingKind::Cancel | PendingKind::Expire => "refund",
            PendingKind::Forfeit => "forfeit",
        }
    }
}

/// A terminal transition that has been decided but whose plan has not posted yet
/// (E4b2 verify fold, M2+M3).
///
/// **The rule the whole subsystem turns on**: a terminal transition is a post under
/// the contract's deterministic key at its generation, *followed by* the state write.
/// While the post has not succeeded the contract stays live and carries this record,
/// so the escrow is still recoverable, the sweep retries it, and boot can finish it by
/// looking the same key up in the idempotency index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingTerminal {
    /// Which transition.
    pub kind: PendingKind,
    /// How many times its plan has been refused.
    pub attempts: u8,
    /// When it was first attempted (wall clock).
    pub since: i64,
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
    /// A short human label (≤ [`MAX_CONTRACT_NAME_BYTES`]). **Attribution, not
    /// consent**: it is outside the consent surface, so renaming a draft does not
    /// void anybody's acceptance, and a name can never be the reason money moves.
    #[serde(default)]
    pub name: SmolStr,
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
    /// When it was drafted (wall clock) — the `DRAFT_TTL_SECS` GC clock.
    #[serde(default)]
    pub created_at: i64,
    /// How many times the consent surface has been re-opened by an amendment
    /// (08e answer 5). It is the **generation** of every deterministic key this
    /// contract posts under: an amended contract can be refunded a second time, and
    /// `contract/{id}/refund` would otherwise resolve to the FIRST refund and give
    /// nothing back — an idempotency HIT quietly stranding the new escrow.
    #[serde(default)]
    pub amendments: u32,
    /// When it entered `Disputed` (wall clock) — the `DISPUTE_TTL_SECS` clock the
    /// sweep pages on. `None` on a contract that has never been disputed.
    #[serde(default)]
    pub disputed_at: Option<i64>,
    /// The chronicle line this contract's settlement became, when it became one
    /// (E4c verify fold, V2+V3+V5) — `canon:{id}`, written by
    /// [`crate::mark_canon_published`] after the bridge's commit **succeeded**.
    ///
    /// It replaces the bridge's RAM-tail replay guard with a **durable** one, and it
    /// is what makes publication more than best-effort: a settlement of a `Public`
    /// contract whose marker is still unset after a sweep period is re-announced, so
    /// a canon commit that failed (an exhausted id allocator, an unsafe append) is
    /// retried, and a settlement finished by boot's `finished_transition` — which
    /// emits no event at all — is chronicled at last.
    #[serde(default)]
    pub canon_published: Option<SmolStr>,
    /// Whether the roster is frozen (08e answers 5/7). The **first acceptance**
    /// locks it: a party who has funded escrow agreed to a specific set of
    /// counterparties and a specific set of shares, and adding a runner afterwards
    /// dilutes the split they signed. A later substitution is an amendment, which
    /// re-opens the surface and re-collects every acceptance.
    #[serde(default)]
    pub roster_locked: bool,
    /// What was deposited under this contract, and by whom (≤ [`MAX_DEPOSITS`]).
    #[serde(default)]
    pub deposits: Vec<DepositRecord>,
    /// Recorded party positions on a dispute (≤ [`MAX_AGREEMENTS`]).
    #[serde(default)]
    pub agreements: Vec<PartyAgreement>,
    /// A terminal transition whose plan has not posted yet (E4b2 verify fold,
    /// M2+M3). While it is `Some` the contract is still **live**: its escrow is
    /// recoverable, `@contract/amend` is refused, the sweep retries the transition,
    /// and boot finishes it from the idempotency index.
    #[serde(default)]
    pub pending: Option<PendingTerminal>,
    /// The separate roles (08e answer 22). Attribution, never authority.
    #[serde(default)]
    pub roles: ContractRoles,
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
    /// A referenced object carries no title this contract can pledge or move.
    #[error("no such asset: #{0}")]
    NoSuchAsset(ObjId),
    /// A referenced escrow hold does not exist.
    #[error("no such hold: {0}")]
    NoSuchHold(HoldId),
    /// An escrow hold is no longer `Held` — the escrow has been broken.
    #[error("hold {0} is no longer held")]
    HoldNotHeld(HoldId),
    /// A [`Consideration`] in the consent surface has no qualifying escrow hold
    /// behind it (E4a verify fold, finding 3): either it was never funded, or the
    /// hold that was supposed to fund it is gone, or what `escrow` lists is not
    /// this contract's escrow at all.
    #[error("the escrow does not cover every consideration")]
    EscrowIncomplete,
    /// A refund was asked of a contract that has not been cancelled or expired
    /// (E4a verify fold, finding 6): a refund releases escrow, so on a live or
    /// settled contract it is an unwind, not a refund.
    #[error("contract {0} is not cancelled or expired")]
    WrongState(ContractId),
    /// A `Before`/`After` condition on an [`EconomicClock::Manual`] clock (E4a
    /// verify fold, finding 10). A manual clock's position is carried by the record
    /// that names it, and a condition has no such record — so the deadline would be
    /// read against whatever the caller happened to pass.
    #[error("condition {0} names a clock this contract cannot resolve")]
    UnsupportedClock(usize),
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
    /// Two (or more) outcomes' `when` conditions are all satisfied, so the contract
    /// does not say what happened — a dispute, never a guess (§3.5).
    #[error("two outcomes are satisfied at once; the contract does not say which happened")]
    AmbiguousOutcome,
    /// No outcome's `when` conditions are satisfied yet.
    #[error("no outcome is satisfied")]
    NoOutcome,
    /// A condition kind the consent surface's permitted-evidence list does not admit.
    #[error("condition {0} is not a permitted kind of evidence for this contract")]
    EvidenceNotPermitted(usize),
    /// Checked arithmetic overflowed.
    #[error("arithmetic overflow")]
    Overflow,
    /// An acceptance that survived an amendment (E6a step 1, consent by authorship —
    /// DL-01 Option A′) is no longer backed: a consideration its party owes has no
    /// `Held` escrow hold behind it, so the acceptance cannot be re-stamped onto the
    /// amended surface and the offer is refused.
    #[error(
        "a surviving acceptance is no longer backed by escrow for every consideration its party owes"
    )]
    AcceptanceUnbacked,
    /// The post a contract operation had to make — an acceptance's escrow, an
    /// amendment's partial refund — was refused by the engine (E6a step 1). Carries
    /// the engine's own refusal so the command path renders it behind the same
    /// existence-hiding rule a money verb would.
    #[error("{0}")]
    Refused(crate::plan::EconomyError),
    /// The actor has no standing over the contract for this operation — it does
    /// not exist, they may not view it, they are not a party (an accept), or they
    /// are not its issuer (an amendment). **One answer for all four** (§4.5): the
    /// programmatic seams disclose exactly what `@contract/*` does, which is the
    /// nonexistent-id text.
    #[error("no such contract")]
    NoStanding,
    /// The party has already accepted the surface as it stands.
    #[error("the contract is already accepted on that party's behalf")]
    AlreadyAccepted,
    /// A `System` principal was named as a party, or a `System`-owned account as a
    /// consideration's source (E6a step 1, M4 / D-E6-4). Nobody acts for a `System`
    /// principal, so such a contract could be offered and never accepted — a zombie
    /// holding an offer open forever. The command path renders it as the account's
    /// own existence-hiding text, never a faucet-shaped message.
    #[error("a system principal cannot be a party")]
    SystemParty,
}

impl ContractError {
    /// Every resource this refusal **names** (E4b2 verify fold, A2).
    ///
    /// The existence-hiding rule (§4.5) is one rule, and it has to reach a contract
    /// refusal the same way it reaches an [`crate::EconomyError`]: an unviewable vault
    /// and a nonexistent one must render identically, or `@contract/offer` becomes a
    /// probe for other people's wallets, vaults and goods. Everything else here is a
    /// shape or cap complaint about the caller's own draft and keeps its own text.
    #[must_use]
    pub fn named_resources(&self) -> Vec<NamedResource> {
        match self {
            ContractError::NoSuchAccount(id) => vec![NamedResource::Account(*id)],
            ContractError::NoSuchVault(id) => vec![NamedResource::Vault(*id)],
            ContractError::NoSuchAsset(o) => vec![NamedResource::Asset(*o)],
            ContractError::NoSuchHold(h) | ContractError::HoldNotHeld(h) => {
                vec![NamedResource::Hold(*h)]
            }
            ContractError::State(id) | ContractError::WrongState(id) => {
                vec![NamedResource::Contract(*id)]
            }
            ContractError::Refused(inner) => inner.named_resources(),
            _ => Vec::new(),
        }
    }

    /// Whether this refusal is a **policy answer about the resources the contract
    /// names** rather than a shape complaint about the draft.
    ///
    /// `InstrumentMismatch` says "one of the accounts you named holds a different
    /// instrument" and `EscrowIncomplete` says "one of the holds behind your
    /// considerations is gone" — both are facts about accounts and holds the caller
    /// may not be allowed to see, so they render behind the hiding rule against the
    /// contract's own referenced resources. A cap or an infeasible split is the
    /// caller's own draft and keeps its text.
    #[must_use]
    pub fn concerns_named_subjects(&self) -> bool {
        match self {
            ContractError::InstrumentMismatch
            | ContractError::EscrowIncomplete
            | ContractError::NotAccepted
            | ContractError::AcceptanceUnbacked => true,
            ContractError::Refused(inner) => inner.concerns_named_subjects(),
            _ => false,
        }
    }

    /// Every resource a contract's consent surface names — the `also` list a refusal
    /// that `concerns_named_subjects` is rendered against.
    #[must_use]
    pub fn surface_resources(c: &EconomicContract) -> Vec<NamedResource> {
        let terms = &c.consent.terms;
        let mut out: Vec<NamedResource> = Vec::new();
        for con in &terms.consideration {
            out.push(NamedResource::Account(con.account));
        }
        for s in &terms.split {
            out.push(NamedResource::Account(s.to));
        }
        for o in &terms.outcomes {
            for s in &o.payouts {
                out.push(NamedResource::Account(s.to));
            }
        }
        out.push(NamedResource::Account(terms.remainder_to));
        if let Expiry::Forfeit { to } = terms.on_expiry {
            out.push(NamedResource::Account(to));
        }
        for d in &terms.deliverables {
            match d {
                Deliverable::DepositInto { object, vault } => {
                    out.push(NamedResource::Asset(*object));
                    out.push(NamedResource::Vault(*vault));
                }
                Deliverable::TitleTransfer { object, .. }
                | Deliverable::Collateral { object, .. } => out.push(NamedResource::Asset(*object)),
                Deliverable::Obligation { template } => {
                    out.push(NamedResource::Account(template.debit_account));
                    out.push(NamedResource::Account(template.credit_account));
                }
            }
        }
        for cond in &c.consent.conditions {
            match cond {
                TypedCondition::DepositInto { object, vault } => {
                    out.push(NamedResource::Asset(*object));
                    out.push(NamedResource::Vault(*vault));
                }
                TypedCondition::TitleHeld { object, .. } => out.push(NamedResource::Asset(*object)),
                _ => {}
            }
        }
        for h in &c.escrow {
            out.push(NamedResource::Hold(*h));
        }
        out
    }
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

// ---- E6b: B4 — DL-11's vendor accept --------------------------------------

/// What a wiring-frozen party capability permits (E6b, DL-11 / M15).
///
/// Deliberately **one** variant. The problem it solves is narrow — a staff-built
/// NPC vendor org has no `acts_for` holder, so its side of a deal cannot be
/// accepted at all without a staffer — and every wider power (spending, drafting,
/// attesting, disputing) already has a capability in the E3 vocabulary that an org
/// grants to real people.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PartyCap {
    /// This principal's **acceptance** of a contract it is already a party to may
    /// be performed programmatically. It confers no money authority: the escrow
    /// still posts under the principal's own debit authority and still refuses
    /// when the account is short.
    Accept,
}

/// The owned canonical token of a principal — the key of the wiring-frozen
/// party-capability table, because `PrincipalId` is `Hash`-but-not-`Ord` (E6b).
#[must_use]
pub fn principal_token(p: &PrincipalId) -> SmolStr {
    match p {
        PrincipalId::Object(o) => SmolStr::new(format!("obj:{}", o.0)),
        PrincipalId::Organization(g) => SmolStr::new(format!("org:{}", g.0)),
        PrincipalId::System(s) => SmolStr::new(format!("sys:{s}")),
    }
}

/// The canonical form of a visibility inside a consent hash: an audience list is
/// sorted by [`principal_key`] and deduplicated, so two orderings of one audience
/// are **one** consent surface and a party who accepted cannot be told afterwards
/// that the list "changed" because someone re-typed it.
///
/// Every other variant is returned unchanged, so the canonical bytes of `Parties`,
/// `Staff` and `Public` are exactly what they were before E4b2 existed.
#[must_use]
pub fn canonical_visibility(v: &EconomicVisibility) -> EconomicVisibility {
    match v {
        EconomicVisibility::Audience {
            members,
            announce_issuer,
        } => {
            let mut out: Vec<PrincipalId> = members.clone();
            out.sort_by(|a, b| principal_key(a).cmp(&principal_key(b)));
            out.dedup_by(|a, b| principal_key(a) == principal_key(b));
            EconomicVisibility::Audience {
                members: out,
                announce_issuer: *announce_issuer,
            }
        }
        other => other.clone(),
    }
}

#[derive(Serialize)]
struct CanonicalConsent<'a> {
    issuer: &'a PrincipalId,
    parties: Vec<&'a PrincipalId>,
    terms: &'a ContractTerms,
    conditions: &'a [TypedCondition],
    deadline: &'a Option<EconomicDeadline>,
    visibility: EconomicVisibility,
    scope: &'a Option<OrgScopeId>,
    /// E4b2: the adjudicator and the permitted evidence are part of what a runner
    /// consents to when they accept (08e answer 6), so they are hashed.
    adjudicator: &'a Option<AttestorSet>,
    evidence: &'a [EvidenceKind],
}

/// The canonical SHA-256 of a consent surface — **everything** an acceptance binds
/// to (B5): the issuer, parties, terms, conditions, deadline, visibility, and scope.
///
/// The `issuer` is in here because it is part of what a party agrees to (E4a verify
/// fold, finding 14a): who offered a contract decides who answers for it, who its
/// `MAX_OPEN_CONTRACTS_PER_ISSUER` budget belongs to, and — since `settlement_plan`
/// stamps the settlement's `actor` from it — whose name the ledger records. It lives
/// on [`EconomicContract`] rather than inside [`ConsentSurface`], so it is passed in.
///
/// Parties are sorted by [`principal_key`] so the hash is order-independent; every
/// other vector's order *is* consented content (`conditions` are indexed by
/// [`TypedCondition::After`], and the split's order decides who is floored first).
/// Deterministic for the same reason [`crate::plan_hash`] is: field-ordered structs,
/// no floats, no maps.
#[must_use]
pub fn consent_hash(issuer: &PrincipalId, consent: &ConsentSurface) -> [u8; 32] {
    let mut parties: Vec<&PrincipalId> = consent.parties.iter().collect();
    parties.sort_by(|a, b| principal_key(a).cmp(&principal_key(b)));
    let canonical = CanonicalConsent {
        issuer,
        parties,
        terms: &consent.terms,
        conditions: &consent.conditions,
        deadline: &consent.deadline,
        visibility: canonical_visibility(&consent.visibility),
        scope: &consent.scope,
        adjudicator: &consent.adjudicator,
        evidence: &consent.evidence,
    };
    let bytes = serde_json::to_vec(&canonical).expect("canonical consent serializes");
    Sha256::digest(&bytes).into()
}

/// One of a contract's deterministic idempotency keys, at a **generation** (§12's C8
/// list): `contract/{id}/{verb}` at generation 0, `contract/{id}/{verb}/{n}` after
/// `n` amendments.
///
/// Generation 0 is byte-identical to the key E4a specified, so every stored
/// idempotency record keeps resolving. It exists because an amendment genuinely
/// re-opens a contract: the same id can refund, activate and settle **again** on the
/// new terms, and a key that did not move would resolve to the first post and hand
/// back an `AlreadyPosted` for money that never moved the second time.
#[must_use]
pub fn contract_op_key(id: ContractId, verb: &str, generation: u32) -> IdempotencyKey {
    let key = if generation == 0 {
        format!("{}/{verb}", id.0)
    } else {
        format!("{}/{verb}/{generation}", id.0)
    };
    IdempotencyKey::new(SmolStr::new_static("contract"), SmolStr::new(key))
}
/// The deterministic idempotency key for a contract's settlement at generation 0:
/// `contract/{id}/settle`. Settling twice is `AlreadyPosted`.
#[must_use]
pub fn settle_key(id: ContractId) -> IdempotencyKey {
    contract_op_key(id, "settle", 0)
}
/// The deterministic idempotency key for a contract's refund at generation 0.
#[must_use]
pub fn refund_key(id: ContractId) -> IdempotencyKey {
    contract_op_key(id, "refund", 0)
}

impl EconomicContract {
    /// Whether the stored `consent_hash` still describes the consent surface. A
    /// mutation after `Offer` makes this `false` and every acceptance void.
    #[must_use]
    pub fn consent_intact(&self) -> bool {
        consent_hash(&self.issuer, &self.consent) == self.consent_hash
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
    /// The escrowed total per account, counting only **qualifying** holds
    /// ([`qualifying_escrow`]).
    #[must_use]
    pub fn escrowed_by_account(&self, snapshot: &EconomySnapshot) -> BTreeMap<AccountId, Amount> {
        let mut out: BTreeMap<AccountId, Amount> = BTreeMap::new();
        for hold in qualifying_escrow(self, snapshot).holds {
            let slot = out.entry(hold.account).or_insert(Amount::ZERO);
            *slot = slot.checked_add(hold.amount).unwrap_or(*slot);
        }
        out
    }
}

/// Which of a contract's listed escrow holds actually back its consent surface.
#[derive(Clone, Debug)]
pub struct QualifyingEscrow<'a> {
    /// The qualifying holds, in the order `escrow` lists them.
    pub holds: Vec<&'a Hold>,
    /// Whether **every** [`Consideration`] in the terms is matched by one of them —
    /// the precondition for a settlement that pays out what the parties agreed.
    pub complete: bool,
}

/// The subset of `c.escrow` that is real escrow for `c` (E4a verify fold, finding 3).
///
/// `escrow` is an ordinary mutable field on the contract record: whoever can edit a
/// contract can list *any* hold id in it, including a stranger's `Manual` hold. Left
/// untrusted, that one line would raise [`ContractAuth`]'s debit ceiling on the
/// stranger's account, let [`refund_plan`] release their money, and pad
/// [`settlement_plan`]'s payout total with funds nobody consented to move.
///
/// So a listed hold counts only when all three of these hold:
///
/// - its `purpose` is `Escrow { contract: c.id }` — it was placed **for this
///   contract**, which the accept path is the only thing that does;
/// - it is still `Held`;
/// - its `(account, amount)` matches a [`Consideration`] in the consent surface, and
///   each consideration is claimed by **at most one** hold (so two holds cannot both
///   stand for one agreed payment).
#[must_use]
pub fn qualifying_escrow<'a>(
    c: &EconomicContract,
    snapshot: &'a EconomySnapshot,
) -> QualifyingEscrow<'a> {
    let (claims, complete) = qualifying_claims(c, snapshot);
    QualifyingEscrow {
        holds: claims.into_iter().map(|(hold, _)| hold).collect(),
        complete,
    }
}

/// [`qualifying_escrow`] with the **consideration index each hold claims** — the
/// pairing E6a step 1's consent-by-authorship rules read (DL-01 Option A′): an
/// amendment releases only the holds backing *other* parties' considerations, and an
/// offer re-stamps a surviving acceptance only when every consideration its party
/// owes is claimed. The second element is `QualifyingEscrow::complete`.
#[must_use]
pub fn qualifying_claims<'a>(
    c: &EconomicContract,
    snapshot: &'a EconomySnapshot,
) -> (Vec<(&'a Hold, usize)>, bool) {
    let considerations = &c.consent.terms.consideration;
    let mut claimed = vec![false; considerations.len()];
    let mut claims: Vec<(&Hold, usize)> = Vec::with_capacity(c.escrow.len());
    for id in &c.escrow {
        let Some(hold) = snapshot.hold(*id) else {
            continue;
        };
        if !hold.status.is_held() {
            continue;
        }
        if hold.purpose != (HoldPurpose::Escrow { contract: c.id }) {
            continue;
        }
        let matched = considerations.iter().enumerate().position(|(i, con)| {
            !claimed[i] && con.account == hold.account && con.amount == hold.amount
        });
        let Some(i) = matched else { continue };
        claimed[i] = true;
        claims.push((hold, i));
    }
    (claims, claimed.iter().all(|c| *c))
}

/// Whether **every** consideration `party` owes on this surface is claimed by a
/// `Held` escrow hold (E6a step 1, A′): the precondition for a surviving acceptance
/// to be re-stamped at `Offer`. A party owing nothing is trivially backed.
#[must_use]
pub fn party_escrow_backed(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
    party: &PrincipalId,
) -> bool {
    let (claims, _) = qualifying_claims(c, snapshot);
    c.consent
        .terms
        .consideration
        .iter()
        .enumerate()
        .filter(|(_, con)| principal_key(&con.from) == principal_key(party))
        .all(|(i, _)| claims.iter().any(|(_, k)| *k == i))
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
pub fn apply_split(
    total: Amount,
    terms: &ContractTerms,
) -> Result<Vec<(AccountId, Amount)>, ContractError> {
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

/// Apply one outcome's payouts to `total` with **no implicit remainder** (08e
/// answer 7: "no unspecified remainder to the fixer").
///
/// [`Share::Exact`] first, then [`Share::Permille`] of what remains, floored — the
/// same arithmetic [`apply_split`] uses — and then the sum must equal `total`
/// exactly. A table whose shares leave a rounding crumb is [`ContractError::
/// SplitInfeasible`] at `Offer`, where a human can still fix it, rather than a
/// silent payment to whoever happened to be named `remainder_to`.
pub fn apply_outcome(
    total: Amount,
    payouts: &[PayoutShare],
) -> Result<Vec<(AccountId, Amount)>, ContractError> {
    let total_w = total.widen();
    if total_w < 0 {
        return Err(ContractError::SplitInfeasible);
    }
    let mut exact_sum: i128 = 0;
    let mut permille_sum: u64 = 0;
    for s in payouts {
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
    let mut out: Vec<(AccountId, Amount)> = Vec::with_capacity(payouts.len());
    let mut assigned: i128 = 0;
    for s in payouts {
        let amount = match s.share {
            Share::Exact(a) => a,
            Share::Permille(p) => {
                let v = remaining * i128::from(p) / i128::from(PERMILLE);
                Amount::from_widened(v).ok_or(ContractError::Overflow)?
            }
        };
        assigned += amount.widen();
        if !amount.is_zero() {
            out.push((s.to, amount));
        }
    }
    if assigned != total_w {
        return Err(ContractError::SplitInfeasible);
    }
    Ok(out)
}

// ---- the outcome table (08e answers 7-8) ----------------------------------

/// The condition indexes some outcome's `when` names — the conditions that decide
/// *which* result happened.
///
/// They are deliberately **excluded** from the all-conditions gate: an outcome table
/// exists precisely because "the run succeeded" and "the run failed" cannot both be
/// true, so requiring every condition would make an outcome-indexed contract
/// unsettleable. Everything else stays a gating condition and must hold whatever the
/// outcome — the deposit that had to be delivered, the title that had to be held,
/// the delay that had to elapse.
#[must_use]
pub fn outcome_conditions(c: &EconomicContract) -> Vec<bool> {
    let mut out = vec![false; c.consent.conditions.len()];
    for o in &c.consent.terms.outcomes {
        for i in &o.when {
            if let Some(slot) = out.get_mut(*i) {
                *slot = true;
            }
        }
    }
    out
}

/// Which outcome the world says happened: `Ok(None)` when the table is empty (the
/// E4a single-split case), `Ok(Some(i))` for exactly one satisfied outcome,
/// [`ContractError::NoOutcome`] when none is, and [`ContractError::AmbiguousOutcome`]
/// when two are — which §3.5 makes a dispute, because a contract that does not say
/// what happened must not pay anybody.
pub fn resolve_outcome(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> Result<Option<usize>, ContractError> {
    if c.consent.terms.outcomes.is_empty() {
        return Ok(None);
    }
    let mut hit: Option<usize> = None;
    for (i, o) in c.consent.terms.outcomes.iter().enumerate() {
        // An outcome with no `when` is unconditional; two of those are ambiguous,
        // which is exactly what the check below says.
        let all = o.when.iter().all(|k| condition_holds(c, *k, snapshot, env));
        if !all {
            continue;
        }
        if hit.is_some() {
            return Err(ContractError::AmbiguousOutcome);
        }
        hit = Some(i);
    }
    hit.map(Some).ok_or(ContractError::NoOutcome)
}

// ---- feasibility at Offer -------------------------------------------------

fn account_instrument(
    snapshot: &EconomySnapshot,
    id: AccountId,
    seen: &mut Option<InstrumentId>,
) -> Result<(), ContractError> {
    let account = snapshot
        .account(id)
        .ok_or(ContractError::NoSuchAccount(id))?;
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

/// The **structural** half of feasibility (§3.5) — everything that is a fact about
/// the record itself, with no world in it: the caps, the split and every outcome's
/// arithmetic, the parties, the payers, the condition graph, and the attestor sets.
///
/// It is a separate function because boot needs exactly this half (E4b2 verify fold,
/// B6): a stored contract's `Offered` surface is not evidence that it is still
/// offerable, and a record that could never settle — a cycle among its `After`
/// anchors, a clock nothing can resolve, an infeasible split, a payer who is not a
/// party — must be quarantined at boot rather than left holding escrow forever. The
/// world-dependent half ([`check_offerable`]) needs a snapshot boot is still building.
///
/// `outcomes` is the wiring-frozen [`CanonOutcomeKinds`] table (E4c verify fold, E1):
/// a `Canon` attestor set naming a kind outside it is refused here, at `Offer`, where
/// a human can still fix the draft.
pub fn check_structure(
    c: &EconomicContract,
    outcomes: &CanonOutcomeKinds,
) -> Result<(), ContractError> {
    check_structure_basic(c)?;
    check_canon_sets(c, outcomes)
}

/// The **world-free, table-free** half of [`check_structure`].
///
/// [`check_offerable`] and every settlement/forfeit plan re-run this and *not* the
/// canon-kind rule, deliberately: a live contract must never be stranded because the
/// composition root's outcome table changed under it. The table is checked once, at
/// `Offer`, where a human can still fix the draft.
fn check_structure_basic(c: &EconomicContract) -> Result<(), ContractError> {
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
        return Err(ContractError::Infeasible(
            "satisfaction vector does not match the conditions",
        ));
    }
    if consent.parties.is_empty() {
        return Err(ContractError::Infeasible(
            "a contract with no parties binds nobody",
        ));
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
    // E6a step 1 (M4 / D-E6-4): **a `System` principal is never a party.** Nobody
    // acts for one, staff included, so a contract naming one could be offered and
    // never accepted — a zombie holding an offer open with nothing to expire it — and
    // a faucet named as a payer would be an uncontrolled mint the moment anything
    // did accept for it. Faucets mint INTO org accounts; they are never parties.
    if consent
        .parties
        .iter()
        .any(|p| matches!(p, PrincipalId::System(_)))
    {
        return Err(ContractError::SystemParty);
    }

    let mut total: i128 = 0;
    for con in &consent.terms.consideration {
        if !con.amount.widen().gt(&0) {
            return Err(ContractError::Infeasible(
                "a consideration must be positive",
            ));
        }
        if matches!(con.from, PrincipalId::System(_)) {
            return Err(ContractError::SystemParty);
        }
        // E4b2 verify fold, M8: **every payer is a party.** A consideration is an
        // escrow obligation, and `@contract/accept` places exactly the ones whose
        // `from` is the accepting party — so a `from` who is not a party is money the
        // contract waits for that nobody can ever place, and `qualifying_escrow` could
        // never be `complete`. Refuse it where a human can still fix the draft.
        if !consent
            .parties
            .iter()
            .any(|p| principal_key(p) == principal_key(&con.from))
        {
            return Err(ContractError::Infeasible(
                "a consideration names a payer who is not a party",
            ));
        }
        total += con.amount.widen();
    }
    let escrow_total = Amount::from_widened(total).ok_or(ContractError::Overflow)?;
    apply_split(escrow_total, &consent.terms)?;

    // The outcome table (08e answers 7-8). Each outcome's shares must be feasible
    // AND sum to the escrowed total exactly — no implicit remainder — and every
    // index it names must exist, or the table would silently never resolve.
    if consent.terms.outcomes.len() > MAX_OUTCOMES {
        return Err(ContractError::TooLarge("too many outcomes"));
    }
    for o in &consent.terms.outcomes {
        // E6a verify fold (D2): a row name is a stored, broadcast string, and an
        // uncapped one reaches the frozen surface, every projection over it, and
        // every notice. Refused here, so no producer can freeze one.
        if o.name.is_empty() || o.name.len() > MAX_OUTCOME_NAME_BYTES {
            return Err(ContractError::TooLarge(
                "an outcome row name is empty or too long",
            ));
        }
        if o.payouts.len() > MAX_SHARES {
            return Err(ContractError::TooLarge("too many shares in one outcome"));
        }
        if o.when.len() > MAX_CONDITIONS {
            return Err(ContractError::TooLarge(
                "too many conditions in one outcome",
            ));
        }
        if o.when.iter().any(|i| *i >= consent.conditions.len()) {
            return Err(ContractError::Infeasible(
                "an outcome names a condition that does not exist",
            ));
        }
        if o.asset_consequences
            .iter()
            .any(|i| *i >= consent.terms.deliverables.len())
        {
            return Err(ContractError::Infeasible(
                "an outcome names a deliverable that does not exist",
            ));
        }
        apply_outcome(escrow_total, &o.payouts)?;
        if o.payouts.len() + consent.terms.consideration.len() > MAX_LINES_PER_TX {
            return Err(ContractError::TooLarge("too many settlement lines"));
        }
    }
    // The permitted-evidence list is exhaustive when it is non-empty (08e answer 6):
    // the parties named which kinds of proof may release their money, and a
    // condition of any other kind is not something they agreed to.
    if !consent.evidence.is_empty() {
        for (i, con) in consent.conditions.iter().enumerate() {
            if !consent.evidence.contains(&con.evidence_kind()) {
                return Err(ContractError::EvidenceNotPermitted(i));
            }
        }
    }
    if let EconomicVisibility::Audience { members, .. } = &consent.visibility
        && members.len() > crate::record::MAX_AUDIENCE
    {
        return Err(ContractError::TooLarge("too many audience members"));
    }

    // Deliverables must fit the ONE plan settlement builds (E3's per-plan caps), and
    // the module admits exactly one custody effect per plan.
    let mut title_ops = 0usize;
    let mut deposits = 0usize;
    for d in &consent.terms.deliverables {
        match d {
            Deliverable::TitleTransfer { .. } | Deliverable::Collateral { .. } => title_ops += 1,
            Deliverable::DepositInto { .. } => deposits += 1,
            Deliverable::Obligation { .. } => {}
        }
    }
    if title_ops > MAX_ASSET_OPS_PER_PLAN {
        return Err(ContractError::TooLarge(
            "too many title ops for one settlement",
        ));
    }
    if deposits > 1 {
        return Err(ContractError::Infeasible("one custody op per settlement"));
    }
    if consent.terms.split.len() + consent.terms.consideration.len() + 1 > MAX_LINES_PER_TX {
        return Err(ContractError::TooLarge("too many settlement lines"));
    }

    // E4b2 verify fold, A3: an attestor set **nobody could ever satisfy** is a
    // condition that can never be discharged, so the escrow behind it is stuck until
    // the deadline with nothing saying why. A `Capability` set with no scope is
    // exactly that — there is no compartment to hold the capability on, and
    // `WorldAuth::authorize_attestor` answers `false` for every actor.
    for con in &consent.conditions {
        if let TypedCondition::Attestation { by } = con
            && !attestor_satisfiable(by)
        {
            return Err(ContractError::Infeasible(
                "a condition names an attestor set nobody can satisfy",
            ));
        }
    }
    if consent
        .adjudicator
        .as_ref()
        .is_some_and(|set| !attestor_satisfiable(set))
    {
        return Err(ContractError::Infeasible(
            "the adjudicator is a set nobody can satisfy",
        ));
    }

    // Conditions must be resolvable: an `After` anchor must exist and never be
    // itself (a self-anchored condition can never become true).
    for (i, con) in consent.conditions.iter().enumerate() {
        match con {
            TypedCondition::After {
                condition, clock, ..
            } => {
                if *condition >= consent.conditions.len() || *condition == i {
                    return Err(ContractError::BadConditionRef(i));
                }
                // Finding 10: a `Manual` clock's position is carried by the record
                // that names it, and a condition carries none — so `reverify` would
                // read the delay against a position nobody set. Refuse at Offer,
                // where a human can still fix the draft.
                if *clock == EconomicClock::Manual {
                    return Err(ContractError::UnsupportedClock(i));
                }
            }
            TypedCondition::Before { deadline } if deadline.clock == EconomicClock::Manual => {
                return Err(ContractError::UnsupportedClock(i));
            }
            _ => {}
        }
    }
    // …and no CYCLE among the `After` anchors (finding 14b). Two conditions that
    // anchor on each other pass the pairwise check above and are individually
    // plausible, but neither can ever be satisfied — the contract is unsettleable,
    // its escrow is stuck until expiry, and nothing says why. Each node has at most
    // one outgoing edge, so following the chain is enough; the walk is bounded by
    // MAX_CONDITIONS (16) both ways.
    for i in 0..consent.conditions.len() {
        if !matches!(
            consent.conditions.get(i),
            Some(TypedCondition::After { .. })
        ) {
            continue;
        }
        let mut seen = vec![false; consent.conditions.len()];
        let mut at = i;
        while let Some(TypedCondition::After { condition, .. }) = consent.conditions.get(at) {
            match seen.get_mut(at) {
                Some(slot) if *slot => return Err(ContractError::BadConditionRef(i)),
                Some(slot) => *slot = true,
                None => return Err(ContractError::BadConditionRef(i)),
            }
            at = *condition;
        }
    }
    Ok(())
}

/// E4c verify fold, E1 + E2: every `Canon` attestor set on the surface must cite a
/// **producer-declared outcome kind** and a **specific source**.
///
/// A lifecycle record any participant can emit — `scene.concluded` — is not an
/// outcome, and a set naming no source is a wildcard over every record of that kind.
/// Between them, a payee who can conclude a scene could manufacture their own
/// settlement, which is exactly what 08e answer 6 refuses.
fn check_canon_sets(
    c: &EconomicContract,
    outcomes: &CanonOutcomeKinds,
) -> Result<(), ContractError> {
    for set in c
        .consent
        .conditions
        .iter()
        .filter_map(|con| match con {
            TypedCondition::Attestation { by } => Some(by),
            _ => None,
        })
        .chain(c.consent.adjudicator.iter())
    {
        if let AttestorSet::Canon { kind, source } = set {
            if source.is_none() {
                return Err(ContractError::Infeasible(
                    "a canon attestor must name the specific source it cites",
                ));
            }
            if !outcomes.contains(kind) {
                return Err(ContractError::Infeasible(
                    "that canon kind is not a declared outcome kind, so it settles nothing",
                ));
            }
        }
    }
    Ok(())
}

/// Whether **anyone at all** could ever qualify for this attestor set (A3).
#[must_use]
fn attestor_satisfiable(set: &AttestorSet) -> bool {
    match set {
        AttestorSet::Capability { scope, .. } => scope.is_some(),
        // A canon set with no source is a wildcard, not an unsatisfiable set; it is
        // refused for that reason a few lines above, with its own message.
        AttestorSet::Party(_) | AttestorSet::Staff | AttestorSet::Canon { .. } => true,
    }
}

/// Feasibility at `Offer` (§3.5): [`check_structure`] plus the world half — every
/// referenced account/vault existing and coherent, and every pledge really pledged by
/// a party who holds its title.
///
/// Checked **again** at settlement, because the world moves between the two.
pub fn check_offerable(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
) -> Result<(), ContractError> {
    check_structure_basic(c)?;
    let consent = &c.consent;
    // Every account exists, is active, and holds ONE instrument: there is no
    // implicit exchange in this economy, so a mixed-instrument contract could never
    // balance (invariant 3).
    let mut instrument: Option<InstrumentId> = None;
    for con in &consent.terms.consideration {
        account_instrument(snapshot, con.account, &mut instrument)?;
    }
    for s in &consent.terms.split {
        account_instrument(snapshot, s.to, &mut instrument)?;
    }
    account_instrument(snapshot, consent.terms.remainder_to, &mut instrument)?;
    if let Expiry::Forfeit { to } = consent.terms.on_expiry {
        account_instrument(snapshot, to, &mut instrument)?;
    }
    for o in &consent.terms.outcomes {
        for s in &o.payouts {
            account_instrument(snapshot, s.to, &mut instrument)?;
        }
    }
    for d in &consent.terms.deliverables {
        match d {
            Deliverable::DepositInto { vault, .. } => {
                snapshot
                    .vault(*vault)
                    .ok_or(ContractError::NoSuchVault(*vault))?;
            }
            // E5b verify fold, T2: **a contract's standing charge debits a party.**
            // Same rule, same reason as the `Collateral` clause below — an
            // `Obligation` deliverable creates a per-period debit at activation, and
            // the only consent anywhere near it is the parties' acceptance of this
            // surface. A template naming a bystander charged somebody who was never
            // asked and whose acceptance was never required; a template naming a
            // *system* account made the contract a standing mint, which is what
            // `@obligation/create` and `@contract/deliver` both refuse by name.
            Deliverable::Obligation { template } => {
                account_instrument(snapshot, template.debit_account, &mut instrument)?;
                account_instrument(snapshot, template.credit_account, &mut instrument)?;
                if matches!(template.debtor, PrincipalId::System(_)) {
                    return Err(ContractError::Infeasible(
                        "a system source/sink cannot be the debtor of a contract's obligation: \
                         that would be a standing mint",
                    ));
                }
                if !consent
                    .parties
                    .iter()
                    .any(|p| principal_key(p) == principal_key(&template.debtor))
                {
                    return Err(ContractError::Infeasible(
                        "a contract's obligation debits someone who is not a party",
                    ));
                }
            }
            // E4b2 verify fold, M1: **a contract encumbers only its own parties'
            // property.** A pledge is a claim placed against somebody's asset without
            // that somebody being asked at the moment of placing, so the consent has
            // to be in the surface: the pledger is a party (so they will have to
            // accept), and they hold the title. Without this, naming a bystander's
            // object as collateral encumbered it at activation and made it
            // untransferable until the contract ended.
            Deliverable::Collateral { object, from } => {
                if !consent
                    .parties
                    .iter()
                    .any(|p| principal_key(p) == principal_key(from))
                {
                    return Err(ContractError::Infeasible(
                        "a collateral is pledged by someone who is not a party",
                    ));
                }
                let asset = snapshot
                    .asset(*object)
                    .ok_or(ContractError::NoSuchAsset(*object))?;
                if asset.lifecycle != AssetLifecycle::Active || &asset.title_owner != from {
                    return Err(ContractError::NoSuchAsset(*object));
                }
            }
            Deliverable::TitleTransfer { .. } => {}
        }
    }
    for con in &consent.conditions {
        if let TypedCondition::DepositInto { vault, .. } = con {
            let v = snapshot
                .vault(*vault)
                .ok_or(ContractError::NoSuchVault(*vault))?;
            // E6b verify fold (M1): **the vault's owner has to be a party.** E6b let
            // a `DepositInto` on a live contract stand in for the vault owner's
            // consent to the deposit (`WorldAuth::authorize_vault`), and that is only
            // true if they agreed to the surface — otherwise anyone holding
            // `vault.view` on a compartment could name somebody else's vault in a
            // one-party contract, deposit into it, and leave the object
            // **custody-locked** where its owner could not take it out. It is the
            // same rule `Deliverable::Collateral` already applies to a pledger a few
            // lines above, and it is what makes "both sides agreed" true.
            if !consent
                .parties
                .iter()
                .any(|p| principal_key(p) == principal_key(&v.owner))
            {
                return Err(ContractError::Infeasible(
                    "a deposit condition names a vault whose owner is not a party",
                ));
            }
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
        return Err(ContractError::Infeasible(
            "satisfaction vector does not match the conditions",
        ));
    }
    // With an outcome table, the conditions an outcome's `when` names are the ones
    // that decide WHICH result happened, and they are mutually exclusive by
    // construction — requiring all of them would make every outcome-indexed contract
    // unsettleable. `resolve_outcome` asks about those; this asks about the rest.
    let outcome_gated = outcome_conditions(c);
    for i in 0..c.consent.conditions.len() {
        if outcome_gated.get(i).copied().unwrap_or(false) {
            continue;
        }
        if !condition_holds(c, i, snapshot, env) {
            return Err(ContractError::ConditionFailed(i));
        }
    }
    Ok(())
}

/// Whether a recorded attestation satisfies one attestor set.
///
/// A `Party` attestation **stands once given** (identity is a fact about the past).
/// A `Capability`/`Staff` attestor is re-qualified **live**, exactly like an approver
/// (E3's A5 rule): an attestor who lost the capability stops counting. A `Canon`
/// reference must be the RIGHT canon reference — kind and (unless the set accepts
/// any) source (E4a verify fold, finding 5), or a bar brawl would discharge a
/// smuggling contract.
#[must_use]
fn attestation_matches(att: &Attestation, set: &AttestorSet, env: &ValidateEnv<'_>) -> bool {
    // A staff resolution stands against every set (E4c verify fold, E4b): it is the
    // documented level-2 answer to evidence that can no longer be produced, and it is
    // written by exactly one command, on a Disputed contract, by a staffer, with a
    // reason that stays on the record.
    if matches!(&att.by, AttestorRef::StaffOverride { .. }) {
        return true;
    }
    match set {
        AttestorSet::Party(p) => matches!(&att.by, AttestorRef::Principal(a) if a == p),
        AttestorSet::Capability { .. } | AttestorSet::Staff => {
            matches!(&att.by, AttestorRef::Principal(a) if env.auth.authorize_attestor(a, set))
        }
        AttestorSet::Canon { .. } => match &att.by {
            AttestorRef::Canon { kind, source, .. } => canon_set_accepts(set, kind, source),
            _ => false,
        },
    }
}

/// Whether `set` is a [`AttestorSet::Canon`] that a canon record of this `kind` and
/// `source` satisfies — **kind and source both** (E4a verify fold, finding 5; E5's
/// owner answer 20: progress must cite a specific front, opportunity or operation,
/// so a `scene.concluded` involving any faction member is deliberately not a match).
/// A set naming no source accepts any source of that kind.
///
/// It is public because E4c's canon observer asks exactly this question *before*
/// there is an attestation to compare, and the observer and [`reverify`] must never
/// be able to disagree about what "the right canon record" means: an observer that
/// recorded a looser match would write attestations that never re-verify, freezing
/// the escrow of a contract whose evidence had arrived.
#[must_use]
pub fn canon_set_accepts(set: &AttestorSet, kind: &str, source: &str) -> bool {
    match set {
        AttestorSet::Canon {
            kind: want_kind,
            source: want_source,
        } => want_kind == kind && want_source.as_ref().is_none_or(|s| s == source),
        _ => false,
    }
}

/// Whether condition `i` holds **right now** — the one place a condition kind is
/// evaluated, so `reverify`, `resolve_outcome` and the readiness check can never
/// drift apart. An index past the end is `false` (never a panic).
#[must_use]
pub fn condition_holds(
    c: &EconomicContract,
    i: usize,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> bool {
    let Some(condition) = c.consent.conditions.get(i) else {
        return false;
    };
    let Some(recorded) = c.satisfied.get(i) else {
        return false;
    };
    let clocks = Clocks::new(env.now, env.ic_now);
    {
        match condition {
            // The condition's own attestor set, OR — when the consent surface names
            // one — the **adjudicator's** (08e answer 6). A named adjudicator
            // replaces each condition's set at attestation time, so re-verification
            // has to accept their word symmetrically or an adjudicated contract
            // could be attested and then never settle.
            TypedCondition::Attestation { by } => match recorded {
                None => false,
                Some(att) => {
                    attestation_matches(att, by, env)
                        || c.consent
                            .adjudicator
                            .as_ref()
                            .is_some_and(|set| attestation_matches(att, set, env))
                }
            },
            TypedCondition::DepositInto { object, vault } => snapshot
                .vault(*vault)
                .is_some_and(|v| env.custody.holds(v, *object)),
            TypedCondition::TitleHeld { object, by } => snapshot
                .asset(*object)
                .is_some_and(|a| a.lifecycle == AssetLifecycle::Active && &a.title_owner == by),
            // `None` (unknown) is unsatisfied, never a pass: the fact provider seam
            // is E4c and defaults to unknown, so a contract cannot settle on a fact
            // nobody can answer.
            TypedCondition::Fact { fact } => env.facts.resolve(fact) == Some(true),
            // Defence in depth for finding 10: `check_offerable` refuses a `Manual`
            // deadline outright, but a hand-built or migrated record could carry
            // one — so the manual position handed in here is `i64::MAX`, i.e. the
            // deadline has ALWAYS passed and the condition is unsatisfied. A clock
            // this engine cannot resolve must never be the reason money moves.
            TypedCondition::Before { deadline } => clocks
                .now_on(deadline.clock, snapshot, i64::MAX)
                .is_some_and(|now| now <= deadline.at),
            TypedCondition::After {
                condition,
                delay,
                clock,
            } => {
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
        }
    }
}

/// Whether this condition kind is **deterministic** — decided by the world and the
/// clock rather than by somebody's word (08e answer 6's third route).
#[must_use]
fn is_deterministic(cond: &TypedCondition) -> bool {
    !matches!(cond, TypedCondition::Attestation { .. })
}

/// Stamp every deterministic condition that **holds right now and carries no stamp
/// yet** with [`AttestorRef::Observed`] (E4c verify fold, E5). Returns whether
/// anything changed.
///
/// A deterministic condition never filled its satisfaction slot, so an
/// [`TypedCondition::After`] anchored on one had nothing to count its delay from and
/// "pay 60 s after the job is approved" could never mature. The stamp is an
/// **observation**, not evidence: `condition_holds` still re-evaluates the condition
/// against the live world at settlement, so a fact that stops holding still stops
/// settling. Written **once**, like every other attestation (E4b2 fold M5), so the
/// anchor cannot be pushed forward.
///
/// The clock is the one the anchoring `After` will compare on — two stamps are
/// comparable only on the same clock (doc 09 §2.5) — and `RealUtc` when nothing
/// anchors on it.
pub fn stamp_observed(
    c: &mut EconomicContract,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> bool {
    if c.satisfied.len() != c.consent.conditions.len() {
        return false;
    }
    let clocks = Clocks::new(env.now, env.ic_now);
    let mut wrote = false;
    for i in 0..c.consent.conditions.len() {
        if !c.consent.conditions.get(i).is_some_and(is_deterministic) {
            continue;
        }
        if c.satisfied.get(i).is_some_and(Option::is_some) {
            continue;
        }
        if !condition_holds(c, i, snapshot, env) {
            continue;
        }
        let clock = c
            .consent
            .conditions
            .iter()
            .find_map(|cond| match cond {
                TypedCondition::After {
                    condition, clock, ..
                } if *condition == i => Some(*clock),
                _ => None,
            })
            .unwrap_or(EconomicClock::RealUtc);
        // A clock this engine cannot resolve stamps nothing: an anchor nobody can
        // place is worse than no anchor at all.
        let Some(at) = clocks.now_on(clock, snapshot, env.now) else {
            continue;
        };
        if let Some(slot) = c.satisfied.get_mut(i) {
            *slot = Some(Attestation {
                by: AttestorRef::Observed { clock },
                at,
                clock,
                source: SmolStr::new_static("observed"),
            });
            wrote = true;
        }
    }
    wrote
}

/// Where a live contract stands right now.
///
/// The three answers are genuinely different and must not be collapsed: *not yet* is
/// the ordinary waiting state, *ready* settles, and **undecidable** is a decision a
/// human has to make. An ambiguous outcome table read as "not yet" would leave the
/// escrow frozen forever with nobody told — the contract would be waiting for a
/// condition that is already true.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// Every gating condition holds and the outcome (if any) is decided.
    Ready(Option<usize>),
    /// Something the contract is still waiting for.
    NotYet,
    /// The contract cannot settle at all as it stands, and no retry will change it
    /// (two outcomes satisfied at once; a satisfaction vector that does not match the
    /// conditions; a mutated consent surface).
    Undecidable(ContractError),
}

/// Where `c` stands — the one readiness verdict the command path and the sweep both
/// read, so they can never disagree about what a contract is waiting for.
#[must_use]
pub fn readiness(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> Readiness {
    match reverify(c, snapshot, env) {
        Err(ContractError::ConditionFailed(_)) => Readiness::NotYet,
        Err(e) => Readiness::Undecidable(e),
        Ok(()) => match resolve_outcome(c, snapshot, env) {
            Ok(outcome) => Readiness::Ready(outcome),
            Err(ContractError::NoOutcome) => Readiness::NotYet,
            Err(e) => Readiness::Undecidable(e),
        },
    }
}

/// Whether every condition is currently satisfied — the readiness question
/// [`reverify`] answers, as a predicate.
#[must_use]
pub fn conditions_met(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> bool {
    matches!(readiness(c, snapshot, env), Readiness::Ready(_))
}

// ---- the settlement and refund plans --------------------------------------

/// The claim this contract places on a pledged object at activation, and releases at
/// settlement or refund (§5, D-E4-8).
///
/// A contract that also creates an obligation **is a lease** — the pledge secures a
/// stream of payments rather than a single settlement — so the claim names its
/// `lessor` and reads as one in `@asset/show`. Everything else is plain
/// [`Encumbrance::Collateral`]. Both are contract-placed and contract-released;
/// neither is ever staff-minted, because the contract IS the consent.
///
/// The same function decides the encumbrance at both ends, so a lease can never be
/// placed as a `Lease` and then looked for as a `Collateral` — which would leave the
/// claim on the asset forever and make it untransferable (invariant 12).
#[must_use]
pub fn contract_encumbrance(c: &EconomicContract, from: &PrincipalId) -> Encumbrance {
    let leased = c
        .consent
        .terms
        .deliverables
        .iter()
        .any(|d| matches!(d, Deliverable::Obligation { .. }));
    if leased {
        Encumbrance::Lease {
            lessor: from.clone(),
            contract: c.id,
        }
    } else {
        Encumbrance::Collateral { contract: c.id }
    }
}

/// The `AssetOp::Release`s that take this contract's own claims back off the assets
/// that really carry them.
///
/// The **live check matters**: a contract cancelled while it was still `Offered`
/// never activated, so nothing was encumbered, and a `Release` naming an encumbrance
/// the asset does not carry is `NoSuchEncumbrance` — which would fail the whole
/// refund plan and strand the escrow it was meant to give back. Releasing only what
/// is actually there keeps the unwind total.
#[must_use]
pub fn release_asset_ops(c: &EconomicContract, snapshot: &EconomySnapshot) -> Vec<AssetOp> {
    c.consent
        .terms
        .deliverables
        .iter()
        .filter_map(|d| match d {
            Deliverable::Collateral { object, from } => {
                let encumbrance = contract_encumbrance(c, from);
                snapshot
                    .asset(*object)
                    .filter(|a| a.encumbrances.contains(&encumbrance))
                    .map(|_| AssetOp::Release {
                        object: *object,
                        encumbrance,
                    })
            }
            _ => None,
        })
        .collect()
}

/// The asset ops that place this contract's claims at activation — one per
/// `Collateral` deliverable, in declared order.
#[must_use]
pub fn activation_asset_ops(c: &EconomicContract) -> Vec<AssetOp> {
    c.consent
        .terms
        .deliverables
        .iter()
        .filter_map(|d| match d {
            Deliverable::Collateral { object, from } => Some(AssetOp::Encumber {
                object: *object,
                encumbrance: contract_encumbrance(c, from),
            }),
            _ => None,
        })
        .collect()
}

/// The deterministic idempotency key for a contract's activation, at its own
/// generation.
#[must_use]
pub fn activate_key(c: &EconomicContract) -> IdempotencyKey {
    contract_op_key(c.id, "activate", c.amendments)
}

/// Build the **forfeit** plan for an expired contract whose `on_expiry` is
/// [`Expiry::Forfeit`] (§5): capture the whole escrow and pay it to `to`.
///
/// `to` is inside the consent surface — [`check_offerable`] proves the account
/// exists, is `Active`, and holds the contract's one instrument — so a forfeit can
/// only ever pay somewhere every party saw before they funded it.
///
/// It is deliberately available only for a **fully accepted** contract: an `Offered`
/// contract that lapsed never came into force, and taking the money someone escrowed
/// toward an agreement that never bound anybody is not a forfeit, it is a
/// confiscation. E4b2's expiry path refunds those instead (§5.1's deviation 3).
pub fn forfeit_plan(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
) -> Result<EconomyPlan, ContractError> {
    // Either the transition has already landed (boot, a retry after a restart) or it
    // is **in flight** — `pending: Forfeit` (M2+M3). The plan posts first and the
    // state moves after it, so "already `Expired`" cannot be the only way in.
    let in_flight = c
        .pending
        .as_ref()
        .is_some_and(|p| p.kind == PendingKind::Forfeit);
    if !matches!(c.state, ContractState::Expired) && !in_flight {
        return Err(ContractError::WrongState(c.id));
    }
    let Expiry::Forfeit { to } = c.consent.terms.on_expiry else {
        return Err(ContractError::Infeasible(
            "this contract does not forfeit on expiry",
        ));
    };
    if !c.consent_intact() {
        return Err(ContractError::ConsentMismatch);
    }
    if !c.fully_accepted() {
        return Err(ContractError::NotAccepted);
    }
    check_offerable(c, snapshot)?;
    let escrow = qualifying_escrow(c, snapshot);
    let mut hold_ops: Vec<HoldOp> = Vec::with_capacity(escrow.holds.len());
    let mut expected_hold_revisions: Vec<HoldRevision> = Vec::with_capacity(escrow.holds.len());
    let mut lines: Vec<RequestedLine> = Vec::with_capacity(escrow.holds.len() + 1);
    let mut total: i128 = 0;
    for hold in &escrow.holds {
        total += hold.amount.widen();
        hold_ops.push(HoldOp::Capture {
            hold: hold.id,
            amount: hold.amount,
        });
        expected_hold_revisions.push(HoldRevision {
            hold: hold.id,
            revision: hold.revision,
        });
        let debit = hold.amount.checked_neg().ok_or(ContractError::Overflow)?;
        lines.push(RequestedLine {
            account: hold.account,
            delta: debit,
            memo: SmolStr::new(format!("contract {} forfeit", c.id)),
        });
    }
    let total = Amount::from_widened(total).ok_or(ContractError::Overflow)?;
    if total.is_zero() {
        return Err(ContractError::Infeasible("nothing is escrowed to forfeit"));
    }
    lines.push(RequestedLine {
        account: to,
        delta: total,
        memo: SmolStr::new(format!("contract {} forfeit", c.id)),
    });
    let mut plan = EconomyPlan::new(
        contract_op_key(c.id, "forfeit", c.amendments),
        c.issuer.clone(),
        EconomicCause::Internal {
            kind: SmolStr::new_static("contract"),
            id: c.id.0,
        },
        lines,
    );
    plan.description = format!("forfeit of contract {}", c.id);
    plan.visibility = c.consent.visibility.clone();
    // A pledge comes back whatever happened; only the money is forfeit.
    plan.asset_ops = release_asset_ops(c, snapshot);
    plan.hold_ops = hold_ops;
    plan.expected_hold_revisions = expected_hold_revisions;
    Ok(plan)
}

/// The deterministic idempotency key for a contract's forfeit at generation 0.
#[must_use]
pub fn forfeit_key(id: ContractId) -> IdempotencyKey {
    contract_op_key(id, "forfeit", 0)
}

/// Build the settlement plan for a `ReadyToSettle` contract (§3.5).
///
/// Captures **every escrow hold with an explicit amount**, applies the deterministic
/// split, and carries the title ops and collateral releases the surface describes —
/// all as one plan, so the ordinary `validate`/`apply` path proves every invariant
/// and the whole settlement is one durable bundle or nothing. Cause
/// `Internal { kind: "contract", id }`, key `contract/{id}/settle`, so settling twice
/// is `AlreadyPosted` (scenario 16).
///
/// It takes the live [`ValidateEnv`] because it **re-verifies every condition
/// first** (E4a verify fold, finding 6). Handing back a settlement for a contract
/// whose deposit has been withdrawn or whose attestor lost their capability — and
/// leaving the re-check to whoever remembers to call [`reverify`] separately — is
/// the exact shape of a payout on an unmet condition, so there is no way to get the
/// plan without the proof.
pub fn settlement_plan(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
    env: &ValidateEnv<'_>,
) -> Result<EconomyPlan, ContractError> {
    if !matches!(
        c.state,
        ContractState::ReadyToSettle | ContractState::Active
    ) {
        return Err(ContractError::State(c.id));
    }
    if !c.consent_intact() {
        return Err(ContractError::ConsentMismatch);
    }
    if !c.fully_accepted() {
        return Err(ContractError::NotAccepted);
    }
    check_offerable(c, snapshot)?;
    reverify(c, snapshot, env)?;
    let outcome = resolve_outcome(c, snapshot, env)?;

    // Every escrow hold must still be live — a broken escrow is a dispute, never a
    // settlement that pays out less than the parties agreed — and must actually BE
    // this contract's escrow (finding 3): a hold that merely got listed in `escrow`
    // is not money anyone consented to spend.
    for id in &c.escrow {
        let hold = snapshot.hold(*id).ok_or(ContractError::NoSuchHold(*id))?;
        if !hold.status.is_held() {
            return Err(ContractError::HoldNotHeld(*id));
        }
    }
    let escrow = qualifying_escrow(c, snapshot);
    if !escrow.complete {
        return Err(ContractError::EscrowIncomplete);
    }
    let mut hold_ops: Vec<HoldOp> = Vec::with_capacity(escrow.holds.len());
    let mut expected_hold_revisions: Vec<HoldRevision> = Vec::with_capacity(escrow.holds.len());
    let mut debits: Vec<RequestedLine> = Vec::with_capacity(escrow.holds.len());
    let mut total: i128 = 0;
    for hold in &escrow.holds {
        total += hold.amount.widen();
        hold_ops.push(HoldOp::Capture {
            hold: hold.id,
            amount: hold.amount,
        });
        expected_hold_revisions.push(HoldRevision {
            hold: hold.id,
            revision: hold.revision,
        });
        let debit = hold.amount.checked_neg().ok_or(ContractError::Overflow)?;
        debits.push(RequestedLine {
            account: hold.account,
            delta: debit,
            memo: SmolStr::new(format!("contract {} escrow", c.id)),
        });
    }
    let total = Amount::from_widened(total).ok_or(ContractError::Overflow)?;
    // The resolved outcome's exact payouts, or E4a's single split when the table is
    // empty. An outcome has no implicit remainder (08e answer 7).
    let payouts = match outcome.and_then(|i| c.consent.terms.outcomes.get(i)) {
        Some(o) => apply_outcome(total, &o.payouts)?,
        None => apply_split(total, &c.consent.terms)?,
    };

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
    //
    // With an outcome table, a title moves only if the **resolved outcome** names it
    // (`asset_consequences`): a failed run does not hand over the goods a successful
    // one would. A collateral release is deliberately NOT outcome-gated — a pledge
    // has to come back whichever way the contract went, or the encumbrance strands.
    let mut asset_ops: Vec<AssetOp> = Vec::new();
    let consequences = outcome
        .and_then(|i| c.consent.terms.outcomes.get(i))
        .map(|o| o.asset_consequences.clone());
    for (i, d) in c.consent.terms.deliverables.iter().enumerate() {
        match d {
            Deliverable::TitleTransfer { object, to, .. } => {
                if consequences.as_ref().is_some_and(|xs| !xs.contains(&i)) {
                    continue;
                }
                asset_ops.push(AssetOp::TransferTitle {
                    object: *object,
                    to: to.clone(),
                    to_scope: c
                        .consent
                        .scope
                        .filter(|_| matches!(to, PrincipalId::Organization(_))),
                });
            }
            Deliverable::Collateral { object, from } => {
                // Only what the asset really carries (see `release_asset_ops`).
                let encumbrance = contract_encumbrance(c, from);
                if snapshot
                    .asset(*object)
                    .is_some_and(|a| a.encumbrances.contains(&encumbrance))
                {
                    asset_ops.push(AssetOp::Release {
                        object: *object,
                        encumbrance,
                    });
                }
            }
            Deliverable::DepositInto { .. } | Deliverable::Obligation { .. } => {}
        }
    }

    let mut plan = EconomyPlan::new(
        contract_op_key(c.id, "settle", c.amendments),
        c.issuer.clone(),
        EconomicCause::Internal {
            kind: SmolStr::new_static("contract"),
            id: c.id.0,
        },
        lines,
    );
    plan.description = format!("settlement of contract {}", c.id);
    plan.visibility = c.consent.visibility.clone();
    plan.asset_ops = asset_ops;
    plan.hold_ops = hold_ops;
    plan.expected_hold_revisions = expected_hold_revisions;
    Ok(plan)
}

// ---- E6b: B3 — DL-03, an org or system depositor's deposit comes home -----

/// How many vaults [`owner_home`]'s search visits (E6b verify fold, D11).
pub const MAX_HOME_VAULT_VISIT: usize = 256;

/// Where one recorded deposit goes at unwind (E6b, DL-03).
///
/// `to` is `None` when the deposit is **stranded**: a depositor who is not a world
/// object and owns no `Active` vault with a world backing to return it into. The
/// caller pages on that rather than silently dropping it (08a's staff-scarce rule:
/// never park state on "wait for staff" without saying exactly what is stuck).
pub struct DepositReturn<'a> {
    /// The deposit itself.
    pub deposit: &'a DepositRecord,
    /// Where it goes: a world object for an `Object` depositor, the backing of a
    /// vault they own for an organization or system principal.
    pub to: Option<ObjId>,
    /// The vault `to` backs, when the destination is one.
    pub into_vault: Option<VaultId>,
}

/// Route every recorded deposit home (E6b, DL-03). **One function**, shared by
/// [`refund_plan`] (which builds the ops) and [`ContractAuth::unwinding`] (which
/// authorizes exactly them), so the plan and the authority can never disagree about
/// where somebody's property is allowed to go.
#[must_use]
pub fn deposit_returns<'a>(
    c: &'a EconomicContract,
    snapshot: &EconomySnapshot,
) -> Vec<DepositReturn<'a>> {
    c.deposits
        .iter()
        .take(MAX_DEPOSITS)
        .map(|d| match &d.deposited_by {
            PrincipalId::Object(o) => DepositReturn {
                deposit: d,
                to: Some(*o),
                into_vault: None,
            },
            owner @ (PrincipalId::Organization(_) | PrincipalId::System(_)) => {
                match owner_home(snapshot, owner) {
                    Some((vault, backing)) => DepositReturn {
                        deposit: d,
                        to: Some(backing),
                        into_vault: Some(vault),
                    },
                    None => DepositReturn {
                        deposit: d,
                        to: None,
                        into_vault: None,
                    },
                }
            }
        })
        .collect()
}

/// The lowest-id `Active` vault `owner` owns whose backing is a single world
/// object — deterministic, so the plan and the authority pick the same one.
///
/// Public because [`crate::contract_live`]'s returnability check must ask exactly
/// this question rather than a second, drifting one.
pub fn owner_home(snapshot: &EconomySnapshot, owner: &PrincipalId) -> Option<(VaultId, ObjId)> {
    // E6b verify fold (D11): bounded on vaults **visited**. It is asked once per
    // deposit, from the plan, from the authority and from the returnability check,
    // so an unbounded scan here is three unbounded scans per unwind.
    snapshot
        .vaults()
        .take(MAX_HOME_VAULT_VISIT)
        .filter(|v| &v.owner == owner && v.status == crate::record::VaultStatus::Active)
        .find_map(|v| match &v.backing {
            crate::record::CustodyRef::Container(o)
            | crate::record::CustodyRef::SpatialInterior(o) => Some((v.id, *o)),
            // A provider-registered backing has no single world object, so there is
            // nothing a `Withdraw` could name as a destination.
            crate::record::CustodyRef::Registered { .. } => None,
        })
}

/// Build the refund plan: release every **qualifying** escrow hold and nothing else —
/// E4b2's collateral refinement (D-E4b2-24): a `Collateral` pledge is released by the
/// same `release_asset_ops` a settlement uses, so a refund never leaves a pledge
/// encumbered. Effect-only (legal under invariant 10), key `contract/{id}/refund`.
///
/// Two guards the E4a verify fold added:
///
/// - the contract must already be `Cancelled` or `Expired` (finding 6). A refund
///   releases escrow, so on a live contract it is a way to strip the funding out
///   from under an agreement that is still standing, and on a `Settled` one there is
///   nothing left to give back. The state change comes first; the refund follows it.
/// - only holds that pass [`qualifying_escrow`] are released (finding 3), so a hold
///   listed in `escrow` by someone editing the record cannot be freed by this plan.
///
/// **E4b2 adds the other two halves of an unwind**, in the same plan and therefore
/// the same durable bundle:
///
/// - every claim this contract placed at activation is released (a pledge outlives
///   neither the agreement nor its own contract);
/// - every recorded [`DepositRecord`] is **returned to its depositor** with a
///   `CustodyOp::Withdraw`. That is the replacement for the deleted custody-lock
///   exception (E4a verify fold, finding 2): the lock lifts on the state change,
///   which is why a refund runs *after* it, and the withdrawal is authorized by
///   [`ContractAuth::unwinding`] and by nothing else. The plan's `actor` becomes the
///   depositor when it carries a withdrawal, because invariant 15's destination rule
///   is "the actor, or the actor's location" — returning someone's property to them
///   is an act performed on their behalf.
pub fn refund_plan(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
) -> Result<EconomyPlan, ContractError> {
    // As for `forfeit_plan`: already terminal, or the unwind is **in flight** with
    // `pending` set (M2+M3). A live contract with no pending transition still gets
    // `WrongState` — a refund is not a way to strip the funding out from under an
    // agreement that is standing.
    let in_flight = c
        .pending
        .as_ref()
        .is_some_and(|p| matches!(p.kind, PendingKind::Cancel | PendingKind::Expire));
    if !matches!(c.state, ContractState::Cancelled | ContractState::Expired) && !in_flight {
        return Err(ContractError::WrongState(c.id));
    }
    // E6b (DL-03): an org or system depositor's deposit **comes home** — to the
    // backing of a vault that principal owns. Before, it was filtered out here
    // ("no world object to hand the thing back to") and left in the counterparty's
    // vault, which is exactly the stranded consignment 08f §4.3 names.
    let returns: Vec<(&DepositRecord, ObjId)> = deposit_returns(c, snapshot)
        .into_iter()
        .filter_map(|r| r.to.map(|to| (r.deposit, to)))
        .take(MAX_DEPOSITS)
        .collect();
    let actor = returns
        .first()
        .map_or_else(|| c.issuer.clone(), |(d, _)| d.deposited_by.clone());
    let mut plan = EconomyPlan::new(
        contract_op_key(c.id, "refund", c.amendments),
        actor,
        EconomicCause::Internal {
            kind: SmolStr::new_static("contract"),
            id: c.id.0,
        },
        Vec::new(),
    );
    plan.description = format!("refund of contract {}", c.id);
    plan.visibility = c.consent.visibility.clone();
    plan.hold_ops = qualifying_escrow(c, snapshot)
        .holds
        .iter()
        .map(|hold| HoldOp::Release { hold: hold.id })
        .collect();
    plan.asset_ops = release_asset_ops(c, snapshot);
    plan.custody_ops = returns
        .iter()
        .map(|(d, to)| CustodyOp::Withdraw {
            object: d.object,
            vault: d.vault,
            to: *to,
        })
        .collect();
    Ok(plan)
}

/// Build the **partial** refund an amendment posts (E6a step 1, DL-01 Option A′ —
/// consent by authorship): release every qualifying escrow hold that backs a
/// consideration owed by a party **other than** `keep`, and nothing else.
///
/// The amending party's own acceptance survives the amendment, and so does the
/// escrow behind it — an author is not asked to re-consent to terms they wrote —
/// while every other acceptance is voided and its escrow released exactly as E4b2's
/// whole-surface unwind does. Under the same deterministic key at the same
/// generation (`contract/{id}/refund/{n}`), under [`ContractAuth::unwinding`], so an
/// ambiguous ack replays to `AlreadyPosted` and the state write that follows can be
/// finished rather than repeated.
///
/// Only an `Offered` contract is amendable, and an `Offered` contract has neither
/// pledges (they are placed at activation) nor recorded deposits (the same), so the
/// plan carries hold releases alone. An empty plan means the voided parties escrowed
/// nothing: the caller writes the state and posts nothing, and the generation does
/// not move (M16 — a refund key that was never used need not be spent).
pub fn refund_plan_except(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
    keep: &PrincipalId,
) -> Result<EconomyPlan, ContractError> {
    if c.state != ContractState::Offered {
        return Err(ContractError::State(c.id));
    }
    if !c.consent_intact() {
        return Err(ContractError::ConsentMismatch);
    }
    let (claims, _) = qualifying_claims(c, snapshot);
    let considerations = &c.consent.terms.consideration;
    let mut plan = EconomyPlan::new(
        contract_op_key(c.id, "refund", c.amendments),
        c.issuer.clone(),
        EconomicCause::Internal {
            kind: SmolStr::new_static("contract"),
            id: c.id.0,
        },
        Vec::new(),
    );
    plan.description = format!("amendment refund of contract {}", c.id);
    plan.visibility = c.consent.visibility.clone();
    plan.hold_ops = claims
        .iter()
        .filter(|(_, i)| {
            considerations
                .get(*i)
                .is_some_and(|con| principal_key(&con.from) != principal_key(keep))
        })
        .map(|(hold, _)| HoldOp::Release { hold: hold.id })
        .collect();
    Ok(plan)
}

// ---- ContractAuth ---------------------------------------------------------

/// What a [`ContractAuth`] is being built to do — and therefore the exact set of
/// operations it authorizes. Nothing outside its own mode's list is ever `true`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractPurpose {
    /// **Settlement**: capture this contract's own escrow, credit the payees the
    /// resolved outcome names **exactly what the split owes them**, move the titles
    /// that outcome causes, release the pledges. No release of escrow, no debit
    /// authority, no vault op.
    ///
    /// E4b2 verify fold, M4: the payouts carry their **amounts**, not just the
    /// account ids. A membership check ("is this account a payee?") authorized any
    /// distribution at all among the named accounts — a hand-built plan crediting the
    /// whole escrow to `remainder_to`, or paying one runner four times their share,
    /// passed every check, because the plan is what says who gets what and the
    /// authority was only saying who *may* be paid.
    Settle {
        /// What the resolved payout credits each account, netted per account.
        payouts: Vec<(AccountId, Amount)>,
        /// The `deliverables` indexes whose title moves this settlement carries.
        titles: Vec<usize>,
    },
    /// **Forfeit** (`Expiry::Forfeit { to }`): capture the escrow and credit exactly
    /// `to` — and exactly the escrowed `total` — which the consent surface named and
    /// `check_offerable` proved usable. No title moves: a forfeited deadline is not a
    /// delivery.
    Forfeit {
        /// The single account the escrow pays.
        to: AccountId,
        /// Exactly how much it pays.
        total: Amount,
    },
    /// **Unwind** (refund, cancellation, expiry-refund, amendment): release this
    /// contract's own escrow, release its pledges, and return each recorded deposit
    /// to its depositor. No capture, no credit, no title move.
    Unwind,
}

/// The [`crate::AuthEval`] a contract's own acceptances constitute (D-E4-4).
///
/// It authorizes **exactly** the surface, and nothing else. Concretely, per
/// [`ContractPurpose`]:
///
/// | op | `Settle` | `Forfeit` | `Unwind` |
/// |---|---|---|---|
/// | `HoldOp::Capture` on a **qualifying** escrow hold, ≤ its amount | yes | yes | no |
/// | `HoldOp::Release` on a qualifying escrow hold | no | no | yes |
/// | `HoldOp::Place` / `HoldOp::Expire` | no | no | no |
/// | a **credit** to a named payee | the resolved payouts | exactly `to` | no |
/// | a **debit** of any size | no | no | no |
/// | `AssetOp::TransferTitle` for a deliverable both sides accepted | the outcome's | no | no |
/// | `AssetOp::Encumber`/`Release` of this contract's own pledge | yes | yes | yes |
/// | `CustodyOp::Withdraw` of a recorded deposit to its depositor | no | no | yes |
/// | vault deposits, maintenance, approvers, anything else | no | no | no |
///
/// No authority is stored anywhere: this value is built from the live record at the
/// moment of use and thrown away. It grants **no bare debit authority at all** (E4a
/// verify fold, finding 1) — a contract's power to take money is precisely its power
/// to capture its own escrow, which runs through
/// [`crate::AuthEval::authorize_hold`] plus the engine's rule that a captured amount
/// is pre-authorized debit authority. Keeping a "debit up to what is escrowed on
/// that account" ceiling *as well* would double count the escrow: the engine only
/// asks about the part of a debit the captures do not cover, so 100 in escrow would
/// fund a 100 capture **and** a further 100 of lines the parties never agreed to.
pub struct ContractAuth<'a> {
    contract: &'a EconomicContract,
    /// The holds that qualify as this contract's escrow (finding 3) — the ONLY ones
    /// its authority reaches. `contract.escrow` is a mutable list of ids and is not
    /// evidence of anything on its own.
    qualifying: Vec<HoldId>,
    purpose: ContractPurpose,
    /// E6b (DL-03): `(object, backing)` pairs an **unwind** may return a deposit
    /// into, for a depositor with no world object of their own. Empty for every
    /// other purpose, and empty for an object depositor (whose return goes through
    /// the ordinary destination rule).
    homes: Vec<(ObjId, ObjId)>,
}

impl<'a> ContractAuth<'a> {
    /// The settlement authority over every payee the terms name — E4a's shape, kept
    /// for a single-split contract and for callers that resolve no outcome.
    #[must_use]
    pub fn new(contract: &'a EconomicContract, snapshot: &EconomySnapshot) -> ContractAuth<'a> {
        ContractAuth::settling(contract, snapshot, None)
    }

    /// The settlement authority for one **resolved outcome**: only that outcome's
    /// payees may be credited, and only the titles it causes may move.
    #[must_use]
    pub fn settling(
        contract: &'a EconomicContract,
        snapshot: &EconomySnapshot,
        outcome: Option<usize>,
    ) -> ContractAuth<'a> {
        let terms = &contract.consent.terms;
        let total = escrowed_total(contract, snapshot);
        // The SAME arithmetic `settlement_plan` uses, over the same qualifying escrow
        // (M4). An amount the plan cannot justify from the consented split is an
        // amount this authority does not know about, and an unresolvable split
        // authorizes no credit at all.
        let payouts = match outcome.and_then(|i| terms.outcomes.get(i)) {
            Some(o) => apply_outcome(total, &o.payouts),
            None => apply_split(total, terms),
        };
        let titles = match outcome.and_then(|i| terms.outcomes.get(i)) {
            Some(o) => o.asset_consequences.clone(),
            None => (0..terms.deliverables.len()).collect(),
        };
        ContractAuth {
            contract,
            qualifying: qualifying_ids(contract, snapshot),
            purpose: ContractPurpose::Settle {
                payouts: net_payouts(&payouts.unwrap_or_default()),
                titles,
            },
            homes: Vec::new(),
        }
    }

    /// The forfeit authority: capture the escrow, credit exactly `to`, exactly the
    /// escrowed total.
    #[must_use]
    pub fn forfeiting(
        contract: &'a EconomicContract,
        snapshot: &EconomySnapshot,
        to: AccountId,
    ) -> ContractAuth<'a> {
        let total = escrowed_total(contract, snapshot);
        ContractAuth {
            contract,
            qualifying: qualifying_ids(contract, snapshot),
            purpose: ContractPurpose::Forfeit { to, total },
            homes: Vec::new(),
        }
    }

    /// The unwind authority: release this contract's escrow, release its pledges,
    /// and hand each recorded deposit back to its depositor.
    ///
    /// It deliberately does **not** require [`EconomicContract::fully_accepted`]. A
    /// contract cancelled while it was still `Offered` has one party's money escrowed
    /// and no full acceptance, and that money has to come back; requiring unanimity
    /// to give people their own funds back would strand exactly the contracts that
    /// never came into force.
    #[must_use]
    pub fn unwinding(
        contract: &'a EconomicContract,
        snapshot: &EconomySnapshot,
    ) -> ContractAuth<'a> {
        ContractAuth {
            contract,
            qualifying: qualifying_ids(contract, snapshot),
            purpose: ContractPurpose::Unwind,
            // E6b (DL-03): the vault-backing destinations this unwind — and nothing
            // else in the world — may send a deposit home to. Resolved here, from
            // the same `deposit_returns` the plan is built from, and thrown away
            // with the authority.
            homes: deposit_returns(contract, snapshot)
                .iter()
                .filter_map(|r| match (r.to, r.into_vault) {
                    (Some(to), Some(_)) => Some((r.deposit.object, to)),
                    _ => None,
                })
                .collect(),
        }
    }

    /// What this authority is for.
    #[must_use]
    pub fn purpose(&self) -> &ContractPurpose {
        &self.purpose
    }

    /// Whether the surface still binds — full acceptance for anything that moves
    /// money, mere consent-hash integrity for an unwind's give-backs.
    fn bound(&self) -> bool {
        match self.purpose {
            ContractPurpose::Unwind => self.contract.consent_intact(),
            ContractPurpose::Settle { .. } | ContractPurpose::Forfeit { .. } => {
                self.contract.fully_accepted()
            }
        }
    }

    /// Whether this contract's own pledge on `object` may be placed or released.
    fn pledges(&self, object: ObjId, encumbrance: &Encumbrance) -> bool {
        let mine = match encumbrance {
            Encumbrance::Collateral { contract } | Encumbrance::Lease { contract, .. } => {
                *contract == self.contract.id
            }
            Encumbrance::Lien { .. } | Encumbrance::Disputed { .. } => false,
        };
        mine && self.contract.consent.terms.deliverables.iter().any(|d| {
            matches!(d, Deliverable::Collateral { object: o, from }
                if *o == object && &contract_encumbrance(self.contract, from) == encumbrance)
        })
    }
}

/// The ids of the holds that really are this contract's escrow.
fn qualifying_ids(c: &EconomicContract, snapshot: &EconomySnapshot) -> Vec<HoldId> {
    qualifying_escrow(c, snapshot)
        .holds
        .iter()
        .map(|h| h.id)
        .collect()
}

/// What this contract's **qualifying** escrow adds up to right now — the same total
/// `settlement_plan` and `forfeit_plan` distribute (M4). An overflow reads as zero,
/// which authorizes nothing.
#[must_use]
fn escrowed_total(c: &EconomicContract, snapshot: &EconomySnapshot) -> Amount {
    let mut total: i128 = 0;
    for hold in qualifying_escrow(c, snapshot).holds {
        total += hold.amount.widen();
    }
    Amount::from_widened(total).unwrap_or(Amount::ZERO)
}

/// Net a payout list per account, so an account named twice by one split is owed the
/// **sum** — which is exactly what the engine asks about, because it authorizes the
/// netted delta per account, never the individual lines.
#[must_use]
fn net_payouts(payouts: &[(AccountId, Amount)]) -> Vec<(AccountId, Amount)> {
    let mut out: BTreeMap<AccountId, Amount> = BTreeMap::new();
    for (account, amount) in payouts {
        let slot = out.entry(*account).or_insert(Amount::ZERO);
        let Some(sum) = slot.checked_add(*amount) else {
            return Vec::new();
        };
        *slot = sum;
    }
    out.into_iter().collect()
}

impl crate::engine::AuthEval for ContractAuth<'_> {
    fn authorize(
        &self,
        _actor: &PrincipalId,
        account: &crate::record::Account,
        delta: Amount,
    ) -> bool {
        if !self.bound() {
            return false;
        }
        if delta.is_negative() {
            // No bare debit authority (finding 1): a contract takes money by
            // CAPTURING its escrow, and the engine excuses exactly that much of the
            // debit on its own. Anything the engine still has to ask about here is,
            // by construction, a line beyond the escrow — an extra pair smuggled
            // into the settlement — and the answer is no.
            return false;
        }
        // M4: the credit must be EXACTLY what this account is owed. The engine asks
        // about the netted delta per account, so one question answers both halves of
        // the rule — "the right amount" and "credited once".
        match &self.purpose {
            ContractPurpose::Settle { payouts, .. } => payouts
                .iter()
                .any(|(id, owed)| *id == account.id && *owed == delta),
            ContractPurpose::Forfeit { to, total } => *to == account.id && *total == delta,
            // An unwind moves no money at all: it hands reservations back, and a
            // release is an effect, not a credit.
            ContractPurpose::Unwind => false,
        }
    }

    fn authorize_hold(&self, _actor: &PrincipalId, hold: &Hold, op: &HoldOp) -> bool {
        if !self.bound() || !self.qualifying.contains(&hold.id) {
            return false;
        }
        match op {
            HoldOp::Capture { hold: id, amount } => {
                !matches!(self.purpose, ContractPurpose::Unwind)
                    && *id == hold.id
                    && !amount.is_negative()
                    && !amount.is_zero()
                    && *amount <= hold.amount
            }
            // Only an unwind gives escrow back: a settlement that could release
            // escrow could break the very agreement it is performing.
            HoldOp::Release { hold: id } => {
                matches!(self.purpose, ContractPurpose::Unwind) && *id == hold.id
            }
            HoldOp::Place { .. } | HoldOp::Expire { .. } => false,
        }
    }

    fn authorize_asset(
        &self,
        _actor: &PrincipalId,
        asset: Option<&crate::record::EconomicAsset>,
        op: &AssetOp,
    ) -> bool {
        if !self.bound() {
            return false;
        }
        let Some(asset) = asset else { return false };
        match op {
            AssetOp::TransferTitle { object, to, .. } => {
                let ContractPurpose::Settle { titles, .. } = &self.purpose else {
                    return false;
                };
                self.contract
                    .consent
                    .terms
                    .deliverables
                    .iter()
                    .enumerate()
                    .any(|(i, d)| {
                        titles.contains(&i)
                            && matches!(d, Deliverable::TitleTransfer { object: o, from, to: t }
                            if o == object
                                && t == to
                                && from == &asset.title_owner
                                // D-E3-12: BOTH sides accepted, or a title lands on
                                // someone who never agreed to its liabilities.
                                && self.contract.accepted_by(from)
                                && self.contract.accepted_by(t))
                    })
            }
            // **Placing** a claim takes something away: an encumbered asset cannot be
            // transferred (invariant 12). So it asks two more questions than a release
            // (E4b2 verify fold, M1): the pledger must hold the title *now*, and must
            // have accepted this contract. A pledge is consent, and consent is the
            // acceptance.
            AssetOp::Encumber {
                object,
                encumbrance,
            } => {
                self.pledges(*object, encumbrance)
                    && self.contract.consent.terms.deliverables.iter().any(|d| {
                        matches!(d, Deliverable::Collateral { object: o, from }
                            if *o == *object
                                && from == &asset.title_owner
                                && self.contract.accepted_by(from))
                    })
            }
            // **Releasing** stays unconditional within the pledge check: a pledge
            // always comes back, whatever happened to the acceptance or the title,
            // or the claim strands on the asset forever.
            AssetOp::Release {
                object,
                encumbrance,
            } => self.pledges(*object, encumbrance),
            _ => false,
        }
    }

    fn authorize_title_receipt(
        &self,
        _actor: &PrincipalId,
        to: &PrincipalId,
        _to_scope: Option<OrgScopeId>,
    ) -> bool {
        let ContractPurpose::Settle { titles, .. } = &self.purpose else {
            return false;
        };
        self.contract.fully_accepted()
            && self.contract.accepted_by(to)
            && self
                .contract
                .consent
                .terms
                .deliverables
                .iter()
                .enumerate()
                .any(|(i, d)| {
                    titles.contains(&i)
                        && matches!(d, Deliverable::TitleTransfer { to: t, .. } if t == to)
                })
    }

    /// The **one** vault op a contract ever authorizes: an unwind returning a
    /// recorded deposit to the principal the journal says put it there.
    ///
    /// This is the replacement for the deleted custody-lock exception (E4a verify
    /// fold, finding 2). The lock has no exception while the contract is live; it
    /// lifts on the state change; and the refund that follows that change is the only
    /// thing that can take the object out, because `WorldAuth` never sees a plan
    /// carrying it and this method is `false` for every other shape.
    fn authorize_vault(
        &self,
        _actor: &PrincipalId,
        vault: &crate::record::Vault,
        op: &CustodyOp,
    ) -> bool {
        if !matches!(self.purpose, ContractPurpose::Unwind) || !self.bound() {
            return false;
        }
        match op {
            CustodyOp::Withdraw {
                object,
                vault: v,
                to,
            } => {
                *v == vault.id
                    && self.contract.deposits.iter().any(|d| {
                        d.object == *object
                            && d.vault == vault.id
                            // The depositor is a world object and the object goes
                            // straight back to them...
                            && (d.deposited_by == PrincipalId::Object(*to)
                                // ...or (E6b, DL-03) the depositor has no world
                                // object at all and `to` is the vault backing this
                                // very authority resolved for them at construction.
                                || self.homes.contains(&(*object, *to)))
                    })
            }
            CustodyOp::Deposit { .. } => false,
        }
    }

    /// E6b (DL-03): the vault-backing destinations this unwind may use. Bounded by
    /// [`MAX_DEPOSITS`], resolved once at construction from the live snapshot, and
    /// never anything but a vault the deposit's own depositor owns.
    fn authorize_vault_home(&self, object: ObjId, to: ObjId) -> bool {
        matches!(self.purpose, ContractPurpose::Unwind)
            && self.bound()
            && self.homes.contains(&(object, to))
    }

    /// **The custody lock's one key** (E4b2 verify fold, M2+M3).
    ///
    /// The lock lifts on the terminal state change, and that used to be the whole
    /// story: `unwind` wrote `Cancelled`/`Expired` first and posted the refund
    /// afterwards. But then a refused refund left a *terminal* contract holding live
    /// escrow, which nothing retries and no state machine can recover — so the fold
    /// inverts the order: **the plan posts first, the state moves after**. The refund
    /// therefore has to run while the contract is still live, with the lock still on,
    /// and something has to be able to open it exactly once.
    ///
    /// This is that key, and it is unforgeable from a command: only
    /// `ContractAuth::unwinding` answers `true`, only for a deposit **this** contract
    /// recorded in **this** vault, and a `ContractAuth` reaches the engine only
    /// through `post_with(.., PostAuth::Injected(..))` from inside this crate.
    /// `WorldAuth` never answers `true`, so no `@vault/withdraw` can ever use it.
    fn authorize_custody_unlock(&self, object: ObjId, vault: VaultId) -> bool {
        matches!(self.purpose, ContractPurpose::Unwind)
            && self.bound()
            && self
                .contract
                .deposits
                .iter()
                .any(|d| d.object == object && d.vault == vault)
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
            outcomes: Vec::new(),
        }
    }

    #[test]
    fn permille_shares_floor_and_the_remainder_is_deterministic() {
        let t = terms(
            vec![
                PayoutShare {
                    to: AccountId(1),
                    share: Share::Permille(333),
                },
                PayoutShare {
                    to: AccountId(2),
                    share: Share::Permille(333),
                },
                PayoutShare {
                    to: AccountId(3),
                    share: Share::Permille(333),
                },
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
                PayoutShare {
                    to: AccountId(1),
                    share: Share::Exact(Amount::from_minor(40)),
                },
                PayoutShare {
                    to: AccountId(2),
                    share: Share::Permille(500),
                },
            ],
            AccountId(9),
        );
        let out = apply_split(Amount::from_minor(100), &t).unwrap();
        assert_eq!(
            out,
            vec![
                (AccountId(1), Amount::from_minor(40)),
                (AccountId(2), Amount::from_minor(30)),
                (AccountId(9), Amount::from_minor(30)),
            ]
        );
        let bad = terms(
            vec![PayoutShare {
                to: AccountId(1),
                share: Share::Exact(Amount::from_minor(500)),
            }],
            AccountId(9),
        );
        assert_eq!(
            apply_split(Amount::from_minor(100), &bad),
            Err(ContractError::SplitInfeasible)
        );
    }
}
