use super::records::{Census, Head};
use super::*;
use crate::LotId;
use mush_core::ModuleCtx;
use std::collections::BTreeMap;

impl CoreInputHandle {
    /// Mandatory paired-owner boot seam. All retained rows are reconstructed,
    /// including an empty inventory. Industry must independently join its order
    /// index in both directions; this local census cannot prove its own absence.
    pub fn operations_for_boot(
        &self,
        ctx: &ModuleCtx<'_>,
    ) -> Result<Vec<CoreInputOperation>, Error> {
        if ctx
            .w
            .module_state::<crate::InventoryStore>()
            .is_none_or(|s| !s.is_complete())
        {
            return Err(Error::Unavailable);
        }
        self.audit(ctx)
    }
    pub(super) fn audit(&self, ctx: &ModuleCtx<'_>) -> Result<Vec<CoreInputOperation>, Error> {
        let f = &self.shared;
        let mut by_sequence = BTreeMap::new();
        let mut by_order = BTreeMap::new();
        let mut after = None;
        loop {
            let page = ctx
                .scan_action_state(&f.operations, after.as_deref(), 128)
                .map_err(bad)?;
            for entry in page.entries() {
                let op = entry.value();
                op.validate()?;
                if entry.encoded_key() != registration::key(op.identity.order())
                    || by_sequence.insert(op.sequence, op.clone()).is_some()
                    || by_order.insert(op.identity.order(), op.clone()).is_some()
                    || by_order.len() as u64 > MAX_OPERATIONS
                {
                    return Err(Error::Integrity);
                }
            }
            after = page.next_after().cloned();
            if after.is_none() {
                break;
            }
        }
        let mut expected = Census::empty()?;
        let mut heads = BTreeMap::<LotId, Head>::new();
        for op in by_sequence.values() {
            expected = expected.push(op)?;
            match heads.get(&op.lot.id()) {
                None if op.previous.is_none() && op.attempt == 1 => {}
                Some(prior)
                    if op.previous == Some(prior.identity.order())
                        && op.attempt == prior.attempt.checked_add(1).ok_or(Error::Capacity)? =>
                {
                    let previous = by_order
                        .get(&prior.identity.order())
                        .ok_or(Error::Integrity)?;
                    if previous.state()? != CoreInputState::Released {
                        return Err(Error::Integrity);
                    }
                }
                _ => return Err(Error::Integrity),
            }
            heads.insert(op.lot.id(), Head::from_operation(op));
        }
        let census = ctx.action_state(&f.census, &()).map_err(bad)?;
        let census_page = ctx.scan_action_state(&f.census, None, 2).map_err(bad)?;
        if census_page.next_after().is_some()
            || census_page.entries().len() != usize::from(census.is_some())
            || census_page
                .entries()
                .iter()
                .any(|e| e.encoded_key() != "registry")
        {
            return Err(Error::Integrity);
        }
        match census {
            None if by_sequence.is_empty() => {}
            Some(c) if c == &expected && c.count > 0 => {}
            _ => return Err(Error::Integrity),
        }
        let mut seen = 0usize;
        let mut after = None;
        loop {
            let page = ctx
                .scan_action_state(&f.heads, after.as_deref(), 128)
                .map_err(bad)?;
            for entry in page.entries() {
                let head = entry.value();
                head.validate()?;
                if entry.encoded_key() != registration::key(head.lot.0)
                    || heads.get(&head.lot) != Some(head)
                {
                    return Err(Error::Integrity);
                }
                let op = by_order
                    .get(&head.identity.order())
                    .ok_or(Error::Integrity)?;
                let lot = ctx.action_state(&f.lots, &head.lot).map_err(bad)?;
                match op.state()? {
                    CoreInputState::Held if lot != Some(&op.lot) => return Err(Error::Integrity),
                    CoreInputState::Consumed if lot.is_some() => return Err(Error::Integrity),
                    // Released snapshots are historical: later legitimate movement,
                    // split/merge or dissolution must not invalidate old evidence.
                    _ => {}
                }
                seen = seen.checked_add(1).ok_or(Error::Capacity)?;
                if seen > heads.len() {
                    return Err(Error::Integrity);
                }
            }
            after = page.next_after().cloned();
            if after.is_none() {
                break;
            }
        }
        if seen != heads.len() {
            return Err(Error::Integrity);
        }
        Ok(by_sequence.into_values().collect())
    }
}

pub(crate) fn reconcile(ctx: &ModuleCtx<'_>, families: &crate::InventoryFamilies) -> bool {
    families
        .core_inputs
        .as_ref()
        .is_none_or(|h| h.audit(ctx).is_ok())
}
