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
//!   affecting nothing on the hull. It is written onto the unit's mind row
//!   (onto its reflex machine until 17b cut M4.6) and read by its next think,
//!   so an order changes *what the controller decides* and never what the
//!   physics does.
//!
//! - **[`IntentKind::Place`]** (16b cut I.3) is a staff act that rides the same
//!   path: discrete, resolved in phase 4, and exempt from the three gates that
//!   exist for a pilot's orders (the wreck, `Linkdead` and the seat's rate).
//! - **[`IntentKind::Emcon`]** (16b cut O.2) is a pilot's discrete action like
//!   `STARTUP`, on [`Channel::Sensors`], resolved in phase 4 by
//!   [`resolve_emcon`]: the jamming pod's switch.
//! - **[`IntentKind::Goal`]** (17b cut M1a.4; doc 17 §4 D-UM-10) is a pilot's
//!   `DRIVE`: continuous, on [`Channel::Helm`], latest-wins, and counted on
//!   the seat's rate. It writes the hull's motor row
//!   ([`crate::arena::motor::MotorRow`]) and the motor drives it between
//!   ticks; a seat's `HEADING` or `SPEED` afterward takes the helm back.
//! - **[`IntentKind::Condition`]** (17b cut M1b.1; doc 17 §5 D-UM-16) is an
//!   arming of a condition slot: **discrete**, on [`Channel::Fire`], word
//!   `fire`, queued at phase 2 and resolved in phase 4 in arrival order with
//!   `LOCK` and `FIRE` by `Arena::resolve_arming`; a mind's lands at its
//!   landing. A slot's own shot is a `Fire` under [`IntentSource::Cond`].
//!
//! A refusal is always one of the named [`Refusal`] words, never a silent drop:
//! cut K's cockpit turns each into a line a pilot can act on (a place's is a
//! `place_refused` line on the tape instead, and is never said).

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::doctrine::MachineState;
use crate::arena::ledger::EventKind;
use crate::arena::state::{Arena, Channel, Classification, ContactId, Controller, DueKind, Power};

/// What a seat is (doc 16 §3.1's `Control`; doc 07 §1.2 for linkdead).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
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
        /// **The slot this shot cites** (17b cut M1b.1; doc 17 §5, D-UM-17):
        /// `None` for a seat's typed `FIRE` and a machine's (T0's) shot, `Some(n)`
        /// for a mind's citation of its slot `n` (gated from 17b cut M1b.2) and
        /// for the evaluator's own slot shot. Absent on the wire when `None`, so
        /// every carried `Fire` of format 6's shape reads the same bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slot: Option<u8>,
    },
    /// **Arm or disarm a condition slot** (17b cut M1b.1; doc 17 §4 D-UM-10,
    /// §5 D-UM-16): the same intent a pilot's `FIRE <weapon> WHEN …` stages
    /// (17b cut M1b.4) and a mind's `arm`/`disarm` — one intent, two
    /// principals. **Discrete**, [`Channel::Fire`], word `fire` (the pilot
    /// typed `FIRE`, so a staged refusal reads `FIRE cannon refused. …`).
    ///
    /// A seat's arming is queued at phase 2 and **resolved at the pulse's end
    /// in phase 4, in arrival order with `LOCK` and `FIRE`** — `LOCK BA` then
    /// `FIRE cannon WHEN …` typed in one 5,000 ms pulse lands the lock, then the
    /// arming, and the arming's first evaluation reads the new lock, where a
    /// continuous kind applied at the pulse's start would have fired at the old
    /// one for a pulse of shots; a mind's lands through
    /// `Arena::apply_machine_intents`' queued branch at its landing millisecond
    /// in emission order. Both resolve through
    /// [`crate::arena::state::Arena::resolve_arming`] (17b cut M1b.1). It
    /// meets [`apply_intent`]'s discrete gates exactly as a `LOCK` does — the
    /// `Linkdead` gate, the seat's window, the queue — and no others there; a
    /// mind's on a hull with no weapons station is refused `Channel` by the
    /// mind gate every fire-channel intent meets.
    Condition(crate::arena::conditions::Arming),
    /// **A squad's standing order** (16b cut N.2; doc 16 §6 "Deliberation"),
    /// carried in one per unit of the squad.
    ///
    /// It is neither continuous nor discrete in the sense the other five are: it
    /// sets nothing on the hull and queues nothing for phase 4. It is *doctrine*,
    /// written onto the unit's mind row — re-keyed at write time to the hull's
    /// own held contacts (D-UM-15, `Arena::rekey_order`), with its issuer beside
    /// it (17b cut M4.6, the owner's STOP C) — and read by the mind's next think,
    /// which is what keeps an order rate-independent (until 17b cut M4.6 a reflex
    /// hull's machine took it the same way, read at its next tick).
    ///
    /// **Only a machine takes one** ([`Refusal::NotMachine`]; `NotReflex` until
    /// 17b cut M4.6): a pilot's tank has no machine to steer, and an order that
    /// silently did nothing to a seated tank would be a commander who thinks they
    /// have twelve tanks and has six. It travels on [`Channel::Helm`] because it
    /// is, in the end, a route and a posture.
    Order(Box<crate::deliberation::order::Order>),
    /// **Hand a squad back to doctrine** (16b cut Z.2; 16c Q44): drops the
    /// unit's standing order when it was written for `squad`
    /// (`release_machine_order`; through the reflex's own helper, the one its
    /// expiry block used, until 17b cut M4.6). Continuous,
    /// [`Channel::Helm`], word `release`; shares `Order`'s buffer key in the
    /// arena task's pre-pulse buffer, so of an `ORDER` and a `RELEASE` in one
    /// pulse the later `seq` is what applies.
    ///
    /// Like [`IntentKind::Order`] it is taken before the continuous gates (a
    /// release to a cold or engine-killed machine lands, as an order to one
    /// does), only a machine takes one ([`Refusal::NotMachine`]), and its
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
    /// **Switch the unit's jamming pod** (16b cut O.2, 16c N12: `EMCON on|off`).
    /// Discrete, [`Channel::Sensors`] (the pod is part of what the platform
    /// senses with, and the channel `LOCK` travels on), word `emcon`; resolved
    /// in pulse phase 4 by [`resolve_emcon`].
    ///
    /// It meets every gate a pilot's discrete order meets in [`apply_intent`]
    /// — the unit, the epoch, the channel, the wreck, `Linkdead` and the seat's
    /// rate — and nothing else: a pod switches whatever the hull's power, since
    /// the switch is a position and power is what makes the position radiate.
    /// The reflex controller never gave one, leaving the switch where it found
    /// it — on at check-out, or off on a released seat whose pilot had switched
    /// it off (a recorded residual of cut O) — until 17b cut M4.6 deleted it;
    /// the stock mind gives none either, and a scripted mind may
    /// (`emcon(on)`, `crate::minds::stations`).
    Emcon {
        /// `true` to radiate, `false` to fall silent.
        on: bool,
    },
    /// **Hand the helm to the hull's motor** (17b cut M1a.4; doc 17 §4 D-UM-3
    /// and D-UM-10; doc 16 §3.7): a pilot's `DRIVE`, and from 17b cut M1a.5 a
    /// mind's `goto`. Continuous, [`Channel::Helm`], word `drive` — so a
    /// refusal staged at phase 2 reads `DRIVE refused. …`.
    ///
    /// It meets the continuous gates (`PowerNotOnline`, `MotionDisabled`) and,
    /// from a seat, the seat's rate: a [`MotorGoal::Cell`] spends a path search
    /// from the arena-wide budget the motor shares, so a seat may not spam one
    /// ([`apply_intent`]'s `Goal` arm has the whole rule). The word is `DRIVE`
    /// and not doc 17's `GOTO` because core's baseline `goto` (aliases `go`,
    /// `move`) holds that name and the command registry refuses a collision —
    /// the M1a plan's departure (f).
    Goal(MotorGoal),
    /// **Switch the profile the hull's mind will fly** (17b cut M5.2; doc 17
    /// §11, §12, D-UM-44; the owner's OQ-3): a seated pilot's `MIND PROFILE`
    /// (the station verb lands in 17b cut M5.4), naming one of the hull's
    /// sealed profiles by its index in
    /// [`crate::arena::mind::SealRef::profiles`]. Continuous, [`Channel::Helm`],
    /// word `mind` — the verb the pilot types, as `Goal` reads `drive` — under
    /// a buffer key of its own (`continuous_code` 5 in the arena task), so of
    /// two switches in one pulse the later stands and a `HEADING` or an
    /// `ORDER` of the same pulse keeps its own key.
    ///
    /// Taken in its own arm of [`apply_intent`] **before the continuous
    /// gates**, on [`IntentKind::Order`]'s precedent: the profile is what the
    /// mind will fly, not motion, so a cold or immobile hull switches. Only a
    /// seat gives one — a mind does not switch its own profile and no station
    /// function builds one, so [`IntentSource::Mind`], `Machine` and `Cond`
    /// are refused [`Refusal::Channel`] — and the arena refuses
    /// [`Refusal::Profile`] a hull with no seal, an index past the sealed set,
    /// the profile already active, or a switch inside 10,000 ms of the last
    /// (D-UM-44's "at most once per 10 s"). A switch writes
    /// [`crate::arena::state::UnitState::card_profile`], the card of a mind row
    /// that stands (dormant under the seat), and one `mind_card` line.
    Profile {
        /// The profile switched to: an index into the hull's sealed set.
        index: u8,
    },
    /// A delivered detachment objective or cancellation, fenced independently
    /// of the recipient's helm epoch and never issued by a unit mind.
    DetachmentCommand(Box<super::detachment::DetachmentCommand>),
}

/// **What a [`IntentKind::Goal`] asks of the motor** (17b cut M1a.4; doc 17 §4
/// D-UM-10).
///
/// The motor ([`crate::arena::motor`]) drives one hull to one goal cell with
/// the bounded route-finding the machines use; this is the whole of what a
/// principal may say to it. A **contact is never a goal**: a goal is ground,
/// and naming a contact's cell would be the motor reading a plot it was never
/// handed (doc 16 §3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MotorGoal {
    /// **Drive to this arena cell**, fanned at landing off a same-side hull's
    /// arrival square inside another side's entry zone
    /// (`crate::arena::state::Arena::fan_goal`).
    Cell {
        /// The column.
        x: u16,
        /// The row.
        y: u16,
    },
    /// **Stop here**: the goal is dropped and the hull ordered to a standstill.
    /// A hold keeps no goal, so it never "arrives", says nothing and schedules
    /// no motor tick — and a hull shoved off the spot does not come back
    /// (simplicity, declared by the cut).
    Hold,
    /// **Let go**: the goal is dropped and the hull keeps whatever heading and
    /// speed it was last ordered.
    Off,
}

impl IntentKind {
    /// Whether this order is a *desired value* rather than an action: continuous
    /// orders are latest-wins and never queue.
    ///
    /// **17b cut M1a.4**: a [`IntentKind::Goal`] is one — a goal is the desired
    /// value of the motor, and a second `DRIVE` replaces the first.
    #[must_use]
    pub const fn is_continuous(&self) -> bool {
        matches!(
            self,
            IntentKind::Heading { .. }
                | IntentKind::Speed { .. }
                | IntentKind::Order(_)
                | IntentKind::ReleaseOrder { .. }
                | IntentKind::Goal(_)
                | IntentKind::Profile { .. }
                | IntentKind::DetachmentCommand(_)
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
    ///
    /// **17b cut M1b.1**: an arming ([`IntentKind::Condition`]) is discrete —
    /// counted on the seat's window, bounded by the task's buffer and resolved
    /// in phase 4 in arrival order with `LOCK` and `FIRE` (see the kind).
    #[must_use]
    pub const fn is_discrete(&self) -> bool {
        matches!(
            self,
            IntentKind::Startup
                | IntentKind::Lock { .. }
                | IntentKind::Fire { .. }
                | IntentKind::Place { .. }
                | IntentKind::Emcon { .. }
                | IntentKind::Condition(_)
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
            | IntentKind::DetachmentCommand(_)
            | IntentKind::Order(_)
            | IntentKind::ReleaseOrder { .. }
            | IntentKind::Place { .. }
            // 17b cut M1a.4: the motor is the helm's drive-by-wire.
            | IntentKind::Goal(_)
            // 17b cut M5.2: a profile is a posture, as an order is.
            | IntentKind::Profile { .. } => Channel::Helm,
            IntentKind::Lock { .. } | IntentKind::Emcon { .. } => Channel::Sensors,
            // 17b cut M1b.1: an arming is the weapons station's.
            IntentKind::Fire { .. } => Channel::Fire,
            IntentKind::Condition(arming) => arming.channel(),
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
            // 17b cut M1b.1: the pilot typed `FIRE <weapon> WHEN …`, so a staged
            // refusal of an arming reads `FIRE cannon refused. …`.
            IntentKind::Fire { .. } => "fire",
            IntentKind::Condition(arming) => match arming.channel() {
                Channel::Sensors => "lock",
                _ => "fire",
            },
            IntentKind::Order(_) => "order",
            IntentKind::DetachmentCommand(_) => "detachment_order",
            IntentKind::ReleaseOrder { .. } => "release",
            IntentKind::Place { .. } => "place",
            IntentKind::Emcon { .. } => "emcon",
            // 17b cut M1a.4: the cockpit's word, not doc 17's `goto` (departure
            // (f)), so the staged refusal names the verb the pilot typed.
            IntentKind::Goal(_) => "drive",
            // 17b cut M5.2: the verb the pilot types (`MIND PROFILE`, 17b cut
            // M5.4), so a staged refusal reads `MIND refused. …`.
            IntentKind::Profile { .. } => "mind",
        }
    }

    /// **An uncited `FIRE` of `weapon`** (17b cut M1b.1): `Fire { weapon, slot:
    /// None }`, a seat's or a machine's shot — the short form for the many
    /// places that build one; the struct literal stays public.
    #[must_use]
    pub fn fire(weapon: &str) -> IntentKind {
        IntentKind::Fire {
            weapon: SmolStr::new(weapon),
            slot: None,
        }
    }
}

/// One order with its authority and its place in the pilot's burst.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
/// world's seam, or a machine's own (the motor's; a mind's, a slot's).
///
/// It decides one thing: whether the order counts against the per-seat rate
/// [`apply_intent`] holds a discrete order to. Pulse phase 2 passes
/// [`IntentSource::Seat`] for every order the world sent; the motor passes
/// [`IntentSource::Machine`] for the orders it gives inside a substep (as the
/// reflex machine did until 17b cut M4.6 deleted it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentSource {
    /// A person at a seat, through the world's seam — counted against
    /// `intents.max_pending_per_unit` per arena second on the unit it orders.
    Seat,
    /// A machine's own order — never counted and never refused for the rate:
    /// the reflex controller's until 17b cut M4.6 deleted it, and since **17b
    /// cut M1a.4** the motor's own heading and speed (`crate::arena::motor`'s
    /// `Arena::apply_motor_plan`), which is why a motor order never takes the
    /// helm back from a `DRIVE` the way a seat's `HEADING` does.
    Machine,
    /// **A mind's landed intent** (17b cut M1a.5; doc 17 D-UM-14): attributed
    /// as the mind's, **never counted on the seat window**, refused `ORDER`,
    /// `RELEASE` and `PLACE` with [`Refusal::Channel`] (D-UM-11, D-UM-52's
    /// second wall: a mind commands nobody, and a staff act is staff's), and
    /// refused [`Refusal::Channel`] on a station its hull lacks (D-UM-47: a
    /// hull with no weapons station grants no `fire` channel). Landed by
    /// `crate::arena::mind`'s landing arm at the epochs its think recorded.
    Mind,
    /// **A condition slot's own shot** (17b cut M1b.1; doc 17 §5, D-UM-14: *a
    /// condition fire is journalled `by: cond` and counted on neither*): the
    /// `FIRE` `Arena::evaluate_conditions` builds when a slot's warrant holds.
    /// **Never counted on the seat window**, and **exempt from the discrete
    /// branch's `Linkdead` gate** — an arming already given stands through a
    /// pilot's linkdead grace as a continuous order already given stands; it
    /// meets every other gate a [`IntentSource::Machine`] order meets.
    Cond,
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
///
/// **17b cut M1a.5**: serde-able, as the payload of a
/// [`crate::arena::perception::FrameNotice::Refused`] in a mind's hashed and
/// carried notice buffer; its words are [`Refusal::as_str`]'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// **An `ORDER` (or a `RELEASE`) for a unit a pilot is flying** (16b cut
    /// N.2; doc 16 §6). A squad's order steers the machine flying the hull, and
    /// a seated tank has none. `NotReflex`, word `not_reflex`, until 17b cut
    /// M4.6 deleted the reflex controller and renamed it, as doc 17 said it
    /// would; the pilot's sentence (`crate::match_::cockpit`) is unchanged.
    #[error("that unit is not flown by a machine")]
    NotMachine,
    /// A delayed persona order has lost its live command hull. This does not
    /// revoke previously accepted orders or turn a mind into an order author.
    #[error("that lance leader has no active command hull")]
    NoCommander,
    /// **`EMCON` for a unit that carries no jamming pod** (16b cut O.2, 16c
    /// N12). The cockpit refuses it locally off the held frame; this is the
    /// word the arena's own refusal path uses when the order gets that far.
    #[error("that unit carries no jamming pod")]
    NoJammer,
    /// **A shot with no warrant** (17b cut M1b.1; doc 17 §5, D-UM-17, D-UM-19):
    /// a mind's `FIRE` that cites no slot, or whose cited slot's warrant terms
    /// do not hold at the landing, and a mind's or a slot's shot under a
    /// standing `hold_fire`. Gated for a slot's own shot from 17b cut M1b.1
    /// (the hold-fire term) and for a MIND's fire from 17b cut M1b.2; a
    /// pilot's typed `FIRE` and a machine's are never gated.
    #[error("that shot has no warrant")]
    Unwarranted,
    /// **An arming past the hull's slot count** (17b cut M1b.1) — the core's
    /// slots since 17b cut M3.2 (`Arena::slot_capacity`, D-UM-16), so reached
    /// only on a hull with fewer slots than weapons; the count's own word, not
    /// the law's.
    #[error("every slot is armed")]
    NoSlot,
    /// **A condition under the match's Compact floor, or naming a target kind
    /// the floor forbids** (17b cut M1b.1; doc 17 §5, D-UM-17 as amended
    /// 2026-10-04), at the landing of an arming or a cited fire:
    /// refuse-not-clamp. Its sentence names the floor
    /// ([`crate::arena::perception::RefusalDetail::Floor`]).
    #[error("that condition is under the Compact floor")]
    Unlawful,
    /// **A profile switch the arena cannot take now** (17b cut M5.2; doc 17
    /// §11, D-UM-44): a [`IntentKind::Profile`] landing on a hull with no
    /// seal, naming an index past its sealed set, naming the profile already
    /// active, or inside 10,000 ms of the last switch. The cockpit refuses
    /// each case locally off the held frame (17b cut M5.4); this is the
    /// arena's word when the order gets that far — the `NoJammer`
    /// precedent. Outside a mind's API (no station function builds a switch),
    /// so `API_VERSION` stands.
    #[error("that profile cannot be switched to now")]
    Profile,
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
            Refusal::NotMachine => "not_machine",
            Refusal::NoCommander => "no_commander",
            Refusal::NoJammer => "no_jammer",
            // 17b cut M1b.1: the law's two words and the count's.
            Refusal::Unwarranted => "unwarranted",
            Refusal::NoSlot => "no_slot",
            Refusal::Unlawful => "unlawful",
            // 17b cut M5.2: a profile switch's one word.
            Refusal::Profile => "profile",
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
///
/// **Who a `Released` hands the hull to** (17b cut M3.5; doc 17 §12 "Release
/// and possess"): [`Controller::Mind`] — since 17b cut M4.3 always, seeded
/// `Withdraw`: the dormant row a possess left, adopted again with its memory,
/// or a fresh one (the reflex controller in `Withdraw` for a hull with no row
/// before 17b cut M4.3; the reflex was deleted in 17b cut M4.6). A `Released`
/// names no seal and touches none: a re-seat after it is a seat activation
/// inside the same sortie.
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
    // station, or the linkdead grace run out. The tank goes to a mind in
    // `Withdraw` (17b cut M4.3; the reflex controller's until then) and drives
    // itself home, which is what K4's "the tank idles" was standing in for
    // until this cut existed. `Live` is the seat a
    // pilot's again — a reclaim inside the grace, a fresh activation, or (16b cut
    // Y.3) a dropped pilot's `BOARD` past it — and takes it back, forgetting the
    // machine. `Linkdead` changes nothing.
    match c.state {
        ControlState::Released => {
            // **16b cut F.1**: the seat is gone, so the arena stops building that
            // pilot a frame. Until this cut `Arena::pilots` was fixed at check-out
            // and nothing ever pruned it, so a pilot who typed `EXIT` in the first
            // minute went on reading contact notices and keyframes — pose, weapons,
            // every contact's cell — for a tank a machine was driving.
            // Doc 16 §3.6 is written about what *a pilot* sees; that person is not
            // one any more. Idempotent: `Released` arrives once per channel.
            if let Some(pilot) = c.pilot {
                arena.drop_pilot(pilot, c.unit);
            }
            // **17b cut M4.3** (the M4 row's finding (5); doc 17 §12; 16b cut
            // N.1): every hull goes to a mind — the dormant row a possess left
            // (`Arena::suspend_mind`, resumed with its memory, strikes and
            // drop intact and its order cleared) or a fresh one — seeded
            // `Withdraw`, so it drives itself home and rallies on Q.7's clock
            // as the reflex machine did (deleted in 17b cut M4.6).
            arena.set_controller(c.unit, Controller::Mind, MachineState::Withdraw);
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
                arena.set_controller(c.unit, Controller::Pilot, MachineState::Withdraw);
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
/// Three continuous kinds are taken in arms of their own BEFORE the next two
/// gates, so neither answers them: a squad's [`IntentKind::Order`] and its
/// [`IntentKind::ReleaseOrder`] (16b cuts N.2 and Z.2 — an order to a cold or
/// immobile tank is how it is told to start up, and still says whether it
/// shoots), and (**17b cut M5.2**) a seat's [`IntentKind::Profile`] (the
/// profile is what the mind will fly, not motion: `apply_profile`).
///
/// Then, for every other **continuous** order: power must be online
/// ([`Refusal::PowerNotOnline`], doc 16 §3.3) and motion must not be disabled
/// ([`Refusal::MotionDisabled`]); the desired value is set, latest-wins. A
/// heading is normalised modulo 360,000. A speed is clamped to
/// `[-max_speed_cms × reverse_factor_pct / 100, max_speed_cms]` — the *order* is
/// clamped to what the unit could ever do, while what it actually reaches each
/// substep is motion's business (doc 16 §3.4 step 2, terrain and slow
/// consequences).
///
/// **17b cut M1a.4**: a continuous [`IntentKind::Goal`] meets those two gates
/// and then, from an [`IntentSource::Seat`], the seat's rate below — the one
/// continuous kind that does; it lands through `land_goal`. And a seat's (or a
/// mind's) `Heading` or `Speed` drops the motor's goal: the principal took the
/// helm back (on any hull but a reflex controller's until 17b cut M4.6).
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
/// unit per arena second** — and, since 17b cut M1a.4, its continuous
/// [`IntentKind::Goal`]s on the same window (the other continuous kinds never
/// count) — counted in
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
/// on the controller instead would leave a gunner still seated on a tank a
/// machine drives — after the driver's `EXIT` hands it over — with no bound at
/// all. The `pending` queue itself has no bound inside a pulse: it is
/// bounded by the caller's slice, which the arena task bounds per unit.
pub fn apply_intent(arena: &mut Arena, i: &Intent, source: IntentSource) -> IntentOutcome {
    // **17b cut M1a.5** (D-UM-11, D-UM-52's second wall): a mind never issues
    // a squad's order, a release of one or a staff place — refused `Channel`
    // before any other gate, because no channel a hull grants carries them for
    // a mind, and so before anything else is read.
    if source == IntentSource::Mind
        && matches!(
            i.kind,
            IntentKind::Order(_)
                | IntentKind::ReleaseOrder { .. }
                | IntentKind::Place { .. }
                | IntentKind::DetachmentCommand(_)
        )
    {
        return IntentOutcome::Refused(Refusal::Channel);
    }
    let Some(found) = arena.unit(i.unit) else {
        return IntentOutcome::Refused(Refusal::Unknown);
    };
    // **17b cut M1a.5** (D-UM-47; doc 17 §22's italic line, *an ungranted
    // channel refuses `Channel`*): a hull with no weapons station grants no
    // `fire` channel, so a mind's `FIRE` there is refused `Channel` before any
    // resolver. A seated pilot's `FIRE` on the same hull keeps its own path
    // and its `NoWeapon` (the cockpit grants the seat all three channels).
    if source == IntentSource::Mind
        && i.kind.channel_required() == Channel::Fire
        && found.weapons.is_empty()
    {
        return IntentOutcome::Refused(Refusal::Channel);
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
    // **17b cut M5.2** — a seat's profile switch (doc 17 §11, D-UM-44), taken
    // here, before the continuous gates, on the order's precedent: the profile
    // is what the mind will fly, not motion, so neither `PowerNotOnline` nor
    // `MotionDisabled` answers it and a cold or immobile hull switches.
    if let IntentKind::Profile { index } = i.kind {
        return apply_profile(arena, i.unit, index, source);
    }
    if let IntentKind::DetachmentCommand(command) = &i.kind {
        return arena.apply_detachment_command(i.unit, command, source);
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
        // **17b cut M1a.5** (D-UM-15 as decided): a pilot's hull is refused
        // `NotMachine` (`NotReflex` until 17b cut M4.6 renamed it); a mind
        // holds the order **re-keyed at write time** to its own HELD contacts,
        // or none (`Arena::rekey_order`) — the only machine since 17b cut M4.6
        // deleted the reflex, which took it as it stood.
        if unit.controller == Controller::Pilot {
            return IntentOutcome::Refused(Refusal::NotMachine);
        }
        if arena.detachment_owns(i.unit) {
            return IntentOutcome::Refused(Refusal::NoCommander);
        }
        // A provider reply may arrive after the commander was wrecked. The
        // world record still names its persona, so recheck the actual arena
        // hull before changing any surviving member's standing order.
        if let crate::deliberation::CharacterRef::Persona { squad } = &order.by {
            let commands = *squad == order.squad
                && arena.squads.iter().any(|roster| {
                    roster.id == *squad && arena.commander_can_order(roster.commander)
                });
            if !commands {
                return IntentOutcome::Refused(Refusal::NoCommander);
            }
        }
        let at = arena.now_ms;
        // **16b cut Y.1**: the hash is asked **before** the order is written —
        // `Arena::event_hash`'s own rule, *every resolver asks before it writes* —
        // so the line carries the state the order landed on, and every `ordered`
        // line of one pulse shares phase 2's one lazy hash (`step.rs` opens
        // `ActionHash::Pending` around the intent loop).
        let hash = arena.event_hash();
        let standing = crate::deliberation::order::StandingOrder::accepted((**order).clone(), at);
        let rekeyed = arena.rekey_order(i.unit, &standing.order, standing.expires_ms);
        // **17b cut M5.2** (the design note's item 5): a row laid here flies
        // the hull's active sealed profile (`Arena::active_card`; the Line
        // card for a default seal, byte for byte `MindState::fresh`), as
        // `Arena::adopt_mind` lays one. Latent: every mind hull is handed its
        // row at the check-out or its release, and no row is ever removed.
        if !arena.minds.contains_key(&i.unit) {
            let seal_digest = arena
                .unit(i.unit)
                .map_or([0; 32], |state| state.seal.digest);
            arena.minds.insert(
                i.unit,
                crate::arena::mind::MindState {
                    seal_digest,
                    ..crate::arena::mind::MindState::fresh(&arena.arena_cfg.reflex)
                },
            );
        }
        let Some(row) = arena.minds.get_mut(&i.unit) else {
            return IntentOutcome::Refused(Refusal::Unknown);
        };
        let watch_expiry = rekeyed.expires_ms;
        row.order = Some(rekeyed);
        // **17b cut M4.6** (the owner's STOP C ruling, 2026-10-09): the issuer
        // beside the order, written here where it lands — from the same
        // `Order` the `ordered` line below journals — carried by the snapshot
        // and never folded (`MindState::order_issuer`), so the hash asked above
        // is the one the line would carry without it.
        row.order_issuer = order.issuer;
        // **17b cut M5.2** (doc 17 §24's STOP E, the owner's remedy): the
        // charge of a `Support` beside it, from the same `Order` — the unit,
        // not this hull's row for it, so the arena ends the order when the
        // charge leaves the match whether or not the supporter held it
        // classified here (`Arena::end_mind_support_whose_charge_left`). A
        // self-charge is `Engage` (`Arena::rekey_order`) and holds none.
        // Folded inside the order's `Support` bytes, so the hash asked above
        // is the one the line would carry without it.
        row.order_charge = match order.objective {
            crate::deliberation::order::Objective::Support { of } if of != i.unit => Some(of),
            _ => None,
        };
        arena.note_accepted_order(i.unit, order, watch_expiry);
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
        // 17b cut M1a.5: a release reaches a mind's order through one helper;
        // a pilot's hull keeps its refusal (`NotMachine` since 17b cut M4.6).
        if unit.controller == Controller::Pilot {
            return IntentOutcome::Refused(Refusal::NotMachine);
        }
        if arena.detachment_owns(i.unit) {
            return IntentOutcome::Refused(Refusal::NoCommander);
        }
        release_machine_order(arena, i.unit, *squad, *issuer);
        return IntentOutcome::AppliedContinuous;
    }
    if i.kind.is_continuous() {
        if unit.power != Power::Online {
            return IntentOutcome::Refused(Refusal::PowerNotOnline);
        }
        if unit.limits.motion_disabled {
            return IntentOutcome::Refused(Refusal::MotionDisabled);
        }
        // **17b cut M1a.4** — the seat's rate on a goal. A `Goal { Cell }`
        // spends a path search from the budget every hull shares, so a seat
        // that could send one per keystroke could starve the field's searches;
        // it is counted on the same per-unit window a discrete order is, and
        // refused `QueueFull` past it. A mind's and the motor's own goals are
        // never counted (D-UM-14), for the reason the machine's discrete
        // orders are not (this function's head).
        if matches!(i.kind, IntentKind::Goal(_)) && source == IntentSource::Seat {
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
        match &i.kind {
            IntentKind::Heading { mdeg } => unit.desired_heading_mdeg = mdeg % 360_000,
            IntentKind::Speed { cms } => {
                let forward = i32::try_from(unit.mobility.max_speed_cms).unwrap_or(i32::MAX);
                let reverse = i32::try_from(unit.mobility.reverse_ceiling_cms())
                    .unwrap_or(i32::MAX)
                    .saturating_neg();
                unit.desired_speed_cms = (*cms).clamp(reverse, forward);
            }
            // 17b cut M1a.4: a hold is a stop (the motor row is written below,
            // once this borrow of the unit has ended).
            IntentKind::Goal(MotorGoal::Hold) => unit.desired_speed_cms = 0,
            IntentKind::Goal(MotorGoal::Cell { .. } | MotorGoal::Off)
            | IntentKind::Startup
            | IntentKind::Lock { .. }
            | IntentKind::Fire { .. }
            | IntentKind::Order(_)
            | IntentKind::DetachmentCommand(_)
            | IntentKind::ReleaseOrder { .. }
            | IntentKind::Place { .. }
            | IntentKind::Emcon { .. }
            | IntentKind::Condition(_)
            // 17b cut M5.2: taken in its own arm above.
            | IntentKind::Profile { .. } => {}
        }
        match &i.kind {
            IntentKind::Goal(goal) => land_goal(arena, i.unit, *goal, source),
            // **17b cut M1a.4 — the principal takes the helm back** (D-UM-10:
            // *`HEADING` or `SPEED` typed afterward disengages it*). A seat's
            // heading or speed lets go of the motor's goal as `DRIVE OFF`
            // does, so the hull steers what was typed. The motor's own orders
            // (`IntentSource::Machine`) never do — they are how the goal is
            // driven. **17b cut M1a.5**: a mind's own heading or speed lets go
            // of its goal the same way — the mind took the helm back from its
            // own motor. (A reflex hull's goals were its machine's, which a
            // seat's order did not reach, until 17b cut M4.6 deleted it.)
            IntentKind::Heading { .. } | IntentKind::Speed { .. }
                if matches!(source, IntentSource::Seat | IntentSource::Mind)
                    && arena.motors.row(i.unit).is_some_and(|motor| {
                        motor.goal.is_some() || motor.requested_goal.is_some()
                    }) =>
            {
                arena.motors.forget_goal(i.unit);
            }
            _ => {}
        }
        return IntentOutcome::AppliedContinuous;
    }
    // **16b cut I.3**: nor does a linkdead seat refuse a staff place — a
    // linkdead pilot's wedged tank is the other stuck unit staff reach for, and
    // nobody is at the seat to have given the order anyway.
    //
    // **17b cut M1b.1**: nor a condition slot's own shot
    // (`IntentSource::Cond`): an arming already given stands through a pilot's
    // linkdead grace, as a continuous order already given stands — the slot is
    // the principal's decision, written down before the link dropped.
    if unit.linkdead && !place && source != IntentSource::Cond {
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

/// **The least time between two profile switches of one hull**, arena
/// milliseconds (17b cut M5.2; doc 17 D-UM-44: "at most once per 10 s"),
/// checked at the switch's landing against
/// [`crate::arena::card::ActiveProfile::switched_ms`].
pub const PROFILE_SWITCH_MS: i64 = 10_000;

/// **Apply one seat's profile switch** (17b cut M5.2; doc 17 §11, §12,
/// D-UM-44), for [`apply_intent`]'s [`IntentKind::Profile`] arm once the unit,
/// the epoch, the channel and the wreck have been read.
///
/// Refused, in this order: [`Refusal::Channel`] from anything but an
/// [`IntentSource::Seat`] (a mind does not switch its own profile; no
/// station function builds one); then [`Refusal::Profile`] for a hull with no
/// seal ([`crate::arena::mind::SealRef::none`]'s zero digest), an `index` not
/// under the sealed set's `len`, the profile already active, or a landing
/// inside [`PROFILE_SWITCH_MS`] of the last switch.
///
/// Taken, in this order: the hash asked **before** anything is written
/// (`Arena::event_hash`'s rule — under phase 2's one lazy hash); the hull's
/// [`crate::arena::state::UnitState::card_profile`] written (`index`, stamped
/// `now_ms`); the card of a mind row that STANDS — dormant under the seat
/// (`Arena::suspend_mind`) — written to the preset the profile names, over
/// the arena's own doctrine ([`crate::arena::card::preset`]), so the release
/// adopts the row flying it; and ONE [`EventKind::MindCard`] line at `now_ms`
/// with the card's name, the sliders the switch moved (from the row's card,
/// or from the profile it switched from when no row stands) and the whole
/// card. A hull with no row is laid one on the active profile at its release
/// (`Arena::adopt_mind`).
///
/// **The row it writes is a dormant one** (an invariant, not a check): only a
/// seat's switch is taken, and a seat's helm epoch is current only while the
/// hull is [`Controller::Pilot`]'s — a `Released` raises the epoch, so a seat
/// cannot reach a hull a mind is flying, and staff `@arena/intent` builds no
/// `Profile`. A standing row here is therefore always the dormant row of a
/// seated hull (`Arena::suspend_mind`), never a flying mind's, whose card is
/// `@mind/live`'s lane alone. 17b cut M5.4's `MIND PROFILE` forwards from the
/// helm seat only, which keeps it so. (The `af1` shape walk applies a seat's
/// switch to a mind-flown hull directly, a state no production path reaches,
/// to put the switched profile under the pin.)
fn apply_profile(arena: &mut Arena, unit: ObjId, index: u8, source: IntentSource) -> IntentOutcome {
    if source != IntentSource::Seat {
        return IntentOutcome::Refused(Refusal::Channel);
    }
    let now_ms = arena.now_ms;
    let Some(own) = arena.unit(unit) else {
        return IntentOutcome::Refused(Refusal::Unknown);
    };
    if own.seal.digest == crate::arena::mind::SealRef::none().digest {
        return IntentOutcome::Refused(Refusal::Profile);
    }
    let Some(runtime) = arena.profiles.get(&unit) else {
        return IntentOutcome::Refused(Refusal::Profile);
    };
    let ids = own
        .seal
        .weapons
        .iter()
        .map(|weapon| weapon.id.clone())
        .collect::<Vec<_>>();
    let profile = arena.classes.row(own.seal.class);
    let prepared = match runtime.prepare_switch(
        &arena.cards,
        unit,
        index,
        now_ms,
        &profile,
        &ids,
        &own.seal.channels,
        arena.odds_floor_permille,
    ) {
        Ok(prepared) => prepared,
        Err(_) => return IntentOutcome::Refused(Refusal::Profile),
    };
    let Some(key) = runtime.profiles.key(index) else {
        return IntentOutcome::Refused(Refusal::Profile);
    };
    let Ok(next) = arena.cards.resolve(key) else {
        return IntentOutcome::Refused(Refusal::Profile);
    };
    let name = SmolStr::new(runtime.profiles.name(index).unwrap_or("profile"));
    let changed = next
        .values()
        .into_iter()
        .flat_map(|values| values.keys())
        .filter(|key| next.get(key) != runtime.card.get(key))
        .cloned()
        .collect();
    let hash = arena.event_hash();
    let Some(runtime) = arena.profiles.get_mut(&unit) else {
        return IntentOutcome::Refused(Refusal::Profile);
    };
    if runtime.commit(&mut arena.conditions, prepared).is_err() {
        return IntentOutcome::Refused(Refusal::Profile);
    }
    arena.constrain_profile_hardware(unit);
    arena.cancel_profile_think(unit);
    if let Some(own) = arena.unit_mut(unit) {
        own.card_profile = crate::arena::card::ActiveProfile {
            index,
            switched_ms: Some(now_ms),
        };
    }
    let _ = arena.ledger.append(
        now_ms,
        EventKind::MindCard {
            unit,
            name,
            changed,
            card: key,
            stock: next.stock().cloned(),
        },
        Vec::new(),
        hash,
    );
    IntentOutcome::AppliedContinuous
}

/// **Apply one unit's release** (16b cut Z.2; 16c Q44), for [`apply_intent`]'s
/// [`IntentKind::ReleaseOrder`] arm once the unit is known to be a machine's —
/// a mind's since **17b cut M1a.5** (one helper for the reflex controller's and
/// a mind's until 17b cut M4.6 deleted the reflex, so a release read the same
/// against either).
///
/// For a mind (D-UM-15), the order is the [`crate::arena::perception::FrameOrder`]
/// on its [`crate::arena::mind::MindState`], matched by its rendered squad word;
/// dropping it — with its issuer, 17b cut M4.6 — is the whole of the
/// restoration — a mind's goal is its own and a release does not touch its motor
/// row — and the line and the squad's mark are the machine's.
///
/// A unit under no order, or under an order written for another squad, applies
/// nothing and journals nothing: a release names a squad, and a machine the
/// squad's commander has not ordered is not the pilot's to hand back. Otherwise,
/// in this order:
///
/// 1. the hash is asked **before** anything is written — `Arena::event_hash`'s
///    rule, so under phase 2's `Pending` slot every `ordered` and
///    `order_released` line of one pulse shares the phase's one hash;
/// 2. the mind's held order and its issuer are dropped (the reflex machine
///    dropped to doctrine here through its expiry block's own restoration, its
///    motor row's goal forgotten, until 17b cut M4.6 deleted it);
/// 3. one `order_released` line at `now_ms`;
/// 4. **the squad is marked for its next signal only when no unit of its roster
///    still holds an order written for it** (a wreck's leftover order does not
///    count: a wreck is no subject of any order) — so the next signal says
///    `OrderExpired` once the whole squad is off orders, and a wingman's release
///    of the one machine of their pair does not hand the lead's squad to the LLM
///    seam while the lead's order still runs on the rest of it. The read is
///    bounded by the roster (at most `SQUAD_SIZE_MAX` units).
fn release_machine_order(
    arena: &mut Arena,
    unit: ObjId,
    squad: crate::deliberation::SquadId,
    issuer: Option<ObjId>,
) {
    if !holds_order_for(arena, unit, squad) {
        return;
    }
    // `holds_order_for` reads a row only under `Controller::Mind`, so the
    // unit is a mind's here.
    let at = arena.now_ms;
    let hash = arena.event_hash();
    if let Some(state) = arena.minds.get_mut(&unit) {
        state.order = None;
        // 17b cut M4.6 (the owner's STOP C): the issuer goes with the order,
        // and (17b cut M5.2, STOP E) the charge.
        state.order_issuer = None;
        state.order_charge = None;
    }
    arena.clear_order_watch_member(unit);
    journal_released(arena, unit, squad, issuer, at, hash);
}

/// **Whether `unit` holds an order written for `squad`** (17b cut M1a.5): a
/// mind's held one (matched by the squad's rendered word, which is how a
/// [`crate::arena::perception::FrameOrder`] carries it; a reflex machine's
/// standing order too until 17b cut M4.6).
///
/// **17b cut M3.5** (doc 17 §12 "Release and possess"): the mind's row is
/// read only while the hull IS the mind's ([`Controller::Mind`]) — a possess
/// keeps the row dormant (`Arena::suspend_mind`), and a dormant row's held
/// order is not the pilot's: it neither keeps the squad marked as ordered
/// nor gives a release anything to drop.
fn holds_order_for(arena: &Arena, unit: ObjId, squad: crate::deliberation::SquadId) -> bool {
    arena
        .unit(unit)
        .is_some_and(|state| state.controller == Controller::Mind)
        && arena
            .minds
            .get(&unit)
            .and_then(|state| state.order.as_ref())
            .is_some_and(|order| order.squad == squad.to_string())
}

/// **The `order_released` line and the squad's mark** (16b cut Z.2; one body
/// for both machines since 17b cut M1a.5) — steps 3 and 4 of
/// [`release_machine_order`].
fn journal_released(
    arena: &mut Arena,
    unit: ObjId,
    squad: crate::deliberation::SquadId,
    issuer: Option<ObjId>,
    at: i64,
    hash: u64,
) {
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
                    && holds_order_for(arena, *member, squad)
            })
        });
    if !still_ordered {
        arena.note_order_expired(squad);
    }
}

/// **Write one landed goal onto the hull's motor row** (17b cut M1a.4; doc 17
/// §4 D-UM-10), for [`apply_intent`]'s [`IntentKind::Goal`] arm once the
/// continuous gates have passed.
///
/// * [`MotorGoal::Cell`]: the cell is fanned off a same-side arrival square
///   (`Arena::fan_goal`), the row is laid down if the hull has none, the goal
///   written, the route and the remembered stop dropped — and the search stamp
///   dropped **only for a seat's goal**: a typed order gets its search now,
///   while a mind's or the machine's own goal keeps `searched_ms`, so the
///   `repath_ms` throttle the reflex controller has always lived under holds
///   for a mind that re-names its goal every think (M1a.5 and M1a.6 land
///   through here). Then the hull is put on the motor's cadence
///   (`Arena::engage_motor`).
/// * [`MotorGoal::Hold`] and [`MotorGoal::Off`]: the goal, the route and the
///   remembered stop go ([`crate::arena::motor::MotorTable::forget_goal`]); a
///   hold's stop was written by the caller. A hull with no row gains none.
///
/// (A goal to a reflex hull was written like any other and left to its
/// machine's next tick, with nothing put on the motor's cadence for it, until
/// 17b cut M4.6 deleted the reflex controller.)
fn land_goal(arena: &mut Arena, unit: ObjId, goal: MotorGoal, source: IntentSource) {
    match goal {
        MotorGoal::Cell { x, y } => {
            // Reasserting a live request or its successful completion is
            // idempotent. A failed request may be retried explicitly.
            if arena.motors.row(unit).is_some_and(|motor| {
                motor.requested_goal == Some((x, y))
                    && (motor.goal.is_some()
                        || motor.outcome == Some(crate::arena::perception::DriveOutcome::Arrived))
            }) {
                return;
            }
            let cell = arena.fan_goal(unit, (x, y));
            let motor = arena.motors.ensure(unit);
            motor.goal = Some(cell);
            motor.fan = crate::arena::motor::GoalFan::between((x, y), cell);
            motor.requested_goal = Some((x, y));
            motor.outcome = None;
            motor.route.clear();
            motor.refused = None;
            if source == IntentSource::Seat {
                motor.searched_ms = None;
            }
            arena.engage_motor(unit);
        }
        MotorGoal::Hold | MotorGoal::Off => arena.motors.forget_goal(unit),
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
/// pilot's `STARTUP`; a machine's landing passes its own millisecond (the
/// reflex controller passed its tick's own timestamp until 17b cut M4.6),
/// which is earlier than the substep end `now_ms` had already been advanced to
/// when it ran.
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

/// **Resolve `EMCON on|off`** (16b cut O.2, 16c N12): switch the unit's jamming
/// pod, in pulse phase 4 beside [`resolve_startup`] and in its shape.
///
/// A **no-op** — the switch already where the order puts it — answers `Ok(())`
/// and journals and stages nothing. The cockpit refuses the no-op locally
/// (`EMCON is already on.`), reading its own hint before the mirror; one still
/// reaches here when two pilots of one tank race, or when a pilot's hint and
/// the arena disagree, and the arena path journals **real flips only**, so the
/// tape never carries a switch that switched nothing.
///
/// A **flip**, in this order:
///
/// 1. the hash is asked **before** anything is written — `Arena::event_hash`'s
///    rule, *every resolver asks before it writes* — so the line shares phase
///    4's one hash with every other action the phase resolves;
/// 2. `ecm_on` is written ([`crate::arena::state::UnitState::ecm_on`]'s one
///    writer);
/// 3. [`Arena::jam_fold`] is cleared, because it memoises a function of the
///    switch: a memo that outlived a flip would give the rest of the substep the
///    old answer;
/// 4. one `emcon` line at `at_ms`;
/// 5. one [`crate::arena::perception::FrameNotice::Emcon`] to the unit's own
///    pilots, carrying the pod's radius in whole metres.
///
/// **No floor on flips** (16b cut O plan item 38). A pilot flipping at the
/// seat's rate — `max_pending_per_unit` a second — writes that many `emcon`
/// lines, but it costs each observer inside the bubble **at most one
/// `receiver` transition and one notice per `sensors.pass_ms`**: an observer's
/// receiver is written only at that observer's own sensor pass, and a
/// transition is journalled and told only when its `is_some()` changes
/// ([`Arena::sensor_pass`]'s rule), so every flip between two of its passes
/// collapses into whatever the switch reads at the next. That is the accepted
/// bound and the ledger note records it.
///
/// # Errors
///
/// [`Refusal::Unknown`] for a unit this arena does not hold, [`Refusal::Wreck`]
/// for a wreck, [`Refusal::NoJammer`] for a unit that carries no pod. A ledger
/// that has reached its byte bound does not fail the flip, for the reason it
/// does not fail [`resolve_startup`]'s.
pub fn resolve_emcon(arena: &mut Arena, unit: ObjId, on: bool, at_ms: i64) -> Result<(), Refusal> {
    let Some(state) = arena.unit(unit) else {
        return Err(Refusal::Unknown);
    };
    if state.limits.wreck {
        return Err(Refusal::Wreck);
    }
    let Some(jammer) = state.jammer else {
        return Err(Refusal::NoJammer);
    };
    if state.ecm_on == on {
        return Ok(());
    }
    let hash = arena.event_hash();
    if let Some(state) = arena.unit_mut(unit) {
        state.ecm_on = on;
    }
    arena.jam_fold.clear();
    let _ = arena
        .ledger
        .append(at_ms, EventKind::Emcon { unit, on }, Vec::new(), hash);
    arena.stage_notice(
        unit,
        crate::arena::perception::FrameNotice::Emcon {
            on,
            radius_m: jammer.radius_cm / 100,
        },
    );
    Ok(())
}

impl Arena {
    /// **An order re-keyed to one hull's own contacts** (17b cut M1a.3; doc 17 §3
    /// "Two more fields"; D-UM-15 **as decided**: *re-keyed at write time to the
    /// unit's own held `ContactId` or none*).
    ///
    /// An [`crate::deliberation::order::Order`] names contacts in its
    /// **issuer's** scope, which is somebody else's — a pilot's tank or the squad
    /// commander's. A hull must only ever read a row of its own, so each contact
    /// the order names is resolved where it was written and then looked for in
    /// this hull's store:
    ///
    /// * `Engage` — the order's preferred contact
    ///   ([`crate::deliberation::order::Order::preferred_contact`]) is resolved in
    ///   the issuer's scope (`order.issuer`, else the squad commander's) exactly as
    ///   the reflex's `ordered_target` resolved it (deleted in 17b cut M4.6), to a
    ///   target; the answer is
    ///   **this hull's own CLASSIFIED row for that target**, else `None` —
    ///   D-UM-17's rule applied to the hint (17b cut M1b.3, the lead's ruling on
    ///   M1a's residual (2), [`Arena::held_classified`]): an `ORDER ENGAGE BA`
    ///   tells a mind nothing of which unknown row is BA until its own sensors
    ///   classify it. **`rekey_order` runs once, at the order's landing, and
    ///   the SAME order is not re-keyed when the row classifies later** — T0's
    ///   `ordered_target` re-resolves its hint every tick, so this is the
    ///   declared divergence `order_hint_late` (doc 17 §24); a late re-key
    ///   needs the hint's object kept on the mind row, a 16c candidate and not
    ///   this cut's. (A reflex hull's FRAME re-keyed at every build —
    ///   [`Arena::frame_order`]'s reflex arm, until 17b cut M4.6 — so its frame
    ///   read the row once classified; the declaration is the mind's.)
    ///   Never the pure id `draw_contact_id` would mint: that is the row the hull
    ///   *will* hold the target under, and writing it for a target the hull does
    ///   not yet hold would link a later `X<n>` detection to the ordered target —
    ///   identity before classification, which no pilot hearing `ENGAGE BA` has
    ///   (the review's finding; the plan's departure (c) withdrawn).
    /// * `Support { of }` — this hull's own CLASSIFIED row for `of`, `None`
    ///   when it holds none or holds it unclassified (17b cut M1b.3, the same
    ///   filter). **A self-charge** (`of` is the hull itself) is re-keyed to
    ///   `Engage { target }` through the `Engage` arm's own resolution (17b cut
    ///   L.2; D-UM-15, the plan's Q-L6): a unit whose `of` is itself fights on
    ///   doctrine — the `Objective::Support` doc's own rule — which is `Engage`'s
    ///   semantics, and the hint resolved is the order's preferred contact
    ///   (`Order.target` for a `Support`), the same one the reflex's
    ///   `ordered_target` fell through to on a self-charge, so a hinted
    ///   self-charge hunts what the reflex controller would have hunted and an unhinted
    ///   one is `Engage { target: None }`. Before cut L.2 it was `Support { of:
    ///   None }`, under which a mind stood as a blind supporter (M1a.6's residual
    ///   (3)). Declared: a mind holding it journals `engage` when it runs out
    ///   (`crate::arena::mind`'s expiry reads the held objective's word) where
    ///   its `ordered` line (and a reflex hull's expiry, until 17b cut M4.6) say
    ///   `support`.
    /// * `Hold`, `Advance`, `Withdraw`, `Screen` — copied; they name no contact.
    ///
    /// `MindObjective`'s shape is unchanged by 17b cut M1b.3: a target on
    /// `Engage` and a charge on `Support`, as built, and nothing else names a
    /// contact.
    ///
    /// The constraints are copied flat, the squad is rendered (`red-1`), and
    /// `expires_ms` is the caller's — the standing order's arena deadline.
    #[must_use]
    pub(crate) fn rekey_order(
        &self,
        unit: ObjId,
        order: &crate::deliberation::order::Order,
        expires_ms: i64,
    ) -> crate::arena::perception::FrameOrder {
        let viewer = order
            .issuer
            .or_else(|| self.squad_of(unit).map(|squad| squad.commander));
        let mut objective = order.objective;
        // The legacy target field may override the objective's hint.
        if matches!(
            objective,
            crate::deliberation::order::Objective::Engage { .. }
        ) || matches!(objective, crate::deliberation::order::Objective::Support { of } if of == unit)
        {
            objective = crate::deliberation::order::Objective::Engage {
                target_hint: order.preferred_contact(),
            };
        }
        self.rekey_objective(
            unit,
            viewer,
            order.squad,
            super::detachment::DetachmentObjective {
                objective,
                constraints: order.constraints,
            },
            expires_ms,
        )
    }

    /// Shared observation boundary for legacy and detachment doctrine.
    pub(crate) fn rekey_objective(
        &self,
        unit: ObjId,
        viewer: Option<ObjId>,
        squad: crate::deliberation::SquadId,
        directive: super::detachment::DetachmentObjective,
        expires_ms: i64,
    ) -> crate::arena::perception::FrameOrder {
        use crate::arena::perception::{FrameOrder, MindObjective};
        use crate::deliberation::order::Objective;
        // The `Engage` arm's resolution, lifted so a self-charged `Support`
        // (17b cut L.2) resolves its hint by the same steps: the preferred
        // contact, in the issuer's scope or the squad commander's, to a target,
        // then this hull's own row for it or none — and, since 17b cut M1b.3
        // (D-UM-17 applied to the hint), only a row it has CLASSIFIED.
        let resolve = || {
            let hint = match directive.objective {
                Objective::Engage { target_hint } => target_hint,
                _ => None,
            };
            hint.and_then(|hint| {
                let target = self.contacts.resolve(viewer?, hint)?.target;
                self.contacts
                    .id_of(unit, target)
                    .filter(|id| self.held_classified(unit, *id))
            })
        };
        let objective = match directive.objective {
            Objective::Hold => MindObjective::Hold,
            Objective::Advance { to } => MindObjective::Advance { to },
            Objective::Engage { .. } => MindObjective::Engage { target: resolve() },
            Objective::Withdraw => MindObjective::Withdraw,
            Objective::Screen { facing_mdeg } => MindObjective::Screen { facing_mdeg },
            // 17b cut L.2: a self-charge fights on doctrine, which is `Engage`.
            Objective::Support { of } if of == unit => MindObjective::Engage { target: resolve() },
            // 17b cut M1b.3: the charge's own row, classified, or none.
            Objective::Support { of } => MindObjective::Support {
                of: self
                    .contacts
                    .id_of(unit, of)
                    .filter(|id| self.held_classified(unit, *id)),
            },
        };
        FrameOrder {
            identity: None,
            squad: SmolStr::new(squad.to_string()),
            objective,
            hold_fire: directive.constraints.hold_fire,
            max_range_band: directive.constraints.max_range_band,
            expires_ms,
        }
    }

    /// **Whether `unit` holds row `id` and has told it apart** (17b cut
    /// M1b.3; D-UM-17's classified-only rule): the one predicate
    /// [`Arena::rekey_order`] keeps a row by.
    #[must_use]
    pub(crate) fn held_classified(&self, unit: ObjId, id: ContactId) -> bool {
        self.contacts
            .get(unit, id)
            .is_some_and(|held| matches!(held.class, Classification::Classified { .. }))
    }

    /// **The standing order a hull's frame carries** (17b cut M1a.3; doc 17 §3),
    /// re-keyed by `Arena::rekey_order`, or `None`.
    ///
    /// * A [`Controller::Pilot`] hull: `None`. A pilot is not under a squad's
    ///   standing order on this surface, so every frame a pilot is sent carries
    ///   `"order": null`.
    /// * A [`Controller::Mind`] hull (17b cut M1a.5): the order the mind holds,
    ///   re-keyed when it was **written** (D-UM-15 as decided) and read back
    ///   as it was written — never its issuer (`MindState::order_issuer`, 17b
    ///   cut M4.6), which is a record field and rides no frame.
    ///
    /// (A reflex hull's frame carried its machine's standing order re-keyed
    /// **at frame time**, with no reader, until 17b cut M4.6 deleted the
    /// reflex controller and that arm.)
    #[must_use]
    pub fn frame_order(&self, unit: ObjId) -> Option<crate::arena::perception::FrameOrder> {
        match self.unit(unit)?.controller {
            Controller::Pilot => None,
            Controller::Mind => self.minds.get(&unit).and_then(|state| state.order.clone()),
        }
    }
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
                weapon: SmolStr::new("cannon"),
                slot: None,
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
                weapon: SmolStr::new("cannon"),
                slot: None,
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
                            weapon: SmolStr::new("cannon"),
                            slot: None,
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
                        weapon: SmolStr::new("cannon"),
                        slot: None,
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
                    slot: None,
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
    /// handed, not at `now_ms` — a machine's landing at 1,009 ms (a reflex
    /// tick's, until 17b cut M4.6) inside a substep whose end `now_ms` already
    /// reads 1,250 starts up at 1,009.
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
            // 16b cut O.2.
            Refusal::NoJammer,
        ]
        .map(|refusal| refusal.as_str());
        let unique: std::collections::BTreeSet<&str> = words.iter().copied().collect();
        assert_eq!(unique.len(), words.len());
        assert!(words.iter().all(|word| !word.is_empty()));
        assert_eq!(ControlState::Linkdead.as_str(), "linkdead");
        assert_eq!(Refusal::NoJammer.as_str(), "no_jammer");
    }

    // -----------------------------------------------------------------------
    // 16b cut O.2 — `EMCON`, the pod's switch (16c N12)
    // -----------------------------------------------------------------------

    /// A pod on RED: 100 m of bubble, which reaches BLUE 71 m off on the
    /// diagonal, and the radius a flip's notice carries as 100.
    fn o2_pod() -> crate::arena::perception::Jammer {
        crate::arena::perception::Jammer {
            radius_cm: 10_000,
            attenuation_cost: 25,
            signature_permille: 150,
        }
    }

    /// Two tanks, RED carrying [`o2_pod`] with the match's bubble radius set as
    /// `Arena::new` would have resolved it.
    fn o2_arena() -> Arena {
        let mut arena = two_tanks(1);
        if let Some(state) = arena.unit_mut(RED) {
            state.jammer = Some(o2_pod());
        }
        arena.jam_radius_cm = o2_pod().radius_cm;
        arena
    }

    /// [`o2_arena`] with RED's power up, so its pod radiates, and the index
    /// built as phase 3(b) leaves it.
    fn o2_radiating() -> Arena {
        let mut arena = o2_arena();
        online(&mut arena, RED);
        let units = arena.units.clone();
        arena.index.rebuild(&arena.definition.terrain, &units);
        arena.jam_fold.clear();
        arena
    }

    /// Every line the ledger holds, flushed, as `(t_ms, kind word, json)`.
    fn o2_lines(arena: &mut Arena) -> Vec<(i64, &'static str, String)> {
        y1_lines(arena)
            .into_iter()
            .map(|event| {
                (
                    event.t_ms,
                    event.kind.as_str(),
                    serde_json::to_string(&event.kind).unwrap_or_default(),
                )
            })
            .collect()
    }

    /// **The kind**: `EMCON` is the ninth arm — discrete, on the sensors
    /// channel, word `emcon` — so it queues for phase 4 and is counted by the
    /// seat's rate like any pilot's action.
    #[test]
    fn o2_emcon_is_a_discrete_order_on_the_sensors_channel() {
        let kind = IntentKind::Emcon { on: false };
        assert!(kind.is_discrete());
        assert!(!kind.is_continuous());
        assert_eq!(kind.channel_required(), Channel::Sensors);
        assert_eq!(kind.as_str(), "emcon");
        assert_eq!(IntentKind::Emcon { on: true }.as_str(), "emcon");
        // Refused on the helm, as a gunner's grant is not a driver's.
        let mut arena = o2_arena();
        assert_eq!(
            apply_intent(
                &mut arena,
                &Intent {
                    unit: RED,
                    channel: Channel::Helm,
                    epoch: 1,
                    kind: IntentKind::Emcon { on: false },
                    seq: 1,
                },
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Channel)
        );
        // Queued on the sensors channel whatever the power — the switch is a
        // position — and counted against the seat's rate.
        assert_eq!(arena.unit(RED).map(|state| state.power), Some(Power::Off));
        let bound = arena.arena_cfg.intents.max_pending_per_unit;
        for seq in 0..u64::from(bound) {
            assert_eq!(
                apply_intent(
                    &mut arena,
                    &intent(RED, 1, seq, IntentKind::Emcon { on: seq % 2 == 1 }),
                    IntentSource::Seat
                ),
                IntentOutcome::Queued,
                "order {seq}"
            );
        }
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 99, IntentKind::Emcon { on: false }),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::QueueFull),
            "the seat's rate counts EMCON like any action"
        );
    }

    /// **The pilot's gates**: a wreck and a linkdead seat refuse `EMCON` at
    /// apply, as they refuse every discrete order a pilot gives.
    #[test]
    fn o2_emcon_is_refused_on_a_wreck_and_on_a_linkdead_seat() {
        let mut wreck = o2_arena();
        if let Some(state) = wreck.unit_mut(RED) {
            state.limits.wreck = true;
        }
        assert_eq!(
            apply_intent(
                &mut wreck,
                &intent(RED, 1, 1, IntentKind::Emcon { on: false }),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Wreck)
        );
        let mut dead = o2_arena();
        apply_control(
            &mut dead,
            &Control {
                unit: RED,
                channel: Channel::Sensors,
                epoch: 1,
                state: ControlState::Linkdead,
                pilot: None,
            },
        );
        assert_eq!(
            apply_intent(
                &mut dead,
                &intent(RED, 1, 1, IntentKind::Emcon { on: false }),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::Linkdead)
        );
        assert!(dead.pending.is_empty() && wreck.pending.is_empty());
    }

    /// **`resolve_emcon`'s three refusals**, each writing, journalling and
    /// staging nothing: no such unit, a wreck, and a unit with no pod.
    #[test]
    fn o2_resolve_emcon_refuses_an_unknown_unit_a_wreck_and_a_unit_with_no_pod() {
        let mut arena = o2_arena();
        let before = arena.ledger.seq();
        assert_eq!(
            resolve_emcon(&mut arena, ObjId(77), false, 0),
            Err(Refusal::Unknown)
        );
        assert_eq!(
            resolve_emcon(&mut arena, BLUE, false, 0),
            Err(Refusal::NoJammer),
            "BLUE carries no pod"
        );
        assert_eq!(arena.unit(BLUE).map(|state| state.ecm_on), Some(true));
        if let Some(state) = arena.unit_mut(RED) {
            state.limits.wreck = true;
        }
        assert_eq!(
            resolve_emcon(&mut arena, RED, false, 0),
            Err(Refusal::Wreck)
        );
        assert_eq!(arena.unit(RED).map(|state| state.ecm_on), Some(true));
        assert_eq!(arena.ledger.seq(), before, "a refusal journals nothing");
        assert!(arena.pending_notices.is_empty(), "and stages nothing");
    }

    /// **A no-op journals and stages nothing**: the switch already where the
    /// order puts it answers `Ok(())` and leaves every trace as it was — the
    /// memo included.
    #[test]
    fn o2_a_no_op_emcon_journals_nothing_and_stages_nothing() {
        let mut arena = o2_radiating();
        let mut counters = crate::arena::perception::PerceptionCounters::default();
        let folded = arena.jam_fold_of(BLUE, &mut counters);
        assert!(folded.is_some(), "BLUE stands in RED's bubble");
        let before = arena.ledger.seq();
        assert_eq!(resolve_emcon(&mut arena, RED, true, 0), Ok(()));
        assert_eq!(arena.unit(RED).map(|state| state.ecm_on), Some(true));
        assert_eq!(arena.ledger.seq(), before, "a no-op journals nothing");
        assert!(arena.pending_notices.is_empty(), "a no-op stages nothing");
        assert_eq!(
            arena.jam_fold.get(&BLUE),
            Some(&folded),
            "a no-op leaves the memo alone"
        );
    }

    /// **The flip**: it writes the switch, clears the memo, journals one
    /// `emcon` line at the millisecond it is handed under the phase's hash
    /// (asked before the write), and stages one notice with the pod's radius in
    /// whole metres — each way.
    #[test]
    fn o2_a_flip_writes_clears_the_memo_journals_one_line_and_stages_one_notice() {
        use crate::arena::perception::FrameNotice;

        let mut arena = o2_radiating();
        arena.now_ms = 12_000;
        let mut counters = crate::arena::perception::PerceptionCounters::default();
        assert!(arena.jam_fold_of(BLUE, &mut counters).is_some());
        assert!(!arena.jam_fold.is_empty());
        // The hash the line must carry: the state before the write.
        let expected = crate::arena::hash::state_hash(&arena);
        assert_eq!(resolve_emcon(&mut arena, RED, false, 10_000), Ok(()));
        assert_eq!(arena.unit(RED).map(|state| state.ecm_on), Some(false));
        assert!(arena.jam_fold.is_empty(), "the flip cleared the memo");
        assert_eq!(
            arena.jam_fold_of(BLUE, &mut counters),
            None,
            "and the next fold reads the silent pod"
        );
        assert_eq!(
            arena.pending_notices.get(&RED),
            Some(&vec![FrameNotice::Emcon {
                on: false,
                radius_m: 100
            }])
        );
        assert!(
            !arena.pending_notices.contains_key(&BLUE),
            "nobody else is told"
        );
        let lines = y1_lines(&mut arena);
        let emcon: Vec<&crate::arena::ledger::LedgerEvent> = lines
            .iter()
            .filter(|event| event.kind.as_str() == "emcon")
            .collect();
        assert_eq!(emcon.len(), 1, "{lines:?}");
        assert_eq!(emcon.first().map(|event| event.t_ms), Some(10_000));
        assert_eq!(emcon.first().map(|event| event.hash), Some(expected));
        assert_eq!(
            emcon.first().map(|event| event.kind.clone()),
            Some(EventKind::Emcon {
                unit: RED,
                on: false
            })
        );
        // And back on: one more line, one more notice.
        arena.pending_notices.clear();
        assert_eq!(resolve_emcon(&mut arena, RED, true, 11_000), Ok(()));
        assert_eq!(arena.unit(RED).map(|state| state.ecm_on), Some(true));
        assert_eq!(
            arena.pending_notices.get(&RED),
            Some(&vec![FrameNotice::Emcon {
                on: true,
                radius_m: 100
            }])
        );
        let words: Vec<(i64, &'static str, String)> = o2_lines(&mut arena);
        assert_eq!(
            words
                .iter()
                .filter(|(_, word, _)| *word == "emcon")
                .map(|(t_ms, _, json)| (*t_ms, json.clone()))
                .collect::<Vec<_>>(),
            vec![(
                11_000,
                format!("{{\"kind\":\"emcon\",\"unit\":{},\"on\":true}}", RED.0)
            )]
        );
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
    /// live — and a refused order (here a seated tank's, `NotMachine`;
    /// `NotReflex` until 17b cut M4.6) journals
    /// nothing at all.
    ///
    /// **17b cut M4.5a**: the two hulls are minds (the controller byte alone;
    /// the order's Mind arm lays each row). Every line field is the ORDER's,
    /// so both lines are unchanged in every field; the order standing on each
    /// mind row is the `FrameOrder` re-keyed at write time — the issuer #77
    /// holds no row, so the hint resolves to `Engage { target: None }` while
    /// the line still carries the order's own hint — and "stamped at
    /// `now_ms`" reads through its deadline (`expires_ms = now_ms + ttl`,
    /// `StandingOrder::accepted`'s; a `FrameOrder` carries no stamp).
    #[test]
    fn y1_an_order_journals_one_ordered_line_per_unit_and_a_refused_one_nothing() {
        let mut arena = two_tanks(1);
        arena.now_ms = 7_000;
        for unit in [RED, BLUE] {
            if let Some(state) = arena.unit_mut(unit) {
                state.controller = Controller::Mind;
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
            // 17b cut L.1: the pilot's character, never read by the arena.
            by: crate::deliberation::order::CharacterRef::Character {
                id: mush_core::ObjId(9_001),
            },
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
        // The order stands on both minds, stamped at `now_ms` (read through its
        // deadline: 7,000 + the ttl 600,000).
        for unit in [RED, BLUE] {
            let held = arena.minds.get(&unit).and_then(|mind| mind.order.clone());
            assert_eq!(
                held.as_ref().map(|order| order.expires_ms - 600_000),
                Some(7_000)
            );
            assert_eq!(
                held,
                Some(crate::arena::perception::FrameOrder {
                    identity: None,
                    squad: SmolStr::new("red-1"),
                    objective: crate::arena::perception::MindObjective::Engage { target: None },
                    hold_fire: true,
                    max_range_band: None,
                    expires_ms: 607_000,
                }),
                "re-keyed at write time: the issuer holds no row for the hint"
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
            IntentOutcome::Refused(Refusal::NotMachine)
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
            // 17b cut M4.5a: since M4.3 a `Released` hands the hull to a mind,
            // whose row a reclaim keeps dormant (17b cut M3.5) and never reads —
            // so "no machine survived the reclaim" is that no machine reads the
            // hull. (No order is written here; the possess's clear is
            // `m4_3_a_possess_clears_the_dormant_order`'s.)
            assert!(
                arena.machine_state(RED).is_none(),
                "order {n}: a machine still reads the hull after the reclaim"
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
        // 17b cut M4.3: a release hands the hull to a mind.
        assert_eq!(
            arena.unit(RED).map(|unit| unit.controller),
            Some(Controller::Mind)
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
        // 17b cut M4.5a: no machine reads the hull (the mind's row from the
        // first drop is dormant under the pilot).
        assert!(arena.machine_state(RED).is_none());
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
            Some(Controller::Mind)
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
                // 17b cut L.1: the pilot's character, never read by the arena.
                by: crate::deliberation::order::CharacterRef::Character {
                    id: mush_core::ObjId(9_001),
                },
                spot: None,
            },
            0,
        )
    }

    /// Two red machines in squad `red-1`, both under its `Hold`, and a blue tank a
    /// pilot flies.
    ///
    /// 17b cut M4.5a: the two machines are minds (`set_controller` from the
    /// pilot lays each row and schedules a think no fixture here steps to),
    /// each holding the `Hold` re-keyed as the order's Mind arm writes it.
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
            arena.set_controller(unit, Controller::Mind, MachineState::Patrol);
            let standing = z2_order(crate::deliberation::order::Objective::Hold);
            let held = arena.rekey_order(unit, &standing.order, standing.expires_ms);
            arena.minds.get_mut(&unit).expect("the mind's row").order = Some(held);
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

    /// The order `unit`'s mind row holds (17b cut M4.5a: what
    /// `ReflexMemory::order` read for the reflex machine).
    fn z2_held(arena: &Arena, unit: ObjId) -> Option<&crate::arena::perception::FrameOrder> {
        arena.minds.get(&unit).and_then(|mind| mind.order.as_ref())
    }

    /// **16b cut Z.2 (16c Q44)**: `ReleaseOrder` is a continuous `Helm` order
    /// with the word `release`; on a machine under that squad's order it drops
    /// the order to doctrine and journals one `order_released` line carrying the
    /// issuer, at `now_ms`; under another squad's order, or none, it applies
    /// nothing and journals nothing; and on a pilot's tank it is `NotMachine`
    /// (`NotReflex` until 17b cut M4.6).
    /// With two machines of one squad under its order, releasing one does not
    /// mark the squad for its next signal and releasing the second does.
    ///
    /// **17b cut M4.5a**: the two machines are minds (`z2_arena`), and every
    /// read is the mind row's held order — the release's Mind arm drops it
    /// (`release_machine_order`), matched by the squad's rendered word; the
    /// line, the squad's mark and the wreck's skip are the one body both
    /// machines share (`journal_released`).
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
        // 17b cut M4.5a: a drive the machine was making when the order is
        // handed back. The reflex's `drop_to_doctrine` forgot the goal and the
        // route with the order; a mind's release leaves the motor row to the
        // mind's next think (the release's Mind arm touches the order alone),
        // a declared divergence pinned below.
        {
            let motor = arena.motors.ensure(RED);
            motor.goal = Some((1, 1));
            motor.route = vec![(2, 2), (1, 1)];
        }
        let before = arena.ledger.seq();
        // Another squad's word: nothing.
        assert_eq!(
            apply_intent(&mut arena, &z2_release(RED, 2, 1), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert!(
            z2_held(&arena, RED).is_some(),
            "red-2's release is not red-1's"
        );
        // The squad's own word on one machine: that one drops.
        assert_eq!(
            apply_intent(&mut arena, &z2_release(RED, 1, 2), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(z2_held(&arena, RED), None);
        assert_eq!(
            arena
                .motors
                .row(RED)
                .map(|motor| (motor.goal, motor.route.len())),
            Some((Some((1, 1)), 2)),
            "a mind's release leaves its drive to its next think"
        );
        assert!(
            z2_held(&arena, WING).is_some(),
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
            IntentOutcome::Refused(Refusal::NotMachine)
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
        assert_eq!(z2_held(&arena, WING), None);
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
        assert!(z2_held(&arena, WING).is_some(), "the wreck's row keeps it");
        assert_eq!(
            apply_intent(&mut arena, &z2_release(RED, 1, 2), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert!(
            arena.squads_expired.contains(&z2_squad(1)),
            "a wreck's leftover order is no order the squad is under"
        );
    }

    // -----------------------------------------------------------------------
    // 17b cut M1a.3 — the order re-keyed (doc 17 §3, D-UM-15 as decided)
    // -----------------------------------------------------------------------

    /// **17b cut M1a.3** (D-UM-15 as decided): an order's contacts are resolved
    /// in the scope they were written in — the issuer's, else the squad
    /// commander's, exactly as `ordered_target` resolves a hint — and re-keyed to
    /// the receiving hull's OWN held row for that target, or `None` when it holds
    /// none (never the id it WOULD hold the target under: identity before
    /// classification). `Support` names the hull's own row for the charge, and
    /// `None` for itself. `frame_order` is `None` on a piloted hull and the
    /// re-keyed standing order on a machine.
    ///
    /// **17b cut M4.5a**: RED and WING are minds (as `z2_arena`'s), and a
    /// mind's order is re-keyed when it is WRITTEN (the order's Mind arm;
    /// D-UM-15 as decided) and read back as written — where the reflex's was
    /// re-keyed at frame time. The leg that tells the two apart: the pilot's
    /// `ENGAGE BA` lands on WING through `apply_intent` while WING holds BLUE as
    /// `WING_SEES_BA`, and after that row is removed WING's frame still names
    /// it (a frame-time re-key would answer `None`, as `rekey_order` then
    /// does). The dormant row of a piloted hull stands in for the reflex row's
    /// order on it, and the release of a squad's order (its Mind arm) for
    /// `drop_to_doctrine`.
    #[test]
    fn m1a3_rekey_order_resolves_the_hint_in_the_issuers_scope_and_re_keys_it_to_a_held_row_or_none()
     {
        use crate::arena::perception::{FrameOrder, MindObjective};
        use crate::arena::state::{SeenPose, ShadowContact};
        use crate::deliberation::order::{Constraints, Objective, Order, RangeBand};
        /// The piloted tank a pilot typed `ORDER` in.
        const PILOT: ObjId = ObjId(6);
        /// The pilot's row for BLUE — the hint `ENGAGE BA` carries.
        const PILOT_SAYS_BA: ContactId = ContactId(0x77);
        /// The commander's row for BLUE.
        const LEAD_SAYS_BA: ContactId = ContactId(0x55);
        /// WING's own rows for BLUE and for RED.
        const WING_SEES_BA: ContactId = ContactId(0x33);
        const WING_SEES_RA: ContactId = ContactId(0x44);
        // `z2_arena`'s three tanks and a fourth, red and piloted.
        let mut arena = arena_at_epoch(
            vec![
                tank(RED, Team::Red, pose(1_500, 1_500)),
                tank(WING, Team::Red, pose(2_500, 1_500)),
                tank(PILOT, Team::Red, pose(3_500, 1_500)),
                tank(BLUE, Team::Blue, pose(6_500, 6_500)),
            ],
            1,
        );
        for unit in [RED, WING] {
            arena.set_controller(unit, Controller::Mind, MachineState::Patrol);
            let standing = z2_order(Objective::Hold);
            let held = arena.rekey_order(unit, &standing.order, standing.expires_ms);
            arena.minds.get_mut(&unit).expect("the mind's row").order = Some(held);
        }
        arena.squads = vec![crate::deliberation::squad::SquadRoster {
            id: z2_squad(1),
            commander: RED,
            units: vec![RED, WING],
        }];
        let hold = |arena: &mut Arena, viewer: ObjId, id: ContactId, target: ObjId| {
            let team = arena.unit(target).map_or(Team::Blue, |unit| unit.team);
            let seen = SeenPose::of(arena.unit(target).unwrap_or_else(|| panic!("{target}")));
            arena.contacts.insert(
                viewer,
                id,
                ShadowContact::sighted(target, Classification::Classified { team }, 0, seen),
            );
        };
        hold(&mut arena, PILOT, PILOT_SAYS_BA, BLUE);
        hold(&mut arena, RED, LEAD_SAYS_BA, BLUE);
        hold(&mut arena, WING, WING_SEES_BA, BLUE);
        hold(&mut arena, WING, WING_SEES_RA, RED);
        let order = |objective: Objective, issuer: Option<ObjId>| Order {
            squad: z2_squad(1),
            objective,
            target: None,
            horizon_ms: 600_000,
            constraints: Constraints {
                hold_fire: true,
                max_range_band: Some(RangeBand::Effective),
            },
            ttl_ms: 600_000,
            issued_ms: 0,
            issuer,
            // 17b cut L.1: a pilot's character with an issuer, the leader without.
            by: issuer.map_or(
                crate::deliberation::order::CharacterRef::Persona { squad: z2_squad(1) },
                |_| crate::deliberation::order::CharacterRef::Character {
                    id: mush_core::ObjId(9_001),
                },
            ),
            spot: None,
        };
        let engage = |hint: ContactId| Objective::Engage {
            target_hint: Some(hint),
        };

        // A pilot's `ORDER … ENGAGE BA`: the pilot's row resolves to BLUE, and
        // WING reads its OWN row for BLUE. The flat fields are copied.
        let told = order(engage(PILOT_SAYS_BA), Some(PILOT));
        assert_eq!(
            arena.rekey_order(WING, &told, 54_250),
            FrameOrder {
                identity: None,
                squad: SmolStr::new_static("red-1"),
                objective: MindObjective::Engage {
                    target: Some(WING_SEES_BA)
                },
                hold_fire: true,
                max_range_band: Some(RangeBand::Effective),
                expires_ms: 54_250,
            }
        );
        // The commander holds BLUE under its own id: that one.
        assert_eq!(
            arena.rekey_order(RED, &told, 0).objective,
            MindObjective::Engage {
                target: Some(LEAD_SAYS_BA)
            }
        );
        // `issuer: None` resolves in the squad commander's store: the
        // commander's id names BLUE there, and the pilot's id names nothing.
        assert_eq!(
            arena
                .rekey_order(WING, &order(engage(LEAD_SAYS_BA), None), 0)
                .objective,
            MindObjective::Engage {
                target: Some(WING_SEES_BA)
            }
        );
        assert_eq!(
            arena
                .rekey_order(WING, &order(engage(PILOT_SAYS_BA), None), 0)
                .objective,
            MindObjective::Engage { target: None },
            "the pilot's row is not the commander's"
        );
        // `SUPPORT RA`: WING's own row for RED; RED itself is self-charged.
        let support = order(Objective::Support { of: RED }, Some(PILOT));
        assert_eq!(
            arena.rekey_order(WING, &support, 0).objective,
            MindObjective::Support {
                of: Some(WING_SEES_RA)
            }
        );
        // 17b cut L.2 (Q-L6, D-UM-15; M1a.6's residual (3)): a self-charge is
        // re-keyed through the Engage arm — unhinted here, so `None`.
        assert_eq!(
            arena.rekey_order(RED, &support, 0).objective,
            MindObjective::Engage { target: None },
            "a self-charge fights on doctrine"
        );
        // 17b cut M4.5a: the pilot's `ENGAGE BA` written onto WING (a mind)
        // through the order's own arm, while WING still holds BLUE — re-keyed
        // at write time to WING's own row.
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(WING, 1, 1, IntentKind::Order(Box::new(told.clone()))),
                IntentSource::Seat
            ),
            IntentOutcome::AppliedContinuous
        );
        let written = arena.frame_order(WING);
        assert_eq!(
            written.as_ref().map(|order| order.objective),
            Some(MindObjective::Engage {
                target: Some(WING_SEES_BA)
            }),
            "written re-keyed to WING's own row"
        );
        // A hull that does NOT hold the ordered target re-keys to nothing — not
        // to the id it would mint for BLUE later.
        arena.contacts.remove(WING, WING_SEES_BA);
        arena.contacts.remove(WING, WING_SEES_RA);
        assert_eq!(
            arena.rekey_order(WING, &told, 0).objective,
            MindObjective::Engage { target: None }
        );
        // ...and the order WING already holds keeps the row it was keyed to:
        // a mind's order is re-keyed when written, never again at frame time.
        assert_eq!(
            arena.frame_order(WING),
            written,
            "the written target survives the removal of its row"
        );
        assert_eq!(
            arena.rekey_order(WING, &support, 0).objective,
            MindObjective::Support { of: None }
        );
        // The four that name no contact are copied.
        for (objective, expected) in [
            (Objective::Hold, MindObjective::Hold),
            (
                Objective::Advance { to: (3, 4) },
                MindObjective::Advance { to: (3, 4) },
            ),
            (Objective::Withdraw, MindObjective::Withdraw),
            (
                Objective::Screen {
                    facing_mdeg: 90_000,
                },
                MindObjective::Screen {
                    facing_mdeg: 90_000,
                },
            ),
        ] {
            let rekeyed = arena.rekey_order(WING, &order(objective, None), 0);
            assert_eq!(rekeyed.objective, expected);
            assert_eq!(rekeyed.objective.as_str(), objective.as_str());
            assert_eq!(rekeyed.objective.code(), objective.code());
        }

        // `frame_order`: a machine under an order reads it re-keyed (17b cut
        // M4.5a: a mind's, as it was written — RED's `Hold` names no contact,
        // and WING's `Engage` still names the row it was keyed to); a piloted
        // hull reads nothing, even with an order on its row.
        let standing = z2_order(Objective::Hold);
        assert_eq!(
            arena.frame_order(RED),
            Some(arena.rekey_order(RED, &standing.order, standing.expires_ms))
        );
        assert!(written.is_some(), "WING is under the pilot's ENGAGE");
        // And `frame_for` carries exactly that — the field 17b cut M1a.5's
        // `mind_view` reads, wired, not a literal `None`.
        assert_eq!(
            arena
                .frame_for(WING, crate::arena::clock::Tempo::Engaged)
                .map(|frame| frame.order),
            Some(written),
            "the frame's order is frame_order's"
        );
        // 17b cut M4.5a: an order on a piloted hull's row is a dormant mind
        // row's (a possess keeps it, 17b cut M3.5).
        let dormant = z2_order(Objective::Withdraw);
        let dormant = arena.rekey_order(PILOT, &dormant.order, dormant.expires_ms);
        arena
            .minds
            .entry(PILOT)
            .or_insert_with(|| crate::arena::mind::MindState::fresh(&arena.arena_cfg.reflex))
            .order = Some(dormant);
        assert_eq!(
            arena.unit(PILOT).map(|unit| unit.controller),
            Some(Controller::Pilot)
        );
        assert_eq!(arena.frame_order(PILOT), None, "a pilot's frame says null");
        // A machine with no order reads none (17b cut M4.5a: RED's `Hold`
        // released through the release's Mind arm).
        assert_eq!(
            apply_intent(&mut arena, &z2_release(RED, 1, 2), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(arena.frame_order(RED), None);
    }

    // -----------------------------------------------------------------------
    // 17b cut M1a.4 — the goal (doc 17 §4 D-UM-3, D-UM-10)
    // -----------------------------------------------------------------------

    /// How many motor ticks the heap holds for `unit`.
    fn m1a4_motor_ticks(arena: &Arena, unit: ObjId) -> usize {
        arena
            .due
            .iter()
            .filter(|event| event.kind == DueKind::MotorTick { unit })
            .count()
    }

    #[test]
    fn m5r_reasserted_goals_keep_completion_until_a_real_override() {
        use crate::arena::perception::DriveOutcome;

        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        let requested = MotorGoal::Cell { x: 5, y: 5 };
        land_goal(&mut arena, RED, requested, IntentSource::Seat);
        arena.motors.ensure(RED).route = vec![(2, 2), (3, 3)];
        land_goal(&mut arena, RED, requested, IntentSource::Seat);
        assert_eq!(arena.motors.ensure(RED).route, vec![(2, 2), (3, 3)]);
        assert_eq!(m1a4_motor_ticks(&arena, RED), 1);

        let completed = arena.motors.ensure(RED);
        completed.goal = None;
        completed.route.clear();
        completed.outcome = Some(DriveOutcome::Arrived);
        land_goal(&mut arena, RED, requested, IntentSource::Seat);
        let completed = arena.motors.ensure(RED);
        assert_eq!(completed.requested_goal, Some((5, 5)));
        assert_eq!(completed.goal, None);
        assert_eq!(completed.outcome, Some(DriveOutcome::Arrived));

        // A machine's steering is the motor itself, so it keeps completion;
        // a principal's heading disengages even when the active goal is spent.
        let heading = intent(RED, 1, 1, IntentKind::Heading { mdeg: 90_000 });
        assert_eq!(
            apply_intent(&mut arena, &heading, IntentSource::Machine),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(
            arena.motors.ensure(RED).outcome,
            Some(DriveOutcome::Arrived)
        );
        assert_eq!(
            apply_intent(&mut arena, &heading, IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        let cleared = arena.motors.ensure(RED);
        assert_eq!(cleared.requested_goal, None);
        assert_eq!(cleared.outcome, None);

        // Failure can be retried at the same requested cell.
        let failed = arena.motors.ensure(RED);
        failed.requested_goal = Some((5, 5));
        failed.outcome = Some(DriveOutcome::NoRoute);
        land_goal(&mut arena, RED, requested, IntentSource::Seat);
        let retried = arena.motors.ensure(RED);
        assert_eq!(retried.requested_goal, Some((5, 5)));
        assert_eq!(retried.goal, Some((5, 5)));
        assert_eq!(retried.outcome, None);
    }

    #[test]
    fn m5r_fanned_goals_keep_their_lane_until_zone_exit_or_disengagement() {
        use crate::arena::motor::GoalFan;
        use crate::arena::perception::DriveOutcome;

        let mut arena = two_tanks(1);
        arena.unit_mut(BLUE).expect("second hull").team = Team::Red;
        arena.motors.ensure(BLUE).goal = Some((6, 3));
        land_goal(
            &mut arena,
            RED,
            MotorGoal::Cell { x: 6, y: 3 },
            IntentSource::Seat,
        );
        let first = arena.motors.row(RED).expect("fanned hull");
        assert_eq!(first.goal, Some((6, 6)));
        assert_eq!(first.requested_goal, Some((6, 3)));
        assert_eq!(first.fan, GoalFan::between((6, 3), (6, 6)));

        // The physical destination is spent at arrival, but the next logical
        // waypoint inside this zone must retain the private three-row lane.
        arena.motors.release(BLUE);
        let arrived = arena.motors.ensure(RED);
        arrived.goal = None;
        arrived.outcome = Some(DriveOutcome::Arrived);
        land_goal(
            &mut arena,
            RED,
            MotorGoal::Cell { x: 7, y: 3 },
            IntentSource::Seat,
        );
        let continued = arena.motors.row(RED).expect("continued drive");
        assert_eq!(continued.goal, Some((7, 6)));
        assert_eq!(continued.requested_goal, Some((7, 3)));
        assert_eq!(continued.fan, GoalFan::between((7, 3), (7, 6)));
        assert_eq!(continued.outcome, None);

        // A retry keeps the lane; a normal goal outside its original zone does
        // not carry the offset into unrelated ground.
        let failed = arena.motors.ensure(RED);
        failed.goal = None;
        failed.outcome = Some(DriveOutcome::NoRoute);
        land_goal(
            &mut arena,
            RED,
            MotorGoal::Cell { x: 7, y: 3 },
            IntentSource::Seat,
        );
        assert_eq!(arena.motors.row(RED).and_then(|row| row.goal), Some((7, 6)));
        land_goal(
            &mut arena,
            RED,
            MotorGoal::Cell { x: 3, y: 3 },
            IntentSource::Seat,
        );
        let exited = arena.motors.row(RED).expect("normal drive");
        assert_eq!(exited.goal, Some((3, 3)));
        assert_eq!(exited.fan, None);
        for goal in [MotorGoal::Hold, MotorGoal::Off] {
            arena.motors.ensure(RED).fan = GoalFan::between((6, 3), (6, 6));
            land_goal(&mut arena, RED, goal, IntentSource::Seat);
            assert_eq!(arena.motors.row(RED).and_then(|row| row.fan), None);
        }
        // An offset falling outside the zone cannot clamp an unrelated request
        // into the lane or escape the zone's bounds.
        arena.motors.ensure(RED).fan = GoalFan::between((6, 3), (6, 6));
        land_goal(
            &mut arena,
            RED,
            MotorGoal::Cell { x: 6, y: 6 },
            IntentSource::Seat,
        );
        assert_eq!(arena.motors.row(RED).and_then(|row| row.goal), Some((6, 6)));
        assert_eq!(arena.motors.row(RED).and_then(|row| row.fan), None);
    }

    /// **The goal lands on the motor row, a seat's helm order takes it back,
    /// and the gates and the seat's rate hold** (17b cut M1a.4; doc 17 §4
    /// D-UM-10, D-UM-14).
    ///
    /// The kind: continuous, on the helm, word `drive`. On a pilot's powered
    /// tank: a seat's `Cell` writes the goal, drops the route, the remembered
    /// stop and the search stamp, and puts ONE motor tick on the heap (a second
    /// goal rides it); a machine's `Cell` keeps the search stamp; a machine's
    /// `Heading` keeps the goal and a seat's drops it; `Hold` drops it and
    /// zeroes the ordered speed, `Off` drops it and leaves the speed. A hull with
    /// no row gains none from a `Hold`. The three continuous gates refuse a
    /// goal by name, and the phase-2 line they stage reads `DRIVE refused. …`.
    /// A seat's goals are counted on the per-unit window — the one past the
    /// bound is `QueueFull` — and a machine's are not. (17b cut M4.5a: the two
    /// reflex-machine legs are deleted with the reflex decision; their NOT
    /// DONE note is re-homed in the body.)
    #[test]
    fn m1a4_a_goal_sets_the_motors_goal_and_a_seat_helm_order_clears_it() {
        use crate::arena::motor::Refusal as Stop;
        let goal =
            |unit: ObjId, kind: MotorGoal, seq: u64| intent(unit, 1, seq, IntentKind::Goal(kind));
        let cell = |x: u16, y: u16| MotorGoal::Cell { x, y };

        // The kind.
        let kind = IntentKind::Goal(MotorGoal::Off);
        assert!(kind.is_continuous());
        assert!(!kind.is_discrete());
        assert_eq!(kind.channel_required(), Channel::Helm);
        assert_eq!(kind.as_str(), "drive");

        // A seat's cell, on a pilot's powered tank with a stale row.
        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        {
            let motor = arena.motors.ensure(RED);
            motor.route = vec![(1, 1)];
            motor.searched_ms = Some(500);
            motor.refused = Some(Stop {
                at: (1, 1),
                cell: (2, 2),
                cause: crate::arena::motion::BlockedCause::Impassable,
                goal: (3, 3),
                until_ms: 30_000,
            });
        }
        assert_eq!(
            apply_intent(&mut arena, &goal(RED, cell(5, 5), 1), IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        let row = arena.motors.row(RED).cloned().unwrap_or_default();
        assert_eq!(row.goal, Some((5, 5)));
        assert!(row.route.is_empty() && row.refused.is_none());
        assert_eq!(row.searched_ms, None, "a typed goal gets its search now");
        assert_eq!(m1a4_motor_ticks(&arena, RED), 1, "one motor tick");
        assert!(arena.pending.is_empty(), "a goal never queues");
        // A machine's goal keeps the stamp, and rides the same tick.
        if let Some(motor) = arena.motors.row_mut(RED) {
            motor.searched_ms = Some(700);
        }
        assert_eq!(
            apply_intent(&mut arena, &goal(RED, cell(6, 6), 2), IntentSource::Machine),
            IntentOutcome::AppliedContinuous
        );
        let row = arena.motors.row(RED).cloned().unwrap_or_default();
        assert_eq!((row.goal, row.searched_ms), (Some((6, 6)), Some(700)));
        assert_eq!(m1a4_motor_ticks(&arena, RED), 1, "still one tick");

        // The motor's own heading keeps the goal; a seat's takes it back.
        let heading = |seq| intent(RED, 1, seq, IntentKind::Heading { mdeg: 90_000 });
        assert_eq!(
            apply_intent(&mut arena, &heading(3), IntentSource::Machine),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(arena.motors.row(RED).and_then(|m| m.goal), Some((6, 6)));
        // A heading the machine's own did not give, so the typed one shows.
        let typed = intent(RED, 1, 4, IntentKind::Heading { mdeg: 180_000 });
        assert_eq!(
            apply_intent(&mut arena, &typed, IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(arena.motors.row(RED).and_then(|m| m.goal), None);
        assert_eq!(
            arena.unit(RED).map(|unit| unit.desired_heading_mdeg),
            Some(180_000),
            "and steers what was typed"
        );
        // A seat's speed does too.
        let _ = apply_intent(&mut arena, &goal(RED, cell(5, 5), 5), IntentSource::Seat);
        let _ = apply_intent(
            &mut arena,
            &intent(RED, 1, 6, IntentKind::Speed { cms: 300 }),
            IntentSource::Seat,
        );
        assert_eq!(arena.motors.row(RED).and_then(|m| m.goal), None);

        // Hold: no goal, a stop. Off: no goal, the speed left alone.
        let _ = apply_intent(&mut arena, &goal(RED, cell(5, 5), 7), IntentSource::Seat);
        assert_eq!(
            apply_intent(
                &mut arena,
                &goal(RED, MotorGoal::Hold, 8),
                IntentSource::Seat
            ),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(arena.motors.row(RED).and_then(|m| m.goal), None);
        assert_eq!(arena.unit(RED).map(|unit| unit.desired_speed_cms), Some(0));
        let _ = apply_intent(
            &mut arena,
            &intent(RED, 1, 9, IntentKind::Speed { cms: 300 }),
            IntentSource::Seat,
        );
        let _ = apply_intent(&mut arena, &goal(RED, cell(5, 5), 10), IntentSource::Seat);
        assert_eq!(
            apply_intent(
                &mut arena,
                &goal(RED, MotorGoal::Off, 11),
                IntentSource::Seat
            ),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(arena.motors.row(RED).and_then(|m| m.goal), None);
        assert_eq!(
            arena.unit(RED).map(|unit| unit.desired_speed_cms),
            Some(300)
        );
        // A hold on a hull nothing steers lays no row down.
        online(&mut arena, BLUE);
        let _ = apply_intent(
            &mut arena,
            &goal(BLUE, MotorGoal::Hold, 12),
            IntentSource::Seat,
        );
        assert!(arena.motors.row(BLUE).is_none());

        // The gates, by name, and the line phase 2 stages for each.
        for (refusal, line) in [
            (Refusal::Wreck, "DRIVE refused. Your tank is a wreck."),
            (
                Refusal::MotionDisabled,
                "DRIVE refused. Your tank cannot move.",
            ),
            (
                Refusal::PowerNotOnline,
                "DRIVE refused. Power not online: STARTUP first.",
            ),
        ] {
            let mut arena = two_tanks(1);
            online(&mut arena, RED);
            if let Some(state) = arena.unit_mut(RED) {
                match refusal {
                    Refusal::Wreck => state.limits.wreck = true,
                    Refusal::MotionDisabled => state.limits.motion_disabled = true,
                    _ => state.power = Power::Off,
                }
            }
            assert_eq!(
                apply_intent(
                    &mut arena.clone(),
                    &goal(RED, cell(5, 5), 1),
                    IntentSource::Seat
                ),
                IntentOutcome::Refused(refusal)
            );
            let out = arena.step(0, &[], &[goal(RED, cell(5, 5), 1)]);
            let told: Vec<String> = out
                .frames
                .iter()
                .filter(|(_, frame)| frame.unit == RED)
                .flat_map(|(_, frame)| crate::match_::cockpit::notice_lines(frame))
                .collect();
            assert!(
                told.iter().any(|said| said == line),
                "{refusal:?}: {told:?}"
            );
            assert!(arena.motors.row(RED).is_none_or(|m| m.goal.is_none()));
            assert_eq!(m1a4_motor_ticks(&arena, RED), 0);
        }

        // 17b cut M4.5a: the two reflex legs that stood here (a goal to a
        // reflex machine written with nothing on the motor's cadence; a
        // patrolling machine's next tick overwriting it, a withdrawing one's
        // keeping it as its route home) are deleted: they pinned the reflex
        // DECISION's handling of a found goal, which ceases to exist, and a
        // seat's goal on a mind hull is a fixture-only path a pin would cement.
        // Their note, re-homed (the cut's NOT DONE, for the lead): no seat
        // reaches a machine hull's helm today (a mind holds no pilot's lease);
        // the day a seam can put a goal on one, what becomes of it is the
        // mind's — the goal lands as any other (`land_goal`, one motor tick
        // on the heap, since `engage_motor` skips only a reflex hull) and
        // stands until the mind's next landing names a DIFFERENT cell (the
        // stock mind re-emits a goal only when it changes) — and that rule is
        // decided then, not pinned now.

        // The seat's rate counts a seat's goal and never a machine's.
        let mut rated = two_tanks(1);
        online(&mut rated, RED);
        let bound = rated.arena_cfg.intents.max_pending_per_unit;
        for seq in 0..u64::from(bound) {
            assert_eq!(
                apply_intent(&mut rated, &goal(RED, cell(5, 5), seq), IntentSource::Seat),
                IntentOutcome::AppliedContinuous,
                "goal {seq}"
            );
        }
        assert_eq!(
            apply_intent(&mut rated, &goal(RED, cell(5, 5), 99), IntentSource::Seat),
            IntentOutcome::Refused(Refusal::QueueFull)
        );
        let mut free = two_tanks(1);
        online(&mut free, RED);
        for seq in 0..u64::from(bound).saturating_mul(2) {
            assert_eq!(
                apply_intent(
                    &mut free,
                    &goal(RED, cell(5, 5), seq),
                    IntentSource::Machine
                ),
                IntentOutcome::AppliedContinuous
            );
        }
        assert_eq!(free.unit(RED).map(|unit| unit.orders_in_window), Some(0));
    }

    /// **17b cut M1b.1**: an arming is a discrete `fire` order on the fire
    /// channel, serde-able as `pending` carries it; a `Fire`'s `slot` is absent
    /// on the wire when `None` and defaults when read; and a condition slot's
    /// own shot (`IntentSource::Cond`) is never counted on the seat window and
    /// never refused `Linkdead`.
    #[test]
    fn m1b1_an_arming_is_discrete_and_a_slot_shot_is_counted_on_nothing() {
        use crate::arena::conditions::{ASPECTS_ANY, Arming, FireCondition, SlotTarget};
        let arming = IntentKind::Condition(Arming::Arm(FireCondition {
            weapon: SmolStr::new("cannon"),
            target: SlotTarget::BestHostile,
            min_permille: 100,
            aspects: ASPECTS_ANY,
            min_rounds_left: 2,
            min_range_m: 0,
            max_range_m: 600,
        }));
        assert!(arming.is_discrete());
        assert!(!arming.is_continuous());
        assert_eq!(arming.channel_required(), Channel::Fire);
        assert_eq!(arming.as_str(), "fire");
        let text = serde_json::to_string(&arming).expect("an arming serialises");
        assert_eq!(
            text,
            "{\"condition\":{\"arm\":{\"weapon\":\"cannon\",\"target\":{\"target\":\"best_hostile\"},\
             \"min_permille\":100,\"aspects\":15,\"min_rounds_left\":2,\"min_range_m\":0,\
             \"max_range_m\":600}}}"
        );
        let back: IntentKind = serde_json::from_str(&text).expect("and reads back");
        assert_eq!(back, arming);
        let disarm = IntentKind::Condition(Arming::Disarm { weapon: None });
        let back: IntentKind =
            serde_json::from_str(&serde_json::to_string(&disarm).expect("serialises"))
                .expect("reads back");
        assert_eq!(back, disarm);
        // `Fire.slot`: absent when `None`, read back when present, defaulted
        // when a format-6 `Fire` is read.
        assert_eq!(
            serde_json::to_string(&IntentKind::fire("cannon")).expect("serialises"),
            "{\"fire\":{\"weapon\":\"cannon\"}}"
        );
        let cited = IntentKind::Fire {
            weapon: SmolStr::new("cannon"),
            slot: Some(2),
        };
        let text = serde_json::to_string(&cited).expect("serialises");
        assert_eq!(text, "{\"fire\":{\"weapon\":\"cannon\",\"slot\":2}}");
        let back: IntentKind =
            serde_json::from_str("{\"fire\":{\"weapon\":\"cannon\"}}").expect("a bare fire reads");
        assert_eq!(back, IntentKind::fire("cannon"));

        // A seat's arming counts on the window as a `LOCK` does.
        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        let max = arena.arena_cfg.intents.max_pending_per_unit;
        for seq in 0..u64::from(max) {
            assert_eq!(
                apply_intent(
                    &mut arena,
                    &intent(RED, 1, seq, arming.clone()),
                    IntentSource::Seat
                ),
                IntentOutcome::Queued
            );
        }
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 99, arming.clone()),
                IntentSource::Seat
            ),
            IntentOutcome::Refused(Refusal::QueueFull)
        );
        // A slot's own shot past the window, and through a linkdead seat.
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 100, IntentKind::fire("cannon")),
                IntentSource::Cond
            ),
            IntentOutcome::Queued
        );
        if let Some(state) = arena.unit_mut(RED) {
            state.linkdead = true;
        }
        assert_eq!(
            apply_intent(&mut arena, &intent(RED, 1, 101, arming), IntentSource::Seat),
            IntentOutcome::Refused(Refusal::Linkdead),
            "a seat's arming meets the linkdead gate"
        );
        assert_eq!(
            apply_intent(
                &mut arena,
                &intent(RED, 1, 102, IntentKind::fire("cannon")),
                IntentSource::Cond
            ),
            IntentOutcome::Queued,
            "a slot's own shot does not"
        );
        assert_eq!(
            arena.unit(RED).map(|state| state.orders_in_window),
            Some(max),
            "the slot's shots counted nothing"
        );
        // The three new words.
        assert_eq!(Refusal::Unwarranted.as_str(), "unwarranted");
        assert_eq!(Refusal::NoSlot.as_str(), "no_slot");
        assert_eq!(Refusal::Unlawful.as_str(), "unlawful");
        assert_eq!(
            serde_json::to_string(&Refusal::Unlawful).expect("serialises"),
            "\"unlawful\""
        );
    }

    /// A native lock rule requires Sensors and its current epoch even when
    /// the sender holds Fire. Removing a Sensors slot takes the same channel.
    #[test]
    fn m5rules_lock_rules_cannot_borrow_fire_authority() {
        use crate::arena::conditions::{Arming, LockSelector, Rule};
        let kind = IntentKind::Condition(Arming::Rules {
            slot: Some(1),
            rules: vec![Rule::lock_rule(LockSelector::Nearest, None, Vec::new())],
        });
        assert!(kind.is_discrete());
        assert_eq!(kind.channel_required(), Channel::Sensors);
        assert_eq!(kind.as_str(), "lock");
        let mut arena = two_tanks(1);
        online(&mut arena, RED);
        let mut order = intent(RED, 1, 0, kind);
        order.channel = Channel::Fire;
        assert_eq!(
            apply_intent(&mut arena, &order, IntentSource::Seat),
            IntentOutcome::Refused(Refusal::Channel)
        );
        order.channel = Channel::Sensors;
        order.epoch = 0;
        assert_eq!(
            apply_intent(&mut arena, &order, IntentSource::Seat),
            IntentOutcome::Refused(Refusal::Epoch)
        );
        order.epoch = 1;
        assert_eq!(
            apply_intent(&mut arena, &order, IntentSource::Seat),
            IntentOutcome::Queued
        );
        let remove = IntentKind::Condition(Arming::Remove {
            slot: 1,
            channel: Channel::Sensors,
        });
        assert_eq!(remove.channel_required(), Channel::Sensors);
    }

    /// **17b cut M5.2: the profile switch's intent and word** (doc 17 §11,
    /// D-UM-44; the design note's item 4): `IntentKind::Profile` is
    /// continuous, not discrete, on the helm, word `mind` (the verb the pilot
    /// types), carried as `{"profile": {"index": n}}`; `Refusal::Profile`'s
    /// word is `profile`, carried as `"profile"`, its sentence `that profile
    /// cannot be switched to now`.
    #[test]
    fn m5_the_profile_intent_and_its_refusal_word() {
        let kind = IntentKind::Profile { index: 1 };
        assert!(kind.is_continuous());
        assert!(!kind.is_discrete());
        assert_eq!(kind.channel_required(), Channel::Helm);
        assert_eq!(kind.as_str(), "mind");
        let wire = serde_json::to_value(&kind).expect("serialises");
        assert_eq!(wire, serde_json::json!({"profile": {"index": 1}}));
        assert_eq!(serde_json::from_value::<IntentKind>(wire).ok(), Some(kind));
        assert_eq!(Refusal::Profile.as_str(), "profile");
        assert_eq!(
            serde_json::to_string(&Refusal::Profile).expect("serialises"),
            "\"profile\""
        );
        assert_eq!(
            Refusal::Profile.to_string(),
            "that profile cannot be switched to now"
        );
        assert_eq!(PROFILE_SWITCH_MS, 10_000);
    }

    /// **A possess clears the dormant row's order** (17b cut M4.3 over
    /// M3.5's rule; doc 17 §12 "Release and possess"): `holds_order_for`
    /// reads a mind's row only while the hull is the mind's. Red's mind holds
    /// an order for red-1; a possess keeps the row with its order CLEARED
    /// (`Arena::suspend_mind`: an order given to the mind is not the pilot's
    /// and does not outlive the seat), so the hull holds none; the hand-back
    /// still holds none and starts under doctrine, seeded `Withdraw`.
    #[test]
    fn m4_3_a_possess_clears_the_dormant_order() {
        let squad = crate::deliberation::SquadId {
            team: Team::Red,
            index: 1,
        };
        let mut arena = two_tanks(1);
        arena.set_controller(RED, Controller::Mind, MachineState::Withdraw);
        if let Some(state) = arena.minds.get_mut(&RED) {
            state.order = Some(crate::arena::perception::FrameOrder {
                identity: None,
                squad: SmolStr::new(squad.to_string()),
                objective: crate::arena::perception::MindObjective::Hold,
                hold_fire: false,
                max_range_band: None,
                expires_ms: 600_000,
            });
        }
        assert!(holds_order_for(&arena, RED, squad), "the mind holds it");
        arena.set_controller(RED, Controller::Pilot, MachineState::Withdraw);
        assert!(
            arena
                .minds
                .get(&RED)
                .is_some_and(|state| state.order.is_none()),
            "the possess cleared the dormant row's order"
        );
        assert!(
            !holds_order_for(&arena, RED, squad),
            "a dormant row's order is nobody's"
        );
        arena.set_controller(RED, Controller::Mind, MachineState::Withdraw);
        assert!(
            !holds_order_for(&arena, RED, squad),
            "the hand-back starts under doctrine, not under the old order"
        );
        assert_eq!(
            arena.minds.get(&RED).map(|state| state.doctrine_state),
            Some(MachineState::Withdraw),
            "seeded Withdraw"
        );
        assert_eq!(arena.machine_state(RED), Some(MachineState::Withdraw));
    }
}
