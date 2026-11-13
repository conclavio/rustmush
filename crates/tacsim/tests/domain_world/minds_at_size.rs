//! **Doc 44 unit MW.20 — minds measured at size** (Q-44-34, Q-44-48).
//!
//! Four `#[ignore]` measurements, run by hand in the release profile and by no job. Each
//! prints `MW20` lines under `--nocapture` and asserts only that its own rig is sound (the
//! field is fought, the bubbles opened, every think ran clean): **no figure is a gate**. The
//! figures of the recorded runs, with the host and what misses, are in
//! `docs/design/44-cut-notes.md` under MW.20. The simulator match's half of the unit is the
//! fill harness's minds lane (`tests/harness.rs`, `tacsim_harness_arena_minds` at
//! `TACSIM_UNITS=100`, with its `rhai_authored` sizing variant for the Rhai seats), and the
//! squads' spend at the LLM is that file's cost run on the mock provider.
//!
//! ```text
//! cargo test --release -p tacsim --test domain_world minds_at_size:: -- --ignored --nocapture --test-threads=1
//! ```
//!
//! **1. `mw20_a_field_of_minds_at_size`: an IC field.** GC.12's construction
//! (`tests/domain_arena/field_scale.rs`): Ahuna Flank's own ground priced as the field host
//! prices it, `MW20_UNITS` (100) hulls of the mixed cohort in `MW20_PARTIES` parties (3: the
//! miners, their escort and a raider, the first two friends and the raider marked by both;
//! 6: GC.12's own six), every hull a mind. `MW20_RHAI` of them (0) are sealed at Class I on
//! the scripted engine with Standard Doctrine, dealt round the parties; the rest are the
//! production fill (the native doctrine). The blocks stand 18 cells apart, the raider's
//! between the miners' and the escort's: a hull with no order holds its ground and fights
//! what comes into its reach, so parties stood further apart never fight (three blocks 31
//! cells apart stayed at `Alert` tempo for 180 arena seconds with nobody firing). It is
//! opened as `arena_task::spawn_field` opens
//! one (every hull running, the field's own duration) and stepped **on the task's policy**:
//! the thinks a wake collects are handed to a pool of `MW20_WORKERS` threads
//! (`[tacsim.minds].workers`) after the step, awaited and landed at the next wake's head, and
//! awaited before a due snapshot's encode. Each wake's step is timed alone, as the task's
//! `took` is; the landing, the wait and the snapshot are timed beside it. With
//! `MW20_LATE_PER_MILLE` a seeded share of wakes is one lattice interval late, which runs the
//! first interval's thinks inline inside the step (the minds lane's jitter). With
//! `MW20_REAL=1` the loop sleeps to each wake's due time, as the task does, and says how far
//! behind it woke. `MW20_RUNS` (3) runs of each configuration; one line a run and one for
//! the runs together (medians, and the worst).
//!
//! **2. `mw20_eight_bubbles_think_on_the_pool`: space.** One world for each pool size of
//! `MW20_POOLS` (`1,2,4`), because a world builds its pool once. `MW20_BUBBLES` (8) pilots
//! each charter a hull at Ceres, load, board and clear for a different venue in the same IC
//! second, all through the commands a player types, on a lane seeded with a customs picket on
//! each of those routes (`voyage::flown_into_pickets`; the shipped lane interdicts one leg,
//! which is one bubble however many hulls fly it, and a test may not write the bubble book).
//! The eight cross their pickets on one tick, so eight picket bubbles open together, each on
//! its own domain with its two cast minds, the corvette and the raider. Eight is also
//! `mush_sim::MAX_ACTIVE_DOMAINS`. The crews stay linked and give no order, which leaves
//! every bubble open for as long as the lane runs. With `MW20_BACKUP=1` every crew's link is
//! dropped and the grace waited out, so the house backup mind flies each hull, a third mind
//! a bubble; it heaves her to, customs rule, and the bubbles close about thirty ticks on.
//! The host is told to hold late steps, as the
//! server's world loop tells it, and the world is ticked a second at a time for `MW20_TICKS`
//! (120) ticks, `MW20_PACE_MS` (1000) of wall time apart. Each tick's module ticks are fired
//! one at a time through the engine's own `fire_due` (the scheduler is drained and each due
//! entry put back alone), so the space tick's cost on the world thread is timed apart from
//! the port's and the rest. `MW20_PACE_MS=0` runs the ticks back to back, which is where a
//! pool that is too small shows as held steps. The lane also counts the store writes a
//! second leaves, and sizes one bubble's record and times a copy and an encoding of it on
//! this thread, outside the tick: the space tick copies the book and encodes every bubble
//! that stepped, every second, and the figure says how much of the tick that is.
//!
//! **3. `mw20_a_field_and_eight_bubbles_on_one_server`: both.** One world, one pool (the
//! store's, which a match and the space host share in production): lane 2's eight bubbles
//! ticked a wall second apart on this thread, which stands for the world thread, while lane
//! 1's field is stepped in real time on a thread of its own, which stands for its task, for
//! `MW20_TOGETHER_S` (60) seconds.
//!
//! **4. `mw20_an_idle_fire_by_tick`: the floor.** The stocked world at the live rate (24:1),
//! nobody connected, nothing in flight: `MW20_FIRES` (600) seconds, each module tick's fire
//! timed alone as in lane 2. PR #576 measured this in a test build (doc 43 cut notes); this
//! is the release figure.
//!
//! Nothing here changes game code. Three things are a test's and not a player's: the space
//! lane's lane is seeded with seven more picket waypoints, the engine's scheduler is drained
//! and refilled round each tick so each module tick is fired alone, and the field's arena is
//! built by hand as GC.12 builds it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use mush_core::{GOD, ObjId, ObjKind, ScheduledKind};
use mush_testkit::TestWorld;
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::mind::{MindRunner, ThinkJob, ThinkResult};
use tacsim::arena::party::{EndCondition, FieldHost, Mark, PartyId};
use tacsim::arena::state::{Arena, CustodyOut};
use tacsim::definitions::content;
use tacsim::minds::{BatchHandle, MindPool};

use crate::driving_rig::AHUNA;
use crate::support;
use crate::voyage::{
    Rig, Roster, drain_space_and_industry, drop_link, flown_into_pickets, land_quietly, set_rate,
    stocked_with, store,
};

// ---------------------------------------------------------------------------
// Knobs and figures
// ---------------------------------------------------------------------------

/// An environment knob, or its default.
fn knob<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// A comma list of numbers.
fn list<T: std::str::FromStr>(text: &str) -> Vec<T> {
    text.split(',')
        .filter_map(|word| word.trim().parse().ok())
        .collect()
}

/// The build's profile word.
fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// The host's name and its cores, for the head line of each lane.
fn host() -> String {
    let name = std::fs::read_to_string("/etc/hostname")
        .map_or_else(|_| "unknown".to_owned(), |name| name.trim().to_owned());
    let cores = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    format!("{name}/{cores}")
}

/// The one-minute load average, where the host has one.
fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "na".to_owned())
}

/// Milliseconds since `from`.
fn ms_since(from: Instant) -> f64 {
    from.elapsed().as_secs_f64() * 1_000.0
}

/// The median, the 95th and 99th percentiles and the worst of some samples.
#[derive(Clone, Copy, Debug, Default)]
struct Spread {
    n: usize,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

impl Spread {
    fn of(samples: impl IntoIterator<Item = f64>) -> Spread {
        let mut sorted: Vec<f64> = samples.into_iter().collect();
        sorted.sort_by(f64::total_cmp);
        let at = |percent: usize| {
            sorted
                .get((sorted.len() * percent / 100).min(sorted.len().saturating_sub(1)))
                .copied()
                .unwrap_or(0.0)
        };
        Spread {
            n: sorted.len(),
            p50: at(50),
            p95: at(95),
            p99: at(99),
            max: sorted.last().copied().unwrap_or(0.0),
        }
    }
}

impl std::fmt::Display for Spread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.3}/{:.3}/{:.3}/{:.3} (n={})",
            self.p50, self.p95, self.p99, self.max, self.n
        )
    }
}

/// The median of a few runs' figures, and the worst of them.
fn over_runs(values: &[f64]) -> (f64, f64) {
    let spread = Spread::of(values.iter().copied());
    (spread.p50, spread.max)
}

// ---------------------------------------------------------------------------
// The field
// ---------------------------------------------------------------------------

/// GC.12's cohort, in the order a party's hulls take it.
const COHORT: [&str; 8] = [
    content::LINE_TANK,
    content::LINE_TANK,
    content::LINE_TANK_LASER,
    content::BIPED_MK1,
    content::LINE_TANK,
    content::LINE_TANK_THERMAL,
    content::LINE_TANK_ECM,
    content::LINE_TANK_ECCM,
];

/// GC.12's ring: six block centres 18 cells out from the pad, each 18 cells from the next.
const RING: [(i64, i64); 6] = [(18, 0), (9, 16), (-9, 16), (-18, 0), (-9, -16), (9, -16)];

/// **Where a party's block stands.** Six parties take the ring in order, as GC.12 stands
/// them. With fewer, the raider (the last party) takes the second place and the others the
/// places round it, so the raider is 18 cells from the miners and from their escort.
fn block_of(party: usize, parties: usize) -> (i64, i64) {
    if parties == RING.len() {
        RING[party]
    } else if party == parties - 1 {
        RING[1]
    } else {
        RING[[0, 2, 3, 4, 5][party]]
    }
}

/// How long the opening is left out of the figures, arena milliseconds: every mind's first
/// think is staggered across its own period, and the first contacts are made.
const SETTLE_MS: i64 = 5_000;

/// The windows the wakes are folded into, arena milliseconds (the harness's).
const WINDOW_MS: i64 = 500;

/// **The opening relations of `parties` parties.** Six are GC.12's own (two pairs of
/// friends, marks that are not symmetric, a raider marking everyone). Any other count is an
/// IC field as doc 27 has it: the first party the miners, the second their escort and their
/// friends, the last a raider who marks every party and is marked by every party; a party
/// between them minds only the raider.
fn opening_host(parties: usize) -> FieldHost {
    let party = |at: usize| PartyId::nth(at).expect("a party");
    let (marks, friends) = if parties == 6 {
        let (a, b, c, d, e, f) = (party(0), party(1), party(2), party(3), party(4), party(5));
        let mut marks = vec![
            (a, c, Mark::Hostile),
            (a, d, Mark::Hostile),
            (a, f, Mark::Hostile),
            (b, c, Mark::Hostile),
            (b, f, Mark::Hostile),
            (c, a, Mark::Hostile),
            (c, b, Mark::Hostile),
            (d, a, Mark::Hostile),
            (e, f, Mark::Hostile),
        ];
        for other in [a, b, c, d, e] {
            marks.push((f, other, Mark::Hostile));
        }
        (marks, vec![(a, b), (c, d)])
    } else {
        let raider = party(parties - 1);
        let mut marks = Vec::new();
        for other in (0..parties - 1).map(party) {
            marks.push((other, raider, Mark::Hostile));
            marks.push((raider, other, Mark::Hostile));
        }
        (marks, vec![(party(0), party(1))])
    };
    FieldHost {
        all_hostile: false,
        marks,
        friends,
        end: EndCondition::Host,
        every_class: true,
        weapons_hold: true,
        tows: Vec::new(),
    }
}

/// A field checked out and not yet opened: the custody, and the source each scripted seal
/// names.
struct Fielded {
    custody: CustodyOut,
    sources: BTreeMap<[u8; 32], String>,
}

/// **The field's custody** (GC.12's `field_arena`, with the parties and the seals a
/// parameter): `n` hulls on Ahuna Flank in `parties` blocks round the pad, the first `rhai`
/// of them (dealt round the parties, since hull `i` is of party `i % parties`) sealed at
/// Class I on the scripted engine, the rest the production fill.
fn field_custody(tw: &mut TestWorld, n: usize, parties: usize, rhai: usize) -> Fielded {
    use tacsim::arena::state::{ArenaDefinition, ArenaTerrain, TerrainCell, Waypoints};
    assert!(
        (3..=RING.len()).contains(&parties),
        "MW20_PARTIES is 3 to 6: {parties}"
    );
    let site = mush_core::CanonicalId::new(AHUNA).expect("a canonical site id");
    let w = tw.world();
    let (composite, biome) =
        tacsim::surface::ground::site_terrain_and_biome(w, &site).expect("the site's terrain");
    let ground = composite
        .surface(&mush_atlas::SurfaceRef::Ground)
        .expect("a ground surface");
    let (width, height) = (ground.cells.width(), ground.cells.height());
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
    for y in 0..height {
        for x in 0..width {
            let cell = ground.cells.get(x, y).expect("a cell");
            cells.push(TerrainCell {
                class: cell.class,
                elev_cm: cell.elev_cm,
                move_cost: table
                    .cost(cell.class.label(), "tracked", &[])
                    .map_or(0, |cost| u16::try_from(cost).unwrap_or(u16::MAX)),
            });
        }
    }
    let open = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && x < i64::from(width)
            && y < i64::from(height)
            && usize::try_from(y * i64::from(width) + x)
                .ok()
                .and_then(|at| cells.get(at))
                .is_some_and(|cell| cell.move_cost > 0)
    };
    let pad = tacsim::surface::excursion::site_pad_cell(w, &site);
    let mut taken = BTreeSet::new();
    let mut placed = Vec::with_capacity(n);
    for i in 0..n {
        let party = i % parties;
        let rank = i / parties;
        let (cx, cy) = block_of(party, parties);
        // A block is rows of five, two cells apart, growing away from the pad's row.
        let want = (
            pad.0 + cx + (i64::try_from(rank % 5).expect("small") - 2) * 2,
            pad.1 + cy + (i64::try_from(rank / 5).expect("small") - 1) * 2,
        );
        let cell = (0_i64..16)
            .flat_map(|r| (-r..=r).flat_map(move |dx| (-r..=r).map(move |dy| (r, dx, dy))))
            .filter(|(r, dx, dy)| dx.abs().max(dy.abs()) == *r)
            .map(|(_, dx, dy)| (want.0 + dx, want.1 + dy))
            .find(|c| open(c.0, c.1) && !taken.contains(c))
            .expect("a free cell near the block");
        taken.insert(cell);
        placed.push((i, party, rank, cell));
    }
    let terrain = ArenaTerrain::priced(width, height, composite.frame.cell_cm, cells, table)
        .expect("a priced window")
        .with_biome(biome);
    let definition = ArenaDefinition {
        id: SmolStr::new(format!("field:{AHUNA}:mw20:{n}:{parties}:{rhai}")),
        terrain,
        entry_zones: Vec::new(),
        key_spots: Vec::new(),
        waypoints: Waypoints::new(Vec::new(), &[]),
        modes: Vec::new(),
        homes: Vec::new(),
    };
    let catalog = store(tw).definitions().clone();
    let fill = tacsim::match_::SealTerms::production_fixture();
    let scripted = tacsim::match_::SealTerms {
        rated: tacsim::arena::core::CoreClass::I,
        class_cap: tacsim::arena::core::CoreClass::I,
        engine_for_minds: tacsim::arena::SealEngine::Scripted,
        ..tacsim::match_::SealTerms::fixture()
    };
    let mut units = Vec::with_capacity(n);
    let mut sources = BTreeMap::new();
    for (i, party, rank, cell) in placed {
        let entry = COHORT[rank % COHORT.len()];
        let spec = catalog
            .bindings
            .get(&mush_core::CanonicalId::new(entry).expect("a canonical entry id"))
            .map(|binding| binding.spec.clone())
            .unwrap_or_else(|| panic!("{entry} is bound in the fielded catalog"));
        let id = support::with_ctx(tw, |ctx| {
            let obj = ctx
                .w
                .create(ObjKind::Thing, "Scale stand-in", GOD, None)
                .expect("the stand-in");
            mush_tactics::module::create_unit(ctx, obj, &spec).expect("its unit record");
            obj
        });
        let record = mush_tactics::module::unit_of(tw.world(), id)
            .expect("the unit")
            .clone();
        let pose = tacsim::surface::excursion::cell_pose([cell.0, cell.1], 0).expect("a pose");
        let unit = tacsim::match_::custody_unit(
            &catalog,
            record,
            id,
            PartyId::nth(party).expect("a party"),
            pose,
            0,
            tacsim::arena::state::Controller::Mind,
            u8::try_from(rank + 1).expect("a slot"),
            if i < rhai { &scripted } else { &fill },
        )
        .expect("the hull seals");
        if unit.seal.engine == tacsim::arena::SealEngine::Scripted {
            assert_eq!(
                unit.seal.source_sha256,
                tacsim::minds::standard_digest().0,
                "a scripted seat is sealed on Standard Doctrine"
            );
            sources.insert(unit.seal.digest(), tacsim::minds::STANDARD_RHAI.to_owned());
        }
        units.push(unit);
    }
    Fielded {
        custody: CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: 2350,
            mode: tacsim::arena::MatchMode::Skirmish,
            host: opening_host(parties),
        },
        sources,
    }
}

/// **A runner with every think's wall time kept**: the lane's clock round the game's own
/// runner, which it calls and changes nothing of.
#[derive(Debug)]
struct Timed {
    inner: Arc<dyn MindRunner>,
    us: Arc<Mutex<Vec<u64>>>,
}

impl MindRunner for Timed {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let at = Instant::now();
        let result = self.inner.think(job);
        let us = u64::try_from(at.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.us
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(us);
        result
    }
}

/// The samples a [`Timed`] runner kept, emptied.
fn take(us: &Mutex<Vec<u64>>) -> Vec<u64> {
    std::mem::take(&mut *us.lock().unwrap_or_else(PoisonError::into_inner))
}

/// A field opened and ready to step.
struct Opened {
    arena: Arena,
    arena_cfg: tacsim::config::ArenaConfig,
    /// Each native think's microseconds (the production fill and the ROM).
    native_us: Arc<Mutex<Vec<u64>>>,
    /// Each Rhai think's microseconds.
    rhai_us: Arc<Mutex<Vec<u64>>>,
    /// The Rhai seats' one recording runner, for the operations they spent.
    recorder: Option<Arc<tacsim::minds::RhaiRunner>>,
    rhai_seats: usize,
}

/// **Open the field as `arena_task::spawn_field` opens one**: the world's own mind limits,
/// the field's duration, every hull running, each scripted seal's runner installed by the
/// production installer; then the lane's clocks put round the runners.
fn open_field(fielded: Fielded) -> Opened {
    let arena_cfg = tacsim::config::ArenaConfig::default();
    let match_cfg = tacsim::config::MatchConfig {
        max_duration_ms: tacsim::arena_bridge::FIELD_MAX_DURATION_MS,
        ..tacsim::config::MatchConfig::default()
    };
    let Fielded { custody, sources } = fielded;
    let mut arena = Arena::new_with_minds(
        custody,
        &arena_cfg,
        &match_cfg,
        &tacsim::config::SensorsConfig::default(),
        &tacsim::config::MindsConfig::default(),
    )
    .expect("the field's arena checks out");
    for unit in &mut arena.units {
        unit.power = tacsim::arena::state::Power::Online;
    }
    let installed = tacsim::minds::install_sealed_runners(&mut arena, &arena_cfg, &|digest| {
        sources.get(digest).cloned()
    })
    .expect("every scripted seal's runner installs");
    let native_us = Arc::new(Mutex::new(Vec::new()));
    let rhai_us = Arc::new(Mutex::new(Vec::new()));
    let recorder = (installed > 0).then(|| {
        Arc::new(
            tacsim::minds::RhaiRunner::new(
                &arena_cfg,
                tacsim::minds::STANDARD_RHAI,
                tacsim::minds::standard_digest(),
            )
            .expect("Standard Doctrine compiles")
            .recording(),
        )
    });
    if let Some(recorder) = &recorder {
        let timed: Arc<dyn MindRunner> = Arc::new(Timed {
            inner: Arc::clone(recorder) as Arc<dyn MindRunner>,
            us: Arc::clone(&rhai_us),
        });
        for runner in arena.runners.values_mut() {
            *runner = Arc::clone(&timed);
        }
    }
    arena.mind_runner = Arc::new(Timed {
        inner: Arc::clone(&arena.mind_runner),
        us: Arc::clone(&native_us),
    });
    Opened {
        arena,
        arena_cfg,
        native_us,
        rhai_us,
        recorder,
        rhai_seats: installed,
    }
}

/// How the wakes are paced.
#[derive(Clone, Copy, Debug)]
struct Pace {
    /// Sleep to each wake's due time, as the task does; otherwise the wall is the lattice.
    real: bool,
    /// The share of wakes one lattice interval late, per mille (never with `real`).
    late_per_mille: u32,
}

/// One wake.
#[derive(Clone, Copy, Debug)]
struct Wake {
    arena_ms: i64,
    /// `Arena::step` alone: the task's `took`.
    step_ms: f64,
    /// Landing the last wake's answers at this wake's head, the wait not counted.
    land_ms: f64,
    /// How long the head waited for the last wake's batch, microseconds.
    wait_us: Option<u64>,
    /// How long the next wake was away when this one's batch was handed over, milliseconds.
    gap_ms: i64,
    tempo: Tempo,
    late: bool,
    /// Thinks this wake collected, and how many of them went to the pool.
    thinks: u32,
    pooled: u32,
    /// How far behind its due time the wake woke (`real` only), milliseconds.
    behind_ms: f64,
    lagged: bool,
}

/// What a drive produced.
#[derive(Debug, Default)]
struct Driven {
    wakes: Vec<Wake>,
    /// Each snapshot's encode, milliseconds, and its bytes.
    snapshots: Vec<(f64, usize)>,
    faults: u64,
    overflows: u64,
    failed_batches: u32,
    ledger_bytes: u64,
    ended: Option<String>,
}

/// **Step `arena` until `until_ms` on the field task's policy** (`arena_task.rs`,
/// `Task::pulse`): at the head the batch the last wake handed to the pool is awaited and its
/// answers landed; the step is timed alone; the chunks are acknowledged as the world
/// acknowledges them; the wake's thinks are handed to the pool; and a due snapshot waits for
/// that batch and is encoded. `stop` ends it early (the lane that runs it beside the
/// bubbles).
fn drive(
    arena: &mut Arena,
    pool: &MindPool,
    until_ms: i64,
    pace: Pace,
    stop: Option<&AtomicBool>,
) -> Driven {
    let mut driven = Driven::default();
    let snapshot_ms = arena.arena_cfg.snapshot_ms;
    let mut last_snapshot = None;
    let mut outstanding: Option<BatchHandle> = None;
    // Await and land a batch; `(wait_us, land_ms)`.
    let land = |arena: &mut Arena, handle: BatchHandle, driven: &mut Driven| -> (u64, f64) {
        let at = Instant::now();
        match handle.wait() {
            Ok(answers) => {
                let wait_us = u64::try_from(at.elapsed().as_micros()).unwrap_or(u64::MAX);
                let at = Instant::now();
                let settled = arena.deliver_answers(answers);
                driven.faults += u64::from(settled.faults);
                driven.overflows += u64::from(settled.overflows);
                (wait_us, ms_since(at))
            }
            Err(_) => {
                driven.failed_batches += 1;
                arena.discard_outstanding();
                (
                    u64::try_from(at.elapsed().as_micros()).unwrap_or(u64::MAX),
                    0.0,
                )
            }
        }
    };
    let base = Instant::now();
    let mut due = 0_i64;
    let mut interval = 0_i64;
    let mut index = 0_u32;
    while arena.now_ms < until_ms && !stop.is_some_and(|stop| stop.load(Ordering::Relaxed)) {
        let (wall, behind_ms, late) = if pace.real {
            let target = base + Duration::from_millis(u64::try_from(due).unwrap_or(0));
            if let Some(ahead) = target.checked_duration_since(Instant::now()) {
                std::thread::sleep(ahead);
            }
            let elapsed = ms_since(base);
            #[allow(clippy::cast_possible_truncation)]
            let wall = elapsed as i64;
            #[allow(clippy::cast_precision_loss)]
            let behind = (elapsed - due as f64).max(0.0);
            (wall.max(due), behind, false)
        } else {
            let late = pace.late_per_mille > 0
                && interval > 0
                && tacsim::arena::rng::stagger_ms(0x4d57_3230, ObjId(index), 1_000)
                    < i64::from(pace.late_per_mille);
            (if late { due + interval } else { due }, 0.0, late)
        };
        let (mut wait_us, mut land_ms) = (None, 0.0);
        if let Some(handle) = outstanding.take() {
            let (waited, landed) = land(arena, handle, &mut driven);
            wait_us = Some(waited);
            land_ms = landed;
        }
        let at = Instant::now();
        let mut out = arena.step(wall, &[], &[]);
        let step_ms = ms_since(at);
        driven.faults += u64::from(out.motor.mind_faults);
        driven.overflows += u64::from(out.motor.mind_overflows);
        for chunk in &out.ledger_chunks {
            driven.ledger_bytes += u64::try_from(chunk.bytes.len()).unwrap_or(u64::MAX);
        }
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        let thinks = std::mem::take(&mut out.thinks);
        let pooled = u32::try_from(thinks.len()).unwrap_or(u32::MAX);
        if !thinks.is_empty() {
            outstanding = Some(pool.submit(thinks));
        }
        if out.ended.is_none()
            && tacsim::arena_task::snapshot_due(&mut last_snapshot, arena.now_ms, snapshot_ms)
        {
            if let Some(handle) = outstanding.take() {
                let (waited, landed) = land(arena, handle, &mut driven);
                wait_us = Some(wait_us.unwrap_or(0).saturating_add(waited));
                land_ms += landed;
            }
            let at = Instant::now();
            let snapshot = tacsim::arena::Snapshot::encode(arena).expect("the field encodes");
            driven
                .snapshots
                .push((ms_since(at), snapshot.bytes().len()));
        }
        interval = (out.next_due_ms - wall).max(1);
        driven.wakes.push(Wake {
            arena_ms: arena.now_ms,
            step_ms,
            land_ms,
            wait_us,
            gap_ms: interval,
            tempo: out.tempo,
            late,
            thinks: out.motor.thinks,
            pooled,
            behind_ms,
            lagged: out.lagged,
        });
        index = index.wrapping_add(1);
        if let Some(end) = &out.ended {
            driven.ended = Some(format!("{end:?}"));
            break;
        }
        due = out.next_due_ms.max(wall + 1);
    }
    if let Some(handle) = outstanding.take() {
        let _ = land(arena, handle, &mut driven);
    }
    driven
}

/// One run's figures, and whether it held.
#[derive(Clone, Debug, Default)]
struct FieldFigures {
    step: Spread,
    engaged: Spread,
    window_max_ms: f64,
    window_p99_ms: f64,
    wait_us: Spread,
    late_batches: usize,
    cpu_window_p99_ms: f64,
    behind: Spread,
    held: bool,
    misses: Vec<String>,
}

/// **Read one run and print its line.** What *held* means here, each a reason when it does
/// not: the `Engaged` wakes' step is within `[tacsim.harness].engaged_p99_ms` at the 99th
/// percentile (the budget GC.12 set the field's cap against); no wake is past
/// `tempo.stall_ms` (the watchdog would void the field); no think faulted and no batch
/// failed; every batch was back before the next wake fell due (a batch that is not makes
/// that wake wait, which is the arena's rule for a late pool); the Rhai seats' operations in
/// any window are inside the pool `match_::pool` allots a field of this size; and, paced in
/// real time, no pulse lagged.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
fn read_field(label: &str, opened: &Opened, driven: &Driven, units: usize) -> FieldFigures {
    let budget = f64::from(tacsim::config::HarnessConfig::default().engaged_p99_ms);
    let stall = f64::from(opened.arena_cfg.tempo.stall_ms);
    let wakes: Vec<&Wake> = driven
        .wakes
        .iter()
        .filter(|wake| wake.arena_ms > SETTLE_MS)
        .collect();
    let step = Spread::of(wakes.iter().map(|wake| wake.step_ms));
    let engaged = Spread::of(
        wakes
            .iter()
            .filter(|wake| wake.tempo == Tempo::Engaged)
            .map(|wake| wake.step_ms),
    );
    let on_time = Spread::of(
        wakes
            .iter()
            .filter(|wake| !wake.late)
            .map(|wake| wake.step_ms),
    );
    let late = Spread::of(
        wakes
            .iter()
            .filter(|wake| wake.late)
            .map(|wake| wake.step_ms),
    );
    let land = Spread::of(wakes.iter().map(|wake| wake.land_ms));
    let mut windows: BTreeMap<i64, f64> = BTreeMap::new();
    for wake in &wakes {
        *windows.entry(wake.arena_ms / WINDOW_MS).or_default() += wake.step_ms + wake.land_ms;
    }
    let window = Spread::of(windows.values().copied());
    let wait_us = Spread::of(
        wakes
            .iter()
            .filter_map(|wake| wake.wait_us)
            .map(|us| us as f64),
    );
    // A batch handed over at wake k is awaited at wake k + 1's head: it is late when its
    // wait outlasted the gap wake k left.
    let late_batches = driven
        .wakes
        .windows(2)
        .filter(|pair| {
            pair[1]
                .wait_us
                .is_some_and(|us| us / 1_000 > u64::try_from(pair[0].gap_ms).unwrap_or(0))
        })
        .count();
    let native = take(&opened.native_us);
    let rhai = take(&opened.rhai_us);
    let native_us = Spread::of(native.iter().map(|us| *us as f64));
    let rhai_us = Spread::of(rhai.iter().map(|us| *us as f64));
    let arena_s = (opened.arena.now_ms as f64 / 1_000.0).max(1.0);
    let cpu_ms: f64 = native.iter().chain(&rhai).map(|us| *us as f64).sum::<f64>() / 1_000.0;
    // The thinks' CPU a window, taken as even over the run: the pool's threads' time, not
    // the task's.
    let cpu_window_ms = cpu_ms / arena_s * (WINDOW_MS as f64 / 1_000.0);
    let stats = opened
        .recorder
        .as_ref()
        .map(|recorder| recorder.take_stats())
        .unwrap_or_default();
    let ops: u64 = stats
        .iter()
        .map(|stat| stat.ops.saturating_add(stat.fee))
        .sum();
    let ops_per_s = ops as f64 / arena_s;
    let pool_ops_s = tacsim::match_::pool::pool_for(&tacsim::config::MindsConfig::default(), units);
    let snapshot_ms = Spread::of(driven.snapshots.iter().map(|(ms, _)| *ms));
    let snapshot_bytes = driven.snapshots.iter().map(|(_, bytes)| *bytes).max();
    let behind = Spread::of(wakes.iter().map(|wake| wake.behind_ms));
    let lagged = wakes.iter().filter(|wake| wake.lagged).count();
    let tempo_count = |tempo: Tempo| wakes.iter().filter(|wake| wake.tempo == tempo).count();
    let thinks: u64 = wakes.iter().map(|wake| u64::from(wake.thinks)).sum();
    let pooled: u64 = wakes.iter().map(|wake| u64::from(wake.pooled)).sum();
    let standing = opened
        .arena
        .units
        .iter()
        .filter(|unit| !unit.limits.wreck)
        .count();

    let mut misses = Vec::new();
    if engaged.n == 0 {
        misses.push("no Engaged wake: the budget was not measured".to_owned());
    } else if engaged.p99 > budget {
        misses.push(format!(
            "engaged step p99 {:.3} ms is past {budget} ms",
            engaged.p99
        ));
    }
    if step.max > stall {
        misses.push(format!(
            "a wake took {:.1} ms, past stall_ms {stall}",
            step.max
        ));
    }
    if driven.faults > 0 || driven.failed_batches > 0 {
        misses.push(format!(
            "{} thinks faulted and {} batches failed",
            driven.faults, driven.failed_batches
        ));
    }
    if late_batches > 0 {
        misses.push(format!(
            "{late_batches} batches were not back when the next wake fell due"
        ));
    }
    if ops_per_s > pool_ops_s as f64 {
        misses.push(format!(
            "{ops_per_s:.0} Rhai ops a second is past the pool's {pool_ops_s}"
        ));
    }
    if lagged > 0 {
        misses.push(format!("{lagged} pulses lagged"));
    }
    let held = misses.is_empty();
    println!(
        "MW20 field {label}: {} wakes over {:.0} arena s (quiet {} / alert {} / engaged {}), \
         {standing} of {units} standing, ended={}; step ms p50/p95/p99/max all {step} | engaged \
         {engaged} | on time {on_time} | a late wake {late}; landing ms {land}; {WINDOW_MS} ms \
         window (step and landing) ms {window}; budget {budget} ms engaged p99, stall {stall} \
         ms; thinks {thinks} ({pooled} pooled, {} inline), native us {native_us}, rhai us \
         {rhai_us} over {} seats, think cpu {cpu_ms:.1} ms = {cpu_window_ms:.3} ms a window; \
         rhai ops {ops} = {ops_per_s:.0} a second against the pool's {pool_ops_s}; batch wait \
         us {wait_us}, {late_batches} late; snapshot encode ms {snapshot_ms}, {} B; ledger {} \
         B; behind ms {behind}, lagged {lagged}; faults {} overflows {} failed batches {}; \
         state hash {:016x}; held={} {}",
        wakes.len(),
        arena_s,
        tempo_count(Tempo::Quiet),
        tempo_count(Tempo::Alert),
        tempo_count(Tempo::Engaged),
        driven.ended.as_deref().unwrap_or("no"),
        thinks.saturating_sub(pooled),
        opened.rhai_seats,
        snapshot_bytes.unwrap_or(0),
        driven.ledger_bytes,
        driven.faults,
        driven.overflows,
        driven.failed_batches,
        tacsim::arena::hash::state_hash(&opened.arena),
        if held { "yes" } else { "no" },
        misses.join("; "),
    );
    FieldFigures {
        step,
        engaged,
        window_max_ms: window.max,
        window_p99_ms: window.p99,
        wait_us,
        late_batches,
        cpu_window_p99_ms: cpu_window_ms,
        behind,
        held,
        misses,
    }
}

/// The runs of one configuration, as one line: the median of the runs and the worst.
#[allow(clippy::cast_precision_loss)]
fn summarize(label: &str, runs: &[FieldFigures]) {
    let of = |f: &dyn Fn(&FieldFigures) -> f64| {
        let values: Vec<f64> = runs.iter().map(f).collect();
        let (median, worst) = over_runs(&values);
        format!("{median:.3} (worst {worst:.3})")
    };
    let held = runs.iter().filter(|run| run.held).count();
    let misses: BTreeSet<&str> = runs
        .iter()
        .flat_map(|run| run.misses.iter().map(String::as_str))
        .collect();
    println!(
        "MW20 field summary {label}: runs={} held={held}/{}; step ms p50 {} p99 {} max {}; \
         engaged step ms p50 {} p99 {} max {}; window ms p99 {} max {}; batch wait us p50 {} \
         p99 {} max {}; late batches {}; think cpu ms a window {}; behind ms p99 {} max {}; \
         misses: {}",
        runs.len(),
        runs.len(),
        of(&|run| run.step.p50),
        of(&|run| run.step.p99),
        of(&|run| run.step.max),
        of(&|run| run.engaged.p50),
        of(&|run| run.engaged.p99),
        of(&|run| run.engaged.max),
        of(&|run| run.window_p99_ms),
        of(&|run| run.window_max_ms),
        of(&|run| run.wait_us.p50),
        of(&|run| run.wait_us.p99),
        of(&|run| run.wait_us.max),
        of(&|run| run.late_batches as f64),
        of(&|run| run.cpu_window_p99_ms),
        of(&|run| run.behind.p99),
        of(&|run| run.behind.max),
        if misses.is_empty() {
            "none".to_owned()
        } else {
            misses.into_iter().collect::<Vec<_>>().join("; ")
        },
    );
}

/// **Doc 44 unit MW.20, lane 1: an IC field of minds at size** (the module's notes).
#[test]
#[ignore = "MW.20 measurement lane: by hand, in release, with --nocapture"]
fn mw20_a_field_of_minds_at_size() {
    let units: usize = knob("MW20_UNITS", 100);
    let parties: Vec<usize> = list(&knob("MW20_PARTIES", "3,6".to_owned()));
    let rhai: Vec<usize> = list(&knob("MW20_RHAI", "0".to_owned()));
    let arena_s: i64 = knob("MW20_ARENA_S", 120);
    let runs: usize = knob("MW20_RUNS", 3);
    let workers: u32 = knob(
        "MW20_WORKERS",
        tacsim::config::MindsConfig::default().workers,
    );
    let pace = Pace {
        real: knob("MW20_REAL", 0_u8) == 1,
        late_per_mille: knob("MW20_LATE_PER_MILLE", 0),
    };
    println!(
        "MW20 field build={} host={} load={} units={units} parties={parties:?} rhai={rhai:?} \
         arena_s={arena_s} runs={runs} workers={workers} pace={pace:?}",
        profile(),
        host(),
        load_average(),
    );
    for &parties in &parties {
        for &rhai in &rhai {
            let label = format!("units={units} parties={parties} rhai={rhai} workers={workers}");
            let mut figures = Vec::new();
            for run in 0..runs {
                // A fresh world a run, so a run's hulls, and so its field, are a function of
                // the configuration alone.
                let mut rig = stocked_with(Roster::default());
                let fielded = field_custody(&mut rig.tw, units, parties, rhai.min(units));
                let mut opened = open_field(fielded);
                let pool = MindPool::new(workers);
                let driven = drive(&mut opened.arena, &pool, arena_s * 1_000, pace, None);
                assert_eq!(driven.failed_batches, 0, "no runner panicked");
                let figure = read_field(&format!("{label} run={run}"), &opened, &driven, units);
                figures.push(figure);
            }
            summarize(&label, &figures);
        }
    }
}

// ---------------------------------------------------------------------------
// Module ticks, one at a time
// ---------------------------------------------------------------------------

/// What one scheduled entry is called in the tables.
fn label_of(kind: &ScheduledKind) -> String {
    match kind {
        ScheduledKind::ModuleTick { module, tag } => format!("{module}:{tag}"),
        ScheduledKind::SceneTick { module, tag, .. } => format!("{module}:{tag} (scene)"),
        ScheduledKind::WaitCmd { .. } => "a waited command".to_owned(),
        ScheduledKind::CommandContinuation(_) => "a command continuation".to_owned(),
        ScheduledKind::ConnectionCompletion(_) => "a connection completion".to_owned(),
        ScheduledKind::Control(name) => format!("control:{name}"),
        ScheduledKind::RequestTimeout { .. } => "a request timeout".to_owned(),
    }
}

/// **One wall second, each due entry fired alone.** The world's clock moves a second; the
/// scheduler is drained of everything due, and each entry is put back by itself and fired
/// through `TestWorld::fire_due`, which is the engine's own `fire_due` with its outbox and
/// its effects applied, so a tick runs exactly as it does in a whole fire and re-arms
/// itself. What a tick makes due at once is fired after it under its own name. Answers
/// each entry's name and milliseconds, in firing order, and how many store writes the
/// second left (drained, as the server folds them into its next batch, so a long lane does
/// not keep them).
fn second_by_tick(rig: &mut Rig) -> (Vec<(String, f64)>, usize) {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let now_ms = t.saturating_mul(1_000);
    let mut fired = Vec::new();
    let mut due: std::collections::VecDeque<ScheduledKind> = std::collections::VecDeque::new();
    // Bounded, so an entry that re-arms itself for this same millisecond cannot spin the lane.
    for _ in 0..4_096 {
        // `pop_due` hands over at most a step's worth at a time; what the last entry made
        // due at once joins the back of the line.
        due.extend(rig.tw.engine_mut().sched.pop_due(now_ms));
        let Some(kind) = due.pop_front() else {
            break;
        };
        let label = label_of(&kind);
        let _ = rig.tw.engine_mut().sched.insert(now_ms, kind);
        let at = Instant::now();
        let _ = rig.tw.fire_due(now_ms);
        fired.push((label, ms_since(at)));
    }
    assert!(due.is_empty(), "the second's entries were all fired");
    let queued = drain_space_and_industry(&mut rig.tw);
    land_quietly(rig, &queued);
    let writes = rig.tw.take_module_ops().len();
    (fired, writes)
}

/// One wall second fired whole, timed.
fn second_whole(rig: &mut Rig) -> f64 {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let at = Instant::now();
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let ms = ms_since(at);
    let queued = drain_space_and_industry(&mut rig.tw);
    land_quietly(rig, &queued);
    drop(rig.tw.take_module_ops());
    ms
}

/// Every tick's fires over some seconds.
#[derive(Default)]
struct ByTick(BTreeMap<String, Vec<f64>>);

impl ByTick {
    fn absorb(&mut self, fired: Vec<(String, f64)>) {
        for (label, ms) in fired {
            self.0.entry(label).or_default().push(ms);
        }
    }

    /// The table: a line a tick, the costliest a wall second first, and the sum.
    #[allow(clippy::cast_precision_loss)]
    fn print(&self, head: &str, seconds: usize) {
        let mut rows: Vec<(&str, Spread, f64)> = self
            .0
            .iter()
            .map(|(label, ms)| {
                (
                    label.as_str(),
                    Spread::of(ms.iter().copied()),
                    ms.iter().sum::<f64>() / seconds.max(1) as f64,
                )
            })
            .collect();
        rows.sort_by(|a, b| b.2.total_cmp(&a.2));
        let total: f64 = rows.iter().map(|row| row.2).sum();
        println!(
            "MW20 {head}: {seconds} s, {} ticks, {total:.3} ms a wall second in all",
            rows.len()
        );
        for (label, spread, per_s) in rows {
            let mean = self
                .0
                .get(label)
                .map_or(0.0, |ms| ms.iter().sum::<f64>() / ms.len().max(1) as f64);
            println!(
                "MW20 {head}:   {label:<44} fires {:>4}  ms a fire mean {mean:>8.3} p50 {:>8.3} \
                 p99 {:>8.3} max {:>8.3}  ms a wall second {per_s:>8.3}",
                spread.n, spread.p50, spread.p99, spread.max
            );
        }
    }
}

/// **Doc 44 unit MW.20, lane 4: what an idle fire of each module tick costs** (the
/// module's notes).
#[test]
#[ignore = "MW.20 measurement lane: by hand, in release, with --nocapture"]
fn mw20_an_idle_fire_by_tick() {
    let fires: usize = knob("MW20_FIRES", 600);
    let rate: u32 = knob("MW20_RATE", 24);
    let mut rig = stocked_with(Roster::default());
    set_rate(&mut rig.tw, rate);
    println!(
        "MW20 idle build={} host={} load={} fires={fires} rate={rate}:1 ticks registered {}",
        profile(),
        host(),
        load_average(),
        rig.tw.engine().registry.module_ticks().len(),
    );
    // Whole fires first: what a second costs when it is fired as the engine fires it.
    for _ in 0..30 {
        let _ = second_whole(&mut rig);
    }
    let whole: Vec<f64> = (0..fires).map(|_| second_whole(&mut rig)).collect();
    #[allow(clippy::cast_precision_loss)]
    let whole_mean = whole.iter().sum::<f64>() / whole.len().max(1) as f64;
    println!(
        "MW20 idle whole: a wall second's fire ms mean {whole_mean:.3} p50/p95/p99/max {}",
        Spread::of(whole.iter().copied())
    );
    // Then the same seconds with each tick fired alone.
    let mut by_tick = ByTick::default();
    for _ in 0..fires {
        by_tick.absorb(second_by_tick(&mut rig).0);
    }
    by_tick.print("idle by tick", fires);
}

// ---------------------------------------------------------------------------
// Space
// ---------------------------------------------------------------------------

/// Whether a table name is the space tick's.
fn is_space_tick(label: &str) -> bool {
    label
        .rsplit_once(':')
        .is_some_and(|(_, tag)| tag == tacsim::space::tick::SPACE_TICK)
}

/// The bubbles open now, and the steps each has taken.
fn bubbles(tw: &TestWorld) -> Vec<u32> {
    tw.world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .map(|book| book.by_domain.values().map(|active| active.steps).collect())
        .unwrap_or_default()
}

/// **A world on a pool of `workers` threads with `count` picket bubbles open** (the
/// module's notes), every one opened through production commands. With `backup`, every
/// crew's link is dropped and the grace waited out inside the bubbles, so the house backup
/// mind has each helm. The world is left at the live rate, its host holding late steps as
/// the server's does.
fn eight_bubbles(workers: u32, count: usize, backup: bool) -> Rig {
    let (mut rig, crews) = flown_into_pickets(workers, count);
    if backup {
        for (pilot, _) in &crews {
            drop_link(&mut rig, *pilot);
        }
        for _ in 0..3 {
            let _ = second_whole(&mut rig);
        }
        let past = rig
            .tw
            .world()
            .now()
            .saturating_add(tacsim::space::SPACE_LINKDEAD_GRACE_WALL_S);
        rig.tw.set_now(past);
        let _ = rig.tw.fire_due(past.saturating_mul(1_000));
        let handed = drain_space_and_industry(&mut rig.tw);
        land_quietly(&mut rig, &handed);
        // The tick the grace runs out on collects each mind's first think; the next lands it.
        let _ = second_whole(&mut rig);
        for (_, hull) in &crews {
            let flown = rig
                .tw
                .world()
                .module_state::<tacsim::space::SpaceBubbleBook>()
                .and_then(|book| book.linkdead_hulls.get(hull))
                .is_some_and(tacsim::space::SpaceLinkdeadState::backup_has_helm);
            assert!(flown, "past the grace the backup mind has her helm");
        }
    }
    set_rate(&mut rig.tw, 24);
    assert_eq!(bubbles(&rig.tw).len(), count, "{count} bubbles are open");
    tacsim::space::thinks::run_on_the_wall_clock(rig.tw.world());
    rig
}

/// What a run of ticks over the bubbles came to.
struct SpaceRun {
    /// The space tick's milliseconds on the world thread, a tick with every bubble open.
    space_ms: Vec<f64>,
    /// Everything else that fired in those seconds, milliseconds a second.
    rest_ms: Vec<f64>,
    by_tick: ByTick,
    ticks: usize,
    /// Ticks that began with every bubble open.
    full_ticks: usize,
    /// How many bubbles were open at the head of each tick.
    open: Vec<usize>,
    /// Steps the bubbles took in the ticks that began with every bubble open.
    steps: u64,
    stats: tacsim::space::thinks::ThinkStats,
    queued_peak: usize,
    /// Store writes the full ticks left.
    writes: usize,
    /// One bubble's record at the end ([`record_cost`]).
    record: Option<RecordCost>,
}

/// **What one bubble's record costs to copy and to encode**, on this thread and outside
/// the tick: its encoded bytes, and the median microseconds of a clone and of
/// `serde_json::to_value`, which is what the space tick's persistence does for every
/// bubble that stepped.
#[derive(Clone, Copy, Debug)]
struct RecordCost {
    bytes: usize,
    clone_us: f64,
    encode_us: f64,
}

/// See [`RecordCost`].
fn record_cost(tw: &TestWorld) -> Option<RecordCost> {
    let book = tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()?;
    let active = book.by_domain.values().next()?;
    let bytes = serde_json::to_vec(active).ok()?.len();
    let mut clone_us = Vec::new();
    let mut encode_us = Vec::new();
    for _ in 0..50 {
        let at = Instant::now();
        std::hint::black_box(active.clone());
        clone_us.push(ms_since(at) * 1_000.0);
        let at = Instant::now();
        std::hint::black_box(serde_json::to_value(active).ok());
        encode_us.push(ms_since(at) * 1_000.0);
    }
    Some(RecordCost {
        bytes,
        clone_us: Spread::of(clone_us).p50,
        encode_us: Spread::of(encode_us).p50,
    })
}

/// **Tick the world a second at a time over its bubbles**, `pace` of wall time apart, until
/// `ticks` are done or `stop` says so; each due entry fired alone ([`second_by_tick`]).
fn tick_bubbles(
    rig: &mut Rig,
    count: usize,
    ticks: usize,
    pace: Duration,
    stop: Option<&AtomicBool>,
) -> SpaceRun {
    let host = store(&rig.tw).space_thinks();
    let pool = store(&rig.tw).mind_pool();
    let before = host.stats();
    let steps_before: u64 = bubbles(&rig.tw).iter().map(|steps| u64::from(*steps)).sum();
    let mut run = SpaceRun {
        space_ms: Vec::new(),
        rest_ms: Vec::new(),
        by_tick: ByTick::default(),
        ticks: 0,
        full_ticks: 0,
        open: Vec::new(),
        steps: 0,
        stats: tacsim::space::thinks::ThinkStats::default(),
        queued_peak: 0,
        writes: 0,
        record: None,
    };
    let mut steps_last = steps_before;
    let base = Instant::now();
    for tick in 0..ticks {
        if stop.is_some_and(|stop| stop.load(Ordering::Relaxed)) {
            break;
        }
        let target = base + pace * u32::try_from(tick).unwrap_or(u32::MAX);
        if let Some(ahead) = target.checked_duration_since(Instant::now()) {
            std::thread::sleep(ahead);
        }
        let open = bubbles(&rig.tw).len();
        let (fired, writes) = second_by_tick(rig);
        run.queued_peak = run.queued_peak.max(pool.queued());
        run.ticks += 1;
        run.open.push(open);
        let steps_now: u64 = bubbles(&rig.tw).iter().map(|steps| u64::from(*steps)).sum();
        // A bubble that closes takes its steps out of the sum, so only a tick that began
        // with every bubble open is counted.
        let stepped = steps_now.saturating_sub(steps_last);
        steps_last = steps_now;
        if open == count {
            run.full_ticks += 1;
            run.steps += stepped;
            run.writes += writes;
            let space_ms: f64 = fired
                .iter()
                .filter(|(label, _)| is_space_tick(label))
                .map(|(_, ms)| *ms)
                .sum();
            let all: f64 = fired.iter().map(|(_, ms)| *ms).sum();
            run.space_ms.push(space_ms);
            run.rest_ms.push(all - space_ms);
            run.by_tick.absorb(fired);
        }
    }
    run.record = record_cost(&rig.tw);
    let after = host.stats();
    run.stats = tacsim::space::thinks::ThinkStats {
        thinks: after.thinks - before.thinks,
        batches: after.batches - before.batches,
        held: after.held - before.held,
        runners_built: after.runners_built,
    };
    run
}

/// One line for a run over the bubbles.
#[allow(clippy::cast_precision_loss)]
fn print_space(label: &str, count: usize, run: &SpaceRun) {
    let due = run.full_ticks * count;
    // The bubbles open at each tick's head, as runs of one count: `8x31 0x29`.
    let mut timeline: Vec<(usize, usize)> = Vec::new();
    for open in &run.open {
        match timeline.last_mut() {
            Some((count, ticks)) if count == open => *ticks += 1,
            _ => timeline.push((*open, 1)),
        }
    }
    let timeline = timeline
        .iter()
        .map(|(open, ticks)| format!("{open}x{ticks}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "MW20 space {label}: {} ticks (bubbles open by tick: {timeline}), {} with all {count} \
         open; the space tick on the world thread ms p50/p95/p99/max {}; the rest of the \
         second's ticks ms {}; steps taken in those ticks {} of {due} due; thinks {} in {} \
         batches over every tick = {:.1} a tick; held steps {} = {:.2} per cent of steps due; \
         runners built {}; pool queue peak {}; store writes {:.1} a tick; one bubble's record \
         {}",
        run.ticks,
        run.full_ticks,
        Spread::of(run.space_ms.iter().copied()),
        Spread::of(run.rest_ms.iter().copied()),
        run.steps,
        run.stats.thinks,
        run.stats.batches,
        run.stats.thinks as f64 / run.ticks.max(1) as f64,
        run.stats.held,
        run.stats.held as f64 * 100.0 / (due.max(1)) as f64,
        run.stats.runners_built,
        run.queued_peak,
        run.writes as f64 / run.full_ticks.max(1) as f64,
        run.record.map_or_else(
            || "na".to_owned(),
            |record| format!(
                "{} B, a clone {:.1} us, an encoding {:.1} us",
                record.bytes, record.clone_us, record.encode_us
            )
        ),
    );
}

/// **Doc 44 unit MW.20, lane 2: eight bubbles thinking on the pool** (the module's notes).
#[test]
#[ignore = "MW.20 measurement lane: by hand, in release, with --nocapture"]
fn mw20_eight_bubbles_think_on_the_pool() {
    let count: usize = knob("MW20_BUBBLES", 8);
    let pools: Vec<u32> = list(&knob("MW20_POOLS", "1,2,4".to_owned()));
    let ticks: usize = knob("MW20_TICKS", 120);
    let pace = Duration::from_millis(knob("MW20_PACE_MS", 1_000));
    let backup = knob("MW20_BACKUP", 0_u8) == 1;
    let runs: usize = knob("MW20_RUNS", 3);
    println!(
        "MW20 space build={} host={} load={} bubbles={count} pools={pools:?} ticks={ticks} \
         pace_ms={} backup={backup} runs={runs} shipped workers={}",
        profile(),
        host(),
        load_average(),
        pace.as_millis(),
        tacsim::config::MindsConfig::default().workers,
    );
    for &workers in &pools {
        for run in 0..runs {
            let mut rig = eight_bubbles(workers, count, backup);
            let host = store(&rig.tw).space_thinks();
            assert_eq!(host.workers(), usize::try_from(workers).unwrap_or(0));
            assert!(host.holds_late_steps(), "the host holds, as the server's");
            let measured = tick_bubbles(&mut rig, count, ticks, pace, None);
            assert!(measured.stats.thinks > 0, "the bubbles' minds thought");
            print_space(
                &format!("workers={workers} pace_ms={} run={run}", pace.as_millis()),
                count,
                &measured,
            );
            if run == 0 {
                measured.by_tick.print(
                    &format!("space by tick workers={workers}"),
                    measured.full_ticks,
                );
            }
        }
    }
}

/// **Doc 44 unit MW.20, lane 3: a field and eight bubbles on one server** (the module's
/// notes).
#[test]
#[ignore = "MW.20 measurement lane: by hand, in release, with --nocapture"]
fn mw20_a_field_and_eight_bubbles_on_one_server() {
    let units: usize = knob("MW20_UNITS", 100);
    let parties: usize = knob("MW20_TOGETHER_PARTIES", 3);
    let rhai: usize = knob("MW20_TOGETHER_RHAI", 0);
    let count: usize = knob("MW20_BUBBLES", 8);
    let seconds: usize = knob("MW20_TOGETHER_S", 60);
    let backup = knob("MW20_BACKUP", 0_u8) == 1;
    let runs: usize = knob("MW20_RUNS", 3);
    let workers: u32 = knob(
        "MW20_WORKERS",
        tacsim::config::MindsConfig::default().workers,
    );
    println!(
        "MW20 together build={} host={} load={} units={units} parties={parties} rhai={rhai} \
         bubbles={count} seconds={seconds} backup={backup} runs={runs} workers={workers}",
        profile(),
        host(),
        load_average(),
    );
    let mut figures = Vec::new();
    for run in 0..runs {
        let mut rig = eight_bubbles(workers, count, backup);
        let fielded = field_custody(&mut rig.tw, units, parties, rhai.min(units));
        let mut opened = open_field(fielded);
        // The server's one pool: the space host's, and the one a match is handed.
        let pool = tacsim::minds::pool::of_world(rig.tw.world());
        assert!(Arc::ptr_eq(&pool, &store(&rig.tw).mind_pool()));
        let stop = AtomicBool::new(false);
        let until_ms = i64::try_from(seconds).unwrap_or(0) * 1_000;
        let (driven, measured) = std::thread::scope(|scope| {
            let field = scope.spawn(|| {
                let driven = drive(
                    &mut opened.arena,
                    &pool,
                    until_ms,
                    Pace {
                        real: true,
                        late_per_mille: 0,
                    },
                    Some(&stop),
                );
                stop.store(true, Ordering::Relaxed);
                driven
            });
            let measured = tick_bubbles(
                &mut rig,
                count,
                seconds,
                Duration::from_secs(1),
                Some(&stop),
            );
            stop.store(true, Ordering::Relaxed);
            (field.join().expect("the field's thread"), measured)
        });
        assert_eq!(driven.failed_batches, 0, "no runner panicked");
        let label = format!(
            "together units={units} parties={parties} rhai={rhai} bubbles={count} \
             workers={workers} run={run}"
        );
        figures.push(read_field(&label, &opened, &driven, units));
        print_space(&label, count, &measured);
    }
    summarize(
        &format!("together units={units} parties={parties} rhai={rhai} workers={workers}"),
        &figures,
    );
}
