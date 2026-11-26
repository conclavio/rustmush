//! The signal lifecycle (doc 29 §9.3 as amended by the ops review; doc 33
//! §4.5).
//!
//! | Signal | Effect |
//! |---|---|
//! | `SIGUSR1` | **drain** (the swap script only): leave the marker, stop accepting, warn on schedule, close at the deadline |
//! | `SIGTERM`, `SIGINT` | **finish**: stop accepting, one warning, close after the grace period |
//! | `SIGHUP` | the daemon's [`Host::reload`] |
//! | `SIGUSR2` | the daemon's [`Host::reopen_logs`] |
//!
//! `SIGTERM` finishing quickly is what keeps `systemctl stop`/`restart`, a
//! reboot or needrestart from turning into a 24 h outage; the long drain is
//! only ever asked for by the swap script, after a successor is serving. A
//! drain sends no `STOPPING=1` (29a §7 H2: a unit that reports itself
//! stopping breaks the swap script's two-generation guard); a finish does,
//! since the unit is stopping. The process exits when the last session ends,
//! or [`HARD_EXIT_AFTER`] after the close (or the grace) if some session never
//! finished closing.
//!
//! The daemon installs the signal streams first, before anything slow
//! happens: a default `SIGHUP` or `SIGUSR2` (logrotate's postrotate) would
//! otherwise kill a generation that is still starting (29a §7 M4).

use std::sync::Arc;
use std::time::Duration;

use crate::drain::{
    DEFAULT_DEADLINE, DEFAULT_GRACE, DEFAULT_MESSAGE, Drain, Session, Warning, default_warnings,
};

/// What a drain says and when. The default is the doorway's (doc 29 §9.4,
/// Q-29-2): a 24 h deadline, warnings at 12h, 6h, 2h, 1h, 15m and 1m
/// remaining, a 30 s grace on a finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrainConfig {
    /// When the drain closes what is left, after it begins.
    pub deadline: Duration,
    /// The warnings, as time remaining before the deadline.
    pub warnings: Vec<Warning>,
    /// The warning line; `{remaining}` is replaced by the warning's label.
    pub message: String,
    /// How long a finish waits after its one warning.
    pub grace: Duration,
}

impl Default for DrainConfig {
    fn default() -> Self {
        DrainConfig {
            deadline: DEFAULT_DEADLINE,
            warnings: default_warnings(),
            message: DEFAULT_MESSAGE.to_owned(),
            grace: DEFAULT_GRACE,
        }
    }
}

/// What the lifecycle asks of its daemon.
pub trait Host {
    /// The drain's settings, read when a drain or a finish begins (so a
    /// reload that changed them counts).
    fn drain_config(&self) -> DrainConfig;

    /// `SIGHUP`. The doorway reloads its routing and limits; a daemon with
    /// nothing to reload logs and goes on.
    fn reload(&mut self) {
        tracing::warn!("SIGHUP received; nothing to reload");
    }

    /// `SIGUSR2`. The doorway reopens its log file (logrotate's postrotate);
    /// a daemon with no file sink logs and goes on.
    fn reopen_logs(&mut self) {
        tracing::warn!("SIGUSR2 received; no log file to reopen");
    }

    /// Called once, when a drain or a finish begins: the generation has
    /// stopped accepting ([`Drain::begin`] ran) and the daemon closes what
    /// it accepts on and gives up what exactly one generation may hold (the
    /// gateway's listeners and its Discord bot, doc 33 §4.5). The doorway's
    /// accept loops watch the phase instead and need nothing here.
    fn stopped_accepting(&mut self) {}
}

/// How long after the drain's close (or a finish's grace) the process exits
/// even if some session never finished closing.
pub const HARD_EXIT_AFTER: Duration = Duration::from_secs(60);

/// Begin the drain: stop accepting, warn on schedule, close at the deadline;
/// the task returns early if the generation empties first.
pub fn start_drain<S: Session>(
    drain: &Arc<Drain<S>>,
    cfg: DrainConfig,
) -> tokio::task::JoinHandle<()> {
    drain.begin();
    tracing::warn!(
        sessions = drain.live(),
        deadline_secs = cfg.deadline.as_secs(),
        "draining: stopped accepting"
    );
    let drain = Arc::clone(drain);
    tokio::spawn(async move {
        drain.run(cfg.deadline, &cfg.warnings, &cfg.message).await;
    })
}

/// Finish now: stop accepting, one warning, close after the grace. A drain
/// in progress is superseded. A repeated signal does not restart the grace
/// period.
pub fn finish<S: Session>(
    drain: &Arc<Drain<S>>,
    cfg: DrainConfig,
    work: &mut Option<tokio::task::JoinHandle<()>>,
    finishing: &mut bool,
) {
    if *finishing {
        return;
    }
    *finishing = true;
    crate::notify::notify("STOPPING=1");
    drain.begin();
    if let Some(task) = work.take() {
        task.abort();
    }
    tracing::warn!(
        sessions = drain.live(),
        grace_secs = cfg.grace.as_secs(),
        "finishing now"
    );
    let drain = Arc::clone(drain);
    *work = Some(tokio::spawn(async move {
        drain.finish_now(cfg.grace, &cfg.message).await;
    }));
}

/// What a signal (or a [`Trigger`]) asks of the lifecycle. `Reload` and
/// `ReopenLogs` come from `SIGHUP` and `SIGUSR2`, which only Unix has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
enum Event {
    Reload,
    ReopenLogs,
    Drain,
    Finish,
}

/// An in-process stand-in for `SIGUSR1` or `SIGTERM`: the tests' seam. A
/// process signal names the whole process, and the gateway's two-generation
/// tests run both generations in one test process, so [`run_with_trigger`]
/// also listens on a channel of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// As `SIGUSR1`: drain.
    Drain,
    /// As `SIGTERM`: finish now.
    Finish,
}

/// The signal streams. On Unix: `SIGHUP`, `SIGUSR1`, `SIGUSR2`, `SIGTERM`
/// and `SIGINT`. Elsewhere only Ctrl-C exists, and it finishes; nothing
/// drains (there is no second generation to swap to on Windows, and the
/// gateway's tests there exercise the trigger instead).
pub struct Signals {
    #[cfg(unix)]
    hup: tokio::signal::unix::Signal,
    #[cfg(unix)]
    usr1: tokio::signal::unix::Signal,
    #[cfg(unix)]
    usr2: tokio::signal::unix::Signal,
    #[cfg(unix)]
    term: tokio::signal::unix::Signal,
    #[cfg(unix)]
    int: tokio::signal::unix::Signal,
    #[cfg(not(unix))]
    _ctrl_c_only: (),
}

impl std::fmt::Debug for Signals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if cfg!(unix) {
            f.write_str("Signals { HUP, USR1, USR2, TERM, INT }")
        } else {
            f.write_str("Signals { Ctrl-C }")
        }
    }
}

impl Signals {
    /// Install the handlers. Call this first: before anything slow, so a
    /// default `SIGHUP` or `SIGUSR2` never kills a generation that is still
    /// starting (29a §7 M4).
    #[cfg(unix)]
    pub fn install() -> Result<Signals, String> {
        use tokio::signal::unix::{SignalKind, signal};
        let sig = |k: SignalKind| signal(k).map_err(|e| format!("signal handler: {e}"));
        Ok(Signals {
            hup: sig(SignalKind::hangup())?,
            usr1: sig(SignalKind::user_defined1())?,
            usr2: sig(SignalKind::user_defined2())?,
            term: sig(SignalKind::terminate())?,
            int: sig(SignalKind::interrupt())?,
        })
    }

    /// Install the handlers: nothing to install off Unix (Ctrl-C is
    /// registered on first use).
    #[cfg(not(unix))]
    pub fn install() -> Result<Signals, String> {
        Ok(Signals { _ctrl_c_only: () })
    }

    #[cfg(unix)]
    async fn next(&mut self) -> Event {
        tokio::select! {
            _ = self.hup.recv() => Event::Reload,
            _ = self.usr2.recv() => Event::ReopenLogs,
            _ = self.usr1.recv() => Event::Drain,
            _ = self.term.recv() => Event::Finish,
            _ = self.int.recv() => Event::Finish,
        }
    }

    #[cfg(not(unix))]
    async fn next(&mut self) -> Event {
        match tokio::signal::ctrl_c().await {
            Ok(()) => Event::Finish,
            Err(e) => {
                tracing::warn!(error = %e, "cannot listen for Ctrl-C");
                std::future::pending().await
            }
        }
    }
}

/// Serve the signals until the generation should exit: when its last session
/// ends after a drain or finish began, or [`HARD_EXIT_AFTER`] after the
/// close. `marker` is this generation's drain marker (see [`crate::marker`]);
/// a drain leaves it and nothing here removes it.
pub async fn run<S: Session>(
    sigs: &mut Signals,
    drain: &Arc<Drain<S>>,
    marker: &std::path::Path,
    host: &mut impl Host,
) {
    // A sender that is never used and never dropped: the channel stays open,
    // so the trigger arm below pends instead of reading a close.
    let (_never_sent, mut never) = tokio::sync::mpsc::unbounded_channel::<Trigger>();
    run_with_trigger(sigs, &mut never, drain, marker, host).await;
}

/// Explicit state for the drain/finish background task in [`run_with_trigger`].
///
/// Checking `!task.is_finished()` before awaiting a `JoinHandle` races with a
/// fast-finishing task (e.g., zero grace or a task that completes while another
/// `select!` branch is running): once `is_finished()` flips true, the branch
/// would pend forever and `hard_exit` would never arm. Tracking the explicit
/// lifecycle transitions ensures every spawned task is awaited once and always
/// transitions to `Done` + arms `hard_exit`.
enum WorkState {
    Idle,
    Running(tokio::task::JoinHandle<()>),
    Done,
}

impl WorkState {
    const fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    const fn is_stopping(&self) -> bool {
        !self.is_idle()
    }
}

/// [`run`], also answering [`Trigger`]s sent on `trigger` exactly as it
/// answers the corresponding signals. A closed channel is ignored.
pub async fn run_with_trigger<S: Session>(
    sigs: &mut Signals,
    trigger: &mut tokio::sync::mpsc::UnboundedReceiver<Trigger>,
    drain: &Arc<Drain<S>>,
    marker: &std::path::Path,
    host: &mut impl Host,
) {
    run_with_trigger_and_hard_exit(sigs, trigger, drain, marker, host, HARD_EXIT_AFTER).await;
}

async fn run_with_trigger_and_hard_exit<S: Session>(
    sigs: &mut Signals,
    trigger: &mut tokio::sync::mpsc::UnboundedReceiver<Trigger>,
    drain: &Arc<Drain<S>>,
    marker: &std::path::Path,
    host: &mut impl Host,
    hard_exit_after: Duration,
) {
    let mut work = WorkState::Idle;
    let mut finishing = false;
    let mut told_host = false;
    let mut trigger_open = true;
    let mut hard_exit: Option<tokio::time::Instant> = None;
    loop {
        // An empty generation ends the run, but never while the drain's own
        // task still runs: that task may be inside its close, which empties
        // the generation as it queues each close and then waits out the
        // flush window. The task ends at once when the generation empties
        // before its close (`Drain::run`, `Drain::finish_now`), and its close
        // is bounded, so waiting for it costs a clean exit nothing (doc 51
        // unit FL.7).
        let stopping = work.is_stopping() && !matches!(work, WorkState::Running(_));
        let empty = async {
            if stopping {
                drain.wait_empty().await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        let work_done = async {
            match &mut work {
                WorkState::Running(task) => {
                    let _ = task.await;
                }
                WorkState::Idle | WorkState::Done => std::future::pending::<()>().await,
            }
        };
        let hard = async {
            match hard_exit {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending::<()>().await,
            }
        };
        let event = async {
            tokio::select! {
                e = sigs.next() => Some(e),
                t = async {
                    if trigger_open {
                        trigger.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => t.map(|t| match t {
                    Trigger::Drain => Event::Drain,
                    Trigger::Finish => Event::Finish,
                }),
            }
        };
        tokio::select! {
            ev = event => match ev {
                Some(Event::Reload) => host.reload(),
                Some(Event::ReopenLogs) => host.reopen_logs(),
                Some(Event::Drain) => {
                    if work.is_idle() {
                        crate::marker::leave(marker);
                        work = WorkState::Running(start_drain(drain, host.drain_config()));
                        if !told_host {
                            told_host = true;
                            host.stopped_accepting();
                        }
                    }
                }
                Some(Event::Finish) => {
                    if !finishing {
                        let mut slot = match std::mem::replace(&mut work, WorkState::Idle) {
                            WorkState::Running(task) => Some(task),
                            WorkState::Idle | WorkState::Done => None,
                        };
                        finish(drain, host.drain_config(), &mut slot, &mut finishing);
                        work = match slot {
                            Some(task) => WorkState::Running(task),
                            None => WorkState::Done,
                        };
                    }
                    if !told_host {
                        told_host = true;
                        host.stopped_accepting();
                    }
                }
                // The trigger channel closed: only the signals from here on.
                None => trigger_open = false,
            },
            () = work_done => {
                // Transition to `Done` so a completed `JoinHandle` is never
                // polled twice, and arm `hard_exit` (keeping an earlier hard
                // exit if a prior drain had already armed one).
                work = WorkState::Done;
                let next = tokio::time::Instant::now() + hard_exit_after;
                hard_exit = Some(hard_exit.map_or(next, |prev| prev.min(next)));
            }
            () = empty => return,
            () = hard => {
                tracing::warn!(sessions = drain.live(), "sessions still closing; exiting anyway");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drain::tests::{Rec, Recorded};

    fn short() -> DrainConfig {
        DrainConfig {
            deadline: Duration::from_millis(200),
            warnings: vec![Warning {
                remaining: Duration::from_millis(100),
                label: "soon".into(),
            }],
            message: "W {remaining}".into(),
            grace: Duration::from_millis(100),
        }
    }

    #[test]
    fn the_default_config_is_the_doorways() {
        let d = DrainConfig::default();
        assert_eq!(d.deadline, Duration::from_secs(86_400));
        assert_eq!(d.grace, Duration::from_secs(30));
        assert_eq!(d.warnings.len(), 6);
        assert_eq!(d.warnings[0].label, "12h");
        assert_eq!(d.warnings[5].label, "1m");
        assert!(d.message.contains("{remaining}"));
    }

    #[tokio::test]
    async fn a_drain_warns_then_closes_at_its_deadline() {
        let drain = Drain::<Rec>::new();
        let s = Arc::new(Recorded::default());
        let _g = drain.enter(1);
        drain.register(1, Rec(Arc::clone(&s)));
        let work = start_drain(&drain, short());
        assert_eq!(drain.phase(), crate::drain::Phase::Draining);
        tokio::time::timeout(Duration::from_secs(10), work)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(s.lines(), ["W soon"]);
        assert!(s.is_closed());
    }

    #[tokio::test]
    async fn a_finish_supersedes_a_drain_and_does_not_restart_its_grace() {
        let drain = Drain::<Rec>::new();
        let s = Arc::new(Recorded::default());
        let _g = drain.enter(1);
        drain.register(1, Rec(Arc::clone(&s)));
        let mut cfg = short();
        cfg.deadline = Duration::from_secs(60);
        cfg.warnings = vec![];
        let mut work = Some(start_drain(&drain, cfg.clone()));
        let mut finishing = false;
        finish(&drain, cfg.clone(), &mut work, &mut finishing);
        assert!(finishing);
        let first = work.as_ref().map(tokio::task::JoinHandle::id);
        // A second signal changes nothing.
        finish(&drain, cfg, &mut work, &mut finishing);
        assert_eq!(work.as_ref().map(tokio::task::JoinHandle::id), first);
        tokio::time::timeout(Duration::from_secs(10), work.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(s.lines(), ["W 0s"]);
        assert!(s.is_closed());
    }

    /// The in-process seam: a `Trigger::Drain` leaves the marker, tells the
    /// host once, warns and closes on the schedule, and the run ends when the
    /// last session's guard drops; a second trigger changes nothing.
    #[tokio::test]
    async fn a_trigger_drains_like_sigusr1_and_the_run_ends_when_empty() {
        struct Counting {
            stopped: usize,
        }
        impl Host for Counting {
            fn drain_config(&self) -> DrainConfig {
                short()
            }
            fn stopped_accepting(&mut self) {
                self.stopped += 1;
            }
        }
        let dir = std::env::temp_dir().join(format!(
            "mush-drain-trigger-{}-{}",
            std::process::id(),
            rand_tag()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("draining-a");
        let drain = Drain::<Rec>::new();
        let s = Arc::new(Recorded::default());
        let guard = drain.enter(1);
        drain.register(1, Rec(Arc::clone(&s)));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sigs = Signals::install().unwrap();
        let mut host = Counting { stopped: 0 };
        tx.send(Trigger::Drain).unwrap();
        tx.send(Trigger::Drain).unwrap();
        // The session's close drops its guard, as a real transport's would.
        let closer = tokio::spawn(async move {
            while !s.is_closed() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            drop(guard);
            s
        });
        tokio::time::timeout(
            Duration::from_secs(10),
            run_with_trigger(&mut sigs, &mut rx, &drain, &marker, &mut host),
        )
        .await
        .unwrap();
        let s = closer.await.unwrap();
        assert_eq!(s.lines(), ["W soon"]);
        assert!(marker.exists(), "the drain leaves its marker");
        assert_eq!(host.stopped, 1, "the host hears it once");
        assert_eq!(drain.phase(), crate::drain::Phase::Draining);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A session whose close drops its own live guard there and then, as the
    /// gateway's does: it leaves the live set when its close is queued, long
    /// before the transport has written it.
    struct DropsOnClose(std::sync::Mutex<Option<crate::drain::LiveGuard<DropsOnClose>>>);

    impl crate::drain::Session for DropsOnClose {
        fn inject<'a>(&'a self, _line: &'a str) -> futures::future::BoxFuture<'a, ()> {
            Box::pin(async {})
        }
        fn close(&self) -> futures::future::BoxFuture<'_, ()> {
            Box::pin(async move {
                drop(self.0.lock().unwrap().take());
            })
        }
    }

    /// Run the lifecycle on one [`DropsOnClose`] session after `trigger`, and
    /// answer whether the close had finished, its flush window waited out,
    /// when the run returned. A loaded box can only make the run slower, so
    /// the answer can be wrongly yes, never wrongly no.
    async fn the_close_finished_before_the_run_returned(trigger: Trigger) -> bool {
        struct Quick;
        impl Host for Quick {
            fn drain_config(&self) -> DrainConfig {
                DrainConfig {
                    grace: Duration::ZERO,
                    warnings: vec![],
                    ..short()
                }
            }
        }
        let dir = std::env::temp_dir().join(format!(
            "mush-drain-flush-{}-{}",
            std::process::id(),
            rand_tag()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let drain = Drain::<DropsOnClose>::new();
        let guard = drain.enter(1);
        drain.register(1, DropsOnClose(std::sync::Mutex::new(Some(guard))));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sigs = Signals::install().unwrap();
        tx.send(trigger).unwrap();
        let started = tokio::time::Instant::now();
        tokio::time::timeout(
            Duration::from_secs(10),
            run_with_trigger(
                &mut sigs,
                &mut rx,
                &drain,
                &dir.join("draining-a"),
                &mut Quick,
            ),
        )
        .await
        .unwrap();
        let took = started.elapsed();
        std::fs::remove_dir_all(&dir).ok();
        took >= crate::drain::CLOSE_FLUSH
    }

    /// Doc 51 unit FL.7: a finish's close runs to its end before the run
    /// returns. The generation empties as the close is queued; ending on that
    /// cut the flush window, the gateway's runtime dropped with the close
    /// reason still unwritten, and the player never read why the line went.
    #[tokio::test]
    async fn a_finish_lets_its_close_flush_before_the_run_returns() {
        assert!(the_close_finished_before_the_run_returned(Trigger::Finish).await);
    }

    /// The same at a drain's deadline, where `Drain::run` raced its own
    /// close against the generation emptying.
    #[tokio::test]
    async fn a_drain_lets_its_close_flush_before_the_run_returns() {
        assert!(the_close_finished_before_the_run_returned(Trigger::Drain).await);
    }

    /// A closed trigger channel is not an event: the run keeps serving the
    /// signals (here: until a finish trigger sent before the close).
    #[tokio::test]
    async fn a_closed_trigger_channel_is_ignored() {
        struct Bare;
        impl Host for Bare {
            fn drain_config(&self) -> DrainConfig {
                short()
            }
        }
        let dir = std::env::temp_dir().join(format!(
            "mush-drain-trigger-closed-{}-{}",
            std::process::id(),
            rand_tag()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let drain = Drain::<Rec>::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sigs = Signals::install().unwrap();
        // Nothing live: a finish with no sessions ends the run at once, but
        // only once the finish arrives; the close of the channel before it
        // must not end (or spin) the run.
        let sender = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            tx.send(Trigger::Finish).unwrap();
            drop(tx);
        });
        let started = tokio::time::Instant::now();
        tokio::time::timeout(
            Duration::from_secs(10),
            run_with_trigger(
                &mut sigs,
                &mut rx,
                &drain,
                &dir.join("draining-a"),
                &mut Bare,
            ),
        )
        .await
        .unwrap();
        sender.await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(150));
        assert!(
            !dir.join("draining-a").exists(),
            "a finish leaves no marker"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    fn rand_tag() -> u64 {
        use std::hash::{BuildHasher, Hasher};
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    }

    #[test]
    fn a_host_with_nothing_to_reload_takes_the_defaults() {
        struct Bare;
        impl Host for Bare {
            fn drain_config(&self) -> DrainConfig {
                DrainConfig::default()
            }
        }
        let mut h = Bare;
        h.reload();
        h.reopen_logs();
        h.stopped_accepting();
        assert_eq!(h.drain_config(), DrainConfig::default());
    }

    /// Regression test (SSH-M5): even when the finish/drain task finishes
    /// immediately (before `run_with_trigger` re-enters `select!` — e.g. zero
    /// grace plus another concurrent event on the trigger channel), `hard_exit`
    /// still arms and exits after the hard-exit grace instead of wedging while
    /// a stubborn session holds its guard.
    #[tokio::test]
    async fn fast_finishing_work_still_arms_hard_exit() {
        struct ZeroGrace;
        impl Host for ZeroGrace {
            fn drain_config(&self) -> DrainConfig {
                DrainConfig {
                    deadline: Duration::ZERO,
                    warnings: vec![],
                    message: "W {remaining}".into(),
                    grace: Duration::ZERO,
                }
            }
        }
        let dir = std::env::temp_dir().join(format!(
            "mush-drain-hard-exit-{}-{}",
            std::process::id(),
            rand_tag()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let drain = Drain::<Rec>::new();
        let s = Arc::new(Recorded::default());
        // Hold the session guard across the entire run so `wait_empty` never
        // resolves; only `hard_exit` can terminate the lifecycle loop.
        let _stuck_guard = drain.enter(1);
        drain.register(1, Rec(Arc::clone(&s)));

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sigs = Signals::install().unwrap();
        tx.send(Trigger::Finish).unwrap();
        // Queue a duplicate trigger and close the channel so extra select
        // iterations run while the zero-grace task finishes immediately.
        tx.send(Trigger::Finish).unwrap();
        drop(tx);

        let hard_cap = Duration::from_millis(120);
        let started = tokio::time::Instant::now();
        tokio::time::timeout(
            Duration::from_secs(5),
            run_with_trigger_and_hard_exit(
                &mut sigs,
                &mut rx,
                &drain,
                &dir.join("draining-a"),
                &mut ZeroGrace,
                hard_cap,
            ),
        )
        .await
        .expect("hard_exit must arm even when work task finishes before select");
        assert!(started.elapsed() >= Duration::from_millis(80));
        assert!(s.is_closed());
        std::fs::remove_dir_all(&dir).ok();
    }
}
