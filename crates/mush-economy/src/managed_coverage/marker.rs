//! Closed contract marker, structurally decoded but never an owner capability.
use super::{BIND_ACTION, CoverageConsentV1, action_matches, refused};
use mush_core::participant::ParticipantError;
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CoverageMarkerV1 {
    operation: CommitToken,
    effect: DefinitionRef,
    product: CommitToken,
    product_digest: DefinitionDigest,
    quote: CommitToken,
    quote_digest: DefinitionDigest,
    consent_digest: DefinitionDigest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    operation: CommitToken,
    effect: DefinitionRef,
    product: CommitToken,
    product_digest: DefinitionDigest,
    quote: CommitToken,
    quote_digest: DefinitionDigest,
    consent_digest: DefinitionDigest,
}
impl<'de> Deserialize<'de> for CoverageMarkerV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let row = Self {
            operation: w.operation,
            effect: w.effect,
            product: w.product,
            product_digest: w.product_digest,
            quote: w.quote,
            quote_digest: w.quote_digest,
            consent_digest: w.consent_digest,
        };
        row.validate().map_err(serde::de::Error::custom)?;
        Ok(row)
    }
}
impl CoverageMarkerV1 {
    pub(crate) fn of(c: &CoverageConsentV1) -> Result<Self, ParticipantError> {
        c.validate()?;
        Ok(Self {
            operation: c.operation,
            effect: c.effect.clone(),
            product: c.basis.product,
            product_digest: c.basis.product_digest,
            quote: c.basis.quote,
            quote_digest: c.basis.quote_digest,
            consent_digest: c.digest()?,
        })
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        if !action_matches(&self.effect, BIND_ACTION)
            || self.operation == self.product
            || self.operation == self.quote
            || self.product == self.quote
            || [self.product_digest, self.quote_digest, self.consent_digest]
                .iter()
                .any(|d| d.as_bytes() == &[0; 32])
        {
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
    pub const fn product(&self) -> CommitToken {
        self.product
    }
    pub const fn product_digest(&self) -> DefinitionDigest {
        self.product_digest
    }
    pub const fn quote(&self) -> CommitToken {
        self.quote
    }
    pub const fn quote_digest(&self) -> DefinitionDigest {
        self.quote_digest
    }
    pub const fn consent_digest(&self) -> DefinitionDigest {
        self.consent_digest
    }
}
