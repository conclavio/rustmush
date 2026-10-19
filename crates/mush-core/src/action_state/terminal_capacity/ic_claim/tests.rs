//! Real Engine composition, registered action preparation, four owner binds,
//! typed installation and cold hydration. Claim JSON is inert fixture data;
//! these tests do not certify a physical host event or commercial entitlement.
use super::*;
use crate::action::*;
use crate::authoritative::{AuthoritativeKey, ReplayWrite};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    DefinitionValidationContext, RulesValue,
};
use crate::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
    ParticipantHandle,
};
use crate::{Engine, GameModule, ModError, Registrar, Registry, World};
use std::sync::{Mutex, OnceLock};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<String, Value>;
type Disk = BTreeMap<(SmolStr, SmolStr), Value>;
type Trace = Arc<Mutex<Vec<(u8, &'static str)>>>;
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn dg(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn err() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "IC fixture refusal",
    )
    .unwrap()
}
fn moderr(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
#[derive(Clone, Default)]
struct Cell<const N: u8>(u64);
struct Adapter<const N: u8> {
    trace: Trace,
}
struct Patch {
    before: u64,
    after: u64,
    refuse: bool,
}
struct Bound<'a, const N: u8> {
    state: &'a mut Cell<N>,
    after: u64,
    trace: Trace,
}
fn owner<const N: u8>() -> OwnerId {
    owner_id(protocol::OWNERS[usize::from(N)]).unwrap()
}
impl<const N: u8> BoundParticipant for Bound<'_, N> {
    fn apply(self) {
        self.trace.lock().unwrap().push((N, "apply"));
        self.state.0 = self.after;
    }
}
impl<const N: u8> OwnerParticipant for Adapter<N> {
    type State = Cell<N>;
    type Request = bool;
    type Output = ();
    type Patch = Patch;
    type Bound<'a> = Bound<'a, N>;
    fn prepare(
        &self,
        state: &Cell<N>,
        _: &OwnerPrepareContext<'_>,
        refuse: bool,
    ) -> Result<OwnerPreparation<(), Patch>, ParticipantError> {
        let after = state.0.checked_add(1).ok_or(ParticipantError::Encoding)?;
        Ok(OwnerPreparation::new(
            (),
            Patch {
                before: state.0,
                after,
                refuse,
            },
            vec![ReplayWrite::ModuleJsonSet {
                namespace: owner::<N>(),
                key: AuthoritativeKey::parse_canonical("ic_fixture/image").unwrap(),
                value: json!(after),
            }],
        ))
    }
    fn recheck(
        &self,
        state: &Cell<N>,
        _: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        if state.0 == patch.before {
            Ok(())
        } else {
            Err(ParticipantError::Slot)
        }
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut Cell<N>,
        patch: Patch,
    ) -> Result<Bound<'a, N>, ParticipantError> {
        self.trace.lock().unwrap().push((N, "bind"));
        if state.0 != patch.before || patch.refuse {
            return Err(ParticipantError::Slot);
        }
        Ok(Bound {
            state,
            after: patch.after,
            trace: self.trace.clone(),
        })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        matches!(write,ReplayWrite::ModuleJsonSet{namespace,key,value}
        if namespace==&owner::<N>() && key.as_str()=="ic_fixture/image" && value.is_u64())
    }
}
#[derive(Clone)]
struct Config {
    enabled: bool,
    images: usize,
    missing_grant: bool,
    refuse: bool,
    reverse: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            images: 4,
            missing_grant: false,
            refuse: false,
            reverse: false,
        }
    }
}
struct Defs {
    manifest_descriptor: DefinitionDescriptor,
    manifest: DefinitionRef,
    participants: [DefinitionRef; 4],
    actions: [DefinitionRef; 2],
    ordinary: DefinitionRef,
    outcome: DefinitionRef,
}
fn defs(generation: u64) -> TestResult<Arc<Defs>> {
    let manifest_descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("registry_manifest")?,
        DefinitionId::new("combat")?,
        1,
        json!({"generation":generation}),
    )?;
    let manifest = DefinitionRegistryBuilder::new()
        .register(owner_id("tacsim")?, manifest_descriptor.clone())?;
    let participants = protocol::participant_refs()?;
    let actions = [
        protocol::action_reference(&manifest, &participants, 0)?,
        protocol::action_reference(&manifest, &participants, 1)?,
    ];
    let other = |kind: &str, id: &str| -> TestResult<DefinitionRef> {
        Ok(DefinitionRegistryBuilder::new().register(
            owner_id("tacsim")?,
            DefinitionDescriptor::new(
                DefinitionKind::new(kind)?,
                DefinitionId::new(id)?,
                1,
                json!({}),
            )?,
        )?)
    };
    Ok(Arc::new(Defs {
        manifest_descriptor,
        manifest,
        participants,
        actions,
        ordinary: other("action", "ordinary_ic_fixture")?,
        outcome: other("outcome", "ic_fixture")?,
    }))
}
struct Module<const N: u8> {
    defs: Arc<Defs>,
    handle: Arc<OnceLock<Handle>>,
    trace: Trace,
    config: Config,
}
fn validate_kind(
    reg: &mut Registrar<'_>,
    kind: &str,
    expected: Vec<(DefinitionRef, DefinitionDescriptor)>,
) -> Result<(), ModError> {
    reg.definition_validator(
        DefinitionKind::new(kind).map_err(moderr)?,
        move |ctx: &DefinitionValidationContext<'_>| {
            let actual = ctx.definition();
            let Some((_, descriptor)) = expected
                .iter()
                .find(|(reference, _)| reference == actual.reference())
            else {
                return Err("IC fixture definition".into());
            };
            if actual.payload() == descriptor.payload() {
                Ok(())
            } else {
                Err("IC fixture payload".into())
            }
        },
    )
}
impl<const N: u8> GameModule for Module<N> {
    fn name(&self) -> &'static str {
        protocol::OWNERS[usize::from(N)]
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let (_, descriptor) = protocol::participant_descriptors()
            .map_err(moderr)?
            .into_iter()
            .nth(usize::from(N))
            .ok_or_else(|| moderr("owner"))?;
        validate_kind(
            reg,
            "owner_participant",
            vec![(
                self.defs.participants[usize::from(N)].clone(),
                descriptor.clone(),
            )],
        )?;
        let participant = reg.definition(descriptor)?;
        reg.state(Cell::<N>::default());
        let allowed = self
            .defs
            .actions
            .iter()
            .cloned()
            .chain([self.defs.ordinary.clone()])
            .collect();
        reg.owner_participant(
            participant,
            allowed,
            Adapter::<N> {
                trace: self.trace.clone(),
            },
        )?;
        if N == 0 {
            let handle = reg.state_family(
                family_descriptor().map_err(moderr)?,
                |k: &String| Ok(k.clone()),
                |v: &Value| Ok(v.clone()),
                |_: &Value| Ok(()),
                |v: &Value| Ok(v.clone()),
            )?;
            reg.delegate_state_family(&handle, "tacsim")?;
            self.handle
                .set(handle)
                .map_err(|_| moderr("duplicate handle"))?;
        }
        if N == 1 {
            validate_kind(
                reg,
                "registry_manifest",
                vec![(
                    self.defs.manifest.clone(),
                    self.defs.manifest_descriptor.clone(),
                )],
            )?;
            reg.definition(self.defs.manifest_descriptor.clone())?;
            let ordinary = DefinitionDescriptor::new(
                DefinitionKind::new("action").map_err(moderr)?,
                DefinitionId::new("ordinary_ic_fixture").map_err(moderr)?,
                1,
                json!({}),
            )
            .map_err(moderr)?;
            let outcome = DefinitionDescriptor::new(
                DefinitionKind::new("outcome").map_err(moderr)?,
                DefinitionId::new("ic_fixture").map_err(moderr)?,
                1,
                json!({}),
            )
            .map_err(moderr)?;
            let mut action_definitions = Vec::new();
            for index in 0..2 {
                action_definitions.push((
                    self.defs.actions[index].clone(),
                    protocol::action_descriptor(
                        &self.defs.manifest,
                        &self.defs.participants,
                        index,
                    )
                    .map_err(moderr)?,
                ));
            }
            action_definitions.push((self.defs.ordinary.clone(), ordinary));
            validate_kind(reg, "action", action_definitions.clone())?;
            validate_kind(
                reg,
                "outcome",
                vec![(self.defs.outcome.clone(), outcome.clone())],
            )?;
            reg.definition(outcome)?;
            for (index, (reference, descriptor)) in action_definitions.into_iter().enumerate() {
                reg.definition(descriptor)?;
                let first = reg.request_owner_participant::<Adapter<0>>(reference.clone())?;
                let second = reg.request_owner_participant::<Adapter<1>>(reference.clone())?;
                let third = reg.request_owner_participant::<Adapter<2>>(reference.clone())?;
                let fourth = if self.config.missing_grant && index == 1 {
                    None
                } else {
                    Some(reg.request_owner_participant::<Adapter<3>>(reference.clone())?)
                };
                reg.action_provider(
                    reference,
                    Provider {
                        index,
                        defs: self.defs.clone(),
                        handle: self.handle.clone(),
                        first,
                        second,
                        third,
                        fourth,
                        config: self.config.clone(),
                    },
                )?;
            }
            if self.config.enabled {
                reg.ic_mind_claim_capacity(
                    self.defs.manifest.clone(),
                    self.defs.participants.clone(),
                    self.defs.actions.clone(),
                )?;
            }
        }
        Ok(())
    }
}
struct Provider {
    index: usize,
    defs: Arc<Defs>,
    handle: Arc<OnceLock<Handle>>,
    first: ParticipantHandle<Adapter<0>>,
    second: ParticipantHandle<Adapter<1>>,
    third: ParticipantHandle<Adapter<2>>,
    fourth: Option<ParticipantHandle<Adapter<3>>>,
    config: Config,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut crate::resolution::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let row = inv.parameters().get("row").cloned().ok_or_else(err)?;
        let operation: CommitToken =
            parsed(row.pointer("/open/operation").ok_or_else(err)?).map_err(|_| err())?;
        let handle = self.handle.get().ok_or_else(err)?;
        if self.index == 1 {
            txn.replace(handle, &operation.to_hex(), row)
                .map_err(|_| err())?;
        } else {
            txn.insert(handle, &operation.to_hex(), row)
                .map_err(|_| err())?;
        }
        if self.config.images > 0 {
            ctx.prepare_owner(&self.first, false).map_err(|_| err())?;
        }
        if self.config.images > 1 {
            ctx.prepare_owner(&self.second, false).map_err(|_| err())?;
        }
        if self.config.images > 2 {
            ctx.prepare_owner(&self.third, self.config.refuse)
                .map_err(|_| err())?;
        }
        if self.config.images > 3 {
            ctx.prepare_owner(self.fourth.as_ref().ok_or_else(err)?, false)
                .map_err(|_| err())?;
        }
        if self.index == 0 {
            ctx.reserve_ic_mind_claim_completion(txn)
                .map_err(|_| err())?;
        }
        if self.index == 1 {
            ctx.close_ic_mind_claim_completion(txn, operation)
                .map_err(|_| err())?;
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.defs.outcome.clone(), json!({})).map_err(|_| err())?,
        ))
    }
}
struct Fixture {
    engine: Engine,
    defs: Arc<Defs>,
    handle: Handle,
    trace: Trace,
    disk: Disk,
}
fn fixture(generation: u64, config: Config) -> TestResult<Fixture> {
    let defs = defs(generation)?;
    let handle = Arc::new(OnceLock::new());
    let trace = Arc::new(Mutex::new(Vec::new()));
    macro_rules! module {
        ($n:literal) => {
            Box::new(Module::<$n> {
                defs: defs.clone(),
                handle: handle.clone(),
                trace: trace.clone(),
                config: config.clone(),
            }) as Box<dyn GameModule>
        };
    }
    let mut modules = vec![module!(0), module!(1), module!(2), module!(3)];
    if config.reverse {
        modules.reverse();
    }
    let engine = Engine::with_modules(World::new_minimal(), Registry::new(), modules)?;
    Ok(Fixture {
        engine,
        defs,
        handle: handle.get().ok_or("handle")?.clone(),
        trace,
        disk: Disk::new(),
    })
}
fn protocol_value(d: &Defs) -> Value {
    json!({"manifest":d.manifest,"participants":{"retention":d.participants[0],"seal":d.participants[1],"custody":d.participants[2],"exclusion":d.participants[3]}})
}
fn open(d: &Defs, op: CommitToken) -> Value {
    json!({"format":1,"closed":null,"open":{"operation":op,"action":d.actions[0],"protocol":protocol_value(d),
        "unit":100,"sortie":1,"dispatcher":7,"hull_title_revision":1,"licence":"1","term":"1","charter":"1","line":"1","filed":"1",
        "revision":{"format":1,"revision":{"key":{"character":7,"mind":1,"number":1},"digest":dg(1)},"source":dg(2)},
        "rating":{"job":token(99),"id":dg(3),"schema":1,"receipt_digest":dg(4)},
        "window":{"format":1,"eligibility_start":0,"issued_at":0,"term_end":100,"grace_end":120},"capacity":{"kind":"paid_seats","seats":1},
        "at_ic":{"clock":{"kind":"world_ic","generation":1},"at":5},"seal_digest":dg(5),"custody_digest":dg(6),"exclusion_digest":dg(7)}})
}
fn close(d: &Defs, row: &Value, commit: CommitToken) -> TestResult<Value> {
    let mut next = row.clone();
    let open = field(row, "open")?;
    next["closed"] = json!({"operation":commit,"action":d.actions[1],"protocol":protocol_value(d),"open":field(open,"operation")?,
        "open_digest":open_digest(open)?,"unit":field(open,"unit")?,"sortie":field(open,"sortie")?,"transition":2,"event_sequence":1,"event_digest":dg(8),
        "kind":{"kind":"destroyed"},"at_ic":{"clock":{"kind":"world_ic","generation":2},"at":-100}});
    Ok(next)
}
fn invocation(row: Value, actor: ObjId) -> TestResult<ActionInvocation> {
    let request = RequestUid::from_bytes([1; 16]);
    let principal = PrincipalId::Object { id: actor };
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"row":row}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: owner_id("tacsim")?,
                namespace: CanonicalId::new("ic_fixture")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
fn prepare(
    f: &mut Fixture,
    index: usize,
    row: Value,
    commit: CommitToken,
    actor: ObjId,
) -> TestResult<crate::composed::PreparedComposedTxn> {
    let action = if index == 2 {
        &f.defs.ordinary
    } else {
        f.defs.actions.get(index).ok_or("action")?
    };
    Ok(f.engine
        .prepare_registered_composed_authoritative_action(action, &invocation(row, actor)?, commit)?
        .0)
}
fn install(f: &mut Fixture, prepared: crate::composed::PreparedComposedTxn) -> TestResult {
    let (writes, installation) = prepared.into_parts();
    for write in writes {
        match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => {
                f.disk
                    .insert((namespace.as_str().into(), key.as_str().into()), value);
            }
            ReplayWrite::ModuleDelete { namespace, key } => {
                f.disk
                    .remove(&(namespace.as_str().into(), key.as_str().into()));
            }
            _ => return Err("unexpected write".into()),
        }
    }
    f.engine.install_composed_action_announcing(
        installation,
        &mut crate::Outbox::new(),
        &mut crate::engine::Effects::new(),
    )?;
    Ok(())
}
fn typed_disk(disk: &Disk) -> Disk {
    disk.iter()
        .filter(|((_, key), _)| key.starts_with("action-state/"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
fn hydrate(f: &Fixture, disk: &Disk) -> TestResult<ActionStateStore> {
    let mut store = ActionStateStore::new(f.engine.action_state().registry.clone());
    store.hydrate(
        typed_disk(disk)
            .into_iter()
            .map(|((ns, key), value)| ActionStateBootRecord::new(ns, key, value)),
    )?;
    Ok(store)
}
fn unchanged(f: &Fixture) -> TestResult {
    assert!(f.engine.action_state().is_empty());
    assert_eq!(f.engine.world.module_state::<Cell<0>>().ok_or("roll")?.0, 0);
    assert_eq!(f.engine.world.module_state::<Cell<1>>().ok_or("seal")?.0, 0);
    assert_eq!(
        f.engine.world.module_state::<Cell<2>>().ok_or("custody")?.0,
        0
    );
    assert_eq!(
        f.engine
            .world
            .module_state::<Cell<3>>()
            .ok_or("exclusion")?
            .0,
        0
    );
    Ok(())
}

#[test]
fn actual_four_owner_open_close_and_tacsim_metadata_install_only_after_all_binds() -> TestResult {
    for reverse in [false, true] {
        let mut f = fixture(
            1,
            Config {
                reverse,
                ..Config::default()
            },
        )?;
        let row = open(&f.defs, token(1));
        let p = prepare(&mut f, 0, row.clone(), token(1), ObjId(7))?;
        unchanged(&f)?;
        let writes = p.authoritative_replay_writes();
        assert_eq!(writes.len(), 6);
        assert!(writes.iter().any(|w|matches!(w,ReplayWrite::ModuleJsonSet{namespace,key,..} if namespace.as_str()=="tacsim" && is_capacity_key(key.as_str()))));
        install(&mut f, p)?;
        assert_eq!(f.engine.action_state().capacity.records.len(), 1);
        let store = f.engine.action_state();
        assert_eq!(store.capacity_usage.records, 2);
        assert_eq!(
            store.capacity_usage.bytes,
            CEILING + store.capacity.metadata_bytes()
        );
        let trace = f.trace.lock().unwrap().clone();
        assert_eq!(trace.len(), 8);
        assert!(trace.iter().take(4).all(|(_, step)| *step == "bind"));
        assert!(trace.iter().skip(4).all(|(_, step)| *step == "apply"));
        hydrate(&f, &f.disk)?;
        let closed = close(&f.defs, &row, token(2))?;
        let p = prepare(&mut f, 1, closed, token(2), ObjId(42))?;
        install(&mut f, p)?;
        assert!(f.engine.action_state().capacity.is_empty());
        hydrate(&f, &f.disk)?;
        assert_eq!(
            f.engine
                .world
                .module_state::<Cell<3>>()
                .ok_or("exclusion")?
                .0,
            2
        );
    }
    Ok(())
}
#[test]
fn missing_grant_partial_images_ordinary_action_and_last_bind_refuse() -> TestResult {
    assert!(
        fixture(
            1,
            Config {
                missing_grant: true,
                ..Config::default()
            }
        )
        .is_err()
    );
    for count in 0..4 {
        let mut f = fixture(
            1,
            Config {
                images: count,
                ..Config::default()
            },
        )?;
        let row = open(&f.defs, token(1));
        assert!(prepare(&mut f, 0, row, token(1), ObjId(7)).is_err());
        unchanged(&f)?;
    }
    let mut f = fixture(1, Config::default())?;
    let row = open(&f.defs, token(1));
    assert!(prepare(&mut f, 2, row, token(1), ObjId(7)).is_err());
    unchanged(&f)?;
    let mut f = fixture(
        1,
        Config {
            refuse: true,
            ..Config::default()
        },
    )?;
    let row = open(&f.defs, token(1));
    let p = prepare(&mut f, 0, row, token(1), ObjId(7))?;
    let (_, installation) = p.into_parts();
    assert!(
        f.engine
            .install_composed_action_announcing(
                installation,
                &mut crate::Outbox::new(),
                &mut crate::engine::Effects::new()
            )
            .is_err()
    );
    unchanged(&f)?;
    let trace = f.trace.lock().unwrap();
    assert_eq!(trace.len(), 4);
    assert!(trace.iter().all(|(_, step)| *step == "bind"));
    Ok(())
}
#[test]
fn crossed_open_actor_commit_protocol_and_closed_shape_refuse() -> TestResult {
    for mode in 0..6 {
        let mut f = fixture(1, Config::default())?;
        let mut row = open(&f.defs, token(1));
        match mode {
            0 => row["open"]["dispatcher"] = json!(8),
            1 => row["open"]["operation"] = json!(token(3)),
            2 => row["open"]["action"] = json!(f.defs.actions[1]),
            3 => row["open"]["protocol"]["participants"]["seal"]["digest"] = json!(dg(99)),
            4 => {
                row.as_object_mut().ok_or("row")?.remove("closed");
            }
            _ => row["open"]["capacity"]["actor"] = json!(7),
        };
        assert!(prepare(&mut f, 0, row, token(1), ObjId(7)).is_err());
        unchanged(&f)?;
    }
    Ok(())
}
#[test]
fn immutable_open_current_close_and_exact_event_joins_are_required() -> TestResult {
    for mode in 0..8 {
        let mut f = fixture(1, Config::default())?;
        let row = open(&f.defs, token(1));
        let p = prepare(&mut f, 0, row.clone(), token(1), ObjId(7))?;
        install(&mut f, p)?;
        let mut next = close(&f.defs, &row, token(2))?;
        match mode {
            0 => next["open"]["seal_digest"] = json!(dg(99)),
            1 => next["closed"]["operation"] = json!(token(3)),
            2 => next["closed"]["open"] = json!(token(4)),
            3 => next["closed"]["unit"] = json!(101),
            4 => next["closed"]["event_sequence"] = json!(0),
            5 => next["closed"]["transition"] = json!(1),
            6 => {
                next["closed"]["at_ic"] = json!({"clock":{"kind":"world_ic","generation":1},"at":4})
            }
            _ => next["closed"]["kind"] = json!({"kind":"match_end"}),
        };
        assert!(prepare(&mut f, 1, next, token(2), ObjId(42)).is_err());
        assert_eq!(f.engine.action_state().capacity.records.len(), 1);
        assert_eq!(
            f.engine.action_state().get(&f.handle, &token(1).to_hex())?,
            Some(&row)
        );
    }
    Ok(())
}
#[test]
fn cold_reverse_metadata_checks_and_old_manifest_drain_are_exact() -> TestResult {
    let mut old = fixture(1, Config::default())?;
    let row = open(&old.defs, token(1));
    let p = prepare(&mut old, 0, row.clone(), token(1), ObjId(7))?;
    install(&mut old, p)?;
    for mode in 0..4 {
        let mut disk = typed_disk(&old.disk);
        let metadata = disk
            .keys()
            .find(|(_, key)| is_capacity_key(key))
            .cloned()
            .ok_or("metadata")?;
        let claim = disk
            .keys()
            .find(|(_, key)| !is_capacity_key(key))
            .cloned()
            .ok_or("claim")?;
        match mode {
            0 => {
                disk.remove(&metadata);
            }
            1 => {
                disk.remove(&claim);
            }
            2 => {
                let value = disk.remove(&metadata).ok_or("metadata")?;
                disk.insert(("roll".into(), metadata.1), value);
            }
            _ => {
                disk.get_mut(&metadata).ok_or("metadata")?["protocol_fingerprint"] = json!(dg(99));
            }
        };
        assert!(hydrate(&old, &disk).is_err());
    }
    let disabled = fixture(
        1,
        Config {
            enabled: false,
            ..Config::default()
        },
    )?;
    assert!(hydrate(&disabled, &old.disk).is_err());
    let mut current = fixture(2, Config::default())?;
    current.engine.hydrate_action_state(
        typed_disk(&old.disk)
            .into_iter()
            .map(|((ns, key), value)| ActionStateBootRecord::new(ns, key, value)),
    )?;
    current.disk = old.disk.clone();
    let next = close(&current.defs, &row, token(2))?;
    let p = prepare(&mut current, 1, next, token(2), ObjId(42))?;
    install(&mut current, p)?;
    hydrate(&current, &current.disk)?;
    let fresh = open(&old.defs, token(3));
    assert!(prepare(&mut current, 0, fresh, token(3), ObjId(7)).is_err());
    Ok(())
}
#[test]
fn ordinary_typed_bypass_cannot_create_mutate_or_delete_claims() -> TestResult {
    let mut f = fixture(1, Config::default())?;
    let row = open(&f.defs, token(1));
    {
        let mut txn = f.engine.action_transaction(&f.handle)?;
        txn.insert(&f.handle, &token(1).to_hex(), row.clone())?;
        assert!(txn.prepare().is_err());
    }
    let p = prepare(&mut f, 0, row.clone(), token(1), ObjId(7))?;
    install(&mut f, p)?;
    {
        let mut txn = f.engine.action_transaction(&f.handle)?;
        txn.replace(&f.handle, &token(1).to_hex(), row)?;
        assert!(txn.prepare().is_err());
    }
    {
        let mut txn = f.engine.action_transaction(&f.handle)?;
        txn.remove(&f.handle, &token(1).to_hex())?;
        assert!(txn.prepare().is_err());
    }
    Ok(())
}
#[test]
fn complete_maximum_close_and_exact_fixed_descriptor_are_bounded() -> TestResult {
    let f = fixture(1, Config::default())?;
    let mut raw = open(&f.defs, token(1));
    for name in ["licence", "term", "charter", "line", "filed"] {
        raw["open"][name] = json!(u64::MAX.to_string());
    }
    raw["open"]["sortie"] = json!(u64::MAX - 1);
    raw["open"]["hull_title_revision"] = json!(u64::MAX);
    raw["open"]["capacity"] = json!({"kind":"paid_seats","seats":1024});
    let claim = Claim::parse(raw, token(1))?;
    let protocol = f
        .engine
        .action_state()
        .registry
        .ic_mind_claim
        .as_ref()
        .ok_or("protocol")?;
    require_close_room(&claim, token(1), protocol)?;
    let targets = targets(token(1))?;
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].ceiling, 16384);
    let mut builder = StateFamilyRegistryBuilder::new();
    let descriptor = family_descriptor()?.with_limits(192, CEILING - 1)?;
    let handle = builder.register(
        owner_id("roll")?,
        descriptor,
        |k: &String| Ok(k.clone()),
        |v: &Value| Ok(v.clone()),
        |_: &Value| Ok(()),
        |v: &Value| Ok(v.clone()),
    )?;
    builder.delegate_family(owner_id("roll")?, &handle, owner_id("tacsim")?)?;
    builder.ic_mind_claim_capacity(
        &owner_id("tacsim")?,
        f.defs.manifest.clone(),
        f.defs.participants.clone(),
        f.defs.actions.clone(),
    )?;
    let owners = protocol::OWNERS
        .into_iter()
        .map(owner_id)
        .collect::<Result<BTreeSet<_>, _>>()?;
    assert!(
        builder
            .freeze_for_owners_with_readiness(
                &owners,
                MechanicalOwnerReadiness::for_owners(&owners)
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn exact_registration_rejects_wrong_owner_swapped_actions_and_descriptor_drift() -> TestResult {
    let f = fixture(1, Config::default())?;
    assert!(
        Protocol::new(
            &owner_id("roll")?,
            f.defs.manifest.clone(),
            f.defs.participants.clone(),
            f.defs.actions.clone()
        )
        .is_err()
    );
    let mut actions = f.defs.actions.clone();
    actions.swap(0, 1);
    assert!(
        Protocol::new(
            &owner_id("tacsim")?,
            f.defs.manifest.clone(),
            f.defs.participants.clone(),
            actions
        )
        .is_err()
    );
    let mut participants = f.defs.participants.clone();
    let actual = participants.get(1).ok_or("participant")?;
    participants[1] = DefinitionRef::new(actual.key().clone(), actual.version(), dg(99))?;
    assert!(
        Protocol::new(
            &owner_id("tacsim")?,
            f.defs.manifest.clone(),
            participants,
            f.defs.actions.clone()
        )
        .is_err()
    );
    let mut builder = StateFamilyRegistryBuilder::new();
    builder.ic_mind_claim_capacity(
        &owner_id("tacsim")?,
        f.defs.manifest.clone(),
        f.defs.participants.clone(),
        f.defs.actions.clone(),
    )?;
    assert!(
        builder
            .ic_mind_claim_capacity(
                &owner_id("tacsim")?,
                f.defs.manifest.clone(),
                f.defs.participants.clone(),
                f.defs.actions.clone()
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn transferred_owned_claim_keeps_exact_open_through_close_and_cold_hydration() -> TestResult {
    let mut f = fixture(1, Config::default())?;
    let mut row = open(&f.defs, token(1));
    row["format"] = json!(2);
    row["open"]["capacity"] = json!({"kind":"owned_dispatch"});
    row["open"]["charter_ownership"] =
        json!({"generation":2,"operation":token(17),"digest":dg(19)});
    let claimed = Claim::parse(row.clone(), token(1))?;
    let p = prepare(&mut f, 0, row.clone(), token(1), ObjId(7))?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    let mut crossed = close(&f.defs, &row, token(2))?;
    crossed["open"]["charter_ownership"]["generation"] = json!(3);
    crossed["closed"]["open_digest"] = json!(open_digest(field(&crossed, "open")?)?);
    assert!(prepare(&mut f, 1, crossed, token(2), ObjId(42)).is_err());
    let closed = close(&f.defs, &row, token(2))?;
    let closing = Claim::parse(closed.clone(), token(1))?;
    assert_eq!(claimed.fingerprint, closing.fingerprint);
    let p = prepare(&mut f, 1, closed, token(2), ObjId(42))?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    Ok(())
}

#[test]
fn charter_ownership_is_versioned_bounded_and_never_a_paid_claim_field() -> TestResult {
    let f = fixture(1, Config::default())?;
    let old = open(&f.defs, token(1));
    Claim::parse(old.clone(), token(1))?;
    let mut next = old.clone();
    next["format"] = json!(2);
    next["open"]["capacity"] = json!({"kind":"owned_dispatch"});
    next["open"]["charter_ownership"] =
        json!({"generation":65_537,"operation":token(17),"digest":dg(19)});
    Claim::parse(next.clone(), token(1))?;
    assert_ne!(
        open_digest(field(&old, "open")?)?,
        open_digest(field(&next, "open")?)?
    );
    for generation in [0, 1, 65_538, u64::MAX] {
        let mut bad = next.clone();
        bad["open"]["charter_ownership"]["generation"] = json!(generation);
        assert!(Claim::parse(bad, token(1)).is_err());
    }
    for ownership in [
        Value::Null,
        json!({"generation":2,"operation":token(17)}),
        json!({"generation":2,"operation":token(17),"digest":dg(19),"authority":true}),
    ] {
        let mut bad = next.clone();
        bad["open"]["charter_ownership"] = ownership;
        assert!(Claim::parse(bad, token(1)).is_err());
    }
    let mut bad = next.clone();
    bad["format"] = json!(1);
    assert!(Claim::parse(bad, token(1)).is_err());
    let mut bad = next.clone();
    bad["open"]["capacity"] = json!({"kind":"paid_seats","seats":1});
    assert!(Claim::parse(bad, token(1)).is_err());
    let mut bad = next;
    bad["open"]
        .as_object_mut()
        .ok_or("open")?
        .remove("charter_ownership");
    assert!(Claim::parse(bad, token(1)).is_err());
    let mut bad = old;
    bad["open"]["charter_ownership"] = Value::Null;
    assert!(Claim::parse(bad, token(1)).is_err());
    Ok(())
}
