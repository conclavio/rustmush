#![cfg(test)]

use super::*;
use crate::JourneyBook;
use crate::data::{
    DomainClock, DomainRecord, DomainStatus, HostAssignments, HostStations, StationAssignment,
    StationId,
};
use crate::focus::{ActiveFocus, FocusState, ReachProof};
use mush_core::{AccountId, BehaviorBinding, BehaviorId, GOD, LIMBO, ObjKind};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture {
    world: Box<World>,
    state: Box<SpatialState>,
    first: ObjId,
    second: ObjId,
    bay: ObjId,
    unit: ObjId,
    cabin: ObjId,
    walker: ObjId,
}
impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut world = Box::new(World::new_minimal());
        world.set_now(10);
        let first = world.create(ObjKind::Thing, "First hull", GOD, Some(LIMBO))?;
        let second = world.create(ObjKind::Thing, "Second hull", GOD, Some(LIMBO))?;
        let bay = world.create(ObjKind::Room, "Bay", GOD, None)?;
        let unit = world.create(ObjKind::Thing, "Nested hull", GOD, Some(bay))?;
        let cabin = world.create(ObjKind::Room, "Cabin", GOD, None)?;
        let walker = world.create(ObjKind::Npc, "Unplaced walker", GOD, Some(cabin))?;
        for (object, behavior) in [
            (unit, "sim.participant"),
            (bay, "sim.interior"),
            (cabin, "sim.interior"),
        ] {
            world.attach_behavior(
                object,
                BehaviorId::new(behavior)?,
                BehaviorBinding {
                    schema_version: 1,
                    attached_by: GOD,
                    attached_at: 0,
                },
            )?;
        }
        let mut state = Box::new(SpatialState::default());
        for (space, host) in [(bay, first), (cabin, unit)] {
            state.spaces.insert(
                space,
                InteriorSpaceRecord {
                    object: space,
                    anchor: SpaceAnchor::MovingObject { host },
                    compartments: BTreeMap::new(),
                    rooms: BTreeSet::from([space]),
                    egresses: BTreeMap::new(),
                },
            );
        }
        state.placements.insert(
            unit,
            PlacementRecord {
                object: unit,
                parent: SpatialParent::Interior {
                    space: bay,
                    slot: None,
                },
                revision: 1,
                unplaced: None,
            },
        );
        state.derived = graph::rebuild(&state);
        world.module_state_insert(FocusState::default());
        world.module_state_insert(JourneyBook::default());
        Ok(Self {
            world,
            state,
            first,
            second,
            bay,
            unit,
            cabin,
            walker,
        })
    }
    fn reanchor(&self) -> Transition {
        Transition::Space {
            space: self.bay,
            anchor: SpaceAnchor::MovingObject { host: self.second },
        }
    }
    fn intent(&self, transition: Transition) -> Result<Intent, ParticipantError> {
        let (_, before) = current_facts(&self.state, &self.world, GOD, &transition)?;
        Ok(Intent {
            at: self.world.now(),
            transition,
            before,
        })
    }
    fn focus(&self, account: AccountId, actor: ObjId, unit: ObjId) -> ActiveFocus {
        ActiveFocus {
            account,
            actor,
            driver: actor,
            unit,
            station: StationId::new("pilot").expect("station"),
            acquired_at: self.world.world_ic_now(),
            epoch: 1,
            reach: ReachProof::Console { room: self.cabin },
            proxy: false,
        }
    }
}

fn patch(f: &Fixture, intent: Intent) -> Result<Patch, ParticipantError> {
    let CapturedPlan {
        plan,
        digest,
        capture,
        nodes,
        domains,
        ..
    } = capture_plan(&f.state, &f.world, GOD, &intent, &f.world.world_ic_now())?;
    let intent = PreparedCrossingIntent {
        intent,
        prepared_at: capture.prepared_at,
        capture_digest: capture.capture_digest,
    };
    let audit = f
        .state
        .recovery
        .prepare_append(RECOVERY_LOG, plan.audit.clone())
        .map_err(|_| ParticipantError::Encoding)?;
    let audit_write = audit
        .authoritative_replay_write(owner()?)
        .map_err(|_| ParticipantError::Encoding)?;
    let fence = StateFence {
        digest: state_digest(&f.state, &nodes, &domains)?,
        nodes,
        domains,
    };
    Ok(Patch {
        intent,
        digest,
        plan,
        audit,
        audit_write,
        fence,
    })
}

#[test]
fn dt_physical_zero_projection_reanchor_fences_both_roots_and_nested_walkers() -> TestResult {
    let mut f = Fixture::new()?;
    let transition = f.reanchor();
    let (_, nodes, _) = dependencies(&f.state, &f.world, &transition)?;
    for node in [f.first, f.second, f.bay, f.unit, f.cabin, f.walker] {
        check!(nodes.contains(&node));
    }
    check!(!f.state.placements.contains_key(&f.walker));
    let input = f.intent(transition)?;
    let (_, plan, _) = evaluate(&f.state, &f.world, GOD, &input)?;
    check!(plan.projections.is_empty());
    check_eq!(
        f.world.get(f.walker).and_then(|o| o.location),
        Some(f.cabin)
    );
    f.world.move_to(f.walker, LIMBO)?;
    check!(
        evaluate(&f.state, &f.world, GOD, &input).is_err(),
        "room facts changed with no structural placement delta"
    );
    Ok(())
}

#[test]
fn dt_physical_bind_refuses_changed_topology_and_retains_unrelated_rows() -> TestResult {
    let mut f = Fixture::new()?;
    let prepared = patch(&f, f.intent(f.reanchor())?)?;
    f.state.spaces.get_mut(&f.bay).ok_or("bay")?.anchor = SpaceAnchor::Unanchored;
    f.state.derived = graph::rebuild(&f.state);
    let before = f.state.clone();
    check!(
        Arc::new(SpatialParticipant)
            .bind(&mut f.state, prepared)
            .is_err()
    );
    check_eq!(f.state, before);
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);

    let prepared = patch(&f, f.intent(f.reanchor())?)?;
    let unrelated = f
        .world
        .create(ObjKind::Thing, "Unrelated", GOD, Some(LIMBO))?;
    f.state.control_gen.insert(unrelated, 7);
    let bound = Arc::new(SpatialParticipant).bind(&mut f.state, prepared)?;
    bound.apply();
    check_eq!(f.state.control_gen.get(&unrelated), Some(&7));
    check_eq!(
        f.state.spaces.get(&f.bay).ok_or("bay")?.anchor,
        SpaceAnchor::MovingObject { host: f.second }
    );
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 1);
    graph::verify_indexes(&f.state)?;
    Ok(())
}

#[test]
fn dt_physical_occupied_descendants_and_unplaced_focused_walkers_refuse() -> TestResult {
    let mut f = Fixture::new()?;
    f.state.assignments.insert(
        f.unit,
        HostAssignments {
            host: f.unit,
            seats: BTreeMap::from([(
                StationId::new("pilot").ok_or("station")?,
                StationAssignment {
                    actor: f.walker,
                    assigned_at: 0,
                    granted: None,
                },
            )]),
        },
    );
    check!(f.intent(f.reanchor()).is_err());
    f.state.assignments.clear();
    let account = AccountId::mint("walker", 1)?;
    let focus = f.focus(account, f.walker, f.second);
    f.world
        .module_state_mut::<FocusState>()
        .ok_or("focus")?
        .claim(focus);
    check!(
        f.intent(f.reanchor()).is_err(),
        "unplaced room actor is part of affected physical closure"
    );
    f.world
        .module_state_mut::<FocusState>()
        .ok_or("focus")?
        .relinquish(account);
    check!(f.intent(f.reanchor()).is_ok());
    Ok(())
}

#[test]
fn dt_physical_focus_unit_index_tracks_replacement_and_relinquishment() -> TestResult {
    let f = Fixture::new()?;
    let mut focus = FocusState::default();
    let first = AccountId::mint("first", 1)?;
    let second = AccountId::mint("second", 1)?;
    focus.claim(f.focus(first, f.walker, f.first));
    check!(focus.has_focus_on(f.first));
    focus.claim(f.focus(first, GOD, f.second));
    check!(!focus.has_focus_on(f.first));
    check!(focus.has_focus_on(f.second));
    focus.claim(f.focus(second, GOD, f.first));
    check!(!focus.has_focus_on(f.second));
    check!(focus.focus_of_account(first).is_none());
    check!(focus.has_focus_on(f.first));
    focus.relinquish(second);
    check!(!focus.has_focus_on(f.first));
    check!(focus.is_empty());
    Ok(())
}

#[test]
fn dt_physical_limits_refuse_without_truncation_or_mutation() -> TestResult {
    let mut f = Fixture::new()?;
    let before = f.state.clone();
    let input = f.intent(f.reanchor())?;
    for n in 0..=MAX_ROOM_CONTENTS {
        f.world
            .create(ObjKind::Thing, &format!("Cargo {n}"), GOD, Some(f.cabin))?;
    }
    check!(evaluate(&f.state, &f.world, GOD, &input).is_err());
    check_eq!(f.state, before);
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    check!(encoded_size(&"x".repeat(MAX_REQUEST_BYTES + 1), MAX_REQUEST_BYTES).is_err());
    for n in 0..=MAX_RESIDENT_RECORDS {
        f.state.control_gen.insert(ObjId(100_000 + n as u32), 1);
    }
    check!(resident_bound(&f.state).is_err());
    Ok(())
}

#[test]
fn dt_physical_refuses_nonadmin_expired_and_other_lifecycle_owners() -> TestResult {
    let mut f = Fixture::new()?;
    check!(current_facts(&f.state, &f.world, f.walker, &f.reanchor()).is_err());
    check!(
        f.intent(Transition::CarrierMove {
            root: f.first,
            parent: None
        })
        .is_err()
    );
    check!(
        f.intent(Transition::Quarantine {
            root: f.first,
            reason: "repair".into()
        })
        .is_err()
    );
    let input = f.intent(f.reanchor())?;
    f.world.set_now(71);
    check!(evaluate(&f.state, &f.world, GOD, &input).is_err());
    f.world.set_now(9);
    check!(evaluate(&f.state, &f.world, GOD, &input).is_err());
    Ok(())
}

#[test]
fn dt_physical_zero_projection_reanchor_refuses_departure_and_held_journeys() -> TestResult {
    let mut f = Fixture::new()?;
    let input = f.intent(f.reanchor())?;
    let route = crate::RouteDef::new(
        CanonicalId::new("test")?,
        (f.first, f.second),
        vec![crate::LegDef::new(f.second, 30)?],
        CanonicalId::new("manual")?,
        0,
    )?;
    let now = f.world.world_ic_now();
    let journey = crate::CarrierJourney::new(
        crate::JourneyId(1),
        f.first,
        route,
        SpatialParent::Interior {
            space: f.bay,
            slot: None,
        },
        now.clone(),
        now,
        0,
        crate::JourneyState::Cruising,
        None,
    )?;
    f.world
        .module_state_mut::<JourneyBook>()
        .ok_or("journeys")?
        .journeys
        .insert(f.first, journey);
    check!(
        evaluate(&f.state, &f.world, GOD, &input).is_err(),
        "departure changed zero-projection ancestry"
    );
    let book = f
        .world
        .module_state_mut::<JourneyBook>()
        .ok_or("journeys")?;
    book.journeys.clear();
    book.held
        .insert("cjourney:broken".into(), "bad data".into());
    check!(f.intent(f.reanchor()).is_err());
    Ok(())
}

#[test]
fn dt_physical_record_codec_requires_exact_keys_and_canonical_fields() -> TestResult {
    let f = Fixture::new()?;
    let row = f.state.placements.get(&f.unit).ok_or("unit")?;
    let exact = write(placement_key(f.unit).to_string(), row)?;
    check!(SpatialParticipant.accepts_write(&exact));
    let wrong = write(format!("placement:0{}", f.unit.0), row)?;
    check!(!SpatialParticipant.accepts_write(&wrong));
    if let ReplayWrite::ModuleJsonSet {
        namespace,
        key,
        mut value,
    } = exact
    {
        value
            .as_object_mut()
            .ok_or("object")?
            .insert("unrecognized".into(), json!(true));
        check!(
            !SpatialParticipant.accepts_write(&ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value
            })
        );
    }
    Ok(())
}

#[test]
fn dt_physical_held_ancestry_and_unbound_placement_facts_refuse() -> TestResult {
    let mut f = Fixture::new()?;
    let input = f.intent(f.reanchor())?;
    f.state.held_placements.insert(
        f.first,
        crate::HeldPlacement {
            key: placement_key(f.first),
            reason: "unreadable ancestor".into(),
        },
    );
    check!(evaluate(&f.state, &f.world, GOD, &input).is_err());
    f.state.held_placements.clear();
    f.state.placements.insert(
        f.first,
        PlacementRecord {
            object: f.first,
            parent: SpatialParent::Unplaced,
            revision: 1,
            unplaced: Some(crate::UnplacedInfo {
                reason: "recovery".into(),
                by: None,
                at: f.world.now(),
                previous: None,
                quarantined: true,
            }),
        },
    );
    check!(f.intent(f.reanchor()).is_err());
    f.state.placements.remove(&f.first);
    let prepared = patch(&f, f.intent(f.reanchor())?)?;
    f.state
        .unbound_placement_docs
        .insert("placement:unknown".into(), "bad key".into());
    check!(f.intent(f.reanchor()).is_err());
    check!(
        Arc::new(SpatialParticipant)
            .bind(&mut f.state, prepared)
            .is_err()
    );
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    Ok(())
}

#[test]
fn dt_physical_space_and_members_require_a_live_authored_space_room() -> TestResult {
    let mut f = Fixture::new()?;
    let plain_room = f
        .world
        .create(ObjKind::Room, "Walkable member", GOD, None)?;
    let members = Transition::Members {
        space: f.bay,
        rooms: BTreeSet::from([f.bay, plain_room]),
    };
    let members_input = f.intent(members.clone())?;
    check!(evaluate(&f.state, &f.world, GOD, &members_input).is_ok());
    let space_input = f.intent(f.reanchor())?;
    let before = f.state.clone();

    f.world.get_mut(f.bay).ok_or("bay")?.kind = ObjKind::Thing;
    check!(f.intent(f.reanchor()).is_err());
    check!(f.intent(members.clone()).is_err());
    f.world.get_mut(f.bay).ok_or("bay")?.kind = ObjKind::Room;
    f.world
        .detach_behavior(f.bay, &BehaviorId::new("sim.interior")?)?;
    check!(evaluate(&f.state, &f.world, GOD, &space_input).is_err());
    check!(evaluate(&f.state, &f.world, GOD, &members_input).is_err());
    check!(f.intent(members).is_err());
    check_eq!(f.state, before);
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    Ok(())
}

#[test]
fn dt_physical_unplace_cannot_forge_or_omit_the_issuing_actor() -> TestResult {
    let f = Fixture::new()?;
    let transition = |by| Transition::Unplace {
        obj: f.unit,
        reason: "staff recovery".into(),
        by,
        keep: true,
    };
    check!(f.intent(transition(Some(GOD))).is_ok());
    check!(f.intent(transition(Some(f.walker))).is_err());
    check!(f.intent(transition(None)).is_err());
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    Ok(())
}

#[test]
fn dt_physical_missing_or_going_read_dependencies_refuse() -> TestResult {
    let mut f = Fixture::new()?;
    let input = f.intent(f.reanchor())?;
    f.world
        .get_mut(f.second)
        .ok_or("second")?
        .flags
        .insert(Flags::GOING);
    check!(evaluate(&f.state, &f.world, GOD, &input).is_err());
    check!(f.intent(f.reanchor()).is_err());
    let missing = Transition::Space {
        space: f.bay,
        anchor: SpaceAnchor::MovingObject {
            host: ObjId(u32::MAX),
        },
    };
    check!(f.intent(missing).is_err());
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    Ok(())
}

#[test]
fn dt_physical_empty_control_hosts_refuse_but_generation_history_survives() -> TestResult {
    let mut f = Fixture::new()?;
    f.state.control_gen.insert(f.unit, 7);
    let input = f.intent(f.reanchor())?;
    let prepared = patch(&f, input.clone())?;
    f.state.stations.insert(
        f.unit,
        HostStations {
            host: f.unit,
            stations: BTreeMap::new(),
        },
    );
    check!(f.intent(f.reanchor()).is_err());
    check!(evaluate(&f.state, &f.world, GOD, &input).is_err());
    check!(
        Arc::new(SpatialParticipant)
            .bind(&mut f.state, prepared)
            .is_err()
    );
    f.state.stations.remove(&f.unit);

    f.state.assignments.insert(
        f.first,
        HostAssignments {
            host: f.first,
            seats: BTreeMap::new(),
        },
    );
    check!(f.intent(f.reanchor()).is_err());
    check_eq!(f.state.control_gen.get(&f.unit), Some(&7));
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    f.state.assignments.remove(&f.first);

    let prepared = patch(&f, f.intent(f.reanchor())?)?;
    Arc::new(SpatialParticipant)
        .bind(&mut f.state, prepared)?
        .apply();
    check_eq!(f.state.control_gen.get(&f.unit), Some(&7));
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 1);
    Ok(())
}

#[test]
fn dt_physical_implicit_space_room_captures_unplaced_focused_walkers() -> TestResult {
    let mut f = Fixture::new()?;
    let outside_unit = f
        .world
        .create(ObjKind::Thing, "Remote focus", GOD, Some(LIMBO))?;
    f.state
        .spaces
        .get_mut(&f.cabin)
        .ok_or("cabin")?
        .rooms
        .clear();
    f.state.derived = graph::rebuild(&f.state);
    let transition = Transition::Space {
        space: f.cabin,
        anchor: SpaceAnchor::MovingObject { host: f.unit },
    };
    let (_, nodes, _) = dependencies(&f.state, &f.world, &transition)?;
    check!(nodes.contains(&f.walker));
    check!(!nodes.contains(&outside_unit));
    check!(!f.state.placements.contains_key(&f.walker));
    check!(f.intent(transition.clone()).is_ok());
    let focus = f.focus(AccountId::mint("legacy walker", 1)?, f.walker, outside_unit);
    f.world
        .module_state_mut::<FocusState>()
        .ok_or("focus")?
        .claim(focus);
    check!(f.intent(transition.clone()).is_err());

    // A newly defined space has the same existing physical occupants.
    f.state.spaces.remove(&f.cabin);
    f.state.derived = graph::rebuild(&f.state);
    let (_, nodes, _) = dependencies(&f.state, &f.world, &transition)?;
    check!(nodes.contains(&f.walker));
    check!(!nodes.contains(&outside_unit));
    check!(f.intent(transition).is_err());
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    Ok(())
}

#[test]
fn dt_physical_retire_requires_its_lifecycle_owner_even_for_an_empty_passive_domain() -> TestResult
{
    let mut f = Fixture::new()?;
    let host = f
        .world
        .create(ObjKind::Thing, "Empty domain", GOD, Some(LIMBO))?;
    let domain = DomainRecord {
        id: DomainId(7),
        host,
        driver: crate::data::passive_definition_ref(),
        status: DomainStatus::Active,
        clock: DomainClock::Passive,
        presentation_location: LIMBO,
        revision: 1,
        window: None,
    };
    f.state.domains.insert(domain.id, domain.clone());
    f.state.derived = graph::rebuild(&f.state);
    let before = f.state.clone();
    check!(f.intent(Transition::Retire { domain: domain.id }).is_err());
    let mut retired = domain;
    retired.status = DomainStatus::Retired;
    retired.revision += 1;
    check!(!SpatialParticipant.accepts_write(&write(
        crate::data::domain_key(retired.id).to_string(),
        &retired,
    )?));
    check_eq!(f.state, before);
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    Ok(())
}

#[test]
fn dt_physical_capture_uses_exact_preparation_time_and_rechecks_frozen_facts() -> TestResult {
    let mut f = Fixture::new()?;
    let input = f.intent(f.reanchor())?;
    let captured = capture_plan(&f.state, &f.world, GOD, &input, &f.world.world_ic_now())?;
    let mut request = PreparedCrossingIntent {
        intent: input,
        prepared_at: captured.capture.prepared_at.clone(),
        capture_digest: captured.capture.capture_digest,
    };
    let initial = evaluate_capture(&f.state, &f.world, GOD, &request, true)?;
    check_eq!(initial.digest, captured.digest);

    f.world.set_now(11);
    let later = evaluate_capture(&f.state, &f.world, GOD, &request, false)?;
    check_eq!(later.capture.prepared_at, captured.capture.prepared_at);
    check_eq!(later.digest, captured.digest);
    check!(evaluate_capture(&f.state, &f.world, GOD, &request, true).is_err());

    request.capture_digest =
        DefinitionDigest::for_canonical_value("sim.test.wrong_capture", &json!({"wrong":true}))?;
    check!(evaluate_capture(&f.state, &f.world, GOD, &request, false).is_err());
    request.capture_digest = captured.capture.capture_digest;
    f.world.set_now(9);
    check!(check_capture_clock(&f.world, &request.prepared_at, false).is_err());
    f.world.set_now(11);
    let next = f.world.next_world_timeline_epoch(2)?;
    f.world.append_world_timeline_epoch(next)?;
    check!(check_capture_clock(&f.world, &request.prepared_at, false).is_err());
    check!(evaluate_capture(&f.state, &f.world, GOD, &request, false).is_err());
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    Ok(())
}

#[test]
fn dt_physical_capture_only_nested_occupants_and_hosts_pass_the_same_control_gates() -> TestResult {
    let mut f = Fixture::new()?;
    // This hull is physically in the bay without a structural placement. Its
    // separate anchored cabin and NPC are discovered only by the capture pass.
    f.state.placements.remove(&f.unit);
    f.state.derived = graph::rebuild(&f.state);
    let remote = f
        .world
        .create(ObjKind::Thing, "Remote focus", GOD, Some(LIMBO))?;
    let transition = f.reanchor();
    let (_, baseline, _) = dependencies(&f.state, &f.world, &transition)?;
    for node in [f.unit, f.cabin, f.walker, remote] {
        check!(!baseline.contains(&node));
    }
    let input = f.intent(transition)?;
    let captured = capture_plan(&f.state, &f.world, GOD, &input, &f.world.world_ic_now())?;
    for node in [f.unit, f.cabin, f.walker] {
        check!(captured.nodes.contains(&node));
    }
    let request = PreparedCrossingIntent {
        intent: input.clone(),
        prepared_at: captured.capture.prepared_at,
        capture_digest: captured.capture.capture_digest,
    };
    let account = AccountId::mint("physical cabin walker", 1)?;
    let focus = f.focus(account, f.walker, remote);
    f.world
        .module_state_mut::<FocusState>()
        .ok_or("focus")?
        .claim(focus);
    let before = f.state.clone();
    check!(evaluate(&f.state, &f.world, GOD, &input).is_ok());
    check!(capture_plan(&f.state, &f.world, GOD, &input, &f.world.world_ic_now()).is_err());
    check!(evaluate_capture(&f.state, &f.world, GOD, &request, false).is_err());
    check_eq!(f.state, before);
    f.world
        .module_state_mut::<FocusState>()
        .ok_or("focus")?
        .relinquish(account);

    f.state.stations.insert(
        f.unit,
        HostStations {
            host: f.unit,
            stations: BTreeMap::new(),
        },
    );
    let before = f.state.clone();
    check!(evaluate(&f.state, &f.world, GOD, &input).is_ok());
    check!(capture_plan(&f.state, &f.world, GOD, &input, &f.world.world_ic_now()).is_err());
    check!(evaluate_capture(&f.state, &f.world, GOD, &request, false).is_err());
    check_eq!(f.state, before);
    check_eq!(f.state.recovery.next_seq(RECOVERY_LOG), 0);
    check_eq!(
        f.world.get(f.walker).and_then(|o| o.location),
        Some(f.cabin)
    );
    Ok(())
}

#[test]
fn dt_physical_receipt_projects_only_the_committed_root_and_exact_v2_shape() -> TestResult {
    let mut definitions = mush_core::DefinitionRegistryBuilder::new();
    let action = definitions.register(
        owner()?,
        mush_core::physical_transition::action_descriptor()?,
    )?;
    let schema = definitions.register(owner()?, outcome_descriptor()?)?;
    let historical = definitions.register(owner()?, outcome_descriptor_v1()?)?;
    let digest = DefinitionDigest::for_canonical_value("sim.test.receipt", &json!({}))?;
    let data = json!({"root":42,"structural_digest":digest});
    let outcome = RulesValue::new(schema.clone(), data.clone())?;
    let projected = project_structural_receipt(&action, &schema, &action, &outcome)?;
    check_eq!(projected.summary(), "Spatial update for #42 committed.");
    check_eq!(projected.data(), &json!({"root":42}));

    for invalid in [
        json!({"root":42}),
        json!({"root":u64::MAX,"structural_digest":digest}),
        json!({"root":42,"structural_digest":"invalid"}),
        json!({"root":42,"structural_digest":digest,"name":"Hidden cabin"}),
    ] {
        let outcome = RulesValue::new(schema.clone(), invalid)?;
        check!(project_structural_receipt(&action, &schema, &action, &outcome).is_err());
    }
    let old_outcome = RulesValue::new(historical.clone(), data)?;
    check!(project_structural_receipt(&action, &schema, &action, &old_outcome).is_err());
    check!(project_structural_receipt(&action, &schema, &historical, &outcome).is_err());
    Ok(())
}
