//! Server → client events: [`OutputEvent`] and its payload types.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::text::RichText;

/// A wire-friendly reference to a game object: opaque id plus display name.
///
/// The id is the classic dbref string form (`"#123"`); clients treat it as
/// opaque (it keys click-to-run commands, unread cursors, etc.).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ObjRef {
    /// Opaque object id, e.g. `"#123"`.
    pub id: String,
    /// Display name at the time of the event.
    pub name: String,
}

impl ObjRef {
    /// Builds a reference from id and display name.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

/// The flavor of a narrative line — how the actor produced it.
///
/// Renderers use this to format the line (`Say` quotes, `Pose` prefixes the
/// name, …); it is shared by room narration and channel messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum NarrativeKind {
    /// `say` — quoted speech.
    Say,
    /// `pose` (`:`) — action prefixed with the actor's name and a space.
    Pose,
    /// semipose (`;`) — action prefixed with the actor's name, no space.
    SemiPose,
    /// `@emit` — free-form narration, no automatic actor prefix.
    Emit,
    /// `page` — private message.
    Page,
    /// OOC (out-of-character) chatter.
    Ooc,
    /// `@emit`-style text whose apparent source is not its true sender
    /// (spoof-tracking surfaces the real actor to staff).
    Spoof,
    /// `whisper` — quiet speech only nearby targets receive.
    Whisper,
}

/// One exit as shown in a room: primary name plus aliases (`north`/`n`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExitRef {
    /// Primary exit name, e.g. `"north"`.
    pub name: String,
    /// Alternative names that also match, e.g. `["n"]`.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// A full structured room render: `look`, movement, resume snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RoomView {
    /// Room display name.
    pub name: String,
    /// Room object id, e.g. `"#42"`.
    pub id: String,
    /// Room description.
    pub desc: RichText,
    /// Visible contents (players, NPCs, things).
    #[serde(default)]
    pub contents: Vec<ObjRef>,
    /// Visible exits.
    #[serde(default)]
    pub exits: Vec<ExitRef>,
    /// The zone this room belongs to, when visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ObjRef>,
}

/// One row of a WHO listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WhoEntry {
    /// Character name.
    pub name: String,
    /// Short alias, when the character has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Seconds since the player's last input.
    pub idle_s: u64,
    /// Seconds connected.
    pub conn_s: u64,
    /// Location name, when the viewer is allowed to see it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The player's `@doing` string, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
}

/// Whether a [`OutputEvent::Panel`] frame replaces panel state or patches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum PanelFrameKind {
    /// Full state snapshot; replaces everything the panel holds.
    Keyframe,
    /// Incremental update to apply on top of the last keyframe.
    Delta,
}

/// How the client should treat subsequent input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    /// Normal echoed line input.
    Normal,
    /// Input echoed as `*` (passwords).
    Masked,
    /// Input not echoed at all.
    NoEcho,
}

/// Input-mode change plus an optional prompt to display.
///
/// Modules may set a rich prompt (dice pools, compose mode, chargen steps);
/// `text: None` keeps the client's default prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PromptSpec {
    /// The input mode to switch to.
    pub mode: InputMode,
    /// Prompt text to display, or `None` for the client default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<RichText>,
}

/// Everything the server can say to a client — the one semantic event
/// vocabulary (treaty ruling 1).
///
/// Adjacently tagged on the wire: `{"t": "narrative", "d": {...}}`. The ANSI
/// renderer turns each event into scrollback text; the web client routes
/// panel-oriented events (`Channel`, `Who`, `Panel`, …) to native panels
/// *and/or* scrollback. One event, two renders — the core never emits twice.
///
/// Clients MUST ignore events whose `t` they do not recognize (additive
/// evolution).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum OutputEvent {
    // -- narrative stream --
    /// A full room render (look, movement, resume snapshot).
    Room(RoomView),
    /// A narrative line: say/pose/emit/page/… with optional attributed actor.
    Narrative {
        /// How the line was produced (formats the render).
        kind: NarrativeKind,
        /// The acting object, when the line is attributed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<ObjRef>,
        /// The line itself.
        text: RichText,
    },
    /// Server/system messages and errors.
    Sys(RichText),

    // -- panel-oriented (web panels; SSH renders a text fallback) --
    /// A channel message. Carries the channel's own monotonically increasing
    /// `seq` so clients can keep per-channel unread cursors and request
    /// backlog ([`Query::ChannelBacklog`](crate::Query::ChannelBacklog)).
    Channel {
        /// Channel name, e.g. `"pub"`.
        channel: SmolStr,
        /// Per-channel message sequence number (monotonic).
        seq: u64,
        /// The speaker.
        actor: ObjRef,
        /// Comsys title, when the speaker has one set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Say/pose/emit flavor of the message.
        kind: NarrativeKind,
        /// The message text.
        text: RichText,
        /// Server timestamp, Unix milliseconds.
        ts: i64,
    },
    /// New-mail notification.
    MailNotice {
        /// The sender.
        from: ObjRef,
        /// Message subject.
        subject: String,
        /// Mail message id.
        id: u64,
        /// The recipient's unread count after this message.
        unread: u32,
    },
    /// A WHO listing (in response to `WHO` or a who-panel refresh).
    Who(Vec<WhoEntry>),
    /// A module panel frame: opaque payload routed by `feed` id (e.g.
    /// `"wod.roll"`). The web client dispatches to the panel component
    /// registered for the feed; the ANSI renderer calls the module's
    /// registered text-fallback renderer.
    Panel {
        /// Panel feed id, e.g. `"wod.roll"`, `"btech.map"`.
        feed: SmolStr,
        /// Snapshot vs. incremental frame.
        kind: PanelFrameKind,
        /// Module-defined payload; opaque to this crate.
        data: serde_json::Value,
    },
    /// Ambient weather line for the zone the character is in.
    Weather {
        /// Zone identifier.
        zone: SmolStr,
        /// The weather flavor line.
        text: RichText,
        /// Icon hint for the web client, e.g. `"rain"`.
        icon: SmolStr,
    },

    // -- request/response correlation --
    /// The correlated response to a [`ClientCommand::Req`]: wraps the
    /// payload event and echoes the request's `id`, so a `MailList` reply
    /// or a `ChannelBacklog` batch is distinguishable from live events.
    ///
    /// [`ClientCommand::Req`]: crate::ClientCommand::Req
    Reply {
        /// The `id` from the originating request.
        id: u64,
        /// The payload (e.g. a `Who` listing or a `Panel` keyframe).
        event: Box<OutputEvent>,
    },

    // -- session control --
    /// The server's answer to a
    /// [`ClientCommand::Resume`](crate::ClientCommand::Resume), sent before
    /// any replayed or fresh events.
    ///
    /// `replayed: true` — every event after the client's `last_seq` follows
    /// in order; the client keeps its local state. `replayed: false` — the
    /// gap exceeded the session's resume buffer (the client's `last_seq` is
    /// too old): the client must discard local scrollback/panel state and
    /// treat what follows (fresh `Room` + panel snapshots) as a full
    /// re-hydration.
    Resumed {
        /// Whether the gap could be replayed from the resume buffer.
        replayed: bool,
    },
    /// Input-mode change / prompt update.
    Prompt(PromptSpec),
    /// Server keepalive; the client answers with
    /// [`ClientCommand::Pong`](crate::ClientCommand::Pong) carrying the
    /// same nonce.
    Ping {
        /// Nonce to echo back in the `Pong`.
        nonce: u64,
    },
    /// First frame after connect: capability negotiation.
    Hello {
        /// Protocol version ([`PROTO_VERSION`](crate::PROTO_VERSION)).
        proto: u32,
        /// Server software and version, e.g. `"rustmush 0.1"`.
        server: String,
        /// Feature identifiers the server supports, e.g. `"mail"`,
        /// `"channels"`, `"wod"`.
        features: Vec<String>,
        /// Panel feed ids the server can publish.
        #[serde(default)]
        panels: Vec<String>,
    },
    /// Sent after attach: who and what this session is.
    SessionInfo {
        /// The attached character.
        character: ObjRef,
        /// Account name.
        account: String,
        /// Number of live connections attached to this session.
        connections: u8,
    },
    /// The server is closing this connection.
    Disconnect {
        /// Human-readable reason.
        reason: RichText,
    },
}
