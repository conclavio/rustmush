//! Trusted composition bridge to the actual Industry owner's acknowledged offer.
//! This is not a caller credential: requests name a subject, and the fixed policy
//! reads the private boot-verified Industry projection itself on every check.
use super::{CoreFundingSubject, CoreIntegrationError as Error, CorePaymentPolicy};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, World};
use serde::{Deserialize, Serialize};

/// Historical attribution only. Deserialization never proves acknowledgement or
/// current permission; only the configured owner policy can supply live facts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreOfferAttribution {
    pub operation: CommitToken,
    pub digest: DefinitionDigest,
    pub action: DefinitionRef,
    pub operator: ObjId,
    pub observed_ic: ClockStamp,
    pub observed_utc: i64,
    pub expires_utc: i64,
}
impl CoreOfferAttribution {
    pub(crate) fn validate(
        &self,
        policy: &CorePaymentPolicy,
        subject: &CoreFundingSubject,
    ) -> Result<(), Error> {
        if self.operation != subject.offer
            || self.digest != subject.offer_digest
            || self.action != policy.action_ref("offer_core_integration")?
            || self.operator.0 == 0
            || self.observed_ic.clock != subject.accepted_at.clock
            || self.observed_ic.at > subject.accepted_at.at
            || self.observed_utc.checked_add(300) != Some(self.expires_utc)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}

/// Installed only by trusted module composition. Its implementation must read
/// the exact actual Industry-owned, boot-audited immutable offer and refuse when
/// the owner is missing, frozen, not ready, or the offer has already been used.
/// It must match the complete subject (including actual certificate/lot/recipe)
/// and retain current Industry generation checks in the Industry participant.
/// Economy separately validates Org/E4 authority, accounts and the real charge.
/// No invocation accepts an implementation of this trait or caller-built facts.
pub trait CoreOfferAuthority: Send + Sync + 'static {
    fn acknowledged_offer(
        &self,
        world: &World,
        subject: &CoreFundingSubject,
    ) -> Result<CoreOfferAttribution, Error>;
}
