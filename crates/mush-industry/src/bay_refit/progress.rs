//! Bounded replay of real K occurrences (the bay's copy of the core refit's). No elapsed-time counter or caller-set
//! completion flag can replace the actual DurableSchedule transition.
use super::{BayRefitError as Error, CADENCE, MAX_WORK_UNITS};
const MAX_PROGRESS_BYTES: usize = 32768;
fn bad<T>(_: T) -> Error {
    Error::Integrity
}
use crate::workorder::{ProductionSchedule, WorkOrderId};
use mush_core::{
    CanonicalId, ClockRef, ClockStamp, CommitToken, DurableSchedule, DurableScheduleDraft,
    ScheduleAdvance, ScheduleCatchUpOrder, ScheduleDriveCause, ScheduleEvaluation, ScheduleId,
    ScheduleRestartPolicy,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

// K deliberately does not decode ScheduleAdvance/its drive cause as authority.
// This closed history tag is only data used to reproduce the actual plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DriveCause {
    Live,
    Restart,
}
impl DriveCause {
    const fn capture(cause: ScheduleDriveCause) -> Self {
        match cause {
            ScheduleDriveCause::Live => Self::Live,
            ScheduleDriveCause::Restart => Self::Restart,
        }
    }
    const fn native(self) -> ScheduleDriveCause {
        match self {
            Self::Live => ScheduleDriveCause::Live,
            Self::Restart => ScheduleDriveCause::Restart,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OccurrenceProof {
    commit: CommitToken,
    observed: ClockStamp,
    cause: DriveCause,
    occurrences: u16,
}

/// One permanent bounded proof per acknowledged advance, replayable from the
/// actual admission. There are at most 167 because each must do nonzero work.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BayProgress {
    order: WorkOrderId,
    production: ProductionSchedule,
    cursor: DurableSchedule,
    advances: Vec<OccurrenceProof>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressWire {
    order: WorkOrderId,
    production: ProductionSchedule,
    cursor: DurableSchedule,
    advances: Vec<OccurrenceProof>,
}
impl<'de> Deserialize<'de> for BayProgress {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = ProgressWire::deserialize(d)?;
        let r = Self {
            order: w.order,
            production: w.production,
            cursor: w.cursor,
            advances: w.advances,
        };
        r.validate().map_err(serde::de::Error::custom)?;
        Ok(r)
    }
}

pub(crate) fn schedule_id(order: WorkOrderId) -> Result<ScheduleId, Error> {
    let id = CanonicalId::new(format!("bay.{:016x}", order.0)).map_err(bad)?;
    ScheduleId::derive(&crate::record::owner(), &id).map_err(bad)
}

impl BayProgress {
    pub(crate) fn new(
        order: WorkOrderId,
        start: ClockStamp,
        work_units: u64,
    ) -> Result<Self, Error> {
        if !matches!(start.clock, ClockRef::WorldIc { generation } if generation > 0)
            || !(1..=MAX_WORK_UNITS).contains(&work_units)
        {
            return Err(Error::Invalid);
        }
        let id = schedule_id(order)?;
        let expires = mush_inventory::reservation_deadline(&start).map_err(bad)?;
        let duration = i64::try_from(work_units)
            .map_err(bad)?
            .checked_mul(CADENCE)
            .ok_or(Error::Capacity)?;
        if start.at.checked_add(duration).ok_or(Error::Capacity)? >= expires.at {
            return Err(Error::Invalid);
        }
        let cursor = DurableSchedule::new(DurableScheduleDraft {
            id,
            owner: crate::record::owner(),
            clock: start.clock.clone(),
            next_due: ClockStamp {
                clock: start.clock.clone(),
                at: start.at.checked_add(CADENCE).ok_or(Error::Capacity)?,
            },
            cadence: Some(CADENCE),
            generation: 1,
            next_ordinal: 0,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            // All occurrences before the real seven-day expiry fit; none may
            // be silently skipped and then credited as completed production.
            max_catch_up: crate::workorder::PRODUCTION_MAX_CATCH_UP,
        })
        .map_err(bad)?;
        let production = ProductionSchedule::new(
            id,
            start,
            CanonicalId::new(super::CLOCK_POLICY).map_err(bad)?,
            work_units,
            0,
        )
        .map_err(bad)?;
        Ok(Self {
            order,
            production,
            cursor,
            advances: Vec::new(),
        })
    }

    pub(crate) fn order(&self) -> WorkOrderId {
        self.order
    }

    pub(crate) fn cursor(&self) -> &DurableSchedule {
        &self.cursor
    }
    pub(crate) fn production(&self) -> &ProductionSchedule {
        &self.production
    }

    fn live_time(&self, at: &ClockStamp) -> Result<(), Error> {
        let start = self.production.started();
        let expiry = mush_inventory::reservation_deadline(start).map_err(bad)?;
        if at.clock != start.clock || at.at < start.at {
            return Err(Error::Invalid);
        }
        if at.at >= expiry.at {
            return Err(Error::Expired);
        }
        if self.advances.last().is_some_and(|p| at.at < p.observed.at) {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    /// Called only after the real schedule source has placed the identical
    /// cursor postimage in this ActionTxn. The provider compares that image and
    /// binds the exact scheduler ingress; this method creates no authority.
    pub(crate) fn advance(
        &self,
        commit: CommitToken,
        supplied: &ScheduleAdvance,
    ) -> Result<Self, Error> {
        self.validate()?;
        self.advance_checked(commit, supplied)
    }

    fn advance_checked(
        &self,
        commit: CommitToken,
        supplied: &ScheduleAdvance,
    ) -> Result<Self, Error> {
        self.live_time(supplied.observed_at())?;
        if self.production.is_complete()
            || self.advances.len() >= usize::try_from(MAX_WORK_UNITS).map_err(bad)?
            || self.advances.iter().any(|p| p.commit == commit)
        {
            return Err(Error::Invalid);
        }
        let evaluation = self
            .cursor
            .plan_due(
                self.cursor.revision(),
                supplied.observed_at(),
                supplied.cause(),
            )
            .map_err(bad)?;
        let ScheduleEvaluation::Advance(actual) = evaluation else {
            return Err(Error::Invalid);
        };
        if actual.as_ref() != supplied
            || supplied.occurrences().is_empty()
            || supplied.skipped().is_some()
        {
            return Err(Error::Integrity);
        }
        let occurrences = u16::try_from(supplied.occurrences().len()).map_err(bad)?;
        let mut next = self.clone();
        next.production = self
            .production
            .advanced(u64::from(occurrences))
            .map_err(bad)?;
        next.cursor = supplied.post_image().clone();
        next.advances.push(OccurrenceProof {
            commit,
            observed: supplied.observed_at().clone(),
            cause: DriveCause::capture(supplied.cause()),
            occurrences,
        });
        next.require_room()?;
        Ok(next)
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.advances.len() > usize::try_from(MAX_WORK_UNITS).map_err(bad)? {
            return Err(Error::Capacity);
        }
        self.require_room()?;
        let mut rebuilt = Self::new(
            self.order,
            self.production.started().clone(),
            self.production.total_units(),
        )?;
        let mut commits = BTreeSet::new();
        for proof in &self.advances {
            if !commits.insert(proof.commit) {
                return Err(Error::Integrity);
            }
            let ScheduleEvaluation::Advance(actual) = rebuilt
                .cursor
                .plan_due(
                    rebuilt.cursor.revision(),
                    &proof.observed,
                    proof.cause.native(),
                )
                .map_err(bad)?
            else {
                return Err(Error::Integrity);
            };
            if actual.occurrences().len() != usize::from(proof.occurrences) {
                return Err(Error::Integrity);
            }
            rebuilt = rebuilt.advance_checked(proof.commit, &actual)?;
        }
        if &rebuilt != self {
            return Err(Error::Integrity);
        }
        Ok(())
    }

    fn require_room(&self) -> Result<(), Error> {
        let value = serde_json::to_value(self).map_err(bad)?;
        mush_core::mechanics::canonical_json_bytes_with_limit(&value, MAX_PROGRESS_BYTES)
            .map_err(|_| Error::Capacity)?;
        Ok(())
    }
}
