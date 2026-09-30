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
use std::collections::BTreeMap;

use mush_bot::arena::{
    Lane, LaneEvent, Purpose, Send, Side,
    lobby::{Centre, Step},
    picture::Power,
    settings::Settings,
};

/// The fake clock's step, and the runner's tick.
const STEP_MS: u64 = 10;
const TICK_MS: u64 = 50;

/// The lines the lane may ever type.
fn allowed(line: &str) -> bool {
    const EXACT: [&str; 17] = [
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

/// **The scripted server.**
struct Server {
    now: u64,
    queue: Vec<(u64, String)>,
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
    speed: i32,
}

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
        }
    }

    fn say(&mut self, delay: u64, text: &str) {
        let mut chunk = String::new();
        for line in text.lines() {
            chunk.push_str(line);
            chunk.push_str("\r\n");
        }
        self.queue.push((self.now + delay, chunk));
    }

    fn due(&mut self, now: u64) -> Vec<String> {
        let (due, later): (Vec<_>, Vec<_>) = self.queue.drain(..).partition(|(at, _)| *at <= now);
        self.queue = later;
        let mut due = due;
        due.sort_by_key(|(at, _)| *at);
        due.into_iter().map(|(_, text)| text).collect()
    }

    fn cycle_left(&self, weapon: &str) -> Option<u64> {
        let cycle = if weapon == "cannon" { 6_000 } else { 1_000 };
        let fired = *self.fired.get(weapon)?;
        (self.now < fired + cycle).then(|| fired + cycle - self.now)
    }

    fn weapon_row(&self, weapon: &str, short: &str) -> String {
        let state = match self.cycle_left(weapon) {
            Some(left) => format!("cycling ({} s)", left.div_ceil(1_000)),
            None => "ready".into(),
        };
        let chance = if self.locked.is_some() {
            "  band 1 of 3  62% to hit"
        } else {
            ""
        };
        format!("{weapon}{short}  {state}  30 of 40 round(s){chance}")
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
        );
        if lobby && let Some(transient) = self.transient.take() {
            self.say(3, &transient);
            return;
        }
        let me = self.me.clone();
        let id = self.match_id;
        match line {
            "look" => self.say(3, "Limbo #0\nA grey place with no edges."),
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
            "SIM TEAM red" => {
                if self.board != "idle" && self.board != "selection" {
                    self.say(3, "The last match is cleared from the board and the tanks are back at muster.");
                }
                self.board = "selection".into();
                self.say(
                    4,
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
                self.say(3, &format!("{me} gives up red slot 1."));
            }
            _ if self.outside => self.say(3, "You are not at the station of a deployed unit."),
            "CONTACTS" => {
                if self.contacts.is_empty() {
                    self.say(3, "No contacts.");
                } else {
                    let rows = self.contacts.join("\n");
                    self.say(3, &rows);
                }
            }
            "STATUS" => {
                let (heading, speed) = (self.heading, self.speed);
                self.say(
                    3,
                    &format!(
                        "Callsign RA.\nRed Tank 1: power online, {speed} kph, heading {heading:03}°\n  at 60 m east, 970 m north; scope holds a hostile\n  nearest spot: The Ford (A), 400 m at 090°\n  hull  intact  internals 60/60\n    front 400/400  left 250/250  right 250/250  rear 150/150\n  turret  intact\n    front 350/350  left 220/220  right 220/220  rear 140/140"
                    ),
                );
            }
            "WEAPONSTATUS" => {
                let mut reply = String::new();
                if let Some(lock) = &self.locked {
                    reply.push_str(&format!("Locked on {lock}, 62% to hit.\n"));
                }
                reply.push_str(&self.weapon_row("cannon", ""));
                reply.push('\n');
                reply.push_str(&self.weapon_row("machine_gun", " (mg)"));
                self.say(3, &reply);
            }
            "RANGE" => match self.locked.clone() {
                Some(lock) => self.say(3, &format!("Range to {lock}: 440 m.")),
                None => self.say(3, "No lock: LOCK a contact first, or name one."),
            },
            "BEARING B" => self.say(3, "Bearing to The Crossroads (B): 090° (rel 000°)."),
            _ => {
                if let Some(deg) = line.strip_prefix("HEADING ") {
                    self.heading = deg.parse().unwrap_or(0);
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
                        self.say(3, &format!("Making {kph} kph."));
                    }
                } else if let Some(target) = line.strip_prefix("LOCK ") {
                    self.say(3, &format!("Locking {target}."));
                    self.locked = Some(target.to_owned());
                    self.say(500, &format!("Locked on {target}, 62% to hit."));
                } else if let Some(weapon) = line.strip_prefix("FIRE ") {
                    if self.locked.is_none() {
                        self.say(
                            3,
                            &format!("FIRE {weapon} refused. No lock: LOCK a contact first."),
                        );
                    } else if let Some(left) = self.cycle_left(weapon) {
                        self.say(
                            3,
                            &format!(
                                "FIRE {weapon} refused. Still cycling: {} s to go.",
                                left.div_ceil(1_000)
                            ),
                        );
                    } else {
                        self.fired.insert(weapon.to_owned(), self.now);
                        self.say(3, &format!("Firing {weapon}."));
                        let lock = self.locked.clone().unwrap_or_default();
                        if weapon == "cannon" {
                            self.say(600, &format!("Hit on {lock}: turret."));
                        } else {
                            self.say(600, &format!("Miss on {lock}: the target moved."));
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
        }
    }

    fn step(&mut self) {
        self.now += STEP_MS;
        for text in self.server.due(self.now) {
            let fed = self.lane.feed(self.now, &text);
            if !fed.journal.is_empty() {
                self.journal.push(fed.journal);
            }
        }
        if self.now.is_multiple_of(TICK_MS) {
            for send in self.lane.tick(self.now) {
                self.server.answer(self.now, &send.line);
                self.sent.push(send);
            }
        }
        for event in self.lane.events() {
            if let LaneEvent::Said(text) = event {
                self.said.push(text);
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
    let mut run = Run::new(settings);
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
    let run = to_the_fight(Settings::default());
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
    run.server.contacts = vec!["X1  unknown  bearing 000° (rel 270°)  far  450 m  1 s".into()];
    run.server.now = run.now;
    run.server.say(0, "New contact X1: bearing 000°, far.");
    run.until(3_000, |_| false);
    run.server.contacts = vec!["BA  blue  bearing 000° (rel 270°)  far  450 m  1 s".into()];
    run.server.now = run.now;
    run.server.say(0, "Contact X1 identified: BA, blue.");
}

#[test]
fn a_five_minute_fight_keeps_under_four_lines_a_second_and_the_vocabulary() {
    let mut run = to_the_fight(Settings::default());
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
    let mut run = to_the_fight(Settings::default());
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
        ..Settings::default()
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
    let mut run = to_the_fight(Settings::default());
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
    let mut run = to_the_fight(Settings::default());
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
        let mut run = Run::new(Settings::default());
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
    let mut run = to_the_fight(Settings::default());
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
        ..Settings::default()
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
        ..Settings::default()
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
    let mut run = to_the_fight(Settings::default());
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
    let mut run = Run::new(Settings::default());
    run.server.no_centre = true;
    assert!(run.until(10_000, |run| run.lane.result().is_some()));
    let result = run.lane.result().expect("a result");
    assert_eq!(result.status, "blocked");
    assert_eq!(result.outcome, "This server has no simulator centre.");
    assert!(run.lane.report().matches.is_empty());
}
