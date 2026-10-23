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
//!
//! **16b cut J.2** adds the arena's own query through the terrain's horizon
//! index as a third column on the same pairs, and one fight on the same ground
//! replayed with and without the index.

mod support;

use mush_atlas::trace::{
    CellGrid, EYE_HEIGHT_CM, Eye, Sight, canonical_ends, sight_only, trace_cells,
};
use mush_testkit::TestWorld;
use support::{Muster, booted_world, ledger_events, shipped_arena};
use tacsim::arena::clock::Tempo;
use tacsim::arena::state::{Arena, ArenaDefinition, ArenaTerrain, Team};
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

/// **The J.2 column for one ordered pair** (16b cut J.2): the arena's own query
/// through the terrain's horizon index answers `sight_only`'s exact sight and
/// never reads more cells; answers whether it read fewer.
fn j2_agrees(terrain: &ArenaTerrain, a: Eye, b: Eye) -> bool {
    let through = terrain.sight(a, b);
    let walked = sight_only(terrain, a, b);
    assert_eq!(
        through.sight, walked.sight,
        "the horizon index and sight_only disagree from {a:?} to {b:?}"
    );
    assert!(
        through.cell_probes <= walked.visited,
        "{a:?} to {b:?}: the index read {} cells against sight_only's {}",
        through.cell_probes,
        walked.visited
    );
    through.cell_probes < walked.visited
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
///
/// **16b cut J.2's third column**, on the same pairs: the arena's own query
/// through the terrain's horizon index ([`ArenaTerrain::sight`]) is
/// `sight_only`'s exact sight on every pair and never reads more cells, and over
/// the run it reads fewer on at least a quarter of the pairs, so the index is
/// not vacuous on the ground the game ships.
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
    let mut cheaper = 0_u32;
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
            for (a, b) in [(from, to), (to, from)] {
                cheaper += u32::from(j2_agrees(&arena.terrain, a, b));
            }
        }
    }
    assert_eq!(pairs, 256_000);
    // The ground is worth testing over: all three answers, often. The partial
    // bound is fifteen hundred and not five thousand: the proving ground's light
    // woods are a few belts, and this stream crosses one and stops short of a
    // hundred on 1,852 of its pairs (against 116,596 clear and 137,552 blocked
    // since doc 21 cut G.7b's hills; 2,904 against 204,048 and 49,048 on the
    // flatter version 2).
    let [clear, partial, blocked] = counts;
    assert!(
        clear > 5_000 && partial > 1_500 && blocked > 5_000,
        "{clear} clear, {partial} partial, {blocked} blocked"
    );
    // 16b cut J.2: the index read fewer cells than `sight_only` on at least a
    // quarter of the pairs.
    println!("j2_proving_ground: {cheaper} of {pairs} pairs read fewer cells through the index");
    assert!(
        cheaper >= pairs / 4,
        "the horizon index read fewer cells on only {cheaper} of {pairs} pairs"
    );
}

/// What one shipped-ground replay left: every ledger byte, every per-substep
/// hash, each unit's final pose, limits and speed (as `Debug`, which is all of
/// their fields), and the run's perception cost.
struct Fight {
    ledger: Vec<u8>,
    hashes: Vec<u64>,
    finals: Vec<String>,
    horizon_probes: u64,
    cell_probes: u64,
    fired: usize,
}

/// Step `arena` at a pinned 500 ms `Engaged` for `until_ms` of arena time.
fn fight(mut arena: Arena, until_ms: i64) -> Fight {
    let mut ledger: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let (mut horizon_probes, mut cell_probes) = (0_u64, 0_u64);
    let mut wall = arena.now_ms;
    while arena.now_ms < until_ms {
        arena.clock.pin_tempo_for_tests(Tempo::Engaged);
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            ledger.extend_from_slice(&chunk.bytes);
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        horizon_probes += out.perception.horizon_probes;
        cell_probes += out.perception.cell_probes;
        if out.ended.is_some() {
            break;
        }
        wall += 500;
    }
    let fired = ledger_events(&mut arena, &mut ledger)
        .iter()
        .filter(|event| event.kind.as_str() == "fired")
        .count();
    Fight {
        ledger,
        hashes,
        finals: arena
            .units
            .iter()
            .map(|unit| {
                format!(
                    "{:?} {:?} {:?} {}",
                    unit.id, unit.pose, unit.limits, unit.speed_cms
                )
            })
            .collect(),
        horizon_probes,
        cell_probes,
        fired,
    }
}

/// **The horizon index changes nothing on the shipped ground, with fire in it**
/// (16b cut J.2): one stock machine a side — the E.1 musters of
/// `tests/shipped_scale.rs`, Blue at `(20, 97)` (`tests/report.rs`' `BLUE_AT`)
/// and Red at the first passable cell with a way out 30 cells from it in
/// Chebyshev distance, scanning outward, because the authored musters open about
/// 2 km apart, outside the 600 m sight — stepped at a pinned 500 ms `Engaged`
/// for sixty arena seconds twice, the second time on the same arena with its
/// terrain made [`ArenaTerrain::without_horizon`]. Every ledger byte, every
/// per-substep hash and the final poses, limits and speeds are equal; the index
/// read blocks and fewer cells; and the replay carried fire.
///
/// **17b cut M4.5a**: the stock machine a side is a unit mind flown by
/// Standard Doctrine on the native stock mind ([`Muster::mind`], what
/// production fields since 17b cut M4.3), where it was the reflex controller;
/// every claim is kept as written. A mind's think reads the same perception the
/// index serves, so the replay still carries it through fire.
#[test]
fn j2_a_shipped_fight_is_one_replay_with_and_without_the_index() {
    const BLUE_AT: (u16, u16) = (20, 97);
    let world = booted_world();
    let terrain = shipped_arena(&world, &[&[Muster::mind(Team::Blue, 1)]])
        .definition
        .terrain
        .clone();
    let red_at = (30..=40_i32)
        .flat_map(|d| {
            let (bx, by) = (i32::from(BLUE_AT.0), i32::from(BLUE_AT.1));
            (by - d..=by + d).flat_map(move |y| {
                (bx - d..=bx + d)
                    .filter(move |x| (x - bx).abs() == d || (y - by).abs() == d)
                    .map(move |x| (x, y))
            })
        })
        .filter_map(|(x, y)| Some((u16::try_from(x).ok()?, u16::try_from(y).ok()?)))
        .find(|&(x, y)| terrain.has_way_out(x, y))
        .expect("a cell with a way out 30 to 40 cells from Blue");
    let arena = || {
        shipped_arena(
            &world,
            &[
                &[Muster::mind(Team::Red, 1).at(red_at)],
                &[Muster::mind(Team::Blue, 1).at(BLUE_AT)],
            ],
        )
    };
    let built = fight(arena(), 60_000);
    let mut walk_only = arena();
    let terrain = walk_only.definition.terrain.clone().without_horizon();
    std::sync::Arc::make_mut(&mut walk_only.definition).terrain = terrain;
    let walked = fight(walk_only, 60_000);
    println!(
        "j2_shipped_fight: red at {red_at:?}, {} fired, {} ledger bytes, {} hashes; built {} \
         cells {} blocks, walk-only {} cells",
        built.fired,
        built.ledger.len(),
        built.hashes.len(),
        built.cell_probes,
        built.horizon_probes,
        walked.cell_probes
    );
    assert!(built.ledger == walked.ledger, "the ledgers differ");
    assert_eq!(built.hashes, walked.hashes, "the substep hashes differ");
    assert_eq!(
        built.finals, walked.finals,
        "a final pose, limit or speed differs"
    );
    assert_eq!(walked.horizon_probes, 0);
    assert!(built.horizon_probes > 0, "the index read no block");
    assert!(
        built.cell_probes < walked.cell_probes,
        "the index read {} cells against the walk's {}",
        built.cell_probes,
        walked.cell_probes
    );
    assert!(built.fired > 0, "sixty seconds carried no shot");
}
