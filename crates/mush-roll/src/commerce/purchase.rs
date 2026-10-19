//! Permanent paid allocation identities shared with the closed Market builder.
//! These strict records are data. Decoding them never authorizes money, roots,
//! a grant, or a historical effect; actual registered participants do that.

use mush_core::{CommitToken, DefinitionRef, ObjId};
use mush_economy::managed_purchase::FundingReceipt;
use serde::{Deserialize, Serialize};

use super::paid::{PaidForm, PaidTerms};
use super::rating::RatingRef;
use super::records::{PublicationError, Record};
use super::{CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration};

/// Exact historical dependencies of the funded action. The manifest comes
/// from its frozen quote subject; these two digests cannot be regenerated from
/// today's configuration. This is not a caller-mintable participant handle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ParticipantsWire")]
pub struct PurchaseParticipants {
    economy: DefinitionRef,
    retention: DefinitionRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParticipantsWire {
    economy: DefinitionRef,
    retention: DefinitionRef,
}

impl TryFrom<ParticipantsWire> for PurchaseParticipants {
    type Error = PublicationError;
    fn try_from(wire: ParticipantsWire) -> Result<Self, Self::Error> {
        Self::new(wire.economy, wire.retention)
    }
}

impl PurchaseParticipants {
    /// Validate a pair of structural references. The registered action and
    /// its actual grants must separately match these references at funding.
    pub fn new(economy: DefinitionRef, retention: DefinitionRef) -> Result<Self, PublicationError> {
        let value = Self { economy, retention };
        value.validate()?;
        Ok(value)
    }
    pub fn economy(&self) -> &DefinitionRef {
        &self.economy
    }
    pub fn retention(&self) -> &DefinitionRef {
        &self.retention
    }
    pub fn ordered(&self) -> [DefinitionRef; 2] {
        [self.economy.clone(), self.retention.clone()]
    }
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        for (reference, (owner, id)) in [&self.economy, &self.retention]
            .into_iter()
            .zip([("economy", "initial_filing"), ("roll", "commercial_roots")])
        {
            if reference.key().owner.as_str() != owner
                || reference.key().kind.as_str() != "owner_participant"
                || reference.key().id.as_str() != id
                || reference.version() != 1
            {
                return Err(PublicationError::Invalid);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PurchasePhase {
    Funded {},
    Granted { commit: CommitToken },
    Refunded { commit: CommitToken },
}
impl PurchasePhase {
    pub fn validate(
        self,
        operation: CommitToken,
        updated_by: CommitToken,
    ) -> Result<(), PublicationError> {
        let expected = match self {
            Self::Funded {} => operation,
            Self::Granted { commit } | Self::Refunded { commit } => {
                if commit == operation {
                    return Err(PublicationError::Invalid);
                }
                commit
            }
        };
        if updated_by != expected {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

/// Permanent allocator birth, including refunded purchases. It explains the
/// shared licence ID even when no licence or term is ever granted. All new-row
/// constructors remain inside Roll's closed purchase writer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "AllocationWire")]
pub struct PaidAllocation {
    pub(crate) format: u16,
    pub(crate) operation: CommitToken,
    pub(crate) participants: PurchaseParticipants,
    pub(crate) licence: LicenceId,
    pub(crate) term: TermGeneration,
    pub(crate) quote: CommitToken,
    pub(crate) quote_digest: mush_core::DefinitionDigest,
    pub(crate) product_digest: mush_core::DefinitionDigest,
    pub(crate) holder: ObjId,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) rating: RatingRef,
    pub(crate) terms: PaidTerms,
    pub(crate) funding: FundingReceipt,
    pub(crate) phase: PurchasePhase,
    pub(crate) updated_by: CommitToken,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllocationWire {
    format: u16,
    operation: CommitToken,
    participants: PurchaseParticipants,
    licence: LicenceId,
    term: TermGeneration,
    quote: CommitToken,
    quote_digest: mush_core::DefinitionDigest,
    product_digest: mush_core::DefinitionDigest,
    holder: ObjId,
    charter: CharterId,
    line: LineId,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    rating: RatingRef,
    terms: PaidTerms,
    funding: FundingReceipt,
    phase: PurchasePhase,
    updated_by: CommitToken,
}
impl TryFrom<AllocationWire> for PaidAllocation {
    type Error = PublicationError;
    fn try_from(row: AllocationWire) -> Result<Self, Self::Error> {
        let value = Self {
            format: row.format,
            operation: row.operation,
            participants: row.participants,
            licence: row.licence,
            term: row.term,
            quote: row.quote,
            quote_digest: row.quote_digest,
            product_digest: row.product_digest,
            holder: row.holder,
            charter: row.charter,
            line: row.line,
            filed: row.filed,
            revision: row.revision,
            rating: row.rating,
            terms: row.terms,
            funding: row.funding,
            phase: row.phase,
            updated_by: row.updated_by,
        };
        value.validate()?;
        Ok(value)
    }
}

impl PaidAllocation {
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        if bytes.is_empty() || bytes.len() > Self::MAX_BYTES {
            return Err(PublicationError::Capacity);
        }
        // Typed-first parsing rejects duplicate known fields before a Value
        // could collapse them. This decoder is never a publication capability.
        serde_json::from_slice(bytes).map_err(|_| PublicationError::Invalid)
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
    pub const fn quote_digest(&self) -> mush_core::DefinitionDigest {
        self.quote_digest
    }
    pub const fn product_digest(&self) -> mush_core::DefinitionDigest {
        self.product_digest
    }
    pub const fn holder(&self) -> ObjId {
        self.holder
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

impl Record for PaidAllocation {
    const FAMILY: &'static str = "roll_mind_paid_purchase";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if !matches!(self.format, 1 | 2)
            || self.holder.0 == 0
            || self.term.get() != 1
            || self.quote == self.operation
            || self.terms.seats().get() != 1
            || (self.format == 1 && self.terms.form() != (PaidForm::Sealed {}))
        {
            return Err(PublicationError::Invalid);
        }
        self.participants.validate()?;
        self.phase.validate(self.operation, self.updated_by)
    }
}
