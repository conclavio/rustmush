#![cfg(test)]
//! 17g C4 item 5 (doc 17f §9, doc 17g §6, doc 44 Q-44-48): **Registry load
//! beside live arenas, reported.** A measurement, not a gate: one genuine
//! Registry rating, in ONE server process, while the simulator centre runs
//! AI-filled matches back to back, and one `C4_LOAD` line of what it cost.
//! Timing misses are reported, never asserted (the owner's report-only
//! ruling); the story fails only when the rating or the rig itself fails.
//!
//! The rig is the server's own (`player_market_commands::world_with_terms`):
//! the production launch composition through `game_modules`, the server's
//! world task, persist path and writer, an in-memory store. The matches are
//! started the way a player starts one: a character standing in the centre
//! types `SIM TEAM red`, `SIM AI <n>`, `SIM START` and `BOARD`, then `EXIT`s,
//! so every hull of the `n` a side is flown by the stock AI fill (Standard
//! Doctrine on the native stock mind); the lead's hull goes to its sealed
//! mind at the `EXIT`. A server holds ONE simulator centre and the centre one
//! match at a time (`tacsim::match_::lobby::Centre`, `PODS_PER_SIDE` = 6), so
//! the load is sized by the match's side, not by a count of matches; when a
//! match ends the lead starts the next at once.
//!
//! Configuration, by environment (unset is the baseline):
//! - `C4_LOAD_SIDE`: units a side, 0 to 6; 0 runs no match;
//! - `C4_LOAD_AUTHORS`: 1, or 2 for the queue probe (a second author on
//!   another login asks for a rating right after the first);
//! - `MIND_PULSE_MS`: the Registry pulse (`paid_purchase::mind_pulse`);
//! - `C4_LOAD_REV`: the revision the report names;
//! - `C4_LOAD_GUARD_S`: a shake-down's shorter guard (the span ends there,
//!   the report says `complete=no` and the story fails).
//!
//! What it reads, all measured: the store's own commit stamp on every lease's
//! grant and settle (`paid_purchase::store::lease_clock`); the arena task's
//! existing per-batch `pool_wait_us` trace (a tracing layer that takes only
//! that event); the world's barrier round trip every 250 ms; and `/proc`
//! (process and per-thread CPU, peak RSS, load average, host).
use super::paid_purchase::runtime::{Seen, rows};
use super::paid_purchase::store::lease_clock::{LeaseCommit, Leg};
use super::player_market_commands::{TestResult, World, active, saved, world_with_terms};
use crate::world_task::{WorldHandle, WorldReadError};
use mush_core::{CommitToken, GOD, ObjId};
use mush_ipc::SendTarget;
use mush_roll::commerce::RevisionIdentity;
use mush_roll::commerce::rating::{RatingJob, RatingJobState};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::field::{Field, Visit};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata};
use tracing_subscriber::layer::{Context, Layer};

/// The rating's hang guard: two interleaved ratings at the 50 ms pulse in a
/// debug build fit with room.
const GUARD: Duration = Duration::from_secs(6 * 3_600);
/// How long the first match may take to reach `running`.
const WARM_UP: Duration = Duration::from_secs(600);
/// The match driver's cadence.
const DRIVE_EVERY: Duration = Duration::from_secs(2);
/// The Roll job poll's cadence.
const POLL_EVERY: Duration = Duration::from_secs(1);
/// The world probe's cadence.
const PROBE_EVERY: Duration = Duration::from_millis(250);
/// Lines kept per watched character.
const LINES_KEPT: usize = 64;
/// `/proc/<pid>/stat` ticks a second (Linux's `USER_HZ`, 100 on every
/// x86-64 kernel this runs on).
const CLK_TCK: u64 = 100;

fn knob(name: &str, default: u8, max: u8) -> TestResult<u8> {
    match std::env::var(name) {
        Ok(raw) if !raw.trim().is_empty() => {
            let n: u8 = raw
                .trim()
                .parse()
                .map_err(|_| format!("{name} is not a small number: {raw:?}"))?;
            check!(n <= max, "{name} is at most {max}: {n}");
            Ok(n)
        }
        _ => Ok(default),
    }
}

// ---- the arena's think-wait trace -----------------------------------------

/// Every `a think batch was delivered` event: when, which match, `pool_wait_us`.
#[derive(Clone, Default)]
struct ThinkLog(Arc<Mutex<Vec<(Instant, String, u64)>>>);
struct ThinkLayer(ThinkLog);
fn think_event(meta: &Metadata<'_>) -> bool {
    meta.is_event()
        && meta.target() == "tacsim::arena_task"
        && meta.fields().field("pool_wait_us").is_some()
}
#[derive(Default)]
struct ThinkFields {
    wait_us: Option<u64>,
    match_id: String,
}
impl Visit for ThinkFields {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "pool_wait_us" {
            self.wait_us = Some(value);
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "match_id" {
            self.match_id = format!("{value:?}");
        }
    }
}
impl<S: tracing::Subscriber> Layer<S> for ThinkLayer {
    fn register_callsite(&self, meta: &'static Metadata<'static>) -> Interest {
        if think_event(meta) {
            Interest::always()
        } else {
            Interest::never()
        }
    }
    fn enabled(&self, meta: &Metadata<'_>, _: Context<'_, S>) -> bool {
        think_event(meta)
    }
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut fields = ThinkFields::default();
        event.record(&mut fields);
        if let (Some(wait), Ok(mut log)) = (fields.wait_us, self.0.0.lock())
            && log.len() < 4_000_000
        {
            log.push((Instant::now(), fields.match_id, wait));
        }
    }
}
/// The process's one subscriber, taking that event alone; `None` when the
/// process already has one (then the think-wait figures read `na`).
fn install_think_probe() -> Option<ThinkLog> {
    use tracing_subscriber::layer::SubscriberExt;
    let log = ThinkLog::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(ThinkLayer(log.clone())),
    )
    .ok()
    .map(|()| log)
}

// ---- /proc ------------------------------------------------------------------

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}
/// `utime + stime` of a `stat` line, in ticks.
fn stat_ticks(stat: &str) -> Option<u64> {
    let rest = stat.rsplit_once(") ")?.1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}
/// One `/proc` reading: the process's CPU ticks, each live thread's name and
/// ticks, the load average.
struct Proc {
    cpu: Option<u64>,
    threads: BTreeMap<String, (String, u64)>,
    load: String,
}
fn snapshot() -> Proc {
    let mut threads = BTreeMap::new();
    if let Ok(dir) = std::fs::read_dir("/proc/self/task") {
        for entry in dir.flatten() {
            let tid = entry.file_name().to_string_lossy().into_owned();
            let comm = read(&format!("/proc/self/task/{tid}/comm")).unwrap_or_default();
            if let Some(ticks) =
                read(&format!("/proc/self/task/{tid}/stat")).and_then(|s| stat_ticks(&s))
            {
                threads.insert(tid, (comm.trim().to_owned(), ticks));
            }
        }
    }
    Proc {
        cpu: read("/proc/self/stat").and_then(|s| stat_ticks(&s)),
        threads,
        load: read("/proc/loadavg")
            .map(|l| l.split_whitespace().take(3).collect::<Vec<_>>().join("/"))
            .unwrap_or_else(|| "na".into()),
    }
}
/// The thread group a thread's name puts it in.
fn group(comm: &str) -> &'static str {
    if comm.starts_with("mind-bulk") {
        "bulk"
    } else if comm.starts_with("tacsim-mind") {
        "think"
    } else if comm.starts_with("tokio-runtime") {
        "tokio"
    } else {
        "other"
    }
}
/// CPU seconds per group between two readings, over the threads alive at the
/// second (a thread that exited between them is not counted).
fn group_cpu(start: &Proc, end: &Proc) -> BTreeMap<&'static str, f64> {
    let mut out = BTreeMap::new();
    for (comm, secs) in name_cpu(start, end) {
        *out.entry(group(&comm)).or_insert(0.0) += secs;
    }
    out
}
/// CPU seconds per thread name (the kernel's 15-character `comm`), as
/// `group_cpu` counts them.
fn name_cpu(start: &Proc, end: &Proc) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for (tid, (comm, ticks)) in &end.threads {
        let before = start.threads.get(tid).map_or(0, |(_, ticks)| *ticks);
        *out.entry(comm.replace(' ', "_")).or_insert(0.0) +=
            ticks.saturating_sub(before) as f64 / CLK_TCK as f64;
    }
    out
}
fn status_kb(field: &str) -> Option<u64> {
    read("/proc/self/status")?
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}
/// The host, one token a field.
fn host() -> String {
    let cpuinfo = read("/proc/cpuinfo").unwrap_or_default();
    let online = cpuinfo
        .lines()
        .filter(|l| l.starts_with("processor"))
        .count();
    let model = cpuinfo
        .lines()
        .find_map(|l| l.strip_prefix("model name"))
        .and_then(|l| l.split_once(':'))
        .map_or("na".to_owned(), |(_, m)| m.trim().replace(' ', "_"));
    let allowed = std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get);
    let mem_gb = read("/proc/meminfo")
        .and_then(|m| {
            m.lines()
                .find_map(|l| l.strip_prefix("MemTotal:"))
                .and_then(|r| r.split_whitespace().next()?.parse::<u64>().ok())
        })
        .map_or("na".to_owned(), |kb| {
            format!("{:.1}", kb as f64 / 1_048_576.0)
        });
    let kernel =
        read("/proc/sys/kernel/osrelease").map_or("na".to_owned(), |k| k.trim().to_owned());
    format!(
        "cores_online={online} cores_allowed={allowed} mem_gb={mem_gb} kernel={kernel} cpu={model}"
    )
}

// ---- figures ------------------------------------------------------------------

/// Nearest-rank percentile of sorted values.
fn pct(sorted: &[u64], p: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}
/// `p50/p95/p99/max` of microseconds, in milliseconds.
fn dist_ms(values: &mut [u64], points: &[usize]) -> String {
    values.sort_unstable();
    let mut out: Vec<String> = points
        .iter()
        .map(|p| pct(values, *p).map_or("na".to_owned(), |v| format!("{:.1}", v as f64 / 1000.0)))
        .collect();
    out.push(
        values
            .last()
            .map_or("na".to_owned(), |v| format!("{:.1}", *v as f64 / 1000.0)),
    );
    out.join("/")
}
fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}
fn secs_after(from: Instant, at: Option<Instant>) -> String {
    at.map_or("na".to_owned(), |at| {
        format!("{:.1}", at.saturating_duration_since(from).as_secs_f64())
    })
}

/// One job's leases, from the store's clock.
struct Leases {
    count: usize,
    first_grant: Option<Instant>,
    last_settle: Option<Instant>,
    /// Grant commit to settle commit, per lease, microseconds.
    latency: Vec<u64>,
    /// Settle commit to the next grant commit, microseconds.
    gap: Vec<u64>,
    grants: BTreeMap<u64, Instant>,
}
fn leases(commits: &[LeaseCommit], job: CommitToken) -> Leases {
    let mut grants = BTreeMap::new();
    let mut settles = BTreeMap::new();
    for commit in commits.iter().filter(|c| c.job == job) {
        let into = match commit.leg {
            Leg::Grant => &mut grants,
            Leg::Settle => &mut settles,
        };
        into.entry(commit.sequence).or_insert(commit.at);
    }
    let latency = grants
        .iter()
        .filter_map(|(seq, g)| {
            settles
                .get(seq)
                .map(|s: &Instant| micros(s.saturating_duration_since(*g)))
        })
        .collect();
    let gap = settles
        .iter()
        .filter_map(|(seq, s)| {
            grants
                .get(&(seq + 1))
                .map(|g: &Instant| micros(g.saturating_duration_since(*s)))
        })
        .collect();
    Leases {
        count: settles.len(),
        first_grant: grants.values().min().copied(),
        last_settle: settles.values().max().copied(),
        latency,
        gap,
        grants,
    }
}

// ---- lines --------------------------------------------------------------------

/// What reached the watched characters, with when.
struct Mail {
    origin: Instant,
    lines: BTreeMap<u32, Vec<(Duration, String)>>,
}
impl Mail {
    fn new(origin: Instant, watched: &[ObjId]) -> Self {
        Self {
            origin,
            lines: watched.iter().map(|id| (id.0, Vec::new())).collect(),
        }
    }
    fn push(&mut self, who: u64, line: String) {
        let at = self.origin.elapsed();
        if let Ok(who) = u32::try_from(who)
            && let Some(kept) = self.lines.get_mut(&who)
            && kept.len() < LINES_KEPT
        {
            kept.push((at, line));
        }
    }
    /// Routes one drain: `actor`'s own lines, then every other line by its
    /// target.
    fn absorb(&mut self, actor: ObjId, seen: Seen) {
        for line in seen.own {
            self.push(u64::from(actor.0), line);
        }
        for (line, to) in seen.others.into_iter().zip(seen.to) {
            if let SendTarget::Conn(n) | SendTarget::Char(n) = to {
                self.push(n, line);
            }
        }
    }
    fn of(&self, who: ObjId) -> &[(Duration, String)] {
        self.lines.get(&who.0).map_or(&[], Vec::as_slice)
    }
}

/// `actor` types `line`; whether any line it caused carries `needle`.
async fn says(
    w: &World,
    mail: &mut Mail,
    actor: ObjId,
    line: &str,
    needle: &str,
) -> TestResult<(bool, Vec<String>)> {
    let seen = w.server.command_seen(actor, line.to_owned()).await?;
    let own = seen.own.clone();
    let hit = seen
        .own
        .iter()
        .chain(&seen.others)
        .any(|l| l.contains(needle));
    mail.absorb(actor, seen);
    Ok((hit, own))
}

// ---- the match driver -------------------------------------------------------

/// The lead of the red side, starting AI-filled matches back to back.
struct Driver {
    lead: ObjId,
    lobby: ObjId,
    red: ObjId,
    side: u8,
    started: u32,
    seated: bool,
    running_since: Option<Instant>,
    running: Duration,
    /// Every `(state, count)` the board was read in.
    states: BTreeMap<String, u32>,
    /// The first few replies that were not what the step expected.
    refusals: Vec<String>,
}
impl Driver {
    async fn state(&mut self, w: &World, mail: &mut Mail) -> TestResult<String> {
        let (_, own) = says(w, mail, self.lead, "SIM STATUS", "Simulator centre: ").await?;
        let state = own
            .iter()
            .find_map(|l| l.strip_prefix("Simulator centre: "))
            .and_then(|rest| rest.split([' ', '(']).next())
            .unwrap_or("unknown")
            .to_owned();
        *self.states.entry(state.clone()).or_insert(0) += 1;
        Ok(state)
    }
    async fn step(&mut self, w: &World, mail: &mut Mail) -> TestResult<String> {
        let state = self.state(w, mail).await?;
        let live = matches!(state.as_str(), "running" | "stopping");
        match (live, self.running_since) {
            (true, None) => self.running_since = Some(Instant::now()),
            (false, Some(since)) => {
                self.running += since.elapsed();
                self.running_since = None;
                self.seated = false;
            }
            _ => {}
        }
        match state.as_str() {
            "idle" | "ended" | "aborted" | "void" => {
                self.god(w, mail, self.lobby).await?;
                if self
                    .expect(w, mail, "SIM TEAM red", "takes red slot")
                    .await?
                {
                    self.select(w, mail).await?;
                }
            }
            "selection" => self.select(w, mail).await?,
            "boarding" => self.board(w, mail).await?,
            // The board answers only inside the centre: the lead is walked in.
            "unknown" => self.god(w, mail, self.lobby).await?,
            "running" if self.seated => {
                let out = self.expect(w, mail, "EXIT", "You climb out of").await?;
                self.seated = !out;
            }
            _ => {}
        }
        Ok(state)
    }
    async fn select(&mut self, w: &World, mail: &mut Mail) -> TestResult {
        let ai = format!("SIM AI {}", self.side);
        if self.expect(w, mail, &ai, "AI fill set to").await?
            && self.expect(w, mail, "SIM START", "is boarding").await?
        {
            self.board(w, mail).await?;
        }
        Ok(())
    }
    async fn board(&mut self, w: &World, mail: &mut Mail) -> TestResult {
        if self.seated {
            return Ok(());
        }
        self.god(w, mail, self.red).await?;
        if self.expect(w, mail, "BOARD", "You climb into").await? {
            self.seated = true;
            self.started += 1;
        }
        Ok(())
    }
    async fn god(&mut self, w: &World, mail: &mut Mail, room: ObjId) -> TestResult {
        let line = format!("@teleport #{}=#{}", self.lead.0, room.0);
        let seen = w.server.command_seen(GOD, line).await?;
        mail.absorb(GOD, seen);
        Ok(())
    }
    async fn expect(
        &mut self,
        w: &World,
        mail: &mut Mail,
        line: &str,
        needle: &str,
    ) -> TestResult<bool> {
        let (hit, own) = says(w, mail, self.lead, line, needle).await?;
        if !hit && self.refusals.len() < 12 {
            self.refusals.push(format!("{line} -> {own:?}"));
        }
        Ok(hit)
    }
}

/// The dbref an `@find` listing gives `name`.
async fn find(w: &World, mail: &mut Mail, name: &str) -> TestResult<ObjId> {
    let seen = w.server.command_seen(GOD, format!("@find {name}")).await?;
    let found = seen.own.iter().find_map(|line| {
        let rest = line.split_once(name)?.1;
        let inside = rest.split_once("(#")?.1;
        let digits: String = inside.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u32>().ok()
    });
    let own = seen.own.clone();
    mail.absorb(GOD, seen);
    Ok(ObjId(found.ok_or_else(|| {
        format!("no dbref for {name:?} in {own:?}")
    })?))
}

/// The world's barrier round trip every `PROBE_EVERY`: microseconds, and how
/// many reads the world answered `Busy`.
fn spawn_probe(
    handle: WorldHandle,
    stop: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<(Vec<u64>, u64)> {
    tokio::spawn(async move {
        let mut rtt = Vec::new();
        let mut busy = 0_u64;
        while !stop.load(Ordering::Relaxed) {
            let started = Instant::now();
            match tokio::time::timeout(Duration::from_secs(120), handle.barrier_result()).await {
                Ok(Ok(())) => rtt.push(micros(started.elapsed())),
                Ok(Err(WorldReadError::Busy)) => busy += 1,
                _ => break,
            }
            tokio::time::sleep(PROBE_EVERY).await;
        }
        (rtt, busy)
    })
}

/// One rating requested: who, which revision, when, and what it was told.
struct Asked {
    author: ObjId,
    identity: RevisionIdentity,
    sent: Instant,
    reply: Vec<String>,
    job: Option<CommitToken>,
    queued_seen: Option<Instant>,
    running_seen: Option<Instant>,
    completed_seen: Option<Instant>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "17g C4 item 5: a Registry rating beside live simulator matches, measured; run by hand"]
async fn c4_registry_rating_beside_live_simulator_matches_is_measured() -> TestResult {
    let side = knob("C4_LOAD_SIDE", 0, 6)?;
    let authors = knob("C4_LOAD_AUTHORS", 1, 2)?.max(1);
    let think = install_think_probe();
    let origin = Instant::now();
    let mut w = world_with_terms("mush-c4-load", tacsim::launch::MarketTerms::PROVISIONAL).await?;
    // The fill on, and no unattended stop: every pilot of these matches has
    // left (the lead `EXIT`s), so the default five minutes would end each one.
    w.server.stop().await?;
    w.launch.config.tacsim.match_.ai_fill = true;
    w.launch.config.tacsim.match_.unattended_ms = 0;
    w.server = active(&w.launch, w.alt, w.bystander).await?;
    let result = measure(&w, side, authors, think, origin).await;
    w.server.stop().await?;
    let _ = std::fs::remove_dir_all(&w.data_dir);
    result
}

async fn measure(
    w: &World,
    side: u8,
    authors: u8,
    think: Option<ThinkLog>,
    origin: Instant,
) -> TestResult {
    let p = w.launch.principals;
    let lead = w.bystander;
    let mut mail = Mail::new(origin, &[p.author, p.buyer, lead]);
    let lobby = find(w, &mut mail, "Simulator Lobby").await?;
    let red = find(w, &mut mail, "Red Room").await?;
    let mut asked = vec![(p.author, saved(w, p.author, "Sentinel").await?)];
    if authors == 2 {
        asked.push((p.buyer, saved(w, p.buyer, "Probe").await?));
    }
    let mut driver = Driver {
        lead,
        lobby,
        red,
        side,
        started: 0,
        seated: false,
        running_since: None,
        running: Duration::ZERO,
        states: BTreeMap::new(),
        refusals: Vec::new(),
    };
    // Warm-up: the first match running before the rating is asked for.
    if side > 0 {
        let end = Instant::now() + WARM_UP;
        loop {
            let state = driver.step(w, &mut mail).await?;
            if state == "running" && !driver.seated {
                break;
            }
            check!(
                Instant::now() < end,
                "the first match runs: {state}; {:?}; {:?}",
                driver.refusals,
                mail.of(lead)
            );
            tokio::time::sleep(DRIVE_EVERY).await;
        }
    }
    let warm_starts = driver.started;
    let guard = std::env::var("C4_LOAD_GUARD_S")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or(GUARD, Duration::from_secs)
        .min(GUARD);
    let start = snapshot();
    let started_at = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let probe = spawn_probe(w.server.handle()?.clone(), stop.clone());
    let mut jobs: Vec<Asked> = Vec::new();
    for (author, revision) in &asked {
        let name = if *author == p.author {
            "Sentinel@1"
        } else {
            "Probe@1"
        };
        let sent = Instant::now();
        let seen = w
            .server
            .command_seen(*author, format!("@mind/rate {name}"))
            .await?;
        let reply = seen.own.clone();
        mail.absorb(*author, seen);
        jobs.push(Asked {
            author: *author,
            identity: RevisionIdentity::capture(revision)?,
            sent,
            reply,
            job: None,
            queued_seen: None,
            running_seen: None,
            completed_seen: None,
        });
    }
    let first_sent = jobs.first().map_or(started_at, |a| a.sent);
    let mut next_drive = Instant::now();
    let mut next_poll = Instant::now();
    loop {
        mail.absorb(p.author, w.server.take_seen(p.author)?);
        let now = Instant::now();
        if now >= next_poll {
            next_poll = now + POLL_EVERY;
            for row in rows(&w.launch.store, "roll", "rating_jobs").await? {
                let job: RatingJob = serde_json::from_value(row)?;
                let Some(entry) = jobs
                    .iter_mut()
                    .find(|a| a.identity == job.subject().revision())
                else {
                    continue;
                };
                entry.job.get_or_insert(job.job());
                entry.queued_seen.get_or_insert(now);
                match job.state() {
                    RatingJobState::Queued {} => {}
                    RatingJobState::Running { .. } => {
                        entry.running_seen.get_or_insert(now);
                    }
                    RatingJobState::Completed { .. } => {
                        entry.completed_seen.get_or_insert(now);
                    }
                    other => return Err(format!("a rating ended {other:?}").into()),
                }
            }
            if jobs.iter().all(|a| a.completed_seen.is_some()) {
                break;
            }
        }
        // A shake-down's short guard ends the span early and still reports
        // (`complete=no`); the story then fails.
        if first_sent.elapsed() >= guard {
            break;
        }
        if side > 0 && now >= next_drive {
            next_drive = now + DRIVE_EVERY;
            driver.step(w, &mut mail).await?;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let ended_at = Instant::now();
    stop.store(true, Ordering::Relaxed);
    let end = snapshot();
    if let Some(since) = driver.running_since {
        driver.running += since.elapsed();
    }
    let (mut rtt, busy) = probe.await?;
    let commits = w.launch.store.lease_commits();

    // ---- the report ----
    let cfg = &w.launch.config;
    let profile = if cfg!(debug_assertions) {
        "test"
    } else {
        "release"
    };
    let rev = std::env::var("C4_LOAD_REV").unwrap_or_else(|_| "unknown".into());
    let units = u32::from(side) * 2;
    let config = format!(
        "side={side} units={units} authors={authors} pulse_ms={} proving_workers={} mind_workers={} tokio_workers=8 \
         ai_fill=native_standard_doctrine max_duration_ms={} profile={profile} rev={rev} {} load_start={} load_end={}",
        cfg.tacsim.minds.bulk_maintenance_ms,
        cfg.tacsim.minds.proving_workers,
        cfg.tacsim.minds.workers,
        cfg.tacsim.match_.max_duration_ms,
        host(),
        start.load,
        end.load,
    );
    eprintln!("C4_LOAD_CONFIG {config}");
    let span = ended_at.saturating_duration_since(started_at).as_secs_f64();
    let cpu = match (start.cpu, end.cpu) {
        (Some(a), Some(b)) => b.saturating_sub(a) as f64 / CLK_TCK as f64,
        _ => f64::NAN,
    };
    let groups = group_cpu(&start, &end);
    let group = |name: &str| groups.get(name).copied().unwrap_or(0.0);
    // Think waits and batch intervals, over the measured span.
    let (mut waits, mut intervals, batches) = match &think {
        Some(log) => {
            let log = log.0.lock().map_err(|_| "think log poisoned")?;
            let mut waits = Vec::new();
            let mut last: BTreeMap<&str, Instant> = BTreeMap::new();
            let mut intervals = Vec::new();
            for (at, id, wait) in log
                .iter()
                .filter(|(at, _, _)| *at >= started_at && *at <= ended_at)
            {
                waits.push(*wait);
                if let Some(prev) = last.insert(id.as_str(), *at) {
                    intervals.push(micros(at.saturating_duration_since(prev)));
                }
            }
            let batches = waits.len();
            (waits, intervals, batches.to_string())
        }
        None => (Vec::new(), Vec::new(), "na".to_owned()),
    };
    let mut lines = Vec::new();
    let complete = jobs.iter().all(|a| a.completed_seen.is_some());
    for (index, a) in jobs.iter().enumerate() {
        let Some(job) = a.job else {
            eprintln!(
                "C4_LOAD side={side} rating={index} complete=no job=none reply={:?}",
                a.reply
            );
            continue;
        };
        let mut l = leases(&commits, job);
        let lease_cpu_ms = if l.count == 0 {
            f64::NAN
        } else {
            group("bulk") * 1000.0 / l.count as f64
        };
        lines.push(format!(
            "C4_LOAD side={side} units={units} authors={authors} rating={index} complete={} rating_wall_s={} \
             queued_s={} first_grant_s={} last_settle_s={} leases={} lease_ms_p50_p95_max={} \
             between_leases_ms_p50_p95_max={} matches_started={} match_running_s={:.0} span_s={span:.0} \
             think_batches={batches} think_wait_ms_p50_p95_p99_max={} batch_interval_ms_p50_p95_p99_max={} \
             world_rtt_ms_p50_p95_p99_max={} world_busy={busy} proc_cpu_s={cpu:.0} avg_cores={:.2} \
             bulk_cpu_s={:.0} bulk_cpu_ms_per_lease={lease_cpu_ms:.0} think_cpu_s={:.0} tokio_cpu_s={:.0} \
             other_cpu_s={:.0} peak_rss_mb={} rss_end_mb={} reply={:?}",
            if a.completed_seen.is_some() { "yes" } else { "no" },
            secs_after(a.sent, a.completed_seen),
            secs_after(a.sent, a.queued_seen),
            secs_after(a.sent, l.first_grant),
            secs_after(a.sent, l.last_settle),
            l.count,
            dist_ms(&mut l.latency, &[50, 95]),
            dist_ms(&mut l.gap, &[50, 95]),
            driver.started,
            driver.running.as_secs_f64(),
            dist_ms(&mut waits, &[50, 95, 99]),
            dist_ms(&mut intervals, &[50, 95, 99]),
            dist_ms(&mut rtt, &[50, 95, 99]),
            cpu / span,
            group("bulk"),
            group("think"),
            group("tokio"),
            group("other"),
            status_kb("VmHWM:").map_or("na".to_owned(), |kb| (kb / 1024).to_string()),
            status_kb("VmRSS:").map_or("na".to_owned(), |kb| (kb / 1024).to_string()),
            a.reply,
        ));
    }
    for line in &lines {
        eprintln!("{line}");
    }
    eprintln!(
        "C4_LOAD_MATCHES side={side} warm_up_starts={warm_starts} states={:?} refusals={:?}",
        driver.states, driver.refusals
    );
    let mut by_name: Vec<(String, f64)> = name_cpu(&start, &end)
        .into_iter()
        .filter(|(_, secs)| *secs > 0.0)
        .collect();
    by_name.sort_by(|a, b| b.1.total_cmp(&a.1));
    by_name.truncate(12);
    let by_name: Vec<String> = by_name
        .iter()
        .map(|(name, secs)| format!("{name}={secs:.1}"))
        .collect();
    eprintln!("C4_LOAD_THREADS side={side} cpu_s {}", by_name.join(" "));
    // The queue: what the second author was told, and when its leases ran.
    if let [first, second] = jobs.as_slice()
        && let (Some(a), Some(b)) = (first.job, second.job)
    {
        let la = leases(&commits, a);
        let lb = leases(&commits, b);
        let mut order: Vec<(Instant, bool)> = la
            .grants
            .values()
            .map(|at| (*at, false))
            .chain(lb.grants.values().map(|at| (*at, true)))
            .collect();
        order.sort_by_key(|(at, _)| *at);
        let overlap_end = la
            .grants
            .values()
            .max()
            .copied()
            .min(lb.grants.values().max().copied());
        let (mut run, mut max_run, mut last) = (0_u32, 0_u32, None);
        for (_, is_b) in order.iter().filter(|(at, _)| {
            lb.first_grant.is_some_and(|f| *at >= f) && overlap_end.is_some_and(|e| *at <= e)
        }) {
            run = if last == Some(*is_b) { run + 1 } else { 1 };
            max_run = max_run.max(run);
            last = Some(*is_b);
        }
        let a_before_b = lb
            .first_grant
            .map_or(0, |f| la.grants.values().filter(|at| **at < f).count());
        // Each line the second author read, seconds after its `@mind/rate`.
        let asked_at = second.sent.saturating_duration_since(origin);
        let told: Vec<String> = mail
            .of(second.author)
            .iter()
            .filter(|(at, _)| *at >= asked_at)
            .map(|(at, line)| format!("{:.1}s:{line}", at.saturating_sub(asked_at).as_secs_f64()))
            .collect();
        eprintln!(
            "C4_LOAD_QUEUE second_reply={:?} second_queued_s={} second_first_grant_s={} first_grants_before_second={a_before_b} \
             longest_same_job_run_in_overlap={max_run} first_last_settle_s={} second_last_settle_s={} told={told:?}",
            second.reply,
            secs_after(second.sent, second.queued_seen),
            secs_after(second.sent, lb.first_grant),
            secs_after(first.sent, la.last_settle),
            secs_after(second.sent, lb.last_settle),
        );
    }
    check!(
        complete,
        "the ratings complete within the guard ({}s)",
        guard.as_secs()
    );
    let distinct: BTreeSet<CommitToken> = commits.iter().map(|c| c.job).collect();
    check_eq!(
        distinct.len(),
        jobs.len(),
        "the lease clock saw one job a rating: {:?}",
        mail.of(p.author)
    );
    Ok(())
}
