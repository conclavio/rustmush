//! Actual IC host facts and the separate, live backing of a mission-role grant.
use super::{families::UnitGrantRegistry, record::*, runtime::UnitAuthorityWorldFacts};
use crate::{OrgStore, wiring::WorldPredicates};
use mush_core::{ObjId, World, read_service::OwnerReadError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionAuthority {
    pub allowed: bool,
    /// An existing Org row, not an authority token. The caller fences its live
    /// lineage separately from the actual mission-owner eligibility facts.
    pub role_grant: Option<UnitGrantId>,
}
struct Facts<'a> {
    base: &'a dyn UnitAuthorityFacts,
    mission: &'a MissionFacts,
}
impl UnitAuthorityFacts for Facts<'_> {
    fn principal_exists(&self, a: ObjId) -> bool {
        self.base.principal_exists(a)
    }
    fn asset(&self, u: ObjId) -> Option<UnitAssetFacts> {
        self.base.asset(u)
    }
    fn equipment_lease(&self, o: crate::OrgId, lease: &UnitLeaseBinding) -> bool {
        self.base.equipment_lease(o, lease)
    }
    fn organization_control(
        &self,
        o: crate::OrgId,
        u: ObjId,
        c: UnitCapability,
        n: i64,
    ) -> Option<OrgControlFacts> {
        self.base.organization_control(o, u, c, n)
    }
    fn recovery_custody(&self, u: ObjId, a: ObjId) -> Option<RecoveryCustodyFacts> {
        self.base.recovery_custody(u, a)
    }
    fn mission(&self, b: &MissionBinding) -> Option<MissionFacts> {
        (b == &self.mission.binding).then(|| self.mission.clone())
    }
}
pub(super) fn commands(
    world: &World,
    store: &OrgStore,
    factory: &dyn UnitAuthorityWorldFacts,
    registry: &UnitGrantRegistry,
    mission: &MissionFacts,
    crew: &[(ObjId, ObjId)],
) -> Result<Vec<MissionAuthority>, OwnerReadError> {
    if mission.binding.validate().is_err()
        || mission.units.is_empty()
        || mission.units.len() > 6
        || mission.units.iter().any(|u| u.0 == 0)
        || crew.len() > 6
        || crew
            .iter()
            .any(|(u, a)| u.0 == 0 || a.0 == 0 || !mission.units.contains(u))
        || mission.leader.is_some_and(|l| {
            let kind = match l {
                MissionLeader::Player(_) => mush_core::ObjKind::Player,
                MissionLeader::Npc(_) => mush_core::ObjKind::Npc,
            };
            l.character().0 == 0
                || mission.term == 0
                || world
                    .get(l.character())
                    .is_none_or(|o| o.kind != kind || o.flags.contains(mush_core::Flags::GOING))
        })
    {
        return Err(OwnerReadError::Refused);
    }
    let base = factory.facts(world);
    let facts = Facts {
        base: base.as_ref(),
        mission,
    };
    let predicates = WorldPredicates {
        w: world,
        providers: store.providers(),
    };
    Ok(crew
        .iter()
        .map(|(unit, actor)| {
            let decision = super::authorize_unit(
                store.state(),
                &registry.grants,
                &facts,
                &predicates,
                &UnitAuthRequest {
                    actor: *actor,
                    unit: *unit,
                    action: UnitAction::Exercise(UnitCapability::CommandAssignedAi),
                    now: world.now(),
                    mission: Some(MissionCommand {
                        binding: mission.binding.clone(),
                        term: mission.term,
                    }),
                },
            );
            let role_grant = if let UnitDecision::Allow(witness) = &decision {
                witness.sources.iter().rev().find_map(|s| match s {
                    UnitAuthoritySource::Grant { id, .. }
                        if registry.grants.get(id).is_some_and(|g| {
                            g.grantee == UnitGrantee::CurrentLeader(mission.binding.clone())
                        }) =>
                    {
                        Some(*id)
                    }
                    _ => None,
                })
            } else {
                None
            };
            MissionAuthority {
                allowed: decision.is_allow(),
                role_grant,
            }
        })
        .collect())
}
