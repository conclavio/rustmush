#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{GOD, LIMBO};
use mush_sim::data::{
    HostAssignments, HostStations, InteriorSpaceRecord, PlacementRecord, SpaceAnchor,
    SpatialParent, StationSpec, passive_rules_value,
};
use mush_sim::focus::ActiveFocus;
use mush_tactics::detachment::Slot;
use mush_tactics::detachment::planning::{Consent, Plan, PlanId, SlotPlan};

use super::*;
use crate::detachment::crew::personnel::{self, PersonnelIssue};
use crate::detachment::crew::{CrewContract, CrewDuty, HireTerms, NpcMissionScope};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn employment(
    fixture: &Fixture,
    npc: ObjId,
    scope: NpcMissionScope,
) -> Result<CrewContract, String> {
    let terms = HireTerms {
        schema: 1,
        port: mush_core::CanonicalId::new("ceres").map_err(|e| e.to_string())?,
        mission_scope: scope,
        duties: BTreeSet::from([CrewDuty::Mining]),
    };
    Ok(CrewContract {
        schema: 1,
        npc,
        employer: fixture.draft.organizer,
        hired_at: fixture.world.world_ic_now(),
        terms_hash: terms.digest()?,
        terms,
    })
}

struct Fixture {
    world: Box<World>,
    draft: Draft,
    actor: ObjId,
    unit: ObjId,
    carrier: ObjId,
    bay: ObjId,
    console: ObjId,
    side_room: ObjId,
    station: StationId,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut world = Box::new(World::new_minimal());
        let carrier = world.create(ObjKind::Thing, "Private carrier name", GOD, Some(LIMBO))?;
        let bay = world.create(ObjKind::Room, "Private bay name", GOD, None)?;
        let unit = world.create(ObjKind::Thing, "Private unit name", GOD, Some(bay))?;
        let console = world.create(ObjKind::Room, "Private console name", GOD, None)?;
        let side_room = world.create(ObjKind::Room, "Private cabin name", GOD, None)?;
        let actor = world.create(ObjKind::Player, "Private pilot name", GOD, Some(console))?;
        let mut state = SpatialState::default();
        for (space, host, rooms) in [
            (bay, carrier, BTreeSet::from([bay])),
            (console, unit, BTreeSet::from([console, side_room])),
        ] {
            state.spaces.insert(
                space,
                InteriorSpaceRecord {
                    object: space,
                    anchor: SpaceAnchor::MovingObject { host },
                    compartments: BTreeMap::new(),
                    rooms,
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
        let station = StationId::new("pilot").ok_or("station id")?;
        let helm = ChannelId::new("helm").ok_or("helm channel")?;
        state.stations.insert(
            unit,
            HostStations {
                host: unit,
                stations: BTreeMap::from([(
                    station.clone(),
                    StationSpec {
                        role: "pilot".into(),
                        channels: BTreeSet::from([helm]),
                        priority: 10,
                        space: Some(console),
                        policy: passive_rules_value(serde_json::json!({}))?,
                    },
                )]),
            },
        );
        state.assignments.insert(
            unit,
            HostAssignments {
                host: unit,
                seats: BTreeMap::from([(
                    station.clone(),
                    StationAssignment {
                        actor,
                        assigned_at: 0,
                        granted: None,
                    },
                )]),
            },
        );
        state.control_gen.insert(unit, 5);
        state.derived = mush_sim::graph::rebuild(&state);
        world.module_state_insert(state);
        let mut focus = FocusState::default();
        focus.claim_for_test(ActiveFocus {
            account: AccountId::mint("pilot", 0)?,
            actor,
            driver: actor,
            unit,
            station: station.clone(),
            acquired_at: world.world_ic_now(),
            // Captured focus epoch is not compared to live generation.
            epoch: 1,
            reach: ReachProof::Console { room: console },
            proxy: false,
        });
        world.module_state_insert(focus);
        let mut units = mush_tactics::module::UnitStore::default();
        units
            .units
            .insert(unit, crate::arena::test_support::tactical_unit(unit));
        world.module_state_insert(units);
        let plan = Plan {
            carrier,
            slots: vec![SlotPlan {
                slot: Slot::new(1)?,
                unit: Some(unit),
                controller: PlannedController::Player { character: actor },
                role: None,
                mind: None,
            }],
            npc_commander: None,
            financial_terms: None,
        };
        let draft = Draft {
            id: PlanId(1),
            organizer: GOD,
            revision: 1,
            plan_hash: plan.material_hash()?,
            plan,
            source_preset: None,
            consents: BTreeMap::new(),
        };
        Ok(Self {
            world,
            draft,
            actor,
            unit,
            carrier,
            bay,
            console,
            side_room,
            station,
        })
    }

    fn spatial(&mut self, edit: impl FnOnce(&mut SpatialState)) -> TestResult {
        let mut state = self
            .world
            .module_state::<SpatialState>()
            .ok_or("spatial state")?
            .clone();
        edit(&mut state);
        state.derived = mush_sim::graph::rebuild(&state);
        self.world.module_state_insert(state);
        Ok(())
    }

    fn focus(&mut self, edit: impl FnOnce(&mut ActiveFocus)) -> TestResult {
        let mut focus = self
            .world
            .module_state::<FocusState>()
            .ok_or("focus state")?
            .focus_of_actor(self.actor)
            .ok_or("focus")?
            .clone();
        edit(&mut focus);
        let mut state = FocusState::default();
        state.claim_for_test(focus);
        self.world.module_state_insert(state);
        Ok(())
    }
}

#[test]
fn dt_preparation_physical_presence_uses_nested_hulls_and_current_member_rooms() -> TestResult {
    let mut fixture = Fixture::new()?;
    check!(physically_aboard(
        &fixture.world,
        fixture.unit,
        fixture.carrier
    ));
    check!(physically_aboard(
        &fixture.world,
        fixture.actor,
        fixture.carrier
    ));
    check!(ready_station(&fixture.world, fixture.actor, &fixture.draft).is_some());
    fixture.world.move_to(fixture.actor, fixture.side_room)?;
    check!(physically_aboard(
        &fixture.world,
        fixture.actor,
        fixture.carrier
    ));
    check!(
        ready_station(&fixture.world, fixture.actor, &fixture.draft).is_none(),
        "a member room is aboard but is not the helm console"
    );
    fixture.world.move_to(fixture.actor, LIMBO)?;
    check!(!physically_aboard(
        &fixture.world,
        fixture.actor,
        fixture.carrier
    ));
    let (unit, carrier) = (fixture.unit, fixture.carrier);
    fixture.focus(|focus| {
        focus.reach = ReachProof::RemoteLink {
            terminal: unit,
            path: carrier,
        }
    })?;
    check!(
        !physically_aboard(&fixture.world, fixture.actor, fixture.carrier),
        "a live remote console is not physical personnel presence"
    );
    check!(ready_station(&fixture.world, fixture.actor, &fixture.draft).is_none());
    Ok(())
}

#[test]
fn dt_preparation_ready_rechecks_billet_narrowing_focus_and_control_generation() -> TestResult {
    let mut fixture = Fixture::new()?;
    let before =
        ready_station(&fixture.world, fixture.actor, &fixture.draft).ok_or("initial seat")?;
    check_eq!(before.unit(), fixture.unit);
    let unit = fixture.unit;
    fixture.spatial(|state| {
        state.control_gen.insert(unit, 6);
    })?;
    let after =
        ready_station(&fixture.world, fixture.actor, &fixture.draft).ok_or("seat after bump")?;
    check!(
        before != after,
        "the final preimage changes even while physical seat remains valid"
    );
    let station = fixture.station.clone();
    fixture.spatial(|state| {
        if let Some(assignment) = state
            .assignments
            .get_mut(&unit)
            .and_then(|host| host.seats.get_mut(&station))
        {
            assignment.granted = Some(BTreeSet::new());
        }
    })?;
    check!(ready_station(&fixture.world, fixture.actor, &fixture.draft).is_none());
    fixture.spatial(|state| {
        if let Some(assignment) = state
            .assignments
            .get_mut(&unit)
            .and_then(|host| host.seats.get_mut(&station))
        {
            assignment.granted = None;
        }
    })?;
    fixture.focus(|focus| focus.proxy = true)?;
    check!(ready_station(&fixture.world, fixture.actor, &fixture.draft).is_none());
    fixture.focus(|focus| {
        focus.proxy = false;
        focus.driver = GOD;
    })?;
    check!(ready_station(&fixture.world, fixture.actor, &fixture.draft).is_none());
    fixture.focus(|focus| {
        focus.driver = focus.actor;
        focus.reach = ReachProof::RemoteLink {
            terminal: unit,
            path: unit,
        };
    })?;
    check!(ready_station(&fixture.world, fixture.actor, &fixture.draft).is_none());
    Ok(())
}

#[test]
fn dt_preparation_stale_placement_cycles_and_missing_seats_fail_closed() -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture.world.move_to(fixture.unit, LIMBO)?;
    check_eq!(
        presence(&fixture.world, fixture.unit, fixture.carrier),
        Presence::Unavailable
    );
    check_eq!(
        presence(&fixture.world, fixture.actor, fixture.carrier),
        Presence::Unavailable
    );
    check!(ready_station(&fixture.world, fixture.actor, &fixture.draft).is_none());
    fixture.world.move_to(fixture.unit, fixture.bay)?;
    let unit = fixture.unit;
    fixture.spatial(|state| {
        state.assignments.remove(&unit);
    })?;
    check!(ready_station(&fixture.world, fixture.actor, &fixture.draft).is_none());
    let bay = fixture.bay;
    fixture.spatial(|state| {
        if let Some(space) = state.spaces.get_mut(&bay) {
            space.anchor = SpaceAnchor::MovingObject { host: unit };
        }
    })?;
    check_eq!(
        presence(&fixture.world, fixture.actor, fixture.carrier),
        Presence::Unavailable
    );
    Ok(())
}

#[test]
fn dt_preparation_reports_only_the_viewers_permissions_and_keeps_activation_unresolved()
-> TestResult {
    let mut fixture = Fixture::new()?;
    fixture.draft.consents.insert(
        fixture.actor,
        Consent {
            actor: fixture.actor,
            revision: fixture.draft.revision,
            plan_hash: fixture.draft.plan_hash.clone(),
            state: ConsentState::Ready,
        },
    );
    let mut queried = Vec::new();
    let lines = report_lines(
        &fixture.world,
        GOD,
        &fixture.draft,
        |_| true,
        |unit, capability| {
            queried.push((unit, capability));
            Permission::Allowed
        },
    )?;
    check_eq!(queried, vec![(fixture.unit, UnitCapability::Deploy)]);
    let rendered = lines.join("\n");
    check!(
        !rendered.contains("Private"),
        "private object names are never looked up"
    );
    check!(rendered.contains("ready declared for this exact revision"));
    check!(rendered.contains("current carrier containment"));
    check!(!rendered.contains("last-port departure personnel proof"));
    check!(rendered.contains("NPC contracts"));
    check!(rendered.contains("mind licences"));
    check!(rendered.contains("unit reservations"));
    check!(rendered.contains("This is not launch admission"));
    queried.clear();
    let allowed = report_lines(
        &fixture.world,
        fixture.actor,
        &fixture.draft,
        |_| true,
        |unit, capability| {
            queried.push((unit, capability));
            Permission::Allowed
        },
    )?
    .join("\n");
    check_eq!(queried, vec![(fixture.unit, UnitCapability::OperateCrew)]);
    check!(allowed.contains("allowed now"));
    let denied = report_lines(
        &fixture.world,
        fixture.actor,
        &fixture.draft,
        |_| true,
        |_, _| Permission::Denied,
    )?
    .join("\n");
    check!(
        denied.contains("denied now"),
        "permission observations are recomputed on every read"
    );
    let unavailable = report_lines(
        &fixture.world,
        fixture.actor,
        &fixture.draft,
        |_| true,
        |_, _| Permission::Unavailable,
    )?
    .join("\n");
    check!(unavailable.contains("authority service unavailable"));
    let stranger = fixture
        .world
        .create(ObjKind::Player, "Stranger", GOD, Some(LIMBO))?;
    check!(
        report_lines(
            &fixture.world,
            stranger,
            &fixture.draft,
            |_| true,
            |_, _| Permission::Allowed
        )
        .is_err()
    );
    // A declaration is retained as a declaration after leaving the station;
    // neither this report nor the stored consent upgrades it to live admission.
    fixture.world.move_to(fixture.actor, fixture.side_room)?;
    let left = report_lines(
        &fixture.world,
        fixture.actor,
        &fixture.draft,
        |_| true,
        |_, _| Permission::Allowed,
    )?
    .join("\n");
    check!(left.contains("ready declared for this exact revision"));
    check!(left.contains("Physical helm station aboard this carrier: not established"));
    Ok(())
}

#[test]
fn dt_preparation_readiness_requires_the_selected_carrier() -> TestResult {
    let mut fixture = Fixture::new()?;
    let other = fixture
        .world
        .create(ObjKind::Thing, "Other carrier", GOD, Some(LIMBO))?;
    fixture.draft.plan.carrier = other;
    fixture.draft.plan_hash = fixture.draft.plan.material_hash()?;
    check!(physical_station(&fixture.world, fixture.actor, fixture.unit).is_some());
    check!(ready_station(&fixture.world, fixture.actor, &fixture.draft).is_none());
    check_eq!(
        fixture
            .world
            .get(fixture.actor)
            .and_then(|actor| actor.location),
        Some(fixture.console)
    );
    Ok(())
}

#[test]
fn dt_preparation_missing_and_inaccessible_references_have_the_same_observation() -> TestResult {
    let mut fixture = Fixture::new()?;
    let actor = fixture.actor;
    check!(
        !observation_allowed(&fixture.world, actor, fixture.carrier, true),
        "a known carrier ID is not a global observation right"
    );
    check!(!observation_allowed(
        &fixture.world,
        actor,
        fixture.unit,
        true
    ));
    check!(!observation_allowed(
        &fixture.world,
        actor,
        ObjId(u32::MAX),
        true
    ));
    check!(
        observation_allowed(&fixture.world, actor, actor, false),
        "self is observable"
    );
    check!(
        !observation_allowed(&fixture.world, GOD, fixture.unit, false),
        "perception concealment is not bypassed by examine authority"
    );
    let distant = report_lines(
        &fixture.world,
        actor,
        &fixture.draft,
        |object| observation_allowed(&fixture.world, actor, object, true),
        |_, _| Permission::Denied,
    )?
    .join("\n");
    fixture
        .world
        .module_state_insert(mush_tactics::module::UnitStore::default());
    check!(
        ready_station(&fixture.world, actor, &fixture.draft).is_none(),
        "a Thing with a station is not necessarily a tactical unit"
    );
    let absent_unit = report_lines(
        &fixture.world,
        actor,
        &fixture.draft,
        |object| observation_allowed(&fixture.world, actor, object, true),
        |_, _| Permission::Denied,
    )?
    .join("\n");
    check_eq!(
        distant,
        absent_unit,
        "hidden unit presence and absent tactical record render identically"
    );
    fixture.world.destroy(fixture.unit)?;
    let missing = report_lines(
        &fixture.world,
        actor,
        &fixture.draft,
        |object| observation_allowed(&fixture.world, actor, object, true),
        |_, _| Permission::Denied,
    )?
    .join("\n");
    check_eq!(
        distant,
        missing,
        "a missing object is indistinguishable from an inaccessible one"
    );
    check!(distant.contains(UNOBSERVED));
    check!(!distant.contains("physically aboard"));
    Ok(())
}

#[test]
fn six_npc_slots_resolve_current_nested_containment_without_boarding_records() -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture.draft.plan.slots.clear();
    let mut contracts = BTreeMap::new();
    for ordinal in 1..=6 {
        let npc = fixture.world.create(
            ObjKind::Npc,
            &format!("Crew {ordinal}"),
            GOD,
            Some(fixture.side_room),
        )?;
        contracts.insert(
            npc,
            employment(&fixture, npc, NpcMissionScope::SubordinateOperator)?,
        );
        fixture.draft.plan.slots.push(SlotPlan {
            slot: Slot::new(ordinal)?,
            unit: None,
            controller: PlannedController::Npc { character: npc },
            role: None,
            mind: None,
        });
    }
    fixture.draft.plan_hash = fixture.draft.plan.material_hash()?;
    let rows = personnel::resolve(&fixture.world, &fixture.draft, |npc| {
        Ok(contracts.get(&npc).cloned())
    })?;
    check_eq!(rows.len(), 6);
    check!(rows.iter().all(|row| row.terms.is_ok()));
    let walker = rows.first().ok_or("first NPC")?.character;
    fixture.world.move_to(walker, LIMBO)?;
    let away = personnel::resolve(&fixture.world, &fixture.draft, |npc| {
        Ok(contracts.get(&npc).cloned())
    })?;
    check_eq!(
        away.first().ok_or("away NPC")?.terms,
        Err(PersonnelIssue::NotAboard)
    );
    fixture.world.move_to(walker, fixture.console)?;
    fixture.world.set_now(1_000_000);
    let returned = personnel::resolve(&fixture.world, &fixture.draft, |npc| {
        Ok(contracts.get(&npc).cloned())
    })?;
    check!(returned.iter().all(|row| row.terms.is_ok()));
    check_eq!(
        rows,
        returned,
        "ordinary departure/return and time passage need no crossing or wage evidence"
    );
    Ok(())
}

#[test]
fn npc_personnel_distinguishes_employment_command_scope_and_missing_assignment() -> TestResult {
    let mut fixture = Fixture::new()?;
    let npc = fixture
        .world
        .create(ObjKind::Npc, "Crew", GOD, Some(fixture.bay))?;
    fixture
        .draft
        .plan
        .slots
        .first_mut()
        .ok_or("slot")?
        .controller = PlannedController::Npc { character: npc };
    fixture.draft.plan.npc_commander = Some(npc);
    fixture.draft.plan_hash = fixture.draft.plan.material_hash()?;
    let mut contract = employment(&fixture, npc, NpcMissionScope::SubordinateOperator)?;
    let rows = personnel::resolve(&fixture.world, &fixture.draft, |_| {
        Ok(Some(contract.clone()))
    })?;
    check_eq!(
        rows.first().ok_or("commander")?.terms,
        Err(PersonnelIssue::CommandOutsideContract)
    );
    contract.terms.mission_scope = NpcMissionScope::DetachmentCommander;
    contract.terms_hash = contract.terms.digest()?;
    let allowed = personnel::resolve(&fixture.world, &fixture.draft, |_| {
        Ok(Some(contract.clone()))
    })?;
    check!(allowed.first().ok_or("commander")?.terms.is_ok());
    contract.employer = fixture.actor;
    let other = personnel::resolve(&fixture.world, &fixture.draft, |_| {
        Ok(Some(contract.clone()))
    })?;
    check_eq!(
        other.first().ok_or("other employer")?.terms,
        Err(PersonnelIssue::NotContracted)
    );
    let missing = personnel::resolve(&fixture.world, &fixture.draft, |_| Ok(None))?;
    check_eq!(
        other,
        missing,
        "another employer's private contract is not disclosed"
    );
    fixture.draft.plan.slots.clear();
    fixture.draft.plan_hash = fixture.draft.plan.material_hash()?;
    let unassigned = personnel::resolve(&fixture.world, &fixture.draft, |_| {
        Err("must not read an unassigned commander's contract".into())
    })?;
    check_eq!(
        unassigned.first().ok_or("unassigned commander")?.terms,
        Err(PersonnelIssue::CommanderUnassigned)
    );
    Ok(())
}

#[test]
fn npc_personnel_refuses_wrong_character_kind_and_corrupt_contract() -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture
        .draft
        .plan
        .slots
        .first_mut()
        .ok_or("slot")?
        .controller = PlannedController::Npc {
        character: fixture.actor,
    };
    fixture.draft.plan_hash = fixture.draft.plan.material_hash()?;
    let wrong_kind = employment(
        &fixture,
        fixture.actor,
        NpcMissionScope::SubordinateOperator,
    )?;
    let rows = personnel::resolve(&fixture.world, &fixture.draft, |_| {
        Ok(Some(wrong_kind.clone()))
    })?;
    check_eq!(
        rows.first().ok_or("wrong kind")?.terms,
        Err(PersonnelIssue::Unavailable)
    );
    let npc = fixture
        .world
        .create(ObjKind::Npc, "Crew", GOD, Some(fixture.bay))?;
    fixture
        .draft
        .plan
        .slots
        .first_mut()
        .ok_or("slot")?
        .controller = PlannedController::Npc { character: npc };
    fixture.draft.plan_hash = fixture.draft.plan.material_hash()?;
    let mut contract = employment(&fixture, npc, NpcMissionScope::SubordinateOperator)?;
    contract.npc = fixture.actor;
    let rows = personnel::resolve(&fixture.world, &fixture.draft, |_| {
        Ok(Some(contract.clone()))
    })?;
    check_eq!(
        rows.first().ok_or("wrong contract identity")?.terms,
        Err(PersonnelIssue::Unavailable)
    );
    let absent = ObjId(u32::MAX);
    for person in [npc, absent] {
        fixture
            .draft
            .plan
            .slots
            .first_mut()
            .ok_or("slot")?
            .controller = PlannedController::Npc { character: person };
        fixture.draft.plan_hash = fixture.draft.plan.material_hash()?;
        let rows = personnel::resolve(&fixture.world, &fixture.draft, |_| Ok(None))?;
        check_eq!(
            rows.first().ok_or("uncontracted person")?.terms,
            Err(PersonnelIssue::NotContracted),
            "an uncontracted ID is not an existence or location oracle"
        );
    }
    Ok(())
}
