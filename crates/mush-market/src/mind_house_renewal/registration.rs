//! Opt-in, final-freeze validated binding to the one real Roll writer and
//! existing Economy participant. No registration-order-dependent authority.
use super::config::{descriptor, error};
use super::{Families, RenewalConfig};
use mush_core::action::{ActionError, ActionErrorCode};
use mush_core::mechanics::{DefinitionKind, DefinitionRegistryBuilder, OwnerId};
use mush_core::{DefinitionRef, ModError, Registrar};
use mush_economy::registry_filing::InitialFilingParticipant;
use mush_roll::commerce::house_renewal::protocol::{self, Transition};

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &RenewalConfig,
) -> Result<Families, ModError> {
    if reg.module() != crate::NS {
        return Err(error("renewals require Market registration"));
    }
    // The action/outcome union is registered by Market's existing purchase path.
    let policy = config.policy_descriptor()?;
    reg.definition_validator(
        DefinitionKind::new("house_renewal_policy").map_err(error)?,
        crate::lot::exact_validator(std::slice::from_ref(&policy)),
    )?;
    if reg.definition(policy)? != config.policy_ref()? {
        return Err(error("renewal policy differs"));
    }
    binding(reg, config)?;
    let outcome = reg.definition(config.outcome_descriptor()?)?;
    let (families, reader) = Families::register(reg, config)?;
    let actions = config.actions()?;
    for (kind, action) in Transition::ALL.into_iter().zip(actions.iter()) {
        if reg.definition(protocol::descriptor(
            kind,
            &config.manifest,
            &config.participants,
        )?)? != *action
        {
            return Err(error("renewal action differs"));
        }
        let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
        let writer = config.writer.for_action(
            reg,
            action,
            &config.manifest,
            &config.participants,
            reader.clone(),
        )?;
        let refusal = ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "This renewal is unavailable or has changed.",
        )
        .map_err(error)?;
        reg.action_provider(
            action.clone(),
            super::provider::Provider {
                action: action.clone(),
                outcome: outcome.clone(),
                kind,
                config: config.clone(),
                families: families.clone(),
                economy,
                writer,
                refusal,
            },
        )?;
    }
    if let Some(action) = config.service_action()? {
        if reg.definition(protocol::service_descriptor(
            &config.manifest,
            &config.participants,
        )?)? != action
        {
            return Err(error("renewal service action differs"));
        }
        let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
        let writer = config.writer.for_action(
            reg,
            &action,
            &config.manifest,
            &config.participants,
            reader.clone(),
        )?;
        let refusal = ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "This retained renewal cleanup is unavailable.",
        )
        .map_err(error)?;
        reg.action_provider(
            action.clone(),
            super::service_provider::Provider {
                action: action.clone(),
                outcome: outcome.clone(),
                config: config.clone(),
                families: families.clone(),
                economy,
                writer,
                refusal,
            },
        )?;
        reg.house_mind_renewal_capacity_with_service(
            config.manifest.clone(),
            config.participants.ordered(),
            actions,
            action,
        )?;
        super::service::register(reg, config, families.clone(), reader)?;
    } else {
        reg.house_mind_renewal_capacity(
            config.manifest.clone(),
            config.participants.ordered(),
            actions,
        )?;
    }
    Ok(families)
}
fn binding(reg: &mut Registrar<'_>, config: &RenewalConfig) -> Result<(), ModError> {
    let manifest = config.manifest.clone();
    let participants = config.participants.clone();
    let actions = config.actions()?;
    let writer = config.writer.clone();
    let service_close = config.service_action()?;
    let mut refs: Vec<DefinitionRef> = vec![
        manifest.clone(),
        participants.economy().clone(),
        participants.retention().clone(),
    ];
    refs.extend(actions.iter().cloned());
    refs.extend(service_close.iter().cloned());
    refs.sort();
    refs.dedup();
    let d = descriptor(
        "house_mind_renewal_binding",
        "roll",
        serde_json::json!({"format":1,"protocol":"house_mind_renewal_v1"}),
    )?
    .with_references(refs.clone());
    let expected = DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(error)?, d.clone())
        .map_err(error)?;
    let payload = d.payload().clone();
    reg.definition_validator(
        DefinitionKind::new("house_mind_renewal_binding").map_err(error)?,
        move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
            let actual = ctx.definition();
            if actual.reference() != &expected
                || actual.payload() != &payload
                || actual.references() != refs.as_slice()
            {
                return Err("renewal writer binding differs".into());
            }
            for r in &refs {
                ctx.get_exact(r).map_err(|e| e.to_string())?;
            }
            writer
                .validate_registration(&manifest, &participants, &actions)
                .map_err(|e| e.to_string())?;
            if let Some(service) = &service_close {
                writer
                    .validate_service_registration(service)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        },
    )?;
    reg.definition(d)?;
    Ok(())
}
