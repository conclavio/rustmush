//! Actual facility/content/scoped operator facts. Accepted production never
//! borrows a departed operator's identity to charge or receive output.
use super::{
    IntegrationError as Error, PublishedCoreRecipe, bad,
    config::CoreFactoryConfig,
    offer::{FactoryOffer, OfferedInput},
    records::AcceptedIntent,
};
use crate::core_production::{DieClass, authority::Definitions};
use mush_core::{ClockStamp, Flags, ObjId, ObjKind, World};
use mush_economy::{EconomyStore, Vault, VaultStatus};
use mush_org::{AuthRequest, CapabilityId, OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use std::any::TypeId;
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Live {
    pub(crate) facility: crate::facility::FacilityRecord,
    pub(crate) recipe: PublishedCoreRecipe,
    pub(crate) at: ClockStamp,
    pub(crate) utc: i64,
    pub(crate) operator: Option<ObjId>,
    pub(crate) vault: Option<Vault>,
    org_revision: u64,
    scope_revision: u64,
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
pub(crate) fn cleanup(world: &World, defs: &Definitions) -> Result<(), Error> {
    let registry = defs.0.get().ok_or(Error::Unavailable)?;
    for owner in ["industry", "inventory", "economy"] {
        registry
            .require_owner_ready(&mush_core::OwnerId::new(owner).map_err(bad)?)
            .map_err(bad)?;
    }
    owned::<super::super::participant::CoreProductionState>(world, "industry")?;
    owned::<crate::IndustryStore>(world, "industry")?;
    owned::<mush_inventory::InventoryStore>(world, "inventory")?;
    owned::<EconomyStore>(world, "economy")?;
    if world
        .module_state::<EconomyStore>()
        .is_none_or(EconomyStore::is_frozen)
        || world
            .module_state::<mush_inventory::InventoryStore>()
            .is_none_or(|s| !s.is_complete())
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
/// The live facts of the site a die of `certified`-ness integrates at (doc 17h cut M8b-2,
/// H2): the chartered factory, or the ungoverning one.
fn current(
    world: &World,
    defs: &Definitions,
    config: &CoreFactoryConfig,
    class: DieClass,
    certified: bool,
) -> Result<Live, Error> {
    let site = config
        .payment
        .site(certified)
        .map_err(|_| Error::Unavailable)?;
    cleanup(world, defs)?;
    owned::<OrgStore>(world, "org")?;
    defs.0
        .get()
        .ok_or(Error::Unavailable)?
        .require_owner_ready(&mush_core::OwnerId::new("org").map_err(bad)?)
        .map_err(bad)?;
    defs.0
        .get()
        .ok_or(Error::Unavailable)?
        .get_exact(&config.payment.recipe)
        .map_err(bad)?;
    let recipe = config
        .recipes
        .resolve(world, class)
        .ok_or(Error::Unavailable)?;
    recipe.validate()?;
    if recipe.definition() != &config.payment.recipe
        || config.recipes.published_digest(world, recipe.pack()) != Some(recipe.pack().digest())
    {
        return Err(Error::Unavailable);
    }
    let store = world
        .module_state::<crate::IndustryStore>()
        .ok_or(Error::Unavailable)?;
    let facility = store.facility(site.factory).ok_or(Error::Unavailable)?;
    if !store.is_complete()
        || store.is_facility_quarantined(facility.object())
        || !super::offer::holds_site(facility, &site)
        || world
            .get(facility.object())
            .is_none_or(|o| o.flags.contains(Flags::GOING))
    {
        return Err(Error::Unavailable);
    }
    let orgs = world
        .module_state::<OrgStore>()
        .ok_or(Error::Unavailable)?
        .state();
    let org = orgs
        .org(site.integrator.organization)
        .ok_or(Error::Unavailable)?;
    let scope = orgs
        .scope(site.integrator.scope)
        .ok_or(Error::Unavailable)?;
    if org.status != OrgStatus::Active || scope.status != ScopeStatus::Active || scope.org != org.id
    {
        return Err(Error::Unavailable);
    }
    Ok(Live {
        facility: facility.clone(),
        recipe,
        at: world.world_ic_now(),
        utc: world.now(),
        operator: None,
        vault: None,
        org_revision: org.revision,
        scope_revision: scope.revision,
    })
}
fn allow(world: &World, actor: ObjId, scope: mush_org::OrgScopeId, cap: &str) -> Result<(), Error> {
    if world
        .get(actor)
        .is_none_or(|o| o.flags.contains(Flags::GOING))
        || !mush_org::authorize_in_world(
            world,
            &AuthRequest {
                actor,
                capability: CapabilityId::new(cap).ok_or(Error::Invalid)?,
                scope,
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
pub(crate) fn can_offer(
    world: &World,
    defs: &Definitions,
    config: &CoreFactoryConfig,
    operator: ObjId,
    class: DieClass,
    certified: bool,
) -> Result<(), Error> {
    current(world, defs, config, class, certified)?;
    let site = config.payment.site(certified).map_err(bad)?;
    allow(
        world,
        operator,
        site.integrator.scope,
        site.operator_capability,
    )
}
pub(crate) fn offer(
    world: &World,
    defs: &Definitions,
    config: &CoreFactoryConfig,
    operator: ObjId,
    requester: ObjId,
    class: DieClass,
    input: &OfferedInput,
) -> Result<Live, Error> {
    let mut live = current(world, defs, config, class, input.certified())?;
    if world
        .get(requester)
        .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
    {
        return Err(Error::Unavailable);
    }
    let site = config.payment.site(input.certified()).map_err(bad)?;
    allow(
        world,
        operator,
        site.integrator.scope,
        site.operator_capability,
    )?;
    if let OfferedInput::SupplierStock { certificate, lot } = input {
        certificate.matches_lot(lot).map_err(bad)?;
        if certificate.class != class
            || certificate.policy.importer != config.payment.supplier.organization
            || !config.payment.supplier_stock.contains(&lot.endpoint().0)
        {
            return Err(Error::Integrity);
        }
        let economy = world
            .module_state::<EconomyStore>()
            .ok_or(Error::Unavailable)?;
        let vault = economy
            .snapshot()
            .vault(lot.endpoint().0)
            .ok_or(Error::Unavailable)?;
        if vault.status != VaultStatus::Active
            || vault.owner != PrincipalId::Organization(config.payment.supplier.organization)
            || vault.scope != Some(config.payment.supplier.scope)
            || !mush_economy::endpoint_state(world, vault.id).is_live()
            || !mush_economy::may_take_from_vault(world, operator, vault.id)
            || world
                .module_state::<mush_inventory::InventoryStore>()
                .is_none_or(|s| s.is_quarantined(lot.endpoint()))
        {
            return Err(Error::Unavailable);
        }
        allow(
            world,
            operator,
            config.payment.supplier.scope,
            "vault.withdraw",
        )?;
        live.vault = Some(vault.clone());
    }
    live.operator = Some(operator);
    Ok(live)
}
pub(crate) fn accept(
    world: &World,
    defs: &Definitions,
    config: &CoreFactoryConfig,
    offer_row: &FactoryOffer,
    intent: &AcceptedIntent,
) -> Result<Live, Error> {
    intent.matches_offer(offer_row)?;
    offer_row.require_current_time(&world.world_ic_now(), world.now())?;
    let live = offer(
        world,
        defs,
        config,
        offer_row.operator,
        offer_row.requester,
        offer_row.recipe.class(),
        &offer_row.input,
    )?;
    if live.facility != intent.facility
        || live.recipe != intent.recipe
        || intent.policy != config.payment
        || live.at.clock != intent.accepted_at.clock
        || live.at.at < intent.accepted_at.at
        || live.at.at >= intent.expires_at.at
    {
        return Err(Error::Unavailable);
    }
    Ok(live)
}
pub(crate) fn complete(
    world: &World,
    defs: &Definitions,
    config: &CoreFactoryConfig,
    intent: &AcceptedIntent,
) -> Result<Live, Error> {
    intent.validate()?;
    let live = current(
        world,
        defs,
        config,
        intent.recipe.class(),
        intent.certified(),
    )?;
    if live.facility != intent.facility
        || live.recipe != intent.recipe
        || config.payment != intent.policy
        || live.at.clock != intent.accepted_at.clock
        || live.at.at < intent.accepted_at.at
        || live.at.at >= intent.expires_at.at
    {
        return Err(Error::Unavailable);
    }
    Ok(live)
}
pub(crate) fn unchanged(before: &Live, after: &Live) -> Result<(), Error> {
    if before.facility != after.facility
        || before.recipe != after.recipe
        || before.operator != after.operator
        || before.vault != after.vault
        || before.org_revision != after.org_revision
        || before.scope_revision != after.scope_revision
        || before.at.clock != after.at.clock
        || after.at.at < before.at.at
        || after.utc < before.utc
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
