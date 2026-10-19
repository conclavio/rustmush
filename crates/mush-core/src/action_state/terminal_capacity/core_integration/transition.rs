use super::*;

pub(in crate::action_state) fn core_integration_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    order: u64,
    admission: bool,
) -> Result<(), ActionStateError> {
    let protocol = txn
        .store
        .registry
        .core_integration
        .as_ref()
        .ok_or_else(|| refused("core integration protocol unavailable"))?;
    let identity = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("core integration actual invocation absent"))?;
    if identity.commit != commit
        || txn.provider_ordinal != Some(identity.ordinal)
        || txn.owner != identity.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.progress.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.industry_delivery.is_some()
    {
        return Err(refused("core integration preparation differs"));
    }
    let index = protocol.action_index(identity)?;
    let view = View {
        registry: &txn.store.registry,
        records: &txn.store.records,
        mutations: Some(&txn.mutations),
    };
    let r = row(&view, order)?;
    if admission {
        if index != 0 || r.accepted != commit {
            return Err(refused("core integration acceptance action/commit"));
        }
        let reservation = Reservation {
            format: 1,
            funding: commit,
            effect: r.action.clone(),
            sponsor: order_key(order)?,
            targets: targets(&r)?,
            protocol_fingerprint: Some(r.fingerprint),
        };
        verify(Phase::Funding, &reservation, &view)?;
        txn.capacity_changes.funding = Some(reservation);
    } else {
        if index == 0 || r.accepted == commit {
            return Err(refused("core integration retained action/commit"));
        }
        let id = ReservationId(owner_id("industry")?, r.accepted.to_hex());
        if matches!(index, 1 | 2) {
            txn.capacity_changes.progress = Some(id)
        } else if index == 3 && delivery::promised(&r.policy)? {
            let mut replacement = txn
                .store
                .capacity
                .records
                .get(&id)
                .ok_or_else(|| refused("core delivery admitted reservation absent"))?
                .record
                .clone();
            replacement.targets = targets(&r)?;
            txn.capacity_changes.industry_delivery = Some(replacement);
        } else {
            txn.capacity_changes.terminal = Some(id)
        }
    }
    Ok(())
}
fn without(v: &Value, names: &[&str]) -> Result<Value, ActionStateError> {
    let mut o = v
        .as_object()
        .cloned()
        .ok_or_else(|| refused("core integration object"))?;
    for n in names {
        o.remove(*n);
    }
    Ok(Value::Object(o))
}
fn exact(
    mutations: &BTreeMap<StateKey, TxnMutation>,
    view: &View<'_>,
    expected: Vec<(Key, bool)>,
) -> Result<(), ActionStateError> {
    let keys = expected
        .into_iter()
        .map(|(k, delete)| Ok((k.resolve(view.registry)?, delete)))
        .collect::<Result<BTreeMap<_, _>, ActionStateError>>()?;
    if keys.len() != mutations.len()
        || mutations.iter().any(|(k, m)| {
            keys.get(k)
                .is_none_or(|delete| *delete != matches!(m, TxnMutation::Delete))
        })
    {
        return Err(refused("core integration exact native mutation set"));
    }
    Ok(())
}
fn scope_key(owner: &str, family: &str, encoded: String) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: owner_id(owner)?,
        family: StateFamilyId::new(family).map_err(bad)?,
        encoded: encoded.into(),
    })
}
fn required_key(keys: &BTreeMap<&str, Key>, name: &str) -> Result<Key, ActionStateError> {
    keys.get(name)
        .cloned()
        .ok_or_else(|| refused("core integration target absent"))
}
fn expected(
    keys: &BTreeMap<&str, Key>,
    names: &[&str],
    deletes: &[&str],
) -> Result<Vec<(Key, bool)>, ActionStateError> {
    names
        .iter()
        .map(|name| Ok((required_key(keys, name)?, deletes.contains(name))))
        .collect()
}
fn capacity_release(before: &View<'_>, after: &View<'_>, r: &Row) -> Result<(), ActionStateError> {
    let keys = r.keys()?;
    let scalar_key = required_key(&keys, SLOTS)?;
    let book_key = required_key(&keys, BOOK)?;
    let scalar: BoundedScalar = parsed(&required(before, &scalar_key)?)?;
    let book: ReservationBook = parsed(&required(before, &book_key)?)?;
    let id: crate::ReservationId = parsed(field(&r.raw, "capacity")?)?;
    let next = book
        .checked_release(book.revision(), scalar, id, 1)
        .map_err(bad)?;
    if required(after, &scalar_key)? != json!(scalar) || required(after, &book_key)? != json!(next)
    {
        return Err(refused("core integration exact native slot release"));
    }
    Ok(())
}
fn advance(
    before: &Row,
    after: &Row,
    identity: &RegisteredTxnIdentity,
) -> Result<(), ActionStateError> {
    if without(&before.raw, &["progress"])? != without(&after.raw, &["progress"])? {
        return Err(refused("core integration progress changed immutable data"));
    }
    let old = field(&before.raw, "progress")?;
    let next = field(&after.raw, "progress")?;
    let prior = field(old, "advances")?
        .as_array()
        .ok_or_else(|| refused("core integration advance array"))?;
    let proof = field(next, "advances")?
        .as_array()
        .ok_or_else(|| refused("core integration advance array"))?;
    if proof.len() != prior.len() + 1 || proof.get(..prior.len()) != Some(prior.as_slice()) {
        return Err(refused("core integration one real occurrence proof"));
    }
    let last = proof
        .last()
        .ok_or_else(|| refused("core integration advance missing"))?;
    let stamp: ClockStamp = parsed(field(last, "observed")?)?;
    let cause = match field(last, "cause")?.as_str() {
        Some("live") => crate::ScheduleDriveCause::Live,
        Some("restart") => crate::ScheduleDriveCause::Restart,
        _ => return Err(refused("core integration drive cause")),
    };
    let cursor: crate::DurableSchedule = parsed(field(old, "cursor")?)?;
    let crate::ScheduleEvaluation::Advance(planned) = cursor
        .plan_due(cursor.revision(), &stamp, cause)
        .map_err(bad)?
    else {
        return Err(refused("core integration occurrence not due"));
    };
    if planned.occurrences().is_empty()
        || planned.skipped().is_some()
        || number(last, "occurrences")? != planned.occurrences().len() as u64
        || field(next, "cursor")? != &json!(planned.post_image())
        || field(last, "commit")? != &json!(identity.commit)
    {
        return Err(refused("core integration native progress differs"));
    }
    let ingress = crate::scheduled_action_ingress(&identity.action, &planned, 0).map_err(bad)?;
    if identity.principal != *ingress.principal() {
        return Err(refused("core integration actual scheduler principal"));
    }
    Ok(())
}
pub(in crate::action_state::terminal_capacity) fn validate_changes(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
    let touches = mutations.keys().any(|k| {
        k.owner.as_str() == "industry"
            && ([ORDER, delivery::CORE, delivery::INDUSTRY].contains(&k.family.as_str())
                || (k.family.as_str() == OUTPUT && store.registry.core_integration.is_some()))
    });
    if !touches {
        return Ok(());
    }
    let protocol = store
        .registry
        .core_integration
        .as_ref()
        .ok_or_else(|| refused("core integration typed write without protocol"))?;
    let identity =
        identity.ok_or_else(|| refused("core integration typed write without invocation"))?;
    let index = protocol.action_index(identity)?;
    let order_keys: Vec<_> = mutations
        .keys()
        .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == ORDER)
        .collect();
    let [order_key] = order_keys.as_slice() else {
        return Err(refused("core integration exactly one order"));
    };
    let order = u64::from_str_radix(&order_key.encoded, 16).map_err(bad)?;
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
    let next = row(&after, order)?;
    let keys = next.keys()?;
    if identity.actor != next.requester()?
        || (index != 2 && identity.principal != (PrincipalId::Object { id: identity.actor }))
    {
        return Err(refused("core integration requester/principal differs"));
    }
    if index == 0 {
        if value(&before, &super::order_key(order)?)?.is_some() || next.accepted != identity.commit
        {
            return Err(refused("core integration order reused"));
        }
        let r = changes
            .funding
            .as_ref()
            .ok_or_else(|| refused("core integration future capacity missing"))?;
        verify(Phase::Funding, r, &after)?;
        let mut expected = expected(&keys, &[ORDER, INPUT, SLOTS, BOOK, SCHEDULE, JOB], &[])?;
        expected.extend([
            (
                scope_key(
                    "industry",
                    "industry_core_integration_deployment",
                    "registry".into(),
                )?,
                false,
            ),
            (
                scope_key(
                    "inventory",
                    "inventory_core_input_heads",
                    format!("{:016x}", number(field(&next.intent, "lot")?, "id")?),
                )?,
                false,
            ),
            (
                scope_key(
                    "inventory",
                    "inventory_core_input_census",
                    "registry".into(),
                )?,
                false,
            ),
            (
                scope_key(
                    "industry",
                    "industry_capacity_index",
                    parsed::<crate::ReservationId>(field(&next.raw, "capacity")?)?
                        .0
                        .to_hex(),
                )?,
                false,
            ),
        ]);
        exact(mutations, &after, expected)?;
        if value(&before, &required_key(&keys, OUTPUT)?)?.is_some() {
            return Err(refused("core integration future output already occupied"));
        }
        return Ok(());
    }
    let prior = row(&before, order)?;
    let id = ReservationId(owner_id("industry")?, prior.accepted.to_hex());
    let retained = store
        .capacity
        .records
        .get(&id)
        .ok_or_else(|| refused("core integration acknowledged reservation absent"))?;
    verify(Phase::Retained, &retained.record, &before)?;
    if next.fingerprint != prior.fingerprint || identity.commit == prior.accepted {
        return Err(refused("core integration immutable admission changed"));
    }
    if index == 5 {
        if !delivery::promised(&prior.policy)?
            || !delivery::completed(&prior)?
            || prior.raw != next.raw
            || delivery::receipts(&before, &prior)?
            || !delivery::receipts(&after, &next)?
            || changes.terminal.as_ref() != Some(&id)
        {
            return Err(refused("core delivery exact pending obligation"));
        }
        let receipt = required(&after, &required_key(&keys, delivery::CORE)?)?;
        if field(&receipt, "operation")? != &json!(identity.commit)
            || field(&receipt, "action")? != &json!(identity.action)
        {
            return Err(refused("core delivery current operation/action"));
        }
        verify(Phase::Terminal, &retained.record, &after)?;
        exact(
            mutations,
            &after,
            expected(&keys, &[ORDER, delivery::CORE, delivery::INDUSTRY], &[])?,
        )?;
    } else if matches!(index, 1 | 2) {
        if changes.progress.as_ref() != Some(&id) {
            return Err(refused("core integration progress marker absent"));
        }
        verify(Phase::Retained, &retained.record, &after)?;
        if index == 1 {
            if without(&prior.raw, &["output"])? != without(&next.raw, &["output"])?
                || next.output()?.is_none()
            {
                return Err(refused("core integration reserved pointer mutation"));
            }
            let next_output = required(&after, &required_key(&keys, OUTPUT)?)?;
            if let Some(old) = prior.output()? {
                if next.output()? != Some(old)
                    || value(&before, &required_key(&keys, OUTPUT)?)? != Some(next_output)
                {
                    return Err(refused("core integration reservation reuse differs"));
                }
                exact(mutations, &after, expected(&keys, &[ORDER, OUTPUT], &[])?)?;
            } else {
                let old = required(&before, &required_key(&keys, DEPLOYMENT)?)?;
                let new = required(&after, &required_key(&keys, DEPLOYMENT)?)?;
                if without(&old, &["born"])? != without(&new, &["born"])?
                    || number(&old, "born")?.checked_add(1) != Some(number(&new, "born")?)
                    || field(&next_output, "reservation")? != &json!(identity.commit)
                    || field(&next_output, "action")? != &json!(identity.action)
                    || field(&next_output, "birth")? != field(&new, "born")?
                {
                    return Err(refused("core integration exact output birth"));
                }
                exact(
                    mutations,
                    &after,
                    expected(&keys, &[ORDER, OUTPUT, DEPLOYMENT], &[])?,
                )?;
            }
        } else {
            advance(&prior, &next, identity)?;
            exact(mutations, &after, expected(&keys, &[ORDER, SCHEDULE], &[])?)?;
        }
    } else {
        let handoff = index == 3 && delivery::promised(&prior.policy)?;
        if (!handoff && changes.terminal.as_ref() != Some(&id))
            || (handoff && changes.industry_delivery.is_none())
            || without(&prior.raw, &["terminal"])? != without(&next.raw, &["terminal"])?
        {
            return Err(refused(
                "core integration terminal admission/progress changed",
            ));
        }
        let terminal = next
            .terminal()?
            .ok_or_else(|| refused("core integration terminal absent"))?;
        if field(terminal, "commit")? != &json!(identity.commit)
            || field(terminal, "action")? != &json!(identity.action)
            || field(field(terminal, "result")?, "outcome")?
                != &json!(if index == 3 { "completed" } else { "cancelled" })
        {
            return Err(refused("core integration current terminal action/commit"));
        }
        if handoff {
            let replacement = changes
                .industry_delivery
                .as_ref()
                .ok_or_else(|| refused("core delivery retained subset absent"))?;
            delivery::require_handoff(&retained.record, replacement)?;
            verify(Phase::Retained, replacement, &after)?;
        } else {
            verify(Phase::Terminal, &retained.record, &after)?;
        }
        capacity_release(&before, &after, &prior)?;
        if index == 3 {
            exact(
                mutations,
                &after,
                expected(
                    &keys,
                    &[
                        ORDER, INPUT, LOT, CENSUS, SLOTS, BOOK, SCHEDULE, JOB, OUTPUT, DEPLOYMENT,
                    ],
                    &[LOT, SCHEDULE, JOB],
                )?,
            )?;
            let old = required(&before, &required_key(&keys, DEPLOYMENT)?)?;
            let new = required(&after, &required_key(&keys, DEPLOYMENT)?)?;
            if without(&old, &["consumed"])? != without(&new, &["consumed"])?
                || number(&old, "consumed")?.checked_add(1) != Some(number(&new, "consumed")?)
            {
                return Err(refused("core integration exact consumed counter"));
            }
            let old = required(&before, &required_key(&keys, CENSUS)?)?
                .as_u64()
                .ok_or_else(|| refused("core integration lot census"))?;
            let new = required(&after, &required_key(&keys, CENSUS)?)?
                .as_u64()
                .ok_or_else(|| refused("core integration lot census"))?;
            if old.checked_sub(1) != Some(new) {
                return Err(refused("core integration exact lot decrement"));
            }
        } else {
            exact(
                mutations,
                &after,
                expected(
                    &keys,
                    &[ORDER, INPUT, SLOTS, BOOK, SCHEDULE, JOB],
                    &[SCHEDULE, JOB],
                )?,
            )?;
        }
    }
    Ok(())
}
pub(in crate::action_state::terminal_capacity) fn audit_boot(
    view: &View<'_>,
    reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
) -> Result<(), ActionStateError> {
    let all: Vec<_> = view
        .records
        .keys()
        .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == ORDER)
        .collect();
    if all.is_empty() && view.registry.core_integration.is_none() {
        return Ok(());
    }
    if view.registry.core_integration.is_none() || all.len() > 8192 {
        return Err(refused("core integration retained orders without protocol"));
    }
    ready(view)?;
    let mut accepted = BTreeSet::new();
    let mut orders = BTreeSet::new();
    for k in all {
        let order = u64::from_str_radix(&k.encoded, 16).map_err(bad)?;
        let r = row(view, order)?;
        if k.encoded != format!("{order:016x}")
            || !accepted.insert(r.accepted)
            || !orders.insert(order)
        {
            return Err(refused("core integration repeated retained identity"));
        }
        paired(view, &r, true)?;
        let id = ReservationId(owner_id("industry")?, r.accepted.to_hex());
        let delivery_pending = delivery::promised(&r.policy)?
            && delivery::completed(&r)?
            && !delivery::receipts(view, &r)?;
        match (r.terminal()?, reservations.get(&id)) {
            (None, Some(stored)) => verify(Phase::Retained, &stored.record, view)?,
            (Some(_), Some(stored)) if delivery_pending => {
                verify(Phase::Retained, &stored.record, view)?
            }
            (Some(_), None) if !delivery_pending => {}
            _ => return Err(refused("core integration missing/extra future reservation")),
        }
    }
    for k in view.records.keys().filter(|k| {
        (k.owner.as_str() == "industry"
            && [OUTPUT, delivery::CORE, delivery::INDUSTRY].contains(&k.family.as_str()))
            || (k.owner.as_str() == "inventory" && k.family.as_str() == INPUT)
    }) {
        let order = if [OUTPUT, delivery::CORE].contains(&k.family.as_str()) {
            k.encoded.parse::<u64>().map_err(bad)?
        } else {
            u64::from_str_radix(&k.encoded, 16).map_err(bad)?
        };
        if !orders.contains(&order) {
            return Err(refused("core integration orphan output/input"));
        }
    }
    Ok(())
}
