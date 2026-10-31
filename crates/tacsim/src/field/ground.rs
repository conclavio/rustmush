//! **A field's ground** (doc 32 cut CF.2; Q-CF-13, Q-CF-22, Q-CF-26, Q-CF-51): the site's own
//! window, as the kernel reads one.
//!
//! The whole site, on its own keyless terrain (`crate::surface::ground::site_terrain_and_biome`),
//! priced per row as an authored arena is (`crate::definitions::arena::from_pack`'s rule), at the
//! composite's own side — never a constant (Q-CF-51). The boundary is the window's edge,
//! impassable as it is on the campaign surface (Q-CF-22). No entry zones (Q-CF-17: promotion is
//! the only entry), no key spots, no patrol graph, no mode but the skirmish.
//!
//! **The site record's objects** (Q-CF-26): remains and crates are static obstacles over their
//! footprints, and give cover by them (doc 32 cut CF.4): each footprint cell reads as a wall
//! ([`OBJECT_CLASS`]) — impassable to every ground row at the class's own price, opaque and
//! `WALL_HEIGHT_CM` tall — so a unit behind one is out of the line of sight and fire of a unit
//! beyond it. That is the kernel's one way to give cover by a cell nobody stands in: a class's own
//! cover is the cover of the unit standing in it. CF.2 set the tracked price to 0 and kept the
//! class, which stopped a tank and no biped. Every picture of the field draws the cells as the
//! kernel holds them. Objects are also sensor returns, seen by line of sight over this ground
//! (`crate::field::known::sensed`), never kernel contacts.

use mush_core::{CanonicalId, World};
use smol_str::SmolStr;

use crate::arena::state::{ArenaDefinition, ArenaTerrain, TerrainCell, Waypoints};
use crate::surface::site_record::SiteRecord;

/// **The class an object's footprint cell reads as** (doc 32 cut CF.4; Q-CF-26): a wall —
/// impassable on the ground, opaque, three metres tall.
pub(crate) const OBJECT_CLASS: mush_atlas::TerrainClass = mush_atlas::TerrainClass::Wall;

/// The ground's id: `field:<site>`.
#[must_use]
pub(crate) fn ground_id(site: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("field:{site}"))
}

/// **The window a field on `site` is fought on**, with `record`'s objects laid on it.
///
/// # Errors
///
/// The site's terrain did not build, or its grid is not a window.
pub(crate) fn definition(
    w: &World,
    site: &CanonicalId,
    record: &SiteRecord,
) -> Result<ArenaDefinition, String> {
    let (composite, biome) = crate::surface::ground::site_terrain_and_biome(w, site)?;
    let ground = composite
        .surface(&mush_atlas::SurfaceRef::Ground)
        .ok_or_else(|| format!("{site}'s window has no ground surface"))?;
    let (width, height) = (ground.cells.width(), ground.cells.height());
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let mut cells: Vec<TerrainCell> =
        Vec::with_capacity(usize::from(width).saturating_mul(usize::from(height)));
    for y in 0..height {
        for x in 0..width {
            let cell = ground
                .cells
                .get(x, y)
                .ok_or_else(|| format!("{site}'s window is missing cell ({x}, {y})"))?;
            cells.push(TerrainCell {
                class: cell.class,
                elev_cm: cell.elev_cm,
                move_cost: crate::definitions::arena::move_cost_of(table, cell.class),
            });
        }
    }
    // Q-CF-26: every object's footprint, centred on its cell, is impassable and hides what is
    // behind it (doc 32 cut CF.4), at the class's own price so every row agrees.
    let wall_cost = crate::definitions::arena::move_cost_of(table, OBJECT_CLASS);
    for object in &record.objects {
        let (cx, cy) = object.pose.cell();
        let half = i64::from(object.footprint) / 2;
        let lo = |c: i64| c.saturating_sub(half);
        for y in lo(cy)..lo(cy).saturating_add(i64::from(object.footprint)) {
            for x in lo(cx)..lo(cx).saturating_add(i64::from(object.footprint)) {
                let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
                    continue;
                };
                if x >= width || y >= height {
                    continue;
                }
                let index = usize::from(y) * usize::from(width) + usize::from(x);
                if let Some(cell) = cells.get_mut(index) {
                    cell.class = OBJECT_CLASS;
                    cell.move_cost = wall_cost;
                }
            }
        }
    }
    let terrain = ArenaTerrain::priced(width, height, composite.frame.cell_cm, cells, table)
        .map_err(|error| error.to_string())?
        .with_biome(biome);
    Ok(ArenaDefinition {
        id: ground_id(site),
        terrain,
        entry_zones: Vec::new(),
        key_spots: Vec::new(),
        waypoints: Waypoints::new(Vec::new(), &[]),
        modes: Vec::new(),
    })
}
