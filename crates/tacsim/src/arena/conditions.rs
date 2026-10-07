//! **Condition slots: the shot decision written down** (17b cut M1b.1; doc 17
//! §5 "The mechanism", "The law"; D-UM-16, D-UM-17 as amended 2026-10-04,
//! D-UM-19).
//!
//! A **condition slot** is the principal's shot decision as native engine
//! state on an armed hull: which weapon, which target (the lock, classified),
//! at what odds and aspect, inside what range, keeping what reserve
//! ([`FireCondition`]). The arena evaluates it — never a mind, never a pulse —
//! at the weapon's own [`crate::arena::state::DueKind::WeaponReady`], at the
//! passes that refresh the lock's plot, at a landing, a lock, an arming and a
//! startup ([`ConditionTrigger`]), each at its own arena millisecond, reading
//! the plot through [`Arena::aim_at`] and [`Arena::resolve_seen`] and never a
//! live pose, and fires through [`crate::arena::intents::apply_intent`] and the
//! resolver at that millisecond (`Arena::evaluate_conditions`). A slot shot is
//! journalled `by: "cond"` with its slot (D-UM-14).
//!
//! # Where the slots live
//!
//! In one transparent table on the arena, [`Arena::conditions`] — for a hull
//! of **any** controller, not in a mind's row (a piloted hull has none, and
//! `Arena::release_mind` deletes a mind's row on every transition): a declared
//! departure from doc 17 §5's placement, for §24. A hull holds a key only while
//! it has a slot armed, so the table of every match that exists is empty and
//! every trigger's first question — [`ConditionTable::is_empty_for`] — is one
//! map miss that touches nothing else. **Slot numbers are stable**: a slot is
//! its 1-based index in the hull's row, an arming of a weapon already armed
//! replaces its terms in place and keeps its number, any other takes the first
//! free slot, and a disarm leaves every other slot where it was. The slots
//! belong to the hull's principal: they are cleared on every controller
//! transition (`Arena::set_controller`), on the wreck (`Arena::destroy_unit`)
//! and, per weapon, when a turret shot off takes its guns
//! (`Consequence::RemoveWeapons`).
//!
//! **The core's slots** (17b cut M3.2; doc 17 §5, §9, D-UM-16: *up to the
//! core's slots*): a hull's row is laid down at its first arming as many slots
//! wide as its core carries (`Arena::slot_capacity` — Class 0's one with no
//! core, I two, II four, III six), at most
//! [`crate::arena::core::MAX_CONDITION_SLOTS`]; a seated pilot and a mind on
//! one hull hold the same row. On shipped content every hull has a slot for
//! each of its weapons, so the count refusal ([`Refusal::NoSlot`]) is reached
//! only on a hull with fewer slots than weapons. A core that loses a class
//! (17b cut M3.4) cuts its row ([`ConditionTable::truncate`]).
//!
//! # The law (D-UM-17 as amended)
//!
//! [`condition_is_lawful`] is the floor's **static** terms, checked at the
//! landing of every arming ([`Arena::resolve_arming`]): the odds term at or
//! above the match's Compact floor ([`Arena::odds_floor_permille`], captured at
//! check-out), and a target kind the floor admits — **in 17b cut M1b any
//! [`SlotTarget::Contact`] is unlawful whatever the id**: the rule language has
//! no contact word and nobody arms one, and a held id and an unheld one must
//! answer one word (M5 admits a classified `Contact` at the landing when the
//! `LOCK` rules arrive). Refuse-not-clamp: a condition under the floor is
//! refused [`Refusal::Unlawful`], never raised to it. The lock's classification
//! is **not** a lawfulness term but a warrant term ([`Arena::warrant`]): a slot
//! on an unclassified lock is idle.
//!
//! # What the warrant is, and is not
//!
//! [`Arena::warrant`] is the pure predicate a slot's shot must meet on the
//! plot: the target is the lock, then the reserve, the rounded range, the
//! aspect and the odds, the first failing term answering. It has **no
//! readiness and no empty-magazine term**: a cycling gun is the evaluator's
//! idle test and the resolver's own `Cycling`/`Empty`, and the law must never
//! turn a cycling gun into `Unwarranted` (doc 17 §5's arbitration: whichever
//! lands first fires, the other reads `Cycling`). A slot that held for heat —
//! the resolver's `Overheated`, dropped silently — waits for the next pass or
//! `WeaponReady`: declared.
//!
//! # Cost
//!
//! Evaluation is bounded by slots × events (doc 17 §5; 17b §1(c)): at a
//! `WeaponReady` one slot, at a pass, a share, a landing, a lock, an arming or a
//! startup at most the hull's row (its core's slots, at most
//! [`crate::arena::core::MAX_CONDITION_SLOTS`]), each one `aim_at` trace and
//! one `resolve_seen` — a slot is armed per weapon, so a row wider than the
//! hull's weapons costs nothing more. Three hundred hulls with two slots each
//! is about six hundred traces in the worst window — against the harness's 2,994-trace peaks per
//! wake — and the in-source `m1b1_the_evaluator_cost_on_a_three_hundred_hull_field`
//! reports the per-call figure in release. A hull with no slot costs one
//! `BTreeMap` miss per event and nothing else: **no trigger touches the action
//! hash before it**, so the production hash, by substep and by line, cannot
//! move (17b cut M1b.0's line-hash pin).

use std::collections::BTreeMap;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::intents::{
    Intent, IntentKind, IntentOutcome, IntentSource, Refusal, apply_intent,
};
use crate::arena::perception::{Aspect, FrameNotice, Named, lock_range_step_m};
use crate::arena::reflex::ReflexCounters;
use crate::arena::resolution::facet_band_seen;
use crate::arena::state::{ActionHash, Arena, Channel, Classification, ContactId};
use crate::arena::view::range_m_rounded;
use crate::arena::weapons::FiredBy;

/// **How many condition slots a Class I hull holds** (17b cut M1b.1; doc 17
/// §9's class I figure). **The Class I row of `ClassTable::STOCK`; production
/// reads the profile, the tests' arithmetic keeps the constant** (17b cut
/// M3.1): a hull's capacity is its core's row's `slots`
/// (`Arena::slot_capacity`), and since 17b cut M3.2 its row is laid down that
/// wide ([`ConditionTable::arm`]); no production code reads this constant.
pub const STOCK_CONDITION_SLOTS: usize = 2;

/// **Every aspect**: the mask bits are `0` front, `1` left, `2` right, `3` rear
/// ([`Aspect::code`]'s order).
pub const ASPECTS_ANY: u8 = 0b1111;

/// **How often one held slot may say why it held, per contact**, arena
/// milliseconds (doc 17 §5: *a held shot said at most once per contact per
/// 10 s*). 17b cut M1b.4 stages the held sentence against
/// [`Arena::held_said`]; the table is carried from this cut.
pub const HELD_SAID_MS: i64 = 10_000;

/// **The mask of `aspects`** — one bit per [`Aspect::code`].
#[must_use]
pub fn aspect_mask(aspects: &[Aspect]) -> u8 {
    aspects
        .iter()
        .fold(0_u8, |mask, aspect| mask | (1_u8 << aspect.code()))
}

/// **Whether `aspect` is in `mask`.**
#[must_use]
pub const fn aspect_in(mask: u8, aspect: Aspect) -> bool {
    mask & (1_u8 << aspect.code()) != 0
}

/// **What a slot fires at** (D-UM-16). A slot never takes a lock (17b's Q-M5):
/// every target word names WHICH lock the slot fires on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "target")]
pub enum SlotTarget {
    /// The hull's lock, when it is held, classified and does not read dead
    /// (the rule language's default target; the read-dead term since 17b cut
    /// M2.2's ruling (a)).
    Lock,
    /// The lock when it is this contact, classified and not reading dead
    /// (the read-dead term since 17b cut M2.2) — **unlawful in 17b cut
    /// M1b whatever the id** ([`condition_is_lawful`]); kept for the type's
    /// meaning, which M5's `LOCK` rules make reachable.
    Contact {
        /// The contact.
        id: ContactId,
    },
    /// The lock when it is held, classified on another side and does not read
    /// dead — **no arena-side ranking**: the best hostile is the principal's
    /// lock, which a mind chooses at its think (the first review's F1).
    BestHostile,
}

impl SlotTarget {
    /// The code byte the state hash folds: `0` lock, `1` contact, `2` best
    /// hostile (item 3d).
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            SlotTarget::Lock => 0,
            SlotTarget::Contact { .. } => 1,
            SlotTarget::BestHostile => 2,
        }
    }
}

/// **One condition slot's terms** (D-UM-16): the weapon, the target, and the
/// four things the plot must show.
///
/// `min_permille` is per mille (a pilot's `odds >= 60` is `600`);
/// `min_range_m`/`max_range_m` bound the lock's **rounded** range, the figure
/// `RANGE` prints (`min_range_m` is 17b's one addition to D-UM-16's field list,
/// declared); `min_rounds_left` is the reserve the slot keeps (it fires only
/// while the weapon holds MORE rounds than this).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FireCondition {
    /// The weapon's component id.
    pub weapon: SmolStr,
    /// What it fires at.
    pub target: SlotTarget,
    /// The least chance to hit it fires at, per mille.
    pub min_permille: u16,
    /// The aspects it fires at ([`ASPECTS_ANY`] for all four).
    pub aspects: u8,
    /// The rounds it keeps back: it fires while the weapon holds more.
    pub min_rounds_left: u32,
    /// The nearest rounded range it fires at, metres.
    pub min_range_m: u32,
    /// The furthest rounded range it fires at, metres (`u32::MAX` for none).
    pub max_range_m: u32,
}

/// **The STATIC terms of the legal floor** (doc 17 §5, D-UM-17 as amended
/// 2026-10-04): the odds term at or above `floor_permille`, and a target kind
/// the floor admits.
///
/// **In 17b cut M1b any [`SlotTarget::Contact`] is unlawful whatever the id**:
/// the rule language has no contact word and nobody arms one, and a held id
/// and an unheld one must answer one word — so the predicate never reads the
/// contact store. 17b cut M5 admits a classified `Contact` at the landing when
/// the `LOCK` rules arrive. The lock's classification is a WARRANT term
/// ([`Arena::warrant`]), not a lawfulness one.
///
/// # Errors
///
/// [`Refusal::Unlawful`] for either term.
pub fn condition_is_lawful(cond: &FireCondition, floor_permille: u32) -> Result<(), Refusal> {
    if u32::from(cond.min_permille) < floor_permille {
        return Err(Refusal::Unlawful);
    }
    if matches!(cond.target, SlotTarget::Contact { .. }) {
        return Err(Refusal::Unlawful);
    }
    Ok(())
}

/// **Every hull's slots** (17b cut M1b.1): a hull holds a key only while one
/// of its slots is armed. `#[serde(transparent)]`, so the snapshot writes one
/// `state.conditions` object (an `ObjId` key writes as a JSON object key).
///
/// **17b cut M3.2** (D-UM-16; `SNAPSHOT_FORMAT` 8): a row is as wide as the
/// hull's core's slots when it was laid down (`Arena::slot_capacity`, at
/// most [`crate::arena::core::MAX_CONDITION_SLOTS`]) — a JSON array of that
/// many entries, `null` for a free slot — where it was a fixed pair.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConditionTable {
    rows: BTreeMap<ObjId, Vec<Option<FireCondition>>>,
}

impl ConditionTable {
    /// **`unit`'s row**, in slot order — as wide as the core's slots it was
    /// laid down at (17b cut M3.2), and EMPTY for a hull with no row: every
    /// reader walks the slice's own length, never a fixed width.
    #[must_use]
    pub fn slots(&self, unit: ObjId) -> &[Option<FireCondition>] {
        self.rows.get(&unit).map_or(&[], Vec::as_slice)
    }

    /// **Whether `unit` has no slot armed** — the fast path every trigger takes
    /// first: one map miss and nothing else.
    #[must_use]
    pub fn is_empty_for(&self, unit: ObjId) -> bool {
        !self.rows.contains_key(&unit)
    }

    /// **The 1-based slot `weapon` is armed in**, or `None`.
    #[must_use]
    pub fn slot_of(&self, unit: ObjId, weapon: &str) -> Option<u8> {
        self.slots(unit)
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|cond| cond.weapon == weapon))
            .and_then(|at| u8::try_from(at.saturating_add(1)).ok())
    }

    /// **Arm `cond` on `unit`**, answering its 1-based slot: the slot of the
    /// same weapon replaced in place wherever it sits (it keeps its number),
    /// else the first free one among the hull's first `capacity` — the core's
    /// slots (`Arena::slot_capacity`), clamped to
    /// [`crate::arena::core::MAX_CONDITION_SLOTS`]. A hull with no row has one
    /// laid down `capacity` slots wide (17b cut M3.2, D-UM-16: *up to the
    /// core's slots*). A row narrower than `capacity` is widened to it; a wider
    /// one keeps its width, and no free slot past `capacity` is taken —
    /// [`ConditionTable::truncate`] is what cuts a row.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSlot`] when every slot the hull holds is armed with another
    /// weapon (the count's own word, not the law's) — and for a capacity of
    /// none, with no row laid down.
    pub fn arm(
        &mut self,
        unit: ObjId,
        cond: FireCondition,
        capacity: usize,
    ) -> Result<u8, Refusal> {
        let holds = capacity.min(crate::arena::core::MAX_CONDITION_SLOTS);
        if holds == 0 {
            return Err(Refusal::NoSlot);
        }
        let row = self.rows.entry(unit).or_default();
        if row.len() < holds {
            row.resize(holds, None);
        }
        let at = row
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|had| had.weapon == cond.weapon))
            .or_else(|| row.iter().take(holds).position(Option::is_none));
        let Some(at) = at else {
            return Err(Refusal::NoSlot);
        };
        let Some(slot) = row.get_mut(at) else {
            return Err(Refusal::NoSlot);
        };
        *slot = Some(cond);
        u8::try_from(at.saturating_add(1)).map_err(|_| Refusal::NoSlot)
    }

    /// **Disarm `unit`'s slot of `weapon`, or every slot for `None`**, answering
    /// the 1-based numbers freed. The key goes when every slot is empty; a
    /// disarm of nothing frees nothing.
    pub fn disarm(&mut self, unit: ObjId, weapon: Option<&str>) -> Vec<u8> {
        let Some(row) = self.rows.get_mut(&unit) else {
            return Vec::new();
        };
        let mut freed = Vec::new();
        for (at, slot) in row.iter_mut().enumerate() {
            let going = slot
                .as_ref()
                .is_some_and(|cond| weapon.is_none_or(|named| cond.weapon == named));
            if going {
                *slot = None;
                if let Ok(n) = u8::try_from(at.saturating_add(1)) {
                    freed.push(n);
                }
            }
        }
        if row.iter().all(Option::is_none) {
            self.rows.remove(&unit);
        }
        freed
    }

    /// **Every slot of `unit` gone** — a controller transition, a wreck.
    pub fn clear(&mut self, unit: ObjId) {
        self.rows.remove(&unit);
    }

    /// **`unit`'s row cut to `capacity` slots** (17b cut M3.2; D-UM-16,
    /// D-UM-34): every slot numbered past `capacity` (clamped to
    /// [`crate::arena::core::MAX_CONDITION_SLOTS`]) cleared and the row
    /// narrowed to it, answering the numbers of the ARMED slots cleared, in
    /// order, so their held-shot rows go with them (`Arena::forget_held_said`).
    /// The key goes when nothing armed is left; a hull with no row, or one
    /// already that narrow, answers nothing. Since 17b cut M3.4
    /// [`Arena::truncate_slots_to`] calls it — through
    /// [`Arena::truncate_slots`] when a core loses a class or is destroyed,
    /// and with Class 0's one slot at every drop to the ROM.
    pub fn truncate(&mut self, unit: ObjId, capacity: usize) -> Vec<u8> {
        let holds = capacity.min(crate::arena::core::MAX_CONDITION_SLOTS);
        let Some(row) = self.rows.get_mut(&unit) else {
            return Vec::new();
        };
        let mut cleared = Vec::new();
        for (at, slot) in row.iter().enumerate().skip(holds) {
            if slot.is_some()
                && let Ok(n) = u8::try_from(at.saturating_add(1))
            {
                cleared.push(n);
            }
        }
        row.truncate(holds);
        if row.iter().all(Option::is_none) {
            self.rows.remove(&unit);
        }
        cleared
    }

    /// **The slots of the weapons in `going` gone** — a turret shot off
    /// (`Consequence::RemoveWeapons`); the rest keep their numbers. Answers the
    /// numbers freed.
    pub fn clear_weapons(&mut self, unit: ObjId, going: &[SmolStr]) -> Vec<u8> {
        let mut freed = Vec::new();
        for weapon in going {
            freed.extend(self.disarm(unit, Some(weapon.as_str())));
        }
        freed.sort_unstable();
        freed
    }

    /// **Every row**, unit-id order — what the state hash folds (item 3d:
    /// its ARMED slots by number, so a row's width folds no byte).
    pub fn rows(&self) -> impl Iterator<Item = (&ObjId, &[Option<FireCondition>])> {
        self.rows
            .iter()
            .map(|(unit, slots)| (unit, slots.as_slice()))
    }
}

/// **Where a slot's evaluation was asked from** (17b cut M1b.1): each at its
/// own arena millisecond, never once per pulse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionTrigger {
    /// The weapon's cycle ran down (`DueKind::WeaponReady`): that weapon's slot.
    WeaponReady,
    /// A pass wrote the lock's plot: the hull's own `SensorPass`, or a `Share`
    /// it received.
    Pass,
    /// A mind's landing, once after every intent of it.
    Landing,
    /// A seat's `LOCK` resolved, in phase 4.
    Lock,
    /// A seat's arming resolved, in phase 4.
    Arming,
    /// Power came online (`DueKind::StartupComplete`).
    Startup,
}

/// **Why a slot held a shot it was evaluated for**, the term that failed (doc
/// 17 §5's four held sentences; 17b cut M1b.4 says them). Carried inside
/// [`crate::arena::perception::FrameNotice::SlotHeld`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "why")]
pub enum HeldWhy {
    /// The chance to hit was under the slot's term.
    Odds {
        /// The chance, per mille.
        permille: u32,
        /// The slot's term, per mille.
        floor: u16,
    },
    /// The lock showed an aspect the slot does not fire at.
    Aspect {
        /// The lock, named.
        contact: Named,
        /// What it showed.
        aspect: Aspect,
    },
    /// The lock's rounded range was outside the slot's bounds.
    Range {
        /// The lock, named.
        contact: Named,
        /// The rounded range, metres — the figure `RANGE` prints.
        range_m: u64,
        /// The slot's nearest, metres.
        min_m: u32,
        /// The slot's furthest, metres.
        max_m: u32,
    },
    /// The weapon held no more rounds than the slot keeps.
    Rounds {
        /// Rounds left.
        rounds: u32,
        /// Rounds the slot keeps.
        keeps: u32,
    },
}

/// **A slot's answer when it does not fire**: idle (nothing to fire at, or
/// the gun not ready — never said), or held for a named reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Held {
    /// No warranted target, no such weapon, or the gun cycling or dry.
    Idle,
    /// The target was there and a term failed.
    Why(HeldWhy),
}

impl Arena {
    /// **The warrant** (D-UM-17): whether `slot`'s terms hold for `unit` at
    /// `at_ms`, answering the lock, the chance and the aspect — or the first
    /// failing term:
    ///
    /// 1. the hull exists, is not a wreck and carries `slot.weapon`, else
    ///    [`Held::Idle`];
    /// 2. **the target is the lock**: [`SlotTarget::Lock`] — the lock when held
    ///    and classified; [`SlotTarget::Contact`] — the lock when it is that id
    ///    and classified; [`SlotTarget::BestHostile`] — the lock when held and
    ///    classified on another side; **all three only while the lock does not
    ///    read dead** ([`Arena::contact_reads_dead`]; 17b cut M2.2, the lead's
    ///    ruling (a): one read-dead rule for the three targets, where 17b cut
    ///    M1b.1 read it for `BestHostile` alone and a `Lock` or `Contact` slot
    ///    kept firing at a hull its own round had destroyed until the track
    ///    expired); and the lock's target still in the match. Else
    ///    [`Held::Idle`] — an unclassified lock is idle (D-UM-17's
    ///    classified-only rule, extended to the default target);
    /// 3. `rounds > slot.min_rounds_left`, else [`HeldWhy::Rounds`];
    /// 4. one [`Arena::aim_at`] trace; the plot's range **rounded** at the
    ///    lock's step for the plot's age (`range_m_rounded` at
    ///    `lock_range_step_m`, the figure `RANGE` prints) inside
    ///    `[min_range_m, max_range_m]`, else [`HeldWhy::Range`];
    /// 5. the aspect the plot shows the hull in `slot.aspects`, else
    ///    [`HeldWhy::Aspect`];
    /// 6. [`Arena::resolve_seen`]'s chance at or above `min_permille`, else
    ///    [`HeldWhy::Odds`].
    ///
    /// It reads the shadow row, the plot and the hull's own state, never
    /// another unit's live pose, and has **no readiness and no empty-magazine
    /// term** (this module's head).
    ///
    /// # Errors
    ///
    /// The [`Held`] answer of the first failing term.
    pub fn warrant(
        &self,
        unit: ObjId,
        slot: &FireCondition,
        at_ms: i64,
    ) -> Result<(ContactId, u32, Aspect), Held> {
        let Some(state) = self.unit(unit) else {
            return Err(Held::Idle);
        };
        if state.limits.wreck || state.weapon(&slot.weapon).is_none() {
            return Err(Held::Idle);
        }
        let Some(lock) = state.lock else {
            return Err(Held::Idle);
        };
        let Some(held) = self.contacts.resolve(unit, lock) else {
            return Err(Held::Idle);
        };
        if self.unit(held.target).is_none() {
            return Err(Held::Idle);
        }
        // 17b cut M2.2, ruling (a): one read-dead rule for the three targets.
        let targeted = match (slot.target, held.class) {
            (_, Classification::Unknown) => false,
            (SlotTarget::Lock, Classification::Classified { .. }) => !self.contact_reads_dead(held),
            (SlotTarget::Contact { id }, Classification::Classified { .. }) => {
                id == lock && !self.contact_reads_dead(held)
            }
            (SlotTarget::BestHostile, Classification::Classified { team }) => {
                team != state.team && !self.contact_reads_dead(held)
            }
        };
        if !targeted {
            return Err(Held::Idle);
        }
        let rounds = state.weapon_ammo(&slot.weapon);
        if rounds <= slot.min_rounds_left {
            return Err(Held::Why(HeldWhy::Rounds {
                rounds,
                keeps: slot.min_rounds_left,
            }));
        }
        let Some(aim) = self.aim_at(unit, lock) else {
            return Err(Held::Idle);
        };
        let age_ms = at_ms.saturating_sub(held.last_seen_ms);
        let range_m = range_m_rounded(aim.range_cm, lock_range_step_m(age_ms));
        if range_m < u64::from(slot.min_range_m) || range_m > u64::from(slot.max_range_m) {
            return Err(Held::Why(HeldWhy::Range {
                contact: self.name_of(unit, lock),
                range_m,
                min_m: slot.min_range_m,
                max_m: slot.max_range_m,
            }));
        }
        let aspect = Aspect::from_band(facet_band_seen(&aim.seen_at, state));
        if !aspect_in(slot.aspects, aspect) {
            return Err(Held::Why(HeldWhy::Aspect {
                contact: self.name_of(unit, lock),
                aspect,
            }));
        }
        let permille = self.resolve_seen(unit, &aim, &slot.weapon).p_permille;
        if permille < u32::from(slot.min_permille) {
            return Err(Held::Why(HeldWhy::Odds {
                permille,
                floor: slot.min_permille,
            }));
        }
        Ok((lock, permille, aspect))
    }

    /// **Evaluate `unit`'s slots at `t_ms`** (D-UM-16), from `trigger` — at a
    /// [`ConditionTrigger::WeaponReady`] the slot of `weapon` alone, otherwise
    /// every armed slot — answering each evaluated slot's number with the shot's
    /// sequence or why it did not fire.
    ///
    /// **The first line returns for a hull with no slot, and nothing of the
    /// arena — the action hash included — is touched before it**; nothing is
    /// evaluated after the match ended. Then per slot: the weapon ready at
    /// `t_ms` and loaded, else [`Held::Idle`] (readiness is this idle test and
    /// never a warrant term); then [`Arena::warrant`]. A warranted slot fires:
    /// the action hash is opened `Pending` here and only here when the caller
    /// has not (so a hull whose slots hold takes no hash), then
    /// `Fire { weapon, slot: Some(n) }` goes through `apply_intent` under
    /// [`IntentSource::Cond`] — never counted on the seat window, exempt from
    /// the `Linkdead` gate — and the queued intent comes straight back out (as
    /// `Arena::apply_machine_intents` pops it) to be resolved by
    /// [`Arena::resolve_fire_by`] with [`FiredBy::Cond`] at `t_ms`;
    /// `counters.shots` and `counters.cond_fires` count an accepted shot. A
    /// refusal at apply or at the resolver (`Cycling` when a landing and a ready
    /// event race, `Overheated`, `PowerNotOnline`, a standing `hold_fire`) is
    /// dropped silently and answers [`Held::Idle`]: the slot stays armed and
    /// waits for the next trigger. `counters.cond_held` counts a
    /// [`Held::Why`].
    ///
    /// **17b cut M1b.4 says both** (doc 17 §5, D-UM-14): an accepted shot
    /// stages [`FrameNotice::SlotFired`] — the slot, the weapon, the lock by its
    /// designator, the chance and the face the warrant answered — and a
    /// [`Held::Why`] stages [`FrameNotice::SlotHeld`] through `say_held`'s
    /// ten-second rate; [`Held::Idle`] and a refusal dropped at apply or at the
    /// resolver say nothing. Both on `unit`'s own hull alone.
    pub(crate) fn evaluate_conditions(
        &mut self,
        unit: ObjId,
        t_ms: i64,
        trigger: ConditionTrigger,
        weapon: Option<&str>,
        counters: &mut ReflexCounters,
    ) -> Vec<(u8, Result<u64, Held>)> {
        if self.conditions.is_empty_for(unit) {
            return Vec::new();
        }
        if self.ended.is_some() {
            return Vec::new();
        }
        let armed: Vec<(u8, FireCondition)> = self
            .conditions
            .slots(unit)
            .iter()
            .enumerate()
            .filter_map(|(at, slot)| {
                let cond = slot.as_ref()?;
                if trigger == ConditionTrigger::WeaponReady
                    && weapon.is_none_or(|named| cond.weapon != named)
                {
                    return None;
                }
                let n = u8::try_from(at.saturating_add(1)).ok()?;
                Some((n, cond.clone()))
            })
            .collect();
        let mut answers = Vec::with_capacity(armed.len());
        for (n, cond) in armed {
            let ready = self.unit(unit).is_some_and(|state| {
                state.weapon_ready(&cond.weapon, t_ms) && state.weapon_ammo(&cond.weapon) > 0
            });
            if !ready {
                answers.push((n, Err(Held::Idle)));
                continue;
            }
            let warranted = match self.warrant(unit, &cond, t_ms) {
                Ok(warranted) => warranted,
                Err(held) => {
                    if let Held::Why(why) = &held {
                        counters.cond_held = counters.cond_held.saturating_add(1);
                        // 17b cut M1b.4 (doc 17 §5): the held sentence, at
                        // most once per slot and contact per `HELD_SAID_MS`.
                        self.say_held(unit, n, &cond.weapon, why, t_ms);
                    }
                    answers.push((n, Err(held)));
                    continue;
                }
            };
            // The one place a slot touches the action hash: opened, not taken —
            // the shot's line takes it (`Arena::event_hash`).
            if self.action_hash == ActionHash::Outside {
                self.action_hash = ActionHash::Pending;
            }
            let intent = Intent {
                unit,
                channel: Channel::Fire,
                epoch: self.epochs.current(unit, Channel::Fire),
                kind: IntentKind::Fire {
                    weapon: cond.weapon.clone(),
                    slot: Some(n),
                },
                seq: u64::from(n),
            };
            let shot = match apply_intent(self, &intent, IntentSource::Cond) {
                IntentOutcome::Queued => {
                    // `apply_intent` pushed exactly this order to the back of
                    // the queue; it comes straight back out, as a machine's does.
                    if let Some(queue) = self.pending.get_mut(&unit) {
                        queue.pop_back();
                        if queue.is_empty() {
                            self.pending.remove(&unit);
                        }
                    }
                    self.resolve_fire_by(unit, &cond.weapon, t_ms, FiredBy::Cond { slot: n })
                        .ok()
                }
                IntentOutcome::AppliedContinuous | IntentOutcome::Refused(_) => None,
            };
            match shot {
                Some(seq) => {
                    counters.shots = counters.shots.saturating_add(1);
                    counters.cond_fires = counters.cond_fires.saturating_add(1);
                    // 17b cut M1b.4 (D-UM-14's attribution): every slot shot
                    // announced once, on the shooter's own hull alone — the
                    // lock its platform holds by its designator, and the chance
                    // and the face the warrant read off the plot.
                    let (lock, p_permille, aspect) = warranted;
                    let contact = self
                        .contacts
                        .get(unit, lock)
                        .map(|_| self.name_of(unit, lock));
                    self.stage_notice(
                        unit,
                        FrameNotice::SlotFired {
                            slot: n,
                            weapon: cond.weapon.clone(),
                            contact,
                            p_permille,
                            aspect,
                        },
                    );
                    answers.push((n, Ok(seq)));
                }
                None => answers.push((n, Err(Held::Idle))),
            }
        }
        answers
    }

    /// **The landing of an arming** (17b cut M1b.1; D-UM-10, D-UM-17 as
    /// amended), for a seat (pulse phase 4) and a mind
    /// (`Arena::apply_machine_intents`) alike, answering the slot armed.
    ///
    /// [`Arming::Arm`]: the hull exists ([`Refusal::Unknown`]), is not a wreck
    /// ([`Refusal::Wreck`]), carries the weapon ([`Refusal::NoWeapon`]); the
    /// condition is lawful against the arena's own captured floor
    /// ([`condition_is_lawful`], [`Refusal::Unlawful`] — the law before the
    /// count); then [`ConditionTable::arm`] ([`Refusal::NoSlot`]; since 17b
    /// cut M3.4 at `Arena::arming_capacity`, and on a hull the ROM thinks for
    /// a `NoSlot` the ROM's own gun answers by taking back a slot the failed
    /// mind left on another, `Arena::rom_takes_slot`) answers
    /// `Some(n)` and the slot's [`Arena::held_said`] rows are dropped, so a
    /// re-armed slot's first held sentence under new terms is not suppressed
    /// by the old terms' stamp. [`Arming::Disarm`]: the named weapon's slot, or
    /// every slot, freed with its rows, answering `None` (a disarm of nothing
    /// is a no-op).
    ///
    /// `at_ms` is the landing's millisecond, for the doc and a future journal
    /// line: nothing is journalled here — the slots are state, folded by the
    /// hash (item 3d).
    ///
    /// # Errors
    ///
    /// The [`Refusal`] word of the first failing term.
    pub fn resolve_arming(
        &mut self,
        unit: ObjId,
        arming: &Arming,
        at_ms: i64,
    ) -> Result<Option<u8>, Refusal> {
        let _ = at_ms;
        match arming {
            Arming::Arm(cond) => {
                let Some(state) = self.unit(unit) else {
                    return Err(Refusal::Unknown);
                };
                if state.limits.wreck {
                    return Err(Refusal::Wreck);
                }
                if state.weapon(&cond.weapon).is_none() {
                    return Err(Refusal::NoWeapon);
                }
                condition_is_lawful(cond, self.odds_floor_permille)?;
                // 17b cut M3.1 (D-UM-35): the hull's own capacity, its core's —
                // and, since 17b cut M3.4, Class 0's one on a mind's hull
                // dropped to the ROM (`Arena::arming_capacity`).
                let capacity = self.arming_capacity(unit);
                let n = match self.conditions.arm(unit, cond.clone(), capacity) {
                    // 17b cut M3.4 (D-UM-32): the ROM owns its one slot — it
                    // takes back a slot the failed mind left on another gun.
                    Err(Refusal::NoSlot) if self.thinks_on_rom(unit) => {
                        self.rom_takes_slot(unit, cond, capacity)?
                    }
                    armed => armed?,
                };
                self.forget_held_said(unit, &[n]);
                Ok(Some(n))
            }
            Arming::Disarm { weapon } => {
                let freed = self.conditions.disarm(unit, weapon.as_deref());
                self.forget_held_said(unit, &freed);
                Ok(None)
            }
        }
    }

    /// **The notice a refused arming is told as** (17b cut M1b.1): the
    /// refused order's notice in `Arena::refusal_notice`'s one shape — word
    /// `fire`, the arming's weapon, the detail read at `at_ms` — with one
    /// refinement: an [`Refusal::Unlawful`] whose odds term met the floor was
    /// refused for its **target kind** (a [`SlotTarget::Contact`]), so it
    /// carries no floor and its sentence is the fallback that names none
    /// (`The Compact floor forbids that condition.`), for a held id and an
    /// unheld one alike. Staged by phase 4 and appended by a mind's landing.
    pub(crate) fn arming_refusal_notice(
        &self,
        unit: ObjId,
        arming: &Arming,
        refusal: Refusal,
        at_ms: i64,
    ) -> crate::arena::perception::FrameNotice {
        let notice = self.refusal_notice(
            unit,
            IntentKind::Condition(arming.clone()).as_str(),
            refusal,
            arming.weapon().cloned(),
            at_ms,
        );
        let target_kind = matches!(
            arming,
            Arming::Arm(cond) if u32::from(cond.min_permille) >= self.odds_floor_permille
        );
        match notice {
            crate::arena::perception::FrameNotice::Refused {
                order,
                refusal: Refusal::Unlawful,
                weapon,
                ..
            } if target_kind => crate::arena::perception::FrameNotice::Refused {
                order,
                refusal: Refusal::Unlawful,
                weapon,
                detail: crate::arena::perception::RefusalDetail::None,
            },
            other => other,
        }
    }

    /// **Whether `unit`'s own sensor pass at `t_ms` wrote its lock's row**
    /// (17b cut M1b.1; 17b §1(d)): the lock's row stamped `last_seen_ms ==
    /// t_ms` by this platform's own sighting (`own_seen` at `t_ms`) — the
    /// guard on the `SensorPass` trigger, so a pass that saw nothing of the
    /// lock evaluates nothing.
    pub(crate) fn own_pass_wrote_lock(&self, unit: ObjId, t_ms: i64) -> bool {
        let Some(lock) = self.unit(unit).and_then(|state| state.lock) else {
            return false;
        };
        self.contacts.get(unit, lock).is_some_and(|held| {
            held.last_seen_ms == t_ms && held.own_seen.is_some_and(|own| own.at_ms == t_ms)
        })
    }

    /// **Every slot of `unit` and its held-shot rows gone** (17b cut M1b.1): a
    /// controller transition (`Arena::set_controller`) and a wreck
    /// (`Arena::destroy_unit`).
    pub(crate) fn clear_conditions(&mut self, unit: ObjId) {
        if self.conditions.is_empty_for(unit)
            && !self.held_said.keys().any(|(on, _, _)| *on == unit)
        {
            return;
        }
        self.conditions.clear(unit);
        self.held_said.retain(|(on, _, _), _| *on != unit);
    }

    /// **Say why a slot held, at most once per slot and contact per
    /// [`HELD_SAID_MS`]** (17b cut M1b.4; doc 17 §5: *a held shot is said at
    /// most once per contact per 10 s*).
    ///
    /// The contact is the one the why names — an [`HeldWhy::Aspect`] or
    /// [`HeldWhy::Range`]'s own — and the hull's lock for an
    /// [`HeldWhy::Odds`] or [`HeldWhy::Rounds`] (the warrant only reaches those
    /// terms with the lock held). [`Arena::held_said`] keeps the millisecond
    /// each `(unit, slot, contact)` was last SAID: the sentence is staged, and
    /// the row written, only when the row is absent or at least
    /// [`HELD_SAID_MS`] old — so a slot held every pass is said once, then
    /// once more ten seconds on, never once a pass. A disarm or a re-arm drops
    /// the slot's rows ([`Arena::resolve_arming`]), so new terms say their
    /// first reason at once. Staged through `Arena::stage_notice` on the
    /// shooter's hull alone — a mind hull's buffer receives it as every
    /// notice (the parity rule); nothing is told to any other hull. Written at
    /// an arena millisecond, so the rate is the tempo's to keep by
    /// construction (D-16-2).
    fn say_held(&mut self, unit: ObjId, slot: u8, weapon: &SmolStr, why: &HeldWhy, t_ms: i64) {
        let contact = match why {
            HeldWhy::Aspect { contact, .. } | HeldWhy::Range { contact, .. } => Some(contact.id),
            HeldWhy::Odds { .. } | HeldWhy::Rounds { .. } => {
                self.unit(unit).and_then(|state| state.lock)
            }
        };
        let Some(contact) = contact else {
            return;
        };
        let key = (unit, slot, contact);
        if self
            .held_said
            .get(&key)
            .is_some_and(|last| t_ms.saturating_sub(*last) < HELD_SAID_MS)
        {
            return;
        }
        self.held_said.insert(key, t_ms);
        self.stage_notice(
            unit,
            FrameNotice::SlotHeld {
                slot,
                weapon: weapon.clone(),
                why: why.clone(),
            },
        );
    }

    /// **`unit`'s condition row cut to `capacity` slots** (17b cut M3.4; doc 17
    /// §5, §9; D-UM-16, D-UM-32, D-UM-34): [`ConditionTable::truncate`], and
    /// the held-shot rows of every armed slot it cleared dropped with them
    /// (`Arena::forget_held_said`), so a slot armed past the cut leaves no
    /// stamp behind. Called with Class 0's `slots` (one) at every drop to the
    /// ROM (`Arena::mind_think`, `Arena::mind_land`) — the ROM's one slot,
    /// whatever the hardware still holds — and through
    /// [`Arena::truncate_slots`] by the core's two consequences. A hull with no
    /// row, or one already that narrow, is untouched. Nothing is journalled:
    /// the slots are state, folded by the hash (item 3d).
    pub fn truncate_slots_to(&mut self, unit: ObjId, capacity: usize) {
        let cleared = self.conditions.truncate(unit, capacity);
        self.forget_held_said(unit, &cleared);
    }

    /// **`unit`'s condition row cut to its core's slots NOW** (17b cut M3.4;
    /// D-UM-34): [`Arena::truncate_slots_to`] at [`Arena::slot_capacity`] —
    /// what `degrade_core` (the lowered class's slots) and `destroy_core`
    /// (Class 0's one) leave the hull.
    pub fn truncate_slots(&mut self, unit: ObjId) {
        let capacity = self.slot_capacity(unit);
        self.truncate_slots_to(unit, capacity);
    }

    /// **Whether `unit` is a mind's hull the ROM thinks for** (17b cut M3.4;
    /// D-UM-32, D-UM-33): `Controller::Mind`, a mind row, and a mind class of
    /// `Zero` ([`Arena::mind_class`]: the row dropped, no core, or a core at
    /// Class 0) — the hull whose every think is the native stock mind at
    /// Class 0's row.
    fn thinks_on_rom(&self, unit: ObjId) -> bool {
        self.unit(unit)
            .is_some_and(|state| state.controller == crate::arena::state::Controller::Mind)
            && self.minds.contains_key(&unit)
            && self.mind_class(unit) == crate::arena::core::CoreClass::Zero
    }

    /// **The slots an arming on `unit` may lay down** (17b cut M3.4): the
    /// hull's own ([`Arena::slot_capacity`]) — except on a mind's hull the
    /// ROM thinks for ([`Arena::thinks_on_rom`]), where it is at most Class
    /// 0's `slots`: the ROM thinks at Class 0's row and holds its one slot
    /// whatever the hardware still holds, and [`ConditionTable::arm`] would
    /// otherwise widen the row a drop cut back to the hardware's width (on a
    /// hull with no core, or a core at Class 0, the hardware's slots are Class
    /// 0's already). A seated pilot (a possessed hull, 17b cut M3.5) arms on
    /// the hardware's slots.
    fn arming_capacity(&self, unit: ObjId) -> usize {
        let hardware = self.slot_capacity(unit);
        if self.thinks_on_rom(unit) {
            hardware.min(self.classes.row(crate::arena::core::CoreClass::Zero).slots)
        } else {
            hardware
        }
    }

    /// **The ROM takes back a slot the failed mind left** (17b cut M3.4;
    /// D-UM-32: the ROM at the fallback preset), for an arming of `cond` that
    /// [`ConditionTable::arm`] refused [`Refusal::NoSlot`] on a hull the ROM
    /// thinks for. A drop keeps the row's first slots (cut to Class 0's
    /// `slots`, `Arena::truncate_slots_to`) armed on the failed mind's own
    /// terms, and a core lowered to Class 0 keeps them on the higher class's;
    /// the ROM arms the hull's FIRST `capacity` weapons in their order
    /// (`stock_mind::arming_for`), so a kept slot on another gun would refuse
    /// every one of its armings `NoSlot` and keep firing on terms the ROM
    /// never set. So: when `cond`'s weapon is one of the hull's first
    /// `capacity`, the first slot within the capacity armed with a weapon that
    /// is NOT is disarmed (its held-shot rows with it) and the arming laid
    /// there. Anything else stays `NoSlot` — a weapon the ROM does not arm
    /// takes no slot from it, and a row full of the ROM's own guns gives none
    /// up.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSlot`], as the refused arming was.
    fn rom_takes_slot(
        &mut self,
        unit: ObjId,
        cond: &FireCondition,
        capacity: usize,
    ) -> Result<u8, Refusal> {
        let own: Vec<SmolStr> = self
            .unit(unit)
            .map(|state| {
                state
                    .weapons
                    .iter()
                    .take(capacity)
                    .map(|weapon| weapon.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        if !own.contains(&cond.weapon) {
            return Err(Refusal::NoSlot);
        }
        let holds = capacity.min(crate::arena::core::MAX_CONDITION_SLOTS);
        let Some(foreign) = self
            .conditions
            .slots(unit)
            .iter()
            .take(holds)
            .flatten()
            .find(|had| !own.contains(&had.weapon))
            .map(|had| had.weapon.clone())
        else {
            return Err(Refusal::NoSlot);
        };
        let freed = self.conditions.disarm(unit, Some(foreign.as_str()));
        self.forget_held_said(unit, &freed);
        self.conditions.arm(unit, cond.clone(), capacity)
    }

    /// **The held-shot rows of `unit`'s `slots` gone** (17b cut M1b.1).
    pub(crate) fn forget_held_said(&mut self, unit: ObjId, slots: &[u8]) {
        if slots.is_empty() {
            return;
        }
        self.held_said
            .retain(|(on, slot, _), _| !(*on == unit && slots.contains(slot)));
    }
}

/// **An arming** (17b cut M1b.1; D-UM-10: one intent, two principals) — the
/// payload of [`IntentKind::Condition`]: a seat's `FIRE <weapon> WHEN …` (17b
/// cut M1b.4 stages it) and a mind's `arm`/`disarm` alike.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Arming {
    /// Arm a slot with these terms (a weapon already armed is re-armed in
    /// place).
    Arm(FireCondition),
    /// Free the named weapon's slot, or every slot for `None`.
    Disarm {
        /// The weapon, or every slot.
        weapon: Option<SmolStr>,
    },
}

impl Arming {
    /// **The weapon this arming names** — the arm's, or the disarm's named one
    /// — so a staged refusal reads `FIRE cannon refused. …`.
    #[must_use]
    pub fn weapon(&self) -> Option<&SmolStr> {
        match self {
            Arming::Arm(cond) => Some(&cond.weapon),
            Arming::Disarm { weapon } => weapon.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::{Controller, Power, SeenPose, ShadowContact, Team};
    use crate::arena::test_support::{arena_of, cannon, line_tank, pose};
    use crate::arena::weapons::FiredBy;

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);
    const FRIEND: ObjId = ObjId(5);

    fn slot(weapon: &str, min_permille: u16) -> FireCondition {
        FireCondition {
            weapon: SmolStr::new(weapon),
            target: SlotTarget::Lock,
            min_permille,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: u32::MAX,
        }
    }

    /// The contact id `viewer` holds `target` under, in the test arena.
    fn id_of(viewer: ObjId, target: ObjId) -> ContactId {
        crate::arena::rng::draw_contact_id(2_350, viewer, target)
    }

    /// Red and blue line tanks four cells apart, a friend beside red, both
    /// rows hand-held classified, red online and locked on blue, odds pinned
    /// at 500 per mille.
    fn locked() -> Arena {
        let mut arena = arena_of(vec![
            line_tank(RED, Team::Red, pose(1_500, 3_500)),
            line_tank(BLUE, Team::Blue, pose(5_500, 3_500)),
            line_tank(FRIEND, Team::Red, pose(1_500, 5_500)),
        ]);
        arena.resolution.p_min = 500;
        arena.resolution.p_max = 500;
        for (target, team) in [(BLUE, Team::Blue), (FRIEND, Team::Red)] {
            let plot = arena.unit(target).map(SeenPose::of).unwrap_or_default();
            arena.contacts.insert(
                RED,
                id_of(RED, target),
                ShadowContact::sighted(target, Classification::Classified { team }, 0, plot),
            );
        }
        if let Some(state) = arena.unit_mut(RED) {
            state.power = Power::Online;
        }
        assert_eq!(arena.resolve_lock(RED, id_of(RED, BLUE), 0), Ok(()));
        arena
    }

    /// **The table**: armings take the first free slot, a weapon already armed
    /// is re-armed in place with its number, the count refuses `NoSlot`, a
    /// disarm leaves the other's number, `None` frees every slot and drops the
    /// key, `clear_weapons` frees by weapon. On a Class I hull's two slots
    /// (17b cut M3.2: the capacity handed in; a hull with no row reads an
    /// empty row, and the first arming lays the row down two wide).
    #[test]
    fn m1b1_the_table_numbers_slots_stably() {
        const STOCK_CONDITION_SLOTS: usize = crate::arena::core::ClassTable::STOCK
            .row(crate::arena::core::CoreClass::I)
            .slots;
        let mut table = ConditionTable::default();
        assert!(table.is_empty_for(RED));
        assert!(table.slots(RED).is_empty(), "no row, no slot");
        assert_eq!(
            table.arm(RED, slot("cannon", 100), STOCK_CONDITION_SLOTS),
            Ok(1)
        );
        assert_eq!(
            table.slots(RED).len(),
            STOCK_CONDITION_SLOTS,
            "laid two wide"
        );
        assert_eq!(
            table.arm(RED, slot("machine_gun", 100), STOCK_CONDITION_SLOTS),
            Ok(2)
        );
        assert_eq!(
            table.arm(RED, slot("missiles", 100), STOCK_CONDITION_SLOTS),
            Err(Refusal::NoSlot)
        );
        assert_eq!(
            table.arm(RED, slot("cannon", 300), STOCK_CONDITION_SLOTS),
            Ok(1),
            "in place"
        );
        assert_eq!(
            table.slots(RED).first().cloned().flatten(),
            Some(slot("cannon", 300))
        );
        assert_eq!(table.slot_of(RED, "machine_gun"), Some(2));
        assert_eq!(table.disarm(RED, Some("cannon")), vec![1]);
        assert_eq!(table.slot_of(RED, "machine_gun"), Some(2), "stable");
        assert_eq!(
            table.arm(RED, slot("missiles", 100), STOCK_CONDITION_SLOTS),
            Ok(1)
        );
        assert_eq!(table.disarm(RED, Some("nothing")), Vec::<u8>::new());
        assert_eq!(
            table.clear_weapons(RED, &[SmolStr::new("missiles")]),
            vec![1]
        );
        assert_eq!(table.slot_of(RED, "machine_gun"), Some(2));
        assert_eq!(table.rows().count(), 1);
        assert_eq!(table.disarm(RED, None), vec![2]);
        assert!(table.is_empty_for(RED), "the key goes with the last slot");
        assert_eq!(table.disarm(RED, None), Vec::<u8>::new());
        let _ = table.arm(BLUE, slot("cannon", 100), STOCK_CONDITION_SLOTS);
        table.clear(BLUE);
        assert!(table.is_empty_for(BLUE));
    }

    /// **17b cut M3.1** (D-UM-35): an arming is held to the hull's capacity —
    /// one slot holds one gun (the second refused `NoSlot`, the in-place
    /// re-arm still taken), six hold six (17b cut M3.2: the row is laid down
    /// at the capacity, so a third gun takes slot 3 where the M3.1 row's
    /// fixed width refused it), and none refuses with no row laid down. On the arena, a hull with no core
    /// (Class 0) holds one and a Class I hull two (`Arena::slot_capacity`),
    /// through `Arena::resolve_arming` as a pilot's `FIRE … WHEN` and a
    /// mind's landing arm.
    #[test]
    fn m3_an_arming_is_held_to_the_hulls_capacity() {
        use crate::arena::core::{ClassTable, CoreClass};
        use crate::arena::test_support::{arena_of, cannon, laser, pose, scout, tactical_unit};
        let mut table = ConditionTable::default();
        assert_eq!(table.arm(RED, slot("cannon", 100), 1), Ok(1));
        assert_eq!(
            table.arm(RED, slot("machine_gun", 100), 1),
            Err(Refusal::NoSlot)
        );
        assert_eq!(table.arm(RED, slot("cannon", 300), 1), Ok(1), "in place");
        assert_eq!(table.arm(BLUE, slot("cannon", 100), 6), Ok(1));
        assert_eq!(table.arm(BLUE, slot("machine_gun", 100), 6), Ok(2));
        assert_eq!(
            table.arm(BLUE, slot("missiles", 100), 6),
            Ok(3),
            "the core's six (17b cut M3.2)"
        );
        assert_eq!(table.slots(BLUE).len(), 6);
        let mut none = ConditionTable::default();
        assert_eq!(none.arm(RED, slot("cannon", 100), 0), Err(Refusal::NoSlot));
        assert!(none.is_empty_for(RED), "no row laid down");

        // Two guns on each hull, so the capacity is what refuses.
        let two_guns = |unit: crate::arena::state::CustodyUnit| {
            crate::arena::state::CustodyUnit {
                weapons: vec![cannon(), laser()],
                ..unit
            }
            .sealed()
        };
        let bare = crate::arena::state::CustodyUnit {
            unit: tactical_unit(BLUE),
            ..two_guns(scout(BLUE, Team::Blue, pose(6_500, 4_500)))
        }
        .sealed();
        let mut arena = arena_of(vec![
            two_guns(scout(RED, Team::Red, pose(1_500, 4_500))),
            bare,
        ]);
        assert_eq!(
            arena.slot_capacity(RED),
            ClassTable::STOCK.row(CoreClass::I).slots
        );
        assert_eq!(
            arena.slot_capacity(BLUE),
            ClassTable::STOCK.row(CoreClass::Zero).slots
        );
        // And through the arena's own arming (`Arena::resolve_arming`, which
        // reads `slot_capacity` at the arm): the Class I hull takes both guns,
        // the coreless one the first alone, the second refused `NoSlot` with
        // nothing laid down for it, and the first still re-armed in place.
        let arm = |weapon: &str| Arming::Arm(slot(weapon, 1_000));
        assert_eq!(arena.resolve_arming(RED, &arm("cannon"), 0), Ok(Some(1)));
        assert_eq!(arena.resolve_arming(RED, &arm("laser"), 0), Ok(Some(2)));
        assert_eq!(arena.resolve_arming(BLUE, &arm("cannon"), 0), Ok(Some(1)));
        assert_eq!(
            arena.resolve_arming(BLUE, &arm("laser"), 0),
            Err(Refusal::NoSlot),
            "a coreless hull holds one slot"
        );
        assert_eq!(arena.conditions.slot_of(BLUE, "laser"), None);
        assert_eq!(arena.resolve_arming(BLUE, &arm("cannon"), 0), Ok(Some(1)));
    }

    /// **17b cut M3.2** (D-UM-16; plan F5): the row is laid down at the
    /// capacity and clamped to `MAX_CONDITION_SLOTS`; a narrow row widens to
    /// a wider capacity; `truncate` clears the slots past a capacity,
    /// answering the ARMED ones' numbers in order, narrows the row and drops
    /// the key when nothing armed is left; a hull with no row truncates to
    /// nothing; an arming past a wider row's capacity takes no slot past it.
    #[test]
    fn m3_the_row_is_the_cores_width_and_truncates() {
        use crate::arena::core::MAX_CONDITION_SLOTS;
        let mut table = ConditionTable::default();
        for (n, weapon) in ["a", "b", "c", "d", "e", "f"].into_iter().enumerate() {
            assert_eq!(
                table.arm(RED, slot(weapon, 100), 9),
                Ok(u8::try_from(n + 1).unwrap())
            );
        }
        assert_eq!(
            table.slots(RED).len(),
            MAX_CONDITION_SLOTS,
            "clamped to six"
        );
        assert_eq!(table.arm(RED, slot("g", 100), 9), Err(Refusal::NoSlot));
        // Free 4; cut to 3: slots 5 and 6 armed, 4 free — answered 5 and 6.
        assert_eq!(table.disarm(RED, Some("d")), vec![4]);
        assert_eq!(table.truncate(RED, 3), vec![5, 6]);
        assert_eq!(table.slots(RED).len(), 3);
        assert_eq!(table.slot_of(RED, "c"), Some(3));
        assert_eq!(table.slot_of(RED, "e"), None);
        assert_eq!(
            table.truncate(RED, 3),
            Vec::<u8>::new(),
            "already that narrow"
        );
        // A narrow row widens to a wider capacity.
        assert_eq!(table.arm(RED, slot("e", 100), 4), Ok(4));
        assert_eq!(table.slots(RED).len(), 4);
        // A wider row takes no slot past a narrower capacity.
        assert_eq!(table.disarm(RED, Some("a")), vec![1]);
        assert_eq!(table.arm(BLUE, slot("a", 100), 2), Ok(1));
        let mut wide = ConditionTable::default();
        assert_eq!(wide.arm(RED, slot("a", 100), 4), Ok(1));
        assert_eq!(wide.arm(RED, slot("b", 100), 1), Err(Refusal::NoSlot));
        assert_eq!(wide.arm(RED, slot("a", 200), 1), Ok(1), "in place");
        // Down to nothing armed: the key goes.
        assert_eq!(wide.truncate(RED, 0), vec![1]);
        assert!(wide.is_empty_for(RED));
        assert_eq!(wide.truncate(BLUE, 1), Vec::<u8>::new(), "no row");
    }

    /// **The mask**: one bit per facet, `ASPECTS_ANY` all four.
    #[test]
    fn m1b1_the_aspect_mask_is_one_bit_a_facet() {
        use crate::arena::perception::Aspect;
        assert_eq!(
            aspect_mask(&[Aspect::Front, Aspect::Left, Aspect::Right, Aspect::Rear]),
            ASPECTS_ANY
        );
        assert_eq!(aspect_mask(&[]), 0);
        let side = aspect_mask(&[Aspect::Left, Aspect::Right]);
        assert_eq!(side, 0b0110);
        assert!(aspect_in(side, Aspect::Right));
        assert!(!aspect_in(side, Aspect::Rear));
    }

    /// **The floor's static terms**: 90, 100 and 110 per mille against a
    /// floor of 100, and a `Contact` target unlawful whatever its terms.
    #[test]
    fn m1b1_the_floor_refuses_under_it_and_any_contact_target() {
        assert_eq!(
            condition_is_lawful(&slot("cannon", 90), 100),
            Err(Refusal::Unlawful)
        );
        assert_eq!(condition_is_lawful(&slot("cannon", 100), 100), Ok(()));
        assert_eq!(condition_is_lawful(&slot("cannon", 110), 100), Ok(()));
        let contact = FireCondition {
            target: SlotTarget::Contact { id: ContactId(1) },
            ..slot("cannon", 1_000)
        };
        assert_eq!(condition_is_lawful(&contact, 10), Err(Refusal::Unlawful));
        let best = FireCondition {
            target: SlotTarget::BestHostile,
            ..slot("cannon", 100)
        };
        assert_eq!(condition_is_lawful(&best, 100), Ok(()));
        assert_eq!(
            Arming::Arm(slot("cannon", 100))
                .weapon()
                .map(SmolStr::as_str),
            Some("cannon")
        );
        assert_eq!(Arming::Disarm { weapon: None }.weapon(), None);
    }

    /// **The three target rules**: an unclassified lock is idle for every
    /// target; `BestHostile` is idle on a friendly; a `Contact` slot fires
    /// only when the lock is that id; and — 17b cut M2.2, the lead's ruling
    /// (a) — a lock that reads dead is idle for ALL THREE targets (17b cut
    /// M1b.1 idled `BestHostile` alone there and kept `Lock` warranted).
    #[test]
    fn m1b1_the_target_rules_read_the_lock_and_its_class() {
        let mut arena = locked();
        let lock = id_of(RED, BLUE);
        assert!(arena.warrant(RED, &slot("cannon", 100), 0).is_ok());
        let best = FireCondition {
            target: SlotTarget::BestHostile,
            ..slot("cannon", 100)
        };
        assert!(arena.warrant(RED, &best, 0).is_ok());
        let on = |id| FireCondition {
            target: SlotTarget::Contact { id },
            ..slot("cannon", 100)
        };
        assert!(arena.warrant(RED, &on(lock), 0).is_ok());
        assert_eq!(arena.warrant(RED, &on(ContactId(1)), 0), Err(Held::Idle));
        // 17b cut M2.2, ruling (a): a row that reads dead is no target for
        // any of the three — the best hostile, the plain lock, nor the
        // contact named by its own id.
        if let Some(held) = arena.contacts.get_mut(RED, lock) {
            held.known_dead = true;
        }
        assert_eq!(arena.warrant(RED, &best, 0), Err(Held::Idle));
        assert_eq!(arena.warrant(RED, &slot("cannon", 100), 0), Err(Held::Idle));
        assert_eq!(arena.warrant(RED, &on(lock), 0), Err(Held::Idle));
        if let Some(held) = arena.contacts.get_mut(RED, lock) {
            held.known_dead = false;
        }
        assert!(arena.warrant(RED, &slot("cannon", 100), 0).is_ok());
        assert!(arena.warrant(RED, &on(lock), 0).is_ok());
        // A friendly lock.
        let _ = arena.resolve_lock(RED, id_of(RED, FRIEND), 0);
        assert_eq!(arena.warrant(RED, &best, 0), Err(Held::Idle));
        // An unclassified lock is idle for every target.
        let _ = arena.resolve_lock(RED, lock, 0);
        if let Some(held) = arena.contacts.get_mut(RED, lock) {
            held.class = Classification::Unknown;
            held.known_dead = false;
        }
        for target in [slot("cannon", 100), best, on(lock)] {
            assert_eq!(arena.warrant(RED, &target, 0), Err(Held::Idle));
        }
        // No such weapon: idle.
        assert_eq!(arena.warrant(RED, &slot("laser", 100), 0), Err(Held::Idle));
    }

    /// **A hull with no slot touches nothing**: every trigger answers empty,
    /// leaves the action hash outside and counts nothing — the property the
    /// production line-hash pin rests on (17b cut M1b.0's note).
    ///
    /// This pins the EVALUATOR's own early return, called directly once per
    /// [`ConditionTrigger`]; it does not drive the six call sites in `step.rs`
    /// and `mind.rs`. Those open no action hash of their own before the call
    /// (each passes straight to `evaluate_conditions`, or to its
    /// `is_empty_for` guard first), a site that TOOK the hash early would show
    /// in the production line pins' next action line, and
    /// `tests/conditions.rs`' `m1b1_the_production_hash_and_line_hashes_do_not_move`
    /// drives them end to end on a match with no slot (no slot row and no held
    /// row after any pulse; the floor folding nothing at any pulse).
    #[test]
    fn m1b1_a_hull_with_no_slot_takes_no_hash_at_any_trigger() {
        let mut arena = locked();
        let before = crate::arena::hash::state_hash(&arena);
        let mut counters = ReflexCounters::default();
        for trigger in [
            ConditionTrigger::WeaponReady,
            ConditionTrigger::Pass,
            ConditionTrigger::Landing,
            ConditionTrigger::Lock,
            ConditionTrigger::Arming,
            ConditionTrigger::Startup,
        ] {
            assert_eq!(arena.action_hash, ActionHash::Outside);
            let answers = arena.evaluate_conditions(RED, 0, trigger, Some("cannon"), &mut counters);
            assert!(answers.is_empty(), "{trigger:?}");
            assert_eq!(arena.action_hash, ActionHash::Outside, "{trigger:?}");
        }
        assert_eq!(counters, ReflexCounters::default());
        assert_eq!(crate::arena::hash::state_hash(&arena), before);
    }

    /// **The evaluator**: a held term is `Why` and counted and takes no hash;
    /// another weapon's ready event evaluates nothing; a warranted slot fires
    /// `by: cond` once, opening the action hash only then; a cycling gun is
    /// idle while `warrant` itself still answers `Ok` (readiness is never a
    /// warrant term).
    #[test]
    fn m1b1_the_evaluator_idles_a_cycling_gun_and_fires_a_warranted_slot() {
        let mut arena = locked();
        let mut counters = ReflexCounters::default();
        assert_eq!(
            arena.resolve_arming(RED, &Arming::Arm(slot("cannon", 600)), 0),
            Ok(Some(1))
        );
        let answers =
            arena.evaluate_conditions(RED, 0, ConditionTrigger::Pass, None, &mut counters);
        assert_eq!(
            answers,
            vec![(
                1,
                Err(Held::Why(HeldWhy::Odds {
                    permille: 500,
                    floor: 600
                }))
            )]
        );
        assert_eq!(counters.cond_held, 1);
        assert_eq!(
            arena.action_hash,
            ActionHash::Outside,
            "a held slot takes no hash"
        );
        assert!(
            arena
                .evaluate_conditions(
                    RED,
                    0,
                    ConditionTrigger::WeaponReady,
                    Some("laser"),
                    &mut counters
                )
                .is_empty()
        );
        let _ = arena.resolve_arming(RED, &Arming::Arm(slot("cannon", 100)), 0);
        let answers =
            arena.evaluate_conditions(RED, 0, ConditionTrigger::Arming, None, &mut counters);
        assert!(matches!(answers.as_slice(), [(1, Ok(1))]), "{answers:?}");
        assert_eq!((counters.cond_fires, counters.shots), (1, 1));
        assert_ne!(arena.action_hash, ActionHash::Outside);
        let answers =
            arena.evaluate_conditions(RED, 1, ConditionTrigger::Pass, None, &mut counters);
        assert_eq!(answers, vec![(1, Err(Held::Idle))]);
        assert!(arena.warrant(RED, &slot("cannon", 100), 1).is_ok());
        assert_eq!(counters.cond_fires, 1);
    }

    /// **`resolve_fire_by`'s gates for a slot's own shot**: `Cycling` and
    /// `Empty` answer before the hold-fire term, the hold-fire term answers
    /// `Unwarranted` before the power, and a seat's and a machine's shot are
    /// never gated (a mind's is, from 17b cut M1b.2).
    #[test]
    fn m1b1_a_slot_shot_reads_cycling_and_empty_before_the_law() {
        use crate::deliberation::order::{
            CharacterRef, Constraints, Objective, Order, StandingOrder,
        };
        let squad = crate::deliberation::squad::SquadId {
            team: Team::Red,
            index: 1,
        };
        let cond = FiredBy::Cond { slot: 1 };
        let mut arena = locked();
        arena.set_controller(
            RED,
            Controller::Reflex,
            crate::arena::reflex::ReflexState::Patrol,
        );
        let _ = arena.resolve_lock(RED, id_of(RED, BLUE), 0);
        arena.reflex.set_order(
            RED,
            StandingOrder::accepted(
                Order {
                    squad,
                    objective: Objective::Hold,
                    target: None,
                    horizon_ms: 600_000,
                    constraints: Constraints {
                        hold_fire: true,
                        ..Constraints::default()
                    },
                    ttl_ms: 600_000,
                    issued_ms: 0,
                    issuer: None,
                    by: CharacterRef::Persona { squad },
                    spot: None,
                },
                0,
            ),
        );
        assert!(arena.holds_fire(RED));
        assert_eq!(
            arena.clone().resolve_fire_by(RED, "cannon", 0, cond),
            Err(Refusal::Unwarranted),
            "hold-fire stops a slot"
        );
        for by in [FiredBy::Seat, FiredBy::Machine] {
            assert!(
                arena.clone().resolve_fire_by(RED, "cannon", 0, by).is_ok(),
                "{by:?}"
            );
        }
        // 17b cut M1b.2 put the law on a mind's shot (`weapons.rs`' in-source
        // tests pin its order): uncited, or citing a slot this hull has not
        // armed, it has no warrant.
        for by in [
            FiredBy::Mind { slot: None },
            FiredBy::Mind { slot: Some(1) },
        ] {
            assert_eq!(
                arena.clone().resolve_fire_by(RED, "cannon", 0, by),
                Err(Refusal::Unwarranted),
                "{by:?}"
            );
        }
        let mut cycling = arena.clone();
        if let Some(state) = cycling.unit_mut(RED) {
            state.ready_at_ms.insert(SmolStr::new("cannon"), 4_000);
        }
        assert_eq!(
            cycling.resolve_fire_by(RED, "cannon", 0, cond),
            Err(Refusal::Cycling)
        );
        let mut empty = arena.clone();
        if let Some(state) = empty.unit_mut(RED) {
            state.ammo.insert(SmolStr::new("cannon"), 0);
        }
        assert_eq!(
            empty.resolve_fire_by(RED, "cannon", 0, cond),
            Err(Refusal::Empty)
        );
        let mut cold = arena.clone();
        if let Some(state) = cold.unit_mut(RED) {
            state.power = Power::Off;
        }
        assert_eq!(
            cold.resolve_fire_by(RED, "cannon", 0, cond),
            Err(Refusal::Unwarranted)
        );
        // No order: the slot's shot is trusted.
        let mut free = locked();
        assert!(!free.holds_fire(RED), "a pilot's hull holds no order");
        assert_eq!(free.resolve_fire_by(RED, "cannon", 0, cond), Ok(1));
    }

    /// **The arming's landing**: unknown, no weapon, unlawful (the law before
    /// the count), wreck; a re-arm drops its slot's `held_said` rows and a
    /// disarm its own; the refusal notice for a `Contact` target carries no
    /// floor and the floor's carries the arena's.
    #[test]
    fn m1b1_the_arming_lands_in_its_order() {
        use crate::arena::perception::RefusalDetail;
        let mut arena = locked();
        let arm = |cond: FireCondition| Arming::Arm(cond);
        assert_eq!(
            arena.resolve_arming(ObjId(77), &arm(slot("cannon", 100)), 0),
            Err(Refusal::Unknown)
        );
        assert_eq!(
            arena.resolve_arming(RED, &arm(slot("laser", 100)), 0),
            Err(Refusal::NoWeapon)
        );
        assert_eq!(
            arena.resolve_arming(RED, &arm(slot("cannon", 50)), 0),
            Err(Refusal::Unlawful)
        );
        let notice =
            arena.arming_refusal_notice(RED, &arm(slot("cannon", 50)), Refusal::Unlawful, 0);
        assert_eq!(notice.detail(), RefusalDetail::Floor { permille: 100 });
        let contact = FireCondition {
            target: SlotTarget::Contact { id: ContactId(1) },
            ..slot("cannon", 100)
        };
        let notice = arena.arming_refusal_notice(RED, &arm(contact), Refusal::Unlawful, 0);
        assert_eq!(notice.detail(), RefusalDetail::None);
        assert_eq!(notice.weapon().map(SmolStr::as_str), Some("cannon"));
        assert_eq!(
            arena.resolve_arming(RED, &arm(slot("cannon", 100)), 0),
            Ok(Some(1))
        );
        arena.held_said.insert((RED, 1, ContactId(3)), 10);
        assert_eq!(
            arena.resolve_arming(RED, &arm(slot("cannon", 200)), 0),
            Ok(Some(1))
        );
        assert!(arena.held_said.is_empty(), "a re-arm drops its rows");
        arena.held_said.insert((RED, 1, ContactId(3)), 10);
        assert_eq!(
            arena.resolve_arming(RED, &Arming::Disarm { weapon: None }, 0),
            Ok(None)
        );
        assert!(arena.held_said.is_empty(), "a disarm drops its rows");
        let _ = arena.resolve_arming(RED, &arm(slot("cannon", 100)), 0);
        arena.held_said.insert((RED, 1, ContactId(3)), 10);
        let mut journal = Vec::new();
        arena.destroy_unit(RED, 0, None, &mut journal);
        assert!(arena.conditions.is_empty_for(RED), "the wreck's slots went");
        assert!(arena.held_said.is_empty(), "and the wreck's held rows");
        assert_eq!(
            arena.resolve_arming(RED, &arm(slot("cannon", 100)), 0),
            Err(Refusal::Wreck)
        );
    }

    /// **The evaluator's cost on a three-hundred-hull field** (17b cut M1b.1;
    /// 17b §1(c); report-only: `cargo test --release -p tacsim --lib
    /// m1b1_the_evaluator_cost -- --nocapture`).
    ///
    /// 300 line tanks on the production window (`stock_definition`'s
    /// 256-cell square), each with two weapons, two slots armed whose odds
    /// term holds them (so every evaluation runs the whole warrant — one
    /// `aim_at` trace and one `resolve_seen` per slot — and fires nothing) and
    /// a classified lock. Arm 1: one `WeaponReady` evaluation per hull (one
    /// slot). Arm 2: one `Pass` evaluation per hull (two slots). Arm 3: the
    /// `Share` trigger as the due loop runs it — 150 red hulls a side in one
    /// share mesh, each share refreshing its mates' lock rows and evaluating
    /// every receiver whose lock it refreshed. The arithmetic: 300 hulls × 2
    /// slots × one trace is about 600 traces in a worst-case window, against
    /// the harness's 2,994-trace peaks per wake. Wall microseconds p50 and p99
    /// per call are printed; nothing is asserted.
    #[test]
    fn m1b1_the_evaluator_cost_on_a_three_hundred_hull_field() {
        use std::time::Instant;
        let mut machine_gun = cannon();
        machine_gun.id = SmolStr::new("machine_gun");
        machine_gun.cycle_ms = 1_000;
        let mut units = Vec::new();
        for i in 0..150_u16 {
            for (team, x, id) in [
                (Team::Red, 100_u16, 10_000 + u32::from(i)),
                (Team::Blue, 110, 20_000 + u32::from(i)),
            ] {
                let mut unit = line_tank(
                    ObjId(id),
                    team,
                    pose(i64::from(x) * 1_000 + 500, i64::from(50 + i) * 1_000 + 500),
                );
                unit.weapons = vec![cannon(), machine_gun.clone()];
                unit.reseal(&crate::match_::SealTerms::fixture())
                    .expect("the fixture terms seal");
                units.push(unit);
            }
        }
        let definition = crate::match_::stock_definition(&SmolStr::new(crate::match_::STAND_IN))
            .expect("the production window");
        let mut arena = Arena::new(
            crate::arena::state::CustodyOut {
                definition,
                units,
                pilots: Vec::new(),
                resolution: crate::arena::test_support::resolution(),
                seed: 2_350,
                mode: crate::arena::MatchMode::Skirmish,
            },
            &crate::config::ArenaConfig::default(),
            &crate::config::MatchConfig::default(),
            &crate::config::SensorsConfig::default(),
        )
        .expect("three hundred hulls");
        arena.resolution.p_min = 500;
        arena.resolution.p_max = 500;
        let ids: Vec<ObjId> = arena.units.iter().map(|unit| unit.id).collect();
        let blue0 = ObjId(20_000);
        for id in &ids {
            let red = id.0 < 20_000;
            let target = if red { blue0 } else { ObjId(id.0 - 10_000) };
            let plot = arena.unit(target).map(SeenPose::of).unwrap_or_default();
            let team = if red { Team::Blue } else { Team::Red };
            // The reds' own sightings of blue 0 stamped in id order, so every
            // share in id order is newer than its mates' rows.
            let stamp = if red { i64::from(id.0 - 10_000) + 1 } else { 0 };
            arena.contacts.insert(
                *id,
                id_of(*id, target),
                ShadowContact::sighted(target, Classification::Classified { team }, stamp, plot),
            );
            if let Some(state) = arena.unit_mut(*id) {
                state.power = Power::Online;
                state.lock = Some(id_of(*id, target));
            }
            for weapon in ["cannon", "machine_gun"] {
                let _ = arena.resolve_arming(*id, &Arming::Arm(slot(weapon, 1_000)), 0);
            }
        }
        let report = |name: &str, mut micros: Vec<u128>| {
            micros.sort_unstable();
            let at = |permille: usize| {
                micros
                    .get(micros.len().saturating_sub(1) * permille / 1_000)
                    .copied()
                    .unwrap_or(0)
            };
            eprintln!(
                "m1b1 evaluator cost [{name}]: {} calls, p50 {} us, p99 {} us",
                micros.len(),
                at(500),
                at(990)
            );
        };
        let mut counters = ReflexCounters::default();
        let mut micros = Vec::with_capacity(ids.len());
        for id in &ids {
            let started = Instant::now();
            let _ = arena.evaluate_conditions(
                *id,
                200,
                ConditionTrigger::WeaponReady,
                Some("cannon"),
                &mut counters,
            );
            micros.push(started.elapsed().as_micros());
        }
        report("WeaponReady, one slot", micros);
        let mut micros = Vec::with_capacity(ids.len());
        for id in &ids {
            let started = Instant::now();
            let _ =
                arena.evaluate_conditions(*id, 200, ConditionTrigger::Pass, None, &mut counters);
            micros.push(started.elapsed().as_micros());
        }
        report("Pass, two slots", micros);
        let mut perception = crate::arena::perception::PerceptionCounters::default();
        let mut journal = Vec::new();
        let mut micros = Vec::new();
        let mut evaluated = 0_usize;
        for id in ids.iter().filter(|id| id.0 < 20_000) {
            let started = Instant::now();
            let refreshed = arena.share_pass(*id, 200, &mut perception, &mut journal);
            let mut receivers: Vec<ObjId> = refreshed
                .iter()
                .filter(|(mate, contact)| {
                    arena
                        .unit(*mate)
                        .is_some_and(|state| state.lock == Some(*contact))
                })
                .map(|(mate, _)| *mate)
                .collect();
            receivers.sort_unstable();
            receivers.dedup();
            evaluated = evaluated.saturating_add(receivers.len());
            for mate in receivers {
                let _ = arena.evaluate_conditions(
                    mate,
                    200,
                    ConditionTrigger::Pass,
                    None,
                    &mut counters,
                );
            }
            micros.push(started.elapsed().as_micros());
        }
        eprintln!(
            "m1b1 evaluator cost [Share]: {evaluated} receiver evaluations over {} shares",
            micros.len()
        );
        report("Share, share_pass and its receivers", micros);
        eprintln!(
            "m1b1 evaluator cost: {} held, {} fired",
            counters.cond_held, counters.cond_fires
        );
    }
}
