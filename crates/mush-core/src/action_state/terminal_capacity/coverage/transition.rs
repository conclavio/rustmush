use super::*;
use wire::{integer, without};
fn protected(k: &StateKey) -> bool {
    FAMILIES
        .iter()
        .any(|(o, f, _)| k.owner.as_str() == *o && k.family.as_str() == *f)
}
pub(in crate::action_state) fn coverage_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    binding: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let p = txn
        .store
        .registry
        .hull_coverage
        .as_ref()
        .ok_or_else(|| refused("coverage protocol unavailable"))?;
    let id = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("coverage registered invocation required"))?;
    if id.commit != commit
        || txn.provider_ordinal != Some(id.ordinal)
        || txn.owner != id.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.progress.is_some()
        || txn.capacity_changes.industry_delivery.is_some()
    {
        return Err(refused("coverage exact actual preparation"));
    }
    let index = p.action_index(id)?;
    let view = View {
        registry: &txn.store.registry,
        records: &txn.store.records,
        mutations: Some(&txn.mutations),
    };
    match binding {
        None if index == 0 => {
            let r = row(&view, commit)?;
            let reservation = Reservation {
                format: 1,
                funding: commit,
                effect: r.action.clone(),
                sponsor: r.key(POLICY)?,
                targets: targets(&r)?,
                protocol_fingerprint: Some(r.fingerprint(&view)?),
            };
            verify(Phase::Funding, &reservation, &view)?;
            txn.capacity_changes.funding = Some(reservation);
        }
        Some(binding) if index > 0 && binding != commit => {
            txn.capacity_changes.terminal = Some(ReservationId(owner("tacsim")?, binding.to_hex()))
        }
        _ => return Err(refused("coverage completion action mismatch")),
    }
    Ok(())
}
fn counts(v: &Value) -> Result<u64, ActionStateError> {
    shape(v, &["format", "products", "quotes", "policies"])?;
    if number(v, "format")? != 1
        || ["products", "quotes", "policies"]
            .iter()
            .any(|n| number(v, n).map_or(true, |v| v > 65536))
    {
        return Err(refused("coverage exact admission counts"));
    }
    number(v, "policies")
}
fn chain(prior: &Row, next: &Row) -> Result<(), ActionStateError> {
    let d = prior
        .decision()?
        .ok_or_else(|| refused("coverage overlapping hull exposure"))?;
    let start: ClockStamp = parsed(field(next.terms()?, "starts_at")?)?;
    let terminal: ClockStamp = parsed(field(d, "at_ic")?)?;
    if next.hull != prior.hull
        || next.previous != Some(prior.binding)
        || prior.sequence.checked_add(1) != Some(next.sequence)
        || integer(next.consent()?, "accepted_at_utc")? < integer(d, "posted_at_utc")?
        || (start.clock == terminal.clock && start.at < terminal.at)
    {
        return Err(refused("coverage hull history chain"));
    }
    Ok(())
}
pub(in crate::action_state::terminal_capacity) fn validate_changes(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
    if !mutations.keys().any(protected) {
        return Ok(());
    }
    let p = store
        .registry
        .hull_coverage
        .as_ref()
        .ok_or_else(|| refused("coverage protected write without profile"))?;
    let id = identity.ok_or_else(|| refused("coverage unregistered protected write"))?;
    let index = p.action_index(id)?;
    let selected = mutations
        .keys()
        .filter(|k| k.owner.as_str() == "tacsim" && k.family.as_str() == POLICY)
        .collect::<Vec<_>>();
    let [selected] = selected.as_slice() else {
        return Err(refused("coverage exactly one policy"));
    };
    let binding = CommitToken::from_hex(&selected.encoded).map_err(bad)?;
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
    let next = row(&after, binding)?;
    if id.principal != (PrincipalId::Object { id: id.actor })
        || changes.progress.is_some()
        || changes.industry_delivery.is_some()
    {
        return Err(refused("coverage actual object invocation"));
    }
    let expected = if index == 0 {
        if binding != id.commit
            || get(&before, &next.key(POLICY)?)?.is_some()
            || next.terminal()?.is_some()
            || id.actor != parsed::<ObjId>(field(next.terms()?, "buyer")?)?
            || protocol::Captured::parse(field(&next.raw, "protocol")?)? != *p.captured()
            || changes.terminal.is_some()
        {
            return Err(refused("coverage fresh actual binding"));
        }
        match next.previous {
            None => {
                if get(&before, &next.key(HULL)?)?.is_some() {
                    return Err(refused("coverage first hull already exists"));
                }
            }
            Some(previous) => {
                let old = row(&before, previous)?;
                chain(&old, &next)?;
                if required(&before, &old.key(HULL)?)? != old.head()? {
                    return Err(refused("coverage stale predecessor head"));
                }
            }
        }
        let prior_counts = required(&before, &key(COUNTS, "registry".into())?)?;
        let next_counts = required(&after, &key(COUNTS, "registry".into())?)?;
        if counts(&prior_counts)?.checked_add(1) != Some(counts(&next_counts)?)
            || without(&prior_counts, &["policies"])? != without(&next_counts, &["policies"])?
        {
            return Err(refused("coverage exact policy admission count"));
        }
        for k in before.records.keys().filter(|k| {
            k.owner.as_str() == "tacsim" && [QUOTE, POLICY, TERMINAL].contains(&k.family.as_str())
        }) {
            let prior = required(&before, &key(k.family.as_str(), k.encoded.to_string())?)?;
            if field(&prior, "request")? == field(&next.raw, "request")?
                || (k.family.as_str() == POLICY
                    && field(&prior, "quote")? == field(&next.raw, "quote")?)
            {
                return Err(refused("coverage reused departure quote"));
            }
        }
        let reservation = changes
            .funding
            .as_ref()
            .ok_or_else(|| refused("coverage no completion reservation"))?;
        verify(Phase::Funding, reservation, &after)?;
        vec![
            next.key(POLICY)?,
            next.key(EXPOSURE)?,
            next.key(HULL)?,
            next.key(RETENTION)?,
            key(COUNTS, "registry".into())?,
        ]
    } else {
        let prior = row(&before, binding)?;
        let reservation_id = ReservationId(owner("tacsim")?, binding.to_hex());
        let saved = store
            .capacity
            .records
            .get(&reservation_id)
            .ok_or_else(|| refused("coverage lost reservation"))?;
        verify(Phase::Retained, &saved.record, &before)?;
        let d = next
            .decision()?
            .ok_or_else(|| refused("coverage missing terminal"))?;
        let terminal = required(&after, &next.key(TERMINAL)?)?;
        if changes.funding.is_some()
            || changes.terminal.as_ref() != Some(&reservation_id)
            || next.immutable()? != prior.immutable()?
            || prior.fingerprint(&before)? != next.fingerprint(&after)?
            || field(d, "operation")? != &json!(id.commit)
            || field(d, "action")? != &json!(id.action)
            || field(d, "actor")? != &json!(id.actor)
            || protocol::Captured::parse(field(&terminal, "protocol")?)? != *p.captured()
            || required(&before, &prior.key(HULL)?)? != prior.head()?
            || required(&after, &next.key(HULL)?)? != next.head()?
        {
            return Err(refused("coverage exact terminal transition"));
        }
        let request = field(&terminal, "request")?;
        for k in before.records.keys().filter(|k| {
            k.owner.as_str() == "tacsim" && [QUOTE, POLICY, TERMINAL].contains(&k.family.as_str())
        }) {
            if field(
                &required(&before, &key(k.family.as_str(), k.encoded.to_string())?)?,
                "request",
            )? == request
            {
                return Err(refused("coverage terminal request reused"));
            }
        }
        verify(Phase::Terminal, &saved.record, &after)?;
        FAMILIES
            .iter()
            .map(|(_, f, _)| next.key(f))
            .collect::<Result<Vec<_>, _>>()?
    };
    let keys = expected
        .iter()
        .map(|k| k.resolve(after.registry))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if keys.len() != mutations.len()
        || mutations
            .iter()
            .any(|(k, v)| !keys.contains(k) || matches!(v, TxnMutation::Delete))
    {
        return Err(refused("coverage exact typed mutation set"));
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
        .filter(|k| protected(k))
        .collect::<Vec<_>>();
    if all.is_empty() && view.registry.hull_coverage.is_none() {
        return Ok(());
    }
    if view.registry.hull_coverage.is_none() {
        return Err(refused("coverage retained data without profile"));
    }
    ready(view)?;
    let mut rows = BTreeMap::new();
    let mut requests = BTreeSet::new();
    for k in view
        .records
        .keys()
        .filter(|k| k.owner.as_str() == "tacsim" && k.family.as_str() == QUOTE)
    {
        let quote = required(view, &key(QUOTE, k.encoded.to_string())?)?;
        if !requests.insert(wire::nonzero_digest(field(&quote, "request")?)?) {
            return Err(refused("coverage duplicate quote request"));
        }
    }
    let mut quotes = BTreeSet::new();
    let mut heads: BTreeMap<ObjId, Vec<CommitToken>> = BTreeMap::new();
    for k in all
        .iter()
        .filter(|k| k.owner.as_str() == "tacsim" && k.family.as_str() == POLICY)
    {
        let binding = CommitToken::from_hex(&k.encoded).map_err(bad)?;
        let r = row(view, binding)?;
        if k.encoded != binding.to_hex()
            || rows.len() >= 65536
            || !requests.insert(wire::nonzero_digest(field(&r.raw, "request")?)?)
            || !quotes.insert(parsed::<CommitToken>(field(&r.raw, "quote")?)?)
        {
            return Err(refused("coverage duplicate policy admission"));
        }
        r.paired(view)?;
        r.future_room(view)?;
        if r.terminal()?.is_some() {
            let t = required(view, &r.key(TERMINAL)?)?;
            if !requests.insert(wire::nonzero_digest(field(&t, "request")?)?) {
                return Err(refused("coverage duplicate terminal request"));
            }
        }
        let id = ReservationId(owner("tacsim")?, binding.to_hex());
        match (r.terminal()?, reservations.get(&id)) {
            (None, Some(s)) => verify(Phase::Retained, &s.record, view)?,
            (Some(_), None) => {}
            _ => return Err(refused("coverage missing or extra completion reservation")),
        }
        heads.entry(r.hull).or_default().push(binding);
        rows.insert(binding, r);
    }
    for sequence in heads.values_mut() {
        sequence.sort_by_key(|id| rows[id].sequence);
        let first = &rows[&sequence[0]];
        if first.sequence != 1 || first.previous.is_some() {
            return Err(refused("coverage missing first hull policy"));
        }
        for pair in sequence.windows(2) {
            chain(&rows[&pair[0]], &rows[&pair[1]])?;
        }
        let latest = &rows[sequence
            .last()
            .ok_or_else(|| refused("coverage empty hull chain"))?];
        if required(view, &latest.key(HULL)?)? != latest.head()? {
            return Err(refused("coverage missing exact latest head"));
        }
    }
    for k in all {
        if k.family.as_str() == HULL {
            let hull = ObjId(u32::from_str_radix(&k.encoded, 16).map_err(bad)?);
            if k.encoded != format!("{:08x}", hull.0) || !heads.contains_key(&hull) {
                return Err(refused("coverage orphan hull head"));
            }
        } else {
            let id = CommitToken::from_hex(&k.encoded).map_err(bad)?;
            let r = rows
                .get(&id)
                .ok_or_else(|| refused("coverage orphan counterpart"))?;
            if k.encoded != id.to_hex()
                || ([TERMINAL, RECEIPT].contains(&k.family.as_str()) && r.terminal()?.is_none())
            {
                return Err(refused("coverage orphan terminal"));
            }
        }
    }
    if let Some(c) = get(view, &key(COUNTS, "registry".into())?)? {
        if counts(&c)? != rows.len() as u64 {
            return Err(refused("coverage missing admission history"));
        }
    } else if !rows.is_empty() {
        return Err(refused("coverage missing counts"));
    }
    Ok(())
}
