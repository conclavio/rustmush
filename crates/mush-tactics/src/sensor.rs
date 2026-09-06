//! E7 cut 7 B3 — **the trusted sensor-policy ingress**, registered here and
//! nowhere else (10b §3.7 as amended by 08i answer 1; D-E7-36).
//!
//! ## Why this file is in `mush-tactics` and not in `mush-propagation`
//!
//! 10b §3.7 as drafted casts `mush-propagation` as the `Registrar::schedule_source`
//! registrant for observations. **The kernel forbids it.** Three fences, all in
//! the tree:
//!
//! 1. `ScheduleSourceRegistryBuilder::register` (`mush-core/src/schedule_source.rs`)
//!    refuses a registration whose schedule family owner or fired action owner is
//!    not the registering module (`ScheduleSourceRegistryError::OwnerMismatch`).
//! 2. `Registrar::schedule_source` (`mush-core/src/module.rs`) stamps the owner
//!    from the registering module — a module cannot register under another's id.
//! 3. `scheduled_action_ingress` (`mush-core/src/schedule_source.rs`) refuses a
//!    draft whose advance owner is not the fired action's owner
//!    (`ScheduleActionIngressError::OwnerMismatch`).
//!
//! The six `contact_*` actions are `mush-tactics`-owned, so a
//! `mush-propagation`-owned source **cannot fire them**. The ingress is therefore
//! tactics' — which is exactly the "trusted sensor-policy ingress" the contact
//! module header (`crate::contact`) has reserved since K6 — and the *policy* is
//! propagation's, handed in at composition through
//! [`TacticsModule::with_sensor_policy`](crate::TacticsModule::with_sensor_policy)
//! as an `Arc<dyn mush_core::SensorPolicy>`. `mush-tactics` gains **no**
//! `mush-propagation` edge: the trait is `mush-core`'s.
//!
//! This keeps the one binding constraint intact: **`mush-tactics` is the sole
//! `ContactState` authority.** Propagation registers no contact family, no
//! contact guard validator, and no schedule source that fires a contact action;
//! a cross-owner key would be refused by `ActionTxn::checked_key` even if it
//! tried.
//!
//! ## The two halves, and why there are two
//!
//! A durable schedule can be created **only** inside an authoritative bundle
//! (`ActionTxn`), and the six contact actions each write a contact — none of
//! them can bootstrap another's cursor. So the ingress is the `handoff.rs`
//! shape, in two halves:
//!
//! - **Arming** — [`SENSOR_ARM_ACTION_ID`], a tactics-owned authoritative action
//!   staged from a staff verb (`@prop/pass`). It takes a bounded, deterministic
//!   set of [`ObservationIntent`]s and writes, per intent, one **one-shot**
//!   `DurableSchedule` in that kind's own family plus one [`SensorJob`] carrying
//!   the intent. **It holds no `ContactState` handle at all** — it is
//!   structurally incapable of writing a contact, which is the probe.
//! - **Firing** — six `reg.schedule_source` registrations, one per
//!   [`ObservationKind`], each firing that kind's already-migrated `contact_*`
//!   provider. One source per family per owner: six kinds, six families, six
//!   ids, all disjoint from `control_handoff`, `production` and `npc`.
//!
//! `SensorScheduleAdapter::plan` re-derives the due observation from the
//! composition-supplied policy at fire time and prefers the policy's live
//! parameters when the policy still names the armed key; when it no longer does,
//! the durably armed intent stands (it was an authoritative decision) and the
//! kernel's own revision check decides whether it may still be applied. Nothing
//! here writes: mutation is exclusively the registered `ContactProvider`'s.

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider,
    ActionStateError, ActionTxn, AuthoritativeActionOutput, CanonicalHash, CanonicalId, ClockStamp,
    ContactId, ContactState, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DurableSchedule, DurableScheduleDraft, DurableScheduleStateCodec, Engine, LegalActionQuery,
    MAX_OBSERVATIONS_PER_PASS, ModError, ObjId, ObservationIntent, ObservationKind, PerceivedRef,
    ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft, Registrar, RevisionSet,
    RulesValue, ScheduleActionAdapter, ScheduleActionDraft, ScheduleAdvance, ScheduleCatchUpOrder,
    ScheduleEvaluation, ScheduleId, ScheduleRestartPolicy, ScheduleState, SensorPolicyHandle,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey,
    durable_schedule_state_family_descriptor,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The arming action: takes a bounded intent set and writes cursors only.
pub const SENSOR_ARM_ACTION_ID: &str = "sensor_arm";
/// The arming outcome.
pub const SENSOR_ARMED_OUTCOME_ID: &str = "sensor_armed";
/// The typed family holding one job per armed cursor.
pub const SENSOR_JOB_FAMILY: &str = "sensor_jobs";
/// The most canonical bytes one [`SensorJob`] may occupy.
pub const MAX_SENSOR_JOB_BYTES: usize = 8 * 1024;
/// The most bytes one `sensor_arm` intent document may carry on the wire.
///
/// **K's cap, cited not redefined**: an offered text parameter is bounded by
/// `mush_core::MAX_PROJECTED_ACTION_TEXT_BYTES`, so this is that figure and a
/// larger one would simply fail to build an offer. A set past it is **refused,
/// never truncated** (§4 rule 1).
pub const MAX_SENSOR_ARM_BYTES: usize = mush_core::MAX_PROJECTED_ACTION_TEXT_BYTES;

/// The most intents ONE arm may carry.
///
/// A compile-time relation, not a runtime assert: an arm can never carry more
/// than one pass.
const _: () = assert!(MAX_INTENTS_PER_ARM <= MAX_OBSERVATIONS_PER_PASS);

///
/// The per-*pass* bound is `mush_core::MAX_OBSERVATIONS_PER_PASS` (256, 10b §7);
/// this is the per-*arm* bound the 4 KiB offer parameter above actually admits.
/// A caller with more due work arms this many and says how many remain — it
/// never truncates silently and never splits into two submissions, because
/// `Ctx::submit_authoritative` twice in one command is a refusal by design.
pub const MAX_INTENTS_PER_ARM: usize = 16;

/// The six schedule families, one per kind — one source per family per owner.
#[must_use]
pub const fn sensor_schedule_family(kind: ObservationKind) -> &'static str {
    match kind {
        ObservationKind::Observe => "sensor_observe_schedules",
        ObservationKind::Refresh => "sensor_refresh_schedules",
        ObservationKind::Share => "sensor_share_schedules",
        ObservationKind::Unshare => "sensor_unshare_schedules",
        ObservationKind::Loss => "sensor_loss_schedules",
        ObservationKind::Reacquire => "sensor_reacquire_schedules",
    }
}

/// The six schedule-source ids. Deliberately prefixed `sensor_` so they are
/// **disjoint** from every other source on the tree (`control_handoff` in
/// `mush-sim`, `production` in `mush-industry`, `npc`) — one source id per owner.
#[must_use]
pub const fn sensor_source_id(kind: ObservationKind) -> &'static str {
    match kind {
        ObservationKind::Observe => "sensor_observe",
        ObservationKind::Refresh => "sensor_refresh",
        ObservationKind::Share => "sensor_share",
        ObservationKind::Unshare => "sensor_unshare",
        ObservationKind::Loss => "sensor_loss",
        ObservationKind::Reacquire => "sensor_reacquire",
    }
}

/// One armed observation, at the cursor that will fire it.
///
/// The durable half of the ingress: the schedule says *when*, this says *what*.
/// Its `TryFrom` wire re-runs the intent constructor, so an intent stored past a
/// cap or with a key that names another kind is **refused on load** (§4 rule 1).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SensorJob {
    schedule: ScheduleId,
    kind: ObservationKind,
    intent: ObservationIntent,
    /// Which `sensor_arm` armed it — the attribution a page needs.
    armed_by: ObjId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SensorJobWire {
    schedule: ScheduleId,
    kind: ObservationKind,
    intent: ObservationIntent,
    armed_by: ObjId,
}

impl<'de> Deserialize<'de> for SensorJob {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SensorJobWire::deserialize(deserializer)?;
        Self::new(wire.schedule, wire.intent, wire.armed_by)
            .map_err(serde::de::Error::custom)
            .and_then(|job| {
                if job.kind == wire.kind {
                    Ok(job)
                } else {
                    Err(serde::de::Error::custom(
                        "a sensor job's stored kind disagrees with its intent",
                    ))
                }
            })
    }
}

impl SensorJob {
    /// The one constructor; the wire re-runs it.
    ///
    /// # Errors
    /// A message when the intent's key does not name the kind the schedule was
    /// derived for — which cannot happen through [`Self::new`] itself, because
    /// the kind is *taken from* the intent, and is exactly what the wire check
    /// above exists to catch on a hand-edited document.
    pub fn new(
        schedule: ScheduleId,
        intent: ObservationIntent,
        armed_by: ObjId,
    ) -> Result<Self, String> {
        let kind = intent.kind();
        Ok(Self {
            schedule,
            kind,
            intent,
            armed_by,
        })
    }

    /// Which cursor fires it.
    #[must_use]
    pub const fn schedule(&self) -> ScheduleId {
        self.schedule
    }

    /// Which transition.
    #[must_use]
    pub const fn kind(&self) -> ObservationKind {
        self.kind
    }

    /// What was armed.
    #[must_use]
    pub const fn intent(&self) -> &ObservationIntent {
        &self.intent
    }

    /// Who armed it.
    #[must_use]
    pub const fn armed_by(&self) -> ObjId {
        self.armed_by
    }

    /// The family validator: the shape is already guaranteed by the wire, so
    /// this re-checks only what a serializer could still get wrong.
    ///
    /// # Errors
    /// A message when the stored kind and the intent's kind disagree.
    pub fn validate(&self) -> Result<(), String> {
        if self.kind != self.intent.kind() {
            return Err("a sensor job's kind disagrees with its intent".to_owned());
        }
        Ok(())
    }
}

/// Derive the deterministic cursor id for one armed intent.
///
/// Over the intent's own key (`<kind>/<observer>/<subject>/<tick>`), so two
/// passes over one unchanged context arm the **same** cursor — a duplicate arm
/// is a HIT, never a second observation.
fn sensor_schedule_id(
    owner: &mush_core::OwnerId,
    key: &mush_core::IdempotencyKeyText,
) -> Result<ScheduleId, String> {
    let local_hash = CanonicalHash::for_canonical_value(
        "tactics.sensor_schedule_local.v1",
        1,
        &json!({ "key": key }),
        1024,
    )
    .map_err(|error| error.to_string())?;
    let local_id =
        CanonicalId::parse_canonical(local_hash.to_hex()).map_err(|error| error.to_string())?;
    ScheduleId::derive(owner, &local_id).map_err(|error| error.to_string())
}

/// Every family the ingress owns, cloned into each provider and adapter.
#[derive(Clone)]
pub(crate) struct SensorFamilies {
    /// One schedule family per kind, in [`ObservationKind::all`] order.
    schedules: [StateFamilyHandle<ScheduleId, DurableSchedule>; 6],
    jobs: StateFamilyHandle<ScheduleId, SensorJob>,
}

impl SensorFamilies {
    fn schedules_for(
        &self,
        kind: ObservationKind,
    ) -> &StateFamilyHandle<ScheduleId, DurableSchedule> {
        let index = ObservationKind::all()
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap_or(0);
        &self.schedules[index]
    }
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
        DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
        1,
        payload,
    )
    .map_err(|error| ModError::msg(error.to_string()))
}

/// The arming outcome descriptor. Carries a COUNT and nothing else: an outcome
/// that named the observed subjects would be a knowledge leak on the arming
/// actor's own receipt.
pub(crate) fn arm_outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    Ok(descriptor(
        "outcome",
        SENSOR_ARMED_OUTCOME_ID,
        json!({
            "fields": { "armed": "usize", "replaced": "usize" },
            "contains_truth_subject": false
        }),
    )?
    .as_current())
}

/// The arming action descriptor.
pub(crate) fn arm_action_descriptor(
    outcome: &DefinitionRef,
) -> Result<DefinitionDescriptor, ModError> {
    Ok(descriptor(
        "action",
        SENSOR_ARM_ACTION_ID,
        json!({
            "internal": false,
            "subject": "self_actor",
            "targets": 0,
            "state_family": SENSOR_JOB_FAMILY,
            "writes_contacts": false
        }),
    )?
    .with_references(vec![outcome.clone()])
    .as_current())
}

/// The arming provider.
///
/// **It has no `StateFamilyHandle<ContactId, ContactState>`.** That is not a
/// convention: `ActionTxn` dispatches by handle, and a handle it does not hold
/// is a family it cannot name. The probe asserts the type has no contact field.
#[derive(Clone)]
struct SensorArmProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: SensorFamilies,
}

impl SensorArmProvider {
    fn err(code: ActionErrorCode, message: &'static str) -> ActionError {
        ActionError::new(code, false, message).expect("static sensor error is bounded")
    }

    fn state_error(_error: &ActionStateError) -> ActionError {
        Self::err(
            ActionErrorCode::Integrity,
            "sensor ingress state is unavailable",
        )
    }

    /// Decode the bounded intent set. Refuses past the byte cap **before**
    /// parsing, and past the pass cap after — never truncates either.
    fn intents(invocation: &ActionInvocation) -> Result<Vec<ObservationIntent>, ActionError> {
        let text = invocation
            .parameters()
            .get("intents")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Self::err(
                    ActionErrorCode::InvalidRequest,
                    "a sensor arm needs an intent document",
                )
            })?;
        if text.len() > MAX_SENSOR_ARM_BYTES {
            return Err(Self::err(
                ActionErrorCode::BoundsExceeded,
                "that sensor pass is larger than one arm may carry",
            ));
        }
        let intents: Vec<ObservationIntent> = serde_json::from_str(text).map_err(|_| {
            Self::err(
                ActionErrorCode::InvalidRequest,
                "a sensor arm's intent document is malformed",
            )
        })?;
        if intents.len() > MAX_INTENTS_PER_ARM {
            return Err(Self::err(
                ActionErrorCode::BoundsExceeded,
                "that sensor pass names more observations than one arm may carry",
            ));
        }
        if intents.is_empty() {
            return Err(Self::err(
                ActionErrorCode::InvalidRequest,
                "a sensor arm names no observation",
            ));
        }
        Ok(intents)
    }
}

impl ActionProvider for SensorArmProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(Self::err(
                ActionErrorCode::InvalidRequest,
                "a sensor arm acts on self with no targets",
            ));
        }
        Self::intents(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        // Arming is a STAFF act. A player who could arm the ingress could
        // author another actor's picture, which is the whole hazard cut 7
        // exists to close; the offer simply does not exist for them.
        if !crate::module::sensor_arm_is_permitted(&engine.world, query.actor()) {
            return Ok(None);
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            Self::err(
                ActionErrorCode::Integrity,
                "sensor definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                Self::err(
                    ActionErrorCode::Integrity,
                    "sensor guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                Self::err(
                    ActionErrorCode::Integrity,
                    "sensor definition guard is unavailable",
                )
            })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: ProjectedParameterSchema::new(vec![mush_core::ProjectedParameter {
                id: CanonicalId::new("intents")
                    .map_err(|_| Self::err(ActionErrorCode::Integrity, "sensor parameter id"))?,
                required: true,
                kind: mush_core::ProjectedParameterKind::Text {
                    maximum_bytes: MAX_SENSOR_ARM_BYTES,
                },
            }])
            .map_err(|_| {
                Self::err(
                    ActionErrorCode::Integrity,
                    "sensor parameter schema is invalid",
                )
            })?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "sensor_arm",
                "arm the sensor ingress",
                "arm one bounded pass of observation intents",
            )
            .map_err(|_| Self::err(ActionErrorCode::Integrity, "sensor presentation is invalid"))?,
            guards: guards.finish().map_err(|_| {
                Self::err(
                    ActionErrorCode::Integrity,
                    "sensor offer guards are invalid",
                )
            })?,
        }))
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(Self::err(
            ActionErrorCode::Unavailable,
            "arming the sensor ingress requires the authoritative action coordinator",
        ))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let intents = Self::intents(invocation)?;
        let owner = self.families.jobs.owner().clone();
        let mut armed = 0_usize;
        let mut replaced = 0_usize;
        for intent in intents {
            let kind = intent.kind();
            let schedules = self.families.schedules_for(kind);
            let id = sensor_schedule_id(&owner, intent.key()).map_err(|_| {
                Self::err(ActionErrorCode::Integrity, "sensor cursor id is invalid")
            })?;
            let prior = txn
                .get(schedules, &id)
                .map_err(|error| Self::state_error(&error))?
                .cloned();
            let prior_job = txn
                .get(&self.families.jobs, &id)
                .map_err(|error| Self::state_error(&error))?
                .is_some();
            // The one-shot cursor: fires once, at the intent's own stamp.
            let schedule = DurableSchedule::new(DurableScheduleDraft {
                id,
                owner: owner.clone(),
                clock: intent.at().clock.clone(),
                next_due: intent.at().clone(),
                cadence: None,
                generation: 1,
                next_ordinal: 0,
                restart: ScheduleRestartPolicy::Resume,
                catch_up_order: ScheduleCatchUpOrder::OldestFirst,
                max_catch_up: 1,
            })
            .map_err(|_| {
                Self::err(
                    ActionErrorCode::InvalidRequest,
                    "that observation's stamp will not carry a cursor",
                )
            })?;
            let job = SensorJob::new(id, intent, invocation.actor())
                .map_err(|_| Self::err(ActionErrorCode::Integrity, "sensor job is invalid"))?;
            // **Re-arming is deliberate, and it is the recovery path.** A cursor
            // that a refused transition left standing is re-armed with the fresh
            // intent rather than accumulating beside it, so `@prop/pass` clears a
            // stuck arm instead of paging a person (08c §4.6). The identity is
            // the intent's own deterministic key, so a genuine duplicate is a HIT
            // that changes nothing.
            if prior.is_some() {
                replaced = replaced.saturating_add(1);
                txn.replace(schedules, &id, schedule)
            } else {
                txn.insert(schedules, &id, schedule)
            }
            .map_err(|error| Self::state_error(&error))?;
            if prior_job {
                txn.replace(&self.families.jobs, &id, job)
            } else {
                txn.insert(&self.families.jobs, &id, job)
            }
            .map_err(|error| Self::state_error(&error))?;
            armed = armed.saturating_add(1);
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "armed": armed, "replaced": replaced }),
        )
        .map_err(|_| Self::err(ActionErrorCode::Integrity, "sensor outcome is invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// The firing half: one adapter per kind, sharing one policy handle.
#[derive(Clone)]
struct SensorScheduleAdapter {
    kind: ObservationKind,
    families: SensorFamilies,
    contacts: StateFamilyHandle<ContactId, ContactState>,
    policy: SensorPolicyHandle,
}

impl SensorScheduleAdapter {
    fn stale(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::Stale, false, message)
            .expect("static sensor stale error is bounded")
    }

    fn integrity(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::Integrity, false, message)
            .expect("static sensor integrity error is bounded")
    }

    /// The live re-derivation: the policy's own answer for this armed key, when
    /// it still names it. Bounded by the policy handle itself; an over-long pass
    /// is refused there and this simply falls back to the durable intent.
    fn re_derived(&self, engine: &Engine, job: &SensorJob) -> Option<ObservationIntent> {
        self.policy
            .due(&engine.world, job.intent().at())
            .ok()?
            .into_iter()
            .find(|candidate| candidate.key() == job.intent().key())
    }
}

impl ScheduleActionAdapter for SensorScheduleAdapter {
    fn plan(
        &self,
        action: &DefinitionRef,
        advance: &ScheduleAdvance,
        engine: &Engine,
    ) -> Result<ScheduleActionDraft, ActionError> {
        let schedules = self.families.schedules_for(self.kind);
        let current = engine
            .action_state()
            .get(schedules, &advance.schedule())
            .map_err(|_| Self::integrity("sensor cursor is unavailable"))?;
        let candidate_is_current = current.is_some_and(|schedule| {
            matches!(
                schedule.plan_due(schedule.revision(), advance.observed_at(), advance.cause()),
                Ok(ScheduleEvaluation::Advance(planned)) if planned.as_ref() == advance
            )
        });
        if !candidate_is_current {
            return Err(Self::stale("sensor cursor candidate is no longer current"));
        }
        let job = engine
            .action_state()
            .get(&self.families.jobs, &advance.schedule())
            .map_err(|_| Self::integrity("sensor job is unavailable"))?
            .ok_or_else(|| Self::integrity("an armed sensor cursor has no job"))?;
        if job.schedule() != advance.schedule()
            || job.kind() != self.kind
            || advance.occurrences().len() != 1
            || advance.skipped().is_some()
            || advance.post_image().state() != ScheduleState::Completed
        {
            return Err(Self::integrity("sensor cursor and job disagree"));
        }
        // The re-derivation. When the policy still names this key its live
        // parameters win (the freshest revision, so a contact that moved between
        // arm and fire is refreshed against what it actually is); when it does
        // not, the durably armed intent stands, because arming was an
        // authoritative decision and the kernel's own revision check is what
        // decides whether it may still be applied.
        let intent = self
            .re_derived(engine, job)
            .unwrap_or_else(|| job.intent().clone());
        // A transition on a contact that no longer exists cannot succeed; refuse
        // here rather than letting the provider refuse after the cursor was
        // seeded — same shape as `handoff.rs`'s pre-check.
        if let Some(contact) = intent.op().contact()
            && engine
                .action_state()
                .get(&self.contacts, &contact)
                .map_err(|_| Self::integrity("contact authority state is unavailable"))?
                .is_none()
        {
            return Err(Self::stale("that contact no longer exists"));
        }
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| Self::integrity("sensor definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| Self::integrity("sensor guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| Self::integrity("sensor definition guard is unavailable"))?;
        Ok(ScheduleActionDraft {
            actor: intent.observer().platform(),
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: intent.parameters(),
            guards: guards
                .finish()
                .map_err(|_| Self::integrity("sensor guards are invalid"))?,
        })
    }
}

/// Register the ingress: seven families, one arming provider, six sources.
///
/// Called from `TacticsModule::register` **after** the contact family and the
/// six contact providers, so every action the sources name is already
/// registered under this owner.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    contacts: StateFamilyHandle<ContactId, ContactState>,
    outcome: DefinitionRef,
    arm_action: DefinitionRef,
    contact_actions: &[(ObservationKind, DefinitionRef)],
    policy: SensorPolicyHandle,
) -> Result<(), ModError> {
    let mut schedules = Vec::with_capacity(6);
    for kind in ObservationKind::all() {
        schedules.push(
            reg.state_family(
                durable_schedule_state_family_descriptor(
                    StateFamilyId::new(sensor_schedule_family(kind))
                        .map_err(|error| ModError::msg(error.to_string()))?,
                )
                .map_err(|error| ModError::msg(error.to_string()))?,
                DurableScheduleStateCodec,
                DurableScheduleStateCodec,
                DurableScheduleStateCodec,
                DurableScheduleStateCodec,
            )?,
        );
    }
    let jobs = reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(SENSOR_JOB_FAMILY)
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("durable_schedule_id.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("tactics.sensor_job.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            1,
            CanonicalId::new("tactics.sensor_job.decode.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("tactics.sensor_job.validate.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("tactics.sensor_job.json.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?
        .with_limits(64, MAX_SENSOR_JOB_BYTES)
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ScheduleId| Ok(key.canonical_hash().to_hex()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
        SensorJob::validate,
        |value: &SensorJob| serde_json::to_value(value).map_err(|error| error.to_string()),
    )?;
    let families = SensorFamilies {
        schedules: schedules
            .try_into()
            .map_err(|_| ModError::msg("tactics: six sensor schedule families are required"))?,
        jobs,
    };
    reg.action_provider(
        arm_action.clone(),
        SensorArmProvider {
            action: arm_action,
            outcome,
            families: families.clone(),
        },
    )?;
    for (kind, action) in contact_actions {
        reg.schedule_source(
            CanonicalId::new(sensor_source_id(*kind))
                .map_err(|error| ModError::msg(error.to_string()))?,
            action.clone(),
            families.schedules_for(*kind).clone(),
            SensorScheduleAdapter {
                kind: *kind,
                families: families.clone(),
                contacts: contacts.clone(),
                policy: policy.clone(),
            },
        )?;
    }
    Ok(())
}

// ---- E7c7 B3: the leak-safe contact-ledger projection ----

/// How often the projection is rebuilt (seconds, `TickCadence::Coarse`).
pub const LEDGER_TICK_SECS: u64 = 1;
/// The registered tick tag.
pub const LEDGER_TICK_TAG: &str = "tactics.contact_ledger";
/// Contact records read in ONE fire. Bounded on records **visited** (D-LBK-5),
/// and `mush_core::MAX_STATE_FAMILY_SCAN_RECORDS` is the kernel ceiling above it.
pub const MAX_LEDGER_SCAN_PER_FIRE: usize = 128;

/// Rebuild the world-visible, leak-safe contact ledger from the contact family.
///
/// **Why the family's own owner does this.** `Ctx::scan_action_state` is
/// owner-fenced: only `mush-tactics` may read `tactics/contacts`. A policy in
/// another crate therefore cannot name a `ContactId` at all — which would leave
/// the ingress able to `Observe` and nothing else. This publishes the *holder's
/// own view* (opaque id, counters, liveness) into `World` state, so a policy can
/// say "refresh contact X" or "issue Loss on my expiring phantom" without ever
/// holding a family handle. `ContactState::truth_subject` never crosses: a real
/// unit's `ObjId` is not in a row, and the one subject that is — a
/// `SubjectKey::Scoped` phantom — is the policy's own mint.
///
/// Bounded, paged and non-panicking: an over-long family pages across fires and
/// the projection is swapped in only when the scan reaches the end, so a policy
/// never reads a half-rebuilt ledger.
pub(crate) fn ledger_tick(
    ctx: &mut mush_core::Ctx<'_>,
    contacts: &StateFamilyHandle<ContactId, ContactState>,
    owner: &mush_core::OwnerId,
    staging: &mut LedgerScan,
) {
    let Ok(page) = ctx.scan_action_state(
        contacts,
        staging.cursor.as_deref(),
        MAX_LEDGER_SCAN_PER_FIRE,
    ) else {
        // A read that will not answer is not a reason to publish an empty
        // ledger: keep the last complete projection and try again next fire.
        return;
    };
    for entry in page.entries() {
        let state = entry.value();
        let phantom = match state.truth_subject() {
            SubjectKey::Scoped {
                owner: subject_owner,
                class,
                id,
            } if subject_owner == owner_of_phantom(owner) && class.as_str() == PHANTOM_CLASS => {
                Some(id.clone())
            }
            _ => None,
        };
        for holder in state.audience() {
            staging.rows.push(mush_core::ContactLedgerRow {
                holder: holder.clone(),
                contact: entry_contact(entry.encoded_key()),
                generation: state.generation(),
                revision: state.revision(),
                active: state.is_active(),
                observed_at: state.observed_at().clone(),
                phantom: phantom.clone(),
            });
        }
        if staging.rows.len() > mush_core::MAX_CONTACT_LEDGER_ROWS.saturating_mul(2) {
            // Refuse to grow without bound while paging: the projection's own
            // installer flags the overflow, and a further page would only cost
            // memory to be dropped.
            staging.cursor = None;
            break;
        }
    }
    match page.next_after() {
        Some(cursor) => staging.cursor = Some(cursor.to_string()),
        None => {
            let at = ctx.w.world_ic_now();
            let rows = std::mem::take(&mut staging.rows);
            staging.cursor = None;
            if let Some(state) = ctx.w.module_state_mut::<mush_core::ContactLedgerState>() {
                state.install(rows, at);
            }
        }
    }
}

/// The `SubjectKey::Scoped` class propagation mints its false contacts under.
/// Named here so the projection can recognise one without a `mush-propagation`
/// edge; propagation asserts the same constant.
pub const PHANTOM_CLASS: &str = "phantom";

/// The owner a phantom subject is scoped to. Deliberately **not** this module:
/// a phantom is propagation's, and a `Scoped` subject under any other owner is
/// simply not a phantom as far as this projection is concerned.
pub const PHANTOM_OWNER: &str = "propagation";

fn owner_of_phantom(_tactics: &mush_core::OwnerId) -> &mush_core::OwnerId {
    // A `OnceLock` so the comparison allocates nothing per row.
    static PHANTOM: std::sync::OnceLock<mush_core::OwnerId> = std::sync::OnceLock::new();
    PHANTOM.get_or_init(|| mush_core::OwnerId::new(PHANTOM_OWNER).expect("static phantom owner id"))
}

fn entry_contact(encoded: &str) -> ContactId {
    // The family's key codec is `ContactId::to_hex`; a key that will not parse
    // cannot have been written by it, so an all-zero id is the loud, inert
    // answer rather than a panic inside a tick.
    let mut bytes = [0_u8; 16];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let Some(pair) = encoded.get(index * 2..index * 2 + 2) else {
            return ContactId::from_bytes([0; 16]);
        };
        match u8::from_str_radix(pair, 16) {
            Ok(byte) => *slot = byte,
            Err(_) => return ContactId::from_bytes([0; 16]),
        }
    }
    ContactId::from_bytes(bytes)
}

/// The rotation state one ledger rebuild carries between fires.
#[derive(Clone, Debug, Default)]
pub struct LedgerScan {
    cursor: Option<String>,
    rows: Vec<mush_core::ContactLedgerRow>,
}

/// The stamp a pass is armed at, used by callers that need to name the tick in
/// a key without reaching for a clock of their own.
#[must_use]
pub fn pass_tick(at: &ClockStamp) -> i64 {
    at.at
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{IdempotencyKeyText, ObservationOp, ObserverRef, PrincipalId};

    fn stamp() -> ClockStamp {
        ClockStamp {
            clock: mush_core::ClockRef::Utc,
            at: 10,
        }
    }

    fn intent(kind: ObservationKind) -> ObservationIntent {
        let observer = ObserverRef::Platform { id: ObjId(7) };
        let key = ObservationIntent::key_for(kind, &observer, "o90", 10).unwrap();
        let op = match kind {
            ObservationKind::Observe => ObservationOp::Observe {
                truth: SubjectKey::Object { id: ObjId(90) },
                audience: vec![PrincipalId::Object { id: ObjId(7) }],
            },
            ObservationKind::Refresh => ObservationOp::Refresh {
                contact: ContactId::from_bytes([1; 16]),
                expected_revision: 1,
            },
            ObservationKind::Share => ObservationOp::Share {
                contact: ContactId::from_bytes([1; 16]),
                expected_revision: 1,
                to: ObserverRef::Platform { id: ObjId(8) },
            },
            ObservationKind::Unshare => ObservationOp::Unshare {
                contact: ContactId::from_bytes([1; 16]),
                expected_revision: 1,
                from: ObserverRef::Platform { id: ObjId(8) },
            },
            ObservationKind::Loss => ObservationOp::Loss {
                contact: ContactId::from_bytes([1; 16]),
                expected_revision: 1,
                reason: CanonicalId::new("occluded").unwrap(),
            },
            ObservationKind::Reacquire => ObservationOp::Reacquire {
                contact: ContactId::from_bytes([1; 16]),
                expected_revision: 1,
            },
        };
        ObservationIntent::new(observer, op, stamp(), key).unwrap()
    }

    #[test]
    fn every_kind_has_a_distinct_family_and_a_distinct_source_id() {
        let mut families: Vec<&str> = ObservationKind::all()
            .iter()
            .map(|k| sensor_schedule_family(*k))
            .collect();
        let mut ids: Vec<&str> = ObservationKind::all()
            .iter()
            .map(|k| sensor_source_id(*k))
            .collect();
        families.sort_unstable();
        families.dedup();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(families.len(), 6, "one source per family per owner");
        assert_eq!(ids.len(), 6);
        // Disjoint from every other schedule source on the tree.
        for id in &ids {
            assert!(!matches!(*id, "control_handoff" | "production" | "npc"));
        }
    }

    #[test]
    fn a_job_whose_stored_kind_disagrees_with_its_intent_is_refused_on_load() {
        let owner = mush_core::OwnerId::new("tactics").unwrap();
        let armed = intent(ObservationKind::Loss);
        let id = sensor_schedule_id(&owner, armed.key()).unwrap();
        let job = SensorJob::new(id, armed, ObjId(1)).unwrap();
        let mut value = serde_json::to_value(&job).unwrap();
        value["kind"] = json!("share");
        assert!(serde_json::from_value::<SensorJob>(value).is_err());
    }

    #[test]
    fn the_cursor_id_is_deterministic_over_the_intents_own_key() {
        let owner = mush_core::OwnerId::new("tactics").unwrap();
        let a = sensor_schedule_id(&owner, intent(ObservationKind::Refresh).key()).unwrap();
        let b = sensor_schedule_id(&owner, intent(ObservationKind::Refresh).key()).unwrap();
        assert_eq!(a, b, "a duplicate arm is a HIT");
        let other = sensor_schedule_id(
            &owner,
            &IdempotencyKeyText::parse_canonical("refresh/p7/o91/a").unwrap(),
        )
        .unwrap();
        assert_ne!(a, other);
    }

    #[test]
    fn every_kinds_intent_renders_the_parameter_document_its_provider_decodes() {
        for kind in ObservationKind::all() {
            let parameters = intent(kind).parameters();
            let object = parameters.as_object().expect("a parameter document");
            match kind {
                ObservationKind::Observe => {
                    assert!(object.contains_key("truth_subject"));
                    assert!(object.contains_key("audience"));
                    assert!(object.contains_key("observed_at"));
                }
                ObservationKind::Refresh | ObservationKind::Reacquire => {
                    assert!(object.contains_key("contact"));
                    assert!(object.contains_key("expected_revision"));
                    assert!(object.contains_key("observed_at"));
                }
                ObservationKind::Share | ObservationKind::Unshare => {
                    assert!(object.contains_key("principal"));
                    assert!(object.contains_key("expected_revision"));
                }
                ObservationKind::Loss => {
                    assert!(object.contains_key("lost_at"));
                    assert!(object.contains_key("reason"));
                }
            }
        }
    }
}
