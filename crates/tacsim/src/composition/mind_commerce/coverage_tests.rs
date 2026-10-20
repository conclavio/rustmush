//! Real fixed composition; registration produces no policy or host event.
use super::*;
fn configured() -> Result<MindCommerce, Box<dyn std::error::Error>> {
    Ok(prepare(CommerceMode::Active)?
        .with_ic_minds(vec![base()?])?
        .with_hull_coverage(mush_core::GOD, vec![])?)
}
fn actual(c: &MindCommerce, omit: Option<&str>) -> Result<Engine, ModError> {
    actual_on(c, omit, layer())
}
fn actual_on(
    c: &MindCommerce,
    omit: Option<&str>,
    layer: TacsimRuleset,
) -> Result<Engine, ModError> {
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
        )?));
    }
    if omit != Some("tacsim") {
        modules.push(Box::new(c.configure_tacsim(layer)?));
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

fn conduct_commerce(layer: &TacsimRuleset) -> Result<MindCommerce, ModError> {
    MindCommerce::prepare(
        layer,
        CommerceMode::Active,
        InitialFilingTerms::new(90 * 86_400, 3 * 86_400).map_err(error)?,
        InitialFilingPolicy::new(
            issuer()?,
            AccountId(1),
            Amount::from_minor(100),
            CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        )
        .map_err(error)?,
    )
}

#[test]
fn qualified_coverage_requires_the_actual_conduct_owner_not_declared_approvals() -> TestResult {
    use crate::underwriting::{
        ApprovedConductV1, CoverageProductV1, CoverageTermsV1, SafeConductRole,
    };
    use mush_core::{CommitToken, DefinitionDigest};
    use mush_org::{OrgId, OrgScopeId};
    use mush_roll::commerce::{CharterId, FiledRevisionId, LineId, RevisionIdentity};
    use std::collections::BTreeMap;

    let absent_layer = layer().with_conduct_carrier()?;
    let layer = layer().with_conduct_carrier()?;
    let configured = conduct_commerce(&layer)?.with_conduct_trials(&layer)?;
    let manifest = configured
        .conduct_manifest()
        .ok_or("actual Conduct manifest")?
        .clone();
    let selected_base = base()?;
    // Deliberately unproved input: construction and registration cannot turn
    // this declaration into a trial receipt, publication or price reduction.
    let revision: RevisionIdentity = serde_json::from_value(serde_json::json!({
        "format":1,"revision":{"key":{"character":10,"mind":1,"number":1},
        "digest":DefinitionDigest::from_bytes([1;32])},"source":DefinitionDigest::from_bytes([2;32])
    }))?;
    let approval = ApprovedConductV1::new(
        CharterId::new(20)?,
        LineId::new(1)?,
        FiledRevisionId::new(1)?,
        revision,
        DefinitionDigest::from_bytes([3; 32]),
        DefinitionDigest::from_bytes([4; 32]),
        SafeConductRole::Escort,
        "Escort".into(),
    )?;
    let product = CoverageProductV1::new(
        OrgId(1),
        OrgScopeId(2),
        AccountId(3),
        mush_core::GOD,
        CoverageTermsV1::launch(
            InstrumentId::new("cr").ok_or("cr")?,
            Amount::from_minor(100),
            Amount::from_minor(100_000),
            60,
        )?,
        vec![selected_base.definition().clone()],
        vec![selected_base.definition().clone()],
        vec![approval],
        600,
    )?;
    let selections = BTreeMap::from([(
        product.digest()?,
        vec![CommitToken::from_hex(&format!("{:032x}", 1))?],
    )]);
    assert!(
        conduct_commerce(&layer)?
            .with_ic_minds(vec![base()?])?
            .with_hull_coverage(mush_core::GOD, vec![product.clone()])
            .is_err()
    );

    let absent = conduct_commerce(&layer)?
        .with_ic_minds(vec![base()?])?
        .with_qualified_hull_coverage(
            mush_core::GOD,
            vec![product.clone()],
            manifest.clone(),
            selections.clone(),
        )?;
    assert!(actual_on(&absent, None, absent_layer).is_err());

    let present = configured
        .with_ic_minds(vec![selected_base])?
        .with_qualified_hull_coverage(mush_core::GOD, vec![product], manifest, selections)?;
    let mut engine = actual_on(&present, None, layer)?;
    assert!(has(&engine, "roll", "owner_read", "conduct_trial"));
    assert!(has(
        &engine,
        "tacsim",
        "coverage_composition",
        "hull_coverage"
    ));
    // Actual owner registration still gives no acknowledged initialization,
    // historical receipt, current qualification, or monetary authority.
    assert!(
        engine
            .modules_loaded(
                Default::default(),
                &mut mush_core::Outbox::new(),
                &mut mush_core::engine::Effects::new()
            )
            .is_err()
    );
    Ok(())
}
