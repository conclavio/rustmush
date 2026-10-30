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
const MISSION_KIND: &str = "mission_grant_lineage_v1";

/// Only the Org half of mission admission: the selected actual role grant must
/// retain current title/loan/delegation/policy backing for this actor and unit.
/// The mission owner separately fences the exact accepted binding and leader.
pub fn mission_revision_key(
    actor: ObjId,
    unit: ObjId,
    grant: super::UnitGrantId,
) -> Result<RevisionKey, ActionError> {
    if actor.0 == 0 || unit.0 == 0 || grant.0 == 0 {
        return Err(super::runtime::error("invalid mission lineage coordinate"));
    }
    Ok(RevisionKey {
        owner: OwnerId::new(crate::NS)
            .map_err(|_| super::runtime::error("invalid mission owner"))?,
        kind: RevisionKind::new(
            CanonicalId::new(MISSION_KIND)
                .map_err(|_| super::runtime::error("invalid mission kind"))?,
        ),
        subject: SubjectKey::Object { id: actor },
        facet: Some(
            CanonicalId::new(format!("{}.{}", unit.0, grant.0))
                .map_err(|_| super::runtime::error("invalid mission facet"))?,
        ),
    })
}
fn parse_mission(key: &RevisionKey) -> Option<(ObjId, ObjId, super::UnitGrantId)> {
    let SubjectKey::Object { id: actor } = key.subject else {
        return None;
    };
    let (unit, grant) = key.facet.as_ref()?.as_str().split_once('.')?;
    let coordinate = (
        actor,
        ObjId(unit.parse().ok()?),
        super::UnitGrantId(grant.parse().ok()?),
    );
    (mission_revision_key(coordinate.0, coordinate.1, coordinate.2)
        .ok()
        .as_ref()
        == Some(key))
    .then_some(coordinate)
}
pub(super) fn mission_kind() -> Result<RevisionKind, mush_core::ModError> {
    Ok(RevisionKind::new(
        CanonicalId::new(MISSION_KIND).map_err(|e| mush_core::ModError::msg(e.to_string()))?,
    ))
}

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
        let direct = parse(key);
        let mission = parse_mission(key);
        if direct.is_none() && mission.is_none() {
            return Ok(None);
        }
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
        if let Some(coordinate) = mission {
            return Ok(super::resolve::mission_lineage(
                store.state(),
                &registry.grants,
                facts.as_ref(),
                &predicates,
                coordinate,
                context.world().now(),
            )
            .is_ok()
            .then_some(1));
        }
        let Some((actor, unit, capability)) = direct else {
            return Ok(None);
        };
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
