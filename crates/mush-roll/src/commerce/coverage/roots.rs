//! Retained Root participant fences are private. Neither a data record nor an
//! arbitrary client can fabricate this request or substitute a native output.
use super::{CoverageOpenFacts, CoverageRetention};
use crate::{
    RollStore,
    commerce::{
        ic_claims::ClaimRecord,
        records::{PublicationError, Record},
        roots::RequiredRoot,
    },
};
use mush_core::{
    ClockStamp, CommitToken, DefinitionRef, Flags, ObjId, ObjKind, action::PrincipalId,
    participant::OwnerPrepareContext,
};

/// A refusal-only check retained by the real Roll participant. It grants no
/// owner output, source root or typed write; the registered Tacsim provider
/// supplies its private host/issuer check alongside the mandatory native proof.
pub trait CoveragePrecommitCheck: Send + Sync {
    fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError>;
}

#[derive(Clone)]
pub(crate) struct CoverageFence {
    inner: FenceKind,
}
#[derive(Clone)]
enum FenceKind {
    Open(Box<OpenFence>),
    Publication(super::publication::PublicationFence),
}
impl CoverageFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        match &self.inner {
            FenceKind::Open(f) => f.check(ctx),
            FenceKind::Publication(f) => f.check(ctx),
        }
    }
    fn commit(&self) -> CommitToken {
        match &self.inner {
            FenceKind::Open(f) => f.commit,
            FenceKind::Publication(f) => f.operation(),
        }
    }
    fn open(fence: OpenFence) -> Self {
        Self {
            inner: FenceKind::Open(Box::new(fence)),
        }
    }
}
#[derive(Clone)]
struct OpenFence {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    observed: ClockStamp,
    utc: i64,
    fresh: Option<CoverageOpenFacts>,
    claim: ClaimRecord,
    extra: std::sync::Arc<dyn CoveragePrecommitCheck>,
}
impl OpenFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        self.extra.check(ctx)?;
        let world = ctx.world();
        let now = world.world_ic_now();
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || ctx.invocation().actor() != self.actor
            || ctx.invocation().principal() != &(PrincipalId::Object { id: self.actor })
            || now.clock != self.observed.clock
            || now.at < self.observed.at
            || world.now() < self.utc
            || world
                .get(self.actor)
                .is_none_or(|p| p.kind != ObjKind::Player || p.flags.contains(Flags::GOING))
            || world
                .module_state_owner(std::any::TypeId::of::<RollStore>())
                .is_none_or(|v| v.as_str() != crate::NS)
        {
            return Err(PublicationError::Stale);
        }
        if let Some(facts) = &self.fresh {
            facts.check_fresh(ctx)?;
        }
        let roll = world
            .module_state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        if roll
            .coverage_ic_claim(self.claim.open.operation)
            .map_err(|_| PublicationError::Unavailable)?
            != &self.claim
        {
            return Err(PublicationError::Stale);
        }
        roll.check_required_root(&crate::commerce::ic_claims::roots::required(&self.claim)?)
            .map_err(|_| PublicationError::Mismatch)?;
        Ok(())
    }
}
pub(crate) struct CoverageRoots {
    fence: CoverageFence,
    before: Vec<RequiredRoot>,
    after: Vec<RequiredRoot>,
}
impl CoverageRoots {
    pub(super) fn quote(
        action: DefinitionRef,
        commit: CommitToken,
        utc: i64,
        facts: CoverageOpenFacts,
        extra: std::sync::Arc<dyn CoveragePrecommitCheck>,
    ) -> Result<Self, PublicationError> {
        let root = facts.root()?;
        let fence = CoverageFence::open(OpenFence {
            action,
            commit,
            actor: facts.actor(),
            observed: facts.observed_ic().clone(),
            utc,
            claim: facts.claim().clone(),
            fresh: Some(facts),
            extra,
        });
        Ok(Self {
            fence,
            before: vec![root.clone()],
            after: vec![root],
        })
    }
    pub(super) fn bind(
        row: &CoverageRetention,
        facts: CoverageOpenFacts,
        extra: std::sync::Arc<dyn CoveragePrecommitCheck>,
    ) -> Result<Self, PublicationError> {
        row.validate()?;
        if row.terminal().is_some() {
            return Err(PublicationError::Mismatch);
        }
        row.matches_claim(facts.claim())?;
        let c = row.consent();
        Ok(Self {
            before: vec![facts.root()?],
            after: vec![row.root()?],
            fence: CoverageFence::open(OpenFence {
                action: c.effect.clone(),
                commit: c.operation,
                actor: c.terms.buyer,
                observed: c.terms.starts_at.clone(),
                utc: c.accepted_at_utc,
                claim: facts.claim().clone(),
                fresh: Some(facts),
                extra,
            }),
        })
    }
    pub(super) fn terminal(
        before: &CoverageRetention,
        after: &CoverageRetention,
        claim: ClaimRecord,
        extra: std::sync::Arc<dyn CoveragePrecommitCheck>,
    ) -> Result<Self, PublicationError> {
        before.validate()?;
        after.matches_claim(&claim)?;
        if before.terminal().is_some()
            || before.consent() != after.consent()
            || before.root()?.anchor != after.root()?.anchor
            || before.root()?.identity != after.root()?.identity
        {
            return Err(PublicationError::Mismatch);
        }
        let terminal = after.terminal().ok_or(PublicationError::Mismatch)?;
        Ok(Self {
            before: vec![before.root()?],
            after: vec![after.root()?],
            fence: CoverageFence::open(OpenFence {
                action: terminal.action.clone(),
                commit: terminal.operation,
                actor: terminal.actor,
                observed: terminal.at_ic.clone(),
                utc: terminal.posted_at_utc,
                claim,
                fresh: None,
                extra,
            }),
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.fence.commit()
    }
    pub(super) fn publication(
        row: &super::publication::CoveragePublicationRetention,
        fence: super::publication::PublicationFence,
        proofs: &[crate::commerce::conduct::ConductTrialProof],
    ) -> Result<Self, PublicationError> {
        Ok(Self {
            fence: CoverageFence {
                inner: FenceKind::Publication(fence),
            },
            before: proofs.iter().map(|p| p.root()).collect::<Result<_, _>>()?,
            after: row.roots(),
        })
    }
    pub(crate) fn fence(&self) -> &CoverageFence {
        &self.fence
    }
    pub(crate) fn before(&self) -> &[RequiredRoot] {
        &self.before
    }
    pub(crate) fn after(&self) -> &[RequiredRoot] {
        &self.after
    }
}
