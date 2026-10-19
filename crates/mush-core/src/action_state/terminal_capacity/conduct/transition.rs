use super::*;
fn protected(k: &StateKey) -> bool {
    k.owner.as_str() == "roll" && FAMILIES.iter().any(|(f, _)| k.family.as_str() == *f)
}
pub(in crate::action_state) fn conduct_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    selected: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let p = txn
        .store
        .registry
        .conduct_trial
        .as_ref()
        .ok_or_else(|| refused("Conduct protocol unavailable"))?;
    let id = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("Conduct registered invocation"))?;
    if id.commit != commit
        || txn.provider_ordinal != Some(id.ordinal)
        || txn.owner != id.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.progress.is_some()
        || txn.capacity_changes.industry_delivery.is_some()
    {
        return Err(refused("Conduct exact preparation"));
    }
    let index = p.action_index(id)?;
    let view = View {
        registry: &txn.store.registry,
        records: &txn.store.records,
        mutations: Some(&txn.mutations),
    };
    match selected {
        None if index == 0 => {
            let (job, _) = pair(&view, commit)?;
            let r = Reservation {
                format: 1,
                funding: commit,
                effect: job.action.clone(),
                sponsor: key(JOB, commit)?,
                targets: targets(commit)?,
                protocol_fingerprint: Some(job.digest()?),
            };
            verify(Phase::Funding, &r, &view)?;
            txn.capacity_changes.funding = Some(r);
        }
        Some(job) if index > 0 && job != commit => {
            let (_, execution) = pair(&view, job)?;
            let reservation = ReservationId(owner("roll")?, job.to_hex());
            if execution.terminal() {
                txn.capacity_changes.terminal = Some(reservation);
            } else {
                txn.capacity_changes.progress = Some(reservation);
            }
        }
        _ => return Err(refused("Conduct reserve/advance action mismatch")),
    }
    Ok(())
}
pub(super) fn progression(
    old: &wire::Execution,
    new: &wire::Execution,
    index: usize,
) -> Result<(), ActionStateError> {
    if old.terminal() || old.operation == new.operation {
        return Err(refused("Conduct repeated or terminal execution"));
    }
    match index {
        1 => {
            let flight = new
                .flight
                .as_ref()
                .ok_or_else(|| refused("Conduct grant needs actual flight"))?;
            if old.flight.is_some()
                || !["queued", "running"].contains(&old.phase.as_str())
                || new.phase != "running"
                || old.sequence.checked_add(1) != Some(new.sequence)
                || old.index != new.index
                || old.simulated != new.simulated
                || old
                    .charged
                    .checked_add(number(flight, "allowance_unit_ms")?)
                    != Some(new.charged)
            {
                return Err(refused("Conduct exact prepaid grant"));
            }
        }
        2 => {
            let flight = old
                .flight
                .as_ref()
                .ok_or_else(|| refused("Conduct settle needs prior flight"))?;
            let floor = old
                .charged
                .checked_sub(number(flight, "allowance_unit_ms")?)
                .ok_or_else(|| refused("Conduct charge underflow"))?;
            if new.flight.is_some()
                || new.sequence != old.sequence
                || !(floor..=old.charged).contains(&new.charged)
                || new.index < old.index
                || new.index > old.index + 1
                || (new.index == old.index && new.simulated < old.simulated)
                || !["running", "complete", "failed", "cancelled"].contains(&new.phase.as_str())
                || (old.phase == "cancel_requested" && new.phase != "cancelled")
            {
                return Err(refused("Conduct exact flight settlement"));
            }
        }
        3 => {
            let phase = if old.flight.is_some() {
                "cancel_requested"
            } else {
                "cancelled"
            };
            if new.phase != phase
                || new.flight != old.flight
                || new.sequence != old.sequence
                || new.index != old.index
                || new.simulated != old.simulated
                || new.charged != old.charged
                || old.phase == "cancel_requested"
            {
                return Err(refused("Conduct cancellation transition"));
            }
        }
        4 => {
            if new.phase != "interrupted"
                || new.flight.is_some()
                || new.sequence != old.sequence
                || new.index != old.index
                || new.simulated != old.simulated
                || new.charged != old.charged
            {
                return Err(refused("Conduct stopped-worker recovery"));
            }
        }
        _ => return Err(refused("Conduct progression action")),
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
        .conduct_trial
        .as_ref()
        .ok_or_else(|| refused("Conduct write without protocol"))?;
    let id = identity.ok_or_else(|| refused("Conduct unregistered write"))?;
    let index = p.action_index(id)?;
    if id.actor != crate::GOD
        || id.principal != (PrincipalId::Object { id: crate::GOD })
        || changes.industry_delivery.is_some()
    {
        return Err(refused("Conduct actual service invocation"));
    }
    let selected = mutations
        .keys()
        .filter(|k| k.owner.as_str() == "roll" && k.family.as_str() == EXECUTION)
        .collect::<Vec<_>>();
    let [selected] = selected.as_slice() else {
        return Err(refused("Conduct exactly one execution"));
    };
    let job_id = CommitToken::from_hex(&selected.encoded).map_err(bad)?;
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
    ready(&after)?;
    let (job, next) = pair(&after, job_id)?;
    p.current(&job, index)?;
    if next.operation != id.commit || next.action != id.action {
        return Err(refused("Conduct exact execution operation"));
    }
    let expected = if index == 0 {
        if job.id != id.commit
            || changes.progress.is_some()
            || changes.terminal.is_some()
            || FAMILIES
                .iter()
                .any(|(f, _)| get(&before, f, job_id).map_or(true, |v| v.is_some()))
            || store
                .records
                .keys()
                .filter(|k| k.owner.as_str() == "roll" && k.family.as_str() == JOB)
                .count()
                >= 4096
        {
            return Err(refused("Conduct fresh bounded admission"));
        }
        verify(
            Phase::Funding,
            changes
                .funding
                .as_ref()
                .ok_or_else(|| refused("Conduct missing future reservation"))?,
            &after,
        )?;
        vec![key(JOB, job_id)?, key(EXECUTION, job_id)?]
    } else {
        let (prior_job, prior) = pair(&before, job_id)?;
        let reservation_id = ReservationId(owner("roll")?, job_id.to_hex());
        let saved = store
            .capacity
            .records
            .get(&reservation_id)
            .ok_or_else(|| refused("Conduct lost reservation"))?;
        verify(Phase::Retained, &saved.record, &before)?;
        if prior_job.raw != job.raw || changes.funding.is_some() {
            return Err(refused("Conduct immutable job"));
        }
        progression(&prior, &next, index)?;
        if next.terminal() {
            if changes.terminal.as_ref() != Some(&reservation_id) || changes.progress.is_some() {
                return Err(refused("Conduct exact terminal consumption"));
            }
            verify(Phase::Terminal, &saved.record, &after)?;
            vec![key(EXECUTION, job_id)?, key(RESULT, job_id)?]
        } else {
            if changes.progress.as_ref() != Some(&reservation_id) || changes.terminal.is_some() {
                return Err(refused("Conduct retained progress reservation"));
            }
            verify(Phase::Retained, &saved.record, &after)?;
            vec![key(EXECUTION, job_id)?]
        }
    };
    let keys = expected
        .iter()
        .map(|k| k.resolve(after.registry))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if keys.len() != mutations.len()
        || mutations
            .iter()
            .any(|(k, m)| !keys.contains(k) || matches!(m, TxnMutation::Delete))
    {
        return Err(refused("Conduct exact typed mutation set"));
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
    if all.is_empty() && view.registry.conduct_trial.is_none() {
        return Ok(());
    }
    if view.registry.conduct_trial.is_none() {
        return Err(refused("Conduct history without configured owner"));
    }
    ready(view)?;
    let mut jobs = BTreeSet::new();
    for k in all.iter().filter(|k| k.family.as_str() == JOB) {
        let id = CommitToken::from_hex(&k.encoded).map_err(bad)?;
        if k.encoded != id.to_hex() || jobs.len() >= 4096 || !jobs.insert(id) {
            return Err(refused("Conduct job inventory"));
        }
        let (job, e) = pair(view, id)?;
        wire::future_room(view, &job)?;
        match (
            e.terminal(),
            reservations.get(&ReservationId(owner("roll")?, id.to_hex())),
        ) {
            (false, Some(r)) => verify(Phase::Retained, &r.record, view)?,
            (true, None) => {}
            _ => return Err(refused("Conduct exact reservation inventory")),
        }
    }
    for k in all {
        let id = CommitToken::from_hex(&k.encoded).map_err(bad)?;
        if k.encoded != id.to_hex() || !jobs.contains(&id) {
            return Err(refused("Conduct orphan counterpart"));
        }
    }
    Ok(())
}
