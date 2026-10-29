#![cfg(test)]

use super::*;
use mush_core::action::{
    ActionInvocationDraft, ActionOfferId, CanonicalHash, CommitToken, IdempotencyKey,
    IdempotencyScope, RequestUid, ScopedIdempotencyKey,
};
use mush_core::locomotion::preview::LocomotionPreview;
use mush_core::locomotion::{LocomotionDecision, LocomotionRequest};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, BehaviorBinding, BehaviorId, DefinitionDescriptor, DefinitionId,
    DefinitionKind, DefinitionRef, Effects, Engine, GameModule, ModError, Outbox, OwnerId,
    Registrar, Registry,
};
use mush_sim::{DomainClock, DomainRecord, DomainStatus, WindowBox, WindowRec, WindowSurface};
use std::sync::{Arc, Mutex};

const OWNER: &str = "joint_window_probe";
#[derive(Default)]
struct Control {
    action: Option<DefinitionRef>,
    room: Option<ObjId>,
    preview: Option<LocomotionPreview>,
}
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
fn refused(_: impl std::fmt::Display) -> ActionError {
    ActionError::new(ActionErrorCode::ProviderRejected, false, "read-only probe")
        .expect("fixed error")
}
struct Probe(Arc<Mutex<Control>>);
impl GameModule for Probe {
    fn name(&self) -> &'static str {
        OWNER
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition_validator(
            DefinitionKind::new("action").map_err(bad)?,
            |_: &mush_core::DefinitionValidationContext<'_>| Ok(()),
        )?;
        let action = reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("action").map_err(bad)?,
                DefinitionId::new("preview").map_err(bad)?,
                1,
                json!({"fixture":"read_only_core_preview","publication":false}),
            )
            .map_err(bad)?,
        )?;
        self.0.lock().map_err(bad)?.action = Some(action.clone());
        reg.action_provider(action, ProbeProvider(self.0.clone()))
    }
}
struct ProbeProvider(Arc<Mutex<Control>>);
impl ActionProvider for ProbeProvider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        _: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let LocomotionDecision::Admitted(admission) = ctx
            .admit_locomotion(txn, LocomotionRequest::exit_name("north").map_err(refused)?)
            .map_err(refused)?
        else {
            return Err(refused("not admitted"));
        };
        let mut control = self.0.lock().map_err(refused)?;
        control.preview = Some(
            admission
                .preview_projection(
                    ctx.world().map_err(refused)?,
                    &BTreeSet::new(),
                    control.room.ok_or_else(|| refused("missing projection"))?,
                )
                .map_err(refused)?,
        );
        // Deliberately refuse before staging: this probe supplies only a real
        // Core read preview, never a substitute physical participant or record.
        Err(refused("capture only"))
    }
}

struct Rig {
    engine: Box<Engine>,
    control: Arc<Mutex<Control>>,
    actor: ObjId,
    from: ObjId,
    to: ObjId,
}
fn domain(id: u64, room: ObjId) -> DomainRecord {
    DomainRecord {
        id: DomainId(id),
        host: room,
        driver: mush_sim::passive_definition_ref(),
        status: DomainStatus::Active,
        clock: DomainClock::Passive,
        presentation_location: room,
        revision: 1,
        window: Some(WindowRec {
            region: "test.region".into(),
            surfaces: vec![WindowSurface::Ground],
            bbox: WindowBox {
                x0: 0,
                y0: 0,
                x1: 300,
                y1: 300,
            },
            cell_cm: 100,
            composited_from: vec![],
            features_seen: 0,
            opened_by: WindowOpener::Staff { by: GOD },
            revision: 1,
        }),
    }
}
impl Rig {
    fn new() -> Self {
        let f = fixture();
        let original = f
            .world
            .module_state::<AtlasState>()
            .expect("fixture")
            .clone();
        let control = Arc::new(Mutex::new(Control::default()));
        let mut registry = Registry::new();
        mush_core::register_core_commands(&mut registry).expect("commands");
        let mut engine = Box::new(
            Engine::with_modules(
                f.world,
                registry,
                vec![
                    Box::new(mush_sim::SimulationModule::new()),
                    Box::new(crate::AtlasModule::new()),
                    Box::new(Probe(control.clone())),
                ],
            )
            .expect("actual owners"),
        );
        engine
            .modules_loaded(Default::default(), &mut Outbox::new(), &mut Effects::new())
            .expect("actual boot");
        *engine
            .world
            .module_state_mut::<AtlasState>()
            .expect("registered Atlas") = original;
        engine.world.set_now(1234);
        let exit = engine
            .world
            .create(ObjKind::Exit, "north", GOD, Some(f.from))
            .expect("exit");
        engine.world.link_exit(exit, Some(f.to)).expect("link");
        engine
            .world
            .attach_behavior(
                f.actor,
                BehaviorId::new("sim.participant").expect("behavior"),
                BehaviorBinding {
                    schema_version: mush_core::BINDING_SCHEMA_VERSION,
                    attached_by: GOD,
                    attached_at: 1234,
                },
            )
            .expect("participant");
        let state = engine
            .world
            .module_state_mut::<SpatialState>()
            .expect("registered Sim");
        state.domains.insert(DomainId(7), domain(7, f.to));
        state.domains.insert(DomainId(8), domain(8, f.from));
        state
            .placements
            .insert(f.actor, placed(f.actor, 8, 50, 50, 0));
        state.derived = mush_sim::graph::rebuild(state);
        let _ = engine.world.take_dirty();
        Self {
            engine,
            control,
            actor: f.actor,
            from: f.from,
            to: f.to,
        }
    }
    fn plan(&self) -> WindowArrivalPlan {
        plan(
            &self.engine.world,
            self.actor,
            Some(self.from),
            self.to,
            self.engine.world.now(),
        )
        .expect("real plan")
    }
    fn preview(&mut self, room: ObjId) -> LocomotionPreview {
        let action = {
            let mut control = self.control.lock().expect("control");
            control.room = Some(room);
            control.action.clone().expect("action")
        };
        let request = RequestUid::from_bytes([201; 16]);
        let principal = mush_core::PrincipalId::Object { id: self.actor };
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([202; 32])),
            subject: mush_core::PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new(OWNER).expect("owner"),
                    namespace: CanonicalId::new("preview").expect("namespace"),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .expect("invocation");
        assert!(
            self.engine
                .prepare_registered_composed_authoritative_action(
                    &action,
                    &invocation,
                    CommitToken::from_bytes([203; 16]),
                )
                .is_err(),
            "probe never stages a movement"
        );
        self.control
            .lock()
            .expect("control")
            .preview
            .take()
            .expect("genuine preview")
    }
}

#[test]
fn actual_selection_core_preview_and_ordered_native_capture_share_one_proposal() {
    let mut rig = Rig::new();
    let atlas = rig.plan();
    let (room, target) = atlas.projection_target().expect("selection data");
    assert_eq!(room, rig.to);
    assert_eq!(target.expect("metric target").domain(), 7);
    let preview = rig.preview(room);
    let joint = atlas
        .capture_sim(&rig.engine.world, preview)
        .expect("joint capture");
    assert_eq!(joint.sim().stage_count(), 3);
    assert_eq!(
        joint.projection().atlas_capture_digest(),
        joint.atlas().digest().unwrap()
    );
    assert_eq!(
        joint.projection().sim_window_capture_digest(),
        joint.sim().digest()
    );
    joint
        .atlas()
        .check_projection(joint.projection())
        .expect("same selector evidence");
    mush_sim::window_movement::validate_evidence(
        &serde_json::from_slice(&joint.sim().encode()).unwrap(),
        joint.sim().digest(),
    )
    .expect("passive historical evidence");
    rig.engine.world.set_now(1240);
    joint
        .recheck(&rig.engine.world)
        .expect("forward time retains original UTC");
    assert_eq!(joint.sim().at_utc(), 1234);
    assert_eq!(
        rig.engine.world.get(rig.actor).unwrap().location,
        Some(rig.from)
    );
    assert_eq!(
        rig.engine
            .world
            .module_state::<SpatialState>()
            .unwrap()
            .placements[&rig.actor]
            .revision,
        3
    );
}

#[test]
fn joint_capture_refuses_wrong_projection_changed_selector_and_native_preimage() {
    for fault in 0..6 {
        let mut rig = Rig::new();
        let atlas = rig.plan();
        let preview = rig.preview(if fault == 0 { rig.from } else { rig.to });
        match fault {
            1 => {
                rig.engine
                    .world
                    .module_state_mut::<AtlasState>()
                    .unwrap()
                    .records
                    .roomgeo
                    .get_mut(&rig.to)
                    .unwrap()
                    .footprint = Footprint::Anchor {
                    point: Point2(0.0, 0.0),
                    radius_m: 1.0,
                }
            }
            2 => rig.engine.world.set_now(1235),
            3 => {
                rig.engine
                    .world
                    .module_state_mut::<SpatialState>()
                    .unwrap()
                    .domains
                    .get_mut(&DomainId(7))
                    .unwrap()
                    .window = None
            }
            4 => {
                rig.engine
                    .world
                    .get_mut(rig.actor)
                    .expect("Core drift")
                    .location = Some(rig.to);
            }
            5 => {
                let state = rig.engine.world.module_state_mut::<SpatialState>().unwrap();
                state.placements.get_mut(&rig.actor).unwrap().parent = SpatialParent::Interior {
                    space: rig.from,
                    slot: None,
                };
                state.derived = mush_sim::graph::rebuild(state);
            }
            _ => {}
        }
        assert!(
            atlas.capture_sim(&rig.engine.world, preview).is_err(),
            "fault {fault}"
        );
        if fault != 4 {
            assert_eq!(
                rig.engine.world.get(rig.actor).unwrap().location,
                Some(rig.from)
            );
        }
    }
    let mut rig = Rig::new();
    let atlas = rig.plan();
    let preview = rig.preview(rig.to);
    let joint = atlas
        .capture_sim(&rig.engine.world, preview)
        .expect("capture");
    rig.engine
        .world
        .module_state_mut::<SpatialState>()
        .unwrap()
        .placements
        .get_mut(&rig.actor)
        .unwrap()
        .revision += 1;
    assert!(joint.recheck(&rig.engine.world).is_err());
}

#[test]
fn joint_release_and_empty_native_work_keep_the_admitted_destination() {
    for placed in [true, false] {
        let mut rig = Rig::new();
        rig.engine
            .world
            .module_state_mut::<AtlasState>()
            .unwrap()
            .windows
            .open
            .clear();
        if !placed {
            let state = rig.engine.world.module_state_mut::<SpatialState>().unwrap();
            state.placements.remove(&rig.actor);
            state.derived = mush_sim::graph::rebuild(state);
        }
        let atlas = rig.plan();
        assert_eq!(atlas.projection_target().unwrap(), (rig.to, None));
        let preview = rig.preview(rig.to);
        let joint = atlas
            .capture_sim(&rig.engine.world, preview)
            .expect("no placement selected");
        assert_eq!(joint.projection().target(), None);
        assert_eq!(joint.projection().projected_room(), rig.to);
        assert_eq!(joint.sim().stage_count(), if placed { 2 } else { 1 });
        let (atlas, sim, projection) = joint.into_parts();
        atlas.check_projection(&projection).unwrap();
        assert_eq!(sim.digest(), projection.sim_window_capture_digest());
    }
}
