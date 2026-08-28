//! Per-connection state: [`ConnId`] and [`ConnectionHandle`].

use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant};

use mush_proto::{ClientCaps, OutputEvent};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

/// Identifies one physical transport (an SSH channel or, later, a WebSocket).
///
/// Newtype over `u64`. `mush-core` uses a bare `type ConnId = u64` alias for
/// the same value; `mushd` bridges with `.0` / `ConnId(n)` (this crate must
/// not depend on `mush-core`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnId(pub u64);

impl fmt::Display for ConnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "conn#{}", self.0)
    }
}

/// The session layer's handle to one live connection. The transport task
/// holds the receiving end of `tx` and owns the socket; this handle is what
/// [`SessionManager`](crate::SessionManager) fans output out through.
///
/// Idle bookkeeping: `last_input` is stored as **milliseconds since
/// `connected_at`** in an `AtomicI64`, so recording input is a lock-free
/// store and WHO can reconstruct an `Instant` exactly.
#[derive(Debug)]
pub struct ConnectionHandle {
    /// Connection id (allocated by the transport layer; unique per process).
    pub id: ConnId,
    /// Peer address.
    pub addr: IpAddr,
    /// Negotiated client capabilities; updated in place on terminal resize.
    pub caps: RwLock<ClientCaps>,
    /// Bounded output queue to the transport task. All delivery is
    /// non-blocking `try_send`; see the slow-client policy in the crate docs.
    pub tx: mpsc::Sender<Arc<OutputEvent>>,
    /// When the transport finished connecting (epoch for `last_input`).
    pub connected_at: Instant,
    /// Milliseconds since `connected_at` of the most recent input line.
    last_input: AtomicI64,
    /// Set when a send found the output buffer full (or the transport gone);
    /// the driver may disconnect the connection as a slow client.
    output_dropped: AtomicBool,
}

impl ConnectionHandle {
    /// New handle connected "now".
    pub fn new(
        id: ConnId,
        addr: IpAddr,
        caps: ClientCaps,
        tx: mpsc::Sender<Arc<OutputEvent>>,
    ) -> Self {
        Self::new_at(id, addr, caps, tx, Instant::now())
    }

    /// New handle with an explicit connect instant (deterministic tests,
    /// drivers that stamp their own clock).
    pub fn new_at(
        id: ConnId,
        addr: IpAddr,
        caps: ClientCaps,
        tx: mpsc::Sender<Arc<OutputEvent>>,
        connected_at: Instant,
    ) -> Self {
        Self {
            id,
            addr,
            caps: RwLock::new(caps),
            tx,
            connected_at,
            // Connect counts as input: idle is measured from connect until
            // the first real line.
            last_input: AtomicI64::new(0),
            output_dropped: AtomicBool::new(false),
        }
    }

    /// Current capabilities (copy).
    pub fn caps(&self) -> ClientCaps {
        *self.caps.read().expect("caps lock poisoned")
    }

    /// Replace capabilities (terminal resize, renegotiation).
    pub fn set_caps(&self, caps: ClientCaps) {
        *self.caps.write().expect("caps lock poisoned") = caps;
    }

    /// Record an input line arriving at `now` (idle bookkeeping).
    pub fn record_input_at(&self, now: Instant) {
        self.last_input
            .store(self.millis_since_connect(now), Ordering::Relaxed);
    }

    /// Instant of the most recent input (or `connected_at` if none yet).
    pub fn last_input_at(&self) -> Instant {
        let ms = self.last_input.load(Ordering::Relaxed);
        self.connected_at + Duration::from_millis(u64::try_from(ms.max(0)).unwrap_or(0))
    }

    /// How long this connection has been idle as of `now`.
    pub fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_input_at())
    }

    /// Non-blocking send. On a full or closed buffer the *dropped* flag is
    /// set and `false` is returned; the event is discarded for this
    /// connection (the caller may disconnect it — see the crate docs).
    pub fn try_send(&self, ev: Arc<OutputEvent>) -> bool {
        match self.tx.try_send(ev) {
            Ok(()) => true,
            Err(err) => {
                let already = self.output_dropped.swap(true, Ordering::Relaxed);
                if !already {
                    match err {
                        TrySendError::Full(_) => {
                            tracing::warn!(
                                conn = self.id.0,
                                "output buffer full; slow client flagged"
                            );
                        }
                        TrySendError::Closed(_) => {
                            tracing::warn!(
                                conn = self.id.0,
                                "output channel closed; connection flagged"
                            );
                        }
                    }
                }
                false
            }
        }
    }

    /// Whether output has been dropped on this connection (slow client).
    pub fn output_dropped(&self) -> bool {
        self.output_dropped.load(Ordering::Relaxed)
    }

    /// Clear the dropped flag (a driver that chooses to forgive).
    pub fn clear_output_dropped(&self) {
        self.output_dropped.store(false, Ordering::Relaxed);
    }

    fn millis_since_connect(&self, now: Instant) -> i64 {
        let ms = now.saturating_duration_since(self.connected_at).as_millis();
        i64::try_from(ms).unwrap_or(i64::MAX)
    }
}
