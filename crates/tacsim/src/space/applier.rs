//! **The one applier from a mind's intents to bubble commands** (doc 44 unit MW.9; Q-44-24).
//!
//! Every mind that flies a hull inside a bubble is read by [`apply_intents`]: the customs
//! corvette, the raider, the interdictor, and the backup mind of a hull whose crew has no
//! link. Before this unit each of the four had its own mapping, and Rust chose for them: a
//! fixed burn for the interdictor, a target for a shot with no lock, a second posture for
//! the raider's `emcon(false)`.
//!
//! # What each intent does in space
//!
//! | The mind asks | The applier does |
//! |---|---|
//! | `lock(id)` | her lock is the ship that row names ([`BubbleInputCommand::SetLockedTarget`]) |
//! | `helm(word)` | the direction of this think's burn, a helm word ([`super::view::helm_word`]) |
//! | `speed(n)`, n above nought | she burns at `n` cm/s², as far as her drive gives, along the helm word; with no helm word, toward her lock; with neither, nothing |
//! | `speed(n)`, n below nought | the same burn the other way |
//! | `speed(0)`, `hold()` | her drive is cut |
//! | `emcon(true)` | her posture is COMBAT |
//! | `emcon(false)` | her posture is the quietest her seat allows ([`Seat::quietest`]) |
//! | `fire(id, slot)` | the mount `id` fires at her lock; each `fire` of a tube is one torpedo of one salvo |
//! | `fire(id, slot)`, `id` a drone's collector | the collector works her lock: one lift of ore from a rock in reach into the drone's hold ([`BubbleInputCommand::WorkRock`]; doc 44 unit MW.12) |
//! | anything else | nothing: no other intent has a meaning in a bubble |
//!
//! A burn stands until a later think changes it, as a heading does on the ground. A shot with
//! no lock is dropped: the applier never picks a target.
//!
//! # What the seat limits
//!
//! The applier is the same function for every actor; a [`Seat`] says what its actor may do.
//! A hull flown for an absent player is seated [`Seat::ABSENT`], which drops every lock and
//! every shot, and every burn but one that heaves her to (doc 35 §11 `B-4`: she never runs a
//! picket under power; [`BurnRule`]).
//!
//! **A mind her holder sealed aboard** (doc 44 unit MW.13; Q-44-26: "helm and sensors
//! first ... the fire channels wait for the mounts' redesign") sits in the same seat with one
//! difference: it may lock ([`Seat::lock`]). Every shot it asks is dropped, as the house
//! default's is, and its burns are held to the same rule.
//!
//! **Return fire** (doc 45 unit AP.9; Q-44-38, Q-45-8, Q-45-23). The absent player's rule is
//! one gate, `crate::arena::absent::permits`, and the bubble calls it: a seat whose
//! [`FireRule`] is `Answer` is handed [`Answers`], the facts the gate reads (the ships whose
//! fire has arrived on this one, whether she stands guard, and the ships whose fire has
//! arrived on another unit of her player), and the applier asks the gate at its two doors, a
//! `lock` and a `fire`. A lock on a ship she may not answer is dropped, because a bubble's
//! fire commands take their target from the lock; a shot at a lock she may not answer is
//! dropped whoever set the lock. The backup seat gives `Answer` to the house default under
//! every posture (doc 47 unit CO.2, R-47-3), and `Never` to every mind sealed aboard, beside
//! which the house default's own think answers by [`apply_answer`] (R-47-2).
//!
//! # A collector drone
//!
//! A drone's mind flies its sortie through this function and no other (doc 44 unit MW.12;
//! Q-44-27): it locks the rock or the ship it left, burns toward its lock or along a helm
//! word, and works the rock with its collector. Its seat is [`Seat::DRONE`]. A drone whose
//! ship's crew is away keeps its helm and loses its collector ([`Seat::work`]): the absent
//! player's rule (`crate::arena::absent`) lets no mind mine for a player who is not there.
//!
//! # What is still Rust's
//!
//! One reflex, the same for every seat that may fire: a hull over [`BLOWDOWN_HEAT_PERMILLE`]
//! vents coolant before she shoots. No intent names it.

use std::collections::{BTreeMap, BTreeSet};

use super::bubble::{
    BubbleInputCommand, BubbleInputTape, EmconPosture, HullStatus, LocalSpaceBubble,
    SpaceCombatEvent, SpaceShipState,
};
use super::view::{drive_mmps2, helm_direction};
use super::weapons::isqrt_i128;
use crate::arena::intents::{IntentKind, MotorGoal};
use crate::arena::state::ContactId;

/// The hull heat, per mille, over which a hull that is about to shoot vents coolant first.
pub const BLOWDOWN_HEAT_PERMILLE: u16 = 650;

/// The most torpedoes one think's `fire` calls put in one salvo.
pub const SALVO_MAX: u8 = 8;

/// **Whether a seat's mind may lock and shoot** (the file head, "return fire").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireRule {
    /// Its doctrine decides.
    Free,
    /// Every lock and every shot is dropped, whatever source asked.
    Never,
    /// **The absent player's rule decides** (doc 45 unit AP.9): a lock or a shot is let
    /// through only at a ship the rule lets her answer, by the [`Answers`] the seat hands
    /// [`apply_intents_answering`]. Applied with no [`Answers`] it is `Never`.
    Answer,
}

/// **What a hull flown for an absent player may answer** (doc 45 unit AP.9): the facts
/// `crate::arena::absent::permits` reads, by ship id in the bubble.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answers {
    /// The ships whose fire has arrived on her since the bubble opened.
    pub fired_on_by: BTreeSet<String>,
    /// She stands on `GUARD` and can still defend herself.
    pub guards: bool,
    /// The ships whose fire has arrived on another unit of her player in the bubble. Read
    /// only while `guards` holds.
    pub fired_on_kin: BTreeSet<String>,
}

impl Answers {
    /// **Whether the absent player's rule lets her fire on `target`**: the one gate
    /// (`crate::arena::absent::permits`), asked with these facts. Space has no hold on her
    /// fire a crew can leave standing, so the gate is told none stands.
    #[must_use]
    pub fn permits(&self, target: &str) -> bool {
        use crate::arena::absent::{Ask, Facts, permits};
        permits(
            &Facts {
                absent: true,
                holds_fire: false,
                fired_on_by: &self.fired_on_by,
                guards: self.guards,
                fired_on_kin: &self.fired_on_kin,
            },
            &Ask::Fire {
                at: target.to_owned(),
            },
        )
        .is_ok()
    }

    /// Every ship she may answer.
    #[must_use]
    pub fn all(&self) -> BTreeSet<String> {
        let mut all = self.fired_on_by.clone();
        if self.guards {
            all.extend(self.fired_on_kin.iter().cloned());
        }
        all
    }
}

/// **Which burns a seat's mind may fly.** Cutting the drive is always allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BurnRule {
    /// Its doctrine decides.
    Free,
    /// Only a burn with a part along this vector: for a hull that is being stopped, the way
    /// the ship that stops her moves against her, so the burn can only take her drift off.
    Toward([i64; 3]),
    /// Every burn is dropped.
    Never,
}

impl BurnRule {
    fn allows(self, thrust_mmps2: [i64; 3]) -> bool {
        match self {
            BurnRule::Free => true,
            BurnRule::Never => false,
            BurnRule::Toward(way) => {
                thrust_mmps2
                    .iter()
                    .zip(way)
                    .map(|(thrust, way)| i128::from(*thrust) * i128::from(way))
                    .sum::<i128>()
                    > 0
            }
        }
    }
}

/// **What the actor in a seat may do**: the limits are the seat's, so no source can step past
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seat {
    /// Whether it may shoot.
    pub fire: FireRule,
    /// **Whether it may lock** (doc 44 unit MW.13): a lock is its sensors' act. A seat that
    /// may shoot may lock. A hull flown for an absent crew may not, unless the mind that
    /// flies her is one her holder sealed aboard, whose seal carries the sensors and no
    /// fire channel: her lock is then no half of a shot.
    pub lock: bool,
    /// Which burns it may fly.
    pub burn: BurnRule,
    /// Whether a burn it asks may be flown into the reserve the hull needs to make a port.
    pub into_reserve: bool,
    /// The quietest posture `emcon(false)` gives it.
    pub quietest: EmconPosture,
    /// Whether its mind may work a rock with a collector (doc 44 unit MW.12). No seat but a
    /// drone's carries one.
    pub work: bool,
}

impl Seat {
    /// A customs or patrol hull: free to fire, keeps its reserve, never runs dark.
    pub const PATROL: Seat = Seat {
        fire: FireRule::Free,
        lock: true,
        burn: BurnRule::Free,
        into_reserve: false,
        quietest: EmconPosture::Standard,
        work: false,
    };

    /// A raider: free to fire, burns what it has, and runs dark when it falls silent.
    pub const RAIDER: Seat = Seat {
        fire: FireRule::Free,
        lock: true,
        burn: BurnRule::Free,
        into_reserve: true,
        quietest: EmconPosture::Ghost,
        work: false,
    };

    /// A hull flown for a crew with no link (doc 35 §11 `B-4`): it never locks or shoots, it
    /// may stop into its reserve, and it does not run dark. It burns only to heave to: the
    /// backup seat gives it [`BurnRule::Toward`] while a stop holds the hull, and this
    /// otherwise.
    pub const ABSENT: Seat = Seat {
        fire: FireRule::Never,
        lock: false,
        burn: BurnRule::Never,
        into_reserve: true,
        quietest: EmconPosture::Standard,
        work: false,
    };

    /// A collector drone (doc 44 unit MW.12; Q-44-27): it locks what it flies to, burns
    /// what it has and works a rock. It carries nothing but its collector, so its freedom
    /// to lock gives it nothing to shoot.
    pub const DRONE: Seat = Seat {
        fire: FireRule::Free,
        lock: true,
        burn: BurnRule::Free,
        into_reserve: true,
        quietest: EmconPosture::Standard,
        work: true,
    };
}

/// **The lock a sealed mind's rule program asks on this think** (doc 44 unit MW.13; the
/// first space seal that runs a rule program). A rule program has three native actions
/// (`crate::arena::conditions::Rule`): `Fire` and `HoldFire` are the fire channel's, which a
/// ship's seal does not carry, so their rows bind inert; `Lock` is the sensors'. In a bubble
/// a `Lock` row acts when:
///
/// - it is not inert (the seal carries the sensors);
/// - its think terms, if it has any, were sampled true by this think (`sample`, for this
///   program);
/// - its selector is `NEAREST` and every native predicate on it is a range. A predicate a
///   bubble has no fact for (odds, a band, an aspect, a scan) is never satisfied, as in the
///   arena.
///
/// It picks the nearest ship the think's view held (`contacts`) that still flies and whose
/// party the hull's party holds hostile, the lowest contact id on a tie. Rows are tried in
/// program order and the first that picks one stands.
#[must_use]
pub fn program_lock(
    program: &crate::arena::profile_rules::BoundProgram,
    sample: Option<&crate::arena::profile_rules::RuleSample>,
    bubble: &LocalSpaceBubble,
    ship_id: &str,
    contacts: &BTreeMap<ContactId, String>,
) -> Option<ContactId> {
    use crate::arena::conditions::{Comparison, LockSelector, Rule, RulePredicate};
    use crate::arena::party::Relation;
    let ship = bubble.ships.get(ship_id)?;
    for (index, row) in program.rows.iter().enumerate() {
        if row.inert {
            continue;
        }
        let Rule::Lock {
            selector: LockSelector::Nearest,
            predicates,
            ..
        } = &row.program.rule
        else {
            continue;
        };
        let sampled = sample.is_some_and(|sample| {
            sample.program == program.digest && sample.truth.get(index).copied().unwrap_or(false)
        });
        if !row.program.think.is_empty() && !sampled {
            continue;
        }
        let mut best: Option<(u64, ContactId)> = None;
        for (id, other_id) in contacts {
            let Some(other) = bubble.ships.get(other_id) else {
                continue;
            };
            if other.status != HullStatus::Operational
                || bubble.party_relation(ship.party, other.party) != Relation::Hostile
            {
                continue;
            }
            let range_m = u64::try_from(isqrt_i128(
                (0..3)
                    .map(|axis| {
                        let d = i128::from(other.pos_m[axis]) - i128::from(ship.pos_m[axis]);
                        d * d
                    })
                    .sum(),
            ))
            .unwrap_or(u64::MAX);
            let holds = predicates.iter().all(|predicate| match predicate {
                RulePredicate::Range { comparison, metres } => {
                    let metres = u64::from(*metres);
                    match comparison {
                        Comparison::Less => range_m < metres,
                        Comparison::Greater => range_m > metres,
                        Comparison::AtLeast => range_m >= metres,
                        Comparison::Equal => range_m == metres,
                    }
                }
                _ => false,
            });
            if holds && best.is_none_or(|(range, at)| (range_m, *id) < (range, at)) {
                best = Some((range_m, *id));
            }
        }
        if let Some((_, id)) = best {
            return Some(id);
        }
    }
    None
}

/// **What one think's intents came to.**
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The mind asked for the drive to be cut: `hold()` or `speed(0)`.
    pub asked_hold: bool,
    /// The mind asked for a burn that had a direction.
    pub asked_burn: bool,
    /// A torpedo or a railgun round was fired.
    pub fired_heavy: bool,
    /// A mount was told to fire at her lock: a beam, a railgun or a tube (doc 45 unit
    /// AP.9). Whether it had a round ready is the bubble's.
    pub fired: bool,
    /// Intents the seat dropped or that mean nothing in a bubble.
    pub dropped: u32,
    /// The mind asked its collector to work its lock, and the seat let it.
    pub asked_work: bool,
}

/// The families of mount a `fire` can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mount {
    Laser,
    Railgun,
    Tube,
    /// A drone's collector: it works a rock and shoots nothing.
    Collector,
}

fn mount_of(ship: &SpaceShipState, weapon: &str) -> Option<Mount> {
    if ship.collector.as_ref().is_some_and(|kit| kit.id == weapon) {
        Some(Mount::Collector)
    } else if ship.lasers.iter().any(|mount| mount.id == weapon) {
        Some(Mount::Laser)
    } else if ship.railguns.iter().any(|mount| mount.id == weapon) {
        Some(Mount::Railgun)
    } else if ship.torpedo_tubes.iter().any(|mount| mount.id == weapon) {
        Some(Mount::Tube)
    } else {
        None
    }
}

/// The burn of strength `cms2` (cm/s², its sign the way) along `direction`, in mm/s², held to
/// what the drive gives.
fn burn_along(direction: [i64; 3], cms2: i32, drive_mmps2: i64) -> Option<[i64; 3]> {
    let length = isqrt_i128(
        direction
            .iter()
            .map(|part| i128::from(*part) * i128::from(*part))
            .sum(),
    );
    if length == 0 {
        return None;
    }
    let strength = (i128::from(cms2.unsigned_abs()) * 10).min(i128::from(drive_mmps2.max(1)));
    let way = i128::from(cms2.signum());
    let part = |axis: i64| -> i64 {
        i64::try_from(i128::from(axis) * strength * way / length).unwrap_or(0)
    };
    Some(direction.map(part))
}

/// **Apply one think's intents to `ship_id`** (the file head), on the tape, pushing what the
/// bubble answers onto `events`. `contacts` is the ship each contact id of the view the mind
/// read names.
pub fn apply_intents(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    ship_id: &str,
    seat: Seat,
    contacts: &BTreeMap<ContactId, String>,
    intents: &[IntentKind],
    events: &mut Vec<SpaceCombatEvent>,
) -> Applied {
    apply_intents_answering(bubble, tape, ship_id, seat, None, contacts, intents, events)
}

/// **[`apply_intents`] for a seat that may answer fire** (doc 45 unit AP.9; the file head,
/// "return fire"): `answers` is what a [`FireRule::Answer`] seat's hull may answer. No other
/// seat reads it.
#[allow(clippy::too_many_arguments)]
pub fn apply_intents_answering(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    ship_id: &str,
    seat: Seat,
    answers: Option<&Answers>,
    contacts: &BTreeMap<ContactId, String>,
    intents: &[IntentKind],
    events: &mut Vec<SpaceCombatEvent>,
) -> Applied {
    // The absent player's rule at its two doors: whether this seat may lock or fire on
    // `target`. A seat free to fire may; one that never fires may not; one that answers
    // asks the gate.
    let may_fire_on = |target: &str| match seat.fire {
        FireRule::Free => true,
        FireRule::Never => false,
        FireRule::Answer => answers.is_some_and(|answers| answers.permits(target)),
    };
    let mut applied = Applied::default();
    if bubble
        .ships
        .get(ship_id)
        .is_none_or(|ship| ship.status != HullStatus::Operational)
    {
        return applied;
    }
    let mut command = |bubble: &mut LocalSpaceBubble, command: BubbleInputCommand| {
        events.extend(tape.record_and_apply(bubble, command));
    };

    let mut heading: Option<u32> = None;
    let mut strength: Option<i32> = None;
    let mut shots: Vec<Mount> = Vec::new();
    for intent in intents {
        match intent {
            IntentKind::Emcon { on } => {
                let posture = if *on {
                    EmconPosture::ActiveCombat
                } else {
                    seat.quietest
                };
                if bubble
                    .ships
                    .get(ship_id)
                    .is_some_and(|ship| ship.emcon != posture)
                {
                    command(
                        bubble,
                        BubbleInputCommand::SetEmcon {
                            ship_id: ship_id.to_owned(),
                            posture,
                        },
                    );
                }
            }
            IntentKind::Lock { contact } if seat.lock || seat.fire == FireRule::Answer => {
                // A seat with the sensors locks what it likes. One that only answers locks
                // a ship it may answer and no other: its lock is half of a shot.
                match contacts
                    .get(contact)
                    .filter(|target| seat.lock || may_fire_on(target))
                {
                    Some(target) => {
                        if bubble
                            .ships
                            .get(ship_id)
                            .is_some_and(|ship| ship.locked_target_id.as_deref() != Some(target))
                        {
                            command(
                                bubble,
                                BubbleInputCommand::SetLockedTarget {
                                    ship_id: ship_id.to_owned(),
                                    target_id: Some(target.clone()),
                                },
                            );
                        }
                    }
                    None => applied.dropped += 1,
                }
            }
            IntentKind::Heading { mdeg } => heading = Some(*mdeg),
            IntentKind::Speed { cms } => strength = Some(*cms),
            IntentKind::Goal(MotorGoal::Hold) => strength = Some(0),
            IntentKind::Fire { weapon, .. } => {
                // A collector is the seat's to work or not; every other mount is the seat's
                // to fire or not.
                match bubble
                    .ships
                    .get(ship_id)
                    .and_then(|ship| mount_of(ship, weapon))
                {
                    Some(Mount::Collector) if seat.work => shots.push(Mount::Collector),
                    Some(mount) if mount != Mount::Collector && seat.fire != FireRule::Never => {
                        shots.push(mount);
                    }
                    _ => applied.dropped += 1,
                }
            }
            _ => applied.dropped += 1,
        }
    }

    // The helm: one burn a think, the last strength asked along the last direction asked.
    if let Some(cms2) = strength
        && let Some(ship) = bubble.ships.get(ship_id)
    {
        let burning = ship.thrust_mmps2 != [0, 0, 0] || ship.thrust_mps2 != [0, 0, 0];
        let want = if cms2 == 0 {
            applied.asked_hold = true;
            Some([0, 0, 0])
        } else {
            let direction = match heading {
                Some(word) => Some(helm_direction(word, 1_000_000)),
                None => ship
                    .locked_target_id
                    .as_deref()
                    .and_then(|locked| bubble.ships.get(locked))
                    .map(|target| {
                        [
                            target.pos_m[0].saturating_sub(ship.pos_m[0]),
                            target.pos_m[1].saturating_sub(ship.pos_m[1]),
                            target.pos_m[2].saturating_sub(ship.pos_m[2]),
                        ]
                    }),
            };
            direction
                .and_then(|direction| burn_along(direction, cms2, drive_mmps2(ship)))
                .filter(|thrust_mmps2| seat.burn.allows(*thrust_mmps2))
        };
        match want {
            Some(thrust_mmps2) => {
                applied.asked_burn = thrust_mmps2 != [0, 0, 0];
                if ship.thrust_mmps2 != thrust_mmps2 || (thrust_mmps2 == [0, 0, 0] && burning) {
                    command(
                        bubble,
                        BubbleInputCommand::CommandBurnMmps2 {
                            ship_id: ship_id.to_owned(),
                            thrust_mmps2,
                            force_into_reserve: seat.into_reserve,
                        },
                    );
                }
            }
            None => applied.dropped += 1,
        }
    }

    // The mounts, at her lock, in the order the mind named them.
    let target = bubble
        .ships
        .get(ship_id)
        .and_then(|ship| ship.locked_target_id.clone());
    let Some(target) = target.filter(|_| !shots.is_empty()) else {
        applied.dropped += u32::try_from(shots.len()).unwrap_or(u32::MAX);
        return applied;
    };
    shoot(
        bubble,
        tape,
        events,
        ship_id,
        &target,
        shots,
        may_fire_on(&target),
        &mut applied,
    );
    applied
}

/// **The house default's answer beside a mind sealed aboard** (doc 47 unit CO.2; R-47-2):
/// the fire channel of the house default's think on a hull whose helm and sensors are the
/// sealed mind's ([`Seat::lock`]). The two minds thought on one view, so `contacts` names
/// the ships of both.
///
/// The target is the ship the think's last `lock` names, and its `fire`s are the shots. The
/// shots go through the one gate, as an answering seat's do: at a ship `answers` does not
/// let her answer they are dropped. Her lock is **not** moved: a bubble's fire command
/// carries its target, so the answer is fired at its ship and the lock the sealed mind set
/// stands. Every other intent of the think (its helm, its emission posture) is dropped: they
/// are the sealed mind's.
pub fn apply_answer(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    ship_id: &str,
    answers: &Answers,
    contacts: &BTreeMap<ContactId, String>,
    intents: &[IntentKind],
    events: &mut Vec<SpaceCombatEvent>,
) -> Applied {
    let mut applied = Applied::default();
    if bubble
        .ships
        .get(ship_id)
        .is_none_or(|ship| ship.status != HullStatus::Operational)
    {
        return applied;
    }
    let mut target: Option<String> = None;
    let mut shots: Vec<Mount> = Vec::new();
    for intent in intents {
        match intent {
            IntentKind::Lock { contact } => match contacts.get(contact) {
                Some(ship) => target = Some(ship.clone()),
                None => applied.dropped += 1,
            },
            IntentKind::Fire { weapon, .. } => match bubble
                .ships
                .get(ship_id)
                .and_then(|ship| mount_of(ship, weapon))
            {
                Some(mount) if mount != Mount::Collector => shots.push(mount),
                _ => applied.dropped += 1,
            },
            _ => applied.dropped += 1,
        }
    }
    let Some(target) = target.filter(|_| !shots.is_empty()) else {
        applied.dropped += u32::try_from(shots.len()).unwrap_or(u32::MAX);
        return applied;
    };
    let permitted = answers.permits(&target);
    shoot(
        bubble,
        tape,
        events,
        ship_id,
        &target,
        shots,
        permitted,
        &mut applied,
    );
    applied
}

/// **Fire `shots` from `ship_id` at `target`**, one command a family, in the order named;
/// `permitted` is whether the absent player's rule lets her fire on `target` (a collector
/// shoots nothing and is worked either way).
#[allow(clippy::too_many_arguments)]
fn shoot(
    bubble: &mut LocalSpaceBubble,
    tape: &mut BubbleInputTape,
    events: &mut Vec<SpaceCombatEvent>,
    ship_id: &str,
    target: &str,
    mut shots: Vec<Mount>,
    permitted: bool,
    applied: &mut Applied,
) {
    let mut command = |bubble: &mut LocalSpaceBubble, command: BubbleInputCommand| {
        events.extend(tape.record_and_apply(bubble, command));
    };
    // The second door (the file head, "return fire"): a shot at a lock the rule does not
    // let her answer is dropped, whoever set that lock. A collector shoots nothing and is
    // the seat's to work or not.
    if !permitted {
        let before = shots.len();
        shots.retain(|mount| *mount == Mount::Collector);
        applied.dropped += u32::try_from(before - shots.len()).unwrap_or(u32::MAX);
        if shots.is_empty() {
            return;
        }
    }
    if shots.iter().any(|mount| *mount != Mount::Collector)
        && bubble
            .ships
            .get(ship_id)
            .is_some_and(|ship| ship.heat_permille > BLOWDOWN_HEAT_PERMILLE)
    {
        command(
            bubble,
            BubbleInputCommand::BlowdownCoolant {
                ship_id: ship_id.to_owned(),
            },
        );
    }
    let salvo = u8::try_from(shots.iter().filter(|mount| **mount == Mount::Tube).count())
        .unwrap_or(SALVO_MAX)
        .min(SALVO_MAX);
    let mut fired: Vec<Mount> = Vec::new();
    for mount in shots {
        // One command a family a think: the bubble fires every mount of it that is ready.
        if fired.contains(&mount) {
            continue;
        }
        fired.push(mount);
        let shooter_id = ship_id.to_owned();
        let target_id = target.to_owned();
        match mount {
            Mount::Laser => {
                applied.fired = true;
                command(
                    bubble,
                    BubbleInputCommand::FireOffensiveLasers {
                        shooter_id,
                        target_id,
                    },
                );
            }
            Mount::Railgun => {
                applied.fired = true;
                applied.fired_heavy = true;
                command(
                    bubble,
                    BubbleInputCommand::FireRailgun {
                        shooter_id,
                        target_id,
                    },
                );
            }
            Mount::Tube => {
                applied.fired = true;
                applied.fired_heavy = true;
                command(
                    bubble,
                    BubbleInputCommand::FireTorpedoSalvo {
                        shooter_id,
                        target_id,
                        count: salvo,
                    },
                );
            }
            Mount::Collector => {
                applied.asked_work = true;
                command(
                    bubble,
                    BubbleInputCommand::WorkRock {
                        ship_id: shooter_id,
                        rock_id: target_id,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::picket::{CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY};
    use super::*;
    use crate::arena::party::Mark;
    use smol_str::SmolStr;

    fn field() -> (
        LocalSpaceBubble,
        BubbleInputTape,
        BTreeMap<ContactId, String>,
    ) {
        let mut bubble = LocalSpaceBubble::new("bubble.test.applier", "domain.3", "Test", 1_000);
        bubble.insert_ship(SpaceShipState::new_corvette(
            "patrol",
            "Patrol",
            CUSTOMS_CORVETTE_PARTY,
            [0, 0, 0],
            [0, 0, 0],
        ));
        bubble.insert_ship(SpaceShipState::new_dray(
            "dray",
            "Dray",
            CARRIER_PARTY,
            [0, 30_000, 40_000],
            [0, 0, 0],
        ));
        bubble.set_party_relation(CUSTOMS_CORVETTE_PARTY, CARRIER_PARTY, Some(Mark::Hostile));
        let tape = BubbleInputTape::new(&bubble);
        let contacts = BTreeMap::from([(ContactId(1), "dray".to_owned())]);
        (bubble, tape, contacts)
    }

    fn thrust(bubble: &LocalSpaceBubble, ship: &str) -> [i64; 3] {
        bubble
            .ships
            .get(ship)
            .map_or([0; 3], |ship| ship.thrust_mmps2)
    }

    #[test]
    fn a_speed_with_no_helm_word_burns_toward_the_lock_and_a_hold_cuts_it() {
        let (mut bubble, mut tape, contacts) = field();
        let mut events = Vec::new();
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            "patrol",
            Seat::PATROL,
            &contacts,
            &[
                IntentKind::Lock {
                    contact: ContactId(1),
                },
                IntentKind::Speed { cms: 500 },
            ],
            &mut events,
        );
        assert!(applied.asked_burn);
        // 5 m/s² along (0, 3, 4) / 5: nothing on x, 3 on y, 4 on z.
        assert_eq!(thrust(&bubble, "patrol"), [0, 3_000, 4_000]);
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            "patrol",
            Seat::PATROL,
            &contacts,
            &[IntentKind::Goal(MotorGoal::Hold)],
            &mut events,
        );
        assert!(applied.asked_hold);
        assert_eq!(thrust(&bubble, "patrol"), [0, 0, 0]);
    }

    #[test]
    fn a_helm_word_gives_the_burn_its_direction_and_a_speed_below_nought_reverses_it() {
        let (mut bubble, mut tape, contacts) = field();
        let mut events = Vec::new();
        let up = super::super::view::helm_word(90_000, 90_000);
        let _ = apply_intents(
            &mut bubble,
            &mut tape,
            "patrol",
            Seat::PATROL,
            &contacts,
            &[
                IntentKind::Heading { mdeg: up },
                IntentKind::Speed { cms: -200 },
            ],
            &mut events,
        );
        assert_eq!(thrust(&bubble, "patrol"), [0, 0, -2_000]);
    }

    #[test]
    fn a_speed_with_no_helm_word_and_no_lock_is_dropped() {
        let (mut bubble, mut tape, contacts) = field();
        let mut events = Vec::new();
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            "patrol",
            Seat::PATROL,
            &contacts,
            &[IntentKind::Speed { cms: 500 }],
            &mut events,
        );
        assert!(!applied.asked_burn);
        assert_eq!(applied.dropped, 1);
        assert_eq!(thrust(&bubble, "patrol"), [0, 0, 0]);
    }

    /// **Doc 44 unit MW.13 (Q-44-26): a mind sealed aboard has the helm and the sensors and
    /// no fire channel.** In the absent seat with its lock let through, its `lock` lands and
    /// its `fire` is dropped, with a mount to fire and a lock to fire it at; its burns are
    /// held to the rule every source is held to.
    #[test]
    fn mw13_a_sealed_minds_seat_locks_and_never_shoots() {
        let (mut bubble, mut tape, _) = field();
        let contacts = BTreeMap::from([(ContactId(1), "patrol".to_owned())]);
        if let Some(ship) = bubble.ships.get_mut("dray") {
            ship.lasers
                .push(super::super::weapons::LaserMount::standard_turret(
                    "laser_1",
                ));
        }
        let sealed = Seat {
            lock: true,
            burn: BurnRule::Toward([1_000, 0, 0]),
            ..Seat::ABSENT
        };
        assert_eq!(sealed.fire, FireRule::Never);
        let mut events = Vec::new();
        let asks = [
            IntentKind::Lock {
                contact: ContactId(1),
            },
            IntentKind::Fire {
                weapon: SmolStr::new_static("laser_1"),
                slot: Some(1),
            },
            IntentKind::Heading { mdeg: 180_000 },
            IntentKind::Speed { cms: 5 },
        ];
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            "dray",
            sealed,
            &contacts,
            &asks,
            &mut events,
        );
        let dray = bubble.ships.get("dray").expect("the dray");
        assert_eq!(
            dray.locked_target_id.as_deref(),
            Some("patrol"),
            "her sensors are the mind's: the lock lands"
        );
        assert_eq!(
            applied.dropped, 2,
            "the shot is dropped, and the burn that would add to her drift"
        );
        assert!(!applied.fired_heavy);
        assert!(events.is_empty(), "nothing was fired: {events:?}");
        assert!(
            !tape.entries.iter().any(|entry| matches!(
                entry.command,
                BubbleInputCommand::FireOffensiveLasers { .. }
                    | BubbleInputCommand::FireRailgun { .. }
                    | BubbleInputCommand::FireTorpedoSalvo { .. }
            )),
            "no fire command reached the tape"
        );
        assert_eq!(dray.thrust_mmps2, [0, 0, 0]);
        // The house default in the same seat, with the same asks: no lock either.
        let (mut bubble, mut tape, _) = field();
        let _ = apply_intents(
            &mut bubble,
            &mut tape,
            "dray",
            Seat::ABSENT,
            &contacts,
            &asks,
            &mut events,
        );
        assert_eq!(
            bubble
                .ships
                .get("dray")
                .and_then(|ship| ship.locked_target_id.clone()),
            None
        );
    }

    /// **Doc 44 unit MW.13: what a bound rule program does in a bubble.** A `LOCK NEAREST`
    /// row picks the nearest hostile the view held; a fire row binds inert on a seal with no
    /// fire channel; a range predicate is weighed; a predicate a bubble has no fact for is
    /// never satisfied; a row with a think term acts only on a think that sampled it true.
    #[test]
    fn mw13_a_lock_row_of_a_rule_program_picks_the_nearest_hostile() {
        use crate::arena::conditions::{Comparison, LockSelector, Rule, RulePredicate};
        use crate::arena::core::{ClassTable, CoreClass};
        use crate::arena::profile_rules::{BoundProgram, RuleSample};
        use crate::arena::schema::{AuthoredRule, ThinkPredicate};
        use crate::arena::state::Channel;

        let (mut bubble, _, _) = field();
        bubble.set_party_relation(CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY, Some(Mark::Hostile));
        let contacts = BTreeMap::from([(ContactId(1), "patrol".to_owned())]);
        let class = ClassTable::STOCK.row(CoreClass::II);
        let bind = |rules: Vec<AuthoredRule>| {
            BoundProgram::bind(&rules, &[], &[Channel::Helm, Channel::Sensors], &class, 100)
                .expect("the program binds")
        };
        let lock = |predicates: Vec<RulePredicate>, think: Vec<ThinkPredicate>| AuthoredRule {
            rule: Rule::lock_rule(LockSelector::Nearest, None, predicates),
            think,
        };
        let asked = |program: &BoundProgram, sample: Option<&RuleSample>| {
            program_lock(program, sample, &bubble, "dray", &contacts)
        };

        assert_eq!(asked(&bind(Vec::new()), None), None, "no row, no lock");
        assert_eq!(
            asked(&bind(vec![lock(Vec::new(), Vec::new())]), None),
            Some(ContactId(1)),
            "LOCK NEAREST picks the hostile"
        );
        // The corvette is 50 km off.
        let near = RulePredicate::Range {
            comparison: Comparison::Less,
            metres: 10_000,
        };
        let far = RulePredicate::Range {
            comparison: Comparison::Greater,
            metres: 10_000,
        };
        assert_eq!(asked(&bind(vec![lock(vec![near], Vec::new())]), None), None);
        assert_eq!(
            asked(&bind(vec![lock(vec![far], Vec::new())]), None),
            Some(ContactId(1))
        );
        let unknowable = RulePredicate::Aspect { mask: 1 };
        assert_eq!(
            asked(&bind(vec![lock(vec![unknowable], Vec::new())]), None),
            None,
            "a bubble has no fact for an aspect"
        );
        // A fire row is inert on a seal with no fire channel, and is no lock.
        let hold = AuthoredRule {
            rule: Rule::hold_rule(SmolStr::new_static("cannon"), Vec::new()),
            think: Vec::new(),
        };
        let program = bind(vec![hold]);
        assert!(program.rows.iter().all(|row| row.inert));
        assert_eq!(asked(&program, None), None);
        // A think term: unsampled it holds the row; sampled true it lets it act.
        let program = bind(vec![lock(
            Vec::new(),
            vec![ThinkPredicate::Hostiles { at_least: 1 }],
        )]);
        assert_eq!(asked(&program, None), None);
        let sample = |truth: bool| RuleSample {
            program: program.digest,
            generation: 0,
            sampled_ms: 0,
            lands_ms: 0,
            expires_ms: 1_000,
            truth: vec![truth],
        };
        assert_eq!(asked(&program, Some(&sample(false))), None);
        assert_eq!(asked(&program, Some(&sample(true))), Some(ContactId(1)));
        // A ship that is not held hostile is not locked by a rule.
        let mut neutral = bubble.clone();
        neutral.set_party_relation(CARRIER_PARTY, CUSTOMS_CORVETTE_PARTY, None);
        assert_eq!(
            program_lock(
                &bind(vec![lock(Vec::new(), Vec::new())]),
                None,
                &neutral,
                "dray",
                &contacts
            ),
            None
        );
    }

    #[test]
    fn the_absent_seat_drops_every_lock_and_every_shot_and_burns_only_to_heave_to() {
        let (mut bubble, mut tape, _) = field();
        let contacts = BTreeMap::from([(ContactId(1), "patrol".to_owned())]);
        if let Some(ship) = bubble.ships.get_mut("dray") {
            ship.lasers
                .push(super::super::weapons::LaserMount::standard_turret(
                    "laser_1",
                ));
        }
        let mut events = Vec::new();
        // The ship that stops her moves along +x against her.
        let stopped = Seat {
            burn: BurnRule::Toward([1_000, 0, 0]),
            ..Seat::ABSENT
        };
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            "dray",
            stopped,
            &contacts,
            &[
                IntentKind::Lock {
                    contact: ContactId(1),
                },
                IntentKind::Fire {
                    weapon: SmolStr::new_static("laser_1"),
                    slot: Some(1),
                },
                IntentKind::Emcon { on: false },
                IntentKind::Heading { mdeg: 0 },
                IntentKind::Speed { cms: 1 },
            ],
            &mut events,
        );
        assert_eq!(applied.dropped, 2, "the lock and the shot");
        let dray = bubble.ships.get("dray").expect("the dray");
        assert_eq!(dray.locked_target_id, None);
        assert_eq!(dray.emcon, EmconPosture::Standard);
        assert_eq!(dray.thrust_mmps2, [10, 0, 0]);
        assert!(events.is_empty(), "nothing was fired: {events:?}");

        // A burn the other way would add to her drift: dropped, and the burn she has stands.
        let away = [
            IntentKind::Heading { mdeg: 180_000 },
            IntentKind::Speed { cms: 5 },
        ];
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            "dray",
            stopped,
            &contacts,
            &away,
            &mut events,
        );
        assert!(!applied.asked_burn);
        assert_eq!(thrust(&bubble, "dray"), [10, 0, 0]);
        // With nothing stopping her the absent seat flies no burn at all, and still cuts one.
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            "dray",
            Seat::ABSENT,
            &contacts,
            &[
                IntentKind::Heading { mdeg: 0 },
                IntentKind::Speed { cms: 5 },
            ],
            &mut events,
        );
        assert!(!applied.asked_burn);
        assert_eq!(thrust(&bubble, "dray"), [10, 0, 0]);
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            "dray",
            Seat::ABSENT,
            &contacts,
            &[IntentKind::Goal(MotorGoal::Hold)],
            &mut events,
        );
        assert!(applied.asked_hold);
        assert_eq!(thrust(&bubble, "dray"), [0, 0, 0]);
    }

    #[test]
    fn a_shot_with_no_lock_is_dropped_and_never_given_a_target() {
        let (mut bubble, mut tape, contacts) = field();
        let weapon = bubble
            .ships
            .get("patrol")
            .and_then(|ship| ship.lasers.first())
            .map(|mount| SmolStr::new(&mount.id))
            .expect("a corvette carries a laser");
        let mut events = Vec::new();
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            "patrol",
            Seat::PATROL,
            &contacts,
            &[IntentKind::Fire { weapon, slot: None }],
            &mut events,
        );
        assert_eq!(applied.dropped, 1);
        assert!(events.is_empty(), "{events:?}");
    }

    /// Doc 44 unit MW.12 (Q-44-27): a `fire` that names a drone's collector works the rock
    /// the drone has locked, as one command on the tape, and only for a seat that may work.
    #[test]
    fn a_collector_works_the_rock_its_drone_has_locked_for_a_seat_that_may_work() {
        use super::super::sortie::{BAY_SHIP, COLLECTOR_ID, DRONE_SHIP, ROCK_SHIP, rig};

        let mut bubble = rig::flight(1, 3).bubble;
        // Beside the rock, at rest against it.
        let rock_at = bubble.ships[ROCK_SHIP].pos_m;
        if let Some(drone) = bubble.ships.get_mut(DRONE_SHIP) {
            drone.pos_m = [rock_at[0] - 50, rock_at[1], rock_at[2]];
        }
        let mut tape = BubbleInputTape::new(&bubble);
        let contacts = BTreeMap::from([
            (ContactId(1), ROCK_SHIP.to_owned()),
            (ContactId(2), BAY_SHIP.to_owned()),
        ]);
        let ask = [
            IntentKind::Lock {
                contact: ContactId(1),
            },
            IntentKind::Fire {
                weapon: SmolStr::new_static(COLLECTOR_ID),
                slot: Some(1),
            },
        ];
        let hold_kg = |bubble: &LocalSpaceBubble| {
            bubble.ships[DRONE_SHIP]
                .collector
                .as_ref()
                .map_or(0, |kit| kit.hold_kg)
        };
        let mut events = Vec::new();

        // A seat that may not work: the lock stands, the lift is dropped.
        for seat in [
            Seat {
                work: false,
                ..Seat::DRONE
            },
            Seat::PATROL,
        ] {
            let applied = apply_intents(
                &mut bubble,
                &mut tape,
                DRONE_SHIP,
                seat,
                &contacts,
                &ask,
                &mut events,
            );
            assert!(!applied.asked_work);
            assert_eq!(applied.dropped, 1);
            assert_eq!(hold_kg(&bubble), 0);
        }
        assert_eq!(
            bubble.ships[DRONE_SHIP].locked_target_id.as_deref(),
            Some(ROCK_SHIP)
        );
        assert!(
            !tape
                .entries
                .iter()
                .any(|entry| matches!(entry.command, BubbleInputCommand::WorkRock { .. }))
        );

        // The drone's own seat: one lift, one command.
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            DRONE_SHIP,
            Seat::DRONE,
            &contacts,
            &ask,
            &mut events,
        );
        assert!(applied.asked_work);
        assert_eq!(applied.dropped, 0);
        assert_eq!(hold_kg(&bubble), 1_000);
        assert_eq!(bubble.ships[ROCK_SHIP].ore_kg, Some(2_000));
        assert_eq!(
            tape.entries.last().map(|entry| entry.command.clone()),
            Some(BubbleInputCommand::WorkRock {
                ship_id: DRONE_SHIP.to_owned(),
                rock_id: ROCK_SHIP.to_owned(),
            })
        );
        assert!(events.is_empty(), "a lift is no combat event: {events:?}");
        // The tape alone reproduces it.
        assert_eq!(tape.replay().0, bubble);

        // Locked on the ship it left, the collector has nothing to work.
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            DRONE_SHIP,
            Seat::DRONE,
            &contacts,
            &[
                IntentKind::Lock {
                    contact: ContactId(2),
                },
                IntentKind::Fire {
                    weapon: SmolStr::new_static(COLLECTOR_ID),
                    slot: Some(1),
                },
            ],
            &mut events,
        );
        assert!(applied.asked_work);
        assert_eq!(hold_kg(&bubble), 1_000, "the ship is not a rock");
        // A mount it does not carry is dropped, as on any hull.
        let applied = apply_intents(
            &mut bubble,
            &mut tape,
            DRONE_SHIP,
            Seat::DRONE,
            &contacts,
            &[IntentKind::Fire {
                weapon: SmolStr::new_static("laser_dorsal"),
                slot: Some(1),
            }],
            &mut events,
        );
        assert_eq!(applied.dropped, 1);
    }

    /// A bubble of three parties for the answering seat: the Dray, a raider that her party
    /// has marked hostile, and a second raider marked the same. Every ship is a contact of
    /// the Dray's. The Dray carries one beam, `laser_1`.
    fn three_parties() -> (
        LocalSpaceBubble,
        BubbleInputTape,
        BTreeMap<ContactId, String>,
    ) {
        use super::super::picket::BELT_RAIDER_PARTY;
        let other = super::super::bubble::admitted_party(4);
        let mut bubble = LocalSpaceBubble::new("bubble.test.answer", "domain.3", "Test", 1_000);
        let mut dray =
            SpaceShipState::new_dray("dray", "Dray", CARRIER_PARTY, [0, 0, 0], [0, 0, 0]);
        dray.lasers.clear();
        dray.lasers
            .push(super::super::weapons::LaserMount::standard_turret(
                "laser_1",
            ));
        bubble.insert_ship(dray);
        bubble.insert_ship(SpaceShipState::new_corvette(
            "raider",
            "Raider",
            BELT_RAIDER_PARTY,
            [0, 3_000, 4_000],
            [0, 0, 0],
        ));
        bubble.insert_ship(SpaceShipState::new_corvette(
            "second",
            "Second",
            other,
            [0, -3_000, 4_000],
            [0, 0, 0],
        ));
        bubble.set_party_relation(CARRIER_PARTY, BELT_RAIDER_PARTY, Some(Mark::Hostile));
        bubble.set_party_relation(CARRIER_PARTY, other, Some(Mark::Hostile));
        let tape = BubbleInputTape::new(&bubble);
        let contacts = BTreeMap::from([
            (ContactId(1), "raider".to_owned()),
            (ContactId(2), "second".to_owned()),
        ]);
        (bubble, tape, contacts)
    }

    fn lock_and_fire(contact: u64) -> [IntentKind; 2] {
        [
            IntentKind::Lock {
                contact: ContactId(contact),
            },
            IntentKind::Fire {
                weapon: SmolStr::new_static("laser_1"),
                slot: Some(1),
            },
        ]
    }

    fn beams_fired(tape: &BubbleInputTape) -> Vec<String> {
        tape.entries
            .iter()
            .filter_map(|entry| match &entry.command {
                BubbleInputCommand::FireOffensiveLasers { target_id, .. } => {
                    Some(target_id.clone())
                }
                _ => None,
            })
            .collect()
    }

    const ANSWERING: Seat = Seat {
        fire: FireRule::Answer,
        ..Seat::ABSENT
    };

    /// **Doc 45 unit AP.9: a seat that answers asks the absent player's rule at a lock and
    /// at a shot.** With the raider's fire arrived on her, her lock on the raider lands and
    /// her beam fires on it; the same asks at the second ship, which her party marks hostile
    /// and which has fired on nobody, are dropped at both doors. With nothing arrived on her
    /// she answers nobody, though her party holds both hostile.
    #[test]
    fn ap9_a_seat_that_answers_locks_and_fires_only_on_a_ship_that_fired_on_her() {
        let hit_by_raider = Answers {
            fired_on_by: BTreeSet::from(["raider".to_owned()]),
            ..Answers::default()
        };
        // The raider fired on her: she answers it.
        let (mut bubble, mut tape, contacts) = three_parties();
        let mut events = Vec::new();
        let applied = apply_intents_answering(
            &mut bubble,
            &mut tape,
            "dray",
            ANSWERING,
            Some(&hit_by_raider),
            &contacts,
            &lock_and_fire(1),
            &mut events,
        );
        assert!(applied.fired, "{applied:?}");
        assert_eq!(applied.dropped, 0);
        assert_eq!(
            bubble
                .ships
                .get("dray")
                .and_then(|ship| ship.locked_target_id.clone())
                .as_deref(),
            Some("raider")
        );
        assert_eq!(beams_fired(&tape), vec!["raider".to_owned()]);
        assert!(
            events.iter().any(
                |event| matches!(event, SpaceCombatEvent::LaserHit { shooter_id, target_id, .. }
                    if shooter_id == "dray" && target_id == "raider")
            ),
            "her beam lands on the raider: {events:?}"
        );
        assert!(
            bubble
                .fired_on_by("raider")
                .is_some_and(|by| by.contains("dray")),
            "and her own fire is on the bubble's record"
        );

        // The second ship is only marked hostile: the lock and the shot are both dropped.
        let (mut bubble, mut tape, contacts) = three_parties();
        let mut events = Vec::new();
        let applied = apply_intents_answering(
            &mut bubble,
            &mut tape,
            "dray",
            ANSWERING,
            Some(&hit_by_raider),
            &contacts,
            &lock_and_fire(2),
            &mut events,
        );
        assert!(!applied.fired);
        assert_eq!(applied.dropped, 2, "the lock, and the shot with no lock");
        assert_eq!(
            bubble
                .ships
                .get("dray")
                .and_then(|ship| ship.locked_target_id.clone()),
            None
        );
        assert!(beams_fired(&tape).is_empty() && events.is_empty());

        // Nothing has arrived on her: she answers nobody, and an `Answer` seat handed no
        // facts at all is a seat that never fires.
        for answers in [Some(Answers::default()), None] {
            let (mut bubble, mut tape, contacts) = three_parties();
            let mut events = Vec::new();
            let applied = apply_intents_answering(
                &mut bubble,
                &mut tape,
                "dray",
                ANSWERING,
                answers.as_ref(),
                &contacts,
                &lock_and_fire(1),
                &mut events,
            );
            assert!(!applied.fired, "{answers:?}");
            assert!(beams_fired(&tape).is_empty() && events.is_empty());
            assert_eq!(
                bubble
                    .ships
                    .get("dray")
                    .and_then(|ship| ship.locked_target_id.clone()),
                None
            );
        }
    }

    /// **Doc 45 unit AP.9: the second door does not trust the lock.** A lock her crew left
    /// standing on a ship she may not answer is not a target: the shot is dropped. The same
    /// asks on `RETURN`'s seat, which never fires, are dropped whatever has arrived on her.
    #[test]
    fn ap9_a_shot_at_a_lock_she_may_not_answer_is_dropped_and_return_never_fires() {
        let hit_by_raider = Answers {
            fired_on_by: BTreeSet::from(["raider".to_owned()]),
            ..Answers::default()
        };
        let fire = [IntentKind::Fire {
            weapon: SmolStr::new_static("laser_1"),
            slot: Some(1),
        }];
        let (mut bubble, mut tape, contacts) = three_parties();
        assert!(bubble.set_locked_target("dray", Some("second".to_owned())));
        let mut events = Vec::new();
        let applied = apply_intents_answering(
            &mut bubble,
            &mut tape,
            "dray",
            ANSWERING,
            Some(&hit_by_raider),
            &contacts,
            &fire,
            &mut events,
        );
        assert!(!applied.fired);
        assert_eq!(applied.dropped, 1);
        assert!(beams_fired(&tape).is_empty() && events.is_empty());

        // `RETURN`: the absent seat as it was, with the raider's fire arrived on her.
        let (mut bubble, mut tape, contacts) = three_parties();
        let applied = apply_intents_answering(
            &mut bubble,
            &mut tape,
            "dray",
            Seat::ABSENT,
            Some(&hit_by_raider),
            &contacts,
            &lock_and_fire(1),
            &mut events,
        );
        assert!(!applied.fired);
        assert_eq!(applied.dropped, 2);
        assert!(beams_fired(&tape).is_empty() && events.is_empty());
    }

    /// **Doc 45 unit AP.9 (Q-45-8): `GUARD` widens the rule by one case.** A ship whose fire
    /// has arrived on another unit of her player is answered by a hull that stands guard and
    /// by no other; the facts are the same but for the one word.
    #[test]
    fn ap9_a_guard_answers_for_her_players_other_unit_and_a_hold_does_not() {
        for (guards, answers_it) in [(true, true), (false, false)] {
            let facts = Answers {
                fired_on_by: BTreeSet::new(),
                guards,
                fired_on_kin: BTreeSet::from(["raider".to_owned()]),
            };
            assert_eq!(facts.permits("raider"), answers_it);
            assert!(
                !facts.permits("second"),
                "never a ship that fired on nobody"
            );
            assert_eq!(facts.all().contains("raider"), answers_it);
            let (mut bubble, mut tape, contacts) = three_parties();
            let mut events = Vec::new();
            let applied = apply_intents_answering(
                &mut bubble,
                &mut tape,
                "dray",
                ANSWERING,
                Some(&facts),
                &contacts,
                &lock_and_fire(1),
                &mut events,
            );
            assert_eq!(applied.fired, answers_it, "guards {guards}");
            assert_eq!(beams_fired(&tape).len(), usize::from(answers_it));
        }
    }
}
