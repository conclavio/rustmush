//! Explicit replacement of a personal dependent owner's current qualification.
//! Immutable source ancestry, acquisition, owner term and paid grants stay intact.
pub(crate) mod current;
pub(crate) mod history;
pub(crate) mod protocol;
mod provider;
pub(crate) mod records;
#[cfg(test)]
mod tests;

use super::{
    CharterId, LicenceId,
    parent_grant::CurrentParentGrant,
    records::PublicationError,
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::participant::OwnerPrepareContext;
use mush_core::{CommitToken, DefinitionRef, Flags, ObjId, ObjKind, World};

pub(crate) use provider::register;

pub(crate) fn current_title(
    world: &World,
    actor: ObjId,
    charter: CharterId,
) -> Result<u64, PublicationError> {
    if world
        .get(actor)
        .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        || world
            .get(charter.object())
            .is_none_or(|o| o.flags.contains(Flags::GOING))
        || world
            .module_state_owner(std::any::TypeId::of::<mush_economy::EconomyStore>())
            .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(PublicationError::Unavailable);
    }
    let economy = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let title = economy
        .snapshot()
        .asset(charter.object())
        .ok_or(PublicationError::Missing)?;
    if economy.is_frozen()
        || title.object != charter.object()
        || title.revision == 0
        || title.title_owner != mush_org::PrincipalId::Object(actor)
        || title.scope.is_some()
        || title.frozen
        || title.stolen.is_some()
        || title.lifecycle != mush_economy::AssetLifecycle::Active
    {
        return Err(PublicationError::Unavailable);
    }
    Ok(title.revision)
}

/// M8c B2/B4: `current_title` for a typed holder. A character holder is
/// `current_title`; an organisation holder passes when the charter's current
/// E4 title is the organisation's (any scope: a House's title is scoped to
/// the House's own scope), the acting character is a live player and acts
/// for the organisation on the title's scope under a live `roll.holder.org`
/// grant (`org_holder::scoped_actor`). Staff and assignment never substitute
/// for either.
pub(crate) fn current_title_for(
    world: &World,
    holder: super::Holder,
    charter: CharterId,
) -> Result<u64, PublicationError> {
    let (org, acting) = match holder {
        super::Holder::Character(actor) => return current_title(world, actor, charter),
        super::Holder::Organization { org, acting } => (org, acting),
    };
    if world
        .get(acting)
        .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        || world
            .get(charter.object())
            .is_none_or(|o| o.flags.contains(Flags::GOING))
        || world
            .module_state_owner(std::any::TypeId::of::<mush_economy::EconomyStore>())
            .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(PublicationError::Unavailable);
    }
    let economy = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let title = economy
        .snapshot()
        .asset(charter.object())
        .ok_or(PublicationError::Missing)?;
    if economy.is_frozen()
        || title.object != charter.object()
        || title.revision == 0
        || title.title_owner != mush_org::PrincipalId::Organization(org)
        || title.frozen
        || title.stolen.is_some()
        || title.lifecycle != mush_economy::AssetLifecycle::Active
        || super::org_holder::scoped_actor(world, org, title.scope, acting).is_err()
    {
        return Err(PublicationError::Unavailable);
    }
    Ok(title.revision)
}

/// This private request is minted only after the actual owner reader and typed
/// preimages agree. Serialized operation data cannot construct its authority.
pub(crate) struct Qualification {
    operation: records::QualificationOperation,
    fence: QualificationFence,
    /// The birth requester who holds the parent grant. The parent root was
    /// retained under them, which a chartered owner who is not the requester
    /// is not.
    source_owner: mush_core::ObjId,
}
impl Qualification {
    pub(crate) fn commit(&self) -> CommitToken {
        self.operation.operation
    }
    pub(crate) fn fence(&self) -> &QualificationFence {
        &self.fence
    }
    pub(crate) fn original_roots(&self) -> [RequiredRoot; 2] {
        [
            RequiredRoot {
                anchor: RootAnchor::Filed(self.operation.filed),
                identity: self.operation.child_revision,
            },
            RequiredRoot {
                anchor: RootAnchor::OwnerParent(self.operation.owner_licence, self.source_owner),
                identity: self.operation.source_parent.revision,
            },
        ]
    }
    pub(crate) fn cause_root(&self) -> Result<RequiredRoot, PublicationError> {
        self.operation.root()
    }
}
#[derive(Clone)]
pub(crate) struct QualificationFence {
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    owner_licence: LicenceId,
    charter: CharterId,
    title_revision: u64,
    parent: CurrentParentGrant,
}
impl QualificationFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        let request = provider::Request::parse(ctx.invocation())?;
        if ctx.action() != &self.action
            || ctx.commit_token() != self.commit
            || ctx.invocation().actor() != self.actor
            || request.owner()? != self.owner_licence
            || request.parent()? != self.parent.licence()
            || current_title(ctx.world(), self.actor, self.charter)? != self.title_revision
        {
            return Err(PublicationError::Stale);
        }
        self.parent.check_fresh(self.actor, ctx.world())
    }
}
