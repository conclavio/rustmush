//! Managed identity follows the actual filing birth and every later title effect.
//! A native namespace census alone cannot prove absence of an old generic sale.
use super::*;
use crate::record::{AssetEffect, AssetEffectKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lineage {
    birth: InitialFilingAuditSubject,
    title: EconomicAsset,
    sales: u64,
    #[serde(default, skip_serializing_if = "zero")]
    pending_sales: u64,
}
fn zero(value: &u64) -> bool {
    *value == 0
}
impl Lineage {
    pub(crate) fn born(tx: &EconomicTransaction) -> Result<Self, ReceiptAuditError> {
        let birth = super::super::receipt::subject_of(tx)?;
        let [effect] = tx.asset_effects.as_slice() else {
            return Err(ReceiptAuditError::Refused);
        };
        Ok(Self {
            birth,
            title: effect.post.clone(),
            sales: 0,
            pending_sales: 0,
        })
    }
    pub(crate) fn object(&self) -> ObjId {
        self.title.object
    }
    pub(crate) fn birth(&self) -> &InitialFilingAuditSubject {
        &self.birth
    }
    pub(crate) fn title(&self) -> &EconomicAsset {
        &self.title
    }
    pub(crate) fn sales(&self) -> u64 {
        self.sales
    }
    pub(crate) fn pending_sales(&self) -> u64 {
        self.pending_sales
    }
    pub(crate) fn advance_pending(
        &self,
        tx: &EconomicTransaction,
        effect: &AssetEffect,
        history: &crate::managed_charter_transfer::CharterPendingHistory,
    ) -> Result<Self, ReceiptAuditError> {
        self.check()?;
        history.validate().map_err(|_| ReceiptAuditError::Refused)?;
        let c = history.consent();
        let terminal = history.terminal().ok_or(ReceiptAuditError::Refused)?;
        if !terminal.decision().reason.settles()
            || terminal.transaction() != tx.id
            || terminal.journal_seq() != tx.journal_seq
            || terminal.decision().posted_at_utc != tx.posted_at
            || tx.actor != PrincipalId::System(history.issuer().clone())
            || terminal.title() != Some(&effect.post)
            || effect.kind != AssetEffectKind::TitleTransferred
            || effect.object != self.object()
            || effect.post.object != self.object()
            || c.terms.charter != self.object()
            || c.terms.birth != self.birth.commit()
            || c.terms.title_revision != self.title.revision
            || self.title.revision.checked_add(1) != Some(effect.post.revision)
            || self.title.last_journal_seq > tx.journal_seq
            || effect.post.last_journal_seq != tx.journal_seq
            || self.title.title_owner != PrincipalId::Object(c.terms.seller)
            || self.title.scope.is_some()
            || self.title.stolen.is_some()
            || !self.title.encumbrances.is_empty()
            || self.title.catalog != effect.post.catalog
            || effect.post.frozen
            || effect.post.frozen_reason.is_some()
        {
            return Err(ReceiptAuditError::Refused);
        }
        let mut after = self.clone();
        after.pending_sales = after
            .pending_sales
            .checked_add(1)
            .ok_or(ReceiptAuditError::Refused)?;
        after.title = effect.post.clone();
        after.check()?;
        Ok(after)
    }
    pub(crate) fn check(&self) -> Result<(), ReceiptAuditError> {
        let (object, _) = self.birth.managed_title_birth();
        if object != self.title.object
            || self.title.revision == 0
            || self.title.frozen
            || self.title.frozen_reason.is_some()
            || self.title.catalog.is_none()
            || self.title.lifecycle != AssetLifecycle::Active
        {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
    pub(crate) fn advance(
        &self,
        tx: &EconomicTransaction,
        effect: &AssetEffect,
        sale: Option<&CharterTransferAuditSubject>,
    ) -> Result<Self, ReceiptAuditError> {
        self.check()?;
        if effect.object != self.object()
            || effect.post.object != self.object()
            || self.title.revision.checked_add(1) != Some(effect.post.revision)
            || self.title.last_journal_seq > tx.journal_seq
            || effect.post.last_journal_seq != tx.journal_seq
            || effect.post.frozen
            || effect.post.frozen_reason.is_some()
        {
            return Err(ReceiptAuditError::Refused);
        }
        let mut after = self.clone();
        match effect.kind {
            AssetEffectKind::TitleTransferred => {
                let sale = sale.ok_or(ReceiptAuditError::Refused)?;
                sale.validate()?;
                if sale.charter != self.object()
                    || sale.title != effect.post
                    || sale.title_revision != self.title.revision
                    || self.title.title_owner != PrincipalId::Object(sale.seller)
                    || self.title.scope.is_some()
                    || self.title.stolen.is_some()
                    || !self.title.encumbrances.is_empty()
                    || self.title.catalog != effect.post.catalog
                {
                    return Err(ReceiptAuditError::Refused);
                }
                after.sales = after
                    .sales
                    .checked_add(1)
                    .ok_or(ReceiptAuditError::Refused)?;
            }
            AssetEffectKind::Assigned
            | AssetEffectKind::Unassigned
            | AssetEffectKind::MaintenanceSet
            | AssetEffectKind::Released
            | AssetEffectKind::StolenReported => {
                if effect.post.title_owner != self.title.title_owner
                    || effect.post.scope != self.title.scope
                    || effect.post.catalog != self.title.catalog
                    || effect.post.lifecycle != self.title.lifecycle
                    || effect.post.acquisition != self.title.acquisition
                {
                    return Err(ReceiptAuditError::Refused);
                }
            }
            AssetEffectKind::Created
            | AssetEffectKind::Encumbered
            | AssetEffectKind::CatalogSet
            | AssetEffectKind::WrittenOff
            | AssetEffectKind::Abandoned => return Err(ReceiptAuditError::Refused),
        }
        after.title = effect.post.clone();
        after.check()?;
        Ok(after)
    }
}

pub(crate) fn is_birth_namespace(ns: &str) -> bool {
    ns == super::super::receipt::FILING_NAMESPACE || ns == super::super::house::NAMESPACE
}
pub(crate) fn birth_key_matches(
    birth: &InitialFilingAuditSubject,
    row: &crate::checkpoint::IdemRow,
) -> bool {
    // The receipt's complete exact shape is checked again against the separately
    // recovered filing audit before managed identity becomes usable.
    birth.matches_checkpoint_key(row)
}
