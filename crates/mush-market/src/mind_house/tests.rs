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
use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_economy::managed_purchase::FundingReceipt;
use mush_economy::{AccountId, Amount, InstrumentId};
use mush_roll::commerce::house_catalogue::HouseIssuer;
use mush_roll::commerce::house_purchase::{
    HouseAllocationV1, HouseGrantEconomicsV1, HouseProtocol, HouseRenewalTemplateV1,
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
fn fixture() -> TestResult<HouseQuoteV1> {
    let protocol = HouseProtocol::HouseV1;
    let form = PaidForm::Sealed {};
    let revision = identity(10)?;
    let subject = HouseSubjectV1 {
        issuer: HouseIssuer::new(mush_org::OrgId(0), mush_org::OrgScopeId(0))?,
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
    };
    let terms: PaidTerms = serde_json::from_value(
        json!({"seats":1,"class":RatingClass::II,"form":form,"transferable":false}),
    )?;
    let revision_policy = RevisionPolicy::Pin {
        filed: subject.filed,
        revision,
    };
    let policy = definition("market", "entitlement_policy", "house_personal", 1)?;
    let instrument = InstrumentId::new("cr").ok_or("instrument")?;
    let renewal = HouseRenewalTemplateV1::new(
        instrument.clone(),
        Amount::from_minor(600),
        800,
        80,
        terms,
        revision_policy,
        policy.clone(),
    )?;
    let grant = HouseGrantEconomicsV1::new(renewal)?;
    let product = HouseProductV1 {
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
    let listing = HouseListingV1 {
        format: 1,
        publication: token(2),
        publisher: ObjId(7),
        subject,
        product,
        price: Amount::from_minor(400),
        instrument,
        venue,
        registry,
        policy,
        observed_at_utc: 100,
    };
    listing.validate()?;
    let mut q = HouseQuoteV1 {
        format: 1,
        publication: token(3),
        listing: listing.publication,
        listing_digest: listing.digest()?,
        buyer: ObjId(8),
        subject: listing.subject,
        product_digest: listing.product.digest()?,
        product: listing.product,
        price: listing.price,
        instrument: listing.instrument,
        registry: listing.registry,
        venue: listing.venue,
        remainder: Recipient {
            account: AccountId(9),
            owner: mush_org::PrincipalId::Organization(mush_org::OrgId(0)),
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
fn reprice(q: &mut HouseQuoteV1) -> TestResult {
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
fn purchase(q: HouseQuoteV1) -> TestResult<HousePurchaseV1> {
    let participants = PurchaseParticipants::new(
        definition("economy", "owner_participant", "initial_filing", 1)?,
        definition("roll", "owner_participant", "commercial_roots", 1)?,
    )?;
    let funding: FundingReceipt = serde_json::from_value(
        json!({"contract":1,"hold":2,"transaction":3,"journal_seq":4,"consent_hash":([0_u8;32]),"at_ic":{"clock":{"kind":"world_ic","generation":1},"at":100}}),
    )?;
    let p = HousePurchaseV1 {
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
fn allocation(p: &HousePurchaseV1) -> TestResult<HouseAllocationV1> {
    let q = &p.quote;
    Ok(serde_json::from_value(
        json!({"format":1,"protocol":q.product.protocol,"operation":p.operation,"participants":p.participants,
        "licence":p.licence,"term":p.term,"quote":q.publication,"quote_digest":p.quote_digest,"product_digest":q.product_digest,
        "holder":q.buyer,"issuer":q.subject.issuer,"charter":q.subject.charter,"line":q.subject.line,"filed":q.subject.filed,"revision":q.subject.revision,"rating":q.subject.rating,"terms":q.product.terms,
        "economics":{"format":1,"licence":p.licence,"generation":p.term,"operation":p.operation,"product_digest":q.product_digest,"grant":q.product.grant},
        "funding":p.funding,"phase":p.phase,"updated_by":p.updated_by}),
    )?)
}
fn obj(v: &mut Value) -> TestResult<&mut serde_json::Map<String, Value>> {
    v.as_object_mut().ok_or_else(|| "object".into())
}

#[test]
fn house_quote_is_sealed_scoped_and_conserves_actual_native_recipe() -> TestResult {
    let q = fixture()?;
    assert_eq!(HouseQuoteV1::decode(&serde_json::to_vec(&q)?)?, q);
    assert_eq!(q.subject.issuer.organization(), mush_org::OrgId(0));
    assert_eq!(q.subject.issuer.scope(), mush_org::OrgScopeId(0));
    let e = q.economic_subject()?;
    let payouts = e.payouts()?;
    assert_eq!(
        payouts
            .iter()
            .try_fold(Amount::ZERO, |sum, p| sum.checked_add(p.amount))
            .ok_or("sum")?,
        q.price
    );
    assert_eq!(payouts.len(), 3);
    assert!(payouts.iter().any(|p| p.owner
        == mush_org::PrincipalId::Organization(mush_org::OrgId(0))
        && p.amount == Amount::from_minor(372)));
    // The initial 400/900/90 product freezes independent 600/800/80 consent.
    assert_eq!(q.product.grant.renewal().price(), Amount::from_minor(600));
    assert_ne!(
        q.product.grant.renewal().term_seconds(),
        q.product.term_seconds
    );
    Ok(())
}
#[test]
fn house_nested_wire_rejects_unknown_missing_null_and_duplicate_fields() -> TestResult {
    let q = fixture()?;
    for path in [
        "/subject",
        "/product",
        "/product/grant",
        "/product/grant/renewal",
        "/registry",
        "/remainder",
    ] {
        let mut v = serde_json::to_value(&q)?;
        obj(v.pointer_mut(path).ok_or("path")?)?.insert("authorized".into(), json!(true));
        assert!(
            serde_json::from_value::<HouseQuoteV1>(v).is_err(),
            "unknown field accepted"
        );
    }
    let raw = String::from_utf8(serde_json::to_vec(&q)?)?;
    assert!(HouseQuoteV1::decode(raw.replacen("{", "{\"format\":1,", 1).as_bytes()).is_err());
    let mut missing = serde_json::to_value(&q)?;
    obj(&mut missing)?.remove("remainder");
    assert!(serde_json::from_value::<HouseQuoteV1>(missing).is_err());
    let mut scope = serde_json::to_value(&q)?;
    obj(scope
        .get_mut("subject")
        .and_then(|s| s.get_mut("issuer"))
        .ok_or("issuer")?)?
    .insert("scope".into(), Value::Null);
    assert!(serde_json::from_value::<HouseQuoteV1>(scope).is_err());
    assert!(HouseQuoteV1::decode(&vec![b' '; MAX_QUOTE_BYTES + 1]).is_err());
    Ok(())
}
#[test]
fn house_product_cannot_acquire_open_resale_follow_or_extra_seats() -> TestResult {
    let q = fixture()?;
    for form in [PaidForm::Open {}, PaidForm::OpenResale {}] {
        let mut v = serde_json::to_value(&q)?;
        *v.pointer_mut("/product/terms/form").ok_or("form")? = serde_json::to_value(form)?;
        *v.pointer_mut("/product/grant/renewal/terms/form")
            .ok_or("renewal form")? = serde_json::to_value(form)?;
        assert!(serde_json::from_value::<HouseQuoteV1>(v).is_err());
    }
    let mut v = serde_json::to_value(&q)?;
    *v.pointer_mut("/product/terms/seats").ok_or("seats")? = json!(2);
    assert!(serde_json::from_value::<HouseQuoteV1>(v).is_err());
    let mut v = serde_json::to_value(&q)?;
    *v.pointer_mut("/product/revision").ok_or("revision")? = json!({"kind":"follow","line":"10"});
    assert!(serde_json::from_value::<HouseQuoteV1>(v).is_err());
    Ok(())
}
#[test]
fn house_allocation_requires_same_issuer_economics_pin_and_operation() -> TestResult {
    let p = purchase(fixture()?)?;
    let a = allocation(&p)?;
    p.matches_allocation(&a)?;
    for (path, value) in [
        ("/issuer/scope", json!(1)),
        ("/holder", json!(99)),
        ("/product_digest", json!(hash(99))),
        ("/operation", json!(token(99))),
    ] {
        let mut v = serde_json::to_value(&a)?;
        *v.pointer_mut(path).ok_or("path")? = value;
        if let Ok(crossed) = serde_json::from_value::<HouseAllocationV1>(v) {
            assert!(p.matches_allocation(&crossed).is_err());
        }
    }
    assert_eq!(HousePurchaseV1::decode(&serde_json::to_vec(&p)?)?, p);
    Ok(())
}
#[test]
fn house_native_payout_identity_cannot_be_hidden_by_rounding() -> TestResult {
    let mut q = fixture()?;
    q.price = Amount::from_minor(1);
    reprice(&mut q)?;
    assert_eq!(q.payouts.0.len(), 1);
    let mut e = q.economic_subject()?;
    e.venue
        .as_mut()
        .ok_or("venue")?
        .recipient
        .as_mut()
        .ok_or("recipient")?
        .account = e.registry.recipient.as_ref().ok_or("registry")?.account;
    assert!(
        e.payouts().is_err(),
        "conflicting identities on rounded zero roles must refuse"
    );
    Ok(())
}
