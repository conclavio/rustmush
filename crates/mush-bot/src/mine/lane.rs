//! The mining pilot's lane (doc 20 cut SM.12c-bot): the loop as one machine of
//! steps, each *(a line, the answer it waits for)*, in a closed vocabulary.
//!
//! **One outstanding line at a time.** Each step either sends a line and waits
//! for its answer, or (a **follow-on**) sends nothing and waits for a tell the
//! step before it set going: the stake's record after the stake, a run's
//! cycles after the drill starts, the unload after the recover, the arrival
//! after the clearance.
//!
//! **Where a step's window starts** (the brief's D21, one rule): every
//! complete line received is kept in one ordered buffer; a step that sends a
//! line opens its window at the first line received after the send; a
//! follow-on opens its window at the line after the one that decided the step
//! before it, so lines already received after that point (one `feed` may carry
//! a filed line and its done line) are carried into it, never lost. Lines
//! before a window's start are never read by it. A settle (the two seconds
//! after a sounding's head or the map's legend) extends the deciding step's own
//! window and moves no start.
//!
//! **The order every window is judged in**: fail, skip, retry, done — the first
//! that matches decides, so `"Rig 1 unloads nothing"` fails before `"Rig 1
//! unloads "` can read as done. An answer that is neither a listed needle nor
//! the done needle decides nothing: the window keeps filling until the step's
//! deadline, whose failure quotes the window's last 600 bytes, which is how an
//! unlisted refusal is reported.
//!
//! **A transient refusal** (the world was committing another action, was
//! busy past its 250 ms deferral, or is still starting) is read before any
//! of that: the line never ran, so it is typed again a moment later
//! ([`TRANSIENT_RETRY_MS`], at most [`TRANSIENT_TRIES`] times in a step), and
//! those sends never count against the step's own tries.
//!
//! The terminal's echo of a line the lane typed and every heard sounding are
//! kept out of every window: the echo is not the game's answer, and a heard
//! line is another party's act, recorded (`LaneEvent::Heard`, the report's
//! `heard`) and never acted on.
#[path = "detachment.rs"]
pub mod detachment;

use std::collections::{BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::lines::{
    self, AGREED, ALL_BARREN, ARRIVED, ASHORE, BACK_IN_BAY, BOARDED, CLAIM_CAP, CLAIM_FUNDS,
    CLAIMED, CLEARANCE, COLD, DRILLING, DRIVE_HERE, GOODS_LEAVING, GOODS_WAIT, LAUNCHED,
    LEARN_NOTHING, LEASE_NO_LICENCE, LEASE_PREVIEWED, LEASE_SHORT, LEASE_SIGNED, LEGEND_HEAD,
    LICENCE_IN_FORCE, LICENCE_NONE, LICENCE_PREVIEWED, LOCKED, Listing, NO_HOLD_ROOM, NO_ORE,
    NO_ROOM, NOT_OPEN, PLAN_HEAD, PlanFigures, Pocket, RECOVER_PREVIEWED, REFITTED, RIG_ONE,
    ROOM_ARRIVALS, ROOM_CHANDLERY, ROOM_CONCOURSE, ROOM_EXCHANGE, ROOM_GALLERY, ROOM_LOBBY,
    ROOM_REGISTRY, SCAN_PREVIEWED, SCANNED, SELL_PREVIEW, SITE_CLOSED, SOUND_STARTED, SOUNDED,
    STAKE_COMMONS, STAKE_COOLING, STAKE_FILED, STAKE_IN_BAY, STAKE_LAPSING, STAKE_NO_EVIDENCE,
    STAKE_RECORDED, STAKE_YOURS, UNLOADED, UNLOADED_EMPTY, UNLOADED_NONE, WORKED_OUT, WOULD_MINE,
    has, normalise,
};
use super::report::{BeatSpan, HeardAt, MilestoneAt, Report, SoundingSpan};
use super::{Family, Settings, Site};
use crate::arena::Assembler;

/// At most two lines a second (revision 2's rule).
pub const LINE_GAP_MS: u64 = 500;
/// An immediate answer (the arena rig's `STEP_TIMEOUT`).
pub const ANSWER_MS: u64 = 20_000;
/// The cold-site and not-yet retries (the story's ten seconds apart).
pub const RETRY_MS: u64 = 10_000;
/// A site whose pad is not open yet (`+ship/plan` and `+ship/lift` answer
/// "is closed: its pad is not ready" until the site clerk's openings are in
/// place, about eight real minutes after a fresh boot at SM.12b's measure):
/// ten minutes of tries.
pub const SITE_TRIES: u32 = 60;
/// The story's at most 20 tries of a cold site.
pub const COLD_TRIES: u32 = 20;
/// `NOT_OPEN`, ten minutes.
pub const ENROL_TRIES: u32 = 60;
/// `+licence` every 5 s for two minutes.
pub const LICENCE_TRIES: u32 = 24;
/// The licence check's wait between tries.
pub const LICENCE_RETRY_MS: u64 = 5_000;
/// The lease's tries while the licence has not landed.
pub const LEASE_TRIES: u32 = 12;
/// Stale quotes (another pilot's deal moved a listing's terms) tolerated in
/// one loop, and stale sales retried.
pub const QUOTE_TRIES: u32 = 6;
/// The wait before a stale quote is quoted afresh or a stale sale sent again.
pub const STALE_RETRY_MS: u64 = 2_000;
/// `GOODS_WAIT` / `GOODS_LEAVING`, every 30 s.
pub const GOODS_TRIES: u32 = 6;
/// The goods waits' spacing.
pub const GOODS_RETRY_MS: u64 = 30_000;
/// A line the world refused as transient ([`lines::is_transient`]: it was
/// committing another action, or busy past its 250 ms deferral, or still
/// starting) is typed again at most this many times in one step — the test rig's
/// `RETRY_BUDGET`. Those sends never count against the step's own tries.
pub const TRANSIENT_TRIES: u32 = 40;
/// The moment a transient refusal asks for before the line goes again.
pub const TRANSIENT_RETRY_MS: u64 = 1_000;
/// A passage (3.7–4.8 real minutes at rate 24) and its clearance.
pub const HOP_MS: u64 = 15 * 60_000;
/// Launch 12.5 s, scan 25 s, sound 50 s, a drive, the recover and its unload.
pub const ACT_MS: u64 = 5 * 60_000;
/// A run of at most eight 75-s cycles (doc 20 §11 Caps).
pub const RUN_MS: u64 = 15 * 60_000;
/// The site clerk records a stake on a port fire (5 s period).
pub const STAKE_RECORD_MS: u64 = 180_000;
/// `sm_story`'s `MAX_RUNS`.
pub const MAX_RUNS: usize = 4;
/// `sm_story`'s six bunker rounds.
pub const MAX_BUNKER_ROUNDS: usize = 6;
/// Then the lane mines open ground (revision 2, F2).
pub const MAX_REFUSED_STAKES: u32 = 3;
/// `sm_story`'s measured 0, plus one (the brief's D9).
pub const EXTRA_BURN_T: u64 = 1;
/// `sm_story`'s measured 0, plus one (the brief's D9). SM.8's measured 1 was
/// the pad stay's charge on the story's own margin, which the leg-home floor
/// caps (doc 20b RK-1); the one kept pays a pad day's stay.
pub const EXTRA_CREW_DAYS: u64 = 1;
/// One IC hour at the play rate 24: outside the per-(sounder, site) heard
/// budget, so the second sounding is heard.
pub const SECOND_SOUNDING_AFTER_MS: u64 = 150_000;
/// How long a sounding's or a map's print may still be arriving after its
/// head.
pub const SETTLE_MS: u64 = 2_000;
/// How long `+market/list`'s board must be quiet before it is whole.
pub const QUIET_MS: u64 = 1_000;
/// The timed step after `+licence/buy go`: the purchase lands on the world's
/// next step.
pub const LICENCE_GO_MS: u64 = 2_000;
/// The wait before the next bunker round's plan (a Draw lands on the world's
/// next step).
pub const BUNKER_ROUND_MS: u64 = 5_000;
/// How much of a window a failure quotes.
const QUOTE_BYTES: usize = 600;
/// How many typed lines the echo filter remembers.
const ECHO_LINES: usize = 4;

/// The loop's beats (doc 20 §1, the SM.8 story's order, the added second
/// sounding).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Beat {
    KitUp,
    Bunkers,
    BoardAndLift,
    Witness,
    OnPad,
    DriveStakeMine,
    SecondSounding,
    Home,
    Sell,
    Done,
}

impl Beat {
    /// The beat's name as the report spells it (`kit_up`).
    pub fn as_str(self) -> &'static str {
        match self {
            Beat::KitUp => "kit_up",
            Beat::Bunkers => "bunkers",
            Beat::BoardAndLift => "board_and_lift",
            Beat::Witness => "witness",
            Beat::OnPad => "on_pad",
            Beat::DriveStakeMine => "drive_stake_mine",
            Beat::SecondSounding => "second_sounding",
            Beat::Home => "home",
            Beat::Sell => "sell",
            Beat::Done => "done",
        }
    }
}

/// A beat's edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Start,
    End,
}

/// The loop's milestones, each reached once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Milestone {
    ArrivedSite,
    Launched,
    Scanned,
    Sounded,
    Staked,
    FirstRunDone,
    SecondSounded,
    Recovered,
    Unloaded,
    DockedHome,
    Sold,
}

/// One line to type, its beat, and the lane's clock when it went.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Send {
    pub line: String,
    pub beat: Beat,
    pub lane_ms: u64,
}

/// What [`Lane::feed`] hands back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fed {
    /// The complete lines received, unfiltered, newline-joined, in order.
    pub journal: String,
}

/// What the runner logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaneEvent {
    /// A sentence for the `mine` event.
    Said(String),
    /// A beat began or ended.
    Beat {
        beat: Beat,
        edge: Edge,
        lane_ms: u64,
    },
    /// A milestone was reached.
    Milestone { name: Milestone, lane_ms: u64 },
    /// A heard sounding arrived.
    Heard { sentence: String, lane_ms: u64 },
}

/// How the lane ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneResult {
    /// `done` (the hold sold) or `blocked`.
    pub status: String,
    pub outcome: String,
}

/// **The closed vocabulary**: whether `line` is one the lane may type —
/// exactly `c`, `registry`, `out`, `concourse`, `exchange`, `chandlery`,
/// `arrivals`, `lobby`, `gallery`; `+coop/enrol <name>`; `+coop/refit
/// prospector`; `+licence/buy`, `+licence/buy go`, `+licence`; `+tv/lease`,
/// `+tv/lease go`; `+tv`; `+chart`; `+market/list`; `+market/quote
/// <n>=<qty>`; `+market/accept <n>`; `+ship/plan <site word|ceres>`;
/// `+ship/board`; `+ship/lift <site word|ceres>`; `+ship/leave`; `+tv/map
/// site`; `+tv/1 launch go`; `+tv/1 scan`; `+tv/1 scan go`; `+tv/1 sound go`;
/// `+tv/1 drive <ADDR> go`; `+claim/stake 1 go`; `+tv/1 mine metals|volatiles`
/// with or without ` go`; `+tv/1 recover`, `+tv/1 recover go`; `+market/sell
/// <n>`, `+market/sell <n>=all` — `<n>` and `<qty>` decimal digits, `<ADDR>`
/// a letter `A`–`P` and 1–3 digits, `<name>` 1–32 characters of `help coop`'s
/// rule. Nothing else, nothing beginning `@`.
pub fn in_vocabulary(line: &str) -> bool {
    const FIXED: [&str; 32] = [
        "c",
        "registry",
        "out",
        "concourse",
        "exchange",
        "chandlery",
        "arrivals",
        "lobby",
        "gallery",
        "+coop/refit prospector",
        "+licence/buy",
        "+licence/buy go",
        "+licence",
        "+tv/lease",
        "+tv/lease go",
        "+tv",
        "+chart",
        "+market/list",
        "+ship/board",
        "+ship/leave",
        "+tv/map site",
        "+tv/1 launch go",
        "+tv/1 scan",
        "+tv/1 scan go",
        "+tv/1 sound go",
        "+claim/stake 1 go",
        "+tv/1 mine metals",
        "+tv/1 mine volatiles",
        "+tv/1 mine metals go",
        "+tv/1 mine volatiles go",
        "+tv/1 recover",
        "+tv/1 recover go",
    ];
    let digits = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_ascii_digit());
    let route = |word: &str| word == "ceres" || Site::ALL.iter().any(|site| site.word() == word);
    if line.starts_with('@') {
        return false;
    }
    if FIXED.contains(&line) {
        return true;
    }
    if let Some(name) = line.strip_prefix("+coop/enrol ") {
        return (1..=32).contains(&name.chars().count())
            && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '\'' | '.'));
    }
    if let Some(rest) = line.strip_prefix("+market/quote ") {
        return rest
            .split_once('=')
            .is_some_and(|(n, qty)| digits(n) && digits(qty));
    }
    if let Some(n) = line.strip_prefix("+market/accept ") {
        return digits(n);
    }
    if let Some(rest) = line.strip_prefix("+market/sell ") {
        return digits(rest.strip_suffix("=all").unwrap_or(rest));
    }
    if let Some(word) = line.strip_prefix("+ship/plan ") {
        return route(word);
    }
    if let Some(word) = line.strip_prefix("+ship/lift ") {
        return route(word);
    }
    if let Some(rest) = line.strip_prefix("+tv/1 drive ") {
        return rest.strip_suffix(" go").is_some_and(lines::is_addr);
    }
    false
}

/// Which `+market/list` board a list step reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Board {
    Exchange,
    Chandlery,
    Sell,
}

/// What a step is: what its window is judged on and what follows it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Detachment(detachment::Task),
    /// A walk, done on the room's name.
    Walk {
        word: &'static str,
        room: &'static str,
    },
    Enrol,
    Refit,
    LicencePreview,
    /// Done [`LICENCE_GO_MS`] after it is sent, whatever arrived.
    LicenceGo,
    LicenceCheck,
    LeasePreview,
    LeaseGo,
    Rigs,
    Chart,
    List(Board),
    BunkerPlan,
    Quote,
    Accept,
    Board,
    PlanOut,
    LiftOut,
    /// Follow-on: `settles(hull)`.
    Settle,
    MapSite,
    Launch,
    ScanPreview,
    ScanGo,
    /// `+tv/1 sound go`, the sounding's ordinal.
    Sound(u8),
    Drive,
    Stake,
    /// Follow-on: the site clerk's record.
    StakeRecord,
    MinePreview,
    MineGo,
    /// Follow-on: the run's cycles.
    Cycles,
    Recover,
    RecoverGo,
    /// Follow-on: the queued unload.
    Unload,
    PlanHome,
    LiftHome,
    /// Follow-on: `docks(hull)`.
    Dock,
    Leave,
    SellPreview,
    SellGo,
}

/// One step: its line (none for a follow-on), its beat, its deadline and its
/// window's start in the buffer.
#[derive(Clone, Debug)]
struct Step {
    kind: Kind,
    beat: Beat,
    line: Option<String>,
    timeout_ms: u64,
    /// The earliest the line may go.
    not_before_ms: u64,
    /// When the line went (or the follow-on opened); `None` while it waits to
    /// be sent.
    opened_ms: Option<u64>,
    /// How many times the line has gone, less the sends the world refused as
    /// transient.
    sends: u32,
    /// How many of its sends the world refused as transient.
    transients: u32,
}

/// One line admitted to the windows, and when it arrived.
#[derive(Clone, Debug)]
struct Got {
    text: String,
    ms: u64,
}

/// What a window says.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    Wait,
    Fail(String),
    /// A closed skip, with the window's text.
    Skip(String),
    /// Send the line again after `wait`, at most `tries` sends in all; the
    /// needle that asked for it.
    Retry {
        needle: &'static str,
        wait: u64,
        tries: u32,
    },
    /// The mine preview's stop answers: mining ends.
    Stop,
    /// The world refused the line as transient: it was never run, so it goes
    /// again after [`TRANSIENT_RETRY_MS`], at most [`TRANSIENT_TRIES`] times.
    Transient,
    Done,
}

/// **The mining pilot's lane**: feed it the session's decoded text, tick it
/// every 50 ms, type what it returns.
pub struct Lane {
    site: Site,
    settings: Settings,
    me: String,
    assembler: Assembler,
    /// The lines admitted since the buffer was last cut: the open step's
    /// window (a send clears it; a follow-on drains it up to the line after
    /// the one that decided the step before).
    buf: Vec<Got>,
    recent: VecDeque<String>,
    last_line_ms: u64,
    last_send_ms: Option<u64>,
    step: Option<Step>,
    beat_open: Option<Beat>,
    beat: Beat,
    reached: BTreeSet<Milestone>,
    events: Vec<LaneEvent>,
    result: Option<LaneResult>,
    report: Report,
    // -- the loop's own state ------------------------------------------------
    exchange_rows: Vec<Listing>,
    chandlery_rows: Vec<Listing>,
    fuel_ask: u64,
    food_ask: u64,
    bunker_rounds: usize,
    buys: VecDeque<(u64, u64)>,
    first_sound_done_ms: Option<u64>,
    pool: Vec<Pocket>,
    used: BTreeSet<String>,
    refused: BTreeSet<char>,
    family: Option<Family>,
    staked: Option<char>,
    target: Option<Pocket>,
    standing_at: Option<String>,
    tried: usize,
    runs: usize,
    cap_t: Option<u64>,
    bid: Option<u64>,
    /// The shortfall being quoted and accepted, and how many quotes of it
    /// went stale.
    buying: Option<(u64, u64)>,
    stale_quotes: u32,
    expedition: Option<detachment::Expedition>,
}

impl Lane {
    /// A lane that plays the loop at `site` as the hull `settings.hull`, the
    /// character called `me`.
    pub fn new(site: Site, settings: Settings, me: String) -> Lane {
        let report = Report {
            site: site.name().to_owned(),
            hull: settings.hull.clone(),
            me: me.clone(),
            epoch_unix_ms: settings.epoch_unix_ms,
            ..Report::default()
        };
        let events = vec![LaneEvent::Said(format!(
            "Mining pilot for {} as the {}.",
            site.name(),
            settings.hull
        ))];
        let mut lane = Lane {
            site,
            settings,
            me,
            assembler: Assembler::default(),
            buf: Vec::new(),
            recent: VecDeque::new(),
            last_line_ms: 0,
            last_send_ms: None,
            step: None,
            beat_open: None,
            beat: Beat::KitUp,
            reached: BTreeSet::new(),
            events,
            result: None,
            report,
            exchange_rows: Vec::new(),
            chandlery_rows: Vec::new(),
            fuel_ask: 0,
            food_ask: 0,
            bunker_rounds: 0,
            buys: VecDeque::new(),
            first_sound_done_ms: None,
            pool: Vec::new(),
            used: BTreeSet::new(),
            refused: BTreeSet::new(),
            family: None,
            staked: None,
            target: None,
            standing_at: None,
            tried: 0,
            runs: 0,
            cap_t: None,
            bid: None,
            buying: None,
            stale_quotes: 0,
            expedition: None,
        };
        lane.walk(Beat::KitUp, "c", ROOM_ARRIVALS, 0);
        lane
    }

    /// Play the hired-crew detachment expedition, retaining the solo travel/trade lane.
    pub fn with_detachment(mut self) -> Self {
        self.expedition = Some(detachment::Expedition::default());
        self.report.detachment = Some(detachment::Evidence::default());
        self
    }

    // -- reading the lane -----------------------------------------------------

    /// Set when the hold sells or a step fails.
    pub fn result(&self) -> Option<&LaneResult> {
        self.result.as_ref()
    }

    /// What happened since the last call, for the runner to log.
    pub fn events(&mut self) -> Vec<LaneEvent> {
        std::mem::take(&mut self.events)
    }

    /// The run's `mine` report block.
    pub fn report(&self) -> Report {
        self.report.clone()
    }

    /// The beat the lane is in.
    pub fn beat(&self) -> Beat {
        self.beat
    }

    /// Whether `milestone` has been reached.
    pub fn reached(&self, milestone: Milestone) -> bool {
        self.reached.contains(&milestone)
    }

    /// The character the lane plays.
    pub fn me(&self) -> &str {
        &self.me
    }

    /// What the runner handed the lane at its start.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The open step's line (sent or waiting to go), `None` while a
    /// follow-on waits for its tell or once the lane has ended.
    pub fn step_line(&self) -> Option<&str> {
        self.step.as_ref().and_then(|step| step.line.as_deref())
    }

    // -- the lines in ---------------------------------------------------------

    /// **Read** decoded terminal text: every complete line into the open
    /// step's window (not the echo of a typed line, not a heard sounding),
    /// every heard line recorded, the bytes counted; then the open step
    /// judged. The journal is every complete line, unfiltered.
    pub fn feed(&mut self, now_ms: u64, text: &str) -> Fed {
        self.report.bytes_received = self
            .report
            .bytes_received
            .saturating_add(u64::try_from(text.len()).unwrap_or(u64::MAX));
        let lines = self.assembler.feed(now_ms, text);
        for line in &lines {
            self.line(now_ms, line);
        }
        self.judge_all(now_ms);
        Fed {
            journal: lines.join("\n"),
        }
    }

    /// One complete line.
    fn line(&mut self, now_ms: u64, raw: &str) {
        self.last_line_ms = now_ms;
        let trimmed = raw.trim();
        if !trimmed.is_empty() && self.recent.iter().any(|sent| sent == trimmed) {
            return;
        }
        if let Some(sentence) = lines::heard_sentence(raw) {
            let sentence = sentence.to_owned();
            self.report.heard.push(HeardAt {
                lane_ms: now_ms,
                at_unix_ms: self.unix(now_ms),
                sentence: sentence.clone(),
                line: raw.to_owned(),
            });
            self.events.push(LaneEvent::Heard {
                sentence,
                lane_ms: now_ms,
            });
            return;
        }
        self.buf.push(Got {
            text: raw.to_owned(),
            ms: now_ms,
        });
    }

    // -- the lines out --------------------------------------------------------

    /// **What to type now.** At most one line, [`LINE_GAP_MS`] after the
    /// last; the stale tail flushed first; deadlines and retries judged.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Send> {
        if let Some(tail) = self.assembler.flush_stale(now_ms) {
            self.line(now_ms, &tail);
        }
        self.judge_all(now_ms);
        if self.result.is_some() {
            return Vec::new();
        }
        let gap_ok = self
            .last_send_ms
            .is_none_or(|last| now_ms >= last.saturating_add(LINE_GAP_MS));
        let Some(step) = self.step.as_mut() else {
            return Vec::new();
        };
        let Some(line) = step.line.clone() else {
            // A follow-on opened when it was made (`follow_on`): nothing to type.
            return Vec::new();
        };
        if step.opened_ms.is_some() || now_ms < step.not_before_ms || !gap_ok {
            return Vec::new();
        }
        step.opened_ms = Some(now_ms);
        step.sends = step.sends.saturating_add(1);
        let beat = step.beat;
        let kind = step.kind.clone();
        // A sending step's window opens at the first line after the send.
        self.buf.clear();
        self.last_send_ms = Some(now_ms);
        self.recent.push_back(line.clone());
        while self.recent.len() > ECHO_LINES {
            self.recent.pop_front();
        }
        self.report.lines_sent = self.report.lines_sent.saturating_add(1);
        self.enter(beat, now_ms);
        if let Kind::Sound(ordinal) = kind
            && self.report.sounding(ordinal).is_none()
        {
            self.report.soundings.push(SoundingSpan {
                ordinal,
                sent_ms: now_ms,
                sent_unix_ms: self.unix(now_ms),
                ..SoundingSpan::default()
            });
        }
        vec![Send {
            line,
            beat,
            lane_ms: now_ms,
        }]
    }

    // -- the machine ----------------------------------------------------------

    /// Judge the open step until nothing more decides (one `feed` may decide
    /// a step and its follow-on).
    fn judge_all(&mut self, now_ms: u64) {
        for _ in 0..64 {
            if self.result.is_some() || !self.judge_once(now_ms) {
                return;
            }
        }
    }

    /// Judge the open step once; whether it decided.
    fn judge_once(&mut self, now_ms: u64) -> bool {
        let Some(step) = self.step.clone() else {
            return false;
        };
        let Some(opened) = step.opened_ms else {
            return false;
        };
        let window: Vec<Got> = self.buf.clone();
        let verdict = self.verdict(&step, opened, &window, now_ms);
        let verdict = match verdict {
            Verdict::Wait if now_ms >= opened.saturating_add(step.timeout_ms) => {
                self.deadline(&step, &window)
            }
            other => other,
        };
        if verdict == Verdict::Wait {
            return false;
        }
        // The line that decided: the shortest prefix that decides (a follow-on
        // opens after it).
        let decided = (0..window.len())
            .find(|&end| self.verdict(&step, opened, &window[..=end], now_ms) != Verdict::Wait)
            .unwrap_or(window.len().saturating_sub(1));
        let decided_at = decided;
        match verdict {
            Verdict::Wait => false,
            Verdict::Fail(outcome) => {
                self.fail(now_ms, outcome);
                true
            }
            Verdict::Retry {
                needle,
                wait,
                tries,
            } => {
                if step.sends >= tries {
                    let line = step.line.clone().unwrap_or_default();
                    self.fail(
                        now_ms,
                        format!(
                            "beat {}: `{line}` still answered `{needle}` after {} tries",
                            step.beat.as_str(),
                            step.sends
                        ),
                    );
                } else if let Some(open) = self.step.as_mut() {
                    open.opened_ms = None;
                    open.not_before_ms = now_ms.saturating_add(wait);
                }
                true
            }
            Verdict::Transient => {
                if step.transients >= TRANSIENT_TRIES {
                    let line = step.line.clone().unwrap_or_default();
                    self.fail(
                        now_ms,
                        format!(
                            "beat {}: `{line}` was refused as transient {} times",
                            step.beat.as_str(),
                            step.transients
                        ),
                    );
                } else if let Some(open) = self.step.as_mut() {
                    // The refused send was never run: it does not count
                    // against the step's own tries.
                    open.transients = open.transients.saturating_add(1);
                    open.sends = open.sends.saturating_sub(1);
                    open.opened_ms = None;
                    open.not_before_ms = now_ms.saturating_add(TRANSIENT_RETRY_MS);
                }
                true
            }
            Verdict::Skip(text) => {
                self.decided(now_ms, &step, Verdict::Skip(text), &window, decided_at);
                true
            }
            other => {
                self.decided(now_ms, &step, other, &window, decided_at);
                true
            }
        }
    }

    /// What `window` says of `step`, in the order fail, skip, retry, done.
    fn verdict(&self, step: &Step, opened: u64, window: &[Got], now_ms: u64) -> Verdict {
        let text = normalise(
            &window
                .iter()
                .map(|got| got.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        let raw: Vec<String> = window.iter().map(|got| got.text.clone()).collect();
        let any = |needles: &[&str]| needles.iter().any(|needle| has(&text, needle));
        let done_if = |yes: bool| if yes { Verdict::Done } else { Verdict::Wait };
        let quiet = window
            .last()
            .is_some_and(|_| now_ms >= self.last_line_ms.saturating_add(QUIET_MS));
        // The time `needle` first arrived in the window.
        let first_at = |needle: &str| {
            (0..window.len()).find_map(|end| {
                let prefix = normalise(
                    &window[..=end]
                        .iter()
                        .map(|got| got.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                has(&prefix, needle).then_some(window[end].ms)
            })
        };
        let settled = |needle: &str| {
            first_at(needle).is_some_and(|at| now_ms >= at.saturating_add(SETTLE_MS))
        };
        let hull = self.settings.hull.as_str();
        // The world's transient refusal of this step's own line comes before
        // every other reading: the line never ran. A follow-on sent nothing,
        // so nothing in its window can be the refusal of its line.
        if step.line.is_some() && window.iter().any(|got| lines::is_transient(&got.text)) {
            return Verdict::Transient;
        }
        match &step.kind {
            Kind::Detachment(task) => detachment::verdict(task, opened, &text, quiet, now_ms),
            Kind::Walk { room, .. } => done_if(has(&text, room)),
            Kind::Enrol => {
                if has(&text, NOT_OPEN) {
                    Verdict::Retry {
                        needle: NOT_OPEN,
                        wait: RETRY_MS,
                        tries: ENROL_TRIES,
                    }
                } else {
                    done_if(has(&text, &format!("now the {hull}")))
                }
            }
            Kind::Refit => done_if(has(&text, REFITTED)),
            Kind::LicencePreview => done_if(has(&text, LICENCE_PREVIEWED)),
            Kind::LicenceGo => done_if(now_ms >= opened.saturating_add(LICENCE_GO_MS)),
            Kind::LicenceCheck => {
                if has(&text, LICENCE_NONE) {
                    Verdict::Retry {
                        needle: LICENCE_NONE,
                        wait: LICENCE_RETRY_MS,
                        tries: LICENCE_TRIES,
                    }
                } else {
                    done_if(has(&text, LICENCE_IN_FORCE))
                }
            }
            Kind::LeasePreview => {
                if has(&text, LEASE_SHORT) {
                    Verdict::Fail(short_lease())
                } else {
                    done_if(has(&text, LEASE_PREVIEWED))
                }
            }
            Kind::LeaseGo => {
                if has(&text, LEASE_SHORT) {
                    Verdict::Fail(short_lease())
                } else if has(&text, LEASE_NO_LICENCE) {
                    Verdict::Retry {
                        needle: LEASE_NO_LICENCE,
                        wait: RETRY_MS,
                        tries: LEASE_TRIES,
                    }
                } else {
                    done_if(has(&text, LEASE_SIGNED))
                }
            }
            Kind::Rigs => done_if(has(&text, RIG_ONE)),
            Kind::Chart => done_if(has(&text, self.site.name())),
            Kind::List(_) => done_if(!lines::listings(&raw).is_empty() && quiet),
            Kind::BunkerPlan | Kind::PlanOut | Kind::LiftOut if has(&text, SITE_CLOSED) => {
                Verdict::Retry {
                    needle: SITE_CLOSED,
                    wait: RETRY_MS,
                    tries: SITE_TRIES,
                }
            }
            Kind::BunkerPlan => done_if(
                has(&text, PLAN_HEAD)
                    && lines::plan_figures(&raw)
                        .is_some_and(|plan| plan.home_burn_t.is_some() && plan.home_days.is_some()),
            ),
            Kind::Quote => done_if(lines::quote_number(&text).is_some()),
            // Another pilot's deal moved the terms: a closed skip, quoted afresh.
            Kind::Accept if has(&text, lines::QUOTE_STALE) => Verdict::Skip(text),
            Kind::Accept => done_if(has(&text, AGREED)),
            Kind::Board => done_if(has(&text, BOARDED)),
            Kind::PlanOut => done_if(has(&text, &format!("{PLAN_HEAD}{}", self.site.name()))),
            Kind::LiftOut => {
                if has(&text, GOODS_WAIT) || has(&text, GOODS_LEAVING) {
                    Verdict::Retry {
                        needle: if has(&text, GOODS_WAIT) {
                            GOODS_WAIT
                        } else {
                            GOODS_LEAVING
                        },
                        wait: GOODS_RETRY_MS,
                        tries: GOODS_TRIES,
                    }
                } else {
                    done_if(has(&text, CLEARANCE))
                }
            }
            Kind::Settle => done_if(has(&text, &lines::settles(hull))),
            Kind::MapSite => {
                if has(&text, COLD) {
                    Verdict::Retry {
                        needle: COLD,
                        wait: RETRY_MS,
                        tries: COLD_TRIES,
                    }
                } else {
                    let legend = window
                        .iter()
                        .find(|got| got.text.starts_with(LEGEND_HEAD))
                        .map(|got| got.ms);
                    done_if(
                        legend.is_some_and(|at| now_ms >= at.saturating_add(SETTLE_MS))
                            && !lines::grid_rows(&raw).is_empty(),
                    )
                }
            }
            Kind::Launch => done_if(has(&text, LAUNCHED)),
            Kind::ScanPreview => {
                if has(&text, COLD) {
                    Verdict::Retry {
                        needle: COLD,
                        wait: RETRY_MS,
                        tries: COLD_TRIES,
                    }
                } else {
                    done_if(has(&text, SCAN_PREVIEWED))
                }
            }
            Kind::ScanGo => done_if(has(&text, SCANNED)),
            Kind::Sound(1) => done_if(settled(SOUNDED)),
            Kind::Sound(_) => {
                if has(&text, LEARN_NOTHING) {
                    Verdict::Skip(text)
                } else if has(&text, SOUNDED) {
                    done_if(settled(SOUNDED))
                } else if now_ms >= opened.saturating_add(ANSWER_MS) && !has(&text, SOUND_STARTED) {
                    // An immediate answer that is not the filed line: closed.
                    Verdict::Skip(text)
                } else {
                    Verdict::Wait
                }
            }
            Kind::Drive => {
                let addr = self.target.as_ref().map_or("", |p| p.addr.as_str());
                done_if(lines::has_addr(&text, ARRIVED, addr) || has(&text, DRIVE_HERE))
            }
            Kind::Stake => {
                if has(&text, STAKE_NO_EVIDENCE) || has(&text, STAKE_IN_BAY) {
                    Verdict::Fail(format!(
                        "the beat order makes this impossible: {}",
                        quote(&text)
                    ))
                } else if has(&text, CLAIM_FUNDS) {
                    Verdict::Fail(
                        "short of funds for the stake at shipped prices (O-23)".to_owned(),
                    )
                } else if has(&text, STAKE_COMMONS) || has(&text, CLAIM_CAP) {
                    Verdict::Fail(format!(
                        "beat {}: the stake was refused: {}",
                        step.beat.as_str(),
                        quote(&text)
                    ))
                } else if any(&[CLAIMED, STAKE_LAPSING, STAKE_COOLING]) {
                    Verdict::Skip(text)
                } else {
                    done_if(any(&[STAKE_FILED, STAKE_YOURS]))
                }
            }
            Kind::StakeRecord => done_if(has(&text, STAKE_RECORDED)),
            Kind::MinePreview => {
                if any(&[CLAIMED, LOCKED, WORKED_OUT, NO_ORE, ALL_BARREN]) {
                    Verdict::Skip(text)
                } else if any(&[NO_ROOM, NO_HOLD_ROOM]) {
                    Verdict::Stop
                } else {
                    done_if(has(&text, WOULD_MINE))
                }
            }
            Kind::MineGo => done_if(has(&text, DRILLING)),
            Kind::Cycles => done_if(lines::last_cycle(&text).is_some_and(|read| read.ended)),
            Kind::Recover => done_if(has(&text, RECOVER_PREVIEWED)),
            Kind::RecoverGo => done_if(has(&text, BACK_IN_BAY)),
            Kind::Unload => {
                if any(&[UNLOADED_EMPTY, UNLOADED_NONE]) {
                    Verdict::Fail(format!(
                        "beat {}: the unload moved nothing: {}",
                        step.beat.as_str(),
                        quote(&text)
                    ))
                } else {
                    done_if(has(&text, UNLOADED))
                }
            }
            Kind::PlanHome => match lines::plan_figures(&raw) {
                Some(plan) if has(&text, PLAN_HEAD) && plan.tank_t < plan.burn_t => Verdict::Fail(
                    format!("short of reaction mass on the pad: {}", figures(&plan)),
                ),
                Some(_) if has(&text, PLAN_HEAD) => Verdict::Done,
                _ => Verdict::Wait,
            },
            Kind::LiftHome => done_if(has(&text, CLEARANCE)),
            Kind::Dock => done_if(has(&text, &lines::docks(hull))),
            Kind::Leave => done_if(has(&text, ASHORE)),
            Kind::SellPreview => done_if(has(&text, SELL_PREVIEW)),
            Kind::SellGo if has(&text, lines::QUOTE_STALE) => Verdict::Retry {
                needle: lines::QUOTE_STALE,
                wait: STALE_RETRY_MS,
                tries: QUOTE_TRIES,
            },
            Kind::SellGo => done_if(lines::sale_receipt(&text).is_some()),
        }
    }

    /// The deadline's verdict: a named failure, or (the second sounding) a
    /// closed skip.
    fn deadline(&self, step: &Step, window: &[Got]) -> Verdict {
        let text = normalise(
            &window
                .iter()
                .map(|got| got.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        match step.kind {
            Kind::Sound(ordinal) if ordinal > 1 => Verdict::Skip(text),
            Kind::StakeRecord => Verdict::Fail(format!(
                "the stake on block {} was not recorded",
                self.target.as_ref().map_or('?', |p| p.block)
            )),
            _ => {
                let what = step.line.clone().unwrap_or_else(|| match step.kind {
                    Kind::Settle => lines::settles(&self.settings.hull),
                    Kind::Dock => lines::docks(&self.settings.hull),
                    Kind::Cycles => "the run's last cycle".to_owned(),
                    Kind::Unload => UNLOADED.to_owned(),
                    _ => "a follow-on".to_owned(),
                });
                Verdict::Fail(format!(
                    "beat {}: no answer to `{what}` within {} s; it read: {}",
                    step.beat.as_str(),
                    step.timeout_ms / 1_000,
                    quote(&text)
                ))
            }
        }
    }

    /// A step decided `verdict` (Done, Skip or Stop): the next step.
    fn decided(
        &mut self,
        now_ms: u64,
        step: &Step,
        verdict: Verdict,
        window: &[Got],
        decided_at: usize,
    ) {
        let raw: Vec<String> = window.iter().map(|got| got.text.clone()).collect();
        let text = normalise(&raw.join(" "));
        let upto = decided_at.saturating_add(1);
        let deciding = normalise(
            &window[..upto.min(window.len())]
                .iter()
                .map(|got| got.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        self.step = None;
        match (&step.kind, verdict) {
            (Kind::Detachment(task), _) => {
                if let Err(error) = self.expedition_next(task.phase, &raw, now_ms) {
                    self.fail(now_ms, format!("detachment {:?}: {error}", task.phase));
                }
            }
            (Kind::LiftOut, _) if self.expedition.is_some() => self.expedition_arrival(false),
            (Kind::LiftHome, _) if self.expedition.is_some() => self.expedition_arrival(true),
            (Kind::LeaseGo, _) if self.expedition.is_some() => self.expedition_start(),
            (Kind::Board, _) if self.expedition.is_some() => self.expedition_board(),
            (Kind::MapSite, _) if self.expedition.is_some() => {
                self.report.witness = lines::grid_rows(&raw);
                self.expedition_launch();
            }
            (Kind::Walk { word, .. }, _) => self.after_walk(now_ms, step.beat, word),
            (Kind::Enrol, _) => self.send(Beat::KitUp, Kind::Refit, "+coop/refit prospector", 0),
            (Kind::Refit, _) => self.send(Beat::KitUp, Kind::LicencePreview, "+licence/buy", 0),
            (Kind::LicencePreview, _) => {
                self.send(Beat::KitUp, Kind::LicenceGo, "+licence/buy go", 0)
            }
            (Kind::LicenceGo, _) => self.send(Beat::KitUp, Kind::LicenceCheck, "+licence", 0),
            (Kind::LicenceCheck, _) => self.walk(Beat::KitUp, "out", ROOM_ARRIVALS, 0),
            (Kind::LeasePreview, _) => self.send(Beat::KitUp, Kind::LeaseGo, "+tv/lease go", 0),
            (Kind::LeaseGo, _) => self.send(Beat::KitUp, Kind::Rigs, "+tv", 0),
            (Kind::Rigs, _) => self.send(Beat::KitUp, Kind::Chart, "+chart", 0),
            (Kind::Chart, _) => {
                self.send(Beat::KitUp, Kind::List(Board::Exchange), "+market/list", 0)
            }
            (Kind::List(board), _) => self.after_list(now_ms, *board, &raw),
            (Kind::BunkerPlan, _) => self.after_bunker_plan(now_ms, &raw),
            (Kind::Quote, _) => match lines::quote_number(&text) {
                Some(quote) => self.send(
                    Beat::Bunkers,
                    Kind::Accept,
                    &format!("+market/accept {quote}"),
                    0,
                ),
                None => self.fail(
                    now_ms,
                    format!("beat bunkers: no quote number in {}", quote(&text)),
                ),
            },
            (Kind::Accept, Verdict::Skip(_)) => self.requote(now_ms),
            (Kind::Accept, _) => self.next_buy(now_ms),
            (Kind::Board, _) => {
                let line = format!("+ship/plan {}", self.site.word());
                self.send(Beat::BoardAndLift, Kind::PlanOut, &line, 0);
            }
            (Kind::PlanOut, _) => {
                let line = format!("+ship/lift {}", self.site.word());
                self.send(Beat::BoardAndLift, Kind::LiftOut, &line, 0);
            }
            (Kind::LiftOut, _) => {
                self.follow_on(now_ms, Beat::BoardAndLift, Kind::Settle, HOP_MS, decided_at)
            }
            (Kind::Settle, _) => {
                self.milestone(Milestone::ArrivedSite, now_ms);
                self.send(Beat::Witness, Kind::MapSite, "+tv/map site", 0);
            }
            (Kind::MapSite, _) => {
                self.report.witness = lines::grid_rows(&raw);
                self.send(Beat::OnPad, Kind::Launch, "+tv/1 launch go", 0);
            }
            (Kind::Launch, _) => {
                self.milestone(Milestone::Launched, now_ms);
                self.send(Beat::OnPad, Kind::ScanPreview, "+tv/1 scan", 0);
            }
            (Kind::ScanPreview, _) => self.send(Beat::OnPad, Kind::ScanGo, "+tv/1 scan go", 0),
            (Kind::ScanGo, _) => {
                self.milestone(Milestone::Scanned, now_ms);
                self.send(Beat::OnPad, Kind::Sound(1), "+tv/1 sound go", 0);
            }
            (Kind::Sound(ordinal), verdict) => {
                self.after_sound(now_ms, *ordinal, verdict, window, &text)
            }
            (Kind::Drive, _) => {
                self.standing_at = self.target.as_ref().map(|p| p.addr.clone());
                self.after_stand();
            }
            (Kind::Stake, Verdict::Skip(_)) => self.stake_refused(now_ms),
            (Kind::Stake, _) => {
                if has(&deciding, STAKE_YOURS) && !has(&deciding, STAKE_FILED) {
                    self.report.receipts.stake = Some(deciding.clone());
                    self.staked = self.target.as_ref().map(|p| p.block);
                    self.milestone(Milestone::Staked, now_ms);
                    self.mine_preview();
                } else {
                    self.follow_on(
                        now_ms,
                        Beat::DriveStakeMine,
                        Kind::StakeRecord,
                        STAKE_RECORD_MS,
                        decided_at,
                    );
                }
            }
            (Kind::StakeRecord, _) => {
                self.report.receipts.stake = Some(deciding.clone());
                self.staked = self.target.as_ref().map(|p| p.block);
                self.milestone(Milestone::Staked, now_ms);
                self.mine_preview();
            }
            (Kind::MinePreview, Verdict::Skip(_)) => {
                if has(&text, CLAIMED)
                    && let Some(block) = self.target.as_ref().map(|p| p.block)
                {
                    self.refused.insert(block);
                }
                if let Some(addr) = self.target.as_ref().map(|p| p.addr.clone()) {
                    self.used.insert(addr);
                }
                if self.tried >= MAX_RUNS {
                    self.go_home(now_ms);
                } else {
                    self.choose(now_ms);
                }
            }
            (Kind::MinePreview, Verdict::Stop) => self.go_home(now_ms),
            (Kind::MinePreview, _) => {
                let line = format!("+tv/1 mine {} go", self.family_word());
                self.send(Beat::DriveStakeMine, Kind::MineGo, &line, 0);
            }
            (Kind::MineGo, _) => self.follow_on(
                now_ms,
                Beat::DriveStakeMine,
                Kind::Cycles,
                RUN_MS,
                decided_at,
            ),
            (Kind::Cycles, _) => self.after_run(now_ms, &text),
            (Kind::Recover, _) => self.send(Beat::Home, Kind::RecoverGo, "+tv/1 recover go", 0),
            (Kind::RecoverGo, _) => {
                self.milestone(Milestone::Recovered, now_ms);
                self.follow_on(now_ms, Beat::Home, Kind::Unload, ACT_MS, decided_at);
            }
            (Kind::Unload, _) => {
                self.milestone(Milestone::Unloaded, now_ms);
                self.report.receipts.unloaded = Some(deciding.clone());
                self.send(Beat::Home, Kind::PlanHome, "+ship/plan ceres", 0);
            }
            (Kind::PlanHome, _) => self.send(Beat::Home, Kind::LiftHome, "+ship/lift ceres", 0),
            (Kind::LiftHome, _) => {
                self.follow_on(now_ms, Beat::Home, Kind::Dock, HOP_MS, decided_at)
            }
            (Kind::Dock, _) => {
                self.milestone(Milestone::DockedHome, now_ms);
                self.send(Beat::Home, Kind::Leave, "+ship/leave", 0);
            }
            (Kind::Leave, _) => self.walk(Beat::Home, "lobby", ROOM_LOBBY, 0),
            (Kind::SellPreview, _) => {
                let line = format!("+market/sell {}=all", self.bid.unwrap_or(0));
                self.send(Beat::Sell, Kind::SellGo, &line, 0);
            }
            (Kind::SellGo, _) => self.sold(now_ms, &text),
        }
        if self.result.is_none() {
            let next = self.step.as_ref().map(|s| s.beat);
            if next != Some(step.beat) {
                self.leave(step.beat, now_ms);
            }
        }
    }

    // -- the beats --------------------------------------------------------------

    /// The walks, each in its beat's order (the SM.8 story's).
    fn after_walk(&mut self, now_ms: u64, beat: Beat, word: &str) {
        match (beat, word) {
            (Beat::KitUp, "c") => self.walk(Beat::KitUp, "registry", ROOM_REGISTRY, 0),
            (Beat::KitUp, "registry") => {
                let line = format!("+coop/enrol {}", self.settings.hull);
                self.send(Beat::KitUp, Kind::Enrol, &line, 0);
            }
            (Beat::KitUp, "out") => self.walk(Beat::KitUp, "concourse", ROOM_CONCOURSE, 0),
            (Beat::KitUp, "concourse") => self.walk(Beat::KitUp, "exchange", ROOM_EXCHANGE, 0),
            (Beat::KitUp, "exchange") => self.send(Beat::KitUp, Kind::LeasePreview, "+tv/lease", 0),
            (Beat::Bunkers, "chandlery") => self.send(
                Beat::Bunkers,
                Kind::List(Board::Chandlery),
                "+market/list",
                0,
            ),
            (Beat::BoardAndLift, "exchange") => {
                self.walk(Beat::BoardAndLift, "concourse", ROOM_CONCOURSE, 0)
            }
            (Beat::BoardAndLift, "concourse") => {
                self.walk(Beat::BoardAndLift, "arrivals", ROOM_ARRIVALS, 0)
            }
            (Beat::BoardAndLift, "arrivals") => {
                self.walk(Beat::BoardAndLift, "lobby", ROOM_LOBBY, 0)
            }
            (Beat::BoardAndLift, "lobby") => {
                self.walk(Beat::BoardAndLift, "gallery", ROOM_GALLERY, 0)
            }
            (Beat::BoardAndLift, "gallery") => {
                self.send(Beat::BoardAndLift, Kind::Board, "+ship/board", 0)
            }
            (Beat::Home, "lobby") => self.walk(Beat::Home, "arrivals", ROOM_ARRIVALS, 0),
            (Beat::Home, "arrivals") => self.walk(Beat::Home, "concourse", ROOM_CONCOURSE, 0),
            (Beat::Home, "concourse") => self.walk(Beat::Home, "exchange", ROOM_EXCHANGE, 0),
            (Beat::Home, "exchange") => {
                self.send(Beat::Sell, Kind::List(Board::Sell), "+market/list", 0)
            }
            _ => self.fail(
                now_ms,
                format!("beat {}: no walk follows `{word}`", beat.as_str()),
            ),
        }
    }

    /// A `+market/list` board read whole.
    fn after_list(&mut self, now_ms: u64, board: Board, raw: &[String]) {
        let rows = lines::listings(raw);
        for row in &rows {
            if !self.report.listings.contains(&row.text) {
                self.report.listings.push(row.text.clone());
            }
        }
        match board {
            Board::Exchange => {
                self.exchange_rows = rows;
                self.walk(Beat::Bunkers, "chandlery", ROOM_CHANDLERY, 0);
            }
            Board::Chandlery => {
                self.chandlery_rows = rows;
                // The chandlery's own rows first: they are the ones quoted here.
                let union: Vec<Listing> = self
                    .chandlery_rows
                    .iter()
                    .chain(self.exchange_rows.iter())
                    .cloned()
                    .collect();
                match (
                    lines::ask_for(&union, "reaction mass"),
                    lines::ask_for(&union, "provisions"),
                ) {
                    (Some(fuel), Some(food)) => {
                        self.fuel_ask = fuel;
                        self.food_ask = food;
                        self.bunker_rounds = 0;
                        self.bunker_plan(0);
                    }
                    _ => self.fail(
                        now_ms,
                        "no reaction mass or provisions listing in `+market/list`".to_owned(),
                    ),
                }
            }
            Board::Sell => {
                let family = self.family.unwrap_or(Family::Metals);
                match lines::bid_row(&rows, family) {
                    Some(row) if !row.not_buying => {
                        self.bid = Some(row.number);
                        let line = format!("+market/sell {}", row.number);
                        self.send(Beat::Sell, Kind::SellPreview, &line, 0);
                    }
                    _ => self.fail(
                        now_ms,
                        format!(
                            "no buying {} bid on the Exchange Hall board",
                            family.ore_label()
                        ),
                    ),
                }
            }
        }
    }

    /// One bunker round's plan, not before `not_before_ms`.
    fn bunker_plan(&mut self, not_before_ms: u64) {
        self.bunker_rounds = self.bunker_rounds.saturating_add(1);
        let line = format!("+ship/plan {}", self.site.word());
        self.send(Beat::Bunkers, Kind::BunkerPlan, &line, not_before_ms);
    }

    /// A round's figures: the beat ends when the tank and the locker cover
    /// the passage out, the leg home and the margins; else the shortfalls are
    /// bought and the next round planned.
    fn after_bunker_plan(&mut self, now_ms: u64, raw: &[String]) {
        let Some(plan) = lines::plan_figures(raw) else {
            self.fail(
                now_ms,
                "beat bunkers: the plan's figures did not read".to_owned(),
            );
            return;
        };
        self.report.plan = Some(plan);
        let want_tank = plan
            .burn_t
            .saturating_add(plan.home_burn_t.unwrap_or(0))
            .saturating_add(EXTRA_BURN_T);
        let want_locker = plan
            .crew_days
            .saturating_add(plan.home_days.unwrap_or(0))
            .saturating_add(
                EXTRA_CREW_DAYS.saturating_mul(if self.expedition.is_some() { 2 } else { 1 }),
            );
        if plan.tank_t >= want_tank && plan.locker >= want_locker {
            self.walk(Beat::BoardAndLift, "exchange", ROOM_EXCHANGE, 0);
            return;
        }
        if self.bunker_rounds >= MAX_BUNKER_ROUNDS {
            self.fail(
                now_ms,
                format!("bunkers short after six rounds: {}", figures(&plan)),
            );
            return;
        }
        self.buys.clear();
        if plan.tank_t < want_tank {
            self.buys
                .push_back((self.fuel_ask, want_tank.saturating_sub(plan.tank_t)));
        }
        if plan.locker < want_locker {
            self.buys
                .push_back((self.food_ask, want_locker.saturating_sub(plan.locker)));
        }
        self.next_buy(now_ms);
    }

    /// The next shortfall's quote, or the next round's plan.
    fn next_buy(&mut self, now_ms: u64) {
        self.buying = self.buys.pop_front();
        match self.buying {
            Some((listing, qty)) => {
                let line = format!("+market/quote {listing}={qty}");
                self.send(Beat::Bunkers, Kind::Quote, &line, 0);
            }
            None => self.bunker_plan(now_ms.saturating_add(BUNKER_ROUND_MS)),
        }
    }

    /// **A stale quote** (another pilot's deal on the listing moved its terms
    /// between the quote and the acceptance): the same shortfall quoted
    /// afresh, at most [`QUOTE_TRIES`] stale quotes in the loop.
    fn requote(&mut self, now_ms: u64) {
        self.stale_quotes = self.stale_quotes.saturating_add(1);
        if self.stale_quotes >= QUOTE_TRIES {
            self.fail(
                now_ms,
                format!(
                    "beat bunkers: `+market/accept` still answered `{}` after {} quotes",
                    lines::QUOTE_STALE,
                    self.stale_quotes
                ),
            );
            return;
        }
        self.events.push(LaneEvent::Said(
            "The quote went stale; quoting it afresh.".to_owned(),
        ));
        let Some((listing, qty)) = self.buying else {
            self.next_buy(now_ms);
            return;
        };
        let line = format!("+market/quote {listing}={qty}");
        self.send(
            Beat::Bunkers,
            Kind::Quote,
            &line,
            now_ms.saturating_add(STALE_RETRY_MS),
        );
    }

    /// A sounding done or (the second) skipped.
    fn after_sound(
        &mut self,
        now_ms: u64,
        ordinal: u8,
        verdict: Verdict,
        window: &[Got],
        text: &str,
    ) {
        let done_ms = window
            .iter()
            .find(|got| got.text.contains(SOUNDED.trim_end()))
            .map_or(now_ms, |got| got.ms);
        let unix = self.unix(done_ms);
        let skipped = match verdict {
            Verdict::Skip(answer) => Some(answer),
            _ => None,
        };
        if let Some(span) = self
            .report
            .soundings
            .iter_mut()
            .find(|span| span.ordinal == ordinal)
        {
            if skipped.is_none() {
                span.done_ms = Some(done_ms);
                span.done_unix_ms = Some(unix);
            }
            span.skipped = skipped.clone();
        }
        if ordinal == 1 {
            self.first_sound_done_ms = Some(done_ms);
            self.milestone(Milestone::Sounded, now_ms);
            let pockets = lines::sounding_pockets(text).unwrap_or_default();
            if pockets.is_empty() {
                self.fail(
                    now_ms,
                    format!(
                        "no metals or volatiles in my sounding at {}",
                        self.site.name()
                    ),
                );
                return;
            }
            let best = lines::ranked(&pockets).first().map(|p| p.family);
            self.family = best;
            self.report.family = best.map(|f| f.word().to_owned());
            self.pool = pockets;
            self.choose(now_ms);
            return;
        }
        match skipped {
            Some(answer) => self.events.push(LaneEvent::Said(format!(
                "The second sounding was not made: {}",
                quote(&answer)
            ))),
            None => {
                self.milestone(Milestone::SecondSounded, now_ms);
                let known: BTreeSet<String> = self.pool.iter().map(|p| p.addr.clone()).collect();
                let base = self.pool.len();
                let fresh: Vec<Pocket> = lines::sounding_pockets(text)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| !known.contains(&p.addr))
                    .enumerate()
                    .map(|(i, p)| Pocket {
                        order: base + i,
                        ..p
                    })
                    .collect();
                self.pool.extend(fresh);
            }
        }
        self.continue_runs(now_ms);
    }

    /// The next target while nothing is mined, or the named failure; drive
    /// to it.
    fn choose(&mut self, now_ms: u64) {
        match self.next_target() {
            Ok(Some(pocket)) => self.drive_to(pocket),
            Ok(None) => self.go_home(now_ms),
            Err(outcome) => self.fail(now_ms, outcome),
        }
    }

    /// **The next target** (the brief's D11, D20) over the pool: every pocket
    /// of the lane's soundings not yet used, whose block is not refused,
    /// ranked. While nothing is mined: the best of the family in the staked
    /// block, else in any block (mining past the claim), else the family
    /// switched once, else the named failure. Once a run has mined: the best
    /// of the family in the staked block (mining open ground: any block), or
    /// `None` (home).
    fn next_target(&mut self) -> Result<Option<Pocket>, String> {
        let Some(mut family) = self.family else {
            return Err(format!(
                "no unclaimed pocket of my readings at {}",
                self.site.name()
            ));
        };
        if self.report.mined_t > 0 {
            let block = if self.report.mined_open_ground {
                None
            } else {
                self.staked
            };
            return Ok(self.best(family, block));
        }
        for attempt in 0..2 {
            if let Some(block) = self.staked {
                if let Some(pocket) = self.best(family, Some(block)) {
                    return Ok(Some(pocket));
                }
                if let Some(pocket) = self.best(family, None) {
                    self.events.push(LaneEvent::Said(format!(
                        "mining past my claim: no {} pocket left in block {block}",
                        family.word()
                    )));
                    return Ok(Some(pocket));
                }
            } else if let Some(pocket) = self.best(family, None) {
                return Ok(Some(pocket));
            }
            if attempt > 0 || self.report.family_switched {
                break;
            }
            let old = family;
            family = old.other();
            self.family = Some(family);
            self.report.family = Some(family.word().to_owned());
            self.report.family_switched = true;
            self.events.push(LaneEvent::Said(format!(
                "switched to {}: no unclaimed {} pocket in my readings at {}",
                family.word(),
                old.word(),
                self.site.name()
            )));
        }
        Err(format!(
            "no unclaimed pocket of my readings at {}",
            self.site.name()
        ))
    }

    /// The best unused pocket of `family` outside the refused blocks, in
    /// `block` when one is named.
    fn best(&self, family: Family, block: Option<char>) -> Option<Pocket> {
        lines::ranked(&self.pool).into_iter().find(|pocket| {
            pocket.family == family
                && !self.used.contains(&pocket.addr)
                && !self.refused.contains(&pocket.block)
                && block.is_none_or(|b| pocket.block == b)
        })
    }

    /// Drive to `pocket`, or (standing there already) go straight on.
    fn drive_to(&mut self, pocket: Pocket) {
        let here = self.standing_at.as_deref() == Some(pocket.addr.as_str());
        let line = format!("+tv/1 drive {} go", pocket.addr);
        self.target = Some(pocket);
        if here {
            self.after_stand();
        } else {
            self.send(Beat::DriveStakeMine, Kind::Drive, &line, 0);
        }
    }

    /// Standing on the target: the stake (once, while not staked and not
    /// mining open ground), else the mine's preview.
    fn after_stand(&mut self) {
        if self.staked.is_none() && !self.report.mined_open_ground {
            self.send(Beat::DriveStakeMine, Kind::Stake, "+claim/stake 1 go", 0);
        } else {
            self.mine_preview();
        }
    }

    /// A refused stake: the block joins the refused set; the next block, or
    /// open ground at the third.
    fn stake_refused(&mut self, now_ms: u64) {
        if let Some(block) = self.target.as_ref().map(|p| p.block) {
            self.refused.insert(block);
        }
        self.report.stakes_refused = self.report.stakes_refused.saturating_add(1);
        if self.report.stakes_refused >= MAX_REFUSED_STAKES {
            self.report.mined_open_ground = true;
            self.events.push(LaneEvent::Said(format!(
                "mined open ground: {} stakes refused at {}",
                self.report.stakes_refused,
                self.site.name()
            )));
        }
        self.choose(now_ms);
    }

    /// `+tv/1 mine {family}`: one more target tried.
    fn mine_preview(&mut self) {
        self.tried = self.tried.saturating_add(1);
        let line = format!("+tv/1 mine {}", self.family_word());
        self.send(Beat::DriveStakeMine, Kind::MinePreview, &line, 0);
    }

    /// A run's last cycle read.
    fn after_run(&mut self, now_ms: u64, text: &str) {
        if text.contains(lines::CYCLE_HEAD)
            && let Some(read) = lines::last_cycle(text)
        {
            self.report.mined_t = read.hopper_t;
            if read.cap_t.is_some() {
                self.cap_t = read.cap_t;
            }
        }
        if let Some(addr) = self.target.as_ref().map(|p| p.addr.clone()) {
            self.used.insert(addr);
        }
        self.runs = self.runs.saturating_add(1);
        if self.runs == 1 {
            self.milestone(Milestone::FirstRunDone, now_ms);
            let after = self
                .first_sound_done_ms
                .unwrap_or(now_ms)
                .saturating_add(SECOND_SOUNDING_AFTER_MS);
            self.send(
                Beat::SecondSounding,
                Kind::Sound(2),
                "+tv/1 sound go",
                after,
            );
            return;
        }
        self.continue_runs(now_ms);
    }

    /// The run loop after a run or the second sounding: on while the hopper
    /// has room and fewer than [`MAX_RUNS`] targets were tried.
    fn continue_runs(&mut self, now_ms: u64) {
        let full = self.cap_t.is_some_and(|cap| self.report.mined_t >= cap);
        if full || self.tried >= MAX_RUNS {
            self.go_home(now_ms);
        } else {
            self.choose(now_ms);
        }
    }

    /// The `Home` beat: nothing mined is a failure; else the recover.
    fn go_home(&mut self, now_ms: u64) {
        if self.report.mined_t == 0 {
            self.fail(now_ms, format!("nothing mined in {MAX_RUNS} runs"));
            return;
        }
        self.send(Beat::Home, Kind::Recover, "+tv/1 recover", 0);
    }

    /// The sale's receipt: the loop is done.
    fn sold(&mut self, now_ms: u64, text: &str) {
        let Some(sale) = lines::sale_receipt(text) else {
            self.fail(now_ms, "beat sell: the sale's line did not read".to_owned());
            return;
        };
        self.milestone(Milestone::Sold, now_ms);
        self.report.sold_t = sale.qty_t;
        let net = sale.net.clone();
        self.report.receipts.sale = Some(sale);
        self.report.success = true;
        self.leave(Beat::Sell, now_ms);
        self.beat = Beat::Done;
        let outcome = format!(
            "mined {} t of {} at {} and sold {} t: {net}",
            self.report.mined_t,
            self.family_word(),
            self.site.name(),
            self.report.sold_t
        );
        self.events.push(LaneEvent::Said(outcome.clone()));
        self.result = Some(LaneResult {
            status: "done".to_owned(),
            outcome,
        });
    }

    // -- the plumbing ----------------------------------------------------------

    fn family_word(&self) -> &'static str {
        self.family.map_or("metals", Family::word)
    }

    /// Queue a walk: `word` → `room`.
    fn walk(&mut self, beat: Beat, word: &'static str, room: &'static str, not_before_ms: u64) {
        self.send(beat, Kind::Walk { word, room }, word, not_before_ms);
    }

    /// Queue a step that sends `line`.
    fn send(&mut self, beat: Beat, kind: Kind, line: &str, not_before_ms: u64) {
        let timeout_ms = match kind {
            Kind::Launch
            | Kind::ScanGo
            | Kind::Sound(_)
            | Kind::Drive
            | Kind::RecoverGo
            | Kind::Unload => ACT_MS,
            _ => ANSWER_MS,
        };
        self.step = Some(Step {
            kind,
            beat,
            line: Some(line.to_owned()),
            timeout_ms,
            not_before_ms,
            opened_ms: None,
            sends: 0,
            transients: 0,
        });
    }

    /// Open a follow-on step at once, its window starting at the line after
    /// `decided_at` (the brief's D21): lines already received after it are
    /// carried into it.
    fn follow_on(
        &mut self,
        now_ms: u64,
        beat: Beat,
        kind: Kind,
        timeout_ms: u64,
        decided_at: usize,
    ) {
        let start = decided_at.saturating_add(1).min(self.buf.len());
        self.buf.drain(..start);
        self.step = Some(Step {
            kind,
            beat,
            line: None,
            timeout_ms,
            not_before_ms: now_ms,
            opened_ms: Some(now_ms),
            sends: 0,
            transients: 0,
        });
    }

    /// A beat's first line went.
    fn enter(&mut self, beat: Beat, now_ms: u64) {
        self.beat = beat;
        if self.beat_open == Some(beat) {
            return;
        }
        self.beat_open = Some(beat);
        self.report.beats.push(BeatSpan {
            beat,
            start_ms: now_ms,
            end_ms: None,
            start_unix_ms: self.unix(now_ms),
            end_unix_ms: None,
        });
        self.events.push(LaneEvent::Beat {
            beat,
            edge: Edge::Start,
            lane_ms: now_ms,
        });
    }

    /// A beat's last answer was read.
    fn leave(&mut self, beat: Beat, now_ms: u64) {
        if self.beat_open != Some(beat) {
            return;
        }
        self.beat_open = None;
        let unix = self.unix(now_ms);
        if let Some(span) = self
            .report
            .beats
            .iter_mut()
            .rev()
            .find(|span| span.beat == beat && span.end_ms.is_none())
        {
            span.end_ms = Some(now_ms);
            span.end_unix_ms = Some(unix);
        }
        self.events.push(LaneEvent::Beat {
            beat,
            edge: Edge::End,
            lane_ms: now_ms,
        });
    }

    /// A milestone, once.
    fn milestone(&mut self, name: Milestone, now_ms: u64) {
        if !self.reached.insert(name) {
            return;
        }
        self.report.milestones.push(MilestoneAt {
            name,
            lane_ms: now_ms,
            at_unix_ms: self.unix(now_ms),
        });
        self.events.push(LaneEvent::Milestone {
            name,
            lane_ms: now_ms,
        });
    }

    /// The lane blocks with `outcome`.
    fn fail(&mut self, _now_ms: u64, outcome: String) {
        self.step = None;
        self.report.failure = Some(outcome.clone());
        self.events.push(LaneEvent::Said(outcome.clone()));
        self.result = Some(LaneResult {
            status: "blocked".to_owned(),
            outcome,
        });
    }

    /// `lane_ms` as unix milliseconds.
    fn unix(&self, lane_ms: u64) -> u64 {
        self.settings.epoch_unix_ms.saturating_add(lane_ms)
    }
}

/// The lease's named failure.
fn short_lease() -> String {
    "short of funds for the lease at shipped prices (O-23)".to_owned()
}

/// A plan's figures as a failure quotes them.
fn figures(plan: &PlanFigures) -> String {
    format!(
        "burn {} t, tank {} t, {} crew-days, locker {}, leg home {:?} t and {:?} crew-days",
        plan.burn_t, plan.tank_t, plan.crew_days, plan.locker, plan.home_burn_t, plan.home_days
    )
}

/// A window's last [`QUOTE_BYTES`] bytes.
fn quote(text: &str) -> String {
    crate::tail(text, QUOTE_BYTES).to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use tacsim::port::{claims, licence, proceeds, site_map, tv, voyage};

    const HULL: &str = "Ardent Lode";
    const EPOCH: u64 = 1_900_000_000_000;

    /// What a stake is answered with.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum StakeAnswer {
        Record,
        Claimed,
        NoEvidence,
    }

    /// How the second `+tv/1 sound go` is answered when it is made.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SecondTell {
        /// The filed line and the `SOUNDED` tell in one feed.
        Now,
        /// The filed line now, the tell [`SOUND_TAKES_MS`] later (as live).
        Late,
        /// An answer no needle lists, and no filed line.
        Refused,
    }

    /// A live sounding's span at rate 24 (1,200 IC s): longer than
    /// [`ANSWER_MS`], so the second sounding must wait past its immediate
    /// answer on the filed line.
    const SOUND_TAKES_MS: u64 = 50_000;
    const _: () = assert!(SOUND_TAKES_MS > ANSWER_MS);

    /// **The scripted server's choices**: the pad sounding's family lines, the
    /// second sounding's (`None`: "You learn nothing new here.") and when its
    /// tell comes, the stakes' answers in order (the last repeats), the
    /// pockets whose mine preview is worked out, the tonnes each run lifts,
    /// how long after a drive's filed line its `ARRIVED` comes (0: the same
    /// feed), and the lines the world refuses once as transient.
    #[derive(Clone, Debug)]
    struct Script {
        first: Vec<&'static str>,
        second: Option<Vec<&'static str>>,
        second_tell: SecondTell,
        stakes: Vec<StakeAnswer>,
        worked_out: BTreeSet<&'static str>,
        run_t: u64,
        arrive_after_ms: u64,
        busy: BTreeSet<&'static str>,
    }

    impl Script {
        /// Test 2's: one rich metals pocket in block F, fair ones in F and G;
        /// one run fills the hopper; the licence purchase and the launch are
        /// each refused once as transient.
        fn story() -> Script {
            Script {
                first: vec!["  metals: F12 fair, F7 rich, G5 fair"],
                second: Some(vec!["  metals: F12 fair, F30 lean"]),
                second_tell: SecondTell::Now,
                stakes: vec![StakeAnswer::Record],
                worked_out: BTreeSet::new(),
                run_t: 12,
                arrive_after_ms: 0,
                busy: ["+licence/buy go", "+tv/1 launch go"].into_iter().collect(),
            }
        }
    }

    /// **A scripted server** answering each line from the game's own `pub`
    /// constants, every tell of an act in the same text as its filed line, so
    /// one `feed` carries both.
    struct Server {
        script: Script,
        room: &'static str,
        tank: u64,
        locker: u64,
        quotes: BTreeMap<u64, (u64, u64)>,
        next_quote: u64,
        stakes: usize,
        sounds: usize,
        hopper: u64,
        at: String,
        site_open: bool,
        /// The first acceptance and the first sale answer that the terms moved.
        stale_accept: bool,
        stale_sale: bool,
        /// The script's `busy` lines already refused once.
        busied: BTreeSet<String>,
        /// The clock the lane's line was answered at, and the tells still to
        /// come: `(due, text)`, delivered by `play` as their own feed.
        now: u64,
        later: Vec<(u64, String)>,
        /// Every deferred tell as it was delivered: `(at, text)`.
        delivered: Vec<(u64, String)>,
    }

    fn lines(parts: &[String]) -> String {
        let mut text = parts.join("\r\n");
        text.push_str("\r\n");
        text
    }

    fn legend() -> Vec<String> {
        let cut = site_map::SITE_LEGEND
            .match_indices("  ")
            .map(|(at, _)| at)
            .filter(|at| site_map::SITE_LEGEND[..*at].chars().count() <= site_map::MAX_LINE_COLUMNS)
            .last()
            .unwrap_or(0);
        vec![
            site_map::SITE_LEGEND[..cut].to_owned(),
            site_map::SITE_LEGEND[cut..].to_owned(),
        ]
    }

    impl Server {
        fn new(script: Script) -> Server {
            Server {
                script,
                room: "Limbo",
                tank: 0,
                locker: 0,
                quotes: BTreeMap::new(),
                next_quote: 40,
                stakes: 0,
                sounds: 0,
                hopper: 0,
                at: "K1".to_owned(),
                site_open: false,
                stale_accept: true,
                stale_sale: true,
                busied: BTreeSet::new(),
                now: 0,
                later: Vec::new(),
                delivered: Vec::new(),
            }
        }

        /// The deferred tells due by `now`, in order, recorded as delivered.
        fn due(&mut self, now: u64) -> Vec<String> {
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.later)
                .into_iter()
                .partition(|(at, _)| *at <= now);
            self.later = later;
            for (_, text) in &due {
                self.delivered.push((now, text.clone()));
            }
            due.into_iter().map(|(_, text)| text).collect()
        }

        /// A tell `after_ms` from now: in this answer when 0, else deferred.
        fn tell(&mut self, after_ms: u64, text: String) -> String {
            if after_ms == 0 {
                text
            } else {
                self.later.push((self.now.saturating_add(after_ms), text));
                String::new()
            }
        }

        fn room(&mut self, room: &'static str) -> String {
            self.room = room;
            lines(&[room.to_owned(), "You see nothing special.".to_owned()])
        }

        fn answer(&mut self, line: &str) -> String {
            let n1 = |text: &str| text.replace("{n}", "1");
            // The world's transient refusal (mush-server's `RefusalReason`),
            // once a line: the line never ran.
            if self.script.busy.contains(line) && self.busied.insert(line.to_owned()) {
                return lines(&[
                    "The world was busy too long for that line; please retry in a moment."
                        .to_owned(),
                ]);
            }
            match line {
                "c" | "out" | "arrivals" => self.room(lines::ROOM_ARRIVALS),
                "registry" => self.room(lines::ROOM_REGISTRY),
                "concourse" => self.room(lines::ROOM_CONCOURSE),
                "exchange" => self.room(lines::ROOM_EXCHANGE),
                "chandlery" => self.room(lines::ROOM_CHANDLERY),
                "lobby" => self.room(lines::ROOM_LOBBY),
                "gallery" => self.room(lines::ROOM_GALLERY),
                "+coop/refit prospector" => lines(&[tacsim::port::refit::REFITTED_PROSPECTOR
                    .replace("{name}", HULL)
                    .replace("{mark}", "DRAY-07")]),
                "+licence/buy" => lines(&[licence::LICENCE_PREVIEW
                    .replace("{issuer}", "Freeport Registry")
                    .replace("{fee}", "2500.00cr")]),
                "+licence/buy go" => String::new(),
                "+licence" => lines(&[licence::LICENCE_IN_FORCE
                    .replace("{issuer}", "Freeport Registry")
                    .replace("{account}", "3")]),
                "+tv/lease" => lines(&[n1(tv::LEASE_PREVIEW).replace("{name}", HULL)]),
                "+tv/lease go" => lines(&[n1(tv::LEASED).replace("{name}", HULL)]),
                "+tv" => lines(&[n1(tv::RIG_LINE).replace("{where}", "in the bay")]),
                "+chart" => lines(&[
                    "Ahuna Flank (Ceres): metals fair; open.".to_owned(),
                    "Kerwan Floor (Ceres): volatiles lean; open.".to_owned(),
                ]),
                "+market/list" => {
                    let mut rows = vec![
                        "#3 reaction mass — 500 × 40.00cr [exchange] at Occator Water & Propellant",
                        "#4 voyage provisions — 200 × 10.00cr [exchange] at Ceres Co-op",
                    ];
                    if self.room == lines::ROOM_EXCHANGE {
                        rows.push("#7 metal ore, run-of-mine — wanted 240 × 70.00cr [exchange] by Freeport Exchange");
                        rows.push("#8 ice ore, run-of-mine — wanted 240 × 35.00cr [exchange] by Occator Water & Propellant");
                    }
                    lines(&rows.into_iter().map(str::to_owned).collect::<Vec<_>>())
                }
                // The site's pad not open yet, once: the plan is retried.
                "+ship/plan ahuna" if !self.site_open => {
                    self.site_open = true;
                    lines(&[
                        "Plan for Ahuna Flank:".to_owned(),
                        voyage::SITE_CLOSED.replace("{site}", "Ahuna Flank"),
                        voyage::NOTHING_WRITTEN.to_owned(),
                    ])
                }
                "+ship/plan ahuna" => lines(&[
                    "Plan for Ahuna Flank:".to_owned(),
                    format!("Burn 3 t (tank {} t; …). Flip and burn.", self.tank),
                    format!("Stores 1 crew-days (locker {}; …).", self.locker),
                    voyage::SITE_HOME
                        .replace("{port}", "Ahuna Flank")
                        .replace("{burn}", "2")
                        .replace("{days}", "1")
                        .replace("{throttle}", "0.5"),
                    voyage::NOTHING_WRITTEN.to_owned(),
                ]),
                "+ship/board" => lines(&[format!("You cycle the lock into the {HULL}.")]),
                "+ship/lift ahuna" => lines(&[
                    voyage::CLEARANCE_FILED.replace("{port}", "Ahuna Flank"),
                    voyage::LIFT_SEALS.replace("{name}", HULL),
                    voyage::SETTLES
                        .replace("{name}", "Cobalt Lode")
                        .replace("{port}", "Ahuna Flank"),
                    voyage::SETTLES
                        .replace("{name}", HULL)
                        .replace("{port}", "Ahuna Flank"),
                ]),
                "+tv/map site" => {
                    let mut print = vec![
                        site_map::MAP_TITLE_SITE
                            .replace("{site}", "Ahuna Flank")
                            .replace("{family}", "metals"),
                    ];
                    print.extend(legend());
                    for row in 0..site_map::SITE_VIEW_CELLS {
                        print.push(if row == 16 {
                            format!("{}HH{}", ",,".repeat(15), ",,".repeat(16))
                        } else {
                            ",^".repeat(site_map::SITE_VIEW_CELLS)
                        });
                    }
                    print.push(site_map::MAP_NO_READINGS.replace("{family}", "metals"));
                    lines(&print)
                }
                "+tv/1 launch go" => lines(&[
                    n1(tv::LAUNCH_FILED),
                    n1(tv::LAUNCHED)
                        .replace("{ramp}", "stern ramp")
                        .replace("{site}", "Ahuna Flank"),
                ]),
                "+tv/1 scan" => lines(&[n1(tv::SCAN_PREVIEW)]),
                "+tv/1 scan go" => lines(&[
                    n1(tv::SCAN_FILED),
                    // Another party's sounding, heard: recorded, never acted on.
                    tv::HEARD
                        .replace("{bearing}", "north")
                        .replace("{band}", tv::BAND_NEAR),
                    n1(tv::SCANNED)
                        .replace("{count}", "9")
                        .replace("{footing}", "rock")
                        .replace("{hidden}", ""),
                    "  K-NW fair metals".to_owned(),
                ]),
                "+tv/1 sound go" => {
                    self.sounds += 1;
                    let families = if self.sounds == 1 {
                        Some(self.script.first.clone())
                    } else {
                        self.script.second.clone()
                    };
                    let tell = if self.sounds == 1 {
                        SecondTell::Now
                    } else {
                        self.script.second_tell
                    };
                    match (families, tell) {
                        (_, SecondTell::Refused) => lines(&[tv::TV_DARK.replace("{name}", HULL)]),
                        (None, _) => lines(&[tv::LEARN_NOTHING.to_owned()]),
                        (Some(families), tell) => {
                            let mut done = vec![
                                n1(tv::SOUNDED)
                                    .replace("{count}", "64")
                                    .replace("{addr}", &self.at)
                                    .replace("{quietly}", ""),
                            ];
                            done.extend(families.into_iter().map(str::to_owned));
                            let filed = lines(&[n1(tv::SOUND_FILED).replace("{when}", "in 50 s")]);
                            let after = if tell == SecondTell::Late {
                                SOUND_TAKES_MS
                            } else {
                                0
                            };
                            let done = self.tell(after, lines(&done));
                            filed + done.as_str()
                        }
                    }
                }
                "+claim/stake 1 go" => {
                    let answer = self
                        .script
                        .stakes
                        .get(self.stakes)
                        .or(self.script.stakes.last())
                        .copied()
                        .unwrap_or(StakeAnswer::Record);
                    self.stakes += 1;
                    let block = &self.at[..1];
                    match answer {
                        StakeAnswer::Record => lines(&[
                            claims::STAKE_FILED
                                .replace("{block}", block)
                                .replace("{site}", "Ahuna Flank")
                                .replace("{total}", "4000.00cr"),
                            claims::STAKE_RECORDED
                                .replace("{issuer}", "Freeport Registry")
                                .replace("{block}", block)
                                .replace("{site}", "Ahuna Flank")
                                .replace("{until}", "2350-09-01"),
                        ]),
                        StakeAnswer::Claimed => lines(&[tv::MINE_CLAIMED.to_owned()]),
                        StakeAnswer::NoEvidence => {
                            lines(&[claims::STAKE_NO_EVIDENCE.replace("{block}", block)])
                        }
                    }
                }
                "+tv/1 recover" => lines(&[n1(tv::RECOVER_PREVIEW)]),
                "+tv/1 recover go" => {
                    let load = format!("{} t of metal ore, run-of-mine", self.hopper);
                    self.hopper = 0;
                    lines(&[
                        n1(tv::RECOVER_FILED),
                        n1(tv::RECOVERED_UNLOADING).replace("{name}", HULL),
                        n1(tv::UNLOADED)
                            .replace("{load}", &load)
                            .replace("{name}", HULL)
                            .replace("{hopper}", "empty"),
                    ])
                }
                "+ship/plan ceres" => lines(&[
                    "Plan for Ceres Freeport:".to_owned(),
                    "Burn 2 t (tank 3 t; 1 t left). Flip and burn.".to_owned(),
                    "Stores 2 crew-days (locker 3; 1 left).".to_owned(),
                ]),
                "+ship/lift ceres" => lines(&[
                    voyage::CLEARANCE_FILED.replace("{port}", "Ceres Freeport"),
                    voyage::DOCKS
                        .replace("{name}", HULL)
                        .replace("{port}", "Ceres Freeport"),
                ]),
                "+ship/leave" => {
                    self.room = lines::ROOM_GALLERY;
                    lines(&["You cycle the lock out into the Visiting Berth Gallery.".to_owned()])
                }
                other => self.answer_more(other),
            }
        }

        fn answer_more(&mut self, line: &str) -> String {
            let n1 = |text: &str| text.replace("{n}", "1");
            if let Some(name) = line.strip_prefix("+coop/enrol ") {
                return lines(&[format!(
                    "The Co-op charters you DRAY-07, laid up as the Dray 7, now the {name}, \
                     cradled at the Visiting Berth Gallery."
                )]);
            }
            if let Some(rest) = line.strip_prefix("+market/quote ") {
                let (listing, qty) = rest.split_once('=').unwrap_or(("0", "0"));
                let quote = self.next_quote;
                self.next_quote += 1;
                self.quotes.insert(
                    quote,
                    (listing.parse().unwrap_or(0), qty.parse().unwrap_or(0)),
                );
                return lines(&[format!(
                    "Quote #{quote}: {qty} × 40.00cr = 1.00cr — good until …"
                )]);
            }
            let stale = "That quote no longer stands — the terms moved or it lapsed. Ask for a \
                         new one; nothing was charged.";
            if line.starts_with("+market/accept ") && self.stale_accept {
                self.stale_accept = false;
                return lines(&[stale.to_owned()]);
            }
            if line.ends_with("=all") && self.stale_sale {
                self.stale_sale = false;
                return lines(&[stale.to_owned()]);
            }
            if let Some(quote) = line.strip_prefix("+market/accept ") {
                let (listing, qty) = self
                    .quotes
                    .get(&quote.parse().unwrap_or(0))
                    .copied()
                    .unwrap_or((0, 0));
                match listing {
                    3 => self.tank += qty,
                    4 => self.locker += qty,
                    _ => {}
                }
                return lines(&["Agreed. Contract #9 carries it.".to_owned()]);
            }
            if let Some(addr) = line
                .strip_prefix("+tv/1 drive ")
                .and_then(|rest| rest.strip_suffix(" go"))
            {
                self.at = addr.to_owned();
                let filed = lines(&[n1(tv::DRIVE_FILED).replace("{target}", addr)]);
                let arrived = lines(&[n1(tv::ARRIVED)
                    .replace("{addr}", addr)
                    .replace("{reading}", ", fair metals")]);
                let arrived = self.tell(self.script.arrive_after_ms, arrived);
                return filed + arrived.as_str();
            }
            if let Some(rest) = line.strip_prefix("+tv/1 mine ") {
                let addr = self.at.clone();
                if let Some(family) = rest.strip_suffix(" go") {
                    let ore = if family == "metals" {
                        "metal ore, run-of-mine"
                    } else {
                        "ice ore, run-of-mine"
                    };
                    let cycles = self.script.run_t.div_ceil(2).max(1);
                    let mut out = vec![n1(tv::MINE_FILED).replace("{addr}", &addr)];
                    for k in 1..=cycles {
                        self.hopper += 2;
                        let mut cycle = n1(tv::CYCLE_DONE)
                            .replace("{k}", &k.to_string())
                            .replace("{cycles}", &cycles.to_string())
                            .replace("{t}", "2")
                            .replace("{ore}", ore)
                            .replace("{hopper}", &format!("{} of 12 t", self.hopper));
                        if k == cycles {
                            cycle.push_str(&tv::RUN_DONE.replace("{addr}", &addr));
                        }
                        out.push(cycle);
                    }
                    return lines(&out);
                }
                if self.script.worked_out.contains(addr.as_str()) {
                    return lines(&[tv::MINE_WORKED_OUT
                        .replace("{addr}", &addr)
                        .replace("{ore}", "metal ore, run-of-mine")]);
                }
                return lines(&[n1(tv::MINE_PREVIEW)
                    .replace("{addr}", &addr)
                    .replace("{class}", "fair")]);
            }
            if let Some(rest) = line.strip_prefix("+market/sell ") {
                if let Some(bid) = rest.strip_suffix("=all") {
                    let label = if bid == "8" {
                        "ice ore, run-of-mine"
                    } else {
                        "metal ore, run-of-mine"
                    };
                    return lines(&[proceeds::SALE_LINE
                        .replace("{qty}", &self.hopper_sold())
                        .replace("{unit}", "t")
                        .replace("{label}", label)
                        .replace("{house}", "Freeport Exchange")
                        .replace("{price}", "70.00cr")
                        .replace("{gross}", "840.00cr")
                        .replace("{share}", "84.00cr")
                        .replace("{net}", "756.00cr")
                        .replace("{signatory}", "The Co-op")]);
                }
                return lines(&[
                    format!(
                        "#{rest} metal ore: Freeport Exchange wants 240 more at 70.00cr each; you hold 12 there."
                    ),
                    format!("+market/sell {rest}=all sells 12 for 840.00cr."),
                ]);
            }
            lines(&["Huh?  (Type \"help\" for help.)".to_owned()])
        }

        fn hopper_sold(&self) -> String {
            "12".to_owned()
        }
    }

    /// **Play the loop against `script`**: a stepped clock (100 ms a tick),
    /// every line the lane types echoed and answered at once, each deferred
    /// tell fed on the first tick it is due.
    fn play(script: Script) -> (Lane, Vec<Send>) {
        let (lane, sent, _) = play_with(script);
        (lane, sent)
    }

    /// [`play`], and the server it played against (its delivered tells).
    fn play_with(script: Script) -> (Lane, Vec<Send>, Server) {
        let mut lane = Lane::new(
            Site::Ahuna,
            Settings {
                hull: HULL.to_owned(),
                epoch_unix_ms: EPOCH,
            },
            "Ardent".to_owned(),
        );
        let mut server = Server::new(script);
        let mut sent = Vec::new();
        lane.feed(0, "Connected as Ardent (#12).\r\nLimbo\r\n");
        let mut now = 0;
        while now < 3 * 3_600_000 && lane.result().is_none() {
            now += 100;
            for text in server.due(now) {
                lane.feed(now, &text);
            }
            for send in lane.tick(now) {
                server.now = now;
                let mut text = format!("{}\r\n", send.line);
                text.push_str(&server.answer(&send.line));
                sent.push(send);
                lane.feed(now, &text);
            }
        }
        (lane, sent, server)
    }

    fn at(sent: &[Send], line: &str) -> Option<usize> {
        sent.iter().position(|send| send.line == line)
    }

    fn lines_of(sent: &[Send]) -> Vec<&str> {
        sent.iter().map(|send| send.line.as_str()).collect()
    }

    /// **The loop in its closed vocabulary** (doc 20 cut SM.12c-bot, B6 test
    /// 2): a scripted server drives the lane to `done`; three acts answer
    /// their filed line and their tell in one `feed` (the drive, the stake,
    /// the recover and its unload), so a follow-on whose window started late
    /// would time out.
    #[test]
    fn mine_lane_types_only_its_vocabulary() {
        let (mut lane, sent) = play(Script::story());
        let report = lane.report();
        assert_eq!(
            lane.result().map(|r| r.status.as_str()),
            Some("done"),
            "{:?}\n{:#?}",
            lane.result(),
            lines_of(&sent)
        );
        assert!(report.success);
        for send in &sent {
            assert!(in_vocabulary(&send.line), "{}", send.line);
            assert!(!send.line.starts_with('@'));
        }
        for pair in sent.windows(2) {
            assert!(
                pair[1].lane_ms >= pair[0].lane_ms + LINE_GAP_MS,
                "{} then {}",
                pair[0].line,
                pair[1].line
            );
        }
        // The closed pad retried, two bunker rounds, and the plan aboard.
        let plans: Vec<u64> = sent
            .iter()
            .filter(|send| send.line == "+ship/plan ahuna")
            .map(|send| send.lane_ms)
            .collect();
        assert_eq!(plans.len(), 4, "{:#?}", lines_of(&sent));
        assert!(plans[1] >= plans[0] + RETRY_MS);
        // A stale quote quoted afresh; a stale sale sent again.
        let fuel_quotes = sent
            .iter()
            .filter(|send| send.line == "+market/quote 3=6")
            .count();
        assert_eq!(fuel_quotes, 2, "{:#?}", lines_of(&sent));
        let sales = sent
            .iter()
            .filter(|send| send.line == "+market/sell 7=all")
            .count();
        assert_eq!(sales, 2, "{:#?}", lines_of(&sent));
        // The world's transient refusals, typed again a moment later: the
        // licence purchase (a timed step) and the launch.
        for busy in ["+licence/buy go", "+tv/1 launch go"] {
            let times: Vec<u64> = sent
                .iter()
                .filter(|send| send.line == busy)
                .map(|send| send.lane_ms)
                .collect();
            assert_eq!(times.len(), 2, "{busy}: {:#?}", lines_of(&sent));
            assert!(
                times[1] >= times[0] + TRANSIENT_RETRY_MS,
                "{busy}: {times:?}"
            );
        }
        let map = at(&sent, "+tv/map site").expect("the witness map");
        let launch = at(&sent, "+tv/1 launch go").expect("the launch");
        assert!(map < launch);
        let drive = at(&sent, "+tv/1 drive F7 go").expect("the best pocket first");
        let stake = at(&sent, "+claim/stake 1 go").expect("the stake");
        assert!(drive < stake);
        let sounds: Vec<usize> = sent
            .iter()
            .enumerate()
            .filter(|(_, send)| send.line == "+tv/1 sound go")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(sounds.len(), 2, "{:#?}", lines_of(&sent));
        let mine_go = at(&sent, "+tv/1 mine metals go").expect("the run");
        let recover = at(&sent, "+tv/1 recover").expect("the recover");
        assert!(mine_go < sounds[1] && sounds[1] < recover);
        let first = report.sounding(1).expect("the pad's sounding");
        let second = report.sounding(2).expect("the second sounding");
        assert!(second.sent_ms >= first.done_ms.unwrap_or(u64::MAX) + SECOND_SOUNDING_AFTER_MS);
        assert!(second.done_ms.is_some() && second.skipped.is_none());
        let names: Vec<Milestone> = report.milestones.iter().map(|m| m.name).collect();
        assert_eq!(
            names,
            vec![
                Milestone::ArrivedSite,
                Milestone::Launched,
                Milestone::Scanned,
                Milestone::Sounded,
                Milestone::Staked,
                Milestone::FirstRunDone,
                Milestone::SecondSounded,
                Milestone::Recovered,
                Milestone::Unloaded,
                Milestone::DockedHome,
                Milestone::Sold,
            ]
        );
        for m in &report.milestones {
            assert_eq!(m.at_unix_ms, EPOCH + m.lane_ms);
        }
        // The receipts, the report's figures and the witness print.
        assert!(
            report
                .receipts
                .stake
                .as_deref()
                .is_some_and(|line| line.contains("records your claim on block F"))
        );
        assert!(
            report
                .receipts
                .unloaded
                .as_deref()
                .is_some_and(|line| line.starts_with("Rig 1 unloads 12 t"))
        );
        assert_eq!(report.mined_t, 12);
        assert_eq!(report.sold_t, 12);
        assert_eq!(report.family.as_deref(), Some("metals"));
        assert!(!report.family_switched);
        assert_eq!(report.witness.len(), lines::SITE_VIEW_CELLS);
        assert_eq!(report.heard.len(), 1, "the heard line recorded");
        assert!(lines::heard_line_is_bare(&report.heard[0].line));
        let plan = report.plan.expect("the bunker plan");
        assert_eq!((plan.home_burn_t, plan.home_days), (Some(2), Some(1)));
        assert!(
            plan.tank_t >= 3 + 2 + EXTRA_BURN_T && plan.locker >= 1 + 1 + EXTRA_CREW_DAYS,
            "{plan:?}"
        );
        let rows = lines::listings(&report.listings);
        assert_eq!(lines::bid_for(&rows, Family::Metals), Some(7));
        assert_eq!(report.lines_sent, sent.len() as u64);
        assert!(at(&sent, "+market/sell 7=all").is_some());
        // The beats: each began once, in order, and every one but `Done` ended.
        let beats: Vec<Beat> = report.beats.iter().map(|span| span.beat).collect();
        assert_eq!(
            beats,
            vec![
                Beat::KitUp,
                Beat::Bunkers,
                Beat::BoardAndLift,
                Beat::Witness,
                Beat::OnPad,
                Beat::DriveStakeMine,
                Beat::SecondSounding,
                Beat::Home,
                Beat::Sell,
            ]
        );
        assert!(report.beats.iter().all(|span| span.end_ms.is_some()));
        assert_eq!(lane.beat(), Beat::Done);
        assert!(
            lane.events()
                .iter()
                .any(|e| matches!(e, LaneEvent::Heard { .. }))
        );

        // The tells as they come live: the drive's `ARRIVED` three seconds
        // after its filed line, the second sounding's `SOUNDED` fifty seconds
        // after its filed line (past `ANSWER_MS`). The stake goes only once
        // the rig has arrived, and nothing goes in between; the second
        // sounding waits on its filed line for its tell.
        let (lane, sent, server) = play_with(Script {
            arrive_after_ms: 3_000,
            second_tell: SecondTell::Late,
            ..Script::story()
        });
        assert_eq!(
            lane.result().map(|r| r.status.as_str()),
            Some("done"),
            "{:?}",
            lane.result()
        );
        let arrived_at = server
            .delivered
            .iter()
            .find(|(_, text)| text.contains("Rig 1 arrives at F7"))
            .map(|(at, _)| *at)
            .expect("the arrival was delivered late");
        let drive = at(&sent, "+tv/1 drive F7 go").expect("the drive");
        assert!(arrived_at >= sent[drive].lane_ms + 3_000);
        assert_eq!(sent[drive + 1].line, "+claim/stake 1 go");
        assert!(
            sent[drive + 1].lane_ms >= arrived_at,
            "staked at {} before the arrival at {arrived_at}",
            sent[drive + 1].lane_ms
        );
        let report = lane.report();
        let second = report.sounding(2).expect("the second sounding");
        assert!(second.skipped.is_none(), "{second:?}");
        assert!(
            second.done_ms.unwrap_or(0) >= second.sent_ms + SOUND_TAKES_MS,
            "{second:?}"
        );
        assert!(lane.reached(Milestone::SecondSounded));

        // The second sounding answered with a line no needle lists and no
        // filed line: a closed skip carrying that answer, never a failure.
        let (lane, sent) = play(Script {
            second_tell: SecondTell::Refused,
            ..Script::story()
        });
        assert_eq!(
            lane.result().map(|r| r.status.as_str()),
            Some("done"),
            "{:?}",
            lane.result()
        );
        let report = lane.report();
        let second = report.sounding(2).expect("the second sounding");
        assert!(
            second
                .skipped
                .as_deref()
                .is_some_and(|text| text.contains("rigs cannot be read right now")),
            "{second:?}"
        );
        assert!(!lane.reached(Milestone::SecondSounded));
        let sounds: Vec<&Send> = sent
            .iter()
            .filter(|send| send.line == "+tv/1 sound go")
            .collect();
        let recover = at(&sent, "+tv/1 recover").expect("the recover");
        assert!(
            sent[recover].lane_ms < sounds[1].lane_ms + ACT_MS,
            "skipped at its immediate answer, not at the act's deadline"
        );
    }

    /// **The next block** (doc 20 cut SM.12c-bot, B6 test 3): a refused stake
    /// drives to a pocket of another block of the lane's own reading; three
    /// refusals and the lane mines open ground; a stake with no evidence is a
    /// named failure; the empty candidate set (D20) switches the family once,
    /// fails by name, or goes home.
    #[test]
    fn mine_lane_drives_to_the_next_block_when_one_is_claimed() {
        // The first stake refused: the next drive is in another block.
        let (lane, sent) = play(Script {
            stakes: vec![StakeAnswer::Claimed, StakeAnswer::Record],
            ..Script::story()
        });
        assert_eq!(lane.result().map(|r| r.status.as_str()), Some("done"));
        let drives: Vec<&str> = lines_of(&sent)
            .into_iter()
            .filter(|line| line.starts_with("+tv/1 drive "))
            .collect();
        assert_eq!(drives[..2], ["+tv/1 drive F7 go", "+tv/1 drive G5 go"]);
        let stakes = sent
            .iter()
            .filter(|s| s.line == "+claim/stake 1 go")
            .count();
        assert_eq!(stakes, 2);
        let report = lane.report();
        assert_eq!(report.stakes_refused, 1);
        assert!(!report.mined_open_ground);
        assert!(
            report
                .receipts
                .stake
                .as_deref()
                .is_some_and(|line| line.contains("block G"))
        );

        // Every stake refused: three stakes, then open ground outside them.
        let (mut lane, sent) = play(Script {
            first: vec!["  metals: F7 rich, G5 fair, J9 fair, K2 lean"],
            stakes: vec![StakeAnswer::Claimed],
            ..Script::story()
        });
        assert_eq!(
            lane.result().map(|r| r.status.as_str()),
            Some("done"),
            "{:?}",
            lane.result()
        );
        let stakes = sent
            .iter()
            .filter(|s| s.line == "+claim/stake 1 go")
            .count();
        assert_eq!(stakes, MAX_REFUSED_STAKES as usize);
        let last_stake = sent
            .iter()
            .rposition(|s| s.line == "+claim/stake 1 go")
            .unwrap_or(0);
        assert_eq!(
            sent[last_stake + 1].line,
            "+tv/1 drive K2 go",
            "mined outside the refused blocks"
        );
        let report = lane.report();
        assert!(report.mined_open_ground);
        assert_eq!(report.stakes_refused, 3);
        assert!(report.receipts.stake.is_none());
        assert!(lane.events().iter().any(|e| matches!(
            e,
            LaneEvent::Said(text) if text.contains("mined open ground")
        )));

        // No evidence: a named failure.
        let (lane, _) = play(Script {
            stakes: vec![StakeAnswer::NoEvidence],
            ..Script::story()
        });
        let result = lane.result().expect("a result");
        assert_eq!(result.status, "blocked");
        assert!(
            result
                .outcome
                .contains("the beat order makes this impossible"),
            "{}",
            result.outcome
        );

        // (a) Metals all in F, volatiles in G: the family switches once and
        // the sale asks the ice-ore bid.
        let (mut lane, sent) = play(Script {
            first: vec!["  metals: F7 rich, F12 fair", "  volatiles: G5 fair"],
            stakes: vec![StakeAnswer::Claimed, StakeAnswer::Record],
            ..Script::story()
        });
        assert_eq!(
            lane.result().map(|r| r.status.as_str()),
            Some("done"),
            "{:?}",
            lane.result()
        );
        let report = lane.report();
        assert!(report.family_switched);
        assert_eq!(report.family.as_deref(), Some("volatiles"));
        assert!(at(&sent, "+tv/1 drive G5 go").is_some());
        assert!(at(&sent, "+tv/1 mine volatiles go").is_some());
        assert!(
            at(&sent, "+market/sell 8=all").is_some(),
            "{:#?}",
            lines_of(&sent)
        );
        assert!(lane.events().iter().any(|e| matches!(
            e,
            LaneEvent::Said(text) if text.starts_with("switched to volatiles")
        )));

        // (b) Every pocket of both families in F, and F refused: the named failure.
        let (lane, _) = play(Script {
            first: vec!["  metals: F7 rich, F12 fair", "  volatiles: F30 fair"],
            stakes: vec![StakeAnswer::Claimed],
            ..Script::story()
        });
        let result = lane.result().expect("a result");
        assert_eq!(result.status, "blocked");
        assert!(
            result
                .outcome
                .contains("no unclaimed pocket of my readings at Ahuna Flank"),
            "{}",
            result.outcome
        );

        // (c) The staked block worked out after a first run mined: home, no failure.
        let (lane, sent) = play(Script {
            first: vec!["  metals: F7 rich, F12 fair, F13 fair, G5 fair"],
            second: None,
            worked_out: ["F12", "F13"].into_iter().collect(),
            run_t: 6,
            ..Script::story()
        });
        assert_eq!(
            lane.result().map(|r| r.status.as_str()),
            Some("done"),
            "{:?}",
            lane.result()
        );
        assert!(
            at(&sent, "+tv/1 drive G5 go").is_none(),
            "never past the claim once mined"
        );
        // After the first run, the second sounding, then every other pocket
        // of the staked block tried in rank order (each previewed, none
        // committed), and only then home.
        let mine_go = at(&sent, "+tv/1 mine metals go").expect("the first run");
        let recover = at(&sent, "+tv/1 recover").expect("the recover");
        assert_eq!(
            lines_of(&sent)[mine_go + 1..=recover],
            [
                "+tv/1 sound go",
                "+tv/1 drive F12 go",
                "+tv/1 mine metals",
                "+tv/1 drive F13 go",
                "+tv/1 mine metals",
                "+tv/1 recover",
            ],
            "{:#?}",
            lines_of(&sent)
        );
        let report = lane.report();
        assert_eq!(report.mined_t, 6);
        assert!(
            report
                .sounding(2)
                .is_some_and(|span| span.skipped.as_deref() == Some(LEARN_NOTHING))
        );
        // Nothing mined and no pocket of either family left: the named failure.
        let (lane, _) = play(Script {
            first: vec!["  metals: F7 rich, F12 fair"],
            worked_out: ["F7", "F12"].into_iter().collect(),
            ..Script::story()
        });
        assert!(
            lane.result().is_some_and(|r| r.status == "blocked"
                && r.outcome
                    .contains("no unclaimed pocket of my readings at Ahuna Flank")),
            "{:?}",
            lane.result()
        );
        // Four skips in a row with pockets still in the pool: home with nothing.
        let (lane, _) = play(Script {
            first: vec!["  metals: F7 rich, F12 fair, F13 fair, F14 fair, F15 fair"],
            worked_out: ["F7", "F12", "F13", "F14", "F15"].into_iter().collect(),
            ..Script::story()
        });
        assert!(
            lane.result().is_some_and(|r| r.status == "blocked"
                && r.outcome == format!("nothing mined in {MAX_RUNS} runs")),
            "{:?}",
            lane.result()
        );
    }

    /// The vocabulary admits exactly its lines.
    #[test]
    fn the_vocabulary_is_closed() {
        for line in [
            "c",
            "+coop/enrol Ardent Lode",
            "+market/quote 3=6",
            "+market/accept 41",
            "+ship/plan kerwan",
            "+ship/lift ceres",
            "+tv/1 drive P256 go",
            "+tv/1 mine volatiles go",
            "+market/sell 7=all",
        ] {
            assert!(in_vocabulary(line), "{line}");
        }
        for line in [
            "@tacsim/port status",
            "+coop/enrol 9lives",
            "+market/quote 3",
            "+ship/plan vesta",
            "+tv/1 drive Q1 go",
            "+tv/1 drive F1234 go",
            "+tv/1 mine carbon",
            "+tv/2 scan go",
            "+market/sell 7=12",
            "say hello",
            "c ",
        ] {
            assert!(!in_vocabulary(line), "{line}");
        }
    }
}
