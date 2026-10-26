//! **Condition slots, through `Arena::step`** (17b cut M1b.1; doc 17 §5 "The
//! mechanism" and "The law", D-UM-16, D-UM-17 as amended 2026-10-04, D-UM-19;
//! doc 16 §3.1's rate independence, 16b §11's mandatory cross-tempo test).
//!
//! A condition slot is the principal's shot decision written down and
//! evaluated by the arena at the weapon's own `WeaponReady`, at the passes
//! that refresh the lock's plot, at a lock, an arming, a landing and a startup
//! — each at its own arena millisecond and never once per pulse. These tests
//! drive the library directly, in the `tests/weapons.rs` (h) harness's shape
//! (copied, not shared: an integration test file is its own crate):
//!
//! 1. the MANDATORY cross-tempo proof: a seat's arming resolves at the end of
//!    the pulse that accepted it and the slot's second shot lands on the
//!    `WeaponReady` millisecond — mid-pulse at the 5,000 ms cadence — at
//!    5,000 / 1,000 / 500 ms and on the task's own wakes, one event list and
//!    one hash sequence;
//! 2. a `LOCK` and an arming typed in ONE non-zero-`dt` pulse land in that
//!    order (the arming is discrete), and a typed `FIRE` then an arming finds
//!    the gun cycling;
//! 3. the production pins, re-run where they live (see the test's doc);
//! 4. a pass that turns the plot to a flank fires the slot at the pass's
//!    millisecond; a wingman's share at the share's; a startup at the
//!    startup's;
//! 5. the floor at the landing: `Unlawful` with the floor named, the captured
//!    floor (not a constant), a `Contact` target refused with the fallback;
//! 6. (in `tests/arena_task.rs`, where the task lives) the floor captured at
//!    check-out survives a resume;
//! 7. an unclassified lock is idle, `BestHostile` never fires on a friendly;
//! 8. a standing `hold_fire` stops a slot and never a pilot;
//! 9. the seat window counts an arming, and a third weapon is `NoSlot`;
//! 10. slots die with the controller and with the turret, and survive a
//!     linkdead grace;
//! 11. (in-source, `conditions.rs`) the evaluator's cost;
//! 12. a snapshot carries the slots, a card, the floor and `held_said`;
//! 13. **17b cut M1b.2 — the law for minds and T1's slots** (doc 17 §5,
//!     D-UM-16 to D-UM-19): a mind's `FIRE` gated in the pinned order
//!     (`Cycling`, `Empty`, `Unlawful`, hold-fire, the warrant, `Overheated`,
//!     the power) with its cross-tempo arm; hold-fire stopping a mind and its
//!     slot and never a pilot, with `hold_after_end` (Q-M7); a laser hull's
//!     T1 arming both its weapons whatever the heat and numbering them as the
//!     arena does; a retarget think's citation on the new lock and the stale
//!     `WeaponReady` window the twin counts as `cond_stale` — every T1
//!     fixture at 5,000, 1,000 and 500 ms; **17b cut M2.6**: and once more
//!     under Gate A's asserting twin (`support::with_twin`, through
//!     [`at_three_cadences`]' keep arm), T1 and T2 compared on every think;
//!     **17b cut M2.7**: and at 5,000, 1,000 and 500 ms on the Rhai Standard
//!     Doctrine ALONE (`support::with_rhai`, the same arm), each run the
//!     native one's event list, hash sequence and thinks.

mod support;

mod m5rules {
    use super::*;
    use tacsim::arena::core::CoreClass;
    use tacsim::arena::rules::{RuleCommand, RuleContact, RuleContext, RuleWeapon, parse_command};

    fn parsed(text: &str) -> Arming {
        let weapons = [
            RuleWeapon {
                id: "cannon".into(),
                rounds_max: 40,
            },
            RuleWeapon {
                id: "machine_gun".into(),
                rounds_max: 300,
            },
            RuleWeapon {
                id: "missiles".into(),
                rounds_max: 20,
            },
        ];
        let contacts = [RuleContact {
            name: "BA".into(),
            id: contact_of(RED, BLUE),
            classified: true,
        }];
        let RuleCommand::Arm(arming) = parse_command(
            text,
            &RuleContext {
                weapons: &weapons,
                floor_permille: 100,
                contacts: &contacts,
            },
        )
        .expect("native grammar") else {
            panic!("an arming");
        };
        arming
    }

    fn station(text: &str, seq: u64) -> Intent {
        intent(RED, IntentKind::Condition(parsed(text)), seq)
    }

    fn field(class: CoreClass, weapons: Vec<WeaponSummary>) -> Arena {
        let mut red = unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            weapons,
            Some(sight(0)),
        );
        red.unit = tactical_unit_classed(RED, class);
        arena_of(vec![
            red.sealed(),
            unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
        ])
    }

    #[test]
    fn sensors_rule_and_fire_rule_keep_exact_event_times_at_every_cadence() {
        let run = at_every_drive(
            || field(CoreClass::I, vec![cannon()]),
            10_000,
            || {
                vec![
                    beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
                    beat(
                        5_000,
                        vec![
                            station("RULE 1 LOCK nearest WHEN no lock", 2),
                            station(
                                "RULE 2 FIRE cannon WHEN target=stopped AND odds>=40 AND rounds>0",
                                3,
                            ),
                        ],
                    ),
                ]
            },
        );
        assert_eq!(
            cond_shots(&run),
            vec![(5_000, 2, u64::from(BLUE.0)), (9_000, 2, u64::from(BLUE.0))]
        );
        assert!(
            run.notices
                .iter()
                .all(|notice| !matches!(notice, FrameNotice::Refused { .. })),
            "{:?}",
            run.notices
        );
    }

    #[test]
    fn veto_precedes_ordered_fire_and_never_blocks_the_manual_trigger() {
        let run = at_every_drive(
            || field(CoreClass::II, vec![cannon()]),
            20_000,
            || {
                vec![
                    beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
                    beat(
                        5_000,
                        vec![
                            intent(
                                RED,
                                IntentKind::Lock {
                                    contact: contact_of(RED, BLUE),
                                },
                                2,
                            ),
                            station("RULE 1 FIRE cannon HOLD WHEN rounds>0", 3),
                            station("RULE 2 FIRE cannon WHEN target=stopped", 4),
                            station("RULE 3 FIRE cannon WHEN lock", 5),
                        ],
                    ),
                    beat(
                        10_000,
                        vec![
                            intent(RED, IntentKind::fire("cannon"), 6),
                            intent(
                                RED,
                                IntentKind::Condition(Arming::Remove {
                                    slot: 1,
                                    channel: Channel::Fire,
                                }),
                                7,
                            ),
                        ],
                    ),
                ]
            },
        );
        assert_eq!(
            run.fired().first().map(|(at, _)| *at),
            Some(10_000),
            "the veto held every automatic shot"
        );
        assert_eq!(
            cond_shots(&run),
            vec![
                (14_000, 2, u64::from(BLUE.0)),
                (18_000, 2, u64::from(BLUE.0))
            ]
        );
        assert!(
            run.fired()
                .iter()
                .any(|(at, event)| *at == 10_000 && event["by"].is_null())
        );
    }

    #[test]
    fn removing_a_veto_rechecks_a_ready_rule_at_the_removal_millisecond() {
        let (mut arena, wall) = locked_duel(100);
        let at = arena.now_ms;
        assert_eq!(
            arena.resolve_arming(RED, &parsed("RULE 1 FIRE cannon HOLD WHEN rounds>0"), at),
            Ok(Some(1))
        );
        assert_eq!(
            arena.resolve_arming(RED, &parsed("RULE 2 FIRE cannon WHEN lock"), at),
            Ok(Some(2))
        );
        assert!(arena.rule_warrant(RED, 2, at).is_err());
        let refused = refused_now(
            &mut arena,
            wall,
            vec![IntentKind::Condition(Arming::Remove {
                slot: 1,
                channel: Channel::Fire,
            })],
        );
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(
            fired_lines(&mut arena),
            vec![(at, u64::from(BLUE.0), Some("cond".into()))]
        );
    }

    #[test]
    fn all_admission_is_atomic_and_explicit_same_weapon_slots_survive_shorthand() {
        let mut arena = field(
            CoreClass::I,
            vec![
                cannon(),
                machine_gun(),
                WeaponSummary {
                    id: "missiles".into(),
                    ..cannon()
                },
            ],
        );
        assert_eq!(
            arena.resolve_arming(RED, &Arming::Arm(cond("cannon", 100, ASPECTS_ANY)), 0),
            Ok(Some(1))
        );
        let before = arena.conditions.clone();
        assert_eq!(
            arena.resolve_arming(RED, &parsed("FIRE ALL HOLD WHEN rounds>0"), 0),
            Err(Refusal::NoSlot)
        );
        assert_eq!(
            arena.conditions, before,
            "no first or second weapon changed after the batch failed"
        );

        let mut arena = field(CoreClass::II, vec![cannon()]);
        assert_eq!(
            arena.resolve_arming(RED, &parsed("RULE 1 FIRE cannon WHEN odds>=20"), 0),
            Ok(Some(1))
        );
        assert_eq!(
            arena.resolve_arming(RED, &parsed("RULE 2 FIRE cannon WHEN odds>=30"), 0),
            Ok(Some(2))
        );
        assert_eq!(
            arena.resolve_arming(RED, &parsed("FIRE cannon WHEN odds>=40"), 0),
            Ok(Some(3))
        );
        assert_eq!(
            arena.resolve_arming(RED, &parsed("FIRE cannon WHEN odds>=50"), 0),
            Ok(Some(3))
        );
        let floors = arena
            .conditions
            .slots(RED)
            .iter()
            .map(|slot| {
                slot.as_ref()
                    .and_then(Rule::fire)
                    .map(|condition| condition.min_permille)
            })
            .collect::<Vec<_>>();
        assert_eq!(floors, vec![Some(200), Some(300), Some(500), None]);
        let before = arena.conditions.clone();
        assert_eq!(
            arena.resolve_arming(
                RED,
                &Arming::Remove {
                    slot: 2,
                    channel: Channel::Sensors
                },
                0
            ),
            Err(Refusal::Channel)
        );
        assert_eq!(arena.conditions, before);
        assert_eq!(
            arena.resolve_arming(
                RED,
                &Arming::Remove {
                    slot: 2,
                    channel: Channel::Fire
                },
                0
            ),
            Ok(None)
        );
        assert!(arena.conditions.slots(RED)[1].is_none());
        assert!(
            arena.conditions.slots(RED)[2].is_some(),
            "removal does not renumber later slots"
        );
    }

    #[test]
    fn named_target_and_motion_predicate_read_the_earned_plot_then_fail_closed() {
        let (mut arena, _) = locked_duel(100);
        assert_eq!(
            arena.resolve_arming(
                RED,
                &parsed("RULE 1 FIRE cannon WHEN target=BA AND target=stopped"),
                arena.now_ms
            ),
            Ok(Some(1))
        );
        assert!(arena.rule_warrant(RED, 1, arena.now_ms).is_ok());
        arena.unit_mut(BLUE).expect("blue").speed_cms = 900;
        assert!(
            arena.rule_warrant(RED, 1, arena.now_ms).is_ok(),
            "unobserved live motion changes no earned predicate"
        );
        let contact = contact_of(RED, BLUE);
        arena
            .contacts
            .get_mut(RED, contact)
            .expect("plot")
            .seen_at
            .speed_cms = 900;
        assert!(
            arena.rule_warrant(RED, 1, arena.now_ms).is_err(),
            "a refreshed plot changes the predicate"
        );
        arena.contacts.get_mut(RED, contact).expect("plot").class = Classification::Unknown;
        assert!(arena.rule_warrant(RED, 1, arena.now_ms).is_err());
        assert_eq!(
            arena.resolve_arming(
                RED,
                &parsed("RULE 1 FIRE cannon WHEN target=BA"),
                arena.now_ms
            ),
            Err(Refusal::Unlawful),
            "classification is checked again at landing"
        );
        arena.contacts.get_mut(RED, contact).expect("plot").class =
            Classification::Classified { team: Team::Blue };
        assert_eq!(
            arena.resolve_arming(
                RED,
                &parsed("RULE 1 FIRE cannon WHEN target nonexistent=red"),
                arena.now_ms
            ),
            Ok(Some(1))
        );
        assert!(
            arena.rule_warrant(RED, 1, arena.now_ms).is_err(),
            "a missing earned section is false"
        );
    }

    #[test]
    fn snapshot_rejects_more_than_six_native_slots_before_accepting_state() {
        let document = malformed_rule_snapshot(|row| {
            row.resize(7, serde_json::Value::Null);
        });
        assert!(matches!(Snapshot::decode(&document, definition()),
            Err(SnapshotError::Malformed(message)) if message == "invalid native rule row"));
    }

    #[test]
    fn snapshot_rejects_more_than_four_native_predicates_before_accepting_state() {
        let document = malformed_rule_snapshot(|row| {
            let predicate = row[0]["predicates"][0].clone();
            row[0]["predicates"] = serde_json::json!(vec![predicate; 5]);
        });
        assert!(matches!(Snapshot::decode(&document, definition()),
            Err(SnapshotError::Malformed(message)) if message == "invalid native rule"));
    }

    #[test]
    fn snapshot_rejects_an_unknown_native_action_instead_of_skipping_it() {
        let document = malformed_rule_snapshot(|row| {
            row[0]["action"] = serde_json::json!("unknown_station_action");
        });
        assert!(matches!(Snapshot::decode(&document, definition()),
            Err(SnapshotError::Malformed(message))
                if message.contains("unknown variant") && message.contains("unknown_station_action")));
    }

    fn malformed_rule_snapshot(
        edit: impl FnOnce(&mut Vec<serde_json::Value>),
    ) -> serde_json::Value {
        use sha2::{Digest, Sha256};

        let mut arena = field(CoreClass::III, vec![cannon()]);
        for command in [
            "RULE 1 FIRE cannon WHEN lock",
            "RULE 2 FIRE cannon HOLD WHEN heat>80%",
            "RULE 3 LOCK nearest WHEN no lock",
        ] {
            arena
                .resolve_arming(RED, &parsed(command), 0)
                .expect("typed rule admitted");
        }
        let mut document = Snapshot::encode(&arena)
            .expect("native rule snapshot")
            .document;
        assert!(
            Snapshot::decode(&document, definition()).is_ok(),
            "the unmodified typed snapshot is valid"
        );
        let mut state: serde_json::Value =
            serde_json::from_str(document["state"].as_str().expect("state string"))
                .expect("state JSON");
        let row = state["conditions"][RED.0.to_string()]
            .as_array_mut()
            .expect("the unit's ordered slots");
        assert_eq!(row.len(), 6);
        edit(row);
        let state = serde_json::to_string(&state).expect("edited state serializes");
        // A correct byte digest must not bypass typed schema/bounds checks.
        // Keep the semantic hash: these cases must fail as Malformed before
        // they can reach a mere HashMismatch refusal.
        let digest: [u8; 32] = Sha256::digest(state.as_bytes()).into();
        document["state_digest"] = serde_json::json!(tacsim::arena::snapshot::digest_text(&digest));
        document["state"] = serde_json::json!(state);
        assert!(Snapshot::head(&document).is_ok());
        document
    }

    #[test]
    fn ordered_rules_snapshot_resume_preserves_slots_events_and_hashes() {
        let mut arena = field(CoreClass::II, vec![cannon()]);
        let _ = replay(
            &mut arena,
            Drive::Every(500),
            5_000,
            &[
                beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
                beat(
                    5_000,
                    vec![
                        station("RULE 1 LOCK nearest WHEN no lock", 2),
                        station("RULE 2 FIRE cannon HOLD WHEN own hull<10%", 3),
                        station("RULE 3 FIRE cannon WHEN target=stopped AND rounds>25%", 4),
                    ],
                ),
            ],
        );
        let snapshot = Snapshot::encode(&arena).expect("ordered rule snapshot");
        let mut resumed =
            Snapshot::decode(&snapshot.document, definition()).expect("ordered rule restore");
        assert_eq!(resumed.conditions, arena.conditions);
        resumed.resume_at(5_000);
        let mut original_bytes = Vec::new();
        let mut resumed_bytes = Vec::new();
        for wall in (5_500..=15_000).step_by(500) {
            arena.clock.pin_tempo_for_tests(Tempo::Quiet);
            resumed.clock.pin_tempo_for_tests(Tempo::Quiet);
            let original = arena.step(wall, &[], &[]);
            let restored = resumed.step(wall, &[], &[]);
            assert_eq!(arena.substep_hashes, resumed.substep_hashes, "at {wall}");
            assert_eq!(arena.conditions, resumed.conditions);
            for chunk in original.ledger_chunks {
                original_bytes.extend_from_slice(&chunk.bytes);
            }
            for chunk in restored.ledger_chunks {
                resumed_bytes.extend_from_slice(&chunk.bytes);
            }
        }
        assert_eq!(
            lines(&mut arena, &mut original_bytes),
            lines(&mut resumed, &mut resumed_bytes)
        );
        assert_eq!(original_bytes, resumed_bytes);
    }
}

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mush_core::{
    CanonicalId, DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId,
};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::conditions::{
    ASPECTS_ANY, Arming, FireCondition, HeldWhy, Rule, STOCK_CONDITION_SLOTS, SlotTarget,
    aspect_mask,
};
use tacsim::arena::intents::{Control, ControlState, Intent, IntentKind, Refusal};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::mind::{MindRunner, MindView, STOCK_THINK_MS, ThinkJob, ThinkResult};
use tacsim::arena::motor::MotorCounters;
use tacsim::arena::perception::{
    Aspect, FrameNotice, FrameOrder, MindObjective, RefusalDetail, SensorProfile, SignatureFactor,
};
use tacsim::arena::resolution::{BandProfile, SectionProfile, UnitProfile};
use tacsim::arena::rng::{draw_contact_id, stagger_ms};
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, Classification, ContactId, Controller,
    CustodyOut, CustodyUnit, EntryZone, Pilot, Power, SeenPose, ShadowContact, Team, TerrainCell,
    TrackedRules, WeaponSummary,
};
use tacsim::arena::step::StepOutput;
use tacsim::arena::weapons::WeaponBand;
use tacsim::arena::{Snapshot, SnapshotError};
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};
use tacsim::match_::cockpit;

// ---------------------------------------------------------------------------
// Fixtures (the `tests/weapons.rs` shapes)
// ---------------------------------------------------------------------------

/// The window: 16 by 16 cells of 1,000 cm.
const SIDE: u16 = 16;
/// The cell size, centimetres.
const CELL_CM: u16 = 1_000;
/// The match seed.
const SEED: u64 = 2_350;
/// The shooter, its wingman, and two targets.
const RED: ObjId = ObjId(4);
const RED2: ObjId = ObjId(5);
const BLUE: ObjId = ObjId(9);
const BLUE2: ObjId = ObjId(12);
/// The pilot `arena_of` seats on the first unit it is handed.
const RED_PILOT: ObjId = ObjId(1_000);
/// The odds every shot of these fixtures resolves at unless a test says
/// otherwise: `p_min = p_max`, so the plot answers exactly this.
const ODDS: u32 = 500;
/// The arena's sensor cadence (`[tacsim.sensors].pass_ms`'s default).
const PASS_MS: i64 = 1_000;

fn dref(kind: &str, id: &str) -> DefinitionRef {
    DefinitionRef::new(
        DefinitionKey::new(
            OwnerId::new("tacsim").expect("a canonical owner"),
            DefinitionKind::new(kind).expect("a canonical kind"),
            DefinitionId::new(id).expect("a canonical id"),
        ),
        1,
        DefinitionDigest::from_bytes([0_u8; 32]),
    )
    .expect("version 1 is not zero")
}

fn rules() -> RulesValue {
    RulesValue::new(dref("schema", "tacsim.arena.test"), serde_json::json!({}))
        .expect("an empty rules blob")
}

/// The smallest M32 unit the arena carries: one hull section.
fn tactical_unit(id: ObjId) -> TacticalUnit {
    // 17b cut M3.1 (plan F6): a Class I core in the hull, through
    // `tacsim::definitions::core::core_component` — the shape `bind` lays
    // down from content — so the hull holds two condition slots and a mind
    // on it thinks at Class I's figures, as it did before cores existed.
    tactical_unit_classed(id, tacsim::arena::core::CoreClass::I)
}

/// [`tactical_unit`] with its hull's core at `class` (17b cut M3.2: the
/// capacity fixtures' Class II line tank and Class III hull).
fn tactical_unit_classed(id: ObjId, class: tacsim::arena::core::CoreClass) -> TacticalUnit {
    let (core, capability) = tacsim::definitions::core::core_component(
        "hull",
        class,
        dref("component", "tacsim.unit.line_tank.core"),
        dref("schema", tacsim::definitions::unit::CORE_RULES_SCHEMA),
    )
    .expect("a core");
    let spec = UnitSpec {
        definition: dref("unit", "tacsim.unit.line_tank"),
        sections: vec![SectionSpec {
            id: SectionId::new("hull").expect("a canonical section"),
            condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
            facets: vec![FacetSpec {
                id: FacetId::new("front").expect("a canonical facet"),
                cells: vec![CellSpec {
                    id: mush_tactics::CellId::new("armor").expect("a cell"),
                    min: 0,
                    max: 10_000,
                    init: 10_000,
                }],
            }],
            slots: Vec::new(),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: 10_000,
                init: 10_000,
            }],
        }],
        unit_cells: Vec::new(),
        components: vec![core],
        capabilities: vec![capability],
        mobility: None,
        rules: rules(),
    };
    TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
}

fn ground() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        elev_cm: 0,
        move_cost: 100,
    }
}

/// The cell centre of `(x, y)`, facing `heading_mdeg`.
fn facing(x: u16, y: u16, heading_mdeg: u32) -> Pose {
    let mut pose = Pose::at(
        i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        0,
        rules(),
    );
    pose.heading_mdeg = heading_mdeg;
    pose
}

fn definition() -> ArenaDefinition {
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.test"),
        terrain: ArenaTerrain::new(
            SIDE,
            SIDE,
            CELL_CM,
            vec![ground(); usize::from(SIDE) * usize::from(SIDE)],
        )
        .expect("the test window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: SIDE - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: SIDE - 2,
                y0: 0,
                x1: SIDE - 1,
                y1: SIDE - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints: tacsim::arena::state::Waypoints::default(),
        modes: Vec::new(),
    }
}

/// A five-cell sight that shares `share_cells` cells.
fn sight(share_cells: u16) -> SensorProfile {
    use mush_atlas::TerrainClass as C;
    SensorProfile {
        component: SmolStr::new("visual_sensor"),
        max_range_cells: 5,
        max_attenuation: 40,
        classify_after_ms: 3_000,
        sees_wrecks: false,
        share_range_cells: share_cells,
        signature: vec![SignatureFactor {
            class: C::Ground,
            moving_permille: 1_000,
            stopped_permille: 1_000,
        }],
    }
}

/// A cannon in the turret: a four-second cycle, forty rounds.
fn cannon() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("cannon"),
        max_range_cm: 250_000,
        cycle_ms: 4_000,
        ammunition: SmolStr::new("tacsim.ammunition.he_105"),
        burst_rounds: 1,
        muzzle_speed_cms: 145_000,
        bands: vec![
            WeaponBand {
                max_range_cm: 150_000,
                dispersion_mrad: 3,
            },
            WeaponBand {
                max_range_cm: 250_000,
                dispersion_mrad: 6,
            },
        ],
        count_per_load: 40,
        damage: 1,
        mount_section: SmolStr::new("turret"),
    }
}

/// A machine gun on the hull: a one-second cycle, two hundred rounds.
fn machine_gun() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("machine_gun"),
        cycle_ms: 1_000,
        count_per_load: 200,
        mount_section: SmolStr::new("hull"),
        ..cannon()
    }
}

/// **A target every band of which presents a silhouette**, so a shot at any
/// aspect resolves on the curve and the clamp `[p_min, p_max]` decides it.
fn profile() -> UnitProfile {
    UnitProfile {
        bands: ["front", "left", "right", "rear"]
            .into_iter()
            .map(|band| BandProfile {
                band: SmolStr::new(band),
                width_cm: 300,
                height_cm: 200,
                sections: vec![(SmolStr::new("hull"), 1)],
            })
            .collect(),
        sections: vec![SectionProfile {
            id: SmolStr::new("hull"),
            facets: vec![SmolStr::new("front")],
            consequences: Vec::new(),
            struck_through: None,
        }],
    }
}

/// One tank.
fn unit(
    id: ObjId,
    team: Team,
    pose: Pose,
    weapons: Vec<WeaponSummary>,
    sensor: Option<SensorProfile>,
) -> CustodyUnit {
    CustodyUnit {
        id,
        team,
        pose,
        mobility: TrackedRules {
            max_speed_cms: 1_000,
            accel_cms2: 200,
            brake_cms2: 400,
            turn_rate_mdeg_s: 30_000,
            reverse_factor_pct: 40,
        },
        sensor_height_cm: 150,
        sensor,
        weapons,
        profile: profile(),
        gunnery: 0,
        controller: Controller::Pilot,
        slot: 0,
        unit: tactical_unit(id),
        seal: tacsim::arena::MindSeal::placeholder(),
    }
    .sealed()
}

/// Power in half a second.
fn match_cfg() -> MatchConfig {
    MatchConfig {
        startup_ms: 500,
        ..MatchConfig::default()
    }
}

/// An arena with a seated pilot per unit (`#1000`, `#1001`, … in the order
/// given) and the odds pinned at [`ODDS`].
fn arena_of(units: Vec<CustodyUnit>) -> Arena {
    let pilots = units
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: unit.id,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition(),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the test arena");
    arena.resolution.p_min = ODDS;
    arena.resolution.p_max = ODDS;
    arena
}

/// **The duel**: red armed with a cannon at (5, 8), blue unarmed four cells
/// east facing `blue_heading` (north shows red its left flank, west its front).
fn duel(blue_heading: u32) -> Arena {
    arena_of(vec![
        unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![cannon()],
            Some(sight(0)),
        ),
        unit(
            BLUE,
            Team::Blue,
            facing(9, 8, blue_heading),
            Vec::new(),
            Some(sight(0)),
        ),
    ])
}

fn intent(unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
    Intent {
        unit,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    }
}

fn contact_of(viewer: ObjId, target: ObjId) -> ContactId {
    draw_contact_id(SEED, viewer, target)
}

/// A slot's terms on the lock.
fn cond(weapon: &str, min_permille: u16, aspects: u8) -> FireCondition {
    FireCondition {
        weapon: SmolStr::new(weapon),
        target: SlotTarget::Lock,
        min_permille,
        aspects,
        min_rounds_left: 0,
        min_range_m: 0,
        max_range_m: u32::MAX,
    }
}

/// An arming of `cond`.
fn arm(cond: FireCondition) -> IntentKind {
    IntentKind::Condition(Arming::Arm(cond))
}

// ---------------------------------------------------------------------------
// The driver
// ---------------------------------------------------------------------------

/// How a run is driven: a fixed wall cadence, or the task's own wakes (each
/// pulse ends at `StepOutput::next_due_ms`, or at the next beat when that is
/// sooner — the world's beat is what wakes the task).
#[derive(Clone, Copy, Debug)]
enum Drive {
    Every(i64),
    Task,
}

/// The four drives every cross-tempo proof here runs.
const DRIVES: [Drive; 4] = [
    Drive::Every(5_000),
    Drive::Every(1_000),
    Drive::Every(500),
    Drive::Task,
];

/// One scripted beat at an arena millisecond on every drive's lattice: its
/// controls and its intents, delivered in a pulse of no wall time.
struct Beat {
    t_ms: i64,
    controls: Vec<Control>,
    intents: Vec<Intent>,
}

fn beat(t_ms: i64, intents: Vec<Intent>) -> Beat {
    Beat {
        t_ms,
        controls: Vec::new(),
        intents,
    }
}

/// What a run produced.
#[derive(Debug)]
struct Run {
    /// Every line: `(t_ms, kind and fields as JSON, hash)`.
    events: Vec<(i64, String, u64)>,
    /// Every substep's hash.
    hashes: Vec<u64>,
    /// The counters, summed.
    counters: MotorCounters,
    /// Every notice the red pilot was sent.
    notices: Vec<FrameNotice>,
    /// **17b cut M1b.4**: every notice EVERY pilot was sent, with the arena
    /// millisecond its frame went out at and the pilot it went to.
    told: Vec<(i64, ObjId, FrameNotice)>,
    /// How many pulses it took.
    pulses: u32,
}

impl Run {
    /// The `fired` lines, parsed.
    fn fired(&self) -> Vec<(i64, serde_json::Value)> {
        self.events
            .iter()
            .filter_map(|(t_ms, json, _)| {
                let value: serde_json::Value = serde_json::from_str(json).ok()?;
                (value.get("kind")? == "fired").then_some((*t_ms, value))
            })
            .collect()
    }
}

fn absorb(out: &StepOutput, bytes: &mut Vec<u8>, notices: &mut Vec<FrameNotice>) {
    for chunk in &out.ledger_chunks {
        bytes.extend_from_slice(&chunk.bytes);
    }
    for (pilot, frame) in &out.frames {
        if *pilot == RED_PILOT {
            notices.extend(frame.notices.iter().cloned());
        }
    }
}

/// Every pilot's notices of one pulse, at the arena millisecond it ended (17b
/// cut M1b.4).
fn told_of(out: &StepOutput, now_ms: i64, told: &mut Vec<(i64, ObjId, FrameNotice)>) {
    for (pilot, frame) in &out.frames {
        told.extend(
            frame
                .notices
                .iter()
                .map(|notice| (now_ms, *pilot, notice.clone())),
        );
    }
}

fn lines(arena: &mut Arena, bytes: &mut Vec<u8>) -> Vec<LedgerEvent> {
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    String::from_utf8(bytes.clone())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .collect()
}

/// **Drive `arena` through `script` under `drive` to `until_ms`**, the tempo
/// pinned `Quiet` (a tempo line is a per-pulse artefact of the cadence), each
/// beat in a zero-`dt` pulse of its own at its own millisecond — the
/// `tests/weapons.rs` (h) driver's rule, so a discrete order resolves at the
/// millisecond the script names at every drive.
fn replay(arena: &mut Arena, drive: Drive, until_ms: i64, script: &[Beat]) -> Run {
    let mut bytes: Vec<u8> = Vec::new();
    let mut notices: Vec<FrameNotice> = Vec::new();
    let mut told: Vec<(i64, ObjId, FrameNotice)> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = MotorCounters::default();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut next_due = 0_i64;
    // The opening pulse anchors arena time to wall `0` (the clock's first
    // step); a script with no beat at `0` gets an empty one, so a wall reading
    // is an arena millisecond on every drive.
    if script.first().is_none_or(|beat| beat.t_ms > 0) {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes, &mut notices);
        told_of(&out, arena.now_ms, &mut told);
        counters.absorb(&out.motor);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        next_due = out.next_due_ms;
    }
    loop {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            next += 1;
            let out = arena.step(wall, &beat.controls, &beat.intents);
            absorb(&out, &mut bytes, &mut notices);
            told_of(&out, arena.now_ms, &mut told);
            counters.absorb(&out.motor);
            hashes.extend(arena.substep_hashes.iter().copied());
            pulses += 1;
            next_due = out.next_due_ms;
        }
        if arena.now_ms >= until_ms {
            break;
        }
        let beat_at = script.get(next).map_or(i64::MAX, |beat| beat.t_ms);
        wall = match drive {
            Drive::Every(cadence_ms) => wall + cadence_ms,
            Drive::Task => next_due.max(wall + 1).min(beat_at).min(until_ms),
        };
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes, &mut notices);
        told_of(&out, arena.now_ms, &mut told);
        counters.absorb(&out.motor);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        next_due = out.next_due_ms;
        if out.ended.is_some() {
            break;
        }
    }
    let events = lines(arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    Run {
        events,
        hashes,
        counters,
        notices,
        told,
        pulses,
    }
}

/// Run `make()` under every drive and assert one event list and one hash
/// sequence; answers the first run.
fn at_every_drive(make: impl Fn() -> Arena, until_ms: i64, script: impl Fn() -> Vec<Beat>) -> Run {
    let mut runs: Vec<(Drive, Run)> = Vec::new();
    for drive in DRIVES {
        let mut arena = make();
        let run = replay(&mut arena, drive, until_ms, &script());
        runs.push((drive, run));
    }
    let (_, first) = runs.remove(0);
    for (drive, run) in &runs {
        if let Some(at) = (0..run.events.len().max(first.events.len()))
            .find(|at| run.events.get(*at) != first.events.get(*at))
        {
            panic!(
                "the event list differs at {drive:?}, line {at}: {:?} against {:?}",
                run.events.get(at),
                first.events.get(at)
            );
        }
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence differs at {drive:?}"
        );
        assert_eq!(
            run.counters.cond_fires, first.counters.cond_fires,
            "the slot shots differ at {drive:?}"
        );
        assert_ne!(run.pulses, first.pulses, "{drive:?} ran the same pulses");
    }
    first
}

/// The `fired` lines `by: cond`, as `(t_ms, slot, target)`.
fn cond_shots(run: &Run) -> Vec<(i64, u64, u64)> {
    run.fired()
        .into_iter()
        .filter(|(_, line)| line.get("by").and_then(serde_json::Value::as_str) == Some("cond"))
        .map(|(t_ms, line)| {
            (
                t_ms,
                line.get("slot")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                line.get("target")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. The mandatory cross-tempo proof
// ---------------------------------------------------------------------------

/// The duel's script: `STARTUP` at 0; at 5,000 — the contact classified at
/// about 4,000 — `LOCK` and then the arming, typed in that order.
fn armed_at_5000(cond: FireCondition) -> Vec<Beat> {
    let contact = contact_of(RED, BLUE);
    vec![
        beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
        beat(
            5_000,
            vec![
                intent(RED, IntentKind::Lock { contact }, 2),
                intent(RED, arm(cond), 3),
            ],
        ),
    ]
}

/// **The mandatory cross-tempo proof** (17b cut M1b.1; 16b §11; doc 17 §5,
/// D-UM-16): a seat's arming resolves at the END of the pulse that accepted it
/// — 5,000, the replay's zero-`dt` beat — and its `Arming` evaluation fires at
/// once (`by: cond`, `slot: 1`); the slot's second shot lands at exactly
/// `5,000 + cycle_ms` = 9,000, the `WeaponReady` millisecond, which at the
/// 5,000 ms cadence is MID-PULSE (the proof), and not at the 10,000 boundary
/// after it. One event list and one hash sequence at 5,000 / 1,000 / 500 ms and
/// on the task's own wakes; two slot shots; the rounds conserved.
#[test]
fn m1b1_a_conditioned_shot_resolves_on_the_weapon_ready_millisecond_at_three_cadences() {
    let run = at_every_drive(
        || duel(0),
        10_000,
        || armed_at_5000(cond("cannon", 100, ASPECTS_ANY)),
    );
    let cycle = i64::from(cannon().cycle_ms);
    assert_eq!(
        cond_shots(&run),
        vec![
            (5_000, 1, u64::from(BLUE.0)),
            (5_000 + cycle, 1, u64::from(BLUE.0))
        ],
        "the arming's own shot at 5,000 and the WeaponReady's at 9,000"
    );
    assert_eq!(run.counters.cond_fires, 2);
    assert!(
        run.notices
            .iter()
            .all(|notice| !matches!(notice, FrameNotice::Refused { .. })),
        "nothing the pilot typed was refused: {:?}",
        run.notices
    );
    // Every shot of the run was the slot's.
    assert_eq!(run.fired().len(), 2);
    let rounds: u64 = run
        .fired()
        .iter()
        .filter_map(|(_, line)| line.get("rounds").and_then(serde_json::Value::as_u64))
        .sum();
    let mut arena = duel(0);
    let _ = replay(
        &mut arena,
        Drive::Every(500),
        10_000,
        &armed_at_5000(cond("cannon", 100, ASPECTS_ANY)),
    );
    assert_eq!(
        u64::from(
            arena
                .unit(RED)
                .map_or(0, |state| state.weapon_ammo("cannon"))
        ) + rounds,
        40,
        "rounds fired plus rounds left is the load"
    );
    assert_eq!(
        arena.conditions.slot_of(RED, "cannon"),
        Some(1),
        "the slot stands"
    );
}

// ---------------------------------------------------------------------------
// 2. A lock and an arming in one pulse
// ---------------------------------------------------------------------------

/// Red, blue and a second blue at (9, 10), all classified and red locked on
/// blue, online, at 5,000; answers the arena and the wall it stands at.
fn two_targets_locked_on_the_first() -> (Arena, i64) {
    let mut arena = arena_of(vec![
        unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![cannon()],
            Some(sight(0)),
        ),
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
        unit(BLUE2, Team::Blue, facing(9, 10, 0), Vec::new(), None),
    ]);
    let mut wall = 0_i64;
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(wall, &[], &[intent(RED, IntentKind::Startup, 1)]);
    while arena.now_ms < 5_000 {
        wall += 500;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
    }
    for target in [BLUE, BLUE2] {
        assert!(
            arena
                .contacts
                .get(RED, contact_of(RED, target))
                .is_some_and(|held| matches!(held.class, Classification::Classified { .. })),
            "red holds #{} classified",
            target.0
        );
    }
    let _ = arena.step(
        wall,
        &[],
        &[intent(
            RED,
            IntentKind::Lock {
                contact: contact_of(RED, BLUE),
            },
            2,
        )],
    );
    assert_eq!(
        arena.unit(RED).and_then(|state| state.lock),
        Some(contact_of(RED, BLUE))
    );
    (arena, wall)
}

/// The `fired` lines of `arena`'s ledger so far, as `(t_ms, target, by)`.
fn fired_lines(arena: &mut Arena) -> Vec<(i64, u64, Option<String>)> {
    let mut bytes = Vec::new();
    lines(arena, &mut bytes)
        .into_iter()
        .filter_map(|event| {
            let value = serde_json::to_value(&event.kind).ok()?;
            (value.get("kind")? == "fired").then(|| {
                (
                    event.t_ms,
                    value
                        .get("target")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    value
                        .get("by")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                )
            })
        })
        .collect()
}

/// **A `LOCK` and an arming typed in ONE non-zero-`dt` pulse land in that
/// order** (17b cut M1b.1; 17b's adversarial F7): red locked on BA1 types
/// `LOCK BA2` then `FIRE cannon WHEN …` inside one 5,000 ms pulse at the
/// `Quiet` tempo. Both are discrete, resolved at the pulse's end in arrival
/// order, so the arming's first evaluation reads the NEW lock: the slot's
/// first shot is at BA2 and no shot ever names BA1. And the reverse — a typed
/// `FIRE` then the arming in one pulse: the typed shot fires first, and the
/// slot's evaluation at the arming finds the gun cycling (no second shot at
/// that millisecond; the slot's own fires at the `WeaponReady` after it).
#[test]
fn m1b1_a_lock_and_an_arming_typed_in_one_pulse_land_in_that_order() {
    let (mut arena, wall) = two_targets_locked_on_the_first();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(
        wall + 5_000,
        &[],
        &[
            intent(
                RED,
                IntentKind::Lock {
                    contact: contact_of(RED, BLUE2),
                },
                3,
            ),
            intent(RED, arm(cond("cannon", 100, ASPECTS_ANY)), 4),
        ],
    );
    let at = arena.now_ms;
    assert_eq!(at, 10_000, "one 5,000 ms pulse");
    let mut wall_now = wall + 5_000;
    while arena.now_ms < 20_000 {
        wall_now += 500;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall_now, &[], &[]);
    }
    let fired = fired_lines(&mut arena);
    assert_eq!(
        fired.first(),
        Some(&(at, u64::from(BLUE2.0), Some("cond".to_owned()))),
        "the slot's first shot is at the new lock, at the pulse's end"
    );
    assert!(
        fired
            .iter()
            .all(|(_, target, _)| *target == u64::from(BLUE2.0)),
        "no shot ever names the old lock: {fired:?}"
    );

    // The reverse: a typed FIRE then the arming in one pulse.
    let (mut arena, wall) = two_targets_locked_on_the_first();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(
        wall + 5_000,
        &[],
        &[
            intent(RED, IntentKind::fire("cannon"), 3),
            intent(RED, arm(cond("cannon", 100, ASPECTS_ANY)), 4),
        ],
    );
    let at = arena.now_ms;
    let mut wall_now = wall + 5_000;
    while arena.now_ms < 16_000 {
        wall_now += 500;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall_now, &[], &[]);
    }
    let fired = fired_lines(&mut arena);
    assert_eq!(
        fired,
        vec![
            (at, u64::from(BLUE.0), None),
            (
                at + i64::from(cannon().cycle_ms),
                u64::from(BLUE.0),
                Some("cond".to_owned())
            ),
        ],
        "the typed shot first; the slot found the gun cycling and fired at its WeaponReady"
    );
}

// ---------------------------------------------------------------------------
// 3. The production pins
// ---------------------------------------------------------------------------

/// **The production hash and line hashes do not move** (17b cut M1b.1; the
/// lead's binding: a unit with no slot folds nothing new). The pins live where
/// they were recorded and are re-run by the gate, never re-recorded here: M1a.1's
/// two substep pins (`tests/reflex.rs`' `m1a1_the_moved_motor_folds_the_same_bytes`,
/// `tests/motor.rs`' `m1a1_the_shipped_duel_hashes_as_before`),
/// `tests/minds.rs`' `m1a5_the_production_hash_does_not_move`, and M1b.0's
/// three `m1b0_the_production_line_hashes_do_not_move` (`tests/weapons.rs`,
/// `tests/motor.rs`, `tests/reflex.rs`) — a trigger that TAKES the lazy action
/// hash early shows in their next action line.
///
/// What this test adds, end to end through `Arena::step`: the seat duel,
/// stepped every 500 ms through a startup, a lock, two typed shots, their
/// `WeaponReady`s and red's passes (every trigger arm but a share's and a
/// landing's), with no slot armed — after EVERY pulse the slot table and
/// `held_said` hold no row (so item 3d and item 3e fold no byte at any pulse),
/// and the same duel on an arena whose captured floor is 50 hashes equal to the
/// floor-100 one after every pulse and journals the same lines (the floor is
/// never folded); no slot shot and no held slot is counted; the seat's two
/// `fired` lines carry no `by` or `slot`. What it cannot see from outside the
/// library is a trigger that only SETS `ActionHash::Pending` for a hull with no
/// slot (`action_hash` is crate-private): `conditions.rs`' in-source
/// `m1b1_a_hull_with_no_slot_takes_no_hash_at_any_trigger` pins the
/// evaluator's own early return for each `ConditionTrigger`, and every call
/// site calls the evaluator with no hash opened of its own.
#[test]
fn m1b1_the_production_hash_and_line_hashes_do_not_move() {
    let contact = contact_of(RED, BLUE);
    let beats = [
        (0_i64, vec![intent(RED, IntentKind::Startup, 1)]),
        (
            5_000,
            vec![
                intent(RED, IntentKind::Lock { contact }, 2),
                intent(RED, IntentKind::fire("cannon"), 3),
            ],
        ),
        (10_000, vec![intent(RED, IntentKind::fire("cannon"), 4)]),
    ];
    let mut arenas = [duel(0), duel(0)];
    arenas[1].odds_floor_permille = 50;
    let mut counters = MotorCounters::default();
    let mut bytes: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    while wall <= 20_000 {
        let delivered: &[Intent] = beats
            .iter()
            .find(|(at, _)| *at == wall)
            .map_or(&[], |(_, intents)| intents.as_slice());
        for (at, arena) in arenas.iter_mut().enumerate() {
            arena.clock.pin_tempo_for_tests(Tempo::Quiet);
            let out = arena.step(wall, &[], delivered);
            for chunk in &out.ledger_chunks {
                bytes[at].extend_from_slice(&chunk.bytes);
            }
            if at == 0 {
                counters.absorb(&out.motor);
            }
            assert!(
                arena.conditions.rows().next().is_none(),
                "a slot row at wall {wall}"
            );
            assert!(arena.held_said.is_empty(), "a held row at wall {wall}");
        }
        let [hundred, fifty] = &arenas;
        assert_eq!(
            tacsim::arena::state_hash(fifty),
            tacsim::arena::state_hash(hundred),
            "the floor folded at wall {wall}"
        );
        assert_eq!(fifty.substep_hashes, hundred.substep_hashes);
        pulses += 1;
        wall += 500;
    }
    assert_eq!(pulses, 41);
    assert_eq!(counters.cond_fires, 0);
    assert_eq!(counters.cond_held, 0);
    let [hundred, fifty] = &mut arenas;
    let [bytes_hundred, bytes_fifty] = &mut bytes;
    let lines_hundred = lines(hundred, bytes_hundred);
    assert_eq!(lines_hundred, lines(fifty, bytes_fifty), "the same lines");
    let fired: Vec<serde_json::Value> = lines_hundred
        .iter()
        .filter_map(|event| serde_json::to_value(&event.kind).ok())
        .filter(|value| value.get("kind").is_some_and(|kind| kind == "fired"))
        .collect();
    assert_eq!(fired.len(), 2, "the seat's two shots: {fired:?}");
    assert!(
        fired
            .iter()
            .all(|line| line.get("by").is_none() && line.get("slot").is_none()),
        "a seat's `fired` line carries no attribution"
    );
}

// ---------------------------------------------------------------------------
// 4. The passes, the share, the startup
// ---------------------------------------------------------------------------

/// Whether `t_ms` is one of `unit`'s sensor-pass milliseconds.
fn is_pass_of(unit: ObjId, t_ms: i64) -> bool {
    (t_ms - stagger_ms(SEED, unit, PASS_MS)).rem_euclid(PASS_MS) == 0
}

/// **A pass that turns the plot to a flank fires the slot at the pass's
/// millisecond** (17b cut M1b.1; doc 17 §5, 17b §1(d)), at three cadences and
/// on the task's wakes:
///
/// * red's slot fires only at `left|right`; blue faces red (its front) and
///   nothing fires through two cycles; at 15,000 blue turns north and the
///   first slot shot is at one of RED's OWN `SensorPass` milliseconds after
///   the turn — the pass that refreshed the plot;
/// * a sightless red fed by a wingman's share fires at the wingman's pass —
///   the `Share`'s millisecond — never at one of its own;
/// * a slot armed while red is still `Starting` fires at the
///   `StartupComplete` millisecond, 500 ms after the startup.
#[test]
fn m1b1_a_slot_fires_when_a_pass_turns_the_plot_to_a_flank() {
    let flank = aspect_mask(&[Aspect::Left, Aspect::Right]);
    let turn = |unit_seq: u64| {
        beat(
            15_000,
            vec![intent(BLUE, IntentKind::Heading { mdeg: 0 }, unit_seq)],
        )
    };
    // (a) Red's own pass.
    let run = at_every_drive(
        || duel(270_000),
        25_000,
        || {
            let mut script = armed_at_5000(cond("cannon", 100, flank));
            if let Some(first) = script.first_mut() {
                first.intents.push(intent(BLUE, IntentKind::Startup, 9));
            }
            script.push(turn(10));
            script
        },
    );
    let shots = cond_shots(&run);
    let Some(&(first, _, _)) = shots.first() else {
        panic!("the slot never fired: {:?}", run.counters);
    };
    assert!(first > 15_000, "nothing fired while blue showed its front");
    assert!(
        is_pass_of(RED, first),
        "the slot fired at {first}, which is not one of red's passes"
    );
    assert!(
        run.counters.cond_held > 0,
        "it held for the aspect before the turn"
    );

    // (b) A sightless red, fed by a wingman's share.
    let wing = || {
        arena_of(vec![
            unit(RED, Team::Red, facing(5, 8, 90_000), vec![cannon()], None),
            unit(BLUE, Team::Blue, facing(9, 8, 270_000), Vec::new(), None),
            unit(
                RED2,
                Team::Red,
                facing(5, 7, 90_000),
                Vec::new(),
                Some(sight(5)),
            ),
        ])
    };
    let run = at_every_drive(wing, 25_000, || {
        let mut script = armed_at_5000(cond("cannon", 100, flank));
        if let Some(first) = script.first_mut() {
            first.intents.push(intent(BLUE, IntentKind::Startup, 9));
        }
        script.push(turn(10));
        script
    });
    let shots = cond_shots(&run);
    let Some(&(first, _, _)) = shots.first() else {
        panic!("the shared slot never fired: {:?}", run.counters);
    };
    assert!(first > 15_000);
    assert!(
        is_pass_of(RED2, first),
        "the slot fired at {first}, which is not the wingman's pass (its share)"
    );
    assert!(
        !is_pass_of(RED, first),
        "the two staggers coincide; pick other ids"
    );

    // (c) A slot armed while starting fires when power comes up.
    let contact = contact_of(RED, BLUE);
    let run = at_every_drive(
        || duel(0),
        10_000,
        || {
            vec![beat(
                5_000,
                vec![
                    intent(RED, IntentKind::Startup, 1),
                    intent(RED, IntentKind::Lock { contact }, 2),
                    intent(RED, arm(cond("cannon", 100, ASPECTS_ANY)), 3),
                ],
            )]
        },
    );
    assert_eq!(
        cond_shots(&run).first().map(|(t, _, _)| *t),
        Some(5_000 + i64::from(match_cfg().startup_ms)),
        "the slot fired at the StartupComplete millisecond"
    );
}

// ---------------------------------------------------------------------------
// 5. The floor at the landing
// ---------------------------------------------------------------------------

/// Red online and locked on blue at 5,000, answering the arena and its wall.
fn locked_duel(floor: u32) -> (Arena, i64) {
    let mut arena = duel(0);
    arena.odds_floor_permille = floor;
    let run = replay(
        &mut arena,
        Drive::Every(500),
        5_000,
        &[
            beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
            beat(
                5_000,
                vec![intent(
                    RED,
                    IntentKind::Lock {
                        contact: contact_of(RED, BLUE),
                    },
                    2,
                )],
            ),
        ],
    );
    assert_eq!(run.fired().len(), 0);
    // The replay delivered the 5,000 beat before it stopped.
    let wall = arena.now_ms;
    assert_eq!(wall, 5_000);
    assert_eq!(
        arena.unit(RED).and_then(|state| state.lock),
        Some(contact_of(RED, BLUE))
    );
    (arena, wall)
}

/// The red pilot's `refused` notices on one zero-`dt` pulse delivering
/// `kinds`, each with the frame it rode.
fn refused_now(
    arena: &mut Arena,
    wall: i64,
    kinds: Vec<IntentKind>,
) -> Vec<(FrameNotice, tacsim::arena::step::Frame)> {
    told_now(arena, wall, kinds)
        .into_iter()
        .filter(|(notice, _)| matches!(notice, FrameNotice::Refused { .. }))
        .collect()
}

/// **EVERY notice the red pilot is sent on one zero-`dt` pulse delivering
/// `kinds`** (17b cut M3.2), each with the frame it rode — not the `refused`
/// ones alone, so a transcript built from it (`heard`) holds any other line
/// the pulse told the pilot.
fn told_now(
    arena: &mut Arena,
    wall: i64,
    kinds: Vec<IntentKind>,
) -> Vec<(FrameNotice, tacsim::arena::step::Frame)> {
    let intents: Vec<Intent> = kinds
        .into_iter()
        .enumerate()
        .map(|(n, kind)| intent(RED, kind, 100 + u64::try_from(n).unwrap_or(0)))
        .collect();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let out = arena.step(wall, &[], &intents);
    out.frames
        .iter()
        .filter(|(pilot, _)| *pilot == RED_PILOT)
        .flat_map(|(_, frame)| {
            frame
                .notices
                .iter()
                .map(|notice| (notice.clone(), frame.clone()))
        })
        .collect()
}

/// Every line of `told`, as the cockpit's own `notice_line` renders it.
fn heard(told: &[(FrameNotice, tacsim::arena::step::Frame)]) -> Vec<String> {
    told.iter().map(sentence).collect()
}

/// The lines of `heard` that name the missiles.
fn about_missiles(heard: &[String]) -> Vec<String> {
    heard
        .iter()
        .filter(|line| line.contains("missiles"))
        .cloned()
        .collect()
}

/// The sentence a notice reads as, on the frame it rode.
fn sentence(refused: &(FrameNotice, tacsim::arena::step::Frame)) -> String {
    cockpit::notice_line(&refused.1, &refused.0, &[])
}

/// **The floor at the landing of a seat's arming** (17b cut M1b.1; doc 17 §5,
/// D-UM-17 as amended; refuse-not-clamp): on a floor-100 arena an arming at
/// 50 per mille is refused `unlawful` from phase 4 with the weapon and the
/// floor named — `FIRE cannon refused. The Compact floor is 10%; a condition
/// may not ask for less.` — and arms nothing; at 100 it lands as S1. An arena
/// whose captured floor is 50 takes 50 and refuses 40 (the arena's floor, not
/// a constant). M5rules admits an earned classified `Contact` target and
/// refuses an unheld id with the fallback sentence. A slot armed AT the floor
/// fires at the floor's odds and holds under them.
#[test]
fn m1b1_an_arming_under_the_floor_is_refused_and_a_contact_must_be_earned() {
    let (mut arena, wall) = locked_duel(100);
    // Hold the slot back from firing: no aspect at all would be unlawful? No —
    // the aspect term is not a law term, so a front-only slot on a flank holds.
    let front = aspect_mask(&[Aspect::Front]);
    let refused = refused_now(&mut arena, wall, vec![arm(cond("cannon", 50, front))]);
    let [notice] = refused.as_slice() else {
        panic!("one refusal: {refused:?}");
    };
    let FrameNotice::Refused {
        order,
        refusal,
        weapon,
        detail,
    } = &notice.0
    else {
        panic!("a refusal");
    };
    assert_eq!(
        (order.as_str(), *refusal, weapon.as_deref(), *detail),
        (
            "fire",
            Refusal::Unlawful,
            Some("cannon"),
            RefusalDetail::Floor { permille: 100 }
        )
    );
    assert_eq!(
        sentence(notice),
        "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less."
    );
    assert!(arena.conditions.is_empty_for(RED), "no slot armed");
    let refused = refused_now(&mut arena, wall, vec![arm(cond("cannon", 100, front))]);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));

    // The captured floor, not a constant.
    let (mut arena, wall) = locked_duel(50);
    assert!(refused_now(&mut arena, wall, vec![arm(cond("cannon", 50, front))]).is_empty());
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));
    let refused = refused_now(&mut arena, wall, vec![arm(cond("cannon", 40, front))]);
    assert!(
        matches!(
            refused.as_slice(),
            [(
                FrameNotice::Refused {
                    refusal: Refusal::Unlawful,
                    detail: RefusalDetail::Floor { permille: 50 },
                    ..
                },
                _
            )]
        ),
        "{refused:?}"
    );
    assert_eq!(
        sentence(&refused[0]),
        "FIRE cannon refused. The Compact floor is 5%; a condition may not ask for less."
    );

    // M5rules deliberately opens classified contact citations. A citation
    // filters the current lock; it never retargets the Fire station itself.
    let (mut arena, wall) = locked_duel(100);
    let named = FireCondition {
        target: SlotTarget::Contact {
            id: contact_of(RED, BLUE),
        },
        ..cond("cannon", 100, front)
    };
    assert!(refused_now(&mut arena, wall, vec![arm(named.clone())]).is_empty());
    assert_eq!(
        arena
            .conditions
            .slots(RED)
            .first()
            .and_then(Option::as_ref)
            .and_then(Rule::fire),
        Some(&named)
    );
    let (mut arena, wall) = locked_duel(100);
    {
        let id = ContactId(0x0bad_cafe);
        let refused = refused_now(
            &mut arena,
            wall,
            vec![arm(FireCondition {
                target: SlotTarget::Contact { id },
                ..cond("cannon", 100, ASPECTS_ANY)
            })],
        );
        let [notice] = refused.as_slice() else {
            panic!("one refusal: {refused:?}");
        };
        assert!(matches!(
            &notice.0,
            FrameNotice::Refused {
                refusal: Refusal::Unlawful,
                detail: RefusalDetail::None,
                ..
            }
        ));
        assert_eq!(
            sentence(notice),
            "FIRE cannon refused. That rule is not lawful for this unit."
        );
    }
    assert!(arena.conditions.is_empty_for(RED));

    // At the floor's odds: fires at 100, holds at 90.
    for (odds, fires) in [(100_u32, true), (90, false)] {
        let (mut arena, wall) = locked_duel(100);
        arena.resolution.p_min = odds;
        arena.resolution.p_max = odds;
        let _ = refused_now(
            &mut arena,
            wall,
            vec![arm(cond("cannon", 100, ASPECTS_ANY))],
        );
        let fired = fired_lines(&mut arena);
        assert_eq!(!fired.is_empty(), fires, "at {odds} per mille: {fired:?}");
        let warrant = arena
            .conditions
            .slots(RED)
            .first()
            .and_then(Option::as_ref)
            .and_then(Rule::fire)
            .map(|slot| arena.warrant(RED, slot, arena.now_ms));
        if !fires {
            assert!(
                matches!(
                    warrant,
                    Some(Err(tacsim::arena::conditions::Held::Why(HeldWhy::Odds {
                        permille: 90,
                        floor: 100
                    })))
                ),
                "{warrant:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 7. The target rules
// ---------------------------------------------------------------------------

/// **A slot is idle on an unclassified lock and fires only on its lock** (17b
/// cut M1b.1; D-UM-17 extended to the default target): a `Lock` slot armed on
/// an `X` row never fires through the classify window and fires at the first
/// evaluation after the row is told apart — red's own pass that classified it;
/// a `BestHostile` slot whose lock is a friendly never fires, and fires once
/// the lock is a classified hostile; the warrant answers the first failing
/// term in order (rounds, the ROUNDED range, aspect, odds) and never looks at
/// readiness.
#[test]
fn m1b1_a_slot_is_idle_on_an_unclassified_lock_and_fires_only_on_its_lock() {
    let mut arena = duel(0);
    let mut wall = 0_i64;
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(wall, &[], &[intent(RED, IntentKind::Startup, 1)]);
    let contact = contact_of(RED, BLUE);
    while arena.contacts.get(RED, contact).is_none() {
        wall += 250;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
    }
    assert_eq!(
        arena.contacts.get(RED, contact).map(|held| held.class),
        Some(Classification::Unknown)
    );
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(
        wall,
        &[],
        &[
            intent(RED, IntentKind::Lock { contact }, 2),
            intent(RED, arm(cond("cannon", 100, ASPECTS_ANY)), 3),
        ],
    );
    assert_eq!(arena.unit(RED).and_then(|state| state.lock), Some(contact));
    assert!(fired_lines(&mut arena).is_empty(), "idle on an X row");
    while arena
        .contacts
        .get(RED, contact)
        .is_some_and(|held| held.class == Classification::Unknown)
    {
        wall += 250;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
        assert!(
            arena
                .contacts
                .get(RED, contact)
                .is_none_or(|held| held.class != Classification::Unknown)
                || fired_lines(&mut arena).is_empty(),
            "fired on an unclassified lock"
        );
    }
    let classified_at = arena
        .contacts
        .get(RED, contact)
        .map(|held| held.last_seen_ms)
        .unwrap_or_default();
    let fired = fired_lines(&mut arena);
    assert_eq!(
        fired.first().map(|(t, _, by)| (*t, by.clone())),
        Some((classified_at, Some("cond".to_owned()))),
        "fired at the pass that told it apart"
    );
    assert!(is_pass_of(RED, classified_at));

    // BestHostile: a friendly lock never fires; a hostile one does.
    let mut arena = arena_of(vec![
        unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![cannon()],
            Some(sight(0)),
        ),
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
        unit(RED2, Team::Red, facing(5, 10, 0), Vec::new(), None),
    ]);
    for (target, team) in [(BLUE, Team::Blue), (RED2, Team::Red)] {
        let plot = arena
            .unit(target)
            .map(SeenPose::of)
            .expect("a fielded unit");
        arena.contacts.insert(
            RED,
            contact_of(RED, target),
            ShadowContact::sighted(target, Classification::Classified { team }, 0, plot),
        );
    }
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
    }
    let best = FireCondition {
        target: SlotTarget::BestHostile,
        ..cond("cannon", 100, ASPECTS_ANY)
    };
    assert_eq!(
        arena.resolve_lock(RED, contact_of(RED, RED2), 0),
        Ok(()),
        "a lock on the friendly"
    );
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(best.clone()), 0),
        Ok(Some(1))
    );
    assert_eq!(
        arena.warrant(RED, &best, 0),
        Err(tacsim::arena::conditions::Held::Idle),
        "a friendly lock is no best hostile"
    );
    let lock = |arena: &mut Arena, target: ObjId| {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(
            0,
            &[],
            &[intent(
                RED,
                IntentKind::Lock {
                    contact: contact_of(RED, target),
                },
                5,
            )],
        );
    };
    lock(&mut arena, RED2);
    assert!(fired_lines(&mut arena).is_empty());
    lock(&mut arena, BLUE);
    assert_eq!(
        fired_lines(&mut arena),
        vec![(0, u64::from(BLUE.0), Some("cond".to_owned()))],
        "the Lock trigger fired the best-hostile slot on the hostile lock"
    );

    // The warrant, term by term, in order; readiness is never a term.
    let (arena, _) = locked_duel(100);
    let at = arena.now_ms;
    let held = |slot: FireCondition| arena.warrant(RED, &slot, at);
    let failing = FireCondition {
        min_rounds_left: 40,
        min_range_m: 1_000,
        aspects: aspect_mask(&[Aspect::Front]),
        min_permille: 900,
        ..cond("cannon", 100, ASPECTS_ANY)
    };
    use tacsim::arena::conditions::Held;
    assert_eq!(
        held(failing.clone()),
        Err(Held::Why(HeldWhy::Rounds {
            rounds: 40,
            keeps: 40
        }))
    );
    let failing = FireCondition {
        min_rounds_left: 0,
        ..failing
    };
    let Err(Held::Why(HeldWhy::Range {
        range_m,
        min_m,
        max_m,
        ..
    })) = held(failing.clone())
    else {
        panic!("the range term");
    };
    // The duel's plot stands four cells (40 m) off: on the lock's 10 m step
    // the rounded figure is the raw one, so this pins the term's place only.
    assert_eq!((range_m, min_m, max_m), (40, 1_000, u32::MAX));
    // **The ROUNDED range**: the plot moved to 47 m and stamped at `at` (age
    // nought, so the lock's 10 m step) — the pilot's `RANGE` prints 50, and
    // the term reads 50, not the raw 47. A window of exactly [50, 50] passes
    // the range term (the aspect, next, answers), which the raw 47 would not.
    let mut moved = arena.clone();
    let red_x = moved.unit(RED).map(|state| state.pose.x_cm).unwrap_or(0);
    if let Some(row) = moved.contacts.get_mut(RED, contact_of(RED, BLUE)) {
        row.seen_at.x_cm = red_x + 4_700;
        row.last_seen_ms = at;
    }
    assert_eq!(
        moved
            .aim_at(RED, contact_of(RED, BLUE))
            .map(|aim| aim.range_cm),
        Some(4_700),
        "the raw range"
    );
    assert!(
        matches!(
            moved.warrant(RED, &failing, at),
            Err(Held::Why(HeldWhy::Range { range_m: 50, .. }))
        ),
        "the rounded figure: {:?}",
        moved.warrant(RED, &failing, at)
    );
    let exactly_fifty = FireCondition {
        min_range_m: 50,
        max_range_m: 50,
        ..failing.clone()
    };
    assert!(
        matches!(
            moved.warrant(RED, &exactly_fifty, at),
            Err(Held::Why(HeldWhy::Aspect { .. }))
        ),
        "50 m is inside [50, 50]: {:?}",
        moved.warrant(RED, &exactly_fifty, at)
    );
    let raw_window = FireCondition {
        min_range_m: 46,
        max_range_m: 48,
        ..failing.clone()
    };
    assert!(
        matches!(
            moved.warrant(RED, &raw_window, at),
            Err(Held::Why(HeldWhy::Range { range_m: 50, .. }))
        ),
        "the raw 47 is never read: {:?}",
        moved.warrant(RED, &raw_window, at)
    );
    let failing = FireCondition {
        min_range_m: 0,
        ..failing
    };
    assert!(matches!(
        held(failing.clone()),
        Err(Held::Why(HeldWhy::Aspect {
            aspect: Aspect::Left,
            ..
        }))
    ));
    let failing = FireCondition {
        aspects: ASPECTS_ANY,
        ..failing
    };
    assert_eq!(
        held(failing),
        Err(Held::Why(HeldWhy::Odds {
            permille: ODDS,
            floor: 900
        }))
    );
    // A cycling gun is no warrant failure.
    let mut cycling = arena.clone();
    if let Some(state) = cycling.unit_mut(RED) {
        state.ready_at_ms.insert(SmolStr::new("cannon"), at + 3_000);
    }
    assert_eq!(
        cycling.warrant(RED, &cond("cannon", 100, ASPECTS_ANY), at),
        Ok((contact_of(RED, BLUE), ODDS, Aspect::Left))
    );
}

// ---------------------------------------------------------------------------
// 8. Hold fire
// ---------------------------------------------------------------------------

// **17b cut M4.5a**: `m1b1_hold_fire_stops_a_slot_and_never_a_pilot` (a
// Reflex-flown hull's arena-armed slot under a persona `hold_fire` order never
// fires, the lock, the slot and its warrant standing; the order dropped, the
// slot fires; a pilot's typed `FIRE` fires) is deleted against its mirror on a
// unit mind, `m1b2_hold_fire_stops_a_mind_and_its_slot_and_never_a_pilot`
// (§ 13 below): a mind under a hold-fire order takes the lock and nothing
// fires, released the slot fires `by: "cond"` at the re-arming landing (leg
// 1); the pilot's typed `FIRE` fires (leg 2); a slot armed while the arena's
// row holds a hold-fire order has every slot and cited shot refused at the
// resolver until the order's deadline, then fires (leg 3). The reflex hull and
// its `drop_to_doctrine` restoration go with the controller.

// ---------------------------------------------------------------------------
// 9. The seat window and the count
// ---------------------------------------------------------------------------

/// **An arming counts on the seat window, and a third weapon is `NoSlot`**
/// (17b cut M1b.1): `max_pending_per_unit + 1` seat armings in one arena
/// second, the last refused `queue_full` at phase 2 (as a `LOCK`'s) and read
/// with its weapon, `FIRE cannon refused. …`; the authored heavy
/// (`tacsim.unit.heavy_mk1`, three weapons — `cannon`, `machine_gun`,
/// `missiles`) **with its core degraded to Class I** (17b cut M3.2: its
/// `core_class` cell set to 1 by a plan before `Arena::new`, so its capacity
/// is two — the shipped command core holds six and refuses nothing, which
/// `m3_the_condition_row_is_the_cores_slots` pins) arms two and the third is
/// refused from phase 4 as `FIRE
/// missiles refused. Every rule slot is occupied; MIND RULE <slot> OFF frees one.`, while
/// an under-floor third is `Unlawful` (the law before the count); a re-arm
/// keeps the slot's number, replaces its terms and clears its
/// `held_said` rows; a disarm of S1 leaves S2 numbered 2 and a new arming
/// takes S1; `Disarm { None }` empties the key.
#[test]
fn m1b1_arming_counts_on_the_seat_window_and_a_third_weapon_is_no_slot() {
    let (mut arena, wall) = locked_duel(100);
    // A fresh arena second: the `LOCK` at 5,000 counted on that one's window.
    let wall = wall + 1_000;
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(wall, &[], &[]);
    let bound = arena.arena_cfg.intents.max_pending_per_unit;
    let front = aspect_mask(&[Aspect::Front]);
    let kinds: Vec<IntentKind> = (0..=bound)
        .map(|_| arm(cond("cannon", 100, front)))
        .collect();
    let refused = refused_now(&mut arena, wall, kinds);
    assert_eq!(
        refused
            .iter()
            .filter_map(|(notice, _)| notice.refusal())
            .collect::<Vec<_>>(),
        vec![Refusal::QueueFull],
        "the one past the window, told at phase 2"
    );
    // Told at phase 2, it names its weapon as every `FIRE` refusal does.
    assert_eq!(
        refused.iter().map(sentence).collect::<Vec<_>>(),
        vec!["FIRE cannon refused. Too many orders at once; try that again.".to_owned()],
        "the arming's queue_full reads as a FIRE refusal with its weapon"
    );

    // The heavy, its core degraded to Class I (17b cut M3.2): two slots.
    let heavy = shipped_heavy(Some(1));
    let weapons: Vec<String> = heavy.weapons.iter().map(|w| w.id.to_string()).collect();
    assert_eq!(weapons, ["cannon", "machine_gun", "missiles"]);
    let mut arena = arena_of(vec![
        heavy,
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
    ]);
    let refused = refused_now(
        &mut arena,
        0,
        vec![
            arm(cond("cannon", 100, front)),
            arm(cond("machine_gun", 100, front)),
            arm(cond("missiles", 100, front)),
        ],
    );
    let [notice] = refused.as_slice() else {
        panic!("one refusal: {refused:?}");
    };
    assert_eq!(notice.0.refusal(), Some(Refusal::NoSlot));
    assert_eq!(
        sentence(notice),
        "FIRE missiles refused. Every rule slot is occupied; MIND RULE <slot> OFF frees one."
    );
    assert_eq!(
        (
            arena.conditions.slot_of(RED, "cannon"),
            arena.conditions.slot_of(RED, "machine_gun"),
            arena.conditions.slot_of(RED, "missiles")
        ),
        (Some(1), Some(2), None)
    );
    // The law before the count: an under-floor arming onto a full table is
    // `Unlawful`, naming the floor, and never `NoSlot`.
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(cond("missiles", 50, front)), 0),
        Err(Refusal::Unlawful)
    );
    // A re-arm: same number, new terms, its held rows gone.
    arena.held_said.insert((RED, 1, ContactId(7)), 1_000);
    arena.held_said.insert((RED, 2, ContactId(7)), 1_000);
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(cond("cannon", 300, front)), 0),
        Ok(Some(1))
    );
    assert_eq!(
        arena
            .conditions
            .slots(RED)
            .first()
            .and_then(Option::as_ref)
            .and_then(Rule::fire),
        Some(&cond("cannon", 300, front))
    );
    assert_eq!(
        arena.held_said.keys().copied().collect::<Vec<_>>(),
        vec![(RED, 2, ContactId(7))],
        "the re-armed slot's rows went; the other's stayed"
    );
    // A disarm of S1 leaves S2; a new arming takes S1.
    assert_eq!(
        arena.resolve_arming(
            RED,
            &Arming::Disarm {
                weapon: Some(SmolStr::new("cannon"))
            },
            0
        ),
        Ok(None)
    );
    assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), Some(2));
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(cond("missiles", 100, front)), 0),
        Ok(Some(1))
    );
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Disarm { weapon: None }, 0),
        Ok(None)
    );
    assert!(arena.conditions.is_empty_for(RED));
    assert!(arena.held_said.is_empty());
    assert_eq!(STOCK_CONDITION_SLOTS, 2);
}

/// **The shipped heavy**, checked out on red at (5, 8) facing east under a
/// pilot, through the production `custody_unit` — and, for `Some(n)`, its
/// core's `core_class` cell set to `n` by a plan BEFORE the check-out (17b
/// cut M3.2: a degraded command core; `None` the shipped Class III).
fn shipped_heavy(core_cell: Option<i64>) -> CustodyUnit {
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let key = mush_core::CanonicalId::new("tacsim.unit.heavy_mk1").expect("a canonical id");
    let spec = catalog.specs.get(&key).expect("the heavy's entry");
    let binding = catalog.bindings.get(&key).expect("the heavy's binding");
    let mut cells = tacsim::definitions::to_tactical_unit(spec, binding, RED).expect("the heavy");
    if let Some(value) = core_cell {
        cells = core_cell_at(&cells, value);
    }
    tacsim::match_::custody_unit(
        &catalog,
        cells,
        RED,
        Team::Red,
        facing(5, 8, 90_000),
        0,
        Controller::Pilot,
        1,
        &tacsim::match_::SealTerms::fixture(),
    )
    .expect("the fixture terms seal")
}

/// `unit` with its core's `core_class` cell set to `value` by one plan
/// through `mush_tactics::apply` — the one writer of a unit's cells.
fn core_cell_at(unit: &TacticalUnit, value: i64) -> TacticalUnit {
    let addr = mush_tactics::CellAddr::Component {
        component: mush_tactics::ComponentId::new("core").expect("a component"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::CORE_CLASS_CELL)
            .expect("a cell"),
    };
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell { addr, value }],
        None,
        None,
    )
    .expect("a plan");
    mush_tactics::apply(unit, &plan, 0)
        .expect("the plan applies")
        .0
}

/// `n` weapon rows of distinct ids, `w1`..`wn`, each a machine gun on the
/// hull.
fn guns(n: usize) -> Vec<WeaponSummary> {
    (1..=n)
        .map(|k| WeaponSummary {
            id: SmolStr::new(format!("w{k}")),
            ..machine_gun()
        })
        .collect()
}

/// **17b cut M3.2: the condition row is the core's slots** (doc 17 §5, §9,
/// D-UM-16: *up to the core's slots*; the plan's F5). On the shipped heavy
/// (the command core, Class III) a pilot arms its three guns through phase 4
/// with no refusal — S1, S2, S3 — the row laid down six wide, and a fourth
/// arming naming a weapon the hull does not carry is `NoWeapon`, never
/// `NoSlot`. On a hand-built Class III hull with SEVEN weapon rows the first
/// six take S1 to S6 and the seventh is `NoSlot`; a `truncate` to Class II's
/// four clears S5 and S6 and answers their numbers, so their `held_said`
/// rows go with them (17b cut M3.4's `Arena::truncate_slots` does that drop;
/// here the answered numbers drive it) while S1 to S4 and their rows stay. A
/// `Controller::Pilot` line tank (a Class II core) arms four distinct guns
/// and the fifth is `NoSlot`.
#[test]
fn m3_the_condition_row_is_the_cores_slots() {
    use tacsim::arena::core::{ClassTable, CoreClass, MAX_CONDITION_SLOTS};
    let front = aspect_mask(&[Aspect::Front]);
    // The shipped heavy: three accepted, six wide.
    let mut arena = arena_of(vec![
        shipped_heavy(None),
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
    ]);
    assert_eq!(arena.slot_capacity(RED), 6, "the command core's six");
    let refused = refused_now(
        &mut arena,
        0,
        vec![
            arm(cond("cannon", 100, front)),
            arm(cond("machine_gun", 100, front)),
            arm(cond("missiles", 100, front)),
        ],
    );
    assert!(refused.is_empty(), "no refusal: {refused:?}");
    assert_eq!(
        (
            arena.conditions.slot_of(RED, "cannon"),
            arena.conditions.slot_of(RED, "machine_gun"),
            arena.conditions.slot_of(RED, "missiles")
        ),
        (Some(1), Some(2), Some(3))
    );
    assert_eq!(arena.conditions.slots(RED).len(), 6, "the row is six wide");
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(cond("laser", 100, front)), 0),
        Err(Refusal::NoWeapon),
        "a fourth weapon id the hull does not carry is no weapon, not no slot"
    );

    // A hand-built Class III hull with seven guns: six slots, the seventh
    // refused.
    let seven = CustodyUnit {
        unit: tactical_unit_classed(RED, CoreClass::III),
        ..unit(RED, Team::Red, facing(5, 8, 90_000), guns(7), None)
    }
    .sealed();
    let mut arena = arena_of(vec![
        seven,
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
    ]);
    for k in 1..=MAX_CONDITION_SLOTS {
        assert_eq!(
            arena.resolve_arming(RED, &Arming::Arm(cond(&format!("w{k}"), 100, front)), 0),
            Ok(Some(u8::try_from(k).expect("a slot number")))
        );
    }
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(cond("w7", 100, front)), 0),
        Err(Refusal::NoSlot),
        "the seventh past the core's six"
    );
    // Held rows on S1, S5 and S6; the core drops to Class II's four.
    for slot in [1_u8, 5, 6] {
        arena.held_said.insert((RED, slot, ContactId(7)), 1_000);
    }
    let four = ClassTable::STOCK.row(CoreClass::II).slots;
    assert_eq!(four, 4);
    let cleared = arena.conditions.truncate(RED, four);
    assert_eq!(
        cleared,
        vec![5, 6],
        "S5 and S6 cleared, their numbers answered"
    );
    arena
        .held_said
        .retain(|(on, slot, _), _| !(*on == RED && cleared.contains(slot)));
    assert_eq!(
        arena.held_said.keys().copied().collect::<Vec<_>>(),
        vec![(RED, 1, ContactId(7))],
        "their held rows went with them; S1's stayed"
    );
    assert_eq!(arena.conditions.slots(RED).len(), 4);
    assert_eq!(arena.conditions.slot_of(RED, "w4"), Some(4));
    assert_eq!(arena.conditions.slot_of(RED, "w5"), None);
    assert_eq!(arena.conditions.slot_of(RED, "w6"), None);

    // A pilot's line tank (Class II): four distinct guns, the fifth refused.
    let line = CustodyUnit {
        unit: tactical_unit_classed(RED, CoreClass::II),
        ..unit(RED, Team::Red, facing(5, 8, 90_000), guns(5), None)
    }
    .sealed();
    assert_eq!(line.controller, Controller::Pilot);
    let mut arena = arena_of(vec![
        line,
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
    ]);
    assert_eq!(arena.slot_capacity(RED), 4);
    let refused = refused_now(
        &mut arena,
        0,
        (1..=5)
            .map(|k| arm(cond(&format!("w{k}"), 100, front)))
            .collect(),
    );
    assert_eq!(
        refused
            .iter()
            .filter_map(|(notice, _)| notice.refusal())
            .collect::<Vec<_>>(),
        vec![Refusal::NoSlot],
        "the fifth, from phase 4"
    );
    assert_eq!(
        (1..=5)
            .map(|k| arena.conditions.slot_of(RED, &format!("w{k}")))
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(3), Some(4), None]
    );
}

/// **17b cut M3.2: the count is told in two lines on a degraded heavy** (the
/// plan's F5; 17b cut M2.2's ruling (c)) — the transcript
/// `tests/cockpit.rs`' `m1b4_fire_when_arms_slots_in_the_arenas_numbers_and_the_count_is_one_sentence`
/// pinned on the shipped heavy until the command core's six slots made it
/// unreachable there. The heavy's core degraded to Class I (two slots, a
/// plan before `Arena::new`): with the cannon and the machine gun armed, the
/// pilot's `FIRE missiles WHEN odds >= 40` is parsed off the frame by the
/// cockpit's own parser and answered by its own reply — the acceptance — and
/// the arming's landing a pulse on is refused `NoSlot`, told as the arena's
/// notice: the two lines, in that order and no other line about the missiles
/// among EVERY notice the pulse told the pilot (`told_now`, not the
/// refusals alone).
/// Then the race on a full table (S2 freed, then the machine gun and the
/// missiles armed in ONE pulse: the machine gun takes S2 and the missiles
/// are refused in the same words) and the freed slot (the table full again,
/// then `FIRE mg OFF` and the missiles' arming in ONE pulse: the missiles
/// take the freed S2).
#[test]
fn m3_the_count_is_told_in_two_lines_on_a_degraded_heavy() {
    let mut arena = arena_of(vec![
        shipped_heavy(Some(1)),
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
    ]);
    assert_eq!(arena.slot_capacity(RED), 2, "the degraded core's two");
    let refused = refused_now(
        &mut arena,
        0,
        vec![
            arm(cond("cannon", 600, ASPECTS_ANY)),
            arm(cond("machine_gun", 300, ASPECTS_ANY)),
        ],
    );
    assert!(refused.is_empty(), "{refused:?}");
    // The pilot's frame, as the cockpit reads it.
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let out = arena.step(1_000, &[], &[]);
    let frame = out
        .frames
        .iter()
        .find(|(pilot, _)| *pilot == RED_PILOT)
        .map(|(_, frame)| frame.clone())
        .expect("the pilot's frame");
    let rows = cockpit::frame_weapons(&frame);
    let missiles = rows
        .iter()
        .find(|row| row.id == "missiles")
        .expect("the missiles' row");
    let (missiles_cond, echo) =
        cockpit::parse_arming(missiles, "odds >= 40", &frame).expect("a lawful rule");
    let mut transcript = vec![cockpit::arming_reply("missiles", &echo)];
    // Every line the pulse told the pilot, not the refusals alone: the
    // transcript is every one that names the missiles.
    let told = told_now(&mut arena, 2_000, vec![arm(missiles_cond.clone())]);
    assert_eq!(
        told.iter()
            .filter(|(notice, _)| matches!(notice, FrameNotice::Refused { .. }))
            .map(sentence)
            .collect::<Vec<_>>(),
        vec![
            "FIRE missiles refused. Every rule slot is occupied; MIND RULE <slot> OFF frees one."
                .to_owned()
        ],
        "the one refusal"
    );
    transcript.extend(about_missiles(&heard(&told)));
    assert_eq!(
        transcript,
        vec![
            "Arming missiles: fires your lock when your chance to hit is 40% or better.".to_owned(),
            "FIRE missiles refused. Every rule slot is occupied; MIND RULE <slot> OFF frees one."
                .to_owned(),
        ],
        "the acceptance, then the arena's refusal: two lines and no more"
    );
    assert_eq!(arena.conditions.slot_of(RED, "missiles"), None);
    assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), Some(2));

    // The race: S2 freed, then two armings in one pulse.
    let refused = refused_now(
        &mut arena,
        3_000,
        vec![IntentKind::Condition(Arming::Disarm {
            weapon: Some(SmolStr::new("machine_gun")),
        })],
    );
    assert!(refused.is_empty());
    assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), None);
    let told = told_now(
        &mut arena,
        4_000,
        vec![
            arm(cond("machine_gun", 300, ASPECTS_ANY)),
            arm(missiles_cond.clone()),
        ],
    );
    let raced = vec![
        "FIRE missiles refused. Every rule slot is occupied; MIND RULE <slot> OFF frees one."
            .to_owned(),
    ];
    assert_eq!(
        told.iter()
            .filter(|(notice, _)| matches!(notice, FrameNotice::Refused { .. }))
            .map(sentence)
            .collect::<Vec<_>>(),
        raced,
        "the one refusal"
    );
    assert_eq!(
        about_missiles(&heard(&told)),
        raced,
        "the one line about the missiles: {:?}",
        heard(&told)
    );
    assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), Some(2));
    assert_eq!(arena.conditions.slot_of(RED, "missiles"), None);

    // The freed slot: the disarm and the arming in one pulse.
    let refused = refused_now(
        &mut arena,
        5_000,
        vec![
            IntentKind::Condition(Arming::Disarm {
                weapon: Some(SmolStr::new("machine_gun")),
            }),
            arm(cond("missiles", 200, ASPECTS_ANY)),
        ],
    );
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), None);
    assert_eq!(arena.conditions.slot_of(RED, "missiles"), Some(2));
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));
}

/// **17b cut M3.2: a slot past the second fires on its `WeaponReady`
/// millisecond at three cadences** (16b §11's mandatory cross-tempo proof for
/// the core-sized row; doc 17 §5, D-UM-16). The shipped heavy (the command
/// core's six slots) under a pilot: `STARTUP` at 0, then at 5,000 the lock
/// and three armings in one pulse — the machine gun (S1), the missiles (S2)
/// and the cannon, which takes **S3**, a slot the fixed two-wide row never
/// had. Each arming's evaluation fires at once at 5,000; the cannon's second
/// shot from S3 lands at exactly `5,000 + cycle_ms` = 11,000 — MID-PULSE at
/// the 5,000 ms cadence — and its third at 17,000 (mid-pulse again), the
/// missiles' second from S2 at 15,000. Run to 20,000 (a pulse end at every
/// cadence). One
/// event list and one hash sequence at 5,000 / 1,000 / 500 ms and on the
/// task's own wakes, and the row is six wide on every drive.
#[test]
fn m3_a_third_slot_fires_on_its_weapon_ready_millisecond_at_three_cadences() {
    let contact = contact_of(RED, BLUE);
    let make = || {
        arena_of(vec![
            shipped_heavy(None),
            unit(
                BLUE,
                Team::Blue,
                facing(9, 8, 0),
                Vec::new(),
                Some(sight(0)),
            ),
        ])
    };
    let script = || {
        vec![
            beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
            beat(
                5_000,
                vec![
                    intent(RED, IntentKind::Lock { contact }, 2),
                    intent(RED, arm(cond("machine_gun", 100, ASPECTS_ANY)), 3),
                    intent(RED, arm(cond("missiles", 100, ASPECTS_ANY)), 4),
                    intent(RED, arm(cond("cannon", 100, ASPECTS_ANY)), 5),
                ],
            ),
        ]
    };
    let run = at_every_drive(make, 20_000, script);
    let shots = cond_shots(&run);
    let of_slot = |slot: u64| -> Vec<i64> {
        shots
            .iter()
            .filter(|(_, on, _)| *on == slot)
            .map(|(t_ms, _, _)| *t_ms)
            .collect()
    };
    assert_eq!(
        of_slot(3),
        vec![5_000, 11_000, 17_000],
        "S3's cannon at the arming and at each WeaponReady: {shots:?}"
    );
    assert_eq!(
        of_slot(2),
        vec![5_000, 15_000],
        "S2's missiles at the arming and at their WeaponReady: {shots:?}"
    );
    assert_eq!(
        of_slot(1).first(),
        Some(&5_000),
        "S1's machine gun from the arming: {shots:?}"
    );
    assert!(
        shots
            .iter()
            .all(|(_, _, target)| *target == u64::from(BLUE.0)),
        "every slot shot at the lock: {shots:?}"
    );
    assert!(
        run.notices
            .iter()
            .all(|notice| !matches!(notice, FrameNotice::Refused { .. })),
        "nothing the pilot typed was refused: {:?}",
        run.notices
    );
    // The row, six wide on every drive (the arena at the end of one).
    let mut arena = make();
    let _ = replay(&mut arena, Drive::Every(500), 20_000, &script());
    assert_eq!(arena.conditions.slots(RED).len(), 6);
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(3));
}

// ---------------------------------------------------------------------------
// 10. The slots' life
// ---------------------------------------------------------------------------

/// **Slots die with the controller and with the turret, and survive a grace**
/// (17b cut M1b.1): a seat's arming then `Released` leaves no slot and the hash
/// sequence from that pulse on equals a never-armed run's; `Mind → Pilot`
/// clears them; `RemoveWeapons` on the turret takes the cannon's slot and
/// leaves the hull machine gun's with its number; a pilot gone `Linkdead`
/// still has the slot fire through the grace (the `Cond` exemption).
#[test]
fn m1b1_slots_die_with_the_controller_and_with_the_turret_and_survive_a_grace() {
    let contact = contact_of(RED, BLUE);
    let released = |armed: bool| {
        let mut beats = vec![beat(0, vec![intent(RED, IntentKind::Startup, 1)])];
        let mut at_5000 = vec![intent(RED, IntentKind::Lock { contact }, 2)];
        // The never-armed run sends a disarm of nothing in the arming's place,
        // so the seat's window counts the same orders on both.
        at_5000.push(intent(
            RED,
            if armed {
                // A slot that holds (front only, blue shows its left).
                arm(cond("cannon", 100, aspect_mask(&[Aspect::Front])))
            } else {
                IntentKind::Condition(Arming::Disarm { weapon: None })
            },
            3,
        ));
        beats.push(beat(5_000, at_5000));
        beats.push(Beat {
            t_ms: 10_000,
            controls: Channel::ALL
                .iter()
                .map(|channel| Control {
                    unit: RED,
                    channel: *channel,
                    epoch: 2,
                    state: ControlState::Released,
                    pilot: Some(RED_PILOT),
                })
                .collect(),
            intents: Vec::new(),
        });
        beats
    };
    let mut armed = duel(0);
    let with = replay(&mut armed, Drive::Every(500), 20_000, &released(true));
    let mut bare = duel(0);
    let without = replay(&mut bare, Drive::Every(500), 20_000, &released(false));
    assert!(armed.conditions.is_empty_for(RED), "the release cleared it");
    assert!(with.counters.cond_held > 0, "it was armed and holding");
    let after = |run: &Run| -> Vec<(i64, String, u64)> {
        run.events
            .iter()
            .filter(|(t, _, _)| *t > 10_000)
            .cloned()
            .collect()
    };
    assert_eq!(
        after(&with),
        after(&without),
        "the same lines after the release"
    );
    assert_eq!(
        tacsim::arena::state_hash(&armed),
        tacsim::arena::state_hash(&bare),
        "and the same state"
    );

    // Mind -> Pilot.
    let (mut arena, _) = locked_duel(100);
    arena.set_controller(
        RED,
        Controller::Mind,
        tacsim::arena::doctrine::MachineState::Patrol,
    );
    let _ = arena.resolve_arming(RED, &Arming::Arm(cond("cannon", 100, ASPECTS_ANY)), 0);
    assert!(!arena.conditions.is_empty_for(RED));
    // A held row stamped under the outgoing principal, and another hull's.
    arena.held_said.insert((RED, 1, ContactId(7)), 1_000);
    arena.held_said.insert((BLUE, 1, ContactId(7)), 1_000);
    arena.set_controller(
        RED,
        Controller::Pilot,
        tacsim::arena::doctrine::MachineState::Patrol,
    );
    assert!(
        arena.conditions.is_empty_for(RED),
        "Mind -> Pilot cleared it"
    );
    assert_eq!(
        arena.held_said.keys().copied().collect::<Vec<_>>(),
        vec![(BLUE, 1, ContactId(7))],
        "the outgoing principal's held rows went with its slots; another hull's stayed"
    );

    // The turret.
    let mut arena = arena_of(vec![
        unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![cannon(), machine_gun()],
            Some(sight(0)),
        ),
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
    ]);
    let front = aspect_mask(&[Aspect::Front]);
    let _ = arena.resolve_arming(RED, &Arming::Arm(cond("cannon", 100, front)), 0);
    let _ = arena.resolve_arming(RED, &Arming::Arm(cond("machine_gun", 100, front)), 0);
    arena.held_said.insert((RED, 1, ContactId(7)), 1_000);
    let mut journal = Vec::new();
    arena.apply_consequence(
        RED,
        &tacsim::arena::resolution::Consequence::RemoveWeapons {
            section: SmolStr::new("turret"),
        },
        0,
        None,
        &mut journal,
    );
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), None);
    assert_eq!(
        arena.conditions.slot_of(RED, "machine_gun"),
        Some(2),
        "the gun that stands keeps its number"
    );
    assert!(arena.held_said.is_empty(), "the gone slot's rows went too");

    // A linkdead grace: the slot still fires at its WeaponReady.
    let mut script = armed_at_5000(cond("cannon", 100, ASPECTS_ANY));
    script.push(Beat {
        t_ms: 5_000,
        controls: Channel::ALL
            .iter()
            .map(|channel| Control {
                unit: RED,
                channel: *channel,
                epoch: 1,
                state: ControlState::Linkdead,
                pilot: Some(RED_PILOT),
            })
            .collect(),
        intents: Vec::new(),
    });
    let mut arena = duel(0);
    let run = replay(&mut arena, Drive::Every(500), 10_000, &script);
    assert!(arena.unit(RED).is_some_and(|state| state.linkdead));
    assert_eq!(
        cond_shots(&run)
            .iter()
            .map(|(t, _, _)| *t)
            .collect::<Vec<_>>(),
        vec![5_000, 9_000],
        "the second shot fired through the grace"
    );
}

// ---------------------------------------------------------------------------
// 12. The snapshot
// ---------------------------------------------------------------------------

/// **A snapshot carries the slots, a card, the floor and `held_said`, and the
/// resume continues the hash sequence** (17b cut M1b.1; `SNAPSHOT_FORMAT` 7):
/// an arena with an armed seat slot, a mind hull whose card was edited, a
/// floor of 50, a `held_said` map populated by hand (the `pairs` path) and, in
/// the mind's notice buffer, the two slot notices (every `HeldWhy`) and a
/// refusal carrying `RefusalDetail::Floor` round-trips at format 7; stepped on,
/// the resumed arena's substep hashes and ledger bytes equal the uninterrupted
/// one's; a format-6 document is refused by name. (`IntentKind::Fire.slot` and
/// `IntentKind::Condition` ride `Arena.pending`, empty between pulses: their
/// round trip is `snapshot.rs`' `af1_every_keyed_map_in_the_tree_is_one_json_writes`.)
#[test]
fn m1b1_a_snapshot_carries_slots_a_card_the_floor_and_held_said_and_resumes_the_hash_sequence() {
    let (mut arena, mut wall) = locked_duel(50);
    // A mind on blue, thinking nothing, its card edited.
    arena.mind_runner = Arc::new(tacsim::arena::mind::NoMind);
    arena.set_controller(
        BLUE,
        Controller::Mind,
        tacsim::arena::doctrine::MachineState::Patrol,
    );
    for (slider, value) in [("min_odds", "30"), ("stance", "orbit")] {
        arena.stage_card(
            BLUE,
            tacsim::arena::CardChange::Set {
                slider: slider.into(),
                value: value.into(),
            },
        );
    }
    let _ = refused_now(
        &mut arena,
        wall,
        vec![arm(cond("cannon", 50, aspect_mask(&[Aspect::Front])))],
    );
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));
    let edited = arena.profiles[&BLUE].card.stock().expect("stock adapter");
    assert_eq!(edited.min_odds_permille, 300);
    assert_eq!(edited.stance, tacsim::arena::card::Stance::Orbit);
    arena
        .held_said
        .insert((RED, 1, contact_of(RED, BLUE)), 4_000);
    // A pulse boundary with no think in flight (the landing scratch is reset).
    loop {
        wall += 250;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
        if arena
            .minds
            .get(&BLUE)
            .is_some_and(|state| state.pending.is_none())
        {
            break;
        }
    }
    // The carried notice shapes format 7 added, buffered on blue's mind by
    // hand (nothing stages a slot notice before 17b cut M1b.4): a slot's shot
    // with and without a contact, a held slot under each of the four whys, and
    // a refusal carrying the floor.
    let named = tacsim::arena::perception::Named {
        id: contact_of(BLUE, RED),
        designator: tacsim::arena::state::Designator::Slot {
            team: Team::Red,
            slot: 1,
        },
    };
    let mut carried = vec![
        FrameNotice::SlotFired {
            slot: 1,
            weapon: SmolStr::new("cannon"),
            contact: Some(named),
            p_permille: 620,
            aspect: Aspect::Left,
        },
        FrameNotice::SlotFired {
            slot: 2,
            weapon: SmolStr::new("machine_gun"),
            contact: None,
            p_permille: 950,
            aspect: Aspect::Rear,
        },
        FrameNotice::Refused {
            order: SmolStr::new("fire"),
            refusal: Refusal::Unlawful,
            weapon: Some(SmolStr::new("cannon")),
            detail: RefusalDetail::Floor { permille: 50 },
        },
    ];
    for why in [
        HeldWhy::Odds {
            permille: 540,
            floor: 600,
        },
        HeldWhy::Aspect {
            contact: named,
            aspect: Aspect::Front,
        },
        HeldWhy::Range {
            contact: named,
            range_m: 1_200,
            min_m: 200,
            max_m: 600,
        },
        HeldWhy::Rounds {
            rounds: 3,
            keeps: 5,
        },
    ] {
        carried.push(FrameNotice::SlotHeld {
            slot: 1,
            weapon: SmolStr::new("cannon"),
            why,
        });
    }
    if let Some(state) = arena.minds.get_mut(&BLUE) {
        state.notices.extend(carried.iter().cloned());
    }
    let snapshot = Snapshot::encode(&arena).expect("an arena between pulses encodes");
    // 17b cut M3.2 moved the format to 8, 17b cut M4.6 to 9 and 17b cut M5.2
    // to 10; M5rules moves the ordered rule tree to 11, M5r motor state to 12.
    // Format 16 also carries record-only accepted-order attribution (R4).
    assert_eq!(snapshot.document["format"], 16);
    let mut resumed =
        Snapshot::decode(&snapshot.document, definition()).expect("the document decodes");
    resumed.mind_runner = Arc::new(tacsim::arena::mind::NoMind);
    assert_eq!(resumed.odds_floor_permille, 50);
    assert_eq!(resumed.conditions, arena.conditions);
    assert_eq!(resumed.held_said, arena.held_said);
    assert_eq!(
        resumed
            .profiles
            .get(&BLUE)
            .map(|runtime| runtime.card.clone()),
        arena
            .profiles
            .get(&BLUE)
            .map(|runtime| runtime.card.clone())
    );
    let resumed_notices = resumed
        .minds
        .get(&BLUE)
        .map(|state| state.notices.clone())
        .unwrap_or_default();
    assert!(
        resumed_notices.ends_with(&carried),
        "the slot notices and the floor detail round-trip: {resumed_notices:?}"
    );
    assert_eq!(
        Some(&resumed_notices),
        arena.minds.get(&BLUE).map(|state| &state.notices)
    );
    let state = snapshot.document["state"].as_str().unwrap_or_default();
    assert!(
        state.contains("\"held_said\":[["),
        "the pairs form: {state}"
    );
    resumed.resume_at(wall);
    let mut hashes: [Vec<u64>; 2] = [Vec::new(), Vec::new()];
    let mut bytes: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    for (at, one) in [&mut arena, &mut resumed].into_iter().enumerate() {
        let mut wall_now = wall;
        while one.now_ms < 20_000 {
            wall_now += 500;
            one.clock.pin_tempo_for_tests(Tempo::Quiet);
            let out = one.step(wall_now, &[], &[]);
            for chunk in &out.ledger_chunks {
                bytes[at].extend_from_slice(&chunk.bytes);
            }
            hashes[at].extend(one.substep_hashes.iter().copied());
        }
    }
    assert_eq!(hashes[0], hashes[1], "the resumed hash sequence");
    // The ledger holds the tail of an open chunk; flush both and read every
    // line each handed out since the snapshot.
    let [first, second] = &mut bytes;
    let after = lines(&mut arena, first);
    let resumed_after = lines(&mut resumed, second);
    assert!(!after.is_empty(), "the run journalled");
    assert_eq!(after, resumed_after, "the resumed lines");
    assert_eq!(bytes[0], bytes[1], "byte for byte");
    assert_eq!(
        tacsim::arena::state_hash(&arena),
        tacsim::arena::state_hash(&resumed)
    );

    // A format-6 document is refused by name (expected 16 after R4; no old-format migration).
    let mut old = snapshot.document.clone();
    old["format"] = serde_json::json!(6);
    assert_eq!(
        Snapshot::head(&old).map(|head| head.format),
        Err(SnapshotError::Format {
            found: 6,
            expected: 16
        })
    );
    assert!(matches!(
        Snapshot::decode(&old, definition()),
        Err(SnapshotError::Format {
            found: 6,
            expected: 16
        })
    ));
}

// ---------------------------------------------------------------------------
// 13. 17b cut M1b.2: the law for a mind's fire, and T1's slots
// ---------------------------------------------------------------------------

/// **A scripted mind** (the `tests/minds.rs` shape, cut down): think `n` of a
/// hull answers its script's intents for `n`, and the memory is echoed. An
/// input to `Arena::step`, as a scripted intent log is (16b §0.4).
#[derive(Debug, Default)]
struct Scripted {
    says: BTreeMap<(ObjId, u64), Vec<IntentKind>>,
}

impl MindRunner for Scripted {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        ThinkResult {
            diagnostics: None,
            rule_sample: None,
            intents: self
                .says
                .get(&(job.unit, job.think_seq))
                .cloned()
                .unwrap_or_default(),
            memory: job.memory.to_vec(),
            fault: None,
            trace: Vec::new(),
        }
    }
}

/// One stock think, kept: the hull, the think's millisecond, its view and its
/// answer.
type Thought = (ObjId, i64, MindView, ThinkResult);

/// **The arena's own runner, its thinks kept** — on a stock field the
/// production stock mind (`NativeStockMind` on the arena's own doctrine, what
/// `Arena::new` installs), on a scripted one the [`Scripted`] mind, answering
/// untouched (an input recorder, not a stand-in: 16b §0.4).
#[derive(Debug)]
struct Kept {
    inner: Arc<dyn MindRunner>,
    thoughts: Mutex<Vec<Thought>>,
}

impl Kept {
    fn on(arena: &Arena) -> Arc<Kept> {
        Arc::new(Kept {
            inner: arena.mind_runner.clone(),
            thoughts: Mutex::new(Vec::new()),
        })
    }

    fn thoughts(&self) -> Vec<Thought> {
        self.thoughts
            .lock()
            .map(|thoughts| thoughts.clone())
            .unwrap_or_default()
    }
}

impl MindRunner for Kept {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.inner.think(job);
        if let Ok(mut thoughts) = self.thoughts.lock() {
            let mut behavior = result.clone();
            // Interpreter operation/fee diagnostics have no native analogue.
            // Compare every simulation input, sampled rule and author trace;
            // return the original answer, including diagnostics, to the arena.
            behavior.diagnostics = None;
            thoughts.push((job.unit, job.t_ms, (**job.view).clone(), behavior));
        }
        result
    }
}

/// The lattice millisecond of `unit`'s think `n`.
fn think_at(unit: ObjId, n: i64) -> i64 {
    stagger_ms(SEED, unit, STOCK_THINK_MS) + n * STOCK_THINK_MS
}

/// A ten-cell sight that shares nothing: a stock mind's bands on this window
/// (a view holds a range in 50 m steps; five cells would leave its engage
/// band under one step — `tests/minds.rs`' `the_j_field` note).
fn wide_sight() -> SensorProfile {
    SensorProfile {
        max_range_cells: 10,
        ..sight(0)
    }
}

/// `unit` flown by a mind.
fn minded(unit: CustodyUnit) -> CustodyUnit {
    CustodyUnit {
        controller: Controller::Mind,
        ..unit
    }
    .sealed()
}

/// **An arena of `units` with only `seated` seated** (pilots `#1001` up), the
/// odds pinned at [`ODDS`], the tempo `Quiet`.
fn seated_arena(units: Vec<CustodyUnit>, seated: &[ObjId]) -> Arena {
    seated_arena_on(definition(), units, seated)
}

/// [`seated_arena`] on the window `def`.
fn seated_arena_on(def: ArenaDefinition, units: Vec<CustodyUnit>, seated: &[ObjId]) -> Arena {
    let pilots = seated
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: *unit,
            pilot: ObjId(1_001 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    let mut arena = Arena::new(
        CustodyOut {
            definition: def,
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the mind arena");
    arena.resolution.p_min = ODDS;
    arena.resolution.p_max = ODDS;
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    arena
}

/// `viewer` holds `target` as `class`, plotted where it stands.
fn hold_row(arena: &mut Arena, viewer: ObjId, target: ObjId, class: Classification) {
    let plot = arena.unit(target).map(SeenPose::of).unwrap_or_default();
    arena.contacts.insert(
        viewer,
        contact_of(viewer, target),
        ShadowContact::sighted(target, class, 0, plot),
    );
}

/// **The mind duel**: red a scripted MIND at (5, 8) facing east with the
/// cannon and the machine gun, online, seeing five cells, holding blue at
/// (9, 8) as `class` (handed it, so its own passes refresh the row and keep
/// the lock) and locked on it; blue seated, unarmed, facing west, presenting
/// a silhouette. Red's mind says `says` (`(think, intents)`).
fn mind_duel(class: Classification, says: &[(u64, Vec<IntentKind>)]) -> Arena {
    let mut arena = seated_arena(
        vec![
            minded(unit(
                RED,
                Team::Red,
                facing(5, 8, 90_000),
                vec![cannon(), machine_gun()],
                Some(sight(0)),
            )),
            unit(BLUE, Team::Blue, facing(9, 8, 270_000), Vec::new(), None),
        ],
        &[BLUE],
    );
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
    }
    hold_row(&mut arena, RED, BLUE, class);
    assert_eq!(arena.resolve_lock(RED, contact_of(RED, BLUE), 0), Ok(()));
    arena.mind_runner = Arc::new(Scripted {
        says: says
            .iter()
            .map(|(n, intents)| ((RED, *n), intents.clone()))
            .collect(),
    });
    arena
}

/// A standing hold-fire order on `unit`'s mind row, as the arena holds a
/// persona's (`FrameOrder`, re-keyed at its landing).
fn hold_fire_on(arena: &mut Arena, unit: ObjId) {
    if let Some(state) = arena.minds.get_mut(&unit) {
        state.order = Some(FrameOrder {
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Hold,
            hold_fire: true,
            max_range_band: None,
            expires_ms: 600_000,
        });
    }
    assert!(arena.holds_fire(unit));
}

/// One `fired` line: `(t_ms, weapon, target, by, slot)`.
type Shot = (i64, String, u64, Option<String>, Option<u64>);

/// The `fired` lines among `events`.
fn shots_of(events: &[LedgerEvent]) -> Vec<Shot> {
    events
        .iter()
        .filter_map(|event| {
            let value = serde_json::to_value(&event.kind).ok()?;
            (value.get("kind")? == "fired").then(|| {
                (
                    event.t_ms,
                    value
                        .get("weapon")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    value
                        .get("target")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    value
                        .get("by")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    value.get("slot").and_then(serde_json::Value::as_u64),
                )
            })
        })
        .collect()
}

/// **A rig stepped one substep a pulse**, every ledger byte kept.
struct Stepper {
    arena: Arena,
    wall: Option<i64>,
    bytes: Vec<u8>,
}

impl Stepper {
    fn new(arena: Arena) -> Stepper {
        Stepper {
            arena,
            wall: None,
            bytes: Vec::new(),
        }
    }

    /// One pulse of one substep, delivering `intents`.
    fn pulse(&mut self, intents: &[Intent]) {
        let step = self.arena.substep_len_ms();
        let wall = self.wall.map_or(0, |wall| wall + step);
        self.wall = Some(wall);
        self.arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let out = self.arena.step(wall, &[], intents);
        absorb(&out, &mut self.bytes, &mut Vec::new());
    }

    /// Pulse until arena time reaches `t_ms`.
    fn to(&mut self, t_ms: i64) {
        while self.arena.now_ms < t_ms {
            self.pulse(&[]);
        }
    }

    /// Every `fired` line so far.
    fn shots(&mut self) -> Vec<Shot> {
        shots_of(&lines(&mut self.arena, &mut self.bytes))
    }

    /// Every line so far, as `(t_ms, kind word, unit)`.
    fn words(&mut self) -> Vec<(i64, String, Option<u64>)> {
        lines(&mut self.arena, &mut self.bytes)
            .iter()
            .filter_map(|event| {
                let value = serde_json::to_value(&event.kind).ok()?;
                Some((
                    event.t_ms,
                    value.get("kind")?.as_str()?.to_owned(),
                    value.get("unit").and_then(serde_json::Value::as_u64),
                ))
            })
            .collect()
    }

    /// The `refused` notices in `unit`'s mind buffer, `(word, weapon, detail)`
    /// (drained by its next think: read between a landing and that think).
    fn refused(&self, unit: ObjId) -> Vec<(Refusal, Option<SmolStr>, RefusalDetail)> {
        self.arena
            .minds
            .get(&unit)
            .map(|state| {
                state
                    .notices
                    .iter()
                    .filter_map(|notice| match notice {
                        FrameNotice::Refused {
                            refusal,
                            weapon,
                            detail,
                            ..
                        } => Some((*refusal, weapon.clone(), *detail)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// An intent on the channel that carries it at the hull's current epoch.
fn current(arena: &Arena, unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
    let channel = kind.channel_required();
    Intent {
        unit,
        channel,
        epoch: arena.epochs.current(unit, channel),
        kind,
        seq,
    }
}

/// The cannon's `Fire`, citing `slot` or not.
fn cannon_fire(slot: Option<u8>) -> IntentKind {
    IntentKind::Fire {
        weapon: SmolStr::new("cannon"),
        slot,
    }
}

/// **The law for a mind's fire** (17b cut M1b.2; doc 17 §5, D-UM-17 as
/// amended 2026-10-04, D-UM-19): red, a scripted mind locked on a classified
/// blue at 50 per cent, lands one think's intents at a time, and the arena's
/// `resolve_fire_by` gates the `FIRE` in the pinned order — `Cycling`,
/// `Empty`, `Unlawful` (the cited slot must exist and name the gun, else
/// `Unwarranted`), hold-fire, the warrant, then `Overheated` (pinned on the
/// laser hull below) and the power:
///
/// * an uncited `FIRE` — `unwarranted` in the mind's buffer, nothing fired;
/// * a citation of a slot that names another gun (the machine gun's S1, on
///   terms the plot meets, so its own warrant holds and it fires its own gun
///   at the landing) — `unwarranted`, the cannon silent: only the weapon match
///   refuses it;
/// * an arming at 50 per mille on the floor-100 arena — refused `unlawful` at
///   its landing with the floor named in the buffered detail
///   (`RefusalDetail::Floor { permille: 100 }`), and the citation after it
///   `unwarranted` (no slot);
/// * a citation of a slot whose odds term is above the plot's 50 per cent —
///   `unwarranted`, the slot armed;
/// * a citation of a slot that holds — `fired`, `by: "mind"`, `slot: 1`, and
///   nothing else fires (the landing's evaluation finds the gun cycling);
/// * a citation landing on the millisecond of the gun's `WeaponReady` after
///   the slot's own shot — the slot fires at the `WeaponReady` first (`by:
///   "cond"`), and the citation reads `cycling`, never `unwarranted`; with the
///   floor raised over the slot, still `cycling` (the weapon before the law);
/// * a citation of a slot on an `X` lock (unclassified) — `unwarranted`, the
///   slot idle (the warrant's target term);
/// * under a standing hold-fire, a lawful citation reads `unwarranted` and an
///   unlawful one `unlawful` (the floor before hold-fire);
/// * a seat's typed `FIRE` at nought per cent fires and misses: a pilot's
///   trigger is never gated (T0's `FiredBy::Machine` neither — the (j)
///   field's line pins in `tests/reflex.rs` re-run unchanged).
#[test]
fn m1b2_the_law_refuses_an_uncited_and_an_unwarranted_mind_fire_and_a_cycling_one_reads_cycling() {
    let classified = Classification::Classified { team: Team::Blue };
    let cannon_word = Some(SmolStr::new("cannon"));
    let unwarranted = vec![(
        Refusal::Unwarranted,
        cannon_word.clone(),
        RefusalDetail::None,
    )];
    let landing = |arena: &Arena, n: i64| think_at(RED, n) + arena.substep_len_ms();
    let case = |class: Classification, says: Vec<IntentKind>| {
        let mut rig = Stepper::new(mind_duel(class, &[(0, says)]));
        let land = landing(&rig.arena, 0);
        rig.to(land);
        let refused = rig.refused(RED);
        (rig.shots(), refused, rig.arena, land)
    };
    // Uncited.
    let (shots, refused, _, _) = case(classified, vec![cannon_fire(None)]);
    assert_eq!(shots, Vec::new());
    assert_eq!(refused, unwarranted);
    // A slot that names another gun — armed on terms the plot MEETS, so only
    // the weapon match can refuse the cannon's citation of it: the machine
    // gun's warrant holds and its own slot fires at the landing's one
    // evaluation (`by: "cond"`), and the cannon's borrowed citation reads
    // `unwarranted` and fires nothing.
    let mg_slot = cond("machine_gun", 100, ASPECTS_ANY);
    let (shots, refused, arena, land) =
        case(classified, vec![arm(mg_slot.clone()), cannon_fire(Some(1))]);
    assert!(
        arena.warrant(RED, &mg_slot, land).is_ok(),
        "the precondition: the machine gun's slot holds on the plot"
    );
    assert_eq!(
        shots,
        vec![(
            land,
            "machine_gun".to_owned(),
            u64::from(BLUE.0),
            Some("cond".to_owned()),
            Some(1)
        )],
        "the slot's own gun fired, never the cannon on its warrant"
    );
    assert_eq!(refused, unwarranted);
    // An arming under the floor, then its citation.
    let (shots, refused, arena, _) = case(
        classified,
        vec![arm(cond("cannon", 50, ASPECTS_ANY)), cannon_fire(Some(1))],
    );
    assert_eq!(shots, Vec::new());
    assert!(arena.conditions.is_empty_for(RED), "nothing armed");
    assert_eq!(
        refused,
        vec![
            (
                Refusal::Unlawful,
                cannon_word.clone(),
                RefusalDetail::Floor { permille: 100 }
            ),
            (
                Refusal::Unwarranted,
                cannon_word.clone(),
                RefusalDetail::None
            ),
        ]
    );
    // A slot whose odds term the plot does not meet.
    let (shots, refused, arena, _) = case(
        classified,
        vec![arm(cond("cannon", 600, ASPECTS_ANY)), cannon_fire(Some(1))],
    );
    assert_eq!(shots, Vec::new());
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));
    assert_eq!(refused, unwarranted);
    // A slot that holds: the cited shot, by the mind, and nothing else.
    let (shots, refused, _, land) = case(
        classified,
        vec![arm(cond("cannon", 100, ASPECTS_ANY)), cannon_fire(Some(1))],
    );
    assert_eq!(refused, Vec::new());
    assert_eq!(
        shots,
        vec![(
            land,
            "cannon".to_owned(),
            u64::from(BLUE.0),
            Some("mind".to_owned()),
            Some(1)
        )]
    );
    // An `X` lock: the slot idle, the citation unwarranted.
    let (shots, refused, arena, _) = case(
        Classification::Unknown,
        vec![arm(cond("cannon", 100, ASPECTS_ANY)), cannon_fire(Some(1))],
    );
    assert_eq!(shots, Vec::new());
    assert_eq!(
        arena.conditions.slot_of(RED, "cannon"),
        Some(1),
        "armed, idle"
    );
    assert_eq!(refused, unwarranted);

    // The citation at the gun's `WeaponReady`: the slot armed at think 0
    // fires at its landing; the cannon's cycle ends exactly at think 4's
    // landing, where the `WeaponReady` pops before the landing and the slot
    // fires again — and the citation reads `cycling`. Once more with the
    // floor raised over the slot after its arming: still `cycling`.
    for raise_floor in [false, true] {
        let mut rig = Stepper::new(mind_duel(
            classified,
            &[
                (0, vec![arm(cond("cannon", 100, ASPECTS_ANY))]),
                (4, vec![cannon_fire(Some(1))]),
            ],
        ));
        let land0 = landing(&rig.arena, 0);
        let land4 = landing(&rig.arena, 4);
        assert_eq!(
            land4 - land0,
            i64::from(cannon().cycle_ms),
            "the precondition"
        );
        rig.to(land0);
        if raise_floor {
            rig.arena.odds_floor_permille = 200;
        }
        rig.to(land4);
        let refused = rig.refused(RED);
        assert_eq!(
            refused.iter().map(|(word, ..)| *word).collect::<Vec<_>>(),
            vec![Refusal::Cycling],
            "floor raised: {raise_floor}"
        );
        let shots: Vec<(i64, Option<String>)> = rig
            .shots()
            .into_iter()
            .map(|(t_ms, _, _, by, _)| (t_ms, by))
            .collect();
        assert_eq!(
            shots,
            vec![
                (land0, Some("cond".to_owned())),
                (land4, Some("cond".to_owned()))
            ]
        );
    }

    // Under a standing hold-fire: the lawful citation `unwarranted`, and the
    // slot's own landing shot dropped silently.
    let mut rig = Stepper::new(mind_duel(
        classified,
        &[(
            0,
            vec![arm(cond("cannon", 100, ASPECTS_ANY)), cannon_fire(Some(1))],
        )],
    ));
    hold_fire_on(&mut rig.arena, RED);
    let land0 = landing(&rig.arena, 0);
    rig.to(land0);
    assert_eq!(rig.refused(RED), unwarranted);
    assert_eq!(rig.shots(), Vec::new());
    // ... and an UNLAWFUL citation under it `unlawful`: the slot armed at the
    // floor, the floor then raised over it.
    let mut rig = Stepper::new(mind_duel(
        classified,
        &[
            (0, vec![arm(cond("cannon", 100, ASPECTS_ANY))]),
            (1, vec![cannon_fire(Some(1))]),
        ],
    ));
    hold_fire_on(&mut rig.arena, RED);
    let land0 = landing(&rig.arena, 0);
    let land1 = landing(&rig.arena, 1);
    rig.to(land0);
    rig.arena.odds_floor_permille = 200;
    rig.to(land1);
    assert_eq!(
        rig.refused(RED),
        vec![(
            Refusal::Unlawful,
            cannon_word.clone(),
            RefusalDetail::Floor { permille: 200 }
        )]
    );
    assert_eq!(rig.shots(), Vec::new());

    // A seat's typed `FIRE` at nought per cent fires, and misses.
    let (arena, _) = locked_duel(100);
    let mut rig = Stepper {
        wall: Some(arena.now_ms),
        arena,
        bytes: Vec::new(),
    };
    rig.arena.resolution.p_min = 0;
    rig.arena.resolution.p_max = 0;
    let fire = intent(RED, IntentKind::fire("cannon"), 50);
    rig.pulse(&[fire]);
    let until = rig.arena.now_ms + 1_000;
    rig.to(until);
    let words: Vec<String> = rig
        .words()
        .into_iter()
        .filter(|(_, word, _)| matches!(word.as_str(), "fired" | "hit" | "miss"))
        .map(|(_, word, _)| word)
        .collect();
    assert_eq!(
        words,
        vec!["fired".to_owned(), "miss".to_owned()],
        "a pilot's trigger at nought per cent"
    );
    assert!(
        rig.shots()
            .iter()
            .all(|(_, _, _, by, slot)| by.is_none() && slot.is_none()),
        "a seat's shot is never attributed"
    );
}

/// What one mind run produced: every line `(t_ms, kind as JSON, hash)`, every
/// substep's hash, and every stock think kept. **17b cut M2.6**: and, on the
/// run [`at_three_cadences`] answers, how many thinks its twin-flown run
/// compared T1 against T2 (`twinned`; 0 where no twin flew), so a T1 fixture
/// asserts it was held to Gate A rather than trusting the keep arm's trigger.
/// **17b cut M2.7**: and how many thinks its three runs flown by the Rhai
/// Standard Doctrine ALONE made (`rhai_alone`; 0 where none flew), each run
/// equal to the native one.
#[derive(Debug)]
struct MindRun {
    events: Vec<(i64, String, u64)>,
    hashes: Vec<u64>,
    thoughts: Vec<Thought>,
    pulses: u32,
    twinned: usize,
    rhai_alone: usize,
}

impl MindRun {
    /// The `fired` lines, as [`Shot`]s.
    fn shots(&self) -> Vec<Shot> {
        self.events
            .iter()
            .filter_map(|(t_ms, json, _)| {
                let value: serde_json::Value = serde_json::from_str(json).ok()?;
                (value.get("kind")? == "fired").then(|| {
                    (
                        *t_ms,
                        value
                            .get("weapon")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        value
                            .get("target")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0),
                        value
                            .get("by")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                        value.get("slot").and_then(serde_json::Value::as_u64),
                    )
                })
            })
            .collect()
    }

    /// The `(t_ms, unit)` of every `kind` line.
    fn words(&self, kind: &str) -> Vec<(i64, Option<u64>)> {
        self.events
            .iter()
            .filter_map(|(t_ms, json, _)| {
                let value: serde_json::Value = serde_json::from_str(json).ok()?;
                (value.get("kind")? == kind)
                    .then(|| (*t_ms, value.get("unit").and_then(serde_json::Value::as_u64)))
            })
            .collect()
    }

    /// The `refused` notices `unit`'s kept think at `t_ms` was handed, `(word,
    /// weapon, detail)` — what the landings before it buffered (a think must
    /// have run there).
    fn refused_at(&self, unit: ObjId, t_ms: i64) -> Vec<(Refusal, Option<SmolStr>, RefusalDetail)> {
        let Some((_, _, view, _)) = self
            .thoughts
            .iter()
            .find(|(by, at, ..)| *by == unit && *at == t_ms)
        else {
            panic!("#{} kept no think at {t_ms}", unit.0);
        };
        view.notices
            .iter()
            .filter_map(|notice| match notice {
                FrameNotice::Refused {
                    refusal,
                    weapon,
                    detail,
                    ..
                } => Some((*refusal, weapon.clone(), *detail)),
                _ => None,
            })
            .collect()
    }

    /// Every `refused` notice a kept think of `unit` was handed.
    fn told(&self, unit: ObjId) -> Vec<Refusal> {
        self.thoughts
            .iter()
            .filter(|(by, ..)| *by == unit)
            .flat_map(|(_, _, view, _)| {
                view.notices.iter().filter_map(|notice| match notice {
                    FrameNotice::Refused { refusal, .. } => Some(*refusal),
                    _ => None,
                })
            })
            .collect()
    }
}

/// A change a run makes to the arena at a millisecond every cadence's pulses
/// stop on (a multiple of 5,000), before the pulse that starts there.
type Edit<'a> = (i64, &'a dyn Fn(&mut Arena));

/// **Step `arena` at a fixed wall `cadence_ms` to `until_ms`**: every beat
/// `(t_ms, kind)` delivered to red at its current epoch in the pulse that
/// STARTS at `t_ms` (a multiple of 5,000, so the same millisecond at every
/// cadence; an `Order` and a `ReleaseOrder` land in phase 2, at the pulse's
/// start), every edit made before that pulse, and `each` called after every
/// pulse. The tempo is pinned `Quiet`.
fn mind_run(
    mut arena: Arena,
    kept: Option<&Kept>,
    cadence_ms: i64,
    until_ms: i64,
    beats: &[(i64, IntentKind)],
    edits: &[Edit<'_>],
    each: &mut dyn FnMut(&Arena),
) -> MindRun {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    let (mut next_beat, mut next_edit) = (0_usize, 0_usize);
    let mut seq = 0_u64;
    loop {
        while let Some((at, edit)) = edits.get(next_edit) {
            if *at > arena.now_ms {
                break;
            }
            edit(&mut arena);
            next_edit += 1;
        }
        let mut intents: Vec<Intent> = Vec::new();
        while let Some((at, kind)) = beats.get(next_beat) {
            if *at > arena.now_ms {
                break;
            }
            seq += 1;
            intents.push(current(&arena, RED, kind.clone(), seq));
            next_beat += 1;
        }
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        // 17b cut M4.1: settled between pulses — the pooled path, done
        // synchronously; at 5,000 ms most thinks settle inline at the next
        // substep's head, at 500 ms every other one crosses a pulse.
        let out = arena.step_settled(wall, &[], &intents);
        absorb(&out, &mut bytes, &mut Vec::new());
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        each(&arena);
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events = lines(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    MindRun {
        events,
        hashes,
        thoughts: kept.map(Kept::thoughts).unwrap_or_default(),
        pulses,
        twinned: 0,
        rhai_alone: 0,
    }
}

/// **17b cut M2.6: the T1 fixture's run was held to Gate A** — its field's
/// fourth run flew an asserting twin and compared at least one think
/// (`MindRun::twinned`, set by [`at_three_cadences`]' keep arm). **17b cut
/// M2.7**: and its three runs on the Rhai Standard Doctrine ALONE, at 5,000,
/// 1,000 and 500 ms, thought (`MindRun::rhai_alone`) — each equal to the
/// native run, which the keep arm asserts.
fn assert_twinned(run: &MindRun) {
    assert!(
        run.twinned > 0,
        "the T1 field flew no think under the twin (the keep arm did not fire)"
    );
    assert!(
        run.rhai_alone > 0,
        "the T1 field flew no think on the Rhai runner alone (the keep arm did not fire)"
    );
}

/// **[`mind_run`] at 5,000, 1,000 and 500 ms**, a fresh arena and recorder
/// each: one event list, one hash sequence and, with `keep`, one sequence of
/// the arena's runner's thinks ([`Kept`]: each view — its buffered refusals
/// included — and each answer) — the mandatory cross-tempo property (16b
/// §11). Answers the 5,000 ms run.
///
/// **17b cut M2.6 — the T1 fixtures under the twin** (doc 17 §17 "Gate A";
/// 17b §1 (iii)): with `keep`, on a field whose runner is the production
/// stock mind (T1, `NativeStockMind` — every stock field; never a
/// [`Scripted`] one), the run is played a fourth time at 5,000 ms with the
/// arena's runner made an asserting `support::TwinRunner` BEFORE the
/// recorder reads it (`support::with_twin`): every think T1 and T2 (the Rhai
/// Standard Doctrine) on one job — the Rhai fault refused first, then the
/// intents, then the memory bytes — with T1's answer flown, so the twin-flown
/// run is the native one: one event list, one hash sequence, one sequence of
/// thinks; the twin compared every kept think (`each` is not called again).
/// The answered run carries the count (`MindRun::twinned`), and every T1
/// fixture held to Gate A here asserts it nonzero ([`twinned`]): the keep
/// arm's trigger (the runner's printed name) cannot stop firing for one of
/// them without that fixture failing.
///
/// **17b cut M2.7 — the T1 fixtures under the Rhai runner ALONE** (doc 17
/// §22's "Gate A on the cross-tempo fixture"): the same arm then plays the
/// stock field at 5,000, 1,000 and 500 ms with the arena's runner the Rhai
/// Standard Doctrine alone (`support::with_rhai`, before the recorder reads
/// it) — T2's own answers flown, so unlike the twin's run these prove T2 ≡
/// T1 on the fixture — each with the native run's event list, hash sequence
/// and sequence of thinks; the thinks flown are carried
/// (`MindRun::rhai_alone`) and asserted nonzero beside `twinned`.
fn at_three_cadences(
    make: &dyn Fn() -> Arena,
    keep: bool,
    until_ms: i64,
    beats: &[(i64, IntentKind)],
    edits: &[Edit<'_>],
    each: &mut dyn FnMut(&Arena),
) -> MindRun {
    // Every cadence's last pulse must stop on the same millisecond.
    assert_eq!(
        until_ms.rem_euclid(5_000),
        0,
        "a run ends on a 5,000 ms boundary"
    );
    let mut runs: Vec<(i64, MindRun)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = make();
        let kept = keep.then(|| Kept::on(&arena));
        if let Some(kept) = &kept {
            arena.mind_runner = kept.clone();
        }
        let run = mind_run(
            arena,
            kept.as_deref(),
            cadence,
            until_ms,
            beats,
            edits,
            each,
        );
        runs.push((cadence, run));
    }
    let (_, mut first) = runs.remove(0);
    for (cadence, run) in &runs {
        if let Some(at) = (0..run.events.len().max(first.events.len()))
            .find(|at| run.events.get(*at) != first.events.get(*at))
        {
            panic!(
                "the event list differs at {cadence} ms, line {at}: {:?} against {:?}",
                run.events.get(at),
                first.events.get(at)
            );
        }
        assert_eq!(
            run.hashes, first.hashes,
            "the hashes differ at {cadence} ms"
        );
        assert_eq!(
            run.thoughts, first.thoughts,
            "a think differs at {cadence} ms"
        );
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
    // 17b cut M2.6: a stock field once more, flown by the twin.
    if keep && format!("{:?}", make().mind_runner).starts_with("NativeStockMind") {
        let mut arena = make();
        let twin = support::with_twin(&mut arena);
        let kept = Kept::on(&arena);
        arena.mind_runner = kept.clone();
        let run = mind_run(
            arena,
            Some(&kept),
            5_000,
            until_ms,
            beats,
            edits,
            &mut |_| {},
        );
        assert_eq!(run.events, first.events, "the event list under the twin");
        assert_eq!(run.hashes, first.hashes, "the hashes under the twin");
        assert_eq!(run.thoughts, first.thoughts, "a think under the twin");
        assert_eq!(twin.divergences(), (0, 0));
        assert_eq!(
            twin.thoughts().len(),
            run.thoughts.len(),
            "the twin compared every kept think"
        );
        assert!(!run.thoughts.is_empty(), "the stock field thought");
        println!(
            "m2_conditions_under_the_twin: thinks_compared={}",
            run.thoughts.len()
        );
        first.twinned = run.thoughts.len();
        // 17b cut M2.7 (doc 17 §22's "Gate A on the cross-tempo fixture"):
        // the stock field three times more, flown by the Rhai Standard
        // Doctrine ALONE (`support::with_rhai`, applied before the recorder
        // reads the arena's runner) — T2's own answers flown, nothing native
        // beside it — at 5,000, 1,000 and 500 ms: each the native run's event
        // list, hash sequence and sequence of thinks (each view and each
        // answer, the Rhai trace among it).
        let mut flown = 0_usize;
        for cadence in [5_000_i64, 1_000, 500] {
            let mut arena = make();
            support::with_rhai(&mut arena);
            let kept = Kept::on(&arena);
            arena.mind_runner = kept.clone();
            let run = mind_run(
                arena,
                Some(&kept),
                cadence,
                until_ms,
                beats,
                edits,
                &mut |_| {},
            );
            if let Some(at) = (0..run.events.len().max(first.events.len()))
                .find(|at| run.events.get(*at) != first.events.get(*at))
            {
                panic!(
                    "the event list on the Rhai runner alone differs at {cadence} ms, line \
                     {at}: {:?} against the native {:?}",
                    run.events.get(at),
                    first.events.get(at)
                );
            }
            assert_eq!(
                run.hashes, first.hashes,
                "the hashes on the Rhai runner alone at {cadence} ms"
            );
            if let Some(at) = (0..run.thoughts.len().max(first.thoughts.len()))
                .find(|at| run.thoughts.get(*at) != first.thoughts.get(*at))
            {
                panic!(
                    "think {at} on the Rhai runner alone differs at {cadence} ms:\n  Rhai: \
                     {:?}\n  native: {:?}",
                    run.thoughts.get(at),
                    first.thoughts.get(at)
                );
            }
            assert!(!run.thoughts.is_empty(), "the stock field thought");
            flown += run.thoughts.len();
        }
        println!("m2_conditions_under_the_rhai_runner: thinks_flown={flown}");
        first.rhai_alone = flown;
    }
    first
}

/// The `Condition` rows of one think's answer.
fn armings(intents: &[IntentKind]) -> Vec<&Arming> {
    intents
        .iter()
        .filter_map(|kind| match kind {
            IntentKind::Condition(arming) => Some(arming),
            _ => None,
        })
        .collect()
}

/// Whether a think's answer arms anything.
fn arms(intents: &[IntentKind]) -> bool {
    armings(intents)
        .iter()
        .any(|arming| matches!(arming, Arming::Arm(_)))
}

/// **The law's cross-tempo arm** (16b §11): the citation at the gun's
/// `WeaponReady` of [`m1b2_the_law_refuses_an_uncited_and_an_unwarranted_mind_fire_and_a_cycling_one_reads_cycling`]
/// — the slot armed at think 0 fires at its landing, fires again at the
/// `WeaponReady` that falls on think 4's landing, and the citation there reads
/// `cycling` (in think 5's buffer, the scripted mind's views kept) — at 5,000,
/// 1,000 and 500 ms: one event list, one hash sequence, one sequence of views.
#[test]
fn m1b2_the_cited_fire_at_a_weapon_ready_reads_cycling_at_three_cadences() {
    let classified = Classification::Classified { team: Team::Blue };
    let make = || {
        mind_duel(
            classified,
            &[
                (0, vec![arm(cond("cannon", 100, ASPECTS_ANY))]),
                (4, vec![cannon_fire(Some(1))]),
            ],
        )
    };
    let run = at_three_cadences(&make, true, 10_000, &[], &[], &mut |_| {});
    let land0 = think_at(RED, 0) + make().substep_len_ms();
    let shots: Vec<(i64, Option<String>, Option<u64>)> = run
        .shots()
        .into_iter()
        .map(|(t_ms, _, _, by, slot)| (t_ms, by, slot))
        .collect();
    let cycle = i64::from(cannon().cycle_ms);
    assert_eq!(
        shots,
        vec![
            (land0, Some("cond".to_owned()), Some(1)),
            (land0 + cycle, Some("cond".to_owned()), Some(1)),
            (land0 + 2 * cycle, Some("cond".to_owned()), Some(1)),
        ],
        "the slot's own shots, at its landing and at each WeaponReady"
    );
    assert_eq!(
        run.refused_at(RED, think_at(RED, 5)),
        vec![(
            Refusal::Cycling,
            Some(SmolStr::new("cannon")),
            RefusalDetail::Cycling { secs: 4 }
        )],
        "the citation read cycling at its landing, never unwarranted"
    );
}

/// **Every case of the law at 5,000, 1,000 and 500 ms** (16b §11; 17b cut
/// M1b.2's law, doc 17 §5, D-UM-17 as amended, D-UM-19): the cases of
/// [`m1b2_the_law_refuses_an_uncited_and_an_unwarranted_mind_fire_and_a_cycling_one_reads_cycling`]
/// each run 10 s on the mind duel at the three cadences through
/// [`at_three_cadences`] with the scripted mind's views kept — so the
/// buffered refusal, which writes no ledger line, is compared across the
/// cadences beside the event list and the hashes — and the refusals the think
/// after the citation's landing was handed asserted exactly, with their weapon
/// and detail:
///
/// * uncited — `unwarranted`, nothing ever fires;
/// * the cannon citing the machine gun's slot (on terms the plot meets) —
///   `unwarranted`; the machine gun's own slot fires at the landing and the
///   cannon never fires;
/// * an arming at 50 per mille — `unlawful` with `Floor { 100 }`, then the
///   citation `unwarranted`; nothing armed, nothing fires;
/// * a slot at 600 per mille on the 500 plot — `unwarranted`, nothing fires;
/// * a slot that holds — nothing refused, the cited shot `by: "mind"` at the
///   landing;
/// * an `X` lock — `unwarranted`, nothing fires at the landing;
/// * a citation at the gun's `WeaponReady` (think 8's landing, two cycles
///   after the slot's first shot) with the floor raised over the slot at
///   5,000 — `cycling`: the weapon before the law;
/// * under a standing hold-fire, a lawful citation `unwarranted` and nothing
///   fires; with the floor raised at 5,000 over a slot armed before it, the
///   citation at think 6 `unlawful` with `Floor { 200 }`: the floor before
///   hold-fire.
#[test]
fn m1b2_the_law_reads_every_case_the_same_at_three_cadences() {
    let classified = Classification::Classified { team: Team::Blue };
    let cannon_word = Some(SmolStr::new("cannon"));
    let unwarranted = vec![(
        Refusal::Unwarranted,
        cannon_word.clone(),
        RefusalDetail::None,
    )];
    let substep = mind_duel(classified, &[]).substep_len_ms();
    let land = |n: i64| think_at(RED, n) + substep;
    let raise = |arena: &mut Arena| arena.odds_floor_permille = 200;
    let law =
        |class: Classification, says: Vec<(u64, Vec<IntentKind>)>, holding: bool, raised: bool| {
            let make = || {
                let mut arena = mind_duel(class, &says);
                if holding {
                    hold_fire_on(&mut arena, RED);
                }
                arena
            };
            let edits: Vec<Edit<'_>> = if raised {
                vec![(5_000, &raise)]
            } else {
                Vec::new()
            };
            at_three_cadences(&make, true, 10_000, &[], &edits, &mut |_| {})
        };
    let at = |run: &MindRun, n: i64| -> Vec<(i64, String, Option<String>)> {
        run.shots()
            .into_iter()
            .filter(|(t_ms, ..)| *t_ms == land(n))
            .map(|(t_ms, weapon, _, by, _)| (t_ms, weapon, by))
            .collect()
    };
    let cited = |min_permille: u16| {
        vec![
            arm(cond("cannon", min_permille, ASPECTS_ANY)),
            cannon_fire(Some(1)),
        ]
    };
    // Uncited.
    let run = law(classified, vec![(0, vec![cannon_fire(None)])], false, false);
    assert_eq!(run.refused_at(RED, think_at(RED, 1)), unwarranted);
    assert_eq!(run.shots(), Vec::new());
    // Another gun's slot, on terms the plot meets.
    let run = law(
        classified,
        vec![(
            0,
            vec![
                arm(cond("machine_gun", 100, ASPECTS_ANY)),
                cannon_fire(Some(1)),
            ],
        )],
        false,
        false,
    );
    assert_eq!(run.refused_at(RED, think_at(RED, 1)), unwarranted);
    assert_eq!(
        at(&run, 0),
        vec![(land(0), "machine_gun".to_owned(), Some("cond".to_owned()))]
    );
    assert!(
        run.shots().iter().all(|(_, weapon, ..)| weapon != "cannon"),
        "the cannon never fired on the machine gun's warrant"
    );
    // An arming under the floor, then its citation.
    let run = law(classified, vec![(0, cited(50))], false, false);
    assert_eq!(
        run.refused_at(RED, think_at(RED, 1)),
        vec![
            (
                Refusal::Unlawful,
                cannon_word.clone(),
                RefusalDetail::Floor { permille: 100 }
            ),
            (
                Refusal::Unwarranted,
                cannon_word.clone(),
                RefusalDetail::None
            ),
        ]
    );
    assert_eq!(run.shots(), Vec::new());
    // A slot whose odds term the plot does not meet.
    let run = law(classified, vec![(0, cited(600))], false, false);
    assert_eq!(run.refused_at(RED, think_at(RED, 1)), unwarranted);
    assert_eq!(run.shots(), Vec::new());
    // A slot that holds.
    let run = law(classified, vec![(0, cited(100))], false, false);
    assert_eq!(run.refused_at(RED, think_at(RED, 1)), Vec::new());
    assert_eq!(
        at(&run, 0),
        vec![(land(0), "cannon".to_owned(), Some("mind".to_owned()))]
    );
    // An `X` lock.
    let run = law(Classification::Unknown, vec![(0, cited(100))], false, false);
    assert_eq!(run.refused_at(RED, think_at(RED, 1)), unwarranted);
    assert_eq!(at(&run, 0), Vec::new());
    // A citation at the gun's `WeaponReady`, the floor raised over the slot.
    let cycle = i64::from(cannon().cycle_ms);
    assert_eq!(
        land(8) - land(0),
        2 * cycle,
        "the precondition: think 8 lands on the slot's third WeaponReady"
    );
    assert!(land(0) < 5_000 && land(8) > 5_000, "the edit between");
    let run = law(
        classified,
        vec![
            (0, vec![arm(cond("cannon", 100, ASPECTS_ANY))]),
            (8, vec![cannon_fire(Some(1))]),
        ],
        false,
        true,
    );
    assert_eq!(
        run.refused_at(RED, think_at(RED, 9)),
        vec![(
            Refusal::Cycling,
            cannon_word.clone(),
            RefusalDetail::Cycling { secs: 4 }
        )]
    );
    assert_eq!(
        at(&run, 8),
        vec![(land(8), "cannon".to_owned(), Some("cond".to_owned()))],
        "the slot's own shot at the WeaponReady"
    );
    // Under a standing hold-fire: lawful, then unlawful.
    let run = law(classified, vec![(0, cited(100))], true, false);
    assert_eq!(run.refused_at(RED, think_at(RED, 1)), unwarranted);
    assert_eq!(run.shots(), Vec::new());
    assert!(
        land(0) < 5_000 && land(6) > 5_000,
        "the precondition: the edit between the arming and the citation"
    );
    let run = law(
        classified,
        vec![
            (0, vec![arm(cond("cannon", 100, ASPECTS_ANY))]),
            (6, vec![cannon_fire(Some(1))]),
        ],
        true,
        true,
    );
    assert_eq!(
        run.refused_at(RED, think_at(RED, 7)),
        vec![(
            Refusal::Unlawful,
            cannon_word.clone(),
            RefusalDetail::Floor { permille: 200 }
        )]
    );
    assert_eq!(run.shots(), Vec::new());
}

/// **The stock field**: red at (5, 8) facing east with the cannon, flown by
/// `controller` (a stock mind, T1, or the reflex controller, T0), online,
/// seeing ten cells, handed `targets` as classified rows (`(id, cell,
/// heading, presents)`), each blue seated and unarmed. A blue that does not
/// present carries the default profile: every shot at it is at nought per
/// cent.
fn stock_field(
    def: ArenaDefinition,
    controller: Controller,
    targets: &[(ObjId, (u16, u16), u32, bool)],
) -> Arena {
    let mut units = vec![
        CustodyUnit {
            controller,
            ..unit(
                RED,
                Team::Red,
                facing(5, 8, 90_000),
                vec![cannon()],
                Some(wide_sight()),
            )
        }
        .sealed(),
    ];
    for (id, (x, y), heading, presents) in targets {
        let mut blue = unit(*id, Team::Blue, facing(*x, *y, *heading), Vec::new(), None);
        if !presents {
            blue.profile = UnitProfile::default();
        }
        units.push(blue);
    }
    let seated: Vec<ObjId> = targets.iter().map(|(id, ..)| *id).collect();
    let mut arena = seated_arena_on(def, units, &seated);
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
    }
    for (id, ..) in targets {
        hold_row(
            &mut arena,
            RED,
            *id,
            Classification::Classified { team: Team::Blue },
        );
    }
    arena
}

/// The test window with cell (13, 2) walled off (`move_cost` nought): a cell
/// no route reaches.
fn walled() -> ArenaDefinition {
    let mut def = definition();
    let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
    if let Some(cell) = cells.get_mut(2 * usize::from(SIDE) + 13) {
        *cell = TerrainCell {
            class: mush_atlas::TerrainClass::Ground,
            elev_cm: 0,
            move_cost: 0,
        };
    }
    def.terrain = ArenaTerrain::new(SIDE, SIDE, CELL_CM, cells).expect("the walled window");
    def
}

/// Red's squad (`red-1`).
const RED_ONE: tacsim::deliberation::squad::SquadId = tacsim::deliberation::squad::SquadId {
    team: Team::Red,
    index: 1,
};

/// A persona's hold-fire `objective` order to red's squad, standing `ttl_ms`.
fn hold_fire_order(objective: tacsim::deliberation::order::Objective, ttl_ms: u32) -> IntentKind {
    use tacsim::deliberation::order::{CharacterRef, Constraints, Order};
    IntentKind::Order(Box::new(Order {
        squad: RED_ONE,
        objective,
        target: None,
        horizon_ms: ttl_ms,
        constraints: Constraints {
            hold_fire: true,
            ..Constraints::default()
        },
        ttl_ms,
        issued_ms: 0,
        issuer: None,
        by: CharacterRef::Persona { squad: RED_ONE },
        spot: None,
    }))
}

/// **Hold-fire stops a mind and its slot, and never a pilot** (17b cut
/// M1b.2; doc 17 §5's precedence, D-UM-19; 17b's Q-M7), at 5,000, 1,000 and
/// 500 ms:
///
/// 1. a stock mind (T1) under a persona's `ENGAGE` with `hold_fire`: no
///    think that reaches its target ARMS — 17b cut M1b.3: it answers a
///    `Condition(Disarm)` only beside a slot its view shows armed, and the
///    order landed before any arming, so its view never shows one and it
///    answers none — and it takes the lock: the lock lands, nothing fires;
///    the order released at 10,000, the next think ARMS and the slot fires at
///    that think's landing (`by: "cond"`, S1). (A mind's lawful citation
///    under the order reads `unwarranted`: the law test's.)
/// 2. a pilot's typed `FIRE` under no order fires;
/// 3. **`hold_after_end`** (Q-M7's default, declared): a hold-fire `ADVANCE`
///    to a walled cell no route reaches, standing 12 s. T1 only stops obeying
///    it (`ignore_until_ms`, the order's own `expires_ms` since 17b cut
///    M1b.5), arms while the arena's row still holds it — and every shot of
///    its slot, and its cited one, is refused at the resolver (the ARENA's row
///    holds the hold-fire) until `expires_ms`, when the landing that reaches
///    the deadline clears the row (`order_expired`) and the slot fires. No
///    `order_expired` is journalled at the failed search (doc 16 §11's
///    `order_end`).
///
/// **17b cut M4.5a**: leg 3's T0 contrast — the reflex machine on the same
/// field ending the order at its failed search (`order_expired` at once) and
/// firing before 12 s — is deleted with the controller (the twin's T0 column
/// retired); the T1 half above is every claim the leg makes about a mind, and
/// is unchanged. This test is also the named mirror of the deleted
/// `m1b1_hold_fire_stops_a_slot_and_never_a_pilot` (§ 8).
#[test]
fn m1b2_hold_fire_stops_a_mind_and_its_slot_and_never_a_pilot() {
    use tacsim::deliberation::order::Objective;
    // 1. T1 under a hold-fire ENGAGE, released at 10,000.
    let make = || {
        stock_field(
            definition(),
            Controller::Mind,
            &[(BLUE, (9, 8), 270_000, true)],
        )
    };
    let beats = [
        (
            0,
            hold_fire_order(Objective::Engage { target_hint: None }, 600_000),
        ),
        (
            10_000,
            IntentKind::ReleaseOrder {
                squad: RED_ONE,
                issuer: None,
            },
        ),
    ];
    let run = at_three_cadences(&make, true, 15_000, &beats, &[], &mut |_| {});
    assert_twinned(&run);
    let held: Vec<&Thought> = run
        .thoughts
        .iter()
        .filter(|(_, t_ms, view, _)| {
            *t_ms < 10_000 && view.order.as_ref().is_some_and(|order| order.hold_fire)
        })
        .collect();
    assert!(!held.is_empty(), "red thought under the order");
    for (_, t_ms, view, result) in &held {
        // 17b cut M1b.3: nothing armed under the order, so nothing to clear.
        assert!(
            view.weapons.iter().all(|row| row.armed.is_none()),
            "a slot armed under the order at {t_ms}: {:?}",
            view.weapons
        );
        assert_eq!(
            armings(&result.intents),
            Vec::<&Arming>::new(),
            "no arming, and no disarm of nothing, under the order at {t_ms}: {:?}",
            result.intents
        );
        assert!(
            !result
                .intents
                .iter()
                .any(|kind| matches!(kind, IntentKind::Fire { .. })),
            "no citation under the order at {t_ms}"
        );
    }
    assert!(
        held.iter().any(|(_, _, _, result)| result
            .intents
            .iter()
            .any(|kind| matches!(kind, IntentKind::Lock { .. }))),
        "the lock is taken under the order"
    );
    assert!(
        run.words("locked")
            .iter()
            .any(|(t_ms, unit)| *t_ms < 10_000 && *unit == Some(u64::from(RED.0))),
        "and lands"
    );
    let shots = run.shots();
    assert!(
        shots.iter().all(|(t_ms, ..)| *t_ms > 10_000),
        "nothing fires under the order: {shots:?}"
    );
    let Some((_, rearmed, ..)) = run
        .thoughts
        .iter()
        .find(|(_, t_ms, _, result)| *t_ms >= 10_000 && arms(&result.intents))
    else {
        panic!("red re-armed after the release");
    };
    let land = rearmed + make().substep_len_ms();
    assert_eq!(
        shots
            .first()
            .map(|(t_ms, _, _, by, slot)| (*t_ms, by.clone(), *slot)),
        Some((land, Some("cond".to_owned()), Some(1))),
        "the slot fires at the re-arming think's landing: {shots:?}"
    );

    // 2. A pilot's typed FIRE under no order fires.
    let (mut arena, wall) = locked_duel(100);
    let _ = refused_now(&mut arena, wall, vec![IntentKind::fire("cannon")]);
    assert_eq!(fired_lines(&mut arena).len(), 1);

    // 3. `hold_after_end`: T1 under a hold-fire ADVANCE to a wall.
    let to_the_wall = || hold_fire_order(Objective::Advance { to: (13, 2) }, 12_000);
    let t1 = || stock_field(walled(), Controller::Mind, &[(BLUE, (9, 8), 270_000, true)]);
    let mind = at_three_cadences(&t1, true, 20_000, &[(0, to_the_wall())], &[], &mut |_| {});
    assert_twinned(&mind);
    assert!(
        mind.thoughts.iter().any(|(_, _, view, result)| view
            .order
            .as_ref()
            .is_some_and(|order| order.hold_fire)
            && arms(&result.intents)),
        "T1 stops obeying and arms while the arena's row still holds the order"
    );
    // 17b cut M1b.5: it stops obeying THAT order — its memory's
    // `ignore_until_ms` is the order's own `expires_ms` at every such think.
    for (_, t_ms, view, result) in &mind.thoughts {
        let Some(order) = view.order.as_ref().filter(|order| order.hold_fire) else {
            continue;
        };
        if !arms(&result.intents) {
            continue;
        }
        let memory = tacsim::arena::stock_mind::StockMemory::from(
            &tacsim::arena::mind::decode_memory(&result.memory),
        );
        assert_eq!(
            memory.ignore_until_ms,
            Some(order.expires_ms),
            "the ended order's deadline at {t_ms}"
        );
    }
    let ended: Vec<i64> = mind
        .words("order_expired")
        .into_iter()
        .filter(|(_, unit)| *unit == Some(u64::from(RED.0)))
        .map(|(t_ms, _)| t_ms)
        .collect();
    let Some(&ended) = ended.first() else {
        panic!("the arena expires the mind's order");
    };
    assert!(ended >= 12_000, "at its deadline, not before: {ended}");
    let shots = mind.shots();
    assert!(
        shots.iter().all(|(t_ms, ..)| *t_ms >= ended),
        "no shot before the arena's row lets go: {shots:?}"
    );
    assert!(
        shots
            .first()
            .is_some_and(|(_, _, _, by, _)| by.as_deref() == Some("cond")),
        "the slot fires once the row is gone: {shots:?}"
    );
    assert!(
        mind.told(RED).contains(&Refusal::Unwarranted),
        "the mind's citation under the arena's order was refused unwarranted"
    );
}

/// **One unit of `entry`, checked out as `match_::start` checks one out**
/// (`tests/extensions.rs`' `unit_of`, copied): the catalog's own binding and
/// every row the arena reads decoded by the production check-out's functions.
fn shipped_unit(
    catalog: &tacsim::definitions::Catalog,
    entry: &str,
    id: ObjId,
    team: Team,
    pose: Pose,
) -> CustodyUnit {
    let key = CanonicalId::new(entry).expect("a canonical entry id");
    let spec = catalog.specs.get(&key).expect("the unit entry");
    let binding = catalog.bindings.get(&key).expect("the unit binding");
    let cells = tacsim::definitions::to_tactical_unit(spec, binding, id).expect("the unit");
    CustodyUnit {
        id,
        team,
        pose,
        mobility: tacsim::match_::tracked_rules(&cells),
        sensor_height_cm: tacsim::match_::sensor_height_cm(catalog, &cells),
        sensor: tacsim::match_::sensor_profile(catalog, &cells),
        weapons: tacsim::match_::weapon_summaries(catalog, &cells),
        profile: tacsim::match_::unit_profile(catalog, &cells),
        gunnery: 0,
        controller: Controller::Pilot,
        slot: 0,
        unit: cells,
        seal: tacsim::arena::MindSeal::placeholder(),
    }
    .sealed()
}

/// The shipped content with the extension packs (the laser and its tank),
/// through the operator's route.
fn laser_catalog() -> tacsim::definitions::Catalog {
    let operator: Vec<tacsim::config::LoadedPack> =
        tacsim::definitions::content::extension_sources()
            .iter()
            .map(|source| tacsim::config::LoadedPack {
                pack: tacsim::pack::load_pack(&source.bytes)
                    .unwrap_or_else(|error| panic!("{}: {error}", source.name)),
                source: source.name.clone(),
            })
            .collect();
    tacsim::definitions::Catalog::build_game(&operator, tacsim::NS)
        .expect("the extension content binds")
}

/// **The laser field**: red the shipped `line_tank_laser` (`laser` — 20 heat
/// a pulse against a 60 ceiling — then the coaxial `machine_gun`, in
/// component order) at (5, 8) facing east, flown by the stock mind, online,
/// its heat sink stopped (`per_s` nought: a hull that cannot cool, as
/// `tests/extensions.rs`' cooked-platform fixture holds it); blue the shipped
/// `line_tank` at (9, 8) facing west, seated, every consequence taken off its
/// profile so no hit ends the match; red handed blue as a classified row.
/// Answers the arena and red's authored sink.
fn laser_field() -> (Arena, u32) {
    let catalog = laser_catalog();
    let red = minded(shipped_unit(
        &catalog,
        tacsim::definitions::content::LINE_TANK_LASER,
        RED,
        Team::Red,
        facing(5, 8, 90_000),
    ));
    let mut blue = shipped_unit(
        &catalog,
        tacsim::definitions::content::LINE_TANK,
        BLUE,
        Team::Blue,
        facing(9, 8, 270_000),
    );
    for section in &mut blue.profile.sections {
        section.consequences.clear();
    }
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition(),
            units: vec![red, blue],
            pilots: vec![Pilot {
                unit: BLUE,
                pilot: ObjId(1_001),
                station: SmolStr::new("cockpit"),
                epoch: 1,
            }],
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the laser field");
    let mut sink = 0;
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
        if let Some(heat) = state.heat.as_mut() {
            sink = heat.per_s;
            heat.per_s = 0;
        }
    }
    hold_row(
        &mut arena,
        RED,
        BLUE,
        Classification::Classified { team: Team::Blue },
    );
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    (arena, sink)
}

/// When the laser field's sink comes back on.
const COOLS_AT: i64 = 10_000;

/// When the laser field lets red's lock go (an edit: the lock lost), so the
/// next think takes a FRESH lock after the cooling and cites beside it — at
/// the millisecond the laser's slot would otherwise fire it again, so the
/// laser is idle (no lock) at its `WeaponReady`, ready at the landing, and the
/// citation takes it.
const UNLOCKS_AT: i64 = 20_000;

/// **T1 arms its first two weapons whatever the heat, and numbers them as the
/// arena does** (17b cut M1b.2; D-UM-16, D-UM-18; 17b's adversarial F6): the
/// laser field ([`laser_field`]) flown by the stock mind for 25 s at 5,000,
/// 1,000 and 500 ms — one event list, one hash sequence, one sequence of
/// thinks:
///
/// * every think that arms arms the `laser` then the `machine_gun`, S1 and
///   S2, or (17b cut M1b.3: on change) the one of them its view shows
///   otherwise — the hot laser included (no heat skip) — and after every
///   pulse the arena's slots are exactly those two in that order, or none;
///   since a think arms only on a difference, the hot ARM is proven on a
///   second run whose laser slot is cleared at the ceiling (an edit at
///   [`COOLS_AT`], the sink never back on): the next think's view shows the
///   laser unarmed on an overheated platform, it arms the laser all the same,
///   and S1 holds it again;
/// * the fresh lock's citation fires `by: "mind"`; the laser's slot then
///   fires at its own `WeaponReady`s until the platform reaches its ceiling,
///   after which the resolver's `Overheated` holds the laser's slot SILENTLY
///   while the slot stays armed and the machine gun's S2 fires at its own
///   `WeaponReady`s; the sink back on at [`COOLS_AT`], the laser's slot fires
///   again (`by: "cond"`);
/// * red's lock let go at [`UNLOCKS_AT`], the think that takes it again
///   cites the cooled LASER (S1) beside the fresh `Lock` — T1's citation
///   after the cooling — and every citation after the edit fires `by: "mind"`
///   with its slot at its landing, never `unwarranted` (a held lock is never
///   cited: the armed slot owns the shot, so a citation needs a fresh lock);
/// * no think is ever told `unwarranted` or `unlawful`;
/// * at the ceiling a mind's cited laser shot reads `Overheated` — the law
///   (lawful, warranted) passed before the heat — and an uncited one
///   `Unwarranted`: the law before the platform (the gate order's
///   `Overheated` step, which needs the content's heat cells).
#[test]
fn m1b2_t1_arms_its_first_two_weapons_whatever_the_heat_and_numbers_them_as_the_arena_does() {
    let probed = std::cell::Cell::new(false);
    let (_, sink) = laser_field();
    assert!(sink > 0, "the authored sink");
    let cool = |arena: &mut Arena| {
        // The gate order at the ceiling, on a clone, before the sink returns.
        let mut probe = arena.clone();
        assert!(
            probe.unit(RED).is_some_and(|state| state.overheated()),
            "the precondition: at the ceiling by {COOLS_AT}"
        );
        let now = probe.now_ms;
        assert_eq!(
            probe.clone().resolve_fire_by(
                RED,
                "laser",
                now,
                tacsim::arena::weapons::FiredBy::Mind { slot: Some(1) }
            ),
            Err(Refusal::Overheated)
        );
        assert_eq!(
            probe.resolve_fire_by(
                RED,
                "laser",
                now,
                tacsim::arena::weapons::FiredBy::Mind { slot: None }
            ),
            Err(Refusal::Unwarranted)
        );
        probed.set(true);
        if let Some(heat) = arena.unit_mut(RED).and_then(|state| state.heat.as_mut()) {
            heat.per_s = sink;
        }
    };
    let make = || laser_field().0;
    let mut numbered = 0_usize;
    let mut each = |arena: &Arena| {
        if arena.conditions.is_empty_for(RED) {
            return;
        }
        let weapons: Vec<Option<&str>> = arena
            .conditions
            .slots(RED)
            .iter()
            .map(|slot| {
                slot.as_ref()
                    .and_then(Rule::fire)
                    .map(|cond| cond.weapon.as_str())
            })
            .collect();
        // 17b cut M3.2: the row is the HULL's (the laser tank's line core,
        // Class II: four wide) while the mind, thinking at Class I, arms its
        // two.
        assert_eq!(
            weapons,
            vec![Some("laser"), Some("machine_gun"), None, None],
            "the arena's slots at {}",
            arena.now_ms
        );
        numbered += 1;
    };
    let unlock = |arena: &mut Arena| {
        if let Some(state) = arena.unit_mut(RED) {
            state.lock = None;
        }
    };
    let run = at_three_cadences(
        &make,
        true,
        25_000,
        &[],
        &[(COOLS_AT, &cool), (UNLOCKS_AT, &unlock)],
        &mut each,
    );
    assert_twinned(&run);
    assert!(probed.get(), "the ceiling was probed");
    assert!(numbered > 0, "the slots were read");
    // Every arming: the laser, then the machine gun — hot or not. **17b cut
    // M1b.3**: a think answers only the rows its view shows otherwise, so an
    // answer is the two in that order or one of them; and a think at the
    // ceiling leaves the laser's slot STANDING — armed by it, or shown armed
    // and not cleared (no heat skip, 17b's adversarial F6).
    let mut armed_hot = false;
    for (_, t_ms, view, result) in &run.thoughts {
        let weapons: Vec<SmolStr> = armings(&result.intents)
            .into_iter()
            .filter_map(|arming| match arming {
                Arming::Arm(cond) => Some(cond.weapon.clone()),
                Arming::Disarm { .. } => None,
                Arming::Rules { .. } | Arming::Remove { .. } => {
                    panic!("stock doctrine emitted an authored-rule edit")
                }
            })
            .collect();
        let order = [SmolStr::new("laser"), SmolStr::new("machine_gun")];
        assert!(
            weapons.is_empty()
                || weapons == order
                || weapons == [SmolStr::new("laser")]
                || weapons == [SmolStr::new("machine_gun")],
            "at {t_ms}: {weapons:?}"
        );
        let disarmed = armings(&result.intents)
            .iter()
            .any(|arming| matches!(arming, Arming::Disarm { .. }));
        let laser_shown = view.weapons.iter().any(|row| {
            row.id == "laser" && row.armed.as_ref().is_some_and(|armed| armed.slot == 1)
        });
        let laser_stands = weapons.contains(&SmolStr::new("laser")) || (laser_shown && !disarmed);
        if view.heat.is_some_and(|heat| heat.overheated()) && laser_stands {
            armed_hot = true;
        }
    }
    assert!(
        armed_hot,
        "a think at the ceiling left the laser armed all the same"
    );
    // The shots.
    let shots = run.shots();
    let laser: Vec<(i64, Option<String>, Option<u64>)> = shots
        .iter()
        .filter(|(_, weapon, ..)| weapon == "laser")
        .map(|(t_ms, _, _, by, slot)| (*t_ms, by.clone(), *slot))
        .collect();
    assert_eq!(
        laser.first().map(|(_, by, slot)| (by.clone(), *slot)),
        Some((Some("mind".to_owned()), Some(1))),
        "the fresh lock's citation: {laser:?}"
    );
    assert!(laser.len() >= 3, "three pulses to the ceiling: {laser:?}");
    let ceiling = laser.get(2).map_or(0, |(t_ms, ..)| *t_ms);
    assert!(
        laser
            .iter()
            .all(|(t_ms, ..)| *t_ms <= ceiling || *t_ms > COOLS_AT),
        "the laser's slot held at the ceiling until the sink came back: {laser:?}"
    );
    assert!(
        laser.iter().any(|(t_ms, by, slot)| *t_ms > COOLS_AT
            && by.as_deref() == Some("cond")
            && *slot == Some(1)),
        "and fired again once cooler: {laser:?}"
    );
    assert!(
        shots
            .iter()
            .any(|(t_ms, weapon, _, by, slot)| *t_ms > ceiling
                && *t_ms <= COOLS_AT
                && weapon == "machine_gun"
                && by.as_deref() == Some("cond")
                && *slot == Some(2)),
        "the machine gun's S2 fired at its own WeaponReady while the laser held: {shots:?}"
    );
    for (_, _, _, by, slot) in &shots {
        assert!(
            matches!(by.as_deref(), Some("mind" | "cond")) && slot.is_some(),
            "every T1 shot is cited: {shots:?}"
        );
    }
    // The citation after the cooling: the fresh lock's think cites, and the
    // shot is the mind's at the landing.
    let substep = make().substep_len_ms();
    let recited: Vec<(i64, SmolStr, u8)> = run
        .thoughts
        .iter()
        .filter(|(_, t_ms, ..)| *t_ms > UNLOCKS_AT)
        .filter_map(|(_, t_ms, _, result)| {
            result.intents.iter().find_map(|kind| match kind {
                IntentKind::Fire {
                    weapon,
                    slot: Some(n),
                } => Some((*t_ms, weapon.clone(), *n)),
                _ => None,
            })
        })
        .collect();
    assert!(
        recited
            .iter()
            .any(|(_, weapon, n)| weapon == "laser" && *n == 1),
        "the laser cited after the cooling: {recited:?} {laser:?}"
    );
    assert!(
        !recited.is_empty(),
        "a citation after the cooling: {:?}",
        run.thoughts
            .iter()
            .filter(|(_, t_ms, ..)| *t_ms > UNLOCKS_AT)
            .map(|(_, t_ms, _, result)| (*t_ms, result.intents.clone()))
            .collect::<Vec<_>>()
    );
    for (think, weapon, n) in &recited {
        assert!(
            shots
                .iter()
                .any(|(t_ms, fired, _, by, slot)| *t_ms == think + substep
                    && fired == weapon.as_str()
                    && by.as_deref() == Some("mind")
                    && *slot == Some(u64::from(*n))),
            "the citation at {think} fired by the mind: {shots:?}"
        );
    }
    let told = run.told(RED);
    assert!(
        !told.contains(&Refusal::Unwarranted) && !told.contains(&Refusal::Unlawful),
        "{told:?}"
    );

    // **17b cut M1b.3** (F6 on change): the laser's slot cleared at the
    // ceiling, the sink never back on — the next think's view shows the laser
    // unarmed on an overheated platform, and it ARMS it all the same (no heat
    // skip), back in S1.
    let strip = |arena: &mut Arena| {
        assert!(
            arena.unit(RED).is_some_and(|state| state.overheated()),
            "the precondition: at the ceiling by {COOLS_AT}"
        );
        assert_eq!(
            arena.conditions.disarm(RED, Some("laser")),
            vec![1],
            "the laser's S1 freed"
        );
    };
    let hot = at_three_cadences(&make, true, 15_000, &[], &[(COOLS_AT, &strip)], &mut |_| {});
    assert_twinned(&hot);
    let laser_arm = |result: &ThinkResult| {
        armings(&result.intents)
            .iter()
            .any(|arming| matches!(arming, Arming::Arm(cond) if cond.weapon == "laser"))
    };
    let Some((_, rearmed, view, result)) = hot
        .thoughts
        .iter()
        .find(|(_, t_ms, _, result)| *t_ms >= COOLS_AT && laser_arm(result))
    else {
        panic!("no think re-armed the laser after its slot was cleared");
    };
    assert!(
        view.heat.is_some_and(|heat| heat.overheated()),
        "the laser re-armed at {rearmed} on an overheated platform: {:?}",
        view.heat
    );
    assert!(
        view.weapons
            .iter()
            .any(|row| row.id == "laser" && row.armed.is_none()),
        "on a view that showed it unarmed at {rearmed}: {:?}",
        view.weapons
    );
    assert!(
        !armings(&result.intents)
            .iter()
            .any(|arming| matches!(arming, Arming::Arm(cond) if cond.weapon == "machine_gun")),
        "and the machine gun, shown armed as wanted, not re-armed: {:?}",
        result.intents
    );
    let landed = rearmed + substep;
    let after: Vec<Option<u8>> = hot
        .thoughts
        .iter()
        .filter(|(_, t_ms, ..)| *t_ms > landed)
        .map(|(_, _, view, _)| {
            view.weapons
                .iter()
                .find(|row| row.id == "laser")
                .and_then(|row| row.armed.as_ref())
                .map(|armed| armed.slot)
        })
        .collect();
    assert!(
        !after.is_empty() && after.iter().all(|slot| *slot == Some(1)),
        "S1 holds the laser again after the landing: {after:?}"
    );
}

/// **The `cond_stale` rule** (`tests/twin.rs`; 17b §1(f), adversarial F5): a
/// slot's shot whose target is not the object of the latest `Lock` its stock
/// mind PLANNED at a think before that millisecond. Answers the stale shots
/// as `(t_ms, target)`.
fn stale(run: &MindRun) -> Vec<(i64, u64)> {
    let object_of = |contact: ContactId| {
        [BLUE, BLUE2]
            .into_iter()
            .find(|target| contact_of(RED, *target) == contact)
            .map(|target| u64::from(target.0))
    };
    run.shots()
        .into_iter()
        .filter(|(_, _, _, by, _)| by.as_deref() == Some("cond"))
        .filter_map(|(t_ms, _, target, _, _)| {
            let latest = run
                .thoughts
                .iter()
                .filter(|(_, think_ms, ..)| *think_ms < t_ms)
                .filter_map(|(_, think_ms, _, result)| {
                    result.intents.iter().find_map(|kind| match kind {
                        IntentKind::Lock { contact } => Some((*think_ms, *contact)),
                        _ => None,
                    })
                })
                .max_by_key(|(think_ms, _)| *think_ms)
                .and_then(|(_, contact)| object_of(contact));
            (latest.is_some() && latest != Some(target)).then_some((t_ms, target))
        })
        .collect()
}

/// **A retarget think: the slot armed before the lock, the citation on the
/// NEW lock — and a `WeaponReady` inside the window fires at the OLD one**
/// (17b cut M1b.2; 17b's adversarial F5; doc 17 §5, D-UM-16). Two blues at
/// one rounded range, both facing red, so the stock mind's score ties and it
/// settles on the lower contact id (`first`); at a 5,000 ms boundary the
/// other (`second`) turns its rear to red, and the think after red's next
/// pass retargets: `Lock { second }`, then `Fire { S1 }` — with a
/// `Condition(Arm)` before them only where the view shows the slot otherwise
/// (17b cut M1b.3: on change; here the view already shows S1 armed on the
/// best hostile as wanted, so the retarget think answers none and the slot it
/// cites is the view's) — landing in that order with no evaluation between
/// them. At 5,000, 1,000 and 500 ms each:
///
/// * **(a) the citation on the new lock**: `first` presents nothing, so the
///   slot holds at nought per cent on it and the cannon stands ready; the
///   retarget think's cited `Fire` lands at the landing ON `second`, `by:
///   "mind"`, S1 — no `cond_stale`;
/// * **(b) the stale window**: both present, the slot fires at `first` at
///   every `WeaponReady`; the turn falls so that the cannon's `WeaponReady`
///   is at the retarget think's landing millisecond, where it pops before the
///   landing (doc 17 §6's order): the slot fires at the OLD lock — counted
///   ONE `cond_stale` by the twin's rule — and the citation reads `cycling`;
///   the next `WeaponReady`'s shot is at `second`.
#[test]
fn m1b2_a_retarget_think_fires_its_slot_at_the_new_lock_at_the_landing() {
    // The tie goes to the lower contact id: that one is `first`.
    let (first, second) = if contact_of(RED, BLUE) < contact_of(RED, BLUE2) {
        (BLUE, BLUE2)
    } else {
        (BLUE2, BLUE)
    };
    let make = |first_presents: bool| {
        move || {
            stock_field(
                definition(),
                Controller::Mind,
                &[
                    (first, (9, 8), 270_000, first_presents),
                    (second, (9, 9), 270_000, true),
                ],
            )
        }
    };
    let turn = |arena: &mut Arena| {
        if let Some(state) = arena.unit_mut(second) {
            state.pose.heading_mdeg = 90_000;
            state.desired_heading_mdeg = 90_000;
        }
    };
    let retarget = |run: &MindRun| -> (i64, Vec<IntentKind>) {
        run.thoughts
            .iter()
            .find(|(_, _, _, result)| {
                result.intents.contains(&IntentKind::Lock {
                    contact: contact_of(RED, second),
                })
            })
            .map(|(_, t_ms, _, result)| (*t_ms, result.intents.clone()))
            .unwrap_or_else(|| panic!("red never retargeted"))
    };
    let landing = |think_ms: i64| think_ms + make(true)().substep_len_ms();
    let in_order = |intents: &[IntentKind]| {
        let at = |want: fn(&IntentKind) -> bool| intents.iter().position(want);
        let arm = at(|kind| matches!(kind, IntentKind::Condition(Arming::Arm(_))));
        let lock = at(|kind| matches!(kind, IntentKind::Lock { .. }));
        let fire = at(|kind| matches!(kind, IntentKind::Fire { slot: Some(1), .. }));
        // 17b cut M1b.3: on change — the view already shows S1 armed on the
        // best hostile as wanted, so the retarget think answers no `Arm`.
        assert!(
            arm.is_none() && lock.is_some() && lock < fire,
            "no Condition, then Lock, Fire: {intents:?}"
        );
    };
    // 17b cut M1b.3: the retarget think reads S1 off its view's armed row.
    let shown = |run: &MindRun, think: i64| {
        run.thoughts
            .iter()
            .find(|(_, t_ms, ..)| *t_ms == think)
            .and_then(|(_, _, view, _)| view.weapons.first())
            .and_then(|row| row.armed.as_ref())
            .map(|armed| armed.slot)
    };

    // (a) The citation lands on the new lock.
    let run = at_three_cadences(
        &make(false),
        true,
        15_000,
        &[],
        &[(10_000, &turn)],
        &mut |_| {},
    );
    assert_twinned(&run);
    let (think, intents) = retarget(&run);
    assert!(think > 10_000, "after the turn: {think}");
    in_order(&intents);
    assert_eq!(shown(&run, think), Some(1), "the view's S1");
    let after: Vec<Shot> = run
        .shots()
        .into_iter()
        .filter(|(t_ms, ..)| *t_ms >= think)
        .collect();
    assert_eq!(
        after.first(),
        Some(&(
            landing(think),
            "cannon".to_owned(),
            u64::from(second.0),
            Some("mind".to_owned()),
            Some(1)
        )),
        "{after:?}"
    );
    assert!(
        run.shots()
            .iter()
            .all(|(_, _, target, ..)| *target != u64::from(first.0)),
        "nothing ever fired at the blue that presents nothing"
    );
    assert_eq!(stale(&run), Vec::new());

    // (b) The window: the turn timed so the cannon's `WeaponReady` falls on
    // the retarget think's landing.
    let run = at_three_cadences(
        &make(true),
        true,
        25_000,
        &[],
        &[(20_000, &turn)],
        &mut |_| {},
    );
    assert_twinned(&run);
    let (think, intents) = retarget(&run);
    in_order(&intents);
    assert_eq!(shown(&run, think), Some(1), "the view's S1");
    let land = landing(think);
    let shots: Vec<(i64, u64, Option<String>)> = run
        .shots()
        .into_iter()
        .map(|(t_ms, _, target, by, _)| (t_ms, target, by))
        .collect();
    assert!(
        shots.contains(&(land, u64::from(first.0), Some("cond".to_owned()))),
        "the WeaponReady at the landing fired at the OLD lock: {shots:?}"
    );
    assert_eq!(
        stale(&run),
        vec![(land, u64::from(first.0))],
        "counted once by the twin's rule"
    );
    assert!(
        run.told(RED).contains(&Refusal::Cycling),
        "the citation read cycling"
    );
    assert!(
        shots.iter().any(|(t_ms, target, by)| *t_ms > land
            && *target == u64::from(second.0)
            && by.as_deref() == Some("cond")),
        "the next WeaponReady's shot is at the new lock: {shots:?}"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M1b.3: T1 reads its slots off its view, landed at three cadences
// ---------------------------------------------------------------------------

/// When the disarm fixture's hold-fire order lands: after red's first arming
/// has landed, so its view shows the slot armed.
const ORDERED_AT: i64 = 5_000;

/// **T1 disarms once, on change, when a hold-fire lands on its armed slot**
/// (17b cut M1b.3; D-UM-4's parity rule; doc 17 §5's precedence, D-UM-19).
/// The stock field's T1, unordered, arms its cannon (S1) at the think that
/// finds blue; a persona's hold-fire `ENGAGE` lands at [`ORDERED_AT`]. At
/// 5,000, 1,000 and 500 ms — one event list, one hash sequence, one sequence
/// of thinks:
///
/// * a think before the order arms, and the slot stands (S1, the best
///   hostile) on every view from its landing to the order;
/// * the FIRST think under the order answers exactly one `Condition`, a
///   `Disarm { weapon: None }`, beside a view that shows S1 armed;
/// * from that think's landing on the arena holds no slot for red (after
///   every pulse), and every later think under the order sees nothing armed
///   and answers no `Condition` — nothing to clear is nothing said;
/// * nothing fires after the order lands (the slot is gated by the order
///   until the disarm lands, and is gone after it).
#[test]
fn m1b3_t1_disarms_once_on_change_when_a_hold_fire_lands_on_its_armed_slot() {
    use tacsim::deliberation::order::Objective;
    let make = || {
        stock_field(
            definition(),
            Controller::Mind,
            &[(BLUE, (9, 8), 270_000, true)],
        )
    };
    let beats = [(
        ORDERED_AT,
        hold_fire_order(Objective::Engage { target_hint: None }, 600_000),
    )];
    let mut tables: Vec<(i64, bool)> = Vec::new();
    let run = at_three_cadences(&make, true, 15_000, &beats, &[], &mut |arena| {
        tables.push((arena.now_ms, arena.conditions.is_empty_for(RED)));
    });
    assert_twinned(&run);
    let substep = make().substep_len_ms();
    // Armed before the order, and shown armed on every view after that.
    let Some((_, armed_at, ..)) = run
        .thoughts
        .iter()
        .find(|(_, t_ms, _, result)| *t_ms < ORDERED_AT && arms(&result.intents))
    else {
        panic!("red never armed before the order");
    };
    let shows_s1 = |view: &MindView| {
        view.weapons
            .first()
            .and_then(|row| row.armed.as_ref())
            .is_some_and(|armed| armed.slot == 1 && armed.target == SlotTarget::BestHostile)
    };
    for (_, t_ms, view, _) in &run.thoughts {
        if *t_ms >= armed_at + substep && *t_ms < ORDERED_AT {
            assert!(
                shows_s1(view),
                "the view's S1 at {t_ms}: {:?}",
                view.weapons
            );
        }
    }
    // The first think under the order: the one disarm, beside an armed row.
    let under: Vec<&Thought> = run
        .thoughts
        .iter()
        .filter(|(_, _, view, _)| view.order.as_ref().is_some_and(|order| order.hold_fire))
        .collect();
    let Some((_, cleared_at, view, result)) = under.first() else {
        panic!("red never thought under the order");
    };
    assert!(*cleared_at >= ORDERED_AT, "under the order at {cleared_at}");
    assert!(
        shows_s1(view),
        "the disarming think's view shows S1: {:?}",
        view.weapons
    );
    assert_eq!(
        armings(&result.intents),
        vec![&Arming::Disarm { weapon: None }],
        "one disarm, on change, at {cleared_at}: {:?}",
        result.intents
    );
    let landed = cleared_at + substep;
    // After its landing: no slot, nothing said.
    let mut later = 0_usize;
    for (_, t_ms, view, result) in under.iter().skip(1) {
        assert!(
            view.weapons.iter().all(|row| row.armed.is_none()),
            "a slot shown armed after the disarm landed, at {t_ms}: {:?}",
            view.weapons
        );
        assert_eq!(
            armings(&result.intents),
            Vec::<&Arming>::new(),
            "a Condition said under the order at {t_ms}: {:?}",
            result.intents
        );
        later += 1;
    }
    assert!(later > 0, "red thought again under the order");
    let after: Vec<&(i64, bool)> = tables.iter().filter(|(t_ms, _)| *t_ms >= landed).collect();
    assert!(
        !after.is_empty() && after.iter().all(|(_, empty)| *empty),
        "the arena's slots after the disarm's landing at {landed}: {after:?}"
    );
    assert!(
        tables
            .iter()
            .any(|(t_ms, empty)| *t_ms < ORDERED_AT && !*empty),
        "the slot stood before the order: {tables:?}"
    );
    let shots = run.shots();
    assert!(
        shots.iter().any(|(t_ms, ..)| *t_ms < ORDERED_AT),
        "the slot fired before the order: {shots:?}"
    );
    assert!(
        shots.iter().all(|(t_ms, ..)| *t_ms <= ORDERED_AT),
        "nothing fires once the order lands: {shots:?}"
    );
}

/// When the citation fixture's edit takes red's turret and its lock.
const TURRET_LOST_AT: i64 = 10_000;

/// **The two-gun stock field**: [`stock_field`]'s red flown by the stock mind
/// (T1) with the cannon on its turret AND the machine gun on its hull, in
/// that component order — T1 arms them S1 and S2 — and blue presenting at
/// (9, 8), handed red as a classified row.
fn two_gun_field() -> Arena {
    let units = vec![
        minded(unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![cannon(), machine_gun()],
            Some(wide_sight()),
        )),
        unit(BLUE, Team::Blue, facing(9, 8, 270_000), Vec::new(), None),
    ];
    let mut arena = seated_arena(units, &[BLUE]);
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
    }
    hold_row(
        &mut arena,
        RED,
        BLUE,
        Classification::Classified { team: Team::Blue },
    );
    arena
}

/// **T1 cites the slot its view shows after its S1 gun is lost** (17b cut
/// M1b.3; D-UM-4: a mind reads its own armed slots exactly as the pilot does;
/// M1b.2's residual (a), removed, landed). [`two_gun_field`] flown by T1 for
/// 15 s at 5,000, 1,000 and 500 ms — one event list, one hash sequence, one
/// sequence of thinks. T1 arms the cannon S1 and the machine gun S2; at
/// [`TURRET_LOST_AT`] an edit takes the turret (`RemoveWeapons`: the cannon
/// and its S1 go, the machine gun keeps S2 — the arena's numbers are stable)
/// and red's lock. The think that takes the lock again sees ONE weapon row,
/// the machine gun at index 0, shown armed in S2 — and cites `Fire {
/// machine_gun, Some(2) }`, the view's number, never `i + 1` (1); the shot
/// lands at its landing `by: "mind"`, slot 2, and no think is ever told
/// `unwarranted`.
#[test]
fn m1b3_t1_cites_the_slot_its_view_shows_after_its_s1_gun_is_lost() {
    use tacsim::arena::resolution::Consequence;
    let lose_the_turret = |arena: &mut Arena| {
        assert_eq!(
            (
                arena.conditions.slot_of(RED, "cannon"),
                arena.conditions.slot_of(RED, "machine_gun")
            ),
            (Some(1), Some(2)),
            "the precondition: both guns armed by {TURRET_LOST_AT}"
        );
        let now = arena.now_ms;
        let mut journal = Vec::new();
        arena.apply_consequence(
            RED,
            &Consequence::RemoveWeapons {
                section: SmolStr::new("turret"),
            },
            now,
            None,
            &mut journal,
        );
        assert_eq!(
            (
                arena.conditions.slot_of(RED, "cannon"),
                arena.conditions.slot_of(RED, "machine_gun")
            ),
            (None, Some(2)),
            "S1 freed with its gun, S2 kept"
        );
        if let Some(state) = arena.unit_mut(RED) {
            state.lock = None;
        }
    };
    let run = at_three_cadences(
        &two_gun_field,
        true,
        15_000,
        &[],
        &[(TURRET_LOST_AT, &lose_the_turret)],
        &mut |_| {},
    );
    assert_twinned(&run);
    let substep = two_gun_field().substep_len_ms();
    // The think that takes the lock again, and what it cites.
    let Some((_, think, view, result)) = run.thoughts.iter().find(|(_, t_ms, _, result)| {
        *t_ms >= TURRET_LOST_AT
            && result
                .intents
                .iter()
                .any(|kind| matches!(kind, IntentKind::Lock { .. }))
    }) else {
        panic!("red never took its lock again after {TURRET_LOST_AT}");
    };
    let rows: Vec<(&str, Option<u8>)> = view
        .weapons
        .iter()
        .map(|row| (row.id.as_str(), row.armed.as_ref().map(|armed| armed.slot)))
        .collect();
    assert_eq!(
        rows,
        vec![("machine_gun", Some(2))],
        "the view at {think}: the one gun, shown in the arena's S2"
    );
    let cited: Vec<(&str, Option<u8>)> = result
        .intents
        .iter()
        .filter_map(|kind| match kind {
            IntentKind::Fire { weapon, slot } => Some((weapon.as_str(), *slot)),
            _ => None,
        })
        .collect();
    assert_eq!(
        cited,
        vec![("machine_gun", Some(2))],
        "the view's S2, where i + 1 is 1, at {think}: {:?}",
        result.intents
    );
    let shots = run.shots();
    assert!(
        shots.contains(&(
            think + substep,
            "machine_gun".to_owned(),
            u64::from(BLUE.0),
            Some("mind".to_owned()),
            Some(2)
        )),
        "the citation fired by the mind at its landing, S2: {shots:?}"
    );
    assert!(
        !run.told(RED).contains(&Refusal::Unwarranted),
        "no citation was unwarranted: {:?}",
        run.told(RED)
    );
}

/// **An order's hint re-keys only to a CLASSIFIED row, at three cadences**
/// (17b cut M1b.3; the lead's ruling on M1a's residual (2), D-UM-17's rule
/// applied to the hint; 16b §11's mandatory cross-tempo property for
/// `Arena::rekey_order`'s change). A scripted mind (red, saying nothing) holds
/// blue as an UNKNOWN row of its own, which its own sensor tells apart about
/// `classify_after_ms` (3 s) into the run; the seated red pilot's tank holds
/// blue classified. A pilot's `ENGAGE` — the hint the pilot's row, `issuer`
/// the pilot's tank — lands at 0 and again, FRESH, at 10,000. At 5,000,
/// 1,000 and 500 ms — one event list, one hash sequence, one sequence of
/// thinks — the objective the mind holds at every 5,000 ms boundary is one
/// sequence:
///
/// * at 0, `Engage { target: None }` — the row unknown;
/// * at 5,000 and 10,000, still `None` though the row is classified by then
///   (asserted) — **`order_hint_late`, declared**: `rekey_order` runs once,
///   at the order's landing;
/// * at 15,000, the fresh order's `Engage { target: Some(row) }`, the mind's
///   own row.
#[test]
fn m1b3_an_order_hint_re_keys_only_to_a_classified_row_at_three_cadences() {
    use tacsim::deliberation::order::{CharacterRef, Constraints, Objective, Order};
    let make = || {
        let mut arena = seated_arena(
            vec![
                minded(unit(
                    RED,
                    Team::Red,
                    facing(5, 8, 90_000),
                    vec![cannon()],
                    Some(sight(0)),
                )),
                unit(
                    RED2,
                    Team::Red,
                    facing(5, 10, 90_000),
                    vec![cannon()],
                    Some(sight(0)),
                ),
                unit(BLUE, Team::Blue, facing(9, 8, 270_000), Vec::new(), None),
            ],
            &[RED2, BLUE],
        );
        for id in [RED, RED2] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        hold_row(&mut arena, RED, BLUE, Classification::Unknown);
        hold_row(
            &mut arena,
            RED2,
            BLUE,
            Classification::Classified { team: Team::Blue },
        );
        arena.mind_runner = Arc::new(Scripted::default());
        arena
    };
    let engage = || {
        IntentKind::Order(Box::new(Order {
            squad: RED_ONE,
            objective: Objective::Engage {
                target_hint: Some(contact_of(RED2, BLUE)),
            },
            target: None,
            horizon_ms: 60_000,
            constraints: Constraints::default(),
            ttl_ms: 60_000,
            issued_ms: 0,
            issuer: Some(RED2),
            by: CharacterRef::Character { id: ObjId(9_001) },
            spot: None,
        }))
    };
    let beats = [(0, engage()), (10_000, engage())];
    let mut held: Vec<(i64, Option<MindObjective>, bool)> = Vec::new();
    let _ = at_three_cadences(&make, true, 15_000, &beats, &[], &mut |arena| {
        if arena.now_ms.rem_euclid(5_000) == 0 {
            let told = arena
                .contacts
                .get(RED, contact_of(RED, BLUE))
                .is_some_and(|row| matches!(row.class, Classification::Classified { .. }));
            held.push((
                arena.now_ms,
                arena
                    .minds
                    .get(&RED)
                    .and_then(|state| state.order.as_ref())
                    .map(|order| order.objective),
                told,
            ));
        }
    });
    let own = Some(contact_of(RED, BLUE));
    let want = vec![
        (0, Some(MindObjective::Engage { target: None }), false),
        (5_000, Some(MindObjective::Engage { target: None }), true),
        (10_000, Some(MindObjective::Engage { target: None }), true),
        (15_000, Some(MindObjective::Engage { target: own }), true),
    ];
    assert_eq!(
        held.len(),
        3 * want.len(),
        "every cadence stops on each 5,000 ms: {held:?}"
    );
    for (cadence, seen) in held.chunks(want.len()).enumerate() {
        assert_eq!(seen, want.as_slice(), "the held objective, run {cadence}");
    }
}

// ---------------------------------------------------------------------------
// 17b cut M1b.4: a piloted arming, said
// ---------------------------------------------------------------------------

/// Blue's seat: `arena_of` seats a pilot per unit in the order given, so the
/// duel's second unit is `#1001`.
const BLUE_PILOT: ObjId = ObjId(1_001);

/// The two slot notices one pilot was sent, in order, each with the arena
/// millisecond its frame was sent at.
fn slot_told(run: &Run, pilot: ObjId) -> Vec<(i64, FrameNotice)> {
    run.told
        .iter()
        .filter(|(_, to, notice)| {
            *to == pilot
                && matches!(
                    notice,
                    FrameNotice::SlotFired { .. } | FrameNotice::SlotHeld { .. }
                )
        })
        .map(|(t_ms, _, notice)| (*t_ms, notice.clone()))
        .collect()
}

/// **The mandatory cross-tempo proof of a piloted arming** (17b cut M1b.4;
/// 16b §11; doc 17 §5, D-UM-14): the duel with blue facing red (its front)
/// and the odds at 70 per cent, both powered at 0; at 5,000 red's pilot types
/// `LOCK` and arms the cannon `>=60% side/rear`; at 10,000 a typed `FIRE` on
/// the front; at 20,000 a re-arm with new terms (`>=60% side`); at 25,000 blue
/// turns north, showing red its left flank. Every beat sits on every drive's
/// lattice.
///
/// * Through the first two cycles nothing fires by the slot, and exactly ONE
///   `slot_held` (`BA shows its front`) reaches red's frames inside ten
///   seconds — at 5,000, the arming's own evaluation — and a second only at
///   or after 15,000, though red's passes and the cannon's `WeaponReady` (at
///   14,000, after the typed shot) evaluate it every second in between;
/// * the typed `FIRE` on the front fires at its pulse's end, ungated, with no
///   `by`;
/// * the re-arm says its first held reason AT ONCE, at 20,000, inside the old
///   terms' ten seconds (the re-arm dropped the slot's `held_said` row);
/// * after the turn the slot fires first at one of red's OWN pass
///   milliseconds — the pass that refreshed the plot — and then on the
///   cannon's `WeaponReady`, each shot `by: cond` `slot: 1` and announced by
///   ONE `slot_fired` to red's frames (the lock by its designator, the chance
///   and the side), never to blue's: blue's frames carry no slot notice at all;
/// * one event list, one hash sequence (`held_said` is folded: when the
///   sentence is said is in the hash) and one sequence of slot notices at
///   5,000 / 1,000 / 500 ms and on the task's wakes.
#[test]
fn m1b4_a_piloted_arming_fires_only_on_flanks_at_three_cadences() {
    let flank_rear = aspect_mask(&[Aspect::Left, Aspect::Right, Aspect::Rear]);
    let flank = aspect_mask(&[Aspect::Left, Aspect::Right]);
    let contact = contact_of(RED, BLUE);
    let make = || {
        let mut arena = duel(270_000);
        arena.resolution.p_min = 700;
        arena.resolution.p_max = 700;
        arena
    };
    let script = || {
        vec![
            beat(
                0,
                vec![
                    intent(RED, IntentKind::Startup, 1),
                    intent(BLUE, IntentKind::Startup, 2),
                ],
            ),
            beat(
                5_000,
                vec![
                    intent(RED, IntentKind::Lock { contact }, 3),
                    intent(RED, arm(cond("cannon", 600, flank_rear)), 4),
                ],
            ),
            beat(
                10_000,
                vec![intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    5,
                )],
            ),
            beat(
                20_000,
                vec![intent(RED, arm(cond("cannon", 600, flank)), 6)],
            ),
            beat(
                25_000,
                vec![intent(BLUE, IntentKind::Heading { mdeg: 0 }, 7)],
            ),
        ]
    };
    let until = 40_000;
    let run = at_every_drive(make, until, script);

    // The slot's sentences are one sequence at every drive.
    let sequence = |run: &Run| -> Vec<FrameNotice> {
        slot_told(run, RED_PILOT)
            .into_iter()
            .map(|(_, notice)| notice)
            .collect()
    };
    let mut fine: Option<Run> = None;
    for drive in DRIVES {
        let mut arena = make();
        let other = replay(&mut arena, drive, until, &script());
        assert_eq!(sequence(&other), sequence(&run), "{drive:?}");
        assert!(
            slot_told(&other, BLUE_PILOT).is_empty(),
            "{drive:?}: the target was told of a slot"
        );
        if matches!(drive, Drive::Every(500)) {
            fine = Some(other);
        }
    }
    let fine = fine.expect("the 500 ms drive ran");

    // The typed FIRE on the front: fired at its pulse's end, ungated.
    let fired = run.fired();
    let typed: Vec<&(i64, serde_json::Value)> = fired
        .iter()
        .filter(|(_, line)| line.get("by").is_none())
        .collect();
    assert_eq!(typed.len(), 1, "{fired:?}");
    assert_eq!(typed[0].0, 10_000);
    assert!(typed[0].1.get("slot").is_none());

    // The slot's shots: none before the turn, the first at red's own pass,
    // then on the cannon's cycle.
    let shots = cond_shots(&run);
    let Some(&(first, _, _)) = shots.first() else {
        panic!("the slot never fired: {:?}", run.counters);
    };
    assert!(first > 25_000, "the slot fired before the turn: {shots:?}");
    assert!(
        is_pass_of(RED, first),
        "the first slot shot at {first} is not one of red's passes"
    );
    let cycle = i64::from(cannon().cycle_ms);
    for (n, (t_ms, slot, target)) in shots.iter().enumerate() {
        assert_eq!(*slot, 1);
        assert_eq!(*target, u64::from(BLUE.0));
        assert_eq!(
            *t_ms,
            first + cycle * i64::try_from(n).unwrap_or(0),
            "on the WeaponReady millisecond"
        );
    }
    assert!(shots.len() >= 2, "{shots:?}");

    // What red was told, and when (the 500 ms drive's frames).
    let told = slot_told(&fine, RED_PILOT);
    let held: Vec<(i64, &FrameNotice)> = told
        .iter()
        .filter(|(_, notice)| matches!(notice, FrameNotice::SlotHeld { .. }))
        .map(|(t_ms, notice)| (*t_ms, notice))
        .collect();
    assert_eq!(held.len(), 3, "{told:?}");
    for (_, notice) in &held {
        let FrameNotice::SlotHeld { slot, weapon, why } = notice else {
            unreachable!("filtered to held");
        };
        assert_eq!(*slot, 1);
        assert_eq!(weapon.as_str(), "cannon");
        assert!(
            matches!(why, HeldWhy::Aspect { contact: named, aspect: Aspect::Front } if named.id == contact),
            "{why:?}"
        );
    }
    assert_eq!(held[0].0, 5_000, "the arming's own evaluation said it");
    assert!(
        (15_000..=16_000).contains(&held[1].0),
        "the second held sentence came at {}, not ten seconds on",
        held[1].0
    );
    assert_eq!(
        held[2].0, 20_000,
        "the re-arm said its first reason at once"
    );
    let fired_told: Vec<&FrameNotice> = told
        .iter()
        .map(|(_, notice)| notice)
        .filter(|notice| matches!(notice, FrameNotice::SlotFired { .. }))
        .collect();
    assert_eq!(fired_told.len(), shots.len(), "one sentence a slot shot");
    for notice in fired_told {
        let FrameNotice::SlotFired {
            slot,
            weapon,
            contact: named,
            p_permille,
            aspect,
        } = notice
        else {
            unreachable!("filtered to fired");
        };
        assert_eq!(*slot, 1);
        assert_eq!(weapon.as_str(), "cannon");
        assert_eq!(named.map(|named| named.id), Some(contact));
        assert_eq!(*p_permille, 700);
        assert!(matches!(aspect, Aspect::Left | Aspect::Right), "{aspect:?}");
    }
    // Nothing the pilot typed was refused.
    assert!(
        fine.notices
            .iter()
            .all(|notice| !matches!(notice, FrameNotice::Refused { .. })),
        "{:?}",
        fine.notices
    );
}

// ---------------------------------------------------------------------------
// 17b cut M2.2: the lead's rulings (a) and (c), landed at three cadences
// ---------------------------------------------------------------------------

/// **A hull one round kills** (17b cut M2.2): [`tactical_unit`]'s one hull
/// section with no armour left and one point inside, so the first round that
/// lands destroys the section.
fn fragile_unit(id: ObjId) -> TacticalUnit {
    let spec = UnitSpec {
        definition: dref("unit", "tacsim.unit.line_tank"),
        sections: vec![SectionSpec {
            id: SectionId::new("hull").expect("a canonical section"),
            condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
            facets: vec![FacetSpec {
                id: FacetId::new("front").expect("a canonical facet"),
                cells: vec![CellSpec {
                    id: mush_tactics::CellId::new("armor").expect("a cell"),
                    min: 0,
                    max: 10,
                    init: 0,
                }],
            }],
            slots: Vec::new(),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: 10,
                init: 1,
            }],
        }],
        unit_cells: Vec::new(),
        components: Vec::new(),
        capabilities: Vec::new(),
        mobility: None,
        rules: rules(),
    };
    TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
}

/// **The kill duel** (17b cut M2.2, ruling (a)): red with a machine gun (a
/// one-second cycle, so its `WeaponReady` comes round while the dead hull's
/// track still stands) at (5, 8); blue at (9, 8) one round from a wreck — no
/// armour, one point inside, its hull `destroy_unit`; the odds pinned at
/// certainty. A second blue hull stands at (14, 1), out of red's five-cell
/// sight, so blue's side still stands after the kill and the match runs on:
/// with blue alone the kill ends a skirmish inside its substep (doc 16 §6.4's
/// elimination) and no later `WeaponReady` or pass is ever evaluated.
fn kill_duel() -> Arena {
    let lethal = UnitProfile {
        sections: vec![SectionProfile {
            consequences: vec![tacsim::arena::resolution::Consequence::DestroyUnit],
            ..profile().sections.remove(0)
        }],
        ..profile()
    };
    let mut arena = arena_of(vec![
        unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![machine_gun()],
            Some(sight(0)),
        ),
        CustodyUnit {
            profile: lethal,
            unit: fragile_unit(BLUE),
            ..unit(
                BLUE,
                Team::Blue,
                facing(9, 8, 0),
                Vec::new(),
                Some(sight(0)),
            )
        }
        .sealed(),
        unit(BLUE2, Team::Blue, facing(14, 1, 0), Vec::new(), None),
    ]);
    arena.resolution.p_min = 1_000;
    arena.resolution.p_max = 1_000;
    arena
}

/// **A `Lock`-target slot idles on a lock that reads dead** (17b cut M2.2, the
/// lead's ruling (a): one read-dead rule for the three targets; doc 17 §5,
/// D-UM-17; 16b §11's mandatory cross-tempo property): on [`kill_duel`] red's
/// pilot locks blue and arms the machine gun on its LOCK at 5,000 (a
/// `Contact` slot is unlawful at the arena in 17b cut M1b —
/// `condition_is_lawful` — so its arm of the rule is the in-source warrant
/// test's). The arming's own evaluation fires, and the round destroys blue —
/// the kill's lines (`fired` `by: cond` `slot: 1`, then `unit_destroyed` by
/// red) — and the shooter's hold of the wreck reads dead (the kill it
/// watched). The match runs on (blue's second hull stands),
/// and every later `WeaponReady` and pass is `Idle` until the track expires:
/// no second `fired` line, the rounds conserved (one fired, the rest in the
/// gun), no `slot_held` notice — and the warrant itself, asked at the gun's
/// next ready with the wreck's track still held, answers `Idle`. Under 17b
/// cut M1b.1's rule (read-dead for `BestHostile` alone) the same tape shows
/// the slot firing at the wreck on the gun's cycle — `fired` at 6,000 and
/// 7,000, the track still held — so with the rule reverted this test fails on
/// its tape, not only on the warrant. One event list and one hash sequence at
/// 5,000, 1,000 and 500 ms and on the task's wakes.
#[test]
fn m2_a_lock_target_slot_idles_on_a_lock_that_reads_dead_at_three_cadences() {
    let contact = contact_of(RED, BLUE);
    let slot = cond("machine_gun", 100, ASPECTS_ANY);
    let script = || {
        vec![
            beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
            beat(
                5_000,
                vec![
                    intent(RED, IntentKind::Lock { contact }, 2),
                    intent(RED, arm(slot.clone()), 3),
                ],
            ),
        ]
    };
    let until = 15_000;
    let run = at_every_drive(kill_duel, until, script);
    // The kill's lines: the arming's shot, then blue destroyed by red.
    let fired = run.fired();
    assert_eq!(fired.len(), 1, "one shot and no more: {fired:?}");
    assert_eq!(cond_shots(&run), vec![(5_000, 1, u64::from(BLUE.0))]);
    let destroyed: Vec<serde_json::Value> = run
        .events
        .iter()
        .filter_map(|(_, json, _)| serde_json::from_str::<serde_json::Value>(json).ok())
        .filter(|line| {
            line.get("kind").and_then(serde_json::Value::as_str) == Some("unit_destroyed")
        })
        .collect();
    assert_eq!(destroyed.len(), 1, "{destroyed:?}");
    assert_eq!(destroyed[0]["unit"], u64::from(BLUE.0));
    assert_eq!(run.counters.cond_fires, 1);
    // No slot was ever held for a reason: an idle slot says nothing.
    assert!(
        slot_told(&run, RED_PILOT)
            .iter()
            .all(|(_, notice)| !matches!(notice, FrameNotice::SlotHeld { .. })),
        "{:?}",
        slot_told(&run, RED_PILOT)
    );
    // The match ran to the end of the script, the rounds conserved, and
    // the slot stood armed throughout.
    let mut arena = kill_duel();
    let _ = replay(&mut arena, Drive::Every(500), until, &script());
    assert_eq!(arena.now_ms, until, "the match outlives the kill");
    let left = u64::from(
        arena
            .unit(RED)
            .map_or(0, |state| state.weapon_ammo("machine_gun")),
    );
    assert_eq!(left + 1, u64::from(machine_gun().count_per_load));
    assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), Some(1));
    assert!(
        arena.unit(BLUE).is_some_and(|state| state.limits.wreck),
        "blue is a wreck"
    );
    // At the kill's next `WeaponReady` the hold still stood and read dead:
    // replayed to 6,000 the warrant answers `Idle` on the held, dead lock.
    let mut early = kill_duel();
    let _ = replay(&mut early, Drive::Every(500), 6_000, &script());
    assert_eq!(early.now_ms, 6_000, "the match outlives the kill");
    let held = early
        .contacts
        .resolve(RED, contact)
        .expect("the shooter still holds the wreck at its gun's next ready");
    assert!(early.contact_reads_dead(held), "the kill it watched");
    assert_eq!(
        early.warrant(RED, &slot, 6_000),
        Err(tacsim::arena::conditions::Held::Idle),
        "a Lock-target slot idles on a lock that reads dead"
    );
}

/// The wide window of the range proof: 70 by 16 cells of 1,000 cm, the
/// entry zones the first two and last two columns.
fn strip_definition() -> ArenaDefinition {
    const WIDE: u16 = 70;
    ArenaDefinition {
        terrain: ArenaTerrain::new(
            WIDE,
            SIDE,
            CELL_CM,
            vec![ground(); usize::from(WIDE) * usize::from(SIDE)],
        )
        .expect("the strip"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: SIDE - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: WIDE - 2,
                y0: 0,
                x1: WIDE - 1,
                y1: SIDE - 1,
            },
        ],
        ..definition()
    }
}

/// **The range duel** (17b cut M2.2, ruling (c)): red with a cannon and a
/// seventy-cell sight at (2, 8) facing east, blue at (62, 8) — sixty cells,
/// 600 m, centre to centre — on [`strip_definition`]; the odds at
/// [`ODDS`].
fn range_duel() -> Arena {
    let units = vec![
        unit(
            RED,
            Team::Red,
            facing(2, 8, 90_000),
            vec![cannon()],
            Some(SensorProfile {
                max_range_cells: 70,
                ..sight(0)
            }),
        ),
        unit(BLUE, Team::Blue, facing(62, 8, 270_000), Vec::new(), None),
    ];
    let pilots = units
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: unit.id,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    let mut arena = Arena::new(
        CustodyOut {
            definition: strip_definition(),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the strip arena");
    arena.resolution.p_min = ODDS;
    arena.resolution.p_max = ODDS;
    arena
}

/// **[`replay`] at a fixed cadence with ONE hand on the field**: after the
/// pulse that first reaches `poke_ms` — an arena millisecond on every
/// cadence's lattice, where the state is one state at every cadence — `poke`
/// runs once. Answers the run.
fn poked_replay(
    arena: &mut Arena,
    cadence_ms: i64,
    until_ms: i64,
    script: &[Beat],
    poke_ms: i64,
    poke: &dyn Fn(&mut Arena),
) -> Run {
    let mut bytes: Vec<u8> = Vec::new();
    let mut notices: Vec<FrameNotice> = Vec::new();
    let mut told: Vec<(i64, ObjId, FrameNotice)> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = MotorCounters::default();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut poked = false;
    let mut pulse = |arena: &mut Arena, wall: i64, beat: Option<&Beat>| {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let (controls, intents) = beat.map_or((&[][..], &[][..]), |beat| {
            (&beat.controls[..], &beat.intents[..])
        });
        let out = arena.step(wall, controls, intents);
        absorb(&out, &mut bytes, &mut notices);
        told_of(&out, arena.now_ms, &mut told);
        counters.absorb(&out.motor);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        out.ended.is_some()
    };
    if script.first().is_none_or(|beat| beat.t_ms > 0) {
        let _ = pulse(arena, wall, None);
    }
    loop {
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            next += 1;
            let _ = pulse(arena, wall, Some(beat));
        }
        if !poked && arena.now_ms >= poke_ms {
            assert_eq!(arena.now_ms, poke_ms, "the poke is on the lattice");
            poke(arena);
            poked = true;
        }
        if arena.now_ms >= until_ms {
            break;
        }
        wall += cadence_ms;
        if pulse(arena, wall, None) {
            break;
        }
    }
    let events = lines(arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    Run {
        events,
        hashes,
        counters,
        notices,
        told,
        pulses,
    }
}

/// **A typed range term is strict at the ROUNDED range** (17b cut M2.2, the
/// lead's ruling (c); doc 17 §5; 16b §11's mandatory cross-tempo property):
/// on [`range_duel`] red's pilot locks blue and arms the cannon on the rule
/// `odds >= 10 AND range < 600` as the cockpit compiles it —
/// [`cockpit::parse_arming`] over red's own first frame and cannon row, so the
/// arena runs the compile and not a hand-written bound: `max_range_m 599`
/// (the reply's figures are pinned by `tests/cockpit.rs`'
/// `m2_a_range_term_is_strict_and_the_reply_reads_the_typed_figure`) — at
/// 5,000, with blue's plot at exactly 600 m, whose rounded range is
/// 600: the slot HOLDS, and says `HeldWhy::Range { range_m: 600, max_m: 599 }`
/// (17b cut M1b.4 stored 600 and fired: a printed 600 m was inside `range <
/// 600`). At 10,000 blue stands five cells nearer — 550 m, the field's one
/// hand, after the pulse that reaches 10,000 at every cadence — and the slot
/// fires at red's first pass after it, whose plot reads 550, `by: cond`
/// `slot: 1`, and on the cannon's cycle after. One event list and one hash
/// sequence at 5,000, 1,000 and 500 ms.
#[test]
fn m2_a_typed_range_term_is_strict_at_the_rounded_range_at_three_cadences() {
    let contact = contact_of(RED, BLUE);
    // The rule as the pilot types it, compiled by the cockpit off red's own
    // frame: the strict bound is the compile's, never this test's.
    let under_600 = {
        let mut arena = range_duel();
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let out = arena.step(0, &[], &[]);
        let frame = out
            .frames
            .iter()
            .find(|(pilot, _)| *pilot == RED_PILOT)
            .map(|(_, frame)| frame.clone())
            .expect("red's pilot has a frame");
        let row = cockpit::frame_weapons(&frame)
            .into_iter()
            .find(|row| row.id == "cannon")
            .expect("red's cannon row");
        let (compiled, _) = cockpit::parse_arming(&row, "odds >= 10 AND range < 600", &frame)
            .expect("the rule compiles");
        compiled
    };
    assert_eq!(
        under_600,
        FireCondition {
            max_range_m: 599,
            ..cond("cannon", 100, ASPECTS_ANY)
        },
        "`range < 600` compiles strict"
    );
    let script = || {
        vec![
            beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
            beat(
                5_000,
                vec![
                    intent(RED, IntentKind::Lock { contact }, 2),
                    intent(RED, arm(under_600.clone()), 3),
                ],
            ),
        ]
    };
    let nearer = |arena: &mut Arena| {
        if let Some(state) = arena.unit_mut(BLUE) {
            state.pose = facing(57, 8, 270_000);
        }
        let units = arena.units.clone();
        let terrain = arena.definition.terrain.clone();
        arena.index.rebuild(&terrain, &units);
    };
    let until = 20_000;
    let runs: Vec<(i64, Run)> = [5_000, 1_000, 500]
        .into_iter()
        .map(|cadence| {
            let mut arena = range_duel();
            let run = poked_replay(&mut arena, cadence, until, &script(), 10_000, &nearer);
            (cadence, run)
        })
        .collect();
    let (_, first) = runs.first().expect("three runs");
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(run.events, first.events, "the event list at {cadence} ms");
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence at {cadence} ms"
        );
        assert_ne!(run.pulses, first.pulses);
    }
    // Held at the rounded 600, said once, naming the stored 599.
    let told = slot_told(first, RED_PILOT);
    let held: Vec<&(i64, FrameNotice)> = told
        .iter()
        .filter(|(_, notice)| matches!(notice, FrameNotice::SlotHeld { .. }))
        .collect();
    let Some((held_at, FrameNotice::SlotHeld { slot, why, .. })) = held.first().copied() else {
        panic!("the slot never held: {told:?}");
    };
    assert_eq!(*held_at, 5_000, "the arming's own evaluation");
    assert_eq!(*slot, 1);
    assert!(
        matches!(
            why,
            HeldWhy::Range { contact: named, range_m: 600, min_m: 0, max_m: 599 } if named.id == contact
        ),
        "{why:?}"
    );
    // Nothing fired before the hand; the first shot at red's first pass after
    // it, then on the cannon's cycle.
    let shots = cond_shots(first);
    let Some(&(shot, _, _)) = shots.first() else {
        panic!("the slot never fired at 550 m: {:?}", first.counters);
    };
    assert!(
        shot > 10_000 && shot <= 10_000 + PASS_MS,
        "the first shot at {shot}, not at red's first pass after the hand"
    );
    assert!(is_pass_of(RED, shot), "{shot} is one of red's passes");
    for (n, (t_ms, slot, target)) in shots.iter().enumerate() {
        assert_eq!((*slot, *target), (1, u64::from(BLUE.0)));
        assert_eq!(
            *t_ms,
            shot + i64::from(cannon().cycle_ms) * i64::try_from(n).unwrap_or(0)
        );
    }
    // The rounded range it fired at reads 550, at a fresh plot's millisecond
    // (the plot's own: older, the lock's step widens): a bound at 550 takes
    // it, one at 549 holds it as 550.
    let mut arena = range_duel();
    let _ = poked_replay(&mut arena, 500, until, &script(), 10_000, &nearer);
    let seen_ms = arena
        .contacts
        .resolve(RED, contact)
        .map(|held| held.last_seen_ms)
        .expect("red still holds blue");
    let held = arena.warrant(
        RED,
        &FireCondition {
            max_range_m: 549,
            ..under_600.clone()
        },
        seen_ms,
    );
    assert!(
        matches!(
            &held,
            Err(tacsim::arena::conditions::Held::Why(HeldWhy::Range {
                contact: named,
                range_m: 550,
                min_m: 0,
                max_m: 549,
            })) if named.id == contact
        ),
        "{held:?}"
    );
    assert!(
        arena
            .warrant(
                RED,
                &FireCondition {
                    max_range_m: 550,
                    ..under_600
                },
                seen_ms
            )
            .is_ok(),
        "550 is inside a 550 bound"
    );
}
