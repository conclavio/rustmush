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
//! the baseline: `TACSIM_UNITS`, `TACSIM_FIGHTING`, `TACSIM_WEAPONS` (**16b cut
//! W.1**: how many of the units the arena measurement arms, doc 16 §1 A2's *150
//! weapons on the field*; the world-side baseline still only reports it),
//! `TACSIM_PULSES`, `TACSIM_SEED`, `TACSIM_JSON`
//! (the report path, default `target/harness.json`) and `TACSIM_ASSERT=1`, which
//! turns the four thresholds into assertions. The default is **report-only**
//! (D-H-8, D-16-19): a shared runner's speed is not a threshold.
//!
//! What it must not do (16b §9's `H` row): touch `mush-sim` or `mush-atlas` — a
//! harness that needs a new API in the thing it measures is measuring its own
//! change — assert by default, or run on push.

use std::collections::BTreeMap;
use std::time::Instant;

use mush_atlas::{AtlasConfig, AtlasState, PackSource, TerrainClass};
use mush_core::{
    Audience, Effect, Effects, GOD, GameModule, Inbound, ObjId, ObjKind, Outbox, WorldCommand,
};
use mush_proto::OutputEvent;
use mush_sim::DomainId;
use mush_testkit::TestWorld;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

// Every test here carries
// `#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch /
// weekly)"]`. `ci_gated_test_inventory` finds the job that owns them by the verbatim
// `-p tacsim --test harness` in `.github/workflows/ci.yml`.

// ---------------------------------------------------------------------------
// Knobs
// ---------------------------------------------------------------------------

/// The world facts one run was driven at, and the four thresholds.
struct Knobs {
    units: u32,
    fighting: u32,
    /// Parsed and reported; no cut produces a weapon yet (cut W).
    weapons: u32,
    pulses: u32,
    seed: u64,
    json: String,
    assert: bool,
    engaged_p99_ms: u32,
    max_traces_per_pulse: u32,
    max_path_queries_per_pulse: u32,
    /// **16b cut S.1** — contact-share operations one pulse may make.
    max_share_ops_per_pulse: u32,
    max_ledger_bytes: u64,
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
            weapons: env_u32("TACSIM_WEAPONS", cfg.weapons),
            pulses: env_u32("TACSIM_PULSES", DEFAULT_PULSES),
            seed: env_u64("TACSIM_SEED", DEFAULT_SEED),
            json: std::env::var("TACSIM_JSON")
                .unwrap_or_else(|_| "target/harness.json".to_string()),
            assert: std::env::var("TACSIM_ASSERT").is_ok_and(|v| v.trim() == "1"),
            engaged_p99_ms: cfg.engaged_p99_ms,
            max_traces_per_pulse: cfg.max_traces_per_pulse,
            max_path_queries_per_pulse: cfg.max_path_queries_per_pulse,
            max_share_ops_per_pulse: cfg.max_share_ops_per_pulse,
            max_ledger_bytes: cfg.max_ledger_bytes,
        }
    }
}

/// 16b §5 H1: sixty pulses is a minute of ground cadence at the default period.
const DEFAULT_PULSES: u32 = 60;

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

/// A seeded smooth elevation field over the arena, in whole centimetres against
/// the region datum.
///
/// Four summed sinusoids with seeded phases and one seeded tilt: reproducible
/// from `TACSIM_SEED` alone, no table, no file, and bounded well inside the
/// `i16` a cell's `elev_cm` is.
struct Relief {
    /// Row-major, `SIDE * SIDE`, southern row first.
    elev_cm: Vec<i32>,
}

impl Relief {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_lossless
    )]
    fn generate(seed: u64) -> Relief {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        // Four waves: two long, two short. Amplitudes in centimetres.
        let waves: Vec<(f64, f64, f64, f64)> = (0..4)
            .map(|n| {
                let periods = [3.0_f64, 5.0, 9.0, 17.0];
                let amps = [900.0_f64, 520.0, 260.0, 120.0];
                let p = periods.get(n).copied().unwrap_or(7.0);
                let a = amps.get(n).copied().unwrap_or(100.0);
                (
                    p,
                    a,
                    rng.random_range(0.0..std::f64::consts::TAU),
                    rng.random_range(0.0..std::f64::consts::TAU),
                )
            })
            .collect();
        let tilt_x = rng.random_range(-4.0_f64..4.0);
        let tilt_y = rng.random_range(-4.0_f64..4.0);
        let side = f64::from(SIDE);
        let mut elev_cm = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE));
        for y in 0..SIDE {
            for x in 0..SIDE {
                let u = f64::from(x) / side;
                let v = f64::from(y) / side;
                let mut h = tilt_x * f64::from(x) + tilt_y * f64::from(y);
                for (period, amp, px, py) in &waves {
                    h += amp
                        * (std::f64::consts::TAU * period * u + px).sin()
                        * (std::f64::consts::TAU * period * v + py).cos();
                }
                // Quantise, then clamp well inside `i16` so no sample is refused.
                let q = (h / f64::from(ELEV_STEP_CM)).round() as i64 * i64::from(ELEV_STEP_CM);
                elev_cm.push(q.clamp(-20_000, 20_000) as i32);
            }
        }
        Relief { elev_cm }
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
    mods.push(comp::content_module(false).expect("the content module composes"));
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

/// Resident memory, kibibytes, where the platform exposes it. `/proc/self/status`
/// on Linux; **absent** everywhere else, never faked (16b §9's `H` row).
fn resident_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse::<u64>().ok())
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

    // ---- the relief, the world and the window ----
    let Window {
        mut tw,
        domain,
        cells: all_cells,
        classes: class_counts,
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

    // No cut produces any of these yet; they are reported as zero so the shape
    // of the report does not change when one starts to.
    let traces_per_pulse = 0_u64;
    let cell_probes_per_pulse = 0_u64;
    let path_queries_per_pulse = 0_u64;
    let share_ops_per_pulse = 0_u64;
    let ledger_bytes = 0_u64;

    // ---- the report ----
    println!(
        "harness_world: units={placed}/{} fighting={sessions} placed_sessions={placed_sessions} weapons={} (reported; the arena measurement below is what arms them)",
        knobs.units, knobs.weapons
    );
    println!(
        "harness_relief: side={SIDE} cell_cm={CELL_CM} arena_m={} quadrants={} seed={} elev_step_cm={ELEV_STEP_CM}",
        i64::from(SIDE) * i64::from(CELL_CM) / 100,
        (SIDE / QUAD) * (SIDE / QUAD),
        knobs.seed
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
    println!(
        "harness_memory: vmrss_start_kib={} vmrss_end_kib={}",
        rss_start_kib.map_or("absent".to_string(), |k| k.to_string()),
        rss_end_kib.map_or("absent".to_string(), |k| k.to_string())
    );
    println!(
        "harness_counters: traces_per_pulse={traces_per_pulse} cell_probes_per_pulse={cell_probes_per_pulse} path_queries_per_pulse={path_queries_per_pulse} share_ops_per_pulse={share_ops_per_pulse} ledger_bytes={ledger_bytes} (no cut produces these yet)"
    );
    println!(
        "harness_thresholds: assert={} engaged_p99_ms={} max_traces_per_pulse={} max_path_queries_per_pulse={} max_ledger_bytes={}",
        knobs.assert,
        knobs.engaged_p99_ms,
        knobs.max_traces_per_pulse,
        knobs.max_path_queries_per_pulse,
        knobs.max_ledger_bytes
    );

    let report = serde_json::json!({
        "world": {
            "units_requested": knobs.units,
            "units_placed": placed,
            "fighting": sessions,
            "sessions_placed": placed_sessions,
            "weapons": knobs.weapons,
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
        "memory": { "vmrss_start_kib": rss_start_kib, "vmrss_end_kib": rss_end_kib },
        "counters": {
            "traces_per_pulse": traces_per_pulse,
            "cell_probes_per_pulse": cell_probes_per_pulse,
            "path_queries_per_pulse": path_queries_per_pulse,
            "share_operations_per_pulse": share_ops_per_pulse,
            "ledger_bytes_streamed": ledger_bytes,
            "note": "no cut produces these yet; cuts A, S and W do",
        },
        "thresholds": {
            "assert": knobs.assert,
            "max_path_queries_per_pulse": knobs.max_path_queries_per_pulse,
            "engaged_p99_ms": knobs.engaged_p99_ms,
            "max_traces_per_pulse": knobs.max_traces_per_pulse,
            "max_path_queries_per_pulse": knobs.max_path_queries_per_pulse,
            "max_ledger_bytes": knobs.max_ledger_bytes,
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
        assert!(
            p99 <= f64::from(knobs.engaged_p99_ms),
            "per-pulse p99 {p99:.3} ms is past harness.engaged_p99_ms {}",
            knobs.engaged_p99_ms
        );
        assert!(
            traces_per_pulse <= u64::from(knobs.max_traces_per_pulse),
            "traces per pulse {traces_per_pulse} is past harness.max_traces_per_pulse {}",
            knobs.max_traces_per_pulse
        );
        assert!(
            path_queries_per_pulse <= u64::from(knobs.max_path_queries_per_pulse),
            "path queries per pulse {path_queries_per_pulse} is past harness.max_path_queries_per_pulse {}",
            knobs.max_path_queries_per_pulse
        );
        assert!(
            ledger_bytes <= knobs.max_ledger_bytes,
            "ledger bytes {ledger_bytes} is past harness.max_ledger_bytes {}",
            knobs.max_ledger_bytes
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
/// factor is `100 / move_cost`. Nothing has yet decided the shipped numbers: the
/// authored `arenas/proving_ground.json` is cut C's and the passability tables are
/// cut T's. So this table is **the harness's own**, chosen to be plausible and to
/// put the water and the walls out of bounds so that units really do run into
/// edges and into each other, and it is named as a test-local table rather than
/// left to look like a shipped decision.
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

/// The stock tank the arena measurement drives: 1,500 cm/s forward, 50% in
/// reverse, 100/200 cm/s², 30 degrees a second (the numbers cut A is measured at;
/// the authored definition is cut C's).
fn harness_mobility() -> tacsim::arena::state::TrackedRules {
    tacsim::arena::state::TrackedRules {
        max_speed_cms: 1_500,
        accel_cms2: 100,
        brake_cms2: 200,
        turn_rate_mdeg_s: 30_000,
        reverse_factor_pct: 50,
    }
}

/// **The sensor every harness unit carries** (16b cut S.1; doc 16 §3.6).
///
/// Harness-local, exactly as [`harness_mobility`] and [`harness_move_cost`] are:
/// the authored `visual_mk1` is cut C's, and what is measured here is the *cost of
/// a pass*, which is a function of the candidate radius and nothing else. The
/// signature table is the shipped one's, so the ground the relief generates really
/// does shorten the reach.
///
/// `max_range_cells` is [`SENSOR_CELLS`] rather than the shipped 60 — see that
/// constant for why, and for the number the shipped reach costs.
fn harness_sensor() -> tacsim::arena::perception::SensorProfile {
    use mush_atlas::TerrainClass as C;
    use tacsim::arena::perception::{SensorProfile, SignatureFactor};
    let row = |class: C, moving: u32, stopped: u32| SignatureFactor {
        class,
        moving_permille: moving,
        stopped_permille: stopped,
    };
    SensorProfile {
        max_range_cells: sensor_cells(),
        max_attenuation: 40,
        classify_after_ms: 3_000,
        sees_wrecks: false,
        share_range_cells: share_cells(),
        signature: vec![
            row(C::Road, 1_000, 1_000),
            row(C::Ground, 1_000, 900),
            row(C::Grass, 1_000, 800),
            row(C::Rubble, 950, 700),
            row(C::WoodsLight, 900, 650),
            row(C::WoodsHeavy, 800, 500),
        ],
    }
}

/// **The candidate radius the arena measurement runs at**, in cells, overridable
/// with `TACSIM_SENSOR_CELLS`.
///
/// Doc 16 §2 W1 is *sensor ranges are short and bounded*, and W4 bounds *traces
/// and cell probes per pulse* by `harness.max_traces_per_pulse` (4,000). Those two
/// facts meet at a number, and this is the arithmetic:
///
/// * 300 units on 256 by 256 cells, of which about 51,000 are passable, is a
///   density near 0.006 units a cell;
/// * the harness's object ids are consecutive (`100_000 + n`), so `id % pass_ms`
///   puts all 300 first passes inside the first 300 ms of each second and the
///   worst `Engaged` pulse carries every one of them;
/// * a pass traces every candidate inside its reach, so the worst pulse walks
///   about `300 × π r² × 0.006` traces, which is 4,000 at `r ≈ 30` and about
///   17,000 at the shipped `r = 60`.
///
/// So the shipped 600 m sight at this density does **not** fit the bound, and the
/// measurement says so rather than hiding it: the run below reports the traces it
/// walked at whatever radius it was given, and `TACSIM_SENSOR_CELLS=60` reproduces
/// the shipped reach's number on demand. 24 is the default because it leaves the
/// bound room for cut W's shots and cut N's paths in the same pulse.
fn sensor_cells() -> u16 {
    u16::try_from(env_u64("TACSIM_SENSOR_CELLS", u64::from(SENSOR_CELLS))).unwrap_or(SENSOR_CELLS)
}

/// The default of [`sensor_cells`], and what [`harness_sensor`] is built with.
const SENSOR_CELLS: u16 = 24;

/// **The comms reach the share mesh runs at**, in cells.
///
/// 16b §9 pins `comms.share_range_cells` at 30 and the shipped sensor authors 30;
/// this is 8, and the reason is the same arithmetic [`sensor_cells`] records.
/// doc 16 §3.6 step 5's mesh is continuous, so one `(sharer, teammate)` pair is
/// considered per sharer per pass — a range test and a trace each — and
/// `harness.max_share_ops_per_pulse` is 256. With all 300 passes landing in one
/// `Engaged` pulse, 256 ops is 0.85 teammates a sharer, which at a density near
/// 0.006 units a cell (half of them teammates) is a reach near 9 cells; the stock
/// 30 is about 2,400 ops and 2,400 further traces in that pulse. So the bound and
/// the authored reach do not agree at 300 units, the measurement runs at the reach
/// that fits and reports it, and `TACSIM_SHARE_CELLS=30` reproduces the stock
/// reach's numbers on demand.
fn share_cells() -> u16 {
    u16::try_from(env_u64("TACSIM_SHARE_CELLS", u64::from(SHARE_CELLS))).unwrap_or(SHARE_CELLS)
}

/// The default of [`share_cells`].
const SHARE_CELLS: u16 = 8;

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
/// 160 m, about eleven seconds at the harness tank's ceiling.
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

/// **The weapon `TACSIM_WEAPONS` of the units carry** (16b cut W.1; doc 16 §1 A2:
/// *150 weapons on the field*).
///
/// Harness-local, as [`harness_mobility`] and [`harness_sensor`] are: the authored
/// `cannon_105` is cut C's. What is measured here is the cost of a *shot* — one
/// insert in the in-flight table, two due events, one ledger line — which is a
/// function of how often a unit may fire and not of the numbers themselves.
///
/// The cycle is 4,000 ms against the scripted fire's every ten `Engaged` pulses
/// (5,000 ms of arena time), so a unit that holds a lock fires on every attempt
/// rather than being refused half of them; the muzzle speed puts a shot across the
/// whole 2.56 km window in about 1.8 s, which is long enough that the impact heap
/// really carries shots between pulses rather than resolving each inside its own.
fn harness_weapon() -> tacsim::arena::state::WeaponSummary {
    tacsim::arena::state::WeaponSummary {
        id: smol_str::SmolStr::new("cannon"),
        max_range_cm: 250_000,
        cycle_ms: 4_000,
        ammunition: smol_str::SmolStr::new("tacsim.ammunition.he_105"),
        burst_rounds: 1,
        muzzle_speed_cms: 145_000,
        bands: vec![
            tacsim::arena::weapons::WeaponBand {
                max_range_cm: 150_000,
                dispersion_mrad: 3,
            },
            tacsim::arena::weapons::WeaponBand {
                max_range_cm: 250_000,
                dispersion_mrad: 6,
            },
        ],
        count_per_load: 40,
        // 16b cut W.2: the round's damage and the section it is mounted in, so the
        // measured pulses carry real hits and real damage plans.
        damage: 120,
        mount_section: smol_str::SmolStr::new("turret"),
    }
}

/// An exact definition identity with a zero digest, for the harness's own shapes.
fn harness_dref(kind: &str, name: &str) -> mush_tactics::DefinitionRef {
    mush_tactics::DefinitionRef::new(
        mush_core::DefinitionKey::new(
            mush_core::OwnerId::new("tacsim").expect("a canonical owner"),
            mush_core::DefinitionKind::new(kind).expect("a canonical kind"),
            mush_core::DefinitionId::new(name).expect("a canonical id"),
        ),
        1,
        mush_core::DefinitionDigest::from_bytes([0_u8; 32]),
    )
    .expect("version 1 is not zero")
}

/// An empty opaque rules blob.
fn harness_rules() -> mush_tactics::RulesValue {
    mush_tactics::RulesValue::new(
        harness_dref("schema", "tacsim.arena.harness"),
        serde_json::json!({}),
    )
    .expect("an empty rules blob")
}

/// **The M32 unit the harness fights with** — the stock tank's structure: five
/// sections, their facets and their internals, the cannon and the sight, and the
/// power capability a wreck loses.
///
/// **16b cut W.2 made it the real shape.** Cut A applied no plan to it (D-16-6) and
/// one hull facet was enough; cut W.2 applies one
/// [`mush_tactics::UnitMutationPlan`] per hit, and a unit whose sections the facet
/// table names would be a unit every hit refused — which would measure the cost of
/// the *miss* path and report it as the cost of the arena. The numbers are close to
/// `content/units/line_tank.json` and deliberately not read from it: a harness that
/// booted the content loader would be measuring the content loader.
fn harness_unit(id: ObjId) -> mush_tactics::TacticalUnit {
    let facet = |name: &str, armor: i64| mush_tactics::FacetSpec {
        id: mush_tactics::FacetId::new(name).expect("a canonical facet"),
        cells: vec![mush_tactics::CellSpec {
            id: mush_tactics::CellId::new("armor").expect("a cell"),
            min: 0,
            max: armor,
            init: armor,
        }],
    };
    let section = |name: &str, facets: Vec<mush_tactics::FacetSpec>, internals: i64| {
        mush_tactics::SectionSpec {
            id: mush_tactics::SectionId::new(name).expect("a canonical section"),
            condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
            facets,
            slots: Vec::new(),
            cells: vec![mush_tactics::CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: internals,
                init: internals,
            }],
        }
    };
    let spec = mush_tactics::UnitSpec {
        definition: harness_dref("unit", "tacsim.unit.line_tank"),
        sections: vec![
            section(
                "hull",
                vec![
                    facet("front", 400),
                    facet("left", 250),
                    facet("right", 250),
                    facet("rear", 150),
                    facet("top", 100),
                ],
                200,
            ),
            section(
                "turret",
                vec![
                    facet("front", 350),
                    facet("left", 220),
                    facet("right", 220),
                    facet("rear", 140),
                ],
                150,
            ),
            section("left_track", vec![facet("left", 120)], 100),
            section("right_track", vec![facet("right", 120)], 100),
            section("engine", Vec::new(), 120),
        ],
        unit_cells: Vec::new(),
        components: vec![
            mush_tactics::ComponentSpec {
                id: mush_tactics::ComponentId::new("cannon").expect("a component"),
                definition: harness_dref("component", "tacsim.unit.line_tank.cannon"),
                install: mush_tactics::InstallPoint::Section(
                    mush_tactics::SectionId::new("turret").expect("a canonical section"),
                ),
                condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
                cells: vec![
                    mush_tactics::CellSpec {
                        id: mush_tactics::CellId::new("ammo").expect("a cell"),
                        min: 0,
                        max: 40,
                        init: 40,
                    },
                    mush_tactics::CellSpec {
                        id: mush_tactics::CellId::new("ready_at_ms").expect("a cell"),
                        min: 0,
                        max: 3_600_000,
                        init: 0,
                    },
                ],
                asset: None,
            },
            mush_tactics::ComponentSpec {
                id: mush_tactics::ComponentId::new("visual_sensor").expect("a component"),
                definition: harness_dref("component", "tacsim.unit.line_tank.visual_sensor"),
                install: mush_tactics::InstallPoint::Section(
                    mush_tactics::SectionId::new("turret").expect("a canonical section"),
                ),
                condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
                cells: vec![mush_tactics::CellSpec {
                    id: mush_tactics::CellId::new("condition").expect("a cell"),
                    min: 0,
                    max: 100,
                    init: 100,
                }],
                asset: None,
            },
        ],
        capabilities: vec![mush_tactics::CapabilityInit {
            key: mush_tactics::CapabilityKey {
                id: mush_tactics::CapabilityId::new("tacsim.power").expect("a capability"),
                source: None,
            },
            rules: harness_rules(),
        }],
        mobility: None,
        rules: harness_rules(),
    };
    mush_tactics::TacticalUnit::from_spec(&spec, id).expect("the harness tank")
}

/// **What a harness unit presents and what a hit may strike** (16b cut W.2), the
/// stock tank's own facet table and silhouettes.
///
/// Harness-local for the same reason [`harness_unit`] is: the authored numbers are
/// cut C's, and what is measured here is the cost of resolving and applying a hit.
fn harness_profile() -> tacsim::arena::resolution::UnitProfile {
    use tacsim::arena::resolution::{BandProfile, Consequence, SectionProfile, UnitProfile};
    let band = |name: &str, width: u32, height: u32, rows: &[(&str, u16)]| BandProfile {
        band: smol_str::SmolStr::new(name),
        width_cm: width,
        height_cm: height,
        sections: rows
            .iter()
            .map(|(section, weight)| (smol_str::SmolStr::new(*section), *weight))
            .collect(),
    };
    let section = |name: &str, facets: &[&str], consequences: Vec<Consequence>| SectionProfile {
        id: smol_str::SmolStr::new(name),
        facets: facets
            .iter()
            .map(|facet| smol_str::SmolStr::new(*facet))
            .collect(),
        consequences,
        struck_through: None,
    };
    UnitProfile {
        bands: vec![
            band("front", 350, 250, &[("hull", 50), ("turret", 50)]),
            band(
                "left",
                700,
                250,
                &[("hull", 40), ("left_track", 30), ("turret", 30)],
            ),
            band(
                "right",
                700,
                250,
                &[("hull", 40), ("right_track", 30), ("turret", 30)],
            ),
            band(
                "rear",
                350,
                250,
                &[("hull", 40), ("engine", 30), ("turret", 30)],
            ),
            band("top", 700, 350, &[("hull", 50), ("turret", 50)]),
        ],
        sections: vec![
            section(
                "hull",
                &["front", "left", "right", "rear", "top"],
                vec![Consequence::DestroyUnit],
            ),
            section(
                "turret",
                &["front", "left", "right", "rear"],
                vec![Consequence::RemoveWeapons {
                    section: smol_str::SmolStr::new("turret"),
                }],
            ),
            section(
                "left_track",
                &["left"],
                vec![
                    Consequence::Slow { percent: 50 },
                    Consequence::NoTurnToward {
                        side: tacsim::arena::state::Side::Left,
                    },
                ],
            ),
            section(
                "right_track",
                &["right"],
                vec![
                    Consequence::Slow { percent: 50 },
                    Consequence::NoTurnToward {
                        side: tacsim::arena::state::Side::Right,
                    },
                ],
            ),
            SectionProfile {
                id: smol_str::SmolStr::new("engine"),
                facets: Vec::new(),
                consequences: vec![Consequence::DisableMotion],
                struck_through: Some((
                    smol_str::SmolStr::new("hull"),
                    smol_str::SmolStr::new("rear"),
                )),
            },
        ],
    }
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
/// `EventKind::Blocked::cause`); the split by the named cell is kept because it
/// answers *where* — interior, the window's ring, or ground nothing may enter —
/// and it is the split earlier cuts' runs were read against. The chunk is JSONL
/// and the line's shape is fixed by `EventKind::Blocked`, so this reads the two
/// integers out of it rather than deserialising a whole event per stop.
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

/// `<stem>-arena.<ext>` beside the baseline's report, so the two measurements do
/// not overwrite each other's JSON.
fn arena_report_path(given: &str) -> std::path::PathBuf {
    let path = report_path(given);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "harness".to_string());
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "json".to_string());
    path.with_file_name(format!("{stem}-arena.{ext}"))
}

/// **Cut A's measurement** (16b §6's verify lens: *the harness at 300 moving units
/// stays under 5 ms per pulse before perception and weapons exist*).
///
/// The same generated relief as the baseline above, converted into an
/// `ArenaDefinition` through [`harness_move_cost`], with `TACSIM_UNITS` tanks
/// scattered over the passable cells and `TACSIM_FIGHTING` of them seated, driven
/// through `Arena::step` at the `Engaged` period with the tempo pinned there.
///
/// **Every unit is brought online the real way**: a `STARTUP` intent in the
/// opening pulse and then `match.startup_ms` of arena time, measured separately as
/// the warm-up, rather than by constructing the units online. Then each is given a
/// heading and its ceiling speed, and a fresh seeded heading every ten pulses, so
/// that they keep moving, hit the water and the window's edge, and block each
/// other rather than settling into a corner.
///
/// The arena frame is the **window's own**: cell `(0, 0)` is at the origin,
/// because `ArenaTerrain` puts the window's corner there. Cut T is where the
/// region frame and the arena frame are reconciled at check-out and check-in.
#[test]
#[ignore = "tacsim harness: run by the CI `tacsim-harness` job (workflow_dispatch / weekly)"]
#[allow(clippy::too_many_lines)]
fn tacsim_harness_arena_step() {
    use tacsim::arena::clock::Tempo as ArenaTempo;
    use tacsim::arena::perception::PerceptionCounters;
    use tacsim::arena::state::{
        Arena, ArenaDefinition, ArenaTerrain, CustodyOut, CustodyUnit, EntryZone, MatchEnd, Pilot,
        Power, Team, TerrainCell,
    };
    use tacsim::arena::step::StepOutput;
    use tacsim::config::{ArenaConfig, MatchConfig};

    let knobs = Knobs::from_env();
    let rss_start_kib = resident_kib();

    // ---- the relief, as an arena definition ----
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
    // The world was needed for the terrain and for nothing else: the arena is a
    // library (doc 16 §3.1) and holds no handle on it.
    drop(tw);

    let convert_at = Instant::now();
    let mut grid = vec![
        TerrainCell {
            class: TerrainClass::Void,
            elev_cm: 0,
            move_cost: 0,
        };
        usize::from(SIDE) * usize::from(SIDE)
    ];
    for (x, y, class, elev_cm) in &all_cells {
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
    // **16b cut N.1**: the harness's own patrol graph. The authored graph is cut
    // C's, on cut C's arena; this window is generated relief, so the harness
    // generates a **coarse grid graph** over it ([`harness_waypoints`]) — 16 by 16
    // nodes on a 16-cell pitch, each snapped to the nearest passable cell, joined
    // to its orthogonal neighbours. That is 256 nodes and at most 480 edges, which
    // is exactly `MAX_WAYPOINT_NODES` and well inside `MAX_WAYPOINT_EDGES`, and it
    // is what gives 276 patrolling units somewhere to be.
    let waypoints = harness_waypoints(&grid);
    let waypoint_nodes = waypoints.nodes().len();
    let definition = ArenaDefinition {
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
    };
    let convert_ms = convert_at.elapsed().as_secs_f64() * 1_000.0;
    assert!(
        u32::try_from(passable.len()).unwrap_or(0) > knobs.units,
        "only {} passable cells for {} units",
        passable.len(),
        knobs.units
    );

    // ---- the units: the fighting ones nose to nose, the rest scattered ----
    //
    // **16b cut N.1**: doc 16 §1 A2 is *300 units placed and moving, 24 of them
    // fighting*, and with the reflex controller driving the field that has to be
    // arranged rather than hoped for. The `TACSIM_FIGHTING` units are set out in two
    // lines across the middle of the window, so they hold each other from the first
    // sensor pass and spend the measured pulses in `Engage`; the rest are scattered
    // over the window as before and patrol the graph. A unit's team is still
    // `n % 2`, so the two sides are even and the two lines alternate.
    let mut rng = ChaCha8Rng::seed_from_u64(knobs.seed ^ 0x000a_4e17);
    let mut pool: Vec<(u16, u16, i16)> = passable.clone();
    // Two lines facing each other across the middle of the window: the even units
    // (red) [`LINE_GAP_CELLS`] west of the centre, the odd (blue) the same to the
    // east, and [`LINE_PITCH_CELLS`] between neighbours down each line. Each wanted
    // cell is answered with the nearest **passable** one, so the lines follow the
    // generated relief rather than standing in the lake.
    //
    // The pitch is not cosmetic: doc 16 §3.6 step 5's share mesh is one operation
    // per `(sharer, teammate)` pair inside `comms.share_range_cells`, so twelve
    // teammates in one blob is 132 operations a pass on its own and blows
    // `harness.max_share_ops_per_pulse` before the scattered field is counted.
    // Three cells of pitch spreads a line over 36 and keeps the mesh inside its
    // bound while leaving the two lines eight cells apart — well inside the sensor
    // reach and the cannon's, so they fight.
    let middle = i32::from(SIDE) / 2;
    let mut arena_cluster: Vec<(u16, u16, i16)> = Vec::new();
    for n in 0..i32::try_from(knobs.fighting).unwrap_or(0) {
        let side = if n % 2 == 0 { -1 } else { 1 };
        let rank = n / 2;
        let want_x = middle + side * i32::from(LINE_GAP_CELLS);
        let want_y = middle - 18 + rank * i32::from(LINE_PITCH_CELLS);
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
    let mut units: Vec<CustodyUnit> = Vec::new();
    let mut pilots: Vec<Pilot> = Vec::new();
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
        let pose = mush_sim::data::Pose {
            x_cm: i64::from(cx) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
            y_cm: i64::from(cy) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
            z_cm: i32::from(elev_cm),
            heading_mdeg: rng.random_range(0..360_u32) * 1_000,
            posture: harness_rules(),
        };
        if n < knobs.fighting {
            pilots.push(Pilot {
                unit: id,
                pilot: ObjId(500_000 + n),
                station: smol_str::SmolStr::new("cockpit"),
                epoch: 1,
            });
        }
        units.push(CustodyUnit {
            id,
            team: if n % 2 == 0 { Team::Red } else { Team::Blue },
            pose,
            mobility: harness_mobility(),
            // 16b cut S.1: every unit sees, so every unit's pass runs and the
            // trace, probe and share counts below are the loaded arena's.
            sensor_height_cm: 150,
            sensor: Some(harness_sensor()),
            // **16b cut W.1**: `TACSIM_WEAPONS` (150 by default) of the units carry
            // one, which is doc 16 §1 A2's *150 weapons on the field* read
            // literally; cut S.1 gave every unit a row for its one number, and the
            // derived `tempo.alert_range_cm` is priced off the longest reach in the
            // *match* (held to the window's diagonal since 16b cut E.1, which is
            // what the harness reports), so arming half the field leaves that
            // unchanged. An unarmed unit is still a target and still a contact.
            weapons: if n < knobs.weapons {
                vec![harness_weapon()]
            } else {
                Vec::new()
            },
            // **16b cut W.2**: every unit presents a silhouette and carries a facet
            // table, so a hit resolves, applies its plan and can take a section off —
            // which is the cost the p99 below is of. An unarmed unit is still a target.
            profile: harness_profile(),
            gunnery: 1,
            // **16b cut N.1**: every unit on the field is the reflex controller's
            // (doc 16 §6). The `TACSIM_FIGHTING` seats below are **frame sinks**,
            // not pilots: doc 16 §1 A2 asks for connected sessions receiving output
            // and the frame builder is a real per-pulse cost, so the seats stay to
            // keep it in the measurement — but no pilot issues an intent, and every
            // order on this field is the machine's. A seated pilot on a `Reflex`
            // unit is the state doc 16 §3.8's released seat leaves behind, so it is
            // not a shape the engine has to invent for the harness.
            controller: tacsim::arena::state::Controller::Reflex,
            // 16b cut Q.1: no pod behind it.
            slot: 0,
            unit: harness_unit(id),
        });
    }
    let placed = u32::try_from(units.len()).unwrap_or(0);
    let seated = u32::try_from(pilots.len()).unwrap_or(0);

    let arena_cfg = ArenaConfig::default();
    let match_cfg = MatchConfig::default();
    let open_at = Instant::now();
    let sensors_cfg = tacsim::config::SensorsConfig::default();
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
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

    // What the streamed ledger says, counted as it goes rather than kept: the
    // egress is a ring and the point of the measurement is that it drains.
    // **16b cut W.1** added the three weapon kinds; **16b cut W.2** added the four
    // outcome kinds, because a pipeline that resolved every shot as a miss would
    // never build a mutation plan and would measure the cheap half of the cut.
    #[derive(Default)]
    struct Counts {
        chunks: u64,
        bytes: u64,
        blocked: u64,
        /// **16b cut E.3**: every refused cell, so the stops can be split by
        /// where they happened (the event carries its cause too since 16b cut
        /// M.3; this split is the *where*).
        blocked_at: Vec<(u16, u16)>,
        locked: u64,
        fired: u64,
        impacts: u64,
        hits: u64,
        misses: u64,
        sections: u64,
        destroyed: u64,
    }
    let mut counts = Counts::default();
    let absorb_out = |out: &StepOutput, counts: &mut Counts| {
        for chunk in &out.ledger_chunks {
            counts.chunks += 1;
            counts.bytes += u64::try_from(chunk.bytes.len()).unwrap_or(0);
            counts.blocked += occurrences(&chunk.bytes, "\"kind\":\"blocked\"");
            counts.blocked_at.extend(blocked_cells_of(&chunk.bytes));
            counts.locked += occurrences(&chunk.bytes, "\"kind\":\"locked\"");
            counts.fired += occurrences(&chunk.bytes, "\"kind\":\"fired\"");
            counts.impacts += occurrences(&chunk.bytes, "\"kind\":\"impact\"");
            counts.hits += occurrences(&chunk.bytes, "\"kind\":\"hit\"");
            counts.misses += occurrences(&chunk.bytes, "\"kind\":\"miss\"");
            counts.sections += occurrences(&chunk.bytes, "\"kind\":\"section_destroyed\"");
            counts.destroyed += occurrences(&chunk.bytes, "\"kind\":\"unit_destroyed\"");
        }
    };

    // ---- the warm-up: the controller starts them up ----
    //
    // **16b cut N.1**: cut A's harness staged 300 `STARTUP` intents by hand and
    // waited `match.startup_ms`. It does not any more, and that is the measurement:
    // every unit is the reflex controller's, so bringing 300 tanks online is the
    // machine's own first state and the warm-up is over when the field is online.
    // The bound is `decide_ms + startup_ms` plus a pulse, and it is asserted —
    // a controller that started nothing would otherwise be measured driving an
    // arena of cold tanks.
    let warm_at = Instant::now();
    let mut wall = 0_i64;
    let out = arena.step(wall, &[], &[]);
    absorb_out(&out, &mut counts);
    let mut warm_pulses = 1_u32;
    let warm_until = i64::from(match_cfg.startup_ms)
        .saturating_add(i64::from(arena_cfg.reflex.decide_ms))
        .saturating_add(period_ms);
    while arena.now_ms < warm_until {
        wall += period_ms;
        let out = arena.step(wall, &[], &[]);
        absorb_out(&out, &mut counts);
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        warm_pulses += 1;
    }
    let warm_ms = warm_at.elapsed().as_secs_f64() * 1_000.0;
    let cold = arena
        .units
        .iter()
        .filter(|unit| unit.power != Power::Online)
        .count();
    assert_eq!(
        cold, 0,
        "{cold} units never came online: the reflex controller did not start the field up"
    );

    // ---- the measured pulses ----
    //
    // **16b cut N.1**: no scripted order, and no scripted fire. Cut A gave every
    // unit a fresh heading every ten pulses and cut W.1 scripted a lock and a shot
    // beside it; both are gone, and what steers, closes, locks and fires below is
    // the reflex controller alone. That is the whole of what this cut measures:
    // 300 machines deciding on their own cadence, the paths they ask for, and the
    // shots those decisions produce.
    let mut per_pulse: Vec<f64> = Vec::with_capacity(usize::try_from(knobs.pulses).unwrap_or(0));
    let mut substeps: Vec<u32> = Vec::new();
    let mut frames: Vec<f64> = Vec::new();
    let mut ended: Option<MatchEnd> = None;
    let mut lagged_pulses = 0_u32;
    // 16b cut S.1: the perception cost, as a running total and as the worst pulse.
    // Doc 16 §2 W4 and W6 bound *per pulse*, so the peak is what the thresholds are
    // asserted against and the total is what says the pass did work at all.
    let mut perception_total = PerceptionCounters::default();
    let mut peak = PerceptionCounters::default();
    // 16b cut N.1: the controller's cost, as a running total and as the worst
    // pulse. `harness.max_path_queries_per_pulse` is asserted against the peak.
    let mut reflex_total = tacsim::arena::reflex::ReflexCounters::default();
    let mut reflex_peak = tacsim::arena::reflex::ReflexCounters::default();
    for _pulse in 0..knobs.pulses {
        wall += period_ms;
        let at = Instant::now();
        let out = arena.step(wall, &[], &[]);
        per_pulse.push(at.elapsed().as_secs_f64() * 1_000.0);
        perception_total.absorb(&out.perception);
        peak.passes = peak.passes.max(out.perception.passes);
        peak.candidates = peak.candidates.max(out.perception.candidates);
        peak.traces = peak.traces.max(out.perception.traces);
        peak.cell_probes = peak.cell_probes.max(out.perception.cell_probes);
        peak.share_ops = peak.share_ops.max(out.perception.share_ops);
        peak.detected = peak.detected.max(out.perception.detected);
        peak.classified = peak.classified.max(out.perception.classified);
        peak.lost = peak.lost.max(out.perception.lost);
        reflex_total.absorb(&out.reflex);
        reflex_peak.ticks = reflex_peak.ticks.max(out.reflex.ticks);
        reflex_peak.path_queries = reflex_peak.path_queries.max(out.reflex.path_queries);
        reflex_peak.path_deferred = reflex_peak.path_deferred.max(out.reflex.path_deferred);
        reflex_peak.path_cells = reflex_peak.path_cells.max(out.reflex.path_cells);
        reflex_peak.path_failures = reflex_peak.path_failures.max(out.reflex.path_failures);
        reflex_peak.traces = reflex_peak.traces.max(out.reflex.traces);
        reflex_peak.intents = reflex_peak.intents.max(out.reflex.intents);
        reflex_peak.shots = reflex_peak.shots.max(out.reflex.shots);
        substeps.push(out.substeps);
        #[allow(clippy::cast_precision_loss)]
        frames.push(out.frames.len() as f64);
        if out.lagged {
            lagged_pulses += 1;
        }
        absorb_out(&out, &mut counts);
        // The world half's acknowledgement (doc 16 §3.1's ack-gated egress), so the
        // bounded ring is measured draining as it would in production.
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        if let Some(end) = out.ended {
            ended = Some(end);
            break;
        }
    }
    // Whatever is still open leaves with the measurement.
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        counts.chunks += 1;
        counts.bytes += u64::try_from(chunk.bytes.len()).unwrap_or(0);
        counts.blocked += occurrences(&chunk.bytes, "\"kind\":\"blocked\"");
        counts.blocked_at.extend(blocked_cells_of(&chunk.bytes));
        counts.locked += occurrences(&chunk.bytes, "\"kind\":\"locked\"");
        counts.fired += occurrences(&chunk.bytes, "\"kind\":\"fired\"");
        counts.impacts += occurrences(&chunk.bytes, "\"kind\":\"impact\"");
        counts.hits += occurrences(&chunk.bytes, "\"kind\":\"hit\"");
        counts.misses += occurrences(&chunk.bytes, "\"kind\":\"miss\"");
        counts.sections += occurrences(&chunk.bytes, "\"kind\":\"section_destroyed\"");
        counts.destroyed += occurrences(&chunk.bytes, "\"kind\":\"unit_destroyed\"");
    }

    let rss_end_kib = resident_kib();
    let tempo = Tempo {
        name: "arena_engaged",
        period_ms: arena_cfg.tempo.engaged_ms,
        ms: per_pulse,
    };
    let (p50, p95, p99, max) = tempo.stats();
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
    // 16b cut W.1: the rounds figure, over the whole field.
    //
    // **16b cut W.2 made it a statement about the survivors and not a conservation
    // law.** `armed` and `loaded` are read off each unit's *live* weapon table, and
    // doc 16 §3.3's `remove_weapons` and `destroy_unit` consequences empty that table
    // — so a tank whose turret came off, and a wreck, take their remaining rounds out
    // of both sides of the sum with them. `fired` below is the black box's own count
    // of pulls and is the number that is conserved; `rounds_spent` is what the tanks
    // still standing have spent. The exact conservation law is
    // `crates/tacsim/tests/resolution.rs`'s, on a fixture where nothing is destroyed.
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
    let loaded: u64 = arena
        .units
        .iter()
        .flat_map(|unit| unit.weapons.iter())
        .map(|weapon| u64::from(weapon.count_per_load))
        .sum();
    let rounds_spent = loaded.saturating_sub(rounds_left);

    // **The measurement was of a loaded arena.** These are not thresholds and they
    // do not wait for `TACSIM_ASSERT`: a regression that refused every `Speed`
    // order, or stopped integrating, would make the pulse *faster* and pass every
    // timing bound in this file, so the report would say the arena is fast at doing
    // nothing. The numbers themselves are noise; that they are non-zero is not.
    assert!(
        moving > 0,
        "no unit was moving: the measurement was of a parked arena"
    );
    assert!(
        counts.blocked > 0,
        "nothing was ever blocked: the units never reached the water, each other or \
         the window's edge, so the boundary test was not measured"
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
        "no shot ever landed: the resolution and the damage plan were not measured          (fired={} impacts={} misses={})",
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
        "harness_arena_world: units={placed}/{} pilots={seated} cell_cm={CELL_CM} side={SIDE} passable_cells={} seed={} arena_ms={}",
        knobs.units,
        passable.len(),
        knobs.seed,
        arena.now_ms
    );
    let classes = class_counts
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!("harness_arena_classes: {classes}");
    println!(
        "harness_arena_setup_ms: generate={generate_ms:.1} boot={boot_ms:.1} pack_apply={apply_ms:.1} window_open={open_ms:.1} terrain_convert={convert_ms:.1} arena_open={arena_open_ms:.1} warm_up={warm_ms:.1} warm_pulses={warm_pulses}"
    );
    println!(
        "harness_arena_engaged: period_ms={} pulses={} p50={p50:.3} p95={p95:.3} p99={p99:.3} max={max:.3}",
        tempo.period_ms,
        tempo.ms.len()
    );
    // **16b cut M.2** (16c `T-05`; doc 16 §3.1 as amended, Q24): whether the
    // cohort's substep divides all three periods — the one thing that makes the
    // scripted three-cadence replays cut arena time on one set of boundaries.
    let substep = tacsim::arena::clock::substep_ms(
        &arena_cfg.tempo,
        CELL_CM,
        harness_mobility().max_speed_cms,
    );
    let substep_divides_periods = [
        arena_cfg.tempo.quiet_ms,
        arena_cfg.tempo.alert_ms,
        arena_cfg.tempo.engaged_ms,
    ]
    .iter()
    .all(|period| period % substep == 0);
    println!(
        "harness_arena_substeps: per_pulse_min={substeps_min} per_pulse_max={substeps_max} substep_ms={substep} divides_periods={substep_divides_periods}"
    );
    // **16b cut E.3**: the stops split by what refused them, so that "`blocked`
    // fell" is evidence and not an observation. Written when the `blocked`
    // event carried the refused cell and no cause, so the window was all there
    // was to ask; since 16b cut M.3 (16c `T-04`) the line names its cause as
    // well, and this split stays the *where* — each bucket named for what the
    // window can actually answer:
    //
    // - **`impassable`**: the named cell is off the window or is a cell nothing
    //   tracked may enter. Only `BlockedCause::Impassable` can name one, so this
    //   bucket is exact.
    // - **`ring`**: a passable cell on the window's **outer ring**. An `Edge`
    //   refusal names `nearest_on_window`, which clamps each axis into the window
    //   and therefore always names a passable ring cell (the unit's own), so every
    //   edge stop is in here. A crossing refused by a unit or a bank that happens
    //   to sit on the ring is in here too, and on a 256-cell window the ring is
    //   under two per cent of the ground.
    // - **`interior`**: a passable cell off the ring — a crossing refused by
    //   another unit, or by a bank too steep to climb, which this field's ground
    //   barely has. No edge stop can reach this bucket, which is the whole point:
    //   this is the number that moves with `occupied_stop_ms` and nothing else in
    //   the split does.
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
        "harness_arena_blocked_split: interior={interior} ring={ring} impassable={impassable} of {} lines",
        counts.blocked_at.len()
    );
    println!(
        "harness_arena_counters: blocked={} ledger_bytes={} chunks={} frames_per_pulse_mean={frames_mean:.2} moving_at_end={moving} held_at_end={held} lagged_pulses={lagged_pulses} ended={}",
        counts.blocked,
        counts.bytes,
        counts.chunks,
        ended.map_or("none".to_string(), |end| format!("{:?}", end.reason))
    );
    println!(
        "harness_arena_weapons: armed={armed} locked={} fired={} impacts={} in_flight_at_end={} shot_seq={} rounds_spent={rounds_spent} rounds_left={rounds_left}",
        counts.locked,
        counts.fired,
        counts.impacts,
        arena.shots.len(),
        arena.next_shot_seq
    );
    // **16b cut W.2**: what the resolution and the damage did with those shots.
    println!(
        "harness_arena_damage: hits={} misses={} sections_destroyed={} units_destroyed={} wrecks_at_end={} disabled_at_end={}",
        counts.hits,
        counts.misses,
        counts.sections,
        counts.destroyed,
        arena.units.iter().filter(|unit| unit.limits.wreck).count(),
        arena.units.iter().filter(|unit| unit.disabled).count()
    );
    // **16b cut N.1**, the same rule the four above follow: a controller that never
    // decided, never searched and never fired would make the pulse cheaper and pass
    // every timing bound in this file, and the report would say the arena is fast at
    // standing still.
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
        "harness_arena_reflex: waypoint_nodes={waypoint_nodes} decide_ms={} path_max_cells={} max_path_queries={} repath_ms={} ticks={} intents={} shots={} path_queries={} path_deferred={} path_cells={} path_failures={} traces={} peak_ticks={} peak_path_queries={} peak_path_cells={} peak_traces={}",
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
        reflex_peak.ticks,
        reflex_peak.path_queries,
        reflex_peak.path_cells,
        reflex_peak.traces
    );
    println!(
        "harness_arena_reflex_states: {}",
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
    println!(
        "harness_arena_perception_peak: passes={} candidates={} traces={} cell_probes={} share_ops={} detected={} classified={} lost={}",
        peak.passes,
        peak.candidates,
        peak.traces,
        peak.cell_probes,
        peak.share_ops,
        peak.detected,
        peak.classified,
        peak.lost
    );
    println!(
        "harness_arena_perception_total: sensor_cells={} share_cells={} pass_ms={} lost_after_passes={} passes={} candidates={} traces={} cell_probes={} share_ops={} detected={} classified={} lost={} contacts_at_end={} alert_range_cm={}",
        sensor_cells(),
        share_cells(),
        sensors_cfg.pass_ms,
        sensors_cfg.lost_after_passes,
        perception_total.passes,
        perception_total.candidates,
        perception_total.traces,
        perception_total.cell_probes,
        perception_total.share_ops,
        perception_total.detected,
        perception_total.classified,
        perception_total.lost,
        arena.contacts.len(),
        arena.alert_range_cm
    );
    println!(
        "harness_arena_memory: vmrss_start_kib={} vmrss_end_kib={}",
        rss_start_kib.map_or("absent".to_string(), |k| k.to_string()),
        rss_end_kib.map_or("absent".to_string(), |k| k.to_string())
    );
    println!(
        "harness_arena_thresholds: assert={} cut_a_p99_ms={ARENA_P99_MS} cut_s_p99_ms={ARENA_S_P99_MS} engaged_p99_ms={} max_traces_per_pulse={} max_share_ops_per_pulse={} max_ledger_bytes={}",
        knobs.assert,
        knobs.engaged_p99_ms,
        knobs.max_traces_per_pulse,
        knobs.max_share_ops_per_pulse,
        knobs.max_ledger_bytes
    );

    let report = serde_json::json!({
        "what": "16b cut N.1: `Arena::step` at the harness's scale with every unit on the reflex controller",
        "world": {
            "units": placed,
            "pilots": seated,
            "cell_cm": CELL_CM,
            "side_cells": SIDE,
            "passable_cells": passable.len(),
            "seed": knobs.seed,
            "arena_ms": arena.now_ms,
            "move_cost_table": "harness-local (harness_move_cost); the authored arena is cut C's",
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
        "substeps": {
            "per_pulse_min": substeps_min,
            "per_pulse_max": substeps_max,
            "substep_ms": substep,
            "substep_divides_periods": substep_divides_periods,
        },
        "counters": {
            "blocked_events": counts.blocked,
            // 16b cut E.3: and where the refused cells were — the split above,
            // named for what the window can answer rather than for a cause the
            // event does not carry.
            "blocked_interior_passable": interior,
            "blocked_ring_passable": ring,
            "blocked_impassable": impassable,
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
            "sensor_cells": sensor_cells(),
            "share_cells": share_cells(),
            "pass_ms": sensors_cfg.pass_ms,
            "lost_after_passes": sensors_cfg.lost_after_passes,
            "alert_range_cm": arena.alert_range_cm,
            "contacts_at_end": arena.contacts.len(),
            "peak_per_pulse": {
                "passes": peak.passes,
                "candidates": peak.candidates,
                "traces": peak.traces,
                "cell_probes": peak.cell_probes,
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
        "memory": { "vmrss_start_kib": rss_start_kib, "vmrss_end_kib": rss_end_kib },
        "thresholds": {
            "assert": knobs.assert,
            "cut_a_p99_ms": ARENA_P99_MS,
            "cut_s_p99_ms": ARENA_S_P99_MS,
            "engaged_p99_ms": knobs.engaged_p99_ms,
            "max_traces_per_pulse": knobs.max_traces_per_pulse,
            "max_share_ops_per_pulse": knobs.max_share_ops_per_pulse,
            "max_ledger_bytes": knobs.max_ledger_bytes,
        },
    });
    let path = arena_report_path(&knobs.json);
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
    println!("harness_arena_json: {}", path.display());

    if knobs.assert {
        // 16b cut M.2 (`T-05`): the cohort's own substep divides the periods.
        assert!(
            substep_divides_periods,
            "the cohort's {substep} ms substep does not divide the three periods"
        );
        assert!(
            p99 <= f64::from(knobs.engaged_p99_ms),
            "per-pulse p99 {p99:.3} ms is past harness.engaged_p99_ms {}",
            knobs.engaged_p99_ms
        );
        // **The interim lens.** `harness.engaged_p99_ms` (25) is doc 16's ceiling
        // for the *finished* arena — perception, weapons, damage and the reflex
        // controller all inside it. 16b §6 asked cut A for 5 ms with none of that
        // built ([`ARENA_P99_MS`]); cut S.1 built perception, so the cut is
        // measured against [`ARENA_S_P99_MS`], which is the same idea one cut on:
        // motion plus perception, and still well inside the 25 ms the finished
        // arena is allowed. [`ARENA_P99_MS`] stays in the report as the record of
        // what the arena cost before the passes ran.
        assert!(
            p99 <= ARENA_S_P99_MS,
            "per-pulse p99 {p99:.3} ms is past 16b §6's {ARENA_S_P99_MS} ms lens for cut S"
        );
        assert!(
            counts.bytes <= knobs.max_ledger_bytes,
            "ledger bytes {} is past harness.max_ledger_bytes {}",
            counts.bytes,
            knobs.max_ledger_bytes
        );
        // 16b cut S.1, doc 16 §2 W4 and W6: the two per-pulse perception bounds,
        // asserted against the **worst** pulse of the run.
        assert!(
            peak.traces <= knobs.max_traces_per_pulse,
            "traces in one pulse {} is past harness.max_traces_per_pulse {} at \
             sensor_cells={} (doc 16 §2 W1: sensor ranges are short and bounded)",
            peak.traces,
            knobs.max_traces_per_pulse,
            sensor_cells()
        );
        assert!(
            peak.share_ops <= knobs.max_share_ops_per_pulse,
            "share operations in one pulse {} is past harness.max_share_ops_per_pulse {} at \
             share_cells={} (doc 16 §2 W6)",
            peak.share_ops,
            knobs.max_share_ops_per_pulse,
            share_cells()
        );
        // **16b cut N.1**, doc 16 §1 A2's last per-pulse bound: *path queries per
        // pulse under their bound*. The arena's own budget is per `decide_ms`
        // window of arena time (D-16-2: a per-pulse throttle would bite differently
        // at each tempo), and at the `Engaged` period of 500 ms one pulse holds at
        // most one window — so the per-pulse figure the harness asserts and the
        // per-window figure the arena enforces are the same number here.
        assert!(
            reflex_peak.path_queries <= knobs.max_path_queries_per_pulse,
            "path queries in one pulse {} is past harness.max_path_queries_per_pulse {}",
            reflex_peak.path_queries,
            knobs.max_path_queries_per_pulse
        );
    }
}

/// 16b §6's verify lens for cut A: *the harness at 300 moving units stays under
/// 5 ms per pulse before perception and weapons exist.* It is a plan constant and
/// not a configured one — `[tacsim.harness].engaged_p99_ms` is the 25 ms ceiling
/// doc 16 sets for the finished arena — so it lives here, where the cut it bounds
/// is measured.
const ARENA_P99_MS: f64 = 5.0;

/// The same lens two cuts on: **16b cut S.1 added the sensor pass and cut S.2 paid
/// for it**, so the arena's per-pulse cost is motion plus perception and
/// [`ARENA_P99_MS`] is no longer the thing being measured.
///
/// 12 ms, against a measured p99 of **3.9** at the defaults. The lens is left where
/// S.1 set it rather than tightened onto the new measurement, because the sentence
/// that justified 12 is unchanged — it is under half the 25 ms
/// `harness.engaged_p99_ms` doc 16 allows the *finished* arena, so cuts W and N have
/// a budget — and that budget is now a real 8 ms rather than S.1's thin 2.4.
/// Tightening it is the owner's call at the S.2 review, not this file's. A plan
/// constant, not a configured one, exactly as [`ARENA_P99_MS`] is.
///
/// **What S.2 did, and what it did not.** S.1 measured p50 0.299 / p95 8.723 /
/// p99 8.984 / max 9.309 at 24 sensor cells and found the cull at the top of the
/// loaded pulse: each of 300 units scanned all 300 of the index's own rows, with an
/// `i128::isqrt` per pair, because the cell-window branch was only taken while the
/// covered area was no larger than the unit count — about eight cells of reach at
/// this density. Cut S.2 fixed both halves in `arena/index.rs` (`within_cm`, and the
/// cell window preferred wherever the pose is on the window) and the same run is now
/// **p50 0.208 / p95 3.898 / p99 3.933 / max 3.993**: the traces and cell probes are
/// unchanged (2,226 and 31,780 in the peak pulse), so the whole of the difference was
/// the cull.
///
/// **At the authored reaches it is a different story, and the numbers are here
/// because the choice is the owner's.** `TACSIM_SENSOR_CELLS=60
/// TACSIM_SHARE_CELLS=30` — the reach the shipped sensor and `comms` rows actually
/// author — measures **p50 0.703 / p95 22.646 / p99 23.334 / max 24.819**, which is
/// inside doc 16's 25 ms and leaves nothing for cuts W and N. There the cost is the
/// *traces*: 13,133 of them and 435,829 cell probes in the peak pulse, against
/// `harness.max_traces_per_pulse` of 4,000. So the two facts the owner is choosing
/// between are unchanged in kind and now measured in full: **a shorter authored
/// reach**, or **larger bounds and a slower `Engaged` pulse**. Nothing in this cut
/// decides it.
///
/// **16b cut N.1 spent part of that budget and stayed inside it.** With the
/// scripted orders and the scripted fire gone and all 300 units on the reflex
/// controller, the same run measures **p50 0.561 / p95 5.882 / p99 5.918 / max
/// 6.207** (a second run of the same tree: 0.496 / 6.507 / 6.899 / 6.930) — about
/// 2 ms more than cut S.2's, which is the controller's 8,826 ticks, its 1,131 path
/// searches over 190,282 expanded cells, and the 707 shots those decisions
/// produced. The lens is left at 12 for the same reason S.1 left it there: the
/// sentence that justified the number has not changed, and tightening it onto a
/// fresh measurement is the owner's call at a review and not this file's.
///
/// At the **authored** reaches the same run measures p50 1.042 / p95 25.655 / p99
/// 26.374 / max 27.643, past doc 16's 25 — and the cost is still the *traces*
/// (13,278 in the peak pulse against a bound of 4,000), not the controller (57).
/// The owner's choice S.2 recorded is unchanged in kind and now measured with the
/// AI on the field.
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
    use tacsim::arena::state::{Arena, ArenaDefinition, ArenaTerrain, CustodyOut, EntryZone, Team};
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

    // ---- the field: the same generated relief the arena harness runs on ----
    let Window {
        tw,
        cells: all_cells,
        ..
    } = open_window(knobs.seed);
    drop(tw);
    let mut grid = vec![
        tacsim::arena::state::TerrainCell {
            class: TerrainClass::Void,
            elev_cm: 0,
            move_cost: 0,
        };
        usize::from(SIDE) * usize::from(SIDE)
    ];
    for (x, y, class, elev_cm) in &all_cells {
        let at = usize::from(*y) * usize::from(SIDE) + usize::from(*x);
        if let Some(cell) = grid.get_mut(at) {
            *cell = tacsim::arena::state::TerrainCell {
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
    let waypoints = harness_waypoints(&grid);
    let definition = ArenaDefinition {
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
    };

    // Every unit on the reflex controller and every unit armed: the cost of
    // commanding a field is what this measures, and a field that never fights
    // never changes a contact set.
    let mut units = Vec::new();
    let mut taken: std::collections::BTreeSet<(u16, u16)> = std::collections::BTreeSet::new();
    let mut at = 0_usize;
    for n in 0..knobs.units {
        let team = if n % 2 == 0 { Team::Red } else { Team::Blue };
        let cell = loop {
            let Some((x, y, _)) = passable.get(at * 7 % passable.len().max(1)).copied() else {
                break None;
            };
            at = at.saturating_add(1);
            if taken.insert((x, y)) {
                break Some((x, y));
            }
            if at > passable.len().saturating_mul(2) {
                break None;
            }
        };
        let Some((x, y)) = cell else { continue };
        let id = mush_core::ObjId(100_000 + n);
        units.push(tacsim::arena::state::CustodyUnit {
            id,
            team,
            pose: mush_sim::data::Pose {
                x_cm: i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
                y_cm: i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
                z_cm: 0,
                heading_mdeg: 0,
                posture: harness_rules(),
            },
            mobility: harness_mobility(),
            sensor_height_cm: 0,
            sensor: Some(harness_sensor()),
            weapons: vec![harness_weapon()],
            profile: harness_profile(),
            gunnery: 2,
            controller: tacsim::arena::state::Controller::Reflex,
            // 16b cut Q.1: no pod behind it.
            slot: 0,
            unit: harness_unit(id),
        });
    }
    let placed = u32::try_from(units.len()).unwrap_or(0);
    let arena_cfg = ArenaConfig::default();
    let match_cfg = MatchConfig::default();
    let sensors_cfg = SensorsConfig::default();
    let mut arena = Arena::new(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::stock_resolution(),
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
    let started = Instant::now();
    while arena.now_ms < until_ms {
        wall = wall.saturating_add(period_ms);
        let out = arena.step(wall, &[], &[]);
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
