//! **The fixture yard** (16b cut Q.3's canonical fixture ground; moved here by
//! 16b cut I.1, because two test binaries now draw on it).
//!
//! `tests/cockpit.rs` blesses the cockpit fixture set over it
//! (`web/src/lib/fixtures/cockpit/`) and `tests/replay.rs` blesses the replay
//! fixture set over the same ground (`web/src/lib/fixtures/replay/`), so the
//! two sets are drawn on one yard and the web and TUI renderers compose both
//! over one terrain. It lives in `support` because an integration test cannot
//! import another one.

use mush_atlas::TerrainClass;
use smol_str::SmolStr;
use tacsim::arena::state::{ArenaTerrain, KeySpot, TerrainCell};

/// The synthetic ground's id, as the payloads' `surface` carries it.
pub const SURFACE: &str = "fixture_yard";

/// The ground is 96 by 64 cells of ten metres.
pub const WIDTH: u16 = 96;
/// As [`WIDTH`], up the ground.
pub const HEIGHT: u16 = 64;

/// The fixture ground: five terrain classes, a building block and a pond.
///
/// NOT the shipped proving ground. A fixture built on shipped content would
/// move whenever the content did, and what these files pin is the *shape* of
/// a payload, not the ground the game happens to ship.
#[must_use]
pub fn terrain() -> ArenaTerrain {
    let cell = |class: TerrainClass, move_cost: u16| TerrainCell {
        class,
        elev_cm: 0,
        move_cost,
    };
    let mut cells = vec![cell(TerrainClass::Grass, 120); usize::from(WIDTH) * usize::from(HEIGHT)];
    let mut put = |x: u16, y: u16, what: TerrainCell| {
        let index = usize::from(y) * usize::from(WIDTH) + usize::from(x);
        if let Some(slot) = cells.get_mut(index) {
            *slot = what;
        }
    };
    // An east-west road across the whole ground, ten cells up.
    for x in 0..WIDTH {
        put(x, 10, cell(TerrainClass::Road, 60));
    }
    // A pond north-east of the muster corner, inside the window.
    for y in 2..6 {
        for x in 20..30 {
            put(x, y, cell(TerrainClass::Water, 0));
        }
    }
    // A building, inside the window, with a contact standing on it.
    for y in 6..9 {
        for x in 12..16 {
            put(x, y, cell(TerrainClass::Wall, 0));
        }
    }
    // Woods well north of the window: on the map, off the view — which is
    // what makes the two legends differ.
    for y in 20..30 {
        for x in 40..60 {
            put(x, y, cell(TerrainClass::WoodsLight, 200));
        }
    }
    ArenaTerrain::new(WIDTH, HEIGHT, 1_000, cells).expect("the fixture grid is 96 by 64")
}

/// **The scene's three named spots** (16b cut V.1), each exercising one
/// rule of the window:
///
/// * `A` The Ford at (25, 3) — inside the window, on the pond: a letter drawn
///   over water, named in every legend;
/// * `B` The Roof at (14, 7) — inside the window and under contact `X3`: the
///   spot goes down first and `X3`'s `?` overwrites it, so `B` is in the
///   window's `units` (placed, then covered) and named in no legend, because
///   the contact owns the cell (in the replay fixture, `BB` stands on it);
/// * `C` The Woods at (50, 25) — off the 39 by 20 window and inside the woods,
///   so `VIEW`'s off-window line names it, and it is on the map (and on the
///   web's default whole-arena view) but not in the window.
#[must_use]
pub fn spots() -> Vec<KeySpot> {
    let spot = |name: &str, x: u16, y: u16| KeySpot {
        name: SmolStr::new(name),
        x,
        y,
    };
    vec![
        spot("The Ford", 25, 3),
        spot("The Roof", 14, 7),
        spot("The Woods", 50, 25),
    ]
}
