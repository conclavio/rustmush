//! The server → client WebSocket envelope.

use serde::{Deserialize, Serialize};

use crate::event::OutputEvent;

/// One server → client frame on the web wire.
///
/// The session layer stamps `seq` (per-session, monotonic — drives resume
/// replay) and `ts`; the event's own `t`/`d` fields flatten in beside them,
/// so the wire shape is:
///
/// ```json
/// {"seq": 4212, "ts": 1756300000123, "t": "narrative", "d": {...}}
/// ```
///
/// SSH connections never see envelopes — the ANSI renderer consumes
/// [`OutputEvent`] directly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Envelope {
    /// Per-session monotonic sequence number, stamped by the session layer.
    pub seq: u64,
    /// Server timestamp, Unix milliseconds.
    pub ts: i64,
    /// The event itself; its `t`/`d` fields flatten into this object.
    #[serde(flatten)]
    pub event: OutputEvent,
}

impl Envelope {
    /// Wraps an event with its sequence number and timestamp.
    pub fn new(seq: u64, ts: i64, event: OutputEvent) -> Self {
        Self { seq, ts, event }
    }
}
