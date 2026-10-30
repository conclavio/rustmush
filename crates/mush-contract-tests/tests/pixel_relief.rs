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

/// Doc 30 §7 (D-30-6, cut WG.2): a flat field draws flat under every biome,
/// and no `mush-pixel` path writes an `elev_cm` the feed did not carry.
#[test]
fn a_flat_field_draws_flat_under_every_biome() {
    use mush_pixel::Frame;
    use mush_pixel::raycast::Camera;

    let (w, h) = (32u16, 32u16);
    let cells: Vec<TerrainCell> = (0..usize::from(w) * usize::from(h))
        .map(|_| TerrainCell {
            class: TerrainClass::Grass,
            elev_cm: 0,
            move_cost: 100,
        })
        .collect();
    let sim_terrain = ArenaTerrain::new(w, h, 1000, cells).expect("flat terrain");
    let base_frame =
        tacsim::match_::cockpit::map_frame(MatchId(1), "flat_field", &sim_terrain, &[]);

    for biome in ["temperate", "mars", "lunar", "ice"] {
        let mut frame = base_frame.clone();
        frame["biome"] = serde_json::json!(biome);
        frame["procedural_relief"] = serde_json::json!(true);

        let picture =
            mush_pixel::cockpit::terrain_from_feed(&frame).expect("all-zero elev decodes");
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    picture.elev_cm(i64::from(x), i64::from(y)),
                    0,
                    "{biome}: cell ({x}, {y}) must remain flat (0 cm)"
                );
            }
        }

        // First-person skyline across a flat field is a single horizontal row
        // (every column meets the horizon haze at row `h / 2 == 110`).
        let mut fp = Frame::new(320, 220);
        let cam = Camera {
            x_cm: 16_000,
            y_cm: 16_000,
            heading_mdeg: 0,
        };
        mush_pixel::heightfield::render(&mut fp, &picture, &cam, None, &[]);
        for x in (0..320).step_by(16) {
            assert_eq!(
                fp.get(x, 108),
                fp.get(0, 108),
                "{biome}: sky row 108 is uniform across columns"
            );
        }

        // Without an `elev` field on the wire, `terrain_from_feed` returns `None`
        // regardless of biome or legacy flags.
        let mut no_elev = frame;
        no_elev.as_object_mut().expect("object").remove("elev");
        assert!(
            mush_pixel::cockpit::terrain_from_feed(&no_elev).is_none(),
            "{biome}: missing `elev` must not synthesize relief"
        );
    }
}

/// Doc 30 §7 (D-30-10, cut WG.2): `rock` is a surface material with zero
/// height extrusion (`obstacle_height_cm(Rock) == 0`), so a `rock` cell at
/// `elev_cm == 0` blocks no sight line in any picture (raycaster, heightfield,
/// overhead, or 2.5D).
#[test]
fn a_rock_cell_at_elev_cm_0_blocks_no_sight_line_in_any_picture() {
    use mush_pixel::Frame;
    use mush_pixel::iso25d::Iso25DPicture;
    use mush_pixel::raycast::{Camera, Ground, Sprite, SpriteKind};

    assert_eq!(
        mush_atlas::physics::obstacle_height_cm(TerrainClass::Rock),
        0
    );
    assert_eq!(Ground::from_legend_name("rock"), Ground::Open);
    assert_eq!(Ground::from_legend_name("rock").height_cm(), 0);
    assert!(!Ground::from_legend_name("rock").blocks());

    let (w, h) = (32u16, 32u16);
    let build_with_barrier = |barrier: TerrainClass| -> serde_json::Value {
        let cells: Vec<TerrainCell> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(_x, y)| TerrainCell {
                class: if y == 14 {
                    barrier
                } else {
                    TerrainClass::Ground
                },
                elev_cm: 0,
                move_cost: 100,
            })
            .collect();
        let terrain = ArenaTerrain::new(w, h, 1000, cells).expect("terrain");
        tacsim::match_::cockpit::map_frame(MatchId(2), "rock_sight", &terrain, &[])
    };

    let open_feed = build_with_barrier(TerrainClass::Ground);
    let rock_feed = build_with_barrier(TerrainClass::Rock);
    let wall_feed = build_with_barrier(TerrainClass::Wall);

    let cam = Camera {
        x_cm: 16_500,
        y_cm: 8_500,
        heading_mdeg: 0,
    };
    let target = Sprite {
        x_cm: 16_500,
        y_cm: 20_500,
        kind: SpriteKind::Red,
        pose: None,
    };

    // 1. Raycaster: rock row at y=14 produces the exact same frame as open ground,
    //    whereas a wall row at y=14 occludes the target behind a 3 m wall.
    let open_map = mush_pixel::cockpit::map_from_feed(&open_feed);
    let rock_map = mush_pixel::cockpit::map_from_feed(&rock_feed);
    let wall_map = mush_pixel::cockpit::map_from_feed(&wall_feed);

    let mut rc_open = Frame::new(320, 220);
    let mut rc_rock = Frame::new(320, 220);
    let mut rc_wall = Frame::new(320, 220);
    mush_pixel::raycast::render(&mut rc_open, &open_map, &cam, &[target]);
    mush_pixel::raycast::render(&mut rc_rock, &rock_map, &cam, &[target]);
    mush_pixel::raycast::render(&mut rc_wall, &wall_map, &cam, &[target]);
    assert_eq!(
        rc_rock.hash(),
        rc_open.hash(),
        "raycaster: rock == open at elev 0"
    );
    assert_ne!(
        rc_rock.hash(),
        rc_wall.hash(),
        "raycaster: wall blocks, rock does not"
    );

    // 2. Heightfield: rock row at y=14 produces the exact same frame as open ground.
    let open_hf = mush_pixel::cockpit::terrain_from_feed(&open_feed).expect("open hf");
    let rock_hf = mush_pixel::cockpit::terrain_from_feed(&rock_feed).expect("rock hf");
    let wall_hf = mush_pixel::cockpit::terrain_from_feed(&wall_feed).expect("wall hf");

    let mut hf_open = Frame::new(320, 220);
    let mut hf_rock = Frame::new(320, 220);
    let mut hf_wall = Frame::new(320, 220);
    mush_pixel::heightfield::render(&mut hf_open, &open_hf, &cam, None, &[target]);
    mush_pixel::heightfield::render(&mut hf_rock, &rock_hf, &cam, None, &[target]);
    mush_pixel::heightfield::render(&mut hf_wall, &wall_hf, &cam, None, &[target]);
    assert_eq!(
        hf_rock.hash(),
        hf_open.hash(),
        "heightfield: rock == open at elev 0"
    );
    assert_ne!(
        hf_rock.hash(),
        hf_wall.hash(),
        "heightfield: wall blocks, rock does not"
    );

    // 3. 2.5D picture: rock row at y=14 produces the exact same frame as open ground.
    let cockpit = serde_json::json!({
        "seated": true,
        "team": "blue",
        "unit_type": "tacsim.unit.biped_mk1",
        "x_cm": 16_500,
        "y_cm": 8_500,
        "z_cm": 0,
        "heading_mdeg": 0,
        "speed_cms": 0,
        "contacts": [{
            "id": "c1",
            "designator": "RA",
            "class": "red",
            "unit_type": "tacsim.unit.line_tank",
            "bearing_deg": 0.0,
            "range_m": 60.0,
            "facing": "front",
            "moving": false
        }]
    });
    let mut iso_open = Iso25DPicture::new();
    iso_open.set_map(open_map);
    let f_iso_open = iso_open.render_frame(&cockpit, 0, 0, 320, 220);

    let mut iso_rock = Iso25DPicture::new();
    iso_rock.set_map(rock_map);
    let f_iso_rock = iso_rock.render_frame(&cockpit, 0, 0, 320, 220);

    let mut iso_wall = Iso25DPicture::new();
    iso_wall.set_map(wall_map);
    let f_iso_wall = iso_wall.render_frame(&cockpit, 0, 0, 320, 220);

    assert_eq!(
        f_iso_rock.hash(),
        f_iso_open.hash(),
        "iso25d: rock == open at elev 0"
    );
    assert_ne!(
        f_iso_rock.hash(),
        f_iso_wall.hash(),
        "iso25d: wall extrudes, rock does not"
    );
}
