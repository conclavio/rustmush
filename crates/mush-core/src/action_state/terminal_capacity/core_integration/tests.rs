#![cfg(test)]

//! Actual Engine/registered transaction tests of Core's fixed capacity proof.
//! Inert Value codecs stand in for business rows: these tests confer no E4,
//! certified die, physical output or supplier authority.
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
use std::sync::Mutex;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Handle = StateFamilyHandle<String, Value>;
type Handles = Arc<Mutex<BTreeMap<String, Handle>>>;
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
        "fixed core capacity fixture refused",
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
    owner_id(["industry", "economy"][usize::from(N)]).unwrap()
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
                key: AuthoritativeKey::parse_canonical("core_capacity_fixture/image").unwrap(),
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
        if namespace==&owner::<N>() && key.as_str()=="core_capacity_fixture/image" && value.is_u64())
    }
}
#[derive(Clone)]
struct Defs {
    policy: Value,
    recipe: DefinitionDescriptor,
    participants: [DefinitionRef; 2],
    actions: [DefinitionRef; 5],
    delivery: Option<DefinitionRef>,
    ordinary: DefinitionRef,
    outcome: DefinitionRef,
}
fn participant(n: usize) -> TestResult<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant")?,
        DefinitionId::new(if n == 0 {
            "core_production"
        } else {
            "initial_filing"
        })?,
        if n == 0 { 2 } else { 1 },
        json!({"fixture":"core_capacity"}),
    )?)
}
fn descriptor_ref(owner: &str, d: DefinitionDescriptor) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(owner_id(owner)?, d)?)
}
fn defs(version: u64) -> TestResult<Arc<Defs>> {
    defs_profile(version, false)
}
fn defs_profile(version: u64, delivery_profile: bool) -> TestResult<Arc<Defs>> {
    let recipe = DefinitionDescriptor::new(
        DefinitionKind::new("recipe")?,
        DefinitionId::new("core_integrate")?,
        1,
        json!({"version":version}),
    )?;
    let recipe_ref = descriptor_ref("industry", recipe.clone())?;
    let mut policy = json!({"recipe":recipe_ref,"factory":100,"output_container":101});
    if delivery_profile {
        policy["delivery"] = json!("personal_custody_v1");
    }
    let actions = std::array::from_fn(|i| action_reference(&policy, i).unwrap());
    let ordinary = descriptor_ref("industry", simple("action", "core_capacity_fixture"))?;
    let outcome = descriptor_ref("industry", simple("outcome", "core_capacity_fixture"))?;
    Ok(Arc::new(Defs {
        policy,
        recipe,
        participants: [
            descriptor_ref("industry", participant(0)?)?,
            descriptor_ref("economy", participant(1)?)?,
        ],
        actions,
        delivery: delivery_profile
            .then(delivery::action_reference)
            .transpose()?,
        ordinary,
        outcome,
    }))
}
fn simple(kind: &str, id: &str) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).unwrap(),
        DefinitionId::new(id).unwrap(),
        1,
        json!({}),
    )
    .unwrap()
}
fn validate(
    reg: &mut Registrar<'_>,
    kind: &str,
    rows: Vec<(DefinitionRef, DefinitionDescriptor)>,
) -> Result<(), ModError> {
    reg.definition_validator(
        DefinitionKind::new(kind).map_err(moderr)?,
        move |ctx: &DefinitionValidationContext<'_>| {
            if rows.iter().any(|(r, d)| {
                ctx.definition().reference() == r && ctx.definition().payload() == d.payload()
            }) {
                Ok(())
            } else {
                Err("capacity fixture exact definition".into())
            }
        },
    )
}
struct Module<const N: u8> {
    defs: Arc<Defs>,
    handles: Handles,
    trace: Trace,
    enabled: bool,
    fault: u8,
}
impl<const N: u8> GameModule for Module<N> {
    fn name(&self) -> &'static str {
        ["industry", "economy", "inventory"][usize::from(N)]
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if N < 2 {
            let d = participant(usize::from(N)).map_err(moderr)?;
            validate(
                reg,
                "owner_participant",
                vec![(self.defs.participants[usize::from(N)].clone(), d.clone())],
            )?;
            let p = reg.definition(d)?;
            reg.state(Cell::<N>::default());
            reg.owner_participant(
                p,
                self.defs
                    .actions
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !(self.fault == 3 && N == 1 && *i == 0))
                    .map(|(_, a)| a.clone())
                    .chain([self.defs.ordinary.clone()])
                    .chain(self.defs.delivery.iter().cloned())
                    .collect(),
                Adapter::<N> {
                    trace: self.trace.clone(),
                },
            )?;
        }
        let mut all = Vec::new();
        for (owner, name, _) in FAMILIES
            .into_iter()
            .chain(self.defs.delivery.iter().flat_map(|_| delivery::FAMILIES))
        {
            if owner == self.name() {
                all.push((name.to_owned(), descriptor(name).map_err(moderr)?));
            }
        }
        for (owner, name) in [
            ("industry", "industry_core_integration_deployment"),
            ("industry", "industry_capacity_index"),
            ("inventory", "inventory_core_input_heads"),
            ("inventory", "inventory_core_input_census"),
        ] {
            if owner == self.name() {
                all.push((
                    name.into(),
                    StateFamilyDescriptor::new(
                        StateFamilyId::new(name).map_err(moderr)?,
                        CanonicalId::new("fixture.key").map_err(moderr)?,
                        CanonicalId::new("fixture.value").map_err(moderr)?,
                        1,
                        CanonicalId::new("fixture.decode").map_err(moderr)?,
                        CanonicalId::new("fixture.validate").map_err(moderr)?,
                        CanonicalId::new("fixture.canonical").map_err(moderr)?,
                    )
                    .map_err(moderr)?,
                ));
            }
        }
        for (name, mut d) in all {
            if self.fault == 1 && name == ORDER {
                d = d.with_limits(64, ORDER_CEILING - 1).map_err(moderr)?;
            }
            let h = reg.state_family(
                d,
                |k: &String| Ok(k.clone()),
                |v: &Value| Ok(v.clone()),
                |_: &Value| Ok(()),
                |v: &Value| Ok(v.clone()),
            )?;
            if N == 2 && self.fault != 2 {
                reg.delegate_state_family(&h, "industry")?;
            }
            self.handles.lock().unwrap().insert(name, h);
        }
        if N == 0 {
            let rr = descriptor_ref("industry", self.defs.recipe.clone()).map_err(moderr)?;
            validate(reg, "recipe", vec![(rr, self.defs.recipe.clone())])?;
            reg.definition(self.defs.recipe.clone())?;
            let mut actions = Vec::new();
            for i in 0..5 {
                actions.push((
                    self.defs.actions[i].clone(),
                    protocol::action_descriptor(&self.defs.policy, i).map_err(moderr)?,
                ));
            }
            actions.push((
                self.defs.ordinary.clone(),
                simple("action", "core_capacity_fixture"),
            ));
            if let Some(action) = &self.defs.delivery {
                let descriptor = DefinitionDescriptor::new(DefinitionKind::new("action").map_err(moderr)?,DefinitionId::new("deliver_core_output").map_err(moderr)?,1,
                    json!({"format":1,"protocol":"certified_core_delivery_v1","authority":"actual_completed_requester","effect":"ordinary_personal_custody","current_recipe_required":false,"core_receipt_bytes":16384,"industry_receipt_bytes":4096})).map_err(moderr)?;
                actions.push((action.clone(), descriptor));
            }
            validate(reg, "action", actions.clone())?;
            let d = simple("outcome", "core_capacity_fixture");
            validate(reg, "outcome", vec![(self.defs.outcome.clone(), d.clone())])?;
            reg.definition(d)?;
            for (index, (action, d)) in actions.into_iter().enumerate() {
                reg.definition(d)?;
                let first = reg.request_owner_participant::<Adapter<0>>(action.clone())?;
                let second = reg.request_owner_participant::<Adapter<1>>(action.clone())?;
                reg.action_provider(
                    action,
                    Provider {
                        index,
                        defs: self.defs.clone(),
                        handles: self.handles.clone(),
                        first,
                        second,
                    },
                )?;
            }
            if self.enabled {
                let mut actions = self.defs.actions.clone();
                if self.fault == 4 {
                    actions.swap(0, 1);
                }
                if let Some(delivery) = &self.defs.delivery {
                    let [a, b, c, d, e] = actions;
                    reg.core_integration_delivery_capacity(
                        self.defs.participants.clone(),
                        [a, b, c, d, e, delivery.clone()],
                    )?;
                } else {
                    reg.core_integration_capacity(self.defs.participants.clone(), actions)?;
                }
                if self.fault == 5 {
                    reg.core_integration_capacity(
                        self.defs.participants.clone(),
                        self.defs.actions.clone(),
                    )?;
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Mutation {
    family: String,
    key: String,
    value: Option<Value>,
}
struct Provider {
    index: usize,
    defs: Arc<Defs>,
    handles: Handles,
    first: ParticipantHandle<Adapter<0>>,
    second: ParticipantHandle<Adapter<1>>,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut crate::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let writes: Vec<Mutation> =
            serde_json::from_value(inv.parameters().get("writes").cloned().ok_or_else(err)?)
                .map_err(|_| err())?;
        for write in writes {
            let h = self
                .handles
                .lock()
                .unwrap()
                .get(&write.family)
                .cloned()
                .ok_or_else(err)?;
            let exists = txn.get(&h, &write.key).map_err(|_| err())?.is_some();
            match (write.value, exists) {
                (Some(v), true) => txn.replace(&h, &write.key, v),
                (Some(v), false) => txn.insert(&h, &write.key, v),
                (None, true) => txn.remove(&h, &write.key),
                (None, false) => return Err(err()),
            }
            .map_err(|_| err())?;
        }
        if self.index < 5 || self.index == 6 {
            ctx.prepare_owner(&self.first, false).map_err(|_| err())?;
            if matches!(self.index, 0 | 3 | 4) {
                ctx.prepare_owner(&self.second, false).map_err(|_| err())?;
            }
        }
        if inv.parameters().get("marker") == Some(&json!(true)) {
            let order = inv
                .parameters()
                .get("order")
                .and_then(Value::as_u64)
                .ok_or_else(err)?;
            if self.index == 0 {
                ctx.reserve_core_integration_completion(txn, order)
            } else {
                ctx.advance_core_integration_completion(txn, order)
            }
            .map_err(|_| err())?;
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.defs.outcome.clone(), json!({})).map_err(|_| err())?,
        ))
    }
}
struct Fixture {
    engine: Box<Engine>,
    defs: Arc<Defs>,
    handles: Handles,
    disk: Disk,
    trace: Trace,
}
fn fixture(version: u64, enabled: bool, reverse: bool) -> TestResult<Fixture> {
    fixture_fault(version, enabled, reverse, 0)
}
fn fixture_fault(version: u64, enabled: bool, reverse: bool, fault: u8) -> TestResult<Fixture> {
    fixture_profile(version, enabled, reverse, fault, false)
}
#[inline(never)]
fn fixture_profile(
    version: u64,
    enabled: bool,
    reverse: bool,
    fault: u8,
    delivery: bool,
) -> TestResult<Fixture> {
    let defs = defs_profile(version, delivery)?;
    let handles = Arc::new(Mutex::new(BTreeMap::new()));
    let trace = Arc::new(Mutex::new(Vec::new()));
    macro_rules! m {
        ($n:literal) => {
            Box::new(Module::<$n> {
                defs: defs.clone(),
                handles: handles.clone(),
                trace: trace.clone(),
                enabled,
                fault,
            }) as Box<dyn GameModule>
        };
    }
    let mut modules = vec![m!(0), m!(1), m!(2)];
    if reverse {
        modules.reverse();
    }
    let engine = Engine::with_modules(World::new_minimal(), Registry::new(), modules)?;
    Ok(Fixture {
        engine: Box::new(engine),
        defs,
        handles,
        disk: BTreeMap::new(),
        trace,
    })
}
fn invocation(
    writes: Vec<Mutation>,
    order: u64,
    marker: bool,
    actor: ObjId,
    principal: Option<PrincipalId>,
) -> TestResult<ActionInvocation> {
    let request = RequestUid::from_bytes([1; 16]);
    let principal = principal.unwrap_or(PrincipalId::Object { id: actor });
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"writes":writes,"order":order,"marker":marker}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: owner_id("industry")?,
                namespace: CanonicalId::new("core_capacity_fixture")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
#[allow(clippy::too_many_arguments)]
fn prepare(
    f: &mut Fixture,
    index: usize,
    writes: Vec<Mutation>,
    order: u64,
    commit: CommitToken,
    marker: bool,
    actor: ObjId,
    principal: Option<PrincipalId>,
) -> TestResult<crate::composed::PreparedComposedTxn> {
    let action = if index == 6 {
        f.defs.delivery.clone().ok_or("delivery definition")?
    } else if index == 5 {
        f.defs.ordinary.clone()
    } else {
        f.defs.actions[index].clone()
    };
    Ok(f.engine
        .prepare_registered_composed_authoritative_action(
            &action,
            &invocation(writes, order, marker, actor, principal)?,
            commit,
        )?
        .0)
}
fn install(f: &mut Fixture, p: crate::composed::PreparedComposedTxn) -> TestResult {
    let (writes, installation) = p.into_parts();
    for w in writes {
        match w {
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
            _ => return Err("unexpected fixture write".into()),
        }
    }
    f.engine.install_composed_action_announcing(
        installation,
        &mut crate::Outbox::new(),
        &mut crate::engine::Effects::new(),
    )?;
    Ok(())
}
fn put(k: Key, v: Value) -> Mutation {
    Mutation {
        family: k.family.as_str().into(),
        key: k.encoded.to_string(),
        value: Some(v),
    }
}
fn del(k: Key) -> Mutation {
    Mutation {
        family: k.family.as_str().into(),
        key: k.encoded.to_string(),
        value: None,
    }
}
fn extra(f: &str, k: &str, v: Value) -> Mutation {
    Mutation {
        family: f.into(),
        key: k.into(),
        value: Some(v),
    }
}
fn hydrate(f: &Fixture, disk: &Disk) -> TestResult<ActionStateStore> {
    let mut store = ActionStateStore::new(f.engine.action_state().registry.clone());
    store.hydrate(
        disk.iter()
            .filter(|((_, key), _)| key.starts_with("action-state/"))
            .map(|((ns, k), v)| ActionStateBootRecord::new(ns.clone(), k.clone(), v.clone())),
    )?;
    Ok(store)
}
fn current(f: &Fixture, k: &Key) -> TestResult<Value> {
    let h = f
        .handles
        .lock()
        .unwrap()
        .get(k.family.as_str())
        .cloned()
        .ok_or("handle")?;
    Ok(f.engine
        .action_state()
        .get(&h, &k.encoded.to_string())?
        .cloned()
        .ok_or("current row")?)
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: crate::ClockRef::WorldIc { generation: 1 },
        at,
    }
}
fn admission(d: &Defs, order: u64, op: CommitToken) -> TestResult<(Value, Vec<Mutation>)> {
    let intent = json!({"format":1,"order":order,"requester":7,"policy":d.policy,"lot":{"id":5,"endpoint":8,"quantity":1},"accepted_at":stamp(0),"expires_at":stamp(604800)});
    let sd = ScheduleId::derive(
        &owner_id("industry")?,
        &CanonicalId::new(format!("core.{order:016x}"))?,
    )?;
    let cursor = crate::DurableSchedule::new(crate::DurableScheduleDraft {
        id: sd,
        owner: owner_id("industry")?,
        clock: stamp(0).clock,
        next_due: stamp(3600),
        cadence: Some(3600),
        generation: 1,
        next_ordinal: 0,
        restart: crate::ScheduleRestartPolicy::Resume,
        catch_up_order: crate::ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 1,
    })?;
    let mut raw = json!({"format":1,"sequence":1,"intent":intent,"accepted":op,"action":d.actions[0],"funding":{"commit":op,"action":d.actions[0]},"capacity":crate::ReservationId(CanonicalHash::from_bytes([7;32])),"progress":{"order":order,"production":{},"cursor":cursor,"advances":[]}});
    let r = Row::parse(raw.clone(), order)?;
    let resource = r.resource()?;
    let holder = r.holder()?;
    let id = crate::ReservationId::derive(&holder, &resource)?;
    raw["capacity"] = json!(id);
    let scalar = BoundedScalar::new(0, 2, 2, 1)?;
    let book = ReservationBook::new(resource.clone(), scalar)?;
    let record = crate::ReservationRecord::held(id, resource, holder, 1, stamp(604800))?;
    let book = book.checked_hold(book.revision(), scalar, record, &stamp(0))?;
    let r = Row::parse(raw.clone(), order)?;
    let keys = r.keys()?;
    let k = |name| keys.get(name).cloned().ok_or("key");
    let input = json!({"identity":{"order":order,"intent":r.intent_digest()?},"lot":field(&r.intent,"lot")?,"accepted":op,"binding":{"accept":d.actions[0]}});
    let writes = vec![
        put(k(ORDER)?, raw.clone()),
        put(k(INPUT)?, input),
        put(k(SLOTS)?, json!(scalar)),
        put(k(BOOK)?, json!(book)),
        put(k(SCHEDULE)?, json!(cursor)),
        put(k(JOB)?, json!(order)),
        extra(
            "industry_core_integration_deployment",
            "registry",
            json!({"accepted":1}),
        ),
        extra(
            "inventory_core_input_heads",
            "0000000000000005",
            json!({"order":order}),
        ),
        extra("inventory_core_input_census", "registry", json!(1)),
        extra("industry_capacity_index", &id.0.to_hex(), json!(book.id())),
    ];
    Ok((raw, writes))
}
fn accepted(reverse: bool) -> TestResult<(Fixture, Value)> {
    accepted_profile(reverse, false)
}
fn accepted_profile(reverse: bool, delivery: bool) -> TestResult<(Fixture, Value)> {
    let mut f = fixture_profile(1, true, reverse, 0, delivery)?;
    let (raw, writes) = admission(&f.defs, 0, token(1))?;
    let r = Row::parse(raw.clone(), 0)?;
    let keys = r.keys()?;
    let seed = vec![
        put(keys[LOT].clone(), field(&r.intent, "lot")?.clone()),
        put(keys[CENSUS].clone(), json!(1)),
        put(
            keys[DEPLOYMENT].clone(),
            json!({"format":1,"initialized_by":token(9),"action":f.defs.ordinary,"born":0,"consumed":0}),
        ),
    ];
    let p = prepare(&mut f, 5, seed, 0, token(9), false, ObjId(7), None)?;
    install(&mut f, p)?;
    let p = prepare(&mut f, 0, writes, 0, token(1), true, ObjId(7), None)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    Ok((f, raw))
}
fn reserved(f: &Fixture, raw: &Value) -> TestResult<(Value, Vec<Mutation>)> {
    let r = Row::parse(raw.clone(), 0)?;
    let keys = r.keys()?;
    let pointer = json!({"object":200,"reservation":token(2),"action":f.defs.actions[1],"object_digest":dg(1)});
    let mut next = raw.clone();
    next["output"] = pointer.clone();
    let birth = crate::core_output::CoreOutputOrder::new(
        0,
        r.intent_digest()?,
        ObjId(7),
        parsed(field(&r.policy, "recipe")?)?,
        ObjId(100),
        ObjId(101),
    )?;
    let output = json!({"format":1,"birth":1,"purpose":"industry.core_integration","order":birth,"object":200,"created_at":0,"object_digest":dg(1),"reservation":token(2),"action":f.defs.actions[1],"state":{"kind":"reserved"}});
    let mut dep = current(f, &keys[DEPLOYMENT])?;
    dep["born"] = json!(1);
    Ok((
        next.clone(),
        vec![
            put(keys[ORDER].clone(), next),
            put(keys[OUTPUT].clone(), output),
            put(keys[DEPLOYMENT].clone(), dep),
        ],
    ))
}
fn terminal(
    f: &Fixture,
    raw: &Value,
    complete: bool,
    commit: CommitToken,
) -> TestResult<(Value, Vec<Mutation>)> {
    let r = Row::parse(raw.clone(), 0)?;
    let keys = r.keys()?;
    let index = if complete { 3 } else { 4 };
    let action = f.defs.actions[index].clone();
    let result = if complete {
        json!({"outcome":"completed","output":{"object":200,"reservation":token(2),"reservation_action":field(field(raw,"output")?,"action")?,"object_digest":dg(1)},"title":{}})
    } else {
        json!({"outcome":"cancelled"})
    };
    let t = json!({"commit":commit,"action":action,"at":stamp(3600),"result":result});
    let mut next = raw.clone();
    next["terminal"] = t;
    let mut input = current(f, &keys[INPUT])?;
    input["terminal"] = json!({"commit":commit,"action":action,"at":stamp(3600),"kind":if complete{"consume"}else{"release"}});
    let scalar: BoundedScalar = parsed(&current(f, &keys[SLOTS])?)?;
    let book: ReservationBook = parsed(&current(f, &keys[BOOK])?)?;
    let after =
        book.checked_release(book.revision(), scalar, parsed(field(raw, "capacity")?)?, 1)?;
    let mut writes = vec![
        put(keys[ORDER].clone(), next.clone()),
        put(keys[INPUT].clone(), input),
        put(keys[SLOTS].clone(), json!(scalar)),
        put(keys[BOOK].clone(), json!(after)),
        del(keys[SCHEDULE].clone()),
        del(keys[JOB].clone()),
    ];
    if complete {
        let mut output = current(f, &keys[OUTPUT])?;
        output["state"] = json!({"kind":"consumed","completion":commit,"action":action});
        let mut dep = current(f, &keys[DEPLOYMENT])?;
        dep["consumed"] = json!(1);
        writes.extend([
            del(keys[LOT].clone()),
            put(keys[CENSUS].clone(), json!(0)),
            put(keys[OUTPUT].clone(), output),
            put(keys[DEPLOYMENT].clone(), dep),
        ]);
    }
    Ok((next, writes))
}
fn metadata(f: &Fixture) -> Vec<Value> {
    f.disk
        .iter()
        .filter(|((_, k), _)| k.starts_with(PREFIX))
        .map(|(_, v)| v.clone())
        .collect()
}

#[test]
fn actual_registered_accept_reserve_complete_retains_exact_capacity_until_terminal() -> TestResult {
    for reverse in [false, true] {
        let (mut f, raw) = accepted(reverse)?;
        let before = metadata(&f);
        assert_eq!(before.len(), 1);
        let (next, writes) = reserved(&f, &raw)?;
        let p = prepare(&mut f, 1, writes, 0, token(2), true, ObjId(7), None)?;
        install(&mut f, p)?;
        assert_eq!(metadata(&f), before);
        hydrate(&f, &f.disk)?;
        let (_, writes) = terminal(&f, &next, true, token(3))?;
        let p = prepare(&mut f, 3, writes, 0, token(3), true, ObjId(7), None)?;
        install(&mut f, p)?;
        assert!(metadata(&f).is_empty());
        hydrate(&f, &f.disk)?;
        assert!(
            f.trace
                .lock()
                .unwrap()
                .iter()
                .any(|(n, event)| *n == 1 && *event == "apply")
        );
    }
    Ok(())
}
#[test]
fn cancel_drains_before_or_after_output_with_changed_recipe_and_preserves_old_identity()
-> TestResult {
    for reserve in [false, true] {
        let (mut old, mut raw) = accepted(false)?;
        if reserve {
            let (next, writes) = reserved(&old, &raw)?;
            let p = prepare(&mut old, 1, writes, 0, token(2), true, ObjId(7), None)?;
            install(&mut old, p)?;
            raw = next;
        }
        let mut f = fixture(2, true, false)?;
        f.engine.hydrate_action_state(
            old.disk
                .iter()
                .filter(|((_, k), _)| k.starts_with("action-state/"))
                .map(|((ns, k), v)| ActionStateBootRecord::new(ns.clone(), k.clone(), v.clone())),
        )?;
        f.disk = old.disk;
        let (_, writes) = terminal(&f, &raw, false, token(4))?;
        let p = prepare(&mut f, 4, writes, 0, token(4), true, ObjId(7), None)?;
        install(&mut f, p)?;
        assert!(metadata(&f).is_empty());
        hydrate(&f, &f.disk)?;
    }
    Ok(())
}
#[test]
fn exact_actual_actor_action_commit_and_marker_refuse_crossed_preparations() -> TestResult {
    for mode in 0..5 {
        let (mut f, raw) = accepted(false)?;
        let (_, mut writes) = reserved(&f, &raw)?;
        let (index, commit, marker, actor) = match mode {
            0 => (1, token(2), true, ObjId(8)),
            1 => (5, token(2), false, ObjId(7)),
            2 => (1, token(7), true, ObjId(7)),
            3 => (1, token(2), false, ObjId(7)),
            _ => {
                writes.pop();
                (1, token(2), true, ObjId(7))
            }
        };
        let before = f.disk.clone();
        assert!(prepare(&mut f, index, writes, 0, commit, marker, actor, None).is_err());
        assert!(before == f.disk);
        hydrate(&f, &f.disk)?;
    }
    Ok(())
}
#[test]
fn cold_missing_capacity_order_input_or_output_and_crossed_fingerprint_refuse() -> TestResult {
    let (mut f, raw) = accepted(false)?;
    let (_, writes) = reserved(&f, &raw)?;
    let p = prepare(&mut f, 1, writes, 0, token(2), true, ObjId(7), None)?;
    install(&mut f, p)?;
    for name in ["capacity", ORDER, INPUT, OUTPUT] {
        let mut disk = f.disk.clone();
        let key = disk
            .iter()
            .find(|((_, k), v)| {
                if name == "capacity" {
                    k.starts_with(PREFIX)
                } else {
                    v.pointer("/family/id") == Some(&json!(name))
                }
            })
            .map(|(k, _)| k.clone())
            .ok_or("target")?;
        disk.remove(&key);
        assert!(hydrate(&f, &disk).is_err());
    }
    let mut disk = f.disk.clone();
    let row = disk
        .iter_mut()
        .find(|((_, k), _)| k.starts_with(PREFIX))
        .ok_or("metadata")?
        .1;
    row["protocol_fingerprint"] = json!(dg(99));
    assert!(hydrate(&f, &disk).is_err());
    let disabled = fixture(1, false, false)?;
    assert!(hydrate(&disabled, &f.disk).is_err());
    Ok(())
}
#[test]
fn progress_uses_actual_native_occurrence_and_keeps_reservation_and_output_absent() -> TestResult {
    let (mut f, raw) = accepted(false)?;
    let r = Row::parse(raw.clone(), 0)?;
    let cursor: crate::DurableSchedule = parsed(field(field(&raw, "progress")?, "cursor")?)?;
    let crate::ScheduleEvaluation::Advance(a) = cursor.plan_due(
        cursor.revision(),
        &stamp(3600),
        crate::ScheduleDriveCause::Live,
    )?
    else {
        return Err("due".into());
    };
    let mut next = raw.clone();
    next["progress"]["cursor"] = json!(a.post_image());
    next["progress"]["advances"] =
        json!([{"commit":token(3),"observed":stamp(3600),"cause":"live","occurrences":1}]);
    let keys = r.keys()?;
    let writes = vec![
        put(keys[ORDER].clone(), next),
        put(keys[SCHEDULE].clone(), json!(a.post_image())),
    ];
    let principal = crate::scheduled_action_ingress(&f.defs.actions[2], &a, 1)?
        .principal()
        .clone();
    let before = metadata(&f);
    let p = prepare(
        &mut f,
        2,
        writes,
        0,
        token(3),
        true,
        ObjId(7),
        Some(principal),
    )?;
    install(&mut f, p)?;
    assert_eq!(metadata(&f), before);
    hydrate(&f, &f.disk)?;
    Ok(())
}
#[test]
fn future_full_envelopes_and_target_accounting_are_bounded_at_acceptance() -> TestResult {
    let (f, raw) = accepted(false)?;
    let r = Row::parse(raw, 0)?;
    let targets = targets(&r)?;
    assert_eq!(targets.len(), 10);
    assert_eq!(
        targets
            .iter()
            .filter(|t| t.mode == SlotMode::IndustryOutput)
            .count(),
        1
    );
    assert_eq!(targets.iter().map(|t| t.ceiling).sum::<usize>(), 354304);
    let usage = f.engine.action_state().capacity_usage;
    assert!(usage.bytes >= targets.iter().map(|t| t.ceiling).sum::<usize>());
    assert!(usage.records >= 11);
    Ok(())
}

#[test]
fn exact_composition_refuses_descriptor_delegation_grant_order_and_duplicate_drift() -> TestResult {
    for reverse in [false, true] {
        fixture(1, true, reverse)?;
        for fault in 1..=5 {
            assert!(
                fixture_fault(1, true, reverse, fault).is_err(),
                "closed composition fault {fault}"
            );
        }
    }
    Ok(())
}
fn sized(mut value: Value, name: &str, bytes: usize) -> TestResult<Value> {
    value
        .as_object_mut()
        .ok_or("sizing object")?
        .insert(name.into(), json!(""));
    let base = canonical_json_bytes_with_limit(&value, bytes)?.len();
    value[name] = json!("x".repeat(bytes.checked_sub(base).ok_or("sizing bound")?));
    assert_eq!(canonical_json_bytes_with_limit(&value, bytes)?.len(), bytes);
    Ok(value)
}
#[test]
fn maximal_nested_terminal_and_full_native_facility_book_fit_complete_envelopes() -> TestResult {
    let (f, mut raw) = accepted(false)?;
    let native_admission = raw.clone();
    raw["intent"] = sized(raw["intent"].clone(), "capacity_probe", 24576)?;
    raw["funding"] = sized(raw["funding"].clone(), "capacity_probe", 16384)?;
    let r = Row::parse(raw.clone(), 0)?;
    let view = View {
        registry: &f.engine.action_state().registry,
        records: &f.engine.action_state().records,
        mutations: None,
    };
    require_future_room(&view, &r)?;
    // Independent maximum component assembly, never a decoded receipt.
    raw["progress"] = sized(json!({}), "capacity_probe", 32768)?;
    raw["terminal"] = sized(json!({}), "capacity_probe", 16384)?;
    raw["output"] = json!({"object":u32::MAX,"reservation":token(255),"action":action_reference(&r.policy,1)?,"object_digest":dg(255)});
    let body = canonical_json_bytes_with_limit(&raw, 98304)?;
    assert!(body.len() > 90_000);
    let full = envelope(ORDER, "ffffffffffffffff", raw)?;
    assert!(canonical_json_bytes_with_limit(&full, ORDER_CEILING).is_ok());
    let resource = r.resource()?;
    let scalar = BoundedScalar::new(
        0,
        crate::MAX_RESERVATION_BOOK_RECORDS as i64,
        crate::MAX_RESERVATION_BOOK_RECORDS as i64,
        1,
    )?;
    let mut book = ReservationBook::new(resource.clone(), scalar)?;
    for order in 0..crate::MAX_RESERVATION_BOOK_RECORDS as u64 {
        let mut row = native_admission.clone();
        row["intent"]["order"] = json!(order);
        let holder = Row::parse(row, order)?.holder()?;
        let id = crate::ReservationId::derive(&holder, &resource)?;
        let held = crate::ReservationRecord::held(id, resource.clone(), holder, 1, stamp(604800))?;
        book = book.checked_hold(book.revision(), scalar, held, &stamp(0))?;
    }
    assert_eq!(book.records().len(), crate::MAX_RESERVATION_BOOK_RECORDS);
    let full = envelope(BOOK, &book.id().canonical_hash().to_hex(), json!(book))?;
    let ceiling = crate::MAX_RESERVATION_BOOK_BYTES + 1024;
    assert!(canonical_json_bytes_with_limit(&full, ceiling).is_ok());
    let id = book.records().first().ok_or("held native record")?.id();
    let closed = book.checked_release(book.revision(), scalar, id, 1)?;
    assert!(
        canonical_json_bytes_with_limit(
            &envelope(BOOK, &book.id().canonical_hash().to_hex(), json!(closed))?,
            ceiling
        )
        .is_ok()
    );
    Ok(())
}

#[path = "tests/delivery.rs"]
mod delivery_tests;

/// Doc 17h cut M8b-2 (H2; fix round 1, L2 and L3): an order's site is read from its intent.
/// With no `uncertified` die it is the policy's own factory and container; with one it is the
/// policy's `ungoverned` site; and an uncertified intent on a policy with no site is refused.
#[test]
fn m8b2_the_order_site_follows_the_intents_die() -> TestResult {
    let action = DefinitionRegistryBuilder::new().register(
        crate::OwnerId::new("industry")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new("accept_core_integration")?,
            1,
            json!({"fixture": true}),
        )?,
    )?;
    let row = |intent: Value| Row {
        raw: json!({}),
        order: 1,
        accepted: CommitToken::from_bytes([1; 16]),
        action: action.clone(),
        policy: intent["policy"].clone(),
        intent,
        fingerprint: DefinitionDigest::from_bytes([2; 32]),
    };
    let chartered = json!({"factory": 10, "output_container": 11});
    let composed = json!({"factory": 10, "output_container": 11,
        "ungoverned": {"factory": 20, "output_container": 21}});
    let certified = row(json!({"policy": composed, "certificate": {}}));
    assert_eq!(
        (certified.site_factory()?, certified.site_container()?),
        (ObjId(10), ObjId(11))
    );
    let back_room = row(json!({"policy": composed, "uncertified": {}}));
    assert_eq!(
        (back_room.site_factory()?, back_room.site_container()?),
        (ObjId(20), ObjId(21))
    );
    let no_site = row(json!({"policy": chartered, "uncertified": {}}));
    assert!(no_site.site_factory().is_err() && no_site.site_container().is_err());
    Ok(())
}
