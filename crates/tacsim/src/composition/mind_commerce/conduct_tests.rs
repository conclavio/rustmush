//! Actual closed registration; no certificate or trial result is constructed.
use super::*;
use mush_core::{GameModule, Registry, World};
use mush_economy::{AccountId, Amount, CatalogRef};
use mush_org::SystemPrincipalId;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn layer() -> TacsimRuleset {
    TacsimRuleset::with_config(crate::TacsimConfig {
        enabled: true,
        ..Default::default()
    })
}
fn config(layer: &TacsimRuleset, mode: CommerceMode) -> TestResult<MindCommerce> {
    Ok(MindCommerce::prepare(
        layer,
        mode,
        InitialFilingTerms::new(90 * 86400, 3 * 86400)?,
        InitialFilingPolicy::new(
            SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
            AccountId(1),
            Amount::from_minor(100),
            CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        )?,
    )?)
}
fn modules(c: &MindCommerce, layer: TacsimRuleset) -> TestResult<Vec<Box<dyn GameModule>>> {
    Ok(vec![
        Box::new(
            c.configure_economy(EconomyModule::new().with_title_issuers(vec![
                SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
            ]))?,
        ),
        Box::new(c.configure_roll(RollModule::with_writers([crate::NS]))),
        Box::new(c.configure_tacsim(layer)?),
    ])
}
#[test]
fn conduct_requires_actual_carrier_active_mode_and_exact_captured_catalogue() -> TestResult {
    let ordinary = layer();
    check!(
        config(&ordinary, CommerceMode::Active)?
            .with_conduct_trials(&ordinary)
            .is_err()
    );
    let with = layer().with_conduct_carrier()?;
    check!(
        config(&with, CommerceMode::Initialize)?
            .with_conduct_trials(&with)
            .is_err()
    );
    let configured = config(&with, CommerceMode::Active)?.with_conduct_trials(&with)?;
    check!(configured.conduct_manifest().is_some());
    check!(configured.configure_tacsim(layer()).is_err());
    // Host timing drift still refuses even though the fixed proving manifest
    // normalizes these values to its own trial policy.
    let mut changed = with.config().clone();
    changed.match_.startup_ms += 1;
    let changed = TacsimRuleset::with_config(changed).with_conduct_carrier()?;
    check!(configured.configure_tacsim(changed).is_err());
    check!(configured.configure_tacsim(with).is_ok());
    check!(
        layer()
            .with_conduct_carrier()?
            .with_conduct_carrier()
            .is_err()
    );
    Ok(())
}
#[test]
fn actual_conduct_three_owners_register_in_both_orders_without_opening_commerce() -> TestResult {
    for reverse in [false, true] {
        let layer = layer().with_conduct_carrier()?;
        let config = config(&layer, CommerceMode::Active)?.with_conduct_trials(&layer)?;
        let mut modules = modules(&config, layer)?;
        if reverse {
            modules.reverse();
        }
        let mut engine = compose_boxed(World::new_minimal(), Registry::new(), modules)?;
        let definitions = engine.registry.definitions().ok_or("definitions")?;
        for id in [
            "conduct_trial_admit",
            "conduct_trial_grant",
            "conduct_trial_settle",
            "conduct_trial_cancel",
            "conduct_trial_recover",
        ] {
            check!(
                definitions
                    .iter()
                    .any(|d| d.reference().key().owner.as_str() == "roll"
                        && d.reference().key().kind.as_str() == "action"
                        && d.reference().key().id.as_str() == id)
            );
        }
        check!(
            definitions
                .iter()
                .any(|d| Some(d.reference()) == config.conduct_manifest())
        );
        check!(
            engine
                .modules_loaded(
                    Default::default(),
                    &mut mush_core::Outbox::new(),
                    &mut mush_core::Effects::new()
                )
                .is_err(),
            "registration cannot fabricate deployment or commercial readiness"
        );
    }
    Ok(())
}
#[test]
fn optional_carrier_preserves_actual_registry_ref_and_default_composition() -> TestResult {
    let ordinary = layer();
    let carrier = layer().with_conduct_carrier()?;
    let old = config(&ordinary, CommerceMode::Active)?;
    let new = config(&carrier, CommerceMode::Active)?;
    check_eq!(old.manifest(), new.manifest());
    check!(old.conduct_manifest().is_none());
    let engine = compose_boxed(
        World::new_minimal(),
        Registry::new(),
        modules(&old, ordinary)?,
    )?;
    check!(
        engine
            .registry
            .definitions()
            .ok_or("definitions")?
            .iter()
            .all(|d| d.reference().key().kind.as_str() != "conduct_manifest")
    );
    Ok(())
}

#[inline(never)]
fn compose_boxed(
    world: World,
    registry: Registry,
    modules: Vec<Box<dyn GameModule>>,
) -> Result<Box<mush_core::Engine>, ModError> {
    mush_core::Engine::with_modules(world, registry, modules).map(Box::new)
}
