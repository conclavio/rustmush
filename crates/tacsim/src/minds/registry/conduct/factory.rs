//! The registered immutable policy creates real private arenas off-world.
//! An owner-created candidate is still rechecked at actual Admit custody.
use super::{Role, manifest::Policy, run::Run, worker::Worker};
use crate::minds::{
    bulk::{
        conduct_tickets::AdmissionData,
        executor::Machine,
        model::{Opponent, Plan, Scenario, WorkKind},
    },
    registry::subject::PreparedSubject,
};
use mush_core::{AccountId, DefinitionRef, GOD};
use mush_roll::commerce::{
    conduct::{ConductCandidate, ConductRole},
    rating::RatingError,
};
use std::sync::Arc;

pub(crate) struct Factory {
    policy: Arc<Policy>,
    manifest: DefinitionRef,
}
pub(in crate::minds) struct PreparedConduct {
    pub(in crate::minds) data: AdmissionData,
    pub(in crate::minds) machine: Box<dyn Machine>,
}
impl Factory {
    pub(super) fn registered(
        policy: Arc<Policy>,
        manifest: DefinitionRef,
    ) -> Result<Self, RatingError> {
        if policy.definition_ref()? != manifest {
            return Err(RatingError::Identity);
        }
        Ok(Self { policy, manifest })
    }
    pub(in crate::minds) fn prepare(
        &self,
        account: AccountId,
        nonce: u64,
        candidate: ConductCandidate,
        source: Arc<str>,
    ) -> Result<PreparedConduct, RatingError> {
        if nonce == 0 || candidate.manifest() != &self.manifest {
            return Err(RatingError::Identity);
        }
        let (revision, subject, _) = candidate.into_parts();
        subject.validate().map_err(|_| RatingError::Identity)?;
        if !subject
            .revision()
            .matches(&revision)
            .map_err(|_| RatingError::Identity)?
        {
            return Err(RatingError::Identity);
        }
        let prepared = Arc::new(PreparedSubject::prepare_reconformance(
            &revision,
            &source,
            &self.policy.physics.cfg.arena,
        )?);
        if prepared.subject() != subject.worker() {
            return Err(RatingError::Identity);
        }
        let role = match subject.role() {
            ConductRole::Escort => Role::Escort,
            ConductRole::Transit => Role::Transit,
        };
        let run = Run::new(
            Arc::clone(&self.policy),
            prepared,
            subject.profile().to_owned(),
            role,
        )?;
        let budget = run.billing_budget()?;
        let start = run.start(&subject)?.prepare()?;
        let plan = Plan {
            kind: WorkKind::Conduct,
            registry: None,
            conduct: Some(budget),
            scenario: Scenario::Duel,
            opponent: Opponent::Doctrine,
            per_side: 0,
            unit_count: 0,
            games: 0,
            seeds: Vec::new(),
            limit_ms: 0,
            native_substep_ms: 0,
            start: start.bytes().descriptor().clone(),
            result_reservation_bytes: 256 * 1024,
        };
        plan.validate().map_err(|_| RatingError::Invalid)?;
        let machine = Worker::new(run).map_err(|_| RatingError::Invalid)?;
        Ok(PreparedConduct {
            data: AdmissionData {
                account,
                character: GOD,
                nonce,
                subject,
                plan,
                start,
            },
            machine,
        })
    }
}
