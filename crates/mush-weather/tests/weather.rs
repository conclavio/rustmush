//! End-to-end weather-module tests through the real `Engine` (via
//! `mush-testkit`): climate load, seeded-tick determinism, pin/release, room
//! flag gating, LookRendered append, empty-room ambient suppression, the
//! `WxSnapshot` query seam, panel keyframe/delta emission, and the full
//! persistence round trip (module ops → `InMemoryStore` → fresh boot).

use std::collections::HashMap;
use std::time::Duration;

use mush_core::{Audience, Flags, GOD, LIMBO, ObjId, ObjKind};
use mush_proto::{OutputEvent, PanelFrameKind};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::TestWorld;
use mush_weather::{WeatherConfig, WeatherModule, WeatherState, WxRng};
use smol_str::SmolStr;

const CLIMATE: &str = "temperate-coastal";

/// A world with the weather module. `ambient` sets the ambient probability
/// (per-mille) and `tick_secs` the coarse-tick interval.
fn world(tick_secs: u64, ambient: u32) -> TestWorld {
    let module = WeatherModule::with_config(WeatherConfig {
        tick: Duration::from_secs(tick_secs),
        ambient_permille: ambient,
    });
    TestWorld::with_modules(vec![Box::new(module)])
}

/// Create a zone master object and one room in it with the given flags,
/// returning `(zone, room)`.
fn zone_with_room(tw: &mut TestWorld, flags: Flags) -> (ObjId, ObjId) {
    let zone = tw
        .world_mut()
        .create(ObjKind::Thing, "Downtown", GOD, Some(LIMBO))
        .unwrap();
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "Plaza", GOD, None)
        .unwrap();
    tw.world_mut().set_zone(room, Some(zone)).unwrap();
    tw.world_mut().set_flag(room, flags, true).unwrap();
    (zone, room)
}

/// Force a known RNG stream on the live weather state (deterministic tests).
fn seed(tw: &mut TestWorld, s: u64) {
    tw.world_mut()
        .module_state_mut::<WeatherState>()
        .unwrap()
        .rng = WxRng::seeded(s);
}

fn cell_state(tw: &TestWorld, zone: ObjId) -> Option<(SmolStr, bool)> {
    tw.world()
        .module_state::<WeatherState>()
        .and_then(|s| s.cell(zone))
        .map(|c| (c.state.clone(), c.pinned))
}

/// Weather frames (text, icon) addressed to a room's occupants.
fn weather_to_room(events: &[(Audience, OutputEvent)], room: ObjId) -> Vec<(String, SmolStr)> {
    events
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (Audience::Room { room: r, .. }, OutputEvent::Weather { text, icon, .. })
                if *r == room =>
            {
                Some((text.to_plain_text(), icon.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Panel frames for the `weather` feed of a given kind, addressed to a room.
fn panels_to_room(
    events: &[(Audience, OutputEvent)],
    room: ObjId,
    want: PanelFrameKind,
) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (Audience::Room { room: r, .. }, OutputEvent::Panel { feed, kind, data })
                if *r == room && feed.as_str() == "weather" && *kind == want =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .collect()
}

// ----------------------------------------------------------------------

#[test]
fn module_registers_provider_and_panel() {
    let tw = world(300, 100);
    // The reserved WeatherQuery seam is filled by exactly this module.
    let (owner, _provider) = tw.registry().weather_query().expect("weather provider");
    assert_eq!(owner.as_str(), "weather");
    assert!(
        tw.registry()
            .panel_feeds()
            .iter()
            .any(|(f, _)| f.as_str() == "weather")
    );
    assert!(
        tw.registry()
            .script_apis()
            .iter()
            .any(|(n, _)| n.as_str() == "weather")
    );
}

#[test]
fn climate_assignment_opts_a_zone_in() {
    let mut tw = world(300, 100);
    let (zone, _room) = zone_with_room(&mut tw, Flags::OUTDOORS);
    let out = tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));
    assert!(
        tw.seen_by(&out, GOD)
            .iter()
            .any(|l| l.contains("now uses climate")),
        "assign should confirm: {:?}",
        tw.seen_by(&out, GOD)
    );
    let (state, pinned) = cell_state(&tw, zone).expect("cell created");
    assert_eq!(state, "clear", "fresh cells start clear");
    assert!(!pinned);
}

#[test]
fn transitions_are_deterministic_under_a_seed() {
    // Same injected seed → identical state trajectory across two worlds.
    let trail = |s: u64| {
        let mut tw = world(60, 0);
        let (zone, _room) = zone_with_room(&mut tw, Flags::OUTDOORS);
        tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));
        seed(&mut tw, s);
        let mut trail = Vec::new();
        for k in 1..=25 {
            tw.fire_due(k * 60_000);
            trail.push(cell_state(&tw, zone).unwrap().0);
        }
        trail
    };
    assert_eq!(trail(0xABCD), trail(0xABCD), "same seed → same trajectory");
    // A cell that never moved would be a useless test; assert it changed.
    let t = trail(0xABCD);
    assert!(
        t.iter().any(|s| s != "clear"),
        "machine never left clear: {t:?}"
    );
}

#[test]
fn pin_holds_state_and_release_resumes() {
    let mut tw = world(60, 0);
    let (zone, room) = zone_with_room(&mut tw, Flags::OUTDOORS);
    tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));

    let out = tw.run(GOD, &format!("@weather #{}=storm", zone.0));
    assert_eq!(
        cell_state(&tw, zone).unwrap(),
        (SmolStr::new("storm"), true)
    );
    // The pin broadcasts a transition line (icon = the pinned state) to the
    // outdoor room.
    let frames = weather_to_room(&out, room);
    assert!(
        frames.iter().any(|(_, icon)| icon.as_str() == "storm"),
        "pin should broadcast: {frames:?}"
    );

    // A pinned cell does not transition, whatever the RNG rolls.
    seed(&mut tw, 12345);
    for k in 1..=20 {
        tw.fire_due(k * 60_000);
    }
    assert_eq!(
        cell_state(&tw, zone).unwrap().0,
        "storm",
        "pinned cell must not move"
    );

    // Release resumes the machine.
    tw.run(GOD, &format!("@weather/release #{}", zone.0));
    assert!(!cell_state(&tw, zone).unwrap().1, "release clears the pin");
}

#[test]
fn room_flags_gate_transition_broadcasts() {
    let mut tw = world(60, 0);
    // One zone with three rooms: OUTDOORS, SHELTERED, INDOORS (no flags).
    let zone = tw
        .world_mut()
        .create(ObjKind::Thing, "Downtown", GOD, Some(LIMBO))
        .unwrap();
    let outdoor = tw
        .world_mut()
        .create(ObjKind::Room, "Plaza", GOD, None)
        .unwrap();
    let sheltered = tw
        .world_mut()
        .create(ObjKind::Room, "Awning", GOD, None)
        .unwrap();
    let indoor = tw
        .world_mut()
        .create(ObjKind::Room, "Cellar", GOD, None)
        .unwrap();
    for r in [outdoor, sheltered, indoor] {
        tw.world_mut().set_zone(r, Some(zone)).unwrap();
    }
    tw.world_mut()
        .set_flag(outdoor, Flags::OUTDOORS, true)
        .unwrap();
    tw.world_mut()
        .set_flag(sheltered, Flags::SHELTERED, true)
        .unwrap();
    tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));

    let out = tw.run(GOD, &format!("@weather #{}=rain", zone.0));

    // OUTDOORS: full transition line.
    let od = weather_to_room(&out, outdoor);
    assert_eq!(od.len(), 1, "outdoor room gets one transition line");
    assert!(
        !od[0].0.starts_with("Through the windows"),
        "outdoor line is not muffled: {od:?}"
    );

    // SHELTERED: muffled transition line.
    let sh = weather_to_room(&out, sheltered);
    assert_eq!(sh.len(), 1, "sheltered room gets one (muffled) line");
    assert!(
        sh[0].0.starts_with("Through the windows"),
        "sheltered line is muffled: {sh:?}"
    );

    // INDOORS: silent.
    assert!(
        weather_to_room(&out, indoor).is_empty(),
        "indoor room stays silent"
    );
}

#[test]
fn look_appends_conditions_for_outdoor_and_sheltered() {
    let mut tw = world(300, 0);
    let zone = tw
        .world_mut()
        .create(ObjKind::Thing, "Downtown", GOD, Some(LIMBO))
        .unwrap();
    let outdoor = tw
        .world_mut()
        .create(ObjKind::Room, "Plaza", GOD, None)
        .unwrap();
    let indoor = tw
        .world_mut()
        .create(ObjKind::Room, "Cellar", GOD, None)
        .unwrap();
    tw.world_mut().set_zone(outdoor, Some(zone)).unwrap();
    tw.world_mut().set_zone(indoor, Some(zone)).unwrap();
    tw.world_mut()
        .set_flag(outdoor, Flags::OUTDOORS, true)
        .unwrap();
    tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));

    let player = tw.spawn_player("Rae");
    tw.world_mut().move_to(player, outdoor).unwrap();
    let out = tw.run(player, "look");
    assert!(
        tw.seen_by(&out, player)
            .iter()
            .any(|l| l.contains("Overhead:")),
        "outdoor look shows conditions: {:?}",
        tw.seen_by(&out, player)
    );

    // Move to the INDOORS room: no weather line.
    tw.world_mut().move_to(player, indoor).unwrap();
    let out = tw.run(player, "look");
    assert!(
        !tw.seen_by(&out, player)
            .iter()
            .any(|l| l.contains("Overhead:")),
        "indoor look shows no weather"
    );
}

#[test]
fn ambient_suppressed_in_empty_rooms() {
    // Ambient always fires (permille 1000) for occupied outdoor rooms, and
    // only for them. Pin the cell so no transition steals the tick.
    let mut tw = world(60, 1000);
    let zone = tw
        .world_mut()
        .create(ObjKind::Thing, "Downtown", GOD, Some(LIMBO))
        .unwrap();
    let occupied = tw
        .world_mut()
        .create(ObjKind::Room, "Plaza", GOD, None)
        .unwrap();
    let empty = tw
        .world_mut()
        .create(ObjKind::Room, "Alley", GOD, None)
        .unwrap();
    tw.world_mut().set_zone(occupied, Some(zone)).unwrap();
    tw.world_mut().set_zone(empty, Some(zone)).unwrap();
    tw.world_mut()
        .set_flag(occupied, Flags::OUTDOORS, true)
        .unwrap();
    tw.world_mut()
        .set_flag(empty, Flags::OUTDOORS, true)
        .unwrap();
    tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));
    tw.run(GOD, &format!("@weather #{}=rain", zone.0)); // pin; drains via return

    let player = tw.spawn_player("Rae");
    tw.world_mut().move_to(player, occupied).unwrap();

    let out = tw.fire_due(60_000);
    assert_eq!(
        weather_to_room(&out, occupied).len(),
        1,
        "occupied outdoor room gets ambient"
    );
    assert!(
        weather_to_room(&out, empty).is_empty(),
        "empty room is suppressed"
    );
}

#[test]
fn wx_snapshot_query_reads_current_conditions() {
    let mut tw = world(300, 0);
    let (zone, room) = zone_with_room(&mut tw, Flags::OUTDOORS);
    let indoor = tw
        .world_mut()
        .create(ObjKind::Room, "Cellar", GOD, None)
        .unwrap();
    tw.world_mut().set_zone(indoor, Some(zone)).unwrap();
    tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));
    tw.run(GOD, &format!("@weather #{}=rain", zone.0));

    let (_owner, provider) = tw.registry().weather_query().unwrap();
    let snap = provider
        .current(tw.world(), room)
        .expect("outdoor room has weather");
    assert_eq!(snap.state, "rain");
    assert!(snap.precipitation > 0.0 && snap.visibility <= 1.0);
    // INDOORS rooms report no weather to rulesets.
    assert!(provider.current(tw.world(), indoor).is_none());
    // Unzoned Limbo: none.
    assert!(provider.current(tw.world(), LIMBO).is_none());
}

#[test]
fn panel_keyframe_on_connect_and_delta_on_change() {
    let mut tw = world(60, 0);
    let (zone, room) = zone_with_room(&mut tw, Flags::OUTDOORS);
    tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));

    // Delta: pinning a state broadcasts a panel delta to the outdoor room.
    let out = tw.run(GOD, &format!("@weather #{}=storm", zone.0));
    let deltas = panels_to_room(&out, room, PanelFrameKind::Delta);
    assert!(!deltas.is_empty(), "state change emits a panel delta");
    assert_eq!(deltas[0]["state"], "storm");
    assert_eq!(deltas[0]["icon"], "storm");

    // Keyframe: a player who becomes live in a weather room gets a keyframe.
    let player = tw
        .world_mut()
        .create(ObjKind::Player, "Rae", GOD, Some(room))
        .unwrap();
    let connect = tw.connect(player);
    let key: Vec<_> = connect
        .iter()
        .filter(|(aud, ev)| {
            matches!(aud, Audience::Actor(a) if *a == player)
                && matches!(ev, OutputEvent::Panel { feed, kind, .. }
                    if feed == "weather" && *kind == PanelFrameKind::Keyframe)
        })
        .collect();
    assert_eq!(
        key.len(),
        1,
        "connect emits one weather keyframe to the actor"
    );
}

#[test]
fn panel_keyframe_refreshes_on_room_change() {
    // Walking into a weather room emits a fresh keyframe to the mover, so the
    // panel never lingers on the previous zone's (or an empty) state — the
    // tick's deltas only reach a room's current occupants, and a pinned zone
    // emits none.
    let mut tw = world(60, 0);
    let zone = tw
        .world_mut()
        .create(ObjKind::Thing, "Downtown", GOD, Some(LIMBO))
        .unwrap();
    let plaza = tw
        .world_mut()
        .create(ObjKind::Room, "Plaza", GOD, None)
        .unwrap();
    let court = tw
        .world_mut()
        .create(ObjKind::Room, "Court", GOD, None)
        .unwrap();
    for r in [plaza, court] {
        tw.world_mut().set_zone(r, Some(zone)).unwrap();
        tw.world_mut().set_flag(r, Flags::OUTDOORS, true).unwrap();
    }
    tw.world_mut()
        .create(ObjKind::Exit, "north;n", GOD, Some(plaza))
        .unwrap();
    let exit = tw.world_mut().exits_of(plaza)[0];
    tw.world_mut().link_exit(exit, Some(court)).unwrap();

    tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));
    tw.run(GOD, &format!("@weather #{}=storm", zone.0));

    // A live player in the Plaza walks north into the Court.
    let player = tw
        .world_mut()
        .create(ObjKind::Player, "Rae", GOD, Some(plaza))
        .unwrap();
    tw.connect(player);
    let out = tw.run(player, "north");

    let keys: Vec<_> = out
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (Audience::Actor(x), OutputEvent::Panel { feed, kind, data })
                if *x == player && feed == "weather" && *kind == PanelFrameKind::Keyframe =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        keys.len(),
        1,
        "entering a weather room emits one keyframe to the mover: {out:?}"
    );
    assert_eq!(
        keys[0]["state"], "storm",
        "keyframe carries the new room's state"
    );
}

// ----------------------------------------------------------------------
// Persistence round trip: mutate cells → module ops → InMemoryStore → fresh
// boot → identical cells.
// ----------------------------------------------------------------------

#[test]
fn persistence_round_trip_rebuilds_identical_cells() {
    let mut tw = world(300, 0);
    // Two zones, one pinned, one running.
    let z1 = tw
        .world_mut()
        .create(ObjKind::Thing, "Coast", GOD, Some(LIMBO))
        .unwrap();
    let r1 = tw
        .world_mut()
        .create(ObjKind::Room, "Pier", GOD, None)
        .unwrap();
    tw.world_mut().set_zone(r1, Some(z1)).unwrap();
    tw.world_mut().set_flag(r1, Flags::OUTDOORS, true).unwrap();
    let z2 = tw
        .world_mut()
        .create(ObjKind::Thing, "Dunes", GOD, Some(LIMBO))
        .unwrap();

    tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", z1.0));
    tw.run(GOD, &format!("@weather #{}=storm", z1.0));
    tw.run(GOD, &format!("@weather/climate #{}=desert", z2.0));

    let ops = tw.take_module_ops();
    assert!(!ops.is_empty(), "assign + pin should persist cells");

    // Through a real store and back into boot data.
    let store = InMemoryStore::new();
    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
    let kv =
        futures::executor::block_on(async { store.module_json_list("weather").await.unwrap() });
    let boot = ModuleBootDataShim::from_kv(kv);

    // A fresh world with the same zone objects recreated (same ids), booted
    // from the store.
    let mut tw2 = world(300, 0);
    // Recreate the exact object graph so the cell zones resolve and survive.
    let z1b = tw2
        .world_mut()
        .create(ObjKind::Thing, "Coast", GOD, Some(LIMBO))
        .unwrap();
    let r1b = tw2
        .world_mut()
        .create(ObjKind::Room, "Pier", GOD, None)
        .unwrap();
    tw2.world_mut().set_zone(r1b, Some(z1b)).unwrap();
    tw2.world_mut()
        .set_flag(r1b, Flags::OUTDOORS, true)
        .unwrap();
    let z2b = tw2
        .world_mut()
        .create(ObjKind::Thing, "Dunes", GOD, Some(LIMBO))
        .unwrap();
    assert_eq!((z1b, z2b), (z1, z2), "reboot recreates the same object ids");

    tw2.modules_loaded(HashMap::from([(SmolStr::new("weather"), boot.into())]));

    let cells1 = &tw.world().module_state::<WeatherState>().unwrap().cells;
    let cells2 = &tw2.world().module_state::<WeatherState>().unwrap().cells;
    assert_eq!(cells1, cells2, "every cell round-trips identically");
    // Spot check the pin survived.
    assert!(cells2.get(&z1).unwrap().pinned);
    assert_eq!(cells2.get(&z1).unwrap().state, "storm");
    assert_eq!(cells2.get(&z2).unwrap().climate, "desert");
}

#[test]
fn boot_drops_cells_for_missing_zones() {
    // A cell whose zone object no longer exists must not be rebuilt.
    let boot = ModuleBootDataShim::from_kv(vec![(
        "cell:999".to_string(),
        serde_json::json!({
            "zone": 999, "climate": CLIMATE, "state": "rain",
            "since": 0, "temp": 5, "pinned": false
        }),
    )]);
    let mut tw = world(300, 0);
    tw.modules_loaded(HashMap::from([(SmolStr::new("weather"), boot.into())]));
    assert!(
        tw.world()
            .module_state::<WeatherState>()
            .unwrap()
            .cells
            .is_empty(),
        "cell for a nonexistent zone is dropped"
    );
}

/// A tiny helper mirroring the driver's kv → `ModuleBootData` shaping (weather
/// has no streams).
struct ModuleBootDataShim {
    kv: Vec<(SmolStr, serde_json::Value)>,
}

impl ModuleBootDataShim {
    fn from_kv(kv: Vec<(String, serde_json::Value)>) -> ModuleBootDataShim {
        ModuleBootDataShim {
            kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        }
    }
}

impl From<ModuleBootDataShim> for mush_core::ModuleBootData {
    fn from(s: ModuleBootDataShim) -> Self {
        mush_core::ModuleBootData {
            kv: s.kv,
            streams: HashMap::new(),
        }
    }
}
