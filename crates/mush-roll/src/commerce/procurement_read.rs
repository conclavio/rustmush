//! Doc 17h §3 M8f (the lead's rulings P1 and P6): the two Roll reads behind
//! tacsim's procurement tick, folded into `holding_reader::OriginalOwnerReader`'s
//! command kinds as every TacSim command read is (17g C2 plan §2.4 fallback 1).
//!
//! **GOD only.** A tick runs as GOD (core dispatches every module tick's reads
//! as GOD), so these two kinds answer the calling actor only when it is GOD and
//! refuse everyone else, identically to a missing row; no player command can
//! build one. Neither reveals anything to a player surface: the tick ranks and
//! counts with them and prints nothing.
//!
//! - `ProcurementTally { rating, from }`: the [`RatingTally`] of up to
//!   [`TALLY_PAGES_PER_READ`] result pages of the exact rating `rating` names,
//!   from page `from`, and where the next read starts. The receipt is bound by
//!   its exact reference (`RatingFamilies::resolve_receipt_read`), and each
//!   page's content digest is checked against the receipt's own page list, so
//!   a page is counted only when it is that receipt's page.
//! - `ProcurementHoldings { org, after }`: the organisation's current paid
//!   licences, in licence order after `after` (its `roll_mind_paid_holdings`
//!   prefix, so no neighbour's row is read): each licence's charter, left out
//!   when its current term is lapsed or revoked. The diversity cap counts these
//!   per charter (D-UM-54).
use super::families::CommerceFamilies;
use super::paid_holdings::{PaidHolding, holder_prefix, page_start};
use super::rating::{
    RatingFamilies, RatingPage, RatingPageKey, RatingReceipt, RatingRef, RatingTally,
};
use super::read::CommerceRead;
use super::records::Record;
use super::{CharterId, Holder, LicenceId, TermPhase, WorldIcSeconds};
use mush_core::GOD;
use mush_core::read_service::{OwnerReadContext, OwnerReadError};
use serde::Serialize;

/// Result pages one `ProcurementTally` read counts at most (inside the
/// command kinds' declared row ceiling).
pub const TALLY_PAGES_PER_READ: u16 = 32;
/// Licences one `ProcurementHoldings` read answers at most; each costs its
/// pointer, its term and its revocation mark.
pub const HOLDINGS_PER_READ: usize = 12;

/// One `ProcurementTally` answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ProcurementTallyPage {
    tally: RatingTally,
    next: Option<u16>,
}
impl ProcurementTallyPage {
    #[must_use]
    pub const fn tally(&self) -> RatingTally {
        self.tally
    }
    /// The page the next read starts from; `None` once every page is counted.
    #[must_use]
    pub const fn next(&self) -> Option<u16> {
        self.next
    }
}

/// One licence the organisation holds now, by its charter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ProcurementHolding {
    licence: LicenceId,
    charter: CharterId,
}
impl ProcurementHolding {
    #[must_use]
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    #[must_use]
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
}

/// One `ProcurementHoldings` answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProcurementHoldingsPage {
    rows: Vec<ProcurementHolding>,
    next: Option<LicenceId>,
}
impl ProcurementHoldingsPage {
    #[must_use]
    pub fn rows(&self) -> &[ProcurementHolding] {
        &self.rows
    }
    #[must_use]
    pub const fn next(&self) -> Option<LicenceId> {
        self.next
    }
}

/// The gate both kinds open with: GOD (the tick) alone; every other caller,
/// a player's command or a panel, is `Refused` before anything is read.
fn god(actor: mush_core::ObjId) -> Result<(), OwnerReadError> {
    if actor == GOD {
        Ok(())
    } else {
        Err(OwnerReadError::Refused)
    }
}

/// `ProcurementTally` (the module note).
pub(super) fn tally(
    ctx: &OwnerReadContext<'_>,
    ratings: &RatingFamilies,
    rating: &RatingRef,
    from: u16,
) -> Result<ProcurementTallyPage, OwnerReadError> {
    god(ctx.actor())?;
    let receipt = ratings
        .resolve_receipt_read(ctx, rating)
        .map_err(|_| OwnerReadError::Refused)?;
    let keys = window_keys(receipt, from)?;
    let found = ctx
        .get_many(&ratings.pages, &keys)
        .map_err(|_| OwnerReadError::Unavailable)?;
    count_window(receipt, from, &found)
}

/// The page keys of the read window that starts at page `from` (at most
/// [`TALLY_PAGES_PER_READ`]); `Refused` when `from` is past the last page.
pub(crate) fn window_keys(
    receipt: &RatingReceipt,
    from: u16,
) -> Result<Vec<RatingPageKey>, OwnerReadError> {
    let refs = receipt.result().pages();
    let start = usize::from(from);
    if start >= refs.len() {
        return Err(OwnerReadError::Refused);
    }
    let end = start
        .saturating_add(usize::from(TALLY_PAGES_PER_READ))
        .min(refs.len());
    refs.get(start..end)
        .ok_or(OwnerReadError::Refused)?
        .iter()
        .map(|r| RatingPageKey::new(receipt.publication().job(), r.ordinal()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| OwnerReadError::Integrity)
}

/// Count the window `from` of `receipt` over the pages read for its keys
/// (`window_keys`), in order: each must be present, keyed as asked and carry
/// the content digest the receipt lists for it (`Integrity` otherwise), and
/// the answer names the page the next read starts from.
pub(crate) fn count_window(
    receipt: &RatingReceipt,
    from: u16,
    found: &[Option<&RatingPage>],
) -> Result<ProcurementTallyPage, OwnerReadError> {
    let keys = window_keys(receipt, from)?;
    let refs = receipt.result().pages();
    let start = usize::from(from);
    let end = start.saturating_add(keys.len());
    let wanted = refs.get(start..end).ok_or(OwnerReadError::Integrity)?;
    if found.len() != keys.len() {
        return Err(OwnerReadError::Integrity);
    }
    let mut tally = RatingTally::default();
    for ((key, page), reference) in keys.iter().zip(found).zip(wanted) {
        let page = page.ok_or(OwnerReadError::Integrity)?;
        if page.key() != *key
            || page
                .content()
                .digest()
                .map_err(|_| OwnerReadError::Integrity)?
                != reference.digest()
        {
            return Err(OwnerReadError::Integrity);
        }
        tally = tally
            .add_page(page.content())
            .ok_or(OwnerReadError::Integrity)?;
    }
    let next = (end < refs.len())
        .then(|| u16::try_from(end).ok())
        .flatten();
    Ok(ProcurementTallyPage { tally, next })
}

/// `ProcurementHoldings` (the module note).
pub(super) fn holdings(
    ctx: &OwnerReadContext<'_>,
    families: &CommerceFamilies,
    org: mush_org::OrgId,
    after: Option<LicenceId>,
) -> Result<ProcurementHoldingsPage, OwnerReadError> {
    god(ctx.actor())?;
    // The index's key prefix is the principal's; `acting` is no part of it.
    let holder = Holder::Organization { org, acting: GOD };
    let start = page_start(holder, after);
    let scanned = ctx.scan_action_state(
        &families.paid_holdings,
        Some(start.as_str()),
        HOLDINGS_PER_READ + 1,
    )?;
    let prefix = format!("{}/", holder_prefix(holder));
    let mine: Vec<(String, PaidHolding)> = scanned
        .entries()
        .iter()
        .take_while(|entry| entry.encoded_key().starts_with(&prefix))
        .map(|entry| (entry.encoded_key().to_owned(), entry.value().clone()))
        .collect();
    let now = WorldIcSeconds(ctx.world().game_now_unix());
    let mut rows = Vec::new();
    let mut last = None;
    for (key, held) in mine.iter().take(HOLDINGS_PER_READ) {
        if held.validate().is_err() || !held.holder().same_principal(holder) || held.key() != *key {
            return Err(OwnerReadError::Integrity);
        }
        last = Some(held.licence());
        let Some(pointer) = ctx
            .get(&families.paid_licences, &held.licence().key())
            .map_err(|_| OwnerReadError::Unavailable)?
        else {
            continue;
        };
        if pointer.id() != held.licence() || !pointer.holder().same_principal(holder) {
            continue;
        }
        let Some(term) = ctx
            .get(
                &families.paid_terms,
                &format!("{}/{}", held.licence().key(), pointer.term().key()),
            )
            .map_err(|_| OwnerReadError::Unavailable)?
        else {
            continue;
        };
        if term.matches_current(pointer).is_err()
            || matches!(term.window().phase_at(now), TermPhase::Lapsed)
        {
            continue;
        }
        let revoked = ctx
            .get(
                &families.revoked_terms,
                &super::registry_revocation::records::RevokedTerm::key_for(
                    held.licence(),
                    term.generation(),
                ),
            )
            .map_err(|_| OwnerReadError::Unavailable)?
            .is_some();
        if !revoked {
            rows.push(ProcurementHolding {
                licence: held.licence(),
                charter: pointer.charter(),
            });
        }
    }
    let next = if mine.len() > HOLDINGS_PER_READ {
        last
    } else {
        None
    };
    Ok(ProcurementHoldingsPage { rows, next })
}

#[cfg(test)]
mod tests {
    use super::god;
    use mush_core::read_service::OwnerReadError;
    use mush_core::{GOD, ObjId};

    /// Doc 17h §3 M8f: the procurement kinds answer GOD alone; a player, the
    /// Registry's clerk or any other character is refused like a missing row.
    #[test]
    fn the_procurement_kinds_refuse_every_caller_but_god() -> Result<(), Box<dyn std::error::Error>>
    {
        check_eq!(god(GOD), Ok(()));
        for caller in [ObjId(0), ObjId(2), ObjId(50), ObjId(u32::MAX)] {
            check_eq!(god(caller), Err(OwnerReadError::Refused), "{caller:?}");
        }
        Ok(())
    }
}
