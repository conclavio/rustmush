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
    mods.extend(comp::sheet_modules(&onboarding, &game));
    mods.push(comp::tacsim_module(cfg));
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
    mods.push(comp::tacsim_module(cfg));
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
            mode: tacsim::arena::MatchMode::Skirmish,
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
