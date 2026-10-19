//! Fixed paid refit completion capacity. Owner-produced payment, physical and
//! Tactics images retain their independent authority; this layer joins exact
//! typed targets and preserves admitted terminal room.
use super::*;
use crate::action::{IdempotencyKeyText, PrincipalId};
use crate::{
    BoundedScalar, ClockStamp, ObjId, ReservationBook, ReservationBookId, ResourceRef, ScheduleId,
    SubjectKey,
};
mod history;
mod protocol;
mod transition;
mod wire;
pub(in crate::action_state) use protocol::Protocol;
use protocol::{action_reference, descriptor};
pub(in crate::action_state) use transition::refit_change;
pub(super) use transition::{audit_boot, validate_changes};
use wire::Row;
const ORDER: &str = "industry_refit_orders";
const ASSET: &str = "industry_refit_assets";
const HULL: &str = "industry_refit_hulls";
const EXCHANGE: &str = "core_component_exchanges";
const SLOTS: &str = "industry_facility_slots";
const BOOK: &str = "industry_facility_book";
const SCHEDULE: &str = "industry_refit_schedule";
const JOB: &str = "industry_refit_schedule_order";
const ORDER_BYTES: usize = 196_608;
const ORDER_CEILING: usize = ORDER_BYTES + 1024;
const FAMILIES: [(&str, usize); 8] = [
    (ORDER, ORDER_CEILING),
    (ASSET, 4096),
    (HULL, 4096),
    (EXCHANGE, 16384),
    (SLOTS, 1024),
    (BOOK, crate::MAX_RESERVATION_BOOK_BYTES + 1024),
    (SCHEDULE, 2048),
    (JOB, 2048),
];
fn bad(_: impl std::fmt::Display) -> ActionStateError {
    refused("core refit exact encoding")
}
fn owner_id() -> Result<OwnerId, ActionStateError> {
    OwnerId::new("industry").map_err(bad)
}
fn field<'a>(v: &'a Value, n: &str) -> Result<&'a Value, ActionStateError> {
    v.get(n).ok_or_else(|| refused("core refit required field"))
}
fn parsed<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(bad)
}
fn number(v: &Value, n: &str) -> Result<u64, ActionStateError> {
    field(v, n)?
        .as_u64()
        .ok_or_else(|| refused("core refit integer"))
}
fn strict(v: &Value, names: &[&str]) -> Result<(), ActionStateError> {
    let o = v.as_object().ok_or_else(|| refused("core refit object"))?;
    if o.len() != names.len() || names.iter().any(|n| !o.contains_key(*n)) {
        return Err(refused("core refit closed shape"));
    }
    Ok(())
}
fn optional<'a>(v: &'a Value, n: &str) -> Result<Option<&'a Value>, ActionStateError> {
    Ok(match field(v, n)? {
        Value::Null => None,
        v => Some(v),
    })
}
fn digest(domain: &str, v: &Value) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value_with_limit(domain, v, ORDER_CEILING).map_err(bad)
}
fn key(family: &str, encoded: String) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: owner_id()?,
        family: StateFamilyId::new(family).map_err(bad)?,
        encoded: encoded.into(),
    })
}
fn order_key(order: u64) -> Result<Key, ActionStateError> {
    key(ORDER, format!("{order:016x}"))
}
fn value(view: &View<'_>, raw: &Key) -> Result<Option<Value>, ActionStateError> {
    let k = raw.resolve(view.registry)?;
    let document = match view.mutations.and_then(|m| m.get(&k)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&k)
            .map(|r| serde_json::from_slice::<Value>(r.canonical.as_slice()).map_err(bad))
            .transpose()?,
    };
    document.map(|v| field(&v, "value").cloned()).transpose()
}
fn required(view: &View<'_>, k: &Key) -> Result<Value, ActionStateError> {
    value(view, k)?.ok_or_else(|| refused("core refit missing counterpart"))
}
fn row(view: &View<'_>, order: u64) -> Result<Row, ActionStateError> {
    Row::parse(required(view, &order_key(order)?)?, order)
}
fn ready(view: &View<'_>) -> Result<(), ActionStateError> {
    for o in ["industry", "economy", "tactics", "sim", "org"] {
        view.registry
            .owner_readiness
            .require_ready(&OwnerId::new(o).map_err(bad)?)
            .map_err(ActionStateError::OwnerNotReady)?;
    }
    Ok(())
}
fn targets(r: &Row) -> Result<Vec<Target>, ActionStateError> {
    let mut out = Vec::new();
    for (name, k) in r.keys()? {
        let ceiling = FAMILIES
            .iter()
            .find(|(f, _)| *f == name)
            .ok_or_else(|| refused("core refit target"))?
            .1;
        let d = descriptor(name)?;
        out.push(Target {
            key: k,
            schema: d.value_schema().clone(),
            schema_version: d.schema_version(),
            ceiling,
            mode: if name == EXCHANGE {
                SlotMode::Create
            } else {
                SlotMode::UpdateExisting
            },
        });
    }
    out.sort_by(|a, b| {
        (&a.key.owner, &a.key.family, &a.key.encoded).cmp(&(
            &b.key.owner,
            &b.key.family,
            &b.key.encoded,
        ))
    });
    Ok(out)
}
fn future_room(r: &Row) -> Result<(), ActionStateError> {
    let mut v = r.raw.clone();
    let o = v
        .as_object_mut()
        .ok_or_else(|| refused("core refit future row"))?;
    o.insert("progress".into(), Value::String("x".repeat(32766)));
    o.insert("terminal".into(), Value::String("x".repeat(16382)));
    o.insert("installation".into(), Value::String("x".repeat(69630)));
    canonical_json_bytes_with_limit(&v, ORDER_BYTES).map_err(bad)?;
    let d = descriptor(ORDER)?;
    let envelope = json!({"family":{"owner":"industry","id":ORDER},"key":format!("{:016x}",r.order),"schema":{"id":d.value_schema(),"version":d.schema_version()},"value":v});
    canonical_json_bytes_with_limit(&envelope, ORDER_CEILING).map_err(bad)?;
    // The fixed Core codec itself preflights every exchange before custody;
    // its complete record ceiling is reserved now and cannot grow later.
    Ok(())
}
pub(super) fn verify(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    ready(view)?;
    let p = view
        .registry
        .core_refit
        .as_ref()
        .ok_or_else(|| refused("core refit protocol absent"))?;
    let order = u64::from_str_radix(&reservation.sponsor.encoded, 16).map_err(bad)?;
    let r = row(view, order)?;
    if reservation.funding != r.accepted
        || reservation.effect != r.action
        || reservation.sponsor != order_key(order)?
        || reservation.targets != targets(&r)?
        || reservation.protocol_fingerprint != Some(r.fingerprint)
    {
        return Err(refused("core refit reservation identity"));
    }
    match phase {
        Phase::Funding => {
            if r.terminal()?.is_some() || r.action != *p.admission_action() {
                return Err(refused("core refit paid admission"));
            }
            future_room(&r)?;
        }
        Phase::Retained => {
            if r.terminal()?.is_some() {
                return Err(refused("core refit retained terminal"));
            }
        }
        Phase::Terminal => {
            if r.terminal()?.is_none() {
                return Err(refused("core refit missing terminal"));
            }
        }
    }
    history::paired(view, &r, false)?;
    ready(view)
}
