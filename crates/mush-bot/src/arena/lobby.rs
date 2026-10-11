//! The lobby machine (cut AJ.2): from wherever the character stands to a
//! powered tank in a running match, and back to the next claim.
//!
//! **Serial.** One lobby line is in flight at a time, each step with the
//! answers it expects and `[arena].step_timeout_ms` to get one; the cockpit's
//! own lines (the pilot's orders and polls) are the lane's and run beside it
//! only while the machine is in [`Step::Fight`].
//!
//! **A closed vocabulary.** Every line this machine can type is a
//! [`LobbyLine`], and [`LobbyLine::line`] is the only way one is built: `look`,
//! `ooc`, `simulator`, `red`/`blue`, `SIM STATUS`, `SIM TEAM red|blue`,
//! `SIM START`, `BOARD`, `STARTUP` and `SIM LEAVE`. It can never type `SIM AI`,
//! `SIM UNIT`, `SIM CONCEDE`, `SIM MODE`, `SIM RANGE`, `SIM ABORT`, `ORDER` or
//! `EXIT` (16c Q-AJ2, Q-AJ3), nor `SIM MIND` or `MIND` (the game's 17b cut M5.1
//! and M5.4: the bot never chooses, reads or switches a mind) — the list
//! `tests/arena_e2e.rs`' `forbidden` enforces. `SIM START` goes out only while
//! the machine believes the pilot leads its side and stands at [`Step::Start`]
//! (cut AM.2: checked in [`Lobby::poll`] and again in [`Lobby::due`], so a lead
//! lost to a line read in between never starts a match). `SIM LEAVE` goes
//! once, as the lane stops (again only on a busy answer, inside the 3 s its
//! answer is waited for), when the centre was last read idle or in selection
//! and the pilot holds a slot — and (cut AM.2, 16c Q-AM3) when a stop finds a
//! `SIM TEAM` or a `SIM START` typed and unanswered, the stop waits for that
//! answer (at most `[arena].step_timeout_ms` from its typing) and then types
//! the one `SIM LEAVE` whatever it learned, none when the answer names the
//! other side's slot (that slot is not this run's to give up). A blocker or
//! a match's end closes without the wait: a refused claim neither waits nor
//! leaves. So a pilot that stops before a match never locks the centre for the
//! people after it (`help simulator`: "the last one to leave closes the
//! match").
//!
//! **The pause** (cut AM.2, 16c Q-AM2). While the lane is paused the machine
//! still reads, but every line is read at the pause instant, so a clock a line
//! sets then (a rematch, a `STARTUP` wanted again, the quiet rule's last line)
//! starts from the resume; `Lobby::resume` moves every lobby wait on by the
//! pause's length (`Lobby::shift`). The match's own times are the centre's
//! and stay real: a boarding or a running read while paused is stamped when
//! it was really read and never shifted, so the boarding bound keeps the
//! window the centre announced, and a match recorded while paused keeps its
//! real length. A pause that spans a match's boarding window and its start
//! loses that match (cut AO.2): the game drops the slot nobody seated, the
//! `BOARD` typed after the resume is answered with no slot, and the match is
//! recorded `left` and the next one claimed — never a blocker.
//!
//! **Who leads** (`help simulator`: "Whoever claims a side first leads it; if
//! they give the slot up … the next who claimed leads"). A claim window per
//! side, opened by anything that clears the board (a match's end, `The last
//! match is cleared from the board`) or by a `SIM STATUS` read of the side's
//! row, is filled in order by every `{name} takes {side} slot {i}.` and emptied
//! by every `{name} gives up {side} slot {i}.`; the pilot leads while nobody else
//! stands ahead of it in its side's window. An error here only ever withholds
//! `SIM START`: with no window the pilot does not lead, and a refusal
//! (`Only a side's lead or staff can start the match.`) closes the window.
//!
//! **The followed match.** Every centre room hears every match's
//! announcements. A `Match {n} is boarding.` counts only after the pilot's own
//! claim ([`Picture::claimed`]); its `n` is the followed match, and the running
//! and ending lines move the machine only for that `n`.
use serde::Serialize;

use super::{
    Side,
    lane::LaneEvent,
    lines::{self, BoardPilot, Line, Standing, is_centre_room, pod_number},
    picture::{Phase, Picture, Power},
    report::MatchResult,
    settings::Settings,
};

/// A transient answer re-sends the same line after this long…
pub const TRANSIENT_RETRY_MS: u64 = 200;
/// …at most this many times.
pub const TRANSIENT_RETRIES: u32 = 40;
/// A `look` or a walk with an answer but no room it can use is read as over
/// once this long has passed since it was typed…
pub const WALK_SETTLE_MS: u64 = 1_500;
/// …and the output has been quiet this long.
pub const QUIET_MS: u64 = 300;
/// A busy centre is read again this often.
pub const SURVEY_EVERY_MS: u64 = 10_000;
/// After a claim under another name and none under the pilot's, `SIM TEAM` is
/// typed once more this long after it.
pub const CLAIM_RECHECK_MS: u64 = 1_000;
/// Waiting for the arena to answer `STARTUP` with `Power online.`.
pub const POWER_WAIT_MS: u64 = 30_000;
/// `BOARD` again this often while the tank is not taken back.
pub const REBOARD_EVERY_MS: u64 = 3_000;
/// After a tank is taken back, how long to let its catch-up block arrive before
/// reading whether its power is up.
pub const RESUME_SETTLE_MS: u64 = 1_000;
/// The next claim goes this long after the followed match's `is over` line.
pub const REMATCH_AFTER_MS: u64 = 1_000;
/// How long `SIM LEAVE`'s answer is waited for; it is never typed again.
pub const LEAVE_WAIT_MS: u64 = 3_000;
/// Added to the boarding window a `Match {n} is boarding.` line announces.
pub const BOARDING_SLACK_MS: u64 = 10_000;
/// A `BOARD` refused as outside the team room walks there at most this often.
pub const BOARD_ROOM_TRIES: u32 = 3;

/// The blocker when the character cannot walk to the centre.
pub const UNREACHABLE: &str = "The simulator is not reachable from here: the pilot starts from Limbo, \
                               the Welcome Hall or the simulator centre.";

/// **Every line the lobby can type.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LobbyLine {
    Look,
    Ooc,
    Simulator,
    /// `red` or `blue`: the team room.
    TeamRoom(Side),
    SimStatus,
    SimTeam(Side),
    SimStart,
    Board,
    Startup,
    SimLeave,
}

impl LobbyLine {
    /// The line as typed: the only way a lobby line is built.
    pub fn line(self) -> String {
        match self {
            LobbyLine::Look => "look".into(),
            LobbyLine::Ooc => "ooc".into(),
            LobbyLine::Simulator => "simulator".into(),
            LobbyLine::TeamRoom(side) => side.as_str().into(),
            LobbyLine::SimStatus => "SIM STATUS".into(),
            LobbyLine::SimTeam(side) => format!("SIM TEAM {}", side.as_str()),
            LobbyLine::SimStart => "SIM START".into(),
            LobbyLine::Board => "BOARD".into(),
            LobbyLine::Startup => "STARTUP".into(),
            LobbyLine::SimLeave => "SIM LEAVE".into(),
        }
    }

    /// A walk (`look`, `ooc`, `simulator`, a team room) rather than a centre verb.
    pub fn is_walk(self) -> bool {
        matches!(
            self,
            LobbyLine::Look | LobbyLine::Ooc | LobbyLine::Simulator | LobbyLine::TeamRoom(_)
        )
    }
}

/// Where the machine is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// `look`: the room heading decides the walk.
    Look,
    /// `ooc` to the Welcome Hall, then `simulator` to the lobby.
    Walk,
    /// `SIM STATUS`: the centre's state and the side's row.
    Survey,
    /// `SIM TEAM {side}`.
    Claim,
    /// The lead's grace, then `SIM START`.
    Start,
    /// Not the lead: waiting for somebody to start the match.
    Await,
    /// `red`/`blue`: into the team room.
    ToRoom,
    /// `BOARD`.
    Board,
    /// In the pod: waiting for `Match {n} is running.`.
    AwaitRunning,
    /// `STARTUP`.
    Startup,
    /// Waiting for `Power online.`.
    AwaitPower,
    /// The pilot flies; the lane types the cockpit's lines.
    Fight,
    /// Out of the tank mid-match: `BOARD` until it is taken back.
    Reboard,
    /// Taken back: letting the catch-up block arrive.
    Resume,
    /// The followed match is over (or left): the next claim, or the end.
    Over,
    /// Nothing more to do.
    Finished,
}

/// The centre's state as last read or inferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Centre {
    Unknown,
    Idle,
    Selection,
    Boarding,
    Running,
    /// Ended, aborted or void: the last match's result is on the board.
    Terminal,
}

impl Centre {
    fn from_word(word: &str) -> Centre {
        match word {
            "idle" => Centre::Idle,
            "selection" => Centre::Selection,
            "boarding" => Centre::Boarding,
            "running" | "stopping" => Centre::Running,
            "ended" | "aborted" | "void" => Centre::Terminal,
            _ => Centre::Unknown,
        }
    }
}

/// The lobby line in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Flight {
    line: LobbyLine,
    sent_ms: u64,
}

/// `SIM LEAVE`'s one use. A transient answer (the command did not commit, so
/// the slot is still held) re-sends it after 200 ms like any lobby line, but
/// only inside the 3 s its answer is waited for; nothing re-sends it after.
///
/// **A stop with a claim or a start in flight** (cut AM.2) waits for that
/// line's answer first ([`Leave::AwaitFlight`]), at most until `until` (its
/// typing and `[arena].step_timeout_ms`), then types the one `SIM LEAVE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leave {
    No,
    /// Stopped with `line` (`SIM TEAM` or `SIM START`) typed and unanswered:
    /// its answer queues the leave (the other side's slot ends it), and so
    /// does `until`.
    AwaitFlight {
        line: LobbyLine,
        until: u64,
    },
    Queued,
    /// Typed first at this time.
    Sent(u64),
    /// Answered as busy: again at `at`, still bounded by the first send.
    Again {
        first: u64,
        at: u64,
    },
    Done,
}

/// What a `SIM STATUS` has shown so far.
#[derive(Clone, Debug, Default)]
struct Survey {
    head: Option<(String, Option<u64>)>,
    red: Option<Vec<BoardPilot>>,
    blue: Option<Vec<BoardPilot>>,
}

impl Survey {
    fn row(&self, side: Side) -> Option<&Vec<BoardPilot>> {
        match side {
            Side::Red => self.red.as_ref(),
            Side::Blue => self.blue.as_ref(),
        }
    }
}

/// A side's callsign for slot `index`: `R` and `A` for red slot 1.
pub fn callsign(side: Side, index: u8) -> String {
    let letter = char::from(b'A'.saturating_add(index.saturating_sub(1)).min(b'Z'));
    format!("{}{letter}", side.letter())
}

/// Case-folded, for comparing a name the centre printed with the pilot's.
fn fold(name: &str) -> String {
    name.trim().to_lowercase()
}

/// **Chat** (cut AO.2), which never belongs to a room's look: a channel's
/// frame (`[Public] …`, and an attributed emit's `[Name] …`), the OOC frame
/// (`<OOC> …`), or a say, whisper or page frame (`Ada says, "…"`). A pose
/// prints as the poser's name and the action (`help communication`), with no
/// frame of its own that line mode can tell from a room's description, so it
/// is read by these frames alone.
fn chat_frame(raw: &str) -> bool {
    let line = raw.trim();
    line.starts_with('[')
        || line.starts_with("<OOC>")
        || [" says, \"", " whispers, \"", " pages: "]
            .iter()
            .any(|frame| line.contains(frame))
}

/// **A walk's line refused** (cut AO.2): the game's answer to a line it does
/// not take (`Huh?  (Type "help" for help.)`), which answers the walk and never
/// belongs to a room's look.
fn walk_refused(raw: &str) -> bool {
    raw.trim().starts_with("Huh?")
}

/// `… or in 30 second(s).` → 30 000.
fn boarding_window_ms(raw: &str) -> Option<u64> {
    let (_, rest) = raw.trim().rsplit_once(" or in ")?;
    let secs = rest.strip_suffix(" second(s).")?;
    secs.parse::<u64>().ok().map(|s| s.saturating_mul(1_000))
}

/// **The lobby machine.**
#[derive(Debug)]
pub struct Lobby {
    side: Side,
    settings: Settings,
    me: String,
    step: Step,
    /// The line wanted, and when it may go.
    want: Option<(LobbyLine, u64)>,
    flight: Option<Flight>,
    /// Transient re-sends of the current line.
    retries: u32,
    /// A line other than the echo has arrived since the flight was typed.
    answered: bool,
    /// **The foreign room's look** (cut AO.2): the real read time of the last
    /// line of another room's look the walk is reading past — its heading,
    /// then its description and exits — while that look is open
    /// ([`Lobby::on_line`]).
    foreign_block_ms: Option<u64>,
    last_line_ms: u64,
    /// A waiting step's deadline, and the blocker it ends in.
    deadline: Option<(u64, String)>,
    centre: Centre,
    /// The slot index the pilot holds.
    slot: Option<u8>,
    /// The slot last held, for the callsign after a match's end releases it.
    last_slot: Option<u8>,
    /// Claim windows, `[red, blue]`, of case-folded names in claim order.
    windows: [Option<Vec<String>>; 2],
    lead: bool,
    claim_ms: u64,
    /// The last claim or give-up by anybody else: the lead's grace restarts.
    last_other_ms: u64,
    busy_since: Option<u64>,
    /// `SIM START` found the other side empty: waiting for a claim there.
    empty_since: Option<u64>,
    waiting_other: bool,
    /// A claim on the pilot's side under another name, while claiming.
    foreign_claim: Option<(String, u8, u64)>,
    claim_rechecked: bool,
    survey: Survey,
    boarding_ms: u64,
    board_window_ms: Option<u64>,
    board_room_tries: u32,
    running_ms: Option<u64>,
    lead_at_boarding: bool,
    /// The hatch opened: the match is ending, and `OUTSIDE_COCKPIT` is expected.
    hatch_seen: bool,
    reboard_since: u64,
    reboard_next: u64,
    reboard_room_first: bool,
    back_away_ms: Option<u64>,
    resume_at: u64,
    reclaims: u32,
    away_ms: u64,
    /// The match the lobby recorded last, and the one whose end it acted on.
    recorded: Option<u64>,
    ended: Option<u64>,
    rematch_at: Option<u64>,
    matches: Vec<MatchResult>,
    blocker: Option<String>,
    closed: bool,
    leave: Leave,
    /// The lane's pause (cut AM.2): every line read while it is set is read
    /// at this instant.
    paused_at: Option<u64>,
    /// When the last line was really read: a match record's length runs to it
    /// even while paused.
    read_ms: u64,
    events: Vec<LaneEvent>,
}

impl Lobby {
    /// A machine for `side`, `me` being the character's in-game name as best
    /// the runner knows it. It starts with `look`.
    pub fn new(side: Side, settings: Settings, me: &str) -> Lobby {
        Lobby {
            side,
            settings,
            me: me.trim().to_owned(),
            step: Step::Look,
            want: Some((LobbyLine::Look, 0)),
            flight: None,
            retries: 0,
            answered: false,
            foreign_block_ms: None,
            last_line_ms: 0,
            deadline: None,
            centre: Centre::Unknown,
            slot: None,
            last_slot: None,
            windows: [None, None],
            lead: false,
            claim_ms: 0,
            last_other_ms: 0,
            busy_since: None,
            empty_since: None,
            waiting_other: false,
            foreign_claim: None,
            claim_rechecked: false,
            survey: Survey::default(),
            boarding_ms: 0,
            board_window_ms: None,
            board_room_tries: 0,
            running_ms: None,
            lead_at_boarding: false,
            hatch_seen: false,
            reboard_since: 0,
            reboard_next: 0,
            reboard_room_first: false,
            back_away_ms: None,
            resume_at: 0,
            reclaims: 0,
            away_ms: 0,
            recorded: None,
            ended: None,
            rematch_at: None,
            matches: Vec::new(),
            blocker: None,
            closed: false,
            leave: Leave::No,
            paused_at: None,
            read_ms: 0,
            events: Vec::new(),
        }
    }

    // -- reading --------------------------------------------------------------

    pub fn step(&self) -> Step {
        self.step
    }

    pub fn side(&self) -> Side {
        self.side
    }

    /// The character's name as the centre prints it, as best known.
    pub fn me(&self) -> &str {
        &self.me
    }

    /// The pilot leads its side (as the claim window reads).
    pub fn lead(&self) -> bool {
        self.lead
    }

    /// The slot index held.
    pub fn slot(&self) -> Option<u8> {
        self.slot
    }

    pub fn centre(&self) -> Centre {
        self.centre
    }

    /// The pilot is flying: the cockpit's lines are the lane's.
    pub fn fighting(&self) -> bool {
        self.step == Step::Fight && !self.closed
    }

    /// The machine has finished: the last match is recorded or a step blocked.
    pub fn finished(&self) -> bool {
        self.step == Step::Finished
    }

    /// Stopped or finished: nothing more but `SIM LEAVE`.
    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn blocker(&self) -> Option<&str> {
        self.blocker.as_deref()
    }

    pub fn matches(&self) -> &[MatchResult] {
        &self.matches
    }

    pub fn matches_asked(&self) -> u32 {
        self.settings.matches
    }

    /// The callsign of the slot held (or last held).
    pub fn callsign(&self) -> Option<String> {
        self.slot
            .or(self.last_slot)
            .map(|index| callsign(self.side, index))
    }

    /// `SIM LEAVE` is queued, or typed and not answered for less than 3 s, or
    /// (cut AM.2) a stop waits for a claim's or a start's answer before it.
    pub fn leave_pending(&self, now_ms: u64) -> bool {
        match self.leave {
            Leave::Queued | Leave::AwaitFlight { .. } => true,
            Leave::Sent(first) | Leave::Again { first, .. } => {
                now_ms < first.saturating_add(LEAVE_WAIT_MS)
            }
            Leave::No | Leave::Done => false,
        }
    }

    /// **A lobby line awaits its answer** (cut AM.1, for the lane's answer
    /// ledger): a line is in flight, or `SIM LEAVE` was typed and its 3 s are
    /// not over.
    pub(crate) fn awaiting(&self, now_ms: u64) -> bool {
        self.flight.is_some()
            || matches!(self.leave, Leave::Sent(first) if now_ms < first.saturating_add(LEAVE_WAIT_MS))
    }

    /// What the machine said since the last call.
    pub fn take_events(&mut self) -> Vec<LaneEvent> {
        std::mem::take(&mut self.events)
    }

    // -- the line out -----------------------------------------------------------

    /// The line to type now, if any. A stop waiting for a claim's or a start's
    /// answer types its `SIM LEAVE` at `until` even unanswered (cut AM.2), and
    /// `SIM START` is never due unless the pilot leads and stands at
    /// [`Step::Start`] (cut AM.2: [`Lobby::poll`] drops it first; this is for a
    /// line read between the two).
    pub fn due(&self, now_ms: u64) -> Option<LobbyLine> {
        match self.leave {
            Leave::Queued => return Some(LobbyLine::SimLeave),
            Leave::AwaitFlight { until, .. } if now_ms >= until => {
                return Some(LobbyLine::SimLeave);
            }
            Leave::Again { first, at }
                if now_ms >= at && now_ms < first.saturating_add(LEAVE_WAIT_MS) =>
            {
                return Some(LobbyLine::SimLeave);
            }
            _ => {}
        }
        if self.closed {
            return None;
        }
        match self.want {
            Some((LobbyLine::SimStart, _)) if !self.may_start() => None,
            Some((line, at)) if now_ms >= at => Some(line),
            _ => None,
        }
    }

    /// `SIM START` may go: the pilot leads, at [`Step::Start`] (cut AM.2).
    fn may_start(&self) -> bool {
        self.lead && self.step == Step::Start
    }

    /// The lane typed `line` at `now_ms`.
    pub fn sent(&mut self, now_ms: u64, line: LobbyLine) {
        if line == LobbyLine::SimLeave {
            self.leave = match self.leave {
                Leave::Again { first, .. } => Leave::Sent(first),
                _ => Leave::Sent(now_ms),
            };
            // A claim or a start a stop waited for is given up on with it
            // (cut AM.2): only `SIM LEAVE`'s answer is awaited now.
            self.flight = None;
            return;
        }
        self.want = None;
        self.flight = Some(Flight {
            line,
            sent_ms: now_ms,
        });
        self.answered = false;
        // A line typed closes a foreign room's look (cut AO.2): what follows
        // may be that line's own answer (a heading read before `look` is
        // typed, then `look`'s own room).
        self.foreign_block_ms = None;
        if self.step == Step::Reboard && line == LobbyLine::Board {
            self.reboard_next = now_ms.saturating_add(REBOARD_EVERY_MS);
        }
    }

    // -- the lines in -------------------------------------------------------------

    /// Read one line (already applied to `picture`; never the echo of a line the
    /// lane typed). While paused (cut AM.2) the line is read at the pause
    /// instant: every clock it sets starts from the resume.
    pub fn on_line(&mut self, now_ms: u64, raw: &str, line: &Line, picture: &mut Picture) {
        self.read_ms = now_ms;
        let now_ms = self.paused_at.unwrap_or(now_ms);
        self.last_line_ms = now_ms;
        // A room heading that is not where the walk's line goes is somebody
        // else's answer (a late `look`, a move it did not ask for): it is no
        // answer to the line in flight, so the quiet rule never reads it as a
        // refusal (cut AM.2).
        let walking = matches!(self.step, Step::Look | Step::Walk);
        let foreign_room = walking && matches!(line, Line::Room(name) if !self.walk_arrives(name));
        // **The foreign room's look** (cut AO.2): one output's pieces arrive
        // at most `STALE_TAIL_MS` apart (the lane's own continuation rule), so
        // a room's description and exits follow its heading within it. The
        // heading opens the block at its real read time; a plain line that is
        // not chat, read within `STALE_TAIL_MS` of the block's last line, is
        // that room's and moves the block on. The block suppresses only
        // `answered`: everything else about the line runs as before, and any
        // other line — a heading the walk arrives at, a busy answer (still
        // sent to `transient`, which types the walk's line again), a refusal,
        // chat, or a line read after a longer gap — closes it and answers the
        // walk, so the quiet rule then blocks as before.
        let real_ms = self.read_ms;
        let foreign_look = walking
            && !foreign_room
            && matches!(line, Line::Other(_))
            && !chat_frame(raw)
            && !walk_refused(raw)
            && self
                .foreign_block_ms
                .is_some_and(|last| real_ms.saturating_sub(last) <= lines::STALE_TAIL_MS);
        self.foreign_block_ms = (foreign_room || foreign_look).then_some(real_ms);
        if self.flight.is_some() && !foreign_room && !foreign_look {
            self.answered = true;
        }
        if let Line::Connected { name, .. } = line {
            self.me = name.trim().to_owned();
        }
        self.centre_line(now_ms, line, picture);
        if self.closed {
            if let Leave::AwaitFlight { line: flown, .. } = self.leave {
                self.await_flight(flown, line, picture);
            } else if matches!(line, Line::Transient(_)) {
                // The one line still in flight is `SIM LEAVE`: a busy answer
                // means it did not commit and the slot is still held.
                self.leave_again(now_ms);
            }
            return;
        }
        if matches!(line, Line::Transient(_)) {
            self.transient(now_ms, picture);
            return;
        }
        if self.match_end(now_ms, picture) {
            return;
        }
        match self.step {
            Step::Look => self.on_look(now_ms, line),
            Step::Walk => self.on_walk(now_ms, line),
            Step::Survey => self.on_survey(now_ms, raw, line, picture),
            Step::Claim => self.on_claim(now_ms, line, picture),
            Step::Start => self.on_start(now_ms, raw, line, picture),
            Step::Await => self.on_await(now_ms, raw, line, picture),
            Step::ToRoom => self.on_to_room(now_ms, line),
            Step::Board => self.on_board(now_ms, line, picture),
            Step::AwaitRunning => self.on_await_running(now_ms, line, picture),
            Step::Startup | Step::AwaitPower => self.on_power(now_ms, line, picture),
            Step::Fight => self.on_fight(now_ms, line, picture),
            Step::Reboard => self.on_reboard(now_ms, line, picture),
            Step::Resume | Step::Over | Step::Finished => {}
        }
    }

    /// The timers: deadlines, quiet walks, the lead's grace, the re-board
    /// cadence, the rematch.
    pub fn poll(&mut self, now_ms: u64, picture: &mut Picture) {
        if self.closed {
            return;
        }
        // **`SIM START` only while leading** (cut AM.2): a start wanted by a
        // pilot that has lost the lead (a match's end releases every lead; a
        // claim window read again) is dropped, and the pilot waits for
        // somebody to start, as after the refusal that says so.
        if matches!(self.want, Some((LobbyLine::SimStart, _))) && !self.may_start() {
            self.want = None;
            if self.step == Step::Start {
                self.await_start(now_ms);
            }
        }
        if let Some(flight) = self.flight
            && self.step != Step::Reboard
            && now_ms >= flight.sent_ms.saturating_add(self.settings.step_timeout_ms)
        {
            let secs = self.settings.step_timeout_ms / 1_000;
            // **A running match is never given up because the tank would not
            // start** (cut AM.2): `STARTUP` is typed again.
            if self.step == Step::Startup && Self::followed_running(picture) {
                self.say(format!(
                    "No answer to `STARTUP` within {secs} s; typing it again."
                ));
                self.ask(now_ms, LobbyLine::Startup);
                return;
            }
            let sentence = format!("No answer to `{}` within {secs} s.", flight.line.line());
            return self.block(now_ms, &sentence, picture);
        }
        if let Some((at, sentence)) = &self.deadline
            && now_ms >= *at
        {
            // …nor because its power did not come up (cut AM.2).
            if self.step == Step::AwaitPower && Self::followed_running(picture) {
                self.deadline = None;
                self.say(format!(
                    "Power did not come online within {} s; typing STARTUP again.",
                    POWER_WAIT_MS / 1_000
                ));
                self.step = Step::Startup;
                self.ask(now_ms, LobbyLine::Startup);
                return;
            }
            let sentence = sentence.clone();
            return self.block(now_ms, &sentence, picture);
        }
        let quiet = |flight: &Flight| {
            now_ms >= flight.sent_ms.saturating_add(WALK_SETTLE_MS)
                && now_ms >= self.last_line_ms.saturating_add(QUIET_MS)
        };
        match self.step {
            Step::Look => {
                // No room the pilot knows: not the centre, not the hall.
                if let Some(flight) = self.flight
                    && self.answered
                    && quiet(&flight)
                {
                    self.walk(now_ms, LobbyLine::Ooc);
                }
            }
            Step::Walk => {
                if let Some(flight) = self.flight
                    && self.answered
                    && quiet(&flight)
                {
                    self.block(now_ms, UNREACHABLE, picture);
                }
            }
            Step::Survey => {
                if self.flight.is_some()
                    && self.survey.head.is_some()
                    && now_ms >= self.last_line_ms.saturating_add(QUIET_MS)
                {
                    self.decide_survey(now_ms, picture);
                }
            }
            Step::Claim => {
                if let Some((_, _, at)) = self.foreign_claim
                    && !self.claim_rechecked
                    && now_ms >= at.saturating_add(CLAIM_RECHECK_MS)
                {
                    self.claim_rechecked = true;
                    self.ask(now_ms, LobbyLine::SimTeam(self.side));
                }
            }
            Step::Start => {
                if let Some(since) = self.empty_since
                    && self.waiting_other
                    && now_ms >= since.saturating_add(self.settings.wait_start_s * 1_000)
                {
                    let sentence = format!(
                        "Nobody claimed the other side within {} s.",
                        self.settings.wait_start_s
                    );
                    return self.block(now_ms, &sentence, picture);
                }
                let after = self.claim_ms.max(self.last_other_ms);
                if self.want.is_none()
                    && self.flight.is_none()
                    && !self.waiting_other
                    && now_ms >= after.saturating_add(self.settings.lead_start_grace_ms)
                {
                    self.ask(now_ms, LobbyLine::SimStart);
                }
            }
            Step::Reboard => {
                if !self.reboard_room_first && now_ms >= self.reboard_next && self.want.is_none() {
                    self.flight = None;
                    self.want = Some((LobbyLine::Board, now_ms));
                }
            }
            Step::Fight => {
                // Aboard a dark tank mid-match (the catch-up block or a `STATUS`
                // read `power off`, or an order came back `Power not online`):
                // start it again, or the lane types nothing until the match ends.
                if picture.aboard
                    && picture.own.power == Power::Off
                    && !picture.own.destroyed
                    && !self.hatch_seen
                {
                    self.step = Step::Startup;
                    self.ask(now_ms, LobbyLine::Startup);
                }
            }
            Step::Resume => {
                if now_ms >= self.resume_at {
                    if picture.own.power == Power::Off && !picture.own.destroyed {
                        self.step = Step::Startup;
                        self.ask(now_ms, LobbyLine::Startup);
                    } else {
                        self.step = Step::Fight;
                    }
                }
            }
            Step::Over => {
                if let Some(at) = self.rematch_at
                    && now_ms >= at
                {
                    self.rematch_at = None;
                    self.step = Step::Claim;
                    self.ask(now_ms, LobbyLine::SimTeam(self.side));
                }
            }
            _ => {}
        }
    }

    /// **Stop**: record a match the pilot had not finished as `blocked`, and
    /// queue the one `SIM LEAVE` when the centre was last read idle or in
    /// selection and the pilot holds a slot. Idempotent.
    ///
    /// **A claim or a start in flight** (cut AM.2, 16c Q-AM3): a `SIM TEAM`
    /// or a `SIM START` typed and not yet answered is kept, and the leave
    /// waits for its answer (`Leave::AwaitFlight`) until its typing and
    /// `[arena].step_timeout_ms`; then one `SIM LEAVE` whatever it learned.
    /// Only a stop waits: a blocker and a match's end close as before.
    pub fn stop(&mut self, now_ms: u64, picture: &Picture) {
        if self.closed {
            return;
        }
        let flight = self.flight;
        self.close(now_ms, picture);
        if let Some(flight) = flight
            && matches!(flight.line, LobbyLine::SimTeam(_) | LobbyLine::SimStart)
        {
            self.flight = Some(flight);
            self.leave = Leave::AwaitFlight {
                line: flight.line,
                until: flight.sent_ms.saturating_add(self.settings.step_timeout_ms),
            };
            self.retries = 0;
            self.say(format!(
                "Stopping: waiting for the answer to `{}` before giving the slot up.",
                flight.line.line()
            ));
        }
    }

    /// **The lane pauses** (cut AM.2): every line read from now is read at
    /// `at`. Idempotent.
    pub(crate) fn pause(&mut self, at: u64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(at);
        }
    }

    /// **The lane resumes** at `now_ms` (cut AM.2): every lobby wait moves on
    /// by the pause's length ([`Lobby::shift`]), so no step lapses while
    /// paused and a clock set by a line read then starts from now.
    pub(crate) fn resume(&mut self, now_ms: u64) {
        if let Some(at) = self.paused_at.take() {
            self.shift(now_ms.saturating_sub(at));
        }
    }

    /// The pause is over without a shift (cut AM.2): a stop's `SIM LEAVE`
    /// runs on the real clock.
    pub(crate) fn unpause(&mut self) {
        self.paused_at = None;
    }

    /// **Every lobby wait moves on by `ms`** (cut AM.2): the line in flight's
    /// typing, the waiting step's deadline, the wanted line's instant, the
    /// rematch, the re-board and resume cadence, the empty side's and the busy
    /// centre's waits, the lead's grace (the claim and the last claim by
    /// anybody else) and the quiet rule's last line. Never the match's own
    /// times (`running_ms`, `reboard_since`, `boarding_ms`) nor the leave.
    fn shift(&mut self, ms: u64) {
        let on = |at: u64| at.saturating_add(ms);
        if let Some(flight) = self.flight.as_mut() {
            flight.sent_ms = on(flight.sent_ms);
        }
        if let Some((at, _)) = self.deadline.as_mut() {
            *at = on(*at);
        }
        if let Some((_, at)) = self.want.as_mut() {
            *at = on(*at);
        }
        self.rematch_at = self.rematch_at.map(on);
        self.reboard_next = on(self.reboard_next);
        self.resume_at = on(self.resume_at);
        self.empty_since = self.empty_since.map(on);
        self.busy_since = self.busy_since.map(on);
        self.claim_ms = on(self.claim_ms);
        self.last_other_ms = on(self.last_other_ms);
        self.last_line_ms = on(self.last_line_ms);
    }

    // -- internals ----------------------------------------------------------------

    /// **The real time** (cut AM.2): `now_ms` but for a line read while
    /// paused, which `on_line` reads at the pause instant. The match's own
    /// times (its boarding, its running, a drop's time away, a record's
    /// length) are the centre's and run on the real clock: they are stamped
    /// with this and never shifted, so a boarding announced mid-pause keeps
    /// the window the centre gave it.
    fn real_ms(&self, now_ms: u64) -> u64 {
        self.read_ms.max(now_ms)
    }

    /// The followed match is running: the only time the pod's `STARTUP` is
    /// typed.
    fn followed_running(picture: &Picture) -> bool {
        matches!(picture.phase, Phase::Running { match_id } if picture.following == Some(match_id))
    }

    /// `name` is where the walk's line in flight goes: any room for `look`,
    /// the Welcome Hall for `ooc`, the Simulator Lobby for `simulator`.
    fn walk_arrives(&self, name: &str) -> bool {
        match self.flight.map(|f| f.line) {
            Some(LobbyLine::Look) => true,
            Some(LobbyLine::Ooc) => name == "Welcome Hall",
            Some(LobbyLine::Simulator) => name == "Simulator Lobby",
            _ => false,
        }
    }

    /// **The answer a stop waits for** (cut AM.2), while closed with
    /// [`Leave::AwaitFlight`]: any answer to the claim or the start queues the
    /// one `SIM LEAVE` (the pilot's own claim records its slot first); the
    /// other side's `You already hold …` ends the leave with nothing typed;
    /// every other line changes nothing.
    fn await_flight(&mut self, flown: LobbyLine, line: &Line, picture: &Picture) {
        let answered = match (flown, line) {
            (_, Line::Transient(_)) => true,
            (LobbyLine::SimTeam(_), Line::Claimed { name, side, index })
                if *side == self.side && self.is_me(name) =>
            {
                self.slot = Some(*index);
                self.last_slot = self.slot;
                true
            }
            (LobbyLine::SimTeam(_), Line::AlreadyHold { side, index }) if *side == self.side => {
                self.slot = Some(*index);
                self.last_slot = self.slot;
                true
            }
            (LobbyLine::SimTeam(_), Line::AlreadyHold { .. }) => {
                // The claim took nothing, and the slot held is not this
                // run's to give up.
                self.flight = None;
                self.leave = Leave::Done;
                return;
            }
            (
                LobbyLine::SimTeam(_),
                Line::SideFull { .. }
                | Line::WrongCentreState { .. }
                | Line::NoCentre
                | Line::LayerOff
                | Line::OutsideCentre
                | Line::NobodyPlaying,
            ) => true,
            (LobbyLine::SimStart, line) if Self::followed_boarding(line, picture).is_some() => true,
            (
                LobbyLine::SimStart,
                Line::NotLead
                | Line::EmptySide { .. }
                | Line::WrongCentreState { .. }
                | Line::NobodyPlaying,
            ) => true,
            _ => false,
        };
        if answered {
            self.flight = None;
            self.leave = Leave::Queued;
            self.retries = 0;
        }
    }

    fn window(&mut self, side: Side) -> &mut Option<Vec<String>> {
        match side {
            Side::Red => &mut self.windows[0],
            Side::Blue => &mut self.windows[1],
        }
    }

    fn is_me(&self, name: &str) -> bool {
        fold(name) == fold(&self.me)
    }

    fn say(&mut self, text: String) {
        self.events.push(LaneEvent::Said(text));
    }

    /// Want `line` now, as a fresh line (its transient count reset).
    fn ask(&mut self, now_ms: u64, line: LobbyLine) {
        self.flight = None;
        self.retries = 0;
        self.want = Some((line, now_ms));
    }

    /// The lead as the window reads it; says so when it turns true after the
    /// claim, and moves a waiting non-lead to `Start`.
    fn recompute_lead(&mut self, now_ms: u64) {
        let me = fold(&self.me);
        let side = self.side;
        let lead = self.slot.is_some()
            && self
                .window(side)
                .as_ref()
                .is_some_and(|names| names.first() == Some(&me));
        if lead == self.lead {
            return;
        }
        self.lead = lead;
        if lead {
            if matches!(self.step, Step::Start | Step::Await) {
                self.say(format!("Now leading the {} side.", side.as_str()));
                if self.step == Step::Await {
                    self.step = Step::Start;
                    self.deadline = None;
                    self.last_other_ms = self.last_other_ms.max(now_ms);
                }
            }
            return;
        }
        // The lead lost: a start wanted and not yet typed goes, whatever is in
        // flight (cut AM.2).
        if matches!(self.want, Some((LobbyLine::SimStart, _))) {
            self.want = None;
        }
        if self.step == Step::Start && self.flight.is_none() {
            self.await_start(now_ms);
        }
    }

    /// The centre's own lines, in every step: its state, the windows, the slot.
    /// A match line about a match other than the followed one is somebody
    /// else's history once the pilot follows a match, and moves nothing.
    fn centre_line(&mut self, now_ms: u64, line: &Line, picture: &Picture) {
        let ours = |id: &u64| picture.following.is_none_or(|followed| followed == *id);
        match line {
            Line::Board { state, .. } => self.centre = Centre::from_word(state),
            Line::BoardCleared => {
                self.windows = [Some(Vec::new()), Some(Vec::new())];
                self.centre = Centre::Selection;
            }
            Line::Over { match_id, .. }
            | Line::Aborted { match_id, .. }
            | Line::CouldNotStart { match_id, .. }
            | Line::AllLeft { match_id }
                if ours(match_id) =>
            {
                // A match's end releases every slot and every lead.
                self.windows = [Some(Vec::new()), Some(Vec::new())];
                self.centre = Centre::Terminal;
                self.slot = None;
                self.lead = false;
                // A busy survey need not wait out its ten seconds.
                if self.step == Step::Survey && self.flight.is_none() {
                    self.want = Some((LobbyLine::SimStatus, now_ms.saturating_add(1_000)));
                }
            }
            Line::Boarding { match_id } if ours(match_id) => self.centre = Centre::Boarding,
            Line::Running { match_id } if ours(match_id) => self.centre = Centre::Running,
            Line::Claimed { name, side, .. } => {
                let folded = fold(name);
                if let Some(names) = self.window(*side)
                    && !names.contains(&folded)
                {
                    names.push(folded);
                }
                if matches!(
                    self.centre,
                    Centre::Idle | Centre::Terminal | Centre::Unknown
                ) {
                    self.centre = Centre::Selection;
                }
                if !self.is_me(name) {
                    self.last_other_ms = now_ms;
                }
                self.recompute_lead(now_ms);
            }
            Line::GaveUp { name, side, index } => {
                let folded = fold(name);
                if let Some(names) = self.window(*side) {
                    names.retain(|n| *n != folded);
                }
                if self.is_me(name) && *side == self.side {
                    self.slot = None;
                    self.leave = Leave::Done;
                    self.say(format!("Gave up {} slot {index}.", side.as_str()));
                } else {
                    self.last_other_ms = now_ms;
                }
                self.recompute_lead(now_ms);
            }
            _ => {}
        }
    }

    /// A transient answer: the same line again after 200 ms, at most 40 times.
    ///
    /// **A busy `STARTUP` never gives a running match up** (cut AO.2): past
    /// the forty, `STARTUP` in the running followed match is typed again after
    /// `[arena].step_timeout_ms` — as the step timeout already types it again —
    /// with its count reset; every other line blocks.
    fn transient(&mut self, now_ms: u64, picture: &Picture) {
        let Some(flight) = self.flight else {
            return;
        };
        if self.retries >= TRANSIENT_RETRIES
            && flight.line == LobbyLine::Startup
            && Self::followed_running(picture)
        {
            let wait = self.settings.step_timeout_ms;
            self.say(format!(
                "The server kept answering `STARTUP` as busy; typing it again in {} s.",
                wait / 1_000
            ));
            self.retries = 0;
            self.flight = None;
            self.want = Some((LobbyLine::Startup, now_ms.saturating_add(wait)));
            return;
        }
        if self.retries >= TRANSIENT_RETRIES {
            let sentence = format!(
                "The server kept answering `{}` as busy.",
                flight.line.line()
            );
            return self.block(now_ms, &sentence, picture);
        }
        self.retries += 1;
        self.flight = None;
        self.want = Some((flight.line, now_ms.saturating_add(TRANSIENT_RETRY_MS)));
    }

    /// A transient answer while `SIM LEAVE` waits: the same line after 200 ms,
    /// at most 40 times, and only inside its 3 s.
    fn leave_again(&mut self, now_ms: u64) {
        let Leave::Sent(first) = self.leave else {
            return;
        };
        let at = now_ms.saturating_add(TRANSIENT_RETRY_MS);
        if self.retries >= TRANSIENT_RETRIES || at >= first.saturating_add(LEAVE_WAIT_MS) {
            return;
        }
        self.retries += 1;
        self.leave = Leave::Again { first, at };
    }

    /// The followed match's end, from any step: record it once, then the
    /// rematch or the end.
    fn match_end(&mut self, now_ms: u64, picture: &Picture) -> bool {
        let Phase::Over { match_id, result } = &picture.phase else {
            return false;
        };
        if picture.following != Some(*match_id) || self.ended == Some(*match_id) {
            return false;
        }
        let match_id = *match_id;
        self.ended = Some(match_id);
        self.flight = None;
        self.want = None;
        self.deadline = None;
        if self.recorded != Some(match_id) {
            self.record(now_ms, picture, match_id, result.as_str(), true);
        }
        self.after_match(now_ms, picture);
        true
    }

    /// After a recorded match: the next claim a second after the end line, or
    /// the end.
    fn after_match(&mut self, now_ms: u64, picture: &Picture) {
        let flown = u32::try_from(self.matches.len()).unwrap_or(u32::MAX);
        if flown >= self.settings.matches {
            self.step = Step::Finished;
            self.close(now_ms, picture);
            return;
        }
        self.step = Step::Over;
        // A tank left for good: the next claim waits for the match to end.
        self.rematch_at = self
            .ended
            .is_some_and(|id| self.recorded == Some(id))
            .then(|| now_ms.saturating_add(REMATCH_AFTER_MS));
    }

    /// Record the followed match under `word`. `over` is whether the centre
    /// ended it: a stop or a tank left for good records a match that is still
    /// running, and the sentence says so rather than calling it over.
    fn record(&mut self, now_ms: u64, picture: &Picture, match_id: u64, word: &str, over: bool) {
        self.recorded = Some(match_id);
        let callsign = picture
            .own
            .callsign
            .clone()
            .or_else(|| self.callsign())
            .unwrap_or_default();
        // A match recorded while paused keeps its real length (cut AM.2).
        let end = self.real_ms(now_ms);
        let duration = self.running_ms.map_or(0, |at| end.saturating_sub(at));
        let result = MatchResult::from_picture(
            picture,
            self.side,
            match_id,
            callsign,
            self.lead_at_boarding,
            word,
            self.reclaims,
            self.away_ms,
            duration,
        );
        let tally = format!(
            "{} shot(s), {} hit(s), {} kill(s); took {} hit(s).",
            result.shots, result.hits_scored, result.kills, result.hits_taken
        );
        self.say(if over {
            format!(
                "Match {match_id} is over ({word}): {}; {tally}",
                result.verdict(self.side)
            )
        } else {
            format!("Stopped flying match {match_id} before its end ({word}); {tally}")
        });
        self.matches.push(result);
    }

    /// Stop here: the unfinished match is `blocked`, and `SIM LEAVE` is queued
    /// when it is due.
    fn close(&mut self, now_ms: u64, picture: &Picture) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(id) = picture.following
            && self.ended != Some(id)
            && self.recorded != Some(id)
            && self.slot.is_some()
        {
            self.record(now_ms, picture, id, "blocked", false);
        }
        if self.step != Step::Finished {
            self.step = Step::Finished;
        }
        self.want = None;
        self.flight = None;
        self.deadline = None;
        if matches!(self.centre, Centre::Idle | Centre::Selection)
            && self.slot.is_some()
            && self.leave == Leave::No
        {
            self.leave = Leave::Queued;
            self.retries = 0;
        }
    }

    fn block(&mut self, now_ms: u64, sentence: &str, picture: &Picture) {
        if self.closed {
            return;
        }
        self.blocker = Some(sentence.to_owned());
        self.say(sentence.to_owned());
        self.close(now_ms, picture);
    }

    fn walk(&mut self, now_ms: u64, line: LobbyLine) {
        self.step = Step::Walk;
        self.ask(now_ms, line);
    }

    fn survey(&mut self, now_ms: u64, at_ms: u64) {
        self.step = Step::Survey;
        self.survey = Survey::default();
        self.flight = None;
        self.retries = 0;
        self.want = Some((LobbyLine::SimStatus, at_ms.max(now_ms)));
    }

    /// Not the lead (or not now): wait for somebody to start, up to
    /// `[arena].wait_start_s`. A start wanted and not yet typed goes (cut
    /// AM.2).
    fn await_start(&mut self, now_ms: u64) {
        if matches!(self.want, Some((LobbyLine::SimStart, _))) {
            self.want = None;
        }
        self.step = Step::Await;
        self.deadline = Some((
            now_ms.saturating_add(self.settings.wait_start_s * 1_000),
            format!(
                "Nobody started the match within {} s.",
                self.settings.wait_start_s
            ),
        ));
    }

    /// `look`'s own answer: the room heading decides the walk. A heading read
    /// before `look` is typed is not its answer (cut AM.2).
    fn on_look(&mut self, now_ms: u64, line: &Line) {
        let Line::Room(name) = line else {
            return;
        };
        if self.flight.map(|f| f.line) != Some(LobbyLine::Look) {
            return;
        }
        if is_centre_room(name) {
            self.survey(now_ms, now_ms);
        } else if name == "Welcome Hall" {
            self.walk(now_ms, LobbyLine::Simulator);
        } else {
            self.walk(now_ms, LobbyLine::Ooc);
        }
    }

    /// The walk's own room: `ooc` to the Welcome Hall, then `simulator` to the
    /// Simulator Lobby. Every other heading is ignored (cut AM.2: a late
    /// `look`'s answer, or a move the walk did not make); a walk that never
    /// arrives blocks by the quiet rule in [`Lobby::poll`] alone.
    fn on_walk(&mut self, now_ms: u64, line: &Line) {
        let Line::Room(name) = line else {
            return;
        };
        match (self.flight.map(|f| f.line), name.as_str()) {
            (Some(LobbyLine::Ooc), "Welcome Hall") => self.walk(now_ms, LobbyLine::Simulator),
            (Some(LobbyLine::Simulator), "Simulator Lobby") => self.survey(now_ms, now_ms),
            _ => {}
        }
    }

    fn on_survey(&mut self, now_ms: u64, raw: &str, line: &Line, picture: &mut Picture) {
        if self.flight.is_none() {
            return;
        }
        match line {
            Line::Board {
                state, match_id, ..
            } => self.survey.head = Some((state.clone(), *match_id)),
            Line::BoardSide { side, pilots } => {
                match side {
                    Side::Red => self.survey.red = Some(pilots.clone()),
                    Side::Blue => self.survey.blue = Some(pilots.clone()),
                }
                if self.survey.head.is_some()
                    && self.survey.red.is_some()
                    && self.survey.blue.is_some()
                {
                    self.decide_survey(now_ms, picture);
                }
            }
            Line::NoCentre => self.block(now_ms, "This server has no simulator centre.", picture),
            Line::LayerOff => self.block(now_ms, raw.trim(), picture),
            Line::OutsideCentre => self.block(now_ms, UNREACHABLE, picture),
            _ => {}
        }
    }

    /// What a `SIM STATUS` said: claim, board, take the tank back, or wait.
    fn decide_survey(&mut self, now_ms: u64, picture: &mut Picture) {
        self.flight = None;
        let Some((state, match_id)) = self.survey.head.clone() else {
            return;
        };
        let centre = Centre::from_word(&state);
        self.centre = centre;
        let row = self.survey.row(self.side).cloned();
        let mine = row
            .as_ref()
            .and_then(|pilots| pilots.iter().find(|p| self.is_me(&p.name)).cloned());
        match centre {
            Centre::Idle | Centre::Selection | Centre::Terminal => {
                // The row names who stands ahead of the pilot's claim: the pilot
                // leads only once every one of them has given the slot up.
                if let Some(pilots) = &row {
                    let names = pilots.iter().map(|p| fold(&p.name)).collect();
                    *self.window(self.side) = Some(names);
                }
                self.busy_since = None;
                self.step = Step::Claim;
                self.foreign_claim = None;
                self.claim_rechecked = false;
                self.ask(now_ms, LobbyLine::SimTeam(self.side));
                return;
            }
            Centre::Boarding => {
                if let (Some(mine), Some(id)) = (&mine, match_id) {
                    self.slot = Some(mine.index);
                    self.last_slot = self.slot;
                    picture.side = Some(self.side);
                    picture.following = Some(id);
                    picture.phase = Phase::Boarding { match_id: id };
                    self.boarding_ms = self.real_ms(now_ms);
                    self.lead_at_boarding = false;
                    self.enter_team_room(now_ms, picture);
                    return;
                }
            }
            Centre::Running => {
                if let (Some(mine), Some(id)) = (&mine, match_id)
                    && mine.standing != Some(Standing::Left)
                {
                    // The tank is the machines' until `BOARD` takes it back.
                    self.slot = Some(mine.index);
                    self.last_slot = self.slot;
                    picture.side = Some(self.side);
                    picture.following = Some(id);
                    picture.phase = Phase::Running { match_id: id };
                    picture.aboard = false;
                    if picture.own.callsign.is_none() {
                        picture.own.callsign = Some(mine.callsign.clone());
                    }
                    self.running_ms = Some(self.real_ms(now_ms));
                    self.lead_at_boarding = false;
                    self.reboard(now_ms);
                    return;
                }
            }
            Centre::Unknown => {}
        }
        let since = *self.busy_since.get_or_insert(now_ms);
        if now_ms >= since.saturating_add(self.settings.wait_start_s * 1_000) {
            let sentence = format!(
                "The centre stayed busy for {} s.",
                self.settings.wait_start_s
            );
            return self.block(now_ms, &sentence, picture);
        }
        self.survey(now_ms, now_ms.saturating_add(SURVEY_EVERY_MS));
    }

    fn on_claim(&mut self, now_ms: u64, line: &Line, picture: &mut Picture) {
        match line {
            Line::Claimed { name, side, index } if *side == self.side => {
                if self.is_me(name) {
                    self.claimed(now_ms, *index, picture);
                } else if self.flight.is_some() || self.foreign_claim.is_none() {
                    self.foreign_claim = Some((name.clone(), *index, now_ms));
                }
            }
            Line::AlreadyHold { side, index } if *side == self.side => {
                // The other name on this very slot was the pilot's own: the
                // centre prints the in-game name, which is the one to use.
                if let Some((name, held, _)) = self.foreign_claim.clone()
                    && held == *index
                {
                    self.me = name;
                }
                self.claimed(now_ms, *index, picture);
            }
            Line::AlreadyHold { .. } => self.block(
                now_ms,
                "The bot already holds a slot on the other side.",
                picture,
            ),
            Line::SideFull { side, .. } => {
                let sentence = format!("The {} side is full.", side.as_str());
                self.block(now_ms, &sentence, picture);
            }
            Line::WrongCentreState { .. } => {
                let since = *self.busy_since.get_or_insert(now_ms);
                if now_ms >= since.saturating_add(self.settings.wait_start_s * 1_000) {
                    let sentence = format!(
                        "The centre stayed busy for {} s.",
                        self.settings.wait_start_s
                    );
                    return self.block(now_ms, &sentence, picture);
                }
                self.survey(now_ms, now_ms.saturating_add(SURVEY_EVERY_MS));
            }
            _ => {}
        }
    }

    /// The pilot holds `side` slot `index`.
    fn claimed(&mut self, now_ms: u64, index: u8, picture: &mut Picture) {
        self.flight = None;
        self.want = None;
        self.slot = Some(index);
        self.last_slot = self.slot;
        self.centre = Centre::Selection;
        self.claim_ms = now_ms;
        self.last_other_ms = now_ms;
        self.busy_since = None;
        self.empty_since = None;
        self.waiting_other = false;
        self.foreign_claim = None;
        self.claim_rechecked = false;
        self.running_ms = None;
        self.hatch_seen = false;
        self.reclaims = 0;
        self.away_ms = 0;
        self.board_room_tries = 0;
        self.leave = Leave::No;
        picture.claimed(self.side, index);
        let me = fold(&self.me);
        let side = self.side;
        if let Some(names) = self.window(side)
            && !names.contains(&me)
        {
            names.push(me.clone());
        }
        let lead = self
            .window(side)
            .as_ref()
            .is_some_and(|names| names.first() == Some(&me));
        self.lead = lead;
        let cs = callsign(side, index);
        self.events.push(LaneEvent::Claimed);
        if lead {
            self.say(format!(
                "Claimed {} slot {index} ({cs}), leading the side.",
                side.as_str()
            ));
            self.step = Step::Start;
            self.deadline = None;
        } else {
            self.say(format!("Claimed {} slot {index} ({cs}).", side.as_str()));
            self.await_start(now_ms);
        }
    }

    /// `Match {n} is boarding.` for the followed match.
    fn boarding(&mut self, now_ms: u64, raw: &str, match_id: u64, picture: &Picture) {
        self.flight = None;
        self.want = None;
        self.deadline = None;
        self.waiting_other = false;
        self.lead_at_boarding = self.lead;
        self.boarding_ms = self.real_ms(now_ms);
        self.board_window_ms = boarding_window_ms(raw);
        self.say(format!(
            "Match {match_id} is boarding; going to the {} pod.",
            self.side.as_str()
        ));
        self.enter_team_room(now_ms, picture);
    }

    fn followed_boarding(line: &Line, picture: &Picture) -> Option<u64> {
        match line {
            Line::Boarding { match_id } if picture.following == Some(*match_id) => Some(*match_id),
            _ => None,
        }
    }

    fn on_start(&mut self, now_ms: u64, raw: &str, line: &Line, picture: &Picture) {
        if let Some(id) = Self::followed_boarding(line, picture) {
            return self.boarding(now_ms, raw, id, picture);
        }
        match line {
            Line::EmptySide { side } if *side == self.side.other() => {
                self.flight = None;
                self.waiting_other = true;
                self.empty_since.get_or_insert(now_ms);
            }
            Line::Claimed { side, .. } if *side == self.side.other() => {
                self.waiting_other = false;
            }
            Line::NotLead => {
                // The window was wrong: close it, and wait for somebody else.
                *self.window(self.side) = None;
                self.lead = false;
                self.flight = None;
                self.await_start(now_ms);
            }
            Line::WrongCentreState { .. } => {
                self.flight = None;
                self.await_start(now_ms);
            }
            _ => {}
        }
    }

    fn on_await(&mut self, now_ms: u64, raw: &str, line: &Line, picture: &Picture) {
        if let Some(id) = Self::followed_boarding(line, picture) {
            self.boarding(now_ms, raw, id, picture);
        }
    }

    /// Into the team room, unless already there or in one of the side's pods.
    fn enter_team_room(&mut self, now_ms: u64, picture: &Picture) {
        let room = picture.room.as_deref().unwrap_or("");
        let team_room = match self.side {
            Side::Red => "Red Room",
            Side::Blue => "Blue Room",
        };
        let in_pod = pod_number(room).is_some_and(|(side, _)| side == self.side);
        if room == team_room || in_pod {
            self.step = Step::Board;
            self.ask(now_ms, LobbyLine::Board);
        } else {
            self.step = Step::ToRoom;
            self.ask(now_ms, LobbyLine::TeamRoom(self.side));
        }
    }

    fn on_to_room(&mut self, now_ms: u64, line: &Line) {
        let team_room = match self.side {
            Side::Red => "Red Room",
            Side::Blue => "Blue Room",
        };
        if matches!(line, Line::Room(name) if name == team_room) {
            self.step = Step::Board;
            self.ask(now_ms, LobbyLine::Board);
        }
    }

    /// `BOARD`'s answers: aboard, the wait for the running line; outside the
    /// team room, the walk there (at most [`BOARD_ROOM_TRIES`] times).
    ///
    /// **A start without us** (cut AO.2): the game drops a claimed slot nobody
    /// seated at the boarding deadline (`{name} never boarded; {side} slot {i}
    /// is dropped.`), so a `BOARD` typed after a running match began — a
    /// pause that spanned the boarding window and the start — is answered
    /// `You hold no slot in the simulator.`. That match is lost to this run:
    /// recorded `left`, as the `no longer in the match` path records it, and
    /// the next one claimed after its end (or, the last one asked for, the run
    /// ends). Outside a running followed match the same answer is left to the
    /// step timeout, as before.
    fn on_board(&mut self, now_ms: u64, line: &Line, picture: &Picture) {
        match line {
            Line::NoSlot if Self::followed_running(picture) => {
                self.flight = None;
                self.want = None;
                self.deadline = None;
                self.slot = None;
                // The end is waited for only when another match follows it:
                // the last one asked ends the run here.
                let flown = self
                    .matches
                    .len()
                    .saturating_add(usize::from(picture.following.is_some()));
                let more = u32::try_from(flown).unwrap_or(u32::MAX) < self.settings.matches;
                let Some(id) = picture.following else {
                    return;
                };
                self.say(if more {
                    format!("Match {id} started without us; waiting for it to end.")
                } else {
                    format!("Match {id} started without us.")
                });
                self.record(now_ms, picture, id, "left", false);
                self.after_match(now_ms, picture);
            }
            Line::ClimbIn { .. } | Line::AlreadyAboard => {
                self.flight = None;
                self.step = Step::AwaitRunning;
                let bound = self
                    .board_window_ms
                    .map(|window| {
                        self.boarding_ms
                            .saturating_add(window)
                            .saturating_add(BOARDING_SLACK_MS)
                    })
                    .unwrap_or_else(|| now_ms.saturating_add(self.settings.wait_start_s * 1_000));
                self.deadline = Some((
                    bound.max(now_ms),
                    format!(
                        "The match did not start within {} s.",
                        bound.saturating_sub(self.boarding_ms.min(bound)) / 1_000
                    ),
                ));
            }
            Line::OutsideTeamRoom => {
                self.board_room_tries += 1;
                if self.board_room_tries > BOARD_ROOM_TRIES {
                    return self.block(now_ms, UNREACHABLE, picture);
                }
                self.step = Step::ToRoom;
                self.ask(now_ms, LobbyLine::TeamRoom(self.side));
            }
            _ => {}
        }
    }

    fn on_await_running(&mut self, now_ms: u64, line: &Line, picture: &Picture) {
        if let Line::Running { match_id } = line
            && picture.following == Some(*match_id)
        {
            self.deadline = None;
            self.running_ms = Some(self.real_ms(now_ms));
            self.say(format!("Match {match_id} is running; starting the tank."));
            self.step = Step::Startup;
            self.ask(now_ms, LobbyLine::Startup);
        }
    }

    /// **`STARTUP`'s every answer** (cut AM.2): begun or still coming up, the
    /// power wait; online, the fight; the dark panel, again in a second; the
    /// tank destroyed or a wreck, the fight's step with nothing to type until
    /// the match's end; the tank no longer in the match, the match recorded
    /// `left` and its end waited for when another match follows (the last one
    /// asked ends the run there); any other refusal, again after
    /// `[arena].step_timeout_ms`; a drop, the re-board.
    fn on_power(&mut self, now_ms: u64, line: &Line, picture: &Picture) {
        match line {
            Line::DestroyedNoOrders => self.wrecked(),
            Line::Refused { order, said, .. }
                if order == "STARTUP" && said.starts_with("Your tank is a wreck") =>
            {
                self.wrecked()
            }
            Line::Refused { order, said, .. }
                if order == "STARTUP"
                    && said.starts_with("That tank is no longer in the match") =>
            {
                self.flight = None;
                self.want = None;
                self.deadline = None;
                // The end is waited for only when another match follows it:
                // the last one asked ends the run here.
                let flown = self
                    .matches
                    .len()
                    .saturating_add(usize::from(picture.following.is_some()));
                let more = u32::try_from(flown).unwrap_or(u32::MAX) < self.settings.matches;
                self.say(if more {
                    "The tank is no longer in the match; waiting for it to end.".to_owned()
                } else {
                    "The tank is no longer in the match.".to_owned()
                });
                if let Some(id) = picture.following {
                    self.record(now_ms, picture, id, "left", false);
                }
                self.after_match(now_ms, picture);
            }
            Line::Refused { order, said, .. } if order == "STARTUP" => {
                let wait = self.settings.step_timeout_ms;
                self.flight = None;
                self.deadline = None;
                self.step = Step::Startup;
                self.retries = 0;
                self.want = Some((LobbyLine::Startup, now_ms.saturating_add(wait)));
                self.say(format!(
                    "STARTUP was refused (`{said}`); typing it again in {} s.",
                    wait / 1_000
                ));
            }
            Line::StartupBegun | Line::PowerStarting if self.step == Step::Startup => {
                self.flight = None;
                self.step = Step::AwaitPower;
                self.deadline = Some((
                    now_ms.saturating_add(POWER_WAIT_MS),
                    format!(
                        "Power did not come online within {} s.",
                        POWER_WAIT_MS / 1_000
                    ),
                ));
            }
            Line::PowerAlreadyOnline | Line::PowerOnline => self.fight(),
            Line::NotStartedYet if self.step == Step::Startup => {
                self.flight = None;
                self.want = Some((LobbyLine::Startup, now_ms.saturating_add(1_000)));
            }
            _ if self.dropped(line, picture) => self.reboard(now_ms),
            _ => {}
        }
    }

    fn fight(&mut self) {
        self.flight = None;
        self.want = None;
        self.deadline = None;
        self.step = Step::Fight;
        self.say("Power online; fighting.".to_owned());
    }

    /// The tank is destroyed (or a wreck): the fight's step, typing nothing
    /// (the picture marks it destroyed, so [`Step::Fight`] never starts it
    /// again), until the match's end records the centre's own word (cut
    /// AM.2).
    fn wrecked(&mut self) {
        self.flight = None;
        self.want = None;
        self.deadline = None;
        self.step = Step::Fight;
        self.say("The tank is destroyed; waiting for the match to end.".to_owned());
    }

    /// `OUTSIDE_COCKPIT`, `RESUME_LINE` or the reconnect line naming the own
    /// tank (`… is flying on its mind; BOARD takes it back.`, the game's 17b cut
    /// M5.3): the pilot is out of it.
    fn dropped(&self, line: &Line, picture: &Picture) -> bool {
        match line {
            Line::OutsideCockpit => !self.hatch_seen,
            Line::Resumed => true,
            Line::OnItsMind { tank } => picture.own.name.as_deref().is_none_or(|own| own == tank),
            _ => false,
        }
    }

    fn on_fight(&mut self, now_ms: u64, line: &Line, picture: &Picture) {
        if matches!(line, Line::HatchOpen { .. }) {
            self.hatch_seen = true;
        } else if self.dropped(line, picture) {
            self.reboard(now_ms);
        }
    }

    fn reboard(&mut self, now_ms: u64) {
        self.step = Step::Reboard;
        self.deadline = None;
        self.reboard_since = self.real_ms(now_ms);
        self.reboard_next = now_ms;
        self.reboard_room_first = false;
        self.back_away_ms = None;
        self.retries = 0;
        self.flight = None;
        self.want = Some((LobbyLine::Board, now_ms));
        self.say("Out of the tank; boarding again.".to_owned());
    }

    fn on_reboard(&mut self, now_ms: u64, line: &Line, picture: &Picture) {
        match line {
            Line::HandingOver { secs, .. } => {
                self.flight = None;
                self.reboard_next = now_ms.saturating_add(u64::from(*secs) * 1_000);
            }
            Line::OutsideTeamRoom => {
                self.reboard_room_first = true;
                self.ask(now_ms, LobbyLine::TeamRoom(self.side));
            }
            Line::Room(name) if self.reboard_room_first => {
                let team_room = match self.side {
                    Side::Red => "Red Room",
                    Side::Blue => "Blue Room",
                };
                if name == team_room {
                    self.reboard_room_first = false;
                    self.ask(now_ms, LobbyLine::Board);
                }
            }
            Line::LeftForGood { .. } => {
                self.flight = None;
                self.want = None;
                if let Some(id) = picture.following {
                    self.record(now_ms, picture, id, "left", false);
                }
                self.after_match(now_ms, picture);
            }
            Line::BackAboard { away_ms, .. } => {
                self.back_away_ms = Some(*away_ms);
                self.back_aboard(now_ms, false);
            }
            Line::PowerOnline => self.back_aboard(now_ms, true),
            Line::AlreadyAboard | Line::ClimbBack { .. } | Line::BackAtStation { .. } => {
                self.back_aboard(now_ms, false)
            }
            _ => {}
        }
    }

    /// Taken back. Only `Power online.` itself says the power is up: every other
    /// success line comes before the catch-up block's `  Power {now}.` row (or
    /// says nothing of the power), and the picture still holds the reading from
    /// before the drop — so the tank settles in [`Step::Resume`] first, and the
    /// power is read once the block has arrived.
    fn back_aboard(&mut self, now_ms: u64, powered: bool) {
        self.flight = None;
        self.want = None;
        self.reclaims = self.reclaims.saturating_add(1);
        let away = self
            .back_away_ms
            .unwrap_or_else(|| self.real_ms(now_ms).saturating_sub(self.reboard_since));
        self.away_ms = self.away_ms.saturating_add(away);
        self.say(format!(
            "Back aboard after {} s away.",
            away.saturating_add(500) / 1_000
        ));
        if powered {
            self.step = Step::Fight;
        } else {
            self.step = Step::Resume;
            self.resume_at = now_ms.saturating_add(RESUME_SETTLE_MS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::classify;

    /// A lobby and the picture it reads, on a fake clock.
    struct Rig {
        lobby: Lobby,
        picture: Picture,
        now: u64,
        typed: Vec<String>,
    }

    impl Rig {
        fn new(side: Side) -> Rig {
            Rig::with(side, Settings::default(), "Pilot")
        }

        fn with(side: Side, settings: Settings, me: &str) -> Rig {
            Rig {
                lobby: Lobby::new(side, settings, me),
                picture: Picture::default(),
                now: 0,
                typed: Vec::new(),
            }
        }

        fn hear(&mut self, raw: &str) {
            let line = classify(raw);
            self.picture.apply(self.now, &line);
            self.lobby.on_line(self.now, raw, &line, &mut self.picture);
        }

        /// Move the clock to `ms`, poll, and type what is due (one line).
        fn at(&mut self, ms: u64) -> Option<String> {
            self.now = ms;
            self.lobby.poll(self.now, &mut self.picture);
            let line = self.lobby.due(self.now)?;
            self.lobby.sent(self.now, line);
            self.typed.push(line.line());
            Some(line.line())
        }

        /// The next line typed at or after now, stepping 50 ms at a time.
        fn next(&mut self, within_ms: u64) -> Option<String> {
            let until = self.now + within_ms;
            let mut t = self.now;
            while t <= until {
                if let Some(line) = self.at(t) {
                    return Some(line);
                }
                t += 50;
            }
            None
        }

        fn said(&mut self) -> Vec<String> {
            self.lobby
                .take_events()
                .into_iter()
                .filter_map(|event| match event {
                    LaneEvent::Said(text) => Some(text),
                    _ => None,
                })
                .collect()
        }

        /// From Limbo to the lobby: `look`, `ooc`, `simulator`.
        fn walk_in(&mut self) {
            assert_eq!(self.at(0).as_deref(), Some("look"));
            self.hear("Limbo #0");
            self.hear("A grey nowhere.");
            assert_eq!(self.next(3_000).as_deref(), Some("ooc"));
            self.hear("Welcome Hall #2");
            assert_eq!(self.next(100).as_deref(), Some("simulator"));
            self.hear("Simulator Lobby #50");
            assert_eq!(self.next(100).as_deref(), Some("SIM STATUS"));
        }

        fn board(&mut self, state: &str, red: &str, blue: &str) {
            self.hear(&format!("Simulator centre: {state}"));
            self.hear(&format!("  red: {red}"));
            self.hear(&format!("  blue: {blue}"));
        }

        /// Walk in, read an idle board, claim red 1: the lead.
        fn claim_lead(&mut self) {
            self.walk_in();
            self.board("idle", "nobody", "nobody");
            assert_eq!(self.next(100).as_deref(), Some("SIM TEAM red"));
            self.hear("Pilot takes red slot 1. `SIM START` begins the match; the machines take the pods nobody claims.");
            assert!(self.lobby.lead());
        }

        /// Through the start, the board and the power-up, into the fight.
        fn fly(&mut self) {
            self.claim_lead();
            let t = self.now;
            assert_eq!(self.next(4_000).as_deref(), Some("SIM START"));
            assert!(self.now >= t + 3_000, "the grace");
            self.hear("Match 1 is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 30 second(s).");
            assert_eq!(self.next(100).as_deref(), Some("red"));
            self.hear("Red Room #57");
            assert_eq!(self.next(100).as_deref(), Some("BOARD"));
            self.hear("You climb into Red Pod 1. The panel wakes when every pilot is seated, or in 29 second(s).");
            self.hear("Red Pod 1 #60");
            assert_eq!(self.next(100), None);
            self.hear("Match 1 is running. Good hunting.");
            assert_eq!(self.next(100).as_deref(), Some("STARTUP"));
            self.hear("Startup sequence begun; power comes online shortly.");
            assert_eq!(self.lobby.step(), Step::AwaitPower);
            self.now += 1_000;
            self.hear("Power online.");
            assert_eq!(self.lobby.step(), Step::Fight);
        }

        /// Through the start and the board to `STARTUP` typed in the running
        /// match (cut AM.2); the time it was typed.
        fn up_to_startup(&mut self) -> u64 {
            self.claim_lead();
            assert_eq!(self.next(4_000).as_deref(), Some("SIM START"));
            self.hear("Match 1 is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 30 second(s).");
            assert_eq!(self.next(100).as_deref(), Some("red"));
            self.hear("Red Room #57");
            assert_eq!(self.next(100).as_deref(), Some("BOARD"));
            self.hear("You climb into Red Pod 1. The panel wakes when every pilot is seated.");
            self.hear("Match 1 is running. Good hunting.");
            assert_eq!(self.next(100).as_deref(), Some("STARTUP"));
            assert_eq!(self.lobby.step(), Step::Startup);
            self.now
        }

        fn types(&self, line: &str) -> usize {
            self.typed.iter().filter(|l| *l == line).count()
        }
    }

    const VOCABULARY: [&str; 12] = [
        "look",
        "ooc",
        "simulator",
        "red",
        "blue",
        "SIM STATUS",
        "SIM TEAM red",
        "SIM TEAM blue",
        "SIM START",
        "BOARD",
        "STARTUP",
        "SIM LEAVE",
    ];

    #[test]
    fn the_whole_walk_from_limbo_to_the_fight() {
        let mut rig = Rig::new(Side::Red);
        rig.fly();
        let said = rig.said();
        assert!(said.contains(&"Claimed red slot 1 (RA), leading the side.".to_owned()));
        assert!(said.contains(&"Match 1 is boarding; going to the red pod.".to_owned()));
        assert!(said.contains(&"Match 1 is running; starting the tank.".to_owned()));
        assert!(said.contains(&"Power online; fighting.".to_owned()));
        assert!(rig.lobby.fighting());
        assert_eq!(
            rig.typed,
            [
                "look",
                "ooc",
                "simulator",
                "SIM STATUS",
                "SIM TEAM red",
                "SIM START",
                "red",
                "BOARD",
                "STARTUP"
            ]
        );
    }

    #[test]
    fn from_the_hall_only_simulator_and_from_the_centre_nothing() {
        let mut rig = Rig::new(Side::Blue);
        assert_eq!(rig.at(0).as_deref(), Some("look"));
        rig.hear("Welcome Hall #2");
        assert_eq!(rig.next(100).as_deref(), Some("simulator"));
        let mut rig = Rig::new(Side::Blue);
        assert_eq!(rig.at(0).as_deref(), Some("look"));
        rig.hear("Blue Room #58");
        assert_eq!(rig.next(100).as_deref(), Some("SIM STATUS"));
    }

    #[test]
    fn an_ooc_that_does_not_reach_the_hall_blocks() {
        let mut rig = Rig::new(Side::Red);
        rig.at(0);
        rig.hear("Limbo #0");
        assert_eq!(rig.next(3_000).as_deref(), Some("ooc"));
        rig.hear("Huh?  (Type \"help\" for help.)");
        assert_eq!(rig.next(3_000), None);
        assert!(rig.lobby.finished());
        assert_eq!(rig.lobby.blocker(), Some(UNREACHABLE));
    }

    #[test]
    fn no_answer_within_the_step_timeout_blocks() {
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        let t = rig.now;
        rig.at(t + 14_999);
        assert!(!rig.lobby.finished());
        rig.at(t + 15_000);
        assert_eq!(
            rig.lobby.blocker(),
            Some("No answer to `SIM STATUS` within 15 s.")
        );
    }

    #[test]
    fn the_centre_refusals_block_with_their_sentences() {
        for (line, sentence) in [
            (
                "This server has no simulator centre.",
                "This server has no simulator centre.",
            ),
            (
                "The TacSim layer is off on this server, so the simulator cannot run.",
                "The TacSim layer is off on this server, so the simulator cannot run.",
            ),
        ] {
            let mut rig = Rig::new(Side::Red);
            rig.walk_in();
            rig.hear(line);
            assert_eq!(rig.lobby.blocker(), Some(sentence));
        }
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        rig.board("idle", "nobody", "nobody");
        rig.next(100);
        rig.hear("The red side is full: it takes 6 pilot(s).");
        assert_eq!(rig.lobby.blocker(), Some("The red side is full."));
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        rig.board("idle", "nobody", "1 (BA): Pilot");
        rig.next(100);
        rig.hear("You already hold blue slot 1. `SIM LEAVE` gives it up.");
        assert_eq!(
            rig.lobby.blocker(),
            Some("The bot already holds a slot on the other side.")
        );
    }

    #[test]
    fn a_busy_centre_is_read_again_every_ten_seconds_then_blocks() {
        let settings = Settings {
            wait_start_s: 30,
            ..Settings::default()
        };
        let mut rig = Rig::with(Side::Red, settings, "Pilot");
        rig.walk_in();
        rig.board("running (match 4)", "1 (RA): Ada", "1 (BA): Bo");
        let t = rig.now;
        assert_eq!(rig.next(9_900), None);
        assert_eq!(rig.next(200).as_deref(), Some("SIM STATUS"));
        assert!(rig.now >= t + SURVEY_EVERY_MS);
        for _ in 0..3 {
            rig.board("running (match 4)", "1 (RA): Ada", "1 (BA): Bo");
            rig.next(10_100);
        }
        assert_eq!(
            rig.lobby.blocker(),
            Some("The centre stayed busy for 30 s.")
        );
    }

    #[test]
    fn each_transient_resends_the_same_line_after_two_hundred_ms() {
        for transient in [
            "The command could not commit; please retry in a moment.",
            "The world is still starting; please try again shortly.",
            "That cockpit will not wake for you: control state unknown",
            "The arena is busy; try that again in a moment.",
        ] {
            let mut rig = Rig::new(Side::Red);
            rig.walk_in();
            let t = rig.now;
            rig.hear(transient);
            assert_eq!(rig.at(t + 199), None, "{transient}");
            assert_eq!(
                rig.at(t + 200).as_deref(),
                Some("SIM STATUS"),
                "{transient}"
            );
        }
    }

    #[test]
    fn forty_transients_running_block() {
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        for _ in 0..TRANSIENT_RETRIES {
            rig.hear("The arena is busy; try that again in a moment.");
            assert_eq!(rig.next(300).as_deref(), Some("SIM STATUS"));
        }
        rig.hear("The arena is busy; try that again in a moment.");
        assert_eq!(
            rig.lobby.blocker(),
            Some("The server kept answering `SIM STATUS` as busy.")
        );
    }

    #[test]
    fn the_first_claim_after_an_empty_row_leads() {
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        assert!(
            rig.said()
                .contains(&"Claimed red slot 1 (RA), leading the side.".to_owned())
        );
        assert_eq!(rig.lobby.step(), Step::Start);
    }

    #[test]
    fn another_claim_first_keeps_it_not_lead_until_that_name_gives_up() {
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        rig.board("idle", "nobody", "nobody");
        assert_eq!(rig.next(100).as_deref(), Some("SIM TEAM red"));
        rig.hear("Ada takes red slot 1. `SIM START` begins the match.");
        rig.hear("Pilot takes red slot 2. `SIM START` begins the match.");
        assert!(!rig.lobby.lead());
        assert_eq!(rig.lobby.step(), Step::Await);
        assert!(rig.said().contains(&"Claimed red slot 2 (RB).".to_owned()));
        // The non-lead never types `SIM START`.
        assert_eq!(rig.next(10_000), None);
        rig.hear("Ada gives up red slot 1.");
        assert!(rig.lobby.lead());
        assert!(rig.said().contains(&"Now leading the red side.".to_owned()));
        assert_eq!(rig.lobby.step(), Step::Start);
        assert_eq!(rig.next(4_000).as_deref(), Some("SIM START"));
    }

    #[test]
    fn a_filled_row_with_no_window_is_not_lead_until_they_leave() {
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        rig.board("selection", "1 (RA): Ada", "nobody");
        rig.next(100);
        rig.hear("Pilot takes red slot 2. `SIM START` begins the match.");
        assert!(!rig.lobby.lead());
        rig.hear("Bo takes blue slot 1. `SIM START` begins the match.");
        assert!(!rig.lobby.lead());
        rig.hear("Ada gives up red slot 1.");
        assert!(rig.lobby.lead());
    }

    #[test]
    fn no_window_means_not_lead() {
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        // A board whose row did not arrive whole: no window to read.
        rig.hear("Simulator centre: selection");
        rig.hear("  blue: nobody");
        assert_eq!(rig.next(1_000).as_deref(), Some("SIM TEAM red"));
        rig.hear("Pilot takes red slot 1. `SIM START` begins the match.");
        assert!(!rig.lobby.lead());
        assert_eq!(rig.lobby.step(), Step::Await);
        assert_eq!(rig.next(20_000), None);
    }

    #[test]
    fn a_refused_start_closes_the_window_and_waits() {
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        assert_eq!(rig.next(4_000).as_deref(), Some("SIM START"));
        rig.hear("Only a side's lead or staff can start the match.");
        assert!(!rig.lobby.lead());
        assert_eq!(rig.lobby.step(), Step::Await);
        rig.hear("Bo gives up blue slot 1.");
        assert!(!rig.lobby.lead(), "the window is closed");
        assert_eq!(rig.next(10_000), None);
    }

    #[test]
    fn the_grace_restarts_on_another_claim_or_give_up() {
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        let t = rig.now;
        rig.at(t + 2_000);
        rig.hear("Ada takes red slot 2. `SIM START` begins the match.");
        assert!(rig.lobby.lead(), "the first claimant still leads");
        assert_eq!(rig.next(2_900), None, "the grace restarted at t + 2 s");
        rig.hear("Bo takes blue slot 1. `SIM START` begins the match.");
        let u = rig.now;
        assert_eq!(rig.next(2_900), None);
        rig.hear("Bo gives up blue slot 1.");
        let v = rig.now;
        assert_eq!(rig.next(3_100).as_deref(), Some("SIM START"));
        assert!(rig.now >= v + 3_000 && v > u);
    }

    #[test]
    fn an_empty_other_side_waits_for_a_claim_there_then_blocks() {
        let settings = Settings {
            wait_start_s: 20,
            lead_start_grace_ms: 0,
            ..Settings::default()
        };
        let mut rig = Rig::with(Side::Red, settings.clone(), "Pilot");
        rig.claim_lead();
        assert_eq!(rig.next(100).as_deref(), Some("SIM START"));
        rig.hear("The blue side has no pilots; a match takes at least one a side.");
        assert_eq!(rig.next(5_000), None);
        rig.hear("Bo takes blue slot 1. `SIM START` begins the match.");
        assert_eq!(rig.next(100).as_deref(), Some("SIM START"), "retried");
        // Nobody ever comes: blocked after wait_start_s.
        let mut rig = Rig::with(Side::Red, settings, "Pilot");
        rig.claim_lead();
        rig.next(100);
        rig.hear("The blue side has no pilots; a match takes at least one a side.");
        rig.next(21_000);
        assert_eq!(
            rig.lobby.blocker(),
            Some("Nobody claimed the other side within 20 s.")
        );
    }

    #[test]
    fn a_non_lead_waits_for_somebody_to_start_then_blocks() {
        let settings = Settings {
            wait_start_s: 10,
            ..Settings::default()
        };
        let mut rig = Rig::with(Side::Red, settings, "Pilot");
        rig.walk_in();
        rig.board("selection", "1 (RA): Ada", "nobody");
        rig.next(100);
        rig.hear("Pilot takes red slot 2. `SIM START` begins the match.");
        rig.next(10_100);
        assert_eq!(
            rig.lobby.blocker(),
            Some("Nobody started the match within 10 s.")
        );
    }

    #[test]
    fn a_rematch_the_bot_claims_first_leads_though_a_human_led_match_one() {
        let mut rig = Rig::with(
            Side::Red,
            Settings {
                matches: 2,
                ..Settings::default()
            },
            "Pilot",
        );
        rig.walk_in();
        rig.board("selection", "1 (RA): Ada", "nobody");
        rig.next(100);
        rig.hear("Pilot takes red slot 2. `SIM START` begins the match.");
        assert!(!rig.lobby.lead());
        rig.hear("Match 1 is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 30 second(s).");
        assert_eq!(rig.next(100).as_deref(), Some("red"));
        rig.hear("Red Room #57");
        rig.next(100);
        rig.hear("You climb into Red Pod 2. The panel wakes when every pilot is seated.");
        rig.hear("Match 1 is running. Good hunting.");
        rig.next(100);
        rig.hear("Power online.");
        // The server's order: the hatch, the team room, then the end.
        rig.hear("The hatch of Red Pod 2 swings open.");
        rig.hear("Red Room #57");
        assert_eq!(rig.next(2_000), None, "not on the hatch");
        rig.hear("Match 1 is over (blue). Its units are checked in.");
        let t = rig.now;
        assert_eq!(rig.next(999), None);
        assert_eq!(rig.next(200).as_deref(), Some("SIM TEAM red"));
        assert!(rig.now >= t + REMATCH_AFTER_MS);
        rig.hear("The last match is cleared from the board and the tanks are back at muster.");
        rig.hear("Pilot takes red slot 1. `SIM START` begins the match.");
        assert!(rig.lobby.lead());
        assert_eq!(rig.lobby.matches().len(), 1);
        assert_eq!(rig.lobby.matches()[0].result, "blue");
        assert!(!rig.lobby.matches()[0].lead);
    }

    #[test]
    fn a_claim_under_another_name_is_confirmed_by_already_hold() {
        // The sheet says "Rosebot"; the game calls the character "Rose".
        let mut rig = Rig::with(Side::Red, Settings::default(), "Rosebot");
        rig.walk_in();
        rig.board("idle", "nobody", "nobody");
        assert_eq!(rig.next(100).as_deref(), Some("SIM TEAM red"));
        rig.hear("Rose takes red slot 1. `SIM START` begins the match.");
        assert_eq!(rig.lobby.step(), Step::Claim);
        let t = rig.now;
        assert_eq!(rig.next(1_100).as_deref(), Some("SIM TEAM red"));
        assert!(rig.now >= t + CLAIM_RECHECK_MS);
        rig.hear("You already hold red slot 1. `SIM LEAVE` gives it up.");
        assert_eq!(rig.lobby.slot(), Some(1));
        assert_eq!(rig.lobby.me(), "Rose");
        assert!(
            rig.lobby.lead(),
            "the window read Rose first, and Rose is me"
        );
        // Somebody else's slot is not the pilot's name.
        let mut rig = Rig::with(Side::Red, Settings::default(), "Rosebot");
        rig.walk_in();
        rig.board("idle", "nobody", "nobody");
        rig.next(100);
        rig.hear("Ada takes red slot 1. `SIM START` begins the match.");
        rig.next(1_100);
        rig.hear("You already hold red slot 2. `SIM LEAVE` gives it up.");
        assert_eq!(rig.lobby.me(), "Rosebot");
        assert!(!rig.lobby.lead());
    }

    #[test]
    fn the_connected_line_names_the_pilot_before_it_claims() {
        // The sheet says "Rosebot"; the login banner says the game calls the
        // character "Rose": the claim is read as the pilot's own at once, with no
        // second `SIM TEAM`.
        let mut rig = Rig::with(Side::Red, Settings::default(), "Rosebot");
        rig.hear("Connected as Rose (account rose, 1 connection).");
        assert_eq!(rig.lobby.me(), "Rose");
        rig.walk_in();
        rig.board("idle", "nobody", "nobody");
        assert_eq!(rig.next(100).as_deref(), Some("SIM TEAM red"));
        rig.hear("Rose takes red slot 1. `SIM START` begins the match.");
        assert_eq!(rig.lobby.slot(), Some(1));
        assert!(rig.lobby.lead());
        assert_eq!(rig.lobby.step(), Step::Start);
        assert_eq!(rig.next(2_000), None, "no second claim inside the grace");
        assert_eq!(rig.typed.iter().filter(|l| *l == "SIM TEAM red").count(), 1);
    }

    #[test]
    fn a_foreign_matchs_lines_move_nothing() {
        let mut rig = Rig::new(Side::Red);
        rig.fly();
        rig.hear("Match 9 is boarding. Every pilot: BOARD your pod.");
        rig.hear("Match 9 is running. Good hunting.");
        rig.hear("Match 9 is over (blue). Its units are checked in.");
        assert_eq!(rig.lobby.step(), Step::Fight);
        assert!(rig.lobby.matches().is_empty());
        // And before the claim, a boarding line is not followed.
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        rig.hear("Match 3 is boarding. Every pilot: BOARD your pod.");
        assert_eq!(rig.picture.following, None);
    }

    #[test]
    fn a_drop_mid_fight_reboards_every_three_seconds_until_taken_back() {
        let mut rig = Rig::new(Side::Red);
        rig.fly();
        rig.hear("Red Tank 1: power online, 30 kph, heading 090°");
        rig.hear("You are not at the station of a deployed unit.");
        assert_eq!(rig.lobby.step(), Step::Reboard);
        let t = rig.now;
        assert_eq!(rig.at(t).as_deref(), Some("BOARD"));
        rig.hear("Red Tank 1 is still being handed over; try again in 4 second(s).");
        assert_eq!(rig.next(3_900), None);
        assert_eq!(rig.next(200).as_deref(), Some("BOARD"));
        let u = rig.now;
        assert_eq!(rig.next(2_900), None);
        assert_eq!(rig.next(200).as_deref(), Some("BOARD"), "every 3 s");
        assert!(rig.now >= u + REBOARD_EVERY_MS);
        rig.hear("Back aboard RA after 9 s away.");
        rig.hear("  Power online.");
        assert!(
            rig.said()
                .contains(&"Back aboard after 9 s away.".to_owned())
        );
        rig.next(1_100);
        assert_eq!(rig.lobby.step(), Step::Fight);
        rig.hear("Match 1 is over (red). Its units are checked in.");
        let result = &rig.lobby.matches()[0];
        assert_eq!((result.reclaims, result.away_ms), (1, 9_000));
        assert!(result.won);
    }

    #[test]
    fn a_reboard_outside_the_team_room_walks_there_first_and_restarts_a_dark_tank() {
        let mut rig = Rig::new(Side::Red);
        rig.fly();
        rig.hear("The server restarted and the match resumed from its last save; what happened after that save did not count.");
        assert_eq!(rig.lobby.step(), Step::Reboard);
        // The reconnect line (`… is flying on doctrine; …` until the game's
        // 17b cut M5.3).
        rig.hear("Red Tank 1 is flying on its mind; BOARD takes it back.");
        assert_eq!(rig.next(100).as_deref(), Some("BOARD"));
        rig.hear("Boarding is through one of the simulator's team rooms.");
        assert_eq!(rig.next(100).as_deref(), Some("red"));
        rig.hear("Red Room #57");
        assert_eq!(rig.next(100).as_deref(), Some("BOARD"));
        // The restart left the tank dark: the catch-up block in the server's own
        // order, its power row last, after the line that says it is taken back.
        // The picture still reads the power from before the restart.
        assert_eq!(rig.picture.own.power, Power::Online);
        rig.hear("Back aboard RA after 12 s away.");
        assert_eq!(rig.lobby.step(), Step::Resume, "not straight to the fight");
        for row in [
            "  The machine was starting up under no order.",
            "  Position: cell (10, 97), unchanged.",
            "  Contacts: none new, none lost.",
            "  Sections: unchanged.",
            "  Rounds spent: none.",
            "  Power off.",
        ] {
            rig.hear(row);
        }
        assert_eq!(rig.picture.own.power, Power::Off);
        assert_eq!(rig.next(1_100).as_deref(), Some("STARTUP"));
        rig.hear("Startup sequence begun; power comes online shortly.");
        rig.hear("Power online.");
        assert_eq!(rig.lobby.step(), Step::Fight);
        assert_eq!(rig.lobby.matches().len(), 0);
    }

    #[test]
    fn a_tank_that_reads_dark_mid_fight_is_started_again() {
        // `You are already in your pod.` says nothing of the power; the first
        // `STATUS` after it does.
        let mut rig = Rig::new(Side::Red);
        rig.fly();
        rig.hear("You are not at the station of a deployed unit.");
        assert_eq!(rig.next(100).as_deref(), Some("BOARD"));
        rig.hear("You are already in your pod.");
        rig.next(1_100);
        assert_eq!(rig.lobby.step(), Step::Fight, "the stale reading is online");
        rig.hear("Power not online: STARTUP first.");
        assert_eq!(rig.next(100).as_deref(), Some("STARTUP"));
        rig.hear("Startup sequence begun; power comes online shortly.");
        assert_eq!(rig.lobby.step(), Step::AwaitPower);
        rig.hear("Power online.");
        assert_eq!(rig.lobby.step(), Step::Fight);
        // Not after the hatch: the match is ending.
        rig.hear("The hatch of Red Pod 1 swings open.");
        rig.hear("Power not online: STARTUP first.");
        assert_eq!(rig.next(500), None);
    }

    #[test]
    fn a_busy_answer_to_sim_leave_types_it_again_inside_its_three_seconds() {
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        rig.lobby.stop(rig.now, &rig.picture);
        assert_eq!(rig.next(100).as_deref(), Some("SIM LEAVE"));
        let first = rig.now;
        rig.hear("The command could not commit; please retry in a moment.");
        assert_eq!(rig.at(first + 199), None);
        assert_eq!(rig.at(first + 200).as_deref(), Some("SIM LEAVE"));
        rig.hear("Pilot gives up red slot 1.");
        assert!(!rig.lobby.leave_pending(rig.now));
        assert_eq!(rig.next(5_000), None);
        // Busy past its three seconds: nothing more.
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        rig.lobby.stop(rig.now, &rig.picture);
        assert_eq!(rig.next(100).as_deref(), Some("SIM LEAVE"));
        let first = rig.now;
        let mut again = Vec::new();
        while rig.now < first + LEAVE_WAIT_MS + 1_000 {
            rig.hear("The arena is busy; try that again in a moment.");
            if let Some(line) = rig.next(300) {
                assert_eq!(line, "SIM LEAVE");
                again.push(rig.now);
            }
        }
        assert!(again.len() >= 5, "re-sent while it waits: {again:?}");
        assert!(
            again.iter().all(|at| *at < first + LEAVE_WAIT_MS),
            "never past its three seconds: {again:?} from {first}"
        );
        assert!(!rig.lobby.leave_pending(rig.now));
    }

    /// Named `a_survey_of_a_running_centre_flown_by_doctrine_reboards`, its row
    /// ending `, flown by doctrine`, until the game's 17b cut M5.3 renamed the
    /// drop clause.
    #[test]
    fn a_survey_of_a_running_centre_flown_by_its_mind_reboards() {
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        rig.board(
            "running (match 6)",
            "1 (RA): Pilot, pod Red Pod 1, flown by its mind",
            "1 (BA): Bo",
        );
        assert_eq!(rig.lobby.step(), Step::Reboard);
        assert_eq!(rig.picture.following, Some(6));
        assert_eq!(rig.next(100).as_deref(), Some("BOARD"));
        rig.hear("Back aboard RA after 1 m 5 s away.");
        rig.hear("Match 6 is over (draw). Its units are checked in.");
        assert_eq!(rig.lobby.matches()[0].result, "draw");
        assert_eq!(rig.lobby.matches()[0].reclaims, 1);
    }

    #[test]
    fn a_tank_left_for_good_ends_the_match_as_left() {
        let mut rig = Rig::new(Side::Red);
        rig.fly();
        rig.hear("You are not at the station of a deployed unit.");
        rig.next(100);
        rig.hear("You left Red Tank 1 for the rest of the match; it cannot be taken back.");
        assert_eq!(rig.lobby.matches()[0].result, "left");
        assert!(rig.lobby.finished());
        let said = rig.said();
        assert!(
            said.iter()
                .any(|s| s.starts_with("Stopped flying match 1 before its end (left); ")),
            "{said:?}"
        );
    }

    #[test]
    fn the_hatch_then_outside_cockpit_is_the_end_not_a_drop() {
        let mut rig = Rig::new(Side::Red);
        rig.fly();
        rig.hear("The hatch of Red Pod 1 swings open.");
        rig.hear("You are not at the station of a deployed unit.");
        assert_eq!(rig.lobby.step(), Step::Fight);
        rig.hear("Match 1 is over (timeout). Its units are checked in.");
        assert!(rig.lobby.finished());
        assert_eq!(rig.lobby.matches()[0].result, "timeout");
    }

    #[test]
    fn an_abort_before_running_ends_the_match_as_abort() {
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        rig.next(4_000);
        rig.hear("Match 1 is boarding. Every pilot: BOARD your pod.");
        rig.next(100);
        rig.hear("Red Room #57");
        rig.next(100);
        rig.hear("You climb into Red Pod 1. The panel wakes when every pilot is seated.");
        rig.hear("Match 1 is aborted: staff called it off.");
        assert_eq!(rig.lobby.matches()[0].result, "abort");
        assert_eq!(rig.lobby.matches()[0].callsign, "RA", "no STATUS was read");
        assert!(rig.lobby.finished());
    }

    #[test]
    fn sim_leave_once_on_stop_in_selection_holding_a_slot() {
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        rig.lobby.stop(rig.now, &rig.picture);
        rig.lobby.stop(rig.now, &rig.picture);
        assert_eq!(rig.next(100).as_deref(), Some("SIM LEAVE"));
        assert!(rig.lobby.leave_pending(rig.now));
        assert_eq!(rig.next(5_000), None, "never again");
        rig.hear("Pilot gives up red slot 1.");
        assert!(!rig.lobby.leave_pending(rig.now));
        assert!(rig.said().contains(&"Gave up red slot 1.".to_owned()));
        assert_eq!(rig.typed.iter().filter(|l| *l == "SIM LEAVE").count(), 1);
        // Never while boarding or running, never without a slot.
        let mut boarding = Rig::new(Side::Red);
        boarding.claim_lead();
        boarding.next(4_000);
        boarding.hear("Match 1 is boarding. Every pilot: BOARD your pod.");
        boarding.lobby.stop(boarding.now, &boarding.picture);
        assert_eq!(boarding.next(5_000), None);
        let mut running = Rig::new(Side::Red);
        running.fly();
        running.lobby.stop(running.now, &running.picture);
        assert_eq!(running.next(5_000), None);
        assert_eq!(running.lobby.matches()[0].result, "blocked");
        // The match is still running on the server: the sentence does not call
        // it over.
        let said = running.said();
        assert!(
            said.iter().any(|s| s
                == "Stopped flying match 1 before its end (blocked); 0 shot(s), 0 hit(s), 0 kill(s); took 0 hit(s)."),
            "{said:?}"
        );
        assert!(!said.iter().any(|s| s.contains("is over")), "{said:?}");
        let mut walking = Rig::new(Side::Red);
        walking.walk_in();
        walking.lobby.stop(walking.now, &walking.picture);
        assert_eq!(walking.next(5_000), None);
        // A blocker in selection holding a slot leaves too.
        let settings = Settings {
            wait_start_s: 10,
            ..Settings::default()
        };
        let mut blocked = Rig::with(Side::Red, settings, "Pilot");
        blocked.walk_in();
        blocked.board("selection", "1 (RA): Ada", "nobody");
        blocked.next(100);
        blocked.hear("Pilot takes red slot 2. `SIM START` begins the match.");
        assert_eq!(blocked.next(10_100).as_deref(), Some("SIM LEAVE"));
        assert!(blocked.lobby.blocker().is_some());
    }

    #[test]
    fn every_line_on_every_path_is_in_the_closed_vocabulary() {
        let mut all = Vec::new();
        let mut a = Rig::with(
            Side::Blue,
            Settings {
                matches: 2,
                ..Settings::default()
            },
            "Pilot",
        );
        assert_eq!(a.at(0).as_deref(), Some("look"));
        a.hear("Limbo #0");
        a.next(3_000);
        a.hear("Welcome Hall #2");
        a.next(100);
        a.hear("Simulator Lobby #50");
        a.next(100);
        a.board("ended (match 2) — red", "nobody", "nobody");
        a.next(100);
        a.hear("The last match is cleared from the board.");
        a.hear("Pilot takes blue slot 1. `SIM START` begins the match.");
        a.next(4_000);
        a.hear("Match 3 is boarding. Every pilot: BOARD your pod.");
        a.next(100);
        a.hear("Blue Room #58");
        a.next(100);
        a.hear("Boarding is through one of the simulator's team rooms.");
        a.next(100);
        a.hear("Blue Room #58");
        a.next(100);
        a.hear("You climb into Blue Pod 1. The panel wakes when every pilot is seated.");
        a.hear("Match 3 is running. Good hunting.");
        a.next(100);
        a.hear("Power is still coming up; hold on.");
        a.hear("Power online.");
        a.hear("You are not at the station of a deployed unit.");
        a.next(100);
        a.hear("Blue Tank 1 is still being handed over; try again in 1 second(s).");
        a.next(1_100);
        a.hear("Back aboard BA after 3 s away.");
        a.next(1_100);
        a.hear("Match 3 is over (blue). Its units are checked in.");
        a.next(1_100);
        a.hear("Pilot takes blue slot 1. `SIM START` begins the match.");
        a.lobby.stop(a.now, &a.picture);
        a.next(100);
        all.append(&mut a.typed);
        let mut b = Rig::new(Side::Red);
        b.fly();
        all.append(&mut b.typed);
        for line in &all {
            assert!(VOCABULARY.contains(&line.as_str()), "{line}");
        }
        for line in VOCABULARY {
            assert!(all.iter().any(|l| l == line), "{line} was never reached");
        }
        // And the enum builds nothing else.
        for line in [
            LobbyLine::Look,
            LobbyLine::Ooc,
            LobbyLine::Simulator,
            LobbyLine::TeamRoom(Side::Red),
            LobbyLine::TeamRoom(Side::Blue),
            LobbyLine::SimStatus,
            LobbyLine::SimTeam(Side::Red),
            LobbyLine::SimTeam(Side::Blue),
            LobbyLine::SimStart,
            LobbyLine::Board,
            LobbyLine::Startup,
            LobbyLine::SimLeave,
        ] {
            assert!(VOCABULARY.contains(&line.line().as_str()));
        }
    }

    #[test]
    fn the_waiting_steps_block_on_their_own_bounds() {
        // In the pod: the boarding window the line announced, plus 10 s.
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        rig.next(4_000);
        let boarding = rig.now;
        rig.hear("Match 1 is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 30 second(s).");
        rig.next(100);
        rig.hear("Red Room #57");
        rig.next(100);
        rig.hear("You climb into Red Pod 1. The panel wakes when every pilot is seated.");
        assert_eq!(rig.lobby.step(), Step::AwaitRunning);
        rig.at(boarding + 39_999);
        assert!(!rig.lobby.finished());
        rig.at(boarding + 40_000);
        assert_eq!(
            rig.lobby.blocker(),
            Some("The match did not start within 40 s.")
        );
        assert_eq!(rig.lobby.matches()[0].result, "blocked");
        // Powering up: 30 s.
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        rig.next(4_000);
        rig.hear("Match 1 is boarding. Every pilot: BOARD your pod.");
        rig.next(100);
        rig.hear("Red Room #57");
        rig.next(100);
        rig.hear("You climb into Red Pod 1.");
        rig.hear("Match 1 is running. Good hunting.");
        rig.next(100);
        rig.hear("Startup sequence begun; power comes online shortly.");
        let begun = rig.now;
        assert_eq!(rig.at(begun + POWER_WAIT_MS - 1), None);
        // The match runs: never given up because the power did not come up
        // (cut AM.2), `STARTUP` again instead.
        assert_eq!(rig.at(begun + POWER_WAIT_MS).as_deref(), Some("STARTUP"));
        assert_eq!(rig.lobby.blocker(), None);
        assert_eq!(rig.lobby.step(), Step::Startup);
        assert!(
            rig.said().contains(
                &"Power did not come online within 30 s; typing STARTUP again.".to_owned()
            )
        );
    }

    #[test]
    fn the_callsign_letters() {
        assert_eq!(callsign(Side::Red, 1), "RA");
        assert_eq!(callsign(Side::Blue, 3), "BC");
        assert_eq!(
            boarding_window_ms(
                "Match 1 is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 60 second(s)."
            ),
            Some(60_000)
        );
    }

    // -- cut AM.2: the stop races, the start, the walk, `STARTUP`, the pause ----

    const CLAIMED: &str = "Pilot takes red slot 1. `SIM START` begins the match; the machines take the pods nobody claims.";

    /// Walk in, read an idle board, and type `SIM TEAM red`; when it went.
    fn claim_typed(rig: &mut Rig) -> u64 {
        rig.walk_in();
        rig.board("idle", "nobody", "nobody");
        assert_eq!(rig.next(100).as_deref(), Some("SIM TEAM red"));
        rig.now
    }

    /// **A stop with a claim in flight** (cut AM.2): it says so, types nothing
    /// until the claim's answer, then exactly one `SIM LEAVE`; the leave is
    /// pending until that leave's own answer.
    #[test]
    fn a_stop_with_a_claim_in_flight_waits_then_leaves_once() {
        let mut rig = Rig::new(Side::Red);
        claim_typed(&mut rig);
        rig.lobby.stop(rig.now, &rig.picture);
        assert!(
            rig.said().contains(
                &"Stopping: waiting for the answer to `SIM TEAM red` before giving the slot up."
                    .to_owned()
            )
        );
        assert!(rig.lobby.leave_pending(rig.now));
        assert_eq!(rig.next(5_000), None, "nothing before the claim's answer");
        assert!(rig.lobby.leave_pending(rig.now));
        rig.hear(CLAIMED);
        assert_eq!(rig.lobby.slot(), Some(1));
        assert!(rig.lobby.leave_pending(rig.now));
        assert_eq!(rig.next(100).as_deref(), Some("SIM LEAVE"));
        assert!(rig.lobby.leave_pending(rig.now));
        assert_eq!(rig.next(2_000), None);
        rig.hear("Pilot gives up red slot 1.");
        assert!(!rig.lobby.leave_pending(rig.now));
        assert!(rig.said().contains(&"Gave up red slot 1.".to_owned()));
        assert_eq!(rig.next(30_000), None, "nothing after it");
        assert_eq!(rig.types("SIM LEAVE"), 1);
        // A second stop changes nothing.
        rig.lobby.stop(rig.now, &rig.picture);
        assert!(rig.said().is_empty());
    }

    /// **A claim never answered** (cut AM.2): the one `SIM LEAVE` goes at the
    /// claim's typing and the step timeout, not one tick before, and once.
    #[test]
    fn a_stop_whose_claim_is_never_answered_leaves_at_the_step_timeout() {
        let mut rig = Rig::new(Side::Red);
        let sent = claim_typed(&mut rig);
        let timeout = Settings::default().step_timeout_ms;
        rig.lobby.stop(rig.now, &rig.picture);
        assert_eq!(rig.at(sent + timeout - 50), None);
        assert_eq!(rig.at(sent + timeout - 1), None);
        assert!(rig.lobby.leave_pending(rig.now));
        assert_eq!(rig.at(sent + timeout).as_deref(), Some("SIM LEAVE"));
        // An unneeded leave is answered harmlessly.
        rig.hear("You hold no slot in the simulator.");
        assert_eq!(rig.next(30_000), None);
        assert_eq!(rig.types("SIM LEAVE"), 1);
        assert!(!rig.lobby.leave_pending(rig.now));
        assert_eq!(rig.lobby.blocker(), None, "a stop is not a blocker");
    }

    /// **A stop with a start in flight** (cut AM.2): the boarding line that
    /// answers it is followed by exactly one `SIM LEAVE`.
    #[test]
    fn a_stop_with_a_start_in_flight_leaves_after_its_answer() {
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        assert_eq!(rig.next(4_000).as_deref(), Some("SIM START"));
        rig.lobby.stop(rig.now, &rig.picture);
        assert!(
            rig.said().contains(
                &"Stopping: waiting for the answer to `SIM START` before giving the slot up."
                    .to_owned()
            )
        );
        assert_eq!(rig.next(1_000), None);
        rig.hear("Match 1 is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 30 second(s).");
        assert_eq!(rig.next(100).as_deref(), Some("SIM LEAVE"));
        assert_eq!(rig.next(30_000), None);
        assert_eq!(rig.types("SIM LEAVE"), 1);
        assert!(!rig.typed.iter().any(|l| l == "red" || l == "BOARD"));
        // Every other answer to a start queues it too.
        for answer in [
            "Only a side's lead or staff can start the match.",
            "The blue side has no pilots; a match takes at least one a side.",
            "The simulator centre is running; that is not something you can do to it now.",
            "Nobody is in the simulator; a match takes at least one pilot.",
            "The arena is busy; try that again in a moment.",
        ] {
            let mut rig = Rig::new(Side::Red);
            rig.claim_lead();
            assert_eq!(rig.next(4_000).as_deref(), Some("SIM START"));
            rig.lobby.stop(rig.now, &rig.picture);
            rig.hear("Ada says, \"Wait for me.\"");
            assert_eq!(rig.next(1_000), None, "{answer}: not chatter");
            rig.hear(answer);
            assert_eq!(rig.next(100).as_deref(), Some("SIM LEAVE"), "{answer}");
            assert_eq!(rig.next(30_000), None, "{answer}");
        }
    }

    /// **Every answer to a claim** a stop waits for queues the one leave (cut
    /// AM.2), the pilot's own claim recording its slot.
    #[test]
    fn every_answer_to_a_claim_queues_the_leave() {
        for answer in [
            "You already hold red slot 2. `SIM LEAVE` gives it up.",
            "The red side is full: it takes 1 pilot(s).",
            "The simulator centre is running; that is not something you can do to it now.",
            "This server has no simulator centre.",
            "The TacSim layer is off on this server, so the simulator cannot run.",
            "The simulator is through the lobby.",
            "Nobody is in the simulator; a match takes at least one pilot.",
            "The command could not commit; please retry in a moment.",
        ] {
            let mut rig = Rig::new(Side::Red);
            claim_typed(&mut rig);
            rig.lobby.stop(rig.now, &rig.picture);
            // Somebody else's claim is no answer to this one.
            rig.hear("Ada takes red slot 1. `SIM START` begins the match.");
            assert_eq!(rig.next(1_000), None, "{answer}");
            rig.hear(answer);
            assert_eq!(rig.next(100).as_deref(), Some("SIM LEAVE"), "{answer}");
            assert_eq!(rig.next(30_000), None, "{answer}");
            assert_eq!(rig.types("SIM LEAVE"), 1, "{answer}");
        }
    }

    /// **A refused claim neither waits nor leaves** (cut AM.2): a blocker
    /// closes as before — no `Stopping` sentence, no `SIM LEAVE`, nothing
    /// pending — and a stop after it changes nothing.
    #[test]
    fn a_refused_claim_neither_waits_nor_leaves() {
        for (answer, blocker) in [
            (
                "The red side is full: it takes 1 pilot(s).",
                "The red side is full.",
            ),
            (
                "You already hold blue slot 2. `SIM LEAVE` gives it up.",
                "The bot already holds a slot on the other side.",
            ),
        ] {
            let mut rig = Rig::new(Side::Red);
            claim_typed(&mut rig);
            rig.hear(answer);
            assert_eq!(rig.lobby.blocker(), Some(blocker));
            rig.lobby.stop(rig.now, &rig.picture);
            assert!(!rig.lobby.leave_pending(rig.now), "{answer}");
            assert_eq!(rig.next(30_000), None, "{answer}");
            assert_eq!(rig.types("SIM LEAVE"), 0, "{answer}");
            let said = rig.said();
            assert!(
                !said.iter().any(|s| s.starts_with("Stopping")),
                "{answer}: {said:?}"
            );
        }
    }

    /// **The other side's slot** (cut AM.2): a stop waiting for a claim that
    /// is answered `You already hold blue slot 2.` types nothing — the claim
    /// took nothing, and that slot is not this run's to give up.
    #[test]
    fn a_stop_answered_by_the_other_sides_slot_types_no_leave() {
        let mut rig = Rig::new(Side::Red);
        claim_typed(&mut rig);
        rig.lobby.stop(rig.now, &rig.picture);
        assert!(rig.lobby.leave_pending(rig.now));
        rig.hear("You already hold blue slot 2. `SIM LEAVE` gives it up.");
        assert_eq!(rig.lobby.leave, Leave::Done);
        assert!(!rig.lobby.leave_pending(rig.now));
        assert_eq!(rig.next(30_000), None);
        assert_eq!(rig.types("SIM LEAVE"), 0);
    }

    /// **A start wanted and not typed goes with the lead** (cut AM.2): a match
    /// end read while the pilot follows none releases every lead in
    /// `centre_line` (not through `recompute_lead`), so it is `poll` and `due`
    /// that drop the start: it is never typed, and the pilot waits.
    #[test]
    fn a_queued_start_is_dropped_when_the_lead_is_lost() {
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        let grace = rig.now + Settings::default().lead_start_grace_ms;
        // Polled past the grace, not typed: the start is wanted.
        rig.now = grace;
        rig.lobby.poll(grace, &mut rig.picture);
        assert!(matches!(rig.lobby.want, Some((LobbyLine::SimStart, _))));
        assert_eq!(rig.lobby.step(), Step::Start);
        // Somebody else's match ends first.
        rig.hear("Match 7 is over (red). Its units are checked in.");
        assert!(!rig.lobby.lead());
        // `due` alone never yields it now.
        assert_eq!(rig.lobby.due(rig.now), None);
        assert_eq!(rig.at(rig.now), None);
        assert_eq!(rig.lobby.step(), Step::Await);
        assert_eq!(rig.lobby.want, None);
        assert_eq!(rig.next(10_000), None);
        assert_eq!(rig.types("SIM START"), 0);
        // And `due` refuses a start wanted at `Start` by a pilot who does not
        // lead, whatever put it there.
        rig.lobby.step = Step::Start;
        rig.lobby.want = Some((LobbyLine::SimStart, 0));
        assert_eq!(rig.lobby.due(rig.now), None);
        // A lead lost through the claim window drops it too, the flight
        // whatever it is.
        let mut rig = Rig::new(Side::Red);
        rig.claim_lead();
        rig.now += Settings::default().lead_start_grace_ms;
        rig.lobby.poll(rig.now, &mut rig.picture);
        assert!(matches!(rig.lobby.want, Some((LobbyLine::SimStart, _))));
        rig.lobby.flight = Some(Flight {
            line: LobbyLine::SimStatus,
            sent_ms: rig.now,
        });
        rig.hear("Pilot gives up red slot 1.");
        assert!(!rig.lobby.lead());
        assert_eq!(rig.lobby.want, None);
    }

    /// **A late `look` answer never blocks the walk** (cut AM.2): `look`, some
    /// noise and the quiet rule's `ooc`; then the late answer's heading is
    /// ignored (no blocker, and it starts no quiet rule), and the walk's own
    /// rooms carry it to `SIM STATUS`.
    #[test]
    fn a_late_look_answer_never_blocks_the_walk() {
        let mut rig = Rig::new(Side::Red);
        assert_eq!(rig.at(0).as_deref(), Some("look"));
        rig.hear("Huh?  (Type \"help\" for help.)");
        assert_eq!(rig.next(1_800).as_deref(), Some("ooc"));
        rig.hear("Simulator Lobby #50");
        assert_eq!(rig.lobby.step(), Step::Walk);
        assert_eq!(rig.next(3_000), None);
        assert_eq!(rig.lobby.blocker(), None);
        assert!(!rig.lobby.finished());
        rig.hear("Welcome Hall #2");
        assert_eq!(rig.next(100).as_deref(), Some("simulator"));
        rig.hear("Simulator Lobby #50");
        assert_eq!(rig.next(100).as_deref(), Some("SIM STATUS"));
        assert_eq!(rig.typed, ["look", "ooc", "simulator", "SIM STATUS"]);
        assert_eq!(rig.lobby.blocker(), None);
        // A heading before `look` is typed is not its answer.
        let mut rig = Rig::new(Side::Red);
        rig.hear("Simulator Lobby #50");
        assert_eq!(rig.lobby.step(), Step::Look);
        assert_eq!(rig.at(0).as_deref(), Some("look"));
        rig.hear("Limbo #0");
        assert_eq!(rig.next(3_000).as_deref(), Some("ooc"));
    }

    /// **A walk waits for its own room** (cut AM.2): during `simulator`'s
    /// flight a Welcome Hall heading is ignored; the Simulator Lobby moves it
    /// on.
    #[test]
    fn a_walk_waits_for_its_own_room() {
        let mut rig = Rig::new(Side::Red);
        assert_eq!(rig.at(0).as_deref(), Some("look"));
        rig.hear("Limbo #0");
        assert_eq!(rig.next(3_000).as_deref(), Some("ooc"));
        rig.hear("Welcome Hall #2");
        assert_eq!(rig.next(100).as_deref(), Some("simulator"));
        rig.hear("Welcome Hall #2");
        assert_eq!(rig.next(3_000), None);
        assert_eq!(rig.lobby.step(), Step::Walk);
        assert_eq!(rig.lobby.blocker(), None);
        rig.hear("Simulator Lobby #50");
        assert_eq!(rig.next(100).as_deref(), Some("SIM STATUS"));
    }

    /// **`STARTUP` knows every answer** (cut AM.2), each from `STARTUP` typed
    /// in a running match.
    #[test]
    fn startup_knows_every_answer() {
        // Begun or still coming up: the power wait.
        for line in [
            "Startup sequence begun; power comes online shortly.",
            "Power is still coming up; hold on.",
        ] {
            let mut rig = Rig::new(Side::Red);
            rig.up_to_startup();
            rig.hear(line);
            assert_eq!(rig.lobby.step(), Step::AwaitPower, "{line}");
        }
        // Online: the fight.
        for line in ["Power is already online.", "Power online."] {
            let mut rig = Rig::new(Side::Red);
            rig.up_to_startup();
            rig.hear(line);
            assert_eq!(rig.lobby.step(), Step::Fight, "{line}");
            assert!(rig.said().contains(&"Power online; fighting.".to_owned()));
        }
        // The dark panel: again in a second.
        let mut rig = Rig::new(Side::Red);
        rig.up_to_startup();
        rig.hear("The panel is dark until the match starts; the simulator is still waiting for the other pods.");
        let t = rig.now;
        assert_eq!(rig.at(t + 999), None);
        assert_eq!(rig.at(t + 1_000).as_deref(), Some("STARTUP"));
        // Destroyed, or a wreck: the fight's step, nothing typed for a minute,
        // no blocker; the match's end records the centre's word.
        for line in [
            "Your tank is destroyed and takes no orders. EXIT leaves the pod.",
            "STARTUP refused. Your tank is a wreck.",
        ] {
            let mut rig = Rig::new(Side::Red);
            rig.up_to_startup();
            rig.hear(line);
            assert_eq!(rig.lobby.step(), Step::Fight, "{line}");
            assert!(rig.picture.own.destroyed, "{line}");
            assert!(
                rig.said()
                    .contains(&"The tank is destroyed; waiting for the match to end.".to_owned()),
                "{line}"
            );
            assert_eq!(rig.next(60_000), None, "{line}");
            assert_eq!(rig.lobby.blocker(), None, "{line}");
            rig.hear("Match 1 is over (blue). Its units are checked in.");
            assert_eq!(rig.lobby.matches()[0].result, "blue", "{line}");
            assert_eq!(rig.lobby.blocker(), None, "{line}");
        }
        // No longer in the match, the last match asked: recorded `left`, and
        // the run ends there (no end is waited for, and the sentence says
        // none is).
        let mut rig = Rig::new(Side::Red);
        rig.up_to_startup();
        rig.hear("STARTUP refused. That tank is no longer in the match.");
        assert_eq!(rig.lobby.matches()[0].result, "left");
        let said = rig.said();
        assert!(
            said.contains(&"The tank is no longer in the match.".to_owned()),
            "{said:?}"
        );
        assert!(
            !said.iter().any(|s| s.contains("waiting for it to end")),
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|s| s.starts_with("Stopped flying match 1 before its end (left); ")),
            "{said:?}"
        );
        assert_eq!(rig.lobby.blocker(), None);
        assert!(rig.lobby.finished());
        // No longer in the match with another to fly: the flight and the want
        // go, nothing is typed and nothing blocks while the match runs on, and
        // the next claim goes after its end; the record stays `left`.
        let mut rig = Rig::with(
            Side::Red,
            Settings {
                matches: 2,
                ..Settings::default()
            },
            "Pilot",
        );
        rig.up_to_startup();
        rig.hear("STARTUP refused. That tank is no longer in the match.");
        assert_eq!(rig.lobby.matches()[0].result, "left");
        assert!(
            rig.said()
                .contains(&"The tank is no longer in the match; waiting for it to end.".to_owned())
        );
        assert_eq!(rig.lobby.step(), Step::Over);
        assert_eq!(rig.lobby.flight, None);
        assert_eq!(rig.lobby.want, None);
        assert_eq!(rig.next(60_000), None);
        assert_eq!(rig.lobby.blocker(), None);
        assert!(!rig.lobby.closed());
        rig.hear("Match 1 is over (blue). Its units are checked in.");
        let end = rig.now;
        assert_eq!(rig.at(end + REMATCH_AFTER_MS - 1), None);
        assert_eq!(
            rig.at(end + REMATCH_AFTER_MS).as_deref(),
            Some("SIM TEAM red")
        );
        assert_eq!(rig.lobby.matches().len(), 1);
        assert_eq!(rig.lobby.matches()[0].result, "left");
        assert_eq!(rig.types("SIM TEAM red"), 2);
        // Any other refusal: again at the step timeout, not every second.
        let mut rig = Rig::new(Side::Red);
        rig.up_to_startup();
        rig.hear("STARTUP refused. The arena has not seated your tank.");
        let t = rig.now;
        let timeout = Settings::default().step_timeout_ms;
        assert!(rig.said().contains(
            &"STARTUP was refused (`The arena has not seated your tank.`); typing it again in 15 s."
                .to_owned()
        ));
        assert_eq!(rig.at(t + 1_000), None);
        assert_eq!(rig.at(t + timeout - 1), None);
        assert_eq!(rig.at(t + timeout).as_deref(), Some("STARTUP"));
        assert_eq!(rig.lobby.blocker(), None);
        // A refused `FIRE` saying the tank is a wreck marks nothing.
        let mut picture = Picture::default();
        picture.apply(0, &classify("FIRE cannon refused. Your tank is a wreck."));
        assert!(!picture.own.destroyed);
        // A drop: the re-board.
        let mut rig = Rig::new(Side::Red);
        rig.up_to_startup();
        rig.hear("You are not at the station of a deployed unit.");
        assert_eq!(rig.lobby.step(), Step::Reboard);
        assert_eq!(rig.next(100).as_deref(), Some("BOARD"));
    }

    /// **A running match is never given up because the tank would not start**
    /// (cut AM.2): no answer to `STARTUP` within the step timeout types it
    /// again, and so does a power wait that runs out; the match's end then
    /// records the centre's own word.
    #[test]
    fn a_running_match_is_never_blocked_by_startup() {
        let mut rig = Rig::new(Side::Red);
        let t = rig.up_to_startup();
        let timeout = Settings::default().step_timeout_ms;
        assert_eq!(rig.at(t + timeout - 1), None);
        assert_eq!(rig.at(t + timeout).as_deref(), Some("STARTUP"));
        assert!(
            rig.said()
                .contains(&"No answer to `STARTUP` within 15 s; typing it again.".to_owned())
        );
        assert_eq!(rig.lobby.blocker(), None);
        rig.hear("Startup sequence begun; power comes online shortly.");
        let begun = rig.now;
        assert_eq!(rig.at(begun + POWER_WAIT_MS - 1), None);
        assert_eq!(rig.at(begun + POWER_WAIT_MS).as_deref(), Some("STARTUP"));
        assert!(
            rig.said().contains(
                &"Power did not come online within 30 s; typing STARTUP again.".to_owned()
            )
        );
        assert_eq!(rig.lobby.step(), Step::Startup);
        rig.hear("Match 1 is over (blue). Its units are checked in.");
        assert_eq!(rig.lobby.matches().len(), 1);
        assert_eq!(rig.lobby.matches()[0].result, "blue");
        assert_eq!(rig.lobby.blocker(), None);
        assert!(rig.lobby.finished());
    }

    /// **`shift` moves every lobby wait** (cut AM.2), field by field, and none
    /// of the match's own times nor the leave.
    #[test]
    fn shift_moves_every_wait() {
        let mut rig = Rig::new(Side::Red);
        let lobby = &mut rig.lobby;
        lobby.flight = Some(Flight {
            line: LobbyLine::SimTeam(Side::Red),
            sent_ms: 1,
        });
        lobby.deadline = Some((2, "a deadline".into()));
        lobby.want = Some((LobbyLine::SimStatus, 3));
        lobby.rematch_at = Some(4);
        lobby.reboard_next = 5;
        lobby.resume_at = 6;
        lobby.empty_since = Some(7);
        lobby.busy_since = Some(8);
        lobby.claim_ms = 9;
        lobby.last_other_ms = 10;
        lobby.last_line_ms = 11;
        lobby.running_ms = Some(12);
        lobby.reboard_since = 13;
        lobby.boarding_ms = 14;
        lobby.leave = Leave::AwaitFlight {
            line: LobbyLine::SimTeam(Side::Red),
            until: 15,
        };
        lobby.shift(1_000);
        assert_eq!(lobby.flight.map(|f| f.sent_ms), Some(1_001));
        assert_eq!(lobby.deadline.as_ref().map(|(at, _)| *at), Some(1_002));
        assert_eq!(lobby.want.map(|(_, at)| at), Some(1_003));
        assert_eq!(lobby.rematch_at, Some(1_004));
        assert_eq!(lobby.reboard_next, 1_005);
        assert_eq!(lobby.resume_at, 1_006);
        assert_eq!(lobby.empty_since, Some(1_007));
        assert_eq!(lobby.busy_since, Some(1_008));
        assert_eq!(lobby.claim_ms, 1_009);
        assert_eq!(lobby.last_other_ms, 1_010);
        assert_eq!(lobby.last_line_ms, 1_011);
        // Never the match's own times, nor the leave.
        assert_eq!(lobby.running_ms, Some(12));
        assert_eq!(lobby.reboard_since, 13);
        assert_eq!(lobby.boarding_ms, 14);
        assert_eq!(
            lobby.leave,
            Leave::AwaitFlight {
                line: LobbyLine::SimTeam(Side::Red),
                until: 15
            }
        );
        // Unset waits stay unset.
        let mut rig = Rig::new(Side::Red);
        rig.lobby.want = None;
        rig.lobby.shift(1_000);
        assert_eq!(
            (
                rig.lobby.flight,
                rig.lobby.deadline.clone(),
                rig.lobby.want,
                rig.lobby.rematch_at,
                rig.lobby.empty_since,
                rig.lobby.busy_since
            ),
            (None, None, None, None, None, None)
        );
    }

    /// **Lines read while paused wait from the resume** (cut AM.2): a followed
    /// match's end read a minute into a ten-minute pause sets the rematch
    /// `REMATCH_AFTER_MS` after the resume and the quiet rule's last line at
    /// the resume, while its record runs to when it was really read; a dark
    /// panel read mid-pause wants `STARTUP` a second after the resume.
    #[test]
    fn lines_read_while_paused_wait_from_the_resume() {
        let mut rig = Rig::with(
            Side::Red,
            Settings {
                matches: 2,
                ..Settings::default()
            },
            "Pilot",
        );
        rig.fly();
        let running = rig.lobby.running_ms.expect("running");
        let paused = rig.now + 5_000;
        rig.lobby.pause(paused);
        rig.now = paused + 60_000;
        rig.hear("Match 1 is over (red). Its units are checked in.");
        assert_eq!(rig.lobby.step(), Step::Over);
        assert_eq!(rig.lobby.rematch_at, Some(paused + REMATCH_AFTER_MS));
        assert_eq!(
            rig.lobby.matches()[0].duration_ms,
            paused + 60_000 - running,
            "the record keeps the real time"
        );
        rig.now = paused + 300_000;
        rig.hear("Ada says, \"Still there?\"");
        assert_eq!(rig.lobby.last_line_ms, paused);
        let resumed = paused + 600_000;
        rig.lobby.resume(resumed);
        assert_eq!(rig.lobby.rematch_at, Some(resumed + REMATCH_AFTER_MS));
        assert_eq!(rig.lobby.last_line_ms, resumed);
        assert_eq!(rig.at(resumed + REMATCH_AFTER_MS - 1), None);
        assert_eq!(
            rig.at(resumed + REMATCH_AFTER_MS).as_deref(),
            Some("SIM TEAM red")
        );
        // A dark panel read mid-pause: `STARTUP` a second after the resume.
        let mut rig = Rig::new(Side::Red);
        let t = rig.up_to_startup();
        rig.lobby.pause(t + 100);
        rig.now = t + 20_000;
        rig.hear("The panel is dark until the match starts; the simulator is still waiting for the other pods.");
        let resumed = t + 100_000;
        rig.lobby.resume(resumed);
        assert_eq!(rig.at(resumed + 999), None);
        assert_eq!(rig.at(resumed + 1_000).as_deref(), Some("STARTUP"));
        assert_eq!(rig.lobby.blocker(), None);
    }

    /// **The match's own times stay real while paused** (cut AM.2): a
    /// boarding announced twenty seconds into a pause keeps the window the
    /// centre gave it (its 30 s and the 10 s of slack from when it was really
    /// read), so a climb-in after a resume 45 s in waits for the match and
    /// blocks only when that real bound lapses; a match running read while
    /// paused, and a drop and a return read then, are timed by when they were
    /// really read.
    #[test]
    fn the_matchs_own_times_stay_real_while_paused() {
        let mut rig = Rig::new(Side::Red);
        rig.walk_in();
        rig.board("selection", "1 (RA): Ada", "nobody");
        rig.next(100);
        rig.hear("Pilot takes red slot 2. `SIM START` begins the match.");
        assert_eq!(rig.lobby.step(), Step::Await);
        let paused = rig.now + 1_000;
        rig.lobby.pause(paused);
        rig.now = paused + 20_000;
        let announced = rig.now;
        rig.hear("Match 1 is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 30 second(s).");
        assert_eq!(rig.lobby.step(), Step::ToRoom);
        assert_eq!(rig.lobby.boarding_ms, announced, "really read");
        let resumed = paused + 45_000;
        rig.lobby.resume(resumed);
        assert_eq!(rig.at(resumed).as_deref(), Some("red"));
        rig.hear("Red Room #57");
        assert_eq!(rig.next(100).as_deref(), Some("BOARD"));
        rig.hear("You climb into Red Pod 2. The panel wakes when every pilot is seated.");
        assert_eq!(rig.lobby.step(), Step::AwaitRunning);
        assert_eq!(rig.at(announced + 40_000 - 1), None);
        assert_eq!(rig.lobby.blocker(), None, "the real window is still open");
        assert!(!rig.lobby.closed());
        // The match runs, read during a second pause.
        let paused = rig.now + 100;
        rig.lobby.pause(paused);
        rig.now = paused + 5_000;
        let running = rig.now;
        rig.hear("Match 1 is running. Good hunting.");
        assert_eq!(rig.lobby.running_ms, Some(running), "really read");
        let resumed = paused + 8_000;
        rig.lobby.resume(resumed);
        assert_eq!(rig.at(resumed).as_deref(), Some("STARTUP"));
        rig.hear("Power online.");
        assert_eq!(rig.lobby.step(), Step::Fight);
        // A drop and a return, both read while paused: five real seconds away.
        rig.lobby.pause(rig.now + 100);
        rig.now += 2_000;
        rig.hear("You are not at the station of a deployed unit.");
        assert_eq!(rig.lobby.step(), Step::Reboard);
        rig.now += 5_000;
        rig.hear("You are already in your pod.");
        assert!(
            rig.said()
                .contains(&"Back aboard after 5 s away.".to_owned())
        );
    }

    /// The lead, paused just after its `SIM START`, through the boarding and
    /// (unless `boarding_only`) the start; resumed, into the team room and
    /// `BOARD` typed; the resume's time.
    fn paused_through_the_start(rig: &mut Rig, boarding_only: bool) -> u64 {
        rig.claim_lead();
        assert_eq!(rig.next(4_000).as_deref(), Some("SIM START"));
        let paused = rig.now + 100;
        rig.lobby.pause(paused);
        rig.now = paused + 1_000;
        rig.hear("Match 1 is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 30 second(s).");
        if !boarding_only {
            rig.now = paused + 41_000;
            rig.hear("Pilot never boarded; red slot 1 is dropped.");
            rig.hear("Match 1 is running. Good hunting.");
        }
        let resumed = paused + 50_000;
        rig.lobby.resume(resumed);
        assert_eq!(rig.at(resumed).as_deref(), Some("red"));
        rig.hear("Red Room #57");
        assert_eq!(rig.next(100).as_deref(), Some("BOARD"));
        resumed
    }

    /// **A `BOARD` answered with no slot in a running match loses it** (cut
    /// AO.2): a pause through the boarding window and the start, then the
    /// game's `You hold no slot in the simulator.` — that match said started
    /// without us, recorded `left` and its end waited for (or, the last one
    /// asked, the run ended as the `no longer in the match` path ends it),
    /// with no blocker; while the match is still boarding the same answer
    /// leaves `BOARD` to its step timeout, as before.
    #[test]
    fn a_board_answered_no_slot_in_a_running_match_loses_it() {
        let two = Settings {
            matches: 2,
            ..Settings::default()
        };
        let timeout = two.step_timeout_ms;
        let mut rig = Rig::with(Side::Red, two, "Pilot");
        paused_through_the_start(&mut rig, false);
        rig.said();
        rig.hear("You hold no slot in the simulator.");
        let said = rig.said();
        assert_eq!(
            said.iter()
                .filter(|s| *s == "Match 1 started without us; waiting for it to end.")
                .count(),
            1,
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|s| s.starts_with("Stopped flying match 1 before its end (left); ")),
            "{said:?}"
        );
        assert_eq!(rig.lobby.matches().len(), 1);
        assert_eq!(rig.lobby.matches()[0].result, "left");
        assert_eq!(rig.lobby.step(), Step::Over);
        assert_eq!(rig.lobby.slot(), None);
        assert_eq!(
            (rig.lobby.flight, rig.lobby.want, rig.lobby.deadline.clone()),
            (None, None, None)
        );
        let t = rig.now;
        assert_eq!(rig.at(t + timeout + 1_000), None);
        assert_eq!(rig.next(60_000), None, "the end is waited for");
        assert_eq!(rig.lobby.blocker(), None);
        assert!(!rig.lobby.closed());
        rig.hear("Match 1 is over (blue). Its units are checked in.");
        let end = rig.now;
        assert_eq!(rig.at(end + REMATCH_AFTER_MS - 1), None);
        assert_eq!(
            rig.at(end + REMATCH_AFTER_MS).as_deref(),
            Some("SIM TEAM red")
        );
        assert_eq!(rig.lobby.matches().len(), 1, "recorded once");
        assert_eq!(rig.lobby.matches()[0].result, "left");
        // The last match asked: said so, recorded, and the run ends there.
        let mut rig = Rig::new(Side::Red);
        paused_through_the_start(&mut rig, false);
        rig.hear("You hold no slot in the simulator.");
        let said = rig.said();
        assert!(
            said.contains(&"Match 1 started without us.".to_owned()),
            "{said:?}"
        );
        assert!(
            !said.iter().any(|s| s.contains("waiting for it to end")),
            "{said:?}"
        );
        assert_eq!(rig.lobby.matches()[0].result, "left");
        assert_eq!(rig.lobby.blocker(), None);
        assert!(rig.lobby.finished());
        // Still boarding: `on_board` as before — nothing said, the flight
        // kept, and the step timeout blocks.
        let mut rig = Rig::new(Side::Red);
        let resumed = paused_through_the_start(&mut rig, true);
        let typed = rig.now;
        assert!(resumed <= typed);
        rig.said();
        rig.hear("You hold no slot in the simulator.");
        assert!(rig.said().is_empty());
        assert_eq!(rig.lobby.step(), Step::Board);
        assert!(rig.lobby.matches().is_empty());
        assert_eq!(rig.at(typed + timeout - 1), None);
        rig.at(typed + timeout);
        assert_eq!(
            rig.lobby.blocker(),
            Some("No answer to `BOARD` within 15 s.")
        );
    }

    /// **Forty busy `STARTUP`s never give a running match up** (cut AO.2):
    /// the forty-first busy answer in a running match types `STARTUP` again
    /// after the step timeout, said, with no blocker (`SIM STATUS` still
    /// blocks: `forty_transients_running_block`).
    #[test]
    fn forty_busy_startups_never_give_a_running_match_up() {
        let mut rig = Rig::new(Side::Red);
        rig.up_to_startup();
        let timeout = Settings::default().step_timeout_ms;
        for _ in 0..TRANSIENT_RETRIES {
            rig.hear("The arena is busy; try that again in a moment.");
            assert_eq!(rig.next(300).as_deref(), Some("STARTUP"));
        }
        rig.said();
        rig.hear("The arena is busy; try that again in a moment.");
        let t = rig.now;
        assert_eq!(rig.lobby.blocker(), None);
        assert_eq!(
            rig.said(),
            ["The server kept answering `STARTUP` as busy; typing it again in 15 s."]
        );
        assert_eq!(rig.at(t + timeout - 1), None);
        assert_eq!(rig.at(t + timeout).as_deref(), Some("STARTUP"));
        assert_eq!(rig.lobby.retries, 0, "a fresh count");
        rig.hear("Power online.");
        assert_eq!(rig.lobby.step(), Step::Fight);
        assert_eq!(rig.lobby.blocker(), None);
    }

    /// **A foreign room's description is not the walk's answer** (cut AO.2):
    /// with `ooc` in flight, another room's heading, its description and its
    /// exits read inside `STALE_TAIL_MS` of each other answer nothing, so the
    /// quiet rule never blocks and the walk's own heading moves it on; a busy
    /// answer inside the look still types the walk's line again; a plain line
    /// read exactly `STALE_TAIL_MS` after the look's last is still the look's;
    /// and a plain line read a millisecond later, a refusal inside the window,
    /// or chat answers the walk, so the quiet rule blocks as before.
    #[test]
    fn a_foreign_rooms_description_is_not_the_walks_answer() {
        let to_ooc = |rig: &mut Rig| -> u64 {
            assert_eq!(rig.at(0).as_deref(), Some("look"));
            rig.hear("Limbo #0");
            rig.hear("A grey nowhere.");
            assert_eq!(rig.next(3_000).as_deref(), Some("ooc"));
            assert_eq!(rig.lobby.step(), Step::Walk);
            rig.now
        };
        let look = |rig: &mut Rig, t: u64| {
            for (at, line) in [
                (t + 100, "Simulator Lobby #50"),
                (t + 400, "A wide hall of pods and boards."),
                (t + 900, "The red and blue doors stand open."),
                (t + 1_200, "Obvious exits: Red Room, Blue Room, out."),
            ] {
                rig.now = at;
                rig.hear(line);
            }
        };
        let mut rig = Rig::new(Side::Red);
        let t = to_ooc(&mut rig);
        look(&mut rig, t);
        assert!(!rig.lobby.answered);
        assert_eq!(rig.next(WALK_SETTLE_MS + QUIET_MS + 3_000), None);
        assert_eq!(rig.lobby.blocker(), None);
        rig.hear("Welcome Hall #2");
        assert_eq!(rig.next(100).as_deref(), Some("simulator"));
        // A busy answer inside the look types the walk's line again.
        let mut rig = Rig::new(Side::Red);
        let t = to_ooc(&mut rig);
        rig.now = t + 100;
        rig.hear("Simulator Lobby #50");
        rig.now = t + 300;
        rig.hear("A wide hall of pods and boards.");
        rig.now = t + 500;
        rig.hear("The command could not commit; please retry in a moment.");
        assert_eq!(rig.at(t + 699), None);
        assert_eq!(rig.at(t + 700).as_deref(), Some("ooc"));
        // The window's edge: a plain line read exactly `STALE_TAIL_MS` after
        // the look's last is still the look's ...
        let mut rig = Rig::new(Side::Red);
        let t = to_ooc(&mut rig);
        rig.now = t + 100;
        rig.hear("Simulator Lobby #50");
        rig.now = t + 400;
        rig.hear("A wide hall of pods and boards.");
        rig.now = t + 400 + lines::STALE_TAIL_MS;
        rig.hear("Somebody's late line.");
        assert!(!rig.lobby.answered, "inside the window");
        // ... and one read a millisecond past it answers the walk: the quiet
        // rule blocks.
        let mut rig = Rig::new(Side::Red);
        let t = to_ooc(&mut rig);
        rig.now = t + 100;
        rig.hear("Simulator Lobby #50");
        rig.now = t + 400;
        rig.hear("A wide hall of pods and boards.");
        rig.now = t + 400 + lines::STALE_TAIL_MS + 1;
        rig.hear("Somebody's late line.");
        assert!(rig.lobby.answered, "past the window");
        rig.next(WALK_SETTLE_MS + QUIET_MS + 100);
        assert_eq!(rig.lobby.blocker(), Some(UNREACHABLE));
        // A refusal read inside the window answers the walk: the quiet rule
        // blocks.
        let mut rig = Rig::new(Side::Red);
        let t = to_ooc(&mut rig);
        rig.now = t + 100;
        rig.hear("Simulator Lobby #50");
        rig.now = t + 400;
        rig.hear("A wide hall of pods and boards.");
        rig.now = t + 700;
        rig.hear("Huh?  (Type \"help\" for help.)");
        assert!(rig.lobby.answered, "a refusal inside the window");
        rig.next(WALK_SETTLE_MS + QUIET_MS + 100);
        assert_eq!(rig.lobby.blocker(), Some(UNREACHABLE));
        // Chat inside the look answers the walk, as any line did.
        let mut rig = Rig::new(Side::Red);
        let t = to_ooc(&mut rig);
        rig.now = t + 100;
        rig.hear("Simulator Lobby #50");
        rig.now = t + 300;
        rig.hear("[Public] Ada: anyone for the sim?");
        assert!(rig.lobby.answered);
        rig.next(WALK_SETTLE_MS + QUIET_MS + 100);
        assert_eq!(rig.lobby.blocker(), Some(UNREACHABLE));
    }
}
