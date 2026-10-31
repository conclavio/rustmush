//! **The catch-up report** (16b cut Y.4; 16c `N4`; doc 07 §1.3 step 5, doc 16
//! §3.6 "What a pilot sees") — what a pilot who takes their tank back is told
//! about the time they could not see it.
//!
//! Doc 07 §1.3's fifth step: *on reclaim, deliver a structured catch-up report
//! covering elapsed ticks, movement, damage, contacts, ammunition/heat changes,
//! actions taken, and the current plan*. Cut Y.3 lets a pilot take the tank
//! back and says only that it is theirs again where it stands; this module is
//! the report, as two views and two functions.
//!
//! # The baseline is the moment the pilot lost sight
//!
//! [`ReleaseSnapshot`] is taken by `apply_control` on the **first** `Linkdead`
//! or `Released` that names a pilot — `entry().or_insert` on the unit, so the
//! three channels of one drop take one snapshot and the `Released` that ends a
//! 300 s grace does not replace the `Linkdead` that began it. Taken at the
//! `Released` alone it would silently absorb the whole grace, during which the
//! tank drove on at its last speed and fired nothing (doc 16 §3.8
//! "Disconnects"). A `Live` that finds the pilot still on the frame roster —
//! K4's revival inside the grace — removes it and builds nothing, so a stale
//! baseline cannot outlive a revival and surface on a later drop. The `Live`
//! that re-seats a dropped pilot ([`crate::arena::state::Arena::seat_pilot`]
//! answering `true`) consumes it into a [`Catchup`], which `Arena::frames` puts
//! on that pilot's next frame once ([`crate::arena::step::Frame::catchup`]).
//!
//! # "Now" is the re-seat, not the frame
//!
//! Every `_now` figure of a [`Catchup`] — the cell, the power, the wreck, the
//! contacts, the sections, the rounds — is read where `apply_control`'s `Live`
//! arm builds it: pulse phase 1, **before** that pulse's motion. The frame that
//! carries the report is built after the motion (and, on a lagging world, after
//! whatever pulses `merge_held` folded in), so a tank still under way at the
//! reclaim — the machine was withdrawing it — can stand a cell further on in
//! the carrying frame than in its report. That is deliberate: the re-seat is a
//! world-scripted beat at one arena millisecond, so the report is one set of
//! figures at every cadence (16b cut Y.4's cross-tempo assertion), where the
//! carrying frame's own cell is a function of the pulse length. The report says
//! what happened up to the moment the pilot had the tank back; the frame
//! beside it says where the tank is now.
//!
//! # Two views, and why neither is hashed
//!
//! [`crate::arena::state::Arena::at_release`] and
//! [`crate::arena::state::Arena::pending_catchup`] are **views**, on the
//! argument `last_frames` makes: each is a fact about what a pilot will be told,
//! not about the match, and folding either into
//! [`crate::arena::hash::state_hash`] would make the hash a function of who was
//! connected. Nothing that steers reads them — no mind's think, no sensor pass,
//! no resolver — so they cannot move an outcome either, and the cross-tempo
//! property is untouched (16b cut Y.4's assertion is that the report is the
//! same at every cadence, and the hash sequence is the same with it or without
//! it). Both are dropped with the arena and bounded by its units.
//!
//! # The observer rule
//!
//! Everything in the report is the pilot's own tank — its cell, its power, its
//! sections, its rounds, its machine's state — which is physical knowledge and
//! outside the observer rule (doc 16 §1, §3.6), **except the contacts**. Those
//! are `Arena::frame_contacts`' own rows as `(id, designator)`: the filter the
//! frame applies (a contact whose target has left the match is no contact the
//! frame ever showed) and the names the frame prints. A lost contact is named as
//! it read at the snapshot — a name the pilot's own frame carried then. Nothing
//! about a unit the platform never held can reach the report, because nothing
//! but that store is read.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::motor::MotorRow;
use crate::arena::state::{Arena, ContactId, Controller, Designator, Power};

/// **One section as its own crew reads it** (16b cut Y.4): the M32 condition
/// word, the facets' armour summed, the internals — so a tank shot up short of
/// losing a section still reports the damage (the word only ever moves to
/// `destroyed`).
///
/// `internals` is `0` for a section whose content declares no internals cell;
/// two readings of such a section compare equal, so it is never listed for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionWords {
    /// The section's opaque condition, as M32 holds it (`intact`, `destroyed`,
    /// `wreck`).
    pub condition: SmolStr,
    /// The current armour of every facet of the section, summed.
    pub armour: i64,
    /// The section's current internals.
    pub internals: i64,
}

/// **What a platform held and was when its pilot lost sight of it** (16b cut
/// Y.4; doc 07 §1.3 step 5) — the baseline a [`Catchup`] is measured against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSnapshot {
    /// The arena millisecond of the control that took it.
    pub at_ms: i64,
    /// The cell the tank stood in, or `None` off the window.
    pub cell: Option<(u16, u16)>,
    /// Its power.
    pub power: Power,
    /// What the platform held, as its frame's own rows: the row key and the
    /// designator the frame printed for it, in contact-id order.
    pub contacts: Vec<(ContactId, Designator)>,
    /// Each section's words, in `Arena::frame_sections` order (section-id).
    pub sections: Vec<(SmolStr, SectionWords)>,
    /// Each weapon's rounds left, `(weapon id, rounds)`, in component order —
    /// the figures `Arena::frame_weapons` prints.
    pub rounds: Vec<(SmolStr, u32)>,
}

/// **What the machine was doing when the pilot took the tank back** (16b cut
/// Y.4) — the machine's own words, read before `set_controller` suspends its
/// row: a mind's since 17b cut M4.3 ([`MachineWords::of_mind`]; the reflex
/// row's until 17b cut M4.6 deleted the reflex controller and
/// `MachineWords::of` with it). Its three fields and their words are the
/// wire's (`catchup.machine`), unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineWords {
    /// [`crate::arena::doctrine::MachineState::as_str`]'s word.
    pub state: SmolStr,
    /// The cell it was making for.
    pub goal: Option<(u16, u16)>,
    /// The standing order's objective word
    /// ([`crate::deliberation::Objective::as_str`]), or `None` under doctrine.
    pub objective: Option<SmolStr>,
}

impl MachineWords {
    /// **The words of a mind's hull** (17b cut M4.3): its doctrine state
    /// (`Arena::machine_state`, which reads `unknown` for a mind not sealed
    /// on Standard Doctrine since 17b cut M4.6), its motor row's goal — where
    /// the goal has lived since 17b cut M1a.1, `None` when the table holds
    /// none — and its held order's objective word: the same three a reflex
    /// row gave until 17b cut M4.6, so the sentence a reclaiming pilot reads
    /// kept its words when the machine under it changed.
    #[must_use]
    pub fn of_mind(
        state: crate::arena::doctrine::MachineState,
        order: Option<&crate::arena::perception::FrameOrder>,
        motor: Option<&MotorRow>,
    ) -> MachineWords {
        MachineWords {
            state: SmolStr::new_static(state.as_str()),
            goal: motor.and_then(|motor| motor.goal),
            objective: order.map(|order| SmolStr::new_static(order.objective.as_str())),
        }
    }
}

/// **The report** (16b cut Y.4; doc 07 §1.3 step 5), built once at the `Live`
/// that re-seats the pilot and carried once on their next frame.
///
/// Every list is **capped** and the overflow counted beside it: combat-scale
/// platforms carry up to ten sections and ten weapons, a long drop can see a
/// field of contacts come and go, and neither panel's notices ring caps a line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catchup {
    /// Arena milliseconds from the snapshot to the re-seat.
    pub away_ms: i64,
    /// The cell at the snapshot.
    pub cell_then: Option<(u16, u16)>,
    /// The cell at the re-seat — before the reclaiming pulse's motion, so a
    /// tank under way may read a cell further on in the carrying frame (see
    /// the module's "Now is the re-seat").
    pub cell_now: Option<(u16, u16)>,
    /// Power at the snapshot.
    pub power_then: Power,
    /// Power at the re-seat.
    pub power_now: Power,
    /// The tank is a wreck at the re-seat.
    pub wreck: bool,
    /// Contacts held at the re-seat and not then, sorted by designator, at most
    /// [`MAX_CATCHUP_CONTACTS`].
    pub contacts_new: Vec<Designator>,
    /// Contacts held then and not at the re-seat, named as they read then, sorted by
    /// designator, at most [`MAX_CATCHUP_CONTACTS`].
    pub contacts_lost: Vec<Designator>,
    /// How many contacts the two caps left out, both lists together.
    pub contacts_more: u32,
    /// Each section whose word changed or whose armour or internals fell, as
    /// `(id, then, now)` in section order, at most [`MAX_CATCHUP_SECTIONS`].
    pub sections: Vec<(SmolStr, SectionWords, SectionWords)>,
    /// How many such sections the cap left out.
    pub sections_more: u32,
    /// Each weapon whose rounds fell, as `(weapon id, rounds spent)` in
    /// component order, at most [`MAX_CATCHUP_WEAPONS`].
    pub rounds_spent: Vec<(SmolStr, u32)>,
    /// How many such weapons the cap left out.
    pub rounds_more: u32,
    /// What the machine was doing, or `None` when no mind flew the tank (no
    /// reflex row, until 17b cut M4.6).
    pub machine: Option<MachineWords>,
}

/// The most new or lost contacts the report names, each list.
pub const MAX_CATCHUP_CONTACTS: usize = 12;
/// The most changed sections the report names.
pub const MAX_CATCHUP_SECTIONS: usize = 6;
/// The most weapons the report names rounds spent for.
pub const MAX_CATCHUP_WEAPONS: usize = 6;

/// **The order a contact list is said in** (16b cut Y.4): told-apart tanks
/// first, by side and slot (`RA`, `BA`, `BB`), then the provisional numbers
/// ascending (`X2`, `X10`). The contact-id order the rows arrive in is a seeded
/// shuffle and reads as random; this reads as the frame's own names do.
fn said_order(designator: &Designator) -> (bool, Designator) {
    (
        matches!(designator, Designator::Provisional(_)),
        *designator,
    )
}

/// Cut `list` to `max`, answering what was kept and how many went.
fn capped<T>(mut list: Vec<T>, max: usize) -> (Vec<T>, u32) {
    let more = list.len().saturating_sub(max);
    list.truncate(max);
    (list, u32::try_from(more).unwrap_or(u32::MAX))
}

impl Arena {
    /// **What `unit` holds and is, now** (16b cut Y.4), as the baseline of a
    /// later [`Catchup`] — `None` for a unit this arena does not hold.
    ///
    /// Four reads the frame itself makes, so the report and the frame cannot
    /// disagree: `frame_contacts`' rows, `frame_sections`' cells, the weapons'
    /// rounds and the window's `cell_of`.
    #[must_use]
    pub(crate) fn snapshot_at_release(&self, unit: ObjId) -> Option<ReleaseSnapshot> {
        let state = self.unit(unit)?;
        Some(ReleaseSnapshot {
            at_ms: self.now_ms,
            cell: self.definition.terrain.cell_of(&state.pose),
            power: state.power,
            contacts: self.catchup_contacts(unit),
            sections: self.section_words(unit),
            rounds: self.rounds_left(unit),
        })
    }

    /// **The report for `unit` against `then`** (16b cut Y.4), with the machine's
    /// words the caller read before the mind's row was suspended.
    ///
    /// Contacts are diffed by [`ContactId`] — new is held now and not then, lost
    /// is held then and not now, named as it read then — each list sorted by
    /// designator and cut at [`MAX_CATCHUP_CONTACTS`]. A section is listed where
    /// its condition word changed or its armour or internals fell; a weapon where
    /// its rounds fell. The rounds now are read off `UnitState::ammo` by the
    /// snapshot's own weapon ids, not off the weapon table: a turret shot off
    /// takes its `weapons` rows and leaves its rounds on the record (16b cut
    /// F.1), so what it fired before it went is still said. A unit the arena no
    /// longer holds reads as nothing now: no cell, power off, nothing held,
    /// nothing spent.
    #[must_use]
    pub(crate) fn catchup_since(
        &self,
        unit: ObjId,
        then: &ReleaseSnapshot,
        machine: Option<MachineWords>,
    ) -> Catchup {
        let state = self.unit(unit);
        let now_contacts = self.catchup_contacts(unit);
        let mut new: Vec<Designator> = now_contacts
            .iter()
            .filter(|(id, _)| !then.contacts.iter().any(|(held, _)| held == id))
            .map(|(_, designator)| *designator)
            .collect();
        let mut lost: Vec<Designator> = then
            .contacts
            .iter()
            .filter(|(id, _)| !now_contacts.iter().any(|(held, _)| held == id))
            .map(|(_, designator)| *designator)
            .collect();
        new.sort_by_key(said_order);
        lost.sort_by_key(said_order);
        let (contacts_new, new_more) = capped(new, MAX_CATCHUP_CONTACTS);
        let (contacts_lost, lost_more) = capped(lost, MAX_CATCHUP_CONTACTS);
        let changed: Vec<(SmolStr, SectionWords, SectionWords)> = self
            .section_words(unit)
            .into_iter()
            .filter_map(|(id, now)| {
                let (_, was) = then.sections.iter().find(|(held, _)| *held == id)?;
                let moved = was.condition != now.condition
                    || now.armour < was.armour
                    || now.internals < was.internals;
                moved.then(|| (id, was.clone(), now))
            })
            .collect();
        let (sections, sections_more) = capped(changed, MAX_CATCHUP_SECTIONS);
        let spent: Vec<(SmolStr, u32)> = state.map_or_else(Vec::new, |state| {
            then.rounds
                .iter()
                .filter_map(|(weapon, was)| {
                    let now = state.weapon_ammo(weapon);
                    (now < *was).then(|| (weapon.clone(), was.saturating_sub(now)))
                })
                .collect()
        });
        let (rounds_spent, rounds_more) = capped(spent, MAX_CATCHUP_WEAPONS);
        Catchup {
            away_ms: self.now_ms.saturating_sub(then.at_ms),
            cell_then: then.cell,
            cell_now: state.and_then(|state| self.definition.terrain.cell_of(&state.pose)),
            power_then: then.power,
            power_now: state.map_or(Power::Off, |state| state.power),
            wreck: state.is_some_and(|state| state.limits.wreck),
            contacts_new,
            contacts_lost,
            contacts_more: new_more.saturating_add(lost_more),
            sections,
            sections_more,
            rounds_spent,
            rounds_more,
            machine,
        }
    }

    /// **The pilot of `unit` lost sight of it** (16b cut Y.4): `apply_control`'s
    /// `Linkdead` and `Released` arms, for a control that names a pilot. The
    /// first one wins — three channels of one drop, and the `Released` that ends
    /// the grace after the `Linkdead` that began it, take one snapshot.
    pub(crate) fn note_sight_lost(&mut self, unit: ObjId) {
        if self.at_release.contains_key(&unit) {
            return;
        }
        if let Some(snapshot) = self.snapshot_at_release(unit) {
            self.at_release.insert(unit, snapshot);
        }
    }

    /// **A `Live` named `pilot` in `unit`** (16b cut Y.4): `apply_control`'s
    /// `Live` arm, after [`Arena::seat_pilot`] and **before** the tank is handed
    /// back to `Controller::Pilot`, which is what keeps the machine's row
    /// readable here.
    ///
    /// `reseated` is `seat_pilot`'s answer. `true` — a dropped pilot back on the
    /// roster — turns the unit's snapshot into the pilot's pending report and
    /// removes it. `false` — a revival inside the grace, or a pilot who never
    /// left — removes the snapshot and builds nothing: the pilot never lost the
    /// frames, and a baseline kept past a revival would be the one a later drop
    /// found and reported from.
    pub(crate) fn settle_catchup(&mut self, pilot: ObjId, unit: ObjId, reseated: bool) {
        let Some(then) = self.at_release.remove(&unit) else {
            return;
        };
        if !reseated {
            return;
        }
        // **17b cut M4.3**: the words are a mind's — off its doctrine state
        // (`Arena::machine_state`: `unknown` for a mind not sealed on Standard
        // Doctrine, 17b cut M4.6), its held order and its motor row
        // (`MachineWords::of_mind`), read before the `Live` suspends the row;
        // a pilot's hull none (the reflex's row was read here until 17b cut
        // M4.6 deleted it).
        let machine = match self.unit(unit).map(|state| state.controller) {
            Some(Controller::Mind) => self.machine_state(unit).map(|state| {
                MachineWords::of_mind(
                    state,
                    self.minds.get(&unit).and_then(|row| row.order.as_ref()),
                    self.motors.row(unit),
                )
            }),
            _ => None,
        };
        let catchup = self.catchup_since(unit, &then, machine);
        self.pending_catchup.insert(pilot, catchup);
    }

    /// The platform's contacts as its frame names them: `frame_contacts`' rows
    /// as `(id, designator)`, in contact-id order.
    fn catchup_contacts(&self, unit: ObjId) -> Vec<(ContactId, Designator)> {
        self.frame_contacts(unit)
            .into_iter()
            .map(|row| (row.id, row.designator))
            .collect()
    }

    /// Each section's words, off `frame_sections`' own projection of the cells.
    fn section_words(&self, unit: ObjId) -> Vec<(SmolStr, SectionWords)> {
        self.frame_sections(unit)
            .into_iter()
            .map(|section| {
                let armour = section
                    .facets
                    .iter()
                    .fold(0_i64, |sum, facet| sum.saturating_add(facet.armor));
                (
                    section.id,
                    SectionWords {
                        condition: section.condition,
                        armour,
                        internals: section.internals.map_or(0, |(current, _)| current),
                    },
                )
            })
            .collect()
    }

    /// Each weapon's rounds left, in component order.
    fn rounds_left(&self, unit: ObjId) -> Vec<(SmolStr, u32)> {
        self.unit(unit).map_or_else(Vec::new, |state| {
            state
                .weapons
                .iter()
                .map(|weapon| (weapon.id.clone(), state.weapon_ammo(&weapon.id)))
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::intents::{Control, ControlState, apply_control};
    use crate::arena::rng::draw_contact_id;
    use crate::arena::state::{Channel, Classification, Pilot, SeenPose, ShadowContact, PartyId};
    use crate::arena::test_support::{line_tank, pose, tank};
    use mush_tactics::{CellAddr, CellId, ConditionId, FacetId, SectionId, UnitMutationOp};

    const RED: ObjId = ObjId(1);
    const PILOT: ObjId = ObjId(900);

    /// A stock tank for red, and `blues` plain blue tanks to hold, one a cell
    /// along the rows of the 8 by 8 window from `(2, 0)`.
    fn arena(blues: u32) -> Arena {
        let mut units = vec![line_tank(RED, PartyId::RED, pose(500, 500))];
        for n in 0..blues {
            let x = 2 + i64::from(n % 6);
            let y = i64::from(n / 6);
            units.push(tank(
                ObjId(10 + n),
                PartyId::BLUE,
                pose(x * 1_000 + 500, y * 1_000 + 500),
            ));
        }
        let custody = crate::arena::test_support::custody(
            units,
            vec![Pilot {
                unit: RED,
                pilot: PILOT,
                station: SmolStr::new("cockpit"),
                epoch: 1,
            }],
        );
        Arena::new(
            custody,
            &crate::config::ArenaConfig::default(),
            &crate::config::MatchConfig::default(),
            &crate::config::SensorsConfig::default(),
        )
        .expect("the catch-up arena")
    }

    /// Red holds `target`, told apart (so it reads by its callsign), or not
    /// (so it reads by a provisional number red's book mints for it).
    fn hold(arena: &mut Arena, target: ObjId, classified: bool) -> ContactId {
        let id = draw_contact_id(arena.seed, RED, target);
        let class = if classified {
            Classification::Classified { party: PartyId::BLUE }
        } else {
            arena.designators.assign(RED, target);
            Classification::Unknown
        };
        let seen = arena.unit(target).map(SeenPose::of).unwrap_or_default();
        arena
            .contacts
            .insert(RED, id, ShadowContact::sighted(target, class, 0, seen));
        id
    }

    fn drop_row(arena: &mut Arena, target: ObjId) {
        if let Some(id) = arena.contacts.id_of(RED, target) {
            arena.contacts.remove(RED, id);
        }
    }

    /// Apply one plan to red's cells, through the one assignment site.
    fn hurt(arena: &mut Arena, ops: Vec<UnitMutationOp>) {
        let unit = arena
            .unit(RED)
            .map(|state| state.unit.clone())
            .expect("red");
        let plan = mush_tactics::UnitMutationPlan::new(ops, None, None).expect("a plan");
        let (post, _) = mush_tactics::apply(&unit, &plan, 0).expect("the plan applies");
        arena.set_unit(RED, post, true);
    }

    fn armour(section: &str, facet: &str, delta: i64) -> UnitMutationOp {
        UnitMutationOp::AdjustCell {
            addr: CellAddr::Facet {
                section: SectionId::new(section).expect("a section"),
                facet: FacetId::new(facet).expect("a facet"),
                cell: CellId::new("armor").expect("a cell"),
            },
            delta,
        }
    }

    fn internals(section: &str, delta: i64) -> UnitMutationOp {
        UnitMutationOp::AdjustCell {
            addr: CellAddr::Section {
                section: SectionId::new(section).expect("a section"),
                cell: CellId::new("internals").expect("a cell"),
            },
            delta,
        }
    }

    /// The callsign the arena drew for the `n`-th blue (`#9 + n`): since doc 32
    /// cut CF.0 a unit letter is drawn, not spelled from a slot.
    fn slot(arena: &Arena, n: u32) -> Designator {
        arena
            .unit(ObjId(9 + n))
            .map(|unit| unit.callsign)
            .expect("a blue")
    }

    #[test]
    fn a_unit_the_arena_does_not_hold_has_no_snapshot() {
        let arena = arena(1);
        assert_eq!(arena.snapshot_at_release(ObjId(999)), None);
        assert!(arena.snapshot_at_release(RED).is_some());
    }

    #[test]
    fn contacts_new_and_lost_are_diffed_by_id_and_said_in_designator_order() {
        let mut arena = arena(4);
        // Then: BA and BB (told apart) and X1 (red's first number, on #13).
        hold(&mut arena, ObjId(10), true);
        hold(&mut arena, ObjId(11), true);
        hold(&mut arena, ObjId(13), false);
        let then = arena.snapshot_at_release(RED).expect("red");
        assert_eq!(then.contacts.len(), 3);
        // Now: BB still, BA gone, X1 gone, BC new.
        drop_row(&mut arena, ObjId(10));
        drop_row(&mut arena, ObjId(13));
        hold(&mut arena, ObjId(12), true);
        arena.now_ms = 42_500;
        let report = arena.catchup_since(RED, &then, None);
        assert_eq!(report.away_ms, 42_500);
        assert_eq!(report.contacts_new, vec![slot(&arena, 3)]);
        assert_eq!(
            report.contacts_lost,
            vec![slot(&arena, 1), Designator::Provisional(1)],
            "the slot names first, then the numbers"
        );
        assert_eq!(report.contacts_more, 0);
        assert_eq!(report.machine, None);
        // Nothing moved on the tank itself.
        assert!(report.sections.is_empty());
        assert!(report.rounds_spent.is_empty());
        assert_eq!(report.cell_then, report.cell_now);
        assert!(!report.wreck);
    }

    /// **The observer rule** (doc 16 §1, §3.6): the report reads the frame's own
    /// scope and nothing wider. A blue the platform never held is named by
    /// neither list; a held row whose target has left the match — which the
    /// frame drops, and which `designator_for` would print as some `X` — is
    /// neither baseline nor news, whether it left before the snapshot or after.
    #[test]
    fn a_unit_never_held_and_a_row_whose_target_left_are_never_said() {
        let mut arena = arena(4);
        // Then: BA held; #12 held as a number and gone from the match.
        hold(&mut arena, ObjId(10), true);
        hold(&mut arena, ObjId(12), false);
        arena.units.retain(|unit| unit.id != ObjId(12));
        let then = arena.snapshot_at_release(RED).expect("red");
        assert_eq!(
            then.contacts
                .iter()
                .map(|(_, designator)| *designator)
                .collect::<Vec<_>>(),
            vec![slot(&arena, 1)],
            "the snapshot is the frame's scope: {:?}",
            then.contacts
        );
        // Now: #12's row goes (a reader of the raw store would call it lost);
        // #13 is held as a number and leaves the match (a reader of the raw
        // store would call it new). #11 is never held at all.
        drop_row(&mut arena, ObjId(12));
        hold(&mut arena, ObjId(13), false);
        arena.units.retain(|unit| unit.id != ObjId(13));
        arena.now_ms = 3_000;
        let report = arena.catchup_since(RED, &then, None);
        assert!(report.contacts_new.is_empty(), "{:?}", report.contacts_new);
        assert!(
            report.contacts_lost.is_empty(),
            "{:?}",
            report.contacts_lost
        );
        assert_eq!(report.contacts_more, 0);
        // And the one blue the platform does hold is still held, so neither
        // list is empty for want of a scope at all.
        assert_eq!(
            arena
                .frame_contacts(RED)
                .iter()
                .map(|row| row.designator)
                .collect::<Vec<_>>(),
            vec![slot(&arena, 1)]
        );
    }

    #[test]
    fn a_long_list_is_capped_at_twelve_and_the_rest_counted() {
        let mut arena = arena(16);
        let then = arena.snapshot_at_release(RED).expect("red");
        assert!(then.contacts.is_empty());
        for n in 0..16 {
            hold(&mut arena, ObjId(10 + n), true);
        }
        let report = arena.catchup_since(RED, &then, None);
        assert_eq!(report.contacts_new.len(), MAX_CATCHUP_CONTACTS);
        // Said in designator order: the twelve least of the sixteen drawn.
        let mut drawn: Vec<Designator> = (1..=16).map(|n| slot(&arena, n)).collect();
        drawn.sort();
        drawn.truncate(MAX_CATCHUP_CONTACTS);
        assert_eq!(report.contacts_new, drawn);
        assert_eq!(report.contacts_more, 4);
        assert!(report.contacts_lost.is_empty());
    }

    #[test]
    fn a_section_is_listed_for_a_word_changed_or_armour_or_internals_fallen() {
        let mut arena = arena(0);
        let then = arena.snapshot_at_release(RED).expect("red");
        assert_eq!(
            then.sections
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            ["engine", "hull", "left_track", "right_track", "turret"]
        );
        // The hull's front loses 150 armour and nothing else; the left track
        // is destroyed outright; the turret loses internals only; the engine
        // and the right track are untouched.
        hurt(
            &mut arena,
            vec![
                armour("hull", "front", -150),
                armour("left_track", "left", -120),
                internals("left_track", -100),
                UnitMutationOp::SetSectionCondition {
                    section: SectionId::new("left_track").expect("a section"),
                    condition: ConditionId::new("destroyed").expect("a condition"),
                },
                internals("turret", -40),
            ],
        );
        let report = arena.catchup_since(RED, &then, None);
        let listed: Vec<(&str, &SectionWords, &SectionWords)> = report
            .sections
            .iter()
            .map(|(id, was, now)| (id.as_str(), was, now))
            .collect();
        assert_eq!(listed.len(), 3, "{listed:?}");
        let (id, was, now) = listed[0];
        assert_eq!(id, "hull");
        assert_eq!((was.armour, now.armour), (1_150, 1_000));
        assert_eq!(was.condition, now.condition, "the word did not move");
        assert_eq!(was.internals, now.internals);
        let (id, was, now) = listed[1];
        assert_eq!(id, "left_track");
        assert_eq!(
            (was.condition.as_str(), now.condition.as_str()),
            ("intact", "destroyed")
        );
        let (id, was, now) = listed[2];
        assert_eq!(id, "turret");
        assert_eq!((was.internals, now.internals), (150, 110));
        assert_eq!(was.armour, now.armour);
        assert_eq!(report.sections_more, 0);
    }

    #[test]
    fn rounds_spent_are_listed_and_a_full_rack_is_not() {
        let mut arena = arena(0);
        let then = arena.snapshot_at_release(RED).expect("red");
        assert_eq!(then.rounds, vec![(SmolStr::new("cannon"), 40)]);
        let report = arena.catchup_since(RED, &then, None);
        assert!(report.rounds_spent.is_empty(), "a full rack spent nothing");
        if let Some(state) = arena.unit_mut(RED) {
            state.ammo.insert(SmolStr::new("cannon"), 37);
        }
        let report = arena.catchup_since(RED, &then, None);
        assert_eq!(report.rounds_spent, vec![(SmolStr::new("cannon"), 3)]);
        assert_eq!(report.rounds_more, 0);
    }

    /// A weapon shot off after it fired still says what it spent: the turret's
    /// loss takes the weapon row and leaves the rounds on the record (16b cut
    /// F.1), and the report reads the record.
    #[test]
    fn rounds_fired_by_a_weapon_later_shot_off_are_still_spent() {
        let mut arena = arena(0);
        let then = arena.snapshot_at_release(RED).expect("red");
        assert_eq!(then.rounds, vec![(SmolStr::new("cannon"), 40)]);
        if let Some(state) = arena.unit_mut(RED) {
            state.ammo.insert(SmolStr::new("cannon"), 30);
        }
        hurt(
            &mut arena,
            vec![UnitMutationOp::SetSectionCondition {
                section: SectionId::new("turret").expect("a section"),
                condition: ConditionId::new("destroyed").expect("a condition"),
            }],
        );
        let mut journal = Vec::new();
        arena.apply_consequence(
            RED,
            &crate::arena::resolution::Consequence::RemoveWeapons {
                section: SmolStr::new("turret"),
            },
            0,
            None,
            &mut journal,
        );
        assert!(
            arena
                .unit(RED)
                .is_some_and(|state| state.weapons.is_empty()),
            "the turret took its gun with it"
        );
        let report = arena.catchup_since(RED, &then, None);
        assert_eq!(
            report.rounds_spent,
            vec![(SmolStr::new("cannon"), 10)],
            "the ten rounds fired before the turret went"
        );
        assert_eq!(
            report
                .sections
                .iter()
                .map(|(id, was, now)| (id.as_str(), was.condition.as_str(), now.condition.as_str()))
                .collect::<Vec<_>>(),
            [("turret", "intact", "destroyed")]
        );
    }

    /// A combat-scale platform: eight sections `s0`..`s7`, each one facet of 10
    /// armour and 10 internals, and eight guns `w0`..`w7` of 40 rounds.
    fn big(id: ObjId) -> crate::arena::state::CustodyUnit {
        use crate::arena::test_support::{cannon, dref, rules};
        use mush_tactics::{CellSpec, FacetSpec, SectionSpec, UnitSpec};
        let cell = |name: &str| CellSpec {
            id: CellId::new(name).expect("a cell"),
            min: 0,
            max: 10,
            init: 10,
        };
        let spec = UnitSpec {
            definition: dref("unit", "tacsim.unit.big"),
            sections: (0..8)
                .map(|n| SectionSpec {
                    id: SectionId::new(format!("s{n}")).expect("a section"),
                    condition: ConditionId::new("intact").expect("a condition"),
                    facets: vec![FacetSpec {
                        id: FacetId::new("front").expect("a facet"),
                        cells: vec![cell("armor")],
                    }],
                    slots: Vec::new(),
                    cells: vec![cell("internals")],
                })
                .collect(),
            unit_cells: Vec::new(),
            components: Vec::new(),
            capabilities: Vec::new(),
            mobility: None,
            rules: rules(),
        };
        crate::arena::state::CustodyUnit {
            weapons: (0..8)
                .map(|n| crate::arena::state::WeaponSummary {
                    id: SmolStr::new(format!("w{n}")),
                    ..cannon()
                })
                .collect(),
            unit: mush_tactics::TacticalUnit::from_spec(&spec, id).expect("the big unit"),
            ..tank(id, PartyId::RED, pose(500, 500))
        }
        .sealed()
    }

    #[test]
    fn sections_and_weapons_are_capped_at_six() {
        let mut arena = crate::arena::test_support::arena_of(vec![big(RED)]);
        let then = arena.snapshot_at_release(RED).expect("red");
        assert_eq!((then.sections.len(), then.rounds.len()), (8, 8));
        // Every section loses armour, the last one internals too; every gun
        // spends rounds.
        let mut ops: Vec<UnitMutationOp> = (0..8)
            .map(|n| armour(&format!("s{n}"), "front", -3))
            .collect();
        ops.push(internals("s7", -2));
        hurt(&mut arena, ops);
        if let Some(state) = arena.unit_mut(RED) {
            for n in 0..8_u32 {
                state
                    .ammo
                    .insert(SmolStr::new(format!("w{n}")), 40 - (n + 1));
            }
        }
        let report = arena.catchup_since(RED, &then, None);
        assert_eq!(report.sections.len(), MAX_CATCHUP_SECTIONS);
        assert_eq!(report.sections_more, 2);
        assert_eq!(
            report
                .sections
                .first()
                .map(|(id, was, now)| (id.as_str(), was.armour, now.armour)),
            Some(("s0", 10, 7)),
            "in section order"
        );
        assert_eq!(report.rounds_spent.len(), MAX_CATCHUP_WEAPONS);
        assert_eq!(report.rounds_more, 2);
        assert_eq!(
            report.rounds_spent,
            (0..6_u32)
                .map(|n| (SmolStr::new(format!("w{n}")), n + 1))
                .collect::<Vec<_>>(),
            "in component order, each the rounds it spent"
        );
    }

    #[test]
    fn an_unchanged_position_and_a_wreck_read_as_they_are() {
        let mut arena = arena(0);
        if let Some(state) = arena.unit_mut(RED) {
            state.power = Power::Online;
        }
        let then = arena.snapshot_at_release(RED).expect("red");
        if let Some(state) = arena.unit_mut(RED) {
            state.limits.wreck = true;
            state.power = Power::Off;
        }
        let report = arena.catchup_since(RED, &then, None);
        assert_eq!(report.cell_then, Some((0, 0)));
        assert_eq!(report.cell_now, Some((0, 0)));
        assert_eq!(
            (report.power_then, report.power_now),
            (Power::Online, Power::Off)
        );
        assert!(report.wreck);
    }

    fn every_channel(epoch: u64, state: ControlState) -> Vec<Control> {
        Channel::ALL
            .iter()
            .map(|channel| Control {
                unit: RED,
                channel: *channel,
                epoch,
                state,
                pilot: Some(PILOT),
            })
            .collect()
    }

    /// The whole life of one report through `apply_control`: the `Linkdead`
    /// takes the snapshot and the `Released` keeps it; the machine's words are
    /// read at the `Live` before the row goes dormant; the report is the
    /// pilot's and the snapshot is spent. **17b cut M4.3**: the machine is a
    /// mind seeded `Withdraw`, read through `Arena::machine_state` and
    /// `MachineWords::of_mind`; its motor row is its first landing's, not the
    /// adoption's, so none stands before a think lands.
    #[test]
    fn the_first_control_that_took_sight_wins_and_the_live_reads_the_machine_first() {
        let mut arena = arena(2);
        hold(&mut arena, ObjId(10), true);
        arena.now_ms = 1_000;
        for control in every_channel(2, ControlState::Linkdead) {
            apply_control(&mut arena, &control);
        }
        assert_eq!(
            arena.at_release.get(&RED).map(|then| then.at_ms),
            Some(1_000)
        );
        // The grace runs out later; the snapshot is the `Linkdead`'s.
        arena.now_ms = 4_000;
        hold(&mut arena, ObjId(11), true);
        for control in every_channel(2, ControlState::Released) {
            apply_control(&mut arena, &control);
        }
        assert_eq!(
            arena.at_release.get(&RED).map(|then| then.at_ms),
            Some(1_000)
        );
        assert!(!arena.pilots.iter().any(|seat| seat.pilot == PILOT));
        assert_eq!(
            arena.unit(RED).map(|unit| unit.controller),
            Some(Controller::Mind),
            "a released tank goes to a mind"
        );
        let machine = arena.machine_state(RED).map(|state| {
            MachineWords::of_mind(
                state,
                arena.minds.get(&RED).and_then(|row| row.order.as_ref()),
                arena.motors.row(RED),
            )
        });
        assert_eq!(
            machine.as_ref().map(|words| words.state.as_str()),
            Some("withdraw"),
            "a released tank goes to the mind in Withdraw"
        );
        assert_eq!(
            machine.as_ref().and_then(|words| words.goal),
            None,
            "no think landed, so no motor row stands yet"
        );
        arena.now_ms = 9_500;
        for control in every_channel(3, ControlState::Live) {
            apply_control(&mut arena, &control);
        }
        assert_eq!(
            arena.unit(RED).map(|unit| unit.controller),
            Some(Controller::Pilot)
        );
        assert!(
            arena.minds.contains_key(&RED),
            "the row is kept dormant (17b cut M3.5)"
        );
        assert!(
            arena.motors.row(RED).is_none(),
            "no motor row for the pilot's hull (17b cut M1a.1)"
        );
        assert!(arena.at_release.is_empty(), "the snapshot is spent");
        let report = arena
            .pending_catchup
            .get(&PILOT)
            .expect("the pilot's report");
        assert_eq!(report.away_ms, 8_500, "counted from the Linkdead");
        assert_eq!(report.contacts_new, vec![slot(&arena, 2)]);
        assert_eq!(report.machine, machine, "read before the row went");
    }

    /// **17b cut M5.3 (D14)**: the same life on a hull whose sealed mind is
    /// not Standard Doctrine — an explicit draft fixture identity rather
    /// than a renamed doctrine identity — comes back to its pilot with the
    /// machine's word `unknown` (`Arena::machine_state`, off the seal), and the
    /// catch-up says it as the mind's own: `  The machine was flying on its own
    /// mind under no order.` The word names no mind, so a pilot whose hull a
    /// fixture flew and one whose hull another mind flew read one sentence.
    #[test]
    fn m5_d14_a_hull_not_on_doctrine_comes_back_flying_on_its_own_mind() {
        use crate::arena::doctrine::MachineState;
        let mut arena = arena(0);
        let seal = {
            let mut red = line_tank(RED, PartyId::RED, pose(500, 500));
            let terms = crate::match_::SealTerms {
                identity: crate::match_::SealIdentity {
                    mind: "Fixture I".into(),
                    origin: crate::arena::profile_seal::MindOrigin::Draft {
                        character: RED,
                        mind: 1,
                        revision: 1,
                        digest: mush_core::DefinitionDigest::from_bytes([9; 32]),
                    },
                    source_sha256: [9; 32],
                },
                ..crate::match_::SealTerms::fixture()
            };
            red.reseal(&terms).expect("the draft fixture terms seal");
            red.seal
        };
        if let Some(unit) = arena.unit_mut(RED) {
            unit.seal = seal.seal_ref();
        }
        assert!(!arena.unit(RED).is_some_and(|unit| unit.seal.doctrine));
        arena.now_ms = 1_000;
        for control in every_channel(2, ControlState::Linkdead) {
            apply_control(&mut arena, &control);
        }
        arena.now_ms = 4_000;
        for control in every_channel(2, ControlState::Released) {
            apply_control(&mut arena, &control);
        }
        assert_eq!(
            arena.unit(RED).map(|unit| unit.controller),
            Some(Controller::Mind),
            "a released tank goes to its sealed mind"
        );
        assert_eq!(arena.machine_state(RED), Some(MachineState::Unknown));
        arena.now_ms = 9_500;
        for control in every_channel(3, ControlState::Live) {
            apply_control(&mut arena, &control);
        }
        let report = arena
            .pending_catchup
            .get(&PILOT)
            .expect("the pilot's report");
        assert_eq!(
            report.machine.as_ref().map(|words| words.state.as_str()),
            Some("unknown"),
            "the wire's word is unchanged"
        );
        let red = Designator::Unit { party: PartyId::RED, letter: 'A' };
        let lines = crate::match_::cockpit::catchup_lines(red, report);
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some("  The machine was flying on its own mind under no order."),
            "{lines:?}"
        );
    }

    /// K4's revival inside the grace: the `Live` finds the pilot on the roster,
    /// so the snapshot goes and nothing is built — and a later drop takes a
    /// fresh one.
    #[test]
    fn a_revival_inside_the_grace_spends_the_snapshot_and_builds_nothing() {
        let mut arena = arena(0);
        arena.now_ms = 1_000;
        for control in every_channel(2, ControlState::Linkdead) {
            apply_control(&mut arena, &control);
        }
        assert!(arena.at_release.contains_key(&RED));
        for control in every_channel(3, ControlState::Live) {
            apply_control(&mut arena, &control);
        }
        assert!(
            arena.at_release.is_empty(),
            "a stale baseline outlived a revival"
        );
        assert!(arena.pending_catchup.is_empty());
        arena.now_ms = 7_000;
        for control in every_channel(4, ControlState::Released) {
            apply_control(&mut arena, &control);
        }
        assert_eq!(
            arena.at_release.get(&RED).map(|then| then.at_ms),
            Some(7_000)
        );
        // A control naming no pilot takes no snapshot of its own.
        let mut quiet = arena_at_no_pilot();
        apply_control(
            &mut quiet,
            &Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 2,
                state: ControlState::Linkdead,
                pilot: None,
            },
        );
        assert!(quiet.at_release.is_empty());
    }

    /// A report no frame carried yet goes with the seat: a re-seat and a drop
    /// in one pulse owe the pilot nothing, and the drop takes a fresh snapshot.
    #[test]
    fn a_drop_before_the_report_is_framed_takes_it_back() {
        let mut arena = arena(0);
        for control in every_channel(2, ControlState::Released) {
            apply_control(&mut arena, &control);
        }
        arena.now_ms = 6_000;
        for control in every_channel(3, ControlState::Live) {
            apply_control(&mut arena, &control);
        }
        assert!(arena.pending_catchup.contains_key(&PILOT));
        for control in every_channel(4, ControlState::Released) {
            apply_control(&mut arena, &control);
        }
        assert!(
            arena.pending_catchup.is_empty(),
            "the report outlived the seat"
        );
        assert_eq!(
            arena.at_release.get(&RED).map(|then| then.at_ms),
            Some(6_000)
        );
    }

    fn arena_at_no_pilot() -> Arena {
        crate::arena::test_support::arena_of(vec![line_tank(RED, PartyId::RED, pose(500, 500))])
    }
}
