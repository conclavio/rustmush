//! **The absent player's rule** (doc 44 cut MW.7; Q-44-18, Q-44-22, Q-44-37, Q-44-38; doc 35
//! §11.1 `B-4`).
//!
//! Every unit always has a mind, and presence decides who holds the controls (Q-44-22). While a
//! mind flies a unit for a player who is not at them, three things are true of the unit whatever
//! mind it is, the house default or the one the player sealed:
//!
//! 1. **It never opens fire first** (Q-44-38). It may fire only at a unit whose fire has arrived
//!    on **this** unit since the field or the bubble opened. A leader's hostile mark, a party
//!    held hostile, or fire on a friend is not enough, and the doctrine's cornered stand is a
//!    shot like any other.
//! 2. **A stricter order the player left stays** (Q-44-18): a hold on its fire still holds, so
//!    such a unit answers nobody.
//! 3. **It neither mines nor trades** (Q-44-38).
//!
//! [`permits`] is that rule, and it is the only copy: the arena calls it where a mind's shot is
//! resolved (`Arena::resolve_fire_by`, the one door every shot that no seat typed goes through:
//! a mind's own, a standing fire rule's, a machine's), and a seat in space calls it with facts
//! of its own. **What a caller must provide** is [`Facts`] and nothing else: whether the unit is
//! flown for an absent player, whether a hold on its fire stands, and the set of units that have
//! fired on it. The ids are whatever the caller's units are known by, as long as the target of
//! the [`Ask`] and the set use the same ones. The rule reads no arena, no clock and no dice.
//!
//! **In the arena** the three facts are on the unit:
//! [`crate::arena::state::UnitState::absent`], written by the IC field host through
//! [`Arena::set_absent`] and never by a practice match (Q-44-19);
//! [`crate::arena::state::UnitState::fired_on_by`], written at every impact, hit or miss
//! (`Arena::resolve_impact`), in a field whose end is its host's; and the hold, which is the
//! seat's own master gate ([`crate::arena::state::UnitState::weapons_hold`], the hold the pilot
//! left) or a standing order's `hold_fire` (`Arena::holds_fire`).
//!
//! **Going home.** [`Arena::set_absent`] also lays the standing objective every mind reads as
//! its order (`MindObjective::Withdraw`, no deadline): the stock mind obeys it as it obeys a
//! leader's, driving for its party's home and never rallying while it stands, and a player's
//! sealed mind reads it as `v.order`. An order to advance or attack that stood lapses with it;
//! its `hold_fire` is kept (Q-44-18). The order is the unit's while the flag stands: a
//! detachment command that arrives meanwhile can only tighten it.
//!
//! **What the mind is shown.** While the flag stands, [`Arena::mind_view`] shows the mind as
//! hostile only the contacts the rule would let it answer (`Arena::answerable`); every other
//! contact its party holds hostile reads neutral. So the stock mind's cornered stand, which
//! stops for the best hostile in range, stops only for a unit that has fired on it, and a unit
//! with a marked party in view drives on. The view is narrowed, never widened, and the gate
//! does not depend on it: a mind that fires at a contact shown neutral is refused at the shot.
//!
//! **Handing back** (Q-44-37): the host clears the flag when the player takes the controls
//! again. The seat's `Live` control has by then discarded the mind's pending think and its held
//! order (`Arena::suspend_mind`); clearing the flag ends the rule.

use std::collections::BTreeSet;

use mush_core::ObjId;

use crate::arena::state::{Arena, ContactId, Controller};

/// **What a mind flying for an absent player asks to do.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask<Id> {
    /// Fire on the unit `at`.
    Fire {
        /// The target, by the id the caller's [`Facts::fired_on_by`] uses.
        at: Id,
    },
    /// Work a deposit, a rock or a wreck for what it yields.
    Mine,
    /// Buy, sell or contract.
    Trade,
}

/// **What the rule reads of the unit** (this module's head): everything a caller provides.
#[derive(Clone, Copy, Debug)]
pub struct Facts<'a, Id: Ord> {
    /// A mind flies the unit for a player who is not at its controls.
    pub absent: bool,
    /// A hold on its fire stands: one the player left, or a standing order's.
    pub holds_fire: bool,
    /// The units whose fire has arrived on this one since the field or the bubble opened.
    pub fired_on_by: &'a BTreeSet<Id>,
}

/// **Why the rule refused.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denied {
    /// The target has not fired on this unit: the shot would open fire first.
    FiredFirst,
    /// A hold on the unit's fire stands, and holds through an attack.
    HeldFire,
    /// A mind does not mine for an absent player.
    Mining,
    /// A mind does not trade for an absent player.
    Trading,
}

impl Denied {
    /// The refusal as a sentence, for a trace or a staff read.
    #[must_use]
    pub const fn sentence(self) -> &'static str {
        match self {
            Denied::FiredFirst => {
                "Its player is away: it fires only on a unit that has fired on it."
            }
            Denied::HeldFire => "Its player is away and left its fire held: it answers nobody.",
            Denied::Mining => "Its player is away: it does not mine.",
            Denied::Trading => "Its player is away: it does not trade.",
        }
    }
}

/// **The absent player's rule** (this module's head): `Ok` for a unit its player flies, and for
/// a shot at a unit that has fired on this one while no hold stands.
///
/// # Errors
///
/// [`Denied`], naming which of the three limits refused.
pub fn permits<Id: Ord>(facts: &Facts<'_, Id>, ask: &Ask<Id>) -> Result<(), Denied> {
    if !facts.absent {
        return Ok(());
    }
    match ask {
        Ask::Mine => Err(Denied::Mining),
        Ask::Trade => Err(Denied::Trading),
        Ask::Fire { .. } if facts.holds_fire => Err(Denied::HeldFire),
        Ask::Fire { at } if facts.fired_on_by.contains(at) => Ok(()),
        Ask::Fire { .. } => Err(Denied::FiredFirst),
    }
}

impl Arena {
    /// **Whether a mind flies `unit` for an absent player** (this module's head).
    #[must_use]
    pub fn absent(&self, unit: ObjId) -> bool {
        self.unit(unit).is_some_and(|state| state.absent)
    }

    /// **The host says whether `unit` is flown for an absent player** (this module's head;
    /// `crate::arena_task::ArenaMsg::Absent`, applied at a pulse's head). Setting it lays the
    /// go-home order on a hull a mind already flies; a hull still its pilot's gets the order at
    /// the `Released` that hands it over (`crate::arena::intents::apply_control`). Clearing it
    /// lifts the order if the hull is still a mind's. Answers whether the arena holds the unit.
    pub fn set_absent(&mut self, unit: ObjId, absent: bool) -> bool {
        let Some(state) = self.unit_mut(unit) else {
            return false;
        };
        let was = std::mem::replace(&mut state.absent, absent);
        let minded = state.controller == Controller::Mind;
        if absent && minded {
            self.lay_go_home(unit);
        } else if was && !absent && minded {
            // A hand-back to a pilot has cleared the row already (`Arena::suspend_mind`); a
            // flag cleared on a hull still a mind's leaves it to its doctrine again.
            self.clear_go_home(unit);
        }
        true
    }

    /// **The standing objective of an absent player's unit** (this module's head, "going
    /// home"): `Withdraw`, with no deadline, replacing whatever order stood and keeping that
    /// order's `hold_fire`. Nothing for a hull with no mind row.
    pub(crate) fn lay_go_home(&mut self, unit: ObjId) {
        use crate::deliberation::order::{Constraints, Objective};
        let Some(team) = self.unit(unit).map(|state| state.team) else {
            return;
        };
        let held = self
            .minds
            .get(&unit)
            .and_then(|row| row.order.as_ref())
            .is_some_and(|order| order.hold_fire);
        let order = self.rekey_objective(
            unit,
            None,
            crate::deliberation::SquadId { team, index: 1 },
            crate::arena::detachment::DetachmentObjective {
                objective: Objective::Withdraw,
                constraints: Constraints {
                    hold_fire: held,
                    ..Constraints::default()
                },
            },
            i64::MAX,
        );
        self.clear_order_watch_member(unit);
        if let Some(row) = self.minds.get_mut(&unit) {
            row.order = Some(order);
            row.order_issuer = None;
            row.order_charge = None;
        }
    }

    /// Lift [`Arena::lay_go_home`]'s order.
    fn clear_go_home(&mut self, unit: ObjId) {
        if let Some(row) = self.minds.get_mut(&unit) {
            row.order = None;
            row.order_issuer = None;
            row.order_charge = None;
        }
        self.clear_order_watch_member(unit);
    }

    /// **Whether the rule refuses `unit` a shot at `target`** (this module's head): the facts
    /// read off the unit, the rule [`permits`]. `None` for a unit its player flies, and for a
    /// unit this arena does not hold.
    #[must_use]
    pub fn absent_denies_fire(&self, unit: ObjId, target: ObjId) -> Option<Denied> {
        let state = self.unit(unit)?;
        if !state.absent {
            return None;
        }
        let facts = Facts {
            absent: true,
            holds_fire: state.weapons_hold || self.holds_fire(unit),
            fired_on_by: &state.fired_on_by,
        };
        permits(&facts, &Ask::Fire { at: target }).err()
    }

    /// **`shooter`'s fire has arrived on `target`** (this module's head), hit or miss: recorded
    /// on the target in a field whose end is its host's, and nowhere else, so a practice match
    /// carries and hashes no byte of it. A unit's own fire on itself records nothing.
    pub(crate) fn note_fired_on(&mut self, shooter: ObjId, target: ObjId) {
        if shooter == target || self.end != crate::arena::party::EndCondition::Host {
            return;
        }
        if let Some(state) = self.unit_mut(target) {
            state.fired_on_by.insert(shooter);
        }
    }

    /// **The contacts an absent player's `unit` may answer** (this module's head, "what the
    /// mind is shown"): its own contact ids for the units that have fired on it, or none at all
    /// while a hold on its fire stands. `None` for a unit its player flies.
    #[must_use]
    pub(crate) fn answerable(&self, unit: ObjId) -> Option<BTreeSet<ContactId>> {
        let state = self.unit(unit).filter(|state| state.absent)?;
        if state.weapons_hold || self.holds_fire(unit) {
            return Some(BTreeSet::new());
        }
        Some(
            state
                .fired_on_by
                .iter()
                .filter_map(|by| self.contacts.id_of(unit, *by))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(absent: bool, holds_fire: bool, by: &BTreeSet<u32>) -> Facts<'_, u32> {
        Facts {
            absent,
            holds_fire,
            fired_on_by: by,
        }
    }

    /// A unit its player flies is refused nothing by this rule.
    #[test]
    fn mw7_a_present_player_is_not_bound() {
        let nobody = BTreeSet::new();
        for ask in [Ask::Fire { at: 7 }, Ask::Mine, Ask::Trade] {
            assert_eq!(permits(&facts(false, true, &nobody), &ask), Ok(()));
        }
    }

    /// The three limits, and the one shot the rule lets through.
    #[test]
    fn mw7_an_absent_players_unit_answers_only_who_fired_on_it() {
        let by = BTreeSet::from([7_u32]);
        let free = facts(true, false, &by);
        assert_eq!(permits(&free, &Ask::Fire { at: 7 }), Ok(()));
        assert_eq!(
            permits(&free, &Ask::Fire { at: 8 }),
            Err(Denied::FiredFirst),
            "a unit that has not fired on it, whatever its party holds it"
        );
        assert_eq!(permits(&free, &Ask::Mine), Err(Denied::Mining));
        assert_eq!(permits(&free, &Ask::Trade), Err(Denied::Trading));
        let held = facts(true, true, &by);
        assert_eq!(
            permits(&held, &Ask::Fire { at: 7 }),
            Err(Denied::HeldFire),
            "a hold the player left holds through an attack"
        );
    }
}
