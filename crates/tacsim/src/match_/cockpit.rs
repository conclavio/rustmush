//! **The cockpit: boarding, the pilot's nineteen verbs, and the frame they
//! read** (16b cut K.2; 16b §8 cut K, K2–K4; doc 16 §3.3 "Startup", "Stations
//! are on the tank", §3.6 "What a pilot sees", §3.7, §3.8 "Disconnects", §5, §6
//! "Deliberation").
//!
//! Cut K.1 built the lobby: a centre, slots, pods locked to pilots, and the
//! named `Boarding → Running` trigger waiting for somebody to report a seat.
//! This is the half that reports one, and everything the pilot can do once
//! seated.
//!
//! **Nineteen**: the seventeen cockpit selectors of [`COCKPIT_VERBS`], plus
//! [`BOARD_VERB`] and [`EXIT_VERB`], which are not selectors — they are how a
//! pilot gets into the cockpit and out of it. The table below is that list in
//! that order, and **16b cut D.2** made the three agree: the head said twelve in
//! one place and fourteen in another, and the table was missing `RANGE` and
//! `BEARING` altogether (16c `T-77`). **16b cut Y.1** added the thirteenth
//! selector, `ORDER` — see "Commanding the machines" below
//! [`crate::match_::cockpit::resolve_designator`]. **16b cut O.2** added the
//! fourteenth, `EMCON`: the jamming pod's switch (16c N12). **16b cut AP.3**
//! added the fifteenth, `WATCH`: the switch that pushes a line-mode pilot a
//! short block of their own picture whenever it would read differently (16c
//! N19) — see "The line-mode digest" at
//! [`crate::match_::cockpit::watch_lines`]. **17b cut M1a.4** added the
//! sixteenth, `DRIVE`: the tank's own drive-by-wire, steering for a named spot
//! or a cell on the machines' route-finding (doc 17 §4 D-UM-10; the word is
//! `DRIVE` because core's `goto` holds that name). **17b cut M5.4** added the
//! seventeenth, `MIND`: the hull's sealed mind read from the seat — what it
//! is, its profiles, who holds each channel — and `MIND PROFILE`, the seat's
//! switch to another sealed profile (doc 17 §12 "The `MIND` verb").
//!
//! ```text
//! BOARD                      climb into your pod and take the tank's cockpit
//! STARTUP                    bring power up (idempotent)
//! HEADING <0-359>            steer
//! SPEED <kph>                make way (negative is reverse)
//! VIEW [BRIEF]               the arena window, as a text map; BRIEF says it in sentences
//! CONTACTS                   what the platform holds
//! LOCK <contact>             hold one of them
//! RANGE [<contact>|<spot>]   how far away one of them plotted, or a named spot is
//! BEARING [<contact>|<spot>] which way one of them plotted, or a named spot is
//! SCAN [<contact>]           what is left of one of them, by section
//! FIRE [<weapon>]            shoot it
//! FIRE <weapon> WHEN <cond>  arm it to shoot your lock whenever every condition holds; FIRE <weapon|ALL> OFF clears it
//! WEAPONSTATUS               ammunition and readiness, and under an armed weapon its slot
//! EMCON [on|off]             the jamming pod's switch, if the tank carries one
//! STATUS                     callsign, power, speed, heading, scope, nearest spot, damage
//! ORDER <squad|ALL> <what>   command your side's machines (HOLD, ADVANCE, ENGAGE, WITHDRAW, SCREEN, SUPPORT; RELEASE)
//! WATCH [ON|OFF]             push your own line, guns and contacts as they change, for line mode
//! DRIVE <spot|x,y|HOLD|OFF>  drive to a named spot or a cell on the machine's own route-finding
//! MIND [PROFILE <name>|RULES] your tank's sealed mind, dormant while you fly; PROFILE switches its profile
//! EXIT                       climb out
//! ```
//!
//! **16b cut V.1** named the ground: the arena's authored key spots are on the
//! map payload, on `VIEW`'s window as a letter with its name in the legend, and
//! behind `RANGE`/`BEARING` as a second resolution arm — see "The named ground"
//! below [`crate::match_::cockpit::stale_clause`].
//!
//! # Boarding is four facts, in one order
//!
//! Doc 16 §3.3 says *stations are on the tank*, and §3.7 says the pod *is where
//! the body sits*. Those two sentences are the whole design, and they fix the
//! order [`crate::match_::cockpit::board`] does its work in:
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
//!    the account's single foreground focus. K6 currently leases only single-channel
//!    stations: this three-channel cockpit has no control lease. Focus, billet
//!    grants and arena epochs enforce its current single-station control. A lease
//!    per channel is deferred to the multi-station cut (16c T-54).
//!    The cockpit opens no second path and mints no grant of its own.
//! 4. **the centre** — [`crate::match_::slot_seated`], cut K.1's trigger input.
//!    When this seat was the last one the match was waiting for, the check-out
//!    fires inside this same command.
//!
//! [`crate::match_::cockpit::exit`] undoes 3, 2 and (during `Boarding`) 4, in that order, and sends
//! `Control { Released }` **first** — before the seat itself is released — so
//! that a pilot who types `FIRE` and `EXIT` in one burst has the epoch raised
//! under the queued shot and the arena purges it (doc 16 §3.1).
//!
//! # The verbs are contextual, and the gate is what makes them so
//!
//! There is no such thing as a verb that exists only in a cockpit: the command
//! table is global. So all nineteen are registered globally and
//! [`CockpitGate`] refuses each of them, with one line, wherever it does not
//! mean anything — `BOARD` outside a team room, `EXIT` outside a pod, and the
//! seventeen cockpit selectors to anyone who is not focused on a station of a
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
//! [`crate::match_::cockpit::status_frame`] is what a pulse sends: the numbers, the contacts (16b cut S.2),
//! the notices it owes and the weapons table. [`crate::match_::cockpit::view_frame`] is that same frame
//! **plus** the
//! arena window in the grid shape `mush_ansi::panel` already renders, which is
//! what makes `VIEW` a text map on a line client and a map panel on the web with
//! no per-feed code in either. The grid rides `VIEW` and not the pulse
//! deliberately: a line client would otherwise be sent twenty rows of map twice a
//! second.
//!
//! ## And a third, which is not a cockpit frame at all
//!
//! **16b cut Q.3** added [`crate::match_::cockpit::map_frame`]: the whole arena's terrain on its own feed
//! ([`crate::arena_bridge::MAP_FEED`]), carrying no unit of any kind. It is only
//! ever the answer to a **query**, never pushed, so a line client is never sent
//! it and a panel client asks for it once per match. A panel client that holds a
//! map and a cockpit frame composes them itself; [`crate::match_::cockpit::view_window`] is the same
//! composition done server-side for `VIEW`, and the fixtures under
//! `web/src/lib/fixtures/cockpit/` are the one set all three renderers are held
//! to (doc 16 §5).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, ChannelId, CmdError, CommandSpec, Ctx, ObjId, Registry, World};
use mush_proto::{OutputEvent, RichText};
use smol_str::SmolStr;

use crate::arena::catchup::{Catchup, MAX_CATCHUP_CONTACTS, SectionWords};
use crate::arena::condition::{ConditionTier, MAX_SCANNED_SECTIONS};
use crate::arena::conditions::{
    ASPECTS_ANY, Arming, FireCondition, HeldWhy, SlotTarget, aspect_in,
};
use crate::arena::damage;
use crate::arena::intents::MotorGoal;
use crate::arena::intents::{ControlState, IntentKind};
use crate::arena::motion::BlockedCause;
use crate::arena::perception::{Aspect, DriveOutcome, FrameNotice, NoticeKind, RefusalDetail};
use crate::arena::state::{ArenaTerrain, Channel, ContactId, Designator, KeySpot, Power};
use crate::arena::step::{Frame, FrameMind};
use crate::arena::weapons::FrameArming;
use crate::arena_bridge::{
    ArenaRegistry, CockpitFrame, FRAME_FEED, IntentRefusal, NOTICE_CATEGORY, NOTICE_SENTENCE_TAG,
    forward_intent, send_control,
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
///
/// **16b cut Y.1** added `order`, the thirteenth and the first that speaks to a
/// unit other than the pilot's own: doc 16 §6's orders from a pilot rather than
/// from the LLM seam.
///
/// **16b cut O.2** added `emcon`, the fourteenth: the jamming pod's switch (16c
/// N12), a discrete intent on the sensors channel.
///
/// **16b cut AG.1** gave `view` a `BRIEF` arm — the same window said in
/// sentences (16c N17) — and no fifteenth selector: a second word on a read
/// verb is not a verb.
///
/// **16b cut AP.3** added `watch`, the fifteenth (16c N19): `WATCH ON` asks the
/// bridge to push a line-mode pilot a short block of their own picture
/// whenever it would read differently ([`watch_lines`]). Appended, so every
/// selector before it keeps its place.
///
/// **17b cut M1a.4** added `drive`, the sixteenth (doc 17 §4 D-UM-10): a
/// continuous goal on the helm channel the arena's motor drives. Appended
/// after `watch`. Doc 17 calls it `GOTO`, and core's baseline `goto` (aliases
/// `go`, `move`) holds that name — the registry refuses a collision, so the
/// word is `DRIVE` (the M1a plan's departure (f)).
///
/// **17b cut M5.4** added `mind`, the seventeenth (doc 17 §12 "The `MIND`
/// verb"; the M5 design note's C-1: D-UM-10's "16 with `GOTO` and `MIND`"
/// read as 17, with `DRIVE` and `MIND`). Appended after `drive`. It shifts
/// a prefix and takes none: no other bare primary verb of the production
/// roster starts with `m` (`@`/`+` names and aliases never prefix-match), so
/// `m`, `mi` and `min` went from no command to `MIND`.
pub const COCKPIT_VERBS: [&str; 17] = [
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
    "order",
    "emcon",
    "watch",
    "drive",
    "mind",
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

/// How many cells of the arena window one `VIEW` carries, across.
///
/// **16b cut B.4** (`B-24`): half the 78 columns the cap was when the window was sized
/// (`mush_ansi::panel::GRID_MAX_COLUMNS`), because each cell is drawn two columns
/// wide so the ground is square — a terminal cell is about twice as tall as it is
/// wide, and at one column a cell the window drew the ground four times wider than
/// tall. The window says so itself (`cell_columns: 2`, [`view_window`]). It is a
/// close-in view: `CONTACTS` and the beyond-window line
/// ([`beyond_window_line`]) cover the rest of the sight.
pub const VIEW_COLUMNS: u16 = 39;

/// As [`VIEW_COLUMNS`], up the window: the twenty rows a line client printed
/// (`mush_ansi::panel::GRID_MAX_ROWS`, 20 when the window was sized), one row a cell.
pub const VIEW_ROWS: u16 = 20;

/// **How many printed columns a line client draws each window cell as** (16b cut
/// B.4, `B-24`): the `cell_columns` key [`view_window`] writes and the grid
/// renderer reads. Only the line client doubles; the TUI's map pane and the web
/// canvas draw one cell per cell.
pub const VIEW_CELL_COLUMNS: u16 = 2;

/// **The key-spot alphabet and its one reader** (16b cut V.1), defined in the
/// arena library since 16b cut O.1 — a Hold match's objective carries its
/// spot's letter, and the arena reaches into no module outside itself for it —
/// and re-exported here, where every map, legend and `RANGE` reads it. One
/// alphabet, one place: see [`crate::arena::state::SPOT_LETTERS`].
pub use crate::arena::state::{SPOT_LETTERS, spot_letter};

/// **The step a spot's range is rounded to**, metres (16b cut V.1). Ground does
/// not move and is not a plot, so it earns neither the scope's coarse step nor
/// the lock's age-bound one: ten metres, a cell.
pub const SPOT_RANGE_STEP_M: u64 = 10;

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

/// **What `RANGE` and `BEARING` answer about a named spot before the arena has
/// told the pilot anything** (16b cut V.1).
///
/// The spot is real — the ground is surveyed and every pilot knows its names —
/// and what is missing is the pilot's own position, so this is not
/// [`NO_SUCH_CONTACT`]: that sentence is about what the platform holds, and a
/// spot is not held. A **contact** word with no frame still answers
/// [`NO_SUCH_CONTACT`], exactly as it did.
pub const NO_FRAME_YET: &str = "The panel has told you nothing yet.";

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
/// Byte for byte the sentence `refusal_line` says for
/// [`crate::arena::intents::Refusal::MotionDisabled`], and the pin test below
/// holds the two together, for the reason [`POWER_NOT_ONLINE`] is pinned to the
/// arena's word.
pub const IMMOBILE: &str = "Your tank cannot move.";

/// **The line `EMCON` answers for a tank that carries no jamming pod** (16b cut
/// O.2, 16c N12), read off [`Frame::ecm`] — and, after the `EMCON refused.`
/// prefix, the sentence the arena's own
/// [`crate::arena::intents::Refusal::NoJammer`] renders.
pub const NO_JAMMER: &str = "This tank carries no jamming pod.";

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
    /// **`BOARD` too soon after a drop** (16b cut Y.3; 16c `N4`): the tank went
    /// to its machine — a unit mind (the reflex controller until 17b cut M4.3)
    /// — less than `[tacsim.match].reclaim_min_ms` ago.
    #[error("{unit} is still being handed over; try again in {secs} second(s).")]
    HandingOver {
        /// The pod's tank, by name — the pilot's own.
        unit: String,
        /// Seconds left, rounded up.
        secs: i64,
    },
    /// **`BOARD` after an `EXIT` in a running match** (16b cut Y.3; doc 16 §11's
    /// N.1 amendment): a deliberate leave is one-way, whatever came before it.
    #[error("You left {unit} for the rest of the match; it cannot be taken back.")]
    LeftForGood {
        /// The pod's tank, by name — the pilot's own.
        unit: String,
    },
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
    /// A named spot, asked about before the arena has sent the pilot a frame
    /// (16b cut V.1).
    #[error("{NO_FRAME_YET}")]
    NoFrameYet,
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
    /// **A squad the pilot may not order** (16b cut Y.1): the other side's, one
    /// that does not exist, or the own side's beyond the pilot's authority — one
    /// line for all three.
    #[error("{NO_SUCH_SQUAD}")]
    NoSuchSquad,
    /// **`ORDER ALL` with nothing to order** (16b cut Y.1).
    #[error("{NO_SQUAD_TO_ORDER}")]
    NoSquadToOrder,
    /// **A `SUPPORT` callsign no tank of the pilot's side flies** (16b cut Z.2):
    /// the other side's, a provisional, a word that is no callsign, a slot
    /// nobody flies — one line for all four.
    #[error("{NO_SUCH_FRIENDLY}")]
    NoSuchFriendly,
    /// **One more squad would be past the span** (16b cut Y.2; doc 16 §2 W8,
    /// `[tacsim.command].max_span`): the pilot already holds live orders on
    /// `held` squads the order does not name, and those alone fill the span —
    /// so no squad can be ordered, one or `ALL`, until one runs out.
    ///
    /// The span holds until the world's watch on each of those squads runs out
    /// ([`HUMAN_ORDER_TTL_MS`]) or the pilot hands one back: the world never
    /// learns that the arena ended an order early (16b cut AA.1, `B-41`), and
    /// since **16b cut Z.2** (16c Q44) `ORDER <squad> RELEASE` drops the watch
    /// the pilot's own order put on a squad. So the sentence names the two
    /// things that free the span, and says nothing of re-ordering one of them
    /// — a replacement frees nothing.
    #[error("You hold orders on {held} squad(s) already; wait for one to run out, or RELEASE one.")]
    SpanFull {
        /// Live orders on squads other than the one named.
        held: usize,
    },
    /// **`ORDER ALL` would be past the span** (16b cut Y.2): the squads the
    /// pilot holds besides the ones `ALL` names, plus those, is more than
    /// `span`, while the ones it does not name leave room for one more. The
    /// grammar takes one squad or `ALL`, and one squad at a time does pass, so
    /// the sentence says to order them one at a time.
    #[error(
        "ORDER ALL would put {would} squads under you; your span is {span}. Order them one at \
         a time."
    )]
    SpanAll {
        /// The squads the pilot would hold, the ones `ALL` names included.
        would: usize,
        /// `[tacsim.command].max_span`.
        span: u32,
    },
    /// **Past the order rate** (16b cut Y.2; doc 16 §2 W8's human trigger,
    /// `[tacsim.command].orders_per_minute`): `n` orders in the last sixty
    /// world seconds.
    #[error("You have given {n} orders in the last minute; wait.")]
    RateFull {
        /// Orders given in the window.
        n: u32,
    },
    /// The unit carries no weapon by that name.
    #[error("That unit carries no weapon called `{0}`.")]
    NoSuchWeapon(String),
    /// **A bare `FIRE` in a unit that carries no weapon at all** (16b cut AE.3):
    /// a drone, or a tank whose every weapon has been shot off.
    ///
    /// The same sentence `WEAPONSTATUS` prints for an empty list. Before this
    /// variant a bare `FIRE` answered [`CockpitError::NoSuchWeapon`] with
    /// [`DEFAULT_WEAPON`] in it — "That unit carries no weapon called
    /// `cannon`." — naming a gun the pilot never mentioned. A *named* weapon on
    /// an empty list keeps `NoSuchWeapon`, because there the pilot did name one.
    #[error("This unit carries no weapon.")]
    NoWeapon,
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
    /// **`EMCON` on a tank that carries no jamming pod** (16b cut O.2), read off
    /// [`Frame::ecm`]; byte for byte what `refusal_line` says for
    /// [`crate::arena::intents::Refusal::NoJammer`] after its prefix.
    #[error("{NO_JAMMER}")]
    NoJammer,
    /// **A `FIRE` the cockpit refused locally, in the arena's own sentence** (16b
    /// cut D.2, `B-16`).
    ///
    /// Every `FIRE` refusal reads `FIRE <weapon> refused. <reason>`, whichever
    /// half answered: the cockpit's pre-check against the last frame, or the
    /// arena's own a pulse later. Both compose it through [`order_refused`], so
    /// the early answer and the late one are one string — which is exactly the
    /// contract cut F.2 wrote `refusal_line` for, read once more now that the
    /// weapon is in the sentence.
    #[error("{0}")]
    Refused(String),
    /// **A condition under the match's Compact floor** (17b cut M1b.1; doc 17
    /// §5, D-UM-17 as amended): the ONE home of the sentence, which the arena's
    /// landing refusal (`refusal_line` over `RefusalDetail::Floor`) and 17b cut
    /// M1b.4's typed pre-check (through `fire_refused`) both print, so the two
    /// halves are one string.
    #[error("The Compact floor is {floor_pct}%; a condition may not ask for less.")]
    Unlawful {
        /// The floor, whole per cent (the floor is validated in steps of ten
        /// per mille).
        floor_pct: u32,
    },
    /// **An arming past the hull's slot count** (17b cut M1b.1): the ONE home
    /// of the count's sentence, `<weapon>` literal.
    #[error("Every rule slot is occupied; MIND RULE <slot> OFF frees one.")]
    NoSlot,
    /// **A condition this core does not take** (17b cut M1b.4; doc 17 §5,
    /// D-UM-18's slot-fitting subset): the ONE said-half for every rule
    /// outside the eight terms [`parse_arming`] compiles — another word, an
    /// `OR`, a fifth condition, a malformed or missing number, an empty `WHEN`,
    /// a term repeated with another value. Printed through `fire_refused`, so
    /// it reads `FIRE cannon refused. That condition is …`. The rest of the
    /// rule language is the rule-language cut's, after 17b cut M5.7's gate
    /// (the owner's OQ-4; re-pointed by 17b cut M5.4).
    #[error(
        "That condition is not one this core takes; help cockpit lists the supported native rules."
    )]
    NotThisCore,
}

// ---------------------------------------------------------------------------
// Reading the cockpit
// ---------------------------------------------------------------------------

/// The channels a cockpit seat grants (doc 16 §3.7: `helm`, `sensors`, `fire`).
///
/// Built from the arena's own channel vocabulary, so the seat and the orders
/// cannot name two different sets. **17b cut M1a.5** (doc 17 §4, D-UM-47): from
/// [`crate::arena::state::COCKPIT_CHANNEL_IDS`], the one table where the arena's
/// closed `Channel` and `mush-sim`'s open [`ChannelId`] meet by wire word.
#[must_use]
pub fn cockpit_channels() -> BTreeSet<ChannelId> {
    crate::arena::state::COCKPIT_CHANNEL_IDS
        .iter()
        .filter_map(|id| ChannelId::new(*id))
        .collect()
}

/// The station id, as a typed [`mush_sim::StationId`].
fn station_id() -> Option<mush_sim::StationId> {
    mush_sim::StationId::new(STATION)
}

/// **The cockpit seat one line is typed from** (16b cut U.4, 16c `T-53`): who
/// holds it, on which unit, at which station — resolved once, by [`flying`],
/// and handed to every cockpit verb.
///
/// `driver` is [`mush_sim::focus::driving_actor`] of the typist: the typist for
/// an ordinary pilot, and the NPC for a human driving an NPC's seat through a
/// proxy. It is the subject of the focus and of every
/// [`crate::arena_bridge::ArenaRegistry`] key — the last frame, the power hint,
/// the pilot binding — so a verb that reads the cockpit reads it as `driver`.
/// Every reply is still addressed to the typist (`ctx.actor`), who is the one
/// reading the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    /// The character whose seat this is: the focus's subject.
    pub driver: ObjId,
    /// The unit the seat is on — one an arena holds.
    pub unit: ObjId,
    /// The station the seat is on.
    pub station: mush_sim::StationId,
}

/// **The seat an actor is flying from** — the cockpit's one context test.
///
/// The actor's focused station (through the proxy ingress, so a driver of an NPC
/// gunner resolves to the NPC), narrowed to a unit an arena actually holds
/// custody of. A pilot who has boarded but whose match has not started yet is
/// deliberately **not** in a cockpit: there is no arena to order.
///
/// **16b cut U.4 (16c `T-53`)** made the answer the whole [`Seat`], the driver
/// included. The unit was resolved through the proxy driver while every read of
/// the cockpit was keyed on the typist, so a human flying an NPC's seat would
/// have read their own (empty) mirror about the NPC's tank; now the one
/// resolution names both, and the verbs take it.
#[must_use]
pub fn flying(w: &World, actor: ObjId) -> Option<Seat> {
    let driver = mush_sim::focus::driving_actor(w, actor);
    let focus = w
        .module_state::<mush_sim::FocusState>()?
        .focus_of_actor(driver)?;
    let (unit, station) = (focus.unit, focus.station.clone());
    mush_sim::deployed::is_deployed(w, unit).then_some(Seat {
        driver,
        unit,
        station,
    })
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
pub(crate) fn last_frame(w: &World, pilot: ObjId) -> Option<CockpitFrame> {
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
pub(crate) fn require_alive(w: &World, pilot: ObjId) -> Result<(), CockpitError> {
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

/// **The view** (17b cut M1a.2; doc 17 §3 "One formatter", D-UM-4): the
/// contact rows, the range rounding and its memory, the scope word, the band
/// word, the lock reads, the weapon a bare `FIRE` fires and the four `VIEW`
/// glyphs moved to [`crate::arena::view`], the arena library, so the row a
/// pilot reads and the row a mind reads are one function. Re-exported here so
/// every path that named them in this module — the bridge, `CONTACTS`, the
/// panel rows, the bot's contract test — names the same function; the
/// sentences built on them ([`contact_sentence`], [`lock_line`],
/// [`notice_line`], [`brief_lines`], [`watch_lines`]) stay in this module.
pub use crate::arena::view::{
    ContactRow, FRIEND_GLYPH, HOSTILE_GLYPH, OddsCell, UNKNOWN_GLYPH, YOU_GLYPH, band_bounds_m,
    contact_band, fired_by_default, frame_contacts, frame_contacts_held, frame_lock, held_range_m,
    lock_designator, own_scope, range_m_rounded, settle_ranges,
};
// 17b cut M1a.2: the two crate-private readers the notice line and `BEARING`
// still call here.
pub(crate) use crate::arena::view::{class_word, relative_mdeg};

/// **The range steps and the lock's own step** (16b cut F.3, Q.1, D.4), which
/// live in the arena library since **16b cut U.2** (16c `T-30`): the arena's
/// `Frame::same_as` has to know when a lock's published metres would move, and
/// the library may not reach up into this module. They are re-exported here so
/// every caller — `RANGE`, `lock_range_m`, the contact rows — still names them
/// where it always did, and `T-20`'s *one function* rule holds: there is one
/// [`lock_range_step_m`], and the frame comparison calls the same one.
pub use crate::arena::perception::{
    CONTACT_RANGE_STEP_M, LOCK_RANGE_STEP_M, PLOT_DRIFT_M_PER_S, lock_range_step_m,
};

mod frames;
mod notices;
mod orders;
mod sensors_spots;
mod weapons_fire;

pub(crate) use self::frames::terrain_rows;
#[allow(unused_imports)]
use self::frames::*;
pub use self::frames::{
    ELEV_ENCODING, FRAME_SCHEMA, FRAME_VERSION, MAP_MAX_BYTES, MAP_SCHEMA, MAP_SHAPE, MAP_VERSION,
    MAP_WINDOW_SIDES, RETAIN_LATEST, beyond_window_line, cockpit_cleared, contact_health,
    contact_markers, elev_field, elev_field_window, map_frame, map_frame_for_focus,
    map_frame_window, map_frame_window_bounded, map_unseated, mind_json, status_frame,
    status_frame_held, view_frame, view_frame_held, view_window, window_centered_on,
};
#[allow(unused_imports)]
use self::notices::*;
pub use self::notices::{
    DRIVE_ARRIVED, DRIVE_HOLDING, DRIVE_NO_ROUTE, DRIVE_NOT_A_GOAL, DRIVE_OFF_WINDOW,
    DRIVE_RELEASED, DRIVE_USAGE, EjectReport, MIND_CHANNELS_ARMED, MIND_CHANNELS_UNARMED,
    MIND_NO_TRACE, MIND_RULES_LINE, MIND_USAGE, NO_SEAL, OWN_DESTROYED, OWN_DISABLED,
    PROFILE_REFUSED, RECEIVER_CLEAR, RECEIVER_DEGRADED, WATCH_IS_OFF, WATCH_IS_ON, WATCH_OFF,
    WATCH_USAGE, WATCHING, aged_weapons, board, catchup_lines, emcon_status_line, exit,
    frame_weapons, lock_line, mind_head_line, mind_lines, mind_profile_line, mind_rules_line,
    next_switch_s, notice_line, notice_lines, order_refused, profile_ack, profile_switch,
    receiver_line,
};
pub(crate) use self::notices::{
    consequence_words, eject_all, eject_all_at_boot, eject_pilot, hand_seat_back, miss_words,
    tell_hatch_open,
};
#[allow(unused_imports)]
use self::orders::*;
pub(crate) use self::orders::{Command, command_of};
pub use self::orders::{
    HUMAN_ORDER_TTL_MS, NO_SQUAD_TO_ORDER, NO_SUCH_FRIENDLY, NO_SUCH_SQUAD, ORDER_USAGE,
    ObjectiveWords, OrderWords, SquadWord, order_ack, pair, parse_order,
};
#[allow(unused_imports)]
use self::sensors_spots::*;
pub use self::sensors_spots::{
    BRIEF_BEARINGS, DIAGONAL_PER_MILLE, GroundRun, RayEnd, WATCH_COLUMNS, bearing_line,
    brief_header, brief_lines, brief_step_m, class_prose, contact_sentence, ground_line,
    ground_ray, in_window_spots_line, nearest_spot, nearest_spot_line, off_window_spots_line,
    range_line, ray_cm, resolve_spot, scan_lines, spot_bearing_line, spot_bearing_mdeg,
    spot_markers, spot_range_cm, spot_range_line, stale_clause, watch_lines,
};
pub(crate) use self::sensors_spots::{pass_ms, spots_of};
#[allow(unused_imports)]
use self::weapons_fire::*;
pub use self::weapons_fire::{
    ArmingEcho, BRIEF_END_LINE, CLEARED_ALL, MAX_ARMING_RANGE_M, MAX_ARMING_TERMS,
    NO_CONTACTS_IN_WINDOW, OUT_OF_REACH, VIEW_USAGE, WEAPONS_FREE, WEAPONS_HOLD, WEAPONS_HOLD_LINE,
    WeaponRow, arming_json, arming_reply, arming_target_word, arming_words, on_range, parse_arming,
    range_sentence, resolve_designator, section_lines, weapon_initials, weapon_short_of,
    weapons_of,
};

// ---------------------------------------------------------------------------
// The gate and the dispatch
// ---------------------------------------------------------------------------

/// **The contextual gate**: the nineteen verbs exist everywhere and mean
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
        // content latch up to fifteen times — three rooms and then all twelve pods.
        // A gate asked about `look` must cost what it always cost: two string
        // compares and a seventeen-element scan, and no world read at all.
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
/// billet is on a **roster tank of that centre** — the objects the twelve pods
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
            held == station && !mush_sim::deployed::is_custodied(w, unit) && roster.contains(&unit)
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

/// Run one cockpit verb: resolve the station first, so every one of the seventeen
/// answers the same way to somebody who is not flying.
///
/// **16b cut U.4 (16c `T-53`)**: the seat is resolved **once**, here, and every
/// verb takes it — each reads the cockpit as [`Seat::driver`] and answers
/// `ctx.actor`.
fn cockpit_command(ctx: &mut Ctx<'_>, verb: &'static str) -> Result<(), CmdError> {
    if verb == "order" {
        let raw = ctx.args.raw().trim().to_owned();
        if let Some(result) = crate::detachment::sortie::field_order(ctx, &raw) {
            return result.map_err(CmdError::msg);
        }
    }
    let Some(seat) = flying(ctx.w, ctx.actor) else {
        return Err(CmdError::msg(waiting_to_start(ctx.w, ctx.actor)));
    };
    let rest = ctx.args.raw().trim().to_owned();
    let outcome = match verb {
        "startup" => startup(ctx, &seat),
        "heading" => heading(ctx, &seat, &rest),
        "speed" => speed(ctx, &seat, &rest),
        // 16b cut AG.1: and its one second word, `BRIEF`.
        "view" => view(ctx, &seat, &rest),
        "contacts" => contacts(ctx, &seat),
        "lock" => lock(ctx, &seat, &rest),
        "fire" => fire(ctx, &seat, &rest),
        "weaponstatus" => weapon_status(ctx, &seat),
        "status" => status(ctx, &seat),
        // 16b cut Q.1: the two read verbs, off the same plot every other surface
        // reads.
        "range" => range(ctx, &seat, &rest),
        "bearing" => bearing(ctx, &seat, &rest),
        // 16b cut Q.2: what is left of a contact, off the reading the arena put
        // on it.
        "scan" => scan(ctx, &seat, &rest),
        // 16b cut Y.1: the side's machines, on the six objectives and the release (16b cut Z.2).
        "order" => order(ctx, &seat, &rest),
        // 16b cut O.2: the jamming pod's switch.
        "emcon" => emcon(ctx, &seat, &rest),
        // 16b cut AP.3: the line-mode digest's switch.
        "watch" => watch(ctx, &seat, &rest),
        // 17b cut M1a.4: the motor's goal.
        "drive" => drive(ctx, &seat, &rest),
        // 17b cut M5.4: the hull's sealed mind, off the held keyframe.
        "mind" => mind(ctx, &seat, &rest),
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
        // 16b cut AG.1: and the same window in sentences, the help's own row.
        "view" => "VIEW [BRIEF]        the arena window around your platform, as a map; BRIEF \
                   says the same window in sentences."
            .to_owned(),
        // 16b cut Q.1: the id is a designator now, and the help says so — a pilot
        // who reads `opaque id` will go looking for sixteen digits that are not
        // printed anywhere.
        "contacts" => "CONTACTS            what your platform holds: its designator, its class, \
                       bearing, range band, range and how old the plot is."
            .to_owned(),
        "lock" => "LOCK <contact>      hold one of your contacts by the designator CONTACTS \
                   prints (BA, or X2 before it is identified). LOCK nearest|best_odds|weakest|flank|newest \
                   WHEN <cond> arms a Sensors rule over identified hostiles; see help cockpit."
            .to_owned(),
        // 16b cut V.1: and a named spot of the ground, by its name or its letter.
        "range" => "RANGE [<contact>]   how far away a contact plotted; your lock if you name \
                    none. A locked contact reads to the nearest ten metres. A named spot of the \
                    ground works too, by its name or its map letter: RANGE The Ford."
            .to_owned(),
        "bearing" => "BEARING [<contact>] which way a contact plotted, absolute and off your own \
                      nose; your lock if you name none. A named spot works too: BEARING a."
            .to_owned(),
        // 16b cut Q.2: and the help says what earns a reading, because a pilot
        // who reads `No reading yet` needs to know the answer is to drive closer.
        "scan" => "SCAN [<contact>]    what is left of a contact, section by section, as green, \
                   yellow, red or destroyed; your lock if you name none. Only for a contact you \
                   have identified and had a close look at, and never an exact number."
            .to_owned(),
        // 16b cut AE.2: the scout car's main gun is its autocannon, so "the
        // cannon" is no longer every machine's bare `FIRE` — and the laser tank
        // carries none, so its bare `FIRE` is the beam.
        // 17b cut M1b.4 (doc 17 §5): and the arming, which is FIRE's grammar
        // grown rather than a verb of its own.
        "fire" => "FIRE [<weapon>]     shoot your lock with your machine's main gun — the cannon \
                   on a tank that carries one — unless you name another weapon. FIRE <weapon> \
                   WHEN <cond> [AND <cond>]... arms the weapon: it fires your lock whenever every \
                   condition holds, and tells you each time. FIRE <weapon|ALL> HOLD WHEN <cond> \
                   vetoes automatic fire; FIRE <weapon|ALL> OFF clears fire and hold rules. \
                   FIRE ALL fires every ready weapon that reaches your lock, once each. \
                   FIRE HOLD|FREE is the master switch: under HOLD your armed rules stand but \
                   none fires until FIRE FREE (free at the start). \
                   See help cockpit, \"Arming a weapon\"."
            .to_owned(),
        // 16b cut Y.1: who may order what, in the help, because the refusal for
        // a squad a pilot may not order is deliberately one bare sentence.
        // 16b cut Z.2: the two squad roles and the release. 17b cut M5.6 (the
        // M5.3 close's D9 ruling): "back to their own doctrine" until then —
        // a released squad flies each hull's sealed mind, one word for every
        // mind, as the release acknowledgement says it.
        "order" => "ORDER <squad|ALL> <objective>  command your side's machines: HOLD, ADVANCE \
                    <spot>, ENGAGE [<contact>], WITHDRAW, SCREEN <bearing> or SUPPORT \
                    <callsign>, with HOLD FIRE at the end to forbid shooting; RELEASE hands them \
                    back to their own minds. A side's lead orders any squad of the side; anyone \
                    else their pod pair's tank."
            .to_owned(),
        // 16b cut F.2 gave both of these more to say, and a help line that
        // undersells a surface is a pilot who never looks at it.
        "weaponstatus" => "WEAPONSTATUS        every weapon you carry: ready or cycling, its \
                           rounds, its own chance against your lock, and that lock's range band \
                           — followed by the complete ordered native rules."
            .to_owned(),
        // 16b cut O.2 (16c N12): what the switch does to whom, because the
        // cost of each position is the whole reason to type it.
        "emcon" => "EMCON [on|off]      switch your jamming pod, if you carry one: on jams every \
                    sight inside its bubble, yours among them; off clears your own sight and \
                    hides you from nothing. On at check-out."
            .to_owned(),
        // 16b cut AP.3 (16c N19): who it is for, because a pilot on the TUI or
        // the web already has the screen it says in three lines.
        "watch" => "WATCH [ON|OFF]      push your own line, your guns and your contacts as a \
                    short block whenever it would read differently — for a line-mode pilot; OFF \
                    stops it; bare WATCH says which."
            .to_owned(),
        // 17b cut M1a.4 (doc 17 §4 D-UM-10): what it drives to, and the two
        // ways to have the helm back.
        "drive" => "DRIVE <spot|x,y>    drive to a named spot of the ground, by its name or its \
                    map letter, or to an arena cell, on the machine's own route-finding; HEADING \
                    or SPEED takes the helm back. DRIVE HOLD stops and holds here; DRIVE OFF \
                    lets go."
            .to_owned(),
        // 17b cut M5.4 (doc 17 §12 "The `MIND` verb"; OQ-8): what the block
        // says, that the mind is dormant while the pilot flies, and the
        // switch's one limit.
        "mind" => "MIND [PROFILE <name>|RULES]  your tank's sealed mind: what it is, its \
                   profiles and the one it will fly when you leave the seat, and who holds each \
                   channel; it is dormant while you fly. MIND PROFILE <name> switches to another \
                   sealed profile, once in ten seconds. MIND RULES [slot] reads live pilot rules; \
                   MIND RULE <slot> <rule or OFF> replaces or removes one."
            .to_owned(),
        // 16b cut D.2 (`T-77`): the callsign is `STATUS`'s first line and the
        // help did not mention it, so a pilot asked "who is RA?" had nowhere to
        // be sent; and cut D.1 replaced the arena's global tempo on the third
        // line with this platform's own scope.
        _ => "STATUS              your callsign, power, speed, heading, position, the named spot \
              nearest you, what your own scope holds, your pod's switch if you carry one, and \
              your armour and internals section by section."
            .to_owned(),
    }
}

/// Register the cockpit on the **tacsim** module: nineteen verbs and one gate.
///
/// The state the verbs read is [`ArenaRegistry`]'s and [`crate::match_::MatchBook`]'s, both
/// registered by [`crate::arena_bridge::register`], so nothing here declares a
/// slot.
///
/// # Errors
///
/// [`ModError`] if one of the verb names is already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    // `BOARD` writes a station document, a placement, a billet and (at the
    // trigger) cut T.3's whole check-out; `EXIT` writes the same in reverse. Of
    // the seventeen selectors, seven stage at most one arena intent each (cut
    // O.2's `EMCON` among them, which also stamps a bridge hint, and 17b cut
    // M1a.4's `DRIVE`), cut Y.1's `ORDER` stages one per unit it fans out to,
    // seven — cut Q.1's `RANGE` and `BEARING` and cut Q.2's `SCAN` among them —
    // only read the last frame, cut AP.3's `WATCH` sets or clears one RAM
    // flag on the bridge's registry, and 17b cut M5.4's `MIND` reads the last
    // frame, its `PROFILE` form staging one arena intent on the helm;
    // none of the seventeen writes a document.
    // They are
    // registered on the same
    // lane as their two neighbours rather than split across two classifications
    // for one subsystem.
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: BOARD_VERB.into(),
            shape: ArgShape::Raw,
            // 16b cut Y.3: and the reclaim. The reconnect line tells a dropped
            // pilot "BOARD takes it back", so the help they read next says the
            // same (doc 07 §1.3).
            help: "BOARD               climb into the pod your simulator slot holds and take its \
                   tank's cockpit. In a team room, once the match is boarding; EXIT gets you out \
                   again. In a running match, a pilot whose line dropped types BOARD again, from \
                   the pod or the team room, to take the tank back where it stands."
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
            // Cut Y.3 narrowed that to this verb: a dropped line can be
            // reclaimed, an `EXIT` cannot, even after a drop. 17b cut M5.7: `the
            // tank goes to its autopilot` until then — it is the tank's sealed
            // mind that flies it, in `help simulator`'s words.
            help: "EXIT               climb out of the pod you are in: the seat, its control \
                   lease and your focus are released, and a match still boarding stops waiting \
                   for you. Once the match is RUNNING this is final: your tank's mind flies it \
                   for the rest of the match and you cannot board again."
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
    crate::detachment::commands::register(reg)?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command_gate(Arc::new(CockpitGate));
    Ok(())
}

#[cfg(test)]
mod tests;
