#![cfg(test)]
//! Real composed Core fixture; Sim's canonical spatial planning is tested by Sim.
use super::*;
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocationDraft, ActionOfferId, ActionProvider,
    AuthoritativeActionOutput, CanonicalHash, IdempotencyKey, IdempotencyScope,
    PreparedAuthoritativeAction, PreparedCausalRef, RequestUid, ScopedIdempotencyKey,
};
use crate::authoritative::{
    AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeKey, ReplayWrite,
};
use crate::composed::PreparedComposedTxn;
use crate::mechanics::{DefinitionValidationContext, RulesValue};
use crate::participant::{BoundParticipant, OwnerPreparation};
use crate::{Engine, GOD, GameModule, Obj, ObjKind, Registry};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
struct PhysicalFixtureState(u64);
#[derive(Clone)]
struct Request {
    root: ObjId,
    moves: Vec<PhysicalMove>,
    read_dependencies: BTreeSet<ObjId>,
    refuse_bind: bool,
    caught: bool,
}
struct Patch {
    before: u64,
    refuse_bind: bool,
}
struct Adapter;
struct Bound<'a>(&'a mut PhysicalFixtureState);
impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        self.0.0 += 1;
    }
}
fn participant_descriptor() -> DefinitionDescriptor {
    participant_descriptor_version(2)
}
fn participant_descriptor_version(version: u32) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").unwrap(),
        DefinitionId::new(PARTICIPANT).unwrap(),
        version,
        json!({"fixture":"structural"}),
    )
    .unwrap()
}
fn action_ref() -> DefinitionRef {
    crate::DefinitionRegistryBuilder::new()
        .register(OwnerId::new("sim").unwrap(), action_descriptor().unwrap())
        .unwrap()
}
impl OwnerParticipant for Adapter {
    type State = PhysicalFixtureState;
    type Request = Request;
    type Output = PhysicalTransitionDraft;
    type Patch = Patch;
    type Bound<'a> = Bound<'a>;
    fn prepare(
        &self,
        state: &Self::State,
        ctx: &OwnerPrepareContext<'_>,
        request: Request,
    ) -> Result<OwnerPreparation<Self::Output, Patch>, ParticipantError> {
        let digest = DefinitionDigest::for_canonical_value(
            "physical.fixture",
            &json!({"generation":state.0}),
        )
        .unwrap();
        let draft = PhysicalTransitionDraft::new(
            ctx,
            request.root,
            digest,
            request.moves,
            request.read_dependencies,
        )?;
        Ok(OwnerPreparation::new(
            draft,
            Patch {
                before: state.0,
                refuse_bind: request.refuse_bind,
            },
            vec![ReplayWrite::ModuleJsonSet {
                namespace: OwnerId::new("sim").unwrap(),
                key: AuthoritativeKey::parse_canonical("physical_fixture").unwrap(),
                value: json!(state.0 + 1),
            }],
        ))
    }
    fn recheck(
        &self,
        state: &Self::State,
        _: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        if state.0 == patch.before {
            Ok(())
        } else {
            Err(refused())
        }
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut Self::State,
        patch: Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        if patch.refuse_bind || state.0 != patch.before {
            return Err(refused());
        }
        Ok(Bound(state))
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        matches!(write, ReplayWrite::ModuleJsonSet{namespace,key,value} if namespace.as_str()=="sim"
            && key.as_str()=="physical_fixture" && value.as_u64().is_some())
    }
}
#[derive(Default)]
struct Hook {
    veto: AtomicBool,
    calls: AtomicUsize,
    move_veto: AtomicBool,
    move_calls: AtomicUsize,
}
impl crate::EventHook for Hook {
    fn move_attempt(
        &self,
        ctx: &crate::HookReadContext<'_>,
        actor: ObjId,
        exit: Option<ObjId>,
        destination: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        self.move_calls.fetch_add(1, Ordering::SeqCst);
        let exact = exit.is_none()
            && ctx.physical_transition_attempt().is_some_and(|a| {
                a.matches_actor_projection(actor, destination) && a.commit() == token()
            });
        if !exact || self.move_veto.load(Ordering::SeqCst) {
            Err(mush_proto::RichText::plain("actor movement veto"))
        } else {
            Ok(())
        }
    }
    fn carry_attempt(
        &self,
        ctx: &crate::HookReadContext<'_>,
        actor: ObjId,
        object: ObjId,
        destination: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let exact = ctx.physical_transition_attempt().is_some_and(|a| {
            a.matches_projection(actor, object, destination)
                && a.action() == &action_ref()
                && a.commit() == token()
        });
        if !exact || self.veto.load(Ordering::SeqCst) {
            Err(mush_proto::RichText::plain("foreign hook veto"))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone)]
struct Handles {
    service: CorePhysicalTransitions<Adapter>,
    action: DefinitionRef,
    audited: Arc<Mutex<Vec<PhysicalTransitionReceipt>>>,
}
#[derive(Clone, Copy)]
enum History {
    None,
    Registered,
    Missing,
}
struct Sim {
    request: Arc<Mutex<Request>>,
    hook: Arc<Hook>,
    handles: Arc<Mutex<Option<Handles>>>,
    history: History,
}
fn validator(
    reg: &mut Registrar<'_>,
    descriptors: Vec<DefinitionDescriptor>,
) -> Result<(), ModError> {
    let kind = descriptors[0].kind().clone();
    reg.definition_validator(kind, move |ctx: &DefinitionValidationContext<'_>| {
        let d = ctx.definition();
        if descriptors.iter().any(|descriptor| {
            d.reference().key().id == *descriptor.id()
                && d.reference().version() == descriptor.version()
                && d.payload() == descriptor.payload()
        }) {
            Ok(())
        } else {
            Err("unexpected physical descriptor".into())
        }
    })
}
impl GameModule for Sim {
    fn name(&self) -> &'static str {
        "sim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let outcome = DefinitionDescriptor::new(
            DefinitionKind::new("outcome").unwrap(),
            DefinitionId::new("physical_fixture").unwrap(),
            1,
            json!({}),
        )
        .unwrap();
        validator(reg, vec![action_descriptor()?, action_descriptor_v1()?])?;
        validator(
            reg,
            vec![participant_descriptor(), participant_descriptor_version(1)],
        )?;
        validator(reg, vec![outcome.clone()])?;
        reg.state(PhysicalFixtureState(0));
        let action = reg.definition(action_descriptor()?)?;
        let participant = reg.definition(participant_descriptor())?;
        let outcome = reg.definition(outcome)?;
        reg.owner_participant(
            participant.clone(),
            BTreeSet::from([action.clone()]),
            Adapter,
        )?;
        let service = match self.history {
            History::None => CorePhysicalTransitions::register(reg, action.clone(), participant)?,
            History::Registered | History::Missing => {
                let old_action = action_descriptor_v1()?;
                let old_participant = participant_descriptor_version(1);
                let retained = if matches!(self.history, History::Registered) {
                    (
                        reg.definition(old_action)?,
                        reg.definition(old_participant)?,
                    )
                } else {
                    let mut definitions = crate::DefinitionRegistryBuilder::new();
                    (
                        definitions
                            .register(OwnerId::new("sim").unwrap(), old_action)
                            .unwrap(),
                        definitions
                            .register(OwnerId::new("sim").unwrap(), old_participant)
                            .unwrap(),
                    )
                };
                CorePhysicalTransitions::register_with_history(
                    reg,
                    action.clone(),
                    participant,
                    vec![retained],
                )?
            }
        };
        reg.hook(crate::HookPoint::CarryAttempt, self.hook.clone());
        reg.hook(crate::HookPoint::MoveAttempt, self.hook.clone());
        reg.action_provider(
            action.clone(),
            Provider {
                service: service.clone(),
                request: Arc::clone(&self.request),
                outcome,
            },
        )?;
        *self.handles.lock().unwrap() = Some(Handles {
            service,
            action,
            audited: Arc::new(Mutex::new(Vec::new())),
        });
        Ok(())
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        let handles = self.handles.lock().unwrap().clone().unwrap();
        handles.service.audit(ctx).map_err(bad)?;
        assert!(handles.service.scan_receipts(ctx, None, 0).is_err());
        assert!(
            handles
                .service
                .scan_receipts(ctx, None, MAX_PHYSICAL_RECEIPT_SCAN + 1)
                .is_err()
        );
        assert!(
            handles
                .service
                .read_receipt(ctx, &CommitToken::from_bytes([0; 16]))
                .map_err(bad)?
                .is_none()
        );
        let mut after = None;
        let mut receipts = Vec::new();
        loop {
            let page = handles
                .service
                .scan_receipts(ctx, after.as_deref(), 1)
                .map_err(bad)?;
            for receipt in page.entries() {
                let one = handles
                    .service
                    .read_receipt(ctx, &receipt.commit())
                    .map_err(bad)?
                    .unwrap();
                assert_eq!(one.row, receipt.row);
                receipts.push(one);
            }
            after = page.next_after().map(ToString::to_string);
            if after.is_none() {
                break;
            }
        }
        *handles.audited.lock().unwrap() = receipts;
        Ok(())
    }
}
struct Provider {
    service: CorePhysicalTransitions<Adapter>,
    request: Arc<Mutex<Request>>,
    outcome: DefinitionRef,
}
fn fail() -> ActionError {
    ActionError::new(ActionErrorCode::ProviderRejected, false, "physical refused").unwrap()
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        if inv.parameters() == &json!({}) {
            Ok(())
        } else {
            Err(fail())
        }
    }
    fn prepare_authoritative_with_context(
        &self,
        _: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut crate::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let request = self.request.lock().unwrap().clone();
        let caught = request.caught;
        let result = ctx.prepare_physical_transition(&self.service, txn, request);
        if caught {
            assert!(result.is_err());
        } else {
            result.map_err(|_| fail())?;
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({})).unwrap(),
        )
        .with_definitions(vec![self.outcome.clone()]))
    }
}
type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);
struct Rig {
    engine: Engine,
    handles: Handles,
    request: Arc<Mutex<Request>>,
    hook: Arc<Hook>,
    actor: ObjId,
    source: ObjId,
    destination: ObjId,
    first: ObjId,
    second: ObjId,
    child: ObjId,
}
fn token() -> CommitToken {
    CommitToken::from_bytes([7; 16])
}
fn rig() -> Rig {
    rig_with_history(History::None)
}
fn rig_with_history(history: History) -> Rig {
    let mut world = World::new_minimal();
    world.set_now(100);
    let source = world.create(ObjKind::Room, "source", GOD, None).unwrap();
    let destination = world
        .create(ObjKind::Room, "destination", GOD, None)
        .unwrap();
    let actor = world
        .create(ObjKind::Player, "operator", GOD, Some(source))
        .unwrap();
    let first = world
        .create(ObjKind::Thing, "first hull", GOD, Some(source))
        .unwrap();
    let second = world
        .create(ObjKind::Thing, "second hull", GOD, Some(source))
        .unwrap();
    let child = world
        .create(ObjKind::Npc, "occupant", GOD, Some(first))
        .unwrap();
    world.take_dirty();
    let request = Arc::new(Mutex::new(Request {
        root: first,
        read_dependencies: BTreeSet::new(),
        moves: vec![
            PhysicalMove {
                object: first,
                source: Some(source),
                destination,
            },
            PhysicalMove {
                object: second,
                source: Some(source),
                destination,
            },
        ],
        refuse_bind: false,
        caught: false,
    }));
    let hook = Arc::new(Hook::default());
    let handles = Arc::new(Mutex::new(None));
    let engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![Box::new(Sim {
            request: Arc::clone(&request),
            hook: Arc::clone(&hook),
            handles: Arc::clone(&handles),
            history,
        })],
    )
    .unwrap();
    let handles = handles.lock().unwrap().clone().unwrap();
    Rig {
        engine,
        handles,
        request,
        hook,
        actor,
        source,
        destination,
        first,
        second,
        child,
    }
}
impl Rig {
    fn invocation(&self) -> ActionInvocation {
        let request = RequestUid::from_bytes([7; 16]);
        let principal = PrincipalId::Object { id: self.actor };
        ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("sim").unwrap(),
                    namespace: CanonicalId::new("physical_fixture").unwrap(),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap()
    }
    fn prepare(&mut self) -> Result<Prepared, String> {
        self.engine
            .prepare_registered_composed_authoritative_action(
                &self.handles.action,
                &self.invocation(),
                token(),
            )
            .map_err(|e| e.to_string())
    }
    fn objects(&self) -> BTreeMap<ObjId, Obj> {
        [
            self.actor,
            self.source,
            self.destination,
            self.first,
            self.second,
            self.child,
        ]
        .into_iter()
        .map(|id| (id, self.engine.world.get(id).unwrap().clone()))
        .collect()
    }
    fn stage(&mut self, p: Prepared) -> Result<(), String> {
        let inv = self.invocation();
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = p.1.into_parts();
        definitions.push(self.handles.action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([3; 32]),
            action: self.handles.action.clone(),
            provider: OwnerId::new("sim").unwrap(),
            cause: PreparedCausalRef::new(inv.request(), None, 0).unwrap(),
            definitions,
            writes: p.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token())),
            semantic_events,
            eligible_witnesses,
            durable_outbox,
        })
        .map_err(|e| e.to_string())?;
        let guards = self
            .engine
            .registry
            .server_guard_set_builder(crate::RevisionSet::default())
            .unwrap()
            .finish()
            .unwrap();
        self.engine.world.take_dirty();
        self.engine
            .stage_authoritative_action(p.0, guards, bundle)
            .map_err(|e| e.to_string())
    }
    fn committed(&self) -> bool {
        self.engine
            .action_state()
            .get(&self.handles.service.records, &token())
            .unwrap()
            .is_some()
    }
}

#[test]
fn batch_merges_shared_containers_and_publishes_only_after_all_binds() {
    let mut r = rig();
    let before = r.objects();
    let p = r.prepare().unwrap();
    let images: Vec<_> =
        p.0.authoritative_replay_writes()
            .into_iter()
            .filter_map(|w| match w {
                ReplayWrite::ObjectSet { object } => Some(object),
                _ => None,
            })
            .collect();
    assert_eq!(images.len(), 4);
    assert_eq!(
        images.iter().map(|o| o.id).collect::<BTreeSet<_>>().len(),
        4
    );
    assert_eq!(r.objects(), before);
    r.stage(p).unwrap();
    assert_eq!(r.objects(), before);
    let (_, installation) = r.engine.take_staged_action().unwrap().unwrap().into_parts();
    let calls = r.hook.calls.load(Ordering::SeqCst);
    r.engine.install_composed_action(installation).unwrap();
    assert_eq!(r.hook.calls.load(Ordering::SeqCst), calls);
    assert_eq!(
        r.engine.world.get(r.source).unwrap().contents,
        vec![r.actor]
    );
    assert_eq!(
        r.engine.world.get(r.destination).unwrap().contents,
        vec![r.first, r.second]
    );
    assert_eq!(r.engine.world.get(r.child), before.get(&r.child));
    assert_eq!(
        r.engine.world.module_state::<PhysicalFixtureState>(),
        Some(&PhysicalFixtureState(1))
    );
    assert!(r.committed());
    assert!(r.prepare().is_err());
}

#[test]
fn moving_container_and_child_merge_without_losing_other_children() {
    let mut r = rig();
    r.request.lock().unwrap().moves.push(PhysicalMove {
        object: r.child,
        source: Some(r.first),
        destination: r.second,
    });
    let p = r.prepare().unwrap();
    let (_, installation) = p.0.into_parts();
    r.engine.install_composed_action(installation).unwrap();
    assert!(r.engine.world.get(r.first).unwrap().contents.is_empty());
    assert_eq!(
        r.engine.world.get(r.second).unwrap().contents,
        vec![r.child]
    );
    assert_eq!(
        r.engine.world.get(r.child).unwrap().location,
        Some(r.second)
    );
}

#[test]
fn zero_projection_still_commits_exact_spatial_patch_and_receipt() {
    let mut r = rig();
    r.request.lock().unwrap().moves.clear();
    let before = r.objects();
    let p = r.prepare().unwrap();
    assert!(
        !p.0.authoritative_replay_writes()
            .iter()
            .any(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
    );
    r.stage(p).unwrap();
    let (_, installation) = r.engine.take_staged_action().unwrap().unwrap().into_parts();
    r.engine.install_composed_action(installation).unwrap();
    assert_eq!(r.objects(), before);
    assert_eq!(r.hook.calls.load(Ordering::SeqCst), 0);
    assert!(r.committed());
}

#[test]
fn zero_projection_read_dependencies_survive_through_acknowledged_bind() {
    let mut r = rig();
    {
        let mut request = r.request.lock().unwrap();
        request.moves.clear();
        request.read_dependencies.insert(r.destination);
    }
    let p = r.prepare().unwrap();
    assert!(
        !p.0.authoritative_replay_writes()
            .iter()
            .any(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
    );
    r.stage(p).unwrap();
    let (_, installation) = r.engine.take_staged_action().unwrap().unwrap().into_parts();
    r.engine
        .world
        .set_name(r.destination, "changed proposed host after custody")
        .unwrap();
    let before = r.objects();
    assert!(r.engine.install_composed_action(installation).is_err());
    assert_eq!(r.objects(), before);
    assert_eq!(
        r.engine.world.module_state::<PhysicalFixtureState>(),
        Some(&PhysicalFixtureState(0))
    );
    assert!(!r.committed());

    let mut missing = rig();
    missing
        .request
        .lock()
        .unwrap()
        .read_dependencies
        .insert(ObjId(u32::MAX));
    assert!(missing.prepare().is_err());
    let mut oversized = rig();
    oversized.request.lock().unwrap().read_dependencies = (0..=MAX_PHYSICAL_READ_DEPENDENCIES
        as u32)
        .map(ObjId)
        .collect();
    assert!(oversized.prepare().is_err());
}

#[test]
fn duplicate_source_cycle_and_caught_refusals_never_publish() {
    for case in 0..5 {
        let mut r = rig();
        {
            let mut q = r.request.lock().unwrap();
            match case {
                0 => {
                    let m = q.moves[0].clone();
                    q.moves.push(m);
                }
                1 => q.moves[0].source = Some(r.destination),
                2 => {
                    q.moves[0].destination = r.second;
                    q.moves[1].destination = r.first;
                }
                3 => q.moves[0].destination = r.child,
                _ => {
                    q.moves[0].source = None;
                    q.caught = true;
                }
            }
        }
        let before = r.objects();
        assert!(r.prepare().is_err(), "case {case}");
        assert_eq!(r.objects(), before);
        assert!(!r.committed());
        assert_eq!(
            r.engine.world.module_state::<PhysicalFixtureState>(),
            Some(&PhysicalFixtureState(0))
        );
    }
}

#[test]
fn stale_source_ancestor_descendant_and_spatial_state_refuse_at_every_boundary() {
    for boundary in 0..3 {
        for target in 0..4 {
            let mut r = rig();
            let p = r.prepare().unwrap();
            let mut p = Some(p);
            let mut installation = None;
            if boundary > 0 {
                r.stage(p.take().unwrap()).unwrap();
            }
            if boundary == 2 {
                installation = Some(
                    r.engine
                        .take_staged_action()
                        .unwrap()
                        .unwrap()
                        .into_parts()
                        .1,
                );
            }
            if target == 3 {
                r.engine
                    .world
                    .module_state_mut::<PhysicalFixtureState>()
                    .unwrap()
                    .0 = 9;
            } else {
                let id = [r.source, r.actor, r.child][target];
                r.engine.world.set_name(id, "changed preimage").unwrap();
            }
            let before = r.objects();
            let refused = match boundary {
                0 => r.stage(p.take().unwrap()).is_err(),
                1 => r.engine.take_staged_action().is_err(),
                _ => r
                    .engine
                    .install_composed_action(installation.take().unwrap())
                    .is_err(),
            };
            assert!(refused, "boundary {boundary}, target {target}");
            assert_eq!(r.objects(), before);
            assert!(!r.committed());
        }
    }
}

#[test]
fn foreign_hook_veto_and_late_owner_bind_failure_have_no_partial_apply() {
    for boundary in 0..3 {
        let mut r = rig();
        let before = r.objects();
        if boundary == 0 {
            r.hook.veto.store(true, Ordering::SeqCst);
            assert!(r.prepare().is_err());
        } else {
            if boundary == 2 {
                r.request.lock().unwrap().refuse_bind = true;
            }
            let p = r.prepare().unwrap();
            if boundary == 1 {
                r.hook.veto.store(true, Ordering::SeqCst);
                assert!(r.stage(p).is_err());
            } else {
                r.stage(p).unwrap();
                let (_, installation) =
                    r.engine.take_staged_action().unwrap().unwrap().into_parts();
                assert!(r.engine.install_composed_action(installation).is_err());
            }
        }
        assert_eq!(r.objects(), before);
        assert!(!r.committed());
        assert_eq!(
            r.engine.world.module_state::<PhysicalFixtureState>(),
            Some(&PhysicalFixtureState(0))
        );
    }
}

#[test]
fn strict_receipt_hydration_and_crossed_key_audit() {
    let mut r = rig();
    let p = r.prepare().unwrap();
    let (writes, installation) = p.0.into_parts();
    r.engine.install_composed_action(installation).unwrap();
    let (key, envelope) = writes
        .into_iter()
        .find_map(|w| match w {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "sim" && key.as_str() != "physical_fixture" => {
                Some((key, value))
            }
            _ => None,
        })
        .unwrap();
    for corrupt in 0..3 {
        let mut fresh = rig();
        let mut value = envelope.clone();
        if corrupt == 1 {
            value["value"]["commit"] = json!(CommitToken::from_bytes([8; 16]));
        }
        if corrupt == 2 {
            value["value"]["unknown"] = json!(true);
        }
        let hydrated = fresh.engine.hydrate_action_state(vec![
            crate::action_state::ActionStateBootRecord::new("sim", key.as_str(), value),
        ]);
        if corrupt == 2 {
            assert!(hydrated.is_err());
            continue;
        }
        hydrated.unwrap();
        let audit = fresh.engine.all_modules_loaded(
            &mut crate::Outbox::new(),
            &mut crate::engine::Effects::new(),
        );
        if corrupt == 1 {
            assert!(audit.is_err());
        } else {
            audit.unwrap();
            assert!(fresh.committed());
        }
    }
}

fn receipt_boot_record(r: &mut Rig) -> crate::action_state::ActionStateBootRecord {
    let p = r.prepare().unwrap();
    p.0.authoritative_replay_writes()
        .into_iter()
        .find_map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "sim" && key.as_str() != "physical_fixture" => Some(
                crate::action_state::ActionStateBootRecord::new("sim", key.as_str(), value),
            ),
            _ => None,
        })
        .unwrap()
}

#[test]
fn retained_v1_and_current_v2_receipts_have_exact_paginated_readonly_metadata() {
    let mut r = rig_with_history(History::Registered);
    let current = receipt_boot_record(&mut r);
    let mut legacy = current.clone();
    let old_commit = CommitToken::from_bytes([6; 16]);
    let (old_action, old_participant) = &r.handles.service.retained[0];
    legacy.key = legacy
        .key
        .replace(&token().to_string(), &old_commit.to_string())
        .into();
    legacy.value["key"] = json!(old_commit.to_string());
    legacy.value["value"]["commit"] = json!(old_commit);
    legacy.value["value"]["action"] = json!(old_action);
    legacy.value["value"]["participant"] = json!(old_participant);
    legacy.value["value"]["at_utc"] = json!(99);
    let mut fresh = rig_with_history(History::Registered);
    fresh
        .engine
        .hydrate_action_state(vec![current, legacy.clone()])
        .unwrap();
    fresh
        .engine
        .all_modules_loaded(
            &mut crate::Outbox::new(),
            &mut crate::engine::Effects::new(),
        )
        .unwrap();
    let rows = fresh.handles.audited.lock().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].commit(), old_commit);
    assert_eq!(rows[0].action(), old_action);
    assert_eq!(rows[0].participant(), old_participant);
    assert_eq!(rows[0].at_utc(), 99);
    assert_eq!(rows[1].commit(), token());
    assert_eq!(rows[1].action(), &fresh.handles.action);
    assert_eq!(rows[1].action().version(), 2);
    assert_eq!(rows[1].participant(), &fresh.handles.service.participant);
    assert_eq!(rows[1].actor(), fresh.actor);
    assert_eq!(rows[1].root(), fresh.first);
    assert_eq!(rows[1].at_utc(), 100);
    assert_eq!(rows[1].structural_digest(), &rows[0].row.structural_digest);
    assert!(!rows[1].object_digests().is_empty());

    // Supporting the family decoder is insufficient: the owner must explicitly
    // retain this exact historical pair before it can inspect the row.
    let mut unsupported = rig();
    unsupported
        .engine
        .hydrate_action_state(vec![legacy])
        .unwrap();
    assert!(
        unsupported
            .engine
            .all_modules_loaded(
                &mut crate::Outbox::new(),
                &mut crate::engine::Effects::new()
            )
            .is_err()
    );
}

#[test]
fn receipt_versions_and_registered_participant_digests_fail_closed() {
    let mut r = rig_with_history(History::Registered);
    let valid = receipt_boot_record(&mut r);
    let (old_action, old_participant) = r.handles.service.retained[0].clone();
    let other_participant = crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("sim").unwrap(),
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant").unwrap(),
                DefinitionId::new(PARTICIPANT).unwrap(),
                2,
                json!({"fixture":"different"}),
            )
            .unwrap(),
        )
        .unwrap();
    let other_action = crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("sim").unwrap(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").unwrap(),
                DefinitionId::new(ACTION).unwrap(),
                2,
                json!({"protocol":"different"}),
            )
            .unwrap(),
        )
        .unwrap();
    for case in 0..4 {
        let mut record = valid.clone();
        match case {
            0 => record.value["value"]["participant"] = json!(old_participant),
            1 => record.value["value"]["action"] = json!(old_action),
            2 => record.value["value"]["participant"] = json!(other_participant),
            _ => record.value["value"]["action"] = json!(other_action),
        }
        let mut fresh = rig_with_history(History::Registered);
        let hydration = fresh.engine.hydrate_action_state(vec![record]);
        if case == 2 {
            hydration.unwrap();
            assert!(
                fresh
                    .engine
                    .all_modules_loaded(
                        &mut crate::Outbox::new(),
                        &mut crate::engine::Effects::new()
                    )
                    .is_err()
            );
        } else {
            assert!(
                hydration.is_err(),
                "crossed or inexact definition case {case}"
            );
        }
    }
    // Even an empty store must not accept configured history that was never
    // registered by the actual owner composition.
    let mut missing = rig_with_history(History::Missing);
    assert!(
        missing
            .engine
            .all_modules_loaded(
                &mut crate::Outbox::new(),
                &mut crate::engine::Effects::new()
            )
            .is_err()
    );
}

#[test]
fn actor_projection_runs_actual_move_hooks_until_final_take() {
    for boundary in 0..4 {
        let mut r = rig();
        r.request.lock().unwrap().moves = vec![PhysicalMove {
            object: r.child,
            source: Some(r.first),
            destination: r.destination,
        }];
        let before = r.objects();
        if boundary == 0 {
            r.hook.move_veto.store(true, Ordering::SeqCst);
            assert!(r.prepare().is_err());
        } else {
            let p = r.prepare().unwrap();
            if boundary == 1 {
                r.hook.move_veto.store(true, Ordering::SeqCst);
                assert!(r.stage(p).is_err());
            } else {
                r.stage(p).unwrap();
                if boundary == 2 {
                    r.hook.move_veto.store(true, Ordering::SeqCst);
                    assert!(r.engine.take_staged_action().is_err());
                } else {
                    let (_, installation) =
                        r.engine.take_staged_action().unwrap().unwrap().into_parts();
                    let calls = r.hook.move_calls.load(Ordering::SeqCst);
                    r.hook.move_veto.store(true, Ordering::SeqCst);
                    r.engine.install_composed_action(installation).unwrap();
                    assert_eq!(r.hook.move_calls.load(Ordering::SeqCst), calls);
                    assert_eq!(
                        r.engine.world.get(r.child).unwrap().location,
                        Some(r.destination)
                    );
                    assert!(r.committed());
                    continue;
                }
            }
        }
        assert_eq!(r.objects(), before);
        assert!(!r.committed());
        assert!(r.hook.move_calls.load(Ordering::SeqCst) > 0);
    }
}

#[test]
fn occupied_descendant_depth_and_bounded_frontier_refuse_whole_batch() {
    let mut r = rig();
    let mut destination = r.destination;
    for _ in 0..crate::MAX_CONTAINMENT_DEPTH - 2 {
        destination = r
            .engine
            .world
            .create(ObjKind::Thing, "nested", GOD, Some(destination))
            .unwrap();
    }
    r.request.lock().unwrap().moves = vec![PhysicalMove {
        object: r.first,
        source: Some(r.source),
        destination,
    }];
    let before = r.objects();
    assert!(r.prepare().is_err(), "occupied child exceeds final depth");
    assert_eq!(r.objects(), before);
    let mut r = rig();
    for _ in 0..MAX_PHYSICAL_MOVES {
        r.engine
            .world
            .create(ObjKind::Thing, "bounded child", GOD, Some(r.first))
            .unwrap();
    }
    let before = r.objects();
    assert!(
        r.prepare().is_err(),
        "frontier refuses before collecting a large subtree"
    );
    assert_eq!(r.objects(), before);
}
