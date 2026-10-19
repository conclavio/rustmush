//! Client → server messages: [`ClientCommand`] and the panel-hydration
//! [`Query`] surface.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::{ProjectedCommitToken, caps::ClientCaps};

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
    /// The command table as this viewer may use it (delivered as a `Panel`
    /// keyframe on the `commands.list` feed): name, aliases, switches, a
    /// usage line and a summary per command, filtered server-side by role
    /// and power, bounded, deterministic. Feeds the command palette
    /// (interface plan I1, D-I-5).
    Commands,
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
    /// Historical room-narrative lines the viewer already saw, newest-first
    /// from `before_seq` (interface plan I12). Answered at the gateway from
    /// the viewer's own resume ring — every line was already perceive/@ignore
    /// gated at delivery — and returned as a `Panel` delta on the `narrative`
    /// label, wrapped in a [`Reply`](crate::OutputEvent::Reply).
    NarrativeBacklog {
        /// Return lines with the per-session envelope `seq` strictly below
        /// this; `None` means start from the newest.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_seq: Option<u64>,
        /// Maximum number of lines to return (server-clamped).
        limit: u32,
    },
    /// Read this character's proving runs or one bounded report page. This is
    /// a closed read request; proving and bench actions remain commands.
    MindBulk {
        /// Owned proving run; absent means the bounded run list.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<u64>,
        /// One-based report page, valid only with a run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page: Option<u16>,
    },
    /// Read bounded public listings or this character's own original-mind offers.
    MarketOriginalMinds(MarketOriginalMindsRequest),
    /// Subscribe to a panel feed; the server responds with a keyframe and
    /// then streams deltas.
    PanelSubscribe {
        /// Panel feed id, e.g. `"wod.roll"`.
        feed: SmolStr,
    },
}

/// One bounded original-mind offer view. Ownership is always the attached character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum MarketOriginalMindsKind {
    /// Current public listings at the character's admitted venue.
    Listings,
    /// Only this character's listings and private quotes.
    Own,
}

/// Closed read selectors for the original-mind Market handler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MarketOriginalMindsRequest {
    /// Public venue listings or private offers owned by the attached character.
    pub kind: MarketOriginalMindsKind,
    /// Canonical 32-lowercase-hex publication cursor, not actor or account authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub after: Option<ProjectedCommitToken>,
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
    /// A closed module request, dispatched with normal mutation/persistence authority.
    Invoke {
        /// Client correlation id; asynchronous final results echo this id too.
        id: u64,
        /// Typed service request; never interpreted as command text.
        request: crate::InvokeRequest,
    },
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
    /// Structured-client acknowledgement of one projected authoritative
    /// result. This retires only bounded presentation retention; it does not
    /// alter authoritative state or the durable status/tombstone record.
    Ack {
        /// Exact token carried by the `resolution.outcome` view.
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        commit: ProjectedCommitToken,
    },
    /// The player started (`true`) or stopped (`false`) composing a line.
    /// The transport rate-limits it and the world fans it out to the room
    /// as [`OutputEvent::Composing`](crate::OutputEvent::Composing); it never
    /// stamps idle (interface plan I1, D-I-11).
    Composing {
        /// Whether a draft is in progress.
        active: bool,
    },
    /// A client's answer to a server
    /// [`Request`](crate::OutputEvent::Request) (interface plan I10),
    /// correlated by `id`. The client reports only the choice; the server
    /// owns the effect (D-I-21). A stale, forged or duplicate `id` is
    /// ignored by the server.
    Respond {
        /// Opaque lifetime of the engine that opened this request.
        #[serde(default)]
        epoch: String,
        /// The `id` from the originating request.
        id: u64,
        /// The viewer's choice.
        choice: crate::RespondChoice,
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
