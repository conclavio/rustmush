//! 17g C2f (the C2b brief §3; the C2e plan §3.2): the per-(buyer, licence)
//! paid-resale offer index, `market_mind_paid_resale_buyer_offers`.
//!
//! One row per buyer and licence with a live seller offer naming that buyer,
//! keyed `{buyer:020}/licence/{licence:016x}` (a 45-byte key, hence the
//! family's 64-byte key limit). It **authorizes nothing**: it only lets C2g's
//! named-offer read find the offers naming the actor; the Accept and Fund
//! providers still reread the offer row and recheck its named buyer. It is
//! written only inside the protocol's own transactions, which mush-core's
//! closed owner-index table admits exactly (`terminal_capacity::owner_index`,
//! R2):
//! - **Offer**: the buyer's retained tokens, minus each whose offer has
//!   expired at the world clock (the economy refuses an acceptance at or after
//!   `expires_at_utc`), plus the new one; a ninth live offer refuses the
//!   seller's Offer (no money moves at Offer).
//! - **Accept and Fund**: the accepted offer leaves its own `(terms.buyer,
//!   licence)` row, which is deleted when empty.
//!
//! Nothing is written at a Settle, Refund or Registry revocation: the token
//! left the row at Accept or Fund. Only the licence's holder can make an
//! Offer, so no third party can fill a buyer's row.
use super::families::Families;
use super::records::MindPaidResaleOffer;
use mush_core::mechanics::CanonicalId;
use mush_core::{
    ActionTxn, CommitToken, ModError, ObjId, Registrar, StateFamilyDescriptor, StateFamilyHandle,
    StateFamilyId,
};
use mush_roll::commerce::LicenceId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The C2b bound on one buyer's live offers for one licence.
pub(super) const MAX_BUYER_OFFERS: usize = 8;

/// The live seller offers naming one buyer for one licence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResaleBuyerOffers {
    pub(super) format: u16,
    pub(super) buyer: ObjId,
    pub(super) licence: LicenceId,
    pub(super) offers: BTreeSet<CommitToken>,
}
impl ResaleBuyerOffers {
    pub const FAMILY: &'static str = "market_mind_paid_resale_buyer_offers";
    pub const MAX_BYTES: usize = 2048;
    /// The row key, `{buyer:020}/licence/{licence:016x}`.
    #[must_use]
    pub fn key_for(buyer: ObjId, licence: LicenceId) -> String {
        format!("{:020}/{}", buyer.0, licence.key())
    }
    #[must_use]
    pub fn key(&self) -> String {
        Self::key_for(self.buyer, self.licence)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format != 1
            || self.buyer.0 == 0
            || self.offers.is_empty()
            || self.offers.len() > MAX_BUYER_OFFERS
        {
            return Err("invalid paid resale buyer offer index".into());
        }
        Ok(())
    }
}

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// The family's registration (in `Families::register`). Its key carries the
/// buyer and licence, so its key limit is 64 bytes, not the 32 of the
/// token-keyed resale families.
pub(super) fn register(
    reg: &mut Registrar<'_>,
) -> Result<StateFamilyHandle<(ObjId, LicenceId), ResaleBuyerOffers>, ModError> {
    let family = ResaleBuyerOffers::FAMILY;
    let id = |suffix| CanonicalId::new(format!("{family}.{suffix}.v1")).map_err(error);
    let d = StateFamilyDescriptor::new(
        StateFamilyId::new(family).map_err(error)?,
        id("key")?,
        id("value")?,
        1,
        id("decode")?,
        id("validate")?,
        id("canonical")?,
    )
    .and_then(|d| d.with_limits(64, ResaleBuyerOffers::MAX_BYTES))
    .map_err(error)?;
    reg.state_family(
        d,
        |key: &(ObjId, LicenceId)| Ok(ResaleBuyerOffers::key_for(key.0, key.1)),
        |value: &serde_json::Value| {
            let bytes =
                mush_core::canonical_json_bytes_with_limit(value, ResaleBuyerOffers::MAX_BYTES)
                    .map_err(|e| e.to_string())?;
            let row: ResaleBuyerOffers =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            row.validate()?;
            if serde_json::to_value(&row).map_err(|e| e.to_string())? != *value {
                return Err("noncanonical paid resale buyer offer index".into());
            }
            Ok(row)
        },
        ResaleBuyerOffers::validate,
        |row: &ResaleBuyerOffers| serde_json::to_value(row).map_err(|e| e.to_string()),
    )
}

fn row_of(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    buyer: ObjId,
    licence: LicenceId,
) -> Result<Option<ResaleBuyerOffers>, String> {
    let row = txn
        .get(&f.buyer_offers, &(buyer, licence))
        .map_err(|e| e.to_string())?
        .cloned();
    if let Some(row) = &row {
        row.validate()?;
        if row.buyer != buyer || row.licence != licence {
            return Err("crossed paid resale buyer offer index".into());
        }
    }
    Ok(row)
}

fn put(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    existed: bool,
    buyer: ObjId,
    licence: LicenceId,
    offers: BTreeSet<CommitToken>,
) -> Result<(), String> {
    let key = (buyer, licence);
    if offers.is_empty() {
        if existed {
            txn.remove(&f.buyer_offers, &key)
                .map_err(|e| e.to_string())?;
        }
        return if txn
            .get(&f.buyer_offers, &key)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            Ok(())
        } else {
            Err("paid resale buyer offer post-image differs".into())
        };
    }
    let row = ResaleBuyerOffers {
        format: 1,
        buyer,
        licence,
        offers,
    };
    row.validate()?;
    if existed {
        txn.replace(&f.buyer_offers, &key, row.clone())
    } else {
        txn.insert(&f.buyer_offers, &key, row.clone())
    }
    .map_err(|e| e.to_string())?;
    if txn.get(&f.buyer_offers, &key).map_err(|e| e.to_string())? != Some(&row) {
        return Err("paid resale buyer offer post-image differs".into());
    }
    Ok(())
}

/// The Offer's token set: the buyer's `retained` tokens, minus each whose
/// offer has expired at `now` (`expiry` answers each retained offer's
/// `expires_at_utc`, or refuses one that is missing or names another buyer
/// or licence), plus `offer`. A ninth live offer refuses. Pure, so the rules
/// are proven apart from the typed families (`buyer_offers` tests).
pub(super) fn plan_offer(
    retained: BTreeSet<CommitToken>,
    offer: CommitToken,
    now: i64,
    mut expiry: impl FnMut(CommitToken) -> Result<i64, String>,
) -> Result<BTreeSet<CommitToken>, String> {
    let mut live = BTreeSet::new();
    for token in retained {
        if now < expiry(token)? {
            live.insert(token);
        }
    }
    if live.len() >= MAX_BUYER_OFFERS {
        return Err("paid resale buyer offer capacity unavailable".into());
    }
    if !live.insert(offer) {
        return Err("paid resale offer already indexed".into());
    }
    Ok(live)
}
/// The Accept's and Fund's token set: `retained` (which must hold `offer`)
/// without it; an empty set is a Delete.
pub(super) fn plan_consume(
    retained: Option<BTreeSet<CommitToken>>,
    offer: CommitToken,
) -> Result<BTreeSet<CommitToken>, String> {
    let mut offers = retained.ok_or("accepted offer has no buyer index")?;
    if !offers.remove(&offer) {
        return Err("accepted offer is not indexed for its buyer".into());
    }
    Ok(offers)
}

/// The Offer's write, after the offer row is inserted (`plan_offer` over
/// the buyer's row and each retained offer's own row).
pub(super) fn offered(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    offer: &MindPaidResaleOffer,
    now: i64,
) -> Result<(), String> {
    let buyer = offer.terms().buyer;
    let licence = offer.binding().licence();
    let existing = row_of(txn, f, buyer, licence)?;
    let existed = existing.is_some();
    let retained = existing.map(|row| row.offers).unwrap_or_default();
    let live = plan_offer(retained, offer.operation(), now, |token| {
        let indexed = txn
            .get(&f.offers, &token)
            .map_err(|e| e.to_string())?
            .ok_or("indexed paid resale offer missing")?;
        if indexed.operation() != token
            || indexed.terms().buyer != buyer
            || indexed.binding().licence() != licence
        {
            return Err("indexed paid resale offer differs".into());
        }
        Ok(indexed.terms().expires_at_utc)
    })?;
    put(txn, f, existed, buyer, licence, live)
}

/// The Accept's and Fund's write: the accepted offer leaves its own named
/// buyer's row (which must hold it); the row goes when it is empty.
pub(super) fn consumed(
    txn: &mut ActionTxn<'_>,
    f: &Families,
    offer: &MindPaidResaleOffer,
) -> Result<(), String> {
    let buyer = offer.terms().buyer;
    let licence = offer.binding().licence();
    let retained = row_of(txn, f, buyer, licence)?.map(|row| row.offers);
    let offers = plan_consume(retained, offer.operation())?;
    put(txn, f, true, buyer, licence, offers)
}

/// The clock-free boot audit of the index (the paid-resale boot audit's last
/// step, and the rig's whole boot): every retained seller offer's named
/// buyer and licence, the offers a retained operation accepted, and every
/// index row, scanned in bounded pages, then `check`.
pub(super) fn audit(ctx: &mush_core::ModuleCtx<'_>, f: &Families) -> Result<(), String> {
    let mut offers = BTreeMap::new();
    scan(ctx, &f.offers, |key, offer| {
        if key != offer.operation().to_hex() {
            return Err("paid resale offer key differs".into());
        }
        offers.insert(
            offer.operation(),
            (offer.terms().buyer, offer.binding().licence()),
        );
        Ok(())
    })?;
    let mut used = BTreeSet::new();
    scan(ctx, &f.operations, |key, operation| {
        if key != operation.operation().to_hex() {
            return Err("paid resale operation key differs".into());
        }
        used.insert(operation.offer);
        Ok(())
    })?;
    let mut rows = Vec::new();
    scan(ctx, &f.buyer_offers, |key, row| {
        rows.push((key.to_owned(), row.clone()));
        Ok(())
    })?;
    check(rows, &offers, &used)
}

fn scan<K: Send + Sync + 'static, R: Clone + Send + Sync + 'static>(
    ctx: &mush_core::ModuleCtx<'_>,
    family: &StateFamilyHandle<K, R>,
    mut inspect: impl FnMut(&str, &R) -> Result<(), String>,
) -> Result<(), String> {
    let mut after: Option<String> = None;
    let mut count = 0_usize;
    loop {
        let page = ctx
            .scan_action_state(family, after.as_deref(), 128)
            .map_err(|e| e.to_string())?;
        for entry in page.entries() {
            count = count.checked_add(1).ok_or("paid resale index count")?;
            if count > 65536 {
                return Err("paid resale index bound".into());
            }
            inspect(entry.encoded_key(), entry.value())?;
        }
        let Some(next) = page.next_after() else {
            return Ok(());
        };
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err("nonprogressing paid resale page".into());
        }
        after = Some(next.to_string());
    }
}

/// The clock-free boot check: `rows` are the index rows with their stored
/// keys, `offers` every retained seller offer's named buyer and licence, and
/// `used` the offers an operation accepted. Each indexed offer exists, names
/// that buyer and licence, and has no operation. There is no "every open
/// offer is indexed": openness depends on the clock. Under R2 the index row
/// is a precondition of the Accept and the Fund (`consumed` refuses an offer
/// its buyer's row does not hold, and mush-core's exact set requires the
/// row's exact consumption), so an offer whose row was lost boots, cannot be
/// accepted or funded, moves no money, and lapses at its expiry.
pub(super) fn check(
    rows: impl IntoIterator<Item = (String, ResaleBuyerOffers)>,
    offers: &BTreeMap<CommitToken, (ObjId, LicenceId)>,
    used: &BTreeSet<CommitToken>,
) -> Result<(), String> {
    for (key, row) in rows {
        row.validate()?;
        if row.key() != key {
            return Err("paid resale buyer offer key differs".into());
        }
        for token in &row.offers {
            if offers.get(token) != Some(&(row.buyer, row.licence)) || used.contains(token) {
                return Err("indexed paid resale offer is missing, foreign or accepted".into());
            }
        }
    }
    Ok(())
}
