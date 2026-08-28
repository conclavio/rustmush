//! Client → server messages: [`ClientCommand`] and the panel-hydration
//! [`Query`] surface.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::caps::ClientCaps;

/// A pull request for panel hydration, carried inside
/// [`ClientCommand::Req`].
///
/// Kept deliberately tiny: a MUSH is command-driven, and web UI buttons
/// mostly submit ordinary [`ClientCommand::Line`]s. `Req` exists only so
/// panels can populate without spamming scrollback (mail list on login,
/// channel backlog on tab open, …).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum Query {
    /// Mail summaries for one folder.
    MailList {
        /// Folder name; `None` means the default folder (inbox).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
    },
    /// The WHO listing, as an [`OutputEvent::Who`](crate::OutputEvent::Who).
    Who,
    /// The viewer's character sheet (delivered as a `Panel` keyframe on the
    /// active ruleset's sheet feed).
    Sheet,
    /// Historical channel messages, newest-first from `before_seq`.
    ChannelBacklog {
        /// Channel name.
        channel: SmolStr,
        /// Return messages with `seq` strictly below this; `None` means
        /// start from the newest.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_seq: Option<u64>,
        /// Maximum number of messages to return.
        limit: u32,
    },
    /// Subscribe to a panel feed; the server responds with a keyframe and
    /// then streams deltas.
    PanelSubscribe {
        /// Panel feed id, e.g. `"wod.roll"`.
        feed: SmolStr,
    },
}

/// Everything a client can say to the server.
///
/// Adjacently tagged on the wire, same scheme as
/// [`OutputEvent`](crate::OutputEvent): `{"t": "line", "d": "look"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum ClientCommand {
    /// A command line as typed — THE primary input for both transports.
    /// SSH lines arrive here after line editing; the web input box submits
    /// here too, keeping both clients behind one parser.
    Line(String),
    /// A panel-hydration request; the response correlates by `id`.
    Req {
        /// Client-chosen correlation id, echoed with the response.
        id: u64,
        /// What to fetch.
        q: Query,
    },
    /// Web reconnect: replay everything after `last_seq` from the session's
    /// resume buffer (or send fresh snapshots when the gap is too large).
    Resume {
        /// The last envelope sequence number the client received.
        last_seq: u64,
    },
    /// Keepalive response to a server
    /// [`Ping`](crate::OutputEvent::Ping).
    Pong {
        /// The nonce from the server's ping.
        nonce: u64,
    },
    /// Declare or update this connection's capabilities. The web transport
    /// sends it on connect and again on viewport resize; SSH never sends it
    /// (the server derives caps from the PTY request and window-change).
    Caps(ClientCaps),
}
