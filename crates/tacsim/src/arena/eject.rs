//! **A pilot ejecting from a doomed unit** (doc 42 cut CR.4; Q-42-10 to Q-42-12): the arena's
//! one read for it.
//!
//! The IC field host sends [`crate::arena_task::ArenaMsg::Eject`]; at the next pulse's head,
//! before the step, [`Arena::eject_read`] reads the arena's own live facts — the unit's cells and
//! limits, every other unit's pose and reach, the parties' books — and answers an
//! [`crate::arena_task::ArenaOut::Ejected`]. **A read, never a step**: no state moves, nothing is
//! journalled and nothing is hashed (the withdraw's precedent, without the withdraw's removal).
//! The seat's release is the host's `Released` control (doc 32 cut CF.5), and the draw is the
//! host's, seeded from the answer's arena millisecond.
//!
//! **Doomed** (Q-42-11): the test that would strand the pilot at check-in, read on the live
//! cells — no power, no control, no motion, or a wreck or irrecoverable state, or the kernel's own
//! wreck, motion-disabled or disabled limits — **or** a control section already damaged
//! ([`crate::field::crew::doomed`]).
//!
//! **Holding the cell** (Q-42-12, Draft 0.3's 42-1): a party P holds it when (a) P holds the
//! pilot's party Hostile in its book; (b) a live unit of P, armed at check-out or having fired,
//! has the doomed unit's position inside its own [`UnitState::fight_inner_cm`]; and (c) no unit of
//! the pilot's own party or a Friend's but the ejecting one — not a wreck, armed, with a non-zero
//! reach — has it inside its reach. The answer lists every holder, nearest unit first; (d), a
//! carrier of the holder's on the site, is the host's to read.

use mush_core::ObjId;

use crate::arena::PartyId;
use crate::arena::party::Relation;
use mush_sim::data::Pose;

use crate::arena::state::{Arena, UnitState};

/// **Why the arena refused an eject.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EjectRefusal {
    /// The arena holds no such unit.
    NotHere,
    /// The unit can still bring its pilot home.
    NotDoomed,
}

/// **The arena's facts for an eject** ([`Arena::eject_read`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EjectRead {
    /// The ejecting unit's party.
    pub party: PartyId,
    /// The parties holding the cell, nearest unit first (empty: nobody holds it).
    pub holders: Vec<PartyId>,
    /// The crew's condition the live sections give (Q-42-13: the eject keeps it).
    pub condition: crate::field::crew::CrewCondition,
    /// The arena millisecond of the read: the draw's pulse.
    pub at_ms: i64,
}

/// Whether `viewer` is armed and has `at` inside its own reach, at `cell_cm`.
fn reaches(viewer: &UnitState, at: &Pose, cell_cm: u16) -> bool {
    let reach = viewer.fight_inner_cm(cell_cm);
    !viewer.limits.wreck
        && (viewer.armed_at_checkout() || viewer.shots_fired > 0)
        && reach != 0
        && crate::arena::index::within_cm(&viewer.pose, at, reach)
}

/// The squared distance between two poses, centimetres squared.
fn apart_sq(a: &Pose, b: &Pose) -> i128 {
    let dx = i128::from(a.x_cm) - i128::from(b.x_cm);
    let dy = i128::from(a.y_cm) - i128::from(b.y_cm);
    let dz = i128::from(a.z_cm) - i128::from(b.z_cm);
    dx * dx + dy * dy + dz * dz
}

impl Arena {
    /// **The eject's read for `unit`** (this module's head).
    ///
    /// # Errors
    ///
    /// [`EjectRefusal`]; the arena is unchanged either way.
    pub fn eject_read(&self, unit: ObjId) -> Result<EjectRead, EjectRefusal> {
        let state = self.unit(unit).ok_or(EjectRefusal::NotHere)?;
        let doomed = state.limits.wreck
            || state.limits.motion_disabled
            || state.disabled
            || crate::field::crew::doomed(&state.unit);
        if !doomed {
            return Err(EjectRefusal::NotDoomed);
        }
        let cell_cm = self.definition.terrain.cell_cm();
        let pilot_party = state.team;
        let at = state.pose.clone();
        let others = || self.units.iter().filter(move |other| other.id != unit);
        // (c): the pilot's own party or a Friend's contests the cell.
        let contested = others().any(|friend| {
            matches!(
                self.relations.relation(pilot_party, friend.team),
                Relation::Own | Relation::Friend
            ) && reaches(friend, &at, cell_cm)
        });
        let mut near: Vec<(i128, PartyId)> = if contested {
            Vec::new()
        } else {
            // (a) and (b).
            others()
                .filter(|holder| {
                    holder.team != pilot_party
                        && self.relations.relation(holder.team, pilot_party) == Relation::Hostile
                        && reaches(holder, &at, cell_cm)
                })
                .map(|holder| (apart_sq(&holder.pose, &at), holder.team))
                .collect()
        };
        near.sort();
        let mut holders: Vec<PartyId> = Vec::new();
        for (_, party) in near {
            if !holders.contains(&party) {
                holders.push(party);
            }
        }
        Ok(EjectRead {
            party: pilot_party,
            holders,
            condition: crate::field::crew::cells_condition(&state.unit),
            at_ms: self.now_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::party::Mark;
    use crate::arena::state::Power;
    use crate::arena::test_support::{arena_of, line_tank, pose};

    const PILOT: ObjId = ObjId(4);
    const RAIDER: ObjId = ObjId(9);
    const WING: ObjId = ObjId(12);

    /// The pilot's tank beside a hostile raider's, both online; `wing` adds a third tank of the
    /// pilot's party at `wing`'s pose.
    fn field(wing: Option<Pose>) -> Arena {
        let mut units = vec![
            line_tank(PILOT, PartyId::RED, pose(1_500, 1_500)),
            line_tank(RAIDER, PartyId::BLUE, pose(2_500, 1_500)),
        ];
        if let Some(at) = wing {
            units.push(line_tank(WING, PartyId::RED, at));
        }
        let mut arena = arena_of(units);
        for id in [PILOT, RAIDER, WING] {
            if let Some(state) = arena.unit_mut(id) {
                state.power = Power::Online;
            }
        }
        arena
            .relations
            .mark(PartyId::BLUE, PartyId::RED, Some(Mark::Hostile));
        arena
    }

    /// Doc 42 cut CR.4 (Q-42-11): a unit that can still bring its pilot home refuses; a motion-
    /// disabled one is doomed.
    #[test]
    fn cr4_only_a_doomed_unit_ejects() {
        let mut arena = field(None);
        assert_eq!(arena.eject_read(PILOT).err(), Some(EjectRefusal::NotDoomed));
        assert_eq!(
            arena.eject_read(ObjId(77)).err(),
            Some(EjectRefusal::NotHere)
        );
        if let Some(state) = arena.unit_mut(PILOT) {
            state.limits.motion_disabled = true;
        }
        let read = arena.eject_read(PILOT).expect("doomed");
        assert_eq!(read.party, PartyId::RED);
        assert!(arena.unit(PILOT).is_some(), "a read takes nothing out");
    }

    /// Doc 42 cut CR.4 (Q-42-12, 42-1): a hostile in reach holds the cell; the pilot's own wing
    /// in reach contests it; the ejecting unit itself never does; a party that holds the pilot
    /// Neutral holds nothing.
    #[test]
    fn cr4_holding_the_cell() {
        let mut arena = field(None);
        if let Some(state) = arena.unit_mut(PILOT) {
            state.limits.motion_disabled = true;
        }
        let reach = arena
            .unit(RAIDER)
            .map(|raider| raider.fight_inner_cm(arena.cell_cm()))
            .expect("the raider");
        assert!(
            reach > 1_000,
            "the line tank reaches the next cell: {reach}"
        );
        assert_eq!(
            arena.eject_read(PILOT).expect("doomed").holders,
            vec![PartyId::BLUE],
            "the ejecting tank's own mount never contests its cell"
        );
        // The wing beside the pilot contests it.
        let mut wing = field(Some(pose(1_500, 2_500)));
        if let Some(state) = wing.unit_mut(PILOT) {
            state.limits.motion_disabled = true;
        }
        assert!(wing.eject_read(PILOT).expect("doomed").holders.is_empty());
        // A wrecked wing contests nothing.
        if let Some(state) = wing.unit_mut(WING) {
            state.limits.wreck = true;
        }
        assert_eq!(
            wing.eject_read(PILOT).expect("doomed").holders,
            vec![PartyId::BLUE]
        );
        // No Hostile mark, no fire: Neutral holds nothing.
        arena.relations.mark(PartyId::BLUE, PartyId::RED, None);
        assert!(arena.eject_read(PILOT).expect("doomed").holders.is_empty());
    }
}
