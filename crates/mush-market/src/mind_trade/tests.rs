//! Codec and real registration/refusal tests. The successful paid operation,
//! real Registry certification and funding/restart gates remain separate.
use super::*;
use mush_core::{ClockRef, ClockStamp, ObjId};
use mush_roll::commerce::rating::{RatingClass, RatingRef};
use mush_roll::commerce::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
    WorldIcSeconds,
};
use records::*;

fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn hash() -> DefinitionDigest {
    DefinitionDigest::for_canonical_value("market.quote.test", &json!({}))
        .expect("valid test fixture")
}
fn at(n: i64) -> WorldIcSeconds {
    WorldIcSeconds::from_stamp(&ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: n,
    })
    .expect("valid test fixture")
}
fn subject() -> OriginalSubject {
    let identity: RevisionIdentity = serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":7,"mind":1,"number":1},"digest":hash()},"source":hash()}))
    .expect("valid test fixture");
    let rating: RatingRef = serde_json::from_value(
        json!({"job":token(1),"id":hash(),"schema":1,"receipt_digest":hash()}),
    )
    .expect("valid test fixture");
    OriginalSubject {
        seller: ObjId(7),
        charter: CharterId::new(20).expect("valid test fixture"),
        title_revision: 1,
        line: LineId::new(1).expect("valid test fixture"),
        filed: FiledRevisionId::new(1).expect("valid test fixture"),
        sequence: 1,
        owner_licence: LicenceId::new(1).expect("valid test fixture"),
        owner_term: TermGeneration::new(1).expect("valid test fixture"),
        owner_window: TermWindow::new(at(0), at(0), at(100), at(110)).expect("valid test fixture"),
        revision: identity,
        schema: hash(),
        definition: hash(),
        rating,
        compatibility: hash(),
        declared: RatingClass::II,
        minimum: RatingClass::II,
    }
}
fn product(s: &OriginalSubject) -> OriginalProduct {
    OriginalProduct {
        kind: ProductKind::OriginalPersonal {},
        class: RatingClass::II,
        seats: 1,
        form: mush_roll::commerce::paid::PaidForm::Sealed {},
        revision: mush_roll::commerce::records::RevisionPolicy::Pin {
            filed: s.filed,
            revision: s.revision,
        },
        transferable: true,
        term_seconds: 90 * 86400,
        grace_seconds: 3 * 86400,
    }
}
fn venue() -> QuoteVenue {
    QuoteVenue::new(
        crate::MarketId::new("registry").expect("valid test fixture"),
        mush_org::PrincipalId::System(
            SystemPrincipalId::new("registry.venue").expect("valid test fixture"),
        ),
        AccountId(12),
        20,
    )
    .expect("valid test fixture")
}
fn listing() -> MindListing {
    let s = subject();
    MindListing {
        format: 1,
        publication: token(2),
        product: product(&s),
        subject: s,
        price: Amount::from_minor(400),
        instrument: InstrumentId::new("cr").expect("valid test fixture"),
        venue: venue(),
        registry_recipient: SystemPrincipalId::new("roll.registry").expect("valid test fixture"),
        registry_account: AccountId(11),
        registry_permille: 50,
        policy: hash(),
        observed_at_utc: 100,
    }
}
fn quote() -> MindQuote {
    let l = listing();
    let split = mush_roll::commerce::proceeds::calculate(
        l.price,
        mush_roll::commerce::proceeds::GrossCut {
            to: l.registry_account,
            permille: l.registry_permille,
        },
        Some(mush_roll::commerce::proceeds::GrossCut {
            to: l.venue.account,
            permille: l.venue.permille,
        }),
        &[],
        AccountId(9),
    )
    .expect("valid test fixture");
    MindQuote {
        format: 1,
        publication: token(3),
        listing: l.publication,
        listing_digest: l.digest().expect("valid test fixture"),
        buyer: ObjId(8),
        subject: l.subject,
        product_digest: l.product.digest().expect("valid test fixture"),
        product: l.product,
        price: l.price,
        instrument: l.instrument,
        venue: l.venue,
        registry_recipient: l.registry_recipient,
        registry_account: l.registry_account,
        registry_permille: l.registry_permille,
        payer: AccountId(8),
        seller_account: AccountId(9),
        payouts: payouts(split).expect("valid test fixture"),
        policy: l.policy,
        quoted_at_utc: 100,
        expires_at_utc: 160,
    }
}

#[test]
fn strict_quote_roundtrips_real_identifiers_and_exact_gross_split() {
    let l = listing();
    assert_eq!(
        MindListing::decode(&serde_json::to_vec(&l).expect("valid test fixture"))
            .expect("valid test fixture"),
        l
    );
    let q = quote();
    assert_eq!(
        MindQuote::decode(&serde_json::to_vec(&q).expect("valid test fixture"))
            .expect("valid test fixture"),
        q
    );
    assert_eq!(
        q.payouts
            .iter()
            .map(|p| (p.account, p.amount))
            .collect::<Vec<_>>(),
        vec![
            (AccountId(9), Amount::from_minor(372)),
            (AccountId(11), Amount::from_minor(20)),
            (AccountId(12), Amount::from_minor(8))
        ]
    );
    assert_ne!(
        q.digest().expect("valid test fixture"),
        l.digest().expect("valid test fixture")
    );
}

#[test]
fn promoted_certificate_keeps_actual_minimum_class_in_the_product() {
    let mut listing = listing();
    listing.subject.declared = RatingClass::I;
    assert_eq!(listing.subject.minimum, RatingClass::II);
    assert_eq!(listing.product.class, RatingClass::II);
    listing.validate().expect("measured promotion is lawful");
    let encoded = serde_json::to_vec(&listing).expect("encode promoted listing");
    assert_eq!(
        MindListing::decode(&encoded).expect("promoted listing"),
        listing
    );
    listing.product.class = RatingClass::I;
    assert!(
        listing.validate().is_err(),
        "cannot sell below actual certification"
    );
    listing.product.class = RatingClass::III;
    listing.subject.declared = RatingClass::III;
    assert!(
        listing.validate().is_err(),
        "certificate cannot undercut declaration"
    );
}

#[test]
fn nested_unknowns_and_normalized_aliases_are_not_silently_admitted() {
    for pointer in [
        "/product/form",
        "/subject/rating",
        "/subject/revision/revision",
        "/venue/recipient",
    ] {
        let mut value = json!(quote());
        value
            .pointer_mut(pointer)
            .expect("valid test fixture")
            .as_object_mut()
            .expect("valid test fixture")
            .insert("authorised".into(), json!(true));
        assert!(
            MindQuote::decode(&serde_json::to_vec(&value).expect("valid test fixture")).is_err(),
            "{pointer}"
        );
    }
    for pointer in ["/instrument", "/venue/name"] {
        let mut value = json!(quote());
        *value.pointer_mut(pointer).expect("valid test fixture") = json!(" REGISTRY ");
        assert!(
            MindQuote::decode(&serde_json::to_vec(&value).expect("valid test fixture")).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn duplicate_raw_consent_fields_refuse_before_value_normalization() {
    let raw = serde_json::to_string(&quote()).expect("quote JSON");
    for field in ["\"format\":1", "\"seats\":1"] {
        assert!(raw.contains(field));
        let ambiguous = raw.replacen(field, &format!("{field},{field}"), 1);
        assert!(MindQuote::decode(ambiguous.as_bytes()).is_err(), "{field}");
    }
}

#[test]
fn consent_sensitive_terms_and_each_observed_identity_change_the_digest() {
    let baseline = quote();
    let before = baseline.digest().expect("valid test fixture");
    let mut variants = Vec::new();
    let mut q = baseline.clone();
    q.buyer = ObjId(9);
    variants.push(q);
    let mut q = baseline.clone();
    q.subject.title_revision += 1;
    variants.push(q);
    let mut q = baseline.clone();
    q.listing = token(9);
    variants.push(q);
    let mut q = baseline.clone();
    q.expires_at_utc += 1;
    variants.push(q);
    let mut q = baseline.clone();
    q.product.transferable = false;
    q.product_digest = q.product.digest().expect("valid test fixture");
    variants.push(q);
    let mut q = baseline.clone();
    q.product.term_seconds -= 1;
    q.product_digest = q.product.digest().expect("valid test fixture");
    variants.push(q);
    for q in variants {
        q.validate().expect("valid test fixture");
        assert_ne!(q.digest().expect("valid test fixture"), before);
    }
}

#[test]
fn structural_quote_refuses_misbound_product_rate_account_and_clock() {
    let mut cases = Vec::new();
    let q = quote();
    let mut c = q.clone();
    c.buyer = c.subject.seller;
    cases.push(c);
    let mut c = q.clone();
    c.payer = c.seller_account;
    cases.push(c);
    let mut c = q.clone();
    c.product.class = RatingClass::I;
    cases.push(c);
    let mut c = q.clone();
    c.registry_permille = 990;
    cases.push(c);
    let mut c = q.clone();
    c.expires_at_utc = c.quoted_at_utc;
    cases.push(c);
    let mut c = q.clone();
    c.expires_at_utc = c.quoted_at_utc + MAX_QUOTE_SECONDS + 1;
    cases.push(c);
    let mut c = q.clone();
    c.product.term_seconds = mush_roll::commerce::MAX_TERM_SECONDS + 1;
    cases.push(c);
    let mut c = q.clone();
    c.product.seats = 2;
    cases.push(c);
    let mut c = q.clone();
    c.payouts.reverse();
    cases.push(c);
    let mut c = q.clone();
    c.product.revision = mush_roll::commerce::records::RevisionPolicy::Follow {
        line: c.subject.line,
    };
    cases.push(c);
    for c in cases {
        assert!(MindQuote::decode(&serde_json::to_vec(&c).expect("valid test fixture")).is_err());
    }
    assert!(MindQuote::decode(&vec![b' '; MAX_QUOTE_BYTES + 1]).is_err());
    assert!(MindListing::decode(&vec![b' '; MAX_LISTING_BYTES + 1]).is_err());
}

#[derive(Debug)]
struct Gate;
impl crate::VenueGate for Gate {
    fn venue_of_room(&self, _: &mush_core::World, _: ObjId) -> Option<crate::MarketId> {
        crate::MarketId::new("registry")
    }
    fn endpoint_for(
        &self,
        _: &mush_core::World,
        _: ObjId,
        _: &crate::MarketId,
        _: crate::Presence,
    ) -> Option<mush_inventory::EndpointRef> {
        None
    }
    fn admit(
        &self,
        _: &mush_core::World,
        _: ObjId,
        _: &crate::MarketId,
        _: Option<mush_inventory::EndpointRef>,
        _: crate::Presence,
    ) -> Result<(), crate::VenueRefusal> {
        Ok(())
    }
    fn governed(&self, _: &mush_core::World, _: mush_inventory::EndpointRef) -> bool {
        false
    }
    fn port_seller(
        &self,
        _: &mush_core::World,
        _: &crate::MarketId,
        _: &mush_org::PrincipalId,
    ) -> bool {
        false
    }
}
struct Manifest;
fn manifest_descriptor() -> DefinitionDescriptor {
    descriptor(
        "registry_manifest",
        "combat",
        json!({"fixture":"registration_refusal_only"}),
    )
    .expect("valid test fixture")
}
fn manifest() -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("tacsim").expect("valid test fixture"),
            manifest_descriptor(),
        )
        .expect("valid test fixture")
}
impl mush_core::GameModule for Manifest {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, r: &mut Registrar<'_>) -> Result<(), ModError> {
        let expected = manifest_descriptor();
        r.definition_validator(
            DefinitionKind::new("registry_manifest").map_err(error)?,
            move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
                let d = ctx.definition();
                if d.reference().key().owner.as_str() == "tacsim"
                    && d.reference().key().id.as_str() == "combat"
                    && d.reference().version() == 1
                    && d.payload() == expected.payload()
                    && d.references().is_empty()
                {
                    Ok(())
                } else {
                    Err("unexpected fixture manifest".into())
                }
            },
        )?;
        r.definition(manifest_descriptor())?;
        Ok(())
    }
}
fn config() -> OriginalQuoteConfig {
    OriginalQuoteConfig::new(
        OriginalQuotePolicy::new(
            OriginalPrice::new(
                Amount::from_minor(400),
                InstrumentId::new("cr").expect("valid test fixture"),
                SystemPrincipalId::new("roll.registry").expect("valid test fixture"),
                AccountId(11),
                50,
            )
            .expect("valid test fixture"),
            venue(),
            OriginalDuration::new(true, 90 * 86400, 3 * 86400, 60).expect("valid test fixture"),
        )
        .expect("valid test fixture"),
        manifest(),
    )
    .expect("valid test fixture")
}
fn engine(with_gate: bool) -> Result<mush_core::Engine, ModError> {
    let mut market = crate::MarketModule::new().with_original_mind_quotes(config());
    if with_gate {
        market = market.with_venue_gate(Arc::new(Gate));
    }
    engine_with_market(Box::new(market))
}
fn engine_with_market(
    market: Box<dyn mush_core::GameModule>,
) -> Result<mush_core::Engine, ModError> {
    let roll = mush_roll::commerce::module::CommerceConfig::new(
        mush_roll::commerce::module::CommerceMode::Active,
        mush_roll::commerce::filing::InitialFilingTerms::new(90 * 86400, 3 * 86400)
            .expect("valid test fixture"),
        manifest(),
        mush_roll::commerce::rating::RatingCompatibility::combat_v1(manifest().digest()),
    )
    .expect("valid test fixture")
    .with_original_market_quotes();
    let issuer = SystemPrincipalId::new("roll.registry").expect("valid test fixture");
    let policy = mush_economy::registry_filing::InitialFilingPolicy::new(
        issuer.clone(),
        AccountId(11),
        Amount::from_minor(100),
        mush_economy::CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )
    .expect("valid test fixture");
    let economy = mush_economy::EconomyModule::new()
        .with_title_issuers(vec![issuer])
        .with_initial_filing(
            policy,
            roll.initial_filing_action().expect("valid test fixture"),
        );
    mush_core::Engine::with_modules(
        mush_core::World::new_minimal(),
        mush_core::Registry::new(),
        vec![
            Box::new(Manifest),
            Box::new(economy),
            Box::new(mush_roll::RollModule::with_writers(["tacsim"]).with_commerce(roll)),
            market,
        ],
    )
}

#[path = "lifecycle_tests.rs"]
mod lifecycle;
fn invocation(parameters: Value) -> mush_core::ActionInvocation {
    invocation_as(parameters, mush_core::GOD)
}
fn invocation_as(parameters: Value, actor: ObjId) -> mush_core::ActionInvocation {
    use mush_core::action::*;
    let request = RequestUid::from_bytes([9; 16]);
    let principal = PrincipalId::Object { id: actor };
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("market").expect("valid test fixture"),
                namespace: CanonicalId::new("original_quote_test").expect("valid test fixture"),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .expect("valid test fixture")
}
#[test]
fn actual_registered_listing_refuses_unrecovered_roll_and_caller_supplied_terms_without_writes() {
    let mut e = engine(true).expect("valid test fixture");
    let action = config().listing_action().expect("valid test fixture");
    for parameters in [
        json!({"owner_licence":"1"}),
        json!({"owner_licence":"1","price":1}),
        json!({"owner_licence":"1","seller":9}),
        json!({"owner_licence":"1","validated":true}),
    ] {
        assert!(
            e.prepare_registered_composed_authoritative_action(
                &action,
                &invocation(parameters),
                token(6)
            )
            .is_err()
        );
        assert_eq!(e.action_state().len(), 0);
        assert!(!e.action_state().has_prepared());
        assert!(!e.action_state().reservation_lost());
    }
    let error = engine(false)
        .err()
        .expect("missing real venue gate must refuse");
    assert!(error.to_string().contains("actual venue gate"), "{error}");
}

#[path = "purchase_order_tests.rs"]
mod purchase_order;

#[test]
fn source_forms_are_explicit_v2_products_and_v1_sealed_bytes_keep_their_domains() {
    use mush_roll::commerce::paid::PaidForm;
    let old = quote();
    let old_bytes = serde_json::to_vec(&old).expect("v1 quote");
    assert_eq!(
        old.product.digest().expect("product"),
        digest("market.original_product.v1", &old.product, MAX_QUOTE_BYTES)
            .expect("old product domain")
    );
    assert_eq!(
        old.digest().expect("quote"),
        digest("market.original_quote.v1", &old, MAX_QUOTE_BYTES).expect("old quote domain")
    );
    assert_eq!(
        MindQuote::decode(&old_bytes).expect("historical quote"),
        old
    );
    for form in [
        PaidForm::Sealed {},
        PaidForm::Open {},
        PaidForm::OpenResale {},
    ] {
        let mut l = listing();
        l.format = 2;
        l.product.kind = ProductKind::OriginalPersonalSourceForms {};
        l.product.form = form;
        let mut q = old.clone();
        q.format = 2;
        q.listing_digest = l.digest().expect("new listing");
        q.product = l.product.clone();
        q.product_digest = q.product.digest().expect("new product");
        assert_eq!(
            MindListing::decode(&serde_json::to_vec(&l).expect("listing")).expect("v2 listing"),
            l
        );
        assert_eq!(
            MindQuote::decode(&serde_json::to_vec(&q).expect("quote")).expect("v2 quote"),
            q
        );
        assert_ne!(
            q.product_digest, old.product_digest,
            "even v2 Sealed has explicit new protocol identity"
        );
        for (pointer, replacement) in [
            ("/format", json!(1)),
            ("/product/kind", json!({"kind":"original_personal"})),
            ("/product/form", json!({"kind":"open","authority":true})),
            ("/product/form", json!({"kind":"all"})),
        ] {
            let mut bad = json!(q);
            *bad.pointer_mut(pointer).expect("field") = replacement;
            assert!(MindQuote::decode(&serde_json::to_vec(&bad).expect("bad quote")).is_err());
        }
    }
    for form in [PaidForm::Open {}, PaidForm::OpenResale {}] {
        let mut bad = old.clone();
        bad.product.form = form;
        assert!(
            bad.product.validate(&bad.subject).is_err(),
            "v1 cannot become Open by changing a form"
        );
        assert!(MindQuote::decode(&serde_json::to_vec(&bad).expect("bad v1")).is_err());
    }
    assert_eq!(serde_json::to_vec(&old).expect("v1 reencode"), old_bytes);
}
#[test]
fn source_form_configuration_is_explicit_and_rejects_implicit_listing_consent() {
    let old = config();
    let new = old.clone().with_source_forms();
    assert_eq!(old.listing_action().expect("old action").version(), 1);
    assert_eq!(new.listing_action().expect("new action").version(), 2);
    assert_ne!(
        old.policy_digest().expect("old policy"),
        new.policy_digest().expect("new policy")
    );
    let payload = old.action(LIST_ACTION).expect("old descriptor");
    assert_eq!(
        payload.payload(),
        &json!({"format":1,"original_only":true,"personal_only":true,"subject":"self_actor","targets":0,"funding":false,"terms":old.policy,"product":{"source":"sealed","seats":1,"revision":"pin","class":"certified_minimum"}})
    );
    // Actual provider parsing is also covered through the genuine registered
    // purchase story: v2 missing-form and caller-added holder both refuse.
}
