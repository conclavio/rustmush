//! Fixed authenticated action vocabulary shared by the offer and purchase providers.
use super::HouseConfig;
use super::guard::HouseGuard;
use mush_core::action::{ActionError, ActionInvocation, PerceivedRef, PrincipalId};
use mush_core::{DefinitionRef, Engine, LegalActionQuery, ProviderOfferDraft};
pub(super) fn requester(inv: &ActionInvocation, action: &DefinitionRef) -> Result<(), String> {
    if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner != action.key().owner
    {
        return Err("invalid house requester".into());
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(super) fn offer(
    action: &DefinitionRef,
    actual: &DefinitionRef,
    query: &LegalActionQuery,
    engine: &Engine,
    config: &HouseConfig,
    guard: &HouseGuard,
    refund: bool,
    parameters: Vec<mush_core::ProjectedParameter>,
    refusal: &ActionError,
) -> Result<Option<ProviderOfferDraft>, ActionError> {
    if action != actual
        || query.principal() != &(PrincipalId::Object { id: query.actor() })
        || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
        || query.projected_subject() != &PerceivedRef::SelfActor
        || engine.world.get(query.actor()).is_none_or(|o| {
            o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
        })
        || (!refund && guard.current(&engine.world, query.actor()).is_err())
    {
        return Ok(None);
    }
    let bad = || refusal.clone();
    let defs = engine.registry.definitions().ok_or_else(bad)?;
    let mut guards = engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())
        .map_err(|_| bad())?;
    for d in [
        action,
        &config.manifest,
        config.participants.economy(),
        config.participants.retention(),
    ] {
        guards.add_definition(d.clone(), defs).map_err(|_| bad())?;
    }
    // Historical refunds need the current closed refund definition, but do not
    // regain selling authority merely because a policy has changed.
    if !refund {
        guards
            .add_definition(config.policy_ref().map_err(|_| bad())?, defs)
            .map_err(|_| bad())?;
        guards
            .add(guard.witness(query.actor()))
            .map_err(|_| bad())?;
    }
    Ok(Some(ProviderOfferDraft {
        fixed_parameters: serde_json::json!({}),
        parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| bad())?,
        targets: mush_core::ProjectedTargetSchema::None,
        required_channels: vec![],
        expires: None,
        presentation: mush_core::ActionPresentation::new(
            "market-mind-house",
            "Mind licence terms",
            "Record organization-issued licence terms or complete their funded purchase.",
        )
        .map_err(|_| bad())?,
        guards: guards.finish().map_err(|_| bad())?,
    }))
}
pub(super) fn parameter(
    id: &str,
    kind: mush_core::ProjectedParameterKind,
) -> Result<mush_core::ProjectedParameter, String> {
    Ok(mush_core::ProjectedParameter {
        id: mush_core::mechanics::CanonicalId::new(id).map_err(|e| e.to_string())?,
        required: true,
        kind,
    })
}
