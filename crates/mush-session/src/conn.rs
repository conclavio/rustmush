//! Per-connection state: [`ConnId`], the output [`Sink`], and
//! [`ConnectionHandle`].

use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant};

use mush_proto::{ClientCaps, Envelope, OutputEvent};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

use crate::{read_locked, write_locked};

/// Identifies one physical transport (an SSH channel or a WebSocket).
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

/// A connection's output channel — which of the two renders it receives
/// (treaty ruling 1: one semantic event, two renders).
///
/// The sink kind is fixed at connect time; it is the routing discriminator
/// [`SessionManager`](crate::SessionManager) fans out on. Capabilities
/// ([`ClientCaps::structured`]) describe the same fact for renderers, but
/// the sink is what actually types the wire.
#[derive(Clone, Debug)]
pub enum Sink {
    /// ANSI-rendering transport (SSH): receives plain [`OutputEvent`]s and
    /// renders them through `mush-ansi`. Never sees envelopes.
    Ansi(mpsc::Sender<Arc<OutputEvent>>),
    /// SSH with sequence metadata for live/history deduplication; no receipt ACKs.
    AnsiStamped(mpsc::Sender<Arc<Envelope>>),
    /// Structured transport (web): receives seq/ts-stamped [`Envelope`]s
    /// (serialized as JSON on the wire). Never sees bare events.
    Structured(mpsc::Sender<Arc<Envelope>>),
}

/// The session layer's handle to one live connection. The transport task
/// holds the receiving end of the sink and owns the socket; this handle is
/// what [`SessionManager`](crate::SessionManager) fans output out through.
///
/// Idle bookkeeping: `last_input` is stored as **milliseconds since
/// `connected_at`** in an `AtomicI64`, so recording input is a lock-free
/// store and WHO can reconstruct an `Instant` exactly.
#[derive(Debug)]
pub struct ConnectionHandle {
    output_brackets: RwLock<mush_proto::OutputBrackets>,
    reply_serial: Arc<tokio::sync::Mutex<()>>,
    /// Connection id (allocated by the transport layer; unique per process).
    pub id: ConnId,
    /// Peer address.
    pub addr: IpAddr,
    /// Negotiated client capabilities; updated in place on terminal resize.
    pub caps: RwLock<ClientCaps>,
    /// Bounded output queue to the transport task, typed by transport kind.
    /// All delivery is non-blocking `try_send`; see the slow-client policy
    /// in the crate docs.
    sink: Sink,
    /// When the transport finished connecting (epoch for `last_input`).
    pub connected_at: Instant,
    /// Milliseconds since `connected_at` of the most recent input line.
    last_input: AtomicI64,
    /// Set when a send found the output buffer full (or the transport gone);
    /// the driver may disconnect the connection as a slow client.
    output_dropped: AtomicBool,
}

impl ConnectionHandle {
    /// New ANSI (SSH) handle connected "now". (The historical constructor —
    /// existing SSH call sites keep working unchanged.)
    pub fn new(
        id: ConnId,
        addr: IpAddr,
        caps: ClientCaps,
        tx: mpsc::Sender<Arc<OutputEvent>>,
    ) -> Self {
        Self::new_with_sink_at(id, addr, caps, Sink::Ansi(tx), Instant::now())
    }

    /// New ANSI (SSH) handle with an explicit connect instant
    /// (deterministic tests, drivers that stamp their own clock).
    pub fn new_at(
        id: ConnId,
        addr: IpAddr,
        caps: ClientCaps,
        tx: mpsc::Sender<Arc<OutputEvent>>,
        connected_at: Instant,
    ) -> Self {
        Self::new_with_sink_at(id, addr, caps, Sink::Ansi(tx), connected_at)
    }

    /// New structured (web) handle connected "now": receives [`Envelope`]s.
    pub fn new_structured(
        id: ConnId,
        addr: IpAddr,
        caps: ClientCaps,
        tx: mpsc::Sender<Arc<Envelope>>,
    ) -> Self {
        Self::new_with_sink_at(id, addr, caps, Sink::Structured(tx), Instant::now())
    }

    /// New structured (web) handle with an explicit connect instant.
    pub fn new_structured_at(
        id: ConnId,
        addr: IpAddr,
        caps: ClientCaps,
        tx: mpsc::Sender<Arc<Envelope>>,
        connected_at: Instant,
    ) -> Self {
        Self::new_with_sink_at(id, addr, caps, Sink::Structured(tx), connected_at)
    }

    /// New handle over any [`Sink`] with an explicit connect instant.
    pub fn new_with_sink_at(
        id: ConnId,
        addr: IpAddr,
        caps: ClientCaps,
        sink: Sink,
        connected_at: Instant,
    ) -> Self {
        Self {
            output_brackets: RwLock::new(Default::default()),
            reply_serial: Arc::new(tokio::sync::Mutex::new(())),
            id,
            addr,
            caps: RwLock::new(caps),
            sink,
            connected_at,
            // Connect counts as input: idle is measured from connect until
            // the first real line.
            last_input: AtomicI64::new(0),
            output_dropped: AtomicBool::new(false),
        }
    }

    /// Current delimiters for this physical connection; never copied to another login.
    pub fn output_brackets(&self) -> mush_proto::OutputBrackets {
        read_locked(&self.output_brackets, "reply brackets").clone()
    }

    /// Replace the row validated by the server's command dispatcher.
    pub fn set_output_brackets(&self, brackets: mush_proto::OutputBrackets) {
        *write_locked(&self.output_brackets, "reply brackets") = brackets;
    }

    /// Serialize typed replies until their complete body has been delivered.
    pub async fn reply_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.reply_serial.lock().await
    }

    /// An owned reply guard for a transport-local command.
    pub async fn owned_reply_lock(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.reply_serial.clone().lock_owned().await
    }

    /// Whether this connection's sink is [`Sink::Structured`] (web).
    pub fn is_structured(&self) -> bool {
        matches!(self.sink, Sink::Structured(_))
    }

    /// Whether this sink needs envelope metadata, independently of receipt ACK support.
    pub fn uses_envelopes(&self) -> bool {
        matches!(self.sink, Sink::Structured(_) | Sink::AnsiStamped(_))
    }

    /// Current capabilities (copy).
    pub fn caps(&self) -> ClientCaps {
        *read_locked(&self.caps, "caps")
    }

    /// Replace capabilities (terminal resize, renegotiation).
    pub fn set_caps(&self, caps: ClientCaps) {
        *write_locked(&self.caps, "caps") = caps;
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

    /// Non-blocking send of a plain event to an **ANSI** connection. On a
    /// full or closed buffer the *dropped* flag is set and `false` is
    /// returned; the event is discarded for this connection (the caller may
    /// disconnect it — see the crate docs).
    ///
    /// Calling this on a structured connection is a caller bug (structured
    /// sinks carry envelopes): the event is discarded with an error log,
    /// `false` is returned, and the dropped flag is **not** set — a routing
    /// mistake must not read as a slow client.
    pub fn try_send(&self, ev: Arc<OutputEvent>) -> bool {
        match &self.sink {
            Sink::Ansi(tx) => match tx.try_send(ev) {
                Ok(()) => true,
                Err(err) => {
                    self.note_send_failure(matches!(err, TrySendError::Full(_)));
                    false
                }
            },
            Sink::Structured(_) | Sink::AnsiStamped(_) => {
                tracing::error!(
                    conn = self.id.0,
                    "bare OutputEvent sent to a structured connection; \
                     stamp an Envelope instead (caller bug)"
                );
                false
            }
        }
    }

    /// Non-blocking send of a stamped envelope to a **structured**
    /// connection; flag semantics identical to [`try_send`](Self::try_send).
    /// Calling this on an ANSI connection is the mirror-image caller bug
    /// and is handled the same way.
    pub fn try_send_envelope(&self, env: Arc<Envelope>) -> bool {
        match &self.sink {
            Sink::Structured(tx) | Sink::AnsiStamped(tx) => match tx.try_send(env) {
                Ok(()) => true,
                Err(err) => {
                    self.note_send_failure(matches!(err, TrySendError::Full(_)));
                    false
                }
            },
            Sink::Ansi(_) => {
                tracing::error!(
                    conn = self.id.0,
                    "Envelope sent to an ANSI connection; \
                     send the plain OutputEvent instead (caller bug)"
                );
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

    /// Set the dropped flag; log the first failure (full vs. closed).
    fn note_send_failure(&self, full: bool) {
        let already = self.output_dropped.swap(true, Ordering::Relaxed);
        if !already {
            if full {
                tracing::warn!(conn = self.id.0, "output buffer full; slow client flagged");
            } else {
                tracing::warn!(
                    conn = self.id.0,
                    "output channel closed; connection flagged"
                );
            }
        }
    }

    fn millis_since_connect(&self, now: Instant) -> i64 {
        let ms = now.saturating_duration_since(self.connected_at).as_millis();
        i64::try_from(ms).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poison::tests::poison_rwlock;
    use mush_proto::{ClientKind, ColorDepth};

    fn caps(width: u16) -> ClientCaps {
        ClientCaps {
            kind: ClientKind::Ssh,
            color: ColorDepth::Ansi256,
            width,
            height: 24,
            utf8: true,
            structured: false,
        }
    }

    fn handle() -> ConnectionHandle {
        let (tx, _rx) = mpsc::channel(4);
        ConnectionHandle::new(ConnId(1), IpAddr::from([127, 0, 0, 1]), caps(80), tx)
    }

    /// The failure this converts away: a panic while a resize held the caps
    /// lock used to make `caps()` — read on every render for *this*
    /// connection — panic in turn, on a task that owns live SSH channels.
    /// Recovery keeps the last capabilities the panicking writer left, and the
    /// next renegotiation still replaces them.
    #[test]
    fn a_poisoned_caps_lock_still_reads_and_writes() {
        let conn = handle();
        conn.set_caps(caps(132));

        poison_rwlock(&conn.caps);

        assert_eq!(
            conn.caps().width,
            132,
            "the capabilities the panicking task left are still readable"
        );
        conn.set_caps(caps(80));
        assert_eq!(conn.caps().width, 80, "and still replaceable");
    }
}
