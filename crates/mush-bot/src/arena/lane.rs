//! The lane (cut AJ.2): the pilot's one channel to the game, and everything
//! that decides what goes down it and when.
//!
//! [`Lane::feed`] takes the decoded terminal text, splits it into lines
//! ([`Assembler`]), reads each ([`classify`]) into the [`Picture`] and the
//! [`Lobby`], and hands back the **journal**: the lines worth the bot's memory
//! and attention. [`Lane::tick`] (the runner calls it every 50 ms) says which
//! lines to type now. Pure: no I/O and no clock — `now_ms` is the runner's
//! session clock, and the runner does the reading and the writing.
//!
//! **The limiter.** At most `[arena].max_lines_per_s` lines (4, 16c Q-AJ4) in
//! any 1 000 ms of the lane's own clock, every purpose together: a line may go
//! when the oldest of the last N went at least 1 000 ms ago. Each [`Send`]
//! carries that stamp (`lane_ms`) and the runner writes it into the transcript's
//! `arena-send` record, so the limiter is checked on the clock it ran on.
//!
//! **What goes first** when a slot is free (cut AL.1): the lobby's line; the
//! match's first `STATUS` and first `WEAPONSTATUS` (straight after `Power
//! online.`); the doubt's `WEAPONSTATUS` (once, at once, when a lost section
//! said `its weapons are gone`); **every gun the pilot fires, main gun first**,
//! each on its own cycle and never while its own last `FIRE` is unanswered;
//! `LOCK`; `CONTACTS` when due (never deferred: a gun fires only on a row read
//! inside two seconds); the post-shot `WEAPONSTATUS`; `HEADING`; `SPEED`; the
//! lock's `RANGE` (never deferred: it is good only inside its 250 ms window,
//! and it refines the lock's fix); and then **the polls that can wait** —
//! `STATUS`, the periodic `WEAPONSTATUS`, `BEARING B` — each only when it
//! leaves a slot free for every gun due inside the next second
//! (`Lane::reserve_ok`), or once it is [`POLL_STARVE_MS`] overdue. One line
//! per free slot per tick. (The bot's own transcripts: 212 of 347 machine-gun
//! `FIRE`s went with three lines already in the second before, the gun sitting
//! behind `CONTACTS`; the schedule's slack, not refusals, held its rate.)
//!
//! **The polls** (a line-mode pilot is sent no periodic picture, and a frame
//! changes at most once a pulse — 500, 1 000 or 5 000 ms engaged, on alert,
//! quiet): `CONTACTS` every 1 000 ms while any contact is held, every 5 000 ms
//! otherwise, and once 250 ms after any contact notice; while locked, a bare
//! `RANGE` straight after every second `CONTACTS` reply commits, so it lands
//! inside the picture's 400 ms and refines that row's fix ("a fresh one reads
//! to 10 m", `help cockpit`); `STATUS` every 2 000 ms; `WEAPONSTATUS`
//! [`WEAPONS_AFTER_SHOT_MS`] after each shot of a gun whose cycle is not yet
//! learned (inside the first second after the ack, where a row prints the whole
//! cycle), then every 3 000 ms while locked and every 5 000 ms otherwise — and
//! every [`WEAPONS_LEARNED_MS`] once every gun carried has its cycle and none
//! reads `0%` (while locked, each gun's chance read since the last lock line;
//! the rounds come down by acks between rows); `BEARING B` every
//! 5 000 ms while the pilot searches; never the same poll twice inside 500 ms
//! (but the doubt's `WEAPONSTATUS`); the first `STATUS` and `WEAPONSTATUS`
//! straight after `Power online.`.
//!
//! **Orders are closed-loop.** The pilot re-decides every tick. A `HEADING`,
//! `SPEED` or `LOCK` not acknowledged (`Steering to`, `Making`, `Locking`)
//! within 1 500 ms is forgotten and so re-sent when re-decided; a `FIRE`
//! refused with `Too many orders at once; try that again.` waits 1 000 ms. No
//! pipelined reply is ever matched to the line that asked for it; each `FIRE`
//! is marked with the weapon it names ([`Picture::sent_fire`]), so the
//! picture tells the cockpit's pre-check of that gun from the arena's refusal
//! of another's shot. What the pilot says of what it does (a hunt, a fall
//! back, a held tank turned further; [`Pilot::take_notes`]) goes to the runner
//! as [`LaneEvent::Said`] every tick.
//!
//! **The journal filter.** The lane remembers the last 32 lines it typed; a line
//! equal to one of them is the terminal's echo and is not read at all, and
//! every line [`Picture::apply`] marks `journal: false` (read replies, acks,
//! transients, `OUTSIDE_COCKPIT`) is left out of [`Fed::journal`]. The runner's
//! transcript still records the raw text whole.
//!
//! **The commander** (cut AJ.3; [`super::commander`]). The lane owns its
//! [`Schedule`]; the runner owns the call. [`Lane::commander_ready`] says an
//! ask would be due (the runner checks the budget then),
//! [`Lane::commander_due`] hands the runner the picture to send when an ask is
//! due and none is in flight,
//! [`Lane::commander_reply`] takes what came back (a valid doctrine is flown
//! from the next tick), and [`Lane::commander_off`] is the runner's budget
//! stop. The lane feeds the schedule its triggers from the lines it reads (a
//! contact identified to the other side, an own section destroyed) and the
//! lock from the picture, begins it at each match's `Power online.`, and stops
//! it at the match's end, when the tank is destroyed and when the pilot stops.
//! The stale bound is twice the `CONTACTS` period in force at the ask — the
//! poll that refreshes a line-mode pilot's picture, which is never refreshed by
//! the arena's pulse — and an ask still in flight past it is given up on the
//! next feed or tick ([`Schedule::expire`], cut AK.2), so the runner aborts it.
//! What the schedule records reaches the runner as [`LaneEvent::Record`].
//!
//! **The pilot's rule on a `withdraw`** (cut AK.2; the commander's module head
//! has the smoke's figures). Every applied doctrine goes through
//! [`super::commander::vet`]: a `withdraw` on a sound hull, or in the rest
//! after a hold, is flown as `hunt` with no fresh hostile held, else as the
//! pilot's own engagement (cut AL.3: `[arena] engagement`'s stance, `hold` by
//! default; it was always `orbit`), recorded `commander-overruled`. A kept
//! `withdraw` is one hold per episode: the episode's first sets
//! `doctrine_until_ms` to `hold_s` on, a later one while it runs replaces the
//! doctrine but never moves it; when it passes a standing `withdraw` becomes
//! the pilot's default (`doctrine-lapsed`, `"the hold ran out"`) and a rest of
//! [`super::commander::HOLD_MAX_S`] begins (`rest_until_ms`). Any stop of the
//! commander clears a standing `withdraw` the same way (`"the commander
//! stopped"`); every other stance survives a stop.
use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;
use serde_json::{Value, json};

use super::{
    Assembler, Side, classify,
    commander::{
        CommanderReply, Doctrine, HOLD_DEFAULT_S, HOLD_MAX_S, LAPSED_COMMANDER_STOPPED,
        LAPSED_HOLD_RAN_OUT, OFF_DESTROYED, OFF_MATCH_OVER, OFF_STOPPED, Schedule, Stance, Vetted,
        vet,
    },
    lines::Line,
    lobby::{Lobby, Step},
    picture::{self, Phase, Picture, Power},
    pilot::{HuntLeg, Orders, Pilot, Turn, live_turn},
    report::Report,
    settings::Settings,
};

/// The lines the lane remembers typing, for the echo.
pub const RECENT_SENT: usize = 32;
/// The window the limiter counts lines in.
pub const LIMIT_WINDOW_MS: u64 = 1_000;
/// `CONTACTS` this often while a contact is held…
pub const CONTACTS_HELD_MS: u64 = 1_000;
/// …and this often while none is.
pub const CONTACTS_EMPTY_MS: u64 = 5_000;
/// `CONTACTS` this long after a contact notice.
pub const CONTACTS_AFTER_NOTICE_MS: u64 = 250;
/// The lock's `RANGE` must go this soon after its `CONTACTS` reply commits.
pub const RANGE_WINDOW_MS: u64 = 250;
/// `STATUS` this often.
pub const STATUS_EVERY_MS: u64 = 2_000;
/// **`WEAPONSTATUS` this long after a shot, until the weapon's cycle is
/// learned** (cut AL.1): inside the first second after the ack, where a
/// `cycling` row prints the whole cycle, and after the frame the shot leaves in
/// (the bot's own rows 250 to 500 ms after a machine-gun ack still read `ready`
/// 11 times of 49; rows 500 to 1 000 ms after read the whole cycle every time).
/// Re-armed at every ack of a gun whose cycle is unknown. Read from the
/// transcript, not tunable.
pub const WEAPONS_AFTER_SHOT_MS: u64 = 500;
const _: () = assert!(WEAPONS_AFTER_SHOT_MS < picture::CYCLE_READ_WITHIN_MS);
/// `WEAPONSTATUS` this often while locked…
pub const WEAPONS_LOCKED_MS: u64 = 3_000;
/// …and this often otherwise…
pub const WEAPONS_IDLE_MS: u64 = 5_000;
/// …and this often, locked or not, once every gun carried has its cycle and
/// none reads `0%` (cut AL.1): the clock needs no row then, and the rounds come
/// down by acks. A gun at `0%` is left unfired until a row says otherwise, so
/// it keeps the faster cadence; so does a lane that has read no gun, and,
/// while locked, one whose chance was read before the last lock line (it is
/// of no lock, or of the lock before). AL.3 may tune it within 5 000..=10 000
/// by the brief, but the lane's tests admit only values above
/// `WEAPONS_LOCKED_MS + POLL_STARVE_MS + LIMIT_WINDOW_MS` (6 000): see the
/// tuning list in [`super::pilot`].
pub const WEAPONS_LEARNED_MS: u64 = 8_000;
/// **A poll that can wait goes anyway once it is this overdue** (cut AL.1), so
/// the reservation for the guns never holds `STATUS`, the periodic
/// `WEAPONSTATUS` or `BEARING B` back for more than this past its due time.
/// It lifts only the reservation: everything ahead of these polls in
/// [`Lane::tick`]'s order still goes first, so a one-line-a-second lane that
/// holds a contact (whose `CONTACTS`, due every second and never deferred,
/// takes each free slot no gun does) may not reach them at all. Not tunable.
pub const POLL_STARVE_MS: u64 = 2_000;
/// `BEARING B` this often while searching.
pub const BEARING_EVERY_MS: u64 = 5_000;
/// Never the same poll twice inside this.
pub const POLL_GAP_MS: u64 = 500;
/// An order unanswered this long is re-sent when re-decided.
pub const ACK_WAIT_MS: u64 = 1_500;
/// At most one `FIRE` per weapon this often (cut AL.1: a lattice step, so a
/// refused `FIRE` is tried again inside its second; the gun's own clock, and
/// its last `FIRE` answered first, govern the rest). Not tunable.
pub const FIRE_GAP_MS: u64 = 250;
/// A `FIRE` refused as too many orders waits this long.
pub const FIRE_BUSY_WAIT_MS: u64 = 1_000;

/// Why a line went out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// `look`, `ooc`, `simulator`, a team room.
    Walk,
    /// A centre verb: `SIM …`, `BOARD`, `STARTUP`.
    Lobby,
    /// A cockpit read: `CONTACTS`, `STATUS`, `WEAPONSTATUS`, `RANGE`, `BEARING B`.
    Poll,
    /// A cockpit order: `HEADING`, `SPEED`, `LOCK`, `FIRE`.
    Order,
}

/// One line to type, and the lane's clock when the limiter let it go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Send {
    pub line: String,
    pub purpose: Purpose,
    pub lane_ms: u64,
}

/// What [`Lane::feed`] hands back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fed {
    /// The lines worth the bot's memory and attention, newline-joined, in order.
    pub journal: String,
}

/// The `arena-state` record: written on every phase, lead or aboard change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StateRecord {
    pub phase: String,
    #[serde(rename = "match")]
    pub match_id: Option<u64>,
    pub callsign: Option<String>,
    pub lead: bool,
    pub aboard: bool,
}

/// What the runner logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaneEvent {
    /// A sentence for the `arena` event.
    Said(String),
    /// The `arena-state` record.
    State(StateRecord),
    /// The pilot claimed a slot: the run's time limit starts over.
    Claimed,
    /// A transcript record (cut AJ.3's commander): `arena-doctrine`,
    /// `commander-stale`, `commander-invalid`, `commander-error`,
    /// `commander-off`, and (cut AK.2) `commander-overruled` and
    /// `doctrine-lapsed`, and its data.
    Record { kind: String, data: Value },
}

/// How the lane ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LaneResult {
    /// `done` (every match won) or `blocked`.
    pub status: String,
    pub outcome: String,
}

/// The fight's own bookkeeping: when each poll last went, what is due, which
/// orders wait for their ack.
#[derive(Clone, Debug, Default)]
struct Fight {
    last_contacts: Option<u64>,
    last_status: Option<u64>,
    last_weapons: Option<u64>,
    last_range: Option<u64>,
    last_bearing: Option<u64>,
    contacts_soon: Option<u64>,
    contacts_open: bool,
    contacts_commits: u32,
    range_until: Option<u64>,
    /// The post-shot `WEAPONSTATUS`, per gun (cut AL.1): each gun whose cycle
    /// is not yet known, as its ack named it, and when its read is due (its
    /// ack and [`WEAPONS_AFTER_SHOT_MS`]). Kept per gun so that one gun's ack
    /// never moves another's read out of that gun's first second, nor brings
    /// it forward into the frame before its shot left.
    weapons_after_shot: BTreeMap<String, u64>,
    heading_ack: Option<u64>,
    speed_ack: Option<u64>,
    lock_ack: Option<u64>,
    last_fire: BTreeMap<String, u64>,
    fire_hold: BTreeMap<String, u64>,
}

/// A poll may go: `last` is at least `every` ago (or never) and at least the
/// poll gap.
fn poll_due(last: Option<u64>, every: u64, now_ms: u64) -> bool {
    last.is_none_or(|at| now_ms >= at.saturating_add(every.max(POLL_GAP_MS)))
}

fn gap_ok(last: Option<u64>, now_ms: u64) -> bool {
    last.is_none_or(|at| now_ms >= at.saturating_add(POLL_GAP_MS))
}

/// The phase as one word.
pub fn phase_word(phase: &Phase) -> &'static str {
    match phase {
        Phase::Unknown => "unknown",
        Phase::Walking => "walking",
        Phase::InCentre => "in_centre",
        Phase::Claimed { .. } => "claimed",
        Phase::Boarding { .. } => "boarding",
        Phase::Seated { .. } => "seated",
        Phase::Running { .. } => "running",
        Phase::Over { .. } => "over",
    }
}

/// The main gun: the cannon when carried, else the first weapon.
fn main_gun(picture: &Picture) -> Option<&str> {
    picture
        .weapons
        .iter()
        .find(|w| w.id == "cannon")
        .or_else(|| picture.weapons.first())
        .map(|w| w.id.as_str())
}

/// **The lane.**
#[derive(Debug)]
pub struct Lane {
    side: Side,
    settings: Settings,
    assembler: Assembler,
    picture: Picture,
    lobby: Lobby,
    pilot: Pilot,
    /// The match the pilot's state belongs to.
    pilot_match: Option<u64>,
    doctrine: Doctrine,
    /// The end of the running `withdraw` episode (cut AK.2): set by its first
    /// kept `withdraw`, never moved by a later one.
    doctrine_until_ms: Option<u64>,
    /// No `withdraw` is kept before then: the rest after a hold.
    rest_until_ms: u64,
    /// When the commander asks, and what it has done this match.
    commander: Schedule,
    /// When each of the last `max_lines_per_s` lines went.
    ring: VecDeque<u64>,
    /// The last lines typed, for the echo.
    recent: VecDeque<String>,
    events: Vec<LaneEvent>,
    /// Journal lines read by a tick (a stale tail), handed out by the next feed.
    held_journal: Vec<String>,
    fight: Option<Fight>,
    last_state: Option<StateRecord>,
    result: Option<LaneResult>,
    stopped: bool,
}

impl Lane {
    /// A lane flying for `side`, `settings.matches` matches, the character
    /// called `me` as best the runner knows.
    pub fn new(side: Side, settings: Settings, me: String) -> Lane {
        let events = vec![LaneEvent::Said(format!(
            "Arena pilot flying for {}; {} match(es).",
            side.as_str(),
            settings.matches
        ))];
        Lane {
            side,
            lobby: Lobby::new(side, settings.clone(), &me),
            doctrine: Doctrine::pilot_default(&settings),
            doctrine_until_ms: None,
            rest_until_ms: 0,
            commander: Schedule::new(&settings),
            settings,
            assembler: Assembler::default(),
            picture: Picture::default(),
            pilot: Pilot::new(),
            pilot_match: None,
            ring: VecDeque::new(),
            recent: VecDeque::new(),
            events,
            held_journal: Vec::new(),
            fight: None,
            last_state: None,
            result: None,
            stopped: false,
        }
    }

    // -- reading --------------------------------------------------------------

    pub fn picture(&self) -> &Picture {
        &self.picture
    }

    pub fn lobby(&self) -> &Lobby {
        &self.lobby
    }

    pub fn side(&self) -> Side {
        self.side
    }

    /// The doctrine the pilot flies (AJ.3's commander replaces it).
    pub fn doctrine(&self) -> &Doctrine {
        &self.doctrine
    }

    pub fn set_doctrine(&mut self, doctrine: Doctrine) {
        self.doctrine = doctrine;
    }

    /// Set when the last match ends or a step blocks; never after [`Lane::stop`].
    pub fn result(&self) -> Option<&LaneResult> {
        self.result.as_ref()
    }

    /// [`Lane::stop`] was called.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Still flying: neither finished nor stopped.
    pub fn flying(&self) -> bool {
        !self.lobby.closed()
    }

    /// `SIM LEAVE` is queued, or typed and its answer not yet in (at most 3 s).
    pub fn leave_pending(&self, now_ms: u64) -> bool {
        self.lobby.leave_pending(now_ms)
    }

    /// The run's `arena` report block, each match with its commander counts.
    pub fn report(&self) -> Report {
        let matches = self
            .lobby
            .matches()
            .iter()
            .map(|result| match self.commander.tally(result.match_id) {
                Some(tally) => result.clone().with_commander(tally),
                None => result.clone(),
            })
            .collect();
        Report::new(self.side, self.settings.matches, matches)
    }

    /// What happened since the last call, for the runner to log.
    pub fn events(&mut self) -> Vec<LaneEvent> {
        std::mem::take(&mut self.events)
    }

    /// The picture in compact JSON (the `arena-picture` record).
    pub fn picture_json(&self) -> Value {
        let p = &self.picture;
        let own = &p.own;
        let contacts: Vec<Value> = p
            .contacts
            .values()
            .map(|c| {
                json!({"designator": c.designator, "side": c.side, "bearing": c.bearing_deg,
                    "range_m": c.range_m, "age_ms": c.age_ms,
                    "track": c.track.as_ref().map(|t| json!({"course": t.course_deg.round(),
                        "mps": (t.speed_mps * 10.0).round() / 10.0}))})
            })
            .collect();
        let weapons: Vec<Value> = p
            .weapons
            .iter()
            .map(|w| {
                json!({"id": w.id, "ready": w.ready, "rounds": w.rounds, "percent": w.percent,
                    "cycle_ms": w.cycle_ms, "slack_ms": w.slack_ms})
            })
            .collect();
        // The pilot's own state for a reader of the transcript (cut AK.1): the
        // hunt (whom, which leg, how old its plot), the fall back, the stall.
        let hunt = self.pilot.hunt.as_ref().map(|hunt| {
            json!({"d": hunt.trail.designator,
                "leg": match hunt.leg {
                    HuntLeg::ToPlot => "to_plot",
                    HuntLeg::Along => "along",
                    HuntLeg::Sweep { .. } => "sweep",
                },
                "age_s": p.last_ms.saturating_sub(hunt.trail.plot_ms) / 1_000})
        });
        // Cut AL.2: the turn into a hit (its bearing, whole degrees, and the
        // whole seconds it has left), the live turn with one track gone, the
        // held-ground marks and what the hold is doing with the range.
        let threat = self.pilot.threat.map(|(bearing, until)| {
            json!({"bearing": (bearing.round() as u32) % 360,
                "until_s": until.saturating_sub(p.last_ms).div_ceil(1_000)})
        });
        // Cut AL.3: the wait flown at the last decision, either kind — a
        // return not yet told apart, or a hunted hostile expected back — whom
        // it is for and the whole seconds it has left.
        let wait = self.pilot.wait.as_ref().map(
            |(d, until)| json!({"d": d, "left_s": until.saturating_sub(p.last_ms).div_ceil(1_000)}),
        );
        json!({"phase": phase_word(&p.phase), "match": p.following, "step": self.lobby.step(),
            "lead": self.lobby.lead(), "aboard": p.aboard,
            "own": {"callsign": own.callsign, "power": own.power.as_str(), "kph": own.speed_kph,
                "heading": own.heading_deg,
                "pos": own.pos_m.map(|(x, y)| [x.round(), y.round()]),
                "held": own.held.is_some(), "immobile": own.immobile, "disabled": own.disabled,
                "destroyed": own.destroyed},
            "lock": p.lock, "lock_percent": p.lock_percent, "contacts": contacts,
            "weapons": weapons, "struck": p.struck, "doubt": p.weapons_doubt.is_some(),
            "tally": p.tally, "doctrine": self.doctrine,
            "pilot": {"hunt": hunt, "fallen_back": self.pilot.fallen_back,
                "stall": self.pilot.stall_level, "threat": threat,
                "live_turn": live_turn(p).map(Turn::as_str),
                "marks": self.pilot.held_marks.len(),
                "hold": self.pilot.hold_motion.as_str(), "wait": wait}})
    }

    /// The pilot's own state (for tests and `/arena`).
    pub fn pilot(&self) -> &Pilot {
        &self.pilot
    }

    // -- stopping -------------------------------------------------------------

    /// **Stop.** Queue the one `SIM LEAVE` when it is due (the centre last read
    /// idle or in selection, a slot held); [`Lane::tick`] then returns it and
    /// nothing after it. Idempotent.
    pub fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        let now = self.picture.last_ms;
        self.lobby.stop(now, &self.picture);
        self.fight = None;
        self.commander_sync(now);
        self.events.extend(self.lobby.take_events());
        self.commander_records();
    }

    // -- the commander (cut AJ.3) ----------------------------------------------

    /// **An ask is due**: the picture to send ([`Picture::commander_view`]),
    /// when the pilot is flying, the commander is on and no call is in flight.
    /// The ask is then in flight, with the `CONTACTS` period in force (1 000 ms
    /// holding a contact, 5 000 ms searching; the stale bound is twice it).
    pub fn commander_due(&mut self, now_ms: u64) -> Option<Value> {
        self.commander_sync(now_ms);
        let due = if self.fighting() {
            let held = !self.picture.contacts.is_empty();
            self.commander.due(now_ms, held)
        } else {
            None
        };
        // The call limit may have stopped it.
        self.withdraw_rule(now_ms);
        self.commander_records();
        due.map(|_| self.picture.commander_view(now_ms, &self.doctrine))
    }

    /// **Would an ask be due now?** [`Lane::commander_due`] without marking or
    /// counting anything ([`Schedule::ready`]): the runner asks this first,
    /// then checks the budget, then takes the ask.
    pub fn commander_ready(&mut self, now_ms: u64) -> bool {
        self.commander_sync(now_ms);
        self.commander_records();
        self.fighting() && self.commander.ready(now_ms)
    }

    /// **What came back** for the call in flight: a timely valid doctrine goes
    /// through the pilot's rule ([`vet`]; an overrule is recorded
    /// `commander-overruled` and counted) and is flown from the next tick;
    /// everything else keeps the doctrine. The first kept `withdraw` of an
    /// episode starts its hold, `hold_s` long; a later one while the hold runs
    /// replaces the doctrine but never moves its end. A reply to a call the
    /// lane no longer waits for is ignored.
    pub fn commander_reply(&mut self, now_ms: u64, reply: CommanderReply) {
        // A hold already over is closed first, so its rest is in force for
        // this answer.
        self.withdraw_rule(now_ms);
        if let Some(doctrine) = self.commander.reply(now_ms, reply) {
            let may_hold = now_ms >= self.rest_until_ms;
            // An overruled withdraw with a hostile held is flown as the
            // pilot's own engagement (cut AL.3; it was always the orbit).
            let Vetted {
                doctrine,
                overruled,
            } = vet(
                doctrine,
                &self.picture,
                may_hold,
                self.settings.engagement.stance(),
            );
            if let Some((from, to, why)) = overruled {
                self.commander.overruled(from, to, why);
            }
            if doctrine.stance == Stance::Withdraw && self.doctrine_until_ms.is_none() {
                let hold_s = u64::from(doctrine.hold_s.unwrap_or(HOLD_DEFAULT_S));
                self.doctrine_until_ms = Some(now_ms.saturating_add(hold_s.saturating_mul(1_000)));
            }
            self.doctrine = doctrine;
        }
        // Five errors or five stale answers may have stopped it.
        self.withdraw_rule(now_ms);
        self.commander_records();
    }

    /// **Off for the rest of this lane** (the runner's budget stop, after
    /// [`Lane::commander_ready`] and before the ask is taken, or at the start
    /// of a lane after the session's budget has blocked), with its reason,
    /// once. A standing `withdraw` is cleared.
    pub fn commander_off(&mut self, why: &str) {
        self.commander.stop_for_good(why);
        let now = self.picture.last_ms;
        self.withdraw_rule(now);
        self.commander_records();
    }

    /// **The bounded withdraw** (cut AK.2), after every place the commander
    /// can stop or time can pass: a standing `withdraw` is cleared to the
    /// pilot's default when the commander is off (`"the commander stopped"`)
    /// or when its episode's hold has run out (`"the hold ran out"`); the end
    /// of an episode, whatever stands then, begins the rest.
    fn withdraw_rule(&mut self, now_ms: u64) {
        if self.doctrine.stance == Stance::Withdraw && self.commander.off() {
            self.lapse(now_ms, LAPSED_COMMANDER_STOPPED);
            return;
        }
        if let Some(until) = self.doctrine_until_ms
            && now_ms >= until
        {
            if self.doctrine.stance == Stance::Withdraw {
                self.lapse(now_ms, LAPSED_HOLD_RAN_OUT);
            } else {
                self.doctrine_until_ms = None;
                self.rest_until_ms = now_ms.saturating_add(u64::from(HOLD_MAX_S) * 1_000);
            }
        }
    }

    /// A standing `withdraw` gives way to the pilot's own doctrine
    /// (`doctrine-lapsed` `{"why", "was"}`), and the rest begins.
    fn lapse(&mut self, now_ms: u64, why: &str) {
        let was = std::mem::replace(&mut self.doctrine, Doctrine::pilot_default(&self.settings));
        self.commander
            .record("doctrine-lapsed", json!({"why": why, "was": was}));
        self.doctrine_until_ms = None;
        self.rest_until_ms = now_ms.saturating_add(u64::from(HOLD_MAX_S) * 1_000);
    }

    /// A commander call is in flight and the lane still waits for it. The
    /// runner aborts a call the lane has stopped waiting for (the match's end,
    /// the tank destroyed, the pilot stopped).
    pub fn commander_in_flight(&self) -> bool {
        self.commander.in_flight().is_some()
    }

    /// The commander's schedule (for tests and `/arena`).
    pub fn commander(&self) -> &Schedule {
        &self.commander
    }

    /// Begin the schedule at a match's `Power online.`, feed it the lock, and
    /// stop it at the match's end, when the tank is destroyed, and when the
    /// pilot stops or its part of the match ends early; give up an ask at its
    /// stale bound; and hold the bounded withdraw to its bounds.
    fn commander_sync(&mut self, now_ms: u64) {
        if let Some(match_id) = self.commander.match_id()
            && !self.commander.off()
        {
            let over = matches!(
                &self.picture.phase,
                Phase::Over { match_id: m, .. } if *m == match_id
            );
            let ours = self.picture.following == Some(match_id);
            if over {
                self.commander.stop(OFF_MATCH_OVER, false);
            } else if ours && self.picture.own.destroyed {
                self.commander.stop(OFF_DESTROYED, true);
            } else if self.stopped
                || self.lobby.closed()
                || !ours
                || self.lobby.matches().iter().any(|m| m.match_id == match_id)
            {
                self.commander.stop(OFF_STOPPED, true);
            }
        }
        // An ask still in flight past its stale bound is given up (cut AK.2).
        self.commander.expire(now_ms);
        if self.fighting()
            && let Phase::Running { match_id } = self.picture.phase
        {
            if self.commander.begin(match_id, now_ms) {
                // A new match's contacts are new: the last match's doctrine
                // (its target above all) means nothing in it, and nor does
                // its hold or rest.
                self.doctrine = Doctrine::pilot_default(&self.settings);
                self.doctrine_until_ms = None;
                self.rest_until_ms = 0;
            }
            if self.commander.match_id() == Some(match_id) {
                self.commander.lock(self.picture.lock.is_some(), now_ms);
            }
        }
        self.withdraw_rule(now_ms);
    }

    /// Hand the schedule's records on as events.
    fn commander_records(&mut self) {
        for (kind, data) in self.commander.take_records() {
            self.events.push(LaneEvent::Record { kind, data });
        }
    }

    // -- the lines in ---------------------------------------------------------

    /// **Read** decoded terminal text: into the picture and the lobby, and the
    /// journal out.
    pub fn feed(&mut self, now_ms: u64, text: &str) -> Fed {
        let mut journal = std::mem::take(&mut self.held_journal);
        for raw in self.assembler.feed(now_ms, text) {
            self.line(now_ms, &raw, &mut journal);
        }
        // The match's end or the tank's loss voids the commander's call at once.
        self.commander_sync(now_ms);
        self.after(now_ms);
        Fed {
            journal: journal.join("\n"),
        }
    }

    fn line(&mut self, now_ms: u64, raw: &str, journal: &mut Vec<String>) {
        let trimmed = raw.trim();
        // The terminal's echo of a line the lane typed: not the game's words.
        if !trimmed.is_empty() && self.recent.iter().any(|sent| sent == trimmed) {
            return;
        }
        let line = classify(raw);
        // The commander's events: it asks at once (cut AJ.3).
        match &line {
            Line::Identified { side, .. } if *side == self.side.other() => {
                self.commander.trigger();
            }
            Line::SectionLost { .. } => self.commander.trigger(),
            _ => {}
        }
        if let Some(fight) = self.fight.as_mut() {
            match &line {
                Line::ContactRow(_) | Line::NoContacts => fight.contacts_open = true,
                Line::Steering { .. } => fight.heading_ack = None,
                Line::Making { .. } | Line::SpeedLimits { .. } => fight.speed_ack = None,
                Line::Immobile => {
                    fight.heading_ack = None;
                    fight.speed_ack = None;
                }
                Line::Locking { .. } => fight.lock_ack = None,
                Line::NewContact { .. }
                | Line::Identified { .. }
                | Line::Unidentified { .. }
                | Line::ContactLost { .. }
                | Line::LockBroken { .. } => {
                    fight
                        .contacts_soon
                        .get_or_insert(now_ms.saturating_add(CONTACTS_AFTER_NOTICE_MS));
                }
                Line::Firing { weapon } => {
                    let learned = self
                        .picture
                        .weapons
                        .iter()
                        .any(|w| w.id == *weapon && w.cycle_ms.is_some());
                    // Armed at every ack of a gun whose cycle is unknown, for
                    // that gun alone (cut AL.1): a later ack of the same gun
                    // re-arms it (the earlier shot's first second is over by
                    // then), and another gun's ack moves nothing of this one.
                    if !learned {
                        fight
                            .weapons_after_shot
                            .insert(weapon.clone(), now_ms.saturating_add(WEAPONS_AFTER_SHOT_MS));
                    }
                }
                Line::Refused {
                    order,
                    weapon: Some(weapon),
                    said,
                    ..
                } if order == "FIRE" && said.starts_with("Too many orders at once") => {
                    fight
                        .fire_hold
                        .insert(weapon.clone(), now_ms.saturating_add(FIRE_BUSY_WAIT_MS));
                }
                _ => {}
            }
        }
        let applied = self.picture.apply(now_ms, &line);
        self.lobby.on_line(now_ms, raw, &line, &mut self.picture);
        self.check_commit(now_ms);
        if applied.journal && !trimmed.is_empty() {
            journal.push(raw.to_owned());
        }
    }

    /// A `CONTACTS` reply just committed: every second one while locked asks
    /// for the lock's `RANGE`.
    fn check_commit(&mut self, now_ms: u64) {
        let locked = self.picture.lock.is_some();
        let Some(fight) = self.fight.as_mut() else {
            return;
        };
        if fight.contacts_open && self.picture.open.contacts.is_none() {
            fight.contacts_open = false;
            if locked {
                fight.contacts_commits = fight.contacts_commits.saturating_add(1);
                if fight.contacts_commits.is_multiple_of(2) {
                    fight.range_until = Some(now_ms.saturating_add(RANGE_WINDOW_MS));
                }
            }
        }
    }

    // -- the lines out --------------------------------------------------------

    /// **What to type now.** The runner calls it every 50 ms and sends every
    /// line in order.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Send> {
        if let Some(tail) = self.assembler.flush_stale(now_ms) {
            let mut journal = std::mem::take(&mut self.held_journal);
            self.line(now_ms, &tail, &mut journal);
            self.held_journal = journal;
        }
        self.picture.settle(now_ms);
        self.check_commit(now_ms);
        if !self.stopped {
            self.lobby.poll(now_ms, &mut self.picture);
        }
        self.sync(now_ms);
        self.commander_sync(now_ms);
        let mut orders = if self.fighting() {
            self.pilot.decide(&self.picture, &self.doctrine, now_ms)
        } else {
            Orders::default()
        };
        // What the pilot said (a hunt, a fall back, a held tank) and what the
        // picture said of the guns (a gun struck, emptied or every gun held,
        // cut AL.1): `arena` events, not journalled (cut AK.1).
        for note in self.picture.take_notes() {
            self.events.push(LaneEvent::Said(note));
        }
        for note in self.pilot.take_notes() {
            self.events.push(LaneEvent::Said(note));
        }
        let mut out = Vec::new();
        while self.slot_free(now_ms) {
            let Some((line, purpose)) = self.next(now_ms, &mut orders) else {
                break;
            };
            self.ring.push_back(now_ms);
            while self.ring.len() > self.max_lines() {
                self.ring.pop_front();
            }
            self.recent.push_back(line.clone());
            while self.recent.len() > RECENT_SENT {
                self.recent.pop_front();
            }
            out.push(Send {
                line,
                purpose,
                lane_ms: now_ms,
            });
        }
        self.after(now_ms);
        out
    }

    fn max_lines(&self) -> usize {
        usize::try_from(self.settings.max_lines_per_s.clamp(1, 4)).unwrap_or(4)
    }

    /// The limiter: a line may go when fewer than N went in the last second.
    fn slot_free(&self, now_ms: u64) -> bool {
        self.ring.len() < self.max_lines()
            || self
                .ring
                .front()
                .is_some_and(|oldest| now_ms >= oldest.saturating_add(LIMIT_WINDOW_MS))
    }

    /// The pilot is at the controls of a powered tank in its running match.
    fn fighting(&self) -> bool {
        self.fight.is_some()
            && !self.stopped
            && self.picture.aboard
            && self.picture.own.power == Power::Online
            && !self.picture.own.destroyed
            && matches!(self.picture.phase, Phase::Running { .. })
    }

    /// The pilot and the fight's bookkeeping follow the lobby; unanswered orders
    /// are forgotten.
    fn sync(&mut self, now_ms: u64) {
        if self.picture.following != self.pilot_match {
            self.pilot = Pilot::new();
            self.pilot_match = self.picture.following;
        }
        if self.lobby.fighting() && !self.stopped {
            if self.fight.is_none() {
                // Back at the controls (the first time, or after a re-board or a
                // restart): whatever the pilot ordered before is not the tank's
                // order now — the machine flew it meanwhile, and "is yours again
                // where it stands" — so the heading and the speed go again.
                self.pilot.forget_heading();
                self.pilot.forget_speed();
                // …and a one-track commit or a turn into a hit (cut AL.2) is
                // not the tank's now either, nor a hit taken meanwhile.
                self.pilot.forget_turns(&self.picture);
                self.fight = Some(Fight::default());
            }
        } else {
            self.fight = None;
        }
        let Some(fight) = self.fight.as_mut() else {
            return;
        };
        let lapsed =
            |ack: Option<u64>| ack.is_some_and(|at| now_ms >= at.saturating_add(ACK_WAIT_MS));
        if lapsed(fight.heading_ack) {
            fight.heading_ack = None;
            self.pilot.forget_heading();
        }
        if lapsed(fight.speed_ack) {
            fight.speed_ack = None;
            self.pilot.forget_speed();
        }
        if lapsed(fight.lock_ack) {
            fight.lock_ack = None;
        }
    }

    /// The next line by priority, with its side effects.
    fn next(&mut self, now_ms: u64, orders: &mut Orders) -> Option<(String, Purpose)> {
        if let Some(line) = self.lobby.due(now_ms) {
            self.lobby.sent(now_ms, line);
            let purpose = if line.is_walk() {
                Purpose::Walk
            } else {
                Purpose::Lobby
            };
            return Some((line.line(), purpose));
        }
        if !self.fighting() {
            return None;
        }
        let main = main_gun(&self.picture).map(str::to_owned);
        let locked = self.picture.lock.is_some();
        let held = !self.picture.contacts.is_empty();
        let doubt = self.picture.weapons_doubt;
        // Every gun carried has its cycle and none reads `0%` (cut AL.1) — with
        // at least one gun read, and, while locked, each gun's chance read
        // since the last lock line (a chance read before a lock, or against
        // the lock before, says nothing of this one: the guns are read again
        // at the locked cadence). The chance, not the row: a `WEAPONSTATUS`
        // reply's own lock line comes before its rows, and a row the
        // stale-ready guard holds still reads its chance.
        let p = &self.picture;
        let learned = !p.weapons.is_empty()
            && p.weapons.iter().all(|w| {
                w.cycle_ms.is_some()
                    && w.percent != Some(0)
                    && (p.lock.is_none() || w.chance_ms >= p.lock_read_ms)
            });
        // Whether a poll that can wait leaves a slot for every gun due inside
        // the next second (cut AL.1), taken before this line goes.
        let reserve = self.reserve_ok(now_ms);
        let fight = self.fight.as_mut()?;
        // 0. The first `STATUS` and `WEAPONSTATUS` go straight after `Power
        // online.`: nothing can be aimed or fired before they are read, and at
        // one line a second the `CONTACTS` cadence would take every slot.
        if fight.last_status.is_none() {
            fight.last_status = Some(now_ms);
            return Some(("STATUS".into(), Purpose::Poll));
        }
        if fight.last_weapons.is_none() {
            fight.last_weapons = Some(now_ms);
            return Some(("WEAPONSTATUS".into(), Purpose::Poll));
        }
        // 1. The doubt's `WEAPONSTATUS` (cut AL.1): a lost section said `its
        // weapons are gone`, and nothing is fired until the guns are read
        // again — so it goes at once, past the poll gap, once a doubt.
        if let Some(since) = doubt
            && fight.last_weapons.is_none_or(|at| at < since)
        {
            fight.last_weapons = Some(now_ms);
            // It reads every gun whose post-shot read is due; one not yet due
            // keeps its own (its row now may be the frame before its shot).
            fight.weapons_after_shot.retain(|_, at| *at > now_ms);
            return Some(("WEAPONSTATUS".into(), Purpose::Poll));
        }
        // 2. Every gun the pilot fires, main gun first (cut AL.1): each on its
        // own clock (the pilot's), never inside a lattice step of its last
        // `FIRE`, never while that `FIRE` is unanswered, and never inside a
        // busy refusal's wait.
        let picture = &self.picture;
        let can_fire = |fight: &Fight, weapon: &str| {
            fight
                .last_fire
                .get(weapon)
                .is_none_or(|at| now_ms >= at.saturating_add(FIRE_GAP_MS))
                && fight
                    .fire_hold
                    .get(weapon)
                    .is_none_or(|until| now_ms >= *until)
                && !picture.fire_pending(weapon)
        };
        let gun = main
            .as_ref()
            .and_then(|main| orders.fire.iter().position(|w| w == main))
            .filter(|index| can_fire(fight, &orders.fire[*index]))
            .or_else(|| orders.fire.iter().position(|w| can_fire(fight, w)));
        if let Some(index) = gun {
            let weapon = orders.fire.remove(index);
            fight.last_fire.insert(weapon.clone(), now_ms);
            self.picture.sent_fire(now_ms, &weapon);
            return Some((format!("FIRE {weapon}"), Purpose::Order));
        }
        // 3. The lock.
        if let Some(target) = orders.lock.take() {
            fight.lock_ack = Some(now_ms);
            self.pilot.sent_lock(now_ms);
            return Some((format!("LOCK {target}"), Purpose::Order));
        }
        // 4. `CONTACTS`: never deferred, since a gun fires only on a row read
        // inside two seconds.
        let every = if held {
            CONTACTS_HELD_MS
        } else {
            CONTACTS_EMPTY_MS
        };
        let soon = fight.contacts_soon.is_some_and(|at| now_ms >= at);
        if (poll_due(fight.last_contacts, every, now_ms) || soon)
            && gap_ok(fight.last_contacts, now_ms)
        {
            fight.last_contacts = Some(now_ms);
            fight.contacts_soon = None;
            return Some(("CONTACTS".into(), Purpose::Poll));
        }
        // 5. The post-shot `WEAPONSTATUS`, inside the first second after the
        // ack of a gun whose cycle is not yet known: due when any such gun's
        // read is; a gun whose cycle a row has set since (or one struck from
        // the fire schedule) needs none.
        fight.weapons_after_shot.retain(|armed, _| {
            !picture.struck.contains(armed)
                && !picture.weapons.iter().any(|w| {
                    (w.id == *armed || w.short.as_deref() == Some(armed.as_str()))
                        && w.cycle_ms.is_some()
                })
        });
        let after_shot = fight.weapons_after_shot.values().any(|at| now_ms >= *at);
        if after_shot && gap_ok(fight.last_weapons, now_ms) {
            fight.last_weapons = Some(now_ms);
            fight.weapons_after_shot.retain(|_, at| *at > now_ms);
            return Some(("WEAPONSTATUS".into(), Purpose::Poll));
        }
        // 6. The heading, 7. the speed.
        if let Some(deg) = orders.heading.take() {
            fight.heading_ack = Some(now_ms);
            self.pilot.sent_heading(deg, now_ms);
            return Some((format!("HEADING {deg}"), Purpose::Order));
        }
        if let Some(kph) = orders.speed.take() {
            fight.speed_ack = Some(now_ms);
            self.pilot.sent_speed(kph);
            return Some((format!("SPEED {kph}"), Purpose::Order));
        }
        // 8. The lock's `RANGE`, straight after its reply: never deferred, as
        // it is good only inside its window and it refines the lock's fix.
        if let Some(until) = fight.range_until {
            fight.range_until = None;
            if locked && now_ms <= until && gap_ok(fight.last_range, now_ms) {
                fight.last_range = Some(now_ms);
                return Some(("RANGE".into(), Purpose::Poll));
            }
        }
        // 9. The polls that can wait (cut AL.1): each only when it leaves a
        // slot for every gun due inside the next second, or once it is
        // `POLL_STARVE_MS` overdue.
        let may_wait = |last: Option<u64>, every: u64| {
            reserve
                || last.is_none_or(|at| {
                    now_ms
                        >= at
                            .saturating_add(every.max(POLL_GAP_MS))
                            .saturating_add(POLL_STARVE_MS)
                })
        };
        // 9a. `STATUS`.
        if poll_due(fight.last_status, STATUS_EVERY_MS, now_ms)
            && may_wait(fight.last_status, STATUS_EVERY_MS)
        {
            fight.last_status = Some(now_ms);
            return Some(("STATUS".into(), Purpose::Poll));
        }
        // 9b. The periodic `WEAPONSTATUS`.
        let every = if learned {
            WEAPONS_LEARNED_MS
        } else if locked {
            WEAPONS_LOCKED_MS
        } else {
            WEAPONS_IDLE_MS
        };
        if poll_due(fight.last_weapons, every, now_ms) && may_wait(fight.last_weapons, every) {
            fight.last_weapons = Some(now_ms);
            // As the doubt's: only the post-shot reads already due are done.
            fight.weapons_after_shot.retain(|_, at| *at > now_ms);
            return Some(("WEAPONSTATUS".into(), Purpose::Poll));
        }
        // 9c. `BEARING B`.
        if orders.ask_spot_b
            && poll_due(fight.last_bearing, BEARING_EVERY_MS, now_ms)
            && may_wait(fight.last_bearing, BEARING_EVERY_MS)
        {
            orders.ask_spot_b = false;
            fight.last_bearing = Some(now_ms);
            return Some(("BEARING B".into(), Purpose::Poll));
        }
        None
    }

    /// **The guns' reservation** (cut AL.1): whether a poll that can wait may
    /// go now and still leave a slot for every gun due inside the next second.
    /// Always while the pilot fires nothing: with no lock, a friendly lock, or
    /// every gun in doubt (whose `WEAPONSTATUS` is the one read that ends it).
    /// Otherwise the carried guns the pilot may fire — not struck, rounds
    /// left, not read at `0%` (the pilot leaves such a gun unfired until a row
    /// says otherwise, so a slot kept for it would only hold back the read
    /// that could) — that have a clock (`next_ready_ms`), each due at
    /// `d = max(clock, now)`, those with `d` inside the limiter's window,
    /// sorted by `d`: the `k`-th (from 0) must find the lines already in the
    /// window ending at `d`, this poll, the `k` guns due before it and itself
    /// inside [`Lane::max_lines`].
    fn reserve_ok(&self, now_ms: u64) -> bool {
        let p = &self.picture;
        if p.lock.is_none() || p.lock_friendly || p.weapons_doubt.is_some() {
            return true;
        }
        let mut due: Vec<u64> = p
            .weapons
            .iter()
            .filter(|w| !p.struck.contains(&w.id) && w.rounds_left() > 0 && w.percent != Some(0))
            .filter_map(|w| w.next_ready_ms)
            .map(|at| at.max(now_ms))
            .filter(|d| *d < now_ms.saturating_add(LIMIT_WINDOW_MS))
            .collect();
        due.sort_unstable();
        due.iter().enumerate().all(|(k, d)| {
            let in_window = self
                .ring
                .iter()
                .filter(|sent| sent.saturating_add(LIMIT_WINDOW_MS) > *d)
                .count();
            // The lines already in the window, this poll, the guns due before
            // this one, and this gun.
            let lines = in_window + 1 + k + 1;
            lines <= self.max_lines()
        })
    }

    /// After every feed and tick: the lobby's sentences, the state record, the
    /// result.
    fn after(&mut self, _now_ms: u64) {
        self.events.extend(self.lobby.take_events());
        self.commander_records();
        let record = StateRecord {
            phase: phase_word(&self.picture.phase).to_owned(),
            match_id: self.picture.following,
            callsign: self
                .picture
                .own
                .callsign
                .clone()
                .or_else(|| self.lobby.callsign()),
            lead: self.lobby.lead(),
            aboard: self.picture.aboard,
        };
        let changed = self.last_state.as_ref().is_none_or(|last| {
            last.phase != record.phase
                || last.match_id != record.match_id
                || last.lead != record.lead
                || last.aboard != record.aboard
        });
        if changed {
            self.last_state = Some(record.clone());
            self.events.push(LaneEvent::State(record));
        }
        if self.result.is_none() && !self.stopped && self.lobby.finished() {
            let report = self.report();
            let blocker = self.lobby.blocker();
            let flown = u32::try_from(report.matches.len()).unwrap_or(u32::MAX);
            if blocker.is_some() || flown >= self.settings.matches {
                self.result = Some(LaneResult {
                    status: if report.done(blocker.is_some()) {
                        "done"
                    } else {
                        "blocked"
                    }
                    .into(),
                    outcome: report.outcome(blocker),
                });
            }
        }
    }

    /// The step the lobby is on (for tests and `/arena`).
    pub fn step(&self) -> Step {
        self.lobby.step()
    }
}
