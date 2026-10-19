//! Exact future-close accounting. Physical event and entitlement authority stay
//! with the four real owner participants; serialized claims cannot mint them.
use super::*;
use crate::action::PrincipalId;
use crate::{ClockRef, ClockStamp, ObjId};
mod protocol;
use protocol::CapturedProtocol;
pub(in crate::action_state) use protocol::Protocol;

const FAMILY: &str = "roll_mind_ic_claim";
const CEILING: usize = 16384;
fn bad(_: impl std::fmt::Display) -> ActionStateError {
    refused("IC claim encoding or definition")
}
fn owner_id(owner: &str) -> Result<OwnerId, ActionStateError> {
    OwnerId::new(owner).map_err(bad)
}
fn family_descriptor() -> Result<StateFamilyDescriptor, ActionStateError> {
    let schema = |suffix: &str| CanonicalId::new(format!("{FAMILY}.{suffix}.v1")).map_err(bad);
    StateFamilyDescriptor::new(
        StateFamilyId::new(FAMILY).map_err(bad)?,
        schema("key")?,
        schema("value")?,
        1,
        schema("strict_decode")?,
        schema("invariants")?,
        schema("canonical")?,
    )
    .and_then(|d| d.with_limits(192, CEILING))
    .map_err(bad)
}
fn key(open: CommitToken) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: owner_id("roll")?,
        family: StateFamilyId::new(FAMILY).map_err(bad)?,
        encoded: open.to_hex().into(),
    })
}
fn targets(open: CommitToken) -> Result<Vec<Target>, ActionStateError> {
    Ok(vec![Target {
        key: key(open)?,
        schema: CanonicalId::new(format!("{FAMILY}.value.v1")).map_err(bad)?,
        schema_version: 1,
        ceiling: CEILING,
        mode: SlotMode::UpdateExisting,
    }])
}
/// The sole cross-owner metadata shape. This recognizes no arbitrary target,
/// schema, owner or limit. The full verifier must additionally prove the row.
pub(super) fn metadata_shape(record: &Reservation) -> bool {
    record.effect.key().owner.as_str() == "tacsim"
        && record.effect.key().kind.as_str() == "action"
        && record.effect.key().id.as_str() == "open_ic_mind"
        && record.effect.version() == 1
        && key(record.funding).is_ok_and(|key| key == record.sponsor)
        && targets(record.funding).is_ok_and(|targets| targets == record.targets)
        && record.protocol_fingerprint.is_some()
}
fn value(view: &View<'_>, open: CommitToken) -> Result<Option<Value>, ActionStateError> {
    let key = key(open)?.resolve(view.registry)?;
    let document = match view.mutations.and_then(|m| m.get(&key)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&key)
            .map(|row| serde_json::from_slice::<Value>(row.canonical.as_slice()).map_err(bad))
            .transpose()?,
    };
    document
        .map(|row| {
            row.get("value")
                .cloned()
                .ok_or_else(|| refused("IC claim value missing"))
        })
        .transpose()
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ActionStateError> {
    value
        .get(name)
        .ok_or_else(|| refused("IC claim field missing"))
}
fn strict(value: &Value, fields: &[&str]) -> Result<(), ActionStateError> {
    let object = value
        .as_object()
        .ok_or_else(|| refused("IC claim object"))?;
    if object.len() != fields.len() || fields.iter().any(|name| !object.contains_key(*name)) {
        return Err(refused("IC claim fields differ"));
    }
    Ok(())
}
fn parsed<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, ActionStateError> {
    serde_json::from_value(value.clone()).map_err(bad)
}
fn number(value: &Value, name: &str) -> Result<u64, ActionStateError> {
    field(value, name)?
        .as_u64()
        .filter(|n| *n != 0)
        .ok_or_else(|| refused("IC positive number"))
}
fn signed(value: &Value, name: &str) -> Result<i64, ActionStateError> {
    field(value, name)?
        .as_i64()
        .ok_or_else(|| refused("IC signed number"))
}
fn decimal(value: &Value) -> Result<u64, ActionStateError> {
    let text = value.as_str().ok_or_else(|| refused("IC decimal string"))?;
    let n = text.parse::<u64>().map_err(bad)?;
    if n == 0 || n.to_string() != text {
        return Err(refused("IC canonical positive decimal"));
    }
    Ok(n)
}
fn stamp(value: &Value) -> Result<ClockStamp, ActionStateError> {
    let stamp: ClockStamp = parsed(value)?;
    if !matches!(stamp.clock,ClockRef::WorldIc{generation} if generation!=0) {
        return Err(refused("IC world clock"));
    }
    Ok(stamp)
}
fn digest(domain: &str, value: &Value) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value(domain, value).map_err(bad)
}
fn open_digest(open: &Value) -> Result<DefinitionDigest, ActionStateError> {
    digest("roll.ic_mind_claim.open.v1", open)
}
struct Claim {
    format: u64,
    open: Value,
    closed: Option<Value>,
    action: DefinitionRef,
    fingerprint: DefinitionDigest,
}
impl Claim {
    fn parse(raw: Value, operation: CommitToken) -> Result<Self, ActionStateError> {
        strict(&raw, &["format", "open", "closed"])?;
        let format = number(&raw, "format")?;
        if !matches!(format, 1 | 2 | 3) {
            return Err(refused("IC claim format"));
        }
        let open = field(&raw, "open")?.clone();
        let mut fields = vec![
            "operation",
            "action",
            "protocol",
            "unit",
            "sortie",
            "dispatcher",
            "hull_title_revision",
            "licence",
            "term",
            "charter",
            "line",
            "filed",
            "revision",
            "rating",
            "window",
            "capacity",
            "at_ic",
            "seal_digest",
            "custody_digest",
            "exclusion_digest",
        ];
        // Each new capture has its own exact wire form. Old format-one bytes
        // remain unchanged; a paid holder chain never becomes charter title.
        if format == 2 || format == 3 {
            let name = if format == 2 {
                "charter_ownership"
            } else {
                "paid_ownership"
            };
            fields.push(name);
            let ownership = field(&open, name)?;
            strict(ownership, &["generation", "operation", "digest"])?;
            if !(2..=65_537).contains(&number(ownership, "generation")?) {
                return Err(refused("IC captured ownership generation"));
            }
            parsed::<CommitToken>(field(ownership, "operation")?)?;
            parsed::<DefinitionDigest>(field(ownership, "digest")?)?;
        }
        strict(&open, &fields)?;
        let protocol: CapturedProtocol = parsed(field(&open, "protocol")?)?;
        let action: DefinitionRef = parsed(field(&open, "action")?)?;
        let unit: ObjId = parsed(field(&open, "unit")?)?;
        let dispatcher: ObjId = parsed(field(&open, "dispatcher")?)?;
        if parsed::<CommitToken>(field(&open, "operation")?)? != operation
            || action != protocol.action(0)?
            || unit.0 == 0
            || dispatcher.0 == 0
            || unit == dispatcher
        {
            return Err(refused("IC immutable open identity"));
        }
        number(&open, "sortie")?;
        number(&open, "hull_title_revision")?;
        for name in ["licence", "term", "charter", "line", "filed"] {
            decimal(field(&open, name)?)?;
        }
        for name in ["seal_digest", "custody_digest", "exclusion_digest"] {
            parsed::<DefinitionDigest>(field(&open, name)?)?;
        }
        let at = stamp(field(&open, "at_ic")?)?;
        let window = field(&open, "window")?;
        strict(
            window,
            &[
                "format",
                "eligibility_start",
                "issued_at",
                "term_end",
                "grace_end",
            ],
        )?;
        let start = signed(window, "eligibility_start")?;
        let issued = signed(window, "issued_at")?;
        let end = signed(window, "term_end")?;
        let grace = signed(window, "grace_end")?;
        if field(window, "format")? != &json!(1)
            || end <= start
            || grace < end
            || issued >= grace
            || at.at < start
            || at.at < issued
            || at.at >= grace
        {
            return Err(refused("IC captured admission window"));
        }
        if !end
            .checked_sub(start)
            .is_some_and(|n| (1..=90 * 24 * 60 * 60).contains(&n))
            || !grace
                .checked_sub(end)
                .is_some_and(|n| (0..=3 * 24 * 60 * 60).contains(&n))
        {
            return Err(refused("IC captured term bounds"));
        }
        let capacity = field(&open, "capacity")?;
        match field(capacity, "kind")?.as_str() {
            Some("owned_dispatch") => {
                if format == 3 {
                    return Err(refused("IC owned claim cannot carry paid ownership"));
                }
                strict(capacity, &["kind"])?;
            }
            Some("paid_seats") => {
                if format == 2 {
                    return Err(refused("IC paid claim cannot carry charter ownership"));
                }
                strict(capacity, &["kind", "seats"])?;
                if number(capacity, "seats")? > 1024 {
                    return Err(refused("IC paid seats"));
                }
            }
            _ => return Err(refused("IC capacity kind")),
        }
        // The actual registered Roll codec validates source/rating business
        // identities. Core independently closes their shape and commits all bytes.
        let revision = field(&open, "revision")?;
        strict(revision, &["format", "revision", "source"])?;
        if field(revision, "format")? != &json!(1) {
            return Err(refused("IC revision format"));
        }
        parsed::<DefinitionDigest>(field(revision, "source")?)?;
        let reference = field(revision, "revision")?;
        strict(reference, &["key", "digest"])?;
        parsed::<DefinitionDigest>(field(reference, "digest")?)?;
        let revision_key = field(reference, "key")?;
        strict(revision_key, &["character", "mind", "number"])?;
        for name in ["character", "mind", "number"] {
            number(revision_key, name)?;
        }
        let rating = field(&open, "rating")?;
        strict(rating, &["job", "id", "schema", "receipt_digest"])?;
        parsed::<CommitToken>(field(rating, "job")?)?;
        parsed::<DefinitionDigest>(field(rating, "id")?)?;
        parsed::<DefinitionDigest>(field(rating, "receipt_digest")?)?;
        number(rating, "schema")?;
        let closed = match field(&raw, "closed")? {
            Value::Null => None,
            row => Some(row.clone()),
        };
        if let Some(closed) = &closed {
            validate_close(closed, &open, operation)?;
        }
        let fingerprint = open_digest(&open)?;
        Ok(Self {
            format,
            open,
            closed,
            action,
            fingerprint,
        })
    }
}
fn validate_close(
    closed: &Value,
    open: &Value,
    operation: CommitToken,
) -> Result<(), ActionStateError> {
    strict(
        closed,
        &[
            "operation",
            "action",
            "protocol",
            "open",
            "open_digest",
            "unit",
            "sortie",
            "transition",
            "event_sequence",
            "event_digest",
            "kind",
            "at_ic",
        ],
    )?;
    let protocol: CapturedProtocol = parsed(field(closed, "protocol")?)?;
    let at = stamp(field(closed, "at_ic")?)?;
    let started = stamp(field(open, "at_ic")?)?;
    if parsed::<CommitToken>(field(closed, "operation")?)? == operation
        || parsed::<CommitToken>(field(closed, "open")?)? != operation
        || parsed::<DefinitionDigest>(field(closed, "open_digest")?)? != open_digest(open)?
        || field(closed, "unit")? != field(open, "unit")?
        || field(closed, "sortie")? != field(open, "sortie")?
        || number(closed, "transition")? <= number(open, "sortie")?
        || (at.clock == started.clock && at.at < started.at)
        || parsed::<DefinitionRef>(field(closed, "action")?)? != protocol.action(1)?
    {
        return Err(refused("IC exact close identity"));
    }
    number(closed, "event_sequence")?;
    parsed::<DefinitionDigest>(field(closed, "event_digest")?)?;
    let kind = field(closed, "kind")?;
    match field(kind, "kind")?.as_str() {
        Some("destroyed") => strict(kind, &["kind"])?,
        Some("returned_to_base" | "powered_down_in_base" | "recovered_to_base") => {
            strict(kind, &["kind", "base"])?;
            let base: ObjId = parsed(field(kind, "base")?)?;
            if base.0 == 0 || field(kind, "base")? == field(open, "unit")? {
                return Err(refused("IC close base"));
            }
        }
        _ => return Err(refused("IC close event kind")),
    }
    Ok(())
}
fn claim(view: &View<'_>, operation: CommitToken) -> Result<Claim, ActionStateError> {
    Claim::parse(
        value(view, operation)?.ok_or_else(|| refused("IC claim missing"))?,
        operation,
    )
}
fn envelope(operation: CommitToken, row: &Value) -> Value {
    json!({"family":{"owner":"roll","id":FAMILY},"key":operation.to_hex(),"schema":{"id":format!("{FAMILY}.value.v1"),"version":1},"value":row})
}
fn require_close_room(
    row: &Claim,
    operation: CommitToken,
    protocol: &Protocol,
) -> Result<(), ActionStateError> {
    // Accounting only: maximum-width fields and the longest fixed event kind.
    // Never persists or confers authority on this projected physical event.
    let mut projected = json!({"format":row.format,"open":row.open,"closed":{
        "operation":CommitToken::from_bytes([255;16]),"action":protocol.actions[1],"protocol":CapturedProtocol::current(protocol),
        "open":operation,"open_digest":row.fingerprint,"unit":field(&row.open,"unit")?,"sortie":field(&row.open,"sortie")?,
        "transition":u64::MAX,"event_sequence":u64::MAX,"event_digest":DefinitionDigest::from_bytes([255;32]),
        "kind":{"kind":"powered_down_in_base","base":u32::MAX},"at_ic":{"clock":{"kind":"world_ic","generation":u64::MAX},"at":i64::MIN}}});
    // A later valid manifest/action can have wider decimal digest bytes than
    // today's reference. Account for that complete fixed-locator future shape.
    let maximum_digest = json!(DefinitionDigest::from_bytes([255; 32]));
    for pointer in ["/closed/action/digest", "/closed/protocol/manifest/digest"] {
        *projected
            .pointer_mut(pointer)
            .ok_or_else(|| refused("IC projected reference shape"))? = maximum_digest.clone();
    }
    canonical_json_bytes_with_limit(&envelope(operation, &projected), CEILING)
        .map_err(|_| refused("IC complete future Close exceeds ceiling"))?;
    Ok(())
}
fn ready(view: &View<'_>) -> Result<(), ActionStateError> {
    for owner in protocol::OWNERS {
        view.registry
            .owner_readiness
            .require_ready(&owner_id(owner)?)
            .map_err(ActionStateError::OwnerNotReady)?;
    }
    Ok(())
}
pub(super) fn verify(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    let protocol = view
        .registry
        .ic_mind_claim
        .as_ref()
        .ok_or_else(|| refused("IC protocol unavailable"))?;
    ready(view)?;
    let row = claim(view, reservation.funding)?;
    if reservation.effect != row.action
        || reservation.sponsor != key(reservation.funding)?
        || reservation.targets != targets(reservation.funding)?
        || reservation.protocol_fingerprint != Some(row.fingerprint)
        || match phase {
            Phase::Funding => row.closed.is_some() || row.action != *protocol.admission_action(),
            Phase::Retained => row.closed.is_some(),
            Phase::Terminal => row.closed.is_none(),
        }
    {
        return Err(refused("IC reservation or phase differs"));
    }
    if matches!(phase, Phase::Funding) {
        require_close_room(&row, reservation.funding, protocol)?;
    }
    ready(view)
}
pub(in crate::action_state) fn ic_claim_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    open: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let protocol = txn
        .store
        .registry
        .ic_mind_claim
        .as_ref()
        .ok_or_else(|| refused("IC protocol unavailable"))?;
    let identity = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("IC actual invocation unavailable"))?;
    if identity.commit != commit
        || txn.provider_ordinal != Some(identity.ordinal)
        || txn.owner != identity.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.progress.is_some()
    {
        return Err(refused("IC actual preparation identity"));
    }
    let index = protocol.action_index(identity)?;
    if let Some(open) = open {
        if index != 1 || open == commit {
            return Err(refused("IC Close identity"));
        }
        txn.capacity_changes.terminal = Some(ReservationId(owner_id("tacsim")?, open.to_hex()));
    } else {
        if index != 0 {
            return Err(refused("IC Open action"));
        }
        let view = View {
            registry: &txn.store.registry,
            records: &txn.store.records,
            mutations: Some(&txn.mutations),
        };
        let row = claim(&view, commit)?;
        let reservation = Reservation {
            format: 1,
            funding: commit,
            effect: protocol.admission_action().clone(),
            sponsor: key(commit)?,
            targets: targets(commit)?,
            protocol_fingerprint: Some(row.fingerprint),
        };
        verify(Phase::Funding, &reservation, &view)?;
        txn.capacity_changes.funding = Some(reservation);
    }
    Ok(())
}
fn protected(key: &StateKey) -> bool {
    key.owner.as_str() == "roll" && key.family.as_str() == FAMILY
}
pub(super) fn validate_changes(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
    if !mutations.keys().any(protected) {
        if store
            .registry
            .ic_mind_claim
            .as_ref()
            .is_some_and(|protocol| {
                identity.is_some_and(|identity| protocol.action_index(identity).is_ok())
            })
        {
            return Err(refused("IC action lacks its exact typed claim"));
        }
        return Ok(());
    }
    let protocol = store
        .registry
        .ic_mind_claim
        .as_ref()
        .ok_or_else(|| refused("IC write without configured protocol"))?;
    let identity = identity.ok_or_else(|| refused("IC write without actual invocation"))?;
    let index = protocol.action_index(identity)?;
    if identity.principal != (PrincipalId::Object { id: identity.actor }) {
        return Err(refused("IC actual principal differs"));
    }
    let operation = if index == 0 {
        identity.commit
    } else {
        let id = changes
            .terminal
            .as_ref()
            .ok_or_else(|| refused("IC Close lacks reservation"))?;
        if id.0.as_str() != "tacsim" {
            return Err(refused("IC metadata owner differs"));
        }
        CommitToken::from_hex(&id.1).map_err(bad)?
    };
    let state_key = key(operation)?.resolve(&store.registry)?;
    if mutations.len() != 1 || !matches!(mutations.get(&state_key), Some(TxnMutation::Put { .. })) {
        return Err(refused("IC exact sole typed target"));
    }
    let before = View {
        registry: &store.registry,
        records: &store.records,
        mutations: None,
    };
    let after = View {
        registry: &store.registry,
        records: &store.records,
        mutations: Some(mutations),
    };
    let row = claim(&after, operation)?;
    if index == 0 {
        if value(&before, operation)?.is_some()
            || parsed::<ObjId>(field(&row.open, "dispatcher")?)? != identity.actor
        {
            return Err(refused("IC reused Open or dispatcher"));
        }
        let reservation = changes
            .funding
            .as_ref()
            .ok_or_else(|| refused("IC Open lacks reservation"))?;
        if reservation.funding != identity.commit {
            return Err(refused("IC Open commit differs"));
        }
        verify(Phase::Funding, reservation, &after)?;
    } else {
        let prior = claim(&before, operation)?;
        let id = ReservationId(owner_id("tacsim")?, operation.to_hex());
        let retained = store
            .capacity
            .records
            .get(&id)
            .ok_or_else(|| refused("IC acknowledged reservation missing"))?;
        verify(Phase::Retained, &retained.record, &before)?;
        let closed = row
            .closed
            .as_ref()
            .ok_or_else(|| refused("IC Close cause missing"))?;
        if row.open != prior.open
            || parsed::<CommitToken>(field(closed, "operation")?)? != identity.commit
            || field(closed, "action")? != &json!(identity.action)
            || field(closed, "protocol")? != &json!(CapturedProtocol::current(protocol))
        {
            return Err(refused("IC immutable Open or current Close differs"));
        }
        verify(Phase::Terminal, &retained.record, &after)?;
    }
    Ok(())
}
pub(super) fn audit_boot(
    view: &View<'_>,
    reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
) -> Result<(), ActionStateError> {
    let keys = view.records.keys().filter(|key| protected(key));
    if view.registry.ic_mind_claim.is_none() {
        if keys.count() != 0 {
            return Err(refused("retained IC claim without configured protocol"));
        }
        return Ok(());
    }
    for key in keys {
        let operation = CommitToken::from_hex(&key.encoded).map_err(bad)?;
        if key.encoded.as_str() != operation.to_hex() {
            return Err(refused("IC canonical claim key"));
        }
        let row = claim(view, operation)?;
        let reservation = reservations.get(&ReservationId(owner_id("tacsim")?, operation.to_hex()));
        match (row.closed.is_none(), reservation) {
            (true, Some(retained)) => verify(Phase::Retained, &retained.record, view)?,
            (false, None) => (),
            _ => return Err(refused("IC reverse reservation completeness")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
