//! Closed, bounded authoring requests. These run with mutation authority;
//! panel subscriptions and read-only queries never carry source or saves.

use serde::{Deserialize, Serialize};

/// Maximum complete JSON request or result, before deserialization.
pub const MAX_INVOKE_BYTES: usize = 272 * 1024;
/// Maximum encoded card metadata, excluding source and transport framing.
pub const MAX_MIND_ENVELOPE_BYTES: usize = 64 * 1024;
/// Maximum UTF-8 source bytes, independently of the whole envelope bound.
pub const MAX_MIND_SOURCE_BYTES: usize = 32 * 1024;
/// Maximum container nesting before parsing a request.
pub const MAX_INVOKE_DEPTH: usize = 32;

/// Finite module service vocabulary; an owner can register only this family.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "service",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum InvokeRequest {
    /// Character-owned unit mind authoring.
    Mind(MindRequest),
}
impl InvokeRequest {
    /// Registration key, independent of caller-controlled strings.
    #[must_use]
    pub const fn service(&self) -> &'static str {
        match self {
            Self::Mind(_) => "mind",
        }
    }
    /// Validate the complete wire envelope and source bounds again at ingress.
    pub fn validate(&self) -> Result<(), &'static str> {
        let bytes = serde_json::to_vec(self).map_err(|_| "invalid request")?;
        validate_invoke_json(&bytes)?;
        let Self::Mind(request) = self;
        if let MindRequest::Replace { draft, .. } = request {
            if draft.source.len() > MAX_MIND_SOURCE_BYTES || draft.source.contains('\0') {
                return Err("source exceeds 32 KiB or contains NUL");
            }
            let metadata = serde_json::json!({
                "schema": draft.schema, "presets": draft.presets,
                "profiles": draft.profiles, "rules": draft.rules,
                "declared_class": draft.declared_class, "mind_abi": draft.mind_abi
            });
            if serde_json::to_vec(&metadata)
                .map_err(|_| "invalid metadata")?
                .len()
                > MAX_MIND_ENVELOPE_BYTES
            {
                return Err("metadata envelope exceeds 64 KiB");
            }
        }
        Ok(())
    }
}

/// Compare-and-swap token returned by the owner; never a validation receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MindEditBase {
    /// Working-copy edit generation.
    pub edit: u64,
    /// Last saved local revision, if any.
    pub revision: Option<u64>,
}

/// Complete editable envelope. Game semantics are checked by the same owner
/// services as line commands; JSON is never evaluated as command text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MindDraft {
    /// Exact source, including line endings.
    pub source: String,
    /// Declarative card schema.
    pub schema: serde_json::Value,
    /// Named card presets.
    pub presets: serde_json::Value,
    /// Named profile values.
    pub profiles: serde_json::Value,
    /// Ordered rules by profile.
    pub rules: serde_json::Value,
    /// Declared core class.
    pub declared_class: String,
    /// Executable mind ABI.
    pub mind_abi: String,
}

/// Structured counterparts of the card/profile/rule editors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindEdit {
    /// Set one schema-defined field.
    Set {
        /// Profile name.
        profile: String,
        /// Field identifier.
        field: String,
        /// The schema's textual value syntax.
        value: String,
    },
    /// Save current profile values as a preset.
    PresetSave {
        /// Profile name.
        profile: String,
        /// Preset name.
        preset: String,
    },
    /// Apply a preset to one profile.
    PresetApply {
        /// Profile name.
        profile: String,
        /// Preset name.
        preset: String,
    },
    /// Remove one preset.
    PresetDelete {
        /// Preset name.
        preset: String,
    },
    /// Add a profile, optionally copying another.
    ProfileAdd {
        /// New name.
        name: String,
        /// Existing profile to copy.
        from: Option<String>,
    },
    /// Remove one profile.
    ProfileDelete {
        /// Profile name.
        name: String,
    },
    /// Rename one profile.
    ProfileRename {
        /// Existing name.
        old: String,
        /// New name.
        new: String,
    },
    /// Replace/remove an ordered rule using the native rule grammar.
    Rule {
        /// Profile name.
        profile: String,
        /// One-based authored slot.
        slot: u16,
        /// Empty removes; otherwise one bounded rule.
        text: String,
    },
}

/// Library lifecycle changes use the same exact draft token as working edits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindLifecycle {
    /// Rename this library entry without changing immutable revisions.
    Rename {
        /// New library name.
        new_name: String,
    },
    /// Remove the working entry; named and retained revisions remain rooted.
    // A tagged unit variant ignores unknown fields in serde; keep an empty
    // struct variant so the closed request grammar rejects extra fields.
    Remove {},
    /// Rename an existing local version label.
    VersionRename {
        /// Existing version label.
        old: String,
        /// New version label.
        new: String,
    },
    /// Release a version label into the bounded unnamed revision history.
    VersionForget {
        /// Version label to release.
        version: String,
    },
}

/// All requests act as the connection's selected character. No actor, account,
/// source path, validation receipt, or arbitrary command is accepted here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindRequest {
    /// Change a library entry or version label with exact edit authority.
    Lifecycle {
        /// Current library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
        /// Owner operation.
        edit: MindLifecycle,
    },
    /// Remove up to this many unreferenced revisions owned by this character.
    Prune {
        /// One through 128; all retained roots still apply.
        limit: u16,
    },
    /// Select an authorized own-hull trace feed; this is never source access.
    Trace {
        /// Hull object selector, resolved through the current trace ACL.
        unit: u64,
        /// One through 64 records; absent means three.
        count: Option<u8>,
        /// Keep following after this bounded read.
        follow: bool,
    },
    /// List this character's current library.
    List {},
    /// Read working copy (`None`) or one local revision/version/sha reference.
    Read {
        /// Library name.
        name: String,
        /// Local revision selector, without a name or owner.
        revision: Option<String>,
    },
    /// Create a new copy; `from` uses the existing owned-copy reference syntax.
    Create {
        /// New library name.
        name: String,
        /// Optional owned source or the public Standard template.
        from: Option<String>,
    },
    /// Replace the working copy. This does not save an immutable revision.
    Replace {
        /// Library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
        /// New complete working copy.
        draft: Box<MindDraft>,
    },
    /// Edit one card/profile/rule through the shared typed editor.
    Edit {
        /// Library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
        /// Typed edit.
        edit: MindEdit,
    },
    /// Check the exact working copy in the six isolated pictures, without save.
    Check {
        /// Library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
    },
    /// Validate then publish an immutable revision of this exact draft.
    Save {
        /// Library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
        /// Bounded revision notes.
        notes: String,
        /// Optional local version label.
        version: Option<String>,
    },
    /// Inspect a job submitted on this same connection lifetime.
    Status {
        /// Opaque queued job number.
        job: u64,
    },
    /// Cancel a job submitted on this same connection lifetime.
    Cancel {
        /// Opaque queued job number.
        job: u64,
    },
}

/// A bounded diagnostic that never includes source excerpts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MindDiagnostic {
    /// Field or check stage.
    pub field: String,
    /// Short owner-approved explanation.
    pub message: String,
    /// Actual one-based parser line when available; absent means unknown.
    pub line: Option<u32>,
    /// Actual one-based parser column when available; absent means unknown.
    pub column: Option<u32>,
}
/// One actual isolated picture result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MindPictureReport {
    /// Stable picture identifier.
    pub picture: String,
    /// Whether this case had existing armings.
    pub armed: bool,
    /// Metered interpreter operations.
    pub operations: u64,
    /// Metered host fees.
    pub fees: u64,
    /// Bounded rendered intent descriptions.
    pub intents: Vec<String>,
    /// Coarse fault category, never raw source.
    pub fault: Option<String>,
    /// Actual station landing refusals.
    pub station_refusals: Vec<String>,
    /// Declared-class violations.
    pub class_violations: Vec<String>,
}
/// Final responses and explicit asynchronous progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindResponse {
    /// The library entry was removed; retained immutable revisions survive.
    Removed {
        /// Former library name.
        name: String,
    },
    /// A bounded revision collection completed; source bytes remain retained.
    Pruned {
        /// Unreferenced revision documents removed.
        revisions: u16,
        /// Distinct sources of these owned revisions, all still retained.
        /// This count does not expose another character's source references.
        source_candidates_retained: u16,
    },
    /// Authorized bounded trace read and subscription cursor.
    Trace {
        /// Trace feed's source-free projection.
        data: serde_json::Value,
    },
    /// This character's bounded library labels.
    Library {
        /// Owned names.
        names: Vec<String>,
    },
    /// Exact source and envelope; token always describes the current draft.
    Read {
        /// Library name.
        name: String,
        /// Requested revision selector.
        revision: Option<String>,
        /// Current draft token.
        base: MindEditBase,
        /// Exact owned working copy or revision.
        draft: Box<MindDraft>,
        /// Canonical rule grammar by profile, rendered by the shared schema owner.
        rules_text: std::collections::BTreeMap<String, Vec<String>>,
        /// Exact editable grammar by profile/slot; null has no lossless spelling.
        rules_source: std::collections::BTreeMap<String, Vec<Option<String>>>,
        /// Invalid working metadata is reported without rewriting or hiding source.
        diagnostics: Vec<MindDiagnostic>,
    },
    /// A working edit completed; clients must use this new token next.
    Edited {
        /// Library name.
        name: String,
        /// New current draft token.
        base: MindEditBase,
    },
    /// Worker accepted; a final result follows with the original request id.
    Queued {
        /// Opaque job number.
        job: u64,
    },
    /// Explicit status poll; this is not a final result.
    Pending {
        /// Opaque job number.
        job: u64,
    },
    /// Check finished without saving.
    Checked {
        /// Opaque job number.
        job: u64,
        /// Six isolated cases, or no cases on compilation failure.
        reports: Vec<MindPictureReport>,
        /// Bounded diagnostics.
        diagnostics: Vec<MindDiagnostic>,
    },
    /// Immutable save completed through normal deferred persistence.
    Saved {
        /// Opaque job number.
        job: u64,
        /// Local revision number.
        revision: u64,
        /// Immutable executable definition digest.
        digest: String,
        /// New current draft token.
        base: MindEditBase,
    },
    /// Job cancelled; a running worker keeps its quota until it exits.
    Cancelled {
        /// Opaque job number.
        job: u64,
    },
    /// Refusal; source ownership failures have the same bounded response.
    Error {
        /// Stable error category.
        code: String,
        /// Source-free explanation.
        message: String,
    },
}
impl MindResponse {
    /// Whether an SSH operation has reached its final result.
    #[must_use]
    pub const fn is_final(&self) -> bool {
        !matches!(self, Self::Queued { .. } | Self::Pending { .. })
    }
    /// Exit status reflects checks as well as request-level refusals.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        match self {
            Self::Error { .. } | Self::Cancelled { .. } => false,
            Self::Checked {
                reports,
                diagnostics,
                ..
            } => {
                diagnostics.is_empty()
                    && reports
                        .iter()
                        .all(|r| r.fault.is_none() && r.class_violations.is_empty())
            }
            _ => true,
        }
    }
}

/// Closed service reply, carried only to the requesting physical connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "service",
    content = "result",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum InvokeResult {
    /// Character-owned unit mind service.
    Mind(MindResponse),
}
impl InvokeResult {
    /// Generic ingress/registration refusal in the current service family.
    #[must_use]
    pub fn error(code: &str, message: &str) -> Self {
        Self::Mind(MindResponse::Error {
            code: code.into(),
            message: message.into(),
        })
    }
    /// Enforce response bounds before any output is queued.
    #[must_use]
    pub fn bounded(self) -> Self {
        if serde_json::to_vec(&self).is_ok_and(|v| validate_invoke_json(&v).is_ok()) {
            self
        } else {
            Self::error("too_large", "Result exceeds the structured response limit.")
        }
    }
}

/// Bounded linear preflight before serde allocates containers. This is not a
/// second JSON parser: serde remains responsible for all syntax validation.
pub fn validate_invoke_json(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() > MAX_INVOKE_BYTES {
        return Err("request exceeds 272 KiB");
    }
    let (mut depth, mut string, mut escaped) = (0_usize, false, false);
    for &byte in bytes {
        if string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                string = false;
            }
        } else {
            match byte {
                b'"' => string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_INVOKE_DEPTH {
                        return Err("request nesting exceeds 32");
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preflight_bounds_bytes_depth_and_escaped_delimiters() {
        assert!(validate_invoke_json(&vec![b' '; MAX_INVOKE_BYTES + 1]).is_err());
        assert!(validate_invoke_json(&[b'['; MAX_INVOKE_DEPTH + 1]).is_err());
        assert!(validate_invoke_json(br#"{"source":"[[[\"[[["}"#).is_ok());
    }
    #[test]
    fn closed_shape_refuses_owner_and_validated_receipts() {
        for json in [
            r#"{"service":"mind","request":{"op":"list","actor":7}}"#,
            r#"{"service":"mind","request":{"op":"check","name":"mine","base":{"edit":1,"revision":null},"validated":true}}"#,
        ] {
            assert!(serde_json::from_str::<InvokeRequest>(json).is_err());
        }
    }

    #[test]
    fn lifecycle_remove_is_closed_and_keeps_its_wire_shape()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = InvokeRequest::Mind(MindRequest::Lifecycle {
            name: "mine".into(),
            base: MindEditBase {
                edit: 1,
                revision: None,
            },
            edit: MindLifecycle::Remove {},
        });
        let wire = serde_json::json!({
            "service": "mind",
            "request": {
                "op": "lifecycle", "name": "mine",
                "base": {"edit": 1, "revision": null},
                "edit": {"kind": "remove"}
            }
        });
        if serde_json::to_value(&request)? != wire {
            return Err("lifecycle remove changed its valid wire shape".into());
        }
        if serde_json::from_value::<InvokeRequest>(wire)? != request {
            return Err("lifecycle remove did not round-trip".into());
        }
        for edit in [
            r#"{"kind":"remove","actor":7}"#,
            r#"{"kind":"remove","account":"foreign"}"#,
            r#"{"kind":"remove","validated":true}"#,
            r#"{"kind":"remove","source":"private"}"#,
            r#"{"kind":"remove","new_name":"other"}"#,
        ] {
            if serde_json::from_str::<MindLifecycle>(edit).is_ok() {
                return Err(format!("lifecycle remove accepted an extra field: {edit}").into());
            }
            let nested = format!(
                r#"{{"service":"mind","request":{{"op":"lifecycle","name":"mine","base":{{"edit":1,"revision":null}},"edit":{edit}}}}}"#
            );
            if serde_json::from_str::<InvokeRequest>(&nested).is_ok() {
                return Err(
                    format!("nested lifecycle remove accepted an extra field: {edit}").into(),
                );
            }
        }
        Ok(())
    }
}
