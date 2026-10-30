//! **The `arena.observer` payload** (16b cut I.4; 16c N10, the referee's third
//! lever; doc 16 §1, §5).
//!
//! `@arena/watch <match>` binds a staffer to a running match, and the arena
//! builds, at most once per pulse and at most once an arena second, a
//! [`crate::arena::observer::Observer`] — the whole field, objective, every unit
//! by object number. This module turns one into the keyframe the world
//! publishes on [`crate::arena_bridge::OBSERVER_FEED`] to at most
//! [`crate::arena_bridge::MAX_WATCHERS`] watchers, and says what a panel is sent
//! when there is nothing to watch ([`observer_cleared`]).
//!
//! # Deliberately not in the cockpit module
//!
//! The cockpit module builds what a **pilot** is sent, and doc 16 §1 says a
//! pilot is told nothing about a unit their platform has not detected. This
//! payload names every unit, so it lives here and the cockpit module never
//! names it: `crates/mush-contract-tests/tests/arena_library.rs`'s
//! `the_observer_has_no_path_into_a_pilot_feed` scans that file (and the text
//! of `Arena::frames`) for this module's names. It is a **staff surface** and
//! names object numbers on purpose.
//!
//! # The shape
//!
//! ```json
//! { "schema": { "id": "arena.observer", "version": 1, "retain": "latest" },
//!   "match": 12, "now_ms": 90000, "clock": "1:30", "tempo": "engaged",
//!   "floor_ms": null, "cell_cm": 1000, "width": 256, "height": 256,
//!   "in_flight": 2,
//!   "units": [["RA", "R", 120, 97, 90, 301, "pm"], ["BA", "B", null, null, 270, 302, "w"]] }
//! ```
//!
//! One positional row per unit, in unit-id order: `[callsign, team letter, x
//! cell or null, y cell or null, heading in whole degrees, object number,
//! flags]`, the flags a string of `w` wreck, `p` power online, `s` starting,
//! `m` machine (a mind flies it — or, in a fixture until 17b cut M4.6 deleted
//! it, the reflex controller),
//! `h` held — in that order, `""`
//! when none. Compact on purpose: a 300-unit picture serialises well under the
//! 16,384-byte frame bound (`[tacsim.harness].max_frame_bytes`), pinned below.
//! The payload declares no `shape`, so a line client prints nothing for it; the
//! TUI and the web draw it on screen 0's picture layout as a live picture.

use serde_json::Value;

use crate::arena::observer::{Observer, ObserverUnit};
use crate::arena::state::{ArenaTerrain, Power};
use crate::arena_task::MatchId;

/// The feed's schema id — the feed's own name, as every arena payload's is.
pub const OBSERVER_SCHEMA: &str = "arena.observer";

/// The payload's version. Additive changes keep it.
pub const OBSERVER_VERSION: u32 = 1;

/// **One unit's flags**, in the payload's fixed order: `w` wreck, `p` power
/// online, `s` starting, `m` machine, `h` held; `""` when none apply.
/// **17b cut M4.3**: one attribute, one flag — `m` for every hull a machine
/// flies (`Controller::is_machine`: a mind — or, in a fixture until 17b cut
/// M4.6 deleted it, the reflex controller); M1a.5's `n` is retired, which the renderers never knew
/// (`web/src/lib/replay.ts`, the TUI's replay), so a mind read as a pilot's
/// hull until this cut. Additive, so the version stays.
#[must_use]
pub fn unit_flags(unit: &ObserverUnit) -> String {
    let mut flags = String::with_capacity(5);
    if unit.wreck {
        flags.push('w');
    }
    match unit.power {
        Power::Online => flags.push('p'),
        Power::Starting => flags.push('s'),
        Power::Off => {}
    }
    if unit.controller.is_machine() {
        flags.push('m');
    }
    if unit.held {
        flags.push('h');
    }
    flags
}

/// **The `arena.observer` keyframe** (16b cut I.4): one staff picture of match
/// `match_id` on `ground`, with the staff tempo floor the world last delivered
/// to the task (`MatchMirror::tempo_floor_ms`), or `None`.
///
/// `ground` is the match's own terrain, which the registry holds for the life
/// of the match (`ArenaRegistry::arena_of`); only its size is read.
#[must_use]
pub fn observer_frame(
    match_id: MatchId,
    observer: &Observer,
    floor_ms: Option<u32>,
    ground: &ArenaTerrain,
) -> Value {
    let units: Vec<Value> = observer
        .units
        .iter()
        .map(|unit| {
            serde_json::json!([
                unit.callsign.to_string(),
                unit.team.letter().to_string(),
                unit.cell.map(|(x, _)| x),
                unit.cell.map(|(_, y)| y),
                super::replay::whole_degrees(unit.heading_mdeg),
                unit.unit.0,
                unit_flags(unit),
            ])
        })
        .collect();
    serde_json::json!({
        "schema": {
            "id": OBSERVER_SCHEMA,
            "version": OBSERVER_VERSION,
            "retain": super::cockpit::RETAIN_LATEST,
        },
        "match": match_id.0,
        "now_ms": observer.now_ms,
        "clock": super::lobby::clock_text(observer.now_ms),
        "tempo": observer.tempo.as_str(),
        "floor_ms": floor_ms,
        "cell_cm": ground.cell_cm(),
        "width": ground.width(),
        "height": ground.height(),
        "in_flight": observer.in_flight,
        "units": units,
    })
}

/// **Nothing to watch** (16b cut I.4): what a panel is sent when its watch
/// ends — `@arena/watch off`, a disconnect, the match's end, a staffer who lost
/// the power — and what the `arena.observer` query answers anybody who is not
/// watching. A client that holds a live picture closes it and marks the match
/// dismissed. It carries the same `retain` marker every picture does, so it
/// replaces the last picture in a session's per-feed latest slot.
#[must_use]
pub fn observer_cleared() -> Value {
    serde_json::json!({
        "schema": {
            "id": OBSERVER_SCHEMA,
            "version": OBSERVER_VERSION,
            "retain": super::cockpit::RETAIN_LATEST,
        },
        "watching": false,
    })
}

#[cfg(test)]
mod tests {
    use mush_core::ObjId;

    use super::*;
    use crate::arena::clock::Tempo;
    use crate::arena::state::Controller;
    use crate::arena::state::{Designator, PartyId, TerrainCell};

    fn ground() -> ArenaTerrain {
        ArenaTerrain::uniform(
            256,
            256,
            1_000,
            TerrainCell {
                class: mush_atlas::TerrainClass::Ground,
                elev_cm: 0,
                move_cost: 100,
            },
        )
        .expect("the shipped window's size")
    }

    fn row(unit: u32, team: PartyId, slot: u16, cell: Option<(u16, u16)>) -> ObserverUnit {
        ObserverUnit {
            unit: ObjId(unit),
            callsign: Designator::Unit {
                party: team,
                letter: char::from(b'A'.saturating_add(u8::try_from(slot).unwrap_or(1).saturating_sub(1))),
            },
            team,
            cell,
            heading_mdeg: 90_400,
            speed_cms: 0,
            power: Power::Off,
            wreck: false,
            held: false,
            controller: Controller::Pilot,
        }
    }

    /// **16b cut I.4**: every key of the payload, the rows positional, a null
    /// cell for a unit off the window, and the floor as the mirror has it.
    #[test]
    fn i4_the_picture_carries_every_key_and_a_row_per_unit() {
        let mut red = row(301, PartyId::RED, 1, Some((120, 97)));
        red.power = Power::Online;
        // 17b cut M4.5a: the machine is a mind (one machine flag, `m`).
        red.controller = Controller::Mind;
        let mut blue = row(302, PartyId::BLUE, 1, None);
        blue.heading_mdeg = 270_000;
        blue.wreck = true;
        let picture = Observer {
            now_ms: 90_000,
            tempo: Tempo::Engaged,
            in_flight: 2,
            units: vec![red, blue],
        };
        let data = observer_frame(MatchId(12), &picture, None, &ground());
        assert_eq!(
            data,
            serde_json::json!({
                "schema": { "id": "arena.observer", "version": 1, "retain": "latest" },
                "match": 12,
                "now_ms": 90_000,
                "clock": "1:30",
                "tempo": "engaged",
                "floor_ms": null,
                "cell_cm": 1_000,
                "width": 256,
                "height": 256,
                "in_flight": 2,
                "units": [
                    ["RA", "R", 120, 97, 90, 301, "pm"],
                    ["BA", "B", null, null, 270, 302, "w"],
                ],
            })
        );
        let floored = observer_frame(MatchId(12), &picture, Some(5_000), &ground());
        assert_eq!(floored["floor_ms"], serde_json::json!(5_000));
        assert!(data.get("shape").is_none() && data["schema"].get("shape").is_none());
    }

    /// **16b cut I.4**: the flags are `w p s m h`, in that order, and `""` when
    /// none apply; power online and starting never both. **17b cut M4.3**: a
    /// mind is `m` — one machine flag (17b cut M4.5a: the machine here is the
    /// mind, where it was the reflex controller).
    #[test]
    fn i4_the_flags_are_in_their_fixed_order() {
        let mut unit = row(1, PartyId::RED, 1, Some((0, 0)));
        assert_eq!(unit_flags(&unit), "");
        unit.held = true;
        assert_eq!(unit_flags(&unit), "h");
        unit.controller = Controller::Mind;
        assert_eq!(unit_flags(&unit), "mh");
        unit.power = Power::Starting;
        assert_eq!(unit_flags(&unit), "smh");
        unit.power = Power::Online;
        unit.wreck = true;
        assert_eq!(unit_flags(&unit), "wpmh");
    }

    /// **16b cut I.4**: nothing to watch is the schema and one key.
    #[test]
    fn i4_the_cleared_picture_is_watching_false() {
        assert_eq!(
            observer_cleared(),
            serde_json::json!({
                "schema": { "id": "arena.observer", "version": 1, "retain": "latest" },
                "watching": false,
            })
        );
    }

    /// **16b cut I.4**: a 300-unit picture at its widest — three-letter
    /// callsigns, three-digit cells, six-digit object numbers, four flags a
    /// unit, a ten-second floor — serialises under the 16,384-byte frame
    /// bound. The figure is printed.
    #[test]
    fn i4_a_three_hundred_unit_picture_fits_the_frame_bound() {
        let units: Vec<ObserverUnit> = (0..300_u16)
            .map(|n| {
                let team = if n % 2 == 0 { PartyId::RED } else { PartyId::BLUE };
                let mut unit = row(
                    900_000 + u32::from(n),
                    team,
                    // Past 26 a side: `RAA` and beyond.
                    27 + n / 2,
                    Some((200 + n % 56, 200 + n % 56)),
                );
                unit.heading_mdeg = 359_999;
                unit.wreck = true;
                unit.power = Power::Online;
                unit.controller = Controller::Mind;
                unit.held = true;
                unit
            })
            .collect();
        let picture = Observer {
            now_ms: 3_599_999,
            tempo: Tempo::Engaged,
            in_flight: 9_999,
            units,
        };
        let bytes = serde_json::to_vec(&observer_frame(
            MatchId(u64::from(u32::MAX)),
            &picture,
            Some(10_000),
            &ground(),
        ))
        .expect("the picture serialises")
        .len();
        println!("a 300-unit arena.observer keyframe is {bytes} bytes");
        assert!(bytes < 16_384, "{bytes} bytes");
    }
}
