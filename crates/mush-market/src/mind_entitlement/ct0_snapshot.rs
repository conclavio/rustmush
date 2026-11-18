#![cfg(test)]

//! Doc 46 CT.0: the personal purchase twin's committed snapshot
//! (`tests/ct0/purchase_personal.snap`).
//!
//! For each protocol (original V3 sealed, derived V1 open resale) the walk
//! takes the existing fixtures (`fixture`, `listing_fixture`, `purchase`)
//! through Listed, Quoted, Funded and then Granted or Refunded, each step one
//! real typed transaction over the families exactly as `Families::register`
//! registers them (`token_family`, `actors_family`) with the per-buyer index
//! writers (`index::quoted`, `funded`, `released`), and writes each step's
//! replay writes, the stored documents, every row in field order, its key and
//! digests, and the receipt. The terminal phase and receipt are built as
//! `publication::terminal` builds them (it needs an E4 terminal output, which
//! no fast test can mint) and checked by `matches_purchase`. Then the
//! protocol's action ids and descriptors, the configuration's declarations,
//! and a codec pin of each `data!` type (`purchase.rs`, `records.rs`).
use super::super::families::{Counts, actors_family, token_family};
use super::super::index::{self, BuyerKey, EntitlementActorIndex};
use super::super::protocol::{self as closed, Transition};
use super::*;
use mush_core::authoritative::ReplayWrite;
use mush_core::{
    ActionTxn, Engine, GameModule, ModError, Registrar, Registry, StateFamilyDescriptor,
    StateFamilyHandle, World,
};
use mush_testkit::snapshot::Snapshot;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Ct0Families {
    listings: StateFamilyHandle<CommitToken, EntitlementListingV1>,
    quotes: StateFamilyHandle<CommitToken, EntitlementQuoteV1>,
    operations: StateFamilyHandle<CommitToken, EntitlementPurchaseV1>,
    receipts: StateFamilyHandle<CommitToken, EntitlementReceiptV1>,
    actors: StateFamilyHandle<BuyerKey, EntitlementActorIndex>,
}
impl Ct0Families {
    fn index(&self) -> index::Index<'_> {
        index::Index {
            quotes: &self.quotes,
            operations: &self.operations,
            actors: &self.actors,
        }
    }
}
struct Ct0Module(Arc<Mutex<Option<Ct0Families>>>);
impl GameModule for Ct0Module {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let families = Ct0Families {
            listings: token_family(reg)?,
            quotes: token_family(reg)?,
            operations: token_family(reg)?,
            receipts: token_family(reg)?,
            actors: actors_family(reg)?,
        };
        *self
            .0
            .lock()
            .map_err(|_| ModError::msg("ct0 family lock"))? = Some(families);
        Ok(())
    }
}
#[inline(never)]
fn rig() -> TestResult<(Box<Engine>, Ct0Families)> {
    let slot = Arc::new(Mutex::new(None));
    let engine = Box::new(Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Ct0Module(slot.clone()))],
    )?);
    let families = slot
        .lock()
        .map_err(|_| "ct0 family lock")?
        .take()
        .ok_or("ct0 families")?;
    Ok((engine, families))
}

/// One step as one typed transaction; its replay writes.
fn step(
    engine: &mut Engine,
    families: &Ct0Families,
    run: impl FnOnce(&mut ActionTxn<'_>, &Ct0Families) -> Result<(), String>,
) -> TestResult<Vec<ReplayWrite>> {
    let mut txn = engine.action_transaction(&families.operations)?;
    run(&mut txn, families)?;
    let prepared = txn.prepare()?;
    let writes = prepared.authoritative_replay_writes();
    let (_, install) = prepared.into_parts();
    engine.install_prepared_action(install)?;
    Ok(writes)
}

fn descriptor(snap: &mut Snapshot, name: &str, owner: &str, d: &StateFamilyDescriptor) {
    snap.line(&format!("{name} family owner"), owner);
    snap.line(&format!("{name} family id"), d.id().as_str());
    snap.line(&format!("{name} key codec"), d.key_codec().as_str());
    snap.line(&format!("{name} value schema"), d.value_schema().as_str());
    snap.line(
        &format!("{name} schema version"),
        &d.schema_version().to_string(),
    );
    snap.line(&format!("{name} decoder"), d.decoder().as_str());
    snap.line(&format!("{name} validator"), d.validator().as_str());
    snap.line(&format!("{name} serializer"), d.serializer().as_str());
    snap.line(
        &format!("{name} max key bytes"),
        &d.max_key_bytes().to_string(),
    );
    snap.line(
        &format!("{name} max record bytes"),
        &d.max_record_bytes().to_string(),
    );
}

/// A row's family, ceiling, key and field-order wire, through `Row`.
fn row<R: Row>(snap: &mut Snapshot, label: &str, value: &R) -> TestResult {
    snap.line(&format!("{label} family"), R::FAMILY);
    snap.line(&format!("{label} max bytes"), &R::MAX_BYTES.to_string());
    snap.line(&format!("{label} key"), &value.key());
    value.validate()?;
    snap.wire(label, value)?;
    Ok(())
}

/// The stored document of `key` in `family`, if any.
fn stored<V: Clone + Send + Sync + 'static>(
    snap: &mut Snapshot,
    engine: &Engine,
    label: &str,
    family: &StateFamilyHandle<CommitToken, V>,
    key: CommitToken,
) -> TestResult {
    match engine.action_state().canonical(family, &key)? {
        Some(bytes) => snap.bytes(label, bytes.as_slice()),
        None => snap.line(label, "absent"),
    }
    Ok(())
}

fn buyer_row(snap: &mut Snapshot, engine: &Engine, f: &Ct0Families, buyer: BuyerKey) -> TestResult {
    snap.line("buyer index key", &buyer.key());
    match engine.action_state().get(&f.actors, &buyer)? {
        Some(row) => snap.wire("buyer index", row)?,
        None => snap.line("buyer index", "absent"),
    }
    Ok(())
}

/// `p` closed by the terminal commit 5, and its receipt, as
/// `publication::terminal` builds them.
fn terminal(
    p: &EntitlementPurchaseV1,
    grant: bool,
) -> TestResult<(EntitlementPurchaseV1, EntitlementReceiptV1)> {
    let mut after = p.clone();
    after.updated_by = token(5);
    after.phase = if grant {
        PurchasePhase::Granted { commit: token(5) }
    } else {
        PurchasePhase::Refunded { commit: token(5) }
    };
    after.validate()?;
    let receipt = EntitlementReceiptV1 {
        format: 1,
        operation: p.operation,
        commit: token(5),
        outcome: if grant {
            Outcome::Granted {}
        } else {
            Outcome::Refunded {}
        },
        contract: p.funding.contract(),
        hold: p.funding.hold(),
        transaction: mush_economy::TransactionId(6),
        journal_seq: 7,
        licence: p.licence,
        term: p.term,
        quote_digest: p.quote_digest,
        product_digest: p.quote.product_digest,
        grant_economics_digest: p.quote.product.grant.digest()?,
        issued_at_ic: grant.then_some(ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 170,
        }),
    };
    receipt.matches_purchase(&after)?;
    Ok((after, receipt))
}

/// One protocol's walk: Listed, Quoted, Funded, then Granted (this store) and
/// Refunded (a second store from the same start).
#[inline(never)]
fn walk(
    snap: &mut Snapshot,
    name: &str,
    protocol: EntitlementProtocol,
    depth: u64,
    form: PaidForm,
) -> TestResult {
    let quote = Box::new(fixture(protocol, depth, form)?);
    let listing = Box::new(listing_fixture(protocol, depth, form, quote.listing)?);
    let funded = Box::new(super::purchase(*quote.clone())?);
    let buyer = BuyerKey::of(quote.buyer);
    let mut first = None;
    for grant in [true, false] {
        let (mut engine, f) = rig()?;
        // The steps before the terminal are the same in both stores: the
        // first writes them, the second must write the same replay writes.
        let mut scratch = Snapshot::new();
        let early: &mut Snapshot = if first.is_none() {
            &mut *snap
        } else {
            &mut scratch
        };
        let listed = step(&mut engine, &f, |t, f| {
            t.insert(&f.listings, &listing.publication, *listing.clone())
                .map_err(|e| e.to_string())
        })?;
        early.section(&format!("personal purchase {name}: listed"));
        row(early, "listing", listing.as_ref())?;
        early.line("listing digest", &listing.digest()?.to_hex());
        early.line("product digest", &listing.product.digest()?.to_hex());
        early.line(
            "grant economics digest",
            &listing.product.grant.digest()?.to_hex(),
        );
        early.line("royalty chain digest", &listing.royalty.digest()?.to_hex());
        early.replay_writes(&listed)?;
        stored(
            early,
            &engine,
            "stored listing",
            &f.listings,
            listing.publication,
        )?;

        let quoted = step(&mut engine, &f, |t, f| {
            t.insert(&f.quotes, &quote.publication, *quote.clone())
                .map_err(|e| e.to_string())?;
            index::quoted(t, &f.index(), buyer, quote.publication, 100)
        })?;
        early.section(&format!("personal purchase {name}: quoted"));
        row(early, "quote", quote.as_ref())?;
        early.line("quote digest", &quote.digest()?.to_hex());
        early.line("quote raw digest", &quote.raw_digest()?.to_hex());
        early.line("quote product digest", &quote.product_digest.to_hex());
        early.line("quote listing digest", &quote.listing_digest.to_hex());
        early.line(
            "economic subject digest",
            &quote.economic_subject()?.economic_digest()?.to_hex(),
        );
        early.replay_writes(&quoted)?;
        stored(early, &engine, "stored quote", &f.quotes, quote.publication)?;
        buyer_row(early, &engine, &f, buyer)?;

        let fund = step(&mut engine, &f, |t, f| {
            t.remove(&f.quotes, &quote.publication)
                .map_err(|e| e.to_string())?;
            t.insert(&f.operations, &funded.operation, *funded.clone())
                .map_err(|e| e.to_string())?;
            index::funded(t, &f.index(), buyer, quote.publication, funded.operation)
        })?;
        early.section(&format!("personal purchase {name}: funded"));
        row(early, "purchase", funded.as_ref())?;
        early.wire("phase", &funded.phase)?;
        early.wire("effect", &funded.effect)?;
        early.replay_writes(&fund)?;
        stored(
            early,
            &engine,
            "stored purchase",
            &f.operations,
            funded.operation,
        )?;
        stored(early, &engine, "stored quote", &f.quotes, quote.publication)?;
        buyer_row(early, &engine, &f, buyer)?;
        match &first {
            Some(writes) => check_eq!(writes, &(listed, quoted, fund)),
            None => first = Some((listed, quoted, fund)),
        }
        let (after, receipt) = terminal(&funded, grant)?;
        let end = step(&mut engine, &f, |t, f| {
            t.replace(&f.operations, &after.operation, after.clone())
                .map_err(|e| e.to_string())?;
            t.insert(&f.receipts, &receipt.operation, receipt.clone())
                .map_err(|e| e.to_string())?;
            index::released(t, &f.index(), buyer, after.operation)
        })?;
        let title = if grant { "granted" } else { "refunded" };
        snap.section(&format!("personal purchase {name}: {title}"));
        row(snap, "purchase", &after)?;
        snap.wire("phase", &after.phase)?;
        row(snap, "receipt", &receipt)?;
        snap.replay_writes(&end)?;
        stored(
            snap,
            &engine,
            "stored purchase",
            &f.operations,
            after.operation,
        )?;
        stored(
            snap,
            &engine,
            "stored receipt",
            &f.receipts,
            receipt.operation,
        )?;
        buyer_row(snap, &engine, &f, buyer)?;
        check_eq!(
            EntitlementPurchaseV1::decode(&serde_json::to_vec(&after)?)?,
            after
        );
        check_eq!(
            EntitlementReceiptV1::decode(&serde_json::to_vec(&receipt)?)?,
            receipt
        );
    }
    Ok(())
}

#[inline(never)]
fn identity(snap: &mut Snapshot) -> TestResult {
    snap.section("personal purchase: identity");
    let (_engine, f) = rig()?;
    descriptor(
        snap,
        "listing",
        f.listings.owner().as_str(),
        f.listings.descriptor(),
    );
    descriptor(
        snap,
        "quote",
        f.quotes.owner().as_str(),
        f.quotes.descriptor(),
    );
    descriptor(
        snap,
        "purchase",
        f.operations.owner().as_str(),
        f.operations.descriptor(),
    );
    descriptor(
        snap,
        "receipt",
        f.receipts.owner().as_str(),
        f.receipts.descriptor(),
    );
    descriptor(
        snap,
        "buyer index",
        f.actors.owner().as_str(),
        f.actors.descriptor(),
    );
    snap.line("counts family", <Counts as Row>::FAMILY);
    snap.line("counts max bytes", &<Counts as Row>::MAX_BYTES.to_string());
    snap.line("max listing bytes", &MAX_LISTING_BYTES.to_string());
    snap.line("max quote bytes", &MAX_QUOTE_BYTES.to_string());
    snap.line(
        "buyer index key, organisation",
        &BuyerKey::Organization(mush_org::OrgId(3)).key(),
    );
    let config = command_config(400)?;
    let participants = config.participants().clone();
    for protocol in [
        EntitlementProtocol::OriginalV3,
        EntitlementProtocol::DerivedV1,
    ] {
        let p = protocol.name();
        for kind in ["product", "listing", "quote"] {
            snap.line(
                &format!("{p} {kind} domain"),
                closed::domain(protocol, kind)?,
            );
        }
        for t in [Transition::Fund, Transition::Grant, Transition::Refund] {
            snap.line(
                &format!("{p} {} action", t.name()),
                closed::action(protocol, t),
            );
            let d = closed::descriptor(protocol, t, config.manifest(), &participants)?;
            snap.wire(&format!("{p} {} payload", t.name()), d.payload())?;
        }
        snap.wire(
            &format!("{p} historical fund"),
            &closed::historical_fund(protocol, hash(9), &participants)?,
        )?;
        for (n, action) in config.owner_actions(protocol)?.iter().enumerate() {
            snap.wire(&format!("{p} owner action {n}"), action)?;
        }
        snap.wire(
            &format!("{p} quote action"),
            &config.quote_action(protocol)?,
        )?;
    }
    snap.wire("policy ref", &config.policy_ref()?)?;
    for d in config.declarations()? {
        snap.wire("declared payload", d.payload())?;
        snap.wire(
            "declared ref",
            &DefinitionRegistryBuilder::new().register(OwnerId::new("market")?, d)?,
        )?;
    }
    Ok(())
}

#[inline(never)]
fn codecs(snap: &mut Snapshot) -> TestResult {
    let original = Box::new(fixture(
        EntitlementProtocol::OriginalV3,
        0,
        PaidForm::Sealed {},
    )?);
    let derived = Box::new(fixture(
        EntitlementProtocol::DerivedV1,
        1,
        PaidForm::OpenResale {},
    )?);
    let format = |json: &mut Value| json["format"] = 9.into();
    snap.codec(
        "EntitlementSubjectV1 (no parent)",
        &original.subject,
        |json| json["seller"] = 0.into(),
    )?;
    snap.codec("EntitlementSubjectV1 (parent)", &derived.subject, |json| {
        json["seller"] = 0.into()
    })?;
    snap.codec(
        "EntitlementProductV1 (original)",
        &original.product,
        |json| json["term_seconds"] = 0.into(),
    )?;
    snap.codec("EntitlementProductV1 (derived)", &derived.product, |json| {
        json["term_seconds"] = 0.into()
    })?;
    let listing = listing_fixture(
        EntitlementProtocol::OriginalV3,
        0,
        PaidForm::Sealed {},
        original.listing,
    )?;
    snap.codec("EntitlementListingV1", &listing, format)?;
    snap.codec("EntitlementQuoteV1 (original)", original.as_ref(), format)?;
    snap.codec("EntitlementQuoteV1 (derived)", derived.as_ref(), format)?;
    let funded = super::purchase(*original)?;
    snap.codec("EntitlementPurchaseV1 (funded)", &funded, format)?;
    let (granted, granted_receipt) = terminal(&funded, true)?;
    snap.codec("EntitlementPurchaseV1 (granted)", &granted, format)?;
    snap.codec(
        "EntitlementReceiptV1 (granted, issued)",
        &granted_receipt,
        format,
    )?;
    let (_, refunded_receipt) = terminal(&funded, false)?;
    snap.codec(
        "EntitlementReceiptV1 (refunded, not issued)",
        &refunded_receipt,
        format,
    )?;
    Ok(())
}

#[test]
fn ct0_personal_purchase_snapshot() -> TestResult {
    let mut snap = Snapshot::new();
    identity(&mut snap)?;
    walk(
        &mut snap,
        "original",
        EntitlementProtocol::OriginalV3,
        0,
        PaidForm::Sealed {},
    )?;
    walk(
        &mut snap,
        "derived",
        EntitlementProtocol::DerivedV1,
        1,
        PaidForm::OpenResale {},
    )?;
    codecs(&mut snap)?;
    snap.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/purchase_personal.snap",
    )
    .map_err(Into::into)
}
