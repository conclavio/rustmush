//! Actual scoped authorization, repeated before custody. Witnesses are attribution.
use super::*;
use mush_core::{ClockStamp, Flags, World};
use mush_org::{AuthRequest, CapabilityId, Decision, OrgStatus, OrgStore, ScopeStatus};
use serde::Deserializer;
use std::{
    any::TypeId,
    sync::{Arc, OnceLock},
};

#[derive(Clone, Default)]
pub(crate) struct Readiness(Arc<OnceLock<DefinitionRegistry>>);
impl Readiness {
    pub(crate) fn capture(&self, ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
        let defs = ctx
            .reg
            .definitions()
            .ok_or_else(|| error("recall definitions unavailable"))?;
        for ns in [crate::NS, "org"] {
            defs.require_owner_ready(&OwnerId::new(ns).map_err(error)?)
                .map_err(error)?;
        }
        self.0
            .set(defs.clone())
            .map_err(|_| error("recall readiness already captured"))
    }
    pub(crate) fn check(&self, policy: &RegistryRecallPolicy) -> Result<(), PublicationError> {
        let defs = self.0.get().ok_or(PublicationError::Unavailable)?;
        for ns in [crate::NS, "org"] {
            defs.require_owner_ready(&OwnerId::new(ns).map_err(bad)?)
                .map_err(bad)?;
        }
        defs.get_exact(&policy.action().map_err(bad)?)
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Authorization {
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
    fn same_sources(&self, old: &Self) -> bool {
        self.at >= old.at
            && self.organization_revision == old.organization_revision
            && self.scope_revision == old.scope_revision
            && self.sources == old.sources
    }
}
pub(crate) fn observe(
    world: &World,
    readiness: &Readiness,
    p: &RegistryRecallPolicy,
    actor: ObjId,
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
            capability: CapabilityId::new(CAPABILITY).ok_or(PublicationError::Invalid)?,
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
pub(crate) fn recheck(
    world: &World,
    readiness: &Readiness,
    p: &RegistryRecallPolicy,
    actor: ObjId,
    before: &Authorization,
    stamp: &ClockStamp,
) -> Result<(), PublicationError> {
    let (facts, now) = observe(world, readiness, p, actor)?;
    if !facts.same_sources(before) || now.clock != stamp.clock || now.at < stamp.at {
        return Err(PublicationError::Stale);
    }
    Ok(())
}
