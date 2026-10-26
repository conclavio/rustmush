//! Exercise semantic refusal through the real provider registry, before any
//! operation can become executable. Full setup/filing tests cover valid wiring.

use super::*;
use crate::commerce::bootstrap_tests::{Manifest, manifest_descriptor, reference};
use mush_core::{Engine, GameModule, Registry, World};

struct Declaration {
    config: CommerceConfig,
    definition: DefinitionDescriptor,
}
impl GameModule for Declaration {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.config.register_validators(reg)?;
        reg.definition(self.definition.clone())?;
        Ok(())
    }
}

fn config(mode: CommerceMode) -> Result<CommerceConfig, ModError> {
    let manifest = reference("tacsim", manifest_descriptor());
    CommerceConfig::new(
        mode,
        InitialFilingTerms::new(90 * 24 * 60 * 60, 3 * 24 * 60 * 60).map_err(error)?,
        manifest.clone(),
        RatingCompatibility::combat_v1(manifest.digest()),
    )
}

fn refuses(config: CommerceConfig, definition: DefinitionDescriptor) -> Result<(), ModError> {
    let result = boxed_test_engine(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(Manifest),
            Box::new(Declaration { config, definition }),
        ],
    );
    let refusal = result
        .err()
        .ok_or_else(|| ModError::msg("tampered definition admitted"))?;
    check!(
        refusal
            .to_string()
            .contains("definition differs from the closed owner configuration"),
        "unexpected registration refusal: {refusal}"
    );
    Ok(())
}

#[test]
fn exact_registered_commerce_definitions_reject_unknown_fields_versions_and_current_flags()
-> Result<(), ModError> {
    for mode in [CommerceMode::Initialize, CommerceMode::Active] {
        let config = config(mode)?;
        for expected in config.definitions()? {
            let mut payload = expected.payload().clone();
            payload
                .as_object_mut()
                .ok_or_else(|| ModError::msg("nonobject declaration"))?
                .insert("authority".into(), json!(true));
            refuses(
                config.clone(),
                descriptor(expected.kind().as_str(), expected.id().as_str(), payload)?
                    .with_references(expected.references().to_vec()),
            )?;
            refuses(
                config.clone(),
                DefinitionDescriptor::new(
                    expected.kind().clone(),
                    expected.id().clone(),
                    2,
                    expected.payload().clone(),
                )
                .map_err(error)?
                .with_references(expected.references().to_vec()),
            )?;
            refuses(config.clone(), expected.as_current())?;
        }
    }
    Ok(())
}

#[test]
fn configured_terms_manifest_and_mode_are_not_replaceable_registration_inputs()
-> Result<(), ModError> {
    let config = config(CommerceMode::Active)?;
    let altered_terms =
        InitialFilingTerms::new(60 * 24 * 60 * 60, 3 * 24 * 60 * 60).map_err(error)?;
    refuses(
        config.clone(),
        descriptor("action", "initial_filing", altered_terms.payload())?
            .with_references(vec![config.manifest.clone()]),
    )?;
    refuses(
        config.clone(),
        descriptor("action", "initial_filing", config.terms.payload())?,
    )?;
    refuses(
        config.clone(),
        descriptor("outcome", "initial_filing_committed", json!({"format":1}))?
            .with_references(vec![config.manifest.clone()]),
    )?;
    refuses(
        config.clone(),
        descriptor(
            "action",
            "initialize_commerce",
            json!({"format":1,"explicit_setup":true}),
        )?,
    )?;
    refuses(
        config.clone(),
        descriptor(
            "owner_participant",
            "other_roots",
            json!({"format":1,"effect":"initial_original_filing"}),
        )?,
    )?;
    // Missing and mistyped fields must not be interpreted as defaults.
    refuses(
        config.clone(),
        descriptor("action", RESERVE_ACTION, json!({"format":1}))?,
    )?;
    refuses(
        config,
        descriptor("outcome", "initial_filing_committed", json!({"format":"1"}))?,
    )?;
    Ok(())
}

struct RegistryManifest(RegistryDefinitions);
impl GameModule for RegistryManifest {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        Manifest.register(reg)?;
        let expected = vec![self.0.quota().clone(), self.0.artifacts().clone()];
        reg.definition_validator(
            DefinitionKind::new("owner_participant").map_err(error)?,
            move |ctx: &DefinitionValidationContext<'_>| {
                validate_definitions("tacsim", &expected, ctx)
            },
        )?;
        reg.definition(self.0.quota().clone())?;
        reg.definition(self.0.artifacts().clone())?;
        Ok(())
    }
}
struct RegistryDeclaration {
    config: CommerceConfig,
    registry: RegistryDefinitions,
    definitions: Vec<DefinitionDescriptor>,
}
impl GameModule for RegistryDeclaration {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.config
            .register_validators_for(reg, Some(&self.registry))?;
        reg.definition(commercial_roots_descriptor()?)?;
        for definition in &self.definitions {
            reg.definition(definition.clone())?;
        }
        Ok(())
    }
}
fn freeze_registry_declarations(
    config: CommerceConfig,
    registry: RegistryDefinitions,
    definitions: Vec<DefinitionDescriptor>,
) -> Result<Box<Engine>, ModError> {
    boxed_test_engine(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(RegistryManifest(registry.clone())),
            Box::new(RegistryDeclaration {
                config,
                registry,
                definitions,
            }),
        ],
    )
}

#[test]
fn every_registry_action_accepts_only_its_exact_canonical_reference_set() -> Result<(), ModError> {
    let config = config(CommerceMode::Active)?;
    let registry = RegistryDefinitions::new(config.manifest.clone())?;
    let definitions: Vec<_> = registry.definitions().collect();
    check_eq!(
        definitions
            .iter()
            .filter(|d| d.kind().as_str() == "action")
            .count(),
        5
    );
    // The production descriptors put the manifest first; core sorts the four
    // dependencies before exposing them to this exact semantic validator.
    check!(definitions.iter().any(|d| {
        let mut sorted = d.references().to_vec();
        sorted.sort();
        sorted != d.references()
    }));
    freeze_registry_declarations(config.clone(), registry.clone(), definitions.clone())?;
    let reordered = definitions
        .iter()
        .map(|d| {
            let mut refs = d.references().to_vec();
            refs.reverse();
            d.clone().with_references(refs)
        })
        .collect();
    freeze_registry_declarations(config.clone(), registry.clone(), reordered)?;
    let outcome = reference(
        crate::NS,
        definitions
            .iter()
            .find(|d| d.kind().as_str() == "outcome")
            .cloned()
            .ok_or_else(|| ModError::msg("Registry outcome"))?,
    );

    for (at, expected) in definitions.iter().enumerate() {
        if expected.kind().as_str() != "action" {
            continue;
        }
        check_eq!(expected.references().len(), 4);
        for omitted in expected.references() {
            let mut changed = definitions.clone();
            let replacement = expected.clone().with_references(
                expected
                    .references()
                    .iter()
                    .filter(|r| *r != omitted)
                    .cloned()
                    .collect(),
            );
            *changed
                .get_mut(at)
                .ok_or_else(|| ModError::msg("fixture action"))? = replacement;
            let result = freeze_registry_declarations(config.clone(), registry.clone(), changed);
            let refusal = result
                .err()
                .ok_or_else(|| ModError::msg("missing Registry dependency admitted"))?;
            check!(
                refusal
                    .to_string()
                    .contains("definition differs from the closed owner configuration")
            );
        }
        // A different dependency really registered in this same composition is
        // still not one of the four configured authority dependencies.
        let mut refs = expected.references().to_vec();
        *refs
            .first_mut()
            .ok_or_else(|| ModError::msg("Registry dependencies"))? = outcome.clone();
        let mut changed = definitions.clone();
        *changed
            .get_mut(at)
            .ok_or_else(|| ModError::msg("fixture action"))? =
            expected.clone().with_references(refs);
        let refusal = freeze_registry_declarations(config.clone(), registry.clone(), changed)
            .err()
            .ok_or_else(|| ModError::msg("crossed Registry dependency admitted"))?;
        check!(
            refusal
                .to_string()
                .contains("definition differs from the closed owner configuration")
        );
    }
    Ok(())
}

#[test]
fn house_filing_opt_in_preserves_personal_identity_and_refuses_unconfigured_schema()
-> Result<(), ModError> {
    let personal = config(CommerceMode::Active)?;
    check!(personal.house_filing_action().is_err());
    check!(
        config(CommerceMode::Initialize)?
            .with_house_filings()
            .is_err()
    );
    let house = personal.clone().with_house_filings()?;
    check_eq!(
        house.initial_filing_action()?,
        personal.initial_filing_action()?
    );
    check!(house.clone().with_house_filings().is_err());
    refuses(personal, house.house_action_descriptor()?)?;
    let expected = house.house_action_descriptor()?;
    refuses(
        house,
        descriptor(
            "action",
            mush_economy::registry_filing::HOUSE_FILING_ACTION,
            json!({"format":1,"protocol":"scoped_original_house_filing_v1"}),
        )?
        .with_references(expected.references().to_vec()),
    )?;
    Ok(())
}

#[inline(never)]
fn boxed_test_engine(
    world: mush_core::World,
    registry: mush_core::Registry,
    modules: Vec<Box<dyn mush_core::GameModule>>,
) -> std::result::Result<Box<mush_core::Engine>, mush_core::ModError> {
    mush_core::Engine::with_modules(world, registry, modules).map(Box::new)
}
