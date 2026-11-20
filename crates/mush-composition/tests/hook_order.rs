//! CRIT-17 regression tests: deny-only `MoveAttempt` hooks (`Scenes`, `Simulation`,
//! `Tactics`, `Industry`, `Housing`) must be registered BEFORE `Atlas` (`TravelHook`),
//! so an intercepting timed-portal departure cannot short-circuit and bypass deny-only
//! movement vetoes.

use mush_atlas::AtlasState;
use mush_content::{Appliers, ContentModule, parse_pack};
use mush_core::{
    Effects, Engine, GOD, HookPoint, Inbound, ObjId, ObjKind, Outbox, Registry, World,
};
use mush_economy::EconomyStore;

fn module_position(modules: &[Box<dyn mush_core::GameModule>], name: &str) -> usize {
    modules
        .iter()
        .position(|m| m.name() == name)
        .unwrap_or_else(|| panic!("module {name:?} missing from server_modules"))
}

fn signed_pack(mut doc: serde_json::Value) -> mush_content::Pack {
    doc["manifest"]["digest"] = serde_json::Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = serde_json::Value::String(digest);
    parse_pack(&serde_json::to_string(&doc).expect("json")).expect("signed pack parses")
}

fn run_cmd(engine: &mut Engine, conn: u64, actor: ObjId, line: &str) -> String {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::Cmd {
            conn,
            actor,
            line: line.to_string(),
        },
        &mut out,
        &mut fx,
    );
    format!("{:?}", out.take())
}

#[test]
fn crit17_deny_only_move_hooks_precede_atlas_in_server_modules() {
    let modules = mush_composition::server_modules();
    let atlas_pos = module_position(&modules, mush_atlas::NS);
    for deny_only in ["scenes", "sim", "tactics", "industry", "housing"] {
        let pos = module_position(&modules, deny_only);
        assert!(
            pos < atlas_pos,
            "deny-only MoveAttempt module {deny_only:?} (index {pos}) must register before \
             atlas (index {atlas_pos})"
        );
    }

    // Also verify the live HookPoint::MoveAttempt chain order on the booted Engine.
    let engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        mush_composition::server_modules(),
    )
    .expect("server_modules boot cleanly");
    let hook_modules: Vec<&str> = engine
        .registry
        .hooks(HookPoint::MoveAttempt)
        .iter()
        .map(|reg| reg.module.as_str())
        .collect();
    let atlas_hook_idx = hook_modules
        .iter()
        .position(|m| *m == mush_atlas::NS)
        .expect("atlas MoveAttempt hook registered");
    for deny_only in ["scenes", "sim", "housing"] {
        let idx = hook_modules
            .iter()
            .position(|m| *m == deny_only)
            .unwrap_or_else(|| {
                panic!("{deny_only} MoveAttempt hook missing from {hook_modules:?}")
            });
        assert!(
            idx < atlas_hook_idx,
            "{deny_only} MoveAttempt hook (idx {idx}) must precede atlas (idx {atlas_hook_idx}) \
             in {hook_modules:?}"
        );
    }
}

#[test]
fn crit17_housing_denies_timed_portal_before_travel_hook_intercepts() {
    let game = tacsim::economy::GameEconomyConfig {
        housing: true,
        ..Default::default()
    };
    let mut modules = mush_composition::server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &mush_core::OnboardingConfig::default(),
        &game,
        mush_campaign::RecoveryConfig::default(),
    );

    // Load a real timed portal through `ContentModule` + `mush_atlas` appliers
    // (zero `module_state_mut` fiat writes).
    let region = signed_pack(serde_json::json!({
        "manifest": { "id": "region.crit17", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.crit17": { "kind": "region", "def": {
                "name": "HookTown", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [100, 100]], "datum_m": 0.0,
                "background": { "surface": "grass", "mix": [], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 1000 } },
            "locality.crit17": { "kind": "locality", "def": {
                "region": "region.crit17", "name": "Lane", "kind": "district",
                "footprint": [[0, 0], [100, 0], [100, 100], [0, 100]] } }
        }
    }));
    let hood = signed_pack(serde_json::json!({
        "manifest": { "id": "neighborhood.crit17", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.crit17", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.crit17", "locality": "locality.crit17", "name": "Row",
                "setting": "rural", "footprint": [[0, 0], [100, 0], [100, 100], [0, 100]],
                "room_budget": { "target": 2, "maximum": 8 } } },
            "room.street": { "kind": "room", "def": {
                "name": "Street", "surface": "ground",
                "footprint": { "rect": [[10, 10], [40, 40]] } } },
            "room.flat": { "kind": "room", "def": {
                "name": "Private Flat", "surface": "ground",
                "footprint": { "rect": [[10, 60], [40, 90]] } } },
            "exit.flat": { "kind": "exit", "def": {
                "from": "room.street", "to": "room.flat",
                "names": ["Flat <FLAT>;flat;north", "Street <ST>;street;south"],
                "portal": { "distance_m": 300, "timed": ["foot"] } } }
        }
    }));
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    modules.push(Box::new(
        ContentModule::new()
            .with_appliers(appliers)
            .with_boot_packs(vec![region, hood]),
    ));

    let mut engine = Engine::with_modules(World::new_minimal(), Registry::new(), modules)
        .expect("server_modules_with(housing=true) + content boot cleanly");
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect("modules_loaded");
    let _ = out.take();

    let room_a = engine
        .world
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == "Street")
        .map(|o| o.id)
        .expect("Street room from pack");
    let room_b = engine
        .world
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == "Private Flat")
        .map(|o| o.id)
        .expect("Private Flat room from pack");

    let bob = engine
        .world
        .create(ObjKind::Player, "Bob", GOD, Some(room_b))
        .expect("bob");
    let alice = engine
        .world
        .create(ObjKind::Player, "Alice", GOD, Some(room_a))
        .expect("alice");

    // Let `room_b` to Bob via Housing (whose deny-only AccessHook previously
    // registered AFTER Atlas in `server_modules`).
    engine.step(
        Inbound::ConnUp {
            conn: 1,
            actor: GOD,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    engine.step(
        Inbound::ConnUp {
            conn: 2,
            actor: alice,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
    let _ = out.take();

    let _ = run_cmd(
        &mut engine,
        1,
        GOD,
        "@econ/cadence create chronicle source=manual",
    );
    let cadence = engine
        .world
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().cadences().next().map(|c| c.id.0))
        .expect("cadence created");
    let prop_out = run_cmd(
        &mut engine,
        1,
        GOD,
        &format!(
            "@property/create Private Flat=#{} tier=2 landlord=housing",
            room_b.0
        ),
    );
    assert!(
        !prop_out.contains("error"),
        "property creation must succeed: {prop_out}"
    );
    let ten_out = run_cmd(
        &mut engine,
        1,
        GOD,
        &format!("@tenancy/create 0=#{} cadence={cadence}", bob.0),
    );
    assert!(
        !ten_out.contains("error"),
        "tenancy creation must succeed: {ten_out}"
    );

    // Stranger Alice walks through the timed portal exit toward Bob's private flat.
    let rendered = run_cmd(&mut engine, 2, alice, "flat");
    assert_eq!(
        engine.world.get(alice).and_then(|o| o.location),
        Some(room_a),
        "Alice must remain on Street: {rendered}"
    );
    assert!(
        rendered.contains("You can't go that way."),
        "Housing AccessHook must deny first: {rendered}"
    );
    assert!(
        !rendered.contains("You set off along"),
        "TravelHook must NOT intercept before Housing denies: {rendered}"
    );

    // Fire due ticks to verify TravelHook did NOT queue a PendingDeparture.
    let now = engine.world.now() + 30;
    engine.world.set_now(now);
    engine.fire_due(now.saturating_mul(1_000), &mut out, &mut fx);
    assert!(
        engine
            .world
            .module_state::<AtlasState>()
            .expect("atlas")
            .travel
            .journey_of(alice)
            .is_none(),
        "no journey may start after Housing vetoed the timed portal"
    );
    assert_eq!(
        engine.world.get(alice).and_then(|o| o.location),
        Some(room_a),
        "Alice must not be moved into transit or the private flat"
    );
}
