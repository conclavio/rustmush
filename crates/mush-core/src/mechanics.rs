//! Shared, ruleset-neutral mechanics identities and frozen registries.
//!
//! This module deliberately contains no world mutation, persistence, command,
//! or bridge-execution code.  Module registrars stamp an [`OwnerId`] while a
//! module is being composed, and the resulting builders are frozen before the
//! world starts accepting mechanical work.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;
use thiserror::Error;

/// Maximum UTF-8 bytes in any canonical identifier component.
pub const CANONICAL_ID_MAX_BYTES: usize = 64;
/// Default maximum canonical bytes in a single rules value.
pub const MAX_RULES_VALUE_BYTES: usize = 1_048_576;
/// Maximum subjects retained on one recovery incident.
pub const MAX_RECOVERY_SUBJECTS: usize = 64;
/// Maximum detailed incidents or migration plans retained per module report.
pub const MAX_BOOT_REPORT_DETAILS: usize = 256;
/// Maximum module owners represented in the process-lifetime readiness roster.
pub const MAX_MECHANICS_OWNERS: usize = 256;

/// Hard composition and value-size limits.  Tests and small deployments may
/// lower them, but callers cannot bypass the compile-time identifier bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MechanicsLimits {
    pub definition_owners: usize,
    pub definitions_total: usize,
    pub definition_bytes_total: usize,
    pub definitions_per_owner: usize,
    pub definition_bytes: usize,
    pub definition_bytes_per_owner: usize,
    pub references_per_definition: usize,
    pub bridges: usize,
    pub migrations_per_definition: usize,
    pub rules_value_bytes: usize,
}

impl Default for MechanicsLimits {
    fn default() -> Self {
        Self {
            definition_owners: MAX_MECHANICS_OWNERS,
            definitions_total: 16_384,
            definition_bytes_total: 134_217_728,
            definitions_per_owner: 4_096,
            definition_bytes: 1_048_576,
            definition_bytes_per_owner: 33_554_432,
            references_per_definition: 256,
            bridges: 4_096,
            migrations_per_definition: 128,
            rules_value_bytes: MAX_RULES_VALUE_BYTES,
        }
    }
}

impl MechanicsLimits {
    /// Apply caller-selected lower limits without permitting a caller to raise
    /// any process-wide mechanics bound. These ceilings are part of the trust
    /// boundary, not tuning defaults.
    #[must_use]
    fn capped(self) -> Self {
        let hard = Self::default();
        Self {
            definition_owners: self.definition_owners.min(hard.definition_owners),
            definitions_total: self.definitions_total.min(hard.definitions_total),
            definition_bytes_total: self.definition_bytes_total.min(hard.definition_bytes_total),
            definitions_per_owner: self.definitions_per_owner.min(hard.definitions_per_owner),
            definition_bytes: self.definition_bytes.min(hard.definition_bytes),
            definition_bytes_per_owner: self
                .definition_bytes_per_owner
                .min(hard.definition_bytes_per_owner),
            references_per_definition: self
                .references_per_definition
                .min(hard.references_per_definition),
            bridges: self.bridges.min(hard.bridges),
            migrations_per_definition: self
                .migrations_per_definition
                .min(hard.migrations_per_definition),
            rules_value_bytes: self.rules_value_bytes.min(hard.rules_value_bytes),
        }
    }
}

/// Why a string is not an acceptable canonical identifier.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CanonicalIdError {
    #[error("canonical identifier is empty")]
    Empty,
    #[error("canonical identifier is {actual} bytes; maximum is {max}")]
    TooLong { actual: usize, max: usize },
    #[error("canonical identifier contains an empty dot segment")]
    EmptySegment,
    #[error("canonical identifier contains invalid byte {byte:#04x}")]
    InvalidByte { byte: u8 },
    #[error("identifier is not in canonical form; expected `{canonical}`")]
    NonCanonical { canonical: String },
}

fn normalize_identifier(raw: &str) -> Result<SmolStr, CanonicalIdError> {
    let canonical = raw.trim().to_ascii_lowercase();
    validate_canonical_identifier(&canonical)?;
    Ok(canonical.into())
}

fn parse_canonical_identifier(raw: &str) -> Result<SmolStr, CanonicalIdError> {
    let canonical = normalize_identifier(raw)?;
    if canonical.as_str() != raw {
        return Err(CanonicalIdError::NonCanonical {
            canonical: canonical.to_string(),
        });
    }
    Ok(canonical)
}

fn validate_canonical_identifier(value: &str) -> Result<(), CanonicalIdError> {
    if value.is_empty() {
        return Err(CanonicalIdError::Empty);
    }
    if value.len() > CANONICAL_ID_MAX_BYTES {
        return Err(CanonicalIdError::TooLong {
            actual: value.len(),
            max: CANONICAL_ID_MAX_BYTES,
        });
    }
    if value.starts_with('.') || value.ends_with('.') || value.contains("..") {
        return Err(CanonicalIdError::EmptySegment);
    }
    if let Some(byte) = value
        .bytes()
        .find(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.'))
    {
        return Err(CanonicalIdError::InvalidByte { byte });
    }
    Ok(())
}

macro_rules! canonical_id_type {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(SmolStr);

        impl $name {
            /// Canonicalize trusted registration input (trim and lowercase),
            /// then validate its shape and hard byte limit.
            pub fn new(value: impl AsRef<str>) -> Result<Self, CanonicalIdError> {
                normalize_identifier(value.as_ref()).map(Self)
            }

            /// Parse persisted or wire input without silently normalizing it.
            pub fn parse_canonical(value: impl AsRef<str>) -> Result<Self, CanonicalIdError> {
                parse_canonical_identifier(value.as_ref()).map(Self)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.0).finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = CanonicalIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse_canonical(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse_canonical(value).map_err(D::Error::custom)
            }
        }
    };
}

canonical_id_type!(CanonicalId, "A general canonical mechanics identifier.");
canonical_id_type!(OwnerId, "The module that owns a mechanical contract.");
canonical_id_type!(DefinitionKind, "A rules-neutral definition category.");
canonical_id_type!(
    DefinitionId,
    "A definition's owner-local stable identifier."
);

/// An exact SHA-256 identity, serialized as 64 lowercase hexadecimal digits.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DefinitionDigest([u8; 32]);

impl DefinitionDigest {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_hex(value: &str) -> Result<Self, DefinitionDigestError> {
        if value.len() != 64 {
            return Err(DefinitionDigestError::Length {
                actual: value.len(),
            });
        }
        let bytes = value.as_bytes();
        if bytes.iter().any(u8::is_ascii_uppercase) {
            return Err(DefinitionDigestError::Uppercase);
        }
        let mut digest = [0_u8; 32];
        let (pairs, remainder) = bytes.as_chunks::<2>();
        debug_assert!(remainder.is_empty());
        for (index, pair) in pairs.iter().enumerate() {
            let high = hex_nibble(pair[0])
                .ok_or(DefinitionDigestError::InvalidHex { index: index * 2 })?;
            let low = hex_nibble(pair[1]).ok_or(DefinitionDigestError::InvalidHex {
                index: index * 2 + 1,
            })?;
            digest[index] = (high << 4) | low;
        }
        Ok(Self(digest))
    }

    /// Hash a JSON value using the shared deterministic encoding.  `domain`
    /// is length-prefixed into the digest so callers cannot accidentally
    /// compare hashes produced for different contracts.
    pub fn for_canonical_value(
        domain: &str,
        value: &Value,
    ) -> Result<Self, CanonicalEncodingError> {
        Self::for_canonical_value_with_limit(domain, value, usize::MAX)
    }

    /// Bounded form of [`Self::for_canonical_value`] for untrusted legacy or
    /// wire values.
    pub fn for_canonical_value_with_limit(
        domain: &str,
        value: &Value,
        max_bytes: usize,
    ) -> Result<Self, CanonicalEncodingError> {
        let bytes = canonical_json_bytes_with_limit(value, max_bytes)?;
        Ok(hash_parts(domain.as_bytes(), [&bytes[..]]))
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(64);
        for byte in self.0 {
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        encoded
    }
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

impl fmt::Debug for DefinitionDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DefinitionDigest")
            .field(&self.to_hex())
            .finish()
    }
}

impl fmt::Display for DefinitionDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for DefinitionDigest {
    type Err = DefinitionDigestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex(value)
    }
}

impl Serialize for DefinitionDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for DefinitionDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DefinitionDigestError {
    #[error("definition digest is {actual} bytes; expected 64 lowercase hexadecimal digits")]
    Length { actual: usize },
    #[error("definition digest contains uppercase hexadecimal")]
    Uppercase,
    #[error("definition digest has a non-hexadecimal byte at offset {index}")]
    InvalidHex { index: usize },
}

/// Error produced while applying the shared deterministic JSON encoding.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CanonicalEncodingError {
    #[error("canonical JSON is larger than the {max}-byte limit")]
    TooLarge { max: usize },
}

/// Deterministically encode JSON: object keys are bytewise sorted, arrays keep
/// their order, and scalar spelling is provided by `serde_json`.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, CanonicalEncodingError> {
    canonical_json_bytes_with_limit(value, usize::MAX)
}

/// As [`canonical_json_bytes`], but fail before retaining more than `max`
/// bytes.  This is the form used at all untrusted value boundaries.
pub fn canonical_json_bytes_with_limit(
    value: &Value,
    max: usize,
) -> Result<Vec<u8>, CanonicalEncodingError> {
    let mut output = Vec::new();
    encode_canonical_json(value, &mut output, max)?;
    Ok(output)
}

fn push_bytes(
    output: &mut Vec<u8>,
    bytes: &[u8],
    max: usize,
) -> Result<(), CanonicalEncodingError> {
    let new_len = output
        .len()
        .checked_add(bytes.len())
        .ok_or(CanonicalEncodingError::TooLarge { max })?;
    if new_len > max {
        return Err(CanonicalEncodingError::TooLarge { max });
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn encode_canonical_json(
    value: &Value,
    output: &mut Vec<u8>,
    max: usize,
) -> Result<(), CanonicalEncodingError> {
    match value {
        Value::Null => push_bytes(output, b"null", max),
        Value::Bool(value) => push_bytes(output, if *value { b"true" } else { b"false" }, max),
        Value::Number(number) => push_bytes(output, number.to_string().as_bytes(), max),
        Value::String(value) => {
            let encoded =
                serde_json::to_string(value).expect("serializing a JSON string cannot fail");
            push_bytes(output, encoded.as_bytes(), max)
        }
        Value::Array(values) => {
            push_bytes(output, b"[", max)?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    push_bytes(output, b",", max)?;
                }
                encode_canonical_json(value, output, max)?;
            }
            push_bytes(output, b"]", max)
        }
        Value::Object(values) => {
            push_bytes(output, b"{", max)?;
            let mut keys: Vec<&str> = values.keys().map(String::as_str).collect();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    push_bytes(output, b",", max)?;
                }
                let encoded =
                    serde_json::to_string(key).expect("serializing a JSON object key cannot fail");
                push_bytes(output, encoded.as_bytes(), max)?;
                push_bytes(output, b":", max)?;
                encode_canonical_json(&values[key], output, max)?;
            }
            push_bytes(output, b"}", max)
        }
    }
}

fn hash_parts<'a>(domain: &[u8], parts: impl IntoIterator<Item = &'a [u8]>) -> DefinitionDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"rustmush.mechanics.sha256.v1\0");
    update_hash_part(&mut hasher, domain);
    for part in parts {
        update_hash_part(&mut hasher, part);
    }
    DefinitionDigest(hasher.finalize().into())
}

fn update_hash_part(hasher: &mut Sha256, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
}

/// The stable owner/kind/id tuple.  Version and digest deliberately live in
/// [`DefinitionRef`] so lookup cannot accidentally drop exact identity.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionKey {
    pub owner: OwnerId,
    pub kind: DefinitionKind,
    pub id: DefinitionId,
}

impl DefinitionKey {
    #[must_use]
    pub const fn new(owner: OwnerId, kind: DefinitionKind, id: DefinitionId) -> Self {
        Self { owner, kind, id }
    }
}

impl fmt::Display for DefinitionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.owner, self.kind, self.id)
    }
}

/// A key and version without a claimed digest.  It is intentionally not a
/// wire type: legacy adapters must already know the owner/kind context before
/// they may explicitly resolve it through a frozen registry.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DefinitionLocator {
    pub key: DefinitionKey,
    pub version: u32,
}

impl DefinitionLocator {
    pub fn new(key: DefinitionKey, version: u32) -> Result<Self, DefinitionDescriptorError> {
        if version == 0 {
            return Err(DefinitionDescriptorError::ZeroVersion);
        }
        Ok(Self { key, version })
    }
}

/// Exact definition identity.  All persisted mechanical state carries this
/// complete value.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "DefinitionRefWire", deny_unknown_fields)]
pub struct DefinitionRef {
    key: DefinitionKey,
    version: u32,
    digest: DefinitionDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionRefWire {
    key: DefinitionKey,
    version: u32,
    digest: DefinitionDigest,
}

impl TryFrom<DefinitionRefWire> for DefinitionRef {
    type Error = DefinitionDescriptorError;

    fn try_from(value: DefinitionRefWire) -> Result<Self, Self::Error> {
        Self::new(value.key, value.version, value.digest)
    }
}

impl DefinitionRef {
    pub fn new(
        key: DefinitionKey,
        version: u32,
        digest: DefinitionDigest,
    ) -> Result<Self, DefinitionDescriptorError> {
        if version == 0 {
            return Err(DefinitionDescriptorError::ZeroVersion);
        }
        Ok(Self {
            key,
            version,
            digest,
        })
    }

    #[must_use]
    pub fn locator(&self) -> DefinitionLocator {
        DefinitionLocator {
            key: self.key.clone(),
            version: self.version,
        }
    }

    #[must_use]
    pub const fn key(&self) -> &DefinitionKey {
        &self.key
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub const fn digest(&self) -> DefinitionDigest {
        self.digest
    }
}

impl fmt::Display for DefinitionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}#{}", self.key, self.version, self.digest)
    }
}

/// Rules-owned data with the exact schema identity required to interpret it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "RulesValueWire", deny_unknown_fields)]
pub struct RulesValue {
    schema: DefinitionRef,
    data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesValueWire {
    schema: DefinitionRef,
    data: Value,
}

impl TryFrom<RulesValueWire> for RulesValue {
    type Error = RulesValueError;

    fn try_from(value: RulesValueWire) -> Result<Self, Self::Error> {
        Self::new(value.schema, value.data)
    }
}

impl RulesValue {
    pub fn new(schema: DefinitionRef, data: Value) -> Result<Self, RulesValueError> {
        Self::new_with_limit(schema, data, MAX_RULES_VALUE_BYTES)
    }

    pub fn new_with_limit(
        schema: DefinitionRef,
        data: Value,
        max_bytes: usize,
    ) -> Result<Self, RulesValueError> {
        let max_bytes = max_bytes.min(MAX_RULES_VALUE_BYTES);
        canonical_json_bytes_with_limit(&data, max_bytes)
            .map_err(|_| RulesValueError::TooLarge { max: max_bytes })?;
        Ok(Self { schema, data })
    }

    #[must_use]
    pub const fn schema(&self) -> &DefinitionRef {
        &self.schema
    }

    #[must_use]
    pub const fn data(&self) -> &Value {
        &self.data
    }

    #[must_use]
    pub fn into_data(self) -> Value {
        self.data
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RulesValueError {
    #[error("rules value is larger than the {max}-byte limit")]
    TooLarge { max: usize },
}

/// Registration input.  It contains neither owner nor digest nor a self-ref;
/// the registrar stamps the owner and the builder computes exact identity.
#[derive(Clone, Debug, PartialEq)]
pub struct DefinitionDescriptor {
    kind: DefinitionKind,
    id: DefinitionId,
    version: u32,
    payload: Value,
    references: Vec<DefinitionRef>,
    current: bool,
}

impl DefinitionDescriptor {
    pub fn new(
        kind: DefinitionKind,
        id: DefinitionId,
        version: u32,
        payload: Value,
    ) -> Result<Self, DefinitionDescriptorError> {
        if version == 0 {
            return Err(DefinitionDescriptorError::ZeroVersion);
        }
        Ok(Self {
            kind,
            id,
            version,
            payload,
            references: Vec::new(),
            current: false,
        })
    }

    /// Serialize a typed definition body into the canonical JSON value carried
    /// by a descriptor.  Exact identity is still produced only by registering
    /// the descriptor with an owner-stamping builder.
    pub fn from_serializable<T: Serialize>(
        kind: DefinitionKind,
        id: DefinitionId,
        version: u32,
        payload: &T,
    ) -> Result<Self, DefinitionDescriptorError> {
        let payload = serde_json::to_value(payload).map_err(|error| {
            DefinitionDescriptorError::Serialization {
                message: bounded_message(error.to_string()),
            }
        })?;
        Self::new(kind, id, version, payload)
    }

    #[must_use]
    pub fn with_references(mut self, references: Vec<DefinitionRef>) -> Self {
        self.references = references;
        self
    }

    /// Select this exact version as the key's authoring default.  Frozen
    /// lookup never infers a current version from numeric ordering.
    #[must_use]
    pub fn as_current(mut self) -> Self {
        self.current = true;
        self
    }

    #[must_use]
    pub const fn kind(&self) -> &DefinitionKind {
        &self.kind
    }

    #[must_use]
    pub const fn id(&self) -> &DefinitionId {
        &self.id
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub const fn payload(&self) -> &Value {
        &self.payload
    }

    #[must_use]
    pub fn references(&self) -> &[DefinitionRef] {
        &self.references
    }

    #[must_use]
    pub const fn is_current(&self) -> bool {
        self.current
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DefinitionDescriptorError {
    #[error("definition version zero is reserved")]
    ZeroVersion,
    #[error("definition payload could not be serialized: {message}")]
    Serialization { message: String },
}

/// A validated definition stored in a frozen registry.
#[derive(Clone, Debug, PartialEq)]
pub struct DefinitionEntry {
    reference: DefinitionRef,
    payload: Value,
    references: Vec<DefinitionRef>,
    encoded_bytes: usize,
    current: bool,
}

impl DefinitionEntry {
    #[must_use]
    pub const fn reference(&self) -> &DefinitionRef {
        &self.reference
    }

    #[must_use]
    pub const fn payload(&self) -> &Value {
        &self.payload
    }

    #[must_use]
    pub fn references(&self) -> &[DefinitionRef] {
        &self.references
    }

    #[must_use]
    pub const fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }

    #[must_use]
    pub const fn is_current(&self) -> bool {
        self.current
    }
}

/// Read-only context supplied to a rules provider's semantic definition
/// validator while the deployment registry is frozen.
///
/// The erased registry owns canonical identity, size bounds, and exact
/// dependency availability.  The provider validator owns the meaning of its
/// payload.  Giving it read-only access to the complete candidate set lets a
/// typed catalog verify that the payload's derived references and any
/// dependency-specific constraints agree with the descriptor before the
/// registry can become authoritative.
pub struct DefinitionValidationContext<'a> {
    definition: &'a DefinitionEntry,
    definitions: &'a BTreeMap<DefinitionLocator, DefinitionEntry>,
}

impl<'a> DefinitionValidationContext<'a> {
    /// The definition currently being validated.
    #[must_use]
    pub const fn definition(&self) -> &'a DefinitionEntry {
        self.definition
    }

    /// Resolve one exact dependency from the complete candidate registry.
    /// Structural freeze validation runs before provider validation, so every
    /// declared reference is guaranteed to resolve exactly.
    #[allow(clippy::result_large_err)]
    pub fn get_exact(
        &self,
        reference: &DefinitionRef,
    ) -> Result<&'a DefinitionEntry, DefinitionLookupError> {
        let Some(entry) = self.definitions.get(&reference.locator()) else {
            return Err(DefinitionLookupError::Missing {
                requested: reference.clone(),
            });
        };
        if entry.reference.digest != reference.digest {
            return Err(DefinitionLookupError::DigestMismatch {
                requested: reference.clone(),
                registered: entry.reference.clone(),
            });
        }
        Ok(entry)
    }
}

/// Provider-owned semantic validation for one definition kind.
///
/// Validators must be pure and deterministic. They run once per matching
/// definition, in exact registry order, during module composition. Returning
/// `Ok(())` certifies that the provider has strictly decoded and completely
/// validated the payload, including agreement between references derived from
/// the typed body and [`DefinitionEntry::references`].
pub trait DefinitionValidator: Send + Sync {
    fn validate(&self, context: &DefinitionValidationContext<'_>) -> Result<(), String>;
}

impl<F> DefinitionValidator for F
where
    F: Fn(&DefinitionValidationContext<'_>) -> Result<(), String> + Send + Sync,
{
    fn validate(&self, context: &DefinitionValidationContext<'_>) -> Result<(), String> {
        self(context)
    }
}

/// Mutable only during module composition.
pub struct DefinitionRegistryBuilder {
    limits: MechanicsLimits,
    entries: BTreeMap<DefinitionLocator, DefinitionEntry>,
    owner_usage: BTreeMap<OwnerId, (usize, usize)>,
    current: BTreeMap<DefinitionKey, DefinitionRef>,
    provider_validation_required: BTreeSet<(OwnerId, DefinitionKind)>,
    provider_validators: BTreeMap<(OwnerId, DefinitionKind), Arc<dyn DefinitionValidator>>,
    total_bytes: usize,
}

impl fmt::Debug for DefinitionRegistryBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefinitionRegistryBuilder")
            .field("limits", &self.limits)
            .field("entries", &self.entries)
            .field("owner_usage", &self.owner_usage)
            .field("current", &self.current)
            .field(
                "provider_validation_required",
                &self.provider_validation_required,
            )
            .field(
                "provider_validator_keys",
                &self.provider_validators.keys().collect::<Vec<_>>(),
            )
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

impl Default for DefinitionRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// Composition errors intentionally retain all conflicting exact identities so
// an operator can diagnose drift without re-reading mutable builder state.
#[allow(clippy::result_large_err)]
impl DefinitionRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(MechanicsLimits::default())
    }

    #[must_use]
    pub fn with_limits(limits: MechanicsLimits) -> Self {
        Self {
            limits: limits.capped(),
            entries: BTreeMap::new(),
            owner_usage: BTreeMap::new(),
            current: BTreeMap::new(),
            provider_validation_required: BTreeSet::new(),
            provider_validators: BTreeMap::new(),
            total_bytes: 0,
        }
    }

    /// Register through an identity-stamping provider registrar. Unlike the
    /// public low-level [`Self::register`] helper, every definition admitted by
    /// this path must have one owner/kind validator before freeze can succeed.
    pub(crate) fn register_provider_definition(
        &mut self,
        owner: OwnerId,
        descriptor: DefinitionDescriptor,
    ) -> Result<DefinitionRef, DefinitionRegistryError> {
        let validation_key = (owner.clone(), descriptor.kind().clone());
        let definition = self.register(owner, descriptor)?;
        self.provider_validation_required.insert(validation_key);
        Ok(definition)
    }

    /// Register the one provider-owned semantic validator for an owner/kind
    /// pair. The owner is stamped by [`crate::module::Registrar`].
    pub(crate) fn register_provider_validator(
        &mut self,
        owner: OwnerId,
        kind: DefinitionKind,
        validator: Arc<dyn DefinitionValidator>,
    ) -> Result<(), DefinitionRegistryError> {
        let key = (owner, kind);
        if self.provider_validators.contains_key(&key) {
            return Err(DefinitionRegistryError::DuplicateValidator {
                owner: key.0,
                kind: key.1,
            });
        }
        self.provider_validators.insert(key, validator);
        Ok(())
    }

    pub fn register(
        &mut self,
        owner: OwnerId,
        descriptor: DefinitionDescriptor,
    ) -> Result<DefinitionRef, DefinitionRegistryError> {
        let DefinitionDescriptor {
            kind,
            id,
            version,
            payload,
            mut references,
            current,
        } = descriptor;
        if references.len() > self.limits.references_per_definition {
            return Err(DefinitionRegistryError::TooManyReferences {
                count: references.len(),
                max: self.limits.references_per_definition,
            });
        }
        references.sort();
        if references.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(DefinitionRegistryError::DuplicateReference);
        }
        let payload_bytes = canonical_json_bytes_with_limit(&payload, self.limits.definition_bytes)
            .map_err(|_| DefinitionRegistryError::DefinitionTooLarge {
                max: self.limits.definition_bytes,
            })?;
        let key = DefinitionKey::new(owner.clone(), kind, id);
        let digest = definition_digest(&key, version, &payload_bytes, &references);
        let reference = DefinitionRef {
            key: key.clone(),
            version,
            digest,
        };
        if references
            .iter()
            .any(|dependency| dependency.key == key && dependency.version == version)
        {
            return Err(DefinitionRegistryError::SelfReference {
                definition: reference,
            });
        }
        let locator = reference.locator();
        if let Some(existing) = self.entries.get(&locator) {
            return if existing.reference.digest == reference.digest {
                Err(DefinitionRegistryError::Duplicate {
                    definition: reference,
                })
            } else {
                Err(DefinitionRegistryError::DigestConflict {
                    locator,
                    registered: existing.reference.digest,
                    attempted: reference.digest,
                })
            };
        }
        if current && let Some(registered) = self.current.get(&key) {
            return Err(DefinitionRegistryError::DuplicateCurrent {
                key,
                registered: registered.clone(),
                attempted: reference,
            });
        }
        if self.entries.len() >= self.limits.definitions_total {
            return Err(DefinitionRegistryError::DefinitionLimit {
                max: self.limits.definitions_total,
            });
        }
        if !self.owner_usage.contains_key(&owner)
            && self.owner_usage.len() >= self.limits.definition_owners
        {
            return Err(DefinitionRegistryError::OwnerLimit {
                max: self.limits.definition_owners,
            });
        }
        let encoded_bytes = definition_encoded_len(&key, payload_bytes.len(), &references).ok_or(
            DefinitionRegistryError::DefinitionTooLarge {
                max: self.limits.definition_bytes,
            },
        )?;
        if encoded_bytes > self.limits.definition_bytes {
            return Err(DefinitionRegistryError::DefinitionTooLarge {
                max: self.limits.definition_bytes,
            });
        }
        let (owner_count, owner_bytes) = self.owner_usage.get(&owner).copied().unwrap_or_default();
        if owner_count >= self.limits.definitions_per_owner {
            return Err(DefinitionRegistryError::OwnerDefinitionLimit {
                owner,
                max: self.limits.definitions_per_owner,
            });
        }
        let new_owner_bytes = owner_bytes.checked_add(encoded_bytes).ok_or_else(|| {
            DefinitionRegistryError::OwnerBytesExceeded {
                owner: owner.clone(),
                max: self.limits.definition_bytes_per_owner,
            }
        })?;
        if new_owner_bytes > self.limits.definition_bytes_per_owner {
            return Err(DefinitionRegistryError::OwnerBytesExceeded {
                owner,
                max: self.limits.definition_bytes_per_owner,
            });
        }
        let new_total_bytes = self.total_bytes.checked_add(encoded_bytes).ok_or(
            DefinitionRegistryError::TotalBytesExceeded {
                max: self.limits.definition_bytes_total,
            },
        )?;
        if new_total_bytes > self.limits.definition_bytes_total {
            return Err(DefinitionRegistryError::TotalBytesExceeded {
                max: self.limits.definition_bytes_total,
            });
        }
        self.owner_usage.insert(
            reference.key.owner.clone(),
            (owner_count + 1, new_owner_bytes),
        );
        self.total_bytes = new_total_bytes;
        if current {
            self.current.insert(key, reference.clone());
        }
        self.entries.insert(
            locator,
            DefinitionEntry {
                reference: reference.clone(),
                payload,
                references,
                encoded_bytes,
                current,
            },
        );
        Ok(reference)
    }

    pub fn freeze(self) -> Result<DefinitionRegistry, DefinitionRegistryError> {
        for entry in self.entries.values() {
            for dependency in &entry.references {
                let Some(found) = self.entries.get(&dependency.locator()) else {
                    return Err(DefinitionRegistryError::MissingReference {
                        definition: entry.reference.clone(),
                        dependency: dependency.clone(),
                    });
                };
                if found.reference.digest != dependency.digest {
                    return Err(DefinitionRegistryError::ReferenceDigestMismatch {
                        definition: entry.reference.clone(),
                        requested: dependency.clone(),
                        registered: found.reference.clone(),
                    });
                }
            }
        }
        for (owner, kind) in &self.provider_validation_required {
            let Some(validator) = self.provider_validators.get(&(owner.clone(), kind.clone()))
            else {
                return Err(DefinitionRegistryError::MissingValidator {
                    owner: owner.clone(),
                    kind: kind.clone(),
                });
            };
            for entry in self.entries.values().filter(|entry| {
                entry.reference.key.owner == *owner && entry.reference.key.kind == *kind
            }) {
                let context = DefinitionValidationContext {
                    definition: entry,
                    definitions: &self.entries,
                };
                validator.validate(&context).map_err(|message| {
                    DefinitionRegistryError::SemanticValidation {
                        definition: entry.reference.clone(),
                        message: bounded_message(message),
                    }
                })?;
            }
        }
        let provider_validated_definitions = self
            .entries
            .values()
            .filter(|entry| {
                self.provider_validation_required.contains(&(
                    entry.reference.key.owner.clone(),
                    entry.reference.key.kind.clone(),
                ))
            })
            .count();
        let fingerprint = definition_registry_fingerprint(&self.entries);
        let summary = DefinitionRegistrySummary {
            definition_count: self.entries.len().try_into().unwrap_or(u32::MAX),
            owner_count: self.owner_usage.len().try_into().unwrap_or(u32::MAX),
            current_count: self.current.len().try_into().unwrap_or(u32::MAX),
            encoded_bytes: self.total_bytes.try_into().unwrap_or(u64::MAX),
            fingerprint,
        };
        Ok(DefinitionRegistry {
            entries: self.entries,
            current: self.current,
            summary,
            provider_validated_definitions,
            owner_readiness: None,
        })
    }
}

fn definition_digest(
    key: &DefinitionKey,
    version: u32,
    payload: &[u8],
    references: &[DefinitionRef],
) -> DefinitionDigest {
    let owner = key.owner.as_str().as_bytes();
    let kind = key.kind.as_str().as_bytes();
    let id = key.id.as_str().as_bytes();
    let version = version.to_be_bytes();
    let mut reference_bytes = Vec::with_capacity(references.len().saturating_mul(128));
    for reference in references {
        append_reference_bytes(&mut reference_bytes, reference);
    }
    hash_parts(
        b"definition.v1",
        [owner, kind, id, &version, payload, &reference_bytes],
    )
}

fn definition_encoded_len(
    key: &DefinitionKey,
    payload_bytes: usize,
    references: &[DefinitionRef],
) -> Option<usize> {
    // Six length-prefixed hash fields: owner, kind, id, version, payload, and
    // the complete ordered dependency block.
    let mut length = 6_usize.checked_mul(std::mem::size_of::<u64>())?;
    for component in [key.owner.as_str(), key.kind.as_str(), key.id.as_str()] {
        length = length.checked_add(component.len())?;
    }
    length = length.checked_add(std::mem::size_of::<u32>())?;
    length = length.checked_add(payload_bytes)?;
    for reference in references {
        // Each exact ref encodes three length-prefixed strings, a version, and
        // a fixed digest.
        length = length.checked_add(3 * std::mem::size_of::<u64>())?;
        for component in [
            reference.key.owner.as_str(),
            reference.key.kind.as_str(),
            reference.key.id.as_str(),
        ] {
            length = length.checked_add(component.len())?;
        }
        length = length.checked_add(std::mem::size_of::<u32>())?;
        length = length.checked_add(reference.digest.as_bytes().len())?;
    }
    Some(length)
}

fn append_reference_bytes(output: &mut Vec<u8>, reference: &DefinitionRef) {
    for bytes in [
        reference.key.owner.as_str().as_bytes(),
        reference.key.kind.as_str().as_bytes(),
        reference.key.id.as_str().as_bytes(),
    ] {
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        output.extend_from_slice(&length.to_be_bytes());
        output.extend_from_slice(bytes);
    }
    output.extend_from_slice(&reference.version.to_be_bytes());
    output.extend_from_slice(reference.digest.as_bytes());
}

fn definition_registry_fingerprint(
    entries: &BTreeMap<DefinitionLocator, DefinitionEntry>,
) -> DefinitionDigest {
    let mut encoded = Vec::with_capacity(entries.len().saturating_mul(128));
    for entry in entries.values() {
        append_reference_bytes(&mut encoded, &entry.reference);
        encoded.push(u8::from(entry.current));
    }
    hash_parts(b"definition_registry.v1", [&encoded[..]])
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DefinitionRegistryError {
    #[error("definition {definition} was registered more than once")]
    Duplicate { definition: DefinitionRef },
    #[error(
        "definition {locator:?} changes digest from {registered} to {attempted} without changing version"
    )]
    DigestConflict {
        locator: DefinitionLocator,
        registered: DefinitionDigest,
        attempted: DefinitionDigest,
    },
    #[error(
        "definition key {key} has two authoring-current versions: {registered} and {attempted}"
    )]
    DuplicateCurrent {
        key: DefinitionKey,
        registered: DefinitionRef,
        attempted: DefinitionRef,
    },
    #[error("a definition declared the same exact dependency more than once")]
    DuplicateReference,
    #[error("definition {definition} declares itself as a dependency")]
    SelfReference { definition: DefinitionRef },
    #[error("definition has {count} dependencies; maximum is {max}")]
    TooManyReferences { count: usize, max: usize },
    #[error("definition payload exceeds the {max}-byte limit")]
    DefinitionTooLarge { max: usize },
    #[error("definition registry has reached its {max}-definition limit")]
    DefinitionLimit { max: usize },
    #[error("definition registry has reached its {max}-owner limit")]
    OwnerLimit { max: usize },
    #[error("owner {owner} exceeds its {max}-definition limit")]
    OwnerDefinitionLimit { owner: OwnerId, max: usize },
    #[error("owner {owner} exceeds its {max}-byte definition budget")]
    OwnerBytesExceeded { owner: OwnerId, max: usize },
    #[error("definition registry exceeds its {max}-byte total budget")]
    TotalBytesExceeded { max: usize },
    #[error("definition {definition} depends on missing exact definition {dependency}")]
    MissingReference {
        definition: DefinitionRef,
        dependency: DefinitionRef,
    },
    #[error(
        "definition {definition} requested dependency {requested}, but registry contains {registered}"
    )]
    ReferenceDigestMismatch {
        definition: DefinitionRef,
        requested: DefinitionRef,
        registered: DefinitionRef,
    },
    #[error("definition validator for owner {owner} kind {kind} was registered more than once")]
    DuplicateValidator {
        owner: OwnerId,
        kind: DefinitionKind,
    },
    #[error("owner {owner} did not register a semantic validator for definition kind {kind}")]
    MissingValidator {
        owner: OwnerId,
        kind: DefinitionKind,
    },
    #[error("definition {definition} failed provider semantic validation: {message}")]
    SemanticValidation {
        definition: DefinitionRef,
        message: String,
    },
}

/// Immutable definitions and their deterministic composition summary.
#[derive(Clone, Debug)]
pub struct DefinitionRegistry {
    entries: BTreeMap<DefinitionLocator, DefinitionEntry>,
    current: BTreeMap<DefinitionKey, DefinitionRef>,
    summary: DefinitionRegistrySummary,
    provider_validated_definitions: usize,
    /// Bound only by full module composition. Standalone structural registries
    /// intentionally remain useful for pure validation and identity fixtures.
    owner_readiness: Option<MechanicalOwnerReadiness>,
}

#[allow(clippy::result_large_err)]
impl DefinitionRegistry {
    /// Resolve exact immutable metadata without asserting that its owner may
    /// execute mechanics. This is the deliberate structural path for recovery
    /// diagnostics and composition inspection; callers must use [`Self::get_exact`]
    /// before interpreting a rules-bearing instance or invoking provider code.
    pub fn inspect_exact(
        &self,
        reference: &DefinitionRef,
    ) -> Result<&DefinitionEntry, DefinitionLookupError> {
        let Some(entry) = self.entries.get(&reference.locator()) else {
            return Err(DefinitionLookupError::Missing {
                requested: reference.clone(),
            });
        };
        if entry.reference.digest != reference.digest {
            return Err(DefinitionLookupError::DigestMismatch {
                requested: reference.clone(),
                registered: entry.reference.clone(),
            });
        }
        Ok(entry)
    }

    /// Resolve exact metadata and require its composed owner to be mechanically
    /// ready. A standalone structural registry is not execution authority; it
    /// returns `RegistryUnavailable`. A frozen owner returns a typed readiness
    /// error, never `Missing`.
    pub fn get_exact(
        &self,
        reference: &DefinitionRef,
    ) -> Result<&DefinitionEntry, DefinitionLookupError> {
        let entry = self.inspect_exact(reference)?;
        let readiness =
            self.owner_readiness
                .as_ref()
                .ok_or_else(|| DefinitionLookupError::OwnerNotReady {
                    requested: reference.clone(),
                    source: Box::new(MechanicalOwnerReadinessError::RegistryUnavailable),
                })?;
        readiness
            .require_ready(&entry.reference.key.owner)
            .map_err(|source| DefinitionLookupError::OwnerNotReady {
                requested: reference.clone(),
                source: Box::new(source),
            })?;
        Ok(entry)
    }

    pub(crate) fn bind_owner_readiness(&mut self, readiness: MechanicalOwnerReadiness) {
        debug_assert!(self.owner_readiness.is_none());
        self.owner_readiness = Some(readiness);
    }

    /// The process-local composition latch bound to executable lookups.
    ///
    /// This stays crate-private: callers that need executable mechanics must
    /// go through the composed [`Registry`](crate::Registry), rather than
    /// assembling independently frozen registries into a look-alike runtime.
    pub(crate) const fn owner_readiness(&self) -> Option<&MechanicalOwnerReadiness> {
        self.owner_readiness.as_ref()
    }

    /// Explicit compatibility resolution after an adapter has supplied full
    /// owner/kind/id/version context.  There is no id-only inference API.
    pub fn resolve_locator(
        &self,
        locator: &DefinitionLocator,
    ) -> Result<DefinitionRef, DefinitionLookupError> {
        self.entries
            .get(locator)
            .map(|entry| entry.reference.clone())
            .ok_or_else(|| DefinitionLookupError::MissingLocator {
                locator: locator.clone(),
            })
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &DefinitionEntry> {
        self.entries.values()
    }

    /// Return an explicitly selected authoring version.  `None` means the
    /// provider did not nominate one; numeric "latest" inference is forbidden.
    #[must_use]
    pub fn current(&self, key: &DefinitionKey) -> Option<&DefinitionRef> {
        self.current.get(key)
    }

    #[must_use]
    pub const fn summary(&self) -> &DefinitionRegistrySummary {
        &self.summary
    }

    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.summary.fingerprint
    }

    /// Whether every definition was admitted through a provider registrar and
    /// passed its owner/kind semantic validator. Deployment registries frozen
    /// by module composition always return `true`; low-level registries used
    /// to compute identities or validate structural fixtures may return
    /// `false` without losing that utility.
    #[must_use]
    pub fn is_fully_provider_validated(&self) -> bool {
        self.provider_validated_definitions == self.entries.len()
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DefinitionLookupError {
    #[error("exact definition {requested} is unavailable")]
    Missing { requested: DefinitionRef },
    #[error(
        "definition {requested} has the wrong digest; registered exact identity is {registered}"
    )]
    DigestMismatch {
        requested: DefinitionRef,
        registered: DefinitionRef,
    },
    #[error("exact definition {requested} is mechanically unavailable: {source}")]
    OwnerNotReady {
        requested: DefinitionRef,
        source: Box<MechanicalOwnerReadinessError>,
    },
    #[error("definition locator {locator:?} is unavailable")]
    MissingLocator { locator: DefinitionLocator },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionRegistrySummary {
    pub definition_count: u32,
    pub owner_count: u32,
    pub current_count: u32,
    pub encoded_bytes: u64,
    pub fingerprint: DefinitionDigest,
}

/// Directional, exact bridge identity.  No executable callback is admitted in
/// K1; this registry only establishes unambiguous ownership and availability.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeKey {
    pub source_owner: OwnerId,
    pub target_owner: OwnerId,
    pub interaction: DefinitionRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeDescriptor {
    definition: DefinitionRef,
    key: BridgeKey,
}

impl BridgeDescriptor {
    #[must_use]
    pub const fn new(
        definition: DefinitionRef,
        source_owner: OwnerId,
        target_owner: OwnerId,
        interaction: DefinitionRef,
    ) -> Self {
        Self {
            definition,
            key: BridgeKey {
                source_owner,
                target_owner,
                interaction,
            },
        }
    }

    #[must_use]
    pub const fn key(&self) -> &BridgeKey {
        &self.key
    }

    #[must_use]
    pub const fn definition(&self) -> &DefinitionRef {
        &self.definition
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeEntry {
    provider: OwnerId,
    definition: DefinitionRef,
    key: BridgeKey,
}

impl BridgeEntry {
    #[must_use]
    pub const fn provider(&self) -> &OwnerId {
        &self.provider
    }

    #[must_use]
    pub const fn key(&self) -> &BridgeKey {
        &self.key
    }

    #[must_use]
    pub const fn definition(&self) -> &DefinitionRef {
        &self.definition
    }
}

#[derive(Debug)]
pub struct BridgeRegistryBuilder {
    limits: MechanicsLimits,
    entries: BTreeMap<BridgeKey, BridgeEntry>,
}

impl Default for BridgeRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::result_large_err)]
impl BridgeRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(MechanicsLimits::default())
    }

    #[must_use]
    pub fn with_limits(limits: MechanicsLimits) -> Self {
        Self {
            limits: limits.capped(),
            entries: BTreeMap::new(),
        }
    }

    pub fn register(
        &mut self,
        provider: OwnerId,
        descriptor: BridgeDescriptor,
    ) -> Result<(), BridgeRegistryError> {
        let BridgeDescriptor { definition, key } = descriptor;
        if definition.key.owner != provider {
            return Err(BridgeRegistryError::OwnerMismatch {
                provider,
                definition,
            });
        }
        if definition.key.kind.as_str() != "bridge" {
            return Err(BridgeRegistryError::WrongDefinitionKind { definition });
        }
        if key.source_owner == key.target_owner {
            return Err(BridgeRegistryError::SameOwner {
                owner: key.source_owner,
            });
        }
        if self.entries.len() >= self.limits.bridges {
            return Err(BridgeRegistryError::Limit {
                max: self.limits.bridges,
            });
        }
        if let Some(existing) = self.entries.get(&key) {
            return Err(BridgeRegistryError::Duplicate {
                key,
                registered_provider: existing.provider.clone(),
                attempted_provider: provider,
            });
        }
        self.entries.insert(
            key.clone(),
            BridgeEntry {
                provider,
                definition,
                key,
            },
        );
        Ok(())
    }

    pub fn freeze(
        self,
        definitions: &DefinitionRegistry,
    ) -> Result<BridgeRegistry, BridgeRegistryError> {
        self.freeze_with_readiness(definitions, None)
    }

    fn freeze_with_readiness(
        self,
        definitions: &DefinitionRegistry,
        owner_readiness: Option<MechanicalOwnerReadiness>,
    ) -> Result<BridgeRegistry, BridgeRegistryError> {
        if let Some(readiness) = &owner_readiness {
            let definition_readiness = definitions
                .owner_readiness()
                .ok_or(BridgeRegistryError::RegistryCompositionMismatch)?;
            if !readiness.shares_latch_with(definition_readiness) {
                return Err(BridgeRegistryError::RegistryCompositionMismatch);
            }
        }
        for entry in self.entries.values() {
            let lookup = |reference: &DefinitionRef| {
                if owner_readiness.is_some() {
                    definitions.get_exact(reference)
                } else {
                    definitions.inspect_exact(reference)
                }
            };
            lookup(&entry.definition).map_err(|source| BridgeRegistryError::MissingDefinition {
                definition: entry.definition.clone(),
                source,
            })?;
            lookup(&entry.key.interaction).map_err(|source| {
                BridgeRegistryError::MissingInteraction {
                    key: entry.key.clone(),
                    source,
                }
            })?;
        }
        let fingerprint = bridge_registry_fingerprint(&self.entries);
        Ok(BridgeRegistry {
            entries: self.entries,
            fingerprint,
            owner_readiness,
        })
    }

    /// Freeze bridges as part of a concrete module composition. In addition
    /// to exact definition availability, both endpoint owners must name loaded
    /// modules; otherwise a declaration could claim a bridge to a provider
    /// which cannot participate in the interaction at runtime.
    pub(crate) fn freeze_for_owners_with_readiness(
        self,
        definitions: &DefinitionRegistry,
        module_owners: &BTreeSet<OwnerId>,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<BridgeRegistry, BridgeRegistryError> {
        for entry in self.entries.values() {
            for (endpoint, owner) in [
                ("source", &entry.key.source_owner),
                ("target", &entry.key.target_owner),
            ] {
                if !module_owners.contains(owner) {
                    return Err(BridgeRegistryError::MissingEndpointOwner {
                        endpoint,
                        owner: owner.clone(),
                        key: Box::new(entry.key.clone()),
                    });
                }
            }
        }
        self.freeze_with_readiness(definitions, Some(owner_readiness))
    }
}

fn bridge_registry_fingerprint(entries: &BTreeMap<BridgeKey, BridgeEntry>) -> DefinitionDigest {
    let mut encoded = Vec::with_capacity(entries.len().saturating_mul(192));
    for entry in entries.values() {
        for value in [
            entry.provider.as_str(),
            entry.key.source_owner.as_str(),
            entry.key.target_owner.as_str(),
        ] {
            let bytes = value.as_bytes();
            encoded
                .extend_from_slice(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
            encoded.extend_from_slice(bytes);
        }
        append_reference_bytes(&mut encoded, &entry.key.interaction);
        append_reference_bytes(&mut encoded, &entry.definition);
    }
    hash_parts(b"bridge_registry.v1", [&encoded[..]])
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BridgeRegistryError {
    #[error("bridge registry and definitions do not belong to one module composition")]
    RegistryCompositionMismatch,
    #[error("bridge provider {provider} does not own bridge definition {definition}")]
    OwnerMismatch {
        provider: OwnerId,
        definition: DefinitionRef,
    },
    #[error("bridge definition {definition} must have definition kind `bridge`")]
    WrongDefinitionKind { definition: DefinitionRef },
    #[error("a bridge must cross owners; both ends were {owner}")]
    SameOwner { owner: OwnerId },
    #[error("bridge limit of {max} was reached")]
    Limit { max: usize },
    #[error(
        "bridge {key:?} is already provided by {registered_provider}; {attempted_provider} cannot also provide it"
    )]
    Duplicate {
        key: BridgeKey,
        registered_provider: OwnerId,
        attempted_provider: OwnerId,
    },
    #[error("bridge {key:?} refers to an unavailable interaction definition: {source}")]
    MissingInteraction {
        key: BridgeKey,
        source: DefinitionLookupError,
    },
    #[error("bridge definition {definition} is unavailable: {source}")]
    MissingDefinition {
        definition: DefinitionRef,
        source: DefinitionLookupError,
    },
    #[error(
        "bridge {key:?} names {endpoint} endpoint owner {owner}, but that module is not present in the composed roster"
    )]
    MissingEndpointOwner {
        endpoint: &'static str,
        owner: OwnerId,
        key: Box<BridgeKey>,
    },
}

#[derive(Clone, Debug)]
pub struct BridgeRegistry {
    entries: BTreeMap<BridgeKey, BridgeEntry>,
    fingerprint: DefinitionDigest,
    owner_readiness: Option<MechanicalOwnerReadiness>,
}

impl BridgeRegistry {
    #[must_use]
    /// Structural metadata lookup only. This does not authorize execution;
    /// executable bridge consumers must use [`Self::require_interaction`].
    pub fn get(&self, key: &BridgeKey) -> Option<&BridgeEntry> {
        self.entries.get(key)
    }

    /// Resolve the only legal route for an owner-to-owner mechanical
    /// interaction. Same-owner work needs no bridge; cross-owner work fails
    /// closed unless this frozen registry contains the exact directional key.
    /// Future action executors consume this route rather than probing bridge
    /// metadata ad hoc.
    pub fn require_interaction(
        &self,
        definitions: &DefinitionRegistry,
        source_owner: &OwnerId,
        target_owner: &OwnerId,
        interaction: &DefinitionRef,
    ) -> Result<InteractionRoute, BridgeLookupError> {
        let readiness = self
            .owner_readiness
            .as_ref()
            .ok_or(BridgeLookupError::RegistryUnavailable)?;
        let definition_readiness = definitions
            .owner_readiness()
            .ok_or(BridgeLookupError::RegistryCompositionMismatch)?;
        if !readiness.shares_latch_with(definition_readiness) {
            return Err(BridgeLookupError::RegistryCompositionMismatch);
        }
        for (role, owner) in [("source", source_owner), ("target", target_owner)] {
            readiness
                .require_ready(owner)
                .map_err(|source| BridgeLookupError::OwnerNotReady {
                    role,
                    owner: owner.clone(),
                    source: Box::new(source),
                })?;
        }
        definitions.get_exact(interaction).map_err(|source| {
            BridgeLookupError::InvalidInteraction {
                interaction: Box::new(interaction.clone()),
                source: Box::new(source),
            }
        })?;
        if source_owner == target_owner {
            return Ok(InteractionRoute::SameOwner);
        }
        let key = BridgeKey {
            source_owner: source_owner.clone(),
            target_owner: target_owner.clone(),
            interaction: interaction.clone(),
        };
        let entry = self
            .entries
            .get(&key)
            .ok_or_else(|| BridgeLookupError::Missing {
                key: Box::new(key.clone()),
            })?;
        readiness.require_ready(&entry.provider).map_err(|source| {
            BridgeLookupError::OwnerNotReady {
                role: "provider",
                owner: entry.provider.clone(),
                source: Box::new(source),
            }
        })?;
        Ok(InteractionRoute::Bridge(entry.definition.clone()))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &BridgeEntry> {
        self.entries.values()
    }

    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }
}

/// Authorization route selected for one exact owner interaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InteractionRoute {
    SameOwner,
    Bridge(DefinitionRef),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BridgeLookupError {
    #[error("bridge execution is unavailable outside module composition")]
    RegistryUnavailable,
    #[error("bridge registry and definitions do not belong to one module composition")]
    RegistryCompositionMismatch,
    #[error("interaction definition {interaction} is unavailable: {source}")]
    InvalidInteraction {
        interaction: Box<DefinitionRef>,
        source: Box<DefinitionLookupError>,
    },
    #[error("bridge {role} owner {owner} is mechanically unavailable: {source}")]
    OwnerNotReady {
        role: &'static str,
        owner: OwnerId,
        source: Box<MechanicalOwnerReadinessError>,
    },
    #[error("no exact bridge is registered for cross-owner interaction {key:?}")]
    Missing { key: Box<BridgeKey> },
}

/// Migration callback constrained to immutable JSON input and no world or
/// persistence handle. Providers must treat it as pure; [`DefinitionMigrationRegistry::dry_run`]
/// invokes each edge twice and rejects differing canonical outputs.
pub type DefinitionMigrationFn =
    Arc<dyn Fn(&Value) -> Result<Value, String> + Send + Sync + 'static>;

/// One explicitly versioned, contiguous definition migration edge. The exact
/// `definition` digest is the provider-declared implementation identity and is
/// included in the frozen migration-registry fingerprint; changing migration
/// code therefore requires changing that definition payload/version.
#[derive(Clone)]
pub struct DefinitionMigrationDescriptor {
    definition: DefinitionRef,
    from: DefinitionRef,
    to: DefinitionRef,
    transform: DefinitionMigrationFn,
}

impl fmt::Debug for DefinitionMigrationDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefinitionMigrationDescriptor")
            .field("definition", &self.definition)
            .field("from", &self.from)
            .field("to", &self.to)
            .field("transform", &"<function>")
            .finish()
    }
}

impl DefinitionMigrationDescriptor {
    #[must_use]
    pub fn new(
        definition: DefinitionRef,
        from: DefinitionRef,
        to: DefinitionRef,
        transform: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            definition,
            from,
            to,
            transform: Arc::new(transform),
        }
    }

    #[must_use]
    pub const fn definition(&self) -> &DefinitionRef {
        &self.definition
    }

    #[must_use]
    pub const fn from(&self) -> &DefinitionRef {
        &self.from
    }

    #[must_use]
    pub const fn to(&self) -> &DefinitionRef {
        &self.to
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct MigrationEdgeKey {
    key: DefinitionKey,
    from_version: u32,
}

#[derive(Clone)]
struct DefinitionMigrationEntry {
    provider: OwnerId,
    definition: DefinitionRef,
    from: DefinitionRef,
    to: DefinitionRef,
    transform: DefinitionMigrationFn,
}

impl fmt::Debug for DefinitionMigrationEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefinitionMigrationEntry")
            .field("provider", &self.provider)
            .field("definition", &self.definition)
            .field("from", &self.from)
            .field("to", &self.to)
            .field("transform", &"<function>")
            .finish()
    }
}

#[derive(Debug)]
pub struct DefinitionMigrationRegistryBuilder {
    limits: MechanicsLimits,
    entries: BTreeMap<MigrationEdgeKey, DefinitionMigrationEntry>,
    counts: BTreeMap<DefinitionKey, usize>,
}

impl Default for DefinitionMigrationRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::result_large_err)]
impl DefinitionMigrationRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(MechanicsLimits::default())
    }

    #[must_use]
    pub fn with_limits(limits: MechanicsLimits) -> Self {
        Self {
            limits: limits.capped(),
            entries: BTreeMap::new(),
            counts: BTreeMap::new(),
        }
    }

    pub fn register(
        &mut self,
        provider: OwnerId,
        descriptor: DefinitionMigrationDescriptor,
    ) -> Result<(), MigrationError> {
        if descriptor.definition.key.owner != provider
            || descriptor.from.key.owner != provider
            || descriptor.to.key.owner != provider
        {
            return Err(MigrationError::OwnerMismatch { provider });
        }
        if descriptor.definition.key.kind.as_str() != "migration" {
            return Err(MigrationError::WrongDefinitionKind {
                definition: descriptor.definition,
            });
        }
        if descriptor.from.key != descriptor.to.key {
            return Err(MigrationError::DifferentKeys {
                from: descriptor.from,
                to: descriptor.to,
            });
        }
        if descriptor.from.version.checked_add(1) != Some(descriptor.to.version) {
            return Err(MigrationError::NonContiguous {
                from: descriptor.from,
                to: descriptor.to,
            });
        }
        let count = self.counts.get(&descriptor.from.key).copied().unwrap_or(0);
        if count >= self.limits.migrations_per_definition {
            return Err(MigrationError::Limit {
                key: descriptor.from.key,
                max: self.limits.migrations_per_definition,
            });
        }
        let edge_key = MigrationEdgeKey {
            key: descriptor.from.key.clone(),
            from_version: descriptor.from.version,
        };
        if let Some(existing) = self.entries.get(&edge_key) {
            return Err(MigrationError::Duplicate {
                from: descriptor.from,
                registered: existing.definition.clone(),
                attempted: descriptor.definition,
            });
        }
        self.counts.insert(descriptor.from.key.clone(), count + 1);
        self.entries.insert(
            edge_key,
            DefinitionMigrationEntry {
                provider,
                definition: descriptor.definition,
                from: descriptor.from,
                to: descriptor.to,
                transform: descriptor.transform,
            },
        );
        Ok(())
    }

    pub fn freeze(
        self,
        definitions: &DefinitionRegistry,
    ) -> Result<DefinitionMigrationRegistry, MigrationError> {
        self.freeze_with_readiness(definitions, None)
    }

    pub(crate) fn freeze_for_composition(
        self,
        definitions: &DefinitionRegistry,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<DefinitionMigrationRegistry, MigrationError> {
        self.freeze_with_readiness(definitions, Some(owner_readiness))
    }

    fn freeze_with_readiness(
        self,
        definitions: &DefinitionRegistry,
        owner_readiness: Option<MechanicalOwnerReadiness>,
    ) -> Result<DefinitionMigrationRegistry, MigrationError> {
        if let Some(readiness) = &owner_readiness {
            let definition_readiness = definitions
                .owner_readiness()
                .ok_or(MigrationError::RegistryCompositionMismatch)?;
            if !readiness.shares_latch_with(definition_readiness) {
                return Err(MigrationError::RegistryCompositionMismatch);
            }
        }
        for entry in self.entries.values() {
            for reference in [&entry.definition, &entry.from, &entry.to] {
                let result = if owner_readiness.is_some() {
                    definitions.get_exact(reference)
                } else {
                    definitions.inspect_exact(reference)
                };
                result.map_err(|source| MigrationError::MissingDefinition {
                    migration: entry.definition.clone(),
                    source,
                })?;
            }
        }
        let fingerprint = migration_registry_fingerprint(&self.entries);
        Ok(DefinitionMigrationRegistry {
            entries: self.entries,
            limits: self.limits,
            fingerprint,
            owner_readiness,
        })
    }
}

fn migration_registry_fingerprint(
    entries: &BTreeMap<MigrationEdgeKey, DefinitionMigrationEntry>,
) -> DefinitionDigest {
    let mut encoded = Vec::with_capacity(entries.len().saturating_mul(384));
    for entry in entries.values() {
        let provider = entry.provider.as_str().as_bytes();
        encoded.extend_from_slice(
            &u64::try_from(provider.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        encoded.extend_from_slice(provider);
        append_reference_bytes(&mut encoded, &entry.definition);
        append_reference_bytes(&mut encoded, &entry.from);
        append_reference_bytes(&mut encoded, &entry.to);
    }
    hash_parts(b"migration_registry.v1", [&encoded[..]])
}

#[derive(Clone, Debug)]
pub struct DefinitionMigrationRegistry {
    entries: BTreeMap<MigrationEdgeKey, DefinitionMigrationEntry>,
    limits: MechanicsLimits,
    fingerprint: DefinitionDigest,
    owner_readiness: Option<MechanicalOwnerReadiness>,
}

#[allow(clippy::result_large_err)]
impl DefinitionMigrationRegistry {
    /// Execute a bounded plan entirely in memory.  It neither exposes nor
    /// accepts a persistence handle; durable application is a later gate.
    pub fn dry_run(
        &self,
        source: &RulesValue,
        target: &DefinitionRef,
        definitions: &DefinitionRegistry,
    ) -> Result<MigrationPlan, MigrationError> {
        let readiness = self
            .owner_readiness
            .as_ref()
            .ok_or(MigrationError::RegistryUnavailable)?;
        let definition_readiness = definitions
            .owner_readiness()
            .ok_or(MigrationError::RegistryCompositionMismatch)?;
        if !readiness.shares_latch_with(definition_readiness) {
            return Err(MigrationError::RegistryCompositionMismatch);
        }
        definitions
            .get_exact(source.schema())
            .map_err(|source_error| MigrationError::InvalidEndpoint {
                endpoint: source.schema().clone(),
                source: source_error,
            })?;
        definitions
            .get_exact(target)
            .map_err(|source_error| MigrationError::InvalidEndpoint {
                endpoint: target.clone(),
                source: source_error,
            })?;
        if source.schema().key != target.key {
            return Err(MigrationError::DifferentKeys {
                from: source.schema().clone(),
                to: target.clone(),
            });
        }
        if source.schema().version > target.version {
            return Err(MigrationError::Downgrade {
                from: source.schema().clone(),
                to: target.clone(),
            });
        }
        if source.schema().version == target.version && source.schema() != target {
            return Err(MigrationError::TargetDigestMismatch {
                from: source.schema().clone(),
                to: target.clone(),
            });
        }

        let before_digest =
            DefinitionDigest::for_canonical_value("rules_value.data.v1", source.data())?;
        let mut current = source.schema().clone();
        let mut data = source.data().clone();
        let mut steps = Vec::new();
        while current.version < target.version {
            let edge_key = MigrationEdgeKey {
                key: current.key.clone(),
                from_version: current.version,
            };
            let edge = self
                .entries
                .get(&edge_key)
                .ok_or_else(|| MigrationError::MissingEdge {
                    from: current.clone(),
                    target: target.clone(),
                })?;
            if edge.from != current {
                return Err(MigrationError::EdgeDigestMismatch {
                    current,
                    registered: edge.from.clone(),
                });
            }
            readiness.require_ready(&edge.provider).map_err(|source| {
                MigrationError::OwnerNotReady {
                    owner: edge.provider.clone(),
                    source: Box::new(source),
                }
            })?;
            let first = (edge.transform)(&data).map_err(|message| MigrationError::Transform {
                migration: edge.definition.clone(),
                message: bounded_message(message),
            })?;
            readiness.require_ready(&edge.provider).map_err(|source| {
                MigrationError::OwnerNotReady {
                    owner: edge.provider.clone(),
                    source: Box::new(source),
                }
            })?;
            let repeated =
                (edge.transform)(&data).map_err(|message| MigrationError::Transform {
                    migration: edge.definition.clone(),
                    message: bounded_message(message),
                })?;
            readiness.require_ready(&edge.provider).map_err(|source| {
                MigrationError::OwnerNotReady {
                    owner: edge.provider.clone(),
                    source: Box::new(source),
                }
            })?;
            let first_bytes =
                canonical_json_bytes_with_limit(&first, self.limits.rules_value_bytes).map_err(
                    |_| MigrationError::OutputTooLarge {
                        migration: edge.definition.clone(),
                        max: self.limits.rules_value_bytes,
                    },
                )?;
            let repeated_bytes =
                canonical_json_bytes_with_limit(&repeated, self.limits.rules_value_bytes).map_err(
                    |_| MigrationError::OutputTooLarge {
                        migration: edge.definition.clone(),
                        max: self.limits.rules_value_bytes,
                    },
                )?;
            if first_bytes != repeated_bytes {
                return Err(MigrationError::NonDeterministic {
                    migration: edge.definition.clone(),
                });
            }
            data = first;
            steps.push(MigrationStep {
                migration: edge.definition.clone(),
                from: edge.from.clone(),
                to: edge.to.clone(),
            });
            current = edge.to.clone();
        }
        if current != *target {
            return Err(MigrationError::TargetDigestMismatch {
                from: current,
                to: target.clone(),
            });
        }
        let after_digest = DefinitionDigest::for_canonical_value("rules_value.data.v1", &data)?;
        let output =
            RulesValue::new_with_limit(target.clone(), data, self.limits.rules_value_bytes)
                .map_err(|_| MigrationError::OutputTooLarge {
                    migration: steps
                        .last()
                        .map_or_else(|| target.clone(), |step| step.migration.clone()),
                    max: self.limits.rules_value_bytes,
                })?;
        Ok(MigrationPlan {
            from: source.schema().clone(),
            to: target.clone(),
            before_digest,
            after_digest,
            steps,
            output,
        })
    }

    pub fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = (&DefinitionRef, &DefinitionRef, &DefinitionRef)> {
        self.entries
            .values()
            .map(|entry| (&entry.definition, &entry.from, &entry.to))
    }

    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }
}

fn bounded_message(mut message: String) -> String {
    const MAX_BYTES: usize = 512;
    if message.len() <= MAX_BYTES {
        return message;
    }
    let mut end = MAX_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationStep {
    pub migration: DefinitionRef,
    pub from: DefinitionRef,
    pub to: DefinitionRef,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MigrationPlan {
    from: DefinitionRef,
    to: DefinitionRef,
    before_digest: DefinitionDigest,
    after_digest: DefinitionDigest,
    steps: Vec<MigrationStep>,
    output: RulesValue,
}

impl MigrationPlan {
    #[must_use]
    pub const fn from(&self) -> &DefinitionRef {
        &self.from
    }

    #[must_use]
    pub const fn to(&self) -> &DefinitionRef {
        &self.to
    }

    #[must_use]
    pub const fn before_digest(&self) -> DefinitionDigest {
        self.before_digest
    }

    #[must_use]
    pub const fn after_digest(&self) -> DefinitionDigest {
        self.after_digest
    }

    #[must_use]
    pub fn steps(&self) -> &[MigrationStep] {
        &self.steps
    }

    #[must_use]
    pub const fn output(&self) -> &RulesValue {
        &self.output
    }

    #[must_use]
    pub fn into_output(self) -> RulesValue {
        self.output
    }

    #[must_use]
    pub fn summary(&self) -> MigrationPlanSummary {
        MigrationPlanSummary {
            from: self.from.clone(),
            to: self.to.clone(),
            step_count: self.steps.len().try_into().unwrap_or(u32::MAX),
            before_digest: self.before_digest,
            after_digest: self.after_digest,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationPlanSummary {
    pub from: DefinitionRef,
    pub to: DefinitionRef,
    pub step_count: u32,
    pub before_digest: DefinitionDigest,
    pub after_digest: DefinitionDigest,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MigrationError {
    #[error("migration execution is unavailable outside module composition")]
    RegistryUnavailable,
    #[error("migration registry and definitions do not belong to one module composition")]
    RegistryCompositionMismatch,
    #[error("migration owner {owner} is mechanically unavailable: {source}")]
    OwnerNotReady {
        owner: OwnerId,
        source: Box<MechanicalOwnerReadinessError>,
    },
    #[error("migration provider {provider} does not own all of its definition and endpoints")]
    OwnerMismatch { provider: OwnerId },
    #[error("migration definition {definition} must have definition kind `migration`")]
    WrongDefinitionKind { definition: DefinitionRef },
    #[error("migration endpoints use different keys: {from} -> {to}")]
    DifferentKeys {
        from: DefinitionRef,
        to: DefinitionRef,
    },
    #[error("migration edge is not contiguous: {from} -> {to}")]
    NonContiguous {
        from: DefinitionRef,
        to: DefinitionRef,
    },
    #[error("definition {key} exceeds its {max}-migration limit")]
    Limit { key: DefinitionKey, max: usize },
    #[error("migration from {from} is already registered as {registered}; attempted {attempted}")]
    Duplicate {
        from: DefinitionRef,
        registered: DefinitionRef,
        attempted: DefinitionRef,
    },
    #[error("migration {migration} refers to an unavailable definition: {source}")]
    MissingDefinition {
        migration: DefinitionRef,
        source: DefinitionLookupError,
    },
    #[error("migration endpoint {endpoint} is unavailable: {source}")]
    InvalidEndpoint {
        endpoint: DefinitionRef,
        source: DefinitionLookupError,
    },
    #[error("migration downgrade is not supported: {from} -> {to}")]
    Downgrade {
        from: DefinitionRef,
        to: DefinitionRef,
    },
    #[error("same-version migration endpoints disagree on digest: {from} -> {to}")]
    TargetDigestMismatch {
        from: DefinitionRef,
        to: DefinitionRef,
    },
    #[error("migration chain has no edge from {from} toward {target}")]
    MissingEdge {
        from: DefinitionRef,
        target: DefinitionRef,
    },
    #[error("migration chain expected {current}, but registered edge starts at {registered}")]
    EdgeDigestMismatch {
        current: DefinitionRef,
        registered: DefinitionRef,
    },
    #[error("migration {migration} failed: {message}")]
    Transform {
        migration: DefinitionRef,
        message: String,
    },
    #[error("migration {migration} produced different outputs for the same input")]
    NonDeterministic { migration: DefinitionRef },
    #[error("migration {migration} produced more than {max} canonical bytes")]
    OutputTooLarge {
        migration: DefinitionRef,
        max: usize,
    },
    #[error(transparent)]
    CanonicalEncoding(#[from] CanonicalEncodingError),
}

/// Stable incident identity allocated by the recovery ledger.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct IncidentId(pub u64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanicalState {
    Active,
    Frozen {
        incident: IncidentId,
    },
    Migrating {
        from: Box<DefinitionRef>,
        to: Box<DefinitionRef>,
    },
    Quarantined {
        incident: IncidentId,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryScope {
    Record,
    Object,
    Module,
    World,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoverySeverity {
    Warning,
    Error,
    Critical,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentState {
    Open,
    Acknowledged,
    Resolved,
}

/// Structured subject identity; recovery never relies on a display string.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySubject {
    pub owner: OwnerId,
    pub kind: CanonicalId,
    pub id: CanonicalId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryReason {
    DefinitionUnavailable {
        definition: DefinitionRef,
    },
    DefinitionDigestMismatch {
        requested: DefinitionRef,
        registered: DefinitionRef,
    },
    MigrationUnavailable {
        from: DefinitionRef,
        to: DefinitionRef,
    },
    MalformedRecord {
        code: CanonicalId,
    },
    OwnershipMismatch {
        expected: OwnerId,
        actual: OwnerId,
    },
    ProviderUnavailable {
        owner: OwnerId,
    },
    IntegrityViolation {
        code: CanonicalId,
    },
}

/// The first integrity incident that mechanically disabled one composed owner.
///
/// This is runtime readiness state, not a replacement for the owner's durable
/// recovery report.  The first incident wins so a later record cannot obscure
/// the reason an owner's executable providers were taken out of service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MechanicalOwnerFreeze {
    pub incident: IncidentId,
    pub reason: RecoveryReason,
}

#[derive(Debug, Default)]
struct MechanicalOwnerReadinessState {
    owners: BTreeMap<OwnerId, Option<MechanicalOwnerFreeze>>,
}

/// Shared, one-way readiness latch for mechanically executable module owners.
///
/// The roster is fixed at module composition.  Owners begin ready and may only
/// transition to frozen.  Registry clones (notably the exact action-provider
/// registry) share the same latch, so an owner integrity freeze cannot be
/// bypassed by a stale copy of composition metadata.
#[derive(Clone, Default)]
pub struct MechanicalOwnerReadiness {
    state: Arc<RwLock<MechanicalOwnerReadinessState>>,
}

impl fmt::Debug for MechanicalOwnerReadiness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f.debug_struct("MechanicalOwnerReadiness")
            .field("owners", &state.owners)
            .finish()
    }
}

impl MechanicalOwnerReadiness {
    pub(crate) fn for_owners(owners: &BTreeSet<OwnerId>) -> Self {
        debug_assert!(owners.len() <= MAX_MECHANICS_OWNERS);
        Self {
            state: Arc::new(RwLock::new(MechanicalOwnerReadinessState {
                owners: owners.iter().cloned().map(|owner| (owner, None)).collect(),
            })),
        }
    }

    /// Whether two registry components belong to the same process-local
    /// module composition. Clones compare equal because they share the one
    /// monotonic latch; independently composed registries never do.
    pub(crate) fn shares_latch_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    pub(crate) fn freeze(
        &self,
        owner: &OwnerId,
        incident: IncidentId,
        reason: RecoveryReason,
    ) -> Result<MechanicalOwnerFreeze, MechanicalOwnerReadinessError> {
        // No caller-controlled code runs while this guard is held. Recovering a
        // poisoned guard is therefore safe and preserves fail-closed one-way
        // state instead of accidentally making an owner ready again.
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(slot) = state.owners.get_mut(owner) else {
            return Err(MechanicalOwnerReadinessError::UnknownOwner {
                owner: owner.clone(),
            });
        };
        Ok(slot
            .get_or_insert_with(|| MechanicalOwnerFreeze { incident, reason })
            .clone())
    }

    pub(crate) fn freeze_for(
        &self,
        owner: &OwnerId,
    ) -> Result<Option<MechanicalOwnerFreeze>, MechanicalOwnerReadinessError> {
        let state = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.owners.get(owner).cloned().ok_or_else(|| {
            MechanicalOwnerReadinessError::UnknownOwner {
                owner: owner.clone(),
            }
        })
    }

    pub(crate) fn require_ready(
        &self,
        owner: &OwnerId,
    ) -> Result<(), MechanicalOwnerReadinessError> {
        match self.freeze_for(owner)? {
            None => Ok(()),
            Some(freeze) => Err(MechanicalOwnerReadinessError::Frozen {
                owner: owner.clone(),
                incident: freeze.incident,
                reason: Box::new(freeze.reason),
            }),
        }
    }
}

/// Mechanical owner readiness could not authorize executable work.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MechanicalOwnerReadinessError {
    #[error("mechanics owner readiness is unavailable before module composition")]
    RegistryUnavailable,
    #[error("mechanics owner {owner} is not part of the composed module roster")]
    UnknownOwner { owner: OwnerId },
    #[error("mechanics owner {owner} is frozen by incident {incident:?}: {reason:?}")]
    Frozen {
        owner: OwnerId,
        incident: IncidentId,
        reason: Box<RecoveryReason>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RecoveryIncidentWire", deny_unknown_fields)]
pub struct RecoveryIncident {
    pub id: IncidentId,
    pub owner: OwnerId,
    pub scope: RecoveryScope,
    pub severity: RecoverySeverity,
    pub reason: RecoveryReason,
    pub subjects: Vec<RecoverySubject>,
    pub payload_digest: Option<DefinitionDigest>,
    /// Unix seconds are a temporary K1 discovery stamp.  K4 can replace the
    /// source with `ClockStamp` without weakening exact mechanics identity.
    pub discovered_unix: i64,
    pub state: IncidentState,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryIncidentWire {
    id: IncidentId,
    owner: OwnerId,
    scope: RecoveryScope,
    severity: RecoverySeverity,
    reason: RecoveryReason,
    subjects: Vec<RecoverySubject>,
    payload_digest: Option<DefinitionDigest>,
    discovered_unix: i64,
    state: IncidentState,
}

impl TryFrom<RecoveryIncidentWire> for RecoveryIncident {
    type Error = RecoveryIncidentError;

    fn try_from(value: RecoveryIncidentWire) -> Result<Self, Self::Error> {
        Self::new(
            value.id,
            value.owner,
            value.scope,
            value.severity,
            value.reason,
            value.subjects,
            value.payload_digest,
            value.discovered_unix,
            value.state,
        )
    }
}

impl RecoveryIncident {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: IncidentId,
        owner: OwnerId,
        scope: RecoveryScope,
        severity: RecoverySeverity,
        reason: RecoveryReason,
        subjects: Vec<RecoverySubject>,
        payload_digest: Option<DefinitionDigest>,
        discovered_unix: i64,
        state: IncidentState,
    ) -> Result<Self, RecoveryIncidentError> {
        if subjects.len() > MAX_RECOVERY_SUBJECTS {
            return Err(RecoveryIncidentError::TooManySubjects {
                count: subjects.len(),
                max: MAX_RECOVERY_SUBJECTS,
            });
        }
        Ok(Self {
            id,
            owner,
            scope,
            severity,
            reason,
            subjects,
            payload_digest,
            discovered_unix,
            state,
        })
    }

    #[must_use]
    pub fn summary(&self) -> RecoveryIncidentSummary {
        RecoveryIncidentSummary {
            id: self.id,
            owner: self.owner.clone(),
            scope: self.scope,
            severity: self.severity,
            reason: self.reason.clone(),
            subject_count: self.subjects.len().try_into().unwrap_or(u32::MAX),
            payload_digest: self.payload_digest,
            discovered_unix: self.discovered_unix,
            state: self.state,
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RecoveryIncidentError {
    #[error("recovery incident has {count} subjects; maximum is {max}")]
    TooManySubjects { count: usize, max: usize },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryIncidentSummary {
    pub id: IncidentId,
    pub owner: OwnerId,
    pub scope: RecoveryScope,
    pub severity: RecoverySeverity,
    pub reason: RecoveryReason,
    pub subject_count: u32,
    pub payload_digest: Option<DefinitionDigest>,
    pub discovered_unix: i64,
    pub state: IncidentState,
}

/// Bounded operator-facing module composition/recovery report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModuleBootReport {
    pub owner: OwnerId,
    pub active: u64,
    pub frozen: u64,
    pub migrating: u64,
    pub quarantined: u64,
    pub incident_count: u64,
    pub migration_plan_count: u64,
    pub incidents: Vec<RecoveryIncidentSummary>,
    pub migration_plans: Vec<MigrationPlanSummary>,
    pub details_truncated: bool,
}

impl ModuleBootReport {
    #[must_use]
    pub fn new(owner: OwnerId) -> Self {
        Self {
            owner,
            active: 0,
            frozen: 0,
            migrating: 0,
            quarantined: 0,
            incident_count: 0,
            migration_plan_count: 0,
            incidents: Vec::new(),
            migration_plans: Vec::new(),
            details_truncated: false,
        }
    }

    pub fn record_state(&mut self, state: &MechanicalState) {
        let counter = match state {
            MechanicalState::Active => &mut self.active,
            MechanicalState::Frozen { .. } => &mut self.frozen,
            MechanicalState::Migrating { .. } => &mut self.migrating,
            MechanicalState::Quarantined { .. } => &mut self.quarantined,
        };
        *counter = counter.saturating_add(1);
    }

    pub fn push_incident(&mut self, incident: RecoveryIncidentSummary) {
        self.incident_count = self.incident_count.saturating_add(1);
        if self.incidents.len() < MAX_BOOT_REPORT_DETAILS {
            self.incidents.push(incident);
        } else {
            self.details_truncated = true;
        }
    }

    pub fn push_migration_plan(&mut self, plan: MigrationPlanSummary) {
        self.migration_plan_count = self.migration_plan_count.saturating_add(1);
        if self.migration_plans.len() < MAX_BOOT_REPORT_DETAILS {
            self.migration_plans.push(plan);
        } else {
            self.details_truncated = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn owner(value: &str) -> OwnerId {
        OwnerId::new(value).unwrap()
    }

    fn kind(value: &str) -> DefinitionKind {
        DefinitionKind::new(value).unwrap()
    }

    fn id(value: &str) -> DefinitionId {
        DefinitionId::new(value).unwrap()
    }

    fn descriptor(
        kind_name: &str,
        id_name: &str,
        version: u32,
        payload: Value,
    ) -> DefinitionDescriptor {
        DefinitionDescriptor::new(kind(kind_name), id(id_name), version, payload).unwrap()
    }

    fn bind_definitions_for(
        mut definitions: DefinitionRegistry,
        roster: &BTreeSet<OwnerId>,
    ) -> (DefinitionRegistry, MechanicalOwnerReadiness) {
        let readiness = MechanicalOwnerReadiness::for_owners(roster);
        definitions.bind_owner_readiness(readiness.clone());
        (definitions, readiness)
    }

    #[test]
    fn constructors_canonicalize_but_deserialization_is_strict() {
        assert_eq!(OwnerId::new("  WoD.V5 ").unwrap().as_str(), "wod.v5");
        assert_eq!(
            serde_json::from_str::<OwnerId>(r#""wod.v5""#).unwrap(),
            owner("wod.v5")
        );
        for invalid in [r#"" WOD.v5 ""#, r#""wod..v5""#, r#""wod-v5""#] {
            assert!(serde_json::from_str::<OwnerId>(invalid).is_err());
        }
        assert!(OwnerId::new("x".repeat(CANONICAL_ID_MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn digest_wire_form_is_exact_lowercase_hex() {
        let digest = DefinitionDigest::from_bytes([0xab; 32]);
        let encoded = serde_json::to_string(&digest).unwrap();
        assert_eq!(encoded, format!("\"{}\"", "ab".repeat(32)));
        assert_eq!(
            serde_json::from_str::<DefinitionDigest>(&encoded).unwrap(),
            digest
        );
        assert!(DefinitionDigest::from_hex(&"AB".repeat(32)).is_err());
        assert!(DefinitionDigest::from_hex("ab").is_err());
    }

    #[test]
    fn canonical_hash_is_independent_of_object_insertion_order() {
        let left = json!({"z": [3, 2, 1], "a": {"b": true, "a": null}});
        let right: Value =
            serde_json::from_str(r#"{"a":{"a":null,"b":true},"z":[3,2,1]}"#).unwrap();
        assert_eq!(
            canonical_json_bytes(&left).unwrap(),
            canonical_json_bytes(&right).unwrap()
        );
        assert_eq!(
            DefinitionDigest::for_canonical_value("test.v1", &left).unwrap(),
            DefinitionDigest::for_canonical_value("test.v1", &right).unwrap()
        );
        assert_ne!(
            DefinitionDigest::for_canonical_value("test.v1", &left).unwrap(),
            DefinitionDigest::for_canonical_value("other.v1", &left).unwrap()
        );
    }

    #[test]
    fn canonical_encoding_and_digest_have_a_golden_wire_contract() {
        let value = json!({"z": [true, null, "x"], "a": 1});
        assert_eq!(
            canonical_json_bytes(&value).unwrap(),
            br#"{"a":1,"z":[true,null,"x"]}"#
        );
        assert_eq!(
            DefinitionDigest::for_canonical_value("golden.definition.v1", &value)
                .unwrap()
                .to_hex(),
            "3f0c32352c690c662515a6edb71a7655c4e00b2ef506a59d218e779386d52c0f"
        );

        // JSON integer and floating-point representations are deliberately
        // distinct definition bytes. Canonicalization orders containers; it
        // does not silently coerce provider-authored numeric types.
        assert_ne!(
            canonical_json_bytes(&json!(1)).unwrap(),
            canonical_json_bytes(&json!(1.0)).unwrap()
        );
        assert_ne!(
            canonical_json_bytes(&json!(0)).unwrap(),
            canonical_json_bytes(&json!(-0.0)).unwrap()
        );
    }

    #[test]
    fn definition_identity_and_registry_fingerprint_have_golden_contracts() {
        let mut builder = DefinitionRegistryBuilder::new();
        let reference = builder
            .register(
                owner("golden"),
                descriptor(
                    "schema",
                    "widget",
                    7,
                    json!({"z": [true, null, "x"], "a": 1}),
                )
                .as_current(),
            )
            .unwrap();
        assert_eq!(
            reference.to_string(),
            "golden:schema:widget@7#d09ba2e5a090c0f03647071581d89effd456087bc92ee81f83368267246a3313"
        );

        let registry = builder.freeze().unwrap();
        assert_eq!(
            registry.fingerprint().to_hex(),
            "b99b35f2e8b464e7ba90044a656e36eae92291c5267a5f92f91d2c7ecf90d491"
        );
    }

    #[test]
    fn rules_value_and_exact_ref_reject_unknown_or_invalid_wire_data() {
        let reference = DefinitionRef::new(
            DefinitionKey::new(owner("core"), kind("schema"), id("unit")),
            1,
            DefinitionDigest::from_bytes([1; 32]),
        )
        .unwrap();
        let value = RulesValue::new(reference.clone(), json!({"ok": true})).unwrap();
        let mut encoded = serde_json::to_value(value).unwrap();
        encoded
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(1));
        assert!(serde_json::from_value::<RulesValue>(encoded).is_err());

        let mut encoded_ref = serde_json::to_value(reference).unwrap();
        encoded_ref["version"] = json!(0);
        assert!(serde_json::from_value::<DefinitionRef>(encoded_ref).is_err());
        assert!(
            RulesValue::new_with_limit(
                DefinitionRef::new(
                    DefinitionKey::new(owner("core"), kind("schema"), id("small")),
                    1,
                    DefinitionDigest::from_bytes([2; 32]),
                )
                .unwrap(),
                json!("too long"),
                2,
            )
            .is_err()
        );
    }

    #[test]
    fn definitions_are_exact_bounded_and_reference_checked() {
        let mut builder = DefinitionRegistryBuilder::new();
        let dependency = builder
            .register(
                owner("core"),
                descriptor("schema", "base", 1, json!({"x": 1})),
            )
            .unwrap();
        let dependent_descriptor = descriptor("schema", "child", 1, json!({"x": 2}))
            .with_references(vec![dependency.clone()]);
        let dependent = builder
            .register(owner("core"), dependent_descriptor.clone())
            .unwrap();
        assert!(matches!(
            builder.register(owner("core"), dependent_descriptor),
            Err(DefinitionRegistryError::Duplicate { .. })
        ));
        assert!(matches!(
            builder.register(
                owner("core"),
                descriptor("schema", "child", 1, json!({"x": 3}))
            ),
            Err(DefinitionRegistryError::DigestConflict { .. })
        ));
        let registry = builder.freeze().unwrap();
        assert_eq!(
            registry.inspect_exact(&dependent).unwrap().references(),
            &[dependency]
        );
        assert!(matches!(
            registry.get_exact(&dependent),
            Err(DefinitionLookupError::OwnerNotReady { source, .. })
                if *source == MechanicalOwnerReadinessError::RegistryUnavailable
        ));

        let mut broken = DefinitionRegistryBuilder::new();
        let missing = DefinitionRef::new(
            DefinitionKey::new(owner("missing"), kind("schema"), id("gone")),
            1,
            DefinitionDigest::from_bytes([9; 32]),
        )
        .unwrap();
        broken
            .register(
                owner("core"),
                descriptor("schema", "broken", 1, json!({})).with_references(vec![missing]),
            )
            .unwrap();
        assert!(matches!(
            broken.freeze(),
            Err(DefinitionRegistryError::MissingReference { .. })
        ));
    }

    #[test]
    fn provider_definitions_require_one_semantic_validator_and_run_it_at_freeze() {
        let provider = owner("provider");
        let schema = kind("schema");

        let mut missing = DefinitionRegistryBuilder::new();
        missing
            .register_provider_definition(
                provider.clone(),
                descriptor("schema", "unit", 1, json!({"shape": "bad"})),
            )
            .unwrap();
        assert_eq!(
            missing.freeze().unwrap_err(),
            DefinitionRegistryError::MissingValidator {
                owner: provider.clone(),
                kind: schema.clone(),
            }
        );

        let mut invalid = DefinitionRegistryBuilder::new();
        invalid
            .register_provider_validator(
                provider.clone(),
                schema.clone(),
                Arc::new(|context: &DefinitionValidationContext<'_>| {
                    if context.definition().payload() == &json!({"shape": "valid"}) {
                        Ok(())
                    } else {
                        Err("typed shape is invalid".into())
                    }
                }),
            )
            .unwrap();
        assert_eq!(
            invalid.register_provider_validator(
                provider.clone(),
                schema.clone(),
                Arc::new(|_: &DefinitionValidationContext<'_>| Ok(())),
            ),
            Err(DefinitionRegistryError::DuplicateValidator {
                owner: provider.clone(),
                kind: schema.clone(),
            })
        );
        let rejected = invalid
            .register_provider_definition(
                provider.clone(),
                descriptor("schema", "unit", 1, json!({"shape": "bad"})),
            )
            .unwrap();
        assert_eq!(
            invalid.freeze().unwrap_err(),
            DefinitionRegistryError::SemanticValidation {
                definition: rejected,
                message: "typed shape is invalid".into(),
            }
        );

        let mut valid = DefinitionRegistryBuilder::new();
        valid
            .register_provider_validator(
                provider.clone(),
                schema,
                Arc::new(|context: &DefinitionValidationContext<'_>| {
                    (context.definition().payload() == &json!({"shape": "valid"}))
                        .then_some(())
                        .ok_or_else(|| "typed shape is invalid".into())
                }),
            )
            .unwrap();
        valid
            .register_provider_definition(
                provider,
                descriptor("schema", "unit", 1, json!({"shape": "valid"})),
            )
            .unwrap();
        assert!(valid.freeze().unwrap().is_fully_provider_validated());

        // Identity-building and structural fixture users intentionally retain
        // the low-level path; it does not make a semantic-validation claim.
        let mut structural = DefinitionRegistryBuilder::new();
        structural
            .register(
                owner("fixture"),
                descriptor("schema", "opaque", 1, json!({"anything": true})),
            )
            .unwrap();
        assert!(!structural.freeze().unwrap().is_fully_provider_validated());
    }

    #[test]
    fn definition_fingerprint_is_registration_order_independent() {
        fn registry(reverse: bool) -> DefinitionRegistry {
            let mut builder = DefinitionRegistryBuilder::new();
            let descriptors = [
                descriptor("schema", "a", 1, json!({"a": 1})),
                descriptor("schema", "b", 1, json!({"b": 2})),
            ];
            if reverse {
                for descriptor in descriptors.into_iter().rev() {
                    builder.register(owner("core"), descriptor).unwrap();
                }
            } else {
                for descriptor in descriptors {
                    builder.register(owner("core"), descriptor).unwrap();
                }
            }
            builder.freeze().unwrap()
        }
        let left = registry(false);
        let right = registry(true);
        assert_eq!(left.fingerprint(), right.fingerprint());
        let locator = DefinitionLocator::new(
            DefinitionKey::new(owner("core"), kind("schema"), id("a")),
            1,
        )
        .unwrap();
        assert_eq!(
            left.resolve_locator(&locator).unwrap(),
            right.resolve_locator(&locator).unwrap()
        );
    }

    #[test]
    fn owner_readiness_is_monotonic_and_keeps_structural_recovery_lookup() {
        let provider = owner("tactics");
        let roster = BTreeSet::from([provider.clone(), owner("unrelated")]);
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let mut definitions = DefinitionRegistryBuilder::new();
        let definition = definitions
            .register(
                provider.clone(),
                descriptor("schema", "unit", 1, json!({"shape": "known"})),
            )
            .unwrap();
        let mut definitions = definitions.freeze().unwrap();
        definitions.bind_owner_readiness(readiness.clone());
        assert!(definitions.get_exact(&definition).is_ok());

        let first_reason = RecoveryReason::IntegrityViolation {
            code: CanonicalId::new("first_conflict").unwrap(),
        };
        let first = readiness
            .freeze(&provider, IncidentId(41), first_reason.clone())
            .unwrap();
        assert_eq!(
            readiness
                .freeze(
                    &provider,
                    IncidentId(99),
                    RecoveryReason::IntegrityViolation {
                        code: CanonicalId::new("later_conflict").unwrap(),
                    },
                )
                .unwrap(),
            first
        );
        assert!(matches!(
            definitions.get_exact(&definition),
            Err(DefinitionLookupError::OwnerNotReady { source, .. })
                if matches!(
                    source.as_ref(),
                    MechanicalOwnerReadinessError::Frozen {
                        incident: IncidentId(41),
                        reason,
                        ..
                    } if reason.as_ref() == &first_reason
                )
        ));
        assert_eq!(
            definitions.inspect_exact(&definition).unwrap().reference(),
            &definition
        );
        assert_eq!(
            definitions.resolve_locator(&definition.locator()).unwrap(),
            definition
        );
        assert!(matches!(
            readiness.freeze(
                &owner("not_composed"),
                IncidentId(1),
                RecoveryReason::ProviderUnavailable {
                    owner: owner("not_composed")
                }
            ),
            Err(MechanicalOwnerReadinessError::UnknownOwner { .. })
        ));
        assert!(readiness.require_ready(&owner("unrelated")).is_ok());
    }

    #[test]
    fn current_definition_is_explicit_unique_and_fingerprinted() {
        let key = DefinitionKey::new(owner("core"), kind("schema"), id("unit"));

        let mut selected = DefinitionRegistryBuilder::new();
        let v1 = selected
            .register(
                owner("core"),
                descriptor("schema", "unit", 1, json!({"version": 1})),
            )
            .unwrap();
        let v2 = selected
            .register(
                owner("core"),
                descriptor("schema", "unit", 2, json!({"version": 2})).as_current(),
            )
            .unwrap();
        assert!(matches!(
            selected.register(
                owner("core"),
                descriptor("schema", "unit", 3, json!({"version": 3})).as_current(),
            ),
            Err(DefinitionRegistryError::DuplicateCurrent { .. })
        ));
        let selected = selected.freeze().unwrap();
        assert_eq!(selected.current(&key), Some(&v2));
        assert_ne!(selected.current(&key), Some(&v1));

        let mut unselected = DefinitionRegistryBuilder::new();
        unselected
            .register(
                owner("core"),
                descriptor("schema", "unit", 1, json!({"version": 1})),
            )
            .unwrap();
        unselected
            .register(
                owner("core"),
                descriptor("schema", "unit", 2, json!({"version": 2})),
            )
            .unwrap();
        let unselected = unselected.freeze().unwrap();
        assert!(unselected.current(&key).is_none());
        assert_ne!(selected.fingerprint(), unselected.fingerprint());
    }

    #[test]
    fn definition_registry_enforces_global_count_owner_and_byte_limits() {
        let count_limits = MechanicsLimits {
            definitions_total: 1,
            ..MechanicsLimits::default()
        };
        let mut count_builder = DefinitionRegistryBuilder::with_limits(count_limits);
        count_builder
            .register(owner("one"), descriptor("schema", "a", 1, json!({})))
            .unwrap();
        assert!(matches!(
            count_builder.register(owner("one"), descriptor("schema", "b", 1, json!({}))),
            Err(DefinitionRegistryError::DefinitionLimit { max: 1 })
        ));

        let owner_limits = MechanicsLimits {
            definition_owners: 1,
            ..MechanicsLimits::default()
        };
        let mut owner_builder = DefinitionRegistryBuilder::with_limits(owner_limits);
        owner_builder
            .register(owner("one"), descriptor("schema", "a", 1, json!({})))
            .unwrap();
        assert!(matches!(
            owner_builder.register(owner("two"), descriptor("schema", "b", 1, json!({}))),
            Err(DefinitionRegistryError::OwnerLimit { max: 1 })
        ));

        let byte_limits = MechanicsLimits {
            definition_bytes_total: 1,
            ..MechanicsLimits::default()
        };
        let mut byte_builder = DefinitionRegistryBuilder::with_limits(byte_limits);
        assert!(matches!(
            byte_builder.register(owner("one"), descriptor("schema", "a", 1, json!({}))),
            Err(DefinitionRegistryError::TotalBytesExceeded { max: 1 })
        ));
    }

    #[test]
    fn caller_limits_can_only_tighten_process_wide_ceilings() {
        let raised = MechanicsLimits {
            definition_owners: usize::MAX,
            definitions_total: usize::MAX,
            definition_bytes_total: usize::MAX,
            definitions_per_owner: usize::MAX,
            definition_bytes: usize::MAX,
            definition_bytes_per_owner: usize::MAX,
            references_per_definition: usize::MAX,
            bridges: usize::MAX,
            migrations_per_definition: usize::MAX,
            rules_value_bytes: usize::MAX,
        };
        assert_eq!(
            DefinitionRegistryBuilder::with_limits(raised).limits,
            MechanicsLimits::default()
        );
        assert_eq!(
            BridgeRegistryBuilder::with_limits(raised).limits,
            MechanicsLimits::default()
        );
        assert_eq!(
            DefinitionMigrationRegistryBuilder::with_limits(raised).limits,
            MechanicsLimits::default()
        );

        let reference = DefinitionRef::new(
            DefinitionKey::new(owner("core"), kind("schema"), id("bounded")),
            1,
            DefinitionDigest::from_bytes([7; 32]),
        )
        .unwrap();
        let too_large = "x".repeat(MAX_RULES_VALUE_BYTES + 1);
        assert!(matches!(
            RulesValue::new_with_limit(reference, json!(too_large), usize::MAX),
            Err(RulesValueError::TooLarge {
                max: MAX_RULES_VALUE_BYTES
            })
        ));
    }

    #[test]
    fn bridge_registry_is_directional_exact_and_fails_closed() {
        let mut definitions = DefinitionRegistryBuilder::new();
        let interaction = definitions
            .register(
                owner("core"),
                descriptor("interaction", "ram", 1, json!({"mode": "impact"})),
            )
            .unwrap();
        let bridge_definition = definitions
            .register(
                owner("core"),
                descriptor("bridge", "tactics_to_vehicles_ram", 1, json!({})),
            )
            .unwrap();
        let mut definitions = definitions.freeze().unwrap();
        let roster = BTreeSet::from([owner("core"), owner("tactics"), owner("vehicles")]);
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        definitions.bind_owner_readiness(readiness.clone());
        let mut bridges = BridgeRegistryBuilder::new();
        let descriptor = BridgeDescriptor::new(
            bridge_definition.clone(),
            owner("tactics"),
            owner("vehicles"),
            interaction.clone(),
        );
        bridges.register(owner("core"), descriptor.clone()).unwrap();
        assert!(matches!(
            bridges.register(owner("core"), descriptor),
            Err(BridgeRegistryError::Duplicate { .. })
        ));
        let registry = bridges
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness.clone())
            .unwrap();
        let tactics = owner("tactics");
        let vehicles = owner("vehicles");
        let key = BridgeKey {
            source_owner: tactics.clone(),
            target_owner: vehicles.clone(),
            interaction: interaction.clone(),
        };
        assert!(registry.get(&key).is_some());
        assert_eq!(
            registry
                .require_interaction(&definitions, &tactics, &vehicles, &interaction)
                .unwrap(),
            InteractionRoute::Bridge(bridge_definition)
        );
        assert_eq!(
            registry
                .require_interaction(&definitions, &tactics, &tactics, &interaction)
                .unwrap(),
            InteractionRoute::SameOwner
        );
        assert!(matches!(
            registry.require_interaction(&definitions, &vehicles, &tactics, &interaction),
            Err(BridgeLookupError::Missing { .. })
        ));
        let drifted = DefinitionRef::new(
            interaction.key().clone(),
            interaction.version(),
            DefinitionDigest::from_bytes([0x55; 32]),
        )
        .unwrap();
        assert!(matches!(
            registry.require_interaction(&definitions, &tactics, &tactics, &drifted),
            Err(BridgeLookupError::InvalidInteraction { .. })
        ));
        readiness
            .freeze(
                &vehicles,
                IncidentId(7),
                RecoveryReason::ProviderUnavailable {
                    owner: vehicles.clone(),
                },
            )
            .unwrap();
        assert!(
            registry.get(&key).is_some(),
            "frozen owners retain structural bridge metadata for diagnostics"
        );
        assert!(matches!(
            registry.require_interaction(&definitions, &tactics, &vehicles, &interaction),
            Err(BridgeLookupError::OwnerNotReady {
                role: "target",
                owner,
                source,
            }) if owner == vehicles
                && matches!(
                    *source,
                    MechanicalOwnerReadinessError::Frozen {
                        incident: IncidentId(7),
                        ..
                    }
                )
        ));
    }

    #[test]
    fn migration_registry_plans_contiguous_pure_transform_chain() {
        let mut definitions = DefinitionRegistryBuilder::new();
        let v1 = definitions
            .register(owner("tactics"), descriptor("state", "unit", 1, json!({})))
            .unwrap();
        let v2 = definitions
            .register(owner("tactics"), descriptor("state", "unit", 2, json!({})))
            .unwrap();
        let v3 = definitions
            .register(owner("tactics"), descriptor("state", "unit", 3, json!({})))
            .unwrap();
        let m12 = definitions
            .register(
                owner("tactics"),
                descriptor("migration", "unit_v1_v2", 1, json!({"from": 1, "to": 2})),
            )
            .unwrap();
        let m23 = definitions
            .register(
                owner("tactics"),
                descriptor("migration", "unit_v2_v3", 1, json!({"from": 2, "to": 3})),
            )
            .unwrap();
        let definitions = definitions.freeze().unwrap();
        let roster = BTreeSet::from([owner("tactics")]);
        let (definitions, readiness) = bind_definitions_for(definitions, &roster);

        let mut migrations = DefinitionMigrationRegistryBuilder::new();
        migrations
            .register(
                owner("tactics"),
                DefinitionMigrationDescriptor::new(m12, v1.clone(), v2.clone(), |value| {
                    let mut value = value.clone();
                    value["v"] = json!(2);
                    Ok(value)
                }),
            )
            .unwrap();
        migrations
            .register(
                owner("tactics"),
                DefinitionMigrationDescriptor::new(m23, v2, v3.clone(), |value| {
                    let mut value = value.clone();
                    value["v"] = json!(3);
                    Ok(value)
                }),
            )
            .unwrap();
        let migrations = migrations
            .freeze_for_composition(&definitions, readiness)
            .unwrap();
        let source = RulesValue::new(v1, json!({"v": 1})).unwrap();
        let source_before = source.clone();
        let plan = migrations.dry_run(&source, &v3, &definitions).unwrap();
        assert_eq!(source, source_before);
        assert_eq!(plan.steps().len(), 2);
        assert_eq!(plan.output().schema(), &v3);
        assert_eq!(plan.output().data(), &json!({"v": 3}));
        assert_ne!(plan.before_digest(), plan.after_digest());
    }

    #[test]
    fn migration_dry_run_rejects_a_nondeterministic_transform() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let mut definitions = DefinitionRegistryBuilder::new();
        let v1 = definitions
            .register(owner("tactics"), descriptor("state", "unit", 1, json!({})))
            .unwrap();
        let v2 = definitions
            .register(owner("tactics"), descriptor("state", "unit", 2, json!({})))
            .unwrap();
        let migration = definitions
            .register(
                owner("tactics"),
                descriptor(
                    "migration",
                    "unit_v1_v2_nondeterministic",
                    1,
                    json!({"implementation": "test-counter-v1"}),
                ),
            )
            .unwrap();
        let definitions = definitions.freeze().unwrap();
        let roster = BTreeSet::from([owner("tactics")]);
        let (definitions, readiness) = bind_definitions_for(definitions, &roster);
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let mut migrations = DefinitionMigrationRegistryBuilder::new();
        migrations
            .register(
                owner("tactics"),
                DefinitionMigrationDescriptor::new(
                    migration.clone(),
                    v1.clone(),
                    v2.clone(),
                    move |_| Ok(json!({"call": observed.fetch_add(1, Ordering::SeqCst)})),
                ),
            )
            .unwrap();
        let migrations = migrations
            .freeze_for_composition(&definitions, readiness)
            .unwrap();
        let source = RulesValue::new(v1, json!({})).unwrap();
        assert_eq!(
            migrations.dry_run(&source, &v2, &definitions),
            Err(MigrationError::NonDeterministic { migration })
        );
    }

    #[test]
    fn migration_registration_rejects_gaps_and_wrong_owner() {
        let key = DefinitionKey::new(owner("tactics"), kind("state"), id("unit"));
        let v1 = DefinitionRef::new(key.clone(), 1, DefinitionDigest::from_bytes([1; 32])).unwrap();
        let v2 = DefinitionRef::new(key.clone(), 2, DefinitionDigest::from_bytes([2; 32])).unwrap();
        let v3 = DefinitionRef::new(key, 3, DefinitionDigest::from_bytes([3; 32])).unwrap();
        let migration = DefinitionRef::new(
            DefinitionKey::new(owner("tactics"), kind("migration"), id("gap")),
            1,
            DefinitionDigest::from_bytes([4; 32]),
        )
        .unwrap();
        let mut builder = DefinitionMigrationRegistryBuilder::new();
        assert!(matches!(
            builder.register(
                owner("tactics"),
                DefinitionMigrationDescriptor::new(migration, v1.clone(), v3, |value| {
                    Ok(value.clone())
                }),
            ),
            Err(MigrationError::NonContiguous { .. })
        ));

        let foreign_migration = DefinitionRef::new(
            DefinitionKey::new(owner("foreign"), kind("migration"), id("unit_v1_v2")),
            1,
            DefinitionDigest::from_bytes([5; 32]),
        )
        .unwrap();
        assert_eq!(
            builder.register(
                owner("tactics"),
                DefinitionMigrationDescriptor::new(foreign_migration, v1, v2, |value| {
                    Ok(value.clone())
                }),
            ),
            Err(MigrationError::OwnerMismatch {
                provider: owner("tactics")
            })
        );
    }

    #[test]
    fn migration_dry_run_reports_a_missing_contiguous_edge() {
        let mut definitions = DefinitionRegistryBuilder::new();
        let v1 = definitions
            .register(owner("tactics"), descriptor("state", "unit", 1, json!({})))
            .unwrap();
        let v2 = definitions
            .register(owner("tactics"), descriptor("state", "unit", 2, json!({})))
            .unwrap();
        let definitions = definitions.freeze().unwrap();
        let roster = BTreeSet::from([owner("tactics")]);
        let (definitions, readiness) = bind_definitions_for(definitions, &roster);
        let migrations = DefinitionMigrationRegistryBuilder::new()
            .freeze_for_composition(&definitions, readiness)
            .unwrap();
        let source = RulesValue::new(v1.clone(), json!({})).unwrap();

        assert_eq!(
            migrations.dry_run(&source, &v2, &definitions),
            Err(MigrationError::MissingEdge {
                from: v1,
                target: v2,
            })
        );
    }

    #[test]
    fn recovery_incidents_and_boot_reports_are_bounded() {
        let incident = RecoveryIncident::new(
            IncidentId(7),
            owner("tactics"),
            RecoveryScope::Object,
            RecoverySeverity::Error,
            RecoveryReason::ProviderUnavailable {
                owner: owner("tactics"),
            },
            vec![],
            None,
            123,
            IncidentState::Open,
        )
        .unwrap();
        let mut report = ModuleBootReport::new(owner("tactics"));
        report.record_state(&MechanicalState::Frozen {
            incident: incident.id,
        });
        for _ in 0..=MAX_BOOT_REPORT_DETAILS {
            report.push_incident(incident.summary());
        }
        assert_eq!(report.frozen, 1);
        assert_eq!(report.incident_count, (MAX_BOOT_REPORT_DETAILS + 1) as u64);
        assert_eq!(report.incidents.len(), MAX_BOOT_REPORT_DETAILS);
        assert!(report.details_truncated);

        let too_many = vec![
            RecoverySubject {
                owner: owner("tactics"),
                kind: CanonicalId::new("unit").unwrap(),
                id: CanonicalId::new("one").unwrap(),
            };
            MAX_RECOVERY_SUBJECTS + 1
        ];
        assert!(
            RecoveryIncident::new(
                IncidentId(8),
                owner("tactics"),
                RecoveryScope::Object,
                RecoverySeverity::Error,
                RecoveryReason::ProviderUnavailable {
                    owner: owner("tactics")
                },
                too_many,
                None,
                123,
                IncidentState::Open,
            )
            .is_err()
        );
    }
}
