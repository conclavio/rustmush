//! `mush-economy` (E2a) — the pure accountable-value engine.
//!
//! This crate is **world-free**: checked money amounts, the instrument/account/
//! journal record model, the authoritative [`EconomySnapshot`], the pure
//! plan/[`validate`]/[`apply`] posting engine over the nine §7.1 invariants, the
//! canonical [`plan_hash`], and the idempotent reversal builder — all unit-tested on
//! hand-built state, exactly the shipped M32/E1a pure-library shape. The
//! `EconomyModule` that attaches this to the world (the journal on the E0 `Ledger`,
//! the full-bundle `WriteBatch`, the durable money ack-gate, directional boot
//! self-heal) is **E2b**.
//!
//! ## Design spine (reviewed)
//!
//! - **The journal is the sole authority.** Balances, the idempotency index, the
//!   allocators, and the txid index are all projections re-derived from the journal
//!   at boot; a projection that merely lags self-heals, and only real drift / a lost
//!   tail / an unreplayable journal freezes (E2b).
//! - **Money arithmetic is checked, always.** [`Amount`] has no `Add`/`Sub`/`Neg` —
//!   only `checked_*` (R-R3). Validation sums in `i128`; a posted line and projected
//!   balance must fit `i64`.
//! - **Validate is total; apply is infallible.** [`validate`] proves the nine
//!   invariants + per-instrument zero-sum + credit policy + boundedness caps against
//!   one immutable snapshot; [`apply`] then folds the projections without failing.
//! - **`plan_hash` is a deterministic function of the plan alone** — sorted lines,
//!   excluding concurrency metadata — so a retry rebuilt against a re-fetched
//!   snapshot still matches (R-B3).
//! - **Reversal is idempotent + at-most-once** — a deterministic `econ.reverse/{id}`
//!   key plus a `reverses`/`is_reversed` guard (R-B4).
//! - **Authorization is injected + live.** The [`AuthEval`] seam keeps this crate
//!   independent of the org module and is re-evaluated at every post (R-D4).
//!
//! ## E3a — assets, vaults, approvals (still pure)
//!
//! A transaction now carries **money and/or title and custody** in one durable
//! bundle: [`AssetOp`]/[`CustodyOp`] in the plan, [`AssetEffect`]/[`CustodyEffect`]
//! in the journal entry, [`EconomicAsset`]/[`Vault`]/[`ApprovalRequest`] in the
//! snapshot, and invariants 10–22 in [`validate`]. Three spine points:
//!
//! - **Effects are journal authority** (D-E3-1): each [`AssetEffect`] carries the
//!   full post-state, so replay rebuilds title from the journal alone and a dropped
//!   `asset:{obj}` doc can never desync title from the money that bought it. The
//!   `frozen` latch is doc authority, never inside an effect — the `Account.status`
//!   split.
//! - **Approvals are re-derived, never stored as "satisfied"** (D-E3-2): a request
//!   binds the [`plan_hash`], and every recorded approver is re-qualified LIVE
//!   through [`AuthEval::authorize_approver`] at post time.
//! - **Both sides of a title transfer need authority** (D-E3-12): title carries
//!   liabilities that a money credit does not, so receiving is not free.
//!
//! The world-attached half (persistence, boot heal, the custody providers, the
//! commands, the approval workflow) is E3b.
//!
//! ## E3b2 — org wallets, approvals, and the disclosure boundary
//!
//! The live authorization half. Four spine points, all of them about failing
//! **closed** where E2 failed open:
//!
//! - **The money rules moved into the bridge** (§4.4). A negative delta on a
//!   `System` faucet needs staff *in [`WorldAuth`]*, not merely in the command that
//!   happens to build the plan — E2's comment said "gated by the command layer",
//!   which stopped being true the moment `@econ/pay` learned about divisional
//!   wallets. A scoped debit additionally needs the org `Active` and the scope
//!   `Active`; a credit reaches a `Dissolving` org (final settlement) but never a
//!   `Suspended` or `Closed` one.
//! - **Approvals are live, minted, and singular** (§4.6, invariant 17).
//!   [`post`] mints one [`ApprovalRequest`] per plan hash, re-serves it on every
//!   later submission, and consumes it in the same durable bundle as the
//!   transaction; every recorded approver is re-qualified through
//!   [`AuthEval::authorize_approver`] at post time, and the requester never counts.
//! - **Reads are bounded and existence-hiding** (§4.5, D-E3-6). Non-staff may read
//!   what they own or hold a `*.view` capability over; every other refusal renders
//!   as "No such …", so "denied" and "nonexistent" are one message.
//! - **The module refuses shapes it cannot apply atomically.** A prepared plan with
//!   more than one custody effect is refused before any provider runs: custody moves
//!   one provider call at a time, so a refusal on the second would strand the first.

// E2a — the pure library.
mod amount;
mod engine;
mod hash;
mod ids;
mod plan;
mod record;
mod snapshot;

// E4a — clocks, holds, obligations, contracts (still pure).
mod clock;
mod contract;
mod obligation;

// E4b1 — the graded escalation ladder and the one economy sweep tick.
mod escalation;
mod sweep;

// E4b2 — contracts live: the `@contract/*` surface and the settlement/unwind
// machinery the sweep shares with it.
mod contract_live;

// E4c — the seams that connect the economy to the rest of the world: external
// facts, the canon attestation observer, and the post-commit event seam.
mod attest;
mod facts;
mod observer;

// E2b — the world-attached module (the journal on the E0 Ledger, the atomic
// full-bundle post, the strict durable money ack-gate, and directional boot
// self-heal). E3b1 adds the custody seam, asset/vault persistence + boot, the
// destroy gate, and the `@asset`/`@vault` surface.
mod api;
mod boot;
mod commands;
mod config;
mod custody;
mod declared;
mod module;

pub use amount::{Amount, MAX_SCALE};
pub use engine::{
    APPROVAL_GC_AFTER_SECS, APPROVAL_TTL_SECS, AllowAll, AuthEval, CAPABILITIES, CustodyEval,
    ESCALATION_RENOTIFY_SECS, ESCALATION_WINDOW_SECS, FactEval, HOLD_GC_AFTER_SECS,
    MAX_ACTIVE_HOLDS_PER_ACCOUNT, MAX_APPROVERS_PER_REQUEST, MAX_ASSET_OPS_PER_PLAN,
    MAX_CUSTODY_OPS_PER_PLAN, MAX_DESCRIPTION_BYTES, MAX_ENCUMBRANCES_PER_ASSET, MAX_ENTRY_BYTES,
    MAX_HOLD_EXPIRIES_PER_SWEEP, MAX_HOLD_OPS_PER_PLAN, MAX_LINES_PER_TX, MAX_MEMO_BYTES,
    MAX_OBLIGATIONS_PER_SWEEP, MAX_PENDING_PER_ACCOUNT, MAX_PENDING_PER_REQUESTER,
    MAX_POSTS_PER_SWEEP, MAX_REASON_BYTES, NoCustody, NoFacts, ValidateEnv, apply, plan_reversal,
    validate,
};
pub use hash::plan_hash;
pub use ids::{
    AccountId, ApprovalRequestId, CadenceId, ContractId, EscalationId, HoldId, InstrumentId,
    ObligationId, TransactionId, VaultId,
};
pub use plan::{
    AccountRevision, ApprovalNeed, AssetOp, AssetRevision, CustodyOp, EconomyError, EconomyPlan,
    HoldOp, HoldRevision, NamedResource, PlanResult, Prepared, RequestedLine,
};
pub use record::{
    Account, AccountStatus, AcquisitionRecord, Approval, ApprovalRequest, ApprovalRule,
    ApprovalStatus, AssetAssignment, AssetEffect, AssetEffectKind, AssetLifecycle, CatalogRef,
    CausalRef, ContractStateKind, CreditPolicy, CustodyDirection, CustodyEffect, CustodyRef,
    Decision, EconomicAsset, EconomicCause, EconomicEvent, EconomicTransaction, EconomicVisibility,
    Encumbrance, ExternalRef, Hold, HoldEffect, HoldEffectKind, HoldPurpose, HoldStatus,
    IdempotencyKey, IdempotencyRecord, InstrumentDef, InstrumentKind, InstrumentStatus,
    JournalLine, MAX_AUDIENCE, NegativePolicy, ObligationStatusKind, Vault, VaultStatus, Verdict,
    object_principal,
};
pub use snapshot::EconomySnapshot;

// E4a — clocks, holds, obligations, contracts.
pub use clock::{
    CADENCE_GENERATION_INITIAL, CADENCE_SCHEMA_DECLARED, CADENCE_SCHEMA_LEGACY, CadenceClock,
    CadenceSource, Clocks, DEFAULT_CATCH_UP, EconomicClock, EconomicDeadline, EconomicSchedule,
    MAX_CATCH_UP_CAP, SWEEP_GAP_DOWNTIME_SECS, SWEEP_PERIOD_SECS, advance, advance_cadence,
    advance_cadence_manual, cadence_elapsed, due_periods, reanchor,
};
pub use contract::{
    Acceptance, Attestation, AttestorRef, AttestorSet, CanonOutcomeKinds, ConsentSurface,
    Consideration, ContractAuth, ContractError, ContractPurpose, ContractRoles, ContractState,
    ContractTerms, DISPUTE_TTL_SECS, DRAFT_TTL_SECS, Deliverable, DepositRecord, EconomicContract,
    EvidenceKind, ExpenseTreatment, Expiry, MAX_AGREEMENTS, MAX_AMENDMENTS, MAX_CONDITIONS,
    MAX_CONTRACT_NAME_BYTES, MAX_DELIVERABLES, MAX_DEPOSITS, MAX_LIVE_CONTRACTS_PER_PARTY,
    MAX_OPEN_CONTRACTS_PER_ISSUER, MAX_OUTCOMES, MAX_PARTIES, MAX_SETTLE_ATTEMPTS, MAX_SHARES,
    ObligationTemplate, OutcomeTerms, PERMILLE, PartyAgreement, PayoutShare, PendingKind,
    PendingTerminal, QualifyingEscrow, Readiness, Resolution, STALE_CONTRACT_AGE_SECS, Share,
    TypedCondition, activate_key, activation_asset_ops, apply_outcome, apply_split,
    canon_set_accepts, canonical_visibility, check_offerable, check_structure, condition_holds,
    conditions_met, consent_hash, contract_encumbrance, contract_op_key, forfeit_key, forfeit_plan,
    outcome_conditions, principal_key, qualifying_escrow, readiness, refund_key, refund_plan,
    release_asset_ops, resolve_outcome, reverify, settle_key, settlement_plan, stamp_observed,
};
pub use obligation::{
    DEFAULT_GRACE_PERIODS, ForgiveCause, GracePolicy, MAX_GRACE_PERIODS, MAX_OBLIGATION_HISTORY,
    MAX_SUSPENSION_KEY_BYTES, Obligation, ObligationCause, ObligationError, ObligationEvent,
    ObligationStatus, SettleOutcome, SuspendReason, SuspensionKey, SweepIntent,
    advance as advance_obligation, classify_forgiveness, due_now, forgive, forgive_all, hold,
    obligation_key, plan_sweep, release,
};

// E4b1 verify fold (M1) — the ruleset's configurable limits. SP1 (D-LBK-7) — the
// declared cadences and their composition-frozen clock provider.
pub use config::{
    DEFAULT_MINT_OBLIGATION_CAP_MAJOR, DeclaredCadence, DeclaredCadenceClock, DeclaredCadenceError,
    EconomyConfig, InstrumentCaps, MAX_DECLARED_CADENCES, declared_cadence_clock_id,
};
pub use declared::{DeclaredCadenceProvider, cadence_by_name};

// E4b1 — the ladder + the sweep.
pub use escalation::{
    ESCALATION_GC_AFTER_SECS, Escalation, EscalationKind, EscalationSubject, MAX_BOOT_ESCALATIONS,
    MAX_ESCALATION_CONTEXT_BYTES, MAX_RECOMMENDED_COMMANDS, escalation_key,
};
pub use sweep::SWEEP_TAG;

// E4b2 — contracts live. E5b adds the lease seam a layered module offers under.
pub use contract_live::{LeaseEnd, LeaseKey, LeaseSpec, end_lease, may_view_contract, offer_lease};

// E4c — facts, events, evidence.
pub use attest::MAX_OBSERVER_SETTLEMENTS_PER_EVENT;
pub use facts::{FactProvider, FactState, FactTable, WorldFacts};
pub use module::CANON_TAIL;
pub use observer::{EconomyObserver, MAX_OBSERVER_DEPTH, ObserverTable, observer_depth};
pub use sweep::MAX_REPUBLISH_PER_SWEEP;

// E2b/E3b1 public surface.
pub use api::{
    CadenceError, MAX_CADENCE_NAME_BYTES, MAX_CADENCES, META_KEY, ObligationSpec, PostOutcome,
    account_key, acknowledge_escalation, approval_key, asset_key, cadence_key, contract_key,
    create_cadence, create_system_obligation, create_vault, hold_key, instrument_key, issue_title,
    mark_canon_published, obligation_key as obligation_doc_key, open_account, open_escalation,
    post, post_capability, register_instrument, resume_obligation, reverse, set_account_approval,
    set_account_status, set_asset_frozen, set_economy_frozen, set_system_obligation,
    set_vault_status, suspend_obligation, vault_key,
};
pub use commands::{may_view_account, may_view_hold, may_view_obligation};
pub use custody::{ContainerProvider, CustodyError, CustodyProvider, CustodyTable};
pub use module::{
    CHECKPOINT_REVIEW_ENTRIES, CHECKPOINT_REVIEW_SECS, EconomyModule, EconomyScopeUse,
    EconomyStore, JOURNAL_LOG, NS,
};
