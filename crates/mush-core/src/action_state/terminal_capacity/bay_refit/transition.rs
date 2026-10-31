//! The exact write set of each pinned action, and the cold audit.
use super::*;
pub(in crate::action_state) fn bay_refit_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    order: u64,
    admission: bool,
) -> Result<(), ActionStateError> {
    let p = txn
        .store
        .registry
        .bay_refit
        .as_ref()
        .ok_or_else(|| refused("bay refit protocol unavailable"))?;
    let identity = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("bay refit actual invocation absent"))?;
    if identity.commit != commit
        || txn.provider_ordinal != Some(identity.ordinal)
        || txn.owner != identity.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.progress.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.industry_delivery.is_some()
    {
        return Err(refused("bay refit exact preparation"));
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
            return Err(refused("bay refit admission action/commit"));
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
            return Err(refused("bay refit terminal/progress identity"));
        }
        let id = ReservationId(owner_id("industry")?, r.accepted.to_hex());
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
        .ok_or_else(|| refused("bay refit object"))?;
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
        return Err(refused("bay refit exact native mutation set"));
    }
    Ok(())
}
fn advance(
    before: &Row,
    after: &Row,
    identity: &RegisteredTxnIdentity,
) -> Result<(), ActionStateError> {
    if without(&before.raw, &["progress"])? != without(&after.raw, &["progress"])? {
        return Err(refused("bay refit progress changed immutable data"));
    }
    let old = field(&before.raw, "progress")?;
    let next = field(&after.raw, "progress")?;
    let prior = field(old, "advances")?
        .as_array()
        .ok_or_else(|| refused("bay refit advance array"))?;
    let proof = field(next, "advances")?
        .as_array()
        .ok_or_else(|| refused("bay refit advance array"))?;
    if proof.len() != prior.len() + 1 || proof.get(..prior.len()) != Some(prior.as_slice()) {
        return Err(refused("bay refit one real occurrence proof"));
    }
    let last = proof
        .last()
        .ok_or_else(|| refused("bay refit advance missing"))?;
    let stamp: ClockStamp = parsed(field(last, "observed")?)?;
    let cursor: crate::DurableSchedule = parsed(field(old, "cursor")?)?;
    let crate::ScheduleEvaluation::Advance(planned) = cursor
        .plan_due(cursor.revision(), &stamp, wire::cause(last)?)
        .map_err(bad)?
    else {
        return Err(refused("bay refit occurrence not due"));
    };
    if planned.occurrences().is_empty()
        || planned.skipped().is_some()
        || number(last, "occurrences")? != planned.occurrences().len() as u64
        || field(next, "cursor")? != &json!(planned.post_image())
        || field(last, "commit")? != &json!(identity.commit)
    {
        return Err(refused("bay refit native progress differs"));
    }
    let ingress = crate::scheduled_action_ingress(&identity.action, &planned, 0).map_err(bad)?;
    if identity.principal != *ingress.principal() {
        return Err(refused("bay refit actual scheduler principal"));
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
        return Err(refused("bay refit exact native slot release"));
    }
    Ok(())
}
/// Accept's preimages: the hull is in no bay (nor pending a core refit),
/// the part's lot is in the bay's stores and not yet held for this order,
/// and the returned lot does not exist yet.
fn admission_heads(before: &View<'_>, r: &Row) -> Result<(), ActionStateError> {
    if let Some(h) = value(before, &r.key(HULL)?)?
        && h.get("pending") != Some(&Value::Null)
    {
        return Err(refused("bay refit hull already in a bay"));
    }
    if before.registry.core_refit.is_some()
        && let Some(h) = value(before, &key(CORE_HULL, r.hull()?.0.to_string())?)?
        && h.get("pending") != Some(&Value::Null)
    {
        return Err(refused("bay refit hull pending a core refit"));
    }
    if let Some(lot) = r.part_lot()? {
        let current = required(before, &lot)?;
        if field(&current, "endpoint")? != &json!(r.stores()?)
            || value(before, &r.key(PART)?)?.is_some()
        {
            return Err(refused("bay refit part lot preimage"));
        }
    }
    if let Some(lot) = r.returned_lot()?
        && value(before, &lot)?.is_some()
    {
        return Err(refused("bay refit returned lot occupied"));
    }
    Ok(())
}
/// Install's stores census: one part out, one part back, exactly.
fn census(before: &View<'_>, after: &View<'_>, r: &Row) -> Result<(), ActionStateError> {
    let count = |v: &View<'_>| -> Result<u64, ActionStateError> {
        required(v, &r.key(CENSUS)?)?
            .as_u64()
            .ok_or_else(|| refused("bay refit lot census"))
    };
    let (out, back) = (r.part()?.is_some(), r.returned()?.is_some());
    let expected = count(before)?
        .checked_sub(u64::from(out))
        .and_then(|n| n.checked_add(u64::from(back)));
    if expected != Some(count(after)?) {
        return Err(refused("bay refit exact stores census"));
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
        (k.owner.as_str() == "industry" && [ORDER, HULL].contains(&k.family.as_str()))
            || (k.owner.as_str() == "inventory" && k.family.as_str() == PART)
    }) {
        return Ok(());
    }
    let p = store
        .registry
        .bay_refit
        .as_ref()
        .ok_or_else(|| refused("bay refit protected write without protocol"))?;
    let identity = identity.ok_or_else(|| refused("bay refit unregistered write"))?;
    let index = p.action_index(identity)?;
    let selected = mutations
        .keys()
        .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == ORDER)
        .collect::<Vec<_>>();
    let [selected] = selected.as_slice() else {
        return Err(refused("bay refit exactly one order"));
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
        return Err(refused("bay refit requester identity"));
    }
    let part = next.part()?.is_some();
    if index == 0 {
        if value(&before, &order_key(order)?)?.is_some() || next.accepted != identity.commit {
            return Err(refused("bay refit fresh order identity"));
        }
        for k in before
            .records
            .keys()
            .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == ORDER)
        {
            let prior = row(&before, u64::from_str_radix(&k.encoded, 16).map_err(bad)?)?;
            if field(&prior.offer, "operation")? == field(&next.offer, "operation")? {
                return Err(refused("bay refit supplier offer reused"));
            }
        }
        let reservation = changes
            .funding
            .as_ref()
            .ok_or_else(|| refused("bay refit missing future capacity"))?;
        verify(Phase::Funding, reservation, &after)?;
        admission_heads(&before, &next)?;
        let mut expected = keys(&next, &[ORDER, HULL, SLOTS, BOOK, SCHEDULE, JOB], &[])?;
        expected.push((key(DEPLOYMENT, "registry".into())?, false));
        expected.push((
            key(
                "industry_capacity_index",
                parsed::<crate::ReservationId>(field(&next.raw, "capacity")?)?
                    .0
                    .to_hex(),
            )?,
            false,
        ));
        if part {
            expected.extend(keys(&next, &[PART, PART_HEAD], &[])?);
            expected.push((key(PART_CENSUS, "registry".into())?, false));
        }
        exact(mutations, &after, expected)?;
        return Ok(());
    }
    let prior = row(&before, order)?;
    let id = ReservationId(owner_id("industry")?, prior.accepted.to_hex());
    let stored = store
        .capacity
        .records
        .get(&id)
        .ok_or_else(|| refused("bay refit reservation missing"))?;
    verify(Phase::Retained, &stored.record, &before)?;
    if prior.fingerprint != next.fingerprint || identity.commit == prior.accepted {
        return Err(refused("bay refit immutable admission changed"));
    }
    if index == 1 {
        if changes.progress.as_ref() != Some(&id) {
            return Err(refused("bay refit missing progress marker"));
        }
        advance(&prior, &next, identity)?;
        verify(Phase::Retained, &stored.record, &after)?;
        exact(mutations, &after, keys(&next, &[ORDER, SCHEDULE], &[])?)?;
        return Ok(());
    }
    if changes.terminal.as_ref() != Some(&id)
        || without(&prior.raw, &["terminal", "installation"])?
            != without(&next.raw, &["terminal", "installation"])?
    {
        return Err(refused("bay refit changed terminal preimage"));
    }
    let terminal = next
        .terminal()?
        .ok_or_else(|| refused("bay refit terminal absent"))?;
    if field(terminal, "commit")? != &json!(identity.commit)
        || field(terminal, "action")? != &json!(identity.action)
        || field(field(terminal, "result")?, "outcome")?
            != &json!(if index == 2 { "installed" } else { "cancelled" })
    {
        return Err(refused("bay refit actual terminal identity"));
    }
    verify(Phase::Terminal, &stored.record, &after)?;
    release(&before, &after, &prior)?;
    let mut expected = keys(
        &next,
        &[ORDER, HULL, SLOTS, BOOK, SCHEDULE, JOB],
        &[SCHEDULE, JOB],
    )?;
    if part {
        expected.extend(keys(&next, &[PART], &[])?);
    }
    if index == 2 {
        if let Some(lot) = next.part_lot()? {
            expected.push((lot, true));
        }
        if let Some(lot) = next.returned_lot()? {
            expected.push((lot, false));
        }
        if part || next.returned()?.is_some() {
            census(&before, &after, &next)?;
            expected.extend(keys(&next, &[CENSUS], &[])?);
        }
    }
    exact(mutations, &after, expected)
}
pub(in crate::action_state::terminal_capacity) fn audit_boot(
    view: &View<'_>,
    reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
) -> Result<(), ActionStateError> {
    let protected = |k: &&StateKey| {
        (k.owner.as_str() == "industry" && [ORDER, HULL].contains(&k.family.as_str()))
            || (k.owner.as_str() == "inventory" && k.family.as_str() == PART)
    };
    let any = view.records.keys().any(|k| protected(&k));
    if !any && view.registry.bay_refit.is_none() {
        return Ok(());
    }
    if view.registry.bay_refit.is_none() {
        return Err(refused("bay refit retained evidence without protocol"));
    }
    ready(view)?;
    let mut rows = BTreeMap::new();
    let mut accepted = BTreeSet::new();
    let mut offers = BTreeSet::new();
    let mut sequences = BTreeSet::new();
    for k in view
        .records
        .keys()
        .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == ORDER)
    {
        let order = u64::from_str_radix(&k.encoded, 16).map_err(bad)?;
        let r = row(view, order)?;
        if rows.len() >= 8192
            || k.encoded != format!("{order:016x}")
            || !accepted.insert(r.accepted)
            || !offers.insert(parsed::<CommitToken>(field(&r.offer, "operation")?)?)
            || !sequences.insert(number(&r.raw, "sequence")?)
        {
            return Err(refused("bay refit duplicate birth"));
        }
        history::paired(view, &r, true)?;
        let id = ReservationId(owner_id("industry")?, r.accepted.to_hex());
        match (r.terminal()?, reservations.get(&id)) {
            (None, Some(s)) => verify(Phase::Retained, &s.record, view)?,
            (Some(_), None) => {}
            _ => return Err(refused("bay refit missing/extra terminal capacity")),
        }
        rows.insert(order, r);
    }
    if !sequences.iter().copied().eq(1..=rows.len() as u64) {
        return Err(refused("bay refit admission sequence"));
    }
    history::heads(view, &rows)?;
    let mut schedules = BTreeSet::new();
    for r in rows
        .values()
        .filter(|r| r.terminal().is_ok_and(|t| t.is_none()))
    {
        schedules.insert(r.key(SCHEDULE)?.encoded);
    }
    for k in view.records.keys() {
        let orphan = match (k.owner.as_str(), k.family.as_str()) {
            ("industry", SCHEDULE | JOB) => !schedules.contains(&k.encoded),
            ("inventory", PART) => {
                let id = u64::from_str_radix(&k.encoded, 16).map_err(bad)?;
                k.encoded != format!("{id:016x}")
                    || rows
                        .get(&id)
                        .is_none_or(|r| r.part().is_ok_and(|p| p.is_none()))
            }
            _ => false,
        };
        if orphan {
            return Err(refused("bay refit orphan schedule or part"));
        }
    }
    Ok(())
}
