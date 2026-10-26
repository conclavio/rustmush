use super::*;
pub(in crate::action_state) fn refit_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    order: u64,
    admission: bool,
) -> Result<(), ActionStateError> {
    let p = txn
        .store
        .registry
        .core_refit
        .as_ref()
        .ok_or_else(|| refused("core refit protocol unavailable"))?;
    let identity = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("core refit actual invocation absent"))?;
    if identity.commit != commit
        || txn.provider_ordinal != Some(identity.ordinal)
        || txn.owner != identity.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.progress.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.industry_delivery.is_some()
    {
        return Err(refused("core refit exact preparation"));
    }
    let index = p.action_index(identity)?;
    let view = View {
        registry: &txn.store.registry,
        records: &txn.store.records,
        mutations: Some(&txn.mutations),
    };
    let r = row(&view, order)?;
    if admission {
        if index != 0 || r.accepted != commit {
            return Err(refused("core refit admission action/commit"));
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
            return Err(refused("core refit terminal/progress identity"));
        }
        let id = ReservationId(owner_id()?, r.accepted.to_hex());
        if index == 1 {
            txn.capacity_changes.progress = Some(id)
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
        .ok_or_else(|| refused("core refit object"))?;
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
        return Err(refused("core refit exact native mutation set"));
    }
    Ok(())
}

fn advance(
    before: &Row,
    after: &Row,
    identity: &RegisteredTxnIdentity,
) -> Result<(), ActionStateError> {
    if without(&before.raw, &["progress"])? != without(&after.raw, &["progress"])? {
        return Err(refused("core refit progress changed immutable data"));
    }
    let old = field(&before.raw, "progress")?;
    let next = field(&after.raw, "progress")?;
    let prior = field(old, "advances")?
        .as_array()
        .ok_or_else(|| refused("core refit advance array"))?;
    let proof = field(next, "advances")?
        .as_array()
        .ok_or_else(|| refused("core refit advance array"))?;
    if proof.len() != prior.len() + 1 || proof.get(..prior.len()) != Some(prior.as_slice()) {
        return Err(refused("core refit one real occurrence proof"));
    }
    let last = proof
        .last()
        .ok_or_else(|| refused("core refit advance missing"))?;
    let stamp: ClockStamp = parsed(field(last, "observed")?)?;
    let cause = match field(last, "cause")?.as_str() {
        Some("live") => crate::ScheduleDriveCause::Live,
        Some("restart") => crate::ScheduleDriveCause::Restart,
        _ => return Err(refused("core refit drive cause")),
    };
    let cursor: crate::DurableSchedule = parsed(field(old, "cursor")?)?;
    let crate::ScheduleEvaluation::Advance(planned) = cursor
        .plan_due(cursor.revision(), &stamp, cause)
        .map_err(bad)?
    else {
        return Err(refused("core refit occurrence not due"));
    };
    if planned.occurrences().is_empty()
        || planned.skipped().is_some()
        || number(last, "occurrences")? != planned.occurrences().len() as u64
        || field(next, "cursor")? != &json!(planned.post_image())
        || field(last, "commit")? != &json!(identity.commit)
    {
        return Err(refused("core refit native progress differs"));
    }
    let ingress = crate::scheduled_action_ingress(&identity.action, &planned, 0).map_err(bad)?;
    if identity.principal != *ingress.principal() {
        return Err(refused("core refit actual scheduler principal"));
    }
    Ok(())
}

fn keys(r: &Row, names: &[&str], deletes: &[&str]) -> Result<Vec<(Key, bool)>, ActionStateError> {
    names
        .iter()
        .map(|n| Ok((r.key(n)?, deletes.contains(n))))
        .collect()
}
fn release(before: &View<'_>, after: &View<'_>, r: &Row) -> Result<(), ActionStateError> {
    let scalar: BoundedScalar = parsed(&required(before, &r.key(SLOTS)?)?)?;
    let book: ReservationBook = parsed(&required(before, &r.key(BOOK)?)?)?;
    let id: crate::ReservationId = parsed(field(&r.raw, "capacity")?)?;
    let next = book
        .checked_release(book.revision(), scalar, id, 1)
        .map_err(bad)?;
    if required(after, &r.key(SLOTS)?)? != json!(scalar)
        || required(after, &r.key(BOOK)?)? != json!(next)
    {
        return Err(refused("core refit exact native slot release"));
    }
    Ok(())
}
fn admission_heads(before: &View<'_>, r: &Row) -> Result<(), ActionStateError> {
    if let Some(a) = value(before, &r.key(ASSET)?)?
        && a.pointer("/position/state") != Some(&json!("released"))
    {
        return Err(refused("core refit incoming preimage"));
    }
    match (value(before, &r.key(HULL)?)?, r.displaced()?) {
        (None, None) => {}
        (Some(h), old)
            if h.get("pending") == Some(&Value::Null) && h.get("mounted") == Some(&json!(old)) => {}
        _ => return Err(refused("core refit hull preimage")),
    }
    Ok(())
}
pub(in crate::action_state::terminal_capacity) fn validate_changes(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
    if !mutations.keys().any(|k| {
        k.owner.as_str() == "industry"
            && ([ORDER, ASSET, HULL].contains(&k.family.as_str())
                || (k.family.as_str() == EXCHANGE && store.registry.core_refit.is_some()))
    }) {
        return Ok(());
    }
    let p = store
        .registry
        .core_refit
        .as_ref()
        .ok_or_else(|| refused("core refit protected write without protocol"))?;
    let identity = identity.ok_or_else(|| refused("core refit unregistered write"))?;
    let index = p.action_index(identity)?;
    let selected = mutations
        .keys()
        .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == ORDER)
        .collect::<Vec<_>>();
    let [selected] = selected.as_slice() else {
        return Err(refused("core refit exactly one order"));
    };
    let order = u64::from_str_radix(&selected.encoded, 16).map_err(bad)?;
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
    if identity.actor != next.requester()?
        || (index != 1 && identity.principal != (PrincipalId::Object { id: identity.actor }))
    {
        return Err(refused("core refit requester identity"));
    }
    if index == 0 {
        if value(&before, &order_key(order)?)?.is_some() || next.accepted != identity.commit {
            return Err(refused("core refit fresh order identity"));
        }
        for k in before
            .records
            .keys()
            .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == ORDER)
        {
            let prior = row(&before, u64::from_str_radix(&k.encoded, 16).map_err(bad)?)?;
            if field(&prior.offer, "operation")? == field(&next.offer, "operation")? {
                return Err(refused("core refit supplier offer reused"));
            }
        }
        let reservation = changes
            .funding
            .as_ref()
            .ok_or_else(|| refused("core refit missing future capacity"))?;
        verify(Phase::Funding, reservation, &after)?;
        admission_heads(&before, &next)?;
        if value(&before, &next.key(EXCHANGE)?)?.is_some() {
            return Err(refused("core refit future receipt occupied"));
        }
        let mut expected = keys(
            &next,
            &[ORDER, ASSET, HULL, SLOTS, BOOK, SCHEDULE, JOB],
            &[],
        )?;
        expected.push((key("industry_refit_deployment", "registry".into())?, false));
        expected.push((
            key(
                "industry_capacity_index",
                parsed::<crate::ReservationId>(field(&next.raw, "capacity")?)?
                    .0
                    .to_hex(),
            )?,
            false,
        ));
        exact(mutations, &after, expected)?;
        return Ok(());
    }
    let prior = row(&before, order)?;
    let id = ReservationId(owner_id()?, prior.accepted.to_hex());
    let stored = store
        .capacity
        .records
        .get(&id)
        .ok_or_else(|| refused("core refit reservation missing"))?;
    verify(Phase::Retained, &stored.record, &before)?;
    if prior.fingerprint != next.fingerprint || identity.commit == prior.accepted {
        return Err(refused("core refit immutable admission changed"));
    }
    if index == 1 {
        if changes.progress.as_ref() != Some(&id) {
            return Err(refused("core refit missing progress marker"));
        }
        advance(&prior, &next, identity)?;
        verify(Phase::Retained, &stored.record, &after)?;
        exact(mutations, &after, keys(&next, &[ORDER, SCHEDULE], &[])?)?;
    } else {
        if changes.terminal.as_ref() != Some(&id)
            || without(&prior.raw, &["terminal", "installation"])?
                != without(&next.raw, &["terminal", "installation"])?
        {
            return Err(refused("core refit changed terminal preimage"));
        }
        let terminal = next
            .terminal()?
            .ok_or_else(|| refused("core refit terminal absent"))?;
        if field(terminal, "commit")? != &json!(identity.commit)
            || field(terminal, "action")? != &json!(identity.action)
            || field(field(terminal, "result")?, "outcome")?
                != &json!(if index == 2 { "installed" } else { "cancelled" })
        {
            return Err(refused("core refit actual terminal identity"));
        }
        verify(Phase::Terminal, &stored.record, &after)?;
        release(&before, &after, &prior)?;
        let mut expected = keys(
            &next,
            &[ORDER, ASSET, HULL, SLOTS, BOOK, SCHEDULE, JOB],
            &[SCHEDULE, JOB],
        )?;
        if index == 2 {
            expected.push((next.key(EXCHANGE)?, false));
            if let Some(old) = next.displaced()? {
                expected.push((key(ASSET, old.0.to_string())?, false));
            }
        }
        exact(mutations, &after, expected)?;
    }
    Ok(())
}
pub(in crate::action_state::terminal_capacity) fn audit_boot(
    view: &View<'_>,
    reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
) -> Result<(), ActionStateError> {
    let all = view
        .records
        .keys()
        .filter(|k| {
            k.owner.as_str() == "industry" && [ORDER, ASSET, HULL].contains(&k.family.as_str())
        })
        .collect::<Vec<_>>();
    if all.is_empty() && view.registry.core_refit.is_none() {
        return Ok(());
    }
    if view.registry.core_refit.is_none() {
        return Err(refused("core refit retained evidence without protocol"));
    }
    ready(view)?;
    let mut rows = BTreeMap::new();
    let mut accepted = BTreeSet::new();
    let mut offers = BTreeSet::new();
    let mut sequences = BTreeSet::new();
    for k in all.iter().filter(|k| k.family.as_str() == ORDER) {
        let order = u64::from_str_radix(&k.encoded, 16).map_err(bad)?;
        let r = row(view, order)?;
        if rows.len() >= 8192
            || k.encoded != format!("{order:016x}")
            || !accepted.insert(r.accepted)
            || !offers.insert(parsed::<CommitToken>(field(&r.offer, "operation")?)?)
            || !sequences.insert(number(&r.raw, "sequence")?)
        {
            return Err(refused("core refit duplicate birth"));
        }
        history::paired(view, &r, true)?;
        let id = ReservationId(owner_id()?, r.accepted.to_hex());
        match (r.terminal()?, reservations.get(&id)) {
            (None, Some(s)) => verify(Phase::Retained, &s.record, view)?,
            (Some(_), None) => {}
            _ => return Err(refused("core refit missing/extra terminal capacity")),
        }
        rows.insert(order, r);
    }
    if !sequences.iter().copied().eq(1..=rows.len() as u64) {
        return Err(refused("core refit admission sequence"));
    }
    history::heads(view, &rows)?;
    let mut schedules = BTreeSet::new();
    for r in rows
        .values()
        .filter(|r| r.terminal().is_ok_and(|t| t.is_none()))
    {
        schedules.insert(r.key(SCHEDULE)?.encoded);
    }
    for family in [EXCHANGE, SCHEDULE, JOB] {
        for k in view
            .records
            .keys()
            .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == family)
        {
            if family == EXCHANGE {
                let id = u64::from_str_radix(&k.encoded, 16).map_err(bad)?;
                if k.encoded != format!("{id:016x}")
                    || rows
                        .get(&id)
                        .is_none_or(|r| r.installation().is_ok_and(|i| i.is_none()))
                {
                    return Err(refused("core refit orphan physical receipt"));
                }
            } else if !schedules.contains(&k.encoded) {
                return Err(refused("core refit orphan schedule"));
            }
        }
    }
    Ok(())
}
