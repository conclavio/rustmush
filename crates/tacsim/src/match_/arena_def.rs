//! **Where a match is fought, and how fast a tank is** (16b cut T.3; doc 16 §3.2
//! "Space", §3.3 "Mobility mode rules", §4 "Content packs").
//!
//! Two functions the check-out needs and nothing else has yet:
//!
//! * [`stock_definition`] — the arena a match names. Cut C authors
//!   `content/arenas/proving_ground.json` with real tiles, entry zones, key spots
//!   and a waypoint graph, and this function becomes a registry lookup. Until
//!   then it builds the **same window** (2.56 km at 10 m cells, D-16-16) out of
//!   uniform ground, so the geometry, the pose arithmetic, the index and the
//!   custody round trip are all exercised at production size rather than at a
//!   toy one.
//! * [`tracked_rules`] — the `tracked` mode's kinematics, read out of the M32
//!   unit's opaque [`ModeRules`] by the game crate, which is the only layer
//!   allowed to interpret them (doc 16 §3.3).
//!
//! [`ModeRules`]: mush_tactics::ModeRules

use mush_sim::data::Pose;
use smol_str::SmolStr;

use crate::arena::state::{
    ArenaDefinition, ArenaTerrain, EntryZone, Team, TerrainCell, TrackedRules,
};

/// The arena window's side, in cells. 256 cells at 10 m is 2.56 km (D-16-16).
pub const ARENA_CELLS: u16 = 256;

/// One cell's side, centimetres: 10 m (doc 16 §3.2).
pub const ARENA_CELL_CM: u16 = 1_000;

/// How deep, in cells, each side's entry zone is.
const ENTRY_DEPTH: u16 = 8;

/// The arena definition every match this cut runs is fought on.
///
/// `id` is the definition the match named. Exactly one id resolves today — the
/// proving ground — and everything else answers `None`, so a typo is a refusal
/// naming the id rather than a match on a window nobody authored.
#[must_use]
pub fn stock_definition(id: &SmolStr) -> Option<ArenaDefinition> {
    if id.as_str() != PROVING_GROUND && id.as_str() != "proving_ground" {
        return None;
    }
    let ground = TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        // Flat, because cut T.3 is testing custody and not terrain: every
        // interesting tile is cut C's, and a relief this cut invented would be a
        // second authored arena nobody reviewed.
        elev_cm: 0,
        move_cost: 100,
    };
    let terrain = ArenaTerrain::uniform(ARENA_CELLS, ARENA_CELLS, ARENA_CELL_CM, ground).ok()?;
    Some(ArenaDefinition {
        id: SmolStr::new_static(PROVING_GROUND),
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

/// The one arena definition id this cut resolves.
pub const PROVING_GROUND: &str = "tacsim.arena.proving_ground";

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
/// **stock tank's** values for what is missing — 15 m/s, 2 m/s² away, 4 m/s²
/// back, 30°/s, 40 % in reverse — which are the numbers doc 16 §3.3's
/// `line_tank` carries and cut C's `units/line_tank.json` will author. A unit that
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

/// The locomotion id doc 16 §3.3's mode rules belong to.
const TRACKED: &str = "tracked";

/// Doc 16 §3.3's stock tank, as the documented fallback.
const STOCK_TRACKED: TrackedRules = TrackedRules {
    max_speed_cms: 1_500,
    accel_cms2: 200,
    brake_cms2: 400,
    turn_rate_mdeg_s: 30_000,
    reverse_factor_pct: 40,
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
