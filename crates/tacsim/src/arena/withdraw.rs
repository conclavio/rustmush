//! **A unit leaving a running field at its carrier's ramp** (doc 32 cut CF.5; Q-CF-18).
//!
//! The kernel had no way for a unit to leave a running arena: a unit stayed until the end, and
//! its check-in rode [`crate::arena_task::ArenaOut::Ended`]. Recovery as exit needs one. The IC
//! field host sends [`crate::arena_task::ArenaMsg::Withdraw`] with the ramp's site position; at
//! the next pulse's head, before the step, [`Arena::withdraw`] decides it on the arena's own
//! facts — the unit's live pose and its live damage, which only the arena holds while it runs —
//! and either refuses with a [`WithdrawRefusal`] or takes the unit out and answers its check-in
//! row, exactly the row the end would have carried ([`Arena::returned_units`]).
//!
//! **The rule** (recorded in doc 32 §8): the unit stands **beside the ramp** — its cell within
//! one cell of the ramp's, Chebyshev, the surface recover's own rule (doc 20 SM.3b) and the DT
//! recover's (`at_ramp`) — and it can **return under its own power**: no wreck, motion not
//! disabled, not disabled, and its anatomy holds power and control and, where it has a mobility
//! profile, locomotion (the DT recover's own `recover_unit` test, read here on the live cells so
//! the recover the host submits next cannot refuse what the kernel let go).
//!
//! **What leaves with it.** The unit's row, its seats, its mind and its runner, its motor row,
//! its queued orders and card changes, its jamming fold; the spatial index is rebuilt. Every
//! other reader already treats a unit "gone from the match" as gone — the frame drops a contact
//! row whose target left, an order watch on it lapses, a catch-up names neither — so the
//! sensors lose it at their next pass, which is what a hull driving up a ramp looks like.
//! Nothing is journalled: like a leader's mark, the exit is the host's to record (its check-in
//! and the site record), and the black box keeps the pulses either side of it.

use mush_core::ObjId;

use crate::arena::damage::UnitReturn;
use crate::arena::state::Arena;

/// **Why the arena kept a unit the host asked to withdraw.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WithdrawRefusal {
    /// The arena holds no such unit (it left already, or never came).
    NotHere,
    /// The unit cannot return under its own power: a wreck, motion disabled, disabled, or its
    /// anatomy short of power, control or locomotion.
    NotUnderOwnPower,
    /// The unit does not stand within one cell of the ramp.
    NotAtRamp,
}

impl WithdrawRefusal {
    /// The refusal's line to whoever asked.
    #[must_use]
    pub const fn sentence(self) -> &'static str {
        match self {
            WithdrawRefusal::NotHere => "That unit is not in the field.",
            WithdrawRefusal::NotUnderOwnPower => {
                "That unit cannot come in under its own power; it stays in the field."
            }
            WithdrawRefusal::NotAtRamp => {
                "That unit is not beside the ramp; drive it there first, then recover it."
            }
        }
    }
}

/// Cells apart, Chebyshev.
fn cells_apart(a: (u16, u16), b: (u16, u16)) -> u16 {
    a.0.abs_diff(b.0).max(a.1.abs_diff(b.1))
}

impl Arena {
    /// **Take `unit` out of the field at the ramp standing at `ramp` (cm)**, answering its
    /// check-in row (this module's head).
    ///
    /// # Errors
    ///
    /// [`WithdrawRefusal`], and the arena is unchanged.
    pub fn withdraw(
        &mut self,
        unit: ObjId,
        ramp: (i64, i64),
    ) -> Result<UnitReturn, WithdrawRefusal> {
        let state = self.unit(unit).ok_or(WithdrawRefusal::NotHere)?;
        let terrain = &self.definition.terrain;
        let beside = match (
            terrain.cell_of(&state.pose),
            terrain.cell_at(ramp.0, ramp.1),
        ) {
            (Some(here), Some(at)) => cells_apart(here, at) <= 1,
            _ => false,
        };
        let own_power = !state.limits.wreck
            && !state.limits.motion_disabled
            && !state.disabled
            && crate::field_ops::assess_tacsim_unit(&state.unit, None).is_ok_and(|c| {
                c.has_power && c.has_control && (c.can_move || state.unit.mobility().is_none())
            });
        if !own_power {
            return Err(WithdrawRefusal::NotUnderOwnPower);
        }
        if !beside {
            return Err(WithdrawRefusal::NotAtRamp);
        }
        let back = self
            .returned_units()
            .into_iter()
            .find(|row| row.unit == unit)
            .ok_or(WithdrawRefusal::NotHere)?;
        self.remove_unit(unit);
        Ok(back)
    }

    /// **Drop `unit` from the arena** with no check: its row and everything the arena keeps for
    /// it alone (this module's head). A boot resuming a field from a snapshot taken before a
    /// unit's exit uses it too, since that unit has already been checked in.
    pub fn remove_unit(&mut self, unit: ObjId) {
        self.units.retain(|state| state.id != unit);
        self.pilots.retain(|pilot| pilot.unit != unit);
        let _ = self.motors.take(unit);
        self.minds.remove(&unit);
        self.runners.remove(&unit);
        self.profiles.remove(&unit);
        self.pending.remove(&unit);
        self.pending_cards.remove(&unit);
        self.awaiting.remove(&unit);
        self.jam_fold.remove(&unit);
        let terrain = std::sync::Arc::clone(&self.definition);
        self.index.rebuild(&terrain.terrain, &self.units);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::{PartyId, Power};
    use crate::arena::test_support::{arena_of, line_tank, pose};

    const NEAR: ObjId = ObjId(4);
    const FAR: ObjId = ObjId(9);

    fn field() -> Arena {
        let mut arena = arena_of(vec![
            line_tank(NEAR, PartyId::RED, pose(1_500, 1_500)),
            line_tank(FAR, PartyId::BLUE, pose(5_500, 5_500)),
        ]);
        for id in [NEAR, FAR] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        arena
    }

    #[test]
    fn a_unit_beside_the_ramp_leaves_and_the_field_carries_on() {
        let mut arena = field();
        // The ramp in the next cell: within one cell, Chebyshev.
        let back = arena.withdraw(NEAR, (2_500, 2_500)).expect("it leaves");
        assert_eq!(back.unit, NEAR);
        assert_eq!((back.pose.x_cm, back.pose.y_cm), (1_500, 1_500));
        assert!(arena.unit(NEAR).is_none(), "gone from the arena");
        assert!(arena.pilots.iter().all(|pilot| pilot.unit != NEAR));
        assert_eq!(
            arena.withdraw(NEAR, (2_500, 2_500)).err(),
            Some(WithdrawRefusal::NotHere)
        );
        // The rest of the field steps on.
        for wall in (100..=2_000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        assert!(arena.unit(FAR).is_some());
        assert!(arena.ended.is_none(), "the field carries on without it");
    }

    #[test]
    fn a_unit_away_from_the_ramp_or_unable_to_move_stays() {
        let mut arena = field();
        assert_eq!(
            arena.withdraw(FAR, (1_500, 1_500)).err(),
            Some(WithdrawRefusal::NotAtRamp)
        );
        if let Some(state) = arena.unit_mut(NEAR) {
            state.limits.motion_disabled = true;
        }
        assert_eq!(
            arena.withdraw(NEAR, (1_500, 1_500)).err(),
            Some(WithdrawRefusal::NotUnderOwnPower)
        );
        if let Some(state) = arena.unit_mut(NEAR) {
            state.limits.motion_disabled = false;
            state.limits.wreck = true;
        }
        assert_eq!(
            arena.withdraw(NEAR, (1_500, 1_500)).err(),
            Some(WithdrawRefusal::NotUnderOwnPower)
        );
        assert!(arena.unit(NEAR).is_some() && arena.unit(FAR).is_some());
    }
}
