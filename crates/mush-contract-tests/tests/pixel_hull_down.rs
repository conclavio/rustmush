//! **The hull-down check** (doc 22 §6, cut U.6): a contact behind a crest
//! shows rows above it exactly when the sim's ray sees it.
//!
//! The heightfield picture clips a sprite against the ground with its
//! per-pixel depth (doc 22 §5, "hull-down, as built"); the sim decides sight
//! with its own ray over the arena's cells (`tacsim::arena::horizon`, exact to
//! `mush_atlas::sight_only`). `mush-pixel` cannot depend on either, so this
//! test holds the two to each other on doc 22's fixture: a 2 m berm 100 m,
//! 200 m and 500 m out, three cells deep, and a 2.5 m tall contact behind each
//! — once where the line from a 1.5 m eye to the contact's top clears the
//! crest, once well into its shadow. Seen draws at least one row of the
//! contact's body above the crest; unseen draws none. (The berm is three
//! cells deep because the picture's ground is interpolated between cell
//! centres: a one-cell lip is a tent the picture's march may sample off its
//! peak, where the sim reads a flat 2 m cell.)

use mush_atlas::TerrainClass;
use mush_atlas::trace::Eye;
use mush_pixel::heightfield::{self, Terrain};
use mush_pixel::look::{self, Facing};
use mush_pixel::raycast::{Camera, Ground, Map, Sprite, SpriteKind, SpritePose};
use tacsim::arena::state::{ArenaTerrain, TerrainCell};

/// Ten-metre cells, the arena's.
const CELL_CM: u16 = 1000;
/// The eye's height, the stock sensor's (`mush_atlas::trace::EYE_HEIGHT_CM`).
const EYE_CM: i32 = 150;
/// The berm's height.
const BERM_CM: i16 = 200;
/// The contact's height: the line tank's look, 7 m by 2.5 m.
const CONTACT_CM: i32 = 250;
/// The map: eleven cells across, the eye on the middle column, looking north
/// down it.
const W: u16 = 11;
/// See [`W`].
const H: u16 = 160;
/// The eye's cell.
const EYE_CELL: (u16, u16) = (5, 2);
/// A frame wide enough that half a metre at 1.5 km is a row (the focal
/// length is three-quarters of the width).
const FRAME: (usize, usize) = (4096, 240);

/// The cells with a berm `berm_m` out (three cells deep).
fn berm(berm_m: u16) -> impl Fn(u16) -> i16 {
    let first = EYE_CELL.1 + berm_m / 10;
    move |y| {
        if (first..first + 3).contains(&y) {
            BERM_CM
        } else {
            0
        }
    }
}

/// What the sim's ray says: the 1.5 m eye to the contact's top.
fn sim_sees(berm_m: u16, contact_m: u16) -> bool {
    let elev = berm(berm_m);
    let cells: Vec<TerrainCell> = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .map(|(_, y)| TerrainCell {
            class: TerrainClass::Grass,
            elev_cm: elev(y),
            move_cost: 1,
        })
        .collect();
    let terrain = ArenaTerrain::new(W, H, CELL_CM, cells).expect("the fixture's terrain");
    let from = Eye {
        cell: EYE_CELL,
        height_cm: EYE_CM,
    };
    let to = Eye {
        cell: (EYE_CELL.0, EYE_CELL.1 + contact_m / 10),
        height_cm: CONTACT_CM,
    };
    terrain.sight(from, to).sight.is_clear_or_partial()
}

/// What the picture draws: the rows of the contact's body it shows.
fn picture_rows(berm_m: u16, contact_m: u16) -> usize {
    let elev = berm(berm_m);
    let mut map = Map::new(usize::from(W), usize::from(H), u32::from(CELL_CM));
    for y in 0..usize::from(H) {
        for x in 0..usize::from(W) {
            map.set(x, y, Ground::Grass);
        }
    }
    let mut terrain = Terrain::new(map);
    for y in 0..H {
        for x in 0..W {
            terrain.set_elev_cm(usize::from(x), usize::from(y), i32::from(elev(y)));
        }
    }
    let centre = |cell: u16| i64::from(cell) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2;
    let cam = Camera {
        x_cm: centre(EYE_CELL.0),
        y_cm: centre(EYE_CELL.1),
        heading_mdeg: 0,
    };
    let sheet = look::for_unit("tacsim.unit.line_tank")
        .and_then(|l| l.sprites.as_ref())
        .expect("the line tank's sheet");
    assert_eq!(
        sheet.size_cm.1,
        CONTACT_CM.unsigned_abs(),
        "a 2.5 m contact"
    );
    let contact = Sprite {
        x_cm: centre(EYE_CELL.0),
        y_cm: centre(EYE_CELL.1 + contact_m / 10),
        kind: SpriteKind::Red,
        pose: Some(SpritePose {
            sheet,
            facing: Facing::Front,
            moving: false,
            step: 0,
        }),
    };
    heightfield::visible_rows(&terrain, &cam, Some(i64::from(EYE_CM)), FRAME, &contact)
}

#[test]
fn a_contact_behind_a_crest_shows_rows_exactly_when_the_sims_ray_sees_it() {
    // (berm, contact, whether the line to the contact's top clears the crest)
    let cases: [(u16, u16, bool); 6] = [
        (100, 140, true),
        (100, 400, false),
        (200, 240, true),
        (200, 800, false),
        (500, 540, true),
        (500, 1450, false),
    ];
    for (berm_m, contact_m, clears) in cases {
        let seen = sim_sees(berm_m, contact_m);
        assert_eq!(
            seen, clears,
            "the fixture: the sim's ray over a berm at {berm_m} m to a contact at {contact_m} m"
        );
        let rows = picture_rows(berm_m, contact_m);
        if seen {
            assert!(
                rows >= 1,
                "berm {berm_m} m, contact {contact_m} m: the sim sees it, the picture shows no row"
            );
        } else {
            assert_eq!(
                rows, 0,
                "berm {berm_m} m, contact {contact_m} m: the sim does not see it, the picture shows {rows} rows"
            );
        }
    }
    // With no berm the whole contact shows.
    assert!(picture_rows(2000, 140) >= picture_rows(100, 140) + 2);
}
