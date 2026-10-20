//! Fast real registration and missing-deployment gates before the expensive
//! certificate/purchase story. No commercial row or readiness is fabricated.
use super::*;
use mush_economy::managed_entitlement::{EntitlementProtocol, GrossRecipientCut, Recipient};
fn policy() -> Result<mush_market::mind_entitlement::EntitlementPolicy, ModError> {
    Ok(mush_market::mind_entitlement::EntitlementPolicy {
        price: Amount::from_minor(100),
        instrument: InstrumentId::new("cr").ok_or_else(|| ModError::msg("instrument"))?,
        registry: GrossRecipientCut {
            permille: 100,
            recipient: Some(Recipient {
                account: AccountId(1),
                owner: PrincipalId::System(issuer()?),
            }),
        },
        venue_name: MarketId::new("registry").ok_or_else(|| ModError::msg("venue"))?,
        venue: GrossRecipientCut {
            permille: 0,
            recipient: None,
        },
        transferable: true,
        term_seconds: 90 * 86_400,
        grace_seconds: 3 * 86_400,
        quote_seconds: 600,
        renewal_price: Amount::from_minor(75),
        renewal_term_seconds: 90 * 86_400,
        renewal_grace_seconds: 3 * 86_400,
    })
}
fn prepared() -> Result<MindCommerce, ModError> {
    prepare(CommerceMode::Active)?
        .with_original_source_purchases(quotes()?)?
        .with_authored_forks()?
        .with_marked_registry()?
        .with_dependent_owner_filings()
}
#[test]
fn entitlement_opt_in_preserves_old_actions_and_freezes_actual_two_owner_protocols() -> TestResult {
    assert!(
        prepare(CommerceMode::Initialize)?
            .with_mind_entitlements(policy()?)
            .is_err()
    );
    assert!(
        prepare(CommerceMode::Active)?
            .with_mind_entitlements(policy()?)
            .is_err()
    );
    let base = prepared()?;
    assert!(!base.configure_tacsim(layer())?.entitlement_holdings);
    let prepared = base.with_mind_entitlements(policy()?)?;
    let config = prepared
        .entitlements
        .as_ref()
        .ok_or("actual new configuration")?;
    let old = prepared
        .purchases
        .as_ref()
        .ok_or("actual old configuration")?
        .actions()?;
    let mut registered = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(
                prepared
                    .configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?]))?,
            ),
            // Preserve the production Market-before-Roll writer registration order.
            Box::new(
                prepared.configure_market(
                    MarketModule::new()
                        .with_venue_gate(std::sync::Arc::new(crate::port::market::PortVenueGate)),
                ),
            ),
            Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
            Box::new(prepared.configure_tacsim(layer())?),
        ],
    )?;
    let definitions = registered.registry.definitions().ok_or("definitions")?;
    let original = config.owner_actions(EntitlementProtocol::OriginalV3)?;
    let derived = config.owner_actions(EntitlementProtocol::DerivedV1)?;
    for reference in old.iter().chain(&original).chain(&derived) {
        assert_eq!(
            definitions
                .iter()
                .filter(|e| e.reference() == reference)
                .count(),
            1
        );
    }
    for id in [
        "personal_entitlement_sale",
        "personal_entitlement_holding",
        "personal_entitlement_source",
    ] {
        assert!(has(&registered, "roll", "owner_read", id));
    }
    assert!(
        registered
            .modules_loaded(
                Default::default(),
                &mut mush_core::Outbox::new(),
                &mut mush_core::engine::Effects::new()
            )
            .is_err(),
        "registration does not manufacture an acknowledged deployment"
    );
    assert!(prepared.with_mind_entitlements(policy()?).is_err());
    Ok(())
}
#[test]
fn entitlement_configuration_requires_the_exact_installed_roll_writer() -> TestResult {
    let prepared = prepared()?.with_mind_entitlements(policy()?)?;
    assert!(
        Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(
                    prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?])
                    )?
                ),
                Box::new(
                    prepared.configure_market(
                        MarketModule::new().with_venue_gate(std::sync::Arc::new(
                            crate::port::market::PortVenueGate
                        ))
                    )
                ),
                Box::new(RollModule::with_writers([crate::NS])),
                Box::new(prepared.configure_tacsim(layer())?),
            ]
        )
        .is_err()
    );
    Ok(())
}
