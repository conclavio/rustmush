//! **The cockpit: boarding, the pilot's fourteen verbs, and the frame they
//! read** (16b cut K.2; 16b §8 cut K, K2–K4; doc 16 §3.3 "Startup", "Stations
//! are on the tank", §3.6 "What a pilot sees", §3.7, §3.8 "Disconnects", §5).
//!
//! Cut K.1 built the lobby: a centre, slots, pods locked to pilots, and the
//! named `Boarding → Running` trigger waiting for somebody to report a seat.
//! This is the half that reports one, and everything the pilot can do once
//! seated.
//!
//! **Fourteen**: the twelve cockpit selectors of [`COCKPIT_VERBS`], plus
//! [`BOARD_VERB`] and [`EXIT_VERB`], which are not selectors — they are how a
//! pilot gets into the cockpit and out of it. The table below is that list in
//! that order, and **16b cut D.2** made the three agree: the head said twelve in
//! one place and fourteen in another, and the table was missing `RANGE` and
//! `BEARING` altogether (16c `T-77`).
//!
//! ```text
//! BOARD                      climb into your pod and take the tank's cockpit
//! STARTUP                    bring power up (idempotent)
//! HEADING <0-359>            steer
//! SPEED <kph>                make way (negative is reverse)
//! VIEW                       the arena window, as a text map
//! CONTACTS                   what the platform holds
//! LOCK <contact>             hold one of them
//! RANGE [<contact>]          how far away one of them plotted
//! BEARING [<contact>]        which way one of them plotted
//! SCAN [<contact>]           what is left of one of them, by section
//! FIRE [<weapon>]            shoot it
//! WEAPONSTATUS               ammunition and readiness
//! STATUS                     callsign, power, speed, heading, scope, damage
//! EXIT                       climb out
//! ```
//!
//! # Boarding is four facts, in one order
//!
//! Doc 16 §3.3 says *stations are on the tank*, and §3.7 says the pod *is where
//! the body sits*. Those two sentences are the whole design, and they fix the
//! order [`board`] does its work in:
//!
//! 1. **the station** — one `cockpit` billet on the **tank**, carrying `helm`,
//!    `sensors` and `fire`, whose console `space` is the **pod**. So the grants
//!    have the tank as their subject (which is what the arena authorises against)
//!    and reach is satisfied by a body in the pod (which is where the pilot is).
//!    Written through [`mush_sim::declare_station`], idempotently: the same pod
//!    always flies the same tank, so a second boarding rewrites nothing and
//!    invalidates no grant.
//! 2. **the body** — [`mush_sim::place_interior`] puts the character inside the
//!    pod. It comes *before* the seat, because the seat's reach proof is
//!    `Console { room: the pod }` and a pilot standing in the team room does not
//!    satisfy it.
//! 3. **the seat** — `mush_sim::request_seat_activation(Activate)`, which is the
//!    one path in the tree that writes a billet, narrows its channel set, claims
//!    the account's single foreground focus and asks K6 for the control lease. The
//!    cockpit opens no second path and mints no grant of its own.
//! 4. **the centre** — [`crate::match_::slot_seated`], cut K.1's trigger input.
//!    When this seat was the last one the match was waiting for, the check-out
//!    fires inside this same command.
//!
//! [`exit`] undoes 3, 2 and (during `Boarding`) 4, in that order, and sends
//! `Control { Released }` **first** — before the seat itself is released — so
//! that a pilot who types `FIRE` and `EXIT` in one burst has the epoch raised
//! under the queued shot and the arena purges it (doc 16 §3.1).
//!
//! # The verbs are contextual, and the gate is what makes them so
//!
//! There is no such thing as a verb that exists only in a cockpit: the command
//! table is global. So all fourteen are registered globally and
//! [`CockpitGate`] refuses each of them, with one line, wherever it does not
//! mean anything — `BOARD` outside a team room, `EXIT` outside a pod, and the
//! twelve cockpit selectors to anyone who is not focused on a station of a
//! **deployed** unit. A pilot who has boarded but whose match has not started
//! yet is in that last case on purpose: there is no arena to order until the
//! trigger fires.
//!
//! # Where the numbers come from
//!
//! The arena is the only thing that knows a tank's speed, heading, power or
//! tempo, and it publishes them as the `arena.cockpit` keyframe cut T.2 opened.
//! The world keeps the last one per pilot
//! ([`crate::arena_bridge::ArenaRegistry::last_frame`]) and every read verb
//! renders **that** — never a second model of the same fact. A pilot with no
//! frame yet has a cold tank, which is true and is also the fail-closed answer:
//! `HEADING` and `SPEED` refuse with [`POWER_NOT_ONLINE`] before the arena would.
//!
//! ## One frame, two shapes
//!
//! [`status_frame`] is what a pulse sends: the numbers, the contacts (16b cut S.2),
//! the notices it owes and the weapons table. [`view_frame`] is that same frame
//! **plus** the
//! arena window in the grid shape `mush_ansi::panel` already renders, which is
//! what makes `VIEW` a text map on a line client and a map panel on the web with
//! no per-feed code in either. The grid rides `VIEW` and not the pulse
//! deliberately: a line client would otherwise be sent twenty rows of map twice a
//! second.
//!
//! ## And a third, which is not a cockpit frame at all
//!
//! **16b cut Q.3** added [`map_frame`]: the whole arena's terrain on its own feed
//! ([`crate::arena_bridge::MAP_FEED`]), carrying no unit of any kind. It is only
//! ever the answer to a **query**, never pushed, so a line client is never sent
//! it and a panel client asks for it once per match. A panel client that holds a
//! map and a cockpit frame composes them itself; [`view_window`] is the same
//! composition done server-side for `VIEW`, and the fixtures under
//! `web/src/lib/fixtures/cockpit/` are the one set all three renderers are held
//! to (doc 16 §5).

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, ChannelId, CmdError, CommandSpec, Ctx, ObjId, Registry, World};
use mush_proto::{OutputEvent, RichText};
use smol_str::SmolStr;

use crate::arena::damage;
use crate::arena::intents::{ControlState, IntentKind};
use crate::arena::motion::BlockedCause;
use crate::arena::perception::{FrameNotice, NoticeKind, RefusalDetail};
use crate::arena::state::{ArenaTerrain, Channel, Classification, ContactId, Designator, Power};
use crate::arena::step::Frame;
use crate::arena_bridge::{
    ArenaRegistry, CockpitFrame, FRAME_FEED, IntentRefusal, NOTICE_CATEGORY, forward_intent,
    send_control,
};
use crate::arena_task::MatchId;
use crate::match_::MatchState;
use crate::match_::centre::{self, CentreRoom};
use crate::match_::lobby::{self, SimError};

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

/// The boarding verb.
pub const BOARD_VERB: &str = "board";

/// The leaving verb. A primary verb of its own: nothing in the tree registered
/// `exit` (the spatial layer's player-facing leave is `egress`, which needs a
/// domain to step onto and a pod is unanchored), so doc 16 §3.7's spelling is
/// free and is what a pilot will type.
pub const EXIT_VERB: &str = "exit";

/// The cockpit selectors, in doc 16 §3.7's order. Registered lowercase because
/// the command table folds case on the way in (`Registry::lookup_idx`), so doc
/// 16's capitals and a tired pilot's lower case are one verb.
///
/// **16b cut Q.1** added `range` and `bearing`. Doc 16 §3.7 listed nine; the
/// owner's first hand-played match wanted the two questions a pilot asks about a
/// contact once they have one — *how far* and *which way* — as verbs rather than
/// as columns to re-read `CONTACTS` for.
///
/// **16b cut Q.2** added `scan`, which is the third: *what have I done to it*.
pub const COCKPIT_VERBS: [&str; 12] = [
    "startup",
    "heading",
    "speed",
    "view",
    "contacts",
    "lock",
    "fire",
    "weaponstatus",
    "status",
    "range",
    "bearing",
    "scan",
];

/// The station id one pod's pilot holds **on the tank** (doc 16 §3.3).
///
/// Deliberately not `helm`, `sensors` or `fire`: a cockpit seat carries all
/// three channels, and [`crate::arena_bridge::open_match`]'s rule is that a
/// station named after one channel raises that one and any other name raises all
/// three. The name is the thing that makes the seat a cockpit.
pub const STATION: &str = "cockpit";

/// The station's descriptive role (`StationSpec::role`, which the engine reads
/// nothing from).
pub const STATION_ROLE: &str = "pilot";

/// Its dispatch priority — `@spatial/station`'s own `pilot` default, so a tank's
/// cockpit ranks like every other pilot billet in the tree.
pub const STATION_PRIORITY: u16 = 2;

/// The weapon `FIRE` takes when the pilot names none, re-exported from the arena
/// (16b cut F.3 moved it there, because the arena's own band advice and the
/// reflex controller's range caps are read against the same weapon's bands and
/// there may be exactly one rule for which weapon that is).
pub use crate::arena::weapons::DEFAULT_WEAPON;

/// How many cells of the arena window one `VIEW` carries, across and up. The
/// widest and tallest a line client prints (`mush_ansi::panel`'s own bounds), so
/// the frame is the map and the renderer only has to clip it to a narrow
/// terminal.
pub const VIEW_COLUMNS: u16 = 78;

/// As [`VIEW_COLUMNS`], up the window.
pub const VIEW_ROWS: u16 = 20;

/// The glyph the pilot's own platform draws with.
pub const YOU_GLYPH: &str = "@";

/// **The glyph an unresolved contact draws with** on `VIEW` (16b cut S.2; doc 16
/// §3.6: *a marker per contact*). A return, not a tank: the pilot knows something
/// is there and nothing else.
pub const UNKNOWN_GLYPH: &str = "?";

/// A contact resolved to the platform's **own** side (doc 16 §3.6: *teammates
/// appear when shared*).
pub const FRIEND_GLYPH: &str = "+";

/// A contact resolved to the **other** side.
pub const HOSTILE_GLYPH: &str = "x";

// ---------------------------------------------------------------------------
// The refusals
// ---------------------------------------------------------------------------

/// `BOARD` outside a team room of the centre.
pub const OUTSIDE_TEAM_ROOM: &str = "Boarding is through one of the simulator's team rooms.";

/// `EXIT` outside a pod.
pub const NOT_IN_A_POD: &str = "You are not in a pod.";

/// A cockpit verb typed by somebody who is not flying anything.
pub const OUTSIDE_COCKPIT: &str = "You are not at the station of a deployed unit.";

/// **A cockpit verb typed between `BOARD` and the trigger** (16b cut F.1).
///
/// The K row's deviation 13 records the mechanism — the gate needs a deployed
/// unit, and nothing is deployed until every pilot is seated — but the pilot was
/// shown [`OUTSIDE_COCKPIT`], which is the sentence for a lost seat or a stray
/// `+focus`, while sitting in the cockpit with the hatch shut. This is the
/// sentence for the one case that is not a fault: the pilot holds the seat, the
/// match has simply not started.
pub const NOT_STARTED_YET: &str =
    "The panel is dark until the match starts; the simulator is still waiting for the other pods.";

/// **The one line a motion or fire order refuses with while the tank is cold**
/// (doc 16 §3.3 "Startup").
///
/// The cockpit refuses before the arena does, so the pilot gets the reason at
/// once instead of a pulse later; `the_cold_refusal_carries_the_arenas_own_word`
/// below pins it to contain [`crate::arena::intents::Refusal::PowerNotOnline`]'s
/// own word, so the early refusal and the late one can never drift apart.
pub const POWER_NOT_ONLINE: &str = "Power not online: STARTUP first.";

/// **The line a motion or fire order refuses with while power is coming up**
/// (16b cut F.1; doc 16 §3.3 "Startup": *`STARTUP` sets `power` to `starting`*).
///
/// [`POWER_NOT_ONLINE`] told a pilot who had already typed `STARTUP` to type it
/// again, for the ten seconds the sequence takes. The tank is doing exactly what
/// it was asked to; the only useful thing to say is *wait*.
pub const POWER_STARTING: &str = "Power is still coming up; hold on.";

/// **The one line `FIRE` refuses with for want of a lock** (doc 16 §3.7: *no
/// lock*).
///
/// The arena's own refusal renders these same words, for the same reason
/// [`POWER_NOT_ONLINE`] does.
pub const NO_LOCK: &str = "No lock: LOCK a contact first.";

/// **The line the read verbs refuse with for want of both a lock and a name**
/// (16b cut Q.1).
///
/// [`NO_LOCK`] is **`FIRE`'s** line and is not touched: `FIRE` takes a weapon and
/// never a contact, so telling its pilot they could "name one" instead would be a
/// lie about what the verb accepts. `RANGE` and `BEARING` do take a contact, so
/// they get their own sentence and both are pinned.
pub const NO_LOCK_OR_NAME: &str = "No lock: LOCK a contact first, or name one.";

/// **The words `FIRE` refuses with while the breech is busy** (16b cut W.1; doc 16
/// §3.7: *cycling*). The seconds left follow it, so the const is the stable needle
/// and the line is the useful sentence.
pub const CYCLING: &str = "Still cycling";

/// **The words `FIRE` refuses with when a weapon is out of rounds** (16b cut W.1;
/// doc 16 §3.7: *empty*).
pub const EMPTY: &str = "Empty";

/// **The words `FIRE` refuses with at the heat ceiling** (16b cut X.1; doc 16 §9
/// item 1: *a shot refused above a heat ceiling*).
///
/// The arena's own refusal renders this same word, for the same reason
/// [`POWER_NOT_ONLINE`] does, and the pin test below holds the two together.
pub const OVERHEATED: &str = "Overheated";

/// **The line every order refuses with once the tank is a wreck** (16b cut D.2,
/// `B-03`; doc 16 §3.3 "A wreck": *no longer the subject of any order*).
///
/// Read off [`Frame::wreck`], before the power check: a dead tank's power is
/// beside the point, and `Power not online: STARTUP first.` is the one answer a
/// pilot in a wreck must never be given. It names what to do instead, because
/// the whole of `B-03` is a pilot typing into a dead tank and being told nothing.
pub const DESTROYED: &str = "Your tank is destroyed and takes no orders. EXIT leaves the pod.";

/// **The line `HEADING` and `SPEED` refuse with once motion is gone** (16b cut
/// D.2, `B-03`).
///
/// Byte for byte the sentence [`refusal_line`] says for
/// [`crate::arena::intents::Refusal::MotionDisabled`], and the pin test below
/// holds the two together, for the reason [`POWER_NOT_ONLINE`] is pinned to the
/// arena's word.
pub const IMMOBILE: &str = "Your tank cannot move.";

/// **What follows a friendly contact's designator on the three lines that name
/// one** (16b cut D.2, `B-18`; doc 16 §3.6).
///
/// Friendly fire is **said and never gated**: the arena refuses nothing, because
/// a crew that means to shoot at a friend is a thing the game is entitled to let
/// happen and a refusal would be the engine deciding. What the pilot gets is the
/// word, on the `LOCK` echo, on [`lock_line`] (so `WEAPONSTATUS` says it too) and
/// on the hit. It is read at render time off the frame's own row for that
/// contact, so an unidentified return is never friendly.
pub const FRIENDLY_MARK: &str = " (FRIENDLY)";

/// **The one byte-identical refusal for any contact id the platform does not
/// hold** (doc 16 §3.6: *unresolvable, unowned or expired* are one answer).
///
/// **16b cut S.1 moved the string** to [`crate::arena::state::NO_SUCH_CONTACT`],
/// beside `ShadowContacts::resolve` — the only lookup a pilot's id goes through —
/// and re-exports it here under K.2's name. There is exactly one of this sentence
/// in the tree, so the three cases cannot drift into three answers, and every
/// caller that had this path keeps it.
pub use crate::arena::state::NO_SUCH_CONTACT;

/// Why a cockpit verb refused. Every variant is the line the pilot sees.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CockpitError {
    /// `BOARD` was typed outside a team room.
    #[error("{OUTSIDE_TEAM_ROOM}")]
    OutsideTeamRoom,
    /// `EXIT` was typed outside a pod.
    #[error("{NOT_IN_A_POD}")]
    NotInAPod,
    /// A cockpit verb was typed by somebody with no deployed station.
    #[error("{OUTSIDE_COCKPIT}")]
    OutsideCockpit,
    /// A cockpit verb was typed by a seated pilot whose match has not started
    /// (16b cut F.1).
    #[error("{NOT_STARTED_YET}")]
    NotStartedYet,
    /// The tacsim layer did not compose.
    #[error("The arena is not available on this server.")]
    Unavailable,
    /// The centre refused, in its own words.
    #[error("{0}")]
    Centre(#[from] SimError),
    /// The pilot's slot is on the other side.
    #[error("Your slot is on the {team} side; board there.")]
    WrongRoom {
        /// The side the slot is on.
        team: &'static str,
    },
    /// The pilot is in the pod already.
    #[error("You are already in your pod.")]
    AlreadyAboard,
    /// The tank's station could not be written.
    #[error("That pod's tank has no cockpit: {0}")]
    Station(String),
    /// The character could not be put in the pod, or taken out of it.
    #[error("The hatch will not move: {0}")]
    Hatch(String),
    /// The seat itself refused.
    #[error("That cockpit will not wake for you: {0}")]
    Seat(String),
    /// T11's guardrail: a proxy driver may not abandon what it was lent.
    #[error("{0}")]
    Guarded(String),
    /// A cockpit order was refused by the world's own routing. Its words are
    /// [`IntentRefusal`]'s — one named reason each, so the cockpit never says
    /// "no" — read as the sentence a pilot is told: "Refused: you hold no
    /// station on that unit."
    #[error("Refused: {0}.")]
    Order(#[from] IntentRefusal),
    /// The order's argument did not parse, or is out of range.
    #[error("{0}")]
    Usage(String),
    /// Power is not online (doc 16 §3.3).
    #[error("{POWER_NOT_ONLINE}")]
    PowerNotOnline,
    /// Power is coming up (16b cut F.1; doc 16 §3.3 "Startup").
    #[error("{POWER_STARTING}")]
    PowerStarting,
    /// No lock (doc 16 §3.7).
    #[error("{NO_LOCK}")]
    NoLock,
    /// No lock and no contact named, for a read verb that takes either (16b cut
    /// Q.1).
    #[error("{NO_LOCK_OR_NAME}")]
    NoLockOrName,
    /// The named weapon's cycle has not finished (16b cut W.1).
    #[error("{CYCLING}: {secs} s to go.")]
    Cycling {
        /// Seconds left, rounded up — a pilot counts in seconds.
        secs: i64,
    },
    /// The named weapon has no rounds left (16b cut W.1).
    #[error("{EMPTY}: that weapon has nothing left to fire.")]
    Empty,
    /// The platform is at its heat ceiling (16b cut X.1; doc 16 §9 item 1).
    #[error("{OVERHEATED}: {heat} of {max}. Let it cool.")]
    Overheated {
        /// What it is carrying.
        heat: u32,
        /// The ceiling.
        max: u32,
    },
    /// One line for every contact id the platform does not hold.
    #[error("{NO_SUCH_CONTACT}")]
    NoSuchContact,
    /// The unit carries no weapon by that name.
    #[error("That unit carries no weapon called `{0}`.")]
    NoSuchWeapon(String),
    /// **The word names more than one of the platform's weapons** (16b cut D.2,
    /// `B-17`): a short name or a prefix that two rows answer to.
    #[error("`{word}` could be {ids}; say more.")]
    AmbiguousWeapon {
        /// What the pilot typed, trimmed.
        word: String,
        /// The candidate ids, in row order, joined with ` or `.
        ids: String,
    },
    /// **The tank is a wreck** (16b cut D.2, `B-03`), read off [`Frame::wreck`].
    #[error("{DESTROYED}")]
    Destroyed,
    /// **The tank cannot move** (16b cut D.2, `B-03`), read off
    /// [`Frame::immobile`]. `HEADING` and `SPEED` only: a tank with a dead engine
    /// still sees and still shoots.
    #[error("{IMMOBILE}")]
    Immobile,
    /// **A `FIRE` the cockpit refused locally, in the arena's own sentence** (16b
    /// cut D.2, `B-16`).
    ///
    /// Every `FIRE` refusal reads `FIRE <weapon> refused. <reason>`, whichever
    /// half answered: the cockpit's pre-check against the last frame, or the
    /// arena's own a pulse later. Both compose it through [`order_refused`], so
    /// the early answer and the late one are one string — which is exactly the
    /// contract cut F.2 wrote [`refusal_line`] for, read once more now that the
    /// weapon is in the sentence.
    #[error("{0}")]
    Refused(String),
}

// ---------------------------------------------------------------------------
// Reading the cockpit
// ---------------------------------------------------------------------------

/// The channels a cockpit seat grants (doc 16 §3.7: `helm`, `sensors`, `fire`).
///
/// Built from the arena's own [`Channel`] vocabulary, so the seat and the orders
/// cannot name two different sets.
#[must_use]
pub fn cockpit_channels() -> BTreeSet<ChannelId> {
    Channel::ALL
        .iter()
        .filter_map(|channel| ChannelId::new(channel.as_str()))
        .collect()
}

/// The station id, as a typed [`mush_sim::StationId`].
fn station_id() -> Option<mush_sim::StationId> {
    mush_sim::StationId::new(STATION)
}

/// **The unit and station an actor is flying** — the cockpit's one context test.
///
/// The actor's focused station (through the proxy ingress, so a driver of an NPC
/// gunner resolves to the NPC), narrowed to a unit an arena actually holds
/// custody of. A pilot who has boarded but whose match has not started yet is
/// deliberately **not** in a cockpit: there is no arena to order.
#[must_use]
pub fn flying(w: &World, actor: ObjId) -> Option<(ObjId, mush_sim::StationId)> {
    let driver = mush_sim::focus::driving_actor(w, actor);
    let focus = w
        .module_state::<mush_sim::FocusState>()?
        .focus_of_actor(driver)?;
    let (unit, station) = (focus.unit, focus.station.clone());
    mush_sim::deployed::is_deployed(w, unit).then_some((unit, station))
}

/// The seats one actor holds on one station id, whatever its focus says.
///
/// `EXIT` reads this rather than the focus: a pilot who typed `+focus` at
/// something else still holds the billet, and leaving the pod has to give it
/// back.
fn seats_of(w: &World, actor: ObjId) -> Vec<(ObjId, mush_sim::StationId)> {
    w.module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.derived.seats_by_actor.get(&actor))
        .map(|seats| seats.iter().cloned().collect())
        .unwrap_or_default()
}

/// The pod an actor's body is sitting in, if it is sitting in one.
fn pod_of_body(w: &World, actor: ObjId) -> Option<ObjId> {
    let at = w.get(actor)?.location?;
    centre::pod_slot(w, at).map(|_| at)
}

/// **Whether an actor's body is inside one of the centre's pods** (16b cut F.1),
/// for a caller that has no other reason to know which one.
#[must_use]
pub fn in_a_pod(w: &World, actor: ObjId) -> bool {
    pod_of_body(w, actor).is_some()
}

/// The last frame the arena sent that pilot.
fn last_frame(w: &World, pilot: ObjId) -> Option<CockpitFrame> {
    w.module_state::<ArenaRegistry>()
        .and_then(|registry| registry.last_frame(pilot))
        .cloned()
}

/// **The power state the cockpit will act on**: the last frame's, or
/// [`Power::Off`] for a pilot the arena has not spoken to yet.
///
/// **16b cut D.1 put the `Starting` hint in front of the `Off` answer.** A frame
/// at the `Quiet` tempo a match opens in is up to five seconds away, so between
/// `STARTUP` and the arena's answer every motion verb told the pilot to type
/// `STARTUP`. Where the mirror says `Off` — or says nothing at all — and the
/// world has forwarded a `STARTUP` whose hint has not expired
/// ([`crate::arena_bridge::ArenaRegistry::starting_hint`]), the answer is
/// [`Power::Starting`]: the pilot is told to wait, which is true, instead of told
/// to do the thing they have done. Every frame that says anything but `Off`, and
/// every frame that carries the arena's refusal of that same `STARTUP`, clears
/// the hint at the bridge before this is read again.
#[must_use]
pub fn power_of(w: &World, pilot: ObjId) -> Power {
    let held = last_frame(w, pilot).map_or(Power::Off, |held| held.frame.power);
    if held != Power::Off {
        return held;
    }
    let starting = w
        .module_state::<ArenaRegistry>()
        .is_some_and(|registry| registry.starting_hint(pilot, w.now()));
    if starting {
        Power::Starting
    } else {
        Power::Off
    }
}

/// **Refuse every order once the tank is a wreck** (16b cut D.2, `B-03`; doc 16
/// §3.3 "A wreck").
///
/// It goes **before** [`require_power`] wherever both run, and that order is the
/// whole point: a wreck's power is beside the point, and cut D.1's `Starting`
/// hint would otherwise answer `Power is still coming up; hold on.` for six
/// seconds to a pilot whose tank had just been shot to pieces — which is the
/// worst sentence in the tree for that moment.
///
/// A pilot the arena has not spoken to yet has no frame and is **not** refused:
/// a cold tank is not a dead one, and `Frame::wreck` is a fact the arena
/// publishes rather than one the world can infer.
///
/// # Errors
///
/// [`CockpitError::Destroyed`].
fn require_alive(w: &World, pilot: ObjId) -> Result<(), CockpitError> {
    match last_frame(w, pilot) {
        Some(held) if held.frame.wreck => Err(CockpitError::Destroyed),
        _ => Ok(()),
    }
}

/// **Refuse `HEADING` and `SPEED` once motion is gone** (16b cut D.2, `B-03`).
///
/// [`CockpitError::Immobile`]'s sentence is byte for byte what [`refusal_line`]
/// says for [`crate::arena::intents::Refusal::MotionDisabled`] a pulse later. A
/// tank with a dead engine still sees, still locks and still shoots, so this runs
/// on the two motion verbs and on nothing else.
///
/// # Errors
///
/// [`CockpitError::Immobile`].
fn require_mobile(w: &World, pilot: ObjId) -> Result<(), CockpitError> {
    match last_frame(w, pilot) {
        Some(held) if held.frame.immobile => Err(CockpitError::Immobile),
        _ => Ok(()),
    }
}

/// Refuse a motion or fire order while the tank is cold.
fn require_power(w: &World, pilot: ObjId) -> Result<(), CockpitError> {
    // **16b cut F.1**: a tank that is already starting up is told to wait, not
    // told to do the thing it is doing.
    match power_of(w, pilot) {
        Power::Online => Ok(()),
        Power::Starting => Err(CockpitError::PowerStarting),
        Power::Off => Err(CockpitError::PowerNotOnline),
    }
}

/// **The contacts the platform holds**, as a pilot is told about them (doc 16
/// §3.6 "What a pilot sees").
///
/// **16b cut S.1 filled the arena side** — the sensor pass mints the ids and
/// mirrors them into [`Frame::contacts`] — and **this is cut S.2's read of it**:
/// the row key unchanged and never printed, the short designator 16b cut Q.1 gave
/// a pilot to read and type, the class as `unknown` or as the side the platform
/// has resolved, the bearing in whole degrees both absolute and relative to the
/// platform's own heading, the range as one of [`contact_band`]'s four words
/// (16b cut D.1: the pilot's word, thirds of the lesser of the platform's own
/// sight and the fired weapon's reach), and the age in whole seconds.
///
/// Three things a pilot is deliberately **not** given, and each is the observer
/// rule on this surface: the target's object number, its name, and its team while
/// the contact is still `unknown`. The row carries the contact's last-seen
/// **cell** because that is what `VIEW` draws a marker in, which is the map's own
/// resolution and no finer.
///
/// **16b cut F.3** changed what the numbers are *of*. Every figure here —
/// bearing, range, band, cell — is read off the **plot** the arena recorded at
/// the sighting that last refreshed the contact, so a row is what the platform
/// knows and never where the target now is; and `age_ms` is the age of that
/// sighting, which is how a pilot tells a live track from a dying one. It also
/// adds the range in metres ([`ContactRow::range_m`]), because a band alone told
/// a pilot nothing at all.
#[must_use]
pub fn frame_contacts(frame: &Frame) -> Vec<ContactRow> {
    // 16b cut F.3: the bands of the weapon a bare `FIRE` would fire — the weapon
    // the band word is advice about. Resolved once for the list.
    let bands: &[u32] = fired_by_default(&frame.weapons).map_or(&[], |weapon| &weapon.bands_cm);
    frame
        .contacts
        .iter()
        .map(|held| ContactRow {
            id: held.id,
            // 16b cut Q.1: the short name and the number behind it, both as the
            // arena resolved them off the live contact.
            designator: held.designator,
            alias: held.alias,
            class: SmolStr::new(class_word(held.class)),
            bearing_deg: held.bearing_mdeg / 1_000,
            relative_deg: relative_mdeg(frame.pose.heading_mdeg, held.bearing_mdeg) / 1_000,
            // 16b cut D.1: thirds of the lesser of the sight and the weapon's
            // reach. 16b cut E.1: and the only band word there is.
            band: SmolStr::new(contact_band(held.range_cm, bands, frame.sight_cm)),
            range_m: range_m_rounded(held.range_cm, CONTACT_RANGE_STEP_M),
            age_ms: held.age_ms,
            cell: held.cell,
            friendly: held.class.team() == Some(frame.team),
        })
        .collect()
}

/// **How coarsely a contact's range is reported**, metres (16b cut F.3).
///
/// Doc 16 §3.6 gives `CONTACTS` a *range band* and no range, and cut S.2 read
/// that strictly. In play it left a pilot with no distance information whatever:
/// the band could only ever read `close` (see [`contact_band`]), so from
/// acquisition to kill every row said the same word whether the other tank was
/// 600 m away or 40 m away.
///
/// A passive sight giving a coarse range is a **playtest-usability decision**,
/// recorded here for the owner to revisit: it departs from doc 16 §3.6 in the
/// direction of a pilot being able to act. The rounding is the whole of the
/// coarseness and it lives in this one constant, so tightening or loosening it is
/// one number.
pub const CONTACT_RANGE_STEP_M: u64 = 50;

/// **The finest a *locked* contact's range is ever reported**, metres (16b cut
/// Q.1; 16b cut D.4 made it the floor rather than the step).
///
/// A lock is a fire-control solution: the platform is tracking one return
/// continuously and resolving a hit chance against it every pulse, so the range
/// it can quote about that one contact is better than the range it quotes about
/// the rest of the scope. It is a second step past doc 16 §3.6's band-only
/// wording, recorded here beside the first for the owner to revisit, and it is
/// what makes `RANGE` worth typing after `LOCK`.
///
/// [`lock_range_step_m`] is what a lock's range is actually rounded to: this is
/// the floor it clamps up from, which is what a plot taken this instant is worth.
pub const LOCK_RANGE_STEP_M: u64 = 10;

/// **The fastest thing the catalog fields**, metres a second, rounded up (16b cut
/// D.4, `T-20`).
///
/// 17 m/s is about 60 kph, which is over the stock tank's 54 and over every other
/// hull the game ships. It is the drift term of [`lock_range_step_m`]: the
/// furthest a plot's subject could have got since the plot was taken.
pub const PLOT_DRIFT_M_PER_S: i64 = 17;

/// **How finely a locked contact's range may honestly be reported**, metres,
/// given a plot `age_ms` old (16b cut D.4, `T-20`; 16c §5 Q28: *derived from the
/// plot's age, floor 10 m*).
///
/// `LOCK_RANGE_STEP_M` was a fixed 10 m, which is finer than a plot one pass old
/// can carry: a target making 60 kph has moved 17 m in the second since the
/// sighting, so a 10 m figure quoted about it is three significant digits of
/// which the last one is invented.
///
/// The step is the drift, `age_ms × `[`PLOT_DRIFT_M_PER_S`]` / 1000`, rounded
/// **up** to a multiple of ten and clamped into
/// `LOCK_RANGE_STEP_M..=CONTACT_RANGE_STEP_M`: a plot up to 647 ms old keeps
/// 10 m, a one-second plot gets 20 m, two seconds gets 40 m, and about two and a
/// half seconds and older reads at the ordinary scope step, because a lock on a
/// plot that stale is not a better fire-control solution than any other row.
///
/// **It publishes nothing new.** The one number it is a function of — the
/// `age_ms` on the contact's own row — is already on the frame the pilot holds,
/// and is the same `1 s` `CONTACTS` prints. It is deliberately **not** priced off
/// `[tacsim.sensors].pass_ms`, which a pilot has never been sent.
///
/// The **metres** are a different matter and a client must not compute them: a
/// contact row publishes `range_m` already rounded to [`CONTACT_RANGE_STEP_M`],
/// so re-rounding that to this step gives the coarse figure back (450 m where the
/// lock reads 440). The lock's own metres are published once, as the frame's
/// `lock_range_m`, and `RANGE` prints the same call of the same function — which
/// is the whole of `T-20`'s *one function* rule.
#[must_use]
pub fn lock_range_step_m(age_ms: i64) -> u64 {
    let drift_m = age_ms.max(0).saturating_mul(PLOT_DRIFT_M_PER_S) / 1_000;
    let step = drift_m.saturating_add(9) / 10 * 10;
    u64::try_from(step)
        .unwrap_or(CONTACT_RANGE_STEP_M)
        .clamp(LOCK_RANGE_STEP_M, CONTACT_RANGE_STEP_M)
}

/// A contact's range in metres, rounded to the nearest `step` (16b cut F.3;
/// 16b cut Q.1 made the step an argument). Half a step rounds up.
///
/// The step is the caller's because there are two: [`CONTACT_RANGE_STEP_M`] for
/// a row on the scope, [`LOCK_RANGE_STEP_M`] for the one contact the platform is
/// locked on.
///
/// **The answer is never nought** (16b cut D.1). Rounding to the nearest step put
/// everything inside half a step at `0 m` — under 25 m on a scope row, under 5 m
/// on a lock — and `0 m` is the one distance a contact cannot be at: a platform
/// that close is being rammed. It floors at one step, so the coarsest thing the
/// row can say about a contact on top of you is *one step away*, which is what a
/// coarse reading is entitled to say.
#[must_use]
pub fn range_m_rounded(range_cm: u64, step_m: u64) -> u64 {
    let step = step_m.max(1);
    let metres = range_cm / 100;
    metres
        .saturating_add(step / 2)
        .saturating_div(step)
        .saturating_mul(step)
        .max(step)
}

/// **What this platform's own scope holds** (16b cut D.1; doc 16 §5, §1).
///
/// `STATUS`'s third line ended `; the arena is running {tempo}` — the arena's one
/// **global** word, raised when anybody anywhere on the map holds a contact. A
/// pilot sitting in a wreck with an empty scope read `engaged`, which is a fact
/// about somebody else's fight and exactly the shape of thing doc 16 §1 says a
/// platform has to earn. The three words are read off this platform's own contact
/// list and nothing else:
///
/// * `hostile` — it holds a contact classified to another team;
/// * `unidentified` — it holds a contact that is not a classified friend;
/// * `clear` — it holds nothing, or nothing but classified friends.
///
/// The arena's `tempo` stays on the frame, because a panel's own timers read the
/// cadence off it.
#[must_use]
pub fn own_scope(frame: &Frame) -> &'static str {
    let mut unidentified = false;
    for held in &frame.contacts {
        match held.class.team() {
            Some(team) if team == frame.team => {}
            Some(_) => return "hostile",
            None => unidentified = true,
        }
    }
    if unidentified {
        "unidentified"
    } else {
        "clear"
    }
}

/// The word a class renders as: `unknown`, or the **side** the platform has
/// resolved (doc 16 §3.6: *class or `unknown`*).
///
/// Not `Classification::as_str`, which answers the *state* (`classified`): what a
/// pilot wants off a resolved return is whose it is.
#[must_use]
fn class_word(class: Classification) -> &'static str {
    match class.team() {
        Some(team) => team.as_str(),
        None => class.as_str(),
    }
}

/// A bearing read off the platform's own nose: the absolute bearing less the
/// platform's heading, wrapped into `0..360_000` millidegrees.
#[must_use]
fn relative_mdeg(heading_mdeg: u32, bearing_mdeg: u32) -> u32 {
    let turn = i64::from(bearing_mdeg) - i64::from(heading_mdeg);
    u32::try_from(turn.rem_euclid(360_000)).unwrap_or(0)
}

/// **The range band** (16b cut D.1; doc 16 §3.6's *range band*, read against what
/// this platform can actually see).
///
/// **There is one band word, and this is it** (16b cut E.1, 16c `T-81`). Cut D.1
/// left a second — `range_band`, thirds of the *weapon's* innermost band, which
/// the commander's brief and an order's ceiling read — so a lead's `close` was
/// 500 m and a pilot's was 200 m. Cut E.1 deleted it; this function is what the
/// contact row, the acquisition notice, the squad brief and
/// [`crate::deliberation::order::RangeBand::ceiling_cm`] (its inverse, pinned by
/// a test) all say.
///
/// `CONTACTS` could only ever print two of its four words. The stock cannon's
/// bands are 1.5 km and 3 km, so a word banded on the gun alone put `close` at
/// 500 m and `effective` at 1,500 m — while the shipped sight is 60 cells of
/// 1,000 cm, 600 m, and one mesh hop adds 300 m. The furthest plot any shipped
/// platform can hold is 900 m, so from acquisition to the kill a contact read
/// `effective` and then `close`, and `far` and `beyond` could not print at all.
///
/// So the word is banded against **the lesser of the sight and the weapon's last
/// band**, which on every shipped platform is the sight. That makes it **scope
/// advice**: how far across your own sight the contact is — `close` the nearest
/// third, `effective` the middle third, `far` the outer third and anything a
/// teammate is passing you from beyond it. It is a deliberate change of what the
/// word *answers*, not a side effect: a contact at 450 m, which the cannon makes
/// comfortably, reads `far`. What the gun makes of the distance is the lock's
/// `% to hit` and `WEAPONSTATUS`, and `docs/help/cockpit.md` says so in the
/// pilot's own terms.
///
/// The rule, in order:
///
/// * no bands at all, or `range_cm` past the last band: `beyond` — still literally
///   true, because nothing the platform fires reaches it, **and the honest word
///   for a platform that carries nothing that fires**;
/// * otherwise `limit` is `min(last band, sight_cm)`, and the last band alone when
///   `sight_cm` is `0` (a platform with no sensor, which holds no contacts anyway);
/// * `range_cm × 3 ≤ limit`: `close`; else `range_cm × 3 ≤ limit × 2`:
///   `effective`; else `far`.
///
/// On the stock tank that is `close` to 200 m, `effective` to 400 m, `far` beyond
/// — and a mesh plot past the platform's own sight is `far`.
#[must_use]
pub fn contact_band(range_cm: u64, bands_cm: &[u32], sight_cm: u32) -> &'static str {
    let Some(last) = bands_cm.last() else {
        return "beyond";
    };
    let last = u64::from(*last);
    if range_cm > last {
        return "beyond";
    }
    let sight = u64::from(sight_cm);
    let limit = if sight == 0 { last } else { last.min(sight) };
    if range_cm.saturating_mul(3) <= limit {
        "close"
    } else if range_cm.saturating_mul(3) <= limit.saturating_mul(2) {
        "effective"
    } else {
        "far"
    }
}

/// One contact, as a pilot is told about it (doc 16 §3.6 "What a pilot sees").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactRow {
    /// The opaque arena id. **16b cut Q.1**: the stable row key a panel
    /// re-identifies a row by, never printed and never typed — see
    /// [`ContactRow::designator`] and [`resolve_designator`].
    pub id: ContactId,
    /// **The short name the pilot reads and types** (16b cut Q.1): `BA` while the
    /// contact is classified now, `X2` while it is not.
    pub designator: Designator,
    /// **The provisional number this row's target still answers to** (16b cut
    /// Q.1), where the row is naming it by slot and a number was issued. It is
    /// what makes `LOCK X2` keep working after the rename.
    pub alias: Option<u16>,
    /// `unknown`, or the side the platform has resolved.
    pub class: SmolStr,
    /// Bearing, degrees clockwise from north.
    pub bearing_deg: u32,
    /// The same bearing off the platform's own nose, degrees clockwise (16b cut
    /// S.2): `0` is dead ahead.
    pub relative_deg: u32,
    /// The range band's label ([`contact_band`], which since 16b cut E.1 is the
    /// one band word a pilot, a commander and an order all read).
    pub band: SmolStr,
    /// **The range to the plot, metres**, rounded to [`CONTACT_RANGE_STEP_M`]
    /// (16b cut F.3) — see [`range_m_rounded`] for why a passive sight reports one
    /// at all.
    pub range_m: u64,
    /// **How stale the plot is**, milliseconds of arena time (16b cut F.3): now
    /// less the sighting that last refreshed the contact, this platform's own or
    /// the sharer's. Cut S.2 printed the hold time since first detection, which
    /// carried no staleness signal at all.
    pub age_ms: i64,
    /// The cell it was last seen in, for `VIEW`'s marker (16b cut S.2).
    pub cell: Option<(u16, u16)>,
    /// Whether the resolved side is the platform's own — what decides the glyph
    /// `VIEW` draws it with. `false` while the contact is `unknown`.
    pub friendly: bool,
}

impl ContactRow {
    /// The line `CONTACTS` prints for this row.
    ///
    /// **16b cut F.3** put the range in it, between the band and the age, and
    /// **16b cut Q.1** put the short designator where the sixteen digits were,
    /// left-aligned in a column of four so `X2` and `BA` line up with `RAA`:
    /// `BA  blue  bearing 090° (rel 000°)  close  300 m  1 s`.
    #[must_use]
    pub fn render(&self) -> String {
        // Rendered first: `Designator`'s own `Display` writes straight to the
        // formatter, so a width on it would be ignored.
        let named = self.designator.to_string();
        format!(
            "{named:<4}{}  bearing {:03}° (rel {:03}°)  {}  {} m  {} s",
            self.class,
            self.bearing_deg,
            self.relative_deg,
            self.band,
            self.range_m,
            self.age_ms / 1_000
        )
    }

    /// The glyph `VIEW` draws this contact with: [`UNKNOWN_GLYPH`] while it is
    /// unresolved, then [`FRIEND_GLYPH`] or [`HOSTILE_GLYPH`].
    #[must_use]
    pub fn glyph(&self) -> &'static str {
        if self.class == "unknown" {
            UNKNOWN_GLYPH
        } else if self.friendly {
            FRIEND_GLYPH
        } else {
            HOSTILE_GLYPH
        }
    }
}

/// **The line one notice prints** (16b cut S.2, extended by 16b cut F.2; doc 16
/// §3.6: *notices fire once per transition*, §3.4 step 4, §5: *`Notice` events for
/// transitions and refusals*).
///
/// The one renderer, so the bridge's output line, the frame's `notices` array and
/// any later surface cannot word the same transition three ways. `rows` is the
/// frame's own contact list, which is where an acquisition's bearing and band come
/// from; a `lost` contact is not in it any more and is named by its id alone.
/// `frame` is the frame the notice rides — a lock's percentage is the one this
/// pulse resolved, read off the very rows `WEAPONSTATUS` prints, so the two can
/// never be two numbers. Every sentence is otherwise built from the notice's
/// **own** fields: the frame is a source of numbers and never of identities, so
/// two locks that resolved in one pulse each name the contact they were about.
///
/// **The observer rule is checked sentence by sentence here.** A shooter's line
/// names the contact its own platform holds and never a unit; a victim's line
/// names the victim's own section and facet and carries no id, no bearing, no team
/// and no weapon; a blocked line names a cause and never a cell's occupant.
///
/// **16b cut Q.1**: every contact is named by its short designator and never by
/// the sixteen digits, and **one notice renders to nothing** — a `classified`
/// whose contact was never provisional, which is a mesh arrival whose
/// acquisition already read `New contact RB: …`. An empty answer means *say
/// nothing*, and [`notice_lines`] is where that is honoured.
#[must_use]
pub fn notice_line(frame: &Frame, notice: &FrameNotice, rows: &[ContactRow]) -> String {
    match notice {
        FrameNotice::Contact {
            kind,
            contact,
            class,
            was,
        } => {
            let row = rows.iter().find(|row| row.id == contact.id);
            let named = contact.designator;
            match kind {
                NoticeKind::Acquired => match row {
                    Some(row) => format!(
                        "New contact {named}: bearing {:03}°, {}.",
                        row.bearing_deg, row.band
                    ),
                    None => format!("New contact {named}."),
                },
                // **16b cut Q.1**: the rename, in one sentence, quoting what the
                // pilot has been calling it. A classification that was never
                // provisional — a mesh arrival, or a match run with
                // `designate_on_detection` on — says **nothing**: its acquisition
                // already read `New contact RB: …`, and a second line renaming
                // `RB` to `RB` is noise.
                NoticeKind::Classified => match was {
                    Some(was) => {
                        format!("Contact {was} identified: {named}, {}.", class_word(*class))
                    }
                    None => String::new(),
                },
                // **16b cut D.4** (`B-20`): the other direction of the same
                // sentence. A contact that was told apart and has come back a
                // number is quoted under both names, because the pilot has been
                // typing the first one — and the first one stops working the
                // moment this line is printed, which is the whole reason it is
                // printed. It is **not** an alias: see `resolve_designator`.
                NoticeKind::Unidentified => match was {
                    Some(was) => format!("{was} is unidentified again: {named}."),
                    None => String::new(),
                },
                // 16b cut W.1: the lock broke because the contact stopped being
                // held. Said at once, because the pilot's next `FIRE` is going to
                // be refused for it.
                NoticeKind::LockLost => format!("Lock on {named} broken: contact gone."),
                // `Lost`, and anything a later cut puts in the contact arm
                // without a sentence of its own.
                _ => format!("Contact {named} lost."),
            }
        }
        // 16b cut F.2: doc 16 §3.5 step 5's percentage, said when the lock
        // resolves. The same sentence `WEAPONSTATUS` prints, from the same place.
        //
        // The percentage is quoted **only** when the frame's lock is still this
        // notice's contact. `LOCK` is a discrete, queued intent, so two of them
        // drain in one pulse and the frame carries the second one's lock: a
        // superseded lock's line would otherwise be rendered with the chance
        // against a contact the pilot has already stopped aiming at, and both
        // lines would name the same id. The bare sentence is the honest answer
        // for a lock that was taken and then replaced inside one pulse.
        FrameNotice::Locked { contact } => {
            let named = contact.designator;
            match frame_lock(frame) {
                Some(held) if held == contact.id => {
                    lock_line(frame).unwrap_or_else(|| format!("Locked on {named}."))
                }
                _ => format!("Locked on {named}."),
            }
        }
        // **16b cut Q.2**: the section word, where the shooter has earned one —
        // the platform holds the target and has told it apart. `Hit on BA: turret.`
        // against `Hit on X2.` against `Hit.`, which are the three things a gunner
        // can honestly be told about where a round went.
        // **16b cut D.2**: `(FRIENDLY)` after the designator (`B-18`), and the
        // one clause a gunner who has told the target apart earns when the round
        // found a section that was already gone (`B-15`). A gunner holding the
        // target *unidentified* carries no section and no `spent`, so his line is
        // byte for byte what it was.
        FrameNotice::Hit {
            contact,
            section,
            spent,
        } => {
            let already = if *spent { ", already destroyed" } else { "" };
            match (contact, section) {
                (Some(contact), Some(section)) => format!(
                    "Hit on {}{}: {section}{already}.",
                    contact.designator,
                    friendly_mark(frame, contact.id)
                ),
                (Some(contact), None) => format!(
                    "Hit on {}{}.",
                    contact.designator,
                    friendly_mark(frame, contact.id)
                ),
                (None, _) => "Hit.".to_owned(),
            }
        }
        FrameNotice::Miss { contact, reason } => match contact {
            Some(contact) => format!("Miss on {}: {}.", contact.designator, miss_words(reason)),
            None => format!("Miss: {}.", miss_words(reason)),
        },
        FrameNotice::Kill { contact } => match contact {
            Some(contact) => format!("{} destroyed.", contact.designator),
            None => "Target destroyed.".to_owned(),
        },
        FrameNotice::Struck {
            section,
            armor_section,
            facet,
            armor_left,
            armor_max,
            internals_left,
            internals_max,
        } => {
            // Doc 16 §3.3: a section with no armour of its own is **struck
            // through** another's, and the armour numbers are then that other
            // section's cell. The sentence says so rather than hanging a foreign
            // facet off the section that took the round — a pilot who reads
            // `the engine's rear facet` on a tank whose engine has no facets is
            // being told something about their own machine that is not true.
            let place = if armor_section == section {
                format!("the {section}'s {facet} facet")
            } else {
                format!("the {section}, through the {armor_section}'s {facet} facet")
            };
            if *armor_left > 0 {
                format!("Hit on {place}: {armor_left} of {armor_max} armour left.")
            } else {
                format!("Hit on {place}: armour gone, {internals_left} of {internals_max} inside.")
            }
        }
        // **16b cut D.2** (`B-15`): the round found a section that was already
        // destroyed, so there are no numbers left to report and one sentence says
        // the whole of it — at most once every `SPENT_NOTICE_MS` per section, so
        // six of these in ten seconds is one.
        FrameNotice::StruckSpent { section } => {
            format!("Hit on the {section}, which is already destroyed.")
        }
        FrameNotice::SectionLost {
            section,
            consequences,
        } => {
            let cost = consequence_words(consequences);
            if cost.is_empty() {
                format!("The {section} is destroyed.")
            } else {
                format!("The {section} is destroyed: {cost}.")
            }
        }
        // **16b cut Q.2**: the gunner's half. The victim's line above says what it
        // cost *them*; this one says only that the section went, because what a
        // lost track does to a tank is the crew's own knowledge and a gunner
        // watching from 400 m has not read it.
        FrameNotice::TargetSectionLost { contact, section } => {
            format!("{}'s {section} is destroyed.", contact.designator)
        }
        FrameNotice::Refused {
            order,
            refusal,
            weapon,
            detail,
        } => refusal_line(order, *refusal, weapon.as_deref(), *detail),
        FrameNotice::Blocked { cause } => match cause {
            BlockedCause::Impassable => "Held: the ground ahead will not take the tank.".to_owned(),
            BlockedCause::Occupied => "Held: something is in the way.".to_owned(),
            BlockedCause::Edge => "Held: this is the edge of the arena.".to_owned(),
            // 16b cut X.6: the ground ahead would take the tank; the slope onto
            // it will not. The difference is worth a sentence of its own, because
            // what a pilot does about it is drive round rather than give up.
            BlockedCause::TooSteep => "Held: the slope ahead is too steep for the tank.".to_owned(),
        },
        // **16b cut Q.1**: the far end of `STARTUP`. Three words, because the
        // only thing a pilot waiting on the sequence wants is to know it is over.
        FrameNotice::PowerOnline => "Power online.".to_owned(),
        // **16b cut D.2** (`B-03`): the pilot's own tank, and nothing about who
        // did it. Both sentences say what the pilot may do next, because the
        // thing the smoke matches found is somebody sitting in a dead tank with
        // no idea whether the match was still going on.
        FrameNotice::OwnDestroyed => OWN_DESTROYED.to_owned(),
        FrameNotice::OwnDisabled => OWN_DISABLED.to_owned(),
        // **16b cut D.4** (`B-22`): the pilot's own machine. It says that the
        // sight is being interfered with and nothing whatever about the pod — no
        // source, no bearing, no side, no id — which is 16c §5 Q17's own sentence.
        FrameNotice::Receiver { degraded } => if *degraded {
            RECEIVER_DEGRADED
        } else {
            RECEIVER_CLEAR
        }
        .to_owned(),
    }
}

/// **`(FRIENDLY)` where this frame's own row for `contact` says so** (16b cut
/// D.2, `B-18`), and the empty string everywhere else.
///
/// Read off the frame's rows at render time and never carried on the notice: a
/// contact's side is a thing the platform resolves and may re-resolve, and an
/// unidentified return has no side at all — so an `unknown` row is never marked,
/// by construction.
fn friendly_mark(frame: &Frame, contact: ContactId) -> &'static str {
    let friendly = frame
        .contacts
        .iter()
        .find(|held| held.id == contact)
        .is_some_and(|held| held.class.team() == Some(frame.team));
    if friendly { FRIENDLY_MARK } else { "" }
}

/// **The line a pilot reads when their own tank becomes a wreck** (16b cut D.2,
/// `B-03`).
pub const OWN_DESTROYED: &str = "Your tank is destroyed. The match runs on without you: wait it \
                                 out, EXIT to the team room, or SIM ABORT to call it off.";

/// **The line a pilot reads when their own tank can neither move nor shoot**
/// (16b cut D.2, `B-03`).
pub const OWN_DISABLED: &str = "Your tank can neither move nor shoot. The match runs on: wait it \
                                out, EXIT to the team room, or SIM ABORT to call it off.";

/// **The line a pilot reads when their own receiver goes into interference** (16b
/// cut D.4, `B-22`).
///
/// It names no source, because the platform has none to name: the fold is read at
/// the pilot's own cell and nothing about the emitter reaches it
/// ([`crate::arena::perception::Receiver`]).
pub const RECEIVER_DEGRADED: &str = "Receiver degraded: something is jamming the sight.";

/// **The line a pilot reads when their own receiver comes back out of it** (16b
/// cut D.4, `B-22`).
pub const RECEIVER_CLEAR: &str = "Receiver clear.";

/// **`STATUS`'s receiver line** (16b cut D.4, `B-22`), split out of the verb the
/// way [`range_line`] is, so the one sentence can be pinned against a value
/// rather than against a booted server.
///
/// The budget the platform's last pass really ran with against what that budget
/// is when nothing is jamming, and what is left of the reach. Three integers
/// about the pilot's own machine and **nothing about the emitter**: no source, no
/// bearing, no side, no cell, no object number and no designator, which is 16c
/// §5 Q17 read literally. See [`crate::arena::perception::Receiver`] for the two
/// limits the integers are declared to have.
#[must_use]
pub fn receiver_line(receiver: crate::arena::perception::Receiver) -> String {
    format!(
        "  receiver degraded: {} of {} through cover, {}% reach",
        receiver.budget,
        receiver.budget_max,
        receiver.reach_permille / 10
    )
}

/// **A miss reason in plain words** (16b cut F.2), one per
/// [`crate::arena::damage::MISS_REASONS`] — the ledger's own vocabulary read as a
/// gunner hears it. An unknown word is printed as it stands rather than swallowed,
/// so a seventh reason is a sentence somebody has to come and write.
fn miss_words(reason: &str) -> &str {
    match reason {
        damage::TARGET_GONE => "the target is gone",
        damage::WRECK => "the target is already a wreck",
        damage::MOVED => "the target moved",
        damage::BLOCKED => "the line was blocked",
        damage::COVER => "the target is behind cover",
        damage::DISPERSION => "the round went wide",
        damage::REFUSED => "the round found nothing to break",
        other => other,
    }
}

/// **What a lost section costs the tank, in plain words** (16b cut F.2), joined in
/// authored order.
///
/// The words are `crate::arena::resolution::Consequence::label`'s, so the pilot's
/// sentence and the black box name the same things. The **list** is not the
/// ledger line's: `crate::arena::damage::Arena::fired_consequences` has already
/// dropped the one consequence that can be authored and not fire — a biped's
/// `disable_motion_when_all_of` on the first of its two legs — so every word
/// joined here is true of the tank as it now stands.
fn consequence_words(consequences: &[SmolStr]) -> String {
    consequences
        .iter()
        .map(|label| match label.as_str() {
            "destroy_unit" => "the tank is finished",
            "remove_weapons" => "its weapons are gone",
            "slow" => "the tank is slowed",
            "no_turn_toward" => "it will not turn that way",
            "disable_motion" | "disable_motion_when_all_of" => "the tank cannot move",
            other => other,
        })
        .collect::<Vec<&str>>()
        .join("; ")
}

/// **The line an arena-side refusal prints** (16b cut F.2).
///
/// It is the **same sentence** the cockpit's own pre-check prints for the same
/// condition, built from the same [`CockpitError`] — so the answer a pilot gets
/// when the frame was fresh enough to refuse locally and the answer they get a
/// pulse later are one string, and neither can drift. The prefix names the order,
/// because a refusal that arrives a pulse after the pilot typed something has to
/// say what it is about.
///
/// **Every [`Refusal`](crate::arena::intents::Refusal) variant is matched by
/// name** (16b cut F.3). Cut F.2 ended the match with a catch-all that formatted
/// the `thiserror` text, so a later cut that routed a new variant into phase 4
/// would have shipped a pilot the sentence `FIRE refused. Refused: that unit
/// already has as many actions queued as it may.` — lower-case, doubly prefixed,
/// in nobody's voice. A new variant is now a compile error instead, which is a
/// sentence somebody has to come and write.
pub(crate) fn refusal_line(
    order: &str,
    refusal: crate::arena::intents::Refusal,
    weapon: Option<&str>,
    detail: RefusalDetail,
) -> String {
    use crate::arena::intents::Refusal;
    let said = match (refusal, detail) {
        (Refusal::NoContact, _) => CockpitError::NoSuchContact.to_string(),
        (Refusal::NoLock, _) => CockpitError::NoLock.to_string(),
        (Refusal::Cycling, RefusalDetail::Cycling { secs }) => {
            CockpitError::Cycling { secs }.to_string()
        }
        (Refusal::Cycling, _) => CockpitError::Cycling { secs: 0 }.to_string(),
        (Refusal::Empty, _) => CockpitError::Empty.to_string(),
        (Refusal::Overheated, RefusalDetail::Overheated { heat, max }) => {
            CockpitError::Overheated { heat, max }.to_string()
        }
        // 16b cut F.3: a sentence, not the bare word. `OVERHEATED` is the column
        // heading `WEAPONSTATUS` prints and the opener `CockpitError::Overheated`
        // builds on, and a refusal staged before the heat cell was read has no
        // numbers to put after it — so it says the half of that sentence that is
        // still true rather than trailing off.
        (Refusal::Overheated, _) => format!("{OVERHEATED}: let it cool."),
        (Refusal::PowerNotOnline, _) => CockpitError::PowerNotOnline.to_string(),
        (Refusal::NoWeapon, _) => {
            CockpitError::NoSuchWeapon(weapon.unwrap_or_default().to_owned()).to_string()
        }
        // The rest cannot be produced by a pilot's own order today — a wreck and a
        // unit the arena has let go are refused at phase 2 unless the tank died
        // between the queue and the apply, which is exactly when a pilot most
        // wants to be told. One short sentence each, in the same voice.
        (Refusal::Wreck, _) => "Your tank is a wreck.".to_owned(),
        (Refusal::Unknown, _) => "That tank is no longer in the match.".to_owned(),
        (Refusal::Epoch, _) => "Your seat changed hands; try that again.".to_owned(),
        (Refusal::Channel, _) => "That order does not travel on this station.".to_owned(),
        (Refusal::QueueFull, _) => "Too many orders at once; try that again.".to_owned(),
        (Refusal::MotionDisabled, _) => "Your tank cannot move.".to_owned(),
        (Refusal::Linkdead, _) => "Your seat has gone linkdead.".to_owned(),
        (Refusal::NotReflex, _) => "That tank has a pilot in it.".to_owned(),
    };
    order_refused(order, weapon, &said)
}

/// **The one shape every refused order's sentence has** (16b cut D.2, `B-16`):
/// `HEADING refused. …`, and `FIRE machine_gun refused. …` where the order named
/// a weapon.
///
/// Cut F.2's prefix was the order word alone, so a pilot who carried two guns
/// read `FIRE refused. Still cycling: 3 s to go.` and had to guess which breech
/// was busy — and the cockpit's own pre-check said a *different* sentence for the
/// same condition, with no prefix at all. Both halves compose through here now,
/// so the early answer and the late one are one string and a pilot who fires two
/// weapons reads which one was refused.
///
/// The weapon goes **after the order word and before `refused`**, because that is
/// where the pilot typed it.
#[must_use]
pub fn order_refused(order: &str, weapon: Option<&str>, said: &str) -> String {
    match weapon {
        Some(weapon) if !weapon.trim().is_empty() => {
            format!("{} {weapon} refused. {said}", order.to_uppercase())
        }
        _ => format!("{} refused. {said}", order.to_uppercase()),
    }
}

/// **A `FIRE` the cockpit refused against its own mirror** (16b cut D.2,
/// `B-16`), in the sentence the arena would have said a pulse later.
///
/// The `said` half is the [`CockpitError`]'s own `Display`, which is where each
/// of those sentences already lives — so this adds the prefix and invents
/// nothing.
fn fire_refused(weapon: &str, said: &CockpitError) -> CockpitError {
    CockpitError::Refused(order_refused(
        IntentKind::Fire {
            weapon: SmolStr::new(weapon),
        }
        .as_str(),
        Some(weapon),
        &said.to_string(),
    ))
}

/// Every line the notices on one frame print, in order — what the bridge says and
/// what a read verb's frame carries, from one place.
///
/// **16b cut Q.1**: a notice that renders to nothing is dropped rather than said
/// as a blank line (see [`notice_line`]). The filter is here, at the one funnel
/// every spoken line goes through, so there is one rule and not one per caller.
#[must_use]
pub fn notice_lines(frame: &Frame) -> Vec<String> {
    let rows = frame_contacts(frame);
    frame
        .notices
        .iter()
        .map(|notice| notice_line(frame, notice, &rows))
        .filter(|line| !line.is_empty())
        .collect()
}

/// **The lock the platform holds**, as the frame carries it (16b cut W.1 filled
/// it): always one of [`frame_contacts`]'s rows, because the arena clears a lock the
/// moment its contact stops being held.
#[must_use]
pub fn frame_lock(frame: &Frame) -> Option<ContactId> {
    frame.lock
}

/// **The line the arena's `Locked` notice and `WEAPONSTATUS` both print about the
/// lock** (16b cut W.2; doc 16 §3.5 step 5: *the cockpit shows `p_permille` as a
/// percentage in `WEAPONSTATUS` and on `LOCK` before any shot*).
///
/// The `LOCK` verb itself no longer prints it: 16b cut F.2 moved that to the
/// arena's own notice, said when the lock resolves, so there is one owner of the
/// sentence rather than a cockpit echo beside it (16b §0.2).
///
/// `Locked on 3f2c…, 32% to hit.` when the arena has resolved one, and `Locked on
/// 3f2c….` when it has not — which is the pulse between a `LOCK` being typed and the
/// frame that answers it, and a platform carrying no weapon at all. `None` for no
/// lock, so the caller says nothing rather than saying there is nothing.
///
/// The percentage is the row of **the weapon a bare `FIRE` would fire**
/// ([`fired_by_default`]), rounded to whole per cent: it is the same number the
/// draw will be compared against, resolved by the arena through doc 16 §3.5's five
/// steps with the trace and the cover in it, and never a second estimate computed
/// here.
///
/// **16b cut F.2** changed which number that is. Cut W.2 showed the **best** of
/// the platform's weapons, so a pilot who read `46% to hit` and reached for the
/// coaxial during the cannon's cycle fired at 21 %; doc 16 §3.5 step 5 defines
/// `p_permille` per shot, and the shot a bare `FIRE` takes is the cannon's. Each
/// weapon's own chance is on its `WEAPONSTATUS` row.
///
/// **What it is computed from** (16b cut F.3): the locked contact's **plot** —
/// where the platform last saw the target, and how it was facing and moving then
/// — through `crate::arena::resolution::Arena::lock_aim`, which resolves the
/// range and the cover, and
/// `crate::arena::resolution::Arena::resolve_seen`, which reads the face and the
/// crossing speed off the same sighting. So this sentence, every `WEAPONSTATUS`
/// row's percentage and the contact row's own band are three readings of one plot
/// and cannot be three numbers. The shot itself is resolved against where the
/// target *is*, so for a target that has moved since the sighting the advertised
/// chance and the chance the draw is compared against differ; that is what a
/// stale track costs, and `resolve_seen` has the argument.
#[must_use]
pub fn lock_line(frame: &Frame) -> Option<String> {
    // 16b cut Q.1: the short name, off the frame's own row for that contact — a
    // lock is always one of the rows, because the arena clears a lock the moment
    // its contact stops being held.
    let named = lock_designator(frame)?;
    // 16b cut D.2 (`B-18`): a lock on a friend says so, here and therefore on
    // `WEAPONSTATUS` too. Nothing is refused.
    let mark = frame_lock(frame).map_or("", |contact| friendly_mark(frame, contact));
    Some(
        match fired_by_default(&frame.weapons).and_then(|weapon| weapon.p_permille) {
            Some(permille) => format!("Locked on {named}{mark}, {}% to hit.", permille / 10),
            None => format!("Locked on {named}{mark}."),
        },
    )
}

/// **The designator of the contact the platform is locked on** (16b cut Q.1), or
/// `None` with no lock.
///
/// Read off the frame's own contact list rather than carried beside the lock, so
/// the name in `WEAPONSTATUS`'s lock line, in `RANGE`'s answer and in the row
/// `CONTACTS` prints is one string resolved once.
#[must_use]
pub fn lock_designator(frame: &Frame) -> Option<Designator> {
    let contact = frame_lock(frame)?;
    frame
        .contacts
        .iter()
        .find(|held| held.id == contact)
        .map(|held| held.designator)
}

/// **The weapon a bare `FIRE` would fire** (16b cut F.2): [`DEFAULT_WEAPON`], else
/// the first the platform carries — the same choice [`choose_weapon`] makes for an
/// empty argument, which is what makes `LOCK`'s percentage the percentage of the
/// shot the pilot is about to take.
fn fired_by_default(
    weapons: &[crate::arena::weapons::FrameWeapon],
) -> Option<&crate::arena::weapons::FrameWeapon> {
    weapons
        .iter()
        .find(|weapon| weapon.id.as_str() == DEFAULT_WEAPON)
        .or_else(|| weapons.first())
}

/// **Every weapon the platform carries, live** (16b cut W.1), as the arena's frame
/// carries it.
///
/// This is the reading that matters while a match runs: the arena is the single
/// writer of `ready_at_ms` and `ammo` (D-16-1), so the frame's numbers are the
/// truth and [`weapons_of`]'s structural read of the M32 component cells is the
/// answer for a unit no arena holds — a tank in the motor pool, or a pilot who has
/// boarded and whose arena has not spoken yet.
///
/// **An empty answer is an answer** (16b cut D.1): a frame that lists no weapon
/// means the platform carries none, and [`carried`] does not go looking for an
/// older one.
#[must_use]
pub fn frame_weapons(frame: &Frame) -> Vec<WeaponRow> {
    frame
        .weapons
        .iter()
        .map(|weapon| WeaponRow {
            id: weapon.id.clone(),
            ready: weapon.ready,
            ready_in_ms: weapon.ready_in_ms,
            rounds: i64::from(weapon.rounds),
            rounds_max: i64::from(weapon.rounds_max),
            range_band: weapon.range_band,
            p_permille: weapon.p_permille,
            // 16b cut D.2 (`B-16`): straight off the frame's own row.
            heat_per_shot: weapon.heat_per_shot,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// K2: `BOARD`
// ---------------------------------------------------------------------------

/// **`BOARD`** — climb into your pod and take the tank's cockpit (doc 16 §3.7).
///
/// See the module head for the order and why it is that order. Every refusal
/// happens before the first write; a seat that refuses after the body is in the
/// pod takes the body back out.
///
/// # Errors
///
/// [`CockpitError`], as the line the pilot sees.
pub fn board(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // **16b cut F.4**: the layer, first. A pod on a server with the layer off has
    // no tank behind it, so boarding one can only end in a refusal further down
    // the path; this is that refusal, said at the word the player typed.
    lobby::require_layer(ctx.w)?;
    let Some(CentreRoom::Team(team)) = centre::where_in_centre(ctx.w, actor) else {
        return Err(CockpitError::OutsideTeamRoom);
    };
    // Every refusal, read out of the centre in one borrow.
    let (match_id, pod, index) = {
        let centre = lobby::centre(ctx.w).ok_or(CockpitError::Unavailable)?;
        // The slot before the state, deliberately: somebody with no slot at all
        // is told the thing they can act on ("claim one"), not the thing they
        // cannot (what the centre happens to be doing).
        let slot = centre
            .slot_of_pilot(actor)
            .cloned()
            .ok_or(SimError::NoSlot)?;
        if centre.state != MatchState::Boarding {
            return Err(SimError::WrongState {
                state: centre.state.as_str(),
            }
            .into());
        }
        if slot.team != team {
            return Err(CockpitError::WrongRoom {
                team: slot.team.as_str(),
            });
        }
        if slot.is_seated() {
            return Err(CockpitError::AlreadyAboard);
        }
        let pod = slot.pod.ok_or(SimError::WrongState {
            state: centre.state.as_str(),
        })?;
        let match_id = centre.match_id.ok_or(SimError::WrongState {
            state: centre.state.as_str(),
        })?;
        (match_id, pod, slot.index)
    };
    let tank = centre::tank_of(ctx.w, pod).ok_or(SimError::NoUnit {
        team: team.as_str(),
        index,
    })?;
    let station = station_id().ok_or(CockpitError::Unavailable)?;
    let room = ctx.w.get(pod).and_then(|object| object.location);

    // (1) The station, on the TANK, with the POD as its console.
    mush_sim::declare_station(
        ctx,
        tank,
        &station,
        STATION_ROLE,
        &cockpit_channels(),
        Some(pod),
        STATION_PRIORITY,
    )
    .map_err(|error| CockpitError::Station(error.to_string()))?;

    // (2) The body, inside the pod. One seat, so no compartment: the pack
    // authors none, and naming one the space does not declare is a refusal.
    mush_sim::place_interior(ctx, actor, pod, None)
        .map_err(|error| CockpitError::Hatch(error.to_string()))?;

    // (3) The seat. On a refusal the body comes back out, because a pilot shut
    // in a pod with no cockpit is the one state this verb must not leave behind.
    if let Err(error) = mush_sim::request_seat_activation(
        ctx,
        &mush_sim::SeatRequest::Activate {
            unit: tank,
            station: station.clone(),
            // Not a proxy: the pilot flies their own tank, so the seated actor
            // and the driver are the same character and `driving_actor` is a
            // fixed point on them.
            npc: actor,
            granted: cockpit_channels(),
            fallback: None,
        },
    ) {
        if let Some(room) = room {
            put_out(ctx, actor, room);
        }
        return Err(CockpitError::Seat(error.to_string()));
    }
    // The activation already republished the seated actor's grants; this is the
    // idempotent restatement doc 16 §3.7 names, and it costs one recompute.
    mush_sim::links::refresh_actor_links(ctx.w, actor);

    // **16b cut F.1**: what this used to say was "the panel wakes: STARTUP,
    // then HEADING and SPEED" — and every one of those verbs is refused until
    // the last pilot boards, because nothing is deployed until the trigger
    // fires. The first pilot to board always typed `STARTUP` and was told they
    // were not at the station of a deployed unit. So the line says what is
    // actually true: the panel waits for the other pods, or for the deadline,
    // and leaving once the match is running is final.
    let waiting = lobby::centre(ctx.w)
        .and_then(|centre| centre.board_deadline_ms)
        .map(|deadline| {
            deadline
                .saturating_sub(ctx.w.now().saturating_mul(1_000))
                .max(0)
                / 1_000
        });
    let pilot_line = match waiting {
        Some(secs) => format!(
            "You climb into {}. The panel wakes when every pilot is seated, or in {secs} \
             second(s). EXIT to get out — once the match is running, leaving abandons the tank \
             for the rest of it.",
            display_name(ctx.w, pod)
        ),
        None => format!(
            "You climb into {}. The panel wakes when every pilot is seated. EXIT to get out — \
             once the match is running, leaving abandons the tank for the rest of it.",
            display_name(ctx.w, pod)
        ),
    };
    let room_line = format!(
        "{} climbs into {}.",
        display_name(ctx.w, actor),
        display_name(ctx.w, pod)
    );
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, pilot_line);
    if let Some(room) = room {
        tell_room(ctx, room, Vec::new(), room_line);
    }

    // (4) The centre, and — when this was the last seat — cut K.1's trigger.
    //
    // A refusal here is the check-out's own, which cut K.1 has already announced
    // and rolled back: the pilot keeps their pod and their seat (both harmless,
    // and `EXIT` gives them up), so the line is reported and nothing is undone.
    crate::match_::slot_seated(ctx, match_id, actor, tank)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// K2: `EXIT`
// ---------------------------------------------------------------------------

/// **`EXIT`** — climb out (doc 16 §3.7).
///
/// The order is the reverse of [`board`]'s, with the authority first:
///
/// 1. `Control { Released }` on all three channels, which raises the arena's
///    epoch and **purges whatever that seat had queued** — the reason a pilot who
///    types `FIRE` and `EXIT` in one burst fires nothing;
/// 2. the seat, through the same one seam that took it, which clears the
///    narrowing, relinquishes the focus and fires the station's declared fallback
///    if it has one;
/// 3. the body, back into the team room;
/// 4. the centre, when the match is still `Boarding`, so the trigger stops
///    counting a pod nobody is in.
///
/// # Errors
///
/// [`CockpitError`], as the line the pilot sees.
pub fn exit(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // **T11's guardrail** (16b cut K.2; `ProxyAct::Abandonment`'s station-lane
    // face, restored by this cut). A seat is tactical discretion: a driver lent
    // somebody else's crew member may not climb out of their unit and leave it
    // idling in a live match. A pilot in their own tank is not a proxy driver, so
    // the predicate stands aside for them and this costs them nothing.
    mush_sim::seat_restriction(ctx.w, actor, mush_sim::ProxyAct::Abandonment)
        .map_err(CockpitError::Guarded)?;
    let pod = pod_of_body(ctx.w, actor).ok_or(CockpitError::NotInAPod)?;
    let room = ctx.w.get(pod).and_then(|object| object.location);
    let station = station_id().ok_or(CockpitError::Unavailable)?;
    let seats: Vec<ObjId> = seats_of(ctx.w, actor)
        .into_iter()
        .filter(|(_, held)| *held == station)
        .map(|(unit, _)| unit)
        .collect();
    // 16b cut F.1: whether this is the one-way leave of doc 16 §11's N.1
    // amendment, read before the seat goes.
    let running = lobby::centre(ctx.w).is_some_and(|centre| centre.state == MatchState::Running);

    // (1) and (2): the arena hears first, then the seat is stood down and the
    // billet given back — all of it through [`hand_seat_back`], the one way this
    // crate lets a cockpit seat go.
    for unit in &seats {
        hand_seat_back(ctx, *unit, &station, actor, true);
    }
    // (3) The body. The pod is an unanchored interior space, so there is no
    // domain to `egress` onto: the placement is released where it stands and the
    // character walks back into the room the pod is in.
    if let Some(room) = room {
        put_out(ctx, actor, room);
    }
    // **16b cut F.1**: during `Running` this is final (doc 16 §11's N.1
    // amendment: *a released seat hands the tank to the reflex controller and the
    // pilot cannot reclaim it*), and the pilot is told so in the same breath
    // rather than discovering it when `BOARD` refuses. No confirmation step: the
    // verb does what it says, and what it says is now the whole truth.
    let line = match (running, seats.first()) {
        (true, Some(unit)) => format!(
            "You climb out of {}. {} is on its own for the rest of the match; you cannot take it \
             back.",
            display_name(ctx.w, pod),
            display_name(ctx.w, *unit)
        ),
        _ => format!("You climb out of {}.", display_name(ctx.w, pod)),
    };
    // The slot itself was marked `left` by [`hand_seat_back`]'s own `Released`
    // above, in [`crate::arena_bridge::send_control`], where every route to a
    // released seat passes — not here, because `EXIT` is only one of them.
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    if let Some(room) = room {
        let text = format!(
            "{} climbs out of {}.",
            display_name(ctx.w, actor),
            display_name(ctx.w, pod)
        );
        tell_room(ctx, room, vec![actor], text);
    }

    // (4) The centre, while it is still boarding.
    let boarding = lobby::centre(ctx.w).and_then(|centre| {
        (centre.state == MatchState::Boarding && centre.slot_of_pilot(actor).is_some())
            .then_some(centre.match_id)?
    });
    if let Some(match_id) = boarding {
        crate::match_::slot_vacated(ctx, match_id, actor)?;
    }
    Ok(())
}

/// Take one character out of the pod it is placed inside and stand it in `room`.
///
/// Two steps, because the pod is **unanchored**: the placement is released where
/// it is (`keep`, so nothing is projected to the recovery room) and the body is
/// then moved into the room the pod stands in. An unplaced object projects
/// nowhere, so boot's own projection reconcile leaves it exactly here.
fn put_out(ctx: &mut Ctx<'_>, actor: ObjId, room: ObjId) {
    if let Err(error) = mush_sim::unplace(ctx, actor, "left a simulator pod") {
        tracing::debug!(%actor, %error, "tacsim: a pilot's pod placement did not release");
    }
    let from = ctx.w.get(actor).and_then(|object| object.location);
    if from == Some(room) {
        return;
    }
    if let Err(error) = ctx.w.move_to(actor, room) {
        tracing::error!(%actor, %room, %error, "tacsim: a pilot could not leave their pod");
        return;
    }
    ctx.emit_world_event(&mush_core::WorldEvent::Moved {
        who: actor,
        from,
        to: room,
    });
}

/// **What one sweep gave back** (16b cut F.1).
///
/// Two lists rather than two counts, because the caller that reports to a
/// staffer wants to name what it found and the caller that decides whether
/// anything changed wants to ask one question.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EjectReport {
    /// The bodies taken out of a pod, in pod order.
    pub bodies: Vec<ObjId>,
    /// The cockpit billets given back, as `(tank, pilot)`, in pod order.
    pub seats: Vec<(ObjId, ObjId)>,
}

impl EjectReport {
    /// Whether the sweep found nothing at all — which is what an idle centre and
    /// a second `@arena/reset` both answer.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty() && self.seats.is_empty()
    }

    /// Fold one pod's findings into the whole.
    fn merge(&mut self, other: EjectReport) {
        self.bodies.extend(other.bodies);
        self.seats.extend(other.seats);
    }
}

/// **Give one cockpit seat back** (16b cut F.1) — the one way, wherever a seat
/// has to be let go.
///
/// Three steps, and the order is `EXIT`'s because `EXIT`'s was right:
///
/// 1. `Control { Released }` on all three channels, which raises the arena's
///    epoch and **purges whatever that seat had queued** — the reason a pilot who
///    types `FIRE` and `EXIT` in one burst fires nothing — and, since this cut,
///    takes the pilot off the arena's frame roster and off the registry's pilot
///    binding ([`send_control`]);
/// 2. the seat, stood down through the one seam that took it (the narrowing
///    cleared, the focus relinquished, the station's fallback fired);
/// 3. the **billet**, given back. A stand-down deliberately leaves the billet
///    where it is, which is right for an NPC's own station and wrong for a pod
///    lent for one match: the billet *is* the loan, and one left behind is the
///    next pilot's `StationHeld`.
///
/// `by_pilot` is whose choice it was. `Release` and `Revoke` reach one
/// `stand_down` in `mush-sim`; the variant is the statement, and a sweep after an
/// abort or at boot must use the one that does not require the absent driver's
/// account to still resolve.
///
/// Answers whether the billet went, which is the half that strands a pod.
pub(crate) fn hand_seat_back(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    station: &mush_sim::StationId,
    actor: ObjId,
    by_pilot: bool,
) -> bool {
    for channel in Channel::ALL {
        if let Err(error) = send_control(ctx, unit, channel, ControlState::Released, Some(actor)) {
            tracing::debug!(
                %unit,
                channel = channel.as_str(),
                %error,
                "tacsim: a released cockpit seat could not be announced to the arena"
            );
        }
    }
    let request = if by_pilot {
        mush_sim::SeatRequest::Release {
            unit,
            station: station.clone(),
            npc: actor,
        }
    } else {
        mush_sim::SeatRequest::Revoke {
            unit,
            station: station.clone(),
            npc: actor,
        }
    };
    if let Err(error) = mush_sim::request_seat_activation(ctx, &request) {
        tracing::debug!(
            %unit,
            %error,
            "tacsim: a cockpit seat did not stand down"
        );
    }
    match mush_sim::vacate_station(ctx, unit, station, actor) {
        // `false` is "there was no billet to give back", which is the answer a
        // second sweep gets and is not a failure.
        Ok(vacated) => vacated,
        Err(error) => {
            tracing::error!(
                %unit,
                %error,
                "tacsim: a cockpit billet could not be given back; the next pilot of that pod \
                 will be refused and staff must @spatial/unassign it"
            );
            false
        }
    }
}

/// Every actor holding the cockpit billet on `unit`, given it back (16b cut
/// F.1).
fn hand_back_seats_of_unit(ctx: &mut Ctx<'_>, unit: ObjId) -> EjectReport {
    let Some(station) = station_id() else {
        return EjectReport::default();
    };
    let held: Vec<ObjId> = ctx
        .w
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.assignments.get(&unit))
        .map(|host| {
            host.seats
                .iter()
                .filter(|(at, _)| **at == station)
                .map(|(_, seat)| seat.actor)
                .collect()
        })
        .unwrap_or_default();
    let mut report = EjectReport::default();
    for actor in held {
        if hand_seat_back(ctx, unit, &station, actor, false) {
            report.seats.push((unit, actor));
        }
    }
    report
}

/// Every cockpit billet one actor holds, given back (16b cut F.1).
fn hand_back_seats_of_actor(ctx: &mut Ctx<'_>, actor: ObjId) -> EjectReport {
    let Some(station) = station_id() else {
        return EjectReport::default();
    };
    let units: Vec<ObjId> = seats_of(ctx.w, actor)
        .into_iter()
        .filter(|(_, held)| *held == station)
        .map(|(unit, _)| unit)
        .collect();
    let mut report = EjectReport::default();
    for unit in units {
        if hand_seat_back(ctx, unit, &station, actor, false) {
            report.seats.push((unit, actor));
        }
    }
    report
}

/// **Open every pod, give back every cockpit billet, and stand whoever is
/// inside in the team room** (16b cut F.1; doc 16 §3.8: *seats, leases, focus and
/// pods released atomically*).
///
/// This replaces cut K.2's `disembark_all`, which did the **body** half alone.
/// That was the bug the 2026-09-18 review found in four shapes at once: an abort
/// during `Boarding`, the board deadline emptying a side, a `SIM LEAVE` by a
/// seated pilot and a restart mid-match all left the persisted `assignment:<tank>`
/// billet behind, and `@arena/reset` then moved the body out without it — after
/// which the pilot could not even `EXIT`, and the next pilot of that pod was
/// refused `StationHeld` forever. A sweep that opens a hatch and leaves the seat
/// held is not a release.
///
/// Keyed on the **pods** and on their tanks rather than on the centre's slots, so
/// it is equally the repair a staff `@arena/reset` performs on a centre some
/// earlier failure left with somebody aboard, and the boot pass's answer to a
/// restart. Idempotent, and fallible per pod without failing the others.
pub(crate) fn eject_all(ctx: &mut Ctx<'_>) -> EjectReport {
    let pods: Vec<(ObjId, Option<ObjId>, Option<ObjId>)> = centre::POD_ENTRIES
        .iter()
        .filter_map(|(team, index, _)| centre::pod_object(ctx.w, *team, *index))
        .map(|pod| {
            (
                pod,
                ctx.w.get(pod).and_then(|object| object.location),
                centre::tank_of(ctx.w, pod),
            )
        })
        .collect();
    let mut report = EjectReport::default();
    for (pod, room, tank) in pods {
        // The billets on the pod's own tank, whether or not anybody is still
        // sitting in the pod: the two halves are stranded independently.
        if let Some(tank) = tank {
            report.merge(hand_back_seats_of_unit(ctx, tank));
        }
        let Some(room) = room else {
            continue;
        };
        let aboard: Vec<ObjId> = ctx
            .w
            .get(pod)
            .map(|object| object.contents.clone())
            .unwrap_or_default();
        for body in aboard {
            if ctx
                .w
                .get(body)
                .is_none_or(|object| object.kind != mush_core::ObjKind::Player)
            {
                continue;
            }
            // A pilot who wandered into somebody else's pod still holds their own
            // tank's billet; the pod's tank alone would not find it.
            report.merge(hand_back_seats_of_actor(ctx, body));
            put_out(ctx, body, room);
            let line = format!("The hatch of {} swings open.", display_name(ctx.w, pod));
            ctx.out.sys_to_actor(body, NOTICE_CATEGORY, line);
            report.bodies.push(body);
        }
    }
    report
}

/// **The per-pilot form of [`eject_all`]** (16b cut F.1): one pilot's cockpit
/// billet given back and one pilot's body put out of whatever pod it is in.
///
/// `SIM LEAVE` by a seated pilot is what wants it — dropping the slot while the
/// pilot sits in the pod holding the tank's billet is what made the pod
/// unboardable for the next crew — and so is any other place that has one person
/// in hand rather than a centre.
pub(crate) fn eject_pilot(ctx: &mut Ctx<'_>, actor: ObjId) -> EjectReport {
    let mut report = hand_back_seats_of_actor(ctx, actor);
    let Some(pod) = pod_of_body(ctx.w, actor) else {
        return report;
    };
    let Some(room) = ctx.w.get(pod).and_then(|object| object.location) else {
        return report;
    };
    put_out(ctx, actor, room);
    let line = format!("The hatch of {} swings open.", display_name(ctx.w, pod));
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    report.bodies.push(actor);
    report
}

// ---------------------------------------------------------------------------
// K3: the twelve cockpit verbs
// ---------------------------------------------------------------------------

/// `STARTUP` — bring power up (doc 16 §3.3).
///
/// **16b cut D.1: it reads the mirror first, and it stamps a hint.** It used to
/// forward the intent whatever the power was and say `Startup sequence begun;
/// power comes online shortly.` — so a second `STARTUP` on a tank that was
/// already running said it again and the pilot waited for a `Power online.` that
/// had already been said and would never come again. Three answers now:
///
/// * [`Power::Online`]: `Power is already online.`, and nothing is sent;
/// * [`Power::Starting`]: [`POWER_STARTING`], and nothing is sent — the arena is
///   already doing it, whether the mirror knows that from a frame or from the
///   hint this function stamped;
/// * [`Power::Off`]: the intent is forwarded as before, the hint is stamped, and
///   the pilot reads the sentence they always read.
///
/// The hint expires one second past a whole `Quiet` period, which is the longest
/// a frame can be away; a lost `STARTUP` therefore tells the pilot to type it
/// again, which by then is the true advice. See
/// [`crate::arena_bridge::ArenaRegistry::starting_hints`] for the four ways it
/// ends.
fn startup(ctx: &mut Ctx<'_>, unit: ObjId) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // 16b cut D.2 (`B-03`): before the power, so a pilot in a wreck is never told
    // to wait for a startup that is not coming.
    require_alive(ctx.w, actor)?;
    match power_of(ctx.w, actor) {
        Power::Online => {
            ctx.out
                .sys_to_actor(actor, NOTICE_CATEGORY, "Power is already online.");
            return Ok(());
        }
        Power::Starting => {
            ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, POWER_STARTING);
            return Ok(());
        }
        Power::Off => {}
    }
    forward_intent(ctx, actor, unit, IntentKind::Startup)?;
    // The optimism is stamped only once the order is away: a refused forward is
    // not a startup in progress.
    let stands_s =
        crate::match_::lobby::configs(ctx.w).map_or(HINT_FALLBACK_S, |(arena_cfg, _, _)| {
            // Whole seconds, rounded up, plus one: the hint must outlast the
            // longest a frame can be away.
            let quiet_ms = i64::from(arena_cfg.tempo.quiet_ms).max(0);
            quiet_ms.saturating_add(999) / 1_000 + 1
        });
    let until_s = ctx.w.now().saturating_add(stands_s);
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.hint_starting(actor, until_s);
    }
    ctx.out.sys_to_actor(
        actor,
        NOTICE_CATEGORY,
        "Startup sequence begun; power comes online shortly.",
    );
    Ok(())
}

/// **How long the `Starting` hint stands** when the configuration cannot be read
/// (16b cut D.1), seconds: one whole `Quiet` period of the shipped default, plus
/// one.
const HINT_FALLBACK_S: i64 = 6;

/// `HEADING <deg>` — 0 to 359, clockwise from north (doc 16 §3.7).
fn heading(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let deg: u32 = rest
        .trim()
        .parse()
        .ok()
        .filter(|deg| *deg < 360)
        .ok_or_else(|| CockpitError::Usage("HEADING <0-359>, clockwise from north.".to_owned()))?;
    // 16b cut D.2 (`B-03`), in the arena's own order: the wreck, then the motion
    // limit, then the power.
    require_alive(ctx.w, actor)?;
    require_mobile(ctx.w, actor)?;
    require_power(ctx.w, actor)?;
    forward_intent(
        ctx,
        actor,
        unit,
        IntentKind::Heading {
            mdeg: deg.saturating_mul(1_000),
        },
    )?;
    let line = format!("Steering to {deg}°.");
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// `SPEED <kph>` — the unit's reverse ceiling to its own maximum (doc 16 §3.7).
/// The wire is centimetres a second (doc 16 §3.2's integers), so the conversion
/// is here and the pilot never types one.
///
/// **16b cut D.3 (`T-60`) made the refusal true.** It read `That unit makes 54
/// kph; SPEED 0 to 54.` — a range the verb did not enforce at either end: it
/// compared the **magnitude**, so `SPEED -54` was accepted on a tank whose
/// reverse ceiling is 40 per cent of its forward one, and the arena silently
/// clamped it to `-21` at apply. The two ceilings are different numbers and the
/// sentence now names both, read the way
/// [`crate::arena::intents`] reads them at apply:
/// [`crate::arena::state::TrackedRules::reverse_ceiling_cms`].
fn speed(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let kph: i32 =
        rest.trim().parse().ok().ok_or_else(|| {
            CockpitError::Usage("SPEED <kph>; a negative speed reverses.".to_owned())
        })?;
    let (max_cms, reverse_cms) = speed_ceilings(ctx.w, unit);
    let max_kph = cms_to_kph(i32::try_from(max_cms).unwrap_or(i32::MAX));
    let reverse_kph = cms_to_kph(i32::try_from(reverse_cms).unwrap_or(i32::MAX));
    if kph > max_kph || kph < -reverse_kph {
        return Err(CockpitError::Usage(format!(
            "That unit makes {max_kph} kph ahead and {reverse_kph} astern; SPEED -{reverse_kph} \
             to {max_kph}."
        )));
    }
    // 16b cut D.2 (`B-03`): as `HEADING`.
    require_alive(ctx.w, actor)?;
    require_mobile(ctx.w, actor)?;
    require_power(ctx.w, actor)?;
    forward_intent(
        ctx,
        actor,
        unit,
        IntentKind::Speed {
            cms: kph_to_cms(kph),
        },
    )?;
    let line = format!("Making {kph} kph.");
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// `LOCK <contact>` — hold one of the platform's contacts (doc 16 §3.7).
///
/// One refusal for every id the platform does not hold, whether it is malformed,
/// somebody else's or expired (doc 16 §3.6). **16b cut S.2 gave it something to
/// resolve**: the id is looked up in the platform's own contact list, which is the
/// world's mirror of `crate::arena::state::ShadowContacts` — the store
/// `ShadowContacts::resolve` answers from, keyed by the same
/// `(viewer platform, id)` pair and refusing with the same one sentence. A pilot
/// naming a malformed id, another platform's id or an expired one takes the same
/// path, byte for byte, because all three simply fail to be in the list.
///
/// **16b cut W.1 gave the order its effect**: the arena's phase 4 records the lock
/// on the platform, journals one `locked` line, and puts the contact's id on the
/// frame — so the `Locking {id}.` this says is now a statement about the arena and
/// not only about the world's mirror. A contact the platform then *loses* clears the
/// lock and the pilot is told so once (`NoticeKind::LockLost`).
///
/// **16b cut F.2 took the percentage off this verb.** Cut W.2 printed it here off
/// the frame the world already held, which meant a first `LOCK` was silent and a
/// repeat quoted the *previous* pulse's chance; the arena now says it itself when
/// the lock resolves ([`crate::arena::perception::FrameNotice::Locked`]), at the
/// timestamp it resolves at. There is one owner of the sentence and no second
/// lane (16b §0.2), so this says `Locking {id}.` and nothing else.
fn lock(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // 16b cut D.2 (`B-03`): a wreck takes no order at all, and the arena's own
    // refusal would be a pulse away. It is the FIRST thing this verb does, so a
    // pilot in a wreck is never answered about which contacts are out there.
    require_alive(ctx.w, actor)?;
    // **16b cut D.3 (`T-60`): a bare `LOCK` is usage and not a refusal.** An
    // empty argument is not a contact id that failed to resolve — it is a pilot
    // who has not said which one — and `No such contact.` over it read as a
    // statement about the scope. The one byte-identical refusal is still the
    // answer to every *non-empty* word, which is what keeps `LOCK BA` against an
    // unheld, a still-unknown and a re-acquired-unknown `BA` indistinguishable
    // from `LOCK ZZ`.
    if rest.trim().is_empty() {
        return Err(CockpitError::Usage(
            "LOCK <contact>; CONTACTS lists them.".to_owned(),
        ));
    }
    let held = last_frame(ctx.w, actor)
        .map(|held| frame_contacts(&held.frame))
        .unwrap_or_default();
    // **16b cut Q.1**: the word is a designator, not sixteen digits, and the
    // resolution is against the platform's own rows — so an id that names
    // nothing, an id that names somebody else's contact and an id that has
    // expired all take one path, byte for byte.
    let contact = resolve_designator(&held, rest).ok_or(CockpitError::NoSuchContact)?;
    // 16b cut D.2 (`B-18`): the echo says `(FRIENDLY)` where the row is one, off
    // the row's own `friendly` — which is `false` for every unidentified return,
    // so a blip is never marked. Nothing is refused.
    let named = held
        .iter()
        .find(|row| row.id == contact)
        .map(|row| {
            format!(
                "{}{}",
                row.designator,
                if row.friendly { FRIENDLY_MARK } else { "" }
            )
        })
        .unwrap_or_default();
    forward_intent(ctx, actor, unit, IntentKind::Lock { contact })?;
    let line = format!("Locking {named}.");
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **`RANGE [<designator>]`** — how far away one contact plotted (16b cut Q.1;
/// doc 16 §3.6, the owner's ask of 2026-09-19).
///
/// The named contact, else the platform's lock, else [`NO_LOCK_OR_NAME`]. It
/// reads the **last frame's plot** and nothing else — the same figure
/// `CONTACTS` and `WEAPONSTATUS` are reading — so three surfaces cannot give
/// three distances.
///
/// A locked contact earns [`LOCK_RANGE_STEP_M`] and the rest of the scope
/// [`CONTACT_RANGE_STEP_M`]: a lock is a fire-control solution and the platform
/// is tracking that one return continuously.
fn range(ctx: &mut Ctx<'_>, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let (frame, row) = named_row(held.as_ref(), rest)?;
    let line = range_line(frame, &row, pass_ms(ctx.w));
    say_and_frame(ctx, held.as_ref(), vec![line]);
    Ok(())
}

/// **The sentence `RANGE` says** (16b cut Q.1), split out from the world read the
/// way [`stale_clause`] is — so the step a lock earns and the staleness clause are
/// one pure function, and the branch between the two steps can be pinned against a
/// plot held still rather than against a live approach.
///
/// A locked contact earns [`lock_range_step_m`] of its own plot's age and the rest
/// of the scope [`CONTACT_RANGE_STEP_M`]: a lock is a fire-control solution and
/// the platform is tracking that one return continuously — but no finer than the
/// plot in front of the pilot can carry (16b cut D.4, `T-20`). The age is the row's
/// own, which is the `1 s` `CONTACTS` already prints.
#[must_use]
pub fn range_line(frame: &Frame, row: &ContactRow, pass_ms: i64) -> String {
    let locked = frame_lock(frame) == Some(row.id);
    let step = if locked {
        lock_range_step_m(row.age_ms)
    } else {
        CONTACT_RANGE_STEP_M
    };
    let metres = frame
        .contacts
        .iter()
        .find(|contact| contact.id == row.id)
        .map_or(row.range_m, |contact| {
            range_m_rounded(contact.range_cm, step)
        });
    format!(
        "Range to {}: {metres} m{}.",
        row.designator,
        stale_clause(row.age_ms, pass_ms)
    )
}

/// **`BEARING [<designator>]`** — which way one contact plotted (16b cut Q.1),
/// absolute and off the platform's own nose. The same rules [`range`] follows.
fn bearing(ctx: &mut Ctx<'_>, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let (_, row) = named_row(held.as_ref(), rest)?;
    let line = bearing_line(&row, pass_ms(ctx.w));
    say_and_frame(ctx, held.as_ref(), vec![line]);
    Ok(())
}

/// **The sentence `BEARING` says** (16b cut Q.1), for the reason [`range_line`] is
/// its own function. A bearing has one step, so this is the designator, the two
/// degrees and the staleness clause.
#[must_use]
pub fn bearing_line(row: &ContactRow, pass_ms: i64) -> String {
    format!(
        "Bearing to {}: {:03}° (rel {:03}°){}.",
        row.designator,
        row.bearing_deg,
        row.relative_deg,
        stale_clause(row.age_ms, pass_ms)
    )
}

/// **`SCAN [<designator>]`** — what is left of one contact, section by section
/// (16b cut Q.2; doc 16 §3.6, the owner's ask of 2026-09-19: *see the enemy
/// status, not exact numbers but green/yellow/red*).
///
/// The same argument rules [`range`] follows: the named contact, else the
/// platform's lock, else [`NO_LOCK_OR_NAME`]. What it prints is the reading the
/// arena put on the contact ([`crate::arena::perception::FrameContact::condition`])
/// and never a second computation, so the words a pilot reads and the bands a
/// panel draws are one answer.
fn scan(ctx: &mut Ctx<'_>, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let (frame, row) = named_row(held.as_ref(), rest)?;
    let lines = scan_lines(frame, &row);
    say_and_frame(ctx, held.as_ref(), lines);
    Ok(())
}

/// **The two-space indent a wrapped `SCAN` run carries** — `STATUS`'s damage
/// block's own, so the two blocks line up under one another on a narrow terminal.
const SCAN_INDENT: &str = "  ";

/// **The lines `SCAN` says about one contact** (16b cut Q.2), split out from the
/// world read the way [`range_line`] is, so every sentence can be pinned against a
/// frame built by hand.
///
/// Four answers, and each is a different thing the pilot does not know:
///
/// * **unidentified** — `X2 is not identified yet; there is nothing to read.`
///   Section ids are a chassis vocabulary, so a blip has no sections to read
///   bands into; it says so rather than refusing, because the pilot does hold the
///   contact and telling them otherwise would be a lie about their own scope.
/// * **classified, no reading** — `No reading on BA yet: close the range for a
///   look.` The close look is the price, and this is the only line that says what
///   the price is.
/// * **a reading** — the header with the **look's** own age, then one
///   `section band` pair per section, comma-separated and wrapped at
///   [`DAMAGE_COLUMNS`].
/// * **unheld** — never reaches this function: [`named_row`] answers the one
///   byte-identical [`NO_SUCH_CONTACT`] first.
///
/// **Never a number other than the age.** The bands are what a sight is worth
/// (`crate::arena::condition`), and there is no exact figure on this path for a
/// later surface to print.
#[must_use]
pub fn scan_lines(frame: &Frame, row: &ContactRow) -> Vec<String> {
    let named = row.designator;
    if row.class == "unknown" {
        return vec![format!(
            "{named} is not identified yet; there is nothing to read."
        )];
    }
    let reading = frame
        .contacts
        .iter()
        .find(|held| held.id == row.id)
        .and_then(|held| held.condition);
    let Some(reading) = reading else {
        return vec![format!(
            "No reading on {named} yet: close the range for a look."
        )];
    };
    // The vocabulary the bands are in the order of. A reading whose table is not
    // on the frame cannot be produced by the arena — the index is handed out by
    // the walk that builds the list — and is answered with no sections rather
    // than with a guess.
    let names: &[SmolStr] = frame
        .scan_tables
        .get(usize::from(reading.table))
        .map_or(&[], Vec::as_slice);
    let mut lines = vec![format!(
        "Scan of {named}, read {} s ago:",
        reading.age_ms / 1_000
    )];
    let mut run = String::new();
    for (index, section) in names.iter().enumerate() {
        let Some(band) = reading.bands.band(index) else {
            continue;
        };
        let cell = format!("{section} {}", band.as_str());
        // The comma stays on the line it follows, so a wrapped run reads as one
        // sentence rather than as two lists.
        if !run.is_empty() && run.len() + 2 + cell.len() > DAMAGE_COLUMNS {
            run.push(',');
            lines.push(run.clone());
            run.clear();
        }
        if run.is_empty() {
            run.push_str(SCAN_INDENT);
        } else {
            run.push_str(", ");
        }
        run.push_str(&cell);
    }
    if !run.is_empty() {
        lines.push(run);
    }
    lines
}

/// **The contact a read verb is about** (16b cut Q.1): the one the pilot named,
/// else the platform's lock.
///
/// # Errors
///
/// [`CockpitError::NoSuchContact`] for a word that resolves to nothing — the one
/// byte-identical refusal — and [`CockpitError::NoLockOrName`] when nothing was
/// named and there is no lock to fall back on.
fn named_row<'a>(
    held: Option<&'a CockpitFrame>,
    rest: &str,
) -> Result<(&'a Frame, ContactRow), CockpitError> {
    let named = rest.trim();
    // A platform the arena has not spoken to yet holds nothing, and the two
    // refusals have to stay in the right order even then: a **name** takes the
    // one byte-identical `No such contact.` whether the scope is empty or
    // missing, because a pilot who could tell those two apart could tell a name
    // that exists from one that does not.
    let frame = match held {
        Some(held) => &held.frame,
        None if named.is_empty() => return Err(CockpitError::NoLockOrName),
        None => return Err(CockpitError::NoSuchContact),
    };
    let rows = frame_contacts(frame);
    let id = if named.is_empty() {
        frame_lock(frame).ok_or(CockpitError::NoLockOrName)?
    } else {
        resolve_designator(&rows, named).ok_or(CockpitError::NoSuchContact)?
    };
    rows.into_iter()
        .find(|row| row.id == id)
        .ok_or(CockpitError::NoSuchContact)
        .map(|row| (frame, row))
}

/// **How old a plot has to be before a read verb says so** (16b cut Q.1): the
/// sensor pass's own period, read the way [`lobby::configs`] reads it.
///
/// A plot older than one pass is a plot the platform did not refresh last time it
/// looked, which is exactly when a pilot needs to know they are shooting at a
/// memory. The sentence is [`stale_clause`]'s; this is the only world read the two
/// read verbs make.
fn pass_ms(w: &World) -> i64 {
    lobby::configs(w).map_or(1_000_i64, |(_, _, sensors)| {
        i64::from(sensors.pass_ms.max(1))
    })
}

/// **The clause itself** (16b cut Q.1), split out from the configuration read so
/// the sentence can be pinned without a world: `, plotted 4 s ago` past the
/// threshold, and nothing at or under it.
#[must_use]
pub fn stale_clause(age_ms: i64, pass_ms: i64) -> String {
    if age_ms > pass_ms.max(1) {
        format!(", plotted {} s ago", age_ms / 1_000)
    } else {
        String::new()
    }
}

/// `FIRE [<weapon>]` — shoot the lock (doc 16 §3.7).
///
/// **The pre-checks are doc 16 §3.7's refusal column, in its order**: the weapon has
/// to exist, then there has to be a lock, then the weapon has to be out of its cycle,
/// then it has to have a round, and *then* the tank has to be awake. Power last is
/// the doc's order and the useful one — a cold tank with no lock is told to get a
/// lock, which is the longer job.
///
/// Every one of them is a read of the **last frame**, so the pilot is told at once
/// instead of a pulse later; the arena refuses in the same order against its own
/// state ([`crate::arena::state::Arena::resolve_fire`]) when the mirror was stale, so
/// a frame one pulse behind changes which half answers and never what it says.
fn fire(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // 16b cut D.2 (`B-03`): a wreck fires nothing, and it is asked first — a
    // destroyed tank's weapons are gone, so every later check would answer about
    // the rack instead of about the tank.
    require_alive(ctx.w, actor)?;
    let held = last_frame(ctx.w, actor);
    let weapon = choose_weapon(&carried(ctx.w, unit, held.as_ref()), rest)?;
    // **16b cut D.2** (`B-16`): every one of these is `FIRE <weapon> refused. …`,
    // which is the sentence the arena's own refusal says a pulse later. The
    // `NoSuchWeapon` above is the one that is not: `choose_weapon` failed to find
    // a weapon, so there is none to name.
    let refused = |said: &CockpitError| fire_refused(&weapon.id, said);
    if held
        .as_ref()
        .and_then(|held| frame_lock(&held.frame))
        .is_none()
    {
        return Err(refused(&CockpitError::NoLock));
    }
    if !weapon.ready {
        return Err(refused(&CockpitError::Cycling {
            secs: weapon.ready_in_ms.saturating_add(999) / 1_000,
        }));
    }
    if weapon.rounds <= 0 {
        return Err(refused(&CockpitError::Empty));
    }
    // **16b cut X.1** (doc 16 §9 item 1), in the arena's own order: the platform's
    // heat after the weapon's own two refusals and before the power. A platform
    // whose content declares no heat carries no `heat` on its frame and is never
    // refused here.
    //
    // **16b cut D.2** (`B-16`): and neither is a weapon that makes no heat, on
    // the platform's own row, which is the same clause `Arena::resolve_fire` and
    // `reflex::would_overheat` apply.
    if let Some(heat) = held.as_ref().and_then(|held| held.frame.heat)
        && heat.overheated()
        && weapon.heat_per_shot > 0
    {
        return Err(refused(&CockpitError::Overheated {
            heat: heat.heat,
            max: heat.max,
        }));
    }
    require_power(ctx.w, actor).map_err(|said| refused(&said))?;
    forward_intent(
        ctx,
        actor,
        unit,
        IntentKind::Fire {
            weapon: weapon.id.clone(),
        },
    )?;
    let line = format!("Firing {}.", weapon.id);
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// `CONTACTS` — what the platform holds (doc 16 §3.6 "What a pilot sees").
fn contacts(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let rows = held
        .as_ref()
        .map(|held| frame_contacts(&held.frame))
        .unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    if rows.is_empty() {
        lines.push("No contacts.".to_owned());
    }
    for row in &rows {
        lines.push(row.render());
    }
    say_and_frame(ctx, held.as_ref(), lines);
    Ok(())
}

/// `WEAPONSTATUS` — ammunition and readiness, per weapon (doc 16 §5).
///
/// **16b cut W.1 made the numbers live**: they come off the arena's own frame, which
/// is the single writer of the cycle and the rounds while a match runs, and carry the
/// band of the platform's lock where there is one.
fn weapon_status(ctx: &mut Ctx<'_>, unit: ObjId) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let weapons = carried(ctx.w, unit, held.as_ref());
    let mut lines: Vec<String> = Vec::new();
    if weapons.is_empty() {
        lines.push("This unit carries no weapon.".to_owned());
    }
    if let Some(line) = held.as_ref().and_then(|held| lock_line(&held.frame)) {
        lines.push(line);
    }
    for weapon in &weapons {
        lines.push(weapon.render(&weapons));
    }
    say_and_frame(ctx, held.as_ref(), lines);
    Ok(())
}

/// `STATUS` — the status block of doc 16 §5, as the last frame carries it.
fn status(ctx: &mut Ctx<'_>, unit: ObjId) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let lines = match held.as_ref() {
        Some(held) => {
            let frame = &held.frame;
            let mut lines = vec![
                // **16b cut Q.1**, first: the name everybody who has told this
                // tank apart is calling it by. A pilot asked "who is RA?" over
                // the radio needs one place to look.
                format!("Callsign {}.", frame.callsign),
                format!(
                    "{}{}: power {}, {} kph, heading {}°{}{}",
                    display_name(ctx.w, unit),
                    // **16b cut D.3 (`T-60`)**: the chassis, when it is not the
                    // one the pod authors.
                    fitted_bracket(ctx.w, unit, frame),
                    frame.power.as_str(),
                    cms_to_kph(frame.speed_cms),
                    frame.pose.heading_mdeg / 1_000,
                    if frame.blocked { ", HELD" } else { "" },
                    // 16b cut D.2 (`B-03`): beside `HELD`, and the one mark a
                    // pilot who has stopped being told anything needs.
                    if frame.wreck { ", DESTROYED" } else { "" }
                ),
                // **16b cut D.1**: the pilot's own alert state, off their own
                // scope, where the arena's one global tempo word used to be
                // ([`own_scope`]).
                format!(
                    "  at {} m east, {} m north; {}",
                    frame.pose.x_cm / 100,
                    frame.pose.y_cm / 100,
                    match own_scope(frame) {
                        "hostile" => "scope holds a hostile",
                        "unidentified" => "scope holds an unidentified contact",
                        _ => "scope clear",
                    }
                ),
            ];
            // **16b cut X.1**: a line of its own, and only for a platform that
            // carries heat.
            if let Some(heat) = frame.heat {
                lines.push(format!(
                    "  heat {} of {}{}",
                    heat.heat,
                    heat.max,
                    if heat.overheated() {
                        ", OVERHEATED"
                    } else {
                        ""
                    }
                ));
            }
            // **16b cut D.4** (`B-22`): a line of its own, after the heat and only
            // for a platform whose own last pass ran under interference. The three
            // integers are the pilot's own instrument — the budget the pass really
            // used against what it has when nothing is jamming, and what is left
            // of the reach — and nothing here says whose the emitter is or where
            // it stands ([`crate::arena::perception::Receiver`]).
            if let Some(receiver) = frame.receiver {
                lines.push(receiver_line(receiver));
            }
            // **16b cut F.2**: doc 16 §5's *armor and internals per section as
            // current over max*, one line per section and wrapped to eighty
            // columns by construction — the section, its internals, then each
            // facet's armour. It is the recorded open point K7's whole surface,
            // and a pilot's own tank is outside the observer rule entirely.
            lines.extend(section_lines(&frame.sections));
            lines
        }
        None => vec![format!(
            "{}: power off. The panel has told you nothing yet.",
            display_name(ctx.w, unit)
        )],
    };
    say_and_frame(ctx, held.as_ref(), lines);
    Ok(())
}

/// **The chassis bracket `STATUS`'s second line carries** (16b cut D.3, `T-60`),
/// or an empty string.
///
/// The smoke transcript's own complaint: a pilot who typed `SIM UNIT biped_mk1`
/// read `Red Tank 1: power online, …` all match, because the object's name is
/// pack content (`Red Tank 1` is the pod's tank whatever is fitted to it) and
/// nothing on the panel said otherwise. The name stays — renaming a persistent
/// object per match is a much worse answer — and the fitted type goes beside it
/// in the bracket the board already uses for exactly this, `[biped_mk1]`, spelled
/// by the board's own [`lobby::type_word`].
///
/// **Only when it differs from the authored type**, so a stock tank's line is
/// byte for byte the one cut D.1 wrote. The pod is read off the frame's own
/// callsign — the team and slot the lobby fielded this tank from — and then the
/// pod is asked for **its own tank by name**, because
/// [`crate::arena::state::Arena`]'s `resolve_callsigns` gives *every* unit a
/// `Designator::Slot`: the harness's three hundred and a staff `@arena/start`'s
/// two included. The designator alone therefore says nothing about where a tank
/// came from, and without the second question a staff-fielded biped would be
/// bracketed against the authored type of a centre pod it was never in. A unit
/// the centre does not field from that pod brackets nothing.
///
/// It is the pilot's own tank, so the observer rule is not in play: doc 16 §5
/// puts a pilot's own platform outside it entirely.
fn fitted_bracket(w: &World, unit: ObjId, frame: &Frame) -> String {
    let Designator::Slot { team, slot } = frame.callsign else {
        return String::new();
    };
    let Ok(index) = u8::try_from(slot) else {
        return String::new();
    };
    // The centre's own pod, and no other unit's: see the head.
    if centre::tank_for_slot(w, team, index) != Some(unit) {
        return String::new();
    }
    let Some(authored) = lobby::authored_unit_type(w, team, index) else {
        return String::new();
    };
    // The registry's `DefinitionId` and the pack-local entry id are the same
    // string (see `crate::definitions`' module head), so the tactics document's
    // own schema is directly comparable with the arena pack's authored type
    // without re-resolving either through the catalog.
    let Some(fitted) = mush_tactics::module::unit_of(w, unit)
        .map(|held| held.schema().key().id.as_str().to_owned())
    else {
        return String::new();
    };
    if fitted == authored.as_str() {
        return String::new();
    }
    format!(" [{}]", lobby::type_word(w, &fitted))
}

/// How wide a `STATUS` damage line may run before it is wrapped — inside the
/// eighty columns a line client is entitled to assume, with room for the
/// renderer's own margin.
const DAMAGE_COLUMNS: usize = 76;

/// **The damage block `STATUS` prints** (16b cut F.2; doc 16 §5: *armor and
/// internals per section as current over max*).
///
/// Two lines per section — the section with its condition and its internals, then
/// its facets' armour, wrapped at [`DAMAGE_COLUMNS`] — and one for a section that
/// declares no facets. Empty for a platform the arena has told the world nothing
/// about, so a fixture with no structure prints the two lines it always did.
#[must_use]
pub fn section_lines(sections: &[crate::arena::step::FrameSection]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for section in sections {
        let inside = match section.internals {
            Some((current, max)) => format!("  internals {current}/{max}"),
            None => String::new(),
        };
        lines.push(format!("  {}  {}{inside}", section.id, section.condition));
        let mut run = String::new();
        for facet in &section.facets {
            let cell = format!("{} {}/{}", facet.id, facet.armor, facet.armor_max);
            if !run.is_empty() && run.len() + cell.len() + 2 > DAMAGE_COLUMNS {
                lines.push(run.clone());
                run.clear();
            }
            if run.is_empty() {
                run.push_str("    ");
            } else {
                run.push_str("  ");
            }
            run.push_str(&cell);
        }
        if !run.is_empty() {
            lines.push(run);
        }
    }
    lines
}

/// `VIEW` — the arena window, as the grid frame (doc 16 §3.6, §5).
///
/// **16b cut Q.3**: the panel goes first and the one line that may follow it
/// comes after, because the line is *about* the map the pilot has just been
/// shown ([`beyond_window_line`]).
fn view(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // The map is the frame's; a line client's renderer draws it. Nothing is said
    // twice.
    let Some(held) = last_frame(ctx.w, actor) else {
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            "The window is dark: the panel has told you nothing yet.",
        );
        return Ok(());
    };
    let payload = view_frame(ctx, held.match_id, &held.frame);
    let beyond = beyond_window_line(&held.frame, &payload);
    ctx.out.push(
        mush_core::Audience::Actor(actor),
        OutputEvent::Panel {
            feed: SmolStr::new_static(FRAME_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: payload,
        },
    );
    if let Some(line) = beyond {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    Ok(())
}

/// Say what a read verb answers and push the frame it read (doc 16 §5: one feed,
/// and the line renderer prints the frame).
///
/// **16b cut Q.3** took the `with_view` flag off: `VIEW` is the one verb that
/// pushes the grid, and it now owns its own order — the panel first, then the
/// line it owes about what the panel could not fit.
fn say_and_frame(ctx: &mut Ctx<'_>, held: Option<&CockpitFrame>, lines: Vec<String>) {
    let actor = ctx.actor;
    for line in lines {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    let Some(held) = held else {
        return;
    };
    let payload = status_frame(held.match_id, &held.frame);
    ctx.out.push(
        mush_core::Audience::Actor(actor),
        OutputEvent::Panel {
            feed: SmolStr::new_static(FRAME_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: payload,
        },
    );
}

// ---------------------------------------------------------------------------
// The frame (doc 16 §5)
// ---------------------------------------------------------------------------

/// The frame's schema id, which every consumer keys its decode on.
pub const FRAME_SCHEMA: &str = "arena.cockpit";

/// Its version. Cuts S and W add fields; a field added is not a version.
pub const FRAME_VERSION: u32 = 1;

/// **The terrain frame's schema id** (16b cut Q.3), carried on
/// [`crate::arena_bridge::MAP_FEED`].
pub const MAP_SCHEMA: &str = "arena.map";

/// Its version. The map is terrain and an origin; a field added is not a
/// version, exactly as [`FRAME_VERSION`]'s comment says of the cockpit frame.
pub const MAP_VERSION: u32 = 1;

/// **The terrain frame's shape** (16b cut Q.3).
///
/// Deliberately **not** `grid`: `mush_ansi::panel` renders a `grid` keyframe as
/// twenty rows of text, and the whole arena is 256 by 256. A line client that
/// somehow received one must print nothing, and a shape it does not know is how
/// it does that.
pub const MAP_SHAPE: &str = "terrain";

/// **The `schema.retain` marker that makes a keyframe replace its own
/// predecessors in a session's resume ring** (16b cut Q.3; the rule is
/// implemented in `mush_session::SessionManager`, which cannot see this crate —
/// the string is the contract).
///
/// Both arena feeds carry it, and so does [`view_frame`]'s replacement schema.
/// Two keyframes a second of `Engaged` play would otherwise evict everything
/// else from a 512-entry ring in about four minutes; every frame either feed
/// sends is the whole state, so the older ones are not history and nothing is
/// lost by dropping them.
pub const RETAIN_LATEST: &str = "latest";

/// **The cockpit keyframe** (doc 16 §5): where the tank is, how fast, which way,
/// whether it is awake, whether it is held, how often it is being told things,
/// what it holds and what it carries.
///
/// **16b cut S.2 filled `contacts` and added `notices`; 16b cut W.1 made `weapons`
/// live and filled `lock`.**
///
/// One contact is `{ id, class, bearing_deg, relative_deg, band, age_ms, age_s,
/// cell, friendly }` — no object number, no name and **no exact range**, which is
/// doc 16 §3.6's *range band* read strictly. `cell` is the **arena window's** own
/// cell, so it is comparable with `x_cm / cell_cm`; [`view_frame`]'s `units` carry
/// the same marker at a cell relative to *that* frame's `origin`, which is what the
/// grid renderer draws from. `notices` is the lines this frame owes
/// its pilot, already rendered ([`notice_line`]), so a web or TUI panel prints the
/// same sentence the line client was sent; a frame read back out of the world's
/// mirror carries none, because the mirror is stored with them cleared.
///
/// One weapon is `{ id, ready, ready_in_ms, rounds, rounds_max, range_band }`, and
/// `range_band` is `{ index, of, max_range_cm, dispersion_mrad }` for the band the
/// platform's lock falls in — `null` with no lock, which is also when `lock` is
/// `null`. It takes no world handle: the arena is the single writer of both cells
/// while a match runs, so the frame is the whole answer.
///
/// **16b cut W.2** added `lock` a companion: `lock_p_permille` and `lock_percent`,
/// doc 16 §3.5 step 5's chance against the held lock, per mille and as the whole per
/// cent the line client prints. Both are `null` with no lock and when the arena has
/// not resolved one, which is what keeps a panel from drawing a 0 % that means
/// *unknown*. **16b cut F.2** made them the chance of *the weapon a bare `FIRE`
/// would fire* rather than the best of the platform's, and gave every weapon row
/// its own `p_permille` and `percent`.
///
/// **16b cut F.3** added `range_m` to each contact: the coarse range
/// [`CONTACTS`](contacts) prints, metres rounded to [`CONTACT_RANGE_STEP_M`]. It
/// also changed what every one of a contact's numbers is *of* — `bearing_deg`,
/// `range_m`, `band` and `cell` are read off the **plot** the arena recorded at
/// the sighting that last refreshed the contact, and `age_ms`/`age_s` are the age
/// of that sighting rather than the hold time since first detection. A panel is
/// therefore drawing what the platform knows, exactly as the line client is.
///
/// **16b cut Q.2** added `condition` to each contact — `{ table, bands, age_ms }`
/// or `null` — and `scan_tables` beside `contacts`. `bands` is the compact
/// spelling (`"ggyrd"`, one letter a section: green, yellow, red, destroyed) in
/// the order of `scan_tables[table]`, and `age_ms` is the age of the **look** and
/// not of the plot. The table index lives inside the object on purpose: a contact
/// with no reading can publish no section vocabulary by construction, and
/// `scan_tables` lists a chassis only for a contact that carries one. Each notice
/// also gained `section` — the section of the **target** a gunner's line is
/// about, `null` on every other kind — so the new `target_section_lost` notice
/// carries its section as a field and not only inside `text`.
///
/// **16b cut F.2** also added `sections` — doc 16 §5's status block, one entry per
/// section as `{ id, condition, internals, internals_max, facets[{ id, armor,
/// armor_max }] }` — and `heat` as `{ heat, max, overheated }` or `null`, which
/// cut X.1 put on `STATUS`'s text and not on the payload, so the two renderers had
/// drifted.
///
/// **One owner per sentence.** `notices` carries the same rendered line the bridge
/// sends the pilot as a `Sys` line, and both go to the same audience: a panel
/// renderer that prints this array **as well as** the scrollback will print every
/// notice twice. The array is the shape a panel should *draw* from — kinds, ids,
/// and the text as a fallback — and a renderer that owns the scrollback should not
/// print the text again. A frame read back out of the world's mirror carries no
/// notices at all, because the mirror is stored with them cleared.
///
/// **16b cut D.1** added two top-level keys: `sight_m`, the platform's own
/// nominal sensor reach in metres (`0` for a platform with no sensor), and
/// `scope`, the pilot's own alert state — `clear`, `unidentified` or `hostile`
/// ([`own_scope`]). Each contact's `band` is [`contact_band`]'s word: thirds of
/// the lesser of `sight_m` and the fired weapon's reach. `tempo` is unchanged and
/// stays the arena's global word.
#[must_use]
pub fn status_frame(match_id: MatchId, frame: &Frame) -> serde_json::Value {
    let rows = frame_contacts(frame);
    let contacts: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                // 16b cut Q.1: `id` stays the stable row key a panel matches on
                // across pulses and is never displayed; `designator` is what a
                // person reads and types, and `alias` is the number behind a
                // renamed row.
                "id": row.id.to_string(),
                "designator": row.designator.to_string(),
                "alias": row.alias.map(|n| Designator::Provisional(n).to_string()),
                "class": row.class.as_str(),
                "bearing_deg": row.bearing_deg,
                "relative_deg": row.relative_deg,
                "band": row.band.as_str(),
                // 16b cut F.3: the coarse range, the same figure `CONTACTS`
                // prints, so a panel and a line client read one number.
                "range_m": row.range_m,
                "age_ms": row.age_ms,
                "age_s": row.age_ms / 1_000,
                "cell": row.cell.map(|(x, y)| [x, y]),
                "friendly": row.friendly,
                // 16b cut Q.2: what the platform has read off this contact's
                // hull, or `null`. The table index is inside the object, so a
                // row with no reading carries no section vocabulary at all —
                // which is the observer rule on this surface, by construction.
                "condition": frame
                    .contacts
                    .iter()
                    .find(|held| held.id == row.id)
                    .and_then(|held| held.condition)
                    .map(|reading| serde_json::json!({
                        "table": reading.table,
                        "bands": reading.bands.letters(),
                        "age_ms": reading.age_ms,
                    })),
            })
        })
        .collect();
    // 16b cut Q.1: a notice that renders to nothing owes the pilot nothing, so it
    // is left out here for the reason `notice_lines` drops it — a panel drawing
    // this array would otherwise draw a blank row.
    let notices: Vec<serde_json::Value> = frame
        .notices
        .iter()
        .filter_map(|notice| {
            let text = notice_line(frame, notice, &rows);
            if text.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "kind": notice.kind().as_str(),
                "contact": notice.contact().map(|id| id.to_string()),
                "designator": notice.designator().map(|named| named.to_string()),
                // 16b cut Q.2: the section of the *target* a gunner's line is
                // about, so a panel that wants to flash it does not have to
                // regex it back out of `text`. `null` on every other notice,
                // the victim's own two included — what a round did inside this
                // tank is `sections` above, with its facet and its cost.
                "section": notice.section().map(SmolStr::as_str),
                // **16b cut D.2** (`B-07`): the sentence's own parts. Until this
                // cut the whole of a refusal and the whole of a block lived in
                // `text`, which a panel correctly never reads (one sentence, one
                // owner) — so the web cockpit printed a bare `refused` with no
                // order, no weapon and no reason. Every key here is `null` on a
                // notice it does not apply to.
                "cause": notice.cause().map(|cause| cause.as_str()),
                "refusal": notice.refusal().map(|refusal| refusal.as_str()),
                "order": notice.order().map(SmolStr::as_str),
                "weapon": notice.weapon().map(SmolStr::as_str),
                "detail": match notice.detail() {
                    RefusalDetail::None => serde_json::Value::Null,
                    RefusalDetail::Cycling { secs } => serde_json::json!({ "secs": secs }),
                    RefusalDetail::Overheated { heat, max } => {
                        serde_json::json!({ "heat": heat, "max": max })
                    }
                },
                // 16b cut D.2 (`B-18`): whether the contact this notice names is
                // on the platform's own side, off the frame's own row — so a
                // panel marks a friendly lock without a table of its own. Always
                // `false` for a notice that names no contact, and for every
                // unidentified one.
                "friendly": notice
                    .contact()
                    .is_some_and(|id| !friendly_mark(frame, id).is_empty()),
                // 16b cut D.2 (`B-15`): the round found a section that was
                // already destroyed.
                "spent": notice.spent(),
                "text": text,
            }))
        })
        .collect();
    let weapons: Vec<serde_json::Value> = frame_weapons(frame)
        .iter()
        .map(|weapon| {
            serde_json::json!({
                "id": weapon.id.as_str(),
                "ready": weapon.ready,
                "ready_in_ms": weapon.ready_in_ms,
                "rounds": weapon.rounds,
                "rounds_max": weapon.rounds_max,
                "range_band": weapon.range_band.map(|band| serde_json::json!({
                    "index": band.index,
                    "of": band.of,
                    "max_range_cm": band.max_range_cm,
                    "dispersion_mrad": band.dispersion_mrad,
                })),
                "p_permille": weapon.p_permille,
                "percent": weapon.p_permille.map(|permille| permille / 10),
                // 16b cut D.2 (`B-16`): what one shot of it costs the platform,
                // and `0` for a cold gun — which is what tells a panel why an
                // overheated tank may still fire this one.
                "heat_per_shot": weapon.heat_per_shot,
            })
        })
        .collect();
    // **16b cut F.2**: doc 16 §5's status block, the same numbers `STATUS` prints
    // and from the same frame — so the line client, the TUI and the web draw one
    // tank (§5: *the TUI and web render the same structure; parity is a golden
    // test in each renderer*).
    let sections: Vec<serde_json::Value> = frame
        .sections
        .iter()
        .map(|section| {
            serde_json::json!({
                "id": section.id.as_str(),
                "condition": section.condition.as_str(),
                "internals": section.internals.map(|(current, _)| current),
                "internals_max": section.internals.map(|(_, max)| max),
                "facets": section.facets.iter().map(|facet| serde_json::json!({
                    "id": facet.id.as_str(),
                    "armor": facet.armor,
                    "armor_max": facet.armor_max,
                })).collect::<Vec<serde_json::Value>>(),
            })
        })
        .collect();
    let chosen = fired_by_default(&frame.weapons).and_then(|weapon| weapon.p_permille);
    // 16b cut Q.1: the lock's own name and its finer range, resolved once.
    let lock_designator = lock_designator(frame);
    // **16b cut D.4** (`T-20`): the same step `RANGE` uses, off the same row's own
    // age — one function, so the verb, the frame and the two panels' lock lines
    // cannot give two distances.
    let lock_range_m = frame_lock(frame).and_then(|contact| {
        frame
            .contacts
            .iter()
            .find(|held| held.id == contact)
            .map(|held| range_m_rounded(held.range_cm, lock_range_step_m(held.age_ms)))
    });
    serde_json::json!({
        // 16b cut Q.3: `retain` tells the session ring that this keyframe
        // supersedes the ones before it on this feed (see [`RETAIN_LATEST`]).
        "schema": { "id": FRAME_SCHEMA, "version": FRAME_VERSION, "retain": RETAIN_LATEST },
        "match": match_id.0,
        "unit": frame.unit.0,
        // 16b cut Q.1: the platform's own public name, and the arena cell it is
        // standing in — what a panel's map draws the `@` at.
        "callsign": frame.callsign.to_string(),
        "cell": frame.cell.map(|(x, y)| [x, y]),
        "x_cm": frame.pose.x_cm,
        "y_cm": frame.pose.y_cm,
        "z_cm": frame.pose.z_cm,
        "heading_mdeg": frame.pose.heading_mdeg,
        "heading_deg": frame.pose.heading_mdeg / 1_000,
        "speed_cms": frame.speed_cms,
        "speed_kph": cms_to_kph(frame.speed_cms),
        "power": frame.power.as_str(),
        "blocked": frame.blocked,
        // **16b cut D.2** (`B-03`): the pilot's own two limits. A pilot's tank is
        // outside the observer rule entirely, and these are what a panel greys
        // the order controls on.
        "wreck": frame.wreck,
        "immobile": frame.immobile,
        "tempo": frame.tempo.as_str(),
        // **16b cut D.1**: the platform's own nominal sight, metres — what each
        // contact's `band` is read against ([`contact_band`]), so a panel can say
        // why a contact at 450 m reads `far`. A fact about the pilot's own
        // machine, outside the observer rule.
        "sight_m": frame.sight_cm / 100,
        // **16b cut D.1**: the pilot's own alert state ([`own_scope`]) —
        // `clear`, `unidentified` or `hostile`. `tempo` above stays, because a
        // panel's own timers read the cadence off it, but it is the arena's one
        // global word and not this platform's.
        "scope": own_scope(frame),
        "lock": frame_lock(frame).map(|id| id.to_string()),
        "lock_designator": lock_designator.map(|named| named.to_string()),
        "lock_range_m": lock_range_m,
        "lock_p_permille": chosen,
        "lock_percent": chosen.map(|permille| permille / 10),
        "contacts": contacts,
        // 16b cut Q.2: the section vocabularies the readings above are in the
        // order of, one entry per distinct chassis and only for contacts that
        // carry a reading. One name in Rust and on the wire.
        "scan_tables": frame.scan_tables,
        "notices": notices,
        "weapons": weapons,
        "sections": sections,
        "heat": frame.heat.map(|heat| serde_json::json!({
            "heat": heat.heat,
            "max": heat.max,
            "overheated": heat.overheated(),
        })),
        // **16b cut D.4** (`B-22`): what the platform's own receiver is running
        // under, or `null`. A fact about the pilot's own machine, with no source,
        // no bearing, no side and no object number on it
        // ([`crate::arena::perception::Receiver`]).
        "receiver": frame.receiver.map(|receiver| serde_json::json!({
            "budget": receiver.budget,
            "budget_max": receiver.budget_max,
            "reach_percent": receiver.reach_permille / 10,
        })),
    })
}

/// **One rectangle of terrain, run-length encoded** (16b cut Q.3), and the
/// classes it actually carries in first-seen order.
///
/// Row `0` of the answer is the **south** edge (`y0`), which is what the grid
/// shape means by `rows` and what [`map_frame`] and [`view_window`] both publish.
/// One glyph per terrain class, from the atlas's own table, so the arena's map
/// and the atlas's map read alike.
fn terrain_rows(
    terrain: &ArenaTerrain,
    origin: (u16, u16),
    size: (u16, u16),
) -> (Vec<serde_json::Value>, Vec<mush_atlas::TerrainClass>) {
    let (x0, y0) = origin;
    let (width, height) = size;
    let mut rows: Vec<serde_json::Value> = Vec::with_capacity(usize::from(height));
    let mut classes: Vec<mush_atlas::TerrainClass> = Vec::new();
    for y in y0..y0.saturating_add(height) {
        // Run-length, as the grid shape wants it: one pair per run of a glyph.
        let mut runs: Vec<serde_json::Value> = Vec::new();
        let mut run: Option<(char, u32)> = None;
        for x in x0..x0.saturating_add(width) {
            let class = terrain
                .get(x, y)
                .map_or(mush_atlas::TerrainClass::Void, |cell| cell.class);
            if !classes.contains(&class) {
                classes.push(class);
            }
            let glyph = class.glyph();
            match run {
                Some((held, n)) if held == glyph => run = Some((held, n.saturating_add(1))),
                Some((held, n)) => {
                    runs.push(serde_json::json!([held.to_string(), n]));
                    run = Some((glyph, 1));
                }
                None => run = Some((glyph, 1)),
            }
        }
        if let Some((held, n)) = run {
            runs.push(serde_json::json!([held.to_string(), n]));
        }
        rows.push(serde_json::Value::Array(runs));
    }
    (rows, classes)
}

/// **The whole arena's terrain, and nothing else** (16b cut Q.3; doc 16 §3.2,
/// §3.6).
///
/// The knowledge rule it rests on is the cut Q design note's, and doc 16 §3.2
/// and §3.6 are owed the sentence: *the proving ground is surveyed ground — its
/// terrain is known to every pilot at check-out, its units are not*. Terrain
/// names no unit, and `VIEW` has drawn ground behind hills since cut K.2.
///
/// There is no `units` key, no `you` key and nothing about any unit **by
/// construction**: the whole observer rule on this surface is that the map is a
/// fact about the ground. What moves goes on the cockpit frame, which is
/// per-pilot and projected; a panel client draws the markers of
/// [`status_frame`] over the rows of this one.
///
/// It is only ever the answer to a query
/// ([`crate::arena_bridge::MAP_FEED`]'s handler) and is never pushed: it is the
/// largest message the arena produces and a match's opening pulse is the least
/// reliable moment to send one, because a full per-connection output queue drops
/// an event silently and nothing retries. A client that holds a cockpit frame of
/// match M and no map of M asks, and asks again until it has one.
#[must_use]
pub fn map_frame(match_id: MatchId, surface: &str, terrain: &ArenaTerrain) -> serde_json::Value {
    let (rows, classes) = terrain_rows(terrain, (0, 0), (terrain.width(), terrain.height()));
    let legend: Vec<serde_json::Value> = classes
        .iter()
        .map(|class| serde_json::json!([class.glyph().to_string(), class.label()]))
        .collect();
    serde_json::json!({
        "schema": {
            "id": MAP_SCHEMA,
            "version": MAP_VERSION,
            "shape": MAP_SHAPE,
            "retain": RETAIN_LATEST,
        },
        "match": match_id.0,
        "surface": surface,
        "cell_cm": terrain.cell_cm(),
        "width": terrain.width(),
        "height": terrain.height(),
        // Always the arena's own corner today. It is in the payload because it
        // is what a windowed map would carry: doc 16 §11's 10 km path would
        // answer a `window` parameter with a slice of the same shape, and a
        // renderer written against this key keeps working when it does.
        "origin_cell": [0, 0],
        "rows": rows,
        "legend": legend,
    })
}

/// **The answer to an `arena.map` query from somebody who is not flying**
/// (16b cut Q.3): the schema, and the one fact that is theirs to have.
///
/// Nothing about any match: not its id, not its ground, not whether one is
/// running. A bystander, a staffer and a pilot who has typed `EXIT` read the
/// same two keys.
#[must_use]
pub fn map_unseated() -> serde_json::Value {
    serde_json::json!({
        "schema": {
            "id": MAP_SCHEMA,
            "version": MAP_VERSION,
            "shape": MAP_SHAPE,
            "retain": RETAIN_LATEST,
        },
        "seated": false,
    })
}

/// **The empty cockpit frame** (16b cut Q.3) — what a panel is sent when its
/// pilot stops being one, and what an `arena.cockpit` query answers anybody who
/// is not flying.
///
/// The bridge sends it once through the ordinary output path when a pilot is
/// unbound (`EXIT`, a released seat, the match ending, a void). It prints
/// nothing on a line client, because it declares no shape; a panel clears; and —
/// because it carries [`RETAIN_LATEST`] like every frame before it — it is the
/// one frame left in the session's resume ring, so a client that reconnects
/// after the match does not resurrect a dead cockpit.
#[must_use]
pub fn cockpit_cleared() -> serde_json::Value {
    serde_json::json!({
        "schema": { "id": FRAME_SCHEMA, "version": FRAME_VERSION, "retain": RETAIN_LATEST },
        "seated": false,
    })
}

/// **The cockpit keyframe with the window in it** — [`status_frame`] plus the
/// `grid` shape `mush_ansi::panel` renders as a text map and the web draws as a
/// map panel (doc 16 §5: *the ANSI line renderer prints the frame as text; the
/// TUI and web render the same structure*).
///
/// **16b cut Q.3** split the window itself out into [`view_window`], which is a
/// pure function of a frame and a terrain: it is what the canonical fixtures
/// call, and what makes the three renderers' compositions comparable. All this
/// adds is the two things a window cannot know without a world — which ground
/// the match is being fought over, and what the pilot's own tank is called.
#[must_use]
pub fn view_frame(ctx: &Ctx<'_>, match_id: MatchId, frame: &Frame) -> serde_json::Value {
    let payload = status_frame(match_id, frame);
    let Some(ground) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
    else {
        return payload;
    };
    let mut payload = view_window(payload, ground.id.as_str(), &ground.terrain, frame);
    // The pilot's own tank's name, which is theirs to have and which a pure
    // window has no world to ask for: [`view_window`] leaves the key out rather
    // than inventing a value the live feed does not send, and this is the one
    // place it is filled in. The whole payload is byte for byte what it was
    // before cut Q.3 apart from `schema.retain`.
    if let Some(you) = payload
        .get_mut("units")
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|units| units.first_mut())
        .filter(|unit| unit.get("you").and_then(serde_json::Value::as_bool) == Some(true))
        .and_then(serde_json::Value::as_object_mut)
    {
        you.insert(
            "name".to_owned(),
            serde_json::json!(display_name(ctx.w, frame.unit)),
        );
    }
    payload
}

/// **The arena window, as a pure function of the frame and the ground** (16b cut
/// Q.3).
///
/// `payload` is a [`status_frame`] this widens in place: the `schema` object is
/// **replaced** by one carrying `shape: "grid"` — and [`RETAIN_LATEST`] with it,
/// or a typed `VIEW` would re-create the ring flood one frame at a time — and
/// the window's own keys are added beside the cockpit's.
///
/// The window is [`VIEW_COLUMNS`] by [`VIEW_ROWS`] cells around the platform,
/// clamped to the arena: the most a line client prints, so the renderer only has
/// to clip it to a narrow terminal and never has to ask for more. A pose off the
/// window answers the payload untouched.
///
/// **The marker order is the rule every compositor follows**: the pilot's own
/// marker first, then the contacts in frame order, a later marker overwriting an
/// earlier one on the same cell (which is what `mush_ansi::panel::expand_rows`
/// does with `units`, and what the TUI and the web must do too).
///
/// The own marker carries **no `name`**: a window has no world, and the name the
/// live feed sends there is the tank's display name, which only [`view_frame`]
/// can ask for. An absent key is honest; a callsign in its place would teach
/// every renderer written against the fixtures a value the server never sends.
#[must_use]
pub fn view_window(
    payload: serde_json::Value,
    surface: &str,
    terrain: &ArenaTerrain,
    frame: &Frame,
) -> serde_json::Value {
    let mut payload = payload;
    let Some((cx, cy)) = terrain.cell_of(&frame.pose) else {
        return payload;
    };
    let width = VIEW_COLUMNS.min(terrain.width());
    let height = VIEW_ROWS.min(terrain.height());
    let x0 = cx
        .saturating_sub(width / 2)
        .min(terrain.width().saturating_sub(width));
    let y0 = cy
        .saturating_sub(height / 2)
        .min(terrain.height().saturating_sub(height));
    let (rows, classes) = terrain_rows(terrain, (x0, y0), (width, height));
    let mut legend: Vec<serde_json::Value> = classes
        .iter()
        .map(|class| serde_json::json!([class.glyph().to_string(), class.label()]))
        .collect();
    legend.push(serde_json::json!([YOU_GLYPH, "you"]));
    // The platform at the centre, then one marker per contact. Cells are
    // relative to the window's own origin, which is what the grid renderer
    // centres on.
    let you = [
        u32::from(cx.saturating_sub(x0)),
        u32::from(cy.saturating_sub(y0)),
    ];
    let mut units = vec![serde_json::json!({
        "id": format!("#{}", frame.unit.0),
        "cell": you,
        "glyph": YOU_GLYPH,
        "you": true,
    })];
    let (markers, glyphs) = contact_markers(frame, (x0, y0), (width, height));
    units.extend(markers);
    for entry in glyphs {
        if !legend.contains(&entry) {
            legend.push(entry);
        }
    }
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "schema".to_owned(),
            serde_json::json!({
                "id": FRAME_SCHEMA,
                "version": FRAME_VERSION,
                "shape": "grid",
                "retain": RETAIN_LATEST,
            }),
        );
        object.insert("title".to_owned(), serde_json::json!("Arena window"));
        object.insert("surface".to_owned(), serde_json::json!(surface));
        object.insert("width".to_owned(), serde_json::json!(width));
        object.insert("height".to_owned(), serde_json::json!(height));
        object.insert("cell_cm".to_owned(), serde_json::json!(terrain.cell_cm()));
        object.insert(
            "origin".to_owned(),
            serde_json::json!([
                i64::from(x0) * i64::from(terrain.cell_cm()),
                i64::from(y0) * i64::from(terrain.cell_cm())
            ]),
        );
        object.insert("rows".to_owned(), serde_json::Value::Array(rows));
        object.insert("legend".to_owned(), serde_json::Value::Array(legend));
        object.insert("units".to_owned(), serde_json::Value::Array(units));
        object.insert("you".to_owned(), serde_json::json!(you));
    }
    payload
}

/// **The one honest line `VIEW` owes a pilot whose scope is wider than their
/// terminal** (16b cut Q.3; doc 16 §3.6).
///
/// A sensor reaches sixty cells every way and the line client's window is
/// [`VIEW_COLUMNS`] by [`VIEW_ROWS`], because a terminal is eighty by
/// twenty-four. A contact whose plot falls outside that window — or off the
/// arena entirely, which is a plot with no cell — is on `CONTACTS` and not on
/// the map, and the pilot is told so rather than left to conclude the scope is
/// empty. `None` when every held contact is drawn.
///
/// `window` is the payload [`view_window`] just produced: its `units` are the
/// markers that were drawn, so the count is the drawing's own and cannot drift
/// from it. A payload with no window at all owes nothing.
#[must_use]
pub fn beyond_window_line(frame: &Frame, window: &serde_json::Value) -> Option<String> {
    let drawn = window
        .get("units")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter(|unit| unit.get("you").and_then(serde_json::Value::as_bool) != Some(true))
        .count();
    match frame.contacts.len().saturating_sub(drawn) {
        0 => None,
        1 => Some("1 contact plots beyond this window; CONTACTS lists it.".to_owned()),
        many => Some(format!(
            "{many} contacts plot beyond this window; CONTACTS lists them."
        )),
    }
}

/// **The contact markers a window carries, and the legend entries they earn**
/// (16b cut S.2), clipped to the window `origin` and `size` name and with cells
/// relative to the window's own corner.
///
/// Split out of [`view_frame`] because it is the one part of a window that is a
/// pure function of the frame — the terrain, the pilot's own marker and the
/// display name all need a world, and the observer rule on the map is entirely
/// about what goes in *here*.
///
/// **The observer rule on this surface.** A marker carries a cell, a glyph, the
/// class word as its `name`, and — since **16b cut Q.1** — the short designator
/// as its `id`: the grid shape's `id` is display text (the atlas's own markers
/// carry `#5` in it), so a panel that labels a marker labels it with the word the
/// pilot types and never with the sixteen digits. No object number, no unit name,
/// nothing the platform has not resolved.
///
/// The legend gains a glyph only when the window actually carries one: a legend
/// that always advertised `hostile` would tell a pilot with an empty scope what a
/// full one looks like, which is one more thing than they hold.
#[must_use]
pub fn contact_markers(
    frame: &Frame,
    origin: (u16, u16),
    size: (u16, u16),
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let (x0, y0) = origin;
    let (width, height) = size;
    let mut markers: Vec<serde_json::Value> = Vec::new();
    let mut legend: Vec<serde_json::Value> = Vec::new();
    for row in frame_contacts(frame) {
        let Some((mx, my)) = row.cell else {
            continue;
        };
        if mx < x0 || my < y0 {
            continue;
        }
        let (dx, dy) = (mx.saturating_sub(x0), my.saturating_sub(y0));
        if dx >= width || dy >= height {
            continue;
        }
        markers.push(serde_json::json!({
            "id": row.designator.to_string(),
            "name": row.class.as_str(),
            "cell": [u32::from(dx), u32::from(dy)],
            "glyph": row.glyph(),
            "you": false,
        }));
        let label = match row.glyph() {
            FRIEND_GLYPH => "friendly",
            HOSTILE_GLYPH => "hostile",
            _ => "contact",
        };
        let entry = serde_json::json!([row.glyph(), label]);
        if !legend.contains(&entry) {
            legend.push(entry);
        }
    }
    (markers, legend)
}

// ---------------------------------------------------------------------------
// The unit's own numbers
// ---------------------------------------------------------------------------

/// One weapon, as [`frame_weapons`] and [`weapons_of`] read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponRow {
    /// The component id (`cannon`, `machine_gun`).
    pub id: SmolStr,
    /// Whether its cycle has finished.
    pub ready: bool,
    /// **How long until it has**, milliseconds; `0` when it is ready (16b cut W.1).
    /// Zero from [`weapons_of`], whose world-side cells a running match does not
    /// move.
    pub ready_in_ms: i64,
    /// Rounds left.
    pub rounds: i64,
    /// Rounds a full load carries.
    pub rounds_max: i64,
    /// **The band the platform's lock falls in for this weapon** (16b cut W.1), or
    /// `None` with no lock, with the lock out of the weapon's reach, or from
    /// [`weapons_of`], which has no lock to read.
    pub range_band: Option<crate::arena::weapons::FrameBand>,
    /// **This weapon's own chance against the lock**, per mille (16b cut F.2; doc
    /// 16 §3.5 step 5 and §5's *per weapon*), or `None` with no lock and from
    /// [`weapons_of`], which has none to resolve against.
    pub p_permille: Option<u32>,
    /// **What one shot of it adds to the platform's heat** (16b cut D.2, `B-16`),
    /// and `0` for a weapon that makes none — which is every weapon of a stock
    /// tank, and also what [`weapons_of`] answers, because a unit no arena holds
    /// has no heat state to read.
    ///
    /// The `FIRE` pre-check skips the heat refusal for a row of nought, which is
    /// the clause [`crate::arena::state::Arena::resolve_fire`] applies a pulse
    /// later.
    pub heat_per_shot: u32,
}

impl WeaponRow {
    /// The line `WEAPONSTATUS` prints for this row (doc 16 §5's status block).
    ///
    /// `cannon  ready  40 of 40 round(s)  band 1 of 2  46% to hit` while there is
    /// something to shoot at, and `cannon  cycling (4 s)  39 of 40 round(s)` while
    /// the breech is still busy. The band and the percentage are only ever printed
    /// when the platform has a lock, because that is the only time either means
    /// anything — and the percentage is **this weapon's**, which is the whole of
    /// 16b cut F.2's item 4.
    ///
    /// **16b cut D.2** (`B-17`) opens the short name the pilot may type —
    /// `machine_gun (mg)` — where this row has one that no other row shares.
    /// That is why the whole list is the argument: a short name is a fact about
    /// the *platform's* weapons and not about one of them, and a row that
    /// advertised an ambiguous `m` would be advertising a refusal.
    #[must_use]
    pub fn render(&self, rows: &[WeaponRow]) -> String {
        let named = match weapon_short_of(rows, &self.id) {
            Some(short) => format!("{} ({short})", self.id),
            None => self.id.to_string(),
        };
        let ready = if self.ready {
            "ready".to_owned()
        } else {
            format!(
                "cycling ({} s)",
                self.ready_in_ms.saturating_add(999) / 1_000
            )
        };
        let band = match self.range_band {
            Some(band) => format!("  band {} of {}", band.index, band.of),
            None => String::new(),
        };
        let chance = match self.p_permille {
            Some(permille) => format!("  {}% to hit", permille / 10),
            None => String::new(),
        };
        format!(
            "{named}  {ready}  {} of {} round(s){band}{chance}",
            self.rounds, self.rounds_max
        )
    }
}

/// The cell a weapon component's ammunition lives in (doc 16 §3.3).
const AMMO_CELL: &str = "ammo";

/// The cell its cycle lives in.
const READY_AT_CELL: &str = "ready_at_ms";

/// **Every weapon a unit carries**, in component order (doc 16 §3.3's
/// components).
///
/// A weapon is a component with both an `ammo` and a `ready_at_ms` cell, which
/// is exactly what the unit decoder gives one (`definitions/unit.rs`). Read
/// structurally rather than by looking the weapon definition up, because the
/// question here is what *this* tank is carrying now.
#[must_use]
pub fn weapons_of(w: &World, unit: ObjId) -> Vec<WeaponRow> {
    let Some(cells) = mush_tactics::module::unit_of(w, unit) else {
        return Vec::new();
    };
    let (Ok(ammo), Ok(ready_at)) = (
        mush_tactics::CellId::new(AMMO_CELL),
        mush_tactics::CellId::new(READY_AT_CELL),
    ) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (component, _) in cells.components() {
        let Some(rounds) = cells.cell(&mush_tactics::CellAddr::Component {
            component: component.clone(),
            cell: ammo.clone(),
        }) else {
            continue;
        };
        let Some(cycle) = cells.cell(&mush_tactics::CellAddr::Component {
            component: component.clone(),
            cell: ready_at.clone(),
        }) else {
            continue;
        };
        rows.push(WeaponRow {
            id: SmolStr::new(component.as_str()),
            ready: cycle.current == 0,
            ready_in_ms: 0,
            rounds: rounds.current,
            rounds_max: rounds.max,
            range_band: None,
            p_permille: None,
            // 16b cut D.2: a unit no arena holds has no heat state to read, and
            // no frame either — so nothing is ever refused for heat off this row.
            heat_per_shot: 0,
        });
    }
    rows
}

/// **Every weapon the pilot can name**, live where a frame says so (16b cut W.1).
///
/// The frame's rows while the arena has spoken, and [`weapons_of`]'s structural read
/// of the M32 components **only where there is no frame at all** — the same
/// fail-closed rule [`power_of`] follows, and the reason a pilot who has just
/// boarded can still be told the cannon exists.
///
/// **16b cut D.1 narrowed the fallback to that one case.** It used to fall back
/// whenever the frame's list was *empty*, so a pilot whose turret had been shot
/// off read `cannon ready 40 of 40 round(s)` off the pre-match world record,
/// typed `FIRE`, was told `Firing cannon.` and learned the truth a pulse later
/// from the arena's own refusal. An empty list on a live frame is a **fact**: the
/// arena is the single writer while a match runs (D-16-1), and what it says the
/// platform carries is what the platform carries. `WEAPONSTATUS` then prints
/// `This unit carries no weapon.` and `FIRE` refuses in the cockpit with
/// [`CockpitError::NoSuchWeapon`], which is the same sentence and one pulse
/// earlier.
fn carried(w: &World, unit: ObjId, held: Option<&CockpitFrame>) -> Vec<WeaponRow> {
    match held {
        Some(held) => frame_weapons(&held.frame),
        None => weapons_of(w, unit),
    }
}

/// **The short name of a weapon id** (16b cut D.2, `B-17`): the initials of an
/// id of two or more `_`-separated words, and `None` for a one-word id.
///
/// `machine_gun` gives `mg`; `cannon` and `laser` give nothing, because a
/// one-letter `c` is not a name, it is a keystroke that would collide with the
/// first prefix anybody added.
///
/// **It is derived and not content-declared**, and that is a deliberate
/// departure from §5 Q14's default. Declaring short names means a new version of
/// all six unit packs, a re-blessed digest for each, and a story for every roster
/// tank's `DefinitionRef` on the live box — all for one two-letter word that the
/// id already spells. The derivation has the property the declaration was wanted
/// for (the pilot types two letters) and costs no content at all.
///
/// **It is not [`crate::match_::lobby`]'s private `short_name`, and the two are
/// unrelated.** That one answers the LAST SEGMENT of a canonical *unit* id —
/// `tacsim.unit.line_tank` gives `line_tank` — which is the opposite rule about a
/// different kind of id. This one is initials, over a component id.
#[must_use]
pub fn weapon_initials(id: &str) -> Option<String> {
    let words: Vec<&str> = id.split('_').filter(|word| !word.is_empty()).collect();
    if words.len() < 2 {
        return None;
    }
    let mut short = String::with_capacity(words.len());
    for word in words {
        for letter in word.chars().take(1) {
            short.push(letter.to_ascii_lowercase());
        }
    }
    Some(short)
}

/// **The short name `id` may be typed as on this platform** (16b cut D.2,
/// `B-17`), or `None` when it has none or when another row shares it.
///
/// A shared short name is not a name: two rows answering to `mg` means `FIRE mg`
/// is [`CockpitError::AmbiguousWeapon`], and a `WEAPONSTATUS` row that advertised
/// it would be advertising a refusal.
#[must_use]
pub fn weapon_short_of(rows: &[WeaponRow], id: &str) -> Option<String> {
    let short = weapon_initials(id)?;
    let shared = rows.iter().any(|row| {
        !row.id.eq_ignore_ascii_case(id)
            && weapon_initials(&row.id).is_some_and(|other| other == short)
    });
    (!shared).then_some(short)
}

/// The weapon `FIRE` means: the one the pilot named, else [`DEFAULT_WEAPON`],
/// else the first the unit carries — answering the whole row, so the caller's
/// pre-checks read the same numbers the choice was made from.
///
/// **16b cut D.2** (`B-17`) gave the named case three steps, case-folded and
/// trimmed, each tried in full before the next:
///
/// 1. the **exact id** — `FIRE machine_gun`, which is what every existing script
///    and every help line types and which therefore can never be ambiguous;
/// 2. the **short name** ([`weapon_initials`]) — `FIRE mg`;
/// 3. the **prefix** — `FIRE can`, `FIRE mach`.
///
/// Two or more candidates at a step is [`CockpitError::AmbiguousWeapon`] naming
/// them, and no candidate at any step is the existing
/// [`CockpitError::NoSuchWeapon`]. A step is not fallen through on ambiguity:
/// `m` matching two short names must not quietly become a prefix search that
/// matches the same two.
///
/// `B-17`: `help cockpit` told a pilot to type `FIRE mg` for four cuts and no
/// such weapon existed, because the ids are component ids.
fn choose_weapon(rows: &[WeaponRow], rest: &str) -> Result<WeaponRow, CockpitError> {
    let named = rest.trim();
    if !named.is_empty() {
        let folded = named.to_ascii_lowercase();
        let steps: [fn(&WeaponRow, &str) -> bool; 3] = [
            |row, word| row.id.eq_ignore_ascii_case(word),
            |row, word| weapon_initials(&row.id).is_some_and(|short| short == word),
            |row, word| row.id.to_ascii_lowercase().starts_with(word),
        ];
        for matches in steps {
            let found: Vec<&WeaponRow> = rows.iter().filter(|row| matches(row, &folded)).collect();
            match found.as_slice() {
                [] => {}
                [one] => return Ok((*one).clone()),
                many => {
                    return Err(CockpitError::AmbiguousWeapon {
                        word: named.to_owned(),
                        ids: many
                            .iter()
                            .map(|row| row.id.as_str())
                            .collect::<Vec<&str>>()
                            .join(" or "),
                    });
                }
            }
        }
        return Err(CockpitError::NoSuchWeapon(named.to_owned()));
    }
    rows.iter()
        .find(|weapon| weapon.id.as_str() == DEFAULT_WEAPON)
        .or_else(|| rows.first())
        .cloned()
        .ok_or_else(|| CockpitError::NoSuchWeapon(DEFAULT_WEAPON.to_owned()))
}

/// **Both of a unit's speed ceilings in cm/s** (16b cut D.3, `T-60`): forward,
/// and the reverse magnitude.
///
/// One read of the same `tracked` rules the arena's own apply reads
/// ([`crate::arena::state::TrackedRules::reverse_ceiling_cms`]), so `SPEED`'s
/// refusal and the arena's clamp cannot disagree about where the floor is. A unit
/// with no tactical record at all makes nothing in either direction.
fn speed_ceilings(w: &World, unit: ObjId) -> (u32, u32) {
    match mush_tactics::module::unit_of(w, unit) {
        Some(cells) => {
            let rules = crate::match_::tracked_rules(cells);
            (rules.max_speed_cms, rules.reverse_ceiling_cms())
        }
        None => (0, 0),
    }
}

/// Kilometres an hour to centimetres a second, and back. Integer both ways (doc
/// 16 §3.2: the wire is whole centimetres), so a pilot's round number is a round
/// number of cm/s and the readback is the same round number.
fn kph_to_cms(kph: i32) -> i32 {
    kph.saturating_mul(100_000) / 3_600
}

/// See [`kph_to_cms`].
fn cms_to_kph(cms: i32) -> i32 {
    cms.saturating_mul(36) / 1_000
}

/// **The contact a pilot's word names** (16b cut Q.1), or `None`.
///
/// It replaces cut K.2's `parse_contact`, and it **replaces** it: the sixteen
/// base-16 digits stop being typable altogether (16b §0.2 — no second lane). The
/// owner's first hand-played match is the reason: `lock 29b81a…` is not a thing a
/// person types while being shot at.
///
/// Two spellings resolve, both case-folded and trimmed:
///
/// * the row's **current** designator — `BA` while that contact is classified
///   now, `X2` while it is not;
/// * the row's **alias** rendered `X<n>`, which is the number the platform gave
///   the target before it was told apart, so a lock taken as `X2` is still
///   `LOCK X2` afterwards.
///
/// Everything else is `None`, and every `None` is the one byte-identical
/// [`NO_SUCH_CONTACT`]. There is deliberately no second wording for *well formed
/// but not held*: because a row carries the slot designator only while its
/// contact is classified **now**, `LOCK BA` against an unheld, a still-unknown or
/// a re-acquired-unknown `BA` has to be indistinguishable from `LOCK ZZ`, or the
/// refusal itself would say which tanks are out there.
///
/// The arena still only ever receives `IntentKind::Lock { contact: ContactId }`,
/// so a lock taken under one spelling survives the rename by construction.
#[must_use]
pub fn resolve_designator(rows: &[ContactRow], word: &str) -> Option<ContactId> {
    let named = word.trim();
    if named.is_empty() {
        return None;
    }
    rows.iter()
        .find(|row| {
            row.designator.to_string().eq_ignore_ascii_case(named)
                || row.alias.is_some_and(|n| {
                    Designator::Provisional(n)
                        .to_string()
                        .eq_ignore_ascii_case(named)
                })
        })
        .map(|row| row.id)
}

// ---------------------------------------------------------------------------
// The gate and the dispatch
// ---------------------------------------------------------------------------

/// **The contextual gate**: the fourteen verbs exist everywhere and mean
/// something in a team room, in a pod, and at a deployed unit's station.
///
/// One line each, and nothing else refused: a gate that is asked about `look`
/// stands aside, and a world with no centre at all lets `BOARD` through to the
/// verb's own "This server has no simulator centre.".
pub struct CockpitGate;

impl mush_core::CommandGate for CockpitGate {
    fn check(
        &self,
        w: &World,
        _reg: &Registry,
        actor: ObjId,
        verb: &str,
        _line: &str,
    ) -> Result<(), String> {
        // **16b cut F.4: the gate stands aside outside the centre.**
        //
        // A command gate runs at step 2 of `mush_core::cmd::dispatch`, before the
        // alias stage and before home, exits and the command table — so a gate
        // that refuses anywhere eats every line whose first word is its verb,
        // wherever the player is standing. `BOARD` did: the OOC tour bus's own
        // exit is named `Board the Bus <BUS>;bus;board`, and a newcomer at the Bus
        // Stop who typed `board` was told "Boarding is through one of the
        // simulator's team rooms." and did not move. `board`, `exit` and `status`
        // are ordinary English and content will collide with them again.
        //
        // So outside the centre's rooms and pods this gate decides nothing and
        // the line goes on to ordinary alias, exit and command matching. Inside
        // it, every arm is what it was — and a cockpit verb that does reach its
        // handler out there is refused by the handler with the same sentence
        // (`cockpit_command` opens on `flying`), so nothing is admitted that was
        // not admitted before.
        //
        // **The verb test comes first**, because `check` runs on EVERY dispatched
        // line (twice on the alias path) while `where_in_centre` resolves the
        // content latch up to eleven times — three rooms and then all eight pods.
        // A gate asked about `look` must cost what it always cost: two string
        // compares and an eleven-element scan, and no world read at all.
        let ours = verb.eq_ignore_ascii_case(BOARD_VERB)
            || verb.eq_ignore_ascii_case(EXIT_VERB)
            || COCKPIT_VERBS
                .iter()
                .any(|cockpit| verb.eq_ignore_ascii_case(cockpit));
        if !ours {
            return Ok(());
        }
        let standing = centre::where_in_centre(w, actor);
        if standing.is_none() {
            return Ok(());
        }
        if verb.eq_ignore_ascii_case(BOARD_VERB) {
            return match standing {
                Some(CentreRoom::Team(_)) => Ok(()),
                _ => Err(OUTSIDE_TEAM_ROOM.to_owned()),
            };
        }
        if verb.eq_ignore_ascii_case(EXIT_VERB) {
            return match pod_of_body(w, actor) {
                Some(_) => Ok(()),
                None => Err(NOT_IN_A_POD.to_owned()),
            };
        }
        if COCKPIT_VERBS
            .iter()
            .any(|cockpit| verb.eq_ignore_ascii_case(cockpit))
            && flying(w, actor).is_none()
        {
            // **16b cut F.1**: the third arm. A pilot who holds a cockpit billet
            // on a tank nothing has deployed yet is not somebody who has lost
            // their seat — they are sitting in it, waiting — and the sentence for
            // a fault is the wrong one to hand them.
            return Err(waiting_to_start(w, actor).to_owned());
        }
        Ok(())
    }
}

/// Which of the two "you are not flying" lines one actor is owed (16b cut F.1).
///
/// [`NOT_STARTED_YET`] for somebody who holds a cockpit billet on a tank the
/// centre has not deployed yet, [`OUTSIDE_COCKPIT`] for everybody else. The
/// billet rather than the focus, for the same reason `EXIT` reads the billet: a
/// pilot who typed `+focus` at something else still holds the seat.
///
/// **16b cut D.3 (`B-36`) narrowed it to the case it was written for.** The
/// sentence promises *the match has not started yet*, and the test was "holds a
/// cockpit billet on an undeployed unit" — which is true of a staffer sitting in
/// a tank of their own in an `Idle` centre, of somebody stationed on a unit that
/// belongs to no simulator at all, and of a pilot in a centre whose match ended
/// an hour ago. All three were told to wait for a match nobody is going to start.
/// Two facts are asked for now: the centre is really **`Boarding`**, and the
/// billet is on a **roster tank of that centre** — the eight objects the pods
/// resolve to, which is the same ladder [`eject_all`] walks.
fn waiting_to_start(w: &World, actor: ObjId) -> &'static str {
    if !lobby::centre(w).is_some_and(|centre| centre.state == MatchState::Boarding) {
        return OUTSIDE_COCKPIT;
    }
    let roster: Vec<ObjId> = centre::POD_ENTRIES
        .iter()
        .filter_map(|(team, index, _)| centre::pod_object(w, *team, *index))
        .filter_map(|pod| centre::tank_of(w, pod))
        .collect();
    let seated = station_id().is_some_and(|station| {
        seats_of(w, actor).into_iter().any(|(unit, held)| {
            held == station && !mush_sim::deployed::is_deployed(w, unit) && roster.contains(&unit)
        })
    });
    if seated {
        NOT_STARTED_YET
    } else {
        OUTSIDE_COCKPIT
    }
}

/// One object's name, or its dbref when it has none.
fn display_name(w: &World, object: ObjId) -> String {
    mush_core::output::display_name(w, object)
}

/// Say one line in a room, to everybody but `exclude`.
///
/// **16b cut D.3 (`T-60`)**: the exclusion is why this takes a list. `BOARD`
/// needs none — the body is inside the pod by the time the line is said, so a
/// room fan-out never reaches it — but `EXIT` puts the body back in the room
/// first, and the pilot then read both their own `You climb out of …` and the
/// room's `{name} climbs out of …`, which is the one line that was never meant
/// for them.
fn tell_room(ctx: &mut Ctx<'_>, room: ObjId, exclude: Vec<ObjId>, text: String) {
    ctx.out.to_room(
        room,
        exclude,
        OutputEvent::Sys(RichText::tag(NOTICE_CATEGORY, text)),
    );
}

/// Render one refusal as the line the pilot sees.
fn refusal(error: CockpitError) -> CmdError {
    CmdError::msg(error.to_string())
}

/// Run one cockpit verb: resolve the station first, so every one of the twelve
/// answers the same way to somebody who is not flying.
fn cockpit_command(ctx: &mut Ctx<'_>, verb: &'static str) -> Result<(), CmdError> {
    let Some((unit, _station)) = flying(ctx.w, ctx.actor) else {
        return Err(CmdError::msg(waiting_to_start(ctx.w, ctx.actor)));
    };
    let rest = ctx.args.raw().trim().to_owned();
    let outcome = match verb {
        "startup" => startup(ctx, unit),
        "heading" => heading(ctx, unit, &rest),
        "speed" => speed(ctx, unit, &rest),
        "view" => view(ctx),
        "contacts" => contacts(ctx),
        "lock" => lock(ctx, unit, &rest),
        "fire" => fire(ctx, unit, &rest),
        "weaponstatus" => weapon_status(ctx, unit),
        "status" => status(ctx, unit),
        // 16b cut Q.1: the two read verbs, off the same plot every other surface
        // reads.
        "range" => range(ctx, &rest),
        "bearing" => bearing(ctx, &rest),
        // 16b cut Q.2: what is left of a contact, off the reading the arena put
        // on it.
        "scan" => scan(ctx, &rest),
        // Not reachable: the table below registers exactly `COCKPIT_VERBS`, and
        // this arm is the closed-set proof rather than a panic.
        _ => Err(CockpitError::OutsideCockpit),
    };
    outcome.map_err(refusal)
}

/// The help one cockpit verb carries.
fn cockpit_help(verb: &str) -> String {
    match verb {
        "startup" => "STARTUP             bring the tank's power up. Idempotent; motion and fire \
                      refuse until it is online."
            .to_owned(),
        "heading" => "HEADING <0-359>     steer to a bearing, clockwise from north. Refused until \
                      power is online."
            .to_owned(),
        "speed" => "SPEED <kph>         make way at that speed; a negative speed reverses. \
                    Refused until power is online."
            .to_owned(),
        "view" => "VIEW                the arena window around your platform, as a map.".to_owned(),
        // 16b cut Q.1: the id is a designator now, and the help says so — a pilot
        // who reads `opaque id` will go looking for sixteen digits that are not
        // printed anywhere.
        "contacts" => "CONTACTS            what your platform holds: its designator, its class, \
                       bearing, range band, range and how old the plot is."
            .to_owned(),
        "lock" => "LOCK <contact>      hold one of your contacts by the designator CONTACTS \
                   prints (BA, or X2 before it is identified)."
            .to_owned(),
        "range" => "RANGE [<contact>]   how far away a contact plotted; your lock if you name \
                    none. A locked contact reads to the nearest ten metres."
            .to_owned(),
        "bearing" => "BEARING [<contact>] which way a contact plotted, absolute and off your own \
                      nose; your lock if you name none."
            .to_owned(),
        // 16b cut Q.2: and the help says what earns a reading, because a pilot
        // who reads `No reading yet` needs to know the answer is to drive closer.
        "scan" => "SCAN [<contact>]    what is left of a contact, section by section, as green, \
                   yellow, red or destroyed; your lock if you name none. Only for a contact you \
                   have identified and had a close look at, and never an exact number."
            .to_owned(),
        "fire" => "FIRE [<weapon>]     shoot your lock; the cannon unless you name another weapon."
            .to_owned(),
        // 16b cut F.2 gave both of these more to say, and a help line that
        // undersells a surface is a pilot who never looks at it.
        "weaponstatus" => "WEAPONSTATUS        every weapon you carry: ready or cycling, its \
                           rounds, its own chance against your lock, and that lock's range band."
            .to_owned(),
        // 16b cut D.2 (`T-77`): the callsign is `STATUS`'s first line and the
        // help did not mention it, so a pilot asked "who is RA?" had nowhere to
        // be sent; and cut D.1 replaced the arena's global tempo on the third
        // line with this platform's own scope.
        _ => "STATUS              your callsign, power, speed, heading, position, what your own \
              scope holds, and your armour and internals section by section."
            .to_owned(),
    }
}

/// Register the cockpit on the **tacsim** module: fourteen verbs and one gate.
///
/// The state the verbs read is [`ArenaRegistry`]'s and [`MatchBook`]'s, both
/// registered by [`crate::arena_bridge::register`], so nothing here declares a
/// slot.
///
/// # Errors
///
/// [`ModError`] if one of the verb names is already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    // `BOARD` writes a station document, a placement, a billet and (at the
    // trigger) cut T.3's whole check-out; `EXIT` writes the same in reverse. Of
    // the twelve selectors, five stage one arena intent each and the other seven
    // — cut Q.1's `RANGE` and `BEARING` and cut Q.2's `SCAN` among them — only
    // read the last frame; none of the twelve writes anything at all. They are
    // registered on the same
    // lane as their two neighbours rather than split across two classifications
    // for one subsystem.
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: BOARD_VERB.into(),
            shape: ArgShape::Raw,
            help: "BOARD               climb into the pod your simulator slot holds and take its \
                   tank's cockpit. In a team room, once the match is boarding; EXIT gets you out \
                   again."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(|ctx: &mut Ctx<'_>| board(ctx).map_err(refusal)),
    )?;
    reg.command(
        CommandSpec {
            name: EXIT_VERB.into(),
            shape: ArgShape::Raw,
            // 16b cut F.1: and the sentence that was missing — doc 16 §11's N.1
            // amendment is that a released seat cannot be retaken, and a pilot
            // who reads the help before typing the word deserves to know it.
            help: "EXIT                climb out of the pod you are in: the seat, its control \
                   lease and your focus are released, and a match still boarding stops waiting \
                   for you. Once the match is RUNNING this is final: the tank goes to its \
                   autopilot for the rest of the match and you cannot board again."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(|ctx: &mut Ctx<'_>| exit(ctx).map_err(refusal)),
    )?;
    for verb in COCKPIT_VERBS {
        reg.command(
            CommandSpec {
                name: verb.into(),
                shape: ArgShape::Raw,
                help: cockpit_help(verb),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| cockpit_command(ctx, verb)),
        )?;
    }
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command_gate(Arc::new(CockpitGate));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::intents::Refusal;

    #[test]
    fn the_cold_refusal_carries_the_arenas_own_word() {
        // The cockpit refuses before the arena does; the two must read alike, so
        // the sentence has to contain the arena's own word for it.
        let word = Refusal::PowerNotOnline.as_str().replace('_', " ");
        assert!(
            POWER_NOT_ONLINE.to_ascii_lowercase().contains(&word),
            "`{POWER_NOT_ONLINE}` does not contain `{word}`"
        );
    }

    /// **Every refusal has a sentence, and the sentence is somebody's voice**
    /// (16b cut F.3; F.2's judge).
    ///
    /// Cut F.2 ended [`refusal_line`]'s match with a catch-all that formatted the
    /// `thiserror` text, so a later cut routing a new variant into phase 4 would
    /// have shipped `FIRE refused. Refused: that unit already has as many actions
    /// queued as it may.` The match is exhaustive now; this is the other half,
    /// which is that each arm says something a pilot can read. The three the judge
    /// named — `Overheated`, `Wreck`, `Unknown` — are included with the rest,
    /// because a list that skips the awkward ones is the gap all over again.
    #[test]
    fn every_refusal_has_a_sentence_a_pilot_can_read() {
        use crate::arena::perception::RefusalDetail;
        for refusal in [
            Refusal::Epoch,
            Refusal::Channel,
            Refusal::PowerNotOnline,
            Refusal::QueueFull,
            Refusal::Wreck,
            Refusal::MotionDisabled,
            Refusal::Linkdead,
            Refusal::Unknown,
            Refusal::NoContact,
            Refusal::NoLock,
            Refusal::Cycling,
            Refusal::Empty,
            Refusal::Overheated,
            Refusal::NoWeapon,
            Refusal::NotReflex,
        ] {
            // 16b cut D.2 (`B-16`): the weapon is in the prefix now, so a pilot
            // with two guns reads which breech was refused.
            let line = refusal_line("fire", refusal, Some("cannon"), RefusalDetail::None);
            assert!(
                line.starts_with("FIRE cannon refused. "),
                "{refusal:?}: {line}"
            );
            let said = line.trim_start_matches("FIRE cannon refused. ");
            assert!(!said.is_empty(), "{refusal:?} says nothing");
            assert!(
                !said.starts_with("Refused:"),
                "{refusal:?} fell through to a catch-all: {line}"
            );
            assert!(said.ends_with('.'), "{refusal:?} is not a sentence: {line}");
            let opener = said.chars().next().unwrap_or(' ');
            assert!(
                opener.is_ascii_uppercase(),
                "{refusal:?} opens in lower case: {line}"
            );
        }
    }

    /// The three the judge named, in full, so their exact bytes are pinned.
    #[test]
    fn the_overheated_the_wreck_and_the_stray_unit_each_have_their_own_line() {
        use crate::arena::perception::RefusalDetail;
        assert_eq!(
            refusal_line(
                "fire",
                Refusal::Overheated,
                Some("laser"),
                RefusalDetail::Overheated { heat: 60, max: 60 },
            ),
            format!(
                "FIRE laser refused. {}",
                CockpitError::Overheated { heat: 60, max: 60 }
            )
        );
        // With no numbers behind it — a refusal staged before the cell was read —
        // it falls back to the cockpit's own constant rather than inventing a
        // nought that would read as "cold".
        assert_eq!(
            refusal_line("fire", Refusal::Overheated, None, RefusalDetail::None),
            format!("FIRE refused. {OVERHEATED}: let it cool.")
        );
        assert_eq!(
            refusal_line("fire", Refusal::Wreck, None, RefusalDetail::None),
            "FIRE refused. Your tank is a wreck."
        );
        assert_eq!(
            refusal_line("startup", Refusal::Unknown, None, RefusalDetail::None),
            "STARTUP refused. That tank is no longer in the match."
        );
    }

    /// **16b cut W.1**, the same rule one test up, for the two words `FIRE` gained:
    /// the cockpit refuses before the arena does, so each early line has to contain
    /// the arena's own word for the same refusal or the two can drift apart.
    #[test]
    fn the_fire_refusals_carry_the_arenas_own_words() {
        for (line, refusal) in [
            (NO_LOCK, Refusal::NoLock),
            (CYCLING, Refusal::Cycling),
            (EMPTY, Refusal::Empty),
        ] {
            let word = refusal.as_str().replace('_', " ");
            assert!(
                line.to_ascii_lowercase().contains(&word),
                "`{line}` does not contain `{word}`"
            );
        }
    }

    /// The line `WEAPONSTATUS` prints, at both ends of a cycle.
    #[test]
    fn a_weapon_row_renders_its_readiness_its_rounds_and_its_band() {
        let ready = WeaponRow {
            id: SmolStr::new("cannon"),
            ready: true,
            ready_in_ms: 0,
            rounds: 40,
            rounds_max: 40,
            range_band: Some(crate::arena::weapons::FrameBand {
                index: 1,
                of: 2,
                max_range_cm: 150_000,
                dispersion_mrad: 3,
            }),
            // 16b cut F.2: this weapon's own chance, not the platform's best.
            p_permille: Some(464),
            heat_per_shot: 0,
        };
        // 16b cut D.2: `cannon` is one word, so it has no short name to open.
        assert_eq!(
            ready.render(std::slice::from_ref(&ready)),
            "cannon  ready  40 of 40 round(s)  band 1 of 2  46% to hit"
        );
        // Cycling, with the seconds rounded **up**: a pilot told "0 s" would try
        // again and be refused. No lock, so no band and no chance.
        let cycling = WeaponRow {
            ready: false,
            ready_in_ms: 3_001,
            rounds: 39,
            range_band: None,
            p_permille: None,
            ..ready.clone()
        };
        assert_eq!(
            cycling.render(std::slice::from_ref(&cycling)),
            "cannon  cycling (4 s)  39 of 40 round(s)"
        );
        // **16b cut D.2** (`B-17`): a two-word id opens its initials, and only
        // where no other row answers to them.
        let mg = WeaponRow {
            id: SmolStr::new("machine_gun"),
            ..ready.clone()
        };
        let rows = vec![ready.clone(), mg.clone()];
        assert_eq!(
            mg.render(&rows),
            "machine_gun (mg)  ready  40 of 40 round(s)  band 1 of 2  46% to hit"
        );
        let missile = WeaponRow {
            id: SmolStr::new("missile_gun"),
            ..ready.clone()
        };
        let shared = vec![mg.clone(), missile.clone()];
        assert_eq!(
            mg.render(&shared),
            "machine_gun  ready  40 of 40 round(s)  band 1 of 2  46% to hit",
            "a short name two rows answer to is not a name"
        );
    }

    #[test]
    fn a_cockpit_seat_carries_exactly_the_three_channels() {
        let channels = cockpit_channels();
        assert_eq!(channels.len(), 3);
        for channel in Channel::ALL {
            assert!(
                ChannelId::new(channel.as_str()).is_some_and(|id| channels.contains(&id)),
                "{}",
                channel.as_str()
            );
        }
        // And the station is NOT named after one of them, which is what makes
        // `open_match` raise all three for it.
        assert_eq!(Channel::parse(STATION), None);
    }

    #[test]
    fn speed_converts_both_ways_at_the_stock_tanks_numbers() {
        // 15 m/s is the stock tank's maximum: 54 kph.
        assert_eq!(cms_to_kph(1_500), 54);
        assert_eq!(kph_to_cms(54), 1_500);
        assert_eq!(kph_to_cms(0), 0);
        assert_eq!(kph_to_cms(-20), -555);
        assert_eq!(cms_to_kph(-555), -19);
        // Saturating, not panicking, at the edges.
        assert_eq!(kph_to_cms(i32::MAX), i32::MAX / 3_600);
    }

    /// **16b cut Q.1**: a contact id still *renders* as its sixteen digits — the
    /// ledger, the deliberation brief and the frame's row key all carry that
    /// string — and it is no longer something a pilot can **type**. The parse
    /// half went with `parse_contact`; nothing replaced it in a second lane.
    #[test]
    fn a_contact_id_still_renders_as_sixteen_digits_and_is_no_longer_typable() {
        let id = ContactId(0x0123_4567_89ab_cdef);
        assert_eq!(id.to_string(), "0123456789abcdef");
        let rows = vec![row(
            id,
            Designator::Slot {
                team: crate::arena::state::Team::Blue,
                slot: 1,
            },
            Some(2),
        )];
        assert_eq!(resolve_designator(&rows, &id.to_string()), None);
        assert_eq!(resolve_designator(&rows, "1"), None);
    }

    /// One row, with only the three fields [`resolve_designator`] reads.
    fn row(id: ContactId, designator: Designator, alias: Option<u16>) -> ContactRow {
        ContactRow {
            id,
            designator,
            alias,
            class: SmolStr::new_static("blue"),
            bearing_deg: 87,
            relative_deg: 32,
            band: SmolStr::new_static("effective"),
            range_m: 450,
            age_ms: 2_000,
            cell: Some((4, 4)),
            friendly: false,
        }
    }

    /// **16b cut Q.1**: the two spellings a pilot may type, case-folded and
    /// trimmed, and nothing else.
    #[test]
    fn a_designator_resolves_by_its_current_name_or_by_the_number_behind_it() {
        let id = ContactId(7);
        let slot = Designator::Slot {
            team: crate::arena::state::Team::Blue,
            slot: 1,
        };
        let rows = vec![row(id, slot, Some(2))];
        for word in ["BA", "ba", " Ba ", "X2", "x2"] {
            assert_eq!(resolve_designator(&rows, word), Some(id), "{word}");
        }
        for word in ["", "ZZ", "X1", "B", "BAA", "2"] {
            assert_eq!(resolve_designator(&rows, word), None, "{word}");
        }
        // A row that is still provisional has no alias, and its slot spelling
        // resolves against nothing: `LOCK BA` on an unknown `BA` is refused
        // exactly as `LOCK ZZ` is.
        let unknown = vec![row(id, Designator::Provisional(2), None)];
        assert_eq!(resolve_designator(&unknown, "X2"), Some(id));
        assert_eq!(resolve_designator(&unknown, "BA"), None);
    }

    /// The row `CONTACTS` prints, and the column the designator sits in.
    #[test]
    fn a_contact_row_renders_its_designator_in_a_column_of_four() {
        let slot = Designator::Slot {
            team: crate::arena::state::Team::Blue,
            slot: 1,
        };
        assert_eq!(
            row(ContactId(7), slot, Some(2)).render(),
            "BA  blue  bearing 087° (rel 032°)  effective  450 m  2 s"
        );
        let mut unknown = row(ContactId(7), Designator::Provisional(12), None);
        unknown.class = SmolStr::new_static("unknown");
        assert_eq!(
            unknown.render(),
            "X12 unknown  bearing 087° (rel 032°)  effective  450 m  2 s"
        );
    }

    /// **16b cut Q.1**: two steps, one rounding rule. A lock earns ten metres and
    /// the rest of the scope fifty; half a step rounds up either way.
    #[test]
    fn a_range_rounds_to_its_own_step_and_a_lock_earns_the_finer_one() {
        assert_eq!(CONTACT_RANGE_STEP_M, 50);
        assert_eq!(LOCK_RANGE_STEP_M, 10);
        // 434 m: 450 at fifty, 430 at ten.
        assert_eq!(range_m_rounded(43_400, CONTACT_RANGE_STEP_M), 450);
        assert_eq!(range_m_rounded(43_400, LOCK_RANGE_STEP_M), 430);
        // Half a step up.
        assert_eq!(range_m_rounded(2_500, CONTACT_RANGE_STEP_M), 50);
        assert_eq!(range_m_rounded(500, LOCK_RANGE_STEP_M), 10);
        // A step of zero is read as one rather than dividing by it.
        assert_eq!(range_m_rounded(43_400, 0), 434);
    }

    /// **16b cut Q.1**: `FIRE`'s line is untouched, and the read verbs have their
    /// own — `FIRE` takes a weapon and never a contact, so "or name one" would
    /// be a lie to its pilot.
    #[test]
    fn the_read_verbs_refusal_is_its_own_and_fires_is_unchanged() {
        assert_eq!(NO_LOCK, "No lock: LOCK a contact first.");
        assert_eq!(
            NO_LOCK_OR_NAME,
            "No lock: LOCK a contact first, or name one."
        );
        assert_ne!(NO_LOCK, NO_LOCK_OR_NAME);
        assert_eq!(CockpitError::NoLock.to_string(), NO_LOCK);
        assert_eq!(CockpitError::NoLockOrName.to_string(), NO_LOCK_OR_NAME);
    }

    /// **16b cut Q.1**: a plot older than one sensor pass says so, and a live one
    /// says nothing — a track that announced `plotted 0 s ago` on every line
    /// would be noise where the point is a warning.
    #[test]
    fn a_read_verb_says_how_stale_a_plot_is_only_once_it_is() {
        assert_eq!(stale_clause(0, 1_000), "");
        assert_eq!(stale_clause(1_000, 1_000), "", "one pass is not stale yet");
        assert_eq!(stale_clause(1_001, 1_000), ", plotted 1 s ago");
        assert_eq!(stale_clause(4_200, 1_000), ", plotted 4 s ago");
        // A cadence of zero is read as one rather than dividing by it, so two
        // milliseconds are already past it.
        assert_eq!(stale_clause(1, 0), "");
        assert_eq!(stale_clause(2, 0), ", plotted 0 s ago");
        // And it is a clause, not a sentence: the full stop is the caller's.
        assert!(!stale_clause(4_200, 1_000).ends_with('.'));
    }

    #[test]
    fn every_registered_cockpit_verb_has_a_help_line_and_a_dispatch_arm() {
        for verb in COCKPIT_VERBS {
            assert!(!cockpit_help(verb).is_empty(), "{verb}");
            assert!(
                cockpit_help(verb)
                    .to_ascii_lowercase()
                    .contains(&verb.to_ascii_lowercase()),
                "{verb}'s help names it"
            );
        }
        assert_eq!(
            COCKPIT_VERBS.len(),
            12,
            "doc 16 section 3.7's nine, cut Q.1's two read verbs and cut Q.2's SCAN"
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut D.2
    // -----------------------------------------------------------------------

    /// One weapon row with an id and nothing else that matters here.
    fn weapon_row(id: &str) -> WeaponRow {
        WeaponRow {
            id: SmolStr::new(id),
            ready: true,
            ready_in_ms: 0,
            rounds: 10,
            rounds_max: 10,
            range_band: None,
            p_permille: None,
            heat_per_shot: 0,
        }
    }

    /// **`B-17`**: the short name is the id's initials, and a one-word id has
    /// none — a single letter is a keystroke, not a name.
    #[test]
    fn d2_a_weapon_of_two_words_is_typable_as_its_initials() {
        assert_eq!(weapon_initials("machine_gun").as_deref(), Some("mg"));
        assert_eq!(weapon_initials("missile_rack_two").as_deref(), Some("mrt"));
        assert_eq!(weapon_initials("MACHINE_GUN").as_deref(), Some("mg"));
        assert_eq!(weapon_initials("cannon"), None);
        assert_eq!(weapon_initials("laser"), None);
        assert_eq!(weapon_initials(""), None);
        assert_eq!(weapon_initials("_"), None);
        // It is NOT `lobby::short_name`, which answers the last segment of a
        // canonical unit id. The two rules are opposite and are about different
        // kinds of id.
        assert_ne!(
            weapon_initials("tacsim_unit_line_tank").as_deref(),
            Some("line_tank")
        );
    }

    /// **`B-17`**: `FIRE mg` works, `FIRE can` works, and a word two weapons
    /// answer to is refused saying which.
    #[test]
    fn d2_choose_weapon_takes_the_id_the_short_name_and_the_prefix() {
        let rows = vec![weapon_row("cannon"), weapon_row("machine_gun")];
        let chosen = |word: &str| {
            choose_weapon(&rows, word)
                .map(|row| row.id.to_string())
                .map_err(|err| err.to_string())
        };
        // (1) the exact id, case-folded and trimmed.
        assert_eq!(chosen("machine_gun"), Ok("machine_gun".to_owned()));
        assert_eq!(chosen("  MACHINE_GUN "), Ok("machine_gun".to_owned()));
        // (2) the short name.
        assert_eq!(chosen("mg"), Ok("machine_gun".to_owned()));
        assert_eq!(chosen("MG"), Ok("machine_gun".to_owned()));
        // (3) the prefix.
        assert_eq!(chosen("can"), Ok("cannon".to_owned()));
        assert_eq!(chosen("mach"), Ok("machine_gun".to_owned()));
        // None: the existing sentence, unchanged.
        assert_eq!(
            chosen("railgun"),
            Err("That unit carries no weapon called `railgun`.".to_owned())
        );
        // An empty word is the default weapon and never a search.
        assert_eq!(chosen(""), Ok(DEFAULT_WEAPON.to_owned()));
        assert_eq!(chosen("   "), Ok(DEFAULT_WEAPON.to_owned()));
        // Ambiguous at the SHORT-NAME step, and at the PREFIX step, each naming
        // the candidates in row order.
        let two = vec![weapon_row("machine_gun"), weapon_row("missile_gun")];
        assert_eq!(
            choose_weapon(&two, "mg").map_err(|err| err.to_string()),
            Err("`mg` could be machine_gun or missile_gun; say more.".to_owned())
        );
        assert_eq!(
            choose_weapon(&two, "m").map_err(|err| err.to_string()),
            Err("`m` could be machine_gun or missile_gun; say more.".to_owned())
        );
        // A step is not fallen through on ambiguity: `m` matching two short names
        // must not quietly become a prefix search that matches the same two.
        assert!(matches!(
            choose_weapon(&two, "mis"),
            Ok(row) if row.id == "missile_gun"
        ));
        // And the exact id always works, however crowded the rack is.
        assert!(matches!(
            choose_weapon(&two, "machine_gun"),
            Ok(row) if row.id == "machine_gun"
        ));
    }

    /// **`B-17`**: the row advertises a short name only where it is one.
    #[test]
    fn d2_a_weapon_row_opens_its_short_name_only_when_it_is_unshared() {
        let rows = vec![weapon_row("cannon"), weapon_row("machine_gun")];
        assert_eq!(weapon_short_of(&rows, "machine_gun").as_deref(), Some("mg"));
        assert_eq!(weapon_short_of(&rows, "cannon"), None);
        let shared = vec![weapon_row("machine_gun"), weapon_row("missile_gun")];
        assert_eq!(weapon_short_of(&shared, "machine_gun"), None);
        assert_eq!(weapon_short_of(&shared, "missile_gun"), None);
    }

    /// **`B-16`**: one sentence for every `FIRE` refusal, and the weapon is in
    /// it. The `LOCK` and `STARTUP` shapes are untouched.
    #[test]
    fn d2_every_fire_refusal_names_the_weapon_and_the_others_do_not() {
        use crate::arena::intents::Refusal;
        use crate::arena::perception::RefusalDetail;
        assert_eq!(
            refusal_line(
                "fire",
                Refusal::Overheated,
                Some("machine_gun"),
                RefusalDetail::Overheated { heat: 60, max: 60 },
            ),
            "FIRE machine_gun refused. Overheated: 60 of 60. Let it cool."
        );
        assert_eq!(
            refusal_line("lock", Refusal::PowerNotOnline, None, RefusalDetail::None),
            format!("LOCK refused. {POWER_NOT_ONLINE}")
        );
        assert_eq!(
            refusal_line("startup", Refusal::Wreck, None, RefusalDetail::None),
            "STARTUP refused. Your tank is a wreck."
        );
        // The shape itself, and its empty-weapon case.
        assert_eq!(
            order_refused("heading", None, "Your tank cannot move."),
            "HEADING refused. Your tank cannot move."
        );
        assert_eq!(
            order_refused("fire", Some("  "), "No lock: LOCK a contact first."),
            "FIRE refused. No lock: LOCK a contact first."
        );
        // The cockpit's own pre-check builds the SAME string through the SAME
        // function, so the early answer and the late one cannot drift.
        assert_eq!(
            fire_refused("cannon", &CockpitError::NoLock).to_string(),
            refusal_line("fire", Refusal::NoLock, Some("cannon"), RefusalDetail::None)
        );
        assert_eq!(
            fire_refused("cannon", &CockpitError::Empty).to_string(),
            refusal_line("fire", Refusal::Empty, Some("cannon"), RefusalDetail::None)
        );
        assert_eq!(
            fire_refused("cannon", &CockpitError::Cycling { secs: 3 }).to_string(),
            refusal_line(
                "fire",
                Refusal::Cycling,
                Some("cannon"),
                RefusalDetail::Cycling { secs: 3 }
            )
        );
        assert_eq!(
            fire_refused("laser", &CockpitError::Overheated { heat: 60, max: 60 }).to_string(),
            refusal_line(
                "fire",
                Refusal::Overheated,
                Some("laser"),
                RefusalDetail::Overheated { heat: 60, max: 60 }
            )
        );
        assert_eq!(
            fire_refused("cannon", &CockpitError::PowerNotOnline).to_string(),
            refusal_line(
                "fire",
                Refusal::PowerNotOnline,
                Some("cannon"),
                RefusalDetail::None
            )
        );
        // `PowerStarting` has no `Refusal` of its own — the arena never answers
        // it — but it takes the same prefix, so a pilot reads one shape.
        assert_eq!(
            fire_refused("cannon", &CockpitError::PowerStarting).to_string(),
            format!("FIRE cannon refused. {POWER_STARTING}")
        );
    }

    /// **`B-03`**: the two local sentences, and the motion one pinned to the
    /// arena's own word for the same condition.
    #[test]
    fn d2_the_wreck_and_the_motion_limit_have_their_own_sentences() {
        use crate::arena::intents::Refusal;
        use crate::arena::perception::RefusalDetail;
        assert_eq!(
            CockpitError::Destroyed.to_string(),
            "Your tank is destroyed and takes no orders. EXIT leaves the pod."
        );
        assert_eq!(CockpitError::Immobile.to_string(), IMMOBILE);
        assert_eq!(
            refusal_line(
                "heading",
                Refusal::MotionDisabled,
                None,
                RefusalDetail::None
            ),
            format!("HEADING refused. {IMMOBILE}"),
            "the local sentence and the arena's are one string"
        );
        assert_eq!(
            OWN_DESTROYED,
            "Your tank is destroyed. The match runs on without you: wait it out, EXIT to the \
             team room, or SIM ABORT to call it off."
        );
        assert_eq!(
            OWN_DISABLED,
            "Your tank can neither move nor shoot. The match runs on: wait it out, EXIT to the \
             team room, or SIM ABORT to call it off."
        );
        // Neither names anything but the pilot's own tank.
        for line in [OWN_DESTROYED, OWN_DISABLED, DESTROYED, IMMOBILE] {
            assert!(!line.contains("bearing"), "{line}");
            assert!(!line.contains('#'), "{line}");
        }
    }

    /// **`T-77`**: the module head, the verb table and [`COCKPIT_VERBS`] agree,
    /// and the `STATUS` help names the callsign line.
    #[test]
    fn d2_the_verb_table_and_the_help_agree_with_the_selector_list() {
        assert_eq!(COCKPIT_VERBS.len(), 12);
        let head = include_str!("cockpit.rs");
        for verb in COCKPIT_VERBS {
            let upper = verb.to_uppercase();
            assert!(
                head.contains(&format!("//! {upper}")),
                "the module head's table omits {upper}"
            );
        }
        assert!(head.contains("//! BOARD"));
        assert!(head.contains("//! EXIT"));
        let status = cockpit_help("status");
        assert!(status.contains("callsign"), "{status}");
        assert!(status.contains("scope"), "{status}");
        assert!(
            !status.contains("tempo"),
            "the help still promises the arena's global tempo: {status}"
        );
    }
}
