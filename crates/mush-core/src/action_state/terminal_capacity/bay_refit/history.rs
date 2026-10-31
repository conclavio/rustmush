//! The bay order's counterparts: the hull held in the bay, the schedule, the
//! slot and Inventory's part custody, retained and terminal.
use super::wire::Outcome;
use super::*;
/// The hull's bay head: held for the order from Accept, released at its
/// terminal.
pub(super) fn hull(r: &Row, terminal: bool) -> Result<Value, ActionStateError> {
    let operation = if terminal {
        r.terminal_commit()?
    } else {
        json!(r.accepted)
    };
    Ok(
        json!({"format":1,"hull":r.hull()?,"operation":operation,"pending":if terminal{Value::Null}else{json!(r.order)}}),
    )
}
/// Inventory's custody of the part fitted: held from Accept, consumed at
/// Install, released at Cancel.
pub(super) fn part(r: &Row, lot: u64) -> Result<Value, ActionStateError> {
    let (state, terminal) = match r.outcome()? {
        None => ("held", Value::Null),
        Some(Outcome::Installed) => ("consumed", r.terminal_commit()?),
        Some(Outcome::Cancelled) => ("released", r.terminal_commit()?),
    };
    Ok(
        json!({"format":1,"order":r.order,"lot":lot,"stores":r.stores()?,"accepted":r.accepted,"state":state,"terminal":terminal}),
    )
}
pub(super) fn paired(view: &View<'_>, r: &Row, historical: bool) -> Result<(), ActionStateError> {
    if let Some(lot) = r.part()?
        && required(view, &r.key(PART)?)? != part(r, lot)?
    {
        return Err(refused("bay refit part custody"));
    }
    match r.outcome()? {
        None => {
            if required(view, &r.key(HULL)?)? != hull(r, false)?
                || required(view, &r.key(SCHEDULE)?)?
                    != *field(field(&r.raw, "progress")?, "cursor")?
                || required(view, &r.key(JOB)?)? != json!(r.order)
            {
                return Err(refused("bay refit retained hull/schedule"));
            }
            if let Some(k) = r.part_lot()?
                && value(view, &k)?.is_none()
            {
                return Err(refused("bay refit held part lot missing"));
            }
            if let Some(k) = r.returned_lot()?
                && value(view, &k)?.is_some()
            {
                return Err(refused("bay refit returned lot early"));
            }
            let scalar: BoundedScalar = parsed(&required(view, &r.key(SLOTS)?)?)?;
            let book: ReservationBook = parsed(&required(view, &r.key(BOOK)?)?)?;
            book.checked_available(scalar).map_err(bad)?;
            let id: crate::ReservationId = parsed(field(&r.raw, "capacity")?)?;
            let held = book
                .records()
                .iter()
                .find(|x| x.id() == id)
                .ok_or_else(|| refused("bay refit actual slot missing"))?;
            if book.resource() != &r.resource()?
                || held.holder() != &r.holder()?
                || held.state() != crate::ReservationState::Held
                || held.amount() != 1
                || held.expires() != &parsed::<ClockStamp>(field(&r.intent, "expires_at")?)?
            {
                return Err(refused("bay refit exact native slot"));
            }
        }
        Some(outcome) => {
            if let Some(i) = r.installation()?
                && !historical
                && field(i, "participants")?
                    != &json!(
                        view.registry
                            .bay_refit
                            .as_ref()
                            .ok_or_else(|| refused("bay refit protocol"))?
                            .terminal_participants()
                    )
            {
                return Err(refused("bay refit actual terminal owners"));
            }
            // A later order may take the hull or a lot this one freed, so
            // the current images are joined only at the terminal itself.
            if !historical {
                if value(view, &r.key(SCHEDULE)?)?.is_some()
                    || value(view, &r.key(JOB)?)?.is_some()
                    || required(view, &r.key(HULL)?)? != hull(r, true)?
                {
                    return Err(refused("bay refit terminal hull/schedule"));
                }
                let installed = outcome == Outcome::Installed;
                if let Some(k) = r.part_lot()?
                    && value(view, &k)?.is_some() == installed
                {
                    return Err(refused("bay refit part lot terminal"));
                }
                if let Some(k) = r.returned_lot()?
                    && value(view, &k)?.is_some() != installed
                {
                    return Err(refused("bay refit returned lot terminal"));
                }
            }
        }
    }
    Ok(())
}
/// Reconstruct the current hull heads from the immutable payment ordering.
/// A later order may replace a head; it never rewrites an earlier row.
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
            return Err(refused("bay refit duplicate journal identity"));
        }
        if let Some(t) = r.terminal()?
            && events
                .insert(number(t, "journal_seq")?, (*id, true))
                .is_some()
        {
            return Err(refused("bay refit duplicate terminal identity"));
        }
    }
    let mut hulls = BTreeMap::<ObjId, Value>::new();
    for (id, terminal) in events.values() {
        let r = rows
            .get(id)
            .ok_or_else(|| refused("bay refit history row"))?;
        let unit = r.hull()?;
        if !terminal {
            if hulls
                .get(&unit)
                .is_some_and(|h| h.get("pending") != Some(&Value::Null))
            {
                return Err(refused("bay refit historical hull already in a bay"));
            }
        } else if hulls.get(&unit) != Some(&hull(r, false)?) {
            return Err(refused("bay refit historical terminal preimage"));
        }
        hulls.insert(unit, hull(r, *terminal)?);
    }
    let mut remaining = hulls;
    for k in view
        .records
        .keys()
        .filter(|k| k.owner.as_str() == "industry" && k.family.as_str() == HULL)
    {
        let raw = Key {
            owner: k.owner.clone(),
            family: k.family.clone(),
            encoded: k.encoded.clone(),
        };
        let n = k.encoded.parse::<u32>().map_err(bad)?;
        if k.encoded != n.to_string() || remaining.remove(&ObjId(n)) != Some(required(view, &raw)?)
        {
            return Err(refused("bay refit reverse current head"));
        }
    }
    if !remaining.is_empty() {
        return Err(refused("bay refit missing current head"));
    }
    Ok(())
}
