//! Durable, typed-clock schedule cursors and bounded catch-up planning.
//!
//! The in-memory scheduler remains a wake-up hint.  This module holds the
//! authoritative arithmetic: a provider reads one persisted cursor, obtains
//! an exact [`ClockStamp`], and prepares the returned post-image together with
//! every selected occurrence in one idempotent authoritative action.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::action::{CanonicalHash, CanonicalHashError};
use crate::clock::{ClockMismatch, ClockRef, ClockStamp};
use crate::mechanics::{CanonicalId, OwnerId};
use crate::{
    StateFamilyDescriptor, StateFamilyDescriptorError, StateFamilyId, StateKeyCodec,
    StateValueDecoder, StateValueSerializer, StateValueValidator,
};

/// Maximum occurrences one authoritative schedule advance may select.
pub const MAX_SCHEDULE_CATCH_UP: u32 = 1_024;

/// Canonical engine contract for owner-local durable schedule state.
#[derive(Clone, Copy, Debug, Default)]
pub struct DurableScheduleStateCodec;

impl StateKeyCodec<ScheduleId> for DurableScheduleStateCodec {
    fn encode_key(&self, key: &ScheduleId) -> Result<String, String> {
        Ok(key.canonical_hash().to_hex())
    }
}

impl StateValueDecoder<DurableSchedule> for DurableScheduleStateCodec {
    fn decode_value(&self, value: &serde_json::Value) -> Result<DurableSchedule, String> {
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }
}

impl StateValueValidator<DurableSchedule> for DurableScheduleStateCodec {
    fn validate_value(&self, value: &DurableSchedule) -> Result<(), String> {
        value.validate().map_err(|error| error.to_string())
    }
}

impl StateValueSerializer<DurableSchedule> for DurableScheduleStateCodec {
    fn serialize_value(&self, value: &DurableSchedule) -> Result<serde_json::Value, String> {
        serde_json::to_value(value).map_err(|error| error.to_string())
    }
}

/// Build the reusable typed-family descriptor for an owner-local schedule
/// collection.
pub fn durable_schedule_state_family_descriptor(
    family: StateFamilyId,
) -> Result<StateFamilyDescriptor, StateFamilyDescriptorError> {
    StateFamilyDescriptor::new(
        family,
        CanonicalId::new("durable_schedule_id.v1").expect("static canonical id"),
        CanonicalId::new("durable_schedule.v1").expect("static canonical id"),
        DurableSchedule::SCHEMA_VERSION,
        CanonicalId::new("durable_schedule_decode.v1").expect("static canonical id"),
        CanonicalId::new("durable_schedule_validate.v1").expect("static canonical id"),
        CanonicalId::new("durable_schedule_serialize.v1").expect("static canonical id"),
    )
}

/// Stable identity of one provider-owned schedule across cursor revisions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ScheduleId(CanonicalHash);

impl ScheduleId {
    pub fn derive(owner: &OwnerId, local_id: &CanonicalId) -> Result<Self, CanonicalHashError> {
        CanonicalHash::for_canonical_value(
            "durable_schedule_id.v1",
            1,
            &json!({ "owner": owner, "local_id": local_id }),
            16 * 1024,
        )
        .map(Self)
    }

    #[must_use]
    pub const fn canonical_hash(self) -> CanonicalHash {
        self.0
    }
}

/// Policy used only when boot discovers an elapsed active cursor.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleRestartPolicy {
    /// Apply the ordinary bounded catch-up policy.
    Resume,
    /// Forgive every elapsed occurrence and advance to the first future one.
    SkipElapsed,
    /// Persist a suspended state without moving the cursor.
    SuspendIfElapsed,
}

/// Which end of an oversized due range is selected for provider work.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleCatchUpOrder {
    /// Select the earliest bounded prefix and forgive the newer excess.
    OldestFirst,
    /// Forgive the older excess and select the latest bounded suffix.
    LatestFirst,
}

/// Why the durable cursor is being evaluated.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleDriveCause {
    Live,
    Restart,
}

/// Durable cursor lifecycle.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleState {
    Active,
    Suspended,
    Completed,
    /// No later deadline can be represented on the clock's `i64` scalar.
    ClockExhausted,
}

/// Construction surface for a validated schedule record.
#[derive(Clone, Debug)]
pub struct DurableScheduleDraft {
    pub id: ScheduleId,
    pub owner: OwnerId,
    pub clock: ClockRef,
    pub next_due: ClockStamp,
    /// Positive interval on `clock`, or `None` for a one-shot schedule.
    pub cadence: Option<i64>,
    pub generation: u64,
    pub next_ordinal: u64,
    pub restart: ScheduleRestartPolicy,
    pub catch_up_order: ScheduleCatchUpOrder,
    pub max_catch_up: u32,
}

/// Persisted authoritative schedule cursor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DurableSchedule {
    schema_version: u16,
    id: ScheduleId,
    owner: OwnerId,
    clock: ClockRef,
    next_due: ClockStamp,
    cadence: Option<i64>,
    generation: u64,
    next_ordinal: u64,
    restart: ScheduleRestartPolicy,
    catch_up_order: ScheduleCatchUpOrder,
    max_catch_up: u32,
    state: ScheduleState,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableScheduleWire {
    schema_version: u16,
    id: ScheduleId,
    owner: OwnerId,
    clock: ClockRef,
    next_due: ClockStamp,
    cadence: Option<i64>,
    generation: u64,
    next_ordinal: u64,
    restart: ScheduleRestartPolicy,
    catch_up_order: ScheduleCatchUpOrder,
    max_catch_up: u32,
    state: ScheduleState,
    revision: u64,
}

impl<'de> Deserialize<'de> for DurableSchedule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DurableScheduleWire::deserialize(deserializer)?;
        Self::from_parts(wire).map_err(serde::de::Error::custom)
    }
}

impl DurableSchedule {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn new(draft: DurableScheduleDraft) -> Result<Self, DurableScheduleError> {
        Self::from_parts(DurableScheduleWire {
            schema_version: Self::SCHEMA_VERSION,
            id: draft.id,
            owner: draft.owner,
            clock: draft.clock,
            next_due: draft.next_due,
            cadence: draft.cadence,
            generation: draft.generation,
            next_ordinal: draft.next_ordinal,
            restart: draft.restart,
            catch_up_order: draft.catch_up_order,
            max_catch_up: draft.max_catch_up,
            state: ScheduleState::Active,
            revision: 1,
        })
    }

    fn from_parts(wire: DurableScheduleWire) -> Result<Self, DurableScheduleError> {
        if wire.schema_version != Self::SCHEMA_VERSION {
            return Err(DurableScheduleError::UnsupportedSchema {
                actual: wire.schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        if wire.generation == 0 {
            return Err(DurableScheduleError::ZeroGeneration);
        }
        if wire.revision == 0 {
            return Err(DurableScheduleError::ZeroRevision);
        }
        if wire.clock != wire.next_due.clock {
            return Err(DurableScheduleError::ClockIdentityMismatch);
        }
        if mutable_clock_generation(&wire.clock) == Some(0) {
            return Err(DurableScheduleError::ZeroClockGeneration);
        }
        if wire.cadence.is_some_and(|cadence| cadence <= 0) {
            return Err(DurableScheduleError::InvalidCadence);
        }
        if wire.max_catch_up == 0 || wire.max_catch_up > MAX_SCHEDULE_CATCH_UP {
            return Err(DurableScheduleError::InvalidCatchUp {
                actual: wire.max_catch_up,
                max: MAX_SCHEDULE_CATCH_UP,
            });
        }
        Ok(Self {
            schema_version: wire.schema_version,
            id: wire.id,
            owner: wire.owner,
            clock: wire.clock,
            next_due: wire.next_due,
            cadence: wire.cadence,
            generation: wire.generation,
            next_ordinal: wire.next_ordinal,
            restart: wire.restart,
            catch_up_order: wire.catch_up_order,
            max_catch_up: wire.max_catch_up,
            state: wire.state,
            revision: wire.revision,
        })
    }

    /// Re-run every durable shape invariant.
    pub fn validate(&self) -> Result<(), DurableScheduleError> {
        Self::from_parts(DurableScheduleWire {
            schema_version: self.schema_version,
            id: self.id,
            owner: self.owner.clone(),
            clock: self.clock.clone(),
            next_due: self.next_due.clone(),
            cadence: self.cadence,
            generation: self.generation,
            next_ordinal: self.next_ordinal,
            restart: self.restart,
            catch_up_order: self.catch_up_order,
            max_catch_up: self.max_catch_up,
            state: self.state,
            revision: self.revision,
        })
        .map(|_| ())
    }

    #[must_use]
    pub const fn id(&self) -> ScheduleId {
        self.id
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn clock(&self) -> &ClockRef {
        &self.clock
    }

    #[must_use]
    pub const fn next_due(&self) -> &ClockStamp {
        &self.next_due
    }

    #[must_use]
    pub const fn cadence(&self) -> Option<i64> {
        self.cadence
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn next_ordinal(&self) -> u64 {
        self.next_ordinal
    }

    #[must_use]
    pub const fn restart_policy(&self) -> ScheduleRestartPolicy {
        self.restart
    }

    #[must_use]
    pub const fn catch_up_order(&self) -> ScheduleCatchUpOrder {
        self.catch_up_order
    }

    #[must_use]
    pub const fn max_catch_up(&self) -> u32 {
        self.max_catch_up
    }

    #[must_use]
    pub const fn state(&self) -> ScheduleState {
        self.state
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Evaluate one exact clock observation without mutating live state.
    pub fn plan_due(
        &self,
        expected_revision: u64,
        now: &ClockStamp,
        cause: ScheduleDriveCause,
    ) -> Result<ScheduleEvaluation, DurableScheduleError> {
        self.require_revision(expected_revision)?;
        if self.state != ScheduleState::Active {
            return Ok(ScheduleEvaluation::Inactive(self.state));
        }
        self.next_due.checked_cmp(now)?;
        if now.at < self.next_due.at {
            return Ok(ScheduleEvaluation::Waiting {
                next_due: self.next_due.clone(),
            });
        }

        if cause == ScheduleDriveCause::Restart
            && self.restart == ScheduleRestartPolicy::SuspendIfElapsed
        {
            return Ok(ScheduleEvaluation::Advance(Box::new(ScheduleAdvance {
                deterministic_key: self.advance_key()?,
                cause,
                observed_at: now.clone(),
                occurrences: Vec::new(),
                skipped: None,
                post_image: Self {
                    state: ScheduleState::Suspended,
                    revision: self.next_revision()?,
                    ..self.clone()
                },
            })));
        }

        let total_due = self.total_due(now)?;
        let selected_count = if cause == ScheduleDriveCause::Restart
            && self.restart == ScheduleRestartPolicy::SkipElapsed
        {
            0
        } else {
            total_due.min(u64::from(self.max_catch_up))
        };
        let skipped_count = total_due - selected_count;
        let (selected_start, skipped_start) = match self.catch_up_order {
            ScheduleCatchUpOrder::OldestFirst => (0, selected_count),
            ScheduleCatchUpOrder::LatestFirst => (skipped_count, 0),
        };

        let occurrences = (0..selected_count)
            .map(|offset| self.occurrence(selected_start + offset))
            .collect::<Result<Vec<_>, _>>()?;
        let skipped = (skipped_count != 0)
            .then(|| self.gap(skipped_start, skipped_count))
            .transpose()?;
        let post_image = self.advanced(total_due)?;
        Ok(ScheduleEvaluation::Advance(Box::new(ScheduleAdvance {
            deterministic_key: self.advance_key()?,
            cause,
            observed_at: now.clone(),
            occurrences,
            skipped,
            post_image,
        })))
    }

    /// Explicitly resume a suspended cursor. Whether elapsed work is then
    /// caught up remains governed by its existing bounded policy.
    pub fn checked_resume(&self, expected_revision: u64) -> Result<Self, DurableScheduleError> {
        self.require_revision(expected_revision)?;
        if self.state != ScheduleState::Suspended {
            return Err(DurableScheduleError::NotSuspended { state: self.state });
        }
        Ok(Self {
            state: ScheduleState::Active,
            revision: self.next_revision()?,
            ..self.clone()
        })
    }

    /// Kernel-only terminalization of an already advanced cursor. Its revision
    /// and occurrence ordinal belong to that same advance, not a second write.
    pub(crate) fn completed_post_image(&self) -> Self {
        Self {
            state: ScheduleState::Completed,
            ..self.clone()
        }
    }

    fn total_due(&self, now: &ClockStamp) -> Result<u64, DurableScheduleError> {
        let Some(cadence) = self.cadence else {
            return Ok(1);
        };
        let delta = i128::from(now.at) - i128::from(self.next_due.at);
        let total = delta / i128::from(cadence) + 1;
        u64::try_from(total).map_err(|_| DurableScheduleError::ArithmeticOverflow)
    }

    fn occurrence(&self, offset: u64) -> Result<ScheduleOccurrence, DurableScheduleError> {
        let ordinal = self
            .next_ordinal
            .checked_add(offset)
            .ok_or(DurableScheduleError::ArithmeticOverflow)?;
        let due = self.deadline_at(offset)?;
        let deterministic_key = CanonicalHash::for_canonical_value(
            "durable_schedule_occurrence.v1",
            1,
            &json!({
                "schedule": self.id,
                "owner": self.owner,
                "generation": self.generation,
                "ordinal": ordinal,
                "due": due,
            }),
            16 * 1024,
        )?;
        Ok(ScheduleOccurrence {
            schedule: self.id,
            owner: self.owner.clone(),
            generation: self.generation,
            ordinal,
            due,
            deterministic_key,
        })
    }

    fn advance_key(&self) -> Result<CanonicalHash, DurableScheduleError> {
        CanonicalHash::for_canonical_value(
            "durable_schedule_advance.v1",
            1,
            &json!({
                "schedule": self.id,
                "owner": self.owner,
                "generation": self.generation,
                "next_ordinal": self.next_ordinal,
                "revision": self.revision,
            }),
            16 * 1024,
        )
        .map_err(DurableScheduleError::Identity)
    }

    fn gap(&self, start: u64, count: u64) -> Result<ScheduleGap, DurableScheduleError> {
        let last_offset = start
            .checked_add(count)
            .and_then(|value| value.checked_sub(1))
            .ok_or(DurableScheduleError::ArithmeticOverflow)?;
        Ok(ScheduleGap {
            first_ordinal: self
                .next_ordinal
                .checked_add(start)
                .ok_or(DurableScheduleError::ArithmeticOverflow)?,
            count,
            from: self.deadline_at(start)?,
            through: self.deadline_at(last_offset)?,
        })
    }

    fn deadline_at(&self, offset: u64) -> Result<ClockStamp, DurableScheduleError> {
        let cadence = self.cadence.unwrap_or(0);
        let at = i128::from(self.next_due.at)
            .checked_add(i128::from(cadence) * i128::from(offset))
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(DurableScheduleError::ArithmeticOverflow)?;
        Ok(ClockStamp {
            clock: self.clock.clone(),
            at,
        })
    }

    fn advanced(&self, total_due: u64) -> Result<Self, DurableScheduleError> {
        let next_ordinal = self
            .next_ordinal
            .checked_add(total_due)
            .ok_or(DurableScheduleError::ArithmeticOverflow)?;
        let revision = self.next_revision()?;
        let Some(cadence) = self.cadence else {
            return Ok(Self {
                next_ordinal,
                state: ScheduleState::Completed,
                revision,
                ..self.clone()
            });
        };
        let next_at = i128::from(self.next_due.at)
            .checked_add(i128::from(cadence) * i128::from(total_due))
            .and_then(|value| i64::try_from(value).ok());
        let Some(next_at) = next_at else {
            return Ok(Self {
                next_ordinal,
                state: ScheduleState::ClockExhausted,
                revision,
                ..self.clone()
            });
        };
        Ok(Self {
            next_due: ClockStamp {
                clock: self.clock.clone(),
                at: next_at,
            },
            next_ordinal,
            revision,
            ..self.clone()
        })
    }

    fn require_revision(&self, expected: u64) -> Result<(), DurableScheduleError> {
        if self.revision != expected {
            return Err(DurableScheduleError::StaleRevision {
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }

    fn next_revision(&self) -> Result<u64, DurableScheduleError> {
        self.revision
            .checked_add(1)
            .ok_or(DurableScheduleError::RevisionExhausted)
    }
}

fn mutable_clock_generation(clock: &ClockRef) -> Option<u64> {
    match clock {
        ClockRef::Utc => None,
        ClockRef::WorldIc { generation }
        | ClockRef::SceneTurn { generation, .. }
        | ClockRef::DomainTick { generation, .. }
        | ClockRef::Registered { generation, .. } => Some(*generation),
    }
}

/// One provider-visible due occurrence with a stable idempotency component.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleOccurrence {
    schedule: ScheduleId,
    owner: OwnerId,
    generation: u64,
    ordinal: u64,
    due: ClockStamp,
    deterministic_key: CanonicalHash,
}

impl ScheduleOccurrence {
    #[must_use]
    pub const fn schedule(&self) -> ScheduleId {
        self.schedule
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn ordinal(&self) -> u64 {
        self.ordinal
    }

    #[must_use]
    pub const fn due(&self) -> &ClockStamp {
        &self.due
    }

    #[must_use]
    pub const fn deterministic_key(&self) -> CanonicalHash {
        self.deterministic_key
    }
}

/// Contiguous elapsed range deliberately not presented for provider work.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleGap {
    first_ordinal: u64,
    count: u64,
    from: ClockStamp,
    through: ClockStamp,
}

impl ScheduleGap {
    #[must_use]
    pub const fn first_ordinal(&self) -> u64 {
        self.first_ordinal
    }

    #[must_use]
    pub const fn count(&self) -> u64 {
        self.count
    }

    #[must_use]
    pub const fn from(&self) -> &ClockStamp {
        &self.from
    }

    #[must_use]
    pub const fn through(&self) -> &ClockStamp {
        &self.through
    }
}

/// Non-mutating evaluation result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleEvaluation {
    Waiting { next_due: ClockStamp },
    Inactive(ScheduleState),
    Advance(Box<ScheduleAdvance>),
}

/// Exact bounded provider work and the cursor post-image that must commit
/// atomically with its outcomes and any attributed skipped-range event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleAdvance {
    deterministic_key: CanonicalHash,
    cause: ScheduleDriveCause,
    observed_at: ClockStamp,
    occurrences: Vec<ScheduleOccurrence>,
    skipped: Option<ScheduleGap>,
    post_image: DurableSchedule,
}

impl ScheduleAdvance {
    /// Stable top-level idempotency component for this exact pre-state
    /// cursor. It identifies the whole advance, not any one occurrence.
    #[must_use]
    pub const fn deterministic_key(&self) -> CanonicalHash {
        self.deterministic_key
    }

    #[must_use]
    pub const fn schedule(&self) -> ScheduleId {
        self.post_image.id
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.post_image.owner
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.post_image.generation
    }

    #[must_use]
    pub const fn cause(&self) -> ScheduleDriveCause {
        self.cause
    }

    #[must_use]
    pub const fn observed_at(&self) -> &ClockStamp {
        &self.observed_at
    }

    #[must_use]
    pub fn occurrences(&self) -> &[ScheduleOccurrence] {
        &self.occurrences
    }

    #[must_use]
    pub const fn skipped(&self) -> Option<&ScheduleGap> {
        self.skipped.as_ref()
    }

    #[must_use]
    pub const fn post_image(&self) -> &DurableSchedule {
        &self.post_image
    }

    #[must_use]
    pub fn into_post_image(self) -> DurableSchedule {
        self.post_image
    }
}

/// Invalid schedule record or deterministic evaluation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DurableScheduleError {
    #[error("unsupported durable schedule schema {actual}; supported schema is {supported}")]
    UnsupportedSchema { actual: u16, supported: u16 },
    #[error("schedule generation zero is reserved")]
    ZeroGeneration,
    #[error("schedule revision zero is reserved")]
    ZeroRevision,
    #[error("schedule clock and next deadline identity differ")]
    ClockIdentityMismatch,
    #[error("mutable schedule clock generation zero is reserved")]
    ZeroClockGeneration,
    #[error("schedule cadence must be positive")]
    InvalidCadence,
    #[error("schedule catch-up {actual} must be in 1..={max}")]
    InvalidCatchUp { actual: u32, max: u32 },
    #[error("schedule revision is stale: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("schedule is not suspended: {state:?}")]
    NotSuspended { state: ScheduleState },
    #[error("schedule arithmetic overflow")]
    ArithmeticOverflow,
    #[error("schedule revision is exhausted")]
    RevisionExhausted,
    #[error("schedule clock mismatch: {0}")]
    Clock(#[from] ClockMismatch),
    #[error("schedule identity hashing failed: {0}")]
    Identity(#[from] CanonicalHashError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::Registered {
                owner: OwnerId::new("schedule_test").unwrap(),
                id: CanonicalId::new("production").unwrap(),
                generation: 4,
            },
            at,
        }
    }

    fn schedule(restart: ScheduleRestartPolicy, order: ScheduleCatchUpOrder) -> DurableSchedule {
        let owner = OwnerId::new("schedule_test").unwrap();
        DurableSchedule::new(DurableScheduleDraft {
            id: ScheduleId::derive(&owner, &CanonicalId::new("factory_a").unwrap()).unwrap(),
            owner,
            clock: stamp(0).clock,
            next_due: stamp(10),
            cadence: Some(10),
            generation: 3,
            next_ordinal: 20,
            restart,
            catch_up_order: order,
            max_catch_up: 3,
        })
        .unwrap()
    }

    fn advance(evaluation: ScheduleEvaluation) -> ScheduleAdvance {
        let ScheduleEvaluation::Advance(advance) = evaluation else {
            panic!("expected schedule advance")
        };
        *advance
    }

    #[test]
    fn latest_first_catch_up_forgives_old_excess_and_advances_past_all_due() {
        let schedule = schedule(
            ScheduleRestartPolicy::Resume,
            ScheduleCatchUpOrder::LatestFirst,
        );
        let advance = advance(
            schedule
                .plan_due(1, &stamp(100), ScheduleDriveCause::Restart)
                .unwrap(),
        );

        assert_eq!(
            advance
                .occurrences()
                .iter()
                .map(ScheduleOccurrence::ordinal)
                .collect::<Vec<_>>(),
            vec![27, 28, 29]
        );
        assert_eq!(
            advance
                .occurrences()
                .iter()
                .map(|occurrence| occurrence.due().at)
                .collect::<Vec<_>>(),
            vec![80, 90, 100]
        );
        let gap = advance.skipped().expect("older forgiven range");
        assert_eq!(gap.first_ordinal(), 20);
        assert_eq!(gap.count(), 7);
        assert_eq!((gap.from().at, gap.through().at), (10, 70));
        assert_eq!(advance.post_image().next_ordinal(), 30);
        assert_eq!(advance.post_image().next_due().at, 110);
        assert_eq!(advance.post_image().revision(), 2);
    }

    #[test]
    fn oldest_first_selects_prefix_but_still_records_the_skipped_suffix() {
        let schedule = schedule(
            ScheduleRestartPolicy::Resume,
            ScheduleCatchUpOrder::OldestFirst,
        );
        let advance = advance(
            schedule
                .plan_due(1, &stamp(100), ScheduleDriveCause::Live)
                .unwrap(),
        );

        assert_eq!(
            advance
                .occurrences()
                .iter()
                .map(ScheduleOccurrence::ordinal)
                .collect::<Vec<_>>(),
            vec![20, 21, 22]
        );
        let gap = advance.skipped().expect("newer skipped range");
        assert_eq!(gap.first_ordinal(), 23);
        assert_eq!(gap.count(), 7);
        assert_eq!((gap.from().at, gap.through().at), (40, 100));
    }

    #[test]
    fn restart_skip_and_suspend_are_explicit_durable_transitions() {
        let skipped = schedule(
            ScheduleRestartPolicy::SkipElapsed,
            ScheduleCatchUpOrder::LatestFirst,
        );
        let skip_advance = advance(
            skipped
                .plan_due(1, &stamp(100), ScheduleDriveCause::Restart)
                .unwrap(),
        );
        assert!(skip_advance.occurrences().is_empty());
        assert_eq!(skip_advance.skipped().unwrap().count(), 10);
        assert_eq!(skip_advance.post_image().next_due().at, 110);

        let suspend = schedule(
            ScheduleRestartPolicy::SuspendIfElapsed,
            ScheduleCatchUpOrder::LatestFirst,
        );
        let suspend_advance = advance(
            suspend
                .plan_due(1, &stamp(100), ScheduleDriveCause::Restart)
                .unwrap(),
        );
        assert!(suspend_advance.occurrences().is_empty());
        assert!(suspend_advance.skipped().is_none());
        assert_eq!(
            suspend_advance.post_image().state(),
            ScheduleState::Suspended
        );
        assert_eq!(suspend_advance.post_image().next_due().at, 10);
        assert_eq!(suspend_advance.post_image().next_ordinal(), 20);
        assert_eq!(
            suspend_advance
                .post_image()
                .checked_resume(2)
                .unwrap()
                .state(),
            ScheduleState::Active
        );
    }

    #[test]
    fn restart_policy_does_not_turn_live_lag_into_an_implicit_skip() {
        let schedule = schedule(
            ScheduleRestartPolicy::SkipElapsed,
            ScheduleCatchUpOrder::LatestFirst,
        );
        let advance = advance(
            schedule
                .plan_due(1, &stamp(30), ScheduleDriveCause::Live)
                .unwrap(),
        );
        assert_eq!(advance.occurrences().len(), 3);
        assert!(advance.skipped().is_none());
    }

    #[test]
    fn waits_without_revision_change_and_one_shot_completes_once() {
        let recurring = schedule(
            ScheduleRestartPolicy::Resume,
            ScheduleCatchUpOrder::OldestFirst,
        );
        assert_eq!(
            recurring
                .plan_due(1, &stamp(9), ScheduleDriveCause::Live)
                .unwrap(),
            ScheduleEvaluation::Waiting {
                next_due: stamp(10)
            }
        );

        let owner = OwnerId::new("schedule_test").unwrap();
        let one_shot = DurableSchedule::new(DurableScheduleDraft {
            id: ScheduleId::derive(&owner, &CanonicalId::new("one_shot").unwrap()).unwrap(),
            owner,
            clock: stamp(0).clock,
            next_due: stamp(10),
            cadence: None,
            generation: 1,
            next_ordinal: 0,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: 1,
        })
        .unwrap();
        let advance = advance(
            one_shot
                .plan_due(1, &stamp(10), ScheduleDriveCause::Live)
                .unwrap(),
        );
        assert_eq!(advance.occurrences().len(), 1);
        assert_eq!(advance.post_image().state(), ScheduleState::Completed);
        assert_eq!(
            advance
                .post_image()
                .plan_due(2, &stamp(100), ScheduleDriveCause::Live)
                .unwrap(),
            ScheduleEvaluation::Inactive(ScheduleState::Completed)
        );
    }

    #[test]
    fn occurrence_identity_is_stable_and_generation_scoped() {
        let schedule = schedule(
            ScheduleRestartPolicy::Resume,
            ScheduleCatchUpOrder::OldestFirst,
        );
        let first = advance(
            schedule
                .plan_due(1, &stamp(10), ScheduleDriveCause::Live)
                .unwrap(),
        );
        let retry = advance(
            schedule
                .plan_due(1, &stamp(10), ScheduleDriveCause::Live)
                .unwrap(),
        );
        assert_eq!(
            first.occurrences()[0].deterministic_key(),
            retry.occurrences()[0].deterministic_key()
        );
    }

    #[test]
    fn strict_decode_revalidates_clock_cadence_and_bounds() {
        let valid_schedule = schedule(
            ScheduleRestartPolicy::Resume,
            ScheduleCatchUpOrder::OldestFirst,
        );
        let mut value = serde_json::to_value(valid_schedule).unwrap();
        value["cadence"] = json!(0);
        assert!(serde_json::from_value::<DurableSchedule>(value).is_err());

        let mut value = serde_json::to_value(schedule(
            ScheduleRestartPolicy::Resume,
            ScheduleCatchUpOrder::OldestFirst,
        ))
        .unwrap();
        value["extra"] = json!(true);
        assert!(serde_json::from_value::<DurableSchedule>(value).is_err());

        let valid_schedule = schedule(
            ScheduleRestartPolicy::Resume,
            ScheduleCatchUpOrder::OldestFirst,
        );
        let descriptor =
            durable_schedule_state_family_descriptor(StateFamilyId::new("schedules").unwrap())
                .unwrap();
        assert_eq!(descriptor.schema_version(), DurableSchedule::SCHEMA_VERSION);
        assert_eq!(
            DurableScheduleStateCodec
                .encode_key(&valid_schedule.id())
                .unwrap(),
            valid_schedule.id().canonical_hash().to_hex()
        );
        let encoded = DurableScheduleStateCodec
            .serialize_value(&valid_schedule)
            .unwrap();
        assert_eq!(
            DurableScheduleStateCodec.decode_value(&encoded).unwrap(),
            valid_schedule
        );
    }

    #[test]
    fn incomparable_observation_and_stale_revision_fail_without_a_plan() {
        let schedule = schedule(
            ScheduleRestartPolicy::Resume,
            ScheduleCatchUpOrder::OldestFirst,
        );
        assert!(matches!(
            schedule.plan_due(2, &stamp(10), ScheduleDriveCause::Live),
            Err(DurableScheduleError::StaleRevision { .. })
        ));
        assert!(matches!(
            schedule.plan_due(
                1,
                &ClockStamp {
                    clock: ClockRef::Utc,
                    at: 10,
                },
                ScheduleDriveCause::Live,
            ),
            Err(DurableScheduleError::Clock(_))
        ));
    }

    #[test]
    fn final_representable_tick_exhausts_instead_of_wrapping() {
        let owner = OwnerId::new("schedule_test").unwrap();
        let schedule = DurableSchedule::new(DurableScheduleDraft {
            id: ScheduleId::derive(&owner, &CanonicalId::new("edge").unwrap()).unwrap(),
            owner,
            clock: ClockRef::Utc,
            next_due: ClockStamp {
                clock: ClockRef::Utc,
                at: i64::MAX,
            },
            cadence: Some(1),
            generation: 1,
            next_ordinal: 0,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: 1,
        })
        .unwrap();
        let advance = advance(
            schedule
                .plan_due(
                    1,
                    &ClockStamp {
                        clock: ClockRef::Utc,
                        at: i64::MAX,
                    },
                    ScheduleDriveCause::Live,
                )
                .unwrap(),
        );
        assert_eq!(advance.occurrences().len(), 1);
        assert_eq!(advance.post_image().state(), ScheduleState::ClockExhausted);
    }
}
