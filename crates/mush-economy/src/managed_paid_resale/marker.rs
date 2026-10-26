//! Distinct native managed kind; no old purchase/renewal marker is repurposed.
use super::*;
use mush_core::{CommitToken, DefinitionDigest};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaidResaleMarkerV1 {
    pub format: u16,
    pub operation: CommitToken,
    pub effect: DefinitionRef,
    pub offer: CommitToken,
    pub offer_digest: DefinitionDigest,
    pub consent_digest: DefinitionDigest,
    pub mode: PaidResaleModeV1,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    format: u16,
    operation: CommitToken,
    effect: DefinitionRef,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    consent_digest: DefinitionDigest,
    mode: PaidResaleModeV1,
}
impl<'de> Deserialize<'de> for PaidResaleMarkerV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let row = Self {
            format: w.format,
            operation: w.operation,
            effect: w.effect,
            offer: w.offer,
            offer_digest: w.offer_digest,
            consent_digest: w.consent_digest,
            mode: w.mode,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl PaidResaleMarkerV1 {
    pub fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.offer == self.operation
            || !action_matches(
                &self.effect,
                if self.mode == PaidResaleModeV1::Immediate {
                    ACCEPT
                } else {
                    FUND
                },
            )
        {
            return Err(refused());
        }
        subject::bounded(self, 2048)?;
        Ok(())
    }
    pub(crate) fn of(c: &PaidResaleConsentV1) -> Result<Self, ParticipantError> {
        let row = Self {
            format: 1,
            operation: c.operation,
            effect: c.effect.clone(),
            offer: c.terms.offer,
            offer_digest: c.offer_digest,
            consent_digest: c.digest()?,
            mode: c.mode,
        };
        row.validate()?;
        Ok(row)
    }
}
