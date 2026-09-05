//! The board's ear on the economy (08f §2's wiring, D-E6-5, the housing precedent).
//!
//! One [`EconomyObserver`], frozen into `EconomyModule::with_observers` at wiring
//! when `[rulesets].enabled` names `shadowrun`. It does two things and nothing else:
//!
//! - **mirrors `ContractChanged` onto the listing** — `Active` ⇒ `Crewed`, a terminal
//!   state ⇒ `Closed` — and releases the crewed party through `mush_party`'s seam,
//!   which is the *party-side* back-reference D-E6-5 permits (the contract never
//!   points at the party);
//! - **drives lifestyle from `ObligationChanged`** — the miss/downgrade rule of
//!   §3.2 "Life", keyed on the record's own `missed_total` delta.
//!
//! It is exhaustive, `?`-free and panic-free, as every observer handler must be: an
//! observer that unwinds takes the post that emitted the event with it.

use mush_core::Ctx;
use mush_economy::{ContractStateKind, EconomicEvent, EconomyObserver, EconomyStore};

use crate::economy::config::SrEconomyConfig;
use crate::economy::record::{RunId, RunState};
use crate::economy::store::{SrEconomyStore, put_run};

/// The Shadowrun board's economy observer.
#[derive(Clone, Debug)]
pub struct SrBoardObserver {
    cfg: SrEconomyConfig,
}

impl SrBoardObserver {
    /// The observer for a server running `cfg`.
    #[must_use]
    pub fn new(cfg: SrEconomyConfig) -> SrBoardObserver {
        SrBoardObserver { cfg }
    }
}

impl EconomyObserver for SrBoardObserver {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        if !self.cfg.enabled {
            return;
        }
        match ev {
            EconomicEvent::ContractChanged { id, state, .. } => {
                on_contract(ctx, *id, *state);
                // E6b (B9): the fence's own mirror — a quote whose deal reached a
                // terminal state is forgotten, the same trade `del_run` makes for a
                // closed listing. It is a projection of a price anyone can ask for
                // again, so losing it costs a row and never an escrow.
                on_fence_contract(ctx, *id, *state);
            }
            EconomicEvent::ObligationChanged { id, status_kind } => {
                crate::economy::lifestyle::on_obligation_changed(ctx, &self.cfg, *id, *status_kind);
            }
            // E6b (B2): a title record changed. The board does not care — a run
            // moves no title — and neither does the fence, which buys **custody**
            // and never title (D-E6-10): a `@asset/stolen` report on an object under
            // an open quote changes no price, because a hidden fact contributes
            // nothing to one (D-E6-11). The arm is named so that stays deliberate.
            EconomicEvent::AssetTitleChanged { .. } => {}
            // Ids and kinds only (D-E4-13), and none of the other three says
            // anything about a run: a posted transaction, a reservation and a staff
            // page are all summarised by the two records above.
            EconomicEvent::TransactionPosted { .. }
            | EconomicEvent::HoldChanged { .. }
            | EconomicEvent::EscalationOpened { .. } => {}
        }
    }
}

/// Mirror one contract transition onto the listing it belongs to.
fn on_contract(ctx: &mut Ctx<'_>, id: mush_economy::ContractId, state: ContractStateKind) {
    let Some(run) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.run_of_contract(id).cloned())
    else {
        return;
    };
    let next = match state {
        // A `Draft` is the window between an A′ amendment and its re-offer, and an
        // `Offered` contract after a crew was added is `Crewing`. Neither walks the
        // board backwards.
        ContractStateKind::Draft | ContractStateKind::Offered => run.state,
        // **`Active` is not by itself `Crewed`** (S1 deviation 2): a one-party
        // contract activates the moment its author accepts, so a `Listed` run's
        // contract is `Active` with the payer's escrow standing and nobody else
        // bound. The board is `Crewed` only once a crew is on it.
        ContractStateKind::Active | ContractStateKind::ReadyToSettle => {
            if run.party.is_some() {
                RunState::Crewed
            } else {
                run.state
            }
        }
        ContractStateKind::Settled
        | ContractStateKind::Disputed
        | ContractStateKind::Cancelled
        | ContractStateKind::Expired => RunState::Closed,
    };
    if next == run.state {
        return;
    }
    let party = run.party;
    let run_id = run.id;
    let contract = run.contract;
    let mut run = run;
    run.state = next;
    if next.is_closed() {
        run.applied.clear();
    }
    run.revision = run.revision.saturating_add(1);
    // A closed listing is **deleted** rather than written (E6a verify fold, D1);
    // `put_run` routes it. The back-references are read above, before it goes.
    put_run(ctx, run);
    if !next.is_closed() {
        return;
    }
    // The party's lifecycle ends with its contract (§3.2): `Settled` when the run
    // paid out, `Released` for every other terminal state.
    let how = match state {
        ContractStateKind::Settled => mush_party::DisbandCause::Settled,
        _ => mush_party::DisbandCause::Released,
    };
    crate::economy::board::release_party(ctx, party, how);
    announce_closure(ctx, run_id, contract);
}

/// Forget a quote whose contract reached a terminal state (E6b, B9).
fn on_fence_contract(ctx: &mut Ctx<'_>, id: mush_economy::ContractId, state: ContractStateKind) {
    if !matches!(
        state,
        ContractStateKind::Settled
            | ContractStateKind::Cancelled
            | ContractStateKind::Expired
            | ContractStateKind::Disputed
    ) {
        return;
    }
    let Some(quote) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.quote_of_contract(id))
    else {
        return;
    };
    // A **disputed** deal keeps its quote: the goods and the money are both still in
    // play, and deleting the record would leave the seller with no id to name.
    if matches!(state, ContractStateKind::Disputed) {
        return;
    }
    crate::economy::fence::close_quote(ctx, quote);
    tracing::info!(
        quote = quote.0,
        contract = id.0,
        "sr.economy: a fence quote closed with its contract"
    );
}

/// Say, in the log, that a listing closed. The people who need telling are told by
/// the economy itself: parties get the contract's own notices and an organization's
/// officers get DL-04's `OrgInbox` line, with no Shadowrun code at all.
///
/// The contract id is **passed in**: the listing document is gone by the time this
/// runs (D1), so it cannot be looked up from the board any more.
fn announce_closure(ctx: &Ctx<'_>, id: RunId, contract: mush_economy::ContractId) {
    let live = ctx.w.module_state::<EconomyStore>().and_then(|s| {
        s.snapshot()
            .contract(contract)
            .map(|c| format!("{:?}", c.state.kind()))
    });
    tracing::info!(
        run = id.0,
        contract = contract.0,
        state = live.unwrap_or_else(|| "gone".to_string()),
        "sr.economy: a run listing closed with its contract, and its board entry was removed"
    );
}
