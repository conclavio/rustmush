//! Structural records for the first personal paid-licence product.
//!
//! These codecs allocate nothing and grant nothing. No family, sale service or
//! boot readiness is registered here. A later closed sale builder must join the
//! actual filed line/rating, economic receipt, current authority and retained
//! roots before publishing either record. Registry revocation/recall, pending
//! transfer and claims are separate required state, not omitted admission tests.
//! Organisation holders require their real scoped warrant path; this personal
//! schema cannot silently stand in for that path.

use mush_core::{CommitToken, ObjId};
use mush_economy::managed_paid_resale::{PaidBirthRefV1, PaidHolderRefV1};
use serde::{Deserialize, Serialize};

use super::rating::RatingClass;
use super::records::{PublicationError, Record, RevisionPolicy};
use super::{CharterId, LicenceId, LineId, PaidSeats, TermGeneration, TermWindow};

pub const PAID_RECORD_FORMAT: u16 = 1;
pub const MAX_PAID_LICENCE_BYTES: usize = 4 * 1024;
pub const MAX_PAID_TERM_BYTES: usize = 8 * 1024;

/// Negotiated source form, not a current source-access capability. In
/// particular OpenResale concerns listing a derivative, whereas transferability
/// below concerns selling this existing licence. The two facts are independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PaidForm {
    // Empty struct variants make nested unknown fields fail with pinned serde.
    Sealed {},
    Open {},
    OpenResale {},
}

/// Immutable consent facts for one paid term. This is never OwnedDispatch;
/// claim accounting must separately enforce the purchased concurrent-seat cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaidTerms {
    seats: PaidSeats,
    class: RatingClass,
    form: PaidForm,
    transferable: bool,
}
impl PaidTerms {
    pub(crate) fn original_personal(
        class: RatingClass,
        transferable: bool,
    ) -> Result<Self, PublicationError> {
        Self::original_personal_form(class, PaidForm::Sealed {}, transferable)
    }
    pub(crate) fn original_personal_form(
        class: RatingClass,
        form: PaidForm,
        transferable: bool,
    ) -> Result<Self, PublicationError> {
        Ok(Self {
            seats: PaidSeats::new(1).map_err(|_| PublicationError::Invalid)?,
            class,
            form,
            transferable,
        })
    }
    /// Structural consent for the explicitly versioned HouseV2 sealed Pin.
    /// This constructs no licence, paid history, or current admission authority.
    pub(crate) fn house_two_seat_pin(
        class: RatingClass,
        transferable: bool,
    ) -> Result<Self, PublicationError> {
        Ok(Self {
            seats: PaidSeats::new(2).map_err(|_| PublicationError::Invalid)?,
            class,
            form: PaidForm::Sealed {},
            transferable,
        })
    }
    #[must_use]
    pub const fn seats(self) -> PaidSeats {
        self.seats
    }
    #[must_use]
    pub const fn class(self) -> RatingClass {
        self.class
    }
    #[must_use]
    pub const fn form(self) -> PaidForm {
        self.form
    }
    #[must_use]
    pub const fn transferable(self) -> bool {
        self.transferable
    }
}

/// Current holder/term pointer for a personal paid licence. Holder changes do
/// not rewrite an issued term or an already captured sortie warrant. The holder
/// identifies a personal object principal, not an account id or title-backed
/// licence object. The service must still prove that principal is a character.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PersonalPaidLicenceWire")]
pub struct PersonalPaidLicence {
    format: u16,
    id: LicenceId,
    charter: CharterId,
    line: LineId,
    holder: ObjId,
    term: TermGeneration,
    operation: CommitToken,
    #[serde(skip_serializing_if = "Option::is_none")]
    ownership: Option<PaidHolderRefV1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonalPaidLicenceWire {
    format: u16,
    id: LicenceId,
    charter: CharterId,
    line: LineId,
    holder: ObjId,
    term: TermGeneration,
    operation: CommitToken,
    #[serde(default, deserialize_with = "present_paid_ownership")]
    ownership: Option<Option<PaidHolderRefV1>>,
}
fn present_paid_ownership<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<PaidHolderRefV1>>, D::Error> {
    Option::<PaidHolderRefV1>::deserialize(d).map(Some)
}
impl TryFrom<PersonalPaidLicenceWire> for PersonalPaidLicence {
    type Error = PublicationError;
    fn try_from(row: PersonalPaidLicenceWire) -> Result<Self, Self::Error> {
        let ownership = match (row.format, row.ownership) {
            (1, None) => None,
            (2, Some(Some(reference))) if reference.generation >= 2 => Some(reference),
            _ => return Err(PublicationError::Invalid),
        };
        let value = Self {
            format: row.format,
            id: row.id,
            charter: row.charter,
            line: row.line,
            holder: row.holder,
            term: row.term,
            operation: row.operation,
            ownership,
        };
        value.validate()?;
        Ok(value)
    }
}
impl PersonalPaidLicence {
    pub(crate) fn original(
        id: LicenceId,
        charter: CharterId,
        line: LineId,
        holder: ObjId,
        term: TermGeneration,
        operation: CommitToken,
    ) -> Result<Self, PublicationError> {
        let value = Self {
            format: PAID_RECORD_FORMAT,
            id,
            charter,
            line,
            holder,
            term,
            operation,
            ownership: None,
        };
        value.validate()?;
        Ok(value)
    }
    /// Structural next-term pointer. The actual renewal owner separately proves
    /// the granted generation; this preserves an existing transferred holder.
    /// Structural projection for an already proved historical acceptance.
    /// Current publication must retain the actual pointer and owner fence.
    pub(crate) fn with_captured_ownership(
        mut self,
        ownership: Option<mush_economy::managed_paid_resale::PaidHolderRefV1>,
    ) -> Result<Self, PublicationError> {
        self.format = if ownership.is_some() { 2 } else { 1 };
        self.ownership = ownership;
        self.validate()?;
        Ok(self)
    }
    pub(crate) fn with_next_term(&self, term: TermGeneration) -> Result<Self, PublicationError> {
        self.validate()?;
        if self.term.get().checked_add(1) != Some(term.get()) {
            return Err(PublicationError::Mismatch);
        }
        let next = Self {
            term,
            ..self.clone()
        };
        next.validate()?;
        Ok(next)
    }
    /// Exact structural transfer relation, never a source of publication authority.
    /// The writer must obtain this holder row from the actual opaque E4 output.
    pub(crate) fn transferred(
        &self,
        birth: &PaidBirthRefV1,
        row: &super::paid_resale::PaidHolderRecord,
    ) -> Result<Self, PublicationError> {
        self.validate()?;
        birth.validate().map_err(|_| PublicationError::Invalid)?;
        row.validate()?;
        let before = self.ownership.unwrap_or(PaidHolderRefV1 {
            generation: 1,
            operation: self.operation,
            digest: birth.digest,
        });
        if birth.operation != self.operation
            || row.birth() != birth
            || row.licence() != self.id
            || row.predecessor() != &before
            || row.holder() == self.holder
            || row.term_generation() != self.term
        {
            return Err(PublicationError::Mismatch);
        }
        let next = Self {
            format: 2,
            holder: row.holder(),
            ownership: Some(row.ownership()?),
            ..self.clone()
        };
        next.validate()?;
        Ok(next)
    }
    #[must_use]
    pub fn ownership(&self) -> Option<&PaidHolderRefV1> {
        self.ownership.as_ref()
    }
    /// Bounded data decoder. A decoded row is not proof of payment or authority.
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        if bytes.is_empty() || bytes.len() > MAX_PAID_LICENCE_BYTES {
            return Err(PublicationError::Capacity);
        }
        serde_json::from_slice(bytes).map_err(|_| PublicationError::Invalid)
    }
    #[must_use]
    pub const fn id(&self) -> LicenceId {
        self.id
    }
    #[must_use]
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    #[must_use]
    pub const fn line(&self) -> LineId {
        self.line
    }
    #[must_use]
    pub const fn holder(&self) -> ObjId {
        self.holder
    }
    #[must_use]
    pub const fn term(&self) -> TermGeneration {
        self.term
    }
    #[must_use]
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
}
impl Record for PersonalPaidLicence {
    const FAMILY: &'static str = "roll_mind_personal_paid_licence";
    const MAX_BYTES: usize = MAX_PAID_LICENCE_BYTES;
    fn key(&self) -> String {
        self.id.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.holder.0 == 0 || !matches!((self.format, &self.ownership), (1, None) | (2, Some(_)))
        {
            return Err(PublicationError::Invalid);
        }
        if let Some(reference) = &self.ownership {
            reference
                .validate()
                .map_err(|_| PublicationError::Invalid)?;
            if reference.generation < 2 || reference.operation == self.operation {
                return Err(PublicationError::Invalid);
            }
        }
        Ok(())
    }
}

/// Immutable terms selected by a licence's current generation. Historical
/// generations remain for exact receipt/claim joins; a current holder transfer
/// must not reset these dates. The operation is a reference to the real future
/// closed paid-operation record, never a fabricated economic proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PaidTermWire")]
pub struct PaidTermRecord {
    format: u16,
    licence: LicenceId,
    generation: TermGeneration,
    window: TermWindow,
    terms: PaidTerms,
    revision: RevisionPolicy,
    operation: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaidTermWire {
    format: u16,
    licence: LicenceId,
    generation: TermGeneration,
    window: TermWindow,
    terms: PaidTerms,
    revision: RevisionPolicy,
    operation: CommitToken,
}
impl TryFrom<PaidTermWire> for PaidTermRecord {
    type Error = PublicationError;
    fn try_from(row: PaidTermWire) -> Result<Self, Self::Error> {
        let value = Self {
            format: row.format,
            licence: row.licence,
            generation: row.generation,
            window: row.window,
            terms: row.terms,
            revision: row.revision,
            operation: row.operation,
        };
        value.validate()?;
        Ok(value)
    }
}
impl PaidTermRecord {
    pub(crate) fn original(
        licence: LicenceId,
        generation: TermGeneration,
        window: TermWindow,
        terms: PaidTerms,
        revision: RevisionPolicy,
        operation: CommitToken,
    ) -> Result<Self, PublicationError> {
        let value = Self {
            format: PAID_RECORD_FORMAT,
            licence,
            generation,
            window,
            terms,
            revision,
            operation,
        };
        value.validate()?;
        Ok(value)
    }
    /// Bounded data decoder; no grant, rating or source eligibility is inferred.
    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        if bytes.is_empty() || bytes.len() > MAX_PAID_TERM_BYTES {
            return Err(PublicationError::Capacity);
        }
        serde_json::from_slice(bytes).map_err(|_| PublicationError::Invalid)
    }
    #[must_use]
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    #[must_use]
    pub const fn generation(&self) -> TermGeneration {
        self.generation
    }
    #[must_use]
    pub const fn window(&self) -> TermWindow {
        self.window
    }
    #[must_use]
    pub const fn terms(&self) -> PaidTerms {
        self.terms
    }
    #[must_use]
    pub const fn policy(&self) -> RevisionPolicy {
        self.revision
    }
    #[must_use]
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }

    /// Only local pointer consistency. A Pin still needs the actual retained
    /// filed revision's line/rating check; a Follow needs live line resolution.
    /// Current Registry status, payment, holder authority and claims are absent
    /// from these two rows and must be verified by the closed owner service.
    pub fn matches_current(&self, licence: &PersonalPaidLicence) -> Result<(), PublicationError> {
        self.validate()?;
        licence.validate()?;
        if self.licence != licence.id || self.generation != licence.term {
            return Err(PublicationError::Mismatch);
        }
        if let RevisionPolicy::Follow { line } = self.revision
            && line != licence.line
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
impl Record for PaidTermRecord {
    const FAMILY: &'static str = "roll_mind_paid_term";
    const MAX_BYTES: usize = MAX_PAID_TERM_BYTES;
    fn key(&self) -> String {
        format!("{}/{}", self.licence.key(), self.generation.key())
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != PAID_RECORD_FORMAT {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
