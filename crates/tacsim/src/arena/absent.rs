//! **The absent player's rule** (doc 44 cut MW.7; Q-44-18, Q-44-22, Q-44-37, Q-44-38; doc 35
//! §11.1 `B-4`), **and the posture a mind flies under it** (doc 45 unit AP.6; Q-45-5 to
//! Q-45-9, Q-45-12, Q-45-16).
//!
//! Every unit always has a mind, and presence decides who holds the controls (Q-44-22). While a
//! mind flies a unit for a player who is not at them, three things are true of the unit whatever
//! mind it is, the house default or the one the player sealed:
//!
//! 1. **It never opens fire first** (Q-44-38). It may fire only at a unit whose fire has arrived
//!    on **this** unit since the field or the bubble opened. A leader's hostile mark, a party
//!    held hostile, a stance an organisation declared (Q-45-6), or fire on a friend is not
//!    enough, and the doctrine's cornered stand is a shot like any other. **One widening, and
//!    no other** (Q-45-8): a unit standing on `GUARD`, inside its leash, may also answer a unit
//!    whose fire has arrived on another unit of the same player.
//! 2. **A stricter order stays** (Q-44-18): a standing order's hold on its fire still holds, so
//!    such a unit answers nobody.
//! 3. **It neither mines nor trades** (Q-44-38), whatever its player left (Q-45-12).
//!
//! [`permits`] is that rule, and it is the only copy: the arena calls it where a mind's shot is
//! resolved (`Arena::resolve_fire_by`, the one door every shot that no seat typed goes through:
//! a mind's own, a standing fire rule's, a machine's), and a seat in space calls it with facts
//! of its own. **What a caller must provide** is [`Facts`] and nothing else: whether the unit is
//! flown for an absent player, whether a hold on its fire stands, the set of units that have
//! fired on it, whether it stands guard, and the set of units that have fired on its player's
//! other units. The ids are whatever the caller's units are known by, as long as the target of
//! the [`Ask`] and the sets use the same ones. The rule reads no arena, no clock and no dice.
//!
//! **In the arena** the facts are on the unit:
//! [`crate::arena::state::UnitState::absent`], written through [`Arena::set_absent`] by the
//! IC field host and, in a practice match, only for a pilot who lets go of the controls in
//! the pod to practise a posture (doc 45 unit AP.10; `crate::match_::practice`), never for
//! a pilot who drops (Q-44-19): there any control that names the practising pilot ends the
//! practice before it applies (`Arena::end_practice`);
//! [`crate::arena::state::UnitState::fired_on_by`], written at every impact, hit or miss
//! (`Arena::resolve_impact`), in a field whose end is its host's and, in a practice match,
//! on a hull a pilot has named a player for; the hold, which is a standing
//! order's `hold_fire` (`Arena::holds_fire`); the posture
//! ([`crate::arena::state::UnitState::posture`]) with the cell the mind stands it on
//! ([`crate::arena::state::UnitState::stand_at`]); and whose the unit is
//! ([`crate::arena::state::UnitState::player`]).
//!
//! **The posture and the seat's fire hold** (Q-45-5, Q-45-7 to Q-45-9). The posture is the one
//! word a player leaves standing. The seat's master switch (`FIRE HOLD`,
//! [`crate::arena::state::UnitState::weapons_hold`]) is the pilot's own fire decision, made for
//! a pilot who is watching: like the condition slots, which are cleared at the hand-over
//! (`Arena::set_controller`), it binds the seat and never the mind that flies for an absent
//! player. So a unit whose seat was left at hold, or was never touched because its player was
//! away when the field opened, still answers a unit that fires on it, under every posture.
//! Until this unit the switch did bind the mind (MW.7's default, flagged there for the owner).
//!
//! **Flying a posture.** [`Arena::flown_posture`] is what the unit's mind is held to:
//!
//! - `RETURN`, the default: **going home.** [`Arena::set_absent`] lays the standing objective
//!   every mind reads as its order (`MindObjective::Withdraw`, no deadline): the stock mind
//!   obeys it as it obeys a leader's, driving for its party's home and never rallying while it
//!   stands, and a player's sealed mind reads it as `v.order`. **It answers fire on the way
//!   and does not stop to** (doc 47 unit CO.2, R-47-3): the stock mind answers by a cornered
//!   stand, and the landing lands that stand's stop as the drive home
//!   ([`Arena::keep_going_home`]).
//! - `HOLD` and `GUARD`: **staying.** The order laid is `Hold`: the unit parks where it stands.
//!   At each think (`Arena::stand_posture`) the order is turned to a `Screen` that faces the
//!   nearest contact the unit may answer, and back to `Hold` when it holds none, so the stock
//!   mind stands, turns and returns fire without leaving its cell, by the orders it already
//!   obeys. The kernel does not let such a unit out at its ramp
//!   ([`crate::arena::withdraw::WithdrawRefusal::Standing`]).
//! - `GUARD` **ends when the unit can no longer defend itself** (Q-45-9): no fire channel with
//!   a round left, or its motion disabled. It is then flown as `RETURN`: the frame and the
//!   view read `return`, the order is the go-home one, and the widening is gone. The player's
//!   record is not rewritten.
//! - **A unit that carries nothing to fire cannot guard** (doc 45 unit AP.7; Q-45-10): it
//!   never could defend itself, so its `GUARD` did not end, it never began. The player's word
//!   was to stay, and the unit stays: it is flown as `HOLD`, and reads `hold`. (Until AP.7 it
//!   was flown as `RETURN` at once, AP.6's default 6.)
//!
//! An order to advance or attack that stood lapses when either order is laid; its `hold_fire`
//! is kept (Q-44-18). The order is the unit's while the flag stands: a detachment command that
//! arrives meanwhile can only tighten it.
//!
//! **The leash** ([`GUARD_LEASH_CELLS`]). `GUARD` is stood where the mind took it
//! (`stand_at`). The house mind stays on that cell while it has a round to fire: the kernel
//! spends the doctrine's one retreat when the posture is first stood
//! (`crate::arena::mind::seed_stay`), so a hurt unit does not break for home. A mind that
//! moves (a player's sealed one) keeps the widening only inside the leash: past it the unit
//! answers for itself alone, as on `HOLD`. A unit of the player's that has left the field
//! (gone aboard) takes its fire record with it: a guard answers for the units still there.
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
//! order (`Arena::suspend_mind`); clearing the flag ends the rule. The posture stays the
//! unit's (Q-45-18).
//!
//! **Nothing an attacker reads changes** (Q-45-16): no contact row, notice or presence row
//! carries the flag, the posture, the cell stood on or the player.

use std::collections::BTreeSet;

use mush_core::ObjId;

use crate::arena::state::{Arena, ContactId, Controller, UnitState};
use crate::posture::Posture;

/// **How far from where it was set a unit on `GUARD` still answers for its player's other
/// units**, cells, Chebyshev (doc 45 unit AP.6; Q-45-8). Provisional, as doc 45 §6 says of
/// every tuning figure: ten.
pub const GUARD_LEASH_CELLS: u16 = 10;

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
    /// A hold on its fire stands: a standing order's.
    pub holds_fire: bool,
    /// The units whose fire has arrived on this one since the field or the bubble opened.
    pub fired_on_by: &'a BTreeSet<Id>,
    /// It stands on `GUARD`, inside the leash of where it was set, and can still defend
    /// itself (doc 45 unit AP.6; Q-45-8, Q-45-9).
    pub guards: bool,
    /// The units whose fire has arrived on **another unit of the same player** since the
    /// field or the bubble opened. Read only while `guards` holds.
    pub fired_on_kin: &'a BTreeSet<Id>,
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
                "Its player is away: it fires only on a unit that has fired on it, or on GUARD on one that has fired on another unit of theirs."
            }
            Denied::HeldFire => {
                "Its player is away and an order holds its fire: it answers nobody."
            }
            Denied::Mining => "Its player is away: it does not mine.",
            Denied::Trading => "Its player is away: it does not trade.",
        }
    }
}

/// **The absent player's rule** (this module's head): `Ok` for a unit its player flies, and,
/// while no hold stands, for a shot at a unit that has fired on this one or, from a unit that
/// stands guard, at one that has fired on another unit of the same player.
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
        Ask::Fire { at } if facts.guards && facts.fired_on_kin.contains(at) => Ok(()),
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
    /// standing order of the unit's posture on a hull a mind already flies; a hull still its
    /// pilot's gets the order at the `Released` that hands it over
    /// (`crate::arena::intents::apply_control`). Clearing it lifts the order if the hull is
    /// still a mind's, and forgets where the posture was stood. Answers whether the arena
    /// holds the unit.
    pub fn set_absent(&mut self, unit: ObjId, absent: bool) -> bool {
        let Some(state) = self.unit_mut(unit) else {
            return false;
        };
        let was = std::mem::replace(&mut state.absent, absent);
        if !absent {
            state.stand_at = None;
        }
        let minded = state.controller == Controller::Mind;
        if absent && minded {
            self.lay_standing(unit);
        } else if was && !absent && minded {
            // A hand-back to a pilot has cleared the row already (`Arena::suspend_mind`); a
            // flag cleared on a hull still a mind's leaves it to its doctrine again.
            self.clear_standing(unit);
        }
        true
    }

    /// **A pilot's practice ends** (doc 45 unit AP.10; Q-45-25, with doc 44's Q-44-19):
    /// called by `crate::arena::intents::apply_control` for every control that names a
    /// pilot, before the control's own state is applied. In a match (never in a field
    /// whose end is its host's, where the host alone says who is absent), a hull flown for
    /// an absent player **is** a pilot practising a posture, and a word about that pilot's
    /// seat is the pilot at the controls again or gone from them: the practice is over
    /// either way. The absent player's rule is lifted, the posture the arena was told goes
    /// back to `RETURN`, and the hull is handed from its mind to the pilot, exactly as a
    /// `Live` hands one over. The control then applies to a hull its pilot holds, so a
    /// `Released` (a drop, an `EXIT`, a resume) hands it to the sealed mind seeded as every
    /// release seeds it, and a `Linkdead` leaves it a link-dead pilot's.
    ///
    /// It is here, on the control, and not a message of its own, because the task keeps one
    /// control a channel a pulse and the latest stands: a hand-back sent ahead of a release
    /// would be collapsed away and the release would meet a hull its mind already held.
    ///
    /// **Whichever seat the control names.** A hull's controller is the hull's and not a
    /// seat's: any pilot's `Live` takes a hull from its mind and any pilot's `Released` gives
    /// it one (`apply_control`), so a word about a second seat of a practising hull moves who
    /// flies it as it always did, and the practice cannot outlive that.
    ///
    /// **The hull is stopped**, as a `Released` stops one: the speed its mind last ordered is
    /// nobody's order once the mind is off the controls, and a hull handed back or held
    /// through a lost link's grace must not drive on by it. Its heading is left, as there.
    ///
    /// What is left of the practice on the hull is what reads nowhere without the flag:
    /// whose it is ([`UnitState::player`]) and the fire record kept since
    /// ([`UnitState::fired_on_by`]), for the pilot's next practice in the match; and what
    /// the mind remembers of having flown, which a release adopts again as it adopts any
    /// dormant mind's memory. Answers whether a practice was ended.
    pub(crate) fn end_practice(&mut self, unit: ObjId) -> bool {
        if self.end == crate::arena::party::EndCondition::Host {
            return false;
        }
        if !self.unit(unit).is_some_and(|state| state.absent) {
            return false;
        }
        self.set_absent(unit, false);
        self.set_posture(unit, Posture::Return);
        self.set_controller(
            unit,
            Controller::Pilot,
            crate::arena::doctrine::MachineState::Withdraw,
        );
        if let Some(state) = self.unit_mut(unit) {
            state.desired_speed_cms = 0;
        }
        true
    }

    /// **The host says whose unit `unit` is** (this module's head;
    /// `crate::arena_task::ArenaMsg::Posture`): the player `GUARD`'s widening reads. Answers
    /// whether the arena holds the unit.
    pub fn set_player(&mut self, unit: ObjId, player: Option<ObjId>) -> bool {
        let Some(state) = self.unit_mut(unit) else {
            return false;
        };
        state.player = player;
        true
    }

    /// **The posture `unit`'s mind is held to** (this module's head, "flying a posture"): the
    /// one its player left, except that a `GUARD` flown for an absent player by a unit that
    /// can no longer defend itself is `RETURN` (Q-45-9), and by a unit that carries nothing
    /// to fire is `HOLD` (doc 45 unit AP.7; Q-45-10). `RETURN` for a unit the arena does not
    /// hold.
    #[must_use]
    pub fn flown_posture(&self, unit: ObjId) -> Posture {
        self.unit(unit).map_or(Posture::Return, flown)
    }

    /// **The standing order of an absent player's unit, by its posture** (this module's
    /// head): the go-home order on `RETURN`, a `Hold` where it stands on `GUARD` and `HOLD`,
    /// which the unit's next think turns to face what it may answer
    /// ([`Arena::stand_posture`]). Nothing for a hull with no mind row.
    pub(crate) fn lay_standing(&mut self, unit: ObjId) {
        if self.flown_posture(unit).is_return() {
            self.lay_go_home(unit);
        } else {
            self.lay_order(unit, crate::deliberation::order::Objective::Hold);
        }
    }

    /// **The standing objective of an absent player's unit on `RETURN`** (this module's head,
    /// "going home"): `Withdraw`, with no deadline, replacing whatever order stood and keeping
    /// that order's `hold_fire`. Nothing for a hull with no mind row.
    pub(crate) fn lay_go_home(&mut self, unit: ObjId) {
        self.lay_order(unit, crate::deliberation::order::Objective::Withdraw);
    }

    /// Lay `objective` as `unit`'s standing order: no deadline, no issuer, the `hold_fire` of
    /// the order it replaces kept (Q-44-18).
    fn lay_order(&mut self, unit: ObjId, objective: crate::deliberation::order::Objective) {
        use crate::deliberation::order::Constraints;
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
                objective,
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

    /// Lift the standing order [`Arena::lay_standing`] laid.
    fn clear_standing(&mut self, unit: ObjId) {
        if let Some(row) = self.minds.get_mut(&unit) {
            row.order = None;
            row.order_issuer = None;
            row.order_charge = None;
        }
        self.clear_order_watch_member(unit);
    }

    /// **A mind's think begins on a unit that stands a posture** (this module's head, "flying
    /// a posture"; `Arena::collect_think`, ahead of the view, with no think of the unit in
    /// flight). For a unit a mind flies for an absent player:
    ///
    /// - on `RETURN`, or on a `GUARD` it can no longer defend, the go-home order is laid if it
    ///   does not stand, and the cell stood on is forgotten;
    /// - on `HOLD` and `GUARD`, at the first such think the cell the unit stands on is kept
    ///   ([`UnitState::stand_at`]) and a Standard Doctrine memory is told to stay
    ///   (`crate::arena::mind::seed_stay`: a drive home is stopped and the doctrine's one
    ///   retreat is spent), so the unit stops where it is and does not break for home when it
    ///   is hurt; then the order is a `Screen` onto the nearest contact the unit may answer
    ///   ([`Arena::answerable`]) that its party holds hostile, kept while that contact is
    ///   inside the screen's arc, and a `Hold` when there is none.
    ///
    /// Nothing for any other unit: a match nobody practises a posture in (doc 45 unit
    /// AP.10) never reaches past the first test.
    pub(crate) fn stand_posture(&mut self, unit: ObjId) {
        use crate::arena::doctrine::{SCREEN_HALF_ARC_MDEG, arc_between};
        use crate::arena::party::Relation;
        use crate::arena::perception::MindObjective;
        use crate::deliberation::order::Objective;
        let Some(state) = self
            .unit(unit)
            .filter(|state| state.absent && state.controller == Controller::Mind)
        else {
            return;
        };
        let doctrine = state.seal.doctrine;
        let first = state.stand_at.is_none();
        let standing = self
            .minds
            .get(&unit)
            .and_then(|row| row.order.as_ref())
            .map(|order| order.objective);
        if flown(state).is_return() {
            if let Some(state) = self.unit_mut(unit) {
                state.stand_at = None;
            }
            if standing != Some(MindObjective::Withdraw) {
                self.lay_go_home(unit);
            }
            return;
        }
        let Some(frame) = self.frame_for(unit, self.clock.tempo()) else {
            return;
        };
        if first {
            // A unit off the ground's window stands nowhere yet: it is laid its hold and
            // asked again at its next think.
            let Some(cell) = frame.cell else {
                if !matches!(
                    standing,
                    Some(MindObjective::Hold | MindObjective::Screen { .. })
                ) {
                    self.lay_order(unit, Objective::Hold);
                }
                return;
            };
            if let Some(state) = self.unit_mut(unit) {
                state.stand_at = Some(cell);
            }
            if doctrine && let Some(row) = self.minds.get_mut(&unit) {
                row.memory = crate::arena::mind::seed_stay(&row.memory);
                row.doctrine_state = crate::arena::mind::doctrine_state_of(&row.memory);
            }
        }
        // The nearest contact it may answer, as its view will show it: held hostile by its
        // party and let through by the rule. The bearing is the view's whole degrees.
        let may = self.answerable(unit).unwrap_or_default();
        let facing = frame
            .contacts
            .iter()
            .filter(|held| held.relation == Relation::Hostile && may.contains(&held.id))
            .min_by_key(|held| (held.range_cm, held.id))
            .map(|held| (held.bearing_mdeg / 1_000).saturating_mul(1_000));
        let objective = match (facing, standing) {
            (Some(bearing), Some(MindObjective::Screen { facing_mdeg }))
                if arc_between(bearing, facing_mdeg) <= SCREEN_HALF_ARC_MDEG =>
            {
                return;
            }
            (Some(bearing), _) => Objective::Screen {
                facing_mdeg: bearing % 360_000,
            },
            (None, Some(MindObjective::Hold)) => return,
            (None, _) => Objective::Hold,
        };
        self.lay_order(unit, objective);
    }

    /// **A house mind on `RETURN` answers on its way home** (this module's head, "flying a
    /// posture"; doc 47 unit CO.2, R-47-3): what the landing does with a think of the
    /// Standard Doctrine flying `unit` for an absent player on `RETURN` under the go-home
    /// order (`Arena::mind_land`, ahead of the intents' apply).
    ///
    /// The doctrine answers fire on its way home by a cornered stand: it stops (`Goal(Hold)`)
    /// and faces the unit that fired (`Heading`), fires, dwells, and stands again while that
    /// unit is in range. Under the go-home order a stop is not the player's word, so the
    /// landing lands the stop as the drive home: a `Goal(Hold)` becomes the goal the doctrine
    /// itself drives `Withdraw` to (its party's entry cell, read as the doctrine reads it), and
    /// so does the facing `Heading`, which would otherwise let the goal go. The lock and the
    /// shot land as thought, through the one gate at the shot. A ground shot needs no facing,
    /// so the unit fires as it drives.
    ///
    /// Nothing for any other unit or think: a pilot's, a sealed mind's (its helm is its own:
    /// a sealed script that ignores the order home is not driven home, Q-35-B4), a unit on
    /// `HOLD` or `GUARD`, a think that stops nothing, or a unit with no home cell. So no
    /// doctrine source, no `STOCK_VERSION` and no `MIND_ABI` moved.
    pub(crate) fn keep_going_home(
        &self,
        unit: ObjId,
        intents: Vec<crate::arena::intents::IntentKind>,
    ) -> Vec<crate::arena::intents::IntentKind> {
        use crate::arena::intents::{IntentKind, MotorGoal};
        use crate::arena::perception::MindObjective;
        let Some(state) = self.unit(unit).filter(|state| {
            state.absent && state.controller == Controller::Mind && state.seal.doctrine
        }) else {
            return intents;
        };
        let going_home = flown(state).is_return()
            && self
                .minds
                .get(&unit)
                .and_then(|row| row.order.as_ref())
                .is_some_and(|order| order.objective == MindObjective::Withdraw);
        if !going_home
            || !intents
                .iter()
                .any(|intent| matches!(intent, IntentKind::Goal(MotorGoal::Hold)))
        {
            return intents;
        }
        let Some(home) = self
            .definition
            .terrain
            .cell_of(&state.pose)
            .and_then(|here| {
                crate::arena::motor::entry_cell(
                    &self.definition,
                    state.team,
                    crate::arena::state::CostRow::Tracked,
                    here,
                )
            })
        else {
            return intents;
        };
        // One intent for one, so the landing's outcomes still line up with the think's own
        // intents (the author trace joins them by position): the facing `Heading` is landed
        // as the same drive, which the motor takes as the goal it already holds.
        let drive = IntentKind::Goal(MotorGoal::Cell {
            x: home.0,
            y: home.1,
        });
        intents
            .into_iter()
            .map(|intent| match intent {
                IntentKind::Goal(MotorGoal::Hold) | IntentKind::Heading { .. } => drive.clone(),
                intent => intent,
            })
            .collect()
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
        let guards = self.guards(state);
        let kin = if guards {
            self.fired_on_kin(state)
        } else {
            BTreeSet::new()
        };
        let facts = Facts {
            absent: true,
            holds_fire: self.holds_fire(unit),
            fired_on_by: &state.fired_on_by,
            guards,
            fired_on_kin: &kin,
        };
        permits(&facts, &Ask::Fire { at: target }).err()
    }

    /// **Whether `state` stands guard** (this module's head, "the leash"): flown on `GUARD`
    /// for an absent player, on a cell its mind has stood, no further than
    /// [`GUARD_LEASH_CELLS`] from it.
    fn guards(&self, state: &UnitState) -> bool {
        state.absent
            && flown(state) == Posture::Guard
            && state
                .stand_at
                .zip(self.definition.terrain.cell_of(&state.pose))
                .is_some_and(|(stood, here)| {
                    stood.0.abs_diff(here.0).max(stood.1.abs_diff(here.1)) <= GUARD_LEASH_CELLS
                })
    }

    /// **The units whose fire has arrived on another unit of `state`'s player** (this
    /// module's head; Q-45-8): every unit the host named the same player for, less this one,
    /// and never a unit of that player's own. Empty for a unit whose player the host has not
    /// named.
    fn fired_on_kin(&self, state: &UnitState) -> BTreeSet<ObjId> {
        let Some(player) = state.player else {
            return BTreeSet::new();
        };
        let own = |unit: &ObjId| {
            self.unit(*unit)
                .is_some_and(|other| other.player == Some(player))
        };
        self.units
            .iter()
            .filter(|other| other.id != state.id && other.player == Some(player))
            .flat_map(|other| other.fired_on_by.iter().copied())
            .filter(|by| !own(by))
            .collect()
    }

    /// **`shooter`'s fire has arrived on `target`** (this module's head), hit or miss: recorded
    /// on the target in a field whose end is its host's, and in a practice match on a target
    /// a pilot has named a player for by setting a posture (doc 45 unit AP.10: the practising
    /// hull and the player's other hulls, from the pulse the word lands), and nowhere else, so
    /// a match nobody practises in carries and hashes no byte of it. A unit's own fire on
    /// itself records nothing.
    pub(crate) fn note_fired_on(&mut self, shooter: ObjId, target: ObjId) {
        if shooter == target {
            return;
        }
        let host = self.end == crate::arena::party::EndCondition::Host;
        if let Some(state) = self.unit_mut(target)
            && (host || state.player.is_some())
        {
            state.fired_on_by.insert(shooter);
        }
    }

    /// **The contacts an absent player's `unit` may answer** (this module's head, "what the
    /// mind is shown"): its own contact ids for the units that have fired on it and, while it
    /// stands guard, for those that have fired on its player's other units; or none at all
    /// while a standing order holds its fire. `None` for a unit its player flies.
    #[must_use]
    pub(crate) fn answerable(&self, unit: ObjId) -> Option<BTreeSet<ContactId>> {
        let state = self.unit(unit).filter(|state| state.absent)?;
        if self.holds_fire(unit) {
            return Some(BTreeSet::new());
        }
        let kin = if self.guards(state) {
            self.fired_on_kin(state)
        } else {
            BTreeSet::new()
        };
        Some(
            state
                .fired_on_by
                .iter()
                .chain(&kin)
                .filter_map(|by| self.contacts.id_of(unit, *by))
                .collect(),
        )
    }
}

/// **The posture `state`'s mind is held to** ([`Arena::flown_posture`]).
pub(crate) fn flown(state: &UnitState) -> Posture {
    flown_as(
        state.posture,
        state.absent,
        !state.weapons.is_empty(),
        can_defend(state),
    )
}

/// **The posture a unit is flown on, from the word its player left** (this module's head,
/// "flying a posture"): the one rule the arena and a bubble both read (doc 45 unit AP.9;
/// `crate::space::backup`). `armed` is whether the unit carries anything to fire at all;
/// `can_defend` whether it can still defend itself (Q-45-9). Only a `GUARD` flown for an
/// absent player is read differently from the record.
#[must_use]
pub const fn flown_as(posture: Posture, absent: bool, armed: bool, can_defend: bool) -> Posture {
    match posture {
        // Doc 45 unit AP.7 (Q-45-10): nothing to fire, so no guard to stand or to lose.
        Posture::Guard if absent && !armed => Posture::Hold,
        Posture::Guard if absent && !can_defend => Posture::Return,
        posture => posture,
    }
}

/// **Whether `state` can still defend itself** (Q-45-9): not a wreck, its motion not
/// disabled, and a fire channel with a round left.
fn can_defend(state: &UnitState) -> bool {
    !state.limits.wreck
        && !state.limits.motion_disabled
        && !state.disabled
        && state
            .weapons
            .iter()
            .any(|weapon| state.weapon_ammo(&weapon.id) > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(absent: bool, holds_fire: bool, by: &BTreeSet<u32>) -> Facts<'_, u32> {
        Facts {
            absent,
            holds_fire,
            fired_on_by: by,
            guards: false,
            fired_on_kin: by,
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
            "a standing order's hold holds through an attack"
        );
    }

    /// **Doc 45 unit AP.6: the one widening** (Q-45-8). A unit that stands guard may also
    /// answer a unit that fired on another unit of its player, and nothing else moves: one
    /// that is not standing guard reads the second set for nothing, a unit in neither set is
    /// still a first shot, a hold still holds, and mining and trading are still refused.
    #[test]
    fn ap6_a_guard_answers_for_its_players_other_units_and_for_nobody_else() {
        let (on_it, on_kin) = (BTreeSet::from([7_u32]), BTreeSet::from([8_u32]));
        let with = |guards: bool, holds_fire: bool| Facts {
            absent: true,
            holds_fire,
            fired_on_by: &on_it,
            guards,
            fired_on_kin: &on_kin,
        };
        let guard = with(true, false);
        assert_eq!(permits(&guard, &Ask::Fire { at: 7 }), Ok(()));
        assert_eq!(
            permits(&guard, &Ask::Fire { at: 8 }),
            Ok(()),
            "the unit that fired on its player's other unit"
        );
        assert_eq!(
            permits(&guard, &Ask::Fire { at: 9 }),
            Err(Denied::FiredFirst),
            "a unit that fired on a stranger, or on nobody"
        );
        assert_eq!(permits(&guard, &Ask::Mine), Err(Denied::Mining));
        assert_eq!(permits(&guard, &Ask::Trade), Err(Denied::Trading));
        // HOLD and RETURN, and a guard off its leash or out of rounds: for itself alone.
        let hold = with(false, false);
        assert_eq!(permits(&hold, &Ask::Fire { at: 7 }), Ok(()));
        assert_eq!(
            permits(&hold, &Ask::Fire { at: 8 }),
            Err(Denied::FiredFirst),
            "only a guard answers for a neighbour"
        );
        // A standing order's hold holds for a guard too.
        for at in [7, 8] {
            assert_eq!(
                permits(&with(true, true), &Ask::Fire { at }),
                Err(Denied::HeldFire)
            );
        }
        // A unit its player flies is not asked, whatever it holds.
        let present = Facts {
            absent: false,
            ..guard
        };
        assert_eq!(permits(&present, &Ask::Fire { at: 9 }), Ok(()));
    }
}
