#![cfg(test)]

use super::*;
use crate::data::{InteriorSpaceRecord, PlacementRecord, SpaceAnchor, SpatialParent, UnplacedInfo};
use crate::personnel::CrossingDirection;
use mush_core::{GOD, LIMBO};
use std::collections::BTreeMap;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn space(object: ObjId, host: ObjId) -> InteriorSpaceRecord {
    InteriorSpaceRecord {
        object,
        anchor: SpaceAnchor::MovingObject { host },
        compartments: BTreeMap::new(),
        rooms: BTreeSet::from([object]),
        egresses: BTreeMap::new(),
    }
}
struct Fixture {
    world: Box<World>,
    state: Box<SpatialState>,
    first: ObjId,
    second: ObjId,
    source: ObjId,
    destination: ObjId,
    actor: ObjId,
    exit: ObjId,
}
impl Fixture {
    fn new() -> TestResult<Self> {
        let mut world = Box::new(World::new_minimal());
        world.set_now(10);
        let first = world.create(ObjKind::Thing, "First carrier", GOD, Some(LIMBO))?;
        let second = world.create(ObjKind::Thing, "Second carrier", GOD, Some(LIMBO))?;
        let source = world.create(ObjKind::Room, "First cabin", GOD, None)?;
        let destination = world.create(ObjKind::Room, "Second cabin", GOD, None)?;
        let actor = world.create(ObjKind::Player, "Walker", GOD, Some(source))?;
        let exit = world.create(ObjKind::Exit, "across", GOD, Some(source))?;
        world.link_exit(exit, Some(destination))?;
        let mut state = Box::new(SpatialState::default());
        state.spaces.insert(source, space(source, first));
        state.spaces.insert(destination, space(destination, second));
        state.derived = crate::graph::rebuild(&state);
        Ok(Self {
            world,
            state,
            first,
            second,
            source,
            destination,
            actor,
            exit,
        })
    }
    fn route(&self) -> ActorMove {
        ActorMove {
            actor: self.actor,
            source: self.world.get(self.actor).and_then(|o| o.location),
            exit: Some(self.exit),
            destination: self.destination,
        }
    }
    fn capture(&self) -> Result<CaptureFacts, ParticipantError> {
        derive(
            &self.state,
            &self.world,
            &self.route(),
            &self.world.world_ic_now(),
        )
    }
}

#[test]
fn ordinary_transfer_records_actual_presence_without_creating_placement() -> TestResult {
    let f = Fixture::new()?;
    let before_actor = f.world.get(f.actor).ok_or("actor")?.clone();
    let before_spaces = f.state.spaces.clone();
    let facts = f.capture()?;
    check_eq!(
        facts.crossings,
        vec![
            Crossing {
                character: f.actor,
                carrier: f.first,
                direction: CrossingDirection::Exit
            },
            Crossing {
                character: f.actor,
                carrier: f.second,
                direction: CrossingDirection::Enter
            }
        ]
    );
    check_eq!(f.world.get(f.actor), Some(&before_actor));
    check!(!f.state.placements.contains_key(&f.actor));
    check_eq!(f.state.spaces, before_spaces);
    for object in [
        f.actor,
        f.source,
        f.destination,
        f.exit,
        f.first,
        f.second,
        LIMBO,
    ] {
        check!(facts.read_dependencies.contains(&object));
    }
    Ok(())
}

#[test]
fn explicit_unplaced_metadata_is_preserved_and_legacy_singleton_rooms_work() -> TestResult {
    let mut f = Fixture::new()?;
    for record in f.state.spaces.values_mut() {
        record.rooms.clear();
    }
    let record = PlacementRecord {
        object: f.actor,
        parent: SpatialParent::Unplaced,
        revision: 41,
        unplaced: Some(UnplacedInfo {
            reason: "retained staff release".into(),
            by: Some(GOD),
            at: 3,
            previous: None,
            quarantined: false,
        }),
    };
    f.state.placements.insert(f.actor, record.clone());
    f.state.derived = crate::graph::rebuild(&f.state);
    check_eq!(f.capture()?.crossings.len(), 2);
    check_eq!(f.state.placements.get(&f.actor), Some(&record));
    Ok(())
}

#[test]
fn same_interior_member_walk_has_zero_crossings_and_changes_ancestry_only() -> TestResult {
    let mut f = Fixture::new()?;
    f.state.spaces.remove(&f.destination);
    f.state
        .spaces
        .get_mut(&f.source)
        .ok_or("space")?
        .rooms
        .insert(f.destination);
    let record = PlacementRecord {
        object: f.actor,
        parent: SpatialParent::Interior {
            space: f.source,
            slot: None,
        },
        revision: 9,
        unplaced: None,
    };
    f.state.placements.insert(f.actor, record.clone());
    f.state.derived = crate::graph::rebuild(&f.state);
    let facts = f.capture()?;
    check!(facts.crossings.is_empty());
    check_ne!(facts.before_ancestry_digest, facts.after_ancestry_digest);
    check_eq!(f.state.placements.get(&f.actor), Some(&record));
    Ok(())
}

#[test]
fn ordinary_self_loop_exit_retains_zero_crossing_capture_without_reordering_contents() -> TestResult
{
    let mut f = Fixture::new()?;
    f.world.link_exit(f.exit, Some(f.source))?;
    let before_contents = f.world.get(f.source).ok_or("source")?.contents.clone();
    let route = ActorMove {
        destination: f.source,
        ..f.route()
    };
    let facts = derive(&f.state, &f.world, &route, &f.world.world_ic_now())?;
    check!(facts.crossings.is_empty());
    check_eq!(facts.before_ancestry_digest, facts.after_ancestry_digest);
    check_eq!(
        &f.world.get(f.source).ok_or("source")?.contents,
        &before_contents
    );
    let home = ActorMove {
        exit: None,
        ..route
    };
    check!(derive(&f.state, &f.world, &home, &f.world.world_ic_now()).is_err());
    Ok(())
}

#[test]
fn explicit_interior_cannot_be_reinterpreted_as_a_new_canonical_space() -> TestResult {
    let mut f = Fixture::new()?;
    f.state.placements.insert(
        f.actor,
        PlacementRecord {
            object: f.actor,
            parent: SpatialParent::Interior {
                space: f.source,
                slot: None,
            },
            revision: 1,
            unplaced: None,
        },
    );
    f.state.derived = crate::graph::rebuild(&f.state);
    check!(f.capture().is_err());
    check_eq!(
        f.world.get(f.actor).and_then(|o| o.location),
        Some(f.source)
    );
    Ok(())
}

#[test]
fn census_follows_nested_actor_contents_and_separate_hull_cabin() -> TestResult {
    let mut f = Fixture::new()?;
    let passenger = f
        .world
        .create(ObjKind::Npc, "Passenger", GOD, Some(f.actor))?;
    let nested = f
        .world
        .create(ObjKind::Thing, "Carried hull", GOD, Some(f.actor))?;
    let cabin = f
        .world
        .create(ObjKind::Room, "Separate hull cabin", GOD, None)?;
    let nested_person = f
        .world
        .create(ObjKind::Npc, "Nested passenger", GOD, Some(cabin))?;
    f.state.spaces.insert(cabin, space(cabin, nested));
    f.state.derived = crate::graph::rebuild(&f.state);
    let facts = f.capture()?;
    check_eq!(facts.crossings.len(), 6);
    for actor in [f.actor, passenger, nested_person] {
        check!(facts.crossings.iter().any(|row| row.character == actor
            && row.carrier == f.first
            && row.direction == CrossingDirection::Exit));
        check!(facts.crossings.iter().any(|row| row.character == actor
            && row.carrier == f.second
            && row.direction == CrossingDirection::Enter));
    }
    check!(facts.crossings.iter().all(|row| row.carrier != nested));
    for object in [passenger, nested, cabin, nested_person] {
        check!(facts.read_dependencies.contains(&object));
    }
    Ok(())
}

#[test]
fn stationary_bystanders_are_not_part_of_the_moving_actor_census() -> TestResult {
    let mut f = Fixture::new()?;
    let mut bystanders = Vec::new();
    for i in 0..300 {
        bystanders.push(f.world.create(
            ObjKind::Npc,
            &format!("Bystander {i}"),
            GOD,
            Some(if i % 2 == 0 { f.source } else { f.destination }),
        )?);
    }
    let facts = f.capture()?;
    check_eq!(facts.crossings.len(), 2);
    check!(facts.crossings.iter().all(|row| row.character == f.actor));
    check!(
        bystanders
            .iter()
            .all(|actor| !facts.read_dependencies.contains(actor))
    );
    Ok(())
}

#[test]
fn crossing_pair_limit_counts_each_transfer_side_and_refuses_whole_move() -> TestResult {
    let mut f = Fixture::new()?;
    for index in 1..128 {
        f.world.create(
            ObjKind::Npc,
            &format!("Carried {index}"),
            GOD,
            Some(f.actor),
        )?;
    }
    check_eq!(f.capture()?.crossings.len(), MAX_CROSSINGS);
    f.world
        .create(ObjKind::Npc, "One too many carried", GOD, Some(f.actor))?;
    check!(f.capture().is_err());
    check_eq!(
        f.world.get(f.actor).and_then(|o| o.location),
        Some(f.source)
    );
    Ok(())
}

#[test]
fn stale_route_and_incoherent_actual_inverse_are_refused() -> TestResult {
    let mut f = Fixture::new()?;
    let mut route = f.route();
    route.source = Some(LIMBO);
    check!(derive(&f.state, &f.world, &route, &f.world.world_ic_now()).is_err());
    route = f.route();
    route.destination = f.source;
    check!(derive(&f.state, &f.world, &route, &f.world.world_ic_now()).is_err());
    f.world
        .get_mut(f.destination)
        .ok_or("destination")?
        .contents
        .push(f.actor);
    check!(f.capture().is_err());
    f.world
        .get_mut(f.destination)
        .ok_or("destination")?
        .contents
        .retain(|id| *id != f.actor);
    f.world
        .get_mut(f.source)
        .ok_or("source")?
        .contents
        .retain(|id| *id != f.actor);
    check!(f.capture().is_err());
    Ok(())
}

#[test]
fn home_from_unlocated_actor_has_actual_empty_before_ancestry() -> TestResult {
    let mut f = Fixture::new()?;
    let actor = f
        .world
        .create(ObjKind::Npc, "Unlocated traveller", GOD, None)?;
    let route = ActorMove {
        actor,
        source: None,
        exit: None,
        destination: f.destination,
    };
    let facts = derive(&f.state, &f.world, &route, &f.world.world_ic_now())?;
    check_eq!(
        facts.crossings,
        vec![Crossing {
            character: actor,
            carrier: f.second,
            direction: CrossingDirection::Enter
        }]
    );
    check_eq!(f.world.get(actor).and_then(|o| o.location), None);
    check!(!f.state.placements.contains_key(&actor));
    Ok(())
}

#[test]
fn frozen_preparation_clock_survives_only_forward_same_generation_recheck() -> TestResult {
    let mut f = Fixture::new()?;
    let stamp = f.world.world_ic_now();
    crate::physical::check_capture_clock(&f.world, &stamp, true)?;
    let facts = derive(&f.state, &f.world, &f.route(), &stamp)?;
    f.world.set_now(11);
    crate::physical::check_capture_clock(&f.world, &stamp, false)?;
    check_eq!(derive(&f.state, &f.world, &f.route(), &stamp)?, facts);
    check!(crate::physical::check_capture_clock(&f.world, &stamp, true).is_err());
    let mut wrong = stamp.clone();
    wrong.clock = ClockRef::WorldIc { generation: 99 };
    check!(crate::physical::check_capture_clock(&f.world, &wrong, false).is_err());
    wrong = f.world.world_ic_now();
    wrong.at += 1;
    check!(crate::physical::check_capture_clock(&f.world, &wrong, false).is_err());
    Ok(())
}

fn batch() -> TestResult<CrossingBatch> {
    let f = Fixture::new()?;
    let facts = f.capture()?;
    let action =
        DefinitionRegistryBuilder::new().register(OwnerId::new("sim")?, action_descriptor()?)?;
    let participant = DefinitionRegistryBuilder::new().register(
        OwnerId::new("sim")?,
        crate::locomotion::participant_descriptor()?,
    )?;
    Ok(CrossingBatch {
        format: 1,
        commit: CommitToken::from_bytes([6; 16]),
        action,
        participant,
        actor: f.actor,
        source: Some(f.source),
        exit: None,
        destination: f.destination,
        prepared_at: facts.prepared_at,
        at_utc: 10,
        plan_digest: plan_digest(&f.route())?,
        capture_digest: facts.capture_digest,
        before_ancestry_digest: facts.before_ancestry_digest,
        after_ancestry_digest: facts.after_ancestry_digest,
        coverage: Coverage::UncertifiedLegacyPaths,
        crossings: facts.crossings,
    })
}

#[test]
fn record_requires_explicit_nullable_route_uncertified_coverage_and_exact_fields() -> TestResult {
    let original = batch()?;
    let value = encoded(&original)?;
    check_eq!(decode(&value)?, original);
    for field in [
        "source",
        "exit",
        "coverage",
        "plan_digest",
        "capture_digest",
        "prepared_at",
        "action",
        "participant",
    ] {
        let mut changed = value.clone();
        changed.as_object_mut().ok_or("object")?.remove(field);
        check!(decode(&changed).is_err());
        check!(serde_json::from_value::<CrossingBatch>(changed).is_err());
    }
    for coverage in [json!(null), json!("certified"), json!(true)] {
        let mut changed = value.clone();
        changed["coverage"] = coverage;
        check!(decode(&changed).is_err());
    }
    let mut changed = value.clone();
    changed["eligible"] = json!(true);
    check!(decode(&changed).is_err());
    let mut changed = value;
    changed["crossings"][0]["wages"] = json!(1);
    check!(decode(&changed).is_err());
    Ok(())
}

#[test]
fn capture_digest_detects_material_tampering_and_is_not_a_structural_capture() -> TestResult {
    let original = batch()?;
    let value = encoded(&original)?;
    for field in [
        "before_ancestry_digest",
        "after_ancestry_digest",
        "capture_digest",
    ] {
        let mut changed = value.clone();
        changed[field] = json!(original.plan_digest);
        check!(decode(&changed).is_err());
    }
    let mut changed = value.clone();
    changed["prepared_at"]["at"] = json!(999);
    check!(decode(&changed).is_err());
    let mut changed = value.clone();
    changed["crossings"][0]["direction"] = json!("enter");
    check!(decode(&changed).is_err());
    let mut changed = value;
    changed["crossings"][0]["character"] = json!(900);
    check!(decode(&changed).is_err());
    check_ne!(
        original.capture_digest,
        crate::personnel::capture_digest(
            &original.prepared_at,
            &original.before_ancestry_digest,
            &original.after_ancestry_digest,
            &original.crossings
        )?
    );
    Ok(())
}

#[test]
fn scene_and_ordinary_capture_families_reject_each_others_exact_protocol() -> TestResult {
    let ordinary = batch()?;
    let ordinary_value = encoded(&ordinary)?;
    check!(decode_with(&ordinary_value, Protocol::WithScenes).is_err());
    let mut joint = ordinary.clone();
    let mut definitions = DefinitionRegistryBuilder::new();
    joint.action = definitions.register(
        OwnerId::new("sim")?,
        mush_core::locomotion::scene_transition::action_descriptor()?,
    )?;
    joint.participant = definitions.register(
        OwnerId::new("sim")?,
        crate::locomotion_scenes::participant_descriptor()?,
    )?;
    let joint_value = encoded(&joint)?;
    check_eq!(decode_with(&joint_value, Protocol::WithScenes)?, joint);
    check!(decode(&joint_value).is_err());
    for field in [
        "source",
        "exit",
        "coverage",
        "prepared_at",
        "plan_digest",
        "capture_digest",
    ] {
        let mut missing = joint_value.clone();
        missing.as_object_mut().ok_or("object")?.remove(field);
        check!(decode_with(&missing, Protocol::WithScenes).is_err());
    }
    joint.participant = ordinary.participant;
    check!(decode_with(&encoded(&joint)?, Protocol::WithScenes).is_err());
    check_eq!(decode(&ordinary_value)?.action, ordinary.action);
    Ok(())
}

#[test]
fn records_refuse_unsorted_duplicate_self_pairs_and_wrong_clock_domains() -> TestResult {
    let original = batch()?;
    let mut wrong = original.crossings.clone();
    wrong.reverse();
    check!(
        capture_digest(
            &original.prepared_at,
            &original.before_ancestry_digest,
            &original.after_ancestry_digest,
            &wrong
        )
        .is_err()
    );
    wrong = vec![original.crossings[0].clone(), original.crossings[0].clone()];
    check!(
        capture_digest(
            &original.prepared_at,
            &original.before_ancestry_digest,
            &original.after_ancestry_digest,
            &wrong
        )
        .is_err()
    );
    wrong = vec![Crossing {
        character: ObjId(7),
        carrier: ObjId(7),
        direction: CrossingDirection::Enter,
    }];
    check!(
        capture_digest(
            &original.prepared_at,
            &original.before_ancestry_digest,
            &original.after_ancestry_digest,
            &wrong
        )
        .is_err()
    );
    for clock in [ClockRef::Utc, ClockRef::WorldIc { generation: 0 }] {
        let stamp = ClockStamp { clock, at: 10 };
        check!(
            capture_digest(
                &stamp,
                &original.before_ancestry_digest,
                &original.after_ancestry_digest,
                &[]
            )
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn durable_definition_versions_are_strict_and_zero_crossings_are_retained() -> TestResult {
    let mut original = batch()?;
    original.crossings.clear();
    original.capture_digest = capture_digest(
        &original.prepared_at,
        &original.before_ancestry_digest,
        &original.after_ancestry_digest,
        &original.crossings,
    )?;
    original.validate()?;
    check_eq!(decode(&encoded(&original)?)?, original);
    let mut descriptor = crate::locomotion::participant_descriptor()?;
    descriptor = mush_core::DefinitionDescriptor::new(
        descriptor.kind().clone(),
        descriptor.id().clone(),
        2,
        descriptor.payload().clone(),
    )?;
    original.participant =
        DefinitionRegistryBuilder::new().register(OwnerId::new("sim")?, descriptor)?;
    check!(original.validate().is_err());
    Ok(())
}

#[test]
fn route_digest_binds_source_exit_actor_and_destination() -> TestResult {
    let f = Fixture::new()?;
    let route = f.route();
    let digest = plan_digest(&route)?;
    for changed in [
        ActorMove {
            source: None,
            ..route.clone()
        },
        ActorMove {
            exit: None,
            ..route.clone()
        },
        ActorMove {
            actor: GOD,
            ..route.clone()
        },
        ActorMove {
            destination: LIMBO,
            ..route
        },
    ] {
        check_ne!(plan_digest(&changed)?, digest);
    }
    Ok(())
}
