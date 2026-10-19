use super::*;
use super::{participant::CoreProductionState, provider::Runtime, records::*};
use mush_core::ModuleCtx;
use mush_inventory::Lineage;
use std::collections::{BTreeMap, BTreeSet};
pub(crate) fn audit(ctx: &ModuleCtx<'_>, runtime: &Runtime) -> Result<(), Error> {
    let f = &runtime.families;
    let state = ctx
        .w
        .module_state::<CoreProductionState>()
        .ok_or(Error::Unavailable)?;
    let typed = ctx.action_state(&f.deployment, &()).map_err(bad)?;
    let marker = ctx.scan_action_state(&f.deployment, None, 2).map_err(bad)?;
    if marker.next_after().is_some()
        || marker.entries().len() != usize::from(typed.is_some())
        || marker
            .entries()
            .iter()
            .any(|e| e.encoded_key() != "registry")
    {
        return Err(Error::Integrity);
    }
    let mut certs = BTreeMap::new();
    let mut lots = BTreeMap::new();
    let mut after = None;
    loop {
        let page = ctx
            .scan_action_state(&f.certificates, after.as_deref(), 128)
            .map_err(bad)?;
        for entry in page.entries() {
            let c = entry.value();
            c.validate()?;
            runtime.definitions.historical(&c.policy)?;
            if entry.encoded_key() != c.key()
                || certs.insert(c.sequence, c).is_some()
                || lots.insert(c.lot, c).is_some()
                || certs.len() as u64 > MAX_CERTIFICATES
            {
                return Err(Error::Integrity);
            }
            if let Some(lot) = ctx.action_state(&f.inventory.lots, &c.lot).map_err(bad)? {
                c.matches_lot(lot)?;
            }
        }
        after = page.next_after().map(|cursor| cursor.to_string());
        if after.is_none() {
            break;
        }
    }
    let mut expected_quotas = BTreeMap::<String, Quota>::new();
    match (&state.index, typed) {
        (None, None) => {
            if runtime.config.mode != CoreOriginMode::Initialize || !certs.is_empty() {
                return Err(Error::Integrity);
            }
        }
        (Some(index), Some(typed)) if index == typed => {
            index.validate()?;
            runtime.definitions.historical(&index.initialize_policy)?;
            if certs.len() as u64 != index.issued {
                return Err(Error::Integrity);
            }
            let mut expected = OriginIndex::new(&index.initialize_policy, index.initialized_by)?;
            for c in certs.values() {
                expected = expected.issue(c)?;
                let key = c.quota_key()?;
                let encoded = key.key()?;
                let q = Quota::issue(
                    key,
                    c.policy.monthly_quotas[c.class.index()],
                    expected_quotas.get(&encoded),
                    c.operation,
                )?;
                if q.issued != c.ordinal || q.maximum != c.quota_maximum {
                    return Err(Error::Integrity);
                }
                expected_quotas.insert(encoded, q);
            }
            if &expected != index {
                return Err(Error::Integrity);
            }
        }
        _ => return Err(Error::Integrity),
    }
    let mut seen = BTreeSet::new();
    let mut after = None;
    loop {
        let page = ctx
            .scan_action_state(&f.quotas, after.as_deref(), 128)
            .map_err(bad)?;
        for entry in page.entries() {
            let q = entry.value();
            q.validate()?;
            if q.key.key() != Ok(entry.encoded_key().to_owned())
                || expected_quotas.get(entry.encoded_key()) != Some(q)
                || !seen.insert(entry.encoded_key().to_owned())
            {
                return Err(Error::Integrity);
            }
        }
        after = page.next_after().map(|cursor| cursor.to_string());
        if after.is_none() {
            break;
        }
    }
    if seen.len() != expected_quotas.len() {
        return Err(Error::Integrity);
    }
    // Inventory owns retirement and may legitimately remove an Origin by merge or
    // dissolution. Historical certification never resurrects such a lot. Every
    // present claim to this source namespace still requires the exact real receipt.
    let mut after = None;
    let mut visited = 0usize;
    loop {
        let page = ctx
            .scan_action_state(&f.inventory.lots, after.as_deref(), 128)
            .map_err(bad)?;
        for entry in page.entries() {
            visited = visited.checked_add(1).ok_or(Error::Capacity)?;
            if visited > mush_core::action_state::MAX_ACTION_STATE_BOOT_RECORDS {
                return Err(Error::Capacity);
            }
            let lot = entry.value();
            if let Lineage::Origin { source } = lot.lineage() {
                if source.provider.as_str() == crate::NS
                    && source.kind.starts_with("certified_core_die")
                {
                    let c = lots.get(&lot.id()).ok_or(Error::Integrity)?;
                    c.matches_lot(lot)?;
                }
            }
        }
        after = page.next_after().map(|cursor| cursor.to_string());
        if after.is_none() {
            break;
        }
    }
    Ok(())
}
