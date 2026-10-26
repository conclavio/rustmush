//! Strict data and immutable links. Decoding a record never supplies a grant.
use super::super::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermPhase,
    TermWindow, WorldIcSeconds,
    dependent_owner::OwnerParentBinding,
    filed::ParentContent,
    paid::PaidForm,
    personal_paid_holdings::PersonalPaidHolding,
    records::{FiledRecord, InitialFilingOperation, OwnerTerm, PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use serde::{Deserialize, Deserializer, Serialize};

fn invalid<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}

/// Immutable actual paid-term evidence selected by this replacement. Subsequent
/// current checks resolve that licence again, without changing this evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectedParent {
    pub(crate) licence: LicenceId,
    pub(crate) term: TermGeneration,
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) window: TermWindow,
    pub(crate) form: PaidForm,
    pub(crate) operation: CommitToken,
    pub(crate) terminal: CommitToken,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_holder_basis"
    )]
    pub(crate) holder_basis: Option<ParentHolderBasis>,
}

/// Versioned immutable attribution. Only an actual paid holding can capture it;
/// decoding is data validation and never grants current parent authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ParentHolderWire")]
pub(crate) struct ParentHolderBasis {
    format: u16,
    pub(crate) acceptance: CommitToken,
    pub(crate) ownership: mush_economy::managed_paid_resale::PaidHolderRefV1,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentHolderWire {
    format: u16,
    acceptance: CommitToken,
    ownership: mush_economy::managed_paid_resale::PaidHolderRefV1,
}
impl TryFrom<ParentHolderWire> for ParentHolderBasis {
    type Error = PublicationError;
    fn try_from(w: ParentHolderWire) -> Result<Self, Self::Error> {
        let value = Self {
            format: w.format,
            acceptance: w.acceptance,
            ownership: w.ownership,
        };
        value.validate()?;
        Ok(value)
    }
}
impl ParentHolderBasis {
    fn capture(value: &mush_economy::managed_paid_resale::RenewalHolderBasisV1) -> Self {
        Self {
            format: 1,
            acceptance: value.acceptance,
            ownership: value.ownership,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        self.ownership.validate().map_err(invalid)?;
        if self.format != 1 || self.ownership.generation < 2 {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}
/// Accepted source ownership at an actual parent-reader observation. This is
/// immutable attribution, not a current grant or a caller-supplied clock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ParentHolderCaptureWire")]
pub(crate) struct ParentHolderCapture {
    format: u16,
    pub(crate) basis: ParentHolderBasis,
    pub(crate) observed_ic: ClockStamp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentHolderCaptureWire {
    format: u16,
    basis: ParentHolderBasis,
    observed_ic: ClockStamp,
}
impl TryFrom<ParentHolderCaptureWire> for ParentHolderCapture {
    type Error = PublicationError;
    fn try_from(w: ParentHolderCaptureWire) -> Result<Self, Self::Error> {
        let value = Self {
            format: w.format,
            basis: w.basis,
            observed_ic: w.observed_ic,
        };
        value.validate()?;
        Ok(value)
    }
}
impl ParentHolderCapture {
    pub(crate) fn capture(actual: &super::super::parent_grant::CurrentParentGrant) -> Option<Self> {
        actual.holding().holder_basis().map(|basis| Self {
            format: 1,
            basis: ParentHolderBasis::capture(basis),
            observed_ic: actual.observed_ic().clone(),
        })
    }
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        self.basis.validate()?;
        WorldIcSeconds::from_stamp(&self.observed_ic).map_err(invalid)?;
        if self.format != 1 {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}
pub(crate) fn present_holder_capture<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<ParentHolderCapture>, D::Error> {
    ParentHolderCapture::deserialize(d).map(Some)
}

fn present_holder_basis<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<ParentHolderBasis>, D::Error> {
    ParentHolderBasis::deserialize(d).map(Some)
}
impl SelectedParent {
    pub(crate) fn capture(holding: &PersonalPaidHolding) -> Self {
        Self {
            licence: holding.licence(),
            term: holding.term(),
            filed: holding.filed(),
            revision: holding.identity(),
            window: holding.window(),
            form: holding.terms().form(),
            operation: holding.operation(),
            terminal: holding.terminal(),
            holder_basis: holding.holder_basis().map(ParentHolderBasis::capture),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        if let Some(basis) = &self.holder_basis {
            basis.validate()?;
        }
        if self.operation == self.terminal
            || !matches!(self.form, PaidForm::Open {} | PaidForm::OpenResale {})
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OperationWire")]
pub(crate) struct QualificationOperation {
    pub(crate) format: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) charter_ownership:
        Option<super::super::charter_transfer::records::CharterOwnershipRef>,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) manifest: DefinitionRef,
    pub(crate) owner: ObjId,
    pub(crate) owner_licence: LicenceId,
    pub(crate) owner_term: TermGeneration,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) child_revision: RevisionIdentity,
    pub(crate) birth_operation: CommitToken,
    pub(crate) title_revision: u64,
    pub(crate) source_parent: ParentContent,
    pub(crate) parent_charter: CharterId,
    pub(crate) parent_line: LineId,
    pub(crate) selected_parent: SelectedParent,
    pub(crate) ordinal: u64,
    pub(crate) previous: Option<CommitToken>,
    pub(crate) observed_ic: ClockStamp,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationWire {
    format: u16,
    #[serde(
        default,
        deserialize_with = "super::super::charter_transfer::records::present_ownership"
    )]
    charter_ownership: Option<super::super::charter_transfer::records::CharterOwnershipRef>,
    operation: CommitToken,
    action: DefinitionRef,
    manifest: DefinitionRef,
    owner: ObjId,
    owner_licence: LicenceId,
    owner_term: TermGeneration,
    charter: CharterId,
    line: LineId,
    filed: FiledRevisionId,
    child_revision: RevisionIdentity,
    birth_operation: CommitToken,
    title_revision: u64,
    source_parent: ParentContent,
    parent_charter: CharterId,
    parent_line: LineId,
    selected_parent: SelectedParent,
    ordinal: u64,
    #[serde(deserialize_with = "required_option")]
    previous: Option<CommitToken>,
    observed_ic: ClockStamp,
}
fn required_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<CommitToken>, D::Error> {
    Option::<CommitToken>::deserialize(d)
}
impl TryFrom<OperationWire> for QualificationOperation {
    type Error = PublicationError;
    fn try_from(w: OperationWire) -> Result<Self, Self::Error> {
        let value = Self {
            format: w.format,
            charter_ownership: w.charter_ownership,
            operation: w.operation,
            action: w.action,
            manifest: w.manifest,
            owner: w.owner,
            owner_licence: w.owner_licence,
            owner_term: w.owner_term,
            charter: w.charter,
            line: w.line,
            filed: w.filed,
            child_revision: w.child_revision,
            birth_operation: w.birth_operation,
            title_revision: w.title_revision,
            source_parent: w.source_parent,
            parent_charter: w.parent_charter,
            parent_line: w.parent_line,
            selected_parent: w.selected_parent,
            ordinal: w.ordinal,
            previous: w.previous,
            observed_ic: w.observed_ic,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Record for QualificationOperation {
    const FAMILY: &'static str = "roll_mind_owner_qualification_operation";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.selected_parent.validate()?;
        if let Some(ownership) = &self.charter_ownership {
            ownership.transferred()?;
        }
        let now = WorldIcSeconds::from_stamp(&self.observed_ic).map_err(invalid)?;
        if !matches!(
            (self.format, self.charter_ownership.is_some()),
            (1, false) | (2, true)
        ) || self.owner.0 == 0
            || self.title_revision == 0
            || self.owner_term.get() != 1
            || self.ordinal == 0
            || (self.ordinal == 1) != self.previous.is_none()
            || self.previous == Some(self.operation)
            || self.operation == self.birth_operation
            || self.owner_licence == self.selected_parent.licence
            || self.charter == self.parent_charter
            || self.line == self.parent_line
            || self.filed == self.source_parent.filed
            || self.filed == self.selected_parent.filed
            || (self.charter_ownership.is_none()
                && self.owner != self.child_revision.revision().key.character)
            || !matches!(self.observed_ic.clock, ClockRef::WorldIc { generation } if generation != 0)
            || self.action != super::protocol::action(&self.manifest)?
            || !matches!(
                self.selected_parent.window.phase_at(now),
                TermPhase::Active | TermPhase::Grace
            )
        {
            return Err(PublicationError::Invalid);
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(invalid)?,
            Self::MAX_BYTES,
        )
        .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
impl QualificationOperation {
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.owner_parent_qualification.cause.v1",
            &serde_json::to_value(self).map_err(invalid)?,
        )
        .map_err(invalid)
    }
    pub(crate) fn root(&self) -> Result<RequiredRoot, PublicationError> {
        Ok(RequiredRoot {
            anchor: RootAnchor::OwnerQualification(self.operation, self.digest()?),
            identity: self.selected_parent.revision,
        })
    }
    pub(crate) fn head(&self) -> QualificationHead {
        QualificationHead {
            format: 1,
            owner: self.owner,
            owner_licence: self.owner_licence,
            ordinal: self.ordinal,
            operation: self.operation,
        }
    }
    pub(crate) fn matches_birth(
        &self,
        binding: &OwnerParentBinding,
        filed: &FiledRecord,
        term: &OwnerTerm,
        birth: &InitialFilingOperation,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        binding.check_rows(filed, term, birth)?;
        if (self.charter_ownership.is_none() && self.owner != binding.parent.owner)
            || self.owner_licence != binding.licence
            || self.owner_term != binding.term
            || self.charter != binding.charter
            || self.line != birth.line
            || self.filed != binding.filed
            || self.child_revision != filed.content.revision()
            || self.birth_operation != binding.operation
            || self.source_parent != binding.parent.parent
            || self.parent_charter != binding.parent.charter
            || self.parent_line != binding.parent.line
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub(crate) fn follows(&self, prior: Option<&Self>) -> Result<(), PublicationError> {
        self.validate()?;
        match prior {
            None if self.ordinal == 1 && self.previous.is_none() => Ok(()),
            Some(old) => {
                old.validate()?;
                if self.previous != Some(old.operation)
                    || old.ordinal.checked_add(1) != Some(self.ordinal)
                    || self.owner != old.owner
                    || self.owner_licence != old.owner_licence
                    || self.owner_term != old.owner_term
                    || self.charter != old.charter
                    || self.line != old.line
                    || self.filed != old.filed
                    || self.child_revision != old.child_revision
                    || self.birth_operation != old.birth_operation
                    || self.source_parent != old.source_parent
                    || self.parent_charter != old.parent_charter
                    || self.parent_line != old.parent_line
                    || (self.observed_ic.clock == old.observed_ic.clock
                        && self.observed_ic.at < old.observed_ic.at)
                {
                    return Err(PublicationError::Mismatch);
                }
                Ok(())
            }
            _ => Err(PublicationError::Mismatch),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "HeadWire")]
pub(crate) struct QualificationHead {
    pub(crate) format: u16,
    pub(crate) owner: ObjId,
    pub(crate) owner_licence: LicenceId,
    pub(crate) ordinal: u64,
    pub(crate) operation: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadWire {
    format: u16,
    owner: ObjId,
    owner_licence: LicenceId,
    ordinal: u64,
    operation: CommitToken,
}
impl TryFrom<HeadWire> for QualificationHead {
    type Error = PublicationError;
    fn try_from(w: HeadWire) -> Result<Self, Self::Error> {
        let head = Self {
            format: w.format,
            owner: w.owner,
            owner_licence: w.owner_licence,
            ordinal: w.ordinal,
            operation: w.operation,
        };
        head.validate()?;
        Ok(head)
    }
}
impl QualificationHead {
    pub(crate) fn key_for(licence: LicenceId, owner: ObjId) -> String {
        OwnerParentBinding::key_for(licence, owner)
    }
    pub(crate) fn matches(
        &self,
        operation: &QualificationOperation,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        operation.validate()?;
        if self != &operation.head() {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
impl Record for QualificationHead {
    const FAMILY: &'static str = "roll_mind_owner_qualification_head";
    const MAX_BYTES: usize = 2048;
    fn key(&self) -> String {
        Self::key_for(self.owner_licence, self.owner)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1 || self.owner.0 == 0 || self.ordinal == 0 {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}
