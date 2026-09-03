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

// E2a — the pure library.
mod amount;
mod engine;
mod hash;
mod ids;
mod plan;
mod record;
mod snapshot;

// E2b — the world-attached module (the journal on the E0 Ledger, the atomic
// full-bundle post, the strict durable money ack-gate, and directional boot
// self-heal).
mod api;
mod boot;
mod commands;
mod module;

pub use amount::{Amount, MAX_SCALE};
pub use engine::{
    AllowAll, AuthEval, MAX_DESCRIPTION_BYTES, MAX_ENTRY_BYTES, MAX_LINES_PER_TX, MAX_MEMO_BYTES,
    apply, plan_reversal, validate,
};
pub use hash::plan_hash;
pub use ids::{AccountId, InstrumentId, TransactionId};
pub use plan::{AccountRevision, EconomyError, EconomyPlan, PlanResult, Prepared, RequestedLine};
pub use record::{
    Account, AccountStatus, CausalRef, CreditPolicy, EconomicCause, EconomicTransaction,
    EconomicVisibility, IdempotencyKey, IdempotencyRecord, InstrumentDef, InstrumentKind,
    InstrumentStatus, JournalLine, NegativePolicy, object_principal,
};
pub use snapshot::EconomySnapshot;

// E2b public surface.
pub use api::{
    META_KEY, PostOutcome, account_key, instrument_key, open_account, post, post_capability,
    register_instrument, reverse,
};
pub use module::{EconomyModule, EconomyStore, JOURNAL_LOG, NS};
