//! Client → server messages: [`ClientCommand`] and the panel-hydration
//! [`Query`] surface.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::{ProjectedCommitToken, caps::ClientCaps};

/// **Which picture a web [`ClientCommand::Picture`] asks for** (doc 21 cut
/// G.12a, §7.4): the gateway draws the first-person cockpit or the overhead
/// from the same two feeds, at the same clock and size. `cockpit` is the
/// default, so a command without the field is the cockpit it always was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum PictureKind {
    /// The first-person cockpit picture (G.3).
    #[default]
    Cockpit,
    /// The arena north-up around the tank (G.12b's overhead scene).
    Overhead,
}

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
    /// 17g cut C3: one page of the player market's listings with the
    /// character's own quotes and waiting purchases (feed `mind.market`), or
    /// one section of the character's own mind holdings (feed
    /// `mind.holdings`), each answered as a `Panel` keyframe on its feed.
    MindMarket(MindMarketRequest),
    /// Subscribe to a panel feed; the server responds with a keyframe and
    /// then streams deltas.
    PanelSubscribe {
        /// Panel feed id, e.g. `"wod.roll"`.
        feed: SmolStr,
        /// **A pulled feed's query parameters** (doc 20 cut SM.9; the brief's 3.8, decision
        /// D2): an object — `site.map`'s `view`/`rig`/`family`, `body.ground`'s
        /// `body`/`family` — forwarded as the feed query's arguments; absent is `{}`, and a
        /// subscribe without it is byte-identical on the wire to one before the field existed.
        /// The gateway refuses a non-object or one past its size bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params: Option<serde_json::Value>,
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

/// 17g cut C3: the two closed panel reads of the player market. Each kind is
/// its feed's query one-for-one; nothing names an actor, an account or a
/// token (the listing cursor is a code).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindMarketRequest {
    /// One page of current listings and the actor's own quotes and waiting
    /// purchases (feed `mind.market`); `after` is the last code of the
    /// previous page, as `+minds/next <code>` pages.
    Listings {
        /// The last code of the previous page; absent is the first page.
        #[cfg_attr(feature = "ts", ts(type = "string | null"))]
        after: Option<crate::ListingCode>,
    },
    /// One section of the actor's holdings (feed `mind.holdings`); `after`
    /// is 17g cut C2's `OwnHoldings` cursor (an asset id for `owner`, a
    /// licence number for `paid`).
    Holdings {
        /// Which section.
        section: MindHoldingsSection,
        /// The previous page's cursor; absent is the first page.
        after: Option<u64>,
    },
}

/// 17g cut C3: the two sections of `+minds/holdings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum MindHoldingsSection {
    /// The filed minds the actor owns (owner and dependent licences).
    Owner,
    /// The paid licences the actor holds.
    Paid,
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
    /// The web client's pixel view (doc 21 cut G.3): ask the gateway for the
    /// first-person picture of the cockpit, change its look, or stop it.
    /// While it is on, the gateway sends binary WebSocket messages, each one
    /// PNG frame (640x480) drawn from this connection's own `arena.cockpit`
    /// and `arena.map` keyframes, only when the picture changed and at most
    /// at the picture's clock (20 a second). Handled by the web transport; it
    /// never reaches the world.
    Picture {
        /// Whether the picture is wanted.
        on: bool,
        /// The look: the camera's offset from the tank's heading,
        /// millidegrees. A look, not a turn: turning the tank is a
        /// `HEADING` line like any other.
        #[serde(default)]
        look_mdeg: i64,
        /// Which camera (doc 21 cut G.12a, §7.4): the first-person cockpit,
        /// or the overhead the `v` key swaps to. Absent reads `cockpit`, so
        /// a client from before the overhead asks for what it always did.
        #[serde(default)]
        kind: PictureKind,
    },
    /// Declare or update this connection's capabilities. The web transport
    /// sends it on connect and again on viewport resize; SSH never sends it
    /// (the server derives caps from the PTY request and window-change).
    Caps(ClientCaps),
}
