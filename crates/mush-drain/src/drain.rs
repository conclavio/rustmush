//! Draining a generation (doc 29 §9, doc 33 §4.5).
//!
//! Established sessions cannot move between processes (an SSH session's keys
//! and sequence numbers live in this process's memory), so a restart moves
//! only *new* connections: the old generation stops accepting and keeps
//! serving until its sessions end or the deadline passes. Along the way every
//! live session is warned on the owner's schedule (Q-29-2: 12h, 6h, 2h, 1h,
//! 15m and 1m remaining).
//!
//! The registry here is what the warnings and the deadline reach: one entry
//! per registered session, as the daemon sees it ([`Session`]). The doorway's
//! is a russh handle with its channel and the inject queue its pump reads; the
//! gateway's will be a connection handle on its session manager, whose
//! warnings go out as `Sys` events through the normal delivery path.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::sync::{Notify, watch};

use crate::kill::KillSwitch;

/// The default deadline: 24 h after the drain begins (doc 29 §9.4).
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(86_400);
/// The default warnings, as time remaining before the deadline, longest
/// first (Q-29-2, the owner's schedule).
pub const DEFAULT_WARNINGS: [&str; 6] = ["12h", "6h", "2h", "1h", "15m", "1m"];
/// How long a finish (`SIGTERM`) waits after its one warning.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(30);
/// The warning line a daemon with no wording of its own shows; `{remaining}`
/// is replaced by the warning's label. The doorway's wording is operator
/// config whose default, the owner's text, lives in its own crate.
pub const DEFAULT_MESSAGE: &str = "ATTENTION: the server is restarting; please find a safe place and reconnect within the next {remaining}.";

/// What the drain can reach of one session.
///
/// Both calls run concurrently across every registered session, under one
/// 5 s bound, so one session that is not reading never delays another's
/// warning. An implementation should not wait on a full queue: the session
/// is closed at the deadline regardless.
pub trait Session: Send + Sync + 'static {
    /// Show one warning line, between whatever the session is otherwise
    /// writing (never inside an escape sequence).
    fn inject<'a>(&'a self, line: &'a str) -> BoxFuture<'a, ()>;
    /// Close the session, with the daemon's own reason.
    fn close(&self) -> BoxFuture<'_, ()>;
}

/// The generation's lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Accepting and serving.
    Serving,
    /// Not accepting; serving the sessions it has.
    Draining,
}

/// The registry and lifecycle of this generation.
pub struct Drain<S> {
    phase: watch::Sender<Phase>,
    sessions: Mutex<HashMap<u64, Arc<S>>>,
    tasks: Mutex<HashMap<u64, Arc<KillSwitch>>>,
    live: AtomicU64,
    empty: Notify,
}

impl<S> std::fmt::Debug for Drain<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drain")
            .field("phase", &*self.phase.borrow())
            .field("live", &self.live.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl<S> Default for Drain<S> {
    fn default() -> Self {
        Drain {
            phase: watch::channel(Phase::Serving).0,
            sessions: Mutex::new(HashMap::new()),
            tasks: Mutex::new(HashMap::new()),
            live: AtomicU64::new(0),
            empty: Notify::new(),
        }
    }
}

/// Held by every live connection; the generation may exit when none remain.
pub struct LiveGuard<S> {
    drain: Arc<Drain<S>>,
    id: u64,
}

impl<S> std::fmt::Debug for LiveGuard<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveGuard").field("id", &self.id).finish()
    }
}

impl<S> Drop for LiveGuard<S> {
    fn drop(&mut self) {
        self.drain.unregister(self.id);
        if self.drain.live.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.drain.empty.notify_waiters();
        }
    }
}

impl<S> Drain<S> {
    /// A serving generation.
    #[must_use]
    pub fn new() -> Arc<Drain<S>> {
        Arc::new(Drain::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<S>>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Count one connection live.
    pub fn enter(self: &Arc<Self>, id: u64) -> LiveGuard<S> {
        self.live.fetch_add(1, Ordering::SeqCst);
        LiveGuard {
            drain: Arc::clone(self),
            id,
        }
    }

    /// Live connections.
    #[must_use]
    pub fn live(&self) -> u64 {
        self.live.load(Ordering::SeqCst)
    }

    /// Registered sessions (those a warning can reach).
    #[must_use]
    pub fn registered(&self) -> usize {
        self.lock().len()
    }

    /// Register (or replace) a session's reachable parts. A connection's
    /// `LiveGuard` removes the entry again when it ends.
    pub fn register(&self, id: u64, session: S) {
        self.lock().insert(id, Arc::new(session));
    }

    fn unregister(&self, id: u64) {
        self.lock().remove(&id);
        self.tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
    }

    /// Record a connection's kill switch, so a session that never finishes
    /// closing can be cut ([`crate::kill`]). Called before the connection's
    /// task starts, so its `LiveGuard` always removes it again.
    pub fn track(&self, id: u64, task: Arc<KillSwitch>) {
        self.tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, task);
    }

    /// Abort one connection's task: the wedge path, where a polite disconnect
    /// would queue behind data the client is not reading.
    pub fn abort(&self, id: u64) {
        let task = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
        if let Some(t) = task {
            t.kill();
        }
    }

    /// Abort every tracked connection task.
    pub fn abort_all(&self) {
        let tasks: Vec<Arc<KillSwitch>> = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
            .map(|(_, t)| t)
            .collect();
        for t in tasks {
            t.kill();
        }
    }

    /// The current phase.
    #[must_use]
    pub fn phase(&self) -> Phase {
        *self.phase.borrow()
    }

    /// Watch the phase (the accept loops stop on `Draining`).
    #[must_use]
    pub fn watch(&self) -> watch::Receiver<Phase> {
        self.phase.subscribe()
    }

    /// Stop accepting.
    pub fn begin(&self) {
        self.phase.send_replace(Phase::Draining);
    }

    /// Resolves once no connection is live.
    pub async fn wait_empty(&self) {
        loop {
            let notified = self.empty.notified();
            if self.live() == 0 {
                return;
            }
            notified.await;
        }
    }
}

impl<S: Session> Drain<S> {
    fn snapshot(&self) -> Vec<Arc<S>> {
        self.lock().values().map(Arc::clone).collect()
    }

    /// Send one line to every registered session, all at once, under one
    /// bound: one session not reading must not delay everyone else's warning
    /// (adversarial review).
    pub async fn broadcast(&self, line: &str) {
        let targets = self.snapshot();
        let sends = targets.iter().map(|s| s.inject(line));
        let _ =
            tokio::time::timeout(Duration::from_secs(5), futures::future::join_all(sends)).await;
    }

    /// Close every registered session (all at once, under one bound), then
    /// abort every connection task still running.
    pub async fn close_all(&self) {
        let targets = self.snapshot();
        let closes = targets.iter().map(|s| s.close());
        let _ =
            tokio::time::timeout(Duration::from_secs(5), futures::future::join_all(closes)).await;
        // Give the closes a moment to flush, then end what is left.
        tokio::time::sleep(Duration::from_millis(500)).await;
        self.abort_all();
    }

    /// Run a drain to completion: warnings on schedule, then close everything
    /// at the deadline. Returns early if the generation empties first.
    pub async fn run(&self, deadline: Duration, warnings: &[Warning], message: &str) {
        let start = tokio::time::Instant::now();
        let schedule = schedule(deadline, warnings);
        let work = async {
            for (at, label) in schedule {
                tokio::time::sleep_until(start + at).await;
                let line = message.replace("{remaining}", &label);
                tracing::warn!(remaining = %label, sessions = self.live(), "drain warning sent");
                self.broadcast(&line).await;
            }
            tokio::time::sleep_until(start + deadline).await;
            tracing::warn!(sessions = self.live(), "drain deadline reached; closing");
            self.close_all().await;
        };
        tokio::select! {
            () = work => {}
            () = self.wait_empty() => {}
        }
    }

    /// `SIGTERM`: one warning with `grace` remaining, then close everything.
    pub async fn finish_now(&self, grace: Duration, message: &str) {
        let label = format!("{}s", grace.as_secs());
        self.broadcast(&message.replace("{remaining}", &label))
            .await;
        tokio::select! {
            () = tokio::time::sleep(grace) => {}
            () = self.wait_empty() => return,
        }
        self.close_all().await;
    }
}

/// A parsed drain warning: how long before the deadline, and its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// Time remaining when the warning goes out.
    pub remaining: Duration,
    /// The label substituted for `{remaining}` (exactly as configured).
    pub label: String,
}

/// When each warning goes out, as an offset from the drain's start, in order.
/// A warning not shorter than the deadline is skipped.
#[must_use]
pub fn schedule(deadline: Duration, warnings: &[Warning]) -> Vec<(Duration, String)> {
    let mut out: Vec<(Duration, String)> = warnings
        .iter()
        .filter(|w| w.remaining < deadline)
        .map(|w| (deadline.saturating_sub(w.remaining), w.label.clone()))
        .collect();
    out.sort_by_key(|(at, _)| *at);
    out
}

/// A duration like `12h`, `15m` or `30s`; `None` for anything else.
#[must_use]
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let unit = s.chars().last()?;
    let digits = s.get(..s.len().checked_sub(unit.len_utf8())?)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    let secs = match unit {
        's' => n,
        'm' => n.checked_mul(60)?,
        'h' => n.checked_mul(3600)?,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

/// [`DEFAULT_WARNINGS`], parsed, longest remaining first.
#[must_use]
pub fn default_warnings() -> Vec<Warning> {
    DEFAULT_WARNINGS
        .iter()
        .filter_map(|label| {
            Some(Warning {
                remaining: parse_duration(label)?,
                label: (*label).to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// A session that records what reached it.
    #[derive(Debug, Default)]
    pub(crate) struct Recorded {
        pub(crate) lines: Mutex<Vec<String>>,
        pub(crate) closed: AtomicBool,
    }

    impl Recorded {
        pub(crate) fn lines(&self) -> Vec<String> {
            self.lines.lock().unwrap().clone()
        }
        pub(crate) fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }
    }

    /// The registry's view of a [`Recorded`] session.
    pub(crate) struct Rec(pub(crate) Arc<Recorded>);

    impl Session for Rec {
        fn inject<'a>(&'a self, line: &'a str) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                self.0.lines.lock().unwrap().push(line.to_owned());
            })
        }
        fn close(&self) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                self.0.closed.store(true, Ordering::SeqCst);
            })
        }
    }

    fn w(secs: u64, label: &str) -> Warning {
        Warning {
            remaining: Duration::from_secs(secs),
            label: label.into(),
        }
    }

    fn ms(millis: u64, label: &str) -> Warning {
        Warning {
            remaining: Duration::from_millis(millis),
            label: label.into(),
        }
    }

    #[test]
    fn the_owners_schedule_from_a_24h_deadline() {
        let ws = [
            w(43_200, "12h"),
            w(21_600, "6h"),
            w(7_200, "2h"),
            w(3_600, "1h"),
            w(900, "15m"),
            w(60, "1m"),
        ];
        let s = schedule(Duration::from_secs(86_400), &ws);
        let hours: Vec<(u64, &str)> = s.iter().map(|(d, l)| (d.as_secs(), l.as_str())).collect();
        assert_eq!(
            hours,
            [
                (43_200, "12h"),
                (64_800, "6h"),
                (79_200, "2h"),
                (82_800, "1h"),
                (85_500, "15m"),
                (86_340, "1m"),
            ]
        );
    }

    #[test]
    fn the_defaults_are_the_doorways() {
        assert_eq!(DEFAULT_DEADLINE, Duration::from_secs(86_400));
        assert_eq!(DEFAULT_GRACE, Duration::from_secs(30));
        let ws = default_warnings();
        let labels: Vec<&str> = ws.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, DEFAULT_WARNINGS);
        assert!(ws.windows(2).all(|p| p[0].remaining > p[1].remaining));
        assert!(ws.iter().all(|w| w.remaining < DEFAULT_DEADLINE));
        assert!(DEFAULT_MESSAGE.contains("{remaining}"));
    }

    #[test]
    fn warnings_past_the_deadline_are_skipped() {
        let s = schedule(Duration::from_secs(100), &[w(200, "x"), w(10, "y")]);
        assert_eq!(s, [(Duration::from_secs(90), "y".to_owned())]);
    }

    #[test]
    fn durations_parse_as_the_config_writes_them() {
        assert_eq!(parse_duration("12h"), Some(Duration::from_secs(43_200)));
        assert_eq!(parse_duration("15m"), Some(Duration::from_secs(900)));
        assert_eq!(parse_duration(" 30s "), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("0m"), Some(Duration::ZERO));
        for bad in ["", "h", "10", "10x", "-1m", "1.5h", "99999999999999999999h"] {
            assert_eq!(parse_duration(bad), None, "{bad:?}");
        }
    }

    #[tokio::test]
    async fn live_counting_and_wait_empty() {
        let d = Drain::<Rec>::new();
        let a = d.enter(1);
        let b = d.enter(2);
        assert_eq!(d.live(), 2);
        let waiter = {
            let d = Arc::clone(&d);
            tokio::spawn(async move { d.wait_empty().await })
        };
        drop(a);
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        drop(b);
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.phase(), Phase::Serving);
        d.begin();
        assert_eq!(d.phase(), Phase::Draining);
        assert_eq!(*d.watch().borrow(), Phase::Draining);
    }

    #[tokio::test]
    async fn the_registry_reaches_every_session_and_forgets_an_ended_one() {
        let d = Drain::<Rec>::new();
        let one = Arc::new(Recorded::default());
        let two = Arc::new(Recorded::default());
        let g1 = d.enter(1);
        let g2 = d.enter(2);
        d.register(1, Rec(Arc::clone(&one)));
        d.register(2, Rec(Arc::clone(&two)));
        assert_eq!(d.registered(), 2);
        d.broadcast("hello").await;
        assert_eq!(one.lines(), ["hello"]);
        assert_eq!(two.lines(), ["hello"]);

        // Registering again replaces the entry: the new parts get the line.
        let replaced = Arc::new(Recorded::default());
        d.register(2, Rec(Arc::clone(&replaced)));
        assert_eq!(d.registered(), 2);
        d.broadcast("again").await;
        assert_eq!(two.lines(), ["hello"]);
        assert_eq!(replaced.lines(), ["again"]);

        // A connection that ends leaves the registry with its guard.
        drop(g1);
        assert_eq!(d.registered(), 1);
        d.broadcast("last").await;
        assert_eq!(one.lines(), ["hello", "again"]);
        assert_eq!(replaced.lines(), ["again", "last"]);

        d.close_all().await;
        assert!(!one.is_closed());
        assert!(replaced.is_closed());
        drop(g2);
        assert_eq!(d.registered(), 0);
    }

    #[tokio::test]
    async fn tracked_tasks_are_cut_on_abort_and_abort_all() {
        let d = Drain::<Rec>::new();
        let a = KillSwitch::new();
        let b = KillSwitch::new();
        d.track(1, Arc::clone(&a));
        d.track(2, Arc::clone(&b));
        d.abort(1);
        assert!(a.is_killed());
        assert!(!b.is_killed());
        // Aborting again is harmless.
        d.abort(1);
        d.abort_all();
        assert!(b.is_killed());
    }

    #[tokio::test]
    async fn a_run_warns_on_schedule_then_closes_at_the_deadline() {
        let d = Drain::<Rec>::new();
        let s = Arc::new(Recorded::default());
        let _g = d.enter(7);
        d.register(7, Rec(Arc::clone(&s)));
        d.begin();
        // Out of order on purpose: the schedule sorts them.
        let warnings = [ms(150, "1"), ms(300, "2"), ms(0, "late")];
        tokio::time::timeout(
            Duration::from_secs(10),
            d.run(Duration::from_millis(400), &warnings, "T {remaining}"),
        )
        .await
        .unwrap();
        assert_eq!(s.lines(), ["T 2", "T 1", "T late"]);
        assert!(s.is_closed());
    }

    #[tokio::test]
    async fn a_run_returns_early_when_the_generation_empties() {
        let d = Drain::<Rec>::new();
        let s = Arc::new(Recorded::default());
        let g = d.enter(1);
        d.register(1, Rec(Arc::clone(&s)));
        let run = {
            let d = Arc::clone(&d);
            tokio::spawn(async move {
                d.run(Duration::from_secs(60), &[], "T {remaining}").await;
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!run.is_finished());
        drop(g);
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap();
        assert!(s.lines().is_empty());
        assert!(!s.is_closed());
    }

    #[tokio::test]
    async fn a_finish_warns_once_then_closes_after_the_grace() {
        let d = Drain::<Rec>::new();
        let s = Arc::new(Recorded::default());
        let _g = d.enter(1);
        d.register(1, Rec(Arc::clone(&s)));
        tokio::time::timeout(
            Duration::from_secs(10),
            d.finish_now(Duration::from_millis(100), "bye in {remaining}"),
        )
        .await
        .unwrap();
        assert_eq!(s.lines(), ["bye in 0s"]);
        assert!(s.is_closed());
    }
}
