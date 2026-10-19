use super::*;
pub(super) fn asset(r: &Row, terminal: bool) -> Result<Value, ActionStateError> {
    let position = if !terminal {
        json!({"state":"pending","order":r.order})
    } else if r.installation()?.is_some() {
        json!({"state":"mounted","order":r.order,"hull":r.hull()?})
    } else {
        json!({"state":"released","order":r.order})
    };
    let operation = if terminal {
        field(
            r.terminal()?.ok_or_else(|| refused("refit terminal"))?,
            "commit",
        )?
        .clone()
    } else {
        json!(r.accepted)
    };
    Ok(json!({"format":1,"object":r.incoming()?,"operation":operation,"position":position}))
}
pub(super) fn hull(r: &Row, terminal: bool) -> Result<Value, ActionStateError> {
    let installed = terminal && r.installation()?.is_some();
    let operation = if terminal {
        field(
            r.terminal()?.ok_or_else(|| refused("refit terminal"))?,
            "commit",
        )?
        .clone()
    } else {
        json!(r.accepted)
    };
    Ok(
        json!({"format":1,"hull":r.hull()?,"operation":operation,"pending":if terminal{Value::Null}else{json!(r.order)},"mounted":if installed{Some(r.incoming()?)}else{r.displaced()?}}),
    )
}
pub(super) fn detached(r: &Row) -> Result<Value, ActionStateError> {
    let terminal = r.terminal()?.ok_or_else(|| refused("refit terminal"))?;
    Ok(
        json!({"format":1,"object":r.displaced()?.ok_or_else(||refused("refit displaced"))?,"operation":field(terminal,"commit")?,"position":{"state":"detached","order":r.order,"from_hull":r.hull()?,"condition":digest("industry.core_refit.displaced.v1",field(&r.offer,"displaced")?)?}}),
    )
}
fn mounted(v: &Value, hull: ObjId) -> Result<bool, ActionStateError> {
    Ok(field(field(v, "position")?, "state")? == &json!("mounted")
        && field(field(v, "position")?, "hull")? == &json!(hull))
}
pub(super) fn paired(view: &View<'_>, r: &Row, historical: bool) -> Result<(), ActionStateError> {
    let exchange = value(view, &r.key(EXCHANGE)?)?;
    match (r.terminal()?, r.installation()?, exchange) {
        (None, None, None) => {
            if required(view, &r.key(ASSET)?)? != asset(r, false)?
                || required(view, &r.key(HULL)?)? != hull(r, false)?
                || required(view, &r.key(SCHEDULE)?)?
                    != *field(field(&r.raw, "progress")?, "cursor")?
                || required(view, &r.key(JOB)?)? != json!(r.order)
            {
                return Err(refused("core refit retained heads/schedule"));
            }
            if let Some(old) = r.displaced()? {
                if !mounted(&required(view, &key(ASSET, old.0.to_string())?)?, r.hull()?)? {
                    return Err(refused("core refit displaced pending head"));
                }
            }
            let scalar: BoundedScalar = parsed(&required(view, &r.key(SLOTS)?)?)?;
            let book: ReservationBook = parsed(&required(view, &r.key(BOOK)?)?)?;
            book.checked_available(scalar).map_err(bad)?;
            let id: crate::ReservationId = parsed(field(&r.raw, "capacity")?)?;
            let held = book
                .records()
                .iter()
                .find(|x| x.id() == id)
                .ok_or_else(|| refused("core refit actual slot missing"))?;
            if book.resource() != &r.resource()?
                || held.holder() != &r.holder()?
                || held.state() != crate::ReservationState::Held
                || held.amount() != 1
                || held.expires() != &parsed::<ClockStamp>(field(&r.intent, "expires_at")?)?
            {
                return Err(refused("core refit exact native slot"));
            }
        }
        (Some(_), None, None) => {}
        (Some(t), Some(i), Some(exchange)) => {
            if field(&exchange, "format")? != &json!(1)
                || field(&exchange, "order")? != &json!(r.order)
                || field(&exchange, "operation")? != field(t, "commit")?
                || field(&exchange, "action")? != field(t, "action")?
                || field(&exchange, "requester")? != &json!(r.requester()?)
                || field(&exchange, "hull")? != &json!(r.hull()?)
                || field(&exchange, "unit_revision")? != field(&r.offer, "unit_revision")?
                || field(&exchange, "incoming")? != &json!(r.incoming()?)
                || field(&exchange, "source")? != &json!(r.requester()?)
                || field(&exchange, "outgoing")? != &json!(r.displaced()?)
                || field(&exchange, "return_to")?
                    != &json!(r.displaced()?.map(|_| r.requester()).transpose()?)
                || field(&exchange, "at_utc")? != field(t, "posted_at")?
                || field(i, "core_receipt")?
                    != &json!(digest(
                        "industry.core_component_exchange.receipt.v1",
                        &exchange
                    )?)
            {
                return Err(refused("core refit permanent physical receipt join"));
            }
            if !historical
                && field(&exchange, "participants")?
                    != &json!(
                        view.registry
                            .core_refit
                            .as_ref()
                            .ok_or_else(|| refused("core refit protocol"))?
                            .terminal_participants()
                    )
            {
                return Err(refused("core refit actual terminal owners"));
            }
        }
        _ => return Err(refused("core refit missing or crossed physical receipt")),
    }
    if r.terminal()?.is_some() && !historical {
        if value(view, &r.key(SCHEDULE)?)?.is_some()
            || value(view, &r.key(JOB)?)?.is_some()
            || required(view, &r.key(ASSET)?)? != asset(r, true)?
            || required(view, &r.key(HULL)?)? != hull(r, true)?
        {
            return Err(refused("core refit terminal heads/schedule"));
        }
        if r.installation()?.is_some()
            && let Some(old) = r.displaced()?
        {
            if required(view, &key(ASSET, old.0.to_string())?)? != detached(r)? {
                return Err(refused("core refit displaced terminal condition"));
            }
        }
    }
    Ok(())
}
/// Reconstruct mutable current heads from immutable actual payment ordering.
/// Later refits may replace the head; they never rewrite earlier terminal rows.
pub(super) fn heads(view: &View<'_>, rows: &BTreeMap<u64, Row>) -> Result<(), ActionStateError> {
    let mut events = BTreeMap::new();
    for (id, r) in rows {
        if events
            .insert(
                number(field(&r.raw, "funding")?, "journal_seq")?,
                (*id, false),
            )
            .is_some()
        {
            return Err(refused("core refit duplicate journal identity"));
        }
        if let Some(t) = r.terminal()? {
            if events
                .insert(number(t, "journal_seq")?, (*id, true))
                .is_some()
            {
                return Err(refused("core refit duplicate terminal identity"));
            }
        }
    }
    let mut assets = BTreeMap::<ObjId, Value>::new();
    let mut hulls = BTreeMap::<ObjId, Value>::new();
    for (id, terminal) in events.values() {
        let r = rows
            .get(id)
            .ok_or_else(|| refused("core refit history row"))?;
        let incoming = r.incoming()?;
        let unit = r.hull()?;
        if !terminal {
            if assets
                .get(&incoming)
                .is_some_and(|v| v.pointer("/position/state") != Some(&json!("released")))
            {
                return Err(refused("core refit incoming already held or installed"));
            }
            match (hulls.get(&unit), r.displaced()?) {
                (None, None) => {}
                (Some(h), old)
                    if h.get("pending") == Some(&Value::Null)
                        && h.get("mounted") == Some(&json!(old)) => {}
                _ => return Err(refused("core refit historical hull reservation")),
            }
            if let Some(old) = r.displaced()? {
                if !assets
                    .get(&old)
                    .map(|v| mounted(v, unit))
                    .transpose()?
                    .unwrap_or(false)
                {
                    return Err(refused("core refit historical displaced head"));
                }
            }
            assets.insert(incoming, asset(r, false)?);
            hulls.insert(unit, hull(r, false)?);
        } else {
            if assets.get(&incoming) != Some(&asset(r, false)?)
                || hulls.get(&unit) != Some(&hull(r, false)?)
            {
                return Err(refused("core refit historical terminal preimage"));
            }
            assets.insert(incoming, asset(r, true)?);
            hulls.insert(unit, hull(r, true)?);
            if r.installation()?.is_some()
                && let Some(old) = r.displaced()?
            {
                assets.insert(old, detached(r)?);
            }
        }
    }
    for (family, expected) in [(ASSET, assets), (HULL, hulls)] {
        let mut remaining = expected;
        for k in view
            .records
            .keys()
            .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == family)
        {
            let raw = Key {
                owner: k.owner.clone(),
                family: k.family.clone(),
                encoded: k.encoded.clone(),
            };
            let n = k.encoded.parse::<u32>().map_err(bad)?;
            if k.encoded != n.to_string()
                || remaining.remove(&ObjId(n)) != Some(required(view, &raw)?)
            {
                return Err(refused("core refit reverse current head"));
            }
        }
        if !remaining.is_empty() {
            return Err(refused("core refit missing current head"));
        }
    }
    Ok(())
}
