//! One permanent source obligation is allocated at admission. Progress and
//! terminal publication reuse its exact bytes, so worker completion cannot
//! require a new legacy retention slot after quota has been charged.
use super::{history::job_root, protocol::ConductHistoryRow, records::*, state::Native, *};
use crate::{RollStore, commerce::roots::RequiredRoot};
use mush_core::{
    CommitToken, DefinitionRef, Flags, GOD, ObjKind,
    action::{PerceivedRef, PrincipalId},
    participant::OwnerPrepareContext,
};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Fence {
    action: DefinitionRef,
    commit: CommitToken,
    stamp: (u64, u64),
    before: Option<(ConductJob, ConductExecution)>,
    native: Arc<dyn Native>,
}
impl Fence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || ctx.invocation().actor() != GOD
            || ctx.invocation().principal() != &(PrincipalId::Object { id: GOD })
            || ctx.invocation().subject() != &PerceivedRef::SelfActor
            || !ctx.invocation().targets().is_empty()
            || ctx
                .world()
                .get(GOD)
                .is_none_or(|p| p.kind != ObjKind::Player || p.flags.contains(Flags::GOING))
        {
            return Err(PublicationError::Stale);
        }
        let roll = ctx
            .world()
            .module_state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        if roll.conduct_stamp().map_err(invalid)? != self.stamp {
            return Err(PublicationError::Stale);
        }
        if let Some((job, execution)) = &self.before {
            self.native.check(
                ctx.world(),
                ConductHistoryRow {
                    job,
                    execution,
                    result: None,
                },
            )?;
            roll.check_required_root(&job_root(job)?).map_err(invalid)?;
        }
        Ok(())
    }
}
pub(crate) struct Roots {
    fence: Fence,
    root: RequiredRoot,
    admission: bool,
}
impl Roots {
    pub(super) fn new(
        roll: &RollStore,
        action: DefinitionRef,
        commit: CommitToken,
        _utc: i64,
        job: &ConductJob,
        before: Option<(ConductJob, ConductExecution)>,
    ) -> Result<Self, PublicationError> {
        let admission = before.is_none();
        if admission != (job.operation() == commit) {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            root: job_root(job)?,
            admission,
            fence: Fence {
                action,
                commit,
                stamp: roll.conduct_stamp().map_err(invalid)?,
                before,
                native: roll.conduct.configuration()?.native.clone(),
            },
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.fence.commit
    }
    pub(crate) fn fence(&self) -> &Fence {
        &self.fence
    }
    pub(crate) fn root(&self) -> &RequiredRoot {
        &self.root
    }
    pub(crate) fn admission(&self) -> bool {
        self.admission
    }
}
