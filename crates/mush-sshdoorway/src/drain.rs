//! The doorway's session as the shared drain sees it (doc 33 §4.5, cut GG.3).
//!
//! The mechanism — the registry, the schedule, the warnings, the deadline and
//! the finish — is [`mush_drain`]'s, lifted out of here with no behaviour
//! change. This module says what a doorway session is to it: a russh
//! [`Handle`], the channel it is on, and the inject queue its pump reads, so
//! a warning lands between upstream chunks and never inside an escape
//! sequence. Before a spliced channel exists (the menu, a notice) the warning
//! goes straight onto the channel; before any channel exists, nowhere, but
//! the session is still closed at the deadline.

use futures::future::BoxFuture;
use russh::server::Handle;
use russh::{ChannelId, Disconnect};
use tokio::sync::mpsc;

pub use mush_drain::drain::{Phase, Warning, schedule};

/// How many injected lines may wait for one session's pump.
pub const INJECT_QUEUE: usize = 8;

/// The disconnect reason the drain closes a session with.
pub const CLOSE_REASON: &str = "the doorway is restarting; reconnect any time";

/// This generation's lifecycle and session registry.
pub type Drain = mush_drain::Drain<DoorwaySession>;

/// Held by every live connection; the generation may exit when none remain.
pub type LiveGuard = mush_drain::LiveGuard<DoorwaySession>;

/// A registered session's reachable parts.
pub struct DoorwaySession {
    /// Lines for the session's pump to write between chunks; `None` before a
    /// spliced channel exists (menu, notice).
    inject: Option<mpsc::Sender<String>>,
    /// Writes directly when there is no pump.
    handle: Handle,
    channel: Option<ChannelId>,
}

impl std::fmt::Debug for DoorwaySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoorwaySession")
            .field("channel", &self.channel)
            .field("pumped", &self.inject.is_some())
            .finish_non_exhaustive()
    }
}

impl DoorwaySession {
    /// What the drain can reach of one session: its handle, the channel to
    /// write on when there is no pump, and the pump's inject queue when there
    /// is one.
    #[must_use]
    pub fn new(
        handle: Handle,
        channel: Option<ChannelId>,
        inject: Option<mpsc::Sender<String>>,
    ) -> DoorwaySession {
        DoorwaySession {
            inject,
            handle,
            channel,
        }
    }
}

impl mush_drain::Session for DoorwaySession {
    /// Through the pump when the session has one, else straight onto its
    /// channel.
    fn inject<'a>(&'a self, line: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            match (&self.inject, self.channel) {
                (Some(tx), _) => {
                    // A full queue means the session is not reading; it
                    // will be closed at the deadline regardless.
                    let _ = tx.try_send(line.to_owned());
                }
                (None, Some(ch)) => {
                    let mut bytes = b"\r\n".to_vec();
                    bytes.extend_from_slice(line.as_bytes());
                    bytes.extend_from_slice(b"\r\n");
                    let _ = self.handle.data(ch, bytes).await;
                }
                (None, None) => {}
            }
        })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let _ = self
                .handle
                .disconnect(
                    Disconnect::ByApplication,
                    CLOSE_REASON.to_owned(),
                    String::new(),
                )
                .await;
        })
    }
}
