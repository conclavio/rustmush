//! Actual scoped authorization, observed on every invocation. Witnesses are attribution.
use super::*;
use mush_core::{ClockStamp, Flags, World};
use mush_org::{AuthRequest, CapabilityId, Decision, OrgStatus, OrgStore, ScopeStatus};
use serde::Deserializer;
use std::{
    any::TypeId,
    sync::{Arc, OnceLock},
};

/// The definition registry captured once the Roll, Org and Economy owners are ready, so an
/// inspection can check its action is the registered one (the revocation twin's
/// `Readiness`). Captured by tacsim's boot hook, which runs after the Roll's.
#[derive(Clone, Default)]
pub(crate) struct Readiness(Arc<OnceLock<DefinitionRegistry>>);
impl Readiness {
    pub(crate) fn capture(&self, ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
        let defs = ctx
            .reg
            .definitions()
            .ok_or_else(|| error("inspection definitions unavailable"))?;
        for ns in [crate::NS, "org", "economy"] {
            defs.require_owner_ready(&OwnerId::new(ns).map_err(error)?)
                .map_err(error)?;
        }
        self.0
            .set(defs.clone())
            .map_err(|_| error("inspection readiness already captured"))
    }
    pub(crate) fn check(&self, policy: &RegistryInspectionPolicy) -> Result<(), PublicationError> {
        let defs = self.0.get().ok_or(PublicationError::Unavailable)?;
        for ns in [crate::NS, "org", "economy"] {
            defs.require_owner_ready(&OwnerId::new(ns).map_err(bad)?)
                .map_err(bad)?;
        }
        defs.get_exact(&policy.action().map_err(bad)?)
            .map_err(bad)?;
        defs.get_exact(&policy.authority().map_err(bad)?)
            .map_err(bad)?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub(crate) tag: String,
    pub(crate) revision: u64,
}
/// The Org authorization an inspection was observed under: the witness's instant, the
/// organization's and scope's revisions and the grant sources, as the revocation records it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub(crate) at: i64,
    pub(crate) organization_revision: u64,
    pub(crate) scope_revision: u64,
    #[serde(deserialize_with = "sources")]
    pub(crate) sources: Vec<Source>,
}
fn sources<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Source>, D::Error> {
    policy::bounded::<D, Source, 64>(d)
}
impl Authorization {
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        if self.organization_revision == 0
            || self.scope_revision == 0
            || self.sources.is_empty()
            || self.sources.len() > 64
            || self
                .sources
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a >= b))
            || self.sources.iter().any(|s| {
                s.revision == 0
                    || !matches!(s.tag.as_str(), "grant" | "role" | "delegation" | "policy")
            })
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}
pub(crate) fn observe(
    world: &World,
    readiness: &Readiness,
    p: &RegistryInspectionPolicy,
    actor: ObjId,
) -> Result<(Authorization, ClockStamp), PublicationError> {
    observe_capability(world, readiness, p, actor, CAPABILITY)
}
/// **The fine's twin** (doc 17h cut M8d D3): the same live observation for
/// [`REGISTRY_FINE_CAPABILITY`] — an operator the inspection grant admits is not thereby a
/// fining operator.
pub(crate) fn observe_fine(
    world: &World,
    readiness: &Readiness,
    p: &RegistryInspectionPolicy,
    actor: ObjId,
) -> Result<(Authorization, ClockStamp), PublicationError> {
    observe_capability(world, readiness, p, actor, REGISTRY_FINE_CAPABILITY)
}
/// **The seizure's twin** (doc 17h cut M8d2 S4): the live observation for
/// [`super::REGISTRY_SEIZE_CAPABILITY`].
pub(crate) fn observe_seize(
    world: &World,
    readiness: &Readiness,
    p: &RegistryInspectionPolicy,
    actor: ObjId,
) -> Result<(Authorization, ClockStamp), PublicationError> {
    observe_capability(world, readiness, p, actor, super::REGISTRY_SEIZE_CAPABILITY)
}
/// **The forfeit's twin** (doc 17h cut M8d2 S5): the live observation for
/// [`super::REGISTRY_FORFEIT_CAPABILITY`].
pub(crate) fn observe_forfeit(
    world: &World,
    readiness: &Readiness,
    p: &RegistryInspectionPolicy,
    actor: ObjId,
) -> Result<(Authorization, ClockStamp), PublicationError> {
    observe_capability(
        world,
        readiness,
        p,
        actor,
        super::REGISTRY_FORFEIT_CAPABILITY,
    )
}
/// **The principal revocation's twin** (doc 17h cut M8d D4): the same live observation for
/// the existing `roll.registry.revoke` capability.
pub(crate) fn observe_revoke(
    world: &World,
    readiness: &Readiness,
    p: &RegistryInspectionPolicy,
    actor: ObjId,
) -> Result<(Authorization, ClockStamp), PublicationError> {
    observe_capability(
        world,
        readiness,
        p,
        actor,
        super::super::registry_revocation::REGISTRY_REVOCATION_CAPABILITY,
    )
}
fn observe_capability(
    world: &World,
    readiness: &Readiness,
    p: &RegistryInspectionPolicy,
    actor: ObjId,
    capability: &str,
) -> Result<(Authorization, ClockStamp), PublicationError> {
    p.validate()?;
    readiness.check(p)?;
    if p.operators.binary_search(&actor).is_err()
        || world
            .get(actor)
            .is_none_or(|o| o.flags.contains(Flags::GOING))
        || world
            .module_state_owner(TypeId::of::<OrgStore>())
            .is_none_or(|v| v.as_str() != "org")
        || world
            .module_state_owner(TypeId::of::<mush_economy::EconomyStore>())
            .is_none_or(|v| v.as_str() != "economy")
        || world
            .module_state_owner(TypeId::of::<crate::RollStore>())
            .is_none_or(|v| v.as_str() != crate::NS)
    {
        return Err(PublicationError::Unavailable);
    }
    let state = world
        .module_state::<OrgStore>()
        .ok_or(PublicationError::Unavailable)?
        .state();
    let org = state
        .org(p.organization)
        .ok_or(PublicationError::Unavailable)?;
    let scope = state.scope(p.scope).ok_or(PublicationError::Unavailable)?;
    if org.status != OrgStatus::Active
        || scope.status != ScopeStatus::Active
        || scope.org != p.organization
    {
        return Err(PublicationError::Unavailable);
    }
    let Decision::Allow(witness) = mush_org::authorize_in_world(
        world,
        &AuthRequest {
            actor,
            capability: CapabilityId::new(capability).ok_or(PublicationError::Invalid)?,
            scope: p.scope,
            now: world.now(),
            amount: None,
        },
    ) else {
        return Err(PublicationError::Unavailable);
    };
    if witness.at != world.now() || witness.source_revisions.len() > 64 {
        return Err(PublicationError::Capacity);
    }
    // Org currently emits generic kind/revision tags, not unique source IDs.
    // Their equality is only a staleness hint; live authorization above is decisive.
    let mut sources: Vec<_> = witness
        .source_revisions
        .into_iter()
        .map(|(tag, revision)| Source {
            tag: tag.to_string(),
            revision,
        })
        .collect();
    sources.sort();
    sources.dedup();
    let facts = Authorization {
        at: witness.at,
        organization_revision: org.revision,
        scope_revision: scope.revision,
        sources,
    };
    facts.validate()?;
    let stamp = world.world_ic_now();
    super::super::WorldIcSeconds::from_stamp(&stamp).map_err(bad)?;
    Ok((facts, stamp))
}
