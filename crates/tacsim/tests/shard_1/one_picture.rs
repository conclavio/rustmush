//! **Doc 32 cut CF.4 — one picture**, over the composed world (`driving`'s keyed rig on Ahuna
//! Flank) with the arena's egress pumped by hand, as CF.2's gate pumps it (`field_host`).
//!
//! The cut's gate: Q-CF-48's and Q-CF-49's statements hold over the one frame family the three
//! clients draw. One site map, two layers: on the open surface `+tv/map` draws the campaign layer —
//! the salvage in sight, the other flights as bands beyond sight and as `o` within it; inside a
//! field the cockpit frame carries the same campaign layer as its `campaign` block, the ground
//! objects its unit's sensors hold as `ground` rows, and a rig's cockpit the quarry screen's rig
//! line as its `rig` panel; `site.map` draws the field layer for a seated viewer and says why not
//! for an unseated one; `+tv` status keeps answering for the rig. A ground object is an obstacle
//! that hides what is behind it on the field's ground. After demotion each party remembers what
//! it saw there (Q-CF-28) and a character whose units never had it in sight does not. The frames' new
//! keys are the shape `web/src/lib/fixtures/cockpit/field_frame.json` pins for the TUI's, the
//! web's and the pixel cockpit's own tests.

use mush_core::ObjId;
use mush_proto::OutputEvent;
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::surface::site_record::{KnownItem, KnownWhat};

use crate::driving::{
    Rig, command, launched, map_frame, on_pad, on_pad_as, run, stocked_tuned, to_cell, warm,
};
use crate::field_host::{
    armed, egress, fragile, frame, pump, seat, second, site_record, task, tune, until, weapon_of,
};
use crate::site_record::{staff, staff_member};

/// **The `arena.cockpit` keyframe `who` gets** through the feed's own query — the same payload a
/// resyncing client is answered with, through the context the field's picture needs.
fn cockpit(rig: &mut Rig, who: ObjId) -> Value {
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    rig.tw.engine_mut().step(
        mush_core::Inbound::Query {
            conn: 7,
            actor: who,
            id: 1,
            name: SmolStr::new(tacsim::arena_bridge::FRAME_FEED),
            args: json!({}),
        },
        &mut out,
        &mut fx,
    );
    out.events()
        .iter()
        .find_map(|(_, event)| match event {
            OutputEvent::Reply { event, .. } => match event.as_ref() {
                OutputEvent::Panel { data, .. } => Some(data.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("a cockpit frame")
}

/// A site-view frame's glyph at the frame cell of site cell `cell` (80 m cells: eight a side).
fn site_glyph(frame: &Value, (x, y): (i64, i64)) -> Option<char> {
    let width = usize::try_from(frame["width"].as_u64()?).ok()?;
    let height = usize::try_from(frame["height"].as_u64()?).ok()?;
    let rows = frame["rows"].as_array()?;
    let grid = mush_ansi::panel::window_rows(rows, width, height, 0, 0, width, height);
    let (fx, fy) = (usize::try_from(x / 8).ok()?, usize::try_from(y / 8).ok()?);
    grid.get(fy)?.get(fx).copied()
}

/// The keys of a JSON object, sorted.
fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    keys
}

/// **The shared fixture's shape holds for the real frame** — every key of the fixture's `campaign`
/// block, its first `ground` row and its `rig` panel is a key of the frame's, and the other way.
fn same_shape(real: &Value, fixture: &Value, key: &str) {
    let (a, b) = match key {
        "ground" => (&real[key][0], &fixture[key][0]),
        _ => (&real[key], &fixture[key]),
    };
    assert_eq!(
        keys(a),
        keys(b),
        "`{key}` in field_frame.json is the server's shape"
    );
}

/// **CF.4's gate** (doc 32 §6; Q-CF-26, Q-CF-28, Q-CF-48, Q-CF-49, Q-CF-50).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf4_one_picture_of_the_site_in_both_layers_and_what_a_party_remembers() {
    // CF.2's fast field, with a longer quiet interval so the checks below run inside the field.
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.disengage_s = 30;
    });
    // A and B fly armed hulls; C's prospector works beside them and C sits in its cab; D's rig is
    // out far to the east; E stays aboard with a rig in the bay. Every carrier on the pad is in
    // the field too (CF.3), out of sight of the crate.
    let (a, _, _) = on_pad(&mut rig, 1);
    let (b, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let (c, _, c_tvs) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    let (d, _, d_tvs) = on_pad_as(&mut rig, "Fourth", 4, "Far Moth", 1);
    let (e, _, _) = on_pad_as(&mut rig, "Fifth", 5, "Still Moth", 1);
    launched(&mut rig, c, c_tvs[0]);
    to_cell(&mut rig, c_tvs[0], (116, 116));
    launched(&mut rig, d, d_tvs[0]);
    to_cell(&mut rig, d_tvs[0], (160, 116));
    let hull_a = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    let hull_b = armed(&mut rig, "Grey Hound", (114, 124), 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    seat(&mut rig, c_tvs[0], c);
    fragile(&mut rig, hull_b);
    // The ground: a crate beside the fight, another far beyond anyone's sight.
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        "@tacsim/site ahuna_port drop crate 2 metal_ore 98,116",
    );
    staff(
        &mut rig,
        s,
        "@tacsim/site ahuna_port drop crate 1 ice 220,40",
    );
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // (1) The open surface, C's map: the crate in sight is drawn and named; D's rig, beyond
    // sight, is a band and never an `o` or a distance (Q-CF-48's presence picture).
    let surface = map_frame(&mut rig, c, json!({ "view": "site" }));
    let words = surface["site"]["lines"].to_string();
    assert_eq!(site_glyph(&surface, (98, 116)), Some('&'), "{words}");
    assert!(words.contains("Also on this map: & crate."), "{words}");
    assert!(words.contains("a crate of 2 metal_ore"), "{words}");
    assert!(words.contains("Presence: E 200-500 m"), "{words}");
    assert!(
        !words.contains("Other rigs:"),
        "a flight beyond sight is a band, never a distance: {words}"
    );
    assert_eq!(surface["site"]["others"], json!([]));
    assert!(
        surface["units"]
            .as_array()
            .is_some_and(|units| units.iter().all(|unit| unit["glyph"] != "o")),
        "no exact cell for a flight beyond sight"
    );

    // (2) A's fire order promotes the site; everyone on the ground is admitted.
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving::land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let field = task(&rig).expect("the field's task");
    // Q-CF-26: the crate's footprint is an obstacle on the field's ground, hiding what is behind.
    let class = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.arena_of(field))
        .and_then(|ground| ground.terrain.get(98, 116).map(|cell| cell.class));
    assert_eq!(class, Some(mush_atlas::TerrainClass::Wall));

    // (3) A's cockpit carries the campaign layer and holds the crate as a ground return.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
            cockpit(rig, a).get("campaign").is_some()
        })
        .await,
        "A's cockpit frame carries the campaign layer"
    );
    let frame_a = cockpit(&mut rig, a);
    let campaign = &frame_a["campaign"];
    assert_eq!(campaign["field"], json!(1), "{campaign}");
    assert_eq!(campaign["site"], json!("Ahuna Flank"));
    let marks = campaign["marks"].as_array().expect("marks");
    assert!(
        marks.iter().any(|mark| mark == &json!([98, 116, "&"])),
        "the crate on the campaign layer: {campaign}"
    );
    assert!(
        marks.iter().any(|mark| mark[2]
            .as_str()
            .is_some_and(|g| g.len() == 1 && g.chars().all(|c| c.is_ascii_uppercase()))),
        "the blocks' letters: {campaign}"
    );
    assert!(
        campaign["legend"]
            .as_array()
            .is_some_and(|pairs| pairs.contains(&json!(["&", "crate"]))),
        "{campaign}"
    );
    let ground = frame_a["ground"]
        .as_array()
        .expect("A holds a ground return");
    assert_eq!(ground.len(), 1, "the far crate is beyond sight: {ground:?}");
    assert_eq!(ground[0]["what"], json!("crate"));
    assert_eq!(ground[0]["cell"], json!([98, 116]));
    assert!(
        frame_a.get("rig").is_none(),
        "an armed hull has no rig panel"
    );

    // (4) C's cab is a cockpit: no weapons, its contacts, the quarry screen's rig line (Q-CF-49).
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
            cockpit(rig, c).get("rig").is_some()
        })
        .await,
        "C's rig cockpit carries its rig panel"
    );
    let frame_c = cockpit(&mut rig, c);
    let line = frame_c["rig"]["line"].as_str().expect("the rig line");
    assert!(line.starts_with("Rig 1: (116, 116), charge "), "{line}");
    assert!(line.contains("hopper"), "{line}");
    assert!(
        frame_c["weapons"]
            .as_array()
            .is_none_or(std::vec::Vec::is_empty),
        "a rig has no weapons: {}",
        frame_c["weapons"]
    );
    // `+tv` status keeps answering for the rig inside the field.
    let status = run(&mut rig.tw, c, "+tv");
    assert!(status.contains("1"), "{status}");
    assert!(!status.contains("Huh?"), "{status}");

    // (5) `site.map` on the promoted site: A's field layer drawn from A's own cockpit frame; D,
    // with a rig in the field and no seat, holds nothing and is told so.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
            map_frame(rig, a, json!({ "view": "site" }))["site"]["field_layer"]["held"]
                .as_u64()
                .is_some_and(|held| held > 0)
        })
        .await,
        "A's site map draws the field layer"
    );
    let field_map = map_frame(&mut rig, a, json!({ "view": "site" }));
    let layer = &field_map["site"]["field_layer"];
    assert!(layer["contacts"].as_array().is_some_and(|c| !c.is_empty()));
    assert!(
        field_map["site"]["field"]
            .as_str()
            .is_some_and(|line| line.contains("Your sensors hold")),
        "{}",
        field_map["site"]["field"]
    );
    let unit_glyph = field_map["units"]
        .as_array()
        .and_then(|units| {
            units
                .iter()
                .find(|unit| unit["id"] == layer["unit"])
                .map(|unit| unit["glyph"].clone())
        })
        .unwrap_or(Value::Null);
    assert_eq!(unit_glyph, json!("@"), "A's hull drawn as the viewer's");
    let unseated = run(&mut rig.tw, d, "+tv/map");
    assert!(
        unseated.contains("You hold no contacts there"),
        "{unseated}"
    );

    // The shared fixture's keys are the server's.
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../web/src/lib/fixtures/cockpit/field_frame.json"
    ))
    .expect("field_frame.json");
    same_shape(&frame_a, &fixture, "campaign");
    same_shape(&frame_a, &fixture, "ground");
    same_shape(&frame_c, &fixture, "rig");

    // (6) A fires on B's hull until it is a wreck — two hostile parties parked in each other's
    // weapons range never demote (doc 32 §7) — and then nobody fires any more: the field demotes,
    // and each party remembers what it saw.
    let gun = weapon_of(&rig, hull_a);
    let wrecked = until(&mut rig, &mut rx, &mut lines, 150, |rig, _| {
        let target = frame(rig, a).and_then(|held| {
            held.contacts
                .iter()
                .find(|contact| {
                    contact
                        .cell
                        .is_some_and(|(x, y)| x.abs_diff(114) <= 2 && y.abs_diff(124) <= 2)
                })
                .map(|contact| (contact.id, contact.designator.to_string(), held.lock))
        });
        if let Some((id, designator, lock)) = target
            && lock != Some(id)
        {
            let _ = run(&mut rig.tw, a, &format!("LOCK {designator}"));
        }
        let _ = run(&mut rig.tw, a, &format!("FIRE {gun}"));
        frame(rig, b).is_some_and(|held| held.wreck)
    })
    .await;
    assert!(wrecked, "B's hull is wrecked by A's fire");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 90, |rig, _| {
            task(rig).is_none()
                && site_record(rig).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes after its quiet interval"
    );
    pump(&mut rig, &mut rx, &mut lines);
    let record = site_record(&mut rig).expect("the record");
    let crate_at = KnownItem(KnownWhat::Crate, 98, 116);
    assert!(record.known_by(a).contains(&crate_at), "{:?}", record.known);
    assert!(record.known_by(c).contains(&crate_at), "{:?}", record.known);
    assert!(
        !record.known_by(d).contains(&crate_at),
        "D's rig never had the crate in sight"
    );
    assert!(
        !record.known_by(e).contains(&crate_at),
        "E's carrier on the pad never had the crate in sight"
    );
    // B's wreck, where it fell: its own party knows it, and so does A, who saw it fall.
    let fell = record
        .units
        .iter()
        .find(|left| left.unit == hull_b)
        .map(|left| KnownItem(KnownWhat::Wreck, left.pose.cell().0, left.pose.cell().1))
        .expect("B's hull is left on the ground");
    assert!(record.known_by(b).contains(&fell), "{:?}", record.known);
    assert!(record.known_by(a).contains(&fell), "{:?}", record.known);
    assert!(
        record
            .known
            .iter()
            .all(|row| !row.items.contains(&KnownItem(KnownWhat::Crate, 220, 40))),
        "nobody saw the far crate"
    );
    // A's map, aboard with no rig out: the crate remembered, drawn and marked so; E's map shows
    // nothing there.
    let after = map_frame(&mut rig, a, json!({ "view": "site" }));
    let words = after["site"]["lines"].to_string();
    assert_eq!(site_glyph(&after, (98, 116)), Some('&'), "{words}");
    assert!(words.contains("a crate"), "{words}");
    assert!(words.contains("(remembered)"), "{words}");
    let never = map_frame(&mut rig, e, json!({ "view": "site" }));
    assert_ne!(site_glyph(&never, (98, 116)), Some('&'));
    assert!(!never["site"]["lines"].to_string().contains("a crate"));
}
