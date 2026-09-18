//! **The arena's inputs** (16b cut A.1; doc 16 §3.1's `Intent` and `Control`
//! messages and pulse phases 1, 2 and 4; doc 09 C8).
//!
//! Two messages come in from the world half and nothing else does.
//!
//! **[`Control`]** raises the epoch table and says what the seat is: `Live`,
//! `Linkdead` or `Released`. The world sends one on every seat activation,
//! deactivation, lease revocation, linkdead onset and revival, `EXIT` and
//! `SIM ABORT`. [`apply_control`] raises `(unit, channel)` and then **purges
//! every queued intent of that unit and channel below the new epoch**, which is
//! doc 16 §3.1's *a pilot who types `FIRE` then `EXIT` in one burst never fires*.
//!
//! **[`Intent`]** is one order. It is validated at the world when it is enqueued
//! — focus, lease, channel, epoch — and **authorised again here, at apply**,
//! against the arena's own table. The second check is the one that matters: the
//! world's answer was true when the order was typed, and an order sits in a queue.
//!
//! # Continuous and discrete
//!
//! doc 16 §3.1 phase 2 splits orders in two, and the split is the whole reason a
//! laggy pilot is not a dangerous one:
//!
//! - **Continuous** orders ([`IntentKind::Heading`], [`IntentKind::Speed`]) set a
//!   *desired value* and are **latest-wins**. They do not queue, so a pilot who
//!   spins the wheel does not fill anything up, and a burst of ten headings is
//!   the last heading.
//! - **Discrete** actions ([`IntentKind::Startup`], [`IntentKind::Lock`],
//!   [`IntentKind::Fire`]) are queued per unit, bounded at
//!   `intents.max_pending_per_unit`, and resolved in phase 4 by the cut that owns
//!   them: `STARTUP` by [`resolve_startup`] here, and `LOCK` and `FIRE` by
//!   [`crate::arena::state::Arena::resolve_lock`] and
//!   [`crate::arena::state::Arena::resolve_fire`] (16b cut W.1).
//!
//! A refusal is always one of the named [`Refusal`] words, never a silent drop:
//! cut K's cockpit turns each into a line a pilot can act on.

use mush_core::ObjId;
use smol_str::SmolStr;

use crate::arena::ledger::EventKind;
use crate::arena::state::{Arena, Channel, ContactId, DueKind, Power};

/// What a seat is (doc 16 §3.1's `Control`; doc 07 §1.2 for linkdead).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControlState {
    /// Seated and connected.
    Live,
    /// Seated and disconnected. **The tank keeps its continuous orders** — a tank
    /// ordered forward keeps rolling, which is the honest answer and the
    /// dangerous one — and accepts no discrete action, so it fires nothing.
    Linkdead,
    /// The seat is gone: `EXIT`, a revoked lease, a deactivated station. The tank
    /// idles.
    Released,
}

impl ControlState {
    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ControlState::Live => "live",
            ControlState::Linkdead => "linkdead",
            ControlState::Released => "released",
        }
    }
}

/// **The authority message** (doc 16 §3.1): this `(unit, channel)` is now at
/// `epoch`, and the seat is in `state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    /// The unit the channel is over.
    pub unit: ObjId,
    /// The channel.
    pub channel: Channel,
    /// The new epoch. Never lowers the table.
    pub epoch: u64,
    /// What the seat is.
    pub state: ControlState,
}

/// **One order** (doc 16 §3.1's `Intent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntentKind {
    /// Steer to a bearing, millidegrees clockwise from north.
    Heading {
        /// The ordered bearing; normalised modulo 360,000 on apply.
        mdeg: u32,
    },
    /// Make way at a speed in cm/s; **negative is reverse**.
    Speed {
        /// The ordered speed; clamped to the unit's own ceilings on apply.
        cms: i32,
    },
    /// Bring power up (doc 16 §3.3 "Startup").
    Startup,
    /// Lock a contact the platform holds (16b cut W.1 resolves it).
    Lock {
        /// The contact's opaque arena id.
        contact: ContactId,
    },
    /// Fire a weapon (16b cut W.1 resolves it).
    Fire {
        /// The weapon's component id.
        weapon: SmolStr,
    },
}

impl IntentKind {
    /// Whether this order is a *desired value* rather than an action: continuous
    /// orders are latest-wins and never queue.
    #[must_use]
    pub const fn is_continuous(&self) -> bool {
        matches!(self, IntentKind::Heading { .. } | IntentKind::Speed { .. })
    }

    /// The channel this order must arrive on. An order on the wrong channel is
    /// refused [`Refusal::Channel`] whatever the epoch table says — a gunner's
    /// grant is not a driver's (the F3 rule).
    #[must_use]
    pub const fn channel_required(&self) -> Channel {
        match self {
            IntentKind::Heading { .. } | IntentKind::Speed { .. } | IntentKind::Startup => {
                Channel::Helm
            }
            IntentKind::Lock { .. } => Channel::Sensors,
            IntentKind::Fire { .. } => Channel::Fire,
        }
    }

    /// The word a refusal or a ledger line names this order by.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            IntentKind::Heading { .. } => "heading",
            IntentKind::Speed { .. } => "speed",
            IntentKind::Startup => "startup",
            IntentKind::Lock { .. } => "lock",
            IntentKind::Fire { .. } => "fire",
        }
    }
}

/// One order with its authority and its place in the pilot's burst.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    /// The unit ordered.
    pub unit: ObjId,
    /// The channel it arrived on.
    pub channel: Channel,
    /// The epoch the world validated it under.
    pub epoch: u64,
    /// What was ordered.
    pub kind: IntentKind,
    /// The world's monotonic sequence within the burst, so a queue's order is the
    /// pilot's order and not the inbox's.
    pub seq: u64,
}

/// What [`apply_intent`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentOutcome {
    /// A continuous order took effect immediately, replacing the previous one.
    AppliedContinuous,
    /// A discrete action joined the unit's queue for phase 4.
    Queued,
    /// Nothing happened, for this named reason.
    Refused(Refusal),
}

/// **Why an order was refused** — one named word each, so the cockpit never says
/// "no".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum Refusal {
    /// The order's epoch is not the channel's current epoch: the seat changed
    /// underneath it.
    #[error("that order is from a seat that has since changed hands")]
    Epoch,
    /// The order arrived on a channel that does not carry it.
    #[error("that order does not travel on that channel")]
    Channel,
    /// Power is not online (doc 16 §3.3 "Startup").
    #[error("power is not online")]
    PowerNotOnline,
    /// The unit already has `intents.max_pending_per_unit` actions queued.
    #[error("that unit already has as many actions queued as it may")]
    QueueFull,
    /// The unit is a wreck (doc 16 §3.3): no longer the subject of any order.
    #[error("that unit is a wreck")]
    Wreck,
    /// Motion is disabled (a destroyed engine).
    #[error("that unit's motion is disabled")]
    MotionDisabled,
    /// The seat is linkdead: continuous orders stand, discrete ones refuse.
    #[error("that seat has gone linkdead")]
    Linkdead,
    /// No such unit in this arena.
    #[error("this arena has no such unit")]
    Unknown,
    /// **`LOCK` named a contact the platform does not hold** (16b cut W.1; doc 16
    /// §3.6). The *pilot's* sentence for this is
    /// [`crate::arena::state::NO_SUCH_CONTACT`], one answer for all three cases;
    /// this is the word the arena's own refusal path uses.
    #[error("that platform holds no such contact")]
    NoContact,
    /// **`FIRE` with no lock, or with a lock the platform no longer holds** (16b
    /// cut W.1; doc 16 §3.5 "Acceptance and cost").
    #[error("that unit has no lock")]
    NoLock,
    /// **`FIRE` on a weapon whose cycle has not finished** (16b cut W.1).
    #[error("that weapon is still cycling")]
    Cycling,
    /// **`FIRE` on a weapon with no rounds left** (16b cut W.1).
    #[error("that weapon is empty")]
    Empty,
    /// **`FIRE` naming a weapon the unit does not carry** (16b cut W.1).
    #[error("that unit carries no such weapon")]
    NoWeapon,
}

impl Refusal {
    /// The word the wire and the ledger use.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Refusal::Epoch => "epoch",
            Refusal::Channel => "channel",
            Refusal::PowerNotOnline => "power_not_online",
            Refusal::QueueFull => "queue_full",
            Refusal::Wreck => "wreck",
            Refusal::MotionDisabled => "motion_disabled",
            Refusal::Linkdead => "linkdead",
            Refusal::Unknown => "unknown",
            Refusal::NoContact => "no_contact",
            Refusal::NoLock => "no_lock",
            Refusal::Cycling => "cycling",
            Refusal::Empty => "empty",
            Refusal::NoWeapon => "no_weapon",
        }
    }
}

/// **Apply a `Control`** (doc 16 §3.1 pulse phase 1).
///
/// In order: raise `(unit, channel)` in the epoch table — which never lowers, so
/// a `Control` that arrives late or twice cannot reopen a closed epoch — then
/// purge every queued intent of that unit **on that channel** whose epoch is
/// below the table's new current value, then apply the seat state:
///
/// - `Linkdead` marks the unit linkdead. Continuous orders already given stand.
/// - `Live` clears it.
/// - `Released` clears it **and zeroes the desired speed**, so a released tank
///   coasts to a stop rather than driving away with a dead man's order. Its
///   heading order is left alone: there is nothing dangerous about a heading, and
///   zeroing it would spin the turret-less hull toward north for no reason.
///
/// A `Control` for a unit this arena does not hold raises the table and nothing
/// else: the table is keyed on `(unit, channel)` and an epoch for an absent unit
/// is inert, which is cheaper than deciding whether the world is confused.
pub fn apply_control(arena: &mut Arena, c: &Control) {
    arena.epochs.raise(c.unit, c.channel, c.epoch);
    let current = arena.epochs.current(c.unit, c.channel);
    if let Some(queue) = arena.pending.get_mut(&c.unit) {
        queue.retain(|queued| !(queued.channel == c.channel && queued.epoch < current));
        if queue.is_empty() {
            arena.pending.remove(&c.unit);
        }
    }
    let Some(unit) = arena.unit_mut(c.unit) else {
        return;
    };
    match c.state {
        ControlState::Linkdead => unit.linkdead = true,
        ControlState::Live => unit.linkdead = false,
        ControlState::Released => {
            unit.linkdead = false;
            unit.desired_speed_cms = 0;
        }
    }
}

/// **Apply an `Intent`** (doc 16 §3.1 pulse phase 2).
///
/// The gates, in this order, because each one makes the next meaningful:
///
/// 1. the unit exists — [`Refusal::Unknown`];
/// 2. the epoch is the channel's **current** epoch — [`Refusal::Epoch`];
/// 3. the channel carries this order — [`Refusal::Channel`];
/// 4. the unit is not a wreck — [`Refusal::Wreck`].
///
/// Then, for a **continuous** order: power must be online
/// ([`Refusal::PowerNotOnline`], doc 16 §3.3) and motion must not be disabled
/// ([`Refusal::MotionDisabled`]); the desired value is set, latest-wins. A
/// heading is normalised modulo 360,000. A speed is clamped to
/// `[-max_speed_cms × reverse_factor_pct / 100, max_speed_cms]` — the *order* is
/// clamped to what the unit could ever do, while what it actually reaches each
/// substep is motion's business (doc 16 §3.4 step 2, terrain and slow
/// consequences).
///
/// For a **discrete** action: the seat must not be linkdead
/// ([`Refusal::Linkdead`]) and the unit's queue must have room
/// ([`Refusal::QueueFull`]); the action is pushed to the back of that queue for
/// phase 4. Power is deliberately **not** checked here: `STARTUP` is the order
/// that turns power on, and `LOCK` and `FIRE` become legal in phase 4, which is
/// where doc 16 §3.1 puts *resolve newly queued discrete actions that became
/// legal this pulse*.
pub fn apply_intent(arena: &mut Arena, i: &Intent) -> IntentOutcome {
    if arena.unit(i.unit).is_none() {
        return IntentOutcome::Refused(Refusal::Unknown);
    }
    if !arena.epochs.authorises(i.unit, i.channel, i.epoch) {
        return IntentOutcome::Refused(Refusal::Epoch);
    }
    if i.kind.channel_required() != i.channel {
        return IntentOutcome::Refused(Refusal::Channel);
    }
    let max_pending = arena.arena_cfg.intents.max_pending_per_unit;
    let Some(unit) = arena.unit_mut(i.unit) else {
        return IntentOutcome::Refused(Refusal::Unknown);
    };
    if unit.limits.wreck {
        return IntentOutcome::Refused(Refusal::Wreck);
    }
    if i.kind.is_continuous() {
        if unit.power != Power::Online {
            return IntentOutcome::Refused(Refusal::PowerNotOnline);
        }
        if unit.limits.motion_disabled {
            return IntentOutcome::Refused(Refusal::MotionDisabled);
        }
        match &i.kind {
            IntentKind::Heading { mdeg } => unit.desired_heading_mdeg = mdeg % 360_000,
            IntentKind::Speed { cms } => {
                let forward = i32::try_from(unit.mobility.max_speed_cms).unwrap_or(i32::MAX);
                let reverse = i32::try_from(unit.mobility.reverse_ceiling_cms())
                    .unwrap_or(i32::MAX)
                    .saturating_neg();
                unit.desired_speed_cms = (*cms).clamp(reverse, forward);
            }
            IntentKind::Startup | IntentKind::Lock { .. } | IntentKind::Fire { .. } => {}
        }
        return IntentOutcome::AppliedContinuous;
    }
    if unit.linkdead {
        return IntentOutcome::Refused(Refusal::Linkdead);
    }
    let queue = arena.pending.entry(i.unit).or_default();
    if u32::try_from(queue.len()).unwrap_or(u32::MAX) >= max_pending {
        return IntentOutcome::Refused(Refusal::QueueFull);
    }
    queue.push_back(i.clone());
    IntentOutcome::Queued
}

/// **Take a unit's queued discrete actions**, in the order they were queued — cut
/// A.2's phase 4 drains each unit in unit-id order and resolves what it finds.
///
/// Draining is total: an action phase 4 cannot resolve this pulse is re-queued by
/// its owner or refused by it, never left to rot in a queue whose bound another
/// order needs.
pub fn drain_pending(arena: &mut Arena, unit: ObjId) -> Vec<Intent> {
    arena
        .pending
        .remove(&unit)
        .map(|queue| queue.into_iter().collect())
        .unwrap_or_default()
}

/// **Resolve `STARTUP`** (doc 16 §3.3 "Startup"): `Off` becomes `Starting` and
/// `StartupComplete` is scheduled at `now + match.startup_ms`, with one `startup`
/// ledger event.
///
/// **Idempotent**: a unit already `Starting` or `Online` is a no-op answering
/// `Ok(())`, so a pilot mashing the key schedules one completion and not five.
///
/// The ledger event carries the state hash as it was *before* the transition,
/// which is the same convention cut A.2 uses for a substep: an event says what
/// the world looked like when it happened. **16b cut W.1** routed it through
/// [`Arena::event_hash`], so every action phase 4 resolves shares the one hash the
/// phase took — which is that convention read once more, and O(1) hashes a pulse
/// instead of one per action.
///
/// # Errors
///
/// [`Refusal::Unknown`] for a unit this arena does not hold, [`Refusal::Wreck`]
/// for a wreck. A ledger that has reached its byte bound does **not** fail the
/// transition: the black box is a record, not an authority (see
/// [`crate::arena::ledger::LedgerFull`]).
pub fn resolve_startup(arena: &mut Arena, unit: ObjId) -> Result<(), Refusal> {
    let Some(state) = arena.unit(unit) else {
        return Err(Refusal::Unknown);
    };
    if state.limits.wreck {
        return Err(Refusal::Wreck);
    }
    if state.power != Power::Off {
        return Ok(());
    }
    let at = arena
        .now_ms
        .saturating_add(i64::from(arena.match_cfg.startup_ms));
    let hash = arena.event_hash();
    if let Some(state) = arena.unit_mut(unit) {
        state.power = Power::Starting;
    }
    arena.due.schedule(at, DueKind::StartupComplete { unit });
    let _ = arena
        .ledger
        .append(arena.now_ms, EventKind::Startup { unit }, None, hash);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::{DueEvent, Power, Team};
    use crate::arena::test_support::{arena_at_epoch, pose, tank};
    use std::collections::VecDeque;

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);

    fn two_tanks(epoch: u64) -> Arena {
        arena_at_epoch(
            vec![
                tank(RED, Team::Red, pose(1_500, 1_500)),
                tank(BLUE, Team::Blue, pose(6_500, 6_500)),
            ],
            epoch,
        )
    }

    fn online(arena: &mut Arena, unit: ObjId) {
        if let Some(state) = arena.unit_mut(unit) {
            state.power = Power::Online;
        }
    }

    fn intent(unit: ObjId, epoch: u64, seq: u64, kind: IntentKind) -> Intent {
        Intent {
            unit,
            channel: kind.channel_required(),
            epoch,
            kind,
            seq,
        }
    }

    #[test]
    fn a_kind_knows_whether_it_is_continuous_and_which_channel_carries_it() {
        assert!(IntentKind::Heading { mdeg: 0 }.is_continuous());
        assert!(IntentKind::Speed { cms: 0 }.is_continuous());
        assert!(!IntentKind::Startup.is_continuous());
        assert!(
            !IntentKind::Lock {
                contact: ContactId(1)
            }
            .is_continuous()
        );
        assert!(
            !IntentKind::Fire {
                weapon: SmolStr::new("cannon")
            }
            .is_continuous()
        );
        assert_eq!(
            IntentKind::Heading { mdeg: 0 }.channel_required(),
            Channel::Helm
        );
        assert_eq!(
            IntentKind::Speed { cms: 0 }.channel_required(),
            Channel::Helm
        );
        assert_eq!(IntentKind::Startup.channel_required(), Channel::Helm);
        assert_eq!(
            IntentKind::Lock {
                contact: ContactId(1)
            }
            .channel_required(),
            Channel::Sensors
        );
        assert_eq!(
            IntentKind::Fire {
                weapon: SmolStr::new("cannon")
            }
            .channel_required(),
            Channel::Fire
        );
    }

    #[test]
    fn continuous_orders_are_latest_wins_and_normalised() {
        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        for (n, mdeg) in [10_000_u32, 90_000, 359_999, 360_000, 725_000]
            .into_iter()
            .enumerate()
        {
            let seq = u64::try_from(n).unwrap_or(0);
            assert_eq!(
                apply_intent(
                    &mut arena,
                    &intent(RED, 1, seq, IntentKind::Heading { mdeg })
                ),
                IntentOutcome::AppliedContinuous
            );
        }
        // The last order wins, normalised into 0..360_000.
        assert_eq!(
            arena.unit(RED).map(|state| state.desired_heading_mdeg),
            Some(5_000)
        );
        // And nothing queued: a spun wheel does not fill the inbox.
        assert!(arena.pending.is_empty());
    }

    #[test]
    fn a_speed_order_is_clamped_to_the_units_own_ceilings() {
        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        let speed = |arena: &Arena| arena.unit(RED).map(|state| state.desired_speed_cms);
        // The stock tank is 1,000 cm/s forward and 40% of that in reverse.
        apply_intent(
            &mut arena,
            &intent(RED, 1, 0, IntentKind::Speed { cms: 400 }),
        );
        assert_eq!(speed(&arena), Some(400));
        apply_intent(
            &mut arena,
            &intent(RED, 1, 1, IntentKind::Speed { cms: 9_999 }),
        );
        assert_eq!(speed(&arena), Some(1_000));
        apply_intent(
            &mut arena,
            &intent(RED, 1, 2, IntentKind::Speed { cms: -100 }),
        );
        assert_eq!(speed(&arena), Some(-100));
        apply_intent(
            &mut arena,
            &intent(RED, 1, 3, IntentKind::Speed { cms: -9_999 }),
        );
        assert_eq!(speed(&arena), Some(-400));
        apply_intent(
            &mut arena,
            &intent(RED, 1, 4, IntentKind::Speed { cms: i32::MIN }),
        );
        assert_eq!(speed(&arena), Some(-400));
        apply_intent(&mut arena, &intent(RED, 1, 5, IntentKind::Speed { cms: 0 }));
        assert_eq!(speed(&arena), Some(0));
    }

    #[test]
    fn a_continuous_order_refuses_before_power_is_online() {
        let mut arena = two_tanks(1);
        assert_eq!(arena.unit(RED).map(|state| state.power), Some(Power::Off));
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 0, IntentKind::Heading { mdeg: 90_000 })
            ),
            IntentOutcome::Refused(Refusal::PowerNotOnline)
        );
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 1, IntentKind::Speed { cms: 500 })
            ),
            IntentOutcome::Refused(Refusal::PowerNotOnline)
        );
        // Starting is not online either.
        if let Some(state) = arena.unit_mut(RED) {
            state.power = Power::Starting;
        }
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 2, IntentKind::Speed { cms: 500 })
            ),
            IntentOutcome::Refused(Refusal::PowerNotOnline)
        );
        // Nothing was ordered.
        assert_eq!(
            arena.unit(RED).map(|state| state.desired_speed_cms),
            Some(0)
        );
        assert_eq!(
            arena.unit(RED).map(|state| state.desired_heading_mdeg),
            Some(0)
        );
        // Online, and the same order lands.
        online(&mut arena, RED);
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 3, IntentKind::Speed { cms: 500 })
            ),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(
            arena.unit(RED).map(|state| state.desired_speed_cms),
            Some(500)
        );
    }

    #[test]
    fn a_continuous_order_refuses_a_wreck_and_disabled_motion() {
        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        if let Some(state) = arena.unit_mut(RED) {
            state.limits.motion_disabled = true;
        }
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 0, IntentKind::Speed { cms: 500 })
            ),
            IntentOutcome::Refused(Refusal::MotionDisabled)
        );
        // A wreck refuses every order, continuous or not, before anything else.
        if let Some(state) = arena.unit_mut(RED) {
            state.limits.wreck = true;
        }
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 1, IntentKind::Speed { cms: 500 })
            ),
            IntentOutcome::Refused(Refusal::Wreck)
        );
        assert_eq!(
            apply_intent(&mut arena, &intent(RED, 1, 2, IntentKind::Startup)),
            IntentOutcome::Refused(Refusal::Wreck)
        );
    }

    #[test]
    fn an_order_on_the_wrong_channel_or_at_the_wrong_epoch_refuses() {
        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        // A stale epoch, and a future one.
        assert_eq!(
            apply_intent(
                &mut arena,
                &Intent {
                    unit: RED,
                    channel: Channel::Helm,
                    epoch: 0,
                    kind: IntentKind::Speed { cms: 100 },
                    seq: 0,
                }
            ),
            IntentOutcome::Refused(Refusal::Epoch)
        );
        assert_eq!(
            apply_intent(
                &mut arena,
                &Intent {
                    unit: RED,
                    channel: Channel::Helm,
                    epoch: 2,
                    kind: IntentKind::Speed { cms: 100 },
                    seq: 1,
                }
            ),
            IntentOutcome::Refused(Refusal::Epoch)
        );
        // The right epoch on the wrong channel: a gunner's grant is not a
        // driver's.
        assert_eq!(
            apply_intent(
                &mut arena,
                &Intent {
                    unit: RED,
                    channel: Channel::Fire,
                    epoch: 1,
                    kind: IntentKind::Speed { cms: 100 },
                    seq: 2,
                }
            ),
            IntentOutcome::Refused(Refusal::Channel)
        );
        // And an unknown unit, before either.
        assert_eq!(
            apply_intent(&mut arena, &intent(ObjId(77), 1, 3, IntentKind::Startup)),
            IntentOutcome::Refused(Refusal::Unknown)
        );
        assert_eq!(
            arena.unit(RED).map(|state| state.desired_speed_cms),
            Some(0)
        );
    }

    #[test]
    fn discrete_actions_queue_in_order_and_the_bound_refuses() {
        let mut arena = two_tanks(1);
        let bound = arena.arena_cfg.intents.max_pending_per_unit;
        assert_eq!(bound, 8);
        for seq in 0..u64::from(bound) {
            assert_eq!(
                apply_intent(
                    &mut arena,
                    &intent(
                        RED,
                        1,
                        seq,
                        IntentKind::Fire {
                            weapon: SmolStr::new("cannon")
                        }
                    )
                ),
                IntentOutcome::Queued,
                "action {seq} of the bound"
            );
        }
        assert_eq!(
            apply_intent(&mut arena, &intent(RED, 1, 99, IntentKind::Startup)),
            IntentOutcome::Refused(Refusal::QueueFull)
        );
        // The other unit's queue is its own.
        assert_eq!(
            apply_intent(&mut arena, &intent(BLUE, 1, 0, IntentKind::Startup)),
            IntentOutcome::Queued
        );
        // Draining answers the pilot's order and empties the queue.
        let drained = drain_pending(&mut arena, RED);
        assert_eq!(drained.len(), usize::try_from(bound).unwrap_or(0));
        assert_eq!(
            drained.iter().map(|order| order.seq).collect::<Vec<_>>(),
            (0..u64::from(bound)).collect::<Vec<u64>>()
        );
        assert!(drain_pending(&mut arena, RED).is_empty());
        assert_eq!(drain_pending(&mut arena, BLUE).len(), 1);
    }

    #[test]
    fn a_linkdead_seat_keeps_its_continuous_orders_and_fires_nothing() {
        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        apply_intent(
            &mut arena,
            &intent(RED, 1, 0, IntentKind::Speed { cms: 700 }),
        );
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 1,
                state: ControlState::Linkdead,
            },
        );
        assert_eq!(arena.unit(RED).map(|state| state.linkdead), Some(true));
        // The order stands: the tank keeps rolling.
        assert_eq!(
            arena.unit(RED).map(|state| state.desired_speed_cms),
            Some(700)
        );
        // And every discrete action refuses.
        assert_eq!(
            apply_intent(&mut arena, &intent(RED, 1, 1, IntentKind::Startup)),
            IntentOutcome::Refused(Refusal::Linkdead)
        );
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(
                    RED,
                    1,
                    2,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon")
                    }
                )
            ),
            IntentOutcome::Refused(Refusal::Linkdead)
        );
        assert!(arena.pending.is_empty());
        // A revival at the same epoch clears it.
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 1,
                state: ControlState::Live,
            },
        );
        assert_eq!(arena.unit(RED).map(|state| state.linkdead), Some(false));
        assert_eq!(
            apply_intent(&mut arena, &intent(RED, 1, 3, IntentKind::Startup)),
            IntentOutcome::Queued
        );
    }

    #[test]
    fn a_released_seat_idles_the_tank() {
        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        apply_intent(
            &mut arena,
            &intent(RED, 1, 0, IntentKind::Heading { mdeg: 90_000 }),
        );
        apply_intent(
            &mut arena,
            &intent(RED, 1, 1, IntentKind::Speed { cms: 700 }),
        );
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 2,
                state: ControlState::Released,
            },
        );
        assert_eq!(
            arena.unit(RED).map(|state| state.desired_speed_cms),
            Some(0)
        );
        // The heading order is left alone; there is nothing dangerous in a
        // bearing.
        assert_eq!(
            arena.unit(RED).map(|state| state.desired_heading_mdeg),
            Some(90_000)
        );
    }

    #[test]
    fn a_control_purges_every_queued_order_below_the_new_epoch() {
        // Doc 16 §3.1's burst: STARTUP queued at epoch 1, then the seat is
        // released at epoch 2. The queue is empty and the epoch-1 order that
        // follows is refused.
        let mut arena = two_tanks(1);
        assert_eq!(
            apply_intent(&mut arena, &intent(RED, 1, 0, IntentKind::Startup)),
            IntentOutcome::Queued
        );
        assert_eq!(arena.pending.get(&RED).map(VecDeque::len), Some(1));
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 2,
                state: ControlState::Released,
            },
        );
        assert!(arena.pending.is_empty(), "the queued STARTUP is gone");
        assert_eq!(arena.epochs.current(RED, Channel::Helm), 2);
        assert_eq!(
            apply_intent(&mut arena, &intent(RED, 1, 1, IntentKind::Startup)),
            IntentOutcome::Refused(Refusal::Epoch)
        );
        // At the new epoch it queues again.
        assert_eq!(
            apply_intent(&mut arena, &intent(RED, 2, 2, IntentKind::Startup)),
            IntentOutcome::Queued
        );
    }

    #[test]
    fn a_purge_is_per_channel_and_a_stale_control_purges_nothing() {
        let mut arena = two_tanks(1);
        apply_intent(&mut arena, &intent(RED, 1, 0, IntentKind::Startup));
        apply_intent(
            &mut arena,
            &intent(
                RED,
                1,
                1,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                },
            ),
        );
        assert_eq!(arena.pending.get(&RED).map(VecDeque::len), Some(2));
        // Raising `sensors` touches neither queued order.
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Sensors,
                epoch: 9,
                state: ControlState::Live,
            },
        );
        assert_eq!(arena.pending.get(&RED).map(VecDeque::len), Some(2));
        // Raising `fire` takes the FIRE and leaves the STARTUP.
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Fire,
                epoch: 2,
                state: ControlState::Live,
            },
        );
        assert_eq!(
            arena.pending.get(&RED).map(|queue| queue
                .iter()
                .map(|order| order.kind.as_str())
                .collect::<Vec<_>>()),
            Some(vec!["startup"])
        );
        // A stale `Control` neither lowers the table nor purges the order it
        // would have been below.
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 0,
                state: ControlState::Live,
            },
        );
        assert_eq!(arena.epochs.current(RED, Channel::Helm), 1);
        assert_eq!(arena.pending.get(&RED).map(VecDeque::len), Some(1));
    }

    #[test]
    fn a_control_for_a_unit_this_arena_does_not_hold_is_inert() {
        let mut arena = two_tanks(1);
        apply_control(
            &mut arena,
            &Control {
                unit: ObjId(77),
                channel: Channel::Helm,
                epoch: 5,
                state: ControlState::Released,
            },
        );
        assert_eq!(arena.epochs.current(ObjId(77), Channel::Helm), 5);
        assert_eq!(arena.units.len(), 2);
    }

    #[test]
    fn startup_schedules_exactly_one_completion_and_is_idempotent() {
        let mut arena = two_tanks(1);
        let before = arena.due.len();
        let ledger_before = arena.ledger.seq();
        assert_eq!(resolve_startup(&mut arena, RED), Ok(()));
        assert_eq!(
            arena.unit(RED).map(|state| state.power),
            Some(Power::Starting)
        );
        assert_eq!(arena.due.len(), before + 1);
        assert_eq!(arena.ledger.seq(), ledger_before + 1);
        let scheduled: Vec<&DueEvent> = arena
            .due
            .iter()
            .filter(|event| event.kind == DueKind::StartupComplete { unit: RED })
            .collect();
        assert_eq!(scheduled.len(), 1);
        // `match.startup_ms` after now, which is zero at the start of a match.
        assert_eq!(scheduled.first().map(|event| event.t_ms), Some(10_000));

        // Mashing the key changes nothing.
        for _ in 0..4 {
            assert_eq!(resolve_startup(&mut arena, RED), Ok(()));
        }
        assert_eq!(arena.due.len(), before + 1);
        assert_eq!(arena.ledger.seq(), ledger_before + 1);

        // Already online is a no-op too.
        online(&mut arena, RED);
        assert_eq!(resolve_startup(&mut arena, RED), Ok(()));
        assert_eq!(
            arena.unit(RED).map(|state| state.power),
            Some(Power::Online)
        );
        assert_eq!(arena.due.len(), before + 1);
    }

    #[test]
    fn startup_is_scheduled_from_the_current_arena_time() {
        let mut arena = two_tanks(1);
        arena.now_ms = 42_000;
        assert_eq!(resolve_startup(&mut arena, BLUE), Ok(()));
        assert_eq!(
            arena
                .due
                .iter()
                .find(|event| event.kind == DueKind::StartupComplete { unit: BLUE })
                .map(|event| event.t_ms),
            Some(52_000)
        );
    }

    #[test]
    fn startup_refuses_an_unknown_unit_and_a_wreck() {
        let mut arena = two_tanks(1);
        assert_eq!(
            resolve_startup(&mut arena, ObjId(77)),
            Err(Refusal::Unknown)
        );
        if let Some(state) = arena.unit_mut(RED) {
            state.limits.wreck = true;
        }
        assert_eq!(resolve_startup(&mut arena, RED), Err(Refusal::Wreck));
        assert_eq!(arena.unit(RED).map(|state| state.power), Some(Power::Off));
    }

    #[test]
    fn every_refusal_has_its_own_word() {
        let words = [
            Refusal::Epoch,
            Refusal::Channel,
            Refusal::PowerNotOnline,
            Refusal::QueueFull,
            Refusal::Wreck,
            Refusal::MotionDisabled,
            Refusal::Linkdead,
            Refusal::Unknown,
        ]
        .map(|refusal| refusal.as_str());
        let unique: std::collections::BTreeSet<&str> = words.iter().copied().collect();
        assert_eq!(unique.len(), words.len());
        assert!(words.iter().all(|word| !word.is_empty()));
        assert_eq!(ControlState::Linkdead.as_str(), "linkdead");
    }
}
