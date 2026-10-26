//! Authenticated self-only offer projection. These forms expose selectors,
//! never hidden account/source/issuer data or an automatic acceptance action.
use super::provider::Provider;
use mush_core::action::{ActionError, PerceivedRef, PrincipalId};
use mush_core::{
    DefinitionRef, Engine, LegalActionQuery, ProjectedParameter, ProjectedParameterKind,
    ProviderOfferDraft,
};
use mush_roll::commerce::renewal::protocol::Transition;

pub(super) fn parameters(kind: Transition) -> Result<Vec<ProjectedParameter>, String> {
    let p = |id: &str, required, kind| -> Result<ProjectedParameter, String> {
        Ok(ProjectedParameter {
            id: mush_core::mechanics::CanonicalId::new(id).map_err(|e| e.to_string())?,
            required,
            kind,
        })
    };
    match kind {
        Transition::Draft => Ok(vec![
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
        Transition::Close => Ok(vec![
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
        &p.config.manifest,
        p.config.participants.economy(),
        p.config.participants.retention(),
    ] {
        guards.add_definition(r.clone(), defs).map_err(|_| bad())?;
    }
    if p.kind == Transition::Draft {
        guards
            .add_definition(p.config.policy_ref().map_err(|_| bad())?, defs)
            .map_err(|_| bad())?;
    }
    let label = match p.kind {
        Transition::Draft => "Request a renewal offer",
        Transition::Fund => "Accept a renewal offer",
        Transition::Grant => "Complete a funded renewal",
        Transition::Refund => "Refund a funded renewal",
        Transition::Close => "Close a renewal offer",
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
            verb(p.kind),
            label,
            "Use the exact acknowledged offer. Requesting an offer does not accept or pay it.",
        )
        .map_err(|_| bad())?,
        guards: guards.finish().map_err(|_| bad())?,
    }))
}

fn verb(kind: Transition) -> &'static str {
    match kind {
        Transition::Draft => "mind-renewal-draft",
        Transition::Fund => "mind-renewal-fund",
        Transition::Grant => "mind-renewal-grant",
        Transition::Refund => "mind-renewal-refund",
        Transition::Close => "mind-renewal-close",
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn renewal_transitions_have_unique_line_selectors() {
        let verbs: std::collections::BTreeSet<_> = [
            Transition::Draft,
            Transition::Fund,
            Transition::Grant,
            Transition::Refund,
            Transition::Close,
        ]
        .into_iter()
        .map(verb)
        .collect();
        assert_eq!(
            verbs,
            std::collections::BTreeSet::from([
                "mind-renewal-draft",
                "mind-renewal-fund",
                "mind-renewal-grant",
                "mind-renewal-refund",
                "mind-renewal-close",
            ])
        );
    }
}
