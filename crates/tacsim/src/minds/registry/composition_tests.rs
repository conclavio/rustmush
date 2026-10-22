use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn enabled() -> TacsimRuleset {
    TacsimRuleset::with_config(TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    })
}

#[test]
fn fixed_ground_is_real_atlas_content_and_missing_tiles_refuse() -> TestResult {
    let records = shipped_ground_records()?;
    let catalog = Arc::new(Catalog::build_fielded(&[], crate::NS)?);
    let policy = FrozenPolicy::capture(Arc::clone(&catalog), &records, enabled().config().clone())?;
    let reference = policy.definition_ref()?;
    let prepared = Registration::prepare(&enabled(), CommerceMode::Active)?;
    check_eq!(&reference, prepared.reference());
    check!(!records.regions.is_empty());
    check!(!records.tiles.is_empty());

    let mut changed = records.clone();
    let ground_id = mush_core::CanonicalId::new("tacsim.arena.proving_ground")?;
    let tile_key = catalog
        .arenas
        .get(&ground_id)
        .ok_or("configured ground")?
        .tile_keys(ground_id.as_str())?
        .into_iter()
        .next()
        .ok_or("configured tile")?;
    let tile = changed.tiles.get_mut(&tile_key).ok_or("applied tile")?;
    let mut cell = tile.cells.get(0, 0).ok_or("actual terrain cell")?;
    cell.elev_cm = cell.elev_cm.checked_add(1).ok_or("elevation headroom")?;
    check!(tile.cells.set(0, 0, cell));
    let changed_policy =
        FrozenPolicy::capture(Arc::clone(&catalog), &changed, enabled().config().clone())?;
    check_ne!(
        reference,
        changed_policy.definition_ref()?,
        "actual ground bytes fence certification identity"
    );

    changed.tiles.clear();
    check!(
        FrozenPolicy::capture(Arc::clone(&catalog), &changed, enabled().config().clone()).is_err()
    );
    let mut missing = (*catalog).clone();
    missing.arenas.clear();
    check!(FrozenPolicy::capture(Arc::new(missing), &records, enabled().config().clone()).is_err());
    Ok(())
}

#[test]
fn capture_and_attachment_refuse_invalid_or_changed_inputs() -> TestResult {
    check!(Registration::prepare(&TacsimRuleset::new(), CommerceMode::Active).is_err());
    let mut invalid = enabled().config().clone();
    invalid.arena.tempo.substep_max_ms = 0;
    check!(
        Registration::prepare(&TacsimRuleset::with_config(invalid), CommerceMode::Active).is_err()
    );
    let mut malformed = enabled().config().clone();
    malformed.packs.push(crate::config::PackSource {
        name: "invalid-pack".into(),
        bytes: "{}".into(),
    });
    check!(
        Registration::prepare(&TacsimRuleset::with_config(malformed), CommerceMode::Active)
            .is_err()
    );

    let registration = Registration::prepare(&enabled(), CommerceMode::Active)?;
    let mut catalog = Catalog::build_fielded(&[], crate::NS)?;
    registration.check(enabled().config(), &catalog)?;
    let mut stale = enabled().config().clone();
    stale.enabled = false;
    check!(registration.check(&stale, &catalog).is_err());
    catalog.references.clear();
    check!(registration.check(enabled().config(), &catalog).is_err());
    Ok(())
}

#[test]
fn manifest_cannot_register_without_its_actual_content_definitions() -> TestResult {
    struct Incomplete(Registration);
    impl mush_core::GameModule for Incomplete {
        fn name(&self) -> &'static str {
            crate::NS
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            self.0.policy.register(reg)?;
            Ok(())
        }
    }
    let registration = Registration::prepare(&enabled(), CommerceMode::Initialize)?;
    check!(
        Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(Incomplete(registration))]
        )
        .is_err()
    );
    Ok(())
}
