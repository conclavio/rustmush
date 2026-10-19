//! Closed opt-in registration. Exact declarations are validated after every
//! owner registers; no eager OnceLock ordering dependency is introduced.
use super::guard::HouseGuard;
use super::{Families, HouseConfig};
use super::{offers, provider};
use mush_core::mechanics::{DefinitionKind, DefinitionRegistryBuilder, OwnerId};
use mush_core::{DefinitionRef, ModError, Registrar};
use mush_economy::registry_filing::InitialFilingParticipant;
use mush_roll::commerce::house_purchase::HouseProtocol;
use mush_roll::commerce::house_purchase::writer::{listing_descriptor, purchase_descriptors};
use std::sync::Arc;
fn error(e: impl std::fmt::Display) -> ModError {
    super::error(e)
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &HouseConfig,
    gate: Arc<dyn crate::VenueGate>,
    validators_registered: bool,
    policy_registered: bool,
) -> Result<Families, ModError> {
    register_with_renewal_definitions(
        reg,
        config,
        gate,
        validators_registered,
        policy_registered,
        &[],
    )
}
pub(crate) fn register_with_renewal_definitions(
    reg: &mut Registrar<'_>,
    config: &HouseConfig,
    gate: Arc<dyn crate::VenueGate>,
    validators_registered: bool,
    policy_registered: bool,
    renewal_definitions: &[mush_core::mechanics::DefinitionDescriptor],
) -> Result<Families, ModError> {
    if reg.module() != crate::NS {
        return Err(error("entitlement providers are Market owned"));
    }
    let mut declarations = config.declarations()?;
    declarations.extend_from_slice(renewal_definitions);
    for kind in ["action", "outcome", "entitlement_policy"] {
        if (validators_registered && kind != "entitlement_policy")
            || (policy_registered && kind == "entitlement_policy")
        {
            continue;
        }
        let matching: Vec<_> = declarations
            .iter()
            .filter(|d| d.kind().as_str() == kind)
            .cloned()
            .collect();
        reg.definition_validator(
            DefinitionKind::new(kind).map_err(error)?,
            crate::lot::exact_validator(&matching),
        )?;
    }
    binding(reg, config)?;
    let policy = reg.definition(config.policy_descriptor()?)?;
    if policy != config.policy_ref()? {
        return Err(error("entitlement policy differs"));
    }
    let outcome = reg.definition(config.outcome_descriptor()?)?;
    let families = Families::register(reg, config)?;
    let guard = HouseGuard::new(config.clone(), gate)?;
    for protocol in [HouseProtocol::HouseV1] {
        let [list, fund, grant, refund] = config.owner_actions(protocol)?;
        let actual = reg.definition(listing_descriptor(
            protocol,
            &config.manifest,
            &config.participants,
        )?)?;
        if actual != list {
            return Err(error("listing definition differs"));
        }
        let writer = config.writer.for_action(
            reg,
            &list,
            &config.manifest,
            &config.participants,
            families.reader.clone(),
        )?;
        reg.action_provider(
            list.clone(),
            offers::Provider {
                action: list,
                outcome: outcome.clone(),
                protocol,
                kind: offers::Kind::List,
                config: config.clone(),
                families: families.clone(),
                writer: Some(writer),
                guard: guard.clone(),
                refusal: offers::refusal()?,
            },
        )?;
        let quote = reg.definition(config.quote_descriptor(protocol)?)?;
        if quote != config.quote_action(protocol)? {
            return Err(error("quote definition differs"));
        }
        reg.action_provider(
            quote.clone(),
            offers::Provider {
                action: quote,
                outcome: outcome.clone(),
                protocol,
                kind: offers::Kind::Quote,
                config: config.clone(),
                families: families.clone(),
                writer: None,
                guard: guard.clone(),
                refusal: offers::refusal()?,
            },
        )?;
        for ((action, descriptor), kind) in [fund, grant, refund]
            .into_iter()
            .zip(purchase_descriptors(
                protocol,
                &config.manifest,
                &config.participants,
            )?)
            .zip([
                provider::Kind::Fund,
                provider::Kind::Grant,
                provider::Kind::Refund,
            ])
        {
            if reg.definition(descriptor)? != action {
                return Err(error("entitlement purchase definition differs"));
            }
            let economy =
                reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
            let writer = config.writer.for_action(
                reg,
                &action,
                &config.manifest,
                &config.participants,
                families.reader.clone(),
            )?;
            reg.action_provider(
                action.clone(),
                provider::Provider {
                    action,
                    outcome: outcome.clone(),
                    protocol,
                    kind,
                    config: config.clone(),
                    families: families.clone(),
                    economy,
                    writer,
                    guard: guard.clone(),
                    refusal: offers::refusal()?,
                },
            )?;
        }
    }
    reg.house_mind_purchase_capacity(
        config.manifest.clone(),
        config.participants.ordered(),
        config.purchase_actions(HouseProtocol::HouseV1)?,
    )?;
    Ok(families)
}
fn binding(reg: &mut Registrar<'_>, config: &HouseConfig) -> Result<(), ModError> {
    let manifest = config.manifest.clone();
    let participants = config.participants.clone();
    let actions = config.owner_actions(HouseProtocol::HouseV1)?;
    let writer = config.writer.clone();
    let mut references: Vec<DefinitionRef> = vec![
        manifest.clone(),
        participants.economy().clone(),
        participants.retention().clone(),
    ];
    references.extend(actions.iter().cloned());
    references.sort();
    references.dedup();
    let d = super::config::descriptor(
        "house_purchase_binding",
        "roll",
        1,
        serde_json::json!({"format":1,"protocols":["house_v1"]}),
    )?
    .with_references(references.clone());
    let expected = DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(error)?, d.clone())
        .map_err(error)?;
    let payload = d.payload().clone();
    reg.definition_validator(
        DefinitionKind::new("house_purchase_binding").map_err(error)?,
        move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
            let actual = ctx.definition();
            if actual.reference() != &expected
                || actual.payload() != &payload
                || actual.references() != references.as_slice()
            {
                return Err("entitlement writer binding differs".into());
            }
            for reference in &references {
                ctx.get_exact(reference).map_err(|e| e.to_string())?;
            }
            writer
                .validate_registration(&manifest, &participants, &actions)
                .map_err(|e| e.to_string())
        },
    )?;
    reg.definition(d)?;
    Ok(())
}
