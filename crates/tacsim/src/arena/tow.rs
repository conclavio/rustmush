//! **The coupled tow as a kernel unit** (doc 40 cut FR.5, §4.1 item 5; Q-40-7 and Q-40-25, ruled
//! 2026-11-03 as proposed).
//!
//! A casualty under tow on the surface (`crate::surface::tow`) is admitted to an IC field as a
//! unit of its tug's party, and the host names the pair in [`FieldHost::tows`]. [`Arena::new`]
//! couples each pair through [`Arena::couple`] once the units are in hand:
//!
//! - **One unit a cell.** The surface stacks the casualty on its tug's cell, and the index holds
//!   one unit a cell, so a casualty sharing its tug's cell is unstacked to the nearest free cell
//!   it may stand on, by the join's ring rule (`crate::arena::join`: nearest ring first,
//!   row-major within a ring, within [`TOW_SPREAD_CELLS`]). The host parts them already
//!   (`crate::field::carrier`'s `unstack`); the kernel does it again so a check-out can never
//!   meet `ArenaError::CellOccupied` over a tow.
//! - **A casualty's limits.** `UnitState::coupled_to` names the tug; its motion is disabled and
//!   its speeds are nought, it fires nothing (`Arena::resolve_fire_by` refuses it), and its
//!   sensors are whatever its parts' condition words leave them (`Arena::apply_part_effects`, run
//!   at admission). It is struck through its own sections and facet bands like any unit.
//! - **A trailer.** Each substep its tug crosses into a new cell, right after the tug's
//!   `integrate` and its index move, the casualty is set on the cell the tug left — its centre,
//!   at the ground's elevation, on the tug's heading — and the index moved for it
//!   (`trail`), so the cell it takes is the one the index has just freed and one unit a cell
//!   holds throughout. It never integrates on its own.
//! - **The coupling ends** when the tug releases ([`Arena::release`], a field act:
//!   `crate::arena_task::ArenaMsg::Release`), when the tug's hull is destroyed
//!   (`Arena::destroy_unit`: the casualty stands where it is) or when the tug leaves the field
//!   (`Arena::withdraw`). A casualty never joins mid-field in this cut.
//! - **The coupled withdraw at the ramp** (Q-40-25): a coupled casualty cannot come in under its
//!   own power, so `Arena::withdraw` lets it out when its tug can — the tug under its own power
//!   and beside the ramp — and the task applies a casualty's withdrawal before its tug's.
//!
//! Every practice match carries no tow: `FieldHost::tows` is empty, no unit is coupled, the
//! trailer list the step builds is empty, and nothing here is folded into the hash.
//!
//! [`FieldHost::tows`]: crate::arena::party::FieldHost::tows

use mush_core::ObjId;

use crate::arena::index::SpatialIndex;
use crate::arena::join::ring_cells;
use crate::arena::state::{Arena, ArenaError, ArenaTerrain, UnitState};

/// **How far from its tug, in cells (Chebyshev), a stacked casualty may be stood**: the pad's own
/// spread (`crate::field::carrier::PAD_SPREAD_CELLS`).
pub const TOW_SPREAD_CELLS: u16 = 8;

/// A refusal of a coupling, by name.
fn refused(casualty: ObjId, tug: ObjId, reason: &'static str) -> ArenaError {
    ArenaError::Coupling {
        casualty,
        tug,
        reason,
    }
}

/// **Stand the casualty apart from its tug** when the two share a cell (this module's head): on
/// the nearest free cell to the tug the casualty may stand on, centred, at the ground's elevation.
/// A casualty on a cell of its own is left where it is. `states` is any unit table; it need not
/// be sorted.
///
/// # Errors
///
/// [`ArenaError::Coupling`] for a unit `states` does not hold, or no free ground within
/// [`TOW_SPREAD_CELLS`] of the tug.
pub(crate) fn unstack(
    terrain: &ArenaTerrain,
    states: &mut [UnitState],
    casualty: ObjId,
    tug: ObjId,
) -> Result<(), ArenaError> {
    let tug_cell = states
        .iter()
        .find(|state| state.id == tug)
        .and_then(|state| terrain.cell_of(&state.pose));
    let Some(at) = states.iter().position(|state| state.id == casualty) else {
        return Err(refused(casualty, tug, "no such unit is in the field"));
    };
    let Some(tug_cell) = tug_cell else {
        return Err(refused(casualty, tug, "no such unit is in the field"));
    };
    if states
        .get(at)
        .and_then(|state| terrain.cell_of(&state.pose))
        != Some(tug_cell)
    {
        return Ok(());
    }
    let held: std::collections::BTreeSet<(u16, u16)> = states
        .iter()
        .filter_map(|state| terrain.cell_of(&state.pose))
        .collect();
    let Some(state) = states.get_mut(at) else {
        return Err(refused(casualty, tug, "no such unit is in the field"));
    };
    let row = state.cost_row;
    let free = (1..=TOW_SPREAD_CELLS)
        .flat_map(|ring| ring_cells(tug_cell, ring))
        .find(|cell| !held.contains(cell) && terrain.passable_for(row, cell.0, cell.1))
        .ok_or_else(|| refused(casualty, tug, "no free ground is beside its tug"))?;
    let heading = state.pose.heading_mdeg;
    set_on(terrain, state, free, heading);
    Ok(())
}

/// Put `state` on the centre of `cell`, at the ground's elevation, facing `heading_mdeg`.
fn set_on(terrain: &ArenaTerrain, state: &mut UnitState, cell: (u16, u16), heading_mdeg: u32) {
    let side = i64::from(terrain.cell_cm());
    state.pose.x_cm = i64::from(cell.0) * side + side / 2;
    state.pose.y_cm = i64::from(cell.1) * side + side / 2;
    if let Some(ground) = terrain.get(cell.0, cell.1) {
        state.pose.z_cm = i32::from(ground.elev_cm);
    }
    state.pose.heading_mdeg = heading_mdeg;
}

/// **The trailers of a substep**: `(tug slot, casualty slot)` in the unit table, one row per
/// coupled casualty whose tug the table holds, in casualty order. Empty — and one `Option` test a
/// unit — in every match without a tow (this module's head).
#[must_use]
pub(crate) fn trailers(units: &[UnitState]) -> Vec<(usize, usize)> {
    units
        .iter()
        .enumerate()
        .filter_map(|(slot, state)| {
            let tug = state.coupled_to?;
            let tug_slot = units.binary_search_by_key(&tug, |state| state.id).ok()?;
            Some((tug_slot, slot))
        })
        .collect()
}

/// **The trailer's step** (this module's head): the casualty in `slot` set on `left`, the cell
/// its tug has just left, on the tug's `heading_mdeg`, and the index moved for it. Nothing moves
/// when the index does not answer `left` free (the tug's own move freed it, so it always is).
pub(crate) fn trail(
    units: &mut [UnitState],
    slot: usize,
    left: (u16, u16),
    heading_mdeg: u32,
    terrain: &ArenaTerrain,
    index: &mut SpatialIndex,
) {
    let Some(state) = units.get_mut(slot) else {
        return;
    };
    let Some(from) = terrain.cell_of(&state.pose) else {
        return;
    };
    if from == left || index.occupant(left.0, left.1).is_some() {
        return;
    }
    set_on(terrain, state, left, heading_mdeg);
    state.speed_cms = 0;
    index.move_unit(state.id, from, left);
}

impl Arena {
    /// **Couple `casualty` to `tug`** (this module's head): the casualty unstacked from its tug's
    /// cell if the two share one, `coupled_to` set, and a casualty's limits laid on it — motion
    /// disabled and its speeds nought; no fire while coupled. Its sensors are its parts' (already
    /// derived at admission).
    ///
    /// # Errors
    ///
    /// [`ArenaError::Coupling`], by name: a unit the field does not hold, a unit coupled to
    /// itself, a tug that is a wreck, parties that differ, a casualty already coupled, a tug
    /// already towing, a tug that is itself under tow, a casualty that is itself towing, or no
    /// free ground beside the tug. The arena is unchanged.
    pub fn couple(&mut self, casualty: ObjId, tug: ObjId) -> Result<(), ArenaError> {
        let (Some(towed), Some(towing)) = (self.unit(casualty), self.unit(tug)) else {
            return Err(refused(casualty, tug, "no such unit is in the field"));
        };
        if casualty == tug {
            return Err(refused(casualty, tug, "a unit never tows itself"));
        }
        if towing.limits.wreck {
            return Err(refused(casualty, tug, "the tug is a wreck"));
        }
        if towed.team != towing.team {
            return Err(refused(
                casualty,
                tug,
                "a casualty is towed only by its own party",
            ));
        }
        if towed.coupled_to.is_some() {
            return Err(refused(casualty, tug, "that casualty is already under tow"));
        }
        if towing.coupled_to.is_some() {
            return Err(refused(casualty, tug, "the tug is itself under tow"));
        }
        if self.units.iter().any(|state| state.coupled_to == Some(tug)) {
            return Err(refused(casualty, tug, "the tug is already towing"));
        }
        if self
            .units
            .iter()
            .any(|state| state.coupled_to == Some(casualty))
        {
            return Err(refused(casualty, tug, "that casualty is itself towing"));
        }
        let terrain = std::sync::Arc::clone(&self.definition);
        unstack(&terrain.terrain, &mut self.units, casualty, tug)?;
        if let Some(state) = self.unit_mut(casualty) {
            state.coupled_to = Some(tug);
            state.limits.motion_disabled = true;
            state.speed_cms = 0;
            state.desired_speed_cms = 0;
            state.occupied_hold = None;
            state.held = None;
            state.carry = crate::arena::state::MotionCarry::default();
        }
        self.motors.release(casualty);
        self.index.rebuild(&terrain.terrain, &self.units);
        Ok(())
    }

    /// **The tug lets its casualty go** (a field act, `crate::arena_task::ArenaMsg::Release`):
    /// the casualty's coupling cleared where it stands; its limits stay a casualty's motion
    /// limits (damage never clears one inside a field), and it may fire again as its guns allow.
    /// Answers the casualty.
    ///
    /// # Errors
    ///
    /// [`ArenaError::TowRelease`], by name: a unit the field does not hold, or one that tows
    /// nothing. The arena is unchanged.
    pub fn release(&mut self, tug: ObjId) -> Result<ObjId, ArenaError> {
        if self.unit(tug).is_none() {
            return Err(ArenaError::TowRelease {
                tug,
                reason: "that unit is not in the field",
            });
        }
        let casualty = self
            .units
            .iter_mut()
            .find(|state| state.coupled_to == Some(tug))
            .ok_or(ArenaError::TowRelease {
                tug,
                reason: "that unit tows nothing",
            })?;
        casualty.coupled_to = None;
        Ok(casualty.id)
    }

    /// **Uncouple whatever `tug` tows**, silently: the tug destroyed or gone from the field. The
    /// casualty stands where it is.
    pub(crate) fn drop_tow(&mut self, tug: ObjId) {
        for state in &mut self.units {
            if state.coupled_to == Some(tug) {
                state.coupled_to = None;
            }
        }
    }

    /// **The casualty `tug` tows**, if any.
    #[must_use]
    pub fn trailer_of(&self, tug: ObjId) -> Option<ObjId> {
        self.units
            .iter()
            .find(|state| state.coupled_to == Some(tug))
            .map(|state| state.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::party::{EndCondition, FieldHost};
    use crate::arena::state::{PartyId, Power};
    use crate::arena::test_support::{custody, line_tank, pose, tank};
    use crate::arena::withdraw::WithdrawRefusal;
    use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};

    const TUG: ObjId = ObjId(4);
    const CASUALTY: ObjId = ObjId(6);
    const RAIDER: ObjId = ObjId(9);

    fn party(n: usize) -> PartyId {
        PartyId::nth(n).expect("a letter")
    }

    /// An IC field: the tug and its casualty (party A), stacked on one cell as the surface
    /// stacks them, and a raider (party B), with the host naming the tow.
    fn towing_field(tug: mush_sim::data::Pose, raider: mush_sim::data::Pose) -> Arena {
        let mut out = custody(
            vec![
                tank(TUG, party(0), tug.clone()),
                line_tank(CASUALTY, party(0), tug),
                line_tank(RAIDER, party(1), raider),
            ],
            Vec::new(),
        );
        out.host = FieldHost {
            end: EndCondition::Host,
            every_class: true,
            weapons_hold: true,
            tows: vec![(CASUALTY, TUG)],
            ..FieldHost::default()
        };
        let mut arena = Arena::new(
            out,
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .expect("the field");
        for state in &mut arena.units {
            state.power = Power::Online;
        }
        arena
    }

    fn cell(arena: &Arena, id: ObjId) -> (u16, u16) {
        arena
            .definition
            .terrain
            .cell_of(&arena.unit(id).expect("in the field").pose)
            .expect("on the window")
    }

    /// Order the tug north at its top speed.
    fn drive_north(arena: &mut Arena) {
        if let Some(state) = arena.unit_mut(TUG) {
            state.desired_heading_mdeg = 0;
            state.desired_speed_cms = 1_000;
        }
    }

    /// One unit a cell, by the check-out's own rule, and the index agreeing with the table.
    fn assert_one_a_cell(arena: &Arena) {
        crate::arena::state::check_poses(&arena.definition.terrain, &arena.units)
            .expect("one unit a cell");
        for state in &arena.units {
            let (x, y) = cell(arena, state.id);
            assert_eq!(
                arena.index.occupant(x, y),
                Some(state.id),
                "the index agrees"
            );
        }
    }

    /// **Doc 40 cut FR.5 (Q-40-7)**: the stacked casualty is unstacked beside its tug at
    /// check-out, coupled, motionless, and then trails it cell for cell — the cell it takes is
    /// always the one its tug just left, one unit a cell holds at every substep, and the substep
    /// bound stands.
    #[test]
    fn fr5_a_trailer_follows_its_tug_cell_for_cell() {
        let mut arena = towing_field(pose(1_500, 1_500), pose(6_500, 6_500));
        let state = arena.unit(CASUALTY).expect("the casualty");
        assert_eq!(state.coupled_to, Some(TUG));
        assert!(state.limits.motion_disabled && state.speed_cms == 0);
        let (tug, towed) = (cell(&arena, TUG), cell(&arena, CASUALTY));
        assert_eq!(
            tug.0.abs_diff(towed.0).max(tug.1.abs_diff(towed.1)),
            1,
            "unstacked on the nearest ring"
        );
        assert_one_a_cell(&arena);
        drive_north(&mut arena);
        let mut tug_was = cell(&arena, TUG);
        let mut crossings = 0;
        for wall in (100..=9_000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
            assert_one_a_cell(&arena);
            crate::arena::state::check_substep(
                &arena.definition.terrain,
                &arena.units,
                &arena.min_cost,
            )
            .expect("the substep bound");
            let tug_now = cell(&arena, TUG);
            if tug_now != tug_was {
                crossings += 1;
                assert_eq!(cell(&arena, CASUALTY), tug_was, "on the cell the tug left");
                let casualty = arena.unit(CASUALTY).expect("the casualty");
                assert_eq!(
                    casualty.pose.heading_mdeg,
                    arena.unit(TUG).expect("the tug").pose.heading_mdeg
                );
                tug_was = tug_now;
            }
        }
        assert!(crossings >= 3, "the tug crossed {crossings} cells");
    }

    /// **Doc 40 cut FR.5 (Q-40-7)**: a hit on a casualty lands against its own sections, and
    /// once its tug releases it, it stands while the tug drives on.
    #[test]
    fn fr5_a_trailer_is_struck_and_released() {
        let mut arena = towing_field(pose(1_500, 1_500), pose(6_500, 6_500));
        let before = arena.unit(CASUALTY).expect("the casualty").unit.clone();
        let mut journal = Vec::new();
        arena.strike_for_test(RAIDER, CASUALTY, 1, 400, &mut journal);
        let after = &arena.unit(CASUALTY).expect("the casualty").unit;
        assert_ne!(&before, after, "the round struck the casualty's own cells");
        assert_eq!(arena.unit(CASUALTY).map(|s| s.coupled_to), Some(Some(TUG)));
        assert_eq!(arena.release(TUG), Ok(CASUALTY));
        assert_eq!(arena.unit(CASUALTY).map(|s| s.coupled_to), Some(None));
        assert!(matches!(
            arena.release(TUG),
            Err(ArenaError::TowRelease {
                reason: "that unit tows nothing",
                ..
            })
        ));
        let stands = cell(&arena, CASUALTY);
        drive_north(&mut arena);
        for wall in (100..=4_000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
            assert_one_a_cell(&arena);
        }
        assert_ne!(cell(&arena, TUG), (1, 1), "the tug drove on");
        assert_eq!(cell(&arena, CASUALTY), stands, "it no longer follows");
    }

    /// **Doc 40 cut FR.5 (Q-40-7)**: a tug destroyed drops its tow — the casualty stays on its
    /// last cell, uncoupled.
    #[test]
    fn fr5_a_tug_destroyed_leaves_its_casualty_standing() {
        let mut arena = towing_field(pose(1_500, 1_500), pose(6_500, 6_500));
        drive_north(&mut arena);
        for wall in (100..=2_500).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        let stands = cell(&arena, CASUALTY);
        let mut journal = Vec::new();
        let now = arena.now_ms;
        arena.destroy_unit(TUG, now, None, &mut journal);
        assert_eq!(arena.unit(CASUALTY).map(|s| s.coupled_to), Some(None));
        for wall in (2_600..=4_000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        assert_eq!(cell(&arena, CASUALTY), stands, "where it stood");
    }

    /// **Doc 40 cut FR.5 (Q-40-25)**: the coupled withdraw at the ramp — the casualty goes out
    /// with its tug beside the ramp, both check-in rows answered; asked alone while its tug is
    /// far, it is refused `NotAtRamp`.
    #[test]
    fn fr5_the_coupled_pair_withdraws_at_the_ramp() {
        let mut arena = towing_field(pose(1_500, 1_500), pose(6_500, 6_500));
        assert_eq!(
            arena.withdraw(CASUALTY, Some((6_500, 1_500))).err(),
            Some(WithdrawRefusal::NotAtRamp),
            "its tug is far from the ramp"
        );
        let ramp = Some((2_500, 1_500));
        let casualty = arena.withdraw(CASUALTY, ramp).expect("it goes out in tow");
        assert_eq!(casualty.unit, CASUALTY);
        let tug = arena.withdraw(TUG, ramp).expect("and the tug after it");
        assert_eq!(tug.unit, TUG);
        assert!(arena.unit(CASUALTY).is_none() && arena.unit(TUG).is_none());
        for wall in (100..=1_000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        assert!(arena.unit(RAIDER).is_some() && arena.ended.is_none());
    }

    /// **Doc 40 cut FR.5**: a tug that leaves the field alone uncouples its trailer, which stands.
    #[test]
    fn fr5_a_tug_taken_out_uncouples_its_trailer() {
        let mut arena = towing_field(pose(1_500, 1_500), pose(6_500, 6_500));
        arena
            .withdraw(TUG, Some((2_500, 1_500)))
            .expect("the tug leaves");
        assert_eq!(arena.unit(CASUALTY).map(|s| s.coupled_to), Some(None));
        assert_eq!(
            arena.withdraw(CASUALTY, Some((2_500, 1_500))).err(),
            Some(WithdrawRefusal::NotUnderOwnPower),
            "uncoupled, it cannot come in on its own"
        );
    }

    /// **Doc 40 cut FR.5**: the refusals by name, each leaving the arena as it was.
    #[test]
    fn fr5_a_coupling_is_refused_by_name() {
        let mut arena = towing_field(pose(1_500, 1_500), pose(6_500, 6_500));
        let hash = crate::arena::hash::state_hash(&arena);
        let reason = |result: Result<(), ArenaError>| match result {
            Err(ArenaError::Coupling { reason, .. }) => reason,
            other => panic!("not a coupling refusal: {other:?}"),
        };
        assert_eq!(
            reason(arena.couple(ObjId(99), TUG)),
            "no such unit is in the field"
        );
        assert_eq!(
            reason(arena.couple(CASUALTY, TUG)),
            "that casualty is already under tow"
        );
        assert_eq!(
            reason(arena.couple(RAIDER, TUG)),
            "a casualty is towed only by its own party"
        );
        assert_eq!(crate::arena::hash::state_hash(&arena), hash, "unchanged");
        let _ = arena.release(TUG);
        if let Some(state) = arena.unit_mut(TUG) {
            state.limits.wreck = true;
        }
        assert_eq!(reason(arena.couple(CASUALTY, TUG)), "the tug is a wreck");
    }

    /// **Doc 40 cut FR.5**: a coupled casualty fires nothing.
    #[test]
    fn fr5_a_coupled_casualty_fires_nothing() {
        let mut arena = towing_field(pose(1_500, 1_500), pose(1_500, 3_500));
        let gun = arena
            .unit(CASUALTY)
            .and_then(|state| state.weapons.first())
            .map(|weapon| weapon.id.clone())
            .expect("a line tank carries a gun");
        assert_eq!(
            arena.resolve_fire(CASUALTY, &gun, 0),
            Err(crate::arena::intents::Refusal::NoWeapon)
        );
    }
}
