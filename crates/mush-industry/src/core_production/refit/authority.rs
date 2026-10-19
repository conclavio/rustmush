//! Actual facility, immutable published recipe, scoped supplier and current
//! refit site. Accepted completion never reauthorizes a departed operator.
use super::{CoreRefitFactoryConfig, Error, PublishedRefitRecipe, bad, records::*};
use crate::core_production::{DieClass, participant::CoreProductionState};
use mush_core::{ClockStamp, Flags, ObjId, World};
use mush_economy::{EconomicAsset, EconomyStore};
use mush_org::{AuthRequest, CapabilityId, OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use std::{any::TypeId, collections::BTreeMap};
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Live {
    pub(crate) facility: crate::facility::FacilityRecord,
    pub(crate) recipe: PublishedRefitRecipe,
    pub(crate) at: ClockStamp,
    pub(crate) utc: i64,
    org_revision: u64,
    scope_revision: u64,
    titles: BTreeMap<ObjId, EconomicAsset>,
    pub(crate) before: mush_core::DefinitionRef,
    pub(crate) unit_revision: u64,
    pub(crate) displaced: SavedComponent,
}
fn owned<T: 'static>(world: &World, owner: &str) -> Result<(), Error> {
    if world
        .module_state_owner(TypeId::of::<T>())
        .is_none_or(|v| v.as_str() != owner)
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
pub(crate) fn cleanup(world: &World, c: &CoreRefitFactoryConfig) -> Result<(), Error> {
    let defs = c.definitions.0.get().ok_or(Error::Unavailable)?;
    for name in ["industry", "economy", "org", "tactics", "sim"] {
        defs.require_owner_ready(&mush_core::OwnerId::new(name).map_err(bad)?)
            .map_err(bad)?;
    }
    owned::<CoreProductionState>(world, "industry")?;
    owned::<crate::IndustryStore>(world, "industry")?;
    owned::<EconomyStore>(world, "economy")?;
    owned::<OrgStore>(world, "org")?;
    owned::<mush_tactics::UnitStore>(world, "tactics")?;
    if world
        .module_state::<EconomyStore>()
        .is_none_or(EconomyStore::is_frozen)
        || world
            .module_state::<crate::IndustryStore>()
            .is_none_or(|s| !s.is_complete())
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn title(
    world: &World,
    object: ObjId,
    owner: PrincipalId,
    scope: Option<mush_org::OrgScopeId>,
) -> Result<EconomicAsset, Error> {
    let e = world
        .module_state::<EconomyStore>()
        .ok_or(Error::Unavailable)?;
    let t = e.snapshot().asset(object).ok_or(Error::Unavailable)?;
    if t.object != object
        || t.title_owner != owner
        || t.scope != scope
        || t.frozen
        || t.stolen.is_some()
        || t.lifecycle != mush_economy::AssetLifecycle::Active
        || !t.encumbrances.is_empty()
    {
        return Err(Error::Unavailable);
    }
    Ok(t.clone())
}
pub(crate) fn current(
    world: &World,
    c: &CoreRefitFactoryConfig,
    requester: ObjId,
    hull: ObjId,
    incoming: ObjId,
    core_acceptance: mush_core::CommitToken,
    class: DieClass,
) -> Result<Live, Error> {
    cleanup(world, c)?;
    let defs = c.definitions.0.get().ok_or(Error::Unavailable)?;
    defs.get_exact(&c.payment.recipe).map_err(bad)?;
    for root in &c.payment.fitted_units {
        defs.get_exact(root).map_err(bad)?;
    }
    let recipe = c.recipes.resolve(world, class).ok_or(Error::Unavailable)?;
    recipe.validate()?;
    if recipe.definition() != &c.payment.recipe
        || c.recipes.published_digest(world, recipe.pack()) != Some(recipe.pack().digest())
    {
        return Err(Error::Unavailable);
    }
    let store = world
        .module_state::<crate::IndustryStore>()
        .ok_or(Error::Unavailable)?;
    let facility = store
        .facility(c.payment.factory)
        .ok_or(Error::Unavailable)?;
    if store.is_facility_quarantined(facility.object())
        || facility.owner() != &PrincipalId::Organization(c.payment.provider.organization)
        || !facility
            .capabilities()
            .iter()
            .any(|v| v.as_str() == "core_refit")
    {
        return Err(Error::Unavailable);
    }
    for id in [requester, hull, incoming, c.payment.factory] {
        if world.get(id).is_none_or(|o| o.flags.contains(Flags::GOING)) {
            return Err(Error::Unavailable);
        }
    }
    if world.get(requester).and_then(|o| o.location) != Some(c.payment.factory)
        || world.get(hull).and_then(|o| o.location) != Some(c.payment.factory)
        || world.get(incoming).and_then(|o| o.location) != Some(requester)
    {
        return Err(Error::Unavailable);
    }
    let native = mush_economy::core_integration::require_core_delivery(world, core_acceptance)
        .map_err(bad)?;
    let expected = match class {
        DieClass::I => mush_economy::core_integration::CoreClass::I,
        DieClass::II => mush_economy::core_integration::CoreClass::II,
        DieClass::III => mush_economy::core_integration::CoreClass::III,
    };
    let Some(t) = native.terminal() else {
        return Err(Error::Unavailable);
    };
    let mush_economy::core_integration::CoreTerminalResult::Completed { output, .. } = t.result()
    else {
        return Err(Error::Unavailable);
    };
    if native.subject().requester != requester
        || native.subject().class != expected
        || output.object != incoming
    {
        return Err(Error::Unavailable);
    }
    let unit = mush_tactics::component_replacement::current_refit_unit(world, requester, hull)
        .map_err(bad)?;
    if !c.payment.fitted_units.contains(unit.schema()) {
        return Err(Error::Unavailable);
    }
    let displaced = SavedComponent::from_unit(unit, &c.component)?;
    let orgs = world
        .module_state::<OrgStore>()
        .ok_or(Error::Unavailable)?
        .state();
    let org = orgs
        .org(c.payment.provider.organization)
        .ok_or(Error::Unavailable)?;
    let scope = orgs
        .scope(c.payment.provider.scope)
        .ok_or(Error::Unavailable)?;
    if org.status != OrgStatus::Active || scope.status != ScopeStatus::Active || scope.org != org.id
    {
        return Err(Error::Unavailable);
    }
    let mut titles = BTreeMap::new();
    titles.insert(
        incoming,
        title(world, incoming, PrincipalId::Object(requester), None)?,
    );
    titles.insert(
        hull,
        title(world, hull, PrincipalId::Object(requester), None)?,
    );
    titles.insert(
        c.payment.factory,
        title(
            world,
            c.payment.factory,
            PrincipalId::Organization(org.id),
            Some(scope.id),
        )?,
    );
    if let Some(a) = displaced.installed.asset {
        if world
            .get(a)
            .is_none_or(|o| o.location != Some(hull) || o.flags.contains(Flags::GOING))
        {
            return Err(Error::Unavailable);
        }
        titles.insert(a, title(world, a, PrincipalId::Object(requester), None)?);
    }
    Ok(Live {
        facility: facility.clone(),
        recipe,
        at: world.world_ic_now(),
        utc: world.now(),
        org_revision: org.revision,
        scope_revision: scope.revision,
        titles,
        before: unit.schema().clone(),
        unit_revision: unit.revision(),
        displaced,
    })
}
pub(crate) fn allow(
    world: &World,
    c: &CoreRefitFactoryConfig,
    operator: ObjId,
) -> Result<(), Error> {
    if world
        .get(operator)
        .is_none_or(|o| o.flags.contains(Flags::GOING))
        || !mush_org::authorize_in_world(
            world,
            &AuthRequest {
                actor: operator,
                capability: CapabilityId::new("industry.core.refit").ok_or(Error::Invalid)?,
                scope: c.payment.provider.scope,
                now: world.now(),
                amount: Some(1),
            },
        )
        .is_allow()
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
pub(crate) fn offer(
    world: &World,
    c: &CoreRefitFactoryConfig,
    row: &RefitOffer,
) -> Result<Live, Error> {
    row.validate()?;
    allow(world, c, row.operator)?;
    let l = current(
        world,
        c,
        row.requester,
        row.hull,
        row.incoming,
        row.core_acceptance,
        row.recipe.class(),
    )?;
    if row.policy != c.payment
        || row.component != c.component
        || row.facility != l.facility
        || row.recipe != l.recipe
        || row.before != l.before
        || row.unit_revision != l.unit_revision
        || row.displaced != l.displaced
    {
        return Err(Error::Unavailable);
    }
    Ok(l)
}
pub(crate) fn accept(
    world: &World,
    c: &CoreRefitFactoryConfig,
    intent: &AcceptedIntent,
) -> Result<Live, Error> {
    intent.validate()?;
    intent
        .offer
        .require_time(&world.world_ic_now(), world.now())?;
    let l = offer(world, c, &intent.offer)?;
    if l.at.clock != intent.accepted_at.clock
        || l.at.at < intent.accepted_at.at
        || l.at.at >= intent.expires_at.at
    {
        return Err(Error::Expired);
    }
    Ok(l)
}
pub(crate) fn install(
    world: &World,
    c: &CoreRefitFactoryConfig,
    intent: &AcceptedIntent,
) -> Result<Live, Error> {
    intent.validate()?;
    let o = &intent.offer;
    let l = current(
        world,
        c,
        o.requester,
        o.hull,
        o.incoming,
        o.core_acceptance,
        o.recipe.class(),
    )?;
    if o.policy != c.payment
        || o.component != c.component
        || o.facility != l.facility
        || o.recipe != l.recipe
        || o.before != l.before
        || o.unit_revision != l.unit_revision
        || o.displaced != l.displaced
        || l.at.clock != intent.accepted_at.clock
        || l.at.at < intent.accepted_at.at
        || l.at.at >= intent.expires_at.at
    {
        return Err(Error::Unavailable);
    }
    Ok(l)
}
pub(crate) fn unchanged(old: &Live, now: &Live) -> Result<(), Error> {
    if old.facility != now.facility
        || old.recipe != now.recipe
        || old.org_revision != now.org_revision
        || old.scope_revision != now.scope_revision
        || old.titles != now.titles
        || old.before != now.before
        || old.unit_revision != now.unit_revision
        || old.displaced != now.displaced
        || now.at.clock != old.at.clock
        || now.at.at < old.at.at
        || now.utc < old.utc
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
