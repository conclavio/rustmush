//! The `arena.cockpit` keyframe as the instruments' inputs: the one place
//! that reads the frame's JSON for the picture, shared by the SSH TUI's
//! view and the fixture example.

use serde_json::Value;

use crate::Frame;
use crate::overlay::{self, Facet, Heat, MapMark, Section, Spot, Status, WeaponRow};
use crate::raycast::{Camera, Map, Sprite, SpriteKind};

/// How a contact is drawn: wreck, friend, classified hostile, or unknown.
#[must_use]
pub fn contact_kind(c: &Value) -> SpriteKind {
    if c.get("reads_dead").and_then(Value::as_bool) == Some(true) {
        SpriteKind::Wreck
    } else if c.get("friendly").and_then(Value::as_bool) == Some(true) {
        SpriteKind::Friendly
    } else if c
        .get("class")
        .and_then(Value::as_str)
        .is_some_and(|s| s != "?" && !s.is_empty() && s != "unknown")
    {
        SpriteKind::Hostile
    } else {
        SpriteKind::Unknown
    }
}

/// The frame's contacts as sprites, placed by bearing and range from `cam`.
#[must_use]
pub fn contacts_as_sprites(cockpit: &Value, cam: &Camera) -> Vec<Sprite> {
    cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            Sprite::at_bearing(
                cam,
                c.get("bearing_deg").and_then(Value::as_f64).unwrap_or(0.0),
                c.get("range_m").and_then(Value::as_f64).unwrap_or(0.0),
                contact_kind(c),
            )
        })
        .collect()
}

fn num(v: Option<&Value>) -> i64 {
    v.and_then(Value::as_i64).unwrap_or(0)
}

fn cell_of(v: Option<&Value>) -> Option<(i64, i64)> {
    let pair = v?.as_array()?;
    Some((pair.first()?.as_i64()?, pair.get(1)?.as_i64()?))
}

/// Draw the instruments over a rendered frame from the cockpit keyframe and
/// the held `arena.map` (for its named spots): minimap, heat, weapons, the
/// armour doll and the status strip.
pub fn draw_instruments(
    frame: &mut Frame,
    arena: &Map,
    cockpit: &Value,
    map: Option<&Value>,
    cam: &Camera,
) {
    // The minimap.
    let marks: Vec<MapMark> = cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(MapMark {
                cell: cell_of(c.get("cell"))?,
                kind: contact_kind(c),
            })
        })
        .collect();
    let spots: Vec<Spot> = map
        .and_then(|m| m.get("spots"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|spot| {
            Some(Spot {
                cell: cell_of(spot.get("cell"))?,
                letter: spot.get("letter")?.as_str()?.chars().next()?,
            })
        })
        .collect();
    let cell = i64::from(arena.cell_cm().max(1));
    let you = cell_of(cockpit.get("cell")).unwrap_or((cam.x_cm / cell, cam.y_cm / cell));
    overlay::draw_minimap(frame, arena, you, cam.heading_mdeg, &marks, &spots);

    // Heat.
    if let Some(heat) = cockpit.get("heat").filter(|h| !h.is_null()) {
        overlay::draw_heat(
            frame,
            &Heat {
                heat: num(heat.get("heat")),
                max: num(heat.get("max")),
                overheated: heat.get("overheated").and_then(Value::as_bool) == Some(true),
            },
        );
    }

    // Weapons.
    let rows: Vec<WeaponRow> = cockpit
        .get("weapons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|w| WeaponRow {
            name: w.get("id").and_then(Value::as_str).unwrap_or("?").to_string(),
            ready: w.get("ready").and_then(Value::as_bool) == Some(true),
            ready_in_ms: num(w.get("ready_in_ms")),
            rounds: num(w.get("rounds")),
            rounds_max: num(w.get("rounds_max")),
            percent: w.get("percent").and_then(Value::as_u64),
        })
        .collect();
    overlay::draw_weapons(frame, &rows);

    // The doll.
    let sections: Vec<Section> = cockpit
        .get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|sec| Section {
            id: sec.get("id").and_then(Value::as_str).unwrap_or("?").to_string(),
            condition: sec
                .get("condition")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            internals: num(sec.get("internals")),
            internals_max: num(sec.get("internals_max")),
            facets: sec
                .get("facets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|f| Facet {
                    id: f.get("id").and_then(Value::as_str).unwrap_or("?").to_string(),
                    armor: num(f.get("armor")),
                    armor_max: num(f.get("armor_max")),
                })
                .collect(),
        })
        .collect();
    overlay::draw_doll(frame, &sections);

    // The status strip.
    let lock = cockpit
        .get("lock_designator")
        .and_then(Value::as_str)
        .map(|who| (who.to_string(), num(cockpit.get("lock_range_m"))));
    overlay::draw_status(
        frame,
        &Status {
            callsign: cockpit
                .get("callsign")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            speed_kph: num(cockpit.get("speed_kph")),
            heading_deg: i64::from(cam.heading_mdeg / 1_000),
            lock,
            power: cockpit
                .get("power")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raycast::{Ground, render};

    const COCKPIT: &str = include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json");

    #[test]
    fn the_fixture_frame_draws_every_instrument_inside_the_palette() {
        let parsed = serde_json::from_str::<Value>(COCKPIT);
        assert!(parsed.is_ok(), "fixture: {parsed:?}");
        let cockpit = parsed.unwrap_or_default();
        let mut arena = Map::new(96, 64, 1000);
        for y in 0..64 {
            for x in 0..96 {
                arena.set(x, y, Ground::Grass);
            }
        }
        let cam = Camera { x_cm: 6_500, y_cm: 4_500, heading_mdeg: 45_000 };
        let sprites = contacts_as_sprites(&cockpit, &cam);
        assert_eq!(sprites.len(), 4);
        assert_eq!(sprites.iter().filter(|s| s.kind == SpriteKind::Hostile).count(), 1, "BA alone is classified");
        assert_eq!(sprites.iter().filter(|s| s.kind == SpriteKind::Unknown).count(), 2, "X3 and X4");
        let mut frame = Frame::new(640, 480);
        render(&mut frame, &arena, &cam, &sprites);
        draw_instruments(&mut frame, &arena, &cockpit, None, &cam);
        assert!(frame.count_colours(256) <= 256);
    }
}
