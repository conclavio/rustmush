//! Structural consent, canonical identity and E4 calculation tests only.
//! No fixture below claims a paid journal entry or a current source grant.
use super::{
    codec::{Bounded, Row},
    protocol,
    purchase::*,
    records::*,
};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::managed_entitlement::{
    EntitlementProtocol, GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use mush_economy::managed_purchase::FundingReceipt;
use mush_economy::{AccountId, Amount, InstrumentId};
use mush_roll::commerce::entitlement::{
    DownstreamTermsV1, EntitlementAllocationV1, FiledRoyaltyTermsV1, GrantEconomicsV1,
    RenewalTemplateV1,
};
use mush_roll::commerce::paid::{PaidForm, PaidTerms};
use mush_roll::commerce::purchase::{PurchaseParticipants, PurchasePhase};
use mush_roll::commerce::rating::{RatingClass, RatingRef};
use mush_roll::commerce::records::RevisionPolicy;
use mush_roll::commerce::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
    WorldIcSeconds,
};
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn hash(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn identity(n: u64) -> TestResult<RevisionIdentity> {
    Ok(serde_json::from_value(
        json!({"format":1,"revision":{"key":{"character":7,"mind":n,"number":1},"digest":hash(1)},"source":hash(2)}),
    )?)
}
fn definition(owner: &str, kind: &str, id: &str, version: u32) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new(owner)?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            version,
            json!({"format":version}),
        )?,
    )?)
}
fn at(n: i64) -> TestResult<WorldIcSeconds> {
    Ok(WorldIcSeconds::from_stamp(&ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: n,
    })?)
}
pub(super) fn fixture(
    protocol: EntitlementProtocol,
    depth: u64,
    form: PaidForm,
) -> TestResult<EntitlementQuoteV1> {
    let revision = identity(10)?;
    let mut edges = Vec::new();
    let mut parents = Vec::new();
    for n in 1..=depth {
        edges.push(json!({"parent":{"filed":n.to_string(),"revision":identity(n)?},"charter":(100+n).to_string(),"line":n.to_string(),
            "acquisition":{"licence":n.to_string(),"term":"1","operation":token(u8::try_from(n)?),"product_digest":hash(3)},
            "royalty_permille":100,"basis":{"kind":"explicit_term_economics_v1","digest":hash(4)}}));
        parents.push(ParentCut(RoyaltyRecipientCut {
            charter: ObjId(u32::try_from(100 + n)?),
            permille: 100,
            title: Some(RoyaltyTitleRecipient {
                title_revision: 1,
                recipient: Recipient {
                    account: AccountId(20 + n),
                    owner: mush_org::PrincipalId::Object(ObjId(u32::try_from(20 + n)?)),
                },
            }),
        }));
    }
    let royalty: FiledRoyaltyTermsV1 = serde_json::from_value(
        json!({"format":1,"filed":"10","revision":revision,"publication":token(9),"edges":edges}),
    )?;
    let subject = EntitlementSubjectV1 {
        seller: ObjId(7),
        title_revision: 1,
        charter: CharterId::new(20)?,
        line: LineId::new(10)?,
        filed: FiledRevisionId::new(10)?,
        sequence: 1,
        owner_licence: LicenceId::new(10)?,
        owner_term: TermGeneration::new(1)?,
        owner_window: TermWindow::new(at(0)?, at(0)?, at(100)?, at(110)?)?,
        revision,
        schema: hash(5),
        definition: hash(6),
        rating: serde_json::from_value::<RatingRef>(
            json!({"job":token(1),"id":hash(7),"schema":1,"receipt_digest":hash(8)}),
        )?,
        compatibility: hash(9),
        declared: RatingClass::II,
        minimum: RatingClass::II,
        parent: royalty.edges().first().map(|e| e.parent()),
    };
    let terms: PaidTerms = serde_json::from_value(
        json!({"seats":1,"class":RatingClass::II,"form":form,"transferable":false}),
    )?;
    let revision_policy = RevisionPolicy::Pin {
        filed: subject.filed,
        revision,
    };
    let policy = definition("market", "entitlement_policy", "personal", 1)?;
    let instrument = InstrumentId::new("cr").ok_or("instrument")?;
    let renewal = RenewalTemplateV1::new(
        instrument.clone(),
        Amount::from_minor(600),
        800,
        80,
        terms,
        revision_policy,
        policy.clone(),
    )?;
    let grant = GrantEconomicsV1::new(
        match form {
            PaidForm::Sealed {} => DownstreamTermsV1::Sealed {},
            _ => DownstreamTermsV1::Source {
                royalty_permille: 0,
            },
        },
        renewal,
        royalty.reference()?,
    )?;
    let product = EntitlementProductV1 {
        protocol,
        terms,
        revision: revision_policy,
        term_seconds: 900,
        grace_seconds: 90,
        grant,
    };
    let registry = Cut(GrossRecipientCut {
        permille: 50,
        recipient: Some(Recipient {
            account: AccountId(11),
            owner: mush_org::PrincipalId::System(
                mush_org::SystemPrincipalId::new("roll.registry").ok_or("system")?,
            ),
        }),
    });
    let venue = Venue {
        name: crate::MarketId::new("registry").ok_or("venue")?,
        cut: Cut(GrossRecipientCut {
            permille: 20,
            recipient: Some(Recipient {
                account: AccountId(12),
                owner: mush_org::PrincipalId::Object(ObjId(12)),
            }),
        }),
    };
    let listing = EntitlementListingV1 {
        format: 1,
        publication: token(2),
        subject,
        product,
        royalty,
        price: Amount::from_minor(400),
        instrument,
        venue,
        registry,
        policy,
        observed_at_utc: 100,
    };
    listing.validate()?;
    let mut q = EntitlementQuoteV1 {
        format: 1,
        publication: token(3),
        listing: listing.publication,
        listing_digest: listing.digest()?,
        buyer: ObjId(8).into(),
        subject: listing.subject,
        product_digest: listing.product.digest()?,
        product: listing.product,
        royalty: listing.royalty,
        price: listing.price,
        instrument: listing.instrument,
        registry: listing.registry,
        venue: listing.venue,
        parents: Bounded(parents),
        remainder: Recipient {
            account: AccountId(9),
            owner: mush_org::PrincipalId::Object(ObjId(7)),
        },
        payer: AccountId(8),
        payouts: Bounded(Vec::new()),
        policy: listing.policy,
        quoted_at_utc: 100,
        expires_at_utc: 160,
    };
    reprice(&mut q)?;
    Ok(q)
}
/// 17g C2c: a listing row over `fixture`'s frozen product and subject,
/// published under `publication` (the command read's tests, which page and
/// look listings up by their token text).
pub(crate) fn listing_fixture(
    protocol: EntitlementProtocol,
    depth: u64,
    form: PaidForm,
    publication: CommitToken,
) -> TestResult<EntitlementListingV1> {
    let q = fixture(protocol, depth, form)?;
    let listing = EntitlementListingV1 {
        format: 1,
        publication,
        subject: q.subject,
        product: q.product,
        royalty: q.royalty,
        price: q.price,
        instrument: q.instrument,
        venue: q.venue,
        registry: q.registry,
        policy: q.policy,
        observed_at_utc: 100,
    };
    listing.validate()?;
    Ok(listing)
}
/// 17g C2c (the C2c fix round): a deployment configuration whose policy is
/// `listing_fixture`'s own at `price_minor` (the instrument, the Registry and
/// venue cuts), and a listing over `fixture`'s product that is current under
/// it (`current_policy`): its price and policy reference are the
/// configuration's. The command read's tests answer through these, so the
/// policy filter they exercise is the real one.
pub(crate) fn command_config(price_minor: i64) -> TestResult<super::EntitlementConfig> {
    let policy = super::EntitlementPolicy {
        price: Amount::from_minor(price_minor),
        instrument: InstrumentId::new("cr").ok_or("instrument")?,
        registry: GrossRecipientCut {
            permille: 50,
            recipient: Some(Recipient {
                account: AccountId(11),
                owner: mush_org::PrincipalId::System(
                    mush_org::SystemPrincipalId::new("roll.registry").ok_or("system")?,
                ),
            }),
        },
        venue_name: crate::MarketId::new("registry").ok_or("venue")?,
        venue: GrossRecipientCut {
            permille: 20,
            recipient: Some(Recipient {
                account: AccountId(12),
                owner: mush_org::PrincipalId::Object(ObjId(12)),
            }),
        },
        transferable: true,
        term_seconds: 900,
        grace_seconds: 90,
        quote_seconds: 60,
        renewal_price: Amount::from_minor(600),
        renewal_term_seconds: 800,
        renewal_grace_seconds: 80,
    };
    Ok(super::EntitlementConfig::new(
        policy,
        definition("tacsim", "registry_manifest", "combat", 1)?,
        PurchaseParticipants::new(
            definition("economy", "owner_participant", "initial_filing", 1)?,
            definition("roll", "owner_participant", "commercial_roots", 1)?,
        )?,
        mush_roll::commerce::entitlement::writer::EntitlementPurchaseWriter::new(),
    )?)
}
/// 17g C2c: an original sealed listing over `fixture`'s subject, listed
/// under `config` (its product built by `product_for`, its price, cuts and
/// policy the configuration's), so `current_policy(config, ..)` admits it.
pub(crate) fn current_listing_fixture(
    config: &super::EntitlementConfig,
    publication: CommitToken,
) -> TestResult<EntitlementListingV1> {
    let q = fixture(EntitlementProtocol::OriginalV3, 0, PaidForm::Sealed {})?;
    let product = config.product_for(
        EntitlementProtocol::OriginalV3,
        &q.subject,
        &q.royalty,
        super::config::ProductChoice {
            form: PaidForm::Sealed {},
            royalty_permille: 0,
            transferable: true,
        },
    )?;
    let listing = EntitlementListingV1 {
        format: 1,
        publication,
        subject: q.subject,
        product,
        royalty: q.royalty,
        price: config.policy.price,
        instrument: config.policy.instrument.clone(),
        venue: config.policy.venue(),
        registry: Cut(config.policy.registry.clone()),
        policy: config.policy_ref()?,
        observed_at_utc: 100,
    };
    listing.validate()?;
    super::current_policy(config, &listing)?;
    Ok(listing)
}
/// 17g C2g: a quote by `buyer` on the listing `listing`, published as
/// `publication`, quoted at `quoted` and fundable until `expires` (world UTC
/// seconds), over `fixture`'s original sealed product; the command read's
/// own-purchase tests index it.
pub(crate) fn command_quote(
    publication: CommitToken,
    listing: CommitToken,
    buyer: ObjId,
    quoted: i64,
    expires: i64,
) -> TestResult<EntitlementQuoteV1> {
    let mut q = fixture(EntitlementProtocol::OriginalV3, 0, PaidForm::Sealed {})?;
    q.publication = publication;
    q.listing = listing;
    q.buyer = buyer.into();
    q.quoted_at_utc = quoted;
    q.expires_at_utc = expires;
    q.validate()?;
    Ok(q)
}
/// 17g C2g: `q` funded as the open purchase `operation`.
pub(crate) fn command_purchase(
    q: EntitlementQuoteV1,
    operation: CommitToken,
) -> TestResult<EntitlementPurchaseV1> {
    let mut p = purchase(q)?;
    p.operation = operation;
    p.updated_by = operation;
    p.validate()?;
    Ok(p)
}
/// 17g C2g: `actor`'s per-buyer index row over `quotes` and `operations`.
pub(crate) fn command_index(
    actor: ObjId,
    quotes: &[CommitToken],
    operations: &[CommitToken],
) -> TestResult<super::EntitlementActorIndex> {
    let row = super::EntitlementActorIndex {
        format: 1,
        buyer: super::BuyerKey::Character(actor),
        quotes: quotes.iter().copied().collect(),
        operations: operations.iter().copied().collect(),
    };
    row.validate()?;
    Ok(row)
}
fn reprice(q: &mut EntitlementQuoteV1) -> TestResult {
    q.product_digest = q.product.digest()?;
    q.payouts = Bounded(
        q.economic_data(q.product_digest)?
            .payouts()?
            .into_iter()
            .map(|p| Payout {
                account: p.account,
                owner: p.owner,
                amount: p.amount,
            })
            .collect(),
    );
    q.validate()?;
    Ok(())
}
pub(super) fn purchase(q: EntitlementQuoteV1) -> TestResult<EntitlementPurchaseV1> {
    let participants = PurchaseParticipants::new(
        definition("economy", "owner_participant", "initial_filing", 1)?,
        definition("roll", "owner_participant", "commercial_roots", 1)?,
    )?;
    let funding: FundingReceipt = serde_json::from_value(
        json!({"contract":1,"hold":2,"transaction":3,"journal_seq":4,"consent_hash":([0_u8;32]),"at_ic":{"clock":{"kind":"world_ic","generation":1},"at":100}}),
    )?;
    let p = EntitlementPurchaseV1 {
        format: 1,
        operation: token(4),
        effect: protocol::historical_fund(
            q.product.protocol,
            q.subject.compatibility,
            &participants,
        )?,
        participants,
        quote_digest: q.digest()?,
        quote: q,
        licence: LicenceId::new(30)?,
        term: TermGeneration::new(1)?,
        funding,
        phase: PurchasePhase::Funded {},
        updated_by: token(4),
    };
    p.validate()?;
    Ok(p)
}
fn allocation(p: &EntitlementPurchaseV1) -> TestResult<EntitlementAllocationV1> {
    let q = &p.quote;
    Ok(serde_json::from_value(
        json!({"format":1,"protocol":q.product.protocol,"operation":p.operation,"participants":p.participants,
        "licence":p.licence,"term":p.term,"quote":q.publication,"quote_digest":p.quote_digest,"product_digest":q.product_digest,
        "holder":q.buyer,"charter":q.subject.charter,"line":q.subject.line,"filed":q.subject.filed,"revision":q.subject.revision,"rating":q.subject.rating,"terms":q.product.terms,
        "economics":{"format":1,"licence":p.licence,"generation":p.term,"operation":p.operation,"product_digest":q.product_digest,"grant":q.product.grant},
        "funding":p.funding,"phase":p.phase,"updated_by":p.updated_by}),
    )?)
}
fn obj(v: &mut Value) -> TestResult<&mut serde_json::Map<String, Value>> {
    v.as_object_mut().ok_or_else(|| "object".into())
}

#[test]
fn new_protocols_roundtrip_and_derive_exact_e4_conservation() -> TestResult {
    for (protocol, depth) in [
        (EntitlementProtocol::OriginalV3, 0),
        (EntitlementProtocol::DerivedV1, 3),
    ] {
        let q = fixture(protocol, depth, PaidForm::Sealed {})?;
        check_eq!(EntitlementQuoteV1::decode(&serde_json::to_vec(&q)?)?, q);
        let e = q.economic_subject()?;
        let payouts = e.payouts()?;
        check_eq!(
            payouts
                .iter()
                .try_fold(Amount::ZERO, |sum, p| sum.checked_add(p.amount))
                .ok_or("sum")?,
            q.price
        );
        check_eq!(payouts.len(), usize::try_from(depth)? + 3);
        check_eq!(e.quote_digest, q.digest()?);
        check_eq!(e.grant_economics_digest, q.product.grant.digest()?);
        check_eq!(e.royalty_chain_digest, q.royalty.digest()?);
    }
    Ok(())
}
#[test]
fn required_nulls_unknowns_duplicates_and_bounded_arrays_refuse() -> TestResult {
    let q = fixture(EntitlementProtocol::OriginalV3, 0, PaidForm::Sealed {})?;
    let raw = serde_json::to_string(&q)?;
    check!(EntitlementQuoteV1::decode(raw.replacen("{", "{\"format\":1,", 1).as_bytes()).is_err());
    let mut v = serde_json::to_value(&q)?;
    obj(&mut v)?.insert("allow".into(), json!(true));
    check!(serde_json::from_value::<EntitlementQuoteV1>(v).is_err());
    let mut v = serde_json::to_value(&q)?;
    obj(v.get_mut("subject").ok_or("subject")?)?.remove("parent");
    check!(serde_json::from_value::<EntitlementQuoteV1>(v).is_err());
    for raw in [
        r#"{"permille":0}"#,
        r#"{"permille":0,"recipient":null,"extra":1}"#,
    ] {
        check!(serde_json::from_str::<Cut>(raw).is_err());
    }
    check!(serde_json::from_str::<ParentCut>(r#"{"charter":1,"permille":0}"#).is_err());
    let mut v = serde_json::to_value(fixture(
        EntitlementProtocol::DerivedV1,
        3,
        PaidForm::Sealed {},
    )?)?;
    let parents = v
        .get_mut("parents")
        .and_then(Value::as_array_mut)
        .ok_or("parents")?;
    parents.push(parents.first().ok_or("parent")?.clone());
    check!(serde_json::from_value::<EntitlementQuoteV1>(v).is_err());
    check!(EntitlementQuoteV1::decode(&vec![b' '; MAX_QUOTE_BYTES + 1]).is_err());
    Ok(())
}
#[test]
fn product_protocol_parent_and_renewal_cannot_be_crossed_after_rehash() -> TestResult {
    let q = fixture(EntitlementProtocol::DerivedV1, 1, PaidForm::OpenResale {})?;
    let mut crossed = q.clone();
    crossed.product.protocol = EntitlementProtocol::OriginalV3;
    crossed.product_digest = crossed.product.digest()?;
    check!(crossed.validate().is_err());
    let mut crossed = q.clone();
    crossed.policy = definition("market", "entitlement_policy", "personal", 2)?;
    check!(crossed.validate().is_err());
    let mut crossed = q.clone();
    crossed.parents.0.first_mut().ok_or("parent")?.0.charter = ObjId(999);
    check!(crossed.validate().is_err());
    let mut crossed = q.clone();
    crossed.instrument = InstrumentId::new("other").ok_or("instrument")?;
    check!(crossed.validate().is_err());
    check!(fixture(EntitlementProtocol::DerivedV1, 3, PaidForm::OpenResale {}).is_err());
    check!(fixture(EntitlementProtocol::DerivedV1, 3, PaidForm::Open {}).is_ok());
    Ok(())
}
#[test]
fn payer_royalty_stays_a_credit_beside_full_gross_and_zero_cuts_keep_identity() -> TestResult {
    let mut q = fixture(EntitlementProtocol::DerivedV1, 1, PaidForm::Sealed {})?;
    let parent = q
        .parents
        .0
        .first_mut()
        .ok_or("parent")?
        .0
        .title
        .as_mut()
        .ok_or("title")?;
    parent.recipient = Recipient {
        account: q.payer,
        owner: q.buyer.principal(),
    };
    reprice(&mut q)?;
    let e = q.economic_subject()?;
    check_eq!(e.price, Amount::from_minor(400));
    check!(
        e.payouts()?
            .iter()
            .any(|p| p.account == q.payer && p.amount == Amount::from_minor(40))
    );
    q.price = Amount::from_minor(1);
    reprice(&mut q)?;
    check_eq!(q.payouts.0.len(), 1);
    q.parents.0.first_mut().ok_or("parent")?.0.title = None;
    check!(q.validate().is_err());
    Ok(())
}
#[test]
fn allocation_economics_is_exact_and_old_shapes_do_not_become_new_births() -> TestResult {
    let p = purchase(fixture(
        EntitlementProtocol::OriginalV3,
        0,
        PaidForm::OpenResale {},
    )?)?;
    let a = allocation(&p)?;
    p.matches_allocation(&a)?;
    check_eq!(EntitlementPurchaseV1::decode(&serde_json::to_vec(&p)?)?, p);
    for field in ["licence", "generation", "operation", "product_digest"] {
        let mut v = serde_json::to_value(&a)?;
        let economics = obj(v.get_mut("economics").ok_or("economics")?)?;
        let changed = match field {
            "licence" | "generation" => json!("99"),
            "operation" => json!(token(99)),
            _ => json!(hash(99)),
        };
        economics.insert(field.into(), changed);
        check!(serde_json::from_value::<EntitlementAllocationV1>(v).is_err());
    }
    let mut old = serde_json::to_value(&a)?;
    obj(&mut old)?.remove("economics");
    obj(&mut old)?.remove("protocol");
    check!(serde_json::from_value::<EntitlementAllocationV1>(old).is_err());
    let mut crossed = p.clone();
    crossed.effect = protocol::historical_fund(
        EntitlementProtocol::DerivedV1,
        p.quote.subject.compatibility,
        &p.participants,
    )?;
    check!(crossed.validate().is_err());
    Ok(())
}
#[test]
fn terminal_receipt_requires_exact_phase_stamp_and_grant_digest() -> TestResult {
    let mut p = purchase(fixture(
        EntitlementProtocol::OriginalV3,
        0,
        PaidForm::Sealed {},
    )?)?;
    p.phase = PurchasePhase::Refunded { commit: token(5) };
    p.updated_by = token(5);
    let r = EntitlementReceiptV1 {
        format: 1,
        operation: p.operation,
        commit: p.updated_by,
        outcome: Outcome::Refunded {},
        contract: p.funding.contract(),
        hold: p.funding.hold(),
        transaction: mush_economy::TransactionId(6),
        journal_seq: 7,
        licence: p.licence,
        term: p.term,
        quote_digest: p.quote_digest,
        product_digest: p.quote.product_digest,
        grant_economics_digest: p.quote.product.grant.digest()?,
        issued_at_ic: None,
    };
    r.matches_purchase(&p)?;
    check_eq!(EntitlementReceiptV1::decode(&serde_json::to_vec(&r)?)?, r);
    let mut v = serde_json::to_value(&r)?;
    obj(&mut v)?.remove("issued_at_ic");
    check!(serde_json::from_value::<EntitlementReceiptV1>(v).is_err());
    let mut bad = r.clone();
    bad.grant_economics_digest = hash(99);
    check!(bad.matches_purchase(&p).is_err());
    let mut bad = r.clone();
    bad.outcome = Outcome::Granted {};
    check!(bad.validate().is_err());
    p.phase = PurchasePhase::Granted { commit: token(5) };
    bad.issued_at_ic = Some(ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 170,
    });
    bad.matches_purchase(&p)?;
    bad.issued_at_ic = Some(ClockStamp {
        clock: ClockRef::WorldIc { generation: 2 },
        at: 170,
    });
    check!(bad.matches_purchase(&p).is_err());
    check!(r.matches_purchase(&p).is_err());
    Ok(())
}

#[test]
fn closed_action_domains_bind_manifest_participants_and_protocol() -> TestResult {
    let participants = PurchaseParticipants::new(
        definition("economy", "owner_participant", "initial_filing", 1)?,
        definition("roll", "owner_participant", "commercial_roots", 1)?,
    )?;
    let manifest = definition("tacsim", "registry_manifest", "combat", 1)?;
    let mut identities = std::collections::BTreeSet::new();
    for protocol in [
        EntitlementProtocol::OriginalV3,
        EntitlementProtocol::DerivedV1,
    ] {
        for transition in [
            protocol::Transition::Fund,
            protocol::Transition::Grant,
            protocol::Transition::Refund,
        ] {
            let descriptor = protocol::descriptor(protocol, transition, &manifest, &participants)?;
            let reference =
                DefinitionRegistryBuilder::new().register(OwnerId::new("market")?, descriptor)?;
            check_eq!(reference.version(), protocol.version());
            check_eq!(
                reference.key().id.as_str(),
                protocol::action(protocol, transition)
            );
            check!(identities.insert(reference.to_string()));
        }
        let historical = protocol::historical_fund(protocol, manifest.digest(), &participants)?;
        let changed = protocol::historical_fund(protocol, hash(91), &participants)?;
        check_ne!(historical, changed);
        let changed_participants = PurchaseParticipants::new(
            definition("economy", "owner_participant", "initial_filing", 1)?,
            DefinitionRegistryBuilder::new().register(
                OwnerId::new("roll")?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant")?,
                    DefinitionId::new("commercial_roots")?,
                    1,
                    json!({"different":"payload"}),
                )?,
            )?,
        )?;
        check_ne!(
            historical,
            protocol::historical_fund(protocol, manifest.digest(), &changed_participants)?
        );
    }
    check_eq!(identities.len(), 6);
    check_ne!(
        protocol::domain(EntitlementProtocol::OriginalV3, "product")?,
        "market.original_product.v2"
    );
    check_ne!(
        protocol::domain(EntitlementProtocol::DerivedV1, "quote")?,
        "market.original_quote.v3"
    );
    check!(
        protocol::descriptor(
            EntitlementProtocol::OriginalV3,
            protocol::Transition::Fund,
            &definition("tacsim", "registry_manifest", "foreign", 1)?,
            &participants
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn quote_bounds_and_measured_payouts_are_revalidated_after_repaired_hashes() -> TestResult {
    let q = fixture(EntitlementProtocol::DerivedV1, 3, PaidForm::Sealed {})?;
    let mut bad = q.clone();
    bad.expires_at_utc = bad.quoted_at_utc;
    check!(bad.validate().is_err());
    let mut bad = q.clone();
    bad.expires_at_utc = bad.quoted_at_utc + 601;
    check!(bad.validate().is_err());
    let mut bad = q.clone();
    bad.buyer = bad.subject.seller.into();
    check!(bad.validate().is_err());
    let mut bad = q.clone();
    bad.payer = bad.remainder.account;
    check!(bad.validate().is_err());
    let mut bad = q.clone();
    bad.payouts.0.first_mut().ok_or("payout")?.amount = Amount::from_minor(1);
    check!(bad.validate().is_err());
    let mut bad = q.clone();
    bad.payouts.0.reverse();
    check!(bad.validate().is_err());
    let mut bad = q.clone();
    bad.price = Amount::ZERO;
    check!(bad.validate().is_err());
    let mut bad = q.clone();
    bad.product.term_seconds = 0;
    check!(bad.product.digest().is_err());
    let p = purchase(q)?;
    let a = allocation(&p)?;
    check_eq!(p.key(), p.operation.to_hex());
    check_eq!(
        EntitlementPurchaseV1::FAMILY,
        "market_mind_entitlement_purchase"
    );
    check!(serde_json::to_vec(&p)?.len() < EntitlementPurchaseV1::MAX_BYTES);
    check!(serde_json::to_vec(&a)?.len() < 8192);
    let mut bad = serde_json::to_value(a)?;
    obj(&mut bad)?.insert("term".into(), json!("2"));
    obj(bad.get_mut("economics").ok_or("economics")?)?.insert("generation".into(), json!("2"));
    check!(serde_json::from_value::<EntitlementAllocationV1>(bad).is_err());
    Ok(())
}

/// 17g C2b (D-UM-40, C2 plan §2.5; test 6): the author's per-listing transfer
/// mark, through the code the List branch and `current_listing` run.
/// - The List wire: an absent mark reads `true`, an explicit one is kept, and
///   the request stays closed.
/// - The List offer's projected schema (what `project_offer` hands core, which
///   refuses any key the schema does not name) admits the mark as an optional
///   boolean, and the marked and the unmarked request alike.
/// - The product body (`EntitlementConfig::product_for`, which `product`
///   runs over the captured sale facts): the request's own choice builds
///   non-transferable terms and renewal template under a transferable policy;
///   a policy of `false` stays `false` whatever the mark; the mark changes the
///   product digest.
/// - `current_listing`'s recompute (`ProductChoice::frozen`) reproduces the
///   marked and the unmarked listed product exactly, so a marked listing stays
///   current.
/// - A quote over the marked listed product validates and hands the grant
///   non-transferable terms; neither a half-widened product nor a quote whose
///   product was swapped under its frozen digest validates.
#[test]
fn the_transfer_mark_narrows_and_never_widens() -> TestResult {
    use super::config::ProductChoice;
    use super::offers::{Kind, List, parameters};
    use super::{EntitlementConfig, EntitlementPolicy};
    use mush_core::ProjectedParameterKind as K;
    let choice = |v: &Value| -> TestResult<ProductChoice> {
        Ok(serde_json::from_value::<List>(v.clone())?.choice())
    };
    let with = |v: &Value, key: &str, mark: Value| -> TestResult<Value> {
        let mut v = v.clone();
        obj(&mut v)?.insert(key.into(), mark);
        Ok(v)
    };
    // The List wire.
    let base = json!({"owner_licence":"1","form":"sealed","royalty_permille":0});
    let marked = with(&base, "transferable", json!(false))?;
    check!(choice(&base)?.transferable, "absent is transferable");
    check!(!choice(&marked)?.transferable);
    check!(choice(&with(&base, "transferable", json!(true))?)?.transferable);
    for bad in [json!("no"), json!(0), Value::Null] {
        check!(choice(&with(&base, "transferable", bad)?).is_err());
    }
    check!(choice(&with(&base, "resale", json!(false))?).is_err());
    // The projected schemas, as `project_offer` builds them.
    let list = mush_core::ProjectedParameterSchema::new(parameters(Kind::List)?)?;
    let shape = list
        .parameters()
        .iter()
        .map(|p| (p.id.as_str().to_owned(), p.required))
        .collect::<Vec<_>>();
    check_eq!(
        shape,
        vec![
            ("form".to_owned(), true),
            ("owner_licence".to_owned(), true),
            ("royalty_permille".to_owned(), true),
            ("transferable".to_owned(), false),
        ]
    );
    let mark = list
        .parameters()
        .iter()
        .find(|p| p.id.as_str() == "transferable")
        .ok_or("mark")?;
    check_eq!(mark.kind, K::Boolean);
    // Core's resolve rule over the schema: every supplied key is named, and
    // every required one is supplied.
    for supplied in [&base, &marked] {
        let supplied = supplied.as_object().ok_or("object")?;
        check!(
            supplied
                .keys()
                .all(|k| list.parameters().iter().any(|p| p.id.as_str() == k))
        );
        check!(
            list.parameters()
                .iter()
                .filter(|p| p.required)
                .all(|p| supplied.contains_key(p.id.as_str()))
        );
    }
    // M8c B3: the quote names the listing and, optionally, the organisation
    // bought for (any integer from 0: organisation 0 is an organisation,
    // M8c F8).
    let quote = parameters(Kind::Quote)?;
    let shape = quote
        .iter()
        .map(|p| (p.id.as_str().to_owned(), p.required))
        .collect::<Vec<_>>();
    check_eq!(
        shape,
        vec![("listing".to_owned(), true), ("org".to_owned(), false)]
    );
    let org = quote.iter().find(|p| p.id.as_str() == "org").ok_or("org")?;
    // M8c F8: organisation 0 is an organisation.
    check_eq!(
        org.kind,
        K::Integer {
            minimum: 0,
            maximum: i64::MAX,
        }
    );
    // The deployment policy and the mark through the product body.
    let config = |transferable: bool| -> TestResult<EntitlementConfig> {
        let policy = EntitlementPolicy {
            price: Amount::from_minor(400),
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            registry: GrossRecipientCut {
                permille: 50,
                recipient: Some(Recipient {
                    account: AccountId(11),
                    owner: mush_org::PrincipalId::System(
                        mush_org::SystemPrincipalId::new("roll.registry").ok_or("system")?,
                    ),
                }),
            },
            venue_name: crate::MarketId::new("registry").ok_or("venue")?,
            venue: GrossRecipientCut {
                permille: 0,
                recipient: None,
            },
            transferable,
            term_seconds: 900,
            grace_seconds: 90,
            quote_seconds: 60,
            renewal_price: Amount::from_minor(600),
            renewal_term_seconds: 800,
            renewal_grace_seconds: 80,
        };
        Ok(EntitlementConfig::new(
            policy,
            definition("tacsim", "registry_manifest", "combat", 1)?,
            PurchaseParticipants::new(
                definition("economy", "owner_participant", "initial_filing", 1)?,
                definition("roll", "owner_participant", "commercial_roots", 1)?,
            )?,
            mush_roll::commerce::entitlement::writer::EntitlementPurchaseWriter::new(),
        )?)
    };
    let open = config(true)?;
    let closed = config(false)?;
    let q = fixture(EntitlementProtocol::OriginalV3, 0, PaidForm::Sealed {})?;
    let product = |c: &EntitlementConfig, choice: ProductChoice| {
        c.product_for(
            EntitlementProtocol::OriginalV3,
            &q.subject,
            &q.royalty,
            choice,
        )
    };
    for form in [
        PaidForm::Sealed {},
        PaidForm::Open {},
        PaidForm::OpenResale {},
    ] {
        let at = |transferable| ProductChoice {
            form,
            royalty_permille: 0,
            transferable,
        };
        let kept = product(&open, at(false))?;
        let free = product(&open, at(true))?;
        check!(!kept.terms.transferable());
        check!(!kept.grant.renewal().terms().transferable());
        check!(free.terms.transferable());
        check!(free.grant.renewal().terms().transferable());
        check!(
            kept.digest()? != free.digest()?,
            "the mark is in the digest"
        );
        // `current_listing`'s frozen recompute reproduces each listed product.
        for listed in [&kept, &free] {
            check_eq!(&product(&open, ProductChoice::frozen(listed))?, listed);
        }
        // A false policy never widens, whatever the mark says.
        for mark in [true, false] {
            let terms = closed.terms(RatingClass::II, form, mark)?;
            check!(!terms.transferable(), "a false policy never widens");
            check_eq!(terms.form(), form);
            check_eq!(terms.class(), RatingClass::II);
            check!(!product(&closed, at(mark))?.terms.transferable());
        }
        // The unmarked listing lapses when the policy closes, as before the
        // mark: its frozen `true` is not what a false policy recomputes.
        check!(product(&closed, ProductChoice::frozen(&free))? != free);
    }
    // The List request's own choice is what the branch builds from, and the
    // listing's frozen choice reads it back.
    let listed = product(&open, choice(&marked)?)?;
    check!(!listed.terms.transferable());
    check_eq!(ProductChoice::frozen(&listed), choice(&marked)?);
    // A quote over the marked listed product validates, and the grant's
    // terms and renewal template are non-transferable.
    let mut sold = q.clone();
    sold.product = listed.clone();
    sold.policy = open.policy_ref()?;
    reprice(&mut sold)?;
    sold.validate()?;
    check_eq!(sold.product_digest, listed.digest()?);
    let granted = super::publication::terms(&sold);
    check!(!granted.terms.transferable());
    check!(!granted.grant.renewal().terms().transferable());
    // A half-widened product (terms flipped, the frozen renewal template
    // kept) breaks `EntitlementProductV1::validate`'s terms = template rule.
    let mut widened = listed.clone();
    widened.terms = product(&open, choice(&base)?)?.terms;
    check!(widened.terms.transferable());
    check!(widened.validate().is_err());
    // A quote whose product is swapped for the unmarked one under the marked
    // product's frozen digest no longer validates.
    let mut swapped = sold.clone();
    swapped.product = product(&open, choice(&base)?)?;
    check!(swapped.product_digest != swapped.product.digest()?);
    check!(swapped.validate().is_err());
    Ok(())
}
