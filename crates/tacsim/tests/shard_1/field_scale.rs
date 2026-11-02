//! **Doc 36 cut GC.12 — the cap, measured** (Q-36-9).
//!
//! One `#[ignore]` measurement, never run by the ordinary shards: by hand or through gate.yml's
//! `sweep` dispatch with `--run-ignored only -E 'test(/^field_scale::/)'` (`--cargo-profile
//! release` for the figures the cap is set from). It fields an IC field's arena on Ahuna Flank's
//! own priced ground (as doc 32 cut S.2's `field_story::story_arena` does) at each size of
//! `GC12_SIZES` (default `9,24,48,64,80,100`: S.2's nine as the control, today's cap, Q-36-9's two
//! sizes and two between them), every unit a stock mind from a mixed cohort, in **six parties under the IC host's
//! terms** — friendships and asymmetric opening marks, so the relation books are not two sides
//! and grow with the fire — and steps each for `GC12_ARENA_S` (default 120) arena seconds two
//! ways:
//!
//! - **lattice**: at `StepOutput::next_due_ms`, the loop the field task runs. The verdict lane:
//!   each wake timed, and every wake folded into 500 ms windows of arena time;
//! - **fixed100**: 100 ms pulses, S.2's loop, comparable with S.2's printed figures.
//!
//! What it prints under `--nocapture`, one `GC12` line per size and lane: pulses, shots, units
//! standing at the end; pulse median, p95, p99 and max (all wakes and `Engaged` wakes alone); the
//! worst 500 ms window; the snapshot at the opening and the end; and the relation cost — contacts
//! held at the end, the parties' books encoded, and the timed cost of
//! `Arena::hostile_in_weapons_range` (the tempo test every pulse runs) and of a full sweep of every
//! held contact through `Relations::of_class` (doctrine's and the frames' read).
//!
//! The budget the cap is set against is `[tacsim.harness].engaged_p99_ms`, 40 ms a pulse in
//! `Engaged` tempo on the production class of host (16c `T-109`). `GC12_ASSERT=1` turns it into
//! an assertion on the lattice lane; the default is report-only, because a shared runner's speed
//! is not a threshold (D-H-8).

use mush_core::{GOD, ObjKind};
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::party::{EndCondition, FieldHost, Mark, PartyId};
use tacsim::arena::state::Arena;
use tacsim::definitions::content;

use crate::driving::{AHUNA, Rig, cid, staff_hand, stocked};

/// The cohort, in the order a party's units take it: tracked, laser, legged, thermal, ECM, ECCM.
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

/// Six parties, `A` to `F`.
const PARTIES: usize = 6;

/// **The opening relations** of the six parties: two pairs of friends by roster, `(A, B)` and
/// `(C, D)`, and marks that are not symmetric — `B` holds `D` neutral while `D` holds `A`
/// hostile, `E` minds only `F`, and `F` is a raider marking everyone (Q-CF-44's fixture rule).
fn opening_host(party: impl Fn(usize) -> PartyId) -> FieldHost {
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
    FieldHost {
        all_hostile: false,
        marks,
        friends: vec![(a, b), (c, d)],
        end: EndCondition::Host,
        every_class: true,
        weapons_hold: true,
    }
}

/// An environment knob, or its default.
fn knob<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// **The field's arena with `n` units on Ahuna Flank** (doc 32 cut S.2's construction): the
/// site's terrain priced as the field host prices it, and the six parties stood in a ring of
/// blocks 18 cells out from the pad, each unit on the nearest free cell a tracked hull may enter.
fn field_arena(rig: &mut Rig, n: usize) -> Arena {
    use tacsim::arena::state::{ArenaDefinition, ArenaTerrain, TerrainCell, Waypoints};
    let w = rig.tw.world();
    let (composite, biome) =
        tacsim::surface::ground::site_terrain_and_biome(w, &cid(AHUNA)).expect("the terrain");
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
    let pad = tacsim::surface::excursion::site_pad_cell(w, &cid(AHUNA));
    let mut taken = std::collections::BTreeSet::new();
    let mut placed = Vec::with_capacity(n);
    for i in 0..n {
        let party = i % PARTIES;
        let rank = i / PARTIES;
        // The party's block centre on a ring of 18 cells, 60 degrees apart; its units two cells
        // apart in rows of five.
        let (cx, cy) = [(18, 0), (9, 16), (-9, 16), (-18, 0), (-9, -16), (9, -16)][party];
        let want = (
            pad.0 + cx + (i64::try_from(rank % 5).expect("small") - 2) * 2,
            pad.1 + cy + (i64::try_from(rank / 5).expect("small") - 1) * 2,
        );
        let cell = (0_i64..12)
            .flat_map(|r| (-r..=r).flat_map(move |dx| (-r..=r).map(move |dy| (r, dx, dy))))
            .filter(|(r, dx, dy)| dx.abs().max(dy.abs()) == *r)
            .map(|(_, dx, dy)| (want.0 + dx, want.1 + dy))
            .find(|c| open(c.0, c.1) && !taken.contains(c))
            .expect("a free cell near the block");
        taken.insert(cell);
        placed.push((party, rank, cell));
    }
    let terrain = ArenaTerrain::priced(width, height, composite.frame.cell_cm, cells, table)
        .expect("a priced window")
        .with_biome(biome);
    let definition = ArenaDefinition {
        id: SmolStr::new(format!("field:{AHUNA}:gc12:{n}")),
        terrain,
        entry_zones: Vec::new(),
        key_spots: Vec::new(),
        waypoints: Waypoints::new(Vec::new(), &[]),
        modes: Vec::new(),
        homes: Vec::new(),
    };
    let catalog = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .map(|store| store.definitions().clone())
        .expect("the layer");
    let party_id = |at: usize| PartyId::nth(at).expect("a party");
    let mut units = Vec::with_capacity(n);
    for (party, rank, cell) in placed {
        let entry = COHORT[rank % COHORT.len()];
        let spec = catalog
            .bindings
            .get(&cid(entry))
            .map(|binding| binding.spec.clone())
            .unwrap_or_else(|| panic!("{entry} is bound in the fielded catalog"));
        let id = staff_hand(rig, |ctx| {
            let obj = ctx
                .w
                .create(ObjKind::Thing, "Scale stand-in", GOD, None)
                .expect("the stand-in");
            mush_tactics::module::create_unit(ctx, obj, &spec).expect("its unit record");
            obj
        });
        let record = mush_tactics::module::unit_of(rig.tw.world(), id)
            .expect("the unit")
            .clone();
        let pose = tacsim::surface::excursion::cell_pose([cell.0, cell.1], 0).expect("a pose");
        units.push(
            tacsim::match_::custody_unit(
                &catalog,
                record,
                id,
                party_id(party),
                pose,
                0,
                tacsim::arena::state::Controller::Mind,
                u8::try_from(rank + 1).expect("a slot"),
                &tacsim::match_::SealTerms::production_fixture(),
            )
            .expect("the production fill seals"),
        );
    }
    let custody = tacsim::arena::state::CustodyOut {
        definition,
        units,
        pilots: Vec::new(),
        resolution: tacsim::match_::resolution_for(&catalog),
        seed: 2350,
        mode: tacsim::arena::MatchMode::Skirmish,
        host: opening_host(party_id),
    };
    Arena::new(
        custody,
        &tacsim::config::ArenaConfig::default(),
        &tacsim::config::MatchConfig::default(),
        &tacsim::config::SensorsConfig::default(),
    )
    .expect("the field's arena checks out")
}

/// The `p`-th percentile of sorted samples.
fn rank(sorted: &[f64], p: usize) -> f64 {
    sorted
        .get((sorted.len() * p / 100).min(sorted.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0.0)
}

/// `median / p95 / p99 / max` of some samples, milliseconds.
fn spread(samples: &[f64]) -> String {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    format!(
        "{:.3} / {:.3} / {:.3} / {:.3}",
        rank(&sorted, 50),
        rank(&sorted, 95),
        rank(&sorted, 99),
        sorted.last().copied().unwrap_or(0.0)
    )
}

/// The shots one pulse's ledger carries.
fn shots_in(out: &tacsim::arena::StepOutput) -> usize {
    out.ledger_chunks
        .iter()
        .filter_map(|chunk| std::str::from_utf8(&chunk.bytes).ok())
        .flat_map(str::lines)
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .filter(|event| matches!(event.kind, EventKind::Fired { .. }))
        .count()
}

/// **The relation cost at this instant**: contacts held, the books encoded, and the median of
/// `reps` timings each of the tempo test and of a sweep of every held contact through the books,
/// microseconds.
fn relation_cost(arena: &Arena, reps: usize) -> (usize, usize, f64, f64) {
    let contacts: usize = arena
        .units
        .iter()
        .map(|unit| arena.contacts.of(unit.id).count())
        .sum();
    let books = serde_json::to_vec(&arena.relations)
        .expect("the books encode")
        .len();
    let mut tempo_us = Vec::with_capacity(reps);
    let mut sweep_us = Vec::with_capacity(reps);
    for _ in 0..reps {
        let started = std::time::Instant::now();
        std::hint::black_box(arena.hostile_in_weapons_range());
        tempo_us.push(started.elapsed().as_secs_f64() * 1e6);
        let started = std::time::Instant::now();
        let mut hostile = 0_usize;
        for unit in &arena.units {
            for (_, held) in arena.contacts.of(unit.id) {
                if arena.relations.hostile(unit.team, held.class) {
                    hostile += 1;
                }
            }
        }
        std::hint::black_box(hostile);
        sweep_us.push(started.elapsed().as_secs_f64() * 1e6);
    }
    tempo_us.sort_by(f64::total_cmp);
    sweep_us.sort_by(f64::total_cmp);
    (contacts, books, rank(&tempo_us, 50), rank(&sweep_us, 50))
}

/// One lane's run: the arena stepped for `arena_s` arena seconds, either at the task's lattice or
/// at fixed 100 ms pulses; the `GC12` line printed; the `Engaged` p99 answered.
fn run_lane(rig: &mut Rig, n: usize, lattice: bool, arena_s: i64) -> f64 {
    let mut arena = field_arena(rig, n);
    let opening = tacsim::arena::Snapshot::encode(&arena)
        .expect("the opening encodes")
        .bytes()
        .len();
    let mut all = Vec::new();
    let mut engaged = Vec::new();
    let mut windows: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    let mut tempo_counts = [0_usize; 3];
    let mut shots = 0_usize;
    let mut peak_contacts = 0_usize;
    // The worst wake's work: traces, cell probes, share operations, path queries and path cells.
    let mut work = [0_u64; 5];
    // Mind faults and overflows over the run.
    let mut faults = [0_u64; 2];
    let mut wall = 0_i64;
    let end_ms = arena_s * 1_000;
    while wall < end_ms {
        let started = std::time::Instant::now();
        let out = arena.step_settled(wall, &[], &[]);
        let ms = started.elapsed().as_secs_f64() * 1_000.0;
        all.push(ms);
        *windows.entry(wall / 500).or_default() += ms;
        let slot = match out.tempo {
            Tempo::Quiet => 0,
            Tempo::Alert => 1,
            Tempo::Engaged => 2,
        };
        tempo_counts[slot] += 1;
        if out.tempo == Tempo::Engaged {
            engaged.push(ms);
        }
        shots += shots_in(&out);
        for (peak, now) in work.iter_mut().zip([
            u64::from(out.perception.traces),
            out.perception.cell_probes,
            u64::from(out.perception.share_ops),
            u64::from(out.motor.path_queries),
            out.motor.path_cells,
        ]) {
            *peak = (*peak).max(now);
        }
        faults[0] += u64::from(out.motor.mind_faults);
        faults[1] += u64::from(out.motor.mind_overflows);
        // The world's acknowledgement, as the field task gives it once the chunks are written
        // (`arena_task`'s `mark_flushed`): without it the egress backs up past
        // `ledger.max_unflushed_chunks` and the field is voided.
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        if all.len() % 50 == 0 {
            let held: usize = arena
                .units
                .iter()
                .map(|unit| arena.contacts.of(unit.id).count())
                .sum();
            peak_contacts = peak_contacts.max(held);
        }
        if let Some(end) = &out.ended {
            println!("GC12 n={n}: the field ended at {wall} ms: {end:?}");
            break;
        }
        wall = if lattice {
            out.next_due_ms.max(wall + 1)
        } else {
            wall + 100
        };
    }
    let closing = tacsim::arena::Snapshot::encode(&arena)
        .expect("the arena encodes")
        .bytes()
        .len();
    let standing = arena.units.iter().filter(|u| !u.limits.wreck).count();
    let (contacts, books, tempo_us, sweep_us) = relation_cost(&arena, 200);
    let window_ms: Vec<f64> = windows.values().copied().collect();
    let mut sorted_windows = window_ms.clone();
    sorted_windows.sort_by(f64::total_cmp);
    let mut sorted_engaged = engaged.clone();
    sorted_engaged.sort_by(f64::total_cmp);
    let engaged_p99 = rank(&sorted_engaged, 99);
    println!(
        "GC12 n={n} lane={} arena_s={arena_s}: {} pulses (quiet {} / alert {} / engaged {}), \
         {shots} shots, {standing} of {} standing; pulse ms median/p95/p99/max all {} | engaged \
         {}; 500 ms window ms p99 {:.3} max {:.3}; snapshot {opening} B opening, {closing} B end; \
         relations: {contacts} contacts held at the end (peak {peak_contacts}), books {books} B, \
         hostile_in_weapons_range {tempo_us:.1} us, full contact sweep {sweep_us:.1} us; \
         ledger {} B of {} B; worst wake: {} traces, {} cell probes, {} share ops, {} path \
         queries, {} path cells; mind faults {} (overflows {}); state hash {:016x}",
        if lattice { "lattice" } else { "fixed100" },
        all.len(),
        tempo_counts[0],
        tempo_counts[1],
        tempo_counts[2],
        arena.units.len(),
        spread(&all),
        spread(&engaged),
        rank(&sorted_windows, 99),
        sorted_windows.last().copied().unwrap_or(0.0),
        arena.ledger.bytes_total(),
        arena.ledger.max_bytes(),
        work[0],
        work[1],
        work[2],
        work[3],
        work[4],
        faults[0],
        faults[1],
        tacsim::arena::hash::state_hash(&arena),
    );
    assert!(shots > 0, "n={n}: the field is fought");
    engaged_p99
}

/// **Doc 36 cut GC.12: the field's pulse at 9 to 100 units, six parties** (Q-36-9).
/// See the module's notes for the knobs and the line it prints.
#[test]
#[ignore = "GC.12 scale harness: by hand or gate.yml's sweep dispatch with --run-ignored only"]
fn gc12_the_field_pulse_is_measured_at_scale() {
    let sizes: Vec<usize> = std::env::var("GC12_SIZES")
        .unwrap_or_else(|_| "9,24,48,64,80,100".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let arena_s: i64 = knob("GC12_ARENA_S", 120);
    let assert_budget = knob("GC12_ASSERT", 0_u8) == 1;
    let budget = tacsim::config::HarnessConfig::default().engaged_p99_ms;
    println!(
        "GC12 build={} sizes={sizes:?} arena_s={arena_s} budget engaged p99 {budget} ms \
         ([tacsim.harness].engaged_p99_ms) assert={assert_budget}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let mut rig = stocked();
    for n in sizes {
        let p99 = run_lane(&mut rig, n, true, arena_s);
        run_lane(&mut rig, n, false, arena_s);
        println!(
            "GC12 verdict n={n}: engaged p99 {p99:.3} ms against {budget} ms: {}",
            if p99 <= f64::from(budget) {
                "within"
            } else {
                "PAST"
            }
        );
        if assert_budget {
            assert!(
                p99 <= f64::from(budget),
                "n={n}: engaged p99 {p99:.3} ms is past the {budget} ms budget"
            );
        }
    }
}
