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
//! **What goes first** when a slot is free: the lobby's line; the match's first
//! `STATUS` and first `WEAPONSTATUS` (straight after `Power online.`); the main
//! gun's `FIRE`; `LOCK`; `CONTACTS` when due; the other guns' `FIRE`s;
//! `HEADING`; `SPEED`; the lock's `RANGE` when due; `STATUS` when due;
//! `WEAPONSTATUS` when due; `BEARING B` when the pilot asks. One line per free
//! slot per tick.
//!
//! **The polls** (a line-mode pilot is sent no periodic picture, and a frame
//! changes at most once a pulse — 500, 1 000 or 5 000 ms engaged, on alert,
//! quiet): `CONTACTS` every 1 000 ms while any contact is held, every 5 000 ms
//! otherwise, and once 250 ms after any contact notice; while locked, a bare
//! `RANGE` straight after every second `CONTACTS` reply commits, so it lands
//! inside the picture's 400 ms and refines that row's fix ("a fresh one reads
//! to 10 m", `help cockpit`); `STATUS` every 2 000 ms; `WEAPONSTATUS` 1 000 ms
//! after each shot until that weapon's cycle is learned, then every 3 000 ms
//! while locked and every 5 000 ms otherwise; `BEARING B` every 5 000 ms while
//! the pilot searches; never the same poll twice inside 500 ms; the first
//! `STATUS` and `WEAPONSTATUS` straight after `Power online.`. Locked and
//! firing both guns that is about 3.75 lines a second, under the cap; the order
//! above decides what slips when it is not.
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
//! the arena's pulse. What the schedule records reaches the runner as
//! [`LaneEvent::Record`].
use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;
use serde_json::{Value, json};

use super::{
    Assembler, Side, classify,
    commander::{CommanderReply, Doctrine, OFF_DESTROYED, OFF_MATCH_OVER, OFF_STOPPED, Schedule},
    lines::Line,
    lobby::{Lobby, Step},
    picture::{Phase, Picture, Power},
    pilot::{HuntLeg, Orders, Pilot},
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
/// `WEAPONSTATUS` this long after a shot, until the weapon's cycle is learned.
pub const WEAPONS_AFTER_SHOT_MS: u64 = 1_000;
/// `WEAPONSTATUS` this often while locked…
pub const WEAPONS_LOCKED_MS: u64 = 3_000;
/// …and this often otherwise.
pub const WEAPONS_IDLE_MS: u64 = 5_000;
/// `BEARING B` this often while searching.
pub const BEARING_EVERY_MS: u64 = 5_000;
/// Never the same poll twice inside this.
pub const POLL_GAP_MS: u64 = 500;
/// An order unanswered this long is re-sent when re-decided.
pub const ACK_WAIT_MS: u64 = 1_500;
/// At most one `FIRE` per weapon this often.
pub const FIRE_GAP_MS: u64 = 700;
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
    /// `commander-stale`, `commander-invalid`, `commander-error` or
    /// `commander-off`, and its data.
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
    weapons_after_shot: Option<u64>,
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
                    "cycle_ms": w.cycle_ms})
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
        json!({"phase": phase_word(&p.phase), "match": p.following, "step": self.lobby.step(),
            "lead": self.lobby.lead(), "aboard": p.aboard,
            "own": {"callsign": own.callsign, "power": own.power.as_str(), "kph": own.speed_kph,
                "heading": own.heading_deg,
                "pos": own.pos_m.map(|(x, y)| [x.round(), y.round()]),
                "held": own.held.is_some(), "immobile": own.immobile, "disabled": own.disabled,
                "destroyed": own.destroyed},
            "lock": p.lock, "lock_percent": p.lock_percent, "contacts": contacts,
            "weapons": weapons, "tally": p.tally, "doctrine": self.doctrine,
            "pilot": {"hunt": hunt, "fallen_back": self.pilot.fallen_back,
                "stall": self.pilot.stall_level}})
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

    /// **What came back** for the call in flight: a timely valid doctrine is
    /// flown from the next tick; everything else keeps the doctrine. A reply
    /// to a call the lane no longer waits for is ignored.
    pub fn commander_reply(&mut self, now_ms: u64, reply: CommanderReply) {
        if let Some(doctrine) = self.commander.reply(now_ms, reply) {
            self.doctrine = doctrine;
        }
        self.commander_records();
    }

    /// **Off for the rest of this lane** (the runner's budget stop, after
    /// [`Lane::commander_ready`] and before the ask is taken, or at the start
    /// of a lane after the session's budget has blocked), with its reason,
    /// once.
    pub fn commander_off(&mut self, why: &str) {
        self.commander.stop_for_good(why);
        self.commander_records();
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
    /// pilot stops or its part of the match ends early.
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
        if self.fighting()
            && let Phase::Running { match_id } = self.picture.phase
        {
            if self.commander.begin(match_id, now_ms) {
                // A new match's contacts are new: the last match's doctrine
                // (its target above all) means nothing in it.
                self.doctrine = Doctrine::pilot_default(&self.settings);
            }
            if self.commander.match_id() == Some(match_id) {
                self.commander.lock(self.picture.lock.is_some(), now_ms);
            }
        }
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
                    if !learned {
                        fight.weapons_after_shot =
                            Some(now_ms.saturating_add(WEAPONS_AFTER_SHOT_MS));
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
        // What the pilot said (a hunt, a fall back, a held tank): `arena`
        // events, not journalled (cut AK.1).
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
        let can_fire = |fight: &Fight, weapon: &str| {
            fight
                .last_fire
                .get(weapon)
                .is_none_or(|at| now_ms >= at.saturating_add(FIRE_GAP_MS))
                && fight
                    .fire_hold
                    .get(weapon)
                    .is_none_or(|until| now_ms >= *until)
        };
        // 1. The main gun.
        if let Some(main) = &main
            && let Some(index) = orders.fire.iter().position(|w| w == main)
            && can_fire(fight, main)
        {
            let weapon = orders.fire.remove(index);
            fight.last_fire.insert(weapon.clone(), now_ms);
            self.picture.sent_fire(now_ms, &weapon);
            return Some((format!("FIRE {weapon}"), Purpose::Order));
        }
        // 2. The lock.
        if let Some(target) = orders.lock.take() {
            fight.lock_ack = Some(now_ms);
            self.pilot.sent_lock(now_ms);
            return Some((format!("LOCK {target}"), Purpose::Order));
        }
        // 3. `CONTACTS`.
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
        // 4. The other guns.
        if let Some(index) = orders.fire.iter().position(|w| can_fire(fight, w)) {
            let weapon = orders.fire.remove(index);
            fight.last_fire.insert(weapon.clone(), now_ms);
            self.picture.sent_fire(now_ms, &weapon);
            return Some((format!("FIRE {weapon}"), Purpose::Order));
        }
        // 5. The heading, 6. the speed.
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
        // 7. The lock's `RANGE`, straight after its reply.
        if let Some(until) = fight.range_until {
            fight.range_until = None;
            if locked && now_ms <= until && gap_ok(fight.last_range, now_ms) {
                fight.last_range = Some(now_ms);
                return Some(("RANGE".into(), Purpose::Poll));
            }
        }
        // 8. `STATUS`.
        if poll_due(fight.last_status, STATUS_EVERY_MS, now_ms) {
            fight.last_status = Some(now_ms);
            return Some(("STATUS".into(), Purpose::Poll));
        }
        // 9. `WEAPONSTATUS`.
        let every = if locked {
            WEAPONS_LOCKED_MS
        } else {
            WEAPONS_IDLE_MS
        };
        let after_shot = fight.weapons_after_shot.is_some_and(|at| now_ms >= at);
        if (poll_due(fight.last_weapons, every, now_ms) || after_shot)
            && gap_ok(fight.last_weapons, now_ms)
        {
            fight.last_weapons = Some(now_ms);
            fight.weapons_after_shot = None;
            return Some(("WEAPONSTATUS".into(), Purpose::Poll));
        }
        // 10. `BEARING B`.
        if orders.ask_spot_b && poll_due(fight.last_bearing, BEARING_EVERY_MS, now_ms) {
            orders.ask_spot_b = false;
            fight.last_bearing = Some(now_ms);
            return Some(("BEARING B".into(), Purpose::Poll));
        }
        None
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
