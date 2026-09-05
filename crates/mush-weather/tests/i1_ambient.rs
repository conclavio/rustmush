//! Interface cut I1: the weather module's structured glance under
//! `RoomView.ambient["weather"]`, beside (not instead of) its prose line.

use std::time::Duration;

use mush_core::output::room_view;
use mush_core::{Flags, GOD, LIMBO, ObjKind};
use mush_testkit::TestWorld;
use mush_weather::{WeatherConfig, WeatherModule};

const CLIMATE: &str = "temperate-coastal";

#[test]
fn room_ambient_carries_the_weather_glance_for_outdoor_rooms_only() {
    let module = WeatherModule::with_config(WeatherConfig {
        tick: Duration::from_secs(300),
        ambient_permille: 0,
        ..WeatherConfig::default()
    });
    let mut tw = TestWorld::with_modules(vec![Box::new(module)]);
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

    let rv = room_view(tw.registry(), tw.world(), player, outdoor).expect("room renders");
    let wx = rv
        .ambient
        .get("weather")
        .expect("outdoor room has a glance");
    assert!(wx["state"].is_string(), "{wx}");
    assert_eq!(wx["icon"], wx["state"], "the icon hint is the state id");
    assert!(wx["temp"].is_number(), "{wx}");
    assert!(
        wx["line"].as_str().is_some_and(|l| l.contains("Overhead:")),
        "the prose line rides along: {wx}"
    );
    assert!(
        rv.ambient.contains_key("ic_time"),
        "core's clock is still there"
    );

    let rv = room_view(tw.registry(), tw.world(), player, indoor).expect("room renders");
    assert!(
        !rv.ambient.contains_key("weather"),
        "an INDOORS room bears no weather: {:?}",
        rv.ambient
    );
}
