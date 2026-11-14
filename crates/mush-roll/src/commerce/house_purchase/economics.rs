//! Immutable public consent for the closed sealed personal house product.
//! Structural values do not grant payment, source, use or renewal authority.
use super::HouseProtocol;
use crate::commerce::paid::{PaidForm, PaidTerms};
use crate::commerce::records::{PublicationError, Record, RevisionPolicy};
use crate::commerce::{LicenceId, MAX_GRACE_SECONDS, MAX_TERM_SECONDS, TermGeneration};
use mush_core::checked;
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef};
use mush_economy::{Amount, InstrumentId};
use serde::{Deserialize, Serialize};
pub const MAX_ECONOMICS_BYTES: usize = 8192;
fn invalid() -> PublicationError {
    PublicationError::Invalid
}
fn value<T: Serialize>(v: &T) -> Result<serde_json::Value, PublicationError> {
    let v = serde_json::to_value(v).map_err(|_| invalid())?;
    mush_core::canonical_json_bytes_with_limit(&v, MAX_ECONOMICS_BYTES)
        .map_err(|_| PublicationError::Capacity)?;
    Ok(v)
}
fn digest<T: Serialize>(domain: &str, v: &T) -> Result<DefinitionDigest, PublicationError> {
    DefinitionDigest::for_canonical_value(domain, &value(v)?).map_err(|_| invalid())
}
fn terms(
    protocol: HouseProtocol,
    terms: PaidTerms,
    revision: RevisionPolicy,
) -> Result<(), PublicationError> {
    if terms.seats().get() != protocol.seats()
        || terms.form() != (PaidForm::Sealed {})
        || !matches!(revision, RevisionPolicy::Pin { .. })
    {
        return Err(invalid());
    }
    Ok(())
}
checked!(pub struct HouseRenewalTemplateV1, RenewalWire, pub(crate) fields, {
    format:u16,instrument:InstrumentId,price:Amount,term_seconds:i64,grace_seconds:i64,
    terms:PaidTerms,revision:RevisionPolicy,policy:DefinitionRef,
});
impl HouseRenewalTemplateV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        price: Amount,
        term_seconds: i64,
        grace_seconds: i64,
        terms: PaidTerms,
        revision: RevisionPolicy,
        policy: DefinitionRef,
    ) -> Result<Self, PublicationError> {
        Self::for_protocol(
            HouseProtocol::HouseV1,
            instrument,
            price,
            term_seconds,
            grace_seconds,
            terms,
            revision,
            policy,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn for_protocol(
        protocol: HouseProtocol,
        instrument: InstrumentId,
        price: Amount,
        term_seconds: i64,
        grace_seconds: i64,
        terms: PaidTerms,
        revision: RevisionPolicy,
        policy: DefinitionRef,
    ) -> Result<Self, PublicationError> {
        let v = Self {
            format: protocol.format(),
            instrument,
            price,
            term_seconds,
            grace_seconds,
            terms,
            revision,
            policy,
        };
        v.validate()?;
        Ok(v)
    }
    pub fn validate(&self) -> Result<(), PublicationError> {
        let protocol = HouseProtocol::from_format(self.format)?;
        terms(protocol, self.terms, self.revision)?;
        if self.price.is_negative()
            || self.price.is_zero()
            || !(1..=MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&self.grace_seconds)
            || self.policy.key().owner.as_str() != "market"
            || self.policy.key().kind.as_str() != "entitlement_policy"
            || self.policy.key().id.as_str() != "house_personal"
            || self.policy.version() != protocol.version()
        {
            return Err(invalid());
        }
        value(self)?;
        Ok(())
    }
    pub const fn format(&self) -> u16 {
        self.format
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub const fn price(&self) -> Amount {
        self.price
    }
    pub const fn term_seconds(&self) -> i64 {
        self.term_seconds
    }
    pub const fn grace_seconds(&self) -> i64 {
        self.grace_seconds
    }
    pub const fn terms(&self) -> PaidTerms {
        self.terms
    }
    pub const fn revision(&self) -> RevisionPolicy {
        self.revision
    }
    pub fn policy(&self) -> &DefinitionRef {
        &self.policy
    }
    pub fn matches_product(
        &self,
        t: PaidTerms,
        r: RevisionPolicy,
        i: &InstrumentId,
        p: &DefinitionRef,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        if self.terms != t || self.revision != r || &self.instrument != i || &self.policy != p {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HouseDownstreamV1 {
    Sealed {},
}
checked!(pub struct HouseGrantEconomicsV1, GrantWire, pub(crate) fields, {
    format:u16,downstream:HouseDownstreamV1,renewal:HouseRenewalTemplateV1,
});
impl HouseGrantEconomicsV1 {
    pub fn new(renewal: HouseRenewalTemplateV1) -> Result<Self, PublicationError> {
        let v = Self {
            format: renewal.format(),
            downstream: HouseDownstreamV1::Sealed {},
            renewal,
        };
        v.validate()?;
        Ok(v)
    }
    pub fn validate(&self) -> Result<(), PublicationError> {
        if self.format != self.renewal.format() {
            return Err(invalid());
        }
        self.renewal.validate()?;
        value(self)?;
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest(
            match HouseProtocol::from_format(self.format)? {
                HouseProtocol::HouseV1 => "roll.mind.house_grant_economics.v1",
                HouseProtocol::HouseV2 => "roll.mind.house_grant_economics.v2",
            },
            self,
        )
    }
    pub const fn format(&self) -> u16 {
        self.format
    }
    pub fn renewal(&self) -> &HouseRenewalTemplateV1 {
        &self.renewal
    }
}
checked!(pub struct HouseTermEconomicsV1, TermWire, pub(crate) fields, {
    format:u16,licence:LicenceId,generation:TermGeneration,operation:CommitToken,
    product_digest:DefinitionDigest,grant:HouseGrantEconomicsV1,
});
impl HouseTermEconomicsV1 {
    pub(crate) fn capture(
        licence: LicenceId,
        generation: TermGeneration,
        operation: CommitToken,
        product_digest: DefinitionDigest,
        grant: HouseGrantEconomicsV1,
    ) -> Result<Self, PublicationError> {
        let v = Self {
            format: grant.format(),
            licence,
            generation,
            operation,
            product_digest,
            grant,
        };
        v.validate()?;
        Ok(v)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        if bytes.is_empty() || bytes.len() > MAX_ECONOMICS_BYTES {
            return Err(PublicationError::Capacity);
        }
        serde_json::from_slice(bytes).map_err(|_| invalid())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest(
            match HouseProtocol::from_format(self.format)? {
                HouseProtocol::HouseV1 => "roll.mind.house_term_economics.v1",
                HouseProtocol::HouseV2 => "roll.mind.house_term_economics.v2",
            },
            self,
        )
    }
    pub const fn format(&self) -> u16 {
        self.format
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn generation(&self) -> TermGeneration {
        self.generation
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub const fn product_digest(&self) -> DefinitionDigest {
        self.product_digest
    }
    pub fn grant(&self) -> &HouseGrantEconomicsV1 {
        &self.grant
    }
}
impl Record for HouseTermEconomicsV1 {
    const FAMILY: &'static str = "roll_mind_house_term_economics";
    const MAX_BYTES: usize = MAX_ECONOMICS_BYTES;
    fn key(&self) -> String {
        format!("{}/{}", self.licence.key(), self.generation.key())
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != self.grant.format() {
            return Err(invalid());
        }
        self.grant.validate()?;
        value(self)?;
        Ok(())
    }
}
