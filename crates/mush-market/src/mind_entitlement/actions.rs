//! Fixed authenticated action vocabulary shared by the offer and purchase providers.
use super::EntitlementConfig;
use super::guard::EntitlementGuard;
use mush_core::action::{ActionError, ActionInvocation, PerceivedRef, PrincipalId};
use mush_core::{DefinitionRef, Engine, LegalActionQuery, ProviderOfferDraft};
pub(super) fn requester(inv: &ActionInvocation, action: &DefinitionRef) -> Result<(), String> {
    if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner != action.key().owner
    {
        return Err("invalid entitlement requester".into());
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(super) fn offer(
    action: &DefinitionRef,
    actual: &DefinitionRef,
    query: &LegalActionQuery,
    engine: &Engine,
    config: &EntitlementConfig,
    guard: &EntitlementGuard,
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
        presentation: presentation(action).map_err(|_| bad())?,
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

// Called only after matching the provider's exact registered action. These
// names select an offer; its full definition and current guards still authorize it.
fn presentation(action: &DefinitionRef) -> Result<mush_core::ActionPresentation, String> {
    if action.key().owner.as_str() != crate::NS || action.key().kind.as_str() != "action" {
        return Err("unknown entitlement presentation".into());
    }
    let (product, version, transition) = match (action.key().id.as_str(), action.version()) {
        ("list_original_mind", 3) => ("original", 3, "list"),
        ("quote_original_mind", 3) => ("original", 3, "quote"),
        ("purchase_original_mind", 3) => ("original", 3, "fund"),
        ("settle_original_mind", 3) => ("original", 3, "grant"),
        ("refund_original_mind", 3) => ("original", 3, "refund"),
        ("list_derived_mind", 1) => ("derived", 1, "list"),
        ("quote_derived_mind", 1) => ("derived", 1, "quote"),
        ("purchase_derived_mind", 1) => ("derived", 1, "fund"),
        ("settle_derived_mind", 1) => ("derived", 1, "grant"),
        ("refund_derived_mind", 1) => ("derived", 1, "refund"),
        _ => return Err("unknown entitlement presentation".into()),
    };
    mush_core::ActionPresentation::new(
        format!("{product}-{transition}-v{version}"),
        format!("{product} licence: {transition}"),
        "Use the exact current listing, quote or funded operation returned by the service.",
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    use mush_core::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    };
    use std::collections::BTreeSet;

    // Structural caller vocabulary; the genuine flow resolves actual signed offers.
    #[test]
    fn entitlement_protocols_and_transitions_have_unique_line_selectors()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut verbs = BTreeSet::new();
        for (product, version) in [("original", 3), ("derived", 1)] {
            for id in ["list", "quote", "purchase", "settle", "refund"] {
                let action = DefinitionRegistryBuilder::new().register(
                    OwnerId::new(crate::NS)?,
                    DefinitionDescriptor::new(
                        DefinitionKind::new("action")?,
                        DefinitionId::new(format!("{id}_{product}_mind"))?,
                        version,
                        serde_json::json!({"presentation_probe":true}),
                    )?,
                )?;
                let p = presentation(&action)?;
                check!(verbs.insert(p.verb), "ambiguous entitlement line selector");
            }
        }
        let expected: BTreeSet<_> = [("original", 3), ("derived", 1)]
            .into_iter()
            .flat_map(|(product, version)| {
                ["list", "quote", "fund", "grant", "refund"]
                    .into_iter()
                    .map(move |t| format!("{product}-{t}-v{version}"))
            })
            .collect();
        check_eq!(verbs, expected);
        for (owner, kind, id, version) in [
            ("roll", "action", "list_original_mind", 3),
            ("market", "outcome", "list_original_mind", 3),
            ("market", "action", "list_original_mind", 2),
            ("market", "action", "list_derived_mind", 3),
            ("market", "action", "list_house_mind", 1),
        ] {
            let action = DefinitionRegistryBuilder::new().register(
                OwnerId::new(owner)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new(kind)?,
                    DefinitionId::new(id)?,
                    version,
                    serde_json::json!({"presentation_probe":true}),
                )?,
            )?;
            check!(presentation(&action).is_err());
        }
        Ok(())
    }
}
