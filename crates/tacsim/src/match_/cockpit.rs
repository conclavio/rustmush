//! **The cockpit: boarding, the pilot's ten verbs, and the frame they read**
//! (16b cut K.2; 16b §8 cut K, K2–K4; doc 16 §3.3 "Startup", "Stations are on
//! the tank", §3.6 "What a pilot sees", §3.7, §3.8 "Disconnects", §5).
//!
//! Cut K.1 built the lobby: a centre, slots, pods locked to pilots, and the
//! named `Boarding → Running` trigger waiting for somebody to report a seat.
//! This is the half that reports one, and everything the pilot can do once
//! seated.
//!
//! ```text
//! BOARD                      climb into your pod and take the tank's cockpit
//! STARTUP                    bring power up (idempotent)
//! HEADING <0-359>            steer
//! SPEED <kph>                make way (negative is reverse)
//! VIEW                       the arena window, as a text map
//! CONTACTS                   what the platform holds
//! LOCK <contact>             hold one of them
//! FIRE [<weapon>]            shoot it
//! WEAPONSTATUS               ammunition and readiness
//! STATUS                     power, speed, heading, tempo
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
//! table is global. So the eleven verbs are registered globally and
//! [`CockpitGate`] refuses each of them, with one line, wherever it does not
//! mean anything — `BOARD` outside a team room, `EXIT` outside a pod, and the
//! nine cockpit selectors to anyone who is not focused on a station of a
//! **deployed** unit. A pilot who has boarded but whose match has not started
//! yet is in that last case on purpose: there is no arena to order until the
//! trigger fires.
//!
//! # Where the numbers come from
//!
//! The arena is the only thing that knows a tank's speed, heading, power or
//! tempo, and it publishes them as the `tacsim.frame` keyframe cut T.2 opened.
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
use crate::arena::state::{Channel, Classification, ContactId, Power};
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

/// The nine cockpit selectors, in doc 16 §3.7's order. Registered lowercase
/// because the command table folds case on the way in (`Registry::lookup_idx`),
/// so doc 16's capitals and a tired pilot's lower case are one verb.
pub const COCKPIT_VERBS: [&str; 9] = [
    "startup",
    "heading",
    "speed",
    "view",
    "contacts",
    "lock",
    "fire",
    "weaponstatus",
    "status",
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

/// The power state the cockpit will act on: the last frame's, or [`Power::Off`]
/// for a pilot the arena has not spoken to yet.
fn power_of(w: &World, pilot: ObjId) -> Power {
    last_frame(w, pilot).map_or(Power::Off, |held| held.frame.power)
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
/// the opaque id unchanged, the class as `unknown` or as the side the platform has
/// resolved, the bearing in whole degrees both absolute and relative to the
/// platform's own heading, the range as one of [`range_band`]'s four words, and
/// the age in whole seconds.
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
            class: SmolStr::new(class_word(held.class)),
            bearing_deg: held.bearing_mdeg / 1_000,
            relative_deg: relative_mdeg(frame.pose.heading_mdeg, held.bearing_mdeg) / 1_000,
            band: SmolStr::new(range_band(held.range_cm, bands)),
            range_m: range_m_rounded(held.range_cm),
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
/// the band could only ever read `close` (see [`range_band`]), so from
/// acquisition to kill every row said the same word whether the other tank was
/// 600 m away or 40 m away.
///
/// A passive sight giving a coarse range is a **playtest-usability decision**,
/// recorded here for the owner to revisit: it departs from doc 16 §3.6 in the
/// direction of a pilot being able to act. The rounding is the whole of the
/// coarseness and it lives in this one constant, so tightening or loosening it is
/// one number.
pub const CONTACT_RANGE_STEP_M: u64 = 50;

/// A contact's range in metres, rounded to the nearest
/// [`CONTACT_RANGE_STEP_M`] (16b cut F.3). Half a step rounds up.
#[must_use]
pub fn range_m_rounded(range_cm: u64) -> u64 {
    let step = CONTACT_RANGE_STEP_M.max(1);
    let metres = range_cm / 100;
    metres
        .saturating_add(step / 2)
        .saturating_div(step)
        .saturating_mul(step)
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

/// **The range band of a contact** (doc 16 §3.6: *range band*), read against the
/// **authored bands of the weapon a bare `FIRE` would fire**
/// ([`crate::arena::weapons::FrameWeapon::bands_cm`], outward, the same list
/// `WeaponSummary::band_of` walks).
///
/// A word and not just a range, because the question a contact's distance answers
/// is *can I shoot it, nearly, or not at all* — and that question is about a
/// **weapon**, which is why the bands it is read against are the ones the weapon
/// declares:
///
/// | word | range |
/// |---|---|
/// | `close` | inside a third of the innermost band, where the cone is tightest |
/// | `effective` | inside the innermost band |
/// | `far` | inside one of the outer bands |
/// | `beyond` | past the last band, **or the platform carries nothing that fires** |
///
/// The last clause is why an empty band list is not a special case: a platform
/// with nothing to shoot with has nothing in range, and `beyond` says so. It is
/// reserved for exactly that — a weapon that declares a reach and authors no
/// bands answers one band at its reach ([`crate::arena::state::WeaponSummary::bands_cm`]),
/// so `beyond` never means *this weapon's content is thin*.
///
/// **16b cut F.3 replaced cut S.2's thirds of `Frame`'s `weapon_reach_cm`**, and
/// deleted the field with it (its one reader was this function). That
/// read the band against the *longest reach the platform carried*, which for a
/// stock tank is the cannon's 3 km, so `close` covered everything inside 1 km —
/// and the shipped sight cannot detect past 600 m, even with a mesh hop. Every
/// contact that has ever existed in this game printed `close`, which inverted doc
/// 16 §3.6's stated purpose for the band exactly.
#[must_use]
pub fn range_band(range_cm: u64, bands_cm: &[u32]) -> &'static str {
    let (Some(first), Some(last)) = (bands_cm.first(), bands_cm.last()) else {
        return "beyond";
    };
    let (first, last) = (u64::from(*first), u64::from(*last));
    if range_cm.saturating_mul(3) <= first {
        "close"
    } else if range_cm <= first {
        "effective"
    } else if range_cm <= last {
        "far"
    } else {
        "beyond"
    }
}

/// One contact, as a pilot is told about it (doc 16 §3.6 "What a pilot sees").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactRow {
    /// The opaque arena id, which is the only name a pilot may use.
    pub id: ContactId,
    /// `unknown`, or the side the platform has resolved.
    pub class: SmolStr,
    /// Bearing, degrees clockwise from north.
    pub bearing_deg: u32,
    /// The same bearing off the platform's own nose, degrees clockwise (16b cut
    /// S.2): `0` is dead ahead.
    pub relative_deg: u32,
    /// The range band's label ([`range_band`]).
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
    /// **16b cut F.3** put the range in it, between the band and the age:
    /// `<id>  <class>  bearing 090° (rel 000°)  close  300 m  1 s`.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{}  {}  bearing {:03}° (rel {:03}°)  {}  {} m  {} s",
            self.id,
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
#[must_use]
pub fn notice_line(frame: &Frame, notice: &FrameNotice, rows: &[ContactRow]) -> String {
    match notice {
        FrameNotice::Contact {
            kind,
            contact,
            class,
        } => {
            let row = rows.iter().find(|row| row.id == *contact);
            match kind {
                NoticeKind::Acquired => match row {
                    Some(row) => format!(
                        "New contact {}: bearing {:03}°, {}.",
                        row.id, row.bearing_deg, row.band
                    ),
                    None => format!("New contact {contact}."),
                },
                NoticeKind::Classified => {
                    format!("Contact {contact} identified: {}.", class_word(*class))
                }
                // 16b cut W.1: the lock broke because the contact stopped being
                // held. Said at once, because the pilot's next `FIRE` is going to
                // be refused for it.
                NoticeKind::LockLost => format!("Lock on {contact} broken: contact gone."),
                // `Lost`, and anything a later cut puts in the contact arm
                // without a sentence of its own.
                _ => format!("Contact {contact} lost."),
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
        FrameNotice::Locked { contact } => match frame_lock(frame) {
            Some(held) if held == *contact => {
                lock_line(frame).unwrap_or_else(|| format!("Locked on {contact}."))
            }
            _ => format!("Locked on {contact}."),
        },
        FrameNotice::Hit { contact } => match contact {
            Some(contact) => format!("Hit on {contact}."),
            None => "Hit.".to_owned(),
        },
        FrameNotice::Miss { contact, reason } => match contact {
            Some(contact) => format!("Miss on {contact}: {}.", miss_words(reason)),
            None => format!("Miss: {}.", miss_words(reason)),
        },
        FrameNotice::Kill { contact } => match contact {
            Some(contact) => format!("{contact} destroyed."),
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
    }
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
    format!("{} refused. {said}", order.to_uppercase())
}

/// Every line the notices on one frame print, in order — what the bridge says and
/// what a read verb's frame carries, from one place.
#[must_use]
pub fn notice_lines(frame: &Frame) -> Vec<String> {
    let rows = frame_contacts(frame);
    frame
        .notices
        .iter()
        .map(|notice| notice_line(frame, notice, &rows))
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
    let contact = frame_lock(frame)?;
    Some(
        match fired_by_default(&frame.weapons).and_then(|weapon| weapon.p_permille) {
            Some(permille) => format!("Locked on {contact}, {}% to hit.", permille / 10),
            None => format!("Locked on {contact}."),
        },
    )
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
        tell_room(ctx, room, room_line);
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
        tell_room(ctx, room, text);
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
// K3: the nine cockpit verbs
// ---------------------------------------------------------------------------

/// `STARTUP` — bring power up (doc 16 §3.3). Idempotent: the arena's own
/// `resolve_startup` is what decides whether anything changes, and a second
/// `STARTUP` while `starting` costs one queued action and no refusal.
fn startup(ctx: &mut Ctx<'_>, unit: ObjId) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    forward_intent(ctx, actor, unit, IntentKind::Startup)?;
    ctx.out.sys_to_actor(
        actor,
        NOTICE_CATEGORY,
        "Startup sequence begun; power comes online shortly.",
    );
    Ok(())
}

/// `HEADING <deg>` — 0 to 359, clockwise from north (doc 16 §3.7).
fn heading(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let deg: u32 = rest
        .trim()
        .parse()
        .ok()
        .filter(|deg| *deg < 360)
        .ok_or_else(|| CockpitError::Usage("HEADING <0-359>, clockwise from north.".to_owned()))?;
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

/// `SPEED <kph>` — 0 to the unit's own maximum, negative in reverse (doc 16
/// §3.7). The wire is centimetres a second (doc 16 §3.2's integers), so the
/// conversion is here and the pilot never types one.
fn speed(ctx: &mut Ctx<'_>, unit: ObjId, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let kph: i32 =
        rest.trim().parse().ok().ok_or_else(|| {
            CockpitError::Usage("SPEED <kph>; a negative speed reverses.".to_owned())
        })?;
    let max_cms = max_speed_cms(ctx.w, unit);
    let max_kph = cms_to_kph(i32::try_from(max_cms).unwrap_or(i32::MAX));
    if kph.abs() > max_kph {
        return Err(CockpitError::Usage(format!(
            "That unit makes {max_kph} kph; SPEED 0 to {max_kph}."
        )));
    }
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
    let held = last_frame(ctx.w, actor)
        .map(|held| frame_contacts(&held.frame))
        .unwrap_or_default();
    let contact = parse_contact(rest)
        .filter(|id| held.iter().any(|row| row.id == *id))
        .ok_or(CockpitError::NoSuchContact)?;
    forward_intent(ctx, actor, unit, IntentKind::Lock { contact })?;
    let line = format!("Locking {contact}.");
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
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
    let held = last_frame(ctx.w, actor);
    let weapon = choose_weapon(&carried(ctx.w, unit, held.as_ref()), rest)?;
    if held
        .as_ref()
        .and_then(|held| frame_lock(&held.frame))
        .is_none()
    {
        return Err(CockpitError::NoLock);
    }
    if !weapon.ready {
        return Err(CockpitError::Cycling {
            secs: weapon.ready_in_ms.saturating_add(999) / 1_000,
        });
    }
    if weapon.rounds <= 0 {
        return Err(CockpitError::Empty);
    }
    // **16b cut X.1** (doc 16 §9 item 1), in the arena's own order: the platform's
    // heat after the weapon's own two refusals and before the power. A platform
    // whose content declares no heat carries no `heat` on its frame and is never
    // refused here.
    if let Some(heat) = held.as_ref().and_then(|held| held.frame.heat)
        && heat.overheated()
    {
        return Err(CockpitError::Overheated {
            heat: heat.heat,
            max: heat.max,
        });
    }
    require_power(ctx.w, actor)?;
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
    say_and_frame(ctx, held.as_ref(), lines, false);
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
        lines.push(weapon.render());
    }
    say_and_frame(ctx, held.as_ref(), lines, false);
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
                format!(
                    "{}: power {}, {} kph, heading {}°{}",
                    display_name(ctx.w, unit),
                    frame.power.as_str(),
                    cms_to_kph(frame.speed_cms),
                    frame.pose.heading_mdeg / 1_000,
                    if frame.blocked { ", HELD" } else { "" }
                ),
                format!(
                    "  at {} m east, {} m north; the arena is running {}",
                    frame.pose.x_cm / 100,
                    frame.pose.y_cm / 100,
                    frame.tempo.as_str()
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
    say_and_frame(ctx, held.as_ref(), lines, false);
    Ok(())
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
fn view(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let held = last_frame(ctx.w, actor);
    let lines = match held.as_ref() {
        // The map is the frame's; a line client's renderer draws it. Nothing is
        // said twice.
        Some(_) => Vec::new(),
        None => vec!["The window is dark: the panel has told you nothing yet.".to_owned()],
    };
    say_and_frame(ctx, held.as_ref(), lines, true);
    Ok(())
}

/// Say what a read verb answers and push the frame it read (doc 16 §5: one feed,
/// and the line renderer prints the frame).
fn say_and_frame(
    ctx: &mut Ctx<'_>,
    held: Option<&CockpitFrame>,
    lines: Vec<String>,
    with_view: bool,
) {
    let actor = ctx.actor;
    for line in lines {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    let Some(held) = held else {
        return;
    };
    let payload = if with_view {
        view_frame(ctx, held.match_id, &held.frame)
    } else {
        status_frame(held.match_id, &held.frame)
    };
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
#[must_use]
pub fn status_frame(match_id: MatchId, frame: &Frame) -> serde_json::Value {
    let rows = frame_contacts(frame);
    let contacts: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "id": row.id.to_string(),
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
            })
        })
        .collect();
    let notices: Vec<serde_json::Value> = frame
        .notices
        .iter()
        .map(|notice| {
            serde_json::json!({
                "kind": notice.kind().as_str(),
                "contact": notice.contact().map(|id| id.to_string()),
                "text": notice_line(frame, notice, &rows),
            })
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
    serde_json::json!({
        "schema": { "id": FRAME_SCHEMA, "version": FRAME_VERSION },
        "match": match_id.0,
        "unit": frame.unit.0,
        "x_cm": frame.pose.x_cm,
        "y_cm": frame.pose.y_cm,
        "z_cm": frame.pose.z_cm,
        "heading_mdeg": frame.pose.heading_mdeg,
        "heading_deg": frame.pose.heading_mdeg / 1_000,
        "speed_cms": frame.speed_cms,
        "speed_kph": cms_to_kph(frame.speed_cms),
        "power": frame.power.as_str(),
        "blocked": frame.blocked,
        "tempo": frame.tempo.as_str(),
        "lock": frame_lock(frame).map(|id| id.to_string()),
        "lock_p_permille": chosen,
        "lock_percent": chosen.map(|permille| permille / 10),
        "contacts": contacts,
        "notices": notices,
        "weapons": weapons,
        "sections": sections,
        "heat": frame.heat.map(|heat| serde_json::json!({
            "heat": heat.heat,
            "max": heat.max,
            "overheated": heat.overheated(),
        })),
    })
}

/// **The cockpit keyframe with the window in it** — [`status_frame`] plus the
/// `grid` shape `mush_ansi::panel` renders as a text map and the web draws as a
/// map panel (doc 16 §5: *the ANSI line renderer prints the frame as text; the
/// TUI and web render the same structure*).
///
/// The window is [`VIEW_COLUMNS`] by [`VIEW_ROWS`] cells around the platform,
/// clamped to the arena — the most a line client prints, so the renderer only
/// has to clip it to a narrow terminal and never has to ask for more. One glyph
/// per terrain class, from the atlas's own table, so the arena's map and the
/// atlas's map read alike.
#[must_use]
pub fn view_frame(ctx: &Ctx<'_>, match_id: MatchId, frame: &Frame) -> serde_json::Value {
    let mut payload = status_frame(match_id, frame);
    let Some(ground) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
    else {
        return payload;
    };
    let terrain = &ground.terrain;
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
        "name": display_name(ctx.w, frame.unit),
        "cell": you,
        "glyph": YOU_GLYPH,
        "you": true,
    })];
    // **16b cut S.2**: the platform's own contacts, at their last known cell, one
    // marker each, clipped to the window the pilot is looking at. The `name` is
    // the class word and the `id` is the opaque contact id — the marker carries no
    // object number and no unit name, which is the observer rule on the map.
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
        units.push(serde_json::json!({
            "id": row.id.to_string(),
            "name": row.class.as_str(),
            "cell": [u32::from(dx), u32::from(dy)],
            "glyph": row.glyph(),
            "you": false,
        }));
        // The legend gains a glyph only when the window actually carries one: a
        // legend that always advertised `hostile` would tell a pilot with an empty
        // scope what a full one looks like, which is one more thing than they hold.
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
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "schema".to_owned(),
            serde_json::json!({ "id": FRAME_SCHEMA, "version": FRAME_VERSION, "shape": "grid" }),
        );
        object.insert("title".to_owned(), serde_json::json!("Arena window"));
        object.insert("surface".to_owned(), serde_json::json!(ground.id.as_str()));
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
    #[must_use]
    pub fn render(&self) -> String {
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
            "{}  {}  {} of {} round(s){band}{chance}",
            self.id, ready, self.rounds, self.rounds_max
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
        });
    }
    rows
}

/// **Every weapon the pilot can name**, live where a frame says so (16b cut W.1).
///
/// The frame's rows while the arena has spoken, and [`weapons_of`]'s structural read
/// of the M32 components before it has — which is the same fail-closed rule
/// [`power_of`] follows, and the reason a pilot who has just boarded can still be
/// told the cannon exists.
fn carried(w: &World, unit: ObjId, held: Option<&CockpitFrame>) -> Vec<WeaponRow> {
    match held {
        Some(held) if !held.frame.weapons.is_empty() => frame_weapons(&held.frame),
        _ => weapons_of(w, unit),
    }
}

/// The weapon `FIRE` means: the one the pilot named, else [`DEFAULT_WEAPON`],
/// else the first the unit carries — answering the whole row, so the caller's
/// pre-checks read the same numbers the choice was made from.
fn choose_weapon(rows: &[WeaponRow], rest: &str) -> Result<WeaponRow, CockpitError> {
    let named = rest.trim();
    if !named.is_empty() {
        return rows
            .iter()
            .find(|weapon| weapon.id.eq_ignore_ascii_case(named))
            .cloned()
            .ok_or_else(|| CockpitError::NoSuchWeapon(named.to_owned()));
    }
    rows.iter()
        .find(|weapon| weapon.id.as_str() == DEFAULT_WEAPON)
        .or_else(|| rows.first())
        .cloned()
        .ok_or_else(|| CockpitError::NoSuchWeapon(DEFAULT_WEAPON.to_owned()))
}

/// The unit's own top speed, centimetres a second, out of its `tracked` mode
/// rules (doc 16 §3.3). A unit the world does not hold gets the stock tank's,
/// which is the same fallback the check-out uses.
fn max_speed_cms(w: &World, unit: ObjId) -> u32 {
    match mush_tactics::module::unit_of(w, unit) {
        Some(cells) => crate::match_::tracked_rules(cells).max_speed_cms,
        None => 0,
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

/// One contact id, as a pilot may type it: the sixteen base-16 digits
/// [`ContactId`] renders as, with or without their leading zeros.
fn parse_contact(rest: &str) -> Option<ContactId> {
    let word = rest.trim();
    if word.is_empty() || word.len() > 16 {
        return None;
    }
    u64::from_str_radix(word, 16).ok().map(ContactId)
}

// ---------------------------------------------------------------------------
// The gate and the dispatch
// ---------------------------------------------------------------------------

/// **The contextual gate**: the eleven verbs exist everywhere and mean something
/// in a team room, in a pod, and at a deployed unit's station.
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
        // compares and a nine-element scan, and no world read at all.
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
fn waiting_to_start(w: &World, actor: ObjId) -> &'static str {
    let seated = station_id().is_some_and(|station| {
        seats_of(w, actor)
            .into_iter()
            .any(|(unit, held)| held == station && !mush_sim::deployed::is_deployed(w, unit))
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

/// Say one line in a room.
fn tell_room(ctx: &mut Ctx<'_>, room: ObjId, text: String) {
    ctx.out.to_room(
        room,
        Vec::new(),
        OutputEvent::Sys(RichText::tag(NOTICE_CATEGORY, text)),
    );
}

/// Render one refusal as the line the pilot sees.
fn refusal(error: CockpitError) -> CmdError {
    CmdError::msg(error.to_string())
}

/// Run one cockpit verb: resolve the station first, so every one of the nine
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
        "contacts" => "CONTACTS            what your platform holds: its opaque id, its class, \
                       bearing, range band, range and how old the plot is."
            .to_owned(),
        "lock" => {
            "LOCK <contact>      hold one of your contacts by the id CONTACTS prints.".to_owned()
        }
        "fire" => "FIRE [<weapon>]     shoot your lock; the cannon unless you name another weapon."
            .to_owned(),
        // 16b cut F.2 gave both of these more to say, and a help line that
        // undersells a surface is a pilot who never looks at it.
        "weaponstatus" => "WEAPONSTATUS        every weapon you carry: ready or cycling, its \
                           rounds, its own chance against your lock, and that lock's range band."
            .to_owned(),
        _ => "STATUS              power, speed, heading, position, the arena's tempo, and your \
              armour and internals section by section."
            .to_owned(),
    }
}

/// Register the cockpit on the **tacsim** module: eleven verbs and one gate.
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
    // trigger) cut T.3's whole check-out; `EXIT` writes the same in reverse. The
    // nine selectors stage one arena intent each and write nothing at all, but
    // they are registered on the same lane as their two neighbours rather than
    // split across two classifications for one subsystem.
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
            let line = refusal_line("fire", refusal, Some("cannon"), RefusalDetail::None);
            assert!(line.starts_with("FIRE refused. "), "{refusal:?}: {line}");
            let said = line.trim_start_matches("FIRE refused. ");
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
                "FIRE refused. {}",
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
        };
        assert_eq!(
            ready.render(),
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
            ..ready
        };
        assert_eq!(cycling.render(), "cannon  cycling (4 s)  39 of 40 round(s)");
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

    #[test]
    fn a_contact_id_parses_as_the_sixteen_digits_it_renders_as() {
        let id = ContactId(0x0123_4567_89ab_cdef);
        assert_eq!(id.to_string(), "0123456789abcdef");
        assert_eq!(parse_contact(&id.to_string()), Some(id));
        assert_eq!(parse_contact(" 1 "), Some(ContactId(1)));
        assert_eq!(parse_contact(""), None);
        assert_eq!(parse_contact("nope"), None);
        assert_eq!(parse_contact("00000000000000001"), None, "over sixteen");
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
        assert_eq!(COCKPIT_VERBS.len(), 9, "doc 16 section 3.7's nine");
    }
}
