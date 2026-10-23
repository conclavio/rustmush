//! **The picture's relief is the sim's** (doc 21 cut G.7b).
//!
//! The arena's cells carry an elevation the sim moves, sights and shoots
//! over; `arena.map` carries every one of them (`elev`, zigzag-LEB128 deltas
//! in base64, tacsim's `elev_field`), and the picture's heightfield is built
//! from that (`mush_pixel::cockpit::terrain_from_feed`). `mush-pixel` cannot
//! depend on tacsim (it compiles to WebAssembly), so this test is what keeps
//! the two ends of the wire the same: every cell's height comes back exactly.

use mush_atlas::TerrainClass;
use tacsim::arena::state::{ArenaTerrain, KeySpot, TerrainCell};
use tacsim::arena_task::MatchId;

#[test]
fn every_cell_height_reaches_the_picture_exactly() {
    let (w, h) = (37u16, 23u16);
    // Rolling ground, a cliff, a pit below the datum and flat runs: every
    // varint width the encoding writes.
    let elev = |x: u16, y: u16| -> i16 {
        let (x, y) = (i32::from(x), i32::from(y));
        let roll = (x * 37 + y * 11) % 200 - 100;
        let cliff = if x > 20 { 3_000 } else { 0 };
        let pit = if (5..9).contains(&x) && (5..9).contains(&y) {
            -12_000
        } else {
            0
        };
        let flat = if y > 18 { 0 } else { roll };
        i16::try_from(flat + cliff + pit).unwrap_or(0)
    };
    let cells: Vec<TerrainCell> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| TerrainCell {
            class: if x % 7 == 0 {
                TerrainClass::Road
            } else {
                TerrainClass::Grass
            },
            elev_cm: elev(x, y),
            move_cost: 1,
        })
        .collect();
    let terrain = ArenaTerrain::new(w, h, 1000, cells).expect("a terrain");
    let spots = [KeySpot {
        name: "The Pit".into(),
        x: 6,
        y: 6,
    }];
    let frame = tacsim::match_::cockpit::map_frame(MatchId(9), "relief_test", &terrain, &spots);
    assert_eq!(
        frame["elev"]["encoding"],
        tacsim::match_::cockpit::ELEV_ENCODING
    );
    let picture = mush_pixel::cockpit::terrain_from_feed(&frame).expect("the relief decodes");
    for y in 0..h {
        for x in 0..w {
            assert_eq!(
                picture.elev_cm(i64::from(x), i64::from(y)),
                i32::from(elev(x, y)),
                "cell ({x}, {y})"
            );
        }
    }
    assert!(picture.is_baked(), "the colour map is on");
    // A feed with no heights is drawn flat, as before.
    let mut flat = frame.clone();
    flat.as_object_mut().expect("an object").remove("elev");
    assert!(mush_pixel::cockpit::terrain_from_feed(&flat).is_none());
    // A truncated stream is not a relief.
    let mut short = frame;
    let data = short["elev"]["data"].as_str().unwrap_or("").to_string();
    short["elev"]["data"] = data[..data.len() / 2].into();
    assert!(mush_pixel::cockpit::terrain_from_feed(&short).is_none());
}
