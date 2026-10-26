//! Immutable personal-owner parent selection and its historical term evidence.
//!
//! A decoded row is never current entitlement. The live reader must resolve the
//! selected owner's actual paid grant again; the boot audit checks its retained
//! history without requiring that grant to remain eligible today.

use mush_core::{CommitToken, ObjId};
use serde::{Deserialize, Serialize};

use super::filed::ParentContent;
use super::parent_grant::CurrentParentGrant;
use super::records::{FiledRecord, InitialFilingOperation, OwnerTerm, PublicationError, Record};
use super::{
    CharterId, FiledRevisionId, LicenceId, LineId, MAX_GRACE_SECONDS, MAX_TERM_SECONDS,
    TermGeneration, TermWindow, WorldIcSeconds,
};

/// Exact immutable qualification used when this owner's first term was issued.
/// The current selected licence may later renew, but that cannot rewrite this
/// historical window or extend the child's already-issued term.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DependentParent {
    pub(crate) owner: ObjId,
    pub(crate) parent: ParentContent,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) licence: LicenceId,
    pub(crate) term: TermGeneration,
    pub(crate) window: TermWindow,
    pub(crate) term_seconds: i64,
    pub(crate) grace_seconds: i64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::owner_qualification::records::present_holder_capture"
    )]
    pub(crate) holder_capture: Option<super::owner_qualification::records::ParentHolderCapture>,
}

impl DependentParent {
    pub(crate) fn capture(
        actual: &CurrentParentGrant,
        term_seconds: i64,
        grace_seconds: i64,
    ) -> Result<Self, PublicationError> {
        let holding = actual.holding();
        let value = Self {
            owner: actual.actor(),
            parent: actual.provenance().parent,
            charter: holding.charter(),
            line: holding.line(),
            licence: holding.licence(),
            term: holding.term(),
            window: holding.window(),
            term_seconds,
            grace_seconds,
            holder_capture: super::owner_qualification::records::ParentHolderCapture::capture(
                actual,
            ),
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        if let Some(capture) = &self.holder_capture {
            capture.validate()?;
        }
        if self.owner.0 == 0
            || !(1..=MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&self.grace_seconds)
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }

    pub(crate) fn child_window(
        &self,
        issued: WorldIcSeconds,
    ) -> Result<TermWindow, PublicationError> {
        self.validate()?;
        super::rights::dependent_owner_window(
            issued,
            self.term_seconds,
            self.grace_seconds,
            self.window,
        )
        .map_err(|_| PublicationError::Invalid)
    }
}

/// A first-generation selector belongs to this exact owner and owner licence.
/// Title transfer alone cannot give another person the original owner's grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerParentBinding {
    pub(crate) format: u16,
    pub(crate) licence: LicenceId,
    pub(crate) term: TermGeneration,
    pub(crate) charter: CharterId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) parent: DependentParent,
    pub(crate) operation: CommitToken,
}

impl OwnerParentBinding {
    pub(crate) fn key_for(licence: LicenceId, owner: ObjId) -> String {
        format!("{}/owner/{:08x}", licence.key(), owner.0)
    }

    /// Data coherence only; the publishing caller additionally supplies real
    /// owner observations, E4 output and exact staged typed row readback.
    pub(crate) fn check_rows(
        &self,
        filed: &FiledRecord,
        term: &OwnerTerm,
        operation: &InitialFilingOperation,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        filed.validate()?;
        term.validate()?;
        operation.validate()?;
        if self.parent.holder_capture.as_ref().is_some_and(|capture| {
            WorldIcSeconds::from_stamp(&capture.observed_ic).ok() != Some(filed.content.filed_at())
        }) {
            return Err(PublicationError::Mismatch);
        }
        if operation.format != 3
            || filed.format != 2
            || term.format != 2
            || self.operation != operation.commit
            || self.licence != operation.licence
            || self.term != operation.term
            || self.charter != operation.charter
            || self.filed != operation.filed
            || self.parent.owner != operation.requester
            || operation.parent.as_ref() != Some(&self.parent)
            || term.parent.as_ref() != Some(&self.parent)
            || term.operation != self.operation
            || term.licence != self.licence
            || term.generation != self.term
            || filed.operation != self.operation
            || filed.content.id() != self.filed
            || filed.content.position().charter != self.charter
            || filed.content.parent() != Some(self.parent.parent)
            || term.window != self.parent.child_window(filed.content.filed_at())?
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

impl Record for OwnerParentBinding {
    const FAMILY: &'static str = "roll_mind_owner_parent";
    const MAX_BYTES: usize = 4 * 1024;

    fn key(&self) -> String {
        Self::key_for(self.licence, self.parent.owner)
    }

    fn validate(&self) -> Result<(), PublicationError> {
        self.parent.validate()?;
        if self.format != 1
            || self.term.get() != 1
            || self.licence == self.parent.licence
            || self.charter == self.parent.charter
            || self.filed == self.parent.parent.filed
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

/// A bounded actual committed content view, used both by final audit and live
/// owner projection. It does not construct or substitute any source records.
pub(crate) struct ContentView<'a, R> {
    pub(crate) ctx: &'a R,
    pub(crate) roll: &'a crate::RollStore,
    pub(crate) families: &'a super::families::CommerceFamilies,
}

impl<R: super::read::CommerceRead> super::filed::ContentLookup for ContentView<'_, R> {
    fn check_available(&self) -> Result<(), super::filed::ContentError> {
        self.roll
            .commerce_ready()
            .or_else(|_| self.roll.commerce_audit_base().map(|_| ()))
            .map_err(|_| super::filed::ContentError::Unavailable)
    }

    fn line_charter(&self, line: LineId) -> Result<Option<CharterId>, super::filed::ContentError> {
        let row = self
            .ctx
            .get(&self.families.lines, &line.key())
            .map_err(|_| super::filed::ContentError::Unavailable)?;
        if let Some(row) = row {
            row.validate()
                .map_err(|_| super::filed::ContentError::Mismatch)?;
            if row.id != line {
                return Err(super::filed::ContentError::Key);
            }
        }
        Ok(row.map(|row| row.charter))
    }

    fn revision(
        &self,
        key: crate::RevisionKey,
    ) -> Result<Option<&crate::Revision>, super::filed::ContentError> {
        self.roll
            .revision(key.character, key)
            .map(Some)
            .map_err(|_| super::filed::ContentError::Unavailable)
    }

    fn filed(
        &self,
        filed: FiledRevisionId,
    ) -> Result<Option<&super::filed::FiledRevisionContent>, super::filed::ContentError> {
        let row = self
            .ctx
            .get(&self.families.filed, &filed.key())
            .map_err(|_| super::filed::ContentError::Unavailable)?;
        if let Some(row) = row {
            row.validate()
                .map_err(|_| super::filed::ContentError::Mismatch)?;
            if row.content.id() != filed {
                return Err(super::filed::ContentError::Key);
            }
        }
        Ok(row.map(|row| &row.content))
    }
}

fn row<'a, T: Record>(
    ctx: &'a impl super::read::CommerceRead,
    handle: &mush_core::StateFamilyHandle<String, T>,
    key: String,
) -> Result<&'a T, PublicationError> {
    let row = ctx
        .get(handle, &key)
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::Missing)?;
    row.validate()?;
    if row.key() != key {
        return Err(PublicationError::Mismatch);
    }
    Ok(row)
}

/// Historical proof only. In particular, no current title, time or issuer
/// permission is required to retain a legitimately issued dependent term.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_history(
    ctx: &impl super::read::CommerceRead,
    roll: &crate::RollStore,
    families: &super::families::CommerceFamilies,
    ratings: &super::rating::RatingFamilies,
    binding: &OwnerParentBinding,
    filed: &FiledRecord,
    term: &OwnerTerm,
    operation: &InitialFilingOperation,
) -> Result<(), PublicationError> {
    use super::paid::PaidForm;
    use super::records::RevisionPolicy;
    binding.check_rows(filed, term, operation)?;
    let p = &binding.parent;
    let parent = row(ctx, &families.filed, p.parent.filed.key())?;
    let licence = row(ctx, &families.paid_licences, p.licence.key())?;
    let paid_term = row(
        ctx,
        &families.paid_terms,
        format!("{}/{}", p.licence.key(), p.term.key()),
    )?;
    let allocation =
        super::entitlement::history::granted(ctx, families, ratings, licence, paid_term)?;
    super::owner_qualification::history::require_captured_holder(
        ctx,
        families,
        ratings,
        licence,
        p.holder_capture.as_ref(),
        allocation.holder(),
        p.owner,
    )?;
    let revision = roll
        .revision(
            operation.revision.revision().key.character,
            operation.revision.revision().key,
        )
        .map_err(|_| PublicationError::Unavailable)?;
    let provenance = crate::DerivativeProvenance {
        parent: p.parent,
        charter: p.charter,
        line: p.line,
    };
    if parent.content.revision() != p.parent.revision
        || parent.content.position().charter != p.charter
        || parent.content.position().line != p.line
        || revision.provenance.as_ref() != Some(&provenance)
        || allocation.licence() != p.licence
        || allocation.term() != p.term
        || licence.charter() != p.charter
        || licence.line() != p.line
        || paid_term.window() != p.window
        || paid_term.policy()
            != (RevisionPolicy::Pin {
                filed: p.parent.filed,
                revision: p.parent.revision,
            })
        || !matches!(
            paid_term.terms().form(),
            PaidForm::Open {} | PaidForm::OpenResale {}
        )
    {
        return Err(PublicationError::Mismatch);
    }
    filed
        .content
        .check_links(&ContentView {
            ctx,
            roll,
            families,
        })
        .map_err(|_| PublicationError::Mismatch)?;
    roll.check_required_root(&super::roots::RequiredRoot {
        anchor: super::roots::RootAnchor::OwnerParent(binding.licence, p.owner),
        identity: p.parent.revision,
    })
    .map_err(|_| PublicationError::Mismatch)
}

/// Current eligibility for this exact holder. This is intentionally absent
/// from the personal-paid resolver, including paid customers of a derivative.
pub(crate) fn require_current(
    ctx: &impl super::read::CommerceRead,
    holder: ObjId,
    families: &super::families::CommerceFamilies,
    ratings: &super::rating::RatingFamilies,
    expected: &super::rating::RatingCompatibility,
    binding: &OwnerParentBinding,
) -> Result<
    super::owner_qualification::current::CurrentQualification,
    super::owner_holdings::OwnerProjectionError,
> {
    super::owner_qualification::current::resolve(ctx, holder, families, ratings, expected, binding)
}

/// Snapshot only the bounded existing ancestry from this real transaction.
/// The new line/content pair is already checked by the publishing builder; no
/// mutable callback or borrowed ActionTxn escapes into the structural lookup.
pub(crate) fn check_prepared_links(
    txn: &mut mush_core::ActionTxn<'_>,
    families: &super::families::CommerceFamilies,
    roll: &crate::RollStore,
    child: &super::filed::FiledRevisionContent,
    line: &super::records::LineRecord,
) -> Result<(), PublicationError> {
    use super::filed::{ContentError, ContentLookup, FiledRevisionContent, MAX_PARENT_DEPTH};
    struct PreparedView<'a> {
        roll: &'a crate::RollStore,
        lines: Vec<(LineId, CharterId)>,
        filed: Vec<FiledRevisionContent>,
    }
    impl ContentLookup for PreparedView<'_> {
        fn check_available(&self) -> Result<(), ContentError> {
            self.roll
                .commerce_ready()
                .map_err(|_| ContentError::Unavailable)
        }
        fn line_charter(&self, line: LineId) -> Result<Option<CharterId>, ContentError> {
            Ok(self
                .lines
                .iter()
                .find(|(id, _)| *id == line)
                .map(|(_, charter)| *charter))
        }
        fn revision(
            &self,
            key: crate::RevisionKey,
        ) -> Result<Option<&crate::Revision>, ContentError> {
            self.roll
                .revision(key.character, key)
                .map(Some)
                .map_err(|_| ContentError::Unavailable)
        }
        fn filed(
            &self,
            id: FiledRevisionId,
        ) -> Result<Option<&FiledRevisionContent>, ContentError> {
            Ok(self.filed.iter().find(|row| row.id() == id))
        }
    }
    line.validate()?;
    if child.position().line != line.id || child.position().charter != line.charter {
        return Err(PublicationError::Mismatch);
    }
    let mut view = PreparedView {
        roll,
        lines: vec![(line.id, line.charter)],
        filed: Vec::new(),
    };
    let mut next = child.parent();
    while let Some(parent) = next {
        if view.filed.len() == MAX_PARENT_DEPTH
            || parent.filed == child.id()
            || view.filed.iter().any(|row| row.id() == parent.filed)
        {
            return Err(PublicationError::Mismatch);
        }
        let row = txn
            .get(&families.filed, &parent.filed.key())
            .map_err(|_| PublicationError::Unavailable)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        row.validate()?;
        if row.content.id() != parent.filed || row.content.revision() != parent.revision {
            return Err(PublicationError::Mismatch);
        }
        let line = txn
            .get(&families.lines, &row.content.position().line.key())
            .map_err(|_| PublicationError::Unavailable)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        line.validate()?;
        if line.id != row.content.position().line {
            return Err(PublicationError::Mismatch);
        }
        view.lines.push((line.id, line.charter));
        next = row.content.parent();
        view.filed.push(row.content);
    }
    child
        .check_links(&view)
        .map_err(|_| PublicationError::Mismatch)
}
