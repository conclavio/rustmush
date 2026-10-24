//! **The pixel picture measures sight as the sim does** (doc 21, the owner's
//! ask of 2026-10-22).
//!
//! The sim's line of sight runs between eyes at a unit's sensor height over
//! cells whose walls and woods stand at the atlas's obstacle heights. The
//! picture (`mush-pixel`'s raycaster and heightfield producer) draws with the
//! same numbers, so what it hides behind a wall is what the sim's sight hides.
//! `mush-pixel` cannot depend on the atlas (it compiles to WebAssembly), so it
//! mirrors them as constants, and this test is what keeps the mirror true.

use mush_atlas::TerrainClass;
use mush_atlas::physics::{WALL_HEIGHT_CM, obstacle_height_cm};
use mush_pixel::raycast::{self, Ground};

#[test]
fn the_picture_measures_sight_as_the_sim_does() {
    assert_eq!(raycast::EYE_HEIGHT_CM, mush_atlas::trace::EYE_HEIGHT_CM);
    assert_eq!(raycast::WALL_HEIGHT_CM, WALL_HEIGHT_CM);
    assert_eq!(
        raycast::WOODS_LIGHT_HEIGHT_CM,
        obstacle_height_cm(TerrainClass::WoodsLight)
    );
    assert_eq!(
        raycast::WOODS_HEAVY_HEIGHT_CM,
        obstacle_height_cm(TerrainClass::WoodsHeavy)
    );
    // The legend words the arena map uses land on the same classes.
    for (word, class) in [
        ("wall", TerrainClass::Wall),
        ("woods_light", TerrainClass::WoodsLight),
        ("woods_heavy", TerrainClass::WoodsHeavy),
        ("grass", TerrainClass::Grass),
        ("water", TerrainClass::Water),
        ("road", TerrainClass::Road),
    ] {
        assert_eq!(
            Ground::from_legend_name(word).height_cm(),
            obstacle_height_cm(class),
            "{word}"
        );
    }
}
