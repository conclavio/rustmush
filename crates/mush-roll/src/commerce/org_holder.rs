//! Doc 17h §3 M8c (the lead's ruling B2; doc 17 §16, D-UM-39): acting for an
//! organisation at the Roll and the market.
//!
//! A character granted `roll.holder.org` on an organisation's scope
//! (`@org/grant <char>=<org> roll.holder.org scope=<scope>`) acts for that
//! organisation: it may buy a licence the organisation then holds, renew or
//! attest one it holds, and read its holdings. The check is `org_actor`: the
//! revocation authority's three conditions (`registry_revocation::authority::
//! observe`) without the operators list — the actor exists and is not GOING,
//! the organisation and the scope are Active and the scope is the
//! organisation's, and a LIVE `mush_org::authorize_in_world` allows
//! `roll.holder.org` on that scope. No witness is stored: the grant is
//! re-asked at every act, so a revoked grant stops the next act and nothing
//! has to be recalled.
//!
//! Staff and assignment never substitute for the grant, exactly as they never
//! substitute for title (`owner_holdings::check_title`).
//!
//! **The scope rule (M8c F5).** The paid/entitlement family is unscoped, so
//! its acts ask the organisation's ROOT scope (`acts_for`): buy, pay,
//! complete, refund, renew, and the paid section of the holdings read. The
//! House attestation asks the House's own scope, and a title-bearing read
//! asks the title's scope (`owner_holdings::check_title`); the command
//! pre-gate of those two asks any Active scope (`acts_anywhere`), each row
//! or act then re-asked on its own scope.
//!
//! **Amounts (M8c F2).** `roll.holder.org` is amount-bearing: the spending
//! acts (the Quote and the Fund) ask the grant with the quoted price in
//! minor units (`acts_for_amount`), so a grant with a `GrantLimit` limits
//! and a request over it is denied; every other act asks with no amount,
//! which mush-org denies for a limited grant (fail closed) and allows for an
//! unlimited one.
use mush_core::{Flags, ObjId, World};
use mush_org::{
    AuthRequest, CapabilityId, Decision, OrgId, OrgScopeId, OrgStatus, OrgStore, ScopeStatus,
};
use std::any::TypeId;

/// The capability word; declared in the game's capability vocabulary so
/// `@org/grant` accepts it (`composition::org_capability_vocabulary`).
pub const HOLDER_ORG_CAPABILITY: &str = "roll.holder.org";

/// Why an actor does not act for an organisation. `NotForOrg` is the one a
/// command answers with `words::NOT_FOR_ORG`; `Unavailable` is the org module
/// or the Roll not being ready, answered as the family's uniform refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrgActorError {
    /// The org store or the Roll is not composed or not ready.
    Unavailable,
    /// The actor is not live, or the organisation, scope or grant does not
    /// authorise it.
    NotForOrg,
}

/// Whether `actor` acts for `org` on `scope` right now (see the module note).
///
/// # Errors
/// `Unavailable` when the org or Roll stores are not composed; `NotForOrg`
/// otherwise when any condition fails.
pub fn org_actor(
    world: &World,
    org: OrgId,
    scope: OrgScopeId,
    actor: ObjId,
) -> Result<(), OrgActorError> {
    org_actor_amount(world, org, scope, actor, None)
}

/// `org_actor`, the grant asked with `amount` (minor units): the spending
/// acts pass the quoted price so a limited grant limits (module note).
///
/// # Errors
/// As `org_actor`; an amount over a grant's limit is `NotForOrg`.
pub fn org_actor_amount(
    world: &World,
    org: OrgId,
    scope: OrgScopeId,
    actor: ObjId,
    amount: Option<i64>,
) -> Result<(), OrgActorError> {
    if world
        .module_state_owner(TypeId::of::<OrgStore>())
        .is_none_or(|v| v.as_str() != "org")
        || world
            .module_state_owner(TypeId::of::<crate::RollStore>())
            .is_none_or(|v| v.as_str() != crate::NS)
    {
        return Err(OrgActorError::Unavailable);
    }
    let state = world
        .module_state::<OrgStore>()
        .ok_or(OrgActorError::Unavailable)?
        .state();
    if actor == ObjId(0)
        || world
            .get(actor)
            .is_none_or(|o| o.flags.contains(Flags::GOING))
    {
        return Err(OrgActorError::NotForOrg);
    }
    let organization = state.org(org).ok_or(OrgActorError::NotForOrg)?;
    let node = state.scope(scope).ok_or(OrgActorError::NotForOrg)?;
    if organization.status != OrgStatus::Active
        || node.status != ScopeStatus::Active
        || node.org != org
    {
        return Err(OrgActorError::NotForOrg);
    }
    let capability = CapabilityId::new(HOLDER_ORG_CAPABILITY).ok_or(OrgActorError::Unavailable)?;
    match mush_org::authorize_in_world(
        world,
        &AuthRequest {
            actor,
            capability,
            scope,
            now: world.now(),
            amount,
        },
    ) {
        Decision::Allow(_) => Ok(()),
        Decision::Deny(_) => Err(OrgActorError::NotForOrg),
    }
}

/// The organisation's root scope, the scope an act names when the holder
/// records only the organisation (`Holder::Organization { org, .. }`): a
/// grant placed without `scope=` covers the whole subtree from the root, and
/// a House's curator is granted on the House's own scope, which the House
/// arm names itself.
#[must_use]
pub fn root_scope(world: &World, org: OrgId) -> Option<OrgScopeId> {
    world
        .module_state::<OrgStore>()
        .and_then(|s| s.state().org(org).map(|o| o.root_scope))
}

/// `org_actor` on `scope` when a title or a record names one, else on the
/// organisation's root scope: a House's title is scoped to the House's own
/// scope, where its curator is granted; an organisation-held paid licence
/// records only the organisation.
///
/// # Errors
/// As `org_actor`.
pub fn scoped_actor(
    world: &World,
    org: OrgId,
    scope: Option<OrgScopeId>,
    actor: ObjId,
) -> Result<(), OrgActorError> {
    match scope {
        Some(scope) => org_actor(world, org, scope, actor),
        None => acts_for(world, org, actor),
    }
}

/// `org_actor` on the organisation's root scope.
///
/// # Errors
/// As `org_actor`; an unknown organisation is `NotForOrg`.
pub fn acts_for(world: &World, org: OrgId, actor: ObjId) -> Result<(), OrgActorError> {
    let scope = root_scope(world, org).ok_or(OrgActorError::NotForOrg)?;
    org_actor(world, org, scope, actor)
}

/// `acts_for`, the grant asked with `amount` (the spending acts: the Quote
/// and the Fund pass the quoted price in minor units, F2).
///
/// # Errors
/// As `org_actor_amount`; an unknown organisation is `NotForOrg`.
pub fn acts_for_amount(
    world: &World,
    org: OrgId,
    actor: ObjId,
    amount: Option<i64>,
) -> Result<(), OrgActorError> {
    let scope = root_scope(world, org).ok_or(OrgActorError::NotForOrg)?;
    org_actor_amount(world, org, scope, actor, amount)
}

/// `org_actor` on the organisation's root scope or, failing that, on any of
/// its Active scopes: the pre-gate of exactly the two command forms whose
/// act asks a title's own scope (F5): `@mind/attest … org=` (the House
/// attestation asks the House's scope) and the owner section of
/// `+minds/holdings org=` (each title re-asked on its scope,
/// `check_title`). Every paid-family form pre-gates with `acts_for`.
///
/// # Errors
/// As `org_actor`; `NotForOrg` when no scope of the organisation admits the
/// actor.
pub fn acts_anywhere(world: &World, org: OrgId, actor: ObjId) -> Result<(), OrgActorError> {
    match acts_for(world, org, actor) {
        Err(OrgActorError::NotForOrg) => {}
        other => return other,
    }
    let scopes: Vec<OrgScopeId> = world
        .module_state::<OrgStore>()
        .ok_or(OrgActorError::Unavailable)?
        .state()
        .scopes_in(org)
        .map(|scope| scope.id)
        .collect();
    if scopes
        .into_iter()
        .any(|scope| org_actor(world, org, scope, actor).is_ok())
    {
        Ok(())
    } else {
        Err(OrgActorError::NotForOrg)
    }
}

/// Whether `actor` may act as `holder`: the character itself, or a character
/// acting for the holder's organisation on its root scope. The one test every
/// holder-side verb and read runs for an organisation-held licence.
///
/// # Errors
/// As `acts_for`; a character holder that is not `actor` is `NotForOrg`.
pub fn acts_as(
    world: &World,
    holder: mush_economy::Holder,
    actor: ObjId,
) -> Result<(), OrgActorError> {
    match holder {
        mush_economy::Holder::Character(c) if c == actor => Ok(()),
        mush_economy::Holder::Character(_) => Err(OrgActorError::NotForOrg),
        mush_economy::Holder::Organization { org, .. } => acts_for(world, org, actor),
    }
}

/// `acts_as`, then the holder as a record written by `actor` now carries it:
/// a character holder unchanged, an organisation holder re-stamped with
/// `actor` as its acting character (the attribution of this act, B1).
///
/// # Errors
/// As `acts_as`.
pub fn acting_as(
    world: &World,
    holder: mush_economy::Holder,
    actor: ObjId,
) -> Result<mush_economy::Holder, OrgActorError> {
    acts_as(world, holder, actor)?;
    Ok(match holder {
        mush_economy::Holder::Character(c) => mush_economy::Holder::Character(c),
        mush_economy::Holder::Organization { org, .. } => {
            mush_economy::Holder::Organization { org, acting: actor }
        }
    })
}

#[cfg(test)]
mod tests;
