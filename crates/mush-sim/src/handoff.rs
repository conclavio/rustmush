//! Authoritative production path for rules-neutral control handoffs.
//!
//! Campaign code chooses the next holder, reason, and safe boundary. This
//! module commits that choice as one pending lease plus one reusable durable
//! schedule per `(subject, channel)`. The schedule driver later advances the
//! exact cursor and changes holder/epoch in the same authoritative bundle.

use std::collections::BTreeMap;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider,
    ActionStateError, ActionTxn, AuthoritativeActionOutput, CanonicalHash, CanonicalId, ChannelId,
    ClockStamp, ControlHandoffDraft, ControlLeaseError, ControlLeaseKey, ControlLeaseState,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionValidationContext,
    DurableSchedule, DurableScheduleDraft, DurableScheduleError, DurableScheduleStateCodec, Engine,
    LegalActionQuery, ModError, ObjId, OwnerId, PerceivedRef, PrincipalId, Registrar, RequestUid,
    RevisionSet, RulesValue, ScheduleActionAdapter, ScheduleActionDraft, ScheduleAdvance,
    ScheduleCatchUpOrder, ScheduleEvaluation, ScheduleId, ScheduleRestartPolicy, ScheduleState,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey,
    canonical_json_bytes_with_limit, durable_schedule_state_family_descriptor,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::NS;
use crate::data::{SpatialState, StationId};
use crate::focus::FocusState;

const REQUEST_ACTION_ID: &str = "control_handoff_request";
const CANCEL_ACTION_ID: &str = "control_handoff_cancel";
const APPLY_ACTION_ID: &str = "control_handoff_apply";
const OUTCOME_ID: &str = "control_handoff_result";
const SCHEDULE_FAMILY: &str = "control_handoff_schedules";
const JOB_FAMILY: &str = "control_handoff_jobs";
const SCHEDULE_SOURCE: &str = "control_handoff";
const MAX_HANDOFF_JOB_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct HandoffJob {
    owner: OwnerId,
    schedule: ScheduleId,
    lease: ControlLeaseKey,
    request: RequestUid,
    pending_revision: u64,
    boundary: ClockStamp,
    // E7c5 B5: the requester's station-grant witness (absent for a pure K6 handoff).
    #[serde(default)]
    witness: Option<GrantWitness>,
}

impl HandoffJob {
    fn new(
        owner: OwnerId,
        lease: ControlLeaseKey,
        request: RequestUid,
        pending_revision: u64,
        boundary: ClockStamp,
        witness: Option<GrantWitness>,
    ) -> Result<Self, String> {
        let schedule = handoff_schedule_id(&owner, &lease)?;
        let job = Self {
            owner,
            schedule,
            lease,
            request,
            pending_revision,
            boundary,
            witness,
        };
        job.validate()?;
        Ok(job)
    }

    fn validate(&self) -> Result<(), String> {
        if self.pending_revision == 0 {
            return Err("control handoff job revision must be non-zero".to_owned());
        }
        if handoff_schedule_id(&self.owner, &self.lease)? != self.schedule {
            return Err("control handoff job schedule identity mismatch".to_owned());
        }
        let value = serde_json::to_value(self).map_err(|error| error.to_string())?;
        canonical_json_bytes_with_limit(&value, MAX_HANDOFF_JOB_BYTES)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn handoff_schedule_id(owner: &OwnerId, lease: &ControlLeaseKey) -> Result<ScheduleId, String> {
    let lease = serde_json::to_value(lease).map_err(|error| error.to_string())?;
    let local_hash =
        CanonicalHash::for_canonical_value("control_handoff_schedule_local.v1", 1, &lease, 1024)
            .map_err(|error| error.to_string())?;
    let local_id =
        CanonicalId::parse_canonical(local_hash.to_hex()).map_err(|error| error.to_string())?;
    ScheduleId::derive(owner, &local_id).map_err(|error| error.to_string())
}

/// The single control channel a station grants, or `None` when the station is
/// absent or grants a set that is not exactly one channel. Cut 5 leases are
/// per-channel; a multi-channel station is not offered through the command path.
fn station_channel(spatial: &SpatialState, unit: ObjId, station: &StationId) -> Option<ChannelId> {
    let spec = spatial.stations.get(&unit)?.stations.get(station)?;
    if spec.channels.len() != 1 {
        return None;
    }
    spec.channels.iter().next().cloned()
}

/// The declared fallback controller for a station (B6's `sim.station_fallback`),
/// or `None` when none is declared. Read from the world-owned RAM mirror.
fn declared_fallback(engine: &Engine, unit: ObjId, station: &StationId) -> Option<ObjId> {
    engine
        .world
        .module_state::<crate::station::StationFallbacks>()
        .and_then(|fallbacks| fallbacks.holder(unit, station))
}

/// The requester's current control-generation on `station` of `unit` (the exact
/// grant epoch `World::channel_grant_is_current` compares), or `None` when the
/// requester holds no such grant.
fn requester_grant_epoch(engine: &Engine, actor: ObjId, unit: ObjId, station: &str) -> Option<u64> {
    engine
        .world
        .command_subjects_of(actor)
        .iter()
        .find(|grant| grant.subject == unit && grant.station.as_str() == station)
        .map(|grant| grant.epoch)
}

#[derive(Clone)]
struct HandoffFamilies {
    leases: StateFamilyHandle<ControlLeaseKey, ControlLeaseState>,
    schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    jobs: StateFamilyHandle<ScheduleId, HandoffJob>,
}

#[derive(Clone, Copy, Debug)]
enum HandoffOperation {
    Request,
    Cancel,
    Apply,
}

#[derive(Clone)]
struct HandoffProvider {
    operation: HandoffOperation,
    outcome: DefinitionRef,
    families: HandoffFamilies,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestParameters {
    channel: ChannelId,
    next_holder: Option<PrincipalId>,
    requested_at: ClockStamp,
    boundary: ClockStamp,
    reason: CanonicalId,
    expected_revision: u64,
    // E7c5 B5 (candidate (iii)): the station-focus grant witness the offer pins as
    // FIXED parameters. The epoch is the requester's control-generation on the named
    // station; `HandoffScheduleAdapter::plan` re-checks it against
    // `World::channel_grant_is_current` at apply, so a focus change between request
    // and apply (which bumps the unit's control generation — B4) invalidates the
    // in-flight request. Absent for a pure K6 handoff that rides no station grant.
    #[serde(default)]
    station: Option<SmolStr>,
    #[serde(default)]
    grant_epoch: Option<u64>,
}

/// The requester's station-grant witness, carried on the durable job so the
/// scheduled apply re-checks that the grant that authorized the request is still
/// current. `subject`/`channel`/`owner` come from the lease key and the leases
/// owner; only `(actor, station, epoch)` are witness-specific.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct GrantWitness {
    actor: ObjId,
    station: SmolStr,
    epoch: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelParameters {
    channel: ChannelId,
    handoff: RequestUid,
    expected_revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyParameters {
    schedule: ScheduleId,
    lease: ControlLeaseKey,
    handoff: RequestUid,
    expected_revision: u64,
    observed_at: ClockStamp,
}

impl HandoffProvider {
    fn invalid(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::InvalidRequest, false, message)
            .expect("static control-handoff request error is bounded")
    }

    fn integrity(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::Integrity, false, message)
            .expect("static control-handoff integrity error is bounded")
    }

    fn stale(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::Stale, false, message)
            .expect("static control-handoff stale error is bounded")
    }

    fn state_error(_error: &ActionStateError) -> ActionError {
        Self::integrity("control-handoff authoritative state is unavailable")
    }

    fn schedule_error(_error: &DurableScheduleError) -> ActionError {
        Self::invalid("control-handoff schedule parameters are invalid")
    }

    fn transition_error(error: &ControlLeaseError) -> ActionError {
        let code = match error {
            ControlLeaseError::StaleRevision { .. }
            | ControlLeaseError::NoPending
            | ControlLeaseError::WrongPending { .. }
            | ControlLeaseError::DuplicatePending { .. }
            | ControlLeaseError::PendingExists { .. }
            | ControlLeaseError::NoControlChange => ActionErrorCode::Stale,
            ControlLeaseError::BeforeBoundary
            | ControlLeaseError::PendingStateMismatch
            | ControlLeaseError::Clock(_)
            | ControlLeaseError::RevisionBehindEpoch { .. }
            | ControlLeaseError::InvalidInitialState
            | ControlLeaseError::MissingTransitionWitness
            | ControlLeaseError::UnsupportedSchema { .. }
            | ControlLeaseError::Encoding(_) => ActionErrorCode::Integrity,
            ControlLeaseError::BoundaryBeforeRequest => ActionErrorCode::InvalidRequest,
            ControlLeaseError::EpochExhausted | ControlLeaseError::RevisionExhausted => {
                ActionErrorCode::BoundsExceeded
            }
        };
        ActionError::new(code, false, "control-handoff transition was refused")
            .expect("static control-handoff transition error is bounded")
    }

    fn subject(invocation: &ActionInvocation) -> Result<mush_core::ObjId, ActionError> {
        if !invocation.targets().is_empty() {
            return Err(Self::invalid("control handoff does not accept targets"));
        }
        match invocation.subject() {
            PerceivedRef::SelfActor => Ok(invocation.actor()),
            PerceivedRef::KnownObject { id, .. } => Ok(*id),
            PerceivedRef::Contact { .. } => Err(Self::invalid(
                "control handoff requires an exact object subject",
            )),
        }
    }

    fn decode<T: for<'de> Deserialize<'de>>(
        invocation: &ActionInvocation,
    ) -> Result<T, ActionError> {
        serde_json::from_value(invocation.parameters().clone())
            .map_err(|_| Self::invalid("control-handoff parameters are malformed"))
    }

    fn require_scheduler(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let PrincipalId::Scoped { owner, class, .. } = invocation.principal() else {
            return Err(Self::invalid(
                "control-handoff apply requires the durable scheduler",
            ));
        };
        if owner != self.families.leases.owner() || class.as_str() != "scheduler" {
            return Err(Self::invalid(
                "control-handoff apply requires the owning durable scheduler",
            ));
        }
        Ok(())
    }

    fn validate(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        Self::subject(invocation)?;
        match self.operation {
            HandoffOperation::Request => {
                let _: RequestParameters = Self::decode(invocation)?;
            }
            HandoffOperation::Cancel => {
                let _: CancelParameters = Self::decode(invocation)?;
            }
            HandoffOperation::Apply => {
                self.require_scheduler(invocation)?;
                let _: ApplyParameters = Self::decode(invocation)?;
            }
        }
        Ok(())
    }

    fn outcome(
        &self,
        operation: &'static str,
        lease: &ControlLeaseState,
        handoff: RequestUid,
    ) -> Result<RulesValue, ActionError> {
        RulesValue::new(
            self.outcome.clone(),
            json!({
                "operation": operation,
                "subject": lease.key().subject().0,
                "channel": lease.key().channel(),
                "handoff": handoff,
                "holder": lease.holder(),
                "epoch": lease.epoch(),
                "revision": lease.revision(),
            }),
        )
        .map_err(|_| Self::integrity("control-handoff outcome is invalid"))
    }

    fn witnesses(invocation: &ActionInvocation, lease: &ControlLeaseState) -> Vec<PrincipalId> {
        let mut witnesses = vec![invocation.principal().clone()];
        if let Some(holder) = lease.holder() {
            witnesses.push(holder.clone());
        }
        if let Some(pending) = lease.pending() {
            witnesses.push(pending.requested_by().clone());
            witnesses.extend(pending.from().cloned());
            witnesses.extend(pending.to().cloned());
        }
        witnesses.sort();
        witnesses.dedup();
        witnesses
    }

    fn prepare_request(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters: RequestParameters = Self::decode(invocation)?;
        let key = ControlLeaseKey::new(Self::subject(invocation)?, parameters.channel);
        let current = txn
            .get(&self.families.leases, &key)
            .map_err(|error| Self::state_error(&error))?
            .cloned();
        let lease = current
            .clone()
            .map_or_else(|| ControlLeaseState::new(key.clone()), Ok)
            .map_err(|error| Self::transition_error(&error))?;
        let next = lease
            .checked_request_handoff(
                parameters.expected_revision,
                ControlHandoffDraft {
                    request: invocation.request(),
                    requested_by: invocation.principal().clone(),
                    next_holder: parameters.next_holder,
                    requested_at: parameters.requested_at,
                    boundary: parameters.boundary.clone(),
                    reason: parameters.reason,
                },
            )
            .map_err(|error| Self::transition_error(&error))?;
        let witness = match (parameters.station.clone(), parameters.grant_epoch) {
            (Some(station), Some(epoch)) => Some(GrantWitness {
                actor: invocation.actor(),
                station,
                epoch,
            }),
            _ => None,
        };
        let job = HandoffJob::new(
            self.families.leases.owner().clone(),
            key.clone(),
            invocation.request(),
            next.revision(),
            parameters.boundary.clone(),
            witness,
        )
        .map_err(|_| Self::integrity("control-handoff job identity is invalid"))?;
        let schedule = DurableSchedule::new(DurableScheduleDraft {
            id: job.schedule,
            owner: self.families.leases.owner().clone(),
            clock: parameters.boundary.clock.clone(),
            next_due: parameters.boundary,
            cadence: None,
            generation: next.revision(),
            next_ordinal: 0,
            restart: ScheduleRestartPolicy::Resume,
            catch_up_order: ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: 1,
        })
        .map_err(|error| Self::schedule_error(&error))?;
        let prior_schedule = txn
            .get(&self.families.schedules, &job.schedule)
            .map_err(|error| Self::state_error(&error))?
            .cloned();
        if prior_schedule.as_ref().is_some_and(|prior| {
            prior.state() != ScheduleState::Completed
                || prior.id() != job.schedule
                || prior.owner() != self.families.leases.owner()
        }) || txn
            .get(&self.families.jobs, &job.schedule)
            .map_err(|error| Self::state_error(&error))?
            .is_some()
        {
            return Err(Self::integrity(
                "control-handoff reusable schedule is still occupied",
            ));
        }

        if current.is_some() {
            txn.replace(&self.families.leases, &key, next.clone())
        } else {
            txn.insert(&self.families.leases, &key, next.clone())
        }
        .map_err(|error| Self::state_error(&error))?;
        if prior_schedule.is_some() {
            txn.replace(&self.families.schedules, &job.schedule, schedule)
        } else {
            txn.insert(&self.families.schedules, &job.schedule, schedule)
        }
        .map_err(|error| Self::state_error(&error))?;
        let schedule_id = job.schedule;
        txn.insert(&self.families.jobs, &schedule_id, job)
            .map_err(|error| Self::state_error(&error))?;

        Ok(AuthoritativeActionOutput::new(self.outcome(
            "requested",
            &next,
            invocation.request(),
        )?)
        .with_eligible_witnesses(Self::witnesses(invocation, &next)))
    }

    fn prepare_cancel(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters: CancelParameters = Self::decode(invocation)?;
        let key = ControlLeaseKey::new(Self::subject(invocation)?, parameters.channel);
        let lease = txn
            .get(&self.families.leases, &key)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
            .ok_or_else(|| Self::invalid("control lease does not exist"))?;
        let pending = lease
            .pending()
            .ok_or_else(|| Self::invalid("control handoff is not pending"))?;
        let schedule = handoff_schedule_id(self.families.leases.owner(), &key)
            .map_err(|_| Self::integrity("control-handoff schedule identity is invalid"))?;
        let job = txn
            .get(&self.families.jobs, &schedule)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
            .ok_or_else(|| Self::integrity("pending control handoff has no durable job"))?;
        let scheduled = txn
            .get(&self.families.schedules, &schedule)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
            .ok_or_else(|| Self::integrity("pending control handoff has no durable schedule"))?;
        if job.owner != *self.families.leases.owner()
            || job.schedule != schedule
            || job.lease != key
            || job.request != pending.request()
            || job.pending_revision != lease.revision()
            || job.boundary != *pending.boundary()
            || scheduled.id() != schedule
            || scheduled.owner() != self.families.leases.owner()
            || scheduled.next_due() != &job.boundary
            || scheduled.generation() != job.pending_revision
            || scheduled.state() != ScheduleState::Active
        {
            return Err(Self::integrity(
                "pending control handoff and durable schedule disagree",
            ));
        }
        let witnesses = Self::witnesses(invocation, &lease);
        let next = lease
            .checked_cancel_handoff(parameters.expected_revision, parameters.handoff)
            .map_err(|error| Self::transition_error(&error))?;
        txn.replace(&self.families.leases, &key, next.clone())
            .map_err(|error| Self::state_error(&error))?;
        txn.remove(&self.families.schedules, &schedule)
            .map_err(|error| Self::state_error(&error))?;
        txn.remove(&self.families.jobs, &schedule)
            .map_err(|error| Self::state_error(&error))?;
        Ok(
            AuthoritativeActionOutput::new(self.outcome("cancelled", &next, parameters.handoff)?)
                .with_eligible_witnesses(witnesses),
        )
    }

    fn prepare_apply(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.require_scheduler(invocation)?;
        let parameters: ApplyParameters = Self::decode(invocation)?;
        if Self::subject(invocation)? != parameters.lease.subject()
            || handoff_schedule_id(self.families.leases.owner(), &parameters.lease)
                .map_err(|_| Self::integrity("control-handoff schedule identity is invalid"))?
                != parameters.schedule
        {
            return Err(Self::integrity(
                "scheduled control-handoff addressing is inconsistent",
            ));
        }
        let job = txn
            .get(&self.families.jobs, &parameters.schedule)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
            .ok_or_else(|| Self::integrity("scheduled control handoff has no durable job"))?;
        let schedule = txn
            .get(&self.families.schedules, &parameters.schedule)
            .map_err(|error| Self::state_error(&error))?
            .ok_or_else(|| Self::integrity("scheduled control handoff cursor is missing"))?;
        if job.owner != *self.families.leases.owner()
            || job.schedule != parameters.schedule
            || job.lease != parameters.lease
            || job.request != parameters.handoff
            || job.pending_revision != parameters.expected_revision
            || job.boundary.checked_cmp(&parameters.observed_at).is_err()
            || schedule.id() != parameters.schedule
            || schedule.owner() != self.families.leases.owner()
            || schedule.next_due() != &job.boundary
            || schedule.generation() != job.pending_revision
            || schedule.state() != ScheduleState::Completed
        {
            return Err(Self::integrity(
                "scheduled control handoff and durable job disagree",
            ));
        }
        let lease = txn
            .get(&self.families.leases, &parameters.lease)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
            .ok_or_else(|| Self::integrity("scheduled control lease is missing"))?;
        let witnesses = Self::witnesses(invocation, &lease);
        let next = lease
            .checked_apply_handoff(
                parameters.expected_revision,
                parameters.handoff,
                parameters.observed_at,
            )
            .map_err(|error| Self::transition_error(&error))?;
        txn.replace(&self.families.leases, &parameters.lease, next.clone())
            .map_err(|error| Self::state_error(&error))?;
        txn.remove(&self.families.jobs, &parameters.schedule)
            .map_err(|error| Self::state_error(&error))?;
        Ok(
            AuthoritativeActionOutput::new(self.outcome("applied", &next, parameters.handoff)?)
                .with_eligible_witnesses(witnesses),
        )
    }

    /// E7c5 B5 (candidate (iii)): project the request/cancel ingress for one
    /// crew member's focused station. The offer declares **`required_channels =
    /// []`** — so the kernel's `project_legal_actions` never runs the
    /// `ControlLease` witness loop and drops it, which is what lets a FIRST
    /// seat-taking request on a vacant lease ride the command path. The authority
    /// split lives here (the action's own guard): a focused billet-holder may
    /// establish a vacant station's lease; a non-holder cannot relieve one; the
    /// control-generation epoch is pinned as a FIXED parameter and re-checked at
    /// apply. Staff and relief-to-another-holder are the direct/kernel path in cut
    /// 5 (see the module note). Apply is scheduler-driven and never offered.
    fn project_station_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        let request = match self.operation {
            HandoffOperation::Request => true,
            HandoffOperation::Cancel => false,
            HandoffOperation::Apply => return Ok(None),
        };
        let SubjectKey::Object { id: unit } = query.truth_subject() else {
            return Ok(None);
        };
        let unit = *unit;
        let actor = query.actor();
        let Some(focus) = engine
            .world
            .module_state::<FocusState>()
            .and_then(|state| state.focus_of_actor(actor))
            .filter(|focus| focus.unit == unit)
        else {
            return Ok(None);
        };
        let station = focus.station.clone();
        let Some(spatial) = engine.world.module_state::<SpatialState>() else {
            return Ok(None);
        };
        let Some(channel) = station_channel(spatial, unit, &station) else {
            return Ok(None);
        };
        let Some(epoch) = requester_grant_epoch(engine, actor, unit, station.as_str()) else {
            return Ok(None);
        };
        let key = ControlLeaseKey::new(unit, channel.clone());
        let lease = engine
            .action_state()
            .get(&self.families.leases, &key)
            .map_err(|error| Self::state_error(&error))?;
        let revision = lease.map_or(0, ControlLeaseState::revision);
        let actor_principal = PrincipalId::Object { id: actor };
        let now = engine.world.world_ic_now();
        let fixed = if request {
            // The authority split, by lease state (the create/relief split of the
            // OWNER DECISION):
            //   * VACANT  -> CREATE: a focused billet-holder establishes the lease
            //     for themselves (cut 5's first-production-creator path).
            //   * HELD by self -> RELIEF to the declared fallback controller (if one
            //     is declared and differs from self); this is the held-relief case
            //     candidate (iii) must also project, and the immediate controller's
            //     path.
            //   * HELD by another -> a non-holder cannot relieve; no offer.
            let next_holder = match lease.and_then(ControlLeaseState::holder) {
                None => actor_principal.clone(),
                Some(holder) if *holder == actor_principal => {
                    let Some(fallback) = declared_fallback(engine, unit, &station) else {
                        return Ok(None);
                    };
                    let next = PrincipalId::Object { id: fallback };
                    if next == actor_principal {
                        return Ok(None);
                    }
                    next
                }
                Some(_) => return Ok(None),
            };
            json!({
                "channel": channel,
                "next_holder": next_holder,
                "requested_at": now,
                "boundary": now,
                "reason": "focus.station_seat",
                "expected_revision": revision,
                "station": station.as_str(),
                "grant_epoch": epoch,
            })
        } else {
            // Cancel is offered only while the actor's own request is still pending.
            let Some(pending) = lease.and_then(ControlLeaseState::pending) else {
                return Ok(None);
            };
            if pending.requested_by() != &actor_principal {
                return Ok(None);
            }
            json!({
                "channel": channel,
                "handoff": pending.request(),
                "expected_revision": revision,
            })
        };
        self.offer_draft(action, engine, fixed).map(Some)
    }

    /// Build the request/cancel offer: `required_channels = []`, no free
    /// parameters (the command supplies none — the offer pins every coordinate),
    /// the mandatory definition guard, and a bounded presentation.
    fn offer_draft(
        &self,
        action: &DefinitionRef,
        engine: &Engine,
        fixed_parameters: Value,
    ) -> Result<mush_core::ProviderOfferDraft, ActionError> {
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| Self::integrity("control-handoff definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| Self::integrity("control-handoff guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| Self::integrity("control-handoff definition guard is unavailable"))?;
        let (verb, label, help) = match self.operation {
            HandoffOperation::Request => (
                "station_handoff",
                "Establish station control",
                "Take the control lease for your focused station.",
            ),
            HandoffOperation::Cancel => (
                "station_handoff_cancel",
                "Cancel a pending station handoff",
                "Withdraw your still-pending station control handoff.",
            ),
            HandoffOperation::Apply => {
                return Err(Self::invalid("control-handoff apply is never offered"));
            }
        };
        Ok(mush_core::ProviderOfferDraft {
            fixed_parameters,
            parameters: mush_core::ProjectedParameterSchema::empty(),
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(verb, label, help)
                .map_err(|_| Self::integrity("control-handoff presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| Self::integrity("control-handoff offer guards are invalid"))?,
        })
    }
}

impl ActionProvider for HandoffProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        self.validate(invocation)
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        self.project_station_offer(action, query, engine)
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "control handoffs require the authoritative action coordinator",
        )
        .expect("static control-handoff migration error is bounded"))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate(invocation)?;
        match self.operation {
            HandoffOperation::Request => self.prepare_request(invocation, txn),
            HandoffOperation::Cancel => self.prepare_cancel(invocation, txn),
            HandoffOperation::Apply => self.prepare_apply(invocation, txn),
        }
    }
}

#[derive(Clone)]
struct HandoffScheduleAdapter {
    schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    jobs: StateFamilyHandle<ScheduleId, HandoffJob>,
}

impl ScheduleActionAdapter for HandoffScheduleAdapter {
    fn plan(
        &self,
        action: &DefinitionRef,
        advance: &ScheduleAdvance,
        engine: &Engine,
    ) -> Result<ScheduleActionDraft, ActionError> {
        let current = engine
            .action_state()
            .get(&self.schedules, &advance.schedule())
            .map_err(|_| HandoffProvider::integrity("control-handoff schedule is unavailable"))?;
        let candidate_is_current = current.is_some_and(|schedule| {
            matches!(
                schedule.plan_due(
                    schedule.revision(),
                    advance.observed_at(),
                    advance.cause(),
                ),
                Ok(ScheduleEvaluation::Advance(planned)) if planned.as_ref() == advance
            )
        });
        if !candidate_is_current {
            return Err(HandoffProvider::stale(
                "control-handoff schedule candidate is no longer current",
            ));
        }
        let job = engine
            .action_state()
            .get(&self.jobs, &advance.schedule())
            .map_err(|_| HandoffProvider::integrity("control-handoff job is unavailable"))?
            .ok_or_else(|| HandoffProvider::integrity("active control-handoff job is missing"))?;
        if job.schedule != advance.schedule()
            || job.owner != *advance.owner()
            || job.pending_revision != advance.generation()
            || advance.occurrences().len() != 1
            || advance.skipped().is_some()
            || advance.occurrences()[0].due() != &job.boundary
            || advance.post_image().state() != ScheduleState::Completed
        {
            return Err(HandoffProvider::integrity(
                "active control-handoff schedule and job disagree",
            ));
        }
        // E7c5 B5 (candidate (iii)): the epoch binding is enforced HERE, at apply.
        // The request pinned the requester's control-generation on the named station
        // as a fixed parameter; re-check it against the kernel's existing
        // `World::channel_grant_is_current`. A focus change between request and apply
        // bumps the unit's control generation (B4's `apply_control_generation`), so
        // the witness is stale and the in-flight handoff is refused rather than
        // applied under authority that has since moved.
        if let Some(witness) = &job.witness
            && !engine.world.channel_grant_is_current(
                witness.actor,
                job.lease.subject(),
                job.owner.as_str(),
                witness.station.as_str(),
                witness.epoch,
                job.lease.channel(),
            )
        {
            return Err(HandoffProvider::stale(
                "control-handoff requester's station grant is no longer current",
            ));
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            HandoffProvider::integrity("control-handoff definition registry is unavailable")
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                HandoffProvider::integrity("control-handoff guard registry is unavailable")
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                HandoffProvider::integrity("control-handoff definition guard is unavailable")
            })?;
        Ok(ScheduleActionDraft {
            actor: job.lease.subject(),
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "schedule": job.schedule,
                "lease": job.lease,
                "handoff": job.request,
                "expected_revision": job.pending_revision,
                "observed_at": advance.observed_at(),
            }),
            guards: guards
                .finish()
                .map_err(|_| HandoffProvider::integrity("control-handoff guards are invalid"))?,
        })
    }
}

struct HandoffDescriptors {
    outcome: DefinitionDescriptor,
    request: DefinitionDescriptor,
    cancel: DefinitionDescriptor,
    apply: DefinitionDescriptor,
}

fn definition_descriptor(
    kind: &str,
    id: &str,
    payload: Value,
) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
        DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
        1,
        payload,
    )
    .map_err(|error| ModError::msg(error.to_string()))
}

fn handoff_descriptors() -> Result<HandoffDescriptors, ModError> {
    let outcome = definition_descriptor(
        "outcome",
        OUTCOME_ID,
        json!({
            "fields": {
                "operation": "requested|cancelled|applied",
                "subject": "obj_id",
                "channel": "channel_id",
                "handoff": "request_uid",
                "holder": "principal_id|null",
                "epoch": "u64",
                "revision": "u64"
            }
        }),
    )?
    .as_current();
    let owner = OwnerId::new(NS).map_err(|error| ModError::msg(error.to_string()))?;
    let outcome_ref = mush_core::DefinitionRegistryBuilder::new()
        .register(owner, outcome.clone())
        .map_err(|error| ModError::msg(error.to_string()))?;
    let action = |id: &str, operation: &str| {
        definition_descriptor(
            "action",
            id,
            json!({
                "internal": true,
                "operation": operation,
                "subject": "object",
                "targets": 0,
                "state": ["control_leases", SCHEDULE_FAMILY, JOB_FAMILY]
            }),
        )
        .map(|descriptor| {
            descriptor
                .with_references(vec![outcome_ref.clone()])
                .as_current()
        })
    };
    Ok(HandoffDescriptors {
        outcome,
        request: action(REQUEST_ACTION_ID, "request")?,
        cancel: action(CANCEL_ACTION_ID, "cancel")?,
        apply: action(APPLY_ACTION_ID, "apply")?,
    })
}

#[derive(Clone)]
struct ExpectedDefinition {
    payload: Value,
    references: Vec<DefinitionRef>,
    current: bool,
}

fn exact_definition_validator(
    descriptors: &[DefinitionDescriptor],
) -> impl Fn(&DefinitionValidationContext<'_>) -> Result<(), String> + Send + Sync + 'static {
    let expected: BTreeMap<_, _> = descriptors
        .iter()
        .map(|descriptor| {
            let mut references = descriptor.references().to_vec();
            references.sort();
            (
                (descriptor.id().clone(), descriptor.version()),
                ExpectedDefinition {
                    payload: descriptor.payload().clone(),
                    references,
                    current: descriptor.is_current(),
                },
            )
        })
        .collect();
    move |context: &DefinitionValidationContext<'_>| {
        let definition = context.definition();
        let key = (
            definition.reference().key().id.clone(),
            definition.reference().version(),
        );
        let Some(shape) = expected.get(&key) else {
            return Err("unknown control-handoff definition".to_owned());
        };
        if definition.payload() != &shape.payload
            || definition.references() != shape.references
            || definition.is_current() != shape.current
        {
            return Err("control-handoff definition shape mismatch".to_owned());
        }
        Ok(())
    }
}

/// The registered station-handoff request action, for cut 5's `@station`
/// command ingress (which selects it through `Ctx::submit_authoritative`).
pub(crate) fn request_action_ref(registry: &mush_core::Registry) -> Option<DefinitionRef> {
    action_ref(registry, REQUEST_ACTION_ID)
}

/// The registered station-handoff cancel action.
pub(crate) fn cancel_action_ref(registry: &mush_core::Registry) -> Option<DefinitionRef> {
    action_ref(registry, CANCEL_ACTION_ID)
}

fn action_ref(registry: &mush_core::Registry, id: &str) -> Option<DefinitionRef> {
    let definitions = registry.definitions()?;
    let owner = OwnerId::new(NS).ok()?;
    let kind = DefinitionKind::new("action").ok()?;
    let id = DefinitionId::new(id).ok()?;
    definitions
        .current(&mush_core::DefinitionKey { owner, kind, id })
        .cloned()
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    leases: StateFamilyHandle<ControlLeaseKey, ControlLeaseState>,
) -> Result<(), ModError> {
    let schedules = reg.state_family(
        durable_schedule_state_family_descriptor(
            StateFamilyId::new(SCHEDULE_FAMILY)
                .map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
        DurableScheduleStateCodec,
    )?;
    let jobs = reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(JOB_FAMILY).map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("durable_schedule_id.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.control_handoff_job.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            1,
            CanonicalId::new("sim.control_handoff_job.decode.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.control_handoff_job.validate.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("sim.control_handoff_job.json.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?
        .with_limits(64, MAX_HANDOFF_JOB_BYTES)
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ScheduleId| Ok(key.canonical_hash().to_hex()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
        HandoffJob::validate,
        |value: &HandoffJob| serde_json::to_value(value).map_err(|error| error.to_string()),
    )?;

    let descriptors = handoff_descriptors()?;
    reg.definition_validator(
        DefinitionKind::new("outcome").map_err(|error| ModError::msg(error.to_string()))?,
        exact_definition_validator(std::slice::from_ref(&descriptors.outcome)),
    )?;
    reg.definition_validator(
        DefinitionKind::new("action").map_err(|error| ModError::msg(error.to_string()))?,
        exact_definition_validator(&[
            descriptors.request.clone(),
            descriptors.cancel.clone(),
            descriptors.apply.clone(),
        ]),
    )?;
    let outcome = reg.definition(descriptors.outcome)?;
    let request = reg.definition(descriptors.request)?;
    let cancel = reg.definition(descriptors.cancel)?;
    let apply = reg.definition(descriptors.apply)?;
    let families = HandoffFamilies {
        leases,
        schedules: schedules.clone(),
        jobs: jobs.clone(),
    };
    for (action, operation) in [
        (request, HandoffOperation::Request),
        (cancel, HandoffOperation::Cancel),
        (apply.clone(), HandoffOperation::Apply),
    ] {
        reg.action_provider(
            action,
            HandoffProvider {
                operation,
                outcome: outcome.clone(),
                families: families.clone(),
            },
        )?;
    }
    reg.schedule_source(
        CanonicalId::new(SCHEDULE_SOURCE).map_err(|error| ModError::msg(error.to_string()))?,
        apply,
        schedules.clone(),
        HandoffScheduleAdapter { schedules, jobs },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{
        ActionInvocationDraft, ActionOfferId, CommitToken, IdempotencyKey, IdempotencyScope,
        IngressEnvelope, ObjId, Registry, ScheduleDriveCause, ScopedIdempotencyKey, World,
        scheduled_action_ingress,
    };

    use crate::SimulationModule;

    fn definition_refs() -> (DefinitionRef, DefinitionRef, DefinitionRef) {
        let descriptors = handoff_descriptors().unwrap();
        let owner = OwnerId::new(NS).unwrap();
        let mut definitions = mush_core::DefinitionRegistryBuilder::new();
        let outcome = definitions
            .register(owner.clone(), descriptors.outcome)
            .unwrap();
        let request = definitions
            .register(owner.clone(), descriptors.request)
            .unwrap();
        let cancel = definitions
            .register(owner.clone(), descriptors.cancel)
            .unwrap();
        let apply = definitions.register(owner, descriptors.apply).unwrap();
        assert_eq!(request.key().owner, outcome.key().owner);
        (request, cancel, apply)
    }

    fn invocation(
        action: &DefinitionRef,
        request: RequestUid,
        subject: ObjId,
        parameters: Value,
    ) -> ActionInvocation {
        let principal = PrincipalId::Object { id: ObjId(1) };
        ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: ObjId(1),
            offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
            subject: PerceivedRef::KnownObject {
                id: subject,
                revision: 0,
            },
            targets: Vec::new(),
            parameters,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: action.key().owner.clone(),
                    namespace: CanonicalId::new(action.key().id.as_str()).unwrap(),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap()
    }

    fn install(engine: &mut Engine, prepared: mush_core::PreparedTxn) -> Vec<mush_core::ModuleOp> {
        let (ops, installation) = prepared.into_parts();
        engine.install_prepared_action(installation);
        ops
    }

    fn lease_value(ops: &[mush_core::ModuleOp]) -> Value {
        ops.iter()
            .find_map(|op| match &op.op {
                mush_core::ModuleWrite::JsonSet { key, value }
                    if key.contains("/control_leases/") =>
                {
                    value.get("value").cloned()
                }
                _ => None,
            })
            .expect("transaction carries a control-lease post-image")
    }

    fn prepare(
        engine: &mut Engine,
        action: &DefinitionRef,
        invocation: &ActionInvocation,
        token: u8,
    ) -> (
        mush_core::PreparedTxn,
        mush_core::PreparedAuthoritativeAction,
    ) {
        engine
            .prepare_registered_authoritative_action(
                action,
                invocation,
                CommitToken::from_bytes([token; 16]),
            )
            .unwrap()
    }

    #[test]
    fn request_schedule_apply_reuse_and_cancel_are_atomic() {
        let mut world = World::new_minimal();
        world.set_now(100);
        let subject = world
            .create(mush_core::ObjKind::Thing, "test vehicle", ObjId(1), None)
            .unwrap();
        let mut engine = Engine::with_modules(
            world,
            Registry::new(),
            vec![Box::new(SimulationModule::new())],
        )
        .unwrap();
        let (request_action, cancel_action, apply_action) = definition_refs();
        let channel = ChannelId::new("movement").unwrap();
        let first_request = RequestUid::from_bytes([1; 16]);
        let requested_at = ClockStamp {
            clock: mush_core::ClockRef::Utc,
            at: 99,
        };
        let boundary = ClockStamp {
            clock: mush_core::ClockRef::Utc,
            at: 101,
        };
        let request_invocation = invocation(
            &request_action,
            first_request,
            subject,
            json!({
                "channel": channel,
                "next_holder": PrincipalId::Object { id: ObjId(7) },
                "requested_at": requested_at,
                "boundary": boundary,
                "reason": "operator_assigned",
                "expected_revision": 0,
            }),
        );
        let error = engine
            .prepare_registered_action(&request_action, &request_invocation)
            .expect_err("control handoffs must not bypass the authoritative coordinator");
        assert!(matches!(
            error,
            mush_core::ActionProviderPrepareError::Provider { source, .. }
                if source.code() == ActionErrorCode::Unavailable
        ));
        let (prepared, _) = prepare(&mut engine, &request_action, &request_invocation, 1);
        assert_eq!(prepared.operation_count(), 3);
        let requested_ops = install(&mut engine, prepared);
        let requested = lease_value(&requested_ops);
        assert_eq!(requested["revision"], 1);
        assert_eq!(requested["epoch"], 0);
        assert!(requested["pending"].is_object());

        let page = engine
            .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
            .unwrap();
        assert!(page.candidates().is_empty());
        engine.world.set_now(101);
        let page = engine
            .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
            .unwrap();
        assert_eq!(page.candidates().len(), 1);
        let candidate = page.candidates()[0].clone();
        assert_eq!(candidate.action(), &apply_action);
        let ingress: IngressEnvelope =
            scheduled_action_ingress(candidate.action(), candidate.advance(), 1).unwrap();
        let plan = engine.plan_scheduled_action(&candidate, &ingress).unwrap();
        let (_, scheduled_invocation, _) = plan.into_parts();
        let (prepared, _) = engine
            .prepare_registered_scheduled_authoritative_action(
                &candidate,
                &scheduled_invocation,
                CommitToken::from_bytes([2; 16]),
            )
            .unwrap();
        assert_eq!(prepared.operation_count(), 3);
        let applied_ops = install(&mut engine, prepared);
        let applied = lease_value(&applied_ops);
        assert_eq!(applied["revision"], 2);
        assert_eq!(applied["epoch"], 1);
        assert_eq!(applied["holder"]["id"], 7);
        assert!(applied["pending"].is_null());
        assert!(
            engine
                .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
                .unwrap()
                .candidates()
                .is_empty()
        );

        engine.world.set_now(150);
        let second_request = RequestUid::from_bytes([3; 16]);
        let second_invocation = invocation(
            &request_action,
            second_request,
            subject,
            json!({
                "channel": ChannelId::new("movement").unwrap(),
                "next_holder": PrincipalId::Object { id: ObjId(8) },
                "requested_at": { "clock": { "kind": "utc" }, "at": 150 },
                "boundary": { "clock": { "kind": "utc" }, "at": 200 },
                "reason": "operator_relief",
                "expected_revision": 2,
            }),
        );
        let (prepared, _) = prepare(&mut engine, &request_action, &second_invocation, 3);
        assert_eq!(prepared.operation_count(), 3);
        install(&mut engine, prepared);
        engine.world.set_now(200);
        let stale_candidate = engine
            .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
            .unwrap()
            .candidates()[0]
            .clone();
        let stale_ingress =
            scheduled_action_ingress(stale_candidate.action(), stale_candidate.advance(), 1)
                .unwrap();
        let cancel = invocation(
            &cancel_action,
            RequestUid::from_bytes([4; 16]),
            subject,
            json!({
                "channel": ChannelId::new("movement").unwrap(),
                "handoff": second_request,
                "expected_revision": 3,
            }),
        );
        let (prepared, _) = prepare(&mut engine, &cancel_action, &cancel, 4);
        assert_eq!(prepared.operation_count(), 3);
        let cancelled_ops = install(&mut engine, prepared);
        let cancelled = lease_value(&cancelled_ops);
        assert_eq!(cancelled["revision"], 4);
        assert_eq!(cancelled["epoch"], 1);
        assert!(cancelled["pending"].is_null());
        let error = engine
            .plan_scheduled_action(&stale_candidate, &stale_ingress)
            .expect_err("cancelled handoff candidate must be stale");
        assert!(matches!(
            error,
            mush_core::ScheduleActionPlanError::Adapter { source, .. }
                if source.code() == ActionErrorCode::Stale
        ));
        engine.world.set_now(250);
        assert!(
            engine
                .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
                .unwrap()
                .candidates()
                .is_empty()
        );
    }
}
