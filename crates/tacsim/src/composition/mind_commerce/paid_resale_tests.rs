//! Real optional registration, with fresh owner configurations for every boot.
use super::*;
fn configured() -> Result<MindCommerce, ModError> {
    prepare(CommerceMode::Active)?
        .with_original_purchases(quotes()?)?
        .with_paid_mind_resales(mush_core::GOD, 60)
}
fn modules(c: &MindCommerce) -> Result<Vec<Box<dyn GameModule>>, ModError> {
    Ok(vec![
        Box::new(c.configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?]))),
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
    assert!(
        prepare(CommerceMode::Initialize)?
            .with_paid_mind_resales(mush_core::GOD, 60)
            .is_err()
    );
    assert!(
        prepare(CommerceMode::Active)?
            .with_paid_mind_resales(mush_core::GOD, 60)
            .is_err()
    );
    assert!(
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
        let actual = Engine::with_modules(World::new_minimal(), Registry::new(), selected)?;
        for action in mush_roll::commerce::paid_resale::protocol::actions()? {
            actual
                .registry
                .definitions()
                .ok_or("definitions")?
                .get_exact(&action)?;
        }
    }
    let plain = prepare(CommerceMode::Active)?.with_original_purchases(quotes()?)?;
    let actual = Engine::with_modules(World::new_minimal(), Registry::new(), modules(&plain)?)?;
    for action in mush_roll::commerce::paid_resale::protocol::actions()? {
        assert!(
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
            Engine::with_modules(
                World::new_minimal(),
                Registry::new(),
                vec![
                    Box::new(e.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )),
                    Box::new(r.configure_roll(RollModule::with_writers([crate::NS]))),
                    Box::new(m.configure_market(
                        MarketModule::new().with_venue_gate(std::sync::Arc::new(
                            crate::port::market::PortVenueGate,
                        )),
                    )),
                    Box::new(selected.configure_tacsim(layer())?),
                ],
            );
        assert!(actual.is_err(), "missing paid resale owner was accepted");
    }
    let c = configured()?;
    let original = c.paid_resales.as_ref().ok_or("optional resale")?;
    let wrong = mush_market::mind_paid_resale::PaidResaleConfig::new(
        issuer()?,
        mush_core::ObjId(20),
        60,
        original.participants().clone(),
        original.writer(),
        None,
    )?;
    let actual = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(c.configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?]))),
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
    assert!(actual.is_err(), "different service binding accepted");
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
        let actual = Engine::with_modules(World::new_minimal(), Registry::new(), modules(&c)?)?;
        actual
            .registry
            .definitions()
            .ok_or("definitions")?
            .get_exact(&policy.action()?)?;
    }
    Ok(())
}
