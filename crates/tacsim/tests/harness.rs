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
//! 0). Every wake is also folded into 500 ms windows of arena time, and the lens
//! binds both: 20 ms per wake on both cohorts, and per window 22 ms on the CI
//! cohort and 32 ms on the authored one — the production VPS's figures since 16c
//! `T-109` (below); 12, 12 and 20 on ALIEN18 before it (17.5 ms at 16b cut
//! J.3b, 20 since J.3c on the owner's word). Two runs: `tacsim_harness_arena_step` at the CI reach (sight
//! scaled 60 to `TACSIM_SENSOR_CELLS`, 24; mesh 30 to `TACSIM_SHARE_CELLS`, 8),
//! and `tacsim_harness_arena_authored_reach` at the reaches as authored; both
//! asserted under `TACSIM_ASSERT` since 16b cut J.3b, the authored one at its own
//! per-window lens ([`window_lens_ms`]). The terrain's move costs are the
//! harness's own one table (`harness_move_cost`) for every row: the cohort's
//! window is built through `ArenaTerrain::new`, which prices every row at the
//! authored cell (16b cut AE.1), so the cohort measures the arena and not the
//! shipped locomotion table.
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
//! counters, the weapons, the perception and motor totals and peaks, the
//! machines' states at the end, the window peaks, the thresholds) exactly, and **the timing block** (the wake p50
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
//! share operations (256). The window was over, so the authored lane stayed
//! **report-only pending the owner's Q26** (the 600 m sight, the bounds, the
//! 300-unit target, or N5's step 3, the work budget) until the owner answered
//! it, as the next paragraph says. The CI cohort is asserted as before; a run of
//! its lane at other sight knobs is a sizing run of another field, and skips the
//! CI cohort's warm-up wreck check ([`open_field`]).
//!
//! **The authored lane asserted since 16b cut J.3b** (16c Q26, answered by the
//! owner on 2026-09-25): the reach and the 300-unit target kept,
//! `harness.max_share_ops_per_pulse` 768, the per-window lens 17.5 ms (20 ms
//! since J.3c, the next paragraph) and the per-wake lens 12 ms, asserted by
//! hand on ALIEN18 (the owner's host) through `scripts/harness-weekly.sh` on
//! every run since, and never in CI (billing-blocked, no `TACSIM_ASSERT`, and
//! — until 17b cut M4.5, whose bless committed `arena.linux.json` — no Linux
//! baseline); the first three asserted runs read per window p99 15.339, 14.057
//! and 14.732 ms and per wake 8.099, 7.785 and 8.150 ms.
//!
//! **The lens at 20 ms since 16b cut J.3c** (the owner, later on 2026-09-25:
//! *bump the lens up to 20 to give us some headroom but keep a reasonable
//! boundary*). The J.3b gate's own run had read 16.298 ms per window, which
//! 17.5 held by only 7.4 per cent, so a loaded week could have missed on noise
//! alone; 20 ms ([`ARENA_AUTHORED_WINDOW_P99_MS`]) held the worst asserted run
//! then on record, 16.298, by 22.7 per cent, is the figure the 300-unit harness
//! gate was first
//! proposed at on 2026-09-16 (before doc 16 set the finished arena's ceiling at
//! 25), and stays under that 25 ms `engaged_p99_ms` ceiling — the check that
//! keeps it a boundary, as the owner asked, pinned by
//! [`the_lens_is_the_cohorts_own`]. Nothing else moved: no bound, no baseline
//! (the CI report's `window_p99_ms` stays 12.0), no reach, no target.
//!
//! **The ratio check retired since 16b cut J.3d** (the owner, 2026-09-29): the
//! guard that the lens clears the worst asserted run by the harness's 16 per
//! cent spread could hold only while the record stayed under 17.241 ms, and
//! cuts AE and AF read 17.6 to 18.0 under the 20 ms lens five times; the record
//! now follows the worst asserted run that passed (18.022) and
//! [`the_lens_is_the_cohorts_own`] holds of the record only that the lens is
//! above it (its exact pins and the ceiling check stand).
//!
//! **The lenses follow the production host since 16c `T-109`** (the owner,
//! 2026-10-09: *scale the wake and window limits to the VPS, since that's where
//! the game will run*; the figures confirmed the same day). Every figure above
//! was measured on the owner's desktop, ALIEN18, and the conclav.io VPS that
//! serves the game (4 vCPU AMD EPYC 7543P) is 1.75 to 2 times slower a core;
//! with `T-87` paid it read the authored wake p99 at 12.8 to 22.6 ms and the
//! window at 24.6 to 27.3, the CI wake at 8.8 to 10.0 and the window at 15.9 to
//! 18.9. The lenses moved to the per-wake 20 ms ([`ARENA_S_P99_MS`], the
//! owner's figure, still half the ceiling), the CI window 22
//! ([`ARENA_CI_WINDOW_P99_MS`], split from the per-wake lens it shared) and the
//! authored window 32 ([`ARENA_AUTHORED_WINDOW_P99_MS`]) — each window the
//! harness's 16 per cent over the VPS's worst ordinary run — and
//! `harness.engaged_p99_ms` to 40 so the authored window stays a boundary under
//! it; [`the_lens_is_the_cohorts_own`] pins all four. The committed
//! `arena.linux.json` is blessed on the VPS under its own host name, so the
//! delta's timing gate is live on the production host and dormant elsewhere.
//!
//! **The lance lane since 16b cut Z.1** (16c N6, *lances*). One more ignored
//! lane, `tacsim_harness_lance`: `TACSIM_LANCE_UNITS` (24) units of the fielded
//! catalog, twelve a side on the **shipped proving ground** at the authored
//! reach, fifty-four cells apart astride the crossroads, dealt into lances of
//! four by `[tacsim.arena].squad_size`, under a scripted commander a side that
//! is its own lowest unit, for `TACSIM_LANCE_ARENA_S` (120) arena seconds
//! through the same wake loop ([`run_wakes`], which since Z.1 steps a
//! [`Stepper`] and hands each wake's phase 2 a script's intents). Under
//! `TACSIM_ASSERT` it asserts W8 — the span and the orders per commander per
//! minute — against `[tacsim.command]`, the tape against the script and the
//! opening round at any knob, **and**, at the lane's own defaults only, the span
//! and the busiest minute pinned to the fight's own figures (the squads a side
//! at Z.1; since 16b cut AP.2 the busiest minute one fewer and the span two
//! fewer, the match ending at 111,250 ms before minute one's last marks; since
//! 17b cut M4.5, the fight at the ROM, the busiest minute the squads a side and
//! the span one fewer, no end inside the window, both sides' first squads
//! released; off the defaults the fight decides those two figures); its timing
//! is **report-only**, printed beside the
//! authored lane's per-wake and per-window lenses (20 and 32 ms since 16c
//! `T-109`, 12 and 20 before). Its JSON
//! (`<stem>-lance.<ext>`) is outside the delta. `z1_the_lance_lane_runs_small`,
//! un-ignored, runs it at one lance a side for fourteen arena seconds (ten
//! until 17b cut M4.5: the ROM closes before it fires) with the
//! any-knob checks asserted.
//!
//! **The Hold lane since 16b cut O.1** (16c N11; plan item 27). The lance
//! lane's shape at `MatchMode::Hold` on the shipped ground's own rule, `hold_ms`
//! raised to the loader's maximum so no win ends the window, cohort
//! `lance_hold`, JSON `<stem>-lance_hold.<ext>`. It is **not** ignored and
//! **not** in the weekly script or the CI job: `tacsim_harness_hold_lane`
//! returns at once unless `TACSIM_HOLD_LANE` is exactly `1`, and it asserts
//! nothing. Whether it joins the weekly job is Q-O3 for the owner. Both lanes
//! print a `harness_lance_goal_cost` line (report-only), which is how the two
//! are read side by side.
//!
//! **The minds lane since 17b cut M4.2** (the M4 row's (ii); doc 17 §18,
//! §22's M4). One more ignored lane on the one ignore string,
//! `tacsim_harness_arena_minds`: the relief cohort fielded as
//! `Controller::Mind` hulls in four variants run FILL FIRST —
//! `fill_authored` (thirty-six Class I hulls a side on
//! `PRODUCTION_FILL_ENGINE`, the rest the ROM), `stock_ci` and
//! `stock_authored` (every hull the ROM, Class 0, native) and
//! `ceiling_authored` (two `heavy_mk1` a side at Class III on the load mind
//! `tests/support/load_iii.rhai`, calibrated to 4,300–4,499 ops + fees) —
//! each stepped with `Arena::step` and a private `MindPool` of
//! `[tacsim.minds].workers` threads on the arena task's own policy (the
//! batch submitted after the publish, awaited at the next wake's head and
//! before a due snapshot's encode), a seeded quarter of its wakes one substep
//! late (`TACSIM_MINDS_JITTER_MS` 250 on a `TACSIM_MINDS_JITTER_PER_MILLE`
//! 250 fraction; on the lattice, not the row's literal 60: the section head
//! says why) so the inline path is measured, every think's ops, fees and
//! microseconds recorded, `view_us` sampled on forty wakes, the lines
//! `harness_minds_{pool,thinks,ops,cpu,verdict}` beside the arena lane's
//! reprinted, and every check collected into one `harness_minds_verdict`
//! line a variant: `held=` over exactly the plan's listed asserts (wake p99
//! the per-wake lens — 20 ms since 16c `T-109`, 12 before — over the
//! unjittered wakes since 16c `T-108` (every wake before, the jittered
//! quarter's two substeps with it), the jittered wakes' p99 printed beside as
//! `wake_p99_other_ms` and a run with none unjittered not held; window p99 the
//! cohort's lens counting inline time, faults nought on every variant, ops
//! nought on the stock variants (on every native-sealed variant since the fill
//! decision, the fill included), ops per window and per
//! substep within the pool, the load mind's ops p1 at least 4,300, the
//! computed CPU lens — the pool's ops per window over the host's measured
//! rate on Standard Doctrine by five quarters, `[tacsim.minds].cpu_lens_ms`
//! printed beside) and `sound=` over the lane's integrity (never a decision
//! input); one panic at the end under `TACSIM_ASSERT`. The fill variant's
//! verdict over three idle-host weekly runs was the decision
//! (`tacsim::match_::PRODUCTION_FILL_ENGINE`), taken on 2026-10-09 from the
//! owner's three ALIEN18 runs, all `held=no sound=yes`: the fill is native.
//! Since then the variant fields the production fill as it is — its Class I
//! hulls on the native stock mind, no Rhai, ops nought — and its `held=`
//! word is the owner's capacity reading of that fill, no longer a decision
//! input. A
//! `stock_ci` `held=no` on ALIEN18 (the owner's host) was to be the owner's
//! STOP before 17b cut M4.5 — a precondition the owner WAIVED on 2026-10-08
//! (the lenses were report-only on this file's Linux bless host then, a
//! cloud container; the VPS asserts them since 16c T-109). Its JSON,
//! `<stem>-minds-<variant>.<ext>`, carries a deterministic `minds` block the
//! delta compares once the baseline carries one — folded under the
//! baseline's `minds` beside the arena report's own `minds.states_at_end`
//! by 17b cut M4.5's one bless, which refuses to bless without all four
//! fresh. `m4_2_the_minds_lane_runs_small`, un-ignored, runs the ceiling and
//! the fill variants on sixteen hulls for five arena seconds with the
//! deterministic checks asserted.
//!
//! **Every lane at the ROM since 17b cut M4.5** (the M4 row's (iv)). The
//! arena lanes, the long run, the cost run and the lance lane field unit
//! minds at the ROM ([`rom_terms`]: `Controller::Mind`, Class 0, native) where
//! they fielded the reflex controller; the warm-up steps
//! `Arena::step_settled`, and every measured wake's thinks are run and
//! written inline right after the step, outside `Wake.ms` and before the
//! snapshot ([`run_wakes`]). The report's `reflex` object is `motor` (the
//! `ReflexConfig` facts and nine counters a block, `ticks` and `traces`
//! gone), the state histogram is `minds.states_at_end` off
//! `Arena::machine_state` (seven fixed leaves, `unknown` a literal nought),
//! "the field decided something" reads the minds' thinks and landings, and
//! the minds' own counters print on a report-only `harness_arena_minds`
//! line. The span of control reads the minds' live orders joined to the
//! tape ([`live_orders_by_issuer`]). The lance lane's W8 fact pins are
//! re-pinned from its fight at the ROM ([`measure_lance`]) — a fight since 17b
//! cut M4.5c's row cut, which keeps every contact not known friendly before a
//! friend, so a ROM hull's eight rows hold the enemy though the eleven others
//! of its side stand nearer (before it they were all its own side and nothing
//! fired: STOP L, resolved by the owner's rulings of 2026-10-09). The
//! baseline is blessed once on a Linux host (`arena.linux.json`; the owner's
//! ruling) by the non-asserting path, procedure β (`docs/ops.md`'s
//! *Blessing*): the lanes without `TACSIM_ASSERT`, then the delta with
//! `TACSIM_BASELINE_BLESS=1` on the same build, under a `HOSTNAME`
//! (`ccr-m4.5-linux-bless`) no later run carries — every cloud container reads
//! `/etc/hostname`'s `vm`, and a blessed host a later run matches makes the
//! timing gate live there — so the gate was dormant everywhere. Since 16c
//! `T-109` the file is blessed on the production VPS under its own host name
//! (`srv1930616`; *The lenses follow the production host*, above), so the
//! gate is live there and dormant on every other host, ALIEN18 and the cloud
//! containers included; the fill decision's `minds.fill_authored` block (the
//! fill native, ops and fees nought) was carried onto that bless when
//! `perf/vps-300` was merged, with no re-bless.
//!
//! What it must not do (16b §9's `H` row): touch `mush-sim` or `mush-atlas` — a
//! harness that needs a new API in the thing it measures is measuring its own
//! change — assert by default, or run on push. The ban on asserting by default
//! covers the thresholds — timings, budgets, bounds — and nothing else: the sight
//! differential (16b cut J.1) is a correctness lane that asserts always, because
//! a wrong sight is a bug on any runner; and `sight_only` is the arena's trace,
//! built for the arena (16c N5), which the lane only checks, not an API the
//! harness asked for.

// 17b cut M4.2: the minds lane's report is one `serde_json::json!` of some two
// hundred leaves, past the default recursion limit of the macro's expansion.
#![recursion_limit = "256"]

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
    /// **16b cut AI.1** (16c `B-84`): A* cells one wake's path searches may
    /// expand, found or failed — asserted beside the queries bound, and a
    /// `thresholds` key.
    max_path_cells_per_pulse: u64,
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
    /// **16b cut AF.2** (16c N16): the largest snapshot state the 30-second
    /// lane may encode, bytes — asserted on `harness_arena_snapshot`'s
    /// `bytes_max`, and deliberately **not** a `thresholds` key (see
    /// [`SnapshotFigures`]).
    max_snapshot_bytes: u64,
    /// **`TACSIM_SNAPSHOT_OFF=1`** (16b cut AF.2): **measurement only.** The
    /// 30-second lane takes no snapshot, so the off-and-on timing measurement
    /// can attribute a lens move to the encode or to noise (16b cut AF.2,
    /// `Q-AF5`). The weekly script never sets it, and a run under
    /// `TACSIM_ASSERT` refuses it: the snapshot bound is never asserted over no
    /// snapshots.
    snapshot_off: bool,
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
            max_path_cells_per_pulse: cfg.max_path_cells_per_pulse,
            max_share_ops_per_pulse: cfg.max_share_ops_per_pulse,
            max_ledger_bytes: cfg.max_ledger_bytes,
            max_cell_probes_per_pulse: cfg.max_cell_probes_per_pulse,
            max_jam_checks_per_pulse: cfg.max_jam_checks_per_pulse,
            max_rss_growth_kib: cfg.max_rss_growth_kib,
            max_frame_bytes: cfg.max_frame_bytes,
            max_output_events_per_pulse: cfg.max_output_events_per_pulse,
            max_snapshot_bytes: cfg.max_snapshot_bytes,
            snapshot_off: std::env::var("TACSIM_SNAPSHOT_OFF").is_ok_and(|v| v.trim() == "1"),
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
    mods.push(comp::tacsim_module_with(tacsim_cfg, boot_packs));
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
/// kinematics and in slope limit (16b cut U.1), and not in class costs. **16b
/// cut AE.1** built the per-row prices (16c `B-58`) and kept this so on
/// purpose: the window goes through `ArenaTerrain::new`, whose rule is that a
/// fixture's authored number is its word about the cell for every unit, so every
/// row pays `harness_move_cost` here. The cohort measures the arena and not the
/// shipped table (Q-AE1); the lanes that build the shipped ground through
/// `from_pack` — the sight differential and the lance lane — price per row.
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
    /// — the field the 12 ms lens and the per-pulse bounds were sized on. Both
    /// cohorts assert under `TACSIM_ASSERT` since 16b cut J.3b; this one per
    /// window at [`ARENA_S_P99_MS`].
    Ci,
    /// **The authored reaches**, exactly as the catalog fields them: asserted
    /// under `TACSIM_ASSERT` since 16b cut J.3b (16c Q26), against its own
    /// per-window lens ([`ARENA_AUTHORED_WINDOW_P99_MS`]) and the per-wake lens
    /// and per-pulse bounds the CI cohort is held to.
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
/// reach, `harness.max_share_ops_per_pulse` was 256 when this was set, and at
/// this density the authored 30 is about 2,400 operations and 2,400 further
/// traces a second. The bound is 768 since 16b cut J.3b (16c Q26), on the
/// authored lane's measured per-wake peak of 452 at the 30-cell mesh; this
/// cohort's reduced mesh is unchanged, and its peak is a small fraction of it.
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
        modes: Vec::new(),
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
/// modes, slope limits, silhouettes, facet tables and weapons as authored —
/// gunnery 1, and **since 17b cut M4.5 every one a unit mind at the ROM**
/// ([`rom_slot`]: `Controller::Mind` sealed `class_cap: Zero` on the native
/// engine, so every think is the native stock mind at Class 0's row — what
/// the reflex controller was before it). The harness-local tank, sight,
/// weapon and profile this replaced are gone: a harness whose units had no
/// locomotion ran the slope limit, the shoulder rule and the per-mode A\*
/// cost in none of 300. Under [`Reach::Ci`] each sight is then scaled
/// ([`scale_to_ci`]).
///
/// Doc 16 §1 A2 is *300 units placed and moving, 24 of them fighting*, and with
/// the minds driving the field that has to be arranged rather than
/// hoped for: the fighting units stand in two lines [`LINE_GAP_CELLS`] either
/// side of the centre, even units (red) west and odd (blue) east,
/// [`LINE_PITCH_CELLS`] apart down each line from row `middle − 18`, each wanted
/// cell answered with the nearest passable one. The pitch is not cosmetic: the
/// share mesh is one operation per `(sharer, teammate)` pair, so twelve
/// teammates in one blob is 132 operations a pass on its own. The seats are
/// **frame sinks**, not pilots: doc 16 §1 A2 asks for connected sessions
/// receiving output and the frame builder is a real cost, but no pilot issues
/// an intent and none possesses its hull: a seat on a machine's hull is a
/// frame sink, and the machine flies it (a released seat in production hands
/// its hull to a mind, 17b cut M4.3; here the hull was never anyone else's).
fn field(
    knobs: &Knobs,
    catalog: &Catalog,
    relief: &[(u16, u16, i16)],
    reach: Reach,
) -> (
    Vec<tacsim::arena::state::CustodyUnit>,
    Vec<tacsim::arena::state::Pilot>,
) {
    field_with(knobs, catalog, relief, reach, rom_slot)
}

/// **The ROM's seal terms** (17b cut M4.5; lifted from 17b cut M4.2's minds
/// lane): the fixture terms capped at Class 0 on the native engine, so
/// `seal_for` writes class `Zero`, `Arena::mind_class` answers `Zero` and
/// every think is handed to the ROM — the native stock mind at Class 0's row
/// (8 contact rows, the lock's odds alone, one slot, 16 keys, a think every
/// 1,000 ms; `ClassTable::STOCK`). No installer: the arena-wide fallback
/// runner is the native stock mind. Every hull of every harness lane but the
/// minds lane's authored slots flies on these terms.
fn rom_terms() -> tacsim::match_::SealTerms {
    tacsim::match_::SealTerms {
        class_cap: tacsim::arena::core::CoreClass::Zero,
        engine_for_minds: tacsim::arena::SealEngine::Native,
        ..tacsim::match_::SealTerms::fixture()
    }
}

/// **Unit number `n`'s slot at the ROM** (17b cut M4.5): its [`COHORT`] entry
/// (`COHORT[(n / 2) % 8]`), `Controller::Mind`, [`rom_terms`], Standard
/// Doctrine's own seal. [`field`]'s every slot and [`field_minds`]' every
/// slot that is not authored.
fn rom_slot(n: u32) -> FieldSlot {
    FieldSlot {
        entry: COHORT
            .get(usize::try_from(n / 2).unwrap_or(0) % COHORT.len())
            .copied()
            .unwrap_or(content::LINE_TANK),
        controller: tacsim::arena::state::Controller::Mind,
        terms: rom_terms(),
        sealed_source: None,
    }
}

/// **The ROM's think period**, milliseconds: Class 0's row of the stock class
/// table (1,000 — the reflex's `decide_ms` it replaced in the warm-up
/// arithmetic, so the warm-up still ends at `startup_ms + 1,000 + engaged_ms`,
/// 11,500 ms). `Arena::new` lays down [`tacsim::arena::core::ClassTable::STOCK`],
/// so this is the arena's own row; read here where a match's length is set
/// before the arena exists. **The one source** the ROM lanes' warm-up
/// arithmetic reads — [`open_field`]'s and [`measure_lance`]'s (17b cut
/// M4.5's review: the lance lane read the arena's row beside it).
fn rom_think_ms() -> i64 {
    tacsim::arena::core::ClassTable::STOCK
        .row(tacsim::arena::core::CoreClass::Zero)
        .think_ms
}

/// **What one slot of a field is** (17b cut M4.2 factored it out of
/// [`field`]): the entry, the controller, the seal terms and — for a hull
/// whose seal names another mind than Standard Doctrine — the mind's name and
/// its source digest, written onto the seal after custody (`seal_for` writes
/// Standard's digest on every seal and the terms carry no source).
struct FieldSlot {
    entry: &'static str,
    controller: tacsim::arena::state::Controller,
    terms: tacsim::match_::SealTerms,
    sealed_source: Option<(&'static str, [u8; 32])>,
}

/// [`field`]'s body with each unit's slot answered by `slot_of(n)` — the
/// arrangement, the seed, the ids, the teams and the seats are the same
/// whatever the slot says, so the minds lane's field ([`field_minds`]) is the
/// arena lane's with the authored slots' terms swapped in.
fn field_with(
    knobs: &Knobs,
    catalog: &Catalog,
    relief: &[(u16, u16, i16)],
    reach: Reach,
    slot_of: impl Fn(u32) -> FieldSlot,
) -> (
    Vec<tacsim::arena::state::CustodyUnit>,
    Vec<tacsim::arena::state::Pilot>,
) {
    use tacsim::arena::state::{Pilot, Team};
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
        let slot = slot_of(n);
        let key = CanonicalId::new(slot.entry).expect("a canonical entry id");
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
        let mut unit = tacsim::match_::custody_unit(
            catalog,
            cells,
            id,
            team,
            pose,
            1,
            slot.controller,
            0,
            &slot.terms,
        )
        .expect("the slot's terms seal");
        // 17b cut M4.2: a seal on another mind than Standard Doctrine — the
        // seal's own digest is computed on call, so the `mind_sealed` line
        // and the installer's lookup read this with no re-digest.
        if let Some((mind, source_sha256)) = slot.sealed_source {
            unit.seal.mind = smol_str::SmolStr::new(mind);
            unit.seal.source_sha256 = source_sha256;
        }
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
/// LLM seam — and the squad it names, by its word (`red-1`). **Since 17b cut
/// M4.5** also the unit it landed on and its time to live, which the span
/// joins a mind's live order to ([`live_orders_by_issuer`]); on an
/// `order_released` line the unit released and a time to live of nought.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OrderedLine {
    t_ms: i64,
    issuer: Option<ObjId>,
    squad: String,
    unit: ObjId,
    ttl_ms: u32,
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
            tacsim::arena::ledger::EventKind::Ordered {
                issuer,
                squad,
                unit,
                ttl_ms,
                ..
            } => Some(OrderedLine {
                t_ms: event.t_ms,
                issuer,
                squad: squad.to_string(),
                unit,
                ttl_ms,
            }),
            _ => None,
        })
        .collect()
}

/// **Every `order_released` line in a chunk** (16b cut Z.2's review), parsed
/// whole as [`ordered_lines_of`] parses the `ordered` ones and for the same
/// reason: a line that says `order_released` and does not parse fails the run
/// rather than dropping out of the lance lane's per-side check.
fn released_lines_of(bytes: &[u8]) -> Vec<OrderedLine> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| line.contains("\"kind\":\"order_released\""))
        .map(|line| {
            serde_json::from_str::<tacsim::arena::ledger::LedgerEvent>(line).unwrap_or_else(|e| {
                panic!("Z.2: an `order_released` line the harness cannot read ({e}): {line}")
            })
        })
        .filter_map(|event| match event.kind {
            tacsim::arena::ledger::EventKind::OrderReleased {
                issuer,
                squad,
                unit,
            } => Some(OrderedLine {
                t_ms: event.t_ms,
                issuer,
                squad: squad.to_string(),
                unit,
                ttl_ms: 0,
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
    /// The most squads under one issuer's live orders at the run's end, an
    /// arena-time figure — since 17b cut M4.5 off the minds' live orders
    /// joined to the tape ([`live_orders_by_issuer`]).
    span_max: usize,
}

/// **Whether [`live_orders_by_issuer`] reads a wreck's live order** (17b cut
/// M4.5's review).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wrecks {
    /// A wreck's mind row keeps the order it died under, and it is counted
    /// until it expires: W8's span, as the reflex reader counted a wreck's row.
    Counted,
    /// Wrecks are passed over: the squads standing hulls hold live orders on.
    Skipped,
}

/// **The squads under each issuer's live orders, off the minds** (17b cut
/// M4.5; the orchestrator's ruling on STOP C): every live order a mind holds
/// — `MindState.order` on a hull the mind flies, unexpired at the read
/// (`expires_ms > now_ms`; wrecks INCLUDED under [`Wrecks::Counted`], as
/// `Arena::live_orders_by_issuer` counted a wreck's reflex row: a wreck's mind
/// row keeps the order it died under, and a wreck never lands, so its order
/// never expires through `Arena::expire_mind_order` — hence the explicit
/// deadline) — grouped by the
/// issuer on that unit's LAST `ordered` line on the tape (orders are
/// latest-wins), `None` filed under the commander of the squad the unit is
/// in (the reflex reader's fallback, and `Arena::rekey_order`'s scope).
///
/// **The issuer is read off the black box**, and the join is ASSERTED: every
/// live order has its line, naming the order's squad, and the line's
/// millisecond plus its time to live is the order's deadline — so a span can
/// never be read off a line that is not the order's. It was not mind-row
/// state when this join was written (17b cut M4.5: `FrameOrder` carries none);
/// **since 17b cut M4.6** (the owner's STOP C ruling) it is,
/// `MindState::order_issuer`, and `Arena::live_orders_by_issuer` groups the
/// mind rows by it exactly this way — which [`CommandFigures::of`] asserts on
/// every lane that reads W8's figures, moving no figure. This join stays the
/// tape's check (W8), and the lance lane's span pins are read off it.
///
/// `wrecks` [`Wrecks::Counted`] is W8's span; [`Wrecks::Skipped`] is the same
/// join over the live orders standing hulls hold, which the lance lane's
/// at-defaults pin reads to show its span rests on no wreck.
fn live_orders_by_issuer(
    arena: &tacsim::arena::state::Arena,
    tape: &[OrderedLine],
    wrecks: Wrecks,
) -> BTreeMap<Option<ObjId>, std::collections::BTreeSet<String>> {
    // The tape is in ledger order, so the last line a unit has is its latest.
    let mut last: BTreeMap<ObjId, &OrderedLine> = BTreeMap::new();
    for line in tape {
        last.insert(line.unit, line);
    }
    let mut grouped: BTreeMap<Option<ObjId>, std::collections::BTreeSet<String>> = BTreeMap::new();
    for (unit, row) in &arena.minds {
        if arena.unit(*unit).is_none_or(|state| {
            state.controller != tacsim::arena::state::Controller::Mind
                || (wrecks == Wrecks::Skipped && state.limits.wreck)
        }) {
            continue;
        }
        let Some(order) = row.order.as_ref() else {
            continue;
        };
        if order.expires_ms <= arena.now_ms {
            continue;
        }
        let line = last.get(unit).unwrap_or_else(|| {
            panic!(
                "W8: #{} holds a live order for {} (until {} ms) and no `ordered` line on the \
                 tape names it",
                unit.0, order.squad, order.expires_ms
            )
        });
        assert!(
            line.squad == order.squad.as_str()
                && line.t_ms.saturating_add(i64::from(line.ttl_ms)) == order.expires_ms,
            "W8: #{}'s live order ({} until {} ms) is not its last `ordered` line's: {line:?}",
            unit.0,
            order.squad,
            order.expires_ms
        );
        let issuer = line
            .issuer
            .or_else(|| arena.squad_of(*unit).map(|roster| roster.commander));
        grouped
            .entry(issuer)
            .or_default()
            .insert(order.squad.to_string());
    }
    grouped
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
        let joined = live_orders_by_issuer(arena, tape, Wrecks::Counted);
        // **17b cut M4.6** (the conventions' D4; the owner's STOP C ruling):
        // the arena's own reader, over the issuer each mind row stored where
        // its order landed, groups exactly as this tape join — asserted here,
        // where W8's figures are read, and moving no figure (the span below is
        // still the join's).
        let stored: BTreeMap<Option<ObjId>, std::collections::BTreeSet<String>> = arena
            .live_orders_by_issuer(arena.now_ms)
            .into_iter()
            .map(|(issuer, squads)| (issuer, squads.iter().map(ToString::to_string).collect()))
            .collect();
        assert_eq!(
            stored, joined,
            "W8: `Arena::live_orders_by_issuer` (the mind rows' stored issuers) and the tape's \
             join (each live order's last `ordered` line) group the live orders differently"
        );
        let span_max = joined
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

    /// The `<prefix>` line — `harness_arena_command` on the two arena lanes,
    /// `harness_lance_command` on the lance lane (16b cut Z.1 gave it the prefix
    /// as an argument; the arena lanes' line is byte-identical).
    /// `orders_per_commander_minute_max` is counted in arena time and `span_max`
    /// read in arena time; the world enforces both in world time.
    fn line(&self, prefix: &str, cohort: &str) -> String {
        format!(
            "{prefix}: cohort={cohort} max_span={} orders_per_minute={} squads={} \
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

    /// **W8, asserted** (both cohorts under `TACSIM_ASSERT` since 16b cut J.3b;
    /// the CI cohort alone before it).
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
                .enumerate()
                .map(|(at, (t_ms, issuer, squad))| OrderedLine {
                    t_ms: *t_ms,
                    issuer: issuer.map(ObjId),
                    squad: (*squad).to_string(),
                    unit: ObjId(100 + u32::try_from(at).unwrap_or(0)),
                    ttl_ms: 600_000,
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
    /// **16b cut Z.2**: every `order_released` line, one per unit a release
    /// dropped to doctrine — counted as the `ordered` lines are.
    released: u64,
    /// **16b cut Z.2's review**: those lines, parsed — who released which
    /// squad, and when — so the lance lane can hold each side's release to its
    /// own commander and its own first squad rather than to a count one side
    /// could fill alone.
    released_lines: Vec<OrderedLine>,
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
        // 16b cut Z.2: the releases, off the same chunk.
        self.released += occurrences(bytes, "\"kind\":\"order_released\"");
        self.released_lines.extend(released_lines_of(bytes));
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
/// `<stem>-arena-authored.<ext>`. Since 16b cut J.3b (16c Q26, the owner's
/// answer) it asserts under `TACSIM_ASSERT` like the CI lane: per wake at the
/// 20 ms lens ([`ARENA_S_P99_MS`]; 12 before 16c `T-109`), per 500 ms window at
/// 32 ms ([`ARENA_AUTHORED_WINDOW_P99_MS`]; 17.5 at J.3b, 20 from J.3c to
/// `T-109`), and the same per-pulse bounds of
/// `[tacsim.harness]`. The delta does not read its report.
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

/// **What [`run_wakes`] steps** (16b cut Z.1 factored it out of [`Field`]): the
/// arena, the lattice it is woken on, the streamed ledger's count and the wake
/// clock. [`Field`] holds one for the relief lanes; the lance lane
/// ([`measure_lance`]) has no relief and builds one of its own, so the two
/// kinds of field go through one loop and one warm-up ([`warm_up`]).
struct Stepper {
    arena: tacsim::arena::state::Arena,
    /// The cohort's substep, the lattice `next_due_ms` answers on.
    substep: u32,
    /// The streamed ledger, counted as it goes — the warm-up's included.
    counts: Counts,
    clock: WakeClock,
    /// The wall the next wake is at.
    wall: i64,
    /// The slope refusals over the whole run, the warm-up included.
    slope_refusals: u64,
    /// **`[tacsim.arena].snapshot_ms`, when this lane takes snapshots** (16b
    /// cut AF.2), or `None` — the default, and what [`open_field`] and the lance
    /// lane build. **Only [`measure_arena`] sets it**, so the 30-second lane
    /// alone pays and measures the encode: the long run's RSS bound and the
    /// lance lane's timings are untouched in fact and not only in words.
    snapshot_ms: Option<u32>,
    /// The period of the last snapshot [`run_wakes`] took, as the task keeps it
    /// (`tacsim::arena_task::snapshot_due`).
    last_snapshot_period: Option<i64>,
}

/// **An arena fielded, opened and warmed up** — everything the measurement lanes
/// share before their measured window (16b cut U.2 factored it out of
/// [`measure_arena`], so the 30-second lane and the long run have one body and
/// differ only in their window and their sampling).
struct Field {
    cohort: &'static str,
    /// The arena and what steps it (16b cut Z.1: [`Stepper`]).
    stepper: Stepper,
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
    slope_45: usize,
    slope_70: usize,
    /// Every unit's magazines as checked out.
    ammo_at_start: BTreeMap<ObjId, u64>,
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
/// every one a unit mind at the ROM (17b cut M4.5; the reflex controller
/// before it), the tempo pinned `Engaged`.
///
/// The warm-up is the minds' own: every unit starts cold and its mind's first
/// think brings it online — a ROM hull's first think falls at its stagger
/// inside the first 1,000 ms and lands a substep later, and the startup
/// completes `startup_ms` after the landing — so the warm-up is over when
/// arena time has passed `startup_ms` plus the ROM's think period
/// ([`rom_think_ms`], the reflex's `decide_ms` before it, the same 1,000) and
/// a period, and it is asserted — a field of minds that started nothing would
/// otherwise be measured driving an arena of cold tanks.
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
    let mut match_cfg = MatchConfig::default();
    // **16b cut AP.2** (16c Q47): the shipped cap is fifteen minutes, so a
    // `TACSIM_LONG_RUN_S` past it would meet `max_duration_ms` inside the long
    // run's window. The lane's own match covers the warm-up, the window and a
    // minute more (never less than the default), so it measures what it is
    // asked; the cap folds into no hash, so the deterministic block is unmoved.
    match_cfg.max_duration_ms = match_cfg.max_duration_ms.max(
        u32::try_from(
            i64::from(match_cfg.startup_ms)
                .saturating_add(rom_think_ms())
                .saturating_add(i64::from(arena_cfg.tempo.engaged_ms))
                .saturating_add(i64::from(knobs.long_run_s).saturating_mul(1_000))
                .saturating_add(60_000),
        )
        .unwrap_or(u32::MAX),
    );
    let sensors_cfg = tacsim::config::SensorsConfig::default();
    let open_at = Instant::now();
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots,
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: knobs.seed,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &arena_cfg,
        &match_cfg,
        &sensors_cfg,
    )
    .expect("the harness arena opens");
    // Host policy only: use 1 for a serial A/B reference on the identical seed.
    let frame_workers = env_u32("TACSIM_FRAME_WORKERS", 1);
    arena.set_frame_workers(frame_workers as usize);
    eprintln!("pilot frame workers: {}", frame_workers.clamp(1, 4));
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

    let mut stepper = Stepper {
        arena,
        substep,
        counts: Counts::default(),
        clock: WakeClock::new(knobs),
        wall: 0,
        // The slope refusals over the whole run, the warm-up included: the
        // minds' goals plan their first routes there.
        slope_refusals: 0,
        // 16b cut AF.2: no snapshots unless the lane opts in (`measure_arena`).
        snapshot_ms: None,
        last_snapshot_period: None,
    };

    // ---- the warm-up: the minds start them up ----
    let warm_at = Instant::now();
    let warm_until = i64::from(match_cfg.startup_ms)
        .saturating_add(rom_think_ms())
        .saturating_add(period_ms);
    let warm_pulses = warm_up(&mut stepper, warm_until);
    let warm_ms = warm_at.elapsed().as_secs_f64() * 1_000.0;
    let arena = &stepper.arena;
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
        "{} units never came online: the ROM did not start the field up: {cold:?}",
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
        stepper,
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
        slope_45,
        slope_70,
        ammo_at_start,
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

/// **16b cut AF.2 (16c N16): the snapshot rule is the 30-second lane's opt-in.**
/// A [`Stepper`] as [`open_field`] builds it — the one every relief lane starts
/// from, the 600-second long run's included — carries no cadence, so
/// [`run_wakes`] takes no snapshot for it: the long run's RSS bound (about
/// 120 kept documents of a 300-unit field otherwise) and every other lane's
/// timings are untouched in fact. Only [`measure_arena`] sets the cadence. The
/// lance lane builds its own [`Stepper`] with `None` beside `slope_refusals`.
/// Runs on every `cargo test`, on a sixteen-unit field — since 17b cut M4.5
/// sixteen ROM minds through [`field`], warmed up settled and asserted
/// online by [`open_field`].
#[test]
fn af2_a_stepper_open_field_builds_takes_no_snapshot() {
    let mut knobs = Knobs::from_env();
    knobs.units = 16;
    knobs.fighting = 4;
    knobs.seed = DEFAULT_SEED;
    knobs.jitter_ms = 0;
    let field = open_field(&knobs, Reach::Ci);
    assert_eq!(field.stepper.snapshot_ms, None);
    assert_eq!(field.stepper.last_snapshot_period, None);
}

/// **The warm-up** (16b cut U.1; one body since 16b cut Z.1): the opening pulse
/// at wall nought, then a wake at every `next_due_ms` until arena time reaches
/// `warm_until`, every chunk counted and acknowledged as [`run_wakes`] does.
/// Answers how many pulses it took.
///
/// **Settled since 17b cut M4.5** (`Arena::step_settled`): every pulse's
/// thinks are run and written before the next, so the warm-up ends with no
/// think outstanding and the first measured wake settles none of the
/// warm-up's inside its own `Wake.ms` (a bare `Arena::step` leaves the last
/// substep's thinks to the next step's head). The warm-up's timing is
/// discarded, so where the inline thinks fall costs nothing here.
fn warm_up(stepper: &mut Stepper, warm_until: i64) -> u32 {
    let arena = &mut stepper.arena;
    let out = arena.step_settled(0, &[], &[]);
    for chunk in &out.ledger_chunks {
        stepper.counts.absorb(&chunk.bytes);
    }
    stepper.wall = stepper.clock.next(out.next_due_ms);
    let mut warm_pulses = 1_u32;
    while arena.now_ms < warm_until {
        let out = arena.step_settled(stepper.wall, &[], &[]);
        stepper.slope_refusals += u64::from(out.motor.slope_refusals);
        for chunk in &out.ledger_chunks {
            stepper.counts.absorb(&chunk.bytes);
        }
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        stepper.wall = stepper.clock.next(out.next_due_ms);
        warm_pulses += 1;
    }
    warm_pulses
}

/// **What one measured window of wakes produced** (16b cut U.2): the per-wake
/// record, the substep partition, the perception and motor counters as a
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
    /// The machines' counters as a total (17b cut M4.5: `MotorCounters`, the
    /// reflex's counters renamed, without `ticks`, `in_state` and `ordered`)
    /// — the step's own and the delivered thinks' faults, as
    /// `Arena::step_settled` folds them.
    motor_total: tacsim::arena::motor::MotorCounters,
    /// The worst wake of each motor counter the report carries.
    motor_peak: tacsim::arena::motor::MotorCounters,
    /// **Each wake's inline delivery, wall milliseconds** (17b cut M4.5): the
    /// step's thinks run and written after the step and outside `Wake.ms`
    /// ([`run_wakes`]). Report-only (`harness_arena_minds`).
    settle_ms: Vec<f64>,
    /// **One sample per snapshot taken** (16b cut AF.2), empty unless the
    /// stepper opted in ([`Stepper::snapshot_ms`]).
    snapshots: Vec<SnapshotSample>,
    /// The documents those snapshots encoded, in order — decoded by
    /// [`decode_snapshots`] after the timed loop, never inside it.
    documents: Vec<serde_json::Value>,
}

/// **One snapshot the 30-second lane took** (16b cut AF.2): the arena
/// millisecond it was taken at, the state's bytes ([`tacsim::arena::Snapshot::bytes`],
/// what the bound is asserted on), the encode's wall time, and the wall time of
/// the wake it followed — the pair production pays back to back on the task's
/// thread.
struct SnapshotSample {
    now_ms: i64,
    bytes: usize,
    encode_ms: f64,
    wake_ms: f64,
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
/// one substep. On the relief lanes there is no scripted order and no scripted
/// fire: what steers, closes, locks and fires is the units' minds alone (the
/// ROM, 17b cut M4.5; the reflex controller before it), and `script` answers
/// nothing.
///
/// **The minds' thinks, settled outside the timer** (17b cut M4.5; the
/// orchestrator's Q9). `Arena::step` leaves its last substep's thinks
/// collected and unanswered (`StepOutput::thinks`), and `Snapshot::encode`
/// refuses an arena that holds one. So after each step — timed ALONE into
/// `Wake.ms`, the minds lane's own definition of a wake — the thinks are run
/// inline and written (`run_batch_inline`, `Arena::deliver_answers`, exactly
/// `Arena::step_settled`'s body, the faults folded into the wake's counters
/// as it folds them), outside the timer and before the snapshot, the sample
/// and the script's next read; their wall time is kept per wake
/// ([`Run::settle_ms`], report-only). The deterministic block is
/// `step_settled`'s (17b cut M4.1's gate: inline ≡ pooled), and the lens is
/// not inflated by think time production spends on its pool's threads.
///
/// **The script** (16b cut Z.1, the lance lane's commanders): before every wake
/// `script` is handed the arena at the arena millisecond that wake's phase 2
/// runs at (`now_ms`, where the previous pulse ended), and whatever intents it
/// answers are that wake's `intents` — applied inside `Arena::step`'s own phase
/// 2, through `apply_intent` as a seat's, so each `ordered` line carries the
/// phase's one hash exactly as a pilot's `ORDER` does.
///
/// Whatever the ledger is still holding leaves with the measurement, counted.
fn run_wakes(
    stepper: &mut Stepper,
    until_ms: i64,
    mut script: impl FnMut(&tacsim::arena::state::Arena) -> Vec<tacsim::arena::intents::Intent>,
    mut sample: impl FnMut(&tacsim::arena::state::Arena),
) -> Run {
    use tacsim::arena::perception::PerceptionCounters;
    let substep_ms = i64::from(stepper.substep);
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
        // 16b cut N.1: the machines' cost, as a running total and as the worst
        // wake. `harness.max_path_queries_per_pulse` is asserted against the peak.
        motor_total: tacsim::arena::motor::MotorCounters::default(),
        motor_peak: tacsim::arena::motor::MotorCounters::default(),
        settle_ms: Vec::new(),
        snapshots: Vec::new(),
        documents: Vec::new(),
    };
    let arena = &mut stepper.arena;
    while arena.now_ms < until_ms {
        let before = arena.now_ms;
        let intents = script(arena);
        let at = Instant::now();
        let mut out = arena.step(stepper.wall, &[], &intents);
        let ms = at.elapsed().as_secs_f64() * 1_000.0;
        // 17b cut M4.5: the step's thinks, run inline and written outside the
        // timer — `Arena::step_settled`'s body, so nothing is left unanswered
        // for the snapshot, the sample or the script.
        let settle_at = Instant::now();
        let thinks = std::mem::take(&mut out.thinks);
        let settled = arena.deliver_answers(tacsim::arena::mind::run_batch_inline(&thinks));
        out.motor.mind_faults = out.motor.mind_faults.saturating_add(settled.faults);
        out.motor.mind_overflows = out.motor.mind_overflows.saturating_add(settled.overflows);
        run.settle_ms
            .push(settle_at.elapsed().as_secs_f64() * 1_000.0);
        // A pulse that ended the match hands out no thinks (nothing is
        // written after the end), and the loop stops on it below.
        assert!(
            out.ended.is_some() || arena.awaiting.is_empty(),
            "every think the step collected is written before the next read: {} hulls \
             still awaiting",
            arena.awaiting.len()
        );
        run.wakes.push(Wake {
            ms,
            now_ms: arena.now_ms,
            traces: out.perception.traces,
            cell_probes: out.perception.cell_probes,
            horizon_probes: out.perception.horizon_probes,
            jam_checks: out.perception.jam_checks,
            share_ops: out.perception.share_ops,
        });
        // **16b cut AF.2**: the snapshot, where the task takes it — after the
        // pulse, on a pulse that did not end the match, by the task's own period
        // rule — timed and run INLINE, between this wake and the next, because
        // production pays it there on the task's thread and its cache cost lands
        // on the next wake there too; a lane that took it elsewhere would measure
        // less than production pays. Never folded into `Wake.ms`: the lens
        // measures `step`. The document is kept; the decode runs after the loop
        // (`decode_snapshots`), so no harness-only work sits between two wakes.
        if let Some(snapshot_ms) = stepper.snapshot_ms
            && out.ended.is_none()
            && tacsim::arena_task::snapshot_due(
                &mut stepper.last_snapshot_period,
                arena.now_ms,
                snapshot_ms,
            )
        {
            let at = Instant::now();
            let encoded = tacsim::arena::Snapshot::encode(arena);
            let encode_ms = at.elapsed().as_secs_f64() * 1_000.0;
            let snapshot = encoded.expect("the arena encodes between two pulses");
            run.snapshots.push(SnapshotSample {
                now_ms: arena.now_ms,
                bytes: snapshot.bytes().len(),
                encode_ms,
                wake_ms: ms,
            });
            run.documents.push(snapshot.document);
        }
        // A substep shorter than the lattice's: the remainder a wake off the
        // lattice cuts, and a phase-4 drain's zero-length one.
        let full = u32::try_from((arena.now_ms - before) / substep_ms.max(1)).unwrap_or(0);
        run.short_substeps += u64::from(out.substeps.saturating_sub(full));
        stepper.slope_refusals += u64::from(out.motor.slope_refusals);
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
        run.motor_total.absorb(&out.motor);
        // The report's nine motor counters, each wake's worst (17b cut M4.5:
        // `ticks` retired with the reflex's tick, `traces` reported nowhere —
        // a mind takes none; the minds' own counters are the minds lane's).
        let motor_peak = &mut run.motor_peak;
        motor_peak.path_queries = motor_peak.path_queries.max(out.motor.path_queries);
        motor_peak.path_deferred = motor_peak.path_deferred.max(out.motor.path_deferred);
        motor_peak.path_cells = motor_peak.path_cells.max(out.motor.path_cells);
        motor_peak.path_failures = motor_peak.path_failures.max(out.motor.path_failures);
        motor_peak.intents = motor_peak.intents.max(out.motor.intents);
        motor_peak.shots = motor_peak.shots.max(out.motor.shots);
        motor_peak.slope_refusals = motor_peak.slope_refusals.max(out.motor.slope_refusals);
        // 16b cut AI.1 (16c `B-84`): the two new counters, each wake's worst.
        motor_peak.path_refused_unchanged = motor_peak
            .path_refused_unchanged
            .max(out.motor.path_refused_unchanged);
        motor_peak.corner_refusals = motor_peak.corner_refusals.max(out.motor.corner_refusals);
        run.substeps.push(out.substeps);
        #[allow(clippy::cast_precision_loss)]
        run.frames.push(out.frames.len() as f64);
        if out.lagged {
            run.lagged_pulses += 1;
        }
        for chunk in &out.ledger_chunks {
            stepper.counts.absorb(&chunk.bytes);
        }
        // The world half's acknowledgement (doc 16 §3.1's ack-gated egress), so the
        // bounded ring is measured draining as it would in production.
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        sample(arena);
        if let Some(end) = out.ended {
            run.ended = Some(end);
            break;
        }
        stepper.wall = stepper.clock.next(out.next_due_ms);
    }
    // Whatever is still open leaves with the measurement.
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        stepper.counts.absorb(&chunk.bytes);
    }
    run
}

/// **The snapshots' correctness pass** (16b cut AF.2), run AFTER the timed
/// loop over the documents [`run_wakes`] kept: each one decoded against the
/// field's own ground (cloned outside the timer), every check of
/// [`tacsim::arena::Snapshot::decode`] applied — the ground's digest, the
/// physics digest, the state digest, the pose refusals and the state hash —
/// and the decoded arena's hash compared again with the head's. Answers the
/// slowest decode, milliseconds, and whether every one passed; a failure is
/// printed as `hash_ok=false` and fails the run under `TACSIM_ASSERT`.
fn decode_snapshots(
    documents: &[serde_json::Value],
    definition: &tacsim::arena::state::ArenaDefinition,
) -> (f64, bool) {
    let mut decode_ms_max = 0.0_f64;
    let mut hash_ok = true;
    for document in documents {
        let ground = definition.clone();
        let at = Instant::now();
        let decoded = tacsim::arena::Snapshot::decode(document, ground);
        decode_ms_max = decode_ms_max.max(at.elapsed().as_secs_f64() * 1_000.0);
        let head = tacsim::arena::Snapshot::head(document);
        match (decoded, head) {
            (Ok(arena), Ok(head))
                if tacsim::arena::hash::state_hash(&arena) == head.state_hash
                    && arena.now_ms == head.at_ms => {}
            (decoded, head) => {
                println!(
                    "harness_arena_snapshot_refused: at_ms={:?} decode={:?}",
                    head.map(|head| head.at_ms),
                    decoded.err()
                );
                hash_ok = false;
            }
        }
    }
    (decode_ms_max, hash_ok)
}

/// **The snapshot figures of one 30-second run** (16b cut AF.2; 16c N16): what
/// `harness_arena_snapshot` prints and the report's `snapshot` object holds.
///
/// **Outside the compared block, on purpose.** [`deterministic_block`] flattens
/// every `thresholds.*` key but two into the block the delta compares against
/// the committed baseline, so `[tacsim.harness].max_snapshot_bytes` is NOT a
/// `thresholds` key and is NOT on `harness_arena_thresholds` — which breaks the
/// pattern of every other bound in this file, deliberately: the bound belongs
/// to the one lane that measures it, and it and every figure here live on the
/// snapshot line and in the `snapshot` object, which neither
/// [`DETERMINISTIC_OBJECTS`] nor [`DETERMINISTIC_LEAVES`] names (as 16b cut U.3
/// placed `command`). The byte counts are a function of the seeded run and the
/// times are not; neither is compared.
struct SnapshotFigures {
    /// `snapshot_ms`, or `None` when the lane took none (`TACSIM_SNAPSHOT_OFF`).
    every_ms: Option<u32>,
    count: usize,
    /// The arena millisecond each was taken at, in order.
    at_ms: Vec<i64>,
    bytes_max: usize,
    bytes_mean: usize,
    encode_p50: f64,
    encode_p99: f64,
    encode_max: f64,
    decode_ms_max: f64,
    wake_plus_encode_ms_max: f64,
    max_snapshot_bytes: u64,
    hash_ok: bool,
}

impl SnapshotFigures {
    fn of(
        every_ms: Option<u32>,
        samples: &[SnapshotSample],
        decode: (f64, bool),
        max_snapshot_bytes: u64,
    ) -> SnapshotFigures {
        let mut encode: Vec<f64> = samples.iter().map(|sample| sample.encode_ms).collect();
        encode.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let total: usize = samples.iter().map(|sample| sample.bytes).sum();
        SnapshotFigures {
            every_ms,
            count: samples.len(),
            at_ms: samples.iter().map(|sample| sample.now_ms).collect(),
            bytes_max: samples.iter().map(|sample| sample.bytes).max().unwrap_or(0),
            bytes_mean: total.checked_div(samples.len()).unwrap_or(0),
            encode_p50: quantile(&encode, 500),
            encode_p99: quantile(&encode, 990),
            encode_max: encode.last().copied().unwrap_or(0.0),
            decode_ms_max: decode.0,
            wake_plus_encode_ms_max: samples
                .iter()
                .map(|sample| sample.wake_ms + sample.encode_ms)
                .fold(0.0, f64::max),
            max_snapshot_bytes,
            hash_ok: decode.1,
        }
    }

    fn every_word(&self) -> String {
        self.every_ms
            .map_or_else(|| "off".to_string(), |ms| ms.to_string())
    }

    fn line(&self, cohort: &str) -> String {
        format!(
            "harness_arena_snapshot: cohort={cohort} every_ms={} count={} bytes_max={} bytes_mean={} encode_ms_p50={:.3} encode_ms_p99={:.3} encode_ms_max={:.3} decode_ms_max={:.3} wake_plus_encode_ms_max={:.3} max_snapshot_bytes={} hash_ok={}",
            self.every_word(),
            self.count,
            self.bytes_max,
            self.bytes_mean,
            self.encode_p50,
            self.encode_p99,
            self.encode_max,
            self.decode_ms_max,
            self.wake_plus_encode_ms_max,
            self.max_snapshot_bytes,
            self.hash_ok
        )
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "every_ms": self.every_ms,
            "count": self.count,
            "at_ms": self.at_ms,
            "bytes_max": self.bytes_max,
            "bytes_mean": self.bytes_mean,
            "encode_ms": {
                "p50": num(self.encode_p50),
                "p99": num(self.encode_p99),
                "max": num(self.encode_max),
            },
            "decode_ms_max": num(self.decode_ms_max),
            "wake_plus_encode_ms_max": num(self.wake_plus_encode_ms_max),
            // The bound, here and never under `thresholds` (this type's doc).
            "max_snapshot_bytes": self.max_snapshot_bytes,
            "hash_ok": self.hash_ok,
        })
    }
}

/// **A machine's doctrine state, by its word** (17b cut M4.5): the machine's
/// state enum (`arena::doctrine::MachineState`, the reflex's `ReflexState`
/// until 17b cut M4.6a; `Unknown` since M4.6c), which the stock mind's memory
/// and `MindState.doctrine_state` carry — one alias, so each move was one line
/// here.
type MachineState = tacsim::arena::doctrine::MachineState;

/// **The leaves [`machine_states`] carries** (17b cut M4.5): seven —
/// [`MachineState::ALL`]'s words, `unknown` the seventh: the word 17b cut
/// M4.6 appended to `ALL` (code 6) for a hull whose seal is not Standard
/// Doctrine's (six words and the literal before it). Fixed at seven across
/// that cut, so the blessed `minds.states_at_end` neither gained nor lost a
/// leaf there.
const MACHINE_STATE_LEAVES: usize = 7;

// `ALL` named every leaf but `unknown` until 17b cut M4.6, and names every
// leaf since: never more leaves than the report carries.
const _: () = assert!(MachineState::ALL.len() <= MACHINE_STATE_LEAVES);

/// **The machines' states at the read, as seven fixed leaves** (17b cut
/// M4.5; the plan's `minds.states_at_end` "off the doctrine state"): every
/// unit's [`tacsim::arena::state::Arena::machine_state`] — the mind row's
/// cached `doctrine_state` (`Startup` before its first landing), `Disabled`
/// for a wreck or a disabled hull — counted into the leaf its code names,
/// never a decode of a memory; a pilot's hull counts in none. The leaves are
/// named off [`MachineState::ALL`] in code order — all seven since 17b cut
/// M4.6, `unknown` last (before it the literal `"unknown"` named the seventh,
/// blessed at 17b cut M4.5 so that cut added no leaf). Always all seven, in
/// this order, zeros included. A hull counts in `unknown` when a mind flies
/// it on a seal that is not Standard Doctrine (`SealRef::doctrine`): every
/// hull of the arena and lance lanes is sealed on it, so the leaf reads 0
/// there.
///
/// **What 17b cut M4.6 changed here: nothing in this body.** The counts are
/// sized by [`MACHINE_STATE_LEAVES`] and named by `ALL`, not by six and a
/// literal: `ALL` carries `Unknown` (code 6, its word `unknown` — any other
/// word would have moved the blessed leaf), names the seventh leaf itself,
/// the literal is overwritten, and an `Unknown` hull counts in leaf 6.
///
/// The old `reflex.states_at_end` counted the reflex's ROWS (a wreck's row
/// in `Disabled` as the reflex moved it there); this counts hulls under a
/// machine, `disabled` being every wreck and every disabled hull.
fn machine_states(
    arena: &tacsim::arena::state::Arena,
) -> [(&'static str, u32); MACHINE_STATE_LEAVES] {
    let mut leaves = [("unknown", 0_u32); MACHINE_STATE_LEAVES];
    for (leaf, state) in leaves.iter_mut().zip(MachineState::ALL) {
        leaf.0 = state.as_str();
    }
    for unit in &arena.units {
        if let Some(leaf) = arena
            .machine_state(unit.id)
            .and_then(|state| leaves.get_mut(usize::from(state.code())))
        {
            leaf.1 += 1;
        }
    }
    leaves
}

/// [`machine_states`] as `word=count` pairs, space-separated, for a line.
fn machine_states_words(states: &[(&'static str, u32); MACHINE_STATE_LEAVES]) -> String {
    states
        .iter()
        .map(|(word, count)| format!("{word}={count}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`machine_states`] as the report's object (key order is the map's).
fn machine_states_json(states: &[(&'static str, u32); MACHINE_STATE_LEAVES]) -> serde_json::Value {
    serde_json::json!(
        states
            .iter()
            .map(|(word, count)| ((*word).to_string(), *count))
            .collect::<BTreeMap<String, u32>>()
    )
}

/// **The arena measurement** (16b cut A.2 through 16b cut U.2): [`open_field`],
/// then [`run_wakes`] for `TACSIM_ARENA_S` arena seconds after the warm-up, and
/// the report.
///
/// Every measured wake is also folded into [`WINDOW_MS`] windows of arena time
/// ([`windows`]) and the lens binds both — 20 ms per wake, and per window the
/// cohort's own ([`window_lens_ms`], 22 ms on the CI cohort and 32 ms on the
/// authored since 16c `T-109`; 12, 12 and 20 on ALIEN18 before it, 17.5 at
/// J.3b): a wake is at most one substep,
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
    // **16b cut AF.2**: this lane — and no other — takes the task's snapshots,
    // at the field's own `[tacsim.arena].snapshot_ms` (5,000: about seven over
    // the thirty seconds), unless `TACSIM_SNAPSHOT_OFF=1` asks for the
    // measurement's off-run.
    field.stepper.snapshot_ms = if knobs.snapshot_off {
        None
    } else {
        Some(field.arena_cfg.snapshot_ms)
    };
    let every_ms = field.stepper.snapshot_ms;
    let until_ms = field
        .warm_end
        .saturating_add(i64::from(knobs.arena_s) * 1_000);
    let mut run = run_wakes(&mut field.stepper, until_ms, |_| Vec::new(), |_| {});
    // 16b cut AF.2: the decodes, after the timed loop, on the field's own ground
    // — and the kept documents (about 4 MB each on this field) dropped BEFORE the
    // end-of-window memory is read: they are the harness's bookkeeping, where
    // production holds at most one snapshot a match, and only until the egress
    // takes it. Read with them, `rss_end_kib` (16c `T-36`'s reported figure)
    // rose by their bytes at AF.2 for nothing production pays.
    let documents = std::mem::take(&mut run.documents);
    let decoded = decode_snapshots(&documents, &field.stepper.arena.definition);
    drop(documents);
    let rss_end_kib = resident_kib();
    let vmrss_end_kib = vmrss_kib();

    let Field {
        cohort,
        stepper:
            Stepper {
                arena,
                substep,
                counts,
                slope_refusals,
                ..
            },
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
        slope_45,
        slope_70,
        ammo_at_start,
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
        motor_total,
        motor_peak,
        settle_ms,
        snapshots,
        documents: _,
    } = run;
    let snapshot = SnapshotFigures::of(every_ms, &snapshots, decoded, knobs.max_snapshot_bytes);
    // 16b cut AF.2: the periods of arena time the measured wakes stood in, every
    // one of which the task's rule snapshots — counted here apart from the rule
    // (a wake that ended the match takes none), so an asserting run that took
    // no snapshot, or too few, fails rather than asserting its bound over nothing.
    let snapshot_periods = every_ms.filter(|ms| *ms > 0).map_or(0, |ms| {
        let measured = if ended.is_some() {
            wakes.len().saturating_sub(1)
        } else {
            wakes.len()
        };
        wakes
            .iter()
            .take(measured)
            .map(|wake| wake.now_ms.div_euclid(i64::from(ms)))
            .collect::<std::collections::BTreeSet<i64>>()
            .len()
    });

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
    // reported and not asserted: every unit is a machine's (a mind's since
    // 17b cut M4.5), the planner refuses steep edges and `hold_routeless`
    // stops a routeless machine before motion reaches one, so such a stop
    // records a motor defect and a working motor may journal none.
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
    // field whose machines never decided, never searched and never fired would
    // make the pulse cheaper and pass every timing bound in this file. **17b
    // cut M4.5**: what decides is a mind, so "decided something" is that the
    // minds thought and their answers landed (the reflex's `ticks` retired
    // with it). `intents` still counts every intent a machine put through
    // `Arena::apply_machine_intents` — a mind's landing among them, and the
    // motor's own heading and speed — and `shots` a mind's cited `Fire` and a
    // slot's own; both are kept, and neither alone says a mind decided.
    assert!(
        motor_total.thinks > 0 && motor_total.landings > 0,
        "no mind ever thought and landed an answer: {motor_total:?}"
    );
    assert!(
        motor_total.intents > 0,
        "no machine ever put an intent through: {motor_total:?}"
    );
    assert!(
        motor_total.path_queries > 0,
        "no machine ever searched for a route: {motor_total:?}"
    );
    assert!(
        motor_total.shots > 0,
        "no machine ever pulled a trigger: {motor_total:?}"
    );
    let states = machine_states(&arena);
    let state_count = |word: &str| {
        states
            .iter()
            .find(|(named, _)| *named == word)
            .map_or(0, |(_, count)| *count)
    };
    let engaging = state_count(MachineState::Engage.as_str())
        .saturating_add(state_count(MachineState::Approach.as_str()));
    assert!(
        engaging > 0,
        "nobody was fighting at the end of the run: {states:?}"
    );

    // 17b cut M4.5: the motor's facts (`[tacsim.arena.reflex]`, which keeps
    // its name: `decide_ms` is the motor tick's period) and its counters —
    // the report's nine, `ticks` and `traces` gone (a mind takes no trace;
    // the counter dies with its one writer at M4.6).
    println!(
        "harness_arena_motor: cohort={cohort} waypoint_nodes={waypoint_nodes} decide_ms={} path_max_cells={} max_path_queries={} repath_ms={} intents={} shots={} path_queries={} path_deferred={} path_cells={} path_failures={} slope_refusals={} path_refused_unchanged={} corner_refusals={} peak_intents={} peak_shots={} peak_path_queries={} peak_path_cells={}",
        arena_cfg.reflex.decide_ms,
        arena_cfg.reflex.path_max_cells,
        arena_cfg.reflex.max_path_queries,
        arena_cfg.reflex.repath_ms,
        motor_total.intents,
        motor_total.shots,
        motor_total.path_queries,
        motor_total.path_deferred,
        motor_total.path_cells,
        motor_total.path_failures,
        motor_total.slope_refusals,
        motor_total.path_refused_unchanged,
        motor_total.corner_refusals,
        motor_peak.intents,
        motor_peak.shots,
        motor_peak.path_queries,
        motor_peak.path_cells
    );
    // 17b cut M4.5 (the orchestrator's Q6): the minds' own figures, printed
    // and in no compared leaf — the minds lane's blocks own them (from its
    // own runs, on the pool), so the baseline never carries one concept
    // twice. `settle_ms` is the inline delivery `run_wakes` times outside the
    // lens.
    let mut settle_sorted = settle_ms;
    settle_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    println!(
        "harness_arena_minds: cohort={cohort} engine=rom class=0 thinks={} landings={} faults={} overflows={} cond_fires={} settle_ms_p50={:.3} settle_ms_p99={:.3} (report-only)",
        motor_total.thinks,
        motor_total.landings,
        motor_total.mind_faults,
        motor_total.mind_overflows,
        motor_total.cond_fires,
        quantile(&settle_sorted, 500),
        quantile(&settle_sorted, 990)
    );
    println!(
        "harness_arena_minds_states: cohort={cohort} {}",
        machine_states_words(&states)
    );
    // **16b cut Y.2** (doc 16 §2 W8): the span of control and the order rate,
    // at the harness's own `[tacsim.command]` (the defaults, as `arena_cfg` is),
    // off the tape the ledger streamed and the minds' live orders at the end
    // (17b cut M4.5: joined to the tape, `live_orders_by_issuer`). On a
    // cohort with the LLM off and no human every figure but the two bounds and
    // the squad count is nought, which is the truthful measurement; the
    // enforcement itself is `tests/cockpit.rs`'s. Read after the measured loop.
    let command_cfg = tacsim::config::CommandConfig::default();
    let command = CommandFigures::of(&arena, &command_cfg, counts.ordered_lines, &counts.ordered);
    println!("{}", command.line("harness_arena_command", cohort));
    // **16b cut AF.2**: the snapshots this lane took — outside the lens and
    // outside the compared block ([`SnapshotFigures`]).
    println!("{}", snapshot.line(cohort));
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
    // Whether this run asserts its thresholds: under `TACSIM_ASSERT`, both
    // cohorts since 16b cut J.3b (the authored lane at its own per-window lens,
    // `window_lens_ms`).
    let asserting = knobs.assert;
    let lens = if asserting {
        "per-wake-and-per-window"
    } else {
        "report-only"
    };
    println!(
        "harness_arena_thresholds: cohort={cohort} assert={asserting} lens={lens} cut_a_p99_ms={ARENA_P99_MS} cut_s_p99_ms={ARENA_S_P99_MS} window_p99_ms={} engaged_p99_ms={} max_traces_per_pulse={} max_share_ops_per_pulse={} max_path_queries_per_pulse={} max_path_cells_per_pulse={} max_cell_probes_per_pulse={} max_jam_checks_per_pulse={} max_ledger_bytes={}",
        window_lens_ms(reach),
        knobs.engaged_p99_ms,
        knobs.max_traces_per_pulse,
        knobs.max_share_ops_per_pulse,
        knobs.max_path_queries_per_pulse,
        knobs.max_path_cells_per_pulse,
        knobs.max_cell_probes_per_pulse,
        knobs.max_jam_checks_per_pulse,
        knobs.max_ledger_bytes
    );

    let report = serde_json::json!({
        "what": "17b cut M4.5: `Arena::step` at the harness's scale, the fielded catalog flown by unit minds at the ROM, woken where the task wakes it, the thinks delivered inline outside the lens, with the cell-probe and jam-check bounds asserted",
        // Report-only execution-policy check; the committed deterministic block
        // and its baseline stay unchanged.
        "frame_execution": {
            "state_hash": tacsim::arena::hash::state_hash(&arena),
            "requested_workers": env_u32("TACSIM_FRAME_WORKERS", 1),
        },
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
            "move_cost_table": "harness-local (harness_move_cost), one table for every row: the cohort measures the arena and not the shipped table (16b cut AE.1)",
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
        // 17b cut M4.5: the `reflex` object, renamed `motor` with the
        // `ReflexConfig` facts kept (the config keeps its name) and the nine
        // counters both blocks carry — `ticks` retired, `traces` reported
        // nowhere (a mind takes none), no mind counter (the minds lane's).
        "motor": {
            "waypoint_nodes": waypoint_nodes,
            "waypoint_pitch_cells": WAYPOINT_PITCH,
            "decide_ms": arena_cfg.reflex.decide_ms,
            "path_max_cells": arena_cfg.reflex.path_max_cells,
            "max_path_queries_per_window": arena_cfg.reflex.max_path_queries,
            "repath_ms": arena_cfg.reflex.repath_ms,
            "total": {
                "intents": motor_total.intents,
                "shots": motor_total.shots,
                "path_queries": motor_total.path_queries,
                "path_deferred": motor_total.path_deferred,
                "path_cells": motor_total.path_cells,
                "path_failures": motor_total.path_failures,
                "slope_refusals": motor_total.slope_refusals,
                // 16b cut AI.1 (16c `B-84`).
                "path_refused_unchanged": motor_total.path_refused_unchanged,
                "corner_refusals": motor_total.corner_refusals,
            },
            "peak_per_pulse": {
                "intents": motor_peak.intents,
                "shots": motor_peak.shots,
                "path_queries": motor_peak.path_queries,
                "path_deferred": motor_peak.path_deferred,
                "path_cells": motor_peak.path_cells,
                "path_failures": motor_peak.path_failures,
                "slope_refusals": motor_peak.slope_refusals,
                // 16b cut AI.1 (16c `B-84`).
                "path_refused_unchanged": motor_peak.path_refused_unchanged,
                "corner_refusals": motor_peak.corner_refusals,
            },
        },
        // 17b cut M4.5: the machines' states at the end, off the doctrine
        // state (`machine_states`, seven fixed leaves). The baseline's
        // `minds` object also carries the minds lane's four variant blocks
        // beside this one, merged in at the bless (`bless_baseline`); the
        // fresh arena report never does.
        "minds": {
            "states_at_end": machine_states_json(&states),
        },
        // 16b cut Y.2 (doc 16 §2 W8): reported, and not in the deterministic
        // block — the committed baseline is untouched by decision.
        "command": command.json(),
        // 16b cut AF.2 (16c N16): the snapshots, the bound among them —
        // outside `DETERMINISTIC_OBJECTS`, `DETERMINISTIC_LEAVES` and
        // `thresholds`, as `command` is (`SnapshotFigures`' doc says why the
        // bound is here and not with the others).
        "snapshot": snapshot.json(),
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
            // 16b cut J.3b: the per-window lens this cohort is held to.
            "window_p99_ms": window_lens_ms(reach),
            "engaged_p99_ms": knobs.engaged_p99_ms,
            "max_traces_per_pulse": knobs.max_traces_per_pulse,
            "max_share_ops_per_pulse": knobs.max_share_ops_per_pulse,
            "max_path_queries_per_pulse": knobs.max_path_queries_per_pulse,
            // 16b cut AI.1 (16c `B-84`).
            "max_path_cells_per_pulse": knobs.max_path_cells_per_pulse,
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
        // **The lens, per wake and per window** (see [`ARENA_S_P99_MS`]): per wake
        // the per-wake lens on both cohorts, per window the cohort's own
        // (`window_lens_ms`; 16b cut J.3b; the VPS's figures since 16c T-109).
        assert!(
            p99 <= ARENA_S_P99_MS,
            "per-wake p99 {p99:.3} ms is past the {ARENA_S_P99_MS} ms lens"
        );
        let window_lens = window_lens_ms(reach);
        assert!(
            w99 <= window_lens,
            "per-window p99 {w99:.3} ms (500 ms windows of arena time) is past the \
             {window_lens} ms lens of the {cohort} cohort"
        );
        assert!(
            counts.bytes <= knobs.max_ledger_bytes,
            "ledger bytes {} is past harness.max_ledger_bytes {}",
            counts.bytes,
            knobs.max_ledger_bytes
        );
        // 16b cut S.1, doc 16 §2 W4 and W6: the two per-pulse perception bounds,
        // asserted against the **worst** wake of the run — the wake is the pulse
        // production runs (16b cut U.1). The reach each message names is the
        // cohort's own: the CI knobs, or the authored 60 and 30 (16b cut J.3b,
        // since the authored lane asserts too).
        let (sight_cells, mesh_cells) = match reach {
            Reach::Ci => (u32::from(sensor_cells()), share_cells()),
            Reach::Authored => (AUTHORED_SIGHT_CELLS, AUTHORED_SHARE_CELLS),
        };
        assert!(
            peak.traces <= knobs.max_traces_per_pulse,
            "traces in one wake {} is past harness.max_traces_per_pulse {} at \
             sensor_cells={} (doc 16 §2 W1: sensor ranges are short and bounded)",
            peak.traces,
            knobs.max_traces_per_pulse,
            sight_cells
        );
        // **16b cut U.2** (16c `T-36`; doc 16 §2 W4 names probes beside traces):
        // the cells those traces read, in the same shape.
        assert!(
            peak.cell_probes <= u64::from(knobs.max_cell_probes_per_pulse),
            "cell probes in one wake {} is past harness.max_cell_probes_per_pulse {} at \
             sensor_cells={} (doc 16 §2 W4: traces and cell probes per pulse)",
            peak.cell_probes,
            knobs.max_cell_probes_per_pulse,
            sight_cells
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
            mesh_cells
        );
        // **16b cut N.1**, doc 16 §1 A2's last per-pulse bound: *path queries per
        // pulse under their bound*. The arena's own budget is per `decide_ms`
        // window of arena time (D-16-2: a per-pulse throttle would bite
        // differently at each tempo), and a wake is at most a substep, inside one
        // window — so the per-wake figure the harness asserts is at most the
        // per-window figure the arena enforces.
        assert!(
            motor_peak.path_queries <= knobs.max_path_queries_per_pulse,
            "path queries in one wake {} is past harness.max_path_queries_per_pulse {}",
            motor_peak.path_queries,
            knobs.max_path_queries_per_pulse
        );
        // **16b cut AI.1** (16c `B-84`): and what they cost. A search counts
        // every cell it expanded, found or failed — a `Budget` failure is
        // `path_max_cells` of them and a `NoRoute` the whole reachable region —
        // so the queries bound alone bounds nothing about a wake's search time.
        // A threshold, never a throttle: the arena searches what it searches.
        assert!(
            motor_peak.path_cells <= knobs.max_path_cells_per_pulse,
            "A* cells expanded in one wake {} is past harness.max_path_cells_per_pulse {}",
            motor_peak.path_cells,
            knobs.max_path_cells_per_pulse
        );
        // **16b cut Y.2** (doc 16 §2 W8): the span of control and the orders
        // per commander per minute, bounded by `[tacsim.command]`.
        command.assert_bounded();
        // **16b cut AF.2** (16c N16): every snapshot decoded to the arena that
        // was encoded, and the largest is under the bound this lane measured.
        // No time bound: the encode's cost is a printed figure, and a lens it
        // moves is 16c `Q-AF5`, never a lens change.
        // And the bound is over something: an asserting run takes the snapshots
        // (`TACSIM_SNAPSHOT_OFF=1` is the off-and-on measurement's alone), one in
        // every period of arena time its measured wakes stood in.
        assert!(
            !knobs.snapshot_off,
            "TACSIM_SNAPSHOT_OFF=1 is for measurement only; an asserting run takes the snapshots"
        );
        assert!(
            snapshot.count >= 1 && snapshot.count == snapshot_periods,
            "the {cohort} cohort took {} snapshots over {snapshot_periods} periods of arena \
             time at every_ms={}",
            snapshot.count,
            snapshot.every_word()
        );
        assert!(
            snapshot.hash_ok,
            "a snapshot of the {cohort} cohort did not decode to the arena that was encoded"
        );
        assert!(
            u64::try_from(snapshot.bytes_max).unwrap_or(u64::MAX) <= knobs.max_snapshot_bytes,
            "the largest snapshot, {} bytes, is past harness.max_snapshot_bytes {}",
            snapshot.bytes_max,
            knobs.max_snapshot_bytes
        );
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
///
/// **17b cut M4.5** moved the controller's three objects with the field onto
/// minds at the ROM: `reflex.total` and `reflex.peak_per_pulse` are
/// `motor.total` and `motor.peak_per_pulse` (nine counters each, `ticks` and
/// `traces` gone), and `reflex.states_at_end` is `minds.states_at_end` (seven
/// leaves, off `Arena::machine_state`). Never `minds` whole: the baseline's
/// `minds` also carries the minds lane's four variant blocks, which the
/// fresh arena report never does and [`minds_delta`] compares on their own.
const DETERMINISTIC_OBJECTS: [&str; 9] = [
    "world",
    "substeps",
    "counters",
    "weapons",
    "perception.total",
    "perception.peak_per_pulse",
    "motor.total",
    "motor.peak_per_pulse",
    "minds.states_at_end",
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
/// file, and the comparison that follows reads it back. **17b cut M4.2**:
/// with `minds` (the four variants' blocks, [`minds_blocks_beside`]) the
/// document carries them under `minds`; without, it carries none and the
/// delta's minds path skips.
///
/// **A merge since 17b cut M4.5**: the fresh arena report carries its own
/// `minds` object (`minds.states_at_end`, a compared object), so the bless
/// KEEPS it and inserts — or, with `None`, removes — only the four variant
/// words ([`MINDS_VARIANT_WORDS`]) under it; nothing else of `minds` is the
/// bless's to write. The delta's bless refuses to write without all four
/// fresh variant files (see the delta test), so `None` is a test's path.
fn bless_baseline(
    path: &std::path::Path,
    fresh: &serde_json::Value,
    minds: Option<&serde_json::Value>,
) {
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let mut doc = fresh.clone();
    if let Some(map) = doc.as_object_mut() {
        map.insert(
            "blessed".to_string(),
            serde_json::json!({
                "commit": head_commit(),
                "date": iso_utc(now_s),
                "host": this_host(),
            }),
        );
        let mut merged = map
            .get("minds")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        for word in MINDS_VARIANT_WORDS {
            merged.remove(word);
        }
        if let Some(variants) = minds.and_then(serde_json::Value::as_object) {
            for (word, block) in variants {
                merged.insert(word.clone(), block.clone());
            }
        }
        if merged.is_empty() {
            map.remove("minds");
        } else {
            map.insert("minds".to_string(), serde_json::Value::Object(merged));
        }
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
        // 17b cut M4.5: the minds lane's four blocks fold in at every bless,
        // or the bless FAILS — a variant file missing, unreadable or older
        // than this binary is refused before anything is written, so a
        // baseline is never blessed without them or off an earlier build's
        // (17b cut M4.2 wrote a block-less file and passed).
        let minds = minds_blocks_beside(&knobs.json, binary_built).unwrap_or_else(|refused| {
            panic!(
                "harness_delta: the bless needs all four fresh minds reports beside the \
                 arena report (run tacsim_harness_arena_minds on this build first), and \
                 refused:\n{}",
                refused.join("\n")
            )
        });
        bless_baseline(&path, &fresh, Some(&minds));
        println!(
            "harness_delta: blessed {} (with the four minds blocks)",
            path.display()
        );
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
    // 17b cut M4.2: the minds lane's blocks, once the baseline carries them.
    let (minds_lines, minds_differences) = minds_delta(&baseline, &knobs.json, binary_built);
    for line in &minds_lines {
        println!("{line}");
    }
    assert_eq!(
        found.differences + minds_differences,
        0,
        "the deterministic block moved in {} field(s) (the `harness_delta:` lines above): \
         the arena did something different at the default knobs. Find the change, or \
         re-bless in the diff that made it with TACSIM_BASELINE_BLESS=1 and say why",
        found.differences + minds_differences
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
    // 16b cut J.3b: the fake report carries `thresholds.window_p99_ms`, the
    // per-window lens a cohort is held to; `report` is the CI cohort's 12.0.
    let report_at_lens = |p99: f64, blocked: u64, asserting: bool, window_lens: f64| {
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
            // 17b cut M4.5: the motor's two objects and the machines' states.
            "motor": {
                "total": { "intents": 7 }, "peak_per_pulse": { "intents": 1 },
            },
            "minds": { "states_at_end": { "engage": 2, "unknown": 0 } },
            "tempos": [{ "p50_ms": 1.0, "p99_ms": p99, "max_ms": p99 * 3.0 }],
            "setup_ms": { "warm_up": 100.0 },
            "thresholds": {
                "assert": asserting,
                "lens": if asserting { "per-wake-and-per-window" } else { "report-only" },
                "window_p99_ms": window_lens,
                "engaged_p99_ms": 25,
            },
        })
    };
    let report =
        |p99: f64, blocked: u64, asserting: bool| report_at_lens(p99, blocked, asserting, 12.0);
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
    // 16b cut J.3b (the value 20.0 since J.3c): a moved lens is a threshold like
    // any other — the authored cohort's 20.0 against a baseline at the CI
    // cohort's 12.0 is exactly one difference, named.
    let found = delta(&baseline, &report_at_lens(2.0, 10, true, 20.0), &host);
    assert_eq!(found.differences, 1, "{:?}", found.lines);
    assert!(
        found
            .lines
            .iter()
            .any(|line| line == "harness_delta: thresholds.window_p99_ms baseline=12.0 now=20.0"),
        "{:?}",
        found.lines
    );
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

/// **The worst asserted authored run on record**, ms per 500 ms window (16b cut
/// J.3c; 16c Q26): the J.3b gate's own run, 16.298 ms, the highest figure the
/// authored lane has read since it began asserting (cut J.3's three weekly runs
/// read 15.092, 14.997 and 14.108, its fourth 15.807, J.3b's three asserted
/// runs 15.339, 14.057 and 14.732). [`ARENA_AUTHORED_WINDOW_P99_MS`] carries
/// the owner's headroom over it, reported here and never asserted since 16b cut
/// J.3d ([`the_lens_is_the_cohorts_own`] holds only that the lens is above it).
/// Whoever records a worse asserted run that still passed the lens moves this
/// constant in the same diff and says why; a run past the lens is a failure the
/// weekly script reports, and the lens is the owner's to move, never the
/// builder's.
///
/// **16b cut Z.2 moved it to 17.087**: the second of Z.2's asserted weekly runs
/// read 17.087 ms per window on the authored cohort and passed the 20 ms lens
/// (the first read 15.341). Z.2 changes nothing the cohort runs — it issues no
/// order — so the figure is this host's noise, recorded as the worst asserted
/// run; 20 ms still clears it by the 16 per cent spread (17.087 × 1.16 =
/// 19.82).
///
/// **16b cut AE.3 moved it to 17.108**: the first of AE.3's asserted weekly
/// runs read 17.108 ms per window on the authored cohort and passed the 20 ms
/// lens (the re-run, after that run's delta read a per-wake p99 timing past its
/// 20 per cent, read 16.152 and a clean delta; the deterministic block was
/// equal both times). AE.3 changes nothing the cohort runs — it fields no
/// drone, the default cap touches no class the cohort fields, and the
/// elimination clause needs a drone — so the figure is this host's noise;
/// 20 ms still clears it by the 16 per cent spread (17.108 × 1.16 = 19.85),
/// under the 17.241 guard.
///
/// **16b cut J.3d moved it to 18.022** (the owner, 2026-09-29): cuts AE.3 and AF
/// read 17.623, 17.773, 17.579, 17.940 and 18.022 ms on the authored cohort with
/// the deterministic block equal every time, all under the 20 ms lens and all
/// past the 17.241 the retired ratio check could admit, so the record had frozen
/// below the runs; the check is gone and the record is the worst asserted run
/// that passed, 18.022 (cut AF's sweep). Twenty holds it by 11 per cent; the
/// spread the lane shows on this host is what a review reads here.
///
/// **16c `T-109` re-based it on the production host, 24.534** (2026-10-09):
/// the record is the VPS's now that the lenses are, and the first asserted
/// weekly run there under the 32 ms lens read the authored window p99 at
/// 24.534 ms (wake p99 12.970 under the 20 ms per-wake lens) and passed.
/// Thirty-two holds it by 30 per cent; ALIEN18's runs above are history.
const AUTHORED_WINDOW_P99_WORST_RECORDED_MS: f64 = 24.534;

/// **The lens is the cohort's own** (16b cut J.3b, moved by J.3c; 16c Q26): the
/// CI cohort is held per window to [`ARENA_S_P99_MS`] exactly as before (to its
/// own [`ARENA_CI_WINDOW_P99_MS`] since 16c `T-109`), the
/// authored cohort to [`ARENA_AUTHORED_WINDOW_P99_MS`], which is the looser of
/// the two; the authored lens is the owner's figure, 20 ms (32 since 16c
/// `T-109`), stays ABOVE the worst asserted run on record
/// ([`AUTHORED_WINDOW_P99_WORST_RECORDED_MS`]) and under the 25 ms (40 since
/// `T-109`) `engaged_p99_ms` ceiling doc 16 sets for the finished arena — the
/// check that keeps it a boundary, as the owner asked. It pins that the lens is
/// never quietly moved off the owner's figure, never sits at or under a run it
/// has already read, never reaches the ceiling, and never loosens the CI
/// cohort's. **16b cut J.3d** (the owner, 2026-09-29) retired the check that the
/// lens clears the worst run by the harness's 16 per cent spread: with the
/// authored lane reading 17.6 to 18.0 ms five times in cuts AE and AF under a
/// 20 ms lens the ratio could hold only by freezing the record below the runs,
/// so the spread is reported in the constant's doc and never asserted; headroom
/// is the owner's call at a review, on the figures.
///
/// **16c `T-109`** (the owner, 2026-10-09) moved all four figures to the
/// production host, the conclav.io VPS: per wake 20 ms, the CI window 22 (its
/// own constant now, split from the per-wake lens), the authored window 32,
/// and the `engaged_p99_ms` ceiling 40. The checks are the same checks.
#[test]
fn the_lens_is_the_cohorts_own() {
    let ci = window_lens_ms(Reach::Ci);
    let authored = window_lens_ms(Reach::Authored);
    assert_eq!(
        ARENA_S_P99_MS, 20.0,
        "the per-wake lens is the owner's 20 ms of 2026-10-09 (16c T-109); only the owner moves it"
    );
    assert_eq!(
        ci, ARENA_CI_WINDOW_P99_MS,
        "the CI cohort's window lens is its own constant"
    );
    assert_eq!(
        ci, 22.0,
        "the CI cohort's window lens is the owner's 22 ms of 2026-10-09 (16c T-109); only the \
         owner moves it"
    );
    assert!(
        ci >= ARENA_S_P99_MS,
        "a window carries two wakes: its lens {ci} ms is not under the per-wake {ARENA_S_P99_MS}"
    );
    assert_eq!(
        authored, ARENA_AUTHORED_WINDOW_P99_MS,
        "the authored cohort's window lens is its own constant"
    );
    assert_eq!(
        authored, 32.0,
        "the authored lens is the owner's figure of 2026-10-09 (16c T-109, after Q26's 20 of \
         2026-09-25); only the owner moves it"
    );
    assert!(
        authored > ci,
        "the authored lens {authored} is the looser of the two, over {ci}"
    );
    assert!(
        authored > AUTHORED_WINDOW_P99_WORST_RECORDED_MS,
        "the authored lens {authored} ms is at or under the worst asserted run on record, \
         {AUTHORED_WINDOW_P99_WORST_RECORDED_MS} ms: the owner's to move (16b cut J.3d)"
    );
    let ceiling = f64::from(tacsim::config::HarnessConfig::default().engaged_p99_ms);
    assert_eq!(
        ceiling, 40.0,
        "doc 16's engaged_p99_ms ceiling is 40 ms since the owner's T-109 of 2026-10-09"
    );
    assert!(
        authored < ceiling,
        "the authored lens {authored} ms reaches doc 16's {ceiling} ms engaged_p99_ms ceiling: no \
         longer a boundary"
    );
    assert!(
        ARENA_S_P99_MS * 2.0 <= ceiling,
        "the per-wake lens {ARENA_S_P99_MS} ms is over half the {ceiling} ms ceiling (16b cut S.1's \
         rule, kept by T-109)"
    );
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
    let run = run_wakes(
        &mut field.stepper,
        until_ms,
        |_| Vec::new(),
        |arena| {
            while arena.now_ms >= next_mark {
                samples.push(((next_mark - warm_end) / 1_000, resident_kib()));
                next_mark = next_mark.saturating_add(LONG_RUN_SAMPLE_S * 1_000);
            }
        },
    );
    let wall_s = started.elapsed().as_secs_f64();
    let rss_end_kib = resident_kib();
    let vmrss_end_kib = vmrss_kib();
    let arena = &field.stepper.arena;
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
        field.stepper.counts.bytes > 0,
        "the ledger streamed nothing: the long run measured an idle arena"
    );
    println!(
        "harness_long_run: cohort={cohort} arena_s={arena_s:.3} wakes={wakes} rss_start_kib={} rss_end_kib={} rss_growth_kib={} rss_peak_kib={} ledger_bytes={} ledger_chunks={} contacts_at_end={} due_heap_at_end={} held_losses_at_end={} ended={ended}{}",
        kib_word(rss_start_kib),
        kib_word(rss_end_kib),
        rss_growth_kib.map_or("absent".to_string(), |kib| kib.to_string()),
        kib_word(rss_peak_kib),
        field.stepper.counts.bytes,
        field.stepper.counts.chunks,
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
            "bytes": field.stepper.counts.bytes,
            "chunks": field.stepper.counts.chunks,
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
            field.stepper.counts.bytes <= knobs.max_ledger_bytes,
            "ledger bytes {} over the long run is past harness.max_ledger_bytes {}",
            field.stepper.counts.bytes,
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

/// **The lens**: 12 ms, asserted under `TACSIM_ASSERT=1` in release — **the
/// per-wake lens for both cohorts, and the CI cohort's per-window lens** (per
/// 500 ms window of arena time); the authored cohort's per-window lens is
/// [`ARENA_AUTHORED_WINDOW_P99_MS`] since 16b cut J.3b.
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
/// **At the authored reaches** ([`Reach::Authored`]) the measurement was
/// report-only until 16b cut J.3b. Cut S.2 recorded p99 23.3 ms and 13,133 traces in the peak pulse
/// there, and cut N.1 26.4 ms and 13,278 traces with the controller on the field,
/// against `harness.max_traces_per_pulse` of 4,000: the owner's choice between a
/// shorter authored reach and larger bounds is unchanged in kind, and the two
/// runs beside each other are what it is made on. [`ARENA_P99_MS`] stays in the
/// report as the record of what the arena cost before the passes ran. Since 16b
/// cut J.3b the authored cohort asserts too, against
/// [`ARENA_AUTHORED_WINDOW_P99_MS`] per window and this constant per wake.
///
/// **20 ms since 16c `T-109`** (the owner, 2026-10-09: *scale the wake and
/// window limits to the VPS, since that's where the game will run*; the
/// figures confirmed the same day). Twelve was a measurement on the owner's
/// desktop (ALIEN18), never a budget derived from the pulse, and the
/// conclav.io VPS that serves the game is 1.75 to 2 times slower a core: with
/// `T-87` paid it reads authored wake p99 12.8 to 22.6 ms (median 13.8) and CI
/// 8.8 to 10.0. Twenty is the owner's figure and is still half the
/// `harness.engaged_p99_ms` ceiling, which moved with it to 40. The CI
/// cohort's window no longer rides on this constant: it is
/// [`ARENA_CI_WINDOW_P99_MS`].
const ARENA_S_P99_MS: f64 = 20.0;

/// **The CI cohort's per-window lens** (16c `T-109`, the owner's figures of
/// 2026-10-09): 22 ms per 500 ms window of arena time, split from the per-wake
/// [`ARENA_S_P99_MS`] it shared since 16b cut S.1 — a window carries two wakes,
/// and the conclav.io VPS reads the CI window p99 at 15.9 to 18.9 ms with
/// `T-87` paid, so 22 is the harness's 16 per cent spread over the worst
/// ordinary run (18.9 × 1.16 = 21.9) where 20 would have held it by six.
const ARENA_CI_WINDOW_P99_MS: f64 = 22.0;

/// **The authored cohort's per-window lens** (16b cut J.3b, moved by J.3c; 16c
/// Q26, answered by the owner on 2026-09-25): 20 ms per 500 ms window of arena
/// time at the shipped 60-cell sight and 30-cell mesh, 300 units, 24 pilots.
///
/// **Where it came from.** Cut J.3 measured the authored cohort three times
/// after the cheap trace and the horizon index: per window p99 15.092, 14.997
/// and 14.108 ms, against the 12 ms lens that the CI cohort is asserted on. The
/// owner chose to keep the reach and the 300-unit target and to set this lens
/// with the harness's own margin over the worst of the three: 15.092 × 1.16 =
/// 17.507, taken as 17.5 by J.3b. J.3b's own gate then read 16.298 ms, which
/// 17.5 held by only 7.4 per cent — inside the spread two runs of one tree have
/// shown — and the owner moved it the same day: *bump the lens up to 20 to give
/// us some headroom but keep a reasonable boundary*. Twenty holds 16.298 by
/// 22.7 per cent, is the figure the 300-unit harness gate was first proposed at
/// on 2026-09-16 (before doc 16 set the finished arena's ceiling at 25), and
/// stays under that 25 ms `harness.engaged_p99_ms` ceiling, the check that keeps
/// it a boundary as the owner asked; [`the_lens_is_the_cohorts_own`] pins all
/// three and only the owner moves it.
///
/// **What it binds.** The per-window figure of [`Reach::Authored`] alone. The
/// per-wake lens is [`ARENA_S_P99_MS`] for both cohorts (the authored per-wake
/// p99 of 8.377 ms at worst holds 12 by 43 per cent), and the CI cohort's
/// window stays on [`ARENA_S_P99_MS`]: this constant loosens nothing there, and
/// the CI report's `thresholds.window_p99_ms` stays 12.0, so the committed
/// baseline does not move with it.
///
/// **32 ms since 16c `T-109`** (the owner, 2026-10-09: the lenses follow the
/// production host; the figures confirmed the same day). The conclav.io VPS
/// reads the authored window p99 at 24.6 to 27.3 ms with `T-87` paid (one run
/// of eight 43.0, the host's spread), so 32 is the harness's 16 per cent over
/// the worst ordinary run (27.3 × 1.16 = 31.7), and it stays under the
/// `harness.engaged_p99_ms` ceiling, moved to 40 for that, as Q26's answer
/// requires. The CI cohort's window is now [`ARENA_CI_WINDOW_P99_MS`], and the
/// CI report's `thresholds.window_p99_ms` reads 22.0.
const ARENA_AUTHORED_WINDOW_P99_MS: f64 = 32.0;

/// **The per-window lens a cohort is held to** (16b cut J.3b): the CI cohort
/// [`ARENA_CI_WINDOW_P99_MS`] (16c `T-109`; [`ARENA_S_P99_MS`] before it), the
/// authored cohort [`ARENA_AUTHORED_WINDOW_P99_MS`].
const fn window_lens_ms(reach: Reach) -> f64 {
    match reach {
        Reach::Ci => ARENA_CI_WINDOW_P99_MS,
        Reach::Authored => ARENA_AUTHORED_WINDOW_P99_MS,
    }
}

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
/// (`combat-scale-targets`), which at the shipped `[tacsim.arena].squad_size`
/// of four (16b cut Z.1) is six squads under command — enough for a per-squad
/// table and far short of what 300 units would spend.
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
            mode: tacsim::arena::MatchMode::Skirmish,
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
        expired = expired.saturating_add(u64::from(out.motor.orders_expired));
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
            // 17b cut L.1: the squad's lance-leader persona, as `match_::start`
            // mints it.
            let brief = SquadBrief::from_signal(
                1,
                signal,
                u16::try_from(members.len()).unwrap_or(u16::MAX),
                tacsim::deliberation::CharacterRef::Persona {
                    squad: signal.squad,
                },
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
    println!("{}", command.line("harness_arena_command", "cost"));

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
// 16c's sketch said "boards N real sessions"; the lobby has twelve pods, so the
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
    let mut cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    // **16b cut AF.2**: no snapshots on this lane. It runs the production task,
    // which reads `[tacsim.arena].snapshot_ms` off this config and would put a
    // `Snapshot` on the egress every 5,000 ms — six more world applies over the
    // match, each moving a snapshot document into an op and serialising a
    // twenty-four-pilot live record, inside `world_ms` and `applies`. The
    // snapshot's cost is the 30-second lane's to measure (`harness_arena_snapshot`),
    // and this lane stays the lane it was, in fact and not only in words.
    cfg.arena.snapshot_ms = 0;
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
    mods.push(comp::tacsim_module_with(cfg, true));
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

// ---------------------------------------------------------------------------
// The lance lane (16b cut Z.1; 16c N6: *lances*; doc 16 §2 W8, §6)
// ---------------------------------------------------------------------------
//
// **What this is.** The arena lanes above field three hundred machines nobody
// commands; the fan-out lane twenty-four pilots who never meet. Neither is the
// owner's "12 v 12 fighting" (`combat-scale-targets`) under command, which is
// what 16c N6 is for. This lane is: twenty-four units of the fielded catalog,
// twelve a side on the **shipped proving ground** at the **authored** reach,
// dealt into lances of four by `[tacsim.arena].squad_size`, with a scripted
// commander a side giving real orders through the arena's own phase 2.
//
// **The field.** Red musters on column 102 heading east and blue on column 156
// heading west, rows 97, 95, 99, 93, 101, … (the fan-out lane's alternation) —
// fifty-four cells apart astride the crossroads at (129, 97), inside the
// authored sight's sixty, so the two sides hold each other from the first
// sensor passes and most of the run is a fight. Seventy cells, the plan's first
// figure, is past the authored sight and would have measured an approach.
//
// **The commanders.** Each side's commander is its own lowest unit id — which
// is also its `red-1` / `blue-1` commander — and every order of that side names
// it as `issuer`, so the harness's [`live_orders_by_issuer`] (since 17b cut
// M4.5 the minds' live orders, each joined to its unit's last `ordered` line
// on the tape for the issuer) files the side's squads under one issuer and
// W8's span is a real figure (an `issuer: None` order is filed under each
// squad's own commander and reads a span of one whatever the script does).
// The script ([`lance_schedule`]) is a table ([`LANCE_TABLE`]) so cut Z.2 can
// extend it with `Screen`, `Support` and a release.
//
// **17b cut L.1** (doc 17 §4, D-UM-52): each side's orders are authored by one
// scripted lead character in `by` ([`RED_LEAD`] / [`BLUE_LEAD`], ids no unit of
// the lane uses) — a human lead's `ORDER` shape exactly, the lead's own tank the
// hint scope in `issuer`, so W8's span keeps its meaning. The world's admission
// (`arena_bridge::admit_order`) is bypassed by design: the lane steps the arena
// directly, and the arena reads no author.
//
// **What it asserts**, under `TACSIM_ASSERT` only. At **any** knob: W8's two
// bounds against `[tacsim.command]`; that the tape's `ordered` lines are exactly
// the orders the script gave (one a unit not already a wreck, at the millisecond
// phase 2 applied it, under the side's commander); that the opening round landed
// on every standing unit of every squad; that the span never reads more squads
// than the script holds under a live order; and that the sides fired. At the
// lane's **defaults** (seed, `TACSIM_LANCE_UNITS`, `TACSIM_LANCE_ARENA_S`,
// jitter nought — the field line's `at_defaults`) also the exact figures the
// defaults' fight gives: the span and the busiest commander minute, each
// commander's release on the tape as `order_released` lines for its first
// squad when that squad has a standing machine at the ninetieth arena second
// (16b cut Z.2; 16b cut AE.1 re-stated it through the wrecks, since a release
// refuses a wreck), and the opening round and one full minute a side on the
// tape at every scripted wake whose squad had a standing machine — fact pins
// (the end, the first squads' standings at the marks) hold the defaults to
// that fight, so the script landed and no figure is vacuous. Since 17b cut
// M4.5 the fight is the ROM's (the span the squads a side less one, the
// busiest minute the squads a side, eight orders, both first squads
// released: `measure_lance` has the figures and their history). Off the
// defaults a lance may be wiped out, reach its crossroads or take a mark a
// wake late, so the fight decides those figures and only the bounds bind them.
// **Timing is report-only** (the lead's rule
// for this cohort): the per-wake p99 is printed beside the per-wake lens and
// the per-window p99 beside the authored window lens (20 and 32 ms since 16c
// T-109, 12 and 20 before), and nothing is asserted about
// either. Its JSON (`<stem>-lance.<ext>`) is not read by the delta.

/// `TACSIM_LANCE_UNITS`' default: twelve a side.
const DEFAULT_LANCE_UNITS: u32 = 24;

/// `TACSIM_LANCE_ARENA_S`' default: two arena minutes after the warm-up — the
/// opening round and one full minute of the script.
const DEFAULT_LANCE_ARENA_S: u32 = 120;

/// Red's muster column, heading east.
const LANCE_RED_COLUMN: u16 = 102;

/// Blue's muster column, heading west.
const LANCE_BLUE_COLUMN: u16 = 156;

/// The muster's first row — the east-west road — and the axis of the
/// alternation.
const LANCE_ROW: i64 = 97;

/// The crossroads (`content/arenas/proving_ground.json`'s key spot and
/// waypoint `xroads`), which the script's `Advance` names.
const CROSSROADS: (u16, u16) = (129, 97);

/// One script minute, arena milliseconds.
const LANCE_MINUTE_MS: i64 = 60_000;

/// Every scripted order's time to live and horizon: longer than a minute, so a
/// squad ordered once a minute is never between orders.
const LANCE_ORDER_MS: u32 = 90_000;

/// **The arms the script steps a squad through**, one a minute, cycling (16b cut
/// Z.1). Every squad not being stepped is ordered `Engage` again, so every squad
/// is under some order at all times and most of the field is fighting.
///
/// **16b cut Z.2** extended it with the two squad roles, after `Withdraw` — so
/// with `Engage` the script gives six arms — and made it a table of
/// [`LanceArm`]s, because the two roles are written per squad: a screen faces
/// the enemy's column, and a support keeps station on the squad's own commander
/// (which fights on doctrine while the rest keep station on it). The release
/// is not an arm: it is given once a side ([`LANCE_RELEASE_MS`]).
const LANCE_TABLE: [LanceArm; 5] = [
    LanceArm::Hold,
    LanceArm::Advance,
    LanceArm::Withdraw,
    LanceArm::Screen,
    LanceArm::Support,
];

/// **One arm of [`LANCE_TABLE`]** (16b cut Z.2), resolved per squad by
/// [`LanceArm::objective`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LanceArm {
    /// `Objective::Hold`.
    Hold,
    /// `Objective::Advance` on [`CROSSROADS`].
    Advance,
    /// `Objective::Withdraw`.
    Withdraw,
    /// `Objective::Screen` toward the enemy's column: 90° for red, 270° for blue.
    Screen,
    /// `Objective::Support` of the squad's own commander.
    Support,
}

impl LanceArm {
    /// The arm, written for `roster`.
    fn objective(
        self,
        roster: &tacsim::deliberation::SquadRoster,
    ) -> tacsim::deliberation::Objective {
        use tacsim::deliberation::Objective;
        match self {
            LanceArm::Hold => Objective::Hold,
            LanceArm::Advance => Objective::Advance { to: CROSSROADS },
            LanceArm::Withdraw => Objective::Withdraw,
            LanceArm::Screen => Objective::Screen {
                facing_mdeg: if roster.id.team == tacsim::arena::state::Team::Red {
                    90_000
                } else {
                    270_000
                },
            },
            LanceArm::Support => Objective::Support {
                of: roster.commander,
            },
        }
    }
}

/// **When each side's commander releases its first squad** (16b cut Z.2; 16c
/// Q44): at the ninetieth arena second — arena millisecond 90,000, on the
/// period, so a wake at jitter nought visits it — one `ReleaseOrder` a side. At
/// the lane's defaults that falls between minute one's first mark (the first
/// squad's new order) and its second.
const LANCE_RELEASE_MS: i64 = 90_000;

/// The lance lane's own knobs.
struct LanceKnobs {
    /// `TACSIM_LANCE_UNITS`, even: half a side each.
    units: u32,
    /// `TACSIM_LANCE_ARENA_S`: arena seconds measured after the warm-up.
    arena_s: u32,
    /// **The rule the lane's match is fought under** (16b cut O.1):
    /// `Skirmish` for the lance lane, `Hold` for the report-only Hold lane
    /// ([`tacsim_harness_hold_lane`]), which runs on the shipped ground's own
    /// rule with `hold_ms` raised to `MAX_HOLD_MS` so no win ends the window.
    mode: tacsim::arena::MatchMode,
}

impl LanceKnobs {
    fn from_env() -> LanceKnobs {
        LanceKnobs {
            units: env_u32("TACSIM_LANCE_UNITS", DEFAULT_LANCE_UNITS),
            arena_s: env_u32("TACSIM_LANCE_ARENA_S", DEFAULT_LANCE_ARENA_S),
            mode: tacsim::arena::MatchMode::Skirmish,
        }
    }
}

/// **One scripted order**: when it is due, which squad, and what — `None` is
/// the squad's release (16b cut Z.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LanceOrder {
    at_ms: i64,
    squad: tacsim::deliberation::SquadId,
    objective: Option<tacsim::deliberation::Objective>,
}

/// **The script** (16b cut Z.1), as a list in the order it is given.
///
/// At `start_ms` (the warm-up's end) every squad of both sides is ordered
/// `Engage`. Then in minute `m` (1, 2, …, starting `m` minutes after
/// `start_ms` rounded up to the 500 ms engaged period) each side's squads are
/// ordered one at a time, ordinal `k` at `k − 1` staggers into the minute —
/// twenty seconds at three squads a side, the stagger `60 s / squads` rounded
/// down to the period — so every mark is a multiple of the period in absolute
/// arena time, which a wake at jitter nought always visits
/// (`Arena::next_due_ms`), and a mark's order lands at the mark exactly. The
/// squad whose ordinal is `(m − 1) %
/// squads_per_side + 1` takes [`LANCE_TABLE`]`[(m − 1) % len]` while every other
/// squad is ordered `Engage` again. Only marks before `until_ms` are listed.
///
/// **16b cut Z.2**: and at [`LANCE_RELEASE_MS`] of arena time, when the run spans it, each side's
/// first squad is released (`objective: None`), once; the list stays in the
/// order it is given, a release between the marks around it.
///
/// Three orders a minute from one commander is exactly `squads_per_side` in any
/// sixty seconds, because the marks of consecutive minutes are the same
/// staggers a minute apart; that is what makes W8's rate figure a real one. A
/// release is not an order and counts in neither figure.
fn lance_schedule(
    squads: &[tacsim::deliberation::SquadRoster],
    start_ms: i64,
    until_ms: i64,
) -> Vec<LanceOrder> {
    use tacsim::arena::state::Team;
    use tacsim::deliberation::Objective;
    let per_side = |team: Team| {
        squads
            .iter()
            .filter(|roster| roster.id.team == team)
            .count()
    };
    let squads_per_side = per_side(Team::Red).max(per_side(Team::Blue)).max(1);
    let stagger =
        (LANCE_MINUTE_MS / i64::try_from(squads_per_side).unwrap_or(1)) / WINDOW_MS * WINDOW_MS;
    let mut out: Vec<LanceOrder> = squads
        .iter()
        .map(|roster| LanceOrder {
            at_ms: start_ms,
            squad: roster.id,
            objective: Some(Objective::Engage { target_hint: None }),
        })
        .collect();
    let anchor = start_ms
        .div_euclid(WINDOW_MS)
        .saturating_add(i64::from(start_ms.rem_euclid(WINDOW_MS) != 0))
        * WINDOW_MS;
    let mut minute = 1_i64;
    loop {
        let base = anchor.saturating_add(minute.saturating_mul(LANCE_MINUTE_MS));
        if base >= until_ms {
            break;
        }
        let stepped = usize::try_from((minute - 1) % i64::try_from(squads_per_side).unwrap_or(1))
            .unwrap_or(0)
            + 1;
        let arm = LANCE_TABLE
            .get(usize::try_from(minute - 1).unwrap_or(0) % LANCE_TABLE.len())
            .copied()
            .unwrap_or(LanceArm::Hold);
        for ordinal in 1..=squads_per_side {
            let at_ms = base.saturating_add(
                i64::try_from(ordinal - 1)
                    .unwrap_or(0)
                    .saturating_mul(stagger),
            );
            if at_ms >= until_ms {
                break;
            }
            for roster in squads
                .iter()
                .filter(|roster| usize::from(roster.id.index) == ordinal)
            {
                out.push(LanceOrder {
                    at_ms,
                    squad: roster.id,
                    objective: Some(if ordinal == stepped {
                        arm.objective(roster)
                    } else {
                        Objective::Engage { target_hint: None }
                    }),
                });
            }
        }
        minute += 1;
    }
    // **16b cut Z.2**: one release a side, of its first squad, at the ninetieth
    // arena second when the run spans it.
    let released_at = LANCE_RELEASE_MS;
    if released_at >= start_ms && released_at < until_ms {
        for roster in squads.iter().filter(|roster| roster.id.index == 1) {
            out.push(LanceOrder {
                at_ms: released_at,
                squad: roster.id,
                objective: None,
            });
        }
    }
    // In the order given: stable, so a mark's own orders keep their order.
    out.sort_by_key(|order| order.at_ms);
    out
}

/// **The red side's scripted lead character** (17b cut L.1): the author, in
/// `by`, of every order the lance lane gives red. No unit of the lane uses the
/// id (the lane's units are `100_000 + n`).
const RED_LEAD: ObjId = ObjId(1_001);

/// **The blue side's scripted lead character** (17b cut L.1), as [`RED_LEAD`].
const BLUE_LEAD: ObjId = ObjId(2_001);

/// The scripted lead character of `team` (17b cut L.1).
const fn lead_of(team: tacsim::arena::state::Team) -> ObjId {
    match team {
        tacsim::arena::state::Team::Red => RED_LEAD,
        tacsim::arena::state::Team::Blue => BLUE_LEAD,
    }
}

/// **One scripted order as the intents phase 2 applies**: one per unit of the
/// squad the arena still holds, at that unit's own current `helm` epoch, the
/// side's commander as issuer and (17b cut L.1) the side's scripted lead
/// character as author. **16b cut Z.2**: a release is one
/// `ReleaseOrder { squad, issuer }` per unit the same way.
fn lance_intents(
    arena: &tacsim::arena::state::Arena,
    order: &LanceOrder,
    issuer: ObjId,
    seq: &mut u64,
) -> Vec<tacsim::arena::intents::Intent> {
    use tacsim::arena::intents::{Intent, IntentKind};
    use tacsim::arena::state::Channel;
    let Some(roster) = arena.squads.iter().find(|roster| roster.id == order.squad) else {
        return Vec::new();
    };
    let kind = match order.objective {
        Some(objective) => IntentKind::Order(Box::new(tacsim::deliberation::Order {
            squad: order.squad,
            objective,
            target: None,
            horizon_ms: LANCE_ORDER_MS,
            constraints: tacsim::deliberation::Constraints::default(),
            ttl_ms: LANCE_ORDER_MS,
            issued_ms: 0,
            issuer: Some(issuer),
            by: tacsim::deliberation::CharacterRef::Character {
                id: lead_of(order.squad.team),
            },
            spot: None,
        })),
        None => IntentKind::ReleaseOrder {
            squad: order.squad,
            issuer: Some(issuer),
        },
    };
    roster
        .units
        .iter()
        .filter(|unit| arena.unit(**unit).is_some())
        .map(|unit| {
            *seq += 1;
            Intent {
                unit: *unit,
                channel: Channel::Helm,
                epoch: arena.epochs.current(*unit, Channel::Helm),
                kind: kind.clone(),
                seq: *seq,
            }
        })
        .collect()
}

/// **How many supporters' minds target their charge's target** (16b cut Z.2;
/// onto minds by 17b cut M4.5), as `(supporters, on_target, charge_unheld)`:
/// every mind under a live `Support` (`MindState.order`'s
/// `MindObjective::Support`, unexpired at the read — a self-charge was
/// re-keyed to `Engage` when written, 17b cut L.2, so every one names another
/// unit), and those whose memory's target is the object the charge's memory
/// targets — each memory's `target` row resolved in its own hull's scope.
/// The charge is the supporter's own row for it (`of`), which is `None`
/// while the supporter does not hold the charge classified (17b cut M1b.3):
/// such a supporter counts, can never be on target, and is counted again as
/// `charge_unheld`. A wreck is no supporter: its mind row keeps the order it
/// died under and a memory nobody updates. A report figure, read once off
/// the last wake and never asserted: whether a supporter holds its charge's
/// target is the fight's.
///
/// **What `charge_unheld` means since 17b cut M4.5c**: a charge NOT HELD by
/// the supporter's frame — the order's `of` is `None`, or the frame has lost
/// the contact it names — and nothing else. The row cut seats the standing
/// order's contact (`Support`'s charge) before every other row, so a charge
/// the frame holds is always among the mind's view rows and is never counted
/// here for a row the cut dropped (before that cut a supporter crowded by
/// hostiles could lose its held charge from its view, and this figure could
/// not see it). At the lane's defaults no `Support` is given in the 120
/// seconds, so it reads nought (17b cut M4.5's fight).
fn supporters_on_charge(arena: &tacsim::arena::state::Arena) -> (usize, usize, usize) {
    use tacsim::arena::perception::MindObjective;
    // The object a hull's mind targets: its memory's row, in its own scope.
    let target_of = |unit: ObjId| -> Option<ObjId> {
        let row = arena.minds.get(&unit)?;
        let id = tacsim::arena::stock_mind::StockMemory::from(&tacsim::arena::mind::decode_memory(
            &row.memory,
        ))
        .target?;
        arena.contacts.get(unit, id).map(|held| held.target)
    };
    let mut supporters = 0_usize;
    let mut on_target = 0_usize;
    let mut charge_unheld = 0_usize;
    for (unit, row) in &arena.minds {
        if arena.unit(*unit).is_none_or(|state| {
            state.controller != tacsim::arena::state::Controller::Mind || state.limits.wreck
        }) {
            continue;
        }
        let Some(order) = row.order.as_ref() else {
            continue;
        };
        let MindObjective::Support { of } = order.objective else {
            continue;
        };
        if order.expires_ms <= arena.now_ms {
            continue;
        }
        supporters += 1;
        let Some(charge) = of
            .and_then(|id| arena.contacts.get(*unit, id))
            .map(|held| held.target)
        else {
            charge_unheld += 1;
            continue;
        };
        let charges_target = target_of(charge);
        if charges_target.is_some() && target_of(*unit) == charges_target {
            on_target += 1;
        }
    }
    (supporters, on_target, charge_unheld)
}

/// **The lance lane** (16b cut Z.1): [`measure_lance`] at `TACSIM_LANCE_UNITS`
/// (24) for `TACSIM_LANCE_ARENA_S` (120) arena seconds, JSON at
/// `<stem>-lance.<ext>`; W8 asserted under `TACSIM_ASSERT`, and at the lane's
/// defaults pinned to the defaults' own fight (at the ROM since 17b cut M4.5:
/// [`measure_lance`]); timing report-only.
#[test]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
fn tacsim_harness_lance() {
    measure_lance(&Knobs::from_env(), &LanceKnobs::from_env());
}

/// Whether the report-only Hold lane was asked for (16b cut O.1; plan item 27).
/// **Exactly `"1"`**, the cost run's rule ([`cost_run_requested`]).
fn hold_lane_requested(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.trim() == "1")
}

/// **The report-only Hold lane** (16b cut O.1; plan item 27; Q-O3): the lance
/// lane's shape at `MatchMode::Hold` on the shipped ground's own rule (The
/// Crossroads), with `hold_ms` raised to `MAX_HOLD_MS` so no win ends the
/// window, printing the lance lane's `harness_*` lines (cohort `lance_hold`,
/// `ended=` among them) and its goal cost — the machines doctrine governs
/// (under `Engage`, a `Support` off station, or a refused call) drive to the
/// spot, and that search is what it costs.
///
/// **On minds since 17b cut M4.5, and under STOP A.** The lane fields the
/// lance lane's ROM minds ([`rom_terms`]); but the Hold mode never reaches a
/// production think — no view carries the spot and no production think hands
/// the stock mind's Hold arms the objective (STOP A: held on the reflex in
/// `tests/reflex.rs` until 17b cut M4.6b deleted those residues, the owner
/// carrying it to the residues cut after 17b's M5 (OQ-6; re-pointed by 17b
/// cut M5.7) — the loss recorded in doc 17 §24; the Hold arms stay
/// pinned on hand-built holds by `stock_mind.rs`' `m1a6_*` kernels) — so its
/// machines are NOT driven to the spot by the
/// Hold objective, and its goal cost is the minds' own fight at Hold's
/// rule, not the spot's search it was built to price. It still runs the
/// lance lane's shape at `MatchMode::Hold` and prints every line (the
/// `ended=` word, the goal cost, the states); it asserts what it still can
/// — nothing, as before: no claim about the Hold objective is one a mind can
/// make yet, and the lance lane's fact pins are the Skirmish fight's.
///
/// **Not `#[ignore]`d, and flag-gated instead**: every ignored test in this file
/// is run by `scripts/harness-weekly.sh` and the CI `tacsim-harness` job, under
/// `TACSIM_ASSERT=1`, and whether this lane joins them is Q-O3 for the owner
/// (default no). It returns at once unless `TACSIM_HOLD_LANE` is exactly `1`,
/// so the ordinary workspace run proves the early return, and it passes
/// `assert: false` whatever `TACSIM_ASSERT` says: it ASSERTS NOTHING — no lens,
/// no bound, no baseline. Run it by hand:
/// `TACSIM_HOLD_LANE=1 cargo test --release -p tacsim --test harness tacsim_harness_hold_lane -- --exact --nocapture`.
#[test]
fn tacsim_harness_hold_lane() {
    if !hold_lane_requested(std::env::var("TACSIM_HOLD_LANE").ok().as_deref()) {
        return;
    }
    let mut knobs = Knobs::from_env();
    knobs.assert = false;
    measure_lance(
        &knobs,
        &LanceKnobs {
            mode: tacsim::arena::MatchMode::Hold,
            ..LanceKnobs::from_env()
        },
    );
}

/// **16b cut O.1**: the Hold lane is off unless its flag says `1` — the cost
/// run's twin — and an ordinary test run does not set it.
#[test]
fn the_hold_lane_is_off_unless_its_flag_says_1() {
    assert!(!hold_lane_requested(None));
    assert!(!hold_lane_requested(Some("")));
    assert!(!hold_lane_requested(Some("0")));
    assert!(!hold_lane_requested(Some("true")));
    assert!(!hold_lane_requested(Some("yes")));
    assert!(hold_lane_requested(Some("1")));
    assert!(hold_lane_requested(Some(" 1 ")));
    assert!(
        !hold_lane_requested(std::env::var("TACSIM_HOLD_LANE").ok().as_deref()),
        "TACSIM_HOLD_LANE is set in this environment: the Hold lane is report-only and run by \
         hand, so an ordinary test run must not request it"
    );
}

/// **The lance lane, small and un-ignored** (16b cut Z.1): eight units — one
/// lance a side — for fourteen arena seconds (ten until 17b cut M4.5, below)
/// on the default seed at jitter nought, its JSON under the system temp
/// directory. It proves the lane runs: the
/// lines print, the opening round landed on both squads under their own
/// commanders, and the report is the `-lance` sibling. It asserts whatever
/// `TACSIM_ASSERT` says, so the lane's any-knob checks — W8's bounds, the tape
/// is the script, the opening round on every standing unit, the span inside
/// what the script holds, the sides fired — run on every `cargo test`; its
/// knobs are off the lane's defaults, so the exact figures do not (and timing
/// is never asserted). It boots the production roster with the proving ground
/// once, which is most of its cost.
///
/// **The window re-derived at 17b cut M4.5, the claim kept** (the M4.4
/// ruling 1 discipline). The reflex fired at anything in its weapon's reach;
/// the ROM's shot is gated by its one condition slot, at the lock's odds (a
/// think's `Fire` cites the slot, which also fires on its own), so it closes
/// first. The two lances muster 54 cells — 540 m — apart
/// ([`LANCE_RED_COLUMN`], [`LANCE_BLUE_COLUMN`]), and on this field the
/// slot's odds clear at about 400 m (the first shot's `range_cm` reads
/// 40,444): the lances must close some 140 m between them, which at the
/// ROM's approach takes them past the old window — at ten seconds (21,500 ms)
/// all eight stood in `approach` and nothing had fired. Measured at the
/// defaults of this smoke: the first `fired` line is at 23,529 ms, 12,029 ms
/// after the opening round at 11,500, and every hull of both lances has fired
/// once by 24,206 ms (12,706 ms after it). The window is whole arena seconds
/// from the 11,500 ms start, so thirteen (to 24,500 ms) is the least that
/// holds every hull's first shot, and fourteen (to 25,500 ms) keeps one ROM
/// think period (1,000 ms) beyond it, so a fight that moves by one think
/// still fires inside the window rather than failing on it. The claim — the
/// lances fired (`fired > 0`) — is unchanged and still discriminates: at ten
/// seconds it fails. Fourteen is still far short of the release at
/// [`LANCE_RELEASE_MS`] (the `released=0` check below).
///
/// **The lattice the window and the first shots fall on.** At jitter nought
/// every wake ends on the 250 ms substep lattice (`substep_ms=250`), so the
/// run stops exactly at the 11,500 ms start plus whole seconds — 25,500 ms
/// here. A ROM hull thinks on its own Class 0 grid, its stagger in
/// `[0, 1,000)` plus whole seconds (`Arena::grid_at` at the class row's
/// 1,000 ms), and a think's answer lands one substep (250 ms) later, where its
/// cited `Fire` is applied: every first shot of this smoke is a landing.
/// The first, #100001's at 23,529 ms, is its think at 23,279 (stagger 279)
/// landed; the last hull's first, #100006's at 24,206 ms, its think at 23,956
/// (stagger 956) — each hull's every cited shot falls at the same millisecond
/// of its second. So a fight that moves moves each first shot by whole think
/// periods on its hull's grid: by one, the last to at most 25,206 ms — inside
/// fourteen seconds' 25,500 and past thirteen's 24,500, which is why the
/// window keeps the one period beyond the least that holds. **17b cut
/// M4.5c's row cut does not move this smoke** (measured again after it: the
/// first shot 23,529 ms, every hull's first by 24,206): a hull here holds at
/// most seven contacts — three friends and four enemies — never more than its
/// eight rows, and a view that truncates nothing is the old cut byte for
/// byte.
#[test]
fn z1_the_lance_lane_runs_small() {
    let mut knobs = Knobs::from_env();
    // Asserting whatever `TACSIM_ASSERT` says, on the default seed at jitter
    // nought, so the run is the same one on every machine.
    knobs.assert = true;
    knobs.seed = DEFAULT_SEED;
    knobs.jitter_ms = 0;
    knobs.json = std::env::temp_dir()
        .join("tacsim-lance-smoke")
        .join("harness.json")
        .to_string_lossy()
        .to_string();
    let report = measure_lance(
        &knobs,
        &LanceKnobs {
            units: 8,
            arena_s: 14,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
    );
    for prefix in [
        "harness_lance_field:",
        "harness_lance_command:",
        "harness_lance_wakes:",
        "harness_lance_window:",
        "harness_lance_minds_states:",
        "harness_lance_weapons:",
        "harness_lance_perception_total:",
        "harness_lance_thresholds:",
        "harness_lance_json:",
    ] {
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.starts_with(prefix) && line.contains("cohort=lance")),
            "no `{prefix}` line: {:#?}",
            report.lines
        );
    }
    assert_eq!(
        report.squads_per_side, 1,
        "four machines a side are one lance"
    );
    // The opening round: both squads, at the warm-up's end, each under its own
    // side's commander, one line a unit.
    let opening: Vec<(String, Option<ObjId>)> = report
        .ordered
        .iter()
        .filter(|line| line.t_ms == report.start_ms)
        .map(|line| (line.squad.clone(), line.issuer))
        .collect();
    for (squad, commander) in [
        ("red-1", report.commanders[0]),
        ("blue-1", report.commanders[1]),
    ] {
        assert_eq!(
            opening
                .iter()
                .filter(|(word, issuer)| word == squad && *issuer == Some(commander))
                .count(),
            4,
            "the opening round landed on every unit of {squad}: {opening:?}"
        );
    }
    let stem = report
        .json
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    assert_eq!(stem, "harness-lance");
    assert!(report.json.exists(), "{}", report.json.display());

    // **16b cut Z.2**: the smoke's fourteen seconds reach no release — the line
    // says so — and the table cycles, read off the script this field would be
    // given over seven minutes: one lance a side, so its one squad is stepped
    // every minute through the five arms and back to the first, and released
    // once at ninety seconds.
    assert!(
        report.lines.iter().any(|line| line
            .starts_with("harness_lance_command:")
            && line.contains(" released=0 ")),
        "{:#?}",
        report.lines
    );
    let rosters: Vec<tacsim::deliberation::SquadRoster> = [
        (tacsim::arena::state::Team::Red, report.commanders[0]),
        (tacsim::arena::state::Team::Blue, report.commanders[1]),
    ]
    .into_iter()
    .map(|(team, commander)| tacsim::deliberation::SquadRoster {
        id: tacsim::deliberation::SquadId { team, index: 1 },
        commander,
        units: vec![commander],
    })
    .collect();
    let script = lance_schedule(&rosters, report.start_ms, report.start_ms + 420_000);
    use tacsim::deliberation::Objective;
    let red: Vec<Option<Objective>> = script
        .iter()
        .filter(|order| {
            order.squad.team == tacsim::arena::state::Team::Red && order.at_ms > report.start_ms
        })
        .map(|order| order.objective)
        .collect();
    assert_eq!(
        red,
        vec![
            Some(Objective::Hold),
            None,
            Some(Objective::Advance { to: CROSSROADS }),
            Some(Objective::Withdraw),
            Some(Objective::Screen {
                facing_mdeg: 90_000
            }),
            Some(Objective::Support {
                of: report.commanders[0]
            }),
            Some(Objective::Hold),
        ],
        "red's lance: the five arms in turn, the release once, and the table again"
    );
    assert_eq!(
        script
            .iter()
            .filter(|order| order.objective.is_none())
            .map(|order| order.squad.to_string())
            .collect::<Vec<_>>(),
        vec!["red-1".to_string(), "blue-1".to_string()],
        "one release a side"
    );
}

/// **The script's own W8 figures** (16b cut Z.1): at three squads a side over
/// the default two minutes, one commander's orders are the opening round and
/// one minute's three, so [`orders_on_tape`] reads four orders and three in the
/// busiest sixty seconds — `squads_per_side` — and every squad but the first
/// (16b cut Z.2: released at ninety seconds, which is no order) is under an
/// order from the commander at the end. At five minutes, and at two squads, the
/// busiest minute is still exactly the squads a side.
#[test]
fn z1_the_lance_script_is_three_orders_a_minute_from_one_commander() {
    use tacsim::arena::state::Team;
    use tacsim::deliberation::{Objective, SquadId, SquadRoster};
    let rosters = |per_side: u16| -> Vec<SquadRoster> {
        let mut out = Vec::new();
        for (team, first) in [(Team::Red, 100_u32), (Team::Blue, 200)] {
            for index in 1..=per_side {
                let commander = ObjId(first + u32::from(index) * 4);
                out.push(SquadRoster {
                    id: SquadId { team, index },
                    commander,
                    units: (0..4).map(|n| ObjId(commander.0 + n)).collect(),
                });
            }
        }
        out
    };
    let issuer_of = |team: Team| {
        if team == Team::Red {
            ObjId(104)
        } else {
            ObjId(204)
        }
    };
    let tape = |schedule: &[LanceOrder]| -> Vec<OrderedLine> {
        schedule
            .iter()
            .filter(|order| order.objective.is_some())
            .map(|order| OrderedLine {
                t_ms: order.at_ms,
                issuer: Some(issuer_of(order.squad.team)),
                squad: order.squad.to_string(),
                unit: issuer_of(order.squad.team),
                ttl_ms: LANCE_ORDER_MS,
            })
            .collect()
    };
    let start = 20_500_i64;

    let three = rosters(3);
    let schedule = lance_schedule(&three, start, start + 120_000);
    let red: Vec<&LanceOrder> = schedule
        .iter()
        .filter(|order| order.squad.team == Team::Red)
        .collect();
    let marks: Vec<(i64, u16, Option<Objective>)> = red
        .iter()
        .map(|order| (order.at_ms - start, order.squad.index, order.objective))
        .collect();
    let engage = Objective::Engage { target_hint: None };
    assert_eq!(
        marks,
        vec![
            (0, 1, Some(engage)),
            (0, 2, Some(engage)),
            (0, 3, Some(engage)),
            (60_000, 1, Some(Objective::Hold)),
            // 16b cut Z.2: red-1's release, once, at the ninetieth arena
            // second — 69.5 s into a run that starts at 20.5 s.
            (69_500, 1, None),
            (80_000, 2, Some(engage)),
            (100_000, 3, Some(engage)),
        ],
        "the opening round, then minute one: red-1 holds, the others engage again, and \
         red-1 is released between the first and second marks"
    );
    let red_tape = tape(&schedule)
        .into_iter()
        .filter(|line| line.squad.starts_with("red"))
        .collect::<Vec<_>>();
    assert_eq!(orders_on_tape(&red_tape, &three), (4, 3));
    // Both sides: eight orders, the busiest commander still three.
    assert_eq!(orders_on_tape(&tape(&schedule), &three), (8, 3));

    // Five minutes: every arm of the table in turn, and never more than three
    // in any sixty seconds.
    let long = lance_schedule(&three, start, start + 300_000);
    let stepped: Vec<(i64, u16, Option<Objective>)> = long
        .iter()
        .filter(|order| order.squad.team == Team::Blue && order.objective != Some(engage))
        .map(|order| (order.at_ms - start, order.squad.index, order.objective))
        .collect();
    assert_eq!(
        stepped,
        vec![
            (60_000, 1, Some(Objective::Hold)),
            (69_500, 1, None),
            (140_000, 2, Some(Objective::Advance { to: CROSSROADS })),
            (220_000, 3, Some(Objective::Withdraw)),
            // 16b cut Z.2: the fourth arm, a screen toward red's column.
            (
                240_000,
                1,
                Some(Objective::Screen {
                    facing_mdeg: 270_000
                })
            ),
        ]
    );
    assert_eq!(orders_on_tape(&tape(&long), &three).1, 3);

    // A warm-up that ends off the period: the opening round at its end, the
    // minutes counted from the next multiple of the period.
    let off = lance_schedule(&three, start - 250, start + 120_000);
    assert_eq!(off.first().map(|order| order.at_ms), Some(start - 250));
    assert!(
        off.iter()
            .skip(6)
            .all(|order| order.at_ms.rem_euclid(WINDOW_MS) == 0),
        "every minute's mark is on the period"
    );
    assert_eq!(orders_on_tape(&tape(&off), &three), (8, 3));

    // Two squads a side: thirty-second staggers, two a minute.
    let two = rosters(2);
    let paired = lance_schedule(&two, start, start + 300_000);
    assert!(
        paired
            .iter()
            .any(|order| order.at_ms == start + 90_000 && order.squad.index == 2)
    );
    assert_eq!(orders_on_tape(&tape(&paired), &two).1, 2);
}

/// **One mind's standing order, written straight onto its row** (17b cut
/// M4.5's review), for the span's join tests: `squad`'s `Hold` until
/// `expires_ms`, no fire restriction.
fn standing_order(squad: &str, expires_ms: i64) -> tacsim::arena::perception::FrameOrder {
    tacsim::arena::perception::FrameOrder {
        squad: smol_str::SmolStr::new(squad),
        objective: tacsim::arena::perception::MindObjective::Hold,
        hold_fire: false,
        max_range_band: None,
        expires_ms,
    }
}

/// One hand-written `ordered` line for the span's join tests.
fn tape_line(
    t_ms: i64,
    issuer: Option<ObjId>,
    squad: &str,
    unit: ObjId,
    ttl_ms: u32,
) -> OrderedLine {
    OrderedLine {
        t_ms,
        issuer,
        squad: squad.to_string(),
        unit,
        ttl_ms,
    }
}

/// **The span's join, branch by branch** (17b cut M4.5's review; the
/// orchestrator's ruling on STOP C): [`live_orders_by_issuer`] over a real
/// arena — the lance smoke's field, two lances of four opened at arena
/// millisecond nought ([`lance_field_arena`]) — whose mind rows carry orders
/// written by hand, against a tape written by hand. The branches, each a hull
/// of the field:
///
/// * `red-1`'s second hull holds a live order whose LAST line names no
///   issuer: filed under `red-1`'s commander (the `None` fallback), and an
///   earlier line of its own with another issuer and another deadline is not
///   the one joined (orders are latest-wins);
/// * `red-1`'s third hull holds an order whose deadline is the read's own
///   millisecond, and no line: not live, so never joined and never refused;
/// * `red-1`'s fourth hull holds no order, and a line of its own: the tape
///   alone is no live order;
/// * `blue-1`'s second hull is a WRECK holding a live order, its line naming
///   `blue-1`'s commander: counted (a wreck keeps the order it died under, as
///   the reflex reader counted a wreck's row), and the only `blue-1` hull that
///   holds one — so the `blue-1` group is the wreck's alone;
/// * `blue-1`'s third hull is flown by a pilot, holding a live order and no
///   line: not a mind's order, so never joined and never refused.
///
/// The figure — one squad under each commander, nothing under the hull's
/// earlier issuer — and, with the wrecks skipped ([`Wrecks::Skipped`]),
/// `red-1`'s group alone. The three refusals the join's assert makes are
/// pinned by the three `#[should_panic]` tests after this one.
#[test]
fn m4_5_the_span_joins_each_live_order_to_its_last_ordered_line() {
    use tacsim::arena::state::{Controller, Team};
    let mut arena = lance_field_arena(8);
    assert_eq!(arena.now_ms, 0, "the field is opened, never stepped");
    let roster = |team: Team| {
        arena
            .squads
            .iter()
            .find(|roster| roster.id.team == team)
            .cloned()
            .expect("one lance a side")
    };
    let (red, blue) = (roster(Team::Red), roster(Team::Blue));
    assert_eq!((red.units.len(), blue.units.len()), (4, 4));
    assert_eq!(
        (red.id.to_string(), blue.id.to_string()),
        ("red-1".to_string(), "blue-1".to_string())
    );
    let (none_issuer, expired, unordered) = (red.units[1], red.units[2], red.units[3]);
    let (wreck, piloted) = (blue.units[1], blue.units[2]);
    for (unit, order) in [
        (none_issuer, Some(standing_order("red-1", 90_000))),
        (expired, Some(standing_order("red-1", 0))),
        (unordered, None),
        (wreck, Some(standing_order("blue-1", 30_000))),
        (piloted, Some(standing_order("blue-1", 60_000))),
    ] {
        arena
            .minds
            .get_mut(&unit)
            .expect("every lance hull is a mind")
            .order = order;
    }
    arena.unit_mut(wreck).expect("the wreck").limits.wreck = true;
    arena
        .unit_mut(piloted)
        .expect("the piloted hull")
        .controller = Controller::Pilot;
    let tape = vec![
        // Another issuer and another deadline, superseded by the line after.
        tape_line(-5_000, Some(red.units[3]), "red-1", none_issuer, 50_000),
        tape_line(-2_000, Some(red.commander), "red-1", unordered, 10_000),
        tape_line(0, Some(blue.commander), "blue-1", wreck, 30_000),
        tape_line(0, None, "red-1", none_issuer, 90_000),
    ];
    let grouped = live_orders_by_issuer(&arena, &tape, Wrecks::Counted);
    let expected: BTreeMap<Option<ObjId>, std::collections::BTreeSet<String>> = [
        (Some(red.commander), ["red-1".to_string()].into()),
        (Some(blue.commander), ["blue-1".to_string()].into()),
    ]
    .into();
    assert_eq!(
        grouped, expected,
        "the `None` issuer filed under red-1's commander off the hull's last line, the wreck's \
         live order counted under blue-1's, nothing else joined"
    );
    // The same join with the wrecks skipped: red-1's group alone, the wreck's
    // order never read.
    assert_eq!(
        live_orders_by_issuer(&arena, &tape, Wrecks::Skipped),
        [(Some(red.commander), ["red-1".to_string()].into())].into(),
        "with the wrecks skipped blue-1's group, the wreck's alone, is gone"
    );
    // blue-1's group is the wreck's order alone: the same hull flown by a
    // pilot is never read, and the group goes with it.
    arena.unit_mut(wreck).expect("the wreck").controller = Controller::Pilot;
    let without = live_orders_by_issuer(&arena, &tape, Wrecks::Counted);
    assert_eq!(
        without.get(&Some(blue.commander)),
        None,
        "blue-1's group was the wreck's order alone"
    );
}

/// **The join refuses a live order with no line** (17b cut M4.5's review):
/// a mind holds a live order and the tape names only another hull — the span
/// can never be read off nothing.
#[test]
#[should_panic(expected = "and no `ordered` line on the tape names it")]
fn m4_5_the_span_s_join_refuses_a_live_order_with_no_line() {
    use tacsim::arena::state::Team;
    let mut arena = lance_field_arena(8);
    let red = arena
        .squads
        .iter()
        .find(|roster| roster.id.team == Team::Red)
        .cloned()
        .expect("one lance a side");
    arena.minds.get_mut(&red.units[1]).expect("a mind").order =
        Some(standing_order("red-1", 90_000));
    let tape = vec![tape_line(
        0,
        Some(red.commander),
        "red-1",
        red.units[2],
        90_000,
    )];
    let _ = live_orders_by_issuer(&arena, &tape, Wrecks::Counted);
}

/// **The join refuses a line that is not the order's** (17b cut M4.5's
/// review): the hull's last line names its squad but its millisecond plus its
/// time to live is not the order's deadline — the span can never be read off
/// a line that is not the order's.
#[test]
#[should_panic(expected = "is not its last `ordered` line's")]
fn m4_5_the_span_s_join_refuses_a_line_that_is_not_the_order_s() {
    use tacsim::arena::state::Team;
    let mut arena = lance_field_arena(8);
    let red = arena
        .squads
        .iter()
        .find(|roster| roster.id.team == Team::Red)
        .cloned()
        .expect("one lance a side");
    arena.minds.get_mut(&red.units[1]).expect("a mind").order =
        Some(standing_order("red-1", 90_000));
    let tape = vec![tape_line(
        0,
        Some(red.commander),
        "red-1",
        red.units[1],
        60_000,
    )];
    let _ = live_orders_by_issuer(&arena, &tape, Wrecks::Counted);
}

/// **The join refuses a line of another squad** (17b cut M4.5's review): the
/// hull's last line carries the order's deadline but names another squad —
/// the span can never file an order under a squad its line does not name.
#[test]
#[should_panic(expected = "is not its last `ordered` line's")]
fn m4_5_the_span_s_join_refuses_a_line_of_another_squad() {
    use tacsim::arena::state::Team;
    let mut arena = lance_field_arena(8);
    let red = arena
        .squads
        .iter()
        .find(|roster| roster.id.team == Team::Red)
        .cloned()
        .expect("one lance a side");
    arena.minds.get_mut(&red.units[1]).expect("a mind").order =
        Some(standing_order("red-1", 90_000));
    let tape = vec![tape_line(
        0,
        Some(red.commander),
        "red-2",
        red.units[1],
        90_000,
    )];
    let _ = live_orders_by_issuer(&arena, &tape, Wrecks::Counted);
}

/// What [`measure_lance`] answers, for the smoke and the lane.
struct LanceReport {
    /// Every `harness_lance_*` line, as printed.
    lines: Vec<String>,
    /// The JSON's path.
    json: std::path::PathBuf,
    /// Arena time at the end of the warm-up: the opening round's millisecond.
    start_ms: i64,
    /// The squads each side was dealt.
    squads_per_side: usize,
    /// Red's commander and blue's.
    commanders: [ObjId; 2],
    /// Every `ordered` line on the tape.
    ordered: Vec<OrderedLine>,
}

/// **Unit `n` of the lance field** (16b cut Z.1's muster; lifted out of
/// [`measure_lance`] by 17b cut M4.5 so [`lance_field_arena`] fields the
/// same hull): `ObjId(100_000 + n)`, red for an even `n` and blue for an odd,
/// the CI cohort's entry (`COHORT[(n / 2) % 8]`) built at the authored reach,
/// mustered on [`LANCE_RED_COLUMN`] heading east or [`LANCE_BLUE_COLUMN`]
/// heading west in the fan-out lane's row alternation, a unit mind at the ROM
/// ([`rom_terms`]), no pilot and no seat.
fn lance_unit(catalog: &Catalog, n: u32) -> tacsim::arena::state::CustodyUnit {
    use tacsim::arena::state::{Controller, Team};
    let id = ObjId(100_000 + n);
    let team = if n.is_multiple_of(2) {
        Team::Red
    } else {
        Team::Blue
    };
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
    // The fan-out lane's alternation: slot one on row 97, then two rows
    // either side of it — 95, 99, 93, 101, … — a clear row between each pair.
    let slot = i64::from(n / 2);
    let row = LANCE_ROW + if slot % 2 == 1 { -1 } else { 1 } * 2 * ((slot + 1) / 2);
    let (column, heading_mdeg) = if team == Team::Red {
        (LANCE_RED_COLUMN, 90_000)
    } else {
        (LANCE_BLUE_COLUMN, 270_000)
    };
    let pose = mush_sim::data::Pose {
        x_cm: i64::from(column) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        y_cm: row * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        // `Arena::new` sets z off the cell (16b cut X.6).
        z_cm: 0,
        heading_mdeg,
        posture: mush_sim::data::passive_rules_value(serde_json::json!({}))
            .expect("an empty posture"),
    };
    tacsim::match_::custody_unit(
        catalog,
        cells,
        id,
        team,
        pose,
        1,
        Controller::Mind,
        0,
        &rom_terms(),
    )
    .expect("the ROM's terms seal")
}

/// **The lance field, opened and never stepped** (17b cut M4.5's review): the
/// shipped proving ground and `units` hulls of [`lance_unit`] dealt into
/// lances of four, at arena millisecond nought — the lance smoke's field
/// without its run, for a test that needs a real arena's squads and mind rows
/// and nothing the arena does. The production roster is booted once per test
/// binary (its fielded catalog and the ground kept), so a call costs the
/// arena's open alone.
fn lance_field_arena(units: u32) -> tacsim::arena::state::Arena {
    use tacsim::arena::state::{Arena, CustodyOut};
    static BOOTED: std::sync::OnceLock<(Catalog, tacsim::arena::state::ArenaDefinition)> =
        std::sync::OnceLock::new();
    let (catalog, definition) = BOOTED.get_or_init(|| {
        let tw = TestWorld::with_modules(fan_out_roster());
        let catalog = fielded_catalog(&tw);
        let definition = tacsim::match_::definition_for(
            tw.world(),
            &smol_str::SmolStr::new(tacsim::match_::PROVING_GROUND),
        )
        .expect("the proving ground's ground builds")
        .expect("the proving ground is defined");
        (catalog, definition)
    });
    Arena::new(
        CustodyOut {
            definition: definition.clone(),
            units: (0..units).map(|n| lance_unit(catalog, n)).collect(),
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(catalog),
            seed: DEFAULT_SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &tacsim::config::ArenaConfig {
            squad_size: 4,
            ..tacsim::config::ArenaConfig::default()
        },
        &tacsim::config::MatchConfig::default(),
        &tacsim::config::SensorsConfig::default(),
    )
    .expect("the lance field opens on the proving ground")
}

/// **The lance measurement** (16b cut Z.1; 16c N6).
///
/// 1. The production roster ([`fan_out_roster`]) with the boot packs on, the
///    fielded catalog off it and the shipped proving ground through
///    `tacsim::match_::definition_for` — the `ArenaDefinition` the lobby opens.
/// 2. `lance.units` units of [`lance_unit`]: built through
///    `tacsim::match_::custody_unit` by the CI cohort's rule
///    (`COHORT[(n / 2) % 8]`, team `n % 2`) at the authored reach, every one
///    a unit mind at the ROM (17b cut M4.5, [`rom_terms`]; the reflex
///    controller before it), no pilot and no seat, mustered on
///    [`LANCE_RED_COLUMN`] and [`LANCE_BLUE_COLUMN`].
/// 3. The arena at the default `[tacsim.arena]` (`squad_size` four), the tempo
///    pinned `Engaged`, warmed up as the relief lanes are ([`warm_up`]).
/// 4. [`run_wakes`] for `lance.arena_s` arena seconds with the script handing
///    each wake's phase 2 the orders due at it.
/// 5. The lines, the JSON, and — under `TACSIM_ASSERT` — W8.
#[allow(clippy::too_many_lines)]
fn measure_lance(knobs: &Knobs, lance: &LanceKnobs) -> LanceReport {
    use tacsim::arena::clock::Tempo as ArenaTempo;
    use tacsim::arena::state::{Arena, CustodyOut, Power, Team};
    use tacsim::config::{ArenaConfig, MatchConfig};

    assert!(
        lance.units >= 2 && lance.units.is_multiple_of(2),
        "TACSIM_LANCE_UNITS is even and at least two: {}",
        lance.units
    );
    let per_side = lance.units / 2;

    // ---- (1) the world, the catalog and the ground ----
    let boot_at = Instant::now();
    let tw = TestWorld::with_modules(fan_out_roster());
    let boot_ms = boot_at.elapsed().as_secs_f64() * 1_000.0;
    let catalog = fielded_catalog(&tw);
    let mut definition = tacsim::match_::definition_for(
        tw.world(),
        &smol_str::SmolStr::new(tacsim::match_::PROVING_GROUND),
    )
    .expect("the proving ground's ground builds")
    .expect("the proving ground is defined");
    drop(tw);
    // **16b cut O.1**: the Hold lane fights on the shipped ground's own rule —
    // The Crossroads, which the musters stand 27 cells either side of — with
    // `hold_ms` raised to `MAX_HOLD_MS` on this copy of the definition, so no win
    // ends the window and the figure is the whole run's goal cost.
    if lance.mode == tacsim::arena::MatchMode::Hold {
        let rule = definition
            .hold_rule()
            .expect("the shipped proving ground declares a Hold");
        definition.modes = vec![tacsim::arena::ArenaMode::Hold(tacsim::arena::HoldRule {
            hold_ms: tacsim::definitions::arena::MAX_HOLD_MS,
            ..rule
        })];
    }

    // ---- (2) the units ----
    let units: Vec<_> = (0..lance.units).map(|n| lance_unit(&catalog, n)).collect();

    // ---- (3) the arena ----
    let arena_cfg = ArenaConfig {
        squad_size: 4,
        ..ArenaConfig::default()
    };
    let match_cfg = MatchConfig::default();
    let sensors_cfg = tacsim::config::SensorsConfig::default();
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: knobs.seed,
            mode: lance.mode,
        },
        &arena_cfg,
        &match_cfg,
        &sensors_cfg,
    )
    .expect("the lance arena opens on the proving ground");
    arena.clock.pin_tempo_for_tests(ArenaTempo::Engaged);
    let substep =
        tacsim::arena::clock::substep_ms(&arena_cfg.tempo, arena.cell_cm(), arena.max_speed_cms());
    let squads = arena.squads.clone();
    let squads_on = |team: Team| {
        squads
            .iter()
            .filter(|roster| roster.id.team == team)
            .count()
    };
    let squads_per_side = squads_on(Team::Red);
    assert_eq!(
        squads_per_side,
        squads_on(Team::Blue),
        "the two sides are dealt alike"
    );
    // Each side's commander: its own lowest unit, which is its first squad's.
    let lowest = |team: Team| {
        squads
            .iter()
            .filter(|roster| roster.id.team == team)
            .map(|roster| roster.commander)
            .min()
            .expect("every side has a squad")
    };
    let commanders = [lowest(Team::Red), lowest(Team::Blue)];
    let mut stepper = Stepper {
        arena,
        substep,
        counts: Counts::default(),
        clock: WakeClock::new(knobs),
        wall: 0,
        slope_refusals: 0,
        // 16b cut AF.2: the lance lane takes no snapshot.
        snapshot_ms: None,
        last_snapshot_period: None,
    };
    // The relief lanes' warm-up arithmetic off the one source they read
    // (17b cut M4.5: [`rom_think_ms`], the ROM's think period, where the
    // 1,000 ms `decide_ms` was), so the opening round is still at 11,500 ms
    // and every mark below with it.
    let warm_at = Instant::now();
    let warm_until = i64::from(match_cfg.startup_ms)
        .saturating_add(rom_think_ms())
        .saturating_add(i64::from(arena_cfg.tempo.engaged_ms));
    let warm_pulses = warm_up(&mut stepper, warm_until);
    let warm_ms = warm_at.elapsed().as_secs_f64() * 1_000.0;
    let cold: Vec<(ObjId, Power)> = stepper
        .arena
        .units
        .iter()
        .filter(|unit| unit.power != Power::Online && !unit.limits.wreck)
        .map(|unit| (unit.id, unit.power))
        .collect();
    assert!(
        cold.is_empty(),
        "{} units never came online on the proving ground: {cold:?}",
        cold.len()
    );
    let killed_warming = stepper
        .arena
        .units
        .iter()
        .filter(|unit| unit.limits.wreck)
        .count();
    let start_ms = stepper.arena.now_ms;
    let until_ms = start_ms.saturating_add(i64::from(lance.arena_s) * 1_000);
    // Each squad's units still standing when the opening round is given: what
    // that round must land on, one `ordered` line a unit.
    let standing_at_start: Vec<usize> = squads
        .iter()
        .map(|roster| {
            roster
                .units
                .iter()
                .filter(|unit| {
                    stepper
                        .arena
                        .unit(**unit)
                        .is_some_and(|state| !state.limits.wreck)
                })
                .count()
        })
        .collect();
    // The figures the equalities below were set from hold at the lane's own
    // defaults, where the run is one deterministic fight; at another seed, length,
    // size or jitter a lance may be wiped out, reach its crossroads, or take a mark
    // a wake late, and the span and the busiest minute are then whatever that
    // fight made them (bounded by W8 and by the script, never equal by rule).
    let at_defaults = knobs.seed == DEFAULT_SEED
        && knobs.jitter_ms == 0
        && lance.units == DEFAULT_LANCE_UNITS
        && lance.arena_s == DEFAULT_LANCE_ARENA_S;

    // ---- (4) the run, with the script ----
    let schedule = lance_schedule(&squads, start_ms, until_ms);
    let mut next = 0_usize;
    let mut seq = 0_u64;
    // How late, in arena milliseconds, the latest scripted order landed after
    // its mark: nought at jitter nought, where every mark is a wake.
    let mut late_max_ms = 0_i64;
    // **What the script gave, as the tape should carry it**: one line for every
    // unit an order went to that was not already a wreck, at the millisecond
    // phase 2 applied it (`apply_intent` journals every order to a held,
    // unwrecked machine at its current epoch — a mind's as the reflex's was —
    // and refuses a wreck before journalling). This is the script's side of the ledger, so the ledger's
    // own tape can be held to it at any knob.
    let mut given: Vec<OrderedLine> = Vec::new();
    // **Each scripted wake as the arena held it** (16b cut AE.1; 16c B-58): the
    // wake's arena millisecond, the squad, whether it was an order (`true`) or
    // the release (`false`), and how many of the squad's machines were standing
    // — not wrecks — when phase 2 was handed it. Read off the arena the lane
    // holds, never off the script: the at-defaults checks below require a line
    // only where a machine stood to take it.
    let mut marks: Vec<(i64, tacsim::deliberation::SquadId, bool, usize)> = Vec::new();
    let run = run_wakes(
        &mut stepper,
        until_ms,
        |arena| {
            let mut intents = Vec::new();
            while let Some(order) = schedule.get(next)
                && order.at_ms <= arena.now_ms
            {
                let issuer = if order.squad.team == Team::Red {
                    commanders[0]
                } else {
                    commanders[1]
                };
                let sent = lance_intents(arena, order, issuer, &mut seq);
                marks.push((
                    arena.now_ms,
                    order.squad,
                    order.objective.is_some(),
                    sent.iter()
                        .filter(|intent| {
                            arena
                                .unit(intent.unit)
                                .is_some_and(|state| !state.limits.wreck)
                        })
                        .count(),
                ));
                // 16b cut Z.2: a release journals `order_released`, never an
                // `ordered` line, so the script's side of the tape skips it.
                if order.objective.is_some() {
                    given.extend(
                        sent.iter()
                            .filter(|intent| {
                                arena
                                    .unit(intent.unit)
                                    .is_some_and(|state| !state.limits.wreck)
                            })
                            .map(|intent| OrderedLine {
                                t_ms: arena.now_ms,
                                issuer: Some(issuer),
                                squad: order.squad.to_string(),
                                unit: intent.unit,
                                ttl_ms: LANCE_ORDER_MS,
                            }),
                    );
                }
                intents.extend(sent);
                late_max_ms = late_max_ms.max(arena.now_ms - order.at_ms);
                next += 1;
            }
            intents
        },
        |_| {},
    );
    let Stepper { arena, counts, .. } = stepper;

    // ---- (5) the report ----
    // 16b cut O.1: the Hold lane's lines carry their own cohort word, so they
    // are never read as the lance lane's.
    let cohort = if lance.mode == tacsim::arena::MatchMode::Hold {
        "lance_hold"
    } else {
        "lance"
    };
    let tempo = Tempo {
        name: "lance_engaged",
        period_ms: arena_cfg.tempo.engaged_ms,
        ms: run.wakes.iter().map(|wake| wake.ms).collect(),
    };
    let (p50, p95, p99, max) = tempo.stats();
    let folded = windows(&run.wakes, start_ms);
    let (_, _, w99, wmax) = Tempo {
        name: "lance_window",
        period_ms: u32::try_from(WINDOW_MS).unwrap_or(0),
        ms: folded.iter().map(|w| w.ms).collect(),
    }
    .stats();
    let window_peak = |pick: fn(&WindowSum) -> u64| folded.iter().map(pick).max().unwrap_or(0);
    let traces_peak = window_peak(|w| w.traces);
    let cell_probes_peak = window_peak(|w| w.cell_probes);
    let horizon_probes_peak = window_peak(|w| w.horizon_probes);
    let jam_checks_peak = window_peak(|w| w.jam_checks);
    let share_ops_peak = window_peak(|w| w.share_ops);
    let command_cfg = tacsim::config::CommandConfig::default();
    let command = CommandFigures::of(&arena, &command_cfg, counts.ordered_lines, &counts.ordered);
    let (supporters, supporters_on_target, supporters_charge_unheld) = supporters_on_charge(&arena);
    let mut command_json = command.json();
    if let Some(object) = command_json.as_object_mut() {
        object.insert("released".to_string(), serde_json::json!(counts.released));
        object.insert("supporters".to_string(), serde_json::json!(supporters));
        object.insert(
            "supporters_on_charges_target".to_string(),
            serde_json::json!(supporters_on_target),
        );
        object.insert(
            "supporters_charge_unheld".to_string(),
            serde_json::json!(supporters_charge_unheld),
        );
    }
    let states = machine_states(&arena);
    let perception = &run.perception_total;
    let asserting = knobs.assert;
    let window_lens = window_lens_ms(Reach::Authored);

    let mut lines = vec![
        format!(
            "harness_lance_field: cohort={cohort} units={} per_side={per_side} \
             squads_per_side={squads_per_side} squad_size={} red_column={LANCE_RED_COLUMN} \
             blue_column={LANCE_BLUE_COLUMN} killed_warming={killed_warming} \
             warm_pulses={warm_pulses} warm_ms={warm_ms:.1} boot_ms={boot_ms:.1} \
             substep_ms={substep} arena_ms={} ended={} script_orders={} \
             script_late_max_ms={late_max_ms} at_defaults={at_defaults}",
            lance.units,
            arena_cfg.squad_size,
            arena.now_ms,
            run.ended
                .map_or("none".to_string(), |end| format!("{:?}", end.reason)),
            next
        ),
        // 16b cut Z.2: the releases, counted as the `ordered` lines are, and a
        // report figure — how many supporters' rows targeted their charge's
        // target on the last wake — beside them.
        format!(
            "{} released={} supporters={supporters} \
             supporters_on_charges_target={supporters_on_target} \
             supporters_charge_unheld={supporters_charge_unheld}",
            command.line("harness_lance_command", cohort),
            counts.released
        ),
        format!(
            "harness_lance_wakes: cohort={cohort} wakes={} p50={p50:.3} p95={p95:.3} \
             p99={p99:.3} max={max:.3} beside_ms={ARENA_S_P99_MS} (report-only)",
            run.wakes.len()
        ),
        format!(
            "harness_lance_window: cohort={cohort} window_ms={WINDOW_MS} windows={} \
             ms_p99={w99:.3} ms_max={wmax:.3} traces_peak={traces_peak} \
             cell_probes_peak={cell_probes_peak} horizon_probes_peak={horizon_probes_peak} \
             jam_checks_peak={jam_checks_peak} share_ops_peak={share_ops_peak} \
             beside_ms={window_lens} (report-only)",
            folded.len()
        ),
        format!(
            "harness_lance_minds_states: cohort={cohort} {}",
            machine_states_words(&states)
        ),
        format!(
            "harness_lance_weapons: cohort={cohort} fired={} impacts={} sections_destroyed={} \
             units_destroyed={}",
            counts.fired, counts.impacts, counts.sections, counts.destroyed
        ),
        // **16b cut O.1**: what the machines' goals cost — the figure the Hold
        // lane exists to report beside the lance lane's own (report-only on
        // both). 17b cut M4.5: the decisions are the minds' thinks, where the
        // reflex's ticks were.
        format!(
            "harness_lance_goal_cost: cohort={cohort} thinks={} landings={} path_queries={} \
             path_deferred={} path_cells={} path_failures={} peak_path_cells={} \
             path_refused_unchanged={} corner_refusals={} (report-only)",
            run.motor_total.thinks,
            run.motor_total.landings,
            run.motor_total.path_queries,
            run.motor_total.path_deferred,
            run.motor_total.path_cells,
            run.motor_total.path_failures,
            run.motor_peak.path_cells,
            run.motor_total.path_refused_unchanged,
            run.motor_total.corner_refusals
        ),
        format!(
            "harness_lance_perception_total: cohort={cohort} passes={} candidates={} traces={} \
             cell_probes={} horizon_probes={} jam_checks={} share_ops={} detected={} \
             classified={} lost={}",
            perception.passes,
            perception.candidates,
            perception.traces,
            perception.cell_probes,
            perception.horizon_probes,
            perception.jam_checks,
            perception.share_ops,
            perception.detected,
            perception.classified,
            perception.lost
        ),
        format!(
            "harness_lance_thresholds: cohort={cohort} assert={asserting} lens=authored \
             cut_a_p99_ms={ARENA_P99_MS} cut_s_p99_ms={ARENA_S_P99_MS} \
             window_p99_ms={window_lens} (report-only for timing) engaged_p99_ms={} \
             max_traces_per_pulse={} max_share_ops_per_pulse={} max_path_queries_per_pulse={} \
             max_path_cells_per_pulse={} max_cell_probes_per_pulse={} max_jam_checks_per_pulse={} \
             max_ledger_bytes={}",
            knobs.engaged_p99_ms,
            knobs.max_traces_per_pulse,
            knobs.max_share_ops_per_pulse,
            knobs.max_path_queries_per_pulse,
            knobs.max_path_cells_per_pulse,
            knobs.max_cell_probes_per_pulse,
            knobs.max_jam_checks_per_pulse,
            knobs.max_ledger_bytes
        ),
    ];

    let report = serde_json::json!({
        "what": "16b cut Z.1 (at the ROM since 17b cut M4.5): twelve minds a side in lances of four on the proving ground, a scripted commander a side",
        "cohort": cohort,
        "field": {
            "units": lance.units,
            "per_side": per_side,
            "squads_per_side": squads_per_side,
            "squad_size": arena_cfg.squad_size,
            "red_column": LANCE_RED_COLUMN,
            "blue_column": LANCE_BLUE_COLUMN,
            "killed_warming": killed_warming,
            "warm_pulses": warm_pulses,
            "substep_ms": substep,
            "arena_ms": arena.now_ms,
            "seed": knobs.seed,
            "script_orders": next,
            "script_late_max_ms": late_max_ms,
            "at_defaults": at_defaults,
        },
        "command": command_json,
        "wakes": {
            "wakes": run.wakes.len(),
            "p50_ms": num(p50),
            "p95_ms": num(p95),
            "p99_ms": num(p99),
            "max_ms": num(max),
        },
        "window": {
            "window_ms": WINDOW_MS,
            "windows": folded.len(),
            "ms_p99": num(w99),
            "ms_max": num(wmax),
            "traces_peak": traces_peak,
            "cell_probes_peak": cell_probes_peak,
            "horizon_probes_peak": horizon_probes_peak,
            "jam_checks_peak": jam_checks_peak,
            "share_ops_peak": share_ops_peak,
        },
        // 17b cut M4.5: the machines' states at the end (`reflex_states` was
        // the reflex's rows), as the arena report carries them.
        "minds": {
            "states_at_end": machine_states_json(&states),
        },
        "weapons": {
            "fired": counts.fired,
            "impacts": counts.impacts,
            "sections_destroyed": counts.sections,
            "units_destroyed": counts.destroyed,
        },
        "perception_total": {
            "passes": perception.passes,
            "candidates": perception.candidates,
            "traces": perception.traces,
            "cell_probes": perception.cell_probes,
            "horizon_probes": perception.horizon_probes,
            "jam_checks": perception.jam_checks,
            "share_ops": perception.share_ops,
            "detected": perception.detected,
            "classified": perception.classified,
            "lost": perception.lost,
        },
        "thresholds": {
            "assert": asserting,
            "lens": "authored",
            "timing": "report-only",
            "cut_a_p99_ms": ARENA_P99_MS,
            "cut_s_p99_ms": ARENA_S_P99_MS,
            "window_p99_ms": window_lens,
            "engaged_p99_ms": knobs.engaged_p99_ms,
            "max_traces_per_pulse": knobs.max_traces_per_pulse,
            "max_share_ops_per_pulse": knobs.max_share_ops_per_pulse,
            "max_path_queries_per_pulse": knobs.max_path_queries_per_pulse,
            // 16b cut AI.1 (16c `B-84`): report-only on this lane.
            "max_path_cells_per_pulse": knobs.max_path_cells_per_pulse,
            "max_cell_probes_per_pulse": knobs.max_cell_probes_per_pulse,
            "max_jam_checks_per_pulse": knobs.max_jam_checks_per_pulse,
            "max_ledger_bytes": knobs.max_ledger_bytes,
        },
    });
    let path = arena_report_path(&knobs.json, cohort);
    write_report(&path, &report);
    lines.push(format!(
        "harness_lance_json: cohort={cohort} {}",
        path.display()
    ));
    for line in &lines {
        println!("{line}");
    }

    if asserting {
        // **W8** (doc 16 §2): both bounds of `[tacsim.command]`, as the arena
        // lanes assert them — at any knob.
        command.assert_bounded();
        // **The tape is the script**, at any knob: every order the script gave
        // landed once on every unit of its squad that was not already a wreck,
        // at the millisecond phase 2 applied it, under its side's commander,
        // and nothing else was ordered. W8's two tape figures are read off this
        // tape, so they are the script's own.
        let sorted = |lines: &[OrderedLine]| {
            let mut keyed: Vec<(i64, String, Option<ObjId>)> = lines
                .iter()
                .map(|line| (line.t_ms, line.squad.clone(), line.issuer))
                .collect();
            keyed.sort();
            keyed
        };
        assert_eq!(
            sorted(&counts.ordered),
            sorted(&given),
            "W8: the `ordered` lines on the tape are not the orders the script gave"
        );
        // **The opening round**, at any knob: at the warm-up's end every squad
        // was ordered, on every unit still standing, under its side's commander
        // (`orders_on_tape` files a side's whole opening round as one order, so
        // the order count cannot see it; the lines can).
        for (roster, standing) in squads.iter().zip(&standing_at_start) {
            let commander = if roster.id.team == Team::Red {
                commanders[0]
            } else {
                commanders[1]
            };
            let word = roster.id.to_string();
            let landed = counts
                .ordered
                .iter()
                .filter(|line| {
                    line.t_ms == start_ms && line.squad == word && line.issuer == Some(commander)
                })
                .count();
            assert_eq!(
                landed, *standing,
                "W8: the opening round landed on {landed} of {word}'s {standing} standing units"
            );
        }
        assert!(
            standing_at_start.iter().any(|standing| *standing > 0),
            "W8: no squad had a unit standing for the opening round"
        );
        // **The span never reads a squad the script is not holding**, at any
        // knob: an order can be ended early (a wreck, an `Advance` that
        // arrived) but never lengthened, so a commander's span is at most the
        // squads it gave an order within `LANCE_ORDER_MS` of the end.
        let mut live_given: BTreeMap<Option<ObjId>, std::collections::BTreeSet<String>> =
            BTreeMap::new();
        for line in &given {
            if line.t_ms.saturating_add(i64::from(LANCE_ORDER_MS)) > arena.now_ms {
                live_given
                    .entry(line.issuer)
                    .or_default()
                    .insert(line.squad.clone());
            }
        }
        let span_bound = live_given
            .values()
            .map(std::collections::BTreeSet::len)
            .max()
            .unwrap_or(0);
        assert!(
            command.span_max <= span_bound,
            "W8: the span read {} where the script holds at most {span_bound} squads under \
             one commander",
            command.span_max
        );
        // **At the lane's defaults, the exact figures** (the brief's equalities,
        // set from the defaults' own fight): the span, the busiest commander
        // minute, each side's release of its first squad and the orders on the
        // tape at every scripted wake whose squad had a standing machine (16b
        // cut AE.1, below) — each held to that fight by fact pins (its end and
        // the standings at the marks), so no W8 figure passes on nothing. Off
        // the defaults the fight decides them (see `at_defaults`), and the
        // three checks above still hold.
        //
        // **The paragraphs from here to 17b cut M4.5's are the reflex's
        // history** (16b cuts Z.2, AE.1, AI.1 and AP.2): what the pins rested
        // on in each of four fights on the reflex controller, each superseded
        // by the next and all by 17b cut M4.5's fight at the ROM — the last
        // paragraph, which the pins below hold. They are the pins' record, not
        // statements about the lane today; the rules they set (Z.2's release
        // on each side's own first squad from its own commander, AE.1's
        // re-statement through the standing machines at the marks) still bind.
        //
        // **16b cut Z.2 — what the span equality rested on then, on the
        // reflex.** Each side's first squad is released at `LANCE_RELEASE_MS`
        // (90 s) and its next mark falls after the run's end, so no LIVING
        // machine of `red-1` or `blue-1` held an order when the span was read.
        // The span still read `squads_per_side` because **a wreck kept the
        // order it died under** — `Arena::damage` adopted it into `Disabled`
        // with its reflex row, `apply_intent` refused it a release
        // (`Refusal::Wreck`), and `Arena::live_orders_by_issuer` counted its
        // row — and at the defaults a machine of each first squad was wrecked
        // under its commander's order before the release. That was
        // pre-existing engine behaviour (a wreck is no longer the subject of
        // any order, doc 16 §3.3, and the release's own roster read already
        // skips wrecks), not what W8 means by a span: a cut that makes a wreck
        // drop its order moves a span that rests on a wreck to one fewer for a
        // released side, and must say so here. (At the ROM a wreck's mind row
        // keeps its order too, and the harness's span counts it until it
        // expires — [`live_orders_by_issuer`]; 17b cut M4.5's span rests on no
        // wreck: every squad it counts has a standing machine holding the
        // order, pinned below.)
        //
        // **16b cut Z.2**: and each side's release landed, **on its own first
        // squad, from its own commander, at the mark** — a count of lines alone
        // could be filled by one side's machines while the other's release
        // landed on nothing.
        //
        // **16b cut AE.1 (16c B-58) — the release and the orders, re-stated
        // through the wrecks, as the span already was.** Z.2 set these checks
        // when every scripted squad was alive at every mark. Since every machine
        // pays its own row, the lane's `biped_mk1` walks the grass at the legged
        // 110 against the tracked 130 and the fight moves: at the defaults all
        // four of `blue-1`'s machines are wrecks before its minute-one mark
        // (71,500 ms) and before the release (90,000 ms). A wreck keeps the
        // order it died under and a release refuses it (the rule above), so no
        // `order_released` line is written for `blue-1` and its minute-one order
        // lands on no machine — the tape carries seven orders, not eight. That
        // is the fight doing what the content says, not a defect. So a side's
        // release line is required only when its first squad had a standing
        // machine at the release's wake, the order count is the scripted wakes
        // at which a machine stood to take one, both read off the arena the lane
        // held (`marks`), and two fact pins below hold the defaults to the fight
        // these re-stated checks were set from — so none of them passes on
        // nothing — and the order count is pinned at seven besides, since its
        // re-stated equality follows from the tape-is-the-script check.
        //
        // **16b cut AI.1 (16c B-84) — the span at the defaults is
        // `squads_per_side - 1`, re-pinned from the fight.** The steer that stops
        // the machines circling moves every track, and with it who is lost when.
        // At the defaults `red-1`'s one loss before the release is its
        // commander's own hull, #100000, and it is **disabled** (in `Withdraw`,
        // at 76 s) five seconds before it is wrecked (81 s): the `Disabled` edge
        // at the top of `decide` resets the row of a hull that is disabled and
        // not yet a wreck, order and all, so the wreck it becomes holds none —
        // only `Arena::damage`'s adoption of a unit wrecked outright keeps the
        // order it died under. Its three other machines stand at 90,000 ms and
        // are released (three `order_released` lines where HEAD's fight wrote
        // two), so no row of `red-1` holds an order when the span is read at
        // 119,000 ms. `blue-1`'s wrecks keep the opening order, which expired
        // at 101,500 ms, as it did before this cut. So each commander holds its
        // second and third squads: the span reads 2, which is the shape the
        // paragraph above names for a released side. Before this cut it read
        // 3 (`red-1`'s wrecks kept its 71,500 ms order). The four fact pins
        // below did not move: `blue-1` stood none at 71,500 and at the release,
        // `red-1` stood three at the release, and the tape carries seven orders.
        //
        // **16b cut AP.2 (16c B-87, Q57) — the fight ends at 111,250 ms, a
        // quarter-second before minute one's last marks, re-pinned from the
        // fight.** A side that came in armed and has no gun left is now out
        // (`Arena::eliminated`). At the defaults blue's last machine in action,
        // #100019, has had its weapons shot off and still drives; at 111,250 ms
        // it is blue's only unit not a wreck, so blue is disarmed and the match
        // ends `Elimination { winner: Some(Red) }` there — where before this cut
        // it drove on until it was wrecked at 119,000 ms. Minute one's third
        // marks (`red-3` and `blue-3`, 111,500 ms) fall after the end and are
        // never given. So at the read each commander holds a live order on its
        // second squad alone (`red-2` and `blue-2`, ordered at 91,500 ms): the
        // first squads hold none (the AI.1 paragraph above), and the third
        // squads' opening orders (11,500 ms) expired at 101,500 ms — the span
        // reads `squads_per_side - 2`, 1, where AI.1's fight read 2. Red's busiest
        // sixty seconds hold its 71,500 and 91,500 ms marks — `squads_per_side -
        // 1`, 2, where the script's full minute gives 3 — and the tape carries
        // five orders, not seven: the opening round a side, red's 71,500 and
        // 91,500 ms marks and blue's 91,500 ms one. The release and the two AE.1
        // fact pins did not move (`blue-1` stood none at 71,500 and at the
        // release, `red-1` three at the release), and a third fact pin below
        // holds the defaults to this end, so the three re-pinned figures pass on
        // a fact. That is the clause doing what doc 16 §3.8 now says, not a
        // defect: the lane's knobs and script are untouched; only the figures
        // its fight gives at the defaults moved.
        //
        // **17b cut M4.5 — the lane at the ROM, re-pinned from its fight**
        // (run under `TACSIM_ASSERT=1` at the defaults, after 17b cut M4.5c's
        // row cut). Every hull is a unit mind at the ROM (`rom_terms`, Class 0:
        // eight contact rows, a think every 1,000 ms). Since 17b cut M4.5c a
        // mind's view seats its lock and its order's contact and then keeps
        // every contact not known friendly before its friends (doc 17 §3 as
        // amended 2026-10-09), so a hull mustered among eleven nearer friends
        // reads the enemy; before that cut the ROM's eight rows were all its
        // own lance, no mind left `Patrol` and nothing fired in the 120
        // seconds (STOP L, which the owner's two rulings resolved). **The
        // fight at the defaults**: the first shot at 22,691 ms; 404 shots (54
        // a mind's cited `Fire`, 350 a slot's own); twelve hulls wrecked —
        // red five, blue seven — and neither side out, so **no end inside the
        // window**: the run reaches 131,500 ms (the reflex's fight ended at
        // 111,250 ms, red's elimination win: 16b cut AP.2's fact pin). Before
        // the minute-one mark (71,500 ms) `red-1` lost its commander's hull
        // #100000 (disabled at 55,638 ms and wrecked at 60,903, keeping its
        // opening order on its mind row — where the reflex's `Disabled` edge
        // cleared a disabled hull's, 16b cut AI.1) and `blue-1` lost #100005
        // and #100003 (53,939 and 71,132 ms): **`red-1` stood three at its
        // minute-one mark and at the release, `blue-1` two** (16b cut AE.1's
        // fact pins were the reflex's fight: `blue-1` none at both, `red-1` at
        // least one at the release). So both sides' releases land — five
        // `order_released` lines, red's three and blue's two, where the
        // reflex's fight wrote red's three alone — and every scripted wake has
        // a standing machine to take it. **The span reads `squads_per_side -
        // 1`, 2, for each commander**: its first squad holds no live order at
        // the read (the standing machines released at 90,000 ms, the wrecks'
        // opening orders expired at 101,500), and its second and third squads
        // hold their 91,500 and 111,500 ms orders on standing machines (16b
        // cut AP.2's span was `squads_per_side - 2`, 1). **The busiest minute
        // is the script's full `squads_per_side`, 3** (71,500, 91,500 and
        // 111,500 ms inside one minute; AP.2's `squads_per_side - 1`, 2, its
        // end falling before the third marks), and **the tape carries the
        // opening round and one full minute a side, `2 * (1 +
        // squads_per_side)`, eight orders** (AP.2's five). Each figure sits
        // inside W8's bounds — the span 2 ≤ `[tacsim.command].max_span` 4,
        // the busiest minute 3 ≤ `orders_per_minute` 12 — and the script's (at
        // most `squads_per_side`, 3, under one commander, in a live order and
        // in any minute), and each is held by fact pins below (the end, the
        // four standings, the release count read back through them), so none
        // passes on nothing. No `Support` is given in the 120 seconds (the
        // table's fifth arm is minute five's), so the report's `supporters`
        // and `supporters_charge_unheld` read nought at the defaults.
        if at_defaults {
            // **The fact pin of 16b cut AP.2, re-pinned by 17b cut M4.5**
            // (at 4bbcbdc, the reflex's fight: `(Some(Elimination { winner:
            // Some(Red) }), 111_250)`): at the ROM the defaults' fight has no
            // end inside the window — red five hulls down, blue seven, neither
            // side out — and runs to 131,500 ms, the warm-up's 11,500 plus 120
            // seconds.
            assert_eq!(
                (run.ended.as_ref().map(|end| end.reason), arena.now_ms),
                (None, 131_500),
                "17b cut M4.5: at the defaults the ROM's fight has no end inside the window \
                 (red five hulls down, blue seven) and the match runs to 131,500 ms"
            );
            // 17b cut M4.5: `squads_per_side - 1` (16b cut AP.2's
            // `squads_per_side - 2`, re-pinned; see above).
            assert_eq!(
                command.span_max,
                squads_per_side - 1,
                "W8: the span read {} where each commander holds live orders on its second and \
                 third squads alone at the read (its first squad's standing machines released, \
                 their wrecks' opening orders expired; 17b cut M4.5, see above)",
                command.span_max
            );
            // **The span rests on no wreck** (17b cut M4.5's review): the same
            // join over the live orders standing hulls hold, the wrecks
            // skipped, reads the same span — every squad it counts has a
            // standing machine holding the order (at the defaults the one
            // wreck under a live order, `blue-2`'s #100011, stands beside
            // #100015 under the same order).
            // Held per commander, squad by squad, not only at the maximum.
            let standing = live_orders_by_issuer(&arena, &counts.ordered, Wrecks::Skipped);
            assert_eq!(
                standing,
                live_orders_by_issuer(&arena, &counts.ordered, Wrecks::Counted),
                "W8: a squad the span counts is held only by a wreck: at the defaults every \
                 commander's squads rest on a standing machine (17b cut M4.5)"
            );
            let standing_span = standing
                .values()
                .map(std::collections::BTreeSet::len)
                .max()
                .unwrap_or(0);
            assert_eq!(
                standing_span, command.span_max,
                "W8: the span read {} but {standing_span} over the live orders standing hulls \
                 hold: at the defaults the span rests on no wreck (17b cut M4.5)",
                command.span_max
            );
            // How many of the squad's machines stood at the scripted wake at
            // `t_ms` (`true` an order, `false` the release), or `None` when the
            // script handed phase 2 no such wake.
            let stood = |t_ms: i64, word: &str, order: bool| {
                marks
                    .iter()
                    .find(|(at, squad, is_order, _)| {
                        *at == t_ms && squad.to_string() == word && *is_order == order
                    })
                    .map(|(_, _, _, standing)| *standing)
            };
            // **The fact pins of 16b cut AE.1, re-pinned by 17b cut M4.5**
            // (at 4bbcbdc, the reflex's fight: `blue-1` stood none at its
            // minute-one mark and at the release, `red-1` at least one at the
            // release): at the ROM `blue-1` stands two at both and `red-1`
            // three at both — the fight the re-stated checks below were set
            // from, so each passes on a fact.
            assert_eq!(
                (
                    stood(71_500, "blue-1", true),
                    stood(LANCE_RELEASE_MS, "blue-1", false),
                    stood(71_500, "red-1", true),
                    stood(LANCE_RELEASE_MS, "red-1", false)
                ),
                (Some(2), Some(2), Some(3), Some(3)),
                "17b cut M4.5: at the defaults blue-1 stands two and red-1 three at 71,500 ms and \
                 at {LANCE_RELEASE_MS} ms: {marks:?}"
            );
            // The sides whose first squad had a standing machine at the release.
            let stood_at_release: Vec<(ObjId, &str)> = commanders
                .iter()
                .copied()
                .zip(["red-1", "blue-1"])
                .filter(|(_, first)| {
                    stood(LANCE_RELEASE_MS, first, false).is_some_and(|standing| standing > 0)
                })
                .collect();
            assert!(
                counts.released >= 1,
                "the release landed on the first squad of each of the {} side(s) whose first \
                 squad stood at {LANCE_RELEASE_MS} ms: {} `order_released` lines",
                stood_at_release.len(),
                counts.released
            );
            for (commander, first) in stood_at_release {
                assert!(
                    counts
                        .released_lines
                        .iter()
                        .any(|line| line.issuer == Some(commander)
                            && line.squad == first
                            && line.t_ms == LANCE_RELEASE_MS),
                    "{first}'s commander {commander} released {first} at \
                     {LANCE_RELEASE_MS} ms: {:?}",
                    counts.released_lines
                );
            }
            assert!(
                counts.released_lines.iter().all(|line| {
                    let side = usize::from(!line.squad.starts_with("red-"));
                    line.issuer == commanders.get(side).copied()
                }),
                "every release is its own side's commander's: {:?}",
                counts.released_lines
            );
            // **The release count, read back through the marks** (17b cut
            // M4.5; the reflex's fight's three lines, red's, were held by the
            // `>= 1` and the per-side checks above): one `order_released` line
            // for every machine each first squad stood at the release — five
            // at the defaults, red's three and blue's two, held by the fact
            // pins above.
            let standing_at_release: usize = ["red-1", "blue-1"]
                .into_iter()
                .filter_map(|first| stood(LANCE_RELEASE_MS, first, false))
                .sum();
            assert_eq!(
                counts.released,
                u64::try_from(standing_at_release).unwrap_or(u64::MAX),
                "17b cut M4.5: one `order_released` line for each of the {standing_at_release} \
                 machines the first squads stood at {LANCE_RELEASE_MS} ms: {:?}",
                counts.released_lines
            );
            // 17b cut M4.5: the script's full minute, `squads_per_side` (16b
            // cut AP.2's `squads_per_side - 1`, re-pinned) — the run reaches
            // every minute-one mark, where AP.2's fight ended before the third.
            assert_eq!(
                command.orders_per_commander_minute_max,
                u64::try_from(squads_per_side).unwrap_or(u64::MAX),
                "W8: the busiest commander minute read {} where the script gives \
                 {squads_per_side} and the run reaches every mark (17b cut M4.5)",
                command.orders_per_commander_minute_max
            );
            // The opening round and one minute's `squads_per_side` a side —
            // `2 * (1 + squads_per_side)` when every squad stands at every mark
            // — counted over the scripted wakes at which the ordered squad had
            // a standing machine, one order per side per wake as
            // `orders_on_tape` files them (16b cut AE.1: seven at the defaults,
            // `blue-1`'s minute-one mark landing on no machine; five since 16b
            // cut AP.2, the match ending before the third squads' marks;
            // eight since 17b cut M4.5, a standing machine at every mark).
            let stood_to_order: std::collections::BTreeSet<(bool, i64)> = marks
                .iter()
                .filter(|(_, _, is_order, standing)| *is_order && *standing > 0)
                .map(|(at, squad, _, _)| (squad.team == Team::Red, *at))
                .collect();
            assert_eq!(
                command.orders,
                u64::try_from(stood_to_order.len()).unwrap_or(u64::MAX),
                "W8: {} orders on the tape where the script gives the opening round and one \
                 minute's {squads_per_side} a side, at the {} scripted wakes whose squad had a \
                 standing machine",
                command.orders,
                stood_to_order.len()
            );
            // **The count itself, pinned** (16b cut AE.1): the equality above is
            // the tape-is-the-script check read back through the marks, so it
            // cannot fail on its own. 16b cut AE.1's fight gave seven and 16b
            // cut AP.2's five (`2 * (1 + squads_per_side) - 3` at 4bbcbdc);
            // **17b cut M4.5's fight at the ROM gives the script's whole
            // `2 * (1 + squads_per_side)`, eight** — a standing machine at
            // every mark, and every mark inside the run.
            assert_eq!(
                command.orders,
                u64::try_from(2 * (1 + squads_per_side)).unwrap_or(u64::MAX),
                "17b cut M4.5: at the defaults the tape carries the opening round and one \
                 minute's {squads_per_side} a side, not {} orders",
                command.orders
            );
        }
        // **The lances fired**, at any knob. At the ROM before 17b cut
        // M4.5c's row cut this failed at the defaults (STOP L: no hull's eight
        // rows held the enemy); since that cut the defaults' fight fires 404
        // shots, and the claim stands as it was.
        assert!(counts.fired > 0, "the lances never fired");
    }

    LanceReport {
        lines,
        json: path,
        start_ms,
        squads_per_side,
        commanders,
        ordered: counts.ordered,
    }
}

// ---------------------------------------------------------------------------
// 17b cut M4.2: the minds lane and the decision
// ---------------------------------------------------------------------------
//
// **What this is** (the M4 row's (ii); doc 17 §18, §22's M4). One more ignored
// lane on the harness's one ignore string, `tacsim_harness_arena_minds`: the
// 300-unit relief cohort fielded as `Controller::Mind` hulls in four variants,
// FILL FIRST — `fill_authored`, `stock_ci`, `stock_authored`,
// `ceiling_authored` — each stepped with `Arena::step` and a private
// [`MindPool`] of `[tacsim.minds].workers` threads on the arena task's own
// policy (the batch submitted after the publish and awaited at the next wake's
// head and before a due snapshot's encode, never right after the submit), a
// seeded quarter of its wakes one substep late so the inline path is measured
// ([`MindsClock`]), every think's ops, fees and microseconds recorded through
// recording Rhai runners and a timed native wrapper ([`TimedNative`]), five
// `harness_minds_*` lines a variant, every check collected into one
// `harness_minds_verdict` line a variant — `held=` over exactly the plan's
// listed asserts, the decision's word until 2026-10-09; `sound=` over the lane's integrity
// checks, never a decision input — and one panic at the end under
// `TACSIM_ASSERT`; and `harness-minds-<variant>.json` with a deterministic
// `minds` block the delta compares once the baseline carries one (folded by
// 17b cut M4.5's one bless). `PRODUCTION_FILL_ENGINE`
// (`tacsim::match_::PRODUCTION_FILL_ENGINE`) is what the fill variant seals
// its thirty-six Class I hulls a side on, and its verdict over three idle-host
// weekly runs was the decision — taken 2026-10-09 on the owner's three
// ALIEN18 runs, all `held=no`, so the constant is `Native`. The variant now
// measures the production fill natively (no Rhai thinks, ops and fees
// nought; the CPU lens still computed off the calibration's Rhai rate, a
// footing every variant shares), and `sound=` holds that its Class I slots
// thought on the native mind above the ROM (`native_minded_thinks`). Its
// `held=` word is printed as before and is no longer a decision input — the
// owner reads it as a capacity figure — but it is still asserted under
// `TACSIM_ASSERT` as every variant's is: on a weekly run
// (`scripts/harness-weekly.sh` exports it) a fill `held=no` fails this lane
// and so blocks that run's bless. On a native fill the two ops-pool checks
// are met trivially — no think runs in Rhai, which the ops-nought check (on
// every native-sealed variant) holds, and the native stock mind returns no
// fault — so its `held=` is driven by the timing checks alone.
//
// **The jitter is one substep, on the lattice** (a declared departure from the
// row's literal 60 ms; the M4.2 brief's review). A wake 60 ms late cuts a
// 60 ms remainder substep — motion, the index rebuild, the due loop and the
// hash over 300 hulls, none of it think time — and the 190 ms wake that
// follows, so every window such a wake lands in carries three substeps where
// the 12 ms lens was blessed on two (window p99 10.7 ms on the blessing host):
// the CI cohort would miss the lens by construction and the decision would
// read `held=no` for a cost that is not a mind's. A wake one substep late cuts
// `[substep, substep]`, two full substeps: the thinks collected at the first
// lattice point are settled INLINE at the second substep's head, inside
// `Wake.ms` (the inline path the plan wants counted), and 17b cut M4.1's gate
// holds the hash sequence equal to the unjittered run's (a lattice-multiple
// wake against one substep a pulse), so the `minds` block is one function of
// the seed. **The windows the lens is asserted on are the COMPARABLE ones**: a
// window is keyed by the wake that ENDS in it, so a late wake due at a window
// boundary keeps its window at the baseline's two substeps (both inside one
// window, the inline thinks with them), while a late wake due mid-window ends
// one substep into the next window and leaves a one-substep window behind a
// three-substep one — neither comparable with a lens blessed on two. The lens
// is asserted over the windows carrying exactly the lattice's substeps with
// no remainder (`comparable=`), the rest printed beside
// (`window_p99_other_ms`, `other=`), and a run with no comparable window does
// not hold (the lens was not measured). The knob takes any value, so doc 17
// §18's literal 60 is one env away as a sizing run.
//
// **The per-wake lens is asserted on the UNJITTERED wakes** (16c `T-108`,
// from the M4 fill-decision runs of 2026-10-09). Until then it was asserted
// over every wake, and a late wake steps two full substeps by design — its
// first lattice point's thinks settled inline inside its `Wake.ms` — and a
// quarter of the wakes at twice the work owns the 99th percentile: even the
// all-native `stock_authored` read 14.0–15.0 ms on ALIEN18 against the 12 ms
// lens and the three authored variants 25.8–28.8 on the VPS against `T-109`'s
// 20, while with the jitter off all four held on ALIEN18. The
// per-wake lens is a one-substep wake's budget; the two-substep wake is the
// window lens's to judge, and it does — a late wake due at a window boundary
// keeps both its substeps and its inline thinks in one comparable window. So
// [`WakeLens`] splits the wakes by the clock's `late` flag (its doc says why
// the flag and not the wake's shape): the lens is asserted over the on-time
// ones (`wake_p99_ms`, `wake_unjittered=` on the verdict line), the late ones'
// p99 printed beside (`wake_p99_other_ms`, `wake_jittered=`), every wake's p99
// still on the `harness_minds_engaged` line; and a run with no unjittered wake
// does not hold (the lens was not measured), the window lens's rule. At
// `TACSIM_MINDS_JITTER_PER_MILLE=0` no wake is late and the asserted figure is
// the all-wake p99 exactly. The inline path's time stays asserted: by the
// window lens over the comparable windows, and by the CPU lens over every
// window (`MindWake::cpu_us` counts the inline thinks beside the pooled
// ones); `sound=`'s `inline_thinks` check shows only that it ran.
//
// **Attribution** is by the wake that COLLECTED a think (`attribution=by-collecting-wake`):
// an inline think ran inside that wake's `Wake.ms`; a pooled think ran between
// that wake's submit and the next head's wait, whose figures are drained there
// and written back onto the collecting wake. Every stat is attributable
// because the order is fixed: head (wait, deliver, drain the pooled figures),
// step (the inline thinks run on this thread), drain the inline figures,
// submit. `pool_wait_us` here is the batch's wall latency on a two-worker pool
// with no pulse gap — the lane's wall is simulated and the next head follows
// the submit at once — an upper bound on production's blocked time (the task
// has most of a pulse of idle for the workers to finish in), report-only.

/// **`TACSIM_MINDS_JITTER_MS`'s default**: one substep of the shipped cohort
/// (250 ms), on the lattice — the section head says why not the row's 60.
const DEFAULT_MINDS_JITTER_MS: u32 = 250;

/// **`TACSIM_MINDS_JITTER_PER_MILLE`'s default**: a quarter of the wakes.
const DEFAULT_MINDS_JITTER_PER_MILLE: u32 = 250;

/// The salt the jitter draw is keyed under beside the seed (`"MINDS_JI"`), so
/// it is not [`WakeClock`]'s own sequence.
const MINDS_JITTER_SALT: u64 = 0x4d49_4e44_535f_4a49;

/// **The ceiling variant's heavies a side**: the plan's two — what the
/// 300-unit pool admits at Class III (`admitted_per_side`), and what a sizing
/// run with more room still fields, the ceiling being the load mind's cost
/// per hull and not the count. **Where they sit**: the two unseated hulls of
/// each side scattered deepest in its own rear (red the farthest west of the
/// centre column, blue the farthest east; [`MindsPlan::of`]) — never on the
/// fighting line, and as far from it as the seeded scatter put anything,
/// because `heavy_mk1`'s turret loss degrades its command core
/// (`degrade_core`), which drops the seal's class to II, where the load mind's
/// 4,4xx ops fault `ops` at the 3,000 ceiling and the variant reads `faults=`
/// for a fight outcome (this host's first run, with the heavies at side index
/// 12 and 13, lost one in thirty seconds). `authored_at_class_at_end=` on the
/// verdict line says whether a run's heavy was degraded all the same.
const MINDS_HEAVIES_PER_SIDE: u32 = 2;

/// How many wakes sample `view_us` (one `Arena::mind_view` on a clone,
/// outside the timer), evenly spaced over the measured window.
const MINDS_VIEW_SAMPLES: usize = 40;

/// **The load mind's band** (the M4 row's (ii)): its `ops + fee` per think is
/// at least this at the first percentile — the asserted figure — and at most
/// [`LOAD_III_OPS_MAX`], under the Class III row's 4,500.
const LOAD_III_OPS_P1_MIN: u64 = 4_300;

/// See [`LOAD_III_OPS_P1_MIN`].
const LOAD_III_OPS_MAX: u64 = 4_499;

/// **The Class III load mind** (`tests/support/load_iii.rhai`): test content,
/// never shipped — a legal intent set with equal cost on both branches and a
/// loop calibrated to the band; its head comment carries the arithmetic.
const LOAD_III_RHAI: &str = include_str!("support/load_iii.rhai");

/// The load mind's digest, pinned (`m4_2_the_load_mind_is_calibrated_to_the_class_iii_row`).
const LOAD_III_SHA256_HEX: &str =
    "e61a8153963f0f4d17c0eb94c3dec465ac4ad8e204e136c797ec4f1f566864f3";

/// The seal's `mind` word for a hull sealed on the load mind.
const LOAD_III_MIND: &str = "Load III";

/// **The CPU lens's headroom over the host's own rate**: five quarters (the M4
/// row's finding (7)).
const CPU_LENS_HEADROOM: f64 = 1.25;

/// How many Standard Doctrine thinks the host's rate is measured over.
const CALIBRATION_THINKS: usize = 200;

/// The load mind's bytes, digested.
fn load_iii_sha256() -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(LOAD_III_RHAI.as_bytes()).into()
}

/// **The source a seal's `source_sha256` names**, lane-local (the harness has
/// no `mod support;`): Standard Doctrine's or the load mind's.
fn source_for(source_sha256: &[u8; 32]) -> Option<&'static str> {
    if *source_sha256 == tacsim::minds::standard_digest().0 {
        Some(tacsim::minds::STANDARD_RHAI)
    } else if *source_sha256 == load_iii_sha256() {
        Some(LOAD_III_RHAI)
    } else {
        None
    }
}

/// The minds lane's own knobs.
struct MindsKnobs {
    /// `TACSIM_MINDS_JITTER_MS`: how late a jittered wake is.
    jitter_ms: i64,
    /// `TACSIM_MINDS_JITTER_PER_MILLE`: what fraction of wakes are late.
    jitter_per_mille: u32,
    /// `[tacsim.minds].workers`: the private pool's threads.
    workers: u32,
    /// `[tacsim.minds].cpu_lens_ms`: doc 17 §18's reference figure, printed
    /// beside the computed lens and never asserted.
    cpu_lens_ms: u32,
    /// **`TACSIM_MINDS_VARIANTS`**, a sizing knob: a comma list of variant
    /// words to run, in the lane's order — the weekly run never sets it and
    /// runs all four, fill first; an unknown word is ignored, an empty list
    /// is all four.
    variants: Vec<MindsVariant>,
}

impl MindsKnobs {
    fn from_env() -> MindsKnobs {
        let minds = tacsim::config::MindsConfig::default();
        let chosen = std::env::var("TACSIM_MINDS_VARIANTS").unwrap_or_default();
        let variants: Vec<MindsVariant> = MINDS_VARIANTS
            .iter()
            .copied()
            .filter(|variant| chosen.split(',').any(|word| word.trim() == variant.word()))
            .collect();
        MindsKnobs {
            jitter_ms: i64::from(env_u32("TACSIM_MINDS_JITTER_MS", DEFAULT_MINDS_JITTER_MS)),
            jitter_per_mille: env_u32(
                "TACSIM_MINDS_JITTER_PER_MILLE",
                DEFAULT_MINDS_JITTER_PER_MILLE,
            )
            .min(1_000),
            workers: minds.workers,
            cpu_lens_ms: minds.cpu_lens_ms,
            variants: if variants.is_empty() {
                MINDS_VARIANTS.to_vec()
            } else {
                variants
            },
        }
    }
}

/// **Whether a run asserts its timings** — the weekly run does; the un-ignored
/// smoke run ([`m4_2_the_minds_lane_runs_small`]) is a debug build on sixteen
/// hulls, whose timings mean nothing, so its lens checks are printed and never
/// pushed into the verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MindsLaneMode {
    Weekly,
    Smoke,
}

impl MindsLaneMode {
    const fn word(self) -> &'static str {
        match self {
            MindsLaneMode::Weekly => "weekly",
            MindsLaneMode::Smoke => "smoke",
        }
    }
}

/// **The four variants**, in the order they run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MindsVariant {
    /// Thirty-six Class I hulls a side on `PRODUCTION_FILL_ENGINE`, the rest
    /// the ROM, at the authored reach: the decision's variant (taken
    /// 2026-10-09: native), the production fill's since.
    FillAuthored,
    /// Every hull the ROM (Class 0, native), at the CI reach: the arena
    /// lane's field (since 17b cut M4.5 the arena lane's own terms), on the
    /// pool and with the jitter.
    StockCi,
    /// Every hull the ROM, at the authored reach.
    StockAuthored,
    /// Two `heavy_mk1` a side at Class III on the load mind, the rest the
    /// ROM, at the authored reach: the pool's ceiling.
    CeilingAuthored,
}

/// Fill first: its verdict was the decision (taken 2026-10-09: native), and
/// is the production fill's reading since.
const MINDS_VARIANTS: [MindsVariant; 4] = [
    MindsVariant::FillAuthored,
    MindsVariant::StockCi,
    MindsVariant::StockAuthored,
    MindsVariant::CeilingAuthored,
];

/// The four words, for the delta ([`minds_delta`]).
const MINDS_VARIANT_WORDS: [&str; 4] = [
    "fill_authored",
    "stock_ci",
    "stock_authored",
    "ceiling_authored",
];

/// **The authored slots a variant fields a side**: their class, the engine
/// they are sealed on, whether they are heavies on the load mind, and the
/// first side index they occupy.
#[derive(Clone, Copy, Debug)]
struct AuthoredSlots {
    class: tacsim::arena::core::CoreClass,
    engine: tacsim::arena::SealEngine,
    heavy: bool,
}

impl MindsVariant {
    /// The word every `harness_minds_*` line and the JSON's `"variant"` carry.
    const fn word(self) -> &'static str {
        match self {
            MindsVariant::FillAuthored => "fill_authored",
            MindsVariant::StockCi => "stock_ci",
            MindsVariant::StockAuthored => "stock_authored",
            MindsVariant::CeilingAuthored => "ceiling_authored",
        }
    }

    /// The reach, and so the window lens ([`window_lens_ms`]).
    const fn reach(self) -> Reach {
        match self {
            MindsVariant::StockCi => Reach::Ci,
            MindsVariant::FillAuthored
            | MindsVariant::StockAuthored
            | MindsVariant::CeilingAuthored => Reach::Authored,
        }
    }

    /// The authored slots a side, or none on the stock variants.
    fn authored(self) -> Option<AuthoredSlots> {
        use tacsim::arena::core::CoreClass;
        match self {
            MindsVariant::FillAuthored => Some(AuthoredSlots {
                class: CoreClass::I,
                engine: tacsim::match_::PRODUCTION_FILL_ENGINE,
                heavy: false,
            }),
            MindsVariant::CeilingAuthored => Some(AuthoredSlots {
                class: CoreClass::III,
                engine: tacsim::arena::SealEngine::Scripted,
                heavy: true,
            }),
            MindsVariant::StockCi | MindsVariant::StockAuthored => None,
        }
    }

    /// The engine word the lines print: the authored slots' seal word, `rom`
    /// where every hull is the ROM.
    fn engine_word(self) -> &'static str {
        self.authored().map_or("rom", |slots| slots.engine.as_str())
    }
}

/// **The admission arithmetic** (the M4 row's finding (8); doc 17 §9): how
/// many hulls a side `admit_side` admits at `class` with the rest at the
/// ROM's 400, classed first, every hull counted — the fighting line's seats
/// included, read `admitted_class(Some(class), fill_class)`, which answers
/// the class whatever the fill's class says (a fixture naming a class is the
/// documented case; 17b cut M5.1 took the seat bit out of `admitted_class`,
/// which never read it for a named class, and with it this function's
/// `fighting` count — no figure here moved). 36 at Class I and 2 at Class III at the 300-unit defaults
/// ([`m4_2_the_harness_s_three_hundred_admit_thirty_six_at_class_i_and_two_at_class_iii_a_side`]).
fn admitted_per_side(
    cfg: &tacsim::config::MindsConfig,
    units: u32,
    class: tacsim::arena::core::CoreClass,
) -> u32 {
    use tacsim::arena::core::CoreClass;
    use tacsim::match_::pool::{admit_side, admitted_class, load_of, pool_for};
    let pool = pool_for(cfg, usize::try_from(units).unwrap_or(usize::MAX));
    let per_side = units / 2;
    (0..=per_side)
        .rev()
        .find(|&n| {
            let loads = (0..per_side).map(|k| {
                load_of(
                    &cfg.classes,
                    admitted_class(
                        Some(if k < n { class } else { CoreClass::Zero }),
                        cfg.fill_class,
                    ),
                )
            });
            admit_side(pool, loads).is_ok()
        })
        .unwrap_or(0)
}

/// **The plan of one variant's field**: the slots a side, how many the pool
/// admits at the variant's class, how many are fielded, and which unit
/// numbers are the authored slots.
#[derive(Clone, Debug)]
struct MindsPlan {
    per_side: u32,
    admitted_per_side: u32,
    fielded_per_side: u32,
    /// The authored slots' unit numbers (`n`, the `ObjId` less 100,000).
    authored: std::collections::BTreeSet<u32>,
}

impl MindsPlan {
    /// The plan over `placement`, every unit's number and the x of the cell
    /// the seeded scatter gave it ([`field_minds`]' dry pass): the fill
    /// variant's slots are the first `fielded` side indices (the fighting
    /// line's seats included: every hull counts); the ceiling variant's are,
    /// per team, the [`MINDS_HEAVIES_PER_SIDE`] unseated hulls deepest in its
    /// own rear — red the farthest west of the centre column, blue the
    /// farthest east, ties by unit number — at most the pool's admitted count.
    fn of(
        knobs: &Knobs,
        cfg: &tacsim::config::MindsConfig,
        variant: MindsVariant,
        placement: &[(u32, i64)],
    ) -> MindsPlan {
        let per_side = knobs.units / 2;
        let Some(slots) = variant.authored() else {
            return MindsPlan {
                per_side,
                admitted_per_side: 0,
                fielded_per_side: 0,
                authored: std::collections::BTreeSet::new(),
            };
        };
        let admitted = admitted_per_side(cfg, knobs.units, slots.class);
        let mut authored = std::collections::BTreeSet::new();
        let fielded = if slots.heavy {
            let middle_cm = i64::from(SIDE) / 2 * i64::from(CELL_CM);
            let mut fielded = 0_u32;
            for team in 0..2_u32 {
                let mut rear: Vec<(i64, u32)> = placement
                    .iter()
                    .filter(|(n, _)| n % 2 == team && *n >= knobs.fighting)
                    .map(|(n, x_cm)| {
                        // Red's rear is west of the centre, blue's east.
                        let depth = if team == 0 {
                            middle_cm - x_cm
                        } else {
                            x_cm - middle_cm
                        };
                        (-depth, *n)
                    })
                    .collect();
                rear.sort_unstable();
                let take = usize::try_from(admitted.min(MINDS_HEAVIES_PER_SIDE)).unwrap_or(0);
                let chosen: Vec<u32> = rear.iter().take(take).map(|(_, n)| *n).collect();
                fielded = fielded.max(u32::try_from(chosen.len()).unwrap_or(u32::MAX));
                authored.extend(chosen);
            }
            fielded
        } else {
            let fielded = admitted.min(per_side);
            authored.extend(
                placement
                    .iter()
                    .map(|(n, _)| *n)
                    .filter(|n| n / 2 < fielded),
            );
            fielded
        };
        MindsPlan {
            per_side,
            admitted_per_side: admitted,
            fielded_per_side: fielded,
            authored,
        }
    }

    /// Whether unit number `n` is an authored slot.
    fn is_authored(&self, n: u32) -> bool {
        self.authored.contains(&n)
    }

    /// Authored hulls fielded, both sides.
    fn authored_fielded(&self) -> u32 {
        u32::try_from(self.authored.len()).unwrap_or(u32::MAX)
    }

    /// The authored slots as a word: the unit numbers when few, else the count.
    fn authored_words(&self) -> String {
        if self.authored.len() <= 8 {
            self.authored
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        } else {
            format!("{}-of-{}", self.authored.len(), self.per_side * 2)
        }
    }
}

/// **The minds field**: [`field`] — the same seed, the same placement, the
/// same ids, the same teams, the same seats as frame sinks, every hull
/// `Controller::Mind` ([`rom_slot`], since 17b cut M4.5 the arena lanes' own
/// field) — with the variant's authored slots' terms swapped in. A ROM hull
/// is sealed `class_cap: Zero` on the native engine, so `seal_for` writes
/// class `Zero`, `Arena::mind_class` answers `Zero` and every think is handed
/// to the ROM (`collect_think`'s `dropped`) — no Rhai, ops nought by
/// construction, admission counting it at 400 all the same. A fill slot is
/// the cohort chassis rated and capped at Class I on `PRODUCTION_FILL_ENGINE`;
/// a ceiling slot is `heavy_mk1` (the one command-core hull) rated and capped
/// at Class III on the scripted engine, its seal's `mind` and
/// `source_sha256` written to the load mind after custody (`seal_for` writes
/// Standard's digest on every seal and the terms carry no source; the seal's
/// own digest is computed on call, so the `mind_sealed` line and the
/// installer's lookup read the override with no re-digest).
fn field_minds(
    knobs: &Knobs,
    catalog: &Catalog,
    relief: &[(u16, u16, i16)],
    variant: MindsVariant,
    minds_cfg: &tacsim::config::MindsConfig,
) -> (
    Vec<tacsim::arena::state::CustodyUnit>,
    Vec<tacsim::arena::state::Pilot>,
    MindsPlan,
) {
    use tacsim::arena::state::Controller;
    use tacsim::match_::SealTerms;
    let slots = variant.authored();
    // The dry pass: the placement the seeded scatter gives every unit number
    // (the pass is deterministic, so the real one below places identically),
    // which the plan reads to put the heavies in each side's rear.
    let (placed, _) = field_with(knobs, catalog, relief, variant.reach(), rom_slot);
    let placement: Vec<(u32, i64)> = placed
        .iter()
        .map(|unit| (unit.id.0.saturating_sub(100_000), unit.pose.x_cm))
        .collect();
    drop(placed);
    let plan = MindsPlan::of(knobs, minds_cfg, variant, &placement);
    let (units, pilots) = field_with(knobs, catalog, relief, variant.reach(), |n| {
        let cohort_entry = COHORT
            .get(usize::try_from(n / 2).unwrap_or(0) % COHORT.len())
            .copied()
            .unwrap_or(content::LINE_TANK);
        match slots.filter(|_| plan.is_authored(n)) {
            Some(slots) => FieldSlot {
                entry: if slots.heavy {
                    content::HEAVY_MK1
                } else {
                    cohort_entry
                },
                controller: Controller::Mind,
                terms: SealTerms {
                    rated: slots.class,
                    class_cap: slots.class,
                    engine_for_minds: slots.engine,
                    ..SealTerms::fixture()
                },
                sealed_source: slots.heavy.then(|| (LOAD_III_MIND, load_iii_sha256())),
            },
            None => rom_slot(n),
        }
    });
    (units, pilots, plan)
}

/// **The native stock mind, timed** — the arena-wide fallback every ROM hull
/// and every native-sealed hull thinks on, with each think's wall microseconds
/// kept (the clock is the lane's, never the library's: doc 17 §23).
#[derive(Debug)]
struct TimedNative {
    inner: tacsim::arena::stock_mind::NativeStockMind,
    samples: std::sync::Mutex<Vec<NativeSample>>,
}

/// One native think: how long, whether it faulted (it never does), and
/// whether the arena handed it as a dropped job — the ROM's think of a
/// Class 0 hull (or of a mind dropped by its faults or its core), against a
/// native-sealed hull thinking at its own class (the fill's, since the fill
/// decision of 2026-10-09).
#[derive(Clone, Copy, Debug)]
struct NativeSample {
    us: u64,
    faulted: bool,
    dropped: bool,
}

impl TimedNative {
    fn new(cfg: &tacsim::config::ReflexConfig) -> TimedNative {
        TimedNative {
            inner: tacsim::arena::stock_mind::NativeStockMind::with(cfg),
            samples: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// The samples so far, emptied.
    fn take(&self) -> Vec<NativeSample> {
        std::mem::take(
            &mut *self
                .samples
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

impl tacsim::arena::mind::MindRunner for TimedNative {
    fn think(&self, job: &tacsim::arena::mind::ThinkJob<'_>) -> tacsim::arena::mind::ThinkResult {
        let at = Instant::now();
        let result = self.inner.think(job);
        let us = u64::try_from(at.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.samples
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(NativeSample {
                us,
                faulted: result.fault.is_some(),
                dropped: job.dropped,
            });
        result
    }
}

/// **The minds lane's wake clock**: [`WakeClock`]'s rule (`TACSIM_WAKE_JITTER_MS`,
/// default nought) and then, on a seeded `jitter_per_mille` fraction of
/// wakes drawn as `stagger_ms(seed ^ salt, index, 1000)`, `jitter_ms` more —
/// a deterministic function of the seed and the wake index, recorded in the
/// block as `jitter_ms`, `jitter_per_mille` and `jittered_wakes`.
struct MindsClock {
    general: WakeClock,
    seed: u64,
    jitter_ms: i64,
    per_mille: u32,
    index: u32,
    jittered: u32,
    /// Whether the last wall answered was a late one.
    late: bool,
}

impl MindsClock {
    fn new(knobs: &Knobs, minds: &MindsKnobs) -> MindsClock {
        MindsClock {
            general: WakeClock::new(knobs),
            seed: knobs.seed ^ MINDS_JITTER_SALT,
            jitter_ms: minds.jitter_ms.max(0),
            per_mille: minds.jitter_per_mille.min(1_000),
            index: 0,
            jittered: 0,
            late: false,
        }
    }

    /// The wall the next wake happens at, after a step that answered
    /// `next_due_ms`.
    fn next(&mut self, next_due_ms: i64) -> i64 {
        let wall = self.general.next(next_due_ms);
        self.index = self.index.wrapping_add(1);
        let draw = tacsim::arena::rng::stagger_ms(self.seed, ObjId(self.index), 1_000);
        self.late = draw < i64::from(self.per_mille);
        if self.late {
            self.jittered += 1;
            wall.saturating_add(self.jitter_ms)
        } else {
            wall
        }
    }
}

/// **What one batch's wait came to** at a wake's head, before the snapshot or
/// at the tail: the blocked microseconds and what the delivery settled.
#[derive(Clone, Copy, Debug, Default)]
struct Awaited {
    us: u64,
    faults: u32,
    overflows: u32,
    dropped: u32,
}

/// **What the recorders held** since the last drain: the Rhai thinks' ops,
/// fees, microseconds and faults, and the native thinks' microseconds and
/// faults.
#[derive(Clone, Debug, Default)]
struct Drained {
    ops: u64,
    fees: u64,
    rhai_us: Vec<u64>,
    /// `ops + fee` per Rhai think, the figure the budget trips on.
    budget: Vec<u64>,
    rhai_faulted: u32,
    native_us: Vec<u64>,
    native_faulted: u32,
    /// The native thinks handed as no dropped job: a native-sealed hull
    /// thinking at its own class, above the ROM.
    native_minded: u32,
}

impl Drained {
    fn rhai_thinks(&self) -> u32 {
        u32::try_from(self.rhai_us.len()).unwrap_or(u32::MAX)
    }

    fn native_thinks(&self) -> u32 {
        u32::try_from(self.native_us.len()).unwrap_or(u32::MAX)
    }

    /// Every think's microseconds, Rhai and native.
    fn us(&self) -> impl Iterator<Item = u64> + '_ {
        self.rhai_us.iter().chain(self.native_us.iter()).copied()
    }

    fn cpu_us(&self) -> u64 {
        self.us().fold(0_u64, u64::saturating_add)
    }
}

/// **What the minds lane steps** — [`Stepper`]'s shape with the pool, the
/// batch in flight, the recorders and the integrity sentences beside it.
struct MindsStepper {
    arena: tacsim::arena::state::Arena,
    substep: u32,
    counts: Counts,
    clock: MindsClock,
    /// The wall the next wake is at, and whether it is a late one.
    wall: i64,
    late: bool,
    slope_refusals: u64,
    pool: std::sync::Arc<tacsim::minds::MindPool>,
    /// The batch submitted after the last step, awaited at the next head.
    outstanding: Option<tacsim::minds::BatchHandle>,
    snapshot_ms: Option<u32>,
    last_snapshot_period: Option<i64>,
    /// The lane's recording runners, one per distinct scripted source,
    /// installed over the production installer's (`Arena::runners`).
    recorders: Vec<std::sync::Arc<tacsim::minds::RhaiRunner>>,
    /// The timed native stock mind on `Arena::mind_runner`.
    native: std::sync::Arc<TimedNative>,
    /// **The integrity sentences** — the verdict's `sound=` word: a failed
    /// batch, a think outstanding at a step's head, a refused snapshot, a
    /// delivery that settled fewer than it was handed. Never a decision input.
    unsound: Vec<String>,
    batches: u32,
    batches_failed: u32,
}

impl MindsStepper {
    /// **The task's `await_thinks`**: the outstanding batch waited for through
    /// [`tacsim::minds::BatchHandle::wait`] directly (the task's
    /// `spawn_blocking` is tokio's paused clock's concern, not this lane's),
    /// its answers delivered (`Arena::deliver_answers`), a failed batch — a
    /// runner panicked; the task would void — recorded as unsound with the
    /// outstanding thinks discarded as the task discards them.
    fn await_batch(&mut self, where_: &str) -> Option<Awaited> {
        let handle = self.outstanding.take()?;
        let jobs = u32::try_from(handle.len()).unwrap_or(u32::MAX);
        let at = Instant::now();
        let awaited = match handle.wait() {
            Ok(answers) => {
                let us = u64::try_from(at.elapsed().as_micros()).unwrap_or(u64::MAX);
                let settled = self.arena.deliver_answers(answers);
                if settled.written.saturating_add(settled.dropped) != jobs {
                    self.unsound.push(format!(
                        "a batch of {jobs} thinks delivered {} written and {} dropped at {where_}",
                        settled.written, settled.dropped
                    ));
                }
                Awaited {
                    us,
                    faults: settled.faults,
                    overflows: settled.overflows,
                    dropped: settled.dropped,
                }
            }
            Err(failed) => {
                self.batches_failed += 1;
                self.unsound.push(format!(
                    "the pool answered a failed batch at {where_}: {failed:?} (a runner \
                     panicked; the task would void the match)"
                ));
                self.arena.discard_outstanding();
                Awaited {
                    us: u64::try_from(at.elapsed().as_micros()).unwrap_or(u64::MAX),
                    ..Awaited::default()
                }
            }
        };
        Some(awaited)
    }

    /// **The figures recorded since the last drain**: pool workers write only
    /// between a submit and the wait that follows it, and inline thinks run
    /// on this thread inside `Arena::step`, so a drain after the wait reads
    /// the batch's figures and a drain after the step the inline ones.
    fn drain(&self) -> Drained {
        let mut drained = Drained::default();
        for recorder in &self.recorders {
            for stat in recorder.take_stats() {
                drained.ops = drained.ops.saturating_add(stat.ops);
                drained.fees = drained.fees.saturating_add(stat.fee);
                drained
                    .rhai_us
                    .push(u64::try_from(stat.rhai_us).unwrap_or(u64::MAX));
                drained.budget.push(stat.ops.saturating_add(stat.fee));
                drained.rhai_faulted += u32::from(stat.faulted);
            }
        }
        for sample in self.native.take() {
            drained.native_us.push(sample.us);
            drained.native_faulted += u32::from(sample.faulted);
            drained.native_minded += u32::from(!sample.dropped);
        }
        drained
    }
}

/// **One measured wake of the minds lane**: [`Wake`]'s figures, the thinks it
/// collected split by where they ran, the landings, the faults, and the
/// figures of its inline thinks (inside `ms`) and its pooled ones (written
/// back at the next head), all keyed to this wake. `Default` since 16c
/// `T-108`, for [`t108_the_minds_per_wake_lens_is_asserted_over_the_unjittered_wakes`]'s
/// fixture wakes.
#[derive(Default)]
struct MindWake {
    ms: f64,
    now_ms: i64,
    substeps: u32,
    /// Substeps shorter than the lattice's this wake cut (a wake off the
    /// lattice, or the general jitter's remainder).
    short_substeps: u32,
    /// Whether this wake was a jittered one ([`MindsClock`]'s `late`, copied
    /// with the wall it answered) — since 16c `T-108` the per-wake lens's
    /// key: the lens is asserted over the wakes where this is false
    /// ([`WakeLens`]).
    late: bool,
    traces: u32,
    cell_probes: u64,
    horizon_probes: u64,
    jam_checks: u32,
    share_ops: u32,
    /// Thinks collected this wake (`MotorCounters::thinks`).
    thinks: u32,
    /// Collected and run at a substep head inside this wake's `ms`.
    inline_thinks: u32,
    /// Collected in the last substep and handed to the pool.
    pooled_thinks: u32,
    /// Collected in a match-ending substep and never run.
    unrun_thinks: u32,
    landings: u32,
    inline_faults: u32,
    inline_overflows: u32,
    pooled_faults: u32,
    pooled_overflows: u32,
    pooled_dropped: u32,
    inline: Drained,
    pooled: Drained,
    /// The wait at the head that followed this wake, when its batch was
    /// still running.
    pool_wait_us: Option<u64>,
}

impl MindWake {
    fn faults(&self) -> u32 {
        self.inline_faults.saturating_add(self.pooled_faults)
    }

    fn ops(&self) -> u64 {
        self.inline.ops.saturating_add(self.pooled.ops)
    }

    fn fees(&self) -> u64 {
        self.inline.fees.saturating_add(self.pooled.fees)
    }

    fn cpu_us(&self) -> u64 {
        self.inline.cpu_us().saturating_add(self.pooled.cpu_us())
    }

    /// The recorded thinks: Rhai and native, inline and pooled.
    fn timed_thinks(&self) -> u32 {
        self.inline
            .rhai_thinks()
            .saturating_add(self.inline.native_thinks())
            .saturating_add(self.pooled.rhai_thinks())
            .saturating_add(self.pooled.native_thinks())
    }

    fn rhai_thinks(&self) -> u32 {
        self.inline
            .rhai_thinks()
            .saturating_add(self.pooled.rhai_thinks())
    }

    /// The native thinks above the ROM (`Drained::native_minded`).
    fn native_minded_thinks(&self) -> u32 {
        self.inline
            .native_minded
            .saturating_add(self.pooled.native_minded)
    }
}

/// **What one measured window of the minds lane produced**: [`Run`]'s shape
/// over [`MindWake`]s, the `view_us` samples and the last substep hash.
struct MindsRun {
    wakes: Vec<MindWake>,
    substeps: Vec<u32>,
    short_substeps: u64,
    frames: Vec<f64>,
    ended: Option<tacsim::arena::state::MatchEnd>,
    lagged_pulses: u32,
    perception_total: tacsim::arena::perception::PerceptionCounters,
    peak: tacsim::arena::perception::PerceptionCounters,
    motor_total: tacsim::arena::motor::MotorCounters,
    snapshots: Vec<SnapshotSample>,
    documents: Vec<serde_json::Value>,
    /// `Arena::mind_view` on a clone, microseconds, on about forty wakes.
    view_us: Vec<u64>,
    /// The last substep's hash of the last wake (`Arena::substep_hashes`).
    final_substep_hash: Option<u64>,
}

/// **The measured wakes, on the task's policy** (the section head). Not
/// [`run_wakes`]: since 17b cut M4.5 that loop settles each step's thinks
/// synchronously right after the step — run inline on this thread and
/// written, outside `Wake.ms` and before the snapshot, `Arena::step_settled`'s
/// path — so no pool ever holds a batch across a wake. Nor
/// `Arena::step_settled` itself, whose inline thinks would sit inside a timer
/// around it. This loop is the task's pooled path: the step's thinks are
/// submitted to the lane's own `MindPool` after the publish and run on its
/// threads while this thread finishes the wake, and the batch is awaited and
/// delivered at the next wake's head — or earlier, past the snapshot's due
/// check, since `Snapshot::encode` refuses an unanswered think — its wait kept
/// beside the wake, never in it. The order of one wake is the task's: the head
/// (wait, deliver), the step (timed alone), the chunks (the publish's
/// stand-in), the submit, the snapshot past its due check (wait, then encode).
#[allow(clippy::too_many_lines)]
fn run_minds_wakes(
    s: &mut MindsStepper,
    until_ms: i64,
    view_sample_every: u32,
    flush_at_end: bool,
) -> MindsRun {
    use tacsim::arena::perception::PerceptionCounters;
    let substep_ms = i64::from(s.substep);
    let mut run = MindsRun {
        wakes: Vec::new(),
        substeps: Vec::new(),
        short_substeps: 0,
        frames: Vec::new(),
        ended: None,
        lagged_pulses: 0,
        perception_total: PerceptionCounters::default(),
        peak: PerceptionCounters::default(),
        motor_total: tacsim::arena::motor::MotorCounters::default(),
        snapshots: Vec::new(),
        documents: Vec::new(),
        view_us: Vec::new(),
        final_substep_hash: None,
    };
    // The previous batch's figures land on the wake that collected it.
    let write_back = |run: &mut MindsRun, awaited: Option<Awaited>, pooled: Drained| {
        if let Some(last) = run.wakes.last_mut() {
            if let Some(awaited) = awaited {
                last.pool_wait_us = Some(awaited.us);
                last.pooled_faults = last.pooled_faults.saturating_add(awaited.faults);
                last.pooled_overflows = last.pooled_overflows.saturating_add(awaited.overflows);
                last.pooled_dropped = last.pooled_dropped.saturating_add(awaited.dropped);
            }
            last.pooled = pooled;
        }
    };
    let mut wake_index = 0_u32;
    while s.arena.now_ms < until_ms {
        // 1. The head: the task's `await_thinks`, outside the timer; then
        //    the batch's figures, written back onto the wake that collected
        //    it. The first wake of a run finds the warm-up's tail already
        //    awaited; its drain reads nothing.
        let awaited = s.await_batch("the wake's head");
        if awaited.is_some() {
            s.batches += 1;
        }
        let pooled = s.drain();
        write_back(&mut run, awaited, pooled);
        // The guard the lens relies on, a plain check in every build: a think
        // still outstanding would be settled at the step's head INSIDE the
        // timer, and its runner's time would read as the wake's.
        let outstanding = s.arena.thinks_outstanding().len();
        if outstanding > 0 || !s.arena.awaiting.is_empty() {
            s.unsound.push(format!(
                "{outstanding} thinks were still outstanding and {} hulls awaiting at the head \
                 of wake {wake_index}: pooled time would be inside Wake.ms",
                s.arena.awaiting.len()
            ));
        }
        // 2. The step, timed alone — `Wake.ms` is what the task's `took` is.
        let before = s.arena.now_ms;
        let late = s.late;
        let at = Instant::now();
        let mut out = s.arena.step(s.wall, &[], &[]);
        let ms = at.elapsed().as_secs_f64() * 1_000.0;
        // 3. The inline figures: the thinks this step collected minus the
        //    ones it handed out ran at substep heads inside `step`. A
        //    match-ending substep's thinks are never run (the output is
        //    empty), so that wake counts them as unrun.
        let inline = s.drain();
        let pooled_thinks = u32::try_from(out.thinks.len()).unwrap_or(u32::MAX);
        let (inline_thinks, unrun_thinks) = if out.ended.is_some() {
            // The ending substep's collections stay in `unsettled`, handed
            // out to nobody; an earlier substep's ran inline as ever.
            let unrun = u32::try_from(s.arena.thinks_outstanding().len()).unwrap_or(u32::MAX);
            (out.motor.thinks.saturating_sub(unrun), unrun)
        } else {
            (out.motor.thinks.saturating_sub(pooled_thinks), 0)
        };
        // 4. The chunks: the publish's stand-in, as `run_wakes` counts them.
        for chunk in &out.ledger_chunks {
            s.counts.absorb(&chunk.bytes);
        }
        s.arena
            .ledger
            .mark_flushed(s.arena.ledger.last_closed_seq());
        // 5. The submit, after the publish (the task's order).
        let thinks = std::mem::take(&mut out.thinks);
        if !thinks.is_empty() {
            s.outstanding = Some(s.pool.submit(thinks));
        }
        // 6. The snapshot, where the task takes it: past the due check, the
        //    batch awaited (its figures are drained at the next head and
        //    written back; its wait is this wake's), then the encode, timed
        //    and never folded into `ms`.
        let mut snapshot_wait = None;
        if let Some(snapshot_ms) = s.snapshot_ms
            && out.ended.is_none()
            && tacsim::arena_task::snapshot_due(
                &mut s.last_snapshot_period,
                s.arena.now_ms,
                snapshot_ms,
            )
        {
            if let Some(awaited) = s.await_batch("before the snapshot's encode") {
                s.batches += 1;
                snapshot_wait = Some(awaited);
            }
            let at = Instant::now();
            match tacsim::arena::Snapshot::encode(&s.arena) {
                Ok(snapshot) => {
                    let encode_ms = at.elapsed().as_secs_f64() * 1_000.0;
                    run.snapshots.push(SnapshotSample {
                        now_ms: s.arena.now_ms,
                        bytes: snapshot.bytes().len(),
                        encode_ms,
                        wake_ms: ms,
                    });
                    run.documents.push(snapshot.document);
                }
                Err(error) => s.unsound.push(format!(
                    "the snapshot at {} ms was refused: {error}",
                    s.arena.now_ms
                )),
            }
        }
        // 7. `view_us`: on about forty wakes, evenly spaced, one hull whose
        //    think this wake collected, on a clone, outside the timer
        //    (`mind_view` is `&mut self`: it moves ranges and notices).
        if wake_index.is_multiple_of(view_sample_every.max(1))
            && run.view_us.len() < MINDS_VIEW_SAMPLES
            && let Some(unit) = s.arena.awaiting.iter().next().copied()
        {
            let mut clone = s.arena.clone();
            let at = Instant::now();
            let _ = clone.mind_view(unit);
            run.view_us
                .push(u64::try_from(at.elapsed().as_micros()).unwrap_or(u64::MAX));
        }
        // 8. The record.
        let full = u32::try_from((s.arena.now_ms - before) / substep_ms.max(1)).unwrap_or(0);
        let short = out.substeps.saturating_sub(full);
        run.short_substeps += u64::from(short);
        let mut wake = MindWake {
            ms,
            now_ms: s.arena.now_ms,
            substeps: out.substeps,
            short_substeps: short,
            late,
            traces: out.perception.traces,
            cell_probes: out.perception.cell_probes,
            horizon_probes: out.perception.horizon_probes,
            jam_checks: out.perception.jam_checks,
            share_ops: out.perception.share_ops,
            thinks: out.motor.thinks,
            inline_thinks,
            pooled_thinks,
            unrun_thinks,
            landings: out.motor.landings,
            inline_faults: out.motor.mind_faults,
            inline_overflows: out.motor.mind_overflows,
            pooled_faults: 0,
            pooled_overflows: 0,
            pooled_dropped: 0,
            inline,
            pooled: Drained::default(),
            pool_wait_us: None,
        };
        if let Some(awaited) = snapshot_wait {
            wake.pool_wait_us = Some(awaited.us);
            wake.pooled_faults = awaited.faults;
            wake.pooled_overflows = awaited.overflows;
            wake.pooled_dropped = awaited.dropped;
        }
        run.wakes.push(wake);
        s.slope_refusals += u64::from(out.motor.slope_refusals);
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
        run.motor_total.absorb(&out.motor);
        run.substeps.push(out.substeps);
        #[allow(clippy::cast_precision_loss)]
        run.frames.push(out.frames.len() as f64);
        if out.lagged {
            run.lagged_pulses += 1;
        }
        wake_index = wake_index.wrapping_add(1);
        if let Some(end) = out.ended {
            run.ended = Some(end);
            break;
        }
        s.wall = s.clock.next(out.next_due_ms);
        s.late = s.clock.late;
    }
    // The tail: the batch in flight awaited and written back, the ledger
    // flushed where the measurement ends (not at the warm-up's end, which
    // `warm_up` never flushes either), the last substep hash read.
    let awaited = s.await_batch("the run's tail");
    if awaited.is_some() {
        s.batches += 1;
    }
    let pooled = s.drain();
    write_back(&mut run, awaited, pooled);
    if flush_at_end {
        s.arena.ledger.flush();
        for chunk in s.arena.ledger.take_chunks() {
            s.counts.absorb(&chunk.bytes);
        }
    }
    run.final_substep_hash = s.arena.substep_hashes.last().copied();
    run
}

/// **One [`WINDOW_MS`] window of the minds lane's wakes, summed**: the wake
/// milliseconds (the window lens's figure), the think microseconds of the
/// thinks its wakes collected (the CPU lens's figure, inline and pooled), the
/// ops, the substeps its wakes ran, whether any wake in it was off the
/// lattice (a jittered wake with a remainder substep, or the general
/// jitter's), and the five counters.
#[derive(Clone, Copy, Debug, Default)]
struct MindsWindow {
    ms: f64,
    cpu_us: u64,
    ops: u64,
    substeps: u32,
    off_lattice: bool,
    traces: u64,
    cell_probes: u64,
    horizon_probes: u64,
    jam_checks: u64,
    share_ops: u64,
}

/// [`windows`]' fold over [`MindWake`]s — the same key, `(now_ms − start − 1)
/// / 500`, so a wake's inline think time is in the window the lens reads and
/// its pooled think time is keyed to the same window.
fn minds_windows(wakes: &[MindWake], start_ms: i64) -> Vec<MindsWindow> {
    let mut folded: BTreeMap<i64, MindsWindow> = BTreeMap::new();
    for wake in wakes {
        let slot = folded
            .entry((wake.now_ms - start_ms - 1).div_euclid(WINDOW_MS))
            .or_default();
        slot.ms += wake.ms;
        slot.cpu_us = slot.cpu_us.saturating_add(wake.cpu_us());
        slot.ops = slot.ops.saturating_add(wake.ops());
        slot.substeps = slot.substeps.saturating_add(wake.substeps);
        slot.off_lattice |= wake.short_substeps > 0;
        slot.traces += u64::from(wake.traces);
        slot.cell_probes += wake.cell_probes;
        slot.horizon_probes += wake.horizon_probes;
        slot.jam_checks += u64::from(wake.jam_checks);
        slot.share_ops += u64::from(wake.share_ops);
    }
    folded.into_values().collect()
}

impl MindsWindow {
    /// **Comparable with the lens's baseline**: exactly the lattice's substeps
    /// (`WINDOW_MS / substep`, two on the shipped cohort) and none of them a
    /// remainder — the section head says why a jittered run has others.
    fn comparable(&self, substep_ms: u32) -> bool {
        let lattice = u32::try_from(WINDOW_MS).unwrap_or(0) / substep_ms.max(1);
        !self.off_lattice && self.substeps == lattice
    }
}

/// **The per-wake lens over the minds lane's wakes, split** (16c `T-108`; the
/// section head's *The per-wake lens is asserted on the UNJITTERED wakes*):
/// the wakes [`MindsClock`] did not make late — the set the lens is asserted
/// over — and the late ones, whose p99 is printed beside as
/// `window_p99_other_ms` is the windows'. Both p99s are [`Tempo::stats`]'
/// nearest rank, the all-wake figure's own, so with no wake late the asserted
/// figure IS the all-wake p99, bit for bit. **The key is the clock's `late`
/// flag, not a wake's shape** (one full substep): the flag is the lane's own
/// perturbation as the clock drew it — the set `jittered_wakes` counts in the
/// deterministic block, so the two counts here sum to the wakes and the
/// jittered one is that figure — and a late wake is in the other set whatever
/// it carried (two substeps on the lattice, a remainder off it), so none can
/// sneak into the asserted one. A shape rule would not be the all-wake figure
/// with the jitter off (the general `TACSIM_WAKE_JITTER_MS`'s remainders, or
/// an on-time wake the arena's own `next_due_ms` set more than one substep
/// out, give an on-time wake another shape) and would excuse such a wake
/// from the lens for its shape, a wider exemption than `T-108`'s. A drawn
/// wake at `TACSIM_MINDS_JITTER_MS=0` is on time and still set aside — the
/// sample shrinks, no late wake enters; `TACSIM_MINDS_JITTER_PER_MILLE=0` is
/// the jitter's off knob.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WakeLens {
    /// The wakes the clock did not make late.
    unjittered: usize,
    /// Their p99, milliseconds: the asserted figure (`0.0` for none, and then
    /// the lens was not measured — [`WakeLens::measured`]).
    p99: f64,
    /// The wakes the clock made late.
    jittered: usize,
    /// Their p99, milliseconds, report-only (`0.0` for none).
    p99_jittered: f64,
}

impl WakeLens {
    /// The two sets of `wakes`, keyed by [`MindWake::late`] alone.
    fn of(wakes: &[MindWake]) -> WakeLens {
        let set = |late: bool| {
            let ms: Vec<f64> = wakes
                .iter()
                .filter(|wake| wake.late == late)
                .map(|wake| wake.ms)
                .collect();
            let count = ms.len();
            let (_, _, p99, _) = Tempo {
                name: "minds_engaged",
                period_ms: 0,
                ms,
            }
            .stats();
            (count, p99)
        };
        let (unjittered, p99) = set(false);
        let (jittered, p99_jittered) = set(true);
        WakeLens {
            unjittered,
            p99,
            jittered,
            p99_jittered,
        }
    }

    /// **Whether the lens was measured at all**: at least one unjittered wake.
    /// The empty set is the window lens's rule — a run with no comparable
    /// window does not hold — because a p99 of no wakes reads `0.000`, and a
    /// knob that jitters every wake (`TACSIM_MINDS_JITTER_PER_MILLE=1000`) or
    /// a run with no wake would otherwise hold the lens by measuring nothing.
    fn measured(&self) -> bool {
        self.unjittered > 0
    }

    /// **The verdict's two per-wake checks**, `(held, sentence)`, both timed:
    /// measured at all, then the unjittered p99 within `lens_ms`. The empty
    /// set's p99 is `0.0`, so a run with no unjittered wake reads one
    /// sentence, the first.
    fn checks(&self, lens_ms: f64) -> [(bool, String); 2] {
        [
            (
                self.measured(),
                format!(
                    "no unjittered wake among {}: the per-wake lens was not measured",
                    self.unjittered + self.jittered
                ),
            ),
            (
                self.p99 <= lens_ms,
                format!(
                    "per-wake p99 {:.3} ms over the {} unjittered wakes is past the {lens_ms} ms \
                     lens (the {} jittered wakes' p99 {:.3} ms beside)",
                    self.p99, self.unjittered, self.jittered, self.p99_jittered
                ),
            ),
        ]
    }
}

/// **The CPU lens, computed** (the M4 row's finding (7)): the pool's ops per
/// window over the host's measured budget ops per microsecond, times
/// [`CPU_LENS_HEADROOM`], in milliseconds — 5.208 ms at 24 ops/µs on the
/// shipped 300-unit pool, 10.417 at 12. It bounds per-window variance against
/// the host's own rate, never interpreter speed; `[tacsim.minds].cpu_lens_ms`
/// is printed beside it and never asserted.
#[allow(clippy::cast_precision_loss)]
fn cpu_lens_ms_for(pool_per_window: u64, budget_per_us: f64) -> f64 {
    if budget_per_us <= 0.0 {
        return 0.0;
    }
    pool_per_window as f64 / budget_per_us * CPU_LENS_HEADROOM / 1_000.0
}

/// `(p50, p99, max)` of `values` as `u64`s (`0`s for none).
fn spread_u64(values: &[u64]) -> (u64, u64, u64) {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let at = |p: usize| {
        sorted
            .get((sorted.len() * p / 100).min(sorted.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0)
    };
    (at(50), at(99), sorted.last().copied().unwrap_or(0))
}

/// The first percentile of `values` (`0` for none).
fn p1_u64(values: &[u64]) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 100).copied().unwrap_or(0)
}

/// A figure in thousandths printed with three decimals (`11.912`).
fn milli_word(milli: u64) -> String {
    format!("{}.{:03}", milli / 1_000, milli % 1_000)
}

/// Ops per microsecond, per think, in thousandths: `budget × 1000 / µs` for
/// every think that took at least a microsecond.
fn rates_milli(budget: &[u64], us: &[u64]) -> Vec<u64> {
    budget
        .iter()
        .zip(us)
        .filter(|(_, us)| **us > 0)
        .map(|(budget, us)| budget.saturating_mul(1_000) / us)
        .collect()
}

/// **The host's rate for the CPU lens**: Standard Doctrine's own thinks over a
/// real view, measured once per lane run — on the first variant's field after
/// its warm-up, a Class I hull where there is one — so the four lenses share
/// one footing (the stock variants run no Rhai). The twin's method
/// (`twin_rhai_ops_per_us`): the whole Rhai think, the wraps, the memory's
/// decode and encode and the script, over [`CALIBRATION_THINKS`] thinks with
/// the memory fed forward; the budget rate (`ops + fee` per µs, the pool's
/// unit) feeds the lens, the ops-only rate is printed beside it for 17b §2's
/// M2.7 row (11.9 on this host, 19.5–24.1 on doc 17 §18's micro-benchmark).
struct Calibration {
    /// `(p50, p99, p1)`, thousandths.
    budget_per_us_milli: (u64, u64, u64),
    /// `(p50, p99, p1)`, thousandths.
    ops_per_us_milli: (u64, u64, u64),
    thinks: usize,
    faulted: u32,
    unit: ObjId,
    class: tacsim::arena::core::CoreClass,
}

impl Calibration {
    /// The budget rate the lens is computed from, ops per microsecond.
    #[allow(clippy::cast_precision_loss)]
    fn budget_per_us(&self) -> f64 {
        self.budget_per_us_milli.0 as f64 / 1_000.0
    }

    fn line(&self) -> String {
        format!(
            "harness_minds_calibration: budget_per_us_p50={} p99={} p1={} ops_per_us_p50={} p99={} p1={} thinks={} faulted={} unit=#{} class={:?} (Standard Doctrine over a real view of the first variant's field, the memory fed forward; the budget rate feeds the CPU lens; doc 17 section 18 measured 19.5-24.1 ops/us on the blessing host's micro-benchmark, 17b section 2's M2.7 row 11.9 on this host)",
            milli_word(self.budget_per_us_milli.0),
            milli_word(self.budget_per_us_milli.1),
            milli_word(self.budget_per_us_milli.2),
            milli_word(self.ops_per_us_milli.0),
            milli_word(self.ops_per_us_milli.1),
            milli_word(self.ops_per_us_milli.2),
            self.thinks,
            self.faulted,
            self.unit.0,
            self.class
        )
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "budget_per_us": {
                "p50": milli_word(self.budget_per_us_milli.0),
                "p99": milli_word(self.budget_per_us_milli.1),
                "p1": milli_word(self.budget_per_us_milli.2),
            },
            "ops_per_us": {
                "p50": milli_word(self.ops_per_us_milli.0),
                "p99": milli_word(self.ops_per_us_milli.1),
                "p1": milli_word(self.ops_per_us_milli.2),
            },
            "thinks": self.thinks,
            "faulted": self.faulted,
            "unit": self.unit.0,
            "class": format!("{:?}", self.class),
            "code": "standard.rhai over a real view, the memory fed forward",
        })
    }
}

/// See [`Calibration`].
fn calibrate_ops_per_us(field: &MindsField) -> Calibration {
    use mush_mind::Digest;
    use std::sync::Arc;
    use tacsim::arena::core::CoreClass;
    use tacsim::arena::mind::{MindRunner, ThinkJob};
    use tacsim::arena::state::Controller;
    let arena = &field.stepper.arena;
    let minded = |class: Option<CoreClass>| {
        arena
            .units
            .iter()
            .filter(|unit| unit.controller == Controller::Mind && !unit.limits.wreck)
            .map(|unit| unit.id)
            .find(|unit| class.is_none_or(|class| arena.mind_class(*unit) == class))
    };
    let unit = minded(Some(CoreClass::I))
        .or_else(|| minded(None))
        .expect("the minds field carries a mind");
    let class = arena.mind_class(unit);
    let mut clone = arena.clone();
    let view = Arc::new(clone.mind_view(unit).expect("the hull has a view"));
    let profile = clone.classes.row(CoreClass::I);
    let runtime = clone.profiles.get(&unit).expect("admitted runtime");
    let card = runtime.card.clone();
    let runner = tacsim::minds::RhaiRunner::new(
        &field.arena_cfg,
        tacsim::minds::STANDARD_RHAI,
        Digest(tacsim::minds::standard_digest().0),
    )
    .expect("Standard Doctrine compiles")
    .recording();
    let substep = i64::from(field.stepper.substep);
    let mut memory: Vec<u8> = Vec::new();
    for n in 0..CALIBRATION_THINKS {
        let job = ThinkJob {
            unit,
            think_seq: u64::try_from(n).unwrap_or(u64::MAX),
            t_ms: view.t_ms,
            lands_ms: view.t_ms.saturating_add(substep),
            seed: clone.seed,
            dropped: false,
            profile: &profile,
            view: &view,
            memory: &memory,
            map: &clone.definition,
            card: &card,
            program: &runtime.program,
            profile_token: runtime.token(),
        };
        memory = runner.think(&job).memory;
    }
    let stats = runner.take_stats();
    let us: Vec<u64> = stats
        .iter()
        .map(|stat| u64::try_from(stat.rhai_us).unwrap_or(u64::MAX))
        .collect();
    let budget: Vec<u64> = stats
        .iter()
        .map(|stat| stat.ops.saturating_add(stat.fee))
        .collect();
    let ops: Vec<u64> = stats.iter().map(|stat| stat.ops).collect();
    let budget_rates = rates_milli(&budget, &us);
    let ops_rates = rates_milli(&ops, &us);
    let (b50, b99, _) = spread_u64(&budget_rates);
    let (o50, o99, _) = spread_u64(&ops_rates);
    Calibration {
        budget_per_us_milli: (b50, b99, p1_u64(&budget_rates)),
        ops_per_us_milli: (o50, o99, p1_u64(&ops_rates)),
        thinks: stats.len(),
        faulted: stats
            .iter()
            .filter(|stat| stat.faulted)
            .count()
            .try_into()
            .unwrap_or(u32::MAX),
        unit,
        class,
    }
}

/// `git rev-parse HEAD` now, or `unknown`.
fn head_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|commit| !commit.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// **A minds field fielded, opened and warmed up** — [`Field`]'s shape for
/// one variant, with the plan, the pool and the runners.
struct MindsField {
    variant: MindsVariant,
    stepper: MindsStepper,
    arena_cfg: tacsim::config::ArenaConfig,
    sensors_cfg: tacsim::config::SensorsConfig,
    plan: MindsPlan,
    class_counts: BTreeMap<&'static str, u64>,
    passable: usize,
    placed: u32,
    seated: u32,
    /// Scripted seals the production installer gave a runner.
    installed: usize,
    ammo_at_start: BTreeMap<ObjId, u64>,
    generate_ms: f64,
    boot_ms: f64,
    apply_ms: f64,
    open_ms: f64,
    convert_ms: f64,
    arena_open_ms: f64,
    warm_ms: f64,
    warm_pulses: u32,
    killed_warming: usize,
    warm_end: i64,
    /// The warm-up's sentences — the cold check, an installer refusal — into
    /// the verdict's `sound=` (never a panic: the fill variant's `held=` word
    /// must be printed whatever the run did).
    unsound_at_open: Vec<String>,
}

/// **The relief, the minds field and the warm-up** ([`open_field`]'s body on
/// [`field_minds`]): the production installer installs every scripted seal's
/// runner from the source its seal digest names, the lane's recording runners
/// are then installed over them (one per distinct source, so the sandbox's
/// compile cache is shared), the timed native wrapper takes the arena-wide
/// fallback, and the warm-up runs through [`run_minds_wakes`] itself on the
/// task's policy until `startup_ms + 2 × Class 0's think_ms + engaged_ms`
/// (the first think at most one period out on the stagger grid, its landing
/// one substep later, the startup itself). The cold check is a verdict
/// sentence, not a panic.
#[allow(clippy::too_many_lines)]
fn open_minds_field(
    knobs: &Knobs,
    minds: &MindsKnobs,
    variant: MindsVariant,
    mode: MindsLaneMode,
) -> MindsField {
    use mush_mind::Digest;
    use std::sync::Arc;
    use tacsim::arena::SealEngine;
    use tacsim::arena::clock::Tempo as ArenaTempo;
    use tacsim::arena::mind::MindRunner;
    use tacsim::arena::state::{Arena, CustodyOut, Power};
    use tacsim::config::{ArenaConfig, MatchConfig};

    let Window {
        tw,
        cells: all_cells,
        classes: class_counts,
        generate_ms,
        boot_ms,
        apply_ms,
        open_ms,
        ..
    } = open_window(knobs.seed);
    let catalog = fielded_catalog(&tw);
    drop(tw);
    // The ceiling variant's chassis: the one command-core hull.
    let heavy = CanonicalId::new(content::HEAVY_MK1).expect("a canonical entry id");
    assert!(
        catalog.specs.contains_key(&heavy) && catalog.bindings.contains_key(&heavy),
        "the booted catalog does not field {}: the ceiling variant has no heavy",
        content::HEAVY_MK1
    );

    let convert_at = Instant::now();
    let (grid, passable) = arena_ground(&all_cells);
    let definition = arena_definition(grid);
    let convert_ms = convert_at.elapsed().as_secs_f64() * 1_000.0;
    assert!(
        u32::try_from(passable.len()).unwrap_or(0) > knobs.units,
        "only {} passable cells for {} units",
        passable.len(),
        knobs.units
    );

    let minds_cfg = tacsim::config::MindsConfig::default();
    let (units, pilots, plan) = field_minds(knobs, &catalog, &passable, variant, &minds_cfg);
    let placed = u32::try_from(units.len()).unwrap_or(0);
    let seated = u32::try_from(pilots.len()).unwrap_or(0);
    // The sources the installer looks up by the SEAL's digest.
    let mut unsound_at_open = Vec::new();
    let mut sources: BTreeMap<[u8; 32], String> = BTreeMap::new();
    for unit in units
        .iter()
        .filter(|unit| unit.seal.engine == SealEngine::Scripted)
    {
        match source_for(&unit.seal.source_sha256) {
            Some(text) => {
                sources.insert(unit.seal.digest(), text.to_owned());
            }
            None => unsound_at_open.push(format!(
                "#{} is sealed scripted on a source the lane does not carry",
                unit.id.0
            )),
        }
    }

    let arena_cfg = ArenaConfig::default();
    let mut match_cfg = MatchConfig::default();
    match_cfg.max_duration_ms = match_cfg.max_duration_ms.max(
        u32::try_from(
            i64::from(match_cfg.startup_ms)
                .saturating_add(i64::from(arena_cfg.reflex.decide_ms))
                .saturating_add(i64::from(arena_cfg.tempo.engaged_ms))
                .saturating_add(i64::from(knobs.arena_s).saturating_mul(1_000))
                .saturating_add(60_000),
        )
        .unwrap_or(u32::MAX),
    );
    let sensors_cfg = tacsim::config::SensorsConfig::default();
    let open_at = Instant::now();
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots,
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: knobs.seed,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &arena_cfg,
        &match_cfg,
        &sensors_cfg,
    )
    .expect("the minds arena opens");
    let arena_open_ms = open_at.elapsed().as_secs_f64() * 1_000.0;
    arena.clock.pin_tempo_for_tests(ArenaTempo::Engaged);
    let substep =
        tacsim::arena::clock::substep_ms(&arena_cfg.tempo, arena.cell_cm(), arena.max_speed_cms());

    // The runners: the production installer first (the path a match takes),
    // then the lane's recorders over its runners, then the timed fallback.
    // Both before the first step: `CollectedThink.runner` is read at
    // collection.
    let installed = match tacsim::minds::install_sealed_runners(&mut arena, &arena_cfg, &|digest| {
        sources.get(digest).cloned()
    }) {
        Ok(installed) => installed,
        Err(error) => {
            unsound_at_open.push(format!("the installer refused the field: {error}"));
            0
        }
    };
    let mut recorders: Vec<Arc<tacsim::minds::RhaiRunner>> = Vec::new();
    let mut recorder_of: BTreeMap<[u8; 32], Arc<tacsim::minds::RhaiRunner>> = BTreeMap::new();
    for text in sources.values() {
        let key: [u8; 32] = {
            use sha2::{Digest as _, Sha256};
            Sha256::digest(text.as_bytes()).into()
        };
        if recorder_of.contains_key(&key) {
            continue;
        }
        match tacsim::minds::RhaiRunner::new(&arena_cfg, text, Digest(key)) {
            Ok(runner) => {
                let recorder = Arc::new(runner.recording());
                recorders.push(Arc::clone(&recorder));
                recorder_of.insert(key, recorder);
            }
            Err(error) => {
                unsound_at_open.push(format!("a recording runner did not compile: {error}"))
            }
        }
    }
    let seal_digests: BTreeMap<ObjId, [u8; 32]> = arena
        .units
        .iter()
        .map(|unit| (unit.id, unit.seal.digest))
        .collect();
    for (unit, runner) in &mut arena.runners {
        let recorder = seal_digests
            .get(unit)
            .and_then(|digest| sources.get(digest))
            .map(|text| {
                use sha2::{Digest as _, Sha256};
                let key: [u8; 32] = Sha256::digest(text.as_bytes()).into();
                key
            })
            .and_then(|key| recorder_of.get(&key));
        match recorder {
            Some(recorder) => *runner = Arc::clone(recorder) as Arc<dyn MindRunner>,
            None => unsound_at_open.push(format!(
                "#{} has an installed runner and no recorder over it",
                unit.0
            )),
        }
    }
    let native = Arc::new(TimedNative::new(&arena_cfg.reflex));
    arena.mind_runner = Arc::clone(&native) as Arc<dyn MindRunner>;

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

    let mut stepper = MindsStepper {
        arena,
        substep,
        counts: Counts::default(),
        clock: MindsClock::new(knobs, minds),
        wall: 0,
        late: false,
        slope_refusals: 0,
        pool: tacsim::minds::MindPool::new(minds.workers),
        outstanding: None,
        snapshot_ms: None,
        last_snapshot_period: None,
        recorders,
        native,
        unsound: Vec::new(),
        batches: 0,
        batches_failed: 0,
    };

    // The warm-up, on the task's policy; its figures are discarded.
    let warm_at = Instant::now();
    let warm_until = i64::from(match_cfg.startup_ms)
        .saturating_add(
            2 * stepper
                .arena
                .classes
                .row(tacsim::arena::core::CoreClass::Zero)
                .think_ms,
        )
        .saturating_add(i64::from(arena_cfg.tempo.engaged_ms));
    let warm = run_minds_wakes(&mut stepper, warm_until, u32::MAX, false);
    let warm_ms = warm_at.elapsed().as_secs_f64() * 1_000.0;
    let warm_pulses = u32::try_from(warm.wakes.len()).unwrap_or(u32::MAX);
    let arena = &stepper.arena;
    let cold: Vec<(ObjId, Power)> = arena
        .units
        .iter()
        .filter(|unit| unit.power != Power::Online && !unit.limits.wreck)
        .map(|unit| (unit.id, unit.power))
        .collect();
    if !cold.is_empty() {
        unsound_at_open.push(format!(
            "{} units never came online by {warm_until} ms: the ROM did not start the field \
             up: {cold:?}",
            cold.len()
        ));
    }
    let killed_warming = arena.units.iter().filter(|unit| unit.limits.wreck).count();
    if mode == MindsLaneMode::Weekly && variant == MindsVariant::StockCi && killed_warming > 0 {
        // The arena lane's rule for its CI cohort at the default reach: a
        // wreck in the warm-up is a regression there (`open_field`); here it
        // is a sentence, so the variant's verdict is still printed.
        if sensor_cells() == SENSOR_CELLS && share_cells() == SHARE_CELLS {
            unsound_at_open.push(format!(
                "{killed_warming} units were wrecked during the stock_ci warm-up at the \
                 default reach"
            ));
        }
    }
    let warm_end = arena.now_ms;
    MindsField {
        variant,
        stepper,
        arena_cfg,
        sensors_cfg,
        plan,
        class_counts,
        passable: passable.len(),
        placed,
        seated,
        installed,
        ammo_at_start,
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
        unsound_at_open,
    }
}

/// **What one variant's run came to**: every line as printed, the JSON's
/// path, the two verdict words with their sentences, and the figures the
/// smoke test reads.
struct MindsReport {
    variant: MindsVariant,
    lines: Vec<String>,
    json: std::path::PathBuf,
    /// The decision's word (taken 2026-10-09: native; a capacity reading
    /// since): the plan's listed asserts, all held.
    held: bool,
    /// The lane's integrity: no failed batch, no outstanding think at a head,
    /// no refused snapshot, a loaded field, the stat cross-check.
    sound: bool,
    failures: Vec<String>,
    unsound: Vec<String>,
    faults: u32,
    inline_thinks: u32,
    pooled_thinks: u32,
    rhai_thinks: u32,
    /// The native thinks above the ROM: a native-sealed slot's (the fill's
    /// since the fill decision of 2026-10-09).
    native_minded_thinks: u32,
    budget_p1: u64,
    budget_max: u64,
}

/// **One variant measured** (the section head): the field's measured window
/// through [`run_minds_wakes`] with the snapshots on (unless
/// `TACSIM_SNAPSHOT_OFF`), the decodes after the loop, the figures, the
/// lines, the checks and the JSON.
#[allow(clippy::too_many_lines)]
fn measure_minds_field(
    knobs: &Knobs,
    minds: &MindsKnobs,
    mut field: MindsField,
    mode: MindsLaneMode,
    calibration: &Calibration,
) -> MindsReport {
    use tacsim::arena::SealEngine;
    use tacsim::match_::pool::pool_for;
    let variant = field.variant;
    let word = variant.word();
    let cohort = variant.reach().word();
    let rss_start_kib = resident_kib();
    let vmrss_start_kib = vmrss_kib();
    field.stepper.snapshot_ms = if knobs.snapshot_off {
        None
    } else {
        Some(field.arena_cfg.snapshot_ms)
    };
    let every_ms = field.stepper.snapshot_ms;
    let until_ms = field
        .warm_end
        .saturating_add(i64::from(knobs.arena_s) * 1_000);
    // About forty `view_us` samples, evenly spaced over the wakes the
    // warm-up's measured rate predicts for the window.
    let expected_wakes = if field.warm_end > 0 {
        u64::from(field.warm_pulses)
            .saturating_mul(u64::from(knobs.arena_s))
            .saturating_mul(1_000)
            / u64::try_from(field.warm_end).unwrap_or(1).max(1)
    } else {
        0
    };
    let view_sample_every =
        u32::try_from(expected_wakes / u64::try_from(MINDS_VIEW_SAMPLES).unwrap_or(40))
            .unwrap_or(u32::MAX)
            .max(1);
    let mut run = run_minds_wakes(&mut field.stepper, until_ms, view_sample_every, true);
    let documents = std::mem::take(&mut run.documents);
    let decoded = decode_snapshots(&documents, &field.stepper.arena.definition);
    drop(documents);
    let rss_end_kib = resident_kib();
    let vmrss_end_kib = vmrss_kib();

    let MindsField {
        stepper,
        arena_cfg,
        sensors_cfg,
        plan,
        class_counts,
        passable,
        placed,
        seated,
        installed,
        ammo_at_start,
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
        unsound_at_open,
        ..
    } = field;
    let MindsStepper {
        arena,
        substep,
        counts,
        clock,
        slope_refusals,
        pool,
        unsound: unsound_stepping,
        batches,
        batches_failed,
        ..
    } = stepper;
    drop(pool);
    let MindsRun {
        wakes,
        substeps,
        short_substeps,
        frames,
        ended,
        lagged_pulses,
        perception_total,
        peak,
        motor_total,
        snapshots,
        documents: _,
        view_us,
        final_substep_hash,
    } = run;
    let snapshot = SnapshotFigures::of(every_ms, &snapshots, decoded, knobs.max_snapshot_bytes);

    // ---- the figures ----
    let minds_cfg = tacsim::config::MindsConfig::default();
    let pool_ops_s = pool_for(&minds_cfg, usize::try_from(placed).unwrap_or(usize::MAX));
    let pool_per_window = pool_ops_s / 2;
    let pool_per_substep = pool_ops_s.saturating_mul(u64::from(substep)) / 1_000;
    let tempo = Tempo {
        name: "minds_engaged",
        period_ms: arena_cfg.tempo.engaged_ms,
        ms: wakes.iter().map(|wake| wake.ms).collect(),
    };
    let (p50, p95, p99, max) = tempo.stats();
    // The per-wake lens's set (16c `T-108`): the unjittered wakes asserted,
    // the jittered ones' p99 beside; `p99` above stays every wake's, printed.
    let wake_lens = WakeLens::of(&wakes);
    let folded = minds_windows(&wakes, warm_end);
    let window_stats = |keep: &dyn Fn(&MindsWindow) -> bool| {
        Tempo {
            name: "minds_window",
            period_ms: u32::try_from(WINDOW_MS).unwrap_or(0),
            ms: folded
                .iter()
                .filter(|window| keep(window))
                .map(|window| window.ms)
                .collect(),
        }
        .stats()
    };
    let (w50, w95, w99, wmax) = window_stats(&|_| true);
    let (_, _, w99_on, _) = window_stats(&|window| window.comparable(substep));
    let (_, _, w99_off, _) = window_stats(&|window| !window.comparable(substep));
    let windows_on_lattice = folded
        .iter()
        .filter(|window| window.comparable(substep))
        .count();
    let windows_off_lattice = folded.len() - windows_on_lattice;
    let windows_remainder = folded.iter().filter(|window| window.off_lattice).count();
    let cpu = Tempo {
        name: "minds_cpu",
        period_ms: u32::try_from(WINDOW_MS).unwrap_or(0),
        #[allow(clippy::cast_precision_loss)]
        ms: folded
            .iter()
            .map(|window| window.cpu_us as f64 / 1_000.0)
            .collect(),
    };
    let (cpu50, _, cpu99, cpu_max) = cpu.stats();
    let lens_ms = cpu_lens_ms_for(pool_per_window, calibration.budget_per_us());
    let window_traces_peak = folded.iter().map(|w| w.traces).max().unwrap_or(0);
    let window_cell_probes_peak = folded.iter().map(|w| w.cell_probes).max().unwrap_or(0);
    let window_horizon_probes_peak = folded.iter().map(|w| w.horizon_probes).max().unwrap_or(0);
    let window_jam_checks_peak = folded.iter().map(|w| w.jam_checks).max().unwrap_or(0);
    let window_share_ops_peak = folded.iter().map(|w| w.share_ops).max().unwrap_or(0);
    let ops_per_window_peak = folded.iter().map(|w| w.ops).max().unwrap_or(0);
    let ops_per_substep_peak = wakes
        .iter()
        .map(|wake| wake.inline.ops.max(wake.pooled.ops))
        .max()
        .unwrap_or(0);
    let sum_u32 =
        |f: &dyn Fn(&MindWake) -> u32| wakes.iter().map(f).fold(0_u32, u32::saturating_add);
    let sum_u64 =
        |f: &dyn Fn(&MindWake) -> u64| wakes.iter().map(f).fold(0_u64, u64::saturating_add);
    let thinks = sum_u32(&|w| w.thinks);
    let inline_thinks = sum_u32(&|w| w.inline_thinks);
    let pooled_thinks = sum_u32(&|w| w.pooled_thinks);
    let unrun_thinks = sum_u32(&|w| w.unrun_thinks);
    let landings = sum_u32(&|w| w.landings);
    let faults = sum_u32(&MindWake::faults);
    let overflows = sum_u32(&|w| w.inline_overflows.saturating_add(w.pooled_overflows));
    let dropped = sum_u32(&|w| w.pooled_dropped);
    let ops_total = sum_u64(&MindWake::ops);
    let fees_total = sum_u64(&MindWake::fees);
    let rhai_thinks = sum_u32(&MindWake::rhai_thinks);
    let native_minded_thinks = sum_u32(&MindWake::native_minded_thinks);
    let timed_thinks = sum_u32(&MindWake::timed_thinks);
    let untimed_thinks = thinks
        .saturating_sub(unrun_thinks)
        .saturating_sub(timed_thinks);
    let stat_faulted = sum_u32(&|w| {
        w.inline
            .rhai_faulted
            .saturating_add(w.inline.native_faulted)
            .saturating_add(w.pooled.rhai_faulted)
            .saturating_add(w.pooled.native_faulted)
    });
    let think_us: Vec<u64> = wakes
        .iter()
        .flat_map(|wake| wake.inline.us().chain(wake.pooled.us()))
        .collect();
    let rhai_us: Vec<u64> = wakes
        .iter()
        .flat_map(|wake| {
            wake.inline
                .rhai_us
                .iter()
                .chain(wake.pooled.rhai_us.iter())
                .copied()
        })
        .collect();
    let native_us: Vec<u64> = wakes
        .iter()
        .flat_map(|wake| {
            wake.inline
                .native_us
                .iter()
                .chain(wake.pooled.native_us.iter())
                .copied()
        })
        .collect();
    let budget: Vec<u64> = wakes
        .iter()
        .flat_map(|wake| {
            wake.inline
                .budget
                .iter()
                .chain(wake.pooled.budget.iter())
                .copied()
        })
        .collect();
    let rates = rates_milli(&budget, &rhai_us);
    let (think50, think99, think_max) = spread_u64(&think_us);
    let (rhai50, rhai99, _) = spread_u64(&rhai_us);
    let (native50, native99, _) = spread_u64(&native_us);
    let (view50, view99, _) = spread_u64(&view_us);
    let (budget50, budget99, budget_max) = spread_u64(&budget);
    let budget_p1 = p1_u64(&budget);
    let (rate50, rate99, _) = spread_u64(&rates);
    let rate_p1 = p1_u64(&rates);
    let waits: Vec<u64> = wakes.iter().filter_map(|wake| wake.pool_wait_us).collect();
    let (wait50, wait99, _) = spread_u64(&waits);
    let wait_total = waits.iter().fold(0_u64, |a, b| a.saturating_add(*b));
    // The late wakes of the measured window (the clock's own count covers
    // the warm-up too, and is printed as `jittered_total`).
    let jittered_wakes = sum_u32(&|w| u32::from(w.late));
    let jittered_total = clock.jittered;
    let substeps_min = substeps.iter().copied().min().unwrap_or(0);
    let substeps_max = substeps.iter().copied().max().unwrap_or(0);
    let late_two_substeps = wakes
        .iter()
        .filter(|wake| wake.late && wake.substeps == 2 && wake.short_substeps == 0)
        .count();
    let jitter_on_lattice = substep > 0 && minds.jitter_ms.rem_euclid(i64::from(substep)) == 0;
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
    let held_at_end = arena
        .units
        .iter()
        .filter(|unit| unit.held.is_some())
        .count();
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
    // The authored hulls still at their class at the end: a degraded heavy
    // (a turret lost, the command core degraded to II) reads here.
    let authored_class = variant.authored().map(|slots| slots.class);
    let authored_at_class_at_end = authored_class.map_or(0, |class| {
        arena
            .units
            .iter()
            .filter(|unit| plan.is_authored(unit.id.0.saturating_sub(100_000)))
            .filter(|unit| arena.mind_class(unit.id) == class)
            .count()
    });
    let snapshot_periods = every_ms.filter(|ms| *ms > 0).map_or(0, |ms| {
        let measured = if ended.is_some() {
            wakes.len().saturating_sub(1)
        } else {
            wakes.len()
        };
        wakes
            .iter()
            .take(measured)
            .map(|wake| wake.now_ms.div_euclid(i64::from(ms)))
            .collect::<std::collections::BTreeSet<i64>>()
            .len()
    });

    // ---- the verdict ----
    // `held=`: exactly the plan's listed asserts (the M4 row's (ii)); the
    // decision read this word (taken 2026-10-09: native), and the owner reads
    // it since as the production fill's capacity. In the smoke run the timing
    // checks (the per-wake and the window lens, each measured at all then
    // within, and the CPU lens) are printed and not pushed.
    let mut failures: Vec<String> = Vec::new();
    let timing = mode == MindsLaneMode::Weekly;
    let mut check = |held: bool, timed: bool, sentence: String| {
        if !held && (!timed || timing) {
            failures.push(sentence);
        }
    };
    // The per-wake lens over the unjittered wakes since 16c `T-108` (every
    // wake before, the jittered quarter's two substeps with it): measured at
    // all, then within the lens — the window lens's two checks' shape.
    for (held, sentence) in wake_lens.checks(ARENA_S_P99_MS) {
        check(held, true, sentence);
    }
    let window_lens = window_lens_ms(variant.reach());
    check(
        windows_on_lattice > 0,
        true,
        format!(
            "no comparable window among {}: the window lens was not measured",
            folded.len()
        ),
    );
    check(
        w99_on <= window_lens,
        true,
        format!(
            "per-window p99 {w99_on:.3} ms over the {windows_on_lattice} comparable windows (the \
             lattice's substeps, inline think time inside Wake.ms) is past the {window_lens} ms \
             lens of the {cohort} cohort"
        ),
    );
    check(
        faults == 0 && overflows == 0,
        false,
        format!(
            "{faults} thinks faulted and {overflows} overflowed: faults are nought on every variant"
        ),
    );
    // Ops are nought on every NATIVE-sealed variant: the stock variants (every
    // hull the ROM) and, since the fill decision, the fill (its slots on the
    // native stock mind). Only the ceiling's load mind thinks in Rhai.
    match variant.authored() {
        None => check(
            ops_total == 0 && rhai_thinks == 0,
            false,
            format!(
                "the stock variant spent {ops_total} ops over {rhai_thinks} Rhai thinks: ops are \
                 nought on the stock variants (every hull the ROM)"
            ),
        ),
        Some(slots) if slots.engine == tacsim::arena::SealEngine::Native => check(
            ops_total == 0 && rhai_thinks == 0,
            false,
            format!(
                "the {} variant spent {ops_total} ops over {rhai_thinks} Rhai thinks: ops are \
                 nought on a native-sealed variant (its slots on the native stock mind, the rest \
                 the ROM)",
                variant.word()
            ),
        ),
        Some(_) => {}
    }
    check(
        ops_per_window_peak <= pool_per_window,
        false,
        format!("ops in one window {ops_per_window_peak} is past the pool's {pool_per_window}"),
    );
    check(
        ops_per_substep_peak <= pool_per_substep,
        false,
        format!("ops in one substep {ops_per_substep_peak} is past the pool's {pool_per_substep}"),
    );
    if variant == MindsVariant::CeilingAuthored {
        check(
            budget_p1 >= LOAD_III_OPS_P1_MIN,
            false,
            format!(
                "the load mind's ops + fee p1 {budget_p1} is under {LOAD_III_OPS_P1_MIN} over \
                 {rhai_thinks} Rhai thinks"
            ),
        );
    }
    check(
        cpu99 <= lens_ms,
        true,
        format!(
            "CPU per window p99 {cpu99:.3} ms is past the computed lens {lens_ms:.3} ms \
             ({pool_per_window} ops per window over {} ops/us by five quarters)",
            milli_word(calibration.budget_per_us_milli.0)
        ),
    );
    // `sound=`: the lane's integrity, never a decision input.
    let mut unsound: Vec<String> = unsound_at_open;
    unsound.extend(unsound_stepping);
    let mut sound_check = |ok: bool, sentence: String| {
        if !ok {
            unsound.push(sentence);
        }
    };
    sound_check(
        lagged_pulses == 0,
        format!("{lagged_pulses} pulses lagged: the measurement was taken while lagging"),
    );
    // The runners' faulted count is at least the arena's: a fault the arena
    // counted was a recorded think. Not equal — a faulted answer of a hull
    // wrecked in its own substep is dropped at the write and counted by no
    // counter; both figures are printed.
    sound_check(
        stat_faulted >= faults,
        format!(
            "the runners recorded {stat_faulted} faulted thinks and the arena counted {faults}: \
             the stats are not the thinks'"
        ),
    );
    sound_check(
        moving > 0 && counts.fired > 0,
        format!(
            "moving_at_end={moving} fired={}: the measurement was of a parked or a quiet field",
            counts.fired
        ),
    );
    sound_check(
        pooled_thinks > 0,
        format!("pooled_thinks={pooled_thinks}: the pool never ran"),
    );
    if minds.jitter_per_mille > 0 && minds.jitter_ms > 0 {
        sound_check(
            inline_thinks > 0,
            format!(
                "inline_thinks={inline_thinks} with {jittered_wakes} jittered wakes: the inline \
                 path never ran"
            ),
        );
    }
    sound_check(
        batches_failed == 0,
        format!("{batches_failed} of {batches} batches failed"),
    );
    match variant.authored() {
        Some(slots) if slots.engine == SealEngine::Scripted => sound_check(
            rhai_thinks > 0 && installed > 0,
            format!(
                "rhai_thinks={rhai_thinks} installed={installed}: the scripted slots never \
                 thought in Rhai"
            ),
        ),
        // The fill decision (2026-10-09): the fill variant is native-sealed,
        // so this arm is the one that proves its Class I slots thought — on
        // the native mind at their own class, never in Rhai and never only
        // the ROM — as the scripted arm proves its slots thought in Rhai.
        Some(_) => sound_check(
            rhai_thinks == 0 && native_minded_thinks > 0,
            format!(
                "rhai_thinks={rhai_thinks} native_minded_thinks={native_minded_thinks} on a \
                 native-sealed variant: its slots think on the native mind above the ROM and \
                 never in Rhai"
            ),
        ),
        None => {}
    }
    if variant == MindsVariant::CeilingAuthored {
        sound_check(
            budget_max <= LOAD_III_OPS_MAX,
            format!(
                "the load mind's ops + fee max {budget_max} is past {LOAD_III_OPS_MAX}: the loop \
                 is out of its band"
            ),
        );
    }
    if every_ms.is_some() {
        sound_check(
            snapshot.count >= 1 && snapshot.count == snapshot_periods && snapshot.hash_ok,
            format!(
                "the {word} variant took {} snapshots over {snapshot_periods} periods at \
                 every_ms={} with hash_ok={}",
                snapshot.count,
                snapshot.every_word(),
                snapshot.hash_ok
            ),
        );
    } else if knobs.assert {
        sound_check(
            false,
            "TACSIM_SNAPSHOT_OFF=1 is for measurement only; an asserting run takes the snapshots"
                .to_string(),
        );
    }
    let held = failures.is_empty();
    let sound = unsound.is_empty();

    // ---- the lines ----
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "harness_minds_world: variant={word} cohort={cohort} units={placed}/{} pilots={seated} cell_cm={CELL_CM} side={SIDE} passable_cells={passable} seed={} arena_ms={} authored_fielded={} authored_slots={} installed={installed}",
        knobs.units,
        knobs.seed,
        arena.now_ms,
        plan.authored_fielded(),
        plan.authored_words()
    ));
    lines.push(format!(
        "harness_minds_setup_ms: variant={word} cohort={cohort} generate={generate_ms:.1} boot={boot_ms:.1} pack_apply={apply_ms:.1} window_open={open_ms:.1} terrain_convert={convert_ms:.1} arena_open={arena_open_ms:.1} warm_up={warm_ms:.1} warm_pulses={warm_pulses} killed_warming={killed_warming}"
    ));
    lines.push(format!(
        "harness_minds_loop: variant={word} cohort={cohort} wakes={wake_count} arena_s={arena_s_run:.3} wakes_per_arena_s={wakes_per_arena_s:.2} substep_ms={substep} jitter_ms={} substeps_per_wake_mean={substeps_per_wake_mean:.3} per_wake_min={substeps_min} per_wake_max={substeps_max} short_substeps={short_substeps} jittered_wakes={jittered_wakes} jittered_total={jittered_total} late_two_substeps={late_two_substeps} minds_jitter_ms={} minds_jitter_per_mille={} jitter_on_lattice={jitter_on_lattice}",
        knobs.jitter_ms, minds.jitter_ms, minds.jitter_per_mille
    ));
    lines.push(format!(
        "harness_minds_engaged: variant={word} cohort={cohort} period_ms={} pulses={wake_count} p50={p50:.3} p95={p95:.3} p99={p99:.3} max={max:.3} unjittered={} p99_unjittered={:.3} jittered={} p99_jittered={:.3} (the per-wake lens is asserted on the unjittered wakes since 16c T-108: a jittered wake is one substep late and steps two, its first lattice point's thinks settled inline inside Wake.ms, which the window lens counts over the comparable windows)",
        tempo.period_ms,
        wake_lens.unjittered,
        wake_lens.p99,
        wake_lens.jittered,
        wake_lens.p99_jittered
    ));
    lines.push(format!(
        "harness_minds_window: variant={word} cohort={cohort} window_ms={WINDOW_MS} windows={} ms_p50={w50:.3} ms_p95={w95:.3} ms_p99={w99:.3} ms_max={wmax:.3} comparable={windows_on_lattice} ms_p99_comparable={w99_on:.3} other={windows_off_lattice} ms_p99_other={w99_off:.3} remainder={windows_remainder} traces_peak={window_traces_peak} cell_probes_peak={window_cell_probes_peak} horizon_probes_peak={window_horizon_probes_peak} jam_checks_peak={window_jam_checks_peak} share_ops_peak={window_share_ops_peak} (the lens is asserted on the comparable windows: exactly the lattice's substeps, none a remainder; inline think time is inside Wake.ms; a late wake due mid-window moves a substep into the next window, so a jittered run has one- and three-substep windows beside the two-substep ones)",
        folded.len()
    ));
    lines.push(format!(
        "harness_minds_pool: variant={word} cohort={cohort} engine={} workers={} units={placed} per_side={} pool_ops_s={pool_ops_s} per_window={pool_per_window} per_substep={pool_per_substep} admitted_class={} admitted_per_side={} fielded_per_side={} pool_wait_us_p50={wait50} pool_wait_us_p99={wait99} pool_wait_us_total={wait_total} batches={batches} batches_failed={batches_failed} (pool_wait_us is the batch's wall latency on a {}-worker pool with no pulse gap, an upper bound on production's blocked time; report-only)",
        variant.engine_word(),
        minds.workers,
        plan.per_side,
        authored_class.map_or("0".to_string(), |class| format!("{class:?}")),
        plan.admitted_per_side,
        plan.fielded_per_side,
        minds.workers
    ));
    lines.push(format!(
        "harness_minds_thinks: variant={word} cohort={cohort} thinks={thinks} inline_thinks={inline_thinks} pooled_thinks={pooled_thinks} unrun_thinks={unrun_thinks} untimed_thinks={untimed_thinks} landings={landings} faults={faults} stat_faulted={stat_faulted} overflows={overflows} dropped={dropped} cond_fires={} think_us_p50={think50} think_us_p99={think99} think_us_max={think_max} rhai_us_p50={rhai50} rhai_us_p99={rhai99} native_us_p50={native50} native_us_p99={native99} native_minded_thinks={native_minded_thinks} view_us_p50={view50} view_us_p99={view99} view_samples={} jittered_wakes={jittered_wakes} jitter_ms={} jitter_per_mille={} (inline_thinks and pooled_thinks are derived per wake from thinks and StepOutput.thinks; landings alone is a counter; an untimed think is a dropped scripted hull's ROM think; native_minded_thinks are the native thinks above the ROM, a native-sealed slot's)",
        motor_total.cond_fires,
        view_us.len(),
        minds.jitter_ms,
        minds.jitter_per_mille
    ));
    lines.push(format!(
        "harness_minds_ops: variant={word} cohort={cohort} ops_total={ops_total} fees_total={fees_total} ops_per_window_peak={ops_per_window_peak} ops_per_substep_peak={ops_per_substep_peak} pool_per_window={pool_per_window} pool_per_substep={pool_per_substep} budget_p1={budget_p1} budget_p50={budget50} budget_p99={budget99} budget_max={budget_max} budget_per_us_p50={} budget_per_us_p99={} budget_per_us_p1={} rhai_thinks={rhai_thinks} attribution=by-collecting-wake (budget is ops + fee per Rhai think, the figure the budget trips on; the ceiling variant's band is {LOAD_III_OPS_P1_MIN} to {LOAD_III_OPS_MAX})",
        milli_word(rate50),
        milli_word(rate99),
        milli_word(rate_p1)
    ));
    lines.push(format!(
        "harness_minds_cpu: variant={word} cohort={cohort} ms_p50={cpu50:.3} ms_p99={cpu99:.3} ms_max={cpu_max:.3} lens_ms={lens_ms:.3} budget_per_us_used={} cpu_lens_ms={} (asserted: ms_p99 <= lens_ms, the pool's ops per window over the host's rate on Standard Doctrine by five quarters, bounding per-window variance and never interpreter speed; cpu_lens_ms is doc 17 section 18's reference figure, printed beside)",
        milli_word(calibration.budget_per_us_milli.0),
        minds.cpu_lens_ms
    ));
    lines.push(format!(
        "harness_minds_counters: variant={word} cohort={cohort} blocked={} ledger_bytes={} chunks={} frames_per_pulse_mean={frames_mean:.2} moving_at_end={moving} held_at_end={held_at_end} lagged_pulses={lagged_pulses} slope_refusals={slope_refusals} ended={}",
        counts.blocked,
        counts.bytes,
        counts.chunks,
        ended.map_or("none".to_string(), |end| format!("{:?}", end.reason))
    ));
    lines.push(format!(
        "harness_minds_weapons: variant={word} cohort={cohort} armed={armed} locked={} fired={} impacts={} hits={} misses={} sections_destroyed={} units_destroyed={} wrecks_at_end={} in_flight_at_end={} rounds_loaded={loaded} rounds_spent={rounds_spent} rounds_left={rounds_left}",
        counts.locked,
        counts.fired,
        counts.impacts,
        counts.hits,
        counts.misses,
        counts.sections,
        counts.destroyed,
        arena.units.iter().filter(|unit| unit.limits.wreck).count(),
        arena.shots.len()
    ));
    lines.push(format!(
        "harness_minds_perception_total: variant={word} cohort={cohort} passes={} candidates={} traces={} cell_probes={} horizon_probes={} jam_checks={} share_ops={} detected={} classified={} lost={} peak_traces={} peak_cell_probes={} peak_jam_checks={} peak_share_ops={} contacts_at_end={}",
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
        peak.traces,
        peak.cell_probes,
        peak.jam_checks,
        peak.share_ops,
        arena.contacts.len()
    ));
    lines.push(snapshot.line(cohort).replacen(
        "harness_arena_snapshot: ",
        &format!("harness_minds_snapshot: variant={word} "),
        1,
    ));
    lines.push(format!(
        "harness_minds_memory: variant={word} cohort={cohort} rss_start_kib={} rss_end_kib={} horizon_bytes={}{}",
        kib_word(rss_start_kib),
        kib_word(rss_end_kib),
        arena.definition.terrain.horizon_bytes(),
        vmrss_words(vmrss_start_kib, vmrss_end_kib)
    ));
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    let sentences = |list: &[String]| {
        if list.is_empty() {
            "none".to_string()
        } else {
            list.join("; ")
        }
    };
    lines.push(format!(
        "harness_minds_verdict: variant={word} cohort={cohort} held={} sound={} assert={} mode={} wake_p99_ms={:.3} wake_lens_ms={ARENA_S_P99_MS} wake_unjittered={} wake_p99_other_ms={:.3} wake_jittered={} window_p99_ms={w99_on:.3} window_lens_ms={window_lens} window_comparable={windows_on_lattice} window_p99_other_ms={w99_off:.3} faults={faults} ops_total={ops_total} budget_p1={budget_p1} cpu_p99_ms={cpu99:.3} cpu_lens_ms={lens_ms:.3} authored_at_class_at_end={authored_at_class_at_end} failures={} | {} | unsound={} | {}",
        yes_no(held),
        yes_no(sound),
        knobs.assert,
        mode.word(),
        wake_lens.p99,
        wake_lens.unjittered,
        wake_lens.p99_jittered,
        wake_lens.jittered,
        failures.len(),
        sentences(&failures),
        unsound.len(),
        sentences(&unsound)
    ));

    // ---- the JSON ----
    let report = serde_json::json!({
        "what": "17b cut M4.2: the minds lane — the relief cohort as Controller::Mind hulls, stepped with Arena::step and a private MindPool on the task's policy, a seeded quarter of wakes one substep late, every think recorded",
        "lane": "minds",
        "variant": word,
        "cohort": cohort,
        "engine": variant.engine_word(),
        "world": {
            "units": placed,
            "pilots": seated,
            "cell_cm": CELL_CM,
            "side_cells": SIDE,
            "passable_cells": passable,
            "seed": knobs.seed,
            "arena_ms": arena.now_ms,
            "pulses": wake_count,
            "authored_fielded": plan.authored_fielded(),
            "authored_slots": plan.authored.iter().copied().collect::<Vec<u32>>(),
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
        "loop": {
            "wakes": wake_count,
            "arena_s": num(arena_s_run),
            "wakes_per_arena_s": num(wakes_per_arena_s),
            "substep_ms": substep,
            "jitter_ms": knobs.jitter_ms,
            "substeps_per_wake_mean": num(substeps_per_wake_mean),
            "per_wake_min": substeps_min,
            "per_wake_max": substeps_max,
            "short_substeps": short_substeps,
            "jittered_total": jittered_total,
            "late_two_substeps": late_two_substeps,
            "jitter_on_lattice": jitter_on_lattice,
        },
        // Every wake's figures, then (16c `T-108`) the per-wake lens's split
        // as `window` carries the comparable windows' beside every window's:
        // the asserted p99 over the unjittered wakes, the jittered ones' beside.
        "tempos": [{
            "name": tempo.name,
            "period_ms": tempo.period_ms,
            "pulses": wake_count,
            "p50_ms": num(p50),
            "p95_ms": num(p95),
            "p99_ms": num(p99),
            "max_ms": num(max),
            "unjittered": wake_lens.unjittered,
            "p99_ms_unjittered": num(wake_lens.p99),
            "jittered": wake_lens.jittered,
            "p99_ms_jittered": num(wake_lens.p99_jittered),
        }],
        "window": {
            "window_ms": WINDOW_MS,
            "windows": folded.len(),
            "ms_p50": num(w50),
            "ms_p95": num(w95),
            "ms_p99": num(w99),
            "ms_max": num(wmax),
            "comparable": windows_on_lattice,
            "ms_p99_comparable": num(w99_on),
            "other": windows_off_lattice,
            "ms_p99_other": num(w99_off),
            "remainder": windows_remainder,
            "traces_peak": window_traces_peak,
            "cell_probes_peak": window_cell_probes_peak,
            "horizon_probes_peak": window_horizon_probes_peak,
            "jam_checks_peak": window_jam_checks_peak,
            "share_ops_peak": window_share_ops_peak,
        },
        "pool": {
            "workers": minds.workers,
            "pool_ops_s": pool_ops_s,
            "per_window": pool_per_window,
            "per_substep": pool_per_substep,
            "admitted_class": authored_class.map(|class| format!("{class:?}")),
            "admitted_per_side": plan.admitted_per_side,
            "fielded_per_side": plan.fielded_per_side,
            "batches": batches,
            "batches_failed": batches_failed,
            "wait_us": { "p50": wait50, "p99": wait99, "total": wait_total },
        },
        // THE DETERMINISTIC BLOCK: every figure a function of the seed and the
        // knobs, none of wall time — compared by the delta once the baseline
        // carries one (`minds_delta`).
        "minds": {
            "thinks": thinks,
            "inline_thinks": inline_thinks,
            "pooled_thinks": pooled_thinks,
            "unrun_thinks": unrun_thinks,
            "landings": landings,
            "faults": faults,
            "overflows": overflows,
            "dropped": dropped,
            "cond_fires": motor_total.cond_fires,
            "ops_total": ops_total,
            "fees_total": fees_total,
            "ops_per_window_peak": ops_per_window_peak,
            "ops_per_substep_peak": ops_per_substep_peak,
            "rhai_thinks": rhai_thinks,
            "final_substep_hash": final_substep_hash.map(|hash| hash.to_string()),
            "jittered_wakes": jittered_wakes,
            "jitter_ms": minds.jitter_ms,
            "jitter_per_mille": minds.jitter_per_mille,
            "admitted_per_side": plan.admitted_per_side,
            "fielded_per_side": plan.fielded_per_side,
            "authored_slots": plan.authored.iter().copied().collect::<Vec<u32>>(),
            "engine": variant.engine_word(),
            "authored_at_class_at_end": authored_at_class_at_end,
            "wakes": wake_count,
            "arena_ms": arena.now_ms,
            // The knobs the block is a function of, so a run off the defaults
            // names itself (the arena block's `world` and `loop.jitter_ms`).
            "units": placed,
            "fighting": seated,
            "arena_s": knobs.arena_s,
            "seed": knobs.seed,
            "wake_jitter_ms": knobs.jitter_ms,
        },
        "timing": {
            "think_us": { "p50": think50, "p99": think99, "max": think_max, "samples": think_us.len() },
            "rhai_us": { "p50": rhai50, "p99": rhai99, "samples": rhai_us.len() },
            "native_us": { "p50": native50, "p99": native99, "samples": native_us.len() },
            "view_us": { "p50": view50, "p99": view99, "samples": view_us.len() },
            "budget_per_us": { "p50": milli_word(rate50), "p99": milli_word(rate99), "p1": milli_word(rate_p1), "samples": rates.len() },
            "untimed_thinks": untimed_thinks,
        },
        "budget": {
            "p1": budget_p1,
            "p50": budget50,
            "p99": budget99,
            "max": budget_max,
        },
        "cpu": {
            "ms_p50": num(cpu50),
            "ms_p99": num(cpu99),
            "ms_max": num(cpu_max),
            "lens_ms": num(lens_ms),
            "budget_per_us_used": milli_word(calibration.budget_per_us_milli.0),
            "cpu_lens_ms": minds.cpu_lens_ms,
        },
        "calibration": calibration.json(),
        "counters": {
            "blocked_events": counts.blocked,
            "ledger_bytes_streamed": counts.bytes,
            "ledger_chunks": counts.chunks,
            "frames_per_pulse_mean": num(frames_mean),
            "moving_at_end": moving,
            "held_at_end": held_at_end,
            "lagged_pulses": lagged_pulses,
            "slope_refusals": slope_refusals,
            "ended": ended.map(|end| format!("{:?}", end.reason)),
        },
        "weapons": {
            "armed_units": armed,
            "locked_events": counts.locked,
            "fired_events": counts.fired,
            "impact_events": counts.impacts,
            "hits": counts.hits,
            "misses": counts.misses,
            "sections_destroyed": counts.sections,
            "units_destroyed": counts.destroyed,
            "wrecks_at_end": arena.units.iter().filter(|unit| unit.limits.wreck).count(),
            "in_flight_at_end": arena.shots.len(),
            "rounds_loaded": loaded,
            "rounds_spent": rounds_spent,
            "rounds_left": rounds_left,
        },
        "perception": {
            "pass_ms": sensors_cfg.pass_ms,
            "contacts_at_end": arena.contacts.len(),
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
            "peak_per_pulse": {
                "traces": peak.traces,
                "cell_probes": peak.cell_probes,
                "jam_checks": peak.jam_checks,
                "share_ops": peak.share_ops,
            },
        },
        "snapshot": snapshot.json(),
        "memory": {
            "rss_start_kib": rss_start_kib,
            "rss_end_kib": rss_end_kib,
            "horizon_bytes": arena.definition.terrain.horizon_bytes(),
            "vmrss_start_kib": vmrss_start_kib,
            "vmrss_end_kib": vmrss_end_kib,
        },
        "verdict": {
            "held": held,
            "sound": sound,
            "assert": knobs.assert,
            "mode": mode.word(),
            "failures": failures,
            "unsound": unsound,
            "host": this_host(),
            "commit": head_commit(),
        },
        "thresholds": {
            "assert": knobs.assert,
            "wake_p99_ms": ARENA_S_P99_MS,
            "window_p99_ms": window_lens,
            "cpu_lens_computed_ms": num(lens_ms),
            "cpu_lens_ms": minds.cpu_lens_ms,
            "pool_per_window": pool_per_window,
            "pool_per_substep": pool_per_substep,
            "load_ops_p1_min": LOAD_III_OPS_P1_MIN,
            "load_ops_max": LOAD_III_OPS_MAX,
        },
    });
    let path = arena_report_path(&knobs.json, &format!("minds-{word}"));
    write_report(&path, &report);
    lines.push(format!(
        "harness_minds_json: variant={word} cohort={cohort} {}",
        path.display()
    ));
    MindsReport {
        variant,
        lines,
        json: path,
        held,
        sound,
        failures,
        unsound,
        faults,
        inline_thinks,
        pooled_thinks,
        rhai_thinks,
        native_minded_thinks,
        budget_p1,
        budget_max,
    }
}

/// **The minds lane** (the section head): the variants in order, each
/// opened, warmed up and measured, its lines printed before the next opens
/// (so a partial run still shows what ran); the host's rate measured once on
/// the first variant's field; and under `TACSIM_ASSERT` one panic at the end
/// naming every variant that did not hold or was not sound. A `stock_ci`
/// `held=no` on ALIEN18 (the owner's host) was to be the owner's STOP before 17b cut
/// M4.5 moved the CI lane onto minds — waived by the owner on 2026-10-08; the
/// verdict stays the owner's to read. The fill variant's `held=` word was
/// the decision's (`PRODUCTION_FILL_ENGINE`, taken 2026-10-09: native) and is
/// the production fill's capacity reading since.
fn measure_minds(
    knobs: &Knobs,
    minds: &MindsKnobs,
    mode: MindsLaneMode,
    variants: &[MindsVariant],
) -> Vec<MindsReport> {
    let mut reports = Vec::new();
    let mut calibration: Option<Calibration> = None;
    for variant in variants {
        let field = open_minds_field(knobs, minds, *variant, mode);
        if calibration.is_none() {
            let measured = calibrate_ops_per_us(&field);
            println!("{}", measured.line());
            calibration = Some(measured);
        }
        let Some(rate) = calibration.as_ref() else {
            break;
        };
        let report = measure_minds_field(knobs, minds, field, mode, rate);
        for line in &report.lines {
            println!("{line}");
        }
        reports.push(report);
    }
    if knobs.assert {
        let failed: Vec<&MindsReport> = reports
            .iter()
            .filter(|report| !report.held || !report.sound)
            .collect();
        let stop = failed
            .iter()
            .any(|report| report.variant == MindsVariant::StockCi && !report.held);
        assert!(
            failed.is_empty(),
            "harness_minds_verdict: {} of {} variants held and sound; see the \
             harness_minds_verdict lines above:\n{}{}",
            reports.len() - failed.len(),
            reports.len(),
            failed
                .iter()
                .map(|report| format!(
                    "  {}: held={} sound={}: {}",
                    report.variant.word(),
                    report.held,
                    report.sound,
                    report
                        .failures
                        .iter()
                        .chain(report.unsound.iter())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ")
                ))
                .collect::<Vec<_>>()
                .join("\n"),
            if stop {
                "\n  stock_ci did not hold: the verdict the owner waived as 17b cut M4.5's \
                 precondition (2026-10-08) and still reads"
            } else {
                ""
            }
        );
    }
    reports
}

/// **The minds lane since 17b cut M4.2** — see the file head and the section
/// head above [`DEFAULT_MINDS_JITTER_MS`].
#[test]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
fn tacsim_harness_arena_minds() {
    let minds = MindsKnobs::from_env();
    let variants = minds.variants.clone();
    measure_minds(&Knobs::from_env(), &minds, MindsLaneMode::Weekly, &variants);
}

/// **The `minds` block of one variant's report, flattened** — every leaf of
/// its `minds` object (the deterministic block [`measure_minds_field`]
/// writes) with the cohort word beside it; a report with no `minds` object
/// reads [`ABSENT`] under `minds`.
fn minds_block(report: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
    let mut block = BTreeMap::new();
    match report.get("minds") {
        Some(minds) => flatten("minds", minds, &mut block),
        None => {
            block.insert("minds".to_string(), serde_json::json!(ABSENT));
        }
    }
    block.insert(
        "minds.cohort".to_string(),
        report
            .get("cohort")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(ABSENT)),
    );
    block
}

/// **One variant's fresh report beside the arena report** (17b cut M4.5),
/// or why it is refused: missing or unreadable, or **older than the test
/// binary** reading it ([`report_predates_the_build`] — an earlier build
/// wrote it, so folding or comparing it would judge this tree on the run
/// before it; the arena report has been refused so since 16b cut U.3).
fn fresh_minds_report(
    json_knob: &str,
    word: &str,
    binary_built: Option<std::time::SystemTime>,
) -> Result<serde_json::Value, String> {
    let path = arena_report_path(json_knob, &format!("minds-{word}"));
    let modified = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok();
    let report: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| {
            format!(
                "harness_delta: minds.{word} no fresh report at {} — run \
                 tacsim_harness_arena_minds first",
                path.display()
            )
        })?;
    if report_predates_the_build(modified, binary_built) {
        return Err(format!(
            "harness_delta: minds.{word} refused: {} is older than this test binary, so an \
             earlier build wrote it — run tacsim_harness_arena_minds on this build",
            path.display()
        ));
    }
    Ok(report)
}

/// **The four variant files beside the arena report**, folded for the bless
/// as `{ "<variant>": { ...minds, "cohort": .. } }` — or every refusal
/// ([`fresh_minds_report`]; a report with no `minds` object or no cohort is
/// refused too), so a bless never writes a partial `minds` block nor one off
/// an earlier build's files (17b cut M4.5).
fn minds_blocks_beside(
    json_knob: &str,
    binary_built: Option<std::time::SystemTime>,
) -> Result<serde_json::Value, Vec<String>> {
    let mut folded = serde_json::Map::new();
    let mut refused = Vec::new();
    for word in MINDS_VARIANT_WORDS {
        let report = match fresh_minds_report(json_knob, word, binary_built) {
            Ok(report) => report,
            Err(why) => {
                refused.push(why);
                continue;
            }
        };
        match (
            report.get("minds").and_then(serde_json::Value::as_object),
            report.get("cohort"),
        ) {
            (Some(minds), Some(cohort)) => {
                let mut block = minds.clone();
                block.insert("cohort".to_string(), cohort.clone());
                folded.insert(word.to_string(), serde_json::Value::Object(block));
            }
            _ => refused.push(format!(
                "harness_delta: minds.{word} refused: the report carries no `minds` block \
                 or no cohort"
            )),
        }
    }
    if refused.is_empty() {
        Ok(serde_json::Value::Object(folded))
    } else {
        Err(refused)
    }
}

/// **The delta's minds path** (17b cut M4.2): compared only once the
/// baseline's `minds` object carries a variant block — **since 17b cut M4.5
/// any of [`MINDS_VARIANT_WORDS`] under it**, because the arena report's own
/// `minds.states_at_end` lives there too and is no variant block; until then
/// it says so and counts nothing. With one, every variant's fresh
/// `<stem>-minds-<variant>.<ext>` is read and its block compared leaf by
/// leaf against `minds.<variant>` as [`block_differences`] compares the arena
/// block, each difference a `harness_delta: minds.<variant>.<leaf>` line; a
/// variant file that is missing or unreadable — or, since 17b cut M4.5,
/// older than this binary ([`fresh_minds_report`]) — is one difference,
/// named. Answers the lines and the count.
fn minds_delta(
    baseline: &serde_json::Value,
    json_knob: &str,
    binary_built: Option<std::time::SystemTime>,
) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut differences = 0;
    let Some(expected) = baseline.get("minds").filter(|minds| {
        MINDS_VARIANT_WORDS
            .iter()
            .any(|word| minds.get(word).is_some())
    }) else {
        lines.push(
            "harness_delta: minds block: the baseline carries none; skipped (17b cut M4.5's \
             bless folds the four)"
                .to_string(),
        );
        return (lines, 0);
    };
    for word in MINDS_VARIANT_WORDS {
        let fresh = match fresh_minds_report(json_knob, word, binary_built) {
            Ok(fresh) => fresh,
            Err(why) => {
                lines.push(why);
                differences += 1;
                continue;
            }
        };
        let mut was = BTreeMap::new();
        match expected.get(word) {
            Some(block) => flatten("minds", block, &mut was),
            None => {
                was.insert("minds".to_string(), serde_json::json!(ABSENT));
            }
        }
        let found: Vec<String> = block_differences(&was, &minds_block(&fresh))
            .into_iter()
            .map(|line| {
                line.replacen(
                    "harness_delta: minds.",
                    &format!("harness_delta: minds.{word}."),
                    1,
                )
            })
            .collect();
        if found.is_empty() {
            lines.push(format!(
                "harness_delta: minds.{word} block equal ({} fields)",
                minds_block(&fresh).len()
            ));
        } else {
            differences += found.len();
            lines.extend(found);
        }
    }
    (lines, differences)
}

/// **The load mind is calibrated to the Class III row** (17b cut M4.2; the M4
/// row's (ii)): its bytes are the compiled-in ones and its digest is pinned;
/// one think on a `heavy_mk1` sealed at Class III on the load mind spends
/// exactly the pinned `(ops, fee)`, inside the band, with no fault, and the
/// same off, starting and online (equal cost on both branches, the memory
/// fed forward); on Class I's row the same think faults `ops`; and the lane's
/// source lookup answers Standard's text and the load mind's by digest.
#[test]
fn m4_2_the_load_mind_is_calibrated_to_the_class_iii_row() {
    use mush_mind::Digest;
    use sha2::{Digest as _, Sha256};
    use std::sync::Arc;
    use tacsim::arena::SealEngine;
    use tacsim::arena::clock::Tempo as ArenaTempo;
    use tacsim::arena::core::{ClassTable, CoreClass};
    use tacsim::arena::mind::{MindRunner, ThinkJob};
    use tacsim::arena::state::{Arena, Controller, CustodyOut, Power, Team};
    use tacsim::config::{ArenaConfig, MatchConfig};

    // 1. The bytes and the digest.
    let on_disk = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("support")
            .join("load_iii.rhai"),
    )
    .expect("the load mind is in the tree");
    assert_eq!(
        on_disk,
        LOAD_III_RHAI.as_bytes(),
        "the compiled-in bytes are the file's"
    );
    let digest = load_iii_sha256();
    let lowercase: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(
        lowercase, LOAD_III_SHA256_HEX,
        "load_iii.rhai moved; re-run the calibration and re-pin its digest and figures"
    );
    assert_eq!(source_for(&digest), Some(LOAD_III_RHAI));
    assert_eq!(
        source_for(&tacsim::minds::standard_digest().0),
        Some(tacsim::minds::STANDARD_RHAI)
    );
    assert_eq!(source_for(&[7; 32]), None);

    // 2. Two heavies on the proving ground, both minds sealed at Class III on
    //    the scripted engine with the load mind's name and digest written
    //    onto the seal after custody — the ceiling variant's slot.
    let tw = TestWorld::with_modules(fan_out_roster());
    let catalog = fielded_catalog(&tw);
    let definition = tacsim::match_::definition_for(
        tw.world(),
        &smol_str::SmolStr::new(tacsim::match_::PROVING_GROUND),
    )
    .expect("the proving ground's ground builds")
    .expect("the proving ground is defined");
    drop(tw);
    let key = CanonicalId::new(content::HEAVY_MK1).expect("a canonical entry id");
    let spec = catalog.specs.get(&key).expect("the heavy's spec");
    let binding = catalog.bindings.get(&key).expect("the heavy's binding");
    let mut units = Vec::new();
    for (n, team, column, heading) in [
        (0_u32, Team::Red, LANCE_RED_COLUMN, 90_000_u32),
        (1, Team::Blue, LANCE_BLUE_COLUMN, 270_000),
    ] {
        let id = ObjId(100_000 + n);
        let cells = tacsim::definitions::unit::to_tactical_unit(spec, binding, id)
            .expect("the heavy builds a unit");
        let pose = mush_sim::data::Pose {
            x_cm: i64::from(column) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
            y_cm: LANCE_ROW * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
            z_cm: 0,
            heading_mdeg: heading,
            posture: mush_sim::data::passive_rules_value(serde_json::json!({}))
                .expect("an empty posture"),
        };
        let mut unit = tacsim::match_::custody_unit(
            &catalog,
            cells,
            id,
            team,
            pose,
            1,
            Controller::Mind,
            0,
            &tacsim::match_::SealTerms {
                rated: CoreClass::III,
                class_cap: CoreClass::III,
                engine_for_minds: SealEngine::Scripted,
                ..tacsim::match_::SealTerms::fixture()
            },
        )
        .expect("the heavy seals at Class III");
        assert_eq!(
            unit.seal.class,
            CoreClass::III,
            "the command core is Class III"
        );
        unit.seal.mind = smol_str::SmolStr::new_static(LOAD_III_MIND);
        unit.seal.source_sha256 = digest;
        units.push(unit);
    }
    let red = units[0].id;
    let arena_cfg = ArenaConfig::default();
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: DEFAULT_SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &arena_cfg,
        &MatchConfig::default(),
        &tacsim::config::SensorsConfig::default(),
    )
    .expect("the two heavies open");
    arena.clock.pin_tempo_for_tests(ArenaTempo::Engaged);
    assert_eq!(arena.mind_class(red), CoreClass::III);
    assert_eq!(arena.mind_profile(red).ops, 4_500);

    // 3. One think per power state, the memory fed forward: the pinned
    //    figures, equal on every arm.
    let runner = tacsim::minds::RhaiRunner::new(&arena_cfg, LOAD_III_RHAI, Digest(digest))
        .expect("the load mind compiles")
        .recording();
    let three = ClassTable::STOCK.row(CoreClass::III);
    let one = ClassTable::STOCK.row(CoreClass::I);
    let mut memory: Vec<u8> = Vec::new();
    let mut figures = Vec::new();
    for power in [Power::Off, Power::Starting, Power::Online] {
        arena.unit_mut(red).expect("red").power = power;
        let view = Arc::new(arena.mind_view(red).expect("red's view"));
        assert_eq!(view.power, power);
        let runtime = arena.profiles.get(&red).expect("admitted runtime");
        let card = runtime.card.clone();
        let job = |profile| ThinkJob {
            unit: red,
            think_seq: 0,
            t_ms: view.t_ms,
            lands_ms: view.t_ms + i64::from(arena_cfg.tempo.engaged_ms / 2),
            seed: arena.seed,
            dropped: false,
            profile,
            view: &view,
            memory: &memory,
            map: &arena.definition,
            card: &card,
            program: &runtime.program,
            profile_token: runtime.token(),
        };
        let result = runner.think(&job(&three));
        assert_eq!(
            result.fault, None,
            "the load mind is clean on Class III at {power:?}"
        );
        assert_eq!(
            result.intents.len(),
            1,
            "one station call either arm: {:?}",
            result.intents
        );
        let stat = runner.take_stats().pop().expect("one think recorded");
        figures.push((stat.ops, stat.fee));
        assert!(
            (LOAD_III_OPS_P1_MIN..=LOAD_III_OPS_MAX).contains(&(stat.ops + stat.fee)),
            "ops + fee {} is out of the band at {power:?}",
            stat.ops + stat.fee
        );
        // Class I's row: the band is above its 1,500.
        let faulted = runner.think(&job(&one));
        assert_eq!(
            faulted.fault.as_ref().map(|fault| fault.cause.as_str()),
            Some("ops"),
            "the load mind faults `ops` on Class I's row"
        );
        let _ = runner.take_stats();
        memory = result.memory;
        assert!(!memory.is_empty(), "the memory carries `x`");
    }
    assert_eq!(
        figures,
        vec![(4_397, 20), (4_397, 20), (4_397, 20)],
        "17 + 3 × 1,460 ops and the think fee, off, starting and online alike"
    );
    // The same figures through the digest the installer keys by.
    let keyed: [u8; 32] = Sha256::digest(LOAD_III_RHAI.as_bytes()).into();
    assert_eq!(keyed, digest);
}

/// **The admission arithmetic at the harness's three hundred** (17b cut
/// M4.2; the M4 row's finding (8); doc 17 §9): the pool is 200,000, so a side
/// admits thirty-six Class I hulls or two Class III heavies beside the ROM's
/// 400 each, classed first — the exact `PoolRefusal` figures of the
/// thirty-seventh and the third — and the plan fields the heavies at side
/// index 12 (the last two slots on a field too small for it).
#[test]
fn m4_2_the_harness_s_three_hundred_admit_thirty_six_at_class_i_and_two_at_class_iii_a_side() {
    use tacsim::arena::core::CoreClass;
    use tacsim::config::MindsConfig;
    use tacsim::match_::pool::{PoolRefusal, admit_side, admitted_class, pool_for};
    let cfg = MindsConfig::default();
    assert_eq!(pool_for(&cfg, 300), 200_000);
    assert_eq!(admitted_per_side(&cfg, 300, CoreClass::I), 36);
    assert_eq!(admitted_per_side(&cfg, 300, CoreClass::III), 2);
    let loads = |classed: u64, n: usize, rest: usize| {
        std::iter::repeat_n(classed, n).chain(std::iter::repeat_n(400, rest))
    };
    assert_eq!(admit_side(200_000, loads(1_500, 36, 114)), Ok(()));
    // 17b cut M5.1: `admit_side` names the refused load's index (the 149th
    // hull is index 148; the 119th, 118), so `StartError::Pool` can name it.
    assert_eq!(
        admit_side(200_000, loads(1_500, 37, 113)),
        Err((
            148,
            PoolRefusal {
                would: 400,
                half: 100_000,
                spoken: 99_900,
            }
        )),
        "the 149th hull: 37 Class I and 111 ROMs spoken for"
    );
    assert_eq!(admit_side(200_000, loads(18_000, 2, 148)), Ok(()));
    assert_eq!(
        admit_side(200_000, loads(18_000, 3, 147)),
        Err((
            118,
            PoolRefusal {
                would: 400,
                half: 100_000,
                spoken: 100_000,
            }
        )),
        "the 119th hull: three heavies and 115 ROMs reach the half exactly"
    );
    // The fill's class is inert for an explicitly classed hull (17b cut M5.1
    // took the seat bit out: it was inert for one too).
    assert_eq!(
        admitted_class(Some(CoreClass::I), CoreClass::Zero),
        Some(CoreClass::I)
    );
    assert_eq!(
        admitted_class(Some(CoreClass::Zero), CoreClass::I),
        Some(CoreClass::Zero)
    );
    // A sizing run: at a hundred units the pool is 400,000, so every Class I
    // hull of a side is admitted and ten heavies are.
    assert_eq!(admitted_per_side(&cfg, 100, CoreClass::I), 50);
    assert_eq!(admitted_per_side(&cfg, 100, CoreClass::III), 10);
    // The plan over a synthetic placement (unit `n` at x = (n × 37 mod 256)
    // cells): thirty-six fill slots a side from side index 0, the seats
    // included; two heavies a side, the unseated hulls deepest in each rear —
    // red's two smallest x, blue's two largest — and on sixteen hulls the
    // same two a side out of the pool's eight.
    let knobs = |units, fighting| Knobs {
        units,
        fighting,
        ..Knobs::from_env()
    };
    let placement = |units: u32| -> Vec<(u32, i64)> {
        (0..units)
            .map(|n| (n, i64::from((n * 37) % 256) * i64::from(CELL_CM)))
            .collect()
    };
    let fill = MindsPlan::of(
        &knobs(300, 24),
        &cfg,
        MindsVariant::FillAuthored,
        &placement(300),
    );
    assert_eq!(
        (
            fill.admitted_per_side,
            fill.fielded_per_side,
            fill.authored_fielded()
        ),
        (36, 36, 72)
    );
    assert!(fill.is_authored(0) && fill.is_authored(71) && !fill.is_authored(72));
    assert_eq!(fill.authored_words(), "72-of-300");
    let ceiling = MindsPlan::of(
        &knobs(300, 24),
        &cfg,
        MindsVariant::CeilingAuthored,
        &placement(300),
    );
    assert_eq!(
        (
            ceiling.admitted_per_side,
            ceiling.fielded_per_side,
            ceiling.authored_fielded()
        ),
        (2, 2, 4)
    );
    let deepest = |team: u32, west: bool| -> Vec<u32> {
        let mut rear: Vec<(i64, u32)> = placement(300)
            .into_iter()
            .filter(|(n, _)| n % 2 == team && *n >= 24)
            .map(|(n, x)| (if west { x } else { -x }, n))
            .collect();
        rear.sort_unstable();
        rear.iter().take(2).map(|(_, n)| *n).collect()
    };
    let mut expected: Vec<u32> = deepest(0, true);
    expected.extend(deepest(1, false));
    expected.sort_unstable();
    assert_eq!(
        ceiling.authored.iter().copied().collect::<Vec<u32>>(),
        expected
    );
    assert!(
        ceiling.authored.iter().all(|n| *n >= 24),
        "never on the fighting line"
    );
    assert_eq!(
        ceiling.authored_words(),
        expected
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    let small = MindsPlan::of(
        &knobs(16, 4),
        &cfg,
        MindsVariant::CeilingAuthored,
        &placement(16),
    );
    assert_eq!(
        (
            small.admitted_per_side,
            small.fielded_per_side,
            small.authored_fielded()
        ),
        (8, 2, 4)
    );
    let stock = MindsPlan::of(
        &knobs(300, 24),
        &cfg,
        MindsVariant::StockCi,
        &placement(300),
    );
    assert_eq!((stock.fielded_per_side, stock.authored_fielded()), (0, 0));
    assert!(!stock.is_authored(0));
    assert_eq!(MindsVariant::StockCi.engine_word(), "rom");
    assert_eq!(MindsVariant::CeilingAuthored.engine_word(), "scripted");
    assert_eq!(
        MindsVariant::FillAuthored.engine_word(),
        tacsim::match_::PRODUCTION_FILL_ENGINE.as_str()
    );
}

/// **The minds clock jitters a seeded quarter of wakes by one substep** (17b
/// cut M4.2): over ten thousand wakes at the default knobs the late count is
/// inside 225 to 275 per thousand, every late wake is `next_due + 250` and
/// every other wake `next_due` exactly (the general jitter at nought is the
/// identity), the sequence is one function of the seed, a thousandth of
/// nought is never late and a thousand always.
#[test]
fn m4_2_the_minds_clock_jitters_a_quarter_of_wakes_by_one_substep() {
    let mut knobs = Knobs::from_env();
    knobs.seed = DEFAULT_SEED;
    knobs.jitter_ms = 0;
    let minds = |per_mille| MindsKnobs {
        jitter_ms: i64::from(DEFAULT_MINDS_JITTER_MS),
        jitter_per_mille: per_mille,
        workers: 2,
        cpu_lens_ms: 5,
        variants: MINDS_VARIANTS.to_vec(),
    };
    let sequence = |knobs: &Knobs, per_mille| {
        let mut clock = MindsClock::new(knobs, &minds(per_mille));
        (1..=10_000_i64)
            .map(|n| {
                let due = n * 250;
                let wall = clock.next(due);
                assert!(
                    wall == due || wall == due + 250,
                    "wake {n}: {wall} is neither {due} nor {}",
                    due + 250
                );
                assert_eq!(clock.late, wall != due);
                wall - due
            })
            .collect::<Vec<i64>>()
    };
    let quarter = sequence(&knobs, DEFAULT_MINDS_JITTER_PER_MILLE);
    let late = quarter.iter().filter(|late| **late > 0).count();
    assert!(
        (2_250..=2_750).contains(&late),
        "{late} of 10,000 wakes were late"
    );
    assert_eq!(
        sequence(&knobs, DEFAULT_MINDS_JITTER_PER_MILLE),
        quarter,
        "one function of the seed"
    );
    let mut other = Knobs::from_env();
    other.seed = DEFAULT_SEED + 1;
    other.jitter_ms = 0;
    assert_ne!(sequence(&other, DEFAULT_MINDS_JITTER_PER_MILLE), quarter);
    assert!(sequence(&knobs, 0).iter().all(|late| *late == 0));
    assert!(sequence(&knobs, 1_000).iter().all(|late| *late == 250));
    // The knob is clamped to a thousand, and the general jitter still applies.
    assert_eq!(MindsClock::new(&knobs, &minds(5_000)).per_mille, 1_000);
    assert_eq!(
        DEFAULT_MINDS_JITTER_MS, 250,
        "one substep of the shipped cohort"
    );
}

/// **The minds lane's per-wake lens is asserted over the unjittered wakes**
/// (16c `T-108`; [`WakeLens`]): a late wake is out of the asserted set
/// whatever it carried and an on-time wake in it whatever it carried (the
/// key is the clock's flag, not the shape); the late wakes' p99 is the
/// figure beside; with no wake late the asserted p99 is the all-wake one
/// exactly; a run with no unjittered wake reads the lens as not measured and
/// does not hold, in one sentence; and over the clock's own seeded draw the
/// split is the clock's. The fixture's values make each rule move a figure
/// and not only a count (a nearest-rank p99 over eight is the seventh, over
/// four the third, so a set's slowest wake never reaches it): the late
/// two-substep wakes sit past the lens, so every wake's p99 — the figure the
/// verdict asserted before `T-108` — misses where the split's holds, and
/// reverting [`WakeLens::of`] to every wake fails this test; the on-time
/// two-substep wake is the slowest of all and the late one-substep wake
/// under the on-time seventh, so a key on the wake's shape moves the asserted
/// p99 itself (10 to 9.5); and with an on-time wake the slowest, the late
/// wakes' p99 (31) is not every wake's (32), so the figure beside is pinned
/// to the late set.
#[test]
fn t108_the_minds_per_wake_lens_is_asserted_over_the_unjittered_wakes() {
    let wake = |ms: f64, late: bool, substeps: u32| MindWake {
        ms,
        late,
        substeps,
        ..MindWake::default()
    };
    let all_p99 = |wakes: &[MindWake]| {
        Tempo {
            name: "minds_engaged",
            period_ms: 500,
            ms: wakes.iter().map(|wake| wake.ms).collect(),
        }
        .stats()
        .2
    };
    // Eight on-time wakes, one of them two substeps for a reason that is not
    // the jitter and the slowest of all (it stays asserted: the one wake in
    // eight the nearest-rank p99 lets past); three late wakes of two
    // substeps past the lens and one late wake of one substep under the
    // on-time seventh (a drawn wake at `TACSIM_MINDS_JITTER_MS=0`, or a
    // remainder off the lattice) — all four set aside.
    let on_time = [4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let mut wakes: Vec<MindWake> = on_time.iter().map(|ms| wake(*ms, false, 1)).collect();
    wakes.push(wake(50.0, false, 2));
    wakes.extend([30.0, 31.0, 32.0].iter().map(|ms| wake(*ms, true, 2)));
    wakes.push(wake(9.5, true, 1));
    let lens = WakeLens::of(&wakes);
    assert_eq!((lens.unjittered, lens.jittered), (8, 4));
    assert_eq!(lens.unjittered + lens.jittered, wakes.len());
    let unjittered: Vec<MindWake> = on_time
        .iter()
        .map(|ms| wake(*ms, false, 1))
        .chain([wake(50.0, false, 2)])
        .collect();
    assert_eq!(lens.p99, all_p99(&unjittered), "the on-time wakes' p99");
    assert_eq!(lens.p99, 10.0, "nearest rank over eight: the seventh");
    let late: Vec<MindWake> = [30.0, 31.0, 32.0]
        .iter()
        .map(|ms| wake(*ms, true, 2))
        .chain([wake(9.5, true, 1)])
        .collect();
    assert_eq!(lens.p99_jittered, all_p99(&late), "the late wakes' p99");
    assert_eq!(lens.p99_jittered, 31.0, "nearest rank over four: the third");
    // The fixture's guards: keyed by the wake's shape (one substep asserted,
    // two set aside) the asserted p99 would be 9.5, not 10; and every wake's
    // p99 is not the late wakes', so neither the key nor the figure beside
    // can drift and leave the figures standing.
    let by_shape: Vec<MindWake> = wakes
        .iter()
        .filter(|w| w.substeps == 1)
        .map(|w| wake(w.ms, w.late, w.substeps))
        .collect();
    assert_eq!(
        by_shape.len(),
        lens.unjittered,
        "the shape key keeps the count"
    );
    assert_eq!(
        all_p99(&by_shape),
        9.5,
        "the shape key moves the asserted p99"
    );
    assert_eq!(
        all_p99(&wakes),
        32.0,
        "every wake's p99 is not the late ones'"
    );
    // The teeth: every wake's p99 — the figure asserted before `T-108` — is
    // past the lens, the split's is under it.
    assert!(
        all_p99(&wakes) > ARENA_S_P99_MS,
        "the fixture's all-wake p99 {} ms must miss the {ARENA_S_P99_MS} ms lens",
        all_p99(&wakes)
    );
    assert!(lens.measured());
    let checks = lens.checks(ARENA_S_P99_MS);
    assert!(
        checks.iter().all(|(held, _)| *held),
        "the unjittered wakes hold the lens: {checks:?}"
    );
    assert!(
        checks[1]
            .1
            .contains("per-wake p99 10.000 ms over the 8 unjittered wakes")
            && checks[1]
                .1
                .contains("the 4 jittered wakes' p99 31.000 ms beside"),
        "{}",
        checks[1].1
    );
    // A slow on-time wake still fails it: the set is not a free pass.
    let mut slow = wakes
        .iter()
        .map(|w| wake(w.ms, w.late, w.substeps))
        .collect::<Vec<MindWake>>();
    for w in slow.iter_mut().filter(|w| !w.late) {
        w.ms += 15.0;
    }
    let slow = WakeLens::of(&slow);
    assert_eq!(slow.p99, 25.0);
    let slow_checks = slow.checks(ARENA_S_P99_MS);
    assert!(slow_checks[0].0 && !slow_checks[1].0, "{slow_checks:?}");

    // The jitter off (`TACSIM_MINDS_JITTER_PER_MILLE=0`): no wake late, so the
    // asserted figure is the all-wake one, bit for bit.
    let off: Vec<MindWake> = wakes.iter().map(|w| wake(w.ms, false, 1)).collect();
    let off_lens = WakeLens::of(&off);
    assert_eq!((off_lens.unjittered, off_lens.jittered), (off.len(), 0));
    assert_eq!(off_lens.p99.to_bits(), all_p99(&off).to_bits());
    assert_eq!(off_lens.p99_jittered.to_bits(), 0.0_f64.to_bits());

    // No unjittered wake (`TACSIM_MINDS_JITTER_PER_MILLE=1000`), and no wake
    // at all: the lens was not measured, which does not hold — one sentence.
    for empty in [
        wakes
            .iter()
            .map(|w| wake(w.ms, true, 2))
            .collect::<Vec<MindWake>>(),
        Vec::new(),
    ] {
        let none = WakeLens::of(&empty);
        assert!(!none.measured());
        assert_eq!(none.unjittered, 0);
        assert_eq!(none.p99.to_bits(), 0.0_f64.to_bits());
        let checks = none.checks(ARENA_S_P99_MS);
        let failed: Vec<&String> = checks
            .iter()
            .filter(|(held, _)| !*held)
            .map(|(_, sentence)| sentence)
            .collect();
        assert_eq!(
            failed,
            [&format!(
                "no unjittered wake among {}: the per-wake lens was not measured",
                empty.len()
            )],
            "the empty set is unmeasured, in one sentence"
        );
    }

    // Over the clock's own draw at the default knobs: the late walls are the
    // set aside, counted as the deterministic block's `jittered_wakes` is.
    let mut knobs = Knobs::from_env();
    knobs.seed = DEFAULT_SEED;
    knobs.jitter_ms = 0;
    let mut clock = MindsClock::new(
        &knobs,
        &MindsKnobs {
            jitter_ms: i64::from(DEFAULT_MINDS_JITTER_MS),
            jitter_per_mille: DEFAULT_MINDS_JITTER_PER_MILLE,
            workers: 2,
            cpu_lens_ms: 5,
            variants: MINDS_VARIANTS.to_vec(),
        },
    );
    let drawn: Vec<MindWake> = (1..=1_000_i64)
        .map(|n| {
            let due = n * 250;
            let wall = clock.next(due);
            let late = wall != due;
            assert_eq!(clock.late, late);
            wake(
                if late { 30.0 } else { 5.0 },
                clock.late,
                1 + u32::from(late),
            )
        })
        .collect();
    let drawn_lens = WakeLens::of(&drawn);
    let late_count = drawn.iter().filter(|w| w.late).count();
    assert!(late_count > 0 && late_count < drawn.len());
    assert_eq!(drawn_lens.jittered, late_count);
    assert_eq!(
        u32::try_from(drawn_lens.jittered).ok(),
        Some(clock.jittered),
        "the clock's own count"
    );
    assert_eq!((drawn_lens.p99, drawn_lens.p99_jittered), (5.0, 30.0));
}

/// **The CPU lens is the pool's window over the host's rate by five
/// quarters** (17b cut M4.2; the M4 row's finding (7)): 5.208 ms at 24 ops/µs
/// on the shipped 300-unit pool's 100,000 per window, 10.417 at 12; a rate
/// of nought answers nought (never a division); and `[tacsim.minds].cpu_lens_ms`
/// reads doc 17 §18's five, printed beside and never asserted.
#[test]
fn m4_2_the_cpu_lens_is_the_pool_s_window_over_the_host_s_rate_by_five_quarters() {
    let at = |rate: f64| (cpu_lens_ms_for(100_000, rate) * 1_000.0).round() / 1_000.0;
    assert!((at(24.0) - 5.208).abs() < 0.001, "{}", at(24.0));
    assert!((at(12.0) - 10.417).abs() < 0.001, "{}", at(12.0));
    assert!((at(0.0)).abs() < f64::EPSILON);
    assert_eq!(tacsim::config::MindsConfig::default().cpu_lens_ms, 5);
    assert_eq!(MindsKnobs::from_env().cpu_lens_ms, 5);
    assert_eq!(
        MindsKnobs::from_env().variants.len(),
        4,
        "the weekly run's four, fill first"
    );
    assert_eq!(milli_word(11_912), "11.912");
    assert_eq!(milli_word(5), "0.005");
    assert_eq!(rates_milli(&[4_417, 100], &[1_000, 0]), vec![4_417]);
    assert_eq!(spread_u64(&[]), (0, 0, 0));
    assert_eq!(p1_u64(&[3, 1, 2]), 1);
}

/// **The delta compares the minds block only once the baseline carries one**
/// (17b cut M4.2): a baseline with no variant block skips the block and
/// counts nothing — **since 17b cut M4.5** also one whose `minds` carries the
/// arena report's own `states_at_end` and no variant word; with one, every
/// variant's fresh file is compared leaf by leaf — a changed leaf is one
/// difference, named; a missing variant file is one difference, named, and
/// so (17b cut M4.5) is one older than the binary — and the bless folds the
/// four fresh blocks in only when all four files exist and are this build's,
/// refusing (naming each) otherwise. The bless MERGES (17b cut M4.5): the
/// fresh arena report's `minds.states_at_end` survives it with the variant
/// blocks and without them.
#[test]
fn m4_2_the_delta_compares_the_minds_block_only_once_the_baseline_carries_one() {
    let dir = std::env::temp_dir().join(format!("tacsim-minds-delta-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let json = dir.join("harness.json").to_string_lossy().to_string();
    // The test's own files are written after this binary was built, so the
    // binary's time refuses none of them; a build time in the future (the
    // year 2100) stands in for an earlier build's files.
    let built = std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::metadata(exe).and_then(|meta| meta.modified()).ok());
    let later_build = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(4_102_444_800));
    let fresh = |word: &str, thinks: u32| {
        serde_json::json!({
            "cohort": if word == "stock_ci" { "ci" } else { "authored" },
            "variant": word,
            "minds": { "thinks": thinks, "faults": 0, "engine": "rom", "final_substep_hash": "7" },
        })
    };
    let states = serde_json::json!({
        "approach": 1, "disabled": 2, "engage": 3, "patrol": 4, "startup": 0, "withdraw": 5,
        "unknown": 0,
    });
    // No files, no baseline block: skipped.
    let bare = serde_json::json!({ "world": { "units": 300 } });
    let (lines, differences) = minds_delta(&bare, &json, built);
    assert_eq!(differences, 0);
    assert!(
        lines[0].starts_with("harness_delta: minds block: the baseline carries none; skipped"),
        "{lines:?}"
    );
    // 17b cut M4.5: the arena report's own `minds.states_at_end` is no variant
    // block — a baseline carrying it alone skips too, rather than reading four
    // missing variants.
    let arena_report = serde_json::json!({
        "world": { "units": 300 },
        "minds": { "states_at_end": states.clone() },
    });
    let (lines, differences) = minds_delta(&arena_report, &json, built);
    assert_eq!(differences, 0, "{lines:?}");
    assert!(
        lines[0].starts_with("harness_delta: minds block: the baseline carries none; skipped"),
        "{lines:?}"
    );
    assert_eq!(
        minds_blocks_beside(&json, built).map_err(|refused| refused.len()),
        Err(4),
        "no files, nothing to fold, and every one named"
    );
    // Three of four files: nothing to fold yet, the missing one named; the
    // delta against a baseline that carries the block names it as one
    // difference.
    for word in &MINDS_VARIANT_WORDS[..3] {
        write_report(
            &arena_report_path(&json, &format!("minds-{word}")),
            &fresh(word, 10),
        );
    }
    let refused = minds_blocks_beside(&json, built).expect_err("a partial set never folds");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].starts_with("harness_delta: minds.ceiling_authored no fresh report at"),
        "{refused:?}"
    );
    write_report(
        &arena_report_path(&json, "minds-ceiling_authored"),
        &fresh("ceiling_authored", 10),
    );
    let folded = minds_blocks_beside(&json, built).expect("all four fold");
    assert_eq!(
        folded["stock_ci"],
        serde_json::json!({ "thinks": 10, "faults": 0, "engine": "rom", "final_substep_hash": "7", "cohort": "ci" })
    );
    // 17b cut M4.5: files an earlier build wrote are refused, every one named,
    // by the bless's fold and by the delta alike.
    let refused = minds_blocks_beside(&json, later_build).expect_err("stale files never fold");
    assert_eq!(refused.len(), 4, "{refused:?}");
    assert!(
        refused
            .iter()
            .all(|why| why.contains(" refused: ") && why.contains("older than this test binary")),
        "{refused:?}"
    );
    let mut baseline = arena_report.clone();
    baseline["minds"]["fill_authored"] = folded["fill_authored"].clone();
    baseline["minds"]["stock_ci"] = folded["stock_ci"].clone();
    baseline["minds"]["stock_authored"] = folded["stock_authored"].clone();
    baseline["minds"]["ceiling_authored"] = folded["ceiling_authored"].clone();
    let (lines, differences) = minds_delta(&baseline, &json, later_build);
    assert_eq!(differences, 4, "{lines:?}");
    let (lines, differences) = minds_delta(&baseline, &json, built);
    assert_eq!(differences, 0, "{lines:?}");
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("block equal (5 fields)"))
            .count(),
        4,
        "{lines:?}"
    );
    // One leaf moved on one variant: one difference, named.
    write_report(
        &arena_report_path(&json, "minds-fill_authored"),
        &fresh("fill_authored", 11),
    );
    let (lines, differences) = minds_delta(&baseline, &json, built);
    assert_eq!(differences, 1, "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|line| line == "harness_delta: minds.fill_authored.thinks baseline=10 now=11"),
        "{lines:?}"
    );
    // A variant file gone: one difference, named.
    std::fs::remove_file(arena_report_path(&json, "minds-stock_authored")).expect("removed");
    let (lines, differences) = minds_delta(&baseline, &json, built);
    assert_eq!(differences, 2, "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("harness_delta: minds.stock_authored no fresh report at")),
        "{lines:?}"
    );
    assert!(minds_blocks_beside(&json, built).is_err());
    // The bless merges: the fresh arena report's `minds.states_at_end` is
    // kept beside the four variant blocks, and kept when none are folded.
    let blessed = dir.join("arena.testos.json");
    let read = |path: &std::path::Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("written")).expect("json")
    };
    bless_baseline(&blessed, &arena_report, Some(&folded));
    let written = read(&blessed);
    assert_eq!(written["minds"]["ceiling_authored"]["thinks"], 10);
    assert_eq!(written["minds"]["states_at_end"], states);
    assert_eq!(
        written["minds"].as_object().map(serde_json::Map::len),
        Some(5),
        "the states and the four variants, nothing else"
    );
    assert!(written.get("blessed").is_some());
    // A fresh report that (wrongly) carried a variant word loses it to the fold.
    let mut carrying = arena_report.clone();
    carrying["minds"]["stock_ci"] = serde_json::json!({ "thinks": 99 });
    bless_baseline(&blessed, &carrying, Some(&folded));
    assert_eq!(read(&blessed)["minds"]["stock_ci"]["thinks"], 10);
    // Without the fold: the variant words go, the states stay.
    bless_baseline(&blessed, &carrying, None);
    let written = read(&blessed);
    assert_eq!(
        written["minds"],
        serde_json::json!({ "states_at_end": states }),
        "no variant block, the arena's own object kept"
    );
    // A report with no `minds` at all blesses none.
    bless_baseline(&blessed, &bare, None);
    assert_eq!(
        read(&blessed).get("minds"),
        None,
        "no block, nothing folded"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The minds lane runs small** (17b cut M4.2; `z1_the_lance_lane_runs_small`'s
/// shape): sixteen hulls, four fighting, five arena seconds, the default seed,
/// the ceiling variant then the fill variant in a debug build — the
/// installer, the seal override, the recording runners, the private pool, the
/// load mind on heavies at the last two slots a side, the task's policy
/// through [`run_minds_wakes`] — with every deterministic check asserted
/// (`held=yes sound=yes`) and the timing checks printed only
/// ([`MindsLaneMode::Smoke`]: debug timings mean nothing). Each variant's
/// authored slots are held to think on the engine they are sealed on: the
/// ceiling's scripted load mind in Rhai, and the fill's slots on
/// `PRODUCTION_FILL_ENGINE` — native since the fill decision of 2026-10-09,
/// so natively above the ROM with no Rhai think (it read the Rhai arm while
/// the fill was scripted). Since 16c `T-108` the per-wake lens's split is read
/// off the engaged and verdict lines and the report's tempo, its two counts
/// summing to the wakes and the jittered one the deterministic block's
/// `jittered_wakes`, and each line's figures and counts the report's
/// (`wake_p99_ms` the unjittered p99, `wake_p99_other_ms` the jittered one).
#[test]
fn m4_2_the_minds_lane_runs_small() {
    let mut knobs = Knobs::from_env();
    knobs.units = 16;
    knobs.fighting = 4;
    knobs.arena_s = 5;
    knobs.seed = DEFAULT_SEED;
    knobs.jitter_ms = 0;
    knobs.assert = true;
    knobs.snapshot_off = false;
    knobs.json = std::env::temp_dir()
        .join("tacsim-minds-smoke")
        .join("harness.json")
        .to_string_lossy()
        .to_string();
    let minds = MindsKnobs {
        jitter_ms: i64::from(DEFAULT_MINDS_JITTER_MS),
        jitter_per_mille: DEFAULT_MINDS_JITTER_PER_MILLE,
        workers: 2,
        cpu_lens_ms: 5,
        variants: MINDS_VARIANTS.to_vec(),
    };
    let reports = measure_minds(
        &knobs,
        &minds,
        MindsLaneMode::Smoke,
        &[MindsVariant::CeilingAuthored, MindsVariant::FillAuthored],
    );
    assert_eq!(reports.len(), 2);
    for report in &reports {
        let word = report.variant.word();
        for prefix in [
            "harness_minds_world:",
            "harness_minds_pool:",
            "harness_minds_thinks:",
            "harness_minds_ops:",
            "harness_minds_cpu:",
            "harness_minds_engaged:",
            "harness_minds_window:",
            "harness_minds_verdict:",
            "harness_minds_json:",
        ] {
            assert!(
                report
                    .lines
                    .iter()
                    .any(|line| line.starts_with(prefix)
                        && line.contains(&format!("variant={word} "))),
                "no `{prefix}` line for {word}: {:#?}",
                report.lines
            );
        }
        assert!(
            report.held && report.sound,
            "{word}: held={} sound={}: {:?} {:?}",
            report.held,
            report.sound,
            report.failures,
            report.unsound
        );
        assert_eq!(report.faults, 0, "{word}");
        assert!(
            report.inline_thinks > 0 && report.pooled_thinks > 0,
            "{word}: inline={} pooled={}",
            report.inline_thinks,
            report.pooled_thinks
        );
        let engine = report
            .variant
            .authored()
            .map(|slots| slots.engine)
            .expect("both smoke variants field authored slots");
        match engine {
            tacsim::arena::SealEngine::Scripted => assert!(
                report.rhai_thinks > 0,
                "{word}: the scripted slots thought in Rhai"
            ),
            tacsim::arena::SealEngine::Native => assert!(
                report.rhai_thinks == 0 && report.native_minded_thinks > 0,
                "{word}: the native slots thought natively above the ROM and never in Rhai \
                 (rhai={} native_minded={})",
                report.rhai_thinks,
                report.native_minded_thinks
            ),
            tacsim::arena::SealEngine::Reflex => panic!("{word}: no slot is sealed reflex"),
        }
        let stem = report
            .json
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        assert_eq!(stem, format!("harness-minds-{word}"));
        assert!(report.json.exists(), "{}", report.json.display());
    }
    let ceiling = &reports[0];
    assert_eq!(ceiling.variant, MindsVariant::CeilingAuthored);
    assert!(
        (LOAD_III_OPS_P1_MIN..=LOAD_III_OPS_MAX).contains(&ceiling.budget_p1)
            && ceiling.budget_max <= LOAD_III_OPS_MAX,
        "the load mind's band: p1 {} max {}",
        ceiling.budget_p1,
        ceiling.budget_max
    );
    assert!(
        ceiling
            .lines
            .iter()
            .any(|line| line.starts_with("harness_minds_pool:")
                && line.contains(" admitted_per_side=8 fielded_per_side=2 ")),
        "{:#?}",
        ceiling.lines
    );
    // The lines' `held=` word is the report's.
    for report in &reports {
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.starts_with("harness_minds_verdict:")
                    && line.contains(" held=yes sound=yes ")
                    && line.contains(" mode=smoke ")),
            "{:#?}",
            report.lines
        );
    }
    // The per-wake lens's split (16c `T-108`): on the engaged and the verdict
    // lines and in the report's tempo, the two counts summing to the wakes,
    // the jittered one the deterministic block's `jittered_wakes`.
    for report in &reports {
        for (prefix, words) in [
            (
                "harness_minds_engaged:",
                [
                    " unjittered=",
                    " p99_unjittered=",
                    " jittered=",
                    " p99_jittered=",
                ],
            ),
            (
                "harness_minds_verdict:",
                [
                    " wake_p99_ms=",
                    " wake_unjittered=",
                    " wake_p99_other_ms=",
                    " wake_jittered=",
                ],
            ),
        ] {
            assert!(
                report
                    .lines
                    .iter()
                    .any(|line| line.starts_with(prefix)
                        && words.iter().all(|word| line.contains(word))),
                "{prefix} {words:?}: {:#?}",
                report.lines
            );
        }
        let json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&report.json).expect("the report is written"),
        )
        .expect("the report is JSON");
        let tempo = &json["tempos"][0];
        let count = |key: &str| tempo[key].as_u64().unwrap_or(u64::MAX);
        assert_eq!(
            count("unjittered").saturating_add(count("jittered")),
            count("pulses"),
            "{tempo}"
        );
        assert!(count("unjittered") > 0, "{tempo}");
        assert_eq!(
            Some(count("jittered")),
            json["minds"]["jittered_wakes"].as_u64(),
            "{}",
            json["minds"]
        );
        assert!(tempo["p99_ms_unjittered"].is_number() && tempo["p99_ms_jittered"].is_number());
        // The lines' figures are the report's, from the one run — the line
        // printed to three decimals and the report rounded to three, so equal
        // within one in the third, whatever debug timings read: the verdict's
        // `wake_p99_ms` is the unjittered p99 and `wake_p99_other_ms` the
        // jittered one (not every wake's, not swapped), the engaged line
        // carries the same split beside every wake's `p99`, and the counts on
        // both lines are the report's.
        let line_of = |prefix: &str| -> String {
            report
                .lines
                .iter()
                .find(|line| line.starts_with(prefix))
                .cloned()
                .unwrap_or_else(|| panic!("no `{prefix}` line: {:#?}", report.lines))
        };
        let word_of = |line: &str, key: &str| -> String {
            line.split_whitespace()
                .find_map(|word| word.strip_prefix(key)?.strip_prefix('='))
                .map(str::to_string)
                .unwrap_or_else(|| panic!("no `{key}=` word: {line}"))
        };
        let verdict = line_of("harness_minds_verdict:");
        let engaged = line_of("harness_minds_engaged:");
        for (line, key, json_key) in [
            (&verdict, "wake_p99_ms", "p99_ms_unjittered"),
            (&verdict, "wake_p99_other_ms", "p99_ms_jittered"),
            (&engaged, "p99_unjittered", "p99_ms_unjittered"),
            (&engaged, "p99_jittered", "p99_ms_jittered"),
            (&engaged, "p99", "p99_ms"),
        ] {
            let on_line: f64 = word_of(line, key)
                .parse()
                .unwrap_or_else(|_| panic!("`{key}=` is not a figure: {line}"));
            let in_report = tempo[json_key].as_f64().unwrap_or(f64::NAN);
            assert!(
                (on_line - in_report).abs() <= 0.0011,
                "`{key}={on_line}` on the line against the report's `{json_key}` {in_report}: {line}"
            );
        }
        for (line, key, json_key) in [
            (&verdict, "wake_unjittered", "unjittered"),
            (&verdict, "wake_jittered", "jittered"),
            (&engaged, "unjittered", "unjittered"),
            (&engaged, "jittered", "jittered"),
            (&engaged, "pulses", "pulses"),
        ] {
            let on_line: u64 = word_of(line, key)
                .parse()
                .unwrap_or_else(|_| panic!("`{key}=` is not a count: {line}"));
            assert_eq!(
                on_line,
                count(json_key),
                "`{key}=` against `{json_key}`: {line}"
            );
        }
    }
}
