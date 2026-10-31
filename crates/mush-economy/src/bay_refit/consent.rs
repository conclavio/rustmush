//! Only trusted composition installs this callback. Its production implementation
//! reads the real acknowledged Industry owner state, never request-supplied facts.
use super::{BayPaymentPolicy, BayRefitError as Error, BaySubject};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, World};
use serde::{Deserialize, Serialize};
/// The supplier's acknowledged offer the subject takes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayOfferAttribution {
    pub operation: CommitToken,
    pub digest: DefinitionDigest,
    pub action: DefinitionRef,
    pub operator: ObjId,
    pub observed_ic: ClockStamp,
    pub observed_utc: i64,
    pub expires_utc: i64,
}
impl BayOfferAttribution {
    pub(crate) fn validate(&self, p: &BayPaymentPolicy, s: &BaySubject) -> Result<(), Error> {
        s.matches_policy(p)?;
        if self.operation != s.offer
            || self.digest != s.offer_digest
            || self.action != p.action_ref(super::OFFER_ACTION)?
            || self.observed_ic.clock != s.accepted_at.clock
            || self.observed_ic.at > s.accepted_at.at
            || self.observed_utc.checked_add(300) != Some(self.expires_utc)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
/// Facts from acknowledged scheduling. Native money independently checks the
/// exact acceptance, clock and due time; the callback cannot extend expiry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BayOrderAttribution {
    pub acceptance: CommitToken,
    pub subject: DefinitionDigest,
    pub owner_generation: u64,
    pub due_at: ClockStamp,
    pub required_work: u64,
    pub completed_work: u64,
    /// The exact pure refit derived from the acknowledged order and the
    /// current unit.
    pub refit: DefinitionDigest,
}
pub trait BayOwnerAuthority: Send + Sync + 'static {
    fn acknowledged_offer(
        &self,
        world: &World,
        subject: &BaySubject,
    ) -> Result<BayOfferAttribution, Error>;
    fn acknowledged_ready_order(
        &self,
        world: &World,
        subject: &BaySubject,
    ) -> Result<BayOrderAttribution, Error>;
}
