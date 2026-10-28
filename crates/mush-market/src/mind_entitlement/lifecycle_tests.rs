#![cfg(test)]

//! 17g C2f (the C2b brief §1, tests 1–5): the per-buyer entitlement index
//! over real typed transactions and the real boot audit (`index::audit`, the
//! entitlement boot audit's own last step), in `mind_trade/lifecycle_tests`'
//! idiom: a capturing market module registers the quote, operation and index
//! families exactly as `Families::register` does, the index writers run in
//! ordinary typed transactions beside inert structural quote and purchase
//! rows, and each step's exact post-images are hydrated into a fresh engine
//! and booted. No row here is paid authority. What this rig does NOT run:
//! the providers' arms (`offers.rs`' Quote, `provider.rs`' Fund, Grant and
//! Refund) that call these writers with their arguments (the actor, the
//! quote's clock, the purchase's own `quote.buyer`), and `Families::
//! audit_with_house`, whose last step is `index::audit` (it needs the
//! economy and Roll's sale reader). Those run only in the composed
//! launches and the M7 stories; mush-core's `owner_index` tests prove the
//! Grant's and Refund's exact row at the transition, a GOD refund included.
use super::codec::Row;
use super::families::{actors_family, token_family};
use super::index::{self, EntitlementActorIndex, MAX_BUYER_PURCHASES, MAX_BUYER_QUOTES};
use super::tests::{fixture, purchase};
use super::{EntitlementPurchaseV1, EntitlementQuoteV1};
use mush_core::{
    ActionTxn, CommitToken, Engine, GameModule, ModError, ModuleBootData, ModuleCtx, ModuleWrite,
    ObjId, Outbox, Registrar, Registry, StateFamilyHandle, World,
};
use mush_economy::managed_entitlement::EntitlementProtocol;
use mush_roll::commerce::paid::PaidForm;
use mush_roll::commerce::purchase::PurchasePhase;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Disk = BTreeMap<String, Value>;

#[derive(Clone)]
struct Handles {
    quotes: StateFamilyHandle<CommitToken, EntitlementQuoteV1>,
    operations: StateFamilyHandle<CommitToken, EntitlementPurchaseV1>,
    actors: StateFamilyHandle<index::BuyerKey, EntitlementActorIndex>,
}
impl Handles {
    fn index(&self) -> index::Index<'_> {
        index::Index {
            quotes: &self.quotes,
            operations: &self.operations,
            actors: &self.actors,
        }
    }
}
struct CapturingMarket;
impl GameModule for CapturingMarket {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let handles = Handles {
            quotes: token_family(reg)?,
            operations: token_family(reg)?,
            actors: actors_family(reg)?,
        };
        reg.state(handles);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _: ModuleBootData,
    ) -> Result<(), ModError> {
        let handles = ctx
            .w
            .module_state::<Handles>()
            .ok_or_else(|| ModError::msg("fixture handles"))?
            .clone();
        index::audit(ctx, &handles.index()).map_err(ModError::msg)
    }
}
fn rig() -> TestResult<(Box<Engine>, Handles)> {
    let e = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(CapturingMarket)],
    )?;
    let h = e
        .world
        .module_state::<Handles>()
        .ok_or("captured handles")?
        .clone();
    Ok((Box::new(e), h))
}
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
const BUYER: ObjId = ObjId(8);

/// A quote by `buyer`, published as `id`, expiring at `expires`.
fn quote(id: u8, buyer: ObjId, expires: i64) -> TestResult<Box<EntitlementQuoteV1>> {
    let mut q = fixture(EntitlementProtocol::OriginalV3, 0, PaidForm::Sealed {})?;
    q.publication = token(id);
    q.buyer = buyer.into();
    q.expires_at_utc = expires;
    q.validate()?;
    Ok(Box::new(q))
}
/// The funded purchase of `q` as operation `op`.
fn funded(q: Box<EntitlementQuoteV1>, op: u8) -> TestResult<Box<EntitlementPurchaseV1>> {
    let mut p = purchase(*q)?;
    p.operation = token(op);
    p.updated_by = token(op);
    p.validate()?;
    Ok(Box::new(p))
}
/// `p` granted (or refunded) by the terminal commit `commit`.
fn closed(
    p: &EntitlementPurchaseV1,
    commit: u8,
    grant: bool,
) -> TestResult<Box<EntitlementPurchaseV1>> {
    let mut p = Box::new(p.clone());
    p.phase = if grant {
        PurchasePhase::Granted {
            commit: token(commit),
        }
    } else {
        PurchasePhase::Refunded {
            commit: token(commit),
        }
    };
    p.updated_by = token(commit);
    p.validate()?;
    Ok(p)
}

/// Run one typed transaction; fold its exact writes into `disk`.
fn change(
    e: &mut Engine,
    h: &Handles,
    disk: &mut Disk,
    run: impl FnOnce(&mut ActionTxn<'_>, &index::Index<'_>) -> Result<(), String>,
) -> Result<(), String> {
    let mut txn = e.action_transaction(&h.actors).map_err(|e| e.to_string())?;
    run(&mut txn, &h.index())?;
    let (ops, install) = txn.prepare().map_err(|e| e.to_string())?.into_parts();
    e.install_prepared_action(install)
        .map_err(|e| e.to_string())?;
    for op in ops {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                disk.insert(key.to_string(), value);
            }
            ModuleWrite::Del { key } => {
                disk.remove(key.as_str());
            }
            _ => return Err("typed fixture emitted a non-document write".into()),
        }
    }
    Ok(())
}
/// The Quote step: the quote row and the buyer's index write, at `now`.
fn quote_step(
    e: &mut Engine,
    h: &Handles,
    disk: &mut Disk,
    q: &EntitlementQuoteV1,
    now: i64,
) -> Result<(), String> {
    change(e, h, disk, |t, i| {
        t.insert(i.quotes, &q.publication, q.clone())
            .map_err(|e| e.to_string())?;
        index::quoted(t, i, index::BuyerKey::of(q.buyer), q.publication, now)
    })
}
/// The Fund step: the quote consumed into its operation, and the index.
fn fund_step(
    e: &mut Engine,
    h: &Handles,
    disk: &mut Disk,
    p: &EntitlementPurchaseV1,
) -> Result<(), String> {
    change(e, h, disk, |t, i| {
        t.remove(i.quotes, &p.quote.publication)
            .map_err(|e| e.to_string())?;
        t.insert(i.operations, &p.operation, p.clone())
            .map_err(|e| e.to_string())?;
        index::funded(
            t,
            i,
            index::BuyerKey::of(p.quote.buyer),
            p.quote.publication,
            p.operation,
        )
    })
}
/// The Grant or Refund step: the closed operation, and the index release
/// from the operation's own buyer's row.
fn terminal_step(
    e: &mut Engine,
    h: &Handles,
    disk: &mut Disk,
    after: &EntitlementPurchaseV1,
) -> Result<(), String> {
    change(e, h, disk, |t, i| {
        t.replace(i.operations, &after.operation, after.clone())
            .map_err(|e| e.to_string())?;
        index::released(
            t,
            i,
            index::BuyerKey::of(after.quote.buyer),
            after.operation,
        )
    })
}
fn row(e: &Engine, h: &Handles, buyer: ObjId) -> TestResult<Option<EntitlementActorIndex>> {
    Ok(e.action_state()
        .get(&h.actors, &index::BuyerKey::Character(buyer))?
        .cloned())
}
fn tokens(ids: &[u8]) -> BTreeSet<CommitToken> {
    ids.iter().map(|n| token(*n)).collect()
}
/// The writer's refusal text, when `step` refused.
fn refusal(step: Result<(), String>) -> Option<String> {
    step.err()
}
/// Hydrate `disk` into a fresh rig and run its boot audit.
fn boot(disk: &Disk) -> TestResult {
    let (mut e, _) = rig()?;
    e.hydrate_action_state(disk.iter().map(|(key, value)| {
        mush_core::ActionStateBootRecord::new(crate::NS, key.clone(), value.clone())
    }))?;
    e.module_loaded(
        crate::NS,
        ModuleBootData::default(),
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )?;
    Ok(())
}

/// 17g C2f (the C2b brief test 1): the buyer's row after each step. Two
/// quotes, the first funded and granted (the second quote keeps the row: a
/// Put), the second expired and pruned by the buyer's next Quote, that quote
/// funded and refunded, and the empty row deleted. The release is handed
/// the purchase's own `quote.buyer`, as the provider's Refund arm hands it
/// (a GOD-invoked refund is proven at the transition by mush-core's
/// `a_god_refund_edits_the_buyers_actors_row_not_gods`; this rig has no
/// invocation).
#[test]
fn buyer_index_follows_quote_fund_grant_and_refund() -> TestResult {
    let (mut e, h) = rig()?;
    let mut disk = Disk::new();
    let a = quote(3, BUYER, 160)?;
    let b = quote(5, BUYER, 160)?;
    quote_step(&mut e, &h, &mut disk, &a, 100)?;
    quote_step(&mut e, &h, &mut disk, &b, 100)?;
    check_eq!(row(&e, &h, BUYER)?.ok_or("row")?.quotes, tokens(&[3, 5]));
    let pa = funded(a, 4)?;
    fund_step(&mut e, &h, &mut disk, &pa)?;
    let r = row(&e, &h, BUYER)?.ok_or("row")?;
    check_eq!((r.quotes, r.operations), (tokens(&[5]), tokens(&[4])));
    terminal_step(&mut e, &h, &mut disk, &*closed(&pa, 6, true)?)?;
    let r = row(&e, &h, BUYER)?.ok_or("the unfunded quote keeps the row")?;
    check_eq!((r.quotes, r.operations), (tokens(&[5]), tokens(&[])));
    // The buyer's next Quote, after quote 5 expired, prunes it.
    let c = quote(7, BUYER, 360)?;
    quote_step(&mut e, &h, &mut disk, &c, 300)?;
    check_eq!(row(&e, &h, BUYER)?.ok_or("row")?.quotes, tokens(&[7]));
    let pc = funded(c, 9)?;
    fund_step(&mut e, &h, &mut disk, &pc)?;
    // The Refund: the writer is handed the purchase's own buyer.
    terminal_step(&mut e, &h, &mut disk, &*closed(&pc, 10, false)?)?;
    check!(row(&e, &h, BUYER)?.is_none(), "the empty row is deleted");
    boot(&disk)?;
    Ok(())
}

/// 17g C2f (the C2b brief test 2): the ninth live quote and the ninth open
/// purchase refuse with the writers' capacity refusals (internal texts the
/// providers return through their existing refusal: no new sentence), and
/// the refused step writes nothing.
#[test]
fn buyer_index_bounds_live_quotes_and_open_purchases() -> TestResult {
    let (mut e, h) = rig()?;
    let mut disk = Disk::new();
    for n in 0..MAX_BUYER_QUOTES {
        quote_step(
            &mut e,
            &h,
            &mut disk,
            &*quote(10 + n as u8, BUYER, 160)?,
            100,
        )?;
    }
    let revision = e.action_state().revision();
    check_eq!(
        refusal(quote_step(
            &mut e,
            &h,
            &mut disk,
            &*quote(30, BUYER, 160)?,
            100
        )),
        Some("entitlement buyer quote capacity unavailable".to_owned())
    );
    check_eq!(e.action_state().revision(), revision, "nothing written");
    // Another buyer is unaffected.
    quote_step(&mut e, &h, &mut disk, &*quote(31, ObjId(9), 160)?, 100)?;
    // Fund all eight; a ninth funded purchase refuses.
    for n in 0..MAX_BUYER_PURCHASES {
        let q = quote(10 + n as u8, BUYER, 160)?;
        fund_step(&mut e, &h, &mut disk, &*funded(q, 50 + n as u8)?)?;
    }
    quote_step(&mut e, &h, &mut disk, &*quote(32, BUYER, 160)?, 100)?;
    let revision = e.action_state().revision();
    check_eq!(
        refusal(fund_step(
            &mut e,
            &h,
            &mut disk,
            &*funded(quote(32, BUYER, 160)?, 70)?
        )),
        Some("entitlement buyer purchase capacity unavailable".to_owned())
    );
    check_eq!(e.action_state().revision(), revision, "nothing written");
    boot(&disk)?;
    Ok(())
}

/// 17g C2f (the C2b brief test 3): the clock-free check refuses an indexed
/// quote that is missing, names another buyer or was funded; an indexed
/// operation that is missing or no longer open; and an open operation that
/// is not indexed. The clean state passes, and boots after a reboot.
#[test]
fn entitlement_audit_refuses_a_missing_foreign_or_consumed_index_entry() -> TestResult {
    let row = |quotes: &[u8], operations: &[u8]| EntitlementActorIndex {
        format: 1,
        buyer: index::BuyerKey::Character(BUYER),
        quotes: tokens(quotes),
        operations: tokens(operations),
    };
    let c = index::BuyerKey::Character;
    let quotes = BTreeMap::from([(token(3), c(BUYER)), (token(5), c(ObjId(9)))]);
    let funded_quotes = BTreeSet::from([token(6)]);
    let operations = BTreeMap::from([(token(4), (c(BUYER), true)), (token(7), (c(BUYER), false))]);
    let clean = vec![("8".to_owned(), row(&[3], &[4]))];
    index::check(clean, &quotes, &funded_quotes, &operations)?;
    let quote_bad = "indexed entitlement quote is missing, foreign or funded";
    let operation_bad = "indexed entitlement operation is missing, foreign or closed";
    for (case, rows, reason) in [
        (
            "missing quote",
            vec![("8".to_owned(), row(&[3, 99], &[4]))],
            quote_bad,
        ),
        (
            "foreign quote",
            vec![("8".to_owned(), row(&[3, 5], &[4]))],
            quote_bad,
        ),
        (
            "funded quote",
            vec![("8".to_owned(), row(&[3, 6], &[4]))],
            quote_bad,
        ),
        (
            "missing operation",
            vec![("8".to_owned(), row(&[3], &[4, 98]))],
            operation_bad,
        ),
        (
            "closed operation",
            vec![("8".to_owned(), row(&[3], &[4, 7]))],
            operation_bad,
        ),
        (
            "open operation not indexed",
            vec![("8".to_owned(), row(&[3], &[]))],
            "open entitlement operation is not indexed",
        ),
        (
            "row under another key",
            vec![("9".to_owned(), row(&[3], &[4]))],
            "entitlement buyer index key differs",
        ),
    ] {
        check_eq!(
            index::check(rows, &quotes, &funded_quotes, &operations).err(),
            Some(reason.to_owned()),
            "{case}"
        );
    }
    // The real boot path over real rows.
    let (mut e, h) = rig()?;
    let mut disk = Disk::new();
    let q = quote(3, BUYER, 160)?;
    quote_step(&mut e, &h, &mut disk, &q, 100)?;
    fund_step(&mut e, &h, &mut disk, &*funded(q, 4)?)?;
    boot(&disk)?;
    Ok(())
}

/// 17g C2f (the C2b brief test 4): a quote abandoned past its expiry stays
/// indexed, and the clock-free audit boots it.
#[test]
fn an_abandoned_quote_survives_reboot_after_it_expires() -> TestResult {
    let (mut e, h) = rig()?;
    let mut disk = Disk::new();
    quote_step(&mut e, &h, &mut disk, &*quote(3, BUYER, 160)?, 100)?;
    e.world.set_now(10_000);
    let (mut later, later_handles) = rig()?;
    later.world.set_now(10_000);
    later.hydrate_action_state(disk.iter().map(|(key, value)| {
        mush_core::ActionStateBootRecord::new(crate::NS, key.clone(), value.clone())
    }))?;
    later.module_loaded(
        crate::NS,
        ModuleBootData::default(),
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )?;
    check_eq!(
        later
            .action_state()
            .get(&later_handles.actors, &index::BuyerKey::Character(BUYER))?
            .ok_or("row")?
            .quotes,
        tokens(&[3])
    );
    Ok(())
}

/// 17g C2f (the C2b brief test 5): the exact post-images after the quote,
/// fund, grant and refund steps each hydrate and boot, with a second open
/// purchase at the Grant and at the Refund (so the Grant's write is the Put
/// branch); wherever an index row exists, dropping it refuses the boot.
#[test]
fn entitlement_index_boots_after_every_step() -> TestResult {
    let (mut e, h) = rig()?;
    let mut disk = Disk::new();
    let key = BUYER.0.to_string();
    let dropped = |disk: &Disk| -> Disk {
        let mut out = disk.clone();
        out.retain(|k, _| {
            !(k.contains(EntitlementActorIndex::FAMILY) && k.ends_with(&format!("/{key}")))
        });
        out
    };
    let a = quote(3, BUYER, 160)?;
    let b = quote(5, BUYER, 160)?;
    quote_step(&mut e, &h, &mut disk, &a, 100)?;
    quote_step(&mut e, &h, &mut disk, &b, 100)?;
    boot(&disk)?;
    check!(
        boot(&dropped(&disk)).is_ok(),
        "indexed quotes alone are not required"
    );
    let pa = funded(a, 4)?;
    fund_step(&mut e, &h, &mut disk, &pa)?;
    let pb = funded(b, 9)?;
    fund_step(&mut e, &h, &mut disk, &pb)?;
    boot(&disk)?;
    check!(
        boot(&dropped(&disk)).is_err(),
        "open purchases must be indexed"
    );
    terminal_step(&mut e, &h, &mut disk, &*closed(&pa, 6, true)?)?;
    check_eq!(row(&e, &h, BUYER)?.ok_or("row")?.operations, tokens(&[9]));
    boot(&disk)?;
    check!(
        boot(&dropped(&disk)).is_err(),
        "the second open purchase still indexed"
    );
    terminal_step(&mut e, &h, &mut disk, &*closed(&pb, 10, false)?)?;
    check!(row(&e, &h, BUYER)?.is_none());
    boot(&disk)?;
    Ok(())
}
