#![cfg(test)]

//! Doc 46 CT.0: the House purchase twin's committed snapshot
//! (`tests/ct0/purchase_house.snap`).
//!
//! The House twin's rig differs from the personal one, so this is its own
//! function, not an instance of the personal walk: `Families::register`
//! requests the Roll's House sale reader, which a bare engine refuses, and the
//! House families have no test-visible registration of their own (the
//! personal twin has `token_family`), so no typed store runs here and no
//! replay write is pinned. The walk takes the existing House V1 fixtures
//! (`fixture`, `purchase`) through Listed, Quoted, Funded and then Granted or
//! Refunded, and writes every row in field order, its `Row` family, ceiling
//! and key, its canonical stored value, its digests, and the receipt. The
//! terminal phase and receipt are
//! built as `publication`'s terminal builds them (it needs an E4 terminal
//! output) and checked by `matches_purchase`. Then the protocol's action ids,
//! domains and descriptors for both House protocols, the configuration's
//! declarations, and a codec pin of each `data!` type.
use super::super::HouseConfig;
use super::super::families::Counts;
use super::super::protocol::{self as closed, Transition};
use super::*;
use mush_roll::commerce::house_purchase::HouseProtocol;
use mush_testkit::snapshot::Snapshot;

/// The House configuration over the fixture's policy, one-seat Pins only.
fn config(q: &HouseQuoteV1) -> TestResult<HouseConfig> {
    let policy = super::super::HousePolicy {
        price: q.price,
        instrument: q.instrument.clone(),
        registry: q.registry.0.clone(),
        venue_name: q.venue.name.clone(),
        venue: q.venue.cut.0.clone(),
        transferable: false,
        term_seconds: 900,
        grace_seconds: 90,
        quote_seconds: 60,
        renewal_price: Amount::from_minor(600),
        renewal_term_seconds: 800,
        renewal_grace_seconds: 80,
    };
    Ok(HouseConfig::new(
        policy,
        definition("tacsim", "registry_manifest", "combat", 1)?,
        PurchaseParticipants::new(
            definition("economy", "owner_participant", "initial_filing", 1)?,
            definition("roll", "owner_participant", "commercial_roots", 1)?,
        )?,
        mush_roll::commerce::house_purchase::writer::HousePurchaseWriter::new(),
    )?)
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

/// A row's canonical stored value (sorted keys, as the store keeps it).
fn stored<R: Row>(snap: &mut Snapshot, label: &str, value: &R) -> TestResult {
    let bytes =
        mush_core::canonical_json_bytes_with_limit(&serde_json::to_value(value)?, R::MAX_BYTES)?;
    snap.bytes(label, &bytes);
    Ok(())
}

/// The listing the fixture's quote was taken from.
fn listing(q: &HouseQuoteV1) -> TestResult<HouseListingV1> {
    let listing = HouseListingV1 {
        format: 1,
        publication: q.listing,
        publisher: ObjId(7),
        subject: q.subject.clone(),
        product: q.product.clone(),
        price: q.price,
        instrument: q.instrument.clone(),
        venue: q.venue.clone(),
        registry: q.registry.clone(),
        policy: q.policy.clone(),
        observed_at_utc: 100,
    };
    listing.validate()?;
    check_eq!(listing.digest()?, q.listing_digest);
    Ok(listing)
}

/// `p` closed by the terminal commit 5, and its receipt, as `publication`
/// builds them.
fn terminal(p: &HousePurchaseV1, grant: bool) -> TestResult<(HousePurchaseV1, HouseReceiptV1)> {
    let mut after = p.clone();
    after.updated_by = token(5);
    after.phase = if grant {
        PurchasePhase::Granted { commit: token(5) }
    } else {
        PurchasePhase::Refunded { commit: token(5) }
    };
    after.validate()?;
    let receipt = HouseReceiptV1 {
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

/// Listed, Quoted, Funded, then Granted and Refunded.
#[inline(never)]
fn walk(snap: &mut Snapshot) -> TestResult {
    let quote = Box::new(fixture()?);
    let listing = Box::new(listing(&quote)?);
    let funded = Box::new(super::purchase(*quote.clone())?);
    snap.section("house purchase: listed");
    row(snap, "listing", listing.as_ref())?;
    stored(snap, "stored listing", listing.as_ref())?;
    snap.line("listing digest", &listing.digest()?.to_hex());
    snap.line("product digest", &listing.product.digest()?.to_hex());
    snap.line(
        "grant economics digest",
        &listing.product.grant.digest()?.to_hex(),
    );

    snap.section("house purchase: quoted");
    row(snap, "quote", quote.as_ref())?;
    stored(snap, "stored quote", quote.as_ref())?;
    snap.line("quote digest", &quote.digest()?.to_hex());
    snap.line("quote raw digest", &quote.raw_digest()?.to_hex());
    snap.line("quote product digest", &quote.product_digest.to_hex());
    snap.line("quote listing digest", &quote.listing_digest.to_hex());
    snap.line(
        "economic subject digest",
        &quote.economic_subject()?.economic_digest()?.to_hex(),
    );

    snap.section("house purchase: funded");
    row(snap, "purchase", funded.as_ref())?;
    stored(snap, "stored purchase", funded.as_ref())?;
    snap.wire("phase", &funded.phase)?;
    snap.wire("effect", &funded.effect)?;
    check_eq!(
        HousePurchaseV1::decode(&serde_json::to_vec(funded.as_ref())?)?,
        *funded
    );

    for grant in [true, false] {
        let (after, receipt) = terminal(&funded, grant)?;
        let title = if grant { "granted" } else { "refunded" };
        snap.section(&format!("house purchase: {title}"));
        row(snap, "purchase", &after)?;
        stored(snap, "stored purchase", &after)?;
        snap.wire("phase", &after.phase)?;
        row(snap, "receipt", &receipt)?;
        stored(snap, "stored receipt", &receipt)?;
        check_eq!(
            HousePurchaseV1::decode(&serde_json::to_vec(&after)?)?,
            after
        );
        check_eq!(
            HouseReceiptV1::decode(&serde_json::to_vec(&receipt)?)?,
            receipt
        );
    }
    Ok(())
}

#[inline(never)]
fn identity(snap: &mut Snapshot, config: &HouseConfig) -> TestResult {
    snap.section("house purchase: identity");
    snap.line("counts family", <Counts as Row>::FAMILY);
    snap.line("counts max bytes", &<Counts as Row>::MAX_BYTES.to_string());
    let participants = config.participants().clone();
    for protocol in [HouseProtocol::HouseV1, HouseProtocol::HouseV2] {
        let p = protocol.protocol_name();
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
    }
    for (label, config) in [
        ("one seat", config.clone()),
        (
            "two seat",
            config.clone().with_two_seat_pins(config.policy.clone())?,
        ),
    ] {
        for protocol in config.protocols().collect::<Vec<_>>() {
            let p = protocol.protocol_name();
            for (n, action) in config.owner_actions(protocol)?.iter().enumerate() {
                snap.wire(&format!("{label} {p} owner action {n}"), action)?;
            }
            snap.wire(
                &format!("{label} {p} quote action"),
                &config.quote_action(protocol)?,
            )?;
            snap.wire(
                &format!("{label} {p} policy ref"),
                &config.policy_ref_for_protocol(protocol)?,
            )?;
        }
        for d in config.declarations()? {
            snap.wire(&format!("{label} declared payload"), d.payload())?;
            snap.wire(
                &format!("{label} declared ref"),
                &DefinitionRegistryBuilder::new().register(OwnerId::new("market")?, d)?,
            )?;
        }
    }
    Ok(())
}

#[inline(never)]
fn codecs(snap: &mut Snapshot) -> TestResult {
    let quote = Box::new(fixture()?);
    let format = |json: &mut Value| json["format"] = 9.into();
    snap.codec("HouseSubjectV1", &quote.subject, |json| {
        json["title_revision"] = 0.into()
    })?;
    snap.codec("HouseProductV1", &quote.product, |json| {
        json["term_seconds"] = 0.into()
    })?;
    snap.codec("HouseListingV1", &listing(&quote)?, format)?;
    snap.codec("HouseQuoteV1", quote.as_ref(), format)?;
    let funded = super::purchase(*quote)?;
    snap.codec("HousePurchaseV1 (funded)", &funded, format)?;
    let (granted, granted_receipt) = terminal(&funded, true)?;
    snap.codec("HousePurchaseV1 (granted)", &granted, format)?;
    snap.codec("HouseReceiptV1 (granted, issued)", &granted_receipt, format)?;
    let (_, refunded_receipt) = terminal(&funded, false)?;
    snap.codec(
        "HouseReceiptV1 (refunded, not issued)",
        &refunded_receipt,
        format,
    )?;
    Ok(())
}

#[test]
fn ct0_house_purchase_snapshot() -> TestResult {
    let mut snap = Snapshot::new();
    let config = config(&fixture()?)?;
    identity(&mut snap, &config)?;
    walk(&mut snap)?;
    codecs(&mut snap)?;
    snap.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/purchase_house.snap")
        .map_err(Into::into)
}
