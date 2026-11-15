//! Authenticated self-only offer projection. These forms expose selectors,
//! never hidden account/source/issuer data or an automatic acceptance action.
use super::Issuer;
use super::provider::Provider;
use mush_core::action::{ActionError, PerceivedRef, PrincipalId};
use mush_core::{
    DefinitionRef, Engine, LegalActionQuery, ProjectedParameter, ProjectedParameterKind,
    ProviderOfferDraft,
};
use mush_roll::commerce::renewal_twin::protocol::Step;

pub(crate) fn parameters(kind: Step) -> Result<Vec<ProjectedParameter>, String> {
    let p = |id: &str, required, kind| -> Result<ProjectedParameter, String> {
        Ok(ProjectedParameter {
            id: mush_core::mechanics::CanonicalId::new(id).map_err(|e| e.to_string())?,
            required,
            kind,
        })
    };
    match kind {
        Step::Draft => Ok(vec![
            p(
                "licence",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::MAX,
                },
            )?,
            p(
                "previous_operation",
                false,
                ProjectedParameterKind::Text { maximum_bytes: 32 },
            )?,
        ]),
        Step::Close => Ok(vec![
            p(
                "operation",
                true,
                ProjectedParameterKind::Text { maximum_bytes: 32 },
            )?,
            p(
                "reason",
                true,
                ProjectedParameterKind::Choice {
                    choices: vec!["declined".into(), "expired".into()],
                },
            )?,
        ]),
        _ => Ok(vec![p(
            "operation",
            true,
            ProjectedParameterKind::Text { maximum_bytes: 32 },
        )?]),
    }
}
pub(crate) fn offer<I: Issuer>(
    p: &Provider<I>,
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
        &p.config.manifest,
        p.config.participants.economy(),
        p.config.participants.retention(),
    ] {
        guards.add_definition(r.clone(), defs).map_err(|_| bad())?;
    }
    if p.kind == Step::Draft {
        guards
            .add_definition(p.config.policy_ref().map_err(|_| bad())?, defs)
            .map_err(|_| bad())?;
    }
    let label = match p.kind {
        Step::Draft => "Request a renewal offer",
        Step::Fund => "Accept a renewal offer",
        Step::Grant => "Complete a funded renewal",
        Step::Refund => "Refund a funded renewal",
        Step::Close => "Close a renewal offer",
    };
    Ok(Some(ProviderOfferDraft {
        fixed_parameters: serde_json::json!({}),
        parameters: mush_core::ProjectedParameterSchema::new(
            parameters(p.kind).map_err(|_| bad())?,
        )
        .map_err(|_| bad())?,
        targets: mush_core::ProjectedTargetSchema::None,
        required_channels: vec![],
        expires: None,
        presentation: mush_core::ActionPresentation::new(
            verb::<I>(p.kind),
            label,
            "Use the exact acknowledged offer. Requesting an offer does not accept or pay it.",
        )
        .map_err(|_| bad())?,
        guards: guards.finish().map_err(|_| bad())?,
    }))
}

/// The issuer's line selector of `kind` (`Issuer::VERBS`, in `Step::ALL`'s order).
pub(crate) fn verb<I: Issuer>(kind: Step) -> &'static str {
    let [draft, fund, grant, refund, close] = I::VERBS;
    match kind {
        Step::Draft => draft,
        Step::Fund => fund,
        Step::Grant => grant,
        Step::Refund => refund,
        Step::Close => close,
    }
}
