//! **Where a match is fought, and how fast a tank is** (16b cut T.3; doc 16 §3.2
//! "Space", §3.3 "Mobility mode rules", §4 "Content packs").
//!
//! Two functions the check-out needs:
//!
//! * [`definition_for`] — the arena a match names. **16b cut C.2** made it a
//!   content lookup: a `@arena/start` that names an authored arena gets
//!   `crates/tacsim/content/arenas/proving_ground.json`'s real ground, built
//!   over this world's atlas records by
//!   [`crate::definitions::from_pack`]. [`stock_definition`] is what is left of
//!   T.3's stand-in — the **same window** (2.56 km at 10 m cells, D-16-16) out of
//!   uniform ground — and it answers only where no pack does, which is a world
//!   the content has not been applied to and the unit tests of this crate.
//! * [`tracked_rules`] — the kinematics of the mode a unit drives under
//!   (`tracked`, or the `legged` one 16b cut X.2 added), read out of the M32
//!   unit's opaque [`ModeRules`] by the game crate, which is the only layer
//!   allowed to interpret them (doc 16 §3.3).
//! * [`sensor_profile`] and [`weapon_summaries`] (**16b cut S.1**) — what a unit
//!   sees with and shoots with, read out of the loaded catalog's authored
//!   `UnitSpec` at check-out. The arena decodes no content and holds no registry
//!   (D-16-1, D-16-3), so the seam carries the decoded rows: cut S needs the
//!   sensor's every field and the weapons' longest `max_range_cm` (which is what
//!   `tempo.alert_range_cm` derives from, 16b §9), and cut W needs the rest of
//!   each weapon row — so the whole of both crosses once, here.
//!
//! [`ModeRules`]: mush_tactics::ModeRules

use mush_core::CanonicalId;
use mush_sim::data::Pose;
use smol_str::SmolStr;

use crate::arena::perception::{SensorProfile, SignatureFactor};
use crate::arena::resolution::{BandProfile, Consequence, SectionProfile, UnitProfile};
use crate::arena::state::{
    ArenaDefinition, ArenaTerrain, CostRow, EntryZone, PartyId, TerrainCell, TrackedRules,
    WeaponSummary,
};
use crate::arena::weapons::WeaponBand;
use crate::definitions::Catalog;
use crate::definitions::resolution::{ErrorTerms, ResolutionDef};
use crate::definitions::unit::ComponentRole;

/// **The stand-in window's side, in cells** — since doc 32 cut CF.0 (Q-CF-51)
/// the shipped default, [`crate::definitions::arena::DEFAULT_WINDOW_SIDE`], and
/// not a constant of the arena: a region's window is its own `ArenaDef`'s.
pub const ARENA_CELLS: u16 = crate::definitions::arena::DEFAULT_WINDOW_SIDE;

/// One cell's side, centimetres: 10 m (doc 16 §3.2).
pub const ARENA_CELL_CM: u16 = 1_000;

/// How deep, in cells, each side's entry zone is.
const ENTRY_DEPTH: u16 = 8;

/// **The engine's own stand-in window** — 2.56 km of uniform ground at 10 m
/// cells, and no content at all (16b cut T.3; re-named by C.2).
///
/// It answers for two ids and no others:
///
/// * [`STAND_IN`], always. A world with no atlas composed and no content applied
///   has no terrain to build an arena out of, and this is what such a world
///   gets: the production geometry — the same side, the same cell, the same two
///   entry zones — with nothing authored in it. Every rig that needs a window to
///   exercise the pose arithmetic, the index, the motion or the custody round
///   trip names this one, and no pack may declare it.
/// * [`PROVING_GROUND`], as a **fallback**. If the shipped content did not bind,
///   a match on the game's own arena still gets a window of the right shape
///   rather than a refusal about a pack.
///
/// Everything else answers `None`, so a typo is a refusal naming the id rather
/// than a match on a window nobody authored.
#[must_use]
pub fn stock_definition(id: &SmolStr) -> Option<ArenaDefinition> {
    if id.as_str() != PROVING_GROUND && id.as_str() != STAND_IN {
        return None;
    }
    let ground = TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        // Flat, because a stand-in is geometry and not terrain: every
        // interesting tile is the content pack's, and a relief invented here
        // would be a second authored arena nobody reviewed.
        elev_cm: 0,
        move_cost: 100,
    };
    let terrain = ArenaTerrain::uniform(ARENA_CELLS, ARENA_CELLS, ARENA_CELL_CM, ground).ok()?;
    Some(ArenaDefinition {
        // The id asked for, so a result record names the arena the match was
        // started on and never a different one.
        id: id.clone(),
        terrain,
        entry_zones: vec![
            EntryZone {
                team: PartyId::RED,
                x0: 0,
                y0: 0,
                x1: ENTRY_DEPTH.saturating_sub(1),
                y1: ARENA_CELLS.saturating_sub(1),
            },
            EntryZone {
                team: PartyId::BLUE,
                x0: ARENA_CELLS.saturating_sub(ENTRY_DEPTH),
                y0: 0,
                x1: ARENA_CELLS.saturating_sub(1),
                y1: ARENA_CELLS.saturating_sub(1),
            },
        ],
        // **16b cut N.1**: the stand-in window has no authored furniture. A
        // controller on it patrols nowhere and holds where it stands, which is the
        // honest answer for a window nobody drew — and inventing a graph here would
        // be a second authored arena nobody reviewed, exactly as the flat ground
        // above would be.
        key_spots: Vec::new(),
        waypoints: crate::arena::state::Waypoints::default(),
        // **16b cut O.1**: and no modes — with no key spot there is nothing to
        // hold, so a stand-in match is a skirmish and nothing else.
        modes: Vec::new(),
        homes: Vec::new(),
    })
}

/// The one arena definition this crate's content ships
/// (`content/arenas/proving_ground.json`).
pub const PROVING_GROUND: &str = "tacsim.arena.proving_ground";

/// The engine's stand-in window (see [`stock_definition`]). No pack declares it,
/// so no content can shadow it and no rig that names it can be broken by a
/// content edit.
pub const STAND_IN: &str = "tacsim.arena.stand_in";

/// **The arena `id` names, over this world** (16b cut C.2).
///
/// The authored one when a loaded pack declares it and this world's atlas holds
/// its ground; otherwise [`stock_definition`]'s uniform window, for the same id.
/// The fallback is not a second lane — it is what a world that has not applied
/// the content pack has, and it answers with the same geometry so a refusal
/// names the arena and never the terrain.
///
/// A pack arena whose atlas records are *missing* is a refusal, not a fallback:
/// silently substituting flat ground for an authored map would make a match a
/// different match, and doc 16 §4's whole point is that a result record names
/// the arena digest it was fought on.
///
/// # Errors
///
/// A message naming the arena when a loaded pack declares it and its ground
/// could not be built.
pub fn definition_for(
    w: &mush_core::World,
    id: &SmolStr,
) -> Result<Option<ArenaDefinition>, String> {
    let Some(def) = w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|slot| {
            mush_core::CanonicalId::new(id.as_str())
                .ok()
                .and_then(|entry| slot.definitions().arenas.get(&entry).cloned())
        })
    else {
        return Ok(stock_definition(id));
    };
    let records = w
        .module_state::<mush_atlas::AtlasState>()
        .map(|state| &state.records)
        .ok_or_else(|| format!("{id}: the atlas is not composed on this world"))?;
    crate::definitions::from_pack(id.as_str(), &def, records)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// **Where a unit stands when the match opens.**
///
/// The unit's own world pose when it is on the window and its cell is free — so a
/// staffer who placed two tanks 300 m apart gets two tanks 300 m apart, and the
/// verify lens's "drive ten cells and check the world placement" is a statement
/// about the same coordinate system at both ends. Otherwise the next free cell of
/// that side's entry zone, in row-major order, which is always deterministic.
///
/// `taken` accumulates the cells already handed out, because doc 16 §3.4's
/// occupancy rule is one unit per cell and `Arena::new` refuses a check-out that
/// breaks it.
///
/// **16b cut M.4** (16c `T-14`): either way the cell must be passable **and
/// have a way out** — a passable orthogonal neighbour
/// ([`ArenaTerrain::has_way_out`](crate::arena::state::ArenaTerrain::has_way_out)).
/// A world pose on a passable cell walled in on four sides falls through to the
/// zone, exactly as one on an occupied cell does, and the zone's own walk skips
/// such a cell; `crate::definitions::arena::from_pack` refuses a zone with none
/// to give, and `Arena::new` refuses an impassable opening cell whoever chose it.
///
/// **16b cut AE.1** (16c `B-58`): both readings are **the unit's own row**
/// (`row`, [`crate::arena::state::CostRow::read`] off the unit being placed):
/// a car a staffer left on rubble falls through to its zone exactly as T-14
/// says, instead of passing here on the tracked reading and being refused
/// `CellImpassable` by `Arena::new`.
#[must_use]
pub fn opening_pose(
    definition: &ArenaDefinition,
    team: PartyId,
    world_pose: Option<Pose>,
    taken: &mut Vec<(u16, u16)>,
    row: CostRow,
) -> Option<Pose> {
    if let Some(pose) = world_pose.clone()
        && pose.heading_mdeg < 360_000
        && let Some(cell) = definition.terrain.cell_of(&pose)
        && !taken.contains(&cell)
        && definition.terrain.has_way_out_for(row, cell.0, cell.1)
    {
        taken.push(cell);
        return Some(pose);
    }
    let zone = definition
        .entry_zones
        .iter()
        .find(|zone| zone.team == team)?;
    // The posture the unit already carried, or the engine's own passive one —
    // a pose needs an opaque posture and the arena never reads it.
    let posture = match world_pose.map(|pose| pose.posture) {
        Some(posture) => posture,
        None => mush_sim::data::passive_rules_value(serde_json::json!({})).ok()?,
    };
    for y in zone.y0..=zone.y1 {
        for x in zone.x0..=zone.x1 {
            if taken.contains(&(x, y)) {
                continue;
            }
            if !definition.terrain.has_way_out_for(row, x, y) {
                continue;
            }
            taken.push((x, y));
            // The cell's centre, so a unit never opens on a boundary the very
            // first substep would test.
            let half = i64::from(definition.terrain.cell_cm() / 2);
            let x_cm = i64::from(x) * i64::from(definition.terrain.cell_cm()) + half;
            let y_cm = i64::from(y) * i64::from(definition.terrain.cell_cm()) + half;
            // Facing the other side: red looks east, blue looks west.
            let heading = match team {
                PartyId::RED => 90_000,
                _ => 270_000,
            };
            return Some(Pose {
                x_cm,
                y_cm,
                z_cm: 0,
                heading_mdeg: heading,
                posture,
            });
        }
    }
    None
}

/// **The world pose an `@arena/pose` opening cell stands for** (16b cut U.4, 16c
/// `T-52`): the cell's centre on `definition`'s window, at the cell's heading,
/// `z_cm` 0, on `posture` — the unit's own, which the pose carries opaquely and
/// the arena never reads.
///
/// The centre for the reason the entry zone's walk uses it: a unit never opens on
/// a boundary the very first substep would test. It is only ever offered to
/// [`opening_pose`], which is where it is judged.
#[must_use]
pub fn opening_cell_pose(
    definition: &ArenaDefinition,
    cell: crate::match_::OpeningCell,
    posture: mush_sim::data::RulesValue,
) -> Pose {
    let cell_cm = i64::from(definition.terrain.cell_cm());
    let half = cell_cm / 2;
    Pose {
        x_cm: i64::from(cell.x) * cell_cm + half,
        y_cm: i64::from(cell.y) * cell_cm + half,
        z_cm: 0,
        heading_mdeg: cell.heading_mdeg,
        posture,
    }
}

/// **One unit, as the check-out hands it to the arena** (16b cut E.1; 16b §0.4).
///
/// It is the literal that used to sit inside [`crate::match_::start`]'s loop,
/// lifted out unchanged. Nothing about it is new and no behaviour moved: it
/// exists so that a test rig that wants a shipped-scale arena builds **what
/// production builds** rather than a second copy of the same eight decodes, one
/// of which will eventually be a decode behind. `start` is its only caller in
/// `src`.
///
/// The caller supplies what only the world knows — the unit's cells, its opening
/// pose (through [`opening_pose`], which is the rest of what `start` does per
/// unit), its side, its gunnery, whether anybody is sitting in it, and the pod it
/// came out of — and everything else is read off the catalog here.
///
/// **17b cut M3.3** (doc 17 §13; D-UM-45, D-UM-46): and, LAST, the seal —
/// [`crate::match_::seal::seal_for`] under `terms`, as a function of the
/// finished custody (its cells, id, controller and weapon rows), so the
/// channel set reads the weapon rows this function has just decoded. A rig
/// that changes the custody afterwards re-seals it
/// (`CustodyUnit::reseal`).
///
/// # Errors
///
/// [`crate::match_::seal::SealError`] when the terms' card does not pair or
/// sits under their Compact floor — refused, never clamped.
#[allow(clippy::too_many_arguments)]
pub fn custody_unit(
    catalog: &Catalog,
    cells: mush_tactics::TacticalUnit,
    id: mush_core::ObjId,
    team: PartyId,
    pose: Pose,
    gunnery: u8,
    controller: crate::arena::state::Controller,
    slot: u8,
    terms: &crate::match_::seal::SealTerms,
) -> Result<crate::arena::state::CustodyUnit, crate::match_::seal::SealError> {
    let mut unit = crate::arena::state::CustodyUnit {
        id,
        team,
        pose,
        mobility: tracked_rules(&cells),
        sensor_height_cm: sensor_height_cm(catalog, &cells),
        sensor: sensor_profile(catalog, &cells),
        weapons: weapon_summaries(catalog, &cells),
        // **16b cut W.2**: what the unit presents and what a hit may strike, and
        // the gunnery the `PilotProfile` seam gives its shots.
        profile: unit_profile(catalog, &cells),
        gunnery,
        // **16b cut N.1** (doc 16 §6): a tank with nobody in it is the reflex
        // controller's. That is the whole of how a lobby fields AI — `SIM START`
        // puts the unclaimed pods' tanks on the sides and they arrive here with
        // no seated pilot — and it is also what the harness's field of 300 is.
        controller,
        // **16b cut Q.1**: the pod it came out of, carried straight across — the
        // lobby is the only thing that knows it, and `Arena::new` is the only
        // thing that resolves it into a name.
        slot,
        // Laid down empty, and sealed below once every other field stands.
        seal: crate::arena::mind::MindSeal::placeholder(),
        unit: cells,
    };
    unit.reseal(terms)?;
    Ok(unit)
}

/// **The mode rules a unit drives under** (doc 16 §3.3 "Mobility mode rules").
///
/// M32 keeps a mode's kinematics in an opaque [`mush_tactics::ModeRules`] — a
/// digest-bound rules value the engine never interprets — and doc 16 §3.3 names
/// the five fields the game crate decodes out of it. This is that decode, and the
/// only place in the tree that reads those keys.
///
/// The mode taken is [`crate::arena::slope::driving_mode`]'s: the profile's
/// `tracked` one, **else its `wheeled` one** (16b cut AE.1: a scout car decodes
/// its own five numbers), **else its `legged` one** (16b cut X.2, doc 16 §9
/// item 2: *motion reads either*), else a mode whose id merely contains the
/// word `tracked` — the same four rungs the unit's slope limit and cost row are
/// read off (16b cuts X.6 and AE.1), written once so that there is one ladder
/// rather than several copies of it. What the readers do when the ladder
/// answers nothing differs, by intent: this one falls through to the stock tank
/// below, while [`crate::arena::slope::SlopeLimit::read`] and
/// [`crate::arena::state::CostRow::read`] fall back to the profile's first mode
/// so that a unit on a locomotion off the ladder (`hover`) is not silently
/// unlimited. The order is
/// the authored one
/// ([`crate::definitions::unit::MobilityDef::modes`]) so that a unit which
/// declared both drives here as it binds there. A unit with no mobility profile,
/// no mode this recognises, or a rules blob missing a field gets the **stock
/// tank's** values for what is missing — 15 m/s, 1 m/s² away, 2 m/s² back, 30°/s,
/// 50 % in reverse — which are the numbers `content/units/line_tank.json` authors
/// (16b cut C.2), so the fallback and the shipped tank cannot disagree. A unit
/// that simply cannot move is a different thing and says so: the profile declares
/// no such mode *and* the caller wanted one, so the defaults still let the arena
/// open, and the unit's own consequences (a destroyed engine, a lost pair of
/// legs, a wreck) are what stop it.
#[must_use]
pub fn tracked_rules(unit: &mush_tactics::TacticalUnit) -> TrackedRules {
    let mut rules = STOCK_TRACKED;
    let Some(profile) = unit.mobility() else {
        return rules;
    };
    // **16b cut X.6** moved the three rungs into `crate::arena::slope`, where the
    // slope limit reads them too: the kinematics and the limit must come off the
    // same mode, and two copies of the ladder are two modes waiting to disagree.
    let Some(mode) = crate::arena::slope::driving_mode(profile) else {
        return rules;
    };
    let data = mode.rules.data();
    if let Some(value) = u32_field(data, "max_speed_cms") {
        rules.max_speed_cms = value;
    } else if mode.nominal_speed_cms != 0 {
        // The one engine-known coarse magnitude, when the ruleset blob says
        // nothing (doc 07 §9's `nominal_speed_cms`).
        rules.max_speed_cms = mode.nominal_speed_cms;
    }
    if let Some(value) = u32_field(data, "accel_cms2") {
        rules.accel_cms2 = value;
    }
    if let Some(value) = u32_field(data, "brake_cms2") {
        rules.brake_cms2 = value;
    }
    if let Some(value) = u32_field(data, "turn_rate_mdeg_s") {
        rules.turn_rate_mdeg_s = value;
    }
    if let Some(value) = u32_field(data, "reverse_factor_pct") {
        rules.reverse_factor_pct = u8::try_from(value.min(100)).unwrap_or(100);
    }
    rules
}

/// **The authored spec behind one M32 unit**, if the loaded catalog holds it.
///
/// A [`mush_tactics::TacticalUnit`] carries its schema as a `DefinitionRef`, and
/// the catalog keys its specs by the pack entry id that minted that reference, so
/// the two meet at the definition id. A unit created outside the content — the
/// fixtures of the contract gates, a hand-made rig — matches nothing and gets
/// `None`, which is what makes the two readers below total.
fn spec_of<'a>(
    catalog: &'a Catalog,
    unit: &mush_tactics::TacticalUnit,
) -> Option<&'a crate::definitions::UnitSpec> {
    let wanted = unit.schema().key().id.as_str();
    catalog
        .specs
        .iter()
        .find(|(id, _)| id.as_str() == wanted)
        .map(|(_, spec)| spec)
}

/// One entry of the catalog, by its id as a string.
fn entry<'a, T>(table: &'a std::collections::BTreeMap<CanonicalId, T>, id: &str) -> Option<&'a T> {
    table
        .iter()
        .find(|(key, _)| key.as_str() == id)
        .map(|(_, value)| value)
}

/// **How high a unit's eye sits**, centimetres (16b cut S.1; doc 16 §3.2 "Line of
/// sight"), off the authored spec; zero when nothing authored it, which
/// `Arena::new` reads as
/// [`crate::arena::state::DEFAULT_SENSOR_HEIGHT_CM`].
#[must_use]
pub fn sensor_height_cm(catalog: &Catalog, unit: &mush_tactics::TacticalUnit) -> u32 {
    spec_of(catalog, unit).map_or(0, |spec| spec.sensor_height_cm)
}

/// **What a unit sees with** (16b cut S.1; doc 16 §3.6), decoded for the arena.
///
/// The unit's **first** `sensor` component, in the spec's component order, and the
/// `sensor` definition it names. One sensor is what doc 16 §3.3's stock tank has
/// (`visual_sensor`); a unit that declares several is read as seeing with the
/// first, and the ECM and ECCM extensions of §9 are where a second one earns its
/// own rule rather than a silent `max`.
///
/// `None` — no spec, no sensor component, or a definition the catalog does not
/// hold — runs no sensor pass at all, which is a unit that cannot see.
///
/// **16b cut M.4** (16c `B-53`): the profile carries the chosen component's own
/// id ([`SensorProfile::component`]), so the gunnery cone's `sensor_condition`
/// term reads the sight's `condition` cell and not the first one in id order.
#[must_use]
pub fn sensor_profile(
    catalog: &Catalog,
    unit: &mush_tactics::TacticalUnit,
) -> Option<SensorProfile> {
    let spec = spec_of(catalog, unit)?;
    let (component, named) =
        spec.components
            .iter()
            .find_map(|component| match &component.role {
                ComponentRole::Sensor { sensor, .. } => Some((component, sensor)),
                _ => None,
            })?;
    let def = entry(&catalog.sensors, named.as_str())?;
    Some(SensorProfile {
        component: SmolStr::new(component.id.as_str()),
        max_range_cells: def.max_range_cells,
        max_attenuation: def.max_attenuation,
        classify_after_ms: def.classify_after_ms,
        sees_wrecks: def.sees_wrecks,
        share_range_cells: def.share_range_cells,
        signature: def
            .signature
            .iter()
            .map(|row| SignatureFactor {
                class: row.class,
                moving_permille: row.moving_permille,
                stopped_permille: row.stopped_permille,
            })
            .collect(),
    })
}

/// **What a unit shoots with** (16b cut S.1 carries it, cut W reads it; doc 16
/// §3.5 "Weapon definition"), in the spec's component order.
///
/// One row per `weapon` component whose definition the catalog holds; a component
/// naming a weapon nothing authored is skipped rather than refusing the match,
/// because the catalog already refused that reference at load (doc 16 §4: *every
/// definition validates its references*) and a second refusal here would only ever
/// fire on a hand-made rig.
///
/// `max_range_cm` is the **outermost** band's edge: the bands are authored nearest
/// first, and the one number cut S reads off this row is the longest reach in
/// the match (16b §9's derived `tempo.alert_range_cm`).
///
/// **16b cut W.1** carries two more: the bands themselves, each with the
/// `dispersion_mrad` cut W.2's error cone begins at, and `count_per_load` off the
/// **ammunition** the weapon names — which is the one field of this row that is not
/// the weapon's own, and which sizes the unit's `ammo` cell exactly as
/// `definitions/unit.rs` sizes the M32 component cell from the same number. A weapon
/// whose ammunition the catalog does not hold gets a load of zero, which is a weapon
/// that refuses `FIRE` as empty; it cannot happen through the loader (a dangling
/// reference is refused at load) and the honest answer is better than a guess.
#[must_use]
pub fn weapon_summaries(
    catalog: &Catalog,
    unit: &mush_tactics::TacticalUnit,
) -> Vec<WeaponSummary> {
    let Some(spec) = spec_of(catalog, unit) else {
        return Vec::new();
    };
    let mut out: Vec<WeaponSummary> = Vec::new();
    for component in &spec.components {
        let ComponentRole::Weapon { weapon, .. } = &component.role else {
            continue;
        };
        let Some(def) = entry(&catalog.weapons, weapon.as_str()) else {
            continue;
        };
        out.push(WeaponSummary {
            id: SmolStr::new(component.id.as_str()),
            max_range_cm: def
                .bands
                .iter()
                .map(|band| band.max_range_cm)
                .max()
                .unwrap_or(0),
            cycle_ms: def.cycle_ms,
            ammunition: SmolStr::new(def.ammunition.as_str()),
            burst_rounds: def.burst_rounds,
            muzzle_speed_cms: def.muzzle_speed_cms,
            bands: def
                .bands
                .iter()
                .map(|band| WeaponBand {
                    max_range_cm: band.max_range_cm,
                    dispersion_mrad: band.dispersion_mrad,
                })
                .collect(),
            count_per_load: entry(&catalog.ammunition, def.ammunition.as_str())
                .map_or(0, |ammunition| ammunition.count_per_load),
            // **16b cut W.2**: the round's own damage, off the same ammunition row,
            // and the section the weapon is mounted in — which is the section
            // `remove_weapons` names when the turret is shot off. The mount is the
            // *weapon definition's* (doc 16 §3.5 "Weapon definition": *`mount`
            // (section and facet)*) and not the component's install point, because
            // the weapon is what says where it can be carried.
            //
            // **16b cut X.2**: unless this unit has no such section. A weapon says
            // where it can be carried on the hull it was drawn for, and a biped
            // carrying a turret gun in its right arm declares no turret at all —
            // so the mount falls back to where the unit actually installed it,
            // which is the section that goes when the arm is shot off. For every
            // shipped unit the two are the same word (both tanks install their
            // turret weapons in the turret), so nothing moves.
            damage: entry(&catalog.ammunition, def.ammunition.as_str())
                .map_or(0, |ammunition| ammunition.damage),
            mount_section: SmolStr::new(if spec.section(&def.mount.section).is_some() {
                def.mount.section.as_str()
            } else {
                component.section.as_str()
            }),
        });
    }
    out
}

/// **What a unit presents and what a hit may strike** (16b cut W.2; doc 16 §3.3
/// "Facet attribution" and "Silhouette"), decoded for the arena.
///
/// The authored spec keeps the two halves apart — `silhouettes` says what a band
/// presents and `facets` says what a hit on it may strike — and every reader of one
/// wants the other, so they are joined on the band id here, once, at check-out. A
/// band with a silhouette and no weights presents a target nothing can be attributed
/// to (a round arrives and finds no structure); a band with weights and no silhouette
/// presents nothing at all. Both are content mistakes the loader cannot refuse (the
/// two lists are independent), and both resolve honestly rather than panicking.
///
/// `None` — no spec in the catalog — is an empty profile, which is a unit that
/// presents nothing and is struck by nothing: every hand-made rig, and every world
/// the content has not been applied to.
#[must_use]
pub fn unit_profile(catalog: &Catalog, unit: &mush_tactics::TacticalUnit) -> UnitProfile {
    let Some(spec) = spec_of(catalog, unit) else {
        return UnitProfile::default();
    };
    let bands = spec
        .silhouettes
        .iter()
        .map(|silhouette| BandProfile {
            band: SmolStr::new(silhouette.band.as_str()),
            width_cm: silhouette.width_cm,
            height_cm: silhouette.height_cm,
            sections: spec
                .facets
                .iter()
                .find(|row| row.band == silhouette.band)
                .map(|row| {
                    row.sections
                        .iter()
                        .map(|weight| (SmolStr::new(weight.section.as_str()), weight.weight))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();
    let sections = spec
        .sections
        .iter()
        .map(|section| SectionProfile {
            id: SmolStr::new(section.id.as_str()),
            facets: section
                .facets
                .iter()
                .map(|facet| SmolStr::new(facet.id.as_str()))
                .collect(),
            consequences: section.consequences.iter().map(consequence_of).collect(),
            struck_through: section.struck_through.as_ref().map(|through| {
                (
                    SmolStr::new(through.section.as_str()),
                    SmolStr::new(through.facet.as_str()),
                )
            }),
        })
        .collect();
    UnitProfile { bands, sections }
}

/// The arena's own copy of one authored consequence. A closed vocabulary on both
/// sides, so this is a total function and not a decode that can fail.
fn consequence_of(authored: &crate::definitions::unit::Consequence) -> Consequence {
    use crate::definitions::unit::Consequence as Authored;
    match authored {
        Authored::DestroyUnit => Consequence::DestroyUnit,
        Authored::RemoveWeapons { section } => Consequence::RemoveWeapons {
            section: SmolStr::new(section.as_str()),
        },
        Authored::Slow { percent } => Consequence::Slow { percent: *percent },
        Authored::NoTurnToward { side } => Consequence::NoTurnToward { side: *side },
        Authored::DisableMotion => Consequence::DisableMotion,
        Authored::DisableMotionWhenAllOf { sections } => Consequence::DisableMotionWhenAllOf {
            sections: sections
                .iter()
                .map(|section| SmolStr::new(section.as_str()))
                .collect(),
        },
        // 17b cut M3.4 (D-UM-34): the two core words, carried across as they
        // are; the plan ops are `crate::arena::damage`'s.
        Authored::DegradeCore => Consequence::DegradeCore,
        Authored::DestroyCore => Consequence::DestroyCore,
    }
}

/// **The resolution table a match is resolved through** (16b cut W.2; doc 16 §3.5
/// steps 2 and 3), off the loaded catalog.
///
/// [`crate::definitions::content::RESOLUTION`] when the content declares it — the
/// one id the shipped pack names — and [`stock_resolution`] otherwise — the same fallback shape [`tracked_rules`] takes, and for the same
/// reason: a world the content pack has not been applied to still runs a match, with
/// the numbers the pack authors, rather than refusing for want of a curve.
#[must_use]
pub fn resolution_for(catalog: &Catalog) -> ResolutionDef {
    entry(
        &catalog.resolutions,
        crate::definitions::content::RESOLUTION,
    )
    .cloned()
    .unwrap_or_else(stock_resolution)
}

/// **Doc 16 §3.5's curve and weights, as the documented fallback** — the numbers
/// `content/resolution.json` authors (16b cut C.2), pinned equal to it by
/// `crates/tacsim/tests/resolution.rs`.
///
/// It exists for the same two callers [`stock_definition`] exists for: a world with
/// no content applied, and the unit tests of this crate, which need a curve without
/// booting a pack loader.
#[must_use]
pub fn stock_resolution() -> ResolutionDef {
    ResolutionDef {
        label: "Stock direct-fire resolution".to_owned(),
        hit_curve: vec![
            [0, 0],
            [250, 50],
            [500, 150],
            [1_000, 350],
            [2_000, 600],
            [4_000, 820],
            [8_000, 940],
            [16_000, 980],
        ],
        error_terms: ErrorTerms {
            shooter_speed: 1_000,
            target_lateral_speed: 800,
            gunnery: 1_000,
            sensor_condition: 500,
            // 16b cut X.1: the term `content/resolution.json` does not author, at
            // its documented default — which is what keeps this fallback and the
            // pack byte-identical when serialised.
            beam_attenuation: crate::definitions::resolution::DEFAULT_BEAM_ATTENUATION,
        },
        p_min: 10,
        p_max: 950,
        note: String::new(),
    }
}

// **16b cut X.6** retired this file's own `TRACKED` and `LEGGED` copies with the
// mode ladder that read them: the ladder now lives in `crate::arena::slope`,
// which reads `crate::definitions::unit`'s originals.

/// Doc 16 §3.3's stock tank, as the documented fallback — the five numbers
/// `content/units/line_tank.json` authors, pinned equal to it by
/// `crates/tacsim/tests/definitions.rs`.
const STOCK_TRACKED: TrackedRules = TrackedRules {
    max_speed_cms: 1_500,
    accel_cms2: 100,
    brake_cms2: 200,
    turn_rate_mdeg_s: 30_000,
    reverse_factor_pct: 50,
};

/// One unsigned field of an opaque rules blob, when it is there and fits.
fn u32_field(data: &serde_json::Value, field: &str) -> Option<u32> {
    u32::try_from(data.get(field)?.as_u64()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stock_arena_is_the_documented_window_with_two_entry_zones() {
        let id = SmolStr::new_static(PROVING_GROUND);
        let definition = stock_definition(&id).unwrap_or_else(|| {
            panic!("the proving ground resolves");
        });
        assert_eq!(definition.terrain.width(), ARENA_CELLS);
        assert_eq!(definition.terrain.height(), ARENA_CELLS);
        assert_eq!(definition.terrain.cell_cm(), ARENA_CELL_CM);
        assert_eq!(
            i64::from(ARENA_CELLS) * i64::from(ARENA_CELL_CM),
            256_000,
            "2.56 km on a side (D-16-16)"
        );
        assert_eq!(definition.entry_zones.len(), 2);
        assert_eq!(stock_definition(&SmolStr::new("nope")), None);
    }

    #[test]
    fn opening_poses_never_share_a_cell() {
        let id = SmolStr::new_static(PROVING_GROUND);
        let definition = stock_definition(&id).unwrap_or_else(|| {
            panic!("the proving ground resolves");
        });
        let mut taken = Vec::new();
        let mut cells = Vec::new();
        for _ in 0..12 {
            let pose = opening_pose(
                &definition,
                PartyId::RED,
                None,
                &mut taken,
                CostRow::Tracked,
            )
            .unwrap_or_else(|| panic!("an entry zone of 8 by 256 has twelve free cells"));
            let cell = definition
                .terrain
                .cell_of(&pose)
                .unwrap_or_else(|| panic!("an opening pose is on the window"));
            assert!(!cells.contains(&cell), "two units opened on {cell:?}");
            cells.push(cell);
        }
    }

    /// **16b cut M.4 (16c `T-14`): an opening cell has a way out.** A world pose
    /// on a passable cell whose four orthogonal neighbours are all impassable
    /// falls through to the zone, exactly as one on an occupied cell does; the
    /// zone's own walk skips such a cell too; a world pose with an open
    /// neighbour is kept.
    #[test]
    fn m4_a_world_pose_on_a_walled_in_cell_falls_through_to_the_zone() {
        use crate::arena::state::{ArenaTerrain, EntryZone, Waypoints};
        let ground = TerrainCell {
            class: mush_atlas::TerrainClass::Ground,
            elev_cm: 0,
            move_cost: 100,
        };
        let wall = TerrainCell {
            class: mush_atlas::TerrainClass::Wall,
            elev_cm: 0,
            move_cost: 0,
        };
        let mut cells = vec![ground; 64];
        // (4, 4) walled in on four sides; (0, 0) walled in by (1, 0) and (0, 1),
        // the zone's first cell in row-major order.
        for (x, y) in [(3, 4), (5, 4), (4, 3), (4, 5), (1, 0), (0, 1)] {
            if let Some(slot) = cells.get_mut(y * 8 + x) {
                *slot = wall;
            }
        }
        let definition = ArenaDefinition {
            id: SmolStr::new("m4.walled"),
            terrain: ArenaTerrain::new(8, 8, 1_000, cells).expect("a window"),
            entry_zones: vec![EntryZone {
                team: PartyId::RED,
                x0: 0,
                y0: 0,
                x1: 2,
                y1: 2,
            }],
            key_spots: Vec::new(),
            waypoints: Waypoints::default(),
            modes: Vec::new(),
            homes: Vec::new(),
        };
        let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
            .unwrap_or_else(|_| panic!("a posture"));
        let at = |x: i64, y: i64| Pose {
            x_cm: x * 1_000 + 500,
            y_cm: y * 1_000 + 500,
            z_cm: 0,
            heading_mdeg: 0,
            posture: posture.clone(),
        };
        assert!(definition.terrain.has_way_out(6, 6));
        assert!(!definition.terrain.has_way_out(4, 4));
        assert!(!definition.terrain.has_way_out(0, 0));
        let mut taken = Vec::new();
        let placed = opening_pose(
            &definition,
            PartyId::RED,
            Some(at(4, 4)),
            &mut taken,
            CostRow::Tracked,
        )
        .unwrap_or_else(|| panic!("the zone has room"));
        let cell = definition.terrain.cell_of(&placed);
        assert_ne!(cell, Some((4, 4)), "the walled-in world pose was kept");
        assert_ne!(cell, Some((0, 0)), "the zone handed out a walled-in cell");
        assert_eq!(cell, Some((2, 0)), "the zone's first cell with a way out");
        let kept = opening_pose(
            &definition,
            PartyId::RED,
            Some(at(6, 6)),
            &mut taken,
            CostRow::Tracked,
        )
        .unwrap_or_else(|| panic!("an open world pose"));
        assert_eq!(definition.terrain.cell_of(&kept), Some((6, 6)));
    }
}
