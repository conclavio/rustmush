use super::*;
fn own(k: &StateKey) -> bool {
    (k.owner.as_str() == "roll" && [ROLL, HEAD, HOLDER].contains(&k.family.as_str()))
        || (k.owner.as_str() == "market" && [OFFER, MARKET, RECEIPT].contains(&k.family.as_str()))
}
fn row_key(k: &StateKey) -> Key {
    Key {
        owner: k.owner.clone(),
        family: k.family.clone(),
        encoded: k.encoded.clone(),
    }
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
        return Err(refused("resale exact typed union differs"));
    }
    Ok(())
}
fn no_other_capacity(changes: &Changes) -> bool {
    changes.funding.is_none()
        && changes.terminal.is_none()
        && changes.progress.is_none()
        && changes.industry_delivery.is_none()
}
fn previous(view: &View<'_>, p: &Pair) -> Result<Option<Pair>, ActionStateError> {
    if p.sequence == 1 {
        return Ok(None);
    }
    let r = field(&p.row, "previous")?;
    let prior = pair(view, token(field(r, "operation")?)?)?;
    if prior.phase == "funded"
        || prior.licence != p.licence
        || prior.sequence.checked_add(1) != Some(p.sequence)
        || operation_ref(&prior)? != *r
    {
        return Err(refused("resale predecessor is not exact terminal"));
    }
    Ok(Some(prior))
}
fn require_unused_offer(view: &View<'_>, p: &Pair) -> Result<(), ActionStateError> {
    for k in view
        .records
        .keys()
        .filter(|k| k.owner.as_str() == "roll" && k.family.as_str() == ROLL)
    {
        let old = required(view, &row_key(k))?;
        if field(&old, "offer")? == field(&p.row, "offer")? {
            return Err(refused("resale offer already accepted"));
        }
    }
    Ok(())
}
fn no_pending_renewal(view: &View<'_>, licence: u64) -> Result<(), ActionStateError> {
    for family in ["roll_mind_renewal_head", "roll_house_mind_renewal_head"] {
        if let Some(row) = optional(view, family, licence_key(licence))?
            && !field(&row, "pending")?.is_null()
        {
            return Err(refused("resale cannot cross open renewal"));
        }
    }
    Ok(())
}
fn offer_current(view: &View<'_>, o: &Value) -> Result<(), ActionStateError> {
    let binding = field(o, "binding")?;
    history::binding_history(view, binding)?;
    let seller = field(field(o, "terms")?, "seller")?;
    history::captured_holder(view, binding, seller)?;
    let id = decimal(field(binding, "licence")?, u64::MAX)?;
    let current = history::current_pointer(view, id)?;
    let g = ownership(field(binding, "ownership")?)?;
    if field(&current, "holder")? != seller
        || field(&current, "term")? != field(field(binding, "term")?, "generation")?
        || current.get("ownership")
            != if g == 1 {
                None
            } else {
                Some(field(binding, "ownership")?)
            }
    {
        return Err(refused("resale seller current pointer differs"));
    }
    if let Some(head) = value(view, &key(HEAD, licence_key(id))?)?
        && text(field(&head, "state")?, "phase")? == "funded"
    {
        return Err(refused("resale prior custody pending"));
    }
    no_pending_renewal(view, id)
}
pub(in crate::action_state::terminal_capacity) fn validate_changes(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
    if !mutations.keys().any(own) {
        return Ok(());
    }
    let protocol = store
        .registry
        .paid_resale
        .as_ref()
        .ok_or_else(|| refused("resale writes without protocol"))?;
    let identity = identity.ok_or_else(|| refused("resale writes without invocation"))?;
    let action = protocol.action_index(identity)?;
    if identity.principal != (PrincipalId::Object { id: identity.actor }) {
        return Err(refused("resale actual principal differs"));
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
    if action == 0 {
        let k = key(OFFER, identity.commit.to_hex())?;
        exact_puts(store, mutations, std::slice::from_ref(&k))?;
        let row = offer(&after, identity.commit)?;
        if value(&before, &k)?.is_some()
            || num(field(&row, "terms")?, "seller")? != u64::from(identity.actor.0)
            || !no_other_capacity(changes)
        {
            return Err(refused("resale offer invocation differs"));
        }
        offer_current(&before, &row)?;
        return Ok(());
    }
    let ops = mutations
        .keys()
        .filter(|k| k.owner.as_str() == "roll" && k.family.as_str() == ROLL)
        .map(|k| {
            let id = token(&json!(k.encoded.as_str()))?;
            if id.to_hex() != k.encoded.as_str() {
                return Err(refused("resale operation key"));
            }
            Ok(id)
        })
        .collect::<Result<Vec<_>, ActionStateError>>()?;
    if ops.len() != 1 {
        return Err(refused("resale one operation per invocation"));
    }
    let id = *ops
        .first()
        .ok_or_else(|| refused("resale operation missing"))?;
    let p = pair(&after, id)?;
    let mut keys = vec![
        key(ROLL, id.to_hex())?,
        key(MARKET, id.to_hex())?,
        key(HEAD, licence_key(p.licence))?,
    ];
    let future = key(HOLDER, holder_key(p.licence, p.generation + 1))?;
    let receipt_key = key(RECEIPT, id.to_hex())?;
    if matches!(action, 1 | 2) {
        if id != identity.commit
            || value(&before, &key(ROLL, id.to_hex())?)?.is_some()
            || value(&before, &key(MARKET, id.to_hex())?)?.is_some()
            || value(&before, &future)?.is_some()
            || value(&before, &receipt_key)?.is_some()
            || num(field(&p.offer, "terms")?, "buyer")? != u64::from(identity.actor.0)
            || reference(field(&p.row, "action")?)? != identity.action
        {
            return Err(refused("resale acceptance invocation differs"));
        }
        if offer(&before, token(field(&p.row, "offer")?)?)? != p.offer {
            return Err(refused("resale offer not acknowledged"));
        }
        require_unused_offer(&before, &p)?;
        no_pending_renewal(&before, p.licence)?;
        match previous(&before, &p)? {
            None => {
                if value(&before, &key(HEAD, licence_key(p.licence))?)?.is_some() {
                    return Err(refused("resale first head exists"));
                }
            }
            Some(prior) => head_matches(&before, &prior)?,
        }
        let old = history::require_current(&before, &p)?;
        if action == 1 {
            if p.phase != "settled"
                || terminal(&p)? != identity.commit
                || !no_other_capacity(changes)
            {
                return Err(refused("resale immediate acceptance differs"));
            }
            keys.extend([key(LICENCE, licence_key(p.licence))?, future, receipt_key]);
            if history::current_pointer(&after, p.licence)? != history::next_pointer(&old, &p)? {
                return Err(refused("resale immediate pointer changed beyond holder"));
            }
            history::outputs(&after, &p, true)?;
        } else {
            if p.phase != "funded"
                || changes.funding.as_ref().map(|r| r.funding) != Some(id)
                || changes.terminal.is_some()
                || changes.progress.is_some()
                || changes.industry_delivery.is_some()
                || history::current_pointer(&after, p.licence)? != old
            {
                return Err(refused("resale funded acceptance differs"));
            }
            capacity::require_terminal_room(&before, &p)?;
            capacity::verify(
                Phase::Funding,
                changes
                    .funding
                    .as_ref()
                    .ok_or_else(|| refused("resale reservation absent"))?,
                &after,
            )?;
        }
    } else {
        let prior = pair(&before, id)?;
        let stored = store
            .capacity
            .records
            .get(&capacity::reservation_id(id)?)
            .ok_or_else(|| refused("resale retained reservation absent"))?;
        capacity::verify(Phase::Retained, &stored.record, &before)?;
        let d = field(field(field(&p.row, "economic")?, "terminal")?, "decision")?;
        if prior.phase != "funded"
            || p.phase != if action == 3 { "settled" } else { "refunded" }
            || fingerprint(&prior)? != fingerprint(&p)?
            || terminal(&p)? != identity.commit
            || reference(field(d, "action")?)? != identity.action
            || num(d, "actor")? != u64::from(identity.actor.0)
            || changes.terminal.as_ref() != Some(&capacity::reservation_id(id)?)
            || changes.funding.is_some()
            || changes.progress.is_some()
            || changes.industry_delivery.is_some()
        {
            return Err(refused("resale terminal invocation differs"));
        }
        head_matches(&before, &prior)?;
        let old = history::require_current(&before, &prior)?;
        if value(&before, &future)?.is_some() || value(&before, &receipt_key)?.is_some() {
            return Err(refused("resale premature terminal row"));
        }
        keys.push(receipt_key);
        if action == 3 {
            keys.extend([key(LICENCE, licence_key(p.licence))?, future]);
            if history::current_pointer(&after, p.licence)? != history::next_pointer(&old, &p)? {
                return Err(refused("resale settled pointer changed beyond holder"));
            }
        } else {
            if history::current_pointer(&after, p.licence)? != old
                || value(&after, &future)?.is_some()
            {
                return Err(refused("resale Refund changed holder"));
            }
            if action == 5 {
                for k in native::sanction_keys(&p.row, d)? {
                    if value(&before, &k)?.is_some() {
                        return Err(refused("resale compound sanction already exists"));
                    }
                    keys.push(k);
                }
            }
        }
        capacity::verify(Phase::Terminal, &stored.record, &after)?;
    }
    exact_puts(store, mutations, &keys)?;
    head_matches(&after, &p)
}
/// Only the exact configured Registry compound action may consume a Market
/// reservation from Roll. Typed transition validation has already checked the
/// full ordinary cause/status union; this independently pins the paired old
/// custody and new terminal to the captured reservation and actual invocation.
pub(in crate::action_state::terminal_capacity) fn compound_terminal_owner(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    record: &Reservation,
    identity: Option<&RegisteredTxnIdentity>,
    owner: &OwnerId,
) -> Result<bool, ActionStateError> {
    let Some(protocol) = &store.registry.paid_resale else {
        return Ok(false);
    };
    let Some(identity) = identity else {
        return Ok(false);
    };
    if owner.as_str() != "roll"
        || identity.action.key().owner != *owner
        || protocol.registry_revocation.as_ref() != Some(&identity.action)
        || record.effect != protocol.actions[2]
        || record.sponsor != key(MARKET, record.funding.to_hex())?
        || identity.principal != (PrincipalId::Object { id: identity.actor })
    {
        return Ok(false);
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
    capacity::verify(Phase::Retained, record, &before)?;
    capacity::verify(Phase::Terminal, record, &after)?;
    let prior = pair(&before, record.funding)?;
    let next = pair(&after, record.funding)?;
    let decision = field(
        field(field(&next.row, "economic")?, "terminal")?,
        "decision",
    )?;
    if next.phase != "refunded"
        || fingerprint(&prior)? != fingerprint(&next)?
        || terminal(&next)? != identity.commit
        || field(decision, "action")? != &json!(identity.action)
        || num(decision, "actor")? != u64::from(identity.actor.0)
        || text(field(decision, "reason")?, "kind")? != "registry_revoked"
        || field(field(decision, "reason")?, "sanction")? != &json!(identity.commit)
    {
        return Err(refused("resale cross-owner terminal differs"));
    }
    Ok(true)
}
pub(in crate::action_state::terminal_capacity) fn extra_terminal_key(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    record: &Reservation,
    identity: Option<&RegisteredTxnIdentity>,
    candidate: &StateKey,
) -> Result<bool, ActionStateError> {
    let Some(protocol) = &store.registry.paid_resale else {
        return Ok(false);
    };
    let Some(identity) = identity else {
        return Ok(false);
    };
    if record.effect != protocol.actions[2]
        || record.sponsor.family.as_str() != MARKET
        || protocol.registry_revocation.as_ref() != Some(&identity.action)
    {
        return Ok(false);
    }
    let view = View {
        registry: &store.registry,
        records: &store.records,
        mutations: Some(mutations),
    };
    let p = pair(&view, record.funding)?;
    if p.phase != "refunded" || terminal(&p)? != identity.commit {
        return Ok(false);
    }
    let d = field(field(field(&p.row, "economic")?, "terminal")?, "decision")?;
    if field(d, "action")? != &json!(identity.action)
        || field(field(d, "reason")?, "sanction")? != &json!(identity.commit)
    {
        return Ok(false);
    }
    for k in native::sanction_keys(&p.row, d)? {
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
    let touched = view.records.keys().filter(|k| own(k)).collect::<Vec<_>>();
    if view.registry.paid_resale.is_none() {
        if !touched.is_empty() {
            return Err(refused("resale retained history without protocol"));
        }
        for k in view
            .records
            .keys()
            .filter(|k| k.owner.as_str() == "roll" && k.family.as_str() == LICENCE)
        {
            if required(view, &row_key(k))?.get("format") == Some(&json!(2)) {
                return Err(refused("resale pointer without declaration"));
            }
        }
        return Ok(());
    }
    let mut chains: BTreeMap<u64, BTreeMap<u64, Pair>> = BTreeMap::new();
    let mut offers = BTreeSet::new();
    let mut expected = BTreeSet::new();
    let mut heads = BTreeSet::new();
    for k in &touched {
        match k.family.as_str() {
            OFFER => {
                let id = token(&json!(k.encoded.as_str()))?;
                if id.to_hex() != k.encoded.as_str() {
                    return Err(refused("resale offer key"));
                }
                let o = offer(view, id)?;
                history::binding_history(view, field(&o, "binding")?)?;
                history::captured_holder(
                    view,
                    field(&o, "binding")?,
                    field(field(&o, "terms")?, "seller")?,
                )?;
            }
            ROLL => {
                let id = token(&json!(k.encoded.as_str()))?;
                if id.to_hex() != k.encoded.as_str() {
                    return Err(refused("resale operation key"));
                }
                let p = pair(view, id)?;
                if !offers.insert(token(field(&p.row, "offer")?)?) {
                    return Err(refused("resale reused seller offer"));
                }
                history::binding_history(view, field(&p.row, "binding")?)?;
                history::captured_holder(
                    view,
                    field(&p.row, "binding")?,
                    field(field(&p.offer, "terms")?, "seller")?,
                )?;
                expected.insert(key(MARKET, id.to_hex())?.resolve(view.registry)?);
                match (
                    p.phase.as_str(),
                    reservations.get(&capacity::reservation_id(id)?),
                ) {
                    ("funded", Some(r)) => {
                        capacity::verify(Phase::Retained, &r.record, view)?;
                        no_pending_renewal(view, p.licence)?;
                    }
                    ("settled" | "refunded", None) => {
                        history::outputs(view, &p, false)?;
                        expected.insert(key(RECEIPT, id.to_hex())?.resolve(view.registry)?);
                        if p.phase == "settled" {
                            expected.insert(
                                key(HOLDER, holder_key(p.licence, p.generation + 1))?
                                    .resolve(view.registry)?,
                            );
                        }
                    }
                    _ => return Err(refused("resale reverse reservation completeness")),
                }
                if chains
                    .entry(p.licence)
                    .or_default()
                    .insert(p.sequence, p)
                    .is_some()
                {
                    return Err(refused("resale duplicate sequence"));
                }
            }
            HEAD => {
                heads.insert((*k).clone());
            }
            MARKET | RECEIPT | HOLDER => (),
            _ => return Err(refused("resale unknown retained family")),
        }
    }
    for (licence, rows) in chains {
        let mut prior: Option<&Pair> = None;
        let mut seq = 1;
        let mut holder = None;
        let mut latest_term = 0;
        for (sequence, p) in &rows {
            let binding = field(&p.row, "binding")?;
            if *sequence != seq {
                return Err(refused("resale sequence gap"));
            }
            match prior {
                None => {
                    if !field(&p.row, "previous")?.is_null() || p.generation != 1 {
                        return Err(refused("resale chain root"));
                    }
                }
                Some(old) => {
                    if old.phase == "funded" || field(&p.row, "previous")? != &operation_ref(old)? {
                        return Err(refused("resale chain predecessor"));
                    }
                }
            }
            if let Some(h) = &holder
                && field(binding, "ownership")? != h
            {
                return Err(refused("resale holder chain discontinuity"));
            }
            let term = decimal(field(field(binding, "term")?, "generation")?, u64::MAX)?;
            if term < latest_term {
                return Err(refused("resale accepted term rollback"));
            }
            latest_term = term;
            holder = Some(if p.phase == "settled" {
                field(field(&p.row, "state")?, "ownership")?.clone()
            } else {
                field(binding, "ownership")?.clone()
            });
            prior = Some(p);
            seq += 1;
        }
        let last = prior.ok_or_else(|| refused("resale empty history"))?;
        head_matches(view, last)?;
        if !heads.remove(&key(HEAD, licence_key(licence))?.resolve(view.registry)?) {
            return Err(refused("resale missing latest head"));
        }
        let current = history::current_pointer(view, licence)?;
        let h = holder.ok_or_else(|| refused("resale current holder missing"))?;
        if ownership(&h)? == 1 {
            if num(&current, "format")? != 1 {
                return Err(refused("resale unearned holder generation"));
            }
        } else if current.get("ownership") != Some(&h) {
            return Err(refused("resale current holder head differs"));
        }
        if decimal(field(&current, "term")?, u64::MAX)? < latest_term {
            return Err(refused("resale current term precedes acceptance"));
        }
    }
    if !heads.is_empty() {
        return Err(refused("resale orphan head"));
    }
    for k in touched
        .into_iter()
        .filter(|k| [MARKET, RECEIPT, HOLDER].contains(&k.family.as_str()))
    {
        if !expected.remove(k) {
            return Err(refused("resale orphan paired row"));
        }
    }
    if !expected.is_empty() {
        return Err(refused("resale paired row missing"));
    }
    Ok(())
}
