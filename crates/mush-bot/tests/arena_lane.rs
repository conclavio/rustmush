//! **The lane, offline** (cut AJ.2): the bot's arena lane against a scripted
//! server that answers each line it types with the game's own sentences, on a
//! fake clock. No game crate, no network, no model.
//!
//! The server here is a script, not the game: it answers the lobby's lines in
//! the order the centre does (a match ends with the hatch, the team room, then
//! `Match N is over (…)`, because the centre releases the pods before it
//! announces the result), and the cockpit's with fixed rows. What these tests
//! hold is the lane: its limiter on its own clock, its closed vocabulary, its
//! poll cadence and priority, its journal filter, its re-sends, its re-board,
//! its rematch, its one `SIM LEAVE` and its report.
//!
//! Three knobs of the scripted server (cut AK.1), all off by default so every
//! older test runs as before, are set through [`to_the_fight_with`]:
//! `pulse_ms` (a `FIRE` that passes starts its cycle at the next pulse, and the
//! cockpit reads the frame of the last one), `magazine` (rounds that count down
//! one a shot) and `held` (a [`HeldKnob`]: ground that holds the tank until a
//! turn far enough from where it stopped, or a reverse, frees it — an
//! assumption about the server, written from the help and the bot's own
//! transcripts). A fourth, cut AK.2's `hull_internals`, is the hull's
//! internals `STATUS` prints: `(60, 60)` unless a test sets it, before the
//! fight or during it.
//!
//! Cut AL.1 adds, off by default too: `lost` (guns the tank no longer carries:
//! a `FIRE` of one answers the no-such-weapon line, `WEAPONSTATUS` leaves it
//! out) and [`Server::lose_turret`]; `no_chance` (guns whose rows read `0% to
//! hit`); a magazine run dry refuses its `FIRE` as `Empty:`; and per-gun counts
//! (`fire_lines_by`, `cycling_refusals_by`) with each acceptance said (`acks`),
//! for the fire-gap assertions ([`mush_bot::arena::report::fire_gaps`] over the
//! lane's own sends, [`mush_bot::arena::report::shot_gaps`] over the acks).
//!
//! Cut AL.2 adds, off by default too: `ba_at` (where BA is: its `CONTACTS` row
//! and the lock's `RANGE` follow it), `pos` (the position `STATUS` prints,
//! `(60, 970)` unless moved), and [`Server::lose_track`] (`track_gone`: the
//! victim's line, the track's `destroyed` row in `STATUS`, and every `HEADING`
//! turned the live way at 30° a second — an assumption modelled on the bot's
//! own transcript), with a log of each `STATUS` heading printed and each
//! `HEADING` answered.
//!
//! Cut AM.3 renders a `CONTACTS` reply when the server hands it out, not when
//! the line arrives ([`Chunk`]): its rows are the scope as it is then, as the
//! game prints a reply from its frame when it prints it, so an identification
//! handed out before the reply is in its rows and one after it follows it.
//!
//! Cut AM.1 adds, off by default too: `fire_transient` (guns whose next `FIRE`
//! is answered `please retry in a moment` instead of its ack, once),
//! `leave_answer_ms` (how long `SIM LEAVE`'s answer takes: 3 ms unless set),
//! `chatter` (a line said in front of the next chunk the server queues, once)
//! and `lobby_transient` (the next line typed equal to it is answered with the
//! transient, once). It also adds **the latency rig**, [`Wire`]: a seeded
//! model of a distant box between the lane and the server (TCP: ordered,
//! delayed, split), switched on with `run.wire = Some(Wire::new(seed))`; with
//! `None` (every older test) the run is the loopback it always was. **The
//! scripted server can reorder answers**: its per-line delays answer a later
//! line before an earlier one when the earlier one's delay is longer, so a
//! test whose proof depends on the order answers come in keeps them in the
//! order typed and asserts that order.
//!
//! Cut AM.2 adds, off by default too: `claim_answer_ms` (how long the claim
//! takes to answer `SIM TEAM red`: 4 ms unless set), `refuse_claim` (a
//! refusal `SIM TEAM red` is answered with instead, every time),
//! `look_answer_ms` (how long `look`'s answer takes: 3 ms unless set) and
//! `look_room` (the heading `look` answers with, `Limbo #0` unless set).
//!
//! Cut AN.2 adds, off by default too, `brief` (the lines `VIEW BRIEF` is
//! answered with; unset, a header, eight bearings of grass and `No
//! contacts.`), and answers `VIEW BRIEF` as the cockpit line it is. **Every
//! older test flies [`base`]**, the pilot as it was before cut AN.2 (`opening
//! = "search"`, `reapproach = false`; and since cut AN.3 `follow_broken =
//! false`, `lattice = false`); AN.2's and AN.3's own tests fly the shipped
//! defaults, or `base` with the one knob they measure.
//!
//! Cut AN.3 adds, off by default too, beside the AL and AM knobs:
//! `lattice_notices` (with `pulse_ms` set, a shot's outcome line is said at
//! the lattice point the shot went on plus 5 ms, rather than 600 ms after its
//! `FIRE`: the arena's own notices on its lattice, as the bot's transcripts
//! read them), `accept_lag_ms` (a `FIRE` passes the pre-check only at `now ≥
//! start + cycle + accept_lag_ms`: the floor test uses 100, inside the
//! "+50 to +150" an earlier reading of the bot's transcripts put past each
//! gun's ready instant; the measured runs from the muster read the live
//! edge nearer +250, and one test flies 225, past the guard) and
//! [`Wire::steady`] (a wire with a fixed delay each way: no jitter, no split,
//! no injection, TCP order kept).
//!
//! Cut AO.1 adds, off by default too, `ba_plotted_s` (BA's row printed that
//! many seconds old, and the lock's `RANGE` on BA answered as old and as
//! coarse as the row: BA read on the scope's rows alone); and since the room
//! rule moves the opening in from a muster with too little ground behind it,
//! the opening's tests whose subject is not the room fly from (400, 970).
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ops::RangeInclusive,
};

use mush_bot::arena::{
    CommanderReply, Doctrine, Lane, LaneEvent, Purpose, Send, Side,
    commander::{
        ERRORS_IN_ROW, HOLD_MAX_S, OFF_BUDGET, OFF_DESTROYED, OFF_ERRORS, OFF_MATCH_OVER,
        OFF_STALE, Stance,
    },
    lane::{
        FIRE_GAP_MS, LIMIT_WINDOW_MS, POLL_GAP_MS, POLL_STARVE_MS, STATUS_EVERY_MS,
        WEAPONS_AFTER_SHOT_MS, WEAPONS_IDLE_MS, WEAPONS_LEARNED_MS, WEAPONS_LOCKED_MS,
    },
    lines::AskKind,
    lobby::{Centre, REMATCH_AFTER_MS, Step, TRANSIENT_RETRY_MS},
    picture::{
        ACK_PAIR_MS, CYCLE_READ_WITHIN_MS, FIRE_RETRY_MS, FIRE_SLACK_MAX_MS, FIRE_SLACK_START_MS,
        FIRE_SLACK_UP_MS, Phase, Picture, Power, ResultWord, SNAPSHOT_SETTLE_MS, Weapon,
        bearing_deg,
    },
    pilot::{
        ENTRY_KEEP_OUT_M, EXCHANGE_MARGIN, EXCHANGE_MIN_TAKEN, EXCHANGE_WINDOW_MS, FOLLOW_BAND_M,
        FRESH_PLOT_MS, FRESH_READ_MS, HEADING_STEP_DEG, HELD_FOR_MS, HELD_MARK_AHEAD_M,
        HELD_MARK_HALF_WIDTH_M, HELD_MARK_NEAR_M, HELD_TURN_DEG, HOLD_BOW_DEG, HOLD_SLACK_M,
        ONE_TRACK_COMMIT_MS, ONE_TRACK_DEADBAND_DEG, OPENING_ARRIVE_M, OPENING_COVER_M,
        OPENING_ROOM_M, REAPPROACH_MAX_MS, REAPPROACH_OPEN_M, REAPPROACH_WAIT_MS, STALL_MS,
        UNIDENTIFIED_WAIT_MS, WITHDRAW_MS, clamp_arena, hull_failing,
    },
    report::{self, GapStats},
    settings::{Engagement, Opening, Settings},
};
use serde_json::{Value, json};

/// **The settings every test written before cut AN.2 flies**: the shipped
/// defaults with the opening's search and no re-approach, the pilot as it
/// was. Cut AN.2's own tests fly `Settings::default()`.
fn base() -> Settings {
    Settings {
        opening: Opening::Search,
        reapproach: false,
        follow_broken: false,
        lattice: false,
        ..Settings::default()
    }
}

/// The fake clock's step, and the runner's tick.
const STEP_MS: u64 = 10;
const TICK_MS: u64 = 50;

/// The lines the lane may ever type.
fn allowed(line: &str) -> bool {
    const EXACT: [&str; 18] = [
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
        "CONTACTS",
        "WEAPONSTATUS",
        "STATUS",
        "RANGE",
        "BEARING B",
        "VIEW BRIEF",
    ];
    if EXACT.contains(&line) {
        return true;
    }
    let number = |rest: &str| {
        let digits = rest.strip_prefix('-').unwrap_or(rest);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    };
    if let Some(rest) = line.strip_prefix("HEADING ") {
        return number(rest) && !rest.starts_with('-');
    }
    if let Some(rest) = line.strip_prefix("SPEED ") {
        return number(rest);
    }
    if let Some(rest) = line.strip_prefix("LOCK ") {
        return !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric());
    }
    if let Some(rest) = line.strip_prefix("FIRE ") {
        return matches!(rest, "cannon" | "machine_gun");
    }
    false
}

/// No 1 000 ms window holds more than `n` lines: every `n + 1`th line is at
/// least a second after the line `n` before it.
fn limiter_holds(sent: &[Send], n: usize) -> bool {
    sent.windows(n + 1)
        .all(|w| w[n].lane_ms >= w[0].lane_ms + 1_000)
}

/// **The held knob** (cut AK.1). **An assumption about the server, not a copy
/// of it**: modelled on `help cockpit` — "Ground a tank cannot enter stops it,
/// and the cockpit says so once per stop", "`STATUS` carries `HELD` until you
/// order something that frees it", "a negative speed reverses" — and on what
/// the bot's own transcripts show (a held tank turns in place; a `HEADING` well
/// away from where it stopped freed it; a 45° turn and back did not). Nothing
/// of the game's own code decides it.
struct HeldKnob {
    /// A `HEADING` at least this far from the heading the tank was stopped on
    /// frees it.
    free_off_deg: u32,
}

/// The smaller angle between two whole headings.
fn angle_between(a: u32, b: u32) -> u32 {
    let d = a.abs_diff(b) % 360;
    d.min(360 - d)
}

/// **One chunk the scripted server has queued** (cut AM.3): text fixed when
/// it was queued, or a reply rendered from the server's state when
/// [`Server::due`] hands it out, as the game prints a reply from its frame
/// when it prints it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Chunk {
    /// Text, its line ends written.
    Text(String),
    /// A `CONTACTS` reply: the rows [`Server::contacts`] holds at delivery
    /// (BA's from [`Server::ba_at`]), or `No contacts.`.
    Contacts,
    /// An identification: at delivery the scope holds `row` alone, and the
    /// chunk is `Contact {was} identified: BA, blue.`.
    Identify { was: String, row: String },
}

impl Chunk {
    /// A text chunk's text; a rendered one has none until it is handed out.
    fn text(&self) -> &str {
        match self {
            Chunk::Text(text) => text,
            Chunk::Contacts | Chunk::Identify { .. } => "",
        }
    }
}

/// **The scripted server.**
struct Server {
    now: u64,
    queue: Vec<(u64, Chunk)>,
    me: String,
    /// The board's state word.
    board: String,
    match_id: u64,
    running: bool,
    /// Answer cockpit verbs `You are not at the station of a deployed unit.`.
    outside: bool,
    /// The contact rows `CONTACTS` prints now.
    contacts: Vec<String>,
    locked: Option<String>,
    fired: BTreeMap<String, u64>,
    /// Answer the next lobby line with this transient.
    transient: Option<String>,
    /// Refuse the reboard this many times as handing over.
    handing_over: u32,
    /// `SIM STATUS` answers the no-centre refusal.
    no_centre: bool,
    heading: u32,
    /// The speed last ordered; a stop on held ground zeroes it, and a held
    /// tank reads 0 whatever it is.
    speed: i32,
    /// The arena's pulse (cut AK.1): a `FIRE` that passes starts its cycle at
    /// the next multiple of it after it arrives, and the cockpit's pre-check
    /// and the `WEAPONSTATUS` rows read the frame of the last multiple at or
    /// before now (a frame before the shot's pulse shows the gun ready, its
    /// round unspent: the pre-shot frame the picture's guard exists for).
    /// Unset, both are now.
    pulse_ms: Option<u64>,
    /// A magazine of this many rounds a gun, counting down one a shot (so the
    /// picture's rounds rule runs); unset, the rows print `30 of 40`.
    magazine: Option<u32>,
    rounds: BTreeMap<String, u32>,
    /// See [`HeldKnob`]; unset, nothing holds the tank.
    held: Option<HeldKnob>,
    /// The heading the tank was stopped on, while it is held.
    held_from: Option<u32>,
    /// The hull's internals `STATUS` prints, `(left, max)` (cut AK.2);
    /// `(60, 60)` unless a test sets it, at any time.
    hull_internals: (u32, u32),
    /// `FIRE` lines answered, shots that left, and `Still cycling` refusals.
    fire_lines: u32,
    shots: BTreeMap<String, u32>,
    cycling_refusals: u32,
    /// The same two counts per gun (cut AL.1), and each `Firing {w}.` said,
    /// at the time it is due.
    fire_lines_by: BTreeMap<String, u32>,
    cycling_refusals_by: BTreeMap<String, u32>,
    acks: Vec<(u64, String)>,
    /// Each `Still cycling: N s to go.` said (cut AL.1): when it is due, the
    /// gun, and its `N`.
    cycling_refusals_at: Vec<(u64, String, u64)>,
    /// Guns the tank has lost (cut AL.1): a `FIRE` of one answers ``That unit
    /// carries no weapon called `{w}`.``, `WEAPONSTATUS` leaves it out, and
    /// with none left it answers `This unit carries no weapon.`. Empty unless
    /// a test loses one (or [`Server::lose_turret`]).
    lost: BTreeSet<String>,
    /// Guns whose rows read `0% to hit` while locked (cut AL.1). Empty unless
    /// a test sets one.
    no_chance: BTreeSet<String>,
    /// How long a `FIRE`'s answer (its ack or its refusal) takes (cut AL.1):
    /// 3 ms unless a test slows it past a tick, as a remote server may.
    fire_answer_ms: u64,
    /// **Where BA is** (cut AL.2): `(bearing, range)` from the tank. Set, every
    /// `CONTACTS` reply prints BA's row from it — the range rounded to the
    /// scope's 50 m, `rel` the bearing less the heading the tank has now — and
    /// the lock's `RANGE` answers the range whole. Unset, the rows a test
    /// wrote and `440 m`.
    ba_at: Option<(u32, u32)>,
    /// The position `STATUS` prints, metres east and north (cut AL.2):
    /// `(60, 970)` unless a test moves it.
    pos: (i32, i32),
    /// **One track gone** (cut AL.2; [`Server::lose_track`]): `STATUS` prints
    /// its `destroyed` row, and every `HEADING` is turned the live way (the
    /// long way round for a heading on the dead side) at 30° a second, the
    /// heading `STATUS` prints moving with it. **An assumption about the
    /// server, modelled on the bot's own transcript** (a right-track tank
    /// ordered `HEADING 150` from about 352° went 307° → 224° → 164°
    /// counter-clockwise, about 30° a second), not a copy of it. Unset, a
    /// `HEADING` turns the tank at once, as before.
    track_gone: Option<&'static str>,
    /// The turn in progress with one track gone: from, to, when it began.
    turning: Option<(u32, u32, u64)>,
    /// Each `STATUS` answered (cut AL.2): when its reply is due, and the
    /// heading it prints.
    status_log: Vec<(u64, u32)>,
    /// Each `HEADING` answered (cut AL.2): when, the heading the tank had,
    /// and the heading ordered.
    heading_log: Vec<(u64, u32, u32)>,
    /// Guns whose next `FIRE` is answered with [`TRANSIENT`] instead of its
    /// ack, once each, after `fire_answer_ms` (cut AM.1). Empty unless set.
    fire_transient: BTreeSet<String>,
    /// How long `SIM LEAVE`'s answer takes (cut AM.1): 3 ms unless set.
    leave_answer_ms: u64,
    /// Said in front of the next chunk the server queues, once (cut AM.1).
    chatter: Option<String>,
    /// The next line typed equal to this is answered with [`TRANSIENT`],
    /// once (cut AM.1).
    lobby_transient: Option<String>,
    /// How long `SIM TEAM red`'s answer (the claim) takes (cut AM.2): 4 ms
    /// unless set.
    claim_answer_ms: u64,
    /// `SIM TEAM red` is answered with this refusal instead of the claim,
    /// every time (cut AM.2). Unset unless a test sets it.
    refuse_claim: Option<String>,
    /// How long `look`'s answer takes (cut AM.2): 3 ms unless set.
    look_answer_ms: u64,
    /// The room heading `look` answers with (cut AM.2), a description line
    /// after it; unset, `Limbo #0`.
    look_room: Option<String>,
    /// **What `VIEW BRIEF` answers** (cut AN.2), 3 ms on: these lines when
    /// set; unset, a header (`Window around RA: cells 0 to 38 east and 88 to
    /// 107 north of 256 by 256; you at (6, 97), heading 090°; 10 m a cell.`),
    /// the eight bearings as `{Bearing}: 90 m of grass, then beyond the
    /// window.` and `No contacts.`. The server never moves the tank: a test
    /// that needs the opening's step to arrive moves `pos` itself.
    brief: Vec<String>,
    /// **The arena's notices on its lattice** (cut AN.3): with `pulse_ms`
    /// set, a shot's `Hit on {lock}: turret.` / `Miss on {lock}: the target
    /// moved.` is said at the pulse the shot went on plus 5 ms, not 600 ms
    /// after its `FIRE`. Off unless a test sets it.
    lattice_notices: bool,
    /// **The acceptance lag** (cut AN.3): a `FIRE` passes the pre-check only
    /// at `now ≥ start + cycle + accept_lag_ms` of the gun's last shot as
    /// well (an earlier reading of the transcripts put the live edge at +50
    /// to +150; the muster runs read it nearer +250); 0, the rule as before,
    /// unless a test sets it.
    accept_lag_ms: u64,
    /// **BA's plot age** (cut AO.1): with `ba_at` set, BA's row prints `{s}
    /// s` and the lock's `RANGE` answers `Range to BA: {r} m, plotted {s} s
    /// ago.` with the range rounded to the scope's 50 m, as its row is: a plot
    /// a couple of seconds old "reads no better than the rest of the scope"
    /// (`help cockpit`, "Seeing"), so BA is read on the scope's rows alone.
    /// Unset, `1 s` and the whole range, as before.
    ba_plotted_s: Option<u32>,
}

/// The transient the cut AM.1 knobs answer with.
const TRANSIENT: &str = "The world is committing another action; please retry in a moment.";

impl Server {
    fn new() -> Server {
        Server {
            now: 0,
            queue: Vec::new(),
            me: "Pilot".into(),
            board: "idle".into(),
            match_id: 1,
            running: false,
            outside: false,
            contacts: Vec::new(),
            locked: None,
            fired: BTreeMap::new(),
            transient: None,
            handing_over: 0,
            no_centre: false,
            heading: 90,
            speed: 0,
            pulse_ms: None,
            magazine: None,
            rounds: BTreeMap::new(),
            held: None,
            held_from: None,
            hull_internals: (60, 60),
            fire_lines: 0,
            shots: BTreeMap::new(),
            cycling_refusals: 0,
            fire_lines_by: BTreeMap::new(),
            cycling_refusals_by: BTreeMap::new(),
            acks: Vec::new(),
            cycling_refusals_at: Vec::new(),
            lost: BTreeSet::new(),
            no_chance: BTreeSet::new(),
            fire_answer_ms: 3,
            ba_at: None,
            pos: (60, 970),
            track_gone: None,
            turning: None,
            status_log: Vec::new(),
            heading_log: Vec::new(),
            fire_transient: BTreeSet::new(),
            leave_answer_ms: 3,
            chatter: None,
            lobby_transient: None,
            claim_answer_ms: 4,
            refuse_claim: None,
            look_answer_ms: 3,
            look_room: None,
            brief: Vec::new(),
            lattice_notices: false,
            accept_lag_ms: 0,
            ba_plotted_s: None,
        }
    }

    /// **A track goes** (cut AL.2): the victim's line, and the knob set.
    fn lose_track(&mut self, track: &'static str) {
        self.say(
            0,
            &format!("The {track} is destroyed: the tank is slowed; it will not turn that way."),
        );
        self.turning = Some((self.heading, self.heading, self.now));
        self.track_gone = Some(track);
    }

    /// The live turn's compass sign with one track gone: `+1` clockwise when
    /// the left track is gone, `-1` counter-clockwise when the right is.
    fn live_way(&self) -> Option<i64> {
        self.track_gone
            .map(|track| if track == "left_track" { 1 } else { -1 })
    }

    /// The heading the tank has now: the last ordered, or with one track gone
    /// as far round the live way as 30° a second has turned it.
    fn heading_now(&self) -> u32 {
        let (Some(way), Some((from, to, since))) = (self.live_way(), self.turning) else {
            return self.heading;
        };
        let whole_turn = (i64::from(to) - i64::from(from)) * way;
        let whole_turn = whole_turn.rem_euclid(360);
        let turned = (i64::try_from(self.now.saturating_sub(since)).unwrap_or(i64::MAX) * 30
            / 1_000)
            .min(whole_turn);
        (i64::from(from) + way * turned).rem_euclid(360) as u32
    }

    /// BA's row from [`Server::ba_at`], as the scope prints it now.
    fn ba_row(&self, bearing: u32, range: u32) -> String {
        let rel = (i64::from(bearing) - i64::from(self.heading_now())).rem_euclid(360);
        let shown = (range + 25) / 50 * 50;
        let age = self.ba_plotted_s.unwrap_or(1);
        format!("BA  blue  bearing {bearing:03}° (rel {rel:03}°)  far  {shown} m  {age} s")
    }

    /// **The turret goes** (cut AL.1): the victim's line, and both guns with
    /// it. An assumption about the server written from the bot's own
    /// transcripts (the line, then every `FIRE` answered as a gun not carried,
    /// and `WEAPONSTATUS` reading `This unit carries no weapon.`).
    fn lose_turret(&mut self) {
        self.say(0, "The turret is destroyed: its weapons are gone.");
        self.lost.insert("cannon".into());
        self.lost.insert("machine_gun".into());
    }

    /// The frame the cockpit reads: the last pulse at or before now.
    fn frame(&self) -> u64 {
        match self.pulse_ms {
            Some(pulse) => self.now / pulse * pulse,
            None => self.now,
        }
    }

    /// Stop the tank on ground it cannot enter (the `held` knob must be set):
    /// the notice, once, and the speed gone — freed, the tank makes only a
    /// speed ordered after the stop.
    fn hold_tank(&mut self) {
        assert!(self.held.is_some(), "the held knob is off");
        self.held_from = Some(self.heading);
        self.speed = 0;
        self.say(0, "Held: the ground ahead will not take the tank.");
    }

    fn say(&mut self, delay: u64, text: &str) {
        let mut chunk = String::new();
        // The `chatter` knob (cut AM.1): in front of this chunk, once.
        if let Some(chatter) = self.chatter.take() {
            chunk.push_str(&chatter);
            chunk.push_str("\r\n");
        }
        for line in text.lines() {
            chunk.push_str(line);
            chunk.push_str("\r\n");
        }
        self.queue.push((self.now + delay, Chunk::Text(chunk)));
    }

    /// Queue a chunk rendered at delivery (cut AM.3), `delay` on; the
    /// `chatter` knob goes in front of it as its own text, once.
    fn queue_rendered(&mut self, delay: u64, chunk: Chunk) {
        if let Some(chatter) = self.chatter.take() {
            self.queue
                .push((self.now + delay, Chunk::Text(format!("{chatter}\r\n"))));
        }
        self.queue.push((self.now + delay, chunk));
    }

    /// Lines ended as [`Server::say`] ends them.
    fn lines_ended(text: &str) -> String {
        text.lines().map(|line| format!("{line}\r\n")).collect()
    }

    /// **A chunk as it is handed out at `at`** (cut AM.3): text as queued; a
    /// `CONTACTS` reply from the scope as it is then (BA's row from where the
    /// knob puts it, its `rel` from the heading the tank has then); an
    /// identification swaps the scope's rows for its one row, then says so.
    fn render(&mut self, at: u64, chunk: Chunk) -> String {
        match chunk {
            Chunk::Text(text) => text,
            Chunk::Contacts => {
                let asked = self.now;
                self.now = at;
                let rows: Vec<String> = self
                    .contacts
                    .iter()
                    .map(|row| match self.ba_at {
                        Some((bearing, range)) if row.starts_with("BA ") => {
                            self.ba_row(bearing, range)
                        }
                        _ => row.clone(),
                    })
                    .collect();
                self.now = asked;
                if rows.is_empty() {
                    Server::lines_ended("No contacts.")
                } else {
                    Server::lines_ended(&rows.join("\n"))
                }
            }
            Chunk::Identify { was, row } => {
                self.contacts = vec![row];
                Server::lines_ended(&format!("Contact {was} identified: BA, blue."))
            }
        }
    }

    fn due(&mut self, now: u64) -> Vec<String> {
        self.due_at(now).into_iter().map(|(_, text)| text).collect()
    }

    /// [`Server::due`] with the time each chunk was due (cut AM.1: the
    /// [`Wire`] carries each from then). Each chunk is rendered as it is
    /// handed out, in delivery-time order (cut AM.3), so a swap due before a
    /// `CONTACTS` reply in the same call is seen by it: the rig steps 10 ms,
    /// and only this keeps a 1 ms offset exact.
    fn due_at(&mut self, now: u64) -> Vec<(u64, String)> {
        let (due, later): (Vec<_>, Vec<_>) = self.queue.drain(..).partition(|(at, _)| *at <= now);
        self.queue = later;
        let mut due = due;
        due.sort_by_key(|(at, _)| *at);
        due.into_iter()
            .map(|(at, chunk)| (at, self.render(at, chunk)))
            .collect()
    }

    /// A gun's cycle, as the scripted guns have it.
    fn cycle_ms(weapon: &str) -> u64 {
        if weapon == "cannon" { 6_000 } else { 1_000 }
    }

    /// Whether the gun's last shot is still waiting for its pulse: as of the
    /// frame the cockpit reads, it has not left yet.
    fn pending(&self, weapon: &str) -> bool {
        self.fired
            .get(weapon)
            .is_some_and(|start| self.frame() < *start)
    }

    /// What is left of the gun's cycle as of the frame the cockpit reads. A
    /// frame before the shot's pulse shows the gun as it was then: ready (the
    /// `FIRE` passed the pre-check on it), its round not yet gone.
    fn cycle_left(&self, weapon: &str) -> Option<u64> {
        let cycle = Server::cycle_ms(weapon);
        let fired = *self.fired.get(weapon)?;
        let at = self.frame();
        (fired <= at && at < fired + cycle).then(|| fired + cycle - at)
    }

    /// The pre-check a `FIRE` meets: the cycle as of the frame, and — an
    /// assumption, the stricter reading — a shot still waiting for its pulse
    /// counts as cycling from that frame, so a second `FIRE` of the gun inside
    /// one pulse is refused.
    fn pre_check_left(&self, weapon: &str) -> Option<u64> {
        if self.pending(weapon) {
            let fired = self.fired.get(weapon).copied().unwrap_or(0);
            return Some(fired + Server::cycle_ms(weapon) - self.frame());
        }
        let left = self.cycle_left(weapon);
        // Cut AN.3: the acceptance lag past the ready instant.
        if self.accept_lag_ms > 0
            && let Some(start) = self.fired.get(weapon)
        {
            let accepted = start + Server::cycle_ms(weapon) + self.accept_lag_ms;
            if self.now < accepted {
                return Some(left.unwrap_or(0).max(accepted - self.now));
            }
        }
        left
    }

    /// The rounds the frame shows: a shot waiting for its pulse has not
    /// spent its round yet.
    fn rounds_shown(&self, weapon: &str, size: u32) -> u32 {
        let left = self.rounds.get(weapon).copied().unwrap_or(size);
        left + u32::from(self.pending(weapon))
    }

    fn weapon_row(&self, weapon: &str, short: &str) -> String {
        let state = match self.cycle_left(weapon) {
            Some(left) => format!("cycling ({} s)", left.div_ceil(1_000)),
            None => "ready".into(),
        };
        let chance = match self.locked {
            Some(_) if self.no_chance.contains(weapon) => "  band 3 of 3  0% to hit",
            Some(_) => "  band 1 of 3  62% to hit",
            None => "",
        };
        let rounds = match self.magazine {
            Some(size) => format!("{} of {size}", self.rounds_shown(weapon, size)),
            None => "30 of 40".into(),
        };
        format!("{weapon}{short}  {state}  {rounds} round(s){chance}")
    }

    /// The answer to one typed line: its echo first, as the PTY does.
    fn answer(&mut self, now: u64, line: &str) {
        self.now = now;
        self.say(1, line);
        let lobby = !matches!(
            line.split_whitespace().next().unwrap_or(""),
            "CONTACTS"
                | "STATUS"
                | "WEAPONSTATUS"
                | "RANGE"
                | "BEARING"
                | "HEADING"
                | "SPEED"
                | "LOCK"
                | "FIRE"
                | "VIEW"
        );
        if lobby && let Some(transient) = self.transient.take() {
            self.say(3, &transient);
            return;
        }
        if self.lobby_transient.as_deref() == Some(line) {
            self.lobby_transient = None;
            self.say(3, TRANSIENT);
            return;
        }
        let me = self.me.clone();
        let id = self.match_id;
        match line {
            "look" => match self.look_room.clone() {
                Some(room) => self.say(
                    self.look_answer_ms,
                    &format!("{room}\nThe room you are in."),
                ),
                None => self.say(self.look_answer_ms, "Limbo #0\nA grey place with no edges."),
            },
            "ooc" => self.say(3, "Welcome Hall #2\nA bright hall."),
            "simulator" => self.say(3, "Simulator Lobby #50\nThe lobby of the simulator."),
            "SIM STATUS" => {
                if self.no_centre {
                    self.say(3, "This server has no simulator centre.");
                } else {
                    let board = self.board.clone();
                    self.say(
                        3,
                        &format!(
                            "Simulator centre: {board}\n  red: nobody\n  blue: nobody\n  AI fill: each side fields 1; machines take the unclaimed pods."
                        ),
                    );
                }
            }
            "SIM TEAM red" if self.refuse_claim.is_some() => {
                let refusal = self.refuse_claim.clone().unwrap_or_default();
                self.say(3, &refusal);
            }
            "SIM TEAM red" => {
                if self.board != "idle" && self.board != "selection" {
                    self.say(3, "The last match is cleared from the board and the tanks are back at muster.");
                }
                self.board = "selection".into();
                self.say(
                    self.claim_answer_ms,
                    &format!("{me} takes red slot 1. `SIM START` begins the match; the machines take the pods nobody claims."),
                );
            }
            "SIM START" => {
                self.board = "boarding".into();
                self.say(3, &format!("Match {id} is boarding. Every pilot: BOARD your pod. The match starts when everyone is seated, or in 30 second(s)."));
            }
            "red" => self.say(3, "Red Room #57\nThe red team room."),
            "BOARD" => {
                if self.running && self.handing_over > 0 {
                    self.handing_over -= 1;
                    self.say(
                        3,
                        "Red Tank 1 is still being handed over; try again in 1 second(s).",
                    );
                } else if self.running && self.outside {
                    self.outside = false;
                    self.say(3, "Back aboard RA after 4 s away.\n  Power online.");
                } else {
                    self.say(3, "You climb into Red Pod 1. The panel wakes when every pilot is seated, or in 29 second(s).\nRed Pod 1 #60");
                    self.board = "running".into();
                    self.running = true;
                    self.say(200, &format!("Match {id} is running. Good hunting."));
                }
            }
            "STARTUP" => {
                self.say(3, "Startup sequence begun; power comes online shortly.");
                self.say(1_000, "Power online.");
            }
            "SIM LEAVE" => {
                self.board = "idle".into();
                self.say(self.leave_answer_ms, &format!("{me} gives up red slot 1."));
            }
            _ if self.outside => self.say(3, "You are not at the station of a deployed unit."),
            // The reply's rows are read from the scope when it is handed out
            // (cut AM.3), BA's from where the knob puts it (cut AL.2).
            "CONTACTS" => self.queue_rendered(3, Chunk::Contacts),
            "STATUS" => {
                // Held, the tank reads 0 kph and `HELD` on the heading it last
                // turned to in place.
                let (speed, held) = match self.held_from {
                    Some(_) => (0, ", HELD"),
                    None => (self.speed, ""),
                };
                let heading = self.heading_now();
                self.status_log.push((self.now + 3, heading));
                let (left, max) = self.hull_internals;
                let condition = if left < max { "damaged" } else { "intact" };
                let (x, y) = self.pos;
                let track = match self.track_gone {
                    Some(track) => format!("\n  {track}  destroyed  internals 0/24"),
                    None => String::new(),
                };
                self.say(
                    3,
                    &format!(
                        "Callsign RA.\nRed Tank 1: power online, {speed} kph, heading {heading:03}°{held}\n  at {x} m east, {y} m north; scope holds a hostile\n  nearest spot: The Ford (A), 400 m at 090°\n  hull  {condition}  internals {left}/{max}\n    front 400/400  left 250/250  right 250/250  rear 150/150\n  turret  intact\n    front 350/350  left 220/220  right 220/220  rear 140/140{track}"
                    ),
                );
            }
            "WEAPONSTATUS" => {
                let rows: Vec<String> = [("cannon", ""), ("machine_gun", " (mg)")]
                    .into_iter()
                    .filter(|(weapon, _)| !self.lost.contains(*weapon))
                    .map(|(weapon, short)| self.weapon_row(weapon, short))
                    .collect();
                if rows.is_empty() {
                    self.say(3, "This unit carries no weapon.");
                } else {
                    let mut reply = String::new();
                    if let Some(lock) = &self.locked {
                        reply.push_str(&format!("Locked on {lock}, 62% to hit.\n"));
                    }
                    reply.push_str(&rows.join("\n"));
                    self.say(3, &reply);
                }
            }
            "RANGE" => match self.locked.clone() {
                Some(lock) => {
                    let range = match self.ba_at {
                        Some((_, range)) if lock == "BA" => range,
                        _ => 440,
                    };
                    // Cut AO.1: BA's plot `ba_plotted_s` old reads as its row.
                    match self.ba_plotted_s {
                        Some(age) if lock == "BA" && self.ba_at.is_some() => {
                            let shown = (range + 25) / 50 * 50;
                            self.say(
                                3,
                                &format!("Range to {lock}: {shown} m, plotted {age} s ago."),
                            );
                        }
                        _ => self.say(3, &format!("Range to {lock}: {range} m.")),
                    }
                }
                None => self.say(3, "No lock: LOCK a contact first, or name one."),
            },
            "BEARING B" => self.say(3, "Bearing to The Crossroads (B): 090° (rel 000°)."),
            // Cut AN.2: the brief, the knob's or a field of grass.
            "VIEW BRIEF" => {
                let lines = if self.brief.is_empty() {
                    let mut lines = vec![
                        "Window around RA: cells 0 to 38 east and 88 to 107 north of 256 by 256; \
                         you at (6, 97), heading 090°; 10 m a cell."
                            .to_owned(),
                    ];
                    for bearing in [
                        "North",
                        "North-east",
                        "East",
                        "South-east",
                        "South",
                        "South-west",
                        "West",
                        "North-west",
                    ] {
                        lines.push(format!("{bearing}: 90 m of grass, then beyond the window."));
                    }
                    lines.push("No contacts.".to_owned());
                    lines
                } else {
                    self.brief.clone()
                };
                self.say(3, &lines.join("\n"));
            }
            _ => {
                if let Some(deg) = line.strip_prefix("HEADING ") {
                    let was = self.heading_now();
                    self.heading = deg.parse().unwrap_or(0);
                    self.heading_log.push((self.now, was, self.heading));
                    // One track gone: the turn starts from where the tank is.
                    if self.track_gone.is_some() {
                        self.turning = Some((was, self.heading, self.now));
                    }
                    // Far enough from where it stopped, the turn frees it;
                    // anything nearer leaves it held, silently.
                    if let (Some(from), Some(knob)) = (self.held_from, &self.held)
                        && angle_between(self.heading, from) >= knob.free_off_deg
                    {
                        self.held_from = None;
                    }
                    self.say(3, &format!("Steering to {deg}°."));
                } else if let Some(kph) = line.strip_prefix("SPEED ") {
                    let kph: i32 = kph.parse().unwrap_or(0);
                    if kph > 54 {
                        self.say(
                            3,
                            "That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.",
                        );
                    } else {
                        self.speed = kph;
                        // Astern frees a held tank; ahead does not.
                        if kph < 0 {
                            self.held_from = None;
                        }
                        self.say(3, &format!("Making {kph} kph."));
                    }
                } else if let Some(target) = line.strip_prefix("LOCK ") {
                    self.say(3, &format!("Locking {target}."));
                    self.locked = Some(target.to_owned());
                    self.say(500, &format!("Locked on {target}, 62% to hit."));
                } else if let Some(weapon) = line.strip_prefix("FIRE ") {
                    self.fire_lines += 1;
                    *self.fire_lines_by.entry(weapon.to_owned()).or_default() += 1;
                    // The `fire_transient` knob (cut AM.1): the line did not
                    // commit, so nothing of the gun moves.
                    if self.fire_transient.remove(weapon) {
                        self.say(self.fire_answer_ms, TRANSIENT);
                        return;
                    }
                    let empty = self
                        .magazine
                        .is_some_and(|_| self.rounds.get(weapon) == Some(&0));
                    if self.lost.contains(weapon) {
                        self.say(
                            self.fire_answer_ms,
                            &format!("That unit carries no weapon called `{weapon}`."),
                        );
                    } else if self.locked.is_none() {
                        self.say(
                            self.fire_answer_ms,
                            &format!("FIRE {weapon} refused. No lock: LOCK a contact first."),
                        );
                    } else if let Some(left) = self.pre_check_left(weapon) {
                        self.cycling_refusals += 1;
                        *self
                            .cycling_refusals_by
                            .entry(weapon.to_owned())
                            .or_default() += 1;
                        let secs = left.div_ceil(1_000);
                        self.say(
                            self.fire_answer_ms,
                            &format!("FIRE {weapon} refused. Still cycling: {secs} s to go."),
                        );
                        self.cycling_refusals_at.push((
                            self.now + self.fire_answer_ms,
                            weapon.to_owned(),
                            secs,
                        ));
                    } else if empty {
                        self.say(
                            self.fire_answer_ms,
                            &format!(
                                "FIRE {weapon} refused. Empty: that weapon has nothing left to fire."
                            ),
                        );
                    } else {
                        // The arena fires on its pulse: the cycle starts at
                        // the next one.
                        let start = match self.pulse_ms {
                            Some(pulse) => (self.now / pulse + 1) * pulse,
                            None => self.now,
                        };
                        self.fired.insert(weapon.to_owned(), start);
                        *self.shots.entry(weapon.to_owned()).or_default() += 1;
                        if let Some(size) = self.magazine {
                            let left = self.rounds.entry(weapon.to_owned()).or_insert(size);
                            *left = left.saturating_sub(1);
                        }
                        self.say(self.fire_answer_ms, &format!("Firing {weapon}."));
                        self.acks
                            .push((self.now + self.fire_answer_ms, weapon.to_owned()));
                        let lock = self.locked.clone().unwrap_or_default();
                        // Cut AN.3: on the lattice point the shot went on.
                        let outcome = match (self.lattice_notices, self.pulse_ms) {
                            (true, Some(_)) => start + 5 - self.now,
                            _ => 600,
                        };
                        if weapon == "cannon" {
                            self.say(outcome, &format!("Hit on {lock}: turret."));
                        } else {
                            self.say(outcome, &format!("Miss on {lock}: the target moved."));
                        }
                    }
                }
            }
        }
    }

    /// End the match the way the centre does: the hatch, the room, the result.
    fn end(&mut self, now: u64, word: &str) {
        self.now = now;
        self.running = false;
        self.board = "ended".into();
        let id = self.match_id;
        self.say(
            0,
            "The hatch of Red Pod 1 swings open.\nRed Room #57\nThe red team room.",
        );
        self.say(
            0,
            &format!("Match {id} is over ({word}). Its units are checked in."),
        );
        self.match_id += 1;
        self.locked = None;
        self.contacts.clear();
        self.fired.clear();
        self.rounds.clear();
        self.lost.clear();
        self.held_from = None;
        self.track_gone = None;
        self.turning = None;
    }
}

/// The latency rig's base delay each way (cut AM.1)…
const WIRE_BASE_MS: u64 = 40;
/// …its jitter on top, each way…
const WIRE_JITTER_MS: u64 = 300;
/// …and how far apart the pieces of one split chunk arrive.
const WIRE_SPLIT_GAP_MS: RangeInclusive<u64> = 160..=300;

/// **The latency rig** (cut AM.1): a model of a distant box between the lane
/// and the scripted server — TCP: ordered, delayed and split — and not of the
/// game. Seeded (the xorshift64 of `lines.rs`'s fuzz test), on the fake clock.
///
/// * **Up**: each line the lane types reaches the server [`WIRE_BASE_MS`] and
///   up to [`WIRE_JITTER_MS`] after it was typed, never before the line typed
///   before it; the server answers it then.
/// * **Down**: each chunk the server queues leaves the same delay after it was
///   due, never before the chunk before it. A chunk of two or more lines is
///   split at one line boundary and one mid-line `char` boundary into three
///   pieces, [`WIRE_SPLIT_GAP_MS`] apart, still in order.
/// * **Injection**: every third such chunk that holds no `Callsign ` line
///   (never a `STATUS` block, where a page's continuation would read as a
///   facet run, and the game never puts a line inside a reply) carries one
///   line at a line boundary inside it, alternately `[RedNet] Ada: on your
///   left` and `Receiver clear.`: both neutral.
struct Wire {
    seed: u64,
    /// **A steady wire** (cut AN.3; [`Wire::steady`]): this delay each way,
    /// and no jitter, no split, no injection.
    steady: Option<u64>,
    /// When the last line typed reached the server, and when the last piece
    /// left down the wire.
    up_last: u64,
    down_last: u64,
    up: VecDeque<(u64, String)>,
    down: VecDeque<(u64, String)>,
    /// The chunks the server had queued before the wire went on: each the
    /// old way, at its own time, rendered by the server then (cut AM.3).
    direct: Vec<(u64, Chunk)>,
    started: bool,
    /// Split chunks with no `Callsign ` line, for the injections.
    eligible: u32,
    chatter_next: bool,
    chunks: u32,
    split_chunks: u32,
    splits: u32,
    mid_line_splits: u32,
    gaps_over_settle: u32,
    injections: u32,
    up_delay_ms: Vec<u64>,
    down_delay_ms: Vec<u64>,
    /// Each line's arrival up, and each piece's delivery down, in order.
    arrivals: Vec<u64>,
    deliveries: Vec<u64>,
    /// Each chunk carried: whether it held a `Callsign ` line, and whether a
    /// line was injected into it.
    carried: Vec<(bool, bool)>,
}

impl Wire {
    fn new(seed: u64) -> Wire {
        Wire {
            seed: seed.max(1),
            steady: None,
            up_last: 0,
            down_last: 0,
            up: VecDeque::new(),
            down: VecDeque::new(),
            direct: Vec::new(),
            started: false,
            eligible: 0,
            chatter_next: true,
            chunks: 0,
            split_chunks: 0,
            splits: 0,
            mid_line_splits: 0,
            gaps_over_settle: 0,
            injections: 0,
            up_delay_ms: Vec::new(),
            down_delay_ms: Vec::new(),
            arrivals: Vec::new(),
            deliveries: Vec::new(),
            carried: Vec::new(),
        }
    }

    /// **A steady wire** (cut AN.3): `one_way_ms` each way, every time; no
    /// jitter, no split, no injection; TCP order kept.
    fn steady(one_way_ms: u64) -> Wire {
        Wire {
            steady: Some(one_way_ms),
            ..Wire::new(1)
        }
    }

    fn next(&mut self) -> u64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        self.seed
    }

    fn rand(&mut self, range: RangeInclusive<u64>) -> u64 {
        let (lo, hi) = (*range.start(), *range.end());
        lo + self.next() % (hi - lo + 1)
    }

    fn delay(&mut self) -> u64 {
        match self.steady {
            Some(one_way_ms) => one_way_ms,
            None => WIRE_BASE_MS + self.rand(0..=WIRE_JITTER_MS),
        }
    }

    /// The lane typed `line` at `sent`.
    fn send(&mut self, sent: u64, line: &str) {
        let arrive = (sent + self.delay()).max(self.up_last);
        self.up_last = arrive;
        self.up_delay_ms.push(arrive - sent);
        self.up.push_back((arrive, line.to_owned()));
    }

    /// The lines that have reached the server by `now`, each with its time.
    fn arrived(&mut self, now: u64) -> Vec<(u64, String)> {
        let mut out = Vec::new();
        while self.up.front().is_some_and(|(at, _)| *at <= now) {
            if let Some((at, line)) = self.up.pop_front() {
                self.arrivals.push(at);
                out.push((at, line));
            }
        }
        out
    }

    /// The server's chunk, due at `queued`, onto the wire.
    fn carry(&mut self, queued: u64, chunk: String) {
        self.chunks += 1;
        let leave = (queued + self.delay()).max(self.down_last);
        self.down_delay_ms.push(leave - queued);
        let status = chunk.contains("Callsign ");
        let mut lines: Vec<String> = chunk.split_inclusive('\n').map(str::to_owned).collect();
        let mut injected = false;
        if lines.len() < 2 || self.steady.is_some() {
            self.down.push_back((leave, chunk));
            self.down_last = leave;
            self.carried.push((status, injected));
            return;
        }
        self.split_chunks += 1;
        if !status {
            self.eligible += 1;
            if self.eligible.is_multiple_of(3) {
                let at = self.rand(1..=(lines.len() - 1) as u64) as usize;
                let line = if self.chatter_next {
                    "[RedNet] Ada: on your left"
                } else {
                    "Receiver clear."
                };
                self.chatter_next = !self.chatter_next;
                lines.insert(at, format!("{line}\r\n"));
                injected = true;
                self.injections += 1;
            }
        }
        // The cuts: after one line, and inside one line's own text.
        let n = lines.len() as u64;
        let starts: Vec<usize> = lines
            .iter()
            .scan(0, |at, line| {
                let start = *at;
                *at += line.len();
                Some(start)
            })
            .collect();
        let boundary = self.rand(1..=n - 1) as usize;
        let mut cuts = vec![starts[boundary]];
        let k = self.rand(0..=n - 1) as usize;
        let text = lines[k].trim_end_matches(['\r', '\n']);
        let chars = text.chars().count() as u64;
        if chars >= 2 {
            let j = self.rand(1..=chars - 1) as usize;
            let offset = text.char_indices().nth(j).map_or(0, |(i, _)| i);
            cuts.push(starts[k] + offset);
            self.mid_line_splits += 1;
        }
        cuts.sort_unstable();
        cuts.dedup();
        let whole: String = lines.concat();
        let mut at = leave;
        let mut from = 0;
        for cut in cuts.iter().copied().chain([whole.len()]) {
            if cut > from {
                if from > 0 {
                    let gap = self.rand(WIRE_SPLIT_GAP_MS);
                    if gap > SNAPSHOT_SETTLE_MS {
                        self.gaps_over_settle += 1;
                    }
                    at += gap;
                }
                self.down.push_back((at, whole[from..cut].to_owned()));
                from = cut;
            }
        }
        self.splits += cuts.len() as u32;
        self.down_last = at;
        self.carried.push((status, injected));
    }

    /// The chunks queued before the wire went on that are due by `now`, in
    /// order, for the server to render as it hands them out (cut AM.3); they
    /// go in front of [`Wire::delivered`]'s.
    fn direct_due(&mut self, now: u64) -> Vec<(u64, Chunk)> {
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.direct)
            .into_iter()
            .partition(|(at, _)| *at <= now);
        self.direct = later;
        let mut due = due;
        due.sort_by_key(|(at, _)| *at);
        due
    }

    /// The pieces delivered by `now`: the wire's own, in order (the chunks
    /// queued before the wire went on come first, from
    /// [`Wire::direct_due`]).
    fn delivered(&mut self, now: u64) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        while self.down.front().is_some_and(|(at, _)| *at <= now) {
            if let Some((at, text)) = self.down.pop_front() {
                self.deliveries.push(at);
                out.push(text);
            }
        }
        out
    }

    fn mean(delays: &[u64]) -> u64 {
        delays.iter().sum::<u64>() / (delays.len() as u64).max(1)
    }

    /// The mean round trip: the mean delay up and the mean delay down.
    fn mean_round_trip_ms(&self) -> u64 {
        Wire::mean(&self.up_delay_ms) + Wire::mean(&self.down_delay_ms)
    }

    fn report(&self) -> String {
        format!(
            "wire: {} chunk(s), {} split into pieces ({} cut(s), {} mid-line), {} gap(s) over \
             {SNAPSHOT_SETTLE_MS} ms, {} injection(s); mean up {} ms (max {}), mean down {} ms \
             (max {}; the chunk behind a split one waits for its pieces), mean round trip {} ms",
            self.chunks,
            self.split_chunks,
            self.splits,
            self.mid_line_splits,
            self.gaps_over_settle,
            self.injections,
            Wire::mean(&self.up_delay_ms),
            self.up_delay_ms.iter().max().copied().unwrap_or(0),
            Wire::mean(&self.down_delay_ms),
            self.down_delay_ms.iter().max().copied().unwrap_or(0),
            self.mean_round_trip_ms(),
        )
    }
}

/// The lane, the server and the clock.
struct Run {
    lane: Lane,
    server: Server,
    now: u64,
    sent: Vec<Send>,
    journal: Vec<String>,
    said: Vec<String>,
    /// The lane's transcript records (cut AJ.3's commander).
    records: Vec<(String, Value)>,
    /// The latency rig between the lane and the server (cut AM.1); `None`
    /// is the loopback every older test runs on.
    wire: Option<Wire>,
}

impl Run {
    fn new(settings: Settings) -> Run {
        Run {
            lane: Lane::new(Side::Red, settings, "Pilot".into()),
            server: Server::new(),
            now: 0,
            sent: Vec::new(),
            journal: Vec::new(),
            said: Vec::new(),
            records: Vec::new(),
            wire: None,
        }
    }

    fn step(&mut self) {
        self.now += STEP_MS;
        let texts = match self.wire.as_mut() {
            None => self.server.due(self.now),
            Some(wire) => {
                // What the server had queued when the wire went on goes the
                // old way.
                if !wire.started {
                    wire.started = true;
                    wire.direct = std::mem::take(&mut self.server.queue);
                }
                for (arrive, line) in wire.arrived(self.now) {
                    self.server.answer(arrive, &line);
                }
                // The old way first, rendered as the server hands them out.
                let mut texts: Vec<String> = wire
                    .direct_due(self.now)
                    .into_iter()
                    .map(|(at, chunk)| self.server.render(at, chunk))
                    .collect();
                for (at, chunk) in self.server.due_at(self.now) {
                    wire.carry(at, chunk);
                }
                texts.extend(wire.delivered(self.now));
                texts
            }
        };
        for text in texts {
            let fed = self.lane.feed(self.now, &text);
            if !fed.journal.is_empty() {
                self.journal.push(fed.journal);
            }
        }
        if self.now.is_multiple_of(TICK_MS) {
            for send in self.lane.tick(self.now) {
                match self.wire.as_mut() {
                    Some(wire) => wire.send(self.now, &send.line),
                    None => self.server.answer(self.now, &send.line),
                }
                self.sent.push(send);
            }
        }
        for event in self.lane.events() {
            match event {
                LaneEvent::Said(text) => self.said.push(text),
                LaneEvent::Record { kind, data } => self.records.push((kind, data)),
                _ => {}
            }
        }
    }

    /// Run until `done` or `limit_ms` more; whether `done` held.
    fn until(&mut self, limit_ms: u64, done: impl Fn(&Run) -> bool) -> bool {
        let end = self.now + limit_ms;
        while self.now < end {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    fn fighting(&self) -> bool {
        self.lane.step() == Step::Fight && self.lane.picture().own.power == Power::Online
    }

    fn lines(&self) -> Vec<&str> {
        self.sent.iter().map(|s| s.line.as_str()).collect()
    }

    fn sends_of(&self, line: &str, from: u64) -> Vec<u64> {
        self.sent
            .iter()
            .filter(|s| s.line == line && s.lane_ms >= from)
            .map(|s| s.lane_ms)
            .collect()
    }
}

fn to_the_fight(settings: Settings) -> Run {
    to_the_fight_with(settings, |_| {})
}

/// To the fight against a server whose knobs `knobs` sets first.
fn to_the_fight_with(settings: Settings, knobs: impl FnOnce(&mut Server)) -> Run {
    let mut run = Run::new(settings);
    knobs(&mut run.server);
    assert!(
        run.until(60_000, Run::fighting),
        "never reached the fight; sent {:?}, said {:?}",
        run.lines(),
        run.said
    );
    run
}

#[test]
fn the_walk_from_limbo_to_power_online() {
    let run = to_the_fight(base());
    let lines = run.lines();
    assert_eq!(
        lines[..9],
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
    // Straight after `Power online.`: the first `STATUS` and `WEAPONSTATUS`,
    // then `CONTACTS` and the ceilings' `SPEED 999`, in the one tick.
    assert_eq!(
        lines[9..],
        ["STATUS", "WEAPONSTATUS", "CONTACTS", "SPEED 999"]
    );
    let tick = run.sent[9].lane_ms;
    assert!(run.sent[9..].iter().all(|s| s.lane_ms == tick));
    let purposes: Vec<Purpose> = run.sent.iter().map(|s| s.purpose).collect();
    assert_eq!(&purposes[..3], [Purpose::Walk; 3]);
    assert_eq!(purposes[3], Purpose::Lobby);
    assert_eq!(purposes[6], Purpose::Walk, "the team room is a walk");
    for sentence in [
        "Arena pilot flying for red; 1 match(es).",
        "Claimed red slot 1 (RA), leading the side.",
        "Match 1 is boarding; going to the red pod.",
        "Match 1 is running; starting the tank.",
        "Power online; fighting.",
    ] {
        assert!(
            run.said.iter().any(|s| s == sentence),
            "{sentence}: {:?}",
            run.said
        );
    }
}

/// The hostile the scripted scope holds: an unknown first, told apart three
/// seconds later.
fn sight_a_hostile(run: &mut Run) {
    sight(run, "X1");
}

/// `BA` sighted as the unknown `unknown`, told apart three seconds later.
fn sight(run: &mut Run, unknown: &str) {
    run.server.contacts = vec![format!(
        "{unknown}  unknown  bearing 000° (rel 270°)  far  450 m  1 s"
    )];
    run.server.now = run.now;
    run.server
        .say(0, &format!("New contact {unknown}: bearing 000°, far."));
    run.until(3_000, |_| false);
    run.server.contacts = vec!["BA  blue  bearing 000° (rel 270°)  far  450 m  1 s".into()];
    run.server.now = run.now;
    run.server
        .say(0, &format!("Contact {unknown} identified: BA, blue."));
}

#[test]
fn a_five_minute_fight_keeps_under_four_lines_a_second_and_the_vocabulary() {
    let mut run = to_the_fight(base());
    sight_a_hostile(&mut run);
    let start = run.now;
    run.until(300_000, |_| false);
    run.server.end(run.now, "red");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let fight: Vec<Send> = run
        .sent
        .iter()
        .filter(|s| s.lane_ms >= start)
        .cloned()
        .collect();
    assert!(fight.len() > 300, "the pilot kept busy: {}", fight.len());
    assert!(limiter_holds(&run.sent, 4), "a window held five lines");
    assert!(
        run.sent
            .windows(4)
            .any(|w| w[3].lane_ms < w[0].lane_ms + 1_000),
        "the cap was reached: four lines inside one second"
    );
    for send in &run.sent {
        assert!(
            allowed(&send.line),
            "{} is not in the vocabulary",
            send.line
        );
    }
    let lines = run.lines();
    assert!(lines.contains(&"LOCK BA"));
    assert!(lines.contains(&"FIRE cannon"));
    assert!(lines.contains(&"SPEED 999"), "the ceilings asked");
    assert!(lines.iter().any(|l| l.starts_with("HEADING ")));
    assert!(
        !lines.iter().any(|l| l.starts_with("LOCK X")),
        "an unknown is never locked"
    );
    let report = run.lane.report();
    assert_eq!(report.matches.len(), 1);
    let flown = &report.matches[0];
    assert_eq!(flown.result, "red");
    assert!(flown.won && flown.shots > 10 && flown.hits_scored > 5);
    assert_eq!(flown.callsign, "RA");
    assert!(flown.lead);
    let result = run.lane.result().expect("a result");
    assert_eq!(result.status, "done");
    assert!(
        result.outcome.starts_with("Won match 1 for red as RA: "),
        "{}",
        result.outcome
    );
}

#[test]
fn the_poll_cadence() {
    let mut run = to_the_fight(base());
    // Nothing held: `CONTACTS` every five seconds.
    let start = run.now;
    run.until(20_000, |_| false);
    let empty = run.sends_of("CONTACTS", start);
    assert!(empty.len() >= 4, "{empty:?}");
    for pair in empty.windows(2) {
        assert_eq!(pair[1] - pair[0], 5_000, "{empty:?}");
    }
    let statuses = run.sends_of("STATUS", start);
    for pair in statuses.windows(2) {
        assert_eq!(pair[1] - pair[0], 2_000, "{statuses:?}");
    }
    // A new contact: held now, so `CONTACTS` goes at the second's cadence at
    // once (the last one was two seconds ago).
    let count = run.sends_of("CONTACTS", 0).len();
    assert!(run.until(6_000, |run| run.sends_of("CONTACTS", 0).len() > count));
    let last = *run.sends_of("CONTACTS", 0).last().expect("a poll");
    let notice_at = last + 2_000;
    run.until(notice_at - run.now, |_| false);
    assert_eq!(run.now % TICK_MS, 0);
    run.server.contacts = vec!["X1  unknown  bearing 000° (rel 270°)  far  450 m  1 s".into()];
    let fed = run
        .lane
        .feed(run.now, "New contact X1: bearing 000°, far.\r\n");
    assert_eq!(fed.journal, "New contact X1: bearing 000°, far.");
    run.until(5_000, |_| false);
    let held = run.sends_of("CONTACTS", notice_at);
    assert_eq!(held.first(), Some(&(notice_at + TICK_MS)), "{held:?}");
    // Held: every second.
    for pair in held.windows(2) {
        assert_eq!(pair[1] - pair[0], 1_000, "{held:?}");
    }
    // A notice 300 ms after a reply: `CONTACTS` 250 ms after the notice, not
    // at the next second.
    let last = *held.last().expect("a poll");
    let next = last + 1_000;
    run.until(next + 300 - run.now, |_| false);
    assert_eq!(run.sends_of("CONTACTS", next).first(), Some(&next));
    run.server
        .contacts
        .push("X2  unknown  bearing 180° (rel 090°)  far  500 m  1 s".into());
    let notice = run.now;
    let fed = run
        .lane
        .feed(notice, "New contact X2: bearing 180°, far.\r\n");
    assert_eq!(fed.journal, "New contact X2: bearing 180°, far.");
    run.until(1_000, |_| false);
    assert_eq!(
        run.sends_of("CONTACTS", notice).first(),
        Some(&(notice + 250)),
        "250 ms after the notice: {:?}",
        run.sent
            .iter()
            .filter(|s| s.lane_ms + 1_500 >= notice && s.lane_ms <= notice + 1_000)
            .map(|s| (s.lane_ms, s.line.clone()))
            .collect::<Vec<_>>()
    );
    // Told apart: the lock.
    run.server.contacts = vec!["BA  blue  bearing 000° (rel 270°)  far  450 m  1 s".into()];
    run.server.now = run.now;
    run.server.say(0, "Contact X1 identified: BA, blue.");
    // Locked: a bare `RANGE` after every second `CONTACTS` reply.
    run.until(4_000, |run| run.lane.picture().lock.is_some());
    assert_eq!(run.lane.picture().lock.as_deref(), Some("BA"));
    let locked_at = run.now;
    run.until(10_000, |_| false);
    let contacts = run.sends_of("CONTACTS", locked_at);
    let ranges = run.sends_of("RANGE", locked_at);
    assert!(!ranges.is_empty());
    for range in &ranges {
        let before = contacts
            .iter()
            .filter(|c| *c < range)
            .max()
            .expect("a reply first");
        assert!(
            range - before <= 400,
            "RANGE at {range} after CONTACTS at {before}"
        );
    }
    let expected = contacts.len() / 2;
    assert!(
        ranges.len().abs_diff(expected) <= 1,
        "{} ranges for {} replies",
        ranges.len(),
        contacts.len()
    );
    // Locked and ready: the cannon fired, the weapons polled a second after.
    let fires = run.sends_of("FIRE cannon", locked_at);
    assert!(!fires.is_empty());
    let weapons = run.sends_of("WEAPONSTATUS", fires[0] + 1);
    assert!(
        weapons.first().is_some_and(|w| w - fires[0] <= 1_100),
        "a WEAPONSTATUS within 1.1 s of the shot: {weapons:?}"
    );
}

#[test]
fn a_due_fire_beats_a_due_poll() {
    // At one line a second there is always a poll due; the gun still goes.
    let settings = Settings {
        max_lines_per_s: 1,
        ..base()
    };
    let mut run = to_the_fight(settings);
    sight_a_hostile(&mut run);
    assert!(run.until(20_000, |run| run.lines().contains(&"FIRE cannon")));
    let fire = run.sends_of("FIRE cannon", 0)[0];
    let last_contacts = run
        .sent
        .iter()
        .filter(|s| s.line == "CONTACTS" && s.lane_ms < fire)
        .map(|s| s.lane_ms)
        .max()
        .expect("a CONTACTS before the shot");
    assert!(
        !run.lane.picture().contacts.is_empty() && fire >= last_contacts + 1_000,
        "CONTACTS was due at the shot's tick, and the shot went"
    );
    // One a second, and the ring binds: demand outruns it, so lines go exactly
    // a second apart.
    assert!(limiter_holds(&run.sent, 1), "two lines inside one second");
    assert!(
        run.sent
            .windows(2)
            .filter(|w| w[1].lane_ms == w[0].lane_ms + 1_000)
            .count()
            >= 5,
        "the one-a-second ring never bound"
    );
    // At four a second, every tick that sends the main gun sends it first.
    let mut run = to_the_fight(base());
    sight_a_hostile(&mut run);
    run.until(30_000, |_| false);
    let fires = run.sends_of("FIRE cannon", 0);
    assert!(fires.len() >= 3, "{fires:?}");
    for fire in fires {
        let first = run
            .sent
            .iter()
            .find(|s| s.lane_ms == fire)
            .expect("the tick's lines");
        assert_eq!(first.line, "FIRE cannon", "at {fire}");
    }
}

#[test]
fn the_journal_keeps_notices_speech_and_rooms_and_drops_the_rest() {
    let mut run = to_the_fight(base());
    run.journal.clear();
    let now = run.now + 5;
    // An echo of a line the lane typed, a read reply, an ack, OUTSIDE_COCKPIT.
    let fed = run.lane.feed(
        now,
        "STARTUP\r\nNo contacts.\r\nSteering to 90°.\r\nYou are not at the station of a deployed unit.\r\n",
    );
    assert_eq!(fed.journal, "");
    let fed = run.lane.feed(
        now,
        "New contact X4: bearing 180°, far.\r\nAda says, \"Good hunting.\"\r\nRed Room #57\r\n",
    );
    assert_eq!(
        fed.journal,
        "New contact X4: bearing 180°, far.\nAda says, \"Good hunting.\"\nRed Room #57"
    );
    // A contact row is never in it.
    let fed = run.lane.feed(
        now,
        "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s\r\n",
    );
    assert_eq!(fed.journal, "");
}

#[test]
fn a_lobby_transient_resends_the_same_line_after_two_hundred_ms() {
    for transient in [
        "The command could not commit; please retry in a moment.",
        "The world is still starting; please try again shortly.",
        "That cockpit will not wake for you: control state unknown",
        "The arena is busy; try that again in a moment.",
    ] {
        let mut run = Run::new(base());
        run.server.transient = Some(transient.into());
        assert!(run.until(2_000, |run| run.sent.len() >= 2));
        assert_eq!(run.sent[0].line, "look");
        assert_eq!(run.sent[1].line, "look", "{transient}");
        let gap = run.sent[1].lane_ms - run.sent[0].lane_ms;
        assert!((200..=250).contains(&gap), "{transient}: {gap} ms");
    }
}

#[test]
fn a_drop_mid_fight_boards_again() {
    let mut run = to_the_fight(base());
    // Nobody in sight for five seconds: the search orders full ahead.
    assert!(run.until(10_000, |run| run.lines().contains(&"SPEED 54")));
    run.server.outside = true;
    run.server.handing_over = 1;
    assert!(run.until(5_000, |run| run.lane.step() == Step::Reboard));
    assert!(
        run.said
            .iter()
            .any(|s| s == "Out of the tank; boarding again.")
    );
    assert!(run.until(10_000, |run| run.lane.step() == Step::Fight));
    assert!(run.said.iter().any(|s| s == "Back aboard after 4 s away."));
    let boards = run.sends_of("BOARD", 0);
    assert!(
        boards.len() >= 3,
        "the board, the handed-over refusal, the reclaim: {boards:?}"
    );
    // Flying again: the polls resume, and the speed is ordered again — the
    // machine may have stopped the tank, and the order from before the drop is
    // not the tank's now.
    let back = run.now;
    run.until(3_000, |_| false);
    assert!(!run.sends_of("STATUS", back).is_empty());
    let speed = run.sends_of("SPEED 54", back);
    assert!(
        speed.first().is_some_and(|at| *at <= back + 2_000),
        "a SPEED within two seconds of the fight: {speed:?}"
    );
    run.server.end(run.now, "blue");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let flown = &run.lane.report().matches[0];
    assert_eq!((flown.reclaims, flown.away_ms), (1, 4_000));
}

#[test]
fn two_matches_reclaim_a_second_after_the_is_over_line() {
    let settings = Settings {
        matches: 2,
        ..base()
    };
    let mut run = to_the_fight(settings);
    run.until(3_000, |_| false);
    let end = run.now;
    run.server.end(end, "blue");
    assert!(run.until(
        3_000,
        |run| run.lines().iter().filter(|l| **l == "SIM TEAM red").count() == 2
    ));
    let claims = run.sends_of("SIM TEAM red", end);
    assert_eq!(claims.len(), 1);
    assert!(
        (end + 1_000..=end + 1_100).contains(&claims[0]),
        "claimed at {} after the end at {end}",
        claims[0]
    );
    assert!(run.until(60_000, Run::fighting), "the second match flies");
    run.server.end(run.now, "red");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let report = run.lane.report();
    assert_eq!(report.matches.len(), 2);
    assert_eq!((report.wins, report.losses), (1, 1));
    let result = run.lane.result().expect("a result");
    assert_eq!(result.status, "blocked", "one match lost");
    assert_eq!(
        result.outcome,
        "Won 1 of 2 matches for red (lost 1, drew 0, timed out 0, other 0)."
    );
    assert!(
        !run.lines().contains(&"SIM LEAVE"),
        "no slot stood at the end"
    );
}

#[test]
fn stop_in_selection_types_exactly_one_sim_leave() {
    let mut run = Run::new(Settings {
        lead_start_grace_ms: 60_000,
        ..base()
    });
    assert!(run.until(10_000, |run| run.lane.lobby().slot().is_some()));
    assert_eq!(run.lane.lobby().centre(), Centre::Selection);
    run.lane.stop();
    run.lane.stop();
    run.until(5_000, |_| false);
    let leaves = run.sends_of("SIM LEAVE", 0);
    assert_eq!(leaves.len(), 1);
    assert_eq!(run.lines().last(), Some(&"SIM LEAVE"), "nothing after it");
    assert!(run.said.iter().any(|s| s == "Gave up red slot 1."));
    assert!(run.lane.result().is_none(), "a stop is not a result");
    assert!(!run.lines().contains(&"SIM START"));
}

fn outcome_of(word: &str) -> (String, String) {
    let mut run = to_the_fight(base());
    sight_a_hostile(&mut run);
    run.until(8_000, |_| false);
    run.server.end(run.now, word);
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let result = run.lane.result().expect("a result").clone();
    (result.status, result.outcome)
}

#[test]
fn the_report_says_won_lost_draw_timeout_and_blocked() {
    let (status, outcome) = outcome_of("red");
    assert_eq!(status, "done");
    assert!(
        outcome.starts_with("Won match 1 for red as RA: ") && outcome.ends_with("; took 0 hit(s)."),
        "{outcome}"
    );
    for word in ["blue", "draw", "timeout"] {
        let (status, outcome) = outcome_of(word);
        assert_eq!(status, "blocked", "{word}");
        assert!(
            outcome.starts_with(&format!(
                "Match 1 ended ({word}); the bot flew RA for red: "
            )),
            "{outcome}"
        );
    }
    // A blocker before any match: its own sentence.
    let mut run = Run::new(base());
    run.server.no_centre = true;
    assert!(run.until(10_000, |run| run.lane.result().is_some()));
    let result = run.lane.result().expect("a result");
    assert_eq!(result.status, "blocked");
    assert_eq!(result.outcome, "This server has no simulator centre.");
    assert!(run.lane.report().matches.is_empty());
}

// -- the commander (cut AJ.3) ----------------------------------------------------

/// A doctrine answer.
fn answer(stance: &str) -> CommanderReply {
    CommanderReply::Answer(
        json!({"target":"BA","stance":stance,"band_m":450,"sense":"auto","note":"test"})
            .to_string(),
    )
}

impl Run {
    /// Step the clock to `at`.
    fn advance_to(&mut self, at: u64) {
        while self.now < at {
            self.step();
        }
    }

    /// Step until the commander has an ask due, at most `limit_ms`; its view.
    fn ask(&mut self, limit_ms: u64) -> Option<Value> {
        let end = self.now + limit_ms;
        loop {
            if let Some(view) = self.lane.commander_due(self.now) {
                return Some(view);
            }
            if self.now >= end {
                return None;
            }
            self.step();
        }
    }

    /// Hand the lane a reply now, and collect what it records.
    fn reply(&mut self, reply: CommanderReply) {
        self.lane.commander_reply(self.now, reply);
        self.step();
    }

    fn records_of(&self, kind: &str) -> Vec<&Value> {
        self.records
            .iter()
            .filter(|(k, _)| k == kind)
            .map(|(_, data)| data)
            .collect()
    }
}

/// The hull's internals at half (cut AK.2: the only hull a commander's
/// `withdraw` is kept on): the pilot's own fall back is seen, and its
/// withdrawal let run out. The scripted range never changes, so it ends on
/// time.
fn fall_back_and_let_it_run_out(run: &mut Run) {
    run.server.hull_internals = (30, 60);
    assert!(
        run.until(5_000, |run| run.lane.pilot().fallen_back),
        "{:?}",
        run.said
    );
    assert!(hull_failing(run.lane.picture()));
    let fell = run.now;
    run.advance_to(fell + WITHDRAW_MS);
    assert_eq!(
        run.lane.pilot().withdraw_until_ms,
        None,
        "its withdrawal is over"
    );
}

/// The commander asks every ten seconds, so an ask inside that is an event's.
fn slow() -> Settings {
    Settings {
        commander_ms: 10_000,
        ..base()
    }
}

#[test]
fn the_commander_asks_at_power_online_one_call_at_a_time_on_its_cadence() {
    let mut run = to_the_fight(base());
    let view = run.ask(200).expect("the first ask at power online");
    assert!(serde_json::to_vec(&view).expect("json").len() <= 2_048);
    assert_eq!(view["own"]["power"], "online");
    assert!(run.lane.commander_in_flight());
    // One call in flight: nothing more is due until it answers.
    run.until(3_000, |_| false);
    assert!(run.lane.commander_due(run.now).is_none());
    let answered = run.now;
    run.reply(answer("close"));
    assert!(!run.lane.commander_in_flight());
    // Searching when it asked: 3 s is inside twice the 5 s contact period.
    assert_eq!(run.lane.doctrine().stance, Stance::Close);
    assert_eq!(run.records_of("arena-doctrine").len(), 1);
    assert_eq!(run.records_of("arena-doctrine")[0]["stance"], "close");
    // The next ask is commander_ms after the answer.
    run.advance_to(answered + 1_490);
    assert!(run.lane.commander_due(run.now).is_none());
    run.advance_to(answered + 1_500);
    assert!(run.lane.commander_due(run.now).is_some());
}

#[test]
fn an_identified_hostile_and_a_lost_section_ask_at_once() {
    let mut run = to_the_fight(slow());
    run.ask(200).expect("first");
    run.reply(answer("orbit"));
    // The hostile is told apart: an ask at once, not in ten seconds.
    sight_a_hostile(&mut run);
    run.step();
    let identified = run.now;
    assert!(
        run.lane.commander_due(run.now).is_some(),
        "asked on the identification"
    );
    // A section lost while that call is in flight asks as soon as it returns.
    run.server
        .say(0, "The left_track is destroyed: the tank is slowed.");
    run.step();
    assert!(run.lane.commander_due(run.now).is_none(), "one in flight");
    run.reply(answer("orbit"));
    assert!(
        run.lane.commander_due(run.now).is_some(),
        "the lost section's ask, straight after the reply"
    );
    assert!(run.now < identified + 1_000);
}

#[test]
fn a_lock_broken_and_not_held_again_for_three_seconds_asks_at_once() {
    let mut run = to_the_fight(slow());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    run.ask(200).expect("an ask");
    run.reply(answer("orbit"));
    run.server.contacts.clear();
    run.server.locked = None;
    run.server.now = run.now;
    run.server
        .say(0, "Lock on BA broken: contact gone.\nContact BA lost.");
    assert!(run.until(500, |run| run.lane.picture().lock.is_none()));
    let broken = run.now;
    let ask = run.ask(5_000).expect("an ask after the lock broke");
    assert!(
        run.now >= broken + 2_950,
        "not before three seconds: {}",
        run.now
    );
    assert!(run.now <= broken + 3_100, "at three seconds: {}", run.now);
    assert_eq!(ask["lock"], Value::Null);
}

/// The heading last ordered at or before `at`, from any time (the pilot
/// re-orders only a change).
fn last_heading(run: &Run, at: u64) -> Option<u32> {
    heading_sends(run, 0)
        .into_iter()
        .rev()
        .find(|(sent, _)| *sent <= at)
        .map(|(_, deg)| deg)
}

/// Every `SPEED` typed in `from..=to`: when, and which.
fn speed_sends(run: &Run, from: u64, to: u64) -> Vec<(u64, i32)> {
    run.sent
        .iter()
        .filter(|s| s.lane_ms >= from && s.lane_ms <= to)
        .filter_map(|s| Some((s.lane_ms, s.line.strip_prefix("SPEED ")?.parse().ok()?)))
        .collect()
}

#[test]
fn a_withdraw_doctrine_backs_away_bow_on() {
    let mut run = to_the_fight(slow());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    // A failing hull: a withdraw on a sound one is flown as a hunt or an orbit
    // (cut AK.2; that has its own test). So the pilot falls back on its own
    // first, backing away; its withdrawal is let run out, so a reverse after
    // it is the doctrine's alone.
    fall_back_and_let_it_run_out(&mut run);
    // Holding BA at its band, bow on (cut AL.2: it used to circle it).
    run.until(2_000, |_| false);
    let heading = |run: &Run, from: u64| -> Vec<u32> {
        run.sent
            .iter()
            .filter(|s| s.lane_ms >= from)
            .filter_map(|s| s.line.strip_prefix("HEADING "))
            .filter_map(|deg| deg.parse().ok())
            .collect()
    };
    let held = heading(&run, 0);
    assert!(
        held.last()
            .is_some_and(|deg| angle_between(*deg, 0) <= HOLD_BOW_DEG),
        "holding BA (due north) bow on, not heading away: {held:?}"
    );
    run.ask(12_000).expect("an ask");
    let applied = run.now;
    run.reply(answer("withdraw"));
    assert_eq!(run.lane.doctrine().stance, Stance::Withdraw);
    run.until(2_000, |_| false);
    // Backing away from BA, due north of the tank, with the bow on it (cut
    // AL.2: it used to head away, south): the heading last ordered through
    // the withdraw (the hold already faced BA, so it may not go again) within
    // `HOLD_BOW_DEG` of 000, and a reverse within two seconds.
    let bow = last_heading(&run, run.now).expect("a heading ordered");
    assert!(angle_between(bow, 0) <= HOLD_BOW_DEG, "{bow}");
    let reverse = speed_sends(&run, applied, applied + 2_000);
    assert!(reverse.iter().any(|(_, kph)| *kph < 0), "{reverse:?}");
}

#[test]
fn an_invalid_answer_keeps_the_doctrine() {
    let mut run = to_the_fight(base());
    run.ask(200).expect("an ask");
    let before = run.lane.doctrine().clone();
    run.reply(CommanderReply::Answer("Orbit BA, please.".into()));
    assert_eq!(run.lane.doctrine(), &before);
    assert_eq!(run.records_of("commander-invalid").len(), 1);
    assert!(run.records_of("commander-invalid")[0]["error"].is_string());
}

#[test]
fn late_answers_are_dropped_at_twice_the_contact_period_and_five_stop_the_commander() {
    // Searching (no contact held): the bound is 10 000 ms.
    let mut run = to_the_fight(base());
    run.ask(200).expect("an ask");
    assert!(run.lane.picture().contacts.is_empty());
    run.until(9_000, |_| false);
    run.reply(answer("close"));
    assert_eq!(run.lane.doctrine().stance, Stance::Close, "9 s is timely");
    // Holding a contact: the bound is 2 000 ms, and five late in a row stop it.
    // Each ask is given up at its bound, before its answer comes (cut AK.2),
    // and the answer that comes after it applies nothing.
    sight_a_hostile(&mut run);
    for _ in 0..5 {
        run.ask(12_000).expect("an ask");
        run.until(2_100, |_| false);
        assert!(!run.lane.commander_in_flight(), "given up at the bound");
        run.reply(answer("orbit"));
    }
    assert_eq!(run.lane.doctrine().stance, Stance::Close, "none applied");
    let stale = run.records_of("commander-stale");
    assert_eq!(stale.len(), 5);
    assert!(stale.iter().all(|r| r["age_ms"].as_u64() > Some(2_000)));
    assert!(
        stale.iter().all(|r| r["abandoned"] == json!(true)),
        "{stale:?}"
    );
    assert_eq!(
        run.records_of("arena-doctrine").len(),
        1,
        "only the timely one"
    );
    assert_eq!(
        run.records_of("commander-off"),
        vec![&json!({"reason": OFF_STALE, "match": 1})]
    );
    assert!(run.ask(20_000).is_none(), "off for the match");
    run.server.end(run.now, "red");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let flown = &run.lane.report().matches[0];
    assert_eq!(
        (
            flown.commander_calls,
            flown.commander_applied,
            flown.commander_stale
        ),
        (6, 1, 5)
    );
    assert_eq!(flown.commander_off.as_deref(), Some(OFF_STALE));
}

#[test]
fn the_tank_destroyed_voids_the_call_and_stops_the_commander() {
    let mut run = to_the_fight(base());
    run.ask(200).expect("an ask");
    run.server.say(
        0,
        "Your tank is destroyed. The match runs on without you: stay to watch, or EXIT.",
    );
    run.step();
    assert!(!run.lane.commander_in_flight(), "the call is voided");
    assert_eq!(
        run.records_of("commander-off"),
        vec![&json!({"reason": OFF_DESTROYED, "match": 1})]
    );
    run.reply(answer("close"));
    assert_ne!(run.lane.doctrine().stance, Stance::Close, "a voided answer");
    run.server.end(run.now, "blue");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let flown = &run.lane.report().matches[0];
    assert_eq!(flown.commander_off.as_deref(), Some(OFF_DESTROYED));
    assert_eq!(run.records_of("commander-off").len(), 1, "once");
}

#[test]
fn the_match_end_stops_the_commander_and_the_next_match_starts_it_again() {
    let settings = Settings {
        matches: 2,
        ..base()
    };
    let mut run = to_the_fight(settings);
    let first = run.ask(200).expect("an ask");
    let first_power = run.lane.picture().power_ms.expect("power online read");
    assert_eq!(first["t_s"], 0, "asked at power online");
    // Match 1 answers a withdraw on BA, which the pilot's rule flies as a
    // hunt: the hull is sound and nothing is held (cut AK.2)...
    run.reply(answer("withdraw"));
    assert_eq!(run.lane.doctrine().stance, Stance::Hunt);
    assert_eq!(run.lane.doctrine().target.as_deref(), Some("BA"));
    assert_eq!(
        run.records_of("commander-overruled"),
        vec![&json!({"from": "withdraw", "to": "hunt", "why": "the hull is sound"})]
    );
    run.until(3_000, |_| false);
    assert!(run.ask(2_000).is_some(), "asked again on its cadence");
    run.server.end(run.now, "red");
    run.step();
    assert!(!run.lane.commander_in_flight());
    assert_eq!(
        run.records_of("commander-off"),
        vec![&json!({"reason": OFF_MATCH_OVER, "match": 1})]
    );
    assert!(run.until(60_000, Run::fighting), "the second match flies");
    // ...and match 2 starts on the pilot's own defaults: no target carried
    // over, and `t_s` counting from this match's power online, not match 1's.
    let view = run.ask(200).expect("asked again at its power online");
    // The pilot's default is the hold (cut AL.2; it was the orbit).
    assert_eq!(view["doctrine"]["stance"], "hold");
    assert_eq!(view["doctrine"]["target"], Value::Null);
    assert_eq!(
        run.lane.doctrine(),
        &Doctrine::pilot_default(&Settings {
            matches: 2,
            ..base()
        })
    );
    assert_eq!(view["t_s"], 0, "{view}");
    assert!(run.lane.picture().power_ms.expect("power online read") > first_power);
    // A model's `stand_off` still reads, as the hold (cut AL.2).
    run.reply(answer("stand_off"));
    assert_eq!(run.lane.doctrine().stance, Stance::Hold);
    run.server.end(run.now, "red");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let report = run.lane.report();
    assert_eq!(report.matches[0].commander_calls, 2);
    assert_eq!(report.matches[0].commander_applied, 1);
    assert_eq!(report.matches[0].commander_off, None, "it ran to the end");
    assert_eq!(
        (
            report.matches[1].commander_calls,
            report.matches[1].commander_applied
        ),
        (1, 1)
    );
}

#[test]
fn no_commander_means_no_ask() {
    let mut run = to_the_fight(Settings {
        commander: false,
        ..base()
    });
    assert!(run.ask(5_000).is_none());
    run.server.end(run.now, "red");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    assert_eq!(run.lane.report().matches[0].commander_calls, 0);
}

#[test]
fn the_budget_stop_holds_for_the_rest_of_the_lane() {
    let settings = Settings {
        matches: 2,
        ..base()
    };
    let mut run = to_the_fight(settings);
    // The runner's order: an ask would be due, the budget refuses it, and the
    // ask is never taken, so it is never counted as a call.
    let end = run.now + 200;
    while !run.lane.commander_ready(run.now) {
        assert!(run.now < end, "an ask is due at power online");
        run.step();
    }
    assert!(!run.lane.commander_in_flight(), "ready marks nothing");
    run.lane.commander_off(OFF_BUDGET);
    run.step();
    assert!(!run.lane.commander_in_flight());
    assert!(!run.lane.commander_ready(run.now));
    run.server.end(run.now, "red");
    assert!(run.until(60_000, Run::fighting), "the second match flies");
    assert!(run.ask(5_000).is_none(), "still off");
    run.server.end(run.now, "red");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let report = run.lane.report();
    assert_eq!(report.matches.len(), 2);
    for flown in &report.matches {
        assert_eq!(
            flown.commander_off.as_deref(),
            Some("the session budget is spent")
        );
        assert_eq!(flown.commander_calls, 0, "no request reached the ledger");
    }
    assert_eq!(run.records_of("commander-off").len(), 1, "once");
}

#[test]
fn a_lane_started_after_the_sessions_budget_block_never_asks() {
    // The runner switches a later lane's commander off at its start: the block
    // holds for the session, not only for the lane that met it.
    let mut run = Run::new(base());
    run.lane.commander_off(OFF_BUDGET);
    for event in run.lane.events() {
        if let LaneEvent::Record { kind, data } = event {
            run.records.push((kind, data));
        }
    }
    assert!(
        run.until(60_000, Run::fighting),
        "the pilot flies on its defaults"
    );
    assert!(run.ask(5_000).is_none(), "no ask in the match");
    run.server.end(run.now, "red");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let flown = &run.lane.report().matches[0];
    assert_eq!(flown.commander_calls, 0);
    assert_eq!(flown.commander_off.as_deref(), Some(OFF_BUDGET));
    assert_eq!(
        run.records_of("commander-off"),
        vec![&json!({"reason": OFF_BUDGET, "match": null})],
        "once, before any match"
    );
}

// -- cut AK.1: the gun's own clock, the held tank, the hunt -----------------------

/// How late the lane may read a gun's readiness: one poll of its picture.
const POLL_MS: u64 = 500;

/// A whole heading from degrees.
fn whole(deg: f64) -> u32 {
    (deg.rem_euclid(360.0).round() as u32) % 360
}

/// Every `HEADING` typed from `from`: when, and which.
fn heading_sends(run: &Run, from: u64) -> Vec<(u64, u32)> {
    run.sent
        .iter()
        .filter(|s| s.lane_ms >= from)
        .filter_map(|s| {
            let deg = s.line.strip_prefix("HEADING ")?.parse().ok()?;
            Some((s.lane_ms, deg))
        })
        .collect()
}

/// **A refused `FIRE` is the exception** (cut AK.1; cut AL.1 per gun): against
/// an arena that fires on its 500 ms pulse and a magazine that counts down,
/// locked for two minutes, at most one `FIRE` line in ten of each gun, plus
/// the slack's climb from its start to its ceiling, is refused as still
/// cycling, and each gun fires at least once a cycle, a pulse, the slack's
/// ceiling and a poll.
#[test]
fn a_refused_fire_is_the_exception() {
    const PULSE_MS: u64 = 500;
    let mut run = to_the_fight_with(base(), |server| {
        server.pulse_ms = Some(PULSE_MS);
        server.magazine = Some(200);
    });
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    let lines = run.server.fire_lines_by.clone();
    let refusals = run.server.cycling_refusals_by.clone();
    let shots = run.server.shots.clone();
    run.until(120_000, |_| false);
    assert_eq!(
        run.lane.picture().lock.as_deref(),
        Some("BA"),
        "locked throughout"
    );
    let per_gun: Vec<(&str, u32, u32)> = ["cannon", "machine_gun"]
        .into_iter()
        .map(|weapon| {
            let lines = count(&run.server.fire_lines_by, weapon) - count(&lines, weapon);
            let refusals =
                count(&run.server.cycling_refusals_by, weapon) - count(&refusals, weapon);
            println!("pulse {PULSE_MS}: {weapon}: {refusals} of {lines} FIRE lines refused");
            (weapon, refusals, lines)
        })
        .collect();
    // A loosening beyond Q-AL2's one in ten, on the 500 ms pulse rig only, by
    // the programme lead's ruling of 2026-09-30. An allowance, not a rate the
    // loop is proved to keep: a FIRE on a pulse boundary needs a slack within
    // 60 ms of the one-pulse ceiling, where a refusal lifts the slack only to
    // the ceiling (`MAX - need`, not UP), so at that worst phase the picture's
    // own model (`FIRE_SLACK_DOWN_MS`) refuses DOWN / (MAX - need + DOWN) of
    // the lines, one in seven at a need of 440 ms; the guns' acks fall on
    // every phase of the pulse, and this rig measured 3 of 21 cannon lines and
    // 8 of 86 machine-gun lines at the built constants. So the bound is one
    // line in ten, plus the climb from the starting slack to the ceiling, one
    // refusal per rise, which a tuning round that lowers START or UP lengthens.
    // The no-pulse rig and the 250 ms lattice rig (the live shape) keep one in
    // ten, strict (`full_rate_on_each_guns_own_cycle`,
    // `full_rate_on_the_arenas_lattice`).
    let climb = (FIRE_SLACK_MAX_MS - FIRE_SLACK_START_MS).div_ceil(FIRE_SLACK_UP_MS);
    for (weapon, refusals, lines) in per_gun {
        let most = u64::from(lines) / 10 + climb;
        assert!(
            u64::from(refusals) <= most,
            "{weapon}: {refusals} of {lines} FIRE lines refused as still cycling, at most {most}"
        );
    }
    for weapon in ["cannon", "machine_gun"] {
        let fired = run.server.shots.get(weapon).copied().unwrap_or(0)
            - shots.get(weapon).copied().unwrap_or(0);
        let floor = 120_000 / (Server::cycle_ms(weapon) + PULSE_MS + FIRE_SLACK_MAX_MS + POLL_MS);
        assert!(
            u64::from(fired) >= floor,
            "{weapon}: {fired} shots in two minutes, at least {floor}"
        );
    }
    let mg = run
        .lane
        .picture()
        .weapons
        .iter()
        .find(|w| w.id == "machine_gun")
        .expect("the machine gun");
    assert!(mg.rounds < 200, "the magazine counted down");
}

/// **A refused `FIRE` is tried again inside the second it was refused for**
/// (cut AL.1): on the 500 ms pulse rig and on the 250 ms lattice, every
/// `Still cycling: N s to go.` answering the lane's own `FIRE` is followed by
/// that gun's next `FIRE` within a tick of `N − 1` whole seconds and the
/// retry step ([`FIRE_RETRY_MS`]) after the step the refusal was read at — or
/// of the lane's own [`FIRE_GAP_MS`] after the refused line, where that is
/// later — unless no tick in between had a slot free for it (the limiter's
/// window full, less the other gun's `FIRE` typed ahead of it in that tick).
#[test]
fn a_refused_fire_is_tried_again_inside_its_second() {
    let max = usize::try_from(base().max_lines_per_s).unwrap_or(usize::MAX);
    for pulse_ms in [500, LATTICE_MS] {
        let mut run = to_the_fight_with(base(), |server| {
            server.pulse_ms = Some(pulse_ms);
            server.magazine = Some(200);
        });
        sight_a_hostile(&mut run);
        assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
        let from = run.server.cycling_refusals_at.len();
        run.until(120_000, |_| false);
        let refusals = run.server.cycling_refusals_at[from..].to_vec();
        assert!(
            !refusals.is_empty(),
            "pulse {pulse_ms}: no refusal to retry"
        );
        let (mut on_time, mut held) = (0, 0);
        for (at, weapon, secs) in &refusals {
            let line = format!("FIRE {weapon}");
            // Read at the first step at or after it is said.
            let read = at.div_ceil(STEP_MS) * STEP_MS;
            let refused_line = run
                .sends_of(&line, 0)
                .into_iter()
                .rfind(|sent| sent <= at)
                .unwrap_or(0);
            let due = (read + (secs - 1) * 1_000 + FIRE_RETRY_MS).max(refused_line + FIRE_GAP_MS);
            if due + TICK_MS > run.now {
                continue;
            }
            let next = run.sends_of(&line, read).first().copied();
            println!(
                "pulse {pulse_ms}: {weapon} refused ({secs} s) at {at}, due again {due}, typed \
                 again {next:?}"
            );
            if next.is_some_and(|next| next <= due + TICK_MS) {
                on_time += 1;
                continue;
            }
            // Late (or not again before the run ended): every tick from the
            // one it was due at had no slot for it.
            let until = next.unwrap_or(run.now + STEP_MS);
            let first_tick = due.div_ceil(TICK_MS) * TICK_MS;
            for tick in (first_tick..until).step_by(TICK_MS as usize) {
                let taken = run
                    .sent
                    .iter()
                    .filter(|s| {
                        (s.lane_ms < tick && s.lane_ms + LIMIT_WINDOW_MS > tick)
                            || (s.lane_ms == tick && s.line.starts_with("FIRE ") && s.line != line)
                    })
                    .count();
                assert!(
                    taken >= max,
                    "pulse {pulse_ms}: {weapon} refused at {at}, due again {due}, passed over \
                     at {tick} with {taken} of {max} slots taken; typed again at {next:?}"
                );
            }
            held += 1;
        }
        println!(
            "pulse {pulse_ms}: {on_time} refused FIRE(s) tried again on time, {held} held by the \
             limiter"
        );
        assert!(on_time > 0, "pulse {pulse_ms}: none checked on its clock");
    }
}

/// Searching on 090 at full speed, then held by the ground: the run, when the
/// notice was said, and the heading it stopped on.
fn held_in_the_search(free_off_deg: u32) -> (Run, u64, u32) {
    let mut run = to_the_fight_with(base(), |server| {
        server.held = Some(HeldKnob { free_off_deg });
    });
    assert!(run.until(10_000, |run| run.lines().contains(&"SPEED 54")));
    run.until(2_500, |_| false);
    let from = run.server.heading;
    assert_eq!(run.lane.picture().own.heading_deg, from);
    run.server.now = run.now;
    run.server.hold_tank();
    let held_at = run.now;
    (run, held_at, from)
}

/// The held turn `k` quarter-turns of `HELD_TURN_DEG` from `from`, by `sense`.
fn turned(from: u32, k: u32, sense: i8) -> u32 {
    whole(f64::from(from) + HELD_TURN_DEG * f64::from(k) * f64::from(sense))
}

#[test]
fn a_held_tank_makes_way_again() {
    let (mut run, held_at, from) = held_in_the_search(30);
    assert!(run.until(1_000, |run| run.lane.picture().own.held.is_some()));
    let read = run.now;
    run.until(TICK_MS, |_| false);
    let sense = run.lane.pilot().sense;
    // Within one tick of the notice: the turn and the speed again.
    let tick: Vec<&Send> = run
        .sent
        .iter()
        .filter(|s| s.lane_ms > held_at && s.lane_ms <= read + TICK_MS)
        .collect();
    let lines: Vec<&str> = tick.iter().map(|s| s.line.as_str()).collect();
    assert!(
        lines.contains(&format!("HEADING {}", turned(from, 1, sense)).as_str()),
        "{lines:?}"
    );
    assert!(lines.contains(&"SPEED 54"), "{lines:?}");
    // The next `STATUS` reads a speed.
    let turn_at = heading_sends(&run, held_at + 1)
        .first()
        .map(|(at, _)| *at)
        .unwrap_or(read);
    assert!(run.until(3_000, |run| {
        run.sends_of("STATUS", turn_at + 1)
            .first()
            .is_some_and(|at| run.now > at + 20)
    }));
    let own = &run.lane.picture().own;
    assert!(own.held.is_none() && own.speed_kph == 54, "{own:?}");
}

#[test]
fn a_tank_held_again_escalates_then_backs_off() {
    // A turn 135° from where it stopped frees it; 45° and 90° do not.
    let (mut run, held_at, from) = held_in_the_search(100);
    // The transcript's picture record shows the stall rising while it is held.
    assert!(
        run.until(60_000, |run| {
            run.lane.picture_json()["pilot"]["stall"]
                .as_u64()
                .is_some_and(|level| level >= 1)
        }),
        "a stall of level 1 or more in the picture record"
    );
    assert!(run.server.held_from.is_some(), "still held at level 1");
    assert!(
        run.until(60_000, |run| run.server.held_from.is_none()),
        "freed"
    );
    let sense = run.lane.pilot().sense;
    let sends = heading_sends(&run, held_at);
    let first = |deg: u32| {
        sends
            .iter()
            .find(|(_, d)| *d == deg)
            .map(|(at, _)| *at)
            .unwrap_or_else(|| panic!("no HEADING {deg}: {sends:?}"))
    };
    let (one, two, three) = (
        first(turned(from, 1, sense)),
        first(turned(from, 2, sense)),
        first(turned(from, 3, sense)),
    );
    assert!(one < two && two + STALL_MS <= three, "{sends:?}");
    for k in [2, 3] {
        let sentence = format!("Held at 0 kph: turning to {:03}°.", turned(from, k, sense));
        assert!(run.said.contains(&sentence), "{sentence}: {:?}", run.said);
    }
    assert!(
        run.sends_of("SPEED -27", held_at).is_empty(),
        "no back-off needed"
    );
    run.until(3_000, |_| false);
    assert!(
        run.lane.picture().own.held.is_none(),
        "the STATUS reads it free"
    );
    // Only a reverse frees it: every turn is under 181° from where it stopped.
    let (mut run, held_at, _) = held_in_the_search(181);
    assert!(
        run.until(90_000, |run| run.server.held_from.is_none()),
        "freed"
    );
    assert!(!run.sends_of("SPEED -27", held_at).is_empty());
    assert!(
        run.said.iter().any(|s| s == "Held at 0 kph: backing off."),
        "{:?}",
        run.said
    );
    run.until(3_000, |_| false);
    assert!(
        run.lane.picture().own.held.is_none(),
        "the STATUS reads it free"
    );
}

#[test]
fn a_lost_contact_is_hunted_in_the_lane() {
    let mut run = to_the_fight(base());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    run.until(2_000, |_| false);
    // Cut AL.3: BA is hurt before it goes (as every hunted machine in AK.3's
    // runs was), so it is hunted at once rather than waited for.
    run.server.now = run.now;
    run.server.say(0, "BA's turret is destroyed.");
    run.until(100, |_| false);
    assert!(run.lane.pilot().hurt.contains("BA"));
    // The scope loses BA: `CONTACTS` answers `No contacts.` from now on.
    // (The notices come after any reply already on its way, which still
    // names BA.)
    run.server.contacts.clear();
    run.server.locked = None;
    run.server.now = run.now;
    run.server
        .say(20, "Lock on BA broken: contact gone.\nContact BA lost.");
    let lost_at = run.now + 20;
    // (A reply that crossed the identification can already have hunted BA
    // for a moment; only what is said from here counts.)
    let said_from = run.said.len();
    assert!(run.until(1_000, |run| {
        run.said[said_from..]
            .iter()
            .any(|s| s.starts_with("Hunting BA: its last plot, "))
    }));
    let trail_plot = run
        .lane
        .pilot()
        .hunt
        .as_ref()
        .unwrap_or_else(|| panic!("hunting: {:?}", run.said))
        .trail
        .plot_m;
    // The plot is where the scripted scope drew BA: 450 m (the lock's
    // `RANGE` says 440) due north of the `STATUS` fix at 60 m east, 970 m
    // north — not merely where the pilot decided to drive.
    let drawn = (60.0, 970.0 + 450.0);
    let off = (trail_plot.0 - drawn.0).hypot(trail_plot.1 - drawn.1);
    assert!(off <= 60.0, "the plot {trail_plot:?} is {off:.0} m off BA");
    let plot = clamp_arena(trail_plot);
    // The transcript's picture record says so too.
    let view = run.lane.picture_json();
    assert_eq!(
        (&view["pilot"]["hunt"]["d"], &view["pilot"]["hunt"]["leg"]),
        (&json!("BA"), &json!("to_plot")),
        "{view}"
    );
    let age_s = view["pilot"]["hunt"]["age_s"]
        .as_u64()
        .expect("whole seconds");
    assert!(age_s <= 10, "the plot is {age_s} s old");
    assert_eq!(view["pilot"]["fallen_back"], json!(false));
    // Within a second, the heading toward the last plot (cut AL.2: the hold
    // already faced BA, due north, where the plot is, so the heading last
    // ordered, from any time, is it) and full ahead.
    run.advance_to(lost_at + 1_000);
    let toward = last_heading(&run, lost_at + 1_000).expect("a heading ordered");
    let own = run
        .lane
        .picture()
        .own_pos_at(lost_at + 1_000)
        .expect("a position");
    assert!(
        angle_between(toward, whole(bearing_deg(own, plot))) <= 10,
        "{toward}: {:?}",
        heading_sends(&run, lost_at)
    );
    let ahead = speed_sends(&run, lost_at, lost_at + 1_000);
    assert!(ahead.iter().any(|(_, kph)| *kph == 54), "{ahead:?}");
    // Sighted again (an unknown first, which does not end the hunt).
    sight(&mut run, "X2");
    assert!(
        run.lane.pilot().hunt.is_some(),
        "an unknown does not end it"
    );
    let said_from = run.said.len();
    assert!(run.until(2_000, |run| {
        run.said[said_from..]
            .iter()
            .any(|s| s == "The hunt for BA ends: BA is in sight.")
    }));
    assert_eq!(run.lane.picture_json()["pilot"]["hunt"], Value::Null);
    // Holding BA again (cut AL.2: it used to orbit it): within two seconds
    // the heading last ordered, from any time, is on BA (due north), and the
    // tank is stopped at the band.
    let ended = run.now;
    run.until(2_000, |_| false);
    let bow = last_heading(&run, ended + 2_000).expect("a heading ordered");
    assert!(
        angle_between(bow, 0) <= HOLD_BOW_DEG,
        "a hold on BA (due north): {bow}"
    );
    assert_eq!(
        run.lane.pilot().last_speed_order,
        Some(0),
        "stopped at the band"
    );
    // Every designator a hunt sentence names, the stream named.
    let named = ["X1", "X2", "BA"];
    for said in &run.said {
        let Some(rest) = said
            .strip_prefix("Hunting ")
            .or_else(|| said.strip_prefix("The hunt for "))
            .or_else(|| said.strip_prefix("Sweeping for "))
        else {
            continue;
        };
        // A designator: a capital letter, then capitals and digits.
        let words: Vec<&str> = rest
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| {
                w.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    && w.chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            })
            .collect();
        assert!(!words.is_empty(), "{said}");
        for word in words {
            assert!(named.contains(&word), "{said}: {word}");
        }
    }
}

// -- cut AK.2: the pilot's rule on a withdraw, the bounded hold ------------------

/// A `withdraw` answer on `BA` holding `hold_s` seconds.
fn withdraw_for(hold_s: u32) -> CommanderReply {
    CommanderReply::Answer(
        json!({"target":"BA","stance":"withdraw","band_m":450,"sense":"auto","hold_s":hold_s})
            .to_string(),
    )
}

/// Every `HEADING` typed in `from..to`: the degrees.
fn headings_between(run: &Run, from: u64, to: u64) -> Vec<u32> {
    heading_sends(run, from)
        .into_iter()
        .filter(|(at, _)| *at < to)
        .map(|(_, deg)| deg)
        .collect()
}

#[test]
fn a_withdraw_on_a_sound_hull_is_flown_as_a_hunt() {
    let mut run = to_the_fight(base());
    assert!(run.until(3_000, |run| {
        run.lane.picture().own.sections.contains_key("hull")
    }));
    assert!(!hull_failing(run.lane.picture()), "60 of 60");
    assert!(run.lane.picture().contacts.is_empty(), "nothing held");
    run.ask(200).expect("an ask");
    run.reply(answer("withdraw"));
    let doctrine = run.lane.doctrine().clone();
    assert_eq!(doctrine.stance, Stance::Hunt);
    assert_eq!(
        (doctrine.target.as_deref(), doctrine.band_m, doctrine.hold_s),
        (Some("BA"), 450, None)
    );
    // The answer is recorded as read, and the rule's flight of it beside it.
    assert_eq!(run.records_of("arena-doctrine")[0]["stance"], "withdraw");
    assert_eq!(
        run.records_of("commander-overruled"),
        vec![&json!({"from": "withdraw", "to": "hunt", "why": "the hull is sound"})]
    );
    run.server.end(run.now, "red");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    let flown = &run.lane.report().matches[0];
    assert_eq!((flown.commander_applied, flown.commander_overruled), (1, 1));
    assert!(
        run.records_of("doctrine-lapsed").is_empty(),
        "no withdraw stood"
    );
}

/// **A kept withdraw holds, then lapses** (cut AK.2), built so that only the
/// doctrine can reverse the tank: the pilot's own fall back has run out and it
/// holds BA bow on (cut AL.2: it used to orbit) when the commander answers
/// `withdraw` for longer than the pilot's own withdrawal. The tank backs away
/// bow on for the hold, counted from the first withdraw however often it is
/// answered again; then the pilot's default hold again; and a withdraw in the
/// rest after it is flown as the pilot's own engagement, the hold (cut AL.3:
/// it was an orbit).
#[test]
fn a_withdraw_on_a_failing_hull_holds_then_lapses() {
    const HOLD_S: u32 = 20;
    let hold_ms = u64::from(HOLD_S) * 1_000;
    assert!(
        hold_ms > WITHDRAW_MS,
        "longer than the pilot's own withdrawal"
    );
    let mut run = to_the_fight(slow());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    fall_back_and_let_it_run_out(&mut run);
    run.until(2_000, |_| false);
    // The heading last ordered (the pilot re-orders only a change).
    let held = headings_between(&run, 0, run.now + 1);
    assert!(
        held.last()
            .is_some_and(|deg| angle_between(*deg, 0) <= HOLD_BOW_DEG),
        "holding BA bow on after the pilot's own withdrawal, not heading away: {held:?}"
    );

    // The first withdraw: kept, for its twenty seconds.
    run.ask(12_000).expect("an ask");
    let first = run.now;
    run.reply(withdraw_for(HOLD_S));
    assert_eq!(run.lane.doctrine().stance, Stance::Withdraw);
    assert_eq!(run.lane.doctrine().hold_s, Some(HOLD_S));
    assert!(run.records_of("commander-overruled").is_empty());
    // A second about ten seconds in (the commander's cadence): applied, but
    // the lapse does not move.
    run.ask(12_000).expect("the next ask on its cadence");
    assert!(
        run.now < first + hold_ms,
        "inside the hold: {}",
        run.now - first
    );
    run.reply(withdraw_for(HOLD_MAX_S));
    assert_eq!(run.lane.doctrine().stance, Stance::Withdraw);
    assert_eq!(run.lane.doctrine().hold_s, Some(HOLD_MAX_S));
    assert_eq!(run.records_of("arena-doctrine").len(), 2);
    // Backing away from BA (due north) with the bow on it through the hold
    // (cut AL.2: it used to head away, south): every heading sent through it
    // (there may be none: the hold already faced BA) and the heading last
    // ordered within `HOLD_BOW_DEG` of 000, and a reverse sent in it.
    run.advance_to(first + hold_ms - TICK_MS);
    assert_eq!(run.lane.doctrine().stance, Stance::Withdraw, "still held");
    let away = headings_between(&run, first, run.now + 1);
    assert!(
        away.iter()
            .all(|deg| angle_between(*deg, 0) <= HOLD_BOW_DEG),
        "{away:?}"
    );
    let bow = last_heading(&run, run.now).expect("a heading ordered");
    assert!(angle_between(bow, 0) <= HOLD_BOW_DEG, "{bow}");
    let reverse = speed_sends(&run, first, run.now);
    assert!(reverse.iter().any(|(_, kph)| *kph < 0), "{reverse:?}");
    assert!(run.records_of("doctrine-lapsed").is_empty());
    // Twenty seconds after the first, not the second: the pilot's own again.
    run.advance_to(first + hold_ms + TICK_MS);
    assert_eq!(
        run.lane.doctrine(),
        &Doctrine::pilot_default(&slow()),
        "the hold ran out"
    );
    let lapsed = run.records_of("doctrine-lapsed");
    assert_eq!(lapsed.len(), 1, "{lapsed:?}");
    assert_eq!(lapsed[0]["why"], "the hold ran out");
    assert_eq!(lapsed[0]["was"]["stance"], "withdraw");
    assert_eq!(lapsed[0]["was"]["hold_s"], HOLD_MAX_S);
    // Holding BA again (cut AL.2: the pilot's default is the hold, not the
    // orbit, and the bow never left BA, so no new heading need go): the
    // heading last ordered, from any time, on BA, and the tank stopped at
    // the band.
    let lapse = run.now;
    run.until(3_000, |_| false);
    let again = last_heading(&run, run.now).expect("a heading ordered");
    assert!(
        angle_between(again, 0) <= HOLD_BOW_DEG,
        "holding again after the lapse: {again}"
    );
    assert_eq!(
        speed_sends(&run, lapse, run.now)
            .last()
            .map(|(_, kph)| *kph),
        Some(0),
        "stopped at the band after the lapse"
    );
    // A third withdraw inside the rest: the hold is spent, and BA is held, so
    // it is flown as the pilot's own engagement, the hold (cut AL.3: it was
    // an orbit).
    run.ask(12_000).expect("an ask");
    assert!(run.now < lapse + u64::from(HOLD_MAX_S) * 1_000);
    run.reply(withdraw_for(HOLD_S));
    assert_eq!(run.lane.doctrine().stance, Stance::Hold);
    assert_eq!(
        run.records_of("commander-overruled"),
        vec![&json!({"from": "withdraw", "to": "hold", "why": "the hold is spent"})]
    );
    // The rest over, a withdraw on the failing hull is kept again.
    run.advance_to(lapse + u64::from(HOLD_MAX_S) * 1_000);
    run.ask(12_000).expect("an ask");
    run.reply(withdraw_for(HOLD_S));
    assert_eq!(
        run.lane.doctrine().stance,
        Stance::Withdraw,
        "the rest ended"
    );
    assert_eq!(run.records_of("commander-overruled").len(), 1, "no new one");
    run.server.end(run.now, "red");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    assert_eq!(run.lane.report().matches[0].commander_overruled, 1);
}

/// **An episode ends at its time whatever stands then** (cut AK.2): a
/// commander that answers `orbit` inside a hold ends nothing early, so
/// alternating `withdraw` and `orbit` still keeps the tank away at most half
/// of any minute. Nothing lapses at the hold's end (no withdraw stands), but
/// the rest begins; after the rest a new episode holds its own full length.
#[test]
fn a_withdraw_episode_answered_away_early_still_ends_on_time_and_rests() {
    const HOLD_S: u32 = 20;
    let hold_ms = u64::from(HOLD_S) * 1_000;
    let rest_ms = u64::from(HOLD_MAX_S) * 1_000;
    let mut run = to_the_fight(slow());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    fall_back_and_let_it_run_out(&mut run);
    // A kept withdraw, then an orbit about ten seconds into its hold.
    run.ask(12_000).expect("an ask");
    let first = run.now;
    run.reply(withdraw_for(HOLD_S));
    assert_eq!(run.lane.doctrine().stance, Stance::Withdraw);
    run.ask(12_000).expect("the next ask on its cadence");
    assert!(run.now < first + hold_ms, "inside the hold");
    run.reply(answer("orbit"));
    assert_eq!(run.lane.doctrine().stance, Stance::Orbit);
    // The hold's time: the orbit stands, nothing lapses, the rest begins.
    run.advance_to(first + hold_ms + TICK_MS);
    let end = run.now;
    assert_eq!(run.lane.doctrine().stance, Stance::Orbit);
    assert!(run.records_of("doctrine-lapsed").is_empty());
    run.ask(12_000).expect("an ask");
    assert!(run.now < end + rest_ms, "inside the rest");
    run.reply(withdraw_for(HOLD_S));
    // Flown as the pilot's own engagement, the hold (cut AL.3: it was an
    // orbit).
    assert_eq!(run.lane.doctrine().stance, Stance::Hold);
    assert_eq!(
        run.records_of("commander-overruled"),
        vec![&json!({"from": "withdraw", "to": "hold", "why": "the hold is spent"})]
    );
    // After the rest, a new episode: kept, and held its own full length (the
    // old episode's end is not inherited).
    run.advance_to(end + rest_ms);
    run.ask(12_000).expect("an ask");
    let second = run.now;
    run.reply(withdraw_for(HOLD_S));
    assert_eq!(run.lane.doctrine().stance, Stance::Withdraw);
    run.advance_to(second + hold_ms - TICK_MS);
    assert_eq!(run.lane.doctrine().stance, Stance::Withdraw, "its own hold");
    assert!(run.records_of("doctrine-lapsed").is_empty());
    run.advance_to(second + hold_ms + TICK_MS);
    let lapsed = run.records_of("doctrine-lapsed");
    assert_eq!(lapsed.len(), 1, "{lapsed:?}");
    assert_eq!(lapsed[0]["why"], "the hold ran out");
    assert_eq!(run.records_of("commander-overruled").len(), 1);
}

#[test]
fn a_withdraw_is_cleared_when_the_commander_stops() {
    /// A kept withdraw on a failing hull, the run's doctrine then.
    fn withdrawn() -> Run {
        let mut run = to_the_fight(base());
        run.server.hull_internals = (30, 60);
        assert!(run.until(5_000, |run| hull_failing(run.lane.picture())));
        run.ask(200).expect("an ask");
        run.reply(withdraw_for(HOLD_MAX_S));
        assert_eq!(run.lane.doctrine().stance, Stance::Withdraw);
        run
    }
    fn cleared(run: &Run) {
        assert_eq!(run.lane.doctrine(), &Doctrine::pilot_default(&base()));
        let lapsed = run.records_of("doctrine-lapsed");
        assert_eq!(lapsed.len(), 1, "{lapsed:?}");
        assert_eq!(lapsed[0]["why"], "the commander stopped");
        assert_eq!(lapsed[0]["was"]["stance"], "withdraw");
    }
    // Five errors in a row.
    let mut run = withdrawn();
    for _ in 0..ERRORS_IN_ROW {
        assert_eq!(run.lane.doctrine().stance, Stance::Withdraw);
        run.ask(12_000).expect("an ask");
        run.reply(CommanderReply::Error("HTTP 500".into()));
    }
    assert_eq!(
        run.records_of("commander-off"),
        vec![&json!({"reason": OFF_ERRORS, "match": 1})]
    );
    cleared(&run);
    // The session budget.
    let mut run = withdrawn();
    run.lane.commander_off(OFF_BUDGET);
    run.step();
    assert_eq!(
        run.records_of("commander-off"),
        vec![&json!({"reason": OFF_BUDGET, "match": 1})]
    );
    cleared(&run);
    // Any other stance survives a stop: the pilot flies on the last doctrine.
    let mut run = to_the_fight(base());
    run.ask(200).expect("an ask");
    run.reply(answer("stand_off"));
    run.lane.commander_off(OFF_BUDGET);
    run.step();
    assert_eq!(run.lane.doctrine().stance, Stance::Hold);
    assert!(run.records_of("doctrine-lapsed").is_empty());
}

// -- cut AL.1: full cadence ---------------------------------------------------------

/// The lead's own bounds on the machine gun's and the cannon's median gap,
/// offline with no pulse (16c's AL entry: its figures, not constants of the
/// bot's).
const LEAD_MG_P50_MS: u64 = 1_200;
const LEAD_CANNON_P50_MS: u64 = 6_500;
/// The arena's own lattice (the bot's transcripts: its notices arrive on a
/// 250 ms lattice within a match), for the lattice rig.
const LATTICE_MS: u64 = 250;
const GUNS: [&str; 2] = ["cannon", "machine_gun"];

/// A per-gun count, none when the gun has none.
fn count(by: &BTreeMap<String, u32>, weapon: &str) -> u32 {
    by.get(weapon).copied().unwrap_or(0)
}

/// The picture's gun `id`.
fn gun<'a>(picture: &'a Picture, id: &str) -> Option<&'a Weapon> {
    picture.weapons.iter().find(|w| w.id == id)
}

/// Both guns' cycles are learned, each the scripted gun's own.
fn cycles_learned(run: &Run) -> bool {
    GUNS.iter()
        .all(|w| gun(run.lane.picture(), w).and_then(|g| g.cycle_ms) == Some(Server::cycle_ms(w)))
}

/// Locked on BA with both cycles learned.
fn locked_and_learned(run: &mut Run) {
    sight_a_hostile(run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    assert!(
        run.until(20_000, cycles_learned),
        "the cycles learned: {:?}",
        run.lane.picture().weapons
    );
}

/// Neither a tick nor the step after one: no reply to a line the lane typed is
/// on its way, so a test may change the server here.
fn between_ticks(run: &Run) -> bool {
    !run.now.is_multiple_of(TICK_MS) && !(run.now - STEP_MS).is_multiple_of(TICK_MS)
}

/// Between ticks, `weapon` due inside the next 300 ms and the next periodic
/// `WEAPONSTATUS` over a second off: its next line will be a `FIRE`, typed
/// before any row can tell the picture what the server has just changed.
fn about_to_fire(run: &Run, weapon: &str) -> bool {
    let last_poll = run.sends_of("WEAPONSTATUS", 0).last().copied().unwrap_or(0);
    between_ticks(run)
        && gun(run.lane.picture(), weapon)
            .and_then(|w| w.next_ready_ms)
            .is_some_and(|at| at > run.now && at <= run.now + 300)
        && run.now + LIMIT_WINDOW_MS < last_poll + WEAPONS_LEARNED_MS
}

/// The lead's count over the lane's own sends, and the same over the server's
/// acceptances, from `from`.
fn both_gaps(run: &Run, from: u64) -> (BTreeMap<String, GapStats>, BTreeMap<String, GapStats>) {
    let fire = report::fire_gaps(
        run.sent
            .iter()
            .filter(|s| s.lane_ms >= from)
            .map(|s| (s.lane_ms, s.line.as_str())),
    );
    let acks: Vec<(u64, String)> = run
        .server
        .acks
        .iter()
        .filter(|(at, _)| *at >= from)
        .map(|(at, w)| (*at, format!("Firing {w}.")))
        .collect();
    let shot = report::shot_gaps(acks.iter().map(|(at, line)| (*at, line.as_str())));
    (fire, shot)
}

/// **Full rate** on a rig whose pulse is `pulse_ms`: locked on BA for two
/// minutes, both cycles read whole; each gun's two gap counts and its
/// refusals printed. The run and when the two minutes began.
fn full_rate(pulse_ms: Option<u64>) -> (Run, u64) {
    let mut run = to_the_fight_with(base(), |server| {
        server.pulse_ms = pulse_ms;
        server.magazine = Some(200);
    });
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    let start = run.now;
    let lines = run.server.fire_lines_by.clone();
    let refusals = run.server.cycling_refusals_by.clone();
    run.until(120_000, |_| false);
    assert_eq!(
        run.lane.picture().lock.as_deref(),
        Some("BA"),
        "locked throughout"
    );
    // The test measures AL's clock: both cycles were read whole.
    assert!(cycles_learned(&run), "{:?}", run.lane.picture().weapons);
    let (fire, shot) = both_gaps(&run, start);
    for weapon in GUNS {
        let lines = count(&run.server.fire_lines_by, weapon) - count(&lines, weapon);
        let refusals = count(&run.server.cycling_refusals_by, weapon) - count(&refusals, weapon);
        println!(
            "pulse {pulse_ms:?}: {weapon}: FIRE lines {:?}; shots {:?}; {refusals} of {lines} FIRE \
             lines refused as still cycling",
            fire.get(weapon),
            shot.get(weapon),
        );
        assert!(refusals * 10 <= lines, "{weapon}: {refusals} of {lines}");
    }
    (run, start)
}

/// **The lead's assertion** (cut AL.1): with the scripted cockpit's pulse
/// unset, so the bot's own schedule is all that is measured, the machine gun's
/// median gap is at most 1 200 ms and the cannon's at most 6 500, by the
/// lead's count of the `FIRE` lines and by the shots that left; at most one
/// `FIRE` line in ten of each gun is refused as still cycling (in
/// [`full_rate`]); and each gun fires at least once a cycle, the slack's
/// ceiling and two ticks.
#[test]
fn full_rate_on_each_guns_own_cycle() {
    let (run, start) = full_rate(None);
    let (fire, shot) = both_gaps(&run, start);
    for (count_name, gaps) in [("FIRE lines", &fire), ("shots", &shot)] {
        let p50 = |w: &str| gaps.get(w).map_or(u64::MAX, |g| g.p50_ms);
        assert!(
            p50("machine_gun") <= LEAD_MG_P50_MS,
            "{count_name}: {gaps:?}"
        );
        assert!(
            p50("cannon") <= LEAD_CANNON_P50_MS,
            "{count_name}: {gaps:?}"
        );
    }
    for weapon in GUNS {
        let shots = run
            .server
            .acks
            .iter()
            .filter(|(at, w)| *at >= start && w == weapon)
            .count() as u64;
        let floor = 120_000 / (Server::cycle_ms(weapon) + FIRE_SLACK_MAX_MS + 2 * TICK_MS);
        assert!(shots >= floor, "{weapon}: {shots} shots, at least {floor}");
    }
}

/// **Full rate on the arena's lattice** (cut AL.1): the scripted arena starts
/// each shot at its next 250 ms lattice point (the bot's transcripts' notice
/// lattice), so a gun is ready one cycle after that point: both gap counts'
/// medians are at most a cycle, a lattice step and two ticks, and at most one
/// `FIRE` line in ten of each gun is refused (in [`full_rate`]).
#[test]
fn full_rate_on_the_arenas_lattice() {
    let (run, start) = full_rate(Some(LATTICE_MS));
    let (fire, shot) = both_gaps(&run, start);
    for (count_name, gaps) in [("FIRE lines", &fire), ("shots", &shot)] {
        for weapon in GUNS {
            let bound = Server::cycle_ms(weapon) + LATTICE_MS + 2 * TICK_MS;
            let p50 = gaps.get(weapon).map_or(u64::MAX, |g| g.p50_ms);
            assert!(
                p50 <= bound,
                "{count_name}: {weapon} p50 {p50} over {bound}: {gaps:?}"
            );
        }
    }
}

/// The reservation's rule, worked out from the lane's picture and its sends: a
/// poll sent at `now` after `before` (the lines typed inside the window before
/// it) leaves a slot, inside `max` lines a window, for each gun the pilot may
/// fire that is due inside the window (none while it fires nothing: no lock, a
/// friendly one, every gun in doubt; never a gun read at `0%`).
fn leaves_the_guns_a_slot(picture: &Picture, before: &[&Send], now: u64, max: usize) -> bool {
    if picture.lock.is_none() || picture.lock_friendly || picture.weapons_doubt.is_some() {
        return true;
    }
    let mut due: Vec<u64> = picture
        .weapons
        .iter()
        .filter(|w| !picture.struck.contains(&w.id) && w.rounds_left() > 0 && w.percent != Some(0))
        .filter_map(|w| w.next_ready_ms)
        .map(|at| at.max(now))
        .filter(|d| *d < now + LIMIT_WINDOW_MS)
        .collect();
    due.sort_unstable();
    due.iter().enumerate().all(|(k, d)| {
        let in_window = before
            .iter()
            .filter(|s| s.lane_ms + LIMIT_WINDOW_MS > *d)
            .count();
        // The lines already in the window, the poll, the guns before, the gun.
        let lines = in_window + 1 + k + 1;
        lines <= max
    })
}

/// **Where in each second [`a_crowded_minute`] says its contact notice**
/// (cut AN.1), in milliseconds past the second. A multiple of [`STEP_MS`], the
/// clock's step, or the notice never fires. Every value from 0 to 990 was
/// flown: 0 to 40 and 850 to 990 hold no `STATUS` back at the settings' own
/// limiter; 550 to 790 hold back 13 to 21 ticks, 600 among the most, with a
/// `STATUS` falling due while the machine gun is due inside the second and
/// the window already full.
const CROWDED_NOTICE_PHASE_MS: u64 = 600;
const _: () =
    assert!(CROWDED_NOTICE_PHASE_MS.is_multiple_of(STEP_MS) && CROWDED_NOTICE_PHASE_MS < 1_000);

/// What a crowded minute came to: whether both cycles were learned before
/// it, ticks with the ring full, ticks a due `STATUS` was held back, of those
/// the ones the reservation alone held (a slot free), polls that went
/// overdue, and when each `STATUS` went.
#[derive(Debug)]
struct Crowded {
    learned: bool,
    crowded: u32,
    held_back: u32,
    reserved: u32,
    starved: u32,
    statuses: Vec<u64>,
}

/// **A crowded minute** on a lane typing at most `max` lines a second: locked,
/// with a contact notice every second so `CONTACTS` (and the lock's `RANGE`
/// after every second reply) crowd the ring. Checked tick by tick against the
/// lane's own picture: no poll that can wait goes in a tick where it leaves no
/// slot for a gun due inside the next second, unless it was `POLL_STARVE_MS`
/// overdue; a `STATUS` held back with a slot free was held by the rule; and a
/// gun that may fire is never passed over in a tick with a slot free for it.
///
/// **Its phase** (cut AN.1): the notice is said [`CROWDED_NOTICE_PHASE_MS`]
/// into each second, which phases the `CONTACTS` it brings on and the lock's
/// `RANGE` after it so that a `STATUS` falls due while a gun is due with the
/// window full, the case the reservation exists for. The old reply order
/// phased it by a crossing hunt (a reply that crossed the identification
/// started one, whose `HEADING` and `SPEED` lines shifted `STATUS` onto a
/// gun's due time); with replies rendered at delivery no hunt starts, and the
/// notice on the second leaves `STATUS` on a cadence that never meets a due
/// gun.
fn a_crowded_minute(max: usize) -> Crowded {
    let settings = Settings {
        max_lines_per_s: u32::try_from(max).unwrap_or(u32::MAX),
        ..base()
    };
    let mut run = to_the_fight(settings);
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    // Fewer lines a second may leave the post-shot read no slot inside the
    // cannon's first second; then the minute checks `STATUS` alone (a
    // `WEAPONSTATUS` may be the post-shot read, whose cadence is not a poll's
    // that can wait).
    let learned = run.until(20_000, cycles_learned);
    run.server
        .contacts
        .push("X7  unknown  bearing 180° (rel 090°)  far  500 m  1 s".into());
    let start = run.now;
    let mut seen = Crowded {
        learned,
        crowded: 0,
        held_back: 0,
        reserved: 0,
        starved: 0,
        statuses: Vec::new(),
    };
    let mut last_poll: BTreeMap<String, u64> = BTreeMap::new();
    for send in &run.sent {
        last_poll.insert(send.line.clone(), send.lane_ms);
    }
    while run.now < start + 60_000 {
        if run.now % 1_000 == CROWDED_NOTICE_PHASE_MS {
            run.server.now = run.now;
            run.server.say(0, "New contact X7: bearing 180°, far.");
        }
        let sent_before = run.sent.len();
        run.step();
        if !run.now.is_multiple_of(TICK_MS) {
            continue;
        }
        let now = run.now;
        let picture = run.lane.picture();
        let tick: Vec<&Send> = run.sent[sent_before..].iter().collect();
        let earlier: Vec<&Send> = run.sent[..sent_before]
            .iter()
            .filter(|s| s.lane_ms + LIMIT_WINDOW_MS > now)
            .collect();
        if earlier.len() + tick.len() >= max {
            seen.crowded += 1;
        }
        // Every poll that can wait (with no gun's cycle unknown, every
        // `WEAPONSTATUS` is the periodic one).
        for (i, send) in tick.iter().enumerate() {
            let every = match send.line.as_str() {
                "STATUS" => STATUS_EVERY_MS,
                "WEAPONSTATUS" if learned => WEAPONS_LEARNED_MS,
                _ => continue,
            };
            let before: Vec<&Send> = earlier.iter().chain(tick[..i].iter()).copied().collect();
            let overdue = last_poll
                .get(&send.line)
                .is_none_or(|at| now >= at + every.max(POLL_GAP_MS) + POLL_STARVE_MS);
            if overdue {
                seen.starved += 1;
            }
            assert!(
                overdue || leaves_the_guns_a_slot(picture, &before, now, max),
                "{max} a second: {} at {now} left no slot: {:?}; sent {:?}",
                send.line,
                picture.weapons,
                before
                    .iter()
                    .map(|s| (s.lane_ms, &s.line))
                    .collect::<Vec<_>>()
            );
        }
        // A `STATUS` due and not sent: held back (for the guns, or the slots).
        if let Some(at) = last_poll.get("STATUS")
            && now >= at + STATUS_EVERY_MS
            && !tick.iter().any(|s| s.line == "STATUS")
        {
            seen.held_back += 1;
            // …and with a slot still free after this tick's lines, the lane
            // stopped only because the reservation said so: the rule, worked
            // out here, leaves no slot for a gun, and it was not overdue. A
            // reservation that refused whatever the guns' clocks said fails
            // here.
            let all: Vec<&Send> = earlier.iter().chain(tick.iter()).copied().collect();
            if all.len() < max {
                assert!(
                    !leaves_the_guns_a_slot(picture, &all, now, max)
                        && now < at + STATUS_EVERY_MS + POLL_STARVE_MS,
                    "{max} a second: STATUS held back at {now} with a slot free and the \
                     guns' slots kept: {:?}; sent {:?}",
                    picture.weapons,
                    all.iter().map(|s| (s.lane_ms, &s.line)).collect::<Vec<_>>()
                );
                seen.reserved += 1;
            }
        }
        // Every gun that may fire goes in a tick with a slot free for it: the
        // guns come first, main gun first.
        let free = max.saturating_sub(earlier.len());
        let ba_fresh = picture.contacts.get("BA").is_some_and(|c| {
            c.age_ms <= FRESH_PLOT_MS && now.saturating_sub(c.read_ms) <= FRESH_READ_MS
        });
        let mut ahead = 0;
        for weapon in GUNS {
            let line = format!("FIRE {weapon}");
            let fired = tick.iter().any(|s| s.line == line);
            let Some(w) = gun(picture, weapon) else {
                continue;
            };
            let last_fire = run.sent[..sent_before]
                .iter()
                .rev()
                .find(|s| s.line == line)
                .map(|s| s.lane_ms);
            let may = picture.lock.as_deref() == Some("BA")
                && ba_fresh
                && picture.weapons_doubt.is_none()
                && w.rounds_left() > 0
                && w.percent != Some(0)
                && w.next_ready_ms.is_some_and(|at| at <= now)
                && (fired || !picture.fire_pending(weapon))
                && last_fire.is_none_or(|at| now >= at + FIRE_GAP_MS);
            if may {
                assert!(
                    fired || ahead >= free,
                    "{max} a second: {weapon} due at {:?} passed over at {now} with {free} \
                     slot(s) free",
                    w.next_ready_ms
                );
                ahead += 1;
            }
        }
        for send in &tick {
            last_poll.insert(send.line.clone(), send.lane_ms);
        }
    }
    seen.statuses = run.sends_of("STATUS", start);
    println!("a due gun keeps its slot, {max} lines a second: {seen:?}");
    let fires = run.sends_of("FIRE machine_gun", start);
    println!(
        "the machine gun fired {} times in the crowded minute",
        fires.len()
    );
    let floor = 60_000 / (Server::cycle_ms("machine_gun") + FIRE_SLACK_MAX_MS + 2 * TICK_MS);
    assert!(
        fires.len() as u64 >= floor,
        "{max} a second: the machine gun kept firing: {} of at least {floor}",
        fires.len()
    );
    seen
}

/// **A due gun keeps its slot from a poll** (cut AL.1): [`a_crowded_minute`]
/// at the settings' own limiter, where `STATUS` is never held past its
/// overdue escape and a window; and one line a second fewer, where the ring
/// is fuller and the reservation, not the limiter, is what holds a poll back
/// in many ticks: there a `STATUS` is held with a slot free in at least one
/// tick in 250 of the minute (6 to 28 such ticks were seen across the tuning
/// bounds, where the settings' own limiter shows 0 to 5; at the minute's
/// re-phased notice, cut AN.1, 49 and 1). (At fewer lines the
/// escape lifts only the reservation: the lines ahead of `STATUS` may hold it
/// longer.)
#[test]
fn a_due_gun_keeps_its_slot_from_a_poll() {
    let max = usize::try_from(base().max_lines_per_s).unwrap_or(usize::MAX);
    let full = a_crowded_minute(max);
    assert!(full.learned, "the cycles learned first: {full:?}");
    assert!(full.crowded > 100, "the ring was crowded: {full:?}");
    assert!(
        full.held_back > 0,
        "a poll was held back for the guns: {full:?}"
    );
    for pair in full.statuses.windows(2) {
        assert!(
            pair[1] - pair[0] <= STATUS_EVERY_MS + POLL_STARVE_MS + LIMIT_WINDOW_MS,
            "STATUS starved: {full:?}"
        );
    }
    let fewer = a_crowded_minute(max - 1);
    assert!(fewer.crowded > 100, "the ring was crowded: {fewer:?}");
    let ticks = 60_000 / TICK_MS;
    assert!(
        u64::from(fewer.reserved) * 250 >= ticks,
        "the reservation itself held a STATUS with a slot free: {fewer:?}"
    );
}

/// **An identification never starts a hunt** (cut AM.3), at every offset
/// from a `CONTACTS` reply. In a fresh fight for each `d` from 0 to 20 ms:
/// the scope holds the unknown X1 and the lane has read its `New contact`
/// notice; on the step the lane types a `CONTACTS` (at `t`, its reply handed
/// out at `t + 3`), the identification is queued at `t + d` ([`Chunk::Identify`],
/// the swap and its sentence rendered when it is handed out). Offsets 0 to 2
/// put it before the reply, which then prints BA's row; offsets 3 to 20 after
/// it, the reply naming X1, and the identification closes that reply first.
/// Both are orders the game prints. After 10 s no `Hunting ` note was said
/// and BA is held. The lane is never locked on X1 here (the pilot never locks
/// an unknown, so it is asserted, not branched on); a lock carried across a
/// rename is the picture's own proof,
/// `a_rename_and_back_keep_the_fixes_and_the_lock` (cut AN.1).
#[test]
fn an_identification_never_starts_a_hunt() {
    const X1_ROW: &str = "X1  unknown  bearing 000° (rel 270°)  far  450 m  1 s";
    const BA_ROW: &str = "BA  blue  bearing 000° (rel 270°)  far  450 m  1 s";
    for d in 0..=20u64 {
        let mut run = to_the_fight(base());
        run.server.contacts = vec![X1_ROW.into()];
        run.server.now = run.now;
        run.server.say(0, "New contact X1: bearing 000°, far.");
        assert!(
            run.until(2_000, |run| run.lane.picture().contacts.contains_key("X1")),
            "{d}: the lane read X1"
        );
        let from = run.now + 1;
        assert!(
            run.until(10_000, |run| !run.sends_of("CONTACTS", from).is_empty()),
            "{d}: a CONTACTS typed"
        );
        let t = run.sends_of("CONTACTS", from)[0];
        assert_eq!(t, run.now, "{d}: on the step it was typed");
        assert_ne!(
            run.lane.picture().lock.as_deref(),
            Some("X1"),
            "{d}: an unknown is never locked"
        );
        run.server.queue.push((
            t + d,
            Chunk::Identify {
                was: "X1".into(),
                row: BA_ROW.into(),
            },
        ));
        // From the step the identification is handed out on, the lane holds
        // BA and never X1 again: no reply naming X1 is read after it.
        let (end, mut crossed) = (run.now + 10_000, Vec::new());
        while run.now < end {
            run.step();
            let handed_out = !run
                .server
                .queue
                .iter()
                .any(|(_, chunk)| matches!(chunk, Chunk::Identify { .. }));
            let held = &run.lane.picture().contacts;
            if handed_out && (held.contains_key("X1") || !held.contains_key("BA")) {
                crossed.push(run.now);
            }
        }
        assert!(
            crossed.is_empty(),
            "{d} ms: X1 held or BA not after the identification, at {crossed:?}"
        );
        let hunts: Vec<&String> = run
            .said
            .iter()
            .filter(|s| s.starts_with("Hunting "))
            .collect();
        assert!(hunts.is_empty(), "{d} ms: {hunts:?}");
        let picture = run.lane.picture();
        let held: Vec<&str> = picture.contacts.keys().map(String::as_str).collect();
        assert!(held.contains(&"BA"), "{d} ms: {held:?}");
        assert_ne!(picture.lock.as_deref(), Some("X1"), "{d} ms");
    }
}

/// Locked with both cycles learned on a rig whose pulse is `pulse_ms` (and,
/// with a pulse, a magazine that counts down, as the live rows do), a minute
/// sends at most one `WEAPONSTATUS` every `WEAPONS_LEARNED_MS` (and one for
/// the minute's edge) while the machine gun keeps firing. The run.
fn thinned(pulse_ms: Option<u64>) -> Run {
    let mut run = to_the_fight_with(base(), |server| {
        server.pulse_ms = pulse_ms;
        server.magazine = pulse_ms.map(|_| 200);
    });
    locked_and_learned(&mut run);
    let from = run.now;
    run.until(60_000, |_| false);
    let polls = run.sends_of("WEAPONSTATUS", from);
    println!("pulse {pulse_ms:?}: WEAPONSTATUS, every cycle learned: {polls:?}");
    assert!(
        polls.len() as u64 <= 60_000 / WEAPONS_LEARNED_MS + 1,
        "pulse {pulse_ms:?}: {polls:?}"
    );
    assert!(!run.sends_of("FIRE machine_gun", from).is_empty());
    run
}

/// **`WEAPONSTATUS` thins once every cycle is known** (cut AL.1): at most once
/// every `WEAPONS_LEARNED_MS` locked ([`thinned`]), and back to
/// `WEAPONS_LOCKED_MS` while a gun's row reads `0%` — on time in the middle,
/// since no slot is kept for a gun the pilot will not fire.
#[test]
fn weaponstatus_thins_once_every_cycle_is_known() {
    let mut run = thinned(None);
    // The machine gun's row reads 0%: it is left unfired, and the guns are
    // read at the locked cadence again.
    run.server.no_chance.insert("machine_gun".into());
    assert!(run.until(
        WEAPONS_LEARNED_MS + POLL_STARVE_MS + LIMIT_WINDOW_MS,
        |run| gun(run.lane.picture(), "machine_gun").and_then(|w| w.percent) == Some(0)
    ));
    let from = run.now;
    run.until(30_000, |_| false);
    let polls = run.sends_of("WEAPONSTATUS", from);
    println!("WEAPONSTATUS, a gun at 0%: {polls:?}");
    assert!(
        polls.len() as u64 >= 30_000 / (WEAPONS_LOCKED_MS + POLL_STARVE_MS),
        "{polls:?}"
    );
    // The locked cadence, held back at most until it is overdue (and a window
    // for a slot), and in the middle well under the learned one.
    let mut gaps: Vec<u64> = polls.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(
        gaps.iter()
            .all(|gap| *gap <= WEAPONS_LOCKED_MS + POLL_STARVE_MS + LIMIT_WINDOW_MS),
        "{polls:?}"
    );
    gaps.sort_unstable();
    assert!(gaps[gaps.len() / 2] < WEAPONS_LEARNED_MS, "{polls:?}");
    assert!(
        gaps[gaps.len() / 2] <= WEAPONS_LOCKED_MS + TICK_MS,
        "the locked cadence on time in the middle: {polls:?}"
    );
    assert!(
        run.sends_of("FIRE machine_gun", from).is_empty(),
        "a gun at 0% is not fired"
    );
}

/// **`WEAPONSTATUS` thins on the arena's lattice too** (cut AL.1): with the
/// scripted arena starting each shot at its next 250 ms lattice point and the
/// rounds counting down, a read that lands on the frame before a shot left
/// (its row held by the stale-ready guard) still reads the chance under the
/// lock, so the learned cadence holds ([`thinned`]).
#[test]
fn weaponstatus_thins_on_the_arenas_lattice() {
    thinned(Some(LATTICE_MS));
}

/// **No `FIRE` for a gun the tank has lost** (cut AL.1): the cannon lost
/// mid-fight (after it was read and fired) is typed at once more, answered as
/// a gun not carried, and never again, while the machine gun keeps its rate;
/// then the turret goes: nothing is fired from its line on, the
/// `WEAPONSTATUS` it asks goes within a tick and once, and after `This unit
/// carries no weapon.` nothing is fired either.
#[test]
fn no_fire_for_a_gun_the_tank_has_lost() {
    let mut run = to_the_fight(base());
    locked_and_learned(&mut run);
    assert!(
        !run.sends_of("FIRE cannon", 0).is_empty(),
        "the cannon fired"
    );
    assert!(run.until(30_000, |run| about_to_fire(run, "cannon")));
    run.server.lost.insert("cannon".into());
    let lost_at = run.now;
    run.until(20_000, |_| false);
    let cannon = run.sends_of("FIRE cannon", lost_at);
    assert_eq!(
        cannon.len(),
        1,
        "one FIRE, answered as not carried: {cannon:?}"
    );
    assert!(run.lane.picture().struck.contains("cannon"));
    assert!(
        run.said
            .iter()
            .any(|s| s == "No cannon aboard any more: struck from the fire schedule."),
        "{:?}",
        run.said
    );
    // The machine gun keeps its rate: a cycle, at most the slack's ceiling and
    // two ticks a shot.
    let (_, shot) = both_gaps(&run, lost_at);
    let mg = shot.get("machine_gun").expect("the machine gun fired");
    println!("the machine gun after the cannon was lost: {mg:?}");
    let most = Server::cycle_ms("machine_gun") + FIRE_SLACK_MAX_MS + 2 * TICK_MS;
    assert!(mg.p50_ms <= most, "{mg:?}");
    assert!(mg.gaps as u64 >= 20_000 / most - 1, "{mg:?}");
    // The turret goes, between ticks (no reply on its way), with the machine
    // gun due by the next tick and two slots free in it: a gun the doubt did
    // not hold would go beside the doubt's `WEAPONSTATUS`, before its reply.
    assert!(
        run.until(60_000, |run| the_turret_can_go(run, 2)),
        "no moment with the machine gun due and two slots free"
    );
    run.server.now = run.now;
    run.server.lose_turret();
    run.step();
    let turret_at = run
        .lane
        .picture()
        .weapons_doubt
        .expect("every gun in doubt");
    assert_eq!(turret_at, run.now);
    run.until(10_000, |_| false);
    let polls = run.sends_of("WEAPONSTATUS", turret_at);
    assert!(
        polls.first().is_some_and(|at| *at <= turret_at + TICK_MS),
        "the doubt's WEAPONSTATUS within a tick: {polls:?}"
    );
    assert_eq!(
        polls
            .iter()
            .filter(|at| **at < turret_at + WEAPONS_LOCKED_MS)
            .count(),
        1,
        "once for the doubt: {polls:?}"
    );
    let fires: Vec<&Send> = run
        .sent
        .iter()
        .filter(|s| s.lane_ms >= turret_at && s.line.starts_with("FIRE"))
        .collect();
    assert!(fires.is_empty(), "no FIRE after the turret: {fires:?}");
    let picture = run.lane.picture();
    assert!(picture.weapons.is_empty() && picture.weapons_doubt.is_none());
    assert!(
        run.said
            .iter()
            .any(|s| s
                == "The turret's weapons are gone: no FIRE until WEAPONSTATUS says what is left."),
        "{:?}",
        run.said
    );
}

/// A moment to lose the turret: between ticks, the last `WEAPONSTATUS`'s
/// reply read, the machine gun due by the next tick, and at most `lines` in the
/// limiter's window ending at that tick.
fn the_turret_can_go(run: &Run, lines: usize) -> bool {
    let next_tick = (run.now / TICK_MS + 1) * TICK_MS;
    let in_window = run
        .sent
        .iter()
        .filter(|s| s.lane_ms + LIMIT_WINDOW_MS > next_tick)
        .count();
    between_ticks(run)
        && last_weapons(run).is_some_and(|at| run.now >= at + 2 * STEP_MS)
        && gun(run.lane.picture(), "machine_gun")
            .and_then(|w| w.next_ready_ms)
            .is_some_and(|at| at <= next_tick)
        && in_window <= lines
}

/// **The doubt's `WEAPONSTATUS` skips the poll gap** (cut AL.1): the turret
/// lost inside [`POLL_GAP_MS`] of the last `WEAPONSTATUS`, with a slot free,
/// is asked about at the next tick all the same, and nothing is fired before
/// its reply.
#[test]
fn the_doubts_weaponstatus_skips_the_poll_gap() {
    let mut run = to_the_fight(base());
    locked_and_learned(&mut run);
    assert!(
        run.until(60_000, |run| {
            let next_tick = (run.now / TICK_MS + 1) * TICK_MS;
            the_turret_can_go(run, 3)
                && last_weapons(run).is_some_and(|at| next_tick < at + POLL_GAP_MS)
        }),
        "no moment inside a WEAPONSTATUS's poll gap with a slot free"
    );
    let last = last_weapons(&run).unwrap_or(0);
    let turret_at = run.now;
    run.server.now = run.now;
    run.server.lose_turret();
    run.until(LIMIT_WINDOW_MS, |_| false);
    let polls = run.sends_of("WEAPONSTATUS", last + 1);
    assert!(
        polls.first().is_some_and(|at| *at < last + POLL_GAP_MS),
        "the doubt's WEAPONSTATUS inside the gap after {last}: {polls:?}"
    );
    let fires = run
        .sent
        .iter()
        .filter(|s| s.lane_ms >= turret_at && s.line.starts_with("FIRE"))
        .count();
    assert_eq!(fires, 0, "nothing fired before the reply");
}

/// **An empty gun is not fired again** (cut AL.1): a magazine the rows had not
/// yet shown dry refuses the next `FIRE` as `Empty:`; the gun is struck from
/// the fire schedule, which is said once, and the cannon fires on.
#[test]
fn an_empty_gun_is_not_fired_again() {
    let mut run = to_the_fight_with(base(), |server| {
        server.magazine = Some(200);
    });
    locked_and_learned(&mut run);
    assert!(run.until(30_000, |run| about_to_fire(run, "machine_gun")));
    run.server.rounds.insert("machine_gun".into(), 0);
    let dry_at = run.now;
    run.until(20_000, |_| false);
    let mg = run.sends_of("FIRE machine_gun", dry_at);
    assert_eq!(mg.len(), 1, "one FIRE, refused as empty: {mg:?}");
    assert_eq!(
        run.said
            .iter()
            .filter(|s| *s == "The machine_gun is empty: struck from the fire schedule.")
            .count(),
        1,
        "{:?}",
        run.said
    );
    assert!(
        run.sends_of("FIRE cannon", dry_at).len() >= 2,
        "the cannon fires on"
    );
}

/// The last `WEAPONSTATUS` the lane typed, if any.
fn last_weapons(run: &Run) -> Option<u64> {
    run.sends_of("WEAPONSTATUS", 0).last().copied()
}

/// The server says `Firing {weapon}.` `delay_ms` on (a multiple of the step):
/// the lane reads it at that step. When it is read.
fn ack_in(run: &mut Run, delay_ms: u64, weapon: &str) -> u64 {
    run.server.now = run.now;
    run.server.say(delay_ms, &format!("Firing {weapon}."));
    run.now + delay_ms
}

/// A `WEAPONSTATUS` went at least [`WEAPONS_AFTER_SHOT_MS`] after the ack read
/// at `ack`, and its reply (said 3 ms after the line, read at the next step)
/// was read inside the first second after it.
fn read_in_first_second(run: &Run, ack: u64) -> bool {
    run.sends_of("WEAPONSTATUS", ack)
        .iter()
        .any(|at| *at >= ack + WEAPONS_AFTER_SHOT_MS && at + STEP_MS - ack < CYCLE_READ_WITHIN_MS)
}

/// Step to a quiet moment of an unlocked lane: between ticks, a second after
/// the last `WEAPONSTATUS` (the polls that go with it out of the limiter's
/// window) and a second before any of the five-second polls is due again.
fn a_quiet_moment(run: &mut Run) {
    assert!(run.until(2 * WEAPONS_IDLE_MS, |run| {
        between_ticks(run)
            && last_weapons(run).is_some_and(|at| {
                run.now >= at + LIMIT_WINDOW_MS + 2 * STEP_MS && run.now < at + 2 * LIMIT_WINDOW_MS
            })
    }));
}

/// **Each gun whose cycle is unknown is read inside its own first second**
/// (cut AL.1): the post-shot `WEAPONSTATUS` is kept per gun, so a second gun's
/// ack 100, 300, 480 or 600 ms after the first's moves neither gun's read out of
/// its first second, nor into the frame before its shot (under
/// [`WEAPONS_AFTER_SHOT_MS`] after its ack); and a periodic `WEAPONSTATUS`
/// that falls due just after an ack (its row may be the frame before the shot
/// left) does not cancel the read. Unlocked, so nothing is fired: the acks
/// are the server's alone, and the rows read `ready`, so no row sets a cycle.
#[test]
fn each_unlearned_gun_is_read_inside_its_own_first_second() {
    for apart in [100, 300, 480, 600] {
        let mut run = to_the_fight(base());
        a_quiet_moment(&mut run);
        assert!(
            GUNS.iter()
                .all(|w| gun(run.lane.picture(), w).is_some_and(|g| g.cycle_ms.is_none())),
            "{:?}",
            run.lane.picture().weapons
        );
        let cannon = ack_in(&mut run, STEP_MS, "cannon");
        let mg = ack_in(&mut run, STEP_MS + apart, "machine_gun");
        run.until(3_000, |_| false);
        for (weapon, ack) in [("cannon", cannon), ("machine_gun", mg)] {
            assert!(
                read_in_first_second(&run, ack),
                "{weapon} acked at {ack}, the guns {apart} ms apart: WEAPONSTATUS at {:?}",
                run.sends_of("WEAPONSTATUS", cannon)
            );
        }
    }
    // A periodic `WEAPONSTATUS` due 150 ms after an ack. The machine gun's
    // ack first, at a quiet moment: its post-shot read moves the periodic one
    // off the other polls' five-second beat, so it goes on its own.
    let mut run = to_the_fight(base());
    a_quiet_moment(&mut run);
    let mg = ack_in(&mut run, STEP_MS, "machine_gun");
    run.until(LIMIT_WINDOW_MS, |_| false);
    assert!(read_in_first_second(&run, mg));
    assert!(run.until(WEAPONS_IDLE_MS, |run| {
        between_ticks(run)
            && last_weapons(run).is_some_and(|at| run.now + 150 >= at + WEAPONS_IDLE_MS)
    }));
    let ack = ack_in(&mut run, STEP_MS, "cannon");
    run.until(3_000, |_| false);
    let polls = run.sends_of("WEAPONSTATUS", ack);
    assert!(
        polls
            .first()
            .is_some_and(|at| *at < ack + WEAPONS_AFTER_SHOT_MS),
        "the periodic one went first: {polls:?} after {ack}"
    );
    assert!(
        read_in_first_second(&run, ack),
        "the read the ack armed still went: {polls:?} after {ack}"
    );
}

/// **A gun is never typed again while its `FIRE` is unanswered** (cut AL.1):
/// with every `FIRE`'s answer slower than [`FIRE_GAP_MS`] and a tick (a remote
/// server), no gun's `FIRE` line follows its last one before that one's
/// answer, and both guns keep firing.
#[test]
fn no_second_fire_while_the_first_is_unanswered() {
    const ANSWER_MS: u64 = FIRE_GAP_MS + 2 * TICK_MS;
    let mut run = to_the_fight_with(base(), |server| {
        server.fire_answer_ms = ANSWER_MS;
    });
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    let from = run.now;
    run.until(30_000, |_| false);
    for weapon in GUNS {
        let fires = run.sends_of(&format!("FIRE {weapon}"), from);
        println!("{weapon}, answers {ANSWER_MS} ms on: FIRE at {fires:?}");
        assert!(
            fires.len() as u64
                >= 30_000
                    / (Server::cycle_ms(weapon) + ANSWER_MS + FIRE_SLACK_MAX_MS + 2 * TICK_MS),
            "{weapon} kept firing: {fires:?}"
        );
        for pair in fires.windows(2) {
            assert!(
                pair[1] >= pair[0] + ANSWER_MS,
                "{weapon} typed again at {} before its FIRE at {} was answered: {fires:?}",
                pair[1],
                pair[0]
            );
        }
    }
}

/// **The learned cadence needs each gun's chance read against the lock
/// held** (cut AL.1): with both cycles learned, a lock line said after the
/// last rows (as a new lock prints it) brings the guns' next read back to the
/// locked cadence, not the learned one.
#[test]
fn a_lock_line_after_the_rows_brings_back_the_locked_cadence() {
    let mut run = to_the_fight(base());
    locked_and_learned(&mut run);
    // The last reply's rows committed, the next read not yet due.
    assert!(run.until(
        WEAPONS_LEARNED_MS + POLL_STARVE_MS + LIMIT_WINDOW_MS,
        |run| {
            between_ticks(run)
                && last_weapons(run)
                    .is_some_and(|at| run.now >= at + POLL_GAP_MS && run.now < at + LIMIT_WINDOW_MS)
        }
    ));
    let last = last_weapons(&run).unwrap_or(0);
    run.server.now = run.now;
    run.server.say(0, "Locked on BA, 62% to hit.");
    run.until(WEAPONS_LEARNED_MS, |_| false);
    let next = run.sends_of("WEAPONSTATUS", last + 1);
    let most = WEAPONS_LOCKED_MS + POLL_STARVE_MS + LIMIT_WINDOW_MS;
    const { assert!(WEAPONS_LOCKED_MS + POLL_STARVE_MS + LIMIT_WINDOW_MS < WEAPONS_LEARNED_MS) };
    assert!(
        next.first().is_some_and(|at| *at <= last + most),
        "read again by {} after {last}: {next:?}",
        last + most
    );
}

/// **A lane that has read no gun keeps the unlearned cadence** (cut AL.1): no
/// row read (the first reply a transient, or a tank that carries none) is not
/// "every gun has its cycle", so the guns are asked for at the idle cadence,
/// never the learned one.
#[test]
fn a_lane_that_has_read_no_gun_keeps_the_unlearned_cadence() {
    let mut run = to_the_fight_with(base(), |server| {
        server.lost = GUNS.iter().map(|w| (*w).to_owned()).collect();
    });
    let from = run.now;
    run.until(20_000, |_| false);
    assert!(run.lane.picture().weapons.is_empty());
    let polls = run.sends_of("WEAPONSTATUS", from);
    assert!(polls.len() >= 3, "{polls:?}");
    for pair in polls.windows(2) {
        assert!(pair[1] - pair[0] < WEAPONS_LEARNED_MS, "{polls:?}");
    }
}

// -- cut AL.2: nose-on ----------------------------------------------------------

/// The signed turn from one whole heading to another, in `-179..=180`:
/// positive is clockwise.
fn signed_turn(from: u32, to: u32) -> i64 {
    let d = (i64::from(to) - i64::from(from)).rem_euclid(360);
    if d > 180 { d - 360 } else { d }
}

/// **A hold faces the hostile and changes range along the line of fire** (cut
/// AL.2): BA due north at 440 m, then at the machines' 420 m (its row reading
/// `400 m`), then 700 m, then 250 m. Every heading ordered is on BA; the tank
/// is stopped, never reversed at 420, then closes full ahead, then opens in
/// reverse.
#[test]
fn a_hold_faces_the_hostile_and_changes_range_along_the_line_of_fire() {
    let mut run = to_the_fight(base());
    run.server.ba_at = Some((0, 440));
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    let start = run.now;
    run.until(5_000, |_| false);
    assert_eq!(
        run.lane.pilot().last_speed_order,
        Some(0),
        "stopped at 440 m"
    );
    // 420 m, read `400 m`: never a reverse.
    run.server.ba_at = Some((0, 420));
    let at_420 = run.now;
    run.until(10_000, |_| false);
    assert!(
        run.lane
            .picture()
            .contacts
            .get("BA")
            .is_some_and(|ba| ba.range_m == Some(400)),
        "the row reads 400 m"
    );
    let speeds = speed_sends(&run, at_420, run.now);
    assert!(speeds.iter().all(|(_, kph)| *kph >= 0), "{speeds:?}");
    assert_eq!(
        run.lane.pilot().last_speed_order,
        Some(0),
        "stopped at 420 m"
    );
    // 700 m: closing, full ahead.
    run.server.ba_at = Some((0, 700));
    let far = run.now;
    assert!(
        run.until(5_000, |run| speed_sends(run, far, run.now)
            .iter()
            .any(|(_, kph)| *kph == 54)),
        "{:?}",
        speed_sends(&run, far, run.now)
    );
    // 250 m: opening, in reverse.
    run.server.ba_at = Some((0, 250));
    let near = run.now;
    assert!(
        run.until(5_000, |run| speed_sends(run, near, run.now)
            .iter()
            .any(|(_, kph)| *kph < 0)),
        "{:?}",
        speed_sends(&run, near, run.now)
    );
    run.until(2_000, |_| false);
    // Bow on throughout.
    let headings = heading_sends(&run, start);
    assert!(
        headings
            .iter()
            .all(|(_, deg)| angle_between(*deg, 0) <= HOLD_BOW_DEG),
        "{headings:?}"
    );
    let bow = last_heading(&run, run.now).expect("a heading ordered");
    assert!(angle_between(bow, 0) <= HOLD_BOW_DEG, "{bow}");
    assert_eq!(run.lane.picture_json()["pilot"]["hold"], "astern");
}

/// **A side hit turns the bow** (cut AL.2): hunting a lost hostile with
/// nothing held, `Hit on the hull's left facet: …` turns the tank within a
/// tick to 90° left of its bow, stopped, and says so.
#[test]
fn a_side_hit_turns_the_bow() {
    let mut run = to_the_fight(base());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    run.until(2_000, |_| false);
    // Cut AL.3: BA hurt before it goes, so the hunt drives rather than waits
    // stopped (the turn's own stop is what this asserts).
    run.server.now = run.now;
    run.server.say(0, "BA's turret is destroyed.");
    run.until(100, |_| false);
    assert!(run.lane.pilot().hurt.contains("BA"));
    run.server.contacts.clear();
    run.server.locked = None;
    run.server.now = run.now;
    run.server
        .say(0, "Lock on BA broken: contact gone.\nContact BA lost.");
    assert!(run.until(2_000, |run| run.lane.pilot().hunt.is_some()));
    run.until(1_000, |_| false);
    assert!(run.lane.pilot().hunt.is_some(), "still hunting");
    assert!(run.lane.picture().contacts.is_empty(), "nothing held");
    // A moment with the limiter's window all but empty (at most one line in
    // the last second), so the turn and the stop both have a slot at once.
    assert!(run.until(10_000, |run| {
        run.now.is_multiple_of(TICK_MS)
            && run
                .sent
                .iter()
                .filter(|s| s.lane_ms + LIMIT_WINDOW_MS > run.now + TICK_MS)
                .count()
                <= 1
    }));
    // The bow: the pilot's reference heading, which is the scripted tank's.
    let bow = run.lane.pilot().reference_heading(run.lane.picture());
    assert_eq!(bow, run.server.heading_now());
    let left = (bow + 270) % 360;
    run.server.now = run.now;
    run.server
        .say(0, "Hit on the hull's left facet: 200 of 250 armour left.");
    let hit = run.now;
    // Read at the next step, decided and typed at the tick after it.
    run.until(STEP_MS + TICK_MS, |_| false);
    let turned = heading_sends(&run, hit + 1);
    assert!(
        turned
            .first()
            .is_some_and(|(_, deg)| angle_between(*deg, left) <= 1),
        "{turned:?} for {left}"
    );
    let speeds = speed_sends(&run, hit + 1, run.now);
    assert_eq!(speeds.first().map(|(_, kph)| *kph), Some(0), "{speeds:?}");
    let sentence =
        format!("Turning into the shot: hit on the hull's left facet; facing {left:03}°.");
    assert!(run.said.contains(&sentence), "{sentence}: {:?}", run.said);
    let view = run.lane.picture_json();
    let threat = &view["pilot"]["threat"];
    assert_eq!(threat["bearing"], left, "{threat}");
    assert!(threat["until_s"].as_u64().is_some_and(|s| s >= 1));
    assert_eq!(view["tally"]["hits_taken_by_facet"]["hull left"], 1);
}

/// **A one-track tank never circles for a small turn** (cut AL.2): with the
/// right track gone and BA's bearing wobbling 10° either side of the bow for a
/// minute, no heading is ordered that turns the tank clockwise (its dead side)
/// by at most `ONE_TRACK_DEADBAND_DEG`, and the tank never goes round; a 90°
/// clockwise need is ordered once and not again inside `ONE_TRACK_COMMIT_MS`
/// unless `STATUS` reads it.
#[test]
fn a_one_track_tank_never_circles_for_a_small_turn() {
    let mut run = to_the_fight(base());
    run.server.ba_at = Some((0, 450));
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    run.until(3_000, |_| false);
    assert_eq!(run.server.heading_now(), 0, "facing BA");
    run.server.now = run.now;
    run.server.lose_track("right_track");
    run.until(3_000, |_| false);
    assert_eq!(
        run.lane.picture_json()["pilot"]["live_turn"],
        "ccw",
        "{}",
        run.lane.picture_json()["pilot"]
    );
    let (from_heading, from_status) = (run.server.heading_log.len(), run.server.status_log.len());
    for k in 0..30 {
        run.server.ba_at = Some((if k % 2 == 0 { 10 } else { 350 }, 450));
        run.until(2_000, |_| false);
    }
    for (at, was, to) in &run.server.heading_log[from_heading..] {
        let s = signed_turn(*was, *to);
        assert!(
            !(s > 0 && s <= i64::from(ONE_TRACK_DEADBAND_DEG)),
            "a small clockwise turn ordered at {at}: {was} to {to}"
        );
    }
    let read = &run.server.status_log[from_status..];
    assert!(
        read.iter().all(|(_, h)| angle_between(*h, 0) <= 30),
        "the tank went round: {read:?}"
    );
    // A 90° clockwise need: ordered once, then nothing until it is read.
    run.server.ba_at = Some((90, 450));
    let need = run.now;
    assert!(run.until(3_000, |run| !heading_sends(run, need).is_empty()));
    // Two seconds into the long way round (the scripted tank near 300°), BA
    // moves to 170° clockwise of where the tank is then (about 110°): from
    // where `STATUS` reads the tank that is another clockwise turn well past
    // the deadband, which only the commit holds back (fixes after review: with
    // BA still, the lane's own dedupe held every re-send, commit or none).
    // (Cut AL.3: taken from the tank's own heading rather than fixed at 110°.
    // The wobble before can leave the bow a few degrees counter-clockwise of
    // 350° — a stop facing a return not yet told apart moved the lines' phase —
    // and from 345° the long way round reaches 287° in two seconds, from which
    // a fixed 110° is a counter-clockwise turn of 177°: the live side.)
    run.until(2_000, |_| false);
    let mid_turn = run.server.heading_now();
    assert!(
        angle_between(mid_turn, 300) <= 30,
        "mid-turn near 300°, BA's move near 110°: {mid_turn}"
    );
    let past = (mid_turn + 170) % 360;
    let read = run.lane.picture().own.heading_deg;
    assert!(
        signed_turn(read, past) > i64::from(ONE_TRACK_DEADBAND_DEG),
        "from where STATUS reads the tank ({read}°), BA at {past}° is a clockwise \
         turn past the deadband"
    );
    run.server.ba_at = Some((past, 450));
    run.until(ONE_TRACK_COMMIT_MS + 1_000, |_| false);
    let sends = heading_sends(&run, need);
    let (ordered, deg) = *sends.first().expect("the turn ordered");
    assert!(angle_between(deg, 90) <= 2, "{sends:?}");
    for (at, again) in &sends[1..] {
        if *at >= ordered + ONE_TRACK_COMMIT_MS {
            break;
        }
        let read_by_then = run.server.status_log.iter().any(|(due, h)| {
            *due > ordered && *due < *at && angle_between(*h, deg) <= HEADING_STEP_DEG
        });
        assert!(
            read_by_then,
            "HEADING {again} at {at}, committed to {deg} at {ordered}: {sends:?}"
        );
    }
    // Clockwise from where the tank was, so the server turned it the long
    // way round, counter-clockwise, as the live track does.
    let (_, was, to) = *run
        .server
        .heading_log
        .iter()
        .find(|(at, _, to)| *at == ordered && *to == deg)
        .expect("answered");
    assert!(signed_turn(was, to) > 0, "a clockwise need: {was} to {to}");
    let sentence = "The right_track is gone: turning counter-clockwise only.";
    assert_eq!(
        run.said.iter().filter(|s| *s == sentence).count(),
        1,
        "{:?}",
        run.said
    );
}

/// **The search does not drive back into held ground** (cut AL.2; the bot's
/// own run-2 loop): the held knob stops the searching tank; freed, and moved
/// west on the same line until the marked point is 10 m inside
/// `HELD_MARK_NEAR_M` (90 m at the built constants), every search `HEADING`
/// for the next 30 s misses the marked point by at least
/// `HELD_MARK_HALF_WIDTH_M`.
#[test]
fn the_search_does_not_drive_back_into_held_ground() {
    let mut run = to_the_fight_with(base(), |server| {
        server.held = Some(HeldKnob { free_off_deg: 30 });
        server.pos = (1_000, 970);
    });
    assert!(run.until(10_000, |run| run.lines().contains(&"SPEED 54")));
    run.until(2_500, |_| false);
    assert_eq!(run.server.heading, 90, "searching on 090");
    // Held just after a `STATUS` is read, so the pilot's own position is the
    // scripted one, not dead-reckoned on past it.
    let statuses = run.sends_of("STATUS", 0).len();
    assert!(run.until(3_000, |run| run.sends_of("STATUS", 0).len() > statuses));
    run.until(2 * STEP_MS, |_| false);
    run.server.now = run.now;
    run.server.hold_tank();
    assert!(run.until(1_000, |run| !run.lane.pilot().held_marks.is_empty()));
    let blocked = run.lane.pilot().held_marks[0].blocked_m;
    assert!(
        (blocked.0 - (1_000.0 + HELD_MARK_AHEAD_M)).abs() < 2.0 && (blocked.1 - 970.0).abs() < 1.0,
        "{blocked:?}"
    );
    assert_eq!(run.lane.picture_json()["pilot"]["marks"], 1);
    assert!(
        run.until(5_000, |run| run.server.held_from.is_none()),
        "freed"
    );
    run.until(HELD_FOR_MS + 1_000, |_| false);
    // West on the same line, the marked point 10 m inside the mark's reach
    // (90 m at the built constants, wherever a tuning round puts the reach):
    // the next `STATUS` reads it.
    let west = (HELD_MARK_NEAR_M - HELD_MARK_AHEAD_M - 10.0).round() as i32;
    run.server.pos = (1_000 - west, 970);
    let o = (f64::from(1_000 - west), 970.0);
    assert!(run.until(3_000, |run| run.lane.picture().own.pos_m == Some(o)));
    let from = run.now;
    run.until(30_000, |_| false);
    let sends = heading_sends(&run, from);
    assert!(!sends.is_empty(), "the search's heading");
    for (at, deg) in &sends {
        let r = f64::from(*deg).to_radians();
        let (dx, dy) = (blocked.0 - o.0, blocked.1 - o.1);
        let ahead = dx * r.sin() + dy * r.cos();
        let aside = (dx * r.cos() - dy * r.sin()).abs();
        assert!(
            ahead <= 0.0 || aside >= HELD_MARK_HALF_WIDTH_M,
            "HEADING {deg} at {at} passes {aside:.0} m from the held ground"
        );
    }
    assert!(
        run.said
            .iter()
            .any(|s| s.starts_with("Steering round held ground: ")),
        "{:?}",
        run.said
    );
}

// -- cut AL.3: seen second, and the engagement --------------------------------------

/// **A return not yet told apart is faced, stopped** (cut AL.3): a second and
/// a half after `Power online.` (before the search, so no heading has been
/// ordered yet), `New contact X1: bearing 087°, far.` and the scripted row
/// `X1  unknown  bearing 087° …  450 m  1 s`: within two ticks of the row
/// being read the lane types `HEADING 87` and `SPEED 0`, says so, and the
/// picture record names the wait. Told apart as BA 700 m off on that bearing,
/// the hold: bow on, closing full ahead.
#[test]
fn an_unidentified_contact_is_faced_stopped() {
    let mut run = to_the_fight(base());
    run.until(1_500, |_| false);
    assert!(
        heading_sends(&run, 0).is_empty(),
        "no heading before the search"
    );
    run.server.contacts = vec!["X1  unknown  bearing 087° (rel 357°)  far  450 m  1 s".into()];
    run.server.now = run.now;
    run.server.say(0, "New contact X1: bearing 087°, far.");
    let noticed = run.now;
    assert!(
        run.until(2_000, |run| run
            .lane
            .picture()
            .contacts
            .get("X1")
            .is_some_and(|c| c.range_m.is_some())),
        "the row is read"
    );
    let row = run.now;
    // The notice alone stopped nothing: no heading or speed went before the
    // row was read.
    assert!(
        heading_sends(&run, noticed)
            .iter()
            .all(|(at, _)| *at >= row)
            && speed_sends(&run, noticed, row)
                .iter()
                .all(|(at, _)| *at >= row),
        "a stop on the notice alone: {:?}",
        run.sent
    );
    run.until(2 * TICK_MS, |_| false);
    // The first heading and speed after the notice: 087° and 0, both inside
    // the row's first two ticks.
    let window = row..=row + 2 * TICK_MS;
    let headings = heading_sends(&run, noticed);
    let first_heading = headings.first().copied();
    assert!(
        first_heading.is_some_and(|(at, deg)| deg == 87 && window.contains(&at)),
        "{headings:?} after the row at {row}"
    );
    let speeds = speed_sends(&run, noticed, run.now);
    let first_speed = speeds.first().copied();
    assert!(
        first_speed.is_some_and(|(at, kph)| kph == 0 && window.contains(&at)),
        "{speeds:?} after the row at {row}"
    );
    assert!(
        run.said
            .contains(&"Stopping for X1: facing 087° while it is told apart.".to_owned()),
        "{:?}",
        run.said
    );
    let view = run.lane.picture_json();
    assert_eq!(view["pilot"]["wait"]["d"], "X1", "{}", view["pilot"]);
    assert!(
        view["pilot"]["wait"]["left_s"]
            .as_u64()
            .is_some_and(|s| (1..=UNIDENTIFIED_WAIT_MS / 1_000).contains(&s))
    );
    // Told apart: BA, 700 m off on 087°. The hold: bow on, full ahead.
    run.server.contacts = vec!["BA  blue  bearing 087° (rel 000°)  far  700 m  1 s".into()];
    run.server.now = run.now;
    run.server.say(0, "Contact X1 identified: BA, blue.");
    let told = run.now;
    assert!(run.until(3_000, |run| {
        speed_sends(run, told, run.now)
            .iter()
            .any(|(_, kph)| *kph == 54)
    }));
    let bow = last_heading(&run, run.now).expect("a heading ordered");
    assert!(angle_between(bow, 87) <= HOLD_BOW_DEG, "{bow}");
    let view = run.lane.picture_json();
    assert_eq!(view["pilot"]["hold"], "ahead", "{}", view["pilot"]);
    assert_eq!(view["pilot"]["wait"], Value::Null);
    assert_eq!(run.lane.pilot().unknown_stops, 1);
}

/// **The lane flies the engagement it is given** (cut AL.3): with
/// `engagement = Orbit` the pilot's own doctrine is the orbit, and BA held due
/// north at the band is circled — a heading on its tangent, 090° or 270°, at
/// full speed; with the default it is held — the bow on BA and stopped.
#[test]
fn the_lane_flies_the_engagement_it_is_given() {
    let fly = |settings: Settings| {
        let mut run = to_the_fight(settings);
        sight_a_hostile(&mut run);
        assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
        run.until(5_000, |_| false);
        let heading = last_heading(&run, run.now).expect("a heading ordered");
        (
            run.lane.doctrine().stance,
            heading,
            run.lane.pilot().last_speed_order,
            run.lane.picture_json()["doctrine"]["stance"].clone(),
        )
    };
    let (stance, heading, speed, word) = fly(Settings {
        engagement: Engagement::Orbit,
        ..base()
    });
    assert_eq!((stance, word), (Stance::Orbit, json!("orbit")));
    assert!(
        angle_between(heading, 90) <= 30 || angle_between(heading, 270) <= 30,
        "the orbit's tangent round BA due north: {heading}"
    );
    assert_eq!(speed, Some(54), "circling at full speed");
    let (stance, heading, speed, word) = fly(base());
    assert_eq!((stance, word), (Stance::Hold, json!("hold")));
    assert!(
        angle_between(heading, 0) <= HOLD_BOW_DEG,
        "the bow on BA: {heading}"
    );
    assert_eq!(speed, Some(0), "stopped at the band");
}

/// **An overruled withdraw is flown as the engagement the lane is given**
/// (cut AL.3): BA held fresh and the hull sound, a commander's `withdraw` is
/// flown as the pilot's own engagement — the orbit under `engagement = Orbit`,
/// the hold by default — and `commander-overruled` names it.
#[test]
fn an_overruled_withdraw_is_flown_as_the_lanes_engagement() {
    for (engagement, stance, word) in [
        (Engagement::Orbit, Stance::Orbit, "orbit"),
        (Engagement::Hold, Stance::Hold, "hold"),
    ] {
        let mut run = to_the_fight(Settings {
            engagement,
            ..base()
        });
        sight_a_hostile(&mut run);
        assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
        assert!(!hull_failing(run.lane.picture()), "60 of 60");
        let ba_fresh = |run: &Run| {
            run.lane
                .picture()
                .contacts
                .get("BA")
                .is_some_and(|c| c.age_ms <= FRESH_PLOT_MS)
        };
        assert!(run.until(3_000, ba_fresh), "BA's row read");
        run.ask(12_000).expect("an ask");
        assert!(
            ba_fresh(&run),
            "BA held fresh at the answer: {:?}",
            run.lane.picture().contacts
        );
        run.reply(answer("withdraw"));
        assert_eq!(run.lane.doctrine().stance, stance, "{word}");
        assert_eq!(
            run.records_of("commander-overruled"),
            vec![&json!({"from": "withdraw", "to": word, "why": "the hull is sound"})],
            "{word}"
        );
    }
}

// -- cut AM.1: replies on a slow wire --------------------------------------------

/// **A transient answering a `FIRE` re-arms that gun** (cut AM.1): the
/// machine gun's next `FIRE` goes on its own clock, inside a lattice step and
/// the limiter's window of the one the transient answered — not after the
/// mark lapses two seconds on — and no lobby line answers it.
#[test]
fn a_transient_answer_to_fire_rearms_that_gun() {
    const {
        assert!(
            FIRE_GAP_MS + LIMIT_WINDOW_MS < ACK_PAIR_MS,
            "the bound discriminates"
        )
    };
    let mut run = to_the_fight(base());
    locked_and_learned(&mut run);
    let from = run.now;
    run.server.fire_transient.insert("machine_gun".into());
    assert!(
        run.until(10_000, |run| run.server.fire_transient.is_empty()),
        "the machine gun fired"
    );
    let answered = run.sends_of("FIRE machine_gun", from)[0];
    assert!(
        run.until(5_000, |run| run.sends_of("FIRE machine_gun", from).len()
            >= 2)
    );
    let next = run.sends_of("FIRE machine_gun", from)[1];
    assert!(
        next - answered < FIRE_GAP_MS + LIMIT_WINDOW_MS,
        "typed again {} ms after the FIRE the transient answered",
        next - answered
    );
    assert!(
        run.sent
            .iter()
            .filter(|s| s.lane_ms >= from)
            .all(|s| matches!(s.purpose, Purpose::Poll | Purpose::Order)),
        "no lobby line answers it: {:?}",
        run.sent
            .iter()
            .filter(|s| s.lane_ms >= from && !matches!(s.purpose, Purpose::Poll | Purpose::Order))
            .collect::<Vec<_>>()
    );
}

/// **A transient answering the lobby's own line is the lobby's** (cut
/// AM.1): the claim answered `please retry in a moment` is typed again
/// [`TRANSIENT_RETRY_MS`] later, it then lands, and the run reaches the fight;
/// the claim waited in the ledger as the lobby's own line, and the transient
/// popped it.
#[test]
fn a_lobby_transient_never_reaches_the_cockpit() {
    let mut run = Run::new(base());
    run.server.lobby_transient = Some("SIM TEAM red".into());
    // Between the transient's queueing and its read, the claim waits in the
    // ledger as the lobby's own line, at its head (the ledger's unit test has
    // a transient pop such a line ahead of a later cockpit line); the
    // transient pops it.
    let mut claim_waiting = false;
    let mut popped = false;
    while run.now < 60_000 && !run.fighting() {
        let waiting = run.server.lobby_transient.is_none()
            && run.sends_of("SIM TEAM red", 0).len() < 2
            && run
                .lane
                .asked()
                .iter()
                .next()
                .is_some_and(|a| a.kind == AskKind::Lobby);
        claim_waiting |= waiting;
        run.step();
        popped |= waiting && run.lane.asked().is_empty();
    }
    assert!(
        run.fighting(),
        "the run reached the fight: {:?}",
        run.lines()
    );
    let claims = run.sends_of("SIM TEAM red", 0);
    assert_eq!(claims.len(), 2, "{:?}", run.lines());
    // Typed again at the first tick past both the retry (the transient is
    // read at the next step) and the limiter's free slot: the walk's lines
    // fill the second before it.
    let before: Vec<u64> = run
        .sent
        .iter()
        .map(|s| s.lane_ms)
        .filter(|at| *at < claims[1])
        .collect();
    let max = usize::try_from(base().max_lines_per_s).unwrap_or(usize::MAX);
    let slot = before
        .len()
        .checked_sub(max)
        .map_or(0, |i| before[i] + LIMIT_WINDOW_MS);
    let due = (claims[0] + STEP_MS + TRANSIENT_RETRY_MS).max(slot);
    let tick = due.div_ceil(TICK_MS) * TICK_MS;
    assert!(
        claims[1] >= claims[0] + TRANSIENT_RETRY_MS && claims[1] == tick,
        "typed again at {} after {}: due {due}, the slot {slot}",
        claims[1],
        claims[0]
    );
    assert!(
        claim_waiting,
        "the claim waited in the ledger as the lobby's line"
    );
    assert!(popped, "the transient popped it");
    assert!(
        run.said
            .iter()
            .any(|s| s == "Claimed red slot 1 (RA), leading the side.")
    );
}

/// A channel line `len` columns long: `[RedNet] Ada: ` and `x`s.
fn channel_line(len: usize) -> String {
    let head = "[RedNet] Ada: ";
    format!("{head}{}", "x".repeat(len - head.len()))
}

/// **A padded channel line never ends the match** (cut AM.1): a teammate's
/// channel line that runs to the width puts `Match 1 is over (…)` at the start
/// of the next line, flush; the lane reads it as the rest of that line. The
/// same when the pad is a run of spaces at the break (the renderer keeps it on
/// the wire, so the untrimmed width is the one measured). A short channel line
/// followed by the real end still ends the match.
#[test]
fn a_padded_channel_line_never_ends_the_match() {
    let end = "Match 1 is over (blue). Its units are checked in.";
    let mut run = to_the_fight(base());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    let running = run.lane.picture().phase.clone();
    assert!(matches!(running, Phase::Running { match_id: 1 }));
    // A 116-column channel line, and the sentence on the next line.
    let padded = channel_line(116);
    assert_eq!(
        mush_ansi::wrap_indent(&format!("{padded} {end}"), 120, 0),
        [padded.clone(), end.to_owned()],
        "the renderer breaks it there"
    );
    // A run of spaces at the break: one channel line, wrapped as the renderer
    // wraps it, whose first line ends in the spaces.
    let spaced = format!("{} {}{end}", channel_line(100), " ".repeat(19));
    let wrapped = mush_ansi::wrap_indent(&spaced, 120, 0);
    assert_eq!(wrapped.len(), 2, "{wrapped:?}");
    assert!(wrapped[1].starts_with("Match 1 is over"), "{wrapped:?}");
    assert!(
        wrapped[0].trim_end().chars().count() == 100 && wrapped[0].chars().count() > 115,
        "{wrapped:?}"
    );
    for chunk in [format!("{padded}\n{end}"), wrapped.join("\n")] {
        let typed = run.sent.len();
        run.server.now = run.now;
        run.server.say(0, &chunk);
        run.until(3_000, |_| false);
        assert_eq!(run.lane.picture().phase, running, "{chunk}");
        assert!(
            !run.said.iter().any(|s| s.starts_with("Match 1 is over")),
            "{:?}",
            run.said
        );
        assert!(run.sent.len() > typed + 4, "the lane keeps typing");
    }
    // A short channel line, then the real end.
    run.server.now = run.now;
    run.server.say(0, &channel_line(60));
    run.server.end(run.now, "blue");
    assert!(run.until(5_000, |run| run.lane.result().is_some()));
    assert_eq!(
        run.lane.picture().phase,
        Phase::Over {
            match_id: 1,
            result: ResultWord::Blue
        }
    );
    assert!(
        run.said
            .iter()
            .any(|s| s.starts_with("Match 1 is over (blue)")),
        "{:?}",
        run.said
    );
}

/// **A full-width channel line never swallows an expected answer** (cut
/// AM.1): a 119-column channel line the renderer would not wrap, read just
/// before the ack of a `FIRE` the lane typed — `Firing cannon.` would not fit
/// on it, but it is the answer the lane waits for, so it is the ack.
#[test]
fn a_full_width_channel_line_never_swallows_an_expected_answer() {
    let long = channel_line(119);
    assert_eq!(
        mush_ansi::wrap_indent(&long, 120, 0),
        std::slice::from_ref(&long)
    );
    let mut run = to_the_fight_with(base(), |server| server.fire_answer_ms = 20);
    locked_and_learned(&mut run);
    // A `FIRE cannon` the server acks (one it refused is tried again).
    let mut acked = false;
    for _ in 0..5 {
        let from = run.now + 1;
        assert!(run.until(15_000, |run| !run.sends_of("FIRE cannon", from).is_empty()));
        let shots = run.lane.picture().tally.shots;
        assert!(run.lane.picture().fire_pending("cannon"));
        if run.server.acks.last() != Some(&(run.now + 20, "cannon".to_owned())) {
            continue;
        }
        // The echo at +1, the channel line at +10, the ack at +20.
        run.server.now = run.now;
        run.server.say(10, &long);
        assert!(
            run.until(100, |run| run.lane.picture().tally.shots > shots),
            "the ack was read as the ack"
        );
        assert!(!run.lane.picture().fire_pending("cannon"), "its mark taken");
        acked = true;
        break;
    }
    assert!(acked, "the server acked a FIRE cannon");
}

/// **A chain runs on through whatever its pieces read** (cut AM.1, review):
/// one padded channel line, wrapped as the renderer wraps it, whose middle
/// piece is the echo of a line the lane typed, an indented piece (a run of
/// spaces straddling a break at the width), an answer the lane expects (a
/// transient-shaped piece while a typed line waits), or whose first line
/// itself ends `please retry in a moment.` — the sentence on the last piece is
/// still the rest of that channel line, and the match runs on.
#[test]
fn a_chain_runs_on_through_whatever_its_pieces_read() {
    let end = "Match 1 is over (blue). Its units are checked in.";
    let mut run = to_the_fight(base());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    let running = run.lane.picture().phase.clone();
    assert!(matches!(running, Phase::Running { match_id: 1 }));
    let wrap = |message: &str| mush_ansi::wrap_indent(message, 120, 0);
    // (a) The second piece is `CONTACTS`, the echo of a line the lane types.
    let echoed = wrap(&format!(
        "{} CONTACTS {} {end}",
        channel_line(115),
        "y".repeat(115)
    ));
    assert_eq!(echoed.len(), 4, "{echoed:?}");
    assert_eq!(echoed[1], "CONTACTS");
    // (b) A run of spaces at a full line starts the second piece indented.
    let indented = wrap(&format!(
        "{}   {} {end}",
        channel_line(120),
        "z".repeat(114)
    ));
    assert_eq!(indented.len(), 3, "{indented:?}");
    assert!(indented[1].starts_with(' ') && indented[1].chars().count() >= 115);
    // (c) The second piece reads as a transient.
    let busy = format!("{} please retry in a moment.", "w".repeat(89));
    let expected = wrap(&format!("{} {busy} {end}", channel_line(115)));
    assert_eq!(expected.len(), 3, "{expected:?}");
    assert_eq!(expected[1], busy);
    // (d) The channel line itself ends as a transient does.
    let retry = format!("{} please retry in a moment.", channel_line(90));
    let own = wrap(&format!("{retry} {end}"));
    assert_eq!(own, [retry.clone(), end.to_owned()]);
    for pieces in [echoed, indented, expected, own] {
        assert_eq!(pieces.last().map(String::as_str), Some(end));
        // Said just after a line the lane typed, before its answer: that
        // line waits in the ledger when the chunk is read.
        assert!(run.until(2_000, |run| {
            run.sent.last().is_some_and(|s| s.lane_ms == run.now)
        }));
        let typed = run.sent.len();
        run.server.now = run.now;
        run.server.say(2, &pieces.join("\n"));
        run.until(3_000, |_| false);
        assert_eq!(run.lane.picture().phase, running, "{pieces:?}");
        assert!(
            !run.said.iter().any(|s| s.starts_with("Match 1 is over")),
            "{pieces:?}: {:?}",
            run.said
        );
        assert!(run.sent.len() > typed + 4, "the lane keeps typing");
    }
}

/// **A chain ends with its output** (cut AM.1, review): the renderer writes a
/// wrapped line's pieces in one output, so a line read seconds after a
/// full-width channel line, with nothing between, is the game's own — here
/// `Match 1 is running.` three seconds after a teammate's 116-column line,
/// while the seated pilot types nothing.
#[test]
fn a_chain_ends_with_its_output() {
    let mut run = Run::new(base());
    assert!(run.until(30_000, |run| run.lines().contains(&"BOARD")));
    let running = run
        .server
        .queue
        .iter_mut()
        .find(|(_, chunk)| chunk.text().contains("is running"))
        .expect("the start queued");
    running.0 += 3_000;
    let at = running.0;
    run.server.now = run.now;
    run.server.say(50, &channel_line(116));
    let typed = run.sent.len();
    assert!(
        run.until(5_000, |run| matches!(
            run.lane.picture().phase,
            Phase::Running { .. }
        )),
        "{:?}",
        run.said
    );
    assert!(run.now >= at);
    assert!(
        run.sent[typed..]
            .iter()
            .all(|s| s.line != "STARTUP" || s.lane_ms >= at),
        "nothing typed between the channel line and the start but the start's own"
    );
}

/// **A full-width channel line never swallows a lock line or a claim** (cut
/// AM.1, review): the lock line that heads a `WEAPONSTATUS` reply the lane
/// asked for is part of that answer, and so is every line the centre says
/// while a lobby line awaits its answer — a 116-column channel line read just
/// before either leaves it read as itself.
#[test]
fn a_full_width_channel_line_never_swallows_a_lock_line_or_a_claim() {
    // The claim, straight after a channel line `Pilot` would not fit on.
    let mut run = Run::new(base());
    assert!(run.until(10_000, |run| run.lines().contains(&"SIM TEAM red")));
    let claim = run
        .server
        .queue
        .iter_mut()
        .find(|(_, chunk)| chunk.text().contains("takes red slot 1"))
        .expect("the claim queued");
    claim.1 = Chunk::Text(format!("{}\r\n{}", channel_line(116), claim.1.text()));
    assert!(run.until(1_000, |run| run.lane.lobby().slot().is_some()));
    assert!(
        run.said
            .iter()
            .any(|s| s == "Claimed red slot 1 (RA), leading the side."),
        "{:?}",
        run.said
    );
    // The lock line, straight after one `Locked` would not fit on.
    let mut run = to_the_fight(base());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.as_deref()
        == Some("BA")));
    let from = run.now + 1;
    assert!(run.until(10_000, |run| !run.sends_of("WEAPONSTATUS", from).is_empty()));
    let reply = run
        .server
        .queue
        .iter_mut()
        .find(|(_, chunk)| chunk.text().starts_with("Locked on BA, 62% to hit."))
        .expect("the locked reply queued");
    reply.1 = Chunk::Text(format!("{}\r\n{}", channel_line(116), reply.1.text()));
    let end = run.now + 2_000;
    while run.now < end {
        run.step();
        assert_eq!(
            run.lane.picture().lock.as_deref(),
            Some("BA"),
            "the lock held at {}",
            run.now
        );
    }
    assert!(
        run.journal.iter().any(|j| j.contains(&channel_line(116))),
        "the channel line was read as itself"
    );
}

/// **A wrapped board row never swallows a claim** (cut AM.1): a chain starts
/// only at a channel line, so a `SIM STATUS` side row that wraps flush, its
/// last piece 115 to 120 columns (past which `Pilot` would not fit), leaves
/// the claim after it read as the claim.
#[test]
fn a_wrapped_board_row_never_swallows_a_claim() {
    // A blue side row whose wrapped tail is 110 to 120 columns.
    let (row, tail) = (1..=6)
        .flat_map(|pilots| (0..40).map(move |pad| (pilots, pad)))
        .find_map(|(pilots, pad)| {
            let entries: Vec<String> = (1..=pilots)
                .map(|i| {
                    let letter = char::from(b'A' + i as u8 - 1);
                    format!(
                        "{i} (B{letter}): Pilot{}{i}, pod Blue Pod {i}, seated",
                        "z".repeat(pad)
                    )
                })
                .collect();
            let row = format!("  blue: {}", entries.join("; "));
            let wrapped = mush_ansi::wrap_indent(&row, 120, 0);
            let tail = wrapped.last()?.clone();
            (wrapped.len() >= 2 && (115..=120).contains(&tail.chars().count()))
                .then(|| (wrapped.join("\n"), tail))
        })
        .expect("a row whose tail is 115 to 120 columns");
    assert!(!tail.starts_with(' ') && !tail.starts_with('['), "{tail}");
    let mut run = Run::new(base());
    assert!(run.until(10_000, |run| run.lines().contains(&"SIM TEAM red")));
    // The claim's answer comes straight after the wrapped row.
    let claim = run
        .server
        .queue
        .iter_mut()
        .find(|(_, chunk)| chunk.text().contains("takes red slot 1"))
        .expect("the claim queued");
    claim.1 = Chunk::Text(format!(
        "Simulator centre: selection\r\n  red: nobody\r\n{}\r\n{}",
        row.replace('\n', "\r\n"),
        claim.1.text()
    ));
    assert!(run.until(1_000, |run| run.lane.lobby().slot().is_some()));
    assert!(
        run.said
            .iter()
            .any(|s| s == "Claimed red slot 1 (RA), leading the side."),
        "{:?}",
        run.said
    );
}

/// **A re-board while locked types no early `RANGE`** (cut AM.1): the
/// fight's reply count is seeded when the fight is made, so the lock's
/// `RANGE` follows the second reply read after it, never one read before.
#[test]
fn a_reboard_while_locked_types_no_early_range() {
    let mut run = to_the_fight(base());
    sight_a_hostile(&mut run);
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    run.until(5_000, |_| false);
    assert!(
        !run.sends_of("RANGE", 0).is_empty(),
        "RANGE before the drop"
    );
    run.server.outside = true;
    assert!(run.until(5_000, |run| run.lane.step() == Step::Reboard));
    assert!(run.until(10_000, |run| run.lane.step() == Step::Fight));
    let back = run.now;
    assert_eq!(
        run.lane.picture().lock.as_deref(),
        Some("BA"),
        "still locked"
    );
    run.until(5_000, |_| false);
    let contacts = run.sends_of("CONTACTS", back);
    let ranges = run.sends_of("RANGE", back);
    assert!(
        contacts.len() >= 3 && !ranges.is_empty(),
        "{contacts:?} {ranges:?}"
    );
    // Each reply comes 3 ms after its line: read at the next 10 ms step.
    assert!(
        ranges[0] > contacts[1],
        "the first RANGE ({}) follows the second reply after the re-board ({contacts:?})",
        ranges[0]
    );
}

/// To the fight with BA at 450 m east and the friendly RB held, locked on BA.
fn locked_with_a_friend() -> Run {
    locked_with_a_friend_in(base())
}

/// [`locked_with_a_friend`] flying `settings` (cut AN.3).
fn locked_with_a_friend_in(settings: Settings) -> Run {
    let mut run = to_the_fight_with(settings, |server| {
        server.ba_at = Some((90, 450));
    });
    sight(&mut run, "X1");
    run.server
        .contacts
        .push("RB  red  bearing 180° (rel 090°)  far  500 m  1 s".into());
    assert!(run.until(5_000, |run| run.lane.picture().lock.as_deref()
        == Some("BA")));
    run
}

/// The seed the slow-wire minutes run on.
const WIRE_SEED: u64 = 0x5eed_0a11_cafe_f00d;

/// **The wire does what it says** (cut AM.1): one seed gives one schedule,
/// nothing arrives before the line or piece before it, no line is injected
/// into a `STATUS` chunk, and a minute of fight splits, cuts mid-line, pauses
/// past the settle and injects at least ten times each.
#[test]
fn the_wire_is_ordered_seeded_and_does_what_it_says() {
    let schedule = |seed: u64| {
        let mut wire = Wire::new(seed);
        let mut up = Vec::new();
        let mut down = Vec::new();
        for i in 0..40u64 {
            wire.send(i * 100, &format!("LINE {i}"));
            wire.carry(i * 100 + 5, format!("a {i}\r\nb {i}°\r\nc {i}\r\n"));
            up.extend(wire.arrived(i * 100 + 50));
            down.extend(wire.delivered(i * 100 + 50));
        }
        up.extend(wire.arrived(u64::MAX));
        down.extend(wire.delivered(u64::MAX));
        (up, down, wire.arrivals.clone(), wire.deliveries.clone())
    };
    let one = schedule(7);
    assert_eq!(one, schedule(7), "one seed, one schedule");
    assert_ne!(one, schedule(8));
    assert_eq!(one.1.concat().matches("\r\n").count(), 40 * 3 + 40 / 3);
    // A minute of fight on the wire.
    let mut run = locked_with_a_friend();
    run.wire = Some(Wire::new(WIRE_SEED));
    run.until(60_000, |_| false);
    let wire = run.wire.as_ref().expect("the wire");
    println!("{}", wire.report());
    assert!(
        wire.arrivals.windows(2).all(|w| w[0] <= w[1]),
        "up in order"
    );
    assert!(
        wire.deliveries.windows(2).all(|w| w[0] <= w[1]),
        "down in order"
    );
    assert!(
        wire.carried.iter().any(|(status, _)| *status),
        "STATUS blocks were carried"
    );
    assert!(
        !wire
            .carried
            .iter()
            .any(|(status, injected)| *status && *injected),
        "nothing injected into a STATUS block"
    );
    for (what, count) in [
        ("split chunks", wire.split_chunks),
        ("mid-line cuts", wire.mid_line_splits),
        ("gaps over the settle", wire.gaps_over_settle),
        ("injections", wire.injections),
    ] {
        assert!(count >= 10, "{what}: {count}; {}", wire.report());
    }
}

/// A minute on the slow wire after [`locked_with_a_friend`], `each` called
/// after every step from the switch on (with the switch's time). The run,
/// and how many sentences it had said at the switch.
fn a_minute_on_the_wire(each: impl FnMut(&Run, u64)) -> (Run, usize) {
    a_minute_on_the_wire_in(base(), each)
}

/// [`a_minute_on_the_wire`] flying `settings` (cut AN.3).
fn a_minute_on_the_wire_in(settings: Settings, mut each: impl FnMut(&Run, u64)) -> (Run, usize) {
    let mut run = locked_with_a_friend_in(settings);
    let switched = run.now;
    let said = run.said.len();
    run.wire = Some(Wire::new(WIRE_SEED));
    while run.now < switched + 60_000 {
        run.step();
        each(&run, switched);
    }
    println!("{}", run.wire.as_ref().expect("the wire").report());
    (run, said)
}

/// **The picture holds on a slow wire** (cut AM.1): a minute on the latency
/// rig — a mean round trip of about 820 ms on [`WIRE_SEED`] (up about 200 ms;
/// down about 625 ms, up to 1.7 s, the chunk behind a split one waiting for
/// its pieces), every chunk of two or more lines split in three pieces
/// [`WIRE_SPLIT_GAP_MS`] apart (each gap past a settle), a line of chatter or
/// a receiver notice inside every third such chunk that is not a `STATUS`
/// block (the rig's report prints the figures) — and from the first
/// `CONTACTS` reply typed after the switch the picture holds BA and RB and
/// the lock on BA at every tick; no hunt is said, the guns are never in doubt
/// and both stay carried.
#[test]
fn the_picture_holds_on_a_slow_wire() {
    let mut armed: Option<(u64, u64)> = None;
    let mut checked = 0;
    let (run, said_before) = a_minute_on_the_wire(|run, switched| {
        let picture = run.lane.picture();
        // The first `CONTACTS` typed after the switch, and the count then.
        if armed.is_none()
            && let Some(at) = run.sends_of("CONTACTS", switched).first()
        {
            armed = Some((*at, picture.contacts_read));
        }
        let Some((_, read_then)) = armed else {
            return;
        };
        assert!(picture.weapons_doubt.is_none(), "at {}", run.now);
        if picture.contacts_read <= read_then || !run.now.is_multiple_of(TICK_MS) {
            return;
        }
        checked += 1;
        let held: Vec<&str> = picture.contacts.keys().map(String::as_str).collect();
        assert!(
            held.contains(&"BA") && held.contains(&"RB"),
            "at {}: {held:?}",
            run.now
        );
        assert_eq!(picture.lock.as_deref(), Some("BA"), "at {}", run.now);
        assert!(
            gun(picture, "cannon").is_some() && gun(picture, "machine_gun").is_some(),
            "at {}: {:?}",
            run.now,
            picture.weapons
        );
    });
    assert!(checked > 1_000, "{checked} ticks checked");
    let hunts: Vec<&String> = run.said[said_before..]
        .iter()
        .filter(|s| s.starts_with("Hunting "))
        .collect();
    assert!(hunts.is_empty(), "{hunts:?}");
}

/// **The fire clock holds on a slow wire** (cut AM.1): over the same minute
/// a learned cycle is never short of the gun's own, each gun's shots are at
/// least one a cycle and three seconds, and at most one `FIRE` line in ten of
/// each gun is refused as still cycling. Prints each gun's lines, acks,
/// refusals, learned cycle and learned fraction (the share of its post-shot
/// rows that taught a cycle: the first row after each ack, teaching when it
/// reads `cycling` inside a second of that ack's `FIRE`), and the rig's mean
/// round trip.
#[test]
fn the_fire_clock_holds_on_a_slow_wire() {
    // Per gun: the rows read after each ack (each ack's first, `ready` or
    // `cycling`), and of those the ones that could teach the cycle: a
    // `cycling` row read inside a second of that ack's `FIRE`.
    let mut post_shot: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    let mut last_row: BTreeMap<&str, u64> = BTreeMap::new();
    let mut counted_ack: BTreeMap<&str, u64> = BTreeMap::new();
    let mut lines_at = BTreeMap::new();
    let mut refusals_at = BTreeMap::new();
    let mut shots_at = BTreeMap::new();
    let mut first = true;
    let (run, _) = a_minute_on_the_wire(|run, _| {
        if first {
            first = false;
            for weapon in GUNS {
                lines_at.insert(weapon, count(&run.server.fire_lines_by, weapon));
                refusals_at.insert(weapon, count(&run.server.cycling_refusals_by, weapon));
                shots_at.insert(weapon, count(&run.server.shots, weapon));
            }
        }
        for weapon in GUNS {
            let Some(w) = gun(run.lane.picture(), weapon) else {
                continue;
            };
            assert!(
                w.cycle_ms.is_none_or(|c| c == Server::cycle_ms(weapon)),
                "{weapon} learned {:?} at {}",
                w.cycle_ms,
                run.now
            );
            // A new row applied since the last step.
            if last_row.get(weapon) == Some(&w.read_ms) {
                continue;
            }
            last_row.insert(weapon, w.read_ms);
            let Some(fired) = w.fired_ms else {
                continue;
            };
            if w.read_ms <= fired || counted_ack.get(weapon) == Some(&fired) {
                continue;
            }
            counted_ack.insert(weapon, fired);
            let entry = post_shot.entry(weapon).or_default();
            entry.0 += 1;
            let sent = w.fired_sent_ms.unwrap_or(fired);
            if w.cycling_s.is_some() && w.read_ms - sent < CYCLE_READ_WITHIN_MS {
                entry.1 += 1;
            }
        }
    });
    let wire = run.wire.as_ref().expect("the wire");
    for weapon in GUNS {
        let lines = count(&run.server.fire_lines_by, weapon) - lines_at[weapon];
        let refused = count(&run.server.cycling_refusals_by, weapon) - refusals_at[weapon];
        let acks = count(&run.server.shots, weapon) - shots_at[weapon];
        let learned = gun(run.lane.picture(), weapon).and_then(|w| w.cycle_ms);
        let (rows, teaching) = post_shot.get(weapon).copied().unwrap_or((0, 0));
        println!(
            "slow wire: {weapon}: {lines} FIRE line(s), {acks} ack(s), {refused} refused as still \
             cycling, cycle learned {learned:?}, learned fraction {teaching} of {rows} post-shot \
             row(s); mean round trip {} ms",
            wire.mean_round_trip_ms()
        );
        let floor = 60_000 / (Server::cycle_ms(weapon) + 3_000);
        assert!(
            acks as u64 >= floor,
            "{weapon}: {acks} shots, at least {floor}"
        );
        assert!(refused * 10 <= lines, "{weapon}: {refused} of {lines}");
    }
}

// ---------------------------------------------------------------------------
// Cut AM.2: the stop races, the pause, the blocked report, the walk on the wire
// ---------------------------------------------------------------------------

/// Step `run` until the lane has typed `line` (60 s at most); when it went.
fn until_typed(run: &mut Run, line: &str) -> u64 {
    assert!(
        run.until(60_000, |run| run.lines().contains(&line)),
        "{line} never typed; sent {:?}, said {:?}",
        run.lines(),
        run.said
    );
    run.sends_of(line, 0)[0]
}

/// **A stop racing a claim never holds the slot** (cut AM.2): stopped right
/// after `SIM TEAM red` is typed, with the claim two seconds away, the lane
/// says it waits, types exactly one `SIM LEAVE` once the claim has come, hears
/// it given up, and types nothing after it.
#[test]
fn a_stop_racing_a_claim_never_holds_the_slot() {
    let mut run = Run::new(base());
    run.server.claim_answer_ms = 2_000;
    let claim = until_typed(&mut run, "SIM TEAM red");
    run.lane.stop();
    // `Lane::stop`'s events reach the run at its next step.
    run.until(10_000, |_| false);
    assert!(
        run.said.iter().any(|s| s
            == "Stopping: waiting for the answer to `SIM TEAM red` before giving the slot up."),
        "{:?}",
        run.said
    );
    let leaves = run.sends_of("SIM LEAVE", 0);
    assert_eq!(leaves.len(), 1, "{:?}", run.lines());
    assert!(
        leaves[0] >= claim + 2_000,
        "the leave at {} before the claim at {}",
        leaves[0],
        claim + 2_000
    );
    assert_eq!(run.lines().last(), Some(&"SIM LEAVE"), "nothing after it");
    assert!(run.said.iter().any(|s| s == "Gave up red slot 1."));
    assert!(!run.lane.leave_pending(run.now));
    assert!(run.lane.result().is_none(), "a stop is not a result");
    assert!(!run.lines().contains(&"SIM START"));
}

/// **A pause longer than the step timeout does not block** (cut AM.2): the
/// lane is paused 100 ms after `SIM TEAM red` is typed with the claim
/// `step_timeout_ms + 8 s` away, and stepped (typing nothing, chatter read
/// half a second before the resume) until `step_timeout_ms + 5 s` past the
/// typing; resumed, it waits on for the claim (about 3 s later) and flies.
/// Before the cut the first tick after the resume blocked with ``No answer to
/// `SIM TEAM red` within 15 s.``.
#[test]
fn a_pause_longer_than_the_step_timeout_does_not_block() {
    let timeout = base().step_timeout_ms;
    let mut run = Run::new(base());
    run.server.claim_answer_ms = timeout + 8_000;
    let claim = until_typed(&mut run, "SIM TEAM red");
    run.until(claim + 100 - run.now, |_| false);
    run.lane.pause(run.now);
    assert!(run.lane.paused());
    let typed = run.sent.len();
    let resume_at = claim + timeout + 5_000;
    run.until(resume_at - 500 - run.now, |_| false);
    run.server.now = run.now;
    run.server.say(0, "Ada says, \"Anyone on red?\"");
    run.until(resume_at - run.now, |_| false);
    assert_eq!(
        run.sent.len(),
        typed,
        "typed while paused: {:?}",
        &run.lines()[typed..]
    );
    assert_eq!(run.lane.lobby().blocker(), None);
    run.lane.resume(run.now);
    assert!(!run.lane.paused());
    assert!(
        run.until(60_000, Run::fighting),
        "never reached the fight; said {:?}",
        run.said
    );
    assert_eq!(run.lane.lobby().blocker(), None);
    assert_eq!(run.lane.lobby().slot(), Some(1));
    for sentence in [
        "Paused: typing nothing until /resume; still reading.",
        "Resumed after 20 s; every wait starts again from now.",
        "Claimed red slot 1 (RA), leading the side.",
    ] {
        assert!(
            run.said.iter().any(|s| s == sentence),
            "{sentence}: {:?}",
            run.said
        );
    }
    assert_eq!(run.sends_of("SIM TEAM red", 0).len(), 1, "claimed once");
}

/// **A match ending mid-pause rematches after the resume** (cut AM.2): two
/// matches; paused in match 1's fight, the match ends 10 s into the pause and
/// is read then; resumed 120 s in, the next `SIM TEAM red` goes
/// `REMATCH_AFTER_MS` after the resume (within a tick), not before, and
/// nothing is typed while paused.
#[test]
fn a_match_ending_mid_pause_rematches_after_the_resume() {
    let mut run = to_the_fight(Settings {
        matches: 2,
        ..base()
    });
    run.until(2_000, |_| false);
    let paused = run.now;
    run.lane.pause(paused);
    let typed = run.sent.len();
    run.until(10_000, |_| false);
    run.server.end(run.now, "red");
    run.until(paused + 120_000 - run.now, |_| false);
    assert_eq!(run.sent.len(), typed, "typed while paused");
    assert_eq!(
        run.lane.report().matches.len(),
        1,
        "match 1 read while paused"
    );
    assert_eq!(run.lane.report().matches[0].result, "red");
    let resumed = run.now;
    run.lane.resume(resumed);
    assert!(run.until(5_000, |run| {
        !run.sends_of("SIM TEAM red", resumed).is_empty()
    }));
    let claim = run.sends_of("SIM TEAM red", resumed)[0];
    assert!(
        (resumed + REMATCH_AFTER_MS..=resumed + REMATCH_AFTER_MS + TICK_MS).contains(&claim),
        "claimed at {claim}, resumed at {resumed}"
    );
    assert!(run.until(60_000, Run::fighting), "the second match flies");
}

/// **A resume mid-fight reads the tank again and orders again** (cut AM.2):
/// paused while the pilot drives (a speed of its own ordered, not the
/// ceilings' `SPEED 999`), the resume makes the fight afresh, as after a
/// re-board: the first lines after it are `STATUS` and `WEAPONSTATUS`, ahead
/// of every other cockpit line, and the speed it had ordered goes again
/// within the limiter's next second — the person may have moved the tank
/// while it sat paused. The old fight believed that speed stood and its
/// reads were not yet due.
#[test]
fn a_resume_mid_fight_reads_the_tank_and_orders_again() {
    let mut run = to_the_fight(base());
    let driving = |run: &Run| {
        run.lines()
            .iter()
            .any(|l| l.starts_with("SPEED ") && *l != "SPEED 999")
    };
    assert!(
        run.until(60_000, driving),
        "the pilot never drove: {:?}",
        run.lines()
    );
    run.until(300, |_| false);
    let speed = run
        .lines()
        .into_iter()
        .rfind(|l| l.starts_with("SPEED "))
        .map(str::to_owned)
        .expect("a speed");
    let paused = run.now;
    run.lane.pause(paused);
    let typed = run.sent.len();
    run.until(1_000, |_| false);
    assert_eq!(run.sent.len(), typed, "typed while paused");
    let resumed = run.now;
    run.lane.resume(resumed);
    run.until(TICK_MS, |_| false);
    let after: Vec<&str> = run.sent[typed..].iter().map(|s| s.line.as_str()).collect();
    assert_eq!(
        after.get(..2),
        Some(&["STATUS", "WEAPONSTATUS"][..]),
        "the tank read first after the resume: {after:?}"
    );
    assert!(
        run.until(1_100, |run| !run.sends_of(&speed, resumed).is_empty()),
        "`{speed}` never ordered again after the resume: {:?}",
        &run.lines()[typed..]
    );
}

/// **A blocked second match reports its blocker** (cut AM.2): match 1 won,
/// the re-claim refused as the side full, the run `blocked` with the blocker
/// first and the matches flown after it.
#[test]
fn a_blocked_second_match_reports_the_blocker() {
    let mut run = to_the_fight(Settings {
        matches: 2,
        ..base()
    });
    run.until(2_000, |_| false);
    run.server.refuse_claim = Some("The red side is full: it takes 1 pilot(s).".into());
    run.server.end(run.now, "red");
    assert!(run.until(10_000, |run| run.lane.result().is_some()));
    let result = run.lane.result().expect("a result");
    assert_eq!(result.status, "blocked");
    assert_eq!(
        result.outcome,
        "The red side is full. Stopped after 1 of 2 match(es) for red: won 1, lost 0, drew 0, \
         timed out 0, other 0."
    );
    assert_eq!(
        run.sends_of("SIM LEAVE", 0).len(),
        0,
        "a refused claim holds nothing"
    );
}

/// **The walk and the claim hold on a slow wire** (cut AM.2): AM.1's latency
/// rig from the first line, a line of chatter riding in front of `look`'s
/// echo, and `look` answered late with the Simulator Lobby: the quiet rule
/// walks on with `ooc`, and the run reaches the fight with no blocker, its
/// walk `look`, `ooc`, `simulator`, `SIM STATUS` in that order. At 1.8 s late
/// (on [`WIRE_SEED`]) the late answer lands after `simulator`'s own; at 1.4 s
/// it lands inside `ooc`'s flight, ahead of the Welcome Hall (the journal
/// shows it first), and is ignored there.
#[test]
fn the_walk_and_the_claim_hold_on_a_slow_wire() {
    for (late_ms, inside_ooc) in [(1_800, false), (1_400, true)] {
        let mut run = Run::new(base());
        run.wire = Some(Wire::new(WIRE_SEED));
        run.server.look_answer_ms = late_ms;
        run.server.look_room = Some("Simulator Lobby #50".into());
        run.server.chatter = Some("Ada says, \"Radio check.\"".into());
        assert!(
            run.until(60_000, Run::fighting),
            "{late_ms}: never reached the fight; sent {:?}, said {:?}",
            run.lines(),
            run.said
        );
        println!(
            "{late_ms} ms late: {}",
            run.wire.as_ref().expect("the wire").report()
        );
        assert_eq!(run.lane.lobby().blocker(), None, "{late_ms}");
        assert_eq!(
            run.lines()[..4],
            ["look", "ooc", "simulator", "SIM STATUS"],
            "{late_ms}: {:?}",
            run.lines()
        );
        assert_eq!(run.sends_of("SIM TEAM red", 0).len(), 1, "{late_ms}");
        if inside_ooc {
            let journal = run.journal.join("\n");
            let late = journal
                .find("Simulator Lobby #50")
                .expect("the late heading");
            let hall = journal.find("Welcome Hall #2").expect("the hall");
            assert!(
                late < hall,
                "{late_ms}: the late heading came first:\n{journal}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Cut AN.2: the opening, the halt and the re-approach, on the shipped defaults
// ---------------------------------------------------------------------------

/// The brief the scripted server's `brief` knob answers with: the shipped
/// header, `rays` and `No contacts.`.
fn brief_of(rays: impl Fn(&str, u32) -> String) -> Vec<String> {
    let mut lines = vec![
        "Window around RA: cells 0 to 38 east and 88 to 107 north of 256 by 256; you at (6, 97), \
         heading 090°; 10 m a cell."
            .to_owned(),
    ];
    for (word, deg) in mush_bot::arena::lines::BRIEF_BEARINGS {
        lines.push(rays(word, deg));
    }
    lines.push("No contacts.".to_owned());
    lines
}

/// **The shipped muster's brief**: north and south 10 m of road then grass,
/// east and west all road, the diagonals 10 m of road then grass.
fn road_brief() -> Vec<String> {
    brief_of(|word, deg| match deg {
        90 | 270 => format!("{word}: 190 m of road, then beyond the window."),
        _ => format!("{word}: 10 m of road, then grass to 90 m, then beyond the window."),
    })
}

/// Every sentence the pilot said that starts `head`.
fn said_starting<'a>(run: &'a Run, head: &str) -> Vec<&'a String> {
    run.said.iter().filter(|s| s.starts_with(head)).collect()
}

/// **The shipped muster's brief as the window clamps it** (cut AO.1):
/// [`road_brief`] with the west ray `60 m of road` to the arena's edge and the
/// east ray `320 m of road` to the window's reach.
fn muster_brief() -> Vec<String> {
    brief_of(|word, deg| match deg {
        270 => format!("{word}: 60 m of road, then the edge of the arena."),
        90 => format!("{word}: 320 m of road, then beyond the window."),
        _ => format!("{word}: 10 m of road, then grass to 90 m, then beyond the window."),
    })
}

/// **A muster with room behind it** (cut AO.1): where the opening's lane
/// tests whose subject is not the room are flown, 400 m off the west edge
/// (the other side's entry is then (2 160, 970)).
const ROOMY_POS: (i32, i32) = (400, 970);

#[test]
fn the_opening_types_view_brief_once_and_steps_off_the_road() {
    let mut run = to_the_fight_with(Settings::default(), |server| {
        server.brief = road_brief();
        server.pos = ROOMY_POS;
    });
    let power = run.now;
    // The brief, then the step: north at 15 kph.
    assert!(
        run.until(10_000, |run| run.lines().contains(&"SPEED 15")),
        "the step: {:?}",
        run.lines()
    );
    assert!(run.lines().contains(&"HEADING 0"), "{:?}", run.lines());
    assert_eq!(run.sends_of("VIEW BRIEF", 0).len(), 1);
    // The road's 10 m, half a cell and the cover: 35 m at the shipped 20.
    let step = 10.0 + 5.0 + OPENING_COVER_M;
    let stepping = format!("Opening: stepping off the road to the north ({step:.0} m).");
    assert!(run.said.contains(&stepping), "{:?}", run.said);
    assert_eq!(run.lane.picture_json()["pilot"]["opening"], "step");
    assert_eq!(run.lane.picture_json()["pilot"]["rule"], "opening");
    assert_eq!(
        run.lane.picture_json()["pilot"]["start_m"],
        json!([400.0, 970.0])
    );
    // The commander is told the rule in force (its first ask, due since the
    // power came on and taken now).
    let view = run.ask(200).expect("an ask");
    assert_eq!(view["rule"], "opening");
    // The server never moves the tank: it is moved to the aim.
    let aim = 970 + step.round() as i32;
    run.server.pos = (ROOMY_POS.0, aim);
    let moved = run.now;
    assert!(
        run.until(10_000, |run| !run.sends_of("SPEED 0", moved).is_empty()),
        "the hold: {:?}",
        run.lines()
    );
    // Facing the other side's entry (2 160, 970) from there: 091°.
    run.until(1_000, |_| false);
    assert!(
        heading_sends(&run, moved).iter().any(|(_, deg)| *deg == 91),
        "{:?}",
        heading_sends(&run, moved)
    );
    assert!(
        run.said.contains(
            &"Opening: holding near our entry, facing 091° toward the other side's entry."
                .to_owned()
        ),
        "{:?}",
        run.said
    );
    assert_eq!(run.lane.picture_json()["pilot"]["opening"], "hold");
    // Two minutes on: held there, no search, no second brief.
    let held = run.now;
    run.until(120_000, |_| false);
    assert!(run.sends_of("BEARING B", power).is_empty(), "no search");
    let ahead = speed_sends(&run, held, run.now);
    assert!(ahead.iter().all(|(_, kph)| *kph <= 0), "{ahead:?}");
    assert_eq!(run.sends_of("VIEW BRIEF", 0).len(), 1, "once a match");
    assert!(run.sent.iter().all(|send| allowed(&send.line)));
    assert!(limiter_holds(&run.sent, 4));
    let brief = run
        .sent
        .iter()
        .find(|send| send.line == "VIEW BRIEF")
        .expect("typed");
    assert_eq!(brief.purpose, Purpose::Poll);
}

#[test]
fn a_start_into_a_running_match_has_no_opening() {
    let mut run = Run::new(Settings::default());
    assert!(
        run.until(60_000, |run| run.lines().contains(&"STARTUP")),
        "{:?}",
        run.lines()
    );
    // The tank is powered already: `STARTUP`'s answer is that, not a start.
    for (_, chunk) in &mut run.server.queue {
        let text = chunk.text().to_owned();
        if text.contains("Startup sequence begun") {
            *chunk = Chunk::Text(String::new());
        } else if text.contains("Power online.") {
            *chunk = Chunk::Text("Power is already online.\r\n".to_owned());
        }
    }
    assert!(run.until(10_000, Run::fighting), "{:?}", run.lines());
    run.until(30_000, |_| false);
    assert!(run.sends_of("VIEW BRIEF", 0).is_empty());
    assert!(said_starting(&run, "Opening").is_empty(), "{:?}", run.said);
    assert_eq!(run.lane.picture_json()["pilot"]["opening"], "done");
    assert_eq!(run.lane.pilot().muster_m, None);
}

#[test]
fn a_posed_start_has_no_opening() {
    // The first `CONTACTS` after power holds an unknown at 500 m, told apart
    // as BA a second later.
    let mut run = Run::new(Settings::default());
    assert!(run.until(60_000, |run| run.lines().contains(&"STARTUP")));
    run.server.contacts = vec!["X1  unknown  bearing 090° (rel 000°)  far  500 m  0 s".into()];
    assert!(run.until(10_000, Run::fighting));
    run.server.now = run.now;
    run.server.queue_rendered(
        1_000,
        Chunk::Identify {
            was: "X1".into(),
            row: "BA  blue  bearing 090° (rel 000°)  far  500 m  1 s".into(),
        },
    );
    run.until(20_000, |_| false);
    assert!(
        run.sends_of("VIEW BRIEF", 0).is_empty(),
        "{:?}",
        run.lines()
    );
    assert!(said_starting(&run, "Opening").is_empty(), "{:?}", run.said);
    assert!(
        said_starting(&run, "Firing from the halt").is_empty(),
        "{:?}",
        run.said
    );
    // A `New contact` said before the first `STATUS`, nothing on the scope.
    let mut run = Run::new(Settings::default());
    assert!(run.until(60_000, |run| run.lines().contains(&"STARTUP")));
    let at = run
        .server
        .queue
        .iter()
        .find(|(_, chunk)| chunk.text().contains("Power online."))
        .map(|(at, _)| *at)
        .expect("the power queued");
    run.server.queue.push((
        at,
        Chunk::Text("New contact X1: bearing 090°, far.\r\n".to_owned()),
    ));
    assert!(run.until(10_000, Run::fighting));
    run.until(20_000, |_| false);
    assert!(
        run.sends_of("VIEW BRIEF", 0).is_empty(),
        "{:?}",
        run.lines()
    );
    assert!(said_starting(&run, "Opening").is_empty(), "{:?}", run.said);
}

/// **A mover is fired on from the halt** (cut AN.2): the opening holds on a
/// field of grass; BA comes on from 600 m at 10 m/s due east, read through the
/// scope's 50 m rows (too coarse for the picture to publish a track at that
/// speed), and is fired on with the tank stopped and its bow on BA — no
/// positive `SPEED` while BA is past the band, and `FIRE` lines go. Each row
/// 50 m nearer keeps the halt (it has not stopped coming on), so the halt is
/// over only when BA comes to the band and its slack (`BA is at the band.`),
/// never on `has stopped` or its time while BA closes.
#[test]
fn a_mover_is_fired_on_from_the_halt() {
    // Cut AO.1: from a muster with room behind it (the rig's default (60,
    // 970) has 60 m, and the opening would first move in).
    let mut run = to_the_fight_with(Settings::default(), |server| server.pos = ROOMY_POS);
    // The opening holds on a field of grass.
    assert!(
        run.until(10_000, |run| !said_starting(run, "Opening: holding")
            .is_empty()),
        "{:?}",
        run.said
    );
    // BA comes on from 600 m at 10 m/s, due east.
    run.server.ba_at = Some((90, 600));
    run.server.contacts = vec!["BA  blue  bearing 090° (rel 000°)  far  600 m  1 s".into()];
    run.server.now = run.now;
    run.server.say(0, "New contact BA: bearing 090°, far.");
    let sighted = run.now;
    // The band and its slack, as the shipped doctrine flies it (450 m and
    // 50), and the true range at which the scope's 50 m row first reads it.
    let band = 450.0 + HOLD_SLACK_M;
    let mut range = 600_u32;
    let mut past_band_until = sighted;
    let mut at_band_from = None;
    let mut halt_over = None;
    while range > 420 {
        run.until(1_000, |_| false);
        if halt_over.is_none() && !said_starting(&run, "The halt is over").is_empty() {
            halt_over = Some(run.now);
        }
        range = range.saturating_sub(10).max(420);
        run.server.ba_at = Some((90, range));
        if f64::from(range) > band {
            past_band_until = run.now;
        }
        if at_band_from.is_none() && f64::from(range) < band + 25.0 {
            at_band_from = Some(run.now);
        }
    }
    let at_band_from = at_band_from.expect("BA came to the band");
    run.until(2_000, |_| false);
    if halt_over.is_none() && !said_starting(&run, "The halt is over").is_empty() {
        halt_over = Some(run.now);
    }
    assert!(
        run.said
            .iter()
            .any(|s| s.starts_with("Firing from the halt at BA, ")),
        "{:?}",
        run.said
    );
    // Stopped while BA is past the band, firing, the bow on it.
    let speeds = speed_sends(&run, sighted, past_band_until);
    assert!(
        speeds.iter().all(|(_, kph)| *kph <= 0),
        "a positive SPEED while BA is past the band: {speeds:?}; said {:?}",
        run.said
    );
    assert!(
        run.sent.iter().any(|send| send.line.starts_with("FIRE ")
            && send.lane_ms >= sighted
            && send.lane_ms <= past_band_until),
        "fired from the halt"
    );
    let bows = heading_sends(&run, sighted);
    assert!(
        bows.iter()
            .filter(|(at, _)| *at <= past_band_until)
            .all(|(_, deg)| angle_between(*deg, 90) <= HOLD_BOW_DEG),
        "the bow on BA: {bows:?}"
    );
    // The halt is over once, at the band, and not before BA came to it.
    let over: Vec<&String> = said_starting(&run, "The halt is over");
    assert_eq!(
        over,
        ["The halt is over: BA is at the band."],
        "{:?}",
        run.said
    );
    let halt_over = halt_over.expect("the halt is over");
    assert!(
        halt_over > at_band_from,
        "over at {halt_over}, BA at the band from {at_band_from}"
    );
    assert_ne!(run.lane.picture_json()["pilot"]["rule"], "halt");
}

#[test]
fn the_reapproach_backs_off_and_waits_on_the_scripted_server() {
    // Mid-field, so the backing off stays on the ground.
    let mut run = to_the_fight_with(Settings::default(), |server| server.pos = (1_000, 970));
    assert!(run.until(10_000, |run| {
        !said_starting(run, "Opening: holding").is_empty()
    }));
    // BA stopped at the world point 420 m east of the tank.
    let ba = (1_420_i32, 970_i32);
    let place = |run: &mut Run| {
        let (x, _) = run.server.pos;
        let range = u32::try_from(ba.0 - x).unwrap_or(0);
        if range > 480 {
            // Past a stopped tank's sight (`help cockpit`): lost.
            if !run.server.contacts.is_empty() {
                run.server.contacts.clear();
                run.server.ba_at = None;
                run.server.now = run.now;
                run.server.say(0, "Contact BA lost.");
            }
        } else {
            run.server.ba_at = Some((90, range));
        }
    };
    run.server.contacts = vec!["BA  blue  bearing 090° (rel 000°)  far  400 m  1 s".into()];
    place(&mut run);
    run.server.now = run.now;
    run.server.say(0, "New contact BA: bearing 090°, far.");
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    // Hits taken on the front, enough to out-hit the pilot (two over the
    // floor), inside three quarters of the exchange's window: ten in 15 s at
    // the shipped figures.
    let taken = EXCHANGE_MIN_TAKEN.max(EXCHANGE_MARGIN) + 2;
    let spacing = EXCHANGE_WINDOW_MS * 3 / 4 / taken as u64;
    let hits = run.now;
    for i in 0..taken as u32 {
        run.server.now = run.now;
        run.server.say(
            0,
            &format!(
                "Hit on the hull's front facet: {} of 400 armour left.",
                390 - 10 * i
            ),
        );
        run.until(spacing, |_| false);
    }
    assert!(
        run.said
            .iter()
            .any(|s| s.starts_with("Opening the range on BA: ")),
        "{:?}",
        run.said
    );
    // The picture record and the commander are told: the exchange counted,
    // the rule in force.
    let json = run.lane.picture_json();
    let pilot = &json["pilot"];
    assert_eq!(pilot["rule"], "reapproach");
    assert!(
        pilot["exchange"]["taken"]
            .as_u64()
            .is_some_and(|n| n >= EXCHANGE_MIN_TAKEN as u64),
        "{pilot}"
    );
    let view = run.ask(200).expect("an ask");
    assert_eq!(view["rule"], "reapproach");
    let back = speed_sends(&run, hits, run.now);
    assert!(back.iter().any(|(_, kph)| *kph < 0), "{back:?}");
    // Backing west: BA stays where it is in the world.
    let backing = run.now;
    let mut stopped = None;
    // The stock tank's 27 kph astern, 7.5 m a second, in whole metres.
    let mut west_m = 0.0_f64;
    for _ in 0..60 {
        if run.lane.pilot().last_speed_order.is_some_and(|kph| kph < 0) {
            west_m += 7.5;
            run.server.pos.0 = 1_000 - west_m.round() as i32;
        }
        place(&mut run);
        run.until(1_000, |_| false);
        if !said_starting(&run, "Out of BA's sight at ").is_empty() {
            stopped = Some(run.now);
            break;
        }
    }
    let stopped = stopped.unwrap_or_else(|| panic!("never out of sight: {:?}", run.said));
    assert!(
        run.until(2_000, |run| !run
            .sends_of("SPEED 0", stopped - 1_000)
            .is_empty()),
        "{:?}",
        speed_sends(&run, backing, run.now)
    );
    // `REAPPROACH_OPEN_M` from the plot as the pilot reckons it. Its plot is BA's last row,
    // a range rounded to the scope's 50 m (up to 25 m off BA's true point);
    // and between two `STATUS` reads (2 s apart) it dead-reckons astern while
    // this rig moves the tank in whole seconds (up to 15 m more).
    let off = f64::from(ba.0 - run.server.pos.0);
    assert!(off >= REAPPROACH_OPEN_M - 25.0 - 2.0 * 7.5, "{off}");
    // The wait: no hunt for its length (from the step it began, at most a
    // second before it was seen said)…
    run.until(stopped + REAPPROACH_WAIT_MS - 1_000 - run.now, |_| false);
    assert!(said_starting(&run, "Hunting ").is_empty(), "{:?}", run.said);
    assert_eq!(run.lane.picture_json()["pilot"]["rule"], "approach_wait");
    // …then over, and the precedence flies what it would (here the hunt:
    // the trail's plot is younger than the hunt's ceiling).
    run.until(3_000, |_| false);
    let over = run
        .said
        .iter()
        .position(|s| {
            *s == format!(
                "No approach from BA in {} s; moving on.",
                REAPPROACH_WAIT_MS / 1_000
            )
        })
        .unwrap_or_else(|| panic!("the wait's end: {:?}", run.said));
    assert!(
        run.said
            .iter()
            .enumerate()
            .all(|(at, s)| !s.starts_with("Hunting ") || at > over),
        "{:?}",
        run.said
    );
}

#[test]
fn with_the_knobs_off_the_lane_is_as_it_was() {
    let mut run = to_the_fight(base());
    let power = run.now;
    run.until(10_000, |_| false);
    assert!(run.sends_of("VIEW BRIEF", 0).is_empty());
    let bearing = run.sends_of("BEARING B", power);
    assert!(
        bearing
            .first()
            .is_some_and(|at| *at >= power + 5_000 && *at <= power + 6_000),
        "{bearing:?}"
    );
    let headings = heading_sends(&run, power);
    assert!(
        headings
            .first()
            .is_some_and(|(at, deg)| *at >= power + 5_000 && *deg == 90),
        "{headings:?}"
    );
    assert!(said_starting(&run, "Opening").is_empty());
    assert_eq!(run.lane.picture_json()["pilot"]["opening"], "done");
    assert_eq!(run.lane.picture_json()["pilot"]["rule"], Value::Null);
}

// ---------------------------------------------------------------------------
// Cut AN.3: the lattice clock on the rig, the slow and the steady wire, and the
// broken machine on the scripted server
// ---------------------------------------------------------------------------

/// **The lattice rig** (cut AN.3): the arena fires on its 250 ms pulse,
/// says each shot's outcome on the lattice point it went on
/// (`lattice_notices`) and accepts a gun's `FIRE` only `accept_lag_ms` past
/// its ready instant (100 at the floor: inside the "+50 to +150" an earlier
/// reading of the transcripts gave; the muster runs read the live edge nearer
/// +250); a 200-round magazine. Locked on BA with both cycles learned first,
/// then two minutes. Prints each gun's two gap counts and its refusals; the
/// run, when the two minutes began, and each gun's `FIRE` lines and
/// still-cycling refusals in them.
fn at_the_lattice_floor(
    lattice: bool,
    accept_lag_ms: u64,
) -> (Run, u64, Vec<(&'static str, u32, u32)>) {
    let mut run = to_the_fight_with(Settings { lattice, ..base() }, |server| {
        server.pulse_ms = Some(LATTICE_MS);
        server.lattice_notices = true;
        server.accept_lag_ms = accept_lag_ms;
        server.magazine = Some(200);
    });
    locked_and_learned(&mut run);
    let start = run.now;
    let lines = run.server.fire_lines_by.clone();
    let refusals = run.server.cycling_refusals_by.clone();
    run.until(120_000, |_| false);
    assert_eq!(
        run.lane.picture().lock.as_deref(),
        Some("BA"),
        "locked throughout"
    );
    assert!(cycles_learned(&run), "{:?}", run.lane.picture().weapons);
    let (fire, shot) = both_gaps(&run, start);
    let mut counts = Vec::new();
    for weapon in GUNS {
        let lines = count(&run.server.fire_lines_by, weapon) - count(&lines, weapon);
        let refused = count(&run.server.cycling_refusals_by, weapon) - count(&refusals, weapon);
        println!(
            "lag {accept_lag_ms}, lattice {}: {weapon}: FIRE lines {:?}; shots {:?}; {refused} of \
             {lines} FIRE lines refused as still cycling",
            if lattice { "on" } else { "off" },
            fire.get(weapon),
            shot.get(weapon),
        );
        counts.push((weapon, lines, refused));
    }
    (run, start, counts)
}

/// **Full rate at the lattice floor** (cut AN.3): with the lattice on, the
/// machine gun's shots are a median of at most a cycle and a lattice step
/// apart (its line-mode floor) and a p90 of at most two steps, the cannon's
/// median at most its cycle and a step, at most one `FIRE` line in ten of
/// each gun refused as still cycling, and the lattice said once. The same
/// rig with the lattice off prints its figures for the record.
#[test]
fn full_rate_at_the_lattice_floor() {
    let (run, start, counts) = at_the_lattice_floor(true, 100);
    let (_, shot) = both_gaps(&run, start);
    let mg = shot.get("machine_gun").expect("machine-gun shots");
    assert!(mg.p50_ms <= 1_000 + LATTICE_MS, "{shot:?}");
    assert!(mg.p90_ms <= 1_000 + 2 * LATTICE_MS, "{shot:?}");
    let cannon = shot.get("cannon").expect("cannon shots");
    assert!(cannon.p50_ms <= 6_000 + LATTICE_MS, "{shot:?}");
    for (weapon, lines, refused) in counts {
        assert!(refused * 10 <= lines, "{weapon}: {refused} of {lines}");
    }
    assert!(
        gun(run.lane.picture(), "machine_gun").is_some_and(|w| w.on_lattice),
        "{:?}",
        run.lane.picture().weapons
    );
    assert_eq!(
        said_starting(&run, "Firing on the arena's lattice").len(),
        1,
        "{:?}",
        run.said
    );
    let json = run.lane.picture_json();
    assert!(json["pilot"]["lattice"]["phase"].is_u64(), "{json}");
    assert!(
        json["weapons"]
            .as_array()
            .is_some_and(|guns| guns.iter().any(|g| g["lattice"] == true)),
        "{json}"
    );
    // The lattice off, for the record: no assertion.
    let (off, off_start, _) = at_the_lattice_floor(false, 100);
    let (_, off_shot) = both_gaps(&off, off_start);
    println!("lattice off: shots {off_shot:?}");
    assert!(said_starting(&off, "Firing on the arena's lattice").is_empty());
}

/// **Past the guard, the bench** (cut AN.3): the same rig with the arena
/// accepting a `FIRE` only 225 ms past the ready instant, past the lattice
/// clock's guard, as the muster runs read the live arena. The clock's sends
/// are refused, each gun is benched (its note said) and flies AL's clock
/// through the bench. Prints both guns' figures with the lattice on and off
/// for the record; asserts only the bench, and that the machine gun's median
/// shot gap with the lattice on is no worse than off by more than a lattice
/// step (the clock tried again after each bench costs a refusal or two).
#[test]
fn past_the_guard_the_bench() {
    let (on, on_start, on_counts) = at_the_lattice_floor(true, 225);
    let (off, off_start, off_counts) = at_the_lattice_floor(false, 225);
    let (_, on_shot) = both_gaps(&on, on_start);
    let (_, off_shot) = both_gaps(&off, off_start);
    println!("lag 225: shots on {on_shot:?}; off {off_shot:?}");
    println!("lag 225: FIRE lines and refusals on {on_counts:?}; off {off_counts:?}");
    let benched = said_starting(&on, "The machine_gun is off the lattice clock");
    assert!(!benched.is_empty(), "{:?}", on.said);
    assert!(
        benched.iter().all(
            |note| *note == "The machine_gun is off the lattice clock for 30 s: refused twice."
        ),
        "{benched:?}"
    );
    assert!(said_starting(&off, "The machine_gun is off the lattice clock").is_empty());
    let mg_on = on_shot.get("machine_gun").expect("machine-gun shots, on");
    let mg_off = off_shot.get("machine_gun").expect("machine-gun shots, off");
    assert!(
        mg_on.p50_ms <= mg_off.p50_ms + LATTICE_MS,
        "on {on_shot:?}; off {off_shot:?}"
    );
}

/// **The slow wire is as it was with the lattice** (cut AN.3): AM.1's
/// latency rig on [`WIRE_SEED`] (a mean round trip of about 820 ms), flown
/// with the lattice on and with it off, types the same lines at the same
/// times, and no gun is ever on the lattice: over `LATTICE_RTT_MAX_MS` the
/// lattice is off, and above a round trip of about 250 ms no cycle is
/// learned.
#[test]
fn the_slow_wire_is_as_it_was_with_the_lattice() {
    let fly = |settings: Settings| {
        let mut on_lattice = false;
        let (run, _) = a_minute_on_the_wire_in(settings, |run, _| {
            on_lattice |= run.lane.picture().weapons.iter().any(|w| w.on_lattice);
        });
        (run, on_lattice)
    };
    let (on, on_lattice) = fly(Settings {
        lattice: true,
        ..base()
    });
    let (off, _) = fly(base());
    assert!(!on_lattice, "a gun on the lattice on the slow wire");
    let sends = |run: &Run| {
        run.sent
            .iter()
            .map(|send| (send.lane_ms, send.line.clone()))
            .collect::<Vec<_>>()
    };
    for weapon in GUNS {
        println!(
            "slow wire, lattice on: {weapon}: cycle learned {:?}; round trip {:?}",
            gun(on.lane.picture(), weapon).and_then(|w| w.cycle_ms),
            on.lane.picture().lattice.rtt()
        );
    }
    assert_eq!(sends(&on), sends(&off));
    assert!(said_starting(&on, "Firing on the arena's lattice").is_empty());
}

/// **A steady wire learns the lattice** (cut AN.3): 80 ms each way on the
/// lattice rig (no acceptance lag): both cycles are learned, the machine gun
/// is on the lattice, the round trip read is the wire's (150 to 200 ms), and
/// the machine gun's shots are a median of at most a cycle, a lattice step
/// and a tick apart.
#[test]
fn a_steady_wire_learns_the_lattice() {
    let mut run = to_the_fight_with(
        Settings {
            lattice: true,
            ..base()
        },
        |server| {
            server.pulse_ms = Some(LATTICE_MS);
            server.lattice_notices = true;
            server.magazine = Some(200);
        },
    );
    run.wire = Some(Wire::steady(80));
    locked_and_learned(&mut run);
    let start = run.now;
    run.until(60_000, |_| false);
    let picture = run.lane.picture();
    assert!(cycles_learned(&run), "{:?}", picture.weapons);
    assert!(
        gun(picture, "machine_gun").is_some_and(|w| w.on_lattice),
        "{:?}",
        picture.weapons
    );
    let rtt = picture.lattice.rtt();
    assert!(rtt.is_some_and(|r| (150..=200).contains(&r)), "{rtt:?}");
    let (fire, shot) = both_gaps(&run, start);
    println!("steady wire: FIRE lines {fire:?}; shots {shot:?}; round trip {rtt:?}");
    let mg = shot.get("machine_gun").expect("machine-gun shots");
    assert!(mg.p50_ms <= 1_000 + LATTICE_MS + TICK_MS, "{shot:?}");
}

/// The keep-out's note on BA, as the pilot says it.
const NOT_FOLLOWING_BA: &str =
    "Not following BA into its entry zone; backing off out of its sight.";

/// **BA at its own world point** (cut AN.3): `ba_at` is range and bearing
/// from the tank, so the test keeps BA's world point and the tank's (`x`,
/// metres east on the 970 m line) and re-computes the row from the two; past
/// a mover's 600 m the row goes, with `Contact BA lost.` said once.
fn place_ba(run: &mut Run, x: f64, ba: f64, seen: &mut bool) {
    run.server.pos.0 = x.round() as i32;
    let range = (ba - x).round();
    if range > 600.0 {
        if *seen {
            *seen = false;
            run.server.contacts.clear();
            run.server.ba_at = None;
            run.server.now = run.now;
            run.server.say(0, "Contact BA lost.");
        }
    } else if *seen {
        run.server.ba_at = Some((90, range as u32));
    }
}

/// **A broken machine is followed on the scripted server** (cut AN.3): from a
/// muster at (300, 970) (the other side's entry (2 260, 970) mirrored), BA is
/// sighted 450 m east, its turret destroyed, and drives east for home at
/// 15 m/s (the stock tank's 54 kph: the scope's 50 m rows and the lock's finer
/// ranges carry a track at that). The pilot follows it with a positive `SPEED`
/// toward it; once BA is within `ENTRY_KEEP_OUT_M` of the entry line (or the
/// tank within that and the band), it says so, reverses (a negative `SPEED`),
/// and stops (`SPEED 0`) out of BA's sight to wait; no `Hunting ` is said.
#[test]
fn a_broken_machine_is_followed_on_the_scripted_server() {
    let mut run = to_the_fight_with(Settings::default(), |server| server.pos = (300, 970));
    assert!(
        run.until(10_000, |run| !said_starting(run, "Opening: holding")
            .is_empty()),
        "{:?}",
        run.said
    );
    let line_x = 2_560.0 - 300.0;
    let home = line_x - ENTRY_KEEP_OUT_M + 60.0;
    let mut x = 300.0_f64;
    let mut ba = 750.0_f64;
    let mut seen = true;
    run.server.contacts = vec!["BA  blue  bearing 090° (rel 000°)  far  450 m  1 s".into()];
    place_ba(&mut run, x, ba, &mut seen);
    run.server.now = run.now;
    run.server.say(0, "New contact BA: bearing 090°, far.");
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    run.server.now = run.now;
    run.server.say(0, "BA's turret is destroyed.");
    let driving = run.now;
    let mut kept_out = None;
    let mut stopped = None;
    // The farthest east the tank came, and whether it drove ahead with the
    // follow the rule flown.
    let mut max_x = x;
    let mut ahead_following = false;
    for _ in 0..300 {
        if ba < home {
            ba = (ba + 15.0).min(home);
        }
        let kph = run.lane.pilot().last_speed_order.unwrap_or(0);
        x += f64::from(kph) / 3.6;
        max_x = max_x.max(x);
        place_ba(&mut run, x, ba, &mut seen);
        run.until(1_000, |_| false);
        ahead_following |= run.lane.pilot().rule() == Some("follow")
            && run.lane.pilot().last_speed_order.is_some_and(|kph| kph > 0);
        if kept_out.is_none() && run.said.iter().any(|s| s == NOT_FOLLOWING_BA) {
            kept_out = Some(run.now);
        }
        if let Some(from) = kept_out
            && !run.sends_of("SPEED 0", from).is_empty()
        {
            stopped = Some(run.now);
            break;
        }
    }
    let following = format!(
        "Following BA home: it is hurt and driving away; holding {FOLLOW_BAND_M:.0} m behind it."
    );
    assert!(run.said.contains(&following), "{:?}", run.said);
    let kept_out = kept_out.unwrap_or_else(|| panic!("never kept out: {:?}", run.said));
    let ahead = speed_sends(&run, driving, kept_out);
    assert!(ahead.iter().any(|(_, kph)| *kph > 0), "{ahead:?}");
    assert!(ahead_following, "never ahead on the follow: {ahead:?}");
    let back = speed_sends(&run, kept_out - 1_000, run.now);
    let reversed = back
        .iter()
        .position(|(_, kph)| *kph < 0)
        .unwrap_or_else(|| panic!("no reverse: {back:?}"));
    assert!(
        back.iter().skip(reversed).any(|(_, kph)| *kph == 0),
        "{back:?}"
    );
    assert!(stopped.is_some(), "{back:?}; said {:?}", run.said);
    assert!(
        said_starting(&run, "Waiting for BA to come back, ").len() == 1,
        "{:?}",
        run.said
    );
    assert!(said_starting(&run, "Hunting ").is_empty(), "{:?}", run.said);
    let json = run.lane.picture_json();
    assert_eq!(json["pilot"]["rule"], "return_wait", "{json}");
    // Short of the entry line: the tank never went past the keep-out with
    // the band by more than one step's travel at full ahead (15 m: the
    // keep-out is judged on the step after the line is crossed), however far
    // it then backed off.
    println!(
        "farthest east {max_x:.0} m; the own keep-out at {:.0} m",
        line_x - ENTRY_KEEP_OUT_M - FOLLOW_BAND_M
    );
    assert!(
        max_x <= line_x - ENTRY_KEEP_OUT_M - FOLLOW_BAND_M + 15.0,
        "{max_x}"
    );
    assert!(x < max_x, "backed off: {x} from {max_x}");
}

/// **A start into a running match follows nothing** (cut AN.3): `STARTUP`
/// into a tank already powered mid-match leaves no muster and so no entry
/// line; a hurt BA driving away is held as before, and nothing says
/// `Following `.
#[test]
fn a_start_into_a_running_match_follows_nothing() {
    let mut run = Run::new(Settings::default());
    run.server.pos = (300, 970);
    assert!(
        run.until(60_000, |run| run.lines().contains(&"STARTUP")),
        "{:?}",
        run.lines()
    );
    for (_, chunk) in &mut run.server.queue {
        let text = chunk.text().to_owned();
        if text.contains("Startup sequence begun") {
            *chunk = Chunk::Text(String::new());
        } else if text.contains("Power online.") {
            *chunk = Chunk::Text("Power is already online.\r\n".to_owned());
        }
    }
    assert!(run.until(10_000, Run::fighting), "{:?}", run.lines());
    run.until(2_000, |_| false);
    assert_eq!(run.lane.pilot().muster_m, None);
    let mut x = 300.0_f64;
    let mut ba = 750.0_f64;
    let mut seen = true;
    run.server.contacts = vec!["BA  blue  bearing 090° (rel 000°)  far  450 m  1 s".into()];
    place_ba(&mut run, x, ba, &mut seen);
    run.server.now = run.now;
    run.server.say(0, "New contact BA: bearing 090°, far.");
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    run.server.now = run.now;
    run.server.say(0, "BA's turret is destroyed.");
    let mut tracked = false;
    for _ in 0..40 {
        ba += 15.0;
        let kph = run.lane.pilot().last_speed_order.unwrap_or(0);
        x += f64::from(kph) / 3.6;
        place_ba(&mut run, x, ba, &mut seen);
        run.until(1_000, |_| false);
        tracked |= run
            .lane
            .picture()
            .contacts
            .get("BA")
            .is_some_and(|c| c.track.is_some());
    }
    assert!(tracked, "BA was seen driving away, a track published");
    assert!(run.lane.pilot().hurt.contains("BA"));
    assert!(
        said_starting(&run, "Following ").is_empty(),
        "{:?}",
        run.said
    );
    assert_eq!(run.lane.pilot().follow, None);
    assert_eq!(run.lane.pilot().ret, None);
}

// ---------------------------------------------------------------------------
// Cut AO.1: off the wall
// ---------------------------------------------------------------------------

/// **The opening moves in from the muster and holds with room** (cut AO.1):
/// the shipped defaults at the rig's (60, 970) with [`muster_brief`] (60 m of
/// road to the west edge): the move in is said, `HEADING 90` and a full-ahead
/// `SPEED` go; the test moves the tank to the aim (300, 970) and the brief
/// there to [`road_brief`]; then `SPEED 0`, the second `VIEW BRIEF`, the step
/// north and the hold, at least the room less the arrival east of the west
/// edge. Exactly two `VIEW BRIEF`s, every line in the vocabulary, and the
/// limiter holding.
#[test]
fn the_opening_moves_in_from_the_muster_and_holds_with_room() {
    let mut run = to_the_fight_with(Settings::default(), |server| {
        server.brief = muster_brief();
    });
    assert!(
        run.until(10_000, |run| !said_starting(run, "Opening: moving in ")
            .is_empty()),
        "the move in: {:?}",
        run.said
    );
    let leg = OPENING_ROOM_M - 60.0 + OPENING_ARRIVE_M;
    let moving = format!(
        "Opening: moving in {:.0} m toward the other side's entry: 60 m of ground behind us, \
         {:.0} m wanted.",
        leg.round(),
        OPENING_ROOM_M.round()
    );
    assert_eq!(said_starting(&run, "Opening: moving in "), [&moving]);
    assert!(
        run.until(3_000, |run| run.lines().contains(&"HEADING 90")
            && run.lines().contains(&"SPEED 54")),
        "{:?}",
        run.lines()
    );
    assert_eq!(run.lane.picture_json()["pilot"]["opening"], "inward");
    assert_eq!(run.lane.picture_json()["pilot"]["rule"], "opening");
    assert_eq!(run.sends_of("VIEW BRIEF", 0).len(), 1);
    // The server never moves the tank: it is moved to the aim, where the
    // brief has room.
    let aim_x = 60 + leg.round() as i32;
    run.server.pos = (aim_x, 970);
    run.server.brief = road_brief();
    let moved = run.now;
    assert!(
        run.until(10_000, |run| !run.sends_of("SPEED 0", moved).is_empty()),
        "the stop: {:?}",
        run.lines()
    );
    let stopped = run.sends_of("SPEED 0", moved)[0];
    assert!(
        run.until(10_000, |run| run.sends_of("VIEW BRIEF", 0).len() == 2),
        "the second brief: {:?}",
        run.lines()
    );
    let second = run.sends_of("VIEW BRIEF", 0)[1];
    assert!(stopped <= second, "stopped at {stopped}, asked at {second}");
    assert!(
        run.until(10_000, |run| run.lines().contains(&"SPEED 15")),
        "the step: {:?}",
        run.lines()
    );
    let step = 10.0 + 5.0 + OPENING_COVER_M;
    let stepping = format!("Opening: stepping off the road to the north ({step:.0} m).");
    assert!(run.said.contains(&stepping), "{:?}", run.said);
    assert!(
        heading_sends(&run, second).iter().any(|(_, deg)| *deg == 0),
        "{:?}",
        heading_sends(&run, second)
    );
    // Moved to the step's aim: the hold.
    run.server.pos = (aim_x, 970 + step.round() as i32);
    assert!(
        run.until(10_000, |run| !said_starting(run, "Opening: holding")
            .is_empty()),
        "the hold: {:?}",
        run.said
    );
    let held_x = run.lane.picture().own.pos_m.expect("a position").0;
    assert!(
        held_x >= OPENING_ROOM_M - OPENING_ARRIVE_M - 10.0,
        "held at {held_x}"
    );
    // A minute on: still two briefs.
    run.until(60_000, |_| false);
    assert_eq!(run.sends_of("VIEW BRIEF", 0).len(), 2, "{:?}", run.lines());
    assert!(
        run.sent
            .iter()
            .filter(|send| send.line == "VIEW BRIEF")
            .all(|send| send.purpose == Purpose::Poll)
    );
    assert!(run.sent.iter().all(|send| allowed(&send.line)));
    assert!(limiter_holds(&run.sent, 4));
    assert_eq!(run.lane.picture_json()["pilot"]["opening"], "hold");
}

/// **A re-approach at the edge holds rather than backing into it** (cut
/// AO.1): the re-approach alone on, the tank at (80, 1 005) and BA stopped
/// 420 m east out-hitting it. It opens the range in reverse; the test moves
/// the tank west as the reverse would; where the ground behind runs out it
/// stops (`SPEED 0`), says `No room behind` once, and gives nothing up in the
/// next `REAPPROACH_MAX_MS`.
#[test]
fn a_reapproach_at_the_edge_holds_rather_than_backing_into_it() {
    let mut run = to_the_fight_with(
        Settings {
            reapproach: true,
            ..base()
        },
        |server| server.pos = (80, 1_005),
    );
    // BA stopped at the world point 420 m east of the tank.
    let ba = (500_i32, 1_005_i32);
    let place = |run: &mut Run| {
        let (x, _) = run.server.pos;
        run.server.ba_at = Some((90, u32::try_from(ba.0 - x).unwrap_or(0)));
    };
    run.server.contacts = vec!["BA  blue  bearing 090° (rel 000°)  far  400 m  1 s".into()];
    place(&mut run);
    run.server.now = run.now;
    run.server.say(0, "New contact BA: bearing 090°, far.");
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    // Hits taken on the front, enough to out-hit the pilot.
    let taken = EXCHANGE_MIN_TAKEN.max(EXCHANGE_MARGIN) + 2;
    let spacing = EXCHANGE_WINDOW_MS * 3 / 4 / taken as u64;
    let hits = run.now;
    for i in 0..taken as u32 {
        run.server.now = run.now;
        run.server.say(
            0,
            &format!(
                "Hit on the hull's front facet: {} of 400 armour left.",
                390 - 10 * i
            ),
        );
        run.until(spacing, |_| false);
    }
    assert!(
        !said_starting(&run, "Opening the range on BA: ").is_empty(),
        "{:?}",
        run.said
    );
    let back = speed_sends(&run, hits, run.now);
    assert!(back.iter().any(|(_, kph)| *kph < 0), "{back:?}");
    // Backing west at the stock tank's 27 kph astern, 7.5 m a second.
    let mut west_m = 0.0_f64;
    let mut held = None;
    for _ in 0..30 {
        if run.lane.pilot().last_speed_order.is_some_and(|kph| kph < 0) {
            west_m += 7.5;
            run.server.pos.0 = 80 - west_m.round() as i32;
        }
        place(&mut run);
        run.until(1_000, |_| false);
        if !said_starting(&run, "No room behind: ").is_empty() {
            held = Some(run.now);
            break;
        }
    }
    let held = held.unwrap_or_else(|| panic!("never held: {:?}", run.said));
    assert!(
        run.until(2_000, |run| !run
            .sends_of("SPEED 0", held - 1_000)
            .is_empty()),
        "{:?}",
        speed_sends(&run, hits, run.now)
    );
    println!(
        "held at {} m east: {:?}",
        run.server.pos.0,
        said_starting(&run, "No room behind: ")
    );
    run.until(REAPPROACH_MAX_MS, |_| false);
    assert_eq!(
        said_starting(&run, "No room behind: ").len(),
        1,
        "{:?}",
        run.said
    );
    assert!(
        said_starting(&run, "Could not open the range").is_empty(),
        "{:?}",
        run.said
    );
    assert!(run.server.pos.0 >= 0, "never past the edge");
}

/// **A track-less broken machine is followed on the scripted server** (cut
/// AO.1): as AN.3's [`a_broken_machine_is_followed_on_the_scripted_server`],
/// BA kept at its own world point; but BA drives east at 10 m/s read on the
/// scope's rows alone (`ba_plotted_s`: no lock reads it finer), so the
/// picture never publishes a track for it. Its turret destroyed and its range
/// from the tank opening, it is followed: `Following BA home` said, a positive
/// `SPEED` toward it, and no `Hunting `.
#[test]
fn a_trackless_broken_machine_is_followed_on_the_scripted_server() {
    let mut run = to_the_fight_with(Settings::default(), |server| {
        server.pos = (300, 970);
        server.ba_plotted_s = Some(2);
    });
    assert!(
        run.until(10_000, |run| !said_starting(run, "Opening: holding")
            .is_empty()),
        "{:?}",
        run.said
    );
    let mut x = 300.0_f64;
    let mut ba = 750.0_f64;
    let mut seen = true;
    run.server.contacts = vec!["BA  blue  bearing 090° (rel 000°)  far  450 m  2 s".into()];
    place_ba(&mut run, x, ba, &mut seen);
    run.server.now = run.now;
    run.server.say(0, "New contact BA: bearing 090°, far.");
    assert!(run.until(5_000, |run| run.lane.picture().lock.is_some()));
    run.server.now = run.now;
    run.server.say(0, "BA's turret is destroyed.");
    let driving = run.now;
    let mut tracked = false;
    let mut ahead_following = None;
    for _ in 0..40 {
        ba += 10.0;
        let kph = run.lane.pilot().last_speed_order.unwrap_or(0);
        x += f64::from(kph) / 3.6;
        place_ba(&mut run, x, ba, &mut seen);
        run.until(1_000, |_| false);
        tracked |= run
            .lane
            .picture()
            .contacts
            .get("BA")
            .is_some_and(|c| c.track.is_some());
        if ahead_following.is_none()
            && run.lane.pilot().rule() == Some("follow")
            && run.lane.pilot().last_speed_order.is_some_and(|kph| kph > 0)
        {
            ahead_following = Some(run.now);
        }
        if ahead_following.is_some_and(|at| run.now >= at + 3_000) {
            break;
        }
    }
    println!(
        "BA at {ba:.0} m east, the tank at {x:.0}; said {:?}",
        said_starting(&run, "Following ")
    );
    assert!(!tracked, "BA published a track");
    let following = format!(
        "Following BA home: it is hurt and driving away; holding {FOLLOW_BAND_M:.0} m behind it."
    );
    assert!(run.said.contains(&following), "{:?}", run.said);
    assert!(ahead_following.is_some(), "never ahead on the follow");
    let ahead = speed_sends(&run, driving, run.now);
    assert!(ahead.iter().any(|(_, kph)| *kph > 0), "{ahead:?}");
    assert!(said_starting(&run, "Hunting ").is_empty(), "{:?}", run.said);
}
