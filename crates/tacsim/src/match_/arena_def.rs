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
//! * [`tracked_rules`] — the `tracked` mode's kinematics, read out of the M32
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
use crate::arena::state::{
    ArenaDefinition, ArenaTerrain, EntryZone, Team, TerrainCell, TrackedRules, WeaponSummary,
};
use crate::arena::weapons::WeaponBand;
use crate::definitions::Catalog;
use crate::definitions::unit::ComponentRole;

/// The arena window's side, in cells. 256 cells at 10 m is 2.56 km (D-16-16).
pub const ARENA_CELLS: u16 = 256;

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
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: ENTRY_DEPTH.saturating_sub(1),
                y1: ARENA_CELLS.saturating_sub(1),
            },
            EntryZone {
                team: Team::Blue,
                x0: ARENA_CELLS.saturating_sub(ENTRY_DEPTH),
                y0: 0,
                x1: ARENA_CELLS.saturating_sub(1),
                y1: ARENA_CELLS.saturating_sub(1),
            },
        ],
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
#[must_use]
pub fn opening_pose(
    definition: &ArenaDefinition,
    team: Team,
    world_pose: Option<Pose>,
    taken: &mut Vec<(u16, u16)>,
) -> Option<Pose> {
    if let Some(pose) = world_pose.clone()
        && pose.heading_mdeg < 360_000
        && let Some(cell) = definition.terrain.cell_of(&pose)
        && !taken.contains(&cell)
        && definition
            .terrain
            .get(cell.0, cell.1)
            .is_some_and(TerrainCell::passable)
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
            if !definition
                .terrain
                .get(x, y)
                .is_some_and(TerrainCell::passable)
            {
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
                Team::Red => 90_000,
                Team::Blue => 270_000,
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

/// **The `tracked` mode's rules** (doc 16 §3.3 "Mobility mode rules").
///
/// M32 keeps a mode's kinematics in an opaque [`mush_tactics::ModeRules`] — a
/// digest-bound rules value the engine never interprets — and doc 16 §3.3 names
/// the five fields the game crate decodes out of it. This is that decode, and the
/// only place in the tree that reads those keys.
///
/// The mode taken is the unit's **effective** mode when its locomotion is
/// `tracked`, else the first `tracked` mode the profile declares. A unit with no
/// mobility profile, no tracked mode, or a rules blob missing a field gets the
/// **stock tank's** values for what is missing — 15 m/s, 1 m/s² away, 2 m/s²
/// back, 30°/s, 50 % in reverse — which are the numbers
/// `content/units/line_tank.json` authors (16b cut C.2), so the fallback and the
/// shipped tank cannot disagree. A unit that
/// simply cannot move is a different thing and says so: the profile declares no
/// tracked mode *and* the caller wanted one, so the defaults still let the arena
/// open, and the unit's own consequences (a destroyed engine, a wreck) are what
/// stop it.
#[must_use]
pub fn tracked_rules(unit: &mush_tactics::TacticalUnit) -> TrackedRules {
    let mut rules = STOCK_TRACKED;
    let Some(profile) = unit.mobility() else {
        return rules;
    };
    let mode = profile
        .modes()
        .iter()
        .find(|mode| mode.locomotion.as_str() == TRACKED)
        .or_else(|| {
            profile
                .modes()
                .iter()
                .find(|mode| mode.id.as_str().contains(TRACKED))
        });
    let Some(mode) = mode else {
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
#[must_use]
pub fn sensor_profile(
    catalog: &Catalog,
    unit: &mush_tactics::TacticalUnit,
) -> Option<SensorProfile> {
    let spec = spec_of(catalog, unit)?;
    let named = spec
        .components
        .iter()
        .find_map(|component| match &component.role {
            ComponentRole::Sensor { sensor } => Some(sensor),
            _ => None,
        })?;
    let def = entry(&catalog.sensors, named.as_str())?;
    Some(SensorProfile {
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
        });
    }
    out
}

/// The locomotion id doc 16 §3.3's mode rules belong to.
const TRACKED: &str = "tracked";

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
            let pose = opening_pose(&definition, Team::Red, None, &mut taken)
                .unwrap_or_else(|| panic!("an entry zone of 8 by 256 has twelve free cells"));
            let cell = definition
                .terrain
                .cell_of(&pose)
                .unwrap_or_else(|| panic!("an opening pose is on the window"));
            assert!(!cells.contains(&cell), "two units opened on {cell:?}");
            cells.push(cell);
        }
    }
}
