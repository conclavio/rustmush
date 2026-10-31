//! The actual bay, its organization, the scoped supplier, the requester and
//! the hull, read live at every transition. Installation re-checks the
//! offering operator's grant, as the core refit's does (doc 17h cut M8b G10).
use super::records::{BayIntent, BayOffer};
use super::{BayRefitConfig, BayRefitError as Error};
use mush_core::{ClockStamp, DefinitionRef, Flags, ObjId, World};
use mush_economy::{EconomicAsset, EconomyStore};
use mush_org::{AuthRequest, CapabilityId, OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use std::{any::TypeId, collections::BTreeMap};

/// The facility capability word a bay holds.
pub const BAY_CAPABILITY: &str = "bay_refit";

fn bad<T>(_: T) -> Error {
    Error::Integrity
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Live {
    pub(crate) at: ClockStamp,
    pub(crate) utc: i64,
    org_revision: u64,
    scope_revision: u64,
    titles: BTreeMap<ObjId, EconomicAsset>,
    pub(crate) before: DefinitionRef,
    pub(crate) unit_revision: u64,
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
/// Every owner the bay joins is composed, ready and live.
pub(crate) fn cleanup(world: &World, c: &BayRefitConfig) -> Result<(), Error> {
    let defs = c.registry()?;
    for name in ["industry", "economy", "inventory", "org", "tactics", "sim"] {
        defs.require_owner_ready(&mush_core::OwnerId::new(name).map_err(bad)?)
            .map_err(|_| Error::Unavailable)?;
    }
    owned::<super::state::BayRefitState>(world, "industry")?;
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
/// The supplier's operator holds the bay grant on the bay's scope now.
pub(crate) fn allow(
    world: &World,
    c: &BayRefitConfig,
    operator: ObjId,
    facility: ObjId,
) -> Result<(), Error> {
    let site = c.payment.site(facility).map_err(|_| Error::Unavailable)?;
    if world
        .get(operator)
        .is_none_or(|o| o.flags.contains(Flags::GOING))
        || !mush_org::authorize_in_world(
            world,
            &AuthRequest {
                actor: operator,
                capability: CapabilityId::new(mush_economy::bay_refit::OPERATOR_CAPABILITY)
                    .ok_or(Error::Invalid)?,
                scope: site.provider.scope,
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
/// **The live facts a bay order rests on**: the bay is an open,
/// unquarantined facility of the provider's organization holding the bay
/// word; the requester and the hull are at it; the hull is the requester's
/// idle registered unit within the refit family; both titles stand.
pub(crate) fn current(
    world: &World,
    c: &BayRefitConfig,
    requester: ObjId,
    hull: ObjId,
    facility: ObjId,
) -> Result<Live, Error> {
    cleanup(world, c)?;
    let defs = c.registry()?;
    defs.get_exact(c.policy.recipe())
        .map_err(|_| Error::Unavailable)?;
    let site = c.payment.site(facility).map_err(|_| Error::Unavailable)?;
    let store = world
        .module_state::<crate::IndustryStore>()
        .ok_or(Error::Unavailable)?;
    let record = store.facility(facility).ok_or(Error::Unavailable)?;
    if store.is_facility_quarantined(facility)
        || record.owner() != &PrincipalId::Organization(site.provider.organization)
        || !record
            .capabilities()
            .iter()
            .any(|w| w.as_str() == BAY_CAPABILITY)
    {
        return Err(Error::Unavailable);
    }
    for id in [requester, hull, facility] {
        if world.get(id).is_none_or(|o| o.flags.contains(Flags::GOING)) {
            return Err(Error::Unavailable);
        }
    }
    if world.get(requester).and_then(|o| o.location) != Some(facility)
        || world.get(hull).and_then(|o| o.location) != Some(facility)
    {
        return Err(Error::Unavailable);
    }
    let unit = mush_tactics::component_replacement::current_refit_unit(world, requester, hull)
        .map_err(|_| Error::Unavailable)?;
    if !c.roots.contains(unit.schema()) {
        return Err(Error::Unavailable);
    }
    let orgs = world
        .module_state::<OrgStore>()
        .ok_or(Error::Unavailable)?
        .state();
    let org = orgs
        .org(site.provider.organization)
        .ok_or(Error::Unavailable)?;
    let scope = orgs.scope(site.provider.scope).ok_or(Error::Unavailable)?;
    if org.status != OrgStatus::Active || scope.status != ScopeStatus::Active || scope.org != org.id
    {
        return Err(Error::Unavailable);
    }
    let mut titles = BTreeMap::new();
    titles.insert(
        hull,
        title(world, hull, PrincipalId::Object(requester), None)?,
    );
    titles.insert(
        facility,
        title(
            world,
            facility,
            PrincipalId::Organization(org.id),
            Some(scope.id),
        )?,
    );
    Ok(Live {
        at: world.world_ic_now(),
        utc: world.now(),
        org_revision: org.revision,
        scope_revision: scope.revision,
        titles,
        before: unit.schema().clone(),
        unit_revision: unit.revision(),
    })
}
fn joined(row: &BayOffer, c: &BayRefitConfig, l: &Live) -> Result<(), Error> {
    if row.policy != c.policy
        || row.before != l.before
        || row.unit_revision != l.unit_revision
        || c.stores.get(&row.facility) != Some(&row.stores)
        || row.recipe.work_units != c.work_units
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
pub(crate) fn offer(world: &World, c: &BayRefitConfig, row: &BayOffer) -> Result<Live, Error> {
    row.validate()?;
    allow(world, c, row.operator, row.facility)?;
    let l = current(world, c, row.requester, row.hull, row.facility)?;
    joined(row, c, &l)?;
    Ok(l)
}
fn window(l: &Live, intent: &BayIntent) -> Result<(), Error> {
    if l.at.clock != intent.accepted_at.clock
        || l.at.at < intent.accepted_at.at
        || l.at.at >= intent.expires_at.at
    {
        return Err(Error::Expired);
    }
    Ok(())
}
pub(crate) fn accept(world: &World, c: &BayRefitConfig, intent: &BayIntent) -> Result<Live, Error> {
    intent.validate()?;
    intent
        .offer
        .require_time(&world.world_ic_now(), world.now())?;
    let l = offer(world, c, &intent.offer)?;
    window(&l, intent)?;
    Ok(l)
}
pub(crate) fn install(
    world: &World,
    c: &BayRefitConfig,
    intent: &BayIntent,
) -> Result<Live, Error> {
    intent.validate()?;
    let o = &intent.offer;
    allow(world, c, o.operator, o.facility)?;
    let l = current(world, c, o.requester, o.hull, o.facility)?;
    joined(o, c, &l)?;
    window(&l, intent)?;
    Ok(l)
}
pub(crate) fn unchanged(old: &Live, now: &Live) -> Result<(), Error> {
    if old.org_revision != now.org_revision
        || old.scope_revision != now.scope_revision
        || old.titles != now.titles
        || old.before != now.before
        || old.unit_revision != now.unit_revision
        || now.at.clock != old.at.clock
        || now.at.at < old.at.at
        || now.utc < old.utc
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
