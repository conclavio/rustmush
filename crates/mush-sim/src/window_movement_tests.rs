#![cfg(test)]

use super::*;
use crate::data::{
    DomainClock, DomainRecord, DomainStatus, PlacementRecord, WindowBox, WindowOpener, WindowRec,
    WindowSurface,
};
use mush_core::action::{
    ActionInvocationDraft, ActionOfferId, CanonicalHash, CommitToken, IdempotencyKey,
    IdempotencyScope, RequestUid, ScopedIdempotencyKey,
};
use mush_core::locomotion::{LocomotionDecision, LocomotionRequest};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, BehaviorBinding, BehaviorId, CanonicalId, DefinitionDescriptor,
    DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder, Engine, GOD,
    GameModule, ModError, ObjKind, Outbox, OwnerId, Registrar, RulesValue, StateFamilyDescriptor,
    StateFamilyHandle, StateFamilyId,
};
use std::sync::{Arc, Mutex};

const OWNER: &str = "window_probe";
#[derive(Default)]
struct Control {
    action: Option<DefinitionRef>,
    projection: Option<ObjId>,
    dependencies: BTreeSet<ObjId>,
    preview: Option<LocomotionPreview>,
}
fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
fn rejected(_: impl std::fmt::Display) -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "window probe refused",
    )
    .expect("fixed error")
}
fn definition(reg: &mut Registrar<'_>, kind: &str, id: &str) -> Result<DefinitionRef, ModError> {
    let kind = DefinitionKind::new(kind).map_err(bad)?;
    let descriptor = DefinitionDescriptor::new(
        kind.clone(),
        DefinitionId::new(id).map_err(bad)?,
        1,
        json!({"fixture":"window_probe"}),
    )
    .map_err(bad)?;
    let expected = DefinitionRegistryBuilder::new()
        .register(OwnerId::new(OWNER).map_err(bad)?, descriptor.clone())
        .map_err(bad)?;
    reg.definition_validator(
        kind,
        move |ctx: &mush_core::DefinitionValidationContext<'_>| {
            if ctx.definition().reference() != &expected {
                return Err("unknown probe definition".into());
            }
            Ok(())
        },
    )?;
    reg.definition(descriptor)
}
struct Probe(Arc<Mutex<Control>>);
impl GameModule for Probe {
    fn name(&self) -> &'static str {
        OWNER
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let action = definition(reg, "action", "capture")?;
        let outcome = definition(reg, "outcome", "captured")?;
        let marker = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("records").map_err(bad)?,
                CanonicalId::new("u32.decimal.v1").map_err(bad)?,
                CanonicalId::new("u32.value.v1").map_err(bad)?,
                1,
                CanonicalId::new("u32.decode.v1").map_err(bad)?,
                CanonicalId::new("u32.validate.v1").map_err(bad)?,
                CanonicalId::new("u32.serialize.v1").map_err(bad)?,
            )
            .map_err(bad)?,
            |key: &u32| Ok(key.to_string()),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |_value: &u32| Ok(()),
            |value: &u32| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;
        reg.action_provider(
            action.clone(),
            Provider {
                control: self.0.clone(),
                outcome,
                marker,
            },
        )?;
        self.0.lock().map_err(bad)?.action = Some(action);
        Ok(())
    }
}
struct Provider {
    control: Arc<Mutex<Control>>,
    outcome: DefinitionRef,
    marker: StateFamilyHandle<u32, u32>,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        _: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let mut control = self.control.lock().map_err(rejected)?;
        let LocomotionDecision::Admitted(admission) = ctx
            .admit_locomotion(
                txn,
                LocomotionRequest::exit_name("north").map_err(rejected)?,
            )
            .map_err(rejected)?
        else {
            return Err(rejected("already home"));
        };
        control.preview = Some(match control.projection {
            Some(room) => admission
                .preview_projection(ctx.world().map_err(rejected)?, &control.dependencies, room)
                .map_err(rejected)?,
            None => admission
                .preview(ctx.world().map_err(rejected)?, &control.dependencies)
                .map_err(rejected)?,
        });
        // A genuine independent native fixture marker satisfies the staging
        // contract. It is aborted, and is never claimed as movement evidence.
        txn.insert(&self.marker, &0, 1).map_err(rejected)?;
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({})).map_err(rejected)?,
        )
        .with_definitions(vec![self.outcome.clone()]))
    }
}

struct Rig {
    engine: Box<Engine>,
    control: Arc<Mutex<Control>>,
    actor: ObjId,
    source: ObjId,
    admitted: ObjId,
    projected: ObjId,
}
fn domain(id: u64, host: ObjId, room: ObjId) -> DomainRecord {
    DomainRecord {
        id: DomainId(id),
        host,
        driver: crate::passive_definition_ref(),
        status: DomainStatus::Active,
        clock: DomainClock::Passive,
        presentation_location: room,
        revision: 1,
        window: Some(WindowRec {
            region: "fixture".into(),
            surfaces: vec![WindowSurface::Ground],
            bbox: WindowBox {
                x0: 0,
                y0: 0,
                x1: 100,
                y1: 100,
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
        let control = Arc::new(Mutex::new(Control::default()));
        let mut test = mush_testkit::TestWorld::with_modules(vec![
            Box::new(crate::SimulationModule::new()),
            Box::new(Probe(control.clone())),
        ]);
        test.set_now(100);
        let source = test
            .world_mut()
            .create(ObjKind::Room, "source", GOD, None)
            .expect("source");
        let admitted = test
            .world_mut()
            .create(ObjKind::Room, "admitted", GOD, None)
            .expect("admitted");
        let projected = test
            .world_mut()
            .create(ObjKind::Room, "projected", GOD, None)
            .expect("projected");
        let actor = test
            .world_mut()
            .create(ObjKind::Player, "walker", GOD, Some(source))
            .expect("actor");
        let exit = test
            .world_mut()
            .create(ObjKind::Exit, "north", GOD, Some(source))
            .expect("exit");
        test.world_mut()
            .link_exit(exit, Some(admitted))
            .expect("link");
        test.world_mut()
            .attach_behavior(
                actor,
                BehaviorId::new("sim.participant").expect("behavior"),
                BehaviorBinding {
                    schema_version: mush_core::BINDING_SCHEMA_VERSION,
                    attached_by: GOD,
                    attached_at: 100,
                },
            )
            .expect("attach");
        let state = test
            .world_mut()
            .module_state_mut::<SpatialState>()
            .expect("actual owner");
        state
            .domains
            .insert(DomainId(1), domain(1, admitted, projected));
        state.domains.insert(DomainId(2), domain(2, source, source));
        state.placements.insert(
            actor,
            PlacementRecord {
                object: actor,
                parent: SpatialParent::Direct {
                    domain: DomainId(2),
                    pose: crate::Pose::at(
                        10,
                        20,
                        0,
                        crate::passive_rules_value(json!({})).expect("posture"),
                    ),
                },
                revision: 7,
                unplaced: None,
            },
        );
        state.derived = crate::graph::rebuild(state);
        crate::window::register_room_resolver(
            test.world_mut(),
            Arc::new(move |_, id, _| {
                if id == DomainId(1) {
                    Some(projected)
                } else {
                    Some(source)
                }
            }),
        )
        .expect("resolver");
        let _ = test.take_module_ops();
        let _ = test.world_mut().take_dirty();
        Self {
            engine: Box::new(test.into_engine()),
            control,
            actor,
            source,
            admitted,
            projected,
        }
    }
    fn state(&mut self) -> &mut SpatialState {
        self.engine
            .world
            .module_state_mut::<SpatialState>()
            .expect("actual Sim owner")
    }
    fn preview(&mut self, projected: Option<ObjId>) -> LocomotionPreview {
        let action = {
            let mut control = self.control.lock().expect("control");
            control.projection = projected;
            control.dependencies =
                BTreeSet::from([self.actor, self.source, self.admitted, self.projected]);
            control.action.clone().expect("action")
        };
        let request = RequestUid::from_bytes([101; 16]);
        let principal = mush_core::PrincipalId::Object { id: self.actor };
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([102; 32])),
            subject: mush_core::PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new(OWNER).expect("owner"),
                    namespace: CanonicalId::new("probe").expect("namespace"),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .expect("invocation");
        let (prepared, _) = self
            .engine
            .prepare_registered_composed_authoritative_action(
                &action,
                &invocation,
                CommitToken::from_bytes([103; 16]),
            )
            .expect("real admission");
        let (_, installation) = prepared.into_parts();
        self.engine
            .abort_prepared_action(installation)
            .expect("abort independent marker");
        self.control
            .lock()
            .expect("control")
            .preview
            .take()
            .expect("preview")
    }
    fn capture(
        &mut self,
        target: Option<WindowTarget>,
        projected: Option<ObjId>,
    ) -> WindowMovementCapture {
        let preview = self.preview(projected);
        capture(&self.engine.world, preview, target).expect("capture")
    }
}
fn value(capture: &WindowMovementCapture) -> Value {
    serde_json::from_slice(&capture.encode()).expect("encoded")
}

fn ordered(
    rig: &mut Rig,
    target: Option<WindowTarget>,
    projection: Option<ObjId>,
) -> OrderedWindowMovementCapture {
    let core = rig.preview(projection);
    capture_ordered(&rig.engine.world, core, target).expect("ordered candidate")
}

#[test]
fn ordered_capture_keeps_native_v1_bytes_and_three_linked_original_stages() {
    let mut rig = Rig::new();
    let target = Some(WindowTarget::at(DomainId(1), 30, 40, 50));
    let projected = Some(rig.projected);
    let original = rig.capture(target, projected);
    let captured = ordered(&mut rig, target, projected);
    let data: Value = serde_json::from_slice(&captured.encode()).expect("bytes");
    assert_eq!(data["format"], json!(2));
    assert_eq!(data["native"], value(&original));
    assert_ne!(captured.digest(), original.digest());
    assert_eq!(captured.stage_count(), 3);
    let stages = data["stages"].as_array().expect("stages");
    assert_eq!(
        stages
            .iter()
            .map(|s| s["kind"].as_str().expect("kind"))
            .collect::<Vec<_>>(),
        vec!["core_admission", "release", "place"]
    );
    for pair in stages.windows(2) {
        assert_eq!(
            pair[0]["after_ancestry_digest"],
            pair[1]["before_ancestry_digest"]
        );
    }
    for stage in stages {
        assert_eq!(stage["crossings"], json!([]));
        assert_eq!(stage["prepared_at"], json!(captured.prepared_at()));
    }
    assert_eq!(captured.actor(), rig.actor);
    assert_eq!(captured.at_utc(), 100);
    assert!(captured.read_dependencies().contains(&rig.projected));
    captured
        .recheck(&rig.engine.world)
        .expect("same actual facts");
    rig.engine.world.set_now(101);
    captured
        .recheck(&rig.engine.world)
        .expect("later time retains original facts");
    assert_eq!(captured.at_utc(), 100);
    assert_eq!(
        rig.engine.world.get(rig.actor).expect("actor").location,
        Some(rig.source)
    );
    assert_eq!(
        rig.engine
            .world
            .module_state::<SpatialState>()
            .expect("Sim owner")
            .placements
            .get(&rig.actor)
            .expect("record")
            .revision,
        7
    );
}

#[test]
fn ordered_loop_retains_both_steps_and_release_or_no_native_work_stays_distinct() {
    let mut rig = Rig::new();
    let source = rig.source;
    let captured = ordered(
        &mut rig,
        Some(WindowTarget::at(DomainId(2), 30, 40, 0)),
        Some(source),
    );
    let value: Value = serde_json::from_slice(&captured.encode()).expect("bytes");
    assert_eq!(captured.stage_count(), 2);
    assert_eq!(
        value["stages"][0]["before_ancestry_digest"],
        value["stages"][1]["after_ancestry_digest"]
    );
    assert_ne!(
        value["stages"][0]["before_ancestry_digest"],
        value["stages"][0]["after_ancestry_digest"]
    );
    assert_eq!(value["stages"][1]["kind"], json!("place"));
    let released = ordered(&mut rig, None, None);
    assert_eq!(released.stage_count(), 2);
    let actor = rig.actor;
    rig.state().placements.remove(&actor);
    let rebuilt = crate::graph::rebuild(rig.state());
    rig.state().derived = rebuilt;
    let empty = ordered(&mut rig, None, None);
    assert_eq!(empty.stage_count(), 1);
}

#[test]
fn ordered_capture_rechecks_native_core_and_original_clock_facts() {
    for mode in 0..3 {
        let mut rig = Rig::new();
        let projected = Some(rig.projected);
        let captured = ordered(
            &mut rig,
            Some(WindowTarget::at(DomainId(1), 30, 40, 0)),
            projected,
        );
        match mode {
            0 => {
                rig.state()
                    .domains
                    .get_mut(&DomainId(1))
                    .expect("domain")
                    .status = DomainStatus::Retired;
            }
            1 => {
                rig.engine.world.get_mut(rig.projected).expect("room").name = "changed".into();
            }
            _ => {
                let stamp = rig.engine.world.world_ic_now();
                rig.engine.world.set_now(99);
                assert_eq!(rig.engine.world.world_ic_now().clock, stamp.clock);
            }
        }
        assert!(captured.recheck(&rig.engine.world).is_err());
    }
}

#[test]
fn ordered_occupancy_refuses_coordinate_identity_and_membership_drift_without_revision_bumps() {
    for change in 0..7 {
        let mut rig = Rig::new();
        let occupant = rig
            .engine
            .world
            .create(ObjKind::Thing, "other occupant", GOD, Some(rig.projected))
            .expect("occupant");
        let spare = rig
            .engine
            .world
            .create(ObjKind::Thing, "spare", GOD, Some(rig.projected))
            .expect("spare");
        let actor = rig.actor;
        let mut record = rig
            .state()
            .placements
            .get(&actor)
            .expect("placement")
            .clone();
        record.object = occupant;
        if let SpatialParent::Direct { domain, .. } = &mut record.parent {
            *domain = DomainId(1);
        }
        rig.state().placements.insert(occupant, record.clone());
        rig.state().derived = crate::graph::rebuild(rig.state());
        let projected = rig.projected;
        let capture = ordered(
            &mut rig,
            Some(WindowTarget::at(DomainId(1), 30, 40, 0)),
            Some(projected),
        );
        capture
            .recheck_occupancy(rig.state())
            .expect("actual native slot");
        match change {
            0 | 1 => {
                if let SpatialParent::Direct { pose, .. } = &mut rig
                    .state()
                    .placements
                    .get_mut(&occupant)
                    .expect("occupant")
                    .parent
                {
                    if change == 0 {
                        pose.x_cm += 1;
                    } else {
                        pose.y_cm += 1;
                    }
                }
            }
            2 => {
                rig.state()
                    .placements
                    .get_mut(&occupant)
                    .expect("occupant")
                    .revision += 1;
            }
            3 => {
                rig.state().placements.remove(&occupant);
            }
            4 => {
                if let SpatialParent::Direct { domain, .. } = &mut rig
                    .state()
                    .placements
                    .get_mut(&occupant)
                    .expect("occupant")
                    .parent
                {
                    *domain = DomainId(2);
                }
            }
            5 => {
                record.object = spare;
                rig.state().placements.insert(spare, record);
            }
            _ => {
                rig.state()
                    .placements
                    .get_mut(&occupant)
                    .expect("occupant")
                    .object = spare;
            }
        }
        rig.state().derived = crate::graph::rebuild(rig.state());
        assert!(
            capture.recheck_occupancy(rig.state()).is_err(),
            "change {change}"
        );
        assert!(
            capture.recheck(&rig.engine.world).is_err(),
            "change {change}"
        );
        assert_eq!(
            rig.engine.world.get(rig.actor).expect("actor").location,
            Some(rig.source)
        );
    }
}

#[test]
fn ordered_occupancy_ignores_unconsulted_pose_fields_and_has_no_fence_without_a_target() {
    let mut rig = Rig::new();
    let occupant = rig
        .engine
        .world
        .create(ObjKind::Thing, "occupant", GOD, Some(rig.projected))
        .expect("occupant");
    let actor = rig.actor;
    let mut record = rig
        .state()
        .placements
        .get(&actor)
        .expect("actor placement")
        .clone();
    record.object = occupant;
    if let SpatialParent::Direct { domain, .. } = &mut record.parent {
        *domain = DomainId(1);
    }
    rig.state().placements.insert(occupant, record);
    rig.state().derived = crate::graph::rebuild(rig.state());
    let projected = rig.projected;
    let capture = ordered(
        &mut rig,
        Some(WindowTarget::at(DomainId(1), 30, 40, 0)),
        Some(projected),
    );
    if let SpatialParent::Direct { pose, .. } = &mut rig
        .state()
        .placements
        .get_mut(&occupant)
        .expect("occupant")
        .parent
    {
        pose.z_cm += 1;
        pose.heading_mdeg += 1;
        pose.posture = crate::passive_rules_value(json!({"changed":true})).expect("posture");
    }
    capture
        .recheck_occupancy(rig.state())
        .expect("same cell observations");
    capture
        .recheck(&rig.engine.world)
        .expect("unconsulted pose fields");
    let empty = ordered(&mut rig, None, None);
    let encoded: Value = serde_json::from_slice(&empty.encode()).expect("bytes");
    assert_eq!(encoded["occupancy"], Value::Null);
    rig.state().placements.remove(&occupant);
    empty
        .recheck_occupancy(rig.state())
        .expect("no occupancy was consulted");
}

#[test]
fn ordered_capture_counts_both_sides_of_a_transient_carrier_visit_against_one_budget() {
    for children in [127, 128] {
        let mut rig = Rig::new();
        let carrier = rig
            .engine
            .world
            .create(ObjKind::Thing, "carrier", GOD, Some(mush_core::LIMBO))
            .expect("carrier");
        for index in 0..children {
            rig.engine
                .world
                .create(
                    ObjKind::Npc,
                    &format!("carried {index}"),
                    GOD,
                    Some(rig.actor),
                )
                .expect("person");
        }
        let actor = rig.actor;
        let admitted = rig.admitted;
        let projected = rig.projected;
        let state = rig.state();
        state.placements.remove(&actor);
        state.domains.get_mut(&DomainId(1)).expect("window").host = projected;
        state.spaces.insert(
            admitted,
            crate::InteriorSpaceRecord {
                object: admitted,
                anchor: crate::SpaceAnchor::MovingObject { host: carrier },
                compartments: Default::default(),
                rooms: BTreeSet::from([admitted]),
                egresses: Default::default(),
            },
        );
        state.derived = crate::graph::rebuild(state);
        let core = rig.preview(Some(projected));
        let capture = capture_ordered(
            &rig.engine.world,
            core,
            Some(WindowTarget::at(DomainId(1), 30, 40, 0)),
        );
        if children == 127 {
            let capture = capture.expect("256 total crossings");
            let value: Value = serde_json::from_slice(&capture.encode()).expect("bytes");
            assert_eq!(
                value["stages"][0]["crossings"]
                    .as_array()
                    .expect("entries")
                    .len(),
                128
            );
            assert_eq!(
                value["stages"][1]["crossings"]
                    .as_array()
                    .expect("exits")
                    .len(),
                128
            );
            assert_eq!(
                value["stages"][0]["crossings"][0]["direction"],
                json!("enter")
            );
            assert_eq!(
                value["stages"][1]["crossings"][0]["direction"],
                json!("exit")
            );
        } else {
            assert!(matches!(capture, Err(ParticipantError::ImageLimit)));
        }
        assert_eq!(
            rig.engine.world.get(actor).expect("actor").location,
            Some(rig.source)
        );
        assert!(!rig.state().placements.contains_key(&actor));
    }
}

#[test]
fn real_release_and_place_phases_match_legacy_application_without_publication() {
    let mut rig = Rig::new();
    let target = WindowTarget::at(DomainId(1), 30, 40, 50);
    let projected = rig.projected;
    let captured = rig.capture(Some(target), Some(projected));
    let data = value(&captured);
    assert_eq!(
        data.pointer("/phases/0/placements/0/revision"),
        Some(&json!(8))
    );
    assert_eq!(
        data.pointer("/phases/1/placements/0/revision"),
        Some(&json!(9))
    );
    assert_eq!(
        data.pointer("/phases/0/placements/0/unplaced/at"),
        Some(&json!(100))
    );
    assert_eq!(
        rig.engine.world.get(rig.actor).expect("actor").location,
        Some(rig.source)
    );
    assert_eq!(
        rig.state()
            .placements
            .get(&captured.actor())
            .expect("placement")
            .revision,
        7
    );
    assert_eq!(rig.state().recovery.next_seq(crate::data::RECOVERY_LOG), 0);
    assert!(!rig.engine.has_legacy_persistence_work());
    captured.recheck(&rig.engine.world).expect("unchanged");
    let mut legacy = rig.state().clone();
    rig.engine
        .world
        .move_to(rig.actor, rig.admitted)
        .expect("ordinary move");
    let release = crate::transition::plan_at(
        &legacy,
        &rig.engine.world,
        &Transition::Unplace {
            obj: rig.actor,
            reason: "left the map".into(),
            by: Some(rig.actor),
            keep: true,
        },
        100,
    )
    .expect("release");
    let mut out = Outbox::default();
    crate::transition::apply(&mut legacy, &mut rig.engine.world, &mut out, release)
        .expect("apply release");
    let place = crate::transition::plan_at(
        &legacy,
        &rig.engine.world,
        &Transition::Place {
            obj: rig.actor,
            parent: SpatialParent::Direct {
                domain: DomainId(1),
                pose: crate::Pose::at(
                    30,
                    40,
                    50,
                    crate::passive_rules_value(json!({})).expect("posture"),
                ),
            },
        },
        100,
    )
    .expect("place");
    crate::transition::apply(&mut legacy, &mut rig.engine.world, &mut out, place)
        .expect("apply place");
    assert_eq!(
        data.get("after"),
        Some(&serde_json::to_value(legacy.placements.get(&rig.actor)).expect("actual after"))
    );
    assert_eq!(
        rig.engine.world.get(rig.actor).expect("actor").location,
        Some(rig.projected)
    );
    assert!(captured.recheck(&rig.engine.world).is_err());
}

#[test]
fn same_domain_repose_and_release_only_preserve_the_actual_revision_policy() {
    let mut rig = Rig::new();
    let admitted = rig.admitted;
    let captured = rig.capture(
        Some(WindowTarget::at(DomainId(2), 10, 20, 0)),
        Some(admitted),
    );
    let data = value(&captured);
    assert_eq!(
        data.get("phases").and_then(Value::as_array).map(Vec::len),
        Some(1)
    );
    assert_eq!(data.pointer("/after/revision"), Some(&json!(8)));
    let captured = rig.capture(None, None);
    let data = value(&captured);
    assert_eq!(
        data.get("phases").and_then(Value::as_array).map(Vec::len),
        Some(1)
    );
    assert_eq!(data.pointer("/after/parent/kind"), Some(&json!("Unplaced")));
    assert_eq!(data.pointer("/after/unplaced/by"), Some(&json!(rig.actor)));
}

#[test]
fn current_no_placement_and_missing_record_stay_distinct_checked_observations() {
    let mut rig = Rig::new();
    let actor = rig.actor;
    rig.state().placements.remove(&actor);
    rig.state().derived = crate::graph::rebuild(rig.state());
    let absent = rig.capture(None, None);
    let mut record = PlacementRecord {
        object: actor,
        parent: SpatialParent::Unplaced,
        revision: 5,
        unplaced: None,
    };
    rig.state().placements.insert(actor, record.clone());
    rig.state().derived = crate::graph::rebuild(rig.state());
    let unplaced = rig.capture(None, None);
    assert_ne!(absent.digest(), unplaced.digest());
    assert_eq!(value(&absent).get("before"), Some(&Value::Null));
    assert_eq!(value(&unplaced).get("phases"), Some(&json!([])));
    record.revision = 6;
    rig.state().placements.insert(actor, record);
    assert!(unplaced.recheck(&rig.engine.world).is_err());
}

#[test]
fn selected_native_facts_and_revisions_are_fenced_even_without_counter_changes() {
    let mut rig = Rig::new();
    let projected = rig.projected;
    let captured = rig.capture(
        Some(WindowTarget::at(DomainId(1), 30, 40, 0)),
        Some(projected),
    );
    rig.state()
        .domains
        .get_mut(&DomainId(1))
        .expect("domain")
        .window
        .as_mut()
        .expect("window")
        .features_seen = 99;
    assert!(captured.recheck(&rig.engine.world).is_err());
    let captured = rig.capture(
        Some(WindowTarget::at(DomainId(1), 30, 40, 0)),
        Some(projected),
    );
    let actor = rig.actor;
    rig.state()
        .placements
        .get_mut(&actor)
        .expect("placement")
        .revision = 9;
    assert!(captured.recheck(&rig.engine.world).is_err());
}

#[test]
fn missing_participant_closed_domain_and_exhausted_revision_refuse_without_a_partial_release() {
    let mut rig = Rig::new();
    let projected = rig.projected;
    let target = Some(WindowTarget::at(DomainId(1), 30, 40, 0));
    rig.state()
        .domains
        .get_mut(&DomainId(1))
        .expect("domain")
        .status = DomainStatus::Retired;
    rig.state().derived = crate::graph::rebuild(rig.state());
    let preview = rig.preview(Some(projected));
    assert!(capture(&rig.engine.world, preview, target).is_err());
    rig.state()
        .domains
        .get_mut(&DomainId(1))
        .expect("domain")
        .status = DomainStatus::Active;
    rig.state().derived = crate::graph::rebuild(rig.state());
    let actor = rig.actor;
    rig.state()
        .placements
        .get_mut(&actor)
        .expect("placement")
        .revision = u64::MAX - 1;
    let preview = rig.preview(Some(projected));
    assert!(capture(&rig.engine.world, preview, target).is_err());
    assert_eq!(
        rig.state()
            .placements
            .get(&actor)
            .expect("unchanged source")
            .revision,
        u64::MAX - 1
    );
    rig.state()
        .placements
        .get_mut(&actor)
        .expect("placement")
        .revision = 7;
    rig.engine
        .world
        .detach_behavior(
            actor,
            &BehaviorId::new("sim.participant").expect("behavior"),
        )
        .expect("detach");
    let preview = rig.preview(Some(projected));
    assert!(capture(&rig.engine.world, preview, target).is_err());
    assert_eq!(
        rig.engine.world.get(actor).expect("actor").location,
        Some(rig.source)
    );
}

#[test]
fn malformed_indexes_and_deep_unrelated_rules_refuse_before_resident_encoding() {
    let mut rig = Rig::new();
    let preview = rig.preview(None);
    rig.state().derived = Default::default();
    assert!(capture(&rig.engine.world, preview, None).is_err());
    rig.state().derived = crate::graph::rebuild(rig.state());
    let mut deep = Value::Null;
    for _ in 0..70 {
        deep = json!([deep]);
    }
    let actor = rig.actor;
    let mut record = rig.state().placements.get(&actor).expect("source").clone();
    record.object = ObjId(9999);
    let SpatialParent::Direct { pose, .. } = &mut record.parent else {
        panic!("direct")
    };
    pose.posture = crate::passive_rules_value(deep).expect("bounded rules value");
    rig.state().placements.insert(record.object, record);
    rig.state().derived = crate::graph::rebuild(rig.state());
    let preview = rig.preview(None);
    assert!(capture(&rig.engine.world, preview, None).is_err());
}

#[test]
fn structural_parents_dependents_control_hosts_and_domain_aliases_require_later_owners() {
    for unsupported in 0..7 {
        let mut rig = Rig::new();
        let actor = rig.actor;
        let projected = rig.projected;
        let child = rig
            .engine
            .world
            .create(ObjKind::Thing, "child", GOD, Some(actor))
            .expect("child");
        let space = rig
            .engine
            .world
            .create(ObjKind::Room, "anchored space", GOD, None)
            .expect("space");
        let preview = rig.preview(Some(projected));
        match unsupported {
            0 => {
                rig.state()
                    .placements
                    .get_mut(&actor)
                    .expect("actor")
                    .parent = SpatialParent::Interior {
                    space: rig.source,
                    slot: None,
                }
            }
            1 => {
                rig.state()
                    .placements
                    .get_mut(&actor)
                    .expect("actor")
                    .parent = SpatialParent::Attached {
                    host: rig.source,
                    mount: crate::MountId::new("mount").expect("mount"),
                    local_pose: None,
                }
            }
            2 => {
                rig.state()
                    .domains
                    .insert(DomainId(3), domain(3, actor, projected));
                rig.state().placements.insert(
                    child,
                    PlacementRecord {
                        object: child,
                        parent: SpatialParent::Direct {
                            domain: DomainId(3),
                            pose: crate::Pose::passive(
                                crate::passive_rules_value(json!({})).expect("passive"),
                            ),
                        },
                        revision: 1,
                        unplaced: None,
                    },
                );
            }
            3 => {
                rig.state().stations.insert(
                    actor,
                    crate::HostStations {
                        host: actor,
                        stations: Default::default(),
                    },
                );
            }
            4 => {
                rig.state()
                    .domains
                    .get_mut(&DomainId(1))
                    .expect("domain")
                    .id = DomainId(99);
            }
            5 => {
                rig.state().spaces.insert(
                    space,
                    crate::InteriorSpaceRecord {
                        object: space,
                        anchor: crate::SpaceAnchor::MovingObject { host: actor },
                        compartments: Default::default(),
                        rooms: BTreeSet::from([space]),
                        egresses: Default::default(),
                    },
                );
            }
            _ => {
                rig.state()
                    .domains
                    .insert(DomainId(3), domain(3, actor, projected));
            }
        }
        rig.state().derived = crate::graph::rebuild(rig.state());
        assert!(
            capture(
                &rig.engine.world,
                preview,
                Some(WindowTarget::at(DomainId(1), 30, 40, 0))
            )
            .is_err()
        );
        assert_eq!(
            rig.engine.world.get(actor).expect("actor").location,
            Some(rig.source)
        );
    }
}

#[test]
fn a_missing_window_or_oversized_selected_record_and_unrequested_projection_refuse() {
    let mut rig = Rig::new();
    let projected = rig.projected;
    let target = Some(WindowTarget::at(DomainId(1), 30, 40, 0));
    let preview = rig.preview(Some(projected));
    assert!(
        capture(&rig.engine.world, preview, None).is_err(),
        "no target cannot imply an extra projection"
    );
    let window = rig
        .state()
        .domains
        .get_mut(&DomainId(1))
        .expect("domain")
        .window
        .take();
    let preview = rig.preview(Some(projected));
    assert!(capture(&rig.engine.world, preview, target).is_err());
    rig.state()
        .domains
        .get_mut(&DomainId(1))
        .expect("domain")
        .window = window;
    rig.state()
        .domains
        .get_mut(&DomainId(1))
        .expect("domain")
        .window
        .as_mut()
        .expect("window")
        .region = "x".repeat(RECORD_BYTES).into();
    let preview = rig.preview(Some(projected));
    assert!(capture(&rig.engine.world, preview, target).is_err());
    assert_eq!(
        rig.engine.world.get(rig.actor).expect("actor").location,
        Some(rig.source)
    );
}
