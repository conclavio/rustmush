//! Real fixed composition; registration produces no policy or host event.
use super::*;
fn configured() -> Result<MindCommerce, Box<dyn std::error::Error>> {
    Ok(prepare(CommerceMode::Active)?
        .with_ic_minds(vec![base()?])?
        .with_hull_coverage(mush_core::GOD, vec![])?)
}
fn actual(c: &MindCommerce, omit: Option<&str>) -> Result<Engine, ModError> {
    let (tactics, sim) = c.configure_ic_physical(
        mush_tactics::TacticsModule::new(),
        mush_sim::SimulationModule::new(),
    );
    let mut modules: Vec<Box<dyn GameModule>> = vec![Box::new(mush_org::OrgModule::new())];
    if omit != Some("tactics") {
        modules.push(Box::new(tactics));
    }
    if omit != Some("sim") {
        modules.push(Box::new(sim));
    }
    if omit != Some("economy") {
        modules.push(Box::new(c.configure_economy(
            EconomyModule::new().with_title_issuers(vec![issuer()?]),
        )));
    }
    if omit != Some("tacsim") {
        modules.push(Box::new(c.configure_tacsim(layer())?));
    }
    if omit != Some("roll") {
        modules.push(Box::new(
            c.configure_roll(RollModule::with_writers([crate::NS])),
        ));
    }
    Engine::with_modules(World::new_minimal(), Registry::new(), modules)
}
#[test]
fn coverage_requires_explicit_active_ic_and_cannot_enable_twice() -> TestResult {
    for mode in [CommerceMode::Initialize, CommerceMode::Active] {
        assert!(
            prepare(mode)?
                .with_hull_coverage(mush_core::GOD, vec![])
                .is_err()
        );
    }
    assert!(
        configured()?
            .with_hull_coverage(mush_core::GOD, vec![])
            .is_err()
    );
    assert!(layer().coverage_host().is_none());
    let c = prepare(CommerceMode::Active)?.with_ic_minds(vec![base()?])?;
    let e = actual(&c, None)?;
    assert!(!has(&e, "tacsim", "action", "bind_hull_coverage"));
    Ok(())
}
#[test]
fn coverage_registers_publication_metadata_and_exact_two_monetary_owners() -> TestResult {
    let c = configured()?;
    let mut e = actual(&c, None)?;
    for id in [
        "publish_hull_coverage",
        "quote_hull_coverage",
        "bind_hull_coverage",
        "claim_hull_coverage",
        "release_hull_coverage",
    ] {
        assert!(has(&e, "tacsim", "action", id));
    }
    assert!(has(&e, "tacsim", "owner_participant", "coverage_metadata"));
    assert!(has(&e, "roll", "owner_read", "hull_coverage_open"));
    let config = c.coverage.as_ref().ok_or("configured coverage")?;
    let actual = config.actions()?;
    let defs = e.registry.definitions().ok_or("definitions")?;
    for r in actual {
        assert!(defs.get_exact(&r).is_ok());
    }
    assert!(c.configure_tacsim(layer())?.coverage_host().is_some());
    // Real boot still needs acknowledged Roll initialization; no configuration
    // or empty native inventory can fake it.
    assert!(
        e.modules_loaded(
            Default::default(),
            &mut mush_core::Outbox::new(),
            &mut mush_core::engine::Effects::new()
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn coverage_missing_actual_owner_refuses_registration() -> TestResult {
    for missing in ["tactics", "sim", "economy", "tacsim", "roll"] {
        assert!(
            actual(&configured()?, Some(missing)).is_err(),
            "missing exact owner"
        );
    }
    Ok(())
}
