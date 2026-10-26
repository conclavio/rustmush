//! Structural version boundaries only. Real payment/seats are proved by the
//! separate signed HouseV2 and four-owner resale child.
use super::*;

fn two() -> TestResult<HouseQuoteV1> {
    let mut q = fixture()?;
    let protocol = HouseProtocol::HouseV2;
    let terms = mush_roll::commerce::house_purchase::writer::HousePurchaseTerms::personal_terms_for_protocol(protocol, RatingClass::II, true)?;
    let policy = definition("market", "entitlement_policy", "house_personal", 2)?;
    let renewal = HouseRenewalTemplateV1::for_protocol(
        protocol,
        q.instrument.clone(),
        Amount::from_minor(600),
        800,
        80,
        terms,
        q.product.revision,
        policy.clone(),
    )?;
    q.product.protocol = protocol;
    q.product.terms = terms;
    q.product.grant = HouseGrantEconomicsV1::new(renewal)?;
    q.policy = policy;
    reprice(&mut q)?;
    Ok(q)
}

#[test]
fn house_v2_has_exact_two_seat_nested_versions_and_distinct_domains() -> TestResult {
    let old = fixture()?;
    let new = two()?;
    check_eq!(old.product.protocol, HouseProtocol::HouseV1);
    check_eq!(old.product.terms.seats().get(), 1);
    check_eq!(old.product.grant.format(), 1);
    check_eq!(old.product.grant.renewal().format(), 1);
    check_eq!(new.product.terms.seats().get(), 2);
    check_eq!(new.product.grant.format(), 2);
    check_eq!(new.product.grant.renewal().format(), 2);
    check_eq!(new.product.grant.renewal().policy().version(), 2);
    check_eq!(
        serde_json::from_slice::<HouseQuoteV1>(&serde_json::to_vec(&new)?)?,
        new
    );
    for kind in ["product", "listing", "quote"] {
        check_ne!(
            protocol::domain(HouseProtocol::HouseV1, kind)?,
            protocol::domain(HouseProtocol::HouseV2, kind)?
        );
    }
    check_eq!(
        protocol::domain(HouseProtocol::HouseV1, "product")?,
        "market.house_product.v1"
    );
    check_eq!(
        protocol::domain(HouseProtocol::HouseV1, "listing")?,
        "market.house_listing.v1"
    );
    check_eq!(
        protocol::domain(HouseProtocol::HouseV1, "quote")?,
        "market.house_quote.v1"
    );
    Ok(())
}

#[test]
fn house_versions_refuse_crossed_seats_nested_formats_and_policy() -> TestResult {
    let old = fixture()?;
    let new = two()?;
    for (mut value, path, replacement) in [
        (
            serde_json::to_value(&old.product)?,
            "/terms/seats",
            json!(2),
        ),
        (
            serde_json::to_value(&new.product)?,
            "/terms/seats",
            json!(1),
        ),
        (
            serde_json::to_value(&new.product)?,
            "/protocol",
            json!("house_v1"),
        ),
        (
            serde_json::to_value(&old.product)?,
            "/protocol",
            json!("house_v2"),
        ),
        (
            serde_json::to_value(&new.product)?,
            "/grant/format",
            json!(1),
        ),
        (
            serde_json::to_value(&new.product)?,
            "/grant/renewal/format",
            json!(1),
        ),
        (
            serde_json::to_value(&new.product)?,
            "/grant/renewal/policy",
            serde_json::to_value(old.product.grant.renewal().policy())?,
        ),
        (
            serde_json::to_value(&new.product)?,
            "/grant/renewal/terms/seats",
            json!(1),
        ),
    ] {
        *value.pointer_mut(path).ok_or("fixture leaf")? = replacement;
        check!(serde_json::from_value::<HouseProductV1>(value).is_err());
    }
    let mut value = serde_json::to_value(&new.product)?;
    value
        .as_object_mut()
        .ok_or("product")?
        .insert("seats_override".into(), json!(2));
    check!(serde_json::from_value::<HouseProductV1>(value).is_err());
    Ok(())
}

#[test]
fn house_v1_action_bytes_remain_exact_and_v2_requires_its_full_refs() -> TestResult {
    let manifest = definition("tacsim", "registry_manifest", "combat", 1)?;
    let participants = PurchaseParticipants::new(
        definition("economy", "owner_participant", "initial_filing", 1)?,
        definition("roll", "owner_participant", "commercial_roots", 1)?,
    )?;
    for (t, id, name) in [
        (protocol::Transition::Fund, "purchase_house_mind", "fund"),
        (protocol::Transition::Grant, "settle_house_mind", "grant"),
        (protocol::Transition::Refund, "refund_house_mind", "refund"),
    ] {
        let refs = vec![
            manifest.clone(),
            participants.economy().clone(),
            participants.retention().clone(),
        ];
        let old = protocol::descriptor(HouseProtocol::HouseV1, t, &manifest, &participants)?;
        let expected = DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new(id)?,
            1,
            json!({"format":1,"protocol":"house_mind_purchase_v1","transition":name}),
        )?
        .with_references(refs.clone());
        check_eq!(old.payload(), expected.payload());
        let reference = |d| -> TestResult<DefinitionRef> {
            Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("market")?, d)?)
        };
        check_eq!(reference(old)?, reference(expected)?);
        let new = protocol::descriptor(HouseProtocol::HouseV2, t, &manifest, &participants)?;
        check_eq!(
            new.payload(),
            &json!({"format":2,"protocol":"house_mind_purchase_v2","transition":name})
        );
        let expected_new = DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new(id)?,
            2,
            json!({"format":2,"protocol":"house_mind_purchase_v2","transition":name}),
        )?
        .with_references(refs);
        check_eq!(reference(new)?, reference(expected_new)?);
    }
    Ok(())
}

#[test]
fn house_two_seat_opt_in_preserves_old_policy_and_refuses_duplicate_configuration() -> TestResult {
    let q = fixture()?;
    let policy = crate::mind_house::HousePolicy {
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
    let manifest = definition("tacsim", "registry_manifest", "combat", 1)?;
    let participants = PurchaseParticipants::new(
        definition("economy", "owner_participant", "initial_filing", 1)?,
        definition("roll", "owner_participant", "commercial_roots", 1)?,
    )?;
    let old = crate::mind_house::HouseConfig::new(
        policy.clone(),
        manifest,
        participants,
        mush_roll::commerce::house_purchase::writer::HousePurchaseWriter::new(),
    )?;
    check!(old.owner_actions(HouseProtocol::HouseV2).is_err());
    let one = old.owner_actions(HouseProtocol::HouseV1)?;
    let old_policy = old.policy_ref()?;
    let both = old.with_two_seat_pins(policy.clone())?;
    check_eq!(both.owner_actions(HouseProtocol::HouseV1)?, one);
    check_eq!(both.policy_ref()?, old_policy);
    check_eq!(
        both.policy_ref_for_protocol(HouseProtocol::HouseV2)?
            .version(),
        2
    );
    check!(
        both.owner_actions(HouseProtocol::HouseV2)?
            .iter()
            .all(|r| r.version() == 2)
    );
    check!(both.with_two_seat_pins(policy).is_err());
    Ok(())
}
