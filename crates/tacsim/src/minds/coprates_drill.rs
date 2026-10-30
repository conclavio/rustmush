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
    state::{Channel, Classification, ContactId, Controller, Power, PartyId},
    test_support::{arena_painted, ground, line_tank, pose},
};
use crate::config::ArenaConfig;
use mush_roll::{Envelope, RevisionAdapter};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const SOURCE: &str = include_str!("../../content/minds/house/coprates_drill.rhai");
const ENVELOPE: &str = include_str!("../../content/minds/house/coprates_drill.envelope.json");
const MANIFEST: &str = include_str!("../../content/minds/house/coprates_drill.manifest.json");

struct Harness {
    runner: RhaiRunner,
    card: RuntimeCard,
    program: BoundProgram,
    map: Arc<crate::arena::state::ArenaDefinition>,
}
impl Harness {
    fn new(role: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_source(role, SOURCE)
    }
    fn with_source(role: &str, source: &str) -> Result<Self, Box<dyn std::error::Error>> {
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
            source,
            fixture::digest(source),
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
        check!(
            result.fault.is_none(),
            "source fault {:?}; operations={} fees={}",
            result.fault,
            measured.ops,
            measured.fee
        );
        check!(measured.ops + measured.fee <= 3000);
        check!(result.intents.len() <= 16);
        check!(
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
        identity: None,
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

fn ready_target() -> MindView {
    let mut view = engaged();
    view.lock = Some(fixture::HOSTILE);
    view.speed_cms = 0;
    view
}

#[test]
fn coprates_has_exact_independent_source_and_provisional_profiles() -> TestResult {
    let manifest: serde_json::Value = serde_json::from_str(MANIFEST)?;
    for (key, bytes) in [
        ("source_sha256", SOURCE.as_bytes()),
        ("envelope_sha256", ENVELOPE.as_bytes()),
    ] {
        check_eq!(
            manifest.get(key).and_then(|v| v.as_str()),
            Some(crate::arena::digest_text(&Sha256::digest(bytes).into()).as_str())
        );
    }
    check_eq!(
        manifest.get("content_version").and_then(|v| v.as_u64()),
        Some(3)
    );
    check_eq!(
        manifest.get("status").and_then(|v| v.as_str()),
        Some("unfiled_combat_content")
    );
    let envelope: Envelope = serde_json::from_str(ENVELOPE)?;
    super::authoring::RevisionRules.validate(SOURCE, &envelope)?;
    check_eq!(envelope.declared_class, "II");
    check_eq!(envelope.mind_abi, super::MIND_ABI);
    check_eq!(envelope.profiles.as_object().ok_or("profiles")?.len(), 2);
    check_eq!(envelope.presets, envelope.profiles);
    for forbidden in [
        "stock::think",
        "stock::hunt",
        "import ",
        "eval(",
        "order(",
        "World",
        "ObjId",
        "standing(",
        "account(",
    ] {
        check!(
            !SOURCE.contains(forbidden),
            "unexpected content surface {forbidden}"
        );
    }
    check_ne!(
        Sha256::digest(SOURCE.as_bytes()),
        Sha256::digest(super::STANDARD_RHAI.as_bytes())
    );
    check_ne!(
        Sha256::digest(SOURCE.as_bytes()),
        Sha256::digest(include_bytes!(
            "../../content/minds/house/safe_conduct.rhai"
        ))
    );
    for name in ["Drill", "Guard"] {
        let _ = Harness::new(name)?;
    }
    Ok(())
}

#[test]
fn requested_hold_and_lock_are_not_already_observed_stationary_fire() -> TestResult {
    for name in ["Drill", "Guard"] {
        let harness = Harness::new(name)?;
        let mut view = engaged();
        view.speed_cms = 100;
        let locking = harness.think(view.clone(), &[], 0)?;
        check!(has_goal(&locking, MotorGoal::Hold));
        check!(locking.intents.contains(&IntentKind::Lock {
            contact: fixture::HOSTILE
        }));
        check!(!arms(&locking));
        view.lock = Some(fixture::HOSTILE);
        view.t_ms += 500;
        let coasting = harness.think(view.clone(), &locking.memory, 1)?;
        check!(has_goal(&coasting, MotorGoal::Hold));
        check!(
            !arms(&coasting),
            "confirmed lock alone does not prove stopped motion"
        );
        view.speed_cms = -1;
        check!(!arms(&harness.think(view.clone(), &coasting.memory, 2)?));
        view.speed_cms = 0;
        view.t_ms += 500;
        let settled = harness.think(view.clone(), &coasting.memory, 3)?;
        check!(!conditions(&settled).is_empty());
        view.lock = Some(fixture::FRIEND);
        let changed = harness.think(view, &settled.memory, 4)?;
        check!(changed.intents.contains(&IntentKind::Lock {
            contact: fixture::HOSTILE
        }));
        check!(
            !arms(&changed),
            "a different observed lock is not the target"
        );
    }
    Ok(())
}

#[test]
fn drill_and_guard_bind_actual_floor_rounds_and_current_target() -> TestResult {
    for (name, odds, kept) in [("Drill", 500, 2), ("Guard", 700, 4)] {
        let harness = Harness::new(name)?;
        let result = harness.think(ready_target(), &[], 0)?;
        let rules = conditions(&result);
        check!(!rules.is_empty());
        for rule in rules {
            check_eq!(
                rule.target,
                SlotTarget::Contact {
                    id: fixture::HOSTILE
                }
            );
            check_eq!(rule.min_permille, odds);
            check_eq!(rule.min_rounds_left, kept);
            check!(rule.max_range_m > 0);
        }
        let mut lawful = ready_target();
        lawful.odds_floor_permille = 800;
        let result = harness.think(lawful, &[], 1)?;
        check!(!conditions(&result).is_empty());
        check!(
            conditions(&result)
                .iter()
                .all(|rule| rule.min_permille == 800)
        );
        let mut empty = ready_target();
        for weapon in &mut empty.weapons {
            weapon.rounds = kept;
        }
        check!(!arms(&harness.think(empty, &[], 2)?));
    }
    Ok(())
}

#[test]
fn exact_ordered_target_does_not_substitute_a_fresh_other_hostile() -> TestResult {
    for (name, age, range) in [("Drill", 2500, 400), ("Guard", 1500, 250)] {
        let harness = Harness::new(name)?;
        for mode in 0..8 {
            let mut view = ready_target();
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
                0 => target.age_ms = age + 1,
                1 => target.age_ms = -1,
                2 => target.class = Classification::Unknown,
                3 => target.friendly = true,
                4 => target.cell = None,
                5 => target.reads_dead = Some(true),
                6 => target.range_m = range + 1,
                _ => {
                    view.order = Some(order(MindObjective::Engage {
                        target: Some(ContactId(998)),
                    }))
                }
            }
            let result = harness.think(view, &[], mode)?;
            check!(has_goal(&result, MotorGoal::Hold));
            check!(!arms(&result));
            check!(
                !result
                    .intents
                    .iter()
                    .any(|intent| matches!(intent, IntentKind::Lock { .. }))
            );
        }
    }
    Ok(())
}

#[test]
fn movement_drills_and_hold_do_not_fire_even_with_a_ready_hostile_lock() -> TestResult {
    for name in ["Drill", "Guard"] {
        let harness = Harness::new(name)?;
        let mut advance = healthy(MindObjective::Advance { to: (6, 4) });
        advance.lock = Some(fixture::HOSTILE);
        let moving = harness.think(advance, &[], 0)?;
        check!(has_goal(&moving, MotorGoal::Cell { x: 6, y: 4 }));
        check!(!arms(&moving));
        for objective in [
            MindObjective::Hold,
            MindObjective::Support {
                of: Some(fixture::FRIEND),
            },
        ] {
            let mut view = healthy(objective);
            view.lock = Some(fixture::HOSTILE);
            let result = harness.think(view, &[], 1)?;
            check!(has_goal(&result, MotorGoal::Hold));
            check!(!arms(&result));
        }
        let mut withdrawing = healthy(MindObjective::Withdraw);
        withdrawing.cell = Some((3, 4));
        withdrawing.x_cm = 3500;
        let result = harness.think(withdrawing, &[], 2)?;
        check!(has_goal(&result, MotorGoal::Cell { x: 1, y: 4 }));
        check!(!arms(&result));
    }
    Ok(())
}

#[test]
fn screen_requires_current_arc_stopped_motion_and_confirmed_lock() -> TestResult {
    for (name, half) in [("Drill", 60), ("Guard", 45)] {
        let harness = Harness::new(name)?;
        for (bearing, expected) in [(half, true), (half + 1, false)] {
            let mut view = healthy(MindObjective::Screen { facing_mdeg: 0 });
            view.lock = Some(fixture::HOSTILE);
            view.contacts
                .iter_mut()
                .find(|row| row.id == fixture::HOSTILE)
                .ok_or("target")?
                .bearing_deg = bearing;
            let result = harness.think(view, &[], 0)?;
            check!(has_goal(&result, MotorGoal::Hold));
            check!(result.intents.contains(&IntentKind::Heading { mdeg: 0 }));
            check_eq!(arms(&result), expected);
        }
    }
    Ok(())
}

#[test]
fn missing_expired_or_hold_fire_order_clears_prior_conditions() -> TestResult {
    let previous = fixture::view()
        .weapons
        .into_iter()
        .find_map(|weapon| weapon.armed)
        .ok_or("actual prior condition")?;
    for mode in 0..3 {
        let mut view = ready_target();
        view.weapons.first_mut().ok_or("weapon")?.armed = Some(previous.clone());
        match mode {
            0 => view.order = None,
            1 => view.order.as_mut().ok_or("order")?.expires_ms = view.t_ms,
            _ => view.order.as_mut().ok_or("order")?.hold_fire = true,
        }
        let result = Harness::new("Drill")?.think(view, &[], mode)?;
        check!(has_goal(&result, MotorGoal::Hold));
        // Hold-fire never arms. With no current order (modes 0 and 1) the
        // standing guard may run its drill afresh, never on the old condition.
        if mode == 2 {
            check!(!arms(&result));
        }
        check!(
            conditions(&result)
                .iter()
                .all(|c| c.target != previous.target || c.min_permille != previous.min_permille)
        );
        check!(
            result
                .intents
                .contains(&IntentKind::Condition(Arming::Disarm { weapon: None }))
        );
    }
    Ok(())
}

/// Doc 17h M8: with no current order the drill stands guard. It halts and
/// runs the same stationary fire drill on a fresh hostile in range (lock
/// first, then fire only once stopped), marches to the match objective when
/// nothing hostile is in range, and holds when there is neither.
#[test]
fn no_current_order_stands_guard_and_drills_or_marches_to_the_objective() -> TestResult {
    for (name, odds, kept) in [("Drill", 500, 2), ("Guard", 700, 4)] {
        let harness = Harness::new(name)?;
        let mut view = ready_target();
        view.order = None;
        let drilling = harness.think(view.clone(), &[], 0)?;
        check!(has_goal(&drilling, MotorGoal::Hold));
        check!(!conditions(&drilling).is_empty());
        for rule in conditions(&drilling) {
            check_eq!(
                rule.target,
                SlotTarget::Contact {
                    id: fixture::HOSTILE
                }
            );
            check_eq!(rule.min_permille, odds);
            check_eq!(rule.min_rounds_left, kept);
        }
        let mut unlocked = view.clone();
        unlocked.lock = None;
        let locking = harness.think(unlocked, &[], 1)?;
        check!(locking.intents.contains(&IntentKind::Lock {
            contact: fixture::HOSTILE
        }));
        check!(!arms(&locking));
        let mut moving = view.clone();
        moving.speed_cms = 100;
        check!(!arms(&harness.think(moving, &[], 2)?));
        let mut clear = view.clone();
        clear.lock = None;
        clear.contacts.retain(|row| row.id == fixture::FRIEND);
        let marching = harness.think(clear.clone(), &[], 3)?;
        check!(has_goal(&marching, MotorGoal::Cell { x: 6, y: 4 }));
        check!(!arms(&marching));
        clear.objective = None;
        let holding = harness.think(clear, &[], 4)?;
        check!(has_goal(&holding, MotorGoal::Hold));
        check!(!arms(&holding));
    }
    Ok(())
}

#[test]
fn actual_order_range_cap_controls_stationary_firing_not_a_requested_stop() -> TestResult {
    let mut harness = Harness::new("Drill")?;
    Arc::make_mut(&mut harness.map).terrain =
        crate::arena::state::ArenaTerrain::uniform(64, 16, 1000, ground())?;
    let mut view = ready_target();
    let target = view
        .contacts
        .iter_mut()
        .find(|row| row.id == fixture::HOSTILE)
        .ok_or("target")?;
    target.range_m = 220;
    target.cell = Some((23, 4));
    let original = crate::arena::stock_mind::view_ranges(&view, 600, view.order.as_ref());
    check!(22000 < original.engage_cm);
    check!(arms(&harness.think(view.clone(), &[], 0)?));
    view.order.as_mut().ok_or("order")?.max_range_band =
        Some(crate::deliberation::order::RangeBand::Close);
    let limited = crate::arena::stock_mind::view_ranges(&view, 600, view.order.as_ref());
    check!(limited.engage_cm < 22000);
    let expected = crate::arena::stock_mind::stand_off_of(
        &harness.map,
        &view,
        view.contacts
            .iter()
            .find(|row| row.id == fixture::HOSTILE)
            .ok_or("target")?,
        limited.engage_cm,
    )
    .ok_or("public station")?;
    let result = harness.think(view, &[], 1)?;
    check!(has_goal(
        &result,
        MotorGoal::Cell {
            x: expected.0,
            y: expected.1
        }
    ));
    check!(
        !arms(&result),
        "a movement request cannot count as a completed stationary drill"
    );
    Ok(())
}

#[test]
fn support_follows_its_earned_charge_plot_and_refuses_lost_or_foreign_charge() -> TestResult {
    let mut harness = Harness::new("Drill")?;
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
        3000,
    )
    .ok_or("public station")?;
    let moving = harness.think(view.clone(), &[], 0)?;
    check!(has_goal(
        &moving,
        MotorGoal::Cell {
            x: expected.0,
            y: expected.1
        }
    ));
    check!(!arms(&moving));
    for mode in 0..3 {
        let mut missing = view.clone();
        match mode {
            0 => {
                missing
                    .contacts
                    .iter_mut()
                    .find(|row| row.id == fixture::FRIEND)
                    .ok_or("charge")?
                    .age_ms = 2501
            }
            1 => {
                missing.order = Some(order(MindObjective::Support {
                    of: Some(ContactId(997)),
                }))
            }
            _ => {
                missing.order = Some(order(MindObjective::Support {
                    of: Some(fixture::HOSTILE),
                }))
            }
        }
        let result = harness.think(missing, &moving.memory, mode)?;
        check!(has_goal(&result, MotorGoal::Hold));
        check!(!arms(&result));
    }
    Ok(())
}

#[test]
fn full_public_rows_and_unarmed_hulls_keep_the_real_runtime_limits() -> TestResult {
    for name in ["Drill", "Guard"] {
        let harness = Harness::new(name)?;
        let mut view = healthy(MindObjective::Screen { facing_mdeg: 90000 });
        view.lock = Some(fixture::HOSTILE);
        let template = view
            .contacts
            .iter()
            .find(|row| row.id == fixture::HOSTILE)
            .ok_or("target")?
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
            check!(!conditions(&result).is_empty());
            check!(conditions(&result).iter().all(|rule| rule.target
                == SlotTarget::Contact {
                    id: fixture::HOSTILE
                }));
            memory = result.memory;
        }
        view.weapons.clear();
        let result = harness.think(view, &memory, 8)?;
        check!(
            result
                .intents
                .iter()
                .all(|intent| intent.channel_required() != Channel::Fire)
        );
        let mut unarmed = engaged();
        unarmed.weapons.clear();
        let result = harness.think(unarmed, &[], 9)?;
        check!(
            result
                .intents
                .iter()
                .all(|intent| intent.channel_required() != Channel::Fire)
        );
    }
    Ok(())
}

#[test]
fn drill_actual_native_no_route_repeats_only_after_each_fresh_delay() -> TestResult {
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
    let mut hull = line_tank(fixture::UNIT, PartyId::RED, pose(3500, 3500));
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
    check_eq!(
        apply_intent(&mut arena, &input, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    let mut counters = MotorCounters::default();
    arena.motor_step(fixture::UNIT, 5000, &mut counters);
    let mut actual = arena.mind_view(fixture::UNIT).ok_or("actual public view")?;
    check_eq!(
        actual.motor.ok_or("actual motor")?.outcome,
        Some(DriveOutcome::NoRoute)
    );
    actual.order = Some(order(MindObjective::Advance { to: (3, 6) }));
    let mut harness = Harness::new("Drill")?;
    harness.map = Arc::clone(&arena.definition);
    let stopped = harness.think(actual.clone(), &[], 0)?;
    check!(has_goal(&stopped, MotorGoal::Hold));
    let hold = Intent {
        kind: IntentKind::Goal(MotorGoal::Hold),
        seq: 1,
        ..input
    };
    check_eq!(
        apply_intent(&mut arena, &hold, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    arena.now_ms = 6999;
    let mut after_hold = arena.mind_view(fixture::UNIT).ok_or("after hold")?;
    after_hold.order = actual.order.clone();
    let waiting = harness.think(after_hold.clone(), &stopped.memory, 1)?;
    check!(
        has_goal(&waiting, MotorGoal::Hold),
        "HOLD removing a motor row must not erase retry delay"
    );
    after_hold.t_ms = 7000;
    let retry = harness.think(after_hold, &waiting.memory, 2)?;
    check!(has_goal(&retry, MotorGoal::Cell { x: 3, y: 6 }));
    check!(!arms(&retry));

    // Land that retry through the unchanged native route solver. The same
    // enclosed destination really fails again; it must earn a new delay.
    arena.now_ms = 7000;
    let second_attempt = Intent { seq: 2, ..input };
    check_eq!(
        apply_intent(&mut arena, &second_attempt, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    arena.motor_step(fixture::UNIT, 7000, &mut counters);
    let mut second_failure = arena.mind_view(fixture::UNIT).ok_or("second failure")?;
    check_eq!(
        second_failure.motor.ok_or("second native motor")?.outcome,
        Some(DriveOutcome::NoRoute)
    );
    second_failure.order = actual.order.clone();
    let second_stop = harness.think(second_failure, &retry.memory, 3)?;
    check!(has_goal(&second_stop, MotorGoal::Hold));
    let second_hold = Intent { seq: 3, ..hold };
    check_eq!(
        apply_intent(&mut arena, &second_hold, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    arena.now_ms = 8999;
    let mut second_wait = arena.mind_view(fixture::UNIT).ok_or("second hold")?;
    second_wait.order = actual.order;
    let waiting_again = harness.think(second_wait.clone(), &second_stop.memory, 4)?;
    check!(has_goal(&waiting_again, MotorGoal::Hold));
    second_wait.t_ms = 9000;
    let retry_again = harness.think(second_wait, &waiting_again.memory, 5)?;
    check!(has_goal(&retry_again, MotorGoal::Cell { x: 3, y: 6 }));
    check!(!arms(&retry_again));
    Ok(())
}

#[test]
fn drill_actual_native_arrival_holds_without_firing() -> TestResult {
    let mut hull = line_tank(fixture::UNIT, PartyId::RED, pose(3500, 3500));
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
    check_eq!(
        apply_intent(&mut arena, &input, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
    arena.motor_step(fixture::UNIT, 5000, &mut MotorCounters::default());
    let mut view = arena
        .mind_view(fixture::UNIT)
        .ok_or("actual public arrival")?;
    check_eq!(
        view.motor.ok_or("motor")?.outcome,
        Some(DriveOutcome::Arrived)
    );
    view.order = Some(order(MindObjective::Advance { to: (3, 3) }));
    let harness = Harness::new("Drill")?;
    let result = harness.think(view, &[], 0)?;
    check!(has_goal(&result, MotorGoal::Hold));
    check!(!arms(&result));
    Ok(())
}

#[test]
fn guard_withdraws_earlier_and_a_cold_hull_starts_up_without_arming() -> TestResult {
    let mut hurt = ready_target();
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
    check!((300..450).contains(&crate::arena::stock_mind::health_permille(&hurt)));
    check!(arms(&Harness::new("Drill")?.think(hurt.clone(), &[], 0)?));
    let guarded = Harness::new("Guard")?.think(hurt, &[], 0)?;
    check!(has_goal(&guarded, MotorGoal::Cell { x: 1, y: 4 }));
    check!(!arms(&guarded));
    for name in ["Drill", "Guard"] {
        let harness = Harness::new(name)?;
        let mut cold = ready_target();
        cold.power = Power::Off;
        let commanded = harness.think(cold.clone(), &[], 0)?;
        check!(commanded.intents.contains(&IntentKind::Startup));
        check!(!arms(&commanded));
        // Doc 17h M8: with no current order a cold hull starts up for its
        // standing guard, and still arms nothing on that think.
        cold.order = None;
        let idle = harness.think(cold, &[], 1)?;
        check!(idle.intents.contains(&IntentKind::Startup));
        check!(!arms(&idle));
    }
    Ok(())
}

/// Doc 17h M8: the unordered doctrine changes nothing under a current order.
/// The shipped-before source (content version 3 as first pinned, rebuilt here
/// by taking the new `stand_guard` out and restoring the old prologue, and proved
/// by its old digest) and the current source emit the same intents and memory
/// for every order objective, power state and hold-fire flag.
#[test]
fn under_a_current_order_the_doctrine_is_unchanged() -> TestResult {
    let start = SOURCE
        .find("// With no current order")
        .ok_or("new doctrine")?;
    let end = SOURCE.find("fn think(view, card)").ok_or("think")?;
    let new_prologue = "    if view.power != \"online\" { startup(); return; }\n    let order = view.order;\n    if order == () || order.expires_ms <= t_ms() { this.stand_guard(view, card); return; }\n";
    let old_prologue = "    let order = view.order;\n    if order == () || order.expires_ms <= t_ms() { hold(); return; }\n    if view.power != \"online\" { startup(); return; }\n";
    check!(SOURCE.contains(new_prologue));
    let before =
        format!("{}{}", &SOURCE[..start], &SOURCE[end..]).replace(new_prologue, old_prologue);
    check_eq!(
        crate::arena::digest_text(&Sha256::digest(before.as_bytes()).into()),
        "36fd55d2c6494a56be77d483f70239f5926346f1b7e4723e67fd753ca86c9320"
    );
    let objectives = [
        MindObjective::Hold,
        MindObjective::Advance { to: (6, 4) },
        MindObjective::Engage {
            target: Some(fixture::HOSTILE),
        },
        MindObjective::Engage { target: None },
        MindObjective::Withdraw,
        MindObjective::Screen {
            facing_mdeg: 90_000,
        },
        MindObjective::Support {
            of: Some(fixture::FRIEND),
        },
    ];
    for name in ["Drill", "Guard"] {
        let (old, new) = (Harness::with_source(name, &before)?, Harness::new(name)?);
        let mut sequence = 0;
        for objective in &objectives {
            for power in [Power::Online, Power::Off] {
                for hold_fire in [false, true] {
                    for locked in [false, true] {
                        let mut view = healthy(*objective);
                        view.power = power;
                        view.order.as_mut().ok_or("order")?.hold_fire = hold_fire;
                        view.lock = locked.then_some(fixture::HOSTILE);
                        let was = old.think(view.clone(), &[], sequence)?;
                        let is = new.think(view, &[], sequence)?;
                        check_eq!(was.intents, is.intents, "{name} {objective:?} {power:?}");
                        check_eq!(was.memory, is.memory);
                        sequence += 1;
                    }
                }
            }
        }
    }
    Ok(())
}
