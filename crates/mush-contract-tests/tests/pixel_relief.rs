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

/// Doc 30 §7 (D-30-10, cut WG.2): `rock` is a surface material on the real
/// heightfield — its own surface paint, zero height extrusion
/// (`obstacle_height_cm(Rock) == 0`), and `blocks() == false` — so a pilot
/// reading the ground for a wheeled route can see `rock`, while a `rock` cell
/// at `elev_cm == 0` blocks no sight line (hides no target rows) in any picture.
#[test]
fn a_rock_cell_at_elev_cm_0_blocks_no_sight_line_in_any_picture() {
    use mush_pixel::Frame;
    use mush_pixel::iso25d::Iso25DPicture;
    use mush_pixel::overhead::OverheadPicture;
    use mush_pixel::picture::{Look, Picture};
    use mush_pixel::raycast::{Camera, Ground, Map, Sprite, SpriteKind};
    use std::collections::HashMap;

    assert_eq!(
        mush_atlas::physics::obstacle_height_cm(TerrainClass::Rock),
        0
    );
    assert_eq!(Ground::from_legend_name("rock"), Ground::Rock);
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

    let diff_pixels =
        |with_sprite: &Frame, without_sprite: &Frame| -> Vec<(usize, usize, [u8; 3])> {
            let mut out = Vec::new();
            for y in 0..with_sprite.height() {
                for x in 0..with_sprite.width() {
                    let px = with_sprite.get(x, y);
                    if px != without_sprite.get(x, y) {
                        out.push((x, y, px));
                    }
                }
            }
            out
        };

    // 1. Raycaster: rock row at y=14 paints visibly distinct from bare ground,
    //    while hiding zero rows/pixels of the target sprite behind it (unlike wall).
    let open_map = mush_pixel::cockpit::map_from_feed(&open_feed);
    let rock_map = mush_pixel::cockpit::map_from_feed(&rock_feed);
    let wall_map = mush_pixel::cockpit::map_from_feed(&wall_feed);

    let mut rc_open_bg = Frame::new(320, 220);
    let mut rc_rock_bg = Frame::new(320, 220);
    let mut rc_wall_bg = Frame::new(320, 220);
    mush_pixel::raycast::render(&mut rc_open_bg, &open_map, &cam, &[]);
    mush_pixel::raycast::render(&mut rc_rock_bg, &rock_map, &cam, &[]);
    mush_pixel::raycast::render(&mut rc_wall_bg, &wall_map, &cam, &[]);

    let mut rc_open = Frame::new(320, 220);
    let mut rc_rock = Frame::new(320, 220);
    let mut rc_wall = Frame::new(320, 220);
    mush_pixel::raycast::render(&mut rc_open, &open_map, &cam, &[target]);
    mush_pixel::raycast::render(&mut rc_rock, &rock_map, &cam, &[target]);
    mush_pixel::raycast::render(&mut rc_wall, &wall_map, &cam, &[target]);

    assert_ne!(
        rc_rock_bg.hash(),
        rc_open_bg.hash(),
        "raycaster: rock surface has its own paint distinct from bare ground"
    );
    let rc_open_target = diff_pixels(&rc_open, &rc_open_bg);
    let rc_rock_target = diff_pixels(&rc_rock, &rc_rock_bg);
    let rc_wall_target = diff_pixels(&rc_wall, &rc_wall_bg);
    assert!(!rc_open_target.is_empty(), "raycaster: target is drawn");
    assert_eq!(
        rc_rock_target, rc_open_target,
        "raycaster: rock hides no row of the target sprite"
    );
    assert_ne!(
        rc_rock_target, rc_wall_target,
        "raycaster: wall occludes the target sprite, rock does not"
    );

    // 2. Heightfield: rock row at y=14 paints visibly distinct from bare ground,
    //    while hiding zero rows/pixels of the target sprite behind it (unlike wall).
    let open_hf = mush_pixel::cockpit::terrain_from_feed(&open_feed).expect("open hf");
    let rock_hf = mush_pixel::cockpit::terrain_from_feed(&rock_feed).expect("rock hf");
    let wall_hf = mush_pixel::cockpit::terrain_from_feed(&wall_feed).expect("wall hf");

    let mut hf_open_bg = Frame::new(320, 220);
    let mut hf_rock_bg = Frame::new(320, 220);
    let mut hf_wall_bg = Frame::new(320, 220);
    mush_pixel::heightfield::render(&mut hf_open_bg, &open_hf, &cam, None, &[]);
    mush_pixel::heightfield::render(&mut hf_rock_bg, &rock_hf, &cam, None, &[]);
    mush_pixel::heightfield::render(&mut hf_wall_bg, &wall_hf, &cam, None, &[]);

    let mut hf_open = Frame::new(320, 220);
    let mut hf_rock = Frame::new(320, 220);
    let mut hf_wall = Frame::new(320, 220);
    mush_pixel::heightfield::render(&mut hf_open, &open_hf, &cam, None, &[target]);
    mush_pixel::heightfield::render(&mut hf_rock, &rock_hf, &cam, None, &[target]);
    mush_pixel::heightfield::render(&mut hf_wall, &wall_hf, &cam, None, &[target]);

    assert_ne!(
        hf_rock_bg.hash(),
        hf_open_bg.hash(),
        "heightfield: rock surface has its own paint distinct from bare ground"
    );
    let hf_open_target = diff_pixels(&hf_open, &hf_open_bg);
    let hf_rock_target = diff_pixels(&hf_rock, &hf_rock_bg);
    let hf_wall_target = diff_pixels(&hf_wall, &hf_wall_bg);
    assert!(!hf_open_target.is_empty(), "heightfield: target is drawn");
    assert_eq!(
        hf_rock_target, hf_open_target,
        "heightfield: rock hides no row of the target sprite"
    );
    assert_ne!(
        hf_rock_target, hf_wall_target,
        "heightfield: wall occludes the target sprite, rock does not"
    );

    // 3. 2.5D & Overhead pictures: rock row at y=14 has its own surface paint
    //    and hides zero pixels of a contact standing immediately north of y=14.
    let make_cockpit = |with_contact: bool| -> serde_json::Value {
        let contacts = if with_contact {
            serde_json::json!([{
                "id": "c1",
                "designator": "RA",
                "class": "red",
                "unit_type": "tacsim.unit.line_tank",
                "bearing_deg": 35.0,
                "range_m": 80.0,
                "facing": "front",
                "moving": false
            }])
        } else {
            serde_json::json!([])
        };
        serde_json::json!({
            "seated": true,
            "team": "blue",
            "unit_type": "tacsim.unit.biped_mk1",
            "x_cm": 16_500,
            "y_cm": 8_500,
            "z_cm": 0,
            "heading_mdeg": 0,
            "speed_cms": 0,
            "contacts": contacts
        })
    };

    let render_iso = |map_val: &Map, with_contact: bool| -> Frame {
        let mut pic = Iso25DPicture::new();
        pic.set_map(map_val.clone());
        pic.render_frame(&make_cockpit(with_contact), 0, 0, 320, 220)
    };

    let iso_open_bg = render_iso(&open_map, false);
    let iso_rock_bg = render_iso(&rock_map, false);
    let iso_wall_bg = render_iso(&wall_map, false);
    let iso_open = render_iso(&open_map, true);
    let iso_rock = render_iso(&rock_map, true);
    let iso_wall = render_iso(&wall_map, true);

    assert_ne!(
        iso_rock_bg.hash(),
        iso_open_bg.hash(),
        "iso25d: rock surface has its own paint distinct from bare ground"
    );
    let iso_open_coords: Vec<(usize, usize)> = diff_pixels(&iso_open, &iso_open_bg)
        .into_iter()
        .map(|(x, y, _)| (x, y))
        .collect();
    let iso_rock_coords: Vec<(usize, usize)> = diff_pixels(&iso_rock, &iso_rock_bg)
        .into_iter()
        .map(|(x, y, _)| (x, y))
        .collect();
    let iso_wall_coords: Vec<(usize, usize)> = diff_pixels(&iso_wall, &iso_wall_bg)
        .into_iter()
        .map(|(x, y, _)| (x, y))
        .collect();
    assert!(!iso_open_coords.is_empty(), "iso25d: target is drawn");
    assert_eq!(
        iso_rock_coords, iso_open_coords,
        "iso25d: rock hides no pixel of the target unit"
    );
    assert!(
        iso_wall_coords.len() < iso_rock_coords.len(),
        "iso25d: wall occludes the target unit, rock does not"
    );

    // Overhead also paints rock distinctly from open ground and wall.
    let render_ov = |map_val: &serde_json::Value| -> Frame {
        let mut feeds = HashMap::new();
        feeds.insert("arena.cockpit".to_owned(), make_cockpit(true));
        feeds.insert("arena.map".to_owned(), map_val.clone());
        let mut pic = OverheadPicture::new();
        let mut frame = Frame::new(320, 200);
        assert!(pic.draw(&mut frame, &feeds, &Look::default()));
        frame
    };
    let ov_open = render_ov(&open_feed);
    let ov_rock = render_ov(&rock_feed);
    let ov_wall = render_ov(&wall_feed);
    assert_ne!(ov_rock.hash(), ov_open.hash(), "overhead: rock != open");
    assert_ne!(ov_rock.hash(), ov_wall.hash(), "overhead: rock != wall");
}
