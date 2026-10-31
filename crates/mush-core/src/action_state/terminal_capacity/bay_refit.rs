//! Fixed paid bay refit completion capacity (doc 31 cut UD.4b): the sibling of
//! [`core_refit`] for a component refit in a facility bay.
//! A hull is held in the bay from Accept to Install or Cancel; the part fitted
//! comes from an Inventory lot held for the order, and the part taken out goes
//! back to the same stores as a new lot. Payment, the physical part and the
//! Tactics image keep their independent authority; this layer joins the exact
//! typed rows and preserves the admitted terminal room.
use super::*;
use crate::action::{IdempotencyKeyText, PrincipalId};
use crate::{
    BoundedScalar, ClockStamp, ObjId, ReservationBook, ReservationBookId, ResourceRef, ScheduleId,
    SubjectKey,
};
mod history;
mod protocol;
#[cfg(test)]
mod tests;
mod transition;
mod wire;
pub(in crate::action_state) use protocol::Protocol;
pub use protocol::{ACTIONS, action_descriptor, descriptor as family_descriptor};
use protocol::{action_reference, descriptor};
pub(in crate::action_state) use transition::bay_refit_change;
pub(super) use transition::{audit_boot, validate_changes};
use wire::Row;
const ORDER: &str = "industry_bay_orders";
const HULL: &str = "industry_bay_hulls";
const SLOTS: &str = "industry_facility_slots";
const BOOK: &str = "industry_facility_book";
const SCHEDULE: &str = "industry_bay_schedule";
const JOB: &str = "industry_bay_schedule_order";
const PART: &str = "inventory_bay_parts";
const LOT: &str = "inventory_lots";
const CENSUS: &str = "inventory_lot_census";
/// Written at Accept, beside the fixed families: Industry's admission census
/// and Inventory's part heads and part census.
const DEPLOYMENT: &str = "industry_bay_deployment";
const PART_HEAD: &str = "inventory_bay_part_heads";
const PART_CENSUS: &str = "inventory_bay_part_census";
/// The core refit's hull heads, read at Accept so a hull pending a core
/// refit is not also taken into a bay.
const CORE_HULL: &str = "industry_refit_hulls";
const ORDER_BYTES: usize = 196_608;
const ORDER_CEILING: usize = ORDER_BYTES + 1024;
const CADENCE: i64 = 3600;
const MAX_WORK_UNITS: u64 = 167;
const RESERVATION_SECONDS: i64 = 604_800;
/// Every family the protocol names with its owner: the nine it reserves room
/// in, then the three Accept also writes. Industry's and Inventory's
/// registrations must use [`family_descriptor`] for each.
pub const FAMILY_NAMES: [(&str, &str); 12] = [
    ("industry", ORDER),
    ("industry", HULL),
    ("industry", SLOTS),
    ("industry", BOOK),
    ("industry", SCHEDULE),
    ("industry", JOB),
    ("inventory", PART),
    ("inventory", LOT),
    ("inventory", CENSUS),
    ("industry", DEPLOYMENT),
    ("inventory", PART_HEAD),
    ("inventory", PART_CENSUS),
];
const FAMILIES: [(&str, &str, usize); 9] = [
    ("industry", ORDER, ORDER_CEILING),
    ("industry", HULL, 4096),
    ("industry", SLOTS, 1024),
    ("industry", BOOK, crate::MAX_RESERVATION_BOOK_BYTES + 1024),
    ("industry", SCHEDULE, 2048),
    ("industry", JOB, 2048),
    ("inventory", PART, 4096),
    ("inventory", LOT, 16384),
    ("inventory", CENSUS, 1024),
];
fn bad(_: impl std::fmt::Display) -> ActionStateError {
    refused("bay refit exact encoding")
}
fn owner_id(o: &str) -> Result<OwnerId, ActionStateError> {
    OwnerId::new(o).map_err(bad)
}
fn family_owner(family: &str) -> &'static str {
    if family.starts_with("inventory_") {
        "inventory"
    } else {
        "industry"
    }
}
fn field<'a>(v: &'a Value, n: &str) -> Result<&'a Value, ActionStateError> {
    v.get(n).ok_or_else(|| refused("bay refit required field"))
}
fn parsed<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(bad)
}
fn number(v: &Value, n: &str) -> Result<u64, ActionStateError> {
    field(v, n)?
        .as_u64()
        .ok_or_else(|| refused("bay refit integer"))
}
fn strict(v: &Value, names: &[&str]) -> Result<(), ActionStateError> {
    let o = v.as_object().ok_or_else(|| refused("bay refit object"))?;
    if o.len() != names.len() || names.iter().any(|n| !o.contains_key(*n)) {
        return Err(refused("bay refit closed shape"));
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
        owner: owner_id(family_owner(family))?,
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
    value(view, k)?.ok_or_else(|| refused("bay refit missing counterpart"))
}
fn row(view: &View<'_>, order: u64) -> Result<Row, ActionStateError> {
    Row::parse(required(view, &order_key(order)?)?, order)
}
fn ready(view: &View<'_>) -> Result<(), ActionStateError> {
    for o in ["industry", "economy", "inventory", "tactics", "sim", "org"] {
        view.registry
            .owner_readiness
            .require_ready(&owner_id(o)?)
            .map_err(ActionStateError::OwnerNotReady)?;
    }
    Ok(())
}
fn targets(r: &Row) -> Result<Vec<Target>, ActionStateError> {
    let mut out = Vec::new();
    for (name, k, mode) in r.targets()? {
        let ceiling = FAMILIES
            .iter()
            .find(|(_, f, _)| *f == name)
            .ok_or_else(|| refused("bay refit target"))?
            .2;
        let d = descriptor(name)?;
        out.push(Target {
            key: k,
            schema: d.value_schema().clone(),
            schema_version: d.schema_version(),
            ceiling,
            mode,
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
/// The order row's terminal image fits now, at Accept: room for the longest
/// progress, terminal receipt and installation evidence it can grow to.
fn future_room(r: &Row) -> Result<(), ActionStateError> {
    let mut v = r.raw.clone();
    let o = v
        .as_object_mut()
        .ok_or_else(|| refused("bay refit future row"))?;
    o.insert("progress".into(), Value::String("x".repeat(32766)));
    o.insert("terminal".into(), Value::String("x".repeat(16382)));
    o.insert("installation".into(), Value::String("x".repeat(69630)));
    canonical_json_bytes_with_limit(&v, ORDER_BYTES).map_err(bad)?;
    let d = descriptor(ORDER)?;
    let envelope = json!({"family":{"owner":"industry","id":ORDER},"key":format!("{:016x}",r.order),"schema":{"id":d.value_schema(),"version":d.schema_version()},"value":v});
    canonical_json_bytes_with_limit(&envelope, ORDER_CEILING).map_err(bad)?;
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
        .bay_refit
        .as_ref()
        .ok_or_else(|| refused("bay refit protocol absent"))?;
    let order = u64::from_str_radix(&reservation.sponsor.encoded, 16).map_err(bad)?;
    let r = row(view, order)?;
    if reservation.funding != r.accepted
        || reservation.effect != r.action
        || reservation.sponsor != order_key(order)?
        || reservation.targets != targets(&r)?
        || reservation.protocol_fingerprint != Some(r.fingerprint)
    {
        return Err(refused("bay refit reservation identity"));
    }
    match phase {
        Phase::Funding => {
            if r.terminal()?.is_some() || r.action != *p.admission_action() {
                return Err(refused("bay refit paid admission"));
            }
            future_room(&r)?;
        }
        Phase::Retained => {
            if r.terminal()?.is_some() {
                return Err(refused("bay refit retained terminal"));
            }
        }
        Phase::Terminal => {
            if r.terminal()?.is_none() {
                return Err(refused("bay refit missing terminal"));
            }
        }
    }
    history::paired(view, &r, false)?;
    ready(view)
}
