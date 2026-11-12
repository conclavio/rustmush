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
//! **A carrier lifting off its pad** (doc 32 F-6, ruled 2026-10-31; built in cut CF.7) asks with
//! no ramp. After the pilot's leave-behind confirmation the carrier is removed from the field
//! outright and lifts: it stands on its pad, about four cells from its own ramp, and a lift is
//! not a drive, so neither test below applies to it. What a hit does to lift is F-3's.
//!
//! **The rule** (recorded in doc 32 §8): the unit stands **beside the ramp** — its cell within
//! one cell of the ramp's, Chebyshev, the surface recover's own rule (doc 20 SM.3b) and the DT
//! recover's (`at_ramp`) — and it can **return under its own power**: no wreck, motion not
//! disabled, not disabled, and its anatomy holds power and control and, where it has a mobility
//! profile, locomotion (the DT recover's own `recover_unit` test, read here on the live cells so
//! the recover the host submits next cannot refuse what the kernel let go).
//!
//! **A carrier lost** (doc 40, the ramp exit's boarding window; the owner's ruling, 2026-11-04,
//! "back in the field"): a withdrawal counts only once the unit is aboard, so a raider's hull,
//! whose host names the carrier it boards ([`Arena::withdraw_to`]), is kept while that carrier is
//! a wreck or no longer in the field — [`WithdrawRefusal::CarrierLost`], decided on the arena's
//! own live facts at the same pulse head as the release, so no wreck can land between the two.
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
    /// The carrier it would board is a wreck or no longer in the field (this module's head):
    /// there is nothing to go aboard, so it stays in the field.
    CarrierLost,
    /// **The ask was recalled** (doc 44 cut MW.7; Q-44-37): the host asked the unit out for a
    /// player who was away, and that player took the controls back before the pulse that
    /// would have let it go. Answered by the task, never by [`Arena::withdraw`].
    Recalled,
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
            WithdrawRefusal::CarrierLost => {
                "That unit's carrier is lost; there is nothing to go aboard, so it stays in the field."
            }
            WithdrawRefusal::Recalled => {
                "That unit's pilot is back at its controls; it stays in the field."
            }
        }
    }
}

/// **Whether `state` can return under its own power** (this module's head): no wreck, motion
/// not disabled, not disabled, and its anatomy holds power and control and, where it has a
/// mobility profile, locomotion.
fn under_own_power(state: &crate::arena::state::UnitState) -> bool {
    !state.limits.wreck
        && !state.limits.motion_disabled
        && !state.disabled
        && crate::field_ops::assess_tacsim_unit(&state.unit, None).is_ok_and(|c| {
            c.has_power && c.has_control && (c.can_move || state.unit.mobility().is_none())
        })
}

/// Cells apart, Chebyshev.
fn cells_apart(a: (u16, u16), b: (u16, u16)) -> u16 {
    a.0.abs_diff(b.0).max(a.1.abs_diff(b.1))
}

impl Arena {
    /// **Take `unit` out of the field at the ramp standing at `ramp` (cm)**, answering its
    /// check-in row (this module's head). `None` is a carrier lifting off its pad (doc 32 F-6):
    /// no ramp and no drive, so only [`WithdrawRefusal::NotHere`] refuses it.
    ///
    /// # Errors
    ///
    /// [`WithdrawRefusal`], and the arena is unchanged.
    pub fn withdraw(
        &mut self,
        unit: ObjId,
        ramp: Option<(i64, i64)>,
    ) -> Result<UnitReturn, WithdrawRefusal> {
        self.withdraw_to(unit, ramp, None)
    }

    /// **[`Arena::withdraw`], boarding `carrier`** (this module's head, "a carrier lost"): with a
    /// ramp and a carrier named, the unit is kept with [`WithdrawRefusal::CarrierLost`] while
    /// that carrier is a wreck or not in the field, wherever the unit stands.
    ///
    /// # Errors
    ///
    /// [`WithdrawRefusal`], and the arena is unchanged.
    pub fn withdraw_to(
        &mut self,
        unit: ObjId,
        ramp: Option<(i64, i64)>,
        carrier: Option<ObjId>,
    ) -> Result<UnitReturn, WithdrawRefusal> {
        let state = self.unit(unit).ok_or(WithdrawRefusal::NotHere)?;
        let Some(ramp) = ramp else {
            return self.take_out(unit);
        };
        if let Some(carrier) = carrier
            && self
                .unit(carrier)
                .is_none_or(|carrier| carrier.limits.wreck)
        {
            return Err(WithdrawRefusal::CarrierLost);
        }
        // **Doc 40 cut FR.5** (§4.1 item 5, Q-40-25): a casualty under tow cannot come in under
        // its own power, so it is judged by its tug — the tug in the field, under its own power
        // and beside the ramp — and goes out in tow with its own check-in row. The task applies
        // a casualty's withdrawal before its tug's, so the tug still stands here to be read.
        let judged = match state.coupled_to {
            Some(tug) => self.unit(tug).ok_or(WithdrawRefusal::NotUnderOwnPower)?,
            None => state,
        };
        if !under_own_power(judged) {
            return Err(WithdrawRefusal::NotUnderOwnPower);
        }
        if !self.beside(judged, ramp) {
            return Err(WithdrawRefusal::NotAtRamp);
        }
        self.take_out(unit)
    }

    /// Whether `state` stands within one cell of the ramp at `ramp` (cm), Chebyshev.
    fn beside(&self, state: &crate::arena::state::UnitState, ramp: (i64, i64)) -> bool {
        let terrain = &self.definition.terrain;
        match (
            terrain.cell_of(&state.pose),
            terrain.cell_at(ramp.0, ramp.1),
        ) {
            (Some(here), Some(at)) => cells_apart(here, at) <= 1,
            _ => false,
        }
    }

    /// The unit's check-in row, and the unit out of the arena.
    fn take_out(&mut self, unit: ObjId) -> Result<UnitReturn, WithdrawRefusal> {
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
        // Doc 40 cut FR.5 (§4.1 item 5): a tug gone uncouples its trailer, which stands where it
        // is.
        self.drop_tow(unit);
        // Doc 36 cut GC.1b: the chassis words are kept in the unit table's row order, so a row
        // gone shifts every later one; laid down again, as a join lays them.
        self.chassis = crate::arena::state::ChassisWords::table(&self.units);
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
        let back = arena
            .withdraw(NEAR, Some((2_500, 2_500)))
            .expect("it leaves");
        assert_eq!(back.unit, NEAR);
        assert_eq!((back.pose.x_cm, back.pose.y_cm), (1_500, 1_500));
        assert!(arena.unit(NEAR).is_none(), "gone from the arena");
        assert!(arena.pilots.iter().all(|pilot| pilot.unit != NEAR));
        assert_eq!(
            arena.withdraw(NEAR, Some((2_500, 2_500))).err(),
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
            arena.withdraw(FAR, Some((1_500, 1_500))).err(),
            Some(WithdrawRefusal::NotAtRamp)
        );
        if let Some(state) = arena.unit_mut(NEAR) {
            state.limits.motion_disabled = true;
        }
        assert_eq!(
            arena.withdraw(NEAR, Some((1_500, 1_500))).err(),
            Some(WithdrawRefusal::NotUnderOwnPower)
        );
        if let Some(state) = arena.unit_mut(NEAR) {
            state.limits.motion_disabled = false;
            state.limits.wreck = true;
        }
        assert_eq!(
            arena.withdraw(NEAR, Some((1_500, 1_500))).err(),
            Some(WithdrawRefusal::NotUnderOwnPower)
        );
        assert!(arena.unit(NEAR).is_some() && arena.unit(FAR).is_some());
    }

    #[test]
    fn a_unit_whose_carrier_is_lost_stays() {
        // Doc 40, the ramp exit's boarding window: NEAR boards FAR.
        let mut arena = field();
        if let Some(state) = arena.unit_mut(FAR) {
            state.limits.wreck = true;
        }
        assert_eq!(
            arena
                .withdraw_to(NEAR, Some((2_500, 2_500)), Some(FAR))
                .err(),
            Some(WithdrawRefusal::CarrierLost)
        );
        assert_eq!(
            arena
                .withdraw_to(NEAR, Some((2_500, 2_500)), Some(ObjId(77)))
                .err(),
            Some(WithdrawRefusal::CarrierLost),
            "a carrier not in the field is lost too"
        );
        assert!(arena.unit(NEAR).is_some(), "it stays in the field");
        if let Some(state) = arena.unit_mut(FAR) {
            state.limits.wreck = false;
        }
        let back = arena
            .withdraw_to(NEAR, Some((2_500, 2_500)), Some(FAR))
            .expect("a whole carrier takes it");
        assert_eq!(back.unit, NEAR);
    }

    #[test]
    fn f6_a_lift_needs_no_ramp_and_no_drive() {
        let mut arena = field();
        if let Some(state) = arena.unit_mut(FAR) {
            state.limits.motion_disabled = true;
        }
        let back = arena.withdraw(FAR, None).expect("it lifts");
        assert_eq!(back.unit, FAR);
        assert!(arena.unit(FAR).is_none(), "gone from the arena");
        assert_eq!(
            arena.withdraw(FAR, None).err(),
            Some(WithdrawRefusal::NotHere)
        );
        assert!(arena.unit(NEAR).is_some(), "the unit it leaves stays");
    }
}
