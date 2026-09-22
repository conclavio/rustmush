//! **The shipped-scale rig** (16b cut E.1; 16b §0.4).
//!
//! Every other test in this crate builds its own small window out of hand-written
//! fixtures, which is right for an edge: a sixteen-cell map and a two-cell carbine
//! make a state machine's corner visible in three pulses. What none of them can
//! say is what the machines do on **the map the owner plays on** — 2.56 km of
//! authored ground, a 600 m sight, a 3 km cannon and a 1.5 km inner band — and
//! that is the measure cut E is held to.
//!
//! So this module composes the real thing, and composes it the way production
//! does rather than the way a fixture would:
//!
//! * the **production module roster**, assembled with exactly the calls
//!   `mush_server::boot::game_modules` makes (it is `tests/definitions.rs`'s own
//!   `production_roster`, copied because an integration test cannot import
//!   another integration test);
//! * the **authored arena**, through [`tacsim::match_::definition_for`], so the
//!   ground, the entry zones, the waypoint graph and the muster cells are the
//!   pack's;
//! * the **stock tanks standing in that world**, found by the roster slot the
//!   pods fly ([`tacsim::match_::tank_for_slot`]) and read out of M32 with
//!   [`mush_tactics::module::unit_of`];
//! * each unit assembled by [`tacsim::match_::custody_unit`], which is the
//!   literal `match_::start` uses — 16b §0.4's rule that no fixture may
//!   impersonate a production module, applied to the one assembly that decides
//!   what a machine can see and shoot with;
//! * and each unit **posed by [`tacsim::match_::opening_pose`]**, in `start`'s
//!   own order, side by side and unit by unit, against one shared `taken` list.
//!   That is the step it is easiest to get wrong: `start` does *not* hand the
//!   world pose to the arena, it offers it to `opening_pose`, which keeps it only
//!   when its heading is valid and its cell is on the window, passable and free,
//!   and otherwise relocates the unit into its side's entry zone. A rig that
//!   skipped it would field tanks production could not, and two overrides could
//!   share a cell.
//!
//! A [`Muster`]'s optional cell override is therefore offered **as the world
//! pose**: an override that lands somewhere illegal is corrected exactly as
//! production corrects it.
//!
//! 16b cut E.3 and E.4 reuse this, so it stays small.

#![allow(dead_code)]

use mush_core::ObjId;
use mush_sim::data::Pose;
use mush_tactics::{CellAddr, TacticalUnit, UnitMutationOp, UnitMutationPlan};
use mush_testkit::TestWorld;
use smol_str::SmolStr;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::state::{Arena, Controller, CustodyOut, CustodyUnit, Team};
use tacsim::arena::step::StepOutput;
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};
use tacsim::match_::PROVING_GROUND;

/// The match seed every shipped-scale fixture runs on — `arena.rs`'s and
/// `reflex.rs`'s, so a draw here is the draw they would have made.
pub const SEED: u64 = 2_350;

/// One cell of the shipped window, centimetres.
pub const CELL_CM: u16 = tacsim::match_::ARENA_CELL_CM;

/// **One unit the rig fields**, named the way a pod names its tank: a side and a
/// slot from 1 to 4 (doc 16 §3.8's *slot n of a side*, which is the n-th unit the
/// arena pack authors for it).
#[derive(Clone, Copy, Debug)]
pub struct Muster {
    /// Which side.
    pub team: Team,
    /// The slot, 1 to 4.
    pub slot: u8,
    /// Who flies it. The rig's default is the machine, because that is what a
    /// pod nobody claimed gets.
    pub controller: Controller,
    /// Open somewhere other than the authored muster, as a cell.
    ///
    /// It is offered to [`tacsim::match_::opening_pose`] **as the world pose**,
    /// at that cell's centre and with the unit's own authored heading and
    /// posture, so an override onto a wall or onto another unit's cell is
    /// relocated into the side's entry zone exactly as production relocates one.
    pub at: Option<(u16, u16)>,
    /// **What has already happened to this tank when the match opens** (16b cut
    /// E.4), applied to its M32 record **before** [`Arena::new`] reads it — so
    /// the arena's own check-out sizes the magazines and the health from the
    /// state the case wanted, exactly as it would for a tank that checked in
    /// that way after a previous match.
    pub wear: Option<Wear>,
}

/// **The condition a [`Muster`] opens in** (16b cut E.4).
///
/// Each is written as one [`UnitMutationPlan`] through [`mush_tactics::apply`],
/// so the cells that change — and, for [`Wear::EngineOut`], the section
/// condition — are changed the way the damage pipeline changes them and nothing
/// else moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wear {
    /// [`hurt_to_withdraw`]: under `withdraw_permille`, above
    /// `stand_min_permille`, every section still `intact`.
    Hurt,
    /// [`shot_dry`]: every magazine empty, nothing else touched.
    Dry,
    /// [`engine_struck`] (16b cut AA.1): the engine killed through `hull.rear`,
    /// so the tank checks out with its motion disabled and its gun intact.
    EngineOut,
}

impl Muster {
    /// A machine in slot `slot` of `team`, at its authored muster.
    #[must_use]
    pub fn reflex(team: Team, slot: u8) -> Muster {
        Muster {
            team,
            slot,
            controller: Controller::Reflex,
            at: None,
            wear: None,
        }
    }

    /// The same tank with a pilot in it — the arena schedules it no reflex tick
    /// and the test drives it with intents.
    #[must_use]
    pub fn piloted(team: Team, slot: u8) -> Muster {
        Muster {
            controller: Controller::Pilot,
            ..Muster::reflex(team, slot)
        }
    }

    /// Open at `cell` instead of the authored muster.
    #[must_use]
    pub fn at(self, cell: (u16, u16)) -> Muster {
        Muster {
            at: Some(cell),
            ..self
        }
    }

    /// Open hurt enough to withdraw (16b cut E.4) — see [`hurt_to_withdraw`].
    #[must_use]
    pub fn hurt(self) -> Muster {
        Muster {
            wear: Some(Wear::Hurt),
            ..self
        }
    }

    /// Open with every magazine empty (16b cut E.4) — see [`shot_dry`].
    #[must_use]
    pub fn dry(self) -> Muster {
        Muster {
            wear: Some(Wear::Dry),
            ..self
        }
    }

    /// Open with the engine already destroyed (16b cut AA.1) — see
    /// [`engine_struck`].
    #[must_use]
    pub fn engine_out(self) -> Muster {
        Muster {
            wear: Some(Wear::EngineOut),
            ..self
        }
    }
}

/// **A tank hurt to just under `withdraw_permille`** (16b cut E.4), by its
/// outcome and not by a recipe.
///
/// Every **facet** `armor` cell is set to its own `min + 1` and no `internals`
/// cell is touched, so every section stays `intact` — nothing here is a
/// consequence, and the machine that checks out of it is a whole tank that has
/// had its plate shot off rather than a half-wrecked one.
///
/// `Arena::reflex_health_permille` sums **every** `armor` and `internals` cell,
/// each from its own floor, so on the shipped `line_tank` — eleven armoured
/// facets totalling 2,320, and 670 of internals over five sections — the answer
/// is (11 + 670) of 2,990, **227 per mille**: under the shipped
/// `withdraw_permille` of 350, so the machine runs for home on its first decide
/// tick, and above `stand_min_permille` of 175, so the run home may still turn
/// and fight.
///
/// This is the helper the plan asks for because `tests/reflex.rs`'s two-cell
/// `hurt` recipe does not scale: the same two ops leave the shipped tank at 800
/// per mille, a healthy machine that never withdraws and a case that observes
/// nothing.
#[must_use]
pub fn hurt_to_withdraw(unit: &TacticalUnit) -> TacticalUnit {
    let ops: Vec<UnitMutationOp> = unit
        .cells()
        .filter_map(|(addr, cell)| match addr {
            CellAddr::Facet { .. } if cell_named(addr) == Some("armor") => {
                Some(UnitMutationOp::SetCell {
                    addr: addr.clone(),
                    value: cell.min.saturating_add(1).min(cell.max),
                })
            }
            _ => None,
        })
        .collect();
    apply_plan(unit, ops, "test:e4-hurt")
}

/// **A tank whose every magazine is empty** (16b cut E.4).
///
/// Every **component** `ammo` cell to nought, which is what `Arena::new` sizes
/// `UnitState::ammo` from: the rows are still there (so `armed_at_checkout` is
/// true and the tank is `is_dry`, not unarmed), and they all read zero.
#[must_use]
pub fn shot_dry(unit: &TacticalUnit) -> TacticalUnit {
    let ops: Vec<UnitMutationOp> = unit
        .cells()
        .filter_map(|(addr, cell)| match addr {
            CellAddr::Component { .. } if cell_named(addr) == Some("ammo") => {
                Some(UnitMutationOp::SetCell {
                    addr: addr.clone(),
                    value: cell.min.max(0),
                })
            }
            _ => None,
        })
        .collect();
    apply_plan(unit, ops, "test:e4-dry")
}

/// **A tank whose engine was killed through its rear plate** (16b cut AA.1; 16c
/// `B-76`), written as exactly what `Arena::plan_hit` writes on the transition
/// and nothing else, in one plan: the `hull` facet `rear` `armor` cell to its
/// `min`, the `engine` section's `internals` cell to its `min`, and
/// `SetSectionCondition { engine, destroyed }`.
///
/// **The condition op is not optional.** `damage::entry_consequences` collects
/// destroyed sections by their CONDITION and never by a cell, so the two cells
/// alone would check out a mobile tank. With it, the arena's own check-out reads
/// the engine's `disable_motion` and opens the tank with
/// `limits.motion_disabled` set — the fact the case is about, arrived at the way
/// production arrives at it for a tank that checked in that way.
///
/// Conditions are not counted by `Arena::reflex_health_permille`, so on the
/// shipped `line_tank` the health is (2,990 − 150 − 120) of 2,990: 909 per mille,
/// far above `withdraw_permille`. The turret and its gun are untouched, so the
/// tank is armed, not `disabled`, and not out of action.
#[must_use]
pub fn engine_struck(unit: &TacticalUnit) -> TacticalUnit {
    let floor = |addr: CellAddr| -> UnitMutationOp {
        let min = unit
            .cell(&addr)
            .unwrap_or_else(|| panic!("the stock tank has {addr:?}"))
            .min;
        UnitMutationOp::SetCell { addr, value: min }
    };
    let section = |name: &str| {
        mush_tactics::SectionId::new(name).unwrap_or_else(|why| panic!("{name}: {why:?}"))
    };
    let cell = |name: &str| {
        mush_tactics::CellId::new(name).unwrap_or_else(|why| panic!("{name}: {why:?}"))
    };
    let ops = vec![
        floor(CellAddr::Facet {
            section: section("hull"),
            facet: mush_tactics::FacetId::new("rear").unwrap_or_else(|why| panic!("rear: {why:?}")),
            cell: cell(tacsim::definitions::unit::ARMOR_CELL),
        }),
        floor(CellAddr::Section {
            section: section("engine"),
            cell: cell(tacsim::definitions::unit::INTERNALS_CELL),
        }),
        UnitMutationOp::SetSectionCondition {
            section: section("engine"),
            condition: mush_tactics::ConditionId::new(tacsim::arena::damage::DESTROYED)
                .unwrap_or_else(|why| panic!("destroyed: {why:?}")),
        },
    ];
    apply_plan(unit, ops, "test:aa1-engine")
}

/// The cell name one address carries, whatever kind of address it is.
fn cell_named(addr: &CellAddr) -> Option<&str> {
    match addr {
        CellAddr::Facet { cell, .. }
        | CellAddr::Section { cell, .. }
        | CellAddr::Unit { cell }
        | CellAddr::Component { cell, .. } => Some(cell.as_str()),
    }
}

/// One plan, applied through M32's own `apply` — refuse-not-clamp, journalled,
/// atomic.
fn apply_plan(unit: &TacticalUnit, ops: Vec<UnitMutationOp>, reason: &str) -> TacticalUnit {
    if ops.is_empty() {
        return unit.clone();
    }
    let plan = UnitMutationPlan::new(ops, None, Some(SmolStr::new(reason)))
        .unwrap_or_else(|why| panic!("{reason} builds a plan: {why:?}"));
    mush_tactics::apply(unit, &plan, 0)
        .unwrap_or_else(|why| panic!("{reason} applies: {why:?}"))
        .0
}

/// The production roster, composed exactly as `mush_server::boot` composes it,
/// with the game's own boot packs on.
///
/// Copied from `tests/definitions.rs`, which is the only way one integration
/// test can share a composition with another.
#[must_use]
pub fn production_roster() -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = std::sync::Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let mut mods = comp::server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
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
    mods.extend(comp::ruleset_modules(&onboarding, &game));
    mods.push(comp::tacsim_module(cfg));
    // `[game].starter_area` off: the arena's ground is in a region of its own and
    // the shipped lot would only add rooms no window must anchor on.
    mods.push(comp::content_module_with(false, true).expect("the content module composes"));
    mods
}

/// A world with the game's own content applied and the eight stock tanks
/// standing on the proving ground.
#[must_use]
pub fn booted_world() -> TestWorld {
    let tw = TestWorld::with_modules(production_roster());
    let tanks = tacsim::match_::centre_readiness(tw.world()).1;
    assert_eq!(
        tanks, 8,
        "the shipped content did not field its eight stock tanks; the rig has nothing to compose"
    );
    tw
}

/// **The shipped-scale arena**: the authored proving ground, fielding the tanks
/// `sides` names, assembled and posed exactly as [`tacsim::match_::start`] does.
///
/// `sides` is walked in order and each side's musters in order, which is
/// `start`'s own loop and therefore the order the shared `taken` list is filled
/// in: two overrides can never land on one cell, and the second of them is
/// relocated rather than refused.
#[must_use]
pub fn shipped_arena(world: &TestWorld, sides: &[&[Muster]]) -> Arena {
    let w = world.world();
    let definition = tacsim::match_::definition_for(w, &SmolStr::new_static(PROVING_GROUND))
        .expect("the proving ground's ground builds")
        .expect("the proving ground is shipped");
    let catalog = w
        .module_state::<tacsim::TacsimStore>()
        .map(|slot| slot.definitions().clone())
        .unwrap_or_default();
    let mut taken: Vec<(u16, u16)> = Vec::new();
    let mut units: Vec<CustodyUnit> = Vec::new();
    for side in sides {
        for muster in *side {
            let id = tank(world, muster.team, muster.slot);
            let cells = mush_tactics::module::unit_of(w, id)
                .unwrap_or_else(|| panic!("#{} is a tactical unit", id.0))
                .clone();
            // **16b cut E.4**: the wear goes on before `Arena::new`, so the
            // check-out reads the magazines and the plate the case wanted.
            let cells = match muster.wear {
                Some(Wear::Hurt) => hurt_to_withdraw(&cells),
                Some(Wear::Dry) => shot_dry(&cells),
                Some(Wear::EngineOut) => engine_struck(&cells),
                None => cells,
            };
            let placed = mush_sim::pose_of(w, id).map(|(_, pose)| pose);
            // The override is offered as the world pose, so `opening_pose`
            // corrects an illegal one the way production corrects it.
            let world_pose = match (muster.at, placed) {
                (Some(cell), Some(pose)) => Some(at_cell(cell, &pose)),
                (Some(_), None) => None,
                (None, placed) => placed,
            };
            let pose =
                tacsim::match_::opening_pose(&definition, muster.team, world_pose, &mut taken)
                    .unwrap_or_else(|| panic!("{:?} has an entry cell", muster.team));
            units.push(tacsim::match_::custody_unit(
                &catalog,
                cells,
                id,
                muster.team,
                pose,
                0,
                muster.controller,
                muster.slot,
            ));
        }
    }
    Arena::new(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: SEED,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the shipped-scale arena checks out")
}

/// The object a side's slot fields, the way [`shipped_arena`] resolves it.
#[must_use]
pub fn tank(world: &TestWorld, team: Team, slot: u8) -> ObjId {
    tacsim::match_::tank_for_slot(world.world(), team, slot)
        .unwrap_or_else(|| panic!("{team:?} slot {slot} is fielded"))
}

/// The cell a unit stands in, or `None` when it is not on the window.
#[must_use]
pub fn cell_of(arena: &Arena, id: ObjId) -> Option<(u16, u16)> {
    arena
        .unit(id)
        .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
}

/// A pose at `cell`'s centre keeping `from`'s heading and posture — what a
/// [`Muster::at`] override is offered as.
#[must_use]
pub fn at_cell(cell: (u16, u16), from: &Pose) -> Pose {
    let half = i64::from(CELL_CM / 2);
    Pose {
        x_cm: i64::from(cell.0) * i64::from(CELL_CM) + half,
        y_cm: i64::from(cell.1) * i64::from(CELL_CM) + half,
        z_cm: 0,
        heading_mdeg: from.heading_mdeg,
        posture: from.posture.clone(),
    }
}

/// **Step the arena to `until_ms` of arena time at a fixed wall cadence**,
/// handing every pulse to `each_pulse` and answering the ledger bytes the run
/// produced.
///
/// The pulse hook is where a shipped-scale assertion lives: the values a test
/// wants — a machine's goal, its state, its ordered speed — are per tick and a
/// run of ten minutes has six hundred of them, so a property is checked as the
/// run goes rather than reconstructed afterwards.
pub fn run_until(
    arena: &mut Arena,
    until_ms: i64,
    cadence_ms: i64,
    mut each_pulse: impl FnMut(&Arena, &StepOutput),
) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = arena.now_ms;
    while arena.now_ms < until_ms {
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        each_pulse(arena, &out);
        if out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    bytes
}

/// Everything the black box holds, flushed — the shape every other test in this
/// crate reads its ledger through.
pub fn ledger_events(arena: &mut Arena, bytes: &mut Vec<u8>) -> Vec<LedgerEvent> {
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    String::from_utf8(bytes.clone())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .collect()
}

/// **How often [`Timeline`] takes a reading**, arena milliseconds.
pub const TIMELINE_EVERY_MS: i64 = 10_000;

/// **What the machines were doing, every ten arena seconds** (16b cut E.4).
///
/// A shipped-scale endgame case that fails has failed after fifteen minutes of
/// arena time, and the number it prints — "it had not ended" — is not a
/// diagnosis. This is: one line per unit per sample, with the state, the cell,
/// the goal, the target and the health, so the failure message says what the
/// machines were *doing* at 300 s, 600 s and 900 s.
///
/// The `path_failures` it carries is the run's **field-wide** running total, not
/// one unit's: [`tacsim::arena::reflex::ReflexCounters`] is per pulse and per
/// field, and there is no per-unit failure count in the tree to read.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    next_ms: i64,
    failures: u32,
    lines: Vec<String>,
}

impl Timeline {
    /// An empty timeline, due its first reading at 0 ms.
    #[must_use]
    pub fn new() -> Timeline {
        Timeline::default()
    }

    /// Take a reading if this pulse crossed the next ten-second mark, and add
    /// `path_failures` to the running total either way. Called from
    /// [`run_until`]'s pulse hook.
    pub fn sample(&mut self, arena: &Arena, path_failures: u32) {
        self.failures = self.failures.saturating_add(path_failures);
        if arena.now_ms < self.next_ms {
            return;
        }
        self.next_ms = arena
            .now_ms
            .saturating_sub(arena.now_ms.rem_euclid(TIMELINE_EVERY_MS))
            .saturating_add(TIMELINE_EVERY_MS);
        for (id, row) in arena.reflex.all() {
            let cell = cell_of(arena, id);
            self.lines.push(format!(
                "  {:>7} ms #{:<4} {:<9} at {:?} goal {:?} target {:?} health {} ‰ (field \
                 path_failures {})",
                arena.now_ms,
                id.0,
                row.state.as_str(),
                cell,
                row.goal,
                row.target.map(|(_, target)| target.0),
                arena.reflex_health_permille(id),
                self.failures
            ));
        }
    }

    /// Every line taken, for a failure message.
    #[must_use]
    pub fn render(&self) -> String {
        self.lines.join("\n")
    }
}

/// The object id a `fired`, `blocked` or `destroyed` line names.
#[must_use]
pub fn unit_of_event(event: &LedgerEvent) -> Option<ObjId> {
    serde_json::to_value(&event.kind)
        .ok()
        .and_then(|value| value.get("unit").and_then(serde_json::Value::as_u64))
        .and_then(|id| u32::try_from(id).ok())
        .map(ObjId)
}
