//! **The cockpit: boarding, the pilot's eighteen verbs, and the frame they
//! read** (16b cut K.2; 16b §8 cut K, K2–K4; doc 16 §3.3 "Startup", "Stations
//! are on the tank", §3.6 "What a pilot sees", §3.7, §3.8 "Disconnects", §5, §6
//! "Deliberation").
//!
//! Cut K.1 built the lobby: a centre, slots, pods locked to pilots, and the
//! named `Boarding → Running` trigger waiting for somebody to report a seat.
//! This is the half that reports one, and everything the pilot can do once
//! seated.
//!
//! **Eighteen**: the sixteen cockpit selectors of [`COCKPIT_VERBS`], plus
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
//! `DRIVE` because core's `goto` holds that name).
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
//! table is global. So all eighteen are registered globally and
//! [`CockpitGate`] refuses each of them, with one line, wherever it does not
//! mean anything — `BOARD` outside a team room, `EXIT` outside a pod, and the
//! sixteen cockpit selectors to anyone who is not focused on a station of a
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
use crate::arena::conditions::{
    ASPECTS_ANY, Arming, FireCondition, HeldWhy, SlotTarget, aspect_in, aspect_mask,
};
use crate::arena::damage;
use crate::arena::intents::MotorGoal;
use crate::arena::intents::{ControlState, IntentKind};
use crate::arena::motion::BlockedCause;
use crate::arena::perception::{Aspect, DriveOutcome, FrameNotice, NoticeKind, RefusalDetail};
use crate::arena::state::{ArenaTerrain, Channel, ContactId, Designator, KeySpot, Power};
use crate::arena::step::Frame;
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
pub const COCKPIT_VERBS: [&str; 16] = [
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
/// **16b cut B.4** (`B-24`): half the seventy-eight columns a line client prints
/// (`mush_ansi::panel::GRID_MAX_COLUMNS`), because each cell is drawn two columns
/// wide so the ground is square — a terminal cell is about twice as tall as it is
/// wide, and at one column a cell the window drew the ground four times wider than
/// tall. The window says so itself (`cell_columns: 2`, [`view_window`]). It is a
/// close-in view: `CONTACTS` and the beyond-window line
/// ([`beyond_window_line`]) cover the rest of the sight.
pub const VIEW_COLUMNS: u16 = 39;

/// As [`VIEW_COLUMNS`], up the window: the twenty rows a line client prints
/// (`mush_ansi::panel::GRID_MAX_ROWS`), one row a cell.
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
    /// to the reflex controller less than `[tacsim.match].reclaim_min_ms` ago.
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
    #[error("Every slot is armed; FIRE <weapon> OFF frees one.")]
    NoSlot,
    /// **A condition this core does not take** (17b cut M1b.4; doc 17 §5,
    /// D-UM-18's slot-fitting subset): the ONE said-half for every rule
    /// outside the eight terms [`parse_arming`] compiles — another word, an
    /// `OR`, a fifth condition, a malformed or missing number, an empty `WHEN`,
    /// a term repeated with another value. Printed through `fire_refused`, so
    /// it reads `FIRE cannon refused. That condition is …`. The rest of the
    /// rule language arrives with the mind's own card (17b cut M5).
    #[error("That condition is not one this core takes; help cockpit lists the eight it does.")]
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
        // **16b cut AE.2**: a miss on a target the platform no longer holds
        // carries no reason (`Arena::stage_pilot_notice` stages it empty), so it
        // is the one word: where the target went and what stood round it are
        // not the gunner's to know once the contact is gone. `Hit.` is its twin.
        FrameNotice::Miss { contact, reason } => match contact {
            Some(contact) => format!("Miss on {}: {}.", contact.designator, miss_words(reason)),
            None if reason.is_empty() => "Miss.".to_owned(),
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
        // **16b cut O.2** (16c N12): the pilot's own pod, and what the switch
        // costs said in the line — on, the carrier's own sight is inside the
        // bubble with everybody else's; off, the sight clears and the bubble
        // hides nobody. The radius is the pod's own; nothing here names any
        // sight it reaches.
        FrameNotice::Emcon { on: true, radius_m } => format!(
            "EMCON on: the pod radiates; every sight inside {radius_m} m is jammed, your own \
             among them."
        ),
        FrameNotice::Emcon { on: false, .. } => {
            "EMCON off: the pod is silent; your own sight clears, and nothing hides you.".to_owned()
        }
        // **17b cut M1a.4** (doc 17 §4 D-UM-10): the motor's two answers to the
        // pilot's own `DRIVE`, said once each. Nothing about the route, the cell
        // or any other hull.
        FrameNotice::Drive {
            outcome: DriveOutcome::Arrived,
        } => DRIVE_ARRIVED.to_owned(),
        FrameNotice::Drive {
            outcome: DriveOutcome::NoRoute,
        } => DRIVE_NO_ROUTE.to_owned(),
        // **17b cut M1b.1** (doc 17 §5: *every slot shot is announced once*;
        // 17b cut M1b.4 stages it): the pilot's own slot and weapon, the lock
        // its platform holds by its designator (with the friendly mark by the
        // existing rule), and the chance and the side word that row earned.
        FrameNotice::SlotFired {
            slot,
            weapon,
            contact,
            p_permille,
            aspect,
        } => {
            let gun = capitalised(weapon);
            let percent = p_permille / 10;
            let side = aspect.side_word();
            match contact {
                Some(contact) => format!(
                    "{gun} fires at {}{} by S{slot} ({percent}%, {side}).",
                    contact.designator,
                    friendly_mark(frame, contact.id)
                ),
                None => format!("{gun} fires by S{slot} ({percent}%, {side})."),
            }
        }
        // **17b cut M1b.1** (doc 17 §5: *a held shot said at most once per
        // contact per 10 s*; 17b cut M1b.4 stages it): the slot, the weapon and
        // the term that failed.
        FrameNotice::SlotHeld { slot, weapon, why } => {
            format!("S{slot} held the {weapon}: {}.", held_words(why))
        }
    }
}

/// **A weapon id with its first letter upper-cased** (17b cut M1b.1; 17b's
/// Q-M2): `Cannon`, `Machine_gun` — the slot shot's opening word.
fn capitalised(weapon: &str) -> String {
    let mut letters = weapon.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// **Why a slot held, in plain words** (17b cut M1b.1; doc 17 §5's held
/// sentence): `54% under your 60%`, `BA shows its front`, `BA at 1200 m,
/// outside 200 to 600` (`under {max}` when the slot has no nearest, `past
/// {min}` when it has no furthest), `8 rounds, keeps 8`.
fn held_words(why: &HeldWhy) -> String {
    match why {
        HeldWhy::Odds { permille, floor } => {
            format!("{}% under your {}%", permille / 10, floor / 10)
        }
        HeldWhy::Aspect { contact, aspect } => {
            format!("{} shows its {}", contact.designator, aspect.side_word())
        }
        HeldWhy::Range {
            contact,
            range_m,
            min_m,
            max_m,
        } => {
            let bounds = if *min_m == 0 {
                format!("under {max_m}")
            } else if *max_m == u32::MAX {
                format!("past {min_m}")
            } else {
                format!("{min_m} to {max_m}")
            };
            format!("{} at {range_m} m, outside {bounds}", contact.designator)
        }
        HeldWhy::Rounds { rounds, keeps } => format!("{rounds} rounds, keeps {keeps}"),
    }
}

/// **A held slot's reason as the keyframe carries it** (17b cut M1b.1): the
/// notice object's `detail`, `{ "why", "percent", "floor", "aspect",
/// "range_m", "min_m", "max_m", "rounds", "keeps" }` — every key on every
/// held notice, the other whys' `null` — in whole per cents and metres, the
/// facet word for the aspect. The contact's designator rides the notice's own
/// `designator` key.
fn held_detail(why: &HeldWhy) -> serde_json::Value {
    let null = serde_json::Value::Null;
    let (word, percent, floor, aspect, range_m, min_m, max_m, rounds, keeps) = match why {
        HeldWhy::Odds { permille, floor } => (
            "odds",
            serde_json::json!(permille / 10),
            serde_json::json!(floor / 10),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null,
        ),
        HeldWhy::Aspect { aspect, .. } => (
            "aspect",
            null.clone(),
            null.clone(),
            serde_json::json!(aspect.as_str()),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null,
        ),
        HeldWhy::Range {
            range_m,
            min_m,
            max_m,
            ..
        } => (
            "range",
            null.clone(),
            null.clone(),
            null.clone(),
            serde_json::json!(range_m),
            serde_json::json!(min_m),
            serde_json::json!(max_m),
            null.clone(),
            null,
        ),
        HeldWhy::Rounds { rounds, keeps } => (
            "rounds",
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null,
            serde_json::json!(rounds),
            serde_json::json!(keeps),
        ),
    };
    serde_json::json!({
        "why": word,
        "percent": percent,
        "floor": floor,
        "aspect": aspect,
        "range_m": range_m,
        "min_m": min_m,
        "max_m": max_m,
        "rounds": rounds,
        "keeps": keeps,
    })
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
///
/// **16b cut V.3** made it `pub(crate)`: a pilot's report says a miss in exactly
/// these words (`crate::match_::report`).
pub(crate) fn miss_words(reason: &str) -> &str {
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
///
/// **16b cut V.3** made it `pub(crate)`: a pilot's report says a lost section of
/// its own tank in these words, over the same fired list — the debrief's fold
/// puts the ledger line's authored list through the question
/// `fired_consequences` asks (`crate::match_::report::OwnTank`).
pub(crate) fn consequence_words(consequences: &[SmolStr]) -> String {
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
        // 16b cut O.2: the cockpit refuses it locally off the frame; the arena's
        // word reaches here only when the frame was not yet there to say so.
        (Refusal::NoJammer, _) => CockpitError::NoJammer.to_string(),
        // **17b cut M1b.1** (doc 17 §5, D-UM-17 as amended): the floor's
        // sentence, naming the floor the arena captured; and the fallback for
        // an `Unlawful` that carries none — a `Contact` target, which the floor
        // forbids by kind in 17b cut M1b whatever the id.
        (Refusal::Unlawful, RefusalDetail::Floor { permille }) => CockpitError::Unlawful {
            floor_pct: permille / 10,
        }
        .to_string(),
        (Refusal::Unlawful, _) => "The Compact floor forbids that condition.".to_owned(),
        // The count's own word.
        (Refusal::NoSlot, _) => CockpitError::NoSlot.to_string(),
        // The law's other word (gated for a mind from 17b cut M1b.2).
        (Refusal::Unwarranted, _) => "That has no warrant.".to_owned(),
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
        IntentKind::fire(weapon).as_str(),
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

/// **What the machine was doing, in the pilot's words** (16b cut Y.4), by
/// [`crate::arena::reflex::ReflexState::as_str`]'s word. A word this build
/// predates reads as itself with its underscores opened out, which is the
/// panels' own fallback for a word they do not know.
fn machine_state_words(state: &str) -> String {
    match state {
        "startup" => "starting up".to_owned(),
        "patrol" => "patrolling".to_owned(),
        "approach" => "closing on a contact".to_owned(),
        "engage" => "engaging".to_owned(),
        "withdraw" => "withdrawing toward its entry zone".to_owned(),
        "disabled" => "disabled".to_owned(),
        other => other.replace('_', " "),
    }
}

/// **What the machine was under** (16b cut Y.4), by the standing order's
/// objective word ([`crate::deliberation::Objective::as_str`]), or `no order`
/// under doctrine.
fn machine_order_words(objective: Option<&str>) -> String {
    match objective {
        None => "no order".to_owned(),
        Some("hold") => "a hold order".to_owned(),
        Some("advance") => "an advance order".to_owned(),
        Some("engage") => "an engage order".to_owned(),
        Some("withdraw") => "a withdraw order".to_owned(),
        Some(other) => format!("a {} order", other.replace('_', " ")),
    }
}

/// `(x, y)`, the way every cell on the catch-up reads.
fn cell_words((x, y): (u16, u16)) -> String {
    format!("({x}, {y})")
}

/// **One list clause of the catch-up's contacts line** (16b cut Y.4): `none
/// new` / `new BA, X3` / `new BA, X3 and 3 more`.
fn listed_clause(word: &str, names: &[String], more: u32) -> String {
    match (names.is_empty(), more) {
        (true, 0) => format!("none {word}"),
        (true, more) => format!("{word} {more} more"),
        (false, 0) => format!("{word} {}", names.join(", ")),
        (false, more) => format!("{word} {} and {more} more", names.join(", ")),
    }
}

/// **A list with its overflow** (16b cut Y.4): `a, b` or `a, b and 2 more`, or
/// `none_word` when there is nothing at all.
fn listed_or(parts: &[String], separator: &str, more: u32, none_word: &str) -> String {
    match (parts.is_empty(), more) {
        (true, 0) => none_word.to_owned(),
        (true, more) => format!("{more} more"),
        (false, 0) => parts.join(separator),
        (false, more) => format!("{} and {more} more", parts.join(separator)),
    }
}

/// **One section's clause of the catch-up** (16b cut Y.4): the word change when
/// the word moved (`left_track intact then, destroyed now`), else the armour
/// that fell and the internals that fell (`hull armour 230 then, 90 now,
/// internals 60 then, 40 now`).
fn section_clause(id: &str, then: &SectionWords, now: &SectionWords) -> String {
    if then.condition != now.condition {
        return format!("{id} {} then, {} now", then.condition, now.condition);
    }
    let mut clauses: Vec<String> = Vec::new();
    if now.armour < then.armour {
        clauses.push(format!("armour {} then, {} now", then.armour, now.armour));
    }
    if now.internals < then.internals {
        clauses.push(format!(
            "internals {} then, {} now",
            then.internals, now.internals
        ));
    }
    if clauses.is_empty() {
        id.to_owned()
    } else {
        format!("{id} {}", clauses.join(", "))
    }
}

/// **The catch-up report, as the pilot reads it** (16b cut Y.4; doc 07 §1.3
/// step 5): one pure function of the report, and the block line mode is sent
/// as one `Sys` line tagged [`crate::arena_bridge::NOTICE_SENTENCE_TAG`]. Both
/// panels compose the same lines from the frame's `catchup` object
/// (`mush_net_ssh`'s `catchup_words`, the web's `catchupLines`), and
/// `web/src/lib/fixtures/cockpit/catchup_lines.json` pins all three.
///
/// ```text
/// Back aboard RA after 2 m 14 s away.
///   The machine was withdrawing toward its entry zone under no order.
///   Position: cell (64, 97) then, (70, 101) now.
///   Contacts: new BA, X3; lost X2.
///   Sections: left_track intact then, destroyed now; hull armour 230 then, 90 now.
///   Rounds spent: cannon 3, machine_gun 40.
///   Power online.
/// ```
///
/// Line 1's duration is `away_ms.div_euclid(1_000)` whole seconds through
/// [`crate::match_::duration_words`] — the wire's own `away_s`, so the line
/// client and both rings print one figure — and it says `away` and not `on
/// doctrine`, because the count starts at the `Linkdead`, before the machine
/// had the tank. An eighth line, `Your tank is a wreck.`, follows the power
/// line when it is one. A capped list ends `and N more`; the contacts' one
/// overflow count goes on the one list that is full — only a full list can
/// have overflowed — and when both are full, which cannot tell whose it was,
/// it is its own clause, `3 more not named`.
///
/// Nothing on any line names a unit, a pilot, an object number or a position
/// of any other unit: the contacts are the platform's own designators, held now
/// or held then (doc 16 §1, §3.6).
#[must_use]
pub fn catchup_lines(callsign: Designator, catchup: &Catchup) -> Vec<String> {
    let away_s = catchup.away_ms.div_euclid(1_000);
    let mut lines = vec![format!(
        "Back aboard {callsign} after {} away.",
        crate::match_::duration_words(away_s.saturating_mul(1_000))
    )];
    lines.push(match &catchup.machine {
        Some(machine) => format!(
            "  The machine was {} under {}.",
            machine_state_words(&machine.state),
            machine_order_words(machine.objective.as_deref())
        ),
        None => "  The machine had no state.".to_owned(),
    });
    lines.push(match (catchup.cell_then, catchup.cell_now) {
        (_, None) => "  Position: unknown.".to_owned(),
        (None, Some(now)) => format!("  Position: cell unknown then, {} now.", cell_words(now)),
        (Some(then), Some(now)) if then == now => {
            format!("  Position: cell {}, unchanged.", cell_words(now))
        }
        (Some(then), Some(now)) => format!(
            "  Position: cell {} then, {} now.",
            cell_words(then),
            cell_words(now)
        ),
    });
    let new: Vec<String> = catchup
        .contacts_new
        .iter()
        .map(ToString::to_string)
        .collect();
    let lost: Vec<String> = catchup
        .contacts_lost
        .iter()
        .map(ToString::to_string)
        .collect();
    lines.push(contacts_line(&new, &lost, catchup.contacts_more));
    let sections: Vec<String> = catchup
        .sections
        .iter()
        .map(|(id, then, now)| section_clause(id, then, now))
        .collect();
    lines.push(format!(
        "  Sections: {}.",
        listed_or(&sections, "; ", catchup.sections_more, "unchanged")
    ));
    let rounds: Vec<String> = catchup
        .rounds_spent
        .iter()
        .map(|(weapon, rounds)| format!("{weapon} {rounds}"))
        .collect();
    lines.push(format!(
        "  Rounds spent: {}.",
        listed_or(&rounds, ", ", catchup.rounds_more, "none")
    ));
    lines.push(format!("  Power {}.", catchup.power_now.as_str()));
    if catchup.wreck {
        lines.push("  Your tank is a wreck.".to_owned());
    }
    lines
}

/// **The catch-up's contacts line** (16b cut Y.4): `none new, none lost` when
/// nothing moved, else the two clauses split by `; `.
///
/// The wire carries one overflow count for both lists (`contacts_more`), and
/// only a list cut at [`MAX_CATCHUP_CONTACTS`] names can have overflowed, so
/// the count goes after the one list that is full. When both are full the
/// count cannot say whose it was — a platform that gained exactly twelve and
/// lost fifteen must not read as fifteen gained — so it is said as its own
/// clause, `3 more not named`, and so is a count beside no full list, which
/// the arena never sends.
fn contacts_line(new: &[String], lost: &[String], more: u32) -> String {
    if new.is_empty() && lost.is_empty() && more == 0 {
        return "  Contacts: none new, none lost.".to_owned();
    }
    let new_full = new.len() >= MAX_CATCHUP_CONTACTS;
    let lost_full = lost.len() >= MAX_CATCHUP_CONTACTS;
    let (new_more, lost_more, unnamed) = match (more, new_full, lost_full) {
        (0, _, _) => (0, 0, 0),
        (more, true, false) => (more, 0, 0),
        (more, false, true) => (0, more, 0),
        (more, _, _) => (0, 0, more),
    };
    let clauses = format!(
        "{}; {}",
        listed_clause("new", new, new_more),
        listed_clause("lost", lost, lost_more)
    );
    if unnamed == 0 {
        format!("  Contacts: {clauses}.")
    } else {
        format!("  Contacts: {clauses}; {unnamed} more not named.")
    }
}

/// **The catch-up report on the wire** (16b cut Y.4): the `catchup` object of
/// [`status_frame`]. Every figure is the report's own; `away_s` is
/// `away_ms.div_euclid(1_000)`, the one integer the three renderers print.
fn catchup_json(catchup: &Catchup) -> serde_json::Value {
    let words = |words: &SectionWords| {
        serde_json::json!({
            "condition": words.condition.as_str(),
            "armour": words.armour,
            "internals": words.internals,
        })
    };
    serde_json::json!({
        "away_s": catchup.away_ms.div_euclid(1_000),
        "cell_then": catchup.cell_then.map(|(x, y)| [x, y]),
        "cell_now": catchup.cell_now.map(|(x, y)| [x, y]),
        "power_then": catchup.power_then.as_str(),
        "power_now": catchup.power_now.as_str(),
        "wreck": catchup.wreck,
        "contacts_new": catchup
            .contacts_new
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>(),
        "contacts_lost": catchup
            .contacts_lost
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>(),
        "contacts_more": catchup.contacts_more,
        "sections": catchup
            .sections
            .iter()
            .map(|(id, then, now)| serde_json::json!({
                "id": id.as_str(),
                "then": words(then),
                "now": words(now),
            }))
            .collect::<Vec<serde_json::Value>>(),
        "sections_more": catchup.sections_more,
        "rounds_spent": catchup
            .rounds_spent
            .iter()
            .map(|(weapon, rounds)| serde_json::json!({
                "weapon": weapon.as_str(),
                "rounds": rounds,
            }))
            .collect::<Vec<serde_json::Value>>(),
        "rounds_more": catchup.rounds_more,
        "machine": catchup.machine.as_ref().map(|machine| serde_json::json!({
            "state": machine.state.as_str(),
            "goal": machine.goal.map(|(x, y)| [x, y]),
            "objective": machine.objective.as_ref().map(SmolStr::as_str),
        })),
    })
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
/// (`fired_by_default`), rounded to whole per cent: it is the same number the
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
/// `crate::arena::Arena::resolve_seen`, which reads the face and the
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
/// means the platform carries none, and `carried` does not go looking for an
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
            // 17b cut M1b.3: the arming, off the frame's own row too.
            armed: weapon.armed.clone(),
        })
        .collect()
}

/// **The weapon rows as the pilot may act on them now** (16b cut AP.1, 16c
/// `B-86`): [`frame_weapons`] off the held frame, each row's countdown aged by
/// the wall time since the world received that frame
/// ([`CockpitFrame::received`]).
///
/// A frame is never re-sent for a countdown change
/// ([`crate::arena::weapons::FrameWeapon::same_as`] leaves `ready_in_ms` out, on
/// purpose), so the `ready_in_ms` a pilot holds is the figure at the step that
/// built the frame, and read as it stands it refused every `FIRE` typed after the
/// gun's true ready instant and before the ready frame landed — a pulse late on
/// every cycle. Aged, the row reads `remaining = ready_in_ms − elapsed`, held at
/// nought, and is `ready` once `remaining` is nought; a row that was ready stays
/// ready. Nothing else on the row moves.
///
/// **`floored` leaves the rows the frame's own.** Under a staff `@arena/tempo`
/// floor the arena moves at most `max_catch_up_steps × period` of arena time per
/// wake of `max(period, floor)` wall time, so the wall clock runs ahead of the
/// arena's and an aged row would forward early into the arena's own `Still
/// cycling` refusal; a frame built under a floor since lifted carries the same
/// wall time with it. The cockpit's `floored` answers both.
///
/// It is the verbs' reading and never the panel's: the keyframe
/// ([`status_frame_held`]) carries the frame's own figures, which the TUI and the
/// web print as they stand until the next frame — neither counts down — so a
/// `WEAPONSTATUS` line can read `ready` beside a re-pushed panel that still
/// reads the frame's own seconds.
#[must_use]
pub fn aged_weapons(held: &CockpitFrame, now: std::time::Instant, floored: bool) -> Vec<WeaponRow> {
    let rows = frame_weapons(&held.frame);
    if floored {
        return rows;
    }
    // The saturating read: an elapsed past `i64::MAX` ms is every countdown done.
    let elapsed =
        i64::try_from(now.saturating_duration_since(held.received).as_millis()).unwrap_or(i64::MAX);
    rows.into_iter()
        .map(|row| {
            let remaining = row.ready_in_ms.saturating_sub(elapsed).max(0);
            WeaponRow {
                ready: row.ready || remaining == 0,
                ready_in_ms: remaining,
                ..row
            }
        })
        .collect()
}

/// **Whether the held frame's countdown must stay unaged** (16b cut AP.1, 16c
/// `B-86`): a staff tempo floor stands on its match
/// ([`crate::arena_bridge::MatchMirror::tempo_floor_ms`]), or the frame was built
/// at or before the arena time a floor was lifted at
/// ([`crate::arena_bridge::MatchMirror::floor_lifted_at_ms`] against
/// [`CockpitFrame::arena_ms`]). In the second case the rows are the frame's own
/// until the next frame, which carries an arena time past the lift. A match the
/// registry no longer mirrors has no floor.
fn floored(w: &World, held: &CockpitFrame) -> bool {
    w.module_state::<ArenaRegistry>()
        .and_then(|registry| registry.mirror(held.match_id))
        .is_some_and(|mirror| {
            mirror.tempo_floor_ms.is_some()
                || mirror
                    .floor_lifted_at_ms
                    .is_some_and(|at| held.arena_ms <= at)
        })
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
    // **16b cut I.4**: nothing here unbinds a staff watcher, because a watcher
    // cannot reach this verb's success: `BOARD` needs the actor's own centre
    // slot (read below), `@arena/watch` refuses anybody who holds one — theirs
    // or their account's — and a slot is claimed only while the centre is not
    // busy, one match at a time. `open_match` keeps the one defensive unbind.
    // **16b cut F.4**: the layer, first. A pod on a server with the layer off has
    // no tank behind it, so boarding one can only end in a refusal further down
    // the path; this is that refusal, said at the word the player typed.
    lobby::require_layer(ctx.w)?;
    let Some(CentreRoom::Team(team)) = centre::where_in_centre(ctx.w, actor) else {
        return Err(CockpitError::OutsideTeamRoom);
    };
    // Every refusal, read out of the centre in one borrow.
    //
    // **16b cut Y.3** (16c `N4`): the state arm is a match on the centre's
    // state, the slot and the match record's state, because `BOARD` now means a
    // second thing in a running match — a dropped pilot taking their tank back
    // (doc 07 §1.3). `Boarding` is the path it always was, byte for byte.
    let reclaim_min_ms = lobby::configs(ctx.w).map_or_else(
        || crate::config::MatchConfig::default().reclaim_min_ms,
        |(_, m, _)| m.reclaim_min_ms,
    );
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let (match_id, pod, index, reclaim) = {
        let book = ctx
            .w
            .module_state::<crate::match_::MatchBook>()
            .ok_or(CockpitError::Unavailable)?;
        let centre = book.centre();
        // The slot before the state, deliberately: somebody with no slot at all
        // is told the thing they can act on ("claim one"), not the thing they
        // cannot (what the centre happens to be doing).
        let slot = centre
            .slot_of_pilot(actor)
            .cloned()
            .ok_or(SimError::NoSlot)?;
        let record = centre
            .match_id
            .and_then(|match_id| book.get(match_id))
            .map(|record| record.state);
        // The pod's tank, by name, for the three `Running` sentences: the
        // pilot's own, which they may always be told about (doc 16 §1).
        let tank_name = || {
            slot.pod
                .and_then(|pod| centre::tank_of(ctx.w, pod))
                .or(slot.unit)
                .map(|tank| display_name(ctx.w, tank))
                .unwrap_or_default()
        };
        let reclaim = match (centre.state, record) {
            (MatchState::Boarding, _) => {
                if slot.team != team {
                    return Err(CockpitError::WrongRoom {
                        team: slot.team.as_str(),
                    });
                }
                if slot.is_seated() {
                    return Err(CockpitError::AlreadyAboard);
                }
                false
            }
            // A stop is already in flight (`unattended_deadline`, `SIM ABORT`):
            // the centre is still `Running` until `Ended` arrives, and a pilot
            // who took the tank back now would read *yours again* and then the
            // end. The record's word is the true one.
            (MatchState::Running, Some(MatchState::Stopping)) => {
                return Err(SimError::WrongState {
                    state: MatchState::Stopping.as_str(),
                }
                .into());
            }
            (MatchState::Running, _) if slot.is_dropped() => {
                let since = slot
                    .dropped_at_ms
                    .map_or(0, |dropped| now_ms.saturating_sub(dropped));
                let wait_ms = i64::from(reclaim_min_ms).saturating_sub(since);
                if wait_ms > 0 {
                    return Err(CockpitError::HandingOver {
                        unit: tank_name(),
                        // Rounded up: a pilot counts in seconds, and "0" would
                        // be a lie with a millisecond still to go.
                        secs: wait_ms.saturating_add(999) / 1_000,
                    });
                }
                true
            }
            // Out of the cockpit and not by a drop: an `EXIT`, which is one-way
            // (doc 16 §11's N.1 amendment), whatever came before it.
            (MatchState::Running, _) if slot.left => {
                return Err(CockpitError::LeftForGood { unit: tank_name() });
            }
            (MatchState::Running, _) => return Err(CockpitError::AlreadyAboard),
            (state, _) => {
                return Err(SimError::WrongState {
                    state: state.as_str(),
                }
                .into());
            }
        };
        let pod = slot.pod.ok_or(SimError::WrongState {
            state: centre.state.as_str(),
        })?;
        let match_id = centre.match_id.ok_or(SimError::WrongState {
            state: centre.state.as_str(),
        })?;
        (match_id, pod, slot.index, reclaim.then_some(slot.team))
    };
    if let Some(side) = reclaim {
        return reclaim_seat(ctx, match_id, pod, side, index);
    }
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
    // **16b cut B.4** (`B-38`): the pod's interior, after the sentence that put
    // the pilot there — `room_view` takes any object, so this is what `look`
    // from inside the pod shows. The failed-seat rollback above shows nothing
    // beyond its error.
    ctx.show_room(actor, pod);
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

/// **`BOARD` in a running match: take a dropped tank back** (16b cut Y.3; 16c
/// `N4`; doc 07 §1.3 "Reclaiming control"; doc 16 §3.8 "Disconnects").
///
/// [`board`] reaches this for a pilot whose slot [`lobby::Slot::is_dropped`] —
/// the linkdead grace ran out, or they quit — once
/// `[tacsim.match].reclaim_min_ms` has passed since the drop, with the record
/// still `Running`. It is F.1's seat path made **idempotent against what the
/// drop left behind**: the body still in the pod, the cockpit billet still this
/// actor's on the tank, the focus relinquished and the registry binding gone.
///
/// 0. **the room**: the body is inside its own pod — where a drop leaves it —
///    or standing in the slot's team room, where a restart's boot sweep or an
///    `@arena/reset` would have put it; anywhere else is
///    [`CockpitError::OutsideTeamRoom`];
/// 1. **the station**, declared exactly as [`board`] declares it (idempotent);
/// 2. **the body**, placed in the pod only when it is not already there — and
///    remembered, because only a body this verb moved is put back out;
/// 3. **the seat**, activated exactly as [`board`] activates it: the billet is
///    still this actor's, and the activation re-takes it and claims the focus.
///    On a refusal the body goes back out if step 2 moved it, and the pilot
///    reads [`CockpitError::Seat`];
/// 4. the actor's links, restated;
/// 5. **the arena**: the registry binds the pilot again and
///    [`crate::arena_bridge::reseat`] sends `Control { Live }` on every channel
///    at a **new** epoch, exactly as a revival inside the grace does (doc 07
///    §1.3 step 2) — the arena puts them back on its frame roster and takes the
///    tank from the reflex controller at its next pulse, where it stands;
/// 6. **the centre**: [`lobby::slot_reclaimed`] — the slot back in its cockpit,
///    the unattended clock cleared, the leads resettled (the heir keeps the
///    lead: 16c Q42's default);
/// 7. **the lines**: to the pilot that the tank is theirs again where it stands
///    — the last thing this verb says to them before the pod's interior (16b cut
///    Y.4 appends what happened meanwhile after it) — and to the team room, only
///    when step 2 moved the body, that they climbed back in.
///
/// No confirmation step (16c's default: the tank comes back where it is), and
/// no `slot_seated`: that is `Boarding`'s trigger, and the match is running.
fn reclaim_seat(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    pod: ObjId,
    team: crate::arena::state::Team,
    index: u8,
) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // (0) The room: the pilot's own pod, or their side's team room.
    let in_pod = pod_of_body(ctx.w, actor) == Some(pod);
    let in_team_room = pod_of_body(ctx.w, actor).is_none()
        && centre::where_in_centre(ctx.w, actor) == Some(CentreRoom::Team(team));
    if !in_pod && !in_team_room {
        return Err(CockpitError::OutsideTeamRoom);
    }
    let tank = centre::tank_of(ctx.w, pod).ok_or(SimError::NoUnit {
        team: team.as_str(),
        index,
    })?;
    let station = station_id().ok_or(CockpitError::Unavailable)?;
    let room = ctx.w.get(pod).and_then(|object| object.location);

    // (1) The station, as `board` declares it.
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

    // (2) The body, only when it is not in the pod already.
    let moved = !in_pod;
    if moved {
        mush_sim::place_interior(ctx, actor, pod, None)
            .map_err(|error| CockpitError::Hatch(error.to_string()))?;
    }

    // (3) The seat. The billet is still this actor's after a drop, and the
    // activation re-takes it; a body this verb moved goes back out on a refusal,
    // and one it found in the pod stays where the drop left it.
    if let Err(error) = mush_sim::request_seat_activation(
        ctx,
        &mush_sim::SeatRequest::Activate {
            unit: tank,
            station: station.clone(),
            npc: actor,
            granted: cockpit_channels(),
            fallback: None,
        },
    ) {
        if moved && let Some(room) = room {
            put_out(ctx, actor, room);
        }
        return Err(CockpitError::Seat(error.to_string()));
    }
    // (4) As `board`: the idempotent restatement.
    mush_sim::links::refresh_actor_links(ctx.w, actor);

    // (5) The arena: bound again, then `Live` at a new epoch on every channel.
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.bind_pilot(actor, match_id, tank);
    }
    crate::arena_bridge::reseat(ctx, tank, actor);

    // (6) The centre.
    lobby::slot_reclaimed(ctx, match_id, actor, tank)?;

    // (7) The lines.
    let pilot_line = format!(
        "You climb back into {}: {} is yours again where it stands.",
        display_name(ctx.w, pod),
        display_name(ctx.w, tank)
    );
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, pilot_line);
    if moved && let Some(room) = room {
        let room_line = format!(
            "{} climbs back into {}.",
            display_name(ctx.w, actor),
            display_name(ctx.w, pod)
        );
        tell_room(ctx, room, Vec::new(), room_line);
    }
    ctx.show_room(actor, pod);
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
    // **16b cut Y.3**: and the leave is final, whatever came before it. A pilot
    // who dropped, reconnected inside the pod and typed `EXIT` still held the
    // billet (so the loop above handed it back), but their slot was already
    // `left` and `slot_left` changed nothing — the drop's stamp would have
    // survived and the sentence below, *you cannot take it back*, been false.
    if running {
        lobby::slot_exited(ctx, actor);
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
        // **16b cut B.4** (`B-38`): and where they are standing, AFTER the
        // sentence that explains the move — never inside `put_out`, or the
        // room would land before "You climb out".
        ctx.show_room(actor, room);
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
/// Three steps, and the order is `EXIT`'s because `EXIT`'s was right, and since
/// 16b cut Z.4 a fourth after them:
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
///    next pilot's `StationHeld`;
/// 4. **the side's net** (16b cut Z.4), left through
///    [`crate::match_::net::leave_pilot`]: the pilot joined it with the match
///    and leaves it with the seat, and reads comsys's own `You have been
///    removed from channel RedNet.` once.
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
    let vacated = match mush_sim::vacate_station(ctx, unit, station, actor) {
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
    };
    // 4. **16b cut Z.4 (16c `N8`): the radio goes with the seat**, whatever the
    //    billet answered. Both side nets, idempotent — a pilot on neither (a
    //    staff-seated one, a second sweep) reads nothing — so this one line
    //    covers `EXIT`, a seated `SIM LEAVE`, the check-in and the boot's sweep.
    //    A drop past the grace never reaches here, and the pilot stays on their
    //    net for the reclaim.
    crate::match_::net::leave_pilot(ctx, actor);
    vacated
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
/// restart ([`eject_all_at_boot`], its silent sibling). Idempotent, and fallible
/// per pod without failing the others.
pub(crate) fn eject_all(ctx: &mut Ctx<'_>) -> EjectReport {
    sweep_pods(ctx, true, &BTreeSet::new())
}

/// **[`eject_all`] at boot** (16b cut U.4, 16c `B-37`), from the module's
/// `on_all_loaded`: the same bodies moved and the same billets given back, and
/// **nothing said**.
///
/// Boot output reaches nobody — the drivers drop it, because no session is
/// attached yet — and on the split server a pilot's session survives the restart
/// of the server half and is still waiting to be told why they are standing in
/// the team room. So each `(body, pod)` it opens is recorded on the match book
/// ([`crate::match_::MatchBook::defer_hatch`]) and the module's boarding tick
/// says the hatch line and the room once that body's session is attached, for
/// at most [`crate::match_::HATCH_HOLD_S`] ([`crate::match_::sim`]'s
/// `board_tick`).
///
/// **16b cut AF.3: a resumed match is exempt.** When the boot resumed the
/// centre's match (the centre `Running` on a match id, which only the resume
/// leaves it at boot), its slots' pods and their tanks are left alone: the
/// bodies stay inside, no hatch line is owed, and the billets stay on the
/// tanks — Y.3's reclaim re-runs the seat path idempotently from the pod with
/// the billet held. Every other pod is swept as before, and a crash during
/// `Boarding` (no marker, no snapshot, nothing resumed) still opens the hatch.
pub(crate) fn eject_all_at_boot(ctx: &mut Ctx<'_>) -> EjectReport {
    let keep: BTreeSet<ObjId> = lobby::centre(ctx.w)
        .filter(|centre| centre.state == MatchState::Running && centre.match_id.is_some())
        .map(|centre| {
            centre
                .slots
                .iter()
                .filter_map(|slot| slot.pod)
                .flat_map(|pod| [Some(pod), centre::tank_of(ctx.w, pod)])
                .chain(centre.slots.iter().map(|slot| slot.unit))
                .flatten()
                .collect()
        })
        .unwrap_or_default();
    sweep_pods(ctx, false, &keep)
}

/// Both sweeps' one body. `told` says the hatch line and the room at once; its
/// absence defers them to the boarding tick. `keep` names pods and tanks the
/// sweep leaves exactly as they are (16b cut AF.3: a resumed match's).
fn sweep_pods(ctx: &mut Ctx<'_>, told: bool, keep: &BTreeSet<ObjId>) -> EjectReport {
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
        if let Some(tank) = tank.filter(|tank| !keep.contains(tank)) {
            report.merge(hand_back_seats_of_unit(ctx, tank));
        }
        if keep.contains(&pod) {
            continue;
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
            if told {
                tell_hatch_open(ctx, body, pod, room);
            } else if let Some(book) = ctx.w.module_state_mut::<crate::match_::MatchBook>() {
                book.defer_hatch(body, pod);
            }
            report.bodies.push(body);
        }
    }
    report
}

/// **Tell one body its pod is open**: the hatch line, then the room it is
/// standing in (16b cut B.4, `B-38`: after the line that explains it).
///
/// [`eject_all`] says it at once; the boot's sweep defers it, and the module's
/// first tick says it here (16b cut U.4) — with the room the body stands in by
/// then, which is the room the sweep put it in unless it has walked since.
pub(crate) fn tell_hatch_open(ctx: &mut Ctx<'_>, body: ObjId, pod: ObjId, room: ObjId) {
    let line = format!("The hatch of {} swings open.", display_name(ctx.w, pod));
    ctx.out.sys_to_actor(body, NOTICE_CATEGORY, line);
    ctx.show_room(body, room);
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
    // 16b cut B.4 (`B-38`): as `eject_all`, the room after the hatch line.
    tell_hatch_open(ctx, actor, pod, room);
    report.bodies.push(actor);
    report
}

// ---------------------------------------------------------------------------
// K3: the cockpit verbs (twelve at K3; `ORDER`, the thirteenth, is below
// under "Commanding the machines"; `EMCON`, the fourteenth, follows `STARTUP`,
// `WATCH`, the fifteenth, follows `EMCON`, and `DRIVE`, the sixteenth, follows
// `WATCH`)
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
fn startup(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // 16b cut D.2 (`B-03`): before the power, so a pilot in a wreck is never told
    // to wait for a startup that is not coming.
    require_alive(ctx.w, seat.driver)?;
    match power_of(ctx.w, seat.driver) {
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
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Startup)?;
    // The optimism is stamped only once the order is away: a refused forward is
    // not a startup in progress.
    let until_s = hint_until_s(ctx.w);
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.hint_starting(seat.driver, until_s);
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

/// **The world second a hint stamped now expires at** (16b cut D.1; shared by
/// `EMCON`'s hint since 16b cut O.2): now plus one whole `Quiet` period in
/// seconds, rounded up, plus one — the longest a frame can be away, and one
/// second more. [`HINT_FALLBACK_S`] when the configuration cannot be read.
fn hint_until_s(w: &World) -> i64 {
    let stands_s = crate::match_::lobby::configs(w).map_or(HINT_FALLBACK_S, |(arena_cfg, _, _)| {
        // Whole seconds, rounded up, plus one: the hint must outlast the
        // longest a frame can be away.
        let quiet_ms = i64::from(arena_cfg.tempo.quiet_ms).max(0);
        quiet_ms.saturating_add(999) / 1_000 + 1
    });
    w.now().saturating_add(stands_s)
}

/// **`STATUS`'s pod line** (16b cut O.2, 16c N12), printed after the receiver
/// line and only for a platform that carries a pod ([`Frame::ecm`]).
#[must_use]
pub const fn emcon_status_line(on: bool) -> &'static str {
    if on {
        "  emcon on: the pod radiates"
    } else {
        "  emcon off: the pod is silent"
    }
}

/// `EMCON [on|off]` — the jamming pod's switch (16b cut O.2; 16c N12).
///
/// **Bare**, it reads the held frame: `EMCON on: the pod radiates.` or `EMCON
/// off: the pod is silent.`, [`CockpitError::NoJammer`] for a tank with no pod,
/// and [`CockpitError::NoFrameYet`] before the first frame. **With a word**
/// (`on` or `off`, any case; anything else is the usage line) it is a discrete
/// order on the sensors channel:
///
/// * refused locally with [`CockpitError::NoJammer`] when the held frame says
///   the tank carries no pod, and with [`CockpitError::NoFrameYet`] before the
///   first frame — the switch cannot be known to exist, and the arena would
///   refuse it anyway if it does not;
/// * answered `EMCON is already on.` / `EMCON is already off.`, forwarding
///   nothing, when the switch is already where the order puts it — read off
///   **the bridge's hint first and the mirror second**
///   ([`crate::arena_bridge::ArenaRegistry::emcon_hint`]; the cut O plan's item
///   38). An intent is buffered to the next pulse and never steps the arena,
///   so the mirror is up to one `Quiet` period stale: without the hint `EMCON
///   off` then `EMCON on` inside one pulse would answer the second from a
///   mirror that still read `on`, and the tank would end silent while the
///   pilot was told otherwise. With it, both are forwarded with true replies,
///   and the arena resolves them in order to the state the pilot left;
/// * otherwise forwarded, the hint stamped with `STARTUP`'s expiry
///   ([`hint_until_s`]), and answered `EMCON coming on; the pod radiates from
///   the next pulse.` / `EMCON going off; the pod is silent from the next
///   pulse.`
///
/// The wreck comes first, as it does for every order (`require_alive`). Power
/// is not asked: the switch is a position, and the arena's jamming fold is
/// what reads power. What the arena makes of the order is said to the pilot's
/// own frame as an `emcon_on`/`emcon_off` notice, which names what switching
/// off costs.
fn emcon(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    require_alive(ctx.w, seat.driver)?;
    let word = rest.trim().to_ascii_lowercase();
    let wanted = match word.as_str() {
        "" => None,
        "on" => Some(true),
        "off" => Some(false),
        _ => return Err(CockpitError::Usage("EMCON on|off.".to_owned())),
    };
    let held = last_frame(ctx.w, seat.driver).ok_or(CockpitError::NoFrameYet)?;
    let Some(mirror) = held.frame.ecm else {
        return Err(CockpitError::NoJammer);
    };
    let Some(on) = wanted else {
        let line = if mirror {
            "EMCON on: the pod radiates."
        } else {
            "EMCON off: the pod is silent."
        };
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
        return Ok(());
    };
    // The hint first, the mirror second (the cut O plan's item 38).
    let now = ctx.w.now();
    let current = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.emcon_hint(seat.driver, now))
        .unwrap_or(mirror);
    if current == on {
        let line = if on {
            "EMCON is already on."
        } else {
            "EMCON is already off."
        };
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
        return Ok(());
    }
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Emcon { on })?;
    // Stamped only once the order is away, as `STARTUP`'s is.
    let until_s = hint_until_s(ctx.w);
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.hint_emcon(seat.driver, on, until_s);
    }
    let line = if on {
        "EMCON coming on; the pod radiates from the next pulse."
    } else {
        "EMCON going off; the pod is silent from the next pulse."
    };
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **`WATCH ON`'s reply** (16b cut AP.3, 16c N19), said before the first block.
pub const WATCHING: &str = "Watching: your own line, your guns and your contacts are pushed as they change. \
     WATCH OFF stops it.";

/// **`WATCH OFF`'s reply** (16b cut AP.3).
pub const WATCH_OFF: &str = "Watch off.";

/// **Bare `WATCH`, while the switch is on** (16b cut AP.3).
pub const WATCH_IS_ON: &str = "Watch is on.";

/// **Bare `WATCH`, while it is off** — which is how every seat starts (16b cut
/// AP.3).
pub const WATCH_IS_OFF: &str = "Watch is off.";

/// **`WATCH`'s usage line** (16b cut AP.3), for any word but `on` and `off`.
pub const WATCH_USAGE: &str = "WATCH [ON|OFF]; bare WATCH says which.";

/// `WATCH [ON|OFF]` — the line-mode digest's switch (16b cut AP.3; 16c N19).
///
/// **Why it exists.** A line client is pull-only: the bridge pushes the
/// `arena.cockpit` keyframe on every changed frame and prints the frame's
/// transition notices, but `mush_ansi::render` prints a panel only in the grid
/// shape, so a line-mode pilot learns their speed, their guns' readiness and
/// their contacts' ranges only by typing `STATUS`, `WEAPONSTATUS` and
/// `CONTACTS` — and three reads in five said nothing new. With the switch on,
/// the bridge pushes the pilot [`watch_lines`]' block after each keyframe whose
/// block reads differently from the last one pushed
/// (`crate::arena_bridge::on_frames`). Off by default (16c N19's owner
/// default), so a pilot who never types it reads exactly what they read before.
///
/// * `WATCH ON` (any case) sets the flag
///   ([`ArenaRegistry::set_line_watch`]), says [`WATCHING`] and then — when the
///   panel has told the pilot anything — that frame's block at once, as ONE
///   `Sys` tagged [`NOTICE_SENTENCE_TAG`], built with the **aged** weapon rows
///   (`carried`, 16b cut AP.1) so a pilot who types `WATCH ON` and
///   `WEAPONSTATUS` in one breath reads one countdown, not two. The block is
///   recorded ([`ArenaRegistry::note_watch_block`]) so the next frame's
///   identical block is not said again;
/// * `WATCH OFF` clears it and forgets the last block, and says [`WATCH_OFF`];
/// * bare `WATCH` says [`WATCH_IS_ON`] or [`WATCH_IS_OFF`];
/// * any other word is [`WATCH_USAGE`].
///
/// The flag is keyed on [`Seat::driver`], the pilot the registry binds and the
/// one the frames go to, and it is the **seat's**: the registry drops it with
/// the seat (`unbind_pilot`) and with the match (`untrack`), so a pilot who
/// boards again types it again. Nothing crosses the seam and nothing is
/// written but the RAM row: it is a view, never an order, and a wreck may
/// watch.
fn watch(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let on = match rest.trim().to_ascii_lowercase().as_str() {
        "" => {
            let watching = ctx
                .w
                .module_state::<ArenaRegistry>()
                .is_some_and(|registry| registry.line_watching(seat.driver));
            let line = if watching { WATCH_IS_ON } else { WATCH_IS_OFF };
            ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
            return Ok(());
        }
        "on" => true,
        "off" => false,
        _ => return Err(CockpitError::Usage(WATCH_USAGE.to_owned())),
    };
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.set_line_watch(seat.driver, on);
    }
    if !on {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, WATCH_OFF);
        return Ok(());
    }
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, WATCHING);
    let Some(held) = last_frame(ctx.w, seat.driver) else {
        return Ok(());
    };
    // 16b cut AP.1: read once, at the top of the block, as `FIRE` and
    // `WEAPONSTATUS` read it — the aged rows, so this block and a
    // `WEAPONSTATUS` typed in the same breath say one countdown.
    let now = std::time::Instant::now();
    let weapons = carried(ctx.w, seat.unit, Some(&held), now);
    let block = watch_lines(&held.frame, &held.ranges, &weapons, pass_ms(ctx.w)).join("\n");
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        // Said whatever the last block was — the pilot asked — and recorded,
        // so the next frame's identical block is not said again.
        registry.note_watch_block(seat.driver, &block);
    }
    ctx.out.sys_to_actor(actor, NOTICE_SENTENCE_TAG, block);
    Ok(())
}

// ---------------------------------------------------------------------------
// `DRIVE`, the sixteenth selector (17b cut M1a.4; doc 17 §4 D-UM-10)
// ---------------------------------------------------------------------------

/// **`DRIVE`'s usage line** (17b cut M1a.4), for a bare `DRIVE`.
pub const DRIVE_USAGE: &str = "DRIVE <spot|x,y|HOLD|OFF>; STATUS names the nearest spot.";

/// **`DRIVE HOLD`'s reply** (17b cut M1a.4).
pub const DRIVE_HOLDING: &str = "Holding here.";

/// **`DRIVE OFF`'s reply** (17b cut M1a.4).
pub const DRIVE_RELEASED: &str = "DRIVE off; the helm is yours.";

/// **The one byte-identical refusal for any word `DRIVE` cannot drive to**
/// (17b cut M1a.4): not a spot's name or letter, not a cell, not `HOLD` or
/// `OFF`. Never an echo of the word, and a contact's designator is "not a
/// spot" like any other word — so nothing about a held or an unheld contact is
/// read or said (doc 16 §3.6's one-answer rule, read for a verb that takes no
/// contact at all).
pub const DRIVE_NOT_A_GOAL: &str = "DRIVE refused. That is not a spot, a cell, HOLD or OFF.";

/// **`DRIVE x,y` off the arena window** (17b cut M1a.4).
pub const DRIVE_OFF_WINDOW: &str = "DRIVE refused. That cell is not on the window.";

/// **What the motor says when a `DRIVE` arrives** (17b cut M1a.4), off
/// [`crate::arena::perception::DriveOutcome::Arrived`].
pub const DRIVE_ARRIVED: &str = "DRIVE: arrived.";

/// **What the motor says when no route reaches a `DRIVE`'s goal** (17b cut
/// M1a.4), off [`crate::arena::perception::DriveOutcome::NoRoute`]: the goal
/// is dropped and the tank holds where it is.
pub const DRIVE_NO_ROUTE: &str = "DRIVE: no route from here; holding.";

/// **The two integers of `DRIVE x,y` or `DRIVE x y`** (17b cut M1a.4), or
/// `None` for a word that is not exactly two unsigned integers — `12`, `12,`
/// and `12,34,5` are not cells. Wide enough that an integer off any window
/// reads as a cell off the window rather than as "not a cell".
fn drive_cell(word: &str) -> Option<(u64, u64)> {
    let (x, y) = match word.split_once(',') {
        Some((x, y)) => (x.trim(), y.trim()),
        None => {
            let mut parts = word.split_whitespace();
            let (Some(x), Some(y), None) = (parts.next(), parts.next(), parts.next()) else {
                return None;
            };
            (x, y)
        }
    };
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// `DRIVE <spot|x,y|HOLD|OFF>` — hand the helm to the tank's own drive-by-wire
/// (17b cut M1a.4; doc 17 §4 D-UM-3 and D-UM-10 with the M1a plan's departure
/// (f): the word is `DRIVE` because core's baseline `goto`, with its aliases
/// `go` and `move`, holds that name and the command registry refuses a
/// collision; doc 16 §3.7).
///
/// The local pre-checks first, in `HEADING`'s order and words — the wreck
/// ([`CockpitError::Destroyed`]), the dead engine ([`CockpitError::Immobile`]),
/// the cold or starting tank ([`CockpitError::PowerNotOnline`],
/// [`CockpitError::PowerStarting`]) — because a goal is a helm order the arena
/// would refuse for the same three. Then the word, trimmed and case-folded,
/// read by [`drive_goal`]:
///
/// * nothing: [`DRIVE_USAGE`];
/// * `HOLD`: [`crate::arena::intents::MotorGoal::Hold`], [`DRIVE_HOLDING`] —
///   the tank stops and keeps no goal;
/// * `OFF`: [`crate::arena::intents::MotorGoal::Off`], [`DRIVE_RELEASED`] —
///   the goal is dropped and the tank keeps its last heading and speed;
/// * two integers `x,y` or `x y`: a cell of the arena window, `Steering for
///   cell 12,34.`; a cell off the window is [`DRIVE_OFF_WINDOW`];
/// * a key spot's whole name or its map letter, exactly as `RANGE` resolves
///   one ([`resolve_spot`]): its cell, `Steering for The Ford (A).`;
/// * anything else — a contact's designator among them — [`DRIVE_NOT_A_GOAL`],
///   byte for byte.
///
/// The goal is staged through the same forward every ordering verb uses, as a
/// continuous [`IntentKind::Goal`] on the helm channel, and counted on the
/// seat's rate by the arena. What the motor makes of it comes back as a notice
/// ([`DRIVE_ARRIVED`], [`DRIVE_NO_ROUTE`]); the route itself is never shown.
fn drive(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // As `HEADING`: the wreck, then the motion limit, then the power.
    require_alive(ctx.w, seat.driver)?;
    require_mobile(ctx.w, seat.driver)?;
    require_power(ctx.w, seat.driver)?;
    let word = rest.trim().to_ascii_lowercase();
    // The ground is read only for a word that may name a place on it.
    let (spots, cols, rows) = if matches!(word.as_str(), "" | "hold" | "off") {
        (Vec::new(), 0, 0)
    } else {
        let held = last_frame(ctx.w, seat.driver);
        match_of(ctx.w, seat.driver, held.as_ref())
            .and_then(|match_id| {
                ctx.w
                    .module_state::<ArenaRegistry>()
                    .and_then(|registry| registry.arena_of(match_id))
                    .map(|ground| {
                        (
                            ground.key_spots.clone(),
                            ground.terrain.width(),
                            ground.terrain.height(),
                        )
                    })
            })
            .unwrap_or((Vec::new(), 0, 0))
    };
    let (goal, line) = drive_goal(&word, &spots, cols, rows)?;
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Goal(goal))?;
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **What one `DRIVE` word asks of the motor, and what the pilot is told**
/// (17b cut M1a.4): [`drive`]'s whole reading of the word, pure over the
/// ground's key spots and its window of `cols × rows` cells, so the goal a
/// spot's name or letter forwards is the cell the reply names. `word` is
/// already trimmed and case-folded; the arms are [`drive`]'s list. A missing
/// ground (`cols == rows == 0`, no spots) reads every cell as off the window
/// and every other word as not a goal.
fn drive_goal(
    word: &str,
    spots: &[KeySpot],
    cols: u16,
    rows: u16,
) -> Result<(MotorGoal, String), CockpitError> {
    match word {
        "" => Err(CockpitError::Usage(DRIVE_USAGE.to_owned())),
        "hold" => Ok((MotorGoal::Hold, DRIVE_HOLDING.to_owned())),
        "off" => Ok((MotorGoal::Off, DRIVE_RELEASED.to_owned())),
        _ => {
            if let Some((x, y)) = drive_cell(word) {
                let on_window = (u16::try_from(x), u16::try_from(y));
                let (Ok(x), Ok(y)) = on_window else {
                    return Err(CockpitError::Refused(DRIVE_OFF_WINDOW.to_owned()));
                };
                if x >= cols || y >= rows {
                    return Err(CockpitError::Refused(DRIVE_OFF_WINDOW.to_owned()));
                }
                return Ok((
                    MotorGoal::Cell { x, y },
                    format!("Steering for cell {x},{y}."),
                ));
            }
            let not_a_goal = || CockpitError::Refused(DRIVE_NOT_A_GOAL.to_owned());
            let (index, spot) = resolve_spot(spots, word).ok_or_else(not_a_goal)?;
            let letter = spot_letter(index).ok_or_else(not_a_goal)?;
            Ok((
                MotorGoal::Cell {
                    x: spot.x,
                    y: spot.y,
                },
                format!("Steering for {} ({letter}).", spot.name),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// The line-mode digest (16b cut AP.3; 16c N19)
// ---------------------------------------------------------------------------

/// **The widest a digest line may be**, in characters (16b cut AP.3): the
/// terminal-width floor (16c cut AB), so a line client at the floor never
/// wraps one.
pub const WATCH_COLUMNS: usize = 120;

/// **The line-mode digest** (16b cut AP.3, 16c `N19`): one to three lines of
/// the pilot's own picture, which the bridge pushes a `WATCH ON` pilot as one
/// `Sys` whenever it reads differently from the last it pushed.
///
/// 1. **The own line, always**: `Watch RA: online, 24 kph, heading 090°`, with
///    `, HELD` and `, DESTROYED` after it as `STATUS` prints them, in that
///    order — `STATUS`'s second line after its colon, **without** the word
///    `power` (a reader that splits on `": power "`, as the bot's does, must
///    never take the digest for a `STATUS` reply) and **without** a position
///    (the block must not change with every cell).
/// 2. **The guns line, only while `weapons` is not empty**: `Watch guns:
///    cannon ready, machine_gun (mg) cycling (3 s); lock BA, 450 m, 46% to
///    hit` — the rows **given**, in order, each with the short name a pilot may
///    type ([`weapon_short_of`]) and its readiness rounded up to the second as
///    [`WeaponRow::render`] rounds it — or `empty` once its rounds are gone,
///    whatever its cycle, because `FIRE` refuses an empty gun and a gun that
///    read `ready` with nothing in it would be a lie the pilot only finds out
///    by firing; then the lock: its designator, ` (FRIENDLY)`
///    on a friend (`friendly_mark`), the held range of its row and the chance
///    of the shot a bare `FIRE` takes (`fired_by_default`, the figure
///    [`lock_line`] prints); `lock BA, 450 m` with no chance resolved; `no
///    lock` with none. The caller chooses the rows: the bridge passes the
///    frame's own ([`frame_weapons`]), the verb the aged ones (`carried`).
/// 3. **The contacts line, only while the platform holds a contact**: `Watch
///    contacts: BA blue 087° far 450 m; X3 unknown 315° effective 200 m,
///    plotted 4 s ago` — the rows `CONTACTS` prints, in its order, the range
///    held against `ranges` ([`frame_contacts_held`]), and a plot's age only
///    once [`stale_clause`] says it — older than the sensor pass `pass_ms`. A
///    fresh plot's age is never printed, or a held contact would change the
///    block every second.
///
/// **The fit rule** for lines 2 and 3: entries go in in order while the line
/// stays within [`WATCH_COLUMNS`] characters with room kept for the tail; the
/// first that does not fit ends the line `; +N more`, `N` the entries left
/// out. The lock is always printed: the gun rows give way to it.
///
/// **The observer rule by construction** (doc 16 §1, §5): a pure function of
/// the pilot's own frame, the ranges the pilot was told and the rows it is
/// given, and nothing a pilot could not read off `STATUS`, `WEAPONSTATUS` and
/// `CONTACTS` — designators only, never an object number, a name or a pilot,
/// and no position. **No tempo word** (16c `T-59`): `STATUS` prints the
/// pilot's own scope and never the arena's tempo, and neither does this.
#[must_use]
pub fn watch_lines(
    frame: &Frame,
    ranges: &BTreeMap<ContactId, u64>,
    weapons: &[WeaponRow],
    pass_ms: i64,
) -> Vec<String> {
    let mut lines = vec![format!(
        "Watch {}: {}, {} kph, heading {:03}°{}{}",
        frame.callsign,
        frame.power.as_str(),
        cms_to_kph(frame.speed_cms),
        frame.pose.heading_mdeg / 1_000,
        if frame.held { ", HELD" } else { "" },
        if frame.wreck { ", DESTROYED" } else { "" }
    )];
    let rows = frame_contacts_held(frame, ranges);
    if !weapons.is_empty() {
        let guns: Vec<String> = weapons
            .iter()
            .map(|row| {
                let named = match weapon_short_of(weapons, &row.id) {
                    Some(short) => format!("{} ({short})", row.id),
                    None => row.id.to_string(),
                };
                // An empty gun says so before its cycle: `FIRE` refuses it
                // whatever the breech is doing (`CockpitError::Empty`), and
                // the last round leaving is a change the block must push —
                // `WEAPONSTATUS` prints the rounds, and this has no room to.
                if row.rounds <= 0 {
                    format!("{named} empty")
                } else if row.ready {
                    format!("{named} ready")
                } else {
                    format!(
                        "{named} cycling ({} s)",
                        row.ready_in_ms.saturating_add(999) / 1_000
                    )
                }
            })
            .collect();
        let locked = frame_lock(frame).and_then(|lock| rows.iter().find(|row| row.id == lock));
        let lock = match locked {
            Some(row) => {
                let mark = friendly_mark(frame, row.id);
                match fired_by_default(&frame.weapons).and_then(|weapon| weapon.p_permille) {
                    Some(permille) => format!(
                        "; lock {}{mark}, {} m, {}% to hit",
                        row.designator,
                        row.range_m,
                        permille / 10
                    ),
                    None => format!("; lock {}{mark}, {} m", row.designator, row.range_m),
                }
            }
            None => "; no lock".to_owned(),
        };
        let columns = WATCH_COLUMNS.saturating_sub(lock.chars().count());
        let mut line = fit_entries("Watch guns: ", &guns, ", ", columns);
        line.push_str(&lock);
        lines.push(line);
    }
    if !rows.is_empty() {
        let contacts: Vec<String> = rows
            .iter()
            .map(|row| {
                format!(
                    "{} {} {:03}° {} {} m{}",
                    row.designator,
                    row.class,
                    row.bearing_deg,
                    row.band,
                    row.range_m,
                    stale_clause(row.age_ms, pass_ms)
                )
            })
            .collect();
        lines.push(fit_entries(
            "Watch contacts: ",
            &contacts,
            "; ",
            WATCH_COLUMNS,
        ));
    }
    lines
}

/// **One digest line, fitted** (16b cut AP.3): `head` and then `entries`
/// joined by `sep`, in order, while the line stays within `columns`
/// characters — keeping room, whenever entries would remain, for the tail
/// `; +N more` that says how many were left out. Stops at the first entry that
/// does not fit, so what is printed is always a prefix of the list in its own
/// order. Characters and not bytes, because `°` is two bytes and one column.
fn fit_entries(head: &str, entries: &[String], sep: &str, columns: usize) -> String {
    let mut line = head.to_owned();
    let mut width = head.chars().count();
    let mut taken = 0_usize;
    for entry in entries {
        let joiner = if taken == 0 { "" } else { sep };
        let after = width + joiner.chars().count() + entry.chars().count();
        let left = entries.len().saturating_sub(taken + 1);
        let tail = if left == 0 {
            0
        } else {
            more_tail(left).chars().count()
        };
        if after + tail > columns {
            break;
        }
        line.push_str(joiner);
        line.push_str(entry);
        width = after;
        taken += 1;
    }
    let left = entries.len().saturating_sub(taken);
    if left > 0 {
        if taken > 0 {
            line.push_str(&more_tail(left));
        } else {
            // Nothing fitted at all (a width no shipped word reaches): the
            // count alone, with no separator before it.
            line.push_str(&format!("+{left} more"));
        }
    }
    line
}

/// The tail a fitted digest line ends with when entries were left out.
fn more_tail(left: usize) -> String {
    format!("; +{left} more")
}

/// `HEADING <deg>` — 0 to 359, clockwise from north (doc 16 §3.7).
fn heading(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let deg: u32 = rest
        .trim()
        .parse()
        .ok()
        .filter(|deg| *deg < 360)
        .ok_or_else(|| CockpitError::Usage("HEADING <0-359>, clockwise from north.".to_owned()))?;
    // 16b cut D.2 (`B-03`), in the arena's own order: the wreck, then the motion
    // limit, then the power.
    require_alive(ctx.w, seat.driver)?;
    require_mobile(ctx.w, seat.driver)?;
    require_power(ctx.w, seat.driver)?;
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
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
fn speed(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let kph: i32 =
        rest.trim().parse().ok().ok_or_else(|| {
            CockpitError::Usage("SPEED <kph>; a negative speed reverses.".to_owned())
        })?;
    let (max_cms, reverse_cms) = speed_ceilings(ctx.w, seat.unit);
    let max_kph = cms_to_kph(i32::try_from(max_cms).unwrap_or(i32::MAX));
    let reverse_kph = cms_to_kph(i32::try_from(reverse_cms).unwrap_or(i32::MAX));
    if kph > max_kph || kph < -reverse_kph {
        return Err(CockpitError::Usage(format!(
            "That unit makes {max_kph} kph ahead and {reverse_kph} astern; SPEED -{reverse_kph} \
             to {max_kph}."
        )));
    }
    // 16b cut D.2 (`B-03`): as `HEADING`.
    require_alive(ctx.w, seat.driver)?;
    require_mobile(ctx.w, seat.driver)?;
    require_power(ctx.w, seat.driver)?;
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
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
fn lock(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // 16b cut D.2 (`B-03`): a wreck takes no order at all, and the arena's own
    // refusal would be a pulse away. It is the FIRST thing this verb does, so a
    // pilot in a wreck is never answered about which contacts are out there.
    require_alive(ctx.w, seat.driver)?;
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
    let held = last_frame(ctx.w, seat.driver)
        .map(|held| frame_contacts_held(&held.frame, &held.ranges))
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
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Lock { contact })?;
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
///
/// **16b cut V.1**: a word that names no held contact is tried against the
/// ground's named spots second ([`named_target`]), so `RANGE The Ford` and
/// `RANGE a` answer [`spot_range_line`]; a word that is neither is still the one
/// byte-identical [`NO_SUCH_CONTACT`].
fn range(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
    let (spots, cell_cm) = spots_of(ctx.w, match_of(ctx.w, seat.driver, held.as_ref()));
    let line = match named_target(held.as_ref(), &spots, rest)? {
        Target::Contact(frame, row) => range_line(frame, &row, pass_ms(ctx.w)),
        Target::Spot {
            frame,
            letter,
            spot,
        } => spot_range_line(
            &spot,
            letter,
            range_m_rounded(
                spot_range_cm(&frame.pose, &spot, cell_cm),
                SPOT_RANGE_STEP_M,
            ),
        ),
    };
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
///
/// **16b cut B.4** (`T-60`): an unlocked row prints `row.range_m` — the figure the
/// row already carries, which is the HELD figure ([`frame_contacts_held`]) — and
/// does not recompute it. Recomputing from the plot here re-opened the 50 m edge
/// the row's memory closes, so `RANGE` could read `450 m` while `CONTACTS` and
/// both panels held `400 m`; printing the row's own figure is what makes "three
/// surfaces cannot give three distances" true again. Only the lock is recomputed
/// from the plot, at [`lock_range_step_m`] of its age: its finer step is the
/// frame's `lock_range_m`, the same call of the same function, and keeps no memory
/// (16b cut D.4's ruling).
#[must_use]
pub fn range_line(frame: &Frame, row: &ContactRow, pass_ms: i64) -> String {
    let locked = frame_lock(frame) == Some(row.id);
    let metres = if locked {
        frame
            .contacts
            .iter()
            .find(|contact| contact.id == row.id)
            .map_or(row.range_m, |contact| {
                range_m_rounded(contact.range_cm, lock_range_step_m(row.age_ms))
            })
    } else {
        row.range_m
    };
    format!(
        "Range to {}: {metres} m{}.",
        row.designator,
        stale_clause(row.age_ms, pass_ms)
    )
}

/// **`BEARING [<designator>]`** — which way one contact plotted (16b cut Q.1),
/// absolute and off the platform's own nose. The same rules [`range`] follows,
/// the second resolution arm included (16b cut V.1).
fn bearing(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
    let (spots, cell_cm) = spots_of(ctx.w, match_of(ctx.w, seat.driver, held.as_ref()));
    let line = match named_target(held.as_ref(), &spots, rest)? {
        Target::Contact(_, row) => bearing_line(&row, pass_ms(ctx.w)),
        Target::Spot {
            frame,
            letter,
            spot,
        } => {
            let bearing = spot_bearing_mdeg(&frame.pose, &spot, cell_cm);
            spot_bearing_line(
                &spot,
                letter,
                bearing / 1_000,
                relative_mdeg(frame.pose.heading_mdeg, bearing) / 1_000,
            )
        }
    };
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
fn scan(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
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
///   `DAMAGE_COLUMNS`.
/// * **unheld** — never reaches this function: `named_row` answers the one
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
        // sentence rather than as two lists. **16b cut B.4** (`B-26`): the test
        // counts all three characters the cell can cost — the `, ` it is joined
        // with AND the `,` the run takes if it wraps here instead — so no run,
        // comma included, ever passes [`DAMAGE_COLUMNS`] (it was `+ 2`, and a
        // run could print at 77). `section_lines` needs no such margin: it
        // joins with two spaces and pushes no comma on a wrap.
        if !run.is_empty() && run.len() + 3 + cell.len() > DAMAGE_COLUMNS {
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
    let (frame, ranges) = match held {
        Some(held) => (&held.frame, &held.ranges),
        None if named.is_empty() => return Err(CockpitError::NoLockOrName),
        None => return Err(CockpitError::NoSuchContact),
    };
    // 16b cut B.4 (`T-60`): the rows the pilot was told, ranges held.
    let rows = frame_contacts_held(frame, ranges);
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

/// **What `RANGE` or `BEARING` is about** (16b cut V.1): a held contact, as
/// [`named_row`] resolves one, or one of the ground's named spots.
enum Target<'a> {
    /// A contact the platform holds, on the frame that holds it.
    Contact(&'a Frame, ContactRow),
    /// A key spot, with the frame whose pose the range and bearing are read from.
    Spot {
        /// The pilot's last frame.
        frame: &'a Frame,
        /// The spot's marker letter ([`spot_letter`] of its authored index).
        letter: char,
        /// The spot itself.
        spot: KeySpot,
    },
}

/// **The target of a read verb that takes a contact or a spot** (16b cut V.1):
/// `RANGE` and `BEARING` only.
///
/// The word goes to [`resolve_designator`] first and to [`resolve_spot`] second,
/// and a miss on both is [`CockpitError::NoSuchContact`] — **the one
/// byte-identical `No such contact.`**, so `RANGE The Fjord` cannot be told from
/// `RANGE ZZ`, and neither from `RANGE BA` against a `BA` the platform has not
/// found. The two arms cannot collide: a one-character word is never a
/// designator (the shortest is two, `RA` or `X1`), and the load-time name check
/// (`definitions::arena`'s `check_spot_name`) keeps a spot's name from ever
/// having a designator's shape.
///
/// An empty word is the lock, exactly as [`named_row`] reads it. With **no frame
/// held**, a contact word keeps [`CockpitError::NoSuchContact`] (as
/// [`named_row`] answers it: a pilot who could tell an empty scope from a missing
/// one could tell a name that exists from one that does not), and a spot word
/// answers [`CockpitError::NoFrameYet`] — the spot is surveyed ground every
/// pilot knows, and what is missing is the pilot's own position.
///
/// `SCAN` and `LOCK` keep [`named_row`] and [`resolve_designator`] alone: a spot
/// is ground and neither verb means anything about ground.
///
/// # Errors
///
/// [`CockpitError::NoSuchContact`], [`CockpitError::NoLockOrName`] and
/// [`CockpitError::NoFrameYet`], as above.
fn named_target<'a>(
    held: Option<&'a CockpitFrame>,
    spots: &[KeySpot],
    rest: &str,
) -> Result<Target<'a>, CockpitError> {
    let named = rest.trim();
    if named.is_empty() {
        return named_row(held, rest).map(|(frame, row)| Target::Contact(frame, row));
    }
    // The designator arm, first and exactly as `named_row` resolves it.
    if let Some(held) = held {
        let rows = frame_contacts_held(&held.frame, &held.ranges);
        if let Some(row) = resolve_designator(&rows, named)
            .and_then(|id| rows.into_iter().find(|row| row.id == id))
        {
            return Ok(Target::Contact(&held.frame, row));
        }
    }
    // The spot arm. A miss here is the one refusal, whatever the frame.
    let (index, spot) = resolve_spot(spots, named).ok_or(CockpitError::NoSuchContact)?;
    let letter = spot_letter(index).ok_or(CockpitError::NoSuchContact)?;
    let Some(held) = held else {
        return Err(CockpitError::NoFrameYet);
    };
    Ok(Target::Spot {
        frame: &held.frame,
        letter,
        spot: spot.clone(),
    })
}

/// **How old a plot has to be before a read verb says so** (16b cut Q.1): the
/// sensor pass's own period, read the way [`lobby::configs`] reads it.
///
/// A plot older than one pass is a plot the platform did not refresh last time it
/// looked, which is exactly when a pilot needs to know they are shooting at a
/// memory. The sentence is [`stale_clause`]'s; this is the only world read the two
/// read verbs make.
pub(crate) fn pass_ms(w: &World) -> i64 {
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

// ---------------------------------------------------------------------------
// The named ground (16b cut V.1; doc 16 §3.2 "Terrain")
// ---------------------------------------------------------------------------
//
// Doc 16 §3.2 rules the proving ground surveyed: its terrain is known to every
// pilot at check-out, its units are not. The arena pack's `key_spots` are part
// of that ground — The Ford, The Crossroads, North Barn, East Compound and,
// since 16b cut AE.1, The Bluff — and this is where a pilot reads them: a letter on the map ([`SPOT_LETTERS`]), a
// name in the legend, `RANGE`/`BEARING <spot>`, `STATUS`'s nearest spot and
// `VIEW`'s off-window line. Every function here is a pure function of the
// authored spots and the pilot's OWN pose, so nothing in this section can say
// anything about another unit; and ground does not age, so none of it carries
// a stale clause.

/// **The ground's named spots and its cell size** (16b cut V.1), off the match's
/// own definition ([`ArenaRegistry::arena_of`]).
///
/// A clone of at most [`crate::definitions::arena::MAX_KEY_SPOTS`] rows, made
/// only when a verb is typed. A match the registry holds no ground for answers
/// no spots, which every caller reads as "the ground names none".
fn spots_of(w: &World, match_id: Option<MatchId>) -> (Vec<KeySpot>, u16) {
    match_id
        .and_then(|match_id| {
            w.module_state::<ArenaRegistry>()
                .and_then(|registry| registry.arena_of(match_id))
        })
        .map_or((Vec::new(), 1), |ground| {
            (ground.key_spots.clone(), ground.terrain.cell_cm())
        })
}

/// The match a pilot's cockpit reads: the last frame's, else the registry's own
/// binding — which is how a read verb typed before the first frame still knows
/// whose ground it is standing on (16b cut V.1).
fn match_of(w: &World, pilot: ObjId, held: Option<&CockpitFrame>) -> Option<MatchId> {
    held.map(|held| held.match_id).or_else(|| {
        w.module_state::<ArenaRegistry>()
            .and_then(|registry| registry.seat_of_pilot(pilot))
            .map(|(match_id, _)| match_id)
    })
}

/// **The spot a pilot's word names** (16b cut V.1), with its authored index, or
/// `None`.
///
/// Trimmed and ASCII case-folded: the whole name (`the ford`), else a
/// one-character word equal to the spot's letter (`a`). **No prefixes**: `the`
/// is not The Ford, because a prefix that two spots share is a word that names
/// two places. A spot past [`SPOT_LETTERS`] — which the load-time cap makes
/// impossible — is never resolved, by name or otherwise.
///
/// It can never answer a designator's word: a one-character word is shorter
/// than every designator (`RA` and `X1` are two), and the load-time name check
/// (`definitions::arena`'s `check_spot_name`) refuses a name with a designator's
/// shape. So the two arms of `RANGE` and `BEARING` never both match.
#[must_use]
pub fn resolve_spot<'a>(spots: &'a [KeySpot], word: &str) -> Option<(usize, &'a KeySpot)> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let lettered = || spots.iter().zip(SPOT_LETTERS).enumerate();
    if let Some((index, (spot, _))) =
        lettered().find(|(_, (spot, _))| spot.name.eq_ignore_ascii_case(word))
    {
        return Some((index, spot));
    }
    let mut chars = word.chars();
    let (Some(one), None) = (chars.next(), chars.next()) else {
        return None;
    };
    lettered()
        .find(|(_, (_, letter))| letter.eq_ignore_ascii_case(&one))
        .map(|(index, (spot, _))| (index, spot))
}

/// The centre of a spot's cell, arena centimetres: `x × cell_cm + cell_cm / 2`,
/// the same centre a pose placed on that cell stands at.
fn spot_centre_cm(spot: &KeySpot, cell_cm: u16) -> (i64, i64) {
    let cell = i64::from(cell_cm);
    (
        i64::from(spot.x)
            .saturating_mul(cell)
            .saturating_add(cell / 2),
        i64::from(spot.y)
            .saturating_mul(cell)
            .saturating_add(cell / 2),
    )
}

/// **How far a spot is from a pose**, whole centimetres, over the ground (16b
/// cut V.1): 2-D, from the pose to the spot's cell centre, through the arena's
/// one range function ([`crate::arena::index::range_between`]) with both heights
/// at the datum — a spot is a place on the map, not a point in the air.
#[must_use]
pub fn spot_range_cm(pose: &mush_sim::data::Pose, spot: &KeySpot, cell_cm: u16) -> u64 {
    let (sx, sy) = spot_centre_cm(spot, cell_cm);
    crate::arena::index::range_between(pose.x_cm, pose.y_cm, 0, sx, sy, 0)
}

/// **Which way a spot is from a pose**, millidegrees clockwise from north (16b
/// cut V.1), through [`crate::arena::perception::bearing_of`] — the function a
/// contact's bearing is read with — so the cockpit and the arena agree on north.
#[must_use]
pub fn spot_bearing_mdeg(pose: &mush_sim::data::Pose, spot: &KeySpot, cell_cm: u16) -> u32 {
    let (sx, sy) = spot_centre_cm(spot, cell_cm);
    crate::arena::perception::bearing_of(sx.saturating_sub(pose.x_cm), sy.saturating_sub(pose.y_cm))
}

/// **The sentence `RANGE <spot>` says** (16b cut V.1):
/// `Range to The Ford (A): 430 m.` — metres already rounded to
/// [`SPOT_RANGE_STEP_M`]. No stale clause: ground does not age.
#[must_use]
pub fn spot_range_line(spot: &KeySpot, letter: char, metres: u64) -> String {
    format!("Range to {} ({letter}): {metres} m.", spot.name)
}

/// **The sentence `BEARING <spot>` says** (16b cut V.1):
/// `Bearing to The Ford (A): 087° (rel 357°).` — the absolute bearing and the
/// same off the platform's own nose, in whole degrees.
#[must_use]
pub fn spot_bearing_line(
    spot: &KeySpot,
    letter: char,
    bearing_deg: u32,
    relative_deg: u32,
) -> String {
    format!(
        "Bearing to {} ({letter}): {bearing_deg:03}° (rel {relative_deg:03}°).",
        spot.name
    )
}

/// **The spot nearest a pose** (16b cut V.1): its authored index, the spot and
/// its range in centimetres ([`spot_range_cm`]); the least range, ties to the
/// lowest index. `None` when the ground names no spot. A spot past
/// [`SPOT_LETTERS`] is not considered, for the reason [`resolve_spot`] skips it.
#[must_use]
pub fn nearest_spot<'a>(
    spots: &'a [KeySpot],
    pose: &mush_sim::data::Pose,
    cell_cm: u16,
) -> Option<(usize, &'a KeySpot, u64)> {
    let mut best: Option<(usize, &KeySpot, u64)> = None;
    for (index, (spot, _)) in spots.iter().zip(SPOT_LETTERS).enumerate() {
        let range = spot_range_cm(pose, spot, cell_cm);
        // Strictly nearer only, so the first of two equal ranges keeps it.
        if best.is_none_or(|(_, _, held)| range < held) {
            best = Some((index, spot, range));
        }
    }
    best
}

/// **`STATUS`'s nearest-spot line** (16b cut V.1):
/// `  nearest spot: The Ford (A), 120 m at 045°` — the range rounded to
/// [`SPOT_RANGE_STEP_M`] and the absolute bearing. `None` when the ground names
/// no spot, and `STATUS` then prints no line at all.
#[must_use]
pub fn nearest_spot_line(
    spots: &[KeySpot],
    pose: &mush_sim::data::Pose,
    cell_cm: u16,
) -> Option<String> {
    let (index, spot, range) = nearest_spot(spots, pose, cell_cm)?;
    let letter = spot_letter(index)?;
    Some(format!(
        "  nearest spot: {} ({letter}), {} m at {:03}°",
        spot.name,
        range_m_rounded(range, SPOT_RANGE_STEP_M),
        spot_bearing_mdeg(pose, spot, cell_cm) / 1_000
    ))
}

/// **`VIEW`'s line about the named ground its window could not draw** (16b cut
/// V.1): every spot whose index is not in `drawn`, in authored order, as
/// `<letter> <name>, <range> m at <bearing>°`, joined by `; ` and closed with a
/// full stop:
///
/// `Off the window: C North Barn, 1010 m at 045°; E East Compound, 2090 m at 070°.`
///
/// `drawn` is the spots the window placed ([`spot_markers`]' third answer, or
/// read back off a window's `units`) — so a spot hidden under a contact, whose
/// cell IS in the window, is drawn and is not listed. `None` when every spot is
/// drawn or the ground names none.
#[must_use]
pub fn off_window_spots_line(
    spots: &[KeySpot],
    drawn: &[usize],
    pose: &mush_sim::data::Pose,
    cell_cm: u16,
) -> Option<String> {
    // 16b cut AG.1: each clause through `spot_clause`, which the brief's
    // in-window line shares, so the two lines cannot word a spot two ways.
    let parts: Vec<String> = spots
        .iter()
        .zip(SPOT_LETTERS)
        .enumerate()
        .filter(|(index, _)| !drawn.contains(index))
        .map(|(_, (spot, letter))| spot_clause(letter, spot, pose, cell_cm))
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!("Off the window: {}.", parts.join("; ")))
}

/// **One spot as `VIEW`'s spot lines word it** (16b cut V.1's clause, lifted out
/// by 16b cut AG.1): `<letter> <name>, <range> m at <bearing>°` — the range 2-D
/// to the cell centre ([`spot_range_cm`]) rounded to [`SPOT_RANGE_STEP_M`], the
/// bearing absolute ([`spot_bearing_mdeg`]) in whole degrees. The off-window
/// line and the brief's in-window line both call it.
fn spot_clause(letter: char, spot: &KeySpot, pose: &mush_sim::data::Pose, cell_cm: u16) -> String {
    format!(
        "{letter} {}, {} m at {:03}°",
        spot.name,
        range_m_rounded(spot_range_cm(pose, spot, cell_cm), SPOT_RANGE_STEP_M),
        spot_bearing_mdeg(pose, spot, cell_cm) / 1_000
    )
}

// ---------------------------------------------------------------------------
// The brief (16b cut AG.1; 16c N17)
// ---------------------------------------------------------------------------
//
// `VIEW BRIEF` is a SECOND RENDERER OF THE ONE FRAME (doc 16 §5: one feed, and
// every renderer draws the same structure): the payload `VIEW` would push now,
// said as sentences for a screen reader or a pilot who would rather read. It
// reads exactly what the grid draws — the payload's own rectangle from the
// payload's own `@`, the contacts as `CONTACTS` words them, the spots the
// legend names — and says nothing the grid would not draw or name. Every
// function below is pure and world-free; `view`'s brief arm gathers the frame,
// the ground and the payload and calls [`brief_lines`]. Nothing here crosses
// the arena seam, runs per pulse or adds a frame key: the observer rule (doc
// 16 §1) holds by construction, because every input is the pilot's own frame,
// the surveyed ground (doc 16 §3.2) and a pure function of the two.

/// **The eight bearings a brief walks, in compass order** (16b cut AG.1): the
/// word, then the step in cells. `dy` positive is NORTH — arena y grows north,
/// and `terrain_rows`' row 0 is the window's south edge.
pub const BRIEF_BEARINGS: [(&str, i8, i8); 8] = [
    ("North", 0, 1),
    ("North-east", 1, 1),
    ("East", 1, 0),
    ("South-east", 1, -1),
    ("South", 0, -1),
    ("South-west", -1, -1),
    ("West", -1, 0),
    ("North-west", -1, 1),
];

/// **Root two, per mille** (16b cut AG.1): a diagonal step's length in straight
/// steps, to three places — integer, as every figure in the cockpit is.
pub const DIAGONAL_PER_MILLE: u64 = 1_414;

/// **How far `steps` cells reach, centimetres** (16b cut AG.1):
/// `steps × cell_cm`, times [`DIAGONAL_PER_MILLE`] / 1,000 on a diagonal,
/// saturating. At 1,000 cm cells five diagonal steps are 7,070 cm, six 8,484,
/// fifteen 21,210 and nineteen 26,866.
#[must_use]
pub fn ray_cm(steps: u32, cell_cm: u16, diagonal: bool) -> u64 {
    let per_mille = if diagonal { DIAGONAL_PER_MILLE } else { 1_000 };
    u64::from(steps)
        .saturating_mul(u64::from(cell_cm))
        .saturating_mul(per_mille)
        / 1_000
}

/// **The step a brief's reaches are rounded to**, metres (16b cut AG.1):
/// `cell_cm / 100`, clamped to `1 ..= SPOT_RANGE_STEP_M`.
///
/// At the shipped 1,000 cm cell, and every coarser one, that is the spot step,
/// ten metres. Below it the step is the cell's own, because a run is at least
/// one cell long and two runs may never read the same reach: at 500 cm cells a
/// one-cell run is `5 m` and the next `10 m`, where a fixed ten would read both
/// as `10 m`. The one size it cannot serve is 50 cm — the metre floor can merge
/// two adjacent one-cell reaches — which `RESOLUTIONS_CM` admits and no shipped
/// arena uses.
#[must_use]
pub fn brief_step_m(cell_cm: u16) -> u64 {
    (u64::from(cell_cm) / 100).clamp(1, SPOT_RANGE_STEP_M)
}

/// **A terrain class as a brief says it** (16b cut AG.1): prose for a reader —
/// `heavy woods`, `deep water`, `bare ground` — where the legend keeps
/// `TerrainClass::label` (`woods_heavy`), which is a renderer's key to a glyph
/// and not a word anyone reads aloud. This table is the one place the two
/// vocabularies meet, and it is exhaustive: a new class fails to compile here
/// until it is given its words.
#[must_use]
pub fn class_prose(class: mush_atlas::TerrainClass) -> &'static str {
    use mush_atlas::TerrainClass;
    match class {
        TerrainClass::Void => "nothing",
        TerrainClass::Paved => "paving",
        TerrainClass::Ground => "bare ground",
        TerrainClass::Grass => "grass",
        TerrainClass::Water => "water",
        TerrainClass::Floor => "floor",
        TerrainClass::Rubble => "rubble",
        TerrainClass::Road => "road",
        TerrainClass::Sand => "sand",
        TerrainClass::Rock => "rock",
        TerrainClass::Wall => "wall",
        TerrainClass::Door => "doorway",
        TerrainClass::Platform => "platform",
        TerrainClass::WoodsLight => "light woods",
        TerrainClass::WoodsHeavy => "heavy woods",
        TerrainClass::WaterDeep => "deep water",
        TerrainClass::Ice => "ice",
        TerrainClass::Regolith => "regolith",
    }
}

/// **One run of one class along a bearing** (16b cut AG.1). `steps` is the
/// CUMULATIVE count of steps from the pilot's cell at which the run ends, so a
/// run's reach is [`ray_cm`] of it and the first run's reach is its length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroundRun {
    /// The ground class of every cell in the run.
    pub class: mush_atlas::TerrainClass,
    /// The step from the pilot's cell at which the run ends.
    pub steps: u32,
}

/// **What stopped a walk** (16b cut AG.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayEnd {
    /// The next cell is on the arena but outside the window: where the map
    /// stops (`beyond the window`).
    Window,
    /// The next cell is off the arena: where the ground stops (`the edge of the
    /// arena`).
    Arena,
}

/// **The ground along one bearing, run by run** (16b cut AG.1).
///
/// From `from` — the pilot's cell, which is not described — the walk steps
/// `step` one cell at a time (checked `i32` arithmetic, then `u16::try_from`).
/// A cell inside the rectangle `origin.0 .. origin.0 + size.0` by
/// `origin.1 .. origin.1 + size.1` AND on the terrain extends it: its class
/// merges into the last run when equal, else opens a new one. The first cell
/// that is not stops it — [`RayEnd::Arena`] when that cell is off the terrain
/// (negative, or `terrain.get` is `None`), [`RayEnd::Window`] otherwise. A zero
/// step walks nowhere and answers no runs and [`RayEnd::Window`]; no bearing of
/// [`BRIEF_BEARINGS`] is one.
///
/// **The bounds are the clamped window's** ([`view_window`]). A pilot at the
/// centre of a 39-by-20 window walks at most 19 cells east or west, **9 north
/// and 10 south** (`you.dy` is 10: rows 0..=9 lie south of the pilot, 11..=19
/// north). A pilot the clamp put at a window's edge walks the whole span — at
/// most 38 cells straight and 19 diagonal — so a line holds at most 38 runs.
/// And with a `view_window` payload a pilot stands on a window's edge only when
/// that edge is the arena's: the clamp `x0 = (cx − 19).min(W − 39)` leaves `cx`
/// on the window's east edge only at `cx = W − 1` and on its west only at
/// `cx = 0`, and `cy` likewise, so a walk with no runs from production always
/// ends [`RayEnd::Arena`].
#[must_use]
pub fn ground_ray(
    terrain: &ArenaTerrain,
    from: (u16, u16),
    step: (i8, i8),
    origin: (u16, u16),
    size: (u16, u16),
) -> (Vec<GroundRun>, RayEnd) {
    let mut runs: Vec<GroundRun> = Vec::new();
    if step == (0, 0) {
        return (runs, RayEnd::Window);
    }
    let (west, south) = (i32::from(origin.0), i32::from(origin.1));
    let east = west.saturating_add(i32::from(size.0));
    let north = south.saturating_add(i32::from(size.1));
    let (mut x, mut y) = (i32::from(from.0), i32::from(from.1));
    let mut steps: u32 = 0;
    loop {
        let (Some(nx), Some(ny)) = (
            x.checked_add(i32::from(step.0)),
            y.checked_add(i32::from(step.1)),
        ) else {
            return (runs, RayEnd::Arena);
        };
        (x, y) = (nx, ny);
        let cell = match (u16::try_from(x), u16::try_from(y)) {
            (Ok(cx), Ok(cy)) => terrain.get(cx, cy),
            _ => None,
        };
        let Some(cell) = cell else {
            return (runs, RayEnd::Arena);
        };
        if x < west || x >= east || y < south || y >= north {
            return (runs, RayEnd::Window);
        }
        steps = steps.saturating_add(1);
        match runs.last_mut() {
            Some(run) if run.class == cell.class => run.steps = steps,
            _ => runs.push(GroundRun {
                class: cell.class,
                steps,
            }),
        }
    }
}

/// **One bearing's sentence** (16b cut AG.1), verbatim:
///
/// `North: 50 m of grass, then road to 60 m, then grass to 150 m, then beyond
/// the window.`
///
/// The first run states its own length, every later run the reach it ends at,
/// each [`ray_cm`] of its cumulative steps rounded by [`range_m_rounded`] at
/// [`brief_step_m`]; the tail is `then beyond the window.` for
/// [`RayEnd::Window`] (where the map stops) or `then the edge of the arena.`
/// for [`RayEnd::Arena`] (where the ground stops). With no runs at all it is
/// `North: the edge of the arena.` — the one no-runs line a `view_window`
/// payload can produce ([`ground_ray`]) — or `North: beyond the window.`,
/// formatted for completeness over a synthetic rectangle and not one
/// [`view_window`] makes.
#[must_use]
pub fn ground_line(
    bearing: &str,
    runs: &[GroundRun],
    end: RayEnd,
    cell_cm: u16,
    diagonal: bool,
) -> String {
    use std::fmt::Write as _;
    let step = brief_step_m(cell_cm);
    let reach = |run: &GroundRun| range_m_rounded(ray_cm(run.steps, cell_cm, diagonal), step);
    let tail = match end {
        RayEnd::Window => "beyond the window.",
        RayEnd::Arena => "the edge of the arena.",
    };
    let Some((first, later)) = runs.split_first() else {
        return format!("{bearing}: {tail}");
    };
    let mut line = format!(
        "{bearing}: {} m of {}",
        reach(first),
        class_prose(first.class)
    );
    for run in later {
        // Writing to a `String` cannot fail.
        let _ = write!(
            line,
            ", then {} to {} m",
            class_prose(run.class),
            reach(run)
        );
    }
    let _ = write!(line, ", then {tail}");
    line
}

/// **One contact as the brief says it** (16b cut AG.1), verbatim:
///
/// `BA, blue: bearing 045° (rel 000°), far, 450 m, 1 s old.`
///
/// The row's own fields and nothing else — the designator, the class word
/// (`unknown` or the side), the bearing absolute and off the nose, the band,
/// the range and the age in whole seconds — the very figures
/// [`ContactRow::render`] prints for `CONTACTS`, in a sentence. No lock mark, no
/// friendly mark, no condition, because `CONTACTS` prints none of them.
#[must_use]
pub fn contact_sentence(row: &ContactRow) -> String {
    format!(
        "{}, {}: bearing {:03}° (rel {:03}°), {}, {} m, {} s old.",
        row.designator,
        row.class,
        row.bearing_deg,
        row.relative_deg,
        row.band,
        row.range_m,
        row.age_ms / 1_000
    )
}

/// **The named spots the window shows** (16b cut AG.1), the mirror of
/// [`off_window_spots_line`]: every authored index in `named`, in AUTHORED
/// order (not `named`'s), each through the off-window line's own clause, joined
/// by `; `:
///
/// `In the window: A The Ford, 190 m at 093°.`
///
/// `named` is the spots the window's LEGEND names — `view_window`'s owner rule,
/// the letters a line client prints as `legend: … A The Ford` — and not every
/// drawn spot: a spot drawn and then covered by a contact or the `@` is named
/// in no legend, so the brief does not name it either (the brief says nothing
/// the grid's legend does not; 16b cut AG.1's Q-AG2 default). Such a spot
/// still reaches the pilot through `STATUS`'s nearest-spot line and
/// `RANGE <spot>`. `None` when nothing is named.
#[must_use]
pub fn in_window_spots_line(
    spots: &[KeySpot],
    named: &[usize],
    pose: &mush_sim::data::Pose,
    cell_cm: u16,
) -> Option<String> {
    let parts: Vec<String> = spots
        .iter()
        .zip(SPOT_LETTERS)
        .enumerate()
        .filter(|(index, _)| named.contains(index))
        .map(|(_, (spot, letter))| spot_clause(letter, spot, pose, cell_cm))
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!("In the window: {}.", parts.join("; ")))
}

/// **Where the window is, in one sentence** (16b cut AG.1), verbatim:
///
/// `Window around RA: cells 0 to 38 east and 0 to 19 north of 96 by 64; you at
/// (6, 4), heading 045°; 10 m a cell.`
///
/// Read off the payload [`view_window`] wrote, never recomputed: `origin_cell`
/// is the window's corner, `width` and `height` its size (the far cell is
/// `corner + size − 1`, saturating), `extent` the arena's size, `origin_cell +
/// you` the pilot's own cell, `cell_cm / 100` the metres a cell. These are the
/// grid footer's facts (`mush_ansi::panel`'s `grid_lines`) in words on a
/// terminal of at least 80 by 24; below that the line client clips the grid
/// and prints the clipped viewport's footer, and the payload — so this
/// header — is unclipped. The callsign and the heading are the frame's two
/// facts, taken as primitives so a test can pin the sentence without a frame.
/// A missing key reads as `0`; never a panic.
#[must_use]
pub fn brief_header(callsign: &str, heading_mdeg: u32, window: &serde_json::Value) -> String {
    let (x0, y0) = window_pair(window, "origin_cell");
    let (dx, dy) = window_pair(window, "you");
    let (arena_w, arena_h) = window_pair(window, "extent");
    let width = window_number(window, "width");
    let height = window_number(window, "height");
    let metres = window_number(window, "cell_cm") / 100;
    format!(
        "Window around {callsign}: cells {x0} to {} east and {y0} to {} north of {arena_w} by \
         {arena_h}; you at ({}, {}), heading {:03}°; {metres} m a cell.",
        x0.saturating_add(width).saturating_sub(1),
        y0.saturating_add(height).saturating_sub(1),
        x0.saturating_add(dx),
        y0.saturating_add(dy),
        heading_mdeg / 1_000
    )
}

/// One whole-number key of a window payload, `0` when it is missing.
fn window_number(window: &serde_json::Value, key: &str) -> u64 {
    window
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

/// One `[a, b]` pair of a window payload, each `0` when missing.
fn window_pair(window: &serde_json::Value, key: &str) -> (u64, u64) {
    let pair = window.get(key).and_then(serde_json::Value::as_array);
    let at = |n: usize| {
        pair.and_then(|pair| pair.get(n))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    (at(0), at(1))
}

/// **The whole brief** (16b cut AG.1; 16c N17): the payload `VIEW` would push
/// now, said as sentences, in this order —
///
/// 1. [`brief_header`];
/// 2. the eight [`ground_line`]s over [`BRIEF_BEARINGS`], walked by
///    [`ground_ray`] from the payload's own `origin_cell + you` — the cell the
///    grid draws the `@` on, never `terrain.cell_of(&frame.pose)`, which is
///    [`view_window`]'s business — to the payload's own `origin_cell`, `width`
///    and `height`, a bearing diagonal when both its steps are non-zero;
/// 3. one [`contact_sentence`] per row of [`frame_contacts_held`] in frame
///    order, or [`NO_CONTACTS_IN_WINDOW`] when there are none — the brief's
///    own sentence since 16b cut AP.4, so a reader never takes it for a
///    `CONTACTS` reply's `No contacts.`;
/// 4. [`beyond_window_line`], when it says anything;
/// 5. [`in_window_spots_line`] over the spots the payload's legend names;
/// 6. [`off_window_spots_line`] over the spots the payload DREW, as `VIEW`
///    reads it — so a covered spot is in neither spot line, as it is in
///    neither of `VIEW`'s;
/// 7. [`BRIEF_END_LINE`], always, last (16b cut AP.4, 16c `N20`): the
///    brief's fixed close, so a script knows where it ends.
///
/// **Nothing else, ever.** The brief describes the payload's own rectangle
/// from the payload's own `@`, names the legend's own spots, and says nothing
/// the grid would not draw or name; the ground under a marker is surveyed
/// ground the pulled `arena.map` carries, said where the grid draws the glyph.
/// Every sentence is a function of the pilot's own frame, the surveyed ground
/// and the payload, itself a pure function of those two — the observer rule
/// (doc 16 §1) by construction. A payload missing a key reads it as `0` and
/// lets the walk answer; the shape test in `VIEW` refuses such a payload
/// before it gets here.
///
/// **Length**: at most `13 + C` lines for `C` contacts — one header, eight
/// bearings, `max(C, 1)` contact lines, at most one each of the beyond,
/// in-window and off-window lines, and the end line; thirteen at `C = 0`. A
/// platform holds at most one contact per other unit, so the centre's six a
/// side bounds a brief at 24 lines and 24 `Sys` events. **Width**: none. A brief is `Sys` prose that each
/// renderer wraps at its own width; Q-I1's 78-column budget is the grid's, and
/// nothing here is laid out to it. The longest line — 38 alternating one-cell
/// runs from a window's edge — is about 800 characters and wraps as prose.
#[must_use]
pub fn brief_lines(
    frame: &Frame,
    terrain: &ArenaTerrain,
    spots: &[KeySpot],
    window: &serde_json::Value,
    ranges: &BTreeMap<ContactId, u64>,
) -> Vec<String> {
    let cell_cm = terrain.cell_cm();
    let cells = |value: u64| u16::try_from(value).unwrap_or(u16::MAX);
    let (x0, y0) = window_pair(window, "origin_cell");
    let (dx, dy) = window_pair(window, "you");
    let origin = (cells(x0), cells(y0));
    let size = (
        cells(window_number(window, "width")),
        cells(window_number(window, "height")),
    );
    let you = (cells(x0.saturating_add(dx)), cells(y0.saturating_add(dy)));
    let mut lines = vec![brief_header(
        &frame.callsign.to_string(),
        frame.pose.heading_mdeg,
        window,
    )];
    for (bearing, step_x, step_y) in BRIEF_BEARINGS {
        let (runs, end) = ground_ray(terrain, you, (step_x, step_y), origin, size);
        lines.push(ground_line(
            bearing,
            &runs,
            end,
            cell_cm,
            step_x != 0 && step_y != 0,
        ));
    }
    let rows = frame_contacts_held(frame, ranges);
    if rows.is_empty() {
        lines.push(NO_CONTACTS_IN_WINDOW.to_owned());
    }
    lines.extend(rows.iter().map(contact_sentence));
    lines.extend(beyond_window_line(frame, window));
    lines.extend(in_window_spots_line(
        spots,
        &named_spots(window),
        &frame.pose,
        cell_cm,
    ));
    lines.extend(off_window_spots_line(
        spots,
        &drawn_spots(window),
        &frame.pose,
        cell_cm,
    ));
    lines.push(BRIEF_END_LINE.to_owned());
    lines
}

/// **The spot markers a window carries, the legend entries they would earn, and
/// which spots they are** (16b cut V.1), clipped to the window `origin` and
/// `size` exactly as [`contact_markers`] clips, with cells relative to the
/// window's own corner.
///
/// One marker is `{ "id": "A", "name": "The Ford", "cell": [dx, dy], "glyph":
/// "A", "you": false, "spot": true }`: `id` is display text on the grid shape,
/// as a contact marker's is, `name` the authored name, and `spot: true` is what
/// tells it from a contact. One legend entry is `["A", "The Ford"]`; whether it
/// is published is [`view_window`]'s call, after every marker is placed (the
/// owner rule). The third answer is the authored indexes drawn, in order.
///
/// Surveyed ground and nothing else: a spot names no unit.
#[must_use]
pub fn spot_markers(
    spots: &[KeySpot],
    origin: (u16, u16),
    size: (u16, u16),
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>, Vec<usize>) {
    let (x0, y0) = origin;
    let (width, height) = size;
    let mut markers: Vec<serde_json::Value> = Vec::new();
    let mut legend: Vec<serde_json::Value> = Vec::new();
    let mut drawn: Vec<usize> = Vec::new();
    for (index, (spot, letter)) in spots.iter().zip(SPOT_LETTERS).enumerate() {
        if spot.x < x0 || spot.y < y0 {
            continue;
        }
        let (dx, dy) = (spot.x.saturating_sub(x0), spot.y.saturating_sub(y0));
        if dx >= width || dy >= height {
            continue;
        }
        let letter = letter.to_string();
        markers.push(serde_json::json!({
            "id": letter,
            "name": spot.name.as_str(),
            "cell": [u32::from(dx), u32::from(dy)],
            "glyph": letter,
            "you": false,
            "spot": true,
        }));
        legend.push(serde_json::json!([letter, spot.name.as_str()]));
        drawn.push(index);
    }
    (markers, legend, drawn)
}

/// **The spots a composed window drew** (16b cut V.1), read back off its
/// `units` — the markers with `spot: true`, by their letter — so `VIEW`'s
/// off-window line counts the drawing's own spots and cannot drift from it, the
/// way [`beyond_window_line`] counts its contacts.
fn drawn_spots(window: &serde_json::Value) -> Vec<usize> {
    spot_marker_walk(window)
        .into_iter()
        .map(|(index, _)| index)
        .collect()
}

/// **The spots a composed window NAMED** (16b cut AG.1): [`drawn_spots`]' walk
/// with one more filter — the markers whose `[id, name]` pair is an entry of the
/// payload's `legend`, which is exactly the spots [`view_window`]'s owner rule
/// named and a line client prints as `legend: … A The Ford`. A spot drawn and
/// then covered by a later marker is in [`drawn_spots`] and NOT here.
///
/// The answer is **authored indexes**, by the marker's letter, never a marker's
/// position in `units`: [`spot_markers`] pushes only the in-window spots, so
/// near The Crossroads on the shipped ground `B` is `units[0]` and authored
/// index 1.
fn named_spots(window: &serde_json::Value) -> Vec<usize> {
    let legend = window
        .get("legend")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    spot_marker_walk(window)
        .into_iter()
        .filter(|(_, marker)| match (marker.get("id"), marker.get("name")) {
            (Some(id), Some(name)) => legend.contains(&serde_json::json!([id, name])),
            _ => false,
        })
        .map(|(index, _)| index)
        .collect()
}

/// The one walk [`drawn_spots`] and [`named_spots`] share: every `spot: true`
/// marker of a window's `units`, with the authored index its letter maps to
/// through [`SPOT_LETTERS`]; a marker whose `id` is no spot letter is skipped.
fn spot_marker_walk(window: &serde_json::Value) -> Vec<(usize, &serde_json::Value)> {
    window
        .get("units")
        .and_then(serde_json::Value::as_array)
        .map(|units| {
            units
                .iter()
                .filter(|unit| unit.get("spot").and_then(serde_json::Value::as_bool) == Some(true))
                .filter_map(|unit| {
                    let letter = unit
                        .get("id")
                        .and_then(serde_json::Value::as_str)?
                        .chars()
                        .next()?;
                    let index = SPOT_LETTERS.iter().position(|held| *held == letter)?;
                    Some((index, unit))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// **What a `FIRE` line asks for** (17b cut M1b.4), read by [`fire_form`] off
/// its first two words alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FireForm<'a> {
    /// `FIRE [<weapon>]`: the shot, every byte as it was before this cut.
    Shot,
    /// `FIRE <weapon|ALL> OFF`, and bare `FIRE OFF`: `None` clears every slot.
    Off {
        /// The weapon word as typed, or `None` for `ALL` and bare `OFF`.
        weapon: Option<&'a str>,
        /// The words after `OFF`, trimmed (refused unless empty).
        tail: &'a str,
    },
    /// `FIRE <weapon> WHEN <conditions>`.
    When {
        /// The weapon word as typed.
        weapon: &'a str,
        /// Everything after `WHEN`, trimmed.
        conditions: &'a str,
    },
}

/// One whitespace-delimited word off the front of `text`, and the rest with
/// its leading whitespace gone.
fn first_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.find(char::is_whitespace) {
        Some(at) => (
            text.get(..at).unwrap_or_default(),
            text.get(at..).unwrap_or_default().trim_start(),
        ),
        None => (text, ""),
    }
}

/// **Which of `FIRE`'s three forms a line is** (17b cut M1b.4): the second
/// word, case-folded, `when` or `off` — or the whole line `off`. Every other
/// line — `FIRE`, `FIRE mg`, `FIRE all`, `FIRE when odds` — is
/// [`FireForm::Shot`] and goes to `choose_weapon` whole, exactly as before.
fn fire_form(rest: &str) -> FireForm<'_> {
    let (first, after) = first_word(rest);
    let (second, tail) = first_word(after);
    if first.eq_ignore_ascii_case("off") && after.is_empty() {
        return FireForm::Off {
            weapon: None,
            tail: "",
        };
    }
    if first.is_empty() || second.is_empty() {
        return FireForm::Shot;
    }
    if second.eq_ignore_ascii_case("off") {
        let weapon = (!first.eq_ignore_ascii_case("all")).then_some(first);
        return FireForm::Off {
            weapon,
            tail: tail.trim(),
        };
    }
    if second.eq_ignore_ascii_case("when") {
        return FireForm::When {
            weapon: first,
            conditions: tail.trim(),
        };
    }
    FireForm::Shot
}

/// **`FIRE ALL OFF`'s reply** (17b cut M1b.4), and bare `FIRE OFF`'s: said
/// whether or not a slot stood.
pub const CLEARED_ALL: &str = "Cleared: your guns fire only when you say.";

/// **`FIRE <weapon|ALL> OFF`** (17b cut M1b.4; doc 17 §5's precedence: *a
/// cleared weapon fires only when you say*): one discrete
/// `IntentKind::Condition(Arming::Disarm { weapon })` on the fire channel.
///
/// The wreck first, as every order; the weapon through `choose_weapon` with
/// its own refusals; anything typed after `OFF` is the one outside refusal.
/// **The reply is unconditional and reads no mirror**: a reply computed off a
/// mirror up to one `Quiet` pulse old lied when an arming and an `OFF` fell
/// inside one pulse, and a disarm of nothing is a no-op in the arena — so
/// `Cleared: cannon fires only when you say.` / [`CLEARED_ALL`] whether or not
/// a slot stood. Power is not asked: clearing a slot is not a shot.
fn fire_off(
    ctx: &mut Ctx<'_>,
    seat: &Seat,
    weapon: Option<&str>,
    tail: &str,
) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    require_alive(ctx.w, seat.driver)?;
    let (named, line) = match weapon {
        Some(word) => {
            let held = last_frame(ctx.w, seat.driver);
            let rows = carried(ctx.w, seat.unit, held.as_ref(), std::time::Instant::now());
            let row = choose_weapon(&rows, word)?;
            if !tail.is_empty() {
                return Err(fire_refused(&row.id, &CockpitError::NotThisCore));
            }
            let line = format!("Cleared: {} fires only when you say.", row.id);
            (Some(row.id), line)
        }
        None => {
            if !tail.is_empty() {
                return Err(CockpitError::Refused(order_refused(
                    IntentKind::fire("").as_str(),
                    None,
                    &CockpitError::NotThisCore.to_string(),
                )));
            }
            (None, CLEARED_ALL.to_owned())
        }
    };
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
        IntentKind::Condition(Arming::Disarm { weapon: named }),
    )?;
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **`FIRE <weapon> WHEN <condition> [AND <condition>]…`** (17b cut M1b.4;
/// doc 17 §5, D-UM-17 as amended, D-UM-18's slot-fitting subset): the
/// pilot's shot decision written down as one slot.
///
/// In order: the wreck; the weapon through `choose_weapon` (its own
/// refusals); the held frame ([`last_frame`] — with none there is no floor to
/// read, so the answer is the one a typed `FIRE` gives on a mirror holding no
/// rows, `FIRE cannon refused. No lock: LOCK a contact first.`,
/// [`CockpitError::NoLock`] through `fire_refused`: a slot fires only your
/// lock, and before the first frame there is none); then [`parse_arming`] over
/// the text after `WHEN` (the one outside refusal, and the floor checked off
/// the frame's captured `odds_floor_permille` — refused
/// [`CockpitError::Unlawful`] **as typed**, the arena's `Unlawful` at the
/// landing the byte-identical backstop). Every refusal composes through
/// `fire_refused`, so it reads `FIRE cannon refused. …` — exactly the sentence
/// `refusal_line` prints for the arena's word.
///
/// **No count pre-check** (17b cut M2.2, the lead's ruling (c)): the arena's
/// phase-4 `NoSlot` is the one wall for a full table. An arming past
/// [`crate::arena::conditions::STOCK_CONDITION_SLOTS`] armed weapons is
/// accepted here — its sentence printed at once — and refused by the arena a
/// pulse later as the notice
/// `FIRE missiles refused. Every slot is armed; FIRE <weapon> OFF frees one.`
/// (the declared effect; `help cockpit` says so). M1b.4 refused it here off
/// the mirror, which a `FIRE <weapon> OFF` and a new arming typed inside one
/// pulse read wrongly.
///
/// Then ONE discrete `IntentKind::Condition(Arming::Arm(cond))` — the seat
/// window and the queue bound as `LOCK`, resolved at the pulse's end in
/// arrival order — with no power pre-check (a slot may be armed cold), and the
/// English written back once by [`arming_reply`] off the TYPED figures
/// ([`ArmingEcho`]), with **no slot number**: the number is the arena's to
/// give at phase 4, and shows on the next frame's `armed`, in
/// `WEAPONSTATUS`'s arming line and in every attribution.
fn fire_when(
    ctx: &mut Ctx<'_>,
    seat: &Seat,
    word: &str,
    conditions: &str,
) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    require_alive(ctx.w, seat.driver)?;
    let held = last_frame(ctx.w, seat.driver);
    let rows = carried(ctx.w, seat.unit, held.as_ref(), std::time::Instant::now());
    let weapon = choose_weapon(&rows, word)?;
    let refused = |said: &CockpitError| fire_refused(&weapon.id, said);
    // The same answer the typed `FIRE` gives before the first frame: with no
    // frame held there is no lock, so `NoLock`, in `FIRE`'s own bytes.
    let Some(held) = held else {
        return Err(refused(&CockpitError::NoLock));
    };
    let (cond, echo) =
        parse_arming(&weapon, conditions, &held.frame).map_err(|said| refused(&said))?;
    let line = arming_reply(&weapon.id, &echo);
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
        IntentKind::Condition(Arming::Arm(cond)),
    )?;
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// The most conditions one `FIRE … WHEN` rule joins with `AND` (doc 17 §5).
pub const MAX_ARMING_TERMS: usize = 4;

/// The furthest a range term may name, metres.
pub const MAX_ARMING_RANGE_M: u32 = 10_000;

/// **A rule's text as words** (17b cut M1b.4): case-folded, split on
/// whitespace, and `>=`, `<`, `>`, `=` and `,` made words of their own
/// whatever stands round them, so `odds>=60` and `odds >= 60` and
/// `aspect = side,rear` read alike.
fn arming_tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let operator = match c {
            '>' if chars.peek() == Some(&'=') => {
                chars.next();
                Some(">=")
            }
            '>' => Some(">"),
            '<' => Some("<"),
            '=' => Some("="),
            ',' => Some(","),
            _ => None,
        };
        if c.is_whitespace() || operator.is_some() {
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
            if let Some(operator) = operator {
                tokens.push(operator.to_owned());
            }
            continue;
        }
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    tokens
}

/// A whole number of digits only, inside `range`, or the outside refusal.
fn arming_number(text: &str, range: std::ops::RangeInclusive<u32>) -> Result<u32, CockpitError> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CockpitError::NotThisCore);
    }
    text.parse::<u32>()
        .ok()
        .filter(|n| range.contains(n))
        .ok_or(CockpitError::NotThisCore)
}

/// A range term's figure: `600` or `600m`, 1 to [`MAX_ARMING_RANGE_M`].
fn arming_metres(text: &str) -> Result<u32, CockpitError> {
    arming_number(
        text.strip_suffix('m').unwrap_or(text),
        1..=MAX_ARMING_RANGE_M,
    )
}

/// Set one term of a rule, refusing a second value that differs from the
/// first (the same value twice is the same rule).
fn arming_set<T: PartialEq>(slot: &mut Option<T>, value: T) -> Result<(), CockpitError> {
    match slot {
        Some(held) if *held != value => Err(CockpitError::NotThisCore),
        _ => {
            *slot = Some(value);
            Ok(())
        }
    }
}

/// The terms of one rule as typed, before they are compiled.
#[derive(Default)]
struct ArmingTerms {
    /// `odds >= n`, whole per cent.
    odds_pct: Option<u32>,
    /// `aspect = …`, as a mask.
    aspects: Option<u8>,
    /// `rounds > n` / `rounds > n%` / `reserve`, in rounds.
    keeps: Option<u32>,
    /// The nearest a `range >` or a `band` bounds it to, compiled: a `range >
    /// n` stores `n + 1` (17b cut M2.2, ruling (c)).
    min_m: Option<u32>,
    /// The furthest a `range <` or a `band` bounds it to, compiled: a `range <
    /// n` stores `n - 1` (17b cut M2.2, ruling (c)).
    max_m: Option<u32>,
    /// `range < n`'s `n` as typed (the reply's figure).
    under_m: Option<u32>,
    /// `range > n`'s `n` as typed (the reply's figure).
    past_m: Option<u32>,
    /// `band = …`'s metres, `(nearest, furthest)`.
    band: Option<(u32, u32)>,
}

/// **What a `FIRE … WHEN` rule said, as typed** (17b cut M2.2, the lead's
/// ruling (c)): the figures [`arming_reply`] writes back, read off the
/// pilot's own words rather than off the compiled [`FireCondition`] — whose
/// range terms are strict (`range < 600` stores `max_range_m 599`), so a
/// reply read off it would print a figure the pilot never typed. Every M1b.4
/// reply is byte-identical through it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmingEcho {
    /// The chance asked for, per mille: the odds term's, else the frame's
    /// floor (always `Some` from [`parse_arming`]).
    pub min_permille: Option<u16>,
    /// The `aspect = …` mask, when typed.
    pub aspects: Option<u8>,
    /// `range < n`'s `n`, metres.
    pub under_m: Option<u32>,
    /// `range > n`'s `n`, metres.
    pub past_m: Option<u32>,
    /// `band = …`'s `(nearest, furthest)` metres.
    pub band: Option<(u32, u32)>,
    /// The rounds a `rounds > …` or `reserve` term keeps.
    pub keeps: Option<u32>,
}

/// **Compile a `FIRE <weapon> WHEN …` rule** (17b cut M1b.4; D-UM-18's
/// slot-fitting subset — the rest of the rule language is 17b cut M5's) into
/// the [`FireCondition`] the arena arms, off the weapon's row and the held
/// frame. Pure: it reads nothing else and forwards nothing.
///
/// `rest` is the text after `WHEN`, split on `AND` (case-folded) into at most
/// [`MAX_ARMING_TERMS`] conditions, no `OR`; each condition is one of the
/// eight terms, case-folded and whitespace-tolerant around `>=`, `<`, `>`,
/// `=` and `,`:
///
/// * `odds >= n`, or `odds <weapon> >= n` naming THIS weapon (by id,
///   initials or prefix, as `FIRE` resolves one), `n` a whole number 1 to 100
///   → `min_permille = 10 n`. **The floor**: `10 n` under the frame's
///   captured `odds_floor_permille` is [`CockpitError::Unlawful`]
///   (refuse-not-clamp: `The Compact floor is 10%; a condition may not ask for
///   less.`). There is no upper refusal: 100 is lawful and simply never holds
///   under the table's `p_max`;
/// * `aspect = a[,b[,c]]` with `front`, `side` (left and right) and `rear` →
///   the mask (a word twice is fine);
/// * `range < n [m]` → `max_range_m = n - 1`; `range > n [m]` →
///   `min_range_m = n + 1` (17b cut M2.2, the lead's ruling (c): STRICTLY under
///   and over, where M1b.4 stored `n` and the warrant's inclusive bounds let
///   a printed `600 m` fire under `range < 600`); `n` 1 to
///   [`MAX_ARMING_RANGE_M`] (`range < 1` stores 0), tested by the arena
///   against the lock's ROUNDED range — the figure `RANGE` prints. **A
///   declared effect**: a term holds one figure, so `range < 200 AND band =
///   close` (199 against the band's 200) is refused as a differing duplicate,
///   where M1b.4 read both as 200;
/// * `band = close|effective|far` → the metres [`band_bounds_m`] cuts for this
///   weapon's bands on this frame's sight: compiled to FIXED metres here, which
///   the reply prints (a weapon with no bands has no band, and is outside);
/// * `rounds > n` (0 to the weapon's full load) → `min_rounds_left = n`;
///   `rounds > n%` (0 to 100) → `rounds_max × n / 100`, rounded down;
/// * `reserve` → `min_rounds_left = 0` on a piloted hull (the cockpit reads no
///   card before 17b cut M5): it compiles to `rounds > 0`;
/// * `target = identified` → accepted, and adds nothing: every slot fires only
///   a lock its platform has classified;
/// * `lock` → the lock, the default and the only target word here.
///
/// Anything else — another word, `OR`, a fifth condition, a malformed or
/// missing number, an empty rule, a term repeated with another value — is the
/// ONE outside said-half [`CockpitError::NotThisCore`]. The grammar is read
/// whole before the floor is checked, so a rule both malformed and under the
/// floor is told it is malformed.
///
/// The compiled slot: `target: Lock`; `min_permille` the odds term's, else
/// **the frame's floor** (a rule with no odds term asks for the floor);
/// `aspects` [`ASPECTS_ANY`] unless named; `min_rounds_left` 0, `min_range_m`
/// 0 and `max_range_m` `u32::MAX` (unbound) unless named.
///
/// **17b cut M2.2**: with the slot, the [`ArmingEcho`] of the rule as typed —
/// what [`arming_reply`] writes back.
///
/// # Errors
///
/// [`CockpitError::NotThisCore`] or [`CockpitError::Unlawful`] — the
/// said-half; the caller adds `FIRE <weapon> refused.`.
pub fn parse_arming(
    weapon: &WeaponRow,
    rest: &str,
    frame: &Frame,
) -> Result<(FireCondition, ArmingEcho), CockpitError> {
    let tokens = arming_tokens(rest);
    if tokens.is_empty() || tokens.iter().any(|token| token == "or") {
        return Err(CockpitError::NotThisCore);
    }
    let conditions: Vec<Vec<&str>> = tokens
        .split(|token| token == "and")
        .map(|term| term.iter().map(String::as_str).collect())
        .collect();
    if conditions.len() > MAX_ARMING_TERMS || conditions.iter().any(Vec::is_empty) {
        return Err(CockpitError::NotThisCore);
    }
    let mut terms = ArmingTerms::default();
    for condition in &conditions {
        arming_term(condition, weapon, frame, &mut terms)?;
    }
    let floor = frame.odds_floor_permille;
    let min_permille = match terms.odds_pct {
        Some(pct) if pct.saturating_mul(10) < floor => {
            return Err(CockpitError::Unlawful {
                floor_pct: floor / 10,
            });
        }
        Some(pct) => pct.saturating_mul(10),
        None => floor,
    };
    let min_permille = u16::try_from(min_permille).unwrap_or(u16::MAX);
    let cond = FireCondition {
        weapon: weapon.id.clone(),
        target: SlotTarget::Lock,
        min_permille,
        aspects: terms.aspects.unwrap_or(ASPECTS_ANY),
        min_rounds_left: terms.keeps.unwrap_or(0),
        min_range_m: terms.min_m.unwrap_or(0),
        max_range_m: terms.max_m.unwrap_or(u32::MAX),
    };
    let echo = ArmingEcho {
        min_permille: Some(min_permille),
        aspects: terms.aspects,
        under_m: terms.under_m,
        past_m: terms.past_m,
        band: terms.band,
        keeps: terms.keeps,
    };
    Ok((cond, echo))
}

/// One condition of a rule into `terms` ([`parse_arming`]'s eight).
fn arming_term(
    condition: &[&str],
    weapon: &WeaponRow,
    frame: &Frame,
    terms: &mut ArmingTerms,
) -> Result<(), CockpitError> {
    match condition {
        ["odds", ">=", n] => arming_set(&mut terms.odds_pct, arming_number(n, 1..=100)?),
        ["odds", gun, ">=", n] => {
            // The gun named must be this slot's own, by any spelling `FIRE`
            // takes for it.
            let named =
                choose_weapon(&frame_weapons(frame), gun).is_ok_and(|row| row.id == weapon.id);
            if !named {
                return Err(CockpitError::NotThisCore);
            }
            arming_set(&mut terms.odds_pct, arming_number(n, 1..=100)?)
        }
        ["aspect", "=", list @ ..] => {
            if list.is_empty() || list.len() % 2 == 0 {
                return Err(CockpitError::NotThisCore);
            }
            let mut faces: Vec<Aspect> = Vec::new();
            for (at, word) in list.iter().enumerate() {
                if at % 2 == 1 {
                    if *word != "," {
                        return Err(CockpitError::NotThisCore);
                    }
                    continue;
                }
                match *word {
                    "front" => faces.push(Aspect::Front),
                    "side" => faces.extend([Aspect::Left, Aspect::Right]),
                    "rear" => faces.push(Aspect::Rear),
                    _ => return Err(CockpitError::NotThisCore),
                }
            }
            arming_set(&mut terms.aspects, aspect_mask(&faces))
        }
        // 17b cut M2.2, the lead's ruling (c): STRICT — `< n` stores
        // `max_range_m = n - 1` and `> n` stores `min_range_m = n + 1`, so the
        // warrant's inclusive bounds fire strictly under and over `n` at the
        // rounded range (a printed `600 m` is OUTSIDE `range < 600`). The
        // typed `n` is kept for the reply.
        ["range", "<", n] | ["range", "<", n, "m"] => {
            let typed = arming_metres(n)?;
            arming_set(&mut terms.under_m, typed)?;
            arming_set(&mut terms.max_m, typed.saturating_sub(1))
        }
        ["range", ">", n] | ["range", ">", n, "m"] => {
            let typed = arming_metres(n)?;
            arming_set(&mut terms.past_m, typed)?;
            arming_set(&mut terms.min_m, typed.saturating_add(1))
        }
        ["band", "=", word] => {
            let bands: &[u32] = frame
                .weapons
                .iter()
                .find(|row| row.id == weapon.id)
                .map_or(&[], |row| row.bands_cm.as_slice());
            let Some([close, effective, far]) = band_bounds_m(frame.sight_cm, bands) else {
                return Err(CockpitError::NotThisCore);
            };
            let (nearest, furthest) = match *word {
                "close" => close,
                "effective" => effective,
                "far" => far,
                _ => return Err(CockpitError::NotThisCore),
            };
            arming_set(&mut terms.band, (nearest, furthest))?;
            // The nearest of `close` is nought, which is no bound at all.
            if nearest > 0 {
                arming_set(&mut terms.min_m, nearest)?;
            }
            arming_set(&mut terms.max_m, furthest)
        }
        ["rounds", ">", n, "%"] => {
            let pct = arming_number(n, 0..=100)?;
            arming_set(&mut terms.keeps, rounds_kept(weapon, pct))
        }
        ["rounds", ">", n] => match n.strip_suffix('%') {
            Some(pct) => {
                let pct = arming_number(pct, 0..=100)?;
                arming_set(&mut terms.keeps, rounds_kept(weapon, pct))
            }
            None => {
                let full = u32::try_from(weapon.rounds_max.max(0)).unwrap_or(u32::MAX);
                arming_set(&mut terms.keeps, arming_number(n, 0..=full)?)
            }
        },
        ["reserve"] => arming_set(&mut terms.keeps, 0),
        ["target", "=", "identified"] | ["lock"] => Ok(()),
        _ => Err(CockpitError::NotThisCore),
    }
}

/// `pct` per cent of the weapon's full load, rounded down.
fn rounds_kept(weapon: &WeaponRow, pct: u32) -> u32 {
    let full = u64::try_from(weapon.rounds_max.max(0)).unwrap_or(0);
    u32::try_from(full.saturating_mul(u64::from(pct)) / 100).unwrap_or(u32::MAX)
}

/// **The side words a mask names** (17b cut M1b.3's rule, lifted by 17b cut
/// M1b.4 so the arming line and the arming's reply say one thing): `front`,
/// `side` (left or right), `rear`, in that order, each at most once.
fn aspect_side_words(aspects: u8) -> Vec<&'static str> {
    let side = aspect_in(aspects, Aspect::Left) || aspect_in(aspects, Aspect::Right);
    [
        (aspect_in(aspects, Aspect::Front), "front"),
        (side, "side"),
        (aspect_in(aspects, Aspect::Rear), "rear"),
    ]
    .into_iter()
    .filter_map(|(on, word)| on.then_some(word))
    .collect()
}

/// **The English `FIRE … WHEN` writes back** (17b cut M1b.4; doc 17 §5), with
/// **no slot number** — the number is the arena's to give:
/// `Arming {weapon}: fires your lock when {clauses}.`, the clauses joined
/// ` and ` in this order: `your chance to hit is {n}% or better` (always);
/// `it shows its {words}` (the mask's side words joined ` or `, only when not
/// every aspect); `it is under {b} m`, `it is past {a} m` or `it is between
/// {a} and {b} m` (only when bound); `{weapon} keeps {k} round(s)` (only above
/// nought). The gate's rule reads `Arming cannon: fires your lock when your
/// chance to hit is 60% or better and it shows its side or rear.`
///
/// **17b cut M2.2, the lead's ruling (c)**: read off the rule AS TYPED
/// ([`ArmingEcho`]) — `{a}` is the `range >` figure typed, else the band's
/// nearest above nought; `{b}` the `range <` figure typed, else the band's
/// furthest — so `range < 600` still reads `it is under 600 m` while the slot
/// stores 599 (and `WEAPONSTATUS`'s arming line, `arming_words`, shows the
/// stored `under 599m`, the last metre that fires). Every M1b.4 reply is
/// byte-identical. The chance clause is written whenever the echo carries
/// one, which [`parse_arming`]'s always does.
#[must_use]
pub fn arming_reply(weapon: &str, echo: &ArmingEcho) -> String {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(permille) = echo.min_permille {
        clauses.push(format!(
            "your chance to hit is {}% or better",
            permille / 10
        ));
    }
    if let Some(aspects) = echo.aspects
        && aspects & ASPECTS_ANY != ASPECTS_ANY
    {
        let words = aspect_side_words(aspects);
        if !words.is_empty() {
            clauses.push(format!("it shows its {}", words.join(" or ")));
        }
    }
    let past = echo
        .past_m
        .or_else(|| echo.band.map(|(nearest, _)| nearest))
        .filter(|metres| *metres > 0);
    let under = echo
        .under_m
        .or_else(|| echo.band.map(|(_, furthest)| furthest))
        .filter(|metres| *metres < u32::MAX);
    match (past, under) {
        (Some(past), Some(under)) => {
            clauses.push(format!("it is between {past} and {under} m"));
        }
        (None, Some(under)) => clauses.push(format!("it is under {under} m")),
        (Some(past), None) => clauses.push(format!("it is past {past} m")),
        (None, None) => {}
    }
    if let Some(keeps) = echo.keeps.filter(|keeps| *keeps > 0) {
        clauses.push(format!("{weapon} keeps {keeps} round(s)"));
    }
    format!(
        "Arming {weapon}: fires your lock when {}.",
        clauses.join(" and ")
    )
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
///
/// **16b cut AP.1** (16c `B-86`): the cycle is the last frame's countdown aged by
/// the wall clock since the frame was received ([`aged_weapons`]), unaged under a
/// staff tempo floor or on a frame built under one since lifted — so a `FIRE`
/// typed once the gun is truly ready is forwarded instead of refused locally, and
/// the arena takes it at its next wake. A remainder above nought is still refused,
/// in the same sentence. A `FIRE` forwarded inside the OS timer's overshoot of a
/// wake scheduled before the ready instant can still meet the arena's own refusal
/// a pulse later, which `help cockpit`'s "a pulse apart" paragraph already allows.
///
/// **17b cut M1b.4** (doc 17 §5, D-UM-18): the grammar grows two arms and no
/// verb — `FIRE <weapon> WHEN <condition> [AND <condition>]…` arms a slot
/// ([`fire_when`]) and `FIRE <weapon|ALL> OFF` clears one or every slot
/// ([`fire_off`]) — taken ONLY when [`fire_form`] reads `WHEN` or `OFF` as the
/// second word, or `OFF` alone, so every existing `FIRE` line goes on to the
/// pre-checks below byte for byte as it always did. A typed `FIRE` is never
/// gated by a slot (the owner's pin P1, 16c Q45).
fn fire(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    match fire_form(rest) {
        FireForm::Shot => {}
        FireForm::Off { weapon, tail } => return fire_off(ctx, seat, weapon, tail),
        FireForm::When { weapon, conditions } => return fire_when(ctx, seat, weapon, conditions),
    }
    let actor = ctx.actor;
    // 16b cut AP.1: the one wall-clock read this verb ages the mirror by.
    let now = std::time::Instant::now();
    // 16b cut D.2 (`B-03`): a wreck fires nothing, and it is asked first — a
    // destroyed tank's weapons are gone, so every later check would answer about
    // the rack instead of about the tank.
    require_alive(ctx.w, seat.driver)?;
    let held = last_frame(ctx.w, seat.driver);
    let weapon = choose_weapon(&carried(ctx.w, seat.unit, held.as_ref(), now), rest)?;
    // **16b cut D.2** (`B-16`): every one of these is `FIRE <weapon> refused. …`,
    // which is the sentence the arena's own refusal says a pulse later. The
    // `NoSuchWeapon` above is the one that is not: `choose_weapon` failed to find
    // a weapon, so there is none to name — and (16b cut AE.3) neither is
    // `NoWeapon`, a bare `FIRE` on a platform that carries none.
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
    require_power(ctx.w, seat.driver).map_err(|said| refused(&said))?;
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
        IntentKind::Fire {
            weapon: weapon.id.clone(),
            slot: None,
        },
    )?;
    let line = format!("Firing {}.", weapon.id);
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// What `CONTACTS` says of an empty scope.
const NO_CONTACTS_LINE: &str = "No contacts.";

/// What `VIEW BRIEF` says in its contacts' place when the platform holds none
/// (16b cut AP.4, 16c `N20`): the brief's own sentence, never `CONTACTS`'
/// `No contacts.`, so a reader can tell a brief's empty slot from a
/// `CONTACTS` reply. A declared reader sentence (doc 16 section 11).
pub const NO_CONTACTS_IN_WINDOW: &str = "No contacts in the window.";

/// The fixed last line of every `VIEW BRIEF` (16b cut AP.4, 16c `N20`), after
/// the off-window line when there is one: where a brief ends, for a script.
/// A declared reader sentence (doc 16 section 11).
pub const BRIEF_END_LINE: &str = "End of brief.";

/// `CONTACTS` — what the platform holds (doc 16 §3.6 "What a pilot sees").
fn contacts(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
    // 16b cut B.4 (`T-60`): the ranges held against the mirror's memory, the
    // figures the panels were sent.
    let rows = held
        .as_ref()
        .map(|held| frame_contacts_held(&held.frame, &held.ranges))
        .unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    if rows.is_empty() {
        lines.push(NO_CONTACTS_LINE.to_owned());
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
///
/// **16b cut AP.1** (16c `B-86`): and each cycle is aged by the wall clock since the
/// frame was received ([`aged_weapons`]), so `cannon  cycling (1 s)` counts down to
/// `cannon  ready` between frames. The keyframe re-pushed beside the lines is the
/// frame's own, unaged.
fn weapon_status(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    // 16b cut AP.1: read once, at the top of the verb.
    let now = std::time::Instant::now();
    let held = last_frame(ctx.w, seat.driver);
    let weapons = carried(ctx.w, seat.unit, held.as_ref(), now);
    let mut lines: Vec<String> = Vec::new();
    if weapons.is_empty() {
        lines.push("This unit carries no weapon.".to_owned());
    }
    if let Some(line) = held.as_ref().and_then(|held| lock_line(&held.frame)) {
        lines.push(line);
    }
    for weapon in &weapons {
        lines.push(weapon.render(&weapons));
        // **17b cut M1b.4** (doc 17 §5, as built for the 120-column floor):
        // an armed weapon's slot on its own line, two spaces in, under its
        // row — `  S1: >=60% side/rear` — the line the TUI pane and the web
        // print under the same row ([`arming_words`]: three renderers, one
        // shape). The row itself is unchanged.
        if let Some(armed) = &weapon.armed {
            lines.push(format!("  {}", arming_words(armed)));
        }
    }
    say_and_frame(ctx, held.as_ref(), lines);
    Ok(())
}

/// `STATUS` — the status block of doc 16 §5, as the last frame carries it.
fn status(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
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
                    display_name(ctx.w, seat.unit),
                    // **16b cut D.3 (`T-60`)**: the chassis, when it is not the
                    // one the pod authors.
                    fitted_bracket(ctx.w, seat.unit, frame),
                    frame.power.as_str(),
                    cms_to_kph(frame.speed_cms),
                    frame.pose.heading_mdeg / 1_000,
                    if frame.held { ", HELD" } else { "" },
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
            // **16b cut V.1**: the named ground nearest the pilot's own pose, on a
            // line of its own after the position and only when the ground names a
            // spot — so two pilots can say where they are in words. Surveyed
            // ground and the pilot's own position: nothing about any other unit.
            let (spots, cell_cm) = spots_of(ctx.w, Some(held.match_id));
            if let Some(line) = nearest_spot_line(&spots, &frame.pose, cell_cm) {
                lines.push(line);
            }
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
            // **16b cut O.2** (16c N12): after the receiver and only for a
            // platform that carries a pod — the pilot's own switch.
            if let Some(on) = frame.ecm {
                lines.push(emcon_status_line(on).to_owned());
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
            display_name(ctx.w, seat.unit)
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
/// its facets' armour, wrapped at `DAMAGE_COLUMNS` — and one for a section that
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
            // Two spaces join a cell and a wrap pushes no comma, so `+ 2` is
            // the whole cost here (16b cut B.4 widened `scan_lines`' own test,
            // which does push one, and left this alone).
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

/// **What `VIEW` says to any second word but `BRIEF`** (16b cut AG.1): syntax
/// first, in the voice of `SPEED <kph>; a negative speed reverses.`, and the
/// same bytes for every word — a slot designator of a tank out there, a
/// provisional number, a word naming nothing, sixteen digits — so the refusal
/// is never an echo and never tells one word from another.
pub const VIEW_USAGE: &str = "VIEW [BRIEF]; BRIEF says the window in sentences.";

/// `VIEW` (either arm) with no frame held yet. A `const` since 16b cut AG.1,
/// beside its twin below, so both arms say the one string.
const WINDOW_DARK_UNTOLD: &str = "The window is dark: the panel has told you nothing yet.";

/// `VIEW` (either arm) with a frame held but no grid to draw (16b cut B.4,
/// `B-23`). A `const` since 16b cut AG.1, whose brief arm also says it when the
/// ground has gone between the payload's read and the walk's.
const WINDOW_DARK_NO_GROUND: &str =
    "The window is dark: there is no ground under the panel to draw.";

/// `VIEW` — the arena window, as the grid frame (doc 16 §3.6, §5).
///
/// **16b cut Q.3**: the panel goes first and the one line that may follow it
/// comes after, because the line is *about* the map the pilot has just been
/// shown ([`beyond_window_line`]). **16b cut V.1** added a second such line,
/// after the first: the named spots off the window ([`off_window_spots_line`]).
///
/// **16b cut AG.1** (16c N17) gave it one second word. `rest` empty is the map,
/// byte for byte as before; `BRIEF` (case-folded) is the same window said in
/// sentences ([`brief_lines`]) — built off the very payload the map would push,
/// and then NOT pushed: the brief goes through [`say_and_frame`] as `CONTACTS`
/// does, so the one grid a pilot is sent is still the one a bare `VIEW` sends
/// (16b cut Q.3's rule). Any other word is [`VIEW_USAGE`], the same bytes for
/// every word and never an echo of it, so `VIEW BA` says nothing `VIEW ZZ` does
/// not. The two dark-window lines are shared by both arms: the shape test runs
/// before they diverge.
fn view(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let brief = match rest.trim() {
        "" => false,
        word if word.eq_ignore_ascii_case("brief") => true,
        _ => return Err(CockpitError::Usage(VIEW_USAGE.to_owned())),
    };
    let actor = ctx.actor;
    // The map is the frame's; a line client's renderer draws it. Nothing is said
    // twice.
    let Some(held) = last_frame(ctx.w, seat.driver) else {
        ctx.out
            .sys_to_actor(actor, NOTICE_CATEGORY, WINDOW_DARK_UNTOLD);
        return Ok(());
    };
    let payload = view_frame_held(ctx, held.match_id, &held.frame, &held.ranges);
    // **16b cut B.4** (`B-23`): a frame is held but the window cannot be drawn —
    // the registry holds no arena for the match, or the pose is off the terrain
    // — so `view_frame` answered the bare status frame, which declares no shape
    // and a line client prints as nothing: `VIEW` used to answer with a blank.
    // The test is inline, on the key `view_window` writes; it is what
    // `mush_ansi::panel::is_grid` reads on the other side, and this crate takes
    // no dependency on a renderer crate to ask it.
    if payload
        .pointer("/schema/shape")
        .and_then(serde_json::Value::as_str)
        != Some("grid")
    {
        ctx.out
            .sys_to_actor(actor, NOTICE_CATEGORY, WINDOW_DARK_NO_GROUND);
        return Ok(());
    }
    if brief {
        // **16b cut AG.1**: the ground `view_frame_held` just read, read again
        // for the walk. A ground gone between the two reads is the dark window
        // above, never a panic.
        let lines = ctx
            .w
            .module_state::<ArenaRegistry>()
            .and_then(|registry| registry.arena_of(held.match_id))
            .map(|ground| {
                brief_lines(
                    &held.frame,
                    &ground.terrain,
                    &ground.key_spots,
                    &payload,
                    &held.ranges,
                )
            });
        let Some(lines) = lines else {
            ctx.out
                .sys_to_actor(actor, NOTICE_CATEGORY, WINDOW_DARK_NO_GROUND);
            return Ok(());
        };
        // The lines, then the status keyframe re-pushed — never the grid.
        say_and_frame(ctx, Some(&held), lines);
        return Ok(());
    }
    let beyond = beyond_window_line(&held.frame, &payload);
    // **16b cut V.1**: the named ground the window could not draw, after the
    // contacts' line — a spot is a place the pilot may name whether or not it
    // is in sight, because the ground is surveyed. Read against the drawing's
    // own spots, so a spot under a contact (drawn, then covered) is not listed.
    let (spots, cell_cm) = spots_of(ctx.w, Some(held.match_id));
    let off_window =
        off_window_spots_line(&spots, &drawn_spots(&payload), &held.frame.pose, cell_cm);
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
    if let Some(line) = off_window {
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
    // 16b cut B.4 (`T-60`): the frame with the ranges the pilot was told.
    let payload = status_frame_held(held.match_id, &held.frame, &held.ranges);
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
///
/// **17b cut M1a.3 bumped it to 2** (doc 17 §3, D-UM-4, D-UM-5): the fields a
/// mind reads joined the frame — a field added is still not a version, but the
/// first fields added FOR a reader that is not a person are, so a client can
/// say which frame it was built against; every consumer ignores unknown keys
/// and the version, so an older client prints the old row.
///
/// **17b cut M1b.3 bumped it to 3** (doc 17 §3's parity rule, §5, D-UM-4,
/// D-UM-17): two more fields added for a reader that is not a person — each
/// weapon row's `armed` (the condition slot the weapon is armed in, or `null`)
/// and the top-level `odds_floor_permille` (the match's Compact floor) — so a
/// mind reads its own slots and the floor exactly as a pilot does. Every
/// consumer ignores unknown keys and the version, so an older client prints
/// the old rows.
pub const FRAME_VERSION: u32 = 3;

/// **The terrain frame's schema id** (16b cut Q.3), carried on
/// [`crate::arena_bridge::MAP_FEED`].
pub const MAP_SCHEMA: &str = "arena.map";

/// Its version. The map is terrain and an origin; a field added is not a
/// version, exactly as [`FRAME_VERSION`]'s comment says of the cockpit frame.
pub const MAP_VERSION: u32 = 1;

/// **The bound a pulled keyframe is held to**, bytes of encoded JSON (16b cut
/// Q.3's 256 KiB; made a rule by 16b cut U.4, 16c `T-57`).
///
/// [`map_frame`] is the largest message the arena produces and it is only ever
/// the answer to a query, so this is the most one `arena.map` reply may cost a
/// session's output queue. It is enforced **once, when an arena definition is
/// applied** (`crate::definitions::arena::arena_map_within_bound`, called by the
/// arena applier), not every time a map is built: a loaded arena cannot exceed
/// it, so the handler that sends one never has to ask.
pub const MAP_MAX_BYTES: usize = 256 * 1024;

/// **The terrain frame's shape** (16b cut Q.3).
///
/// Deliberately **not** `grid`: `mush_ansi::panel` renders a `grid` keyframe as
/// twenty rows of text, and the whole arena is 256 by 256. A line client that
/// somehow received one must print nothing, and a shape it does not know is how
/// it does that.
pub const MAP_SHAPE: &str = "terrain";

/// **The `schema.retain` marker that makes a keyframe replace its own
/// predecessors for a session's resume** (16b cut Q.3; the rule is implemented
/// in `mush_session::SessionManager`, which cannot see this crate — the string
/// is the contract). Since 16b cut B.5 a marked keyframe lives in a per-feed
/// latest slot beside the resume ring rather than in it.
///
/// Both arena feeds carry it, and so does [`view_frame`]'s replacement schema
/// (`arena.map`'s never fires: it is only ever a conn-scoped query reply, which
/// is never recorded — see [`crate::arena_bridge::MAP_FEED`]). Two keyframes a
/// second of `Engaged` play would otherwise evict everything else from a
/// 512-entry ring in about four minutes; every frame either feed sends is the
/// whole state, so the older ones are not history and nothing is lost by
/// dropping them.
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
/// `CONTACTS` prints, metres rounded to [`CONTACT_RANGE_STEP_M`]. It
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
/// **16b cut B.1** (`T-86`) widened each notice's `section` to the victim's own
/// `struck`, `struck_spent` and `section_lost` (this platform's section, `kind`
/// says whose) and added `was`, the old designator on an `identified` or
/// `unidentified again` line and `null` on every other.
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
/// notices at all, because the mirror is stored with them cleared. **16b cut
/// B.1**: the bridge's `Sys` copy of each notice is tagged
/// [`crate::arena_bridge::NOTICE_SENTENCE_TAG`], so a panel ring that also takes
/// the pilot's replies (tagged [`crate::arena_bridge::NOTICE_CATEGORY`]) can tell
/// the sentence it composes from this array from an answer it has to ring.
///
/// **16b cut D.1** added two top-level keys: `sight_m`, the platform's own
/// nominal sensor reach in metres (`0` for a platform with no sensor), and
/// `scope`, the pilot's own alert state — `clear`, `unidentified` or `hostile`
/// ([`own_scope`]). Each contact's `band` is [`contact_band`]'s word: thirds of
/// the lesser of `sight_m` and the fired weapon's reach. `tempo` is unchanged and
/// stays the arena's global word.
///
/// **16b cut B.4** (`T-60`): this is [`status_frame_held`] with no range memory,
/// for the fixtures and every pure reading of a lone frame.
///
/// **16b cut Y.4** added one top-level key, `catchup`: `null`, or on the first
/// frame after a reclaim `{ away_s, cell_then, cell_now, power_then, power_now,
/// wreck, contacts_new, contacts_lost, contacts_more, sections[{ id, then, now
/// }], sections_more, rounds_spent[{ weapon, rounds }], rounds_more, machine }`
/// ([`catchup_lines`] is its wording). [`FRAME_VERSION`] stays `1`: a field
/// added is not a version.
///
/// **17b cut M1a.3** (doc 17 §3, D-UM-4, D-UM-5) added what a mind reads, and
/// bumped [`FRAME_VERSION`] to `2`. Each contact gained `aspect` — the **facet**
/// word its plot was showing (`front`, `left`, `right`, `rear`; the row prints
/// the side word) — and `fatal`, the target chassis's fatal section words beside
/// a held reading and `null` without one; on a hull with a weapons station each
/// contact also gained `reads_dead` (the own kill, or a held reading showing a
/// fatal section destroyed) and `odds`, `[{ weapon, p_permille, percent }]` for
/// the nearest `STOCK_ODDS_ROWS` classified contacts, the default weapon first,
/// `[]` for every other row. Each weapon gained `bands_cm`. The top level gained
/// `t_ms`, the arena millisecond of the build, and `order`: `null` — every
/// piloted hull — or `{ squad, objective, target, to, facing_mdeg, of,
/// hold_fire, max_range_band, expires_in_ms }`, `target` and `of` the
/// designators of the hull's own rows. **A hull with no weapons station
/// carries no `weapons` key** and no contact carries `odds` or `reads_dead`
/// (doc 17 §22's italic line); `lock`, `lock_*`, `heat`, `scan_tables` and
/// `fatal` stay.
///
/// **17b cut M1b.3** (doc 17 §3, §5, D-UM-4, D-UM-17) bumped [`FRAME_VERSION`]
/// to `3`. Each weapon gained `armed`: `null`, or the slot it is armed in as
/// `{ slot, target, min_permille, percent, aspects, keeps, min_range_m,
/// max_range_m }` — `target` the KIND (`lock`, `best` or `contact`) and never a
/// designator, `aspects` the facet words in the mask in `front`, `left`,
/// `right`, `rear` order, `keeps` the rounds the slot keeps back, `min_range_m`
/// `null` for no lower bound and `max_range_m` `null` for no upper one
/// ([`arming_json`]). The top level gained `odds_floor_permille`, the match's
/// Compact floor captured at check-out, beside `t_ms`. Each contact's `odds`
/// now ride the lock first and only classified contacts of another side
/// (`Arena::frame_odds`, Q2): a friend's row carries `[]`.
#[must_use]
pub fn status_frame(match_id: MatchId, frame: &Frame) -> serde_json::Value {
    status_frame_held(match_id, frame, &BTreeMap::new())
}

/// **The cockpit keyframe with each contact's `range_m` held** (16b cut B.4,
/// `T-60`) against `ranges` — [`status_frame`]'s payload, its rows built by
/// [`frame_contacts_held`]. What the bridge sends every pulse and what every read
/// verb re-pushes off the mirror, so the panel rows, `CONTACTS` and `RANGE` read
/// one figure. `lock_range_m` keeps its own step and no memory.
#[must_use]
pub fn status_frame_held(
    match_id: MatchId,
    frame: &Frame,
    ranges: &BTreeMap<ContactId, u64>,
) -> serde_json::Value {
    let rows = frame_contacts_held(frame, ranges);
    // **17b cut M1a.3** (doc 17 §22's italic line): a hull with no weapons
    // station carries no `weapons` block, and its contacts no `odds` and no
    // `reads_dead`.
    let armed = !frame.weapons.is_empty();
    let contacts: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let held = frame.contacts.iter().find(|held| held.id == row.id);
            let mut contact = serde_json::json!({
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
                "condition": held
                    .and_then(|held| held.condition)
                    .map(|reading| serde_json::json!({
                        "table": reading.table,
                        "bands": reading.bands.letters(),
                        "age_ms": reading.age_ms,
                    })),
                // **17b cut M1a.3** (doc 17 §3, D-UM-5): the FACET word the
                // contact's plot was showing — `front`, `left`, `right` or
                // `rear`; the row prints its side word — and, beside a held
                // reading only, the chassis's fatal section words: what a
                // reading would have to show destroyed for `reads_dead`.
                "aspect": held.map_or("front", |held| held.aspect.as_str()),
                "fatal": held.and_then(|held| held.fatal.as_ref()),
            });
            if armed && let Some(object) = contact.as_object_mut() {
                // **17b cut M1a.3**: the weapons station's two fields. Every
                // `FrameOdds` of this row, the default weapon first (the order
                // `Arena::frame_odds` builds), and `[]` for a row outside the
                // odds rows.
                object.insert(
                    "reads_dead".to_owned(),
                    serde_json::json!(held.and_then(|held| held.reads_dead).unwrap_or(false)),
                );
                object.insert(
                    "odds".to_owned(),
                    serde_json::Value::Array(
                        frame
                            .odds
                            .iter()
                            .filter(|odds| odds.contact == row.id)
                            .map(|odds| {
                                serde_json::json!({
                                    "weapon": odds.weapon.as_str(),
                                    "p_permille": odds.p_permille,
                                    "percent": odds.p_permille / 10,
                                })
                            })
                            .collect(),
                    ),
                );
            }
            contact
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
            let mut object = serde_json::json!({
                "kind": notice.kind().as_str(),
                "contact": notice.contact().map(|id| id.to_string()),
                "designator": notice.designator().map(|named| named.to_string()),
                // **16b cut B.1** (`T-86`): the old name on an `identified` or
                // an `unidentified again` line — the provisional number a
                // contact was told apart from, or the name a flicker cost it —
                // so a panel can say which name the pilot had. `null` on every
                // other notice. A designator this platform issued or read,
                // never an object number.
                "was": notice.was().map(|designator| designator.to_string()),
                // 16b cut Q.2, widened by **16b cut B.1** (`T-86`): the section
                // this notice is about — the target's on the gunner's `hit` and
                // `target_section_lost`, this platform's own on the victim's
                // `struck`, `struck_spent` and `section_lost` — so a panel never
                // regexes it back out of `text`; `kind` says whose it is. The
                // section word only: the facet, the numbers and the cost of a
                // lost section stay in `sections` above and in the sentence.
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
                // **16b cut O.2**: an `emcon_on`/`emcon_off` notice carries the
                // pod's radius here, so a panel composes its own line without
                // reading `text`.
                "detail": match (notice, notice.detail()) {
                    (FrameNotice::Emcon { radius_m, .. }, _) => {
                        serde_json::json!({ "radius_m": radius_m })
                    }
                    // **17b cut M1b.1**: a held slot's reason, every why's keys
                    // present and the other whys' `null`, so a panel composes the
                    // sentence without reading `text`.
                    (FrameNotice::SlotHeld { why, .. }, _) => held_detail(why),
                    (_, RefusalDetail::None) => serde_json::Value::Null,
                    (_, RefusalDetail::Cycling { secs }) => serde_json::json!({ "secs": secs }),
                    (_, RefusalDetail::Overheated { heat, max }) => {
                        serde_json::json!({ "heat": heat, "max": max })
                    }
                    // **17b cut M1b.1**: the floor an `unlawful` refusal was held to.
                    (_, RefusalDetail::Floor { permille }) => {
                        serde_json::json!({ "floor_permille": permille })
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
            });
            // **17b cut M1b.1** (doc 17 §5; 17b cut M1b.4 stages them): a slot
            // notice's own parts, on its own object only — every other notice's
            // object is byte for byte what it was. The pilot's own slot, the
            // chance and the facet word the row earned; the weapon and the
            // designator ride the keys above.
            if let Some(map) = object.as_object_mut() {
                match notice {
                    FrameNotice::SlotFired {
                        slot,
                        p_permille,
                        aspect,
                        ..
                    } => {
                        map.insert("slot".to_owned(), serde_json::json!(slot));
                        map.insert("percent".to_owned(), serde_json::json!(p_permille / 10));
                        map.insert("aspect".to_owned(), serde_json::json!(aspect.as_str()));
                    }
                    FrameNotice::SlotHeld { slot, .. } => {
                        map.insert("slot".to_owned(), serde_json::json!(slot));
                    }
                    _ => {}
                }
            }
            Some(object)
        })
        .collect();
    let weapons: Vec<serde_json::Value> = frame_weapons(frame)
        .iter()
        .zip(frame.weapons.iter())
        .map(|(weapon, carried)| {
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
                // **17b cut M1a.3** (D-UM-4): the band edges, centimetres — what
                // the view's band word and a mind's reach arithmetic read, and
                // which the keyframe did not carry until a reader that is not a
                // person needed it.
                "bands_cm": carried.bands_cm,
                // **17b cut M1b.3** (D-UM-4): the slot this weapon is armed in,
                // or `null` — the pilot's own slot, and never a designator.
                "armed": weapon.armed.as_ref().map(arming_json),
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
    let mut payload = serde_json::json!({
        // 16b cut Q.3: `retain` tells the session layer that this keyframe
        // supersedes the ones before it on this feed — it replaces the feed's
        // entry in the session's latest slot (16b cut B.5; see
        // [`RETAIN_LATEST`]).
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
        // **16b cut M.3** (16c `T-04`): the Rust field is `Frame::held` now, and
        // this wire key is deliberately still `"blocked"` — `web/src/lib/cockpit.ts`
        // and the TUI renderer both read it, and renaming it is a client cut's.
        "blocked": frame.held,
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
        // **17b cut M1a.3** (doc 17 §3 "Two more fields"): the arena millisecond
        // the frame was built at, and the hull's standing order re-keyed to its
        // own contacts — `null` on every piloted hull (`Arena::frame_order`).
        "t_ms": frame.t_ms,
        // **17b cut M1b.3** (doc 17 §5 as amended, D-UM-17): the match's
        // Compact floor, captured at check-out — a public constant.
        "odds_floor_permille": frame.odds_floor_permille,
        "order": frame.order.as_ref().map(|order| order_json(frame, order)),
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
        // **16b cut O.2** (16c N12): the pilot's own pod's switch, `"on"` or
        // `"off"`, and `null` for a platform that carries no pod — every stock
        // tank. The key is on every frame, as `receiver` is; only its value is
        // the pilot's own.
        "ecm": frame.ecm.map(|on| if on { "on" } else { "off" }),
        // **16b cut Y.4** (doc 07 §1.3 step 5): what happened while the pilot
        // could not see, on the first frame after a reclaim and `null` on
        // every other — the mirror stores it cleared, so a read verb's frame
        // is `null` too. The pilot's own tank and the platform's own
        // designators (`catchup_json`); both panels compose their lines from
        // it and the line client was sent the same lines as one `Sys` block.
        "catchup": frame.catchup.as_ref().map(catchup_json),
    });
    // **17b cut M1a.3** (doc 17 §22's italic line): *the keyframe drops the
    // weapons block on one without*. `lock`, `lock_*`, `heat`, `scan_tables`
    // and each contact's `fatal` stay: the lock is the sensors station's.
    if !armed && let Some(object) = payload.as_object_mut() {
        object.remove("weapons");
    }
    payload
}

/// **A standing order as the keyframe carries it** (17b cut M1a.3; doc 17 §3,
/// D-UM-4): the objective's word, its payloads (`to`, `facing_mdeg`, and the
/// contacts `target` and `of` as the **designator** of the hull's own row the
/// order names, `null` where it names none), the two constraints, and the time
/// left as `expires_in_ms` — `expires_ms` less the frame's own `t_ms`, never
/// below nought — so the view carries nothing the keyframe lacks.
fn order_json(frame: &Frame, order: &crate::arena::perception::FrameOrder) -> serde_json::Value {
    use crate::arena::perception::MindObjective;
    let named = |contact: Option<ContactId>| {
        contact.and_then(|id| {
            frame
                .contacts
                .iter()
                .find(|held| held.id == id)
                .map(|held| held.designator.to_string())
        })
    };
    let (target, to, facing_mdeg, of) = match order.objective {
        MindObjective::Engage { target } => (named(target), None, None, None),
        MindObjective::Advance { to } => (None, Some([to.0, to.1]), None, None),
        MindObjective::Screen { facing_mdeg } => (None, None, Some(facing_mdeg), None),
        MindObjective::Support { of } => (None, None, None, named(of)),
        MindObjective::Hold | MindObjective::Withdraw => (None, None, None, None),
    };
    serde_json::json!({
        "squad": order.squad.as_str(),
        "objective": order.objective.as_str(),
        "target": target,
        "to": to,
        "facing_mdeg": facing_mdeg,
        "of": of,
        "hold_fire": order.hold_fire,
        "max_range_band": order.max_range_band.map(|band| band.as_str()),
        "expires_in_ms": order.expires_ms.saturating_sub(frame.t_ms).max(0),
    })
}

/// **One rectangle of terrain, run-length encoded** (16b cut Q.3), and the
/// classes it actually carries in first-seen order.
///
/// Row `0` of the answer is the **south** edge (`y0`), which is what the grid
/// shape means by `rows` and what [`map_frame`] and [`view_window`] both publish.
/// One glyph per terrain class, from the atlas's own table, so the arena's map
/// and the atlas's map read alike.
///
/// **16b cut I.1** made it `pub(crate)`: a replay's window
/// (`crate::match_::replay::replay_window`) draws its ground with this same
/// builder rather than a copy of it.
pub(crate) fn terrain_rows(
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
///
/// **Total, and within [`MAP_MAX_BYTES`] for every loaded arena** (16b cut U.4):
/// the bound is checked when the arena is applied, over this very function at
/// the widest `match` and `surface` the handler can send, so an arena whose map
/// would pass it never loads and nothing here needs to refuse.
///
/// **The second thing the map may say: the named spots** (16b cut V.1). One key
/// after `legend`, `spots`, is `[{ "letter": "A", "name": "The Ford", "cell":
/// [64, 97] }, …]` — the arena's authored `key_spots` in authored order, in
/// arena cells, each with the marker letter [`spot_letter`] gives its index. They
/// are surveyed ground exactly as the terrain is (doc 16 §3.2: known to every
/// pilot at check-out), and they name no unit. A spot past [`SPOT_LETTERS`] —
/// which the load-time cap refuses — is left out rather than drawn with no
/// letter. [`MAP_VERSION`] stays `1`: a field added is not a version.
#[must_use]
pub fn map_frame(
    match_id: MatchId,
    surface: &str,
    terrain: &ArenaTerrain,
    spots: &[KeySpot],
) -> serde_json::Value {
    let (rows, classes) = terrain_rows(terrain, (0, 0), (terrain.width(), terrain.height()));
    let legend: Vec<serde_json::Value> = classes
        .iter()
        .map(|class| serde_json::json!([class.glyph().to_string(), class.label()]))
        .collect();
    let spots: Vec<serde_json::Value> = spots
        .iter()
        .zip(SPOT_LETTERS)
        .map(|(spot, letter)| {
            serde_json::json!({
                "letter": letter.to_string(),
                "name": spot.name.as_str(),
                "cell": [spot.x, spot.y],
            })
        })
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
        "spots": spots,
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
/// because it carries [`RETAIN_LATEST`] like every frame before it — it
/// replaces the live frame in that session's per-feed latest slot, the one
/// `arena.cockpit` frame a resume replays, kept beside the ring rather than in
/// it (16b cut B.5), UNTIL the pilot is seated again, when the next match's
/// first frame replaces it in turn — which is why a panel resets its own
/// notices ring on a change of match and never relies on this frame arriving
/// (16b cut B.2).
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
    view_frame_held(ctx, match_id, frame, &BTreeMap::new())
}

/// [`view_frame`] with each contact's range held against `ranges` (16b cut B.4,
/// `T-60`) — what `VIEW` pushes off the mirror, so its payload's rows agree with
/// the frame the pilot was last sent.
#[must_use]
pub fn view_frame_held(
    ctx: &Ctx<'_>,
    match_id: MatchId,
    frame: &Frame,
    ranges: &BTreeMap<ContactId, u64>,
) -> serde_json::Value {
    let payload = status_frame_held(match_id, frame, ranges);
    let Some(ground) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
    else {
        return payload;
    };
    let mut payload = view_window(
        payload,
        ground.id.as_str(),
        &ground.terrain,
        &ground.key_spots,
        frame,
    );
    // The pilot's own tank's name, which is theirs to have and which a pure
    // window has no world to ask for: [`view_window`] leaves the key out rather
    // than inventing a value the live feed does not send, and this is the one
    // place it is filled in. The whole payload is byte for byte what it was
    // before cut Q.3 apart from `schema.retain`.
    //
    // **16b cut V.1**: the own marker is found by its `you` flag. It used to be
    // `units.first()`, and the first marker is now a spot whenever the window
    // holds one.
    if let Some(you) = payload
        .get_mut("units")
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|units| {
            units
                .iter_mut()
                .find(|unit| unit.get("you").and_then(serde_json::Value::as_bool) == Some(true))
        })
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
/// clamped to the arena: the most a line client prints at
/// [`VIEW_CELL_COLUMNS`] columns a cell (16b cut B.4), so the renderer only has
/// to clip it to a narrow terminal and never has to ask for more. A pose off the
/// window answers the payload untouched.
///
/// **The marker order is the rule every compositor follows** — since **16b cut
/// V.1**: the named spots inside the window in authored order
/// ([`spot_markers`]), then the pilot's own marker, then the contacts in frame
/// order, a later marker overwriting an earlier one on the same cell (which is
/// what `mush_ansi::panel::expand_rows` does with `units`, and what the TUI and
/// the web must do too). So a contact standing on a spot hides its letter — the
/// spot is ground and the contact is what the pilot holds — and so does the
/// pilot's own `@`.
///
/// **The naming order is the reverse of the drawing** (16b cut V.1). `legend`
/// is the terrain classes, then `["@", "you"]`, then the contact glyph labels,
/// and THEN `["A", "The Ford"]` for each spot that **owns its cell** — that no
/// later marker landed on, computed here after all three groups are placed. A
/// covered spot is still in `units` (the grid renderer draws in order and the
/// later write wins, which is the rule) but is named nowhere, so the line
/// client's, the TUI's and the web's legends name the same spots. Contacts come
/// first because a narrow legend is cut from the back — the TUI's `fit_legend`
/// keeps entries from the front and a zoomed canvas's `labelPlan` places labels
/// in order — and a narrow pane must keep `x BA` before the name of a field.
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
    spots: &[KeySpot],
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
    // 16b cut V.1: the named spots inside the window go down FIRST, then the
    // platform at the centre, then one marker per contact. Cells are relative to
    // the window's own origin, which is what the grid renderer centres on.
    let (spot_units, spot_legend, _drawn) = spot_markers(spots, (x0, y0), (width, height));
    let spot_count = spot_units.len();
    let you = [
        u32::from(cx.saturating_sub(x0)),
        u32::from(cy.saturating_sub(y0)),
    ];
    let mut units = spot_units;
    units.push(serde_json::json!({
        "id": format!("#{}", frame.unit.0),
        "cell": you,
        "glyph": YOU_GLYPH,
        "you": true,
    }));
    let (markers, glyphs) = contact_markers(frame, (x0, y0), (width, height));
    units.extend(markers);
    for entry in glyphs {
        if !legend.contains(&entry) {
            legend.push(entry);
        }
    }
    // The spots are NAMED last, and only those that own their cell: a spot
    // marker any later marker (the `@`, a contact, or a later spot) landed on is
    // drawn and then covered, so it is named nowhere — the owner rule the TUI's
    // and the web's legends apply, applied here once every marker is placed.
    for (index, entry) in spot_legend.into_iter().enumerate().take(spot_count) {
        let cell = units.get(index).and_then(|unit| unit.get("cell"));
        let covered = units
            .iter()
            .skip(index.saturating_add(1))
            .any(|later| later.get("cell") == cell);
        if !covered {
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
        // **16b cut B.4** (`B-24`, the `T-60` footer slip): three keys for the
        // grid renderer. `cell_columns` — a line client prints each cell this
        // many columns wide, so the ground is square; `origin_cell` and
        // `extent` — the window's corner and the arena's size in arena cells,
        // so the footer names arena cells and not window cells. The size is
        // lobby knowledge (doc 16 §3.2: the proving ground is surveyed ground)
        // and the corner is read off the pilot's own pose; neither is a fact
        // about any other unit.
        object.insert(
            "cell_columns".to_owned(),
            serde_json::json!(VIEW_CELL_COLUMNS),
        );
        object.insert("origin_cell".to_owned(), serde_json::json!([x0, y0]));
        object.insert(
            "extent".to_owned(),
            serde_json::json!([terrain.width(), terrain.height()]),
        );
    }
    payload
}

/// **The one honest line `VIEW` owes a pilot whose scope is wider than their
/// terminal** (16b cut Q.3; doc 16 §3.6).
///
/// A sensor reaches sixty cells every way and the line client's window is
/// [`VIEW_COLUMNS`] by [`VIEW_ROWS`], because a terminal is eighty by
/// twenty-four and each cell is printed two columns wide (16b cut B.4: a
/// close-in window, so this line is what covers the rest of the sight). A contact whose plot falls outside that window — or off the
/// arena entirely, which is a plot with no cell — is on `CONTACTS` and not on
/// the map, and the pilot is told so rather than left to conclude the scope is
/// empty. `None` when every held contact is drawn.
///
/// `window` is the payload [`view_window`] just produced: its `units` are the
/// markers that were drawn, so the count is the drawing's own and cannot drift
/// from it. A payload with no window at all owes nothing. **16b cut V.1**: a
/// marker is a contact only when it is neither the pilot's own (`you`) nor a
/// named spot (`spot`); the spots have their own line, [`off_window_spots_line`].
#[must_use]
pub fn beyond_window_line(frame: &Frame, window: &serde_json::Value) -> Option<String> {
    let flag = |unit: &serde_json::Value, key: &str| {
        unit.get(key).and_then(serde_json::Value::as_bool) == Some(true)
    };
    let drawn = window
        .get("units")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter(|unit| !flag(unit, "you") && !flag(unit, "spot"))
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

/// **The keyframe's `armed` object** (17b cut M1b.3; doc 17 §3, D-UM-4): the
/// slot, the target's KIND, the odds per mille and as the whole per cent,
/// the facet words in the mask, the rounds kept, and the two range bounds —
/// `null` for an unbound one. **Never a designator**: a
/// [`SlotTarget::Contact`] writes `"contact"` with no id (doc 17 §5, the
/// observer rule; it is unlawful to arm in 17b cut M1b).
#[must_use]
pub fn arming_json(armed: &FrameArming) -> serde_json::Value {
    serde_json::json!({
        "slot": armed.slot,
        "target": arming_target_word(armed.target),
        "min_permille": armed.min_permille,
        "percent": armed.min_permille / 10,
        "aspects": ASPECT_ORDER
            .iter()
            .filter(|aspect| aspect_in(armed.aspects, **aspect))
            .map(Aspect::as_str)
            .collect::<Vec<&str>>(),
        "keeps": armed.keeps,
        "min_range_m": (armed.min_range_m > 0).then_some(armed.min_range_m),
        "max_range_m": (armed.max_range_m < u32::MAX).then_some(armed.max_range_m),
    })
}

/// The four facets in mask order — `front`, `left`, `right`, `rear`
/// ([`crate::arena::perception::Aspect::code`]).
const ASPECT_ORDER: [Aspect; 4] = [Aspect::Front, Aspect::Left, Aspect::Right, Aspect::Rear];

/// **A slot's target, as a word** (17b cut M1b.3): `lock`, `best` or
/// `contact` — the KIND, never which contact.
#[must_use]
pub const fn arming_target_word(target: SlotTarget) -> &'static str {
    match target {
        SlotTarget::Lock => "lock",
        SlotTarget::BestHostile => "best",
        SlotTarget::Contact { .. } => "contact",
    }
}

/// **The arming line** (17b cut M1b.3; doc 17 §5's `WEAPONSTATUS` sentence as
/// built: the arming on its own line under the weapon's row, for the
/// 120-column floor) — ONE formatter for the three renderers: line mode's
/// `WEAPONSTATUS` prints it indented two spaces under the weapon's row (17b
/// cut M1b.4), and the TUI weapons pane and the web's `weaponLines` print the
/// same words (their copies of this rule, pinned equal by the fixture scene).
///
/// `S{n}:` then the terms, one space apart, compact for the TUI's 46-column
/// floor: `best` when the target is the best hostile (`lock` is the default
/// and prints nothing; a `contact` target prints `contact` and no id); then
/// `>={percent}%` always; then the facets as de-duplicated side words —
/// `front`, `side` (left or right), `rear`, in that order, joined `/` — only
/// when the mask is not all four (an empty mask prints none); then the range
/// with the unit attached, `{a}-{b}m`, `under {b}m` or `past {a}m`, only when
/// bound; then `keeps {k}` only above nought. `S1: >=60% side/rear keeps 8`;
/// `S1: >=60% side/rear 200-600m keeps 8`; T1's `S1: best >=10% under 420m`.
#[must_use]
pub fn arming_words(armed: &FrameArming) -> String {
    let mut terms: Vec<String> = vec![format!("S{}:", armed.slot)];
    match armed.target {
        SlotTarget::Lock => {}
        SlotTarget::BestHostile => terms.push("best".to_owned()),
        SlotTarget::Contact { .. } => terms.push("contact".to_owned()),
    }
    terms.push(format!(">={}%", armed.min_permille / 10));
    if armed.aspects & ASPECTS_ANY != ASPECTS_ANY {
        let words = aspect_side_words(armed.aspects);
        if !words.is_empty() {
            terms.push(words.join("/"));
        }
    }
    match (armed.min_range_m > 0, armed.max_range_m < u32::MAX) {
        (true, true) => terms.push(format!("{}-{}m", armed.min_range_m, armed.max_range_m)),
        (false, true) => terms.push(format!("under {}m", armed.max_range_m)),
        (true, false) => terms.push(format!("past {}m", armed.min_range_m)),
        (false, false) => {}
    }
    if armed.keeps > 0 {
        terms.push(format!("keeps {}", armed.keeps));
    }
    terms.join(" ")
}

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
    /// **The slot the weapon is armed in** (17b cut M1b.3; D-UM-4), off the
    /// frame's own row, or `None` — and always `None` from [`weapons_of`],
    /// which reads no arena. 17b cut M1b.4's `WEAPONSTATUS` prints it through
    /// [`arming_words`] on its own line under the row; nothing prints it here.
    pub armed: Option<FrameArming>,
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
            // 17b cut M1b.3: a unit no arena holds holds no slot.
            armed: None,
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
/// `This unit carries no weapon.` and a bare `FIRE` refuses in the cockpit with
/// [`CockpitError::NoWeapon`] (16b cut AE.3), which is the same sentence, one
/// pulse earlier.
///
/// **16b cut AP.1** (16c `B-86`): the frame's rows are read through
/// [`aged_weapons`] at `now`, so each countdown is the frame's own aged by the wall
/// time since the world received it — unaged under a staff tempo floor, or on a
/// frame built under one since lifted (`floored`).
fn carried(
    w: &World,
    unit: ObjId,
    held: Option<&CockpitFrame>,
    now: std::time::Instant,
) -> Vec<WeaponRow> {
    match held {
        Some(held) => aged_weapons(held, now, floored(w, held)),
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
/// [`CockpitError::NoSuchWeapon`]. A bare `FIRE` on a platform with no rows at
/// all is [`CockpitError::NoWeapon`] (16b cut AE.3): the pilot named nothing, so
/// the refusal names nothing. A step is not fallen through on ambiguity:
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
        .ok_or(CockpitError::NoWeapon)
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
// Commanding the machines (16b cut Y.1; doc 16 §3.7, §6; 16c N3, Q38)
// ---------------------------------------------------------------------------
//
// Doc 16 §6's deliberation had one issuer, the LLM seam, which the shipped
// server has off. `ORDER` is the second: a pilot tells their side's machines
// what to do with the same six objectives and the same `IntentKind::Order`
// the seam sends, so the arena cannot tell the two apart except by
// `Order::issuer` — the scope a target hint is written in. Who may order what is
// 16c Q38's default read as the cut Y plan's decision 2: the side's **lead**
// orders every squad of the side; **anyone else** orders the one unit of their
// pod pair when a machine flies it ([`Command`]). Every refusal is one line; the
// squad refusal is byte-identical for the other side's squad, a squad that does
// not exist and one the pilot may not order, and it is said **before** any
// contact is looked at, so a pilot learns nothing about the scope by naming a
// squad they cannot order.

/// **`ORDER`'s grammar**, said for anything [`parse_order`] cannot read (16b cut
/// Y.1; the two squad roles and the release since 16b cut Z.2). About 155
/// characters, inside the TUI notices ring's 200-character line budget.
pub const ORDER_USAGE: &str = "Usage: ORDER <squad|ALL> HOLD | ADVANCE <spot> | ENGAGE \
                               [<contact>] | WITHDRAW | SCREEN <bearing> | SUPPORT <callsign> \
                               [HOLD FIRE], or ORDER <squad|ALL> RELEASE";

/// **A `SUPPORT` callsign that names no tank of the pilot's own side** (16b cut
/// Z.2; 16c N6) — the other side's callsign, a provisional `X<n>`, a word that
/// is no callsign and a slot nobody flies, **byte for byte one sentence**, so a
/// pilot learns nothing about the other side's roster by naming it.
pub const NO_SUCH_FRIENDLY: &str = "No tank of your side flies that callsign.";

/// **A squad the pilot may not order** (16b cut Y.1) — the other side's, one that
/// does not exist, and the pilot's own side's beyond their authority, **byte for
/// byte one sentence**, so the count of the other side's squads is never learned.
pub const NO_SUCH_SQUAD: &str = "No such squad.";

/// **`ORDER ALL` from a pilot who commands nothing** (16b cut Y.1): a pilot who
/// does not lead the side and whose pod pair is a human, an empty pod, or a seat
/// with no slot at all.
pub const NO_SQUAD_TO_ORDER: &str = "You have no squad to order.";

/// **How long a pilot's order stands** (16b cut Y.1; the cut Y plan's decision 7),
/// milliseconds: the ten-minute ceiling, with no key of its own. A human order
/// stands ten minutes or until replaced; the arena's `order_expired` line is the
/// same as for a lance-leader's, and the world's lapse sentence (17b cut L.2) is
/// told to the pilot alone, in their own words: `Squad red-1: your hold order has
/// run out; back on doctrine.`
pub const HUMAN_ORDER_TTL_MS: u32 = crate::deliberation::MAX_TTL_MS;

/// **The squad an `ORDER` names** (16b cut Y.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SquadWord {
    /// `ALL`: every squad the pilot may order.
    All,
    /// A rendered squad id, `red-1` or `blue-2`
    /// ([`crate::deliberation::SquadId::parse`]).
    Id(crate::deliberation::SquadId),
    /// A bare ordinal, `1`: the pilot's own side's squad of that number.
    Ordinal(u16),
}

/// **The objective an `ORDER` names**, in the pilot's words (16b cut Y.1) — the
/// spot and the contact still unresolved, because resolving them is the world's
/// business and comes after authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectiveWords {
    /// `HOLD`.
    Hold,
    /// `ADVANCE <spot>`: every word between `ADVANCE` and the `HOLD FIRE` tail,
    /// joined by one space, as typed (`The Ford`).
    Advance {
        /// The spot's name or letter, as typed.
        spot: String,
    },
    /// `ENGAGE [<contact>]`: at most one word, as typed.
    Engage {
        /// The contact's designator, as typed, or `None` for "what you find".
        contact: Option<String>,
    },
    /// `WITHDRAW`.
    Withdraw,
    /// `SCREEN <bearing>` (16b cut Z.2): the bearing to face, whole degrees
    /// clockwise from north, `0` to `359` — digits only, as `BEARING` prints
    /// one.
    Screen {
        /// The bearing, degrees.
        bearing: u32,
    },
    /// `SUPPORT <callsign>` (16b cut Z.2): exactly one word, as typed, resolved
    /// against the running match's callsigns after authority.
    Support {
        /// The charge's callsign, as typed.
        callsign: String,
    },
    /// `RELEASE` (16b cut Z.2; 16c Q44): hand the squad back to doctrine.
    /// Nothing may follow it, `HOLD FIRE` included.
    Release,
}

impl ObjectiveWords {
    /// The objective's wire word — `crate::deliberation::Objective::as_str`'s,
    /// so the world's watch, the ledger and the sentence name it one way.
    /// `RELEASE` is no objective and answers the release intent's own word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ObjectiveWords::Hold => "hold",
            ObjectiveWords::Advance { .. } => "advance",
            ObjectiveWords::Engage { .. } => "engage",
            ObjectiveWords::Withdraw => "withdraw",
            ObjectiveWords::Screen { .. } => "screen",
            ObjectiveWords::Support { .. } => "support",
            ObjectiveWords::Release => "release",
        }
    }
}

/// **One `ORDER`, parsed** (16b cut Y.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderWords {
    /// Which squad.
    pub squad: SquadWord,
    /// What it is told.
    pub objective: ObjectiveWords,
    /// Whether the trailing `HOLD FIRE` was given.
    pub hold_fire: bool,
}

/// **Parse `ORDER`'s argument** (16b cut Y.1) — pure, so every line of the
/// grammar is pinned without a world:
///
/// ```text
/// ORDER <squad|ALL> HOLD                 [HOLD FIRE]
/// ORDER <squad|ALL> ADVANCE <spot>       [HOLD FIRE]
/// ORDER <squad|ALL> ENGAGE [<contact>]   [HOLD FIRE]
/// ORDER <squad|ALL> WITHDRAW             [HOLD FIRE]
/// ORDER <squad|ALL> SCREEN <bearing>     [HOLD FIRE]
/// ORDER <squad|ALL> SUPPORT <callsign>   [HOLD FIRE]
/// ORDER <squad|ALL> RELEASE
/// ```
///
/// Whitespace-split and case-folded: the keywords and the squad word are
/// compared lower-cased, and a spot, contact or callsign word is kept as typed
/// (all three are resolved case-insensitively later). The trailing `HOLD FIRE`
/// is the last two words **after** the objective word, so `ORDER red-1 HOLD
/// HOLD FIRE` holds and holds fire, and `ORDER red-1 HOLD FIRE` is usage rather
/// than a guess.
///
/// # Errors
///
/// [`CockpitError::Usage`] with [`ORDER_USAGE`] for an empty argument, a squad
/// word that is neither `ALL`, a squad id nor an ordinal, an unknown objective,
/// `ADVANCE` with no spot, `ENGAGE` with more than one word, `SCREEN` with no
/// word, a word that is not all digits or a bearing of 360 and above (16b cut
/// Z.2), `SUPPORT` with no word or two, `RELEASE` with anything after it
/// (`HOLD FIRE` included), and any other trailing word.
pub fn parse_order(rest: &str) -> Result<OrderWords, CockpitError> {
    let usage = || CockpitError::Usage(ORDER_USAGE.to_owned());
    let typed: Vec<&str> = rest.split_whitespace().collect();
    let folded: Vec<String> = typed.iter().map(|word| word.to_ascii_lowercase()).collect();
    let (Some(squad_word), Some(objective_word)) = (folded.first(), folded.get(1)) else {
        return Err(usage());
    };
    let squad = if squad_word == "all" {
        SquadWord::All
    } else if let Some(id) = crate::deliberation::SquadId::parse(squad_word) {
        SquadWord::Id(id)
    } else if let Ok(ordinal) = squad_word.parse::<u16>() {
        SquadWord::Ordinal(ordinal)
    } else {
        return Err(usage());
    };
    // The words after the objective, with the `HOLD FIRE` tail taken off.
    let mut args: &[&str] = typed.get(2..).unwrap_or(&[]);
    let mut hold_fire = false;
    if let [head @ .., hold, fire] = args
        && hold.eq_ignore_ascii_case("hold")
        && fire.eq_ignore_ascii_case("fire")
    {
        hold_fire = true;
        args = head;
    }
    let objective = match objective_word.as_str() {
        "hold" if args.is_empty() => ObjectiveWords::Hold,
        "withdraw" if args.is_empty() => ObjectiveWords::Withdraw,
        "advance" if !args.is_empty() => ObjectiveWords::Advance {
            spot: args.join(" "),
        },
        "engage" => match args {
            [] => ObjectiveWords::Engage { contact: None },
            [one] => ObjectiveWords::Engage {
                contact: Some((*one).to_owned()),
            },
            _ => return Err(usage()),
        },
        // **16b cut Z.2**: one word of digits, below a full turn.
        "screen" => match args {
            [one] if !one.is_empty() && one.bytes().all(|byte| byte.is_ascii_digit()) => {
                match one.parse::<u32>() {
                    Ok(bearing) if bearing < 360 => ObjectiveWords::Screen { bearing },
                    _ => return Err(usage()),
                }
            }
            _ => return Err(usage()),
        },
        "support" => match args {
            [one] => ObjectiveWords::Support {
                callsign: (*one).to_owned(),
            },
            _ => return Err(usage()),
        },
        // Nothing after it: a release holds no fire, because it is no order.
        "release" if args.is_empty() && !hold_fire => ObjectiveWords::Release,
        _ => return Err(usage()),
    };
    Ok(OrderWords {
        squad,
        objective,
        hold_fire,
    })
}

/// **A slot's pod pair** (16b cut Y.1; the cut Y plan's decision 2): pods are
/// claimed in pairs, `1↔2`, `3↔4`, `5↔6` (16b cut Z.3's six a side) — odd `n`
/// with `n + 1`, even `n` with `n − 1`.
/// Saturating at both ends: `0`, which no slot is, pairs with itself.
#[must_use]
pub const fn pair(slot: u8) -> u8 {
    if slot % 2 == 1 {
        slot.saturating_add(1)
    } else {
        slot.saturating_sub(1)
    }
}

/// **What one seat may order** (16b cut Y.1; 16c Q38 read as the cut Y plan's
/// decision 2), read once per `ORDER` in one borrow of the
/// [`crate::match_::MatchBook`].
///
/// **Why the wingman and not "the squad holding my pair".** A pilot's tank is in
/// no squad, and `crate::deliberation::squads_of` deals a side's *machines* in
/// unit-id order into squads of at most `[tacsim.arena].squad_size` (four; 16b
/// cut Z.1), so a side of up to four machines is always **one** squad —
/// so on a side of four machines or fewer "my pair's squad" would hand any pilot whose pair is a
/// machine every machine of the side, and let them silently undo the lead's
/// orders (latest-wins, no radio line). So a non-lead orders exactly one unit:
/// their pod pair's, addressed by the word of the squad that holds it (the
/// arena's own squad filter accepts it) and acknowledged by its callsign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// The side's lead: every roster of the side, in ordinal order.
    Squads(Vec<crate::deliberation::SquadRoster>),
    /// Anyone else whose pod pair a machine flies: that one unit, the squad that
    /// holds it, and its callsign.
    Wingman {
        /// The squad holding the wingman.
        squad: crate::deliberation::SquadId,
        /// The wingman.
        unit: ObjId,
        /// Its callsign, as the arena resolved it.
        callsign: Designator,
    },
    /// Nothing: the pair is a human, empty, or the seat has no slot.
    None,
}

/// **The seat's command** (16b cut Y.1): the running match, the seat's own side
/// (for a bare ordinal) and the [`Command`].
///
/// The centre's slot for the driver gives the side, the slot index and the
/// account; the running record ([`ArenaRegistry::match_of_unit`] on the seat's
/// own tank) gives the squads and the callsigns. A lead
/// ([`crate::match_::lobby::Centre::leads_side`]) commands every roster of the
/// side; anyone else the unit whose callsign is `Slot { team, pair(index) }`
/// **and** that one roster holds — a unit in no roster is a human's tank or one
/// abandoned by `EXIT`, and is nobody's wingman.
pub(crate) fn command_of(
    w: &World,
    seat: &Seat,
) -> (Option<MatchId>, Option<crate::arena::state::Team>, Command) {
    let match_id = w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.match_of_unit(seat.unit));
    let Some(book) = w.module_state::<crate::match_::MatchBook>() else {
        return (match_id, None, Command::None);
    };
    let Some(slot) = book.centre().slot_of_pilot(seat.driver) else {
        return (match_id, None, Command::None);
    };
    let (team, index, account) = (slot.team, slot.index, slot.account);
    let leads = book.centre().leads_side(team, account);
    let Some(record) = match_id.and_then(|match_id| book.get(match_id)) else {
        return (match_id, Some(team), Command::None);
    };
    if leads {
        let mut rosters: Vec<crate::deliberation::SquadRoster> = record
            .squads
            .iter()
            .filter(|roster| roster.id.team == team)
            .cloned()
            .collect();
        rosters.sort_by_key(|roster| roster.id.index);
        return (match_id, Some(team), Command::Squads(rosters));
    }
    let wanted = Designator::Slot {
        team,
        slot: u16::from(pair(index)),
    };
    let wingman = record
        .callsigns
        .iter()
        .find(|(_, callsign)| **callsign == wanted)
        .and_then(|(unit, callsign)| {
            record
                .squads
                .iter()
                .find(|roster| roster.holds(*unit))
                .map(|roster| Command::Wingman {
                    squad: roster.id,
                    unit: *unit,
                    callsign: *callsign,
                })
        });
    (match_id, Some(team), wingman.unwrap_or(Command::None))
}

/// **`ORDER <squad|ALL> <objective> [HOLD FIRE]`** — command the side's machines
/// (16b cut Y.1; doc 16 §3.7, §6; 16c N3, Q38).
///
/// In this order, and the order is the observer rule:
///
/// 1. [`parse_order`] — [`ORDER_USAGE`];
/// 2. [`require_alive`] — a wreck takes no order at all, as every ordering verb;
/// 3. **authority** ([`command_of`]) — a squad word outside the pilot's set is
///    [`NO_SUCH_SQUAD`], byte-identical whether it is the other side's, names
///    nothing, or is the own side's beyond the pilot's authority; `ALL` with an
///    empty set is [`NO_SQUAD_TO_ORDER`];
/// 4. **the objective** — an `ENGAGE` word resolved on the pilot's own held rows
///    ([`resolve_designator`]) and an `ADVANCE` word on the ground's named spots
///    ([`resolve_spot`]); a miss on either is the one byte-identical
///    [`NO_SUCH_CONTACT`], with or without a frame. After authority, so a pilot
///    with no right to order a squad learns nothing about contacts by naming one.
///    A row the pilot's panel classes as their own side drops the hint (the
///    machines never fire on their own side), so the order is a bare `ENGAGE`;
/// 5. **the order** — `issuer: Some(own tank)`, `by: the seat's character`
///    (17b cut L.1, D-UM-52: `CharacterRef::Character { id: seat.driver }`, the
///    author the bridge's admission checks on this same `command_of` rule),
///    [`HUMAN_ORDER_TTL_MS`] as both time to live and horizon, validated against
///    the window;
/// 6. **the span** (16b cut Y.2; doc 16 §2 W8) — the squads the pilot holds a
///    live order on ([`ArenaRegistry::live_orders_of`]) that this order does not
///    name, plus those it names, may not pass `[tacsim.command].max_span`
///    ([`span_refusal`]: [`CockpitError::SpanFull`] when the others alone fill
///    the span, [`CockpitError::SpanAll`] when only `ALL`'s several squads pass
///    it). A replacement of a squad's own live order never counts;
/// 7. **the rate** (16b cut Y.2) — one stamp per `ORDER`
///    ([`ArenaRegistry::note_order_stamp`]), refused past
///    `[tacsim.command].orders_per_minute` in sixty world seconds
///    ([`CockpitError::RateFull`]). Taken after the span and before the
///    fan-out, so an order no unit could be offered still spent it. Both are
///    reads and writes on the world's own RAM: a refusal reaches no pulse;
/// 8. **the fan-out** — [`crate::arena_bridge::forward_order`] per squad (the
///    seat authorised once, on the pilot's own tank), `Busy` when no unit could
///    be offered, and the world's watch on each squad reached
///    ([`ArenaRegistry::note_human_order`]);
/// 9. **the acknowledgement** ([`order_ack`]) to the issuer alone — nothing to
///    the room and nothing to the side: 16c Q38's *no radio line*.
///
/// It writes no document; the arena journals one `ordered` line per unit the
/// order lands on.
fn order(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let words = parse_order(rest)?;
    require_alive(ctx.w, seat.driver)?;

    // (3) Authority, before anything is resolved.
    let (match_id, own_team, command) = command_of(ctx.w, seat);
    let wanted = match words.squad {
        SquadWord::All => None,
        SquadWord::Id(id) => Some(id),
        SquadWord::Ordinal(index) => Some(crate::deliberation::SquadId {
            team: own_team.ok_or(CockpitError::NoSuchSquad)?,
            index,
        }),
    };
    // `(squad, units, who)` per squad the order goes to.
    let targets: Vec<(crate::deliberation::SquadId, Vec<ObjId>, String)> = match (&command, wanted)
    {
        (Command::Squads(rosters), None) => rosters
            .iter()
            .map(|roster| (roster.id, roster.units.clone(), roster.id.to_string()))
            .collect(),
        (Command::Squads(rosters), Some(id)) => rosters
            .iter()
            .find(|roster| roster.id == id)
            .map(|roster| vec![(roster.id, roster.units.clone(), roster.id.to_string())])
            .ok_or(CockpitError::NoSuchSquad)?,
        (
            Command::Wingman {
                squad,
                unit,
                callsign,
            },
            wanted,
        ) => {
            if wanted.is_some_and(|id| id != *squad) {
                return Err(CockpitError::NoSuchSquad);
            }
            vec![(*squad, vec![*unit], callsign.to_string())]
        }
        (Command::None, None) => return Err(CockpitError::NoSquadToOrder),
        (Command::None, Some(_)) => return Err(CockpitError::NoSuchSquad),
    };
    if targets.is_empty() {
        return Err(CockpitError::NoSquadToOrder);
    }
    let Some(match_id) = match_id else {
        return Err(CockpitError::NoSuchSquad);
    };

    // (4) The objective, on the issuer's own scope and the surveyed ground.
    let mut hint: Option<String> = None;
    let mut spot_named: Option<(SmolStr, char)> = None;
    let objective = match &words.objective {
        ObjectiveWords::Hold => crate::deliberation::Objective::Hold,
        ObjectiveWords::Withdraw => crate::deliberation::Objective::Withdraw,
        ObjectiveWords::Engage { contact: None } => {
            crate::deliberation::Objective::Engage { target_hint: None }
        }
        ObjectiveWords::Engage {
            contact: Some(word),
        } => {
            // `named_row`'s rule for a name with no frame: the one refusal.
            let held = last_frame(ctx.w, seat.driver).ok_or(CockpitError::NoSuchContact)?;
            let rows = frame_contacts_held(&held.frame, &held.ranges);
            let id = resolve_designator(&rows, word).ok_or(CockpitError::NoSuchContact)?;
            // **A row the pilot's own panel classes as their own side is no
            // target** (`best_hostile` skips a classified own-side row, so a
            // machine can never fire on it): the hint is dropped and the order
            // is a bare `ENGAGE`, acknowledged and journalled as one, so
            // neither the answer nor the debrief promises a shot that will not
            // come. Not a refusal: the pilot knows their own side, and the
            // squad was still told to engage.
            let row = rows.iter().find(|row| row.id == id);
            if row.is_some_and(|row| row.friendly) {
                crate::deliberation::Objective::Engage { target_hint: None }
            } else {
                hint = row.map(|row| row.designator.to_string());
                crate::deliberation::Objective::Engage {
                    target_hint: Some(id),
                }
            }
        }
        ObjectiveWords::Advance { spot } => {
            // V.1's rule: a word that is neither a contact nor a spot keeps the
            // one refusal.
            let (spots, _) = spots_of(ctx.w, Some(match_id));
            let (index, found) = resolve_spot(&spots, spot).ok_or(CockpitError::NoSuchContact)?;
            let letter = spot_letter(index).ok_or(CockpitError::NoSuchContact)?;
            spot_named = Some((found.name.clone(), letter));
            crate::deliberation::Objective::Advance {
                to: (found.x, found.y),
            }
        }
        // **16b cut Z.2 (16c N6)**: the bearing as typed, in millidegrees.
        ObjectiveWords::Screen { bearing } => crate::deliberation::Objective::Screen {
            facing_mdeg: bearing.saturating_mul(1_000),
        },
        // **16b cut Z.2 (16c N6)**: the charge, resolved against the running
        // match's own callsigns to a tank whose callsign is a slot of the
        // pilot's OWN side — the pilot's own tank included. The other side's
        // callsign, a provisional, a word that is no callsign and a slot
        // nobody flies are one byte-identical refusal, said after authority,
        // so the other side's roster is never learned by naming it. The
        // acknowledgement names the charge by its callsign as the match
        // prints it, not as typed.
        ObjectiveWords::Support { callsign } => {
            let own = own_team.ok_or(CockpitError::NoSuchFriendly)?;
            let (of, designator) = ctx
                .w
                .module_state::<crate::match_::MatchBook>()
                .and_then(|book| book.get(match_id))
                .and_then(|record| {
                    record
                        .callsigns
                        .iter()
                        .find(|(_, designator)| {
                            matches!(designator, Designator::Slot { team, .. } if *team == own)
                                && designator.to_string().eq_ignore_ascii_case(callsign)
                        })
                        .map(|(unit, designator)| (*unit, *designator))
                })
                .ok_or(CockpitError::NoSuchFriendly)?;
            hint = Some(designator.to_string());
            crate::deliberation::Objective::Support { of }
        }
        // **16b cut Z.2 (16c Q44)**: a release carries no order at all; it
        // takes its own road from here (`release`).
        ObjectiveWords::Release => return release(ctx, seat, match_id, targets),
    };

    // (5) The order.
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let spot_word = spot_named.as_ref().map(|(name, _)| name.clone());
    let order_for = |squad: crate::deliberation::SquadId| crate::deliberation::Order {
        squad,
        objective,
        target: None,
        horizon_ms: HUMAN_ORDER_TTL_MS,
        constraints: crate::deliberation::Constraints {
            hold_fire: words.hold_fire,
            max_range_band: None,
        },
        ttl_ms: HUMAN_ORDER_TTL_MS,
        issued_ms: now_ms,
        issuer: Some(seat.unit),
        by: crate::deliberation::CharacterRef::Character { id: seat.driver },
        spot: spot_word.clone(),
    };
    // A spot is on the window by construction, so a refusal here is the bounds
    // check doing its job on something no pilot can type.
    let window = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
        .map(|ground| (ground.terrain.width(), ground.terrain.height()));
    // **16b cut Z.2**: the leash in metres off this ground's own cell, for the
    // acknowledgement — 150 m on the shipped ground.
    let leash_m = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
        .map_or(0, |ground| {
            u32::from(crate::arena::reflex::SUPPORT_LEASH_CELLS)
                .saturating_mul(u32::from(ground.terrain.cell_cm()))
                / 100
        });
    for (squad, _, _) in &targets {
        order_for(*squad)
            .validate(window)
            .map_err(|why| CockpitError::Usage(why.to_string()))?;
    }

    // (6) The span (16b cut Y.2; doc 16 §2 W8), on the world's own watch. The
    // squads this order names are out of `others`, so re-ordering a squad the
    // pilot already holds never counts against them.
    let command_cfg = crate::match_::lobby::command_config(ctx.w).unwrap_or_default();
    let named: std::collections::BTreeSet<crate::deliberation::SquadId> =
        targets.iter().map(|(squad, _, _)| *squad).collect();
    let live = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.live_orders_of(match_id, seat.unit, now_ms))
        .unwrap_or_default();
    let others = live.difference(&named).count();
    if let Some(refusal) = span_refusal(others, named.len(), command_cfg.max_span) {
        return Err(refusal);
    }

    // (7) The rate: one stamp per `ORDER`, spent before the fan-out.
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry
            .note_order_stamp(seat.unit, now_ms, command_cfg.orders_per_minute)
            .map_err(|n| CockpitError::RateFull { n })?;
    }

    // (8) The fan-out: the seat authorised once, on the issuer's own tank.
    let mut reached: Vec<(crate::deliberation::SquadId, String)> = Vec::new();
    for (squad, units, who) in targets {
        let offered = crate::arena_bridge::forward_order(
            ctx,
            seat.driver,
            seat.unit,
            &units,
            order_for(squad),
        )?;
        if offered > 0 {
            reached.push((squad, who));
        }
    }
    if reached.is_empty() {
        return Err(CockpitError::Order(IntentRefusal::Busy));
    }
    let expires_ms = now_ms.saturating_add(i64::from(HUMAN_ORDER_TTL_MS));
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        for (squad, _) in &reached {
            registry.note_human_order(
                match_id,
                *squad,
                SmolStr::new_static(words.objective.as_str()),
                seat.unit,
                crate::deliberation::CharacterRef::Character { id: seat.driver },
                expires_ms,
            );
        }
    }

    // (9) The acknowledgement, to the issuer alone.
    let who: Vec<String> = reached.into_iter().map(|(_, who)| who).collect();
    let line = order_ack(
        &who,
        &words.objective,
        hint.as_deref(),
        spot_named
            .as_ref()
            .map(|(name, letter)| (name.as_str(), *letter)),
        words.hold_fire,
        leash_m,
    );
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **`ORDER <squad|ALL> RELEASE`** (16b cut Z.2; 16c Q44) — hand the squads the
/// pilot may order back to doctrine, from [`order`]'s step 4, after the parse,
/// [`require_alive`] and authority (the same set, and the same
/// [`NO_SUCH_SQUAD`] and [`NO_SQUAD_TO_ORDER`], as every other arm).
///
/// Then, in this order:
///
/// 1. **the rate** — one stamp spent ([`ArenaRegistry::note_order_stamp`]): a
///    release is an `ORDER`, and a pilot who could release freely could spin
///    orders and releases past `[tacsim.command].orders_per_minute`;
/// 2. **no span check**: a release takes no squad, it gives one back;
/// 3. **the fan-out** — [`crate::arena_bridge::forward_release`] per squad (the
///    seat authorised once, on the pilot's own tank), `Busy` when no unit could
///    be offered;
/// 4. **the watch** — [`ArenaRegistry::note_order_released`] per squad reached,
///    which drops the world's watch on it only when this pilot's tank gave the
///    order the watch was kept for;
/// 5. **the acknowledgement** ([`order_ack`] with [`ObjectiveWords::Release`]),
///    to the issuer alone.
///
/// A release to a squad under no order is acknowledged, and the arena applies
/// and journals nothing: the machines are on doctrine already.
fn release(
    ctx: &mut Ctx<'_>,
    seat: &Seat,
    match_id: MatchId,
    targets: Vec<(crate::deliberation::SquadId, Vec<ObjId>, String)>,
) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let command_cfg = crate::match_::lobby::command_config(ctx.w).unwrap_or_default();
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry
            .note_order_stamp(seat.unit, now_ms, command_cfg.orders_per_minute)
            .map_err(|n| CockpitError::RateFull { n })?;
    }
    let mut reached: Vec<(crate::deliberation::SquadId, String)> = Vec::new();
    for (squad, units, who) in targets {
        let offered =
            crate::arena_bridge::forward_release(ctx, seat.driver, seat.unit, &units, squad)?;
        if offered > 0 {
            reached.push((squad, who));
        }
    }
    if reached.is_empty() {
        return Err(CockpitError::Order(IntentRefusal::Busy));
    }
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        for (squad, _) in &reached {
            registry.note_order_released(match_id, *squad, seat.unit);
        }
    }
    let who: Vec<String> = reached.into_iter().map(|(_, who)| who).collect();
    let line = order_ack(&who, &ObjectiveWords::Release, None, None, false, 0);
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **The span's answer** (16b cut Y.2; doc 16 §2 W8): `others` squads the
/// issuer holds a live order on that this order does not name, `named` squads
/// it names, `max_span` the configured span. `None` when `others + named` fits.
///
/// The sentence is chosen by what would pass, never by the squad word, so each
/// says only what is true:
///
/// - `others` alone fill the span — no squad at all can be ordered until one of
///   them runs out, whether the pilot named one or `ALL` (a wingman's `ALL`
///   names one squad, and so may an ex-lead's): [`CockpitError::SpanFull`];
/// - `others` leave room but `ALL`'s several squads pass it — ordering them one
///   at a time does pass: [`CockpitError::SpanAll`]. One squad named can never
///   land here (`others + 1 > max_span` is `others >= max_span`).
#[must_use]
fn span_refusal(others: usize, named: usize, max_span: u32) -> Option<CockpitError> {
    let span = usize::try_from(max_span).unwrap_or(usize::MAX);
    let would = others.saturating_add(named);
    if would <= span {
        None
    } else if others >= span {
        Some(CockpitError::SpanFull { held: others })
    } else {
        Some(CockpitError::SpanAll {
            would,
            span: max_span,
        })
    }
}

/// **`ORDER`'s acknowledgement** (16b cut Y.1) — what the order means, in the
/// doctrine's own words (doc 16 §11 E.1/E.2: an ordered `Withdraw` stands and
/// fights inside its bands unless it holds fire; an `Advance` or a `Hold`
/// suppresses the hostile edges; `HOLD FIRE` stops the trigger and nothing else).
///
/// `who` is the squad ids for a lead (`red-1`, `red-1 and red-2`, `red-1, red-2
/// and red-3`) and the wingman's callsign (`RB`) for anyone else; `hint` is the
/// designator the issuer's own row prints (`BA`); `spot` is `(name, letter)`.
/// Verbatim, with `red-1`:
///
/// ```text
/// Ordered red-1 to hold: it stops where it stands and fires nothing.
/// Ordered red-1 to advance on The Ford (A): it drives there and fights nothing on the way.
/// Ordered red-1 to engage BA: it closes and fires on BA where it can see it, else on what it finds.
/// Ordered red-1 to engage: it closes and fires on what it finds.
/// Ordered red-1 to engage BA, holding fire: it closes and locks but fires nothing.
/// Ordered red-1 to engage, holding fire: it closes and locks on what it finds but fires nothing.
/// Ordered red-1 to withdraw: it breaks for its entry zone and still fights inside its bands.
/// Ordered red-1 to withdraw, holding fire: it breaks for its entry zone and fires nothing.
/// ```
///
/// `HOLD FIRE` on a hold or an advance adds `, holding fire` before the colon and
/// changes nothing else (those arms fire nothing already; the constraint is still
/// set). Every acknowledgement ends ` It stands for 10 minutes or until your next
/// order to it.`
///
/// **16b cut Z.2** (16c N6, Q44): the two squad roles and the release. For a
/// `SUPPORT`, `hint` is the charge's callsign as the match prints it and
/// `leash_m` the leash in metres off the ground's cell (`SUPPORT_LEASH_CELLS ×
/// cell_cm / 100`, 150 on the shipped ground); the bearing prints as `BEARING`
/// prints one. Verbatim, with `red-1`:
///
/// ```text
/// Ordered red-1 to screen 090°: it holds where it stands, faces 090° and fires on what comes inside its arc without giving chase.
/// Ordered red-1 to screen 090°, holding fire: it holds where it stands, faces 090° and locks what comes inside its arc but fires nothing.
/// Ordered red-1 to support RA: it keeps within 150 m of RA and fires first on what RA is fighting.
/// Ordered red-1 to support RA, holding fire: it keeps within 150 m of RA and locks what RA is fighting but fires nothing.
/// Released red-1 to doctrine: it fights on its own reflexes until it is ordered again.
/// Released red-1 and red-2 to doctrine: they fight on their own reflexes until they are ordered again.
/// ```
///
/// The four order lines take the ten-minute tail; the release takes none, and
/// its sentence is true on a server whose machines' own commander may order the
/// squad again at its next signal — which dropping the watch allows on purpose.
/// The plural is the release's alone: the ordered lines keep Y.1's singular.
#[must_use]
pub fn order_ack(
    who: &[String],
    objective: &ObjectiveWords,
    hint: Option<&str>,
    spot: Option<(&str, char)>,
    hold_fire: bool,
    leash_m: u32,
) -> String {
    if matches!(objective, ObjectiveWords::Release) {
        let many = who.len() > 1;
        let who = join_names(who);
        return if many {
            format!(
                "Released {who} to doctrine: they fight on their own reflexes until they are \
                 ordered again."
            )
        } else {
            format!(
                "Released {who} to doctrine: it fights on its own reflexes until it is ordered \
                 again."
            )
        };
    }
    let who = join_names(who);
    let holding = if hold_fire { ", holding fire" } else { "" };
    let said = match (objective, hold_fire) {
        (ObjectiveWords::Hold, _) => {
            format!("to hold{holding}: it stops where it stands and fires nothing.")
        }
        (ObjectiveWords::Advance { .. }, _) => {
            let on = spot.map_or_else(String::new, |(name, letter)| {
                format!(" on {name} ({letter})")
            });
            format!("to advance{on}{holding}: it drives there and fights nothing on the way.")
        }
        (ObjectiveWords::Engage { .. }, false) => match hint {
            Some(hint) => format!(
                "to engage {hint}: it closes and fires on {hint} where it can see it, else on \
                 what it finds."
            ),
            None => "to engage: it closes and fires on what it finds.".to_owned(),
        },
        (ObjectiveWords::Engage { .. }, true) => match hint {
            Some(hint) => {
                format!("to engage {hint}, holding fire: it closes and locks but fires nothing.")
            }
            None => "to engage, holding fire: it closes and locks on what it finds but fires \
                     nothing."
                .to_owned(),
        },
        (ObjectiveWords::Withdraw, false) => {
            "to withdraw: it breaks for its entry zone and still fights inside its bands."
                .to_owned()
        }
        (ObjectiveWords::Withdraw, true) => {
            "to withdraw, holding fire: it breaks for its entry zone and fires nothing.".to_owned()
        }
        (ObjectiveWords::Screen { bearing }, false) => format!(
            "to screen {bearing:03}°: it holds where it stands, faces {bearing:03}° and fires on \
             what comes inside its arc without giving chase."
        ),
        (ObjectiveWords::Screen { bearing }, true) => format!(
            "to screen {bearing:03}°, holding fire: it holds where it stands, faces {bearing:03}° \
             and locks what comes inside its arc but fires nothing."
        ),
        (ObjectiveWords::Support { callsign }, false) => {
            let charge = hint.unwrap_or(callsign);
            format!(
                "to support {charge}: it keeps within {leash_m} m of {charge} and fires first on \
                 what {charge} is fighting."
            )
        }
        (ObjectiveWords::Support { callsign }, true) => {
            let charge = hint.unwrap_or(callsign);
            format!(
                "to support {charge}, holding fire: it keeps within {leash_m} m of {charge} and \
                 locks what {charge} is fighting but fires nothing."
            )
        }
        // Answered above: a release is no order and takes no tail.
        (ObjectiveWords::Release, _) => String::new(),
    };
    let minutes = HUMAN_ORDER_TTL_MS / 60_000;
    format!("Ordered {who} {said} It stands for {minutes} minutes or until your next order to it.")
}

/// `a`, `a and b`, `a, b and c` — the names an acknowledgement lists.
fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

// ---------------------------------------------------------------------------
// The gate and the dispatch
// ---------------------------------------------------------------------------

/// **The contextual gate**: the eighteen verbs exist everywhere and mean
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
        // compares and a sixteen-element scan, and no world read at all.
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

/// Run one cockpit verb: resolve the station first, so every one of the sixteen
/// answers the same way to somebody who is not flying.
///
/// **16b cut U.4 (16c `T-53`)**: the seat is resolved **once**, here, and every
/// verb takes it — each reads the cockpit as [`Seat::driver`] and answers
/// `ctx.actor`.
fn cockpit_command(ctx: &mut Ctx<'_>, verb: &'static str) -> Result<(), CmdError> {
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
                   prints (BA, or X2 before it is identified)."
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
                   condition holds, and tells you each time. FIRE <weapon|ALL> OFF clears it. \
                   See help cockpit, \"Arming a weapon\"."
            .to_owned(),
        // 16b cut Y.1: who may order what, in the help, because the refusal for
        // a squad a pilot may not order is deliberately one bare sentence.
        // 16b cut Z.2: the two squad roles and the release.
        "order" => "ORDER <squad|ALL> <objective>  command your side's machines: HOLD, ADVANCE \
                    <spot>, ENGAGE [<contact>], WITHDRAW, SCREEN <bearing> or SUPPORT \
                    <callsign>, with HOLD FIRE at the end to forbid shooting; RELEASE hands them \
                    back to their own doctrine. A side's lead orders any squad of the side; \
                    anyone else their pod pair's tank."
            .to_owned(),
        // 16b cut F.2 gave both of these more to say, and a help line that
        // undersells a surface is a pilot who never looks at it.
        "weaponstatus" => "WEAPONSTATUS        every weapon you carry: ready or cycling, its \
                           rounds, its own chance against your lock, and that lock's range band \
                           — and, under an armed weapon, what it is armed to do."
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

/// Register the cockpit on the **tacsim** module: eighteen verbs and one gate.
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
    // the sixteen selectors, seven stage at most one arena intent each (cut
    // O.2's `EMCON` among them, which also stamps a bridge hint, and 17b cut
    // M1a.4's `DRIVE`), cut Y.1's `ORDER` stages one per unit it fans out to,
    // seven — cut Q.1's `RANGE` and `BEARING` and cut Q.2's `SCAN` among them —
    // only read the last frame, and cut AP.3's `WATCH` sets or clears one RAM
    // flag on the bridge's registry; none of the sixteen writes a document.
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
            // reclaimed, an `EXIT` cannot, even after a drop.
            help: "EXIT               climb out of the pod you are in: the seat, its control \
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
    // 17b cut M1a.2: the class word moved to `arena::view`, and only the
    // tests here still build a `Classification`.
    use crate::arena::state::Classification;

    /// The wire's `catchup` object, as `catchup_lines.json` writes it (16b cut
    /// Y.4) — a serde mirror, so the fixture is read as data and not as a
    /// second copy of the builder.
    #[derive(serde::Deserialize)]
    struct WireWords {
        condition: String,
        armour: i64,
        internals: i64,
    }

    #[derive(serde::Deserialize)]
    struct WireSection {
        id: String,
        then: WireWords,
        now: WireWords,
    }

    #[derive(serde::Deserialize)]
    struct WireRounds {
        weapon: String,
        rounds: u32,
    }

    #[derive(serde::Deserialize)]
    struct WireMachine {
        state: String,
        goal: Option<(u16, u16)>,
        objective: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct WireCatchup {
        away_s: i64,
        cell_then: Option<(u16, u16)>,
        cell_now: Option<(u16, u16)>,
        power_then: Power,
        power_now: Power,
        wreck: bool,
        contacts_new: Vec<String>,
        contacts_lost: Vec<String>,
        contacts_more: u32,
        sections: Vec<WireSection>,
        sections_more: u32,
        rounds_spent: Vec<WireRounds>,
        rounds_more: u32,
        machine: Option<WireMachine>,
    }

    #[derive(serde::Deserialize)]
    struct WireCase {
        callsign: String,
        catchup: WireCatchup,
        lines: Vec<String>,
    }

    /// A designator as a frame prints it, read back: `X<n>`, or a team letter
    /// and a bijective base-26 slot.
    fn designator(word: &str) -> Designator {
        if let Some(number) = word.strip_prefix(crate::arena::state::PROVISIONAL_PREFIX) {
            return Designator::Provisional(number.parse().expect("a provisional number"));
        }
        let mut letters = word.chars();
        let team = match letters.next() {
            Some('R') => crate::arena::state::Team::Red,
            Some('B') => crate::arena::state::Team::Blue,
            other => panic!("no side is lettered {other:?}"),
        };
        let slot = letters.fold(0_u16, |slot, letter| {
            slot * 26 + (u16::from(u8::try_from(letter).expect("ASCII")) - u16::from(b'A') + 1)
        });
        Designator::Slot { team, slot }
    }

    fn words(wire: &WireWords) -> SectionWords {
        SectionWords {
            condition: SmolStr::new(&wire.condition),
            armour: wire.armour,
            internals: wire.internals,
        }
    }

    fn decoded(wire: &WireCatchup) -> Catchup {
        Catchup {
            away_ms: wire.away_s * 1_000,
            cell_then: wire.cell_then,
            cell_now: wire.cell_now,
            power_then: wire.power_then,
            power_now: wire.power_now,
            wreck: wire.wreck,
            contacts_new: wire.contacts_new.iter().map(|w| designator(w)).collect(),
            contacts_lost: wire.contacts_lost.iter().map(|w| designator(w)).collect(),
            contacts_more: wire.contacts_more,
            sections: wire
                .sections
                .iter()
                .map(|s| (SmolStr::new(&s.id), words(&s.then), words(&s.now)))
                .collect(),
            sections_more: wire.sections_more,
            rounds_spent: wire
                .rounds_spent
                .iter()
                .map(|r| (SmolStr::new(&r.weapon), r.rounds))
                .collect(),
            rounds_more: wire.rounds_more,
            machine: wire
                .machine
                .as_ref()
                .map(|m| crate::arena::catchup::MachineWords {
                    state: SmolStr::new(&m.state),
                    goal: m.goal,
                    objective: m.objective.as_deref().map(SmolStr::new),
                }),
        }
    }

    /// **16b cut Y.4**: `catchup_lines` says every case of the committed parity
    /// fixture word for word — the file the TUI's `catchup_words` and the web's
    /// `catchupLines` are pinned to as well — and `catchup_json` writes each
    /// case's object back exactly as the fixture holds it, so the fixture is
    /// the wire's own shape.
    #[test]
    fn y4_catchup_lines_say_every_case_of_the_parity_fixture() {
        let cases: Vec<WireCase> = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/catchup_lines.json"
        ))
        .expect("catchup_lines.json is an array of cases");
        assert_eq!(cases.len(), 9, "the nine shapes the fixture pins");
        let raw: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/catchup_lines.json"
        ))
        .expect("the same file, as values");
        for (case, raw) in cases.iter().zip(&raw) {
            let report = decoded(&case.catchup);
            assert_eq!(
                catchup_lines(designator(&case.callsign), &report),
                case.lines,
                "{}",
                case.lines.first().map_or("", String::as_str)
            );
            assert_eq!(
                Some(&catchup_json(&report)),
                raw.get("catchup"),
                "the fixture is the wire's shape"
            );
        }
    }

    /// **16b cut Y.4**: `away_s` is the floor of the milliseconds, on the wire
    /// and on line 1 alike, and a negative count reads nought.
    #[test]
    fn y4_the_duration_is_the_wires_floor() {
        let mut report = decoded(
            &serde_json::from_value::<WireCatchup>(serde_json::json!({
                "away_s": 0, "cell_then": null, "cell_now": null,
                "power_then": "off", "power_now": "off", "wreck": false,
                "contacts_new": [], "contacts_lost": [], "contacts_more": 0,
                "sections": [], "sections_more": 0,
                "rounds_spent": [], "rounds_more": 0, "machine": null
            }))
            .expect("a wire object"),
        );
        report.away_ms = 134_999;
        let callsign = designator("RA");
        assert_eq!(
            catchup_lines(callsign, &report).first().map(String::as_str),
            Some("Back aboard RA after 2 m 14 s away.")
        );
        assert_eq!(
            catchup_json(&report).get("away_s"),
            Some(&serde_json::json!(134))
        );
        report.away_ms = -1;
        assert_eq!(
            catchup_lines(callsign, &report).first().map(String::as_str),
            Some("Back aboard RA after 0 s away.")
        );
    }

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
            // 16b cut O.2.
            Refusal::NoJammer,
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
            armed: None,
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
            aspect: SmolStr::new_static("front"),
            odds: crate::arena::view::OddsCell::Unarmed,
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
            16,
            "doc 16 section 3.7's nine, cut Q.1's two read verbs, cut Q.2's SCAN, cut Y.1's \
             ORDER, cut O.2's EMCON, cut AP.3's WATCH and 17b cut M1a.4's DRIVE"
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
            armed: None,
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
        // 16b cut AE.3: a platform with no rows at all (a drone) — a bare
        // `FIRE` names nothing, so neither does the refusal; a named weapon
        // keeps the sentence that names it.
        assert_eq!(choose_weapon(&[], ""), Err(CockpitError::NoWeapon));
        assert_eq!(
            choose_weapon(&[], "  ").map_err(|err| err.to_string()),
            Err("This unit carries no weapon.".to_owned())
        );
        assert_eq!(
            choose_weapon(&[], "cannon").map_err(|err| err.to_string()),
            Err("That unit carries no weapon called `cannon`.".to_owned())
        );
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

    /// **16b cut AE.2**: the weapon rows a fielded unit type's frame carries, in
    /// component order, loaded and ready — what `FIRE` chooses among.
    fn fielded_rows(entry: &str) -> Vec<WeaponRow> {
        let catalog = crate::definitions::Catalog::build_fielded(&[], crate::NS)
            .unwrap_or_else(|error| panic!("the fielded content binds: {error}"));
        let key = mush_core::CanonicalId::new(entry).unwrap_or_else(|_| panic!("{entry}"));
        let (Some(spec), Some(binding)) = (catalog.specs.get(&key), catalog.bindings.get(&key))
        else {
            panic!("{entry} is fielded")
        };
        let cells = crate::definitions::to_tactical_unit(spec, binding, ObjId(31))
            .unwrap_or_else(|why| panic!("{entry} instantiates: {why}"));
        crate::match_::weapon_summaries(&catalog, &cells)
            .into_iter()
            .map(|weapon| WeaponRow {
                rounds: i64::from(weapon.count_per_load),
                rounds_max: i64::from(weapon.count_per_load),
                ..weapon_row(&weapon.id)
            })
            .collect()
    }

    /// **16b cut AE.2**: `FIRE`'s default is the content author's choice by
    /// component order, under HEAD's unchanged rule — the cannon if carried,
    /// else the first weapon. The scout car answers its autocannon, the heavy
    /// its cannon (its rack is `FIRE missiles`, or the prefix `FIRE mis`), the
    /// stock tank its cannon and the laser tank its beam; a one-word id has no
    /// short name, so `FIRE auto` is a prefix; and `WEAPONSTATUS` prints the
    /// new rows in the existing format.
    #[test]
    fn ae2_the_bare_fire_is_the_authors_choice_by_component_order() {
        use crate::definitions::content;
        let chosen = |entry: &str, word: &str| {
            choose_weapon(&fielded_rows(entry), word)
                .map(|row| row.id.to_string())
                .map_err(|err| err.to_string())
        };
        assert_eq!(chosen(content::SCOUT_MK1, ""), Ok("autocannon".to_owned()));
        assert_eq!(chosen(content::HEAVY_MK1, ""), Ok("cannon".to_owned()));
        assert_eq!(chosen(content::LINE_TANK, ""), Ok("cannon".to_owned()));
        assert_eq!(chosen(content::LINE_TANK_LASER, ""), Ok("laser".to_owned()));
        assert_eq!(
            chosen(content::SCOUT_MK1, "auto"),
            Ok("autocannon".to_owned())
        );
        assert_eq!(
            chosen(content::SCOUT_MK1, "mg"),
            Ok("machine_gun".to_owned())
        );
        assert_eq!(
            chosen(content::HEAVY_MK1, "missiles"),
            Ok("missiles".to_owned())
        );
        assert_eq!(chosen(content::HEAVY_MK1, "mis"), Ok("missiles".to_owned()));
        assert_eq!(
            chosen(content::HEAVY_MK1, "m"),
            Err("`m` could be machine_gun or missiles; say more.".to_owned())
        );
        assert_eq!(weapon_initials("autocannon"), None);
        assert_eq!(weapon_initials("missiles"), None);

        // The rows, in the existing format.
        let scout = fielded_rows(content::SCOUT_MK1);
        assert_eq!(
            scout
                .iter()
                .map(|row| row.render(&scout))
                .collect::<Vec<_>>(),
            vec![
                "autocannon  ready  240 of 240 round(s)",
                "machine_gun (mg)  ready  2000 of 2000 round(s)",
            ]
        );
        let mut heavy = fielded_rows(content::HEAVY_MK1);
        assert_eq!(
            heavy
                .iter()
                .map(|row| row.render(&heavy))
                .collect::<Vec<_>>(),
            vec![
                "cannon  ready  40 of 40 round(s)",
                "machine_gun (mg)  ready  2000 of 2000 round(s)",
                "missiles  ready  6 of 6 round(s)",
            ]
        );
        if let Some(rack) = heavy.iter_mut().find(|row| row.id == "missiles") {
            rack.ready = false;
            rack.ready_in_ms = 10_000;
            rack.rounds = 5;
        }
        assert_eq!(
            heavy.get(2).map(|row| row.render(&heavy)).as_deref(),
            Some("missiles  cycling (10 s)  5 of 6 round(s)")
        );
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
        assert_eq!(COCKPIT_VERBS.len(), 16);
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

    // -- 16b cut V.1: the named ground ------------------------------------------

    /// A spot at a cell, as the arena carries one.
    fn spot(name: &str, x: u16, y: u16) -> KeySpot {
        KeySpot {
            name: SmolStr::new(name),
            x,
            y,
        }
    }

    /// The shipped proving ground's five, in authored order (the pack's own
    /// `key_spots`; the fifth, The Bluff, is 16b cut AE.1's).
    fn shipped() -> Vec<KeySpot> {
        vec![
            spot("The Ford", 64, 97),
            spot("The Crossroads", 129, 97),
            spot("North Barn", 106, 194),
            spot("East Compound", 208, 148),
            spot("The Bluff", 144, 111),
        ]
    }

    /// A pose on a cell's centre at ten-metre cells, heading `heading_mdeg`.
    fn pose_on(x: i64, y: i64, heading_mdeg: u32) -> mush_sim::data::Pose {
        let mut pose = mush_sim::data::Pose::at(
            x * 1_000 + 500,
            y * 1_000 + 500,
            0,
            mush_sim::data::passive_rules_value(serde_json::json!({}))
                .expect("an empty posture blob"),
        );
        pose.heading_mdeg = heading_mdeg;
        pose
    }

    /// **The alphabet** (16b cut V.1): as long as the cap, every letter its own,
    /// and no letter a byte the map already draws or the pilot already reads.
    ///
    /// This is about the SPOT letters only. Two existing markers already share a
    /// byte with the ground — `x` (a hostile) is `Rubble`'s glyph and `+` (a
    /// friendly) is `Door`'s — and that is not this cut's to fix.
    #[test]
    fn v1_the_spot_alphabet_is_the_cap_and_collides_with_nothing() {
        assert_eq!(
            SPOT_LETTERS.len(),
            crate::definitions::arena::MAX_KEY_SPOTS,
            "the cap is the alphabet's length"
        );
        for (i, a) in SPOT_LETTERS.iter().enumerate() {
            for b in SPOT_LETTERS.iter().skip(i + 1) {
                assert_ne!(a, b, "{a} is in the alphabet twice");
            }
        }
        let mut taken: Vec<char> = mush_atlas::TerrainClass::ALL
            .iter()
            .map(|class| class.glyph())
            .collect();
        for glyph in [YOU_GLYPH, UNKNOWN_GLYPH, FRIEND_GLYPH, HOSTILE_GLYPH] {
            taken.extend(glyph.chars());
        }
        taken.push(crate::arena::state::PROVISIONAL_PREFIX);
        for letter in SPOT_LETTERS {
            assert!(
                !taken.contains(&letter),
                "the spot letter {letter} is a byte the map or the scope already uses"
            );
        }
        assert_eq!(spot_letter(0), Some('A'));
        assert_eq!(
            spot_letter(3),
            Some('E'),
            "D is a platform, so the fourth is E"
        );
        assert_eq!(spot_letter(21), Some('Z'));
        assert_eq!(spot_letter(22), None, "past the alphabet is nothing");
    }

    /// **`resolve_spot`** (16b cut V.1): the whole name or the one letter, case
    /// folded and trimmed, and nothing else — no prefix, and never a word with a
    /// designator's shape.
    #[test]
    fn v1_a_spot_resolves_by_its_whole_name_or_its_letter_and_nothing_else() {
        let spots = shipped();
        let at = |word: &str| resolve_spot(&spots, word).map(|(index, _)| index);
        assert_eq!(at("The Ford"), Some(0));
        assert_eq!(at("the ford"), Some(0));
        assert_eq!(at("  THE FORD "), Some(0));
        assert_eq!(at("north barn"), Some(2));
        assert_eq!(at("a"), Some(0));
        assert_eq!(at("B"), Some(1));
        assert_eq!(at("c"), Some(2));
        // The fourth spot is `E`: `D` is not a spot letter at all.
        assert_eq!(at("e"), Some(3));
        assert_eq!(at("d"), None);
        // No prefix, no fragment, no near miss.
        for word in ["the", "The Fo", "Ford", "The Fjord", "north", "", "  "] {
            assert_eq!(at(word), None, "{word:?} names no spot");
        }
        // A designator is never a spot, in either case.
        for word in ["RA", "BA", "X1", "x1", "ra"] {
            assert_eq!(at(word), None, "{word} is a designator's shape");
        }
        // The spot answered is the one at that index.
        assert_eq!(
            resolve_spot(&spots, "e").map(|(_, spot)| spot.name.as_str()),
            Some("East Compound")
        );
    }

    /// **Range and bearing to a spot's cell centre** (16b cut V.1), on the
    /// canonical fixture's pose — the pilot on cell (6, 4) at ten-metre cells,
    /// so at (6,500, 4,500) cm — against hand-worked numbers.
    ///
    /// * `A` at (25, 3): its centre is (25,500, 3,500), so dx = 19,000 and
    ///   dy = −1,000. The range is `isqrt(19,000² + 1,000²)` =
    ///   `isqrt(362,000,000)` = 19,026 cm, which rounds to 190 m at the 10 m
    ///   step. The bearing is `atan2(19,000, −1,000)` = 90° + `atan(1/19)` =
    ///   90° + 3.0128° = 93.013°, so 93,013 mdeg.
    /// * `C` at (50, 25): dx = 44,000 and dy = 21,000, so `isqrt(1,936,000,000 +
    ///   441,000,000)` = `isqrt(2,377,000,000)` = 48,754 cm (490 m), bearing
    ///   `atan2(44, 21)` = 64.486° = 64,486 mdeg.
    #[test]
    fn v1_a_spot_s_range_and_bearing_are_read_to_its_cell_centre() {
        let pose = pose_on(6, 4, 45_000);
        let ford = spot("The Ford", 25, 3);
        let woods = spot("The Woods", 50, 25);
        assert_eq!(spot_range_cm(&pose, &ford, 1_000), 19_026);
        assert_eq!(spot_bearing_mdeg(&pose, &ford, 1_000), 93_013);
        assert_eq!(range_m_rounded(19_026, SPOT_RANGE_STEP_M), 190);
        assert_eq!(spot_range_cm(&pose, &woods, 1_000), 48_754);
        assert_eq!(spot_bearing_mdeg(&pose, &woods, 1_000), 64_486);
        assert_eq!(range_m_rounded(48_754, SPOT_RANGE_STEP_M), 490);
        // The two sentences, verbatim.
        assert_eq!(
            spot_range_line(&ford, 'A', 190),
            "Range to The Ford (A): 190 m."
        );
        assert_eq!(
            spot_bearing_line(&ford, 'A', 93, relative_mdeg(45_000, 93_013) / 1_000),
            "Bearing to The Ford (A): 093° (rel 048°)."
        );
        // Standing on the spot's own cell is range 0 and bearing 0, and the
        // range still rounds to one step (`range_m_rounded` never says nought).
        let on = pose_on(25, 3, 0);
        assert_eq!(spot_range_cm(&on, &ford, 1_000), 0);
        assert_eq!(spot_bearing_mdeg(&on, &ford, 1_000), 0);
    }

    /// **`nearest_spot`** (16b cut V.1): the least range, ties to the lowest
    /// authored index, and nothing on ground that names none.
    #[test]
    fn v1_the_nearest_spot_ties_to_the_lowest_index() {
        let pose = pose_on(6, 4, 0);
        // West and east of the pilot, one cell each: the same range.
        let spots = vec![
            spot("West Gate", 5, 4),
            spot("East Gate", 7, 4),
            spot("Far Field", 60, 60),
        ];
        let (index, found, range) = nearest_spot(&spots, &pose, 1_000).expect("a spot");
        assert_eq!((index, found.name.as_str(), range), (0, "West Gate", 1_000));
        // Authored the other way round, the first is still the one kept.
        let flipped = vec![spots[1].clone(), spots[0].clone(), spots[2].clone()];
        assert_eq!(
            nearest_spot(&flipped, &pose, 1_000).map(|(index, spot, _)| (index, spot.name.clone())),
            Some((0, SmolStr::new("East Gate")))
        );
        // A strictly nearer later spot wins.
        let nearer = vec![spot("Far Field", 60, 60), spot("East Gate", 7, 4)];
        assert_eq!(
            nearest_spot(&nearer, &pose, 1_000).map(|(index, _, _)| index),
            Some(1)
        );
        assert!(nearest_spot(&[], &pose, 1_000).is_none());
        // The line: `West Gate` is due west, 10 m.
        assert_eq!(
            nearest_spot_line(&spots, &pose, 1_000).as_deref(),
            Some("  nearest spot: West Gate (A), 10 m at 270°")
        );
        assert_eq!(nearest_spot_line(&[], &pose, 1_000), None);
        // The shipped muster: red slot 1 on (6, 97), 58 cells short of the ford
        // on the same row — 580 m due east.
        assert_eq!(
            nearest_spot_line(&shipped(), &pose_on(6, 97, 90_000), 1_000).as_deref(),
            Some("  nearest spot: The Ford (A), 580 m at 090°")
        );
    }

    /// **`VIEW`'s off-window line** (16b cut V.1), verbatim: nothing when every
    /// spot is drawn (or there are none), one spot, and two in authored order.
    #[test]
    fn v1_the_off_window_line_names_every_undrawn_spot_in_authored_order() {
        let pose = pose_on(6, 4, 45_000);
        let spots = vec![
            spot("The Ford", 25, 3),
            spot("The Roof", 14, 7),
            spot("The Woods", 50, 25),
        ];
        assert_eq!(off_window_spots_line(&[], &[], &pose, 1_000), None);
        assert_eq!(
            off_window_spots_line(&spots, &[0, 1, 2], &pose, 1_000),
            None,
            "every spot drawn"
        );
        assert_eq!(
            off_window_spots_line(&spots, &[0, 1], &pose, 1_000).as_deref(),
            Some("Off the window: C The Woods, 490 m at 064°.")
        );
        assert_eq!(
            off_window_spots_line(&spots, &[1], &pose, 1_000).as_deref(),
            Some("Off the window: A The Ford, 190 m at 093°; C The Woods, 490 m at 064°.")
        );
        // The shipped ground from the red muster, whose 39 by 20 window (x 0..38)
        // draws none of the five. 16b cut AE.1's fifth, The Bluff at (144, 111),
        // is 138 cells east and 14 north of (6, 97): 138.7 cells, 1390 m to the
        // ten, at 084°.
        assert_eq!(
            off_window_spots_line(&shipped(), &[], &pose_on(6, 97, 90_000), 1_000).as_deref(),
            Some(
                "Off the window: A The Ford, 580 m at 090°; B The Crossroads, 1230 m at 090°; \
                 C North Barn, 1390 m at 045°; E East Compound, 2080 m at 075°; \
                 F The Bluff, 1390 m at 084°."
            )
        );
    }

    /// **`spot_markers`** (16b cut V.1): clipped at every edge exactly as
    /// `contact_markers` clips — the west and south edges are the corner itself,
    /// the east and north edges are `corner + size`, exclusive — with cells
    /// relative to the corner, one legend entry per marker, and the authored
    /// indexes drawn.
    #[test]
    fn v1_spot_markers_clip_at_every_edge_and_carry_their_legend() {
        let spots = vec![
            spot("West Out", 9, 25),   // one column west of the window
            spot("West In", 10, 25),   // on its west edge
            spot("South Out", 20, 19), // one row south
            spot("South In", 20, 20),  // on its south edge
            spot("East In", 48, 30),   // on its east edge (10 + 39 - 1)
            spot("East Out", 49, 30),  // one column past it
            spot("North In", 30, 39),  // on its north edge (20 + 20 - 1)
            spot("North Out", 30, 40), // one row past it
        ];
        let (markers, legend, drawn) = spot_markers(&spots, (10, 20), (39, 20));
        assert_eq!(drawn, vec![1, 3, 4, 6]);
        let cells: Vec<serde_json::Value> = markers
            .iter()
            .filter_map(|marker| marker.get("cell").cloned())
            .collect();
        assert_eq!(
            cells,
            vec![
                serde_json::json!([0, 5]),
                serde_json::json!([10, 0]),
                serde_json::json!([38, 10]),
                serde_json::json!([20, 19]),
            ]
        );
        // The letter is the AUTHORED index's, not the drawn order's.
        assert_eq!(
            markers.first(),
            Some(&serde_json::json!({
                "id": "B",
                "name": "West In",
                "cell": [0, 5],
                "glyph": "B",
                "you": false,
                "spot": true,
            }))
        );
        assert_eq!(
            legend,
            vec![
                serde_json::json!(["B", "West In"]),
                serde_json::json!(["E", "South In"]),
                serde_json::json!(["F", "East In"]),
                serde_json::json!(["H", "North In"]),
            ]
        );
        // Nothing to draw, nothing drawn.
        let (none, no_legend, none_drawn) = spot_markers(&[], (0, 0), (39, 20));
        assert!(none.is_empty() && no_legend.is_empty() && none_drawn.is_empty());
    }

    /// **`drawn_spots`** (16b cut V.1) reads the drawing's own spots back off
    /// `units` — every marker flagged `spot`, by its letter's authored index —
    /// **including one a later marker covers**: a spot under a contact is drawn
    /// (then overwritten), so `VIEW` never lists it off the window. The pilot's
    /// own marker and the contacts are never counted, whatever their `id`.
    #[test]
    fn v1_the_drawn_spots_are_every_spot_marker_even_a_covered_one() {
        let window = serde_json::json!({
            "units": [
                { "id": "A", "name": "The Ford", "cell": [25, 3], "glyph": "A", "you": false, "spot": true },
                { "id": "C", "name": "The Roof", "cell": [14, 7], "glyph": "C", "you": false, "spot": true },
                { "id": "@", "cell": [19, 10], "glyph": "@", "you": true },
                // A contact on C's cell: it owns the cell, and C is still drawn.
                { "id": "X3", "cell": [14, 7], "glyph": "?", "you": false },
                // A contact whose designator starts with a spot letter is not one.
                { "id": "BA", "cell": [30, 12], "glyph": "x", "you": false },
            ]
        });
        assert_eq!(drawn_spots(&window), vec![0, 2]);
        assert!(drawn_spots(&serde_json::json!({})).is_empty());
        // Read against it, the off-window line names only the spot not drawn.
        let spots = vec![
            spot("The Ford", 25, 3),
            spot("North Gate", 7, 4),
            spot("The Roof", 14, 7),
        ];
        assert_eq!(
            off_window_spots_line(&spots, &drawn_spots(&window), &pose_on(6, 4, 0), 1_000)
                .as_deref(),
            Some("Off the window: B North Gate, 10 m at 090°.")
        );
    }

    // -- 16b cut AG.1: `VIEW BRIEF`, the window in sentences --------------------
    //
    // Pure pins over hand-painted ground: none of these builds a `Frame` (it
    // derives `Clone` and `Debug` only and this module has no builder), so every
    // test that needs one lives in `tests/cockpit.rs` over `fixture::frame()`.

    /// One cell of `class`, passable, at the datum.
    fn ag1_cell(class: mush_atlas::TerrainClass) -> crate::arena::state::TerrainCell {
        crate::arena::state::TerrainCell {
            class,
            elev_cm: 0,
            move_cost: 100,
        }
    }

    /// A one-row yard, `width` cells at `cell_cm`, road on even columns and grass
    /// on odd — so no two neighbouring cells merge into one run.
    fn ag1_striped(width: u16, cell_cm: u16) -> ArenaTerrain {
        let cells = (0..width)
            .map(|x| {
                ag1_cell(if x % 2 == 0 {
                    mush_atlas::TerrainClass::Road
                } else {
                    mush_atlas::TerrainClass::Grass
                })
            })
            .collect();
        ArenaTerrain::new(width, 1, cell_cm, cells).expect("a one-row yard")
    }

    /// The fixture yard's shape, re-stated (a test binary's `support::yard`
    /// cannot be imported here): 96 by 64 of grass at 1,000 cm, a road along
    /// y = 10, water x 20..=29 by y 2..=5, a wall block x 12..=15 by y 6..=8,
    /// light woods x 40..=59 by y 20..=29.
    fn ag1_yard() -> ArenaTerrain {
        use mush_atlas::TerrainClass;
        let (width, height) = (96_u16, 64_u16);
        let mut cells =
            vec![ag1_cell(TerrainClass::Grass); usize::from(width) * usize::from(height)];
        let mut paint = |xs: std::ops::Range<u16>, ys: std::ops::Range<u16>, class| {
            for y in ys {
                for x in xs.clone() {
                    cells[usize::from(y) * usize::from(width) + usize::from(x)] = ag1_cell(class);
                }
            }
        };
        paint(0..96, 10..11, TerrainClass::Road);
        paint(20..30, 2..6, TerrainClass::Water);
        paint(12..16, 6..9, TerrainClass::Wall);
        paint(40..60, 20..30, TerrainClass::WoodsLight);
        ArenaTerrain::new(width, height, 1_000, cells).expect("the fixture yard's shape")
    }

    /// The eight sentences a pilot at `from` says of `terrain`'s window.
    fn ag1_sentences(
        terrain: &ArenaTerrain,
        from: (u16, u16),
        origin: (u16, u16),
        size: (u16, u16),
    ) -> Vec<(String, RayEnd)> {
        BRIEF_BEARINGS
            .iter()
            .map(|(bearing, dx, dy)| {
                let (runs, end) = ground_ray(terrain, from, (*dx, *dy), origin, size);
                let line =
                    ground_line(bearing, &runs, end, terrain.cell_cm(), *dx != 0 && *dy != 0);
                (line, end)
            })
            .collect()
    }

    /// **The class words** (16b cut AG.1): every class has a prose word, and
    /// none is a label's snake case. The exhaustive match is the compile-time
    /// guard for a new class; this is the run-time one for an empty word.
    #[test]
    fn ag1_class_prose_covers_every_class_with_a_prose_word() {
        for class in mush_atlas::TerrainClass::ALL {
            let word = class_prose(class);
            assert!(!word.trim().is_empty(), "{class:?} has no word");
            assert!(!word.contains('_'), "{class:?} reads as a label: {word}");
        }
        assert_eq!(
            class_prose(mush_atlas::TerrainClass::WoodsHeavy),
            "heavy woods"
        );
        assert_eq!(
            class_prose(mush_atlas::TerrainClass::WaterDeep),
            "deep water"
        );
        assert_eq!(class_prose(mush_atlas::TerrainClass::Ground), "bare ground");
        assert_eq!(class_prose(mush_atlas::TerrainClass::Door), "doorway");
    }

    /// **Cells to centimetres** (16b cut AG.1): `steps × cell_cm` straight, and
    /// root two to three places on a diagonal, rounded at the brief's step.
    #[test]
    fn ag1_ray_cm_is_cells_at_the_cell_size_and_root_two_on_a_diagonal() {
        let step = brief_step_m(1_000);
        let straight: Vec<(u64, u64)> = [4, 5, 6, 15, 19]
            .into_iter()
            .map(|k| {
                let cm = ray_cm(k, 1_000, false);
                (cm, range_m_rounded(cm, step))
            })
            .collect();
        assert_eq!(
            straight,
            vec![
                (4_000, 40),
                (5_000, 50),
                (6_000, 60),
                (15_000, 150),
                (19_000, 190)
            ]
        );
        let diagonal: Vec<(u64, u64)> = [4, 5, 6, 15, 19]
            .into_iter()
            .map(|k| {
                let cm = ray_cm(k, 1_000, true);
                (cm, range_m_rounded(cm, step))
            })
            .collect();
        assert_eq!(
            diagonal,
            vec![
                (5_656, 60),
                (7_070, 70),
                (8_484, 80),
                (21_210, 210),
                (26_866, 270)
            ]
        );
        assert_eq!(ray_cm(0, 1_000, true), 0);
        assert_eq!(
            ray_cm(u32::MAX, u16::MAX, true),
            u64::from(u32::MAX) * u64::from(u16::MAX) * 1_414 / 1_000
        );
    }

    /// **The brief's step** (16b cut AG.1): the spot step at the shipped cell
    /// and every coarser one, and no coarser than a cell below it — so at 500 cm
    /// nineteen one-cell runs read nineteen different reaches.
    #[test]
    fn ag1_the_brief_step_is_the_spot_step_and_no_coarser_than_a_cell() {
        let steps: Vec<u64> = [50, 100, 200, 500, 1_000, 2_500]
            .into_iter()
            .map(brief_step_m)
            .collect();
        assert_eq!(steps, vec![1, 1, 2, 5, 10, 10]);
        let yard = ag1_striped(60, 500);
        let (runs, end) = ground_ray(&yard, (19, 0), (1, 0), (0, 0), (39, 1));
        assert_eq!(end, RayEnd::Window, "(39, 0) is on the arena");
        let reaches: Vec<u64> = runs
            .iter()
            .map(|run| range_m_rounded(ray_cm(run.steps, 500, false), brief_step_m(500)))
            .collect();
        assert_eq!(reaches, (1..=19).map(|k| k * 5).collect::<Vec<u64>>());
        assert!(
            reaches.windows(2).all(|pair| pair[0] < pair[1]),
            "every reach strictly past the last: {reaches:?}"
        );
    }

    /// **One class is one run, to the window on every side** (16b cut AG.1): a
    /// 60-by-40 grass yard and the window `view_window` computes for a pilot at
    /// (29, 20) — origin (10, 10), 39 by 20 — which ends inside the arena every
    /// way. Nine cells north, ten south (`you.dy` is 10), nineteen east and
    /// west; nine diagonal steps are 12,726 cm (`130 m`), ten 14,140 (`140 m`).
    #[test]
    fn ag1_a_ray_over_one_class_is_one_run_to_the_window() {
        let yard = ArenaTerrain::uniform(60, 40, 1_000, ag1_cell(mush_atlas::TerrainClass::Grass))
            .expect("a uniform yard");
        let said = ag1_sentences(&yard, (29, 20), (10, 10), (39, 20));
        let lines: Vec<&str> = said.iter().map(|(line, _)| line.as_str()).collect();
        assert_eq!(
            lines,
            vec![
                "North: 90 m of grass, then beyond the window.",
                "North-east: 130 m of grass, then beyond the window.",
                "East: 190 m of grass, then beyond the window.",
                "South-east: 140 m of grass, then beyond the window.",
                "South: 100 m of grass, then beyond the window.",
                "South-west: 140 m of grass, then beyond the window.",
                "West: 190 m of grass, then beyond the window.",
                "North-west: 130 m of grass, then beyond the window.",
            ]
        );
        assert!(said.iter().all(|(_, end)| *end == RayEnd::Window));
        let (runs, _) = ground_ray(&yard, (29, 20), (1, 1), (10, 10), (39, 20));
        assert_eq!(
            runs,
            vec![GroundRun {
                class: mush_atlas::TerrainClass::Grass,
                steps: 9
            }]
        );
    }

    /// **Alternating classes merge nothing** (16b cut AG.1): nineteen one-cell
    /// runs east, reaches 10 to 190 m, eighteen joins and the tail — nineteen
    /// `then`s.
    #[test]
    fn ag1_a_ray_over_alternating_classes_merges_nothing() {
        let yard = ag1_striped(60, 1_000);
        let (runs, end) = ground_ray(&yard, (19, 0), (1, 0), (0, 0), (39, 1));
        assert_eq!(runs.len(), 19, "cells 20..=38, one run each");
        assert_eq!(end, RayEnd::Window, "(39, 0) is on the arena");
        assert_eq!(
            runs.iter().map(|run| run.steps).collect::<Vec<u32>>(),
            (1..=19).collect::<Vec<u32>>()
        );
        let line = ground_line("East", &runs, end, 1_000, false);
        assert!(
            line.starts_with("East: 10 m of road, then grass to 20 m, then road to 30 m"),
            "{line}"
        );
        assert!(
            line.ends_with(", then road to 190 m, then beyond the window."),
            "{line}"
        );
        assert_eq!(line.matches("then").count(), 19, "{line}");
    }

    /// **The arena's edge is said as the edge** (16b cut AG.1): the fixture
    /// yard, the pilot at (6, 4) and the clamped window (0, 0), 39 by 20 — the
    /// eight sentences of the fixture's golden, verbatim.
    #[test]
    fn ag1_a_ray_at_the_arena_edge_says_the_edge() {
        let said = ag1_sentences(&ag1_yard(), (6, 4), (0, 0), (39, 20));
        let lines: Vec<&str> = said.iter().map(|(line, _)| line.as_str()).collect();
        assert_eq!(
            lines,
            vec![
                "North: 50 m of grass, then road to 60 m, then grass to 150 m, then beyond the window.",
                "North-east: 70 m of grass, then road to 80 m, then grass to 210 m, then beyond the \
                 window.",
                "East: 130 m of grass, then water to 230 m, then grass to 320 m, then beyond the \
                 window.",
                "South-east: 60 m of grass, then the edge of the arena.",
                "South: 40 m of grass, then the edge of the arena.",
                "South-west: 60 m of grass, then the edge of the arena.",
                "West: 60 m of grass, then the edge of the arena.",
                "North-west: 70 m of grass, then road to 80 m, then the edge of the arena.",
            ]
        );
        let ends: Vec<RayEnd> = said.iter().map(|(_, end)| *end).collect();
        assert_eq!(
            ends,
            vec![
                RayEnd::Window,
                RayEnd::Window,
                RayEnd::Window,
                RayEnd::Arena,
                RayEnd::Arena,
                RayEnd::Arena,
                RayEnd::Arena,
                RayEnd::Arena,
            ]
        );
    }

    /// **A pilot on the window's last cell says only the tail** (16b cut AG.1):
    /// at (0, 0) of a window at (0, 0) — the one case production makes — west
    /// and south are `the edge of the arena`. Then, a SYNTHETIC rectangle
    /// `view_window` never produces (a pilot on a window edge that is not the
    /// arena's): the north-east corner of a window inside a larger arena says
    /// `beyond the window` with no run.
    #[test]
    fn ag1_a_pilot_on_the_window_s_last_cell_says_only_the_tail() {
        let yard = ag1_yard();
        let said = ag1_sentences(&yard, (0, 0), (0, 0), (39, 20));
        let at = |bearing: &str| {
            said.iter()
                .map(|(line, _)| line.as_str())
                .find(|line| line.starts_with(&format!("{bearing}:")))
                .map(str::to_owned)
        };
        assert_eq!(at("West").as_deref(), Some("West: the edge of the arena."));
        assert_eq!(
            at("South").as_deref(),
            Some("South: the edge of the arena.")
        );
        assert_eq!(
            at("South-west").as_deref(),
            Some("South-west: the edge of the arena.")
        );
        // Synthetic: view_window's clamp never leaves a pilot here.
        let wide = ArenaTerrain::uniform(60, 40, 1_000, ag1_cell(mush_atlas::TerrainClass::Grass))
            .expect("a uniform yard");
        let corner = ag1_sentences(&wide, (48, 29), (10, 10), (39, 20));
        let lines: Vec<&str> = corner.iter().map(|(line, _)| line.as_str()).collect();
        assert_eq!(lines.first().copied(), Some("North: beyond the window."));
        assert_eq!(
            lines.get(1).copied(),
            Some("North-east: beyond the window.")
        );
        assert_eq!(lines.get(2).copied(), Some("East: beyond the window."));
    }

    /// **A contact sentence is the row's own fields** (16b cut AG.1): the four
    /// fixture contacts, with a friendly mark and an alias on two of them to
    /// show that neither prints.
    #[test]
    fn ag1_the_contact_sentence_is_the_row_s_own_fields() {
        use crate::arena::state::Team;
        let row = |id: u64,
                   designator: Designator,
                   alias: Option<u16>,
                   class: &str,
                   (bearing_deg, relative_deg): (u32, u32),
                   band: &str,
                   range_m: u64,
                   age_ms: i64,
                   friendly: bool| ContactRow {
            id: ContactId(id),
            designator,
            alias,
            class: SmolStr::new(class),
            bearing_deg,
            relative_deg,
            band: SmolStr::new(band),
            range_m,
            age_ms,
            cell: None,
            friendly,
            aspect: SmolStr::new_static("front"),
            odds: crate::arena::view::OddsCell::Unarmed,
        };
        let rows = [
            row(
                1,
                Designator::Slot {
                    team: Team::Blue,
                    slot: 1,
                },
                Some(1),
                "blue",
                (45, 0),
                "far",
                450,
                1_200,
                false,
            ),
            row(
                2,
                Designator::Slot {
                    team: Team::Red,
                    slot: 2,
                },
                None,
                "red",
                (270, 225),
                "close",
                100,
                400,
                true,
            ),
            row(
                3,
                Designator::Provisional(3),
                None,
                "unknown",
                (315, 270),
                "effective",
                200,
                3_500,
                false,
            ),
            row(
                4,
                Designator::Provisional(4),
                None,
                "unknown",
                (30, 345),
                "far",
                600,
                6_000,
                false,
            ),
        ];
        let said: Vec<String> = rows.iter().map(contact_sentence).collect();
        assert_eq!(
            said,
            vec![
                "BA, blue: bearing 045° (rel 000°), far, 450 m, 1 s old.",
                "RB, red: bearing 270° (rel 225°), close, 100 m, 0 s old.",
                "X3, unknown: bearing 315° (rel 270°), effective, 200 m, 3 s old.",
                "X4, unknown: bearing 030° (rel 345°), far, 600 m, 6 s old.",
            ]
        );
    }

    /// **The in-window line names the named spots, in authored order** (16b cut
    /// AG.1), with `spot_clause`'s figures: A The Ford from (6, 4) is 190 m at
    /// 093°; B The Roof at (14, 7) is dx 8,000, dy 3,000, `isqrt` 8,544 cm (90 m
    /// to the ten) at `atan2(8, 3)` = 69.4°, so 069°.
    #[test]
    fn ag1_in_window_spots_line_names_the_named_spots_in_authored_order() {
        let pose = pose_on(6, 4, 45_000);
        let spots = vec![
            spot("The Ford", 25, 3),
            spot("The Roof", 14, 7),
            spot("The Woods", 50, 25),
        ];
        assert_eq!(in_window_spots_line(&spots, &[], &pose, 1_000), None);
        assert_eq!(
            in_window_spots_line(&spots, &[0], &pose, 1_000).as_deref(),
            Some("In the window: A The Ford, 190 m at 093°.")
        );
        assert_eq!(
            in_window_spots_line(&spots, &[1], &pose, 1_000).as_deref(),
            Some("In the window: B The Roof, 90 m at 069°."),
            "a named spot that is not authored index 0"
        );
        let both = Some("In the window: A The Ford, 190 m at 093°; B The Roof, 90 m at 069°.");
        assert_eq!(
            in_window_spots_line(&spots, &[0, 1], &pose, 1_000).as_deref(),
            both
        );
        assert_eq!(
            in_window_spots_line(&spots, &[1, 0], &pose, 1_000).as_deref(),
            both,
            "authored order, not the order of `named`"
        );
        assert_eq!(in_window_spots_line(&[], &[0], &pose, 1_000), None);
    }

    /// **The header is the footer's facts** (16b cut AG.1), read off the
    /// payload: the fixture's window, and the one `view_window` produces for a
    /// pilot at (76, 97) on the shipped 256-by-256 ground (`y0 = 97 − 10`).
    #[test]
    fn ag1_the_header_reads_the_footer_s_facts() {
        let fixture = serde_json::json!({
            "origin_cell": [0, 0],
            "width": 39,
            "height": 20,
            "extent": [96, 64],
            "you": [6, 4],
            "cell_cm": 1000,
        });
        assert_eq!(
            brief_header("RA", 45_000, &fixture),
            "Window around RA: cells 0 to 38 east and 0 to 19 north of 96 by 64; you at (6, 4), \
             heading 045°; 10 m a cell."
        );
        let shipped = serde_json::json!({
            "origin_cell": [57, 87],
            "width": 39,
            "height": 20,
            "extent": [256, 256],
            "you": [19, 10],
            "cell_cm": 1000,
        });
        assert_eq!(
            brief_header("RA", 270_000, &shipped),
            "Window around RA: cells 57 to 95 east and 87 to 106 north of 256 by 256; you at \
             (76, 97), heading 270°; 10 m a cell."
        );
        // A payload with no keys at all reads as nought, and does not panic.
        assert_eq!(
            brief_header("RA", 0, &serde_json::json!({})),
            "Window around RA: cells 0 to 0 east and 0 to 0 north of 0 by 0; you at (0, 0), \
             heading 000°; 0 m a cell."
        );
    }

    /// **`named_spots` is the legend's spots, in authored indexes** (16b cut
    /// AG.1): a drawn-and-covered spot is drawn and not named; and with A off
    /// the window, `B` is `units[0]` and authored index 1 — the case that tells
    /// an authored index from a position in `units`.
    #[test]
    fn ag1_named_spots_are_the_legend_s_in_authored_indexes() {
        let window = serde_json::json!({
            "units": [
                { "id": "A", "name": "The Ford", "cell": [25, 3], "glyph": "A", "you": false, "spot": true },
                { "id": "B", "name": "The Roof", "cell": [14, 7], "glyph": "B", "you": false, "spot": true },
                { "id": "#101", "cell": [6, 4], "glyph": "@", "you": true },
                { "id": "X3", "name": "unknown", "cell": [14, 7], "glyph": "?", "you": false },
            ],
            "legend": [[".", "grass"], ["@", "you"], ["?", "unknown"], ["A", "The Ford"]],
        });
        assert_eq!(drawn_spots(&window), vec![0, 1]);
        assert_eq!(named_spots(&window), vec![0]);
        let crossroads = serde_json::json!({
            "units": [
                { "id": "B", "name": "The Roof", "cell": [3, 4], "glyph": "B", "you": false, "spot": true },
                { "id": "C", "name": "The Woods", "cell": [9, 9], "glyph": "C", "you": false, "spot": true },
                { "id": "#101", "cell": [19, 10], "glyph": "@", "you": true },
                { "id": "X1", "name": "unknown", "cell": [3, 4], "glyph": "?", "you": false },
            ],
            "legend": [["@", "you"], ["?", "unknown"], ["C", "The Woods"]],
        });
        assert_eq!(drawn_spots(&crossroads), vec![1, 2]);
        assert_eq!(named_spots(&crossroads), vec![2]);
        assert!(named_spots(&serde_json::json!({})).is_empty());
    }

    // -----------------------------------------------------------------------
    // 16b cut Y.1 — `ORDER`'s grammar, the pod pair and the acknowledgement
    // -----------------------------------------------------------------------

    fn y1_red(index: u16) -> crate::deliberation::SquadId {
        crate::deliberation::SquadId {
            team: crate::arena::state::Team::Red,
            index,
        }
    }

    fn y1_words(squad: SquadWord, objective: ObjectiveWords, hold_fire: bool) -> OrderWords {
        OrderWords {
            squad,
            objective,
            hold_fire,
        }
    }

    /// **16b cut Y.1**: every line of the grammar, both squad forms and `ALL`,
    /// case-folded; a spot of two words with and without the `HOLD FIRE` tail;
    /// and every usage error answering [`ORDER_USAGE`] verbatim.
    #[test]
    fn y1_parse_order_reads_every_line_of_the_grammar() {
        assert_eq!(
            parse_order("red-1 HOLD"),
            Ok(y1_words(
                SquadWord::Id(y1_red(1)),
                ObjectiveWords::Hold,
                false
            ))
        );
        assert_eq!(
            parse_order("  Blue-2   hold  hold FIRE "),
            Ok(y1_words(
                SquadWord::Id(crate::deliberation::SquadId {
                    team: crate::arena::state::Team::Blue,
                    index: 2,
                }),
                ObjectiveWords::Hold,
                true
            ))
        );
        assert_eq!(
            parse_order("1 WITHDRAW"),
            Ok(y1_words(
                SquadWord::Ordinal(1),
                ObjectiveWords::Withdraw,
                false
            ))
        );
        assert_eq!(
            parse_order("1 withdraw HOLD FIRE"),
            Ok(y1_words(
                SquadWord::Ordinal(1),
                ObjectiveWords::Withdraw,
                true
            ))
        );
        assert_eq!(
            parse_order("ALL engage"),
            Ok(y1_words(
                SquadWord::All,
                ObjectiveWords::Engage { contact: None },
                false
            ))
        );
        assert_eq!(
            parse_order("all ENGAGE ba"),
            Ok(y1_words(
                SquadWord::All,
                ObjectiveWords::Engage {
                    contact: Some("ba".to_owned())
                },
                false
            ))
        );
        assert_eq!(
            parse_order("red-1 ENGAGE BA HOLD FIRE"),
            Ok(y1_words(
                SquadWord::Id(y1_red(1)),
                ObjectiveWords::Engage {
                    contact: Some("BA".to_owned())
                },
                true
            ))
        );
        assert_eq!(
            parse_order("red-1 ENGAGE HOLD FIRE"),
            Ok(y1_words(
                SquadWord::Id(y1_red(1)),
                ObjectiveWords::Engage { contact: None },
                true
            ))
        );
        assert_eq!(
            parse_order("red-1 ADVANCE The Ford"),
            Ok(y1_words(
                SquadWord::Id(y1_red(1)),
                ObjectiveWords::Advance {
                    spot: "The Ford".to_owned()
                },
                false
            ))
        );
        assert_eq!(
            parse_order("1 advance The   Ford hold fire"),
            Ok(y1_words(
                SquadWord::Ordinal(1),
                ObjectiveWords::Advance {
                    spot: "The Ford".to_owned()
                },
                true
            ))
        );
        assert_eq!(
            parse_order("red-1 ADVANCE a"),
            Ok(y1_words(
                SquadWord::Id(y1_red(1)),
                ObjectiveWords::Advance {
                    spot: "a".to_owned()
                },
                false
            ))
        );
        // Every usage error, one sentence.
        for bad in [
            "",
            "   ",
            "red-1",
            "red-1 DANCE",
            "HOLD",
            "green-1 HOLD",
            "squad HOLD",
            "red-1 ADVANCE",
            "red-1 ADVANCE HOLD FIRE",
            "red-1 ENGAGE BA XB",
            "red-1 HOLD now",
            "red-1 HOLD FIRE",
            "red-1 WITHDRAW HOLD",
            "red-1 WITHDRAW FIRE",
        ] {
            assert_eq!(
                parse_order(bad),
                Err(CockpitError::Usage(ORDER_USAGE.to_owned())),
                "{bad:?}"
            );
        }
        assert_eq!(
            CockpitError::Usage(ORDER_USAGE.to_owned()).to_string(),
            "Usage: ORDER <squad|ALL> HOLD | ADVANCE <spot> | ENGAGE [<contact>] | WITHDRAW \
             | SCREEN <bearing> | SUPPORT <callsign> [HOLD FIRE], or ORDER <squad|ALL> RELEASE"
        );
        assert_eq!(CockpitError::NoSuchSquad.to_string(), "No such squad.");
        assert_eq!(
            CockpitError::NoSquadToOrder.to_string(),
            "You have no squad to order."
        );
    }

    /// **16b cut Y.2** (doc 16 §2 W8): the span's two sentences and the rate's,
    /// verbatim — each says only what is true: waiting is what frees a span, and
    /// the grammar takes one squad or `ALL`.
    #[test]
    fn y2_the_span_and_rate_refusals_read_verbatim() {
        assert_eq!(
            CockpitError::SpanFull { held: 1 }.to_string(),
            "You hold orders on 1 squad(s) already; wait for one to run out, or RELEASE one."
        );
        assert_eq!(
            CockpitError::SpanAll { would: 5, span: 4 }.to_string(),
            "ORDER ALL would put 5 squads under you; your span is 4. Order them one at a time."
        );
        assert_eq!(
            CockpitError::RateFull { n: 12 }.to_string(),
            "You have given 12 orders in the last minute; wait."
        );
    }

    /// **16b cut Y.2** (doc 16 §2 W8): the span's sentence is chosen by what
    /// would pass, not by the squad word. `ALL`'s *one at a time* only when one
    /// at a time does pass (the others leave room); *wait for one to run out*
    /// whenever the others alone fill the span — including an `ALL` that names
    /// one squad, as a wingman's does.
    #[test]
    fn y2_the_span_sentence_says_what_would_pass() {
        // Inside the span, a replacement included (named, not other): nothing.
        assert_eq!(span_refusal(0, 1, 1), None);
        assert_eq!(span_refusal(3, 1, 4), None);
        assert_eq!(span_refusal(0, 4, 4), None);
        // One squad named past the span — or `ALL` naming one squad, which is
        // the same count: the others fill it, so waiting, never "one at a
        // time", which would be refused as well.
        assert_eq!(
            span_refusal(1, 1, 1),
            Some(CockpitError::SpanFull { held: 1 })
        );
        assert_eq!(
            span_refusal(4, 1, 4),
            Some(CockpitError::SpanFull { held: 4 })
        );
        // `ALL` naming several past the span with room for one: one at a time.
        assert_eq!(
            span_refusal(2, 3, 4),
            Some(CockpitError::SpanAll { would: 5, span: 4 })
        );
        assert_eq!(
            span_refusal(0, 5, 4),
            Some(CockpitError::SpanAll { would: 5, span: 4 })
        );
        // `ALL` naming several while the others already fill it: waiting.
        assert_eq!(
            span_refusal(4, 3, 4),
            Some(CockpitError::SpanFull { held: 4 })
        );
    }

    /// **16b cut Z.2** (16c N6, Q44): `SCREEN <bearing>`, `SUPPORT <callsign>`
    /// and `RELEASE` read with and without the `HOLD FIRE` tail where the arm
    /// takes one; `SCREEN` with no word, a word that is not all digits, or 360
    /// and above, `SUPPORT` with no word or two, and `RELEASE` with any tail
    /// (`HOLD FIRE` included) are usage, verbatim; and the usage line fits the
    /// TUI notices ring's 200-character line.
    #[test]
    fn z2_parse_order_reads_the_screen_the_support_and_the_release() {
        assert_eq!(
            parse_order("red-1 SCREEN 90"),
            Ok(y1_words(
                SquadWord::Id(y1_red(1)),
                ObjectiveWords::Screen { bearing: 90 },
                false
            ))
        );
        assert_eq!(
            parse_order("1 screen 359 hold fire"),
            Ok(y1_words(
                SquadWord::Ordinal(1),
                ObjectiveWords::Screen { bearing: 359 },
                true
            ))
        );
        assert_eq!(
            parse_order("ALL screen 000"),
            Ok(y1_words(
                SquadWord::All,
                ObjectiveWords::Screen { bearing: 0 },
                false
            ))
        );
        assert_eq!(
            parse_order("1 SUPPORT ra"),
            Ok(y1_words(
                SquadWord::Ordinal(1),
                ObjectiveWords::Support {
                    callsign: "ra".to_owned()
                },
                false
            ))
        );
        assert_eq!(
            parse_order("red-1 support RA HOLD FIRE"),
            Ok(y1_words(
                SquadWord::Id(y1_red(1)),
                ObjectiveWords::Support {
                    callsign: "RA".to_owned()
                },
                true
            ))
        );
        assert_eq!(
            parse_order("red-1 RELEASE"),
            Ok(y1_words(
                SquadWord::Id(y1_red(1)),
                ObjectiveWords::Release,
                false
            ))
        );
        assert_eq!(
            parse_order("all release"),
            Ok(y1_words(SquadWord::All, ObjectiveWords::Release, false))
        );
        for bad in [
            "red-1 SCREEN",
            "red-1 SCREEN 360",
            "red-1 SCREEN 1000",
            "red-1 SCREEN east",
            "red-1 SCREEN -90",
            "red-1 SCREEN +90",
            "red-1 SCREEN 90 45",
            "red-1 SCREEN HOLD FIRE",
            "red-1 SUPPORT",
            "red-1 SUPPORT RA RB",
            "red-1 SUPPORT HOLD FIRE",
            "red-1 RELEASE HOLD FIRE",
            "red-1 RELEASE now",
            "red-1 RELEASE RA",
        ] {
            assert_eq!(
                parse_order(bad),
                Err(CockpitError::Usage(ORDER_USAGE.to_owned())),
                "{bad:?}"
            );
        }
        assert!(
            ORDER_USAGE.chars().count() <= 200,
            "the usage line wraps inside the notices ring: {}",
            ORDER_USAGE.chars().count()
        );
        assert_eq!(
            CockpitError::NoSuchFriendly.to_string(),
            "No tank of your side flies that callsign."
        );
        assert_eq!(ObjectiveWords::Screen { bearing: 1 }.as_str(), "screen");
        assert_eq!(
            ObjectiveWords::Support {
                callsign: String::new()
            }
            .as_str(),
            "support"
        );
        assert_eq!(ObjectiveWords::Release.as_str(), "release");
    }

    /// **16b cut Z.2**: the four new order acknowledgements and the release's,
    /// verbatim — the bearing printed as `BEARING` prints one, the leash in
    /// metres, the charge by its callsign as the match prints it, the order
    /// lines with the ten-minute tail and the release with none, singular for
    /// one squad or a wingman and plural for several.
    #[test]
    fn z2_the_acknowledgements_say_what_the_roles_and_the_release_mean() {
        const TAIL: &str = " It stands for 10 minutes or until your next order to it.";
        let one = vec!["red-1".to_owned()];
        let screen = ObjectiveWords::Screen { bearing: 90 };
        let support = ObjectiveWords::Support {
            callsign: "ra".to_owned(),
        };
        let cases = [
            (
                order_ack(&one, &screen, None, None, false, 150),
                "Ordered red-1 to screen 090°: it holds where it stands, faces 090° and fires on \
                 what comes inside its arc without giving chase.",
            ),
            (
                order_ack(&one, &screen, None, None, true, 150),
                "Ordered red-1 to screen 090°, holding fire: it holds where it stands, faces \
                 090° and locks what comes inside its arc but fires nothing.",
            ),
            (
                order_ack(&one, &support, Some("RA"), None, false, 150),
                "Ordered red-1 to support RA: it keeps within 150 m of RA and fires first on \
                 what RA is fighting.",
            ),
            (
                order_ack(&one, &support, Some("RA"), None, true, 150),
                "Ordered red-1 to support RA, holding fire: it keeps within 150 m of RA and \
                 locks what RA is fighting but fires nothing.",
            ),
        ];
        for (said, want) in cases {
            assert_eq!(said, format!("{want}{TAIL}"));
        }
        assert_eq!(
            order_ack(&one, &ObjectiveWords::Release, None, None, false, 150),
            "Released red-1 to doctrine: it fights on its own reflexes until it is ordered again."
        );
        assert_eq!(
            order_ack(
                &["RB".to_owned()],
                &ObjectiveWords::Release,
                None,
                None,
                false,
                0
            ),
            "Released RB to doctrine: it fights on its own reflexes until it is ordered again."
        );
        assert_eq!(
            order_ack(
                &["red-1".to_owned(), "red-2".to_owned()],
                &ObjectiveWords::Release,
                None,
                None,
                false,
                0
            ),
            "Released red-1 and red-2 to doctrine: they fight on their own reflexes until they \
             are ordered again."
        );
    }

    /// **16b cut Y.1**: pods are claimed in pairs.
    #[test]
    fn y1_the_pod_pair_is_odd_with_the_next_and_even_with_the_last() {
        assert_eq!(pair(1), 2);
        assert_eq!(pair(2), 1);
        assert_eq!(pair(3), 4);
        assert_eq!(pair(4), 3);
        assert_eq!(pair(7), 8);
        assert_eq!(pair(0), 0, "no slot is 0; it pairs with itself");
        assert_eq!(pair(u8::MAX), u8::MAX, "saturating at the top");
        assert_eq!(HUMAN_ORDER_TTL_MS, 600_000);
    }

    /// **16b cut Y.1**: the acknowledgement, verbatim, for every arm — with and
    /// without a hint, with `HOLD FIRE` — for one, two and three squads, and for
    /// a wingman's callsign.
    #[test]
    fn y1_the_acknowledgement_says_what_the_order_means() {
        const TAIL: &str = " It stands for 10 minutes or until your next order to it.";
        let one = vec!["red-1".to_owned()];
        let ack =
            |objective: &ObjectiveWords,
             hint: Option<&str>,
             spot: Option<(&str, char)>,
             hold_fire: bool| order_ack(&one, objective, hint, spot, hold_fire, 150);
        let hold = ObjectiveWords::Hold;
        let advance = ObjectiveWords::Advance {
            spot: "the ford".to_owned(),
        };
        let engage = ObjectiveWords::Engage {
            contact: Some("ba".to_owned()),
        };
        let engage_any = ObjectiveWords::Engage { contact: None };
        let withdraw = ObjectiveWords::Withdraw;
        let cases = [
            (
                ack(&hold, None, None, false),
                "Ordered red-1 to hold: it stops where it stands and fires nothing.",
            ),
            (
                ack(&hold, None, None, true),
                "Ordered red-1 to hold, holding fire: it stops where it stands and fires \
                 nothing.",
            ),
            (
                ack(&advance, None, Some(("The Ford", 'A')), false),
                "Ordered red-1 to advance on The Ford (A): it drives there and fights nothing \
                 on the way.",
            ),
            (
                ack(&advance, None, Some(("The Ford", 'A')), true),
                "Ordered red-1 to advance on The Ford (A), holding fire: it drives there and \
                 fights nothing on the way.",
            ),
            (
                ack(&engage, Some("BA"), None, false),
                "Ordered red-1 to engage BA: it closes and fires on BA where it can see it, \
                 else on what it finds.",
            ),
            (
                ack(&engage_any, None, None, false),
                "Ordered red-1 to engage: it closes and fires on what it finds.",
            ),
            (
                ack(&engage, Some("BA"), None, true),
                "Ordered red-1 to engage BA, holding fire: it closes and locks but fires \
                 nothing.",
            ),
            (
                ack(&engage_any, None, None, true),
                "Ordered red-1 to engage, holding fire: it closes and locks on what it finds \
                 but fires nothing.",
            ),
            (
                ack(&withdraw, None, None, false),
                "Ordered red-1 to withdraw: it breaks for its entry zone and still fights \
                 inside its bands.",
            ),
            (
                ack(&withdraw, None, None, true),
                "Ordered red-1 to withdraw, holding fire: it breaks for its entry zone and \
                 fires nothing.",
            ),
        ];
        for (said, want) in cases {
            assert_eq!(said, format!("{want}{TAIL}"));
        }
        // Two and three squads for a lead, a callsign for a wingman.
        let two = vec!["red-1".to_owned(), "red-2".to_owned()];
        assert_eq!(
            order_ack(&two, &hold, None, None, false, 150),
            format!(
                "Ordered red-1 and red-2 to hold: it stops where it stands and fires nothing.{TAIL}"
            )
        );
        let three = vec!["red-1".to_owned(), "red-2".to_owned(), "red-3".to_owned()];
        assert_eq!(
            order_ack(&three, &withdraw, None, None, false, 150),
            format!(
                "Ordered red-1, red-2 and red-3 to withdraw: it breaks for its entry zone and \
                 still fights inside its bands.{TAIL}"
            )
        );
        let wingman = vec!["RB".to_owned()];
        assert_eq!(
            order_ack(&wingman, &engage, Some("X2"), None, false, 150),
            format!(
                "Ordered RB to engage X2: it closes and fires on X2 where it can see it, else \
                 on what it finds.{TAIL}"
            )
        );
    }

    /// **16b cut Y.1**: the thirteenth selector is registered, walked by the
    /// gate, and carries its help line.
    #[test]
    fn y1_order_is_the_thirteenth_selector() {
        // 16b cut O.2 put `emcon` after it, so it is the thirteenth and no
        // longer the last; 16b cut AP.3 put `watch` after that, and 17b cut
        // M1a.4 `drive` after `watch`.
        assert_eq!(COCKPIT_VERBS.len(), 16);
        assert_eq!(COCKPIT_VERBS.get(12), Some(&"order"));
        assert_eq!(
            cockpit_help("order"),
            "ORDER <squad|ALL> <objective>  command your side's machines: HOLD, ADVANCE <spot>, \
             ENGAGE [<contact>], WITHDRAW, SCREEN <bearing> or SUPPORT <callsign>, with HOLD \
             FIRE at the end to forbid shooting; RELEASE hands them back to their own \
             doctrine. A side's lead orders any squad of the side; anyone else their pod pair's \
             tank."
        );
    }

    /// **16b cut O.2** (16c N12): the fourteenth selector's words, pinned — the
    /// help that says what each position costs, the `STATUS` line, the local
    /// refusal and the arena's own, which are one sentence after the prefix.
    #[test]
    fn o2_emcon_is_the_fourteenth_selector_and_its_words_are_pinned() {
        use crate::arena::perception::RefusalDetail;
        // 16b cut AP.3 appended `watch` after it, so it is the fourteenth and
        // no longer the last; 17b cut M1a.4's `drive` follows `watch`.
        assert_eq!(COCKPIT_VERBS.get(13), Some(&"emcon"));
        assert_eq!(
            cockpit_help("emcon"),
            "EMCON [on|off]      switch your jamming pod, if you carry one: on jams every sight \
             inside its bubble, yours among them; off clears your own sight and hides you from \
             nothing. On at check-out."
        );
        assert!(
            cockpit_help("status").contains(", your pod's switch if you carry one"),
            "{}",
            cockpit_help("status")
        );
        assert_eq!(emcon_status_line(true), "  emcon on: the pod radiates");
        assert_eq!(emcon_status_line(false), "  emcon off: the pod is silent");
        assert_eq!(
            CockpitError::NoJammer.to_string(),
            "This tank carries no jamming pod."
        );
        assert_eq!(
            refusal_line("emcon", Refusal::NoJammer, None, RefusalDetail::None),
            "EMCON refused. This tank carries no jamming pod."
        );
        assert_eq!(Refusal::NoJammer.as_str(), "no_jammer");
    }

    /// **17b cut M1a.4** (doc 17 §4 D-UM-10): the sixteenth selector, its help
    /// line, its four replies, its usage line, its two refusals, the goal each
    /// word forwards ([`drive_goal`]: a spot's name and its letter the spot's
    /// own cell, of two spots, beside the reply naming it), the motor's
    /// two notices — each rendered through [`notice_line`] — and the arena's
    /// own refusal of a `DRIVE`, which names the verb through
    /// [`IntentKind::as_str`].
    #[test]
    fn m1a4_drive_is_the_sixteenth_selector_and_its_words_are_pinned() {
        use crate::arena::perception::RefusalDetail;
        assert_eq!(COCKPIT_VERBS.len(), 16);
        assert_eq!(COCKPIT_VERBS.get(15), Some(&"drive"));
        assert_eq!(
            cockpit_help("drive"),
            "DRIVE <spot|x,y>    drive to a named spot of the ground, by its name or its map \
             letter, or to an arena cell, on the machine's own route-finding; HEADING or SPEED \
             takes the helm back. DRIVE HOLD stops and holds here; DRIVE OFF lets go."
        );
        assert_eq!(
            DRIVE_USAGE,
            "DRIVE <spot|x,y|HOLD|OFF>; STATUS names the nearest spot."
        );
        assert_eq!(DRIVE_HOLDING, "Holding here.");
        assert_eq!(DRIVE_RELEASED, "DRIVE off; the helm is yours.");
        assert_eq!(
            DRIVE_NOT_A_GOAL,
            "DRIVE refused. That is not a spot, a cell, HOLD or OFF."
        );
        assert_eq!(
            DRIVE_OFF_WINDOW,
            "DRIVE refused. That cell is not on the window."
        );
        assert_eq!(DRIVE_ARRIVED, "DRIVE: arrived.");
        assert_eq!(DRIVE_NO_ROUTE, "DRIVE: no route from here; holding.");
        // The cell parser: two unsigned integers, by comma or by space.
        assert_eq!(drive_cell("12,34"), Some((12, 34)));
        assert_eq!(drive_cell("12, 34"), Some((12, 34)));
        assert_eq!(drive_cell("12 34"), Some((12, 34)));
        assert_eq!(drive_cell("70000,1"), Some((70_000, 1)));
        for word in [
            "12", "12,", ",34", "12,34,5", "12 34 5", "ba", "x2", "-1,3", "a",
        ] {
            assert_eq!(drive_cell(word), None, "{word}");
        }
        // The word's reading: a spot's name and its letter forward that
        // spot's own cell — two spots, so a swapped or a wrong index shows —
        // and the reply names the same spot.
        let spots = [
            KeySpot {
                name: SmolStr::new_static("The Ford"),
                x: 12,
                y: 40,
            },
            KeySpot {
                name: SmolStr::new_static("Hill 7"),
                x: 50,
                y: 3,
            },
        ];
        let read = |word: &str| drive_goal(word, &spots, 64, 48);
        for (word, x, y, line) in [
            ("the ford", 12, 40, "Steering for The Ford (A)."),
            ("a", 12, 40, "Steering for The Ford (A)."),
            ("hill 7", 50, 3, "Steering for Hill 7 (B)."),
            ("b", 50, 3, "Steering for Hill 7 (B)."),
            ("12,34", 12, 34, "Steering for cell 12,34."),
            ("63 47", 63, 47, "Steering for cell 63,47."),
        ] {
            assert_eq!(
                read(word),
                Ok((MotorGoal::Cell { x, y }, line.to_owned())),
                "{word}"
            );
        }
        assert_eq!(
            read("hold"),
            Ok((MotorGoal::Hold, DRIVE_HOLDING.to_owned()))
        );
        assert_eq!(read("off"), Ok((MotorGoal::Off, DRIVE_RELEASED.to_owned())));
        assert_eq!(read(""), Err(CockpitError::Usage(DRIVE_USAGE.to_owned())));
        for word in ["64,1", "1,48", "70000,1"] {
            assert_eq!(
                read(word),
                Err(CockpitError::Refused(DRIVE_OFF_WINDOW.to_owned())),
                "{word}"
            );
        }
        for word in ["ba", "x2", "the", "c", "12", "12,"] {
            assert_eq!(
                read(word),
                Err(CockpitError::Refused(DRIVE_NOT_A_GOAL.to_owned())),
                "{word}"
            );
        }
        // No ground: every cell is off the window and no word is a spot.
        assert_eq!(
            drive_goal("1,1", &[], 0, 0),
            Err(CockpitError::Refused(DRIVE_OFF_WINDOW.to_owned()))
        );
        assert_eq!(
            drive_goal("a", &[], 0, 0),
            Err(CockpitError::Refused(DRIVE_NOT_A_GOAL.to_owned()))
        );
        // The motor's two notices, through the one renderer.
        let frame = ap3_scene();
        for (outcome, line) in [
            (DriveOutcome::Arrived, DRIVE_ARRIVED),
            (DriveOutcome::NoRoute, DRIVE_NO_ROUTE),
        ] {
            assert_eq!(
                notice_line(&frame, &FrameNotice::Drive { outcome }, &[]),
                line
            );
        }
        // The arena's own refusal of a goal names the verb the pilot typed.
        let goal = IntentKind::Goal(MotorGoal::Off);
        assert_eq!(goal.as_str(), "drive");
        assert_eq!(
            refusal_line(
                goal.as_str(),
                Refusal::PowerNotOnline,
                None,
                RefusalDetail::None
            ),
            "DRIVE refused. Power not online: STARTUP first."
        );
        assert_eq!(
            refusal_line(goal.as_str(), Refusal::QueueFull, None, RefusalDetail::None),
            "DRIVE refused. Too many orders at once; try that again."
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut AP.1
    // -----------------------------------------------------------------------

    /// **`B-86`, the formula and the floor skip**: a cycling cannon's countdown
    /// is the frame's own less the wall time since receipt, held at nought and
    /// ready there; a ready row stays ready at every instant; and a floored read
    /// is [`frame_weapons`]'s, untouched, at every instant.
    ///
    /// The frame is a real one, the opening frame of a stepped scout-and-tank
    /// arena, with its rack replaced by the two rows this is about.
    #[test]
    fn ap1_the_aged_rows_count_down_from_receipt_and_a_floor_leaves_them_alone() {
        use crate::arena::state::Team;
        use crate::arena::test_support::{arena_at_epoch, pose, scout, tank};
        use crate::arena::weapons::FrameWeapon;
        use std::time::{Duration, Instant};
        const RED: ObjId = ObjId(4);
        const BLUE: ObjId = ObjId(9);
        let mut arena = arena_at_epoch(
            vec![
                scout(RED, Team::Red, pose(1_500, 1_500)),
                tank(BLUE, Team::Blue, pose(6_500, 6_500)),
            ],
            1,
        );
        let mut frame = arena
            .step(0, &[], &[])
            .frames
            .into_iter()
            .map(|(_, frame)| frame)
            .find(|frame| frame.unit == RED)
            .expect("red is sent its opening frame");
        let row = |id: &'static str, ready: bool, ready_in_ms: i64| FrameWeapon {
            id: SmolStr::new_static(id),
            ready,
            ready_in_ms,
            rounds: 10,
            rounds_max: 10,
            range_band: None,
            p_permille: None,
            bands_cm: vec![100_000],
            heat_per_shot: 0,
            armed: None,
        };
        frame.weapons = vec![row("cannon", false, 2_000), row("machine_gun", true, 0)];
        let now = Instant::now();
        let held_at = |back_ms: u64| CockpitFrame {
            match_id: crate::arena_task::MatchId(1),
            frame: frame.clone(),
            ranges: BTreeMap::new(),
            arena_ms: 0,
            received: now
                .checked_sub(Duration::from_millis(back_ms))
                .expect("the clock reaches that far back"),
        };
        let cannon = |rows: &[WeaponRow]| {
            rows.iter()
                .find(|row| row.id == "cannon")
                .map(|row| (row.ready, row.ready_in_ms))
                .expect("the cannon row")
        };
        let gun = |rows: &[WeaponRow]| {
            rows.iter()
                .find(|row| row.id == "machine_gun")
                .map(|row| (row.ready, row.ready_in_ms))
                .expect("the machine gun row")
        };

        // Half a second to go, and the line says so, rounded up.
        let rows = aged_weapons(&held_at(1_500), now, false);
        assert_eq!(cannon(&rows), (false, 500));
        let line = rows
            .iter()
            .find(|row| row.id == "cannon")
            .map(|row| row.render(&rows))
            .expect("the cannon row");
        assert!(line.starts_with("cannon  cycling (1 s)  "), "{line}");
        // At the ready instant and long past it: ready, at nought.
        for back_ms in [2_000, 9_000] {
            let rows = aged_weapons(&held_at(back_ms), now, false);
            assert_eq!(cannon(&rows), (true, 0), "{back_ms} ms after receipt");
        }
        // The ready row stays ready, and nothing but the countdown moves.
        for back_ms in [0, 1_500, 2_000, 9_000] {
            let held = held_at(back_ms);
            let rows = aged_weapons(&held, now, false);
            assert_eq!(gun(&rows), (true, 0), "{back_ms} ms after receipt");
            for (aged, own) in rows.iter().zip(frame_weapons(&held.frame)) {
                assert_eq!(
                    WeaponRow {
                        ready: own.ready,
                        ready_in_ms: own.ready_in_ms,
                        ..aged.clone()
                    },
                    own,
                    "only the countdown is aged"
                );
            }
            // Floored: the frame's own rows, untouched, at every instant.
            assert_eq!(
                aged_weapons(&held, now, true),
                frame_weapons(&held.frame),
                "{back_ms} ms after receipt, under a floor"
            );
        }
        // At receipt itself, the frame's own figure.
        assert_eq!(
            cannon(&aged_weapons(&held_at(0), now, false)),
            (false, 2_000)
        );
    }

    // -----------------------------------------------------------------------
    // 16b cut AP.3
    // -----------------------------------------------------------------------

    /// **The help's own scene** (16b cut AP.3): `RA` online at 24 kph heading
    /// east, a cannon ready and a machine gun three seconds from it, locked on
    /// `BA` (a fresh plot at 450 m, `far` on the shipped 600 m sight) beside an
    /// unidentified `X3` whose plot is four and a half seconds old. A real
    /// opening frame of a stepped arena, its fields replaced.
    fn ap3_scene() -> Frame {
        use crate::arena::perception::FrameContact;
        use crate::arena::state::Team;
        use crate::arena::test_support::{arena_at_epoch, pose, scout, tank};
        use crate::arena::weapons::FrameWeapon;
        const RED: ObjId = ObjId(4);
        const BLUE: ObjId = ObjId(9);
        let mut arena = arena_at_epoch(
            vec![
                scout(RED, Team::Red, pose(1_500, 1_500)),
                tank(BLUE, Team::Blue, pose(6_500, 6_500)),
            ],
            1,
        );
        let mut frame = arena
            .step(0, &[], &[])
            .frames
            .into_iter()
            .map(|(_, frame)| frame)
            .find(|frame| frame.unit == RED)
            .expect("red is sent its opening frame");
        frame.callsign = Designator::Slot {
            team: Team::Red,
            slot: 1,
        };
        frame.team = Team::Red;
        frame.power = Power::Online;
        frame.speed_cms = 667;
        frame.pose.heading_mdeg = 90_000;
        frame.held = false;
        frame.wreck = false;
        frame.sight_cm = 60_000;
        frame.notices.clear();
        frame.contacts = vec![
            FrameContact {
                id: ContactId(1),
                designator: Designator::Slot {
                    team: Team::Blue,
                    slot: 1,
                },
                alias: Some(1),
                class: Classification::Classified { team: Team::Blue },
                bearing_mdeg: 87_000,
                range_cm: 45_000,
                age_ms: 800,
                cell: Some((6, 2)),
                condition: None,
                aspect: crate::arena::perception::Aspect::Front,
                reads_dead: None,
                fatal: None,
            },
            FrameContact {
                id: ContactId(2),
                designator: Designator::Provisional(3),
                alias: None,
                class: Classification::Unknown,
                bearing_mdeg: 315_000,
                range_cm: 22_000,
                age_ms: 4_500,
                cell: Some((0, 3)),
                condition: None,
                aspect: crate::arena::perception::Aspect::Front,
                reads_dead: None,
                fatal: None,
            },
        ];
        frame.lock = Some(ContactId(1));
        let gun = |id: &'static str, ready: bool, ready_in_ms: i64, p: u32| FrameWeapon {
            id: SmolStr::new_static(id),
            ready,
            ready_in_ms,
            rounds: 10,
            rounds_max: 20,
            range_band: None,
            p_permille: Some(p),
            bands_cm: vec![100_000, 300_000],
            heat_per_shot: 0,
            armed: None,
        };
        frame.weapons = vec![
            gun("cannon", true, 0, 460),
            gun("machine_gun", false, 2_500, 210),
        ];
        frame
    }

    /// The scene's three lines, over its own rows and no range memory.
    fn ap3_lines(frame: &Frame) -> Vec<String> {
        watch_lines(frame, &BTreeMap::new(), &frame_weapons(frame), 1_000)
    }

    /// **16b cut AP.3** (16c N19): the digest is the help's three lines for the
    /// help's numbers, one line for a bare platform, a plot's age only once it
    /// is stale, a `; +N more` tail that keeps every line within 120 columns on
    /// both fitted lines, the flags in `STATUS`'s order, the friendly mark —
    /// and no prefix any reader of a `STATUS`, `CONTACTS` or lock line could
    /// take, no tempo word and nothing that moves with the platform's cell.
    #[test]
    fn ap3_watch_lines_are_the_three_lines_within_120_columns() {
        use crate::arena::clock::Tempo;
        use crate::arena::perception::FrameContact;
        use crate::arena::state::Team;
        use crate::arena::weapons::FrameWeapon;

        // (1) `docs/help/cockpit.md`'s "Watching from line mode", verbatim.
        let scene = ap3_scene();
        assert_eq!(
            ap3_lines(&scene),
            vec![
                "Watch RA: online, 24 kph, heading 090°".to_owned(),
                "Watch guns: cannon ready, machine_gun (mg) cycling (3 s); lock BA, 450 m, 46% \
                 to hit"
                    .to_owned(),
                "Watch contacts: BA blue 087° far 450 m; X3 unknown 315° effective 200 m, \
                 plotted 4 s ago"
                    .to_owned(),
            ]
        );
        let help = include_str!("../../../../docs/help/cockpit.md");
        for line in ap3_lines(&scene) {
            assert!(
                help.contains(&format!("  {line}\n")),
                "the help's example omits {line:?}"
            );
        }

        // (2) No gun and no contact: the own line alone.
        let mut bare = scene.clone();
        bare.weapons.clear();
        bare.contacts.clear();
        bare.lock = None;
        assert_eq!(
            ap3_lines(&bare),
            vec!["Watch RA: online, 24 kph, heading 090°".to_owned()]
        );
        // A gun and no lock, and a lock whose chance is not resolved yet.
        let mut unlocked = scene.clone();
        unlocked.lock = None;
        assert!(
            ap3_lines(&unlocked)
                .get(1)
                .is_some_and(|line| line.ends_with("cycling (3 s); no lock")),
            "{:?}",
            ap3_lines(&unlocked)
        );
        let mut unresolved = scene.clone();
        for weapon in &mut unresolved.weapons {
            weapon.p_permille = None;
        }
        assert!(
            ap3_lines(&unresolved)
                .get(1)
                .is_some_and(|line| line.ends_with("; lock BA, 450 m")),
            "{:?}",
            ap3_lines(&unresolved)
        );
        // An empty gun reads `empty`, ready or cycling — `FIRE` refuses it
        // either way — so the last round leaving changes the block.
        let mut dry = scene.clone();
        for weapon in &mut dry.weapons {
            weapon.rounds = 0;
        }
        assert_eq!(
            ap3_lines(&dry).get(1).map(String::as_str),
            Some("Watch guns: cannon empty, machine_gun (mg) empty; lock BA, 450 m, 46% to hit"),
            "an empty gun read as ready or cycling"
        );
        assert_ne!(ap3_lines(&dry), ap3_lines(&scene));

        // (3) A plot's age prints only once it is older than the pass.
        let mut aged = scene.clone();
        if let Some(contact) = aged.contacts.first_mut() {
            contact.age_ms = 1_000;
        }
        let at_pass = ap3_lines(&aged);
        assert!(
            at_pass
                .get(2)
                .is_some_and(|line| line.starts_with("Watch contacts: BA blue 087° far 450 m; ")),
            "{at_pass:?}"
        );
        if let Some(contact) = aged.contacts.first_mut() {
            contact.age_ms = 1_001;
        }
        let past_pass = ap3_lines(&aged);
        assert!(
            past_pass.get(2).is_some_and(|line| line
                .starts_with("Watch contacts: BA blue 087° far 450 m, plotted 1 s ago; ")),
            "{past_pass:?}"
        );

        // (4) Twenty-three contacts of the widest shipped words: one line,
        // within 120, a prefix of the list in frame order and the count of
        // the rest.
        let mut crowded = scene.clone();
        crowded.sight_cm = 0;
        crowded.lock = None;
        crowded.contacts = (0_u16..23)
            .map(|n| FrameContact {
                id: ContactId(100 + u64::from(n)),
                designator: Designator::Provisional(10 + n),
                alias: None,
                class: Classification::Unknown,
                bearing_mdeg: 315_000,
                range_cm: 150_000,
                age_ms: 12_000,
                cell: Some((1, 1)),
                condition: None,
                aspect: crate::arena::perception::Aspect::Front,
                reads_dead: None,
                fatal: None,
            })
            .collect();
        let lines = ap3_lines(&crowded);
        let contacts = lines.get(2).expect("a contacts line");
        assert!(contacts.chars().count() <= WATCH_COLUMNS, "{contacts}");
        let (listed, more) = contacts
            .rsplit_once("; +")
            .expect("twenty-three contacts do not fit one line");
        let more: usize = more
            .strip_suffix(" more")
            .and_then(|n| n.parse().ok())
            .expect("the tail counts what it left out");
        let shown: Vec<&str> = listed
            .strip_prefix("Watch contacts: ")
            .expect("the head")
            .split("; ")
            .collect();
        assert!(!shown.is_empty(), "{contacts}");
        assert_eq!(shown.len() + more, 23, "{contacts}");
        for (n, entry) in shown.iter().enumerate() {
            assert_eq!(
                *entry,
                format!(
                    "X{} unknown 315° effective 1500 m, plotted 12 s ago",
                    10 + n
                ),
                "the entries are the first of the list, in its order: {contacts}"
            );
        }

        // (5) Twelve guns likewise, and the lock is still said after the tail.
        let mut armed = scene.clone();
        armed.weapons = (0..12)
            .map(|n| FrameWeapon {
                id: SmolStr::new(format!("heavy_laser{n}")),
                ready: false,
                ready_in_ms: 11_500,
                rounds: 10,
                rounds_max: 10,
                range_band: None,
                p_permille: Some(460),
                bands_cm: vec![100_000, 300_000],
                heat_per_shot: 0,
                armed: None,
            })
            .collect();
        let lines = ap3_lines(&armed);
        let guns = lines.get(1).expect("a guns line");
        assert!(guns.chars().count() <= WATCH_COLUMNS, "{guns}");
        assert!(guns.ends_with("; lock BA, 450 m, 46% to hit"), "{guns}");
        let (listed, rest) = guns.split_once("; +").expect("twelve guns do not fit");
        let more: usize = rest
            .split_once(" more; lock ")
            .and_then(|(n, _)| n.parse().ok())
            .expect("the tail before the lock");
        let shown: Vec<&str> = listed
            .strip_prefix("Watch guns: ")
            .expect("the head")
            .split(", ")
            .collect();
        assert_eq!(shown.len() + more, 12, "{guns}");
        for (n, entry) in shown.iter().enumerate() {
            // Each shares `hl` with the others, so none advertises it.
            assert_eq!(*entry, format!("heavy_laser{n} cycling (12 s)"), "{guns}");
        }

        // (6) The flags, in `STATUS`'s order; the friendly mark on a lock.
        let mut wreck = scene.clone();
        wreck.held = true;
        wreck.wreck = true;
        assert_eq!(
            ap3_lines(&wreck).first().map(String::as_str),
            Some("Watch RA: online, 24 kph, heading 090°, HELD, DESTROYED")
        );
        let mut friendly = scene.clone();
        friendly.contacts.push(FrameContact {
            id: ContactId(3),
            designator: Designator::Slot {
                team: Team::Red,
                slot: 2,
            },
            alias: Some(2),
            class: Classification::Classified { team: Team::Red },
            bearing_mdeg: 180_000,
            range_cm: 12_000,
            age_ms: 0,
            cell: Some((1, 0)),
            condition: None,
            aspect: crate::arena::perception::Aspect::Front,
            reads_dead: None,
            fatal: None,
        });
        friendly.lock = Some(ContactId(3));
        assert!(
            ap3_lines(&friendly)
                .get(1)
                .is_some_and(|line| line.ends_with("; lock RB (FRIENDLY), 100 m, 46% to hit")),
            "{:?}",
            ap3_lines(&friendly)
        );

        // (7) No prefix a reader of `STATUS`, `CONTACTS` or the lock takes; no
        // tempo word; nothing that moves with the cell, the position or the
        // arena's tempo.
        for frame in [&scene, &bare, &wreck, &friendly, &crowded, &armed] {
            let lines = ap3_lines(frame);
            for line in &lines {
                assert!(line.chars().count() <= WATCH_COLUMNS, "{line}");
                assert!(line.starts_with("Watch "), "{line}");
                assert!(!line.contains(": power "), "{line}");
                assert!(!line.starts_with("Locked on "), "{line}");
                assert!(!line.contains("  bearing "), "{line}");
                for word in ["quiet", "alert", "engaged", "Quiet", "Alert", "Engaged"] {
                    assert!(!line.contains(word), "a tempo word: {line}");
                }
            }
            for tempo in [Tempo::Quiet, Tempo::Alert, Tempo::Engaged] {
                let mut moved = (*frame).clone();
                moved.tempo = tempo;
                moved.cell = Some((7, 7));
                moved.pose.x_cm += 4_000;
                moved.pose.y_cm -= 3_000;
                assert_eq!(ap3_lines(&moved), lines, "the block moved with {tempo:?}");
            }
        }
    }

    /// **17b cut M1a.5: the seat grants the channel table** (doc 17 §4,
    /// D-UM-47): `cockpit_channels()` is exactly the set of
    /// `COCKPIT_CHANNEL_IDS`, which is the arena's three channels' words.
    #[test]
    fn m1a5_the_seat_grants_the_channel_table() {
        let granted: BTreeSet<String> = cockpit_channels()
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        let table: BTreeSet<String> = crate::arena::state::COCKPIT_CHANNEL_IDS
            .iter()
            .map(|id| (*id).to_owned())
            .collect();
        assert_eq!(granted, table);
        assert_eq!(granted.len(), Channel::ALL.len());
    }

    // -----------------------------------------------------------------------
    // 17b cut M1b.1
    // -----------------------------------------------------------------------

    /// **17b cut M1b.1**: the three new refusal words' sentences — the floor's,
    /// naming the floor in whole per cents, its fallback for an `Unlawful` with
    /// no floor (a `Contact` target), the count's and the warrant's — each
    /// through the one prefix every `FIRE` refusal has; and the two
    /// `CockpitError`s as their one home.
    #[test]
    fn m1b1_the_new_refusals_say_their_sentences() {
        assert_eq!(
            refusal_line(
                "fire",
                Refusal::Unlawful,
                Some("cannon"),
                RefusalDetail::Floor { permille: 100 }
            ),
            "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less."
        );
        assert_eq!(
            refusal_line(
                "fire",
                Refusal::Unlawful,
                Some("cannon"),
                RefusalDetail::Floor { permille: 50 }
            ),
            "FIRE cannon refused. The Compact floor is 5%; a condition may not ask for less."
        );
        assert_eq!(
            refusal_line(
                "fire",
                Refusal::Unlawful,
                Some("cannon"),
                RefusalDetail::None
            ),
            "FIRE cannon refused. The Compact floor forbids that condition."
        );
        assert_eq!(
            refusal_line(
                "fire",
                Refusal::NoSlot,
                Some("missiles"),
                RefusalDetail::None
            ),
            "FIRE missiles refused. Every slot is armed; FIRE <weapon> OFF frees one."
        );
        assert_eq!(
            refusal_line(
                "fire",
                Refusal::Unwarranted,
                Some("cannon"),
                RefusalDetail::None
            ),
            "FIRE cannon refused. That has no warrant."
        );
        assert_eq!(
            CockpitError::Unlawful { floor_pct: 10 }.to_string(),
            "The Compact floor is 10%; a condition may not ask for less."
        );
        assert_eq!(
            CockpitError::NoSlot.to_string(),
            "Every slot is armed; FIRE <weapon> OFF frees one."
        );
        // The pre-check's half composes the same string (M1b.4's path).
        assert_eq!(
            fire_refused("cannon", &CockpitError::Unlawful { floor_pct: 10 }).to_string(),
            "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less."
        );
    }

    /// **17b cut M1b.1**: the two slot notices' sentences (defined here,
    /// staged by 17b cut M1b.4) over both halves of `SlotFired`'s contact and
    /// every `HeldWhy`, and their keyframe objects' keys — every other notice's
    /// object unmoved, and a refused `unlawful` carrying `floor_permille`.
    #[test]
    fn m1b1_the_slot_notices_say_their_sentences_and_carry_their_keys() {
        use crate::arena::conditions::HeldWhy;
        use crate::arena::perception::{Aspect, Named};
        let mut frame = ap3_scene();
        let ba = frame_contacts(&frame)
            .into_iter()
            .find(|row| row.designator.to_string() == "BA")
            .map(|row| Named {
                id: row.id,
                designator: row.designator,
            })
            .expect("the scene's BA");
        let held = |why: HeldWhy| FrameNotice::SlotHeld {
            slot: 1,
            weapon: SmolStr::new("cannon"),
            why,
        };
        let cases = [
            (
                FrameNotice::SlotFired {
                    slot: 1,
                    weapon: SmolStr::new("cannon"),
                    contact: Some(ba),
                    p_permille: 620,
                    aspect: Aspect::Left,
                },
                "Cannon fires at BA by S1 (62%, side).",
            ),
            (
                FrameNotice::SlotFired {
                    slot: 2,
                    weapon: SmolStr::new("machine_gun"),
                    contact: None,
                    p_permille: 950,
                    aspect: Aspect::Rear,
                },
                "Machine_gun fires by S2 (95%, rear).",
            ),
            (
                held(HeldWhy::Odds {
                    permille: 540,
                    floor: 600,
                }),
                "S1 held the cannon: 54% under your 60%.",
            ),
            (
                held(HeldWhy::Aspect {
                    contact: ba,
                    aspect: Aspect::Front,
                }),
                "S1 held the cannon: BA shows its front.",
            ),
            (
                held(HeldWhy::Range {
                    contact: ba,
                    range_m: 1_200,
                    min_m: 200,
                    max_m: 600,
                }),
                "S1 held the cannon: BA at 1200 m, outside 200 to 600.",
            ),
            (
                held(HeldWhy::Range {
                    contact: ba,
                    range_m: 1_200,
                    min_m: 0,
                    max_m: 600,
                }),
                "S1 held the cannon: BA at 1200 m, outside under 600.",
            ),
            (
                held(HeldWhy::Range {
                    contact: ba,
                    range_m: 100,
                    min_m: 200,
                    max_m: u32::MAX,
                }),
                "S1 held the cannon: BA at 100 m, outside past 200.",
            ),
            (
                held(HeldWhy::Rounds {
                    rounds: 8,
                    keeps: 8,
                }),
                "S1 held the cannon: 8 rounds, keeps 8.",
            ),
        ];
        let rows = frame_contacts(&frame);
        for (notice, said) in &cases {
            assert_eq!(notice_line(&frame, notice, &rows), *said);
            assert_eq!(notice.section(), None);
            assert!(!notice.spent());
            assert_eq!(notice.detail(), RefusalDetail::None);
        }
        // The keyframe's objects.
        frame.notices = cases.iter().map(|(notice, _)| notice.clone()).collect();
        frame.notices.push(FrameNotice::Refused {
            order: SmolStr::new("fire"),
            refusal: Refusal::Unlawful,
            weapon: Some(SmolStr::new("cannon")),
            detail: RefusalDetail::Floor { permille: 100 },
        });
        let keyframe = status_frame(crate::arena_task::MatchId(1), &frame);
        let notices = keyframe["notices"].as_array().cloned().unwrap_or_default();
        assert_eq!(notices.len(), cases.len() + 1);
        let fired = &notices[0];
        assert_eq!(fired["kind"], "slot_fired");
        assert_eq!(fired["slot"], 1);
        assert_eq!(fired["weapon"], "cannon");
        assert_eq!(fired["designator"], "BA");
        assert_eq!(fired["percent"], 62);
        assert_eq!(fired["aspect"], "left");
        assert_eq!(fired["detail"], serde_json::Value::Null);
        assert_eq!(notices[1]["designator"], serde_json::Value::Null);
        let odds = &notices[2];
        assert_eq!(odds["kind"], "slot_held");
        assert_eq!(odds["slot"], 1);
        assert_eq!(odds["weapon"], "cannon");
        assert_eq!(
            odds["detail"],
            serde_json::json!({
                "why": "odds", "percent": 54, "floor": 60, "aspect": null, "range_m": null,
                "min_m": null, "max_m": null, "rounds": null, "keeps": null
            })
        );
        assert_eq!(notices[3]["detail"]["why"], "aspect");
        assert_eq!(notices[3]["detail"]["aspect"], "front");
        assert_eq!(notices[3]["designator"], "BA");
        assert_eq!(
            notices[4]["detail"],
            serde_json::json!({
                "why": "range", "percent": null, "floor": null, "aspect": null,
                "range_m": 1200, "min_m": 200, "max_m": 600, "rounds": null, "keeps": null
            })
        );
        assert_eq!(notices[7]["detail"]["why"], "rounds");
        assert_eq!(notices[7]["detail"]["keeps"], 8);
        let refused = &notices[cases.len()];
        assert_eq!(refused["refusal"], "unlawful");
        assert_eq!(
            refused["detail"],
            serde_json::json!({ "floor_permille": 100 })
        );
        assert!(
            refused.get("slot").is_none(),
            "a notice that is not a slot's carries no slot key"
        );
    }
}
