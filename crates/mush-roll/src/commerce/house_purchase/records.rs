//! Separate permanent births for the exact six-target entitlement protocol.
//! Decoding is not funding, a grant or proof of an economic journal entry.

use super::super::house_catalogue::HouseIssuer;
use super::HouseProtocol;
use mush_core::{CommitToken, DefinitionDigest, ObjId};
use mush_economy::managed_purchase::FundingReceipt;
use serde::{Deserialize, Serialize};

use super::super::paid::PaidTerms;
use super::super::purchase::{PurchaseParticipants, PurchasePhase};
use super::super::rating::RatingRef;
use super::super::records::{PublicationError, Record, RevisionPolicy};
use super::super::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration,
};
use super::HouseTermEconomicsV1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "AllocationWire")]
pub struct HouseAllocationV1 {
    pub(crate) format: u16,
    pub(crate) protocol: HouseProtocol,
    pub(crate) operation: CommitToken,
    pub(crate) participants: PurchaseParticipants,
    pub(crate) licence: LicenceId,
    pub(crate) term: TermGeneration,
    pub(crate) quote: CommitToken,
    pub(crate) quote_digest: DefinitionDigest,
    pub(crate) product_digest: DefinitionDigest,
    pub(crate) holder: ObjId,
    pub(crate) issuer: HouseIssuer,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) rating: RatingRef,
    pub(crate) terms: PaidTerms,
    pub(crate) economics: HouseTermEconomicsV1,
    pub(crate) funding: FundingReceipt,
    pub(crate) phase: PurchasePhase,
    pub(crate) updated_by: CommitToken,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllocationWire {
    format: u16,
    protocol: HouseProtocol,
    operation: CommitToken,
    participants: PurchaseParticipants,
    licence: LicenceId,
    term: TermGeneration,
    quote: CommitToken,
    quote_digest: DefinitionDigest,
    product_digest: DefinitionDigest,
    holder: ObjId,
    issuer: HouseIssuer,
    charter: CharterId,
    line: LineId,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    rating: RatingRef,
    terms: PaidTerms,
    economics: HouseTermEconomicsV1,
    funding: FundingReceipt,
    phase: PurchasePhase,
    updated_by: CommitToken,
}
impl TryFrom<AllocationWire> for HouseAllocationV1 {
    type Error = PublicationError;
    fn try_from(row: AllocationWire) -> Result<Self, Self::Error> {
        let value = Self {
            format: row.format,
            protocol: row.protocol,
            operation: row.operation,
            participants: row.participants,
            licence: row.licence,
            term: row.term,
            quote: row.quote,
            quote_digest: row.quote_digest,
            product_digest: row.product_digest,
            holder: row.holder,
            issuer: row.issuer,
            charter: row.charter,
            line: row.line,
            filed: row.filed,
            revision: row.revision,
            rating: row.rating,
            terms: row.terms,
            economics: row.economics,
            funding: row.funding,
            phase: row.phase,
            updated_by: row.updated_by,
        };
        value.validate()?;
        Ok(value)
    }
}
impl HouseAllocationV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        if bytes.is_empty() || bytes.len() > Self::MAX_BYTES {
            return Err(PublicationError::Capacity);
        }
        serde_json::from_slice(bytes).map_err(|_| PublicationError::Invalid)
    }
    pub const fn protocol(&self) -> HouseProtocol {
        self.protocol
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn participants(&self) -> &PurchaseParticipants {
        &self.participants
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn term(&self) -> TermGeneration {
        self.term
    }
    pub const fn quote(&self) -> CommitToken {
        self.quote
    }
    pub const fn quote_digest(&self) -> DefinitionDigest {
        self.quote_digest
    }
    pub const fn product_digest(&self) -> DefinitionDigest {
        self.product_digest
    }
    pub const fn holder(&self) -> ObjId {
        self.holder
    }
    pub const fn issuer(&self) -> HouseIssuer {
        self.issuer
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub const fn rating(&self) -> RatingRef {
        self.rating
    }
    pub const fn terms(&self) -> PaidTerms {
        self.terms
    }
    pub fn economics(&self) -> &HouseTermEconomicsV1 {
        &self.economics
    }
    pub fn funding(&self) -> &FundingReceipt {
        &self.funding
    }
    pub const fn phase(&self) -> PurchasePhase {
        self.phase
    }
    pub const fn updated_by(&self) -> CommitToken {
        self.updated_by
    }
}
impl Record for HouseAllocationV1 {
    const FAMILY: &'static str = "roll_mind_house_purchase";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.participants.validate()?;
        self.phase.validate(self.operation, self.updated_by)?;
        self.economics.validate()?;
        let economics = &self.economics;
        if self.format != 1
            || self.holder.0 == 0
            || self.term.get() != 1
            || self.quote == self.operation
            || self.terms.seats().get() != 1
            || self.terms.form() != (super::super::paid::PaidForm::Sealed {})
            || economics.licence() != self.licence
            || economics.generation() != self.term
            || economics.operation() != self.operation
            || economics.product_digest() != self.product_digest
            || economics.grant().renewal().terms() != self.terms
            || economics.grant().renewal().revision()
                != (RevisionPolicy::Pin {
                    filed: self.filed,
                    revision: self.revision,
                })
        {
            return Err(PublicationError::Mismatch);
        }
        let value = serde_json::to_value(self).map_err(|_| PublicationError::Invalid)?;
        mush_core::canonical_json_bytes_with_limit(&value, Self::MAX_BYTES)
            .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
