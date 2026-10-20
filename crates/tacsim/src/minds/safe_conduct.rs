//! Independent house source probes. These are executable public-observation
//! checks, not a Registry certificate, paid licence, cargo host or cover proof.
use super::{RhaiRunner, fixture};
use crate::arena::{
    conditions::{Arming, SlotTarget},
    core::{ClassTable, CoreClass},
    intents::{Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal, apply_intent},
    mind::{MindRunner, MindView, ThinkJob, ThinkResult},
    motor::MotorCounters,
    perception::{DriveOutcome, FrameOrder, MindObjective},
    profile_rules::BoundProgram,
    profile_runtime::ProfileToken,
    runtime_card::{CardCatalog, CardDefinition, ProfileDefinition, ProfileKey, RuntimeCard},
    state::{Channel, Classification, ContactId, Controller, Power, Team},
    test_support::{arena_painted, ground, line_tank, pose},
};
use crate::config::ArenaConfig;
use mush_roll::{Envelope, RevisionAdapter};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const SOURCE: &str = include_str!("../../content/minds/house/safe_conduct.rhai");
const ENVELOPE: &str = include_str!("../../content/minds/house/safe_conduct.envelope.json");
const MANIFEST: &str = include_str!("../../content/minds/house/safe_conduct.manifest.json");

struct Harness {
    runner: RhaiRunner,
    card: RuntimeCard,
    program: BoundProgram,
    map: Arc<crate::arena::state::ArenaDefinition>,
}
impl Harness {
    fn new(role: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let envelope: Envelope = serde_json::from_str(ENVELOPE)?;
        let checked = crate::arena::schema::validate_envelope(
            &envelope.schema,
            &envelope.presets,
            &envelope.profiles,
            &envelope.rules,
            CoreClass::II,
        )?;
        let definition = CardDefinition {
            schema: checked.schema,
            declared_class: CoreClass::II,
            profiles: checked
                .profiles
                .into_iter()
                .map(|(name, values)| ProfileDefinition {
                    name,
                    values,
                    rules: vec![],
                })
                .collect(),
        };
        let index = definition
            .profiles
            .iter()
            .position(|profile| profile.name == role)
            .ok_or("role")?;
        let mut catalog = CardCatalog::default();
        let definition = catalog.admit(definition)?;
        let card = catalog.resolve(ProfileKey {
            definition,
            index: u8::try_from(index)?,
        })?;
        let program = BoundProgram::bind(
            &[],
            &[],
            &[Channel::Helm, Channel::Sensors, Channel::Fire],
            &ClassTable::STOCK.row(CoreClass::II),
            100,
        )?;
        let runner = RhaiRunner::on(
            Arc::new(mush_mind::Sandbox::new(
                &mush_mind::EngineLimits::default(),
                super::register_api,
            )),
            &ArenaConfig::default(),
            SOURCE,
            fixture::digest(SOURCE),
        )
        .map_err(|error| format!("source compile: {error:?}"))?
        .recording();
        Ok(Self {
            runner,
            card,
            program,
            map: Arc::new(fixture::map()),
        })
    }
    fn think(
        &self,
        view: MindView,
        memory: &[u8],
        sequence: u64,
    ) -> Result<ThinkResult, Box<dyn std::error::Error>> {
        let now = view.t_ms;
        let result = self.runner.think(&ThinkJob {
            unit: fixture::UNIT,
            think_seq: sequence,
            t_ms: now,
            lands_ms: now + 100,
            seed: 2350,
            dropped: false,
            profile: &ClassTable::STOCK.row(CoreClass::II),
            view: &Arc::new(view),
            memory,
            map: &self.map,
            card: &self.card,
            program: &self.program,
            profile_token: ProfileToken {
                unit: fixture::UNIT,
                generation: 0,
                key: self.card.key(),
            },
        });
        let measured = self.runner.take_stats().pop().ok_or("actual measurement")?;
        assert!(
            result.fault.is_none(),
            "source fault {:?}; operations={} fees={}",
            result.fault,
            measured.ops,
            measured.fee
        );
        assert!(measured.ops + measured.fee <= 3000);
        assert!(result.intents.len() <= 16);
        assert!(
            mush_mind::memory::decode(&result.memory)
                .ok_or("canonical flat memory")?
                .len()
                <= 4
        );
        Ok(result)
    }
}

fn order(objective: MindObjective) -> FrameOrder {
    FrameOrder {
        squad: "red-1".into(),
        objective,
        hold_fire: false,
        max_range_band: None,
        expires_ms: 60_000,
    }
}
fn healthy(objective: MindObjective) -> MindView {
    let mut view = fixture::view();
    view.notices.clear();
    view.rules.clear();
    view.motor = None;
    view.order = Some(order(objective));
    view.lock = None;
    view.held = false;
    view.power = Power::Online;
    view.speed_cms = 0;
    for section in &mut view.sections {
        if let Some((_, max)) = section.internals {
            section.internals = Some((max, max));
        }
        for facet in &mut section.facets {
            facet.armor = facet.armor_max;
        }
    }
    for weapon in &mut view.weapons {
        weapon.armed = None;
    }
    view.contacts
        .retain(|row| row.id == fixture::FRIEND || row.id == fixture::HOSTILE);
    for row in &mut view.contacts {
        row.age_ms = 0;
        row.reads_dead = Some(false);
        if row.id == fixture::FRIEND {
            row.range_m = 20;
            row.cell = Some((3, 4));
        } else {
            row.range_m = 50;
            row.cell = Some((6, 4));
        }
    }
    view
}
fn support() -> MindView {
    healthy(MindObjective::Support {
        of: Some(fixture::FRIEND),
    })
}
fn transit() -> MindView {
    let mut view = healthy(MindObjective::Advance { to: (6, 4) });
    view.weapons.clear();
    view.contacts.clear();
    view
}
fn has_goal(result: &ThinkResult, wanted: MotorGoal) -> bool {
    result.intents.contains(&IntentKind::Goal(wanted))
}
fn arms(result: &ThinkResult) -> bool {
    result.intents.iter().any(|kind| {
        matches!(
            kind,
            IntentKind::Condition(Arming::Arm(_)) | IntentKind::Fire { .. }
        )
    })
}

#[test]
fn exact_safe_conduct_content_is_distinct_and_validated_without_certification() -> TestResult {
    let manifest: serde_json::Value = serde_json::from_str(MANIFEST)?;
    assert_eq!(
        manifest.get("source_sha256").and_then(|v| v.as_str()),
        Some(crate::arena::digest_text(&Sha256::digest(SOURCE.as_bytes()).into()).as_str())
    );
    assert_eq!(
        manifest.get("content_version").and_then(|v| v.as_u64()),
        Some(2)
    );
    assert_eq!(
        manifest.get("envelope_sha256").and_then(|v| v.as_str()),
        Some(crate::arena::digest_text(&Sha256::digest(ENVELOPE.as_bytes()).into()).as_str())
    );
    assert_eq!(
        manifest.get("status").and_then(|v| v.as_str()),
        Some("unfiled_role_content")
    );
    let envelope: Envelope = serde_json::from_str(ENVELOPE)?;
    super::authoring::RevisionRules.validate(SOURCE, &envelope)?;
    assert_eq!(envelope.declared_class, "II");
    assert_eq!(envelope.mind_abi, super::MIND_ABI);
    assert_eq!(envelope.profiles.as_object().ok_or("profiles")?.len(), 2);
    for forbidden in [
        "stock::think",
        "stock::hunt",
        "import ",
        "eval(",
        "order(",
        "cargo",
        "World",
    ] {
        assert!(
            !SOURCE.contains(forbidden),
            "unexpected source surface {forbidden}"
        );
    }
    assert!(
        SOURCE != super::STANDARD_RHAI,
        "independent authored source"
    );
    for role in ["Escort", "Transit"] {
        let _ = Harness::new(role)?;
    }
    Ok(())
}

#[test]
fn escort_and_transit_apply_different_current_public_orders() -> TestResult {
    let escort = Harness::new("Escort")?;
    let carrier = Harness::new("Transit")?;
    let defended = escort.think(support(), &[], 0)?;
    assert!(has_goal(&defended, MotorGoal::Hold));
    assert!(defended.intents.iter().any(|kind| matches!(kind,
        IntentKind::Condition(Arming::Arm(rule)) if rule.target == SlotTarget::Contact { id: fixture::HOSTILE })));
    assert!(defended.intents.contains(&IntentKind::Lock {
        contact: fixture::HOSTILE
    }));
    let waiting = carrier.think(support(), &[], 0)?;
    assert!(has_goal(&waiting, MotorGoal::Hold));
    assert!(!arms(&waiting));
    let moving = carrier.think(transit(), &[], 1)?;
    assert!(has_goal(&moving, MotorGoal::Cell { x: 6, y: 4 }));
    assert!(
        moving
            .intents
            .iter()
            .all(|kind| kind.channel_required() != Channel::Fire)
    );
    let waiting = escort.think(transit(), &[], 1)?;
    assert!(has_goal(&waiting, MotorGoal::Hold));
    assert!(!arms(&waiting));
    Ok(())
}

#[test]
fn escort_refuses_missing_stale_foreign_or_unidentified_charge_plots() -> TestResult {
    let harness = Harness::new("Escort")?;
    for mode in 0..7 {
        let mut view = support();
        match mode {
            0 => view.contacts.retain(|row| row.id != fixture::FRIEND),
            1 => {
                view.contacts
                    .iter_mut()
                    .find(|r| r.id == fixture::FRIEND)
                    .ok_or("charge")?
                    .age_ms = 3001
            }
            2 => {
                view.contacts
                    .iter_mut()
                    .find(|r| r.id == fixture::FRIEND)
                    .ok_or("charge")?
                    .cell = None
            }
            3 => {
                view.contacts
                    .iter_mut()
                    .find(|r| r.id == fixture::FRIEND)
                    .ok_or("charge")?
                    .class = Classification::Unknown
            }
            4 => {
                view.contacts
                    .iter_mut()
                    .find(|r| r.id == fixture::FRIEND)
                    .ok_or("charge")?
                    .friendly = false
            }
            5 => {
                view.contacts
                    .iter_mut()
                    .find(|r| r.id == fixture::FRIEND)
                    .ok_or("charge")?
                    .reads_dead = Some(true)
            }
            _ => {
                view.order = Some(order(MindObjective::Support {
                    of: Some(ContactId(999)),
                }))
            }
        }
        let result = harness.think(view, &[], mode)?;
        assert!(has_goal(&result, MotorGoal::Hold), "charge mode {mode}");
        assert!(!arms(&result), "charge mode {mode}");
    }
    Ok(())
}

#[test]
fn escort_leash_precedes_engagement_and_never_chases_the_hostile() -> TestResult {
    let mut harness = Harness::new("Escort")?;
    // The public map and earned plot agree: 200 m from (1,4) to (21,4).
    Arc::make_mut(&mut harness.map).terrain =
        crate::arena::state::ArenaTerrain::uniform(64, 16, 1000, ground())?;
    let mut view = support();
    let row = view
        .contacts
        .iter_mut()
        .find(|r| r.id == fixture::FRIEND)
        .ok_or("charge")?;
    row.range_m = 200;
    row.cell = Some((21, 4));
    let expected = crate::arena::stock_mind::stand_off_of(
        &harness.map,
        &view,
        view.contacts
            .iter()
            .find(|r| r.id == fixture::FRIEND)
            .ok_or("charge")?,
        3000,
    )
    .ok_or("public station")?;
    let result = harness.think(view, &[], 0)?;
    assert!(has_goal(
        &result,
        MotorGoal::Cell {
            x: expected.0,
            y: expected.1
        }
    ));
    assert!(!arms(&result));
    assert!(
        !result
            .intents
            .iter()
            .any(|kind| matches!(kind, IntentKind::Lock { .. }))
    );
    let mut view = support();
    let hostile = view
        .contacts
        .iter_mut()
        .find(|r| r.id == fixture::HOSTILE)
        .ok_or("hostile")?;
    hostile.cell = Some((7, 0));
    hostile.range_m = 72;
    let result = harness.think(view, &[], 1)?;
    assert!(has_goal(&result, MotorGoal::Hold));
    assert!(!has_goal(&result, MotorGoal::Cell { x: 7, y: 0 }));
    Ok(())
}

#[test]
fn transit_stops_only_for_current_visible_threat_and_resumes_order() -> TestResult {
    let harness = Harness::new("Transit")?;
    let mut view = healthy(MindObjective::Advance { to: (6, 4) });
    view.weapons.clear();
    let paused = harness.think(view.clone(), &[], 0)?;
    assert!(has_goal(&paused, MotorGoal::Hold));
    assert!(!arms(&paused));
    for mode in 0..4 {
        let mut gone = view.clone();
        let hostile = gone
            .contacts
            .iter_mut()
            .find(|r| r.id == fixture::HOSTILE)
            .ok_or("hostile")?;
        match mode {
            0 => hostile.age_ms = 3001,
            1 => hostile.class = Classification::Unknown,
            2 => hostile.friendly = true,
            _ => hostile.range_m = 301,
        }
        let result = harness.think(gone, &paused.memory, mode + 1)?;
        assert!(has_goal(&result, MotorGoal::Cell { x: 6, y: 4 }));
        assert!(
            result
                .intents
                .iter()
                .all(|kind| kind.channel_required() != Channel::Fire)
        );
    }
    Ok(())
}

#[test]
fn absent_expired_orders_and_hold_fire_never_authorize_engagement() -> TestResult {
    for role in ["Escort", "Transit"] {
        let harness = Harness::new(role)?;
        for absent in [false, true] {
            let mut view = if role == "Escort" {
                support()
            } else {
                transit()
            };
            if absent {
                view.order = None;
            } else {
                view.order.as_mut().ok_or("order")?.expires_ms = view.t_ms;
            }
            let result = harness.think(view, &[], 0)?;
            assert!(has_goal(&result, MotorGoal::Hold));
            assert!(!arms(&result));
        }
    }
    let harness = Harness::new("Escort")?;
    let mut view = support();
    view.order.as_mut().ok_or("order")?.hold_fire = true;
    let result = harness.think(view, &[], 0)?;
    assert!(has_goal(&result, MotorGoal::Hold));
    assert!(!arms(&result));
    Ok(())
}

#[test]
fn transit_actual_native_no_route_waits_then_reissues_exact_goal() -> TestResult {
    let mut wall = ground();
    wall.move_cost = 0;
    let paint = [
        (2, 5),
        (3, 5),
        (4, 5),
        (2, 6),
        (4, 6),
        (2, 7),
        (3, 7),
        (4, 7),
    ]
    .map(|(x, y)| (x, y, wall));
    let mut hull = line_tank(fixture::UNIT, Team::Red, pose(3500, 3500));
    hull.controller = Controller::Mind;
    hull.reseal(&crate::match_::SealTerms::fixture())?;
    let mut arena = arena_painted(vec![hull], &paint);
    arena.unit_mut(fixture::UNIT).ok_or("hull")?.power = Power::Online;
    arena.now_ms = 5000;
    let goal = MotorGoal::Cell { x: 3, y: 6 };
    let input = Intent {
        unit: fixture::UNIT,
        channel: Channel::Helm,
        epoch: arena.epochs.current(fixture::UNIT, Channel::Helm),
        kind: IntentKind::Goal(goal),
        seq: 0,
    };
    assert_eq!(
        apply_intent(&mut arena, &input, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    let mut counters = MotorCounters::default();
    arena.motor_step(fixture::UNIT, 5000, &mut counters);
    let mut actual = arena.mind_view(fixture::UNIT).ok_or("actual public view")?;
    assert_eq!(
        actual.motor.ok_or("actual motor")?.outcome,
        Some(DriveOutcome::NoRoute)
    );
    actual.order = Some(order(MindObjective::Advance { to: (3, 6) }));
    let mut harness = Harness::new("Transit")?;
    harness.map = Arc::clone(&arena.definition);
    let stopped = harness.think(actual.clone(), &[], 0)?;
    assert!(has_goal(&stopped, MotorGoal::Hold));
    let hold = Intent {
        kind: IntentKind::Goal(MotorGoal::Hold),
        seq: 1,
        ..input
    };
    assert_eq!(
        apply_intent(&mut arena, &hold, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    arena.now_ms = 6999;
    let mut after_hold = arena.mind_view(fixture::UNIT).ok_or("after hold")?;
    after_hold.order = actual.order.clone();
    let waiting = harness.think(after_hold.clone(), &stopped.memory, 1)?;
    assert!(
        has_goal(&waiting, MotorGoal::Hold),
        "HOLD removing a motor row must not erase retry delay"
    );
    after_hold.t_ms = 7000;
    let retry = harness.think(after_hold, &waiting.memory, 2)?;
    assert!(has_goal(&retry, MotorGoal::Cell { x: 3, y: 6 }));
    assert!(!arms(&retry));

    // Land that retry through the unchanged native route solver. The same
    // enclosed destination really fails again; it must earn a new delay.
    arena.now_ms = 7000;
    let second_attempt = Intent { seq: 2, ..input };
    assert_eq!(
        apply_intent(&mut arena, &second_attempt, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    arena.motor_step(fixture::UNIT, 7000, &mut counters);
    let mut second_failure = arena.mind_view(fixture::UNIT).ok_or("second failure")?;
    assert_eq!(
        second_failure.motor.ok_or("second native motor")?.outcome,
        Some(DriveOutcome::NoRoute)
    );
    second_failure.order = actual.order.clone();
    let second_stop = harness.think(second_failure, &retry.memory, 3)?;
    assert!(has_goal(&second_stop, MotorGoal::Hold));
    let second_hold = Intent { seq: 3, ..hold };
    assert_eq!(
        apply_intent(&mut arena, &second_hold, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    arena.now_ms = 8999;
    let mut second_wait = arena.mind_view(fixture::UNIT).ok_or("second hold")?;
    second_wait.order = actual.order;
    let waiting_again = harness.think(second_wait.clone(), &second_stop.memory, 4)?;
    assert!(has_goal(&waiting_again, MotorGoal::Hold));
    second_wait.t_ms = 9000;
    let retry_again = harness.think(second_wait, &waiting_again.memory, 5)?;
    assert!(has_goal(&retry_again, MotorGoal::Cell { x: 3, y: 6 }));
    assert!(!arms(&retry_again));
    Ok(())
}

#[test]
fn roles_at_full_public_row_bound_keep_real_class_and_memory_limits() -> TestResult {
    for role in ["Escort", "Transit"] {
        let harness = Harness::new(role)?;
        let mut view = support();
        let template = view
            .contacts
            .iter()
            .find(|r| r.id == fixture::HOSTILE)
            .ok_or("hostile")?
            .clone();
        while view.contacts.len() < 16 {
            let mut row = template.clone();
            row.id = ContactId(100 + u64::try_from(view.contacts.len())?);
            view.contacts.push(row);
        }
        let mut memory = Vec::new();
        for sequence in 0..8 {
            view.t_ms += 500;
            memory = harness.think(view.clone(), &memory, sequence)?.memory;
        }
    }
    Ok(())
}

#[test]
fn actual_native_arrival_and_own_hurt_have_explicit_role_responses() -> TestResult {
    let mut hull = line_tank(fixture::UNIT, Team::Red, pose(3500, 3500));
    hull.controller = Controller::Mind;
    hull.reseal(&crate::match_::SealTerms::fixture())?;
    let mut arena = arena_painted(vec![hull], &[]);
    arena.unit_mut(fixture::UNIT).ok_or("hull")?.power = Power::Online;
    arena.now_ms = 5000;
    let input = Intent {
        unit: fixture::UNIT,
        channel: Channel::Helm,
        epoch: arena.epochs.current(fixture::UNIT, Channel::Helm),
        kind: IntentKind::Goal(MotorGoal::Cell { x: 3, y: 3 }),
        seq: 0,
    };
    assert_eq!(
        apply_intent(&mut arena, &input, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    arena.motor_step(fixture::UNIT, 5000, &mut MotorCounters::default());
    let mut view = arena
        .mind_view(fixture::UNIT)
        .ok_or("actual public arrival")?;
    assert_eq!(
        view.motor.ok_or("motor")?.outcome,
        Some(DriveOutcome::Arrived)
    );
    view.order = Some(order(MindObjective::Advance { to: (3, 3) }));
    let harness = Harness::new("Transit")?;
    let result = harness.think(view, &[], 0)?;
    assert!(has_goal(&result, MotorGoal::Hold));
    assert!(!arms(&result));
    for role in ["Escort", "Transit"] {
        let harness = Harness::new(role)?;
        let mut hurt = support();
        hurt.cell = Some((3, 4));
        hurt.x_cm = 3500;
        for section in &mut hurt.sections {
            if let Some((_, maximum)) = section.internals {
                section.internals = Some((0, maximum));
            }
            for facet in &mut section.facets {
                facet.armor = 0;
            }
        }
        // The original two-cell fixture zone cannot contain the motor's
        // complete arrival square. No boundary goal or firing is permitted.
        let narrow = harness.think(hurt.clone(), &[], 0)?;
        assert!(has_goal(&narrow, MotorGoal::Hold));
        assert!(!arms(&narrow));
        // Keep the positive own-hurt response on the minimum three-cell zone.
        // Actual Conduct scenario terrain and physical-entry acceptance stay
        // unchanged; this is only the small source unit probe's own map.
        let mut harness = harness;
        Arc::make_mut(&mut harness.map)
            .entry_zones
            .iter_mut()
            .find(|zone| zone.team == Team::Red)
            .ok_or("own zone")?
            .x1 = 2;
        let result = harness.think(hurt, &[], 0)?;
        assert!(has_goal(&result, MotorGoal::Cell { x: 1, y: 4 }));
        assert!(!arms(&result));
    }
    Ok(())
}
