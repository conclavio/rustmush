//! Structural pairing is separate from actual borrowed owner evidence. Neither
//! path decides current eligibility or publishes a grant.
use super::records::{MindRenewalOperationV1, MindRenewalReceiptV1, invalid};
use mush_economy::managed_renewal::RenewalOfferView;
use mush_roll::commerce::renewal::RenewalOperationV1;

impl MindRenewalOperationV1 {
    /// Exact data equality with a Roll row. The caller must obtain that row
    /// through the registered owner/typed preimage path, not an arbitrary decode.
    pub fn matches_roll(&self, actual: &RenewalOperationV1) -> Result<(), String> {
        self.validate()?;
        actual.validate().map_err(|_| invalid())?;
        if &self.roll_projection()? != actual {
            return Err(invalid());
        }
        Ok(())
    }
    /// Requires the Economy owner's private combined audit view. This cannot
    /// be called with a serialized receipt or a merely terminal contract.
    pub fn verify_history(&self, actual: &RenewalOfferView<'_>) -> Result<(), String> {
        self.validate()?;
        if actual.offer() != self.offer.as_ref() {
            return Err(invalid());
        }
        self.roll_projection()?
            .verify_history(actual)
            .map_err(|_| invalid())
    }
}
impl MindRenewalReceiptV1 {
    pub fn matches_operation(&self, operation: &MindRenewalOperationV1) -> Result<(), String> {
        self.validate()?;
        operation.validate()?;
        let binding = operation.binding();
        if self.operation != operation.operation()
            || self.licence != binding.licence()
            || self.predecessor != binding.predecessor()
            || self.next != binding.next()
            || self.contract != operation.contract()
            || &self.consent_hash != operation.consent_hash()
            || &self.state != operation.state()
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn verify_history(
        &self,
        operation: &MindRenewalOperationV1,
        actual: &RenewalOfferView<'_>,
    ) -> Result<(), String> {
        self.matches_operation(operation)?;
        operation.verify_history(actual)
    }
}
