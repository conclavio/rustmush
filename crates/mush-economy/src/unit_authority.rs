//! Live Economy facts for the injected unit-authority resolver (DT.2a).
//!
//! Title comes from the current journal-backed asset projection and its exact
//! creating/transferring effect. Assignment, custody, MUSH ownership and generic
//! lease collateral are independent axes and grant no operational capability.
//! The caller still identifies which application objects are field units.
//!
//! The pure resolver derives organization title control and explicit loans from
//! these facts and its canonical grant registry. No current Economy contract
//! encodes an operational/delegation capability, so `organization_control`
//! deliberately retains the trait's denying default. Mission facts are likewise
//! absent until the registered mission owner supplies them in a later cut.

use mush_core::{Flags, ObjId, World};
use mush_org::unit_authority::{UnitAssetFacts, UnitAuthorityFacts, UnitLeaseBinding};
use mush_org::{OrgStatus, PrincipalId};

use crate::{AssetLifecycle, EconomyStore};

/// Borrowed live reads; no authority fact is cached between evaluations.
pub struct EconomyUnitAuthorityFacts<'w> {
    world: &'w World,
}

impl<'w> EconomyUnitAuthorityFacts<'w> {
    /// Read facts from this world at the instant the resolver asks for them.
    #[must_use]
    pub const fn new(world: &'w World) -> Self {
        Self { world }
    }

    fn object_live(&self, object: ObjId) -> bool {
        self.world
            .get(object)
            .is_some_and(|record| !record.flags.contains(Flags::GOING))
    }

    fn owner_active(&self, owner: &PrincipalId) -> bool {
        match owner {
            PrincipalId::Object(object) => self.principal_exists(*object),
            PrincipalId::Organization(org) => self
                .world
                .module_state::<mush_org::OrgStore>()
                .and_then(|store| store.state().org(*org))
                .is_some_and(|record| record.status == OrgStatus::Active),
            // A system issuer is not an operational actor.
            PrincipalId::System(_) => false,
        }
    }
}

impl UnitAuthorityFacts for EconomyUnitAuthorityFacts<'_> {
    fn equipment_lease(&self, organization: mush_org::OrgId, lease: &UnitLeaseBinding) -> bool {
        let Some(asset) = self
            .asset(lease.unit)
            .filter(|a| a.active && a.owner == PrincipalId::Organization(organization))
        else {
            return false;
        };
        let Some(store) = self.world.module_state::<EconomyStore>() else {
            return false;
        };
        let id = crate::ContractId(lease.contract);
        let Some(contract) = store.snapshot().contract(id) else {
            return false;
        };
        let lessor = PrincipalId::Organization(organization);
        asset.unit == lease.unit && self.principal_exists(lease.tenant)
            && contract.state == crate::ContractState::Active
            && contract.issuer == lessor
            && contract.consent_hash == lease.consent_hash
            && contract.consent_hash == crate::contract::consent_hash(&contract.issuer, &contract.consent)
            && contract.consent.parties.contains(&PrincipalId::Object(lease.tenant))
            && store.snapshot().asset(lease.unit).is_some_and(|a| a.encumbrances.iter().any(|e| matches!(e, crate::Encumbrance::Lease { lessor: owner, contract } if *owner==lessor && *contract==id)))
    }
    fn principal_exists(&self, object: ObjId) -> bool {
        self.world
            .get(object)
            .is_some_and(|record| record.kind.is_actor() && !record.flags.contains(Flags::GOING))
    }

    fn asset(&self, unit: ObjId) -> Option<UnitAssetFacts> {
        let store = self.world.module_state::<EconomyStore>()?;
        let asset = store.snapshot().asset(unit)?;
        let journal = store.fold.journal_assets.get(&unit)?;
        let lineage = store.title_lineage(unit)?;
        if !crate::boot::journal_fields_eq(asset, journal)
            || lineage.journal_seq > asset.last_journal_seq
        {
            return None;
        }
        Some(UnitAssetFacts {
            unit,
            owner: asset.title_owner.clone(),
            lineage,
            scope: asset.scope,
            active: !store.is_frozen()
                && !asset.frozen
                && asset.lifecycle == AssetLifecycle::Active
                && self.object_live(unit)
                && self.owner_active(&asset.title_owner)
                && crate::api::scope_coherent(self.world, asset.scope, &asset.title_owner),
        })
    }
}

/// Composition-time factory for the org module's injected live fact seam.
#[derive(Clone, Copy, Debug, Default)]
pub struct EconomyUnitAuthorityWorldFacts;

impl mush_org::unit_authority::runtime::UnitAuthorityWorldFacts for EconomyUnitAuthorityWorldFacts {
    fn facts<'a>(&'a self, world: &'a World) -> Box<dyn UnitAuthorityFacts + 'a> {
        Box::new(EconomyUnitAuthorityFacts::new(world))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{GOD, LIMBO, ObjKind};
    use mush_org::unit_authority::{
        RecoveryPolicy, UnitCapability, UnitGrant, UnitGrantId, UnitGrantParent, UnitGrantStatus,
        UnitGrantee, UnitScope, validate_unit_grant,
    };
    use mush_testkit::TestWorld;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn dt_unit_facts_require_human_principals_but_keep_thing_assets_active() {
        let mut world = TestWorld::with_modules(vec![
            Box::new(mush_org::OrgModule::new()),
            Box::new(crate::EconomyModule::new()),
        ]);
        let player = world.spawn_player("Pilot");
        let npc = world
            .world_mut()
            .create(ObjKind::Npc, "Human NPC", GOD, Some(LIMBO))
            .expect("npc");
        let exit = world
            .world_mut()
            .create(ObjKind::Exit, "Exit", GOD, Some(LIMBO))
            .expect("exit");
        let unit = world
            .world_mut()
            .create(ObjKind::Thing, "Unit", GOD, Some(LIMBO))
            .expect("unit");
        world.run(GOD, &format!("@asset/title #{}=#{}", unit.0, GOD.0));
        let facts = EconomyUnitAuthorityFacts::new(world.world());
        let asset = facts.asset(unit).expect("current title");
        assert!(
            asset.active,
            "a Thing hull does not need to be a human principal"
        );
        let orgs = world
            .world()
            .module_state::<mush_org::OrgStore>()
            .expect("orgs")
            .state();
        for (grantee, human) in [
            (player, true),
            (npc, true),
            (LIMBO, false),
            (exit, false),
            (unit, false),
        ] {
            assert_eq!(facts.principal_exists(grantee), human);
            let grant = UnitGrant {
                id: UnitGrantId(1),
                revision: 1,
                grantor: GOD,
                grantee: UnitGrantee::Character(grantee),
                capability: UnitCapability::OperateCrew,
                scope: UnitScope::Units {
                    units: BTreeSet::from([unit]),
                },
                valid_from: None,
                valid_until: None,
                delegable: false,
                recovery: RecoveryPolicy::None,
                parent: UnitGrantParent::PersonalTitle {
                    unit,
                    owner: GOD,
                    lineage: asset.lineage,
                },
                status: UnitGrantStatus::Active,
            };
            assert_eq!(
                validate_unit_grant(
                    orgs,
                    &BTreeMap::new(),
                    &facts,
                    &mush_org::NoPredicates,
                    &grant,
                    world.world().now(),
                )
                .is_ok(),
                human,
                "character grantee {grantee}"
            );
        }

        world.run(GOD, &format!("@asset/transfer #{}=#{}", unit.0, unit.0));
        let facts = EconomyUnitAuthorityFacts::new(world.world());
        let asset = facts
            .asset(unit)
            .expect("Thing-owned title remains an economic fact");
        assert_eq!(asset.owner, PrincipalId::Object(unit));
        assert!(
            !asset.active,
            "an object title holder is not a human personal owner"
        );
    }

    #[test]
    fn dt_unit_facts_fail_closed_for_missing_lineage_drift_and_frozen_economy() {
        let mut world = TestWorld::with_modules(vec![
            Box::new(mush_org::OrgModule::new()),
            Box::new(crate::EconomyModule::new()),
        ]);
        let unit = world
            .world_mut()
            .create(ObjKind::Thing, "Unit", GOD, Some(LIMBO))
            .expect("unit");
        world.run(GOD, &format!("@asset/title #{}=#{}", unit.0, GOD.0));
        let original = EconomyUnitAuthorityFacts::new(world.world())
            .asset(unit)
            .expect("current title");
        assert!(original.active);

        let state = world
            .world_mut()
            .module_state_mut::<EconomyStore>()
            .expect("economy");
        state.fold.title_lineage.remove(&unit);
        assert!(
            EconomyUnitAuthorityFacts::new(world.world())
                .asset(unit)
                .is_none()
        );
        let state = world
            .world_mut()
            .module_state_mut::<EconomyStore>()
            .expect("economy");
        state.fold.title_lineage.insert(unit, original.lineage);
        state.snapshot.asset_mut(unit).expect("asset").title_owner =
            PrincipalId::Object(ObjId(999));
        assert!(
            EconomyUnitAuthorityFacts::new(world.world())
                .asset(unit)
                .is_none()
        );

        let state = world
            .world_mut()
            .module_state_mut::<EconomyStore>()
            .expect("economy");
        state.snapshot.asset_mut(unit).expect("asset").title_owner = original.owner;
        state.frozen = true;
        assert!(
            !EconomyUnitAuthorityFacts::new(world.world())
                .asset(unit)
                .expect("title")
                .active
        );
        let state = world
            .world_mut()
            .module_state_mut::<EconomyStore>()
            .expect("economy");
        state.frozen = false;
        // Even internally consistent title rows cannot authorize through a
        // missing org compartment or an abandoned lifecycle.
        for row in [
            state.snapshot.asset_mut(unit).expect("asset"),
            state.fold.journal_assets.get_mut(&unit).expect("journal"),
        ] {
            row.lifecycle = AssetLifecycle::Abandoned;
        }
        assert!(
            !EconomyUnitAuthorityFacts::new(world.world())
                .asset(unit)
                .expect("title")
                .active
        );
        let state = world
            .world_mut()
            .module_state_mut::<EconomyStore>()
            .expect("economy");
        for row in [
            state.snapshot.asset_mut(unit).expect("asset"),
            state.fold.journal_assets.get_mut(&unit).expect("journal"),
        ] {
            row.lifecycle = AssetLifecycle::Active;
            row.scope = Some(mush_org::OrgScopeId(999));
        }
        assert!(
            !EconomyUnitAuthorityFacts::new(world.world())
                .asset(unit)
                .expect("title")
                .active
        );
    }
}
