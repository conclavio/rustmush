//! Authenticated quest commands commit exact ordered receipts through typed state.
use super::model::{
    CheckEvidence, MAX_CHECKPOINTS, MAX_QUESTS, QuestIndex, QuestKey, QuestRecord, Stamp,
};
pub use super::model::{MAX_INDEX_BYTES, MAX_RECORD_BYTES};
use crate::port::families::PortHandles;
use crate::port::passenger::store::{Booking, BookingKey};
use crate::port_tick::port_error;
use crate::surface::act::{offer_draft_with_revisions, parameter, self_query};
use mush_core::mechanics::OwnerId;
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalHash, CanonicalId, Ctx, DefinitionRef, Engine,
    IdempotencyKeyText, ModError, ObjId, ProjectedParameterKind, ProviderOfferDraft, Registrar,
    RevisionKey, RevisionKind, RevisionSet, RevisionStamp, RevisionValidator,
    RevisionValidatorContext, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const FAMILY: &str = "tacsim.quest";
pub const INDEX_FAMILY: &str = "tacsim.quest_index";
pub const ACTION: &str = "tacsim_quest";
pub const OUTCOME: &str = "tacsim_quest_result";
const REVISION: &str = "quest_world";

#[derive(Clone, Default)]
struct Projection {
    ready: bool,
    rows: Vec<QuestRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Order {
    before: Option<QuestRecord>,
    next: QuestRecord,
    /// Exact index CAS, without duplicating a potentially 32KiB index in offers.
    index_hash: Option<String>,
}
#[derive(Clone, Default)]
struct Orders(BTreeMap<ObjId, Order>);

pub fn get(ctx: &Ctx<'_>, key: &QuestKey) -> Result<Option<QuestRecord>, String> {
    key.check()?;
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .ok_or("quest service unavailable")?;
    let row = ctx
        .action_state(&handles.families.quests, key)
        .map_err(|_| "quest receipt unreadable")?
        .cloned();
    if let Some(row) = &row {
        row.check()?;
        if &row.key != key {
            return Err("quest receipt identity mismatch".into());
        }
    }
    Ok(row)
}
fn index(ctx: &Ctx<'_>) -> Result<Option<QuestIndex>, String> {
    let handles = ctx
        .w
        .module_state::<PortHandles>()
        .ok_or("quest service unavailable")?;
    let index = ctx
        .action_state(&handles.families.quest_index, &0_u8)
        .map_err(|_| "quest index unreadable")?
        .cloned();
    if let Some(index) = &index {
        index.check()?;
    }
    Ok(index)
}
pub fn list(ctx: &Ctx<'_>) -> Result<Vec<QuestRecord>, String> {
    index(ctx)?
        .unwrap_or_default()
        .keys
        .into_iter()
        .map(|key| get(ctx, &key)?.ok_or_else(|| "quest index names a missing receipt".into()))
        .collect()
}
/// Display only. Failed typed reads clear the projection rather than imply no quest.
pub fn refresh(ctx: &mut Ctx<'_>) -> Result<Vec<QuestRecord>, String> {
    let rows = match list(ctx) {
        Ok(rows) => rows,
        Err(error) => {
            if let Some(projection) = ctx.w.module_state_mut::<Projection>() {
                *projection = Projection::default();
            }
            return Err(error);
        }
    };
    let projection = ctx
        .w
        .module_state_mut::<Projection>()
        .ok_or("quest service unavailable")?;
    projection.ready = true;
    projection.rows.clone_from(&rows);
    let stale: Vec<_> = ctx
        .w
        .module_state::<Orders>()
        .into_iter()
        .flat_map(|orders| orders.0.iter())
        .filter(|(_, order)| {
            rows.iter()
                .any(|row| row.key == order.next.key && row.revision >= order.next.revision)
                || !order_valid(ctx.w, order)
        })
        .map(|(actor, _)| *actor)
        .collect();
    if let Some(orders) = ctx.w.module_state_mut::<Orders>() {
        for actor in stale {
            orders.0.remove(&actor);
        }
    }
    Ok(rows)
}
pub fn projection_ready(w: &World) -> bool {
    w.module_state::<Projection>()
        .is_some_and(|projection| projection.ready)
}
pub fn list_world(w: &World) -> Vec<QuestRecord> {
    w.module_state::<Projection>()
        .map(|projection| projection.rows.clone())
        .unwrap_or_default()
}

/// Pure invariants also checked during prepare; world predicates are additional.
pub(crate) fn progress_valid(before: Option<&QuestRecord>, next: &QuestRecord) -> bool {
    if next.check().is_err() {
        return false;
    }
    let Some(before) = before else {
        return next.revision == 1 && next.checks.is_empty() && next.completed.is_none();
    };
    if before.check().is_err()
        || before.completed.is_some()
        || before.revision.checked_add(1) != Some(next.revision)
    {
        return false;
    }
    let mut fixed = next.clone();
    fixed.revision = before.revision;
    fixed.checks.clone_from(&before.checks);
    fixed.completed.clone_from(&before.completed);
    if fixed != *before {
        return false;
    }
    if next.completed.is_some() {
        next.checks == before.checks && before.checks.len() == before.definition.checkpoints.len()
    } else {
        next.checks.len() == before.checks.len() + 1 && next.checks.starts_with(&before.checks)
    }
}
fn order_valid(w: &World, order: &Order) -> bool {
    progress_valid(order.before.as_ref(), &order.next)
        && super::transition_allowed(w, order.before.as_ref(), &order.next)
}
fn hash<T: Serialize>(domain: &str, value: &T, limit: usize) -> Result<String, String> {
    let value = serde_json::to_value(value).map_err(|_| "quest revision cannot be encoded")?;
    CanonicalHash::for_canonical_value(domain, 1, &value, limit)
        .map(|hash| hash.to_hex())
        .map_err(|_| "quest revision cannot be hashed".into())
}
fn index_hash(index: Option<&QuestIndex>) -> Result<Option<String>, String> {
    index
        .map(|index| {
            index.check()?;
            hash("tacsim.quest_index.v1", index, MAX_INDEX_BYTES)
        })
        .transpose()
}
fn indexed(order: &Order, index: Option<&QuestIndex>) -> bool {
    let contains = index.is_some_and(|index| index.keys.binary_search(&order.next.key).is_ok());
    contains == order.before.is_some()
        && (order.before.is_some() || index.is_none_or(|index| index.keys.len() < MAX_QUESTS))
}
fn submit(ctx: &mut Ctx<'_>, order: Order) -> Result<(), String> {
    if ctx.actor != order.next.actor
        || !order_valid(ctx.w, &order)
        || crate::port::passenger::store::get(ctx, order.next.key.booking)?.as_ref()
            != Some(&order.next.passage)
    {
        return Err("quest change is no longer authorized".into());
    }
    let action = ctx
        .w
        .module_state::<PortHandles>()
        .and_then(|handles| handles.quest.clone())
        .ok_or("quest actions unavailable")?;
    let key = &order.next.key;
    let revision = order.next.revision;
    let parameters = json!({"source":key.source,"booking":key.booking,"revision":revision});
    let digest = hash("tacsim.quest_key.v1", key, 1024)?;
    let action_key =
        IdempotencyKeyText::parse_canonical(format!("tacsim.quest/{digest}/{revision}"))
            .map_err(|_| "quest action key invalid")?;
    let orders = ctx
        .w
        .module_state_mut::<Orders>()
        .ok_or("quest actions unavailable")?;
    if orders.0.len() >= MAX_QUESTS && !orders.0.contains_key(&ctx.actor) {
        return Err("quest actions busy".into());
    }
    orders.0.insert(ctx.actor, order);
    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        parameters,
        action_key,
    );
    Ok(())
}
pub fn stage_accept(ctx: &mut Ctx<'_>, mut row: QuestRecord) -> Result<(), String> {
    if get(ctx, &row.key)?.is_some() {
        return Err("this quest already has a receipt".into());
    }
    row.accepted = Stamp::now(ctx.w);
    let previous = index(ctx)?;
    let order = Order {
        before: None,
        next: row,
        index_hash: index_hash(previous.as_ref())?,
    };
    if !indexed(&order, previous.as_ref()) {
        return Err("quest index is full or inconsistent".into());
    }
    submit(ctx, order)
}
fn stage(ctx: &mut Ctx<'_>, before: &QuestRecord, next: QuestRecord) -> Result<(), String> {
    if get(ctx, &before.key)?.as_ref() != Some(before) {
        return Err("quest receipt changed".into());
    }
    let current = index(ctx)?;
    let order = Order {
        before: Some(before.clone()),
        next,
        index_hash: index_hash(current.as_ref())?,
    };
    if !indexed(&order, current.as_ref()) {
        return Err("quest index is inconsistent".into());
    }
    submit(ctx, order)
}
pub fn stage_check(
    ctx: &mut Ctx<'_>,
    before: &QuestRecord,
    mut evidence: CheckEvidence,
) -> Result<(), String> {
    let mut next = before.clone();
    evidence.at = Stamp::now(ctx.w);
    next.checks.push(evidence);
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or("quest revision exhausted")?;
    stage(ctx, before, next)
}
pub fn stage_finish(ctx: &mut Ctx<'_>, before: &QuestRecord) -> Result<(), String> {
    let mut next = before.clone();
    next.completed = Some(Stamp::now(ctx.w));
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or("quest revision exhausted")?;
    stage(ctx, before, next)
}

fn error(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::Integrity, message)
}
fn generation(
    w: &World,
    actor: ObjId,
    current: Option<&QuestRecord>,
    passage: Option<&Booking>,
    index: Option<&QuestIndex>,
) -> Result<u64, ActionError> {
    let order = w
        .module_state::<Orders>()
        .and_then(|orders| orders.0.get(&actor));
    let index_digest = index_hash(index).map_err(|_| error("quest index invalid"))?;
    let valid = order.is_some_and(|order| {
        order.next.actor == actor
            && order_valid(w, order)
            && order.before.as_ref() == current
            && passage == Some(&order.next.passage)
            && order.index_hash == index_digest
            && indexed(order, index)
    });
    let current_digest = current
        .map(|row| hash("tacsim.quest_record.v1", row, MAX_RECORD_BYTES))
        .transpose()
        .map_err(|_| error("quest receipt invalid"))?;
    let passage_digest = passage
        .map(|row| {
            hash(
                "tacsim.quest_passage.v1",
                row,
                crate::port::passenger::store::MAX_RECORD_BYTES,
            )
        })
        .transpose()
        .map_err(|_| error("quest passage invalid"))?;
    let digest = hash("tacsim.quest_world.v1", &json!({"order":order,"valid":valid,"current":current_digest,"passage":passage_digest,"index":index_digest}), 64 * 1024)
        .map_err(|_| error("quest revision invalid"))?;
    u64::from_str_radix(
        digest
            .get(..16)
            .ok_or_else(|| error("quest revision invalid"))?,
        16,
    )
    .map_err(|_| error("quest revision invalid"))
}
fn revisions(actor: ObjId, generation: u64) -> Result<RevisionSet, ActionError> {
    RevisionSet::from_stamps([RevisionStamp {
        key: RevisionKey {
            owner: OwnerId::new(crate::NS).map_err(|_| error("quest owner invalid"))?,
            kind: RevisionKind::new(
                CanonicalId::new(REVISION).map_err(|_| error("quest revision invalid"))?,
            ),
            subject: SubjectKey::Object { id: actor },
            facet: None,
        },
        generation,
    }])
    .map_err(|_| error("quest revision invalid"))
}
struct Validator {
    records: StateFamilyHandle<QuestKey, QuestRecord>,
    index: StateFamilyHandle<u8, QuestIndex>,
    passengers: StateFamilyHandle<BookingKey, Booking>,
}
impl RevisionValidator for Validator {
    fn current_generation(
        &self,
        ctx: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let SubjectKey::Object { id: actor } = key.subject else {
            return Ok(None);
        };
        let Some(order) = ctx
            .world()
            .module_state::<Orders>()
            .and_then(|orders| orders.0.get(&actor))
        else {
            return Ok(None);
        };
        let current = ctx
            .action_state(&self.records, &order.next.key)
            .map_err(|_| error("quest receipt unreadable"))?;
        let passage = ctx
            .action_state(&self.passengers, &BookingKey(order.next.key.booking))
            .map_err(|_| error("quest passage unreadable"))?;
        let index = ctx
            .action_state(&self.index, &0_u8)
            .map_err(|_| error("quest index unreadable"))?;
        generation(ctx.world(), actor, current, passage, index).map(Some)
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    records: StateFamilyHandle<QuestKey, QuestRecord>,
    index: StateFamilyHandle<u8, QuestIndex>,
    passengers: StateFamilyHandle<BookingKey, Booking>,
) -> Result<(), ModError> {
    reg.state(Orders::default());
    reg.state(Projection::default());
    reg.revision_validator(
        RevisionKind::new(
            CanonicalId::new(REVISION).map_err(|error| ModError::msg(error.to_string()))?,
        ),
        Validator {
            records,
            index,
            passengers,
        },
    )
}
pub(crate) struct Provider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub records: StateFamilyHandle<QuestKey, QuestRecord>,
    pub index: StateFamilyHandle<u8, QuestIndex>,
    pub passengers: StateFamilyHandle<BookingKey, Booking>,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = invocation.parameters();
        let key = parameters
            .get("source")
            .and_then(Value::as_str)
            .zip(parameters.get("booking").and_then(Value::as_u64))
            .map(|(source, booking)| QuestKey {
                source: source.into(),
                booking,
            });
        if invocation.subject() != &mush_core::PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
            || key.is_none_or(|key| key.check().is_err())
            || parameters
                .get("revision")
                .and_then(Value::as_u64)
                .is_none_or(|revision| revision == 0 || revision > MAX_CHECKPOINTS as u64 + 2)
            || parameters.as_object().is_none_or(|object| {
                object
                    .keys()
                    .any(|key| !matches!(key.as_str(), "source" | "booking" | "revision" | "order"))
            })
        {
            return Err(error(
                "quest action must name its source, booking and revision",
            ));
        }
        Ok(())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self_query(action, &self.action, query) {
            return Ok(None);
        }
        let Some(order) = engine
            .world
            .module_state::<Orders>()
            .and_then(|orders| orders.0.get(&query.actor()))
            .cloned()
        else {
            return Ok(None);
        };
        if order.next.actor != query.actor() || !order_valid(&engine.world, &order) {
            return Ok(None);
        }
        let current = engine
            .action_state()
            .get(&self.records, &order.next.key)
            .map_err(|_| error("quest receipt unreadable"))?;
        let passage = engine
            .action_state()
            .get(&self.passengers, &BookingKey(order.next.key.booking))
            .map_err(|_| error("quest passage unreadable"))?;
        let index = engine
            .action_state()
            .get(&self.index, &0_u8)
            .map_err(|_| error("quest index unreadable"))?;
        if current != order.before.as_ref()
            || passage != Some(&order.next.passage)
            || index_hash(index).map_err(|_| error("quest index invalid"))? != order.index_hash
            || !indexed(&order, index)
        {
            return Ok(None);
        }
        let stamp = generation(&engine.world, query.actor(), current, passage, index)?;
        offer_draft_with_revisions(
            action,
            engine,
            json!({"order":order}),
            vec![
                parameter(
                    "source",
                    true,
                    ProjectedParameterKind::Choice {
                        choices: vec![order.next.key.source.clone()],
                    },
                )?,
                parameter(
                    "booking",
                    true,
                    ProjectedParameterKind::Integer {
                        minimum: 0,
                        maximum: i64::MAX,
                    },
                )?,
                parameter(
                    "revision",
                    true,
                    ProjectedParameterKind::Integer {
                        minimum: 1,
                        maximum: MAX_CHECKPOINTS as i64 + 2,
                    },
                )?,
            ],
            (
                "tacsim-quest",
                "Record quest progress",
                "Commit an authenticated, owner-verified quest step.",
            ),
            revisions(query.actor(), stamp)?,
        )
        .map(Some)
    }
    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let order: Order = invocation
            .parameters()
            .get("order")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| error("quest action has no authenticated order"))?;
        if !progress_valid(order.before.as_ref(), &order.next)
            || order.next.actor != invocation.actor()
            || invocation
                .parameters()
                .get("source")
                .and_then(Value::as_str)
                != Some(order.next.key.source.as_str())
            || invocation
                .parameters()
                .get("booking")
                .and_then(Value::as_u64)
                != Some(order.next.key.booking)
            || invocation
                .parameters()
                .get("revision")
                .and_then(Value::as_u64)
                != Some(order.next.revision)
        {
            return Err(error("quest action names another transition"));
        }
        if txn
            .get(&self.records, &order.next.key)
            .map_err(|_| error("quest receipt unreadable"))?
            != order.before.as_ref()
            || txn
                .get(&self.passengers, &BookingKey(order.next.key.booking))
                .map_err(|_| error("quest passage unreadable"))?
                != Some(&order.next.passage)
        {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "quest or original passage changed",
            ));
        }
        let current_index = txn
            .get(&self.index, &0_u8)
            .map_err(|_| error("quest index unreadable"))?
            .cloned();
        if index_hash(current_index.as_ref()).map_err(|_| error("quest index invalid"))?
            != order.index_hash
            || !indexed(&order, current_index.as_ref())
        {
            return Err(port_error(ActionErrorCode::Conflict, "quest index changed"));
        }
        let row = order.next;
        if order.before.is_none() {
            let existed = current_index.is_some();
            let mut next = current_index.unwrap_or_default();
            let position = next
                .keys
                .binary_search(&row.key)
                .err()
                .ok_or_else(|| error("quest already indexed"))?;
            next.keys.insert(position, row.key.clone());
            next.check().map_err(|_| error("quest index full"))?;
            if existed {
                txn.replace(&self.index, &0, next)
            } else {
                txn.insert(&self.index, &0, next)
            }
            .map_err(|_| error("quest index write failed"))?;
            txn.insert(&self.records, &row.key, row.clone())
        } else {
            txn.replace(&self.records, &row.key, row.clone())
        }
        .map_err(|_| error("quest receipt write failed"))?;
        let outcome = RulesValue::new(self.outcome.clone(), json!({"actor":row.actor.0,"npc":row.npc.0,"source":row.key.source,"booking":row.key.booking,"revision":row.revision,"checked":row.checks.len(),"completed":row.completed.is_some()}))
            .map_err(|_| error("quest outcome invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{detail_hash, tests::record};
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn only_one_ordered_step_or_final_completion_can_advance_frozen_terms() -> TestResult {
        let before = record();
        check!(progress_valid(None, &before));
        let mut checked = before.clone();
        checked.checks.push(CheckEvidence {
            checkpoint: "bench".into(),
            room: ObjId(20),
            detail_hash: detail_hash("Scale reads 42.")?,
            detail_text: "Scale reads 42.".into(),
            at: Stamp {
                at_ic: 70,
                at_utc: 70,
            },
        });
        checked.revision += 1;
        check!(progress_valid(Some(&before), &checked));
        check!(!progress_valid(None, &checked));
        let mut finished = checked.clone();
        finished.completed = Some(Stamp {
            at_ic: 80,
            at_utc: 80,
        });
        finished.revision += 1;
        check!(progress_valid(Some(&checked), &finished));
        check!(!progress_valid(Some(&before), &finished));
        check!(!progress_valid(Some(&finished), &finished));
        let mut altered = checked.clone();
        altered.definition.completion = "A different promised reward".into();
        check!(!progress_valid(Some(&before), &altered));
        altered = checked.clone();
        altered.accepted.at_ic += 1;
        check!(!progress_valid(Some(&before), &altered));
        altered = checked;
        altered.passage.hull = ObjId(99);
        check!(!progress_valid(Some(&before), &altered));
        Ok(())
    }

    #[test]
    fn index_cas_distinguishes_absence_and_keeps_completed_instances_spent() -> TestResult {
        let row = record();
        let empty = QuestIndex::default();
        check!(index_hash(None)? != index_hash(Some(&empty))?);
        let order = Order {
            before: None,
            next: row.clone(),
            index_hash: None,
        };
        check!(indexed(&order, None));
        let indexed_row = QuestIndex {
            schema: 1,
            keys: vec![row.key.clone()],
        };
        check!(!indexed(&order, Some(&indexed_row)));
        check!(indexed(
            &Order {
                before: Some(row.clone()),
                next: row,
                index_hash: index_hash(Some(&indexed_row))?
            },
            Some(&indexed_row)
        ));
        check!(serde_json::to_vec(&order)?.len() < mush_core::MAX_ACTION_PARAMETER_BYTES);
        Ok(())
    }
}
