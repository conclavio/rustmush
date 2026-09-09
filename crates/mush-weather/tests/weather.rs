//! End-to-end weather-module tests through the real `Engine` (via
//! `mush-testkit`): climate load, seeded-tick determinism, pin/release, room
//! flag gating, the room-view glance, empty-room ambient suppression, the
//! `WxSnapshot` query seam, panel keyframe/delta emission, the S2b place keying
//! (a neighborhood's cell, else its region's) and the full persistence round
//! trip (module ops → `InMemoryStore` → fresh boot).

mod common;

use std::collections::HashMap;

use common::{CLIMATE, COAST, DOWNTOWN, DUNES, cell_state, downtown_with_climate, room, world};
use mush_core::{Audience, GOD, LIMBO, ObjId, ObjKind};
use mush_proto::{OutputEvent, PanelFrameKind};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::TestWorld;
use mush_weather::{WeatherState, WxRng};
use smol_str::SmolStr;

/// Force a known RNG stream on the live weather state (deterministic tests).
fn seed(tw: &mut TestWorld, s: u64) {
    tw.world_mut()
        .module_state_mut::<WeatherState>()
        .unwrap()
        .rng = WxRng::seeded(s);
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
fn climate_assignment_opts_a_place_in_by_id_name_or_room() {
    let mut tw = world(300, 100);
    let out = tw.run(GOD, &format!("@weather/climate {DOWNTOWN}={CLIMATE}"));
    assert!(
        tw.seen_by(&out, GOD)
            .iter()
            .any(|l| l.contains("Downtown now uses climate")),
        "assign should confirm: {:?}",
        tw.seen_by(&out, GOD)
    );
    let (state, pinned) = cell_state(&tw, DOWNTOWN).expect("cell created");
    assert_eq!(state, "clear", "fresh cells start clear");
    assert!(!pinned);
    // By name, and by a room in it, the same cell answers.
    let out = tw.run(GOD, "@weather Downtown");
    assert!(
        tw.seen_by(&out, GOD)
            .iter()
            .any(|l| l.contains("Weather for Downtown (neighborhood.downtown)")),
        "{:?}",
        tw.seen_by(&out, GOD)
    );
    let plaza = room(&tw, "Plaza");
    let out = tw.run(GOD, &format!("@weather #{}", plaza.0));
    assert!(
        tw.seen_by(&out, GOD)
            .iter()
            .any(|l| l.contains("Weather for Downtown")),
        "{:?}",
        tw.seen_by(&out, GOD)
    );
    // A room off the map keys on nothing.
    let out = tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", LIMBO.0));
    assert!(
        tw.seen_by(&out, GOD)
            .iter()
            .any(|l| l.contains("on no map")),
        "{:?}",
        tw.seen_by(&out, GOD)
    );
}

#[test]
fn transitions_are_deterministic_under_a_seed() {
    // Same injected seed → identical state trajectory across two worlds.
    let trail = |s: u64| {
        let mut tw = world(60, 0);
        downtown_with_climate(&mut tw);
        seed(&mut tw, s);
        let mut trail = Vec::new();
        for k in 1..=25 {
            tw.fire_due(k * 60_000);
            trail.push(cell_state(&tw, DOWNTOWN).unwrap().0);
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
    let plaza = downtown_with_climate(&mut tw);

    let out = tw.run(GOD, &format!("@weather {DOWNTOWN}=storm"));
    assert_eq!(
        cell_state(&tw, DOWNTOWN).unwrap(),
        (SmolStr::new("storm"), true)
    );
    // The pin broadcasts a transition line (icon = the pinned state) to the
    // outdoor room.
    let frames = weather_to_room(&out, plaza);
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
        cell_state(&tw, DOWNTOWN).unwrap().0,
        "storm",
        "pinned cell must not move"
    );

    // Release resumes the machine.
    tw.run(GOD, &format!("@weather/release {DOWNTOWN}"));
    assert!(
        !cell_state(&tw, DOWNTOWN).unwrap().1,
        "release clears the pin"
    );
}

#[test]
fn room_flags_gate_transition_broadcasts() {
    let mut tw = world(60, 0);
    // Downtown holds an OUTDOORS plaza, a SHELTERED awning and an INDOORS cellar.
    let outdoor = downtown_with_climate(&mut tw);
    let sheltered = room(&tw, "Awning");
    let indoor = room(&tw, "Cellar");

    let out = tw.run(GOD, &format!("@weather {DOWNTOWN}=rain"));

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
fn a_room_takes_its_neighborhood_cell_else_its_region_cell() {
    let mut tw = world(60, 0);
    let plaza = room(&tw, "Plaza");
    let pier = room(&tw, "Pier");
    // The whole coast on one cell: both neighborhoods' outdoor rooms hear it.
    tw.run(GOD, &format!("@weather/climate {COAST}={CLIMATE}"));
    let out = tw.run(GOD, &format!("@weather {COAST}=storm"));
    assert_eq!(
        weather_to_room(&out, plaza).len(),
        1,
        "the plaza hears the coast"
    );
    assert_eq!(
        weather_to_room(&out, pier).len(),
        1,
        "the pier hears the coast"
    );
    // Downtown gets its own cell: its rooms follow it and leave the coast's.
    let out = tw.run(GOD, &format!("@weather/climate {DOWNTOWN}=desert"));
    assert_eq!(
        weather_to_room(&out, plaza).len(),
        1,
        "downtown's own cell speaks"
    );
    assert!(
        weather_to_room(&out, pier).is_empty(),
        "the dunes stay on the coast's cell"
    );
    let out = tw.run(GOD, &format!("@weather {COAST}=rain"));
    assert!(
        weather_to_room(&out, plaza).is_empty(),
        "the coast no longer speaks for downtown"
    );
    assert_eq!(weather_to_room(&out, pier).len(), 1);
    // The query seam agrees.
    let (_owner, provider) = tw.registry().weather_query().unwrap();
    let plaza_wx = provider
        .current(tw.world(), plaza)
        .expect("plaza has weather");
    let pier_wx = provider
        .current(tw.world(), pier)
        .expect("pier has weather");
    assert_eq!(pier_wx.state, "rain");
    assert_ne!(
        plaza_wx.state, "rain",
        "downtown's desert cell, not the coast's rain"
    );
}

#[test]
fn look_carries_conditions_for_outdoor_rooms_only() {
    let mut tw = world(300, 0);
    let outdoor = downtown_with_climate(&mut tw);
    let indoor = room(&tw, "Cellar");

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
    let occupied = downtown_with_climate(&mut tw);
    let empty = room(&tw, "Alley");
    tw.run(GOD, &format!("@weather {DOWNTOWN}=rain")); // pin; drains via return

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
    let plaza = downtown_with_climate(&mut tw);
    let indoor = room(&tw, "Cellar");
    tw.run(GOD, &format!("@weather {DOWNTOWN}=rain"));

    let (_owner, provider) = tw.registry().weather_query().unwrap();
    let snap = provider
        .current(tw.world(), plaza)
        .expect("outdoor room has weather");
    assert_eq!(snap.state, "rain");
    assert!(snap.precipitation > 0.0 && snap.visibility <= 1.0);
    // INDOORS rooms report no weather to rulesets.
    assert!(provider.current(tw.world(), indoor).is_none());
    // Limbo, off the map: none.
    assert!(provider.current(tw.world(), LIMBO).is_none());
}

#[test]
fn panel_keyframe_on_connect_and_delta_on_change() {
    let mut tw = world(60, 0);
    let plaza = downtown_with_climate(&mut tw);

    // Delta: pinning a state broadcasts a panel delta to the outdoor room.
    let out = tw.run(GOD, &format!("@weather {DOWNTOWN}=storm"));
    let deltas = panels_to_room(&out, plaza, PanelFrameKind::Delta);
    assert!(!deltas.is_empty(), "state change emits a panel delta");
    assert_eq!(deltas[0]["state"], "storm");
    assert_eq!(deltas[0]["icon"], "storm");
    assert_eq!(deltas[0]["zone"], "Downtown", "the panel names the place");

    // Keyframe: a player who becomes live in a weather room gets a keyframe.
    let player = tw
        .world_mut()
        .create(ObjKind::Player, "Rae", GOD, Some(plaza))
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
    // panel never lingers on the previous place's (or an empty) state — the
    // tick's deltas only reach a room's current occupants, and a pinned cell
    // emits none.
    let mut tw = world(60, 0);
    let plaza = downtown_with_climate(&mut tw);
    tw.run(GOD, &format!("@weather {DOWNTOWN}=storm"));

    // A live player in the Plaza walks north into the Court.
    let player = tw
        .world_mut()
        .create(ObjKind::Player, "Rae", GOD, Some(plaza))
        .unwrap();
    tw.connect(player);
    let out = tw.run(player, "north");
    assert_eq!(
        tw.world().get(player).and_then(|o| o.location),
        Some(room(&tw, "Court"))
    );

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
    // Two places, one pinned, one running.
    tw.run(GOD, &format!("@weather/climate {DOWNTOWN}={CLIMATE}"));
    tw.run(GOD, &format!("@weather {DOWNTOWN}=storm"));
    tw.run(GOD, &format!("@weather/climate {DUNES}=desert"));

    let ops: Vec<_> = tw
        .take_module_ops()
        .into_iter()
        .filter(|op| op.ns == "weather")
        .collect();
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

    // A fresh world with the same packs applied (the same places), booted from
    // the store's weather documents.
    let mut tw2 = world(300, 0);
    tw2.modules_loaded(HashMap::from([(SmolStr::new("weather"), boot.into())]));

    let cells1 = &tw.world().module_state::<WeatherState>().unwrap().cells;
    let cells2 = &tw2.world().module_state::<WeatherState>().unwrap().cells;
    assert_eq!(cells1, cells2, "every cell round-trips identically");
    // Spot check the pin survived.
    let downtown = mush_core::CanonicalId::new(DOWNTOWN).unwrap();
    let dunes = mush_core::CanonicalId::new(DUNES).unwrap();
    assert!(cells2.get(&downtown).unwrap().pinned);
    assert_eq!(cells2.get(&downtown).unwrap().state, "storm");
    assert_eq!(cells2.get(&dunes).unwrap().climate, "desert");
}

#[test]
fn boot_drops_zone_keyed_cells_and_keeps_place_keyed_ones() {
    // A pre-atlas cell keyed by a zone dbref is dropped (doc 14 D-14-28: zones
    // keep locks and exits only); a place-keyed cell is rebuilt.
    let boot = ModuleBootDataShim::from_kv(vec![
        (
            "cell:999".to_string(),
            serde_json::json!({
                "zone": 999, "climate": CLIMATE, "state": "rain",
                "since": 0, "temp": 5, "pinned": false
            }),
        ),
        (
            format!("cell:{DUNES}"),
            serde_json::json!({
                "place": DUNES, "climate": CLIMATE, "state": "fog",
                "since": 0, "temp": 5, "pinned": true
            }),
        ),
    ]);
    let mut tw = world(300, 0);
    tw.modules_loaded(HashMap::from([(SmolStr::new("weather"), boot.into())]));
    let cells = &tw.world().module_state::<WeatherState>().unwrap().cells;
    assert_eq!(cells.len(), 1, "the zone-keyed cell is gone: {cells:?}");
    let dunes = mush_core::CanonicalId::new(DUNES).unwrap();
    assert_eq!(cells.get(&dunes).unwrap().state, "fog");
    // The drop was made durable.
    let ops = tw.take_module_ops();
    assert!(
        ops.iter().any(|op| op.ns == "weather"
            && matches!(&op.op, mush_core::ModuleWrite::Del { key } if key == "cell:999")),
        "the legacy document is deleted: {ops:?}"
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
            ..Default::default()
        }
    }
}
