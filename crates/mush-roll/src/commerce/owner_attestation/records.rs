//! Immutable owner-generation evidence. These records contain no capability.
use super::super::{
    CharterId, FiledRevisionId, LicenceId, LineId, MAX_GRACE_SECONDS, MAX_TERM_SECONDS,
    RevisionIdentity, TermGeneration, TermWindow, WorldIcSeconds,
    filed::ParentContent,
    owner_qualification::records::SelectedParent,
    rating::RatingRef,
    records::{InitialFilingOperation, OwnerTerm, PublicationError, Record, RevisionPolicy},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use serde::{Deserialize, Deserializer, Serialize};

fn invalid<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParentQualification {
    pub(crate) source: ParentContent,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) selected: SelectedParent,
}

/// Separate from the original DependentParent: a replacement grant can have a
/// different revision on the same parent line without rewriting source ancestry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttestationEvidence {
    pub(crate) term_seconds: i64,
    pub(crate) grace_seconds: i64,
    #[serde(deserialize_with = "required_parent")]
    pub(crate) parent: Option<ParentQualification>,
}
fn required_parent<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<ParentQualification>, D::Error> {
    Option::<ParentQualification>::deserialize(d)
}
impl AttestationEvidence {
    pub(crate) fn window(&self, issued: WorldIcSeconds) -> Result<TermWindow, PublicationError> {
        if !(1..=MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&self.grace_seconds)
        {
            return Err(PublicationError::Invalid);
        }
        if let Some(parent) = &self.parent {
            parent.selected.validate()?;
            super::super::rights::dependent_owner_window(
                issued,
                self.term_seconds,
                self.grace_seconds,
                parent.selected.window,
            )
            .map_err(invalid)
        } else {
            let end = issued
                .0
                .checked_add(self.term_seconds)
                .ok_or(PublicationError::Invalid)?;
            let grace = end
                .checked_add(self.grace_seconds)
                .ok_or(PublicationError::Invalid)?;
            TermWindow::new(issued, issued, WorldIcSeconds(end), WorldIcSeconds(grace))
                .map_err(invalid)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OperationWire")]
pub(crate) struct AttestationOperation {
    pub(crate) format: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) charter_ownership:
        Option<super::super::charter_transfer::records::CharterOwnershipRef>,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) manifest: DefinitionRef,
    pub(crate) owner: ObjId,
    pub(crate) licence: LicenceId,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) birth_operation: CommitToken,
    pub(crate) predecessor: TermGeneration,
    pub(crate) predecessor_operation: CommitToken,
    pub(crate) predecessor_digest: DefinitionDigest,
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) rating: RatingRef,
    pub(crate) title_revision: u64,
    pub(crate) observed_ic: ClockStamp,
    pub(crate) observed_utc: i64,
    pub(crate) next: OwnerTerm,
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
    licence: LicenceId,
    charter: CharterId,
    line: LineId,
    birth_operation: CommitToken,
    predecessor: TermGeneration,
    predecessor_operation: CommitToken,
    predecessor_digest: DefinitionDigest,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    rating: RatingRef,
    title_revision: u64,
    observed_ic: ClockStamp,
    observed_utc: i64,
    next: OwnerTerm,
}
impl TryFrom<OperationWire> for AttestationOperation {
    type Error = PublicationError;
    fn try_from(w: OperationWire) -> Result<Self, Self::Error> {
        let v = Self {
            format: w.format,
            charter_ownership: w.charter_ownership,
            operation: w.operation,
            action: w.action,
            manifest: w.manifest,
            owner: w.owner,
            licence: w.licence,
            charter: w.charter,
            line: w.line,
            birth_operation: w.birth_operation,
            predecessor: w.predecessor,
            predecessor_operation: w.predecessor_operation,
            predecessor_digest: w.predecessor_digest,
            filed: w.filed,
            revision: w.revision,
            rating: w.rating,
            title_revision: w.title_revision,
            observed_ic: w.observed_ic,
            observed_utc: w.observed_utc,
            next: w.next,
        };
        v.validate()?;
        Ok(v)
    }
}
impl Record for AttestationOperation {
    const FAMILY: &'static str = "roll_mind_owner_attestation_operation";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.next.validate()?;
        if let Some(ownership) = &self.charter_ownership {
            ownership.transferred()?;
        }
        let at = WorldIcSeconds::from_stamp(&self.observed_ic).map_err(invalid)?;
        let evidence = self
            .next
            .attestation
            .as_ref()
            .ok_or(PublicationError::Missing)?;
        if !matches!(
            (self.format, self.charter_ownership.is_some()),
            (1, false) | (2, true)
        ) || self.owner.0 == 0
            || self.title_revision == 0
            || (self.charter_ownership.is_none()
                && self.owner != self.revision.revision().key.character)
            || self.operation == self.birth_operation
            || self.operation == self.predecessor_operation
            || self.predecessor.get().checked_add(1) != Some(self.next.generation.get())
            || self.next.licence != self.licence
            || self.next.operation != self.operation
            || self.next.revision != (RevisionPolicy::Follow { line: self.line })
            || self.next.window.issued_at() != at
            || !matches!(self.observed_ic.clock,ClockRef::WorldIc {generation} if generation!=0)
            || self.action
                != super::protocol::action(
                    &self.manifest,
                    evidence.term_seconds,
                    evidence.grace_seconds,
                )?
        {
            return Err(PublicationError::Invalid);
        }
        if let Some(parent) = &evidence.parent {
            if parent.charter == self.charter
                || parent.line == self.line
                || parent.selected.licence == self.licence
                || parent.source.filed == self.filed
                || parent.selected.filed == self.filed
            {
                return Err(PublicationError::Mismatch);
            }
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(invalid)?,
            Self::MAX_BYTES,
        )
        .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
pub(crate) fn term_digest(term: &OwnerTerm) -> Result<DefinitionDigest, PublicationError> {
    term.validate()?;
    DefinitionDigest::for_canonical_value(
        "roll.owner_term.v1",
        &serde_json::to_value(term).map_err(invalid)?,
    )
    .map_err(invalid)
}
impl AttestationOperation {
    pub(crate) fn follows(
        &self,
        old: &OwnerTerm,
        birth: &InitialFilingOperation,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        old.validate()?;
        birth.validate()?;
        if self.licence != old.licence
            || self.predecessor != old.generation
            || self.predecessor_operation != old.operation
            || self.predecessor_digest != term_digest(old)?
            || self.birth_operation != birth.commit
            || (self.charter_ownership.is_none() && self.owner != birth.requester)
            || self.licence != birth.licence
            || self.charter != birth.charter
            || self.line != birth.line
            || birth.economic.house.is_some()
            || birth.economic.issued_owner != mush_org::PrincipalId::Object(birth.requester)
            || old.revision != (RevisionPolicy::Follow { line: self.line })
            || self.next.window.issued_at() < old.window.term_end()
            || self.next.window.grace_end() <= old.window.grace_end()
        {
            return Err(PublicationError::Mismatch);
        }
        let parent = self
            .next
            .attestation
            .as_ref()
            .ok_or(PublicationError::Missing)?
            .parent
            .as_ref();
        match (parent, birth.parent.as_ref()) {
            (None, None) => {}
            (Some(p), Some(b))
                if p.source == b.parent && p.charter == b.charter && p.line == b.line => {}
            _ => return Err(PublicationError::Mismatch),
        }
        Ok(())
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.owner_attestation.cause.v1",
            &serde_json::to_value(self).map_err(invalid)?,
        )
        .map_err(invalid)
    }
    pub(crate) fn roots(&self) -> Result<Vec<RequiredRoot>, PublicationError> {
        let digest = self.digest()?;
        let mut roots = vec![RequiredRoot {
            anchor: RootAnchor::OwnerAttestation(self.operation, digest),
            identity: self.revision,
        }];
        if let Some(parent) = self
            .next
            .attestation
            .as_ref()
            .and_then(|e| e.parent.as_ref())
        {
            roots.push(RequiredRoot {
                anchor: RootAnchor::OwnerAttestationParent(self.operation, digest),
                identity: parent.selected.revision,
            });
        }
        Ok(roots)
    }
}
