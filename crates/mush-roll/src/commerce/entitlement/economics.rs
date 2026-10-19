//! Immutable acquisition economics remain distinct from current qualification.
//! Old paid terms are never decoded through these new required records.

use mush_core::{CommitToken, DefinitionDigest, DefinitionRef};
use mush_economy::{Amount, InstrumentId};
use serde::{Deserialize, Deserializer, Serialize};

use super::super::filed::{MAX_PARENT_DEPTH, ParentContent};
use super::super::paid::{PaidForm, PaidTerms};
use super::super::records::{PublicationError, Record, RevisionPolicy};
use super::super::{
    CharterId, FiledRevisionId, LicenceId, LineId, MAX_GRACE_SECONDS, MAX_TERM_SECONDS,
    RevisionIdentity, TermGeneration,
};

pub const ECONOMICS_FORMAT: u16 = 1;
pub const MAX_ECONOMICS_BYTES: usize = 8192;
pub const MAX_ROYALTY_PERMILLE: u16 = 300;

fn invalid() -> PublicationError {
    PublicationError::Invalid
}
fn bounded<T: Serialize>(value: &T) -> Result<serde_json::Value, PublicationError> {
    let value = serde_json::to_value(value).map_err(|_| invalid())?;
    mush_core::canonical_json_bytes_with_limit(&value, MAX_ECONOMICS_BYTES)
        .map_err(|_| PublicationError::Capacity)?;
    Ok(value)
}
fn digest<T: Serialize>(domain: &str, value: &T) -> Result<DefinitionDigest, PublicationError> {
    DefinitionDigest::for_canonical_value(domain, &bounded(value)?).map_err(|_| invalid())
}
fn policy_valid(value: &DefinitionRef) -> bool {
    value.key().owner.as_str() == "market"
        && value.key().kind.as_str() == "entitlement_policy"
        && value.key().id.as_str() == "personal"
        && value.version() == 1
}

/// Explicit consent data. Open source does not imply permission to resell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DownstreamTermsV1 {
    Sealed {},
    Source { royalty_permille: u16 },
}
impl DownstreamTermsV1 {
    pub fn validate(self, form: PaidForm) -> Result<(), PublicationError> {
        match (self, form) {
            (Self::Sealed {}, PaidForm::Sealed {}) => Ok(()),
            (Self::Source { royalty_permille }, PaidForm::Open {} | PaidForm::OpenResale {})
                if royalty_permille <= MAX_ROYALTY_PERMILLE =>
            {
                Ok(())
            }
            _ => Err(invalid()),
        }
    }
    pub const fn royalty_permille(self) -> Option<u16> {
        match self {
            Self::Sealed {} => None,
            Self::Source { royalty_permille } => Some(royalty_permille),
        }
    }
}

macro_rules! record {
    ($name:ident, $wire:ident, {$($field:ident : $ty:ty),* $(,)?}) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        pub struct $name { $(pub(crate) $field: $ty),* }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire { $($field: $ty),* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:Deserializer<'de>>(d:D)->Result<Self,D::Error> {
                let row=$wire::deserialize(d)?;
                let value=Self { $($field:row.$field),* };
                value.validate().map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    };
}

record!(RenewalTemplateV1, RenewalWire, {
    format:u16, instrument:InstrumentId, price:Amount,
    term_seconds:i64, grace_seconds:i64,
    terms:PaidTerms, revision:RevisionPolicy, policy:DefinitionRef,
});
impl RenewalTemplateV1 {
    /// Structural terms only. No renewal contract, charge or scheduling occurs.
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
        let value = Self {
            format: ECONOMICS_FORMAT,
            instrument,
            price,
            term_seconds,
            grace_seconds,
            terms,
            revision,
            policy,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), PublicationError> {
        if self.format != ECONOMICS_FORMAT
            || self.price.is_negative()
            || self.price.is_zero()
            || !(1..=MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&self.grace_seconds)
            || self.terms.seats().get() != 1
            || !matches!(self.revision, RevisionPolicy::Pin { .. })
            || !policy_valid(&self.policy)
        {
            return Err(invalid());
        }
        bounded(self)?;
        Ok(())
    }
    pub fn matches_product(
        &self,
        terms: PaidTerms,
        revision: RevisionPolicy,
        instrument: &InstrumentId,
        policy: &DefinitionRef,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        if self.terms != terms
            || self.revision != revision
            || &self.instrument != instrument
            || &self.policy != policy
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoyaltyChainRefV1 {
    pub(crate) filed: FiledRevisionId,
    pub(crate) digest: DefinitionDigest,
}
impl RoyaltyChainRefV1 {
    pub const fn filed(self) -> FiledRevisionId {
        self.filed
    }
    pub const fn digest(self) -> DefinitionDigest {
        self.digest
    }
}

record!(GrantEconomicsV1, GrantWire, {
    format:u16, downstream:DownstreamTermsV1, renewal:RenewalTemplateV1,
    royalty_chain:RoyaltyChainRefV1,
});
impl GrantEconomicsV1 {
    pub fn new(
        downstream: DownstreamTermsV1,
        renewal: RenewalTemplateV1,
        royalty_chain: RoyaltyChainRefV1,
    ) -> Result<Self, PublicationError> {
        let value = Self {
            format: ECONOMICS_FORMAT,
            downstream,
            renewal,
            royalty_chain,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), PublicationError> {
        self.renewal.validate()?;
        self.downstream.validate(self.renewal.terms.form())?;
        if self.format != ECONOMICS_FORMAT
            || !matches!(self.renewal.revision, RevisionPolicy::Pin {filed,..} if filed==self.royalty_chain.filed)
        {
            return Err(invalid());
        }
        bounded(self)?;
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.mind.grant_economics.v1", self)
    }
    pub const fn downstream(&self) -> DownstreamTermsV1 {
        self.downstream
    }
    pub fn renewal(&self) -> &RenewalTemplateV1 {
        &self.renewal
    }
    pub const fn royalty_chain(&self) -> RoyaltyChainRefV1 {
        self.royalty_chain
    }
}

record!(PaidTermEconomicsV1, TermWire, {
    format:u16, licence:LicenceId, generation:TermGeneration,
    operation:CommitToken, product_digest:DefinitionDigest, grant:GrantEconomicsV1,
});
impl PaidTermEconomicsV1 {
    pub(crate) fn capture(
        licence: LicenceId,
        generation: TermGeneration,
        operation: CommitToken,
        product_digest: DefinitionDigest,
        grant: GrantEconomicsV1,
    ) -> Result<Self, PublicationError> {
        let value = Self {
            format: ECONOMICS_FORMAT,
            licence,
            generation,
            operation,
            product_digest,
            grant,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        if bytes.is_empty() || bytes.len() > MAX_ECONOMICS_BYTES {
            return Err(PublicationError::Capacity);
        }
        serde_json::from_slice(bytes).map_err(|_| invalid())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.mind.paid_term_economics.v1", self)
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
    pub fn grant(&self) -> &GrantEconomicsV1 {
        &self.grant
    }
}
impl Record for PaidTermEconomicsV1 {
    const FAMILY: &'static str = "roll_mind_paid_term_economics";
    const MAX_BYTES: usize = MAX_ECONOMICS_BYTES;
    fn key(&self) -> String {
        format!("{}/{}", self.licence.key(), self.generation.key())
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != ECONOMICS_FORMAT {
            return Err(invalid());
        }
        self.grant.validate()?;
        bounded(self)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcquisitionV1 {
    pub(crate) licence: LicenceId,
    pub(crate) term: TermGeneration,
    pub(crate) operation: CommitToken,
    pub(crate) product_digest: DefinitionDigest,
}
impl AcquisitionV1 {
    pub const fn licence(self) -> LicenceId {
        self.licence
    }
    pub const fn term(self) -> TermGeneration {
        self.term
    }
    pub const fn operation(self) -> CommitToken {
        self.operation
    }
    pub const fn product_digest(self) -> DefinitionDigest {
        self.product_digest
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RoyaltyBasisV1 {
    ExplicitTermEconomicsV1 { digest: DefinitionDigest },
    LegacySourceFormsV2Zero { effect: DefinitionRef },
}
impl RoyaltyBasisV1 {
    fn validate(&self, rate: u16) -> Result<(), PublicationError> {
        if let Self::LegacySourceFormsV2Zero { effect } = self
            && (rate != 0
                || effect.key().owner.as_str() != "market"
                || effect.key().kind.as_str() != "action"
                || effect.key().id.as_str() != "purchase_original_mind"
                || effect.version() != 2)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
record!(RoyaltyEdgeV1, EdgeWire, {
    parent:ParentContent,charter:CharterId,line:LineId,
    acquisition:AcquisitionV1,royalty_permille:u16,basis:RoyaltyBasisV1,
});
impl RoyaltyEdgeV1 {
    fn validate(&self) -> Result<(), PublicationError> {
        if self.royalty_permille > MAX_ROYALTY_PERMILLE {
            return Err(invalid());
        }
        self.basis.validate(self.royalty_permille)?;
        bounded(self)?;
        Ok(())
    }
    pub const fn parent(&self) -> ParentContent {
        self.parent
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
    pub const fn acquisition(&self) -> AcquisitionV1 {
        self.acquisition
    }
    pub const fn royalty_permille(&self) -> u16 {
        self.royalty_permille
    }
    pub fn basis(&self) -> &RoyaltyBasisV1 {
        &self.basis
    }
}

/// Bounds retained array allocation even when a parent record is decoded as
/// part of a larger envelope. The actual document owner also preflights bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct Edges(Vec<RoyaltyEdgeV1>);
impl<'de> Deserialize<'de> for Edges {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Edges;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("at most three exact acquisition royalty edges")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Edges, A::Error> {
                let mut values = Vec::with_capacity(MAX_PARENT_DEPTH);
                while let Some(value) = seq.next_element()? {
                    if values.len() == MAX_PARENT_DEPTH {
                        return Err(serde::de::Error::custom("too many royalty edges"));
                    }
                    values.push(value);
                }
                Ok(Edges(values))
            }
        }
        d.deserialize_seq(Visitor)
    }
}
record!(FiledRoyaltyTermsV1, FiledWire, {
    format:u16,filed:FiledRevisionId,revision:RevisionIdentity,
    publication:CommitToken,edges:Edges,
});
impl FiledRoyaltyTermsV1 {
    pub(crate) fn capture(
        filed: FiledRevisionId,
        revision: RevisionIdentity,
        publication: CommitToken,
        edges: Vec<RoyaltyEdgeV1>,
    ) -> Result<Self, PublicationError> {
        let value = Self {
            format: ECONOMICS_FORMAT,
            filed,
            revision,
            publication,
            edges: Edges(edges),
        };
        value.validate()?;
        Ok(value)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        if bytes.is_empty() || bytes.len() > MAX_ECONOMICS_BYTES {
            return Err(PublicationError::Capacity);
        }
        serde_json::from_slice(bytes).map_err(|_| invalid())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        digest("roll.mind.filed_royalties.v1", self)
    }
    pub fn reference(&self) -> Result<RoyaltyChainRefV1, PublicationError> {
        Ok(RoyaltyChainRefV1 {
            filed: self.filed,
            digest: self.digest()?,
        })
    }
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub const fn publication(&self) -> CommitToken {
        self.publication
    }
    pub fn edges(&self) -> &[RoyaltyEdgeV1] {
        &self.edges.0
    }
    pub fn total_permille(&self) -> Result<u16, PublicationError> {
        self.edges.0.iter().try_fold(0_u16, |sum, edge| {
            sum.checked_add(edge.royalty_permille).ok_or_else(invalid)
        })
    }
    pub fn check_next_resale(&self, rate: u16) -> Result<(), PublicationError> {
        self.validate()?;
        if self.edges.0.len() >= MAX_PARENT_DEPTH
            || rate > MAX_ROYALTY_PERMILLE
            || self
                .total_permille()?
                .checked_add(rate)
                .is_none_or(|sum| sum > MAX_ROYALTY_PERMILLE)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
impl Record for FiledRoyaltyTermsV1 {
    const FAMILY: &'static str = "roll_mind_filed_royalty_terms";
    const MAX_BYTES: usize = MAX_ECONOMICS_BYTES;
    fn key(&self) -> String {
        self.filed.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != ECONOMICS_FORMAT
            || self.edges.0.len() > MAX_PARENT_DEPTH
            || self.total_permille()? > MAX_ROYALTY_PERMILLE
        {
            return Err(invalid());
        }
        let mut files = std::collections::BTreeSet::from([self.filed]);
        let mut charters = std::collections::BTreeSet::new();
        let mut revisions = std::collections::BTreeSet::from([self.revision.revision().key]);
        for edge in &self.edges.0 {
            edge.validate()?;
            if !files.insert(edge.parent.filed)
                || !charters.insert(edge.charter)
                || !revisions.insert(edge.parent.revision.revision().key)
            {
                return Err(invalid());
            }
        }
        bounded(self)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
