//! Committed passenger bookings. The current index is bounded; receipts are permanent.
use std::collections::BTreeMap;

use crate::port::families::PortHandles;
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

pub const FAMILY: &str = "tacsim.passenger";
pub const INDEX_FAMILY: &str = "tacsim.passenger_index";
pub const ACTION: &str = "tacsim_passenger";
pub const OUTCOME: &str = "tacsim_passenger_result";
pub const MAX_RECORD_BYTES: usize = 4096;
const REVISION: &str = "passenger_world";
const LIMIT: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BookingKey(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Booked,
    Aboard,
    Delivered,
    Cancelled,
}

impl Phase {
    pub fn word(self) -> &'static str {
        match self {
            Self::Booked => "booked",
            Self::Aboard => "aboard",
            Self::Delivered => "delivered",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeStamp {
    pub at_ic: i64,
    pub at_utc: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JourneyProof {
    pub route: String,
    pub departed_ic: i64,
    pub arrival_ic: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Booking {
    pub schema: u16,
    pub id: u64,
    pub revision: u64,
    pub npc: ObjId,
    pub captain: ObjId,
    pub hull: ObjId,
    pub traveller_key: String,
    pub origin: String,
    pub destination: String,
    pub purse: u64,
    pub payee: u64,
    pub fare_minor: i64,
    pub accepted_ic: i64,
    pub deadline_ic: i64,
    pub contract: u64,
    pub phase: Phase,
    pub journey: Option<JourneyProof>,
    pub delivered: Option<OutcomeStamp>,
    pub cancelled: Option<OutcomeStamp>,
}

impl Booking {
    pub fn check(&self) -> Result<(), String> {
        let valid_name = |s: &str| !s.is_empty() && s.len() <= 64 && CanonicalId::new(s).is_ok();
        if self.schema != 1
            || self.id != self.contract
            || self.id > i64::MAX as u64
            || self.revision == 0
            || self.revision > i64::MAX as u64
            || self.npc.0 == 0
            || self.captain.0 == 0
            || self.hull.0 == 0
            || self.npc == self.captain
            || self.purse == self.payee
            || self.fare_minor <= 0
            || self.traveller_key.is_empty()
            || self.traveller_key.len() > 160
            || !self
                .traveller_key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
            || !valid_name(&self.origin)
            || !valid_name(&self.destination)
            || self.origin == self.destination
            || self.deadline_ic <= self.accepted_ic
            || (self.delivered.is_some() != (self.phase == Phase::Delivered))
            || (self.cancelled.is_some() != (self.phase == Phase::Cancelled))
            || self
                .delivered
                .as_ref()
                .is_some_and(|x| x.at_ic < self.accepted_ic || x.at_ic > self.deadline_ic)
            || self
                .cancelled
                .as_ref()
                .is_some_and(|x| x.at_ic < self.accepted_ic)
            || (self.phase == Phase::Delivered && self.journey.is_none())
            || self.journey.as_ref().is_some_and(|j| {
                !valid_name(&j.route)
                    || j.departed_ic < self.accepted_ic
                    || j.arrival_ic < j.departed_ic
                    || self
                        .delivered
                        .as_ref()
                        .is_some_and(|done| done.at_ic < j.arrival_ic)
            })
        {
            return Err("invalid passenger booking".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookingIndex {
    pub schema: u16,
    pub current: BTreeMap<u32, u64>,
}
impl Default for BookingIndex {
    fn default() -> Self {
        Self {
            schema: 1,
            current: BTreeMap::new(),
        }
    }
}
impl BookingIndex {
    pub fn check(&self) -> Result<(), String> {
        if self.schema != 1
            || self.current.len() > LIMIT
            || self
                .current
                .iter()
                .any(|(npc, id)| *npc == 0 || *id > i64::MAX as u64)
        {
            Err("invalid passenger index".into())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Default)]
struct Projection {
    ready: bool,
    rows: Vec<Booking>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Order {
    before: Option<Booking>,
    next: Booking,
    index: Option<BookingIndex>,
}
#[derive(Clone, Default)]
struct Orders(BTreeMap<ObjId, Order>);

pub fn get(ctx: &Ctx<'_>, id: u64) -> Result<Option<Booking>, String> {
    let h = ctx
        .w
        .module_state::<PortHandles>()
        .ok_or("passenger service unavailable")?;
    let row = ctx
        .action_state(&h.families.passengers, &BookingKey(id))
        .map_err(|_| "passenger receipt unreadable")?
        .cloned();
    if let Some(row) = &row {
        row.check()?;
        if row.id != id {
            return Err("passenger receipt identity mismatch".into());
        }
    }
    Ok(row)
}
fn index(ctx: &Ctx<'_>) -> Result<Option<BookingIndex>, String> {
    let h = ctx
        .w
        .module_state::<PortHandles>()
        .ok_or("passenger service unavailable")?;
    let row = ctx
        .action_state(&h.families.passenger_index, &0_u8)
        .map_err(|_| "passenger index unreadable")?
        .cloned();
    if let Some(row) = &row {
        row.check()?;
    }
    Ok(row)
}
pub fn list(ctx: &Ctx<'_>) -> Result<Vec<Booking>, String> {
    index(ctx)?
        .unwrap_or_default()
        .current
        .into_iter()
        .map(|(npc, id)| {
            let row = get(ctx, id)?.ok_or("passenger index names a missing receipt")?;
            if row.npc.0 != npc {
                return Err("passenger index identity mismatch".into());
            }
            Ok(row)
        })
        .collect()
}
/// A display-only projection. Authoritative callers must read the typed family.
pub fn refresh(ctx: &mut Ctx<'_>) -> Result<Vec<Booking>, String> {
    let rows = match list(ctx) {
        Ok(rows) => rows,
        Err(error) => {
            if let Some(projection) = ctx.w.module_state_mut::<Projection>() {
                *projection = Projection::default();
            }
            return Err(error);
        }
    };
    for row in &rows {
        super::economy::refresh_receipt(ctx, row);
    }
    let projection = ctx
        .w
        .module_state_mut::<Projection>()
        .ok_or("passenger service unavailable")?;
    projection.ready = true;
    projection.rows.clone_from(&rows);
    let stale: Vec<_> = ctx
        .w
        .module_state::<Orders>()
        .into_iter()
        .flat_map(|orders| orders.0.iter())
        .filter(|(_, order)| {
            rows.iter()
                .any(|row| row.id == order.next.id && row.revision >= order.next.revision)
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
    w.module_state::<Projection>().is_some_and(|p| p.ready)
}
pub fn list_world(w: &World) -> Vec<Booking> {
    w.module_state::<Projection>()
        .map(|p| p.rows.clone())
        .unwrap_or_default()
}
fn order_valid(w: &World, order: &Order) -> bool {
    let next = &order.next;
    if next.check().is_err() || !super::transition_allowed(w, order.before.as_ref(), next) {
        return false;
    }
    match &order.before {
        None => {
            next.phase == Phase::Booked
                && next.revision == 1
                && next.journey.is_none()
                && super::economy::offered(w, next)
        }
        Some(before) => {
            if before.check().is_err() || before.revision.checked_add(1) != Some(next.revision) {
                return false;
            }
            let mut unchanged = next.clone();
            unchanged.revision = before.revision;
            unchanged.phase = before.phase;
            unchanged.journey.clone_from(&before.journey);
            unchanged.delivered.clone_from(&before.delivered);
            unchanged.cancelled.clone_from(&before.cancelled);
            unchanged == *before
                && matches!(
                    (before.phase, next.phase),
                    (Phase::Booked, Phase::Aboard | Phase::Cancelled)
                        | (Phase::Aboard, Phase::Delivered | Phase::Cancelled)
                )
                && (super::economy::active(w, before)
                    || (next.phase == Phase::Cancelled && super::economy::replaceable(w, before)))
        }
    }
}

fn submit(ctx: &mut Ctx<'_>, order: Order) -> Result<(), String> {
    if ctx.actor != order.next.captain || !order_valid(ctx.w, &order) {
        return Err("passenger change is no longer authorized".into());
    }
    let action = ctx
        .w
        .module_state::<PortHandles>()
        .and_then(|h| h.passenger.clone())
        .ok_or("passenger actions unavailable")?;
    let id = order.next.id;
    let revision = order.next.revision;
    let key = IdempotencyKeyText::parse_canonical(format!("tacsim.passenger/{id}/{revision}"))
        .map_err(|_| "passenger action key invalid")?;
    let orders = ctx
        .w
        .module_state_mut::<Orders>()
        .ok_or("passenger actions unavailable")?;
    if orders.0.len() >= LIMIT && !orders.0.contains_key(&ctx.actor) {
        return Err("passenger actions busy".into());
    }
    orders.0.insert(ctx.actor, order);
    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        json!({"booking":id,"revision":revision}),
        key,
    );
    Ok(())
}
pub fn stage_accept(ctx: &mut Ctx<'_>, row: Booking) -> Result<(), String> {
    if get(ctx, row.id)?.is_some() {
        return Err("passenger booking already exists".into());
    }
    let previous = index(ctx)?;
    for old in list(ctx)? {
        if (old.npc == row.npc || old.captain == row.captain || old.hull == row.hull)
            && !super::economy::replaceable(ctx.w, &old)
        {
            return Err("the passenger or ship already has a booking".into());
        }
    }
    submit(
        ctx,
        Order {
            before: None,
            next: row,
            index: previous,
        },
    )
}
pub fn stage_transition(
    ctx: &mut Ctx<'_>,
    before: &Booking,
    phase: Phase,
    journey: Option<JourneyProof>,
) -> Result<(), String> {
    if get(ctx, before.id)?.as_ref() != Some(before) {
        return Err("passenger booking changed".into());
    }
    let mut next = before.clone();
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or("passenger revision exhausted")?;
    next.phase = phase;
    next.journey = journey.or_else(|| before.journey.clone());
    let at_ic = if phase == Phase::Delivered {
        next.journey
            .as_ref()
            .ok_or("passenger arrival has no journey proof")?
            .arrival_ic
    } else {
        ctx.w.world_ic_now().at
    };
    let stamp = OutcomeStamp {
        at_ic,
        at_utc: ctx.w.now(),
    };
    match phase {
        Phase::Delivered => next.delivered = Some(stamp),
        Phase::Cancelled => next.cancelled = Some(stamp),
        _ => {}
    }
    submit(
        ctx,
        Order {
            before: Some(before.clone()),
            next,
            index: None,
        },
    )
}

fn error(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::Integrity, message)
}
fn generation(w: &World, actor: ObjId, rows: &[Booking]) -> Result<u64, ActionError> {
    let order = w.module_state::<Orders>().and_then(|x| x.0.get(&actor));
    let valid = order.is_some_and(|o| order_valid(w, o));
    let reusable: Vec<_> = rows
        .iter()
        .map(|r| (r.id, super::economy::replaceable(w, r)))
        .collect();
    let hash = CanonicalHash::for_canonical_value(
        "tacsim.passenger_world.v1",
        1,
        &json!({"order":order,"valid":valid,"reusable":reusable}),
        65536,
    )
    .map_err(|_| error("passenger revision invalid"))?;
    u64::from_str_radix(
        hash.to_hex()
            .get(..16)
            .ok_or_else(|| error("passenger revision invalid"))?,
        16,
    )
    .map_err(|_| error("passenger revision invalid"))
}
fn revisions(actor: ObjId, generation: u64) -> Result<RevisionSet, ActionError> {
    RevisionSet::from_stamps([RevisionStamp {
        key: RevisionKey {
            owner: OwnerId::new(crate::NS).map_err(|_| error("passenger owner invalid"))?,
            kind: RevisionKind::new(
                CanonicalId::new(REVISION).map_err(|_| error("passenger revision invalid"))?,
            ),
            subject: SubjectKey::Object { id: actor },
            facet: None,
        },
        generation,
    }])
    .map_err(|_| error("passenger revision invalid"))
}
struct Validator {
    bookings: StateFamilyHandle<BookingKey, Booking>,
    index: StateFamilyHandle<u8, BookingIndex>,
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
        let index = ctx
            .action_state(&self.index, &0)
            .map_err(|_| error("passenger index unreadable"))?
            .cloned()
            .unwrap_or_default();
        index
            .check()
            .map_err(|_| error("passenger index invalid"))?;
        let mut rows = Vec::new();
        for (npc, id) in index.current {
            let row = ctx
                .action_state(&self.bookings, &BookingKey(id))
                .map_err(|_| error("passenger receipt unreadable"))?
                .ok_or_else(|| error("passenger receipt missing"))?;
            if row.npc.0 != npc || row.id != id || row.check().is_err() {
                return Err(error("passenger index invalid"));
            }
            rows.push(row.clone());
        }
        generation(ctx.world(), actor, &rows).map(Some)
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    bookings: StateFamilyHandle<BookingKey, Booking>,
    index: StateFamilyHandle<u8, BookingIndex>,
) -> Result<(), ModError> {
    reg.state(Orders::default());
    reg.state(Projection::default());
    super::economy::register(reg);
    reg.revision_validator(
        RevisionKind::new(CanonicalId::new(REVISION).map_err(|e| ModError::msg(e.to_string()))?),
        Validator { bookings, index },
    )
}
pub(crate) struct Provider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub bookings: StateFamilyHandle<BookingKey, Booking>,
    pub index: StateFamilyHandle<u8, BookingIndex>,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, i: &ActionInvocation) -> Result<(), ActionError> {
        if i.subject() != &mush_core::PerceivedRef::SelfActor
            || !i.targets().is_empty()
            || i.parameters()
                .get("booking")
                .and_then(Value::as_u64)
                .is_none()
            || i.parameters()
                .get("revision")
                .and_then(Value::as_u64)
                .is_none()
        {
            return Err(error("passenger action must name its booking and revision"));
        }
        Ok(())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &mush_core::LegalActionQuery,
        e: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self_query(action, &self.action, q) {
            return Ok(None);
        }
        let Some(order) = e
            .world
            .module_state::<Orders>()
            .and_then(|x| x.0.get(&q.actor()))
            .cloned()
        else {
            return Ok(None);
        };
        if !order_valid(&e.world, &order) {
            return Ok(None);
        }
        let index = e
            .action_state()
            .get(&self.index, &0)
            .map_err(|_| error("passenger index unreadable"))?
            .cloned()
            .unwrap_or_default();
        let mut rows = Vec::new();
        for id in index.current.values() {
            rows.push(
                e.action_state()
                    .get(&self.bookings, &BookingKey(*id))
                    .map_err(|_| error("passenger receipt unreadable"))?
                    .ok_or_else(|| error("passenger receipt missing"))?
                    .clone(),
            );
        }
        let stamp = generation(&e.world, q.actor(), &rows)?;
        offer_draft_with_revisions(
            action,
            e,
            json!({"order":order}),
            vec![
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
                        maximum: i64::MAX,
                    },
                )?,
            ],
            (
                "tacsim-passenger",
                "Record passenger booking",
                "Commit an authenticated passenger booking transition.",
            ),
            revisions(q.actor(), stamp)?,
        )
        .map(Some)
    }
    fn prepare_authoritative(
        &self,
        i: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(i)?;
        let order: Order = i
            .parameters()
            .get("order")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| error("passenger action has no authenticated order"))?;
        let row = order.next;
        row.check().map_err(|_| error("passenger record invalid"))?;
        if row.captain != i.actor()
            || i.parameters().get("booking").and_then(Value::as_u64) != Some(row.id)
            || i.parameters().get("revision").and_then(Value::as_u64) != Some(row.revision)
        {
            return Err(error("passenger action names another booking"));
        }
        let key = BookingKey(row.id);
        if txn
            .get(&self.bookings, &key)
            .map_err(|_| error("passenger receipt unreadable"))?
            != order.before.as_ref()
        {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "passenger booking changed",
            ));
        }
        if order.before.is_none() {
            let current = txn
                .get(&self.index, &0)
                .map_err(|_| error("passenger index unreadable"))?
                .cloned();
            if current != order.index {
                return Err(port_error(
                    ActionErrorCode::Conflict,
                    "passenger index changed",
                ));
            }
            let existed = current.is_some();
            let mut next = current.unwrap_or_default();
            next.current.insert(row.npc.0, row.id);
            next.check().map_err(|_| error("passenger index full"))?;
            if existed {
                txn.replace(&self.index, &0, next)
            } else {
                txn.insert(&self.index, &0, next)
            }
            .map_err(|_| error("passenger index write failed"))?;
            txn.insert(&self.bookings, &key, row.clone())
        } else {
            txn.replace(&self.bookings, &key, row.clone())
        }
        .map_err(|_| error("passenger receipt write failed"))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"actor":i.actor().0,"booking":row.id,"phase":row.phase.word()}),
        )
        .map_err(|_| error("passenger outcome invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![i.principal().clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    pub(super) fn booking() -> Booking {
        Booking {
            schema: 1,
            id: 0,
            revision: 1,
            npc: ObjId(10),
            captain: ObjId(11),
            hull: ObjId(12),
            traveller_key: "tacsim.passengers.stickney/passenger.leila".into(),
            origin: "phobos_port".into(),
            destination: "ceres_port".into(),
            purse: 0,
            payee: 1,
            fare_minor: 50000,
            accepted_ic: 100,
            deadline_ic: 200,
            contract: 0,
            phase: Phase::Booked,
            journey: None,
            delivered: None,
            cancelled: None,
        }
    }

    #[test]
    fn booking_accepts_first_economic_ids_and_authored_slash_key() -> TestResult {
        let row = booking();
        check!(row.check().is_ok());
        let copy: Booking = serde_json::from_value(serde_json::to_value(&row)?)?;
        check_eq!(copy, row);
        Ok(())
    }

    #[test]
    fn delivery_needs_a_real_ordered_journey_and_excludes_cancellation() -> TestResult {
        let mut row = booking();
        row.phase = Phase::Delivered;
        row.delivered = Some(OutcomeStamp {
            at_ic: 150,
            at_utc: 1000,
        });
        check!(row.check().is_err());
        row.journey = Some(JourneyProof {
            route: "phobos_ceres".into(),
            departed_ic: 110,
            arrival_ic: 150,
        });
        check!(row.check().is_ok());
        row.cancelled = Some(OutcomeStamp {
            at_ic: 150,
            at_utc: 1000,
        });
        check!(row.check().is_err());
        row.cancelled = None;
        row.delivered.as_mut().ok_or("missing test receipt")?.at_ic = 140;
        check!(row.check().is_err());
        Ok(())
    }

    #[test]
    fn index_is_bounded_and_unknown_receipt_fields_fail_closed() -> TestResult {
        let mut index = BookingIndex::default();
        for npc in 1..=128 {
            index.current.insert(npc, u64::from(npc));
        }
        check!(index.check().is_ok());
        index.current.insert(129, 129);
        check!(index.check().is_err());
        let mut value = serde_json::to_value(booking())?;
        value
            .as_object_mut()
            .ok_or("missing test object")?
            .insert("paid".into(), json!(true));
        check!(serde_json::from_value::<Booking>(value).is_err());
        Ok(())
    }
}
