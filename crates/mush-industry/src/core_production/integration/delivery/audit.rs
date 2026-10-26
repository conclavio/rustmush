use super::super::{
    IntegrationError as Error, bad,
    records::{CoreIntegrationOrder, IntegrationIndex, MAX_ORDERS, decode},
    runtime::Runtime,
};
use super::{INDEX_BYTES, INDEX_KEY, Index, Record};
use crate::workorder::WorkOrderId;
use mush_core::{ModuleBootData, ModuleCtx};
use std::{collections::BTreeMap, sync::Arc};
pub(crate) fn load(data: &ModuleBootData) -> Result<Option<Index>, Error> {
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
pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    runtime: &Runtime,
    deployment: Option<&IntegrationIndex>,
    loaded: Option<&Index>,
    orders: &BTreeMap<WorkOrderId, &CoreIntegrationOrder>,
) -> Result<BTreeMap<WorkOrderId, Arc<Record>>, Error> {
    let mut rows = BTreeMap::new();
    let mut sequence = BTreeMap::new();
    if let Some(delivery) = &runtime.delivery {
        let mut after = None;
        loop {
            let page = ctx
                .scan_action_state(&delivery.records, after.as_deref(), 128)
                .map_err(bad)?;
            for entry in page.entries() {
                let r = entry.value();
                r.matches_order(orders.get(&r.order).ok_or(Error::Integrity)?)?;
                if entry.encoded_key() != format!("{:016x}", r.order.0)
                    || rows.len() as u64 >= MAX_ORDERS
                    || rows.insert(r.order, Arc::new(r.clone())).is_some()
                    || sequence.insert(r.sequence, r).is_some()
                {
                    return Err(Error::Integrity);
                }
            }
            after = page.next_after().map(ToString::to_string);
            if after.is_none() {
                break;
            }
        }
    } else if loaded.is_some() || orders.values().any(|o| o.intent.policy.delivery.is_some()) {
        return Err(Error::Unavailable);
    }
    match (deployment, loaded) {
        (Some(birth), Some(index))
            if birth.policy.delivery.is_some() && runtime.delivery.is_some() =>
        {
            index.validate()?;
            if index.initialized() != birth.initialized {
                return Err(Error::Integrity);
            }
            let mut rebuilt = index.genesis()?;
            for row in sequence.values() {
                rebuilt = rebuilt.append(row)?;
            }
            if &rebuilt != index {
                return Err(Error::Integrity);
            }
        }
        (Some(birth), None)
            if birth.policy.delivery.is_none()
                && rows.is_empty()
                && orders.values().all(|o| o.intent.policy.delivery.is_none()) => {}
        (None, None) if rows.is_empty() => {}
        _ => return Err(Error::Integrity),
    }
    Ok(rows)
}
