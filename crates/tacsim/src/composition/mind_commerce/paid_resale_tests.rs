//! Real optional registration, with fresh owner configurations for every boot.
use super::*;
fn configured() -> Result<MindCommerce, ModError> {
    prepare(CommerceMode::Active)?
        .with_original_purchases(quotes()?)?
        .with_paid_mind_resales(mush_core::GOD, 60)
}
fn modules(c: &MindCommerce) -> Result<Vec<Box<dyn GameModule>>, ModError> {
    Ok(vec![
        Box::new(c.configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?]))?),
        Box::new(c.configure_roll(RollModule::with_writers([crate::NS]))),
        Box::new(
            c.configure_market(
                MarketModule::new()
                    .with_venue_gate(std::sync::Arc::new(crate::port::market::PortVenueGate)),
            ),
        ),
        Box::new(c.configure_tacsim(layer())?),
    ])
}
#[test]
fn paid_resale_is_explicit_active_and_registers_the_exact_five_actions_in_both_orders() -> TestResult
{
    check!(
        prepare(CommerceMode::Initialize)?
            .with_paid_mind_resales(mush_core::GOD, 60)
            .is_err()
    );
    check!(
        prepare(CommerceMode::Active)?
            .with_paid_mind_resales(mush_core::GOD, 60)
            .is_err()
    );
    check!(
        configured()?
            .with_paid_mind_resales(mush_core::GOD, 60)
            .is_err()
    );
    for reverse in [false, true] {
        let c = configured()?;
        let mut selected = modules(&c)?;
        if reverse {
            selected.reverse();
        }
        let actual = compose_boxed(World::new_minimal(), Registry::new(), selected)?;
        for action in mush_roll::commerce::paid_resale::protocol::actions()? {
            actual
                .registry
                .definitions()
                .ok_or("definitions")?
                .get_exact(&action)?;
        }
    }
    let plain = prepare(CommerceMode::Active)?.with_original_purchases(quotes()?)?;
    let actual = compose_boxed(World::new_minimal(), Registry::new(), modules(&plain)?)?;
    for action in mush_roll::commerce::paid_resale::protocol::actions()? {
        check!(
            actual
                .registry
                .definitions()
                .ok_or("definitions")?
                .get_exact(&action)
                .is_err()
        );
    }
    Ok(())
}
#[test]
fn paid_resale_requires_all_real_owner_providers_and_the_same_service_binding() -> TestResult {
    for missing in ["economy", "roll", "market"] {
        let selected = configured()?;
        let plain = prepare(CommerceMode::Active)?.with_original_purchases(quotes()?)?;
        let e = if missing == "economy" {
            &plain
        } else {
            &selected
        };
        let r = if missing == "roll" { &plain } else { &selected };
        let m = if missing == "market" {
            &plain
        } else {
            &selected
        };
        let actual =
            compose_boxed(
                World::new_minimal(),
                Registry::new(),
                vec![
                    Box::new(e.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )?),
                    Box::new(r.configure_roll(RollModule::with_writers([crate::NS]))),
                    Box::new(m.configure_market(
                        MarketModule::new().with_venue_gate(std::sync::Arc::new(
                            crate::port::market::PortVenueGate,
                        )),
                    )),
                    Box::new(selected.configure_tacsim(layer())?),
                ],
            );
        check!(actual.is_err(), "missing paid resale owner was accepted");
    }
    let c = configured()?;
    let original = c.state.paid_resales.as_ref().ok_or("optional resale")?;
    let wrong = mush_market::mind_paid_resale::PaidResaleConfig::new(
        issuer()?,
        mush_core::ObjId(20),
        60,
        original.participants().clone(),
        original.writer(),
        None,
    )?;
    let actual = compose_boxed(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(
                c.configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?]))?,
            ),
            Box::new(c.configure_roll(RollModule::with_writers([crate::NS]))),
            Box::new(
                c.configure_market(
                    MarketModule::new()
                        .with_venue_gate(std::sync::Arc::new(crate::port::market::PortVenueGate)),
                )
                .with_paid_mind_resale(wrong),
            ),
            Box::new(c.configure_tacsim(layer())?),
        ],
    );
    check!(actual.is_err(), "different service binding accepted");
    Ok(())
}
#[test]
fn paid_resale_compound_revocation_keeps_both_explicit_configuration_orders() -> TestResult {
    for reverse in [false, true] {
        let policy = mush_roll::commerce::registry_revocation::RegistryRevocationPolicy::new(
            issuer()?,
            mush_org::OrgId(4),
            mush_org::OrgScopeId(5),
            vec![mush_core::ObjId(6)],
        )?;
        let c = prepare(CommerceMode::Active)?.with_original_purchases(quotes()?)?;
        let c = if reverse {
            c.with_paid_mind_resales(mush_core::GOD, 60)?
                .with_registry_revocation(policy.clone())?
        } else {
            c.with_registry_revocation(policy.clone())?
                .with_paid_mind_resales(mush_core::GOD, 60)?
        };
        let actual = compose_boxed(World::new_minimal(), Registry::new(), modules(&c)?)?;
        actual
            .registry
            .definitions()
            .ok_or("definitions")?
            .get_exact(&policy.action()?)?;
    }
    Ok(())
}
/// 17g C1a: a refusal-only stand-in for the launch's Coprates admission.
#[derive(Debug)]
struct Stub;
impl mush_roll::commerce::house_purchase::HouseBuyerAdmission for Stub {
    fn check(
        &self,
        _: &World,
        _: mush_roll::commerce::house_catalogue::HouseIssuer,
        _: mush_core::ObjId,
    ) -> Result<(), mush_roll::commerce::records::PublicationError> {
        Err(mush_roll::commerce::records::PublicationError::Unavailable)
    }
}
#[test]
fn paid_resale_inherits_the_house_buyer_admission() -> TestResult {
    let house = || -> Result<MindCommerce, ModError> {
        prepare(CommerceMode::Active)?
            .with_original_purchases(quotes()?)?
            .with_house_filings()
    };
    let fenced = house()?
        .with_house_minds_and_admission(house_policy()?, Some(std::sync::Arc::new(Stub)))?
        .with_paid_mind_resales(mush_core::GOD, 60)?;
    let writer = fenced
        .state
        .paid_resales
        .as_ref()
        .ok_or("paid resale configuration")?
        .writer();
    check!(writer.has_buyer_admission());
    // The fenced writer still composes with every real owner.
    house_engine(&fenced, true)?;
    let open = house()?
        .with_house_minds(house_policy()?)?
        .with_paid_mind_resales(mush_core::GOD, 60)?;
    check!(
        !open
            .state
            .paid_resales
            .as_ref()
            .ok_or("paid resale configuration")?
            .writer()
            .has_buyer_admission()
    );
    let late = house()?
        .with_paid_mind_resales(mush_core::GOD, 60)?
        .with_house_minds_and_admission(house_policy()?, Some(std::sync::Arc::new(Stub)));
    match late {
        Ok(_) => return Err("an admission after paid resale was accepted".into()),
        Err(error) => check_eq!(
            error.to_string(),
            "house buyer admission must precede paid resale"
        ),
    }
    Ok(())
}
