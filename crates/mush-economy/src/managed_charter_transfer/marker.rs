//! Exact closed consent identity. A decoded marker is not a funding capability.
use super::{CharterPendingConsentV1, FUND_ACTION, action_matches, refused};
use mush_core::participant::ParticipantError;
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CharterPendingMarkerV1 {
    operation: CommitToken,
    effect: DefinitionRef,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    terms_digest: DefinitionDigest,
    consent_digest: DefinitionDigest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    operation: CommitToken,
    effect: DefinitionRef,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    terms_digest: DefinitionDigest,
    consent_digest: DefinitionDigest,
}
impl<'de> Deserialize<'de> for CharterPendingMarkerV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let row = Self {
            operation: w.operation,
            effect: w.effect,
            offer: w.offer,
            offer_digest: w.offer_digest,
            terms_digest: w.terms_digest,
            consent_digest: w.consent_digest,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl CharterPendingMarkerV1 {
    pub(crate) fn of(consent: &CharterPendingConsentV1) -> Result<Self, ParticipantError> {
        consent.validate()?;
        Ok(Self {
            operation: consent.operation,
            effect: consent.effect.clone(),
            offer: consent.terms.offer,
            offer_digest: consent.terms.offer_digest,
            terms_digest: consent.terms.digest()?,
            consent_digest: consent.digest()?,
        })
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        if self.operation == self.offer || !action_matches(&self.effect, FUND_ACTION, 1) {
            return Err(refused());
        }
        Ok(())
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn effect(&self) -> &DefinitionRef {
        &self.effect
    }
    pub const fn offer(&self) -> CommitToken {
        self.offer
    }
    pub const fn offer_digest(&self) -> DefinitionDigest {
        self.offer_digest
    }
    pub const fn terms_digest(&self) -> DefinitionDigest {
        self.terms_digest
    }
    pub const fn consent_digest(&self) -> DefinitionDigest {
        self.consent_digest
    }
}
