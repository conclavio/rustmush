//! Current personal owner qualification, distinct from source ancestry and from
//! the immutable grant used when the child was initially filed.
use super::super::{
    LicenceId, TermGeneration, TermWindow,
    dependent_owner::OwnerParentBinding,
    entitlement::history as paid_history,
    families::CommerceFamilies,
    filed::ParentContent,
    owner_holdings::OwnerProjectionError as Error,
    paid::PaidForm,
    personal_paid_holdings::project_personal_paid_for,
    rating::{RatingCompatibility, RatingFamilies},
    read::CommerceRead,
    records::PublicationError,
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::ObjId;
use serde::Serialize;

/// A fresh source-free observation, not a stored grant or public credential.
/// Both child and parent windows are retained, avoiding a fabricated issue date.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CurrentQualification {
    source: ParentContent,
    licence: LicenceId,
    term: TermGeneration,
    form: PaidForm,
    window: TermWindow,
}
impl CurrentQualification {
    pub(crate) const fn source(&self) -> ParentContent {
        self.source
    }
    pub(crate) const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub(crate) const fn term(&self) -> TermGeneration {
        self.term
    }
    pub(crate) const fn form(&self) -> PaidForm {
        self.form
    }
    pub(crate) const fn window(&self) -> TermWindow {
        self.window
    }
}
fn integrity(_: impl std::fmt::Display) -> Error {
    Error::Integrity
}

/// Exact source-parent recall is independent of the replacement's revision.
/// Withdrawal is deliberately not recall for an already acquired pinned source.
pub(crate) fn source_available(
    ctx: &impl CommerceRead,
    roll: &crate::RollStore,
    f: &CommerceFamilies,
    binding: &OwnerParentBinding,
) -> Result<(), PublicationError> {
    let p = &binding.parent;
    let filed = paid_history::read(ctx, &f.filed, p.parent.filed.key())?;
    let status = paid_history::read(ctx, &f.status, p.parent.filed.key())?;
    if filed.content.id() != p.parent.filed
        || filed.content.revision() != p.parent.revision
        || filed.content.position().charter != p.charter
        || filed.content.position().line != p.line
        || status.filed != p.parent.filed
        || status.recalled_by.is_some()
    {
        return Err(PublicationError::Unavailable);
    }
    roll.check_required_root(&RequiredRoot {
        anchor: RootAnchor::Filed(p.parent.filed),
        identity: p.parent.revision,
    })
    .map_err(|_| PublicationError::Mismatch)
}

pub(crate) fn resolve(
    ctx: &impl CommerceRead,
    holder: ObjId,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    expected: &RatingCompatibility,
    binding: &OwnerParentBinding,
) -> Result<CurrentQualification, Error> {
    let birth =
        paid_history::read(ctx, &f.operations, binding.operation.to_hex()).map_err(integrity)?;
    let ownership = super::super::charter_transfer::history::current(ctx, f, birth, holder)
        .map_err(|_| Error::NotHeld)?;
    let roll = ctx
        .world()
        .module_state::<crate::RollStore>()
        .ok_or(Error::Unavailable)?;
    source_available(ctx, roll, f, binding).map_err(|_| Error::NotHeld)?;
    let replacement =
        super::history::selected(ctx, f, holder, binding.licence).map_err(integrity)?;
    let selected = match replacement {
        None if ownership.generation == 1 => binding.parent.licence,
        None => return Err(Error::NotHeld),
        Some(operation) => {
            super::history::verify(ctx, roll, f, ratings, operation).map_err(integrity)?;
            if operation.charter_ownership.as_ref()
                != (ownership.generation > 1).then_some(&ownership)
            {
                return Err(Error::NotHeld);
            }
            operation.selected_parent.licence
        }
    };
    // M8c F1 (the lead's ruling): a personal filing derives from the owner's
    // OWN paid licence only; an organisation-held parent refuses the
    // organisation arm here (`NotHeld`), as `parent_grant` does: organisation
    // intellectual property does not move into personal hands in this cut.
    let actual = project_personal_paid_for(
        ctx,
        super::super::Holder::Character(holder),
        f,
        ratings,
        selected,
        expected,
    )?;
    if actual.actor() != holder
        || actual.charter() != binding.parent.charter
        || actual.line() != binding.parent.line
        || !matches!(
            actual.terms().form(),
            PaidForm::Open {} | PaidForm::OpenResale {}
        )
        || (replacement.is_none()
            && (actual.filed() != binding.parent.parent.filed
                || actual.identity() != binding.parent.parent.revision))
    {
        return Err(Error::NotHeld);
    }
    Ok(CurrentQualification {
        source: binding.parent.parent,
        licence: actual.licence(),
        term: actual.term(),
        form: actual.terms().form(),
        window: actual.window(),
    })
}
