//! **Condition slots: the shot decision written down** (17b cut M1b.1; doc 17
//! §5 "The mechanism", "The law"; D-UM-16, D-UM-17 as amended 2026-10-04,
//! D-UM-19).
//!
//! A **condition slot** is the principal's bounded native [`Rule`]: a fire
//! decision, a conditional fire veto, or a Sensors lock selection. Authored
//! rules use ordered first-match channel precedence; stock [`Arming::Arm`]
//! remains independent gun shorthand in the same table. The arena evaluates
//! the rules — never a mind, never a pulse —
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
//! of **any** controller, not in a mind's row (a piloted hull has none —
//! since 17b cut M3.5 a possessed one keeps a DORMANT row, `Arena::suspend_mind`,
//! but its slots are the pilot's, cleared at the possess as below): a declared
//! departure from doc 17 §5's placement, for §24. A hull holds a key only while
//! it has a slot armed, so the table of every match that exists is empty and
//! every trigger's first question — [`ConditionTable::is_empty_for`] — is one
//! map miss that touches nothing else. **Slot numbers are stable**: a slot is
//! its 1-based index in the hull's row. Explicit edits name that number;
//! shorthand updates only its own implicit rule or takes the first free slot.
//! Removal leaves every other slot where it was. The slots
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
//! one hull hold the same row. Fire, hold and lock rules share that capacity;
//! a multi-weapon batch fits in full or refuses [`Refusal::NoSlot`]. A core that loses a class
//! (17b cut M3.4) cuts its row ([`ConditionTable::truncate`]).
//!
//! # The law (D-UM-17 as amended)
//!
//! [`condition_is_lawful`] is the floor's **static** terms, checked at the
//! landing of every arming ([`Arena::resolve_arming`]): the odds term at or
//! above the match's Compact floor ([`Arena::odds_floor_permille`], captured at
//! check-out). An explicit [`SlotTarget::Contact`] is admitted only when held
//! and classified at the arming's landing, and is checked again at each shot.
//! FIRE targets filter the current lock; LOCK rules select on Sensors.
//! Refuse-not-clamp: a condition
//! under the floor is refused [`Refusal::Unlawful`], never raised to it. The
//! lock's classification is **not** a lawfulness term but a warrant term
//! ([`Arena::warrant`]): a slot on an unclassified lock is idle.
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
//! Each event examines at most six rules with four predicates apiece. Ordered
//! fire selection and veto checks revisit that bounded row; Sensors selection
//! examines the hull's earned contact table and its bounded published odds.
//! `WeaponReady` never runs Sensors selection and fires only the ready weapon.
//! Empty and own-only predicates avoid constructing contact projections.
//! The release harness measures the combined work; old one-slot-per-weapon
//! trace estimates do not describe authored selectors. A hull with no slot
//! costs one `BTreeMap` miss per event and never opens an action hash.

use std::collections::BTreeMap;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::intents::{
    Intent, IntentKind, IntentOutcome, IntentSource, Refusal, apply_intent,
};
use crate::arena::motor::MotorCounters;
use crate::arena::perception::{Aspect, FrameNotice, Named, lock_range_step_m};
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
    /// The lock when it is this classified, living contact. Its classification
    /// is checked at arming and again when the cited shot is evaluated.
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

/// Maximum number of AND terms in one authored native rule.
pub const MAX_RULE_PREDICATES: usize = 4;

/// Integer comparison used by a native predicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    /// Strictly less than.
    Less,
    /// Strictly greater than.
    Greater,
    /// At least.
    AtLeast,
    /// Exactly equal.
    Equal,
}

impl Comparison {
    /// Compare two nonnegative quantities in the same units.
    #[must_use]
    pub const fn matches(self, actual: u64, threshold: u64) -> bool {
        match self {
            Self::Less => actual < threshold,
            Self::Greater => actual > threshold,
            Self::AtLeast => actual >= threshold,
            Self::Equal => actual == threshold,
        }
    }
}

/// The three named ranges; a contact beyond reach matches none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleBand {
    /// The close third of effective reach.
    Close,
    /// The middle third of effective reach.
    Effective,
    /// The far third of effective reach.
    Far,
}

/// A fact earned about a contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetState {
    /// Classified by this hull.
    Identified,
    /// Its last plot had nonzero speed.
    Moving,
    /// Its last plot had zero speed.
    Stopped,
    /// An earned condition reading contains damage.
    Hurt,
}

/// One of the public condition-reading colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionState {
    /// Green.
    Green,
    /// Yellow.
    Yellow,
    /// Red.
    Red,
    /// Destroyed.
    Destroyed,
}

/// Deterministic selection among classified hostile plots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockSelector {
    /// Least rounded range, then contact id.
    Nearest,
    /// Highest plotted odds, then range and contact id.
    BestOdds,
    /// Most damage in earned readings, then range and contact id.
    Weakest,
    /// Rear before side before front, then range and contact id.
    Flank,
    /// Freshest published plot, then range and contact id.
    Newest,
}

/// A native predicate. Missing earned facts never satisfy a predicate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "predicate", rename_all = "snake_case", deny_unknown_fields)]
pub enum RulePredicate {
    /// Minimum plotted odds for the named weapon or the rule's default.
    Odds {
        /// Optional weapon override.
        weapon: Option<SmolStr>,
        /// Chance per thousand.
        min_permille: u16,
    },
    /// Allowed faces, using [`ASPECTS_ANY`]'s bits.
    Aspect {
        /// Allowed mask.
        mask: u8,
    },
    /// Rounded public range in metres.
    Range {
        /// Comparison.
        comparison: Comparison,
        /// Metres.
        metres: u32,
    },
    /// Public range band for the rule's weapon.
    Band {
        /// Desired band.
        band: RuleBand,
    },
    /// Public state of the candidate plot.
    Target {
        /// Desired state.
        state: TargetState,
    },
    /// A named section from an earned scan.
    TargetSection {
        /// Section id.
        section: SmolStr,
        /// Reading color.
        state: SectionState,
    },
    /// Rounds remaining, or a percentage of the weapon's full load.
    Rounds {
        /// Optional weapon override.
        weapon: Option<SmolStr>,
        /// Comparison.
        comparison: Comparison,
        /// Rounds or whole percent.
        amount: u32,
        /// True means percent of the authored full load.
        percent: bool,
    },
    /// Own section internals, from the same numbers as STATUS.
    OwnSection {
        /// Section id.
        section: SmolStr,
        /// Strict percentage threshold.
        below_percent: u8,
    },
    /// Own heat as a percentage of the ceiling.
    Heat {
        /// Strict percentage threshold.
        above_percent: u8,
    },
    /// Whether the hull currently holds a classified lock.
    Lock {
        /// True for lock; false for no lock.
        present: bool,
    },
}

/// One native rule in the hull's shared, ordered slot table.
///
/// `implicit` records the unnumbered command's replacement identity. Explicit
/// slots never get overwritten by an unnumbered command. `independent` marks
/// the legacy stock arming shorthand: its separate guns retain their own cycles.
/// Both shorthands and authored rules use this table and the same evaluator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Rule {
    /// Fire through the ordinary warrant and resolver.
    Fire {
        /// Existing legal fire terms.
        condition: FireCondition,
        /// Additional AND terms.
        predicates: Vec<RulePredicate>,
        /// Created through the unnumbered shorthand.
        implicit: bool,
        /// Stock/legacy arming, independently cycled per weapon.
        independent: bool,
    },
    /// Veto automatic fire for this weapon while every predicate matches.
    HoldFire {
        /// Weapon id; ALL is expanded atomically before insertion.
        weapon: SmolStr,
        /// AND terms.
        predicates: Vec<RulePredicate>,
        /// Created through the unnumbered shorthand.
        implicit: bool,
    },
    /// Select a classified hostile through the Sensors channel.
    Lock {
        /// Selection order.
        selector: LockSelector,
        /// Weapon used for unqualified odds and bands, if any.
        weapon: Option<SmolStr>,
        /// AND terms, evaluated separately for each candidate.
        predicates: Vec<RulePredicate>,
        /// Created through the unnumbered shorthand.
        implicit: bool,
    },
}

impl Rule {
    /// Construct an authored fire rule; placement metadata is set by storage.
    #[must_use]
    pub fn fire_rule(condition: FireCondition, predicates: Vec<RulePredicate>) -> Self {
        Self::Fire {
            condition,
            predicates,
            implicit: false,
            independent: false,
        }
    }

    /// Construct an authored veto.
    #[must_use]
    pub fn hold_rule(weapon: SmolStr, predicates: Vec<RulePredicate>) -> Self {
        Self::HoldFire {
            weapon,
            predicates,
            implicit: false,
        }
    }

    /// Construct an authored Sensors rule.
    #[must_use]
    pub fn lock_rule(
        selector: LockSelector,
        weapon: Option<SmolStr>,
        predicates: Vec<RulePredicate>,
    ) -> Self {
        Self::Lock {
            selector,
            weapon,
            predicates,
            implicit: false,
        }
    }

    /// Required station channel.
    #[must_use]
    pub const fn channel(&self) -> Channel {
        match self {
            Self::Lock { .. } => Channel::Sensors,
            _ => Channel::Fire,
        }
    }

    /// Weapon used by this action, if one is named.
    #[must_use]
    pub fn weapon(&self) -> Option<&SmolStr> {
        match self {
            Self::Fire { condition, .. } => Some(&condition.weapon),
            Self::HoldFire { weapon, .. } => Some(weapon),
            Self::Lock { weapon, .. } => weapon.as_ref(),
        }
    }

    /// Fire terms, absent for a veto or a sensor rule.
    #[must_use]
    pub const fn fire(&self) -> Option<&FireCondition> {
        match self {
            Self::Fire { condition, .. } => Some(condition),
            _ => None,
        }
    }

    /// The bounded list of AND predicates.
    #[must_use]
    pub fn predicates(&self) -> &[RulePredicate] {
        match self {
            Self::Fire { predicates, .. }
            | Self::HoldFire { predicates, .. }
            | Self::Lock { predicates, .. } => predicates,
        }
    }

    fn placement(&mut self, implicit: bool, independent: bool) {
        match self {
            Self::Fire {
                implicit: old,
                independent: separate,
                ..
            } => {
                *old = implicit;
                *separate = independent;
            }
            Self::HoldFire { implicit: old, .. } | Self::Lock { implicit: old, .. } => {
                *old = implicit
            }
        }
    }

    fn implicit(&self) -> bool {
        match self {
            Self::Fire { implicit, .. }
            | Self::HoldFire { implicit, .. }
            | Self::Lock { implicit, .. } => *implicit,
        }
    }

    fn independent(&self) -> bool {
        matches!(
            self,
            Self::Fire {
                independent: true,
                ..
            }
        )
    }

    fn same_shorthand(&self, other: &Self) -> bool {
        self.implicit()
            && self.independent() == other.independent()
            && std::mem::discriminant(self) == std::mem::discriminant(other)
            && self.weapon() == other.weapon()
    }
}

/// Validate the bounded wire/snapshot shape before doing any rule work.
///
/// # Errors
/// `Unlawful` for invalid numeric bounds, too many terms or unbounded symbols.
pub fn validate_rule_shape(rule: &Rule) -> Result<(), Refusal> {
    let symbol = |word: &str| {
        !word.is_empty()
            && word.len() <= 64
            && word
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    };
    if rule.predicates().len() > MAX_RULE_PREDICATES
        || rule.weapon().is_some_and(|word| !symbol(word))
    {
        return Err(Refusal::Unlawful);
    }
    if rule.fire().is_some_and(|condition| {
        condition.min_permille > 1_000
            || condition.aspects == 0
            || condition.aspects & !ASPECTS_ANY != 0
            || condition.min_range_m > condition.max_range_m
    }) {
        return Err(Refusal::Unlawful);
    }
    for predicate in rule.predicates() {
        let valid = match predicate {
            RulePredicate::Odds {
                weapon,
                min_permille,
            } => *min_permille <= 1_000 && weapon.as_ref().is_none_or(|word| symbol(word)),
            RulePredicate::Aspect { mask } => *mask != 0 && *mask & !ASPECTS_ANY == 0,
            RulePredicate::TargetSection { section, .. }
            | RulePredicate::OwnSection { section, .. } => symbol(section),
            RulePredicate::Rounds {
                weapon,
                amount,
                percent,
                ..
            } => (!percent || *amount <= 100) && weapon.as_ref().is_none_or(|word| symbol(word)),
            RulePredicate::Heat { above_percent } => *above_percent <= 100,
            _ => true,
        };
        if !valid
            || matches!(predicate, RulePredicate::OwnSection { below_percent, .. } if *below_percent > 100)
        {
            return Err(Refusal::Unlawful);
        }
    }
    Ok(())
}

/// **The STATIC terms of the legal floor** (doc 17 §5, D-UM-17 as amended
/// 2026-10-04): the odds term at or above `floor_permille`, and a target kind
/// the floor admits.
///
/// M5rules admits a classified `Contact` at landing through the owning hull's
/// contact store. This pure check does not read that store: the resolver checks
/// both, and the warrant rechecks classification at each shot.
///
/// # Errors
///
/// [`Refusal::Unlawful`] for an odds term below the captured floor.
pub fn condition_is_lawful(cond: &FireCondition, floor_permille: u32) -> Result<(), Refusal> {
    if u32::from(cond.min_permille) < floor_permille {
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
    rows: BTreeMap<ObjId, Vec<Option<Rule>>>,
}

/// Opaque validated replacement, paired with one prepared profile state.
#[derive(Clone, Debug)]
pub struct PreparedProfileRules {
    unit: ObjId,
    row: Vec<Option<Rule>>,
}

impl ConditionTable {
    /// **`unit`'s row**, in slot order — as wide as the core's slots it was
    /// laid down at (17b cut M3.2), and EMPTY for a hull with no row: every
    /// reader walks the slice's own length, never a fixed width.
    #[must_use]
    pub fn slots(&self, unit: ObjId) -> &[Option<Rule>] {
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
            .position(|slot| {
                slot.as_ref()
                    .and_then(Rule::fire)
                    .is_some_and(|cond| cond.weapon == weapon)
            })
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
        self.arm_avoiding(unit, cond, capacity, 0)
    }

    fn arm_avoiding(
        &mut self,
        unit: ObjId,
        cond: FireCondition,
        capacity: usize,
        protected: u8,
    ) -> Result<u8, Refusal> {
        let mut rule = Rule::fire_rule(cond, Vec::new());
        rule.placement(true, true);
        let mut row = self.rows.get(&unit).cloned().unwrap_or_default();
        let n = Self::put_rule(&mut row, None, rule, capacity, protected)?;
        self.rows.insert(unit, row);
        Ok(n)
    }

    fn put_rule(
        row: &mut Vec<Option<Rule>>,
        slot: Option<u8>,
        rule: Rule,
        capacity: usize,
        protected: u8,
    ) -> Result<u8, Refusal> {
        let holds = capacity.min(crate::arena::core::MAX_CONDITION_SLOTS);
        if holds == 0 {
            return Err(Refusal::NoSlot);
        }
        if row.len() < holds {
            row.resize(holds, None);
        }
        let at = match slot {
            Some(n) => usize::from(n).checked_sub(1).filter(|at| *at < holds),
            None => row
                .iter()
                .take(holds)
                .position(|had| had.as_ref().is_some_and(|had| had.same_shorthand(&rule)))
                .or_else(|| {
                    row.iter()
                        .take(holds)
                        .enumerate()
                        .find_map(|(index, place)| {
                            (place.is_none() && protected & (1 << index) == 0).then_some(index)
                        })
                }),
        }
        .ok_or(Refusal::NoSlot)?;
        if protected & (1 << at) != 0 {
            return Err(Refusal::Channel);
        }
        let place = row.get_mut(at).ok_or(Refusal::NoSlot)?;
        if place
            .as_ref()
            .is_some_and(|had| had.channel() != rule.channel())
        {
            return Err(Refusal::Channel);
        }
        *place = Some(rule);
        u8::try_from(at.saturating_add(1)).map_err(|_| Refusal::NoSlot)
    }

    /// Atomically insert a same-channel authored rule batch. A numbered edit
    /// contains exactly one rule; unnumbered edits own only their shorthand slots.
    ///
    /// # Errors
    /// `Unlawful` for an empty, oversized or malformed batch; `Channel` for
    /// mixed authority or replacing another channel; `NoSlot` for capacity.
    pub fn set_rules(
        &mut self,
        unit: ObjId,
        slot: Option<u8>,
        rules: &[Rule],
        capacity: usize,
    ) -> Result<Vec<u8>, Refusal> {
        self.set_rules_avoiding(unit, slot, rules, capacity, 0)
    }

    fn set_rules_avoiding(
        &mut self,
        unit: ObjId,
        slot: Option<u8>,
        rules: &[Rule],
        capacity: usize,
        protected: u8,
    ) -> Result<Vec<u8>, Refusal> {
        let Some(first) = rules.first() else {
            return Err(Refusal::Unlawful);
        };
        if rules.len() > crate::arena::core::MAX_CONDITION_SLOTS
            || (slot.is_some() && rules.len() != 1)
            || rules
                .iter()
                .any(|rule| rule.predicates().len() > MAX_RULE_PREDICATES)
        {
            return Err(Refusal::Unlawful);
        }
        if rules.iter().any(|rule| rule.channel() != first.channel())
            || (rules.len() > 1 && first.channel() != Channel::Fire)
        {
            return Err(Refusal::Channel);
        }
        let mut row = self.rows.get(&unit).cloned().unwrap_or_default();
        let mut written = Vec::with_capacity(rules.len());
        for rule in rules {
            validate_rule_shape(rule)?;
            let mut rule = rule.clone();
            rule.placement(slot.is_none(), false);
            let n = Self::put_rule(&mut row, slot, rule, capacity, protected)?;
            if written.contains(&n) {
                return Err(Refusal::Unlawful);
            }
            written.push(n);
        }
        self.rows.insert(unit, row);
        Ok(written)
    }

    /// Replace a complete sealed profile's ordered native rows atomically.
    /// Both station channels may be present; this is admission, not a seat edit.
    /// The complete profile program and its sample gate are committed alongside.
    /// # Errors
    /// Invalid capacity or shape; no slot changes before every row is validated.
    pub fn replace_profile_rules(
        &mut self,
        unit: ObjId,
        slots: &[Option<Rule>],
        capacity: usize,
    ) -> Result<(), Refusal> {
        let prepared = Self::prepare_profile_rules(unit, slots, capacity)?;
        self.apply_profile_rules(prepared);
        Ok(())
    }

    /// Validate/allocate one hull's row before a multi-field profile commit.
    /// # Errors
    /// Invalid rule shape or capacity; no table is changed.
    pub fn prepare_profile_rules(
        unit: ObjId,
        slots: &[Option<Rule>],
        capacity: usize,
    ) -> Result<PreparedProfileRules, Refusal> {
        if capacity > crate::arena::core::MAX_CONDITION_SLOTS || slots.len() > capacity {
            return Err(Refusal::NoSlot);
        }
        let mut row = Vec::with_capacity(capacity);
        for rule in slots {
            if let Some(rule) = rule {
                validate_rule_shape(rule)?;
                let mut rule = rule.clone();
                rule.placement(false, false);
                row.push(Some(rule));
            } else {
                row.push(None);
            }
        }
        row.resize(capacity, None);
        Ok(PreparedProfileRules { unit, row })
    }

    /// Infallible synchronous publication after all profile fields are prepared.
    pub fn apply_profile_rules(&mut self, prepared: PreparedProfileRules) {
        if prepared.row.iter().all(Option::is_none) {
            self.rows.remove(&prepared.unit);
        } else {
            self.rows.insert(prepared.unit, prepared.row);
        }
    }

    /// Remove exactly one stable slot, checking its channel at application time.
    ///
    /// # Errors
    /// `NoSlot` for an absent number and `Channel` for a different station's rule.
    pub fn remove_slot(&mut self, unit: ObjId, slot: u8, channel: Channel) -> Result<(), Refusal> {
        let at = usize::from(slot).checked_sub(1).ok_or(Refusal::NoSlot)?;
        let row = self.rows.get_mut(&unit).ok_or(Refusal::NoSlot)?;
        let place = row.get_mut(at).ok_or(Refusal::NoSlot)?;
        let rule = place.as_ref().ok_or(Refusal::NoSlot)?;
        if rule.channel() != channel {
            return Err(Refusal::Channel);
        }
        *place = None;
        if row.iter().all(Option::is_none) {
            self.rows.remove(&unit);
        }
        Ok(())
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
            let going = slot.as_ref().is_some_and(|rule| {
                rule.channel() == Channel::Fire
                    && weapon.is_none_or(|named| rule.weapon().is_some_and(|id| id == named))
            });
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
    pub fn rows(&self) -> impl Iterator<Item = (&ObjId, &[Option<Rule>])> {
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

    fn rule_odds(&self, unit: ObjId, contact: ContactId, weapon: &str) -> Option<u32> {
        // The lock's weapon odds are always on WEAPONSTATUS; other contacts
        // have only the bounded published odds rows, never a secret extra query.
        let own = self.unit(unit)?;
        own.weapon(weapon)?;
        if own.lock == Some(contact) {
            let aim = self.aim_at(unit, contact)?;
            return Some(self.resolve_seen(unit, &aim, weapon).p_permille);
        }
        self.frame_odds(unit)
            .into_iter()
            .find(|row| row.contact == contact && row.weapon == weapon)
            .map(|row| row.p_permille)
    }

    fn rule_predicates_hold(
        &self,
        unit: ObjId,
        rule: &Rule,
        contact: Option<ContactId>,
        at_ms: i64,
    ) -> bool {
        if rule.predicates().is_empty() {
            return true;
        }
        let Some(own) = self.unit(unit) else {
            return false;
        };
        let default_weapon = rule
            .weapon()
            .map(SmolStr::as_str)
            .or_else(|| own.default_weapon().map(|weapon| weapon.id.as_str()));
        let held = contact
            .and_then(|id| self.contacts.resolve(unit, id))
            .filter(|held| {
                matches!(held.class, Classification::Classified { .. })
                    && !self.contact_reads_dead(held)
            });
        let needs_row = rule.predicates().iter().any(|predicate| {
            matches!(
                predicate,
                RulePredicate::Aspect { .. } | RulePredicate::TargetSection { .. }
            )
        });
        let row = if needs_row {
            contact
                .and_then(|id| {
                    self.frame_contacts(unit)
                        .into_iter()
                        .find(|row| row.id == id)
                })
                .filter(|_| held.is_some())
        } else {
            None
        };
        rule.predicates().iter().all(|predicate| match predicate {
            RulePredicate::Odds {
                weapon,
                min_permille,
            } => contact
                .filter(|_| held.is_some())
                .zip(weapon.as_deref().or(default_weapon))
                .and_then(|(id, weapon)| self.rule_odds(unit, id, weapon))
                .is_some_and(|odds| odds >= u32::from(*min_permille)),
            RulePredicate::Aspect { mask } => row
                .as_ref()
                .is_some_and(|row| held.is_some() && aspect_in(*mask, row.aspect)),
            RulePredicate::Range { comparison, metres } => held.is_some_and(|held| {
                let step = if own.lock == contact {
                    lock_range_step_m(at_ms.saturating_sub(held.last_seen_ms))
                } else {
                    crate::arena::perception::CONTACT_RANGE_STEP_M
                };
                comparison.matches(
                    range_m_rounded(held.seen_at.range_from(&own.pose), step),
                    u64::from(*metres),
                )
            }),
            RulePredicate::Band { band } => held
                .zip(default_weapon.and_then(|id| own.weapon(id)))
                .is_some_and(|(held, weapon)| {
                    let word = crate::arena::view::contact_band(
                        held.seen_at.range_from(&own.pose),
                        &weapon.bands_cm(),
                        own.sight_cm(self.definition.terrain.cell_cm()),
                    );
                    word == match band {
                        RuleBand::Close => "close",
                        RuleBand::Effective => "effective",
                        RuleBand::Far => "far",
                    }
                }),
            RulePredicate::Target { state } => held.is_some_and(|held| match state {
                TargetState::Identified => true,
                TargetState::Moving => held.seen_at.speed_cms != 0,
                TargetState::Stopped => held.seen_at.speed_cms == 0,
                TargetState::Hurt => held
                    .seen_condition
                    .is_some_and(|reading| reading.condition.bands != 0),
            }),
            RulePredicate::TargetSection { section, state } => row
                .as_ref()
                .and_then(|row| row.condition)
                .is_some_and(|reading| {
                    let tables = self.frame_scan_tables(unit);
                    tables
                        .get(usize::from(reading.table))
                        .and_then(|names| names.iter().position(|name| name == section))
                        .and_then(|index| reading.bands.band(index))
                        .is_some_and(|band| {
                            band.as_str()
                                == match state {
                                    SectionState::Green => "green",
                                    SectionState::Yellow => "yellow",
                                    SectionState::Red => "red",
                                    SectionState::Destroyed => "destroyed",
                                }
                        })
                }),
            RulePredicate::Rounds {
                weapon,
                comparison,
                amount,
                percent,
            } => weapon
                .as_deref()
                .or(default_weapon)
                .and_then(|id| own.weapon(id))
                .is_some_and(|weapon| {
                    let rounds = u64::from(own.weapon_ammo(&weapon.id));
                    if *percent {
                        weapon.count_per_load > 0
                            && comparison.matches(
                                rounds.saturating_mul(100),
                                u64::from(*amount).saturating_mul(u64::from(weapon.count_per_load)),
                            )
                    } else {
                        comparison.matches(rounds, u64::from(*amount))
                    }
                }),
            RulePredicate::OwnSection {
                section,
                below_percent,
            } => self
                .frame_sections(unit)
                .iter()
                .find(|row| row.id == *section)
                .and_then(|row| row.internals)
                .is_some_and(|(now, max)| {
                    max > 0
                        && i128::from(now).saturating_mul(100)
                            < i128::from(max).saturating_mul(i128::from(*below_percent))
                }),
            RulePredicate::Heat { above_percent } => own
                .heat_now()
                .zip(own.heat_max())
                .is_some_and(|(now, max)| {
                    max > 0
                        && u64::from(now).saturating_mul(100)
                            > u64::from(max).saturating_mul(u64::from(*above_percent))
                }),
            RulePredicate::Lock { present } => {
                own.lock
                    .and_then(|id| self.contacts.resolve(unit, id))
                    .is_some_and(|held| {
                        matches!(held.class, Classification::Classified { .. })
                            && !self.contact_reads_dead(held)
                    })
                    == *present
            }
        })
    }

    /// Whether a conditional HOLD forbids this machine-made shot. Manual pilot
    /// fire retains its existing station-trigger semantics.
    #[must_use]
    pub fn rule_vetoed(&self, unit: ObjId, weapon: &str, at_ms: i64) -> bool {
        let lock = self.unit(unit).and_then(|state| state.lock);
        self.conditions
            .slots(unit)
            .iter()
            .enumerate()
            .any(|(index, rule)| {
                let Some(rule) = rule else {
                    return false;
                };
                let Ok(slot) = u8::try_from(index.saturating_add(1)) else {
                    return false;
                };
                self.runtime_permits(unit, slot, rule, at_ms)
                    && matches!(rule, Rule::HoldFire { weapon: held, .. } if held == weapon)
                    && self.rule_predicates_hold(unit, rule, lock, at_ms)
            })
    }

    fn first_fire_rule(&self, unit: ObjId, at_ms: i64) -> Option<u8> {
        let lock = self.unit(unit).and_then(|state| state.lock);
        self.conditions
            .slots(unit)
            .iter()
            .enumerate()
            .find_map(|(at, rule)| {
                let rule = rule.as_ref()?;
                let slot = u8::try_from(at.saturating_add(1)).ok()?;
                let condition = rule.fire()?;
                if !self.runtime_permits(unit, slot, rule, at_ms)
                    || rule.independent()
                    || self.rule_vetoed(unit, &condition.weapon, at_ms)
                    || !self.rule_predicates_hold(unit, rule, lock, at_ms)
                    || self.warrant(unit, condition, at_ms).is_err()
                {
                    return None;
                }
                u8::try_from(at.saturating_add(1)).ok()
            })
    }

    /// Recheck a cited rule at the shot's millisecond, including conditional
    /// vetoes and ordered authored precedence. Readiness is deliberately absent:
    /// a cycling first match never grants a different authored weapon a turn.
    ///
    /// # Errors
    /// The same held reason as the ordinary warrant, or idle for veto/precedence.
    pub fn rule_warrant(
        &self,
        unit: ObjId,
        slot: u8,
        at_ms: i64,
    ) -> Result<(ContactId, u32, Aspect), Held> {
        let at = usize::from(slot).checked_sub(1).ok_or(Held::Idle)?;
        let rule = self
            .conditions
            .slots(unit)
            .get(at)
            .and_then(Option::as_ref)
            .ok_or(Held::Idle)?;
        let condition = rule.fire().ok_or(Held::Idle)?;
        if !self.runtime_permits(unit, slot, rule, at_ms)
            || self.rule_vetoed(unit, &condition.weapon, at_ms)
        {
            return Err(Held::Idle);
        }
        let selected = self.first_fire_rule(unit, at_ms);
        if (rule.independent() && selected.is_some())
            || (!rule.independent() && selected != Some(slot))
        {
            return Err(Held::Idle);
        }
        let lock = self.unit(unit).and_then(|state| state.lock);
        if !self.rule_predicates_hold(unit, rule, lock, at_ms) {
            return Err(Held::Idle);
        }
        self.warrant(unit, condition, at_ms)
    }

    /// Explain a rejected authored FIRE without granting it firing authority.
    /// Only the existing held vocabulary is emitted; predicates about another
    /// weapon or missing readings remain idle rather than naming a false cause.
    fn rejected_rule_reason(&self, unit: ObjId, rule: &Rule, at_ms: i64) -> Option<HeldWhy> {
        let condition = rule.fire()?;
        if self.holds_fire(unit) || self.rule_vetoed(unit, &condition.weapon, at_ms) {
            return None;
        }
        match self.warrant(unit, condition, at_ms) {
            Err(Held::Why(why)) => return Some(why),
            Err(Held::Idle) => return None,
            Ok(_) => {}
        }
        let own = self.unit(unit)?;
        let lock = own.lock?;
        let held = self.contacts.resolve(unit, lock)?;
        for predicate in rule.predicates() {
            // At most four terms, on the same public facts as selection. This
            // diagnostic walk stops at the first failure and never resolves FIRE.
            let term = Rule::fire_rule(condition.clone(), vec![predicate.clone()]);
            if self.rule_predicates_hold(unit, &term, Some(lock), at_ms) {
                continue;
            }
            return match predicate {
                RulePredicate::Odds {
                    weapon,
                    min_permille,
                } if weapon.as_ref().is_none_or(|id| id == &condition.weapon) => self
                    .rule_odds(unit, lock, &condition.weapon)
                    .map(|permille| HeldWhy::Odds {
                        permille,
                        floor: *min_permille,
                    }),
                RulePredicate::Aspect { .. } => self
                    .frame_contacts(unit)
                    .into_iter()
                    .find(|row| row.id == lock)
                    .map(|row| HeldWhy::Aspect {
                        contact: self.name_of(unit, lock),
                        aspect: row.aspect,
                    }),
                RulePredicate::Range { comparison, metres }
                    if matches!(comparison, Comparison::Less | Comparison::Greater) =>
                {
                    Some(HeldWhy::Range {
                        contact: self.name_of(unit, lock),
                        range_m: range_m_rounded(
                            held.seen_at.range_from(&own.pose),
                            lock_range_step_m(at_ms.saturating_sub(held.last_seen_ms)),
                        ),
                        min_m: if *comparison == Comparison::Greater {
                            metres.saturating_add(1)
                        } else {
                            0
                        },
                        max_m: if *comparison == Comparison::Less {
                            metres.saturating_sub(1)
                        } else {
                            u32::MAX
                        },
                    })
                }
                RulePredicate::Rounds {
                    weapon,
                    comparison: Comparison::Greater,
                    amount,
                    percent,
                } if weapon.as_ref().is_none_or(|id| id == &condition.weapon) => {
                    let weapon = own.weapon(&condition.weapon)?;
                    // rounds > p% is exactly rounds > floor(p * capacity / 100).
                    let keeps = if *percent {
                        u32::try_from(u64::from(*amount) * u64::from(weapon.count_per_load) / 100)
                            .ok()?
                    } else {
                        *amount
                    };
                    Some(HeldWhy::Rounds {
                        rounds: own.weapon_ammo(&condition.weapon),
                        keeps,
                    })
                }
                _ => None,
            };
        }
        None
    }

    fn rule_lock_target(&self, unit: ObjId, rule: &Rule, at_ms: i64) -> Option<ContactId> {
        let Rule::Lock {
            selector, weapon, ..
        } = rule
        else {
            return None;
        };
        let own = self.unit(unit)?;
        let weapon = weapon
            .as_deref()
            .or_else(|| own.default_weapon().map(|weapon| weapon.id.as_str()));
        self.frame_contacts(unit)
            .into_iter()
            .filter(|row| {
                row.class.team().is_some_and(|team| team != own.team)
                    && row.reads_dead != Some(true)
                    && self.rule_predicates_hold(unit, rule, Some(row.id), at_ms)
            })
            .filter_map(|row| {
                let rank = match selector {
                    LockSelector::Nearest => 0_i64,
                    LockSelector::BestOdds => -i64::from(self.rule_odds(unit, row.id, weapon?)?),
                    LockSelector::Weakest => {
                        let reading = row.condition?;
                        let damage: i64 = (0..usize::from(reading.bands.sections))
                            .filter_map(|index| reading.bands.band(index))
                            .map(|band| match band {
                                crate::arena::condition::ConditionBand::Green => 0,
                                crate::arena::condition::ConditionBand::Yellow => 1,
                                crate::arena::condition::ConditionBand::Red => 2,
                                crate::arena::condition::ConditionBand::Destroyed => 3,
                            })
                            .sum();
                        -damage
                    }
                    LockSelector::Flank => match row.aspect {
                        Aspect::Rear => 0,
                        Aspect::Left | Aspect::Right => 1,
                        Aspect::Front => 2,
                    },
                    // Freshest *published plot*, not the private acquisition clock.
                    LockSelector::Newest => row.age_ms,
                };
                Some((
                    (
                        rank,
                        range_m_rounded(
                            row.range_cm,
                            crate::arena::perception::CONTACT_RANGE_STEP_M,
                        ),
                        row.id,
                    ),
                    row.id,
                ))
            })
            .min_by_key(|(key, _)| *key)
            .map(|(_, id)| id)
    }

    /// A sensor pass can trigger a LOCK rule without an existing lock.
    #[must_use]
    pub fn has_sensor_rules(&self, unit: ObjId) -> bool {
        self.conditions
            .slots(unit)
            .iter()
            .flatten()
            .any(|rule| rule.channel() == Channel::Sensors)
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
        counters: &mut MotorCounters,
    ) -> Vec<(u8, Result<u64, Held>)> {
        if self.conditions.is_empty_for(unit) {
            return Vec::new();
        }
        if self.ended.is_some() || self.rule_evaluation_deferred(unit, t_ms) {
            return Vec::new();
        }
        // Sensors rules own their own channel and use the normal admission
        // and lock resolver. One matching rule wins, even if its lock is already
        // held. A ready gun never invents an extra sensor event.
        if trigger != ConditionTrigger::WeaponReady {
            let target =
                self.conditions
                    .slots(unit)
                    .iter()
                    .enumerate()
                    .find_map(|(index, rule)| {
                        let rule = rule.as_ref()?;
                        let slot = u8::try_from(index.saturating_add(1)).ok()?;
                        self.runtime_permits(unit, slot, rule, t_ms)
                            .then(|| self.rule_lock_target(unit, rule, t_ms))
                            .flatten()
                    });
            if let Some(contact) = target
                && self
                    .unit(unit)
                    .is_some_and(|state| state.lock != Some(contact))
            {
                let intent = Intent {
                    unit,
                    channel: Channel::Sensors,
                    epoch: self.epochs.current(unit, Channel::Sensors),
                    kind: IntentKind::Lock { contact },
                    seq: 0,
                };
                if matches!(
                    apply_intent(self, &intent, IntentSource::Cond),
                    IntentOutcome::Queued
                ) {
                    if let Some(queue) = self.pending.get_mut(&unit) {
                        queue.pop_back();
                        if queue.is_empty() {
                            self.pending.remove(&unit);
                        }
                    }
                    if self.action_hash == ActionHash::Outside {
                        self.action_hash = ActionHash::Pending;
                    }
                    let _ = self.resolve_lock(unit, contact, t_ms);
                }
            }
        }
        // Choose once against this event's picture. A shot changing its own
        // ammo cannot make a second authored rule fire during the same event.
        let selected = self.first_fire_rule(unit, t_ms);
        let armed: Vec<(u8, Rule, bool)> = self
            .conditions
            .slots(unit)
            .iter()
            .enumerate()
            .filter_map(|(at, slot)| {
                let rule = slot.as_ref()?;
                let cond = rule.fire()?;
                let n = u8::try_from(at.saturating_add(1)).ok()?;
                if !self.runtime_permits(unit, n, rule, t_ms) {
                    return None;
                }
                if selected.is_some() && selected != Some(n) {
                    return None;
                }
                if trigger == ConditionTrigger::WeaponReady
                    && weapon.is_none_or(|named| cond.weapon != named)
                {
                    return None;
                }
                Some((n, rule.clone(), selected.is_none() && !rule.independent()))
            })
            .collect();
        let mut answers = Vec::with_capacity(armed.len());
        for (n, rule, diagnostic_only) in armed {
            let Some(cond) = rule.fire() else { continue };
            let ready = self.unit(unit).is_some_and(|state| {
                state.weapon_ready(&cond.weapon, t_ms) && state.weapon_ammo(&cond.weapon) > 0
            });
            if !ready {
                answers.push((n, Err(Held::Idle)));
                continue;
            }
            if diagnostic_only {
                let held = match self.rejected_rule_reason(unit, &rule, t_ms) {
                    Some(why) => {
                        counters.cond_held = counters.cond_held.saturating_add(1);
                        self.say_held(unit, n, &cond.weapon, &why, t_ms);
                        Held::Why(why)
                    }
                    None => Held::Idle,
                };
                answers.push((n, Err(held)));
                continue;
            }
            let warranted = match self.rule_warrant(unit, n, t_ms) {
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
        self.resolve_arming_avoiding(unit, arming, at_ms, 0)
    }

    fn resolve_arming_avoiding(
        &mut self,
        unit: ObjId,
        arming: &Arming,
        at_ms: i64,
        protected: u8,
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
                validate_rule_shape(&Rule::fire_rule(cond.clone(), Vec::new()))?;
                self.rule_contact_is_lawful(unit, cond)?;
                // 17b cut M3.1 (D-UM-35): the hull's own capacity, its core's —
                // and, since 17b cut M3.4, Class 0's one on a mind's hull
                // dropped to the ROM (`Arena::arming_capacity`).
                let capacity = self.arming_capacity(unit);
                let n = match self
                    .conditions
                    .arm_avoiding(unit, cond.clone(), capacity, protected)
                {
                    // 17b cut M3.4 (D-UM-32): the ROM owns its one slot — it
                    // takes back a slot the failed mind left on another gun.
                    Err(Refusal::NoSlot) if protected == 0 && self.thinks_on_rom(unit) => {
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
            Arming::Rules { slot, rules } => {
                let Some(first) = rules.first() else {
                    return Err(Refusal::Unlawful);
                };
                if rules.len() > crate::arena::core::MAX_CONDITION_SLOTS
                    || (slot.is_some() && rules.len() != 1)
                {
                    return Err(Refusal::Unlawful);
                }
                if rules.iter().any(|rule| rule.channel() != first.channel())
                    || (rules.len() > 1 && first.channel() != Channel::Fire)
                {
                    return Err(Refusal::Channel);
                }
                let state = self.unit(unit).ok_or(Refusal::Unknown)?;
                if state.limits.wreck {
                    return Err(Refusal::Wreck);
                }
                for rule in rules {
                    validate_rule_shape(rule)?;
                    if rule
                        .weapon()
                        .is_some_and(|weapon| state.weapon(weapon).is_none())
                    {
                        return Err(Refusal::NoWeapon);
                    }
                    for predicate in rule.predicates() {
                        if matches!(predicate, RulePredicate::Odds { min_permille, .. } if u32::from(*min_permille) < self.odds_floor_permille)
                        {
                            return Err(Refusal::Unlawful);
                        }
                        let weapon = match predicate {
                            RulePredicate::Odds { weapon, .. }
                            | RulePredicate::Rounds { weapon, .. } => weapon.as_ref(),
                            _ => None,
                        };
                        if weapon.is_some_and(|weapon| state.weapon(weapon).is_none()) {
                            return Err(Refusal::NoWeapon);
                        }
                    }
                    if let Some(condition) = rule.fire() {
                        condition_is_lawful(condition, self.odds_floor_permille)?;
                        self.rule_contact_is_lawful(unit, condition)?;
                    }
                }
                let capacity = self.arming_capacity(unit);
                let written = self
                    .conditions
                    .set_rules_avoiding(unit, *slot, rules, capacity, protected)?;
                self.forget_held_said(unit, &written);
                Ok(written.first().copied())
            }
            Arming::Remove { slot, channel } => {
                let state = self.unit(unit).ok_or(Refusal::Unknown)?;
                if state.limits.wreck {
                    return Err(Refusal::Wreck);
                }
                self.conditions.remove_slot(unit, *slot, *channel)?;
                self.forget_held_said(unit, &[*slot]);
                Ok(None)
            }
        }
    }

    /// Automatic condition edits cannot rewrite the immutable authored program.
    /// The legacy disarm-all shorthand removes its mutable rows only, so it can
    /// coexist with a later conditional HOLD or an authored LOCK binding.
    pub(crate) fn resolve_machine_arming(
        &mut self,
        unit: ObjId,
        arming: &Arming,
        at_ms: i64,
    ) -> Result<Option<u8>, Refusal> {
        let protected = |slot: u8| {
            self.profiles.get(&unit).is_some_and(|runtime| {
                slot.checked_sub(1).is_some_and(|index| {
                    usize::from(index) < runtime.program.rows.len()
                        && runtime.overridden & (1 << index) == 0
                })
            })
        };
        match arming {
            Arming::Rules {
                slot: Some(slot), ..
            }
            | Arming::Remove { slot, .. }
                if protected(*slot) =>
            {
                return Err(Refusal::Channel);
            }
            Arming::Disarm { weapon } => {
                let mut removable = Vec::new();
                for (index, row) in self.conditions.slots(unit).iter().enumerate() {
                    let Some(rule) = row else {
                        continue;
                    };
                    let Ok(slot) = u8::try_from(index.saturating_add(1)) else {
                        continue;
                    };
                    let matches = rule.channel() == Channel::Fire
                        && weapon.as_ref().is_none_or(|wanted| {
                            rule.weapon().is_some_and(|named| wanted == named)
                        });
                    if !matches {
                        continue;
                    }
                    if protected(slot) {
                        if weapon.is_some() {
                            return Err(Refusal::Channel);
                        }
                    } else {
                        removable.push((slot, rule.channel()));
                    }
                }
                for (slot, channel) in &removable {
                    self.conditions.remove_slot(unit, *slot, *channel)?;
                }
                let freed = removable
                    .into_iter()
                    .map(|(slot, _)| slot)
                    .collect::<Vec<_>>();
                self.forget_held_said(unit, &freed);
                return Ok(None);
            }
            _ => {}
        }
        let protected = self.profiles.get(&unit).map_or(0, |runtime| {
            let used = (1_u8 << runtime.program.rows.len().min(6)).saturating_sub(1);
            used & !runtime.overridden
        });
        self.resolve_arming_avoiding(unit, arming, at_ms, protected)
    }

    /// A successful manual write takes ownership of every touched sealed slot,
    /// even when it replaces a rule with an identical-looking expression.
    pub(crate) fn resolve_manual_arming(
        &mut self,
        unit: ObjId,
        arming: &Arming,
        at_ms: i64,
    ) -> Result<Option<u8>, Refusal> {
        let before = self.conditions.slots(unit).to_vec();
        let result = self.resolve_arming(unit, arming, at_ms)?;
        let after = self.conditions.slots(unit);
        if let Some(runtime) = self.profiles.get_mut(&unit) {
            for index in 0..before.len().max(after.len()) {
                if before.get(index) != after.get(index)
                    && let Ok(slot) = u8::try_from(index.saturating_add(1))
                {
                    runtime.override_slot(slot);
                }
            }
            if let Some(slot) = result {
                runtime.override_slot(slot);
            }
            if let Arming::Remove { slot, .. } = arming {
                runtime.override_slot(*slot);
            }
        }
        Ok(result)
    }

    fn rule_contact_is_lawful(
        &self,
        unit: ObjId,
        condition: &FireCondition,
    ) -> Result<(), Refusal> {
        if let SlotTarget::Contact { id } = condition.target
            && !self.contacts.resolve(unit, id).is_some_and(|held| {
                matches!(held.class, Classification::Classified { .. })
                    && self.unit(held.target).is_some()
            })
        {
            return Err(Refusal::Unlawful);
        }
        Ok(())
    }

    /// **The notice a refused arming is told as** (17b cut M1b.1): the
    /// refused order's notice in `Arena::refusal_notice`'s one shape — word
    /// `fire`, the arming's weapon, the detail read at `at_ms` — with one
    /// refinement: an [`Refusal::Unlawful`] whose odds term met the floor was
    /// refused for its **target kind** (a [`SlotTarget::Contact`]), so it
    /// carries no floor and its sentence is the fallback that names none
    /// (`That rule is not lawful for this unit.`), for a held id and an
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
        let below_floor = match arming {
            Arming::Arm(cond) => u32::from(cond.min_permille) < self.odds_floor_permille,
            Arming::Rules { rules, .. }
                if rules.len() <= crate::arena::core::MAX_CONDITION_SLOTS
                    && rules.iter().all(|rule| rule.predicates().len() <= MAX_RULE_PREDICATES) =>
            {
                rules.iter().any(|rule| {
                    rule.fire().is_some_and(|condition| u32::from(condition.min_permille) < self.odds_floor_permille)
                        || rule.predicates().iter().any(|predicate| matches!(predicate,
                            RulePredicate::Odds { min_permille, .. } if u32::from(*min_permille) < self.odds_floor_permille))
                })
            }
            _ => false,
        };
        match notice {
            crate::arena::perception::FrameNotice::Refused {
                order,
                refusal: Refusal::Unlawful,
                weapon,
                ..
            } if !below_floor => crate::arena::perception::FrameNotice::Refused {
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
        self.disable_profile_slots(unit, 1..=crate::arena::core::MAX_CONDITION_SLOTS as u8);
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
        self.disable_profile_slots(unit, cleared.iter().copied());
        self.forget_held_said(unit, &cleared);
    }

    /// **`unit`'s condition row cut to its core's slots NOW** (17b cut M3.4;
    /// D-UM-34): [`Arena::truncate_slots_to`] at [`Arena::slot_capacity`] —
    /// what `degrade_core` (the lowered class's slots) and `destroy_core`
    /// (Class 0's one) leave the hull.
    pub fn truncate_slots(&mut self, unit: ObjId) {
        self.invalidate_rule_sample(unit);
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
    pub(super) fn arming_capacity(&self, unit: ObjId) -> usize {
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
        let Some((foreign, channel)) = self
            .conditions
            .slots(unit)
            .iter()
            .enumerate()
            .take(holds)
            .filter_map(|(at, had)| Some((at, had.as_ref()?)))
            .find(|(_, had)| {
                !had.independent() || had.fire().is_none_or(|cond| !own.contains(&cond.weapon))
            })
            .and_then(|(at, had)| Some((u8::try_from(at.saturating_add(1)).ok()?, had.channel())))
        else {
            return Err(Refusal::NoSlot);
        };
        self.conditions.remove_slot(unit, foreign, channel)?;
        self.forget_held_said(unit, &[foreign]);
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
    /// An authored edit, or atomic ALL expansion. A numbered edit has one rule.
    Rules {
        /// Explicit stable slot; None selects the implicit shorthand slot.
        slot: Option<u8>,
        /// Same-channel rules, bounded by the core's maximum capacity.
        rules: Vec<Rule>,
    },
    /// Remove a numbered rule under the channel that owns it.
    Remove {
        /// Stable one-based slot.
        slot: u8,
        /// Expected station channel; rechecked against the actual slot.
        channel: Channel,
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
            Arming::Rules { rules, .. } => rules.first().and_then(Rule::weapon),
            Arming::Remove { .. } => None,
        }
    }

    /// Channel of the requested operation. Empty and mixed batches are refused
    /// before applying anything, even if an empty batch reaches Fire admission.
    #[must_use]
    pub const fn channel(&self) -> Channel {
        match self {
            Self::Rules { rules, .. } => match rules.as_slice().first() {
                Some(rule) => rule.channel(),
                None => Channel::Fire,
            },
            Self::Remove { channel, .. } => *channel,
            Self::Arm(_) | Self::Disarm { .. } => Channel::Fire,
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

    #[test]
    fn m5rules_explicit_and_shorthand_identities_stay_distinct() {
        let mut table = ConditionTable::default();
        let authored = |odds| Rule::fire_rule(slot("cannon", odds), Vec::new());
        assert_eq!(
            table.set_rules(RED, Some(1), &[authored(100)], 3),
            Ok(vec![1])
        );
        assert_eq!(table.set_rules(RED, None, &[authored(200)], 3), Ok(vec![2]));
        assert_eq!(table.set_rules(RED, None, &[authored(300)], 3), Ok(vec![2]));
        assert_eq!(table.arm(RED, slot("cannon", 400), 3), Ok(3));
        assert_eq!(table.arm(RED, slot("cannon", 500), 3), Ok(3));
        let odds: Vec<_> = table
            .slots(RED)
            .iter()
            .flatten()
            .filter_map(Rule::fire)
            .map(|condition| condition.min_permille)
            .collect();
        assert_eq!(odds, vec![100, 300, 500]);
        assert_eq!(table.remove_slot(RED, 2, Channel::Fire), Ok(()));
        assert!(table.slots(RED).get(1).is_some_and(Option::is_none));
        assert_eq!(table.slot_of(RED, "cannon"), Some(1));
        assert_eq!(table.arm(RED, slot("cannon", 600), 3), Ok(3));
    }

    #[test]
    fn m5rules_batch_edits_are_atomic_and_channel_bounded() {
        let mut table = ConditionTable::default();
        let lock = Rule::lock_rule(LockSelector::Nearest, None, Vec::new());
        let fire = Rule::fire_rule(slot("cannon", 100), Vec::new());
        assert_eq!(
            table.set_rules(RED, Some(1), std::slice::from_ref(&lock), 2),
            Ok(vec![1])
        );
        let before = table.clone();
        assert_eq!(
            table.set_rules(RED, Some(1), std::slice::from_ref(&fire), 2),
            Err(Refusal::Channel)
        );
        assert_eq!(
            table.remove_slot(RED, 1, Channel::Fire),
            Err(Refusal::Channel)
        );
        assert_eq!(
            table.set_rules(RED, None, &[fire.clone(), lock], 2),
            Err(Refusal::Channel)
        );
        let other = Rule::fire_rule(slot("machine_gun", 100), Vec::new());
        assert_eq!(
            table.set_rules(RED, None, &[fire, other], 2),
            Err(Refusal::NoSlot)
        );
        assert_eq!(
            table, before,
            "failed ALL must not leave its first insertion"
        );
        assert!(
            table.disarm(RED, None).is_empty(),
            "FIRE ALL OFF has no Sensors authority"
        );
        assert_eq!(table.remove_slot(RED, 1, Channel::Sensors), Ok(()));
        assert!(table.is_empty_for(RED));
    }

    #[test]
    fn m5rules_authoritative_admission_bounds_terms_and_floor() {
        let mut arena = locked();
        let install = |rule| Arming::Rules {
            slot: None,
            rules: vec![rule],
        };
        let too_many = Rule::fire_rule(
            slot("cannon", 100),
            vec![RulePredicate::Lock { present: true }; MAX_RULE_PREDICATES + 1],
        );
        assert_eq!(
            arena.resolve_arming(RED, &install(too_many), 0),
            Err(Refusal::Unlawful)
        );
        let below_floor = Rule::fire_rule(
            slot("cannon", 100),
            vec![RulePredicate::Odds {
                weapon: None,
                min_permille: 1,
            }],
        );
        assert_eq!(
            arena.resolve_arming(RED, &install(below_floor), 0),
            Err(Refusal::Unlawful)
        );
        let missing_weapon = Rule::fire_rule(
            slot("cannon", 100),
            vec![RulePredicate::Rounds {
                weapon: Some(SmolStr::new("absent")),
                comparison: Comparison::Greater,
                amount: 0,
                percent: false,
            }],
        );
        assert_eq!(
            arena.resolve_arming(RED, &install(missing_weapon), 0),
            Err(Refusal::NoWeapon)
        );
        let oversized = Arming::Rules {
            slot: None,
            rules: vec![
                Rule::fire_rule(slot("cannon", 100), Vec::new());
                crate::arena::core::MAX_CONDITION_SLOTS + 1
            ],
        };
        assert_eq!(
            arena.resolve_arming(RED, &oversized, 0),
            Err(Refusal::Unlawful)
        );
        assert!(arena.conditions.is_empty_for(RED));
    }

    #[test]
    fn m5rules_contact_admission_and_shots_recheck_classification() {
        let mut arena = locked();
        let contact = id_of(RED, BLUE);
        let mut condition = slot("cannon", 100);
        condition.target = SlotTarget::Contact { id: contact };
        assert_eq!(
            arena.resolve_arming(RED, &Arming::Arm(condition.clone()), 0),
            Ok(Some(1))
        );
        assert!(arena.rule_warrant(RED, 1, 0).is_ok());
        arena
            .contacts
            .get_mut(RED, contact)
            .expect("held plot")
            .class = Classification::Unknown;
        assert_eq!(arena.rule_warrant(RED, 1, 0), Err(Held::Idle));
        assert_eq!(
            arena.resolve_arming(RED, &Arming::Arm(condition), 0),
            Err(Refusal::Unlawful)
        );
        let mut unknown = slot("cannon", 100);
        unknown.target = SlotTarget::Contact { id: ContactId(77) };
        assert_eq!(
            arena.resolve_arming(RED, &Arming::Arm(unknown), 0),
            Err(Refusal::Unlawful)
        );
    }

    #[test]
    fn m5rules_rejected_authored_fire_reports_own_held_reason_without_firing() {
        let mut arena = locked();
        let lock = id_of(RED, BLUE);
        let aspect = arena
            .frame_contacts(RED)
            .into_iter()
            .find(|row| row.id == lock)
            .unwrap()
            .aspect;
        let rejected = Rule::fire_rule(
            slot("cannon", 100),
            vec![RulePredicate::Aspect {
                mask: ASPECTS_ANY & !aspect_mask(&[aspect]),
            }],
        );
        arena
            .conditions
            .set_rules(RED, None, &[rejected], 2)
            .unwrap();
        arena.pending_notices.clear();
        let mut counters = MotorCounters::default();
        for (at_ms, expected_notices) in [(0, 1), (1, 0), (HELD_SAID_MS, 1)] {
            assert_eq!(arena.first_fire_rule(RED, at_ms), None);
            let answers =
                arena.evaluate_conditions(RED, at_ms, ConditionTrigger::Pass, None, &mut counters);
            assert!(matches!(
                answers.as_slice(),
                [(1, Err(Held::Why(HeldWhy::Aspect { .. })))]
            ));
            let notices = arena.pending_notices.remove(&RED).unwrap_or_default();
            assert_eq!(notices.len(), expected_notices);
            assert!(notices.iter().all(|notice| matches!(notice,
                FrameNotice::SlotHeld { slot: 1, weapon, why: HeldWhy::Aspect { aspect: got, .. } }
                if weapon == "cannon" && *got == aspect)));
            assert!(
                arena.pending_notices.is_empty(),
                "other hulls hear no held notice"
            );
            assert!(arena.shots.is_empty());
            assert_eq!(arena.action_hash, ActionHash::Outside);
        }
        assert_eq!(counters.cond_fires, 0);
        // A later matching authored row wins; a rejected earlier row cannot
        // fire or emit a precedence-confusing diagnostic during that event.
        arena
            .conditions
            .set_rules(
                RED,
                Some(2),
                &[Rule::fire_rule(slot("cannon", 100), Vec::new())],
                2,
            )
            .unwrap();
        let answers = arena.evaluate_conditions(
            RED,
            HELD_SAID_MS + 1,
            ConditionTrigger::Pass,
            None,
            &mut counters,
        );
        assert!(matches!(answers.as_slice(), [(2, Ok(_))]));
        assert_eq!(counters.cond_fires, 1);
        assert!(
            !arena
                .pending_notices
                .get(&RED)
                .unwrap()
                .iter()
                .any(|notice| matches!(notice, FrameNotice::SlotHeld { .. }))
        );
    }

    #[test]
    fn m5rules_rejected_rule_diagnostics_obey_readiness_fog_and_hold() {
        #[derive(Clone, Copy, Debug)]
        enum Gate {
            Cycling,
            Empty,
            Unknown,
            Dead,
            NoLock,
            ConditionalHold,
        }
        for gate in [
            Gate::Cycling,
            Gate::Empty,
            Gate::Unknown,
            Gate::Dead,
            Gate::NoLock,
            Gate::ConditionalHold,
        ] {
            let mut arena = locked();
            let rejected = Rule::fire_rule(slot("cannon", 600), Vec::new());
            arena
                .conditions
                .set_rules(RED, None, &[rejected], 2)
                .unwrap();
            let lock = id_of(RED, BLUE);
            match gate {
                Gate::Cycling => {
                    arena
                        .unit_mut(RED)
                        .unwrap()
                        .ready_at_ms
                        .insert("cannon".into(), 1);
                }
                Gate::Empty => {
                    arena.unit_mut(RED).unwrap().ammo.insert("cannon".into(), 0);
                }
                Gate::Unknown => {
                    arena.contacts.get_mut(RED, lock).unwrap().class = Classification::Unknown
                }
                Gate::Dead => arena.contacts.get_mut(RED, lock).unwrap().known_dead = true,
                Gate::NoLock => arena.unit_mut(RED).unwrap().lock = None,
                Gate::ConditionalHold => {
                    arena
                        .conditions
                        .set_rules(
                            RED,
                            Some(2),
                            &[Rule::hold_rule("cannon".into(), Vec::new())],
                            2,
                        )
                        .unwrap();
                }
            }
            arena.pending_notices.clear();
            let mut counters = MotorCounters::default();
            let answers =
                arena.evaluate_conditions(RED, 0, ConditionTrigger::Pass, None, &mut counters);
            assert_eq!(answers, vec![(1, Err(Held::Idle))], "{gate:?}");
            assert!(arena.pending_notices.is_empty(), "{gate:?}");
            assert_eq!(counters.cond_held, 0, "{gate:?}");
            assert!(arena.shots.is_empty(), "{gate:?}");
        }
    }

    #[test]
    fn m5rules_first_authored_fire_keeps_precedence_while_cycling() {
        let mut arena = locked();
        let second = crate::arena::test_support::laser();
        let own = arena.unit_mut(RED).expect("own hull");
        own.ammo.insert(second.id.clone(), second.count_per_load);
        own.weapons.push(second);
        for (number, weapon) in [(1, "cannon"), (2, "laser")] {
            assert_eq!(
                arena.conditions.set_rules(
                    RED,
                    Some(number),
                    &[Rule::fire_rule(slot(weapon, 100), Vec::new())],
                    2
                ),
                Ok(vec![number])
            );
        }
        assert_eq!(arena.first_fire_rule(RED, 0), Some(1));
        assert!(arena.resolve_fire(RED, "cannon", 0).is_ok());
        assert_eq!(
            arena.first_fire_rule(RED, 1),
            Some(1),
            "readiness does not choose another rule"
        );
        assert!(arena.warrant(RED, &slot("laser", 100), 1).is_ok());
        assert_eq!(arena.rule_warrant(RED, 2, 1), Err(Held::Idle));
        let mut counters = MotorCounters::default();
        assert!(
            arena
                .evaluate_conditions(
                    RED,
                    1,
                    ConditionTrigger::WeaponReady,
                    Some("laser"),
                    &mut counters
                )
                .is_empty()
        );
        assert_eq!(counters.cond_fires, 0);
    }

    #[test]
    fn m5rules_conditional_hold_vetoes_native_and_cited_fire_but_not_pilot_trigger() {
        let mut arena = locked();
        assert_eq!(arena.conditions.arm(RED, slot("cannon", 100), 2), Ok(1));
        let hold = Rule::hold_rule(
            SmolStr::new("cannon"),
            vec![RulePredicate::Lock { present: true }],
        );
        assert_eq!(
            arena.conditions.set_rules(RED, Some(2), &[hold], 2),
            Ok(vec![2])
        );
        assert!(arena.rule_vetoed(RED, "cannon", 0));
        assert_eq!(arena.rule_warrant(RED, 1, 0), Err(Held::Idle));
        for by in [
            FiredBy::Cond { slot: 1 },
            FiredBy::Mind { slot: Some(1) },
            FiredBy::Machine,
        ] {
            assert_eq!(
                arena.clone().resolve_fire_by(RED, "cannon", 0, by),
                Err(Refusal::Unwarranted)
            );
        }
        assert!(arena.resolve_fire(RED, "cannon", 0).is_ok());
    }

    #[test]
    fn m5rules_lock_selects_classified_hostiles_and_acquires_without_existing_lock() {
        let mut arena = locked();
        arena.unit_mut(RED).expect("own hull").lock = None;
        let lock = Rule::lock_rule(
            LockSelector::Nearest,
            None,
            vec![RulePredicate::Lock { present: false }],
        );
        assert_eq!(
            arena
                .conditions
                .set_rules(RED, Some(1), std::slice::from_ref(&lock), 2),
            Ok(vec![1])
        );
        assert!(arena.has_sensor_rules(RED));
        assert_eq!(
            arena.rule_lock_target(RED, &lock, 0),
            Some(id_of(RED, BLUE)),
            "closer friendly is excluded"
        );
        let mut counters = MotorCounters::default();
        assert!(
            arena
                .evaluate_conditions(RED, 0, ConditionTrigger::Pass, None, &mut counters)
                .is_empty()
        );
        assert_eq!(
            arena.unit(RED).expect("own hull").lock,
            Some(id_of(RED, BLUE))
        );
        assert_eq!(counters.cond_fires, 0);
    }

    #[test]
    fn m5rules_movement_reads_the_published_plot_and_unknown_facts_are_false() {
        let mut arena = locked();
        arena.unit_mut(BLUE).expect("target").speed_cms = 1_000;
        let contact = Some(id_of(RED, BLUE));
        let rule = |predicate| Rule::fire_rule(slot("cannon", 100), vec![predicate]);
        let stopped = rule(RulePredicate::Target {
            state: TargetState::Stopped,
        });
        let moving = rule(RulePredicate::Target {
            state: TargetState::Moving,
        });
        assert!(arena.rule_predicates_hold(RED, &stopped, contact, 0));
        assert!(!arena.rule_predicates_hold(RED, &moving, contact, 0));
        arena
            .contacts
            .get_mut(RED, id_of(RED, BLUE))
            .expect("plot")
            .seen_at
            .speed_cms = 1_000;
        assert!(!arena.rule_predicates_hold(RED, &stopped, contact, 0));
        assert!(arena.rule_predicates_hold(RED, &moving, contact, 0));
        let unknown_section = rule(RulePredicate::TargetSection {
            section: SmolStr::new("unknown"),
            state: SectionState::Destroyed,
        });
        assert!(!arena.rule_predicates_hold(RED, &unknown_section, contact, 0));
        assert!(!arena.rule_predicates_hold(RED, &moving, None, 0));
    }

    #[test]
    fn m5rules_losing_an_odds_operand_weapon_makes_its_fact_unavailable() {
        let mut arena = locked();
        arena
            .unit_mut(RED)
            .expect("own hull")
            .weapons
            .push(crate::arena::test_support::laser());
        let rule = Rule::fire_rule(
            slot("cannon", 100),
            vec![RulePredicate::Odds {
                weapon: Some(SmolStr::new("laser")),
                min_permille: 100,
            }],
        );
        let target = Some(id_of(RED, BLUE));
        assert!(arena.rule_predicates_hold(RED, &rule, target, 0));
        arena
            .unit_mut(RED)
            .expect("own hull")
            .weapons
            .retain(|weapon| weapon.id != "laser");
        assert!(!arena.rule_predicates_hold(RED, &rule, target, 0));
    }

    /// Two hostile plots on the same bearing, with no current lock. Their
    /// live hull positions intentionally differ from these earned plots.
    fn selector_plots() -> Arena {
        let mut arena = locked();
        arena.unit_mut(RED).expect("own hull").lock = None;
        for (target, x_cm) in [(BLUE, 11_500), (FRIEND, 11_600)] {
            let held = arena
                .contacts
                .get_mut(RED, id_of(RED, target))
                .expect("plot");
            held.class = Classification::Classified { team: Team::Blue };
            held.seen_at.x_cm = x_cm;
            held.seen_at.y_cm = 3_500;
            held.seen_at.heading_mdeg = 0;
        }
        arena
    }

    #[test]
    fn m5rules_nearest_ties_use_rounded_public_range_then_contact_id() {
        let mut arena = selector_plots();
        let lower = id_of(RED, BLUE).min(id_of(RED, FRIEND));
        let higher = id_of(RED, BLUE).max(id_of(RED, FRIEND));
        // Deliberately make the lower id a metre farther away. Both plots
        // still read 100 m, so a hidden centimetre tie-break must not win.
        arena
            .contacts
            .get_mut(RED, lower)
            .expect("plot")
            .seen_at
            .x_cm = 11_600;
        arena
            .contacts
            .get_mut(RED, higher)
            .expect("plot")
            .seen_at
            .x_cm = 11_500;
        let rule = Rule::lock_rule(LockSelector::Nearest, None, Vec::new());
        assert_eq!(arena.rule_lock_target(RED, &rule, 0), Some(lower));
        arena.unit_mut(BLUE).expect("live hull").pose.x_cm = 90_000;
        arena.unit_mut(FRIEND).expect("live hull").pose.x_cm = 1_501;
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 0),
            Some(lower),
            "live movement cannot break the tie"
        );
        arena
            .contacts
            .get_mut(RED, lower)
            .expect("plot")
            .seen_at
            .x_cm = 21_500;
        assert_eq!(arena.rule_lock_target(RED, &rule, 0), Some(higher));
    }

    #[test]
    fn m5rules_best_odds_ranks_published_chance_and_refuses_missing_weapon_facts() {
        let mut arena = selector_plots();
        arena.resolution.p_min = 0;
        arena.resolution.p_max = 1_000;
        arena.resolution.hit_curve = vec![[0, 0], [100_000, 1_000]];
        arena.resolution.error_terms.target_lateral_speed = 1_000_000;
        let blue = id_of(RED, BLUE);
        let other = id_of(RED, FRIEND);
        {
            let held = arena.contacts.get_mut(RED, blue).expect("plot");
            held.seen_at.x_cm = 3_500;
            held.seen_at.speed_cms = 1_000;
        }
        arena
            .contacts
            .get_mut(RED, other)
            .expect("plot")
            .seen_at
            .x_cm = 5_500;
        let rule = Rule::lock_rule(LockSelector::BestOdds, Some("cannon".into()), Vec::new());
        let nearer = arena
            .rule_odds(RED, blue, "cannon")
            .expect("published odds");
        let farther = arena
            .rule_odds(RED, other, "cannon")
            .expect("published odds");
        assert!(
            farther > nearer,
            "fixture needs a better but farther shot: {farther} > {nearer}"
        );
        assert_eq!(arena.rule_lock_target(RED, &rule, 0), Some(other));
        // Equal probabilities use the same rounded-range/id order as nearest.
        arena.resolution.p_min = 500;
        arena.resolution.p_max = 500;
        for id in [blue, other] {
            arena.contacts.get_mut(RED, id).expect("plot").seen_at.x_cm = 11_500;
        }
        assert_eq!(arena.rule_lock_target(RED, &rule, 0), Some(blue.min(other)));
        arena.unit_mut(RED).expect("own hull").weapons.clear();
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 0),
            None,
            "no weapon means no odds, not a zero-valued score"
        );
    }

    #[test]
    fn m5rules_weakest_uses_only_earned_damage_and_has_a_total_tie_order() {
        use crate::arena::condition::{Condition, Reading};
        let mut arena = selector_plots();
        let blue = id_of(RED, BLUE);
        let other = id_of(RED, FRIEND);
        let rule = Rule::lock_rule(LockSelector::Weakest, None, Vec::new());
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 0),
            None,
            "unread hulls have no damage score"
        );
        let reading = |bands| {
            Some(Reading {
                at_ms: 0,
                condition: Condition { bands, sections: 5 },
            })
        };
        arena
            .contacts
            .get_mut(RED, blue)
            .expect("plot")
            .seen_condition = reading(1);
        assert_eq!(arena.rule_lock_target(RED, &rule, 0), Some(blue));
        arena
            .contacts
            .get_mut(RED, other)
            .expect("plot")
            .seen_condition = reading(2);
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 0),
            Some(other),
            "more observed damage wins"
        );
        arena
            .contacts
            .get_mut(RED, blue)
            .expect("plot")
            .seen_condition = reading(2);
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 0),
            Some(blue.min(other)),
            "equal damage and rounded range use id"
        );
        arena.contacts.get_mut(RED, blue).expect("plot").known_dead = true;
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 0),
            Some(other),
            "a read-dead hull is never weakest"
        );
        arena
            .contacts
            .get_mut(RED, other)
            .expect("plot")
            .seen_condition = None;
        assert_eq!(arena.rule_lock_target(RED, &rule, 0), None);
    }

    #[test]
    fn m5rules_flank_prefers_rear_then_side_and_reads_plot_heading_only() {
        let mut arena = selector_plots();
        let blue = id_of(RED, BLUE);
        let other = id_of(RED, FRIEND);
        let rule = Rule::lock_rule(LockSelector::Flank, None, Vec::new());
        arena
            .contacts
            .get_mut(RED, blue)
            .expect("plot")
            .seen_at
            .heading_mdeg = 90_000;
        arena
            .contacts
            .get_mut(RED, other)
            .expect("plot")
            .seen_at
            .heading_mdeg = 270_000;
        let aspects: Vec<_> = arena
            .frame_contacts(RED)
            .into_iter()
            .map(|row| (row.id, row.aspect))
            .collect();
        assert!(
            aspects.contains(&(blue, Aspect::Rear)) && aspects.contains(&(other, Aspect::Front)),
            "fixture aspects: {aspects:?}"
        );
        assert_eq!(arena.rule_lock_target(RED, &rule, 0), Some(blue));
        arena
            .contacts
            .get_mut(RED, blue)
            .expect("plot")
            .seen_at
            .heading_mdeg = 0;
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 0),
            Some(blue),
            "side precedes front"
        );
        arena
            .contacts
            .get_mut(RED, other)
            .expect("plot")
            .seen_at
            .heading_mdeg = 180_000;
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 0),
            Some(blue.min(other)),
            "left and right have equal rank"
        );
        arena.unit_mut(BLUE).expect("live hull").pose.heading_mdeg = 270_000;
        arena.unit_mut(FRIEND).expect("live hull").pose.heading_mdeg = 90_000;
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 0),
            Some(blue.min(other)),
            "unseen turns cannot change rank"
        );
    }

    #[test]
    fn m5rules_newest_ranks_last_published_plot_not_first_acquisition() {
        let mut arena = selector_plots();
        arena.now_ms = 1_000;
        let blue = id_of(RED, BLUE);
        let other = id_of(RED, FRIEND);
        {
            let held = arena.contacts.get_mut(RED, blue).expect("plot");
            held.first_seen_ms = 100;
            held.last_seen_ms = 900;
        }
        {
            let held = arena.contacts.get_mut(RED, other).expect("plot");
            held.first_seen_ms = 700;
            held.last_seen_ms = 800;
        }
        let rule = Rule::lock_rule(LockSelector::Newest, None, Vec::new());
        assert_eq!(arena.rule_lock_target(RED, &rule, 1_000), Some(blue));
        arena
            .contacts
            .get_mut(RED, blue)
            .expect("plot")
            .first_seen_ms = 899;
        arena
            .contacts
            .get_mut(RED, other)
            .expect("plot")
            .first_seen_ms = 0;
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 1_000),
            Some(blue),
            "changing only acquisition times cannot change rank"
        );
        arena
            .contacts
            .get_mut(RED, other)
            .expect("plot")
            .last_seen_ms = 900;
        assert_eq!(
            arena.rule_lock_target(RED, &rule, 1_000),
            Some(blue.min(other)),
            "same published age uses rounded range then id"
        );
    }

    fn set_test_section_cell(
        arena: &mut Arena,
        unit: ObjId,
        section: &str,
        cell: &str,
        value: i64,
    ) {
        use mush_tactics::{CellAddr, CellId, SectionId, UnitMutationOp, UnitMutationPlan};
        let addr = CellAddr::Section {
            section: SectionId::new(section).expect("section id"),
            cell: CellId::new(cell).expect("cell id"),
        };
        let plan = UnitMutationPlan::new(
            vec![UnitMutationOp::SetCell { addr, value }],
            None,
            Some(SmolStr::new("test:rule-boundary")),
        )
        .expect("one cell plan");
        let changed = mush_tactics::apply(&arena.unit(unit).expect("hull").unit, &plan, 0)
            .expect("cell value in bounds")
            .0;
        arena.unit_mut(unit).expect("hull").unit = changed;
    }

    #[test]
    fn m5rules_own_section_threshold_is_strict_and_needs_a_real_own_section() {
        let mut arena = locked();
        let hull_max = arena
            .frame_sections(RED)
            .iter()
            .find(|row| row.id == "hull")
            .and_then(|row| row.internals)
            .expect("own internals")
            .1;
        assert_eq!(hull_max % 2, 0, "fixture has an exact halfway point");
        set_test_section_cell(&mut arena, RED, "hull", "internals", hull_max / 2);
        let rule = |section: &str, below_percent| {
            Rule::hold_rule(
                "cannon".into(),
                vec![RulePredicate::OwnSection {
                    section: section.into(),
                    below_percent,
                }],
            )
        };
        assert!(!arena.rule_predicates_hold(RED, &rule("hull", 50), None, 0));
        assert!(
            arena.rule_predicates_hold(RED, &rule("hull", 51), None, 0),
            "own-only rules do not need a contact"
        );
        assert!(!arena.rule_predicates_hold(RED, &rule("absent", 100), None, 0));
        assert!(!arena.rule_predicates_hold(ObjId(77), &rule("hull", 100), None, 0));
    }

    #[test]
    fn m5rules_heat_threshold_is_strict_and_absent_heat_is_not_zero_heat() {
        let rule = |above_percent| {
            Rule::hold_rule("laser".into(), vec![RulePredicate::Heat { above_percent }])
        };
        assert!(!locked().rule_predicates_hold(RED, &rule(0), None, 0));
        let mut arena = arena_of(vec![crate::arena::test_support::laser_tank(
            RED,
            Team::Red,
            pose(1_500, 3_500),
        )]);
        let max = arena
            .unit(RED)
            .and_then(|own| own.heat_max())
            .expect("heat ceiling");
        assert_eq!(max % 2, 0, "fixture has an exact halfway point");
        set_test_section_cell(
            &mut arena,
            RED,
            "hull",
            crate::definitions::unit::HEAT_CELL,
            i64::from(max / 2),
        );
        assert!(!arena.rule_predicates_hold(RED, &rule(50), None, 0));
        assert!(arena.rule_predicates_hold(RED, &rule(49), None, 0));
        set_test_section_cell(
            &mut arena,
            RED,
            "hull",
            crate::definitions::unit::HEAT_CELL,
            i64::from(max / 2 + 1),
        );
        assert!(arena.rule_predicates_hold(RED, &rule(50), None, 0));
        arena.unit_mut(RED).expect("own hull").heat = None;
        assert!(!arena.rule_predicates_hold(RED, &rule(0), None, 0));
    }

    #[test]
    fn m5rules_rounds_percent_uses_exact_fraction_and_missing_weapon_is_false() {
        let mut arena = locked();
        arena
            .unit_mut(RED)
            .expect("own hull")
            .weapons
            .first_mut()
            .expect("cannon")
            .count_per_load = 41;
        arena
            .unit_mut(RED)
            .expect("own hull")
            .ammo
            .insert("cannon".into(), 20);
        let rule = |comparison, amount, percent, weapon| {
            Rule::hold_rule(
                "cannon".into(),
                vec![RulePredicate::Rounds {
                    weapon,
                    comparison,
                    amount,
                    percent,
                }],
            )
        };
        assert!(arena.rule_predicates_hold(RED, &rule(Comparison::Less, 50, true, None), None, 0));
        assert!(!arena.rule_predicates_hold(
            RED,
            &rule(Comparison::Greater, 50, true, None),
            None,
            0
        ));
        assert!(!arena.rule_predicates_hold(
            RED,
            &rule(Comparison::Greater, 20, false, None),
            None,
            0
        ));
        arena
            .unit_mut(RED)
            .expect("own hull")
            .ammo
            .insert("cannon".into(), 21);
        assert!(arena.rule_predicates_hold(
            RED,
            &rule(Comparison::Greater, 50, true, None),
            None,
            0
        ));
        assert!(!arena.rule_predicates_hold(
            RED,
            &rule(Comparison::Greater, 0, false, Some("absent".into())),
            None,
            0
        ));
        arena
            .unit_mut(RED)
            .expect("own hull")
            .ammo
            .insert("cannon".into(), 0);
        assert!(
            !arena.rule_predicates_hold(RED, &rule(Comparison::Greater, 0, false, None), None, 0),
            "reserve requires one real round"
        );
    }

    #[test]
    fn m5rules_band_boundaries_match_the_public_band_and_need_a_classified_weapon_plot() {
        let mut arena = locked();
        let target = id_of(RED, BLUE);
        let rule = |band| Rule::fire_rule(slot("cannon", 100), vec![RulePredicate::Band { band }]);
        let own = arena.unit(RED).expect("own hull");
        assert_eq!(own.sight_cm(arena.definition.terrain.cell_cm()), 60_000);
        assert_eq!(
            own.weapon("cannon").expect("cannon").bands_cm().last(),
            Some(&250_000)
        );
        for (range_cm, expected) in [
            (20_000, RuleBand::Close),
            (20_001, RuleBand::Effective),
            (40_000, RuleBand::Effective),
            (40_001, RuleBand::Far),
            (250_000, RuleBand::Far),
        ] {
            arena
                .contacts
                .get_mut(RED, target)
                .expect("plot")
                .seen_at
                .x_cm = 1_500 + range_cm;
            for band in [RuleBand::Close, RuleBand::Effective, RuleBand::Far] {
                assert_eq!(
                    arena.rule_predicates_hold(RED, &rule(band), Some(target), 0),
                    band == expected,
                    "{range_cm} cm: {band:?}"
                );
            }
        }
        arena
            .contacts
            .get_mut(RED, target)
            .expect("plot")
            .seen_at
            .x_cm = 251_501;
        assert!(!arena.rule_predicates_hold(RED, &rule(RuleBand::Far), Some(target), 0));
        arena
            .contacts
            .get_mut(RED, target)
            .expect("plot")
            .seen_at
            .x_cm = 5_500;
        arena.contacts.get_mut(RED, target).expect("plot").class = Classification::Unknown;
        assert!(!arena.rule_predicates_hold(RED, &rule(RuleBand::Close), Some(target), 0));
        arena.contacts.get_mut(RED, target).expect("plot").class =
            Classification::Classified { team: Team::Blue };
        arena.unit_mut(RED).expect("own hull").weapons.clear();
        assert!(!arena.rule_predicates_hold(RED, &rule(RuleBand::Close), Some(target), 0));
    }

    #[test]
    fn m5rules_target_section_requires_an_earned_named_reading_not_live_damage() {
        use crate::arena::condition::{Condition, ConditionBand, Reading};
        let mut arena = locked();
        let target = id_of(RED, BLUE);
        let rule = |section: &str, state| {
            Rule::fire_rule(
                slot("cannon", 100),
                vec![RulePredicate::TargetSection {
                    section: section.into(),
                    state,
                }],
            )
        };
        let green = rule("hull", SectionState::Green);
        assert!(
            !arena.rule_predicates_hold(RED, &green, Some(target), 0),
            "no scan is not a green scan"
        );
        arena
            .contacts
            .get_mut(RED, target)
            .expect("plot")
            .seen_condition = Some(Reading {
            at_ms: 0,
            condition: Condition {
                bands: 0,
                sections: 5,
            },
        });
        assert!(arena.rule_predicates_hold(RED, &green, Some(target), 0));
        set_test_section_cell(&mut arena, BLUE, "hull", "internals", 0);
        assert!(
            arena.rule_predicates_hold(RED, &green, Some(target), 0),
            "unseen damage does not refresh a scan"
        );
        assert!(!arena.rule_predicates_hold(
            RED,
            &rule("absent", SectionState::Green),
            Some(target),
            0
        ));
        let index = arena
            .frame_scan_tables(RED)
            .first()
            .expect("scan vocabulary")
            .iter()
            .position(|section| section == "hull")
            .expect("hull name");
        let reading = arena
            .contacts
            .get_mut(RED, target)
            .expect("plot")
            .seen_condition
            .as_mut()
            .expect("scan");
        reading.condition = reading.condition.with_band(index, ConditionBand::Yellow);
        assert!(!arena.rule_predicates_hold(RED, &green, Some(target), 0));
        assert!(arena.rule_predicates_hold(
            RED,
            &rule("hull", SectionState::Yellow),
            Some(target),
            0
        ));
        arena.contacts.get_mut(RED, target).expect("plot").class = Classification::Unknown;
        assert!(!arena.rule_predicates_hold(
            RED,
            &rule("hull", SectionState::Yellow),
            Some(target),
            0
        ));
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
            table
                .slots(RED)
                .first()
                .and_then(Option::as_ref)
                .and_then(Rule::fire)
                .cloned(),
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

    /// The floor's static terms; classified Contact admission is checked on
    /// the owning hull at landing, rather than rejected by this pure predicate.
    #[test]
    fn m5rules_the_floor_and_contact_admission_are_distinct_checks() {
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
        assert_eq!(condition_is_lawful(&contact, 10), Ok(()));
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
        let mut counters = MotorCounters::default();
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
        assert_eq!(counters, MotorCounters::default());
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
        let mut counters = MotorCounters::default();
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
    /// `Empty` answer before the hold-fire term, which answers `Unwarranted`
    /// before power. M5rules applies the veto to every machine-made shot; a
    /// seat's immediate trigger keeps its human semantics.
    ///
    /// 17b cut M4.5a: the hull under the hold-fire order is a mind holding it
    /// on its row (re-keyed as the order's Mind arm writes it), which
    /// `holds_fire` reads.
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
            Controller::Mind,
            crate::arena::doctrine::MachineState::Patrol,
        );
        let _ = arena.resolve_lock(RED, id_of(RED, BLUE), 0);
        let standing = StandingOrder::accepted(
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
        );
        let held = arena.rekey_order(RED, &standing.order, standing.expires_ms);
        arena.minds.get_mut(&RED).expect("the mind's row").order = Some(held);
        assert!(arena.holds_fire(RED));
        let mut diagnostics = arena.clone();
        diagnostics
            .conditions
            .set_rules(
                RED,
                None,
                &[Rule::fire_rule(slot("cannon", 600), Vec::new())],
                2,
            )
            .unwrap();
        diagnostics.pending_notices.clear();
        let answers = diagnostics.evaluate_conditions(
            RED,
            0,
            ConditionTrigger::Pass,
            None,
            &mut MotorCounters::default(),
        );
        assert_eq!(answers, vec![(1, Err(Held::Idle))]);
        assert!(
            diagnostics.pending_notices.is_empty(),
            "ORDER HOLD outranks a rejected-rule reason"
        );
        assert_eq!(
            arena.clone().resolve_fire_by(RED, "cannon", 0, cond),
            Err(Refusal::Unwarranted),
            "hold-fire stops a slot"
        );
        assert!(
            arena
                .clone()
                .resolve_fire_by(RED, "cannon", 0, FiredBy::Seat)
                .is_ok()
        );
        assert_eq!(
            arena
                .clone()
                .resolve_fire_by(RED, "cannon", 0, FiredBy::Machine),
            Err(Refusal::Unwarranted)
        );
        // 17b cut M1b.2 put the law on a mind's shot (`weapons.rs`' in-source
        // tests pin its order): uncited or cited, an ORDER veto still binds.
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
        // No order or veto: native evaluator dispatch retains its trusted citation.
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
        let mut counters = MotorCounters::default();
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

    fn m6_install_program(arena: &mut Arena, rules: Vec<crate::arena::schema::AuthoredRule>) {
        use crate::arena::runtime_card::{RuntimeProfiles, stock_definition};
        let own = arena.unit(RED).expect("own hull");
        let class = own.seal.class;
        let weapons = own
            .seal
            .weapons
            .iter()
            .map(|weapon| weapon.id.clone())
            .collect::<Vec<_>>();
        let channels = own.seal.channels.clone();
        let old = arena.profiles.get(&RED).expect("runtime");
        let rom = old.rom.clone();
        let mut definition = stock_definition(
            &[(
                "authored".into(),
                old.card.stock().expect("stock fixture").clone(),
            )],
            class,
        )
        .expect("definition");
        definition.profiles[0].rules = rules;
        let digest = arena.cards.admit(definition).expect("catalog admission");
        let profiles = RuntimeProfiles::bind(&arena.cards, digest, &[]).expect("profiles");
        let runtime = crate::arena::profile_runtime::ProfileRuntime::new(
            &arena.cards,
            RED,
            profiles.clone(),
            rom,
            &arena.classes.row(class),
            &weapons,
            &channels,
            arena.odds_floor_permille,
        )
        .expect("runtime admission");
        runtime
            .program
            .install(&mut arena.conditions, RED, arena.classes.row(class).slots)
            .expect("complete native table");
        arena.profiles.insert(RED, runtime);
        let own = arena.unit_mut(RED).expect("own");
        own.seal.profiles = profiles;
        own.seal.doctrine = false;
    }

    fn m6_authored(rule: Rule, sampled: bool) -> crate::arena::schema::AuthoredRule {
        crate::arena::schema::AuthoredRule {
            rule,
            think: if sampled {
                vec![crate::arena::schema::ThinkPredicate::Hostiles { at_least: 1 }]
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn m6_machine_disarm_preserves_program_and_manual_edit_overrides_it() {
        let mut arena = locked();
        let hold = Rule::hold_rule("cannon".into(), Vec::new());
        m6_install_program(&mut arena, vec![m6_authored(hold.clone(), true)]);
        assert_eq!(
            arena.resolve_machine_arming(RED, &Arming::Arm(slot("cannon", 100)), 0),
            Ok(Some(2))
        );
        assert_eq!(
            arena.resolve_machine_arming(RED, &Arming::Disarm { weapon: None }, 0),
            Ok(None)
        );
        assert_eq!(
            arena.conditions.slots(RED).first().and_then(Option::as_ref),
            Some(&hold)
        );
        assert!(
            arena
                .conditions
                .slots(RED)
                .get(1)
                .is_none_or(Option::is_none)
        );
        let before = arena.conditions.clone();
        assert_eq!(
            arena.resolve_machine_arming(
                RED,
                &Arming::Remove {
                    slot: 1,
                    channel: Channel::Fire
                },
                0
            ),
            Err(Refusal::Channel)
        );
        assert_eq!(
            arena.resolve_machine_arming(
                RED,
                &Arming::Disarm {
                    weapon: Some("cannon".into())
                },
                0
            ),
            Err(Refusal::Channel)
        );
        assert_eq!(arena.conditions, before, "refusal is atomic");
        assert_eq!(
            arena.resolve_manual_arming(
                RED,
                &Arming::Rules {
                    slot: Some(1),
                    rules: vec![hold.clone()]
                },
                0
            ),
            Ok(Some(1))
        );
        assert!(
            arena.runtime_permits(RED, 1, &hold, 0),
            "identical manual replacement owns its law without sampled truth"
        );
        assert_eq!(
            arena.resolve_machine_arming(
                RED,
                &Arming::Remove {
                    slot: 1,
                    channel: Channel::Fire
                },
                0
            ),
            Ok(None)
        );
    }

    #[test]
    fn m6_automatic_allocation_skips_reserved_program_holes() {
        let mut arena = locked();
        m6_install_program(
            &mut arena,
            vec![m6_authored(
                Rule::hold_rule("cannon".into(), Vec::new()),
                true,
            )],
        );
        arena
            .conditions
            .remove_slot(RED, 1, Channel::Fire)
            .expect("simulate absent native row");
        assert_eq!(
            arena.resolve_machine_arming(RED, &Arming::Arm(slot("cannon", 100)), 0),
            Ok(Some(2))
        );
        assert!(
            arena
                .conditions
                .slots(RED)
                .first()
                .is_some_and(Option::is_none)
        );
    }

    fn m6_pending(arena: &mut Arena, boundary: i64, truth: Option<Vec<bool>>) {
        use crate::arena::mind::{MindState, PendingThink};
        use crate::arena::profile_rules::RuleSample;
        let runtime = arena.profiles.get(&RED).expect("runtime");
        let class = arena.unit(RED).expect("own").seal.class;
        let period = arena.classes.row(class).think_ms;
        let sampled_ms = boundary - arena.substep_len_ms();
        let sample = truth.map(|truth| RuleSample {
            program: runtime.program.digest,
            generation: runtime.generation,
            sampled_ms,
            lands_ms: boundary,
            expires_ms: boundary + period,
            truth,
        });
        let pending = PendingThink {
            think_seq: 9,
            t_ms: sampled_ms,
            epochs: Channel::ALL.map(|channel| arena.epochs.current(RED, channel)),
            profile_token: runtime.token(),
            lands_ms: boundary,
            period_ms: period,
            profile_class: class,
            rule_sample: sample,
            intents: Vec::new(),
            memory: b"fresh".to_vec(),
        };
        let mut state = MindState::fresh(&arena.arena_cfg.reflex);
        state.seal_digest = arena.unit(RED).expect("own").seal.digest;
        state.memory = b"prior".to_vec();
        state.pending = Some(pending);
        arena.minds.insert(RED, state);
        arena.unit_mut(RED).expect("own").controller = Controller::Mind;
        arena.due.schedule(
            boundary,
            crate::arena::state::DueKind::MindLand {
                unit: RED,
                think_seq: 9,
            },
        );
    }

    fn m6_old_sample(arena: &mut Arena, boundary: i64, truth: Vec<bool>) -> (u64, i64) {
        let runtime = arena.profiles.get(&RED).expect("runtime");
        let generation = runtime.generation;
        let period = arena.mind_profile(RED).think_ms;
        let lands_ms = boundary - period;
        let sample = crate::arena::profile_rules::RuleSample {
            program: runtime.program.digest,
            generation,
            sampled_ms: lands_ms - arena.substep_len_ms(),
            lands_ms,
            expires_ms: boundary,
            truth,
        };
        arena
            .land_rule_sample(RED, Some(sample), lands_ms, period)
            .expect("old sample");
        (generation, lands_ms)
    }

    #[test]
    fn m6_stale_station_epoch_discards_sample_but_keeps_current_channel_intents() {
        let mut arena = locked();
        m6_install_program(
            &mut arena,
            vec![m6_authored(
                Rule::hold_rule("cannon".into(), Vec::new()),
                true,
            )],
        );
        let boundary = 2_000;
        m6_old_sample(&mut arena, boundary, vec![true]);
        let contact = arena.unit(RED).expect("own").lock.expect("held contact");
        arena.unit_mut(RED).expect("own").lock = None;
        m6_pending(&mut arena, boundary, Some(vec![true]));
        arena
            .minds
            .get_mut(&RED)
            .expect("mind")
            .pending
            .as_mut()
            .expect("pending")
            .intents = vec![
            IntentKind::Heading { mdeg: 90_000 },
            IntentKind::Lock { contact },
        ];
        let epoch = arena.epochs.current(RED, Channel::Helm) + 1;
        arena.epochs.raise(RED, Channel::Helm, epoch);
        arena.now_ms = boundary;
        arena.mind_land(RED, 9, boundary, &mut MotorCounters::default());
        assert_eq!(arena.unit(RED).expect("own").lock, Some(contact));
        let mind = arena.minds.get(&RED).expect("mind");
        assert_eq!(mind.memory, b"fresh");
        assert!(mind.pending.is_none());
        assert!(mind.notices.iter().any(|notice| matches!(
            notice,
            FrameNotice::Refused { order, refusal: Refusal::Epoch, .. }
                if order == "heading"
        )));
        assert!(
            arena
                .profiles
                .get(&RED)
                .expect("runtime")
                .lease
                .sample()
                .is_none()
        );
        assert!(!arena.rule_vetoed(RED, "cannon", boundary));
    }

    #[test]
    fn m6_sampled_hold_renewal_has_no_shot_gap_and_false_releases_at_boundary() {
        for renewed in [true, false] {
            let mut arena = locked();
            m6_install_program(
                &mut arena,
                vec![
                    m6_authored(Rule::hold_rule("cannon".into(), Vec::new()), true),
                    m6_authored(Rule::fire_rule(slot("cannon", 100), Vec::new()), false),
                ],
            );
            let boundary = 2 * arena.mind_profile(RED).think_ms;
            let (generation, lands) = m6_old_sample(&mut arena, boundary, vec![true, true]);
            m6_pending(&mut arena, boundary, Some(vec![renewed, true]));
            arena.now_ms = boundary;
            let ammo = arena.unit(RED).expect("own").weapon_ammo("cannon");
            let mut counters = MotorCounters::default();
            arena.expire_rule_sample(RED, generation, lands, boundary, &mut counters);
            assert!(
                arena
                    .evaluate_conditions(
                        RED,
                        boundary,
                        ConditionTrigger::WeaponReady,
                        Some("cannon"),
                        &mut counters
                    )
                    .is_empty()
            );
            assert_eq!(
                arena.unit(RED).expect("own").weapon_ammo("cannon"),
                ammo,
                "ordinary due events wait for the pending complete answer"
            );
            arena.mind_land(RED, 9, boundary, &mut counters);
            assert_eq!(
                arena.unit(RED).expect("own").weapon_ammo("cannon"),
                ammo - u32::from(!renewed)
            );
            assert_eq!(arena.minds.get(&RED).expect("mind").memory, b"fresh");
            assert!(!arena.rule_evaluation_deferred(RED, boundary));
        }
    }

    #[test]
    fn m6_sampled_fire_false_or_missing_never_extends_the_old_truth() {
        for truth in [Some(vec![false]), None] {
            let mut arena = locked();
            m6_install_program(
                &mut arena,
                vec![m6_authored(
                    Rule::fire_rule(slot("cannon", 100), Vec::new()),
                    true,
                )],
            );
            let boundary = 2 * arena.mind_profile(RED).think_ms;
            let (generation, lands) = m6_old_sample(&mut arena, boundary, vec![true]);
            m6_pending(&mut arena, boundary, truth);
            arena.now_ms = boundary;
            let ammo = arena.unit(RED).expect("own").weapon_ammo("cannon");
            let mut counters = MotorCounters::default();
            arena.expire_rule_sample(RED, generation, lands, boundary, &mut counters);
            arena.mind_land(RED, 9, boundary, &mut counters);
            let _ = arena.evaluate_conditions(
                RED,
                boundary,
                ConditionTrigger::WeaponReady,
                Some("cannon"),
                &mut counters,
            );
            assert_eq!(arena.unit(RED).expect("own").weapon_ammo("cannon"), ammo);
        }
    }

    #[test]
    fn m6_stale_or_missing_completion_clears_hold_and_cannot_defer_forever() {
        for missing in [false, true] {
            let mut arena = locked();
            m6_install_program(
                &mut arena,
                vec![
                    m6_authored(Rule::hold_rule("cannon".into(), Vec::new()), true),
                    m6_authored(Rule::fire_rule(slot("cannon", 100), Vec::new()), false),
                ],
            );
            let boundary = 2 * arena.mind_profile(RED).think_ms;
            let (generation, lands) = m6_old_sample(&mut arena, boundary, vec![true, true]);
            m6_pending(&mut arena, boundary, Some(vec![true, true]));
            if missing {
                arena.awaiting.insert(RED);
            } else {
                arena
                    .minds
                    .get_mut(&RED)
                    .expect("mind")
                    .pending
                    .as_mut()
                    .expect("pending")
                    .epochs[0] += 1;
            }
            arena.now_ms = boundary;
            let ammo = arena.unit(RED).expect("own").weapon_ammo("cannon");
            let mut counters = MotorCounters::default();
            arena.expire_rule_sample(RED, generation, lands, boundary, &mut counters);
            arena.mind_land(RED, 9, boundary, &mut counters);
            assert_eq!(
                arena.unit(RED).expect("own").weapon_ammo("cannon"),
                ammo - 1
            );
            assert_eq!(arena.minds.get(&RED).expect("mind").memory, b"prior");
            assert!(!arena.rule_evaluation_deferred(RED, boundary));
            assert!(
                arena
                    .profiles
                    .get(&RED)
                    .expect("runtime")
                    .lease
                    .sample()
                    .is_none()
            );
        }
    }

    #[test]
    fn m6_possession_disables_and_release_restores_the_selected_program() {
        use crate::arena::doctrine::MachineState;
        let mut arena = locked();
        let fire = Rule::fire_rule(slot("cannon", 100), Vec::new());
        m6_install_program(&mut arena, vec![m6_authored(fire.clone(), true)]);
        arena.set_controller(RED, Controller::Mind, MachineState::Startup);
        arena.minds.get_mut(&RED).expect("mind").memory = b"owned memory".to_vec();
        let before = arena.profiles.get(&RED).expect("runtime").token();
        arena.set_controller(RED, Controller::Pilot, MachineState::Startup);
        assert!(arena.conditions.is_empty_for(RED));
        assert!(
            arena.hydrate_profiles().is_ok(),
            "dormant cleared rows have explicit disable bits"
        );
        assert_eq!(
            arena.resolve_manual_arming(RED, &Arming::Arm(slot("cannon", 400)), 0),
            Ok(Some(1))
        );
        arena.set_controller(RED, Controller::Mind, MachineState::Startup);
        let runtime = arena.profiles.get(&RED).expect("runtime");
        assert!(runtime.generation > before.generation);
        assert_eq!(runtime.overridden, 0);
        assert_eq!(
            arena.conditions.slots(RED).first().and_then(Option::as_ref),
            Some(&fire)
        );
        assert_eq!(arena.minds.get(&RED).expect("mind").memory, b"owned memory");
        assert!(
            !arena.runtime_permits(RED, 1, &fire, 0),
            "handback never restores sampled truth"
        );
    }

    #[test]
    fn m6_restore_requires_exact_program_bindings_and_expiry_schedule() {
        let mut arena = locked();
        arena
            .unit_mut(BLUE)
            .expect("fastest hull")
            .mobility
            .max_speed_cms = 19_000;
        let fire = Rule::fire_rule(slot("cannon", 100), Vec::new());
        m6_install_program(&mut arena, vec![m6_authored(fire, true)]);
        arena.set_controller(
            RED,
            Controller::Mind,
            crate::arena::doctrine::MachineState::Startup,
        );
        let boundary = 2 * arena.mind_profile(RED).think_ms;
        let (_, lands) = m6_old_sample(&mut arena, boundary, vec![true]);
        arena.now_ms = lands;
        arena.hydrate_profiles().expect("valid active sample");
        let snapshot = crate::arena::snapshot::Snapshot::encode(&arena).expect("snapshot");
        let restored = crate::arena::snapshot::Snapshot::decode(
            &snapshot.document,
            (*arena.definition).clone(),
        )
        .expect("restore");
        assert_eq!(
            crate::arena::hash::state_hash(&restored),
            crate::arena::hash::state_hash(&arena)
        );
        let captured_delay = restored.substep_len_ms();
        let mut slower = restored.clone();
        slower
            .unit_mut(BLUE)
            .expect("fastest hull")
            .limits
            .motion_disabled = true;
        assert_ne!(slower.substep_len_ms(), captured_delay);
        slower
            .hydrate_profiles()
            .expect("hardware loss does not rewrite a captured landing");
        let mut missing_expiry = restored.clone();
        missing_expiry.due.retain(|kind| !matches!(kind, crate::arena::state::DueKind::RuleLeaseExpired { unit, .. } if *unit == RED));
        assert!(missing_expiry.hydrate_profiles().is_err());
        let mut missing_rule = restored.clone();
        missing_rule
            .conditions
            .remove_slot(RED, 1, Channel::Fire)
            .expect("tamper one row");
        assert!(missing_rule.hydrate_profiles().is_err());
        let mut future_hit = restored;
        future_hit.unit_mut(RED).expect("own").own_last_hit_ms = Some(lands + 1);
        assert!(future_hit.hydrate_profiles().is_err());
    }

    #[test]
    fn m6_actual_fault_settlement_clears_sample_and_preserves_prior_memory() {
        use crate::arena::mind::{ThinkFault, ThinkResult};
        let mut arena = locked();
        m6_install_program(
            &mut arena,
            vec![
                m6_authored(Rule::hold_rule("cannon".into(), Vec::new()), true),
                m6_authored(Rule::fire_rule(slot("cannon", 100), Vec::new()), false),
            ],
        );
        arena.set_controller(
            RED,
            Controller::Mind,
            crate::arena::doctrine::MachineState::Startup,
        );
        arena.minds.get_mut(&RED).expect("mind").memory = b"prior".to_vec();
        let at = arena.mind_profile(RED).think_ms;
        let boundary = at + arena.substep_len_ms();
        m6_old_sample(&mut arena, boundary, vec![true, true]);
        arena.now_ms = at;
        let mut counters = MotorCounters::default();
        arena.collect_think(RED, 0, at, &mut counters);
        let think = arena.collected.pop().expect("actual collected think");
        let result = ThinkResult {
            fault: Some(ThinkFault {
                cause: "ops".into(),
            }),
            memory: b"must not land".to_vec(),
            ..ThinkResult::default()
        };
        let settled = arena.settle_collected(&think, result);
        assert_eq!(settled.faults, 1);
        assert_eq!(arena.minds.get(&RED).expect("mind").memory, b"prior");
        assert!(arena.minds.get(&RED).expect("mind").pending.is_none());
        assert!(
            arena
                .profiles
                .get(&RED)
                .expect("runtime")
                .lease
                .sample()
                .is_none()
        );
        assert!(!arena.due.iter().any(|event| matches!(event.kind,
            crate::arena::state::DueKind::MindLand { unit, .. }
            | crate::arena::state::DueKind::RuleLeaseExpired { unit, .. } if unit == RED)));
        let ammo = arena.unit(RED).expect("own").weapon_ammo("cannon");
        let _ = arena.evaluate_conditions(RED, at, ConditionTrigger::Landing, None, &mut counters);
        assert_eq!(
            arena.unit(RED).expect("own").weapon_ammo("cannon"),
            ammo - 1,
            "fault did not extend the previous HOLD"
        );
    }

    #[test]
    fn m6_sampled_lock_needs_live_truth_and_false_never_uses_the_prior_sample() {
        let mut arena = locked();
        arena.unit_mut(RED).expect("own").lock = None;
        m6_install_program(
            &mut arena,
            vec![m6_authored(
                Rule::lock_rule(LockSelector::Nearest, None, Vec::new()),
                true,
            )],
        );
        let boundary = 2 * arena.mind_profile(RED).think_ms;
        let (generation, lands) = m6_old_sample(&mut arena, boundary, vec![true]);
        let mut counters = MotorCounters::default();
        let _ =
            arena.evaluate_conditions(RED, lands, ConditionTrigger::Landing, None, &mut counters);
        assert_eq!(
            arena.unit(RED).expect("own").lock,
            Some(id_of(RED, BLUE)),
            "live sample permits Sensors action"
        );
        arena.unit_mut(RED).expect("own").lock = None;
        m6_pending(&mut arena, boundary, Some(vec![false]));
        arena.now_ms = boundary;
        arena.expire_rule_sample(RED, generation, lands, boundary, &mut counters);
        arena.mind_land(RED, 9, boundary, &mut counters);
        assert_eq!(arena.unit(RED).expect("own").lock, None);
        let _ = arena.evaluate_conditions(
            RED,
            boundary + 1,
            ConditionTrigger::Landing,
            None,
            &mut counters,
        );
        assert_eq!(arena.unit(RED).expect("own").lock, None);
    }
}
