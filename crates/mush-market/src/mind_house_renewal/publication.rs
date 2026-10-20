//! Only an actual closed writer result can supply the paired publication here.
//! These helpers do not mint owner evidence or install any rows.
use super::{MindRenewalOperationV1, MindRenewalReceiptV1, RenewalConfig};
use mush_economy::managed_house_renewal::RenewalOffer;
use mush_roll::commerce::house_renewal::RenewalOperationV1;

impl MindRenewalOperationV1 {
    pub(super) fn captured(
        config: &RenewalConfig,
        offer: RenewalOffer,
        row: &RenewalOperationV1,
    ) -> Result<Self, String> {
        let value = Self {
            format: 1,
            operation: row.operation(),
            action: row.action().clone(),
            attempt: row.attempt(),
            manifest: config.manifest.clone(),
            participants: config.participants.ordered(),
            binding: Box::new(row.binding().clone()),
            offer: Box::new(offer),
            offer_digest: row.offer_digest(),
            contract: row.contract(),
            consent_hash: *row.consent_hash(),
            created_at_ic: row.created_at_ic().clone(),
            created_at_utc: row.created_at_utc(),
            state: row.state().clone(),
        };
        value.matches_roll(row)?;
        Ok(value)
    }
    pub(super) fn progressed(&self, row: &RenewalOperationV1) -> Result<Self, String> {
        let value = Self {
            state: row.state().clone(),
            ..self.clone()
        };
        value.matches_roll(row)?;
        Ok(value)
    }
    pub(super) fn receipt(&self) -> Result<MindRenewalReceiptV1, String> {
        let value = MindRenewalReceiptV1 {
            format: 1,
            operation: self.operation,
            licence: self.binding.licence(),
            predecessor: self.binding.predecessor(),
            next: self.binding.next(),
            contract: self.contract,
            consent_hash: self.consent_hash,
            state: self.state.clone(),
        };
        value.matches_operation(self)?;
        Ok(value)
    }
}
