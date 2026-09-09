//! The background recipe (doc 14 §3.2, plan §3.1 P4): what the ground is where nothing
//! is drawn — a seeded hash of `(region, surface, lattice cell)` into a class and an
//! elevation from the recipe's parameters. Pure: two calls agree, and nothing is
//! written until a tile is rasterized over it.

use crate::record::{BackgroundRecipe, RegionRec, SurfaceRef};
use crate::terrain::TerrainClass;

/// What the background says about one point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ground {
    /// The class.
    pub class: TerrainClass,
    /// Elevation against the region datum, centimetres.
    pub elev_cm: i32,
}

/// The background at `(x_cm, y_cm)` on `surface` of `region`.
#[must_use]
pub fn background(region: &RegionRec, surface: &SurfaceRef, x_cm: i64, y_cm: i64) -> Ground {
    let recipe = &region.background;
    let period = i64::from(recipe.relief_period_cm.max(1));
    let lattice_x = x_cm.div_euclid(period);
    let lattice_y = y_cm.div_euclid(period);
    let h = lattice_hash(region, surface, lattice_x, lattice_y);
    let class = pick_class(recipe, h);
    let relief = i32::from(recipe.relief_cm);
    // A second, independent draw for elevation.
    let h2 = fnv1a(&h.to_le_bytes());
    #[allow(clippy::cast_possible_truncation)]
    let noise = if relief == 0 {
        0
    } else {
        (h2 % (u64::from(relief.unsigned_abs()) * 2 + 1)) as i32 - relief
    };
    let elev_cm = region.datum_cm.saturating_add(noise);
    let class = match recipe.water_cm {
        Some(water) if elev_cm < water => TerrainClass::Water,
        _ => class,
    };
    Ground { class, elev_cm }
}

fn pick_class(recipe: &BackgroundRecipe, hash: u64) -> TerrainClass {
    #[allow(clippy::cast_possible_truncation)]
    let roll = (hash % 100) as u32;
    let mut acc: u32 = 0;
    for (class, weight) in &recipe.mix {
        acc = acc.saturating_add(u32::from(*weight));
        if roll < acc {
            return *class;
        }
    }
    recipe.surface
}

fn lattice_hash(region: &RegionRec, surface: &SurfaceRef, lx: i64, ly: i64) -> u64 {
    let mut bytes: Vec<u8> = Vec::with_capacity(96);
    bytes.extend_from_slice(region.key.suffix().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(surface.to_string().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&region.background.seed.to_le_bytes());
    bytes.extend_from_slice(&lx.to_le_bytes());
    bytes.extend_from_slice(&ly.to_le_bytes());
    fnv1a(&bytes)
}

/// FNV-1a, 64-bit.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Point2, Rect2};
    use crate::record::{RecordKey, RegionKind};
    use mush_core::CanonicalId;

    fn region(relief: u16, mix: Vec<(TerrainClass, u8)>) -> RegionRec {
        RegionRec {
            key: RecordKey::new(
                CanonicalId::new("region.limbo").expect("id"),
                CanonicalId::new("region.limbo").expect("id"),
            ),
            name: "The Lot".to_string(),
            kind: RegionKind::Surface {
                body: "earth".to_string(),
            },
            frame: None,
            parent: None,
            extent: Rect2(Point2(0.0, 0.0), Point2(400.0, 400.0)),
            datum_cm: 0,
            background: BackgroundRecipe {
                surface: TerrainClass::Paved,
                mix,
                relief_cm: relief,
                relief_period_cm: 800,
                water_cm: None,
                seed: 7,
            },
            ground: Vec::new(),
            ground_resolution_cm: 1000,
            revision: 1,
        }
    }

    #[test]
    fn two_calls_agree_and_a_flat_recipe_is_flat() {
        let r = region(0, Vec::new());
        let a = background(&r, &SurfaceRef::Ground, 12_345, -678);
        let b = background(&r, &SurfaceRef::Ground, 12_345, -678);
        assert_eq!(a, b);
        assert_eq!(a.elev_cm, 0);
        assert_eq!(a.class, TerrainClass::Paved);
    }

    #[test]
    fn relief_stays_in_band_and_the_mix_is_roughly_honoured() {
        let r = region(150, vec![(TerrainClass::Grass, 30)]);
        let mut grass = 0_u32;
        let mut samples = 0_u32;
        for lx in 0..40_i64 {
            for ly in 0..40_i64 {
                let g = background(&r, &SurfaceRef::Ground, lx * 800, ly * 800);
                assert!(g.elev_cm.abs() <= 150, "relief {} out of band", g.elev_cm);
                samples += 1;
                if g.class == TerrainClass::Grass {
                    grass += 1;
                }
            }
        }
        let share = grass * 100 / samples;
        assert!(
            (20..=40).contains(&share),
            "grass share {share}% is not near 30%"
        );
        // Within one lattice square everything agrees.
        assert_eq!(
            background(&r, &SurfaceRef::Ground, 10, 10),
            background(&r, &SurfaceRef::Ground, 790, 790)
        );
    }

    #[test]
    fn water_takes_low_ground() {
        let mut r = region(200, Vec::new());
        r.background.water_cm = Some(0);
        let mut water = 0_u32;
        for lx in 0..50_i64 {
            let g = background(&r, &SurfaceRef::Ground, lx * 800, 0);
            if g.class == TerrainClass::Water {
                water += 1;
                assert!(g.elev_cm < 0);
            }
        }
        assert!(
            water > 5,
            "some of fifty samples should be below the water line"
        );
    }
}
