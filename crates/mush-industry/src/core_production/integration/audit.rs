//! Mandatory full paired recovery, including genuinely empty inventories.
use super::{
    IntegrationError as Error, bad,
    records::*,
    runtime::Runtime,
    state::{OrderView, Projection},
};
use crate::core_production::{CoreOriginMode, participant::CoreProductionState};
use mush_core::{ModuleCtx, ReservationId, ReservationRecord, ReservationState};
use mush_economy::{EconomyStore, core_integration::CoreTerminalResult};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(crate) fn load_index(
    data: &mush_core::ModuleBootData,
) -> Result<Option<IntegrationIndex>, Error> {
    let mut rows = data.kv.iter().filter(|(k, _)| k.as_str() == INDEX_KEY);
    let first = rows
        .next()
        .map(|(_, v)| decode(v, INDEX_BYTES))
        .transpose()
        .map_err(bad)?;
    if rows.next().is_some() {
        return Err(Error::Integrity);
    }
    Ok(first)
}
pub(crate) fn audit(ctx: &ModuleCtx<'_>, runtime: &Runtime) -> Result<Projection, Error> {
    let f = &runtime.families;
    let loaded = ctx
        .w
        .module_state::<CoreProductionState>()
        .and_then(|s| s.integration.as_ref())
        .ok_or(Error::Unavailable)?;
    let typed = ctx.action_state(&f.deployment, &()).map_err(bad)?;
    let page = ctx.scan_action_state(&f.deployment, None, 2).map_err(bad)?;
    if page.next_after().is_some()
        || page.entries().len() != usize::from(typed.is_some())
        || page.entries().iter().any(|e| e.encoded_key() != "registry")
        || loaded.index.as_ref() != typed
    {
        return Err(Error::Integrity);
    }
    let mut offers = BTreeMap::new();
    let mut offer_order = BTreeMap::new();
    let mut after = None;
    loop {
        let page = ctx
            .scan_action_state(&f.offers, after.as_deref(), 128)
            .map_err(bad)?;
        for entry in page.entries() {
            let offer = entry.value();
            offer.validate()?;
            if entry.encoded_key() != offer.operation.to_hex()
                || offers
                    .insert(offer.operation, Arc::new(offer.clone()))
                    .is_some()
                || offer_order.insert(offer.sequence, offer).is_some()
                || offers.len() as u64 > MAX_ORDERS
            {
                return Err(Error::Integrity);
            }
            if let super::offer::OfferedInput::SupplierStock { certificate, lot } = &offer.input {
                certificate.matches_lot(lot).map_err(bad)?;
                if ctx
                    .action_state(&runtime.origins.certificates, &certificate.key())
                    .map_err(bad)?
                    != Some(certificate.as_ref())
                {
                    return Err(Error::Integrity);
                }
            }
        }
        after = page.next_after().map(ToString::to_string);
        if after.is_none() {
            break;
        }
    }
    let economy = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or(Error::Unavailable)?;
    let payments = economy
        .core_payment_histories()
        .map_err(bad)?
        .map(|h| (h.funding().commit(), h))
        .collect::<BTreeMap<_, _>>();
    let inputs = runtime
        .input_handle
        .operations_for_boot(ctx)
        .map_err(bad)?
        .into_iter()
        .map(|i| (crate::workorder::WorkOrderId(i.identity().order()), i))
        .collect::<BTreeMap<_, _>>();
    let mut orders = BTreeMap::new();
    let mut order_sequence = BTreeMap::new();
    let mut accepted = BTreeSet::new();
    let mut used_offers = BTreeSet::new();
    let mut used_slots = BTreeSet::new();
    let mut schedule_orders = BTreeMap::new();
    let mut after = None;
    loop {
        let page = ctx
            .scan_action_state(&f.orders, after.as_deref(), 128)
            .map_err(bad)?;
        for entry in page.entries() {
            let row = entry.value();
            row.validate()?;
            if entry.encoded_key() != format!("{:016x}", row.intent.order.0)
                || orders.insert(row.intent.order, row).is_some()
                || order_sequence.insert(row.sequence, row).is_some()
                || !accepted.insert(row.accepted)
                || !used_offers.insert(row.intent.offer)
                || !used_slots.insert(row.capacity)
                || orders.len() as u64 > MAX_ORDERS
            {
                return Err(Error::Integrity);
            }
            if ctx
                .action_state(&runtime.industry.work_orders, &row.intent.order)
                .map_err(bad)?
                .is_some()
            {
                return Err(Error::Integrity);
            }
            let offer = offers.get(&row.intent.offer).ok_or(Error::Integrity)?;
            row.intent.matches_offer(offer)?;
            // The die's retained proof: the certificate, or the staff issue of a heirloom or
            // grey die (doc 17h cut M8b-2).
            let retained = match (&row.intent.certificate, &row.intent.uncertified) {
                (Some(c), None) => {
                    ctx.action_state(&runtime.origins.certificates, &c.key())
                        .map_err(bad)?
                        == Some(c.as_ref())
                }
                (None, Some(u)) => {
                    ctx.action_state(&runtime.origins.uncertified.records, &u.key())
                        .map_err(bad)?
                        == Some(u.as_ref())
                }
                _ => false,
            };
            if !retained {
                return Err(Error::Integrity);
            }
            row.verify_payment(payments.get(&row.accepted).ok_or(Error::Integrity)?)?;
            row.verify_input(inputs.get(&row.intent.order).ok_or(Error::Integrity)?)?;
            capacity(ctx, runtime, row)?;
            let id = super::progress::schedule_id(row.intent.order)?;
            let cursor = ctx.action_state(&f.schedules, &id).map_err(bad)?;
            let job = ctx.action_state(&f.jobs, &id).map_err(bad)?;
            if row.terminal.is_some() {
                if cursor.is_some() || job.is_some() {
                    return Err(Error::Integrity);
                }
            } else {
                // A complete progress cursor remains retained until the explicit terminal
                // producer. The driver refuses further occurrences without granting output.
                if cursor != Some(row.progress.cursor()) || job != Some(&row.intent.order) {
                    return Err(Error::Integrity);
                }
                schedule_orders.insert(id, row.intent.order);
            }
        }
        after = page.next_after().map(ToString::to_string);
        if after.is_none() {
            break;
        }
    }
    if inputs.len() != orders.len() || payments.len() != orders.len() {
        return Err(Error::Integrity);
    }
    let deliveries = super::delivery::audit::audit(
        ctx,
        runtime,
        typed,
        loaded.delivery_index.as_ref(),
        &orders,
    )?;
    match typed {
        None => {
            if let Some(d) = &runtime.delivery {
                d.outputs.require_no_deliveries(ctx).map_err(bad)?;
            }
            if runtime.config.mode != CoreOriginMode::Initialize
                || !offers.is_empty()
                || !orders.is_empty()
                || !runtime.outputs.empty_for_initialization(ctx).map_err(bad)?
            {
                return Err(Error::Integrity);
            }
        }
        Some(index) => {
            index.validate()?;
            let mut expected = IntegrationIndex::new(index.initialized, index.policy.clone())?;
            for offer in offer_order.values() {
                expected = expected.offer(offer)?;
            }
            for order in order_sequence.values() {
                expected = expected.accept(order)?;
            }
            if &expected != index {
                return Err(Error::Integrity);
            }
            runtime
                .outputs
                .verify_deployment(ctx, index.initialized)
                .map_err(bad)?;
            let mut seen = BTreeSet::new();
            let mut output_visitor = |h: mush_core::core_output::CoreOutputHistory| {
                let fail = || {
                    mush_core::participant::ParticipantError::refused(
                        "Core output/order recovery differs",
                    )
                };
                let row = orders
                    .get(&crate::workorder::WorkOrderId(h.order().id()))
                    .ok_or_else(fail)?;
                let output = row.output.as_ref().ok_or_else(fail)?;
                output
                    .matches_history(&h, &row.intent)
                    .map_err(|_| fail())?;
                let terminal = row.terminal.as_ref().and_then(|t| {
                    matches!(t.result(), CoreTerminalResult::Completed { .. })
                        .then_some((t.commit(), t.action()))
                });
                if h.completion() != terminal || !seen.insert(row.intent.order) {
                    return Err(fail());
                }
                Ok(())
            };
            if let Some(d) = &runtime.delivery {
                let mut delivered_seen = BTreeSet::new();
                d.outputs
                    .audit(ctx, &mut output_visitor, |h| {
                        let output = h.receipt();
                        let id = crate::workorder::WorkOrderId(output.order().id());
                        let fail = || {
                            mush_core::participant::ParticipantError::refused(
                                "Core delivery counterparts disagree",
                            )
                        };
                        let row = deliveries.get(&id).ok_or_else(fail)?;
                        row.matches_core(output, orders.get(&id).ok_or_else(fail)?)
                            .map_err(|_| fail())?;
                        if !delivered_seen.insert(id) {
                            return Err(fail());
                        }
                        Ok(())
                    })
                    .map_err(bad)?;
                if delivered_seen.len() != deliveries.len() {
                    return Err(Error::Integrity);
                }
            } else {
                runtime
                    .outputs
                    .audit(ctx, &mut output_visitor)
                    .map_err(bad)?;
            }
            if seen.len() != orders.values().filter(|o| o.output.is_some()).count() {
                return Err(Error::Integrity);
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut after = None;
    loop {
        let page = ctx
            .scan_action_state(&f.schedules, after.as_deref(), 128)
            .map_err(bad)?;
        for e in page.entries() {
            let schedule = e.value();
            if e.encoded_key() != schedule.id().canonical_hash().to_hex()
                || !schedule_orders.contains_key(&schedule.id())
                || !seen.insert(schedule.id())
            {
                return Err(Error::Integrity);
            }
        }
        after = page.next_after().map(ToString::to_string);
        if after.is_none() {
            break;
        }
    }
    if seen.len() != schedule_orders.len() {
        return Err(Error::Integrity);
    }
    let mut seen = BTreeSet::new();
    let mut after = None;
    loop {
        let page = ctx
            .scan_action_state(&f.jobs, after.as_deref(), 128)
            .map_err(bad)?;
        for e in page.entries() {
            let id = super::progress::schedule_id(*e.value())?;
            if e.encoded_key() != id.canonical_hash().to_hex()
                || schedule_orders.get(&id) != Some(e.value())
                || !seen.insert(id)
            {
                return Err(Error::Integrity);
            }
        }
        after = page.next_after().map(ToString::to_string);
        if after.is_none() {
            break;
        }
    }
    if seen.len() != schedule_orders.len() {
        return Err(Error::Integrity);
    }
    Ok(Projection {
        ready: true,
        index: typed.cloned(),
        delivery_index: loaded.delivery_index.clone(),
        deliveries,
        offers,
        orders: orders
            .into_iter()
            .map(|(id, row)| (id, Arc::new(OrderView::capture(row))))
            .collect(),
    })
}
fn capacity(
    ctx: &ModuleCtx<'_>,
    runtime: &Runtime,
    row: &CoreIntegrationOrder,
) -> Result<(), Error> {
    let resource = crate::facility::facility_resource(row.intent.site_factory()?).map_err(bad)?;
    let holder = crate::workorder::wo_holder(row.intent.order).map_err(bad)?;
    let id = ReservationId::derive(&holder, &resource).map_err(bad)?;
    if row.capacity != id {
        return Err(Error::Integrity);
    }
    let initial = ReservationRecord::held(id, resource, holder, 1, row.intent.expires_at.clone())
        .map_err(bad)?;
    let key = crate::facility::facility_capacity_key(row.intent.site_factory()?).map_err(bad)?;
    let book = ctx
        .action_state(&runtime.industry.facility_books, &key)
        .map_err(bad)?;
    let actual = book.and_then(|b| b.records().iter().find(|r| r.id() == id));
    if row.terminal.is_none() {
        if actual != Some(&initial)
            || ctx
                .action_state(&runtime.industry.capacity_index, &id)
                .map_err(bad)?
                != Some(&key)
        {
            return Err(Error::Integrity);
        }
    } else if let Some(actual) = actual {
        // Native K may retire older terminal slot records. Any surviving counterpart
        // must still be the exact acknowledged release, never an expired replacement.
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
