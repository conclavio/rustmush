//! **`sight_only` is the walk on the proving ground** (16b cut J.1; doc 16 §3.2
//! "Line of sight", §3.6 step 2; 16c N5).
//!
//! The arena's perception traces through [`mush_atlas::sight_only`] since cut
//! J.1: the canonical line, the same height at every step and the same per-cell
//! rule as [`mush_atlas::trace_cells`], with no allocation and a return at the
//! first blocker. `mush-atlas`' own tests prove it exact over every pair of a
//! synthetic relief; this proves it on **the ground the owner plays on** — the
//! shipped 256 by 256 proving ground, loaded through the production roster and
//! `tacsim::definitions::from_pack` exactly as `tests/definitions.rs` loads it —
//! at the shipped sight's reach and at both of the shipped sensor heights, so
//! the claim the cut's byte-identical outcomes rest on is asserted against the
//! real walls, woods, water and relief and not only against a fixture's.

mod support;

use mush_atlas::trace::{
    CellGrid, EYE_HEIGHT_CM, Eye, Sight, canonical_ends, sight_only, trace_cells,
};
use mush_testkit::TestWorld;
use tacsim::arena::state::ArenaDefinition;
use tacsim::definitions::Catalog;

/// A fixed-seed integer generator: splitmix64, the atlas M.4 tests' own, so the
/// pairs are the same on every run and every platform, and no `rand`.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A sight with the blocker's identity dropped (the atlas M.4 tests' `seen`):
/// which blocker a walk meets first depends on the end it starts from, and the
/// property is about whether the line is seen along, and how attenuated.
fn seen(sight: &Sight) -> (u8, u8) {
    match sight {
        Sight::Clear => (0, 0),
        Sight::Partial(pct) => (1, *pct),
        Sight::BlockedBy(_) => (2, 0),
    }
}

/// **The three J.1 assertions for one ordered pair** (the atlas tests' own),
/// answering the sight: exactly the canonical walk's sight, blocker and all;
/// seen along as the walk asked from `a` is, with one attenuation whenever either
/// is partial; and two cells fewer read at least (the walk counts both
/// endpoints).
fn j1_agrees(grid: &dyn CellGrid, a: Eye, b: Eye) -> Sight {
    let (lo, hi) = if canonical_ends(a.cell, b.cell) == (a.cell, b.cell) {
        (a, b)
    } else {
        (b, a)
    };
    let fast = sight_only(grid, a, b);
    let walk = trace_cells(grid, lo, hi);
    assert_eq!(
        fast.sight, walk.sight,
        "sight_only and the canonical walk disagree from {a:?} to {b:?}"
    );
    let asked = trace_cells(grid, a, b);
    assert_eq!(
        seen(&fast.sight),
        seen(&asked.sight),
        "sight_only and the walk asked from {a:?} disagree to {b:?}: {:?} against {:?}",
        fast.sight,
        asked.sight
    );
    if matches!(fast.sight, Sight::Partial(_)) || matches!(asked.sight, Sight::Partial(_)) {
        assert_eq!(walk.attenuation, asked.attenuation, "{a:?} to {b:?}");
        if let Sight::Partial(pct) = fast.sight {
            assert_eq!(u32::from(pct), asked.attenuation, "{a:?} to {b:?}");
        }
    }
    assert!(
        fast.visited + 2 <= walk.visited,
        "{a:?} to {b:?}: read {} against the walk's {}",
        fast.visited,
        walk.visited
    );
    fast.sight
}

/// The shipped proving ground as an [`ArenaDefinition`], the way
/// `tests/definitions.rs::from_pack_yields_the_window_and_the_right_impassable_cells`
/// gets it: the production roster with the boot packs on, the atlas records off
/// `AtlasState`, the one shipped arena, `from_pack`.
fn proving_ground() -> ArenaDefinition {
    let tw = TestWorld::with_modules(support::production_roster());
    let records = tw
        .world()
        .module_state::<mush_atlas::AtlasState>()
        .map(|state| state.records.clone())
        .expect("the atlas is composed");
    let catalog = Catalog::build_game(&[], tacsim::NS).expect("the shipped content binds");
    let (id, def) = catalog
        .arenas
        .iter()
        .next()
        .expect("the proving ground is shipped");
    tacsim::definitions::from_pack(id.as_str(), def, &records)
        .expect("the arena's ground is in this world's records")
}

/// One offset in `-60..=60` cells — the shipped sight's sixty-cell reach.
fn offset(state: &mut u64) -> i32 {
    i32::try_from(next(state) % 121).unwrap_or(0) - 60
}

/// **`sight_only` is the walk on the proving ground** (16b cut J.1): two
/// thousand seeded origins over the whole 256 by 256 window, each against
/// sixty-four targets inside the shipped sight's sixty cells on each axis
/// (clamped to the window, a target on the origin itself drawn again), asked in
/// both directions — 256,000 ordered pairs — at 150 cm eyes, and at the biped's
/// 900 cm `sensor_height_cm` from every eighth origin, so mixed heights are on
/// the shipped ground too. Every pair is the canonical walk's exact sight.
#[test]
fn j1_sight_only_is_the_walk_on_the_proving_ground() {
    let arena = proving_ground();
    let (width, height) = (arena.terrain.width(), arena.terrain.height());
    assert_eq!((width, height), (256, 256), "the shipped window");
    let clamp = |at: i32, side: u16| -> u16 {
        u16::try_from(at.clamp(0, i32::from(side) - 1)).unwrap_or(0)
    };
    let mut state = 0x4a31_u64;
    let mut counts = [0_u32; 3];
    let mut pairs = 0_u32;
    for origin in 0..2_000_u32 {
        let cell = (
            u16::try_from(next(&mut state) % u64::from(width)).unwrap_or(0),
            u16::try_from(next(&mut state) % u64::from(height)).unwrap_or(0),
        );
        let from = Eye {
            cell,
            height_cm: if origin % 8 == 4 { 900 } else { EYE_HEIGHT_CM },
        };
        for _ in 0..64 {
            let to = loop {
                let at = (
                    clamp(i32::from(cell.0) + offset(&mut state), width),
                    clamp(i32::from(cell.1) + offset(&mut state), height),
                );
                if at != cell {
                    break at;
                }
            };
            let to = Eye::standing(to);
            for sight in [
                j1_agrees(&arena.terrain, from, to),
                j1_agrees(&arena.terrain, to, from),
            ] {
                if let Some(count) = counts.get_mut(usize::from(seen(&sight).0)) {
                    *count += 1;
                }
                pairs += 1;
            }
        }
    }
    assert_eq!(pairs, 256_000);
    // The ground is worth testing over: all three answers, often. The partial
    // bound is two thousand and not five: the proving ground's light woods are a
    // few belts, and this stream crosses one and stops short of a hundred on
    // 2,904 of its pairs (against 204,048 clear and 49,048 blocked).
    let [clear, partial, blocked] = counts;
    assert!(
        clear > 5_000 && partial > 2_000 && blocked > 5_000,
        "{clear} clear, {partial} partial, {blocked} blocked"
    );
}
