//! Free, explicitly requested personal owner-generation transition. No sale,
//! price, paid term, source ancestry or previously captured claim is rewritten.
pub(crate) mod history;
pub(crate) mod protocol;
mod provider;
pub(crate) mod records;
#[cfg(test)]
mod tests;
pub(crate) use provider::register;

use super::{
    CharterId, LicenceId, TermPhase, TermWindow, WorldIcSeconds,
    parent_grant::CurrentParentGrant,
    records::PublicationError,
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{ClockStamp, CommitToken, DefinitionRef, ObjId};

pub(crate) struct Attestation {
    operation: records::AttestationOperation,
    fence: AttestationFence,
}
impl Attestation {
    pub(crate) fn commit(&self) -> CommitToken {
        self.operation.operation
    }
    pub(crate) fn fence(&self) -> &AttestationFence {
        &self.fence
    }
    pub(crate) fn roots(&self) -> Result<Vec<RequiredRoot>, PublicationError> {
        self.operation.roots()
    }
    pub(crate) fn existing_roots(&self) -> Vec<RequiredRoot> {
        let mut roots = vec![RequiredRoot {
            anchor: RootAnchor::Filed(self.operation.filed),
            identity: self.operation.revision,
        }];
        if let Some(p) = self
            .operation
            .next
            .attestation
            .as_ref()
            .and_then(|e| e.parent.as_ref())
        {
            roots.push(RequiredRoot {
                anchor: RootAnchor::Filed(p.source.filed),
                identity: p.source.revision,
            });
            roots.push(RequiredRoot {
                anchor: RootAnchor::Filed(p.selected.filed),
                identity: p.selected.revision,
            });
        }
        roots
    }
}
#[derive(Clone)]
pub(crate) struct AttestationFence {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    licence: LicenceId,
    charter: CharterId,
    title_revision: u64,
    observed: ClockStamp,
    utc: i64,
    predecessor: TermWindow,
    next: TermWindow,
    parent: Option<CurrentParentGrant>,
    readiness: super::withdrawal::EconomyReadiness,
}
impl AttestationFence {
    pub(crate) fn check(
        &self,
        ctx: &mush_core::participant::OwnerPrepareContext<'_>,
    ) -> Result<(), PublicationError> {
        self.readiness.check()?;
        let request = provider::Request::parse(ctx.invocation())?;
        let world = ctx.world();
        let now = world.world_ic_now();
        let at = WorldIcSeconds::from_stamp(&now).map_err(|_| PublicationError::Invalid)?;
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || ctx.invocation().actor() != self.actor
            || request.licence()? != self.licence
            || super::owner_qualification::current_title(world, self.actor, self.charter)?
                != self.title_revision
            || now.clock != self.observed.clock
            || now.at < self.observed.at
            || world.now() < self.utc
            || at < self.predecessor.term_end()
            || !matches!(self.next.phase_at(at), TermPhase::Active | TermPhase::Grace)
        {
            return Err(PublicationError::Stale);
        }
        if let Some(parent) = &self.parent {
            parent.check_fresh(self.actor, world)?;
        }
        Ok(())
    }
}
