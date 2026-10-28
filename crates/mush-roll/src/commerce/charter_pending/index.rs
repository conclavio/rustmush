//! 17g C2f (the C2b brief §4; the C2e plan §3.2): the per-(buyer, charter)
//! pending-charter offer index, `roll_mind_charter_pending_buyer_offers`.
//!
//! It **authorizes nothing** (it only lets C2g's named-offer read find the
//! offers naming the actor), and it is written only inside the protocol's own
//! Offer and Fund transactions, which mush-core's closed owner-index table
//! admits exactly (`terminal_capacity::owner_index`, R2): the Offer's row is
//! the buyer's retained tokens plus the new one; the Fund's is the buyer's
//! row minus the consumed one, deleted when empty. Nothing is written at a
//! Settle, a Refund or a Registry revocation: the token has already left the
//! row at Fund, so "an indexed offer has no operation" holds across any boot.
//!
//! Only the charter's titleholder can make an Offer, so no third party can
//! fill a buyer's row; a ninth live offer for the same (buyer, charter)
//! refuses the seller's Offer before any money moves (no new sentence).
use super::records::{MAX_BUYER_OFFERS, PendingBuyerOffers, PendingOffer};
use crate::commerce::{
    CharterId, families::CommerceFamilies, records::PublicationError, records::Record,
};
use mush_core::{ActionTxn, CommitToken, ObjId};
use std::collections::BTreeSet;

fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}

fn row_of(
    txn: &mut ActionTxn<'_>,
    f: &CommerceFamilies,
    buyer: ObjId,
    charter: CharterId,
) -> Result<Option<PendingBuyerOffers>, PublicationError> {
    let key = PendingBuyerOffers::key_for(buyer, charter);
    let row = txn
        .get(&f.charter_pending_buyer_offers, &key)
        .map_err(bad)?
        .cloned();
    if let Some(row) = &row {
        row.validate()?;
        if row.key() != key || row.buyer != buyer || row.charter != charter {
            return Err(PublicationError::Mismatch);
        }
    }
    Ok(row)
}

fn put(
    txn: &mut ActionTxn<'_>,
    f: &CommerceFamilies,
    existed: bool,
    buyer: ObjId,
    charter: CharterId,
    offers: BTreeSet<CommitToken>,
) -> Result<(), PublicationError> {
    let key = PendingBuyerOffers::key_for(buyer, charter);
    if offers.is_empty() {
        if existed {
            txn.remove(&f.charter_pending_buyer_offers, &key)
                .map_err(bad)?;
        }
        return if txn
            .get(&f.charter_pending_buyer_offers, &key)
            .map_err(bad)?
            .is_none()
        {
            Ok(())
        } else {
            Err(PublicationError::Mismatch)
        };
    }
    let row = PendingBuyerOffers {
        format: 1,
        buyer,
        charter,
        offers,
    };
    row.validate()?;
    if existed {
        txn.replace(&f.charter_pending_buyer_offers, &key, row.clone())
    } else {
        txn.insert(&f.charter_pending_buyer_offers, &key, row.clone())
    }
    .map_err(bad)?;
    if txn
        .get(&f.charter_pending_buyer_offers, &key)
        .map_err(bad)?
        != Some(&row)
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

/// The Offer's write, after the offer row is inserted: the buyer's retained
/// tokens, minus each whose offer has expired at `now` (the economy refuses
/// an acceptance at or after `expires_at_utc`), plus this offer. A ninth live
/// offer refuses (`Capacity`).
pub(super) fn offered(
    txn: &mut ActionTxn<'_>,
    f: &CommerceFamilies,
    offer: &PendingOffer,
    now: i64,
) -> Result<(), PublicationError> {
    let existing = row_of(txn, f, offer.buyer, offer.charter)?;
    let existed = existing.is_some();
    let retained = existing.map(|row| row.offers).unwrap_or_default();
    let live = plan_offer(retained, offer.operation, now, |token| {
        let indexed = txn
            .get(&f.charter_pending_offers, &token.to_hex())
            .map_err(bad)?
            .ok_or(PublicationError::Missing)?;
        if indexed.operation != token
            || indexed.buyer != offer.buyer
            || indexed.charter != offer.charter
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(indexed.expires_at_utc)
    })?;
    put(txn, f, existed, offer.buyer, offer.charter, live)
}

/// The Offer's token set: the buyer's `retained` tokens, minus each whose
/// offer has expired at `now` (`expiry` answers each retained offer's
/// `expires_at_utc`, or refuses one that is missing or names another buyer
/// or charter), plus `offer`; a ninth live offer refuses (`Capacity`). Pure,
/// so the rules are proven apart from the typed families (the tests).
pub(crate) fn plan_offer(
    retained: BTreeSet<CommitToken>,
    offer: CommitToken,
    now: i64,
    mut expiry: impl FnMut(CommitToken) -> Result<i64, PublicationError>,
) -> Result<BTreeSet<CommitToken>, PublicationError> {
    let mut live = BTreeSet::new();
    for token in retained {
        if now < expiry(token)? {
            live.insert(token);
        }
    }
    if live.len() >= MAX_BUYER_OFFERS {
        return Err(PublicationError::Capacity);
    }
    if !live.insert(offer) {
        return Err(PublicationError::Mismatch);
    }
    Ok(live)
}
/// The Fund's token set: `retained` (which must hold `offer`) without it;
/// an empty set is a Delete.
pub(crate) fn plan_consume(
    retained: Option<BTreeSet<CommitToken>>,
    offer: CommitToken,
) -> Result<BTreeSet<CommitToken>, PublicationError> {
    let mut offers = retained.ok_or(PublicationError::Missing)?;
    if !offers.remove(&offer) {
        return Err(PublicationError::Mismatch);
    }
    Ok(offers)
}

/// The Fund's write: the consumed offer leaves its own buyer's row (which
/// must hold it), and the row is deleted when no token is left.
pub(super) fn consumed(
    txn: &mut ActionTxn<'_>,
    f: &CommerceFamilies,
    offer: &PendingOffer,
) -> Result<(), PublicationError> {
    let retained = row_of(txn, f, offer.buyer, offer.charter)?.map(|row| row.offers);
    let offers = plan_consume(retained, offer.operation)?;
    put(txn, f, true, offer.buyer, offer.charter, offers)
}

/// The clock-free boot check of the family (Roll's pending-charter audit):
/// each indexed offer exists (`offer` answers its buyer and charter), names
/// that row's buyer and charter, and has no pending operation (`used`). An
/// expired, unconsumed offer stays indexed (openness depends on the clock).
/// There is no "every open offer is indexed", but under R2 the index row is a
/// precondition of the Fund (`consumed` refuses an offer its buyer's row does
/// not hold, and mush-core's exact set requires the row's exact
/// consumption): an offer whose row was lost boots, cannot be funded, moves
/// no money, and lapses at its expiry.
pub(crate) fn check(
    rows: impl IntoIterator<Item = (String, PendingBuyerOffers)>,
    offer: impl Fn(CommitToken) -> Option<(ObjId, CharterId)>,
    used: &BTreeSet<CommitToken>,
) -> Result<(), PublicationError> {
    for (key, row) in rows {
        row.validate()?;
        if row.key() != key {
            return Err(PublicationError::Mismatch);
        }
        for token in &row.offers {
            if offer(*token) != Some((row.buyer, row.charter)) || used.contains(token) {
                return Err(PublicationError::Mismatch);
            }
        }
    }
    Ok(())
}
