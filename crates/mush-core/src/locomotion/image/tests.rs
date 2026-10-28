#![cfg(test)]
//! Real inert admission feeds the image; registered composition has separate
//! integration coverage. No image test supplies a replacement route or row.
use super::*;
use crate::GOD;
use crate::action::{
    ActionInvocation, ActionInvocationDraft, ActionOfferId, CanonicalHash, IdempotencyKey,
    IdempotencyScope, PerceivedRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
};
use crate::action_state::{ActionStateStore, TypedStateLens};
use crate::locomotion::{LocomotionDecision, LocomotionRequest};
use crate::mechanics::{CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, OwnerId};
use serde_json::json;

struct Fixture {
    world: World,
    actor: ObjId,
    source: ObjId,
    destination: ObjId,
    exit: ObjId,
    action: DefinitionRef,
    participant: DefinitionRef,
    commit: CommitToken,
}
fn definition(kind: &str, id: &str) -> DefinitionRef {
    crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("sim").unwrap(),
            DefinitionDescriptor::new(
                DefinitionKind::new(kind).unwrap(),
                DefinitionId::new(id).unwrap(),
                1,
                json!({"fixture":"ordinary_image"}),
            )
            .unwrap(),
        )
        .unwrap()
}
fn fixture(kind: ObjKind) -> Fixture {
    let mut world = World::new_minimal();
    let source = world.create(ObjKind::Room, "source", GOD, None).unwrap();
    let destination = world
        .create(ObjKind::Room, "destination", GOD, None)
        .unwrap();
    let actor = world.create(kind, "walker", GOD, Some(source)).unwrap();
    let exit = world
        .create(ObjKind::Exit, "north", GOD, Some(source))
        .unwrap();
    world.link_exit(exit, Some(destination)).unwrap();
    world.set_home(actor, Some(destination)).unwrap();
    Fixture {
        world,
        actor,
        source,
        destination,
        exit,
        action: definition("action", "ordinary_locomotion"),
        participant: definition("owner_participant", "locomotion_capture"),
        commit: CommitToken::from_bytes([51; 16]),
    }
}
impl Fixture {
    fn admission(&self, request: LocomotionRequest) -> LocomotionAdmission {
        let request_id = RequestUid::from_bytes([51; 16]);
        let principal = PrincipalId::Object { id: self.actor };
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: request_id,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([52; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("sim").unwrap(),
                    namespace: CanonicalId::new("ordinary_image").unwrap(),
                },
                1,
                IdempotencyKey::Request(request_id),
            ),
        })
        .unwrap();
        let state = ActionStateStore::unregistered();
        match crate::locomotion::derive(
            &self.world,
            &invocation,
            &self.action,
            self.commit,
            &[],
            TypedStateLens::Live(&state),
            request,
        )
        .unwrap()
        {
            LocomotionDecision::Admitted(admission) => admission,
            LocomotionDecision::AlreadyHome => panic!("fixture must admit a route"),
        }
    }
    fn north(&self) -> LocomotionAdmission {
        self.admission(LocomotionRequest::exit_name("north").unwrap())
    }
    fn image(
        &self,
        admission: &LocomotionAdmission,
        dependencies: &BTreeSet<ObjId>,
    ) -> Result<LocomotionObjects, ParticipantError> {
        LocomotionObjects::prepare(
            &self.world,
            admission,
            dependencies,
            self.participant.clone(),
            self.action.clone(),
            self.commit,
        )
    }
}

#[test]
fn player_and_npc_images_match_the_actual_world_move() {
    for kind in [ObjKind::Player, ObjKind::Npc] {
        let mut f = fixture(kind);
        let image = f.image(&f.north(), &BTreeSet::new()).unwrap();
        assert_eq!(image.writes().len(), 3);
        assert_eq!(image.digests().unwrap().len(), 3);
        assert_eq!(f.world.get(f.actor).unwrap().location, Some(f.source));
        let expected = image.after.clone();
        // The legacy primitive is the semantic oracle for row values/order.
        f.world.move_to(f.actor, f.destination).unwrap();
        for (id, object) in &expected {
            assert_eq!(f.world.get(*id), Some(object));
        }
        let (_, storage) = f.world.composed_install_parts_mut();
        assert!(image.bind(storage).is_err());
    }
}

#[test]
fn bind_installs_derived_rows_without_rewriting_inventory() {
    let mut f = fixture(ObjKind::Player);
    let bag = f
        .world
        .create(ObjKind::Thing, "bag", GOD, Some(f.actor))
        .unwrap();
    let cargo = f
        .world
        .create(ObjKind::Thing, "cargo", GOD, Some(bag))
        .unwrap();
    let cargo_before = f.world.get(cargo).unwrap().clone();
    let image = f.image(&f.north(), &BTreeSet::new()).unwrap();
    let (_, storage) = f.world.composed_install_parts_mut();
    image.bind(storage).unwrap().apply();
    assert_eq!(f.world.get(f.actor).unwrap().location, Some(f.destination));
    assert!(!f.world.get(f.source).unwrap().contents.contains(&f.actor));
    assert_eq!(f.world.get(f.destination).unwrap().contents, vec![f.actor]);
    assert_eq!(f.world.get(cargo), Some(&cargo_before));
    assert_eq!(f.world.get(bag).unwrap().location, Some(f.actor));
}

#[test]
fn home_from_unlocated_actor_has_exactly_two_rows() {
    let mut f = fixture(ObjKind::Npc);
    f.world
        .get_mut(f.source)
        .unwrap()
        .contents
        .retain(|id| *id != f.actor);
    f.world.get_mut(f.actor).unwrap().location = None;
    let image = f
        .image(&f.admission(LocomotionRequest::home()), &BTreeSet::new())
        .unwrap();
    assert_eq!(image.writes().len(), 2);
    let source_before = f.world.get(f.source).unwrap().clone();
    let (_, storage) = f.world.composed_install_parts_mut();
    image.bind(storage).unwrap().apply();
    assert_eq!(f.world.get(f.actor).unwrap().location, Some(f.destination));
    assert_eq!(f.world.get(f.source), Some(&source_before));
}

#[test]
fn self_loop_exit_preserves_all_bytes_and_contents_order() {
    let mut f = fixture(ObjKind::Player);
    f.world.link_exit(f.exit, Some(f.source)).unwrap();
    f.world
        .create(ObjKind::Thing, "later occupant", GOD, Some(f.source))
        .unwrap();
    let actor_before = f.world.get(f.actor).unwrap().clone();
    let source_before = f.world.get(f.source).unwrap().clone();
    let image = f.image(&f.north(), &BTreeSet::new()).unwrap();
    assert!(image.writes().is_empty());
    assert!(image.digests().unwrap().is_empty());
    let (_, storage) = f.world.composed_install_parts_mut();
    image.bind(storage).unwrap().apply();
    assert_eq!(f.world.get(f.actor), Some(&actor_before));
    assert_eq!(f.world.get(f.source), Some(&source_before));
}

#[test]
fn duplicate_source_destination_or_inventory_inverse_refuses() {
    for corruption in 0..3 {
        let mut f = fixture(ObjKind::Player);
        match corruption {
            0 => f.world.get_mut(f.source).unwrap().contents.push(f.actor),
            1 => f
                .world
                .get_mut(f.destination)
                .unwrap()
                .contents
                .push(f.actor),
            _ => {
                let cargo = f.world.create(ObjKind::Thing, "cargo", GOD, None).unwrap();
                f.world.get_mut(f.actor).unwrap().contents.push(cargo);
            }
        }
        assert!(f.image(&f.north(), &BTreeSet::new()).is_err());
        assert_eq!(f.world.get(f.actor).unwrap().location, Some(f.source));
    }
}

#[test]
fn read_only_dependencies_and_route_rows_refuse_late_drift_at_bind() {
    for which in 0..3 {
        let mut f = fixture(ObjKind::Player);
        let dependency = f
            .world
            .create(ObjKind::Thing, "topology", GOD, None)
            .unwrap();
        let image = f.image(&f.north(), &BTreeSet::from([dependency])).unwrap();
        f.world
            .set_name([f.source, f.destination, dependency][which], "changed")
            .unwrap();
        assert!(image.local(&f.world).is_err());
        let (_, storage) = f.world.composed_install_parts_mut();
        assert!(image.bind(storage).is_err());
        assert_eq!(f.world.get(f.actor).unwrap().location, Some(f.source));
    }
}

#[test]
fn unrelated_allocation_does_not_stale_an_image() {
    let mut f = fixture(ObjKind::Player);
    let image = f.image(&f.north(), &BTreeSet::new()).unwrap();
    f.world
        .create(ObjKind::Thing, "unrelated", GOD, None)
        .unwrap();
    image.local(&f.world).unwrap();
    let (_, storage) = f.world.composed_install_parts_mut();
    image.bind(storage).unwrap().apply();
    assert_eq!(f.world.get(f.actor).unwrap().location, Some(f.destination));
}

#[test]
fn descendant_depth_and_oversized_dependency_refuse_before_images() {
    let mut f = fixture(ObjKind::Player);
    let mut parent = f.actor;
    for _ in 0..crate::MAX_CONTAINMENT_DEPTH {
        parent = f
            .world
            .create(ObjKind::Thing, "nested", GOD, Some(parent))
            .unwrap();
    }
    assert!(f.image(&f.north(), &BTreeSet::new()).is_err());

    let mut f = fixture(ObjKind::Player);
    let admission = f.north();
    let huge = f.world.create(ObjKind::Thing, "huge", GOD, None).unwrap();
    f.world.get_mut(huge).unwrap().name = "x".repeat(IMAGE_BYTES);
    assert!(f.image(&admission, &BTreeSet::from([huge])).is_err());
}

#[test]
fn admission_cannot_be_transplanted_to_another_action_or_commit() {
    let f = fixture(ObjKind::Player);
    let admission = f.north();
    for (action, commit) in [
        (definition("action", "another_action"), f.commit),
        (f.action.clone(), CommitToken::from_bytes([99; 16])),
    ] {
        assert!(
            LocomotionObjects::prepare(
                &f.world,
                &admission,
                &BTreeSet::new(),
                f.participant.clone(),
                action,
                commit
            )
            .is_err()
        );
    }
}

#[test]
fn actor_subtree_limit_is_exact_and_does_not_leave_partial_images() {
    for children in [MAX_LOCOMOTION_SUBTREE - 1, MAX_LOCOMOTION_SUBTREE] {
        let mut f = fixture(ObjKind::Player);
        for _ in 0..children {
            f.world
                .create(ObjKind::Thing, "cargo", GOD, Some(f.actor))
                .unwrap();
        }
        let result = f.image(&f.north(), &BTreeSet::new());
        assert_eq!(result.is_ok(), children + 1 == MAX_LOCOMOTION_SUBTREE);
        assert_eq!(f.world.get(f.actor).unwrap().location, Some(f.source));
    }
}

#[test]
fn late_oversized_read_dependency_refuses_under_the_exclusive_storage_borrow() {
    let mut f = fixture(ObjKind::Player);
    let dependency = f
        .world
        .create(ObjKind::Thing, "topology", GOD, None)
        .unwrap();
    let image = f.image(&f.north(), &BTreeSet::from([dependency])).unwrap();
    f.world.get_mut(dependency).unwrap().name = "x".repeat(IMAGE_BYTES);
    let (_, storage) = f.world.composed_install_parts_mut();
    assert!(image.bind(storage).is_err());
    assert_eq!(f.world.get(f.actor).unwrap().location, Some(f.source));
}
