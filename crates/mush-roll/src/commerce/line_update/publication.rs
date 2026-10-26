//! Only the checked update producer can request its exact immutable roots.
use super::records::UpdateOperation;
use crate::commerce::{
    TermPhase, TermWindow, WorldIcSeconds,
    records::{FiledRecord, PublicationError},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::action::{PerceivedRef, PrincipalId};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{ClockStamp, CommitToken, DefinitionRef, Flags, ObjId, ObjKind};
#[derive(Clone)]
pub(crate) struct UpdateFence {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    observed: ClockStamp,
    window: TermWindow,
    revision: crate::commerce::RevisionIdentity,
    derived: Option<super::derived::DerivedParent>,
    parent: Option<crate::commerce::parent_grant::CurrentParentGrant>,
}
impl UpdateFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if self.derived.is_some() {
            super::check_derived_action(ctx.action())?;
        } else {
            super::check_action(ctx.action())?;
        }
        let world = ctx.world();
        let now = world.world_ic_now();
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || ctx.invocation().actor() != self.actor
            || ctx.invocation().principal() != &(PrincipalId::Object { id: self.actor })
            || ctx.invocation().subject() != &PerceivedRef::SelfActor
            || !ctx.invocation().targets().is_empty()
            || now.clock != self.observed.clock
            || now.at < self.observed.at
            || world
                .get(self.actor)
                .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        {
            return Err(PublicationError::Mismatch);
        }
        if !matches!(
            self.window
                .phase_at(WorldIcSeconds::from_stamp(&now).map_err(|_| PublicationError::Invalid)?),
            TermPhase::Active | TermPhase::Grace
        ) {
            return Err(PublicationError::Unavailable);
        }
        let roll = world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready()
            .map_err(|_| PublicationError::Unavailable)?;
        let reference = self.revision.revision();
        roll.owned_mind(self.actor, reference.key.mind)
            .map_err(|_| PublicationError::Unavailable)?;
        let revision = roll
            .revision(self.actor, reference.key)
            .map_err(|_| PublicationError::Unavailable)?;
        match (&self.derived, &self.parent) {
            (None, None) => revision
                .require_original_access()
                .map_err(|_| PublicationError::Unavailable)?,
            (Some(evidence), Some(parent)) => {
                parent.check_fresh(self.actor, world)?;
                if revision.provenance.as_ref()
                    != Some(&crate::DerivativeProvenance {
                        parent: evidence.source,
                        charter: evidence.charter,
                        line: evidence.line,
                    })
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            _ => return Err(PublicationError::Mismatch),
        }
        if !self
            .revision
            .matches(revision)
            .map_err(|_| PublicationError::Invalid)?
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
pub(crate) struct UpdateRoots {
    follow: Option<crate::commerce::follow::Change>,
    commit: CommitToken,
    fence: UpdateFence,
    before: Vec<RequiredRoot>,
    after: [RequiredRoot; 2],
}
impl UpdateRoots {
    pub(super) fn new(
        op: &UpdateOperation,
        previous: &FiledRecord,
        window: TermWindow,
    ) -> Result<Self, PublicationError> {
        use crate::commerce::records::Record;
        op.validate()?;
        previous.validate()?;
        if previous.content.id() != op.previous
            || previous.content.position().line != op.line
            || previous.content.position().charter != op.charter
            || previous.content.position().sequence.get().checked_add(1) != Some(op.sequence)
            || previous.content.filed_at().get() > op.at_ic.at
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            commit: op.commit,
            follow: None,
            fence: UpdateFence {
                action: op.action.clone(),
                commit: op.commit,
                actor: op.actor,
                observed: op.at_ic.clone(),
                window,
                revision: op.revision,
                derived: op.parent.clone(),
                parent: None,
            },
            before: vec![
                RequiredRoot {
                    anchor: RootAnchor::Filed(op.previous),
                    identity: previous.content.revision(),
                },
                RequiredRoot {
                    anchor: RootAnchor::Rating {
                        job: op.rating.job(),
                        id: op.rating.id(),
                    },
                    identity: op.revision,
                },
            ],
            after: super::audit::required(op),
        })
    }
    pub(super) fn with_parent(
        mut self,
        evidence: &super::derived::DerivedParent,
        actual: crate::commerce::parent_grant::CurrentParentGrant,
    ) -> Result<Self, PublicationError> {
        if self.fence.derived.as_ref() != Some(evidence)
            || self.fence.parent.is_some()
            || crate::commerce::owner_qualification::records::SelectedParent::capture(
                actual.holding(),
            ) != evidence.selected
            || actual.actor() != self.fence.actor
            || actual.holding().charter() != evidence.charter
            || actual.holding().line() != evidence.line
        {
            return Err(PublicationError::Mismatch);
        }
        self.before.push(RequiredRoot {
            anchor: RootAnchor::Filed(evidence.source.filed),
            identity: evidence.source.revision,
        });
        self.fence.parent = Some(actual);
        Ok(self)
    }
    pub(super) fn with_follow(
        mut self,
        filed: &FiledRecord,
        rating: &crate::commerce::rating::RatingReceipt,
        previous: &FiledRecord,
    ) -> Result<Self, PublicationError> {
        if filed.operation != self.commit
            || self.after[0].anchor != RootAnchor::Filed(filed.content.id())
            || self
                .before
                .first()
                .is_none_or(|root| root.anchor != RootAnchor::Filed(previous.content.id()))
        {
            return Err(PublicationError::Mismatch);
        }
        self.follow = Some(crate::commerce::follow::Change::filing(
            filed,
            rating,
            Some(previous.content.id()),
        )?);
        Ok(self)
    }
    pub(crate) fn follow(&self) -> Option<&crate::commerce::follow::Change> {
        self.follow.as_ref()
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(crate) fn fence(&self) -> &UpdateFence {
        &self.fence
    }
    pub(crate) fn before(&self) -> &[RequiredRoot] {
        &self.before
    }
    pub(crate) fn after(&self) -> &[RequiredRoot] {
        &self.after
    }
}
