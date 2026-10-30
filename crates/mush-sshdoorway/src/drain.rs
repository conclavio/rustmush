//! Draining a generation (doc 29 §9).
//!
//! Established SSH sessions cannot move between processes (their keys and
//! sequence numbers live in this process's memory), so a restart moves only
//! *new* connections: the old generation stops accepting and keeps serving
//! until its sessions end or the deadline passes. Along the way every live
//! session is warned on the owner's schedule (Q-29-2: 12h, 6h, 2h, 1h, 15m
//! and 1m remaining), with the owner's wording.
//!
//! The registry here is what the warnings and the deadline reach: one entry
//! per session that has a channel, holding the inject queue its pump reads
//! (so a warning lands between upstream chunks, never inside an escape
//! sequence) and the session handle that closes it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::server::Handle;
use russh::{ChannelId, Disconnect};
use tokio::sync::{Notify, mpsc, watch};

use crate::config::Warning;

/// How many injected lines may wait for one session's pump.
pub const INJECT_QUEUE: usize = 8;

/// The generation's lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Accepting and serving.
    Serving,
    /// Not accepting; serving the sessions it has (SIGTERM).
    Draining,
}

/// A registered session.
struct Entry {
    /// Lines for the session's pump to write between chunks; `None` before a
    /// spliced channel exists (menu, notice).
    inject: Option<mpsc::Sender<String>>,
    /// Writes directly when there is no pump.
    handle: Handle,
    channel: Option<ChannelId>,
}

/// The registry and lifecycle of this generation.
pub struct Drain {
    phase: watch::Sender<Phase>,
    sessions: Mutex<HashMap<u64, Entry>>,
    live: AtomicU64,
    empty: Notify,
}

impl std::fmt::Debug for Drain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drain")
            .field("phase", &*self.phase.borrow())
            .field("live", &self.live.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl Default for Drain {
    fn default() -> Self {
        Drain {
            phase: watch::channel(Phase::Serving).0,
            sessions: Mutex::new(HashMap::new()),
            live: AtomicU64::new(0),
            empty: Notify::new(),
        }
    }
}

/// Held by every live connection; the generation may exit when none remain.
#[derive(Debug)]
pub struct LiveGuard {
    drain: Arc<Drain>,
    id: u64,
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.drain.unregister(self.id);
        if self.drain.live.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.drain.empty.notify_waiters();
        }
    }
}

impl Drain {
    /// A serving generation.
    #[must_use]
    pub fn new() -> Arc<Drain> {
        Arc::new(Drain::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Entry>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Count one connection live.
    pub fn enter(self: &Arc<Self>, id: u64) -> LiveGuard {
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

    /// Register (or update) a session's reachable parts.
    pub fn register(
        &self,
        id: u64,
        handle: Handle,
        channel: Option<ChannelId>,
        inject: Option<mpsc::Sender<String>>,
    ) {
        self.lock().insert(
            id,
            Entry {
                inject,
                handle,
                channel,
            },
        );
    }

    fn unregister(&self, id: u64) {
        self.lock().remove(&id);
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

    /// Send one line to every registered session: through its pump when it
    /// has one, else straight onto its channel.
    pub async fn broadcast(&self, line: &str) {
        let targets: Vec<(Option<mpsc::Sender<String>>, Handle, Option<ChannelId>)> = self
            .lock()
            .values()
            .map(|e| (e.inject.clone(), e.handle.clone(), e.channel))
            .collect();
        for (inject, handle, channel) in targets {
            match (inject, channel) {
                (Some(tx), _) => {
                    // A full queue means the session is not reading; it will
                    // be closed at the deadline regardless.
                    let _ = tx.try_send(line.to_owned());
                }
                (None, Some(ch)) => {
                    let mut bytes = b"\r\n".to_vec();
                    bytes.extend_from_slice(line.as_bytes());
                    bytes.extend_from_slice(b"\r\n");
                    let _ =
                        tokio::time::timeout(Duration::from_secs(5), handle.data(ch, bytes)).await;
                }
                (None, None) => {}
            }
        }
    }

    /// Disconnect every registered session.
    pub async fn close_all(&self, reason: &str) {
        let handles: Vec<Handle> = self.lock().values().map(|e| e.handle.clone()).collect();
        for h in handles {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                h.disconnect(Disconnect::ByApplication, reason.to_owned(), String::new()),
            )
            .await;
        }
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
            self.close_all("the doorway is restarting; reconnect any time")
                .await;
        };
        tokio::select! {
            () = work => {}
            () = self.wait_empty() => {}
        }
    }

    /// `SIGUSR1`: one warning with `grace` remaining, then close everything.
    pub async fn finish_now(&self, grace: Duration, message: &str) {
        let label = format!("{}s", grace.as_secs());
        self.broadcast(&message.replace("{remaining}", &label))
            .await;
        tokio::select! {
            () = tokio::time::sleep(grace) => {}
            () = self.wait_empty() => return,
        }
        self.close_all("the doorway is restarting; reconnect any time")
            .await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn w(secs: u64, label: &str) -> Warning {
        Warning {
            remaining: Duration::from_secs(secs),
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
    fn warnings_past_the_deadline_are_skipped() {
        let s = schedule(Duration::from_secs(100), &[w(200, "x"), w(10, "y")]);
        assert_eq!(s, [(Duration::from_secs(90), "y".to_owned())]);
    }

    #[tokio::test]
    async fn live_counting_and_wait_empty() {
        let d = Drain::new();
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
    }
}
