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
            .add_definition(
                config
                    .policy_ref_for_protocol(
                        mush_roll::commerce::house_purchase::HouseProtocol::from_format(
                            u16::try_from(action.version()).map_err(|_| bad())?,
                        )
                        .map_err(|_| bad())?,
                    )
                    .map_err(|_| bad())?,
                defs,
            )
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

// Presentation is selected only after the provider has matched its exact full
// registered action above. It grants no authority and never changes the action,
// its parameters, guard set or durable consent. Distinct verbs let the gateway
// retain its strict single-match selection when both products are registered.
fn presentation(action: &DefinitionRef) -> Result<mush_core::ActionPresentation, String> {
    if action.key().owner.as_str() != crate::NS || action.key().kind.as_str() != "action" {
        return Err("unknown house presentation".into());
    }
    let (capacity, label_capacity) = match action.version() {
        1 => ("one", "one-seat"),
        2 => ("two", "two-seat"),
        _ => return Err("unknown house presentation".into()),
    };
    let (transition, label, help) = match action.key().id.as_str() {
        "list_house_mind" => (
            "list",
            "List",
            "Publish current scoped house terms using an owner licence.",
        ),
        "quote_house_mind" => (
            "quote",
            "Quote",
            "Read and freeze current terms for a listed house licence.",
        ),
        "purchase_house_mind" => (
            "fund",
            "Fund",
            "Place the quoted price in escrow; this does not yet grant a licence.",
        ),
        "settle_house_mind" => (
            "grant",
            "Complete",
            "Settle an exact funded purchase and grant its licence.",
        ),
        "refund_house_mind" => (
            "refund",
            "Refund",
            "Refund an exact funded purchase without requiring fresh issuer eligibility.",
        ),
        _ => return Err("unknown house presentation".into()),
    };
    mush_core::ActionPresentation::new(
        format!("house-{transition}-{capacity}-seat"),
        format!("{label} a {label_capacity} house licence"),
        help,
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

    // Structural presentation only. A descriptor built here is not a registered
    // purchase provider or a grant. The genuine House flow selects actual offers
    // by these public verbs and retains all normal signed resolution/ACK checks.
    #[test]
    fn house_transitions_and_protocols_have_unique_line_selectors()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut verbs = BTreeSet::new();
        for version in [1, 2] {
            for id in [
                "list_house_mind",
                "quote_house_mind",
                "purchase_house_mind",
                "settle_house_mind",
                "refund_house_mind",
            ] {
                let action = DefinitionRegistryBuilder::new().register(
                    OwnerId::new(crate::NS)?,
                    DefinitionDescriptor::new(
                        DefinitionKind::new("action")?,
                        DefinitionId::new(id)?,
                        version,
                        serde_json::json!({"presentation_probe":true}),
                    )?,
                )?;
                let p = presentation(&action)?;
                check!(!p.verb.bytes().any(|b| b.is_ascii_whitespace()));
                check!(verbs.insert(p.verb), "ambiguous House line selector");
            }
        }
        let expected: BTreeSet<_> = ["list", "quote", "fund", "grant", "refund"]
            .into_iter()
            .flat_map(|t| {
                ["one", "two"]
                    .into_iter()
                    .map(move |capacity| format!("house-{t}-{capacity}-seat"))
            })
            .collect();
        check_eq!(verbs, expected);
        for (owner, kind, id, version) in [
            ("roll", "action", "list_house_mind", 1),
            ("market", "outcome", "list_house_mind", 1),
            ("market", "action", "settle_original_mind", 1),
            ("market", "action", "list_house_mind", 3),
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
