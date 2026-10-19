//! Actual registered Industry adapter + Inventory K state. The fixture mints
//! ordinary Origin matter through Inventory; it makes no certification/payment claim.
#![allow(clippy::unwrap_used, clippy::panic, clippy::panic_in_result_fn)]
use super::*;
use crate::{
    EndpointRef, Grade, InventoryFamilies, InventoryHandleCell, InventoryModule, InventoryStore,
    LotId,
};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocationDraft, ActionOfferId, ActionProvider,
    AuthoritativeActionOutput, PreparedAuthoritativeAction, PreparedCausalRef,
};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use mush_core::composed::PreparedComposedTxn;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    RulesValue,
};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{
    ActionInvocation, ActionStateBootRecord, CanonicalHash, Engine, GOD, GameModule,
    IdempotencyKey, IdempotencyScope, ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite,
    ObjId, ObjKind, Outbox, PerceivedRef, PrepareCtx, PrincipalId, Registrar, RequestUid,
    ScopedIdempotencyKey,
};
use mush_economy::{EconomyModule, EconomyStore, ExternalRef};
use mush_testkit::{TestWorld, fold_module_ops, fold_module_ops_into};
use serde_json::json;
use smol_str::SmolStr;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Boot = HashMap<SmolStr, ModuleBootData>;
type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);

// ModuleBootData deliberately has no PartialEq. Compare every retained field,
// preserving document/ledger/stream entry order and raw ledger text, while
// canonicalizing only namespace/map order. A failure prints no source payload.
fn boot_image(boot: &Boot) -> BTreeMap<&str, serde_json::Value> {
    boot.iter()
        .map(|(namespace, data)| {
            (
                namespace.as_str(),
                json!({
                    "kv": data.kv,
                    "ledger": data.ledger,
                    "streams": data.streams,
                    "fetch_ms": data.fetch_ms,
                    "omitted": data.omitted,
                }),
            )
        })
        .collect()
}

fn owner() -> OwnerId {
    OwnerId::new("industry").unwrap()
}
fn descriptor(kind: &str, id: &str) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).unwrap(),
        DefinitionId::new(id).unwrap(),
        1,
        json!({"fixture":"exact_core_input_v1","id":id}),
    )
    .unwrap()
}
fn actions() -> [DefinitionRef; 4] {
    let mut builder = DefinitionRegistryBuilder::new();
    [
        ACCEPT_ACTION,
        COMPLETE_ACTION,
        CANCEL_ACTION,
        "wrong_core_input",
    ]
    .map(|id| builder.register(owner(), descriptor("action", id)).unwrap())
}
fn binding() -> CoreInputBinding {
    let a = actions();
    CoreInputBinding::new(a[0].clone(), a[1].clone(), a[2].clone()).unwrap()
}
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn identity(order: u64) -> CoreInputIdentity {
    CoreInputIdentity::new(
        order,
        digest("inventory.input_fixture.v1", &json!({"order":order})).unwrap(),
    )
}
#[derive(Clone, Default)]
struct ProbeState(Arc<AtomicU64>);
#[derive(Clone, Default)]
struct CaptureControl {
    mode: Arc<AtomicU64>,
    captured: Arc<Mutex<Option<Arc<CoreInputWitness>>>>,
}
struct ProbeOwner {
    input: RegisteredCoreInputs,
}
struct ProbePatch {
    witness: Arc<CoreInputWitness>,
    before: u64,
    after: u64,
}
struct ProbeBound<'a> {
    state: &'a mut ProbeState,
    after: u64,
}
impl BoundParticipant for ProbeBound<'_> {
    fn apply(self) {
        self.state.0.store(self.after, Ordering::SeqCst);
    }
}
impl OwnerParticipant for ProbeOwner {
    type State = ProbeState;
    type Request = CoreInputRequest;
    type Output = Arc<CoreInputWitness>;
    type Patch = ProbePatch;
    type Bound<'a> = ProbeBound<'a>;
    fn prepare(
        &self,
        s: &ProbeState,
        c: &OwnerPrepareContext<'_>,
        request: CoreInputRequest,
    ) -> std::result::Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        let witness = self
            .input
            .capture(c, request)
            .map_err(|_| ParticipantError::Authority)?;
        let before = s.0.load(Ordering::SeqCst);
        let after = before.checked_add(1).ok_or(ParticipantError::Authority)?;
        let writes = vec![ReplayWrite::ModuleJsonSet {
            namespace: owner(),
            key: mush_core::AuthoritativeKey::parse_canonical("core_input_probe").unwrap(),
            value: json!(after),
        }];
        Ok(OwnerPreparation::new(
            Arc::clone(&witness),
            ProbePatch {
                witness,
                before,
                after,
            },
            writes,
        ))
    }
    fn recheck(
        &self,
        s: &ProbeState,
        c: &OwnerPrepareContext<'_>,
        p: &ProbePatch,
    ) -> std::result::Result<(), ParticipantError> {
        if s.0.load(Ordering::SeqCst) != p.before {
            return Err(ParticipantError::Authority);
        }
        self.input
            .recheck(c, &p.witness)
            .map_err(|_| ParticipantError::Authority)
    }
    fn bind<'a>(
        self: Arc<Self>,
        s: &'a mut ProbeState,
        p: ProbePatch,
    ) -> std::result::Result<ProbeBound<'a>, ParticipantError> {
        if s.0.load(Ordering::SeqCst) != p.before {
            return Err(ParticipantError::Authority);
        }
        Ok(ProbeBound {
            state: s,
            after: p.after,
        })
    }
    fn accepts_write(&self, w: &ReplayWrite) -> bool {
        matches!(w, ReplayWrite::ModuleJsonSet{namespace,key,value} if namespace==&owner()&&key.as_str()=="core_input_probe"&&value.as_u64().is_some())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    order: u64,
    lot: LotId,
    endpoint: EndpointRef,
}
struct Provider {
    input: RegisteredCoreInputs,
    owner: mush_core::participant::ParticipantHandle<ProbeOwner>,
    outcome: DefinitionRef,
    phase: usize,
    control: CaptureControl,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        // The fixture exercises the actual owner and checked-preimage gates in prepare.
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> std::result::Result<AuthoritativeActionOutput, ActionError> {
        let refuse = || {
            ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "exact input fixture refused",
            )
            .unwrap()
        };
        if self.control.mode.load(Ordering::SeqCst) >= 2 {
            let captured = self
                .control
                .captured
                .lock()
                .map_err(|_| refuse())?
                .clone()
                .ok_or_else(refuse)?;
            let result = self.input.publish(txn, ctx, &captured);
            if self.control.mode.load(Ordering::SeqCst) == 3 {
                // Catching a comparison refusal must not rescue preparation.
                assert!(result.is_err());
                return Ok(AuthoritativeActionOutput::new(
                    RulesValue::new(self.outcome.clone(), json!({"caught":true}))
                        .map_err(|_| refuse())?,
                ));
            }
            let op = result.map_err(|_| refuse())?;
            return Ok(AuthoritativeActionOutput::new(
                RulesValue::new(self.outcome.clone(), json!({"order":op.identity().order()}))
                    .map_err(|_| refuse())?,
            ));
        }
        let p: Input = serde_json::from_value(inv.parameters().clone()).map_err(|_| refuse())?;
        let identity = identity(p.order);
        let request = match self.phase {
            0 | 3 => CoreInputRequest::Reserve {
                identity,
                lot: p.lot,
                endpoint: p.endpoint,
            },
            1 => {
                let old = self
                    .input
                    .handle
                    .operation(txn, p.order)
                    .map_err(|_| refuse())?
                    .ok_or_else(refuse)?;
                let [record] = old.book.records() else {
                    return Err(refuse());
                };
                CoreInputRequest::Consume {
                    identity,
                    endpoint: p.endpoint,
                    expires: record.expires().clone(),
                }
            }
            _ => CoreInputRequest::Release { identity },
        };
        let witness = ctx
            .prepare_owner(&self.owner, request)
            .map_err(|_| refuse())?;
        if self.control.mode.load(Ordering::SeqCst) == 1 {
            *self.control.captured.lock().map_err(|_| refuse())? = Some(witness);
            // The actual owner was invoked, but no capsule leaves this refusal.
            return Err(refuse());
        }
        let op = self
            .input
            .publish(txn, ctx, &witness)
            .map_err(|_| refuse())?;
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({"order":op.identity().order()}))
                .map_err(|_| refuse())?,
        ))
    }
}
struct Industry {
    cell: InventoryHandleCell,
    state: ProbeState,
    control: CaptureControl,
}
impl GameModule for Industry {
    fn name(&self) -> &'static str {
        "industry"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let f = self
            .cell
            .get()
            .ok_or_else(|| ModError::msg("Inventory must register first"))?;
        let input = f
            .core_inputs()
            .ok_or_else(|| ModError::msg("input config"))?
            .request(reg)?;
        for (kind, ids) in [
            (
                "action",
                vec![
                    ACCEPT_ACTION,
                    COMPLETE_ACTION,
                    CANCEL_ACTION,
                    "wrong_core_input",
                ],
            ),
            ("owner_participant", vec!["input_fixture"]),
            ("outcome", vec!["input_fixture"]),
        ] {
            let expected: BTreeMap<_, _> = ids
                .iter()
                .map(|id| {
                    let d = descriptor(kind, id);
                    (d.id().clone(), d)
                })
                .collect();
            reg.definition_validator(
                DefinitionKind::new(kind).unwrap(),
                move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
                    let d = ctx.definition();
                    let e = expected
                        .get(&d.reference().key().id)
                        .ok_or("unknown input fixture definition")?;
                    if d.reference().version() == e.version()
                        && d.payload() == e.payload()
                        && d.references() == e.references()
                        && d.is_current() == e.is_current()
                    {
                        Ok(())
                    } else {
                        Err("invalid input fixture definition".into())
                    }
                },
            )?;
        }
        let registered = [
            ACCEPT_ACTION,
            COMPLETE_ACTION,
            CANCEL_ACTION,
            "wrong_core_input",
        ]
        .map(|id| reg.definition(descriptor("action", id)))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
        let part = reg.definition(descriptor("owner_participant", "input_fixture"))?;
        let outcome = reg.definition(descriptor("outcome", "input_fixture"))?;
        reg.owner_participant(
            part,
            registered.iter().cloned().collect(),
            ProbeOwner {
                input: input.clone(),
            },
        )?;
        reg.state(self.state.clone());
        for (phase, action) in registered.into_iter().enumerate() {
            let owner = reg.request_owner_participant::<ProbeOwner>(action.clone())?;
            reg.action_provider(
                action,
                Provider {
                    input: input.clone(),
                    owner,
                    outcome: outcome.clone(),
                    phase,
                    control: self.control.clone(),
                },
            )?;
        }
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        _: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> std::result::Result<(), ModError> {
        let n = data
            .kv
            .iter()
            .find(|(k, _)| k.as_str() == "core_input_probe")
            .map(|(_, v)| v.as_u64().ok_or_else(|| ModError::msg("probe value")))
            .transpose()?
            .unwrap_or(0);
        self.state.0.store(n, Ordering::SeqCst);
        Ok(())
    }
}
fn modules(
    state: ProbeState,
    control: CaptureControl,
    configured: bool,
) -> (Vec<Box<dyn GameModule>>, InventoryHandleCell) {
    let cell: InventoryHandleCell = Arc::default();
    let mut inventory = InventoryModule::new().with_handle_cell(Arc::clone(&cell));
    if configured {
        inventory = inventory
            .with_quantity_delegates(["industry"])
            .with_core_inputs(binding());
    }
    let mut modules: Vec<Box<dyn GameModule>> =
        vec![Box::new(EconomyModule::new()), Box::new(inventory)];
    if configured {
        modules.push(Box::new(Industry {
            cell: Arc::clone(&cell),
            state,
            control,
        }));
    }
    (modules, cell)
}
fn invocation(
    phase: usize,
    n: u8,
    order: u64,
    lot: LotId,
    endpoint: EndpointRef,
    actor: ObjId,
) -> ActionInvocation {
    invocation_for(
        phase,
        n,
        order,
        lot,
        endpoint,
        actor,
        PrincipalId::Object { id: actor },
    )
}
fn invocation_for(
    phase: usize,
    n: u8,
    order: u64,
    lot: LotId,
    endpoint: EndpointRef,
    actor: ObjId,
    principal: PrincipalId,
) -> ActionInvocation {
    let request = RequestUid::from_bytes([n; 16]);
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([1; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"order":order,"lot":lot,"endpoint":endpoint}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: owner(),
                namespace: cid("input_fixture").unwrap(),
            },
            phase as u64,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap()
}
fn fold_writes(boot: &mut Boot, writes: Vec<ReplayWrite>) {
    fold_module_ops_into(
        boot,
        writes
            .into_iter()
            .map(|w| match w {
                ReplayWrite::ModuleJsonSet {
                    namespace,
                    key,
                    value,
                } => ModuleOp {
                    ns: namespace.as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: key.as_str().into(),
                        value,
                    },
                },
                ReplayWrite::ModuleDelete { namespace, key } => ModuleOp {
                    ns: namespace.as_str().into(),
                    op: ModuleWrite::Del {
                        key: key.as_str().into(),
                    },
                },
                _ => panic!("input must not mutate World/money"),
            })
            .collect(),
    );
}
struct Rig {
    w: TestWorld,
    f: InventoryFamilies,
    boot: Boot,
    endpoints: [EndpointRef; 2],
    state: ProbeState,
    control: CaptureControl,
}
impl Rig {
    fn new() -> Result<Self> {
        let state = ProbeState::default();
        let control = CaptureControl::default();
        let (modules, cell) = modules(state.clone(), control.clone(), true);
        let mut w = TestWorld::with_modules(modules);
        w.world_mut().set_now(1000);
        w.world_mut().set_game_clock(mush_core::clock::GameClock {
            ic_ratio: 1,
            epoch_offset: 0,
        });
        let mut endpoints = Vec::new();
        for name in ["Input depot", "Other depot"] {
            let object = w.world_mut().create(ObjKind::Thing, name, GOD, None)?;
            w.run(
                GOD,
                &format!("@vault/create {name}=container:#{}", object.0),
            );
            let vault = w
                .world()
                .module_state::<EconomyStore>()
                .ok_or("economy")?
                .snapshot()
                .vaults()
                .find(|v| v.name.as_str() == name)
                .ok_or("actual vault")?
                .id;
            endpoints.push(EndpointRef(vault));
        }
        let endpoints: [EndpointRef; 2] = endpoints.try_into().map_err(|_| "endpoints")?;
        let boot = fold_module_ops(w.take_module_ops());
        w.world_mut().take_dirty();
        Ok(Self {
            w,
            f: cell.get().ok_or("families")?.clone(),
            boot,
            endpoints,
            state,
            control,
        })
    }
    fn mint(&mut self, id: LotId, quantity: u64) -> Result {
        let f = self.f.clone();
        let endpoint = self.endpoints[0];
        self.native(|txn| {
            f.mint_origin(
                txn,
                endpoint,
                &cid("test.material").unwrap(),
                quantity,
                Grade::new(3).unwrap(),
                ExternalRef {
                    provider: "fixture".into(),
                    kind: "ordinary_origin".into(),
                    id: format!("{}", id.0).into(),
                    revision: Some(1),
                },
                id,
            )
            .map(|_| ())
        })
    }
    fn native(
        &mut self,
        body: impl FnOnce(&mut ActionTxn<'_>) -> std::result::Result<(), crate::InventoryError>,
    ) -> Result {
        let p = {
            let mut txn = self.w.engine_mut().action_transaction(&self.f.lots)?;
            body(&mut txn)?;
            txn.prepare()?
        };
        let (writes, install) = p.into_parts();
        self.w.engine_mut().install_prepared_action(install)?;
        fold_module_ops_into(&mut self.boot, writes);
        Ok(())
    }
    fn prep(
        &mut self,
        phase: usize,
        n: u8,
        order: u64,
        lot: LotId,
    ) -> Result<(Prepared, ActionInvocation)> {
        let inv = invocation(phase, n, order, lot, self.endpoints[0], GOD);
        let p = self
            .w
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&actions()[phase], &inv, token(n))?;
        Ok((p, inv))
    }
    fn stage(&mut self, p: Prepared, inv: ActionInvocation, phase: usize, n: u8) -> Result {
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = p.1.into_parts();
        let action = actions()[phase].clone();
        definitions.push(action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(n),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([2; 32]),
            action,
            provider: owner(),
            cause: PreparedCausalRef::new(inv.request(), None, 0)?,
            definitions,
            writes: p.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token(n))),
            semantic_events,
            eligible_witnesses,
            durable_outbox,
        })?;
        let guards = self
            .w
            .engine()
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())?
            .finish()?;
        self.w
            .engine_mut()
            .stage_authoritative_action(p.0, guards, bundle)?;
        Ok(())
    }
    fn take_install(&mut self) -> Result {
        let held = self.w.engine_mut().take_staged_action()?.ok_or("staged")?;
        let (bundle, install) = held.into_parts();
        self.w.engine_mut().install_composed_action(install)?;
        fold_writes(&mut self.boot, bundle.writes().to_vec());
        Ok(())
    }
    fn act(&mut self, phase: usize, n: u8, order: u64, lot: LotId) -> Result {
        let (p, inv) = self.prep(phase, n, order, lot)?;
        self.stage(p, inv, phase, n)?;
        self.take_install()
    }
    fn op(&self, order: u64) -> Result<&CoreInputOperation> {
        Ok(self
            .w
            .engine()
            .action_state()
            .get(
                &self
                    .f
                    .core_inputs
                    .as_ref()
                    .ok_or("handle")?
                    .shared
                    .operations,
                &order,
            )?
            .ok_or("operation")?)
    }
    fn cold(&self, mut boot: Boot, configured: bool) -> Result<(TestWorld, InventoryFamilies)> {
        let (modules, cell) = modules(ProbeState::default(), CaptureControl::default(), configured);
        let (_, registry) = mush_testkit::boot_world();
        let mut engine = Engine::with_modules(self.w.world().clone(), registry, modules)?;
        let mut typed = Vec::new();
        for (ns, data) in &mut boot {
            data.kv.retain(|(key, value)| {
                if mush_core::is_action_state_persistence_key(key) {
                    typed.push(ActionStateBootRecord::new(
                        ns.clone(),
                        key.clone(),
                        value.clone(),
                    ));
                    false
                } else {
                    true
                }
            });
        }
        engine.hydrate_action_state(typed)?;
        engine.modules_loaded(boot, &mut Outbox::new(), &mut mush_core::Effects::new())?;
        Ok((
            TestWorld::from_engine(engine),
            cell.get().ok_or("families")?.clone(),
        ))
    }
}

#[test]
fn actual_hold_consume_is_atomic_and_cold_keeps_exact_k_and_lot_census() -> Result {
    let mut r = Rig::new()?;
    let lot = LotId(7);
    r.mint(lot, 1)?;
    let (plain, _) = r.cold(r.boot.clone(), false)?;
    assert!(
        plain
            .world()
            .module_state::<InventoryStore>()
            .ok_or("plain Inventory")?
            .is_complete()
    );
    let before = r.boot.clone();
    let (p, inv) = r.prep(0, 2, 0, lot)?;
    assert!(
        r.w.engine()
            .action_state()
            .get(&r.f.core_inputs.as_ref().unwrap().shared.operations, &0)?
            .is_none()
    );
    assert!(
        boot_image(&r.boot) == boot_image(&before),
        "inert preparation"
    );
    r.stage(p, inv, 0, 2)?;
    r.take_install()?;
    assert_eq!(r.op(0)?.state()?, CoreInputState::Held);
    r.act(1, 3, 0, lot)?;
    assert_eq!(r.op(0)?.state()?, CoreInputState::Consumed);
    assert_eq!(r.op(0)?.terminal_commit(), Some(token(3)));
    assert!(r.w.engine().action_state().get(&r.f.lots, &lot)?.is_none());
    assert_eq!(
        r.w.engine()
            .action_state()
            .get(&r.f.lot_census, &r.endpoints[0])?
            .ok_or("census")?
            .get(),
        0
    );
    let (cold, f) = r.cold(r.boot.clone(), true)?;
    assert!(
        cold.world()
            .module_state::<InventoryStore>()
            .ok_or("store")?
            .is_complete()
    );
    assert_eq!(
        cold.engine()
            .action_state()
            .get(&f.core_inputs.as_ref().unwrap().shared.operations, &0)?,
        Some(r.op(0)?)
    );
    assert!(r.cold(r.boot.clone(), false).is_err());
    assert!(r.act(1, 4, 0, lot).is_err());
    assert!(r.act(2, 5, 0, lot).is_err());
    assert!(r.mint(lot, 1).is_err());
    Ok(())
}

#[test]
fn native_paths_protect_expired_hold_and_release_keeps_a_permanent_reusable_chain() -> Result {
    let mut r = Rig::new()?;
    let lot = LotId(8);
    let other = LotId(9);
    r.mint(lot, 1)?;
    r.mint(other, 2)?;
    r.act(0, 2, 0, lot)?;
    r.w.world_mut()
        .set_now(1000 + crate::RESERVATION_TTL_SECS + 1);
    let f = r.f.clone();
    let a = r.endpoints[0];
    let b = r.endpoints[1];
    let before = r.boot.clone();
    for mode in 0..4 {
        assert!(
            r.native(|tx| match mode {
                0 => f.split(tx, a, lot, 1, LotId(20)).map(|_| ()),
                1 => f.relocate(tx, lot, a, b).map(|_| ()),
                2 => f.merge(tx, a, &[other, lot], LotId(20)).map(|_| ()),
                _ => f.dissolve(tx, a, lot).map(|_| ()),
            })
            .is_err()
        );
    }
    assert!(
        boot_image(&r.boot) == boot_image(&before),
        "native refusals preserve evidence"
    );
    assert!(r.act(1, 3, 0, lot).is_err());
    r.act(2, 4, 0, lot)?;
    assert_eq!(r.op(0)?.state()?, CoreInputState::Released);
    r.act(0, 5, u64::MAX, lot)?;
    assert_eq!(r.op(u64::MAX)?.previous, Some(0));
    assert_eq!(r.op(u64::MAX)?.attempt, 2);
    assert!(r.act(0, 6, 2, lot).is_err());
    r.act(2, 7, u64::MAX, lot)?;
    r.native(|tx| f.relocate(tx, lot, a, b).map(|_| ()))?;
    let (cold, _) = r.cold(r.boot.clone(), true)?;
    assert!(
        cold.world()
            .module_state::<InventoryStore>()
            .unwrap()
            .is_complete()
    );
    r.native(|tx| f.dissolve(tx, b, lot).map(|_| ()))?;
    assert!(r.mint(lot, 1).is_err());
    let (cold, _) = r.cold(r.boot.clone(), true)?;
    assert!(
        cold.world()
            .module_state::<InventoryStore>()
            .unwrap()
            .is_complete()
    );
    Ok(())
}

#[test]
fn every_native_birth_rejects_the_released_then_retired_identity() -> Result {
    let mut r = Rig::new()?;
    let lot = LotId(10);
    r.mint(lot, 1)?;
    r.mint(LotId(11), 2)?;
    r.mint(LotId(12), 1)?;
    r.act(0, 2, 1, lot)?;
    r.act(2, 3, 1, lot)?;
    let f = r.f.clone();
    let a = r.endpoints[0];
    let b = r.endpoints[1];
    let commodity = cid("test.material")?;
    r.native(|tx| f.dissolve(tx, a, lot).map(|_| ()))?;
    let before = r.boot.clone();
    assert!(r.mint(lot, 1).is_err());
    assert!(
        r.native(|tx| f.split(tx, a, LotId(11), 1, lot).map(|_| ()))
            .is_err()
    );
    assert!(
        r.native(|tx| f.merge(tx, a, &[LotId(11), LotId(12)], lot).map(|_| ()))
            .is_err()
    );
    assert!(
        r.native(|tx| f
            .crystallize(tx, a, &commodity, 1, Grade::new(3).unwrap(), lot)
            .map(|_| ()))
            .is_err()
    );
    assert!(
        boot_image(&r.boot) == boot_image(&before),
        "retired identity refusals are inert"
    );
    // Real pool lineage and a real two-unit K hold exercise both sale births.
    r.native(|tx| f.dissolve(tx, a, LotId(11)).map(|_| ()))?;
    let now = r.w.world().world_ic_now();
    let holder = invocation(0, 8, 2, lot, a, GOD).idempotency().clone();
    let reservation = {
        let mut found = None;
        r.native(|tx| {
            found = Some(f.reserve(tx, a, &commodity, &holder, 2, &now)?);
            Ok(())
        })?;
        found.ok_or("hold")?
    };
    let before = r.boot.clone();
    for buyer in [true, false] {
        let sale = crate::LotSale {
            destination: b,
            sold: 1,
            grade: Grade::new(3)?,
            buyer_lot: if buyer { lot } else { LotId(30) },
            remainder_lot: if buyer { LotId(31) } else { lot },
        };
        assert!(
            r.native(|tx| f
                .settle_sale(tx, a, &commodity, reservation, 1, &now, token(9), sale)
                .map(|_| ()))
                .is_err()
        );
    }
    assert!(
        boot_image(&r.boot) == boot_image(&before),
        "both sale births refuse without consuming the pool hold"
    );
    Ok(())
}

#[test]
fn actual_action_actor_and_owner_generation_fences_refuse_without_publication() -> Result {
    let mut r = Rig::new()?;
    let lot = LotId(15);
    r.mint(lot, 1)?;
    let before = r.boot.clone();
    assert!(
        r.prep(3, 2, 1, lot).is_err(),
        "wrong actual registered action"
    );
    let crossed = invocation_for(
        0,
        3,
        1,
        lot,
        r.endpoints[0],
        GOD,
        PrincipalId::Object { id: ObjId(2) },
    );
    assert_ne!(
        crossed.principal(),
        &PrincipalId::Object {
            id: crossed.actor()
        }
    );
    assert!(
        r.w.engine_mut()
            .prepare_registered_composed_authoritative_action(&actions()[0], &crossed, token(3))
            .is_err()
    );
    let (p, inv) = r.prep(0, 4, 1, lot)?;
    r.state.0.fetch_add(1, Ordering::SeqCst);
    assert!(r.stage(p, inv, 0, 4).is_err());
    assert!(
        boot_image(&r.boot) == boot_image(&before),
        "wrong identity/generation preserves actual evidence"
    );
    Ok(())
}

#[test]
fn expiry_and_timeline_fences_stop_stage_and_take_but_owned_work_is_not_revalidated() -> Result {
    for at_take in [false, true] {
        let mut r = Rig::new()?;
        let lot = LotId(16);
        r.mint(lot, 1)?;
        r.act(0, 2, 1, lot)?;
        let (p, inv) = r.prep(1, 3, 1, lot)?;
        let before = r.boot.clone();
        if at_take {
            r.stage(p, inv, 1, 3)?;
            r.w.world_mut().set_now(1000 + crate::RESERVATION_TTL_SECS);
            assert!(r.w.engine_mut().take_staged_action().is_err());
        } else {
            r.w.world_mut().set_now(1000 + crate::RESERVATION_TTL_SECS);
            assert!(r.stage(p, inv, 1, 3).is_err());
        }
        assert!(
            boot_image(&r.boot) == boot_image(&before),
            "expired completion remains held"
        );
        assert_eq!(r.op(1)?.state()?, CoreInputState::Held);
        r.act(2, 4, 1, lot)?;
    }
    let mut r = Rig::new()?;
    let lot = LotId(17);
    r.mint(lot, 1)?;
    r.act(0, 2, 1, lot)?;
    let epoch = r.w.world().next_world_timeline_epoch(2)?;
    r.w.world_mut().append_world_timeline_epoch(epoch)?;
    assert!(r.prep(1, 3, 1, lot).is_err());
    r.act(2, 4, 1, lot)?;
    let mut r = Rig::new()?;
    r.mint(lot, 1)?;
    r.act(0, 2, 1, lot)?;
    let (p, inv) = r.prep(1, 3, 1, lot)?;
    r.stage(p, inv, 1, 3)?;
    let held = r.w.engine_mut().take_staged_action()?.ok_or("custody")?;
    r.w.world_mut().set_now(1000 + crate::RESERVATION_TTL_SECS);
    let (bundle, install) = held.into_parts();
    r.w.engine_mut().install_composed_action(install)?;
    fold_writes(&mut r.boot, bundle.writes().to_vec());
    assert_eq!(r.op(1)?.state()?, CoreInputState::Consumed);
    Ok(())
}

#[test]
fn local_bind_refusal_after_custody_applies_neither_owner_nor_typed_images() -> Result {
    let mut r = Rig::new()?;
    let lot = LotId(18);
    r.mint(lot, 1)?;
    let (p, inv) = r.prep(0, 2, 1, lot)?;
    r.stage(p, inv, 0, 2)?;
    let held = r.w.engine_mut().take_staged_action()?.ok_or("custody")?;
    let before = r.boot.clone();
    r.state.0.fetch_add(1, Ordering::SeqCst);
    let (_, install) = held.into_parts();
    assert!(r.w.engine_mut().install_composed_action(install).is_err());
    assert!(
        boot_image(&r.boot) == boot_image(&before),
        "no applied receipt"
    );
    assert!(r.w.engine().action_state().get(&r.f.lots, &lot)?.is_some());
    assert!(
        r.w.engine()
            .action_state()
            .get(&r.f.core_inputs.as_ref().unwrap().shared.operations, &1)?
            .is_none()
    );
    Ok(())
}

fn edit_family(boot: &mut Boot, family: &str, mut edit: impl FnMut(&mut Value)) -> Result {
    let rows = &mut boot.get_mut("inventory").ok_or("inventory docs")?.kv;
    let mut seen = 0;
    for (_, v) in rows {
        if v.get("family")
            .and_then(|f| f.get("id"))
            .and_then(Value::as_str)
            == Some(family)
        {
            edit(v);
            seen += 1;
        }
    }
    if seen != 1 {
        return Err("expected one exact family row".into());
    }
    Ok(())
}
#[test]
fn cold_audit_reconstructs_heads_census_and_exact_k_proof_and_sweep_keeps_refusal() -> Result {
    let mut r = Rig::new()?;
    let lot = LotId(19);
    r.mint(lot, 1)?;
    r.act(0, 2, 1, lot)?;
    for family in [
        registration::OPERATIONS,
        registration::HEADS,
        registration::CENSUS,
    ] {
        let mut boot = r.boot.clone();
        boot.get_mut("inventory")
            .ok_or("docs")?
            .kv
            .retain(|(_, v)| {
                v.get("family")
                    .and_then(|f| f.get("id"))
                    .and_then(Value::as_str)
                    != Some(family)
            });
        let mut legacy = boot.clone();
        for data in legacy.values_mut() {
            data.kv
                .retain(|(key, _)| !mush_core::is_action_state_persistence_key(key));
        }
        let (mut cold, _) = r.cold(boot, true)?;
        assert!(
            !cold
                .world()
                .module_state::<InventoryStore>()
                .unwrap()
                .is_complete()
        );
        // Re-run the actual module reconciliation with unchanged real legacy
        // evidence; no replacement Inventory state or fake health flag.
        cold.try_modules_loaded(legacy)?;
        assert!(
            !cold
                .world()
                .module_state::<InventoryStore>()
                .unwrap()
                .is_complete()
        );
    }
    for mode in 0..3 {
        let mut boot = r.boot.clone();
        edit_family(&mut boot, registration::HEADS, |v| {
            let h = v.get_mut("value").unwrap();
            match mode {
                0 => h["identity"]["order"] = json!(99),
                1 => h["lot"] = json!(99),
                _ => h["sequence"] = json!(2),
            };
        })?;
        match r.cold(boot, true) {
            Ok((cold, _)) => assert!(
                !cold
                    .world()
                    .module_state::<InventoryStore>()
                    .unwrap()
                    .is_complete()
            ),
            Err(_) => {}
        }
    }
    r.act(1, 3, 1, lot)?;
    let mut bad = r.boot.clone();
    edit_family(&mut bad, registration::OPERATIONS, |v| {
        v["value"]["terminal"]["commit"] = json!(token(4))
    })?;
    assert!(
        r.cold(bad, true).is_err(),
        "exact K commit differs from terminal proof"
    );
    // The full local inventory can be absent consistently. Only the mandatory
    // later independent Industry order/index join can detect this complete loss.
    let mut absent = r.boot.clone();
    absent.get_mut("inventory").unwrap().kv.retain(|(_, v)| {
        !v.get("family")
            .and_then(|f| f.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|f| {
                [
                    registration::OPERATIONS,
                    registration::HEADS,
                    registration::CENSUS,
                ]
                .contains(&f)
            })
    });
    let (cold, _) = r.cold(absent, true)?;
    assert!(
        cold.world()
            .module_state::<InventoryStore>()
            .unwrap()
            .is_complete()
    );
    Ok(())
}

#[test]
fn admission_preflights_complete_terminal_room_and_strict_wire_before_writing() -> Result {
    let mut r = Rig::new()?;
    let lot = LotId(21);
    r.mint(lot, 1)?;
    r.act(0, 2, 1, lot)?;
    let original = r.op(1)?.clone();
    writer::require_terminal_room(&original)?;
    let mut accepted = None;
    let mut largest = None;
    for count in (0..MAX_OPERATION_BYTES).step_by(64) {
        let source = ExternalRef {
            provider: "fixture".into(),
            kind: "ordinary_origin".into(),
            id: "\"\\".repeat(count).into(),
            revision: Some(u64::MAX),
        };
        let mut op = original.clone();
        op.lot = crate::Lot::new(
            lot,
            r.endpoints[0],
            cid("test.material")?,
            1,
            Grade::new(3)?,
            crate::Lineage::Origin { source },
            0,
        )?;
        if let Ok(sizes) = writer::require_terminal_room(&op) {
            assert!(sizes.into_iter().all(|size| size <= MAX_ENVELOPE_BYTES));
            assert!(sizes[1] > sizes[0] && sizes[2] > sizes[0]);
            largest = Some(op.clone());
        }
        if encode(&op).is_ok() && writer::require_terminal_room(&op).is_err() {
            accepted = Some(op);
            break;
        }
    }
    let largest = largest.ok_or("large complete terminal envelope")?;
    let source = match largest.lot.lineage() {
        crate::Lineage::Origin { source } => source.clone(),
        _ => return Err("origin".into()),
    };
    let f = r.f.clone();
    let endpoint = r.endpoints[0];
    let large_lot = LotId(24);
    r.native(|tx| {
        f.mint_origin(
            tx,
            endpoint,
            &cid("test.material").unwrap(),
            1,
            Grade::new(3).unwrap(),
            source,
            large_lot,
        )
        .map(|_| ())
    })?;
    r.act(0, 10, 3, large_lot)?;
    r.act(1, 11, 3, large_lot)?;
    assert_eq!(r.op(3)?.state()?, CoreInputState::Consumed);
    let (cold, _) = r.cold(r.boot.clone(), true)?;
    assert!(
        cold.world()
            .module_state::<InventoryStore>()
            .ok_or("store")?
            .is_complete()
    );
    let oversized = accepted.ok_or("held body fits but complete terminal must refuse")?;
    let f = r.f.clone();
    let newlot = LotId(22);
    let source = match oversized.lot.lineage() {
        crate::Lineage::Origin { source } => source.clone(),
        _ => return Err("origin".into()),
    };
    let endpoint = r.endpoints[0];
    r.native(|tx| {
        f.mint_origin(
            tx,
            endpoint,
            &cid("test.material").unwrap(),
            1,
            Grade::new(3).unwrap(),
            source,
            newlot,
        )
        .map(|_| ())
    })?;
    let before = r.boot.clone();
    assert!(r.prep(0, 3, 2, newlot).is_err());
    assert!(
        boot_image(&r.boot) == boot_image(&before),
        "capacity refusal is inert"
    );
    let mut unknown = encode(&original)?;
    unknown["extra"] = json!(true);
    assert!(decode::<CoreInputOperation>(&unknown).is_err());
    let mut null = encode(&original)?;
    null["terminal"] = Value::Null;
    assert!(decode::<CoreInputOperation>(&null).is_err());
    let mut crossed = original.clone();
    crossed.scalar = mush_core::BoundedScalar::new(0, 1, 0, 1)?;
    assert!(crossed.validate().is_err());
    Ok(())
}

#[test]
fn exact_owner_binding_rejects_same_token_cross_action_or_changed_invocation_even_if_caught()
-> Result {
    for mode in 0..4 {
        let mut r = Rig::new()?;
        let lot = LotId(23);
        r.mint(lot, 1)?;
        let before = r.boot.clone();
        let captured = invocation(0, 8, 1, lot, r.endpoints[0], GOD);
        r.control.mode.store(1, Ordering::SeqCst);
        assert!(
            r.w.engine_mut()
                .prepare_registered_composed_authoritative_action(
                    &actions()[0],
                    &captured,
                    token(8)
                )
                .is_err()
        );
        assert!(
            r.control
                .captured
                .lock()
                .map_err(|_| "capture lock")?
                .is_some()
        );
        r.control
            .mode
            .store(if mode == 3 { 3 } else { 2 }, Ordering::SeqCst);
        let (action, inv) = match mode {
            0 => (actions()[3].clone(), captured.clone()),
            1 => (
                actions()[0].clone(),
                invocation(0, 9, 1, lot, r.endpoints[0], GOD),
            ),
            _ => (
                actions()[0].clone(),
                invocation(0, 8, 2, lot, r.endpoints[0], GOD),
            ),
        };
        // Always reuse the exact commit token and actual owner handle.
        assert!(
            r.w.engine_mut()
                .prepare_registered_composed_authoritative_action(&action, &inv, token(8))
                .is_err()
        );
        assert!(
            boot_image(&r.boot) == boot_image(&before),
            "crossed preparation produces no evidence"
        );
        assert!(
            r.w.engine()
                .action_state()
                .get(&r.f.core_inputs.as_ref().unwrap().shared.operations, &1)?
                .is_none()
        );
        assert!(r.w.engine().action_state().get(&r.f.lots, &lot)?.is_some());
    }
    Ok(())
}
