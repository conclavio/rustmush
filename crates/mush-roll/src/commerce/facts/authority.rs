//! Contract authority is current subject authority, not event attribution.
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
    if ctx.requester().as_str() != "economy"
        || !scope.matches_read(ctx, fact)
        || !matches!(scope.visibility(), EconomicVisibility::Parties)
        || scope.org_scope().is_some()
    {
        return Err(PublicationError::Unavailable);
    }
    let subject = match proof.accepted.kind {
        PersonalKind::Paid => {
            let holder = proof
                .paid_current_holder()
                .ok_or(PublicationError::Unavailable)?;
            // No transfer receipt producer exists. Do not infer a different
            // historical/current holder relationship from the pointer alone.
            if holder != proof.accepted.accepted_holder {
                return Err(PublicationError::Unavailable);
            }
            holder
        }
        PersonalKind::Owner => {
            let economy = ctx
                .world()
                .module_state::<mush_economy::EconomyStore>()
                .ok_or(PublicationError::Unavailable)?;
            if economy.is_frozen() {
                return Err(PublicationError::Unavailable);
            }
            let title = economy
                .snapshot()
                .asset(proof.accepted.charter.object())
                .ok_or(PublicationError::Unavailable)?;
            let PrincipalId::Object(holder) = &title.title_owner else {
                return Err(PublicationError::Unavailable);
            };
            if title.object != proof.accepted.charter.object()
                || title.revision == 0
                || title.scope.is_some()
                || title.frozen
                || title.stolen.is_some()
                || title.lifecycle != mush_economy::AssetLifecycle::Active
            {
                return Err(PublicationError::Unavailable);
            }
            // A genuine current title is separate from the immutable owner who
            // accepted this generation; it never rewrites the lapse attribution.
            *holder
        }
    };
    live(ctx, subject)?;
    if !scope.parties().contains(&PrincipalId::Object(subject)) {
        return Err(PublicationError::Unavailable);
    }
    let authorizer = match scope.purpose() {
        FactPurpose::Attach => scope.actor(),
        FactPurpose::Read | FactPurpose::Evaluate => scope
            .attachment_actor(fact)
            .ok_or(PublicationError::Unavailable)?,
    };
    if authorizer != subject {
        return Err(PublicationError::Unavailable);
    }
    if scope.purpose() == FactPurpose::Read {
        live(ctx, scope.actor())?;
        if !scope
            .parties()
            .contains(&PrincipalId::Object(scope.actor()))
        {
            return Err(PublicationError::Unavailable);
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
        return Err(PublicationError::Unavailable);
    }
    Ok(())
}
