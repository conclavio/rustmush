use super::provider::{Kind, Provider};
use mush_core::action::{ActionError, PerceivedRef, PrincipalId};
use mush_core::{
    DefinitionRef, Engine, LegalActionQuery, ProjectedParameter, ProjectedParameterKind,
    ProviderOfferDraft,
};
fn parameters(kind: Kind) -> Result<Vec<ProjectedParameter>, String> {
    let field = |id: &str, kind| {
        Ok(ProjectedParameter {
            id: mush_core::mechanics::CanonicalId::new(id).map_err(|e| e.to_string())?,
            required: true,
            kind,
        })
    };
    let integer = |minimum, maximum| ProjectedParameterKind::Integer { minimum, maximum };
    match kind {
        Kind::Offer => Ok(vec![
            field("licence", integer(1, i64::MAX))?,
            field("buyer", integer(1, i64::from(u32::MAX)))?,
            field("price", integer(1, i64::MAX))?,
            field("seller_account", integer(0, i64::MAX))?,
        ]),
        Kind::Accept | Kind::Fund => Ok(vec![
            field("offer", ProjectedParameterKind::Text { maximum_bytes: 32 })?,
            field("payer", integer(0, i64::MAX))?,
        ]),
    }
}
pub(super) fn offer(
    p: &Provider,
    action: &DefinitionRef,
    q: &LegalActionQuery,
    engine: &Engine,
) -> Result<Option<ProviderOfferDraft>, ActionError> {
    if action != &p.action
        || q.principal() != &(PrincipalId::Object { id: q.actor() })
        || q.truth_subject() != &(mush_core::SubjectKey::Object { id: q.actor() })
        || q.projected_subject() != &PerceivedRef::SelfActor
        || engine.world.get(q.actor()).is_none_or(|o| {
            o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
        })
    {
        return Ok(None);
    }
    let bad = || p.refusal.clone();
    let defs = engine.registry.definitions().ok_or_else(bad)?;
    let mut guards = engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())
        .map_err(|_| bad())?;
    for r in [
        &p.action,
        p.config.participants.economy(),
        p.config.participants.retention(),
    ] {
        guards.add_definition(r.clone(), defs).map_err(|_| bad())?;
    }
    let label = match p.kind {
        Kind::Offer => "Offer a paid licence for resale",
        Kind::Accept => "Buy an unoccupied paid licence",
        Kind::Fund => "Fund a pending paid licence resale",
    };
    Ok(Some(ProviderOfferDraft {
        fixed_parameters: serde_json::json!({}), parameters: mush_core::ProjectedParameterSchema::new(parameters(p.kind).map_err(|_| bad())?).map_err(|_| bad())?,
        targets: mush_core::ProjectedTargetSchema::None, required_channels: vec![], expires: None,
        presentation: mush_core::ActionPresentation::new("paid-mind-resale", label, "The named offer preserves the licence's existing term and restrictions. Funding does not replace its current holder until settlement.").map_err(|_| bad())?,
        guards: guards.finish().map_err(|_| bad())?,
    }))
}
