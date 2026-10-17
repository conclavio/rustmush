//! Executable M6.10 acceptance of the independent public-picture pilot.
//! Tests use the real bounded runner and the production six-picture provider.

use std::sync::Arc;

use mush_mind::memory::{self, Value};
use mush_roll::{Envelope, RevisionAdapter, WorkingCopy};

use super::check::AuthorPictures;
use super::fixture;
use super::workshop::executor::CheckPictures;
use super::{RhaiRunner, ThinkStat};
use crate::arena::conditions::Arming;
use crate::arena::core::{ClassTable, CoreClass};
use crate::arena::intents::IntentKind;
use crate::arena::mind::{MindRunner, MindView, ThinkJob, ThinkResult};
use crate::arena::profile_rules::BoundProgram;
use crate::arena::profile_runtime::ProfileToken;
use crate::arena::runtime_card::{
    CardCatalog, CardDefinition, ProfileDefinition, ProfileKey, RuntimeCard,
};
use crate::arena::state::{Channel, Classification, ContactId, Power};
use crate::config::{ArenaConfig, MindsConfig};

const SOURCE: &str = include_str!("../../content/minds/acceptance/field_pilot.rhai");
const ENVELOPE: &str = include_str!("../../content/minds/acceptance/field_pilot.envelope.json");
const MANIFEST: &str = include_str!("../../content/minds/acceptance/field_pilot.manifest.json");

fn envelope() -> Envelope {
    serde_json::from_str(ENVELOPE).expect("acceptance envelope")
}

fn healthy() -> MindView {
    let mut view = fixture::view();
    view.notices.clear();
    view.rules.clear();
    view.order = None;
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
        .retain(|contact| contact.id == fixture::HOSTILE);
    view
}

struct Harness {
    runner: RhaiRunner,
    card: RuntimeCard,
    program: BoundProgram,
    map: Arc<crate::arena::state::ArenaDefinition>,
}
impl Harness {
    fn new(name: &str) -> Self {
        let envelope = envelope();
        let checked = crate::arena::schema::validate_envelope(
            &envelope.schema,
            &envelope.presets,
            &envelope.profiles,
            &envelope.rules,
            CoreClass::II,
        )
        .expect("complete cards");
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
            .position(|profile| profile.name == name)
            .expect("profile");
        let mut catalog = CardCatalog::default();
        let digest = catalog.admit(definition).expect("same canonical admission");
        let card = catalog
            .resolve(ProfileKey {
                definition: digest,
                index: u8::try_from(index).unwrap(),
            })
            .unwrap();
        let program = BoundProgram::bind(
            &[],
            &[],
            &[Channel::Helm, Channel::Sensors, Channel::Fire],
            &ClassTable::STOCK.row(CoreClass::II),
            100,
        )
        .unwrap();
        let runner = RhaiRunner::on(
            Arc::new(mush_mind::Sandbox::new(
                &mush_mind::EngineLimits::default(),
                super::register_api,
            )),
            &ArenaConfig::default(),
            SOURCE,
            fixture::digest(SOURCE),
        )
        .expect("bounded source compiles")
        .recording();
        Self {
            runner,
            card,
            program,
            map: Arc::new(fixture::map()),
        }
    }
    fn think(&self, view: MindView, bytes: &[u8], seq: u64) -> (ThinkResult, ThinkStat) {
        let profile = ClassTable::STOCK.row(CoreClass::II);
        let now = view.t_ms;
        let result = self.runner.think(&ThinkJob {
            unit: fixture::UNIT,
            think_seq: seq,
            t_ms: now,
            lands_ms: now + 100,
            seed: 42,
            dropped: false,
            profile: &profile,
            view: &Arc::new(view),
            memory: bytes,
            map: &self.map,
            card: &self.card,
            program: &self.program,
            profile_token: ProfileToken {
                unit: fixture::UNIT,
                generation: 0,
                key: self.card.key(),
            },
        });
        let stat = self.runner.take_stats().pop().expect("measured think");
        assert!(
            result.fault.is_none(),
            "seq {seq}: {:?}, ops {} + fees {}",
            result.fault,
            stat.ops,
            stat.fee
        );
        assert!(
            stat.ops + stat.fee <= 3000,
            "real Class II budget: {stat:?}"
        );
        assert!(result.intents.len() <= 16);
        let memory = memory::decode(&result.memory).expect("canonical flat memory");
        assert!(memory.len() <= 64);
        for value in memory.values() {
            if let Value::Ints(items) = value {
                assert!(items.len() <= 16);
            }
        }
        (result, stat)
    }
}

#[test]
fn field_pilot_manifest_and_production_six_picture_check_are_real() {
    let manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
    assert_eq!(
        super::API_VERSION,
        4,
        "this authored content requires the M6 API"
    );
    assert_eq!(
        SOURCE.len(),
        manifest["source_bytes"].as_u64().unwrap() as usize
    );
    assert!(SOURCE.len() <= mush_roll::MAX_SOURCE_BYTES);
    use sha2::{Digest as _, Sha256};
    assert_eq!(
        crate::arena::digest_text(&Sha256::digest(SOURCE.as_bytes()).into()),
        manifest["source_sha256"]
    );
    assert!(manifest["functions"].as_array().unwrap().len() <= 32);
    assert!(manifest["memory_keys"].as_array().unwrap().len() <= 64);
    for forbidden in [
        "stock::think",
        "stock::target",
        "stock::move",
        "import ",
        "eval(",
    ] {
        assert!(!SOURCE.contains(forbidden), "{forbidden}");
    }
    let envelope = envelope();
    super::authoring::RevisionRules
        .validate(SOURCE, &envelope)
        .expect("production immutable-save validation");
    let reports = AuthorPictures::new(ArenaConfig::default(), MindsConfig::default())
        .run(&WorkingCopy {
            source: SOURCE.into(),
            envelope,
        })
        .expect("real production checker");
    assert_eq!(reports.len(), 6);
    for report in reports {
        assert_eq!(report.fault, None, "{report:?}");
        assert!(report.class_violations.is_empty(), "{report:?}");
        assert!(report.station_refusals.is_empty(), "{report:?}");
        assert!(report.operations + report.fees <= 3000, "{report:?}");
        if !report.armed {
            assert!(
                report
                    .intents
                    .iter()
                    .all(|intent| !intent.contains("\"weapon\"")),
                "{report:?}"
            );
        }
    }
}

#[test]
fn field_pilot_maximum_rows_and_every_profile_stay_in_class_ii() {
    let profile = ClassTable::STOCK.row(CoreClass::II);
    assert_eq!(
        (
            profile.think_ms,
            profile.ops,
            profile.rows,
            profile.odds_rows,
            profile.slots,
            profile.memory_keys,
            profile.line_to,
            profile.profiles
        ),
        (500, 3000, 16, 4, 4, 64, 4, 3)
    );
    for name in ["Hold", "Orbit", "Close"] {
        let harness = Harness::new(name);
        let mut view = healthy();
        let first = view.contacts[0].clone();
        view.contacts = (0..16)
            .map(|i| {
                let mut contact = first.clone();
                contact.id = ContactId(100 + i);
                contact.range_m = 450 + i * 50;
                contact.age_ms = 0;
                if i >= 4 {
                    contact.odds.clear();
                }
                contact
            })
            .collect();
        let mut bytes = Vec::new();
        for seq in 1..=32 {
            view.t_ms = 5000 + i64::try_from(seq - 1).unwrap() * 500;
            let (result, stat) = harness.think(view.clone(), &bytes, seq);
            println!(
                "FIELD_PILOT {name} seq={seq} ops={} fees={} memory={}",
                stat.ops,
                stat.fee,
                result.memory.len()
            );
            bytes = result.memory;
        }
    }
}

#[test]
fn field_pilot_target_hysteresis_and_unknown_admission_use_only_the_public_rows() {
    let harness = Harness::new("Hold");
    let mut view = healthy();
    view.contacts[0].range_m = 500;
    let (first, _) = harness.think(view.clone(), &[], 1);
    let mut other = view.contacts[0].clone();
    other.id = ContactId(800);
    other.range_m = 450;
    view.contacts.push(other);
    view.t_ms += 500;
    let (kept, _) = harness.think(view.clone(), &first.memory, 2);
    assert_eq!(
        memory::decode(&kept.memory).unwrap().get("target"),
        Some(&Value::Int(super::api::contact_int(fixture::HOSTILE)))
    );
    view.contacts[1].range_m = 400;
    view.t_ms += 500;
    let (changed, _) = harness.think(view.clone(), &kept.memory, 3);
    assert_eq!(
        memory::decode(&changed.memory).unwrap().get("target"),
        Some(&Value::Int(800))
    );
    for row in &mut view.contacts {
        row.class = Classification::Unknown;
        row.odds.clear();
    }
    view.t_ms += 500;
    let (unknown, _) = harness.think(view, &changed.memory, 4);
    assert!(!unknown.intents.iter().any(|intent| matches!(
        intent,
        IntentKind::Lock { .. } | IntentKind::Condition(Arming::Arm(_)) | IntentKind::Fire { .. }
    )));
}

#[test]
fn field_pilot_hold_hysteresis_is_not_a_continuous_full_speed_charge() {
    for (distance, current, wanted) in [
        (550, 0, 1500),
        (500, 0, 0),
        (500, 1500, 0),
        (550, 1500, 1500),
        (350, 0, -750),
        (400, -750, 0),
    ] {
        let harness = Harness::new("Hold");
        let mut view = healthy();
        view.contacts[0].range_m = distance;
        view.contacts[0].age_ms = 0;
        view.speed_cms = current;
        let (result, _) = harness.think(view, &[], 1);
        assert!(
            result.intents.contains(&IntentKind::Speed { cms: wanted }),
            "distance={distance} current={current}: {:?}",
            result.intents
        );
    }
}

#[test]
fn field_pilot_does_not_arm_an_unarmed_view_and_does_not_reuse_a_stale_hostile() {
    let harness = Harness::new("Hold");
    let mut view = healthy();
    view.weapons.clear();
    view.contacts[0].age_ms = 3001;
    let (result, _) = harness.think(view, &[], 1);
    assert!(!result.intents.iter().any(|intent| matches!(
        intent,
        IntentKind::Lock { .. } | IntentKind::Condition(_) | IntentKind::Fire { .. }
    )));
}

#[test]
fn field_pilot_auto_orbit_steers_to_a_public_course_rear_without_a_timed_flip() {
    let harness = Harness::new("Orbit");
    let mut view = healthy();
    view.cell = Some((0, 0));
    view.x_cm = 500;
    view.y_cm = 500;
    view.contacts[0].cell = Some((0, 45));
    view.contacts[0].bearing_deg = 0;
    view.contacts[0].range_m = 450;
    view.contacts[0].age_ms = 0;
    let (first, _) = harness.think(view.clone(), &[], 1);
    view.t_ms += 1000;
    view.contacts[0].cell = Some((1, 45));
    let (east, _) = harness.think(view.clone(), &first.memory, 2);
    assert!(
        east.intents
            .contains(&IntentKind::Heading { mdeg: 270_000 })
    );
    view.t_ms += 1000;
    view.contacts[0].cell = Some((0, 45));
    let (west, _) = harness.think(view, &east.memory, 3);
    assert!(west.intents.contains(&IntentKind::Heading { mdeg: 90_000 }));
}

#[test]
fn field_pilot_search_uses_public_b_then_eight_hundred_metres_beyond_and_back() {
    let mut harness = Harness::new("Hold");
    let mut map = fixture::map();
    // The search leg is 800 m beyond a marker 500 m across the map; keep
    // every observed pose and public marker on real authored terrain.
    map.terrain = crate::arena::state::ArenaTerrain::uniform(
        128,
        128,
        1000,
        crate::arena::test_support::ground(),
    )
    .expect("the search fixture's public ground");
    map.key_spots.push(crate::arena::state::KeySpot {
        name: "The Crossroads".into(),
        x: 50,
        y: 50,
    });
    harness.map = Arc::new(map);
    let mut view = healthy();
    view.contacts.clear();
    view.mode = crate::arena::state::MatchMode::Skirmish;
    view.objective = None;
    view.x_cm = 500;
    view.y_cm = 500;
    view.cell = Some((0, 0));
    view.t_ms = 0;
    let (first, _) = harness.think(view.clone(), &[], 1);
    view.t_ms = 5000;
    let (toward, _) = harness.think(view.clone(), &first.memory, 2);
    assert!(
        toward
            .intents
            .contains(&IntentKind::Heading { mdeg: 45_000 })
    );
    view.x_cm = 45_500;
    view.y_cm = 50_500;
    view.cell = Some((45, 50));
    view.t_ms = 5500;
    let (through, _) = harness.think(view.clone(), &toward.memory, 3);
    assert_eq!(
        memory::decode(&through.memory).unwrap().get("search_from"),
        Some(&Value::Ints(vec![45_500, 50_500]))
    );
    view.x_cm += 57_000;
    view.y_cm += 57_000;
    view.cell = Some((102, 107));
    view.t_ms = 6000;
    let (back, _) = harness.think(view, &through.memory, 4);
    assert_eq!(
        memory::decode(&back.memory).unwrap().get("search_from"),
        Some(&Value::Unit)
    );
    assert!(back.intents.iter().any(
        |intent| matches!(intent, IntentKind::Heading { mdeg } if (180_000..270_000).contains(mdeg))
    ));
}
