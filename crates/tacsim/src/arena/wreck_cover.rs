//! **A wreck as cover** (doc 36 cut GC.2; Q-36-7, doc 32 §17's G-8).
//!
//! A wreck occupies its cell as an obstacle to movement (doc 16 §3.4), and until this cut it
//! blocked no line: the fire trace and the sensor trace read the terrain alone. Now **a wreck's
//! cell takes the wall class's cover and opacity in both traces while the wreck stands in it** —
//! the class a site record's objects are laid as (`crate::field::ground`'s `OBJECT_CLASS`, doc 32
//! cut CF.4) — so a second field uses the wreck as cover, and so does the field that made it.
//!
//! Two readers, one rule:
//!
//! * **the fire trace** (`Arena::trace_eye_to`, `damage.rs`) walks `WithWrecks`, the terrain
//!   with every wreck's cell read as [`WRECK_CLASS`] — so the shot, the impact's re-trace and the
//!   cockpit's cover advice all find the wreck in the line;
//! * **the sensor trace** ([`crate::arena::Arena::sight`]) keeps the terrain's horizon index for
//!   the terrain, then asks [`Arena::wreck_on_line`] whether a wreck stands on the line's interior.
//!   An opaque cell blocks whatever attenuation came before it, so a terrain answer that is not
//!   already blocked and a wreck on the line is blocked, exactly as the walk over `WithWrecks`
//!   would say.
//!
//! **An endpoint is never cover for its own line.** The shooter's cell and the target's cell are
//! left as the terrain has them, so a shot at a wreck still reads the ground the wreck stands on,
//! and a wreck's own sensors (it has none) or a unit beside it are not hidden by it.
//!
//! **Its cost.** Nothing at all on a field with no wreck: [`Arena::wreck_cells`] is one pass over
//! the units, and both readers skip the overlay when it is empty. With wrecks, the sensor reader
//! culls each wreck by the line's bounding box before it walks a line's cells, and the fire reader
//! runs once a shot. Wrecks are few (a field holds at most `[tacsim.field].max_units` hulls).
//!
//! **It is a kernel rule**: a wreck now changes which contacts are held and which rounds land, so
//! it reaches every hash the practice pins hold that ever stood a wreck between two units. Those
//! were reblessed once, with the rest of GC.2 (doc 36 §7, Q-36-56).

use mush_atlas::TerrainClass;
use mush_atlas::trace::{Bresenham, CellGrid, canonical_ends};

use crate::arena::state::{Arena, ArenaTerrain};

/// **The class a wreck's cell reads as in the traces**: a wall — full cover, opaque. The same
/// class the field's ground lays a site record's objects as (doc 32 cut CF.4).
pub const WRECK_CLASS: TerrainClass = TerrainClass::Wall;

/// **The terrain with every wreck's cell read as [`WRECK_CLASS`]** — the grid the fire trace
/// walks. `wrecks` holds no endpoint of the line being walked.
pub(crate) struct WithWrecks<'a> {
    /// The arena's ground.
    pub terrain: &'a ArenaTerrain,
    /// The wrecks' cells, the line's two ends left out.
    pub wrecks: &'a [(u16, u16)],
}

impl CellGrid for WithWrecks<'_> {
    fn cell_at(&self, x: u16, y: u16) -> Option<(TerrainClass, i32)> {
        let (class, elev_cm) = CellGrid::cell_at(self.terrain, x, y)?;
        let class = if self.wrecks.contains(&(x, y)) {
            WRECK_CLASS
        } else {
            class
        };
        Some((class, elev_cm))
    }
}

impl Arena {
    /// **The cells wrecks stand in**, in unit order, but for `except` — the two ends of the line
    /// a caller is about to trace. Empty on a field with no wreck.
    #[must_use]
    pub fn wreck_cells(&self, except: [(u16, u16); 2]) -> Vec<(u16, u16)> {
        self.units
            .iter()
            .filter(|unit| unit.limits.wreck)
            .filter_map(|unit| self.definition.terrain.cell_of(&unit.pose))
            .filter(|cell| !except.contains(cell))
            .collect()
    }

    /// **The first wreck cell on the interior of the line from `a` to `b`**, walked from the
    /// line's canonical end as every arena line is (16c `B-56`), or `None`. A wreck in either
    /// end's own cell is not on the line.
    #[must_use]
    pub fn wreck_on_line(&self, a: (u16, u16), b: (u16, u16)) -> Option<(u16, u16)> {
        let (lo, hi) = canonical_ends(a, b);
        let (x0, x1) = (lo.0.min(hi.0), lo.0.max(hi.0));
        let (y0, y1) = (lo.1.min(hi.1), lo.1.max(hi.1));
        let near: Vec<(u16, u16)> = self
            .wreck_cells([a, b])
            .into_iter()
            .filter(|&(x, y)| (x0..=x1).contains(&x) && (y0..=y1).contains(&y))
            .collect();
        if near.is_empty() {
            return None;
        }
        Bresenham::new(lo, hi).find(|cell| *cell != lo && *cell != hi && near.contains(cell))
    }
}

#[cfg(test)]
mod tests {
    use mush_atlas::Cover;
    use mush_atlas::trace::{Blocker, Eye, Sight};
    use mush_core::ObjId;

    use super::WRECK_CLASS;
    use crate::arena::perception::PerceptionCounters;
    use crate::arena::state::{Arena, Classification, PartyId, SeenPose, ShadowContact};
    use crate::arena::test_support::{arena_of, line_tank, pose, tank};

    const RED: ObjId = ObjId(4);
    const HULK: ObjId = ObjId(6);
    const BLUE: ObjId = ObjId(9);

    /// Red at cell (1, 1), blue at (5, 1), a third tank between them at (3, 1).
    fn row() -> Arena {
        arena_of(vec![
            line_tank(RED, PartyId::RED, pose(1_500, 1_500)),
            line_tank(HULK, PartyId::BLUE, pose(3_500, 1_500)),
            line_tank(BLUE, PartyId::BLUE, pose(5_500, 1_500)),
        ])
    }

    fn eye(cell: (u16, u16)) -> Eye {
        Eye {
            cell,
            height_cm: 150,
        }
    }

    fn wreck(arena: &mut Arena, unit: ObjId) {
        if let Some(state) = arena.unit_mut(unit) {
            state.limits.wreck = true;
            state.limits.motion_disabled = true;
        }
    }

    /// **Doc 36 cut GC.2's gate** (Q-36-7, doc 32 §17's G-8): a wreck standing
    /// between two units hides each from the other — in the sensor trace
    /// (`Arena::sight`, which detection and the mesh read) and in the fire trace
    /// (the shot, the impact and the cover advice) — and the same hull standing
    /// there alive hides nothing, since a live unit's cell is no cover.
    #[test]
    fn gc2_a_wreck_hides_what_stands_behind_it() {
        let mut arena = row();
        let mut counters = PerceptionCounters::default();
        assert_eq!(
            arena.sight(eye((1, 1)), eye((5, 1)), &mut counters),
            Sight::Clear,
            "a live hull in the line hides nothing"
        );
        assert_eq!(arena.fire_line(RED, BLUE), (Sight::Clear, Cover::None));
        wreck(&mut arena, HULK);
        let behind = Sight::BlockedBy(Blocker::Terrain {
            at: (3, 1),
            class: WRECK_CLASS,
        });
        assert_eq!(
            arena.sight(eye((1, 1)), eye((5, 1)), &mut counters),
            behind,
            "the wreck blocks the sensor line"
        );
        assert_eq!(
            arena.sight(eye((5, 1)), eye((1, 1)), &mut counters),
            behind,
            "from either end"
        );
        assert_eq!(
            arena.fire_line(RED, BLUE).0,
            behind,
            "the wreck blocks the fire line"
        );
        assert_eq!(arena.wreck_on_line((1, 1), (5, 1)), Some((3, 1)));
        // An end is never cover for its own line: a shot at the wreck itself
        // reads the ground it stands on, and a line past it is open.
        assert_eq!(arena.fire_line(RED, HULK), (Sight::Clear, Cover::None));
        assert_eq!(arena.wreck_on_line((1, 1), (3, 1)), None);
        assert_eq!(
            arena.wreck_on_line((1, 2), (5, 2)),
            None,
            "a line beside it"
        );
        assert_eq!(
            arena.sight(eye((1, 2)), eye((5, 2)), &mut counters),
            Sight::Clear
        );
        assert!(arena.wreck_cells([(0, 0), (0, 0)]).contains(&(3, 1)));
    }

    /// **Doc 36 cut GC.2** (Q-36-6, the tempo half): an armed tank holding a
    /// Hostile contact inside its reach keeps the field `Engaged` only while the
    /// contact's hull was ever armed or has fired.
    #[test]
    fn gc2_an_unarmed_target_keeps_nobody_engaged() {
        let mut arena = arena_of(vec![
            line_tank(RED, PartyId::RED, pose(1_500, 1_500)),
            tank(BLUE, PartyId::BLUE, pose(2_500, 1_500)),
        ]);
        let id = crate::arena::rng::draw_contact_id(arena.seed, RED, BLUE);
        let seen = arena.unit(BLUE).map(SeenPose::of).unwrap_or_default();
        arena.contacts.insert(
            RED,
            id,
            ShadowContact::sighted(
                BLUE,
                Classification::Classified {
                    party: PartyId::BLUE,
                },
                0,
                seen,
            ),
        );
        assert!(
            arena
                .unit(BLUE)
                .is_some_and(|state| !state.armed_at_checkout()),
            "the fixture tank carries no gun"
        );
        assert!(
            !arena.hostile_in_weapons_range(),
            "a never-armed hull held Hostile in reach is not a fight"
        );
        // Once it has fired, it is.
        if let Some(state) = arena.unit_mut(BLUE) {
            state.shots_fired = 1;
        }
        assert!(arena.hostile_in_weapons_range());
        // And an armed hull in the same cell always was.
        if let Some(state) = arena.unit_mut(BLUE) {
            state.shots_fired = 0;
            state.ammo.insert(smol_str::SmolStr::new("cannon"), 0);
        }
        assert!(arena.hostile_in_weapons_range());
    }
}
