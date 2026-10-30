use super::{model::*, prepare};
use crate::detachment::activation::{ActivationHandles, ActiveDetachment, Reservation};
use crate::detachment::crew::{CrewContract, CrewHandles};
use crate::detachment::planning::DraftRef;
use crate::port::families::PortFamilies;
use mush_core::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) const ACTION: &str = "tacsim_detachment_flight";
const OUTCOME: &str = "tacsim_detachment_flight_result";
const OPS: &str = "tacsim.detachment_operations";
const HEADS: &str = "tacsim.detachment_unit_heads";
const FLIGHTS: &str = "tacsim.detachment_flights";
const INDEX: &str = "tacsim.detachment_flight_index";
const REVISION: &str = "detachment_flight_input";
pub(super) type BlackBoxHandle =
    participant::ParticipantHandle<crate::surface::black_box::BlackBoxParticipant>;

#[derive(Clone, Debug)]
pub struct SortieHandles {
    pub index: StateFamilyHandle<(), FlightIndex>,
    pub operations: StateFamilyHandle<OperationId, OperationRecord>,
    pub heads: StateFamilyHandle<ObjId, UnitHead>,
    pub flights: StateFamilyHandle<FlightKey, Flight>,
    pub(super) active: StateFamilyHandle<DraftRef, ActiveDetachment>,
    pub(super) reservations: StateFamilyHandle<ObjId, Reservation>,
    pub(super) contracts: StateFamilyHandle<ObjId, CrewContract>,
    pub(super) ports: Box<PortFamilies>,
    pub(super) action: DefinitionRef,
    pub(super) enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Command {
    Launch {
        operation: OperationId,
        slots: Vec<Assignment>,
    },
    Recover {
        operation: OperationId,
        slot: u16,
        unit: ObjId,
    },
    Cancel {
        operation: OperationId,
        slot: u16,
    },
    End {
        operation: OperationId,
    },
    Finish {
        flight: FlightKey,
        revision: u64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Assignment {
    pub slot: u16,
    pub unit: ObjId,
    pub operator: ObjId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub utc: i64,
    pub ic: ClockStamp,
    pub command: Command,
}
#[derive(Clone, Default)]
struct Pending(BTreeMap<ObjId, Intent>);
pub(super) fn pending(world: &World, actor: ObjId) -> Option<&Intent> {
    world.module_state::<Pending>()?.0.get(&actor)
}
pub(super) fn error(message: impl std::fmt::Display) -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        message.to_string(),
    )
    .unwrap_or_else(|_| {
        crate::port_tick::port_error(ActionErrorCode::ProviderRejected, "Deployment unavailable.")
    })
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(bad)?,
        DefinitionId::new(id).map_err(bad)?,
        1,
        payload,
    )
    .map_err(bad)
}
pub(crate) fn bridge_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "bridge",
        "tacsim_detachment_flight_unit_rights",
        json!({"format":1,"purpose":"current_unit_use_for_detachment_flight"}),
    )
}
pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "outcome",
        OUTCOME,
        json!({"format":1,"fields":["operation","flights"]}),
    )
}
pub(crate) fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(OUTCOME)
        .cloned()
        .ok_or_else(|| bad("flight outcome missing"))?;
    descriptor(
        "action",
        ACTION,
        json!({"format":1,"subject":"self_actor","targets":0,"families":[OPS,HEADS,FLIGHTS,INDEX]}),
    )
    .map(|d| d.as_current().with_references(vec![outcome]))
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    active: &ActivationHandles,
    crew: &CrewHandles,
    ports: &PortFamilies,
    (enabled, black_box): (bool, Option<BlackBoxHandle>),
) -> Result<(), ModError> {
    let index = reg.state_family(
        crate::port::families::family_descriptor(
            INDEX,
            "tacsim.detachment_flight_index.key.v1",
            INDEX,
        )?
        .with_limits(16, 32 * 1024)
        .map_err(bad)?,
        |_: &()| Ok("active".to_owned()),
        |v: &Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        FlightIndex::validate,
        |v: &FlightIndex| serde_json::to_value(v).map_err(|e| e.to_string()),
    )?;
    let operations = reg.state_family(
        crate::port::families::family_descriptor(OPS, "tacsim.detachment_operation.key.v1", OPS)?
            .with_limits(80, 128 * 1024)
            .map_err(bad)?,
        |key: &OperationId| key.key(),
        |v: &Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        OperationRecord::validate,
        |v: &OperationRecord| serde_json::to_value(v).map_err(|e| e.to_string()),
    )?;
    let heads = reg.state_family(
        crate::port::families::family_descriptor(
            HEADS,
            "tacsim.detachment_unit_head.key.v1",
            HEADS,
        )?
        .with_limits(16, 2048)
        .map_err(bad)?,
        |key: &ObjId| Ok(key.0.to_string()),
        |v: &Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        UnitHead::validate,
        |v: &UnitHead| serde_json::to_value(v).map_err(|e| e.to_string()),
    )?;
    let flights = reg.state_family(
        crate::port::families::family_descriptor(
            FLIGHTS,
            "tacsim.detachment_flight.key.v1",
            FLIGHTS,
        )?
        .with_limits(48, 32 * 1024)
        .map_err(bad)?,
        |key: &FlightKey| key.key(),
        |v: &Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        Flight::validate,
        |v: &Flight| serde_json::to_value(v).map_err(|e| e.to_string()),
    )?;
    let action = actions
        .get(ACTION)
        .cloned()
        .ok_or_else(|| bad("flight action missing"))?;
    let outcome = outcomes
        .get(OUTCOME)
        .cloned()
        .ok_or_else(|| bad("flight outcome missing"))?;
    if enabled {
        let bridge = reg.definition(bridge_descriptor()?)?;
        reg.interaction_bridge(BridgeDescriptor::new(
            bridge,
            OwnerId::new(crate::NS).map_err(bad)?,
            OwnerId::new("org").map_err(bad)?,
            action.clone(),
        ))?;
    }
    let handles = SortieHandles {
        index,
        operations,
        heads,
        flights,
        active: active.records.clone(),
        reservations: active.reservations.clone(),
        contracts: crew.contracts.clone(),
        ports: Box::new(ports.clone()),
        action: action.clone(),
        enabled,
    };
    let shared = Arc::new(handles.clone());
    reg.action_provider(
        action,
        Provider {
            handles: Arc::clone(&shared),
            outcome,
            black_box,
        },
    )?;
    reg.revision_validator(
        RevisionKind::new(CanonicalId::new(REVISION).map_err(bad)?),
        Validator(Arc::clone(&shared)),
    )?;
    reg.revision_validator(
        RevisionKind::new(CanonicalId::new("detachment_unreserved_unit").map_err(bad)?),
        Availability(shared),
    )?;
    reg.state(Pending::default());
    reg.state(handles);
    super::tick::register(reg)
}

pub(super) trait Read {
    fn get<K, V>(&mut self, handle: &StateFamilyHandle<K, V>, key: &K) -> Result<Option<V>, String>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static;
}
impl Read for &Engine {
    fn get<K, V>(&mut self, h: &StateFamilyHandle<K, V>, k: &K) -> Result<Option<V>, String>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_state()
            .get(h, k)
            .map(|v| v.cloned())
            .map_err(|e| e.to_string())
    }
}
impl Read for &Ctx<'_> {
    fn get<K, V>(&mut self, h: &StateFamilyHandle<K, V>, k: &K) -> Result<Option<V>, String>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_state(h, k)
            .map(|v| v.cloned())
            .map_err(|e| e.to_string())
    }
}
impl Read for &RevisionValidatorContext<'_> {
    fn get<K, V>(&mut self, h: &StateFamilyHandle<K, V>, k: &K) -> Result<Option<V>, String>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_state(h, k)
            .map(|v| v.cloned())
            .map_err(|e| e.to_string())
    }
}
impl Read for &mut ActionTxn<'_> {
    fn get<K, V>(&mut self, h: &StateFamilyHandle<K, V>, k: &K) -> Result<Option<V>, String>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        ActionTxn::get(self, h, k)
            .map(|v| v.cloned())
            .map_err(|e| e.to_string())
    }
}
fn coordinate(actor: ObjId) -> Result<RevisionKey, ActionError> {
    Ok(RevisionKey {
        owner: OwnerId::new(crate::NS).map_err(error)?,
        kind: RevisionKind::new(CanonicalId::new(REVISION).map_err(error)?),
        subject: SubjectKey::Object { id: actor },
        facet: None,
    })
}
pub(super) fn digest(actor: ObjId, intent: &Intent) -> Result<CanonicalHash, ActionError> {
    CanonicalHash::for_canonical_value(
        "tacsim.detachment_flight_request.v1",
        1,
        &json!({"actor":actor,"intent":intent}),
        4096,
    )
    .map_err(error)
}
struct Validator(Arc<SortieHandles>);
impl RevisionValidator for Validator {
    fn current_generation(
        &self,
        ctx: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let SubjectKey::Object { id: actor } = key.subject else {
            return Ok(None);
        };
        if &coordinate(actor)? != key {
            return Ok(None);
        }
        let Some(intent) = pending(ctx.world(), actor) else {
            return Ok(None);
        };
        Ok(
            prepare::snapshot(ctx.world(), &mut &*ctx, &self.0, actor, intent)
                .ok()
                .map(|s| s.generation()),
        )
    }
}
struct Provider {
    handles: Arc<SortieHandles>,
    outcome: DefinitionRef,
    black_box: Option<BlackBoxHandle>,
}
fn chosen(i: &ActionInvocation) -> Result<(Intent, CanonicalHash), ActionError> {
    if i.principal() != &(PrincipalId::Object { id: i.actor() })
        || i.subject() != &PerceivedRef::SelfActor
        || !i.targets().is_empty()
        || i.parameters().as_object().is_none_or(|p| p.len() != 3)
    {
        return Err(error("Invalid flight selection."));
    }
    let intent: Intent = serde_json::from_value(
        i.parameters()
            .get("intent")
            .cloned()
            .ok_or_else(|| error("Missing intent."))?,
    )
    .map_err(error)?;
    if i.parameters().get("request").and_then(Value::as_str)
        != Some(digest(i.actor(), &intent)?.to_hex().as_str())
    {
        return Err(error("Flight request changed."));
    }
    let facts: CanonicalHash = serde_json::from_value(
        i.parameters()
            .get("facts")
            .cloned()
            .ok_or_else(|| error("Missing facts."))?,
    )
    .map_err(error)?;
    Ok((intent, facts))
}
pub(crate) fn availability_key(unit: ObjId) -> Result<RevisionKey, ActionError> {
    Ok(RevisionKey {
        owner: OwnerId::new(crate::NS).map_err(error)?,
        kind: RevisionKind::new(CanonicalId::new("detachment_unreserved_unit").map_err(error)?),
        subject: SubjectKey::Object { id: unit },
        facet: None,
    })
}
fn available(read: &mut impl Read, h: &SortieHandles, unit: ObjId) -> Result<bool, String> {
    if read.get(&h.reservations, &unit)?.is_some() {
        return Ok(false);
    }
    if read.get(&h.heads, &unit)?.is_some() {
        return prepare::current(read, h, unit).map(|(_, f)| !f.phase.open());
    }
    Ok(true)
}
pub(crate) fn require_available(
    world: &World,
    txn: &mut ActionTxn<'_>,
    unit: ObjId,
) -> Result<(), ActionError> {
    if let Some(h) = world.module_state::<SortieHandles>()
        && !available(&mut &mut *txn, h, unit).map_err(error)?
    {
        return Err(error(
            "Unit is reserved for a detachment or has an open detachment flight.",
        ));
    }
    Ok(())
}
pub(crate) fn offer_available(engine: &Engine, unit: ObjId) -> bool {
    engine
        .world
        .module_state::<SortieHandles>()
        .is_none_or(|h| available(&mut &*engine, h, unit).unwrap_or(false))
}
struct Availability(Arc<SortieHandles>);
impl RevisionValidator for Availability {
    fn current_generation(
        &self,
        ctx: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let SubjectKey::Object { id: unit } = key.subject else {
            return Ok(None);
        };
        if &availability_key(unit)? != key {
            return Ok(None);
        }
        Ok(available(&mut &*ctx, &self.0, unit)
            .map_err(error)?
            .then_some(1))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, i: &ActionInvocation) -> Result<(), ActionError> {
        chosen(i).map(|_| ())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self.handles.enabled
            || action != &self.handles.action
            || q.projected_subject() != &PerceivedRef::SelfActor
            || q.truth_subject() != &(SubjectKey::Object { id: q.actor() })
        {
            return Ok(None);
        }
        let Some(intent) = pending(&engine.world, q.actor()) else {
            return Ok(None);
        };
        let Ok(snapshot) = prepare::snapshot(
            &engine.world,
            &mut &*engine,
            &self.handles,
            q.actor(),
            intent,
        ) else {
            return Ok(None);
        };
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: coordinate(q.actor())?,
            generation: snapshot.generation(),
        }])
        .map_err(error)?;
        crate::surface::act::offer_draft_with_revisions(
            action,
            engine,
            json!({"intent":intent,"facts":snapshot.hash}),
            vec![ProjectedParameter {
                id: CanonicalId::new("request").map_err(error)?,
                required: true,
                kind: ProjectedParameterKind::Text { maximum_bytes: 64 },
            }],
            (
                "detachment-flight",
                "Detachment deployment",
                "Commit the selected physical deployment step.",
            ),
            revisions,
        )
        .map(Some)
    }
    fn resolved_revision_keys(
        &self,
        r: &ResolvedActionInvocation,
    ) -> Result<Vec<RevisionKey>, ActionError> {
        let (intent, _) = chosen(r.invocation())?;
        // The fixed units/operators are re-read from this regenerated selection.
        // Core checks the real owner guards through final take.
        let world_units = matches!(
            intent.command,
            Command::Launch { .. } | Command::Recover { .. }
        );
        if !world_units {
            return Ok(Vec::new());
        }
        prepare::authority_keys(&intent, r.invocation().actor())
    }
    fn resolved_authority_witnesses(
        &self,
        r: &ResolvedActionInvocation,
        engine: &Engine,
    ) -> Result<Vec<AuthorityWitnessRequest>, ActionError> {
        let (intent, _) = chosen(r.invocation())?;
        if let Command::Recover { unit, .. } = &intent.command {
            let (_, flight) =
                prepare::current(&mut &*engine, &self.handles, *unit).map_err(error)?;
            return Ok(vec![AuthorityWitnessRequest::ControlLease {
                provider: OwnerId::new("sim").map_err(error)?,
                subject: *unit,
                channel: ChannelId::new("helm").ok_or_else(|| error("Helm unavailable."))?,
                holder: PrincipalId::Object {
                    id: flight.operator,
                },
            }]);
        }
        let Command::Launch { operation, slots } = intent.command else {
            return Ok(Vec::new());
        };
        let roster = engine
            .action_state()
            .get(&self.handles.active, &operation.reference())
            .map_err(error)?
            .ok_or_else(|| error("Roster missing."))?;
        roster
            .plan
            .slots
            .iter()
            .filter(|s| slots.iter().any(|selected| selected.slot == s.slot.get()))
            .map(|slot| {
                Ok(AuthorityWitnessRequest::ControlLease {
                    provider: OwnerId::new("sim").map_err(error)?,
                    subject: slot.unit.ok_or_else(|| error("Unit missing."))?,
                    channel: ChannelId::new("helm").ok_or_else(|| error("Helm unavailable."))?,
                    holder: PrincipalId::Object {
                        id: slot
                            .controller
                            .character()
                            .ok_or_else(|| error("Operator missing."))?,
                    },
                })
            })
            .collect()
    }
    fn prepare_resolved_authoritative_with_context(
        &self,
        r: &ResolvedActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let (intent, expected) = chosen(r.invocation())?;
        let snapshot = prepare::snapshot(
            ctx.world().map_err(error)?,
            &mut &mut *txn,
            &self.handles,
            r.invocation().actor(),
            &intent,
        )
        .map_err(error)?;
        if snapshot.hash != expected {
            return Err(error("Flight facts changed; inspect and retry."));
        }
        let output = snapshot.apply(txn, ctx, &self.handles, self.black_box.as_ref())?;
        let result = RulesValue::new(self.outcome.clone(), output).map_err(error)?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_eligible_witnesses(vec![r.invocation().principal().clone()]))
    }
}
pub(super) fn submit(ctx: &mut Ctx<'_>, command: Command) -> Result<(), String> {
    enqueue(ctx, ctx.actor, command)
}
pub(super) fn finish_submit(ctx: &mut Ctx<'_>, flight: &Flight) -> Result<(), String> {
    enqueue(
        ctx,
        flight.continuation_actor(),
        Command::Finish {
            flight: flight.key,
            revision: flight.revision,
        },
    )
}
fn enqueue(ctx: &mut Ctx<'_>, actor: ObjId, command: Command) -> Result<(), String> {
    if ctx.authoritative_pending(actor)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return Err("An authoritative request is already pending.".into());
    }
    let action = ctx
        .w
        .module_state::<SortieHandles>()
        .filter(|h| h.enabled)
        .ok_or("Deployment unavailable.")?
        .action
        .clone();
    let intent = Intent {
        utc: ctx.w.now(),
        ic: ctx.w.world_ic_now(),
        command,
    };
    let now = intent.utc;
    let queue = ctx
        .w
        .module_state_mut::<Pending>()
        .ok_or("Deployment unavailable.")?;
    queue.0.retain(|_, i| {
        now.checked_sub(i.utc)
            .is_some_and(|age| (0..=300).contains(&age))
    });
    if queue.0.len() >= 256 && !queue.0.contains_key(&actor) {
        return Err("Deployment is busy.".into());
    }
    queue.0.insert(actor, intent.clone());
    let handles = ctx
        .w
        .module_state::<SortieHandles>()
        .cloned()
        .ok_or("Deployment unavailable.")?;
    if actor == ctx.actor {
        super::commands::check_caller_rights(ctx, &intent.command)?;
    }
    let snapshot = prepare::snapshot(ctx.w, &mut &*ctx, &handles, actor, &intent)?;
    let digest = digest(actor, &intent).map_err(|e| e.to_string())?.to_hex();
    // A return/cancel/relaunch cycle can complete within one wall-clock second.
    // Its new owner preimages must not reuse the first departure's receipt.
    let key = IdempotencyKeyText::parse_canonical(format!(
        "detachment-flight/{}",
        snapshot.hash.to_hex()
    ))
    .map_err(|e| e.to_string())?;
    ctx.out.defer(Deferred::AuthoritativeSubmit(Box::new(
        AuthoritativeSubmit {
            actor,
            action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({"request":digest}),
            key,
        },
    )));
    Ok(())
}
