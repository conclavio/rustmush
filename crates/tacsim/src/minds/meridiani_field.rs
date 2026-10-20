//! Independent house source probes. These are executable public-observation
//! checks, not a Registry certificate, paid licence, cargo host or cover proof.
use super::{RhaiRunner, fixture};
use crate::arena::{
    conditions::{Arming, FireCondition, SlotTarget},
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
const SOURCE: &str = include_str!("../../content/minds/house/meridiani_field.rhai");
const ENVELOPE: &str = include_str!("../../content/minds/house/meridiani_field.envelope.json");
const MANIFEST: &str = include_str!("../../content/minds/house/meridiani_field.manifest.json");

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
                <= 3
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

fn engaged() -> MindView {
    healthy(MindObjective::Engage {
        target: Some(fixture::HOSTILE),
    })
}

fn conditions(result: &ThinkResult) -> Vec<&FireCondition> {
    result
        .intents
        .iter()
        .filter_map(|intent| match intent {
            IntentKind::Condition(Arming::Arm(condition)) => Some(condition),
            _ => None,
        })
        .collect()
}

#[test]
fn meridiani_source_and_profiles_have_exact_unfiled_content_identity() -> TestResult {
    let manifest: serde_json::Value = serde_json::from_str(MANIFEST)?;
    for (key, bytes) in [
        ("source_sha256", SOURCE.as_bytes()),
        ("envelope_sha256", ENVELOPE.as_bytes()),
    ] {
        assert_eq!(
            manifest.get(key).and_then(|v| v.as_str()),
            Some(crate::arena::digest_text(&Sha256::digest(bytes).into()).as_str())
        );
    }
    assert_eq!(
        manifest.get("content_version").and_then(|v| v.as_u64()),
        Some(2)
    );
    assert_eq!(
        manifest.get("status").and_then(|v| v.as_str()),
        Some("unfiled_combat_content")
    );
    assert_eq!(
        manifest
            .get("intended_legal_class")
            .and_then(|v| v.as_str()),
        Some("discretion")
    );
    let envelope: Envelope = serde_json::from_str(ENVELOPE)?;
    super::authoring::RevisionRules.validate(SOURCE, &envelope)?;
    assert_eq!(envelope.declared_class, "II");
    assert_eq!(envelope.mind_abi, super::MIND_ABI);
    assert_eq!(envelope.profiles.as_object().ok_or("profiles")?.len(), 2);
    assert_eq!(envelope.presets, envelope.profiles);
    for forbidden in [
        "stock::think",
        "stock::hunt",
        "import ",
        "eval(",
        "order(",
        "World",
        "ObjId",
    ] {
        assert!(
            !SOURCE.contains(forbidden),
            "unexpected content surface {forbidden}"
        );
    }
    assert_ne!(
        Sha256::digest(SOURCE.as_bytes()),
        Sha256::digest(super::STANDARD_RHAI.as_bytes())
    );
    assert_ne!(
        Sha256::digest(SOURCE.as_bytes()),
        Sha256::digest(include_bytes!(
            "../../content/minds/house/safe_conduct.rhai"
        ))
    );
    for name in ["Field", "Reserve"] {
        let _ = Harness::new(name)?;
    }
    Ok(())
}

#[test]
fn field_and_reserve_emit_distinct_real_condition_terms_and_obey_actual_floor() -> TestResult {
    for (name, odds, kept) in [("Field", 400, 1), ("Reserve", 650, 3)] {
        let harness = Harness::new(name)?;
        let result = harness.think(engaged(), &[], 0)?;
        let rules = conditions(&result);
        assert!(!rules.is_empty());
        for arm in rules {
            assert_eq!(
                arm.target,
                SlotTarget::Contact {
                    id: fixture::HOSTILE
                }
            );
            assert_eq!(arm.min_permille, odds);
            assert_eq!(arm.min_rounds_left, kept);
            assert!(arm.max_range_m > 0);
        }
        let mut lawful = engaged();
        lawful.odds_floor_permille = 700;
        let result = harness.think(lawful, &[], 1)?;
        assert!(!conditions(&result).is_empty());
        assert!(
            conditions(&result)
                .iter()
                .all(|rule| rule.min_permille == 700)
        );
        let mut conserving = engaged();
        for weapon in &mut conserving.weapons {
            weapon.rounds = kept;
        }
        assert!(!arms(&harness.think(conserving, &[], 2)?));
    }
    Ok(())
}

#[test]
fn engage_requires_its_exact_fresh_earned_hostile_without_substitution() -> TestResult {
    let harness = Harness::new("Field")?;
    for mode in 0..8 {
        let mut view = engaged();
        // A second lawful hostile remains visible in every refusal case.
        let mut other = view
            .contacts
            .iter()
            .find(|row| row.id == fixture::HOSTILE)
            .ok_or("target")?
            .clone();
        other.id = ContactId(999);
        view.contacts.push(other);
        let target = view
            .contacts
            .iter_mut()
            .find(|row| row.id == fixture::HOSTILE)
            .ok_or("target")?;
        match mode {
            0 => target.age_ms = 3001,
            1 => target.age_ms = -1,
            2 => target.class = Classification::Unknown,
            3 => target.friendly = true,
            4 => target.cell = None,
            5 => target.reads_dead = Some(true),
            6 => target.range_m = 501,
            _ => {
                view.order = Some(order(MindObjective::Engage {
                    target: Some(ContactId(998)),
                }))
            }
        }
        let result = harness.think(view, &[], mode)?;
        assert!(has_goal(&result, MotorGoal::Hold));
        assert!(!arms(&result), "ordered target refusal {mode}");
        assert!(
            !result
                .intents
                .iter()
                .any(|intent| matches!(intent, IntentKind::Lock { .. }))
        );
    }
    Ok(())
}

#[test]
fn no_order_expiry_or_hold_fire_cannot_reuse_previous_arming() -> TestResult {
    let old = fixture::view()
        .weapons
        .into_iter()
        .find_map(|weapon| weapon.armed)
        .ok_or("actual fixture arming")?;
    for name in ["Field", "Reserve"] {
        let harness = Harness::new(name)?;
        for mode in 0..3 {
            let mut view = engaged();
            view.weapons.first_mut().ok_or("weapon")?.armed = Some(old.clone());
            match mode {
                0 => view.order = None,
                1 => view.order.as_mut().ok_or("order")?.expires_ms = view.t_ms,
                _ => view.order.as_mut().ok_or("order")?.hold_fire = true,
            }
            let result = harness.think(view, &[], mode)?;
            assert!(!arms(&result));
            assert!(
                result
                    .intents
                    .contains(&IntentKind::Condition(Arming::Disarm { weapon: None }))
            );
            assert!(has_goal(&result, MotorGoal::Hold));
        }
    }
    Ok(())
}

#[test]
fn advance_hold_screen_and_support_use_only_their_current_public_objective() -> TestResult {
    let harness = Harness::new("Field")?;
    let moving = harness.think(healthy(MindObjective::Advance { to: (6, 4) }), &[], 0)?;
    assert!(has_goal(&moving, MotorGoal::Cell { x: 6, y: 4 }));
    assert!(
        arms(&moving),
        "combat Advance keeps its destination while defending"
    );
    let held = harness.think(healthy(MindObjective::Hold), &[], 1)?;
    assert!(has_goal(&held, MotorGoal::Hold));
    assert!(!arms(&held));
    for (bearing, expected) in [(90, true), (181, false)] {
        let mut screen = healthy(MindObjective::Screen { facing_mdeg: 0 });
        screen
            .contacts
            .iter_mut()
            .find(|row| row.id == fixture::HOSTILE)
            .ok_or("hostile")?
            .bearing_deg = bearing;
        let result = harness.think(screen, &[], 2)?;
        assert!(has_goal(&result, MotorGoal::Hold));
        assert!(result.intents.contains(&IntentKind::Heading { mdeg: 0 }));
        assert_eq!(arms(&result), expected);
    }
    let supported = harness.think(
        healthy(MindObjective::Support {
            of: Some(fixture::FRIEND),
        }),
        &[],
        3,
    )?;
    assert!(has_goal(&supported, MotorGoal::Hold));
    assert!(arms(&supported));
    let mut lost = healthy(MindObjective::Support {
        of: Some(fixture::FRIEND),
    });
    lost.contacts
        .iter_mut()
        .find(|row| row.id == fixture::FRIEND)
        .ok_or("charge")?
        .age_ms = 3001;
    let result = harness.think(lost, &[], 4)?;
    assert!(has_goal(&result, MotorGoal::Hold));
    assert!(!arms(&result));
    Ok(())
}

#[test]
fn support_leash_returns_to_actual_earned_charge_without_hostile_pursuit() -> TestResult {
    let mut harness = Harness::new("Field")?;
    Arc::make_mut(&mut harness.map).terrain =
        crate::arena::state::ArenaTerrain::uniform(64, 16, 1000, ground())?;
    let mut view = healthy(MindObjective::Support {
        of: Some(fixture::FRIEND),
    });
    let charge = view
        .contacts
        .iter_mut()
        .find(|row| row.id == fixture::FRIEND)
        .ok_or("charge")?;
    charge.range_m = 250;
    charge.cell = Some((26, 4));
    let expected = crate::arena::stock_mind::stand_off_of(
        &harness.map,
        &view,
        view.contacts
            .iter()
            .find(|row| row.id == fixture::FRIEND)
            .ok_or("charge")?,
        4000,
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
    Ok(())
}

#[test]
fn reserve_withdraws_earlier_and_unarmed_hulls_never_emit_fire_channel_verbs() -> TestResult {
    let mut hurt = engaged();
    hurt.cell = Some((3, 4));
    hurt.x_cm = 3500;
    for section in &mut hurt.sections {
        if let Some((_, maximum)) = section.internals {
            section.internals = Some((maximum * 2 / 5, maximum));
        }
        for facet in &mut section.facets {
            facet.armor = facet.armor_max * 2 / 5;
        }
    }
    assert!((350..500).contains(&crate::arena::stock_mind::health_permille(&hurt)));
    let field = Harness::new("Field")?.think(hurt.clone(), &[], 0)?;
    assert!(arms(&field));
    let reserve = Harness::new("Reserve")?.think(hurt, &[], 0)?;
    assert!(has_goal(&reserve, MotorGoal::Cell { x: 1, y: 4 }));
    assert!(!arms(&reserve));
    for objective in [
        MindObjective::Advance { to: (6, 4) },
        MindObjective::Engage {
            target: Some(fixture::HOSTILE),
        },
        MindObjective::Withdraw,
    ] {
        let mut view = healthy(objective);
        view.weapons.clear();
        let result = Harness::new("Field")?.think(view, &[], 0)?;
        assert!(
            result
                .intents
                .iter()
                .all(|intent| intent.channel_required() != Channel::Fire)
        );
    }
    Ok(())
}

#[test]
fn full_public_contact_rows_keep_actual_class_and_memory_bounds() -> TestResult {
    for name in ["Field", "Reserve"] {
        let harness = Harness::new(name)?;
        let mut view = healthy(MindObjective::Advance { to: (6, 4) });
        let template = view
            .contacts
            .iter()
            .find(|row| row.id == fixture::HOSTILE)
            .ok_or("hostile")?
            .clone();
        while view.contacts.len() < 16 {
            let mut row = template.clone();
            row.id = ContactId(fixture::HOSTILE.0 + u64::try_from(view.contacts.len())?);
            view.contacts.push(row);
        }
        let mut memory = Vec::new();
        for sequence in 0..8 {
            view.t_ms += 500;
            let result = harness.think(view.clone(), &memory, sequence)?;
            assert!(!conditions(&result).is_empty());
            assert!(conditions(&result).iter().all(|rule| rule.target
                == SlotTarget::Contact {
                    id: fixture::HOSTILE
                }));
            memory = result.memory;
        }
    }
    Ok(())
}

#[test]
fn field_actual_native_no_route_waits_then_reissues_exact_goal() -> TestResult {
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
    let mut harness = Harness::new("Field")?;
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
fn actual_native_arrival_holds_without_new_combat_authority() -> TestResult {
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
    let harness = Harness::new("Field")?;
    let result = harness.think(view, &[], 0)?;
    assert!(has_goal(&result, MotorGoal::Hold));
    assert!(!arms(&result));
    Ok(())
}

#[test]
fn ordered_pursuit_and_firing_keep_the_actual_commanders_range_cap() -> TestResult {
    let mut harness = Harness::new("Field")?;
    Arc::make_mut(&mut harness.map).terrain =
        crate::arena::state::ArenaTerrain::uniform(64, 16, 1000, ground())?;
    let mut view = engaged();
    let target = view
        .contacts
        .iter_mut()
        .find(|row| row.id == fixture::HOSTILE)
        .ok_or("hostile")?;
    target.range_m = 350;
    target.cell = Some((36, 4));
    let ranges = crate::arena::stock_mind::view_ranges(&view, 750, view.order.as_ref());
    assert!(35000 > ranges.engage_cm && 35000 <= ranges.release_cm);
    let expected = crate::arena::stock_mind::stand_off_of(
        &harness.map,
        &view,
        view.contacts
            .iter()
            .find(|row| row.id == fixture::HOSTILE)
            .ok_or("hostile")?,
        ranges.engage_cm,
    )
    .ok_or("public stand-off")?;
    let pursuing = harness.think(view.clone(), &[], 0)?;
    assert!(has_goal(
        &pursuing,
        MotorGoal::Cell {
            x: expected.0,
            y: expected.1
        }
    ));
    assert!(!conditions(&pursuing).is_empty());
    view.order.as_mut().ok_or("order")?.max_range_band =
        Some(crate::deliberation::order::RangeBand::Close);
    let limited = crate::arena::stock_mind::view_ranges(&view, 750, view.order.as_ref());
    assert!(limited.release_cm < 35000);
    let constrained = harness.think(view, &[], 1)?;
    assert!(!arms(&constrained));
    let mut beyond = engaged();
    beyond
        .contacts
        .iter_mut()
        .find(|row| row.id == fixture::HOSTILE)
        .ok_or("hostile")?
        .range_m = 501;
    let refused = harness.think(beyond, &[], 2)?;
    assert!(has_goal(&refused, MotorGoal::Hold));
    assert!(!arms(&refused));
    Ok(())
}

#[test]
fn explicit_withdraw_and_ordered_startup_never_create_a_combat_fallback() -> TestResult {
    for name in ["Field", "Reserve"] {
        let harness = Harness::new(name)?;
        let mut withdrawing = healthy(MindObjective::Withdraw);
        withdrawing.cell = Some((3, 4));
        withdrawing.x_cm = 3500;
        let result = harness.think(withdrawing, &[], 0)?;
        assert!(has_goal(&result, MotorGoal::Cell { x: 1, y: 4 }));
        assert!(!arms(&result));
        let mut cold = engaged();
        cold.power = Power::Off;
        let result = harness.think(cold.clone(), &[], 1)?;
        assert!(result.intents.contains(&IntentKind::Startup));
        assert!(!arms(&result));
        cold.order = None;
        let result = harness.think(cold, &[], 2)?;
        assert!(has_goal(&result, MotorGoal::Hold));
        assert!(!result.intents.contains(&IntentKind::Startup));
        assert!(!arms(&result));
    }
    Ok(())
}
