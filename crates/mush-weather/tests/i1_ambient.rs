//! Interface cut I1: the weather module's structured glance under
//! `RoomView.ambient["weather"]`; since I3 its `line` is the only prose the
//! module contributes to a room render.

mod common;

use common::{downtown_with_climate, room, world};
use mush_core::output::room_view;

#[test]
fn room_ambient_carries_the_weather_glance_for_outdoor_rooms_only() {
    let mut tw = world(300, 0);
    let outdoor = downtown_with_climate(&mut tw);
    let indoor = room(&tw, "Cellar");
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
