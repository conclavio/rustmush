#![cfg(test)]

//! 17g C2f (the C2b brief §3, tests 7 and 8): the per-(buyer, licence) offer
//! index. No fast fixture can write a paid-resale offer row: mush-core's
//! paid-resale validator refuses any write to the protocol's own families
//! ("resale writes without protocol") unless the whole paid-resale protocol
//! is composed (its Roll families and a birth protocol), which only the
//! composed launches and the M7 stories do. So the index's rules are proven
//! here on the writers' own pure plans (`plan_offer`, `plan_consume`: the
//! exact token sets the Offer, Accept and Fund write, an empty set being the
//! Delete) and the boot audit's own check (`check`), over real offer rows;
//! the writers' typed glue (`offered`, `consumed`) and the audit's scans
//! (`audit`) run in a capturing rig over the real `Families` as far as no
//! offer row is needed (`resale_index_writers_and_audit_run_in_typed_
//! transactions`); mush-core's `index_market_mind_paid_resale_buyer_
//! offers_*` tests prove the same rows are what the transitions admit, and
//! the stories run the providers end to end.
use super::super::families::Families;
use super::super::index::{self, MAX_BUYER_OFFERS, ResaleBuyerOffers};
use super::super::records::MindPaidResaleOffer;
use mush_core::{
    ActionTxn, ClockRef, ClockStamp, CommitToken, DefinitionDigest, Engine, GameModule, ModError,
    ModuleBootData, ModuleCtx, ObjId, Outbox, Registrar, Registry, World,
};
use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_economy::managed_paid_resale::{PaidResaleTermsV1, SETTLEMENT_IC_SECONDS};
use mush_economy::{AccountId, Amount, InstrumentId};
use mush_roll::commerce::LicenceId;
use mush_roll::commerce::paid_resale::{PaidResaleBinding, protocol};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const SELLER: ObjId = ObjId(7);
const BUYER: ObjId = ObjId(8);

fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn digest(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn tokens(ids: &[u8]) -> BTreeSet<CommitToken> {
    ids.iter().map(|n| token(*n)).collect()
}
/// A transferable personal licence's resale binding at its first holder.
fn binding(licence: u64) -> TestResult<PaidResaleBinding> {
    let revision = json!({"format":1,"revision":{"key":{"character":7,"mind":1,"number":1},
        "digest":digest(1)},"source":digest(2)});
    let term = json!({"format":1,"licence":licence.to_string(),"generation":"1",
        "window":{"format":1,"eligibility_start":0,"issued_at":0,"term_end":100_000,"grace_end":100_100},
        "terms":{"seats":1,"class":"II","form":{"kind":"open_resale"},"transferable":true},
        "revision":{"kind":"pin","filed":"3","revision":revision},"operation":token(1)});
    Ok(serde_json::from_value(
        json!({"licence":licence.to_string(),"charter":"9","line":"2",
        "birth":{"route":"entitlement","operation":token(1),"digest":digest(9)},
        "ownership":{"generation":1,"operation":token(1),"digest":digest(9)},
        "term":term,"term_clock":{"kind":"world_ic","generation":1},
        "economics_digest":digest(9),"royalty_chain_digest":digest(8)}),
    )?)
}
/// The seller's offer `id` of `licence` naming `buyer`, offered at
/// `offered` and expiring 1000 seconds later.
pub(in crate::mind_paid_resale) fn offer(
    id: u8,
    buyer: ObjId,
    licence: u64,
    offered: i64,
) -> TestResult<Box<MindPaidResaleOffer>> {
    let binding = binding(licence)?;
    let action = protocol::actions()?[0].clone();
    let row = MindPaidResaleOffer {
        format: 1,
        operation: token(id),
        action: action.clone(),
        terms: PaidResaleTermsV1 {
            format: 1,
            offer: token(id),
            offer_action: action,
            basis: binding.native_basis()?,
            seller: SELLER,
            buyer,
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            price: Amount::from_minor(100),
            registry: GrossRecipientCut {
                permille: 0,
                recipient: None,
            },
            venue: None,
            parents: vec![],
            remainder: Recipient {
                account: AccountId(10),
                owner: mush_org::PrincipalId::Object(SELLER),
            },
            offered_at_ic: ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: offered,
            },
            offered_at_utc: offered,
            expires_at_utc: offered + 1000,
            settle_seconds: SETTLEMENT_IC_SECONDS,
        },
        binding,
    };
    row.validate()?;
    Ok(Box::new(row))
}
/// The Offer's plan over the retained offers' own rows, as the writer reads
/// them (a missing row, or one naming another buyer or licence, refuses).
fn plan(
    retained: &BTreeSet<CommitToken>,
    rows: &BTreeMap<CommitToken, Box<MindPaidResaleOffer>>,
    row: &MindPaidResaleOffer,
    now: i64,
) -> Result<BTreeSet<CommitToken>, String> {
    let (buyer, licence) = (row.terms().buyer, row.binding().licence());
    index::plan_offer(retained.clone(), row.operation(), now, |token| {
        let indexed = rows
            .get(&token)
            .ok_or("indexed paid resale offer missing")?;
        if indexed.terms().buyer != buyer || indexed.binding().licence() != licence {
            return Err("indexed paid resale offer differs".into());
        }
        Ok(indexed.terms().expires_at_utc)
    })
}

/// 17g C2f (the C2b brief test 7): the named buyer's token set after Offer,
/// Accept and Fund, on real offer rows: the Accept of one of two offers
/// keeps a row (a Put), the last consumption empties it (the Delete); an
/// expired offer is pruned at the buyer's next Offer, while a consumed one
/// has already left at its Accept or Fund; a ninth live offer refuses; two
/// buyers' rows are two keys; a row is a bounded, nonempty set.
#[test]
fn resale_buyer_offers_follow_offer_accept_and_fund() -> TestResult {
    let mut rows = BTreeMap::new();
    let a = offer(10, BUYER, 1, 1000)?;
    let b = offer(11, BUYER, 1, 1000)?;
    rows.insert(a.operation(), a.clone());
    let row = plan(&BTreeSet::new(), &rows, &a, 1000)?;
    check_eq!(row, tokens(&[10]));
    rows.insert(b.operation(), b.clone());
    let row = plan(&row, &rows, &b, 1000)?;
    check_eq!(row, tokens(&[10, 11]));
    // Accept of 10: a Put of the rest; Fund of 11: the empty set, a Delete.
    let row = index::plan_consume(Some(row), a.operation())?;
    check_eq!(row, tokens(&[11]));
    let row = index::plan_consume(Some(row), b.operation())?;
    check!(row.is_empty(), "the last consumption deletes the row");
    check!(
        index::plan_consume(None, a.operation()).is_err()
            && index::plan_consume(Some(tokens(&[11])), a.operation()).is_err(),
        "a consumption the row does not hold refuses"
    );
    // Offer 13 expires at 2500; the buyer's next Offer at 3000 prunes it.
    let c = offer(13, BUYER, 1, 1500)?;
    rows.insert(c.operation(), c.clone());
    let row = plan(&BTreeSet::new(), &rows, &c, 1500)?;
    let d = offer(14, BUYER, 1, 3000)?;
    rows.insert(d.operation(), d.clone());
    let row = plan(&row, &rows, &d, 2499)?;
    check_eq!(
        row,
        tokens(&[13, 14]),
        "13 is live one second before expiry"
    );
    let e = offer(15, BUYER, 1, 3000)?;
    rows.insert(e.operation(), e.clone());
    check_eq!(plan(&row, &rows, &e, 3000)?, tokens(&[14, 15]), "13 pruned");
    // A retained offer naming another buyer, or missing, refuses the Offer.
    let foreign = offer(16, ObjId(9), 1, 3000)?;
    rows.insert(foreign.operation(), foreign);
    check!(plan(&tokens(&[16]), &rows, &e, 3000).is_err());
    check!(plan(&tokens(&[99]), &rows, &e, 3000).is_err());
    // Eight live offers; a ninth refuses.
    let mut live = BTreeSet::new();
    for n in 0..MAX_BUYER_OFFERS {
        let o = offer(30 + n as u8, BUYER, 1, 3000)?;
        rows.insert(o.operation(), o.clone());
        live = plan(&live, &rows, &o, 3000)?;
    }
    check_eq!(live.len(), MAX_BUYER_OFFERS);
    let ninth = offer(50, BUYER, 1, 3000)?;
    rows.insert(ninth.operation(), ninth.clone());
    check!(
        plan(&live, &rows, &ninth, 3000).is_err(),
        "a ninth live offer"
    );
    // Two buyers, one licence: two keys.
    let licence = LicenceId::new(1)?;
    check!(
        ResaleBuyerOffers::key_for(BUYER, licence) != ResaleBuyerOffers::key_for(ObjId(9), licence)
    );
    check_eq!(
        ResaleBuyerOffers::key_for(BUYER, licence),
        "00000000000000000008/licence/0000000000000001"
    );
    let full = ResaleBuyerOffers {
        format: 1,
        buyer: BUYER,
        licence,
        offers: live.clone(),
    };
    full.validate()?;
    let mut over = live;
    over.insert(token(99));
    check!(
        ResaleBuyerOffers {
            offers: over,
            ..full.clone()
        }
        .validate()
        .is_err()
    );
    check!(
        ResaleBuyerOffers {
            offers: BTreeSet::new(),
            ..full
        }
        .validate()
        .is_err(),
        "an empty row is a Delete, never a row"
    );
    Ok(())
}

/// 17g C2f (the C2b brief test 8): the boot audit's check refuses an indexed
/// offer that is missing, names another buyer or licence, or has an
/// operation, and a row under a key it does not name; it never reads the
/// clock, so an expired, unconsumed offer passes; a row dropped after an
/// Offer passes the audit (under R2 that offer then cannot be accepted or
/// funded, `consumed` and mush-core's exact set both needing the row, so it
/// moves no money and lapses), while a stale row still naming an accepted
/// offer refuses.
#[test]
fn resale_buyer_offer_audit_is_clock_free() -> TestResult {
    let licence = LicenceId::new(1)?;
    let row = |buyer: ObjId, ids: &[u8]| ResaleBuyerOffers {
        format: 1,
        buyer,
        licence,
        offers: tokens(ids),
    };
    let key = |buyer: ObjId| ResaleBuyerOffers::key_for(buyer, licence);
    // The retained offers (long expired: the check has no clock) and the
    // one an operation accepted.
    let named = |o: &MindPaidResaleOffer| (o.operation(), (o.terms().buyer, o.binding().licence()));
    let offers = BTreeMap::from([
        named(&*offer(10, BUYER, 1, 0)?),
        named(&*offer(11, BUYER, 1, 0)?),
        named(&*offer(12, ObjId(9), 1, 0)?),
        named(&*offer(13, BUYER, 2, 0)?),
    ]);
    let accepted = BTreeSet::from([token(11)]);
    index::check(vec![(key(BUYER), row(BUYER, &[10]))], &offers, &accepted)?;
    check!(
        index::check(Vec::new(), &offers, &accepted).is_ok(),
        "a dropped row boots (its offer can then never be accepted or funded)"
    );
    for (case, rows) in [
        ("missing", vec![(key(BUYER), row(BUYER, &[10, 99]))]),
        ("another buyer", vec![(key(BUYER), row(BUYER, &[10, 12]))]),
        ("another licence", vec![(key(BUYER), row(BUYER, &[10, 13]))]),
        ("accepted", vec![(key(BUYER), row(BUYER, &[10, 11]))]),
        ("crossed key", vec![(key(ObjId(9)), row(BUYER, &[10]))]),
    ] {
        check!(index::check(rows, &offers, &accepted).is_err(), "{case}");
    }
    Ok(())
}

/// The capturing market module: the paid-resale `Families` registered
/// exactly as `Families::register` does, and the index's own boot audit
/// (`index::audit`, the paid-resale boot audit's last step) as its boot.
struct CapturingMarket;
impl GameModule for CapturingMarket {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let families = Families::register(reg)?;
        reg.state(families);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _: ModuleBootData,
    ) -> Result<(), ModError> {
        let families = ctx
            .w
            .module_state::<Families>()
            .ok_or_else(|| ModError::msg("fixture families"))?
            .clone();
        index::audit(ctx, &families).map_err(ModError::msg)
    }
}
/// One typed transaction over the rig's families.
fn change(
    e: &mut Engine,
    f: &Families,
    run: impl FnOnce(&mut ActionTxn<'_>) -> Result<(), String>,
) -> Result<(), String> {
    let mut txn = e
        .action_transaction(&f.buyer_offers)
        .map_err(|e| e.to_string())?;
    run(&mut txn)?;
    let (_, install) = txn.prepare().map_err(|e| e.to_string())?.into_parts();
    e.install_prepared_action(install)
        .map_err(|e| e.to_string())
}
/// The rig's boot audit on its live store.
fn boot(e: &mut Engine) -> Result<(), String> {
    e.module_loaded(
        crate::NS,
        ModuleBootData::default(),
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )
    .map_err(|e| e.to_string())
}
fn indexed(e: &Engine, f: &Families, buyer: ObjId) -> TestResult<Option<BTreeSet<CommitToken>>> {
    let key = (buyer, LicenceId::new(1)?);
    Ok(e.action_state()
        .get(&f.buyer_offers, &key)?
        .map(|row| row.offers.clone()))
}

/// 17g C2f (review F3/T3): the writers' and the audit's typed glue over the
/// real `Families` in real typed transactions. The Offer writer puts the
/// named buyer's first row; the boot audit's scans refuse that row, since
/// no offer row can exist here (a paid-resale offer needs the composed
/// protocol), with the check's own reason; a second Offer, which must read
/// the retained offer's row, refuses and writes nothing; the Accept/Fund
/// writer takes the token out (the Put branch while another token is left,
/// the Delete when none is), refuses an offer the row does not hold or a
/// buyer with no row, and the emptied index boots.
#[test]
fn resale_index_writers_and_audit_run_in_typed_transactions() -> TestResult {
    let mut e = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(CapturingMarket)],
    )?;
    let f = e
        .world
        .module_state::<Families>()
        .ok_or("captured families")?
        .clone();
    let a = offer(10, BUYER, 1, 1000)?;
    let b = offer(11, BUYER, 1, 1000)?;
    change(&mut e, &f, |t| index::offered(t, &f, &a, 1000))?;
    check_eq!(indexed(&e, &f, BUYER)?, Some(tokens(&[10])));
    check_eq!(indexed(&e, &f, SELLER)?, None, "only the named buyer's row");
    check!(
        boot(&mut e).err().is_some_and(
            |e| e.contains("indexed paid resale offer is missing, foreign or accepted")
        ),
        "the audit's scans reach the check"
    );
    let revision = e.action_state().revision();
    check_eq!(
        change(&mut e, &f, |t| index::offered(t, &f, &b, 1000)).err(),
        Some("indexed paid resale offer missing".to_owned()),
        "a retained token whose offer row is missing refuses the Offer"
    );
    check_eq!(e.action_state().revision(), revision, "nothing written");
    // Two tokens in the row (an ordinary typed write; the Offer writer
    // cannot add the second here): the Accept of one is the Put branch.
    let key = (BUYER, LicenceId::new(1)?);
    let both = ResaleBuyerOffers {
        format: 1,
        buyer: BUYER,
        licence: LicenceId::new(1)?,
        offers: tokens(&[10, 11]),
    };
    change(&mut e, &f, |t| {
        t.replace(&f.buyer_offers, &key, both.clone())
            .map_err(|e| e.to_string())
    })?;
    change(&mut e, &f, |t| index::consumed(t, &f, &a))?;
    check_eq!(indexed(&e, &f, BUYER)?, Some(tokens(&[11])), "a Put");
    check_eq!(
        change(&mut e, &f, |t| index::consumed(t, &f, &a)).err(),
        Some("accepted offer is not indexed for its buyer".to_owned())
    );
    let foreign = offer(12, ObjId(9), 1, 1000)?;
    check_eq!(
        change(&mut e, &f, |t| index::consumed(t, &f, &foreign)).err(),
        Some("accepted offer has no buyer index".to_owned())
    );
    change(&mut e, &f, |t| index::consumed(t, &f, &b))?;
    check_eq!(indexed(&e, &f, BUYER)?, None, "the Delete");
    boot(&mut e)?;
    Ok(())
}
