//! Server → client events: [`OutputEvent`] and its payload types.

use std::collections::BTreeMap;
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
    /// Structured ambience for a room band or status line, keyed by
    /// contributor — `"ic_time"` (core), `"weather"`, `"scene"`, … — an
    /// open, bounded map (interface plan I1, D-I-4). Absent on the wire when
    /// empty; readers treat every key as optional and every value
    /// defensively, since each is a module's own shape.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub ambient: BTreeMap<SmolStr, serde_json::Value>,
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
    /// Location name, when the viewer is allowed to see it: always for
    /// staff, and for a player only when the entry is in the viewer's own
    /// room (interface plan I3, D-I-12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Whether the entry is in the viewer's own room. Absent on the wire
    /// when false. The one fact a `Where` column and a "N in your room"
    /// footer need that `location` alone cannot carry for staff, who see
    /// every location.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub here: bool,
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

/// Maximum entries in one [`VitalsFrame`].
pub const MAX_VITALS_PER_FEED: usize = 16;
/// Maximum UTF-8 bytes in a [`Vital::detail`].
pub const MAX_VITAL_DETAIL_BYTES: usize = 64;
/// Maximum UTF-8 bytes in a [`Vital::id`] or [`Vital::label`].
pub const MAX_VITAL_NAME_BYTES: usize = 48;

/// How a client places and weights one vital (interface plan I1, D-I-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum VitalRole {
    /// The one number that ends the scene: the biggest type and its own bar.
    Dominant,
    /// A damage or condition track, rendered as boxes.
    Track,
    /// A spendable pool, rendered as a bar plus digits.
    Pool,
    /// A short text readout (a heading, a wound modifier, a mobility mode).
    Readout,
}

/// The urgency a vital is in, as the ruleset judges it. Clients colour by
/// this and never recompute a threshold themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum VitalState {
    /// Nothing to worry about.
    Ok,
    /// Worth a glance.
    Warn,
    /// Act now.
    Crit,
    /// Exhausted, destroyed, or out.
    Down,
}

/// A vital's value. Internally tagged on `kind` so a client can switch on
/// one field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VitalValue {
    /// A bounded integer (heat, blood, edge, willpower).
    Bounded {
        /// The current value, within `[min, max]`.
        current: i64,
        /// Inclusive lower bound.
        min: i64,
        /// Inclusive upper bound.
        max: i64,
    },
    /// A track of boxes (health, a condition monitor): `None` is an empty
    /// box, `Some(mark)` a filled one carrying the ruleset's mark token.
    Track {
        /// The boxes, in track order.
        boxes: Vec<Option<SmolStr>>,
        /// The label of the deepest filled level ("Bruised"), when the
        /// track has levels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<String>,
    },
    /// A short text readout ("hdg 047", "P4 M6 S4").
    Text {
        /// The readout.
        text: String,
    },
}

/// One vital: a gauge, track or readout a client renders without knowing
/// the ruleset that produced it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Vital {
    /// Stable id, ruleset-scoped (`"wod.health"`, `"shadowrun.stun"`).
    pub id: SmolStr,
    /// Short label ("health", "stun", "heat").
    pub label: String,
    /// Placement and weight.
    pub role: VitalRole,
    /// The value.
    pub value: VitalValue,
    /// The ruleset's urgency verdict.
    pub state: VitalState,
    /// A short qualifier ("shutdown 24", "−2 to every pool"), when useful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A whole vitals frame for one subject: the `data` of a
/// [`OutputEvent::Panel`] keyframe on a `<owner>.vitals` feed. Frames are
/// always whole (never deltas); `revision` is monotonic per subject within
/// a server process, so a client drops an older frame that arrives late and
/// resets its watermark on every `hello` and on `Resumed { replayed: false }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct VitalsFrame {
    /// Whose vitals these are.
    pub subject: ObjRef,
    /// Monotonic per subject within a process.
    pub revision: u64,
    /// At most [`MAX_VITALS_PER_FEED`] entries, in display order.
    pub vitals: Vec<Vital>,
}

/// Why a [`VitalsFrame`] refused to be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VitalsFrameError {
    /// More than [`MAX_VITALS_PER_FEED`] entries.
    TooMany {
        /// The offered count.
        count: usize,
    },
    /// A vital with an empty id.
    EmptyId,
    /// An id or label over [`MAX_VITAL_NAME_BYTES`].
    NameTooLong {
        /// The offending vital's id.
        id: SmolStr,
        /// Its byte length.
        bytes: usize,
    },
    /// A detail over [`MAX_VITAL_DETAIL_BYTES`].
    DetailTooLong {
        /// The offending vital's id.
        id: SmolStr,
        /// Its byte length.
        bytes: usize,
    },
    /// A bounded value outside its own bounds.
    OutOfBounds {
        /// The offending vital's id.
        id: SmolStr,
    },
}

impl fmt::Display for VitalsFrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VitalsFrameError::TooMany { count } => {
                write!(f, "{count} vitals exceed the cap of {MAX_VITALS_PER_FEED}")
            }
            VitalsFrameError::EmptyId => f.write_str("a vital has an empty id"),
            VitalsFrameError::NameTooLong { id, bytes } => write!(
                f,
                "vital {id:?} name is {bytes} bytes (cap {MAX_VITAL_NAME_BYTES})"
            ),
            VitalsFrameError::DetailTooLong { id, bytes } => write!(
                f,
                "vital {id:?} detail is {bytes} bytes (cap {MAX_VITAL_DETAIL_BYTES})"
            ),
            VitalsFrameError::OutOfBounds { id } => {
                write!(f, "vital {id:?} is outside its own bounds")
            }
        }
    }
}

impl std::error::Error for VitalsFrameError {}

impl VitalsFrame {
    /// Build a frame, refusing anything over the caps or a bounded value
    /// outside its bounds — the producer's gate; a consumer still reads
    /// defensively.
    ///
    /// # Errors
    ///
    /// Returns a [`VitalsFrameError`] naming the first offending vital.
    pub fn new(
        subject: ObjRef,
        revision: u64,
        vitals: Vec<Vital>,
    ) -> Result<VitalsFrame, VitalsFrameError> {
        if vitals.len() > MAX_VITALS_PER_FEED {
            return Err(VitalsFrameError::TooMany {
                count: vitals.len(),
            });
        }
        for v in &vitals {
            if v.id.is_empty() {
                return Err(VitalsFrameError::EmptyId);
            }
            let longest = v.id.len().max(v.label.len());
            if longest > MAX_VITAL_NAME_BYTES {
                return Err(VitalsFrameError::NameTooLong {
                    id: v.id.clone(),
                    bytes: longest,
                });
            }
            if let Some(d) = &v.detail
                && d.len() > MAX_VITAL_DETAIL_BYTES
            {
                return Err(VitalsFrameError::DetailTooLong {
                    id: v.id.clone(),
                    bytes: d.len(),
                });
            }
            if let VitalValue::Bounded { current, min, max } = v.value
                && (min > max || current < min || current > max)
            {
                return Err(VitalsFrameError::OutOfBounds { id: v.id.clone() });
            }
        }
        Ok(VitalsFrame {
            subject,
            revision,
            vitals,
        })
    }

    /// The frame as `Panel.data`.
    #[must_use]
    pub fn to_panel_data(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a vitals frame serializes")
    }

    /// Parse a `Panel.data` back into a frame (a consumer's defensive read).
    ///
    /// # Errors
    ///
    /// Returns the serde error when the payload is not a vitals frame.
    pub fn from_panel_data(data: &serde_json::Value) -> Result<VitalsFrame, serde_json::Error> {
        serde_json::from_value(data.clone())
    }
}

/// Everything the server can say to a client — the one semantic event
/// vocabulary (treaty ruling 1).
///
/// Adjacently tagged on the wire: `{"t": "narrative", "d": {...}}`. The ANSI
/// renderer turns each event into scrollback text; the web client routes
/// panel-oriented events (`Channel`, `Who`, `Panel`, …) to native panels
/// *and/or* scrollback. One event, two renders — the core never emits twice.
///
/// The most options a [`RequestKind::Choice`] request may carry.
pub const MAX_REQUEST_OPTIONS: usize = 32;
/// The most UTF-8 bytes in a request prompt.
pub const MAX_REQUEST_PROMPT_BYTES: usize = 2048;
/// The most UTF-8 bytes in a free-text response.
pub const MAX_REQUEST_TEXT_BYTES: usize = 2048;

/// What a server-initiated [`OutputEvent::Request`] asks the client for
/// (interface plan I10). The server owns the effect; the client only reports
/// the choice (D-I-21, choice-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    /// A yes/no confirmation.
    Confirm,
    /// One of the request's `options`.
    Choice,
    /// A target: an object name or dbref the viewer may perceive.
    Target,
    /// Free text.
    Text,
}

/// One selectable option in a [`RequestKind::Choice`] request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestOption {
    /// The stable id the client returns in [`RespondChoice::Choose`].
    pub id: SmolStr,
    /// The label shown to the viewer.
    pub label: String,
}

/// A client's answer to an [`OutputEvent::Request`] (interface plan I10),
/// carried by [`ClientCommand::Respond`](crate::ClientCommand::Respond).
/// The client reports only the choice; the server that opened the request
/// owns whatever it does with it (D-I-21).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum RespondChoice {
    /// Answer to a [`RequestKind::Confirm`].
    Confirm(bool),
    /// The chosen option's id ([`RequestKind::Choice`]).
    Choose(SmolStr),
    /// A [`RequestKind::Target`] spec or [`RequestKind::Text`] body.
    Text(String),
    /// The viewer declined; the server applies its own default.
    Cancel,
}

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
    /// Someone in the room started or stopped composing a line (interface
    /// plan I1, D-I-11). Room-scoped, perceive-gated, rate-limited, opt-out,
    /// never an idle stamp; a client shows a pill and expires it on its own
    /// timer, so a lost `active: false` costs nothing.
    Composing {
        /// Who is typing.
        actor: ObjRef,
        /// `true` when a draft began, `false` when it was sent or abandoned.
        active: bool,
    },

    // -- request/response correlation --
    /// A server-initiated interaction (interface plan I10, D-I-21): the
    /// server poses a bounded choice and the client answers with
    /// [`ClientCommand::Respond`](crate::ClientCommand::Respond) carrying the
    /// same `id`. Advisory: it never blocks the line parser; it expires at
    /// `deadline_ms` and the server applies a default, so a line-only or
    /// disconnected client is never stuck. Perceive- and DARK-gated at the
    /// producer; never an idle stamp.
    Request {
        /// Server-chosen correlation id, echoed in the response.
        id: u64,
        /// What the request asks for.
        kind: RequestKind,
        /// The prompt shown to the viewer (`MAX_REQUEST_PROMPT_BYTES`).
        prompt: RichText,
        /// The options for a [`RequestKind::Choice`] (`MAX_REQUEST_OPTIONS`);
        /// empty for the other kinds.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<RequestOption>,
        /// When the request expires, Unix milliseconds; `None` leaves the
        /// timeout to the server's default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline_ms: Option<u64>,
    },
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
