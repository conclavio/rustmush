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
//!   [`IntentKind::Fire`]) are queued per unit — a seat may give a unit at most
//!   `intents.max_pending_per_unit` of them per arena second (16b cut M.1,
//!   `T-82`; [`apply_intent`]) — and resolved in phase 4 by the cut that owns
//!   them: `STARTUP` by [`resolve_startup`] here, and `LOCK` and `FIRE` by
//!   [`crate::arena::state::Arena::resolve_lock`] and
//!   [`crate::arena::state::Arena::resolve_fire`] (16b cut W.1).
//! - **[`IntentKind::Order`]** (16b cut N.2) is a third thing and says so: a
//!   squad's standing instruction, latest-wins like a continuous order and
//!   affecting nothing on the hull. It is written onto the unit's reflex machine
//!   and read by that machine's next tick, so an order changes *what the
//!   controller decides* and never what the physics does.
//!
//! - **[`IntentKind::Place`]** (16b cut I.3) is a staff act that rides the same
//!   path: discrete, resolved in phase 4, and exempt from the three gates that
//!   exist for a pilot's orders (the wreck, `Linkdead` and the seat's rate).
//!
//! A refusal is always one of the named [`Refusal`] words, never a silent drop:
//! cut K's cockpit turns each into a line a pilot can act on (a place's is a
//! `place_refused` line on the tape instead, and is never said).

use mush_core::ObjId;
use smol_str::SmolStr;

use crate::arena::ledger::EventKind;
use crate::arena::reflex::ReflexState;
use crate::arena::state::{Arena, Channel, ContactId, Controller, DueKind, Power};

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
    /// **Whose seat it is** (16b cut F.1), when the world knows.
    ///
    /// The epoch table is keyed on `(unit, channel)` and needs no pilot; the
    /// **frame roster** does. A doc 16 §3.7 cockpit carries all three channels,
    /// and two pilots may sit in one tank (`Arena::frames` says so), so a
    /// `Released` that dropped *the unit's* pilots would silence a co-pilot who
    /// never left. This names the one who did, and `None` is a control the world
    /// cannot attribute — a lease revocation, a fixture — which changes no
    /// roster.
    pub pilot: Option<ObjId>,
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
    /// **A squad's standing order** (16b cut N.2; doc 16 §6 "Deliberation"),
    /// carried in one per unit of the squad.
    ///
    /// It is neither continuous nor discrete in the sense the other five are: it
    /// sets nothing on the hull and queues nothing for phase 4. It is *doctrine*,
    /// written onto the unit's reflex machine and read by that machine's next tick
    /// — which is what keeps an order rate-independent, exactly as every other
    /// reflex decision is (the module head of [`crate::arena::reflex`] says why).
    ///
    /// **Only a reflex unit takes one** ([`Refusal::NotReflex`]): a pilot's tank
    /// has no machine to steer, and an order that silently did nothing to a seated
    /// tank would be a commander who thinks they have twelve tanks and has six.
    /// It travels on [`Channel::Helm`] because it is, in the end, a route and a
    /// posture.
    Order(Box<crate::deliberation::order::Order>),
    /// **Hand a squad back to doctrine** (16b cut Z.2; 16c Q44): drops the
    /// unit's standing order when it was written for `squad`, through the one
    /// helper the expiry block uses
    /// ([`crate::arena::reflex::ReflexMemory::drop_to_doctrine`]). Continuous,
    /// [`Channel::Helm`], word `release`; shares `Order`'s buffer key in the
    /// arena task's pre-pulse buffer, so of an `ORDER` and a `RELEASE` in one
    /// pulse the later `seq` is what applies.
    ///
    /// Like [`IntentKind::Order`] it is taken before the continuous gates (a
    /// release to a cold or engine-killed machine lands, as an order to one
    /// does), only a reflex unit takes one ([`Refusal::NotReflex`]), and its
    /// refusals are never staged to a pilot.
    ReleaseOrder {
        /// The squad whose order is released.
        squad: crate::deliberation::SquadId,
        /// The tank whose pilot released it, `None` for a release nobody seated
        /// gave.
        issuer: Option<ObjId>,
    },
    /// **Put the unit on a cell, for staff** (16b cut I.3; 16c N10:
    /// `@arena/place`). Discrete, [`Channel::Helm`], word `place`; stamped as
    /// the unit's own seat exactly as `@arena/intent` is, and resolved in pulse
    /// phase 4 by `Arena::resolve_place`, which re-runs the cell entry test,
    /// refuses a unit with a round in flight to or from it, and journals every
    /// outcome (`placed` or `place_refused`) while staging nothing.
    ///
    /// **It skips three gates a pilot's discrete order meets** in
    /// [`apply_intent`] — the wreck, `Linkdead` and the seat's rate — and the
    /// arena task's per-unit pre-pulse bound never refuses it: a wreck blocking
    /// the ford and a linkdead pilot's wedged tank are the stuck units staff most
    /// want to move, and a staff act must not spend one of the pilot's eight a
    /// second, whose refused eighth would be a tell.
    Place {
        /// The column.
        x: u16,
        /// The row.
        y: u16,
    },
}

impl IntentKind {
    /// Whether this order is a *desired value* rather than an action: continuous
    /// orders are latest-wins and never queue.
    #[must_use]
    pub const fn is_continuous(&self) -> bool {
        matches!(
            self,
            IntentKind::Heading { .. }
                | IntentKind::Speed { .. }
                | IntentKind::Order(_)
                | IntentKind::ReleaseOrder { .. }
        )
    }

    /// **Whether this order is an action that queues for phase 4** (16b cut
    /// M.1): `STARTUP`, `LOCK` and `FIRE` — the complement of
    /// [`IntentKind::is_continuous`], which counts a squad's
    /// [`IntentKind::Order`] as continuous — and exactly the kinds the
    /// per-seat rate in [`apply_intent`] counts and the arena task's pre-pulse
    /// buffer bounds per unit. Spelt out rather than derived as
    /// `!is_continuous()` so a new kind has to say which it is.
    ///
    /// **16b cut I.3**: a staff [`IntentKind::Place`] is discrete too — it
    /// queues for phase 4 — but it is neither counted by the rate nor bounded
    /// by the task's buffer (see the kind).
    #[must_use]
    pub const fn is_discrete(&self) -> bool {
        matches!(
            self,
            IntentKind::Startup
                | IntentKind::Lock { .. }
                | IntentKind::Fire { .. }
                | IntentKind::Place { .. }
        )
    }

    /// The channel this order must arrive on. An order on the wrong channel is
    /// refused [`Refusal::Channel`] whatever the epoch table says — a gunner's
    /// grant is not a driver's (the F3 rule).
    #[must_use]
    pub const fn channel_required(&self) -> Channel {
        match self {
            IntentKind::Heading { .. }
            | IntentKind::Speed { .. }
            | IntentKind::Startup
            | IntentKind::Order(_)
            | IntentKind::ReleaseOrder { .. }
            | IntentKind::Place { .. } => Channel::Helm,
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
            IntentKind::Order(_) => "order",
            IntentKind::ReleaseOrder { .. } => "release",
            IntentKind::Place { .. } => "place",
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
    /// The world's monotonic stamp, read by the task's drain, which sorts a
    /// burst into the pilot's order (16b cut M.1: the arena task's pre-pulse
    /// buffer hands each unit's discrete orders to phase 2 sorted stably on it,
    /// and coalesces a continuous order to the one with the highest).
    pub seq: u64,
}

/// **Whose order this is** (16b cut M.1, `T-82`): a person's, through the
/// world's seam, or the reflex controller's own.
///
/// It decides one thing: whether the order counts against the per-seat rate
/// [`apply_intent`] holds a discrete order to. Pulse phase 2 passes
/// [`IntentSource::Seat`] for every order the world sent; the reflex machine
/// passes [`IntentSource::Machine`] for the orders it gives inside a substep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentSource {
    /// A person at a seat, through the world's seam — counted against
    /// `intents.max_pending_per_unit` per arena second on the unit it orders.
    Seat,
    /// The reflex controller's own order (`crate::arena::reflex`) — never
    /// counted and never refused for the rate.
    Machine,
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
    /// **A seat has given this unit its `intents.max_pending_per_unit` discrete
    /// orders for this arena second** (16b cut M.1, `T-82`). The setting is the
    /// discrete orders one seat may give a unit per arena second; also the size,
    /// per unit, of the world side's inbox and pre-pulse buffer. The word
    /// (`queue_full`) and the cockpit's sentence are unchanged from when it was
    /// a queue's bound.
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
    /// **`FIRE` on a platform at its heat ceiling** (16b cut X.1; doc 16 §9 item
    /// 1: *a shot refused above a heat ceiling*). A platform whose content
    /// declares no heat is never refused this.
    #[error("that unit is overheated")]
    Overheated,
    /// **`FIRE` naming a weapon the unit does not carry** (16b cut W.1).
    #[error("that unit carries no such weapon")]
    NoWeapon,
    /// **An `ORDER` for a unit a pilot is flying** (16b cut N.2; doc 16 §6). A
    /// squad's order steers the reflex machine, and a seated tank has none.
    #[error("that unit is not on the reflex controller")]
    NotReflex,
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
            Refusal::Overheated => "overheated",
            Refusal::NoWeapon => "no_weapon",
            Refusal::NotReflex => "not_reflex",
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
///
/// **The stale gate** (16b cut Y.3; 16c `N4`; doc 07 §1.3). A `Control` whose
/// epoch is **below** the channel's current one runs the purge (which then
/// removes nothing it would not have) and **applies no seat state and no
/// controller change**. That is what settles a reclaim racing a release by the
/// epoch table rather than by arrival: a pilot who dropped and `BOARD`s again is
/// sent `Live` at a new epoch, and a `Released` from the drop that reaches the
/// arena after it — a pulse late, or displaced in the task's buffer — is below
/// that epoch and cannot take the pilot back off the roster or hand the tank
/// back to the machine; a `Live` behind a newer `Released` cannot re-seat one
/// either. **Equal epochs still apply** — K4's revival inside the grace is a
/// `Live` that may land at the `Linkdead`'s own epoch and must clear it.
///
/// **The roster re-add** (16b cut Y.3). A `Live` that names a pilot puts them
/// back on the frame roster ([`Arena::seat_pilot`], idempotent) **before** the
/// controller is handed back to `Controller::Pilot`, so the pulse that reseats a
/// dropped pilot builds them a whole keyframe (`drop_pilot` forgot their last
/// frame). Between those two lines the machine still holds the tank: that is
/// the moment a caller that wants the machine's last state reads it.
///
/// **The catch-up report** (16b cut Y.4; doc 07 §1.3 step 5). A `Linkdead` or
/// `Released` that names a pilot snapshots the platform
/// (`Arena::note_sight_lost`, the first one winning); the `Live` that re-seats
/// a dropped pilot turns the snapshot into their report between the two lines
/// above (`Arena::settle_catchup`), reading the machine's words before its row
/// goes, and a `Live` that re-seats nobody spends the snapshot. Both tables are
/// views ([`crate::arena::catchup`]) and a stale control reaches neither.
pub fn apply_control(arena: &mut Arena, c: &Control) {
    arena.epochs.raise(c.unit, c.channel, c.epoch);
    let current = arena.epochs.current(c.unit, c.channel);
    let stale = c.epoch < current;
    if let Some(queue) = arena.pending.get_mut(&c.unit) {
        queue.retain(|queued| !(queued.channel == c.channel && queued.epoch < current));
        if queue.is_empty() {
            arena.pending.remove(&c.unit);
        }
    }
    // **16b cut Y.3**: a stale control has purged, and that is all it does.
    if stale {
        return;
    }
    {
        let Some(unit) = arena.unit_mut(c.unit) else {
            return;
        };
        match c.state {
            // Doc 16 §3.1's honest and dangerous answer: the tank keeps its
            // continuous orders and fires nothing. The controller is **not** handed
            // the tank here — a pilot inside the doc 07 D1 grace still owns it, and
            // an AI that took the wheel for four seconds and gave it back would be
            // worse than a tank that carries on.
            ControlState::Linkdead => unit.linkdead = true,
            ControlState::Live => unit.linkdead = false,
            ControlState::Released => {
                unit.linkdead = false;
                unit.desired_speed_cms = 0;
            }
        }
    }
    // **16b cut Y.4** (doc 07 §1.3 step 5): the moment a pilot loses sight of
    // their tank — the line dropping, or the seat going — is the catch-up
    // report's baseline. The first control that took their sight wins
    // (`Arena::note_sight_lost`), so a drop's three channels take one snapshot
    // and the `Released` that ends the grace keeps the `Linkdead`'s. A view,
    // never hashed: the cross-tempo property is untouched.
    if c.pilot.is_some() && matches!(c.state, ControlState::Linkdead | ControlState::Released) {
        arena.note_sight_lost(c.unit);
    }
    // **16b cut N.1** — who flies it now (doc 16 §3.8 "Disconnects" and its §11
    // amendment; doc 07 §1.1's fallback controller).
    //
    // `Released` is the seat *gone*: `EXIT`, a revoked lease, a deactivated
    // station, or the linkdead grace run out. The tank goes to the reflex
    // controller in `Withdraw` and drives itself home, which is what K4's "the
    // tank idles" was standing in for until this cut existed. `Live` is the seat a
    // pilot's again — a reclaim inside the grace, a fresh activation, or (16b cut
    // Y.3) a dropped pilot's `BOARD` past it — and takes it back, forgetting the
    // machine. `Linkdead` changes nothing.
    match c.state {
        ControlState::Released => {
            // **16b cut F.1**: the seat is gone, so the arena stops building that
            // pilot a frame. Until this cut `Arena::pilots` was fixed at check-out
            // and nothing ever pruned it, so a pilot who typed `EXIT` in the first
            // minute went on reading contact notices and keyframes — pose, weapons,
            // every contact's cell — for a tank the reflex controller was driving.
            // Doc 16 §3.6 is written about what *a pilot* sees; that person is not
            // one any more. Idempotent: `Released` arrives once per channel.
            if let Some(pilot) = c.pilot {
                arena.drop_pilot(pilot, c.unit);
            }
            arena.set_controller(c.unit, Controller::Reflex, ReflexState::Withdraw);
        }
        ControlState::Live => {
            // **16b cut Y.3**: a `Live` naming a pilot is that pilot in the seat,
            // so they are on the frame roster — back on it after a reclaim, where
            // `drop_pilot` took them off at the `Released`, and a no-op for a
            // revival inside the grace or a pilot who never left. Before the
            // controller changes: the machine's state is still readable here.
            if let Some(pilot) = c.pilot
                && arena.unit(c.unit).is_some()
            {
                let reseated = arena.seat_pilot(pilot, c.unit, c.epoch);
                // **16b cut Y.4**: the catch-up report, built here and nowhere
                // else — after the re-seat, so it is owed only to a pilot the
                // arena had dropped, and before the controller changes, so
                // the machine's own words are still on its row. A `Live` that
                // re-seated nobody spends the snapshot and builds nothing.
                arena.settle_catchup(pilot, c.unit, reseated);
            }
            if arena
                .unit(c.unit)
                .is_some_and(|unit| unit.controller != Controller::Pilot)
            {
                arena.set_controller(c.unit, Controller::Pilot, ReflexState::Withdraw);
            }
        }
        ControlState::Linkdead => {}
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
/// ([`Refusal::Linkdead`]) and, for an order from an [`IntentSource::Seat`], the
/// seat must not have spent this arena second's rate ([`Refusal::QueueFull`]);
/// the action is pushed to the back of the unit's queue for phase 4. Power is
/// deliberately **not** checked here: `STARTUP` is the order that turns power
/// on, and `LOCK` and `FIRE` become legal in phase 4, which is where doc 16 §3.1
/// puts *resolve newly queued discrete actions that became legal this pulse*.
///
/// # The rate (16b cut M.1, `T-82`)
///
/// `intents.max_pending_per_unit` is **the discrete orders one seat may give a
/// unit per arena second**, counted in
/// [`crate::arena::state::UnitState::orders_window_s`] and
/// [`crate::arena::state::UnitState::orders_in_window`]: the window is
/// `now_ms.div_euclid(1_000)`, a new window resets the count, and an order that
/// finds the count at the bound is refused. Before this cut the setting bounded a
/// queue drained once a pulse, so the burst a pilot could send was a function of
/// the tempo — eight per 5,000 ms pulse at `Quiet`, eight per 500 ms at
/// `Engaged` — which is what doc 16 §3.1's rate independence forbids; an arena
/// second is the same length at every tempo.
///
/// It is counted on the **source**, not on the unit's controller. The machine's
/// own orders ([`IntentSource::Machine`]) are never counted and never refused
/// here: a machine's tick is already bounded by `reflex.decide_ms`, and its
/// order is taken straight back out of the queue it was pushed to, so a fixture
/// with a fast `decide_ms` must not trip a rate meant for people. Keying the rate
/// on the controller instead would leave a gunner still seated on a tank the
/// reflex controller drives — after the driver's `EXIT` hands it over — with no
/// bound at all. The `pending` queue itself has no bound inside a pulse: it is
/// bounded by the caller's slice, which the arena task bounds per unit.
pub fn apply_intent(arena: &mut Arena, i: &Intent, source: IntentSource) -> IntentOutcome {
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
    let now_ms = arena.now_ms;
    let Some(unit) = arena.unit_mut(i.unit) else {
        return IntentOutcome::Refused(Refusal::Unknown);
    };
    // **16b cut I.3**: a staff place is the one order a wreck still takes — a
    // wreck blocking the ford is exactly the stuck unit staff most want to move.
    let place = matches!(i.kind, IntentKind::Place { .. });
    if unit.limits.wreck && !place {
        return IntentOutcome::Refused(Refusal::Wreck);
    }
    // **16b cut N.2** — the squad's standing order (doc 16 §6 "Deliberation").
    //
    // It is taken here, before the continuous branch, because it is subject to
    // neither of that branch's two gates: an order to a **cold** tank is how a
    // squad is told to start up and advance, and an order to a tank whose engine
    // is gone still tells it whether to shoot. What it *is* subject to is the one
    // gate the other five do not have: the unit must be the controller's, because
    // an order steers a machine and a seated tank has none.
    if let IntentKind::Order(order) = &i.kind {
        if unit.controller != Controller::Reflex {
            return IntentOutcome::Refused(Refusal::NotReflex);
        }
        let at = arena.now_ms;
        // **16b cut Y.1**: the hash is asked **before** the order is written —
        // `Arena::event_hash`'s own rule, *every resolver asks before it writes* —
        // so the line carries the state the order landed on, and every `ordered`
        // line of one pulse shares phase 2's one lazy hash (`step.rs` opens
        // `ActionHash::Pending` around the intent loop).
        let hash = arena.event_hash();
        arena.reflex.set_order(
            i.unit,
            crate::deliberation::order::StandingOrder::accepted((**order).clone(), at),
        );
        // **16b cut Y.1** — the black box records the order that landed, one line
        // per unit, at `now_ms`, with no draws. It is cadence-safe for the reason
        // `StandingOrder::accepted_ms` already is: phase 2 runs at the pulse's
        // start, and a world beat lands in the pulse that reaches it, so the same
        // scripted order journals at the same `t_ms` at 5,000, 1,000 and 500 ms.
        // A refused order returned above and journals nothing.
        let _ = arena.ledger.append(
            at,
            EventKind::Ordered {
                unit: i.unit,
                squad: SmolStr::new(order.squad.to_string()),
                issuer: order.issuer,
                objective: SmolStr::new_static(order.objective.as_str()),
                target: order.preferred_contact(),
                spot: order.spot.clone(),
                hold_fire: order.constraints.hold_fire,
                ttl_ms: order.ttl_ms,
            },
            Vec::new(),
            hash,
        );
        // Latest-wins, like every other continuous order: a squad that is sent two
        // orders in one burst obeys the second, and neither queues.
        return IntentOutcome::AppliedContinuous;
    }
    // **16b cut Z.2** (16c Q44) — the release, beside the order and before the
    // continuous gates for the order's own reason: a release to a cold tank, or
    // to one whose engine is gone, still hands it back to doctrine.
    if let IntentKind::ReleaseOrder { squad, issuer } = &i.kind {
        if unit.controller != Controller::Reflex {
            return IntentOutcome::Refused(Refusal::NotReflex);
        }
        release_order(arena, i.unit, *squad, *issuer);
        return IntentOutcome::AppliedContinuous;
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
            IntentKind::Startup
            | IntentKind::Lock { .. }
            | IntentKind::Fire { .. }
            | IntentKind::Order(_)
            | IntentKind::ReleaseOrder { .. }
            | IntentKind::Place { .. } => {}
        }
        return IntentOutcome::AppliedContinuous;
    }
    // **16b cut I.3**: nor does a linkdead seat refuse a staff place — a
    // linkdead pilot's wedged tank is the other stuck unit staff reach for, and
    // nobody is at the seat to have given the order anyway.
    if unit.linkdead && !place {
        return IntentOutcome::Refused(Refusal::Linkdead);
    }
    // **16b cut M.1** (`T-82`): the rate, in arena seconds, on a seat's orders
    // only — see this function's head for why the machine is not counted.
    // **16b cut I.3**: and never on a staff place, which arrives stamped as the
    // seat's own but is not the pilot's: it must not spend one of the pilot's
    // `max_pending_per_unit` a second, whose refused next order would tell the
    // pilot something happened.
    if source == IntentSource::Seat && !place {
        let window = now_ms.div_euclid(1_000);
        if unit.orders_window_s != window {
            unit.orders_window_s = window;
            unit.orders_in_window = 0;
        }
        if unit.orders_in_window >= max_pending {
            return IntentOutcome::Refused(Refusal::QueueFull);
        }
        unit.orders_in_window = unit.orders_in_window.saturating_add(1);
    }
    arena
        .pending
        .entry(i.unit)
        .or_default()
        .push_back(i.clone());
    IntentOutcome::Queued
}

/// **Apply one unit's release** (16b cut Z.2; 16c Q44), for [`apply_intent`]'s
/// [`IntentKind::ReleaseOrder`] arm once the unit is known to be the
/// controller's.
///
/// A unit under no order, or under an order written for another squad, applies
/// nothing and journals nothing: a release names a squad, and a machine the
/// squad's commander has not ordered is not the pilot's to hand back. Otherwise,
/// in this order:
///
/// 1. the hash is asked **before** anything is written — `Arena::event_hash`'s
///    rule, so under phase 2's `Pending` slot every `ordered` and
///    `order_released` line of one pulse shares the phase's one hash;
/// 2. the machine drops to doctrine through
///    [`crate::arena::reflex::ReflexMemory::drop_to_doctrine`] — the expiry
///    block's own restoration, so a released `Withdraw` whose health does not
///    ask for one reads `Patrol` at once and does not keep driving to its entry
///    zone;
/// 3. one `order_released` line at `now_ms`;
/// 4. **the squad is marked for its next signal only when no unit of its roster
///    still holds an order written for it** (a wreck's leftover order does not
///    count: a wreck is no subject of any order) — so the next signal says
///    `OrderExpired` once the whole squad is off orders, and a wingman's release
///    of the one machine of their pair does not hand the lead's squad to the LLM
///    seam while the lead's order still runs on the rest of it. The read is
///    bounded by the roster (at most `SQUAD_SIZE_MAX` units).
fn release_order(
    arena: &mut Arena,
    unit: ObjId,
    squad: crate::deliberation::SquadId,
    issuer: Option<ObjId>,
) {
    if !arena
        .reflex
        .order(unit)
        .is_some_and(|standing| standing.order.squad == squad)
    {
        return;
    }
    let health_ok = arena.reflex_health_permille(unit) >= arena.reflex.config().withdraw_permille;
    let at = arena.now_ms;
    let hash = arena.event_hash();
    arena.reflex.drop_to_doctrine(unit, health_ok);
    let _ = arena.ledger.append(
        at,
        EventKind::OrderReleased {
            unit,
            squad: SmolStr::new(squad.to_string()),
            issuer,
        },
        Vec::new(),
        hash,
    );
    let still_ordered = arena
        .squads
        .iter()
        .find(|roster| roster.id == squad)
        .is_some_and(|roster| {
            roster.units.iter().any(|member| {
                // A wreck is no longer the subject of any order (doc 16 §3.3):
                // `damage` adopts it into `Disabled` with the standing order it
                // died under and takes its ticks away, and a release refuses it
                // as every order does — so the order a wreck still carries is
                // not an order the squad is under.
                arena.unit(*member).is_some_and(|state| !state.limits.wreck)
                    && arena
                        .reflex
                        .order(*member)
                        .is_some_and(|standing| standing.order.squad == squad)
            })
        });
    if !still_ordered {
        arena.note_order_expired(squad);
    }
}

/// **Take a unit's queued discrete actions**, in the order they were queued — cut
/// A.2's phase 4 drains each unit in unit-id order and resolves what it finds.
///
/// Draining is total: an action phase 4 cannot resolve this pulse is re-queued by
/// its owner or refused by it, never left to rot in a queue.
pub fn drain_pending(arena: &mut Arena, unit: ObjId) -> Vec<Intent> {
    arena
        .pending
        .remove(&unit)
        .map(|queue| queue.into_iter().collect())
        .unwrap_or_default()
}

/// **Resolve `STARTUP`** (doc 16 §3.3 "Startup"): `Off` becomes `Starting` and
/// `StartupComplete` is scheduled at `at_ms + match.startup_ms`, with one
/// `startup` ledger event at `at_ms`.
///
/// **16b cut M.2** (16c `T-02`): `at_ms` is the arena millisecond the order
/// resolves at. Phase 4 passes `now_ms`, the end of the pulse that accepted a
/// pilot's `STARTUP`; the reflex controller passes its tick's own timestamp,
/// which is earlier than the substep end `now_ms` had already been advanced to
/// when the tick ran.
///
/// **Idempotent**: a unit already `Starting` or `Online` is a no-op answering
/// `Ok(())`, so a pilot mashing the key schedules one completion and not five.
///
/// The ledger event carries the state hash as it was *before* the transition,
/// which is the same convention cut A.2 uses for a substep: an event says what
/// the world looked like when it happened. **16b cut W.1** routed it through
/// [`Arena::event_hash`], so every action phase 4 resolves shares the one hash the
/// phase took — which is that convention read once more, and O(1) hashes a pulse
/// instead of one per action. (16b cut M.3, `T-23`: taken at the phase's first
/// journalled line, still before that line's own write, and not at all by a
/// phase that journals nothing.)
///
/// # Errors
///
/// [`Refusal::Unknown`] for a unit this arena does not hold, [`Refusal::Wreck`]
/// for a wreck. A ledger that has reached its byte bound does **not** fail the
/// transition: the black box is a record, not an authority (see
/// [`crate::arena::ledger::LedgerFull`]).
pub fn resolve_startup(arena: &mut Arena, unit: ObjId, at_ms: i64) -> Result<(), Refusal> {
    let Some(state) = arena.unit(unit) else {
        return Err(Refusal::Unknown);
    };
    if state.limits.wreck {
        return Err(Refusal::Wreck);
    }
    if state.power != Power::Off {
        return Ok(());
    }
    let at = at_ms.saturating_add(i64::from(arena.match_cfg.startup_ms));
    let hash = arena.event_hash();
    if let Some(state) = arena.unit_mut(unit) {
        state.power = Power::Starting;
    }
    arena.due.schedule(at, DueKind::StartupComplete { unit });
    let _ = arena
        .ledger
        .append(at_ms, EventKind::Startup { unit }, Vec::new(), hash);
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
        // 16b cut I.3: the staff place, the eighth kind — discrete, on helm.
        let place = IntentKind::Place { x: 3, y: 4 };
        assert!(place.is_discrete() && !place.is_continuous());
        assert_eq!(place.channel_required(), Channel::Helm);
        assert_eq!(place.as_str(), "place");
    }

    /// **16b cut I.3**: a place is queued for phase 4 past the three gates a
    /// pilot's discrete order meets — a wreck, a linkdead seat, and the seat's
    /// rate — and still meets the unit, epoch and channel gates.
    #[test]
    fn i3_a_place_is_queued_past_the_wreck_the_linkdead_and_the_rate() {
        let place = |seq| intent(RED, 1, seq, IntentKind::Place { x: 2, y: 2 });
        let mut arena = two_tanks(1);
        if let Some(unit) = arena.unit_mut(RED) {
            unit.limits.wreck = true;
            unit.linkdead = true;
            unit.orders_in_window = arena_cfg_bound();
        }
        assert_eq!(
            apply_intent(&mut arena, &place(1), IntentSource::Seat),
            IntentOutcome::Queued
        );
        assert_eq!(
            arena.unit(RED).map(|unit| unit.orders_in_window),
            Some(arena_cfg_bound()),
            "the rate was not spent"
        );
        // The three gates it does meet.
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(ObjId(77), 1, 2, IntentKind::Place { x: 2, y: 2 }),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Unknown)
        );
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 0, 3, IntentKind::Place { x: 2, y: 2 }),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Epoch)
        );
        assert_eq!(
            apply_intent(
                &mut arena,
                &Intent {
                    channel: Channel::Sensors,
                    ..place(4)
                },
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Channel)
        );
    }

    /// `intents.max_pending_per_unit` as the default configuration sets it.
    fn arena_cfg_bound() -> u32 {
        crate::config::ArenaConfig::default()
            .intents
            .max_pending_per_unit
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
                    &intent(RED, 1, seq, IntentKind::Heading { mdeg }),
                    IntentSource::Seat
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
            IntentSource::Seat,
        );
        assert_eq!(speed(&arena), Some(400));
        apply_intent(
            &mut arena,
            &intent(RED, 1, 1, IntentKind::Speed { cms: 9_999 }),
            IntentSource::Seat,
        );
        assert_eq!(speed(&arena), Some(1_000));
        apply_intent(
            &mut arena,
            &intent(RED, 1, 2, IntentKind::Speed { cms: -100 }),
            IntentSource::Seat,
        );
        assert_eq!(speed(&arena), Some(-100));
        apply_intent(
            &mut arena,
            &intent(RED, 1, 3, IntentKind::Speed { cms: -9_999 }),
            IntentSource::Seat,
        );
        assert_eq!(speed(&arena), Some(-400));
        apply_intent(
            &mut arena,
            &intent(RED, 1, 4, IntentKind::Speed { cms: i32::MIN }),
            IntentSource::Seat,
        );
        assert_eq!(speed(&arena), Some(-400));
        apply_intent(
            &mut arena,
            &intent(RED, 1, 5, IntentKind::Speed { cms: 0 }),
            IntentSource::Seat,
        );
        assert_eq!(speed(&arena), Some(0));
    }

    #[test]
    fn a_continuous_order_refuses_before_power_is_online() {
        let mut arena = two_tanks(1);
        assert_eq!(arena.unit(RED).map(|state| state.power), Some(Power::Off));
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 0, IntentKind::Heading { mdeg: 90_000 }),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::PowerNotOnline)
        );
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 1, IntentKind::Speed { cms: 500 }),
                IntentSource::Seat
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
                &intent(RED, 1, 2, IntentKind::Speed { cms: 500 }),
                IntentSource::Seat
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
                &intent(RED, 1, 3, IntentKind::Speed { cms: 500 }),
                IntentSource::Seat
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
                &intent(RED, 1, 0, IntentKind::Speed { cms: 500 }),
                IntentSource::Seat
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
                &intent(RED, 1, 1, IntentKind::Speed { cms: 500 }),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Wreck)
        );
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 2, IntentKind::Startup),
                IntentSource::Seat
            ),
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
                },
                IntentSource::Seat
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
                },
                IntentSource::Seat
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
                },
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Channel)
        );
        // And an unknown unit, before either.
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(ObjId(77), 1, 3, IntentKind::Startup),
                IntentSource::Seat
            ),
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
                    ),
                    IntentSource::Seat
                ),
                IntentOutcome::Queued,
                "action {seq} of the bound"
            );
        }
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 99, IntentKind::Startup),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::QueueFull)
        );
        // The other unit's queue is its own.
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(BLUE, 1, 0, IntentKind::Startup),
                IntentSource::Seat
            ),
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
            IntentSource::Seat,
        );
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 1,
                state: ControlState::Linkdead,
                pilot: None,
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
            apply_intent(
                &mut arena,
                &intent(RED, 1, 1, IntentKind::Startup),
                IntentSource::Seat
            ),
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
                ),
                IntentSource::Seat
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
                pilot: None,
            },
        );
        assert_eq!(arena.unit(RED).map(|state| state.linkdead), Some(false));
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 3, IntentKind::Startup),
                IntentSource::Seat
            ),
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
            IntentSource::Seat,
        );
        apply_intent(
            &mut arena,
            &intent(RED, 1, 1, IntentKind::Speed { cms: 700 }),
            IntentSource::Seat,
        );
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 2,
                state: ControlState::Released,
                pilot: None,
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
            apply_intent(
                &mut arena,
                &intent(RED, 1, 0, IntentKind::Startup),
                IntentSource::Seat
            ),
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
                pilot: None,
            },
        );
        assert!(arena.pending.is_empty(), "the queued STARTUP is gone");
        assert_eq!(arena.epochs.current(RED, Channel::Helm), 2);
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 1, IntentKind::Startup),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Epoch)
        );
        // At the new epoch it queues again.
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 2, 2, IntentKind::Startup),
                IntentSource::Seat
            ),
            IntentOutcome::Queued
        );
    }

    #[test]
    fn a_purge_is_per_channel_and_a_stale_control_purges_nothing() {
        let mut arena = two_tanks(1);
        apply_intent(
            &mut arena,
            &intent(RED, 1, 0, IntentKind::Startup),
            IntentSource::Seat,
        );
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
            IntentSource::Seat,
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
                pilot: None,
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
                pilot: None,
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
                pilot: None,
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
                pilot: None,
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
        assert_eq!(resolve_startup(&mut arena, RED, 0), Ok(()));
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
            assert_eq!(resolve_startup(&mut arena, RED, 0), Ok(()));
        }
        assert_eq!(arena.due.len(), before + 1);
        assert_eq!(arena.ledger.seq(), ledger_before + 1);

        // Already online is a no-op too.
        online(&mut arena, RED);
        assert_eq!(resolve_startup(&mut arena, RED, 0), Ok(()));
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
        assert_eq!(resolve_startup(&mut arena, BLUE, 42_000), Ok(()));
        assert_eq!(
            arena
                .due
                .iter()
                .find(|event| event.kind == DueKind::StartupComplete { unit: BLUE })
                .map(|event| event.t_ms),
            Some(52_000)
        );
    }

    /// **16b cut M.2** (16c `T-02`): the order resolves at the millisecond it is
    /// handed, not at `now_ms` — a reflex tick at 1,009 ms inside a substep whose
    /// end `now_ms` already reads 1,250 starts up at 1,009.
    #[test]
    fn m2_startup_resolves_at_the_millisecond_it_is_given_and_not_at_now() {
        let mut arena = two_tanks(1);
        arena.now_ms = 1_250;
        let before = arena.ledger.seq();
        assert_eq!(resolve_startup(&mut arena, BLUE, 1_009), Ok(()));
        assert_eq!(
            arena
                .due
                .iter()
                .find(|event| event.kind == DueKind::StartupComplete { unit: BLUE })
                .map(|event| event.t_ms),
            Some(11_009)
        );
        arena.ledger.flush();
        let lines: Vec<crate::arena::ledger::LedgerEvent> = arena
            .ledger
            .take_chunks()
            .iter()
            .flat_map(|chunk| {
                String::from_utf8_lossy(&chunk.bytes)
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect::<Vec<_>>()
            })
            .collect();
        let startup = lines
            .iter()
            .find(|event| event.seq == before + 1)
            .map(|event| (event.t_ms, event.kind.as_str()));
        assert_eq!(startup, Some((1_009, "startup")));
    }

    #[test]
    fn startup_refuses_an_unknown_unit_and_a_wreck() {
        let mut arena = two_tanks(1);
        assert_eq!(
            resolve_startup(&mut arena, ObjId(77), 0),
            Err(Refusal::Unknown)
        );
        if let Some(state) = arena.unit_mut(RED) {
            state.limits.wreck = true;
        }
        assert_eq!(resolve_startup(&mut arena, RED, 0), Err(Refusal::Wreck));
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

    // -----------------------------------------------------------------------
    // 16b cut Y.1 — the `ordered` line
    // -----------------------------------------------------------------------

    /// Every line the ledger holds, flushed.
    fn y1_lines(arena: &mut Arena) -> Vec<crate::arena::ledger::LedgerEvent> {
        arena.ledger.flush();
        arena
            .ledger
            .take_chunks()
            .iter()
            .flat_map(|chunk| {
                String::from_utf8_lossy(&chunk.bytes)
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// **16b cut Y.1**: an order from a pilot journals one `ordered` line per
    /// unit it lands on, at `now_ms`, with every field off the order — the
    /// issuer, the squad, the objective's word, the hint **as the order carries
    /// it** (the issuer's contact id), the spot, the constraint and the time to
    /// live — and a refused order (here a seated tank's, `NotReflex`) journals
    /// nothing at all.
    #[test]
    fn y1_an_order_journals_one_ordered_line_per_unit_and_a_refused_one_nothing() {
        let mut arena = two_tanks(1);
        arena.now_ms = 7_000;
        for unit in [RED, BLUE] {
            if let Some(state) = arena.unit_mut(unit) {
                state.controller = Controller::Reflex;
            }
        }
        let hint = ContactId(0x2350_0000_0000_0001);
        let squad = crate::deliberation::squad::SquadId {
            team: Team::Red,
            index: 1,
        };
        let told = crate::deliberation::order::Order {
            squad,
            objective: crate::deliberation::order::Objective::Engage {
                target_hint: Some(hint),
            },
            target: None,
            horizon_ms: 600_000,
            constraints: crate::deliberation::order::Constraints {
                hold_fire: true,
                max_range_band: None,
            },
            ttl_ms: 600_000,
            issued_ms: 3,
            issuer: Some(ObjId(77)),
            spot: Some(SmolStr::new("The Ford")),
        };
        let before = arena.ledger.seq();
        for (seq, unit) in [RED, BLUE].into_iter().enumerate() {
            assert_eq!(
                apply_intent(
                    &mut arena,
                    &intent(
                        unit,
                        1,
                        u64::try_from(seq).unwrap_or(0),
                        IntentKind::Order(Box::new(told.clone()))
                    ),
                    IntentSource::Seat
                ),
                IntentOutcome::AppliedContinuous
            );
        }
        let lines: Vec<crate::arena::ledger::LedgerEvent> = y1_lines(&mut arena)
            .into_iter()
            .filter(|event| event.seq > before)
            .collect();
        assert_eq!(lines.len(), 2, "one line per unit: {lines:?}");
        for (line, unit) in lines.iter().zip([RED, BLUE]) {
            assert_eq!(line.t_ms, 7_000, "at now_ms");
            assert!(line.draws.is_empty(), "an order draws nothing");
            assert_eq!(
                line.kind,
                EventKind::Ordered {
                    unit,
                    squad: SmolStr::new("red-1"),
                    issuer: Some(ObjId(77)),
                    objective: SmolStr::new("engage"),
                    target: Some(hint),
                    spot: Some(SmolStr::new("The Ford")),
                    hold_fire: true,
                    ttl_ms: 600_000,
                }
            );
        }
        assert_eq!(lines[0].kind.as_str(), "ordered");
        // The order stands on both machines, stamped at `now_ms`.
        for unit in [RED, BLUE] {
            assert_eq!(
                arena
                    .reflex
                    .order(unit)
                    .map(|standing| standing.accepted_ms),
                Some(7_000)
            );
        }

        // Refused: a seated tank takes no order, and the black box says nothing.
        if let Some(state) = arena.unit_mut(RED) {
            state.controller = Controller::Pilot;
        }
        let before = arena.ledger.seq();
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 9, IntentKind::Order(Box::new(told))),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::NotReflex)
        );
        assert_eq!(
            arena.ledger.seq(),
            before,
            "a refused order journals nothing"
        );
        assert!(y1_lines(&mut arena).is_empty());
    }

    // -----------------------------------------------------------------------
    // 16b cut Y.3: the stale gate and the roster re-add
    // -----------------------------------------------------------------------

    /// `RED`'s seated pilot, as [`arena_at_epoch`] hands the roster over.
    const RED_PILOT: ObjId = ObjId(1_000);

    /// One control per channel for `RED`, naming its pilot.
    fn y3_every_channel(epoch: u64, state: ControlState) -> Vec<Control> {
        Channel::ALL
            .iter()
            .map(|channel| Control {
                unit: RED,
                channel: *channel,
                epoch,
                state,
                pilot: Some(RED_PILOT),
            })
            .collect()
    }

    /// Whether `RED_PILOT` is on `RED`'s frame roster, and how many times.
    fn y3_rows(arena: &Arena) -> usize {
        arena
            .pilots
            .iter()
            .filter(|seat| seat.pilot == RED_PILOT && seat.unit == RED)
            .count()
    }

    /// **A drop and a reclaim in one pulse seat the pilot, in either order**
    /// (16b cut Y.3; doc 07 §1.3): `Released(2)` then `Live(3)` drops and
    /// re-seats; `Live(3)` then `Released(2)` finds the release stale and it
    /// changes nothing. Either way the pilot ends on the roster and the tank is
    /// theirs, not the machine's.
    #[test]
    fn y3_a_release_and_a_newer_live_in_one_step_seat_the_pilot_in_either_order() {
        let released = y3_every_channel(2, ControlState::Released);
        let live = y3_every_channel(3, ControlState::Live);
        let orders: [Vec<Control>; 2] = [
            released.iter().chain(live.iter()).copied().collect(),
            live.iter().chain(released.iter()).copied().collect(),
        ];
        for (n, controls) in orders.iter().enumerate() {
            let mut arena = two_tanks(1);
            arena.step(0, controls, &[]);
            assert_eq!(y3_rows(&arena), 1, "order {n}: the pilot is not seated");
            assert_eq!(
                arena.unit(RED).map(|unit| unit.controller),
                Some(Controller::Pilot),
                "order {n}: the machine kept the tank"
            );
            assert!(
                arena.reflex.unit(RED).is_none(),
                "order {n}: the machine's row survived the reclaim"
            );
            for channel in Channel::ALL {
                assert_eq!(arena.epochs.current(RED, channel), 3, "order {n}");
            }
        }
    }

    /// **A stale `Released` cannot unseat the pilot a newer `Live` seated**
    /// (16b cut Y.3): the release arrives a pulse after the reclaim, below the
    /// epoch the reclaim raised, and purges whatever is queued below the current
    /// epoch — and does nothing else.
    #[test]
    fn y3_a_stale_release_after_a_live_leaves_the_pilot_seated_and_still_purges() {
        let mut arena = two_tanks(1);
        // The drop, then the reclaim in the next pulse: off the roster and back.
        arena.step(0, &y3_every_channel(2, ControlState::Released), &[]);
        assert_eq!(y3_rows(&arena), 0, "the release took the pilot off");
        assert_eq!(
            arena.unit(RED).map(|unit| unit.controller),
            Some(Controller::Reflex)
        );
        arena.step(1_000, &y3_every_channel(3, ControlState::Live), &[]);
        assert_eq!(y3_rows(&arena), 1, "the reclaim put the pilot back");
        // An order below the current epoch, sitting in the queue — what the
        // purge exists to remove.
        arena
            .pending
            .entry(RED)
            .or_default()
            .push_back(intent(RED, 2, 50, IntentKind::Startup));
        // The drop's `Released`, late.
        apply_control(
            &mut arena,
            &Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 2,
                state: ControlState::Released,
                pilot: Some(RED_PILOT),
            },
        );
        assert_eq!(y3_rows(&arena), 1, "a stale release unseated the pilot");
        assert_eq!(
            arena.unit(RED).map(|unit| unit.controller),
            Some(Controller::Pilot),
            "a stale release handed the tank back to the machine"
        );
        assert!(arena.reflex.unit(RED).is_none());
        assert!(
            !arena.pending.contains_key(&RED),
            "the purge below the current epoch still ran"
        );
        assert_eq!(arena.epochs.current(RED, Channel::Helm), 3);
        // And a stale `Live` behind a newer `Released` cannot re-seat one.
        arena.step(2_000, &y3_every_channel(4, ControlState::Released), &[]);
        assert_eq!(y3_rows(&arena), 0);
        arena.step(3_000, &y3_every_channel(3, ControlState::Live), &[]);
        assert_eq!(y3_rows(&arena), 0, "a stale live re-seated the pilot");
        assert_eq!(
            arena.unit(RED).map(|unit| unit.controller),
            Some(Controller::Reflex)
        );
    }

    /// **K4's revival at the `Linkdead`'s own epoch still clears it** (16b cut
    /// Y.3): the stale gate is `<`, never `<=`.
    #[test]
    fn y3_a_live_at_the_linkdead_epoch_still_clears_it() {
        let mut arena = two_tanks(1);
        arena.step(0, &y3_every_channel(2, ControlState::Linkdead), &[]);
        assert_eq!(arena.unit(RED).map(|unit| unit.linkdead), Some(true));
        arena.step(1_000, &y3_every_channel(2, ControlState::Live), &[]);
        assert_eq!(arena.unit(RED).map(|unit| unit.linkdead), Some(false));
        assert_eq!(y3_rows(&arena), 1, "a revival adds no second row");
    }

    /// **`seat_pilot` is idempotent** (16b cut Y.3), and answers whether it
    /// added a row.
    #[test]
    fn y3_seat_pilot_twice_adds_one_row() {
        let mut arena = two_tanks(1);
        assert!(!arena.seat_pilot(RED_PILOT, RED, 1), "already seated");
        assert!(arena.drop_pilot(RED_PILOT, RED));
        assert!(arena.seat_pilot(RED_PILOT, RED, 3));
        assert!(!arena.seat_pilot(RED_PILOT, RED, 3));
        assert_eq!(y3_rows(&arena), 1);
        let row = arena
            .pilots
            .iter()
            .find(|seat| seat.pilot == RED_PILOT)
            .map(|seat| (seat.station.as_str(), seat.epoch));
        assert_eq!(row, Some(("", 3)), "a reclaimed seat carries no station");
    }

    // -----------------------------------------------------------------------
    // 16b cut Z.2 — the release
    // -----------------------------------------------------------------------

    /// The second red machine of the Z.2 fixture.
    const WING: ObjId = ObjId(5);
    /// The releasing pilot's tank.
    const LEAD: ObjId = ObjId(77);

    fn z2_squad(index: u16) -> crate::deliberation::squad::SquadId {
        crate::deliberation::squad::SquadId {
            team: Team::Red,
            index,
        }
    }

    /// A ten-minute order of `objective` for squad `red-1`, from `LEAD`.
    fn z2_order(
        objective: crate::deliberation::order::Objective,
    ) -> crate::deliberation::order::StandingOrder {
        crate::deliberation::order::StandingOrder::accepted(
            crate::deliberation::order::Order {
                squad: z2_squad(1),
                objective,
                target: None,
                horizon_ms: 600_000,
                constraints: crate::deliberation::order::Constraints::default(),
                ttl_ms: 600_000,
                issued_ms: 0,
                issuer: Some(LEAD),
                spot: None,
            },
            0,
        )
    }

    /// Two red machines in squad `red-1`, both under its `Hold`, and a blue tank a
    /// pilot flies.
    fn z2_arena() -> Arena {
        let mut arena = arena_at_epoch(
            vec![
                tank(RED, Team::Red, pose(1_500, 1_500)),
                tank(WING, Team::Red, pose(2_500, 1_500)),
                tank(BLUE, Team::Blue, pose(6_500, 6_500)),
            ],
            1,
        );
        arena.now_ms = 9_000;
        for unit in [RED, WING] {
            if let Some(state) = arena.unit_mut(unit) {
                state.controller = Controller::Reflex;
            }
            arena.reflex.adopt(unit, ReflexState::Patrol);
            arena
                .reflex
                .set_order(unit, z2_order(crate::deliberation::order::Objective::Hold));
        }
        arena.squads = vec![crate::deliberation::squad::SquadRoster {
            id: z2_squad(1),
            commander: RED,
            units: vec![RED, WING],
        }];
        arena
    }

    fn z2_release(unit: ObjId, index: u16, seq: u64) -> Intent {
        intent(
            unit,
            1,
            seq,
            IntentKind::ReleaseOrder {
                squad: z2_squad(index),
                issuer: Some(LEAD),
            },
        )
    }

    /// **16b cut Z.2 (16c Q44)**: `ReleaseOrder` is a continuous `Helm` order
    /// with the word `release`; on a machine under that squad's order it drops
    /// the order to doctrine and journals one `order_released` line carrying the
    /// issuer, at `now_ms`; under another squad's order, or none, it applies
    /// nothing and journals nothing; and on a pilot's tank it is `NotReflex`.
    /// With two machines of one squad under its order, releasing one does not
    /// mark the squad for its next signal and releasing the second does.
    #[test]
    fn z2_a_release_drops_that_squads_order_and_journals_one_line() {
        let kind = IntentKind::ReleaseOrder {
            squad: z2_squad(1),
            issuer: None,
        };
        assert!(kind.is_continuous());
        assert!(!kind.is_discrete());
        assert_eq!(kind.channel_required(), Channel::Helm);
        assert_eq!(kind.as_str(), "release");

        let mut arena = z2_arena();
        let before = arena.ledger.seq();
        // Another squad's word: nothing.
        assert_eq!(
            apply_intent(&mut arena, &z2_release(RED, 2, 1), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert!(
            arena.reflex.order(RED).is_some(),
            "red-2's release is not red-1's"
        );
        // The squad's own word on one machine: that one drops.
        assert_eq!(
            apply_intent(&mut arena, &z2_release(RED, 1, 2), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(arena.reflex.order(RED), None);
        assert!(
            arena.reflex.order(WING).is_some(),
            "the other machine is untouched"
        );
        assert!(
            !arena.squads_expired.contains(&z2_squad(1)),
            "the wingman still holds red-1's order, so the squad is not asked about"
        );
        // A second release to the same machine, now under no order: nothing.
        assert_eq!(
            apply_intent(&mut arena, &z2_release(RED, 1, 3), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        // A pilot's tank has no machine to hand back.
        assert_eq!(
            apply_intent(&mut arena, &z2_release(BLUE, 1, 4), IntentSource::Seat),
            IntentOutcome::Refused(Refusal::NotReflex)
        );
        let lines: Vec<crate::arena::ledger::LedgerEvent> = y1_lines(&mut arena)
            .into_iter()
            .filter(|event| event.seq > before)
            .collect();
        assert_eq!(
            lines.len(),
            1,
            "one line, for the one machine dropped: {lines:?}"
        );
        assert_eq!(lines[0].t_ms, 9_000, "at now_ms");
        assert!(lines[0].draws.is_empty());
        assert_eq!(
            lines[0].kind,
            EventKind::OrderReleased {
                unit: RED,
                squad: SmolStr::new("red-1"),
                issuer: Some(LEAD),
            }
        );
        assert_eq!(lines[0].kind.as_str(), "order_released");
        // The last machine of the squad: now the squad is off orders.
        assert_eq!(
            apply_intent(&mut arena, &z2_release(WING, 1, 5), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(arena.reflex.order(WING), None);
        assert!(
            arena.squads_expired.contains(&z2_squad(1)),
            "the whole squad is off orders, so its next signal asks"
        );

        // A wreck keeps the order it died under, and is refused a release; the
        // squad is off orders once its last living machine is released.
        let mut arena = z2_arena();
        if let Some(state) = arena.unit_mut(WING) {
            state.limits.wreck = true;
        }
        assert_eq!(
            apply_intent(&mut arena, &z2_release(WING, 1, 1), IntentSource::Seat),
            IntentOutcome::Refused(Refusal::Wreck)
        );
        assert!(
            arena.reflex.order(WING).is_some(),
            "the wreck's row keeps it"
        );
        assert_eq!(
            apply_intent(&mut arena, &z2_release(RED, 1, 2), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert!(
            arena.squads_expired.contains(&z2_squad(1)),
            "a wreck's leftover order is no order the squad is under"
        );
    }

    /// Set a machine's armour and internals to their floors but one point of
    /// internals — a hull well under any `withdraw_permille`.
    fn z2_hurt(arena: &mut Arena, unit: ObjId) {
        let at = |cell: &str, facet: bool| {
            if facet {
                mush_tactics::CellAddr::Facet {
                    section: mush_tactics::SectionId::new("hull").expect("a section"),
                    facet: mush_tactics::FacetId::new("front").expect("a facet"),
                    cell: mush_tactics::CellId::new(cell).expect("a cell"),
                }
            } else {
                mush_tactics::CellAddr::Section {
                    section: mush_tactics::SectionId::new("hull").expect("a section"),
                    cell: mush_tactics::CellId::new(cell).expect("a cell"),
                }
            }
        };
        let hurt = {
            let state = arena.unit(unit).expect("the fixture tank");
            let plan = mush_tactics::UnitMutationPlan::new(
                vec![
                    mush_tactics::UnitMutationOp::SetCell {
                        addr: at(crate::definitions::unit::ARMOR_CELL, true),
                        value: 0,
                    },
                    mush_tactics::UnitMutationOp::SetCell {
                        addr: at(crate::definitions::unit::INTERNALS_CELL, false),
                        value: 1,
                    },
                ],
                None,
                Some(SmolStr::new("test:hurt")),
            )
            .expect("a two-op plan");
            mush_tactics::apply(&state.unit, &plan, 0)
                .expect("the plan applies")
                .0
        };
        if let Some(state) = arena.unit_mut(unit) {
            state.unit = hurt;
        }
    }

    /// **16b cut Z.2**: a released `Withdraw` goes through the expiry block's own
    /// restoration (`ReflexMemory::drop_to_doctrine`) — a healthy machine reads
    /// `Patrol` at once and does not keep driving to its entry zone, and a hurt
    /// one (below `withdraw_permille`) stays in `Withdraw`.
    #[test]
    fn z2_a_released_withdraw_reads_patrol_unless_the_hull_asks_for_one() {
        let mut arena = z2_arena();
        z2_hurt(&mut arena, WING);
        assert!(
            arena.reflex_health_permille(WING) < arena.reflex.config().withdraw_permille,
            "the fixture is hurt"
        );
        for unit in [RED, WING] {
            arena.reflex.adopt(unit, ReflexState::Withdraw);
            arena.reflex.set_order(
                unit,
                z2_order(crate::deliberation::order::Objective::Withdraw),
            );
        }
        // A goal, a route and a target written onto the healthy machine after
        // `adopt` (which clears the first two on its own), so the assertion
        // below reads what the release dropped and not what was never there.
        if let Some(row) = arena.reflex.unit_mut(RED) {
            row.goal = Some((1, 1));
            row.route = vec![(2, 2), (1, 1)];
            row.target = Some((crate::arena::state::ContactId(1), BLUE));
        }
        assert_eq!(
            arena
                .reflex
                .unit(RED)
                .map(|row| (row.goal, row.route.len(), row.target.is_some())),
            Some((Some((1, 1)), 2, true)),
            "the fixture's machine is going somewhere after something"
        );
        for (seq, unit) in [RED, WING].into_iter().enumerate() {
            assert_eq!(
                apply_intent(
                    &mut arena,
                    &z2_release(unit, 1, u64::try_from(seq).unwrap_or(0)),
                    IntentSource::Seat
                ),
                IntentOutcome::AppliedContinuous
            );
        }
        let state = |arena: &Arena, unit| arena.reflex.unit(unit).map(|row| row.state);
        assert_eq!(state(&arena, RED), Some(ReflexState::Patrol));
        assert_eq!(
            arena
                .reflex
                .unit(RED)
                .map(|row| (row.goal, row.route.len(), row.target)),
            Some((None, 0, None)),
            "the goal, the route and the target go with the order"
        );
        assert_eq!(state(&arena, WING), Some(ReflexState::Withdraw));
        assert_eq!(arena.reflex.order(WING), None, "the order goes either way");
    }
}
