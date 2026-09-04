//! Server → client events: [`OutputEvent`] and its payload types.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
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

/// Maximum UTF-8 bytes in a stable semantic-view identifier.
pub const MAX_VIEW_ID_BYTES: usize = 96;
/// Maximum serialized JSON bytes in one semantic-view frame.
pub const MAX_VIEW_DATA_BYTES: usize = 1024 * 1024;
/// Maximum bytes in a projected commit/deduplication token.
pub const MAX_VIEW_COMMIT_BYTES: usize = 128;
/// Largest integer represented exactly by every JSON/JavaScript consumer.
pub const MAX_VIEW_WIRE_INTEGER: u64 = (1_u64 << 53) - 1;

/// A client-side position in one versioned semantic view.
///
/// Cursors contain only public routing/version information. They never carry
/// a server-side projection scope, truth revision, or provider guard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ViewCursor {
    /// Stable view identifier, e.g. `"legal.actions"`.
    pub view: SmolStr,
    /// Payload schema understood at this cursor.
    pub schema_version: u32,
    /// View-stream generation. A changed generation invalidates deltas.
    pub generation: u64,
    /// Last applied frame sequence inside the generation.
    pub sequence: u64,
}

/// What a consumer may safely do with an incoming semantic-view frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewFrameDisposition {
    /// Replace all local state with this keyframe.
    Replace,
    /// Apply this delta to the exact cursor named by `base_sequence`.
    ApplyDelta,
    /// The frame was already applied or arrived behind the local cursor.
    IgnoreStale,
    /// Do not apply the frame; request a fresh keyframe.
    ResyncRequired,
}

/// A versioned, gap-detecting semantic view sent to structured consumers.
///
/// This is intentionally distinct from the legacy opaque [`OutputEvent::Panel`]
/// payload. A keyframe has no `base_sequence`; a delta names the exact cursor
/// it extends. `generation` is public projection metadata, not the private
/// server-side projection-scope identifier used to sign action offers.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ViewEnvelope {
    view: SmolStr,
    schema_version: u32,
    generation: u64,
    sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    data: serde_json::Value,
}

impl ViewEnvelope {
    /// Construct a full-state keyframe.
    pub fn keyframe(
        view: impl Into<SmolStr>,
        schema_version: u32,
        generation: u64,
        sequence: u64,
        data: serde_json::Value,
    ) -> Result<Self, ViewEnvelopeError> {
        Self::new(
            view.into(),
            schema_version,
            generation,
            sequence,
            None,
            None,
            data,
        )
    }

    /// Construct a delta extending exactly `base_sequence`.
    pub fn delta(
        view: impl Into<SmolStr>,
        schema_version: u32,
        generation: u64,
        sequence: u64,
        base_sequence: u64,
        data: serde_json::Value,
    ) -> Result<Self, ViewEnvelopeError> {
        Self::new(
            view.into(),
            schema_version,
            generation,
            sequence,
            Some(base_sequence),
            None,
            data,
        )
    }

    fn new(
        view: SmolStr,
        schema_version: u32,
        generation: u64,
        sequence: u64,
        base_sequence: Option<u64>,
        commit: Option<String>,
        data: serde_json::Value,
    ) -> Result<Self, ViewEnvelopeError> {
        let envelope = Self {
            view,
            schema_version,
            generation,
            sequence,
            base_sequence,
            commit,
            data,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    /// Attach the projected stable transaction token used for deduplication.
    pub fn with_commit(mut self, commit: impl Into<String>) -> Result<Self, ViewEnvelopeError> {
        self.commit = Some(commit.into());
        self.validate()?;
        Ok(self)
    }

    /// Validate invariants and frame bounds after construction or decoding.
    pub fn validate(&self) -> Result<(), ViewEnvelopeError> {
        validate_view_id(&self.view)?;
        if self.schema_version == 0 {
            return Err(ViewEnvelopeError::ZeroSchemaVersion);
        }
        if self.generation == 0 {
            return Err(ViewEnvelopeError::ZeroGeneration);
        }
        if self.generation > MAX_VIEW_WIRE_INTEGER {
            return Err(ViewEnvelopeError::IntegerTooLarge {
                field: "generation",
                value: self.generation,
                max: MAX_VIEW_WIRE_INTEGER,
            });
        }
        if self.sequence > MAX_VIEW_WIRE_INTEGER {
            return Err(ViewEnvelopeError::IntegerTooLarge {
                field: "sequence",
                value: self.sequence,
                max: MAX_VIEW_WIRE_INTEGER,
            });
        }
        if let Some(base) = self.base_sequence
            && base >= self.sequence
        {
            return Err(ViewEnvelopeError::InvalidDeltaSequence {
                base,
                sequence: self.sequence,
            });
        }
        if let Some(base) = self.base_sequence
            && base > MAX_VIEW_WIRE_INTEGER
        {
            return Err(ViewEnvelopeError::IntegerTooLarge {
                field: "base_sequence",
                value: base,
                max: MAX_VIEW_WIRE_INTEGER,
            });
        }
        if let Some(commit) = &self.commit {
            if commit.is_empty() {
                return Err(ViewEnvelopeError::EmptyCommit);
            }
            if commit.len() > MAX_VIEW_COMMIT_BYTES {
                return Err(ViewEnvelopeError::CommitTooLarge {
                    actual: commit.len(),
                    max: MAX_VIEW_COMMIT_BYTES,
                });
            }
        }
        let bytes = serde_json::to_vec(&self.data)
            .map_err(|_| ViewEnvelopeError::DataSerialization)?
            .len();
        if bytes > MAX_VIEW_DATA_BYTES {
            return Err(ViewEnvelopeError::DataTooLarge {
                actual: bytes,
                max: MAX_VIEW_DATA_BYTES,
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn view(&self) -> &str {
        &self.view
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn base_sequence(&self) -> Option<u64> {
        self.base_sequence
    }

    #[must_use]
    pub fn commit(&self) -> Option<&str> {
        self.commit.as_deref()
    }

    #[must_use]
    pub const fn data(&self) -> &serde_json::Value {
        &self.data
    }

    #[must_use]
    pub const fn is_keyframe(&self) -> bool {
        self.base_sequence.is_none()
    }

    /// Classify this frame against a consumer's supported schema and cursor.
    /// A caller updates its cursor only for `Replace` or `ApplyDelta`.
    #[must_use]
    pub fn disposition(
        &self,
        expected_view: &str,
        supported_schema_version: u32,
        current: Option<&ViewCursor>,
    ) -> ViewFrameDisposition {
        if self.validate().is_err()
            || self.view != expected_view
            || self.schema_version != supported_schema_version
        {
            return ViewFrameDisposition::ResyncRequired;
        }
        if self.is_keyframe() {
            return match current {
                Some(cursor)
                    if cursor.view == self.view
                        && cursor.schema_version == self.schema_version
                        && cursor.generation == self.generation
                        && cursor.sequence >= self.sequence =>
                {
                    ViewFrameDisposition::IgnoreStale
                }
                _ => ViewFrameDisposition::Replace,
            };
        }
        let Some(cursor) = current else {
            return ViewFrameDisposition::ResyncRequired;
        };
        if cursor.view != self.view
            || cursor.schema_version != self.schema_version
            || cursor.generation != self.generation
        {
            return ViewFrameDisposition::ResyncRequired;
        }
        if self.sequence <= cursor.sequence {
            return ViewFrameDisposition::IgnoreStale;
        }
        if self.base_sequence == Some(cursor.sequence) {
            ViewFrameDisposition::ApplyDelta
        } else {
            ViewFrameDisposition::ResyncRequired
        }
    }

    /// Cursor produced after safely applying this frame.
    #[must_use]
    pub fn cursor(&self) -> ViewCursor {
        ViewCursor {
            view: self.view.clone(),
            schema_version: self.schema_version,
            generation: self.generation,
            sequence: self.sequence,
        }
    }
}

#[derive(Deserialize)]
struct ViewEnvelopeWire {
    view: SmolStr,
    schema_version: u32,
    generation: u64,
    sequence: u64,
    #[serde(default)]
    base_sequence: Option<u64>,
    #[serde(default)]
    commit: Option<String>,
    data: serde_json::Value,
}

impl<'de> Deserialize<'de> for ViewEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ViewEnvelopeWire::deserialize(deserializer)?;
        Self::new(
            wire.view,
            wire.schema_version,
            wire.generation,
            wire.sequence,
            wire.base_sequence,
            wire.commit,
            wire.data,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn validate_view_id(view: &str) -> Result<(), ViewEnvelopeError> {
    if view.is_empty() {
        return Err(ViewEnvelopeError::EmptyView);
    }
    if view.len() > MAX_VIEW_ID_BYTES {
        return Err(ViewEnvelopeError::ViewTooLarge {
            actual: view.len(),
            max: MAX_VIEW_ID_BYTES,
        });
    }
    if let Some((offset, byte)) = view
        .bytes()
        .enumerate()
        .find(|(_, byte)| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    {
        return Err(ViewEnvelopeError::InvalidViewByte { offset, byte });
    }
    Ok(())
}

/// Checked semantic-view construction/decoding failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewEnvelopeError {
    EmptyView,
    ViewTooLarge {
        actual: usize,
        max: usize,
    },
    InvalidViewByte {
        offset: usize,
        byte: u8,
    },
    ZeroSchemaVersion,
    ZeroGeneration,
    IntegerTooLarge {
        field: &'static str,
        value: u64,
        max: u64,
    },
    InvalidDeltaSequence {
        base: u64,
        sequence: u64,
    },
    EmptyCommit,
    CommitTooLarge {
        actual: usize,
        max: usize,
    },
    DataSerialization,
    DataTooLarge {
        actual: usize,
        max: usize,
    },
}

impl fmt::Display for ViewEnvelopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyView => formatter.write_str("semantic view id is empty"),
            Self::ViewTooLarge { actual, max } => {
                write!(
                    formatter,
                    "semantic view id has {actual} bytes; maximum is {max}"
                )
            }
            Self::InvalidViewByte { offset, byte } => write!(
                formatter,
                "semantic view id has invalid byte 0x{byte:02x} at offset {offset}"
            ),
            Self::ZeroSchemaVersion => formatter.write_str("semantic view schema version is zero"),
            Self::ZeroGeneration => formatter.write_str("semantic view generation is zero"),
            Self::IntegerTooLarge { field, value, max } => write!(
                formatter,
                "semantic view {field} value {value} exceeds exact JSON integer maximum {max}"
            ),
            Self::InvalidDeltaSequence { base, sequence } => write!(
                formatter,
                "semantic view delta base {base} must be less than sequence {sequence}"
            ),
            Self::EmptyCommit => formatter.write_str("semantic view commit token is empty"),
            Self::CommitTooLarge { actual, max } => write!(
                formatter,
                "semantic view commit token has {actual} bytes; maximum is {max}"
            ),
            Self::DataSerialization => {
                formatter.write_str("semantic view payload could not be serialized")
            }
            Self::DataTooLarge { actual, max } => write!(
                formatter,
                "semantic view payload has {actual} bytes; maximum is {max}"
            ),
        }
    }
}

impl std::error::Error for ViewEnvelopeError {}

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
        /// Panel feed id, e.g. `"wod.roll"`, `"tactics.map"`.
        feed: SmolStr,
        /// Snapshot vs. incremental frame.
        kind: PanelFrameKind,
        /// Module-defined payload; opaque to this crate.
        data: serde_json::Value,
    },
    /// A bounded, schema-versioned and gap-detecting semantic view.
    ///
    /// Unlike `Panel`, consumers never apply a delta without matching its
    /// exact generation/base cursor. A mismatch requests a fresh keyframe.
    View(ViewEnvelope),
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
