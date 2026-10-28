#![cfg(test)]

use super::*;
use crate::unit_authority::{
    RecoveryPolicy, TitleLineage, UnitAssetFacts, UnitAuthorityConfig, UnitCapability, UnitGrant,
    UnitGrantId, UnitGrantParent, UnitGrantStatus, UnitGrantee, UnitScope,
};
use crate::{OrgModule, PrincipalId};
use mush_core::{GOD, ObjKind};
use mush_testkit::TestWorld;

#[derive(Debug)]
struct FactsFactory;
struct Facts<'a>(&'a World);
impl UnitAuthorityWorldFacts for FactsFactory {
    fn facts<'a>(&'a self, w: &'a World) -> Box<dyn UnitAuthorityFacts + 'a> {
        Box::new(Facts(w))
    }
}
impl UnitAuthorityFacts for Facts<'_> {
    fn principal_exists(&self, actor: ObjId) -> bool {
        self.0.get(actor).is_some_and(|o| o.kind.is_actor())
    }
    fn asset(&self, unit: ObjId) -> Option<UnitAssetFacts> {
        let row = self.0.get(unit)?;
        (row.kind == ObjKind::Thing).then_some(UnitAssetFacts {
            unit,
            owner: PrincipalId::Object(row.owner),
            lineage: TitleLineage {
                journal_seq: 1,
                effect_index: 0,
            },
            scope: None,
            active: true,
        })
    }
}
fn fixture() -> (TestWorld, ObjId, ObjId) {
    let config = UnitAuthorityConfig {
        facts: Arc::new(FactsFactory),
        requesters: BTreeSet::new(),
        dependencies: BTreeSet::new(),
    };
    let mut world =
        TestWorld::with_modules(vec![Box::new(OrgModule::new().with_unit_authority(config))]);
    world.world_mut().set_now(100);
    let recipient = world.spawn_player("Recipient");
    let unit = world
        .world_mut()
        .create(ObjKind::Thing, "Hull", GOD, Some(mush_core::LIMBO))
        .expect("fixture hull");
    (world, recipient, unit)
}
fn row() -> UnitGrant {
    UnitGrant {
        id: UnitGrantId(1),
        revision: 1,
        grantor: GOD,
        grantee: UnitGrantee::Character(ObjId(2)),
        capability: UnitCapability::Deploy,
        scope: UnitScope::Units {
            units: BTreeSet::from([ObjId(3)]),
        },
        valid_from: Some(100),
        valid_until: None,
        delegable: false,
        recovery: RecoveryPolicy::None,
        parent: UnitGrantParent::PersonalTitle {
            unit: ObjId(3),
            owner: GOD,
            lineage: TitleLineage {
                journal_seq: 1,
                effect_index: 0,
            },
        },
        status: UnitGrantStatus::Active,
    }
}

#[test]
fn registry_decode_is_strict_and_retains_monotone_identity() {
    let registry = UnitGrantRegistry::default()
        .with_grant(row())
        .expect("valid grant");
    let canonical = serde_json::to_value(&registry).expect("registry encoding");
    assert_eq!(
        serde_json::from_value::<UnitGrantRegistry>(canonical.clone()).expect("registry decoding"),
        registry
    );
    for field in ["format", "revision", "next_id", "grants"] {
        let mut corrupt = canonical.clone();
        corrupt.as_object_mut().expect("object").remove(field);
        assert!(serde_json::from_value::<UnitGrantRegistry>(corrupt).is_err());
    }
    let mut corrupt = canonical;
    corrupt
        .as_object_mut()
        .expect("object")
        .insert("next_id".into(), json!(99));
    assert!(serde_json::from_value::<UnitGrantRegistry>(corrupt).is_err());
    let mut revoked = row();
    revoked.status = UnitGrantStatus::Revoked;
    revoked.revision = 2;
    let next = registry.with_grant(revoked).expect("revocation");
    assert_eq!(next.next_id, 2);
    assert_eq!(next.grants.len(), 1);
}

#[test]
fn command_stages_only_input_and_failed_admission_cleans_it() {
    let (mut world, recipient, unit) = fixture();
    let _ = world.run(
        GOD,
        &format!("@unitgrant/issue #{}=#{} deploy", unit.0, recipient.0),
    );
    let handles = world
        .world()
        .module_state::<UnitAuthorityHandles>()
        .cloned()
        .expect("handles");
    assert!(pending(world.world(), GOD).is_some());
    assert!(
        world
            .engine()
            .action_state()
            .get(&handles.registry, &())
            .expect("registry read")
            .is_none()
    );
    assert!(world.engine_mut().take_authoritative_submit().is_some());
    let _ = world.run(
        recipient,
        &format!("@unitgrant/issue #{}=#1 deploy", unit.0),
    );
    assert!(pending(world.world(), recipient).is_none());
    assert!(world.engine_mut().take_authoritative_submit().is_none());
}

#[test]
fn live_title_and_replaced_intent_change_the_final_revision() {
    let (mut world, recipient, unit) = fixture();
    let _ = world.run(
        GOD,
        &format!("@unitgrant/issue #{}=#{} deploy", unit.0, recipient.0),
    );
    let registry = UnitGrantRegistry::default();
    let initial = generation(world.world(), &FactsFactory, &registry, GOD).expect("generation");
    world
        .world_mut()
        .chown(unit, recipient)
        .expect("title fixture");
    assert_ne!(
        generation(world.world(), &FactsFactory, &registry, GOD).expect("changed generation"),
        initial
    );
    world.world_mut().chown(unit, GOD).expect("title fixture");
    let _ = world.engine_mut().take_authoritative_submit();
    let _ = world.run(
        GOD,
        &format!("@unitgrant/issue #{}=#{} operate_crew", unit.0, recipient.0),
    );
    assert_ne!(
        generation(world.world(), &FactsFactory, &registry, GOD).expect("changed intent"),
        initial
    );
}
