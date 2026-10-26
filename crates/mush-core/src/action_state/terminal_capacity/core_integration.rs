//! Fixed paid Industry completion accounting. Real supplier, E4 and physical
//! authority remains in the actual owner adapters; this joins exact typed rows.
use super::*;
use crate::action::{IdempotencyKeyText, PrincipalId};
use crate::{
    BoundedScalar, ClockStamp, ObjId, ReservationBook, ReservationBookId, ResourceRef, ScheduleId,
    SubjectKey,
};
mod delivery;
mod protocol;
mod transition;
pub(in crate::action_state) use protocol::Protocol;
use protocol::{action_reference, descriptor};
pub(in crate::action_state) use transition::core_integration_change;

const ORDER: &str = "industry_core_orders";
const OUTPUT: &str = "core_output_reservations";
const DEPLOYMENT: &str = "core_output_deployment";
const INPUT: &str = "inventory_core_inputs";
const LOT: &str = "inventory_lots";
const CENSUS: &str = "inventory_lot_census";
const SLOTS: &str = "industry_facility_slots";
const BOOK: &str = "industry_facility_book";
const SCHEDULE: &str = "industry_core_schedule";
const JOB: &str = "industry_core_schedule_order";
const ORDER_CEILING: usize = 98_304 + 1024;
// Complete envelopes, not body-only bounds. The K book body itself is bounded
// at 192 KiB, and its fixed owner/schema/64-hex key envelope is below 1024.
const FAMILIES: [(&str, &str, usize); 10] = [
    ("industry", ORDER, ORDER_CEILING),
    ("industry", OUTPUT, 9216),
    ("industry", DEPLOYMENT, 9216),
    ("inventory", INPUT, 16384),
    ("inventory", LOT, 16384),
    ("inventory", CENSUS, 1024),
    ("industry", SLOTS, 1024),
    ("industry", BOOK, crate::MAX_RESERVATION_BOOK_BYTES + 1024),
    ("industry", SCHEDULE, 2048),
    ("industry", JOB, 2048),
];
fn bad(_: impl std::fmt::Display) -> ActionStateError {
    refused("core integration encoding or exact identity")
}
fn owner_id(s: &str) -> Result<OwnerId, ActionStateError> {
    OwnerId::new(s).map_err(bad)
}
fn field<'a>(v: &'a Value, n: &str) -> Result<&'a Value, ActionStateError> {
    v.get(n)
        .ok_or_else(|| refused("core integration required field"))
}
fn parsed<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(bad)
}
fn number(v: &Value, n: &str) -> Result<u64, ActionStateError> {
    field(v, n)?
        .as_u64()
        .ok_or_else(|| refused("core integration integer"))
}
fn present<'a>(v: &'a Value, n: &str) -> Result<Option<&'a Value>, ActionStateError> {
    match v.get(n) {
        Some(Value::Null) => Err(refused("core integration explicit null")),
        other => Ok(other),
    }
}
fn strict(v: &Value, required: &[&str], optional: &[&str]) -> Result<(), ActionStateError> {
    let o = v
        .as_object()
        .ok_or_else(|| refused("core integration object"))?;
    if required.iter().any(|k| !o.contains_key(*k))
        || o.keys()
            .any(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str()))
    {
        return Err(refused("core integration closed row"));
    }
    Ok(())
}
fn digest(domain: &str, v: &Value) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value_with_limit(domain, v, ORDER_CEILING).map_err(bad)
}
fn key(family: &str, encoded: String) -> Result<Key, ActionStateError> {
    let (owner, _, _) = FAMILIES
        .iter()
        .chain(delivery::FAMILIES.iter())
        .find(|(_, f, _)| *f == family)
        .ok_or_else(|| refused("core integration unknown target"))?;
    Ok(Key {
        owner: owner_id(owner)?,
        family: StateFamilyId::new(family).map_err(bad)?,
        encoded: encoded.into(),
    })
}
fn order_key(order: u64) -> Result<Key, ActionStateError> {
    key(ORDER, format!("{order:016x}"))
}
fn value(view: &View<'_>, raw: &Key) -> Result<Option<Value>, ActionStateError> {
    let k = raw.resolve(view.registry)?;
    let d = match view.mutations.and_then(|m| m.get(&k)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&k)
            .map(|r| serde_json::from_slice::<Value>(r.canonical.as_slice()).map_err(bad))
            .transpose()?,
    };
    d.map(|d| field(&d, "value").cloned()).transpose()
}
fn required(view: &View<'_>, k: &Key) -> Result<Value, ActionStateError> {
    value(view, k)?.ok_or_else(|| refused("core integration counterpart absent"))
}
struct Row {
    raw: Value,
    order: u64,
    accepted: CommitToken,
    action: DefinitionRef,
    intent: Value,
    policy: Value,
    fingerprint: DefinitionDigest,
}
impl Row {
    fn parse(raw: Value, order: u64) -> Result<Self, ActionStateError> {
        strict(
            &raw,
            &[
                "format", "sequence", "intent", "accepted", "action", "funding", "capacity",
                "progress",
            ],
            &["output", "terminal"],
        )?;
        let intent = field(&raw, "intent")?.clone();
        let policy = field(&intent, "policy")?.clone();
        let accepted = parsed(field(&raw, "accepted")?)?;
        let action: DefinitionRef = parsed(field(&raw, "action")?)?;
        let funding = field(&raw, "funding")?;
        if number(&raw, "format")? != 1
            || number(&intent, "format")? != 1
            || number(&intent, "order")? != order
            || action != action_reference(&policy, 0)?
            || field(funding, "commit")? != field(&raw, "accepted")?
            || field(funding, "action")? != field(&raw, "action")?
            || !(1..=8192).contains(&number(&raw, "sequence")?)
        {
            return Err(refused("core integration admission identity"));
        }
        present(&raw, "output")?;
        present(&raw, "terminal")?;
        canonical_json_bytes_with_limit(&intent, 24576).map_err(bad)?;
        canonical_json_bytes_with_limit(funding, 16384).map_err(bad)?;
        canonical_json_bytes_with_limit(field(&raw, "progress")?, 32768).map_err(bad)?;
        if let Some(terminal) = present(&raw, "terminal")? {
            canonical_json_bytes_with_limit(terminal, 16384).map_err(bad)?;
        }
        let fingerprint = digest(
            "core.industry_completion.v1",
            &json!({"format":1,"sequence":field(&raw,"sequence")?,"intent":intent,"accepted":accepted,"action":action,"funding":funding,"capacity":field(&raw,"capacity")?}),
        )?;
        Ok(Self {
            raw,
            order,
            accepted,
            action,
            intent,
            policy,
            fingerprint,
        })
    }
    fn terminal(&self) -> Result<Option<&Value>, ActionStateError> {
        present(&self.raw, "terminal")
    }
    fn output(&self) -> Result<Option<&Value>, ActionStateError> {
        present(&self.raw, "output")
    }
    fn requester(&self) -> Result<ObjId, ActionStateError> {
        parsed(field(&self.intent, "requester")?)
    }
    fn intent_digest(&self) -> Result<DefinitionDigest, ActionStateError> {
        digest("industry.core_integration_intent.v1", &self.intent)
    }
    fn resource(&self) -> Result<ResourceRef, ActionStateError> {
        let factory: ObjId = parsed(field(&self.policy, "factory")?)?;
        let owner = owner_id("industry")?;
        Ok(ResourceRef::new(
            owner.clone(),
            SubjectKey::Scoped {
                owner,
                class: CanonicalId::new("industry.facility").map_err(bad)?,
                id: IdempotencyKeyText::parse_canonical(format!("f{}", factory.0)).map_err(bad)?,
            },
            CanonicalId::new("slots").map_err(bad)?,
        ))
    }
    fn holder(&self) -> Result<crate::ScopedIdempotencyKey, ActionStateError> {
        let owner = owner_id("industry")?;
        let id = crate::IdempotencyKeyText::parse_canonical(format!("wo{:016x}", self.order))
            .map_err(bad)?;
        Ok(crate::ScopedIdempotencyKey::new(
            crate::IdempotencyScope {
                principal: crate::PrincipalId::Scoped {
                    owner: owner.clone(),
                    class: CanonicalId::new("workorder").map_err(bad)?,
                    id,
                },
                owner,
                namespace: CanonicalId::new("industry").map_err(bad)?,
            },
            0,
            crate::IdempotencyKey::Deterministic(
                crate::CanonicalHash::for_canonical_value(
                    "industry.wo_holder.v1",
                    1,
                    &json!({"order":self.order}),
                    1024,
                )
                .map_err(bad)?,
            ),
        ))
    }
    fn keys(&self) -> Result<BTreeMap<&'static str, Key>, ActionStateError> {
        let lot = field(&self.intent, "lot")?;
        let capacity = ReservationBookId::derive(&self.resource()?)
            .map_err(bad)?
            .canonical_hash()
            .to_hex();
        let schedule = ScheduleId::derive(
            &owner_id("industry")?,
            &CanonicalId::new(format!("core.{:016x}", self.order)).map_err(bad)?,
        )
        .map_err(bad)?
        .canonical_hash()
        .to_hex();
        let mut keys = BTreeMap::from([
            (ORDER, order_key(self.order)?),
            (OUTPUT, key(OUTPUT, self.order.to_string())?),
            (DEPLOYMENT, key(DEPLOYMENT, "registry".into())?),
            (INPUT, key(INPUT, format!("{:016x}", self.order))?),
            (LOT, key(LOT, format!("{:016x}", number(lot, "id")?))?),
            (
                CENSUS,
                key(CENSUS, format!("v{}", number(lot, "endpoint")?))?,
            ),
            (SLOTS, key(SLOTS, capacity.clone())?),
            (BOOK, key(BOOK, capacity)?),
            (SCHEDULE, key(SCHEDULE, schedule.clone())?),
            (JOB, key(JOB, schedule)?),
        ]);
        if delivery::promised(&self.policy)? {
            keys.insert(delivery::CORE, key(delivery::CORE, self.order.to_string())?);
            keys.insert(
                delivery::INDUSTRY,
                key(delivery::INDUSTRY, format!("{:016x}", self.order))?,
            );
        }
        Ok(keys)
    }
}
fn row(view: &View<'_>, order: u64) -> Result<Row, ActionStateError> {
    Row::parse(required(view, &order_key(order)?)?, order)
}
fn targets(r: &Row) -> Result<Vec<Target>, ActionStateError> {
    let mut out = Vec::new();
    let keys = r.keys()?;
    let promise = delivery::promised(&r.policy)?;
    let completed = promise && delivery::completed(r)?;
    for (_, family, ceiling) in FAMILIES
        .into_iter()
        .chain(promise.then_some(delivery::FAMILIES).into_iter().flatten())
    {
        if completed && ![ORDER, delivery::CORE, delivery::INDUSTRY].contains(&family) {
            continue;
        }
        let descriptor = descriptor(family)?;
        out.push(Target {
            key: keys
                .get(family)
                .ok_or_else(|| refused("core integration target key"))?
                .clone(),
            schema: descriptor.value_schema().clone(),
            schema_version: descriptor.schema_version(),
            ceiling,
            mode: if [delivery::CORE, delivery::INDUSTRY].contains(&family) {
                SlotMode::Create
            } else if family == OUTPUT {
                SlotMode::IndustryOutput
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
fn ready(view: &View<'_>) -> Result<(), ActionStateError> {
    for o in ["industry", "economy", "inventory"] {
        view.registry
            .owner_readiness
            .require_ready(&owner_id(o)?)
            .map_err(ActionStateError::OwnerNotReady)?;
    }
    Ok(())
}
fn paired(view: &View<'_>, r: &Row, historical: bool) -> Result<(), ActionStateError> {
    let keys = r.keys()?;
    let get = |f| {
        required(
            view,
            keys.get(f).ok_or_else(|| refused("core integration key"))?,
        )
    };
    let input = get(INPUT)?;
    if field(&input, "identity")? != &json!({"order":r.order,"intent":r.intent_digest()?})
        || field(&input, "lot")? != field(&r.intent, "lot")?
        || field(&input, "accepted")? != &json!(r.accepted)
        || field(field(&input, "binding")?, "accept")? != &json!(r.action)
    {
        return Err(refused("core integration input admission join"));
    }
    match r.terminal()? {
        None => {
            if present(&input, "terminal")?.is_some()
                || get(LOT)? != *field(&r.intent, "lot")?
                || get(SCHEDULE)? != *field(field(&r.raw, "progress")?, "cursor")?
                || get(JOB)? != json!(r.order)
            {
                return Err(refused("core integration pending input/schedule"));
            }
            let scalar: BoundedScalar = parsed(&get(SLOTS)?)?;
            let book: ReservationBook = parsed(&get(BOOK)?)?;
            book.checked_available(scalar).map_err(bad)?;
            let id: crate::ReservationId = parsed(field(&r.raw, "capacity")?)?;
            let held = book
                .records()
                .iter()
                .find(|x| x.id() == id)
                .ok_or_else(|| refused("core integration capacity hold absent"))?;
            if book.resource() != &r.resource()?
                || held.holder() != &r.holder()?
                || held.state() != crate::ReservationState::Held
                || held.amount() != 1
                || held.expires() != &parsed::<ClockStamp>(field(&r.intent, "expires_at")?)?
            {
                return Err(refused("core integration held capacity differs"));
            }
        }
        Some(t) => {
            let closed = present(&input, "terminal")?
                .ok_or_else(|| refused("core integration input terminal absent"))?;
            for name in ["commit", "action", "at"] {
                if field(closed, name)? != field(t, name)? {
                    return Err(refused("core integration terminal identity differs"));
                }
            }
            let result = field(t, "result")?;
            let outcome = field(result, "outcome")?
                .as_str()
                .ok_or_else(|| refused("core integration outcome"))?;
            let index = match outcome {
                "completed" => 3,
                "cancelled" => 4,
                _ => return Err(refused("core integration outcome")),
            };
            if field(t, "action")? != &json!(action_reference(&r.policy, index)?)
                || field(closed, "kind")? != &json!(if index == 3 { "consume" } else { "release" })
                || field(t, "commit")? == &json!(r.accepted)
            {
                return Err(refused("core integration terminal action"));
            }
            if !historical
                && (value(
                    view,
                    keys.get(SCHEDULE).ok_or_else(|| refused("schedule key"))?,
                )?
                .is_some()
                    || value(view, keys.get(JOB).ok_or_else(|| refused("job key"))?)?.is_some())
            {
                return Err(refused("core integration terminal schedule retained"));
            }
        }
    }
    delivery::receipts(view, r)?;
    let output = value(view, keys.get(OUTPUT).ok_or_else(|| refused("output key"))?)?;
    match (r.output()?, output) {
        (None, None) => {}
        (Some(pointer), Some(output)) => {
            let birth = crate::core_output::CoreOutputOrder::new(
                r.order,
                r.intent_digest()?,
                r.requester()?,
                parsed(field(&r.policy, "recipe")?)?,
                parsed(field(&r.policy, "factory")?)?,
                parsed(field(&r.policy, "output_container")?)?,
            )
            .map_err(bad)?;
            if field(&output, "order")? != &json!(birth) {
                return Err(refused("core integration output order differs"));
            }
            for name in ["object", "reservation", "action", "object_digest"] {
                if field(pointer, name)? != field(&output, name)? {
                    return Err(refused("core integration output pointer differs"));
                }
            }
            if let Some(t) = r
                .terminal()?
                .filter(|t| t.pointer("/result/outcome") == Some(&json!("completed")))
            {
                if field(&output, "state")?
                    != &json!({"kind":"consumed","completion":field(t,"commit")?,"action":field(t,"action")?})
                {
                    return Err(refused("core integration output consumption differs"));
                }
                let paid = field(field(t, "result")?, "output")?;
                for name in ["object", "reservation", "object_digest"] {
                    if field(paid, name)? != field(pointer, name)? {
                        return Err(refused("core integration actual title output differs"));
                    }
                }
                if field(paid, "reservation_action")? != field(pointer, "action")? {
                    return Err(refused("core integration reserved action differs"));
                }
            } else if field(&output, "state")? != &json!({"kind":"reserved"}) {
                return Err(refused("core integration premature output consumption"));
            }
        }
        _ => return Err(refused("core integration missing/crossed output")),
    }
    Ok(())
}
fn envelope(family: &str, key: &str, value: Value) -> Result<Value, ActionStateError> {
    let d = descriptor(family)?;
    Ok(
        json!({"family":{"owner":"industry","id":family},"key":key,"schema":{"id":d.value_schema(),"version":d.schema_version()},"value":value}),
    )
}
fn require_future_room(view: &View<'_>, r: &Row) -> Result<(), ActionStateError> {
    // Pure sizing values, never decoded as receipts or returned as authority.
    // The exact Industry progress codec caps its complete nested value at 32768;
    // the native E4 history codec caps each possible terminal at 16384. Replace
    // them with strings of exactly those canonical sizes for an upper bound.
    let mut projected = r.raw.clone();
    let object = projected
        .as_object_mut()
        .ok_or_else(|| refused("core integration sizing object"))?;
    let mut reserve = serde_json::to_value(action_reference(&r.policy, 1)?).map_err(bad)?;
    *reserve
        .get_mut("digest")
        .ok_or_else(|| refused("core integration action digest"))? =
        json!(DefinitionDigest::from_bytes([255; 32]));
    let pointer = json!({"object":u32::MAX,"reservation":CommitToken::from_bytes([255;16]),"action":reserve,"object_digest":DefinitionDigest::from_bytes([255;32])});
    object.insert("output".into(), pointer.clone());
    object.insert("progress".into(), Value::String("x".repeat(32766)));
    object.insert("terminal".into(), Value::String("x".repeat(16382)));
    canonical_json_bytes_with_limit(
        &envelope(ORDER, &format!("{:016x}", r.order), projected)?,
        ORDER_CEILING,
    )
    .map_err(|_| refused("core integration complete future order ceiling"))?;
    let order = crate::core_output::CoreOutputOrder::new(
        r.order,
        r.intent_digest()?,
        r.requester()?,
        parsed(field(&r.policy, "recipe")?)?,
        parsed(field(&r.policy, "factory")?)?,
        parsed(field(&r.policy, "output_container")?)?,
    )
    .map_err(bad)?;
    let mut complete = serde_json::to_value(action_reference(&r.policy, 3)?).map_err(bad)?;
    *complete
        .get_mut("digest")
        .ok_or_else(|| refused("core integration complete digest"))? =
        json!(DefinitionDigest::from_bytes([255; 32]));
    let output = json!({"format":1,"birth":u64::MAX,"purpose":"industry.core_integration","order":order,"object":u32::MAX,"created_at":i64::MIN,"object_digest":DefinitionDigest::from_bytes([255;32]),"reservation":CommitToken::from_bytes([255;16]),"action":field(&pointer,"action")?,"state":{"kind":"consumed","completion":CommitToken::from_bytes([255;16]),"action":complete}});
    delivery::require_future_room(r, &output)?;
    canonical_json_bytes_with_limit(&envelope(OUTPUT, &r.order.to_string(), output)?, 9216)
        .map_err(|_| refused("core integration complete future output ceiling"))?;
    let mut deployment = required(view, &key(DEPLOYMENT, "registry".into())?)?;
    let object = deployment
        .as_object_mut()
        .ok_or_else(|| refused("core integration deployment shape"))?;
    object.insert("born".into(), json!(u64::MAX));
    object.insert("consumed".into(), json!(u64::MAX));
    canonical_json_bytes_with_limit(&envelope(DEPLOYMENT, "registry", deployment)?, 9216)
        .map_err(|_| refused("core integration future deployment ceiling"))?;
    Ok(())
}

pub(super) fn verify(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    ready(view)?;
    let protocol = view
        .registry
        .core_integration
        .as_ref()
        .ok_or_else(|| refused("core integration protocol absent"))?;
    let order = u64::from_str_radix(&reservation.sponsor.encoded, 16).map_err(bad)?;
    let r = row(view, order)?;
    if delivery::promised(&r.policy)? && protocol.delivery.is_none() {
        return Err(refused("core delivery history requires its exact profile"));
    }
    if reservation.funding != r.accepted
        || reservation.effect != r.action
        || reservation.sponsor != order_key(order)?
        || reservation.targets != targets(&r)?
        || reservation.protocol_fingerprint != Some(r.fingerprint)
    {
        return Err(refused("core integration reservation join"));
    }
    match phase {
        Phase::Funding => {
            if r.terminal()?.is_some()
                || r.output()?.is_some()
                || r.action != *protocol.admission_action()
            {
                return Err(refused("core integration funding shape"));
            }
        }
        Phase::Retained => {
            if r.terminal()?.is_some()
                && !(delivery::promised(&r.policy)?
                    && delivery::completed(&r)?
                    && !delivery::receipts(view, &r)?)
            {
                return Err(refused("core integration retained terminal"));
            }
        }
        Phase::Terminal => {
            if r.terminal()?.is_none()
                || (delivery::promised(&r.policy)?
                    && delivery::completed(&r)?
                    && !delivery::receipts(view, &r)?)
            {
                return Err(refused(
                    "core integration missing terminal or promised delivery",
                ));
            }
        }
    }
    if matches!(phase, Phase::Funding) {
        require_future_room(view, &r)?;
    }
    paired(view, &r, false)?;
    ready(view)
}
pub(super) fn prospective_output(record: &Reservation, key: &Key) -> bool {
    record.effect.key().owner.as_str() == "industry"
        && record.effect.key().id.as_str() == "accept_core_integration"
        && record.effect.key().kind.as_str() == "action"
        && record.effect.version() == 1
        && key.owner.as_str() == "industry"
        && key.family.as_str() == OUTPUT
        && record
            .targets
            .iter()
            .filter(|t| t.mode == SlotMode::IndustryOutput)
            .count()
            == 1
        && record
            .targets
            .iter()
            .any(|t| &t.key == key && t.mode == SlotMode::IndustryOutput)
}
pub(super) use delivery::require_handoff as require_delivery_handoff;
pub(super) use transition::{audit_boot, validate_changes};

#[cfg(test)]
mod tests;
