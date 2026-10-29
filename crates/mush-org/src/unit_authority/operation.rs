//! Current unit-use authority for selected coordinator actions. No stored permit.
use super::families::UnitGrantRegistry;
use super::{UnitAction, UnitAuthRequest, UnitAuthorityWorldFacts, UnitCapability, authorize_unit};
use crate::wiring::WorldPredicates;
use mush_core::{
    ActionError, CanonicalId, ObjId, OwnerId, RevisionKey, RevisionKind, RevisionValidator,
    RevisionValidatorContext, SubjectKey,
};
use std::sync::Arc;

const KIND: &str = "unit_use_authority_v1";

/// A provider names the real operator and unit; Org evaluates its own rights.
/// This coordinate checks exercise, never permission to delegate that right.
pub fn revision_key(
    actor: ObjId,
    unit: ObjId,
    capability: UnitCapability,
) -> Result<RevisionKey, ActionError> {
    if actor.0 == 0 || unit.0 == 0 {
        return Err(super::runtime::error("invalid unit-use coordinate"));
    }
    Ok(RevisionKey {
        owner: OwnerId::new(crate::NS)
            .map_err(|_| super::runtime::error("invalid unit-use owner"))?,
        kind: RevisionKind::new(
            CanonicalId::new(KIND).map_err(|_| super::runtime::error("invalid unit-use kind"))?,
        ),
        subject: SubjectKey::Object { id: actor },
        facet: Some(
            CanonicalId::new(format!("{}.{}", unit.0, capability.as_str()))
                .map_err(|_| super::runtime::error("invalid unit-use facet"))?,
        ),
    })
}
fn parse(key: &RevisionKey) -> Option<(ObjId, ObjId, UnitCapability)> {
    let SubjectKey::Object { id: actor } = key.subject else {
        return None;
    };
    let (unit, capability) = key.facet.as_ref()?.as_str().split_once('.')?;
    let unit = ObjId(unit.parse().ok()?);
    let capability = match capability {
        "unit.operate_crew" => UnitCapability::OperateCrew,
        "unit.deploy" => UnitCapability::Deploy,
        "unit.command_assigned_ai" => UnitCapability::CommandAssignedAi,
        "unit.recover_load" => UnitCapability::RecoverLoad,
        "unit.maintain_refit" => UnitCapability::MaintainRefit,
        "unit.transfer_abandon_title" => UnitCapability::TransferAbandonTitle,
        _ => return None,
    };
    (revision_key(actor, unit, capability).ok().as_ref() == Some(key))
        .then_some((actor, unit, capability))
}
pub(super) fn kind() -> Result<RevisionKind, mush_core::ModError> {
    Ok(RevisionKind::new(
        CanonicalId::new(KIND).map_err(|e| mush_core::ModError::msg(e.to_string()))?,
    ))
}
pub(super) struct Validator {
    pub(super) registry: mush_core::StateFamilyHandle<(), UnitGrantRegistry>,
    pub(super) facts: Arc<dyn UnitAuthorityWorldFacts>,
}
impl RevisionValidator for Validator {
    fn current_generation(
        &self,
        context: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let Some((actor, unit, capability)) = parse(key) else {
            return Ok(None);
        };
        let empty = UnitGrantRegistry::default();
        let registry = context
            .action_state(&self.registry, &())
            .map_err(|_| super::runtime::error("unit-use grants unavailable"))?
            .unwrap_or(&empty);
        registry
            .validate()
            .map_err(|_| super::runtime::error("unit-use grants invalid"))?;
        let store = context
            .world()
            .module_state::<crate::OrgStore>()
            .ok_or_else(|| super::runtime::error("unit-use owner unavailable"))?;
        let predicates = WorldPredicates {
            w: context.world(),
            providers: store.providers(),
        };
        let facts = self.facts.facts(context.world());
        let request = UnitAuthRequest {
            actor,
            unit,
            action: UnitAction::Exercise(capability),
            now: context.world().now(),
            mission: None,
        };
        // The coordinate exists only while this actual right is exercisable.
        // Revocation, expiry, title changes and current denies are re-evaluated
        // at resolution and final take by the ordinary guard machinery.
        Ok(authorize_unit(
            store.state(),
            &registry.grants,
            facts.as_ref(),
            &predicates,
            &request,
        )
        .is_allow()
        .then_some(1))
    }
}
