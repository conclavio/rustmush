//! Exact five-action optional composition; providers retain real owner handles.
use super::{PaidResaleConfig, families::Families, provider::Kind};
use mush_core::action::{ActionError, ActionErrorCode};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    ModError, OwnerId, Registrar,
};
use mush_economy::registry_filing::InitialFilingParticipant;
use mush_roll::commerce::paid_resale::{protocol, reader::PaidResaleReader};
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &PaidResaleConfig,
    legacy: Option<crate::mind_trade::PurchaseFamilies>,
) -> Result<Families, ModError> {
    if reg.module() != crate::NS {
        return Err(error("paid resale owner differs"));
    }
    binding(reg, config)?;
    let definitions = protocol::definitions()?;
    let actions = config.actions()?;
    let families = Families::register(reg)?;
    reg.delegate_state_family(&families.offers, "roll")?;
    reg.delegate_state_family(&families.operations, "roll")?;
    reg.delegate_state_family(&families.receipts, "roll")?;
    config.writer.install_market(
        reg,
        std::sync::Arc::new(super::compound::Writer(families.clone())),
    )?;
    let reader = PaidResaleReader::request(reg)?;
    for (index, action) in actions.iter().enumerate() {
        if reg.definition(definitions[index].clone())? != *action {
            return Err(error("paid resale action differs"));
        }
        let outcome = reg.definition(definitions[index + 5].clone())?;
        let writer = config.writer.for_action(reg, action, reader.clone())?;
        let refusal = ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "This paid licence transfer is unavailable or has changed.",
        )
        .map_err(error)?;
        if index < 3 {
            let economy = if index == 0 {
                None
            } else {
                Some(reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?)
            };
            reg.action_provider(
                action.clone(),
                super::provider::Provider {
                    action: action.clone(),
                    outcome,
                    kind: [Kind::Offer, Kind::Accept, Kind::Fund][index],
                    config: config.clone(),
                    families: families.clone(),
                    legacy: legacy.clone(),
                    economy,
                    writer,
                    refusal,
                },
            )?;
        } else {
            let economy =
                reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
            reg.action_provider(
                action.clone(),
                super::service_provider::Provider {
                    action: action.clone(),
                    outcome,
                    config: config.clone(),
                    families: families.clone(),
                    economy,
                    writer,
                    refusal,
                },
            )?;
        }
    }
    reg.paid_resale_capacity(
        config.participants.ordered(),
        actions,
        config.revocation.clone(),
    )?;
    Ok(families)
}

fn binding(reg: &mut Registrar<'_>, config: &PaidResaleConfig) -> Result<(), ModError> {
    let actions = config.actions()?;
    let writer = config.writer.clone();
    let mut refs: Vec<DefinitionRef> = config.participants.ordered().into_iter().collect();
    refs.extend(actions.iter().cloned());
    refs.extend(config.revocation.iter().cloned());
    refs.sort();
    refs.dedup();
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("paid_resale_binding").map_err(error)?,
        DefinitionId::new("roll").map_err(error)?, 1,
        serde_json::json!({"format":1,"protocol":"personal_paid_resale_v1","service":config.service}),
    ).map_err(error)?.with_references(refs.clone());
    let expected = DefinitionRegistryBuilder::new()
        .register(OwnerId::new(crate::NS).map_err(error)?, descriptor.clone())
        .map_err(error)?;
    let payload = descriptor.payload().clone();
    let service = config.service;
    let revocation = config.revocation.clone();
    reg.definition_validator(
        DefinitionKind::new("paid_resale_binding").map_err(error)?,
        move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
            let actual = ctx.definition();
            if actual.reference() != &expected
                || actual.payload() != &payload
                || actual.references() != refs.as_slice()
            {
                return Err("paid resale writer binding differs".into());
            }
            for r in &refs {
                ctx.get_exact(r).map_err(|e| e.to_string())?;
            }
            writer
                .validate_registration(&actions)
                .map_err(|e| e.to_string())?;
            writer
                .validate_service_binding(service, revocation.as_ref())
                .map_err(|e| e.to_string())
        },
    )?;
    reg.definition(descriptor)?;
    Ok(())
}
