//! Actual registration of all four IC owners. No fake seal/claim/event is
//! constructed to make an incomplete or uninitialized roster healthy.
use super::*;
use mush_core::{DefinitionRegistryBuilder, ObjId};
use mush_sim::{DomainId, InteriorSpaceRecord, SpaceAnchor, ic_exclusion::ConfiguredIcBase};
use std::collections::{BTreeMap, BTreeSet};

fn base() -> Result<ConfiguredIcBase, Box<dyn std::error::Error>> {
    let space = InteriorSpaceRecord {
        object: ObjId(700),
        anchor: SpaceAnchor::FixedDomain {
            domain: DomainId(700),
            anchor_pose: None,
        },
        compartments: BTreeMap::new(),
        rooms: BTreeSet::from([ObjId(700)]),
        egresses: BTreeMap::new(),
    };
    let descriptor = ConfiguredIcBase::descriptor(DefinitionId::new("selected_ic_base")?, &space)?;
    let reference =
        DefinitionRegistryBuilder::new().register(OwnerId::new(crate::NS)?, descriptor)?;
    Ok(ConfiguredIcBase::new(reference, space)?)
}
fn composed(config: &MindCommerce, omit: Option<&str>) -> Result<Engine, ModError> {
    let (tactics, sim) = config.configure_ic_physical(
        mush_tactics::TacticsModule::new(),
        mush_sim::SimulationModule::new(),
    );
    // Same critical ordering as production: all physical owners, then Tacsim,
    // then Roll. The deferred writer binding is proved by final definition freeze.
    let mut modules: Vec<Box<dyn GameModule>> = vec![];
    if omit != Some("sim") {
        modules.push(Box::new(sim));
    }
    if omit != Some("tactics") {
        modules.push(Box::new(tactics));
    }
    modules.push(Box::new(config.configure_economy(
        EconomyModule::new().with_title_issuers(vec![issuer()?]),
    )));
    if omit != Some("tacsim") {
        modules.push(Box::new(config.configure_tacsim(layer())?));
    }
    if omit != Some("roll") {
        modules.push(Box::new(
            config.configure_roll(RollModule::with_writers([crate::NS])),
        ));
    }
    Engine::with_modules(World::new_minimal(), Registry::new(), modules)
}
#[test]
fn ic_is_explicit_active_only_and_registers_exact_actual_four_owners() -> TestResult {
    assert!(
        prepare(CommerceMode::Initialize)?
            .with_ic_minds(vec![base()?])
            .is_err()
    );
    assert!(
        prepare(CommerceMode::Active)?
            .with_ic_minds(vec![])
            .is_err()
    );
    let b = base()?;
    assert!(
        prepare(CommerceMode::Active)?
            .with_ic_minds(vec![b.clone(), b])
            .is_err()
    );
    let ordinary = composed(&prepare(CommerceMode::Active)?, None)?;
    assert!(!has(&ordinary, "tacsim", "action", "open_ic_mind"));
    assert!(!has(
        &ordinary,
        "tactics",
        "owner_participant",
        "ic_custody"
    ));
    assert!(!has(&ordinary, "sim", "owner_participant", "ic_exclusion"));
    let config = prepare(CommerceMode::Active)?.with_ic_minds(vec![base()?])?;
    let mut actual = composed(&config, None)?;
    for (owner, kind, id) in [
        ("tacsim", "action", "open_ic_mind"),
        ("tacsim", "action", "close_ic_mind"),
        ("tacsim", "owner_participant", "ic_sortie"),
        ("tactics", "owner_participant", "ic_custody"),
        ("sim", "owner_participant", "ic_exclusion"),
        ("roll", "owner_participant", "commercial_roots"),
        ("tacsim", "ic_base", "selected_ic_base"),
    ] {
        assert!(
            has(&actual, owner, kind, id),
            "missing exact IC declaration {owner}/{kind}/{id}"
        );
    }
    assert!(config.configure_tacsim(layer())?.ic_host().is_some());
    // Registration alone cannot invent acknowledged commercial initialization.
    assert!(
        actual
            .modules_loaded(
                Default::default(),
                &mut mush_core::Outbox::new(),
                &mut mush_core::engine::Effects::new()
            )
            .is_err()
    );
    Ok(())
}
#[test]
fn ic_missing_physical_or_commercial_owner_refuses_actual_freeze() -> TestResult {
    for omitted in ["sim", "tactics", "tacsim", "roll"] {
        let config = prepare(CommerceMode::Active)?.with_ic_minds(vec![base()?])?;
        assert!(
            composed(&config, Some(omitted)).is_err(),
            "missing owner {omitted} must refuse"
        );
    }
    Ok(())
}
#[test]
fn ic_base_descriptor_is_exact_and_double_attachment_refuses() -> TestResult {
    let b = base()?;
    let actual = DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS)?, b.registration_descriptor()?)?;
    assert_eq!(&actual, b.definition());
    let configured = prepare(CommerceMode::Active)?.with_ic_minds(vec![b])?;
    assert!(configured.with_ic_minds(vec![base()?]).is_err());
    assert!(layer().ic_host().is_none());
    Ok(())
}
