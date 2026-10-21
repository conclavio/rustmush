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

/// **16b cut I.1**: the fixture yard both fixture blessers draw on.
pub mod yard;

/// **16b cut AE.1**: the proving ground's generator, moved here from
/// `tests/content.rs` so the version-1 ground a shelf test boots on can be
/// built ([`ground::atlas_pack_at`]).
pub mod ground;
/// **17b cut M5.1**: the holdings seam's test provider — Standard Doctrine and
/// two scripted fixture minds, installed through the production seam.
pub mod holdings;
/// **Doc 20 cut SM.5**: a prospecting licence bought through `+licence/buy go`, for every
/// suite whose pilots lease a rig.
pub mod licence;
/// **Doc 19 cut A.8**: the moon ports stripped away for the v7-shaped lane variants.
pub mod moon;
/// **Doc 20 cut SM.5**: the ore bids, the fixed references, the licence block and
/// Occator's raise stripped away for the v10-shaped lane variants.
pub mod ore;
pub mod rig;
pub mod scripted;
/// **Doc 20 cut SM.2**: the mining sites stripped away for the v8-shaped lane variants; since
/// SM.6 also the helpers that open a keyed site.
pub mod sites;
/// **Doc 20 cut SM.3a**: Occator's rigs and the refit row stripped away for the v9-shaped
/// lane variants.
pub mod vehicles;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use mush_core::ObjId;
use mush_sim::data::Pose;
use mush_tactics::{CellAddr, TacticalUnit, UnitMutationOp, UnitMutationPlan};
use mush_testkit::TestWorld;
use smol_str::SmolStr;
use tacsim::arena::intents::IntentKind;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::mind::{MindRunner, ThinkJob, ThinkResult, decode_memory};
use tacsim::arena::state::{Arena, Controller, CustodyOut, CustodyUnit, MatchEnd, Team};
use tacsim::arena::step::StepOutput;
use tacsim::arena::stock_mind::{NativeStockMind, StockMemory};
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};
use tacsim::match_::PROVING_GROUND;
use tacsim::minds::{RhaiRunner, ThinkStat};

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
    /// Who flies it. The rig's default is a unit mind ([`Muster::mind`]),
    /// because that is what a pod nobody claimed gets (17b cut M4.3's fill).
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
    /// **Fielded with no weapons station** (17b cut M2.5; D-UM-48): the stock
    /// tank with its weapon rows taken off the custody unit
    /// [`tacsim::match_::custody_unit`] assembled ([`Muster::mind_unarmed`]).
    pub unarmed: bool,
    /// **Sealed on a scripted mind** (17b cut M3.5; doc 17 §13): the rig seals
    /// this hull `scripted` (`SealTerms::engine_for_minds`) on Standard
    /// Doctrine's source, and installs its runner through the production
    /// installer (`tacsim::minds::install_sealed_runners`) after
    /// [`Arena::new`] ([`Muster::scripted`]).
    pub scripted: bool,
    /// **Sealed as the production fill is** (17b cut M4.3; doc 17 §9): the
    /// rig seals this hull under `SealTerms::production_fixture()` —
    /// `PRODUCTION_FILL_ENGINE`, rated I, capped at the default
    /// `[tacsim.minds].fill_class` — the terms `tacsim::match_::start` seals
    /// every hull under at the stock keys, so a pin that fields the production
    /// shape ([`Muster::fill`]) hashes as production does; the production
    /// installer runs over it as over a scripted muster's, and since the fill
    /// decision of 2026-10-09 (`PRODUCTION_FILL_ENGINE` is `native`) it
    /// installs nothing for it: the hull thinks on the native fallback.
    pub production: bool,
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
    /// **The one base every constructor is built on** (17b cut M4.5): slot
    /// `slot` of `team` flown by `controller`, at its authored muster, unworn,
    /// armed, sealed under the fixture terms — the defaults every fixture's
    /// seal digest was taken with, so re-basing a constructor on it moves no
    /// digest.
    fn base(team: Team, slot: u8, controller: Controller) -> Muster {
        Muster {
            team,
            slot,
            controller,
            at: None,
            wear: None,
            unarmed: false,
            scripted: false,
            production: false,
        }
    }

    /// **The production fill** (17b cut M4.3; doc 17 §9, plan finding 5): a
    /// [`Muster::mind`] sealed as `tacsim::match_::start` seals an unseated
    /// hull — Standard Doctrine on `PRODUCTION_FILL_ENGINE` at the default
    /// `fill_class`, the production installer run over it — so a pin that
    /// fields it hashes as a production match does. [`Muster::mind`] keeps the
    /// fixture terms (`Native`, rated I, capped III) so the twin's T1 and
    /// every `.sealed()` mind fixture keep their digests. Since the fill
    /// decision of 2026-10-09 the constant is `Native`, so this hull's seal is
    /// [`Muster::mind`]'s byte for byte (the cap seals no byte: `min(I, core,
    /// I)` is `min(I, core, III)` on every shipped core) and the installer
    /// gives it nothing; it stays the one name for the production shape
    /// whatever the constant reads.
    #[must_use]
    pub fn fill(team: Team, slot: u8) -> Muster {
        Muster {
            production: true,
            ..Muster::base(team, slot, Controller::Mind)
        }
    }

    /// The same tank with a pilot in it — no machine thinks for it (the arena
    /// schedules it no think) and the test drives it with intents.
    #[must_use]
    pub fn piloted(team: Team, slot: u8) -> Muster {
        Muster::base(team, slot, Controller::Pilot)
    }

    /// **The same tank flown by a unit mind** (17b cut M1a.5; doc 17 §2,
    /// D-UM-2): `Arena::new` lays a mind row down and schedules its first think,
    /// and the test hands the arena the runner it thinks with
    /// (`Arena::mind_runner`) — the default runner, the native stock mind
    /// (17b cut M1a.6), until it does.
    #[must_use]
    pub fn mind(team: Team, slot: u8) -> Muster {
        Muster::base(team, slot, Controller::Mind)
    }

    /// **A [`Muster::mind`] with no weapons station** (17b cut M2.5; D-UM-48,
    /// doc 17 §22's italics: the twin corpus carries unarmed hulls). The stock
    /// tank's M32 record is fielded whole — its plate, its internals, its
    /// sensor, its drive — and its weapon rows are taken off the custody unit
    /// that [`tacsim::match_::custody_unit`] assembled, before
    /// [`Arena::new`] reads it, the way `stock_mind.rs`' unarmed fixture clears
    /// a scout's `weapons`: the check-out lays down no `ammo` row, so the hull
    /// is unarmed (`armed_at_checkout` false), never dry, and its view carries
    /// no weapon row.
    #[must_use]
    pub fn mind_unarmed(team: Team, slot: u8) -> Muster {
        Muster {
            unarmed: true,
            ..Muster::mind(team, slot)
        }
    }

    /// **A [`Muster::mind`] sealed on a scripted mind** (17b cut M3.5; doc 17
    /// §13): a `Controller::Mind` hull whose seal says
    /// [`tacsim::arena::SealEngine::Scripted`] with Standard Doctrine's source
    /// digest (`SealTerms::engine_for_minds`), its runner a `RhaiRunner`
    /// installed on `Arena::runners` by the production installer after
    /// [`Arena::new`] — never by `with_rhai`, and never the arena-wide
    /// fallback, which stays the native stock mind for every other hull.
    #[must_use]
    pub fn scripted(team: Team, slot: u8) -> Muster {
        Muster {
            scripted: true,
            ..Muster::mind(team, slot)
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
/// `Arena::health_permille` sums **every** `armor` and `internals` cell,
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
/// Conditions are not counted by `Arena::health_permille`, so on the
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
    // Doc 18 cut IC5: `tacsim` the third quantity delegate, the inventory cell
    // handed to the layer below.
    let (mut mods, inventory) = comp::server_modules_with_tacsim(
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
    mods.extend(comp::sheet_modules(&onboarding, &game));
    mods.push(comp::tacsim_module_wired(cfg, false, inventory));
    // `[game].starter_area` off: the arena's ground is in a region of its own and
    // the shipped lot would only add rooms no window must anchor on.
    mods.push(comp::content_module_with(false, true).expect("the content module composes"));
    mods
}

/// A world with the game's own content applied and the twelve stock tanks
/// (six a side since 16b cut Z.3) standing on the proving ground.
#[must_use]
pub fn booted_world() -> TestWorld {
    let tw = TestWorld::with_modules(production_roster());
    let tanks = tacsim::match_::centre_readiness(tw.world()).tanks;
    assert_eq!(
        tanks,
        2 * usize::from(tacsim::match_::PODS_PER_SIDE),
        "the shipped content did not field its twelve stock tanks; the rig has nothing to compose"
    );
    tw
}

// ---------------------------------------------------------------------------
// 16b cut AE.1 — a world standing at the atlas's version 1 and the arena's 4
// ---------------------------------------------------------------------------

/// The digest `tacsim.proving_ground@1` — the atlas region pack, the ground
/// with no bluff — shipped with (16b cut C.2), as `tests/content.rs` pinned it
/// until cut AE.1. [`ground::atlas_pack_at`]`(false)` is held to it, which is
/// the proof that moving the generator changed no byte of version 1.
pub const ATLAS_V1_DIGEST: &str =
    "b0bcabf34632799f1468821c1ddfeefb4acbe30282b626ca5e02fbf32f659706";

/// The digest `tacsim.pack.proving_ground@4` shipped with (16b cut O.1), as
/// `tests/content.rs` pinned it until cut AE.1.
pub const ARENA_V4_DIGEST: &str =
    "fd5860b51a60376725ec5fbe86797e8acef969db9a34fc70c87a23ea56c21b08";

/// The arena entry's `note` as version 4 carried it, verbatim — 1,009 of the
/// note's 1,024 bytes. Version 5 had to say the same in fewer words to make
/// room for its own sentence, so this is the one string of version 4 that
/// cannot be derived from version 5.
pub const ARENA_V4_NOTE: &str = "D-16-5 fixes the window: 256 by 256 cells of 1,000 cm at \
    20,000 m, clear of the lot and the harness. Entry zones are sixteen cells deep. The key \
    spots are the ford, the crossroads and the two compounds. The waypoint graph is two roads \
    and four spurs, so Patrol's route is not a straight line. VERSION 2 (16b cut F.4) moves the \
    muster onto the ford: version 1 mustered on rows 118 to 130, and the river (columns 62 to \
    66) stopped a tank at deep water. The ford is rows 92 to 102; slot 1 stands on the \
    east-west road at row 97 and slots 2 to 4 at 95, 99 and 93, so every tank drives straight \
    across. The muster nodes moved to row 105. VERSION 3 (16b cut Z.3) adds slots 5 and 6 a \
    side: row 101 in the muster column and row 94 one behind it, inside the ford, a clear cell \
    from every other slot and on no other slot's straight road. VERSION 4 (16b cut O.1) \
    declares the modes the ground supports: hold at The Crossroads, six cells, 120 s, on the \
    east-west road 120 cells from the blue muster and 123 from the red.";

/// The key spot version 5 added (16b cut AE.1).
pub const AE1_SPOT: &str = "The Bluff";

/// **Version 4 of the arena pack** (16b cut AE.1), rebuilt from the shipped
/// version 5 by taking The Bluff back out of `key_spots` and putting version
/// 4's note back ([`ARENA_V4_NOTE`]), the version set to 4 and the digest
/// re-derived — and then **held to version 4's own pinned digest**
/// ([`ARENA_V4_DIGEST`]), O.1's [`arena_pack_v3`] precedent one version on.
#[must_use]
pub fn arena_pack_v4() -> String {
    let mut doc: serde_json::Value =
        serde_json::from_str(tacsim::definitions::content::PROVING_GROUND_JSON)
            .expect("the arena pack is json");
    let def = &mut doc["entries"][PROVING_GROUND]["def"];
    let spots = def["key_spots"]
        .as_array_mut()
        .expect("the arena authors key spots");
    let before = spots.len();
    spots.retain(|spot| spot["name"] != AE1_SPOT);
    assert_eq!(spots.len() + 1, before, "version 5 authors The Bluff");
    assert!(
        def["note"]
            .as_str()
            .is_some_and(|note| note.contains("VERSION 5 (16b cut AE.1)")),
        "version 5's note carries its own sentence"
    );
    def["note"] = serde_json::Value::from(ARENA_V4_NOTE);
    doc["manifest"]["version"] = serde_json::Value::from(4);
    doc["manifest"]["digest"] = serde_json::Value::from("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = mush_content::parse_pack(&unsigned)
        .expect("version 4 parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    assert_eq!(
        digest, ARENA_V4_DIGEST,
        "the rebuilt fixture is not version 4: version 5 changed more than the bluff and the note"
    );
    doc["manifest"]["digest"] = serde_json::Value::from(digest);
    serde_json::to_string(&doc).expect("json")
}

/// The boot packs of a world that **took neither the atlas's version 2 nor the
/// arena's version 5** (16b cut AE.1) — dev1's world as it stood before the
/// cut: the shipped starter area (twelve pods), the ground at version 1
/// ([`ground::atlas_pack_at`]`(false)`, held to [`ATLAS_V1_DIGEST`]) and
/// [`arena_pack_v4`] (twelve tanks, four key spots, the Hold declaration).
/// `starter_area` false leaves the lot out, for a rig that needs only the
/// arena.
#[must_use]
pub fn boot_packs_at_arena_v4(starter_area: bool) -> Vec<mush_content::Pack> {
    let mut packs = if starter_area {
        mush_atlas::starter::packs().expect("the starter packs load")
    } else {
        Vec::new()
    };
    let mut game = tacsim::definitions::content::boot_packs().expect("the game packs load");
    let arena = game.pop().expect("the arena pack boots last");
    assert_eq!(
        arena.manifest().id().as_str(),
        tacsim::definitions::content::PROVING_GROUND_PACK
    );
    let atlas = game.pop().expect("the atlas pack boots before it");
    assert_eq!(
        atlas.manifest().id().as_str(),
        tacsim::definitions::content::PROVING_GROUND_ATLAS_PACK
    );
    packs.extend(game);
    let ground =
        mush_content::load_pack(&ground::atlas_pack_at(false)).expect("version 1 verifies");
    assert_eq!(
        ground.manifest().digest().to_hex(),
        ATLAS_V1_DIGEST,
        "the rebuilt ground is not version 1"
    );
    packs.push(ground);
    packs.push(mush_content::load_pack(&arena_pack_v4()).expect("version 4 verifies"));
    packs
}

// ---------------------------------------------------------------------------
// 16b cut O.1 — a world standing at the arena's version 3
// ---------------------------------------------------------------------------

/// The digest `tacsim.pack.proving_ground@3` shipped with (16b cut Z.3), as
/// `tests/content.rs` pinned it until cut O.1.
pub const ARENA_V3_DIGEST: &str =
    "b75b98fd6d1669499384ea3fbf3ac54adaf06f90d0e34625094e9449c1a91c83";

/// The arena entry's `note` as version 3 carried it, verbatim. Version 4 had
/// to say the same in fewer words to make room for its own sentence under the
/// note's 1,024-byte bound, so this is the one string of version 3 that cannot
/// be derived from version 4.
pub const ARENA_V3_NOTE: &str = "D-16-5 fixes the window: 256 by 256 cells at 1,000 cm, 2.56 km \
    square, offset to 20,000 m so its frame cannot touch the lot's or the harness's. The entry \
    zones are sixteen cells deep on each short side. The key spots are where the ground decides \
    something: the ford, the crossroads, the two compounds. The waypoint graph is the two roads \
    and four spurs, the smallest on which Patrol's route is not a straight line. VERSION 2 (16b \
    cut F.4) moves the muster onto the ford: version 1 mustered on rows 118 to 130, and the \
    river (columns 62 to 66) stopped a tank at deep water 550 m out. The ford is rows 92 to 102; \
    slot 1 stands on the east-west road at row 97 and slots 2 to 4 at 95, 99 and 93, so every \
    tank drives straight across. The muster nodes moved to row 105. VERSION 3 (16b cut Z.3) \
    adds slots 5 and 6 a side: row 101 in the muster column and row 94 one column behind it, \
    both inside the ford's rows 92 to 102, a clear cell from every other slot and on no other \
    slot's straight road east or west.";

/// **Version 3 of the arena pack** (16b cut O.1), rebuilt from version 4 by
/// taking its `modes` back out and putting version 3's note back
/// ([`ARENA_V3_NOTE`]), the version set to 3 and the digest re-derived — and
/// then **held to version 3's own pinned digest** ([`ARENA_V3_DIGEST`]), Z.3's
/// [`arena_pack_v2`] precedent one version on.
///
/// **16b cut AE.1 rebuilds it from [`arena_pack_v4`]'s string** and not from
/// the shipped bytes, which are version 5 now: the chain is v5 → v4 → v3 → v2,
/// each step undoing exactly one version's change, so the version 4 assertion,
/// the note restore and the [`ARENA_V3_DIGEST`] pin below stand as O.1 wrote
/// them.
#[must_use]
pub fn arena_pack_v3() -> String {
    let mut doc: serde_json::Value =
        serde_json::from_str(&arena_pack_v4()).expect("the arena pack is json");
    let def = &mut doc["entries"][PROVING_GROUND]["def"];
    assert!(
        def.as_object_mut()
            .expect("the arena entry is an object")
            .remove("modes")
            .is_some(),
        "version 4 declares its modes"
    );
    assert!(
        def["note"]
            .as_str()
            .is_some_and(|note| note.contains("VERSION 4 (16b cut O.1)")),
        "version 4's note carries its own sentence"
    );
    def["note"] = serde_json::Value::from(ARENA_V3_NOTE);
    doc["manifest"]["version"] = serde_json::Value::from(3);
    doc["manifest"]["digest"] = serde_json::Value::from("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = mush_content::parse_pack(&unsigned)
        .expect("version 3 parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    assert_eq!(
        digest, ARENA_V3_DIGEST,
        "the rebuilt fixture is not version 3: version 4 changed more than `modes` and the note"
    );
    doc["manifest"]["digest"] = serde_json::Value::from(digest);
    serde_json::to_string(&doc).expect("json")
}

/// The boot packs of a world that **took the arena's version 3 and not its
/// version 4** (16b cut O.1) — dev1's world as it was wiped on 2026-09-26: the
/// shipped starter area (twelve pods), the arena's ground, and
/// [`arena_pack_v3`] (twelve tanks, no Hold declaration on the latch). The
/// ground is the shipped atlas pack, which is version 2 (The Bluff) since 16b
/// cut AE.1 where dev1's world held version 1: this rig is the arena pack's
/// chain, and the world that still holds the atlas at version 1 is
/// [`boot_packs_at_arena_v4`].
/// `starter_area` false leaves the lot out, for a rig that needs only the
/// arena.
#[must_use]
pub fn boot_packs_at_arena_v3(starter_area: bool) -> Vec<mush_content::Pack> {
    let mut packs = if starter_area {
        mush_atlas::starter::packs().expect("the starter packs load")
    } else {
        Vec::new()
    };
    let mut game = tacsim::definitions::content::boot_packs().expect("the game packs load");
    let arena = game.pop().expect("the arena pack boots last");
    assert_eq!(
        arena.manifest().id().as_str(),
        tacsim::definitions::content::PROVING_GROUND_PACK
    );
    packs.extend(game);
    packs.push(mush_content::load_pack(&arena_pack_v3()).expect("version 3 verifies"));
    packs
}

// ---------------------------------------------------------------------------
// 16b cut Z.3 — a world standing at the arena's version 2
// ---------------------------------------------------------------------------

/// The digest `tacsim.pack.proving_ground@2` shipped with (16b cut F.4), as
/// `tests/content.rs` pinned it until cut Z.3.
pub const ARENA_V2_DIGEST: &str =
    "0ecc97a0922b2fef3d18fd2b510c2a17cdef1613de10ee56b9fce99520924160";

/// The four slots the arena's version 3 added (16b cut Z.3), in authored order.
pub const Z3_TANKS: [&str; 4] = ["red_5", "red_6", "blue_5", "blue_6"];

/// The arena entry's `note` as version 2 carried it, verbatim. Version 3 had
/// to say the same in fewer words to make room for its own sentence under the
/// note's 1,024-byte bound, so this is the one string of version 2 that cannot
/// be derived from version 3.
const ARENA_V2_NOTE: &str = "D-16-5 fixes the window: 256 by 256 cells at 1,000 cm, 2.56 km \
    square. The region is offset to 20,000 m so the arena's frame cannot touch the shipped \
    lot's or the harness's. The entry zones are sixteen cells deep on each short side, 160 m \
    of ground behind a tank at its start. The four key spots are where the ground decides \
    something: the ford, the crossroads, the two compounds. The waypoint graph is the two \
    roads and four spurs, the smallest graph on which Patrol makes a route that is not a \
    straight line. VERSION 2 (16b cut F.4) moves the muster onto the ford. Version 1 mustered \
    on rows 118 to 130, which the river cuts at columns 62 to 66, so a tank pointed at the \
    enemy stopped at deep water 550 m out. The ford is rows 92 to 102: all eight slots stand \
    in it, slot 1 on the east-west road at row 97 and the rest at 95, 99 and 93, a clear cell \
    between each pair, so every tank can drive straight across. The muster nodes moved to row \
    105. The columns and the entry zones are unchanged.";

/// **Version 2 of the arena pack** (16b cut Z.3), rebuilt from version 3 by
/// taking its four tanks back out and putting version 2's note back
/// ([`ARENA_V2_NOTE`]), the version set to 2 and the digest re-derived — and
/// then **held to version 2's own pinned digest**, so the fixture is version 2
/// in everything the digest covers and not merely something like it.
/// (`mush-atlas`'s `tests/interior.rs` builds its OOC fixtures the same way.)
///
/// **16b cut O.1 rebuilds it from [`arena_pack_v3`]'s string** and not from the
/// shipped bytes, which are version 5 now (16b cut AE.1): the chain is v5 → v4
/// → v3 → v2, so each step undoes exactly one version's change and the version 3 assertion, the
/// note restore and the [`ARENA_V2_DIGEST`] pin below stand as Z.3 wrote them.
#[must_use]
pub fn arena_pack_v2() -> String {
    let mut doc: serde_json::Value =
        serde_json::from_str(&arena_pack_v3()).expect("the arena pack is json");
    let def = &mut doc["entries"][PROVING_GROUND]["def"];
    let tanks = def["tanks"]
        .as_array_mut()
        .expect("the arena authors tanks");
    let before = tanks.len();
    tanks.retain(|tank| !Z3_TANKS.iter().any(|id| tank["id"] == *id));
    assert_eq!(
        tanks.len() + Z3_TANKS.len(),
        before,
        "version 3 authors the four"
    );
    assert!(
        def["note"]
            .as_str()
            .is_some_and(|note| note.contains("VERSION 3 (16b cut Z.3)")),
        "version 3's note carries its own sentence"
    );
    def["note"] = serde_json::Value::from(ARENA_V2_NOTE);
    doc["manifest"]["version"] = serde_json::Value::from(2);
    doc["manifest"]["digest"] = serde_json::Value::from("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = mush_content::parse_pack(&unsigned)
        .expect("version 2 parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    assert_eq!(
        digest, ARENA_V2_DIGEST,
        "the rebuilt fixture is not version 2: version 3 changed more than the four tanks and \
         the note"
    );
    doc["manifest"]["digest"] = serde_json::Value::from(digest);
    serde_json::to_string(&doc).expect("json")
}

/// **The production roster with a hand-built content module** (16b cut Z.3):
/// every module `mush_server::boot` composes — the atlas among them, so the
/// `@atlas/pack` shelf is production's own, with the game's two packs on it —
/// but the content module composed here, with the production appliers (the
/// atlas's, the NPC host's and this crate's) and `boot` as its boot packs.
///
/// [`tacsim::composition::content_module_with`] compiles the shipped packs in,
/// so a world that booted an **older** arena cannot be made through it; this is
/// the one seam that differs, and it differs only in which bytes boot. The
/// character sheet is left out, as `tests/match_lifecycle.rs` leaves it out.
#[must_use]
pub fn roster_booting(
    cfg: tacsim::TacsimConfig,
    boot: Vec<mush_content::Pack>,
) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = std::sync::Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    // Doc 18 cut IC5: `tacsim` the third quantity delegate, the inventory cell
    // handed to the layer below.
    let (mut mods, inventory) = comp::server_modules_with_tacsim(
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
    mods.push(comp::tacsim_module_wired(cfg, false, inventory));
    let mut appliers = mush_content::Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    tacsim::register_appliers(&mut appliers).expect("tacsim appliers");
    mods.push(Box::new(
        mush_content::ContentModule::new()
            .with_appliers(appliers)
            .with_boot_packs(boot),
    ));
    mods
}

/// The boot packs of a world that **took the OOC pack's version 7 and not the
/// arena's version 3** (16b cut Z.3): the shipped starter area (twelve pods),
/// the arena's ground, and [`arena_pack_v2`] (eight tanks). `starter_area`
/// false leaves the lot out, for a rig that needs only the arena.
#[must_use]
pub fn boot_packs_at_arena_v2(starter_area: bool) -> Vec<mush_content::Pack> {
    let mut packs = if starter_area {
        mush_atlas::starter::packs().expect("the starter packs load")
    } else {
        Vec::new()
    };
    let mut game = tacsim::definitions::content::boot_packs().expect("the game packs load");
    let arena = game.pop().expect("the arena pack boots last");
    assert_eq!(
        arena.manifest().id().as_str(),
        tacsim::definitions::content::PROVING_GROUND_PACK
    );
    packs.extend(game);
    packs.push(mush_content::load_pack(&arena_pack_v2()).expect("version 2 verifies"));
    packs
}

/// The digest `neighborhood.ooc_area@6` shipped with (16b cut D.5), as
/// `mush-atlas`'s `tests/starter.rs` pinned it until cut Z.3.
pub const OOC_V6_DIGEST: &str = "64f6cf88104b80c4bf9af2b013ec2cba089cf2e664f827a72f29184a9cc54691";

/// The digest `neighborhood.staff_lobby@4` shipped with (16b cut D.5).
pub const STAFF_LOBBY_V4_DIGEST: &str =
    "8d791c31a7c81c794cb78529b2735e0c1ab360aea92aa8d03c3d571388056796";

/// The four pods the OOC pack's version 7 added (16b cut Z.3).
pub const Z3_PODS: [&str; 4] = [
    "item.red_pod_5",
    "item.red_pod_6",
    "item.blue_pod_5",
    "item.blue_pod_6",
];

/// Re-sign `doc` and hold it to `pinned`: the fixture is that version in
/// everything the digest covers, or it is not built at all.
fn signed_as(mut doc: serde_json::Value, pinned: &str, what: &str) -> String {
    doc["manifest"]["digest"] = serde_json::Value::from("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = mush_content::parse_pack(&unsigned)
        .expect("the fixture parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    assert_eq!(
        digest, pinned,
        "the rebuilt fixture is not {what}: a later version changed more than the fixture undoes"
    );
    doc["manifest"]["digest"] = serde_json::Value::from(digest);
    serde_json::to_string(&doc).expect("json")
}

/// **Version 6 of the OOC pack** (16b cut Z.3): the shipped version 7 with its
/// four pods taken back out and the three strings it rewrote put back, held to
/// version 6's own pinned digest — `mush-atlas`'s `tests/interior.rs` builds
/// the same fixture, and an integration test cannot import another one.
#[must_use]
pub fn ooc_pack_v6() -> String {
    let mut text = mush_atlas::starter::OOC_AREA_JSON.to_owned();
    for (shipped, old) in [
        (
            "a red and a blue room of six pods each",
            "a red and a blue room of four pods each",
        ),
        (
            "Six pods in a row under a red stripe",
            "Four pods in a row under a red stripe",
        ),
        (
            "Six pods under a blue stripe",
            "Four pods under a blue stripe",
        ),
    ] {
        assert!(text.contains(shipped), "{shipped:?}: the fixture is stale");
        text = text.replace(shipped, old);
    }
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("the pack is json");
    let entries = doc["entries"].as_object_mut().expect("entries");
    for id in Z3_PODS {
        assert!(entries.remove(id).is_some(), "{id} is in the shipped pack");
    }
    doc["manifest"]["version"] = serde_json::Value::from(6);
    signed_as(doc, OOC_V6_DIGEST, "the OOC pack's version 6")
}

/// **Version 4 of the staff lobby** (16b cut Z.3): the shipped version 5 at
/// version 4 requiring `ooc_area@6`, which is all version 5 changed, held to
/// version 4's pinned digest. A world at `ooc_area@6` cannot boot the shipped
/// staff lobby, whose `requires` names version 7.
#[must_use]
pub fn staff_lobby_pack_v4() -> String {
    let mut doc: serde_json::Value =
        serde_json::from_str(mush_atlas::starter::STAFF_LOBBY_JSON).expect("the pack is json");
    doc["manifest"]["version"] = serde_json::Value::from(4);
    for required in doc["manifest"]["requires"]
        .as_array_mut()
        .expect("the staff lobby requires packs")
    {
        if required["id"] == mush_atlas::starter::OOC_AREA_PACK {
            required["version"] = serde_json::Value::from(6);
        }
    }
    signed_as(doc, STAFF_LOBBY_V4_DIGEST, "the staff lobby's version 4")
}

/// The boot packs of a world that **took neither Z.3 pack's new OOC version**
/// (16b cut Z.3): the region, [`ooc_pack_v6`] (eight pods),
/// [`staff_lobby_pack_v4`], the arena's ground (the shipped atlas pack, version
/// 2 since 16b cut AE.1), and the arena as shipped — version 5 since 16b cut
/// AE.1 (twelve tanks, the Hold declaration, The Bluff; 16b cut O.1 moved it
/// from version 3 to 4) — or, with
/// `arena_v2`, at [`arena_pack_v2`] (eight tanks) —
/// the second being dev1's world before the packs are taken, the first a
/// staffer who took the arena pack and not the OOC pack.
#[must_use]
pub fn boot_packs_at_ooc_v6(arena_v2: bool) -> Vec<mush_content::Pack> {
    let mut packs = vec![
        mush_content::load_pack(mush_atlas::starter::REGION_JSON).expect("the region loads"),
        mush_content::load_pack(&ooc_pack_v6()).expect("version 6 verifies"),
        mush_content::load_pack(&staff_lobby_pack_v4()).expect("version 4 verifies"),
    ];
    let mut game = tacsim::definitions::content::boot_packs().expect("the game packs load");
    if arena_v2 {
        let arena = game.pop().expect("the arena pack boots last");
        assert_eq!(
            arena.manifest().id().as_str(),
            tacsim::definitions::content::PROVING_GROUND_PACK
        );
        game.push(mush_content::load_pack(&arena_pack_v2()).expect("version 2 verifies"));
    }
    packs.extend(game);
    packs
}

/// Plan and apply one shelved pack as `actor`, through the `@atlas/pack`
/// verbs: `(what plan said, what apply said)`. Panics when the plan prints no
/// fingerprint, with what it did print.
pub fn take_from_shelf(tw: &mut TestWorld, actor: ObjId, pack: &str) -> (String, String) {
    let events = tw.run(actor, &format!("@atlas/pack plan {pack}"));
    let planned = tw.seen_by(&events, actor).join("\n");
    let fingerprint = planned
        .lines()
        .find_map(|line| line.trim().strip_prefix("fingerprint "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no fingerprint in:\n{planned}"))
        .to_string();
    let events = tw.run(actor, &format!("@atlas/pack apply {fingerprint}"));
    let applied = tw.seen_by(&events, actor).join("\n");
    (planned, applied)
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
    shipped_arena_seeded(world, sides, SEED)
}

/// **[`shipped_arena`] on another match seed** (17b cut M1a.6): the twin lane
/// (`tests/twin.rs`) plays fifteen seeds, and the seed is the one input of
/// the check-out the rig otherwise pins to [`SEED`].
///
/// **17b cut M3.5** (doc 17 §13): a [`Muster::scripted`] hull is sealed
/// `scripted`, and once [`Arena::new`] has opened the field the production
/// installer (`tacsim::minds::install_sealed_runners`) gives it its runner
/// from [`sealed_sources`] — the rig's sources, Standard Doctrine's text for
/// its digest ([`fixture_source`]).
#[must_use]
pub fn shipped_arena_seeded(world: &TestWorld, sides: &[&[Muster]], seed: u64) -> Arena {
    let custody = shipped_custody_seeded(world, sides, seed);
    let sources = sealed_sources(&custody.units);
    let mut arena = Arena::new(
        custody,
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the shipped-scale arena checks out");
    install_sealed(&mut arena, &sources);
    arena
}

/// **The source a fixture's seal may name, by its SHA-256** (17b cut M3.5):
/// Standard Doctrine's text for its digest (`STANDARD_SHA256`), `None` for
/// any other.
#[must_use]
pub fn fixture_source(source_sha256: &[u8; 32]) -> Option<String> {
    (*source_sha256 == tacsim::minds::standard_digest().0)
        .then(|| tacsim::minds::STANDARD_RHAI.to_owned())
}

/// **The sources the scripted seals of `units` name, keyed by each seal's
/// digest** (17b cut M3.5) — the map production's `open_match` builds off the
/// store, built here off [`fixture_source`]: what
/// `tacsim::minds::install_sealed_runners` is asked for by `UnitState::seal`'s
/// digest. A digest the decode carried resolves through the same map, so a
/// fixture that resumes from a snapshot rebuilds it from the custody it
/// opened on.
#[must_use]
pub fn sealed_sources(units: &[CustodyUnit]) -> BTreeMap<[u8; 32], String> {
    units
        .iter()
        .filter(|unit| unit.seal.engine == tacsim::arena::SealEngine::Scripted)
        .filter_map(|unit| {
            fixture_source(&unit.seal.source_sha256).map(|source| (unit.seal.digest(), source))
        })
        .collect()
}

/// **The production installer over `sources`** (17b cut M3.5;
/// `tacsim::minds::install_sealed_runners`) on the arena's own doctrine —
/// every scripted seal's runner installed on `Arena::runners`. Answers how
/// many.
pub fn install_sealed(arena: &mut Arena, sources: &BTreeMap<[u8; 32], String>) -> usize {
    let cfg = arena.arena_cfg.clone();
    tacsim::minds::install_sealed_runners(arena, &cfg, &|digest| sources.get(digest).cloned())
        .unwrap_or_else(|error| panic!("the fixture's sealed runners install: {error}"))
}

/// **The custody [`shipped_arena_seeded`] opens on** (17b cut M3.5 split it
/// out): every muster assembled, posed and sealed exactly as
/// [`tacsim::match_::start`] does, under the fixture terms — `scripted` for a
/// [`Muster::scripted`] hull.
#[must_use]
pub fn shipped_custody_seeded(world: &TestWorld, sides: &[&[Muster]], seed: u64) -> CustodyOut {
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
            // 16b cut AE.1: the rig fields the stock tanks, whose row is tracked.
            let pose = tacsim::match_::opening_pose(
                &definition,
                muster.team,
                world_pose,
                &mut taken,
                tacsim::arena::CostRow::Tracked,
            )
            .unwrap_or_else(|| panic!("{:?} has an entry cell", muster.team));
            // **17b cut M3.5**: a scripted muster's mind is sealed `scripted`.
            // **17b cut M4.3**: a production muster under the production
            // fill's terms; every other under the fixture's.
            let terms = if muster.scripted {
                tacsim::match_::SealTerms {
                    engine_for_minds: tacsim::arena::SealEngine::Scripted,
                    ..tacsim::match_::SealTerms::fixture()
                }
            } else if muster.production {
                tacsim::match_::SealTerms::production_fixture()
            } else {
                tacsim::match_::SealTerms::fixture()
            };
            let mut unit = tacsim::match_::custody_unit(
                &catalog,
                cells,
                id,
                muster.team,
                pose,
                0,
                muster.controller,
                muster.slot,
                &terms,
            )
            .expect("the fixture terms seal");
            // **17b cut M2.5**: an unarmed muster fields no weapon rows —
            // and (**17b cut M3.3**) is sealed again as it now stands, so its
            // seal grants no fire channel (the seal is a function of the
            // finished custody; doc 17 §13).
            if muster.unarmed {
                unit.weapons.clear();
                unit.reseal(&terms).expect("the fixture terms seal");
                assert_eq!(
                    unit.seal.channels,
                    [
                        tacsim::arena::state::Channel::Helm,
                        tacsim::arena::state::Channel::Sensors
                    ],
                    "an unarmed muster's seal grants no fire channel"
                );
            }
            units.push(unit);
        }
    }
    CustodyOut {
        definition,
        units,
        pilots: Vec::new(),
        resolution: tacsim::match_::resolution_for(&catalog),
        seed,
        mode: tacsim::arena::MatchMode::Skirmish,
    }
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
        // 17b cut M4.1: settled between pulses, so `each_pulse` reads a mind
        // row with its answer written, as the task's own pulse would leave it.
        let out = arena.step_settled(wall, &[], &[]);
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
/// one unit's: [`tacsim::arena::motor::MotorCounters`] is per pulse and per
/// field, and there is no per-unit failure count in the tree to read.
///
/// **17b cut M4.5** reads the machines off the minds: every hull a machine
/// flies (`Controller::is_machine`; a pilot's hull, and the dormant row under
/// it, print nothing), in `arena.units`' id order as the reflex table was
/// walked, its state the doctrine word [`Arena::machine_state`] reads and its
/// target the object its mind row's [`StockMemory`] settled on.
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
        for unit in arena
            .units
            .iter()
            .filter(|unit| unit.controller.is_machine())
        {
            let id = unit.id;
            let cell = cell_of(arena, id);
            // 17b cut M4.5: the target the mind settled on, by the object its
            // own contact row names.
            let target = arena
                .minds
                .get(&id)
                .and_then(|row| StockMemory::from(&decode_memory(&row.memory)).target)
                .and_then(|contact| arena.contacts.resolve(id, contact))
                .map(|held| held.target.0);
            self.lines.push(format!(
                "  {:>7} ms #{:<4} {:<9} at {:?} goal {:?} target {:?} health {} ‰ (field \
                 path_failures {})",
                arena.now_ms,
                id.0,
                arena.machine_state(id).map_or("-", |state| state.as_str()),
                cell,
                // 17b cut M1a.1: the goal is the hull's motor row's.
                arena.motors.row(id).and_then(|motor| motor.goal),
                target,
                arena.health_permille(id),
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

/// Run `f` with a `Ctx` on `tw`'s world as God, under this crate's module name —
/// the shape `tests/reset.rs`'s rig uses for a staffer's direct write.
pub fn with_ctx<T>(tw: &mut TestWorld, f: impl FnOnce(&mut mush_core::Ctx<'_>) -> T) -> T {
    let mut out = mush_core::Outbox::new();
    let engine = tw.engine_mut();
    let mut ctx = mush_core::Ctx {
        w: &mut engine.world,
        actor: mush_core::GOD,
        switches: mush_core::SwitchSet::default(),
        args: mush_core::ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(SmolStr::new(tacsim::NS)),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    f(&mut ctx)
}

/// The cell centre, pose-wise, that the proving ground authors for one slot id
/// — the applier's own arithmetic (`cell × cell_cm + cell_cm / 2`).
#[must_use]
pub fn authored_pose_of(slot: &str) -> (i64, i64, u32) {
    let catalog = tacsim::definitions::Catalog::build_game(&[], tacsim::NS)
        .expect("the shipped content binds");
    let arena = catalog
        .arenas
        .values()
        .next()
        .expect("the proving ground is shipped");
    let tank = arena
        .tanks
        .iter()
        .find(|tank| tank.id.as_str() == slot)
        .unwrap_or_else(|| panic!("`{slot}` is authored"));
    let cell = i64::from(arena.cell_cm);
    (
        i64::from(tank.x) * cell + cell / 2,
        i64::from(tank.y) * cell + cell / 2,
        tank.heading_mdeg,
    )
}

/// **The 95 per cent Wilson score interval** (17b cut M1a.6; doc 17 §17's
/// Gate B): `(low, high)` for `successes` out of `n`, `z = 1.96`, clamped to
/// `[0, 1]`; `(0, 1)` for `n = 0`, which says nothing. Named for 17b cut M6's
/// library lift; the twin lane's Gate B called it until 17b cut M4.5 retired
/// the T0 column, and `tests/twin.rs` holds it to four hand-worked values.
#[must_use]
pub fn wilson_95(successes: u32, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let z = 1.96_f64;
    let n = f64::from(n);
    let p = f64::from(successes) / n;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denominator;
    let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
    ((centre - half).max(0.0), (centre + half).min(1.0))
}

// ---------------------------------------------------------------------------
// 17b cut M2.5 — the twin runner (Gate A), and one match played to its end
// ---------------------------------------------------------------------------

/// **How a [`TwinRunner`] answers a fault or a divergence** (17b cut M2.5;
/// 17b §1 (iii)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TwinMode {
    /// Gate A asserted: the first Rhai fault, then the first intent list,
    /// then the first memory that is not T1's panics with the printout.
    Assert,
    /// Gate A counted: every think recorded ([`TwinThought`]) with the first
    /// printout of each hull kept ([`TwinRunner::printout`]) and the first of
    /// each hull's thinks that faulted or parted outside the fire half
    /// ([`TwinRunner::failure`]); never panics.
    Count,
}

/// **One think, as the twin compared it** (17b cut M2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TwinThought {
    /// The hull.
    pub unit: ObjId,
    /// The think's sequence number.
    pub think_seq: u64,
    /// The think's lattice millisecond.
    pub t_ms: i64,
    /// The Rhai think's operations (`mush_mind::budget`, the runner's
    /// recorded `ThinkStat`).
    pub ops: u64,
    /// Its fees, the per-think fee among them: `ops + fee` is what the class
    /// ceiling bounds (D-UM-26).
    pub fee: u64,
    /// The Rhai think's wall time, microseconds.
    pub rhai_us: u128,
    /// The native think's wall time, microseconds.
    pub native_us: u128,
    /// The Rhai think faulted (its cause word in the printout).
    pub faulted: bool,
    /// The Rhai think did not fault and answered other intents or other
    /// memory bytes than T1.
    pub diverged: bool,
    /// The view carried weapon rows (an armed hull, whose fire half 17b cut
    /// M2.6 ported: every lane asserts its thinks whole).
    pub armed: bool,
    /// The divergence is the fire half's alone: the memory bytes equal, and
    /// the intents equal once every `Condition` and `Fire` is filtered out (a
    /// counting twin's classification; since 17b cut M2.6 no lane allows it).
    pub fire_only: bool,
    /// T1's state code after the think (`MachineState::code`).
    pub state: u8,
    /// T1's `rallied` after the think.
    pub rallied: bool,
    /// T1's `stand_until_ms` is set after the think: a withdrawing hull's
    /// dwell (16b cut Q.7), its clock compared byte for byte.
    pub dwelling: bool,
    /// **17b cut M2.6**: the view carried a standing order (`view.order`
    /// present) — an ordered think, the basis of every order arm Gate A
    /// compares.
    pub ordered: bool,
    /// **17b cut M3.6** (plan §2(v); the review's R11): the job was a
    /// DROPPED one (`ThinkJob::dropped`) — the hull is on the ROM, so the Rhai
    /// runner answered with its native stand-in and recorded no `ThinkStat`,
    /// and Gate A compared native with native on this think. A turret kill on
    /// a shipped line tank (`destroy_core`) drops its mind for the rest of the
    /// match; the lanes count these thinks so the coverage shrink is named.
    pub dropped: bool,
}

/// **Gate A's runner** (17b cut M2.5; doc 17 §17 "Gate A: T1 ≡ T2 exactly";
/// 17b §1 (iii)): T1 (`NativeStockMind`) and T2 (the Rhai Standard Doctrine,
/// `tacsim::minds::STANDARD_RHAI` on a recording `RhaiRunner`) thinking on
/// the SAME job — both pure functions of it — compared, and **the native
/// answer returned**: a match flown by the twin flies on T1 and hashes as T1.
///
/// In [`TwinMode::Assert`] it panics on the first difference, **the Rhai
/// fault FIRST** (17b's adversarial F4: a Rhai fault answers no intents and
/// the memory it was handed, which on a quiet think of T1's is the same
/// answer, so a fault compared as intents and bytes would pass), then the
/// intents, then the memory bytes — each with the printout: the hull, the
/// think, the clocks, the ops and fees, the memory in (decoded and raw),
/// both intent lists, both memories (decoded and raw), the Rhai fault's word
/// and trace, and the view.
#[derive(Debug)]
pub struct TwinRunner {
    native: NativeStockMind,
    rhai: RhaiRunner,
    mode: TwinMode,
    /// Every think compared, in think order.
    pub seen: Mutex<Vec<TwinThought>>,
    /// The first printout of each hull that faulted or diverged (`Count`).
    first: Mutex<BTreeMap<u32, String>>,
    /// The first printout of each hull that faulted or diverged OUTSIDE the
    /// fire half (`Count`): the think a lane's assertion fails on, never an
    /// earlier fire-only one.
    failing: Mutex<BTreeMap<u32, String>>,
}

impl TwinRunner {
    /// **The asserting twin** on the match's doctrine (`cfg.reflex`, which
    /// both runners read).
    #[must_use]
    pub fn new(cfg: &ArenaConfig) -> TwinRunner {
        TwinRunner::with_mode(cfg, TwinMode::Assert)
    }

    /// **The counting twin** — every think recorded, nothing asserted.
    #[must_use]
    pub fn counting(cfg: &ArenaConfig) -> TwinRunner {
        TwinRunner::with_mode(cfg, TwinMode::Count)
    }

    fn with_mode(cfg: &ArenaConfig, mode: TwinMode) -> TwinRunner {
        let rhai = RhaiRunner::new(
            cfg,
            tacsim::minds::STANDARD_RHAI,
            tacsim::minds::standard_digest(),
        )
        .unwrap_or_else(|error| panic!("Standard Doctrine compiles: {error}"))
        .recording();
        TwinRunner {
            native: NativeStockMind::with(&cfg.reflex),
            rhai,
            mode,
            seen: Mutex::new(Vec::new()),
            first: Mutex::new(BTreeMap::new()),
            failing: Mutex::new(BTreeMap::new()),
        }
    }

    /// **`(faults, divergences)`** over every think compared.
    #[must_use]
    pub fn divergences(&self) -> (usize, usize) {
        let seen = self.thoughts();
        (
            seen.iter().filter(|thought| thought.faulted).count(),
            seen.iter().filter(|thought| thought.diverged).count(),
        )
    }

    /// Every think compared so far, in think order.
    #[must_use]
    pub fn thoughts(&self) -> Vec<TwinThought> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }

    /// The first printout `unit` faulted or diverged with (`Count`), if any.
    #[must_use]
    pub fn printout(&self, unit: ObjId) -> Option<String> {
        self.first
            .lock()
            .ok()
            .and_then(|first| first.get(&unit.0).cloned())
    }

    /// **The printout of `unit`'s first think that faulted or parted from T1
    /// outside the fire half** (`Count`), if any — what a lane prints when it
    /// fails on such a think: a lane walking the thoughts in think order
    /// fails on the hull's first, which is this one (an earlier fire-only
    /// divergence of the same hull is [`TwinRunner::printout`]'s, not this).
    #[must_use]
    pub fn failure(&self, unit: ObjId) -> Option<String> {
        self.failing
            .lock()
            .ok()
            .and_then(|failing| failing.get(&unit.0).cloned())
    }
}

/// The intents with the fire half's kinds (`Condition`, `Fire`) taken out.
#[must_use]
pub fn helm_half(intents: &[IntentKind]) -> Vec<&IntentKind> {
    intents
        .iter()
        .filter(|kind| !matches!(kind, IntentKind::Condition(_) | IntentKind::Fire { .. }))
        .collect()
}

/// A memory's bytes, decoded and raw, for a printout.
fn memory_shown(bytes: &[u8]) -> String {
    format!(
        "{:?}\n     {}",
        StockMemory::from(&decode_memory(bytes)),
        String::from_utf8_lossy(bytes)
    )
}

/// **The twin's printout** of one think (the [`TwinRunner`] head).
fn twin_printout(
    job: &ThinkJob<'_>,
    native: &ThinkResult,
    rhai: &ThinkResult,
    stat: Option<ThinkStat>,
) -> String {
    format!(
        "Gate A: unit #{} think {} t_ms {} lands_ms {} (ops {} fee {})\n\
         -- the memory in: {}\n\
         -- T1 intents: {:?}\n\
         -- T2 intents: {:?}\n\
         -- T1 memory: {}\n\
         -- T2 memory: {}\n\
         -- T2 fault: {:?}\n\
         -- T2 trace: {:#?}\n\
         -- the view: {:#?}",
        job.unit.0,
        job.think_seq,
        job.t_ms,
        job.lands_ms,
        stat.map_or(0, |stat| stat.ops),
        stat.map_or(0, |stat| stat.fee),
        memory_shown(job.memory),
        native.intents,
        rhai.intents,
        memory_shown(&native.memory),
        memory_shown(&rhai.memory),
        rhai.fault,
        rhai.trace,
        job.view,
    )
}

impl MindRunner for TwinRunner {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let started = Instant::now();
        let native = self.native.think(job);
        let native_us = started.elapsed().as_micros();
        let rhai = self.rhai.think(job);
        let stat = self.rhai.take_stats().pop();
        let faulted = rhai.fault.is_some();
        let diverged = !faulted && (rhai.intents != native.intents || rhai.memory != native.memory);
        let fire_only = diverged
            && rhai.memory == native.memory
            && helm_half(&rhai.intents) == helm_half(&native.intents);
        if self.mode == TwinMode::Assert {
            // The fault FIRST (the head): a faulted Rhai think answers no
            // intents and the memory it was handed.
            assert!(
                rhai.fault.is_none(),
                "T2 faulted `{}`\n{}",
                rhai.fault.as_ref().map_or("", |fault| fault.cause.as_str()),
                twin_printout(job, &native, &rhai, stat)
            );
            assert_eq!(
                rhai.intents,
                native.intents,
                "the intents differ\n{}",
                twin_printout(job, &native, &rhai, stat)
            );
            assert_eq!(
                rhai.memory,
                native.memory,
                "the memory bytes differ\n{}",
                twin_printout(job, &native, &rhai, stat)
            );
        } else if faulted || diverged {
            // Formatted once, and only for a hull whose printout is not kept
            // yet (the view's `{:#?}` is long).
            let unkept = |kept: &Mutex<BTreeMap<u32, String>>| {
                kept.lock()
                    .is_ok_and(|kept| !kept.contains_key(&job.unit.0))
            };
            let want_first = unkept(&self.first);
            let want_failing = (faulted || !fire_only) && unkept(&self.failing);
            if want_first || want_failing {
                let printout = twin_printout(job, &native, &rhai, stat);
                if want_failing && let Ok(mut failing) = self.failing.lock() {
                    failing
                        .entry(job.unit.0)
                        .or_insert_with(|| printout.clone());
                }
                if want_first && let Ok(mut first) = self.first.lock() {
                    first.entry(job.unit.0).or_insert(printout);
                }
            }
        }
        let after = StockMemory::from(&decode_memory(&native.memory));
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(TwinThought {
                unit: job.unit,
                think_seq: job.think_seq,
                t_ms: job.t_ms,
                ops: stat.map_or(0, |stat| stat.ops),
                fee: stat.map_or(0, |stat| stat.fee),
                rhai_us: stat.map_or(0, |stat| stat.rhai_us),
                native_us,
                faulted,
                diverged,
                armed: !job.view.weapons.is_empty(),
                fire_only,
                state: after.state.code(),
                rallied: after.rallied,
                dwelling: after.stand_until_ms.is_some(),
                ordered: job.view.order.is_some(),
                dropped: job.dropped,
            });
        }
        native
    }
}

/// **`(p50, p99, max)`** of `values` (`0`s for none).
#[must_use]
pub fn spread(values: &[u64]) -> (u64, u64, u64) {
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

/// **What one [`play`] came to** (17b cut M2.5).
#[derive(Clone, Debug)]
pub struct Played {
    /// Every ledger line, flushed.
    pub events: Vec<LedgerEvent>,
    /// **Every substep's hash, ACCUMULATED after every `Arena::step`**
    /// (`substep_hashes` is cleared at the head of each step, so a read at
    /// the end holds the last pulse only — 17b §5's feas F4); its length is
    /// asserted equal to the substeps played.
    pub hashes: Vec<u64>,
    /// How the match ended, or `None` at the cap.
    pub ended: Option<MatchEnd>,
    /// The arena millisecond it stopped at.
    pub t_ms: i64,
    /// The final `state_hash`.
    pub state_hash: u64,
    /// The ledger's SHA-256 over every byte appended.
    pub ledger_digest: [u8; 32],
}

/// **One match on the proving ground** (17b cut M2.5, lifted from
/// `tests/twin.rs`' lane with a runner argument): `red` and `blue` fielded by
/// [`shipped_arena_seeded`] at `seed`, every mind hull thinking on `runner`,
/// stepped **one substep a pulse** (the cross-tempo invariant makes these
/// bytes those of any cadence) to the end — or to `cap_s` arena seconds.
pub fn play(
    world: &TestWorld,
    seed: u64,
    red: &[Muster],
    blue: &[Muster],
    runner: Arc<dyn MindRunner>,
    cap_s: Option<u32>,
) -> Played {
    let mut arena = shipped_arena_seeded(world, &[red, blue], seed);
    arena.mind_runner = runner;
    let cap_ms = cap_s.map(|cap| i64::from(cap) * 1_000);
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut substeps = 0_usize;
    let mut wall = arena.now_ms;
    let ended = loop {
        // 17b cut M4.1: settled between pulses, as the task's pool settles —
        // the final `state_hash` below reads every answer written.
        let out = arena.step_settled(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        substeps += usize::try_from(out.substeps).unwrap_or(0);
        assert_eq!(
            hashes.len(),
            substeps,
            "one hash a substep played, accumulated over every step"
        );
        if out.ended.is_some() {
            break out.ended;
        }
        if cap_ms.is_some_and(|cap| arena.now_ms >= cap) {
            break None;
        }
        wall = arena.now_ms + arena.substep_len_ms();
    };
    let events = ledger_events(&mut arena, &mut bytes);
    Played {
        events,
        hashes,
        ended,
        t_ms: arena.now_ms,
        state_hash: tacsim::arena::hash::state_hash(&arena),
        ledger_digest: arena.ledger.digest(),
    }
}

// ---------------------------------------------------------------------------
// 17b cut M2.6 — the fixtures under the twin, and the authored lance field
// ---------------------------------------------------------------------------

/// **Fly `arena`'s minds on an asserting [`TwinRunner`]** (17b cut M2.6):
/// `arena.mind_runner` becomes the twin on the arena's own doctrine
/// (`arena.arena_cfg`), which is handed back so a fixture can count what it
/// compared. Applied BEFORE any recorder (`tests/minds.rs`' `Stock`,
/// `tests/conditions.rs`' `Kept`) reads the arena's runner, so the recorder
/// wraps the twin. The twin answers T1's answer: every assertion a fixture
/// makes of T1 holds of a twin-flown run unchanged, and every think is
/// Gate A — the Rhai fault first, then the intents, then the memory bytes.
pub fn with_twin(arena: &mut Arena) -> Arc<TwinRunner> {
    let twin = Arc::new(TwinRunner::new(&arena.arena_cfg));
    arena.mind_runner = twin.clone();
    twin
}

/// **What the running test asked its T1 fields to fly** (17b cuts M2.6 and
/// M2.7): the asserting twin ([`under_the_twin`], the twins kept for the
/// count) or the Rhai Standard Doctrine ALONE ([`under_the_rhai_runner`],
/// the fields counted).
enum Asked {
    Twin(Vec<Arc<TwinRunner>>),
    Rhai(usize),
}

thread_local! {
    /// What [`under_the_twin`] or [`under_the_rhai_runner`] asked for on this
    /// thread, `None` outside both (each test runs on its own thread).
    static ASKED: std::cell::RefCell<Option<Asked>> =
        const { std::cell::RefCell::new(None) };
    /// The runs and thinks a fixture body noted ([`note_run`],
    /// [`note_think`]) while [`under_the_rhai_runner`] listens, `None`
    /// otherwise.
    static NOTES: std::cell::RefCell<Option<Notes>> =
        const { std::cell::RefCell::new(None) };
}

/// **Run `test` with its T1 fields flown by the twin** (17b cut M2.6): inside
/// it a fixture's field builder installs an asserting twin on the field
/// ([`twin_if_asked`], through [`with_twin`]) before any recorder reads the
/// arena's runner, so the recorder (`tests/minds.rs`' `Stock::on`) wraps that
/// twin — the SAME fixture body, its assertions on T1 untouched, with every
/// think it flies compared T1 against T2. Answers how many thinks the twins
/// compared, asserted nonzero (a sibling that compared nothing proves
/// nothing) and printed with the fixture's name.
pub fn under_the_twin(name: &str, test: impl FnOnce()) -> usize {
    ASKED.with(|asked| *asked.borrow_mut() = Some(Asked::Twin(Vec::new())));
    test();
    let twins = match ASKED.with(|asked| asked.borrow_mut().take()) {
        Some(Asked::Twin(twins)) => twins,
        _ => Vec::new(),
    };
    let thinks: usize = twins.iter().map(|twin| twin.thoughts().len()).sum();
    for twin in &twins {
        assert_eq!(
            twin.divergences(),
            (0, 0),
            "{name}: an asserting twin counted"
        );
    }
    let budget: Vec<u64> = twins
        .iter()
        .flat_map(|twin| twin.thoughts())
        .map(|thought| thought.ops + thought.fee)
        .collect();
    let (p50, p99, max) = spread(&budget);
    println!(
        "m2_under_the_twin: fixture={name} twins={} thinks_compared={thinks} \
         ops_plus_fee_p50={p50} p99={p99} max={max}",
        twins.len()
    );
    assert!(thinks > 0, "{name}: the twin compared no think");
    thinks
}

/// **[`with_twin`] when the running test asked for it** ([`under_the_twin`]),
/// the twin kept for that count; **[`with_rhai`] when it asked for the Rhai
/// runner alone** ([`under_the_rhai_runner`], 17b cut M2.7), the field
/// counted; otherwise `arena` is left on its runner. A field builder calls it
/// last, so the field it answers flies the twin or the Rhai runner before any
/// recorder reads the arena's runner.
pub fn twin_if_asked(arena: &mut Arena) {
    ASKED.with(|asked| match asked.borrow_mut().as_mut() {
        Some(Asked::Twin(twins)) => twins.push(with_twin(arena)),
        Some(Asked::Rhai(fields)) => {
            with_rhai(arena);
            *fields += 1;
        }
        None => {}
    });
}

// ---------------------------------------------------------------------------
// 17b cut M2.7 — the fixtures under the Rhai runner ALONE
// ---------------------------------------------------------------------------

/// **The Rhai Standard Doctrine alone, on `cfg`'s doctrine** (17b cut M2.7;
/// doc 17 §17 "Gate A", §22's M2 gate): `tacsim::minds::standard_runner` —
/// the production entry point, `RhaiRunner::new(cfg, STANDARD_RHAI,
/// standard_digest())` — with its `Result` handled: a script that does not
/// compile fails the test HERE, by name. Unlike the twin, nothing native
/// answers beside it: a fixture flown on it flies T2's own answers, so a
/// fixture whose event list and hash sequence equal the native run's proves
/// T2 ≡ T1 on that fixture (the twin, which flies T1's answer, cannot).
#[must_use]
pub fn rhai_alone(cfg: &ArenaConfig) -> Arc<dyn MindRunner> {
    tacsim::minds::standard_runner(cfg).unwrap_or_else(|error| {
        panic!("with_rhai: the Standard Doctrine (standard.rhai) does not compile: {error}")
    })
}

/// **Fly `arena`'s minds on the Rhai Standard Doctrine alone** (17b cut
/// M2.7): `arena.mind_runner` becomes [`rhai_alone`] on the arena's own
/// doctrine (`arena.arena_cfg`), asserted to print as a `RhaiRunner`.
/// Applied BEFORE any recorder reads the arena's runner, as [`with_twin`]
/// is. **A decoded snapshot re-installs `default_mind_runner`** (the native
/// stock mind: `Snapshot::decode`), so a fixture that resumes from one calls
/// this again after the decode — the runner's `Debug` name is the only thing
/// that tells a resumed T2 field from a T1 one, since T2 ≡ T1. **Since 17b
/// cut M3.5** that is the rule for THIS lever alone: it swaps the arena-wide
/// fallback under hulls sealed `native`, which no seal records, so nothing
/// the decode carries can bring it back. A hull sealed `scripted`
/// ([`Muster::scripted`]) is given its runner by the production installer at
/// the check-out and again after a decode ([`install_sealed`]), with no lever.
pub fn with_rhai(arena: &mut Arena) -> Arc<dyn MindRunner> {
    let runner = rhai_alone(&arena.arena_cfg);
    arena.mind_runner = Arc::clone(&runner);
    let shown = format!("{:?}", arena.mind_runner);
    assert!(
        shown.starts_with("RhaiRunner"),
        "with_rhai installs the Rhai runner, not {shown:.40}"
    );
    runner
}

/// **Whether the running test asked for the Rhai runner alone**
/// ([`under_the_rhai_runner`]'s second pass, 17b cut M2.7): a recorder that
/// admits the Rhai Standard Doctrine as "the stock mind" admits it only here,
/// so a fixture's own scripted `RhaiRunner` is never recorded as it.
#[must_use]
pub fn rhai_asked() -> bool {
    ASKED.with(|asked| matches!(*asked.borrow(), Some(Asked::Rhai(_))))
}

/// One noted run: its event list `(t_ms, kind as JSON, line hash)` and its
/// substep hash sequence.
pub type NotedRun = (Vec<(i64, String, u64)>, Vec<u64>);

/// **What a fixture body noted while [`under_the_rhai_runner`] listened**
/// (17b cut M2.7): every run's event list `(t_ms, kind as JSON, line hash)`
/// and substep hash sequence ([`note_run`]), and every recorded stock think
/// `(unit, t_ms, answer)` ([`note_think`]), in the order the body made them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notes {
    /// Each run, `(events, hashes)`.
    pub runs: Vec<NotedRun>,
    /// Each think a stock recorder kept, `(unit, t_ms, answer)`.
    pub thinks: Vec<(u32, i64, ThinkResult)>,
}

/// **Note one run's event list and hash sequence** for
/// [`under_the_rhai_runner`] (a no-op outside it). A fixture's run helper
/// calls it once with what it answers.
pub fn note_run(events: &[(i64, String, u64)], hashes: &[u64]) {
    NOTES.with(|notes| {
        if let Some(notes) = notes.borrow_mut().as_mut() {
            notes.runs.push((events.to_vec(), hashes.to_vec()));
        }
    });
}

/// **Note one stock think's answer** for [`under_the_rhai_runner`] (a no-op
/// outside it). A fixture's stock recorder calls it on every think.
pub fn note_think(unit: ObjId, t_ms: i64, result: &ThinkResult) {
    NOTES.with(|notes| {
        if let Some(notes) = notes.borrow_mut().as_mut() {
            let mut behavior = result.clone();
            // Rhai's measured operation/fee diagnostics have no native
            // counterpart; every decision, rule sample and memory byte still
            // participates in the cross-runner comparison.
            behavior.diagnostics = None;
            notes.thinks.push((unit.0, t_ms, behavior));
        }
    });
}

/// **Run `test` twice — natively, then with its T1 fields flown by the Rhai
/// runner ALONE — and hold the second to the first** (17b cut M2.7; doc 17
/// §17 "Gate A", §22's "Gate A on the cross-tempo fixture"; 17b §1 (iii)).
/// The first pass flies every field as the fixture opens it (the native
/// stock mind, T1); the second asks the fields' builders ([`twin_if_asked`])
/// to install [`with_rhai`] before any recorder reads the arena's runner, so
/// the SAME body — its assertions on T1 untouched, its cadences as it plays
/// them — flies T2's own answers. Every run the body noted ([`note_run`]: the
/// event list and the hash sequence of each run at each cadence it plays)
/// and every stock think it recorded ([`note_think`]) must equal the native
/// pass's, in order, index by index; the Rhai pass must have opened at
/// least one field, the body must have noted at least one run or think, and
/// the Rhai pass's notes must be as many as the native pass's.
/// Prints `m2_under_the_rhai_runner: fixture=… fields=… runs=… lines=…
/// substeps=… thinks=…` and answers the fields flown.
pub fn under_the_rhai_runner(name: &str, test: impl Fn()) -> usize {
    let listen = |asked: Option<Asked>| {
        ASKED.with(|slot| *slot.borrow_mut() = asked);
        NOTES.with(|notes| *notes.borrow_mut() = Some(Notes::default()));
        test();
        let fields = match ASKED.with(|slot| slot.borrow_mut().take()) {
            Some(Asked::Rhai(fields)) => fields,
            _ => 0,
        };
        let notes = NOTES
            .with(|notes| notes.borrow_mut().take())
            .unwrap_or_default();
        (fields, notes)
    };
    let (_, native) = listen(None);
    let (fields, rhai) = listen(Some(Asked::Rhai(0)));
    assert!(fields > 0, "{name}: no field flew the Rhai runner");
    // A sibling that compared nothing proves nothing (as `under_the_twin`
    // asserts its thinks): the body must have noted a run or a think.
    assert!(
        !native.runs.is_empty() || !native.thinks.is_empty(),
        "{name}: the body noted no run and no think, so nothing was held to the native pass"
    );
    assert_eq!(
        rhai.runs.len(),
        native.runs.len(),
        "{name}: the runs noted under the Rhai runner"
    );
    for (at, (flown, recorded)) in rhai.runs.iter().zip(&native.runs).enumerate() {
        if let Some(line) = (0..flown.0.len().max(recorded.0.len()))
            .find(|line| flown.0.get(*line) != recorded.0.get(*line))
        {
            panic!(
                "{name}: run {at}'s event list parts from the native run's at line {line}:\n  \
                 Rhai:   {:?}\n  native: {:?}",
                flown.0.get(line),
                recorded.0.get(line)
            );
        }
        assert_eq!(
            flown.1, recorded.1,
            "{name}: run {at}'s hash sequence under the Rhai runner"
        );
    }
    assert_eq!(
        rhai.thinks.len(),
        native.thinks.len(),
        "{name}: the thinks noted under the Rhai runner"
    );
    if let Some(at) =
        (0..rhai.thinks.len()).find(|at| rhai.thinks.get(*at) != native.thinks.get(*at))
    {
        panic!(
            "{name}: think {at} parts from the native one:\n  Rhai:   {:?}\n  native: {:?}",
            rhai.thinks.get(at),
            native.thinks.get(at)
        );
    }
    println!(
        "m2_under_the_rhai_runner: fixture={name} fields={fields} runs={} lines={} substeps={} \
         thinks={}",
        rhai.runs.len(),
        rhai.runs.iter().map(|run| run.0.len()).sum::<usize>(),
        rhai.runs.iter().map(|run| run.1.len()).sum::<usize>(),
        rhai.thinks.len()
    );
    fields
}

/// **The lance lane's cohort** (`tests/harness.rs`' `COHORT`, copied: an
/// integration test cannot import another): hull `n` of the field is entry
/// `LANCE_COHORT[(n / 2) % 8]`, red for even `n` — three line tanks, then the
/// laser, the biped, the thermal, the ECM and the ECCM variants.
pub const LANCE_COHORT: [&str; 8] = [
    tacsim::definitions::content::LINE_TANK,
    tacsim::definitions::content::LINE_TANK,
    tacsim::definitions::content::LINE_TANK,
    tacsim::definitions::content::LINE_TANK_LASER,
    tacsim::definitions::content::BIPED_MK1,
    tacsim::definitions::content::LINE_TANK_THERMAL,
    tacsim::definitions::content::LINE_TANK_ECM,
    tacsim::definitions::content::LINE_TANK_ECCM,
];

/// The lance lane's hulls a side (`TACSIM_LANCE_UNITS` 24 over two).
pub const LANCE_PER_SIDE: u32 = 12;

/// Red's muster column, heading east (`tests/harness.rs`' `LANCE_RED_COLUMN`).
pub const LANCE_RED_COLUMN: u16 = 102;

/// Blue's muster column, heading west (`LANCE_BLUE_COLUMN`).
pub const LANCE_BLUE_COLUMN: u16 = 156;

/// The muster's first row and the axis of the alternation (`LANCE_ROW`).
pub const LANCE_ROW: i64 = 97;

/// The crossroads the script's `Advance` names (`CROSSROADS`).
pub const LANCE_CROSSROADS: (u16, u16) = (129, 97);

/// One script minute (`LANCE_MINUTE_MS`).
pub const LANCE_MINUTE_MS: i64 = 60_000;

/// Every scripted order's time to live and horizon (`LANCE_ORDER_MS`).
pub const LANCE_ORDER_MS: u32 = 90_000;

/// The engaged period the script's marks are rounded to (`WINDOW_MS`).
pub const LANCE_WINDOW_MS: i64 = 500;

/// **The authored lance field, flown by minds** (17b cut M2.6; 17b §1 (iii)'s
/// third lane; doc 17 §17's "authored-cohort runs"): `tests/harness.rs`'
/// `measure_lance` muster with every hull a mind. Twenty-four hulls of the
/// fielded catalog off `world` (the booted production roster's
/// `TacsimStore`, the catalog `match_::start` checks units out against),
/// built by the lance lane's own rule — hull `n` (`ObjId(100_000 + n)`) is
/// [`LANCE_COHORT`]`[(n / 2) % 8]`, red for even `n` — through
/// `definitions::unit::to_tactical_unit` and `match_::custody_unit` at the
/// authored reach; red on column 102 heading east and blue on 156 heading
/// west, rows 97, 95, 99, 93, 101, … (slot `n / 2`: the fan-out lane's
/// alternation); `Controller::Mind`, gunnery 1 as the lance lane's; on the
/// shipped proving ground at `seed`, `ArenaConfig { squad_size: 4 }` (the
/// default) so the arena deals each side into three lances of four. Not a
/// [`Muster`]: a `Muster` names a pod's slot, and the shipped content fields
/// six stock tanks a side, not twelve of the cohort.
#[must_use]
pub fn lance_arena(world: &TestWorld, seed: u64) -> Arena {
    let w = world.world();
    let definition = tacsim::match_::definition_for(w, &SmolStr::new_static(PROVING_GROUND))
        .expect("the proving ground's ground builds")
        .expect("the proving ground is shipped");
    let catalog = w
        .module_state::<tacsim::TacsimStore>()
        .map(|slot| slot.definitions().clone())
        .expect("the tacsim store is booted");
    let mut units: Vec<CustodyUnit> = Vec::new();
    for n in 0..2 * LANCE_PER_SIDE {
        let id = ObjId(100_000 + n);
        let team = if n % 2 == 0 { Team::Red } else { Team::Blue };
        let entry = LANCE_COHORT
            .get(usize::try_from(n / 2).unwrap_or(0) % LANCE_COHORT.len())
            .copied()
            .unwrap_or(tacsim::definitions::content::LINE_TANK);
        let key = mush_core::CanonicalId::new(entry).expect("a canonical entry id");
        let spec = catalog
            .specs
            .get(&key)
            .unwrap_or_else(|| panic!("the booted catalog fields {entry}"));
        let binding = catalog
            .bindings
            .get(&key)
            .unwrap_or_else(|| panic!("the booted catalog binds {entry}"));
        let cells = tacsim::definitions::unit::to_tactical_unit(spec, binding, id)
            .expect("the cohort entry builds a unit");
        let slot = i64::from(n / 2);
        let row = LANCE_ROW + if slot % 2 == 1 { -1 } else { 1 } * 2 * ((slot + 1) / 2);
        let (column, heading_mdeg) = if team == Team::Red {
            (LANCE_RED_COLUMN, 90_000)
        } else {
            (LANCE_BLUE_COLUMN, 270_000)
        };
        let pose = Pose {
            x_cm: i64::from(column) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
            y_cm: row * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
            z_cm: 0,
            heading_mdeg,
            posture: mush_sim::data::passive_rules_value(serde_json::json!({}))
                .expect("an empty posture"),
        };
        units.push(
            tacsim::match_::custody_unit(
                &catalog,
                cells,
                id,
                team,
                pose,
                1,
                Controller::Mind,
                0,
                &tacsim::match_::SealTerms::fixture(),
            )
            .expect("the fixture terms seal"),
        );
    }
    Arena::new(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution: tacsim::match_::resolution_for(&catalog),
            seed,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig {
            squad_size: 4,
            ..ArenaConfig::default()
        },
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the lance arena opens on the proving ground")
}

/// **When the lance lane's script starts** (`measure_lance`'s warm-up end):
/// the match's start-up, one decide period and one engaged period.
#[must_use]
pub fn lance_start_ms(arena: &Arena) -> i64 {
    i64::from(MatchConfig::default().startup_ms)
        .saturating_add(i64::from(arena.arena_cfg.reflex.decide_ms))
        .saturating_add(i64::from(arena.arena_cfg.tempo.engaged_ms))
}

/// **The lance lane's scripted order table** (`tests/harness.rs`'
/// `lance_schedule` and `LANCE_TABLE`, re-expressed: 17b cut M2.6), as
/// `(at_ms, squad, objective, horizon_ms)` in the order given.
///
/// At `start_ms` every squad of both sides is ordered `Engage`. Then in
/// minute `m` (1, 2, …, from `start_ms` rounded up to the engaged period)
/// each side's squads are ordered one at a time, ordinal `k` at `k − 1`
/// staggers into the minute (the stagger `60 s / squads` rounded down to the
/// period): the squad whose ordinal is `(m − 1) % squads_per_side + 1` takes
/// the table's `(m − 1) % 5`-th arm — `Hold`, `Advance` on the crossroads,
/// `Withdraw`, `Screen` toward the enemy's column (90° red, 270° blue),
/// `Support` of its own commander — and every other squad `Engage` again.
/// Every order's horizon (and time to live) is [`LANCE_ORDER_MS`]. Only marks
/// before `until_ms` are listed.
///
/// **The release is not a row of this shape** (no objective, no horizon):
/// `lance_schedule`'s one `ReleaseOrder` a side is [`lance_releases`]. What
/// the lane's own 120 seconds reach of an order's END is the deadline (a
/// lance whose opening order runs out before its first-minute order: the
/// third, at the opening's `start_ms` plus [`LANCE_ORDER_MS`]) and the
/// release; a mind's own early ends (an `Advance` a hull cannot carry out, a
/// `Support` whose charge reads dead) need the table's later arms, which
/// those seconds never reach.
#[must_use]
pub fn lance_orders(
    squads: &[tacsim::deliberation::SquadRoster],
    start_ms: i64,
    until_ms: i64,
) -> Vec<(
    i64,
    tacsim::deliberation::SquadId,
    tacsim::deliberation::Objective,
    u32,
)> {
    use tacsim::deliberation::Objective;
    let per_side = |team: Team| {
        squads
            .iter()
            .filter(|roster| roster.id.team == team)
            .count()
    };
    let squads_per_side = per_side(Team::Red).max(per_side(Team::Blue)).max(1);
    let stagger = (LANCE_MINUTE_MS / i64::try_from(squads_per_side).unwrap_or(1)) / LANCE_WINDOW_MS
        * LANCE_WINDOW_MS;
    let arm = |minute: i64, roster: &tacsim::deliberation::SquadRoster| -> Objective {
        match (minute - 1).rem_euclid(5) {
            0 => Objective::Hold,
            1 => Objective::Advance {
                to: LANCE_CROSSROADS,
            },
            2 => Objective::Withdraw,
            3 => Objective::Screen {
                facing_mdeg: if roster.id.team == Team::Red {
                    90_000
                } else {
                    270_000
                },
            },
            _ => Objective::Support {
                of: roster.commander,
            },
        }
    };
    let mut out: Vec<(i64, tacsim::deliberation::SquadId, Objective, u32)> = squads
        .iter()
        .map(|roster| {
            (
                start_ms,
                roster.id,
                Objective::Engage { target_hint: None },
                LANCE_ORDER_MS,
            )
        })
        .collect();
    let anchor = start_ms
        .div_euclid(LANCE_WINDOW_MS)
        .saturating_add(i64::from(start_ms.rem_euclid(LANCE_WINDOW_MS) != 0))
        * LANCE_WINDOW_MS;
    let mut minute = 1_i64;
    loop {
        let base = anchor.saturating_add(minute.saturating_mul(LANCE_MINUTE_MS));
        if base >= until_ms {
            break;
        }
        let stepped = usize::try_from((minute - 1) % i64::try_from(squads_per_side).unwrap_or(1))
            .unwrap_or(0)
            + 1;
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
                let objective = if ordinal == stepped {
                    arm(minute, roster)
                } else {
                    Objective::Engage { target_hint: None }
                };
                out.push((at_ms, roster.id, objective, LANCE_ORDER_MS));
            }
        }
        minute += 1;
    }
    out.sort_by_key(|(at_ms, ..)| *at_ms);
    out
}

/// **The lance lane's scripted release** (`tests/harness.rs`'
/// `LANCE_RELEASE_MS`): the arena millisecond each side's first squad is
/// released at, once, when the run spans it.
pub const LANCE_RELEASE_MS: i64 = 90_000;

/// **The lance lane's release, re-expressed** (`lance_schedule`'s 16b cut
/// Z.2 rows; 17b cut M2.6): at [`LANCE_RELEASE_MS`], when `start_ms <=`
/// it `< until_ms`, each side's first squad (`index` 1), as `(at_ms,
/// squad)` in the squads' order. The harness lists it after every scripted
/// order of the same millisecond, so the lane issues a mark's orders before
/// a release due with them.
#[must_use]
pub fn lance_releases(
    squads: &[tacsim::deliberation::SquadRoster],
    start_ms: i64,
    until_ms: i64,
) -> Vec<(i64, tacsim::deliberation::SquadId)> {
    if LANCE_RELEASE_MS < start_ms || LANCE_RELEASE_MS >= until_ms {
        return Vec::new();
    }
    squads
        .iter()
        .filter(|roster| roster.id.index == 1)
        .map(|roster| (LANCE_RELEASE_MS, roster.id))
        .collect()
}

/// **One scripted release as the intents phase 2 applies** (17b cut M2.6;
/// `tests/harness.rs`' `lance_intents`' release arm): one
/// `IntentKind::ReleaseOrder` a hull of the squad the arena still holds, at
/// its epoch, from the lance-leader persona that gave the order (`issuer:
/// None`, as [`lance_intents`]).
pub fn lance_release_intents(
    arena: &Arena,
    squad: tacsim::deliberation::SquadId,
    seq: &mut u64,
) -> Vec<tacsim::arena::intents::Intent> {
    squad_intents(
        arena,
        squad,
        &IntentKind::ReleaseOrder {
            squad,
            issuer: None,
        },
        seq,
    )
}

/// **One scripted order as the intents the arena's phase 2 applies** (17b
/// cut M2.6; `tests/harness.rs`' `lance_intents`, and `tests/minds.rs`'
/// `order(..)` shape): one `IntentKind::Order` a hull of the squad the arena
/// still holds, at that hull's own current epoch, the order the squad's
/// lance-leader persona's (`issuer: None`, `by: Persona`) — a machine order,
/// as every order a mind's squad is given by its leader.
pub fn lance_intents(
    arena: &Arena,
    squad: tacsim::deliberation::SquadId,
    objective: tacsim::deliberation::Objective,
    horizon_ms: u32,
    seq: &mut u64,
) -> Vec<tacsim::arena::intents::Intent> {
    let kind = IntentKind::Order(Box::new(tacsim::deliberation::Order {
        squad,
        objective,
        target: None,
        horizon_ms,
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms: horizon_ms,
        issued_ms: 0,
        issuer: None,
        by: tacsim::deliberation::CharacterRef::Persona { squad },
        spot: None,
    }));
    squad_intents(arena, squad, &kind, seq)
}

/// `kind` once a hull of `squad` the arena still holds, each at the hull's
/// own current epoch on the channel `kind` requires.
fn squad_intents(
    arena: &Arena,
    squad: tacsim::deliberation::SquadId,
    kind: &IntentKind,
    seq: &mut u64,
) -> Vec<tacsim::arena::intents::Intent> {
    use tacsim::arena::intents::Intent;
    let Some(roster) = arena.squads.iter().find(|roster| roster.id == squad) else {
        return Vec::new();
    };
    roster
        .units
        .iter()
        .filter(|unit| arena.unit(**unit).is_some())
        .map(|unit| {
            *seq += 1;
            let channel = kind.channel_required();
            Intent {
                unit: *unit,
                channel,
                epoch: arena.epochs.current(*unit, channel),
                kind: kind.clone(),
                seq: *seq,
            }
        })
        .collect()
}

/// One script function as the text reads it: its name, its parameters and
/// its body's lines (17b cut M2.5's pin, lifted here by 17b cut M2.6 so the
/// authored lane reports the same figures).
pub type ScriptFn = (String, Vec<String>, Vec<String>);

/// The script's `fn`s in order — read off the text (every `fn` opens at the
/// start of a line).
#[must_use]
pub fn script_fns(source: &str) -> Vec<ScriptFn> {
    let mut fns: Vec<ScriptFn> = Vec::new();
    for line in source.lines() {
        if let Some(head) = line.strip_prefix("fn ") {
            let (name, rest) = head.split_once('(').expect("a fn head");
            let (params, _) = rest.split_once(')').expect("a fn head");
            let params = params
                .split(',')
                .map(str::trim)
                .filter(|param| !param.is_empty())
                .map(str::to_owned)
                .collect();
            fns.push((name.to_owned(), params, Vec::new()));
        } else if let Some((_, _, body)) = fns.last_mut() {
            body.push(line.to_owned());
        }
    }
    fns
}

/// The names a body binds (`let x`, `for x in`, `for (x, i) in` — 17b cut
/// M2.6), each once.
#[must_use]
pub fn bound_names(body: &[String]) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    for line in body {
        let code = line.split("//").next().unwrap_or_default();
        for binder in ["let ", "for "] {
            for after in code.split(binder).skip(1) {
                let binds = after
                    .strip_prefix('(')
                    .and_then(|pair| pair.split_once(')'))
                    .map_or(after, |(pair, _)| pair);
                for bound in binds.split(',') {
                    let name: String = bound
                        .trim_start()
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        names.insert(name);
                    }
                    if !after.starts_with('(') {
                        break;
                    }
                }
            }
        }
    }
    names
}

/// **The script's figures under the binding limits** (17b cut M2.5's pin,
/// 17b cut M2.6's report): `(functions, the smallest max_function_expr_depth
/// it compiles at, the widest scope's function, its width)` — the width the
/// parameters plus every bound name of one body.
#[must_use]
pub fn script_figures(source: &str) -> (usize, usize, String, usize) {
    use sha2::{Digest as _, Sha256};
    use tacsim::minds::{MIND_ABI, STOCK_OPS_PER_THINK, register_api};
    let fns = script_fns(source);
    let (widest, width) = fns
        .iter()
        .map(|(name, params, body)| {
            let mut names: std::collections::BTreeSet<String> = params.iter().cloned().collect();
            names.extend(bound_names(body));
            (name.clone(), names.len())
        })
        .max_by_key(|(_, width)| *width)
        .unwrap_or_default();
    let digest = mush_mind::Digest(Sha256::digest(source.as_bytes()).into());
    let depth = (1..=24)
        .find(|depth| {
            mush_mind::Sandbox::new(
                &mush_mind::EngineLimits {
                    max_ops_backstop: STOCK_OPS_PER_THINK,
                },
                |engine| {
                    register_api(engine);
                    engine.set_max_expr_depths(48, *depth);
                },
            )
            .compile(source, digest, MIND_ABI)
            .is_ok()
        })
        .unwrap_or(0);
    (fns.len(), depth, widest, width)
}
