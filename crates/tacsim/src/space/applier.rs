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
//! **The seam for return fire.** Q-44-38 lets such a mind answer a unit that has fired on
//! it, and nothing else. That rule is one gate, built in the arena by doc 44 unit MW.7; the
//! space half is [`FireRule`], which today has `Free` and `Never`. The follow-up adds the
//! variant that asks MW.7's rule, and the record of who fired on whom in a bubble
//! (`Relations::fired` is by party, and the rule is by unit).
//!
//! # What is still Rust's
//!
//! One reflex, the same for every seat that may fire: a hull over [`BLOWDOWN_HEAT_PERMILLE`]
//! vents coolant before she shoots. No intent names it.

use std::collections::BTreeMap;

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

/// **Whether a seat's mind may lock and shoot** (the file head has the seam for Q-44-38).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireRule {
    /// Its doctrine decides.
    Free,
    /// Every lock and every shot is dropped, whatever source asked.
    Never,
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
    /// Whether it may lock and shoot.
    pub fire: FireRule,
    /// Which burns it may fly.
    pub burn: BurnRule,
    /// Whether a burn it asks may be flown into the reserve the hull needs to make a port.
    pub into_reserve: bool,
    /// The quietest posture `emcon(false)` gives it.
    pub quietest: EmconPosture,
}

impl Seat {
    /// A customs or patrol hull: free to fire, keeps its reserve, never runs dark.
    pub const PATROL: Seat = Seat {
        fire: FireRule::Free,
        burn: BurnRule::Free,
        into_reserve: false,
        quietest: EmconPosture::Standard,
    };

    /// A raider: free to fire, burns what it has, and runs dark when it falls silent.
    pub const RAIDER: Seat = Seat {
        fire: FireRule::Free,
        burn: BurnRule::Free,
        into_reserve: true,
        quietest: EmconPosture::Ghost,
    };

    /// A hull flown for a crew with no link (doc 35 §11 `B-4`): it never locks or shoots, it
    /// may stop into its reserve, and it does not run dark. It burns only to heave to: the
    /// backup seat gives it [`BurnRule::Toward`] while a stop holds the hull, and this
    /// otherwise.
    pub const ABSENT: Seat = Seat {
        fire: FireRule::Never,
        burn: BurnRule::Never,
        into_reserve: true,
        quietest: EmconPosture::Standard,
    };
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
    /// Intents the seat dropped or that mean nothing in a bubble.
    pub dropped: u32,
}

/// The families of mount a `fire` can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mount {
    Laser,
    Railgun,
    Tube,
}

fn mount_of(ship: &SpaceShipState, weapon: &str) -> Option<Mount> {
    if ship.lasers.iter().any(|mount| mount.id == weapon) {
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
            IntentKind::Lock { contact } if seat.fire == FireRule::Free => {
                match contacts.get(contact) {
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
            IntentKind::Fire { weapon, .. } if seat.fire == FireRule::Free => {
                match bubble
                    .ships
                    .get(ship_id)
                    .and_then(|ship| mount_of(ship, weapon))
                {
                    Some(mount) => shots.push(mount),
                    None => applied.dropped += 1,
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
    if bubble
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
        let target_id = target.clone();
        match mount {
            Mount::Laser => command(
                bubble,
                BubbleInputCommand::FireOffensiveLasers {
                    shooter_id,
                    target_id,
                },
            ),
            Mount::Railgun => {
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
        }
    }
    applied
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
}
