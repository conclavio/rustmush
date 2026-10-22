//! Current lapse-subject or actual transfer-participant authority, never event attribution.
use super::super::records::PublicationError;
use super::history::TermProof;
use super::records::PersonalKind;
use mush_core::read_service::OwnerReadContext;
use mush_core::{Flags, ObjId, ObjKind};
use mush_economy::{ContractFactScope, EconomicVisibility, ExternalRef, FactPurpose};
use mush_org::PrincipalId;

pub(super) fn authorize(
    ctx: &OwnerReadContext<'_>,
    scope: &ContractFactScope,
    fact: &ExternalRef,
    proof: &TermProof,
) -> Result<(), PublicationError> {
    common(ctx, scope, fact)?;
    let subject = match proof.accepted.kind {
        PersonalKind::Paid => proof
            .paid_current_holder()
            .ok_or(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable))?,
        PersonalKind::Owner => {
            let economy = ctx
                .world()
                .module_state::<mush_economy::EconomyStore>()
                .ok_or(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable))?;
            if economy.is_frozen() {
                return Err(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable));
            }
            let title = economy
                .snapshot()
                .asset(proof.accepted.charter.object())
                .ok_or(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable))?;
            let PrincipalId::Object(holder) = &title.title_owner else {
                return Err(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable));
            };
            if title.object != proof.accepted.charter.object()
                || title.revision == 0
                || title.scope.is_some()
                || title.frozen
                || title.stolen.is_some()
                || title.lifecycle != mush_economy::AssetLifecycle::Active
            {
                return Err(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable));
            }
            // A genuine current title is separate from the immutable owner who
            // accepted this generation; it never rewrites the lapse attribution.
            *holder
        }
    };
    participants(ctx, scope, fact, &[subject])
}

/// Historical event authorization follows the actual transaction parties, not
/// its event actor or a later unrelated holder of the same property.
pub(super) fn transfer(
    ctx: &OwnerReadContext<'_>,
    scope: &ContractFactScope,
    fact: &ExternalRef,
    seller: ObjId,
    buyer: ObjId,
) -> Result<(), PublicationError> {
    common(ctx, scope, fact)?;
    participants(ctx, scope, fact, &[seller, buyer])
}
fn common(
    ctx: &OwnerReadContext<'_>,
    scope: &ContractFactScope,
    fact: &ExternalRef,
) -> Result<(), PublicationError> {
    if ctx.requester().as_str() != "economy"
        || !scope.matches_read(ctx, fact)
        || !matches!(scope.visibility(), EconomicVisibility::Parties)
        || scope.org_scope().is_some()
    {
        return Err(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable));
    }
    Ok(())
}
fn participants(
    ctx: &OwnerReadContext<'_>,
    scope: &ContractFactScope,
    fact: &ExternalRef,
    subjects: &[ObjId],
) -> Result<(), PublicationError> {
    let authorizer = match scope.purpose() {
        FactPurpose::Attach => scope.actor(),
        FactPurpose::Read | FactPurpose::Evaluate => scope
            .attachment_actor(fact)
            .ok_or(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable))?,
    };
    live(ctx, authorizer)?;
    if !subjects.contains(&authorizer)
        || !scope.parties().contains(&PrincipalId::Object(authorizer))
    {
        return Err(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable));
    }
    if scope.purpose() == FactPurpose::Read {
        live(ctx, scope.actor())?;
        if !scope
            .parties()
            .contains(&PrincipalId::Object(scope.actor()))
        {
            return Err(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable));
        }
    }
    Ok(())
}

fn live(ctx: &OwnerReadContext<'_>, actor: ObjId) -> Result<(), PublicationError> {
    if ctx
        .world()
        .get(actor)
        .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
    {
        return Err(super::history::diag(concat!(file!(), ":", line!()))(PublicationError::Unavailable));
    }
    Ok(())
}
