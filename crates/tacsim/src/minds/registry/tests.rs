use std::{collections::BTreeMap, sync::Arc};

use mush_core::ObjId;
use mush_roll::{
    Envelope, Revision,
    commerce::rating::{CaseKind, RatingError},
};
use serde_json::json;

use super::{
    meter::{Binding, MeteredRunner},
    plan::{ClassPhysics, Physics, Schedule},
    subject::PreparedSubject,
};
use crate::{
    arena::{
        core::{ClassTable, CoreClass},
        mind::MindRunner,
    },
    config::ArenaConfig,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const SOURCE: &str = "fn think(v,c) { stock::rules(); helm(3); this.landed = t_ms(); }";

pub(super) fn revision(source: &str) -> Result<Revision, Box<dyn std::error::Error>> {
    use crate::arena::{conditions::Rule, schema::AuthoredRule};
    let hold = AuthoredRule {
        rule: Rule::HoldFire {
            weapon: "all".into(),
            predicates: Vec::new(),
            implicit: false,
        },
        think: Vec::new(),
    };
    let envelope = Envelope {
        schema: json!({"sections":[{"id":"helm","label":"Helm","station":"helm","fields":[{"id":"bias","label":"Bias","widget":{"kind":"slider","min":0,"max":10,"step":1,"default":0,"scale":1,"unit":"m","help":"Offset"}}]}]}),
        presets: (0..8)
            .map(|n| (format!("Preset{n}"), json!({"bias":n})))
            .collect::<serde_json::Map<_, _>>()
            .into(),
        profiles: json!({"Alpha":{"bias":1},"Beta":{"bias":2}}),
        rules: json!({"Alpha":[hold],"Beta":[]}),
        declared_class: "I".into(),
        mind_abi: crate::minds::MIND_ABI.into(),
    };
    Ok(serde_json::from_value(
        json!({"format":1,"reference":{"key":{"character":1,"mind":1,"number":1},"digest":envelope.digest(source)?},"source":mush_roll::source_digest(source),"envelope":envelope,"notes":""}),
    )?)
}

#[test]
fn real_source_expansion_retains_every_saved_program_for_every_preset() -> TestResult {
    let revision = revision(SOURCE)?;
    let prepared = PreparedSubject::prepare(&revision, SOURCE, &ArenaConfig::default())?;
    assert_eq!(prepared.variants().len(), 10);
    assert_eq!(prepared.pairs().len(), 18);
    assert_eq!(prepared.aliases().len(), 10);
    for variant in prepared.variants() {
        let [alpha, beta] = variant.definition.profiles.as_slice() else {
            return Err("two exact programs".into());
        };
        assert_eq!(alpha.rules.len(), 1);
        assert!(beta.rules.is_empty());
        if variant.cycle {
            assert_eq!(alpha.values, beta.values);
        }
    }
    for pair in prepared.pairs() {
        let variant = prepared
            .variants()
            .get(usize::from(pair.variant))
            .ok_or("variant")?;
        assert!(
            variant
                .definition
                .profiles
                .get(usize::from(pair.profile))
                .is_some()
        );
    }
    assert!(
        PreparedSubject::prepare(&revision, "fn think(v,c) {}", &ArenaConfig::default()).is_err()
    );
    assert_eq!(prepared.subject().revision().source(), revision.source());
    Ok(())
}

#[test]
fn subject_never_runs_top_level_and_exact_envelope_changes_identity() -> TestResult {
    let source = "while true {} fn think(v,c) {}";
    let original = revision(source)?;
    let a = PreparedSubject::prepare(&original, source, &ArenaConfig::default())?;
    let mut value = serde_json::to_value(&original)?;
    value["envelope"]["presets"]["Preset0"]["bias"] = json!(9);
    let envelope: Envelope = serde_json::from_value(value["envelope"].clone())?;
    value["reference"]["digest"] = serde_json::to_value(envelope.digest(source)?)?;
    let changed: Revision = serde_json::from_value(value)?;
    let b = PreparedSubject::prepare(&changed, source, &ArenaConfig::default())?;
    assert_eq!(
        a.subject().revision().source(),
        b.subject().revision().source()
    );
    assert_ne!(a.subject().expanded(), b.subject().expanded());
    assert_ne!(a.subject().envelope(), b.subject().envelope());
    Ok(())
}

fn physics(class: CoreClass, classes: &ClassTable) -> Result<ClassPhysics, RatingError> {
    let one = Physics {
        units: 1,
        native_ms: 250,
    };
    let two = Physics {
        units: 2,
        native_ms: 250,
    };
    let three = Physics {
        units: 3,
        native_ms: 250,
    };
    let four = Physics {
        units: 4,
        native_ms: 250,
    };
    let density = Physics {
        units: u16::try_from(classes.row(class).rows + 1).map_err(|_| RatingError::Bound)?,
        native_ms: 250,
    };
    Ok(ClassPhysics {
        class,
        trials: [two, four, four],
        probes: [
            one, one, two, two, three, three, three, three, density, density, one, one,
        ],
        cycle: one,
        kernel: one,
        loss_expiry_ms: 4_000,
    })
}

#[test]
fn finite_schedule_covers_all_pairs_and_charges_actual_case_rosters() -> TestResult {
    let prepared = PreparedSubject::prepare(&revision(SOURCE)?, SOURCE, &ArenaConfig::default())?;
    let classes = ClassTable::STOCK;
    let physical = [CoreClass::I, CoreClass::II, CoreClass::III]
        .into_iter()
        .map(|class| physics(class, &classes))
        .collect::<Result<Vec<_>, _>>()?;
    let schedule = Schedule::prepare(&prepared, &classes, &physical)?;
    assert_eq!(schedule.cases().len(), 3 * (10 * 12 + 18 * 12 + 1 + 8));
    assert_eq!(
        schedule
            .cases()
            .iter()
            .filter(|case| matches!(case.key.kind(), CaseKind::Trial { .. }))
            .count(),
        360
    );
    let direct = schedule
        .cases()
        .iter()
        .filter(|case| !matches!(case.key.kind(), CaseKind::Trial { .. }))
        .map(|case| case.maximum_thinks)
        .sum::<u64>();
    assert_eq!(direct, 3 * (18 * 678 + 256 + 8));
    assert!(
        schedule
            .cases()
            .windows(2)
            .all(|pair| matches!(pair, [a,b] if a.key < b.key))
    );
    let sum = schedule
        .cases()
        .iter()
        .map(|case| case.maximum_unit_ms())
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum::<u64>();
    assert_eq!(schedule.maximum_unit_ms(), sum);
    assert!(schedule.cases().iter().any(|case| case.physics.units == 25));
    assert!(schedule.cases().iter().any(|case| case.physics.units == 1));
    let mut wrong = physical.clone();
    wrong
        .first_mut()
        .ok_or("class")?
        .probes
        .get_mut(8)
        .ok_or("density")?
        .units = 1;
    assert!(Schedule::prepare(&prepared, &classes, &wrong).is_err());
    Ok(())
}

#[test]
fn actual_runner_meter_preserves_answers_and_counts_host_fees_at_real_landing() -> TestResult {
    use crate::arena::{
        motor::MotorCounters,
        state::{Controller, Power, Team},
        test_support::{arena_of, line_tank, pose},
    };
    let own = ObjId(4);
    let mut hull = line_tank(own, Team::Red, pose(1500, 4500));
    hull.controller = Controller::Mind;
    hull.reseal(&crate::match_::SealTerms::fixture())?;
    let mut arena = arena_of(vec![
        hull,
        line_tank(ObjId(9), Team::Blue, pose(6500, 4500)),
    ]);
    arena.unit_mut(own).ok_or("own")?.power = Power::Online;
    let prepared = PreparedSubject::prepare(&revision(SOURCE)?, SOURCE, &ArenaConfig::default())?;
    let runtime = arena.profiles.get(&own).ok_or("runtime")?;
    let meter = Arc::new(MeteredRunner::new(
        prepared.runner(),
        arena.mind_profile(own),
        BTreeMap::from([(
            own,
            vec![Binding::capture(&runtime.card, &runtime.program)?],
        )]),
        3,
    )?);
    arena.mind_runner = meter.clone();
    for scheduled in [1001, 2001, 3001] {
        arena.now_ms = scheduled + 249;
        let sequence = arena.minds.get(&own).ok_or("mind")?.think_seq;
        arena.collect_think(own, sequence, scheduled, &mut MotorCounters::default());
        let think = arena.collected.pop().ok_or("real collected job")?;
        let actual = think.think().result;
        let plain = prepared.runner().think(&think.job());
        assert_eq!(actual, plain, "measurement does not alter execution");
        assert!(actual.fault.is_none());
        let settled = arena.settle_collected(&think, actual);
        assert_eq!(settled.faults, 0);
        arena.now_ms = think.lands_ms;
        arena.mind_land(
            own,
            think.think_seq,
            think.lands_ms,
            &mut MotorCounters::default(),
        );
    }
    let evidence = meter.measurement()?;
    assert_eq!(evidence.summary.thinks(), 3);
    assert!(evidence.summary.max_ops() > 0);
    assert!(evidence.summary.max_fees() >= crate::minds::fees::FEE_THINK);
    assert!(evidence.summary.clean());
    assert_eq!(evidence.seen, vec![vec![3]]);
    let mind = arena.minds.get(&own).ok_or("mind")?;
    assert_eq!(mind.strikes, 0);
    assert!(!mind.dropped());
    Ok(())
}

#[test]
fn duplicate_or_foreign_jobs_cannot_become_valid_measurement() -> TestResult {
    use crate::{
        arena::{mind::ThinkJob, profile_runtime::ProfileToken},
        minds::fixture,
    };
    let prepared = PreparedSubject::prepare(&revision(SOURCE)?, SOURCE, &ArenaConfig::default())?;
    let view = Arc::new(fixture::view());
    let map = Arc::new(fixture::map());
    let card = fixture::runtime_card(fixture::card());
    let program = fixture::empty_program();
    let profile = ClassTable::STOCK.row(CoreClass::I);
    let meter = MeteredRunner::new(
        prepared.runner(),
        profile,
        BTreeMap::from([(fixture::UNIT, vec![Binding::capture(&card, &program)?])]),
        4,
    )?;
    let job = ThinkJob {
        unit: fixture::UNIT,
        think_seq: 3,
        t_ms: 5000,
        lands_ms: 5100,
        seed: 42,
        dropped: false,
        profile: &profile,
        view: &view,
        memory: b"{}",
        map: &map,
        card: &card,
        program: &program,
        profile_token: ProfileToken {
            unit: fixture::UNIT,
            generation: 0,
            key: card.key(),
        },
    };
    let first = meter.think(&job);
    assert!(first.fault.is_none());
    assert_eq!(meter.measurement()?.summary.thinks(), 1);
    assert_eq!(
        meter.think(&job),
        first,
        "the real duplicate answer is still unchanged"
    );
    assert!(
        meter.measurement().is_err(),
        "duplicate sequence invalidates the evidence instead of double-counting it"
    );
    Ok(())
}

pub(super) fn frozen_policy() -> Result<super::manifest::FrozenPolicy, Box<dyn std::error::Error>> {
    use mush_atlas::{AtlasModule, AtlasState};
    use mush_content::{Appliers, ContentModule};
    use mush_testkit::TestWorld;
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers)?;
    let atlas = mush_content::parse_pack(crate::definitions::content::PROVING_GROUND_ATLAS_JSON)?;
    let world = TestWorld::with_modules(vec![
        Box::new(AtlasModule::new()),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(vec![atlas]),
        ),
    ]);
    let records = &world
        .world()
        .module_state::<AtlasState>()
        .ok_or("actual atlas")?
        .records;
    let catalog = Arc::new(crate::definitions::Catalog::build_fielded(&[], crate::NS)?);
    Ok(super::manifest::FrozenPolicy::capture(
        catalog,
        records,
        crate::config::TacsimConfig::default(),
    )?)
}
