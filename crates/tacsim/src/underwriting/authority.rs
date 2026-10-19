//! Actual scoped publication authority retained by the Tacsim metadata owner.
//! Serialized witness fields are historical attribution, never authorization.
use super::{CoverageError, CoverageProductV1, protocol::Protocol};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{
    CommitToken, DefinitionRef, DefinitionRegistry, Flags, ModError, ObjId, ObjKind, OwnerId, World,
};
use mush_org::{
    AuthRequest, CapabilityId, Decision, OrgId, OrgScopeId, OrgStatus, OrgStore, ScopeStatus,
};
use serde::{Deserialize, Serialize};
use std::{
    any::TypeId,
    sync::{Arc, OnceLock},
};

const CAPABILITY: &str = "economy.coverage.publish";
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    tag: String,
    revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CoveragePublicationAuthority {
    at: i64,
    organization_revision: u64,
    scope_revision: u64,
    #[serde(deserialize_with = "sources")]
    sources: Vec<Source>,
}
fn sources<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Source>, D::Error> {
    struct Rows;
    impl<'de> serde::de::Visitor<'de> for Rows {
        type Value = Vec<Source>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 64 publication sources")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut a: A,
        ) -> Result<Self::Value, A::Error> {
            if a.size_hint().is_some_and(|n| n > 64) {
                return Err(serde::de::Error::custom("publication source bound"));
            }
            let mut rows = Vec::new();
            while let Some(row) = a.next_element()? {
                if rows.len() == 64 {
                    return Err(serde::de::Error::custom("publication source bound"));
                }
                rows.push(row);
            }
            Ok(rows)
        }
    }
    d.deserialize_seq(Rows)
}
impl CoveragePublicationAuthority {
    pub fn at(&self) -> i64 {
        self.at
    }
    pub fn validate(&self) -> Result<(), CoverageError> {
        if self.organization_revision == 0
            || self.scope_revision == 0
            || self.sources.is_empty()
            || self.sources.len() > 64
            || self.sources.windows(2).any(|v| v[0] >= v[1])
            || self.sources.iter().any(|s| {
                s.revision == 0
                    || !matches!(s.tag.as_str(), "grant" | "role" | "delegation" | "policy")
            })
        {
            return Err(CoverageError::Authority);
        }
        Ok(())
    }
}
#[derive(Clone, Default)]
pub(super) struct Readiness(Arc<OnceLock<DefinitionRegistry>>);
impl Readiness {
    pub(super) fn capture(&self, ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
        if ctx.module.as_deref() != Some(crate::NS) {
            return Err(ModError::msg("coverage actual Tacsim composition"));
        }
        let registry = ctx
            .reg
            .definitions()
            .ok_or_else(|| ModError::msg("coverage definitions unavailable"))?;
        // Capture actual composition now; Tacsim's final audit may follow Roll.
        // Its live shared latch is checked at every later preparation boundary.
        self.0
            .set(registry.clone())
            .map_err(|_| ModError::msg("coverage composition already captured"))
    }
    pub(super) fn issuer(
        &self,
        world: &World,
        product: &CoverageProductV1,
    ) -> Result<(u64, u64), CoverageError> {
        issuer(
            world,
            self.0.get().ok_or(CoverageError::Authority)?,
            product,
        )
    }
    pub(super) fn references<'a>(
        &self,
        refs: impl IntoIterator<Item = &'a DefinitionRef>,
    ) -> Result<(), CoverageError> {
        let definitions = self.0.get().ok_or(CoverageError::Authority)?;
        for reference in refs {
            definitions
                .get_exact(reference)
                .map_err(|_| CoverageError::Authority)?;
        }
        Ok(())
    }
    pub(super) fn check(&self, action: &DefinitionRef) -> Result<(), CoverageError> {
        let r = self.0.get().ok_or(CoverageError::Authority)?;
        for ns in ["roll", "tacsim", "economy", "org"] {
            r.require_owner_ready(&OwnerId::new(ns).map_err(|_| CoverageError::Authority)?)
                .map_err(|_| CoverageError::Authority)?;
        }
        r.get_exact(action).map_err(|_| CoverageError::Authority)?;
        Ok(())
    }
}
pub(super) fn capture(
    world: &World,
    actor: ObjId,
    org: OrgId,
    scope: OrgScopeId,
) -> Result<CoveragePublicationAuthority, CoverageError> {
    if world
        .get(actor)
        .is_none_or(|p| p.kind != ObjKind::Player || p.flags.contains(Flags::GOING))
        || world
            .module_state_owner(TypeId::of::<OrgStore>())
            .is_none_or(|o| o.as_str() != "org")
    {
        return Err(CoverageError::Authority);
    }
    let state = world
        .module_state::<OrgStore>()
        .ok_or(CoverageError::Authority)?
        .state();
    let organization = state.org(org).ok_or(CoverageError::Authority)?;
    let scoped = state.scope(scope).ok_or(CoverageError::Authority)?;
    if organization.status != OrgStatus::Active
        || scoped.status != ScopeStatus::Active
        || scoped.org != org
    {
        return Err(CoverageError::Authority);
    }
    let Decision::Allow(witness) = mush_org::authorize_in_world(
        world,
        &AuthRequest {
            actor,
            capability: CapabilityId::new(CAPABILITY).ok_or(CoverageError::Authority)?,
            scope,
            now: world.now(),
            amount: None,
        },
    ) else {
        return Err(CoverageError::Authority);
    };
    if witness.at != world.now() || witness.source_revisions.len() > 64 {
        return Err(CoverageError::Authority);
    }
    let mut sources = witness
        .source_revisions
        .into_iter()
        .map(|(tag, revision)| Source {
            tag: tag.to_string(),
            revision,
        })
        .collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    let result = CoveragePublicationAuthority {
        at: witness.at,
        organization_revision: organization.revision,
        scope_revision: scoped.revision,
        sources,
    };
    result.validate()?;
    Ok(result)
}

#[derive(Clone)]
pub(super) struct PublicationFence {
    readiness: Readiness,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    org: OrgId,
    scope: OrgScopeId,
    captured: CoveragePublicationAuthority,
}
impl PublicationFence {
    pub(super) fn new(
        readiness: Readiness,
        protocol: &Protocol,
        world: &World,
        commit: CommitToken,
        actor: ObjId,
        org: OrgId,
        scope: OrgScopeId,
    ) -> Result<Self, CoverageError> {
        let action = protocol.action(0)?;
        readiness.check(&action)?;
        let captured = capture(world, actor, org, scope)?;
        Ok(Self {
            readiness,
            action,
            commit,
            actor,
            org,
            scope,
            captured,
        })
    }
    pub(super) fn authority(&self) -> &CoveragePublicationAuthority {
        &self.captured
    }
    pub(super) fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(super) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), CoverageError> {
        self.readiness.check(&self.action)?;
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || ctx.invocation().actor() != self.actor
            || ctx.invocation().principal()
                != &(mush_core::action::PrincipalId::Object { id: self.actor })
        {
            return Err(CoverageError::Authority);
        }
        let actual = capture(ctx.world(), self.actor, self.org, self.scope)?;
        if actual.at < self.captured.at
            || actual.organization_revision != self.captured.organization_revision
            || actual.scope_revision != self.captured.scope_revision
            || actual.sources != self.captured.sources
        {
            return Err(CoverageError::Authority);
        }
        Ok(())
    }
}

pub(super) use CoveragePublicationAuthority as PublicationAuthority;
pub(super) fn player(world: &World, actor: ObjId) -> Result<(), CoverageError> {
    if world
        .get(actor)
        .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
    {
        Err(CoverageError::Authority)
    } else {
        Ok(())
    }
}
pub(super) fn ready(definitions: &DefinitionRegistry) -> Result<(), CoverageError> {
    for ns in ["tacsim", "roll", "org", "economy"] {
        definitions
            .require_owner_ready(&OwnerId::new(ns).map_err(|_| CoverageError::Authority)?)
            .map_err(|_| CoverageError::Authority)?;
    }
    Ok(())
}
pub(super) fn issuer(
    world: &World,
    definitions: &DefinitionRegistry,
    product: &CoverageProductV1,
) -> Result<(u64, u64), CoverageError> {
    ready(definitions)?;
    if world
        .module_state_owner(TypeId::of::<OrgStore>())
        .is_none_or(|o| o.as_str() != "org")
    {
        return Err(CoverageError::Authority);
    }
    let state = world
        .module_state::<OrgStore>()
        .ok_or(CoverageError::Authority)?
        .state();
    let org = state
        .org(product.insurer())
        .ok_or(CoverageError::Authority)?;
    let scope = state
        .scope(product.scope())
        .ok_or(CoverageError::Authority)?;
    if org.status != OrgStatus::Active || scope.status != ScopeStatus::Active || scope.org != org.id
    {
        return Err(CoverageError::Authority);
    }
    Ok((org.revision, scope.revision))
}
