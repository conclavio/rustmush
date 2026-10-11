//! Whole lines out of the terminal stream, and what each one says (cut AJ.1).
//!
//! [`Assembler`] turns decoded terminal text into whole lines; [`classify`]
//! reads one line into a [`Line`]. The classifier is **pure and total**: every
//! input is exactly one variant, [`Line::Other`] is the fallback, and nothing
//! panics on any input. It reads only what the game prints to a line-mode pilot
//! — every sentence below is quoted in `help cockpit` / `help simulator` or is a
//! line the cockpit, the centre or the renderer prints, and the contract test in
//! `tests/arena_contract.rs` holds each one to the game's own formatter.
//!
//! Hand-written `strip_prefix`/`split_once` parsing, no regular expressions:
//! the sentences are fixed and a regex would only hide which part failed.
//! Leading spaces carry meaning in exactly three places — `STATUS`'s indented
//! rows (two spaces) and facet runs (four), the centre board's side rows (two)
//! and the reclaim catch-up block's rows (two) — so those are recognised by their
//! indent first, and **any other indented line is `Other`**: it is a wrapped
//! continuation, and another player's padded speech must never read as a game
//! sentence. A line at column 0 is read trimmed, in this order: a hint, a
//! transient, then the game's sentences.
use serde::{Deserialize, Serialize};

use super::Side;
use crate::transport::PTY_COLUMNS;

/// **A line whose pieces arrive apart is whole up to two seconds; in play
/// only a prompt has no newline, and the lane acts on none** (cut AM.1). How
/// long an unterminated tail may sit before [`Assembler::flush_stale`] hands
/// it out as a line. It was 300 ms, which on a distant box cut a row split
/// mid-line by a pause into two broken lines. The picture's lock line lapses
/// on the same figure (one output's pieces arrive at most this far apart).
pub const STALE_TAIL_MS: u64 = 2_000;
/// The longest tail kept waiting for its newline; a longer one is a line.
pub const MAX_TAIL_BYTES: usize = 16 * 1024;

/// Decoded terminal text in, whole lines out.
#[derive(Debug, Default)]
pub struct Assembler {
    tail: String,
    tail_ms: u64,
}

impl Assembler {
    /// Split `text` (already through `terminal::Decoder`) on `\n`, strip each
    /// complete line's trailing `\r` and trailing spaces, and keep the
    /// unterminated tail with the time it was last extended.
    pub fn feed(&mut self, now_ms: u64, text: &str) -> Vec<String> {
        self.feed_cols(now_ms, text)
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    /// [`Assembler::feed`], each line with its **untrimmed column count** (cut
    /// AM.1): its `char`s once the `\r` is removed and before the trailing
    /// spaces are trimmed. The renderer writes a wrapped line with whatever
    /// run of spaces stood at the break (it splits on single spaces and keeps
    /// runs), so the untrimmed count is the width it measured; the channel
    /// continuation ([`continues`]) reads it. The text is still trimmed.
    pub fn feed_cols(&mut self, now_ms: u64, text: &str) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        if text.is_empty() {
            return out;
        }
        self.tail.push_str(text);
        while let Some(end) = self.tail.find('\n') {
            let line: String = self.tail.drain(..=end).collect();
            out.push(clean_end_cols(&line[..line.len() - 1]));
        }
        if self.tail.len() > MAX_TAIL_BYTES {
            out.push(clean_end_cols(&std::mem::take(&mut self.tail)));
        }
        if !self.tail.is_empty() {
            self.tail_ms = now_ms;
        }
        out
    }

    /// The tail as a line once it has sat [`STALE_TAIL_MS`] unextended and is
    /// not blank.
    pub fn flush_stale(&mut self, now_ms: u64) -> Option<String> {
        self.flush_stale_cols(now_ms).map(|(line, _)| line)
    }

    /// [`Assembler::flush_stale`] with the line's untrimmed column count (see
    /// [`Assembler::feed_cols`]).
    pub fn flush_stale_cols(&mut self, now_ms: u64) -> Option<(String, usize)> {
        if self.tail.is_empty() || now_ms < self.tail_ms.saturating_add(STALE_TAIL_MS) {
            return None;
        }
        let (line, cols) = clean_end_cols(&std::mem::take(&mut self.tail));
        (!line.trim().is_empty()).then_some((line, cols))
    }
}

/// The line with its `\r` and trailing spaces trimmed, and its column count
/// with only the `\r` removed.
fn clean_end_cols(line: &str) -> (String, usize) {
    let cols = line.trim_end_matches('\r').chars().count();
    (line.trim_end_matches(['\r', ' ']).to_owned(), cols)
}

/// **Whether `line` is the rest of a full-width channel line** (cut AM.1): the
/// line before it (`prev_cols` columns, untrimmed) could not have held its
/// first word. That is the renderer's own break rule (`mush-ansi`'s
/// `wrap_indent`, which splits on single spaces): a word goes to a new line
/// when `width + 1 + word > {PTY_COLUMNS}`, the width the bot's PTY asks for.
/// A line with a leading space continues only a line already at the width:
/// the renderer starts a line with a space only where a run of spaces
/// straddles a break, and an empty word breaks only a full line. Widths are
/// `char`s (`°` is one column).
///
/// Pure, and only half the rule: the caller decides whether the line before
/// began a channel chain (a column-0 `[{channel}] …` line, or itself a
/// continuation) and was read just before. Speech, poses, pages and whispers
/// need no rule: their continuations carry a hanging indent and already read
/// as [`Line::Other`]. A channel line wraps **flush**, so a teammate can pad
/// one until a game sentence starts the next line at column 0. The rule
/// cannot tell a wrapped channel line from an unwrapped one 110 to 120
/// columns long, which is why the lane never reads a line it expects (an
/// answer to a line it typed) as a continuation.
pub fn continues(prev_cols: usize, line: &str) -> bool {
    let width = usize::try_from(PTY_COLUMNS).unwrap_or(usize::MAX);
    if line.starts_with(' ') {
        return prev_cols >= width;
    }
    let word = line.split(' ').next().unwrap_or("").chars().count();
    prev_cols.saturating_add(1).saturating_add(word) > width
}

// ---------------------------------------------------------------------------
// What a line says
// ---------------------------------------------------------------------------

/// A tank's power, as `STATUS` words it (`help cockpit`: `STARTUP`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Power {
    #[default]
    Off,
    Starting,
    Online,
}

impl Power {
    fn parse(word: &str) -> Option<Power> {
        match word {
            "off" => Some(Power::Off),
            "starting" => Some(Power::Starting),
            "online" => Some(Power::Online),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Power::Off => "off",
            Power::Starting => "starting",
            Power::Online => "online",
        }
    }
}

/// What the own scope holds, `STATUS`'s position row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Not read yet.
    #[default]
    Unknown,
    Clear,
    Hostile,
    Unidentified,
}

/// Why the tank is held (`help cockpit` "Driving").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeldCause {
    /// `Held: the ground ahead will not take the tank.`
    Impassable,
    /// `Held: something is in the way.`
    Occupied,
    /// `Held: this is the edge of the arena.`
    Edge,
    /// `Held: the slope ahead is too steep for the tank.`
    TooSteep,
    /// `HELD` on `STATUS`'s head with no notice read to say why.
    Unstated,
}

/// **The rule a match is fought under** (cut AN.2), as the board says it
/// (`help simulator`, "Modes", "The range"): every match is a skirmish "until
/// somebody says otherwise", and the board prints the rule two-space indented
/// while a match is set up, boarding and running — `  mode: hold — keep The
/// Crossroads (B) uncontested for 120 s`, `  mode: range — 2 targets on the
/// blue side` — and prints nothing for a skirmish.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// Eliminate the other side: the board prints no mode line.
    #[default]
    Skirmish,
    /// `mode: hold — …`: keep The Crossroads uncontested.
    Hold,
    /// `mode: range — …`: targets that never move.
    Range,
}

/// How a match ended, the word in `Match {n} is over ({word}).`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultWord {
    Red,
    Blue,
    Draw,
    Abort,
    Timeout,
    Void,
    /// A word this build does not know, as printed.
    Other(String),
}

impl ResultWord {
    pub fn parse(word: &str) -> ResultWord {
        match word {
            "red" => ResultWord::Red,
            "blue" => ResultWord::Blue,
            "draw" => ResultWord::Draw,
            "abort" => ResultWord::Abort,
            "timeout" => ResultWord::Timeout,
            "void" => ResultWord::Void,
            other => ResultWord::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            ResultWord::Red => "red",
            ResultWord::Blue => "blue",
            ResultWord::Draw => "draw",
            ResultWord::Abort => "abort",
            ResultWord::Timeout => "timeout",
            ResultWord::Void => "void",
            ResultWord::Other(word) => word,
        }
    }
}

/// One `CONTACTS` row (`help cockpit` "Seeing"):
/// `BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon 62%`.
///
/// **17b cut M1a.3** (a declared reader change, doc 16 §11): the row grew an
/// optional tail after the age — the aspect's side word, then on an armed hull
/// the main gun's odds or a dash. A row with no tail (an older server) still
/// parses, with both new fields `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactRow {
    pub designator: String,
    /// `None` while the class reads `unknown`.
    pub side: Option<Side>,
    pub bearing_deg: u32,
    pub rel_deg: u32,
    /// `close`, `effective`, `far` or `beyond`: scope advice, not the gun's.
    pub band: String,
    /// Rounded to 50 m and held against the last figure printed.
    pub range_m: u32,
    /// The plot's age, whole seconds (rounded down).
    pub age_s: u32,
    /// **17b cut M1a.3**: `front`, `side` or `rear` — which face the contact
    /// was showing when it was last seen; `None` on a row with no tail.
    #[serde(default)]
    pub aspect: Option<String>,
    /// **17b cut M1a.3**: the main gun's odds as `(weapon, percent)`. Outer
    /// `None`: a row with no tail (an older server). Inner `None`: the dash (a
    /// contact the fire control is not resolving) or an unarmed hull's row,
    /// which ends at the aspect.
    #[serde(default)]
    pub odds: Option<Option<(String, u32)>>,
}

/// One `WEAPONSTATUS` row:
/// `machine_gun (mg)  cycling (4 s)  380 of 500 round(s)  band 1 of 2  21% to hit`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponRow {
    pub id: String,
    pub short: Option<String>,
    pub ready: bool,
    /// Seconds left, rounded up, while cycling.
    pub cycling_s: Option<u32>,
    pub rounds: u32,
    pub rounds_max: u32,
    /// `band I of N`: the band the lock falls in for this weapon.
    pub band: Option<(u32, u32)>,
    /// This weapon's own chance against the lock.
    pub percent: Option<u32>,
}

/// `STATUS`'s second line: `Red Tank 1 [biped_mk1]: power online, 27 kph,
/// heading 090°, HELD, DESTROYED`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusHead {
    pub name: String,
    /// The bracketed chassis, printed only when it is not the pod's own.
    pub chassis: Option<String>,
    pub power: Power,
    /// Negative astern.
    pub kph: i32,
    pub heading_deg: u32,
    pub held: bool,
    pub destroyed: bool,
}

/// How a board row says a slot is standing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    Seated,
    Left,
    /// `flown by doctrine`: dropped, the machine holding the tank.
    Doctrine,
}

/// One pilot entry of a board side row: `1 (RA): Ada [biped_mk1], pod Red Pod
/// 1, seated`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardPilot {
    pub index: u8,
    pub callsign: String,
    pub name: String,
    pub unit_type: Option<String>,
    /// **The game's 17b cut M5.1**: `, a mind` after the bracket — another
    /// pilot's slot chose a mind with `SIM MIND` during selection (the board
    /// never names it to anyone but that pilot). This bot never chooses one,
    /// so its own row never carries the named form.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mind: bool,
    pub pod: Option<String>,
    pub standing: Option<Standing>,
}

/// Every kind of line a seated line-mode pilot is sent that the picture reads,
/// and [`Line::Other`] for the rest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Line {
    // -- cockpit read replies (journal false) --------------------------------
    /// A `CONTACTS` row.
    ContactRow(ContactRow),
    /// `No contacts.`
    NoContacts,
    /// `Locked on BA (FRIENDLY), 62% to hit.` — the first line of `WEAPONSTATUS`
    /// with a lock, and the arena's own `Locked` notice: one sentence.
    Lock {
        designator: String,
        friendly: bool,
        percent: Option<u32>,
    },
    /// A `WEAPONSTATUS` row.
    WeaponRow(WeaponRow),
    /// **The arming line under an armed weapon's row** (the game's 17b cut
    /// M1b.4): `  S1: >=60% side/rear` — the slot number and the terms
    /// verbatim after `S<n>: `. Read on its shape alone (this reader keeps no
    /// context between lines); the bot never acts on it, and it does not end
    /// an open `WEAPONSTATUS` reply.
    WeaponArming { slot: u8, terms: String },
    /// `This unit carries no weapon.`
    NoWeapon,
    /// ``That unit carries no weapon called `cannon`.`` — the answer to a `FIRE`
    /// naming a gun the tank does not carry (cut AL.1): in the bot's own
    /// transcripts, every `FIRE` of a gun lost with its turret.
    NoSuchWeapon { weapon: String },
    /// `Callsign RA.` — opens a `STATUS` block.
    StatusCallsign { callsign: String },
    /// `STATUS`'s second line.
    StatusHead(StatusHead),
    /// `  at 1234 m east, -56 m north; scope holds a hostile`
    StatusPosition { x_m: i64, y_m: i64, scope: Scope },
    /// `  nearest spot: The Ford (A), 120 m at 045°`
    StatusNearestSpot {
        name: String,
        letter: char,
        range_m: u32,
        bearing_deg: u32,
    },
    /// `  heat 40 of 100, OVERHEATED`
    StatusHeat {
        heat: u32,
        max: u32,
        overheated: bool,
    },
    /// `  receiver degraded: 15 of 40 through cover, 60% reach`
    StatusReceiver {
        budget: u32,
        budget_max: u32,
        reach_percent: u32,
    },
    /// `  emcon on: the pod radiates` / `  emcon off: the pod is silent`
    StatusEmcon { on: bool },
    /// `  hull  intact  internals 60/60`
    StatusSection {
        id: String,
        condition: String,
        internals: Option<(i64, i64)>,
    },
    /// `    front 400/400  left 250/250` — the facets of the section above.
    StatusFacets { facets: Vec<(String, i64, i64)> },
    /// `Red Tank 1: power off. The panel has told you nothing yet.`
    StatusUntold { name: String },
    /// `The panel has told you nothing yet.` (a spot asked before any frame)
    NoFrameYet,
    /// `Range to BA: 440 m.` / `…, plotted 4 s ago.`
    Range {
        designator: String,
        range_m: u32,
        plotted_s: Option<u32>,
    },
    /// `Bearing to BA: 087° (rel 357°).` / `…, plotted 4 s ago.`
    Bearing {
        designator: String,
        bearing_deg: u32,
        rel_deg: u32,
        plotted_s: Option<u32>,
    },
    /// `Range to The Ford (A): 430 m.`
    SpotRange {
        name: String,
        letter: char,
        range_m: u32,
    },
    /// `Bearing to The Ford (A): 087° (rel 357°).`
    SpotBearing {
        name: String,
        letter: char,
        bearing_deg: u32,
        rel_deg: u32,
    },
    /// `No lock: LOCK a contact first.`
    NoLock,
    /// `No lock: LOCK a contact first, or name one.`
    NoLockOrName,
    /// `No such contact.`
    NoSuchContact,
    /// `That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.`
    SpeedLimits { ahead_kph: i32, astern_kph: i32 },
    /// `Your tank is destroyed and takes no orders. EXIT leaves the pod.`
    DestroyedNoOrders,
    /// `EMCON on: the pod radiates.` / `EMCON off: the pod is silent.` (bare
    /// `EMCON`)
    EmconReading { on: bool },
    /// **`VIEW BRIEF`'s header** (cut AN.2; `help cockpit`: "`VIEW BRIEF`
    /// says the same window in sentences"), read whole: `Window around RA:
    /// cells 0 to 38 east and 88 to 107 north of 256 by 256; you at (6, 97),
    /// heading 090°; 10 m a cell.` — the arena's size in cells (`extent`),
    /// the metres a cell and the own cell. A header cut short (a wrapped
    /// one) is [`Line::Other`].
    ViewHeader {
        extent: (u32, u32),
        cell_m: u32,
        you: (u32, u32),
    },
    /// **One bearing of `VIEW BRIEF`'s ground** (cut AN.2): `North: 10 m of
    /// road, then grass to 90 m, then beyond the window.` — the bearing in
    /// degrees (`North` 0, `North-east` 45, … `North-west` 315), each run as
    /// `(class, reach in metres)` (the first run's length, then each later
    /// run's end reach, all from the own cell's centre), and whether the ray
    /// ends at `the edge of the arena` rather than `beyond the window`. The
    /// class words are the brief's prose (`road`, `grass`, `bare ground`,
    /// `light woods`, `heavy woods`, `deep water`, …). A line with no
    /// recognised tail (one the terminal wrapped) is [`Line::Other`].
    GroundRay {
        bearing_deg: u32,
        runs: Vec<(String, u32)>,
        edge: bool,
    },
    /// `The window is dark: the panel has told you nothing yet.` / `The
    /// window is dark: there is no ground under the panel to draw.` — `VIEW`'s
    /// two answers with no ground (cut AN.2).
    WindowDark,
    /// `No contacts in the window.` — `VIEW BRIEF`'s own empty-contacts
    /// sentence (the game's cut AP.4, 16c `N20`), where it used to say
    /// `CONTACTS`' `No contacts.`: the brief's line, never a `CONTACTS` reply.
    NoContactsInWindow,
    /// `End of brief.` — `VIEW BRIEF`'s fixed last line (the game's cut AP.4):
    /// the brief's close, printed after its last spot line every time.
    BriefEnd,

    // -- acks of the pilot's own orders (journal false) ----------------------
    /// `Startup sequence begun; power comes online shortly.`
    StartupBegun,
    /// `Power is already online.`
    PowerAlreadyOnline,
    /// `Power is still coming up; hold on.`
    PowerStarting,
    /// `Power not online: STARTUP first.`
    PowerNotOnline,
    /// `Your tank cannot move.` — `HEADING`/`SPEED`'s own pre-check.
    Immobile,
    /// `Steering to 90°.`
    Steering { deg: u32 },
    /// `Making -27 kph.`
    Making { kph: i32 },
    /// `Locking BA (FRIENDLY).`
    Locking { designator: String, friendly: bool },
    /// `Firing cannon.`
    Firing { weapon: String },
    /// `EMCON coming on; …` / `EMCON going off; …` / `EMCON is already on.`
    EmconAck { on: bool },

    // -- the unseated, reclaim and resume sentences --------------------------
    /// `You are not at the station of a deployed unit.` (journal false)
    OutsideCockpit,
    /// `You are not in a pod.` (`EXIT` only)
    NotInAPod,
    /// `Boarding is through one of the simulator's team rooms.`
    OutsideTeamRoom,
    /// `The panel is dark until the match starts; …`
    NotStartedYet,
    /// `You are already in your pod.`
    AlreadyAboard,
    /// `Red Tank 1 is still being handed over; try again in 3 second(s).`
    HandingOver { unit: String, secs: u32 },
    /// `You left Red Tank 1 for the rest of the match; it cannot be taken back.`
    LeftForGood { unit: String },
    /// `Back aboard RA after 2 m 14 s away.` — opens the catch-up block.
    BackAboard { callsign: String, away_ms: u64 },
    /// The catch-up block's own `  Power online.` row.
    CatchupPower { power: Power },
    /// `You climb back into Red Pod 1: Red Tank 1 is yours again where it stands.`
    ClimbBack { pod: String, unit: String },
    /// `You are back at your station on Red Tank 1; it held your orders and
    /// fired nothing.` (a revival inside the link-dead grace)
    BackAtStation { unit: String },
    /// `You climb out of Red Pod 1.` (and its running-match form)
    ClimbOut { pod: String },
    /// `The server restarted and the match resumed from its last save; …`
    Resumed,
    /// `Red Tank 1 is flying on doctrine; BOARD takes it back.`
    Doctrine { tank: String },
    /// `Connected as Ada (account ada, 1 connection).`
    Connected { name: String, account: String },

    // -- notices (journal true) -----------------------------------------------
    /// `New contact X2: bearing 087°, effective.` / `New contact X2.`
    NewContact {
        designator: String,
        bearing_deg: Option<u32>,
        band: Option<String>,
    },
    /// `Contact X2 identified: BA, blue.`
    Identified {
        was: String,
        designator: String,
        side: Side,
    },
    /// `BA is unidentified again: X2.`
    Unidentified { was: String, designator: String },
    /// `Lock on BA broken: contact gone.`
    LockBroken { designator: String },
    /// `Contact BA lost.`
    ContactLost { designator: String },
    /// `Hit on BA (FRIENDLY): turret, already destroyed.` / `Hit on X2.` /
    /// `Hit.`
    HitScored {
        designator: Option<String>,
        friendly: bool,
        section: Option<String>,
        already_destroyed: bool,
    },
    /// `Miss on BA: the target moved.` / `Miss.` / `Miss: the round went wide.`
    Missed {
        designator: Option<String>,
        words: Option<String>,
    },
    /// `BA destroyed.` / `Target destroyed.`
    Killed { designator: Option<String> },
    /// `BA's turret is destroyed.`
    TargetSectionLost { designator: String, section: String },
    /// The victim's line — always `Hit on the …`, because a designator never
    /// starts `the `: `Hit on the hull's front facet: 280 of 400 armour left.`,
    /// `…: armour gone, 140 of 200 inside.`, `Hit on the engine, through the
    /// hull's rear facet: …`, `Hit on the left_track, which is already destroyed.`
    HitTaken {
        section: String,
        /// The section whose armour took it, when it is not `section` itself.
        through: Option<String>,
        facet: Option<String>,
        /// `(left, max)` when the line carries the armour.
        armour: Option<(i64, i64)>,
        /// `(left, max)` when the armour was gone.
        internals: Option<(i64, i64)>,
        already_destroyed: bool,
    },
    /// `The left_track is destroyed: the tank is slowed.`
    SectionLost {
        section: String,
        cost: Option<String>,
    },
    /// `Your tank is destroyed. The match runs on without you: …`
    OwnDestroyed,
    /// `Your tank can neither move nor shoot. The match runs on: …`
    OwnDisabled,
    /// `Power online.`
    PowerOnline,
    /// One of the four `Held:` lines.
    Held { cause: HeldCause },
    /// `Receiver degraded: something is jamming the sight.`
    ReceiverDegraded,
    /// `Receiver clear.`
    ReceiverClear,
    /// `EMCON on: the pod radiates; every sight inside 60 m is jammed, …` /
    /// `EMCON off: the pod is silent; …`
    EmconSwitched { on: bool, radius_m: Option<u32> },
    /// `FIRE cannon refused. Still cycling: 4 s to go.` — the cockpit's
    /// pre-check and the arena's refusal say the same sentence.
    Refused {
        /// Upper case, as printed: `FIRE`, `HEADING`.
        order: String,
        weapon: Option<String>,
        said: String,
        /// `Still cycling: {n} s to go.`
        cycling_s: Option<u32>,
    },

    // -- the lobby and the centre (journal true) -----------------------------
    /// `Simulator centre: running (match 8) — red wins: blue conceded`
    Board {
        state: String,
        match_id: Option<u64>,
        tail: Option<String>,
    },
    /// `  red: nobody` / `  red: 1 (RA): Ada, pod Red Pod 1, seated; …`
    BoardSide { side: Side, pilots: Vec<BoardPilot> },
    /// `  AI fill: each side fields 2; machines take the unclaimed pods.`
    AiFill { each: u32 },
    /// ``  `SIM TEAM red` or `SIM TEAM blue` opens the next match.``
    BoardNext,
    /// `  mode: hold — keep The Crossroads (B) uncontested for 120 s` / `  mode:
    /// range — 2 targets on the blue side` (cut AN.2): the board's rule line.
    BoardMode(MatchMode),
    /// `Ada takes red slot 1. …` (the claimant is named by display name)
    Claimed { name: String, side: Side, index: u8 },
    /// `Ada gives up red slot 1.`
    GaveUp { name: String, side: Side, index: u8 },
    /// `Match 8 is boarding. …`
    Boarding { match_id: u64 },
    /// `You climb into Red Pod 1. The panel wakes …`
    ClimbIn { pod: String },
    /// `Ada climbs into Red Pod 1.`
    ClimbsIn { name: String, pod: String },
    /// `Match 8 is running. Good hunting.`
    Running { match_id: u64 },
    /// `Match 8 is over (red). Its units are checked in.`
    Over { match_id: u64, result: ResultWord },
    /// `Match 8 is aborted: …`
    Aborted { match_id: u64, why: String },
    /// `Match 8 could not start: …`
    CouldNotStart { match_id: u64, why: String },
    /// `Match 8 ends: every pilot has left it.`
    AllLeft { match_id: u64 },
    /// `Ada calls the match off.`
    CallsOff { name: String },
    /// `Ada aborts match 8. Its units check back in when the arena answers.`
    AbortsMatch { name: String, match_id: u64 },
    /// `The hatch of Red Pod 1 swings open.`
    HatchOpen { pod: String },
    /// `The last match is cleared from the board…`
    BoardCleared,
    /// ``You already hold red slot 1. `SIM LEAVE` gives it up.``
    AlreadyHold { side: Side, index: u8 },
    /// `The red side is full: it takes 6 pilot(s).`
    SideFull { side: Side, max: u32 },
    /// `Only a side's lead or staff can start the match.`
    NotLead,
    /// `The blue side has no pilots; a match takes at least one a side.`
    EmptySide { side: Side },
    /// `The simulator centre is running; that is not something you can do to it
    /// now.`
    WrongCentreState { state: String },
    /// `This server has no simulator centre.`
    NoCentre,
    /// `The TacSim layer is off on this server, …`
    LayerOff,
    /// `Nobody is in the simulator; a match takes at least one pilot.`
    NobodyPlaying,
    /// `The simulator is through the lobby.`
    OutsideCentre,
    /// `You hold no slot in the simulator.`
    NoSlot,
    /// ``Nothing is running yet. `SIM LEAVE` gives your slot up.``
    NothingToAbort,

    // -- rooms, hints, transients --------------------------------------------
    /// A room heading: `Red Room #57` reads `Red Room`.
    Room(String),
    /// `Hint: …` (the text after the prefix).
    Hint(String),
    /// A line to re-send after a moment: a commit in flight, a world still
    /// starting, a control lease not yet mirrored, a busy arena.
    Transient(String),
    /// Anything else, trimmed: speech, room text, the echo, a wrapped tail.
    Other(String),
}

/// The rooms a heading line can name that the picture follows.
const ROOMS: [&str; 4] = ["Welcome Hall", "Simulator Lobby", "Red Room", "Blue Room"];

/// Whether `name` is one of the centre's own rooms (lobby, team rooms, pods).
pub fn is_centre_room(name: &str) -> bool {
    matches!(name, "Simulator Lobby" | "Red Room" | "Blue Room") || pod_number(name).is_some()
}

/// `Red Pod 3` → `(Red, 3)`.
pub fn pod_number(name: &str) -> Option<(Side, u8)> {
    let (side, rest) = if let Some(rest) = name.strip_prefix("Red Pod ") {
        (Side::Red, rest)
    } else {
        (Side::Blue, name.strip_prefix("Blue Pod ")?)
    };
    let n: u8 = rest.parse().ok()?;
    (1..=6).contains(&n).then_some((side, n))
}

/// A designator a pilot reads and types: `X<n>` for a contact not yet told
/// apart, else the side's letter and the pod's letters (`RA`, `BZ`, `RAA`).
pub fn is_designator(word: &str) -> bool {
    if let Some(number) = word.strip_prefix('X') {
        return (1..=6).contains(&number.len()) && number.bytes().all(|b| b.is_ascii_digit());
    }
    let letters = match word.strip_prefix('R').or_else(|| word.strip_prefix('B')) {
        Some(letters) => letters,
        None => return false,
    };
    (1..=4).contains(&letters.len()) && letters.bytes().all(|b| b.is_ascii_uppercase())
}

/// A component, section or facet id: `cannon`, `left_track`, `front`.
fn is_id(word: &str) -> bool {
    (1..=48).contains(&word.len())
        && word
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn uint(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn int<T: std::str::FromStr>(text: &str) -> Option<T> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// `087°` → 87.
fn degrees(text: &str) -> Option<u32> {
    uint(text.strip_suffix('°')?)
}

/// `087° (rel 032°)` → (87, 32).
fn bearing_pair(text: &str) -> Option<(u32, u32)> {
    let (bearing, rel) = text.split_once(" (rel ")?;
    Some((degrees(bearing)?, degrees(rel.strip_suffix(')')?)?))
}

/// `The Ford (A)` → ("The Ford", 'A').
fn spot_named(text: &str) -> Option<(String, char)> {
    let (name, letter) = text.strip_suffix(')')?.rsplit_once(" (")?;
    let mut chars = letter.chars();
    let (Some(letter), None) = (chars.next(), chars.next()) else {
        return None;
    };
    (letter.is_ascii_uppercase() && !name.is_empty()).then(|| (name.to_owned(), letter))
}

/// `, plotted 4 s ago` after the figure, or nothing.
fn plotted(text: &str) -> Option<(&str, Option<u32>)> {
    match text.split_once(", plotted ") {
        Some((figure, ago)) => Some((figure, Some(uint(ago.strip_suffix(" s ago")?)?))),
        None => Some((text, None)),
    }
}

/// `Match 8 …` → (8, "…").
fn match_numbered(body: &str) -> Option<(u64, &str)> {
    let (number, rest) = body.strip_prefix("Match ")?.split_once(' ')?;
    if !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((number.parse().ok()?, rest))
}

/// `a/b` → (a, b).
fn fraction(text: &str) -> Option<(i64, i64)> {
    let (a, b) = text.split_once('/')?;
    Some((int(a)?, int(b)?))
}

/// The duration words the match record prints (`45 s`, `2 m 14 s`, `1 h 2 m 3
/// s`), in milliseconds.
pub fn duration_ms(words: &str) -> Option<u64> {
    let tokens: Vec<&str> = words.split(' ').collect();
    if tokens.is_empty() || !tokens.len().is_multiple_of(2) {
        return None;
    }
    let mut total: u64 = 0;
    for pair in tokens.chunks(2) {
        let [number, unit] = pair else { return None };
        let n = u64::from(uint(number)?);
        let scale = match *unit {
            "h" => 3_600_000,
            "m" => 60_000,
            "s" => 1_000,
            _ => return None,
        };
        total = total.saturating_add(n.saturating_mul(scale));
    }
    Some(total)
}

/// **Read one line.** Pure and total: every input is exactly one variant, and
/// [`Line::Other`] (the trimmed line) is the fallback.
pub fn classify(line: &str) -> Line {
    let body = line.trim();
    if body.is_empty() {
        return Line::Other(String::new());
    }
    // **An indented line is only ever one of the game's own indented rows.**
    // Every game sentence this reader knows starts at column 0 except the
    // `STATUS` rows and the board's and catch-up block's rows (two spaces) and
    // the facet runs (four). Anything else that arrives indented is a
    // continuation: the renderer wraps a say, pose, page or whisper with a
    // hanging indent of the speaker's name plus one, so another player can pad
    // a line until `Match 1 is over (red). …` or `New contact X9: …` starts a
    // continuation of their own speech. Such a line never falls through to the
    // column-0 sentences below. (A character's name is 2 to 20 characters, so a
    // character's continuation is indented 3 to 20; a three-letter name lands
    // on four, which is why the picture takes `STATUS` rows only inside an open
    // `STATUS` block.)
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 0 {
        let indented = match indent {
            2 => weapon_arming(body)
                .or_else(|| status_row(body))
                .or_else(|| board_row(body))
                .or_else(|| catchup_row(body)),
            4 => facet_run(body),
            _ => None,
        };
        return indented.unwrap_or_else(|| Line::Other(body.to_owned()));
    }
    // **A column-0 `[…` line is another player's** (cut AM.1): a channel line
    // (`render_channel`: `[{channel}] …`) or a notice's `[{category}] …`; no
    // game sentence this reader knows starts with `[`. So a teammate's channel
    // line that ends `please retry in a moment.` is never a transient, and it
    // always begins a channel chain ([`continues`]).
    if body.starts_with('[') {
        return Line::Other(body.to_owned());
    }
    // First-time advice (cut AH), before anything else reads the words in it.
    if let Some(text) = body.strip_prefix("Hint: ") {
        return Line::Hint(text.to_owned());
    }
    if let Some(transient) = transient(body) {
        return transient;
    }
    exact(body)
        .or_else(|| cockpit_reply(body))
        .or_else(|| notice(body))
        .or_else(|| unseated(body))
        .or_else(|| lobby(body))
        .or_else(|| room(body))
        .unwrap_or_else(|| Line::Other(body.to_owned()))
}

/// The four lines worth sending again after a moment. The seat's refusal is
/// read at the END of a line (`That cockpit will not wake for you: control
/// state unknown`), never anywhere inside one, so a player's line that merely
/// mentions it is not taken for it.
fn transient(body: &str) -> Option<Line> {
    let is = body.ends_with("please retry in a moment.")
        || body == "The world is still starting; please try again shortly."
        || body.ends_with("control state unknown")
        || body.ends_with("control state unknown.")
        || body == "The arena is busy; try that again in a moment.";
    is.then(|| Line::Transient(body.to_owned()))
}

/// **`WEAPONSTATUS`'s arming line** (the game's 17b cut M1b.4), after its two
/// spaces: `S1: >=60% side/rear` — `S`, the slot number, `: `, then the terms
/// (never empty), kept verbatim.
fn weapon_arming(body: &str) -> Option<Line> {
    let (slot, terms) = body.strip_prefix('S')?.split_once(": ")?;
    if slot.is_empty() || slot.len() > 3 || !slot.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let terms = terms.trim();
    let slot: u8 = slot.parse().ok()?;
    (!terms.is_empty()).then(|| Line::WeaponArming {
        slot,
        terms: terms.to_owned(),
    })
}

/// `STATUS`'s two-space rows.
fn status_row(body: &str) -> Option<Line> {
    if let Some(rest) = body.strip_prefix("at ") {
        let (x, rest) = rest.split_once(" m east, ")?;
        let (y, scope) = rest.split_once(" m north; ")?;
        let scope = match scope {
            "scope clear" => Scope::Clear,
            "scope holds a hostile" => Scope::Hostile,
            "scope holds an unidentified contact" => Scope::Unidentified,
            _ => return None,
        };
        return Some(Line::StatusPosition {
            x_m: int(x)?,
            y_m: int(y)?,
            scope,
        });
    }
    if let Some(rest) = body.strip_prefix("nearest spot: ") {
        let (spot, rest) = rest.rsplit_once(", ")?;
        let (name, letter) = spot_named(spot)?;
        let (range, bearing) = rest.split_once(" m at ")?;
        return Some(Line::StatusNearestSpot {
            name,
            letter,
            range_m: uint(range)?,
            bearing_deg: degrees(bearing)?,
        });
    }
    if let Some(rest) = body.strip_prefix("heat ") {
        let (rest, overheated) = match rest.strip_suffix(", OVERHEATED") {
            Some(rest) => (rest, true),
            None => (rest, false),
        };
        let (heat, max) = rest.split_once(" of ")?;
        return Some(Line::StatusHeat {
            heat: uint(heat)?,
            max: uint(max)?,
            overheated,
        });
    }
    if let Some(rest) = body.strip_prefix("receiver degraded: ") {
        let (budget, rest) = rest.split_once(" of ")?;
        let (max, reach) = rest.split_once(" through cover, ")?;
        return Some(Line::StatusReceiver {
            budget: uint(budget)?,
            budget_max: uint(max)?,
            reach_percent: uint(reach.strip_suffix("% reach")?)?,
        });
    }
    match body {
        "emcon on: the pod radiates" => return Some(Line::StatusEmcon { on: true }),
        "emcon off: the pod is silent" => return Some(Line::StatusEmcon { on: false }),
        _ => {}
    }
    // `hull  intact  internals 60/60`: the only two-space row with a double
    // space inside it.
    let parts: Vec<&str> = body.split("  ").collect();
    let (id, condition, internals) = match parts.as_slice() {
        [id, condition] => (*id, *condition, None),
        [id, condition, internals] => (
            *id,
            *condition,
            Some(fraction(internals.strip_prefix("internals ")?)?),
        ),
        _ => return None,
    };
    (is_id(id) && is_id(condition)).then(|| Line::StatusSection {
        id: id.to_owned(),
        condition: condition.to_owned(),
        internals,
    })
}

/// A four-space facet run: `front 400/400  left 250/250`.
fn facet_run(body: &str) -> Option<Line> {
    let mut facets = Vec::new();
    for cell in body.split("  ") {
        let (facet, armour) = cell.split_once(' ')?;
        if !is_id(facet) {
            return None;
        }
        let (left, max) = fraction(armour)?;
        facets.push((facet.to_owned(), left, max));
    }
    Some(Line::StatusFacets { facets })
}

/// The board's two-space rows.
fn board_row(body: &str) -> Option<Line> {
    if body == "`SIM TEAM red` or `SIM TEAM blue` opens the next match." {
        return Some(Line::BoardNext);
    }
    if let Some(rest) = body.strip_prefix("AI fill: each side fields ") {
        let each = rest.strip_suffix("; machines take the unclaimed pods.")?;
        return Some(Line::AiFill { each: uint(each)? });
    }
    // The rule (cut AN.2): `mode: hold — …` or `mode: range — …`; any other
    // word is not a mode this reader knows.
    if let Some(rest) = body.strip_prefix("mode: ") {
        let (word, tail) = rest.split_once(" — ")?;
        if tail.is_empty() {
            return None;
        }
        return match word {
            "hold" => Some(Line::BoardMode(MatchMode::Hold)),
            "range" => Some(Line::BoardMode(MatchMode::Range)),
            _ => None,
        };
    }
    let (side, rest) = body.split_once(": ")?;
    let side = match side {
        "red" => Side::Red,
        "blue" => Side::Blue,
        _ => return None,
    };
    if rest == "nobody" {
        return Some(Line::BoardSide {
            side,
            pilots: Vec::new(),
        });
    }
    let pilots = rest
        .split("; ")
        .map(board_pilot)
        .collect::<Option<Vec<BoardPilot>>>()?;
    Some(Line::BoardSide { side, pilots })
}

/// `1 (RA): Ada [biped_mk1], pod Red Pod 1, seated` — and, during
/// selection, `1 (RA): Ada [biped_mk1], a mind` (the game's 17b cut M5.1).
fn board_pilot(entry: &str) -> Option<BoardPilot> {
    let (index, rest) = entry.split_once(" (")?;
    let (callsign, rest) = rest.split_once("): ")?;
    if !is_designator(callsign) {
        return None;
    }
    let mut rest = rest;
    let mut standing = None;
    for (suffix, word) in [
        (", seated", Standing::Seated),
        (", left", Standing::Left),
        (", flown by doctrine", Standing::Doctrine),
    ] {
        if let Some(before) = rest.strip_suffix(suffix) {
            rest = before;
            standing = Some(word);
            break;
        }
    }
    let mut pod = None;
    if let Some((before, name)) = rest.rsplit_once(", pod ") {
        rest = before;
        pod = Some(name.to_owned());
    }
    // The game's 17b cut M5.1: the selection fog's mark, after the bracket.
    let mut mind = false;
    if let Some(before) = rest.strip_suffix(", a mind") {
        rest = before;
        mind = true;
    }
    let mut unit_type = None;
    if let Some(before) = rest.strip_suffix(']')
        && let Some((name, kind)) = before.rsplit_once(" [")
    {
        rest = name;
        unit_type = Some(kind.to_owned());
    }
    let index: u8 = uint(index)?.try_into().ok()?;
    (!rest.is_empty()).then(|| BoardPilot {
        index,
        callsign: callsign.to_owned(),
        name: rest.to_owned(),
        unit_type,
        mind,
        pod,
        standing,
    })
}

/// The catch-up block's `  Power online.` row; the block's other rows are
/// [`Line::Other`].
fn catchup_row(body: &str) -> Option<Line> {
    let word = body.strip_prefix("Power ")?.strip_suffix('.')?;
    Some(Line::CatchupPower {
        power: Power::parse(word)?,
    })
}

/// Whole-line sentences.
fn exact(body: &str) -> Option<Line> {
    Some(match body {
        "No contacts." => Line::NoContacts,
        "No contacts in the window." => Line::NoContactsInWindow,
        "End of brief." => Line::BriefEnd,
        "This unit carries no weapon." => Line::NoWeapon,
        "The window is dark: the panel has told you nothing yet."
        | "The window is dark: there is no ground under the panel to draw." => Line::WindowDark,
        "The panel has told you nothing yet." => Line::NoFrameYet,
        "No lock: LOCK a contact first." => Line::NoLock,
        "No lock: LOCK a contact first, or name one." => Line::NoLockOrName,
        "No such contact." => Line::NoSuchContact,
        "Your tank is destroyed and takes no orders. EXIT leaves the pod." => {
            Line::DestroyedNoOrders
        }
        "EMCON on: the pod radiates." => Line::EmconReading { on: true },
        "EMCON off: the pod is silent." => Line::EmconReading { on: false },
        "Startup sequence begun; power comes online shortly." => Line::StartupBegun,
        "Power is already online." => Line::PowerAlreadyOnline,
        "Power is still coming up; hold on." => Line::PowerStarting,
        "Power not online: STARTUP first." => Line::PowerNotOnline,
        "Your tank cannot move." => Line::Immobile,
        "EMCON coming on; the pod radiates from the next pulse." | "EMCON is already on." => {
            Line::EmconAck { on: true }
        }
        "EMCON going off; the pod is silent from the next pulse." | "EMCON is already off." => {
            Line::EmconAck { on: false }
        }
        "You are not at the station of a deployed unit." => Line::OutsideCockpit,
        "You are not in a pod." => Line::NotInAPod,
        "Boarding is through one of the simulator's team rooms." => Line::OutsideTeamRoom,
        "The panel is dark until the match starts; the simulator is still waiting for the \
         other pods." => Line::NotStartedYet,
        "You are already in your pod." => Line::AlreadyAboard,
        "The server restarted and the match resumed from its last save; what happened after \
         that save did not count." => Line::Resumed,
        // The two own-tank sentences run past 120 columns and wrap: read by the
        // opening, and the wrapped tail is `Other`.
        _ if body.starts_with("Your tank is destroyed. The match runs on") => Line::OwnDestroyed,
        _ if body.starts_with("Your tank can neither move nor shoot.") => Line::OwnDisabled,
        "Power online." => Line::PowerOnline,
        "Held: the ground ahead will not take the tank." => Line::Held {
            cause: HeldCause::Impassable,
        },
        "Held: something is in the way." => Line::Held {
            cause: HeldCause::Occupied,
        },
        "Held: this is the edge of the arena." => Line::Held {
            cause: HeldCause::Edge,
        },
        "Held: the slope ahead is too steep for the tank." => Line::Held {
            cause: HeldCause::TooSteep,
        },
        "Receiver degraded: something is jamming the sight." => Line::ReceiverDegraded,
        "Receiver clear." => Line::ReceiverClear,
        "EMCON off: the pod is silent; your own sight clears, and nothing hides you." => {
            Line::EmconSwitched {
                on: false,
                radius_m: None,
            }
        }
        "Hit." => Line::HitScored {
            designator: None,
            friendly: false,
            section: None,
            already_destroyed: false,
        },
        "Miss." => Line::Missed {
            designator: None,
            words: None,
        },
        "Target destroyed." => Line::Killed { designator: None },
        "Only a side's lead or staff can start the match." => Line::NotLead,
        "This server has no simulator centre." => Line::NoCentre,
        "Nobody is in the simulator; a match takes at least one pilot." => Line::NobodyPlaying,
        "The simulator is through the lobby." => Line::OutsideCentre,
        "You hold no slot in the simulator." => Line::NoSlot,
        "Nothing is running yet. `SIM LEAVE` gives your slot up." => Line::NothingToAbort,
        _ => return None,
    })
}

/// The cockpit's read replies and acks that carry figures.
fn cockpit_reply(body: &str) -> Option<Line> {
    if let Some(line) = contact_row(body) {
        return Some(line);
    }
    if let Some(line) = lock_line(body) {
        return Some(line);
    }
    if let Some(line) = weapon_row(body) {
        return Some(line);
    }
    if let Some(callsign) = body
        .strip_prefix("Callsign ")
        .and_then(|rest| rest.strip_suffix('.'))
    {
        return is_designator(callsign).then(|| Line::StatusCallsign {
            callsign: callsign.to_owned(),
        });
    }
    if let Some(name) = body.strip_suffix(": power off. The panel has told you nothing yet.") {
        return Some(Line::StatusUntold {
            name: name.to_owned(),
        });
    }
    if let Some(line) = status_head(body) {
        return Some(line);
    }
    if let Some(rest) = body
        .strip_prefix("Range to ")
        .and_then(|rest| rest.strip_suffix('.'))
    {
        let (target, figure) = rest.split_once(": ")?;
        let (figure, plotted_s) = plotted(figure)?;
        let range_m = uint(figure.strip_suffix(" m")?)?;
        if let Some((name, letter)) = spot_named(target) {
            return plotted_s.is_none().then_some(Line::SpotRange {
                name,
                letter,
                range_m,
            });
        }
        return is_designator(target).then(|| Line::Range {
            designator: target.to_owned(),
            range_m,
            plotted_s,
        });
    }
    if let Some(rest) = body
        .strip_prefix("Bearing to ")
        .and_then(|rest| rest.strip_suffix('.'))
    {
        let (target, figure) = rest.split_once(": ")?;
        let (figure, plotted_s) = plotted(figure)?;
        let (bearing_deg, rel_deg) = bearing_pair(figure)?;
        if let Some((name, letter)) = spot_named(target) {
            return plotted_s.is_none().then_some(Line::SpotBearing {
                name,
                letter,
                bearing_deg,
                rel_deg,
            });
        }
        return is_designator(target).then(|| Line::Bearing {
            designator: target.to_owned(),
            bearing_deg,
            rel_deg,
            plotted_s,
        });
    }
    if let Some(rest) = body.strip_prefix("That unit makes ") {
        let (ahead, rest) = rest.split_once(" kph ahead and ")?;
        let (astern, rest) = rest.split_once(" astern; SPEED -")?;
        let (floor, ceiling) = rest.strip_suffix('.')?.split_once(" to ")?;
        if floor != astern || ceiling != ahead {
            return None;
        }
        return Some(Line::SpeedLimits {
            ahead_kph: int(ahead)?,
            astern_kph: int(astern)?,
        });
    }
    if let Some(deg) = body
        .strip_prefix("Steering to ")
        .and_then(|rest| rest.strip_suffix("°."))
    {
        return Some(Line::Steering { deg: uint(deg)? });
    }
    if let Some(kph) = body
        .strip_prefix("Making ")
        .and_then(|rest| rest.strip_suffix(" kph."))
    {
        return Some(Line::Making { kph: int(kph)? });
    }
    if let Some(named) = body
        .strip_prefix("Locking ")
        .and_then(|rest| rest.strip_suffix('.'))
    {
        let (designator, friendly) = friendly(named);
        return is_designator(designator).then(|| Line::Locking {
            designator: designator.to_owned(),
            friendly,
        });
    }
    if let Some(weapon) = body
        .strip_prefix("Firing ")
        .and_then(|rest| rest.strip_suffix('.'))
    {
        return is_id(weapon).then(|| Line::Firing {
            weapon: weapon.to_owned(),
        });
    }
    if let Some(weapon) = body
        .strip_prefix("That unit carries no weapon called `")
        .and_then(|rest| rest.strip_suffix("`."))
    {
        return is_id(weapon).then(|| Line::NoSuchWeapon {
            weapon: weapon.to_owned(),
        });
    }
    if let Some(line) = view_header(body) {
        return Some(line);
    }
    ground_ray(body)
}

/// **`VIEW BRIEF`'s header** (cut AN.2), read whole or not at all: `Window
/// around {cs}: cells {a} to {b} east and {c} to {d} north of {W} by {H}; you
/// at ({x}, {y}), heading {h}°; {m} m a cell.` Every figure is a whole number
/// and the callsign a designator; the window's own corners are read and
/// checked but not kept (the pilot works from its own cell and the
/// arena's size).
fn view_header(body: &str) -> Option<Line> {
    let rest = body.strip_prefix("Window around ")?;
    let (callsign, rest) = rest.split_once(": cells ")?;
    let (west, rest) = rest.split_once(" to ")?;
    let (east, rest) = rest.split_once(" east and ")?;
    let (south, rest) = rest.split_once(" to ")?;
    let (north, rest) = rest.split_once(" north of ")?;
    let (width, rest) = rest.split_once(" by ")?;
    let (height, rest) = rest.split_once("; you at (")?;
    let (x, rest) = rest.split_once(", ")?;
    let (y, rest) = rest.split_once("), heading ")?;
    let (heading, rest) = rest.split_once("°; ")?;
    let metres = rest.strip_suffix(" m a cell.")?;
    if !is_designator(callsign) {
        return None;
    }
    for figure in [west, east, south, north, heading] {
        uint(figure)?;
    }
    Some(Line::ViewHeader {
        extent: (uint(width)?, uint(height)?),
        cell_m: uint(metres)?,
        you: (uint(x)?, uint(y)?),
    })
}

/// The eight bearing words of a brief, compass order from north, with their
/// degrees (cut AN.2).
pub const BRIEF_BEARINGS: [(&str, u32); 8] = [
    ("North", 0),
    ("North-east", 45),
    ("East", 90),
    ("South-east", 135),
    ("South", 180),
    ("South-west", 225),
    ("West", 270),
    ("North-west", 315),
];

/// A class word of a brief (cut AN.2): one or more words of lower-case
/// letters, one space between (`grass`, `bare ground`, `deep water`).
fn class_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .split(' ')
            .all(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase()))
}

/// `{n} m` → n.
fn metres(text: &str) -> Option<u32> {
    uint(text.strip_suffix(" m")?)
}

/// **One bearing of a brief's ground** (cut AN.2): `{Bearing}: {n} m of
/// {class}(, then {class} to {n} m)*, then (beyond the window|the edge of the
/// arena).` or `{Bearing}: (beyond the window|the edge of the arena).`; a line
/// with neither tail is not one.
fn ground_ray(body: &str) -> Option<Line> {
    let (word, rest) = body.split_once(": ")?;
    let bearing_deg = BRIEF_BEARINGS
        .iter()
        .find(|(name, _)| *name == word)
        .map(|(_, deg)| *deg)?;
    let (rest, edge) = match rest.strip_suffix("the edge of the arena.") {
        Some(rest) => (rest, true),
        None => (rest.strip_suffix("beyond the window.")?, false),
    };
    if rest.is_empty() {
        return Some(Line::GroundRay {
            bearing_deg,
            runs: Vec::new(),
            edge,
        });
    }
    let rest = rest.strip_suffix(", then ")?;
    let mut pieces = rest.split(", then ");
    let (first, class) = pieces.next()?.split_once(" m of ")?;
    if !class_word(class) {
        return None;
    }
    let mut runs = vec![(class.to_owned(), uint(first)?)];
    for piece in pieces {
        let (class, reach) = piece.rsplit_once(" to ")?;
        if !class_word(class) {
            return None;
        }
        runs.push((class.to_owned(), metres(reach)?));
    }
    Some(Line::GroundRay {
        bearing_deg,
        runs,
        edge,
    })
}

/// `BA (FRIENDLY)` → ("BA", true).
fn friendly(named: &str) -> (&str, bool) {
    match named.strip_suffix(" (FRIENDLY)") {
        Some(designator) => (designator, true),
        None => (named, false),
    }
}

/// `BA  blue  bearing 087° (rel 032°)  far  450 m  2 s` — the designator is
/// left-aligned in four columns, so a two-letter one is followed by two spaces
/// and a four-letter one by none: the class word is read off the end of the
/// head, not by column.
fn contact_row(body: &str) -> Option<Line> {
    let (head, rest) = body.split_once("  bearing ")?;
    let head = head.trim_end();
    let (designator, side) = [
        ("unknown", None),
        ("red", Some(Side::Red)),
        ("blue", Some(Side::Blue)),
    ]
    .into_iter()
    .find_map(|(class, side)| head.strip_suffix(class).map(|d| (d.trim_end(), side)))?;
    if !is_designator(designator) {
        return None;
    }
    let mut parts = rest.split("  ");
    let (bearing_deg, rel_deg) = bearing_pair(parts.next()?)?;
    let band = parts.next()?;
    if !matches!(band, "close" | "effective" | "far" | "beyond") {
        return None;
    }
    let range_m = uint(parts.next()?.strip_suffix(" m")?)?;
    let age_s = uint(parts.next()?.strip_suffix(" s")?)?;
    // 17b cut M1a.3: the optional tail — `  {aspect}`, then optionally `  -`
    // or `  {weapon} {NN}%` — and nothing after it.
    let (aspect, odds) = match parts.next() {
        None => (None, None),
        Some(word) => {
            if !matches!(word, "front" | "side" | "rear") {
                return None;
            }
            let odds = match parts.next() {
                None | Some("-") => None,
                Some(cell) => {
                    let (weapon, percent) = cell.split_once(' ')?;
                    if !is_id(weapon) {
                        return None;
                    }
                    Some((weapon.to_owned(), uint(percent.strip_suffix('%')?)?))
                }
            };
            (Some(word.to_owned()), Some(odds))
        }
    };
    if parts.next().is_some() {
        return None;
    }
    Some(Line::ContactRow(ContactRow {
        designator: designator.to_owned(),
        side,
        bearing_deg,
        rel_deg,
        band: band.to_owned(),
        range_m,
        age_s,
        aspect,
        odds,
    }))
}

/// `Locked on BA (FRIENDLY), 62% to hit.`
fn lock_line(body: &str) -> Option<Line> {
    let rest = body.strip_prefix("Locked on ")?.strip_suffix('.')?;
    let (named, percent) = match rest.rsplit_once(", ") {
        Some((named, chance)) if chance.ends_with("% to hit") => {
            (named, Some(uint(chance.strip_suffix("% to hit")?)?))
        }
        _ => (rest, None),
    };
    let (designator, friendly) = friendly(named);
    is_designator(designator).then(|| Line::Lock {
        designator: designator.to_owned(),
        friendly,
        percent,
    })
}

/// `machine_gun (mg)  cycling (4 s)  380 of 500 round(s)  band 1 of 2  21% to hit`
fn weapon_row(body: &str) -> Option<Line> {
    let mut parts = body.split("  ");
    let named = parts.next()?;
    let (id, short) = match named.split_once(" (") {
        Some((id, short)) => {
            let short = short.strip_suffix(')')?;
            if !is_id(short) {
                return None;
            }
            (id, Some(short.to_owned()))
        }
        None => (named, None),
    };
    if !is_id(id) {
        return None;
    }
    let (ready, cycling_s) = match parts.next()? {
        "ready" => (true, None),
        other => {
            let secs = other.strip_prefix("cycling (")?.strip_suffix(" s)")?;
            (false, Some(uint(secs)?))
        }
    };
    let (rounds, max) = parts
        .next()?
        .strip_suffix(" round(s)")?
        .split_once(" of ")?;
    let mut row = WeaponRow {
        id: id.to_owned(),
        short,
        ready,
        cycling_s,
        rounds: uint(rounds)?,
        rounds_max: uint(max)?,
        band: None,
        percent: None,
    };
    for part in parts {
        if let Some(band) = part.strip_prefix("band ") {
            if row.band.is_some() || row.percent.is_some() {
                return None;
            }
            let (index, of) = band.split_once(" of ")?;
            row.band = Some((uint(index)?, uint(of)?));
        } else {
            let percent = part.strip_suffix("% to hit")?;
            if row.percent.is_some() {
                return None;
            }
            row.percent = Some(uint(percent)?);
        }
    }
    Some(Line::WeaponRow(row))
}

/// `Red Tank 1 [biped_mk1]: power online, -27 kph, heading 090°, HELD, DESTROYED`
fn status_head(body: &str) -> Option<Line> {
    let (who, rest) = body.split_once(": power ")?;
    let mut parts = rest.split(", ");
    let power = Power::parse(parts.next()?)?;
    let kph = int(parts.next()?.strip_suffix(" kph")?)?;
    let heading_deg = degrees(parts.next()?.strip_prefix("heading ")?)?;
    let (mut held, mut destroyed) = (false, false);
    for flag in parts {
        match flag {
            "HELD" if !held => held = true,
            "DESTROYED" if !destroyed => destroyed = true,
            _ => return None,
        }
    }
    let (name, chassis) = match who
        .strip_suffix(']')
        .and_then(|before| before.rsplit_once(" ["))
    {
        Some((name, chassis)) => (name, Some(chassis.to_owned())),
        None => (who, None),
    };
    (!name.is_empty()).then(|| {
        Line::StatusHead(StatusHead {
            name: name.to_owned(),
            chassis,
            power,
            kph,
            heading_deg,
            held,
            destroyed,
        })
    })
}

/// The cockpit's notices (`help cockpit` "Seeing", "Shooting").
fn notice(body: &str) -> Option<Line> {
    if let Some(rest) = body.strip_prefix("New contact ") {
        let rest = rest.strip_suffix('.')?;
        let (designator, bearing_deg, band) = match rest.split_once(": bearing ") {
            Some((designator, rest)) => {
                let (bearing, band) = rest.split_once(", ")?;
                if !matches!(band, "close" | "effective" | "far" | "beyond") {
                    return None;
                }
                (designator, Some(degrees(bearing)?), Some(band.to_owned()))
            }
            None => (rest, None, None),
        };
        return is_designator(designator).then(|| Line::NewContact {
            designator: designator.to_owned(),
            bearing_deg,
            band,
        });
    }
    if let Some(rest) = body.strip_prefix("Contact ") {
        if let Some(designator) = rest.strip_suffix(" lost.") {
            return is_designator(designator).then(|| Line::ContactLost {
                designator: designator.to_owned(),
            });
        }
        let (was, rest) = rest.split_once(" identified: ")?;
        let (designator, side) = rest.strip_suffix('.')?.split_once(", ")?;
        let side = Side::parse(side).filter(|s| s.as_str() == side)?;
        return (is_designator(was) && is_designator(designator)).then(|| Line::Identified {
            was: was.to_owned(),
            designator: designator.to_owned(),
            side,
        });
    }
    if let Some((was, designator)) = body
        .strip_suffix('.')
        .and_then(|rest| rest.split_once(" is unidentified again: "))
    {
        return (is_designator(was) && is_designator(designator)).then(|| Line::Unidentified {
            was: was.to_owned(),
            designator: designator.to_owned(),
        });
    }
    if let Some(designator) = body
        .strip_prefix("Lock on ")
        .and_then(|rest| rest.strip_suffix(" broken: contact gone."))
    {
        return is_designator(designator).then(|| Line::LockBroken {
            designator: designator.to_owned(),
        });
    }
    if let Some(rest) = body.strip_prefix("Hit on the ") {
        return hit_taken(rest.strip_suffix('.')?);
    }
    if let Some(rest) = body
        .strip_prefix("Hit on ")
        .and_then(|rest| rest.strip_suffix('.'))
    {
        let (named, section) = match rest.split_once(": ") {
            Some((named, section)) => (named, Some(section)),
            None => (rest, None),
        };
        let (designator, friendly) = friendly(named);
        let (section, already_destroyed) = match section {
            Some(section) => match section.strip_suffix(", already destroyed") {
                Some(section) => (Some(section), true),
                None => (Some(section), false),
            },
            None => (None, false),
        };
        if !is_designator(designator) || section.is_some_and(|s| !is_id(s)) {
            return None;
        }
        return Some(Line::HitScored {
            designator: Some(designator.to_owned()),
            friendly,
            section: section.map(str::to_owned),
            already_destroyed,
        });
    }
    if let Some(words) = body
        .strip_prefix("Miss: ")
        .and_then(|rest| rest.strip_suffix('.'))
    {
        return Some(Line::Missed {
            designator: None,
            words: Some(words.to_owned()),
        });
    }
    if let Some((designator, words)) = body
        .strip_prefix("Miss on ")
        .and_then(|rest| rest.strip_suffix('.'))
        .and_then(|rest| rest.split_once(": "))
    {
        return is_designator(designator).then(|| Line::Missed {
            designator: Some(designator.to_owned()),
            words: Some(words.to_owned()),
        });
    }
    if let Some((designator, section)) = body
        .strip_suffix(" is destroyed.")
        .and_then(|rest| rest.split_once("'s "))
        && is_designator(designator)
        && is_id(section)
    {
        return Some(Line::TargetSectionLost {
            designator: designator.to_owned(),
            section: section.to_owned(),
        });
    }
    if let Some(rest) = body.strip_prefix("The ")
        && let Some((section, cost)) = rest.split_once(" is destroyed")
        && is_id(section)
    {
        let cost = match cost {
            "." => None,
            cost => Some(cost.strip_prefix(": ")?.strip_suffix('.')?.to_owned()),
        };
        return Some(Line::SectionLost {
            section: section.to_owned(),
            cost,
        });
    }
    if let Some(designator) = body.strip_suffix(" destroyed.")
        && is_designator(designator)
    {
        return Some(Line::Killed {
            designator: Some(designator.to_owned()),
        });
    }
    if let Some(radius) = body
        .strip_prefix("EMCON on: the pod radiates; every sight inside ")
        .and_then(|rest| rest.strip_suffix(" m is jammed, your own among them."))
    {
        return Some(Line::EmconSwitched {
            on: true,
            radius_m: Some(uint(radius)?),
        });
    }
    if let Some((head, said)) = body.split_once(" refused. ") {
        let mut words = head.split(' ');
        let order = words.next()?;
        let weapon = words.next();
        if words.next().is_some()
            || order.len() < 2
            || !order.bytes().all(|b| b.is_ascii_uppercase())
            || weapon.is_some_and(|w| !is_id(w))
            || said.is_empty()
        {
            return None;
        }
        let cycling_s = said
            .strip_prefix("Still cycling: ")
            .and_then(|rest| rest.strip_suffix(" s to go."))
            .and_then(uint);
        return Some(Line::Refused {
            order: order.to_owned(),
            weapon: weapon.map(str::to_owned),
            said: said.to_owned(),
            cycling_s,
        });
    }
    None
}

/// The victim's line after `Hit on the ` and before the full stop.
fn hit_taken(rest: &str) -> Option<Line> {
    if let Some(section) = rest.strip_suffix(", which is already destroyed") {
        return is_id(section).then(|| Line::HitTaken {
            section: section.to_owned(),
            through: None,
            facet: None,
            armour: None,
            internals: None,
            already_destroyed: true,
        });
    }
    let (place, figures) = rest.split_once(": ")?;
    let place = place.strip_suffix(" facet")?;
    let (section, through, plate) = match place.split_once(", through the ") {
        Some((section, plate)) => {
            let (through, facet) = plate.split_once("'s ")?;
            (section, Some(through), facet)
        }
        None => {
            let (section, facet) = place.split_once("'s ")?;
            (section, None, facet)
        }
    };
    if !is_id(section) || !is_id(plate) || through.is_some_and(|t| !is_id(t)) {
        return None;
    }
    let (armour, internals) = if let Some(inside) = figures
        .strip_prefix("armour gone, ")
        .and_then(|rest| rest.strip_suffix(" inside"))
    {
        let (left, max) = inside.split_once(" of ")?;
        (None, Some((int(left)?, int(max)?)))
    } else {
        let (left, max) = figures.strip_suffix(" armour left")?.split_once(" of ")?;
        (Some((int(left)?, int(max)?)), None)
    };
    Some(Line::HitTaken {
        section: section.to_owned(),
        through: through.map(str::to_owned),
        facet: Some(plate.to_owned()),
        armour,
        internals,
        already_destroyed: false,
    })
}

/// The reclaim, resume and login sentences that carry names.
fn unseated(body: &str) -> Option<Line> {
    if let Some((unit, rest)) = body.split_once(" is still being handed over; try again in ") {
        return Some(Line::HandingOver {
            unit: unit.to_owned(),
            secs: uint(rest.strip_suffix(" second(s).")?)?,
        });
    }
    if let Some(unit) = body
        .strip_prefix("You left ")
        .and_then(|rest| rest.strip_suffix(" for the rest of the match; it cannot be taken back."))
    {
        return Some(Line::LeftForGood {
            unit: unit.to_owned(),
        });
    }
    if let Some(rest) = body
        .strip_prefix("Back aboard ")
        .and_then(|rest| rest.strip_suffix(" away."))
    {
        let (callsign, duration) = rest.split_once(" after ")?;
        return is_designator(callsign).then_some(Line::BackAboard {
            callsign: callsign.to_owned(),
            away_ms: duration_ms(duration)?,
        });
    }
    if let Some(rest) = body
        .strip_prefix("You climb back into ")
        .and_then(|rest| rest.strip_suffix(" is yours again where it stands."))
    {
        let (pod, unit) = rest.split_once(": ")?;
        return Some(Line::ClimbBack {
            pod: pod.to_owned(),
            unit: unit.to_owned(),
        });
    }
    if let Some(unit) = body
        .strip_prefix("You are back at your station on ")
        .and_then(|rest| rest.strip_suffix("; it held your orders and fired nothing."))
    {
        return Some(Line::BackAtStation {
            unit: unit.to_owned(),
        });
    }
    if let Some(rest) = body.strip_prefix("You climb out of ") {
        let pod = rest.split_once(". ").map_or(rest, |(pod, _)| pod);
        let pod = pod.strip_suffix('.').unwrap_or(pod);
        return (!pod.is_empty()).then(|| Line::ClimbOut {
            pod: pod.to_owned(),
        });
    }
    if let Some(tank) = body.strip_suffix(" is flying on doctrine; BOARD takes it back.") {
        return Some(Line::Doctrine {
            tank: tank.to_owned(),
        });
    }
    if let Some(rest) = body.strip_prefix("Connected as ") {
        let (name, rest) = rest.split_once(" (account ")?;
        let (account, rest) = rest.split_once(", ")?;
        let count = rest
            .strip_suffix(" connection).")
            .or_else(|| rest.strip_suffix(" connections)."))?;
        uint(count)?;
        return Some(Line::Connected {
            name: name.to_owned(),
            account: account.to_owned(),
        });
    }
    None
}

/// `Ada takes red slot 1.` → ("Ada", Red, 1, rest-after-the-full-stop).
fn slot_sentence<'a>(body: &'a str, verb: &str) -> Option<(&'a str, Side, u8, &'a str)> {
    for side in [Side::Red, Side::Blue] {
        let needle = format!(" {verb} {} slot ", side.as_str());
        if let Some((name, rest)) = body.split_once(needle.as_str()) {
            let (index, rest) = rest.split_once('.')?;
            let index: u8 = uint(index)?.try_into().ok()?;
            return (!name.is_empty()).then_some((name, side, index, rest));
        }
    }
    None
}

/// The centre's announcements and refusals (`help simulator`).
fn lobby(body: &str) -> Option<Line> {
    if let Some(rest) = body.strip_prefix("Simulator centre: ") {
        let (head, tail) = match rest.split_once(" — ") {
            Some((head, tail)) => (head, Some(tail.to_owned())),
            None => (rest, None),
        };
        let (state, match_id) = match head.split_once(" (match ") {
            Some((state, number)) => (state, Some(u64::from(uint(number.strip_suffix(')')?)?))),
            None => (head, None),
        };
        return state
            .bytes()
            .all(|b| b.is_ascii_lowercase())
            .then(|| Line::Board {
                state: state.to_owned(),
                match_id,
                tail,
            });
    }
    if let Some((match_id, rest)) = match_numbered(body) {
        if rest.starts_with("is boarding.") {
            return Some(Line::Boarding { match_id });
        }
        if rest == "is running. Good hunting." {
            return Some(Line::Running { match_id });
        }
        if let Some((word, tail)) = rest
            .strip_prefix("is over (")
            .and_then(|rest| rest.split_once(')'))
        {
            return (tail == ". Its units are checked in.").then(|| Line::Over {
                match_id,
                result: ResultWord::parse(word),
            });
        }
        if let Some(why) = rest.strip_prefix("is aborted: ") {
            return Some(Line::Aborted {
                match_id,
                why: why.to_owned(),
            });
        }
        if let Some(why) = rest.strip_prefix("could not start: ") {
            return Some(Line::CouldNotStart {
                match_id,
                why: why.to_owned(),
            });
        }
        if rest == "ends: every pilot has left it." {
            return Some(Line::AllLeft { match_id });
        }
        return None;
    }
    if let Some(rest) = body.strip_prefix("You climb into ") {
        let pod = rest.split_once(". ").map_or(rest, |(pod, _)| pod);
        let pod = pod.strip_suffix('.').unwrap_or(pod);
        return (!pod.is_empty()).then(|| Line::ClimbIn {
            pod: pod.to_owned(),
        });
    }
    if let Some((name, pod)) = body
        .strip_suffix('.')
        .and_then(|rest| rest.split_once(" climbs into "))
    {
        return Some(Line::ClimbsIn {
            name: name.to_owned(),
            pod: pod.to_owned(),
        });
    }
    if let Some(name) = body.strip_suffix(" calls the match off.") {
        return Some(Line::CallsOff {
            name: name.to_owned(),
        });
    }
    if let Some((name, rest)) = body.split_once(" aborts match ") {
        let (number, tail) = rest.split_once('.')?;
        return (tail == " Its units check back in when the arena answers.").then_some(
            Line::AbortsMatch {
                name: name.to_owned(),
                match_id: u64::from(uint(number)?),
            },
        );
    }
    if let Some(pod) = body
        .strip_prefix("The hatch of ")
        .and_then(|rest| rest.strip_suffix(" swings open."))
    {
        return Some(Line::HatchOpen {
            pod: pod.to_owned(),
        });
    }
    if body.starts_with("The last match is cleared from the board") {
        return Some(Line::BoardCleared);
    }
    if body.starts_with("The TacSim layer is off on this server") {
        return Some(Line::LayerOff);
    }
    if let Some(state) = body
        .strip_prefix("The simulator centre is ")
        .and_then(|rest| rest.strip_suffix("; that is not something you can do to it now."))
    {
        return Some(Line::WrongCentreState {
            state: state.to_owned(),
        });
    }
    if let Some((side, rest)) = body
        .strip_prefix("The ")
        .and_then(|rest| rest.split_once(" side "))
        && let Some(side) = Side::parse(side).filter(|s| s.as_str() == side)
    {
        if rest == "has no pilots; a match takes at least one a side." {
            return Some(Line::EmptySide { side });
        }
        if let Some(max) = rest
            .strip_prefix("is full: it takes ")
            .and_then(|rest| rest.strip_suffix(" pilot(s)."))
        {
            return Some(Line::SideFull {
                side,
                max: uint(max)?,
            });
        }
    }
    if let Some(rest) = body.strip_prefix("You already hold ") {
        let (side, rest) = rest.split_once(" slot ")?;
        let side = Side::parse(side).filter(|s| s.as_str() == side)?;
        let (index, tail) = rest.split_once('.')?;
        let index: u8 = uint(index)?.try_into().ok()?;
        return (tail == " `SIM LEAVE` gives it up.").then_some(Line::AlreadyHold { side, index });
    }
    if let Some((name, side, index, tail)) = slot_sentence(body, "takes") {
        return (tail.is_empty() || tail.starts_with(' ')).then(|| Line::Claimed {
            name: name.to_owned(),
            side,
            index,
        });
    }
    if let Some((name, side, index, tail)) = slot_sentence(body, "gives up") {
        return tail.is_empty().then(|| Line::GaveUp {
            name: name.to_owned(),
            side,
            index,
        });
    }
    None
}

/// A room heading the line renderer prints: the name, a space and the room's
/// `#id`. Only the rooms the picture follows are read.
fn room(body: &str) -> Option<Line> {
    let name = match body.rsplit_once(" #") {
        Some((name, id)) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => body,
    };
    (ROOMS.contains(&name) || pod_number(name).is_some()).then(|| Line::Room(name.to_owned()))
}

// ---------------------------------------------------------------------------
// What a line answers (cut AM.1): the lane's answer ledger reads this
// ---------------------------------------------------------------------------

/// **The kind of line the lane types**, for the answer ledger (cut AM.1):
/// the cockpit's reads and orders by verb, and every line of the lobby's
/// (`look`, the walks, `SIM …`, `BOARD`, `STARTUP`) as one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    Contacts,
    Weapons,
    Status,
    Range,
    Bearing,
    Lock,
    Heading,
    Speed,
    Fire,
    /// `VIEW BRIEF` (cut AN.2).
    View,
    Lobby,
}

/// **Which typed cockpit line a line read starts the answer to**, from the
/// line alone ([`answer_to`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnswerTo {
    /// The first line of the answer to a line of one of these kinds.
    Kinds(&'static [AskKind]),
    /// The answer to that gun's `FIRE`.
    Fire(String),
    /// `{ORDER} [{w}] refused. …`: that order's answer when one waits, else the
    /// arena's own notice.
    Refusal {
        order: String,
        weapon: Option<String>,
    },
    /// Whatever line is oldest, the lobby's included.
    Oldest,
}

/// **What a line answers, for the lane's answer ledger only** (cut AM.1). The
/// picture never consults it: a read reply's end is the picture's own rule
/// ([`super::picture::Picture::apply`]), and a read reply's opener is reported
/// there ([`super::picture::Applied::opened`]).
///
/// | Line | `answer_to` |
/// |---|---|
/// | `Steering` | `Kinds([Heading])` |
/// | `Making`, `SpeedLimits` | `Kinds([Speed])` |
/// | `Immobile` | `Kinds([Heading, Speed])` |
/// | `Locking` | `Kinds([Lock])` |
/// | `Range`, `SpotRange` | `Kinds([Range])` |
/// | `Bearing`, `SpotBearing` | `Kinds([Bearing])` |
/// | `NoLockOrName`, `NoFrameYet` | `Kinds([Range, Bearing])` |
/// | `NoLock` | `Kinds([Fire])` |
/// | `NoSuchContact` | `Kinds([Lock, Range, Bearing])` |
/// | `Firing { weapon }`, `NoSuchWeapon { weapon }` | `Fire(weapon)` |
/// | `Refused { order, weapon, .. }` | `Refusal { order, weapon }` |
/// | `ViewHeader`, `GroundRay`, `WindowDark` | `Kinds([View])` (cut AN.2) |
/// | `NoContactsInWindow`, `BriefEnd` | `None`: the brief's own lines, which the picture reads (the game's cut AP.4) |
/// | `Transient`, `PowerNotOnline`, `PowerStarting`, `DestroyedNoOrders`, `OutsideCockpit`, `NotStartedYet` | `Oldest` |
/// | everything else | `None` |
///
/// "Everything else" is the read replies' rows and openers, `Lock` (which is
/// also the arena's own `Locked` notice), every notice, board, lobby, room,
/// hint and `Other` line, and the answers to the lobby's own verbs: the
/// ledger's lobby entries follow the lobby's own state, not this table. The
/// match names every variant, so one added later must be placed.
pub fn answer_to(line: &Line) -> Option<AnswerTo> {
    use AskKind as K;
    match line {
        Line::Steering { .. } => Some(AnswerTo::Kinds(&[K::Heading])),
        Line::Making { .. } | Line::SpeedLimits { .. } => Some(AnswerTo::Kinds(&[K::Speed])),
        Line::Immobile => Some(AnswerTo::Kinds(&[K::Heading, K::Speed])),
        Line::Locking { .. } => Some(AnswerTo::Kinds(&[K::Lock])),
        Line::Range { .. } | Line::SpotRange { .. } => Some(AnswerTo::Kinds(&[K::Range])),
        Line::Bearing { .. } | Line::SpotBearing { .. } => Some(AnswerTo::Kinds(&[K::Bearing])),
        Line::NoLockOrName | Line::NoFrameYet => Some(AnswerTo::Kinds(&[K::Range, K::Bearing])),
        Line::NoLock => Some(AnswerTo::Kinds(&[K::Fire])),
        Line::NoSuchContact => Some(AnswerTo::Kinds(&[K::Lock, K::Range, K::Bearing])),
        Line::Firing { weapon } | Line::NoSuchWeapon { weapon } => {
            Some(AnswerTo::Fire(weapon.clone()))
        }
        Line::Refused { order, weapon, .. } => Some(AnswerTo::Refusal {
            order: order.clone(),
            weapon: weapon.clone(),
        }),
        // `VIEW BRIEF`'s header and ground, and `VIEW`'s dark answers (cut
        // AN.2). The brief's other lines keep their own mapping: its contact
        // sentences and spot lines are `Other`, and its own empty sentence and
        // end line (the game's cut AP.4) answer nothing — they are the brief's,
        // and the picture reads them.
        Line::ViewHeader { .. } | Line::GroundRay { .. } | Line::WindowDark => {
            Some(AnswerTo::Kinds(&[K::View]))
        }
        Line::NoContactsInWindow | Line::BriefEnd => None,
        Line::Transient(_)
        | Line::PowerNotOnline
        | Line::PowerStarting
        | Line::DestroyedNoOrders
        | Line::OutsideCockpit
        | Line::NotStartedYet => Some(AnswerTo::Oldest),
        // The read replies: the picture reports their openers.
        Line::ContactRow(_)
        | Line::NoContacts
        | Line::WeaponRow(_)
        | Line::WeaponArming { .. }
        | Line::NoWeapon
        | Line::StatusCallsign { .. }
        | Line::StatusHead(_)
        | Line::StatusPosition { .. }
        | Line::StatusNearestSpot { .. }
        | Line::StatusHeat { .. }
        | Line::StatusReceiver { .. }
        | Line::StatusEmcon { .. }
        | Line::StatusSection { .. }
        | Line::StatusFacets { .. }
        | Line::StatusUntold { .. } => None,
        // `WEAPONSTATUS`'s head and the arena's `Locked` notice: one sentence.
        Line::Lock { .. } => None,
        // Acks and readings no cockpit line the lane types asks for.
        Line::EmconReading { .. }
        | Line::StartupBegun
        | Line::PowerAlreadyOnline
        | Line::EmconAck { .. } => None,
        // The unseated, reclaim and resume sentences: the lobby's.
        Line::NotInAPod
        | Line::OutsideTeamRoom
        | Line::AlreadyAboard
        | Line::HandingOver { .. }
        | Line::LeftForGood { .. }
        | Line::BackAboard { .. }
        | Line::CatchupPower { .. }
        | Line::ClimbBack { .. }
        | Line::BackAtStation { .. }
        | Line::ClimbOut { .. }
        | Line::Resumed
        | Line::Doctrine { .. }
        | Line::Connected { .. } => None,
        // The arena's notices.
        Line::NewContact { .. }
        | Line::Identified { .. }
        | Line::Unidentified { .. }
        | Line::LockBroken { .. }
        | Line::ContactLost { .. }
        | Line::HitScored { .. }
        | Line::Missed { .. }
        | Line::Killed { .. }
        | Line::TargetSectionLost { .. }
        | Line::HitTaken { .. }
        | Line::SectionLost { .. }
        | Line::OwnDestroyed
        | Line::OwnDisabled
        | Line::PowerOnline
        | Line::Held { .. }
        | Line::ReceiverDegraded
        | Line::ReceiverClear
        | Line::EmconSwitched { .. } => None,
        // The lobby and the centre.
        Line::Board { .. }
        | Line::BoardSide { .. }
        | Line::AiFill { .. }
        | Line::BoardNext
        | Line::BoardMode(_)
        | Line::Claimed { .. }
        | Line::GaveUp { .. }
        | Line::Boarding { .. }
        | Line::ClimbIn { .. }
        | Line::ClimbsIn { .. }
        | Line::Running { .. }
        | Line::Over { .. }
        | Line::Aborted { .. }
        | Line::CouldNotStart { .. }
        | Line::AllLeft { .. }
        | Line::CallsOff { .. }
        | Line::AbortsMatch { .. }
        | Line::HatchOpen { .. }
        | Line::BoardCleared
        | Line::AlreadyHold { .. }
        | Line::SideFull { .. }
        | Line::NotLead
        | Line::EmptySide { .. }
        | Line::WrongCentreState { .. }
        | Line::NoCentre
        | Line::LayerOff
        | Line::NobodyPlaying
        | Line::OutsideCentre
        | Line::NoSlot
        | Line::NothingToAbort => None,
        // Rooms, hints and chatter.
        Line::Room(_) | Line::Hint(_) | Line::Other(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Decoder;

    fn row(
        designator: &str,
        side: Option<Side>,
        b: u32,
        r: u32,
        band: &str,
        m: u32,
        a: u32,
    ) -> Line {
        Line::ContactRow(ContactRow {
            designator: designator.into(),
            side,
            bearing_deg: b,
            rel_deg: r,
            band: band.into(),
            range_m: m,
            age_s: a,
            aspect: None,
            odds: None,
        })
    }

    /// **17b cut M1a.3**: the row's optional tail — the aspect's side word, then
    /// the main gun's odds, the dash, or nothing (an unarmed hull) — and a row
    /// with a tail the server never prints is not a row.
    #[test]
    fn m1a3_contact_rows_read_the_optional_tail() {
        let tailed = |aspect: &str, odds: Option<(&str, u32)>| {
            let Line::ContactRow(mut row) = row("BA", Some(Side::Blue), 87, 32, "far", 450, 2)
            else {
                unreachable!("the helper builds a contact row")
            };
            row.aspect = Some(aspect.into());
            row.odds = Some(odds.map(|(weapon, percent)| (weapon.into(), percent)));
            Line::ContactRow(row)
        };
        assert_eq!(
            classify("BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon 62%"),
            tailed("side", Some(("cannon", 62)))
        );
        assert_eq!(
            classify("BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  rear  -"),
            tailed("rear", None)
        );
        assert_eq!(
            classify("BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  front"),
            tailed("front", None)
        );
        assert_eq!(
            classify("BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  machine_gun 0%"),
            tailed("side", Some(("machine_gun", 0)))
        );
        for bad in [
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  left",
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon",
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon 62",
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  Cannon 62%",
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  -  extra",
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon 62%  more",
        ] {
            assert!(
                !matches!(classify(bad), Line::ContactRow(_)),
                "{bad} is not a row"
            );
        }
    }

    #[test]
    fn contact_rows_read_every_designator_width() {
        assert_eq!(
            classify("BA  blue  bearing 087° (rel 032°)  far  450 m  2 s"),
            row("BA", Some(Side::Blue), 87, 32, "far", 450, 2)
        );
        assert_eq!(
            classify("X2  unknown  bearing 315° (rel 270°)  effective  250 m  3 s"),
            row("X2", None, 315, 270, "effective", 250, 3)
        );
        assert_eq!(
            classify("RAA red  bearing 000° (rel 000°)  close  50 m  0 s"),
            row("RAA", Some(Side::Red), 0, 0, "close", 50, 0)
        );
        assert_eq!(
            classify("BAAAblue  bearing 359° (rel 001°)  beyond  900 m  12 s"),
            row("BAAA", Some(Side::Blue), 359, 1, "beyond", 900, 12)
        );
        assert_eq!(
            classify("X12 unknown  bearing 045° (rel 045°)  far  600 m  1 s"),
            row("X12", None, 45, 45, "far", 600, 1)
        );
        // A row with a garbage field is not a row.
        for bad in [
            "BA  blue  bearing 0x7° (rel 032°)  far  450 m  2 s",
            "BA  green  bearing 087° (rel 032°)  far  450 m  2 s",
            "BA  blue  bearing 087° (rel 032°)  near  450 m  2 s",
            "ba  blue  bearing 087° (rel 032°)  far  450 m  2 s",
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  extra",
            "BA  blue  bearing 087° (rel 032°)  far  -450 m  2 s",
        ] {
            assert_eq!(classify(bad), Line::Other(bad.into()), "{bad}");
        }
    }

    #[test]
    fn locks_and_weapon_rows() {
        assert_eq!(
            classify("Locked on BA, 62% to hit."),
            Line::Lock {
                designator: "BA".into(),
                friendly: false,
                percent: Some(62)
            }
        );
        assert_eq!(
            classify("Locked on RB (FRIENDLY), 62% to hit."),
            Line::Lock {
                designator: "RB".into(),
                friendly: true,
                percent: Some(62)
            }
        );
        assert_eq!(
            classify("Locked on X2."),
            Line::Lock {
                designator: "X2".into(),
                friendly: false,
                percent: None
            }
        );
        assert_eq!(
            classify("cannon  ready  40 of 40 round(s)  band 1 of 2  46% to hit"),
            Line::WeaponRow(WeaponRow {
                id: "cannon".into(),
                short: None,
                ready: true,
                cycling_s: None,
                rounds: 40,
                rounds_max: 40,
                band: Some((1, 2)),
                percent: Some(46),
            })
        );
        assert_eq!(
            classify("machine_gun (mg)  cycling (4 s)  380 of 500 round(s)  21% to hit"),
            Line::WeaponRow(WeaponRow {
                id: "machine_gun".into(),
                short: Some("mg".into()),
                ready: false,
                cycling_s: Some(4),
                rounds: 380,
                rounds_max: 500,
                band: None,
                percent: Some(21),
            })
        );
        assert_eq!(
            classify("cannon  cycling (4 s)  39 of 40 round(s)"),
            Line::WeaponRow(WeaponRow {
                id: "cannon".into(),
                short: None,
                ready: false,
                cycling_s: Some(4),
                rounds: 39,
                rounds_max: 40,
                band: None,
                percent: None,
            })
        );
        assert_eq!(classify("This unit carries no weapon."), Line::NoWeapon);
        assert_eq!(classify("No contacts."), Line::NoContacts);
    }

    /// **The brief's own two sentences** (the game's cut AP.4, 16c `N20`):
    /// `No contacts in the window.` and `End of brief.` read as their own
    /// lines, never as `CONTACTS`' `No contacts.`, and answer nothing.
    #[test]
    fn ap4_the_briefs_own_empty_sentence_and_end_line() {
        assert_eq!(
            classify("No contacts in the window."),
            Line::NoContactsInWindow
        );
        assert_eq!(classify("End of brief."), Line::BriefEnd);
        assert_eq!(classify("No contacts."), Line::NoContacts);
        assert_eq!(answer_to(&Line::NoContactsInWindow), None);
        assert_eq!(answer_to(&Line::BriefEnd), None);
        for text in [
            "End of brief",
            "No contacts in the window",
            "End of the brief.",
        ] {
            assert!(
                !matches!(classify(text), Line::BriefEnd | Line::NoContactsInWindow),
                "{text}"
            );
        }
    }

    /// **A gun the tank does not carry** (cut AL.1): the sentence with the
    /// weapon in backticks, and nothing like it.
    #[test]
    fn the_no_such_weapon_answer() {
        for weapon in ["cannon", "machine_gun"] {
            assert_eq!(
                classify(&format!("That unit carries no weapon called `{weapon}`.")),
                Line::NoSuchWeapon {
                    weapon: weapon.into()
                }
            );
        }
        for near in [
            "That unit carries no weapon called cannon.",
            "That unit carries no weapon called `cannon`",
            "That unit carries no weapon called `Cannon`.",
            "That unit carries no weapon called ``.",
        ] {
            assert!(matches!(classify(near), Line::Other(_)), "{near}");
        }
        assert_eq!(classify("This unit carries no weapon."), Line::NoWeapon);
    }

    #[test]
    fn the_status_block() {
        assert_eq!(
            classify("Callsign RA."),
            Line::StatusCallsign {
                callsign: "RA".into()
            }
        );
        assert_eq!(
            classify(
                "Red Tank 1 [biped_mk1]: power online, -27 kph, heading 090°, HELD, DESTROYED"
            ),
            Line::StatusHead(StatusHead {
                name: "Red Tank 1".into(),
                chassis: Some("biped_mk1".into()),
                power: Power::Online,
                kph: -27,
                heading_deg: 90,
                held: true,
                destroyed: true,
            })
        );
        // `STATUS` prints the heading unpadded (the live centre reads `heading
        // 90°`); `CONTACTS` and `BEARING` pad theirs to three digits.
        assert!(matches!(
            classify("Red Tank 1: power online, 0 kph, heading 90°"),
            Line::StatusHead(StatusHead {
                heading_deg: 90,
                ..
            })
        ));
        assert_eq!(
            classify("Red Tank 1: power starting, 0 kph, heading 270°"),
            Line::StatusHead(StatusHead {
                name: "Red Tank 1".into(),
                chassis: None,
                power: Power::Starting,
                kph: 0,
                heading_deg: 270,
                held: false,
                destroyed: false,
            })
        );
        assert_eq!(
            classify("  at -12 m east, 975 m north; scope holds an unidentified contact"),
            Line::StatusPosition {
                x_m: -12,
                y_m: 975,
                scope: Scope::Unidentified
            }
        );
        assert_eq!(
            classify("  at 1000 m east, -5 m north; scope clear"),
            Line::StatusPosition {
                x_m: 1000,
                y_m: -5,
                scope: Scope::Clear
            }
        );
        assert_eq!(
            classify("  nearest spot: The Ford (A), 120 m at 045°"),
            Line::StatusNearestSpot {
                name: "The Ford".into(),
                letter: 'A',
                range_m: 120,
                bearing_deg: 45
            }
        );
        assert_eq!(
            classify("  heat 100 of 100, OVERHEATED"),
            Line::StatusHeat {
                heat: 100,
                max: 100,
                overheated: true
            }
        );
        assert_eq!(
            classify("  receiver degraded: 15 of 40 through cover, 60% reach"),
            Line::StatusReceiver {
                budget: 15,
                budget_max: 40,
                reach_percent: 60
            }
        );
        assert_eq!(
            classify("  emcon on: the pod radiates"),
            Line::StatusEmcon { on: true }
        );
        assert_eq!(
            classify("  emcon off: the pod is silent"),
            Line::StatusEmcon { on: false }
        );
        assert_eq!(
            classify("  hull  intact  internals 60/60"),
            Line::StatusSection {
                id: "hull".into(),
                condition: "intact".into(),
                internals: Some((60, 60))
            }
        );
        assert_eq!(
            classify("  gun_mount  destroyed"),
            Line::StatusSection {
                id: "gun_mount".into(),
                condition: "destroyed".into(),
                internals: None
            }
        );
        assert_eq!(
            classify("    front 400/400  left 250/250  right 0/250"),
            Line::StatusFacets {
                facets: vec![
                    ("front".into(), 400, 400),
                    ("left".into(), 250, 250),
                    ("right".into(), 0, 250)
                ]
            }
        );
        assert_eq!(
            classify("Red Tank 1: power off. The panel has told you nothing yet."),
            Line::StatusUntold {
                name: "Red Tank 1".into()
            }
        );
    }

    #[test]
    fn range_bearing_and_spots() {
        assert_eq!(
            classify("Range to BA: 440 m."),
            Line::Range {
                designator: "BA".into(),
                range_m: 440,
                plotted_s: None
            }
        );
        assert_eq!(
            classify("Range to BA: 450 m, plotted 4 s ago."),
            Line::Range {
                designator: "BA".into(),
                range_m: 450,
                plotted_s: Some(4)
            }
        );
        assert_eq!(
            classify("Bearing to BA: 087° (rel 357°)."),
            Line::Bearing {
                designator: "BA".into(),
                bearing_deg: 87,
                rel_deg: 357,
                plotted_s: None
            }
        );
        assert_eq!(
            classify("Bearing to X3: 087° (rel 357°), plotted 6 s ago."),
            Line::Bearing {
                designator: "X3".into(),
                bearing_deg: 87,
                rel_deg: 357,
                plotted_s: Some(6)
            }
        );
        assert_eq!(
            classify("Range to The Ford (A): 430 m."),
            Line::SpotRange {
                name: "The Ford".into(),
                letter: 'A',
                range_m: 430
            }
        );
        assert_eq!(
            classify("Bearing to The Crossroads (B): 087° (rel 357°)."),
            Line::SpotBearing {
                name: "The Crossroads".into(),
                letter: 'B',
                bearing_deg: 87,
                rel_deg: 357
            }
        );
        assert_eq!(classify("No lock: LOCK a contact first."), Line::NoLock);
        assert_eq!(
            classify("No lock: LOCK a contact first, or name one."),
            Line::NoLockOrName
        );
        assert_eq!(classify("No such contact."), Line::NoSuchContact);
        assert_eq!(
            classify("That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54."),
            Line::SpeedLimits {
                ahead_kph: 54,
                astern_kph: 27
            }
        );
        assert_eq!(
            classify("Your tank is destroyed and takes no orders. EXIT leaves the pod."),
            Line::DestroyedNoOrders
        );
    }

    #[test]
    fn acks() {
        assert_eq!(
            classify("Startup sequence begun; power comes online shortly."),
            Line::StartupBegun
        );
        assert_eq!(
            classify("Power is already online."),
            Line::PowerAlreadyOnline
        );
        assert_eq!(
            classify("Power is still coming up; hold on."),
            Line::PowerStarting
        );
        assert_eq!(
            classify("Power not online: STARTUP first."),
            Line::PowerNotOnline
        );
        assert_eq!(classify("Steering to 90°."), Line::Steering { deg: 90 });
        assert_eq!(classify("Making -27 kph."), Line::Making { kph: -27 });
        assert_eq!(
            classify("Locking RB (FRIENDLY)."),
            Line::Locking {
                designator: "RB".into(),
                friendly: true
            }
        );
        assert_eq!(
            classify("Locking X2."),
            Line::Locking {
                designator: "X2".into(),
                friendly: false
            }
        );
        assert_eq!(
            classify("Firing machine_gun."),
            Line::Firing {
                weapon: "machine_gun".into()
            }
        );
    }

    #[test]
    fn unseated_reclaim_resume_and_login() {
        assert_eq!(
            classify("You are not at the station of a deployed unit."),
            Line::OutsideCockpit
        );
        assert_eq!(classify("You are not in a pod."), Line::NotInAPod);
        assert_eq!(
            classify("Boarding is through one of the simulator's team rooms."),
            Line::OutsideTeamRoom
        );
        assert_eq!(
            classify(
                "The panel is dark until the match starts; the simulator is still waiting for the other pods."
            ),
            Line::NotStartedYet
        );
        assert_eq!(
            classify("You are already in your pod."),
            Line::AlreadyAboard
        );
        assert_eq!(
            classify("Red Tank 1 is still being handed over; try again in 3 second(s)."),
            Line::HandingOver {
                unit: "Red Tank 1".into(),
                secs: 3
            }
        );
        assert_eq!(
            classify("You left Red Tank 1 for the rest of the match; it cannot be taken back."),
            Line::LeftForGood {
                unit: "Red Tank 1".into()
            }
        );
        assert_eq!(
            classify("Back aboard RA after 2 m 14 s away."),
            Line::BackAboard {
                callsign: "RA".into(),
                away_ms: 134_000
            }
        );
        assert_eq!(
            classify("Back aboard BC after 1 h 0 m 5 s away."),
            Line::BackAboard {
                callsign: "BC".into(),
                away_ms: 3_605_000
            }
        );
        assert_eq!(
            classify("  Power online."),
            Line::CatchupPower {
                power: Power::Online
            }
        );
        assert_eq!(
            classify("  The machine was withdrawing toward its entry zone under no order."),
            Line::Other("The machine was withdrawing toward its entry zone under no order.".into())
        );
        assert_eq!(
            classify(
                "The server restarted and the match resumed from its last save; what happened after that save did not count."
            ),
            Line::Resumed
        );
        assert_eq!(
            classify("Red Tank 1 is flying on doctrine; BOARD takes it back."),
            Line::Doctrine {
                tank: "Red Tank 1".into()
            }
        );
        assert_eq!(
            classify("You climb back into Red Pod 1: Red Tank 1 is yours again where it stands."),
            Line::ClimbBack {
                pod: "Red Pod 1".into(),
                unit: "Red Tank 1".into()
            }
        );
        assert_eq!(
            classify("You climb out of Red Pod 1."),
            Line::ClimbOut {
                pod: "Red Pod 1".into()
            }
        );
        assert_eq!(
            classify(
                "You climb out of Red Pod 1. Red Tank 1 is on its own for the rest of the match; you cannot take it back."
            ),
            Line::ClimbOut {
                pod: "Red Pod 1".into()
            }
        );
        assert_eq!(
            classify("Connected as Pilot (account pilot, 1 connection)."),
            Line::Connected {
                name: "Pilot".into(),
                account: "pilot".into()
            }
        );
        assert_eq!(
            classify("Connected as Ada Lovelace (account ada, 2 connections)."),
            Line::Connected {
                name: "Ada Lovelace".into(),
                account: "ada".into()
            }
        );
    }

    #[test]
    fn contact_and_fire_notices() {
        assert_eq!(
            classify("New contact X2: bearing 087°, effective."),
            Line::NewContact {
                designator: "X2".into(),
                bearing_deg: Some(87),
                band: Some("effective".into())
            }
        );
        assert_eq!(
            classify("New contact RB."),
            Line::NewContact {
                designator: "RB".into(),
                bearing_deg: None,
                band: None
            }
        );
        assert_eq!(
            classify("Contact X2 identified: BA, blue."),
            Line::Identified {
                was: "X2".into(),
                designator: "BA".into(),
                side: Side::Blue
            }
        );
        assert_eq!(
            classify("BA is unidentified again: X2."),
            Line::Unidentified {
                was: "BA".into(),
                designator: "X2".into()
            }
        );
        assert_eq!(
            classify("Lock on BA broken: contact gone."),
            Line::LockBroken {
                designator: "BA".into()
            }
        );
        assert_eq!(
            classify("Contact BA lost."),
            Line::ContactLost {
                designator: "BA".into()
            }
        );
        assert_eq!(
            classify("Hit on BA (FRIENDLY): turret, already destroyed."),
            Line::HitScored {
                designator: Some("BA".into()),
                friendly: true,
                section: Some("turret".into()),
                already_destroyed: true
            }
        );
        assert_eq!(
            classify("Hit on X2."),
            Line::HitScored {
                designator: Some("X2".into()),
                friendly: false,
                section: None,
                already_destroyed: false
            }
        );
        assert_eq!(
            classify("Hit."),
            Line::HitScored {
                designator: None,
                friendly: false,
                section: None,
                already_destroyed: false
            }
        );
        assert_eq!(
            classify("Miss on BA: the target moved."),
            Line::Missed {
                designator: Some("BA".into()),
                words: Some("the target moved".into())
            }
        );
        assert_eq!(
            classify("Miss: the round went wide."),
            Line::Missed {
                designator: None,
                words: Some("the round went wide".into())
            }
        );
        assert_eq!(
            classify("Miss."),
            Line::Missed {
                designator: None,
                words: None
            }
        );
        assert_eq!(
            classify("BA destroyed."),
            Line::Killed {
                designator: Some("BA".into())
            }
        );
        assert_eq!(
            classify("Target destroyed."),
            Line::Killed { designator: None }
        );
        assert_eq!(
            classify("BA's turret is destroyed."),
            Line::TargetSectionLost {
                designator: "BA".into(),
                section: "turret".into()
            }
        );
    }

    #[test]
    fn the_victims_lines() {
        assert_eq!(
            classify("Hit on the hull's front facet: 280 of 400 armour left."),
            Line::HitTaken {
                section: "hull".into(),
                through: None,
                facet: Some("front".into()),
                armour: Some((280, 400)),
                internals: None,
                already_destroyed: false
            }
        );
        assert_eq!(
            classify("Hit on the hull's front facet: armour gone, 140 of 200 inside."),
            Line::HitTaken {
                section: "hull".into(),
                through: None,
                facet: Some("front".into()),
                armour: None,
                internals: Some((140, 200)),
                already_destroyed: false
            }
        );
        assert_eq!(
            classify("Hit on the engine, through the hull's rear facet: 90 of 150 armour left."),
            Line::HitTaken {
                section: "engine".into(),
                through: Some("hull".into()),
                facet: Some("rear".into()),
                armour: Some((90, 150)),
                internals: None,
                already_destroyed: false
            }
        );
        assert_eq!(
            classify("Hit on the left_track, which is already destroyed."),
            Line::HitTaken {
                section: "left_track".into(),
                through: None,
                facet: None,
                armour: None,
                internals: None,
                already_destroyed: true
            }
        );
        assert_eq!(
            classify("The left_track is destroyed: the tank is slowed."),
            Line::SectionLost {
                section: "left_track".into(),
                cost: Some("the tank is slowed".into())
            }
        );
        assert_eq!(
            classify("The turret is destroyed."),
            Line::SectionLost {
                section: "turret".into(),
                cost: None
            }
        );
        assert_eq!(
            classify(
                "Your tank is destroyed. The match runs on without you: wait it out, EXIT to the team room, or SIM ABORT to call it off."
            ),
            Line::OwnDestroyed
        );
        assert_eq!(
            classify(
                "Your tank can neither move nor shoot. The match runs on: wait it out, EXIT to the team room, or SIM ABORT to call it off."
            ),
            Line::OwnDisabled
        );
        // Wrapped at 120 columns, as a line client reads it: the opening is
        // enough, and the tail is nothing.
        assert_eq!(
            classify(
                "Your tank can neither move nor shoot. The match runs on: wait it out, EXIT to the team room, or SIM ABORT to call it"
            ),
            Line::OwnDisabled
        );
        assert_eq!(classify("off."), Line::Other("off.".into()));
    }

    #[test]
    fn held_receiver_emcon_power_and_refusals() {
        for (line, cause) in [
            (
                "Held: the ground ahead will not take the tank.",
                HeldCause::Impassable,
            ),
            ("Held: something is in the way.", HeldCause::Occupied),
            ("Held: this is the edge of the arena.", HeldCause::Edge),
            (
                "Held: the slope ahead is too steep for the tank.",
                HeldCause::TooSteep,
            ),
        ] {
            assert_eq!(classify(line), Line::Held { cause }, "{line}");
        }
        assert_eq!(classify("Power online."), Line::PowerOnline);
        assert_eq!(
            classify("Receiver degraded: something is jamming the sight."),
            Line::ReceiverDegraded
        );
        assert_eq!(classify("Receiver clear."), Line::ReceiverClear);
        assert_eq!(
            classify(
                "EMCON on: the pod radiates; every sight inside 60 m is jammed, your own among them."
            ),
            Line::EmconSwitched {
                on: true,
                radius_m: Some(60)
            }
        );
        assert_eq!(
            classify("EMCON off: the pod is silent; your own sight clears, and nothing hides you."),
            Line::EmconSwitched {
                on: false,
                radius_m: None
            }
        );
        assert_eq!(
            classify("EMCON on: the pod radiates."),
            Line::EmconReading { on: true }
        );
        assert_eq!(
            classify("FIRE cannon refused. Still cycling: 4 s to go."),
            Line::Refused {
                order: "FIRE".into(),
                weapon: Some("cannon".into()),
                said: "Still cycling: 4 s to go.".into(),
                cycling_s: Some(4)
            }
        );
        assert_eq!(
            classify("HEADING refused. Your tank cannot move."),
            Line::Refused {
                order: "HEADING".into(),
                weapon: None,
                said: "Your tank cannot move.".into(),
                cycling_s: None
            }
        );
        assert_eq!(classify("Your tank cannot move."), Line::Immobile);
    }

    #[test]
    fn the_lobby_and_the_centre() {
        assert_eq!(
            classify("Simulator centre: idle"),
            Line::Board {
                state: "idle".into(),
                match_id: None,
                tail: None
            }
        );
        assert_eq!(
            classify("Simulator centre: ended (match 8) — blue wins: red conceded"),
            Line::Board {
                state: "ended".into(),
                match_id: Some(8),
                tail: Some("blue wins: red conceded".into())
            }
        );
        assert_eq!(
            classify("  red: nobody"),
            Line::BoardSide {
                side: Side::Red,
                pilots: vec![]
            }
        );
        assert_eq!(
            classify(
                "  blue: 1 (BA): Ada [biped_mk1], pod Blue Pod 1, seated; 2 (BB): Bo, flown by doctrine; 3 (BC): Cy"
            ),
            Line::BoardSide {
                side: Side::Blue,
                pilots: vec![
                    BoardPilot {
                        index: 1,
                        callsign: "BA".into(),
                        name: "Ada".into(),
                        unit_type: Some("biped_mk1".into()),
                        mind: false,
                        pod: Some("Blue Pod 1".into()),
                        standing: Some(Standing::Seated)
                    },
                    BoardPilot {
                        index: 2,
                        callsign: "BB".into(),
                        name: "Bo".into(),
                        unit_type: None,
                        mind: false,
                        pod: None,
                        standing: Some(Standing::Doctrine)
                    },
                    BoardPilot {
                        index: 3,
                        callsign: "BC".into(),
                        name: "Cy".into(),
                        unit_type: None,
                        mind: false,
                        pod: None,
                        standing: None
                    },
                ]
            }
        );
        // The game's 17b cut M5.1: the selection fog's mark after the
        // bracket (or the name), and the fill's row, which is not a side row.
        assert_eq!(
            classify("  red: 1 (RA): Ada [biped_mk1], a mind; 2 (RB): Bo, a mind; 3 (RC): Cy"),
            Line::BoardSide {
                side: Side::Red,
                pilots: vec![
                    BoardPilot {
                        index: 1,
                        callsign: "RA".into(),
                        name: "Ada".into(),
                        unit_type: Some("biped_mk1".into()),
                        mind: true,
                        pod: None,
                        standing: None
                    },
                    BoardPilot {
                        index: 2,
                        callsign: "RB".into(),
                        name: "Bo".into(),
                        unit_type: None,
                        mind: true,
                        pod: None,
                        standing: None
                    },
                    BoardPilot {
                        index: 3,
                        callsign: "RC".into(),
                        name: "Cy".into(),
                        unit_type: None,
                        mind: false,
                        pod: None,
                        standing: None
                    },
                ]
            }
        );
        assert_eq!(
            classify("  red fill: a mind"),
            Line::Other("red fill: a mind".into())
        );
        assert_eq!(
            classify("  AI fill: each side fields 2; machines take the unclaimed pods."),
            Line::AiFill { each: 2 }
        );
        assert_eq!(
            classify("  `SIM TEAM red` or `SIM TEAM blue` opens the next match."),
            Line::BoardNext
        );
        assert_eq!(
            classify(
                "Ada takes red slot 1. `SIM START` begins the match; the machines take the pods nobody claims."
            ),
            Line::Claimed {
                name: "Ada".into(),
                side: Side::Red,
                index: 1
            }
        );
        assert_eq!(
            classify("Ada gives up blue slot 2."),
            Line::GaveUp {
                name: "Ada".into(),
                side: Side::Blue,
                index: 2
            }
        );
        assert_eq!(
            classify("Match 8 is boarding. Every pilot: BOARD your pod. The match starts when"),
            Line::Boarding { match_id: 8 }
        );
        assert_eq!(
            classify(
                "You climb into Red Pod 1. The panel wakes when every pilot is seated, or in 30 second(s)."
            ),
            Line::ClimbIn {
                pod: "Red Pod 1".into()
            }
        );
        assert_eq!(
            classify("Ada climbs into Blue Pod 2."),
            Line::ClimbsIn {
                name: "Ada".into(),
                pod: "Blue Pod 2".into()
            }
        );
        assert_eq!(
            classify("Match 8 is running. Good hunting."),
            Line::Running { match_id: 8 }
        );
        for (word, result) in [
            ("red", ResultWord::Red),
            ("blue", ResultWord::Blue),
            ("draw", ResultWord::Draw),
            ("abort", ResultWord::Abort),
            ("timeout", ResultWord::Timeout),
            ("void", ResultWord::Void),
            ("sideways", ResultWord::Other("sideways".into())),
        ] {
            assert_eq!(
                classify(&format!(
                    "Match 12 is over ({word}). Its units are checked in."
                )),
                Line::Over {
                    match_id: 12,
                    result
                }
            );
        }
        assert_eq!(
            classify("Match 3 is aborted: the red side has nobody left to fly it."),
            Line::Aborted {
                match_id: 3,
                why: "the red side has nobody left to fly it.".into()
            }
        );
        assert_eq!(
            classify("Match 3 could not start: nope. The pods are free again."),
            Line::CouldNotStart {
                match_id: 3,
                why: "nope. The pods are free again.".into()
            }
        );
        assert_eq!(
            classify("Match 3 ends: every pilot has left it."),
            Line::AllLeft { match_id: 3 }
        );
        assert_eq!(
            classify("Ada calls the match off."),
            Line::CallsOff { name: "Ada".into() }
        );
        assert_eq!(
            classify("Ada aborts match 3. Its units check back in when the arena answers."),
            Line::AbortsMatch {
                name: "Ada".into(),
                match_id: 3
            }
        );
        assert_eq!(
            classify("The hatch of Red Pod 1 swings open."),
            Line::HatchOpen {
                pod: "Red Pod 1".into()
            }
        );
        assert_eq!(
            classify("The last match is cleared from the board and the tanks are back at muster."),
            Line::BoardCleared
        );
        assert_eq!(
            classify("You already hold red slot 1. `SIM LEAVE` gives it up."),
            Line::AlreadyHold {
                side: Side::Red,
                index: 1
            }
        );
        assert_eq!(
            classify("The blue side is full: it takes 6 pilot(s)."),
            Line::SideFull {
                side: Side::Blue,
                max: 6
            }
        );
        assert_eq!(
            classify("Only a side's lead or staff can start the match."),
            Line::NotLead
        );
        assert_eq!(
            classify("The blue side has no pilots; a match takes at least one a side."),
            Line::EmptySide { side: Side::Blue }
        );
        assert_eq!(
            classify(
                "The simulator centre is running; that is not something you can do to it now."
            ),
            Line::WrongCentreState {
                state: "running".into()
            }
        );
        assert_eq!(
            classify("This server has no simulator centre."),
            Line::NoCentre
        );
        assert_eq!(
            classify(
                "The TacSim layer is off on this server, so the simulator has no arena and no tanks; staff must turn it on and restart."
            ),
            Line::LayerOff
        );
        assert_eq!(
            classify("Nobody is in the simulator; a match takes at least one pilot."),
            Line::NobodyPlaying
        );
        assert_eq!(
            classify("The simulator is through the lobby."),
            Line::OutsideCentre
        );
    }

    #[test]
    fn rooms_hints_and_transients() {
        for room in [
            "Welcome Hall",
            "Simulator Lobby",
            "Red Room",
            "Blue Room",
            "Red Pod 6",
            "Blue Pod 1",
        ] {
            assert_eq!(classify(room), Line::Room(room.into()));
            assert_eq!(classify(&format!("{room} #57")), Line::Room(room.into()));
        }
        assert_eq!(classify("Red Pod 7"), Line::Other("Red Pod 7".into()));
        assert_eq!(
            classify("Market Square #0"),
            Line::Other("Market Square #0".into())
        );
        assert_eq!(
            classify("Hint: the panel is yours now: STARTUP brings your power up."),
            Line::Hint("the panel is yours now: STARTUP brings your power up.".into())
        );
        for line in [
            "The world is committing another action; please retry in a moment.",
            "The world was busy too long for that line; please retry in a moment.",
            "The world is still starting; please try again shortly.",
            "That cockpit will not wake for you: control state unknown",
            "The arena is busy; try that again in a moment.",
        ] {
            assert_eq!(classify(line), Line::Transient(line.into()), "{line}");
        }
        // A hint is read before the transients, whatever its words.
        assert_eq!(
            classify("Hint: if it says please retry in a moment."),
            Line::Hint("if it says please retry in a moment.".into())
        );
        // The seat's refusal is read at the end of a line only.
        let chatter = "Ada says, \"control state unknown, they said\"";
        assert_eq!(classify(chatter), Line::Other(chatter.into()));
    }

    /// **Another player's padded speech never reads as a game sentence.** The
    /// renderer wraps a say, pose, page or whisper with a hanging indent of the
    /// speaker's name plus one (three to twenty columns for a character's name),
    /// so a padded page can put any sentence at the start of a continuation line.
    /// No indent but the game's own two- and four-space rows reads as anything
    /// but `Other`, and those rows read only as their own forms.
    #[test]
    fn indented_continuations_are_never_game_sentences() {
        let sentences = [
            "Match 1 is over (red). Its units are checked in.",
            "Match 1 is running. Good hunting.",
            "Match 1 is aborted: nobody boarded.",
            "New contact X9: bearing 090°, far.",
            "Contact X2 identified: BA, blue.",
            "Your tank is destroyed. The match runs on without you: wait it out,",
            "Your tank can neither move nor shoot. The match runs on: wait it",
            "BA destroyed.",
            "Hit on BA: turret.",
            "Hit on the hull's front facet: 280 of 400 armour left.",
            "The left_track is destroyed: the tank cannot move.",
            "FIRE cannon refused. Still cycling: 4 s to go.",
            "Firing cannon.",
            "Locked on BA, 62% to hit.",
            // (`Power online.` two spaces in is the catch-up block's own row;
            // the picture takes it only inside that block.)
            "Held: something is in the way.",
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s",
            "Back aboard RA after 2 m 14 s away.",
            "Red Tank 1: power online, 0 kph, heading 000°, DESTROYED",
            "Callsign RA.",
            "No contacts.",
            "Red Room",
            "Hint: STARTUP brings your power up.",
            "The world is committing another action; please retry in a moment.",
            "That cockpit will not wake for you: control state unknown",
        ];
        for sentence in sentences {
            for indent in [1, 2, 3, 4, 5, 8, 20] {
                let line = format!("{}{sentence}", " ".repeat(indent));
                assert_eq!(
                    classify(&line),
                    Line::Other(sentence.to_owned()),
                    "{line:?}"
                );
            }
        }
        // The game's own indented rows still read, at their own indent only.
        assert!(matches!(
            classify("  at 10 m east, 20 m north; scope clear"),
            Line::StatusPosition { .. }
        ));
        assert!(matches!(
            classify("    front 400/400  left 250/250"),
            Line::StatusFacets { .. }
        ));
        assert!(matches!(
            classify("  Power online."),
            Line::CatchupPower {
                power: Power::Online
            }
        ));
        assert_eq!(
            classify("   at 10 m east, 20 m north; scope clear"),
            Line::Other("at 10 m east, 20 m north; scope clear".into())
        );
        assert_eq!(
            classify("  front 400/400"),
            Line::Other("front 400/400".into())
        );
    }

    /// Every example line of `docs/help/cockpit.md` and `docs/help/simulator.md`
    /// that is a real game line classifies as what it says. Pasted as literals:
    /// the help is the only description the pilot is written from.
    #[test]
    fn the_help_examples_read_as_what_they_say() {
        type Holds = fn(&Line) -> bool;
        let cases: Vec<(&str, Holds)> = vec![
            (
                "That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.",
                |l| {
                    matches!(
                        l,
                        Line::SpeedLimits {
                            ahead_kph: 54,
                            astern_kph: 27
                        }
                    )
                },
            ),
            ("Power online.", |l| *l == Line::PowerOnline),
            ("Power is still coming up; hold on.", |l| {
                *l == Line::PowerStarting
            }),
            ("Held: the ground ahead will not take the tank.", |l| {
                matches!(
                    l,
                    Line::Held {
                        cause: HeldCause::Impassable
                    }
                )
            }),
            ("Held: something is in the way.", |l| {
                matches!(
                    l,
                    Line::Held {
                        cause: HeldCause::Occupied
                    }
                )
            }),
            ("Held: this is the edge of the arena.", |l| {
                matches!(
                    l,
                    Line::Held {
                        cause: HeldCause::Edge
                    }
                )
            }),
            ("Held: the slope ahead is too steep for the tank.", |l| {
                matches!(
                    l,
                    Line::Held {
                        cause: HeldCause::TooSteep
                    }
                )
            }),
            ("Contact X2 identified: BA, blue.", |l| {
                matches!(
                    l,
                    Line::Identified {
                        side: Side::Blue,
                        ..
                    }
                )
            }),
            ("No such contact.", |l| *l == Line::NoSuchContact),
            (
                "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s",
                |l| matches!(l, Line::ContactRow(row) if row.range_m == 450 && row.age_s == 2),
            ),
            ("Range to BA: 440 m.", |l| {
                matches!(
                    l,
                    Line::Range {
                        range_m: 440,
                        plotted_s: None,
                        ..
                    }
                )
            }),
            ("Bearing to BA: 087° (rel 357°).", |l| {
                matches!(
                    l,
                    Line::Bearing {
                        bearing_deg: 87,
                        rel_deg: 357,
                        ..
                    }
                )
            }),
            ("Range to The Ford (A): 430 m.", |l| {
                matches!(
                    l,
                    Line::SpotRange {
                        letter: 'A',
                        range_m: 430,
                        ..
                    }
                )
            }),
            ("Range to BA: 450 m, plotted 4 s ago.", |l| {
                matches!(
                    l,
                    Line::Range {
                        plotted_s: Some(4),
                        ..
                    }
                )
            }),
            ("No lock: LOCK a contact first, or name one.", |l| {
                *l == Line::NoLockOrName
            }),
            ("BA is unidentified again: X2.", |l| {
                matches!(l, Line::Unidentified { .. })
            }),
            (
                "  receiver degraded: 15 of 40 through cover, 60% reach",
                |l| {
                    matches!(
                        l,
                        Line::StatusReceiver {
                            reach_percent: 60,
                            ..
                        }
                    )
                },
            ),
            (
                "EMCON off: the pod is silent; your own sight clears, and nothing hides you.",
                |l| matches!(l, Line::EmconSwitched { on: false, .. }),
            ),
            (
                "EMCON on: the pod radiates; every sight inside 60 m is jammed, your own among them.",
                |l| {
                    matches!(
                        l,
                        Line::EmconSwitched {
                            on: true,
                            radius_m: Some(60)
                        }
                    )
                },
            ),
            ("  emcon on: the pod radiates", |l| {
                *l == Line::StatusEmcon { on: true }
            }),
            ("  emcon off: the pod is silent", |l| {
                *l == Line::StatusEmcon { on: false }
            }),
            ("The panel has told you nothing yet.", |l| {
                *l == Line::NoFrameYet
            }),
            ("New contact X2: bearing 087°, effective.", |l| {
                matches!(
                    l,
                    Line::NewContact {
                        bearing_deg: Some(87),
                        ..
                    }
                )
            }),
            ("Contact BA lost.", |l| {
                matches!(l, Line::ContactLost { .. })
            }),
            ("Locked on BA, 62% to hit.", |l| {
                matches!(
                    l,
                    Line::Lock {
                        percent: Some(62),
                        friendly: false,
                        ..
                    }
                )
            }),
            ("Locked on RB (FRIENDLY), 62% to hit.", |l| {
                matches!(l, Line::Lock { friendly: true, .. })
            }),
            (
                "Hit on BA: turret.",
                |l| matches!(l, Line::HitScored { section: Some(s), .. } if s == "turret"),
            ),
            ("Miss on BA: the target moved.", |l| {
                matches!(l, Line::Missed { .. })
            }),
            ("BA's turret is destroyed.", |l| {
                matches!(l, Line::TargetSectionLost { .. })
            }),
            ("BA destroyed.", |l| {
                matches!(
                    l,
                    Line::Killed {
                        designator: Some(_)
                    }
                )
            }),
            ("Hit on BA: turret, already destroyed.", |l| {
                matches!(
                    l,
                    Line::HitScored {
                        already_destroyed: true,
                        ..
                    }
                )
            }),
            ("Hit on X2.", |l| {
                matches!(l, Line::HitScored { section: None, .. })
            }),
            ("Hit.", |l| {
                matches!(
                    l,
                    Line::HitScored {
                        designator: None,
                        ..
                    }
                )
            }),
            ("Miss.", |l| {
                matches!(
                    l,
                    Line::Missed {
                        designator: None,
                        words: None
                    }
                )
            }),
            (
                "Hit on the hull's front facet: 280 of 400 armour left.",
                |l| {
                    matches!(
                        l,
                        Line::HitTaken {
                            armour: Some((280, 400)),
                            ..
                        }
                    )
                },
            ),
            (
                "Hit on the hull's front facet: armour gone, 140 of 200 inside.",
                |l| {
                    matches!(
                        l,
                        Line::HitTaken {
                            internals: Some((140, 200)),
                            ..
                        }
                    )
                },
            ),
            ("The left_track is destroyed: the tank is slowed.", |l| {
                matches!(l, Line::SectionLost { .. })
            }),
            ("Hit on the left_track, which is already destroyed.", |l| {
                matches!(
                    l,
                    Line::HitTaken {
                        already_destroyed: true,
                        ..
                    }
                )
            }),
            ("Power not online: STARTUP first.", |l| {
                *l == Line::PowerNotOnline
            }),
            ("No lock: LOCK a contact first.", |l| *l == Line::NoLock),
            ("FIRE cannon refused. Still cycling: 4 s to go.", |l| {
                matches!(
                    l,
                    Line::Refused {
                        cycling_s: Some(4),
                        ..
                    }
                )
            }),
            ("HEADING refused. Your tank cannot move.", |l| {
                matches!(l, Line::Refused { weapon: None, .. })
            }),
            (
                "LOCK refused. Power not online: STARTUP first.",
                |l| matches!(l, Line::Refused { order, .. } if order == "LOCK"),
            ),
            (
                "STARTUP refused. Your tank is a wreck.",
                |l| matches!(l, Line::Refused { order, .. } if order == "STARTUP"),
            ),
            (
                "FIRE cannon refused. Too many orders at once; try that again.",
                |l| matches!(l, Line::Refused { weapon: Some(w), .. } if w == "cannon"),
            ),
            // The game's 17b cut M1b.4: "Arming a weapon" and its refusals.
            (
                "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less.",
                |l| matches!(l, Line::Refused { order, weapon: Some(w), .. } if order == "FIRE" && w == "cannon"),
            ),
            (
                "FIRE cannon refused. Every slot is armed; FIRE <weapon> OFF frees one.",
                |l| matches!(l, Line::Refused { weapon: Some(w), .. } if w == "cannon"),
            ),
            (
                "FIRE cannon refused. That condition is not one this core takes; help cockpit lists the eight it does.",
                |l| matches!(l, Line::Refused { weapon: Some(w), .. } if w == "cannon"),
            ),
            ("Cleared: cannon fires only when you say.", |l| {
                matches!(l, Line::Other(_))
            }),
            ("Cleared: your guns fire only when you say.", |l| {
                matches!(l, Line::Other(_))
            }),
            ("Cannon fires at BA by S1 (62%, side).", |l| {
                matches!(l, Line::Other(_))
            }),
            ("S1 held the cannon: 54% under your 60%.", |l| {
                matches!(l, Line::Other(_))
            }),
            ("S1 held the cannon: BA shows its front.", |l| {
                matches!(l, Line::Other(_))
            }),
            (
                "S1 held the cannon: BA at 650 m, outside 200 to 600.",
                |l| matches!(l, Line::Other(_)),
            ),
            ("S1 held the cannon: 7 rounds, keeps 8.", |l| {
                matches!(l, Line::Other(_))
            }),
            (
                "cannon  ready  40 of 40 round(s)  band 1 of 2  62% to hit",
                |l| matches!(l, Line::WeaponRow(row) if row.percent == Some(62)),
            ),
            ("  S1: >=60% side/rear", |l| {
                *l == Line::WeaponArming {
                    slot: 1,
                    terms: ">=60% side/rear".into(),
                }
            }),
            (
                "Your tank is destroyed and takes no orders. EXIT leaves the pod.",
                |l| *l == Line::DestroyedNoOrders,
            ),
            ("Your tank cannot move.", |l| *l == Line::Immobile),
            // help simulator
            (
                "  AI fill: each side fields 2; machines take the unclaimed pods.",
                |l| *l == Line::AiFill { each: 2 },
            ),
            ("Match 3 is running. Good hunting.", |l| {
                *l == Line::Running { match_id: 3 }
            }),
            ("Match 3 is over (red). Its units are checked in.", |l| {
                matches!(
                    l,
                    Line::Over {
                        result: ResultWord::Red,
                        ..
                    }
                )
            }),
            ("Match 3 ends: every pilot has left it.", |l| {
                *l == Line::AllLeft { match_id: 3 }
            }),
            ("Back aboard RA after 2 m 14 s away.", |l| {
                matches!(
                    l,
                    Line::BackAboard {
                        away_ms: 134_000,
                        ..
                    }
                )
            }),
            ("This server has no simulator centre.", |l| {
                *l == Line::NoCentre
            }),
            (
                "Simulator centre: ended (match 3) — draw: both sides are out of ammunition",
                |l| {
                    matches!(
                        l,
                        Line::Board {
                            match_id: Some(3),
                            ..
                        }
                    )
                },
            ),
        ];
        for (line, holds) in cases {
            let read = classify(line);
            assert!(holds(&read), "{line} read as {read:?}");
        }
    }

    #[test]
    fn classify_is_total() {
        let mut inputs: Vec<String> = vec![
            String::new(),
            "   ".into(),
            "\t\t".into(),
            "x".repeat(16 * 1024),
            " ".repeat(4) + "front",
            "  ".to_owned() + "  ",
            "Match".into(),
            "Match 99999999999999999999999 is running. Good hunting.".into(),
            "Match 1 is over (".into(),
            "Hit on the ".into(),
            "Hit on the .".into(),
            "Hit on ".into(),
            "Locked on , % to hit.".into(),
            "Range to : m.".into(),
            "Bearing to (A): ° (rel °).".into(),
            "Back aboard RA after 2 x 14 s away.".into(),
            "Back aboard RA after  away.".into(),
            "  red: 1 (RA): ; 2 (".into(),
            "  hull  intact  internals /".into(),
            "    front 1/".into(),
            " refused. ".into(),
            "FIRE  refused. x".into(),
            "é refused. è".into(),
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2x s".into(),
            "Steering to 99999999999999°.".into(),
            "Making --5 kph.".into(),
            "You already hold red slot 999. `SIM LEAVE` gives it up.".into(),
            "Connected as  (account , x connection).".into(),
            "°°°°".into(),
        ];
        // Random bytes, decoded lossily, the way a torn stream arrives. The
        // generator steps once per input before the length is drawn, so a
        // zero length can never freeze it (every input is 1 to 96 bytes).
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        fn step(seed: &mut u64) -> u64 {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            *seed
        }
        let mut random = std::collections::BTreeSet::new();
        for _ in 0..2_000 {
            let len = 1 + (step(&mut seed) % 96) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| (step(&mut seed) & 0xff) as u8).collect();
            let input = String::from_utf8_lossy(&bytes).into_owned();
            random.insert(input.clone());
            inputs.push(input);
        }
        assert!(
            random.len() >= 1_900,
            "{} distinct random inputs",
            random.len()
        );
        for input in &inputs {
            let _ = classify(input);
        }
        assert_eq!(classify(""), Line::Other(String::new()));
        assert_eq!(classify("   "), Line::Other(String::new()));
        let long = "x".repeat(16 * 1024);
        assert_eq!(classify(&long), Line::Other(long.clone()));
    }

    #[test]
    fn the_assembler_keeps_tails_and_flushes_stale_prompts() {
        let mut a = Assembler::default();
        assert_eq!(a.feed(0, "Callsign R"), Vec::<String>::new());
        assert_eq!(a.feed(10, "A.\r\nNo cont"), vec!["Callsign RA.".to_owned()]);
        assert_eq!(
            a.flush_stale(STALE_TAIL_MS),
            None,
            "extended at 10, so not stale until 10 + STALE_TAIL_MS"
        );
        assert_eq!(a.feed(20, "acts.   \n"), vec!["No contacts.".to_owned()]);
        assert_eq!(a.flush_stale(10_000), None, "nothing waiting");
        assert_eq!(a.feed(100, "> "), Vec::<String>::new());
        assert_eq!(a.flush_stale(100 + STALE_TAIL_MS - 1), None);
        assert_eq!(a.flush_stale(100 + STALE_TAIL_MS), Some(">".to_owned()));
        assert_eq!(a.flush_stale(600 + STALE_TAIL_MS), None);
        // Blank lines are lines; a blank tail is never flushed.
        let blank = 1_000 + STALE_TAIL_MS;
        assert_eq!(a.feed(blank, "\n\n"), vec![String::new(), String::new()]);
        assert_eq!(a.feed(blank, "   "), Vec::<String>::new());
        assert_eq!(a.flush_stale(blank + STALE_TAIL_MS), None);
        // An overlong tail is a line.
        let long = blank + 2 * STALE_TAIL_MS;
        let lines = a.feed(long, &"y".repeat(MAX_TAIL_BYTES + 1));
        assert_eq!(lines.len(), 1);
        assert_eq!(a.flush_stale(long + 10 * STALE_TAIL_MS), None);
        // Cut AM.1: a line whose pieces arrive apart is whole up to
        // `STALE_TAIL_MS` — a tail extended at 0 and completed just inside it
        // is one line, never two broken ones.
        let mut a = Assembler::default();
        assert_eq!(
            a.feed(0, "cannon  cycling (6 s)  39 "),
            Vec::<String>::new()
        );
        assert_eq!(a.flush_stale(STALE_TAIL_MS - 1), None);
        assert_eq!(
            a.feed(STALE_TAIL_MS - 1, "of 40 round(s)\r\n"),
            vec!["cannon  cycling (6 s)  39 of 40 round(s)".to_owned()]
        );
        // The figure itself, not only its relations: a row cut by a pause of
        // up to two seconds (the old 300 ms broke one cut by 300) is whole.
        const { assert!(STALE_TAIL_MS >= 2_000) };
        assert_eq!(a.feed(0, "machine_gun  ready  "), Vec::<String>::new());
        assert_eq!(a.flush_stale(1_999), None, "1 999 ms apart is one line");
        assert_eq!(
            a.feed(1_999, "200 of 200 round(s)\r\n"),
            vec!["machine_gun  ready  200 of 200 round(s)".to_owned()]
        );
    }

    /// **What each cockpit answer answers** (cut AM.1): one literal sentence
    /// per row of [`answer_to`]'s table, read through [`classify`].
    #[test]
    fn answer_to_maps_every_cockpit_answer() {
        use AskKind as K;
        let kinds = |ks: &'static [AskKind]| Some(AnswerTo::Kinds(ks));
        let cases: Vec<(&str, Option<AnswerTo>)> = vec![
            ("Steering to 90°.", kinds(&[K::Heading])),
            ("Making 27 kph.", kinds(&[K::Speed])),
            (
                "That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.",
                kinds(&[K::Speed]),
            ),
            ("Your tank cannot move.", kinds(&[K::Heading, K::Speed])),
            ("Locking BA.", kinds(&[K::Lock])),
            ("Range to BA: 440 m.", kinds(&[K::Range])),
            ("Range to The Ford (A): 430 m.", kinds(&[K::Range])),
            ("Bearing to BA: 087° (rel 357°).", kinds(&[K::Bearing])),
            (
                "Bearing to The Ford (A): 270° (rel 180°).",
                kinds(&[K::Bearing]),
            ),
            (
                "No lock: LOCK a contact first, or name one.",
                kinds(&[K::Range, K::Bearing]),
            ),
            (
                "The panel has told you nothing yet.",
                kinds(&[K::Range, K::Bearing]),
            ),
            ("No lock: LOCK a contact first.", kinds(&[K::Fire])),
            ("No such contact.", kinds(&[K::Lock, K::Range, K::Bearing])),
            ("Firing cannon.", Some(AnswerTo::Fire("cannon".into()))),
            (
                "That unit carries no weapon called `cannon`.",
                Some(AnswerTo::Fire("cannon".into())),
            ),
            (
                "FIRE cannon refused. Still cycling: 4 s to go.",
                Some(AnswerTo::Refusal {
                    order: "FIRE".into(),
                    weapon: Some("cannon".into()),
                }),
            ),
            (
                "HEADING refused. Your tank cannot move.",
                Some(AnswerTo::Refusal {
                    order: "HEADING".into(),
                    weapon: None,
                }),
            ),
            (
                "The world is committing another action; please retry in a moment.",
                Some(AnswerTo::Oldest),
            ),
            ("Power not online: STARTUP first.", Some(AnswerTo::Oldest)),
            ("Power is still coming up; hold on.", Some(AnswerTo::Oldest)),
            (
                "Your tank is destroyed and takes no orders. EXIT leaves the pod.",
                Some(AnswerTo::Oldest),
            ),
            (
                "You are not at the station of a deployed unit.",
                Some(AnswerTo::Oldest),
            ),
            (
                "The panel is dark until the match starts; the simulator is still waiting for the other pods.",
                Some(AnswerTo::Oldest),
            ),
            // Everything else: the read replies (the picture reports their
            // openers), the lock line, notices, lobby lines, hints, chatter.
            ("BA  blue  bearing 087° (rel 032°)  far  450 m  2 s", None),
            ("No contacts.", None),
            ("cannon  ready  40 of 40 round(s)", None),
            ("Callsign RA.", None),
            ("Locked on BA, 62% to hit.", None),
            ("New contact X2: bearing 090°, far.", None),
            ("Match 1 is over (blue). Its units are checked in.", None),
            (
                "Pilot takes red slot 1. `SIM START` begins the match.",
                None,
            ),
            ("Hint: STARTUP brings your power up.", None),
            ("[RedNet] Ada: on your left", None),
        ];
        for (sentence, expected) in cases {
            let read = classify(sentence);
            assert!(
                !matches!(read, Line::Other(_)) || expected.is_none(),
                "{sentence} read as {read:?}"
            );
            assert_eq!(answer_to(&read), expected, "{sentence}");
        }
    }

    /// **A line that would not fit continues a full-width channel line** (cut
    /// AM.1): the renderer's own break rule on the untrimmed width, `°` one
    /// column, an indented line only after a line at the width (a run of
    /// spaces straddling the break), and a column-0 `[…` line never a game
    /// sentence.
    #[test]
    fn a_line_that_would_not_fit_continues_a_full_width_channel_line() {
        let end = "Match 1 is over (blue). Its units are checked in.";
        assert!(continues(116, end), "116 + 1 + `Match` is past 120");
        assert!(!continues(100, end));
        assert!(!continues(114, end), "114 + 1 + 5 is 120: it fit");
        assert!(continues(115, end));
        // A column-0 `[…` line is another player's whatever it ends with, so
        // it begins a chain and is never a transient.
        for chat in [
            "[RedNet] Ada: the world is busy; please retry in a moment.",
            "[RedNet] Ada: That cockpit will not wake for you: control state unknown",
        ] {
            assert_eq!(classify(chat), Line::Other(chat.to_owned()));
            assert_eq!(answer_to(&classify(chat)), None);
        }
        assert!(
            !continues(119, &format!(" {end}")),
            "an indented line never continues one short of the width"
        );
        // …but a run of spaces that straddles a break at exactly the width
        // starts the next line with a space (an empty word breaks only a full
        // line), so a line at the width is continued by an indented one too.
        let full = format!("[RedNet] Ada: {}", "x".repeat(106));
        assert_eq!(full.chars().count(), 120);
        let wrapped = mush_ansi::wrap_indent(&format!("{full}   {end}"), 120, 0);
        assert_eq!(wrapped.len(), 2, "{wrapped:?}");
        assert_eq!(wrapped[0].chars().count(), 120);
        assert!(wrapped[1].starts_with(' '), "{wrapped:?}");
        assert!(continues(120, &wrapped[1]));
        let short = mush_ansi::wrap_indent(&format!("{}     {end}", &full[..117]), 120, 0);
        assert_eq!(short[0].chars().count(), 120, "the run fills the line");
        assert!(short[1].starts_with(' '), "{short:?}");
        assert!(continues(120, &short[1]));
        // A line with degree signs counts each as one column.
        let mut a = Assembler::default();
        let bearing = "Bearing to BA: 087° (rel 357°).";
        let lines = a.feed_cols(0, &format!("{bearing}\r\n"));
        assert_eq!(lines, vec![(bearing.to_owned(), 31)]);
        assert!(bearing.len() > 31, "bytes are not columns");
        // A wrapped channel line keeps the run of spaces at its break: the
        // untrimmed count is the width the renderer measured.
        let padded = format!("[RedNet] Ada: {}{}", "x".repeat(86), " ".repeat(20));
        let lines = a.feed_cols(10, &format!("{padded}\r\n"));
        assert_eq!(lines, vec![(padded.trim_end().to_owned(), 120)]);
        assert!(continues(120, end));
        assert!(!continues(100, end), "the trimmed width would let it pass");
        // The stale flush counts the same way.
        assert_eq!(a.feed(20, "[RedNet] Ada: hi   "), Vec::<String>::new());
        assert_eq!(
            a.flush_stale_cols(20 + STALE_TAIL_MS),
            Some(("[RedNet] Ada: hi".to_owned(), 19))
        );
    }

    #[test]
    fn split_utf8_and_crlf_through_the_decoder() {
        let mut decoder = Decoder::default();
        let mut a = Assembler::default();
        let wire =
            "Bearing to BA: 087° (rel 357°).\r\nNew contact X2: bearing 090°, far.\r\n".as_bytes();
        let mut lines = Vec::new();
        // One byte at a time: every multi-byte degree sign is split.
        for (i, byte) in wire.iter().enumerate() {
            let text = decoder.feed(std::slice::from_ref(byte));
            lines.extend(a.feed(i as u64, &text));
        }
        assert_eq!(
            lines,
            vec![
                "Bearing to BA: 087° (rel 357°).".to_owned(),
                "New contact X2: bearing 090°, far.".to_owned()
            ]
        );
        assert!(matches!(
            classify(&lines[0]),
            Line::Bearing { rel_deg: 357, .. }
        ));
        assert!(matches!(
            classify(&lines[1]),
            Line::NewContact {
                bearing_deg: Some(90),
                ..
            }
        ));
    }

    /// **`VIEW BRIEF`, read** (cut AN.2): every bearing line's form — runs,
    /// no runs, the arena's edge, two-word classes, a diagonal — the header
    /// with its numbers, a ray or a header cut short (`Other`), both dark
    /// windows, what each answers, and the board's two mode lines.
    #[test]
    fn the_view_brief_lines() {
        let ray = |bearing_deg: u32, runs: &[(&str, u32)], edge: bool| Line::GroundRay {
            bearing_deg,
            runs: runs.iter().map(|(c, m)| ((*c).to_owned(), *m)).collect(),
            edge,
        };
        for (text, want) in [
            (
                "North: 10 m of road, then grass to 90 m, then beyond the window.",
                ray(0, &[("road", 10), ("grass", 90)], false),
            ),
            (
                "North-east: 14 m of road, then bare ground to 70 m, then light woods to 130 m, \
                 then the edge of the arena.",
                ray(
                    45,
                    &[("road", 14), ("bare ground", 70), ("light woods", 130)],
                    true,
                ),
            ),
            (
                "East: 190 m of road, then beyond the window.",
                ray(90, &[("road", 190)], false),
            ),
            (
                "South-west: 40 m of heavy woods, then deep water to 120 m, then beyond the window.",
                ray(225, &[("heavy woods", 40), ("deep water", 120)], false),
            ),
            ("West: the edge of the arena.", ray(270, &[], true)),
            ("North-west: beyond the window.", ray(315, &[], false)),
            (
                "South: 90 m of grass, then beyond the window.",
                ray(180, &[("grass", 90)], false),
            ),
            (
                "South-east: 10 m of road, then rubble to 30 m, then beyond the window.",
                ray(135, &[("road", 10), ("rubble", 30)], false),
            ),
        ] {
            assert_eq!(classify(text), want, "{text}");
        }
        // Cut before its tail (a wrapped line), a stray word, a bad bearing:
        // `Other`.
        for text in [
            "North: 10 m of road, then grass to 90 m, then beyond the",
            "North: 10 m of road, then grass to 90 m,",
            "North: 10 m of Road, then beyond the window.",
            "Northward: 10 m of road, then beyond the window.",
            "North: ten m of road, then beyond the window.",
            "North: 10 m of road then beyond the window.",
        ] {
            assert!(matches!(classify(text), Line::Other(_)), "{text}");
        }
        // The header, its numbers read.
        assert_eq!(
            classify(
                "Window around RA: cells 0 to 38 east and 88 to 107 north of 256 by 256; you at \
                 (6, 97), heading 090°; 10 m a cell."
            ),
            Line::ViewHeader {
                extent: (256, 256),
                cell_m: 10,
                you: (6, 97),
            }
        );
        assert_eq!(
            classify(
                "Window around BB: cells 57 to 95 east and 0 to 19 north of 96 by 64; you at \
                 (76, 4), heading 270°; 5 m a cell."
            ),
            Line::ViewHeader {
                extent: (96, 64),
                cell_m: 5,
                you: (76, 4),
            }
        );
        // A header cut short is `Other`.
        for text in [
            "Window around RA: cells 0 to 38 east and 88 to 107 north of 256 by 256; you at",
            "Window around RA: cells 0 to 38 east and 88 to 107 north of 256 by 256; you at \
             (6, 97), heading 090°; 10 m a",
            "Window around me: cells 0 to 38 east and 88 to 107 north of 256 by 256; you at \
             (6, 97), heading 090°; 10 m a cell.",
        ] {
            assert!(matches!(classify(text), Line::Other(_)), "{text}");
        }
        // The two dark windows.
        for text in [
            "The window is dark: the panel has told you nothing yet.",
            "The window is dark: there is no ground under the panel to draw.",
        ] {
            assert_eq!(classify(text), Line::WindowDark, "{text}");
        }
        // What each answers: `VIEW BRIEF`, the one line typed for them.
        for line in [
            ray(0, &[("grass", 90)], false),
            Line::ViewHeader {
                extent: (256, 256),
                cell_m: 10,
                you: (6, 97),
            },
            Line::WindowDark,
        ] {
            assert_eq!(
                answer_to(&line),
                Some(AnswerTo::Kinds(&[AskKind::View])),
                "{line:?}"
            );
        }
        // The brief's other lines keep their own mapping.
        for text in [
            "BA, blue: bearing 045° (rel 000°), far, 450 m, 1 s old.",
            "In the window: A The Ford, 190 m at 093°.",
            "Off the window: C North Barn, 1390 m at 045°.",
        ] {
            assert!(matches!(classify(text), Line::Other(_)), "{text}");
            assert_eq!(answer_to(&classify(text)), None);
        }
        assert_eq!(answer_to(&Line::NoContacts), None);
        // The board's rule lines, two-space indented; any other word, or at
        // column 0, is not one.
        assert_eq!(
            classify("  mode: hold — keep The Crossroads (B) uncontested for 120 s"),
            Line::BoardMode(MatchMode::Hold)
        );
        assert_eq!(
            classify("  mode: range — 2 targets on the blue side"),
            Line::BoardMode(MatchMode::Range)
        );
        for text in [
            "  mode: skirmish — eliminate the other side",
            "  mode: hold",
            "mode: hold — keep The Crossroads (B) uncontested for 120 s",
            "   mode: hold — keep The Crossroads (B) uncontested for 120 s",
        ] {
            assert!(matches!(classify(text), Line::Other(_)), "{text}");
        }
        assert_eq!(answer_to(&Line::BoardMode(MatchMode::Hold)), None);
    }
}
