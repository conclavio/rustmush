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
const MAX_RETAINED_ENDED_OPS: usize = 4;
const MAX_PRUNE_OPS_PER_TXN: usize = 2;
const MAX_PRUNED_OP_FLIGHTS_PER_TXN: usize = 8;
const PRUNE_OP_SCAN_LIMIT: usize = 32;
pub(super) type BlackBoxHandle =
    participant::ParticipantHandle<crate::surface::black_box::BlackBoxParticipant>;

#[derive(Clone, Debug)]
pub struct SortieHandles {
    pub(super) transport:
        Option<participant::ParticipantHandle<mush_propagation::field_transport::FieldTransport>>,
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
    pub(crate) controls:
        Option<mush_core::read_service::OwnerReadHandle<mush_sim::crew_control::CrewControlReader>>,
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
    Refresh {
        operation: OperationId,
        facts: super::field::OwnerFacts,
    },
    Message(Box<super::field_orders::Request>),
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
/// Maximum entries kept in [`ToldLapses`] and [`PendingLapses`].
const MAX_LAPSE_BOOK: usize = 256;

/// **The lapses already told** (doc 45 unit AP.8; RAM): bounded FIFO deduplication by
/// `(operation, slot, order sequence)` so capacity pressure evicts only the oldest entry instead
/// of wiping the whole set (M34).
#[derive(Clone, Default)]
struct ToldLapses {
    order: std::collections::VecDeque<(OperationId, u16, u64)>,
    seen: std::collections::BTreeSet<(OperationId, u16, u64)>,
}

impl ToldLapses {
    fn insert(&mut self, key: (OperationId, u16, u64)) -> bool {
        if self.seen.contains(&key) {
            return false;
        }
        while self.order.len() >= MAX_LAPSE_BOOK {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        self.seen.insert(key);
        self.order.push_back(key);
        true
    }
}

/// One staged lapse waiting for its authoritative commit before notifying players (M34).
#[derive(Clone)]
struct PendingLapse {
    submitter: ObjId,
    queued_at: i64,
    operation: OperationId,
    leader: Option<mush_tactics::detachment::Leader>,
    lapse: super::field_orders::NewLapse,
}

/// Staged lapses awaiting authoritative commit confirmation (RAM, bounded FIFO).
#[derive(Clone, Default)]
struct PendingLapses(std::collections::VecDeque<PendingLapse>);

/// Stage newly computed lapses for post-commit delivery instead of notifying speculatively before
/// the authoritative bundle lands (M34).
fn queue_lapses(ctx: &mut Ctx<'_>, submitter: ObjId, snapshot: &prepare::Snapshot) {
    let Some((operation, leader, lapses)) = snapshot.new_lapses() else {
        return;
    };
    let queued_at = ctx.w.now();
    let Some(pending) = ctx.w.module_state_mut::<PendingLapses>() else {
        return;
    };
    for lapse in lapses {
        let already = pending.0.iter().any(|p| {
            p.operation == operation
                && p.lapse.slot == lapse.slot
                && p.lapse.sequence == lapse.sequence
        });
        if already {
            continue;
        }
        while pending.0.len() >= MAX_LAPSE_BOOK {
            pending.0.pop_front();
        }
        pending.0.push_back(PendingLapse {
            submitter,
            queued_at,
            operation,
            leader,
            lapse,
        });
    }
}

/// Deliver staged lapse notices only after `OperationRecord` in `ActionStateStore` confirms the
/// lapse committed, dropping failed or expired staged notices cleanly (M34).
pub(super) fn flush_committed_lapses(ctx: &mut Ctx<'_>, handles: &SortieHandles) {
    use crate::port_tick::{NoticeLevel, notify_budget};
    let staged = ctx
        .w
        .module_state_mut::<PendingLapses>()
        .map(|p| std::mem::take(&mut p.0))
        .unwrap_or_default();
    if staged.is_empty() {
        return;
    }
    let now = ctx.w.now();
    let mut keep = std::collections::VecDeque::new();
    for entry in staged {
        let committed = ctx
            .action_state(&handles.operations, &entry.operation)
            .ok()
            .flatten()
            .and_then(|op| op.orders.as_deref())
            .is_some_and(|orders| {
                orders
                    .lapse_of(entry.lapse.slot, entry.lapse.sequence)
                    .is_some()
            });
        if committed {
            let key = (
                entry.operation,
                entry.lapse.slot.get(),
                entry.lapse.sequence,
            );
            let fresh = ctx
                .w
                .module_state_mut::<ToldLapses>()
                .is_some_and(|told| told.insert(key));
            if !fresh {
                continue;
            }
            let lead = entry
                .leader
                .filter(|leader| leader.kind == mush_tactics::detachment::LeaderKind::Player)
                .map(|leader| leader.character);
            let fill = |line: &str| {
                line.replace(
                    "{order}",
                    &super::field_commands::directive_text(&entry.lapse.directive),
                )
                .replace("{slot}", &entry.lapse.slot.get().to_string())
            };
            notify_budget(
                ctx,
                entry.lapse.issuer,
                &fill(super::ORDER_LAPSED),
                NoticeLevel::Trip,
            );
            if let Some(lead) = lead.filter(|lead| *lead != entry.lapse.issuer) {
                notify_budget(
                    ctx,
                    lead,
                    &fill(super::ORDER_LAPSED_LEAD),
                    NoticeLevel::Trip,
                );
            }
            continue;
        }
        let still_valid_age = now
            .checked_sub(entry.queued_at)
            .is_some_and(|age| (0..=300).contains(&age));
        let still_pending =
            ctx.authoritative_pending(entry.submitter) || pending(ctx.w, entry.submitter).is_some();
        if still_valid_age && still_pending {
            keep.push_back(entry);
        }
    }
    if !keep.is_empty()
        && let Some(pending_lapses) = ctx.w.module_state_mut::<PendingLapses>()
    {
        for entry in keep {
            if pending_lapses.0.len() >= MAX_LAPSE_BOOK {
                pending_lapses.0.pop_front();
            }
            pending_lapses.0.push_back(entry);
        }
    }
}

/// Evict expired (>300s) and already-settled entries from `Pending` so committed or stale
/// deployment intents never occupy the 256-slot global queue for 300s (M34).
pub(super) fn prune_settled(ctx: &mut Ctx<'_>, handles: &SortieHandles) {
    let entries: Vec<(ObjId, Intent)> = ctx
        .w
        .module_state::<Pending>()
        .map(|q| q.0.iter().map(|(a, i)| (*a, i.clone())).collect())
        .unwrap_or_default();
    if entries.is_empty() {
        return;
    }
    let now = ctx.w.now();
    let mut evict = Vec::new();
    for (actor, intent) in entries {
        let expired = !now
            .checked_sub(intent.utc)
            .is_some_and(|age| (0..=300).contains(&age));
        let settled = !ctx.authoritative_pending(actor)
            && prepare::snapshot_unregistered(ctx.w, &mut &*ctx, handles, actor, &intent).is_err();
        if expired || settled {
            evict.push((actor, intent));
        }
    }
    if let Some(queue) = ctx.w.module_state_mut::<Pending>() {
        for (actor, intent) in evict {
            if queue.0.get(&actor) == Some(&intent) {
                queue.0.remove(&actor);
            }
        }
    }
}
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
/// Exact source action admitted by the composed Propagation owner participant.
pub fn transport_action_ref() -> Result<DefinitionRef, ModError> {
    let mut registry = DefinitionRegistryBuilder::new();
    let owner = OwnerId::new(crate::NS).map_err(bad)?;
    let outcome = registry
        .register(owner.clone(), outcome_descriptor()?)
        .map_err(bad)?;
    let descriptor = action_descriptor(&BTreeMap::from([(OUTCOME.to_owned(), outcome)]))?;
    registry.register(owner, descriptor).map_err(bad)
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
    let controls = if enabled {
        let reference = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("sim").map_err(bad)?,
                mush_sim::crew_control::definition_descriptor()?,
            )
            .map_err(bad)?;
        Some(reg.request_owner_read::<mush_sim::crew_control::CrewControlReader>(reference)?)
    } else {
        None
    };
    let handles = SortieHandles {
        transport: if enabled {
            Some(
                reg.request_owner_participant::<mush_propagation::field_transport::FieldTransport>(
                    action.clone(),
                )?,
            )
        } else {
            None
        },
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
        controls,
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
        Availability(Arc::clone(&shared)),
    )?;
    super::surface::register(
        reg,
        shared,
        actions.get(crate::surface::act::SURFACE_ACT_ACTION),
    )?;
    reg.state(Pending::default());
    reg.state(ToldLapses::default());
    reg.state(PendingLapses::default());
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
fn operation_prunable_flights(
    read: &mut impl Read,
    handles: &SortieHandles,
    op: &OperationRecord,
    touched: &std::collections::BTreeSet<FlightKey>,
) -> Result<Option<Vec<FlightKey>>, String> {
    if op.ended.is_none() {
        return Ok(None);
    }
    let mut removable = Vec::new();
    for key in &op.flights {
        if touched.contains(key) {
            return Ok(None);
        }
        if read
            .get(&handles.heads, &key.unit)?
            .is_some_and(|head| head.flight == *key)
        {
            return Ok(None);
        }
        if read
            .get(
                &handles.ports.excursions,
                &crate::surface::excursion::RigKey(key.unit),
            )?
            .and_then(|record| record.field_crew.as_deref().map(|crew| crew.flight))
            == Some(*key)
        {
            return Ok(None);
        }
        if let Some(flight) = read.get(&handles.flights, key)? {
            if flight.phase.open() {
                return Ok(None);
            }
            removable.push(*key);
        }
    }
    Ok(Some(removable))
}
fn prunable_ended_operations(
    engine: &Engine,
    handles: &SortieHandles,
    actor: ObjId,
    exclude: OperationId,
    touched: &std::collections::BTreeSet<FlightKey>,
) -> Result<Vec<OperationId>, ActionError> {
    let prefix = format!("{}-", actor.0);
    let page = engine
        .action_state()
        .scan_family(&handles.operations, Some(&prefix), PRUNE_OP_SCAN_LIMIT)
        .map_err(error)?;
    let mut candidates = Vec::new();
    for entry in page.entries() {
        if !entry.encoded_key().starts_with(&prefix) {
            break;
        }
        let op = entry.value();
        if op.id.organizer != actor || op.id == exclude {
            continue;
        }
        if operation_prunable_flights(&mut &*engine, handles, op, touched)
            .map_err(error)?
            .is_some()
        {
            candidates.push((op.began.at, op.id));
        }
    }
    if candidates.len() <= MAX_RETAINED_ENDED_OPS {
        return Ok(Vec::new());
    }
    candidates.sort();
    let count = (candidates.len() - MAX_RETAINED_ENDED_OPS).min(MAX_PRUNE_OPS_PER_TXN);
    Ok(candidates
        .into_iter()
        .take(count)
        .map(|(_, id)| id)
        .collect())
}
fn prune_ended_operations(
    txn: &mut ActionTxn<'_>,
    handles: &SortieHandles,
    actor: ObjId,
    exclude: OperationId,
    touched: &std::collections::BTreeSet<FlightKey>,
    prune_ops: &[OperationId],
) -> Result<(), ActionError> {
    let mut remaining_flight_deletes = MAX_PRUNED_OP_FLIGHTS_PER_TXN;
    let mut removed_flights = std::collections::BTreeSet::new();
    for &op_id in prune_ops {
        if op_id.organizer != actor || op_id == exclude {
            continue;
        }
        let Some(op) = txn
            .get(&handles.operations, &op_id)
            .map_err(error)?
            .cloned()
        else {
            continue;
        };
        let Some(removable) =
            operation_prunable_flights(&mut &mut *txn, handles, &op, touched).map_err(error)?
        else {
            continue;
        };
        let removable: Vec<FlightKey> = removable
            .into_iter()
            .filter(|key| !removed_flights.contains(key))
            .collect();
        if removable.len() <= remaining_flight_deletes {
            for key in removable {
                txn.remove(&handles.flights, &key).map_err(error)?;
                removed_flights.insert(key);
                remaining_flight_deletes -= 1;
            }
            txn.remove(&handles.operations, &op_id).map_err(error)?;
        } else if remaining_flight_deletes > 0 {
            for key in removable.into_iter().take(remaining_flight_deletes) {
                txn.remove(&handles.flights, &key).map_err(error)?;
                removed_flights.insert(key);
            }
            break;
        }
    }
    Ok(())
}
fn chosen(i: &ActionInvocation) -> Result<(Intent, CanonicalHash, Vec<OperationId>), ActionError> {
    let Some(params) = i.parameters().as_object() else {
        return Err(error("Invalid flight selection."));
    };
    let valid_shape = params.len() == 3 || (params.len() == 4 && params.contains_key("prune_ops"));
    if i.principal() != &(PrincipalId::Object { id: i.actor() })
        || i.subject() != &PerceivedRef::SelfActor
        || !i.targets().is_empty()
        || !valid_shape
    {
        return Err(error("Invalid flight selection."));
    }
    let intent: Intent = serde_json::from_value(
        params
            .get("intent")
            .cloned()
            .ok_or_else(|| error("Missing intent."))?,
    )
    .map_err(error)?;
    if params.get("request").and_then(Value::as_str)
        != Some(digest(i.actor(), &intent)?.to_hex().as_str())
    {
        return Err(error("Flight request changed."));
    }
    let facts: CanonicalHash = serde_json::from_value(
        params
            .get("facts")
            .cloned()
            .ok_or_else(|| error("Missing facts."))?,
    )
    .map_err(error)?;
    let prune_ops = if let Some(prune_val) = params.get("prune_ops") {
        let ids: Vec<OperationId> = serde_json::from_value(prune_val.clone()).map_err(error)?;
        if ids.is_empty()
            || ids.len() > MAX_PRUNE_OPS_PER_TXN
            || ids.iter().any(|id| id.organizer != i.actor())
            || ids
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != ids.len()
        {
            return Err(error("Invalid operation pruning selection."));
        }
        ids
    } else {
        Vec::new()
    };
    Ok((intent, facts, prune_ops))
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
/// [`require_available`] for a command's read (DT-4: `+tv/lease return` and the
/// bay repossession): an unreadable row counts as unavailable.
pub(crate) fn ctx_available(ctx: &Ctx<'_>, unit: ObjId) -> bool {
    ctx.w
        .module_state::<SortieHandles>()
        .is_none_or(|h| available(&mut &*ctx, h, unit).unwrap_or(false))
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
        let prune_ops = if matches!(intent.command, Command::Launch { .. } | Command::End { .. }) {
            prunable_ended_operations(
                engine,
                &self.handles,
                q.actor(),
                snapshot.operation(),
                &snapshot.touched_flights(),
            )?
        } else {
            Vec::new()
        };
        let fixed_params = if prune_ops.is_empty() {
            json!({"intent":intent,"facts":snapshot.hash})
        } else {
            json!({"intent":intent,"facts":snapshot.hash,"prune_ops":prune_ops})
        };
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: coordinate(q.actor())?,
            generation: snapshot.generation(),
        }])
        .map_err(error)?;
        crate::surface::act::offer_draft_with_revisions(
            action,
            engine,
            fixed_params,
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
        let (intent, _, _) = chosen(r.invocation())?;
        // The fixed units/operators are re-read from this regenerated selection.
        // Core checks the real owner guards through final take.
        prepare::authority_keys(&intent, r.invocation().actor())
    }
    fn resolved_authority_witnesses(
        &self,
        r: &ResolvedActionInvocation,
        engine: &Engine,
    ) -> Result<Vec<AuthorityWitnessRequest>, ActionError> {
        let (intent, _, _) = chosen(r.invocation())?;
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
        let (intent, expected, prune_ops) = chosen(r.invocation())?;
        if let Command::Message(request) = &intent.command {
            super::field_orders::verify(ctx, txn, &self.handles, r.invocation().actor(), request)?;
        }
        if let Command::Refresh { operation, facts } = &intent.command {
            super::field::verify_owner_facts(ctx, txn, &self.handles, *operation, facts)?;
        }
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
        let op_id = snapshot.operation();
        let touched = snapshot.touched_flights();
        let output = snapshot.apply(txn, ctx, &self.handles, self.black_box.as_ref())?;
        if !prune_ops.is_empty()
            && matches!(intent.command, Command::Launch { .. } | Command::End { .. })
        {
            prune_ended_operations(
                txn,
                &self.handles,
                r.invocation().actor(),
                op_id,
                &touched,
                &prune_ops,
            )?;
        }
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
pub(super) fn enqueue(ctx: &mut Ctx<'_>, actor: ObjId, command: Command) -> Result<(), String> {
    let handles = ctx
        .w
        .module_state::<SortieHandles>()
        .filter(|h| h.enabled)
        .cloned()
        .ok_or("Deployment unavailable.")?;
    // M34: flush any lapses that already committed before validating/replacing operation state.
    flush_committed_lapses(ctx, &handles);
    if ctx.authoritative_pending(actor)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return Err("An authoritative request is already pending.".into());
    }
    let intent = Intent {
        utc: ctx.w.now(),
        ic: ctx.w.world_ic_now(),
        command,
    };
    // M34: validate caller rights and prepare snapshot BEFORE touching the global 256-slot
    // `Pending` queue, so invalid/rejected requests can never pollute queue capacity.
    if actor == ctx.actor {
        super::commands::check_caller_rights(ctx, &intent.command)?;
    }
    let snapshot = prepare::snapshot_unregistered(ctx.w, &mut &*ctx, &handles, actor, &intent)?;
    let digest = digest(actor, &intent).map_err(|e| e.to_string())?.to_hex();
    // A return/cancel/relaunch cycle can complete within one wall-clock second.
    // Its new owner preimages must not reuse the first departure's receipt.
    let key = IdempotencyKeyText::parse_canonical(format!(
        "detachment-flight/{}",
        snapshot.hash.to_hex()
    ))
    .map_err(|e| e.to_string())?;
    prune_settled(ctx, &handles);
    let queue = ctx
        .w
        .module_state_mut::<Pending>()
        .ok_or("Deployment unavailable.")?;
    if queue.0.len() >= 256 && !queue.0.contains_key(&actor) {
        return Err("Deployment is busy.".into());
    }
    queue.0.insert(actor, intent);
    queue_lapses(ctx, actor, &snapshot);
    ctx.out.defer(Deferred::AuthoritativeSubmit(Box::new(
        AuthoritativeSubmit {
            actor,
            action: handles.action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({"request":digest}),
            key,
        },
    )));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m34_told_lapses_evicts_fifo_instead_of_clearing_all_entries() {
        let mut told = ToldLapses::default();
        let op = OperationId {
            organizer: ObjId(1),
            draft: mush_tactics::detachment::planning::PlanId(1),
            activation: 1,
        };
        for seq in 1..=u64::try_from(MAX_LAPSE_BOOK).expect("cap") {
            assert!(told.insert((op, 1, seq)));
        }
        assert!(!told.insert((op, 1, 1)), "duplicate rejected");
        // Inserting one beyond cap evicts ONLY seq 1, keeping 2..=256 deduplicated.
        let overflow = u64::try_from(MAX_LAPSE_BOOK + 1).expect("cap+1");
        assert!(told.insert((op, 1, overflow)));
        assert_eq!(told.seen.len(), MAX_LAPSE_BOOK);
        assert!(
            !told.insert((op, 1, 2)),
            "recent entries must remain deduplicated after overflow"
        );
        assert!(
            told.insert((op, 1, 1)),
            "only the oldest entry was evicted by FIFO"
        );
    }
}
