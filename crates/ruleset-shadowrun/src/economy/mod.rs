//! **E6a — the Shadowrun run-and-pay layer** (08f §3).
//!
//! Exact personal money, contractual work, and an ephemeral crew — built entirely on
//! top of E4 as shipped, with no economy code of its own.
//!
//! ## The four rules this whole module is shaped by
//!
//! - **All money goes through E4.** The board calls
//!   [`offer_contract`](mush_economy::offer_contract) /
//!   [`accept_contract`](mush_economy::accept_contract) /
//!   [`amend_contract`](mush_economy::amend_contract) /
//!   [`cancel_contract`](mush_economy::cancel_contract); lifestyle calls
//!   [`create_obligation`](mush_economy::create_obligation) /
//!   [`cancel_obligation`](mush_economy::cancel_obligation) /
//!   [`reinstate_obligation`](mush_economy::reinstate_obligation); chargen and
//!   migration are one-shot attributed [`post`](mush_economy::post)s under
//!   deterministic keys. **No `@contract` or `@obligation` command is ever
//!   dispatched**, and there is no direct journal write anywhere in this crate.
//! - **Payment never edits a sheet** (D-E6-16). The `nuyen` field survives as the
//!   chargen budget and the migration source, and nothing else; after approval the
//!   *account* is authoritative (D-E6-1) and `@sheet/set …/nuyen` is refused.
//! - **The party is a record, not a principal** (D-E6-5). `mush-party` depends on
//!   `mush-core` alone and never learns about money; the ruleset holds both ends —
//!   it locks the party against a contract id and releases it when the contract
//!   reaches a terminal state, and the contract never points back at the party.
//! - **Existence hiding on every render.** A run, party, account or profile the actor
//!   may not view is answered the way `mush-economy` answers it, through its own
//!   public predicates and helpers, never with a placeholder.
//!
//! ## Layout
//!
//! | module | what |
//! |---|---|
//! | [`config`] | `[shadowrun.economy]`, frozen at wiring |
//! | [`record`] | `SrProfile`, `RunTemplate`, and the wires they load through |
//! | [`store`] | the module state and its two document families |
//! | [`boot`] | the instrument, the faucets, the `sr.month` clock, the two reconciliation passes |
//! | [`lifestyle`] | the upkeep obligation, the miss/downgrade rule, `+lifestyle/pay` |
//! | [`board`] | `+run/*` over E4b2's roles, outcome table and roster lock |
//! | [`chargen`] | the approval allotment and `@sr/migrate` |
//! | [`observer`] | the board's ear on the economy |
//! | [`commands`] | `+run`, `+lifestyle`, `+nuyen`, `@sr` |
//! | [`catalog`] | E6c: the `sr.core` starter pack and the three `mush-market` tables |

pub mod board;
pub mod boot;
pub mod catalog;
pub mod chargen;
pub mod commands;
pub mod config;
pub mod fence;
pub mod fence_record;
pub mod lifestyle;
pub mod observer;
pub mod record;
pub mod store;

use mush_core::{Role, World, perm};

pub use board::{BoardError, NO_SUCH_RUN, PostError, RunSplit, SplitSlot};
pub use catalog::{
    MAX_PACK_BYTES, SR_CORE_PACK, SrAvailability, SrCatalogSource, SrDelivery, baseline_minor,
    goods_instrument,
};
pub use config::{
    BOARD_CAP, BOARD_PAGE, BOARD_TTL_SECS, FAUCET_CHARGEN, FAUCET_MIGRATION, FENCE_DELAY_MAX_SECS,
    FENCE_DELAY_MIN_SECS, FENCE_QUOTE_TTL_SECS, LIFESTYLE_CATCH_UP, LifestyleTier,
    MAX_CATALOG_CODE_BYTES, MAX_FENCES, MAX_OPEN_QUOTES_PER_SELLER, MAX_OPEN_RUNS_PER_PRINCIPAL,
    MAX_QUOTES, MAX_RUN_OUTCOMES, SINK_LIFESTYLE, SR_MONTH, SR_MONTH_CATCH_UP,
    SR_MONTH_PERIOD_SECS, SR_NUYEN, SR_NUYEN_SCALE, SrEconomyConfig, default_lifestyle_costs,
};
pub use fence::{FenceError, NO_SUCH_FENCE, NO_SUCH_QUOTE};
pub use fence_record::{
    FenceId, FenceQuote, FenceSettlement, PriceFactor, QuoteId, QuoteObserved, SrFence,
};
pub use lifestyle::LifestyleError;
pub use observer::SrBoardObserver;
pub use record::{
    BoardVisibility, InstalledAugmentation, MAX_AUGMENTATIONS, Migration, Mint, RunId, RunNotice,
    RunState, RunTemplate, SrProfile,
};
pub use store::{
    NS, SrEconomyStore, fence_key, load as load_store, profile_key, quote_key, run_key,
};

/// Staff, by the economy's own definition — one rule across the E-series, so
/// "staff" means the same thing in `mush-economy`, `mush-housing` and here.
#[must_use]
pub fn is_staff(w: &World, who: mush_core::ObjId) -> bool {
    perm::has_power(w, who, mush_core::Powers::SIM_ADMIN) || perm::has_role(w, who, Role::Royalty)
}
