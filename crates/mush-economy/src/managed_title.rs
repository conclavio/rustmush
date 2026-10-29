//! Permanent title identity derived from acknowledged managed birth history.
//!
//! An asset's editable catalogue, owner, lifecycle and staff flags cannot remove
//! this policy. The map is an in-memory projection, rebuilt from the existing
//! validated journal/checkpoint evidence; it adds no persisted authority row.

use std::collections::BTreeMap;

use mush_core::{CommitToken, ObjId};

use crate::{AssetOp, EconomyError, EconomyPlan, EconomySnapshot, TransactionId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TitleBirth {
    kind: TitleBirthKind,
    operation: CommitToken,
    transaction: TransactionId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum TitleBirthKind {
    Charter,
    Core,
}
impl TitleBirth {
    pub(crate) fn charter(operation: CommitToken, transaction: TransactionId) -> Self {
        Self {
            kind: TitleBirthKind::Charter,
            operation,
            transaction,
        }
    }
    pub(crate) fn core(operation: CommitToken, transaction: TransactionId) -> Self {
        Self {
            kind: TitleBirthKind::Core,
            operation,
            transaction,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ManagedTitleIndex {
    Ready(BTreeMap<ObjId, TitleBirth>),
    /// A frozen managed composition must not become an empty policy set when
    /// an administrator clears the unrelated whole-Economy freeze latch.
    Unavailable,
}
impl Default for ManagedTitleIndex {
    fn default() -> Self {
        Self::Ready(BTreeMap::new())
    }
}
impl ManagedTitleIndex {
    pub(crate) fn charter(&self, object: ObjId) -> Option<&TitleBirth> {
        match self {
            Self::Ready(rows) => rows
                .get(&object)
                .filter(|birth| birth.kind == TitleBirthKind::Charter),
            Self::Unavailable => None,
        }
    }
    pub(crate) fn core(&self, object: ObjId) -> Option<&TitleBirth> {
        match self {
            Self::Ready(rows) => rows
                .get(&object)
                .filter(|birth| birth.kind == TitleBirthKind::Core),
            Self::Unavailable => None,
        }
    }
    pub(crate) fn contains(&self, object: ObjId) -> bool {
        match self {
            Self::Ready(rows) => rows.contains_key(&object),
            Self::Unavailable => true,
        }
    }
    /// The native owner has checked the exact absent title/index preimage at
    /// prepare and bind. Only its infallible acknowledged apply calls this.
    pub(crate) fn install_prepared(&mut self, object: ObjId, birth: TitleBirth) {
        if let Self::Ready(rows) = self {
            rows.insert(object, birth);
        }
    }
    /// Adds only fully replay-verified native Core births; catalogue text never
    /// classifies governed objects. Cross-kind duplicate birth is corruption.
    pub(crate) fn with_cores(
        mut self,
        audit: &crate::core_integration::receipt::CoreReceiptAudit,
    ) -> Result<Self, EconomyError> {
        let Self::Ready(rows) = &mut self else {
            return Err(EconomyError::Unauthorized);
        };
        for history in audit.histories().map_err(|_| EconomyError::Unauthorized)? {
            if let Some((object, birth)) = history.managed_title_birth()
                && rows.insert(object, birth).is_some()
            {
                return Err(EconomyError::Unauthorized);
            }
        }
        Ok(self)
    }
    pub(crate) fn from_filings(
        audit: &crate::registry_filing::ReceiptAudit,
    ) -> Result<Self, EconomyError> {
        let mut rows = BTreeMap::new();
        for receipt in audit.receipts().map_err(|_| EconomyError::Unauthorized)? {
            let (object, birth) = receipt.managed_title_birth();
            if rows.insert(object, birth).is_some() {
                return Err(EconomyError::Unauthorized);
            }
        }
        Ok(Self::Ready(rows))
    }
}

/// All generic entry points, including approvals and contract settlement, reach
/// this check before idempotency or ordinary staff/owner authorization. A future
/// managed transfer must carry a separate private closed plan proof; neither an
/// AuthEval override nor a caller-selected cause/key can exempt it.
pub(crate) fn require_unmanaged_mutations(
    plan: &EconomyPlan,
    snapshot: &EconomySnapshot,
) -> Result<(), EconomyError> {
    if plan.asset_ops.len() > crate::MAX_ASSET_OPS_PER_PLAN {
        return Err(EconomyError::TooLarge("too many asset ops"));
    }
    for operation in &plan.asset_ops {
        let object = operation.object();
        if !snapshot.managed_titles.contains(object) {
            continue;
        }
        // Doc 17h cut M8d2 S1: a Registry seizure's claim is the one encumbrance a managed
        // title can carry, so a `Release` is the seizure family's too, and a held title
        // takes no op at all outside its closed plan (its audit would refuse the boot).
        if matches!(
            operation,
            AssetOp::Title { .. }
                | AssetOp::TransferTitle { .. }
                | AssetOp::Encumber { .. }
                | AssetOp::Release { .. }
                | AssetOp::SetCatalog { .. }
                | AssetOp::Abandon { .. }
                | AssetOp::WriteOff { .. }
        ) || snapshot
            .asset(object)
            .is_some_and(|asset| !asset.encumbrances.is_empty())
        {
            return Err(EconomyError::Unauthorized);
        }
    }
    Ok(())
}

/// **A note on a core's managed title** (doc 17h cut M8b C2; doc 17 §10, D-UM-42): what was
/// done to the core after its birth. A record, never a notice and never a right: M8a's
/// inspection carries it as a record field for M8d's seizure, and nothing prints it to a
/// player. The birth stays the provenance; the note is the crime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreProvenanceNote {
    /// The core was fitted by the ungoverning refit (`core_ungovern`) at `facility`, its
    /// fitting completing at IC second `at_ic`.
    Ungoverned { facility: ObjId, at_ic: i64 },
}

impl crate::EconomyStore {
    /// **The provenance note on `core`'s managed title** (doc 17h cut M8b C2): written by the
    /// completion of an ungoverning refit that installed `core`, read back from the refit's
    /// replay-verified payment history (the journal is the record, as the title index is a
    /// projection of it: no new persisted row). The latest such fitting wins; `Ok(None)` for
    /// a core never ungoverned or an unmanaged object.
    ///
    /// # Errors
    ///
    /// The history's own error when it cannot be read (a frozen economy, an incomplete
    /// replay): the note fails closed (G9), so a reader refuses rather than record "none".
    ///
    /// Retention: the refit histories are never pruned. They live in the fold
    /// (`FoldState::refit_receipts`, `fold.rs`, observed per journal transaction), the
    /// checkpoint carries that fold, and boot discards a checkpoint lacking exact refit-receipt
    /// coverage and replays the journal instead (`boot.rs`, `RefitReceiptAudit::
    /// checkpoint_complete`); the audit keeps every history up to `MAX_ORDERS`.
    pub fn core_provenance_note(
        &self,
        core: ObjId,
    ) -> Result<Option<CoreProvenanceNote>, crate::core_refit::RefitError> {
        let histories = self.core_refit_histories()?;
        if self.snapshot().managed_titles.core(core).is_none() {
            return Ok(None);
        }
        Ok(histories
            .filter(|history| {
                history.subject().kind == crate::core_refit::RefitKind::CoreUngovern
                    && history.subject().incoming == core
            })
            .filter_map(|history| {
                let terminal = history.terminal()?;
                terminal.result().replacement()?;
                let facility = history.policy().ungovern.as_ref()?.factory;
                Some(CoreProvenanceNote::Ungoverned {
                    facility,
                    at_ic: terminal.at_ic().at,
                })
            })
            .max_by_key(|note| match note {
                CoreProvenanceNote::Ungoverned { at_ic, .. } => *at_ic,
            }))
    }
}

#[cfg(test)]
mod m8b_tests {
    use super::*;

    /// **Doc 17h cut M8b G9**: the note fails closed. A store whose refit history was never
    /// replayed, and a frozen one, answer `Err` (a reader refuses); a booted ordinary store
    /// answers `Ok(None)` for a core never ungoverned.
    #[test]
    fn m8b_the_provenance_note_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        check!(
            crate::EconomyStore::default()
                .core_provenance_note(ObjId(5))
                .is_err()
        );
        let world = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_org::OrgModule::new()),
            Box::new(crate::EconomyModule::new()),
        ]);
        let store = world
            .world()
            .module_state::<crate::EconomyStore>()
            .ok_or("the store")?;
        check_eq!(store.core_provenance_note(ObjId(5))?, None);
        let mut frozen = store.clone();
        frozen.frozen = true;
        check!(frozen.core_provenance_note(ObjId(5)).is_err());
        Ok(())
    }
}
