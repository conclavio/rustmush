//! The boot reconstruction of the bay owner's projection from its typed rows,
//! its census and Economy's complete fee history. The kernel's own boot audit
//! independently joins every order to its slot, hull head, schedule and part.
use super::BayRefitError as Error;
use super::records::*;
use super::runtime::BayRuntime;
use super::state::BayRefitState;
use mush_core::{ModuleCtx, StateFamilyHandle};
use mush_economy::EconomyStore;
use std::collections::BTreeMap;
use std::sync::Arc;

fn bad<T>(_: T) -> Error {
    Error::Integrity
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
pub(crate) fn audit(ctx: &ModuleCtx<'_>, r: &BayRuntime) -> Result<BayRefitState, Error> {
    let f = &r.families;
    let loaded = ctx
        .w
        .module_state::<BayRefitState>()
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
    scan(ctx, &f.offers, |key, o| {
        o.validate()?;
        if key != o.operation.to_hex() || offers.insert(o.operation, Arc::new(o.clone())).is_some()
        {
            return Err(Error::Integrity);
        }
        Ok(())
    })?;
    let economy = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or(Error::Unavailable)?;
    let mut orders = BTreeMap::new();
    scan(ctx, &f.orders, |key, o| {
        o.validate()?;
        let history = economy.bay_refit_history(o.accepted).map_err(bad)?;
        o.verify_payment(history, &r.config.payment)?;
        if key != format!("{:016x}", o.intent.order.0)
            || offers.get(&o.intent.offer.operation).map(AsRef::as_ref) != Some(&o.intent.offer)
            || orders.insert(o.intent.order, Arc::new(o.clone())).is_some()
        {
            return Err(Error::Integrity);
        }
        Ok(())
    })?;
    let index_counts = index.map_or((0, 0), |i| (i.offers, i.accepted));
    if index_counts != (offers.len() as u64, orders.len() as u64)
        || economy.bay_refit_histories().map_err(bad)?.len() != orders.len()
    {
        return Err(Error::Integrity);
    }
    Ok(BayRefitState {
        offers,
        orders,
        ..BayRefitState::default()
    })
}
