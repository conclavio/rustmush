#![cfg(test)]

use super::*;
use mush_core::{GOD, LIMBO};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture {
    world: Box<World>,
    state: Box<SpatialState>,
    first: ObjId,
    second: ObjId,
    bay: ObjId,
    people: Vec<ObjId>,
}
impl Fixture {
    fn new(count: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let mut world = Box::new(World::new_minimal());
        world.set_now(10);
        let first = world.create(ObjKind::Thing, "First carrier", GOD, Some(LIMBO))?;
        let second = world.create(ObjKind::Thing, "Second carrier", GOD, Some(LIMBO))?;
        let bay = world.create(ObjKind::Room, "Personnel bay", GOD, None)?;
        let mut people = Vec::new();
        for index in 0..count {
            people.push(world.create(ObjKind::Npc, &format!("Crew {index}"), GOD, Some(bay))?);
        }
        let mut state = Box::new(SpatialState::default());
        state.spaces.insert(bay, space(bay, first));
        state.derived = crate::graph::rebuild(&state);
        Ok(Self {
            world,
            state,
            first,
            second,
            bay,
            people,
        })
    }

    fn baseline(&self) -> BTreeSet<ObjId> {
        BTreeSet::from([self.first, self.second, self.bay])
    }

    fn reanchor(&self) -> Result<(Transition, Plan), Box<dyn std::error::Error>> {
        let transition = Transition::Space {
            space: self.bay,
            anchor: SpaceAnchor::MovingObject { host: self.second },
        };
        let mut replacement = self.state.spaces.get(&self.bay).ok_or("bay")?.clone();
        replacement.anchor = SpaceAnchor::MovingObject { host: self.second };
        let plan = Plan {
            set_spaces: vec![replacement],
            ..Plan::default()
        };
        Ok((transition, plan))
    }

    fn capture(
        &self,
        transition: &Transition,
        plan: &Plan,
    ) -> Result<CaptureFacts, ParticipantError> {
        derive(
            &self.state,
            &self.world,
            transition,
            plan,
            &self.world.world_ic_now(),
            &self.baseline(),
        )
    }
}

fn space(object: ObjId, host: ObjId) -> InteriorSpaceRecord {
    InteriorSpaceRecord {
        object,
        anchor: SpaceAnchor::MovingObject { host },
        compartments: BTreeMap::new(),
        rooms: BTreeSet::from([object]),
        egresses: BTreeMap::new(),
    }
}

fn placement(object: ObjId, parent: SpatialParent) -> PlacementRecord {
    PlacementRecord {
        object,
        parent,
        revision: 1,
        unplaced: None,
    }
}

#[test]
fn dt_capture_reanchor_records_actual_unplaced_people_without_world_moves() -> TestResult {
    let f = Fixture::new(1)?;
    let (transition, plan) = f.reanchor()?;
    let capture = f.capture(&transition, &plan)?;
    check!(plan.projections.is_empty());
    check_eq!(
        capture.crossings,
        vec![
            Crossing {
                character: f.people[0],
                carrier: f.first,
                direction: CrossingDirection::Exit
            },
            Crossing {
                character: f.people[0],
                carrier: f.second,
                direction: CrossingDirection::Enter
            },
        ]
    );
    check_eq!(
        f.world.get(f.people[0]).and_then(|o| o.location),
        Some(f.bay)
    );
    check!(!f.state.is_placed(f.people[0]));
    check_eq!(capture.prepared_at, f.world.world_ic_now());
    check!(capture.read_dependencies.contains(&f.people[0]));
    check!(capture.read_dependencies.contains(&LIMBO));
    Ok(())
}

#[test]
fn dt_capture_legacy_single_room_and_nested_contents_are_real_membership() -> TestResult {
    let mut f = Fixture::new(1)?;
    f.state.spaces.get_mut(&f.bay).ok_or("bay")?.rooms.clear();
    f.state.derived = crate::graph::rebuild(&f.state);
    let box_id = f
        .world
        .create(ObjKind::Thing, "Rescue container", GOD, Some(f.bay))?;
    f.world.move_to(f.people[0], box_id)?;
    let (transition, plan) = f.reanchor()?;
    let capture = f.capture(&transition, &plan)?;
    check_eq!(capture.crossings.len(), 2);
    check!(capture.read_dependencies.contains(&box_id));
    check!(capture.read_dependencies.contains(&f.bay));
    Ok(())
}

#[test]
fn dt_capture_nested_hull_changes_only_outer_carrier_pairs() -> TestResult {
    let mut f = Fixture::new(1)?;
    let nested = f
        .world
        .create(ObjKind::Thing, "Nested carrier", GOD, Some(f.bay))?;
    let cabin = f.world.create(ObjKind::Room, "Nested cabin", GOD, None)?;
    f.state.spaces.insert(cabin, space(cabin, nested));
    f.state.placements.insert(
        nested,
        placement(
            nested,
            SpatialParent::Interior {
                space: f.bay,
                slot: None,
            },
        ),
    );
    f.world.move_to(f.people[0], cabin)?;
    f.state.derived = crate::graph::rebuild(&f.state);
    let (transition, plan) = f.reanchor()?;
    let mut baseline = f.baseline();
    baseline.extend([nested, cabin]);
    let capture = derive(
        &f.state,
        &f.world,
        &transition,
        &plan,
        &f.world.world_ic_now(),
        &baseline,
    )?;
    check_eq!(capture.crossings.len(), 2);
    check!(capture.crossings.iter().all(|row| row.carrier != nested));
    check!(
        capture
            .crossings
            .iter()
            .any(|row| row.carrier == f.first && row.direction == CrossingDirection::Exit)
    );
    check!(
        capture
            .crossings
            .iter()
            .any(|row| row.carrier == f.second && row.direction == CrossingDirection::Enter)
    );
    check!(capture.read_dependencies.contains(&cabin));
    check!(capture.read_dependencies.contains(&nested));
    Ok(())
}

#[test]
fn dt_capture_census_follows_physically_nested_unplaced_hull_cabins() -> TestResult {
    let mut f = Fixture::new(1)?;
    let nested = f
        .world
        .create(ObjKind::Thing, "Unplaced nested hull", GOD, Some(f.bay))?;
    let cabin = f
        .world
        .create(ObjKind::Room, "Separate nested cabin", GOD, None)?;
    let member = f
        .world
        .create(ObjKind::Room, "Nested member room", GOD, None)?;
    let mut nested_space = space(cabin, nested);
    nested_space.rooms.insert(member);
    f.state.spaces.insert(cabin, nested_space);
    f.world.move_to(f.people[0], member)?;
    f.state.derived = crate::graph::rebuild(&f.state);
    check!(!f.state.is_placed(nested));
    let (transition, plan) = f.reanchor()?;
    // Deliberately exclude the physically-only hull and its rooms from baseline.
    let capture = f.capture(&transition, &plan)?;
    check_eq!(capture.crossings.len(), 2);
    check!(
        capture
            .crossings
            .iter()
            .any(|row| row.carrier == f.first && row.direction == CrossingDirection::Exit)
    );
    check!(
        capture
            .crossings
            .iter()
            .any(|row| row.carrier == f.second && row.direction == CrossingDirection::Enter)
    );
    check!(capture.crossings.iter().all(|row| row.carrier != nested));
    for node in [nested, cabin, member, f.people[0]] {
        check!(capture.read_dependencies.contains(&node));
    }
    Ok(())
}

#[test]
fn dt_capture_first_and_last_interior_census_actual_host_contents() -> TestResult {
    let mut f = Fixture::new(1)?;
    let nested = f
        .world
        .create(ObjKind::Thing, "Physical host", GOD, Some(LIMBO))?;
    let cabin = f
        .world
        .create(ObjKind::Room, "New host interior", GOD, None)?;
    f.world.move_to(f.people[0], nested)?;
    let transition = Transition::Space {
        space: cabin,
        anchor: SpaceAnchor::MovingObject { host: nested },
    };
    let first = Plan {
        set_spaces: vec![space(cabin, nested)],
        ..Plan::default()
    };
    let baseline = BTreeSet::from([cabin, nested]);
    let capture = derive(
        &f.state,
        &f.world,
        &transition,
        &first,
        &f.world.world_ic_now(),
        &baseline,
    )?;
    check_eq!(
        capture.crossings,
        vec![Crossing {
            character: f.people[0],
            carrier: nested,
            direction: CrossingDirection::Enter
        }]
    );
    check!(capture.read_dependencies.contains(&nested));

    f.state.spaces.insert(cabin, space(cabin, nested));
    f.state.derived = crate::graph::rebuild(&f.state);
    let transition = Transition::Space {
        space: cabin,
        anchor: SpaceAnchor::MovingObject { host: f.second },
    };
    let last = Plan {
        set_spaces: vec![space(cabin, f.second)],
        ..Plan::default()
    };
    let capture = derive(
        &f.state,
        &f.world,
        &transition,
        &last,
        &f.world.world_ic_now(),
        &baseline,
    )?;
    check_eq!(
        capture.crossings,
        vec![Crossing {
            character: f.people[0],
            carrier: nested,
            direction: CrossingDirection::Exit
        }]
    );
    check!(capture.read_dependencies.contains(&f.people[0]));
    Ok(())
}

#[test]
fn dt_capture_direct_carrier_ancestor_with_interior_presence_refuses() -> TestResult {
    use crate::data::{DomainClock, DomainId, DomainRecord, DomainStatus, Pose};
    let mut f = Fixture::new(1)?;
    let nested = f
        .world
        .create(ObjKind::Thing, "Stale direct hull", GOD, Some(f.bay))?;
    let cabin = f
        .world
        .create(ObjKind::Room, "Direct hull cabin", GOD, None)?;
    let domain = DomainId(7);
    f.state.domains.insert(
        domain,
        DomainRecord {
            id: domain,
            host: LIMBO,
            driver: crate::data::passive_definition_ref(),
            status: DomainStatus::Active,
            clock: DomainClock::Passive,
            presentation_location: LIMBO,
            revision: 1,
            window: None,
        },
    );
    let direct = SpatialParent::Direct {
        domain,
        pose: Pose::passive(crate::data::passive_rules_value(serde_json::json!({}))?),
    };
    f.state
        .placements
        .insert(nested, placement(nested, direct.clone()));
    f.state.spaces.insert(cabin, space(cabin, nested));
    f.world.move_to(f.people[0], cabin)?;
    f.state.derived = crate::graph::rebuild(&f.state);
    let transition = Transition::Place {
        obj: nested,
        parent: direct.clone(),
    };
    let plan = Plan {
        set_placements: vec![placement(nested, direct)],
        projections: vec![(nested, LIMBO)],
        ..Plan::default()
    };
    check!(f.capture(&transition, &plan).is_err());
    // With a coherent before projection the same direct carrier remains valid.
    f.world.move_to(nested, LIMBO)?;
    check!(f.capture(&transition, &plan)?.crossings.is_empty());
    Ok(())
}

#[test]
fn dt_capture_explicit_interior_requires_direct_room_projection() -> TestResult {
    let mut f = Fixture::new(1)?;
    let container = f
        .world
        .create(ObjKind::Thing, "Intervening container", GOD, Some(f.bay))?;
    f.world.move_to(f.people[0], container)?;
    f.state.placements.insert(
        f.people[0],
        placement(
            f.people[0],
            SpatialParent::Interior {
                space: f.bay,
                slot: None,
            },
        ),
    );
    f.state.derived = crate::graph::rebuild(&f.state);
    let (transition, plan) = f.reanchor()?;
    check!(f.capture(&transition, &plan).is_err());
    f.state.placements.remove(&f.people[0]);
    f.state.derived = crate::graph::rebuild(&f.state);
    check_eq!(f.capture(&transition, &plan)?.crossings.len(), 2);
    Ok(())
}

#[test]
fn dt_capture_exact_projection_overlay_records_entry_without_mutation() -> TestResult {
    let mut f = Fixture::new(1)?;
    let person = f.people[0];
    f.world.move_to(person, LIMBO)?;
    let parent = SpatialParent::Interior {
        space: f.bay,
        slot: None,
    };
    let transition = Transition::Place {
        obj: person,
        parent: parent.clone(),
    };
    let plan = Plan {
        set_placements: vec![placement(person, parent)],
        projections: vec![(person, f.bay)],
        ..Plan::default()
    };
    let capture = f.capture(&transition, &plan)?;
    check_eq!(
        capture.crossings,
        vec![Crossing {
            character: person,
            carrier: f.first,
            direction: CrossingDirection::Enter
        }]
    );
    check_eq!(f.world.get(person).and_then(|o| o.location), Some(LIMBO));
    check!(!f.state.is_placed(person));
    Ok(())
}

#[test]
fn dt_capture_same_carrier_walk_and_keep_location_unplace_emit_no_crossing() -> TestResult {
    let mut f = Fixture::new(1)?;
    let cabin = f
        .world
        .create(ObjKind::Room, "Other member room", GOD, None)?;
    f.state
        .spaces
        .get_mut(&f.bay)
        .ok_or("bay")?
        .rooms
        .insert(cabin);
    let person = f.people[0];
    let parent = SpatialParent::Interior {
        space: f.bay,
        slot: None,
    };
    f.state
        .placements
        .insert(person, placement(person, parent.clone()));
    f.state.derived = crate::graph::rebuild(&f.state);
    let transition = Transition::Place {
        obj: person,
        parent,
    };
    let walk = Plan {
        projections: vec![(person, cabin)],
        ..Plan::default()
    };
    let capture = f.capture(&transition, &walk)?;
    check!(capture.crossings.is_empty());
    check_ne!(
        capture.before_ancestry_digest,
        capture.after_ancestry_digest
    );
    let unplace = Plan {
        set_placements: vec![placement(person, SpatialParent::Unplaced)],
        ..Plan::default()
    };
    check!(f.capture(&transition, &unplace)?.crossings.is_empty());
    Ok(())
}

#[test]
fn dt_capture_missing_projection_cannot_turn_placement_into_presence() -> TestResult {
    let mut f = Fixture::new(1)?;
    let person = f.people[0];
    f.world.move_to(person, LIMBO)?;
    let parent = SpatialParent::Interior {
        space: f.bay,
        slot: None,
    };
    let transition = Transition::Place {
        obj: person,
        parent: parent.clone(),
    };
    let plan = Plan {
        set_placements: vec![placement(person, parent)],
        ..Plan::default()
    };
    check!(f.capture(&transition, &plan).is_err());
    Ok(())
}

#[test]
fn dt_capture_pair_limit_counts_both_sides_of_transfer() -> TestResult {
    let mut f = Fixture::new(128)?;
    let (transition, plan) = f.reanchor()?;
    check_eq!(
        f.capture(&transition, &plan)?.crossings.len(),
        MAX_CROSSINGS
    );
    let extra = f
        .world
        .create(ObjKind::Npc, "One too many", GOD, Some(f.bay))?;
    check!(f.capture(&transition, &plan).is_err());
    check_eq!(f.world.get(extra).and_then(|o| o.location), Some(f.bay));
    check!(
        matches!(f.state.spaces.get(&f.bay).map(|s| &s.anchor), Some(SpaceAnchor::MovingObject { host }) if *host == f.first)
    );
    Ok(())
}

#[test]
fn dt_capture_actor_and_dependency_limits_refuse_whole_capture() -> TestResult {
    let f = Fixture::new(MAX_ACTORS + 1)?;
    let (transition, _) = f.reanchor()?;
    check!(f.capture(&transition, &Plan::default()).is_err());
    let mut f = Fixture::new(0)?;
    let mut baseline = f.baseline();
    while baseline.len() <= MAX_DEPENDENCIES {
        baseline.insert(f.world.create(
            ObjKind::Thing,
            &format!("Dependency {}", baseline.len()),
            GOD,
            None,
        )?);
    }
    let (transition, plan) = f.reanchor()?;
    check!(
        derive(
            &f.state,
            &f.world,
            &transition,
            &plan,
            &f.world.world_ic_now(),
            &baseline
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn dt_capture_broken_inverse_duplicate_projection_and_cycle_refuse() -> TestResult {
    let mut f = Fixture::new(1)?;
    let (transition, plan) = f.reanchor()?;
    f.world
        .get_mut(f.bay)
        .ok_or("bay")?
        .contents
        .push(f.people[0]);
    check!(f.capture(&transition, &plan).is_err());
    f.world.get_mut(f.bay).ok_or("bay")?.contents.pop();
    let duplicate = Plan {
        projections: vec![(f.people[0], f.bay), (f.people[0], f.bay)],
        ..Plan::default()
    };
    check!(f.capture(&transition, &duplicate).is_err());
    let mut cycle = plan;
    cycle.set_spaces[0].anchor = SpaceAnchor::MovingObject { host: f.people[0] };
    check!(f.capture(&transition, &cycle).is_err());
    Ok(())
}

#[test]
fn dt_capture_only_world_ic_stamps_and_semantic_digest_are_accepted() -> TestResult {
    let f = Fixture::new(1)?;
    let (transition, plan) = f.reanchor()?;
    let capture = f.capture(&transition, &plan)?;
    check_eq!(
        capture.capture_digest,
        crate::personnel::capture_digest(
            &capture.prepared_at,
            &capture.before_ancestry_digest,
            &capture.after_ancestry_digest,
            &capture.crossings
        )?
    );
    for clock in [ClockRef::Utc, ClockRef::WorldIc { generation: 0 }] {
        check!(
            derive(
                &f.state,
                &f.world,
                &transition,
                &plan,
                &ClockStamp { clock, at: 10 },
                &f.baseline()
            )
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn dt_capture_budget_refuses_before_unbounded_work() -> TestResult {
    let f = Fixture::new(0)?;
    let mut reads = Reads {
        world: &f.world,
        dependencies: BTreeSet::new(),
        contents: BTreeMap::new(),
        steps: MAX_STEPS,
    };
    check!(reads.step().is_err());
    check!(digest("test.personnel.limit", &"x".repeat(MAX_FACT_BYTES)).is_err());
    Ok(())
}
