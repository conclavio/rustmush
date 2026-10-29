//! **Procurement: a House buys mind licences from the Roll** (doc 17h §3 M8f; doc 17 §16,
//! D-UM-54; the lead's rulings P1–P9 of the M8f brief, 2026-10-28).
//!
//! **The policy** ([`crate::launch::ProcurementPolicy`], on the launch deployment beside
//! `NamedHouses`): a named House's organisation, its root scope, its **procurement agent** (an
//! NPC character the operator creates, stands at the market's venue and grants
//! `roll.holder.org` limited to the monthly budget), the monthly budget, the backing faction,
//! D-UM-54's diversity cap and the listing forms it may buy. `[tacsim.procurement]` holds the
//! budget's and the cap's defaults ([`crate::config::ProcurementConfig`]).
//!
//! **The tick** (`tacsim.procurement`, [`PROCUREMENT_TICK`]): Coarse, GOD, and it submits
//! nothing authoritative itself (common rules). Each fire, for each policy, it takes at most ONE
//! step, queued as the agent through the port tick's swap ([`crate::port_tick::Fire`], the
//! Occator dispatcher's shape), the next on a later fire once the previous step's outcome is
//! durable (D-18-32: pull, never trust "submitted"):
//! 1. **settle** a funded purchase the ledger holds as funded: its Grant (`+minds/buy/complete`'s
//!    action, `org=`), then the ledger row's phase once the market says Granted or Refunded;
//!    after [`MAX_GRANT_TRIES`] Grants that did not land, its Refund;
//! 2. **record** a funded purchase of the agent's for the House that the ledger lacks (the
//!    ledger row is written before the Grant, so a restart between them loses nothing); while
//!    the House's open purchases are unreadable the visit is refused, never carried on to a buy;
//! 3. **pay** the agent's own live quote (`/buy/pay`'s Fund; never another member's, H1),
//!    re-checked against the budget; a Fund that never lands skips the listing for the month
//!    and pages (H2);
//! 4. **choose and quote**: once per IC calendar month (the period, [`ledger::Period`]: the
//!    civil date's year and month, `CivilDate::from_ic_seconds`, the clock M8c's Unattestable
//!    sentence uses), read the market's current listings (GOD-only, paged within the fire's read
//!    budget, resuming on the next fire), keep the forms in `roles` priced within the month's
//!    remaining budget whose rating has a score (P1; [`rank`]), rank them by score per credit,
//!    skip a charter at its cap (P6), and quote the best (`+minds/buy … org=`'s Quote). Nothing
//!    left, or a treasury that cannot pay the best, closes the month (the ore-bid precedent); a
//!    rating unreadable this fire keeps the month open (the best scored line may still be bought,
//!    but "nothing left" waits until every listed rating has been read).
//!
//! Only listings are read, never the Roll's filings: a filed mind nobody listed is never a
//! candidate (D-UM-54; P5). A refused step pages (a log line, never a player notice) and the
//! policy moves on: a quote that never appears skips its listing for the month. No player verb,
//! no notice, nothing a seller reads beyond an ordinary purchase's receipt (P8).
//!
//! **The budget** (P7): the `tacsim.procurement_ledger` family's rows ([`ledger::ProcurementRow`]),
//! one per purchase, written by this module's `tacsim_procurement` action queued as the agent (the
//! yard's `DispatchLedger` precedent), keyed by the organisation, the period and an ordinal.
//! **Not the E0 ledger** (`mush_core::Ledger`, which P7 named; the lead's ruling B): a tick
//! cannot sum a month's sealed chunks after a restart without cold-read plumbing, and a typed
//! family is durable and read synchronously. A month's spend is the sum of that period's rows
//! not refunded; the budget is a cap on it, never a mint and never a balance. The rows are the
//! memory: every decision is re-read from them, the market and the Roll each fire, so a restart
//! between or within months changes nothing; RAM ([`tick::ProcurementState`]) only paces (the
//! score cache, the month's closed mark, a quote awaited). An in-flight Quote lost to a restart
//! is abandoned by its own expiry (the market's quote window), a funded purchase is resumed from
//! the agent's open purchases.
//!
//! **The cap** (P6, D-UM-54): with `H` titled hulls (the organisation's Active E4 titles whose
//! object is a world hull, `crate::surface::recovery::world_hulls`) the House may hold licences
//! of one charter for at most `floor(cap_permille × H / 1000)` hulls. A mind licence has no
//! per-hull binding in the tree, so **one licence is one hull's seat**: the cap is "current
//! licences of one charter ≤ floor(cap × H / 1000)", counted from the Roll's GOD-only
//! `ProcurementHoldings` read plus the ledger's funded rows not yet granted.
mod ledger;
mod provider;
pub(crate) mod rank;
pub(crate) mod tick;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar, TickCadence};
use mush_core::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef};
use serde_json::{Value, json};

#[cfg(test)]
pub(crate) use ledger::{Period, ProcurementRow, RowPhase};

/// The tick's tag.
pub const PROCUREMENT_TICK: &str = "tacsim.procurement";
/// How often it fires (Coarse): fast enough that a Quote, its Fund and its Grant land well
/// inside the market's quote window, which the Grant also checks.
pub const PROCUREMENT_TICK_PERIOD: Duration = Duration::from_secs(5);
/// The ledger row's action (the agent's own bookkeeping).
pub const PROCUREMENT_ACTION: &str = "tacsim_procurement";
/// Its outcome.
pub const PROCUREMENT_OUTCOME: &str = "tacsim_procurement_result";
/// The ledger family.
pub const PROCUREMENT_FAMILY: &str = "tacsim.procurement_ledger";
/// Grants of one funded purchase that may fail to land before the tick refunds it.
pub const MAX_GRANT_TRIES: u8 = 3;
/// Refunds that may fail to land after them before the tick pages `escrow held` for staff and
/// moves on (the fix round's H3).
pub const MAX_REFUND_TRIES: u8 = 3;

fn mod_error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(mod_error)?,
        DefinitionId::new(id).map_err(mod_error)?,
        1,
        payload,
    )
    .map_err(mod_error)
}

/// The record action's outcome descriptor (`crate::port_tick`'s one outcome validator).
///
/// # Errors
///
/// A descriptor that does not build.
pub fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "outcome",
        PROCUREMENT_OUTCOME,
        json!({"format":1,"fields":["org","operation","phase"]}),
    )
}

/// The record action's descriptor (`crate::port_tick`'s one action validator).
///
/// # Errors
///
/// The outcome unregistered, or a descriptor that does not build.
pub fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(PROCUREMENT_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the procurement outcome did not register"))?;
    descriptor(
        "action",
        PROCUREMENT_ACTION,
        json!({"format":1,"subject":"self_actor","targets":0,"family":PROCUREMENT_FAMILY}),
    )
    .map(|d| d.with_references(vec![outcome]))
}

/// **Register procurement** (doc 17h §3 M8f): the ledger family, the record action's provider,
/// the tick's RAM slot and the Coarse `tacsim.procurement` tick, unconditionally (a world with
/// no policy visits nothing). Called from `crate::port_tick::register_with_coverage`, after the
/// crate's action and outcome definitions registered.
///
/// # Errors
///
/// Any registration the registrar refuses.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<(), ModError> {
    let rows = ledger::register(reg)?;
    let action = actions
        .get(PROCUREMENT_ACTION)
        .cloned()
        .ok_or_else(|| ModError::msg("the procurement action did not register"))?;
    let outcome = outcomes
        .get(PROCUREMENT_OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("the procurement outcome did not register"))?;
    reg.action_provider(
        action.clone(),
        provider::RecordProvider {
            action: action.clone(),
            outcome,
            rows: rows.clone(),
        },
    )?;
    reg.state(tick::ProcurementState::new(rows, action));
    reg.tick(
        PROCUREMENT_TICK,
        TickCadence::Coarse(PROCUREMENT_TICK_PERIOD),
        Arc::new(tick::procurement_tick),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
