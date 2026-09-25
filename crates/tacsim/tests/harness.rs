//! **Cut H — the TacSim scale harness** (16b §5, H1–H4; doc 16 §1 "A2" and §11;
//! `11-h-implementation-plan.md` §3.3 and D-H-8).
//!
//! What this is. One `#[ignore]` measurement, run by the CI `tacsim-harness` job
//! on `workflow_dispatch` and the weekly schedule (report-only, `--release`,
//! `--nocapture`, `harness.json` kept as an artifact) and locally by hand. It
//! composes the **production** composition root — the same calls
//! `mush_server::boot::game_modules` makes, read out of
//! `tacsim::composition`, so the game crate takes no edge on the host — inside a
//! `mush_testkit::TestWorld` over the in-memory world, generates a relief
//! deterministically from `TACSIM_SEED`, applies it through the atlas's own pack
//! route, opens a domain window over it through the existing `@atlas/window`
//! lane, places `TACSIM_UNITS` objects on the window with
//! `@spatial/place/domain`, connects `TACSIM_FIGHTING` fake sessions as
//! observers, and then drives `TACSIM_PULSES` fires of
//! `mush_sim::tick::PULSE_TICK_TAG` through `TestWorld::fire_due`, timing each
//! one and counting the output events it fans out.
//!
//! H2: before cut A exists there is nothing else to measure — the numbers below
//! are the **baseline** for the world-side lane as it stands, which is what cut
//! A's `Arena::step` is compared against.
//!
//! Knobs, all optional, `tacsim::config::HarnessConfig::default()` (16b §9) being
//! the baseline: `TACSIM_UNITS`, `TACSIM_FIGHTING`, `TACSIM_SEED`, `TACSIM_JSON`
//! (the report path, default `target/harness.json`) and `TACSIM_ASSERT=1`, which
//! turns the thresholds into assertions. The default is **report-only** (D-H-8,
//! D-16-19): a shared runner's speed is not a threshold. `TACSIM_PULSES` is the
//! ground-pulse lane's count of fires and nothing else's. There is no weapons
//! knob (16b cut U.3, 16c `T-37`): `TACSIM_WEAPONS` and `[tacsim.harness].weapons`
//! were parsed and reported and armed nothing since cut W — the cohort is armed as
//! the catalog authors it — so both are gone.
//!
//! **The arena lanes since 16b cut U.1** (16c `T-35`, `T-22`, `T-25`, `T-24`,
//! `T-34`). The field is the game's: every unit is a unit of the fielded catalog
//! checked out through `tacsim::match_::custody_unit` (`COHORT`: the line tank,
//! the laser, the biped, the thermal, the ECM and the ECCM tanks, armed as
//! authored), over an integer-generated relief with two seeded terraces whose
//! rims refuse tracked and legged edges. The loop is the task's: the arena is
//! stepped at `StepOutput::next_due_ms` — the substep lattice in absolute arena
//! time — for `TACSIM_ARENA_S` arena seconds (default 30) after the warm-up, plus
//! a seeded stand-in for the timer's overshoot, `TACSIM_WAKE_JITTER_MS` (default
//! 0). Every wake is also folded into 500 ms windows of arena time, and the 12 ms
//! lens binds both. Two runs: `tacsim_harness_arena_step` at the CI reach (sight
//! scaled 60 to `TACSIM_SENSOR_CELLS`, 24; mesh 30 to `TACSIM_SHARE_CELLS`, 8),
//! asserted under `TACSIM_ASSERT`, and `tacsim_harness_arena_authored_reach` at
//! the reaches as authored, report-only. The terrain's move costs are still the
//! harness's own one table (`harness_move_cost`): 16c `B-58`'s per-locomotion
//! cost layer is not built.
//!
//! **The bounds since 16b cut U.2** (16c `T-36`, `T-29`, `T-30`). The CI lane
//! asserts cell probes and jam checks per wake beside the traces; resident memory
//! is read on every platform through `memory-stats` (Linux's `VmRSS` printed
//! beside it); and two more lanes run on the same ignore string:
//! `tacsim_harness_arena_long_run`, the CI cohort for `TACSIM_LONG_RUN_S` arena
//! seconds (default 600) with memory read every arena minute and its growth
//! bounded, and `tacsim_harness_fan_out`, 24 staff-seated pilots in one match on
//! the shipped proving ground with the real egress applied one message per apply,
//! its keyframe bytes and output events per pulse bounded.
//!
//! **`horizon_probes` since 16b cut J.2** (16c N5 step 2). Beside those bounds
//! the arena lanes print the horizon index's block reads per wake, per window
//! and in total, and its size on the memory line (`horizon_bytes`, under
//! `T-36`'s bound); the delta compares the three counts in the deterministic
//! block. There is **no bound** on them: the counter is what N5 asks for, and a
//! bound would be a seventh config site for a number cut J.3 first measures.
//!
//! **The delta since 16b cut U.3** (16c `T-37`, Q29). A measurement that is never
//! compared with the last one is a number nobody reads, so one more ignored test,
//! `tacsim_harness_delta_against_the_committed_baseline`, reads the fresh CI-cohort
//! report and the committed `tests/harness_baseline/arena.<os>.json` and compares
//! them in two blocks: **the deterministic block** (the world, the substeps, the
//! counters, the weapons, the perception and controller totals and peaks, the
//! window peaks, the thresholds) exactly, and **the timing block** (the wake p50
//! and p99 and the window p99) only on the host that blessed the file, failing
//! only on a fresh figure more than 20 per cent *slower*. `TACSIM_BASELINE_BLESS=1`
//! writes the file from the fresh report instead. A report older than the test
//! binary is refused, so a change is never judged on the last build's numbers.
//! It is the one step that fails the CI job on a *measurement* — which leaves
//! the absolutes report-only, because it runs on a shared runner; the J.1 sight
//! differential below is a correctness lane and fails the first step on a
//! mismatch, never on a number — while
//! `scripts/harness-weekly.sh`, the one command the owner runs by hand, asserts
//! the absolutes as well (D-16-19: *assert locally*).
//!
//! **The sight differential since 16b cut J.1** (16c N5 step 1). The arena's
//! perception traces through `mush_atlas::sight_only`, which stops at the first
//! blocker; `tacsim_harness_sight_differential`, on the same ignore string, walks
//! it against `mush_atlas::trace_cells` on the shipped proving ground (the boot
//! packs on, [`production_roster_with`]) — `TACSIM_SIGHT_ORIGINS` seeded origins
//! (default 2,000), each against every cell within the shipped sight's sixty,
//! both directions — and then on the harness relief with half as many origins,
//! and asserts on every pair, whatever `TACSIM_ASSERT` says, that the two answer
//! one sight. It is a correctness lane; its timings are printed and never
//! asserted, and the delta does not read its JSON (`<stem>-sight.<ext>`). Since
//! 16b cut J.2 it runs a third column on the same pairs, the arena's own query
//! through the terrain's horizon index, asserted `sight_only`'s exact sight and
//! never more cells read, and prints its cells and blocks read.
//!
//! **The authored lane after 16b cut J.3** (16c `T-34`, Q26, N5). With the
//! early-stopping trace and the horizon index in, the authored cohort was
//! measured three times through `scripts/harness-weekly.sh` on the blessing host
//! (Windows, release, `TACSIM_ASSERT=1`): per wake p99 7.699 to 8.156 ms, per
//! window p99 14.108 to 15.092 ms, against the 12 ms lens — and against 10.3 ms,
//! the lens over 1.16 (12.0 / 1.16, so a figure 16 per cent slower still holds
//! 12; 16 per cent is what two runs of one tree have differed by), which is the
//! margin a weekly assertion needs. The per-wake peaks are 2,994 traces
//! (bound 4,000), 19,858 cell probes (32,000), 1,124 jam checks (2,048) and 452
//! share operations (256). The window is over, so the authored lane stays
//! **report-only pending the owner's Q26**: the 600 m sight, the bounds, the
//! 300-unit target, or N5's step 3, the work budget. The CI cohort is asserted
//! as before; a run of its lane at other sight knobs is a sizing run of another
//! field, and skips the CI cohort's warm-up wreck check ([`open_field`]).
//!
//! What it must not do (16b §9's `H` row): touch `mush-sim` or `mush-atlas` — a
//! harness that needs a new API in the thing it measures is measuring its own
//! change — assert by default, or run on push. The ban on asserting by default
//! covers the thresholds — timings, budgets, bounds — and nothing else: the sight
//! differential (16b cut J.1) is a correctness lane that asserts always, because
//! a wrong sight is a bug on any runner; and `sight_only` is the arena's trace,
//! built for the arena (16c N5), which the lane only checks, not an API the
//! harness asked for.

use std::collections::BTreeMap;
use std::time::Instant;

use mush_atlas::{AtlasConfig, AtlasState, PackSource, TerrainClass};
use mush_core::{
    Audience, CanonicalId, Effect, Effects, GOD, GameModule, Inbound, ObjId, ObjKind, Outbox,
    WorldCommand,
};
use mush_proto::OutputEvent;
use mush_sim::DomainId;
use mush_testkit::TestWorld;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use tacsim::definitions::{Catalog, content};

// Every test here carries
// `#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch /
// weekly)"]`. `ci_gated_test_inventory` finds the job that owns them by the verbatim
// `-p tacsim --test harness` in `.github/workflows/ci.yml`.

// ---------------------------------------------------------------------------
// Knobs
// ---------------------------------------------------------------------------

/// The world facts one run was driven at, and the thresholds.
struct Knobs {
    units: u32,
    fighting: u32,
    /// The ground-pulse lane's count of fires. The arena lanes run for
    /// [`Knobs::arena_s`] arena seconds instead (16b cut U.1).
    pulses: u32,
    /// **`TACSIM_ARENA_S`** (16b cut U.1): how many arena seconds the arena lanes
    /// measure after their warm-up. Default [`DEFAULT_ARENA_S`].
    arena_s: u32,
    /// **`TACSIM_WAKE_JITTER_MS`** (16b cut U.1): the most a wake is late, a
    /// seeded stand-in for the OS timer's overshoot, so a report can say what
    /// the remainder substeps it cuts cost. Default nought: the lattice exactly.
    jitter_ms: i64,
    seed: u64,
    json: String,
    assert: bool,
    engaged_p99_ms: u32,
    max_traces_per_pulse: u32,
    max_path_queries_per_pulse: u32,
    /// **16b cut S.1** — contact-share operations one pulse may make.
    max_share_ops_per_pulse: u32,
    max_ledger_bytes: u64,
    /// **16b cut U.2** (16c `T-36`): cells one wake's traces may read.
    max_cell_probes_per_pulse: u32,
    /// **16b cut U.2** (16c `T-29`): index candidates one wake's jammer scans
    /// may look at.
    max_jam_checks_per_pulse: u32,
    /// **16b cut U.2** (16c `T-36`): resident-memory growth the long run may
    /// show, KiB.
    max_rss_growth_kib: u32,
    /// **16b cut U.2** (16c `T-36`): the largest `arena.cockpit` keyframe the
    /// fan-out lane may see, bytes.
    max_frame_bytes: u32,
    /// **16b cut U.2** (16c `T-36`): output events one arena pulse may produce
    /// through the fan-out lane.
    max_output_events_per_pulse: u32,
    /// **`TACSIM_LONG_RUN_S`** (16b cut U.2): the long-run lane's measured
    /// window, arena seconds. Default [`DEFAULT_LONG_RUN_S`].
    long_run_s: u32,
    /// **`TACSIM_SIGHT_ORIGINS`** (16b cut J.1): how many seeded origins the
    /// sight differential walks on the proving ground (half as many on the
    /// relief). Default [`DEFAULT_SIGHT_ORIGINS`].
    sight_origins: u64,
}

fn env_u64(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(fallback)
}

#[allow(clippy::cast_possible_truncation)]
fn env_u32(name: &str, fallback: u32) -> u32 {
    env_u64(name, u64::from(fallback)).min(u64::from(u32::MAX)) as u32
}

impl Knobs {
    fn from_env() -> Knobs {
        let cfg = tacsim::config::HarnessConfig::default();
        Knobs {
            units: env_u32("TACSIM_UNITS", cfg.units),
            fighting: env_u32("TACSIM_FIGHTING", cfg.fighting),
            pulses: env_u32("TACSIM_PULSES", DEFAULT_PULSES),
            arena_s: env_u32("TACSIM_ARENA_S", DEFAULT_ARENA_S),
            jitter_ms: i64::from(env_u32("TACSIM_WAKE_JITTER_MS", 0)),
            seed: env_u64("TACSIM_SEED", DEFAULT_SEED),
            json: std::env::var("TACSIM_JSON")
                .unwrap_or_else(|_| "target/harness.json".to_string()),
            assert: std::env::var("TACSIM_ASSERT").is_ok_and(|v| v.trim() == "1"),
            engaged_p99_ms: cfg.engaged_p99_ms,
            max_traces_per_pulse: cfg.max_traces_per_pulse,
            max_path_queries_per_pulse: cfg.max_path_queries_per_pulse,
            max_share_ops_per_pulse: cfg.max_share_ops_per_pulse,
            max_ledger_bytes: cfg.max_ledger_bytes,
            max_cell_probes_per_pulse: cfg.max_cell_probes_per_pulse,
            max_jam_checks_per_pulse: cfg.max_jam_checks_per_pulse,
            max_rss_growth_kib: cfg.max_rss_growth_kib,
            max_frame_bytes: cfg.max_frame_bytes,
            max_output_events_per_pulse: cfg.max_output_events_per_pulse,
            long_run_s: env_u32("TACSIM_LONG_RUN_S", DEFAULT_LONG_RUN_S),
            sight_origins: env_u64("TACSIM_SIGHT_ORIGINS", DEFAULT_SIGHT_ORIGINS),
        }
    }
}

/// 16b §5 H1: sixty pulses is a minute of ground cadence at the default period.
const DEFAULT_PULSES: u32 = 60;

/// The arena lanes' measured window, arena seconds (16b cut U.1): the thirty
/// the sixty 500 ms pulses they used to step covered.
const DEFAULT_ARENA_S: u32 = 30;

/// **The long-run lane's measured window**, arena seconds (16b cut U.2, 16c
/// `T-36`): ten minutes, sampled every [`LONG_RUN_SAMPLE_S`].
const DEFAULT_LONG_RUN_S: u32 = 600;

/// How often the long run reads resident memory, arena seconds.
const LONG_RUN_SAMPLE_S: i64 = 60;

/// **The sight differential's origins on the proving ground** (16b cut J.1),
/// each against every cell within [`SIGHT_REACH_CELLS`]: about 45 million
/// ordered pairs. The relief gets half as many.
const DEFAULT_SIGHT_ORIGINS: u64 = 2_000;

/// The fixed seed, so a run with no environment at all is reproducible.
const DEFAULT_SEED: u64 = 2_350;

// ---------------------------------------------------------------------------
// The relief
// ---------------------------------------------------------------------------

/// The arena's side in cells: 256 at `[tacsim].ground_cell_cm` is D-16-5's
/// 2.56 km arena, and exactly `mush_atlas::MAX_TILE_SIDE` and
/// `DEFAULT_WINDOW_MAX_CELLS`.
const SIDE: u16 = 256;

/// `[tacsim].ground_cell_cm`. A const because the pack JSON is built with it;
/// the measurement asserts it still equals `mush_config`'s own default, so an
/// operator-facing change to that key cannot leave this reading a stale number.
const CELL_CM: u16 = 1_000;

/// One heightfield entry's side. A `heightfield` entry carries its samples in
/// its own `def`, and `mush_content`'s `MAX_ENTRY_BYTES` is 64 KiB, so a 256 by
/// 256 le16 field (175 KB of base64) does not fit in one entry — the def's own
/// documentation says "a field wider than a tile is several entries". Four
/// 128 by 128 quadrants are about 43 KB of base64 each and tile the arena
/// exactly.
const QUAD: u16 = 128;

/// Where the region's own metric frame starts, metres. Offset off the shipped
/// lot (`region.limbo`, 0..400 m) so the bbox form of `@atlas/window open`
/// cannot collect a lot room as an anchor and refuse the window for spanning
/// two regions.
const ORIGIN_M: i64 = 10_000;

/// How far elevation is quantised, centimetres. The relief is smooth and the
/// cell grid is run-length encoded (`mush_atlas::Cells::encode`, eight bytes a
/// run, `MAX_TILE_DOC_BYTES` 512 KiB), so quantising to whole metres is what
/// keeps the runs long enough for a 256-cell row to encode.
const ELEV_STEP_CM: i32 = 100;

/// **The two terraces' rises**, centimetres (16b cut U.1; 16c `T-35`): the step
/// every cell inside a terrace's disc is raised by, so its rim is one edge that
/// high over one cell.
///
/// Read against the shipped `locomotion.json` rows the cohort drives under —
/// tracked 45 %, legged 70 %, biped 60 % — on 1,000 cm cells:
///
/// * **+800 cm** is an 80 % orthogonal rim and a 57 % diagonal one (the
///   diagonal's run is the atlas's `cell × 141 / 100`): refused to the tracked
///   hulls at 45 either way and to the legged biped at 70 on the orthogonal, but
///   admitted to the legs through the corner — the case the shoulder rule
///   exists for, since a corner both of whose shoulders are that rim is a
///   corner nothing walks through;
/// * **+500 cm** is 50 % orthogonal and 35 % diagonal: the tracked hulls are
///   refused the orthogonal step and admitted through the corner, and the biped
///   climbs it either way.
///
/// The waves under them are smooth enough that neither limit is reached without
/// them, which is why they exist: without a refusing edge the slope limit, the
/// shoulder rule and the per-mode A\* cost were run in none of the 300 units.
const TERRACE_RISE_CM: [i32; 2] = [800, 500];

/// A terrace's radius, cells, seeded inside this range.
const TERRACE_RADIUS_CELLS: std::ops::RangeInclusive<i32> = 12..=20;

/// How far the fighting lines reach from the window's middle row, cells: they
/// stand at rows `middle − 18 .. middle + 15` (see [`field`]), so a terrace
/// whose centre is at least this plus its radius plus two rows off the middle
/// never touches them.
const LINE_ROWS_FROM_MIDDLE: i32 = 18;

/// A seeded smooth elevation field over the arena, in whole centimetres against
/// the region datum, with two seeded terraces on it.
///
/// Four summed waves with seeded phases and one seeded tilt, then the two
/// [`TERRACE_RISE_CM`] discs: reproducible from `TACSIM_SEED` alone, no table,
/// no file, and bounded well inside the `i16` a cell's `elev_cm` is.
///
/// **Integer throughout** (16b cut U.1). The waves were `f64` `sin` and `cos`,
/// which the platform's libm computes, so a baseline committed on one OS need
/// not have been the terrain another OS generated. They are the arena's own
/// fixed-point sine now — `tacsim::arena::motion::sin_fx` and `cos_fx`, the
/// half-degree table motion and the reflex look-ahead already read, scaled to
/// `SIN_ONE` — over integer phases in millidegrees, and the tilt is thousandths
/// of a centimetre a cell; the seeds are integer draws. One relief, one grid,
/// on every host.
struct Relief {
    /// Row-major, `SIDE * SIDE`, southern row first.
    elev_cm: Vec<i32>,
    /// The two terraces as `(centre x, centre y, radius, rise_cm)`, cells.
    terraces: Vec<(u16, u16, u16, i32)>,
}

impl Relief {
    fn generate(seed: u64) -> Relief {
        use tacsim::arena::motion::{SIN_ONE, cos_fx, sin_fx};
        const FULL_TURN_MDEG: i64 = 360_000;
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        // Four waves: two long, two short. Periods across the arena, amplitudes
        // in centimetres, and a seeded phase on each axis in millidegrees.
        let waves: Vec<(i64, i64, i64, i64)> = [(3_i64, 900_i64), (5, 520), (9, 260), (17, 120)]
            .into_iter()
            .map(|(periods, amp_cm)| {
                (
                    periods,
                    amp_cm,
                    rng.random_range(0..FULL_TURN_MDEG),
                    rng.random_range(0..FULL_TURN_MDEG),
                )
            })
            .collect();
        // The tilt, thousandths of a centimetre a cell: -4.000 to 4.000 cm.
        let tilt_x = rng.random_range(-4_000_i64..4_000);
        let tilt_y = rng.random_range(-4_000_i64..4_000);
        let side = i64::from(SIDE);
        // A wave's phase at one coordinate, millidegrees in `0..360_000`.
        let phase = |periods: i64, at: i64, offset: i64| -> u32 {
            u32::try_from(
                (periods * at * FULL_TURN_MDEG / side + offset).rem_euclid(FULL_TURN_MDEG),
            )
            .unwrap_or(0)
        };
        let mut elev_cm = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE));
        for y in 0..side {
            for x in 0..side {
                // Thousandths of a centimetre until the quantisation.
                let mut milli = tilt_x * x + tilt_y * y;
                for (periods, amp_cm, px, py) in &waves {
                    let across = amp_cm * 1_000 * sin_fx(phase(*periods, x, *px)) / SIN_ONE;
                    milli += across * cos_fx(phase(*periods, y, *py)) / SIN_ONE;
                }
                // Quantise to the nearest step, then clamp well inside `i16` so no
                // sample is refused.
                let step = i64::from(ELEV_STEP_CM) * 1_000;
                let q = (milli + step / 2).div_euclid(step) * i64::from(ELEV_STEP_CM);
                elev_cm.push(i32::try_from(q.clamp(-20_000, 20_000)).unwrap_or(0));
            }
        }
        // The terraces, clear of the fighting lines: any column, and a row at
        // least the lines' reach plus the radius plus two off the middle, on a
        // seeded side of it.
        let middle = i32::from(SIDE) / 2;
        let mut terraces = Vec::new();
        for rise in TERRACE_RISE_CM {
            let radius = rng.random_range(TERRACE_RADIUS_CELLS);
            let clear = LINE_ROWS_FROM_MIDDLE + radius + 2;
            let cx = rng.random_range(0..i32::from(SIDE));
            let cy = if rng.random_range(0..2_u8) == 0 {
                rng.random_range(0..=middle - clear)
            } else {
                rng.random_range(middle + clear..i32::from(SIDE))
            };
            for y in (cy - radius).max(0)..=(cy + radius).min(i32::from(SIDE) - 1) {
                for x in (cx - radius).max(0)..=(cx + radius).min(i32::from(SIDE) - 1) {
                    if (x - cx) * (x - cx) + (y - cy) * (y - cy) > radius * radius {
                        continue;
                    }
                    let at = usize::try_from(y * i32::from(SIDE) + x).unwrap_or(usize::MAX);
                    if let Some(cell) = elev_cm.get_mut(at) {
                        *cell = (*cell + rise).clamp(-20_000, 20_000);
                    }
                }
            }
            terraces.push((
                u16::try_from(cx).unwrap_or(0),
                u16::try_from(cy).unwrap_or(0),
                u16::try_from(radius).unwrap_or(0),
                rise,
            ));
        }
        Relief { elev_cm, terraces }
    }

    fn at(&self, x: u16, y: u16) -> i32 {
        self.elev_cm
            .get(usize::from(y) * usize::from(SIDE) + usize::from(x))
            .copied()
            .unwrap_or(0)
    }

    /// One quadrant's samples as base64 over little-endian signed 16-bit
    /// centimetres, southern row first — `HeightfieldEncoding::Le16`.
    fn quadrant_le16(&self, qx: u16, qy: u16) -> String {
        let mut bytes = Vec::with_capacity(usize::from(QUAD) * usize::from(QUAD) * 2);
        for y in 0..QUAD {
            for x in 0..QUAD {
                let v = self.at(qx * QUAD + x, qy * QUAD + y);
                let v = i16::try_from(v).unwrap_or(0);
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        mush_atlas::cells::base64_encode(&bytes)
    }
}

// ---------------------------------------------------------------------------
// The packs
// ---------------------------------------------------------------------------

/// The digest placeholder a freshly authored pack carries until
/// [`sign`] replaces it with the digest `mush_content` computes.
const UNSIGNED: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The region pack's id.
const REGION_PACK: &str = "harness.arena_region";
/// The neighborhood pack's id: one anchor room, so the window has an anchor.
const AREA_PACK: &str = "harness.arena_area";
/// The region entry's id, which every other entry names.
const REGION_ENTRY: &str = "region.arena";

/// Compute a pack's own digest and stamp it into its manifest, exactly as the
/// authoring half of `mush_content::load_pack` does (`parse_pack` then
/// `computed_digest`). `verify_sources` at boot then verifies it like any
/// operator pack.
fn sign(json: &str) -> String {
    let parsed = mush_content::parse_pack(json).expect("the authored pack parses");
    let digest = parsed.computed_digest().expect("the pack digests");
    json.replace(UNSIGNED, &digest.to_hex())
}

/// The region pack: one surface region whose background recipe is the arena's
/// ground cover, and four `heightfield` entries carrying the seeded relief.
///
/// **Why the classes here are patches and not a map.** Cell *classes* come from
/// the region's `background` recipe — a seeded hash of `(region, surface,
/// lattice square)` into the mix — so the roads, the woods and the water below
/// are deterministic *patches* at `relief_period_cm` rather than two continuous
/// roads and a river with one ford.
///
/// Cut H recorded that as a gap in the pack route; 16b cut C.1 closed it
/// (`HeightfieldDef::classes`, a run-length-coded grid of authored
/// `TerrainClass` codes) and cut C.2 uses it for the shipped proving ground.
/// This relief stays a *recipe* on purpose: it is the harness's **generated**
/// terrain, reproducible from `TACSIM_SEED` alone and comparable across runs,
/// and its whole job is to be a cost measurement rather than a map anybody
/// fights on. The authored one lives in `crates/tacsim/content/atlas/`.
fn region_pack(relief: &Relief, seed: u64) -> String {
    let lo = ORIGIN_M;
    let hi = ORIGIN_M + i64::from(SIDE) * i64::from(CELL_CM) / 100;
    let mut entries = String::new();
    entries.push_str(&format!(
        r#""{REGION_ENTRY}":{{"kind":"region","def":{{
      "name":"The Arena",
      "kind":"surface","body":"luna",
      "extent":[[{lo},{lo}],[{hi},{hi}]],
      "datum_m":0.0,
      "background":{{
        "surface":"ground",
        "mix":[["grass",22],["woods_light",16],["woods_heavy",9],["road",6],["rubble",4]],
        "relief_cm":300,
        "relief_period_cm":3000,
        "water_cm":-170,
        "seed":{seed}
      }},
      "ground_resolution_cm":{CELL_CM}
    }}}},"#
    ));
    entries.push_str(&format!(
        r#""locality.arena":{{"kind":"locality","def":{{
      "region":"{REGION_ENTRY}","name":"The Arena Floor","kind":"lot",
      "footprint":[[{lo},{lo}],[{hi},{lo}],[{hi},{hi}],[{lo},{hi}]]
    }}}}"#
    ));
    for qy in 0..(SIDE / QUAD) {
        for qx in 0..(SIDE / QUAD) {
            let ox = ORIGIN_M + i64::from(qx) * i64::from(QUAD) * i64::from(CELL_CM) / 100;
            let oy = ORIGIN_M + i64::from(qy) * i64::from(QUAD) * i64::from(CELL_CM) / 100;
            let data = relief.quadrant_le16(qx, qy);
            entries.push_str(&format!(
                r#","heightfield.q{qx}_{qy}":{{"kind":"heightfield","def":{{
      "region":"{REGION_ENTRY}","origin":[{ox},{oy}],
      "resolution_cm":{CELL_CM},"cols":{QUAD},"rows":{QUAD},
      "encoding":"le16","data":"{data}","scale_cm":1,"datum_cm":0
    }}}}"#
            ));
        }
    }
    let json = format!(
        r#"{{"manifest":{{"id":"{REGION_PACK}","version":1,"kind":"region",
   "digest":"{UNSIGNED}","requires":[]}},
   "entries":{{{entries}}}}}"#
    );
    sign(&json)
}

/// The neighborhood pack: the smallest thing that gives the arena an anchor
/// room, because `mush_atlas::window::open` frames a window from the anchor
/// room's neighborhood's region.
///
/// The room's footprint is a disc rather than a rectangle on purpose: the
/// validator's `topology.isolated` rule refuses a room with no exit unless its
/// footprint is an `anchor` — "a landing" — which is exactly what a muster
/// point reached by being placed on the window is.
fn area_pack(region_digest: &str) -> String {
    // A 60 m square in the arena's south-west corner.
    let a = ORIGIN_M + 20;
    let b = ORIGIN_M + 80;
    let (cx, cy) = (a + 25, a + 25);
    let json = format!(
        r#"{{"manifest":{{"id":"{AREA_PACK}","version":1,"kind":"neighborhood",
   "digest":"{UNSIGNED}",
   "requires":[{{"id":"{REGION_PACK}","version":1,"digest":"{region_digest}"}}]}},
 "entries":{{
  "neighborhood.arena":{{"kind":"neighborhood","def":{{
    "region":"{REGION_ENTRY}","locality":"locality.arena",
    "name":"Arena Muster","setting":"ooc","representation":"authored",
    "footprint":[[{a},{a}],[{b},{a}],[{b},{b}],[{a},{b}]],
    "room_budget":{{"target":1,"maximum":2}},
    "hydration":{{"enabled":false,"max_player_sites":0}},
    "resolution_cm":{CELL_CM}
  }}}},
  "room.muster":{{"kind":"room","def":{{
    "name":"Arena Muster Point","surface":"ground",
    "footprint":{{"anchor":{{"point":[{cx},{cy}],"radius_m":15.0}}}},
    "desc":"A graded square of regolith with a painted muster grid.",
    "kind":"outdoor"
  }}}}
 }}}}"#
    );
    sign(&json)
}

// ---------------------------------------------------------------------------
// The world
// ---------------------------------------------------------------------------

/// The **production** roster, assembled with exactly the calls
/// `mush_server::boot::game_modules` makes (`crates/mush-server/src/boot.rs`),
/// read out of `tacsim::composition` so this crate takes no edge on the host.
/// The LLM service is built from `LlmConfig::default()`, whose kill switch is
/// engaged and which has no provider at all — doc 16 §2's W7, "the harness runs
/// with the LLM disabled".
fn production_roster(atlas: AtlasConfig) -> Vec<Box<dyn GameModule>> {
    production_roster_with(atlas, false)
}

/// [`production_roster`] with the game's own boot packs on or off (16b cut
/// J.1): `boot_packs` is `content_module_with`'s second argument, and `true`
/// puts the shipped proving ground in the world's records, which the sight
/// differential walks. Every other lane runs with it off, as it always has.
fn production_roster_with(atlas: AtlasConfig, boot_packs: bool) -> Vec<Box<dyn GameModule>> {
    use tacsim::composition as comp;

    let llm = std::sync::Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let tacsim_cfg = tacsim::fixture::config();
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let mut mods = comp::server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        atlas,
        &onboarding,
        &game,
        comp::recovery_policy(&tacsim_cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        std::sync::Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.extend(comp::sheet_modules(&onboarding, &game));
    mods.push(comp::tacsim_module(tacsim_cfg));
    // `[game].starter_area` off: the harness's own region is the content under
    // test, and the shipped lot would only add rooms the window must not anchor
    // on. The module itself is composed exactly as production composes it.
    mods.push(comp::content_module_with(false, boot_packs).expect("the content module composes"));
    mods
}

/// One command, with the effects it deferred (`Effect::ModuleAsync` for the
/// atlas's off-loop composite).
fn step(
    tw: &mut TestWorld,
    actor: ObjId,
    line: &str,
) -> (Vec<(Audience, OutputEvent)>, Vec<Effect>) {
    tw.run_with_effects(actor, line)
}

fn said(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

/// Run every deferred atlas composite off-loop and re-enter it, exactly as the
/// world task does (`mush_atlas::run_async` then `WorldCommand::ModuleAsyncDone`).
fn finish_composites(tw: &mut TestWorld, actor: ObjId, effects: &[Effect]) {
    let jobs: Vec<(u64, String)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::ModuleAsync {
                module,
                correlation,
                payload,
                ..
            } if module.as_str() == mush_atlas::NS => Some((*correlation, payload.clone())),
            _ => None,
        })
        .collect();
    for (correlation, payload) in jobs {
        let outcome = mush_atlas::run_async(&payload);
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        tw.engine_mut().step(
            Inbound::Apply(WorldCommand::ModuleAsyncDone {
                module: mush_atlas::NS.into(),
                actor,
                correlation,
                payload: outcome,
            }),
            &mut out,
            &mut fx,
        );
    }
}

/// Apply one shelved pack through the atlas's own staff route: plan it, read the
/// fingerprint the plan printed, apply that fingerprint.
fn apply_pack(tw: &mut TestWorld, id: &str) {
    let (events, effects) = step(tw, GOD, &format!("@atlas/pack plan {id}"));
    let text = said(tw, &events, GOD);
    finish_composites(tw, GOD, &effects);
    let fingerprint = text
        .split_whitespace()
        .skip_while(|w| *w != "fingerprint")
        .nth(1)
        .map(|w| w.trim_end_matches('—').trim().to_string())
        .unwrap_or_else(|| panic!("no fingerprint in the plan for {id}:\n{text}"));
    let (events, effects) = step(tw, GOD, &format!("@atlas/pack apply {fingerprint}"));
    let text = said(tw, &events, GOD);
    assert!(text.contains("Applied"), "{id} did not apply:\n{text}");
    assert!(
        !text.contains("STOPPED PART WAY"),
        "{id} stopped part way:\n{text}"
    );
    finish_composites(tw, GOD, &effects);
}

/// Give `obj` the behavior a directly placed object must carry
/// (`mush_sim::custody`'s `sim.participant`), through the generic staff verb.
fn participant(tw: &mut TestWorld, obj: ObjId) {
    let (events, _) = step(
        tw,
        GOD,
        &format!("@behavior/add #{}=sim.participant", obj.0),
    );
    let text = said(tw, &events, GOD);
    assert!(
        !text.to_ascii_lowercase().contains("usage:"),
        "@behavior/add refused #{}: {text}",
        obj.0
    );
}

/// Classes a ground unit may stand on. `Void` and `Wall` are not surfaces,
/// `Water`/`WaterDeep` and `Door` are not somewhere a scatter should drop a
/// tank.
fn standable(class: TerrainClass) -> bool {
    matches!(
        class,
        TerrainClass::Paved
            | TerrainClass::Ground
            | TerrainClass::Grass
            | TerrainClass::Floor
            | TerrainClass::Rubble
            | TerrainClass::Road
            | TerrainClass::Sand
            | TerrainClass::Rock
            | TerrainClass::Platform
            | TerrainClass::WoodsLight
            | TerrainClass::WoodsHeavy
            | TerrainClass::Ice
            | TerrainClass::Regolith
    )
}

/// The relief, the booted world and the open window every measurement in this
/// file starts from.
///
/// One helper rather than two copies: cut A's `Arena::step` is measured against
/// the world-side baseline, and a comparison whose two halves generated different
/// terrain would not be one. The cells are handed over raw — every cell of the
/// 256 by 256 grid, with its class and its elevation — and each measurement makes
/// of them what it needs: the baseline scatters units on the standable ones, and
/// the arena measurement converts the whole grid into [`TerrainCell`]s.
struct Window {
    tw: TestWorld,
    domain: DomainId,
    /// Every cell, row-major, as `(x, y, class, elev_cm)`.
    cells: Vec<(u16, u16, TerrainClass, i16)>,
    /// How many cells of each class the recipe produced.
    classes: BTreeMap<&'static str, u64>,
    /// The relief's two terraces (16b cut U.1), as [`Relief::terraces`].
    terraces: Vec<(u16, u16, u16, i32)>,
    generate_ms: f64,
    boot_ms: f64,
    apply_ms: f64,
    open_ms: f64,
}

/// Generate the relief, boot the production roster over it, apply both packs and
/// open the 256 by 256 window, answering the world and the cells.
fn open_window(seed: u64) -> Window {
    let started = Instant::now();
    let relief = Relief::generate(seed);
    let terraces = relief.terraces.clone();
    let region_json = region_pack(&relief, seed);
    let region_digest = mush_content::load_pack(&region_json)
        .expect("the region pack verifies")
        .manifest()
        .digest()
        .to_hex();
    let area_json = area_pack(&region_digest);
    let atlas = AtlasConfig {
        // The `[atlas].pack_paths` route: the host reads the bytes, the module
        // shelves them. Nothing is written to disk.
        packs: vec![
            PackSource {
                name: format!("<harness {REGION_PACK}>"),
                bytes: region_json,
            },
            PackSource {
                name: format!("<harness {AREA_PACK}>"),
                bytes: area_json,
            },
        ],
        ..AtlasConfig::default()
    };
    let generate_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let boot_at = Instant::now();
    let mut tw = TestWorld::with_modules(production_roster(atlas));
    tw.set_now(1_700_000_000);
    let boot_ms = boot_at.elapsed().as_secs_f64() * 1_000.0;

    let apply_at = Instant::now();
    apply_pack(&mut tw, REGION_PACK);
    apply_pack(&mut tw, AREA_PACK);
    let apply_ms = apply_at.elapsed().as_secs_f64() * 1_000.0;

    let lo = ORIGIN_M;
    let hi = ORIGIN_M + i64::from(SIDE) * i64::from(CELL_CM) / 100;
    let open_at = Instant::now();
    let (events, effects) = step(
        &mut tw,
        GOD,
        &format!("@atlas/window open {lo},{lo},{hi},{hi}={CELL_CM}"),
    );
    let text = said(&tw, &events, GOD);
    assert!(
        text.contains("opens; its map is being drawn"),
        "the window did not open:\n{text}"
    );
    finish_composites(&mut tw, GOD, &effects);
    let open_ms = open_at.elapsed().as_secs_f64() * 1_000.0;

    let domain = {
        let st = tw
            .world()
            .module_state::<AtlasState>()
            .expect("the atlas state");
        st.windows
            .open
            .keys()
            .max()
            .copied()
            .map(DomainId)
            .expect("a window entry")
    };

    let (grid_w, grid_h, cells, classes) = {
        let st = tw
            .world()
            .module_state::<AtlasState>()
            .expect("the atlas state");
        let entry = st
            .windows
            .open
            .get(&domain.0)
            .expect("the window is still open");
        let composite = entry
            .composite
            .as_ref()
            .expect("the composite landed before the scatter");
        let surface = composite
            .surfaces
            .first()
            .expect("the ground surface composited");
        let mut all: Vec<(u16, u16, TerrainClass, i16)> = Vec::new();
        let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
        for (x, y, cell) in surface.cells.iter() {
            *counts.entry(cell.class.label()).or_default() += 1;
            all.push((x, y, cell.class, cell.elev_cm));
        }
        (surface.grid.width, surface.grid.height, all, counts)
    };
    assert_eq!(
        (grid_w, grid_h),
        (SIDE, SIDE),
        "the window did not frame one 256 by 256 grid"
    );
    Window {
        tw,
        domain,
        cells,
        classes,
        terraces,
        generate_ms,
        boot_ms,
        apply_ms,
        open_ms,
    }
}

/// Where `TACSIM_JSON` lands. `cargo test` runs an integration test with the
/// **package** directory as its working directory, so a relative report path is
/// resolved against the workspace root instead — the same two-levels-up walk
/// from `CARGO_MANIFEST_DIR` the contract gates use. Without this, `harness.json`
/// would land in `crates/tacsim/` and the CI job's artifact step would find
/// nothing at the workspace root it uploads from.
fn report_path(given: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(given);
    if path.is_absolute() {
        return path;
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(|root| root.join(&path))
        .unwrap_or(path)
}

/// **Resident memory, kibibytes, on every platform** (16b cut U.2, 16c `T-36`):
/// the process's physical memory as the `memory-stats` crate reads it from the
/// OS — `GetProcessMemoryInfo`'s working set on Windows, the resident set on
/// Linux and macOS — through a safe API, so the harness stays free of `unsafe`.
///
/// Until 16b cut U.2 this read `/proc/self/status` and answered nothing on any
/// other host, so the Windows box the baselines are taken on printed `absent`
/// and memory was compared nowhere. It is still **absent** when the crate
/// answers `None`, never faked (16b §9's `H` row); [`vmrss_kib`] keeps the
/// Linux figure beside it as a second reading, never in its place.
fn resident_kib() -> Option<u64> {
    memory_stats::memory_stats()
        .map(|stats| u64::try_from(stats.physical_mem).unwrap_or(u64::MAX) / 1_024)
}

/// **Linux's own `VmRSS`**, kibibytes, off `/proc/self/status` — the reading
/// every harness line printed before 16b cut U.2, kept as a second figure beside
/// [`resident_kib`] (`vmrss_*_kib=`). `None` on every other platform.
fn vmrss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse::<u64>().ok())
}

/// One memory reading as a report word: the number, or `absent`.
fn kib_word(kib: Option<u64>) -> String {
    kib.map_or("absent".to_string(), |k| k.to_string())
}

/// ` vmrss_start_kib=… vmrss_end_kib=…` where Linux answered either reading,
/// and nothing at all where it answered neither — the second figure is printed
/// beside the first only on a host that has one.
fn vmrss_words(start: Option<u64>, end: Option<u64>) -> String {
    if start.is_none() && end.is_none() {
        return String::new();
    }
    format!(
        " vmrss_start_kib={} vmrss_end_kib={}",
        kib_word(start),
        kib_word(end)
    )
}

// ---------------------------------------------------------------------------
// Quantiles
// ---------------------------------------------------------------------------

/// The `q`-th per-mille quantile of an already-sorted slice, nearest-rank.
fn quantile(sorted: &[f64], per_mille: u32) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let last = sorted.len() - 1;
    let rank = (usize::try_from(per_mille).unwrap_or(0) * last) / 1_000;
    sorted.get(rank.min(last)).copied().unwrap_or(0.0)
}

/// One tempo's timings. Before cut A there is one tempo, `current`, at the
/// ground cadence.
struct Tempo {
    name: &'static str,
    period_ms: u32,
    ms: Vec<f64>,
}

impl Tempo {
    fn stats(&self) -> (f64, f64, f64, f64) {
        let mut sorted = self.ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        (
            quantile(&sorted, 500),
            quantile(&sorted, 950),
            quantile(&sorted, 990),
            sorted.last().copied().unwrap_or(0.0),
        )
    }
}

fn num(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64((v * 1_000.0).round() / 1_000.0)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

// ---------------------------------------------------------------------------
// The measurement
// ---------------------------------------------------------------------------

#[test]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
#[allow(clippy::too_many_lines)]
fn tacsim_harness_reports_the_ground_pulse_baseline() {
    let knobs = Knobs::from_env();
    assert_eq!(
        CELL_CM,
        mush_config::config::TacsimCfg::default().ground_cell_cm,
        "the harness's cell size is no longer `[tacsim].ground_cell_cm`"
    );
    let rss_start_kib = resident_kib();
    let vmrss_start_kib = vmrss_kib();

    // ---- the relief, the world and the window ----
    let Window {
        mut tw,
        domain,
        cells: all_cells,
        classes: class_counts,
        terraces,
        generate_ms,
        boot_ms,
        apply_ms,
        open_ms,
    } = open_window(knobs.seed);

    // ---- the cells the scatter may use ----
    let cells: Vec<(u16, u16, i32)> = all_cells
        .iter()
        .filter(|(_, _, class, _)| standable(*class))
        .map(|(x, y, _, elev_cm)| (*x, *y, i32::from(*elev_cm)))
        .collect();
    assert!(
        !cells.is_empty(),
        "the relief left no cell a unit could stand on"
    );

    // ---- the units ----
    let mut rng = ChaCha8Rng::seed_from_u64(knobs.seed ^ 0x5ca1e);
    let origin_cm = ORIGIN_M * 100;
    let place_at = Instant::now();
    let mut placed = 0_u32;
    let mut refused: Vec<String> = Vec::new();
    for n in 0..knobs.units {
        let name = format!("harness-unit-{n}");
        let obj = tw
            .world_mut()
            .create(ObjKind::Thing, &name, GOD, Some(mush_core::LIMBO))
            .expect("a unit object is created");
        participant(&mut tw, obj);
        let pick = rng.random_range(0..cells.len());
        let (cx, cy, elev_cm) = cells.get(pick).copied().unwrap_or((0, 0, 0));
        let x_cm = origin_cm + i64::from(cx) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2;
        let y_cm = origin_cm + i64::from(cy) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2;
        let heading = rng.random_range(0..360_u32) * 1_000;
        let (events, _) = step(
            &mut tw,
            GOD,
            &format!(
                "@spatial/place/domain #{}={},{{\"x_cm\":{x_cm},\"y_cm\":{y_cm},\"z_cm\":{elev_cm},\"heading_mdeg\":{heading}}}",
                obj.0, domain.0
            ),
        );
        let text = said(&tw, &events, GOD);
        if text.contains("Placed ") {
            placed += 1;
        } else if refused.len() < 3 {
            refused.push(text);
        }
    }
    let place_ms = place_at.elapsed().as_secs_f64() * 1_000.0;
    assert!(
        refused.is_empty(),
        "a placement was refused: {}",
        refused.join(" | ")
    );

    // ---- the fake sessions ----
    let mut sessions = 0_u32;
    let mut placed_sessions = 0_u32;
    for n in 0..knobs.fighting {
        let who = tw.spawn_player(&format!("HarnessPilot{n}"));
        participant(&mut tw, who);
        sessions += 1;
        let pick = rng.random_range(0..cells.len());
        let (cx, cy, elev_cm) = cells.get(pick).copied().unwrap_or((0, 0, 0));
        let x_cm = origin_cm + i64::from(cx) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2;
        let y_cm = origin_cm + i64::from(cy) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2;
        let (events, _) = step(
            &mut tw,
            GOD,
            &format!(
                "@spatial/place/domain #{}={},{{\"x_cm\":{x_cm},\"y_cm\":{y_cm},\"z_cm\":{elev_cm}}}",
                who.0, domain.0
            ),
        );
        if said(&tw, &events, GOD).contains("Placed ") {
            placed_sessions += 1;
        }
    }

    // ---- tactical cadence ----
    //
    // The world-side pulse only *decides* cadence (`mush_sim::tick::
    // plan_maintenance`): it promotes, arms or stands a domain down and moves
    // nothing. Asking for promotion is what makes it take its working branch
    // rather than its no-op one, so the request is made and its answer reported.
    // At the harness's 300 units it is refused — `MAX_UNITS_PER_DOMAIN` is 64 —
    // and the pulse then re-measures the cap and pages on every fire, which is
    // itself part of the baseline.
    let (events, _) = step(&mut tw, GOD, &format!("@domain/promote {}", domain.0));
    let promotion = said(&tw, &events, GOD).replace('\n', " | ");

    // ---- the pulses ----
    let period_ms = i64::from(tacsim::config::DEFAULT_GROUND_PULSE_MS);
    let mut tempo = Tempo {
        name: "current",
        period_ms: tacsim::config::DEFAULT_GROUND_PULSE_MS,
        ms: Vec::with_capacity(usize::try_from(knobs.pulses).unwrap_or(0)),
    };
    let mut events_per_pulse: Vec<f64> = Vec::new();
    let mut now_ms = 1_700_000_000_i64 * 1_000;
    for _ in 0..knobs.pulses {
        now_ms += period_ms;
        let at = Instant::now();
        let out = tw.fire_due(now_ms);
        tempo.ms.push(at.elapsed().as_secs_f64() * 1_000.0);
        #[allow(clippy::cast_precision_loss)]
        events_per_pulse.push(out.len() as f64);
    }

    // What the sixty fires actually left the domain in, read back through the
    // staff lane. At 300 units this says `passive`: `MAX_UNITS_PER_DOMAIN` is 64,
    // so the pulse re-measures the cap and refuses on every fire instead of
    // applying the promotion — the baseline fact cut A is measured against.
    let (events, _) = step(&mut tw, GOD, &format!("@domain/status {}", domain.0));
    let status = said(&tw, &events, GOD).replace('\n', " | ");

    // **The control.** `TestWorld::fire_due` fires every scheduled entry due at
    // that instant, not the domain pulse alone: the journey tick, the weather,
    // the economy and faction clocks and the rest of the coarse roster all land
    // on the same second. Closing the window retires the domain and releases its
    // participants, so the same number of fires with nothing placed anywhere
    // measures what the rest of the roster costs — and the difference is the
    // domain pass's own share. Without this number the tempo above cannot be read.
    let (events, effects) = step(&mut tw, GOD, &format!("@atlas/window close {}", domain.0));
    let closed = said(&tw, &events, GOD).replace('\n', " | ");
    finish_composites(&mut tw, GOD, &effects);
    let mut control_ms: Vec<f64> = Vec::with_capacity(usize::try_from(knobs.pulses).unwrap_or(0));
    for _ in 0..knobs.pulses {
        now_ms += period_ms;
        let at = Instant::now();
        let _ = tw.fire_due(now_ms);
        control_ms.push(at.elapsed().as_secs_f64() * 1_000.0);
    }
    let control = Tempo {
        name: "no_domain_control",
        period_ms: tacsim::config::DEFAULT_GROUND_PULSE_MS,
        ms: control_ms,
    };
    let (c50, c95, c99, cmax) = control.stats();

    let rss_end_kib = resident_kib();
    let vmrss_end_kib = vmrss_kib();
    let (p50, p95, p99, max) = tempo.stats();
    #[allow(clippy::cast_precision_loss)]
    let events_mean = if events_per_pulse.is_empty() {
        0.0
    } else {
        events_per_pulse.iter().sum::<f64>() / events_per_pulse.len() as f64
    };
    let events_max = events_per_pulse
        .iter()
        .copied()
        .fold(0.0_f64, |a, b| if b > a { b } else { a });

    // **16b cut U.3** (16c `T-37`): this lane reports no per-pulse counters. It
    // printed five hard-coded zeros — traces, cell probes, path queries, share
    // operations, ledger bytes — and asserted three of them under `TACSIM_ASSERT`,
    // which was a green check on a constant: the world-side pulse traces nothing,
    // searches nothing and streams no ledger, and the arena lanes below are where
    // those counters are real. What this lane measures is the passive domain's
    // pulse, and its p99 is what it asserts.

    // ---- the report ----
    println!(
        "harness_world: units={placed}/{} fighting={sessions} placed_sessions={placed_sessions}",
        knobs.units
    );
    println!(
        "harness_relief: side={SIDE} cell_cm={CELL_CM} arena_m={} quadrants={} seed={} elev_step_cm={ELEV_STEP_CM} terraces={}",
        i64::from(SIDE) * i64::from(CELL_CM) / 100,
        (SIDE / QUAD) * (SIDE / QUAD),
        knobs.seed,
        terraces
            .iter()
            .map(|(x, y, r, rise)| format!("({x},{y})r{r}+{rise}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let classes = class_counts
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!("harness_classes: {classes}");
    println!("harness_promotion: {promotion}");
    println!(
        "harness_domain_after: unit_cap={} {status}",
        mush_sim::tick::MAX_UNITS_PER_DOMAIN
    );
    println!(
        "harness_orders: none — `+heading`/`+thrust` need a seat holding the `movement` control channel and stage an authoritative action, and cut T retires them (D-16-20); this is the static pulse"
    );
    println!(
        "harness_setup_ms: generate={generate_ms:.1} boot={boot_ms:.1} pack_apply={apply_ms:.1} window_open={open_ms:.1} place={place_ms:.1}"
    );
    println!(
        "harness_tempo_{}: period_ms={} pulses={} p50={p50:.3} p95={p95:.3} p99={p99:.3} max={max:.3}",
        tempo.name,
        tempo.period_ms,
        tempo.ms.len()
    );
    println!(
        "harness_control_{}: period_ms={} pulses={} p50={c50:.3} p95={c95:.3} p99={c99:.3} max={cmax:.3} ({closed})",
        control.name,
        control.period_ms,
        control.ms.len()
    );
    println!("harness_events: per_pulse_mean={events_mean:.2} per_pulse_max={events_max:.0}");
    // 16b cut U.2 (16c `T-36`): resident memory on every platform, with
    // Linux's own `VmRSS` beside it.
    println!(
        "harness_memory: rss_start_kib={} rss_end_kib={}{}",
        kib_word(rss_start_kib),
        kib_word(rss_end_kib),
        vmrss_words(vmrss_start_kib, vmrss_end_kib)
    );
    println!(
        "harness_thresholds: assert={} engaged_p99_ms={} (the passive domain's pulse; the per-pulse counter bounds are the arena lanes')",
        knobs.assert, knobs.engaged_p99_ms
    );

    let report = serde_json::json!({
        "world": {
            "units_requested": knobs.units,
            "units_placed": placed,
            "fighting": sessions,
            "sessions_placed": placed_sessions,
            "cell_cm": CELL_CM,
            "pulse_ms": tempo.period_ms,
            "pulses": tempo.ms.len(),
            "seed": knobs.seed,
            "side_cells": SIDE,
            "arena_m": i64::from(SIDE) * i64::from(CELL_CM) / 100,
        },
        "classes": class_counts,
        "promotion": promotion,
        "domain_after_pulses": status,
        "domain_unit_cap": mush_sim::tick::MAX_UNITS_PER_DOMAIN,
        "orders": serde_json::Value::Null,
        "setup_ms": {
            "generate": num(generate_ms),
            "boot": num(boot_ms),
            "pack_apply": num(apply_ms),
            "window_open": num(open_ms),
            "place": num(place_ms),
        },
        "tempos": [{
            "name": tempo.name,
            "period_ms": tempo.period_ms,
            "pulses": tempo.ms.len(),
            "p50_ms": num(p50),
            "p95_ms": num(p95),
            "p99_ms": num(p99),
            "max_ms": num(max),
        }],
        "control": {
            "name": control.name,
            "what": "the same fires with the window closed and nothing placed: what the rest of the coarse tick roster costs",
            "closed": closed,
            "pulses": control.ms.len(),
            "p50_ms": num(c50),
            "p95_ms": num(c95),
            "p99_ms": num(c99),
            "max_ms": num(cmax),
        },
        "events": { "per_pulse_mean": num(events_mean), "per_pulse_max": events_max },
        "memory": {
            "rss_start_kib": rss_start_kib,
            "rss_end_kib": rss_end_kib,
            "vmrss_start_kib": vmrss_start_kib,
            "vmrss_end_kib": vmrss_end_kib,
        },
        // 16b cut U.3: no `counters` object — see the note above the report.
        "thresholds": {
            "assert": knobs.assert,
            "engaged_p99_ms": knobs.engaged_p99_ms,
        },
    });
    let path = report_path(&knobs.json);
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).expect("the report directory is writable");
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&report).expect("the report encodes"),
    )
    .expect("the report is written");
    println!("harness_json: {}", path.display());

    if knobs.assert {
        // The passive domain's pulse, measured: the one bound this lane has.
        assert!(
            p99 <= f64::from(knobs.engaged_p99_ms),
            "per-pulse p99 {p99:.3} ms of the passive domain is past harness.engaged_p99_ms {}",
            knobs.engaged_p99_ms
        );
    }
}

// ---------------------------------------------------------------------------
// Cut A: the arena's own pulse, at the same scale
// ---------------------------------------------------------------------------

/// **The `tracked` movement cost of each terrain class, for the harness only.**
///
/// Doc 16 §3.2 says the arena's terrain is *the existing 18-class cell grid*, and
/// `tacsim::arena::state::TerrainCell` carries `move_cost` as data so that a test
/// can lay one down with no atlas at all — `0` is impassable and the terrain
/// factor is `100 / move_cost`. The generated window has no authored cost table
/// (the proving ground's is `content/arenas/proving_ground.json`'s), so this one
/// is **the harness's own**, chosen to be plausible and to put the water and the
/// walls out of bounds so that units really do run into edges and into each
/// other, and it is named as a test-local table rather than left to look like a
/// shipped decision.
///
/// It is one table for every unit on the field: the cohort's modes differ in
/// kinematics and in slope limit (16b cut U.1), and not in class costs, because
/// 16c `B-58`'s per-locomotion cost layer is not built — the arena has one
/// `move_cost` a cell, whoever crosses it.
fn harness_move_cost(class: TerrainClass) -> u16 {
    match class {
        TerrainClass::Ground
        | TerrainClass::Grass
        | TerrainClass::Paved
        | TerrainClass::Sand
        | TerrainClass::Regolith => 100,
        TerrainClass::Road => 80,
        TerrainClass::WoodsLight => 150,
        TerrainClass::WoodsHeavy => 250,
        TerrainClass::Rubble | TerrainClass::Rock | TerrainClass::Ice => 200,
        // Impassable to a tank: water of either depth, a wall, a doorway, an
        // interior floor or platform, and the cells that are not a surface at all.
        TerrainClass::Water
        | TerrainClass::WaterDeep
        | TerrainClass::Wall
        | TerrainClass::Void
        | TerrainClass::Door
        | TerrainClass::Floor
        | TerrainClass::Platform => 0,
    }
}

/// **Which reach a measurement fields its sights at** (16b cut U.1; 16c `T-34`).
///
/// One body ([`measure_arena`]) and two answers, because the owner's choice
/// between a shorter authored reach and larger bounds (see [`ARENA_S_P99_MS`])
/// cannot be read off a harness that only ever measures one of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// **The CI cohort**: every sight scaled by [`sensor_cells`] over the
    /// authored visual's 60 cells and every 30-cell mesh cut to [`share_cells`]
    /// — the field the lens and the per-pulse bounds are asserted on.
    Ci,
    /// **The authored reaches**, exactly as the catalog fields them: report-only
    /// until cut J's `Q26` settles the bound they are measured against.
    Authored,
}

impl Reach {
    /// The word every `harness_arena_*` line and the JSON's `"cohort"` carry.
    const fn word(self) -> &'static str {
        match self {
            Reach::Ci => "ci",
            Reach::Authored => "authored",
        }
    }
}

/// **The cohort table** (16b cut U.1; 16c `T-35`): what the field is made of,
/// eight entries of the fielded catalog, read by unit number.
///
/// Unit `n` fields `COHORT[(n / 2) % 8]`. The team is `n % 2`, so `n % 8` would
/// put every jammer on red and every filter on blue; the halved index gives both
/// sides every type in the same proportions — per sixteen units, each side has
/// three line tanks, the laser tank, the biped, the thermal tank, the ECM tank
/// and the ECCM tank. The fighting line is fielded from the same table, so the
/// units that fight are the mix and not only line tanks.
const COHORT: [&str; 8] = [
    content::LINE_TANK,
    content::LINE_TANK,
    content::LINE_TANK,
    content::LINE_TANK_LASER,
    content::BIPED_MK1,
    content::LINE_TANK_THERMAL,
    content::LINE_TANK_ECM,
    content::LINE_TANK_ECCM,
];

/// The six distinct entries of [`COHORT`], in the order the cohort line reports
/// them, with the word each is reported under.
const COHORT_ENTRIES: [(&str, &str); 6] = [
    (content::LINE_TANK, "line_tank"),
    (content::LINE_TANK_LASER, "laser"),
    (content::BIPED_MK1, "biped"),
    (content::LINE_TANK_THERMAL, "thermal"),
    (content::LINE_TANK_ECM, "ecm"),
    (content::LINE_TANK_ECCM, "eccm"),
];

/// **The fielded catalog, off the booted world** (16b cut U.1): the one
/// `tacsim::TacsimStore::install_packs` built with `Catalog::build_fielded` —
/// the compiled-in content and the five extension units, whatever packs the
/// content module carries — which is the catalog `match_::start` checks units
/// out against. Asserted to carry every [`COHORT`] entry, spec and binding both.
fn fielded_catalog(tw: &TestWorld) -> Catalog {
    let catalog = tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .map(|slot| slot.definitions().clone())
        .expect("the tacsim store is booted");
    for (entry, _) in COHORT_ENTRIES {
        let key = CanonicalId::new(entry).expect("a canonical entry id");
        assert!(
            catalog.specs.contains_key(&key) && catalog.bindings.contains_key(&key),
            "the booted catalog does not field {entry}: the harness cohort is not the game's"
        );
    }
    catalog
}

/// The authored visual sight's reach, cells (`content/sensors/visual_mk1.json`),
/// which is what [`sensor_cells`] is a fraction of.
const AUTHORED_SIGHT_CELLS: u32 = 60;

/// The authored mesh reach, cells, that [`share_cells`] replaces.
const AUTHORED_SHARE_CELLS: u16 = 30;

/// **The CI cohort's sight, as a numerator over the authored visual's 60
/// cells**, overridable with `TACSIM_SENSOR_CELLS` (16b cut U.1).
///
/// Under [`Reach::Ci`] every unit's `max_range_cells` is scaled to `authored ×
/// sensor_cells() / 60`, floored and at least 1: the visual sight's 60 is 24,
/// the thermal's 45 is 18, and a one-cell floor row stays 1. The field keeps
/// the catalog's proportions — a thermal still sees three quarters of what a
/// visual sees — at a reach the per-pulse bounds were sized for.
///
/// Why a reduced reach at all, and why 24 (the arithmetic cut S.1 recorded):
/// doc 16 §2 W1 is *sensor ranges are short and bounded*, and W4 bounds traces
/// and cell probes per pulse by `harness.max_traces_per_pulse` (4,000). 300 units
/// on 256 by 256 cells, about 51,000 of them passable, is a density near 0.006
/// units a cell, and a pass traces every candidate inside its reach, so the
/// traces a second are about `300 × π r² × 0.006`: near 4,000 at `r ≈ 30` and
/// about 17,000 at the authored 60. The authored figure is not hidden: it is
/// [`Reach::Authored`]'s run, beside this one.
fn sensor_cells() -> u16 {
    u16::try_from(env_u64("TACSIM_SENSOR_CELLS", u64::from(SENSOR_CELLS))).unwrap_or(SENSOR_CELLS)
}

/// The default of [`sensor_cells`].
const SENSOR_CELLS: u16 = 24;

/// **The CI cohort's mesh reach**, cells, overridable with `TACSIM_SHARE_CELLS`
/// (16b cut U.1): under [`Reach::Ci`] every sight whose authored
/// `share_range_cells` is 30 shares at this instead, and a row that authors no
/// mesh (0) keeps none.
///
/// 16b §9 pins `comms.share_range_cells` at 30 and the shipped sights author 30;
/// this is 8, and the reason is the arithmetic [`sensor_cells`] records: doc 16
/// §3.6 step 5's mesh is one operation per `(sharer, teammate)` pair inside the
/// reach, `harness.max_share_ops_per_pulse` is 256, and at this density the
/// authored 30 is about 2,400 operations and 2,400 further traces a second.
fn share_cells() -> u16 {
    u16::try_from(env_u64("TACSIM_SHARE_CELLS", u64::from(SHARE_CELLS))).unwrap_or(SHARE_CELLS)
}

/// The default of [`share_cells`].
const SHARE_CELLS: u16 = 8;

/// Scale one checked-out unit's sight to the CI cohort's reach ([`Reach::Ci`]):
/// the rule [`sensor_cells`] and [`share_cells`] state.
fn scale_to_ci(unit: &mut tacsim::arena::state::CustodyUnit) {
    let Some(sensor) = unit.sensor.as_mut() else {
        return;
    };
    let scaled = (u32::from(sensor.max_range_cells) * u32::from(sensor_cells())
        / AUTHORED_SIGHT_CELLS)
        .max(1);
    sensor.max_range_cells = u16::try_from(scaled).unwrap_or(u16::MAX);
    if sensor.share_range_cells == AUTHORED_SHARE_CELLS {
        sensor.share_range_cells = share_cells();
    }
}

/// **The harness's own patrol graph** (16b cut N.1; doc 16 §6: *the arena pack's
/// waypoint graph*).
///
/// The shipped arena authors its graph (`content/arenas/proving_ground.json`'s two
/// roads and four spurs); this window is *generated relief* and has no author, so
/// the harness generates one the way it generates everything else here — from the
/// ground, deterministically, with no file.
///
/// **A coarse grid.** Nodes on a [`WAYPOINT_PITCH`]-cell pitch, offset half a pitch
/// so none sits on the window's edge, each **snapped to the nearest passable cell**
/// inside a half-pitch box (a node in the middle of the lake is dropped, and its
/// edges with it). Edges join each node to its east and north neighbours, which
/// makes the graph a connected lattice wherever the ground allows one. At the
/// shipped 256-cell window and a 16-cell pitch that is 16 by 16 = 256 nodes —
/// exactly `MAX_WAYPOINT_NODES` — and at most 480 edges against a bound of 1,024.
///
/// The pitch is the number that matters: it sets how far a patrolling unit walks
/// between decisions, and therefore how long each A\* search is. Sixteen cells is
/// 160 m, about eleven seconds at a line tank's ceiling.
fn harness_waypoints(
    grid: &[tacsim::arena::state::TerrainCell],
) -> tacsim::arena::state::Waypoints {
    let passable_at = |x: u16, y: u16| -> bool {
        grid.get(usize::from(y) * usize::from(SIDE) + usize::from(x))
            .is_some_and(|cell| cell.move_cost != 0)
    };
    let half = WAYPOINT_PITCH / 2;
    let mut nodes: Vec<tacsim::arena::state::Waypoint> = Vec::new();
    let mut present: Vec<Vec<bool>> = Vec::new();
    let steps = SIDE / WAYPOINT_PITCH;
    for j in 0..steps {
        let mut row = Vec::new();
        for i in 0..steps {
            let (cx, cy) = (i * WAYPOINT_PITCH + half, j * WAYPOINT_PITCH + half);
            // The nearest passable cell inside the half-pitch box, by Chebyshev
            // rings out from the node's own cell: deterministic, and bounded by the
            // pitch.
            let mut found: Option<(u16, u16)> = None;
            'rings: for r in 0..=half {
                for dy in -i32::from(r)..=i32::from(r) {
                    for dx in -i32::from(r)..=i32::from(r) {
                        if dx.abs() != i32::from(r) && dy.abs() != i32::from(r) {
                            continue;
                        }
                        let (Ok(x), Ok(y)) = (
                            u16::try_from(i32::from(cx) + dx),
                            u16::try_from(i32::from(cy) + dy),
                        ) else {
                            continue;
                        };
                        if x < SIDE && y < SIDE && passable_at(x, y) {
                            found = Some((x, y));
                            break 'rings;
                        }
                    }
                }
            }
            row.push(found.is_some());
            if let Some((x, y)) = found {
                nodes.push(tacsim::arena::state::Waypoint {
                    id: smol_str::SmolStr::new(format!("g{i}_{j}")),
                    x,
                    y,
                });
            }
        }
        present.push(row);
    }
    let mut edges: Vec<(smol_str::SmolStr, smol_str::SmolStr)> = Vec::new();
    let here = |i: u16, j: u16| -> bool {
        present
            .get(usize::from(j))
            .and_then(|row| row.get(usize::from(i)))
            .copied()
            .unwrap_or(false)
    };
    for j in 0..steps {
        for i in 0..steps {
            if !here(i, j) {
                continue;
            }
            if i + 1 < steps && here(i + 1, j) {
                edges.push((
                    smol_str::SmolStr::new(format!("g{i}_{j}")),
                    smol_str::SmolStr::new(format!("g{}_{j}", i + 1)),
                ));
            }
            if j + 1 < steps && here(i, j + 1) {
                edges.push((
                    smol_str::SmolStr::new(format!("g{i}_{j}")),
                    smol_str::SmolStr::new(format!("g{i}_{}", j + 1)),
                ));
            }
        }
    }
    tacsim::arena::state::Waypoints::new(nodes, &edges)
}

/// The harness patrol graph's node pitch, in cells (see [`harness_waypoints`]).
const WAYPOINT_PITCH: u16 = 16;

/// How far each fighting line stands from the window's centre, in cells: the two
/// lines are twice this apart.
const LINE_GAP_CELLS: u16 = 4;

/// How far apart neighbours stand down a fighting line, in cells.
const LINE_PITCH_CELLS: u16 = 3;

/// **The window as the arena's ground** (16b cut A.2; one helper since 16b cut
/// U.1, so the arena measurement and the cost run convert it once): every cell
/// of the composited grid with its class, its elevation and
/// [`harness_move_cost`], and the passable cells as `(x, y, elev_cm)`.
fn arena_ground(
    all_cells: &[(u16, u16, TerrainClass, i16)],
) -> (Vec<tacsim::arena::state::TerrainCell>, Vec<(u16, u16, i16)>) {
    use tacsim::arena::state::TerrainCell;
    let mut grid = vec![
        TerrainCell {
            class: TerrainClass::Void,
            elev_cm: 0,
            move_cost: 0,
        };
        usize::from(SIDE) * usize::from(SIDE)
    ];
    for (x, y, class, elev_cm) in all_cells {
        let at = usize::from(*y) * usize::from(SIDE) + usize::from(*x);
        if let Some(cell) = grid.get_mut(at) {
            *cell = TerrainCell {
                class: *class,
                elev_cm: *elev_cm,
                move_cost: harness_move_cost(*class),
            };
        }
    }
    let passable: Vec<(u16, u16, i16)> = grid
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.move_cost != 0)
        .filter_map(|(at, cell)| {
            let x = u16::try_from(at % usize::from(SIDE)).ok()?;
            let y = u16::try_from(at / usize::from(SIDE)).ok()?;
            Some((x, y, cell.elev_cm))
        })
        .collect();
    (grid, passable)
}

/// **The arena definition over the window's ground**: the grid, an entry zone
/// sixteen columns deep on each side, and [`harness_waypoints`]' patrol graph.
fn arena_definition(
    grid: Vec<tacsim::arena::state::TerrainCell>,
) -> tacsim::arena::state::ArenaDefinition {
    use tacsim::arena::state::{ArenaDefinition, ArenaTerrain, EntryZone, Team};
    let waypoints = harness_waypoints(&grid);
    ArenaDefinition {
        id: smol_str::SmolStr::new("tacsim.arena.harness"),
        terrain: ArenaTerrain::new(SIDE, SIDE, CELL_CM, grid).expect("the arena window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 15,
                y1: SIDE - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: SIDE - 16,
                y0: 0,
                x1: SIDE - 1,
                y1: SIDE - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints,
    }
}

/// **The relief's refusing edges** (16b cut U.1): the orthogonal edges between
/// two passable cells steeper than 45 % and than 70 % — the tracked and the
/// legged rows' limits — measured with the arena's own
/// `tacsim::arena::slope::steepness_pct`, the number `too_steep` compares.
fn refusing_edges(grid: &[tacsim::arena::state::TerrainCell]) -> (u64, u64) {
    use mush_atlas::physics::StepKind;
    let at = |x: u16, y: u16| grid.get(usize::from(y) * usize::from(SIDE) + usize::from(x));
    let (mut over_45, mut over_70) = (0_u64, 0_u64);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let Some(here) = at(x, y).filter(|cell| cell.passable()) else {
                continue;
            };
            for (nx, ny) in [(x.saturating_add(1), y), (x, y.saturating_add(1))] {
                if (nx, ny) == (x, y) || nx >= SIDE || ny >= SIDE {
                    continue;
                }
                let Some(there) = at(nx, ny).filter(|cell| cell.passable()) else {
                    continue;
                };
                let steep = tacsim::arena::slope::steepness_pct(
                    here,
                    there,
                    StepKind::Orthogonal,
                    u32::from(CELL_CM),
                );
                over_45 += u64::from(tacsim::arena::slope::too_steep(Some(45), steep));
                over_70 += u64::from(tacsim::arena::slope::too_steep(Some(70), steep));
            }
        }
    }
    (over_45, over_70)
}

/// **The field** (16b cut N.1's arrangement; 16b cut U.1's cohort): the
/// `TACSIM_FIGHTING` units nose to nose across the middle of the window, the
/// rest scattered over `relief` — the window's passable cells — and every one
/// of them a unit of the fielded catalog checked out the way `match_::start`
/// checks one out, handing back the units and the fighting units' seats.
///
/// **The shapes are the game's** (16c `T-35`). Every unit is
/// `tacsim::match_::custody_unit` over `definitions::to_tactical_unit` of its
/// [`COHORT`] entry's spec and binding — the tracked hulls, the legged biped,
/// the laser, the thermal sight, the jamming pod and the filter, with their
/// modes, slope limits, silhouettes, facet tables and weapons as authored — on
/// the reflex controller, gunnery 1. The harness-local tank, sight, weapon and
/// profile this replaced are gone: a harness whose units had no locomotion ran
/// the slope limit, the shoulder rule and the per-mode A\* cost in none of 300.
/// Under [`Reach::Ci`] each sight is then scaled ([`scale_to_ci`]).
///
/// Doc 16 §1 A2 is *300 units placed and moving, 24 of them fighting*, and with
/// the reflex controller driving the field that has to be arranged rather than
/// hoped for: the fighting units stand in two lines [`LINE_GAP_CELLS`] either
/// side of the centre, even units (red) west and odd (blue) east,
/// [`LINE_PITCH_CELLS`] apart down each line from row `middle − 18`, each wanted
/// cell answered with the nearest passable one. The pitch is not cosmetic: the
/// share mesh is one operation per `(sharer, teammate)` pair, so twelve
/// teammates in one blob is 132 operations a pass on its own. The seats are
/// **frame sinks**, not pilots: doc 16 §1 A2 asks for connected sessions
/// receiving output and the frame builder is a real cost, but no pilot issues
/// an intent — a seated pilot on a `Reflex` unit is the state doc 16 §3.8's
/// released seat leaves behind.
fn field(
    knobs: &Knobs,
    catalog: &Catalog,
    relief: &[(u16, u16, i16)],
    reach: Reach,
) -> (
    Vec<tacsim::arena::state::CustodyUnit>,
    Vec<tacsim::arena::state::Pilot>,
) {
    use tacsim::arena::state::{Controller, Pilot, Team};
    let mut rng = ChaCha8Rng::seed_from_u64(knobs.seed ^ 0x000a_4e17);
    let mut pool: Vec<(u16, u16, i16)> = relief.to_vec();
    let middle = i32::from(SIDE) / 2;
    let mut arena_cluster: Vec<(u16, u16, i16)> = Vec::new();
    for n in 0..i32::try_from(knobs.fighting.min(knobs.units)).unwrap_or(0) {
        let side = if n % 2 == 0 { -1 } else { 1 };
        let rank = n / 2;
        let want_x = middle + side * i32::from(LINE_GAP_CELLS);
        let want_y = middle - LINE_ROWS_FROM_MIDDLE + rank * i32::from(LINE_PITCH_CELLS);
        let Some(at) = pool
            .iter()
            .enumerate()
            .min_by_key(|(_, (x, y, _))| {
                let dx = i32::from(*x) - want_x;
                let dy = i32::from(*y) - want_y;
                (dx * dx + dy * dy, i32::from(*x), i32::from(*y))
            })
            .map(|(at, _)| at)
        else {
            break;
        };
        arena_cluster.push(pool.swap_remove(at));
    }
    // Taken from the back below, so the first unit made is the first line position.
    arena_cluster.reverse();
    let mut units = Vec::new();
    let mut pilots = Vec::new();
    for n in 0..knobs.units {
        if pool.is_empty() && arena_cluster.is_empty() {
            break;
        }
        let (cx, cy, elev_cm) = if let Some(cell) = arena_cluster.pop() {
            cell
        } else {
            let pick = rng.random_range(0..pool.len());
            pool.swap_remove(pick)
        };
        let id = ObjId(100_000 + n);
        let team = if n % 2 == 0 { Team::Red } else { Team::Blue };
        let entry = COHORT
            .get(usize::try_from(n / 2).unwrap_or(0) % COHORT.len())
            .copied()
            .unwrap_or(content::LINE_TANK);
        let key = CanonicalId::new(entry).expect("a canonical entry id");
        let spec = catalog.specs.get(&key).expect("the cohort entry's spec");
        let binding = catalog
            .bindings
            .get(&key)
            .expect("the cohort entry's binding");
        let cells = tacsim::definitions::unit::to_tactical_unit(spec, binding, id)
            .expect("the cohort entry builds a unit");
        let pose = mush_sim::data::Pose {
            x_cm: i64::from(cx) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
            y_cm: i64::from(cy) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
            z_cm: i32::from(elev_cm),
            heading_mdeg: rng.random_range(0..360_u32) * 1_000,
            posture: mush_sim::data::passive_rules_value(serde_json::json!({}))
                .expect("an empty posture"),
        };
        if n < knobs.fighting {
            pilots.push(Pilot {
                unit: id,
                pilot: ObjId(500_000 + n),
                station: smol_str::SmolStr::new("cockpit"),
                epoch: 1,
            });
        }
        let mut unit =
            tacsim::match_::custody_unit(catalog, cells, id, team, pose, 1, Controller::Reflex, 0);
        if reach == Reach::Ci {
            scale_to_ci(&mut unit);
        }
        units.push(unit);
    }
    (units, pilots)
}

/// How many times `needle` occurs in `bytes` — the ledger is JSON lines, so
/// counting a kind is counting a substring, and a chunk can be counted as it
/// streams rather than kept.
fn occurrences(bytes: &[u8], needle: &str) -> u64 {
    u64::try_from(String::from_utf8_lossy(bytes).matches(needle).count()).unwrap_or(0)
}

/// **The cell every `blocked` line in a chunk names** (16b cut E.3).
///
/// Written when the event carried the refused cell and no cause, so the only way
/// to say whether a stop was traffic or ground was to ask the window about the
/// cell it names. **Since 16b cut M.3 the line carries its cause** (16c `T-04`,
/// `EventKind::Blocked::cause`), which [`blocked_causes_of`] reads; the split by
/// the named cell is kept because it answers *where* — interior, the window's
/// ring, or ground nothing may enter. The chunk is JSONL and the line's shape is
/// fixed by `EventKind::Blocked`, so this reads the two integers out of it
/// rather than deserialising a whole event per stop.
fn blocked_cells_of(bytes: &[u8]) -> Vec<(u16, u16)> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .filter(|line| line.contains("\"kind\":\"blocked\""))
        .filter_map(|line| {
            let after = |key: &str| -> Option<u16> {
                let at = line.find(key)? + key.len();
                let rest = line.get(at..)?;
                let end = rest
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len());
                rest.get(..end)?.parse::<u16>().ok()
            };
            Some((after("\"x\":")?, after("\"y\":")?))
        })
        .collect()
}

/// The four `BlockedCause` words, in the order [`Counts::blocked_causes`]
/// holds them.
const BLOCKED_CAUSES: [&str; 4] = ["impassable", "occupied", "edge", "too_steep"];

/// **Every `blocked` line's own cause** (16b cut U.1), counted into
/// [`BLOCKED_CAUSES`]' order off the line's `"cause":"…"` word — the *why* the
/// cell split cannot give. A word outside the four is counted nowhere, so the
/// four always sum to the lines that named a known cause.
fn blocked_causes_of(bytes: &[u8]) -> [u64; 4] {
    let mut causes = [0_u64; 4];
    let text = String::from_utf8_lossy(bytes);
    for line in text
        .lines()
        .filter(|line| line.contains("\"kind\":\"blocked\""))
    {
        let key = "\"cause\":\"";
        let Some(word) = line
            .find(key)
            .and_then(|at| line.get(at + key.len()..))
            .and_then(|rest| rest.split('"').next())
        else {
            continue;
        };
        if let Some(slot) = BLOCKED_CAUSES
            .iter()
            .position(|cause| *cause == word)
            .and_then(|at| causes.get_mut(at))
        {
            *slot += 1;
        }
    }
    causes
}

// ---- 16b cut Y.2: doc 16 §2 W8, off the black box ------------------------

/// **One `ordered` line, as W8's counter reads it** (16b cut Y.2): the arena
/// millisecond it landed at, who gave it — the pilot's own tank, `None` for the
/// LLM seam — and the squad it names, by its word (`red-1`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct OrderedLine {
    t_ms: i64,
    issuer: Option<ObjId>,
    squad: String,
}

/// **Every `ordered` line in a chunk** (16b cut Y.2), parsed whole: the line is
/// rare (none on a cohort with the LLM off and no human), so a full
/// `LedgerEvent` per line costs nothing the substring scan above does not.
///
/// A line that says `ordered` and does not parse **fails the run** rather than
/// dropping out: a dropped line would read `orders=0` beside a positive
/// `ordered_lines` and let W8's rate bound pass on nothing
/// ([`CommandFigures::of`] checks the two counts agree as well).
fn ordered_lines_of(bytes: &[u8]) -> Vec<OrderedLine> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| line.contains("\"kind\":\"ordered\""))
        .map(|line| {
            serde_json::from_str::<tacsim::arena::ledger::LedgerEvent>(line).unwrap_or_else(|e| {
                panic!("W8: an `ordered` line the harness cannot read ({e}): {line}")
            })
        })
        .filter_map(|event| match event.kind {
            tacsim::arena::ledger::EventKind::Ordered { issuer, squad, .. } => Some(OrderedLine {
                t_ms: event.t_ms,
                issuer,
                squad: squad.to_string(),
            }),
            _ => None,
        })
        .collect()
}

/// **W8's two tape figures** (16b cut Y.2): `(orders,
/// orders_per_commander_minute_max)`.
///
/// `orders` is the number of **distinct `(issuer-or-commander, t_ms)` pairs**:
/// the arena journals one `ordered` line **per unit** an order lands on, at the
/// pulse's `now_ms`, so counting lines would score a legal `ORDER ALL` to four
/// pairs as eight. A line with no issuer (the LLM seam's) is filed under the
/// commander of the squad it names, so the machines' own orders count against
/// the commander that asked for them.
///
/// A pair **approximates** an order, in both directions, because the line
/// carries the pulse's time and not the world's `issued_ms` (16b cut Y.1's
/// note): one `ORDER` whose offers straddle a pulse on the world path is two
/// pairs (an over-count), and two `ORDER`s from one issuer forwarded between the
/// same two pulses — up to 5 000 ms apart at the quiet tempo — share a `t_ms`
/// and are one pair (an under-count, which lets an overrun of the rate bound
/// below pass unseen). Neither happens on this file's lanes: the arena lanes
/// give no order, and the cost lane applies each order synchronously between
/// two steps, at most one per squad a pulse and each under its own commander,
/// so there a pair is exactly one order. The exact count on the world path needs
/// `issued_ms` on the `ordered` line, which moves the ledger event set and is a
/// 16c residual, not this cut's.
///
/// `orders_per_commander_minute_max` is the largest number of those pairs one
/// issuer has inside any sixty **arena** seconds (`[t, t + 60 000)`). It is
/// arena time; `ArenaRegistry::note_order_stamp` enforces the bound in world
/// time, and the two differ by the arena's lag, which on this harness is none.
fn orders_on_tape(
    lines: &[OrderedLine],
    squads: &[tacsim::deliberation::SquadRoster],
) -> (u64, u64) {
    let commander = |word: &str| {
        squads
            .iter()
            .find(|roster| roster.id.to_string() == word)
            .map(|roster| roster.commander)
    };
    let pairs: std::collections::BTreeSet<(Option<ObjId>, i64)> = lines
        .iter()
        .map(|line| (line.issuer.or_else(|| commander(&line.squad)), line.t_ms))
        .collect();
    // The set is ordered by issuer and then by time, so each issuer's stamps
    // arrive sorted.
    let mut per_issuer: BTreeMap<Option<ObjId>, Vec<i64>> = BTreeMap::new();
    for (issuer, t_ms) in &pairs {
        per_issuer.entry(*issuer).or_default().push(*t_ms);
    }
    let mut busiest = 0_usize;
    for stamps in per_issuer.values() {
        for (at, start) in stamps.iter().enumerate() {
            let inside = stamps
                .get(at..)
                .unwrap_or_default()
                .iter()
                .take_while(|t_ms| **t_ms < start.saturating_add(60_000))
                .count();
            busiest = busiest.max(inside);
        }
    }
    (
        u64::try_from(pairs.len()).unwrap_or(u64::MAX),
        u64::try_from(busiest).unwrap_or(u64::MAX),
    )
}

/// **W8's figures for one run** (16b cut Y.2; doc 16 §2 W8, *the Compact caps
/// span of control* and *orders per commander per minute bounded*).
struct CommandFigures {
    /// `[tacsim.command].max_span`, as the harness configures it.
    max_span: u32,
    /// `[tacsim.command].orders_per_minute`, likewise.
    orders_per_minute: u32,
    /// `arena.squads.len()`.
    squads: usize,
    /// One a squad.
    commanders: usize,
    /// Every `ordered` line on the tape: one per unit an order landed on.
    ordered_lines: u64,
    /// One per order ([`orders_on_tape`]).
    orders: u64,
    /// The busiest issuer's orders in any sixty arena seconds.
    orders_per_commander_minute_max: u64,
    /// The most squads under one issuer's live orders at the run's end
    /// (`Arena::live_orders_by_issuer`, an arena-time figure).
    span_max: usize,
}

impl CommandFigures {
    /// Read off the run's arena, its configured block and its tape.
    fn of(
        arena: &tacsim::arena::state::Arena,
        cfg: &tacsim::config::CommandConfig,
        ordered_lines: u64,
        tape: &[OrderedLine],
    ) -> CommandFigures {
        // Every counted line was read: the substring count and the parse agree,
        // or W8's figures below would be off a partial tape.
        assert_eq!(
            u64::try_from(tape.len()).unwrap_or(u64::MAX),
            ordered_lines,
            "W8: every `ordered` line on the tape is read for the per-order figures"
        );
        let (orders, orders_per_commander_minute_max) = orders_on_tape(tape, &arena.squads);
        let span_max = arena
            .live_orders_by_issuer(arena.now_ms)
            .values()
            .map(std::collections::BTreeSet::len)
            .max()
            .unwrap_or(0);
        CommandFigures {
            max_span: cfg.max_span,
            orders_per_minute: cfg.orders_per_minute,
            squads: arena.squads.len(),
            commanders: arena.squads.len(),
            ordered_lines,
            orders,
            orders_per_commander_minute_max,
            span_max,
        }
    }

    /// The `harness_arena_command` line. `orders_per_commander_minute_max` is
    /// counted in arena time and `span_max` read in arena time; the world
    /// enforces both in world time.
    fn line(&self, cohort: &str) -> String {
        format!(
            "harness_arena_command: cohort={cohort} max_span={} orders_per_minute={} squads={} \
             commanders={} ordered_lines={} orders={} orders_per_commander_minute_max={} \
             span_max={}",
            self.max_span,
            self.orders_per_minute,
            self.squads,
            self.commanders,
            self.ordered_lines,
            self.orders,
            self.orders_per_commander_minute_max,
            self.span_max
        )
    }

    /// The report's `"command"` object — reported, and deliberately **not** in
    /// [`DETERMINISTIC_OBJECTS`]: the committed baseline has stored it since the
    /// 16b cut J.1 bless (whole-file, as every bless is), but nothing on the
    /// cohort issues orders yet, so it is not compared until a later cut that
    /// scripts human orders on the cohort promotes it.
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "max_span": self.max_span,
            "orders_per_minute": self.orders_per_minute,
            "squads": self.squads,
            "commanders": self.commanders,
            "ordered_lines": self.ordered_lines,
            "orders": self.orders,
            "orders_per_commander_minute_max": self.orders_per_commander_minute_max,
            "span_max": self.span_max,
        })
    }

    /// **W8, asserted** (the CI cohort under `TACSIM_ASSERT`).
    fn assert_bounded(&self) {
        let span = usize::try_from(self.max_span).unwrap_or(usize::MAX);
        assert!(
            self.span_max <= span,
            "W8: {} squads under one commander, over [tacsim.command].max_span {}",
            self.span_max,
            self.max_span
        );
        assert!(
            self.orders_per_commander_minute_max <= u64::from(self.orders_per_minute),
            "W8: {} orders from one commander in sixty arena seconds, over \
             [tacsim.command].orders_per_minute {}",
            self.orders_per_commander_minute_max,
            self.orders_per_minute
        );
    }
}

/// **16b cut Y.2**: the `orders` counter counts one per order and never one per
/// unit. Two lines at one arena millisecond from one issuer are one `ORDER`'s
/// fan-out; the same issuer at another millisecond is a second; a line with no
/// issuer counts under the commander of the squad it names, and a second such
/// line at the same millisecond on another unit of that squad is the same one.
#[test]
fn y2_the_orders_counter_counts_orders_and_not_unit_lines() {
    let squads = vec![
        tacsim::deliberation::SquadRoster {
            id: tacsim::deliberation::SquadId {
                team: tacsim::arena::state::Team::Red,
                index: 1,
            },
            commander: ObjId(20),
            units: vec![ObjId(20), ObjId(21)],
        },
        tacsim::deliberation::SquadRoster {
            id: tacsim::deliberation::SquadId {
                team: tacsim::arena::state::Team::Red,
                index: 2,
            },
            commander: ObjId(22),
            units: vec![ObjId(22), ObjId(23)],
        },
    ];
    // Every tape is **real ledger bytes**: `LedgerEvent` lines as the arena
    // serialises them, one other kind between each, through `Counts::absorb` —
    // the path the lanes take — so a change to the `ordered` wire shape fails
    // here and not silently in W8's figures.
    let tape = |lines: &[(i64, Option<u32>, &str)]| -> Vec<OrderedLine> {
        let mut bytes = String::new();
        let mut seq = 0_u64;
        for (at, (t_ms, issuer, squad)) in lines.iter().enumerate() {
            let unit = ObjId(100 + u32::try_from(at).unwrap_or(0));
            for kind in [
                tacsim::arena::ledger::EventKind::Ordered {
                    unit,
                    squad: (*squad).into(),
                    issuer: issuer.map(ObjId),
                    objective: "hold".into(),
                    target: None,
                    spot: None,
                    hold_fire: false,
                    ttl_ms: 600_000,
                },
                tacsim::arena::ledger::EventKind::Void,
            ] {
                seq += 1;
                let event = tacsim::arena::ledger::LedgerEvent {
                    seq,
                    t_ms: *t_ms,
                    kind,
                    draws: Vec::new(),
                    hash: 0,
                };
                bytes.push_str(&serde_json::to_string(&event).expect("a ledger line serialises"));
                bytes.push('\n');
            }
        }
        let mut counts = Counts::default();
        counts.absorb(bytes.as_bytes());
        let want = u64::try_from(lines.len()).unwrap_or(u64::MAX);
        assert_eq!(counts.ordered_lines, want, "one `ordered` line a unit");
        assert_eq!(
            counts.ordered,
            lines
                .iter()
                .map(|(t_ms, issuer, squad)| OrderedLine {
                    t_ms: *t_ms,
                    issuer: issuer.map(ObjId),
                    squad: (*squad).to_string(),
                })
                .collect::<Vec<_>>(),
            "every line read back as written"
        );
        counts.ordered
    };
    // One `ORDER` to a pair: two lines, one pulse, one order.
    let one = tape(&[(1_000, Some(5), "red-1"), (1_000, Some(5), "red-1")]);
    assert_eq!(orders_on_tape(&one, &squads), (1, 1));
    // The same issuer again a pulse later: two.
    let two = tape(&[(1_000, Some(5), "red-1"), (1_500, Some(5), "red-2")]);
    assert_eq!(orders_on_tape(&two, &squads), (2, 2));
    // `ORDER ALL` to both squads lands in one pulse: one.
    let all = tape(&[
        (2_000, Some(5), "red-1"),
        (2_000, Some(5), "red-1"),
        (2_000, Some(5), "red-2"),
        (2_000, Some(5), "red-2"),
    ]);
    assert_eq!(orders_on_tape(&all, &squads), (1, 1));
    // The machines' own order, both units of red-2, filed under its commander
    // (22) — which is not the pilot, so the busiest issuer still has one.
    let machine = tape(&[
        (3_000, None, "red-2"),
        (3_000, None, "red-2"),
        (3_000, Some(5), "red-1"),
    ]);
    assert_eq!(orders_on_tape(&machine, &squads), (2, 1));
    // And the same commander's own order at that millisecond is the same one.
    let same = tape(&[(3_000, None, "red-2"), (3_000, Some(22), "red-2")]);
    assert_eq!(orders_on_tape(&same, &squads), (1, 1));
    // Sixty arena seconds is a half-open window: the stamp at +60 000 is the
    // next minute's.
    let minute = tape(&[
        (0, Some(5), "red-1"),
        (59_999, Some(5), "red-1"),
        (60_000, Some(5), "red-1"),
    ]);
    assert_eq!(orders_on_tape(&minute, &squads), (3, 2));
    assert_eq!(orders_on_tape(&tape(&[]), &squads), (0, 0));
}

/// **16b cut Y.2**: an `ordered` line the harness cannot read fails the run
/// instead of dropping out of W8's figures.
#[test]
#[should_panic(expected = "an `ordered` line the harness cannot read")]
fn y2_an_unreadable_ordered_line_fails_the_run() {
    let mut counts = Counts::default();
    counts.absorb(b"{\"seq\":1,\"t_ms\":0,\"kind\":\"ordered\",\"hash\":0}\n");
}

/// `<stem>-<suffix>.<ext>` beside the baseline's report, so the measurements do
/// not overwrite each other's JSON: `arena` for the CI cohort,
/// `arena-authored` for the authored reaches.
fn arena_report_path(given: &str, suffix: &str) -> std::path::PathBuf {
    let path = report_path(given);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "harness".to_string());
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "json".to_string());
    path.with_file_name(format!("{stem}-{suffix}.{ext}"))
}

/// What the streamed ledger says, counted as it goes rather than kept: the
/// egress is a ring and the point of the measurement is that it drains.
/// **16b cut W.1** added the three weapon kinds; **16b cut W.2** the four
/// outcome kinds, because a pipeline that resolved every shot as a miss would
/// never build a mutation plan and would measure the cheap half of the cut.
#[derive(Default)]
struct Counts {
    chunks: u64,
    bytes: u64,
    blocked: u64,
    /// **16b cut E.3**: every refused cell, so the stops can be split by where
    /// they happened.
    blocked_at: Vec<(u16, u16)>,
    /// **16b cut U.1**: the stops by their own cause, in [`BLOCKED_CAUSES`]
    /// order.
    blocked_causes: [u64; 4],
    locked: u64,
    fired: u64,
    impacts: u64,
    hits: u64,
    misses: u64,
    sections: u64,
    destroyed: u64,
    /// **16b cut Y.2**: every `ordered` line, one per unit an order landed on.
    ordered_lines: u64,
    /// **16b cut Y.2**: those lines, parsed, for W8's per-order figures.
    ordered: Vec<OrderedLine>,
}

impl Counts {
    /// Count one closed chunk.
    fn absorb(&mut self, bytes: &[u8]) {
        self.chunks += 1;
        self.bytes += u64::try_from(bytes.len()).unwrap_or(0);
        self.blocked += occurrences(bytes, "\"kind\":\"blocked\"");
        self.blocked_at.extend(blocked_cells_of(bytes));
        for (total, more) in self.blocked_causes.iter_mut().zip(blocked_causes_of(bytes)) {
            *total += more;
        }
        self.locked += occurrences(bytes, "\"kind\":\"locked\"");
        self.fired += occurrences(bytes, "\"kind\":\"fired\"");
        self.impacts += occurrences(bytes, "\"kind\":\"impact\"");
        self.hits += occurrences(bytes, "\"kind\":\"hit\"");
        self.misses += occurrences(bytes, "\"kind\":\"miss\"");
        self.sections += occurrences(bytes, "\"kind\":\"section_destroyed\"");
        self.destroyed += occurrences(bytes, "\"kind\":\"unit_destroyed\"");
        // 16b cut Y.2: the orders, off the same chunk.
        self.ordered_lines += occurrences(bytes, "\"kind\":\"ordered\"");
        self.ordered.extend(ordered_lines_of(bytes));
    }
}

/// **One measured wake** (16b cut U.1): what the step cost on the wall, the
/// arena millisecond it ended at, and the five per-pulse counters the window
/// fold sums (16b cut J.2 added `horizon_probes`).
struct Wake {
    ms: f64,
    now_ms: i64,
    traces: u32,
    cell_probes: u64,
    horizon_probes: u64,
    jam_checks: u32,
    share_ops: u32,
}

/// **One [`WINDOW_MS`] window of wakes, summed** (16b cut J.2 named the fields
/// of what was a five-tuple): the wake milliseconds and the five counters.
#[derive(Clone, Copy, Debug, Default)]
struct WindowSum {
    ms: f64,
    traces: u64,
    cell_probes: u64,
    horizon_probes: u64,
    jam_checks: u64,
    share_ops: u64,
}

/// The window the per-window figure folds wakes into, arena milliseconds: the
/// shipped `tempo.engaged_ms`, the pulse the 12 ms lens was calibrated on.
const WINDOW_MS: i64 = 500;

/// **The per-window figure** (16b cut U.1): every measured wake folded into
/// [`WINDOW_MS`] windows of arena time after `start_ms` by its post-step
/// `now_ms` — a wake ending at `t` did the work of `(t_prev, t]`, so it belongs
/// to window `(t − start − 1) / 500` — with the wake milliseconds and the five
/// counters summed per window. Answers the windows in order.
fn windows(wakes: &[Wake], start_ms: i64) -> Vec<WindowSum> {
    let mut folded: BTreeMap<i64, WindowSum> = BTreeMap::new();
    for wake in wakes {
        let slot = folded
            .entry((wake.now_ms - start_ms - 1).div_euclid(WINDOW_MS))
            .or_default();
        slot.ms += wake.ms;
        slot.traces += u64::from(wake.traces);
        slot.cell_probes += wake.cell_probes;
        slot.horizon_probes += wake.horizon_probes;
        slot.jam_checks += u64::from(wake.jam_checks);
        slot.share_ops += u64::from(wake.share_ops);
    }
    folded.into_values().collect()
}

/// **Cut A's measurement, on the CI cohort** (16b §6's verify lens; doc 16 §1
/// A2; 16b cut U.1): [`measure_arena`] at [`Reach::Ci`], the thresholds asserted
/// under `TACSIM_ASSERT`.
#[test]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
fn tacsim_harness_arena_step() {
    measure_arena(&Knobs::from_env(), Reach::Ci);
}

/// **The same measurement at the authored reaches** (16b cut U.1; 16c `T-34`'s
/// second run): [`measure_arena`] at [`Reach::Authored`], JSON at
/// `<stem>-arena-authored.<ext>`. Report-only whatever `TACSIM_ASSERT` says —
/// the bound the authored reach is measured against is cut J's `Q26` — so it
/// asserts only that the arena it timed was loaded.
#[test]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
fn tacsim_harness_arena_authored_reach() {
    measure_arena(&Knobs::from_env(), Reach::Authored);
}

/// **The long run** (16b cut U.2, 16c `T-36`): [`measure_long_run`] — the CI
/// cohort's field for `TACSIM_LONG_RUN_S` arena seconds (ten minutes by
/// default), resident memory read after the warm-up and at every minute of arena
/// time, the streamed ledger's size and the due heap's depth at the end, JSON at
/// `<stem>-arena-long.<ext>`. Under `TACSIM_ASSERT` it asserts the memory growth
/// and the ledger bound, and never the p99: the lens is the 30-second lane's.
#[test]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
fn tacsim_harness_arena_long_run() {
    measure_long_run(&Knobs::from_env());
}

/// **The seeded stand-in for the timer's overshoot** (16b cut U.1), one draw a
/// wake: `stagger_ms(seed, wake, TACSIM_WAKE_JITTER_MS + 1)` added to the
/// step's own `next_due_ms`. A struct since 16b cut U.2, so the two lanes that
/// share [`run_wakes`] carry it between the warm-up and the measured window.
struct WakeClock {
    seed: u64,
    cadence: i64,
    index: u32,
}

impl WakeClock {
    fn new(knobs: &Knobs) -> WakeClock {
        WakeClock {
            seed: knobs.seed,
            cadence: knobs.jitter_ms.max(0).saturating_add(1),
            index: 0,
        }
    }

    /// The wall the next wake happens at, after a step that answered
    /// `next_due_ms`.
    fn next(&mut self, next_due_ms: i64) -> i64 {
        self.index = self.index.wrapping_add(1);
        next_due_ms.saturating_add(tacsim::arena::rng::stagger_ms(
            self.seed,
            ObjId(self.index),
            self.cadence,
        ))
    }
}

/// **An arena fielded, opened and warmed up** — everything the measurement lanes
/// share before their measured window (16b cut U.2 factored it out of
/// [`measure_arena`], so the 30-second lane and the long run have one body and
/// differ only in their window and their sampling).
struct Field {
    cohort: &'static str,
    arena: tacsim::arena::state::Arena,
    arena_cfg: tacsim::config::ArenaConfig,
    sensors_cfg: tacsim::config::SensorsConfig,
    class_counts: BTreeMap<&'static str, u64>,
    terraces: Vec<(u16, u16, u16, i32)>,
    relief_edges_45: u64,
    relief_edges_70: u64,
    waypoint_nodes: usize,
    passable: usize,
    placed: u32,
    seated: u32,
    /// Units of each [`COHORT_ENTRIES`] row, in that order.
    fielded: [u32; 6],
    /// The cohort's substep, the lattice `next_due_ms` answers on.
    substep: u32,
    slope_45: usize,
    slope_70: usize,
    /// Every unit's magazines as checked out.
    ammo_at_start: BTreeMap<ObjId, u64>,
    /// The streamed ledger, counted as it goes — the warm-up's included.
    counts: Counts,
    clock: WakeClock,
    /// The wall the next wake is at.
    wall: i64,
    /// The slope refusals over the whole run, the warm-up included.
    slope_refusals: u64,
    generate_ms: f64,
    boot_ms: f64,
    apply_ms: f64,
    open_ms: f64,
    convert_ms: f64,
    arena_open_ms: f64,
    warm_ms: f64,
    warm_pulses: u32,
    killed_warming: usize,
    /// Arena time at the end of the warm-up: the measured window starts here.
    warm_end: i64,
}

/// **The relief, the field and the warm-up** (16b cut A.2 through 16b cut U.1;
/// factored out by 16b cut U.2).
///
/// The same generated relief as the ground-pulse baseline, converted into an
/// `ArenaDefinition` through [`harness_move_cost`], with `TACSIM_UNITS` units of
/// the fielded catalog ([`field`]) on it and `TACSIM_FIGHTING` of them seated,
/// every one on the reflex controller, the tempo pinned `Engaged`.
///
/// The warm-up is the controller's own: every unit starts cold and the
/// machine's first state brings it online, so the warm-up is over when arena
/// time has passed `startup_ms + decide_ms` and a period, and it is asserted —
/// a controller that started nothing would otherwise be measured driving an
/// arena of cold tanks.
///
/// The arena frame is the **window's own**: cell `(0, 0)` is at the origin,
/// because `ArenaTerrain` puts the window's corner there.
#[allow(clippy::too_many_lines)]
fn open_field(knobs: &Knobs, reach: Reach) -> Field {
    use tacsim::arena::clock::Tempo as ArenaTempo;
    use tacsim::arena::state::{Arena, CustodyOut, Power};
    use tacsim::config::{ArenaConfig, MatchConfig};

    // ---- the relief, as an arena definition ----
    let Window {
        tw,
        cells: all_cells,
        classes: class_counts,
        terraces,
        generate_ms,
        boot_ms,
        apply_ms,
        open_ms,
        ..
    } = open_window(knobs.seed);
    // The world was needed for the terrain and the catalog and for nothing
    // else: the arena is a library (doc 16 §3.1) and holds no handle on it.
    let catalog = fielded_catalog(&tw);
    drop(tw);

    let convert_at = Instant::now();
    let (grid, passable) = arena_ground(&all_cells);
    let (relief_edges_45, relief_edges_70) = refusing_edges(&grid);
    let definition = arena_definition(grid);
    let waypoint_nodes = definition.waypoints.nodes().len();
    let convert_ms = convert_at.elapsed().as_secs_f64() * 1_000.0;
    assert!(
        u32::try_from(passable.len()).unwrap_or(0) > knobs.units,
        "only {} passable cells for {} units",
        passable.len(),
        knobs.units
    );

    // ---- the units ----
    let (units, pilots) = field(knobs, &catalog, &passable, reach);
    let placed = u32::try_from(units.len()).unwrap_or(0);
    let seated = u32::try_from(pilots.len()).unwrap_or(0);
    let mut fielded = [0_u32; 6];
    for n in 0..units.len() {
        let entry = COHORT.get((n / 2) % COHORT.len()).copied().unwrap_or("");
        if let Some(slot) = COHORT_ENTRIES
            .iter()
            .position(|(id, _)| *id == entry)
            .and_then(|at| fielded.get_mut(at))
        {
            *slot += 1;
        }
    }

    let arena_cfg = ArenaConfig::default();
    let match_cfg = MatchConfig::default();
    let sensors_cfg = tacsim::config::SensorsConfig::default();
    let open_at = Instant::now();
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots,
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: knobs.seed,
        },
        &arena_cfg,
        &match_cfg,
        &sensors_cfg,
    )
    .expect("the harness arena opens");
    let arena_open_ms = open_at.elapsed().as_secs_f64() * 1_000.0;
    // D-16-2: the tempo decides only how often `step` runs. The measurement is of
    // `Engaged`, which is where the p99 ceiling applies, so it is held there.
    arena.clock.pin_tempo_for_tests(ArenaTempo::Engaged);
    let period_ms = i64::from(arena_cfg.tempo.engaged_ms);
    // **16b cut U.1**: the substep off the cohort's own speed ceiling
    // (`Arena::max_speed_cms`, the true one with the cheapest cell's discount),
    // which is the length `Arena::step` cuts and `next_due_ms`'s lattice.
    let substep =
        tacsim::arena::clock::substep_ms(&arena_cfg.tempo, arena.cell_cm(), arena.max_speed_cms());
    // The locomotion the cohort carried onto the field (16c `T-35`).
    let slope_45 = arena
        .units
        .iter()
        .filter(|unit| unit.slope_limit_pct == Some(45))
        .count();
    let slope_70 = arena
        .units
        .iter()
        .filter(|unit| unit.slope_limit_pct == Some(70))
        .count();
    // Every unit's magazines as checked out, so the rounds figure below is what
    // was spent and not a comparison against one load.
    let ammo_at_start: BTreeMap<ObjId, u64> = arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.ammo.values().map(|rounds| u64::from(*rounds)).sum(),
            )
        })
        .collect();

    let mut counts = Counts::default();
    let mut clock = WakeClock::new(knobs);
    // The slope refusals over the whole run, the warm-up included: the
    // controller plans its first routes there.
    let mut slope_refusals = 0_u64;

    // ---- the warm-up: the controller starts them up ----
    let warm_at = Instant::now();
    let out = arena.step(0, &[], &[]);
    for chunk in &out.ledger_chunks {
        counts.absorb(&chunk.bytes);
    }
    let mut wall = clock.next(out.next_due_ms);
    let mut warm_pulses = 1_u32;
    let warm_until = i64::from(match_cfg.startup_ms)
        .saturating_add(i64::from(arena_cfg.reflex.decide_ms))
        .saturating_add(period_ms);
    while arena.now_ms < warm_until {
        let out = arena.step(wall, &[], &[]);
        slope_refusals += u64::from(out.reflex.slope_refusals);
        for chunk in &out.ledger_chunks {
            counts.absorb(&chunk.bytes);
        }
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        wall = clock.next(out.next_due_ms);
        warm_pulses += 1;
    }
    let warm_ms = warm_at.elapsed().as_secs_f64() * 1_000.0;
    // A unit killed before its own startup completed is a wreck and not a cold
    // tank (16b cut U.1: at the authored reaches the first units online can
    // already see, and shoot at, the last ones still starting). That excuse is
    // the authored run's alone: at the CI cohort's reduced reach nothing is
    // killed before the field is up (the U.1 record reads `killed_warming=0`),
    // so a wreck there is a regression — a machine firing during warm-up, a
    // check-out with broken armour — and it fails the run exactly as a cold tank
    // did before the cut.
    //
    // **At the default knobs only** (16b cut J.3). The CI cohort IS the default
    // knobs — the delta refuses any other run as a different run — and a sight
    // knob run through this lane (`TACSIM_SENSOR_CELLS`, `TACSIM_SHARE_CELLS`)
    // is a sizing run of another field: at 60 and 30
    // [`scale_to_ci`] is the identity and fields the authored reaches exactly,
    // whose warm-up kills one unit as the U.1 record says. `killed_warming` is
    // printed on `harness_arena_setup_ms` for every run, so nothing is hidden;
    // at the defaults this is byte-for-byte U.1's assertion.
    let cold: Vec<(ObjId, Power)> = arena
        .units
        .iter()
        .filter(|unit| unit.power != Power::Online && !unit.limits.wreck)
        .map(|unit| (unit.id, unit.power))
        .collect();
    let killed_warming = arena.units.iter().filter(|unit| unit.limits.wreck).count();
    assert!(
        cold.is_empty(),
        "{} units never came online: the reflex controller did not start the field up: {cold:?}",
        cold.len()
    );
    if reach == Reach::Ci && sensor_cells() == SENSOR_CELLS && share_cells() == SHARE_CELLS {
        let wrecked: Vec<ObjId> = arena
            .units
            .iter()
            .filter(|unit| unit.limits.wreck)
            .map(|unit| unit.id)
            .collect();
        assert!(
            wrecked.is_empty(),
            "{} units were wrecked during the CI cohort's warm-up: {wrecked:?}",
            wrecked.len()
        );
    }
    let warm_end = arena.now_ms;
    Field {
        cohort: reach.word(),
        arena,
        arena_cfg,
        sensors_cfg,
        class_counts,
        terraces,
        relief_edges_45,
        relief_edges_70,
        waypoint_nodes,
        passable: passable.len(),
        placed,
        seated,
        fielded,
        substep,
        slope_45,
        slope_70,
        ammo_at_start,
        counts,
        clock,
        wall,
        slope_refusals,
        generate_ms,
        boot_ms,
        apply_ms,
        open_ms,
        convert_ms,
        arena_open_ms,
        warm_ms,
        warm_pulses,
        killed_warming,
        warm_end,
    }
}

/// **What one measured window of wakes produced** (16b cut U.2): the per-wake
/// record, the substep partition, the perception and controller counters as a
/// total and as the worst wake, and whether the match ended inside it.
struct Run {
    wakes: Vec<Wake>,
    substeps: Vec<u32>,
    short_substeps: u64,
    frames: Vec<f64>,
    ended: Option<tacsim::arena::state::MatchEnd>,
    lagged_pulses: u32,
    perception_total: tacsim::arena::perception::PerceptionCounters,
    peak: tacsim::arena::perception::PerceptionCounters,
    reflex_total: tacsim::arena::reflex::ReflexCounters,
    reflex_peak: tacsim::arena::reflex::ReflexCounters,
}

/// **The measured wakes** (16b cut U.1; one body for both lanes since 16b cut
/// U.2): the field stepped where the task wakes it until arena time reaches
/// `until_ms` or the match ends, `sample` handed the arena after every wake —
/// the long run reads memory there, the 30-second lane nothing.
///
/// **The loop is the task's** (16b cut U.1; 16c `T-22`, Q25). The harness
/// stepped at a fixed 500 ms period the production task never runs; it wakes
/// the arena where `arena_task` does — `wall = out.next_due_ms` after every
/// step, the first at `wall = 0` — plus [`WakeClock`]'s seeded overshoot
/// (default nought), so the record can say what the overshoot's remainder
/// substeps cost. `next_due_ms` is on the substep lattice, so a wake is at most
/// one substep. No scripted order and no scripted fire: what steers, closes,
/// locks and fires is the reflex controller alone.
///
/// Whatever the ledger is still holding leaves with the measurement, counted.
fn run_wakes(
    field: &mut Field,
    until_ms: i64,
    mut sample: impl FnMut(&tacsim::arena::state::Arena),
) -> Run {
    use tacsim::arena::perception::PerceptionCounters;
    let substep_ms = i64::from(field.substep);
    let mut run = Run {
        wakes: Vec::new(),
        substeps: Vec::new(),
        short_substeps: 0,
        frames: Vec::new(),
        ended: None,
        lagged_pulses: 0,
        // 16b cut S.1: the perception cost, as a running total and as the worst
        // wake. Doc 16 §2 W4 and W6 bound *per pulse*, and the pulse production
        // runs is the wake, so the per-wake peak is what the per-pulse bounds are
        // asserted against; the windows carry the per-window figure.
        perception_total: PerceptionCounters::default(),
        peak: PerceptionCounters::default(),
        // 16b cut N.1: the controller's cost, as a running total and as the worst
        // wake. `harness.max_path_queries_per_pulse` is asserted against the peak.
        reflex_total: tacsim::arena::reflex::ReflexCounters::default(),
        reflex_peak: tacsim::arena::reflex::ReflexCounters::default(),
    };
    let arena = &mut field.arena;
    while arena.now_ms < until_ms {
        let before = arena.now_ms;
        let at = Instant::now();
        let out = arena.step(field.wall, &[], &[]);
        let ms = at.elapsed().as_secs_f64() * 1_000.0;
        run.wakes.push(Wake {
            ms,
            now_ms: arena.now_ms,
            traces: out.perception.traces,
            cell_probes: out.perception.cell_probes,
            horizon_probes: out.perception.horizon_probes,
            jam_checks: out.perception.jam_checks,
            share_ops: out.perception.share_ops,
        });
        // A substep shorter than the lattice's: the remainder a wake off the
        // lattice cuts, and a phase-4 drain's zero-length one.
        let full = u32::try_from((arena.now_ms - before) / substep_ms.max(1)).unwrap_or(0);
        run.short_substeps += u64::from(out.substeps.saturating_sub(full));
        field.slope_refusals += u64::from(out.reflex.slope_refusals);
        run.perception_total.absorb(&out.perception);
        let peak = &mut run.peak;
        peak.passes = peak.passes.max(out.perception.passes);
        peak.candidates = peak.candidates.max(out.perception.candidates);
        peak.traces = peak.traces.max(out.perception.traces);
        peak.cell_probes = peak.cell_probes.max(out.perception.cell_probes);
        peak.horizon_probes = peak.horizon_probes.max(out.perception.horizon_probes);
        peak.share_ops = peak.share_ops.max(out.perception.share_ops);
        peak.jam_checks = peak.jam_checks.max(out.perception.jam_checks);
        peak.detected = peak.detected.max(out.perception.detected);
        peak.classified = peak.classified.max(out.perception.classified);
        peak.lost = peak.lost.max(out.perception.lost);
        run.reflex_total.absorb(&out.reflex);
        let reflex_peak = &mut run.reflex_peak;
        reflex_peak.ticks = reflex_peak.ticks.max(out.reflex.ticks);
        reflex_peak.path_queries = reflex_peak.path_queries.max(out.reflex.path_queries);
        reflex_peak.path_deferred = reflex_peak.path_deferred.max(out.reflex.path_deferred);
        reflex_peak.path_cells = reflex_peak.path_cells.max(out.reflex.path_cells);
        reflex_peak.path_failures = reflex_peak.path_failures.max(out.reflex.path_failures);
        reflex_peak.traces = reflex_peak.traces.max(out.reflex.traces);
        reflex_peak.intents = reflex_peak.intents.max(out.reflex.intents);
        reflex_peak.shots = reflex_peak.shots.max(out.reflex.shots);
        reflex_peak.slope_refusals = reflex_peak.slope_refusals.max(out.reflex.slope_refusals);
        run.substeps.push(out.substeps);
        #[allow(clippy::cast_precision_loss)]
        run.frames.push(out.frames.len() as f64);
        if out.lagged {
            run.lagged_pulses += 1;
        }
        for chunk in &out.ledger_chunks {
            field.counts.absorb(&chunk.bytes);
        }
        // The world half's acknowledgement (doc 16 §3.1's ack-gated egress), so the
        // bounded ring is measured draining as it would in production.
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        sample(arena);
        if let Some(end) = out.ended {
            run.ended = Some(end);
            break;
        }
        field.wall = field.clock.next(out.next_due_ms);
    }
    // Whatever is still open leaves with the measurement.
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        field.counts.absorb(&chunk.bytes);
    }
    run
}

/// **The arena measurement** (16b cut A.2 through 16b cut U.2): [`open_field`],
/// then [`run_wakes`] for `TACSIM_ARENA_S` arena seconds after the warm-up, and
/// the report.
///
/// Every measured wake is also folded into [`WINDOW_MS`] windows of arena time
/// ([`windows`]) and the 12 ms lens binds both: a wake is at most one substep,
/// a smaller unit than the 500 ms pulse the lens was calibrated on.
///
/// **16b cut U.2** (16c `T-36`, `T-29`) added the two per-pulse bounds the
/// report had printed and never asserted — cell probes and jam checks, against
/// the worst wake — and resident memory read on every platform
/// ([`resident_kib`]), with Linux's `VmRSS` beside it.
#[allow(clippy::too_many_lines)]
fn measure_arena(knobs: &Knobs, reach: Reach) {
    let rss_start_kib = resident_kib();
    let vmrss_start_kib = vmrss_kib();
    let mut field = open_field(knobs, reach);
    let until_ms = field
        .warm_end
        .saturating_add(i64::from(knobs.arena_s) * 1_000);
    let run = run_wakes(&mut field, until_ms, |_| {});
    let rss_end_kib = resident_kib();
    let vmrss_end_kib = vmrss_kib();

    let Field {
        cohort,
        arena,
        arena_cfg,
        sensors_cfg,
        class_counts,
        terraces,
        relief_edges_45,
        relief_edges_70,
        waypoint_nodes,
        passable,
        placed,
        seated,
        fielded,
        substep,
        slope_45,
        slope_70,
        ammo_at_start,
        counts,
        slope_refusals,
        generate_ms,
        boot_ms,
        apply_ms,
        open_ms,
        convert_ms,
        arena_open_ms,
        warm_ms,
        warm_pulses,
        killed_warming,
        warm_end,
        ..
    } = field;
    let Run {
        wakes,
        substeps,
        short_substeps,
        frames,
        ended,
        lagged_pulses,
        perception_total,
        peak,
        reflex_total,
        reflex_peak,
    } = run;

    let tempo = Tempo {
        name: "arena_engaged",
        period_ms: arena_cfg.tempo.engaged_ms,
        ms: wakes.iter().map(|wake| wake.ms).collect(),
    };
    let (p50, p95, p99, max) = tempo.stats();
    let folded = windows(&wakes, warm_end);
    let window = Tempo {
        name: "arena_window",
        period_ms: u32::try_from(WINDOW_MS).unwrap_or(0),
        ms: folded.iter().map(|w| w.ms).collect(),
    };
    let (w50, w95, w99, wmax) = window.stats();
    let window_traces_peak = folded.iter().map(|w| w.traces).max().unwrap_or(0);
    let window_cell_probes_peak = folded.iter().map(|w| w.cell_probes).max().unwrap_or(0);
    let window_horizon_probes_peak = folded.iter().map(|w| w.horizon_probes).max().unwrap_or(0);
    let window_jam_checks_peak = folded.iter().map(|w| w.jam_checks).max().unwrap_or(0);
    let window_share_ops_peak = folded.iter().map(|w| w.share_ops).max().unwrap_or(0);
    let wake_count = wakes.len();
    #[allow(clippy::cast_precision_loss)]
    let arena_s_run = (arena.now_ms - warm_end) as f64 / 1_000.0;
    #[allow(clippy::cast_precision_loss)]
    let wakes_per_arena_s = if arena_s_run > 0.0 {
        wake_count as f64 / arena_s_run
    } else {
        0.0
    };
    #[allow(clippy::cast_precision_loss)]
    let substeps_per_wake_mean = if substeps.is_empty() {
        0.0
    } else {
        f64::from(substeps.iter().sum::<u32>()) / substeps.len() as f64
    };
    let substeps_min = substeps.iter().copied().min().unwrap_or(0);
    let substeps_max = substeps.iter().copied().max().unwrap_or(0);
    #[allow(clippy::cast_precision_loss)]
    let frames_mean = if frames.is_empty() {
        0.0
    } else {
        frames.iter().sum::<f64>() / frames.len() as f64
    };
    let moving = arena
        .units
        .iter()
        .filter(|unit| unit.speed_cms != 0)
        .count();
    // **16b cut M.3**: the units standing against something at the end — the
    // `held` condition the frame's `HELD` reads — and not the per-substep
    // `blocked` flag, which says only that the last substep was refused.
    let held = arena
        .units
        .iter()
        .filter(|unit| unit.held.is_some())
        .count();
    // 16b cut W.1: the rounds figure, over the whole field — a statement about
    // the survivors (16b cut W.2): `armed` and `rounds_spent` are read off the
    // units whose *live* weapon table is not empty, which `remove_weapons` and
    // `destroy_unit` empty, so a tank whose turret came off and a wreck take
    // their rounds out of the sum. `fired` is the black box's own count of pulls
    // and is the number that is conserved; the exact conservation law is
    // `crates/tacsim/tests/resolution.rs`'s. **16b cut U.1**: `rounds_spent` is
    // each armed unit's magazines at check-out less what it holds now, and
    // `rounds_loaded` the whole field's at check-out. It was one load per weapon
    // less every magazine, which read nought on the catalog's units, whose
    // magazines hold more than one load.
    let armed = arena
        .units
        .iter()
        .filter(|unit| !unit.weapons.is_empty())
        .count();
    let rounds_left: u64 = arena
        .units
        .iter()
        .flat_map(|unit| unit.ammo.values())
        .map(|rounds| u64::from(*rounds))
        .sum();
    let loaded: u64 = ammo_at_start.values().sum();
    let rounds_spent: u64 = arena
        .units
        .iter()
        .filter(|unit| !unit.weapons.is_empty())
        .map(|unit| {
            let now: u64 = unit.ammo.values().map(|rounds| u64::from(*rounds)).sum();
            ammo_at_start
                .get(&unit.id)
                .copied()
                .unwrap_or(0)
                .saturating_sub(now)
        })
        .sum();

    // **The measurement was of a loaded arena.** These are not thresholds and they
    // do not wait for `TACSIM_ASSERT`, and both reaches assert them: a regression
    // that refused every `Speed` order, or stopped integrating, would make the
    // pulse *faster* and pass every timing bound in this file, so the report would
    // say the arena is fast at doing nothing. The numbers themselves are noise;
    // that they are non-zero is not.
    assert!(
        moving > 0,
        "no unit was moving: the measurement was of a parked arena"
    );
    assert!(
        counts.blocked > 0,
        "nothing was ever blocked: the units never reached the water, each other or \
         the window's edge, so the boundary test was not measured"
    );
    // **16b cut U.1** (16c `T-35`): what the design guarantees about the ground
    // and the cohort, and not what a working controller happens to do. The relief
    // has edges steeper than both the tracked and the legged limits, the cohort
    // carried both limits onto the field, and the planner and the follower
    // refused edges by the slope rule. A motion-level `too_steep` stop is
    // reported and not asserted: every unit is the controller's, the planner
    // refuses steep edges and `hold_routeless` stops a routeless machine before
    // motion reaches one, so such a stop records a controller defect and a
    // working controller may journal none.
    assert!(
        relief_edges_45 > 0 && relief_edges_70 > 0,
        "the relief refuses nothing: {relief_edges_45} edges past 45 % and \
         {relief_edges_70} past 70 %"
    );
    assert!(
        slope_45 > 0 && slope_70 > 0,
        "the cohort carried no locomotion: {slope_45} units at 45 % and {slope_70} at 70 %"
    );
    assert!(
        slope_refusals > 0,
        "no edge was ever refused by the slope rule: the slope limit was not measured"
    );
    assert!(
        counts.bytes > 0,
        "the ledger streamed nothing: the egress was not measured"
    );
    // **16b cut W.2**, the same rule once more: a resolution that hit nothing would
    // never build a mutation plan, never apply one and never fire a consequence, and
    // the report would say the arena is fast at missing.
    assert!(
        counts.hits > 0,
        "no shot ever landed: the resolution and the damage plan were not measured \
         (fired={} impacts={} misses={})",
        counts.fired,
        counts.impacts,
        counts.misses
    );
    assert_eq!(
        counts.hits.saturating_add(counts.misses),
        counts.impacts,
        "every arrival is exactly one outcome: {} hits and {} misses against {} impacts",
        counts.hits,
        counts.misses,
        counts.impacts
    );
    // 16b cut W.1, the same rule: a weapon lane that fired nothing would make the
    // pulse cheaper and pass every timing bound in this file, and the report would
    // say the arena is fast at not shooting.
    assert!(
        counts.locked > 0 && counts.fired > 0,
        "no shot was ever accepted: the weapon lane was not measured (locked={} \
         fired={} impacts={})",
        counts.locked,
        counts.fired,
        counts.impacts
    );
    assert!(
        counts.impacts > 0,
        "no shot ever arrived: the impact heap was not measured (fired={})",
        counts.fired
    );
    assert_eq!(lagged_pulses, 0, "the measurement was taken while lagging");
    // 16b cut S.1, the same rule as the three above: a perception pass that
    // detected nothing would make every trace and probe count below a measurement
    // of an arena that is fast at looking at an empty field.
    assert!(
        perception_total.passes > 0 && perception_total.traces > 0,
        "no sensor pass ran: the perception cost was not measured ({perception_total:?})"
    );
    assert!(
        perception_total.detected > 0,
        "nothing was ever detected: the units never came inside each other's reach, so the \
         contact store was not measured ({perception_total:?})"
    );
    assert!(
        perception_total.classified > 0,
        "nothing was ever classified: no contact was held for `classify_after_ms`, so neither \
         the gap rule nor the share mesh was measured ({perception_total:?})"
    );

    // ---- the report ----
    println!(
        "harness_arena_world: cohort={cohort} units={placed}/{} pilots={seated} cell_cm={CELL_CM} side={SIDE} passable_cells={passable} seed={} arena_ms={}",
        knobs.units, knobs.seed, arena.now_ms
    );
    let classes = class_counts
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!("harness_arena_classes: cohort={cohort} {classes}");
    println!(
        "harness_arena_setup_ms: cohort={cohort} generate={generate_ms:.1} boot={boot_ms:.1} pack_apply={apply_ms:.1} window_open={open_ms:.1} terrain_convert={convert_ms:.1} arena_open={arena_open_ms:.1} warm_up={warm_ms:.1} warm_pulses={warm_pulses} killed_warming={killed_warming}"
    );
    let cohort_words = COHORT_ENTRIES
        .iter()
        .zip(fielded.iter())
        .map(|((_, word), count)| format!("{word}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "harness_arena_cohort: cohort={cohort} {cohort_words} slope_45={slope_45} slope_70={slope_70} slope_refusals={slope_refusals} relief_edges_45={relief_edges_45} relief_edges_70={relief_edges_70}"
    );
    println!(
        "harness_arena_terraces: cohort={cohort} {}",
        terraces
            .iter()
            .map(|(x, y, r, rise)| format!("({x},{y}) r={r} rise_cm={rise}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "harness_arena_loop: cohort={cohort} wakes={wake_count} arena_s={arena_s_run:.3} wakes_per_arena_s={wakes_per_arena_s:.2} substep_ms={substep} jitter_ms={} substeps_per_wake_mean={substeps_per_wake_mean:.3} short_substeps={short_substeps}",
        knobs.jitter_ms
    );
    println!(
        "harness_arena_engaged: cohort={cohort} period_ms={} pulses={wake_count} p50={p50:.3} p95={p95:.3} p99={p99:.3} max={max:.3}",
        tempo.period_ms
    );
    println!(
        "harness_arena_window: cohort={cohort} window_ms={WINDOW_MS} windows={} ms_p50={w50:.3} ms_p95={w95:.3} ms_p99={w99:.3} ms_max={wmax:.3} traces_peak={window_traces_peak} cell_probes_peak={window_cell_probes_peak} horizon_probes_peak={window_horizon_probes_peak} jam_checks_peak={window_jam_checks_peak} share_ops_peak={window_share_ops_peak}",
        folded.len()
    );
    // **16b cut M.2** (16c `T-05`; doc 16 §3.1 as amended, Q24): whether the
    // cohort's substep divides all three periods — the one thing that makes the
    // scripted three-cadence replays cut arena time on one set of boundaries, and
    // since 16b cut U.1 what makes every period multiple a lattice point.
    let substep_divides_periods = [
        arena_cfg.tempo.quiet_ms,
        arena_cfg.tempo.alert_ms,
        arena_cfg.tempo.engaged_ms,
    ]
    .iter()
    .all(|period| period % substep == 0);
    println!(
        "harness_arena_substeps: cohort={cohort} per_wake_min={substeps_min} per_wake_max={substeps_max} substep_ms={substep} divides_periods={substep_divides_periods}"
    );
    // **16b cut E.3**: the stops split by where the refused cell is, which the
    // window can answer — **`impassable`**: off the window or ground nothing
    // tracked may enter; **`ring`**: a passable cell on the window's outer ring,
    // where every `Edge` stop names its cell; **`interior`**: a passable cell off
    // the ring, a crossing refused by another unit or by a bank. The *why* is
    // `harness_arena_blocked_causes` below, off the line's own cause.
    let terrain = &arena.definition.terrain;
    let on_ring = |x: u16, y: u16| -> bool {
        x == 0
            || y == 0
            || x.saturating_add(1) >= terrain.width()
            || y.saturating_add(1) >= terrain.height()
    };
    let mut interior = 0_usize;
    let mut ring = 0_usize;
    let mut impassable = 0_usize;
    for (x, y) in &counts.blocked_at {
        if terrain
            .get(*x, *y)
            .is_some_and(tacsim::arena::state::TerrainCell::passable)
        {
            if on_ring(*x, *y) {
                ring += 1;
            } else {
                interior += 1;
            }
        } else {
            impassable += 1;
        }
    }
    println!(
        "harness_arena_blocked_split: cohort={cohort} interior={interior} ring={ring} impassable={impassable} of {} lines",
        counts.blocked_at.len()
    );
    let [
        cause_impassable,
        cause_occupied,
        cause_edge,
        cause_too_steep,
    ] = counts.blocked_causes;
    println!(
        "harness_arena_blocked_causes: cohort={cohort} impassable={cause_impassable} occupied={cause_occupied} edge={cause_edge} too_steep={cause_too_steep} of {} lines",
        counts.blocked
    );
    println!(
        "harness_arena_counters: cohort={cohort} blocked={} ledger_bytes={} chunks={} frames_per_pulse_mean={frames_mean:.2} moving_at_end={moving} held_at_end={held} lagged_pulses={lagged_pulses} ended={}",
        counts.blocked,
        counts.bytes,
        counts.chunks,
        ended.map_or("none".to_string(), |end| format!("{:?}", end.reason))
    );
    println!(
        "harness_arena_weapons: cohort={cohort} armed={armed} locked={} fired={} impacts={} in_flight_at_end={} shot_seq={} rounds_spent={rounds_spent} rounds_left={rounds_left}",
        counts.locked,
        counts.fired,
        counts.impacts,
        arena.shots.len(),
        arena.next_shot_seq
    );
    // **16b cut W.2**: what the resolution and the damage did with those shots.
    println!(
        "harness_arena_damage: cohort={cohort} hits={} misses={} sections_destroyed={} units_destroyed={} wrecks_at_end={} disabled_at_end={}",
        counts.hits,
        counts.misses,
        counts.sections,
        counts.destroyed,
        arena.units.iter().filter(|unit| unit.limits.wreck).count(),
        arena.units.iter().filter(|unit| unit.disabled).count()
    );
    // **16b cut N.1**, the same rule the loaded-arena assertions follow: a
    // controller that never decided, never searched and never fired would make
    // the pulse cheaper and pass every timing bound in this file.
    assert!(
        reflex_total.ticks > 0 && reflex_total.intents > 0,
        "the reflex controller never decided anything: {reflex_total:?}"
    );
    assert!(
        reflex_total.path_queries > 0,
        "the controller never searched for a route: {reflex_total:?}"
    );
    assert!(
        reflex_total.shots > 0,
        "no controller ever pulled a trigger: {reflex_total:?}"
    );
    let distribution = arena.reflex.distribution();
    let engaging = distribution
        .get(usize::from(
            tacsim::arena::reflex::ReflexState::Engage.code(),
        ))
        .copied()
        .unwrap_or(0)
        .saturating_add(
            distribution
                .get(usize::from(
                    tacsim::arena::reflex::ReflexState::Approach.code(),
                ))
                .copied()
                .unwrap_or(0),
        );
    assert!(
        engaging > 0,
        "nobody was fighting at the end of the run: {distribution:?}"
    );

    println!(
        "harness_arena_reflex: cohort={cohort} waypoint_nodes={waypoint_nodes} decide_ms={} path_max_cells={} max_path_queries={} repath_ms={} ticks={} intents={} shots={} path_queries={} path_deferred={} path_cells={} path_failures={} traces={} slope_refusals={} peak_ticks={} peak_path_queries={} peak_path_cells={} peak_traces={}",
        arena_cfg.reflex.decide_ms,
        arena_cfg.reflex.path_max_cells,
        arena_cfg.reflex.max_path_queries,
        arena_cfg.reflex.repath_ms,
        reflex_total.ticks,
        reflex_total.intents,
        reflex_total.shots,
        reflex_total.path_queries,
        reflex_total.path_deferred,
        reflex_total.path_cells,
        reflex_total.path_failures,
        reflex_total.traces,
        reflex_total.slope_refusals,
        reflex_peak.ticks,
        reflex_peak.path_queries,
        reflex_peak.path_cells,
        reflex_peak.traces
    );
    println!(
        "harness_arena_reflex_states: cohort={cohort} {}",
        tacsim::arena::reflex::ReflexState::ALL
            .iter()
            .enumerate()
            .map(|(at, state)| format!(
                "{}={}",
                state.as_str(),
                distribution.get(at).copied().unwrap_or(0)
            ))
            .collect::<Vec<_>>()
            .join(" ")
    );
    // **16b cut Y.2** (doc 16 §2 W8): the span of control and the order rate,
    // at the harness's own `[tacsim.command]` (the defaults, as `arena_cfg` is),
    // off the tape the ledger streamed and the reflex rows at the end. On a
    // cohort with the LLM off and no human every figure but the two bounds and
    // the squad count is nought, which is the truthful measurement; the
    // enforcement itself is `tests/cockpit.rs`'s. Read after the measured loop.
    let command_cfg = tacsim::config::CommandConfig::default();
    let command = CommandFigures::of(&arena, &command_cfg, counts.ordered_lines, &counts.ordered);
    println!("{}", command.line(cohort));
    println!(
        "harness_arena_perception_peak: cohort={cohort} passes={} candidates={} traces={} cell_probes={} horizon_probes={} jam_checks={} share_ops={} detected={} classified={} lost={}",
        peak.passes,
        peak.candidates,
        peak.traces,
        peak.cell_probes,
        peak.horizon_probes,
        peak.jam_checks,
        peak.share_ops,
        peak.detected,
        peak.classified,
        peak.lost
    );
    // The report line reads `authored` where nothing was scaled; the JSON keeps
    // the two keys numbers on the CI cohort, as they were before the cut, and
    // `null` on the authored one (whose top-level `"cohort"` says why), so a
    // reader that parses them as integers is not broken by the CI report.
    let (sight_cells, mesh_cells, sight_json, mesh_json) = match reach {
        Reach::Ci => (
            sensor_cells().to_string(),
            share_cells().to_string(),
            serde_json::json!(sensor_cells()),
            serde_json::json!(share_cells()),
        ),
        Reach::Authored => (
            "authored".to_string(),
            "authored".to_string(),
            serde_json::Value::Null,
            serde_json::Value::Null,
        ),
    };
    println!(
        "harness_arena_perception_total: cohort={cohort} sensor_cells={sight_cells} share_cells={mesh_cells} pass_ms={} lost_after_passes={} passes={} candidates={} traces={} cell_probes={} horizon_probes={} jam_checks={} share_ops={} detected={} classified={} lost={} contacts_at_end={} alert_range_cm={}",
        sensors_cfg.pass_ms,
        sensors_cfg.lost_after_passes,
        perception_total.passes,
        perception_total.candidates,
        perception_total.traces,
        perception_total.cell_probes,
        perception_total.horizon_probes,
        perception_total.jam_checks,
        perception_total.share_ops,
        perception_total.detected,
        perception_total.classified,
        perception_total.lost,
        arena.contacts.len(),
        arena.alert_range_cm
    );
    // **16b cut U.2** (16c `T-36`): resident memory on every platform, with
    // Linux's own `VmRSS` beside it where there is one.
    println!(
        "harness_arena_memory: cohort={cohort} rss_start_kib={} rss_end_kib={} horizon_bytes={}{}",
        kib_word(rss_start_kib),
        kib_word(rss_end_kib),
        arena.definition.terrain.horizon_bytes(),
        vmrss_words(vmrss_start_kib, vmrss_end_kib)
    );
    // Whether this run asserts its thresholds: the CI cohort under
    // `TACSIM_ASSERT`, and the authored reaches never.
    let asserting = knobs.assert && reach == Reach::Ci;
    let lens = if asserting {
        "per-wake-and-per-window"
    } else {
        "report-only"
    };
    println!(
        "harness_arena_thresholds: cohort={cohort} assert={asserting} lens={lens} cut_a_p99_ms={ARENA_P99_MS} cut_s_p99_ms={ARENA_S_P99_MS} engaged_p99_ms={} max_traces_per_pulse={} max_share_ops_per_pulse={} max_path_queries_per_pulse={} max_cell_probes_per_pulse={} max_jam_checks_per_pulse={} max_ledger_bytes={}",
        knobs.engaged_p99_ms,
        knobs.max_traces_per_pulse,
        knobs.max_share_ops_per_pulse,
        knobs.max_path_queries_per_pulse,
        knobs.max_cell_probes_per_pulse,
        knobs.max_jam_checks_per_pulse,
        knobs.max_ledger_bytes
    );

    let report = serde_json::json!({
        "what": "16b cut U.2: `Arena::step` at the harness's scale, the fielded catalog on the reflex controller, woken where the task wakes it, with the cell-probe and jam-check bounds asserted",
        "cohort": cohort,
        "world": {
            "units": placed,
            "pilots": seated,
            "cell_cm": CELL_CM,
            "side_cells": SIDE,
            "passable_cells": passable,
            "seed": knobs.seed,
            "arena_ms": arena.now_ms,
            "pulses": wake_count,
            "move_cost_table": "harness-local (harness_move_cost); 16c B-58's per-locomotion layer is not built",
        },
        "classes": class_counts,
        "setup_ms": {
            "generate": num(generate_ms),
            "boot": num(boot_ms),
            "pack_apply": num(apply_ms),
            "window_open": num(open_ms),
            "terrain_convert": num(convert_ms),
            "arena_open": num(arena_open_ms),
            "warm_up": num(warm_ms),
            "warm_pulses": warm_pulses,
            "killed_warming": killed_warming,
        },
        "cohort_mix": COHORT_ENTRIES
            .iter()
            .zip(fielded.iter())
            .map(|((_, word), count)| ((*word).to_string(), *count))
            .collect::<BTreeMap<String, u32>>(),
        "relief": {
            "edges_over_45": relief_edges_45,
            "edges_over_70": relief_edges_70,
            "terraces": terraces
                .iter()
                .map(|(x, y, r, rise)| serde_json::json!({
                    "x": x, "y": y, "radius_cells": r, "rise_cm": rise,
                }))
                .collect::<Vec<_>>(),
        },
        "slope": {
            "units_at_45": slope_45,
            "units_at_70": slope_70,
            "refusals": slope_refusals,
        },
        "loop": {
            "wakes": wake_count,
            "arena_s": num(arena_s_run),
            "wakes_per_arena_s": num(wakes_per_arena_s),
            "substep_ms": substep,
            "jitter_ms": knobs.jitter_ms,
            "substeps_per_wake_mean": num(substeps_per_wake_mean),
            "short_substeps": short_substeps,
        },
        "tempos": [{
            "name": tempo.name,
            "period_ms": tempo.period_ms,
            "pulses": wake_count,
            "p50_ms": num(p50),
            "p95_ms": num(p95),
            "p99_ms": num(p99),
            "max_ms": num(max),
        }],
        "window": {
            "window_ms": WINDOW_MS,
            "windows": folded.len(),
            "ms_p50": num(w50),
            "ms_p95": num(w95),
            "ms_p99": num(w99),
            "ms_max": num(wmax),
            "traces_peak": window_traces_peak,
            "cell_probes_peak": window_cell_probes_peak,
            "horizon_probes_peak": window_horizon_probes_peak,
            "jam_checks_peak": window_jam_checks_peak,
            "share_ops_peak": window_share_ops_peak,
        },
        "substeps": {
            "per_wake_min": substeps_min,
            "per_wake_max": substeps_max,
            "substep_ms": substep,
            "substep_divides_periods": substep_divides_periods,
        },
        "counters": {
            "blocked_events": counts.blocked,
            "blocked_interior_passable": interior,
            "blocked_ring_passable": ring,
            "blocked_impassable": impassable,
            // 16b cut U.1: and why, off each line's own cause.
            "blocked_causes": BLOCKED_CAUSES
                .iter()
                .zip(counts.blocked_causes.iter())
                .map(|(word, count)| ((*word).to_string(), *count))
                .collect::<BTreeMap<String, u64>>(),
            "ledger_bytes_streamed": counts.bytes,
            "ledger_chunks": counts.chunks,
            "frames_per_pulse_mean": num(frames_mean),
            "moving_at_end": moving,
            "held_at_end": held,
            "lagged_pulses": lagged_pulses,
            "ended": ended.map(|end| format!("{:?}", end.reason)),
        },
        "weapons": {
            "armed_units": armed,
            "locked_events": counts.locked,
            "fired_events": counts.fired,
            "impact_events": counts.impacts,
            "in_flight_at_end": arena.shots.len(),
            // 16b cut W.2's outcomes.
            "hits": counts.hits,
            "misses": counts.misses,
            "sections_destroyed": counts.sections,
            "units_destroyed": counts.destroyed,
            "wrecks_at_end": arena.units.iter().filter(|unit| unit.limits.wreck).count(),
            "disabled_at_end": arena.units.iter().filter(|unit| unit.disabled).count(),
            "shot_seq_at_end": arena.next_shot_seq,
            "rounds_loaded": loaded,
            "rounds_spent": rounds_spent,
            "rounds_left": rounds_left,
        },
        "perception": {
            "sensor_cells": sight_json,
            "share_cells": mesh_json,
            "pass_ms": sensors_cfg.pass_ms,
            "lost_after_passes": sensors_cfg.lost_after_passes,
            "alert_range_cm": arena.alert_range_cm,
            "contacts_at_end": arena.contacts.len(),
            "peak_per_pulse": {
                "passes": peak.passes,
                "candidates": peak.candidates,
                "traces": peak.traces,
                "cell_probes": peak.cell_probes,
                "horizon_probes": peak.horizon_probes,
                "jam_checks": peak.jam_checks,
                "share_ops": peak.share_ops,
                "detected": peak.detected,
                "classified": peak.classified,
                "lost": peak.lost,
            },
            "total": {
                "passes": perception_total.passes,
                "candidates": perception_total.candidates,
                "traces": perception_total.traces,
                "cell_probes": perception_total.cell_probes,
                "horizon_probes": perception_total.horizon_probes,
                "jam_checks": perception_total.jam_checks,
                "share_ops": perception_total.share_ops,
                "detected": perception_total.detected,
                "classified": perception_total.classified,
                "lost": perception_total.lost,
            },
        },
        "reflex": {
            "waypoint_nodes": waypoint_nodes,
            "waypoint_pitch_cells": WAYPOINT_PITCH,
            "decide_ms": arena_cfg.reflex.decide_ms,
            "path_max_cells": arena_cfg.reflex.path_max_cells,
            "max_path_queries_per_window": arena_cfg.reflex.max_path_queries,
            "repath_ms": arena_cfg.reflex.repath_ms,
            "total": {
                "ticks": reflex_total.ticks,
                "intents": reflex_total.intents,
                "shots": reflex_total.shots,
                "path_queries": reflex_total.path_queries,
                "path_deferred": reflex_total.path_deferred,
                "path_cells": reflex_total.path_cells,
                "path_failures": reflex_total.path_failures,
                "traces": reflex_total.traces,
                "slope_refusals": reflex_total.slope_refusals,
            },
            "peak_per_pulse": {
                "ticks": reflex_peak.ticks,
                "path_queries": reflex_peak.path_queries,
                "path_deferred": reflex_peak.path_deferred,
                "path_cells": reflex_peak.path_cells,
                "path_failures": reflex_peak.path_failures,
                "traces": reflex_peak.traces,
                "intents": reflex_peak.intents,
                "shots": reflex_peak.shots,
                "slope_refusals": reflex_peak.slope_refusals,
            },
            "states_at_end": tacsim::arena::reflex::ReflexState::ALL
                .iter()
                .enumerate()
                .map(|(at, state)| (
                    state.as_str().to_string(),
                    distribution.get(at).copied().unwrap_or(0)
                ))
                .collect::<BTreeMap<String, u32>>(),
        },
        // 16b cut Y.2 (doc 16 §2 W8): reported, and not in the deterministic
        // block — the committed baseline is untouched by decision.
        "command": command.json(),
        // 16b cut U.2: the crate's reading on every platform, and Linux's own
        // beside it (`null` elsewhere). 16b cut J.2: the horizon index's bytes,
        // reported under 16c `T-36`'s memory bound and not compared (`memory` is
        // not a deterministic object).
        "memory": {
            "rss_start_kib": rss_start_kib,
            "rss_end_kib": rss_end_kib,
            "horizon_bytes": arena.definition.terrain.horizon_bytes(),
            "vmrss_start_kib": vmrss_start_kib,
            "vmrss_end_kib": vmrss_end_kib,
        },
        "thresholds": {
            "assert": asserting,
            "lens": lens,
            "cut_a_p99_ms": ARENA_P99_MS,
            "cut_s_p99_ms": ARENA_S_P99_MS,
            "engaged_p99_ms": knobs.engaged_p99_ms,
            "max_traces_per_pulse": knobs.max_traces_per_pulse,
            "max_share_ops_per_pulse": knobs.max_share_ops_per_pulse,
            "max_path_queries_per_pulse": knobs.max_path_queries_per_pulse,
            "max_cell_probes_per_pulse": knobs.max_cell_probes_per_pulse,
            "max_jam_checks_per_pulse": knobs.max_jam_checks_per_pulse,
            "max_ledger_bytes": knobs.max_ledger_bytes,
        },
    });
    let path = arena_report_path(
        &knobs.json,
        match reach {
            Reach::Ci => "arena",
            Reach::Authored => "arena-authored",
        },
    );
    write_report(&path, &report);
    println!("harness_arena_json: cohort={cohort} {}", path.display());

    if asserting {
        // 16b cut M.2 (`T-05`): the cohort's own substep divides the periods.
        assert!(
            substep_divides_periods,
            "the cohort's {substep} ms substep does not divide the three periods"
        );
        assert!(
            p99 <= f64::from(knobs.engaged_p99_ms),
            "per-wake p99 {p99:.3} ms is past harness.engaged_p99_ms {}",
            knobs.engaged_p99_ms
        );
        // **The lens, per wake and per window** (see [`ARENA_S_P99_MS`]).
        assert!(
            p99 <= ARENA_S_P99_MS,
            "per-wake p99 {p99:.3} ms is past the {ARENA_S_P99_MS} ms lens"
        );
        assert!(
            w99 <= ARENA_S_P99_MS,
            "per-window p99 {w99:.3} ms (500 ms windows of arena time) is past the \
             {ARENA_S_P99_MS} ms lens"
        );
        assert!(
            counts.bytes <= knobs.max_ledger_bytes,
            "ledger bytes {} is past harness.max_ledger_bytes {}",
            counts.bytes,
            knobs.max_ledger_bytes
        );
        // 16b cut S.1, doc 16 §2 W4 and W6: the two per-pulse perception bounds,
        // asserted against the **worst** wake of the run — the wake is the pulse
        // production runs (16b cut U.1).
        assert!(
            peak.traces <= knobs.max_traces_per_pulse,
            "traces in one wake {} is past harness.max_traces_per_pulse {} at \
             sensor_cells={} (doc 16 §2 W1: sensor ranges are short and bounded)",
            peak.traces,
            knobs.max_traces_per_pulse,
            sensor_cells()
        );
        // **16b cut U.2** (16c `T-36`; doc 16 §2 W4 names probes beside traces):
        // the cells those traces read, in the same shape.
        assert!(
            peak.cell_probes <= u64::from(knobs.max_cell_probes_per_pulse),
            "cell probes in one wake {} is past harness.max_cell_probes_per_pulse {} at \
             sensor_cells={} (doc 16 §2 W4: traces and cell probes per pulse)",
            peak.cell_probes,
            knobs.max_cell_probes_per_pulse,
            sensor_cells()
        );
        // **16b cut U.2** (16c `T-29`; doc 16 §9 item 4): the jammer scans, one
        // per unit between two clears of the memo since U.2 — the substep's two
        // rebuilds and every `StartupComplete` and `Impact` it resolves, so the
        // figure grows with the fire as well as with the field.
        assert!(
            peak.jam_checks <= knobs.max_jam_checks_per_pulse,
            "jam checks in one wake {} is past harness.max_jam_checks_per_pulse {} (doc 16 \
             §9 item 4: one fold per unit between two memo clears, and the memo is cleared \
             at each rebuild and after every startup and impact)",
            peak.jam_checks,
            knobs.max_jam_checks_per_pulse
        );
        assert!(
            peak.share_ops <= knobs.max_share_ops_per_pulse,
            "share operations in one wake {} is past harness.max_share_ops_per_pulse {} at \
             share_cells={} (doc 16 §2 W6)",
            peak.share_ops,
            knobs.max_share_ops_per_pulse,
            share_cells()
        );
        // **16b cut N.1**, doc 16 §1 A2's last per-pulse bound: *path queries per
        // pulse under their bound*. The arena's own budget is per `decide_ms`
        // window of arena time (D-16-2: a per-pulse throttle would bite
        // differently at each tempo), and a wake is at most a substep, inside one
        // window — so the per-wake figure the harness asserts is at most the
        // per-window figure the arena enforces.
        assert!(
            reflex_peak.path_queries <= knobs.max_path_queries_per_pulse,
            "path queries in one wake {} is past harness.max_path_queries_per_pulse {}",
            reflex_peak.path_queries,
            knobs.max_path_queries_per_pulse
        );
        // **16b cut Y.2** (doc 16 §2 W8): the span of control and the orders
        // per commander per minute, bounded by `[tacsim.command]`.
        command.assert_bounded();
    }
}

/// Write one report, creating its directory.
fn write_report(path: &std::path::Path, report: &serde_json::Value) {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).expect("the report directory is writable");
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(report).expect("the report encodes"),
    )
    .expect("the report is written");
}

// ---------------------------------------------------------------------------
// 16b cut J.1: the sight differential
// ---------------------------------------------------------------------------

/// **The shipped sight's reach, cells** (16b cut J.1): every cell within this
/// Chebyshev distance of an origin is a target of the sight differential.
const SIGHT_REACH_CELLS: i32 = 60;

/// **The sight differential, on the proving ground and on the relief** (16b cut
/// J.1; 16c N5 step 1): [`measure_sight`]. A correctness lane — its three
/// assertions hold on every pair whatever `TACSIM_ASSERT` says — whose timings
/// are printed and never asserted; JSON at `<stem>-sight.<ext>`, which the delta
/// does not read.
#[test]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
fn tacsim_harness_sight_differential() {
    measure_sight(&Knobs::from_env());
}

/// What one ground's sweep found (16b cut J.1).
#[derive(Default)]
struct SightSweep {
    origins: u64,
    pairs: u64,
    clear: u64,
    partial: u64,
    blocked: u64,
    /// `trace_cells(a, b)` over every ordered pair — the call perception made
    /// before the cut — timed as one pass, milliseconds.
    walk_ms: f64,
    /// `sight_only(a, b)` over the same pairs, timed as one pass, milliseconds.
    sight_only_ms: f64,
    /// The walk's `CellTrace::visited`, summed: **every cell of each line, both
    /// endpoints included**, whatever blocked it.
    visited_walk: u64,
    /// `CellSight::visited`, summed: **the interior cells read up to and
    /// including the first blocker, never an endpoint**. A different definition
    /// from `visited_walk`'s, so the two figures are not a ratio of like things.
    visited_sight_only: u64,
    /// `ArenaTerrain::sight(a, b)` over the same pairs — the arena's query through
    /// its horizon index (16b cut J.2) — timed as one pass, milliseconds.
    horizon_ms: f64,
    /// `HorizonSight::cell_probes`, summed (16b cut J.2): the cells the horizon
    /// index read — a walked run's up to the blocker, and a blocked proof's one —
    /// on `visited_sight_only`'s definition, so the two are a ratio of like things.
    visited_horizon: u64,
    /// `HorizonSight::horizon_probes`, summed (16b cut J.2): the blocks it read.
    horizon_probes: u64,
}

/// **One ground's sweep** (16b cut J.1): `origins` origins drawn off `rng` over
/// the `width` by `height` grid, each against every cell within
/// [`SIGHT_REACH_CELLS`] (clamped to the grid, the origin itself left out), in
/// both directions; the origin's eye at 150 cm, and at the biped's 900 cm on
/// every eighth origin.
///
/// Per origin, the walk's pass and `sight_only`'s pass over the same ordered
/// pairs are each timed whole, then every pair is checked untimed: `sight_only`
/// is exactly the walk from the canonical ends (each at its own height), it is
/// seen along exactly as the walk asked from `a` is, with one attenuation when
/// either is partial, and it read at least the walk's two endpoints fewer cells.
///
/// **16b cut J.2**: and a third pass, the arena's own query through the
/// terrain's horizon index ([`tacsim::arena::state::ArenaTerrain::sight`]),
/// timed whole and checked untimed: `sight_only`'s exact sight, and never more
/// cells read than it.
fn sight_sweep(
    grid: &tacsim::arena::state::ArenaTerrain,
    width: u16,
    height: u16,
    origins: u64,
    rng: &mut ChaCha8Rng,
) -> SightSweep {
    use mush_atlas::trace::{
        CellSight, CellTrace, EYE_HEIGHT_CM, Eye, Sight, canonical_ends, sight_only, trace_cells,
    };
    use tacsim::arena::horizon::HorizonSight;
    let seen = |sight: &Sight| match sight {
        Sight::Clear => (0_u8, 0_u8),
        Sight::Partial(pct) => (1, *pct),
        Sight::BlockedBy(_) => (2, 0),
    };
    let mut sweep = SightSweep {
        origins,
        ..SightSweep::default()
    };
    let mut pairs: Vec<(Eye, Eye)> = Vec::new();
    let mut walked: Vec<CellTrace> = Vec::new();
    let mut fast: Vec<CellSight> = Vec::new();
    let mut indexed: Vec<HorizonSight> = Vec::new();
    for origin in 0..origins {
        let cell = (rng.random_range(0..width), rng.random_range(0..height));
        let from = Eye {
            cell,
            height_cm: if origin % 8 == 4 { 900 } else { EYE_HEIGHT_CM },
        };
        pairs.clear();
        let (cx, cy) = (i32::from(cell.0), i32::from(cell.1));
        let span = |c: i32, side: u16| {
            (c - SIGHT_REACH_CELLS).max(0)..=(c + SIGHT_REACH_CELLS).min(i32::from(side) - 1)
        };
        for y in span(cy, height) {
            for x in span(cx, width) {
                let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
                    continue;
                };
                if (x, y) == cell {
                    continue;
                }
                let to = Eye::standing((x, y));
                pairs.push((from, to));
                pairs.push((to, from));
            }
        }
        walked.clear();
        let at = Instant::now();
        walked.extend(pairs.iter().map(|(a, b)| trace_cells(grid, *a, *b)));
        sweep.walk_ms += at.elapsed().as_secs_f64() * 1_000.0;
        fast.clear();
        let at = Instant::now();
        fast.extend(pairs.iter().map(|(a, b)| sight_only(grid, *a, *b)));
        sweep.sight_only_ms += at.elapsed().as_secs_f64() * 1_000.0;
        indexed.clear();
        let at = Instant::now();
        indexed.extend(pairs.iter().map(|(a, b)| grid.sight(*a, *b)));
        sweep.horizon_ms += at.elapsed().as_secs_f64() * 1_000.0;
        for (((a, b), (asked, quick)), through) in pairs
            .iter()
            .zip(walked.iter().zip(fast.iter()))
            .zip(indexed.iter())
        {
            // 16b cut J.2: the index is `sight_only`, to the byte, never dearer.
            assert_eq!(
                through.sight, quick.sight,
                "the horizon index and sight_only disagree from {a:?} to {b:?}"
            );
            assert!(
                through.cell_probes <= quick.visited,
                "{a:?} to {b:?}: the index read {} cells against sight_only's {}",
                through.cell_probes,
                quick.visited
            );
            let (lo, hi) = if canonical_ends(a.cell, b.cell) == (a.cell, b.cell) {
                (*a, *b)
            } else {
                (*b, *a)
            };
            let walk = trace_cells(grid, lo, hi);
            assert_eq!(
                quick.sight, walk.sight,
                "sight_only and the canonical walk disagree from {a:?} to {b:?}"
            );
            assert_eq!(
                seen(&quick.sight),
                seen(&asked.sight),
                "sight_only and the walk asked from {a:?} disagree to {b:?}: {:?} against {:?}",
                quick.sight,
                asked.sight
            );
            if matches!(quick.sight, Sight::Partial(_)) || matches!(asked.sight, Sight::Partial(_))
            {
                assert_eq!(walk.attenuation, asked.attenuation, "{a:?} to {b:?}");
            }
            assert!(
                quick.visited + 2 <= walk.visited,
                "{a:?} to {b:?}: sight_only read {} against the walk's {}",
                quick.visited,
                walk.visited
            );
            match quick.sight {
                Sight::Clear => sweep.clear += 1,
                Sight::Partial(_) => sweep.partial += 1,
                Sight::BlockedBy(_) => sweep.blocked += 1,
            }
            sweep.visited_walk += u64::from(asked.visited);
            sweep.visited_sight_only += u64::from(quick.visited);
            sweep.visited_horizon += u64::from(through.cell_probes);
            sweep.horizon_probes += u64::from(through.horizon_probes);
        }
        sweep.pairs += u64::try_from(pairs.len()).unwrap_or(u64::MAX);
    }
    sweep
}

/// **The sight differential** (16b cut J.1; 16c N5 step 1, Q26's "build the
/// cheap trace, re-measure").
///
/// 1. **The proving ground**: [`production_roster_with`] with the boot packs on,
///    the atlas records off `AtlasState`, the one shipped arena through
///    `tacsim::definitions::from_pack` — the ground `tests/sight.rs` walks — and
///    [`sight_sweep`] at `TACSIM_SIGHT_ORIGINS` origins.
/// 2. **The relief**: [`open_window`] at `TACSIM_SEED` and [`arena_ground`], the
///    relief [`measure_arena`] fields, at half as many origins.
///
/// One `harness_sight:` line per ground and one `harness_sight_json:` line.
fn measure_sight(knobs: &Knobs) {
    use tacsim::arena::state::ArenaTerrain;
    let mut rng = ChaCha8Rng::seed_from_u64(knobs.seed);
    let mut grounds: Vec<(&'static str, SightSweep)> = Vec::new();

    // ---- the proving ground ----
    let proving = {
        let tw = TestWorld::with_modules(production_roster_with(AtlasConfig::default(), true));
        let records = tw
            .world()
            .module_state::<AtlasState>()
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
    };
    let terrain = &proving.terrain;
    grounds.push((
        "proving_ground",
        sight_sweep(
            terrain,
            terrain.width(),
            terrain.height(),
            knobs.sight_origins,
            &mut rng,
        ),
    ));
    drop(proving);

    // ---- the relief ----
    let Window { tw, cells, .. } = open_window(knobs.seed);
    drop(tw);
    let (grid, _) = arena_ground(&cells);
    let relief = ArenaTerrain::new(SIDE, SIDE, CELL_CM, grid).expect("the relief's grid");
    grounds.push((
        "relief",
        sight_sweep(&relief, SIDE, SIDE, knobs.sight_origins / 2, &mut rng),
    ));

    let mut rows: Vec<serde_json::Value> = Vec::new();
    for (ground, sweep) in &grounds {
        assert!(sweep.pairs > 0, "the {ground} sweep walked nothing");
        // The two `visited` figures are different definitions: the walk's counts
        // every cell of the line, both endpoints included, and `sight_only`'s the
        // interior cells up to and including the first blocker.
        println!(
            "harness_sight: ground={ground} origins={} pairs={} clear={} partial={} blocked={} walk_ms={:.3} sight_only_ms={:.3} visited_walk={} visited_sight_only={} visited_horizon={} horizon_probes={}",
            sweep.origins,
            sweep.pairs,
            sweep.clear,
            sweep.partial,
            sweep.blocked,
            sweep.walk_ms,
            sweep.sight_only_ms,
            sweep.visited_walk,
            sweep.visited_sight_only,
            sweep.visited_horizon,
            sweep.horizon_probes,
        );
        rows.push(serde_json::json!({
            "ground": ground,
            "origins": sweep.origins,
            "pairs": sweep.pairs,
            "clear": sweep.clear,
            "partial": sweep.partial,
            "blocked": sweep.blocked,
            "walk_ms": sweep.walk_ms,
            "sight_only_ms": sweep.sight_only_ms,
            "visited_walk": sweep.visited_walk,
            "visited_sight_only": sweep.visited_sight_only,
            "horizon_ms": sweep.horizon_ms,
            "visited_horizon": sweep.visited_horizon,
            "horizon_probes": sweep.horizon_probes,
        }));
    }
    let report = serde_json::json!({
        "lane": "sight",
        "seed": knobs.seed,
        "reach_cells": SIGHT_REACH_CELLS,
        "grounds": rows,
    });
    let path = arena_report_path(&knobs.json, "sight");
    write_report(&path, &report);
    println!("harness_sight_json: {}", path.display());
}

// ---------------------------------------------------------------------------
// 16b cut U.3: the delta against a committed baseline
// ---------------------------------------------------------------------------

/// **Where the committed baselines live**: `crates/tacsim/tests/harness_baseline/`
/// (16b cut U.3, 16c `T-37`). A `.json` under `tests/` is not a cargo target,
/// and the underscore in the directory's name keeps anything from reading it as
/// a test module.
fn baseline_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("harness_baseline")
}

/// **This platform's baseline**: `arena.<os>.json`, `<os>` being
/// `std::env::consts::OS` (`windows`, `linux`, `macos`). One file per platform,
/// because the timing block is only ever compared on the host that blessed it
/// and a host is on one platform; the deterministic block is the same in every
/// file, which [`every_committed_baseline_agrees_on_the_deterministic_block`]
/// holds.
fn baseline_path() -> std::path::PathBuf {
    baseline_dir().join(format!("arena.{}.json", std::env::consts::OS))
}

/// **The deterministic block's objects**, compared leaf by leaf and exactly
/// (16b cut U.3; 16c Q29). Everything under each of these is a count or an
/// arena-time figure of one seeded run of an integer arena over an
/// integer-generated relief (16b cut U.1), so it is the same on every host: a
/// difference in any of them is a change in what the arena *did*, never in how
/// fast a box did it. `world` carries the wake count as `world.pulses`.
const DETERMINISTIC_OBJECTS: [&str; 9] = [
    "world",
    "substeps",
    "counters",
    "weapons",
    "perception.total",
    "perception.peak_per_pulse",
    "reflex.total",
    "reflex.peak_per_pulse",
    "reflex.states_at_end",
];

/// **The deterministic block's single fields**: the cohort's word, the wake
/// jitter the run was driven at (a stand-in for the timer's overshoot that
/// changes every count after it), and the five per-window counter peaks (16b
/// cut J.2 added the horizon index's block reads) — the window's
/// *milliseconds* are the timing block's.
const DETERMINISTIC_LEAVES: [&str; 7] = [
    "cohort",
    "loop.jitter_ms",
    "window.traces_peak",
    "window.cell_probes_peak",
    "window.horizon_probes_peak",
    "window.jam_checks_peak",
    "window.share_ops_peak",
];

/// **The thresholds the deterministic block leaves out**: `assert` and `lens`,
/// both the one fact of whether this run asserted (`lens` is `report-only` or
/// `per-wake-and-per-window` according to it). The baseline is blessed under
/// `TACSIM_ASSERT=1` and the CI job never asserts, so comparing either would
/// fail every run by hand without the flag and every CI run for a reason that
/// is not a change. Every other `thresholds.*` key is compared: a bound that
/// moved is a reviewed change and the baseline moves with it.
const RUN_ONLY_THRESHOLDS: [&str; 2] = ["thresholds.assert", "thresholds.lens"];

/// **The timing block** (16b cut U.3; 16c Q29): the wake p50 and p99 and the
/// per-window p99, as `(name printed, JSON pointer)`. Compared only on the host
/// that blessed the baseline, and failing only on a fresh figure more than
/// [`TIMING_SLOWER_PERCENT`] slower.
const TIMING_COMPARED: [(&str, &str); 3] = [
    ("tempos[0].p50_ms", "/tempos/0/p50_ms"),
    ("tempos[0].p99_ms", "/tempos/0/p99_ms"),
    ("window.ms_p99", "/window/ms_p99"),
];

/// **Printed on both sides and never compared**: the worst single wake (one
/// sample) and the warm-up's wall time (a figure the build and the page cache
/// shape as much as the arena does).
const TIMING_PRINTED: [(&str, &str); 2] = [
    ("tempos[0].max_ms", "/tempos/0/max_ms"),
    ("setup_ms.warm_up", "/setup_ms/warm_up"),
];

/// **How much slower a timing figure may be before the delta fails**, per cent
/// of the baseline's (16c Q29 at its default). A figure this much *faster* is
/// printed and passes: a faster box is not a regression, and the bless is one
/// command.
const TIMING_SLOWER_PERCENT: f64 = 20.0;

/// The word a missing field is compared as, so a key present on one side and
/// not the other is a difference and never a silent skip.
const ABSENT: &str = "<absent>";

/// Every leaf under `value` into `out`, keyed by its dotted path — `[i]` for an
/// array element.
fn flatten(path: &str, value: &serde_json::Value, out: &mut BTreeMap<String, serde_json::Value>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                flatten(&format!("{path}.{key}"), child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (at, child) in items.iter().enumerate() {
                flatten(&format!("{path}[{at}]"), child, out);
            }
        }
        leaf => {
            out.insert(path.to_string(), leaf.clone());
        }
    }
}

/// A dotted path as a JSON pointer.
fn pointer(path: &str) -> String {
    format!("/{}", path.replace('.', "/"))
}

/// **The deterministic block of one report**, flattened: every leaf of
/// [`DETERMINISTIC_OBJECTS`], every one of [`DETERMINISTIC_LEAVES`], and every
/// `thresholds.*` leaf but [`RUN_ONLY_THRESHOLDS`]. A named object or field the
/// report does not carry is recorded as [`ABSENT`] under its own name.
fn deterministic_block(report: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
    let mut block = BTreeMap::new();
    for path in DETERMINISTIC_OBJECTS.iter().chain(["thresholds"].iter()) {
        match report.pointer(&pointer(path)) {
            Some(value) => flatten(path, value, &mut block),
            None => {
                block.insert((*path).to_string(), serde_json::json!(ABSENT));
            }
        }
    }
    for path in DETERMINISTIC_LEAVES {
        let value = report
            .pointer(&pointer(path))
            .cloned()
            .unwrap_or_else(|| serde_json::json!(ABSENT));
        block.insert(path.to_string(), value);
    }
    for path in RUN_ONLY_THRESHOLDS {
        block.remove(path);
    }
    block
}

/// Every key on which two deterministic blocks differ, as one
/// `harness_delta: <path> baseline=<v> now=<v>` line each, in key order — a key
/// on one side only included.
fn block_differences(
    baseline: &BTreeMap<String, serde_json::Value>,
    now: &BTreeMap<String, serde_json::Value>,
) -> Vec<String> {
    let absent = serde_json::json!(ABSENT);
    let keys: std::collections::BTreeSet<&String> = baseline.keys().chain(now.keys()).collect();
    keys.into_iter()
        .filter_map(|key| {
            let was = baseline.get(key).unwrap_or(&absent);
            let is = now.get(key).unwrap_or(&absent);
            (was != is).then(|| format!("harness_delta: {key} baseline={was} now={is}"))
        })
        .collect()
}

/// **This host**, as a baseline's `blessed.host` names it: the platform, the
/// cores `std` reports, and the machine's name — `COMPUTERNAME` on Windows,
/// `HOSTNAME` elsewhere, then the first line of `/etc/hostname`, then
/// `unknown`. The timing block is compared only when all three match.
fn this_host() -> serde_json::Value {
    let named = |var: &str| {
        std::env::var(var)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let hostname = if cfg!(windows) {
        named("COMPUTERNAME")
    } else {
        named("HOSTNAME")
    }
    .or_else(|| {
        std::fs::read_to_string("/etc/hostname")
            .ok()
            .and_then(|text| text.lines().next().map(|l| l.trim().to_string()))
            .filter(|v| !v.is_empty())
    })
    .unwrap_or_else(|| "unknown".to_string());
    serde_json::json!({
        "os": std::env::consts::OS,
        "cpus": std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get),
        "hostname": hostname,
    })
}

/// A host as `<os>/<cpus>/<hostname>`.
fn host_words(host: Option<&serde_json::Value>) -> String {
    let field = |key: &str| {
        host.and_then(|h| h.get(key)).map_or_else(
            || "?".to_string(),
            |v| v.to_string().trim_matches('"').to_string(),
        )
    };
    format!("{}/{}/{}", field("os"), field("cpus"), field("hostname"))
}

/// **What one comparison found** (16b cut U.3): every line to print, the
/// deterministic differences, and the timing fields that were past the bound
/// (or missing) on a matching host.
struct Delta {
    lines: Vec<String>,
    differences: usize,
    slower: Vec<String>,
}

/// **The comparison itself**, a pure function of the two reports and this host,
/// so a unit test can hold what fails and what does not
/// ([`the_delta_fails_on_a_difference_and_a_slowdown_and_on_nothing_else`]).
fn delta(
    baseline: &serde_json::Value,
    fresh: &serde_json::Value,
    host: &serde_json::Value,
) -> Delta {
    let mut lines = block_differences(&deterministic_block(baseline), &deterministic_block(fresh));
    let differences = lines.len();
    if differences == 0 {
        lines.push(format!(
            "harness_delta: deterministic block equal ({} fields)",
            deterministic_block(fresh).len()
        ));
    }
    let figure =
        |doc: &serde_json::Value, at: &str| doc.pointer(at).and_then(serde_json::Value::as_f64);
    let word = |v: Option<f64>| v.map_or_else(|| "absent".to_string(), |v| format!("{v:.3}"));
    let blessed_on = baseline.pointer("/blessed/host");
    let mut slower = Vec::new();
    if blessed_on == Some(host) {
        for (name, at) in TIMING_COMPARED {
            match (figure(baseline, at), figure(fresh, at)) {
                (Some(was), Some(is)) if was > 0.0 => {
                    let moved = (is - was) / was * 100.0;
                    if moved > TIMING_SLOWER_PERCENT {
                        lines.push(format!(
                            "harness_delta: {name} baseline={was:.3} now={is:.3} slower than the \
                             baseline by {moved:.1}%, past the {TIMING_SLOWER_PERCENT}% bound"
                        ));
                        slower.push(name.to_string());
                    } else if moved < -TIMING_SLOWER_PERCENT {
                        lines.push(format!(
                            "harness_delta: {name} baseline={was:.3} now={is:.3}"
                        ));
                        lines.push(format!(
                            "harness_delta: {name} faster than the baseline by {:.1}%; re-bless \
                             when this holds",
                            -moved
                        ));
                    } else {
                        lines.push(format!(
                            "harness_delta: {name} baseline={was:.3} now={is:.3} ({moved:+.1}%, \
                             inside {TIMING_SLOWER_PERCENT}%)"
                        ));
                    }
                }
                (was, is) => {
                    lines.push(format!(
                        "harness_delta: {name} baseline={} now={} not comparable",
                        word(was),
                        word(is)
                    ));
                    slower.push(name.to_string());
                }
            }
        }
    } else {
        for (name, at) in TIMING_COMPARED {
            lines.push(format!(
                "harness_delta: {name} baseline={} now={}",
                word(figure(baseline, at)),
                word(figure(fresh, at))
            ));
        }
        lines.push(format!(
            "harness_delta: timing skipped, host differs (baseline {}, now {})",
            host_words(blessed_on),
            host_words(Some(host))
        ));
    }
    for (name, at) in TIMING_PRINTED {
        lines.push(format!(
            "harness_delta: {name} baseline={} now={} (printed, never compared)",
            word(figure(baseline, at)),
            word(figure(fresh, at))
        ));
    }
    Delta {
        lines,
        differences,
        slower,
    }
}

/// **Whether the fresh report predates the binary reading it** (16b cut U.3):
/// the delta reads whatever `harness-arena.json` is on disk, and nothing but
/// the order of the script's two lines (and the CI job's two steps) says the
/// lane that writes it ran first. A report older than this test binary was
/// written by an earlier build — the tree changed since — so comparing it, or
/// blessing it, would judge a change on the numbers of the run before it. The
/// script's and the job's two lines share one build, so their report is always
/// the newer. An unreadable time on either side is not a refusal: the check
/// guards against a stale file, it does not demand a filesystem that keeps
/// times.
fn report_predates_the_build(
    report_modified: Option<std::time::SystemTime>,
    binary_built: Option<std::time::SystemTime>,
) -> bool {
    matches!((report_modified, binary_built), (Some(report), Some(built)) if report < built)
}

/// Whether `TACSIM_BASELINE_BLESS` asks for a bless: exactly `1`, the
/// `TACSIM_ASSERT` rule, so a stray `0` or `false` never rewrites the file.
fn bless_requested(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim() == "1")
}

/// Seconds since the epoch as an ISO 8601 UTC instant, `YYYY-MM-DDTHH:MM:SSZ`
/// (the civil-from-days algorithm, so the harness takes no date crate).
fn iso_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// **Bless**: write `path` from the fresh report with a `blessed` stanza —
/// `commit` (`git rev-parse HEAD` now), `date` (ISO, UTC) and `host`
/// ([`this_host`]) — the panic budget's own shape: the variable rewrites the
/// file, and the comparison that follows reads it back.
fn bless_baseline(path: &std::path::Path, fresh: &serde_json::Value) {
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|commit| !commit.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let mut doc = fresh.clone();
    if let Some(map) = doc.as_object_mut() {
        map.insert(
            "blessed".to_string(),
            serde_json::json!({
                "commit": commit,
                "date": iso_utc(now_s),
                "host": this_host(),
            }),
        );
    }
    let mut text = serde_json::to_string_pretty(&doc).expect("the baseline encodes");
    text.push('\n');
    write_text(path, &text);
}

/// Write `text` to `path`, creating its directory.
fn write_text(path: &std::path::Path, text: &str) {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).expect("the baseline directory is writable");
    }
    std::fs::write(path, text).expect("the baseline is written");
}

/// **The delta against the committed baseline** (16b cut U.3; 16c `T-37`, Q29
/// at its default).
///
/// Reads the fresh CI-cohort report `tacsim_harness_arena_step` left at
/// `<stem>-arena.<ext>` and this platform's committed baseline ([`baseline_path`])
/// and compares them ([`delta`]):
///
/// * **the deterministic block**, exactly, every difference printed as
///   `harness_delta: <path> baseline=<v> now=<v>` before the test fails;
/// * **the timing block**, on the blessing host alone, failing only on a figure
///   more than 20 per cent slower; a figure more than 20 per cent faster says so
///   and passes; on another host every figure is printed and the block skipped.
///
/// No file for this platform prints how to write one and passes. A fresh report
/// older than this test binary is refused before anything is compared or
/// blessed ([`report_predates_the_build`]): run the lanes and the delta as the
/// script's two lines, in order, on one build. `TACSIM_BASELINE_BLESS=1` writes the file first ([`bless_baseline`]), and
/// never on an ordinary run. It is the one step that fails the CI job
/// (`.github/workflows/ci.yml`'s `tacsim-harness`), and the last line of
/// `scripts/harness-weekly.sh`.
///
/// **What a failure means.** A deterministic difference is a change in what the
/// arena did at the default knobs — stop and find the change, or re-bless it in
/// the same diff that made it and say why. A timing-only failure is re-run once
/// before it is believed: two runs of one tree on one box have differed by 16 per
/// cent at the p99 (16b cut N.1's own record).
#[test]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
fn tacsim_harness_delta_against_the_committed_baseline() {
    let knobs = Knobs::from_env();
    let fresh_path = arena_report_path(&knobs.json, "arena");
    let fresh: serde_json::Value = std::fs::read_to_string(&fresh_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| {
            panic!(
                "harness_delta: no fresh report at {} — run tacsim_harness_arena_step first \
                 (scripts/harness-weekly.sh runs it before this)",
                fresh_path.display()
            )
        });
    // A report left by an earlier build is not this tree's: refuse it before it
    // is compared or blessed (`report_predates_the_build`).
    let modified = |path: &std::path::Path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    };
    let report_modified = modified(&fresh_path);
    let binary_built = std::env::current_exe().ok().and_then(|exe| modified(&exe));
    assert!(
        !report_predates_the_build(report_modified, binary_built),
        "harness_delta: {} is older than this test binary, so an earlier build wrote it — \
         run scripts/harness-weekly.sh (the lanes first, then this delta, one build) rather \
         than comparing or blessing a stale report",
        fresh_path.display()
    );
    if let Some(age) = report_modified.and_then(|at| at.elapsed().ok()) {
        println!(
            "harness_delta: fresh report written {:.0} s ago{}",
            age.as_secs_f64(),
            if binary_built.is_some() {
                ", after this binary was built"
            } else {
                " (this binary's build time is unreadable; not checked)"
            }
        );
    }
    let path = baseline_path();
    if bless_requested(std::env::var("TACSIM_BASELINE_BLESS").ok().as_deref()) {
        bless_baseline(&path, &fresh);
        println!("harness_delta: blessed {}", path.display());
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        println!(
            "harness_delta: no baseline for {}; run with TACSIM_BASELINE_BLESS=1 to write one",
            std::env::consts::OS
        );
        return;
    };
    let baseline: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!(
        "harness_delta: fresh {} against {} (blessed at {} on {})",
        fresh_path.display(),
        path.display(),
        baseline
            .pointer("/blessed/commit")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?"),
        baseline
            .pointer("/blessed/date")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?")
    );
    let found = delta(&baseline, &fresh, &this_host());
    for line in &found.lines {
        println!("{line}");
    }
    assert_eq!(
        found.differences, 0,
        "the deterministic block moved in {} field(s) (the `harness_delta:` lines above): \
         the arena did something different at the default knobs. Find the change, or \
         re-bless in the diff that made it with TACSIM_BASELINE_BLESS=1 and say why",
        found.differences
    );
    assert!(
        found.slower.is_empty(),
        "timing past the {TIMING_SLOWER_PERCENT}% bound on the blessing host: {:?}. Re-run \
         once before believing it (two runs of one tree have differed 16% at the p99)",
        found.slower
    );
}

/// **Two hosts' baselines cannot drift** (16b cut U.3): every committed
/// `arena.*.json` carries one deterministic block, it names the platform its
/// file is named for, it carries every object and field the block reads, and at
/// least one file exists. Not ignored: it reads committed files and runs
/// nothing.
#[test]
fn every_committed_baseline_agrees_on_the_deterministic_block() {
    let dir = baseline_dir();
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("arena.") && name.ends_with(".json"))
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no committed baseline under {}: bless one with TACSIM_BASELINE_BLESS=1 \
         scripts/harness-weekly.sh",
        dir.display()
    );
    let mut first: Option<(std::path::PathBuf, BTreeMap<String, serde_json::Value>)> = None;
    for path in files {
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let doc: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let named_for = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("arena."))
            .and_then(|name| name.strip_suffix(".json"))
            .unwrap_or_default()
            .to_string();
        assert_eq!(
            doc.pointer("/blessed/host/os")
                .and_then(serde_json::Value::as_str),
            Some(named_for.as_str()),
            "{} was not blessed on the platform it is named for",
            path.display()
        );
        let block = deterministic_block(&doc);
        let missing: Vec<&String> = block
            .iter()
            .filter(|(_, value)| value.as_str() == Some(ABSENT))
            .map(|(key, _)| key)
            .collect();
        assert!(
            missing.is_empty(),
            "{} lacks part of the deterministic block: {missing:?}",
            path.display()
        );
        match &first {
            None => first = Some((path, block)),
            Some((first_path, first_block)) => {
                let differences = block_differences(first_block, &block);
                assert!(
                    differences.is_empty(),
                    "{} and {} disagree on the deterministic block:\n{}",
                    first_path.display(),
                    path.display(),
                    differences.join("\n")
                );
            }
        }
    }
}

/// **The script by hand runs the job's two lines** (16b cut U.3):
/// `scripts/harness-weekly.sh` carries both `cargo test` lines of the CI
/// `tacsim-harness` job verbatim, the job's comparison step is the delta and
/// nothing else, and the job itself sets no `TACSIM_ASSERT` (D-H-8, D-16-19: a
/// shared runner's speed is not a threshold) while the script does. The two
/// differ in their environment and nowhere else, so a lane added to one and not
/// the other, or an assertion switched on in CI, is a failure here and not a
/// drift somebody finds in a month.
#[test]
fn the_weekly_script_runs_the_ci_jobs_two_lines_verbatim() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf();
    let read = |rel: &str| {
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    };
    let ci = read(".github/workflows/ci.yml");
    let script = read("scripts/harness-weekly.sh");
    let job = ci
        .split_once("\n  tacsim-harness:")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split("\n\n  ").next())
        .expect("ci.yml has a `tacsim-harness` job");
    let job_lines: Vec<&str> = job
        .lines()
        .filter_map(|line| line.trim().strip_prefix("run: cargo test"))
        .collect();
    let script_lines: Vec<&str> = script
        .lines()
        .filter_map(|line| line.trim().strip_prefix("cargo test"))
        .collect();
    assert_eq!(
        job_lines.len(),
        2,
        "the job runs the lanes and then the delta:\n{job}"
    );
    assert_eq!(
        script_lines, job_lines,
        "scripts/harness-weekly.sh and the CI job run different cargo lines"
    );
    assert!(
        job_lines.last().is_some_and(|line| line
            .contains("tacsim_harness_delta_against_the_committed_baseline")
            && line.contains("--exact")),
        "the job's last step is the delta: {job_lines:?}"
    );
    assert!(
        !job.contains("TACSIM_ASSERT"),
        "the CI job asserts no absolutes (D-H-8, D-16-19):\n{job}"
    );
    assert!(
        script.contains("export TACSIM_ASSERT=1")
            && script.contains("export TACSIM_JSON=harness.json"),
        "the script asserts the absolutes and writes the job's report names"
    );
}

/// **The delta's own lens** (16b cut U.3): the comparison fails on a
/// deterministic difference and on a timing figure past the bound on the
/// blessing host, and on nothing else — not on the run-only thresholds, not on a
/// faster box, not on a slower figure from another host, and not on the
/// printed-only fields. And a report older than the binary reading it is
/// refused ([`report_predates_the_build`]).
#[test]
fn the_delta_fails_on_a_difference_and_a_slowdown_and_on_nothing_else() {
    let host = serde_json::json!({ "os": "testos", "cpus": 8, "hostname": "box" });
    let report = |p99: f64, blocked: u64, asserting: bool| {
        serde_json::json!({
            "cohort": "ci",
            "world": { "units": 300, "pulses": 600 },
            "loop": { "jitter_ms": 0 },
            "substeps": { "per_wake_min": 1 },
            "counters": { "blocked_events": blocked },
            "weapons": { "fired_events": 9 },
            "perception": {
                "total": { "traces": 5, "horizon_probes": 11 },
                "peak_per_pulse": { "traces": 2, "horizon_probes": 8 },
            },
            "window": {
                "ms_p99": p99, "traces_peak": 3, "cell_probes_peak": 4,
                "horizon_probes_peak": 9, "jam_checks_peak": 5, "share_ops_peak": 6,
            },
            "reflex": {
                "total": { "ticks": 7 }, "peak_per_pulse": { "ticks": 1 },
                "states_at_end": { "engage": 2 },
            },
            "tempos": [{ "p50_ms": 1.0, "p99_ms": p99, "max_ms": p99 * 3.0 }],
            "setup_ms": { "warm_up": 100.0 },
            "thresholds": {
                "assert": asserting,
                "lens": if asserting { "per-wake-and-per-window" } else { "report-only" },
                "engaged_p99_ms": 25,
            },
        })
    };
    let mut baseline = report(2.0, 10, true);
    baseline["blessed"] = serde_json::json!({ "host": host.clone() });

    // The same run, not asserting: nothing moved.
    let found = delta(&baseline, &report(2.0, 10, false), &host);
    assert_eq!(
        (found.differences, found.slower.len()),
        (0, 0),
        "{:?}",
        found.lines
    );
    // One count moved: one difference, named.
    let found = delta(&baseline, &report(2.0, 11, true), &host);
    assert_eq!(found.differences, 1, "{:?}", found.lines);
    assert!(
        found
            .lines
            .iter()
            .any(|line| line == "harness_delta: counters.blocked_events baseline=10 now=11"),
        "{:?}",
        found.lines
    );
    // A key on one side only is a difference too.
    let mut extra = report(2.0, 10, true);
    extra["weapons"]["hits"] = serde_json::json!(1);
    assert_eq!(delta(&baseline, &extra, &host).differences, 1);
    // A quarter slower on the blessing host fails: the p99 and the window p99.
    let found = delta(&baseline, &report(2.5, 10, true), &host);
    assert_eq!(found.slower.len(), 2, "{:?}", found.lines);
    // A quarter faster passes, and says so.
    let found = delta(&baseline, &report(1.5, 10, true), &host);
    assert!(found.slower.is_empty(), "{:?}", found.lines);
    assert!(
        found
            .lines
            .iter()
            .any(|line| line.contains("faster than the baseline by 25.0%; re-bless")),
        "{:?}",
        found.lines
    );
    // Twice as slow on another host is not compared.
    let other = serde_json::json!({ "os": "testos", "cpus": 8, "hostname": "elsewhere" });
    let found = delta(&baseline, &report(4.0, 10, true), &other);
    assert!(found.slower.is_empty(), "{:?}", found.lines);
    assert!(
        found
            .lines
            .iter()
            .any(|line| line.starts_with("harness_delta: timing skipped, host differs")),
        "{:?}",
        found.lines
    );
    // A report older than the binary reading it is refused; the same second,
    // a newer one, or an unreadable time on either side is not.
    let built = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_790_208_000);
    let second = std::time::Duration::from_secs(1);
    assert!(report_predates_the_build(Some(built - second), Some(built)));
    assert!(!report_predates_the_build(Some(built), Some(built)));
    assert!(!report_predates_the_build(
        Some(built + second),
        Some(built)
    ));
    assert!(!report_predates_the_build(None, Some(built)));
    assert!(!report_predates_the_build(Some(built - second), None));
    // And the bless variable is exactly `1`.
    assert!(bless_requested(Some("1")) && bless_requested(Some(" 1 ")));
    assert!(
        !bless_requested(None) && !bless_requested(Some("0")) && !bless_requested(Some("true"))
    );
    assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso_utc(1_790_208_000), "2026-09-24T00:00:00Z");
}

/// **The long run** (16b cut U.2; 16c `T-36`: *a long-run mode with an RSS
/// growth bound*; the M row's *the harness reports no due-heap depth*).
///
/// [`open_field`] on the CI cohort, then [`run_wakes`] — the 30-second lane's
/// own body — for `TACSIM_LONG_RUN_S` arena seconds after the warm-up, with
/// resident memory read after the warm-up and at every [`LONG_RUN_SAMPLE_S`] of
/// arena time. The claim it checks is doc 16 §4's: memory stays flat because the
/// ledger leaves, so what it reports beside the growth is what could grow — the
/// streamed ledger, the contact store, the due heap (`arena.due.len()`) and the
/// held losses.
///
/// Under `TACSIM_ASSERT`, first that the window was **the whole window**: the
/// match did not end (an elimination, or `[tacsim.match].max_duration_ms`
/// under a `TACSIM_LONG_RUN_S` past it) and arena time reached the mark, because
/// a growth figure over three minutes passed off as ten is a green claim the
/// lane never measured. Then, when a reading exists: the growth from the first
/// reading to the last is under `harness.max_rss_growth_kib`, and the ledger
/// under `harness.max_ledger_bytes`. It does **not** assert the p99: a ten-minute
/// run's tail on a shared box is a different number, and the lens is the
/// 30-second lane's.
#[allow(clippy::too_many_lines)]
fn measure_long_run(knobs: &Knobs) {
    let mut field = open_field(knobs, Reach::Ci);
    let warm_end = field.warm_end;
    let rss_start_kib = resident_kib();
    let vmrss_start_kib = vmrss_kib();
    let until_ms = warm_end.saturating_add(i64::from(knobs.long_run_s) * 1_000);
    // `(arena second, reading)` at every mark, the warm-up's end first.
    let mut samples: Vec<(i64, Option<u64>)> = vec![(0, rss_start_kib)];
    let mut next_mark = warm_end.saturating_add(LONG_RUN_SAMPLE_S * 1_000);
    let started = Instant::now();
    let run = run_wakes(&mut field, until_ms, |arena| {
        while arena.now_ms >= next_mark {
            samples.push(((next_mark - warm_end) / 1_000, resident_kib()));
            next_mark = next_mark.saturating_add(LONG_RUN_SAMPLE_S * 1_000);
        }
    });
    let wall_s = started.elapsed().as_secs_f64();
    let rss_end_kib = resident_kib();
    let vmrss_end_kib = vmrss_kib();
    let arena = &field.arena;
    #[allow(clippy::cast_precision_loss)]
    let arena_s = (arena.now_ms - warm_end) as f64 / 1_000.0;
    let rss_growth_kib: Option<i64> = match (rss_start_kib, rss_end_kib) {
        (Some(start), Some(end)) => Some(
            i64::try_from(end)
                .unwrap_or(i64::MAX)
                .saturating_sub(i64::try_from(start).unwrap_or(i64::MAX)),
        ),
        _ => None,
    };
    let rss_peak_kib = samples
        .iter()
        .filter_map(|(_, kib)| *kib)
        .chain(rss_end_kib)
        .max();
    let wakes = run.wakes.len();
    let (p50, _, p99, max) = Tempo {
        name: "arena_long",
        period_ms: field.arena_cfg.tempo.engaged_ms,
        ms: run.wakes.iter().map(|wake| wake.ms).collect(),
    }
    .stats();
    let ended = run
        .ended
        .map_or("none".to_string(), |end| format!("{:?}", end.reason));
    let cohort = field.cohort;
    assert!(wakes > 0, "the long run woke the arena no times at all");
    assert!(
        field.counts.bytes > 0,
        "the ledger streamed nothing: the long run measured an idle arena"
    );
    println!(
        "harness_long_run: cohort={cohort} arena_s={arena_s:.3} wakes={wakes} rss_start_kib={} rss_end_kib={} rss_growth_kib={} rss_peak_kib={} ledger_bytes={} ledger_chunks={} contacts_at_end={} due_heap_at_end={} held_losses_at_end={} ended={ended}{}",
        kib_word(rss_start_kib),
        kib_word(rss_end_kib),
        rss_growth_kib.map_or("absent".to_string(), |kib| kib.to_string()),
        kib_word(rss_peak_kib),
        field.counts.bytes,
        field.counts.chunks,
        arena.contacts.len(),
        arena.due.len(),
        arena.held_losses.len(),
        vmrss_words(vmrss_start_kib, vmrss_end_kib)
    );
    println!(
        "harness_long_run_samples: cohort={cohort} every_arena_s={LONG_RUN_SAMPLE_S} {}",
        samples
            .iter()
            .map(|(at, kib)| format!("{at}s={}", kib_word(*kib)))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "harness_long_run_wakes: cohort={cohort} wall_s={wall_s:.1} p50={p50:.3} p99={p99:.3} max={max:.3} (reported, never asserted: the lens is the 30-second lane's)"
    );
    println!(
        "harness_long_run_thresholds: cohort={cohort} assert={} max_rss_growth_kib={} max_ledger_bytes={}",
        knobs.assert, knobs.max_rss_growth_kib, knobs.max_ledger_bytes
    );
    let report = serde_json::json!({
        "what": "16b cut U.2: the CI cohort's field for TACSIM_LONG_RUN_S arena seconds, resident memory read every minute of arena time",
        "cohort": cohort,
        "arena_s": num(arena_s),
        "wakes": wakes,
        "wall_s": num(wall_s),
        "memory": {
            "rss_start_kib": rss_start_kib,
            "rss_end_kib": rss_end_kib,
            "rss_growth_kib": rss_growth_kib,
            "rss_peak_kib": rss_peak_kib,
            "vmrss_start_kib": vmrss_start_kib,
            "vmrss_end_kib": vmrss_end_kib,
            "every_arena_s": LONG_RUN_SAMPLE_S,
            "samples": samples
                .iter()
                .map(|(at, kib)| serde_json::json!({ "arena_s": at, "rss_kib": kib }))
                .collect::<Vec<_>>(),
        },
        "ledger": {
            "bytes": field.counts.bytes,
            "chunks": field.counts.chunks,
        },
        "at_end": {
            "contacts": arena.contacts.len(),
            "due_heap": arena.due.len(),
            "held_losses": arena.held_losses.len(),
            "shots_in_flight": arena.shots.len(),
            "ended": run.ended.map(|end| format!("{:?}", end.reason)),
        },
        "wake_ms": { "p50": num(p50), "p99": num(p99), "max": num(max), "asserted": false },
        "thresholds": {
            "assert": knobs.assert,
            "max_rss_growth_kib": knobs.max_rss_growth_kib,
            "max_ledger_bytes": knobs.max_ledger_bytes,
        },
    });
    let path = arena_report_path(&knobs.json, "arena-long");
    write_report(&path, &report);
    println!("harness_long_run_json: cohort={cohort} {}", path.display());

    if knobs.assert {
        // The bounds below are claims about `long_run_s` arena seconds, so a run
        // that stopped short proves nothing about them: refuse it by name.
        assert!(
            run.ended.is_none() && arena.now_ms >= until_ms,
            "the long run covered {arena_s:.3} of TACSIM_LONG_RUN_S={} arena seconds (ended={ended}): \
             its memory and ledger figures are not the long run's",
            knobs.long_run_s
        );
        if let Some(growth) = rss_growth_kib {
            assert!(
                growth <= i64::from(knobs.max_rss_growth_kib),
                "resident memory grew {growth} KiB over {arena_s:.0} arena seconds, past \
                 harness.max_rss_growth_kib {}",
                knobs.max_rss_growth_kib
            );
        }
        assert!(
            field.counts.bytes <= knobs.max_ledger_bytes,
            "ledger bytes {} over the long run is past harness.max_ledger_bytes {}",
            field.counts.bytes,
            knobs.max_ledger_bytes
        );
    }
}

/// 16b §6's verify lens for cut A: *the harness at 300 moving units stays under
/// 5 ms per pulse before perception and weapons exist.* It is a plan constant and
/// not a configured one — `[tacsim.harness].engaged_p99_ms` is the 25 ms ceiling
/// doc 16 sets for the finished arena — so it lives here, where the cut it bounds
/// is measured. Reported, never asserted: the arena it bounded had no sight.
const ARENA_P99_MS: f64 = 5.0;

/// **The lens**: 12 ms, asserted under `TACSIM_ASSERT=1` in release on the CI
/// cohort ([`Reach::Ci`]), **per wake and per 500 ms window** of arena time.
///
/// **Where it came from.** 16b cut S.1 set it when the sensor pass was built, as
/// under half the 25 ms `harness.engaged_p99_ms` doc 16 allows the *finished*
/// arena, leaving the weapons and the controller a budget; every cut since has
/// been measured against it and left it where it was, tightening it being the
/// owner's call at a review and not this file's. It was calibrated on a harness
/// that no longer exists: 500 ms `Engaged` pulses, each carrying up to two
/// substeps and — with the stagger at `id % pass_ms` — every one of the 300
/// sensor passes inside the first 300 ms of each second, over units that shared
/// one harness-local chassis with no locomotion, one sight and one gun.
///
/// **What it binds since 16b cut U.1.** The field is the fielded catalog
/// ([`COHORT`], through `match_::custody_unit`) over a relief with refusing
/// edges; the passes are spread by the seeded stagger; and the loop is the
/// task's own wake rule, so a wake is at most one substep and the per-wake p99
/// measures a quarter of the unit the number was set on. The per-window figure
/// folds the wakes back into the 500 ms windows the lens was calibrated on, and
/// both are asserted: the per-wake bound is the pulse production runs, the
/// per-window one keeps the number meaning what it meant. The per-pulse counter
/// bounds (`max_traces_per_pulse` and its siblings) are asserted per wake.
///
/// **At the authored reaches** ([`Reach::Authored`]) the measurement is
/// report-only. Cut S.2 recorded p99 23.3 ms and 13,133 traces in the peak pulse
/// there, and cut N.1 26.4 ms and 13,278 traces with the controller on the field,
/// against `harness.max_traces_per_pulse` of 4,000: the owner's choice between a
/// shorter authored reach and larger bounds is unchanged in kind, and the two
/// runs beside each other are what it is made on. [`ARENA_P99_MS`] stays in the
/// report as the record of what the arena cost before the passes ran.
const ARENA_S_P99_MS: f64 = 12.0;

// ---------------------------------------------------------------------------
// The cost run (16b cut N.2, N3; doc 16 §6 "The cost run"; 16a Q9b)
// ---------------------------------------------------------------------------
//
// **What this is.** The one mode of the harness that spends money. It turns doc 16
// §6's deliberation layer on for N minutes of arena play, asks a real provider for
// a real order every time a squad's trigger fires, and prints what that cost per
// unit-minute, per squad and per match — the table doc 16 can cite as the cost
// model for the LLM side of the design.
//
// **What keeps it off.** Three gates, all of which must be open (16a Q9b):
//
//   TACSIM_LLM_COST_RUN=1      the explicit flag, and nothing else counts as one
//   OPENROUTER_API_KEY         the key, from the environment, never from a file
//   TACSIM_LLM_COST_CAP_USD    a hard cap; the run stops BEFORE the call that
//                              would pass it
//
// Ordinary CI test runs set none of them. This test is deliberately **not**
// `#[ignore]`d: the workspace run proves it returns early. Without the flag it
// contacts nothing, writes nothing and returns, and
// `the_cost_run_is_off_unless_its_flag_says_1` asserts that environment. A separate
// exact-filter CI step sets the flag with `TACSIM_LLM_COST_MOCK=1` to exercise the
// report without a live provider; it does not run the environment assertion.
//
// **The mock lane.** `TACSIM_LLM_COST_MOCK=1` runs the whole thing against
// `mush_llm::MockProvider` with no key and no cap, which is how the plumbing was
// proved without spending anything: every call is a scripted tool use with a
// scripted `Usage`, so the table comes out with real arithmetic over fake tokens.
// That is the run recorded in the cut's ledger row; the owner's run against
// OpenRouter is theirs to make.

/// Whether a cost run was asked for. **Exactly `"1"`** — a variable that is set to
/// `true`, `yes` or the empty string is not a request, for the same reason
/// `TACSIM_ASSERT` is exact.
fn cost_run_requested(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.trim() == "1")
}

/// The cost run's own knobs.
#[derive(Clone, Debug)]
struct CostKnobs {
    /// Minutes of **arena** play to run for.
    minutes: f64,
    /// Units on the field.
    units: u32,
    /// The hard USD cap, if one was given.
    cap_usd: Option<f64>,
    /// Run against `MockProvider` instead of OpenRouter.
    mock: bool,
    /// The OpenRouter model to route to.
    model: String,
    /// The report path.
    json: String,
    /// The relief seed.
    seed: u64,
}

/// Cost-run defaults. Twenty-four units is the owner's own "12 v 12 fighting"
/// (`combat-scale-targets`), which at `SQUAD_SIZE` two is twelve squads under
/// command — enough for a per-squad table and far short of what 300 units would
/// spend.
const COST_UNITS: u32 = 24;
/// Five minutes, as 16b §8's cut N row asks.
const COST_MINUTES: f64 = 5.0;
/// The route's model, overridable so the owner can cost a different one.
const COST_MODEL: &str = "z-ai/glm-5.3-flash";

impl CostKnobs {
    fn from_env() -> CostKnobs {
        CostKnobs {
            minutes: std::env::var("TACSIM_LLM_COST_MINUTES")
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| *v > 0.0)
                .unwrap_or(COST_MINUTES),
            units: env_u32("TACSIM_LLM_COST_UNITS", COST_UNITS),
            cap_usd: std::env::var("TACSIM_LLM_COST_CAP_USD")
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| *v > 0.0),
            mock: std::env::var("TACSIM_LLM_COST_MOCK").is_ok_and(|v| v.trim() == "1"),
            model: std::env::var("TACSIM_LLM_COST_MODEL")
                .unwrap_or_else(|_| COST_MODEL.to_string()),
            json: std::env::var("TACSIM_JSON")
                .unwrap_or_else(|_| "target/harness.json".to_string()),
            seed: env_u64("TACSIM_SEED", DEFAULT_SEED),
        }
    }
}

/// `<stem>-cost.<ext>` beside the other two reports.
fn cost_report_path(given: &str) -> std::path::PathBuf {
    let path = report_path(given);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "harness".to_string());
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "json".to_string());
    path.with_file_name(format!("{stem}-cost.{ext}"))
}

#[test]
fn the_cost_run_is_off_unless_its_flag_says_1() {
    assert!(!cost_run_requested(None));
    assert!(!cost_run_requested(Some("")));
    assert!(!cost_run_requested(Some("0")));
    assert!(!cost_run_requested(Some("true")));
    assert!(!cost_run_requested(Some("yes")));
    assert!(cost_run_requested(Some("1")));
    assert!(cost_run_requested(Some(" 1 ")));
    // Ordinary CI runs leave the flag unset, so the cost run below returns early
    // and contacts nothing. The separate exact-filter mock step sets the flag
    // but excludes this environment assertion.
    assert!(
        !cost_run_requested(std::env::var("TACSIM_LLM_COST_RUN").ok().as_deref()),
        "TACSIM_LLM_COST_RUN is set in this environment: the cost run contacts a real \
         provider and spends real money, so an ordinary test run must not request it"
    );
}

/// **The cost run** (doc 16 §6; 16a Q9b).
///
/// Not `#[ignore]`d, and gated on `TACSIM_LLM_COST_RUN=1` instead: see the note at
/// the head of this section. Run it with
///
/// ```text
/// TACSIM_LLM_COST_RUN=1 TACSIM_LLM_COST_CAP_USD=1.00 OPENROUTER_API_KEY=... \
///   cargo test --release -p tacsim --test harness -- --nocapture tacsim_harness_llm_cost_run
/// ```
///
/// or with `TACSIM_LLM_COST_MOCK=1` in place of the key and the cap, which spends
/// nothing and proves the table.
#[test]
#[allow(clippy::too_many_lines)]
fn tacsim_harness_llm_cost_run() {
    use tacsim::arena::clock::Tempo as ArenaTempo;
    use tacsim::arena::state::{Arena, CustodyOut};
    use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};
    use tacsim::deliberation::{CostGuard, SquadBrief, cost_table, deliberate};

    if !cost_run_requested(std::env::var("TACSIM_LLM_COST_RUN").ok().as_deref()) {
        println!(
            "harness_cost_skipped: TACSIM_LLM_COST_RUN is not 1; no provider was contacted, no \
             report was written"
        );
        return;
    }
    let knobs = CostKnobs::from_env();

    // ---- the service: OpenRouter, or the mock ----
    let (llm, mock) = if knobs.mock {
        let mock = std::sync::Arc::new(mush_llm::MockProvider::new("openrouter").with_caps(
            mush_llm::ProviderCaps {
                tool_calling: true,
                forced_tool_choice: true,
                ..mush_llm::ProviderCaps::default()
            },
        ));
        let provider: std::sync::Arc<dyn mush_llm::LlmProvider> = mock.clone();
        let llm = std::sync::Arc::new(
            mush_llm::LlmService::builder()
                .provider("openrouter", provider)
                .route(
                    "tacsim_deliberate",
                    mush_llm::RouteConfig {
                        model: knobs.model.clone(),
                        max_tokens: 512,
                        timeout_s: 30,
                        reasoning: None,
                        reasoning_max_tokens: None,
                    },
                )
                .global_enabled(true)
                .build(),
        );
        (llm, Some(mock))
    } else {
        let key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
        assert!(
            !key.trim().is_empty(),
            "the cost run needs OPENROUTER_API_KEY in the environment (or \
             TACSIM_LLM_COST_MOCK=1 to prove the plumbing without spending anything)"
        );
        assert!(
            knobs.cap_usd.is_some(),
            "the cost run needs TACSIM_LLM_COST_CAP_USD: 16a Q9b's hard cap per run"
        );
        let mut providers = std::collections::HashMap::new();
        providers.insert(
            "openrouter".to_string(),
            mush_llm::ProviderConfig {
                kind: mush_llm::ProviderKind::Openai,
                base_url: "https://openrouter.ai/api/v1".to_string(),
                api_key_env: Some("OPENROUTER_API_KEY".to_string()),
                requests_per_minute: 120,
                referer: None,
                title: Some("rustmush tacsim cost run".to_string()),
                anthropic_version: None,
                price_input_per_mtok: None,
                price_output_per_mtok: None,
                tool_calling: true,
                forced_tool_choice: true,
                native_structured_output: false,
            },
        );
        let mut routes = std::collections::HashMap::new();
        routes.insert(
            "tacsim_deliberate".to_string(),
            mush_llm::RouteConfig {
                model: knobs.model.clone(),
                max_tokens: 512,
                timeout_s: 30,
                reasoning: Some("low".to_string()),
                reasoning_max_tokens: None,
            },
        );
        let cfg = mush_llm::LlmConfig {
            enabled: true,
            providers,
            routes,
            ..mush_llm::LlmConfig::default()
        };
        let llm = std::sync::Arc::new(
            mush_llm::LlmService::from_config(&cfg).expect("the cost run's llm service"),
        );
        (llm, None)
    };

    // ---- the field: the same generated relief and the same cohort the arena
    // harness runs on (16b cut U.1: the harness-local tank, sight, gun and
    // profile this built its field from are gone), at the CI reach, every unit
    // in the fighting lines and none of them seated — the cost of commanding a
    // field is what this measures, and a field that never fights never changes
    // a contact set.
    let Window {
        tw,
        cells: all_cells,
        ..
    } = open_window(knobs.seed);
    let catalog = fielded_catalog(&tw);
    drop(tw);
    let (grid, passable) = arena_ground(&all_cells);
    let definition = arena_definition(grid);
    let mut field_knobs = Knobs::from_env();
    field_knobs.units = knobs.units;
    field_knobs.fighting = knobs.units;
    field_knobs.seed = knobs.seed;
    let (units, _seats) = field(&field_knobs, &catalog, &passable, Reach::Ci);
    let placed = u32::try_from(units.len()).unwrap_or(0);
    let arena_cfg = ArenaConfig::default();
    let match_cfg = MatchConfig::default();
    let sensors_cfg = SensorsConfig::default();
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: knobs.seed,
        },
        &arena_cfg,
        &match_cfg,
        &sensors_cfg,
    )
    .expect("the cost run's arena");
    arena.clock.pin_tempo_for_tests(ArenaTempo::Engaged);
    let squads: Vec<(String, mush_core::ObjId)> = arena
        .squads
        .iter()
        .map(|roster| (roster.id.to_string(), roster.commander))
        .collect();
    let squad_units: std::collections::BTreeMap<String, Vec<mush_core::ObjId>> = arena
        .squads
        .iter()
        .map(|roster| (roster.id.to_string(), roster.units.clone()))
        .collect();

    // ---- the run ----
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for the cost run's provider calls");
    let period_ms = i64::from(arena_cfg.tempo.engaged_ms.max(1));
    #[allow(clippy::cast_possible_truncation)]
    let until_ms = (knobs.minutes * 60_000.0) as i64;
    let mut guard = CostGuard::new(knobs.cap_usd);
    let mut stopped_at_cap = false;
    let mut wall = 0_i64;
    let mut asked = 0_u64;
    let mut issued = 0_u64;
    let mut refused = 0_u64;
    let mut expired = 0_u64;
    // **16b cut Y.2**: this lane is the one that applies orders, so its tape is
    // W8's only non-zero reading; kept as the arena lanes keep theirs.
    let mut tape = Counts::default();
    let started = Instant::now();
    while arena.now_ms < until_ms {
        wall = wall.saturating_add(period_ms);
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            tape.absorb(&chunk.bytes);
        }
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        expired = expired.saturating_add(u64::from(out.reflex.orders_expired));
        if out.ended.is_some() {
            break;
        }
        for signal in &out.deliberation {
            let spent = llm.ledger().feature("tacsim_deliberate").cost_usd;
            if !guard.may_call(spent) {
                stopped_at_cap = true;
                break;
            }
            let key = signal.squad.to_string();
            let members = squad_units.get(&key).cloned().unwrap_or_default();
            let brief = SquadBrief::from_signal(
                1,
                signal,
                u16::try_from(members.len()).unwrap_or(u16::MAX),
            );
            if let Some(mock) = mock.as_ref() {
                mock.push_response(mock_order_response());
            }
            asked = asked.saturating_add(1);
            let answer = runtime.block_on(deliberate(&llm, &brief));
            guard.observe(spent, llm.ledger().feature("tacsim_deliberate").cost_usd);
            match answer {
                Ok(mut order) => {
                    order.issued_ms = arena.now_ms;
                    issued = issued.saturating_add(1);
                    for unit in members {
                        let epoch = arena
                            .epochs
                            .current(unit, tacsim::arena::state::Channel::Helm);
                        let intent = tacsim::arena::intents::Intent {
                            unit,
                            channel: tacsim::arena::state::Channel::Helm,
                            epoch,
                            kind: tacsim::arena::intents::IntentKind::Order(Box::new(
                                order.clone(),
                            )),
                            seq: 0,
                        };
                        let _ = tacsim::arena::intents::apply_intent(
                            &mut arena,
                            &intent,
                            tacsim::arena::intents::IntentSource::Seat,
                        );
                    }
                }
                Err(_) => refused = refused.saturating_add(1),
            }
        }
        if stopped_at_cap {
            break;
        }
    }
    let wall_s = started.elapsed().as_secs_f64();
    #[allow(clippy::cast_precision_loss)]
    let minutes = arena.now_ms as f64 / 60_000.0;
    // Whatever is still open leaves with the tape.
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        tape.absorb(&chunk.bytes);
    }
    // **16b cut Y.2** (doc 16 §2 W8): the same line the arena lanes print,
    // **report-only** — this lane runs against OpenRouter when it is not mocked,
    // and a provider's choices are not a bound this harness may fail on.
    let command = CommandFigures::of(
        &arena,
        &tacsim::config::CommandConfig::default(),
        tape.ordered_lines,
        &tape.ordered,
    );
    println!("{}", command.line("cost"));

    // ---- the table ----
    let table = cost_table(
        llm.ledger(),
        llm.audit_log(),
        &squads,
        placed,
        minutes,
        knobs.cap_usd,
        stopped_at_cap,
    );
    println!(
        "harness_cost_run: mode={} model={} units={placed} squads={} arena_minutes={minutes:.3} \
         wall_s={wall_s:.1} deliberate_min_ms={} asked={asked} issued={issued} refused={refused} \
         orders_expired={expired} cap_usd={} stopped_at_cap={stopped_at_cap}",
        if knobs.mock { "mock" } else { "openrouter" },
        table.model,
        table.squads,
        arena_cfg.deliberate_min_ms,
        knobs
            .cap_usd
            .map_or_else(|| "none".to_string(), |cap| format!("{cap:.4}"))
    );
    println!(
        "harness_cost_columns: {}",
        tacsim::deliberation::CostRow::COLUMNS.join(" ")
    );
    for row in &table.rows {
        println!("harness_cost_row: {}", row.line());
    }
    let path = cost_report_path(&knobs.json);
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).expect("the report directory is writable");
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&serde_json::json!({
            "what": "16b cut N.2 (N3) — the LLM cost model for one commanded match",
            "mode": if knobs.mock { "mock" } else { "openrouter" },
            "requests": { "asked": asked, "issued": issued, "refused": refused,
                          "orders_expired": expired,
                          "deliberate_min_ms": arena_cfg.deliberate_min_ms },
            "wall_seconds": num(wall_s),
            // 16b cut Y.2: W8's figures, reported and never asserted here.
            "command": command.json(),
            "columns": tacsim::deliberation::CostRow::COLUMNS,
            "table": table,
        }))
        .expect("the cost report encodes"),
    )
    .expect("the cost report is written");
    println!("harness_cost_json: {}", path.display());

    assert!(asked > 0, "the cost run asked for no order at all");
    assert_eq!(
        table.rows.len(),
        squads.len() + 1,
        "one row per squad plus the match total"
    );
    if let Some(cap) = knobs.cap_usd {
        let spent = table.total().map_or(0.0, |row| row.cost_usd);
        assert!(spent <= cap, "the run spent {spent} past its cap of {cap}");
    }
}

/// One scripted `issue_order` tool call, for the mock lane.
///
/// A short `hold` with a ten-second time to live: long enough to steer the squad,
/// short enough that a five-minute run sees the order run out, the
/// `order_expired` line journalled and the squad ask again — which is the whole
/// loop the cost model is a cost model of.
fn mock_order_response() -> mush_llm::CompletionResponse {
    mush_llm::CompletionResponse {
        content: vec![mush_llm::ContentBlock::ToolUse {
            id: "call_1".to_string(),
            name: "issue_order".to_string(),
            input: serde_json::json!({
                "objective": "engage",
                "ttl_ms": 10_000,
                "horizon_ms": 60_000,
                "hold_fire": false
            }),
        }],
        stop_reason: mush_llm::StopReason::ToolUse,
        usage: mush_llm::Usage {
            input_tokens: 420,
            output_tokens: 48,
            cache_read_tokens: 0,
            reasoning_tokens: 0,
            cost_usd: Some(0.000_08),
        },
        model: "z-ai/glm-5.3-flash".to_string(),
    }
}

// ---------------------------------------------------------------------------
// The fan-out lane (16b cut U.2; 16c `T-36`; 16b §11 "Risks": the fan-out row)
// ---------------------------------------------------------------------------
//
// **What this is.** The arena lanes above step a library and hand its frames to
// nobody, so the arena-to-session fan-out 16b §11 names as the risk had no
// number. This lane is the world side of it: the production roster, one match
// on the shipped proving ground, and the real egress seam pumped the way the
// world loop's `Wake::ArenaEgress` arm pumps it — except **one egress message
// per apply**, so an `ArenaOut::Frames` (one arena pulse) is exactly one apply
// and the per-pulse counts are exact rather than a function of how many pulses
// the host's timing let one drain absorb.
//
// 16c's sketch said "boards N real sessions"; the lobby has eight pods, so the
// twenty-four pilots are seated through the **staff** path instead. The unit,
// its placement, its seat and the match go through production verbs —
// `@unit/create`, `@behavior/add`, `@spatial/place/domain`, `@spatial/station`,
// `@spatial/assign`, `+focus` and `@arena/start` — and two steps are direct
// world calls, as the cockpit rig makes them: the tank's thing is created with
// `World::create` before `@unit/create` makes it a unit, and each pilot is
// walked to the tank's room with `World::move_to` before `+focus` (the rig's
// `walk_to`). The pilots then fly with the cockpit's own `STARTUP` and `SPEED`,
// so the frames the world sends are frames of tanks that move.

/// The boot-hydrated character → account map `mush-server` builds, as the
/// cockpit rig installs one (`tests/cockpit.rs`): a seat is refused to a
/// character the oracle cannot place.
#[derive(Default)]
struct FanOutAccounts(BTreeMap<ObjId, mush_core::AccountId>);

impl mush_core::AccountIdentity for FanOutAccounts {
    fn account_of(&self, character: ObjId) -> Option<mush_core::AccountId> {
        self.0.get(&character).copied()
    }
}

/// **The production roster the cockpit rig boots** (`tests/cockpit.rs`'s
/// `roster_with`): the composition `mush_server::boot` makes, with the starter
/// area (the simulator centre) and the game's boot packs (the proving ground)
/// on, and `[tacsim]` at its defaults with the layer enabled — a real `Quiet`,
/// `Alert` and `Engaged` tempo, because this lane measures the world side and
/// not the arena. The character sheet is left out for the reason that file
/// gives: its chargen payload handler cannot be registered twice in one
/// process.
fn fan_out_roster() -> Vec<Box<dyn GameModule>> {
    use tacsim::composition as comp;
    let cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    let llm = std::sync::Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let mut mods = comp::server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        std::sync::Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.push(comp::tacsim_module(cfg));
    mods.push(comp::content_module_with(true, true).expect("the content module composes"));
    mods
}

/// **The fan-out lane** (16b cut U.2, 16c `T-36`): [`measure_fan_out`] at
/// `TACSIM_FIGHTING` (24) pilots, JSON at `<stem>-fan-out.<ext>`; under
/// `TACSIM_ASSERT`, `panel_bytes_max <= harness.max_frame_bytes` and
/// `events_per_pulse_max <= harness.max_output_events_per_pulse`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
async fn tacsim_harness_fan_out() {
    measure_fan_out(&Knobs::from_env()).await;
}

/// How long the fan-out lane runs the match for, arena milliseconds.
const FAN_OUT_ARENA_MS: i64 = 30_000;

/// The speed the fan-out lane's pilots order once their power is up, kph —
/// below the stock tank's 54, so every order is accepted.
const FAN_OUT_KPH: i32 = 40;

/// What one apply of one egress message did (16b cut U.2).
#[derive(Default)]
struct Applied {
    /// Every output event the apply produced.
    events: u64,
    /// `arena.cockpit` panel keyframes among them.
    panels: u64,
    /// `serde_json::to_vec(data)` of each of those, in bytes.
    panel_bytes: Vec<u64>,
    /// Sentences on the pilots' line (the notices the frames carried).
    sys: u64,
}

/// Count one apply's output: the panel keyframes on the cockpit feed and their
/// encoded size, and the lines.
fn applied(out: &[(Audience, OutputEvent)]) -> Applied {
    let mut count = Applied {
        events: u64::try_from(out.len()).unwrap_or(u64::MAX),
        ..Applied::default()
    };
    for (_, event) in out {
        match event {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::arena_bridge::FRAME_FEED =>
            {
                count.panels += 1;
                let bytes = serde_json::to_vec(data).map_or(0, |encoded| encoded.len());
                count
                    .panel_bytes
                    .push(u64::try_from(bytes).unwrap_or(u64::MAX));
            }
            OutputEvent::Sys(_) => count.sys += 1,
            _ => {}
        }
    }
    count
}

/// Run one line as `actor` and answer what it said to them.
fn line(tw: &mut TestWorld, actor: ObjId, text: &str) -> String {
    let events = tw.run(actor, text);
    tw.seen_by(&events, actor).join("\n")
}

/// **The fan-out measurement** (16b cut U.2; 16c `T-36`: *a third lane that
/// boards N real sessions through `arena_bridge` and bounds bytes per frame and
/// output events per pulse at 24 pilots*).
///
/// 1. The production roster ([`fan_out_roster`]) with the account oracle the
///    cockpit rig installs, and the egress receiver taken as the world loop
///    takes it.
/// 2. `TACSIM_FIGHTING` players; as many things (made with `World::create`)
///    made line tanks with `@unit/create`, placed on the proving ground's muster
///    domain inside their side's entry zone — `content/arenas/proving_ground.json`
///    version 2's muster extended to twelve a side: red in column 6 facing east,
///    blue in column 249 facing west, slot one on row 97 and the rest
///    alternating two rows either side of it (95, 99, 93, 101, …), a clear
///    row between each pair — each given a cockpit station with
///    `@spatial/station` and its pilot with `@spatial/assign`; one world tick
///    for the lease mirror; each pilot walked to the tank's room with
///    `World::move_to` and `+focus` as the pilot.
/// 3. `@arena/start` as God; the match is `Running` and the registry has every
///    pilot seated in it.
/// 4. The egress pumped **one message per apply** for 30 arena seconds, every
///    apply timed, every `Frames` apply's output counted: the pilots `STARTUP`
///    at once and order `SPEED` when their frame says the power is up, so the
///    tanks the frames describe are moving.
/// 5. The report, and `@arena/abort` so the task stops.
///
/// **What it does not measure.** The musters stand 243 cells (2.4 km) apart and
/// the stock sight reaches 60 cells; at `FAN_OUT_KPH` the sides close about
/// 660 m in thirty seconds, so no pilot ever holds an enemy contact, a lock, a
/// reading or a weapon cycle, and no fire or damage notice is rendered. The two
/// bounds it asserts are therefore sized on the opening — every frame carries
/// teammates only, and the busiest pulse is the acquisition burst — and a
/// combat frame or an engaged pulse (16b §11's fan-out risk at twelve a side)
/// is not measured by this lane.
#[allow(clippy::too_many_lines)]
async fn measure_fan_out(knobs: &Knobs) {
    use tacsim::arena_bridge::{ArenaRegistry, EGRESS_PAYLOAD, MODULE};
    use tacsim::arena_task::ArenaOut;
    use tacsim::match_::{self, MatchState};

    let wanted = usize::try_from(knobs.fighting).unwrap_or(0);
    let per_side = wanted.div_ceil(2);
    let boot_at = Instant::now();
    let mut tw = TestWorld::with_modules(fan_out_roster());
    let boot_ms = boot_at.elapsed().as_secs_f64() * 1_000.0;

    // ---- (1) the pilots, their accounts and the egress ----
    let mut accounts = FanOutAccounts::default();
    let mut pilots: Vec<ObjId> = Vec::new();
    for n in 0..wanted {
        let pilot = tw.spawn_player(&format!("Pilot{n}"));
        accounts.0.insert(
            pilot,
            mush_core::AccountId::mint(&format!("account-{n}"), i64::try_from(n).unwrap_or(0))
                .expect("an account id"),
        );
        pilots.push(pilot);
    }
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") =
        mush_sim::AccountOracle::new(std::sync::Arc::new(accounts));
    let tasks = tw
        .world()
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .expect("the arena registry is composed");
    let mut egress = tasks
        .take_egress()
        .expect("the egress is the world loop's to take");

    // ---- (2) the tanks, placed and crewed ----
    let domain = {
        let arena = CanonicalId::new(match_::CENTRE_ARENA).expect("a canonical arena id");
        let room = tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .and_then(|store| store.arena_roster(&arena))
            .map(|roster| ObjId(roster.room))
            .expect("the proving ground's roster names its muster room");
        tw.world()
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.domain_of_host(room))
            .map(|record| record.id)
            .expect("the muster room hosts the proving ground's domain")
    };
    let mut red: Vec<ObjId> = Vec::new();
    let mut blue: Vec<ObjId> = Vec::new();
    let mut units: Vec<ObjId> = Vec::new();
    for (n, pilot) in pilots.iter().enumerate() {
        let is_red = n < per_side;
        let slot = i64::try_from(if is_red { n } else { n - per_side }).unwrap_or(0);
        // The shipped muster, extended (`content/arenas/proving_ground.json`,
        // version 2): column 6 for red and 249 for blue, facing each other, slot
        // one on row 97 and the rest alternating two rows either side of it —
        // 95, 99, 93, 101, … — with a clear cell between each pair.
        let row = 97 + if slot % 2 == 1 { -1 } else { 1 } * 2 * ((slot + 1) / 2);
        let (column, heading_mdeg) = if is_red {
            (6_i64, 90_000)
        } else {
            (249, 270_000)
        };
        let unit = tw
            .world_mut()
            .create(
                ObjKind::Thing,
                &format!("fan-out-tank-{n}"),
                GOD,
                Some(mush_core::LIMBO),
            )
            .expect("a unit object is created");
        let made = line(
            &mut tw,
            GOD,
            &format!("@unit/create #{}={}", unit.0, content::LINE_TANK),
        );
        assert!(
            made.contains("is now a tactical unit"),
            "@unit/create refused #{}: {made}",
            unit.0
        );
        participant(&mut tw, unit);
        let placed = line(
            &mut tw,
            GOD,
            &format!(
                "@spatial/place/domain #{}={},{{\"x_cm\":{},\"y_cm\":{},\"heading_mdeg\":{heading_mdeg}}}",
                unit.0,
                domain.0,
                column * 1_000 + 500,
                row * 1_000 + 500
            ),
        );
        assert!(
            placed.contains("Placed"),
            "#{} was not placed: {placed}",
            unit.0
        );
        let station = line(
            &mut tw,
            GOD,
            &format!(
                "@spatial/station #{}={},{},channels:{}",
                unit.0,
                match_::cockpit::STATION,
                match_::cockpit::STATION_ROLE,
                // Every channel `BOARD` grants (`cockpit_channels`): the helm,
                // the sensors and the gun, so `STARTUP` and `SPEED` are the
                // pilot's to give.
                tacsim::arena::state::Channel::ALL
                    .iter()
                    .map(|channel| channel.as_str())
                    .collect::<Vec<_>>()
                    .join("+")
            ),
        );
        let assigned = line(
            &mut tw,
            GOD,
            &format!(
                "@spatial/assign #{}={},#{}",
                unit.0,
                match_::cockpit::STATION,
                pilot.0
            ),
        );
        let seated = tw
            .world()
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.assignments.get(&unit))
            .is_some_and(|host| host.seats.values().any(|seat| seat.actor == *pilot));
        assert!(
            seated,
            "the staff seat path refused #{}'s cockpit to #{}: @spatial/station said `{station}`, \
             @spatial/assign said `{assigned}`",
            unit.0, pilot.0
        );
        units.push(unit);
        if is_red {
            red.push(unit);
        } else {
            blue.push(unit);
        }
    }
    // The control-lease mirror `+focus` reads is refreshed by `mush-sim`'s own
    // coarse tick: one second of world time, as the cockpit rig's boot does.
    tw.set_now(1);
    let _ = tw.fire_due(1_000);
    for (pilot, unit) in pilots.iter().zip(units.iter()) {
        // A station with no console of its own is reached where its host
        // stands (`mush_sim`'s `console_room`: *move there first*), so the pilot
        // walks to the tank's room before taking the seat, as a lobby pilot
        // walks to the team room before `BOARD`.
        let console = tw
            .world()
            .get(*unit)
            .and_then(|object| object.location)
            .expect("the tank stands somewhere");
        tw.world_mut()
            .move_to(*pilot, console)
            .expect("the pilot walks to the tank's console");
        let focus = line(
            &mut tw,
            *pilot,
            &format!("+focus #{}/{}", unit.0, match_::cockpit::STATION),
        );
        let focused = tw
            .world()
            .module_state::<mush_sim::FocusState>()
            .and_then(|state| state.focus_of_actor(*pilot))
            .is_some_and(|held| held.unit == *unit);
        assert!(
            focused,
            "+focus refused #{} at #{}'s cockpit: {focus}",
            pilot.0, unit.0
        );
    }

    // ---- (3) the match ----
    let ids = |side: &[ObjId]| {
        side.iter()
            .map(|unit| format!("#{}", unit.0))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let started = line(
        &mut tw,
        GOD,
        &format!(
            "@arena/start {}={}/{}",
            match_::PROVING_GROUND,
            ids(&red),
            ids(&blue)
        ),
    );
    assert!(started.contains("is running on"), "@arena/start: {started}");
    let state = match_::centre_of(tw.world()).map(|centre| centre.state);
    assert_eq!(state, Some(MatchState::Running), "{started}");
    let match_id = match_::centre_of(tw.world())
        .and_then(|centre| centre.match_id)
        .expect("the centre adopted the match");
    let bound = pilots
        .iter()
        .filter(|pilot| {
            tw.world()
                .module_state::<ArenaRegistry>()
                .and_then(|registry| registry.seat_of_pilot(**pilot))
                .is_some_and(|(held, _)| held == match_id)
        })
        .count();
    assert_eq!(bound, wanted, "every pilot is on the arena's roster");
    for pilot in &pilots {
        let said = line(&mut tw, *pilot, "startup");
        assert!(
            said.contains("Startup sequence begun"),
            "#{}'s STARTUP was refused: {said}",
            pilot.0
        );
    }

    // ---- (4) the pump ----
    let mut world_ms: Vec<f64> = Vec::new();
    let mut events_per_pulse: Vec<u64> = Vec::new();
    let mut panel_bytes: Vec<u64> = Vec::new();
    let mut sys_lines = 0_u64;
    let mut pulses = 0_u64;
    let mut applies = 0_u64;
    // The busiest pulse: its arena millisecond, its events, its panels and its
    // lines — what a reader needs to say *why* it was the busiest.
    let mut busiest: (i64, u64, u64, u64) = (0, 0, 0, 0);
    let mut first_ms: Option<i64> = None;
    let mut last_ms = 0_i64;
    let mut moving: Vec<bool> = vec![false; wanted];
    let deadline = Instant::now() + std::time::Duration::from_secs(180);
    while last_ms < FAN_OUT_ARENA_MS {
        assert!(
            Instant::now() < deadline,
            "the arena reached {last_ms} ms of {FAN_OUT_ARENA_MS} in three minutes of wall time"
        );
        let message = match egress.try_recv() {
            Ok(message) => message,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                continue;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                panic!("the egress closed with the match at {last_ms} ms")
            }
        };
        let clock = match &message {
            ArenaOut::Frames { now_ms, .. } => Some(*now_ms),
            _ => None,
        };
        assert!(tasks.stage(message), "the staging table took the message");
        let at = Instant::now();
        let out = tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: smol_str::SmolStr::new_static(MODULE),
            actor: GOD,
            correlation: 0,
            payload: EGRESS_PAYLOAD.to_owned(),
        });
        world_ms.push(at.elapsed().as_secs_f64() * 1_000.0);
        tasks.stamp_ledger_writes(1);
        tasks.ack_ledger_upto(1);
        applies += 1;
        if let Some(now_ms) = clock {
            let counted = applied(&out);
            if counted.events > busiest.1 {
                busiest = (now_ms, counted.events, counted.panels, counted.sys);
            }
            pulses += 1;
            events_per_pulse.push(counted.events);
            panel_bytes.extend(counted.panel_bytes);
            sys_lines += counted.sys;
            first_ms.get_or_insert(now_ms);
            last_ms = last_ms.max(now_ms);
        }
        // A pilot whose frame says the power is up makes way, once.
        for (n, pilot) in pilots.iter().enumerate() {
            let Some(flag) = moving.get_mut(n) else {
                continue;
            };
            if *flag {
                continue;
            }
            let online = tw
                .world()
                .module_state::<ArenaRegistry>()
                .and_then(|registry| registry.last_frame(*pilot))
                .is_some_and(|held| held.frame.power == tacsim::arena::state::Power::Online);
            if online {
                let said = line(&mut tw, *pilot, &format!("speed {FAN_OUT_KPH}"));
                assert!(
                    said.contains("Making"),
                    "#{}'s SPEED was refused: {said}",
                    pilot.0
                );
                *flag = true;
            }
        }
    }
    let underway = moving.iter().filter(|flag| **flag).count();

    // ---- (5) the report ----
    #[allow(clippy::cast_precision_loss)]
    let arena_s = (last_ms - first_ms.unwrap_or(0)).max(1) as f64 / 1_000.0;
    let frames = u64::try_from(panel_bytes.len()).unwrap_or(u64::MAX);
    let bytes_total: u64 = panel_bytes.iter().sum();
    #[allow(clippy::cast_precision_loss)]
    let panel_bytes_mean = if panel_bytes.is_empty() {
        0.0
    } else {
        bytes_total as f64 / panel_bytes.len() as f64
    };
    let panel_bytes_max = panel_bytes.iter().copied().max().unwrap_or(0);
    #[allow(clippy::cast_precision_loss)]
    let per_pilot_per_s = bytes_total as f64 / wanted.max(1) as f64 / arena_s;
    #[allow(clippy::cast_precision_loss)]
    let events_mean = if events_per_pulse.is_empty() {
        0.0
    } else {
        events_per_pulse.iter().sum::<u64>() as f64 / events_per_pulse.len() as f64
    };
    let events_max = events_per_pulse.iter().copied().max().unwrap_or(0);
    let (w50, w95, w99, wmax) = Tempo {
        name: "world_apply",
        period_ms: 0,
        ms: world_ms.clone(),
    }
    .stats();
    println!(
        "harness_fan_out: pilots={wanted} units={} arena_s={arena_s:.3} pulses={pulses} frames={frames} panel_bytes_mean={panel_bytes_mean:.1} panel_bytes_max={panel_bytes_max} panel_bytes_per_pilot_per_arena_s={per_pilot_per_s:.1} events_per_pulse_mean={events_mean:.2} events_per_pulse_max={events_max} world_ms_p50={w50:.3} world_ms_p95={w95:.3} world_ms_p99={w99:.3} world_ms_max={wmax:.3}",
        units.len()
    );
    println!(
        "harness_fan_out_detail: applies={applies} sys_lines={sys_lines} underway={underway} busiest_pulse_ms={} busiest_panels={} busiest_lines={} boot_ms={boot_ms:.1} match={}",
        busiest.0, busiest.2, busiest.3, match_id.0
    );
    let asserting = knobs.assert;
    println!(
        "harness_fan_out_thresholds: assert={asserting} max_frame_bytes={} max_output_events_per_pulse={}",
        knobs.max_frame_bytes, knobs.max_output_events_per_pulse
    );
    let report = serde_json::json!({
        "what": "16b cut U.2: one match of 24 staff-seated pilots on the proving ground, the real egress applied one message per apply",
        "pilots": wanted,
        "units": units.len(),
        "arena_s": num(arena_s),
        "pulses": pulses,
        "applies": applies,
        "frames": frames,
        "underway": underway,
        "panel_bytes": {
            "mean": num(panel_bytes_mean),
            "max": panel_bytes_max,
            "total": bytes_total,
            "per_pilot_per_arena_s": num(per_pilot_per_s),
        },
        "events_per_pulse": {
            "mean": num(events_mean),
            "max": events_max,
            "busiest": {
                "arena_ms": busiest.0,
                "events": busiest.1,
                "panels": busiest.2,
                "lines": busiest.3,
            },
        },
        "sys_lines": sys_lines,
        "world_ms": {
            "p50": num(w50),
            "p95": num(w95),
            "p99": num(w99),
            "max": num(wmax),
        },
        "thresholds": {
            "assert": asserting,
            "max_frame_bytes": knobs.max_frame_bytes,
            "max_output_events_per_pulse": knobs.max_output_events_per_pulse,
        },
    });
    let path = arena_report_path(&knobs.json, "fan-out");
    write_report(&path, &report);
    println!("harness_fan_out_json: {}", path.display());

    // The match is stopped so the task does not outlive the lane, and its
    // check-in is pumped through so the world sees it end.
    let aborted = line(&mut tw, GOD, &format!("@arena/abort {}", match_id.0));
    assert!(aborted.contains("is stopping"), "@arena/abort: {aborted}");
    let stop_by = Instant::now() + std::time::Duration::from_secs(10);
    while Instant::now() < stop_by && tasks.is_live(match_id) {
        match egress.try_recv() {
            Ok(message) => {
                if tasks.stage(message) {
                    let _ = tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
                        module: smol_str::SmolStr::new_static(MODULE),
                        actor: GOD,
                        correlation: 0,
                        payload: EGRESS_PAYLOAD.to_owned(),
                    });
                    tasks.stamp_ledger_writes(1);
                    tasks.ack_ledger_upto(1);
                }
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
        }
    }

    // Unconditionally: the lane measured what it said it did.
    assert_eq!(
        wanted,
        pilots.len(),
        "the pilots asked for were the pilots seated"
    );
    assert!(frames > 0, "the world sent no cockpit frame at all");
    assert!(
        underway > 0,
        "no pilot's tank came up and made way: the frames were of a parked field"
    );
    if asserting {
        assert!(
            panel_bytes_max <= u64::from(knobs.max_frame_bytes),
            "a cockpit keyframe of {panel_bytes_max} bytes is past harness.max_frame_bytes {}",
            knobs.max_frame_bytes
        );
        assert!(
            events_max <= u64::from(knobs.max_output_events_per_pulse),
            "one arena pulse produced {events_max} output events, past \
             harness.max_output_events_per_pulse {}",
            knobs.max_output_events_per_pulse
        );
    }
}
