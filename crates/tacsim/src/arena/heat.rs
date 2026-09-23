//! **Heat** (16b cut X.1; doc 16 §9 item 1: *a `heat` cell and dissipation
//! appear on the unit*, §3.5 "Weapon definition"'s `heat_per_shot`).
//!
//! Doc 16 §3.5 gives every weapon a `heat_per_shot` and says the cell *exists for
//! lasers*; this is the rest of that sentence. A shot of a weapon that makes heat
//! raises its shooter's `heat` cell, arena time lowers it, and a platform at its
//! ceiling refuses to fire until it has cooled.
//!
//! # The truth is one M32 cell, and the ceiling is that cell's own maximum
//!
//! `heat` is a section cell on the unit's body, laid down by
//! [`crate::definitions::unit::bind`] and **only** when the content declares a
//! ceiling. That makes three things true at once, and they are the whole design:
//!
//! 1. the check-in hands it back with the armour, because it is one of the cells
//!    (doc 16 §3.1's `CustodyIn { units[{cells, ..}] }`);
//! 2. there is no second number to keep in step with the ceiling — `heat_max` *is*
//!    `cell.max`, so every op this module builds is in bounds by construction and
//!    M32's refuse-never-clamp is a guard rather than a path (the same rule
//!    [`crate::arena::damage`]'s armour spend follows);
//! 3. a unit whose content predates the laser carries no such cell, so its bound
//!    `UnitSpec` is byte-identical to what it was — no new definition version and
//!    no migration for a persisted tank (16b §8 cut X's rule).
//!
//! # Why the fall is carried, not truncated
//!
//! The dissipation is a **rate over arena time** and the state is an integer, and
//! doc 16 makes the model rate-independent (D-16-2): the same match run at 5,000,
//! 1,000 and 500 ms must produce the same numbers. Written literally as
//! `per_s × dt_ms / 1000` it would not, because `tempo.substep_max_ms` is 250 and
//! that expression is **zero** for every rate under four a second — a tank at
//! `Engaged` would never cool at all, and one at `Quiet` would cool in steps that
//! depended on where the substep boundaries happened to fall.
//!
//! So the remainder is carried, in thousandths, on
//! [`HeatState::carry_milli`](crate::arena::state::HeatState::carry_milli): each
//! substep adds `per_s × dt_ms` milli-heat to the carry, spends the whole units out
//! of it, and keeps the rest. The total shed over any interval is then exactly
//! `per_s × ms / 1000` however the interval is chopped, which is the property the
//! cross-tempo test asserts of everything else in the model.
//!
//! # What it costs a match with no lasers in it
//!
//! One `Option` test per unit per substep. A unit with no ceiling has no
//! [`HeatState`](crate::arena::state::HeatState), so the harness's field of 300
//! stock tanks reads no cell, builds no plan and calls `apply` no more often than
//! it did before this cut.

use mush_core::ObjId;
use mush_tactics::{UnitMutationOp, UnitMutationPlan};
use smol_str::SmolStr;

use crate::arena::state::{Arena, UnitState};
use crate::definitions::unit::HEAT_CELL;

/// Thousandths in one whole unit of heat — the scale
/// [`HeatState::carry_milli`](crate::arena::state::HeatState::carry_milli) is
/// measured in.
pub const MILLI: u32 = 1_000;

/// The cause a cooling plan is journalled under (M32's semantic journal tag).
pub const COOL_CAUSE: &str = "heat:cool";

/// The prefix of the cause a shot's heat is journalled under; the shot's own
/// sequence follows it, so a line in M32's journal names the shot that made it —
/// as `shot:<seq>` names the round that did the damage.
pub const SHOT_CAUSE: &str = "heat:shot";

impl UnitState {
    /// **How much heat it is carrying** (16b cut X.1), or `None` for a unit that
    /// carries none at all.
    #[must_use]
    pub fn heat_now(&self) -> Option<u32> {
        let heat = self.heat.as_ref()?;
        let addr = heat.addr(HEAT_CELL)?;
        let cell = self.unit.cell(&addr)?;
        u32::try_from(cell.current.max(0)).ok()
    }

    /// **The ceiling it refuses to fire at**, or `None` for a unit with no heat.
    #[must_use]
    pub fn heat_max(&self) -> Option<u32> {
        self.heat.as_ref().map(|heat| heat.max)
    }

    /// **Whether `FIRE` refuses for heat** (doc 16 §9 item 1: *a shot refused
    /// above a heat ceiling*).
    ///
    /// At the ceiling and not merely past it: the cell cannot hold more than its
    /// maximum, so a shot that would carry a unit past it leaves it exactly at it
    /// and the *next* shot is the one refused. A unit with no heat is never
    /// refused for it, which is every stock tank.
    #[must_use]
    pub fn overheated(&self) -> bool {
        match (self.heat_now(), self.heat_max()) {
            (Some(now), Some(max)) => now >= max,
            _ => false,
        }
    }
}

impl Arena {
    /// **What one shot of `weapon` adds to `unit`** (doc 16 §3.5 "Weapon
    /// definition": `heat_per_shot`), applied as one plan through M32.
    ///
    /// Called by [`Arena::resolve_fire`](crate::arena::state::Arena) from the
    /// accepted shot, after the round and the cycle are spent, so a refused shot
    /// costs no heat. The value is capped at the cell's own maximum before the op
    /// is built — the plan is in bounds by construction, exactly as the armour
    /// spend is — and a weapon that makes no heat builds no plan at all.
    ///
    /// A plan M32 refuses leaves the unit exactly as it was and the shot stands:
    /// the round has left the barrel either way, and a shot unfired because a
    /// journal entry would not take is the wrong failure.
    ///
    /// `t_ms` is the shot's own `at_ms` (16b cut M.2, 16c `T-02`): the pulse's end
    /// for a pilot's shot, the tick's timestamp for a machine's, so the heat and
    /// the `fired` line belong to one arena millisecond.
    pub fn raise_heat(&mut self, unit: ObjId, weapon: &str, shot_seq: u64, t_ms: i64) {
        let Some(state) = self.unit(unit) else {
            return;
        };
        let Some(heat) = state.heat.as_ref() else {
            return;
        };
        let per_shot = heat.per_shot(weapon);
        if per_shot == 0 {
            return;
        }
        let (Some(addr), Some(now)) = (heat.addr(HEAT_CELL), state.heat_now()) else {
            return;
        };
        let value = i64::from(now.saturating_add(per_shot).min(heat.max));
        if value == i64::from(now) {
            return;
        }
        let cause = SmolStr::new(format!("{SHOT_CAUSE}:{shot_seq}"));
        apply_heat(self, unit, addr, value, cause, t_ms);
    }

    /// **The fall** (doc 16 §9 item 1: *dissipation*), over one substep's worth of
    /// arena time.
    ///
    /// In unit-id order, like everything else a substep does, and only for the
    /// units that have a [`HeatState`](crate::arena::state::HeatState) — which is
    /// the units whose content gave them one. See this module's head for why the
    /// remainder is carried rather than truncated.
    pub fn dissipate_heat(&mut self, dt_ms: u32, t_ms: i64) {
        if dt_ms == 0 {
            return;
        }
        let ids: Vec<ObjId> = self
            .units
            .iter()
            .filter(|state| state.heat.is_some())
            .map(|state| state.id)
            .collect();
        for id in ids {
            self.cool_one(id, dt_ms, t_ms);
        }
    }

    /// One unit's fall: the carry, the whole units out of it, and the plan.
    fn cool_one(&mut self, unit: ObjId, dt_ms: u32, t_ms: i64) {
        let Some(state) = self.unit(unit) else {
            return;
        };
        let Some(heat) = state.heat.as_ref() else {
            return;
        };
        let now = state.heat_now().unwrap_or(0);
        let addr = heat.addr(HEAT_CELL);
        // Nothing to shed, and nothing to remember: a cold unit's carry is reset
        // so that heat taken later falls from a known start rather than from
        // whatever the last cooling left over.
        if now == 0 || heat.per_s == 0 {
            if let Some(state) = self.unit_mut(unit)
                && let Some(heat) = state.heat.as_mut()
            {
                heat.carry_milli = 0;
            }
            return;
        }
        let budget = u64::from(heat.per_s)
            .saturating_mul(u64::from(dt_ms))
            .saturating_add(u64::from(heat.carry_milli));
        let whole = u32::try_from(budget / u64::from(MILLI)).unwrap_or(u32::MAX);
        let carry = u32::try_from(budget % u64::from(MILLI)).unwrap_or(0);
        let left = now.saturating_sub(whole);
        if let Some(state) = self.unit_mut(unit)
            && let Some(heat) = state.heat.as_mut()
        {
            // The carry belongs to the arena time that has passed whether or not
            // the heat had that much left in it, so it is kept even when the unit
            // hits zero part-way through the substep.
            heat.carry_milli = carry;
        }
        if left == now {
            return;
        }
        let Some(addr) = addr else {
            return;
        };
        apply_heat(
            self,
            unit,
            addr,
            i64::from(left),
            SmolStr::new_static(COOL_CAUSE),
            t_ms,
        );
    }
}

/// Set one unit's heat cell through M32, atomically and with a cause.
///
/// The one place a `heat` op is built, so the rise and the fall cannot disagree
/// about which cell they are writing or how a refusal is handled.
fn apply_heat(
    arena: &mut Arena,
    unit: ObjId,
    addr: mush_tactics::CellAddr,
    value: i64,
    cause: SmolStr,
    t_ms: i64,
) {
    let Ok(plan) = UnitMutationPlan::new(
        vec![UnitMutationOp::SetCell { addr, value }],
        None,
        Some(cause),
    ) else {
        return;
    };
    let applied = arena
        .unit(unit)
        .map(|state| mush_tactics::apply(&state.unit, &plan, t_ms));
    // **16b cut Q.2**: through `Arena::set_unit`, the one assignment of a unit in
    // non-test code, so the band cache cannot go stale behind a heat cycle. A
    // heat cell is not armour and moves no band; going through the one writer
    // anyway is what makes "one writer" a property and not a habit.
    //
    // **16b cut M.3** (16c `T-28`): and the writer is told so. The plan's own
    // ops decide it (`Arena::plan_touches_bands`: a section cell named `heat`
    // is not one the bands read), so the cache is left as it was rather than
    // recomputed on every substep's cooling of every hot unit.
    if let Some(Ok((post, _))) = applied {
        let touched = Arena::plan_touches_bands(&plan);
        arena.set_unit(unit, post, touched);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::Team;
    use crate::arena::test_support::{arena_of, laser_tank, pose};

    const RED: ObjId = ObjId(4);

    #[test]
    fn a_unit_with_no_ceiling_has_no_heat_state_at_all() {
        let arena = arena_of(vec![crate::arena::test_support::line_tank(
            RED,
            Team::Red,
            pose(1_500, 1_500),
        )]);
        let state = arena.unit(RED).unwrap_or_else(|| panic!("the tank"));
        assert!(state.heat.is_none());
        assert_eq!(state.heat_now(), None);
        assert!(!state.overheated());
    }

    #[test]
    fn the_fall_is_the_same_total_however_the_interval_is_chopped() {
        // Three heat a second against 250 ms substeps: the literal
        // `per_s × dt / 1000` is zero every time, and the carry is what makes the
        // second's worth arrive anyway.
        let mut fine = arena_of(vec![laser_tank(RED, Team::Red, pose(1_500, 1_500))]);
        let mut coarse = arena_of(vec![laser_tank(RED, Team::Red, pose(1_500, 1_500))]);
        for arena in [&mut fine, &mut coarse] {
            set_heat(arena, RED, 30);
        }
        for _ in 0..8 {
            fine.dissipate_heat(250, 0);
        }
        coarse.dissipate_heat(2_000, 0);
        let per_s = fine
            .unit(RED)
            .and_then(|state| state.heat.as_ref())
            .map_or(0, |heat| heat.per_s);
        assert_eq!(
            fine.unit(RED).and_then(UnitState::heat_now),
            Some(30 - per_s * 2),
            "two seconds of arena time at {per_s} a second"
        );
        assert_eq!(
            fine.unit(RED).and_then(UnitState::heat_now),
            coarse.unit(RED).and_then(UnitState::heat_now),
            "eight 250 ms substeps and one 2,000 ms one are the same two seconds"
        );
    }

    #[test]
    fn heat_floors_at_zero_and_the_carry_is_reset_with_it() {
        let mut arena = arena_of(vec![laser_tank(RED, Team::Red, pose(1_500, 1_500))]);
        let carry = |arena: &Arena| {
            arena
                .unit(RED)
                .and_then(|state| state.heat.as_ref())
                .map(|heat| heat.carry_milli)
        };
        set_heat(&mut arena, RED, 2);
        // 700 ms at three a second is 2.1 heat against the 2 it had: the heat
        // floors rather than going under, and the tenth is a remainder.
        arena.dissipate_heat(700, 0);
        assert_eq!(arena.unit(RED).and_then(UnitState::heat_now), Some(0));
        assert_eq!(carry(&arena), Some(100), "the arena time that passed");
        // And the next substep clears it, so heat taken later falls from a known
        // start rather than from whatever the last cooling left over.
        arena.dissipate_heat(250, 0);
        assert_eq!(arena.unit(RED).and_then(UnitState::heat_now), Some(0));
        assert_eq!(carry(&arena), Some(0));
    }

    /// **16b cut M.3** (16c `T-28`): a heat plan — the rise and the fall — goes
    /// through the one writer and leaves the band cache byte-identical, because
    /// the heat cell is not a cell the bands read and the writer is told so.
    #[test]
    fn m3_a_heat_plan_leaves_the_band_cache_as_it_was() {
        use crate::arena::condition::{Condition, ConditionBand};

        let mut arena = arena_of(vec![laser_tank(RED, Team::Red, pose(1_500, 1_500))]);
        // A cache no intact tank has, so a recompute would be visible.
        let marked = Condition::default().with_band(0, ConditionBand::Destroyed);
        if let Some(state) = arena.unit_mut(RED) {
            state.condition = marked;
        }
        set_heat(&mut arena, RED, 30);
        arena.dissipate_heat(1_000, 0);
        assert!(
            arena
                .unit(RED)
                .and_then(UnitState::heat_now)
                .is_some_and(|heat| heat < 30),
            "the fall ran"
        );
        assert_eq!(
            arena.unit(RED).map(|state| state.condition),
            Some(marked),
            "a heat plan recomputed the bands"
        );
    }

    /// Put `value` in the unit's heat cell, through M32 as everything else does.
    fn set_heat(arena: &mut Arena, unit: ObjId, value: u32) {
        let addr = arena
            .unit(unit)
            .and_then(|state| state.heat.as_ref())
            .and_then(|heat| heat.addr(HEAT_CELL))
            .unwrap_or_else(|| panic!("the heat cell"));
        apply_heat(
            arena,
            unit,
            addr,
            i64::from(value),
            SmolStr::new_static("test"),
            0,
        );
        assert_eq!(arena.unit(unit).and_then(UnitState::heat_now), Some(value));
    }
}
