use super::*;
fn reservation_id(operation: CommitToken) -> Result<ReservationId, ActionStateError> {
    Ok(ReservationId(
        OwnerId::new("roll").map_err(|_| refused("pending owner"))?,
        operation.to_hex(),
    ))
}
pub(in crate::action_state) fn charter_pending_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    terminal: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let protocol = txn
        .store
        .registry
        .charter_pending
        .as_ref()
        .ok_or_else(|| refused("pending protocol unavailable"))?;
    let identity = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("pending actual invocation unavailable"))?;
    if identity.commit != commit
        || txn.provider_ordinal != Some(identity.ordinal)
        || txn.owner != identity.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.progress.is_some()
        || txn.capacity_changes.industry_delivery.is_some()
    {
        return Err(refused("pending preparation differs"));
    }
    let action = protocol.action_index(identity)?;
    if let Some(operation) = terminal {
        if !(2..=4).contains(&action) {
            return Err(refused("pending terminal action differs"));
        }
        txn.capacity_changes.terminal = Some(reservation_id(operation)?);
    } else {
        if action != 1 {
            return Err(refused("pending funding action differs"));
        }
        let view = View {
            registry: &txn.store.registry,
            records: &txn.store.records,
            mutations: Some(&txn.mutations),
        };
        let p = pair(&view, commit)?;
        require_terminal_room(&p, protocol)?;
        let r = Reservation {
            format: 1,
            funding: commit,
            effect: protocol.actions[1].clone(),
            sponsor: key(PENDING, commit.to_hex())?,
            targets: targets(&p)?,
            protocol_fingerprint: Some(fingerprint(&p)?),
        };
        verify(Phase::Funding, &r, &view)?;
        txn.capacity_changes.funding = Some(r);
    }
    Ok(())
}
fn row_key(k: &StateKey) -> Key {
    Key {
        owner: k.owner.clone(),
        family: k.family.clone(),
        encoded: k.encoded.clone(),
    }
}
fn relevant(view: &View<'_>, k: &StateKey) -> Result<bool, ActionStateError> {
    if k.owner.as_str() != "roll" {
        return Ok(false);
    }
    if [OFFER, PENDING, HEAD].contains(&k.family.as_str()) {
        return Ok(true);
    }
    if k.family.as_str() == TRANSFER {
        return Ok(value(view, &row_key(k))?.is_some_and(|v| v.get("format") == Some(&json!(2))));
    }
    Ok(false)
}
fn exact_puts(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    keys: &[Key],
) -> Result<(), ActionStateError> {
    let expected = keys
        .iter()
        .map(|k| k.resolve(&store.registry))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if expected.len() != keys.len()
        || mutations.len() != expected.len()
        || mutations
            .iter()
            .any(|(k, m)| !expected.contains(k) || !matches!(m, TxnMutation::Put { .. }))
    {
        return Err(refused("pending exact typed union differs"));
    }
    Ok(())
}
fn previous(view: &View<'_>, p: &Pair) -> Result<Option<Pair>, ActionStateError> {
    if p.sequence == 1 {
        return Ok(None);
    }
    let r = field(&p.row, "previous")?;
    let prior = pair(view, token(field(r, "operation")?)?)?;
    if prior.phase == "funded"
        || prior.charter != p.charter
        || prior.sequence.checked_add(1) != Some(p.sequence)
        || pending_ref(&prior)? != *r
    {
        return Err(refused("pending prior terminal differs"));
    }
    Ok(Some(prior))
}
pub(in crate::action_state::terminal_capacity) fn validate_changes(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
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
    let mut touches = false;
    for k in mutations.keys() {
        if relevant(&before, k)? || relevant(&after, k)? {
            touches = true;
            break;
        }
    }
    if !touches {
        return Ok(());
    }
    let protocol = store
        .registry
        .charter_pending
        .as_ref()
        .ok_or_else(|| refused("pending typed write without protocol"))?;
    let identity =
        identity.ok_or_else(|| refused("pending typed write without actual invocation"))?;
    let action = protocol.action_index(identity)?;
    if identity.principal != (PrincipalId::Object { id: identity.actor }) {
        return Err(refused("pending actual principal differs"));
    }
    if action == 0 {
        let k = key(OFFER, identity.commit.to_hex())?;
        exact_puts(store, mutations, std::slice::from_ref(&k))?;
        let o = offer(&after, identity.commit)?;
        if value(&before, &k)?.is_some()
            || num(&o, "seller")? != u64::from(identity.actor.0)
            || changes.funding.is_some()
            || changes.terminal.is_some()
            || changes.progress.is_some()
            || changes.industry_delivery.is_some()
        {
            return Err(refused("pending offer invocation differs"));
        }
        return Ok(());
    }
    let ops = mutations
        .keys()
        .filter(|k| k.owner.as_str() == "roll" && k.family.as_str() == PENDING)
        .map(|k| {
            let id = token(&json!(k.encoded.as_str()))?;
            if id.to_hex() != k.encoded.as_str() {
                return Err(refused("pending key encoding"));
            }
            Ok(id)
        })
        .collect::<Result<Vec<_>, ActionStateError>>()?;
    if ops.len() != 1 {
        return Err(refused("pending one operation per invocation"));
    }
    let id = *ops.first().ok_or_else(|| refused("pending operation"))?;
    let p = pair(&after, id)?;
    let pending_key = key(PENDING, id.to_hex())?;
    let head_key = key(HEAD, charter_key(p.charter))?;
    if action == 1 {
        exact_puts(store, mutations, &[pending_key.clone(), head_key.clone()])?;
        if id != identity.commit
            || p.phase != "funded"
            || value(&before, &pending_key)?.is_some()
            || num(&p.offer, "buyer")? != u64::from(identity.actor.0)
            || changes.funding.as_ref().map(|r| r.funding) != Some(id)
        {
            return Err(refused("pending funding invocation differs"));
        }
        if offer(&before, token(field(&p.row, "offer")?)?)? != p.offer {
            return Err(refused("pending offer must be acknowledged"));
        }
        require_unused_offer(&before, &p)?;
        match previous(&before, &p)? {
            None => {
                if value(&before, &head_key)?.is_some() {
                    return Err(refused("pending first head already exists"));
                }
            }
            Some(prior) => head_matches(&before, &prior)?,
        }
        require_current_ownership(&before, &p)?;
        require_terminal_room(&p, protocol)?;
        verify(
            Phase::Funding,
            changes
                .funding
                .as_ref()
                .ok_or_else(|| refused("pending reservation absent"))?,
            &after,
        )?;
    } else {
        let old = pair(&before, id)?;
        let decision = field(field(field(&p.row, "economic")?, "terminal")?, "decision")?;
        let expected_phase = if action == 2 { "settled" } else { "refunded" };
        if old.phase != "funded"
            || p.phase != expected_phase
            || fingerprint(&old)? != fingerprint(&p)?
            || identity.commit == id
            || token(field(decision, "operation")?)? != identity.commit
            || reference(field(decision, "action")?)? != identity.action
            || num(decision, "actor")? != u64::from(identity.actor.0)
            || changes.terminal.as_ref() != Some(&reservation_id(id)?)
        {
            return Err(refused(
                "pending terminal invocation or immutable fields differ",
            ));
        }
        head_matches(&before, &old)?;
        require_current_ownership(&before, &old)?;
        let future = key(TRANSFER, transfer_key(p.charter, p.generation + 1))?;
        if value(&before, &future)?.is_some() {
            return Err(refused("pending terminal ownership already exists"));
        }
        let mut keys = vec![pending_key, head_key];
        if action == 2 {
            keys.push(future);
            keys.push(key(OWNERSHIP, charter_key(p.charter))?);
        } else {
            require_current_ownership(&after, &old)?;
            if value(&after, &future)?.is_some() {
                return Err(refused("pending refund created ownership"));
            }
            if action == 4 {
                let extra = sanction_keys(&p.row, decision)?;
                for k in &extra {
                    if value(&before, k)?.is_some() {
                        return Err(refused("pending compound sanction must be new"));
                    }
                }
                keys.extend(extra);
            }
        }
        exact_puts(store, mutations, &keys)?;
        let stored = store
            .capacity
            .records
            .get(&reservation_id(id)?)
            .ok_or_else(|| refused("pending retained reservation absent"))?;
        verify(Phase::Terminal, &stored.record, &after)?;
    }
    Ok(())
}
pub(in crate::action_state::terminal_capacity) fn extra_terminal_key(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    record: &Reservation,
    identity: Option<&RegisteredTxnIdentity>,
    candidate: &StateKey,
) -> Result<bool, ActionStateError> {
    let Some(protocol) = &store.registry.charter_pending else {
        return Ok(false);
    };
    if record.effect != protocol.actions[1] || record.sponsor.family.as_str() != PENDING {
        return Ok(false);
    }
    let Some(identity) = identity else {
        return Ok(false);
    };
    if protocol.registry_revocation.as_ref() != Some(&identity.action) {
        return Ok(false);
    }
    let view = View {
        registry: &store.registry,
        records: &store.records,
        mutations: Some(mutations),
    };
    let p = pair(&view, record.funding)?;
    if p.phase != "refunded" {
        return Ok(false);
    }
    let decision = field(field(field(&p.row, "economic")?, "terminal")?, "decision")?;
    if token(field(decision, "operation")?)? != identity.commit
        || field(decision, "action")? != &json!(identity.action)
        || field(field(decision, "reason")?, "sanction")? != &json!(identity.commit)
    {
        return Ok(false);
    }
    for k in sanction_keys(&p.row, decision)? {
        if &k.resolve(&store.registry)? == candidate {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(in crate::action_state::terminal_capacity) fn audit_boot(
    view: &View<'_>,
    reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
) -> Result<(), ActionStateError> {
    let mut touched = Vec::new();
    for k in view.records.keys() {
        if relevant(view, k)? {
            touched.push(k);
        }
    }
    if view.registry.charter_pending.is_none() {
        if !touched.is_empty() {
            return Err(refused("retained pending history without protocol"));
        }
        return Ok(());
    }
    let mut used_offers = BTreeSet::new();
    let mut chains: BTreeMap<u64, BTreeMap<u64, Pair>> = BTreeMap::new();
    let mut transfer_keys = BTreeSet::new();
    let mut head_keys = BTreeSet::new();
    for k in &touched {
        match k.family.as_str() {
            OFFER => {
                let id = token(&json!(k.encoded.as_str()))?;
                if id.to_hex() != k.encoded.as_str() {
                    return Err(refused("pending offer canonical key"));
                }
                offer(view, id)?;
            }
            PENDING => {
                let id = token(&json!(k.encoded.as_str()))?;
                let p = pair(view, id)?;
                if !used_offers.insert(token(field(&p.row, "offer")?)?) {
                    return Err(refused("pending offer reused"));
                }
                if id.to_hex() != k.encoded.as_str() {
                    return Err(refused("pending canonical key"));
                }
                let reservation = reservations.get(&reservation_id(id)?);
                match (p.phase.as_str(), reservation) {
                    ("funded", Some(r)) => verify(Phase::Retained, &r.record, view)?,
                    ("settled", None) => {
                        require_transfer(view, &p)?;
                        transfer_keys.insert(
                            key(TRANSFER, transfer_key(p.charter, p.generation + 1))?
                                .resolve(view.registry)?,
                        );
                    }
                    ("refunded", None) => (),
                    _ => return Err(refused("pending reverse reservation completeness")),
                }
                if chains
                    .entry(p.charter)
                    .or_default()
                    .insert(p.sequence, p)
                    .is_some()
                {
                    return Err(refused("pending duplicate sequence"));
                }
            }
            HEAD => {
                head_keys.insert((*k).clone());
            }
            TRANSFER => (),
            _ => return Err(refused("pending unexpected family")),
        }
    }
    for (charter, rows) in &chains {
        let mut expected = 1u64;
        let mut prior: Option<&Pair> = None;
        for (sequence, p) in rows {
            if *sequence != expected {
                return Err(refused("pending chain gap"));
            }
            match prior {
                None => {
                    if !field(&p.row, "previous")?.is_null() {
                        return Err(refused("pending chain root"));
                    }
                }
                Some(old) => {
                    if old.phase == "funded" || field(&p.row, "previous")? != &pending_ref(old)? {
                        return Err(refused("pending chain predecessor"));
                    }
                }
            }
            prior = Some(p);
            expected = expected
                .checked_add(1)
                .ok_or_else(|| refused("pending sequence overflow"))?;
        }
        let last = prior.ok_or_else(|| refused("pending empty chain"))?;
        head_matches(view, last)?;
        if !head_keys.remove(&key(HEAD, charter_key(*charter))?.resolve(view.registry)?) {
            return Err(refused("pending missing head"));
        }
        // A settled pending attempt permanently requires the shared current
        // ownership projection. It may have advanced through later immediate
        // or pending transfers; walk that one retained chain per charter.
        if let Some(first) = rows
            .values()
            .filter(|p| p.phase == "settled")
            .min_by_key(|p| p.generation)
        {
            let latest = rows
                .values()
                .filter(|p| p.phase == "settled")
                .map(|p| p.generation + 1)
                .max()
                .ok_or_else(|| refused("pending settled ownership absent"))?;
            require_retained_ownership(view, first, latest)?;
        }
    }
    if !head_keys.is_empty() {
        return Err(refused("pending orphan head"));
    }
    for k in touched
        .into_iter()
        .filter(|k| k.family.as_str() == TRANSFER)
    {
        if !transfer_keys.remove(k) {
            return Err(refused("pending orphan format2 ownership"));
        }
    }
    if !transfer_keys.is_empty() {
        return Err(refused("pending ownership reverse gap"));
    }
    Ok(())
}

fn require_unused_offer(view: &View<'_>, p: &Pair) -> Result<(), ActionStateError> {
    for k in view
        .records
        .keys()
        .filter(|k| k.owner.as_str() == "roll" && k.family.as_str() == PENDING)
    {
        let old = required(view, &row_key(k))?;
        if field(&old, "offer")? == field(&p.row, "offer")? {
            return Err(refused("pending offer already accepted"));
        }
    }
    Ok(())
}
