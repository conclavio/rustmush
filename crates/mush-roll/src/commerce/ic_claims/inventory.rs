//! Bounded derived accounting. This is not an independent durable seat count.
//! The final owner audit supplies every actual claim row and separately proves
//! all four owner inventories agree before this projection may become usable.

use super::super::records::{PublicationError, Record};
use super::super::{Capacity, LicenceId};
use super::{ClaimRecord, CloseCause, MAX_IC_CLAIM_HISTORY, MAX_OPEN_IC_CLAIMS};
use mush_core::{CommitToken, ObjId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct ClaimInventory {
    generation: u64,
    next_transition: u64,
    history: BTreeMap<CommitToken, Arc<ClaimRecord>>,
    open_units: BTreeMap<ObjId, CommitToken>,
    open_paid: BTreeMap<LicenceId, usize>,
}
impl ClaimInventory {
    /// Strict projection only; success does not prove custody or readiness.
    pub(crate) fn reconstruct(
        rows: impl IntoIterator<Item = ClaimRecord>,
    ) -> Result<Self, PublicationError> {
        let mut value = Self {
            generation: 0,
            next_transition: 1,
            history: BTreeMap::new(),
            open_units: BTreeMap::new(),
            open_paid: BTreeMap::new(),
        };
        let mut events = BTreeMap::new();
        let mut closes = BTreeSet::new();
        for row in rows {
            row.validate()?;
            if value.history.len() >= MAX_IC_CLAIM_HISTORY
                || events
                    .insert(row.open.sortie.get(), (row.open.operation, false))
                    .is_some()
                || value.history.contains_key(&row.open.operation)
            {
                return Err(PublicationError::Capacity);
            }
            if let Some(closed) = &row.closed {
                if !closes.insert(closed.operation)
                    || events
                        .insert(closed.transition, (row.open.operation, true))
                        .is_some()
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            value.history.insert(row.open.operation, Arc::new(row));
        }
        if closes
            .iter()
            .any(|commit| value.history.contains_key(commit))
        {
            return Err(PublicationError::Mismatch);
        }
        // Actual owner-assigned order, independent of wall-clock generations.
        // Replaying Open and Close detects historical over-admission, not just
        // the final count. The mandatory audit also compares the actual host's
        // retained next_transition head, including when these rows are empty.
        for (ordinal, (operation, closed)) in events {
            if ordinal != value.next_transition {
                return Err(PublicationError::Mismatch);
            }
            let row = Arc::clone(
                value
                    .history
                    .get(&operation)
                    .ok_or(PublicationError::Mismatch)?,
            );
            if closed {
                value.remove_open(&row)?;
            } else {
                value.add_open(&row)?;
            }
            value.next_transition = value
                .next_transition
                .checked_add(1)
                .ok_or(PublicationError::Capacity)?;
        }
        Ok(value)
    }
    fn add_open(&mut self, row: &ClaimRecord) -> Result<(), PublicationError> {
        if self.open_units.len() >= MAX_OPEN_IC_CLAIMS
            || self.open_units.contains_key(&row.open.unit)
        {
            return Err(PublicationError::Capacity);
        }
        if let Capacity::PaidSeats { seats } = row.open.capacity {
            let count = self.open_paid.entry(row.open.licence).or_default();
            if *count >= usize::from(seats.get()) {
                return Err(PublicationError::Capacity);
            }
            *count = count.checked_add(1).ok_or(PublicationError::Capacity)?;
        }
        self.open_units.insert(row.open.unit, row.open.operation);
        Ok(())
    }
    fn remove_open(&mut self, row: &ClaimRecord) -> Result<(), PublicationError> {
        if self.open_units.get(&row.open.unit) != Some(&row.open.operation) {
            return Err(PublicationError::Mismatch);
        }
        if matches!(row.open.capacity, Capacity::PaidSeats { .. }) {
            let count = self
                .open_paid
                .get_mut(&row.open.licence)
                .ok_or(PublicationError::Mismatch)?;
            *count = count.checked_sub(1).ok_or(PublicationError::Mismatch)?;
            if *count == 0 {
                self.open_paid.remove(&row.open.licence);
            }
        }
        self.open_units.remove(&row.open.unit);
        Ok(())
    }
    pub(crate) const fn next_transition(&self) -> u64 {
        self.next_transition
    }
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn open_len(&self) -> usize {
        self.open_units.len()
    }
    pub(crate) fn history_len(&self) -> usize {
        self.history.len()
    }
    pub(crate) fn paid_open(&self, licence: LicenceId) -> usize {
        self.open_paid.get(&licence).copied().unwrap_or(0)
    }
    pub(crate) fn claim(&self, operation: CommitToken) -> Option<&ClaimRecord> {
        self.history.get(&operation).map(Arc::as_ref)
    }

    /// Called only after actual current holding and four-owner preparation.
    /// The candidate has already captured the holding-derived capacity. This
    /// method does not authenticate that capacity and is not a public API.
    pub(crate) fn opening(&self, row: ClaimRecord) -> Result<Self, PublicationError> {
        row.validate()?;
        if row.closed.is_some()
            || row.open.sortie.get() != self.next_transition
            || self.history.contains_key(&row.open.operation)
            || self.history.values().any(|old| {
                old.open.sortie == row.open.sortie
                    || old
                        .closed
                        .as_ref()
                        .is_some_and(|closed| closed.operation == row.open.operation)
            })
            || self.history.len() >= MAX_IC_CLAIM_HISTORY
            || self.open_units.len() >= MAX_OPEN_IC_CLAIMS
            || self.open_units.contains_key(&row.open.unit)
        {
            return Err(PublicationError::Capacity);
        }
        if let Capacity::PaidSeats { seats } = row.open.capacity {
            if self.paid_open(row.open.licence) >= usize::from(seats.get()) {
                return Err(PublicationError::Capacity);
            }
        }
        let mut post = self.clone();
        post.generation = post
            .generation
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;
        post.next_transition = post
            .next_transition
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;
        post.add_open(&row)?;
        post.history.insert(row.open.operation, Arc::new(row));
        Ok(post)
    }

    /// Exact owner event only. None means the same event was already consumed;
    /// it does not change counts, generation, old receipt or any open identity.
    pub(crate) fn closing(&self, cause: CloseCause) -> Result<Option<Self>, PublicationError> {
        let before = self.claim(cause.open).ok_or(PublicationError::Mismatch)?;
        cause.validate_for(&before.open)?;
        if let Some(closed) = &before.closed {
            return if closed.same_event(&cause) {
                Ok(None)
            } else {
                Err(PublicationError::Mismatch)
            };
        }
        if self.open_units.get(&cause.unit) != Some(&cause.open)
            || cause.transition != self.next_transition
            || self.history.contains_key(&cause.operation)
            || self.history.values().any(|row| {
                row.closed
                    .as_ref()
                    .is_some_and(|closed| closed.operation == cause.operation)
            })
        {
            return Err(PublicationError::Mismatch);
        }
        let mut row = before.clone();
        row.closed = Some(cause);
        row.validate()?;
        let mut post = self.clone();
        post.generation = post
            .generation
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;
        post.next_transition = post
            .next_transition
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;
        post.remove_open(&row)?;
        post.history.insert(row.open.operation, Arc::new(row));
        Ok(Some(post))
    }
}
