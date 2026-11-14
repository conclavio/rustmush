//! Opt-in, final-freeze validated binding to the one real Roll writer and
//! existing Economy participant. No registration-order-dependent authority.
use super::Issuer;
use super::config::RenewalConfig;
use super::config::{descriptor, error};
use super::families::Families;
use mush_core::action::{ActionError, ActionErrorCode};
use mush_core::mechanics::{DefinitionKind, DefinitionRegistryBuilder, OwnerId};
use mush_core::{DefinitionRef, ModError, Registrar};
use mush_economy::registry_filing::InitialFilingParticipant;
use mush_roll::commerce::renewal_twin::protocol::{self, Step};

pub(crate) fn register<I: Issuer>(
    reg: &mut Registrar<'_>,
    config: &RenewalConfig<I>,
) -> Result<Families<I>, ModError> {
    if reg.module() != crate::NS {
        return Err(error("renewals require Market registration"));
    }
    // The action/outcome union is registered by Market's existing purchase path.
    let policy = config.policy_descriptor()?;
    reg.definition_validator(
        DefinitionKind::new(I::POLICY_KIND).map_err(error)?,
        crate::lot::exact_validator(std::slice::from_ref(&policy)),
    )?;
    if reg.definition(policy)? != config.policy_ref()? {
        return Err(error("renewal policy differs"));
    }
    binding(reg, config)?;
    let outcome = reg.definition(config.outcome_descriptor()?)?;
    let (families, reader) = Families::register(reg, config)?;
    let actions = config.actions()?;
    for (kind, action) in Step::ALL.into_iter().zip(actions.iter()) {
        if reg.definition(protocol::descriptor::<I>(
            kind,
            &config.manifest,
            &config.participants,
        )?)? != *action
        {
            return Err(error("renewal action differs"));
        }
        let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
        let writer = I::for_action(
            &config.writer,
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
        if reg.definition(protocol::service_descriptor::<I>(
            &config.manifest,
            &config.participants,
        )?)? != action
        {
            return Err(error("renewal service action differs"));
        }
        let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
        let writer = I::for_action(
            &config.writer,
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
        I::capacity_with_service(
            reg,
            config.manifest.clone(),
            config.participants.ordered(),
            actions,
            action,
        )?;
        super::service::register(reg, config, families.clone(), reader)?;
    } else {
        I::capacity(
            reg,
            config.manifest.clone(),
            config.participants.ordered(),
            actions,
        )?;
    }
    Ok(families)
}
fn binding<I: Issuer>(reg: &mut Registrar<'_>, config: &RenewalConfig<I>) -> Result<(), ModError> {
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
        I::BINDING_KIND,
        "roll",
        serde_json::json!({"format":1,"protocol":<I as mush_economy::renewal_twin::Issuer>::PROTOCOL}),
    )?
    .with_references(refs.clone());
    let expected = DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(error)?, d.clone())
        .map_err(error)?;
    let payload = d.payload().clone();
    reg.definition_validator(
        DefinitionKind::new(I::BINDING_KIND).map_err(error)?,
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
            I::validate_registration(&writer, &manifest, &participants, &actions)
                .map_err(|e| e.to_string())?;
            if let Some(service) = &service_close {
                I::validate_service_registration(&writer, service).map_err(|e| e.to_string())?;
            }
            Ok(())
        },
    )?;
    reg.definition(d)?;
    Ok(())
}
