use super::records::ImportMonth;
use super::*;
use mush_core::mechanics::DefinitionRegistry;
use mush_core::{ClockStamp, Flags, ObjId, World};
use mush_economy::{EconomyStore, Vault, VaultId, VaultStatus};
use mush_inventory::{EndpointRef, InventoryStore};
use mush_org::{AuthRequest, CapabilityId, OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use std::{
    any::TypeId,
    sync::{Arc, OnceLock},
};
#[derive(Clone, Default)]
pub(crate) struct Definitions(pub(crate) Arc<OnceLock<DefinitionRegistry>>);
impl Definitions {
    pub(crate) fn historical(&self, p: &OriginPolicy) -> Result<(), Error> {
        let defs = self.0.get().ok_or(Error::Unavailable)?;
        let expected = origin_content_descriptor(p.grades).map_err(bad)?;
        let actual = defs.inspect_exact(&p.content).map_err(bad)?;
        if actual.payload() != expected.payload() || actual.references() != expected.references() {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn content(&self, p: &OriginPolicy) -> Result<(), Error> {
        let defs = self.0.get().ok_or(Error::Unavailable)?;
        let expected = origin_content_descriptor(p.grades).map_err(bad)?;
        let actual = defs.get_exact(&p.content).map_err(bad)?;
        if actual.payload() != expected.payload() || actual.references() != expected.references() {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LiveFacts {
    pub(crate) stamp: ClockStamp,
    pub(crate) utc: i64,
    org_revision: u64,
    scope_revision: u64,
    depot: Vault,
    stock: Vault,
}
fn owned<T: 'static>(world: &World, owner: &str) -> Result<(), Error> {
    if world
        .module_state_owner(TypeId::of::<T>())
        .is_none_or(|o| o.as_str() != owner)
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
fn vault(world: &World, p: &OriginPolicy, id: VaultId) -> Result<Vault, Error> {
    let economy = world
        .module_state::<EconomyStore>()
        .ok_or(Error::Unavailable)?;
    let row = economy.snapshot().vault(id).ok_or(Error::Unavailable)?;
    if economy.is_frozen()
        || row.id != id
        || row.owner != PrincipalId::Organization(p.importer)
        || row.scope != Some(p.scope)
        || row.status != VaultStatus::Active
        || row.revision == 0
        || !mush_economy::endpoint_state(world, id).is_live()
    {
        return Err(Error::Unavailable);
    }
    let inventory = world
        .module_state::<InventoryStore>()
        .ok_or(Error::Unavailable)?;
    if !inventory.is_complete() || inventory.is_quarantined(EndpointRef(id)) {
        return Err(Error::Unavailable);
    }
    Ok(row.clone())
}
pub(crate) fn check(
    world: &World,
    defs: &Definitions,
    p: &OriginPolicy,
    actor: ObjId,
) -> Result<LiveFacts, Error> {
    p.validate()?;
    defs.content(p)?;
    owned::<EconomyStore>(world, "economy")?;
    owned::<OrgStore>(world, "org")?;
    owned::<InventoryStore>(world, "inventory")?;
    let agent = p.agent(actor)?;
    if world
        .get(actor)
        .is_none_or(|o| o.flags.contains(Flags::GOING))
    {
        return Err(Error::Unavailable);
    }
    let orgs = world
        .module_state::<OrgStore>()
        .ok_or(Error::Unavailable)?
        .state();
    let org = orgs.org(p.importer).ok_or(Error::Unavailable)?;
    let scope = orgs.scope(p.scope).ok_or(Error::Unavailable)?;
    if org.status != OrgStatus::Active
        || scope.status != ScopeStatus::Active
        || scope.org != p.importer
    {
        return Err(Error::Unavailable);
    }
    for capability in [IMPORT_CAPABILITY, "vault.deposit", "vault.withdraw"] {
        let request = AuthRequest {
            actor,
            capability: CapabilityId::new(capability).ok_or(Error::Invalid)?,
            scope: p.scope,
            now: world.now(),
            amount: Some(1),
        };
        if !mush_org::authorize_in_world(world, &request).is_allow() {
            return Err(Error::Unavailable);
        }
    }
    let depot = vault(world, p, p.depot)?;
    let stock = vault(world, p, agent.stock)?;
    // This is E4's real scoped stock authority in addition to explicit Org Allow;
    // staff/member/object ownership cannot replace the preceding fixed grants.
    if !mush_economy::may_take_from_vault(world, actor, p.depot)
        || !mush_economy::may_take_from_vault(world, actor, agent.stock)
    {
        return Err(Error::Unavailable);
    }
    let stamp = world.world_ic_now();
    ImportMonth::from_stamp(&stamp)?;
    Ok(LiveFacts {
        stamp,
        utc: world.now(),
        org_revision: org.revision,
        scope_revision: scope.revision,
        depot,
        stock,
    })
}
pub(crate) fn recheck(
    world: &World,
    defs: &Definitions,
    p: &OriginPolicy,
    actor: ObjId,
    before: &LiveFacts,
) -> Result<(), Error> {
    let now = check(world, defs, p, actor)?;
    if now.stamp.clock != before.stamp.clock
        || now.stamp.at < before.stamp.at
        || ImportMonth::from_stamp(&now.stamp)? != ImportMonth::from_stamp(&before.stamp)?
        || now.org_revision != before.org_revision
        || now.scope_revision != before.scope_revision
        || now.depot != before.depot
        || now.stock != before.stock
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}
