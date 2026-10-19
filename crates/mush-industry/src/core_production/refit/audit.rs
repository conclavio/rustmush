//! Complete independent index, native journal, physical receipt and current-head
//! reconstruction. Historical rows do not require the old supplier or recipe.
use super::{Error, bad, records::*, runtime::Runtime, state::Projection};
use crate::core_production::{CoreOriginMode, participant::CoreProductionState};
use crate::workorder::WorkOrderId;
use mush_core::{
    ModuleCtx, ObjId, ReservationId, ReservationRecord, ReservationState, StateFamilyHandle,
};
use mush_economy::EconomyStore;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub(crate) fn load_index(data: &mush_core::ModuleBootData) -> Result<Option<RefitIndex>, Error> {
    let mut rows = data.kv.iter().filter(|(k, _)| k.as_str() == INDEX_KEY);
    let row = rows
        .next()
        .map(|(_, v)| decode(v, INDEX_BYTES))
        .transpose()
        .map_err(bad)?;
    if rows.next().is_some() {
        return Err(Error::Integrity);
    }
    Ok(row)
}
fn scan<K: Send + Sync + 'static, V: Clone + Send + Sync + 'static>(
    ctx: &ModuleCtx<'_>,
    f: &StateFamilyHandle<K, V>,
    mut visit: impl FnMut(&str, &V) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut cursor = None;
    let mut count = 0usize;
    loop {
        let page = ctx
            .scan_action_state(f, cursor.as_deref(), 128)
            .map_err(bad)?;
        for e in page.entries() {
            count = count.checked_add(1).ok_or(Error::Capacity)?;
            if count > 2 * MAX_ORDERS as usize {
                return Err(Error::Capacity);
            }
            visit(e.encoded_key(), e.value())?;
        }
        cursor = page.next_after().map(ToString::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok(())
}
pub(crate) fn audit(ctx: &ModuleCtx<'_>, r: &Runtime) -> Result<Projection, Error> {
    let f = &r.families;
    let loaded = ctx
        .w
        .module_state::<CoreProductionState>()
        .and_then(|s| s.refit.as_ref())
        .ok_or(Error::Unavailable)?;
    let index = ctx.action_state(&f.deployment, &()).map_err(bad)?;
    let mut markers = 0;
    scan(ctx, &f.deployment, |key, v| {
        markers += 1;
        if key != "registry" || Some(v) != index {
            return Err(Error::Integrity);
        }
        Ok(())
    })?;
    if markers != usize::from(index.is_some()) || loaded.index.as_ref() != index {
        return Err(Error::Integrity);
    }
    let mut offers = BTreeMap::new();
    let mut offer_seq = BTreeMap::new();
    scan(ctx, &f.offers, |key, o| {
        o.validate()?;
        if key != o.operation.to_hex()
            || offers.insert(o.operation, Arc::new(o.clone())).is_some()
            || offer_seq.insert(o.sequence, o.clone()).is_some()
            || offers.len() > MAX_ORDERS as usize
        {
            return Err(Error::Integrity);
        }
        Ok(())
    })?;
    let economy = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or(Error::Unavailable)?;
    let payments = economy
        .core_refit_histories()
        .map_err(bad)?
        .map(|h| (h.funding().commit(), h))
        .collect::<BTreeMap<_, _>>();
    let mut orders = BTreeMap::new();
    let mut order_seq = BTreeMap::new();
    let mut accepted = BTreeSet::new();
    let mut used_offers = BTreeSet::new();
    let mut used_slots = BTreeSet::new();
    let mut schedules = BTreeMap::new();
    let mut events = BTreeMap::new();
    scan(ctx, &f.orders, |key, o| {
        o.validate()?;
        let id = o.intent.order;
        if key != format!("{:016x}", id.0)
            || orders.insert(id, Arc::new(o.clone())).is_some()
            || order_seq.insert(o.sequence, o.clone()).is_some()
            || !accepted.insert(o.accepted)
            || !used_offers.insert(o.intent.offer.operation)
            || !used_slots.insert(o.capacity)
            || orders.len() > MAX_ORDERS as usize
            || offers
                .get(&o.intent.offer.operation)
                .is_none_or(|v| v.as_ref() != &o.intent.offer)
            || ctx
                .action_state(&r.industry.work_orders, &id)
                .map_err(bad)?
                .is_some()
            || ctx
                .action_state(&r.manufacture.orders, &id)
                .map_err(bad)?
                .is_some()
        {
            return Err(Error::Integrity);
        }
        o.verify_payment(payments.get(&o.accepted).ok_or(Error::Integrity)?)?;
        capacity(ctx, r, o)?;
        if events
            .insert(o.funding.journal_seq(), (id, false))
            .is_some()
        {
            return Err(Error::Integrity);
        }
        if let Some(t) = &o.terminal {
            if events.insert(t.journal_seq(), (id, true)).is_some() {
                return Err(Error::Integrity);
            }
        } else {
            schedules.insert(super::progress::schedule_id(id)?, id);
        }
        Ok(())
    })?;
    if payments.len() != orders.len() {
        return Err(Error::Integrity);
    }
    let mut seen = BTreeSet::new();
    r.exchanges
        .audit(ctx, |physical| {
            let error = || {
                mush_core::participant::ParticipantError::refused("Refit physical history differs")
            };
            let id = WorkOrderId(physical.order());
            let row = orders.get(&id).ok_or_else(error)?;
            row.matches_core(&physical).map_err(|_| error())?;
            if !seen.insert(id) {
                return Err(error());
            }
            Ok(())
        })
        .map_err(bad)?;
    if seen.len() != orders.values().filter(|o| o.installation.is_some()).count() {
        return Err(Error::Integrity);
    }
    match index {
        None => {
            if r.config.mode != CoreOriginMode::Initialize
                || !offers.is_empty()
                || !orders.is_empty()
            {
                return Err(Error::Integrity);
            }
        }
        Some(index) => {
            let mut expected = index.genesis()?;
            for o in offer_seq.values() {
                expected = expected.offer(o)?;
            }
            for row in order_seq.values() {
                let mut birth = row.clone();
                birth.terminal = None;
                birth.installation = None;
                birth.progress = super::progress::RefitProgress::new(
                    row.intent.order,
                    row.intent.accepted_at.clone(),
                    row.intent.offer.recipe.work_units(),
                )?;
                expected = expected.accept(&birth)?;
            }
            if &expected != index {
                return Err(Error::Integrity);
            }
        }
    }
    let mut assets = BTreeMap::<ObjId, AssetHead>::new();
    let mut hulls = BTreeMap::<ObjId, HullHead>::new();
    for (id, terminal) in events.values() {
        let row = orders.get(id).ok_or(Error::Integrity)?;
        let o = &row.intent.offer;
        if !terminal {
            if assets
                .get(&o.incoming)
                .is_some_and(|a| !matches!(a.position, AssetPosition::Released { .. }))
            {
                return Err(Error::Integrity);
            }
            match (hulls.get(&o.hull), o.displaced.installed.asset) {
                (None, None) => {}
                (Some(h), old) if h.pending.is_none() && h.mounted == old => {}
                _ => return Err(Error::Integrity),
            }
            if let Some(old) = o.displaced.installed.asset {
                if assets.get(&old).is_none_or(
                    |a| !matches!(a.position,AssetPosition::Mounted{hull,..}if hull==o.hull),
                ) {
                    return Err(Error::Integrity);
                }
            }
            assets.insert(
                o.incoming,
                AssetHead {
                    format: 1,
                    object: o.incoming,
                    operation: row.accepted,
                    position: AssetPosition::Pending { order: *id },
                },
            );
            hulls.insert(
                o.hull,
                HullHead {
                    format: 1,
                    hull: o.hull,
                    operation: row.accepted,
                    pending: Some(*id),
                    mounted: o.displaced.installed.asset,
                },
            );
        } else {
            let t = row.terminal.as_ref().ok_or(Error::Integrity)?;
            let a = assets.get(&o.incoming).ok_or(Error::Integrity)?;
            let h = hulls.get(&o.hull).ok_or(Error::Integrity)?;
            if a.position != (AssetPosition::Pending { order: *id })
                || h.pending != Some(*id)
                || h.mounted != o.displaced.installed.asset
            {
                return Err(Error::Integrity);
            }
            let installed = row.installation.is_some();
            assets.insert(
                o.incoming,
                AssetHead {
                    format: 1,
                    object: o.incoming,
                    operation: t.commit(),
                    position: if installed {
                        AssetPosition::Mounted {
                            order: *id,
                            hull: o.hull,
                        }
                    } else {
                        AssetPosition::Released { order: *id }
                    },
                },
            );
            hulls.insert(
                o.hull,
                HullHead {
                    format: 1,
                    hull: o.hull,
                    operation: t.commit(),
                    pending: None,
                    mounted: if installed {
                        Some(o.incoming)
                    } else {
                        o.displaced.installed.asset
                    },
                },
            );
            if installed && let Some(old) = o.displaced.installed.asset {
                if assets.get(&old).is_none_or(
                    |a| !matches!(a.position,AssetPosition::Mounted{hull,..}if hull==o.hull),
                ) {
                    return Err(Error::Integrity);
                }
                assets.insert(
                    old,
                    AssetHead {
                        format: 1,
                        object: old,
                        operation: t.commit(),
                        position: AssetPosition::Detached {
                            order: *id,
                            from_hull: o.hull,
                            condition: digest(
                                "industry.core_refit.displaced.v1",
                                &o.displaced,
                                ASSET_BYTES,
                            )?,
                        },
                    },
                );
            }
        }
    }
    let mut counted = BTreeSet::new();
    scan(ctx, &f.assets, |key, a| {
        a.validate()?;
        if key != a.object.0.to_string()
            || assets.get(&a.object) != Some(a)
            || !counted.insert(a.object)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    })?;
    if counted.len() != assets.len() {
        return Err(Error::Integrity);
    }
    let mut counted = BTreeSet::new();
    scan(ctx, &f.hulls, |key, h| {
        h.validate()?;
        if key != h.hull.0.to_string() || hulls.get(&h.hull) != Some(h) || !counted.insert(h.hull) {
            return Err(Error::Integrity);
        }
        Ok(())
    })?;
    if counted.len() != hulls.len() {
        return Err(Error::Integrity);
    }
    let mut counted = BTreeSet::new();
    scan(ctx, &f.schedules, |key, c| {
        let id = c.id();
        let order = schedules.get(&id).ok_or(Error::Integrity)?;
        if key != id.canonical_hash().to_hex()
            || orders.get(order).is_none_or(|o| o.progress.cursor() != c)
            || !counted.insert(id)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    })?;
    if counted.len() != schedules.len() {
        return Err(Error::Integrity);
    }
    let mut counted = BTreeSet::new();
    scan(ctx, &f.jobs, |key, id| {
        let schedule = super::progress::schedule_id(*id)?;
        if key != schedule.canonical_hash().to_hex()
            || schedules.get(&schedule) != Some(id)
            || !counted.insert(schedule)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    })?;
    if counted.len() != schedules.len() {
        return Err(Error::Integrity);
    }
    // This observes retained actual records, including a mechanically frozen root.
    // Damage after installation is permitted; a missing or crossed mounted asset
    // is not repaired into permission. No current recipe/eligibility is inferred.
    if !hulls.is_empty() {
        let tactics = ctx
            .w
            .module_state::<mush_tactics::UnitStore>()
            .ok_or(Error::Unavailable)?;
        for h in hulls.values() {
            if let Some(asset) = h.mounted {
                let matches = tactics
                    .records
                    .iter()
                    .filter(|u| u.key_object == Some(h.hull) || u.object == Some(h.hull))
                    .collect::<Vec<_>>();
                let [record] = matches.as_slice() else {
                    return Err(Error::Integrity);
                };
                if record.key_object != Some(h.hull) || record.object != Some(h.hull) {
                    return Err(Error::Integrity);
                }
                let unit = record.unit.as_ref().ok_or(Error::Integrity)?;
                let row = assets.get(&asset).ok_or(Error::Integrity)?;
                let AssetPosition::Mounted { order, hull } = &row.position else {
                    return Err(Error::Integrity);
                };
                let installed = orders.get(order).ok_or(Error::Integrity)?;
                if *hull != h.hull
                    || SavedComponent::from_unit(unit, &installed.intent.offer.component)?
                        .installed
                        .asset
                        != Some(asset)
                    || ctx.w.get(asset).is_none_or(|a| a.location != Some(h.hull))
                {
                    return Err(Error::Integrity);
                }
            }
        }
    }
    Ok(Projection {
        ready: true,
        index: index.cloned(),
        offers,
        orders,
    })
}
fn capacity(ctx: &ModuleCtx<'_>, r: &Runtime, row: &RefitOrder) -> Result<(), Error> {
    let resource =
        crate::facility::facility_resource(row.intent.offer.policy.factory).map_err(bad)?;
    let holder = crate::workorder::wo_holder(row.intent.order).map_err(bad)?;
    let id = ReservationId::derive(&holder, &resource).map_err(bad)?;
    if row.capacity != id {
        return Err(Error::Integrity);
    }
    let initial = ReservationRecord::held(id, resource, holder, 1, row.intent.expires_at.clone())
        .map_err(bad)?;
    let key =
        crate::facility::facility_capacity_key(row.intent.offer.policy.factory).map_err(bad)?;
    let book = ctx
        .action_state(&r.industry.facility_books, &key)
        .map_err(bad)?;
    let actual = book.and_then(|b| b.records().iter().find(|x| x.id() == id));
    if row.terminal.is_none() {
        if actual != Some(&initial)
            || ctx
                .action_state(&r.industry.capacity_index, &id)
                .map_err(bad)?
                != Some(&key)
        {
            return Err(Error::Integrity);
        }
    } else if let Some(actual) = actual {
        if actual.resource() != initial.resource()
            || actual.holder() != initial.holder()
            || actual.amount() != 1
            || actual.expires() != initial.expires()
            || actual.revision() != 2
            || actual.state() != ReservationState::Released
        {
            return Err(Error::Integrity);
        }
    }
    Ok(())
}
