//! **16b cut C.2 acceptance** (doc 16 §3.3, §3.5, §3.6, §4; 16b §8 cut C).
//!
//! The five claims the cut makes, each as a test over the real composition:
//!
//! 1. every shipped definition loads, validates and binds, and a copy with a
//!    dangling reference or an unknown key refuses **naming the entry**;
//! 2. every definition is registered with an **exact** `DefinitionRef` — kind,
//!    id, version and payload digest — which is what a persisted unit points at;
//! 3. `from_pack` yields the 256 by 256 window, and its impassable cells are
//!    exactly the cells whose class the shipped `tracked` locomotion row prices
//!    impassable;
//! 4. on a fresh world with `[tacsim].enabled` the eight stock tanks exist after
//!    boot, and a second boot creates no ninth;
//! 5. the ten commodity families are instruments after boot, beside the credit.

use std::collections::{BTreeMap, HashMap};

use mush_core::{GOD, ModuleBootData, ModuleWrite, ObjId};
use mush_testkit::TestWorld;
use smol_str::SmolStr;
use tacsim::TacsimConfig;
use tacsim::definitions::{Catalog, DefinitionError, content};

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

/// `[tacsim].enabled` with no operator pack at all: the game runs on the content
/// compiled into it.
fn enabled() -> TacsimConfig {
    TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    }
}

/// The production roster, composed exactly as `mush_server::boot` composes it,
/// with the game's own boot packs on.
fn production_roster() -> Vec<Box<dyn mush_core::GameModule>> {
    roster(true)
}

/// The same roster without the character sheet.
///
/// Only the reboot test uses it, and only because `on_world_loaded` cannot be
/// driven twice in one process with the sheet composed: it registers the
/// `pilot.chargen` job payload handler, and `mush_jobs` refuses a second
/// registration of one handler. That is a property of rebooting a `TestWorld`
/// in place, not of this cut, and the sheet has nothing to do with the eight
/// tanks — so the reboot is asserted on the half of the roster that makes them.
fn reboot_roster() -> Vec<Box<dyn mush_core::GameModule>> {
    roster(false)
}

fn roster(sheet: bool) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = std::sync::Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let cfg = enabled();
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let mut mods = comp::server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        std::sync::Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    if sheet {
        mods.extend(comp::ruleset_modules(&onboarding, &game));
    }
    mods.push(comp::tacsim_module(cfg));
    // `[game].starter_area` off: the lot is not what this cut ships, and the
    // arena's ground is in a region of its own.
    mods.push(comp::content_module_with(false, true).expect("the content module composes"));
    mods
}

type Kv = BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_all(tw: &mut TestWorld) -> Kv {
    let mut kv: Kv = BTreeMap::new();
    for op in tw.take_module_ops() {
        let entry = kv.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                entry.retain(|(k, _)| *k != key);
                entry.push((key, value));
            }
            ModuleWrite::Del { key } => entry.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn reboot(tw: &mut TestWorld, kv: &Kv) {
    let data: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, entries)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: entries.clone(),
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(data);
}

/// Every object in the world whose name starts with `prefix`.
fn objects_named(tw: &TestWorld, prefix: &str) -> Vec<ObjId> {
    (0..64)
        .map(ObjId)
        .filter(|id| {
            tw.world()
                .get(*id)
                .is_some_and(|object| object.name.starts_with(prefix))
        })
        .collect()
}

/// One shipped pack's JSON with a substring replaced — how a "copy with a
/// dangling reference or an unknown key" is made without a second content tree.
fn tampered(json: &str, from: &str, to: &str) -> String {
    let edited = json.replace(from, to);
    assert_ne!(edited, json, "the tamper found nothing to replace");
    // Re-sign it, so the refusal under test is the *rule* and never the
    // signature it also lacks.
    let declared = mush_content::parse_pack(json)
        .expect("the shipped pack parses")
        .manifest()
        .digest()
        .to_hex();
    let unsigned = edited.replace(&declared, &"0".repeat(64));
    match mush_content::parse_pack(&unsigned) {
        Ok(parsed) => match parsed.computed_digest() {
            Ok(digest) => unsigned.replace(&"0".repeat(64), &digest.to_hex()),
            Err(_) => unsigned,
        },
        // An unknown key does not parse at the content layer either; hand the
        // bytes over and let `load_pack` produce the refusal.
        Err(_) => unsigned,
    }
}

// ---------------------------------------------------------------------------
// (1) Loading, validation and refusal
// ---------------------------------------------------------------------------

#[test]
fn every_shipped_definition_loads_validates_and_binds() {
    let catalog = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert_eq!(catalog.ammunition.len(), 2, "two ammunition families");
    assert_eq!(catalog.weapons.len(), 2, "the cannon and the coaxial");
    assert_eq!(catalog.sensors.len(), 1, "one visual sight");
    assert_eq!(catalog.resolutions.len(), 1, "one resolution table");
    assert_eq!(catalog.specs.len(), 1, "one unit schema");
    assert_eq!(catalog.arenas.len(), 1, "one arena");
    assert_eq!(catalog.units.len(), 1, "the tactics catalog carries it");

    // Doc 16 §3.3's table, as the shipped tank actually is.
    let spec = catalog
        .specs
        .values()
        .next()
        .expect("the line tank is shipped");
    let sections: Vec<&str> = spec.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        sections,
        ["hull", "turret", "left_track", "right_track", "engine"]
    );
    let components: Vec<&str> = spec.components.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        components,
        ["cannon", "machine_gun", "visual_sensor", "power"]
    );
    assert_eq!(
        spec.sensor_height_cm, 150,
        "doc 16 §3.2's default, authored"
    );

    // W14, asserted on the content and not only in the decoder.
    for weapon in catalog.weapons.values() {
        assert!(
            weapon.cycle_ms >= tacsim::definitions::weapon::CYCLE_FLOOR_MS,
            "{} cycles faster than the floor",
            weapon.label
        );
    }
    // Doc 16 §3.6's two hooks are present and null.
    for sensor in catalog.sensors.values() {
        assert!(sensor.emission.is_none() && sensor.jamming.is_none());
        assert_eq!(sensor.max_range_cells, 60, "16b §9's constant, as content");
        assert_eq!(sensor.classify_after_ms, 3_000);
        // **16b cut S.1**: the comms reach and the signature table, as authored.
        assert_eq!(sensor.share_range_cells, 30, "16b §9's comms reach");
        let rows: BTreeMap<&str, (u32, u32)> = sensor
            .signature
            .iter()
            .map(|row| {
                (
                    row.class.label(),
                    (row.moving_permille, row.stopped_permille),
                )
            })
            .collect();
        assert_eq!(
            rows.len(),
            sensor.signature.len(),
            "the signature table names a ground twice"
        );
        // Doc 16 §3.6 step 3's own sentence, as two rows: *a stopped tank in heavy
        // woods has a lower factor than one moving on a road*.
        assert_eq!(rows.get("road").copied(), Some((1_000, 1_000)));
        assert_eq!(rows.get("woods_heavy").copied(), Some((800, 500)));
        for (class, (moving, stopped)) in &rows {
            assert!(
                *moving <= 1_000 && *stopped <= 1_000,
                "{class} lengthens the sensor's range"
            );
            assert!(
                stopped <= moving,
                "{class} hides a tank better when it is moving"
            );
        }
        // Open water is deliberately absent, so it reads as a target that hides
        // nothing — the rule the table's own note states.
        assert!(!rows.contains_key("water"));
    }
}

/// **16b cut S.1**: the two new bounds on a sensor row, refused by name.
#[test]
fn a_sensor_row_refuses_an_impossible_signature_and_an_absurd_comms_reach() {
    let sensor = || tacsim::definitions::SensorDef {
        label: "test sight".to_owned(),
        max_range_cells: 60,
        max_attenuation: 40,
        classify_after_ms: 3_000,
        sees_wrecks: false,
        share_range_cells: 30,
        signature: Vec::new(),
        emission: None,
        jamming: None,
        note: String::new(),
    };
    let row = |class, moving, stopped| tacsim::definitions::sensor::SignatureRow {
        class,
        moving_permille: moving,
        stopped_permille: stopped,
    };
    sensor().check("ok").expect("the base row is valid");

    // A factor above 1,000 would *lengthen* the range, which is
    // `[tacsim.sensors].max_signature_factor`'s job and not content's.
    let wild = tacsim::definitions::SensorDef {
        signature: vec![row(mush_atlas::TerrainClass::Road, 1_001, 500)],
        ..sensor()
    };
    let refused = wild
        .check("wild")
        .expect_err("a factor past 1,000 is refused");
    assert!(
        refused.to_string().contains("road") && refused.to_string().contains("1001"),
        "{refused}"
    );

    // One ground is one row.
    let twice = tacsim::definitions::SensorDef {
        signature: vec![
            row(mush_atlas::TerrainClass::Road, 1_000, 900),
            row(mush_atlas::TerrainClass::Road, 800, 700),
        ],
        ..sensor()
    };
    let refused = twice
        .check("twice")
        .expect_err("a repeated ground is refused");
    assert!(refused.to_string().contains("twice"), "{refused}");

    // And a comms reach past the window itself is a share to nobody.
    let far = tacsim::definitions::SensorDef {
        share_range_cells: tacsim::definitions::sensor::MAX_SHARE_RANGE_CELLS + 1,
        ..sensor()
    };
    let refused = far.check("far").expect_err("an absurd reach is refused");
    assert!(
        refused.to_string().contains("share_range_cells"),
        "{refused}"
    );

    // Zero shares nothing, and that is legal: a unit with no datalink.
    tacsim::definitions::SensorDef {
        share_range_cells: 0,
        ..sensor()
    }
    .check("mute")
    .expect("a mute sensor is a valid sensor");
}

/// **16b cut Q.2**: the section ceiling is the RULE behind a gunner's reading,
/// and both of its refusals say so by name.
///
/// `crate::arena::condition::Condition` packs two bits a section into one `u32`,
/// so sixteen is what a contact can carry without an allocation per contact per
/// sensor pass; `Condition::of`'s `.take(…)` is a defensive clamp and **this** is
/// what keeps any unit an arena can hold away from it. A cut that re-widened
/// `MAX_SECTIONS` without widening the packing would silently truncate a hull on
/// `SCAN`, so the two constants are asserted equal here as well.
#[test]
fn a_unit_declares_between_one_and_sixteen_sections() {
    assert_eq!(
        tacsim::definitions::unit::MAX_SECTIONS,
        tacsim::arena::condition::MAX_SCANNED_SECTIONS,
        "the authoring bound and the packing bound are one number"
    );
    assert_eq!(tacsim::arena::condition::MAX_SCANNED_SECTIONS, 16);

    let pack = tacsim::pack::load_pack(content::LINE_TANK_JSON).expect("the shipped tank loads");
    let tank = pack
        .entries()
        .values()
        .find_map(|entry| match entry {
            tacsim::pack::PackEntry::Unit(spec) => Some(spec.clone()),
            _ => None,
        })
        .expect("the pack carries the unit");
    assert_eq!(tank.sections.len(), 5, "the stock tank's five");
    tank.check("stock").expect("the shipped spec is valid");

    // A unit with nothing to shoot at is not a unit.
    let empty = tacsim::definitions::UnitSpec {
        sections: Vec::new(),
        ..tank.clone()
    };
    let refused = empty.check("empty").expect_err("no sections is refused");
    assert!(
        refused
            .to_string()
            .contains("a unit declares 1 to 16 sections; 0 is not"),
        "{refused}"
    );

    // And one section past what a reading can carry.
    let wide = tacsim::definitions::UnitSpec {
        sections: tank
            .sections
            .iter()
            .cycle()
            .take(tacsim::definitions::unit::MAX_SECTIONS + 1)
            .cloned()
            .collect(),
        ..tank.clone()
    };
    let refused = wide
        .check("wide")
        .expect_err("seventeen sections is refused");
    assert!(
        refused
            .to_string()
            .contains("a unit has at most 16 sections; 17 is not"),
        "{refused}"
    );

    // Sixteen exactly is the last spec the rule admits, so the clamp and the rule
    // meet with nothing between them.
    let full = tacsim::definitions::UnitSpec {
        sections: tank
            .sections
            .iter()
            .cycle()
            .take(tacsim::definitions::unit::MAX_SECTIONS)
            .cloned()
            .collect(),
        ..tank
    };
    assert_eq!(full.sections.len(), 16);
    if let Err(refused) = full.check("full") {
        assert!(
            !refused.to_string().contains("sections;"),
            "sixteen sections tripped the ceiling: {refused}"
        );
    }
}

#[test]
fn the_six_content_kinds_are_the_six_the_pack_format_owns() {
    // A vocabulary claim, so it is pinned outside the file that declares it:
    // doc 16 §4 names six kinds, `PackEntry` decodes six, and `OWNED_KINDS` — the
    // list that decides whether an entry is this crate's or another module's —
    // carries every one of them. A seventh kind that reached only one of the
    // three would be a kind nothing applies or a kind nothing decodes.
    for kind in tacsim::definitions::CONTENT_KINDS {
        assert!(
            tacsim::pack::OWNED_KINDS.contains(&kind),
            "`{kind}` is a content kind the pack format does not own"
        );
    }
    let catalog = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    let mut labelled: Vec<&str> = Vec::new();
    for source in content::sources() {
        let pack = tacsim::pack::load_pack(&source.bytes).expect("a shipped pack verifies");
        for entry in pack.entries().values() {
            if !labelled.contains(&entry.label()) {
                labelled.push(entry.label());
            }
        }
    }
    labelled.sort_unstable();
    let mut expected: Vec<&str> = tacsim::definitions::CONTENT_KINDS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        labelled, expected,
        "the shipped content exercises every content kind and no other"
    );
    assert!(!catalog.descriptors.is_empty());
}

#[test]
fn the_two_tracked_rules_readings_agree() {
    // `tracked_rules(&UnitSpec)` reads the authored five; the twin in `match_`
    // reads them back out of the instantiated unit's opaque `ModeRules`. They
    // must be the same value, or the arena a spec describes and the arena a unit
    // drives in are two arenas.
    let catalog = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    let (id, spec) = catalog
        .specs
        .iter()
        .next()
        .expect("the line tank is shipped");
    let binding = catalog.bindings.get(id).expect("it bound");
    let unit = tacsim::definitions::to_tactical_unit(spec, binding, ObjId(7))
        .expect("the spec instantiates");
    assert_eq!(
        tacsim::definitions::tracked_rules(spec),
        tacsim::match_::tracked_rules(&unit)
    );
    // And they are doc 16 §3.3's numbers, which are also `match_`'s documented
    // fallback for a unit whose rules blob says nothing: a unit with no blob at
    // all must drive like the shipped tank, or the fallback is a second tank.
    let rules = tacsim::definitions::tracked_rules(spec);
    assert_eq!(rules.max_speed_cms, 1_500);
    assert_eq!(rules.accel_cms2, 100);
    assert_eq!(rules.brake_cms2, 200);
    assert_eq!(rules.turn_rate_mdeg_s, 30_000);
    assert_eq!(rules.reverse_factor_pct, 50);
    assert_eq!(
        rules,
        tacsim::match_::tracked_rules(
            &mush_tactics::TacticalUnit::from_spec(
                &mush_tactics::UnitSpecBody {
                    mobility: None,
                    ..binding.spec.body()
                }
                .with_definition(binding.spec.definition.clone()),
                ObjId(8),
            )
            .expect("a unit with no mobility profile still instantiates")
        ),
        "the documented fallback is the shipped tank"
    );
}

#[test]
fn a_dangling_reference_refuses_naming_the_entry() {
    for (name, json, from, to, wanted) in [
        (
            "a weapon's ammunition",
            content::CANNON_105_JSON,
            "\"ammunition\": \"tacsim.ammunition.he_105\"",
            "\"ammunition\": \"tacsim.ammunition.nothing\"",
            "tacsim.ammunition.nothing",
        ),
        (
            "a unit's weapon",
            content::LINE_TANK_JSON,
            "\"weapon\": \"tacsim.weapon.mg_coax\"",
            "\"weapon\": \"tacsim.weapon.nothing\"",
            "tacsim.weapon.nothing",
        ),
        (
            "a unit's sensor",
            content::LINE_TANK_JSON,
            "\"sensor\": \"tacsim.sensor.visual_mk1\"",
            "\"sensor\": \"tacsim.sensor.nothing\"",
            "tacsim.sensor.nothing",
        ),
        (
            "an arena's unit",
            content::PROVING_GROUND_JSON,
            "\"unit\": \"tacsim.unit.line_tank\"",
            "\"unit\": \"tacsim.unit.nothing\"",
            "tacsim.unit.nothing",
        ),
    ] {
        let edited = tampered(json, from, to);
        let pack = tacsim::pack::load_pack(&edited)
            .unwrap_or_else(|error| panic!("{name}: the tampered pack still loads: {error}"));
        let operator = [tacsim::LoadedPack {
            source: format!("<{name}>"),
            pack,
        }];
        // Built beside the shipped content, so the *only* thing missing is the
        // reference the tamper broke.
        let error = Catalog::build(
            &operator
                .iter()
                .cloned()
                .chain(shipped_except(wanted_pack(name)))
                .collect::<Vec<_>>(),
            tacsim::NS,
        )
        .expect_err("a dangling reference refuses");
        let message = error.to_string();
        assert!(
            matches!(error, DefinitionError::Reference { .. }),
            "{name}: {message}"
        );
        assert!(message.contains(wanted), "{name}: {message}");
    }
}

/// Which shipped pack a tampered copy replaces, so the two do not collide on
/// `(id, version)`.
fn wanted_pack(name: &str) -> &'static str {
    match name {
        "a weapon's ammunition" => "tacsim.pack.cannon_105",
        "an arena's unit" => "tacsim.pack.proving_ground",
        _ => "tacsim.pack.line_tank",
    }
}

/// Every shipped pack but one, loaded.
fn shipped_except(skip: &str) -> Vec<tacsim::LoadedPack> {
    content::sources()
        .into_iter()
        .filter_map(|source| {
            let pack = tacsim::pack::load_pack(&source.bytes).ok()?;
            (pack.manifest().id().as_str() != skip).then_some(tacsim::LoadedPack {
                source: source.name,
                pack,
            })
        })
        .collect()
}

#[test]
fn an_unknown_key_refuses_naming_the_entry() {
    let edited = tampered(
        content::LINE_TANK_JSON,
        "\"sensor_height_cm\": 150",
        "\"sensor_height_cm\": 150,\n        \"turret_traverse\": 12",
    );
    let error = tacsim::pack::load_pack(&edited).expect_err("an unknown key refuses");
    let message = error.to_string();
    assert!(
        message.contains("tacsim.unit.line_tank") && message.contains("turret_traverse"),
        "{message}"
    );
}

#[test]
fn a_value_past_a_bound_refuses_naming_the_field() {
    // W14's floor, on the content rather than in the decoder's own unit test.
    let edited = tampered(
        content::MG_COAX_JSON,
        "\"cycle_ms\": 1000",
        "\"cycle_ms\": 999",
    );
    let error = tacsim::pack::load_pack(&edited).expect_err("a sub-floor cycle refuses");
    let message = error.to_string();
    assert!(
        message.contains("cycle_ms") && message.contains("coaxial machine gun"),
        "{message}"
    );
}

#[test]
fn a_filled_ecm_hook_refuses() {
    let edited = tampered(
        content::VISUAL_MK1_JSON,
        "\"emission\": null",
        "\"emission\": { \"band\": \"radar\" }",
    );
    let error = tacsim::pack::load_pack(&edited).expect_err("a filled hook refuses");
    assert!(error.to_string().contains("emission"), "{error}");
}

#[test]
fn an_arena_in_a_rules_pack_and_a_unit_in_a_seed_pack_both_refuse() {
    // Doc 16 §4's axis: a rules definition is re-pinned and diffed; an `arena`
    // instantiates, so re-running it would duplicate its eight tanks.
    let arena_in_rules = tampered(
        content::PROVING_GROUND_JSON,
        "\"kind\": \"campaign_seed\"",
        "\"kind\": \"rules_definition\"",
    );
    let error = tacsim::pack::load_pack(&arena_in_rules).expect_err("an arena is not a rule");
    assert!(error.to_string().contains("arena"), "{error}");

    let unit_in_seed = tampered(
        content::LINE_TANK_JSON,
        "\"kind\": \"rules_definition\"",
        "\"kind\": \"campaign_seed\"",
    );
    let error = tacsim::pack::load_pack(&unit_in_seed).expect_err("a unit is not a seed");
    assert!(error.to_string().contains("unit"), "{error}");
}

#[test]
fn two_packs_declaring_one_definition_id_refuse_naming_both() {
    let mut twin = shipped_except("");
    // The same file under a second pack id: one definition id, two declarers.
    let doubled = tampered(
        content::VISUAL_MK1_JSON,
        "\"id\": \"tacsim.pack.visual_mk1\"",
        "\"id\": \"tacsim.pack.visual_mk2\"",
    );
    twin.push(tacsim::LoadedPack {
        source: "<twin>".to_string(),
        pack: tacsim::pack::load_pack(&doubled).expect("the twin loads"),
    });
    let error = Catalog::build(&twin, tacsim::NS).expect_err("one id names one definition");
    let message = error.to_string();
    assert!(
        message.contains("tacsim.sensor.visual_mk1")
            && message.contains("tacsim.pack.visual_mk1@1")
            && message.contains("tacsim.pack.visual_mk2@1"),
        "{message}"
    );
}

// ---------------------------------------------------------------------------
// (2) Exact registration
// ---------------------------------------------------------------------------

#[test]
fn every_definition_is_registered_with_an_exact_reference() {
    let tw = TestWorld::with_modules(production_roster());
    let registry = tw
        .registry()
        .definitions()
        .expect("the definition registry is frozen after composition");
    let catalog = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    assert!(
        !catalog.descriptors.is_empty(),
        "the shipped content has descriptors"
    );
    for descriptor in &catalog.descriptors {
        // The reference the catalog minted is the reference the frozen registry
        // holds, digest included — which is the whole claim: a persisted unit
        // points at exact versions.
        let reference = catalog
            .references
            .get(&mush_core::CanonicalId::new(descriptor.id().as_str()).expect("a canonical id"))
            .cloned();
        if let Some(reference) = reference {
            let entry = registry
                .get_exact(&reference)
                .unwrap_or_else(|error| panic!("{} is not registered: {error}", descriptor.id()));
            assert_eq!(entry.payload(), descriptor.payload());
            assert_eq!(entry.reference().version(), descriptor.version());
            assert_eq!(
                entry.reference().key().owner.as_str(),
                tacsim::NS,
                "every game definition is owned by this layer"
            );
        }
    }
    // And the unit schema is the key's *current* version, which is what
    // `@unit/create <obj>=<id>` resolves through.
    let spec = catalog
        .units
        .get_current(content::LINE_TANK, registry)
        .expect("the stock tank is the current unit schema");
    assert_eq!(spec.definition.key().id.as_str(), content::LINE_TANK);
}

#[test]
fn the_production_composition_can_make_the_stock_tank() {
    let mut tw = TestWorld::with_modules(production_roster());
    let thing = tw
        .world_mut()
        .create(mush_core::ObjKind::Thing, "Spare Hull", GOD, Some(ObjId(0)))
        .expect("a thing");
    let events = tw.run(
        GOD,
        &format!("@unit/create #{}={}", thing.0, content::LINE_TANK),
    );
    let text = events
        .iter()
        .map(|(_, e)| mush_testkit::text_of(e))
        .collect::<Vec<_>>()
        .join(
            "
",
        );
    assert!(
        text.contains("is now a tactical unit"),
        "the production catalog carries the stock tank: {text}"
    );
}

// ---------------------------------------------------------------------------
// (3) The window
// ---------------------------------------------------------------------------

#[test]
fn from_pack_yields_the_window_and_the_right_impassable_cells() {
    let tw = TestWorld::with_modules(production_roster());
    let records = tw
        .world()
        .module_state::<mush_atlas::AtlasState>()
        .map(|state| state.records.clone())
        .expect("the atlas is composed");
    let catalog = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    let (id, def) = catalog
        .arenas
        .iter()
        .next()
        .expect("the proving ground is shipped");
    let arena = tacsim::definitions::from_pack(id.as_str(), def, &records)
        .expect("the arena's ground is in this world's records");
    assert_eq!(arena.terrain.width(), 256);
    assert_eq!(arena.terrain.height(), 256);
    assert_eq!(arena.terrain.cell_cm(), 1_000);
    assert_eq!(arena.entry_zones.len(), 2);

    // The impassable set is exactly what the shipped `tracked` row refuses,
    // which on the authored map is the void, the walls, the deep water and the
    // heavy woods — and is **not** the shallow water of the ford, which the
    // table prices at 300.
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let mut impassable: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut passable: BTreeMap<&'static str, usize> = BTreeMap::new();
    for cell in arena.terrain.cells() {
        let refused = !table.passable(cell.class.label(), "tracked", &[]);
        assert_eq!(
            cell.move_cost == 0,
            refused,
            "{:?} disagrees with the shipped locomotion table",
            cell.class
        );
        if refused {
            *impassable.entry(cell.class.label()).or_default() += 1;
        } else {
            *passable.entry(cell.class.label()).or_default() += 1;
        }
    }
    let refused: Vec<&str> = impassable.keys().copied().collect();
    assert_eq!(refused, ["wall", "water_deep", "woods_heavy"]);
    assert!(
        passable.contains_key("water"),
        "the ford's shallow water is passable at a cost: {passable:?}"
    );
    assert!(passable.contains_key("road") && passable.contains_key("woods_light"));
}

#[test]
fn an_arena_whose_ground_is_missing_refuses_rather_than_falling_back() {
    // A world with the atlas but without the arena's region: `from_pack` names
    // the record, and `@arena/start` refuses rather than fighting the match on
    // flat ground nobody authored.
    let tw = TestWorld::with_modules(vec![Box::new(mush_atlas::AtlasModule::default())]);
    let records = tw
        .world()
        .module_state::<mush_atlas::AtlasState>()
        .map(|state| state.records.clone())
        .expect("the atlas is composed");
    let catalog = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    let (id, def) = catalog.arenas.iter().next().expect("one arena");
    let error = tacsim::definitions::from_pack(id.as_str(), def, &records)
        .expect_err("no region, no arena");
    assert!(
        error.to_string().contains("region.proving_ground"),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// (4) and (5) The world after boot
// ---------------------------------------------------------------------------

#[test]
fn a_fresh_boot_makes_the_eight_tanks_and_a_second_makes_no_ninth() {
    let mut tw = TestWorld::with_modules(reboot_roster());
    let red = objects_named(&tw, "Red Tank ");
    let blue = objects_named(&tw, "Blue Tank ");
    assert_eq!(red.len(), 4, "four red tanks after a first boot");
    assert_eq!(blue.len(), 4, "four blue tanks after a first boot");

    // Each is a tactical unit, placed, at the cell its pack authors.
    for unit in red.iter().chain(blue.iter()) {
        assert!(
            mush_tactics::module::unit_of(tw.world(), *unit).is_some(),
            "#{} is a tactical unit",
            unit.0
        );
        let (_, pose) = mush_sim::pose_of(tw.world(), *unit).expect("it is placed");
        assert_eq!(pose.x_cm % 1_000, 500, "on a cell centre");
        assert_eq!(pose.y_cm % 1_000, 500);
    }

    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    assert_eq!(objects_named(&tw, "Red Tank ").len(), 4, "no ninth tank");
    assert_eq!(objects_named(&tw, "Blue Tank ").len(), 4);
}

#[test]
fn the_ten_commodity_families_are_instruments_after_boot() {
    let tw = TestWorld::with_modules(production_roster());
    let store = tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("the economy is composed");
    let snapshot = store.snapshot();
    for family in tacsim::definitions::resources::FAMILIES {
        let id = mush_economy::InstrumentId::new(format!("tacsim.{family}"))
            .expect("a well-formed instrument id");
        let def = snapshot
            .instrument(&id)
            .unwrap_or_else(|| panic!("tacsim.{family} is registered"));
        assert_eq!(def.kind, mush_economy::InstrumentKind::Commodity);
        assert_eq!(def.scale, 0);
    }
    // ...beside the settlement credit, which cut R-b registered and this cut
    // leaves exactly as it was.
    let credit = mush_economy::InstrumentId::new(tacsim::CREDIT).expect("the credit id");
    assert_eq!(
        snapshot
            .instrument(&credit)
            .map(|def| def.kind)
            .expect("the credit is registered"),
        mush_economy::InstrumentKind::Currency
    );
}

#[test]
fn a_match_on_the_shipped_arena_is_fought_on_the_authored_ground() {
    // The end-to-end claim: the arena `@arena/start` resolves is the pack's, not
    // T.3's uniform stand-in, and the two are told apart by the one thing the
    // stand-in has not got — a cell a tracked unit cannot enter.
    let tw = TestWorld::with_modules(production_roster());
    let id = SmolStr::new(content::PROVING_GROUND);
    let authored = tacsim::match_::definition_for(tw.world(), &id)
        .expect("the ground is in this world's records")
        .expect("the arena resolves");
    assert!(
        authored
            .terrain
            .cells()
            .iter()
            .any(|cell| cell.move_cost == 0),
        "the authored window has impassable ground"
    );
    let stand_in = tacsim::match_::stock_definition(&id).expect("the stand-in resolves");
    assert_eq!(authored.terrain.width(), stand_in.terrain.width());
    assert_ne!(
        authored.digest(),
        stand_in.digest(),
        "the authored arena and the stand-in are different arenas"
    );
}
