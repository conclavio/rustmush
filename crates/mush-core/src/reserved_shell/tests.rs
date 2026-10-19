use super::*;
use crate::action::{
    ActionInvocationDraft, ActionOfferId, CanonicalHash, IdempotencyKey, IdempotencyScope,
    ScopedIdempotencyKey,
};
use crate::action_state::{
    ActionStateBootRecord, ActionStateStore, ModuleActionState, PreparedTxn,
    StateFamilyRegistryBuilder, TypedStateLens,
};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    MechanicalOwnerReadiness, OwnerId,
};
use crate::module::{ModuleOp, ModuleWrite};
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    world: World,
    store: ActionStateStore,
    service: ShellReservations,
    reserve: DefinitionRef,
    filing: DefinitionRef,
    initialize: DefinitionRef,
    persisted: BTreeMap<String, Value>,
}

fn bare_fixture() -> Fixture {
    let owner = OwnerId::new("roll").unwrap();
    let owners = BTreeSet::from([owner.clone()]);
    let readiness = MechanicalOwnerReadiness::for_owners(&owners);
    let mut families = StateFamilyRegistryBuilder::new();
    let records = families
        .register(
            owner.clone(),
            descriptor("initial_filing_shells").unwrap(),
            |k: &ShellKey| Ok(k.encoded()),
            |v: &Value| decode::<ShellRecord>(v),
            |r: &ShellRecord| r.validate().map_err(|e| e.to_string()),
            |r: &ShellRecord| encode(r),
        )
        .unwrap();
    let open = families
        .register(
            owner.clone(),
            descriptor("open_initial_filing_shells").unwrap(),
            |k: &ObjId| Ok(k.0.to_string()),
            |v: &Value| decode::<OpenShell>(v),
            |r: &OpenShell| {
                if r.format == FORMAT && r.key.requester.0 > 0 {
                    Ok(())
                } else {
                    Err("invalid open shell".into())
                }
            },
            |r: &OpenShell| encode(r),
        )
        .unwrap();
    let deployment = families
        .register(
            owner.clone(),
            descriptor("initial_filing_shell_deployment").unwrap(),
            |_: &()| Ok("registry".to_owned()),
            |v: &Value| decode::<ShellDeployment>(v),
            |r: &ShellDeployment| r.validate().map_err(|e| e.to_string()),
            |r: &ShellDeployment| encode(r),
        )
        .unwrap();
    let families = families
        .freeze_for_owners_with_readiness(&owners, readiness)
        .unwrap();
    let mut definitions = DefinitionRegistryBuilder::new();
    let mut action = |name: &str| {
        definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new(name).unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap()
    };
    let reserve = action(RESERVE_ACTION);
    let filing = action(FILING_ACTION);
    let initialize = action(INITIALIZE_ACTION);
    let mut world = World::new_minimal();
    world.set_now(1357);
    world.take_dirty();
    Fixture {
        world,
        store: ActionStateStore::new(families),
        service: ShellReservations {
            records,
            open,
            deployment,
        },
        reserve,
        filing,
        initialize,
        persisted: BTreeMap::new(),
    }
}

fn fixture() -> Fixture {
    let mut f = bare_fixture();
    let inv = invocation(GOD, 99, json!({}));
    let mut txn = f.store.transaction(&f.service.deployment).unwrap();
    let shell = f
        .service
        .initialize(&f.world, &inv, &f.initialize, token(99), &mut txn)
        .unwrap();
    let typed = txn.prepare().unwrap();
    f.install(typed, shell);
    f
}

fn invocation(actor: ObjId, byte: u8, parameters: Value) -> ActionInvocation {
    let request = RequestUid::from_bytes([byte; 16]);
    let principal = PrincipalId::Object { id: actor };
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([17; 32])),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("roll").unwrap(),
                namespace: CanonicalId::new("filing").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap()
}
fn token(byte: u8) -> CommitToken {
    CommitToken::from_bytes([byte; 16])
}

impl Fixture {
    fn prepare(&mut self, byte: u8) -> (ShellKey, ObjId, PreparedTxn, ShellObjects) {
        let inv = invocation(GOD, byte, json!({}));
        let mut txn = self.store.transaction(&self.service.records).unwrap();
        let (key, object, shell) = self
            .service
            .prepare_reserve(&self.world, &inv, &self.reserve, token(byte), &mut txn)
            .unwrap();
        (key, object, txn.prepare().unwrap(), shell)
    }
    fn install(&mut self, typed: PreparedTxn, shell: ShellObjects) {
        shell.recheck(&self.world).unwrap();
        let (ops, installation) = typed.into_parts();
        let typed = self.store.bind_installation(installation).unwrap();
        let (_, storage) = self.world.composed_install_parts_mut();
        let shell = shell.bind(storage).unwrap();
        let unsettled = typed.apply_images();
        shell.apply();
        unsettled.complete();
        for ModuleOp { op, .. } in ops {
            match op {
                ModuleWrite::JsonSet { key, value } => {
                    self.persisted.insert(key.to_string(), value);
                }
                ModuleWrite::Del { key } => {
                    self.persisted.remove(key.as_str());
                }
                _ => panic!("shell typed rows cannot append streams"),
            }
        }
    }
    fn reserve(&mut self, byte: u8) -> (ShellKey, ObjId) {
        let (key, object, typed, shell) = self.prepare(byte);
        self.install(typed, shell);
        (key, object)
    }
    fn consume(&mut self, key: ShellKey, byte: u8) {
        let inv = invocation(GOD, byte, json!({}));
        let mut txn = self.store.transaction(&self.service.records).unwrap();
        let resolved = self
            .service
            .resolve(&self.world, &inv, &self.filing, &mut txn, key)
            .unwrap();
        let shell = self
            .service
            .consume(
                &self.world,
                &inv,
                &self.filing,
                &mut txn,
                resolved,
                token(byte),
            )
            .unwrap();
        assert!(!shell.creates_object());
        assert!(shell.writes().is_empty());
        let typed = txn.prepare().unwrap();
        self.install(typed, shell);
    }
    fn audit(
        &mut self,
        owner: &str,
        consumed: impl FnMut(ConsumedShell) -> Result<(), ParticipantError>,
    ) -> Result<(), ParticipantError> {
        let registry = crate::Registry::new();
        let mut out = crate::Outbox::default();
        let ctx = crate::ModuleCtx {
            w: &mut self.world,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &registry,
            module: Some(owner.into()),
            typed_state: ModuleActionState::for_module(
                TypedStateLens::Live(&self.store),
                Some(owner),
            ),
        };
        self.service.audit(&ctx, consumed)
    }
}

#[test]
fn reservation_is_uninstalled_until_all_bindings_succeed_and_does_not_dirty_world() {
    let mut f = fixture();
    let before: Vec<_> = f.world.iter().cloned().collect();
    let before_next = f.world.next_id();
    let (key, object, typed, shell) = f.prepare(1);
    assert_eq!(object, ObjId(before_next));
    assert!(matches!(
        shell.writes(),
        [
            ReplayWrite::ObjectSet { .. },
            ReplayWrite::ObjectAllocatorSet { .. }
        ]
    ));
    assert_eq!(f.world.iter().cloned().collect::<Vec<_>>(), before);
    assert_eq!(f.world.next_id(), before_next);
    assert!(f.store.get(&f.service.records, &key).unwrap().is_none());
    f.install(typed, shell);
    assert_eq!(f.world.next_id(), before_next + 1);
    assert_eq!(
        f.world.get(object),
        Some(&Obj::new(object, ObjKind::Thing, NAME, GOD, 1357))
    );
    assert!(!f.world.has_dirty());
    assert!(f.audit("roll", |_| panic!("no consumed row")).is_ok());
}

#[test]
fn refusal_releases_staged_rows_and_never_spends_an_object_id() {
    let mut f = fixture();
    let before = f.world.next_id();
    let (_, _, typed, shell) = f.prepare(1);
    let (_, installation) = typed.into_parts();
    f.store.abort(installation).unwrap();
    drop(shell);
    assert_eq!(f.world.next_id(), before);
    assert_eq!(
        f.store.len(),
        1,
        "only initialized deployment marker remains"
    );
    let (_, object) = f.reserve(1);
    assert_eq!(object, ObjId(before));
}

#[test]
fn abandoned_shell_is_reused_across_new_request_ids_until_consumed() {
    let mut f = fixture();
    let (first, object) = f.reserve(1);
    let next = f.world.next_id();
    let persisted = f.persisted.clone();
    for byte in 2..=4 {
        assert_eq!(f.reserve(byte), (first, object));
        assert_eq!(f.persisted, persisted);
        assert_eq!(f.world.next_id(), next);
        assert_eq!(
            f.store
                .get(&f.service.records, &first)
                .unwrap()
                .unwrap()
                .receipt,
            token(1)
        );
    }
    f.consume(first, 5);
    let (second, next_object) = f.reserve(6);
    assert_ne!(first, second);
    assert_eq!(next_object, ObjId(next));
    assert_eq!(f.world.next_id(), next + 1);
    let mut consumed = Vec::new();
    f.audit("roll", |row| {
        consumed.push(row);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        consumed,
        vec![ConsumedShell {
            key: first,
            object,
            reservation: token(1),
            filing: token(5)
        }]
    );
}

#[test]
fn same_bundle_shell_cannot_be_used_as_an_acknowledged_filing_reservation() {
    let mut f = fixture();
    let inv = invocation(GOD, 1, json!({}));
    let mut txn = f.store.transaction(&f.service.records).unwrap();
    let (key, _, _) = f
        .service
        .prepare_reserve(&f.world, &inv, &f.reserve, token(1), &mut txn)
        .unwrap();
    assert!(
        f.service
            .resolve(&f.world, &inv, &f.filing, &mut txn, key)
            .is_err()
    );
    drop(txn);
    assert_eq!(
        f.store.len(),
        1,
        "only initialized deployment marker remains"
    );
    assert_eq!(f.world.next_id(), 2);
}

#[test]
fn reservation_rejects_nonempty_parameters_and_a_wrong_action_before_allocation() {
    let mut f = fixture();
    for (inv, action) in [
        (invocation(GOD, 1, json!({"object":2})), f.reserve.clone()),
        (invocation(GOD, 2, json!({})), f.filing.clone()),
        (invocation(ObjId(0), 3, json!({})), f.reserve.clone()),
    ] {
        let mut txn = f.store.transaction(&f.service.records).unwrap();
        assert!(
            f.service
                .prepare_reserve(&f.world, &inv, &action, token(7), &mut txn)
                .is_err()
        );
    }
    assert_eq!(
        f.store.len(),
        1,
        "only initialized deployment marker remains"
    );
    assert_eq!(f.world.next_id(), 2);
}

#[test]
fn consumed_uid_cannot_allocate_or_consume_again_and_same_receipt_is_refused() {
    let mut f = fixture();
    let (key, _) = f.reserve(1);
    let inv = invocation(GOD, 2, json!({}));
    let mut txn = f.store.transaction(&f.service.records).unwrap();
    let resolved = f
        .service
        .resolve(&f.world, &inv, &f.filing, &mut txn, key)
        .unwrap();
    assert!(
        f.service
            .consume(&f.world, &inv, &f.filing, &mut txn, resolved, token(1))
            .is_err()
    );
    drop(txn);
    f.consume(key, 2);
    let mut txn = f.store.transaction(&f.service.records).unwrap();
    assert!(
        f.service
            .resolve(&f.world, &inv, &f.filing, &mut txn, key)
            .is_err()
    );
    let reserve = invocation(GOD, 1, json!({}));
    assert!(
        f.service
            .prepare_reserve(&f.world, &reserve, &f.reserve, token(1), &mut txn)
            .is_err()
    );
    drop(txn);
    assert_eq!(f.world.next_id(), 3);
}

#[test]
fn changed_object_or_allocator_refuses_before_applying_typed_rows() {
    let mut f = fixture();
    let (key, object) = f.reserve(1);
    let inv = invocation(GOD, 2, json!({}));
    let mut txn = f.store.transaction(&f.service.records).unwrap();
    let resolved = f
        .service
        .resolve(&f.world, &inv, &f.filing, &mut txn, key)
        .unwrap();
    let shell = f
        .service
        .consume(&f.world, &inv, &f.filing, &mut txn, resolved, token(2))
        .unwrap();
    let typed = txn.prepare().unwrap();
    // Real World hydration gives a distinct exact object image without using a
    // provider-side mutation callback.
    let mut objects: Vec<_> = f.world.iter().cloned().collect();
    objects.iter_mut().find(|o| o.id == object).unwrap().name = "Changed".into();
    f.world = World::from_objects(objects, f.world.next_id()).unwrap();
    assert!(shell.recheck(&f.world).is_err());
    let (_, installation) = typed.into_parts();
    let bound = f.store.bind_installation(installation).unwrap();
    let (_, storage) = f.world.composed_install_parts_mut();
    assert!(shell.bind(storage).is_err());
    drop(bound);
    assert_eq!(f.store.lost_installations(), 1);
    assert_eq!(
        f.store
            .get(&f.service.records, &key)
            .unwrap()
            .unwrap()
            .state,
        ShellState::Reserved {}
    );
    assert!(f.store.get(&f.service.open, &GOD).unwrap().is_some());
}

#[test]
fn reboot_round_trip_checks_closed_rows_open_index_and_reverse_filing_reference() {
    let mut original = fixture();
    let (key, object) = original.reserve(1);
    original.consume(key, 2);
    original.reserve(3);
    let mut rebooted = bare_fixture();
    rebooted.world =
        World::from_objects(original.world.iter().cloned(), original.world.next_id()).unwrap();
    rebooted
        .store
        .hydrate(
            original
                .persisted
                .iter()
                .map(|(k, v)| ActionStateBootRecord::new("roll", k.as_str(), v.clone())),
        )
        .unwrap();
    let expected = ConsumedShell {
        key,
        object,
        reservation: token(1),
        filing: token(2),
    };
    let mut count = 0;
    rebooted
        .audit("roll", |row| {
            assert_eq!(row, expected);
            count += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        rebooted
            .audit("roll", |_| Err(ParticipantError::Authority))
            .is_err()
    );
    assert!(rebooted.audit("foreign", |_| Ok(())).is_err());
    let mut txn = rebooted
        .store
        .transaction(&rebooted.service.records)
        .unwrap();
    txn.remove(&rebooted.service.open, &GOD).unwrap();
    let installation = txn.prepare().unwrap().into_parts().1;
    rebooted.store.install(installation).unwrap();
    assert!(rebooted.audit("roll", |_| Ok(())).is_err());
}

#[test]
fn normal_boot_requires_initialization_and_missing_all_unconsumed_rows_is_not_empty_success() {
    let mut bare = bare_fixture();
    assert!(bare.audit("roll", |_| Ok(())).is_err());
    let inv = invocation(GOD, 1, json!({}));
    let mut txn = bare.store.transaction(&bare.service.records).unwrap();
    assert!(
        bare.service
            .prepare_reserve(&bare.world, &inv, &bare.reserve, token(1), &mut txn)
            .is_err()
    );
    drop(txn);
    assert!(bare.store.is_empty());

    let mut f = fixture();
    let (key, object) = f.reserve(1);
    let mut txn = f.store.transaction(&f.service.records).unwrap();
    txn.remove(&f.service.records, &key).unwrap();
    txn.remove(&f.service.open, &GOD).unwrap();
    let typed = txn.prepare().unwrap().into_parts().1;
    f.store.install(typed).unwrap();
    assert!(f.world.get(object).is_some());
    assert!(
        f.audit("roll", |_| Ok(())).is_err(),
        "birth count retains the lost reservation obligation"
    );
    let mut txn = f.store.transaction(&f.service.deployment).unwrap();
    txn.remove(&f.service.deployment, &()).unwrap();
    let typed = txn.prepare().unwrap().into_parts().1;
    f.store.install(typed).unwrap();
    assert!(
        f.audit("roll", |_| Ok(())).is_err(),
        "missing marker never means initialized empty"
    );
}

#[test]
fn initialization_is_one_actual_operator_commit_and_cannot_repeat_over_live_families() {
    let mut f = bare_fixture();
    let inv = invocation(ObjId(0), 99, json!({}));
    let mut txn = f.store.transaction(&f.service.deployment).unwrap();
    assert!(
        f.service
            .initialize(&f.world, &inv, &f.initialize, token(99), &mut txn)
            .is_err()
    );
    drop(txn);
    assert!(f.store.is_empty());
    let mut f = fixture();
    let inv = invocation(GOD, 99, json!({}));
    let mut txn = f.store.transaction(&f.service.deployment).unwrap();
    assert!(
        f.service
            .initialize(&f.world, &inv, &f.initialize, token(99), &mut txn)
            .is_err()
    );
    drop(txn);
    assert_eq!(f.world.next_id(), 2);
    assert_eq!(
        f.store.get(&f.service.deployment, &()).unwrap().unwrap(),
        &ShellDeployment {
            format: FORMAT,
            initialized_by: token(99),
            born: 0,
            consumed: 0,
        }
    );
}
