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
            if let Some((object, birth)) = history.managed_title_birth() {
                if rows.insert(object, birth).is_some() {
                    return Err(EconomyError::Unauthorized);
                }
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
        if snapshot.managed_titles.contains(operation.object())
            && matches!(
                operation,
                AssetOp::Title { .. }
                    | AssetOp::TransferTitle { .. }
                    | AssetOp::Encumber { .. }
                    | AssetOp::SetCatalog { .. }
                    | AssetOp::Abandon { .. }
                    | AssetOp::WriteOff { .. }
            )
        {
            return Err(EconomyError::Unauthorized);
        }
    }
    Ok(())
}
