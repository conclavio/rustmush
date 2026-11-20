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
    let mut corrupt = canonical.clone();
    corrupt
        .as_object_mut()
        .expect("object")
        .insert("next_id".into(), json!(1));
    assert!(serde_json::from_value::<UnitGrantRegistry>(corrupt).is_err());
    let mut corrupt_zero = canonical.clone();
    corrupt_zero
        .as_object_mut()
        .expect("object")
        .insert("next_id".into(), json!(0));
    assert!(serde_json::from_value::<UnitGrantRegistry>(corrupt_zero).is_err());
    let mut gapped = canonical;
    gapped
        .as_object_mut()
        .expect("object")
        .insert("next_id".into(), json!(99));
    let decoded_gapped =
        serde_json::from_value::<UnitGrantRegistry>(gapped).expect("pruned gap decodes cleanly");
    assert_eq!(decoded_gapped.next_id, 99);
    let mut revoked = row();
    revoked.status = UnitGrantStatus::Revoked;
    revoked.revision = 2;
    let next = registry.with_grant(revoked).expect("revocation");
    assert_eq!(next.next_id, 2);
    assert_eq!(next.grants.len(), 1);
}

#[test]
fn crit12_pruning_reclaims_terminal_slots_without_reusing_ids_and_revoke_is_exempt_near_limit() {
    let mut registry = UnitGrantRegistry::default();
    let total = super::families::MAX_REGISTRY_BYTES;
    let _ = total;
    // Issue, revoke, and prune more than MAX_UNIT_GRANTS lifetime rows; IDs
    // remain strictly monotone and capacity is never exhausted by dead rows.
    for step in 0..(crate::unit_authority::MAX_UNIT_GRANTS + 50) {
        let mut grant = row();
        grant.id = UnitGrantId(registry.next_id);
        grant.valid_from = Some(100);
        grant.valid_until = Some(200);
        registry = registry
            .pruned(150)
            .with_grant(grant.clone())
            .expect("issue");
        let mut revoked = grant;
        revoked.status = UnitGrantStatus::Revoked;
        revoked.revision = 2;
        registry = registry.with_grant(revoked).expect("revoke");
        assert!(registry.grants.len() <= crate::unit_authority::MAX_UNIT_GRANTS);
        assert_eq!(
            registry.next_id,
            u64::try_from(step).expect("step") + 2,
            "monotone next_id must never reuse pruned identities"
        );
    }
    let after_prune = registry.pruned(150);
    assert!(after_prune.grants.is_empty());
    assert_eq!(
        after_prune.next_id,
        u64::try_from(crate::unit_authority::MAX_UNIT_GRANTS + 51).expect("next_id")
    );
    // Expired grants and their live children are pruned transitively on issue.
    let mut expiring = row();
    expiring.id = UnitGrantId(after_prune.next_id);
    expiring.delegable = true;
    expiring.valid_from = Some(100);
    expiring.valid_until = Some(200);
    let r1 = after_prune.with_grant(expiring.clone()).expect("parent");
    let mut child = row();
    child.id = UnitGrantId(r1.next_id);
    child.grantor = ObjId(2);
    child.grantee = UnitGrantee::Character(ObjId(4));
    child.valid_from = Some(100);
    child.valid_until = Some(200);
    child.parent = UnitGrantParent::UnitGrant { id: expiring.id };
    let r2 = r1.with_grant(child).expect("child");
    assert_eq!(r2.pruned(199).grants.len(), 2);
    assert!(r2.pruned(200).grants.is_empty());

    // Fill a registry up to the hard 256 KiB family ceiling (`MAX_REGISTRY_BYTES`)
    // so new issuance is refused, yet revoking an existing active grant STILL
    // succeeds (exempt from MAX_REGISTRY_BYTES lockout).
    let mut near_full = UnitGrantRegistry::default();
    let mut next_unit = 1_000u32;
    let wide_units = |base: u32| -> BTreeSet<ObjId> { (base..base + 200).map(ObjId).collect() };
    while near_full.grants.len() < crate::unit_authority::MAX_UNIT_GRANTS {
        let mut g = row();
        g.id = UnitGrantId(near_full.next_id);
        g.scope = UnitScope::Units {
            units: wide_units(next_unit),
        };
        next_unit += 200;
        let mut candidate = near_full.clone();
        candidate.next_id += 1;
        candidate.grants.insert(g.id, g);
        let bytes = serde_json::to_vec(&candidate).expect("encode").len();
        if bytes > super::families::MAX_REGISTRY_BYTES {
            break;
        }
        near_full = candidate;
    }
    // Top off with small single-unit grants until adding one more byte-sensitive
    // active grant would exceed MAX_REGISTRY_BYTES.
    while near_full.grants.len() < crate::unit_authority::MAX_UNIT_GRANTS {
        let mut g = row();
        g.id = UnitGrantId(near_full.next_id);
        let mut candidate = near_full.clone();
        candidate.next_id += 1;
        candidate.grants.insert(g.id, g);
        let bytes = serde_json::to_vec(&candidate).expect("encode").len();
        if bytes > super::families::MAX_REGISTRY_BYTES {
            break;
        }
        near_full = candidate;
    }
    near_full.validate().expect("near-full registry is valid");
    let encoded_len = serde_json::to_vec(&near_full).expect("encode").len();
    assert!(
        encoded_len > super::families::MAX_ISSUE_REGISTRY_BYTES,
        "expected registry above MAX_ISSUE_REGISTRY_BYTES, got {encoded_len}"
    );
    // New issuance fails cleanly at the issuance byte ceiling.
    let mut extra = row();
    extra.id = UnitGrantId(near_full.next_id);
    assert!(near_full.with_grant(extra).is_err());
    // Revoking an active grant in this near-256 KiB registry MUST succeed.
    let mut to_revoke = near_full
        .grants
        .values()
        .next()
        .cloned()
        .expect("at least one grant");
    to_revoke.status = UnitGrantStatus::Revoked;
    to_revoke.revision = 2;
    let revoked_reg = near_full
        .with_grant(to_revoke.clone())
        .expect("revoke must succeed even near 256 KiB");
    assert!(revoked_reg.validate().is_ok());
    assert!(
        revoked_reg
            .grants
            .get(&to_revoke.id)
            .is_none_or(|g| g.status == UnitGrantStatus::Revoked)
    );
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
