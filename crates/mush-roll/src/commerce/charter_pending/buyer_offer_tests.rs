#![cfg(test)]

//! 17g C2f (the C2b brief §4, test 11): the per-(buyer, charter) pending
//! offer index. No fast fixture can write a pending offer row: mush-core's
//! pending-charter validator refuses any write to the protocol's own
//! families ("pending typed write without protocol") unless the
//! pending-charter protocol is composed with its owners, which only the
//! composed launches and the M7 stories do. So, as the resale twin
//! (mush-market's `buyer_offers` tests), the rules are proven on the
//! writers' own pure plans (`plan_offer`, `plan_consume`: the exact token
//! sets the Offer and Fund write, an empty set being the Delete) and the
//! boot audit's own `check`, over real offer rows; the writers' typed glue
//! (`offered`, `consumed`) runs over the real `CommerceFamilies` as far as
//! no offer row is needed (`pending_index_writers_run_in_typed_
//! transactions`); mush-core's `index_roll_mind_charter_pending_
//! buyer_offers_*` tests prove these are the rows the transitions admit.
//! Roll's pending-charter audit, whose scan feeds `check`, needs the Roll
//! and economy stores and runs only in the composed launches.
use super::super::index::{self, plan_consume, plan_offer};
use super::super::records::{MAX_BUYER_OFFERS, PendingBuyerOffers, PendingOffer};
use crate::commerce::CharterId;
use crate::commerce::families::CommerceFamilies;
use crate::commerce::records::{PublicationError, Record};
use mush_core::{
    ActionTxn, CommitToken, Engine, GameModule, ModError, ObjId, Registrar, Registry, World,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const BUYER: ObjId = ObjId(8);

fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn tokens(ids: &[u8]) -> BTreeSet<CommitToken> {
    ids.iter().map(|n| token(*n)).collect()
}
/// The seller's pending offer `id` of `charter` naming `buyer`, offered at
/// `offered` and expiring 1000 seconds later (`wide_offer`'s shape).
pub(crate) fn offer(id: u8, buyer: ObjId, charter: u32, offered: i64) -> Result<PendingOffer> {
    let mut value = serde_json::to_value(super::wide_offer()?)?;
    value["operation"] = serde_json::json!(token(id));
    value["buyer"] = serde_json::json!(buyer);
    value["seller"] = serde_json::json!(7);
    value["charter"] = serde_json::json!(charter.to_string());
    value["offered_at_utc"] = serde_json::json!(offered);
    value["expires_at_utc"] = serde_json::json!(offered + 1000);
    let row: PendingOffer = serde_json::from_value(value)?;
    row.validate()?;
    Ok(row)
}
/// The Offer's plan over the retained offers' own rows, as the writer reads
/// them (a missing row, or one naming another buyer or charter, refuses).
fn plan(
    retained: &BTreeSet<CommitToken>,
    rows: &BTreeMap<CommitToken, PendingOffer>,
    row: &PendingOffer,
    now: i64,
) -> std::result::Result<BTreeSet<CommitToken>, PublicationError> {
    plan_offer(retained.clone(), row.operation, now, |token| {
        let indexed = rows.get(&token).ok_or(PublicationError::Missing)?;
        if indexed.buyer != row.buyer || indexed.charter != row.charter {
            return Err(PublicationError::Mismatch);
        }
        Ok(indexed.expires_at_utc)
    })
}

/// 17g C2f (the C2b brief test 11): as the resale twin, for the pending
/// charter. The named buyer's token set after Offer and Fund (a Put while a
/// token is left, the Delete when none is); an expired offer pruned at the
/// buyer's next Offer, a funded one already gone; a ninth live offer refuses
/// (`Capacity`); two buyers' rows are two keys; a row is a bounded, nonempty
/// set. The audit's check refuses a missing, foreign or funded indexed offer
/// and a crossed key, and never reads the clock: an expired, unconsumed
/// offer passes, and a dropped row passes (under R2 that offer then cannot
/// be funded, `consumed` and mush-core's exact set both needing the row, so
/// it moves no money and lapses).
#[test]
fn pending_charter_buyer_offers_follow_offer_and_fund() -> Result {
    let mut rows = BTreeMap::new();
    let a = offer(10, BUYER, 100, 1000)?;
    let b = offer(11, BUYER, 100, 1000)?;
    rows.insert(a.operation, a.clone());
    rows.insert(b.operation, b.clone());
    let row = plan(&BTreeSet::new(), &rows, &a, 1000)?;
    let row = plan(&row, &rows, &b, 1000)?;
    check_eq!(row, tokens(&[10, 11]));
    let row = plan_consume(Some(row), a.operation)?;
    check_eq!(row, tokens(&[11]), "a Put: one token left");
    check!(
        plan_consume(Some(row), b.operation)?.is_empty(),
        "the Delete"
    );
    check!(plan_consume(None, a.operation).is_err());
    // Offer 12 expires at 2500; the buyer's next Offer at 2500 prunes it.
    let c = offer(12, BUYER, 100, 1500)?;
    let d = offer(13, BUYER, 100, 2000)?;
    rows.insert(c.operation, c.clone());
    rows.insert(d.operation, d.clone());
    let row = plan(&BTreeSet::new(), &rows, &c, 1500)?;
    check_eq!(plan(&row, &rows, &d, 2499)?, tokens(&[12, 13]));
    check_eq!(plan(&row, &rows, &d, 2500)?, tokens(&[13]), "12 pruned");
    let foreign = offer(14, ObjId(9), 100, 1000)?;
    rows.insert(foreign.operation, foreign);
    check!(matches!(
        plan(&tokens(&[14]), &rows, &d, 2000),
        Err(PublicationError::Mismatch)
    ));
    let mut live = BTreeSet::new();
    for n in 0..MAX_BUYER_OFFERS {
        let o = offer(30 + n as u8, BUYER, 100, 3000)?;
        rows.insert(o.operation, o.clone());
        live = plan(&live, &rows, &o, 3000)?;
    }
    let ninth = offer(50, BUYER, 100, 3000)?;
    rows.insert(ninth.operation, ninth.clone());
    check!(matches!(
        plan(&live, &rows, &ninth, 3000),
        Err(PublicationError::Capacity)
    ));
    // Keys and rows.
    let charter = CharterId::new(100)?;
    check_eq!(
        PendingBuyerOffers::key_for(BUYER, charter),
        "00000000000000000008/charter/0000000000000064"
    );
    check!(crate::commerce::families::valid_key(
        PendingBuyerOffers::FAMILY,
        &PendingBuyerOffers::key_for(BUYER, charter)
    ));
    check!(
        PendingBuyerOffers::key_for(ObjId(9), charter)
            != PendingBuyerOffers::key_for(BUYER, charter)
    );
    let full = PendingBuyerOffers {
        format: 1,
        buyer: BUYER,
        charter,
        offers: live,
    };
    full.validate()?;
    check!(
        PendingBuyerOffers {
            offers: BTreeSet::new(),
            ..full.clone()
        }
        .validate()
        .is_err(),
        "an empty row is a Delete, never a row"
    );
    // The audit's check: clock-free.
    let named = |o: &PendingOffer| (o.operation, (o.buyer, o.charter));
    let offers: BTreeMap<_, _> = [
        offer(10, BUYER, 100, 0)?,
        offer(11, BUYER, 100, 0)?,
        offer(14, ObjId(9), 100, 0)?,
        offer(15, BUYER, 101, 0)?,
    ]
    .iter()
    .map(named)
    .collect();
    let funded = BTreeSet::from([token(11)]);
    let row = |ids: &[u8]| PendingBuyerOffers {
        format: 1,
        buyer: BUYER,
        charter,
        offers: tokens(ids),
    };
    let key = PendingBuyerOffers::key_for(BUYER, charter);
    index::check(
        vec![(key.clone(), row(&[10]))],
        |t| offers.get(&t).copied(),
        &funded,
    )?;
    index::check(Vec::new(), |t| offers.get(&t).copied(), &funded)?;
    for (case, rows) in [
        ("missing", vec![(key.clone(), row(&[10, 99]))]),
        ("another buyer", vec![(key.clone(), row(&[10, 14]))]),
        ("another charter", vec![(key.clone(), row(&[10, 15]))]),
        ("funded", vec![(key.clone(), row(&[10, 11]))]),
        (
            "crossed key",
            vec![(PendingBuyerOffers::key_for(ObjId(9), charter), row(&[10]))],
        ),
    ] {
        check!(
            index::check(rows, |t| offers.get(&t).copied(), &funded).is_err(),
            "{case}"
        );
    }
    Ok(())
}

struct Capture(Arc<Mutex<Option<CommerceFamilies>>>);
impl GameModule for Capture {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let f = CommerceFamilies::register(reg)?;
        *self.0.lock().map_err(|_| ModError::msg("families"))? = Some(f);
        Ok(())
    }
}
/// One typed transaction over the rig's Roll families.
fn change(
    e: &mut Engine,
    f: &CommerceFamilies,
    run: impl FnOnce(&mut ActionTxn<'_>) -> std::result::Result<(), PublicationError>,
) -> Result {
    let mut txn = e.action_transaction(&f.charter_pending_buyer_offers)?;
    run(&mut txn)?;
    let (_, install) = txn.prepare()?.into_parts();
    e.install_prepared_action(install)?;
    Ok(())
}
fn indexed(
    e: &Engine,
    f: &CommerceFamilies,
    buyer: ObjId,
) -> Result<Option<BTreeSet<CommitToken>>> {
    let key = PendingBuyerOffers::key_for(buyer, CharterId::new(100)?);
    Ok(e.action_state()
        .get(&f.charter_pending_buyer_offers, &key)?
        .map(|row| row.offers.clone()))
}

/// 17g C2f (review F3): the writers' typed glue over the real
/// `CommerceFamilies` in real typed transactions. The Offer writer puts the
/// named buyer's first row (no other key); a second Offer, which must read
/// the retained offer's row (none can exist here), refuses `Missing` and
/// writes nothing; the Fund writer takes the token out (the Put branch while
/// another token is left, the Delete when none is) and refuses an offer the
/// row does not hold (`Mismatch`) or a buyer with no row (`Missing`).
#[test]
fn pending_index_writers_run_in_typed_transactions() -> Result {
    let shared = Arc::new(Mutex::new(None));
    let mut e = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Capture(shared.clone()))],
    )?;
    let f = shared
        .lock()
        .map_err(|_| "families")?
        .clone()
        .ok_or("families")?;
    let a = offer(10, BUYER, 100, 1000)?;
    let b = offer(11, BUYER, 100, 1000)?;
    change(&mut e, &f, |t| index::offered(t, &f, &a, 1000))?;
    check_eq!(indexed(&e, &f, BUYER)?, Some(tokens(&[10])));
    check_eq!(
        indexed(&e, &f, ObjId(7))?,
        None,
        "only the named buyer's row"
    );
    let revision = e.action_state().revision();
    let refused = change(&mut e, &f, |t| index::offered(t, &f, &b, 1000));
    check!(
        refused
            .err()
            .and_then(|e| e.downcast_ref::<PublicationError>().copied())
            == Some(PublicationError::Missing),
        "a retained token whose offer row is missing refuses the Offer"
    );
    check_eq!(e.action_state().revision(), revision, "nothing written");
    let charter = CharterId::new(100)?;
    let key = PendingBuyerOffers::key_for(BUYER, charter);
    let both = PendingBuyerOffers {
        format: 1,
        buyer: BUYER,
        charter,
        offers: tokens(&[10, 11]),
    };
    change(&mut e, &f, |t| {
        t.replace(&f.charter_pending_buyer_offers, &key, both.clone())
            .map_err(|_| PublicationError::Mismatch)
    })?;
    change(&mut e, &f, |t| index::consumed(t, &f, &a))?;
    check_eq!(indexed(&e, &f, BUYER)?, Some(tokens(&[11])), "a Put");
    for (case, row, expected) in [
        (
            "an offer the row does not hold",
            &a,
            PublicationError::Mismatch,
        ),
        (
            "a buyer with no row",
            &offer(12, ObjId(9), 100, 1000)?,
            PublicationError::Missing,
        ),
    ] {
        let refused = change(&mut e, &f, |t| index::consumed(t, &f, row));
        check!(
            refused
                .err()
                .and_then(|e| e.downcast_ref::<PublicationError>().copied())
                == Some(expected),
            "{case}"
        );
    }
    change(&mut e, &f, |t| index::consumed(t, &f, &b))?;
    check_eq!(indexed(&e, &f, BUYER)?, None, "the Delete");
    Ok(())
}
