//! One owner-local bounded book. It never treats absence as a close event.
use super::protocol::{MAX_HISTORY, MAX_OPEN, MAX_TOTAL_BYTES};
use super::records::{Head, Record, bad};
use mush_core::participant::ParticipantError;
use mush_core::{CommitToken, ObjId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct IcSorties {
    pub(super) loaded: bool,
    pub(super) generation: u64,
    pub(super) next_transition: u64,
    pub(super) reserved_bytes: usize,
    pub(super) rows: BTreeMap<CommitToken, Arc<Record>>,
    pub(super) active: BTreeMap<ObjId, CommitToken>,
}
impl IcSorties {
    pub(super) fn reconstruct(
        head: Head,
        rows: impl IntoIterator<Item = Record>,
    ) -> Result<Self, ParticipantError> {
        head.validate()?;
        let mut s = Self {
            loaded: true,
            next_transition: 1,
            reserved_bytes: 128,
            ..Self::default()
        };
        let mut order = BTreeMap::new();
        let mut commits = BTreeSet::new();
        let mut requests = BTreeSet::new();
        for r in rows {
            r.validate()?;
            let open = r.open.operation;
            if s.rows.len() >= MAX_HISTORY
                || !commits.insert(open)
                || !requests.insert(r.open.request)
                || order
                    .insert(r.open.execution.seal.sortie.0, (open, false))
                    .is_some()
            {
                return Err(bad("duplicate or excess history"));
            }
            if let Some(c) = &r.closed {
                if !commits.insert(c.operation)
                    || !requests.insert(c.request)
                    || order.insert(c.transition, (open, true)).is_some()
                {
                    return Err(bad("crossed close"));
                }
            }
            s.reserved_bytes = s
                .reserved_bytes
                .checked_add(r.reserved_bytes()?)
                .ok_or_else(|| bad("bytes"))?;
            if s.reserved_bytes > MAX_TOTAL_BYTES {
                return Err(bad("total bytes"));
            }
            if s.rows.insert(open, Arc::new(r)).is_some() {
                return Err(bad("duplicate open"));
            }
        }
        for (ordinal, (open, closing)) in order {
            if ordinal != s.next_transition {
                return Err(bad("missing transition"));
            }
            let row = s.rows.get(&open).ok_or_else(|| bad("transition"))?;
            let unit = row.open.execution.seal.unit;
            if closing {
                if s.active.remove(&unit) != Some(open) {
                    return Err(bad("crossed release"));
                }
            } else if s.active.len() >= MAX_OPEN || s.active.insert(unit, open).is_some() {
                return Err(bad("historical capacity"));
            }
            s.next_transition = s
                .next_transition
                .checked_add(1)
                .ok_or_else(|| bad("ordinal"))?;
        }
        if head.next_transition != s.next_transition {
            return Err(bad("head completeness"));
        }
        Ok(s)
    }
    pub(super) fn request_used(&self, request: mush_core::DefinitionDigest) -> bool {
        self.rows.values().any(|r| {
            r.open.request == request || r.closed.as_ref().is_some_and(|c| c.request == request)
        })
    }
    pub(super) fn ready(&self) -> Result<(), ParticipantError> {
        // Config::check also requires the actual paired Roll final audit.
        if !self.loaded {
            return Err(bad("four-owner audit incomplete"));
        }
        Ok(())
    }
    pub(super) fn opening(&self, row: Record) -> Result<Self, ParticipantError> {
        self.ready()?;
        row.validate()?;
        if row.closed.is_some()
            || row.open.execution.seal.sortie.0 != self.next_transition
            || self.rows.len() >= MAX_HISTORY
            || self.active.len() >= MAX_OPEN
            || self.request_used(row.open.request)
            || self.rows.contains_key(&row.open.operation)
            || self.active.contains_key(&row.open.execution.seal.unit)
            || self.rows.values().any(|r| {
                r.closed
                    .as_ref()
                    .is_some_and(|c| c.operation == row.open.operation)
            })
        {
            return Err(bad("open capacity/identity"));
        }
        let mut post = self.clone();
        post.reserved_bytes = post
            .reserved_bytes
            .checked_add(row.reserved_bytes()?)
            .ok_or_else(|| bad("bytes"))?;
        if post.reserved_bytes > MAX_TOTAL_BYTES {
            return Err(bad("total bytes"));
        }
        post.active
            .insert(row.open.execution.seal.unit, row.open.operation);
        post.rows.insert(row.open.operation, Arc::new(row));
        post.next_transition = post
            .next_transition
            .checked_add(1)
            .ok_or_else(|| bad("ordinal"))?;
        post.generation = post
            .generation
            .checked_add(1)
            .ok_or_else(|| bad("generation"))?;
        Ok(post)
    }
    pub(super) fn closing(&self, row: Record) -> Result<Self, ParticipantError> {
        self.ready()?;
        row.validate()?;
        let before = self
            .rows
            .get(&row.open.operation)
            .ok_or_else(|| bad("open missing"))?;
        let c = row.closed.as_ref().ok_or_else(|| bad("close missing"))?;
        if before.open != row.open
            || before.closed.is_some()
            || c.transition != self.next_transition
            || self.active.get(&row.open.execution.seal.unit) != Some(&row.open.operation)
            || self.request_used(c.request)
            || self.rows.contains_key(&c.operation)
            || self.rows.values().any(|r| {
                r.closed
                    .as_ref()
                    .is_some_and(|old| old.operation == c.operation)
            })
            || before.reserved_bytes()? != row.reserved_bytes()?
        {
            return Err(bad("close identity"));
        }
        let mut post = self.clone();
        post.active.remove(&row.open.execution.seal.unit);
        post.rows.insert(row.open.operation, Arc::new(row));
        post.next_transition = post
            .next_transition
            .checked_add(1)
            .ok_or_else(|| bad("ordinal"))?;
        post.generation = post
            .generation
            .checked_add(1)
            .ok_or_else(|| bad("generation"))?;
        Ok(post)
    }
}
