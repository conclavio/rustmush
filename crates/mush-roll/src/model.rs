use std::collections::{BTreeMap, VecDeque};

use mush_core::{ModuleOp, ObjId, mechanics::DefinitionDigest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::{RollError, invalid};

pub const FORMAT: u16 = 1;
pub const REVISION_DOMAIN: &str = "tacsim.mind.v1";
pub const MAX_MINDS: usize = 20;
pub const MAX_SOURCE_BYTES: usize = 32 * 1024;
pub const MAX_ENVELOPE_BYTES: usize = 64 * 1024;
/// One owner record may contain six-byte JSON escapes for every source byte,
/// the already-encoded metadata envelope, and the bounded name/history indexes.
/// The 32 KiB overhead covers 128 escaped 64-byte version names with u64 values,
/// 50 unnamed u64 revisions, identity/counters, and JSON field names. Source,
/// envelope, history, library and total-record quotas remain independent.
pub const MAX_DOCUMENT_BYTES: usize = 6 * MAX_SOURCE_BYTES + MAX_ENVELOPE_BYTES + 32 * 1024;
pub const MAX_NOTES_BYTES: usize = 2 * 1024;
pub const MAX_NAMED_VERSIONS: usize = 128;
pub const KEEP_UNNAMED: usize = 50;
pub const MAX_REFERENCE_TARGETS: usize = 128;
pub const MAX_REFERENCE_CHANGES: usize = 128;
pub const MAX_REFERENCE_ID_BYTES: usize = 192;
pub const MAX_RECORDS: usize = 65_536;
pub const MAX_GC_BATCH: usize = 128;

/// Schema-neutral executable metadata. The outer shape and allocation bounds
/// are this owner's; the adapter checks all game-specific meanings and classes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub schema: Value,
    pub presets: Value,
    pub profiles: Value,
    pub rules: Value,
    pub declared_class: String,
    pub mind_abi: String,
}

impl Envelope {
    pub fn validate(&self) -> Result<(), RollError> {
        bounded_text("declared_class", &self.declared_class, 32, false)?;
        bounded_text("mind_abi", &self.mind_abi, 128, false)?;
        object_count("schema", &self.schema, 256, true)?;
        object_count("presets", &self.presets, 8, true)?;
        object_count("profiles", &self.profiles, 4, false)?;
        object_count("rules", &self.rules, 4, true)?;
        if let Some(rules) = self.rules.as_object() {
            for (profile, rows) in rules {
                if !self
                    .profiles
                    .as_object()
                    .is_some_and(|p| p.contains_key(profile))
                {
                    return Err(invalid("rules", format!("unknown profile {profile}")));
                }
                if !rows.as_array().is_some_and(|rows| rows.len() <= 6) {
                    return Err(invalid(
                        "rules",
                        "each profile needs an array of at most 6 rules",
                    ));
                }
            }
        }
        let value = serde_json::to_value(self).map_err(|e| invalid("envelope", e.to_string()))?;
        let mut nodes = 0;
        bounded_json(&value, 0, &mut nodes)?;
        encoded(&value, "envelope", MAX_ENVELOPE_BYTES)?;
        Ok(())
    }

    pub fn digest(&self, normalized_source: &str) -> Result<DefinitionDigest, RollError> {
        self.validate()?;
        bounded_source(normalized_source)?;
        DefinitionDigest::for_canonical_value_with_limit(
            REVISION_DOMAIN,
            &serde_json::json!({"source": normalized_source, "envelope": self}),
            MAX_DOCUMENT_BYTES,
        )
        .map_err(|e| invalid("revision digest", e.to_string()))
    }
}

/// A working draft may contain invalid source; its dimensions and metadata
/// envelope must still fit. Saving, unlike editing, calls the validator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkingCopy {
    pub source: String,
    pub envelope: Envelope,
}

impl WorkingCopy {
    pub fn validate_bounds(&self) -> Result<(), RollError> {
        bounded_source(&self.source)?;
        self.envelope.validate()
    }
}

/// Game-supplied language and schema adapter. Normalization must change only
/// lexical line endings outside literals: preserve literal bytes, line count
/// and diagnostic positions. In particular a global CRLF replace is invalid
/// for languages with multiline strings. The output must be idempotent.
pub trait RevisionAdapter {
    fn normalize_source(&self, source: &str) -> Result<String, RollError>;
    fn validate(&self, source: &str, envelope: &Envelope) -> Result<(), RollError>;
}

/// The existing source owner prepares immutable storage, including refusing an
/// unreadable occupied key. It must not mutate during preparation. Its opaque
/// token stays with the caller, which checks and applies it through that owner.
pub trait SourceAdapter {
    type Token;
    fn prepare_source(
        &self,
        source: &str,
        abi: &str,
    ) -> Result<PreparedSource<Self::Token>, RollError>;
    fn read_source(&self, digest: DefinitionDigest) -> Result<&str, RollError>;
}

pub struct PreparedSource<T> {
    pub digest: DefinitionDigest,
    pub operations: Vec<ModuleOp>,
    pub token: T,
}

pub fn source_digest(source: &str) -> DefinitionDigest {
    DefinitionDigest::from_bytes(Sha256::digest(source.as_bytes()).into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionKey {
    pub character: ObjId,
    pub mind: u64,
    pub number: u64,
}

impl RevisionKey {
    pub(crate) fn key(self) -> String {
        format!(
            "revision:{}:{}:{}",
            self.character.0, self.mind, self.number
        )
    }
}

/// Complete immutable executable identity. A reference never relies on a
/// shortened hash or on source SHA alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRef {
    pub key: RevisionKey,
    pub digest: DefinitionDigest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub(crate) format: u16,
    pub(crate) reference: RevisionRef,
    pub(crate) source: DefinitionDigest,
    pub(crate) envelope: Envelope,
    /// Revision notes are immutable annotations, deliberately outside the digest.
    pub(crate) notes: String,
}

impl Revision {
    pub fn reference(&self) -> RevisionRef {
        self.reference
    }
    pub fn source(&self) -> DefinitionDigest {
        self.source
    }
    pub fn envelope(&self) -> &Envelope {
        &self.envelope
    }
    pub fn notes(&self) -> &str {
        &self.notes
    }
    pub(crate) fn validate(&self) -> Result<(), RollError> {
        format(self.format)?;
        revision_key(self.reference.key)?;
        self.envelope.validate()?;
        bounded_text("revision notes", &self.notes, MAX_NOTES_BYTES, true)
    }
    pub fn verify_source<S: SourceAdapter>(&self, sources: &S) -> Result<(), RollError> {
        let source = sources.read_source(self.source)?;
        if source_digest(source) != self.source
            || self.envelope.digest(source)? != self.reference.digest
        {
            return Err(invalid(
                self.reference.key.key(),
                "source or revision digest mismatch",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditBase {
    pub edit: u64,
    pub revision: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selection {
    Pin { revision: RevisionRef },
    Follow { mind: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Library {
    pub format: u16,
    pub character: ObjId,
    pub next_mind: u64,
    pub names: BTreeMap<String, u64>,
    pub selection: Option<Selection>,
}

impl Library {
    pub fn new(character: ObjId) -> Self {
        Self {
            format: FORMAT,
            character,
            next_mind: 1,
            names: BTreeMap::new(),
            selection: None,
        }
    }
    pub fn key(&self) -> String {
        format!("library:{}", self.character.0)
    }
    pub fn validate(&self) -> Result<(), RollError> {
        format(self.format)?;
        if self.names.len() > MAX_MINDS || self.next_mind == 0 {
            return Err(invalid(self.key(), "library count or allocator bound"));
        }
        let mut ids = std::collections::BTreeSet::new();
        for (name, id) in &self.names {
            if canonical_name(name)? != *name
                || *id == 0
                || *id >= self.next_mind
                || !ids.insert(*id)
            {
                return Err(invalid(self.key(), "invalid or duplicate mind identity"));
            }
        }
        if let Some(selection) = &self.selection {
            let mind = match selection {
                Selection::Pin { revision } if revision.key.character == self.character => {
                    revision.key.mind
                }
                Selection::Follow { mind } => *mind,
                _ => {
                    return Err(invalid(
                        self.key(),
                        "selection belongs to another character",
                    ));
                }
            };
            if !ids.contains(&mind) {
                return Err(invalid(self.key(), "selected mind is absent"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mind {
    pub(crate) format: u16,
    pub(crate) character: ObjId,
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) edit: u64,
    pub(crate) next_revision: u64,
    pub(crate) head: Option<u64>,
    pub(crate) draft: WorkingCopy,
    pub(crate) unnamed: VecDeque<u64>,
    pub(crate) named: BTreeMap<String, u64>,
}

impl Mind {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn draft(&self) -> &WorkingCopy {
        &self.draft
    }
    pub fn base(&self) -> EditBase {
        EditBase {
            edit: self.edit,
            revision: self.head,
        }
    }
    pub fn named(&self) -> &BTreeMap<String, u64> {
        &self.named
    }
    pub fn latest(&self) -> Option<RevisionKey> {
        self.head.map(|number| RevisionKey {
            character: self.character,
            mind: self.id,
            number,
        })
    }
    pub(crate) fn key(&self) -> String {
        format!("mind:{}:{}", self.character.0, self.id)
    }
    pub(crate) fn validate(&self) -> Result<(), RollError> {
        format(self.format)?;
        if canonical_name(&self.name)? != self.name {
            return Err(invalid(self.key(), "mind name is not canonical"));
        }
        self.draft.validate_bounds()?;
        if self.id == 0
            || self.next_revision == 0
            || self.unnamed.len() > KEEP_UNNAMED
            || self.named.len() > MAX_NAMED_VERSIONS
        {
            return Err(invalid(self.key(), "mind bounds or allocator"));
        }
        let mut numbers = std::collections::BTreeSet::new();
        let mut previous = 0;
        for number in &self.unnamed {
            if *number <= previous || *number >= self.next_revision || !numbers.insert(*number) {
                return Err(invalid(self.key(), "invalid unnamed revision"));
            }
            previous = *number;
        }
        for (name, number) in &self.named {
            if canonical_name(name)? != *name
                || *number == 0
                || *number >= self.next_revision
                || !numbers.insert(*number)
            {
                return Err(invalid(self.key(), "invalid named revision"));
            }
        }
        if self
            .head
            .is_some_and(|n| n == 0 || n >= self.next_revision || !numbers.contains(&n))
        {
            return Err(invalid(
                self.key(),
                "head is absent from retained revisions",
            ));
        }
        if self.head != self.next_revision.checked_sub(1).filter(|n| *n != 0)
            || self.head.is_some_and(|n| self.edit < n)
        {
            return Err(invalid(
                self.key(),
                "head or edit counter disagrees with revision allocator",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    NamedVersion,
    UnnamedHistory,
    /// A game-owned volatile lobby selection, released at teardown or boot.
    Selection,
    MatchRecord,
    Tape,
    ProvingRun,
    OpenSeal,
    Listing,
}

impl ReferenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NamedVersion => "named_version",
            Self::UnnamedHistory => "unnamed_history",
            Self::Selection => "selection",
            Self::MatchRecord => "match_record",
            Self::Tape => "tape",
            Self::ProvingRun => "proving_run",
            Self::OpenSeal => "open_seal",
            Self::Listing => "listing",
        }
    }
}

/// A durable owner attests that it still needs every exact revision in this
/// record. Releasing it is an explicit operation by that same owner.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceKey {
    pub owner: String,
    pub kind: ReferenceKind,
    pub record: String,
}

impl ReferenceKey {
    pub(crate) fn key(&self) -> String {
        format!(
            "reference:{}:{}:{}",
            self.owner,
            self.kind.as_str(),
            self.record
        )
    }
    pub(crate) fn validate(&self) -> Result<(), RollError> {
        mush_core::mechanics::OwnerId::parse_canonical(&self.owner)
            .map_err(|e| invalid("reference owner", e.to_string()))?;
        if self.record.is_empty()
            || self.record.len() > MAX_REFERENCE_ID_BYTES
            || !self
                .record
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-/".contains(&c))
        {
            return Err(invalid(
                "reference record",
                "expected 1..192 ASCII identifier bytes",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferenceRecord {
    pub format: u16,
    pub key: ReferenceKey,
    pub revisions: Vec<RevisionRef>,
}

impl ReferenceRecord {
    pub fn validate(&self) -> Result<(), RollError> {
        format(self.format)?;
        self.key.validate()?;
        if matches!(
            self.key.kind,
            ReferenceKind::NamedVersion | ReferenceKind::UnnamedHistory
        ) != (self.key.owner == crate::NS)
        {
            return Err(invalid(
                "reference owner",
                "named versions and retained history belong only to roll",
            ));
        }
        if self.revisions.is_empty() || self.revisions.len() > MAX_REFERENCE_TARGETS {
            return Err(invalid(self.key.key(), "reference count must be 1..128"));
        }
        if (self.key.kind == ReferenceKind::NamedVersion && self.revisions.len() != 1)
            || (self.key.kind == ReferenceKind::UnnamedHistory
                && self.revisions.len() > KEEP_UNNAMED)
        {
            return Err(invalid(
                self.key.key(),
                "named version or retained-history count",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for revision in &self.revisions {
            revision_key(revision.key)?;
            if !seen.insert(revision.key) {
                return Err(invalid(self.key.key(), "duplicate revision reference"));
            }
        }
        Ok(())
    }
}

pub fn canonical_name(name: &str) -> Result<String, RollError> {
    bounded_text("name", name, 64, false)?;
    if name.trim() != name
        || name.contains(['/', '@', '#', ':'])
        || name.chars().any(char::is_control)
    {
        return Err(invalid("name", "no surrounding spaces or / @ # :"));
    }
    Ok(name.to_ascii_lowercase())
}

pub(crate) fn bounded_source(source: &str) -> Result<(), RollError> {
    if source.len() > MAX_SOURCE_BYTES || source.contains('\0') {
        return Err(invalid("source", "maximum 32 KiB; NUL is forbidden"));
    }
    Ok(())
}

pub(crate) fn bounded_text(
    field: &str,
    text: &str,
    max: usize,
    empty: bool,
) -> Result<(), RollError> {
    if text.len() > max
        || (!empty && text.is_empty())
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(invalid(
            field,
            format!("invalid text or exceeds {max} bytes"),
        ));
    }
    Ok(())
}

fn object_count(field: &str, value: &Value, max: usize, empty: bool) -> Result<(), RollError> {
    if !value
        .as_object()
        .is_some_and(|v| v.len() <= max && (empty || !v.is_empty()))
    {
        return Err(invalid(
            field,
            format!("expected {}..{max} object entries", usize::from(!empty)),
        ));
    }
    Ok(())
}

fn bounded_json(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), RollError> {
    *nodes += 1;
    if depth > 16 || *nodes > 4096 {
        return Err(invalid("envelope", "JSON depth or node limit"));
    }
    match value {
        Value::Number(n)
            if !n.as_i64().is_some_and(|value| {
                (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&value)
            }) =>
        {
            return Err(invalid(
                "envelope",
                "integers must be exactly representable across clients (absolute value at most 9007199254740991)",
            ));
        }
        Value::String(s) => bounded_text("envelope string", s, 8192, true)?,
        Value::Array(items) => {
            for item in items {
                bounded_json(item, depth + 1, nodes)?;
            }
        }
        Value::Object(items) => {
            for (key, item) in items {
                bounded_text("envelope key", key, 128, false)?;
                bounded_json(item, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn encoded<T: Serialize>(
    record: &T,
    field: &str,
    max: usize,
) -> Result<Value, RollError> {
    struct Capped {
        bytes: Vec<u8>,
        max: usize,
    }
    impl std::io::Write for Capped {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other("encoded document byte limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Capped {
        bytes: Vec::new(),
        max,
    };
    serde_json::to_writer(&mut output, record)
        .map_err(|e| invalid(field, format!("document (maximum {max} bytes): {e}")))?;
    serde_json::from_slice(&output.bytes).map_err(|e| invalid(field, e.to_string()))
}

pub(crate) fn format(value: u16) -> Result<(), RollError> {
    if value == FORMAT {
        Ok(())
    } else {
        Err(invalid("format", "unsupported roll format"))
    }
}

fn revision_key(key: RevisionKey) -> Result<(), RollError> {
    if key.mind == 0 || key.number == 0 {
        Err(invalid("revision", "zero identity"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod wire_integer_tests {
    #[test]
    fn authored_numbers_survive_json_clients_without_rounding() {
        for value in [-9_007_199_254_740_991_i64, 0, 9_007_199_254_740_991] {
            assert!(super::bounded_json(&serde_json::json!({"nested":[value]}), 0, &mut 0).is_ok());
        }
        for value in [
            serde_json::json!(9_007_199_254_740_992_u64),
            serde_json::json!(-9_007_199_254_740_992_i64),
            serde_json::json!(u64::MAX),
            serde_json::json!(1.5),
        ] {
            assert!(
                super::bounded_json(&serde_json::json!({"nested":[value]}), 0, &mut 0).is_err()
            );
        }
    }
}
