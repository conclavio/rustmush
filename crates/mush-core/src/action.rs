//! Rules-neutral action-kernel identities, bounded envelopes, and frozen providers.
//!
//! This is deliberately the pre-transaction half of K2. It defines stable,
//! canonical identities and validates metadata, but it cannot mutate a
//! [`World`](crate::World), install RAM images, submit persistence, or claim
//! authoritative completion. Commit-token storage and authoritative receipts
//! belong to K3.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;
use thiserror::Error;

use crate::action_state::{
    ActionStateError, ActionStateStore, ActionTxn, PreparedTxn, StateFamilyHandle,
};
use crate::authoritative::{
    AuthoritativeBundleError, AuthoritativeSemanticEvent, DurableOutboxIntent,
    MAX_DURABLE_OUTBOX_INTENTS,
};
use crate::behavior::ChannelId;
use crate::mechanics::{
    BridgeLookupError, BridgeRegistry, CanonicalEncodingError, CanonicalId, CanonicalIdError,
    DefinitionDigest, DefinitionLookupError, DefinitionRef, DefinitionRegistry, InteractionRoute,
    MechanicalOwnerReadiness, MechanicalOwnerReadinessError, OwnerId, RulesValue,
    canonical_json_bytes_with_limit,
};
use crate::obj::ObjId;
use crate::resolution::PreparedResolution;
use crate::world::World;

/// Maximum caller-supplied bytes in an idempotency key.
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 128;
/// Maximum bytes in canonical line-oriented ingress.
pub const MAX_INGRESS_LINE_BYTES: usize = 16 * 1024;
/// Maximum canonical bytes in one structured ingress payload.
pub const MAX_INGRESS_PAYLOAD_BYTES: usize = 64 * 1024;
/// Maximum canonical bytes in normalized action parameters.
pub const MAX_ACTION_PARAMETER_BYTES: usize = 64 * 1024;
/// Maximum perceived targets named by one invocation.
pub const MAX_ACTION_TARGETS: usize = 32;
/// Maximum independent revision stamps retained by one prepared action.
pub const MAX_ACTION_REVISIONS: usize = 256;
/// Maximum authority assertions retained by one prepared action.
pub const MAX_AUTHORITY_GUARDS: usize = 256;
/// Maximum canonical bytes in a complete server-owned guard set.
pub const MAX_GUARD_SET_BYTES: usize = 512 * 1024;
/// Maximum registered action providers in one deployment.
pub const MAX_ACTION_PROVIDERS: usize = 4_096;
/// Maximum registered action providers owned by one module.
pub const MAX_ACTION_PROVIDERS_PER_OWNER: usize = 1_024;
/// Maximum revision-validator registrations in one deployment.
pub const MAX_REVISION_VALIDATORS: usize = 4_096;
/// Maximum revision-validator registrations owned by one module.
pub const MAX_REVISION_VALIDATORS_PER_OWNER: usize = 1_024;
/// Maximum authority-validator registrations in one deployment.
pub const MAX_AUTHORITY_VALIDATORS: usize = 1_024;
/// Maximum authority-validator registrations owned by one module.
pub const MAX_AUTHORITY_VALIDATORS_PER_OWNER: usize = 64;
/// Maximum projected diagnostic bytes on an action failure.
pub const MAX_ACTION_ERROR_BYTES: usize = 512;
/// K2's hard bound on same-commit semantic events.
pub const MAX_SEMANTIC_EVENTS: u32 = 256;
/// Maximum recursive same-commit trigger depth.
pub const MAX_TRIGGER_DEPTH: u8 = 8;
/// Maximum trigger work items admitted over one action's whole cascade.
pub const MAX_CASCADE_TRIGGERS: u32 = 256;
/// Maximum prepared mutation operations over one action's whole cascade.
pub const MAX_CASCADE_OPERATIONS: u32 = 1_024;
/// Maximum canonical plan/event/trigger bytes over one action's whole cascade.
pub const MAX_CASCADE_BYTES: usize = 1_048_576;

const ACTION_DEFINITION_KIND: &str = "action";
const ACTION_HASH_PREAMBLE: &[u8] = b"rustmush.action.sha256.v1\0";

macro_rules! fixed_hex_id {
    ($name:ident, $bytes:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; $bytes]);

        impl $name {
            #[must_use]
            pub const fn from_bytes(bytes: [u8; $bytes]) -> Self {
                Self(bytes)
            }

            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; $bytes] {
                &self.0
            }

            pub fn from_hex(value: &str) -> Result<Self, FixedHexIdError> {
                parse_fixed_hex(value).map(Self)
            }

            #[must_use]
            pub fn to_hex(self) -> String {
                encode_fixed_hex(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name))
                    .field(&self.to_hex())
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.to_hex())
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.to_hex())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::from_hex(&value).map_err(D::Error::custom)
            }
        }
    };
}

fixed_hex_id!(
    RequestUid,
    16,
    "A gateway-assigned, replay-stable 128-bit request identity."
);
fixed_hex_id!(
    CommitToken,
    16,
    "A server-assigned, replay-stable 128-bit authoritative commit identity."
);
fixed_hex_id!(
    ContactId,
    16,
    "An opaque projected contact identity; never an object dbref."
);

fn parse_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], FixedHexIdError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    if value.len() != N * 2 {
        return Err(FixedHexIdError::Length {
            actual: value.len(),
            expected: N * 2,
        });
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(FixedHexIdError::NonCanonical);
    }
    let mut bytes = [0_u8; N];
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    for (index, pair) in pairs.iter().enumerate() {
        let high = HEX
            .iter()
            .position(|candidate| *candidate == pair[0])
            .ok_or(FixedHexIdError::InvalidHex { offset: index * 2 })?;
        let low = HEX
            .iter()
            .position(|candidate| *candidate == pair[1])
            .ok_or(FixedHexIdError::InvalidHex {
                offset: index * 2 + 1,
            })?;
        bytes[index] = ((high as u8) << 4) | low as u8;
    }
    Ok(bytes)
}

fn encode_fixed_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

/// Strict fixed-width hexadecimal decoding failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum FixedHexIdError {
    #[error("hex identity is {actual} bytes; expected exactly {expected}")]
    Length { actual: usize, expected: usize },
    #[error("hex identity must use canonical lowercase digits")]
    NonCanonical,
    #[error("invalid hexadecimal digit at byte {offset}")]
    InvalidHex { offset: usize },
}

/// A domain-separated SHA-256 identity, encoded as lowercase hexadecimal.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CanonicalHash([u8; 32]);

impl CanonicalHash {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_hex(value: &str) -> Result<Self, FixedHexIdError> {
        parse_fixed_hex(value).map(Self)
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        encode_fixed_hex(&self.0)
    }

    /// Hash canonical JSON under a named, versioned action-kernel domain.
    pub fn for_canonical_value(
        domain: &str,
        schema_version: u16,
        value: &Value,
        max_bytes: usize,
    ) -> Result<Self, CanonicalHashError> {
        let domain = CanonicalId::parse_canonical(domain)?;
        if schema_version == 0 {
            return Err(CanonicalHashError::ZeroSchemaVersion);
        }
        let bytes = canonical_json_bytes_with_limit(value, max_bytes)?;
        let mut hash = Sha256::new();
        hash.update(ACTION_HASH_PREAMBLE);
        hash.update((domain.as_str().len() as u64).to_be_bytes());
        hash.update(domain.as_str().as_bytes());
        hash.update(schema_version.to_be_bytes());
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
        Ok(Self(hash.finalize().into()))
    }
}

impl fmt::Debug for CanonicalHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CanonicalHash")
            .field(&self.to_hex())
            .finish()
    }
}

impl fmt::Display for CanonicalHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for CanonicalHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for CanonicalHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).map_err(D::Error::custom)
    }
}

/// Canonical action hash construction failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CanonicalHashError {
    #[error("invalid hash domain: {0}")]
    InvalidDomain(#[from] CanonicalIdError),
    #[error("hash schema version must be non-zero")]
    ZeroSchemaVersion,
    #[error("canonical hash input is invalid: {0}")]
    Canonical(#[from] CanonicalEncodingError),
}

/// A bounded, already-canonical external or internal idempotency component.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IdempotencyKeyText(String);

impl IdempotencyKeyText {
    pub fn parse_canonical(value: impl AsRef<str>) -> Result<Self, IdempotencyKeyTextError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(IdempotencyKeyTextError::Empty);
        }
        if value.len() > MAX_IDEMPOTENCY_KEY_BYTES {
            return Err(IdempotencyKeyTextError::TooLong {
                actual: value.len(),
                max: MAX_IDEMPOTENCY_KEY_BYTES,
            });
        }
        if let Some((offset, byte)) = value.bytes().enumerate().find(|(_, byte)| {
            !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b':' | b'-' | b'/')
        }) {
            return Err(IdempotencyKeyTextError::InvalidByte { offset, byte });
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for IdempotencyKeyText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("IdempotencyKeyText").field(&self.0).finish()
    }
}

impl fmt::Display for IdempotencyKeyText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for IdempotencyKeyText {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IdempotencyKeyText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse_canonical(value).map_err(D::Error::custom)
    }
}

/// Invalid bounded idempotency text.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum IdempotencyKeyTextError {
    #[error("idempotency key is empty")]
    Empty,
    #[error("idempotency key is {actual} bytes; maximum is {max}")]
    TooLong { actual: usize, max: usize },
    #[error("idempotency key contains invalid byte {byte:#04x} at byte {offset}")]
    InvalidByte { offset: usize, byte: u8 },
}

/// A rules-neutral authenticated principal.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrincipalId {
    Object {
        id: ObjId,
    },
    Scoped {
        owner: OwnerId,
        class: CanonicalId,
        id: IdempotencyKeyText,
    },
}

/// Scope in which a caller's key has exactly one semantic meaning.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IdempotencyScope {
    pub principal: PrincipalId,
    pub owner: OwnerId,
    pub namespace: CanonicalId,
}

/// The key class states who assigned the opaque component.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IdempotencyKey {
    Request(RequestUid),
    Caller(IdempotencyKeyText),
    Deterministic(CanonicalHash),
}

/// Complete idempotency identity, including a revocable scope epoch.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedIdempotencyKey {
    scope: IdempotencyScope,
    epoch: u64,
    key: IdempotencyKey,
}

impl ScopedIdempotencyKey {
    #[must_use]
    pub const fn new(scope: IdempotencyScope, epoch: u64, key: IdempotencyKey) -> Self {
        Self { scope, epoch, key }
    }

    #[must_use]
    pub const fn scope(&self) -> &IdempotencyScope {
        &self.scope
    }

    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    #[must_use]
    pub const fn key(&self) -> &IdempotencyKey {
        &self.key
    }
}

/// Versioned authenticated transport surface.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "IngressSurfaceVersionWire")]
pub struct IngressSurfaceVersion {
    id: CanonicalId,
    version: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IngressSurfaceVersionWire {
    id: CanonicalId,
    version: u16,
}

impl IngressSurfaceVersion {
    pub fn new(id: CanonicalId, version: u16) -> Result<Self, ActionMetadataError> {
        if version == 0 {
            return Err(ActionMetadataError::ZeroSchemaVersion);
        }
        Ok(Self { id, version })
    }

    #[must_use]
    pub const fn id(&self) -> &CanonicalId {
        &self.id
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
}

impl TryFrom<IngressSurfaceVersionWire> for IngressSurfaceVersion {
    type Error = ActionMetadataError;

    fn try_from(value: IngressSurfaceVersionWire) -> Result<Self, Self::Error> {
        Self::new(value.id, value.version)
    }
}

/// Canonical transport input, bounded before parsing or truth reads.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    try_from = "IngressPayloadWire",
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IngressPayload {
    Line(String),
    Structured(Value),
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum IngressPayloadWire {
    Line(String),
    Structured(Value),
}

impl IngressPayload {
    pub fn line(value: impl Into<String>) -> Result<Self, ActionMetadataError> {
        let value = value.into();
        if value.len() > MAX_INGRESS_LINE_BYTES {
            return Err(ActionMetadataError::IngressPayloadTooLarge {
                actual: value.len(),
                max: MAX_INGRESS_LINE_BYTES,
            });
        }
        Ok(Self::Line(value))
    }

    pub fn structured(value: Value) -> Result<Self, ActionMetadataError> {
        canonical_json_bytes_with_limit(&value, MAX_INGRESS_PAYLOAD_BYTES).map_err(|error| {
            ActionMetadataError::InvalidCanonicalPayload {
                message: error.to_string(),
            }
        })?;
        Ok(Self::Structured(value))
    }
}

impl TryFrom<IngressPayloadWire> for IngressPayload {
    type Error = ActionMetadataError;

    fn try_from(value: IngressPayloadWire) -> Result<Self, Self::Error> {
        match value {
            IngressPayloadWire::Line(line) => Self::line(line),
            IngressPayloadWire::Structured(value) => Self::structured(value),
        }
    }
}

/// Validated ingress plus its transport-specific canonical identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "IngressEnvelopeWire")]
pub struct IngressEnvelope {
    request: RequestUid,
    principal: PrincipalId,
    surface: IngressSurfaceVersion,
    canonical_payload: IngressPayload,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IngressEnvelopeWire {
    request: RequestUid,
    principal: PrincipalId,
    surface: IngressSurfaceVersion,
    canonical_payload: IngressPayload,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
}

impl IngressEnvelope {
    pub fn new(
        request: RequestUid,
        principal: PrincipalId,
        surface: IngressSurfaceVersion,
        canonical_payload: IngressPayload,
        idempotency: ScopedIdempotencyKey,
    ) -> Result<Self, ActionMetadataError> {
        validate_request_key(request, &idempotency)?;
        if idempotency.scope.principal != principal {
            return Err(ActionMetadataError::PrincipalScopeMismatch);
        }
        let ingress_hash = ingress_hash(&principal, &surface, &canonical_payload)?;
        Ok(Self {
            request,
            principal,
            surface,
            canonical_payload,
            idempotency,
            ingress_hash,
        })
    }

    #[must_use]
    pub const fn request(&self) -> RequestUid {
        self.request
    }

    #[must_use]
    pub const fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    #[must_use]
    pub const fn surface(&self) -> &IngressSurfaceVersion {
        &self.surface
    }

    #[must_use]
    pub const fn canonical_payload(&self) -> &IngressPayload {
        &self.canonical_payload
    }

    #[must_use]
    pub const fn idempotency(&self) -> &ScopedIdempotencyKey {
        &self.idempotency
    }

    #[must_use]
    pub const fn ingress_hash(&self) -> CanonicalHash {
        self.ingress_hash
    }
}

impl TryFrom<IngressEnvelopeWire> for IngressEnvelope {
    type Error = ActionMetadataError;

    fn try_from(value: IngressEnvelopeWire) -> Result<Self, Self::Error> {
        let envelope = Self::new(
            value.request,
            value.principal,
            value.surface,
            value.canonical_payload,
            value.idempotency,
        )?;
        if envelope.ingress_hash != value.ingress_hash {
            return Err(ActionMetadataError::HashMismatch {
                field: "ingress_hash",
            });
        }
        Ok(envelope)
    }
}

fn ingress_hash(
    principal: &PrincipalId,
    surface: &IngressSurfaceVersion,
    payload: &IngressPayload,
) -> Result<CanonicalHash, ActionMetadataError> {
    CanonicalHash::for_canonical_value(
        "ingress.v1",
        1,
        &json!({
            "principal": principal,
            "surface": surface,
            "canonical_payload": payload,
        }),
        MAX_INGRESS_PAYLOAD_BYTES + MAX_INGRESS_LINE_BYTES,
    )
    .map_err(ActionMetadataError::from)
}

/// Opaque, projected legal-action identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ActionOfferId(pub CanonicalHash);

/// A reference a viewer was actually permitted to receive.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PerceivedRef {
    SelfActor,
    KnownObject {
        id: ObjId,
        revision: u64,
    },
    Contact {
        provider: OwnerId,
        id: ContactId,
        generation: u64,
    },
}

/// Inputs used to construct and hash a semantic invocation.
#[derive(Clone, Debug)]
pub struct ActionInvocationDraft {
    pub request: RequestUid,
    pub principal: PrincipalId,
    pub actor: ObjId,
    pub offered_action: ActionOfferId,
    pub subject: PerceivedRef,
    pub targets: Vec<PerceivedRef>,
    pub parameters: Value,
    pub idempotency: ScopedIdempotencyKey,
}

/// Canonical semantic request after authentication, perception, and offer binding.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "ActionInvocationWire")]
pub struct ActionInvocation {
    schema_version: u16,
    request: RequestUid,
    principal: PrincipalId,
    actor: ObjId,
    offered_action: ActionOfferId,
    subject: PerceivedRef,
    targets: Vec<PerceivedRef>,
    parameters: Value,
    idempotency: ScopedIdempotencyKey,
    request_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionInvocationWire {
    schema_version: u16,
    request: RequestUid,
    principal: PrincipalId,
    actor: ObjId,
    offered_action: ActionOfferId,
    subject: PerceivedRef,
    targets: Vec<PerceivedRef>,
    parameters: Value,
    idempotency: ScopedIdempotencyKey,
    request_hash: CanonicalHash,
}

impl ActionInvocation {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn new(draft: ActionInvocationDraft) -> Result<Self, ActionMetadataError> {
        validate_request_key(draft.request, &draft.idempotency)?;
        if draft.idempotency.scope.principal != draft.principal {
            return Err(ActionMetadataError::PrincipalScopeMismatch);
        }
        if draft.targets.len() > MAX_ACTION_TARGETS {
            return Err(ActionMetadataError::TooManyTargets {
                actual: draft.targets.len(),
                max: MAX_ACTION_TARGETS,
            });
        }
        let unique: BTreeSet<_> = draft.targets.iter().collect();
        if unique.len() != draft.targets.len() {
            return Err(ActionMetadataError::DuplicateTarget);
        }
        canonical_json_bytes_with_limit(&draft.parameters, MAX_ACTION_PARAMETER_BYTES).map_err(
            |error| ActionMetadataError::InvalidCanonicalParameters {
                message: error.to_string(),
            },
        )?;
        let request_hash = semantic_request_hash(&draft)?;
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION,
            request: draft.request,
            principal: draft.principal,
            actor: draft.actor,
            offered_action: draft.offered_action,
            subject: draft.subject,
            targets: draft.targets,
            parameters: draft.parameters,
            idempotency: draft.idempotency,
            request_hash,
        })
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn request(&self) -> RequestUid {
        self.request
    }

    #[must_use]
    pub const fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }

    #[must_use]
    pub const fn offered_action(&self) -> ActionOfferId {
        self.offered_action
    }

    #[must_use]
    pub const fn subject(&self) -> &PerceivedRef {
        &self.subject
    }

    #[must_use]
    pub fn targets(&self) -> &[PerceivedRef] {
        &self.targets
    }

    #[must_use]
    pub const fn parameters(&self) -> &Value {
        &self.parameters
    }

    #[must_use]
    pub const fn idempotency(&self) -> &ScopedIdempotencyKey {
        &self.idempotency
    }

    #[must_use]
    pub const fn request_hash(&self) -> CanonicalHash {
        self.request_hash
    }
}

impl TryFrom<ActionInvocationWire> for ActionInvocation {
    type Error = ActionMetadataError;

    fn try_from(value: ActionInvocationWire) -> Result<Self, Self::Error> {
        if value.schema_version != Self::SCHEMA_VERSION {
            return Err(ActionMetadataError::UnsupportedInvocationSchema {
                actual: value.schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        let invocation = Self::new(ActionInvocationDraft {
            request: value.request,
            principal: value.principal,
            actor: value.actor,
            offered_action: value.offered_action,
            subject: value.subject,
            targets: value.targets,
            parameters: value.parameters,
            idempotency: value.idempotency,
        })?;
        if invocation.request_hash != value.request_hash {
            return Err(ActionMetadataError::HashMismatch {
                field: "request_hash",
            });
        }
        Ok(invocation)
    }
}

/// Server-private interpretation of one projected invocation.
///
/// The durable/wire [`ActionInvocation`] retains only references the caller was
/// allowed to see. This wrapper is constructed by the engine after resolving
/// the subject and every selected target through current perception/authority
/// providers. It is intentionally neither serializable nor constructible by a
/// client, module, or action provider.
#[derive(Clone)]
pub struct ResolvedActionInvocation {
    invocation: ActionInvocation,
    truth_subject: SubjectKey,
    truth_targets: Vec<SubjectKey>,
}

impl fmt::Debug for ResolvedActionInvocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedActionInvocation")
            .field("invocation", &self.invocation)
            .field("truth_subject", &"[REDACTED]")
            .field("truth_target_count", &self.truth_targets.len())
            .finish()
    }
}

impl ResolvedActionInvocation {
    pub(crate) fn new(
        invocation: ActionInvocation,
        truth_subject: SubjectKey,
        truth_targets: Vec<SubjectKey>,
    ) -> Option<Self> {
        if invocation.targets().len() != truth_targets.len() {
            return None;
        }
        Some(Self {
            invocation,
            truth_subject,
            truth_targets,
        })
    }

    /// The canonical projected invocation retained in durable action records.
    #[must_use]
    pub const fn invocation(&self) -> &ActionInvocation {
        &self.invocation
    }

    /// Private authoritative subject selected by the regenerated offer.
    #[must_use]
    pub const fn truth_subject(&self) -> &SubjectKey {
        &self.truth_subject
    }

    /// Private authoritative targets in the exact order of projected targets.
    #[must_use]
    pub fn truth_targets(&self) -> &[SubjectKey] {
        &self.truth_targets
    }
}

fn semantic_request_hash(
    draft: &ActionInvocationDraft,
) -> Result<CanonicalHash, ActionMetadataError> {
    CanonicalHash::for_canonical_value(
        "request.v1",
        ActionInvocation::SCHEMA_VERSION,
        &json!({
            "schema_version": ActionInvocation::SCHEMA_VERSION,
            "principal": draft.principal,
            "actor": draft.actor,
            "idempotency_scope": draft.idempotency.scope,
            "idempotency_epoch": draft.idempotency.epoch,
            "offered_action": draft.offered_action,
            "subject": draft.subject,
            "targets": draft.targets,
            "parameters": draft.parameters,
        }),
        MAX_ACTION_PARAMETER_BYTES + 16 * 1024,
    )
    .map_err(ActionMetadataError::from)
}

fn validate_request_key(
    request: RequestUid,
    idempotency: &ScopedIdempotencyKey,
) -> Result<(), ActionMetadataError> {
    if let IdempotencyKey::Request(key_request) = idempotency.key
        && key_request != request
    {
        return Err(ActionMetadataError::RequestKeyMismatch);
    }
    Ok(())
}

/// Invalid ingress or invocation metadata.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionMetadataError {
    #[error("schema version must be non-zero")]
    ZeroSchemaVersion,
    #[error("unsupported invocation schema {actual}; supported schema is {supported}")]
    UnsupportedInvocationSchema { actual: u16, supported: u16 },
    #[error("idempotency scope principal does not match authenticated principal")]
    PrincipalScopeMismatch,
    #[error("request-key idempotency identity does not match request uid")]
    RequestKeyMismatch,
    #[error("ingress payload is {actual} bytes; maximum is {max}")]
    IngressPayloadTooLarge { actual: usize, max: usize },
    #[error("invalid canonical ingress payload: {message}")]
    InvalidCanonicalPayload { message: String },
    #[error("invalid canonical action parameters: {message}")]
    InvalidCanonicalParameters { message: String },
    #[error("action names {actual} targets; maximum is {max}")]
    TooManyTargets { actual: usize, max: usize },
    #[error("action target list contains a duplicate perceived reference")]
    DuplicateTarget,
    #[error("stored {field} does not match canonical metadata")]
    HashMismatch { field: &'static str },
    #[error("cannot construct canonical action hash: {0}")]
    Hash(#[from] CanonicalHashError),
}

/// Rules-neutral identity for revision-producing state.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubjectKey {
    Object {
        id: ObjId,
    },
    Scoped {
        owner: OwnerId,
        class: CanonicalId,
        id: IdempotencyKeyText,
    },
}

/// Structurally distinct revision category.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RevisionKind(CanonicalId);

impl RevisionKind {
    #[must_use]
    pub const fn new(value: CanonicalId) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_id(&self) -> &CanonicalId {
        &self.0
    }
}

/// Exact coordinate for one observed revision.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionKey {
    pub owner: OwnerId,
    pub kind: RevisionKind,
    pub subject: SubjectKey,
    pub facet: Option<CanonicalId>,
}

/// One revision observation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionStamp {
    pub key: RevisionKey,
    pub generation: u64,
}

/// Canonically ordered, duplicate-free revision observations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RevisionSet(BTreeMap<RevisionKey, u64>);

impl RevisionSet {
    pub fn from_stamps(
        stamps: impl IntoIterator<Item = RevisionStamp>,
    ) -> Result<Self, RevisionSetError> {
        let mut values = BTreeMap::new();
        for stamp in stamps {
            if values.len() >= MAX_ACTION_REVISIONS {
                return Err(RevisionSetError::TooMany {
                    max: MAX_ACTION_REVISIONS,
                });
            }
            if values.insert(stamp.key.clone(), stamp.generation).is_some() {
                return Err(RevisionSetError::Duplicate {
                    key: Box::new(stamp.key),
                });
            }
        }
        Ok(Self(values))
    }

    fn from_canonical_stamps(stamps: Vec<RevisionStamp>) -> Result<Self, RevisionSetError> {
        if stamps.len() > MAX_ACTION_REVISIONS {
            return Err(RevisionSetError::TooMany {
                max: MAX_ACTION_REVISIONS,
            });
        }
        if stamps.windows(2).any(|pair| pair[0].key >= pair[1].key) {
            return Err(RevisionSetError::NonCanonicalOrder);
        }
        Self::from_stamps(stamps)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&RevisionKey, &u64)> {
        self.0.iter()
    }

    #[must_use]
    pub fn generation(&self, key: &RevisionKey) -> Option<u64> {
        self.0.get(key).copied()
    }
}

impl Serialize for RevisionSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0
            .iter()
            .map(|(key, generation)| RevisionStamp {
                key: key.clone(),
                generation: *generation,
            })
            .collect::<Vec<_>>()
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RevisionSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let stamps = Vec::<RevisionStamp>::deserialize(deserializer)?;
        Self::from_canonical_stamps(stamps).map_err(D::Error::custom)
    }
}

/// Invalid revision-set structure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RevisionSetError {
    #[error("revision set exceeds its hard limit of {max}")]
    TooMany { max: usize },
    #[error("revision key was supplied more than once: {key:?}")]
    Duplicate { key: Box<RevisionKey> },
    #[error("serialized revision stamps must be strictly key-sorted")]
    NonCanonicalOrder,
}

/// Server-side authority assertion retained from planning.
///
/// This type intentionally has no `Deserialize` implementation: guards are
/// coordinator-created facts, never bearer tokens supplied by a client or LLM.
///
/// ```compile_fail
/// fn accepts_client_json<T: serde::de::DeserializeOwned>() {}
/// accepts_client_json::<mush_core::AuthorityGuard>();
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AuthorityGuard {
    Capability {
        provider: OwnerId,
        actor: ObjId,
        capability: CanonicalId,
        generation: u64,
    },
    /// Exact transient station grant selected while projecting an offer.
    ///
    /// This is distinct from a durable [`Self::ControlLease`]. A rules owner
    /// rebuilds station grants from its authoritative assignments at boot;
    /// core binds the chosen grant into the private offer and re-reads the
    /// projection immediately before commit. One witness is retained per
    /// required channel so authority can never be assembled by unioning
    /// channels or priority across seats.
    ChannelGrant {
        provider: OwnerId,
        actor: ObjId,
        subject: ObjId,
        station: SmolStr,
        channel: ChannelId,
        priority: u16,
        epoch: u64,
    },
    ControlLease {
        provider: OwnerId,
        subject: ObjId,
        channel: ChannelId,
        holder: PrincipalId,
        epoch: u64,
    },
    Contact {
        provider: OwnerId,
        principal: PrincipalId,
        id: ContactId,
        generation: u64,
    },
    Reservation {
        provider: OwnerId,
        id: IdempotencyKeyText,
        revision: u64,
    },
    Window {
        owner: OwnerId,
        id: crate::action_window::ActionWindowId,
        generation: u64,
    },
    Definition(DefinitionRef),
}

/// Closed routing category for policy-owned authority validation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityGuardKind {
    Capability,
    ChannelGrant,
    ControlLease,
    Contact,
    Reservation,
    Window,
    Definition,
}

impl AuthorityGuard {
    #[must_use]
    pub const fn kind(&self) -> AuthorityGuardKind {
        match self {
            Self::Capability { .. } => AuthorityGuardKind::Capability,
            Self::ChannelGrant { .. } => AuthorityGuardKind::ChannelGrant,
            Self::ControlLease { .. } => AuthorityGuardKind::ControlLease,
            Self::Contact { .. } => AuthorityGuardKind::Contact,
            Self::Reservation { .. } => AuthorityGuardKind::Reservation,
            Self::Window { .. } => AuthorityGuardKind::Window,
            Self::Definition(_) => AuthorityGuardKind::Definition,
        }
    }

    /// Explicit policy owner used to route final validation. Definitions are
    /// validated centrally against the frozen exact-definition registry.
    #[must_use]
    pub const fn provider(&self) -> Option<&OwnerId> {
        match self {
            Self::Capability { provider, .. }
            | Self::ChannelGrant { provider, .. }
            | Self::ControlLease { provider, .. }
            | Self::Contact { provider, .. }
            | Self::Reservation { provider, .. } => Some(provider),
            Self::Window { owner, .. } => Some(owner),
            Self::Definition(_) => None,
        }
    }

    fn canonical_value(&self) -> Value {
        match self {
            Self::Capability {
                provider,
                actor,
                capability,
                generation,
            } => json!({
                "kind": "capability",
                "provider": provider,
                "actor": actor,
                "capability": capability,
                "generation": generation,
            }),
            Self::ChannelGrant {
                provider,
                actor,
                subject,
                station,
                channel,
                priority,
                epoch,
            } => json!({
                "kind": "channel_grant",
                "provider": provider,
                "actor": actor,
                "subject": subject,
                "station": station,
                "channel": channel,
                "priority": priority,
                "epoch": epoch,
            }),
            Self::ControlLease {
                provider,
                subject,
                channel,
                holder,
                epoch,
            } => json!({
                "kind": "control_lease",
                "provider": provider,
                "subject": subject,
                "channel": channel,
                "holder": holder,
                "epoch": epoch,
            }),
            Self::Contact {
                provider,
                principal,
                id,
                generation,
            } => json!({
                "kind": "contact",
                "provider": provider,
                "principal": principal,
                "id": id,
                "generation": generation,
            }),
            Self::Reservation {
                provider,
                id,
                revision,
            } => json!({
                "kind": "reservation",
                "provider": provider,
                "id": id,
                "revision": revision,
            }),
            Self::Window {
                owner,
                id,
                generation,
            } => json!({
                "kind": "window",
                "owner": owner,
                "id": id,
                "generation": generation,
            }),
            Self::Definition(reference) => json!({
                "kind": "definition",
                "reference": reference,
            }),
        }
    }
}

/// Immutable state visible to a revision validator. Typed-family access still
/// requires the validator's composition-branded handle, so this does not
/// permit cross-owner state reads.
pub struct RevisionValidatorContext<'a> {
    world: &'a World,
    action_state: &'a ActionStateStore,
}

impl fmt::Debug for RevisionValidatorContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RevisionValidatorContext")
            .finish_non_exhaustive()
    }
}

impl<'a> RevisionValidatorContext<'a> {
    pub(crate) const fn new(world: &'a World, action_state: &'a ActionStateStore) -> Self {
        Self {
            world,
            action_state,
        }
    }

    #[must_use]
    pub const fn world(&self) -> &'a World {
        self.world
    }

    #[must_use]
    pub fn module_state<T: Any + Send>(&self) -> Option<&'a T> {
        self.world.module_state::<T>()
    }

    pub fn action_state<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&'a V>, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_state.get(handle, key)
    }
}

/// Pure, object-safe lookup of a revision's current generation.
///
/// `None` means the subject/facet no longer exists and therefore makes an
/// observed stamp stale. An error is an evaluator/configuration failure, not
/// a stale result. Implementations receive immutable world and typed state.
pub trait RevisionValidator: Send + Sync {
    fn current_generation(
        &self,
        context: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError>;
}

/// Immutable, owner-stamped revision validator entry.
pub struct RevisionValidatorEntry {
    owner: OwnerId,
    kind: RevisionKind,
    validator: Arc<dyn RevisionValidator>,
}

impl fmt::Debug for RevisionValidatorEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevisionValidatorEntry")
            .field("owner", &self.owner)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl RevisionValidatorEntry {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn kind(&self) -> &RevisionKind {
        &self.kind
    }

    #[must_use]
    fn validator(&self) -> &dyn RevisionValidator {
        self.validator.as_ref()
    }
}

/// Mutable, composition-only revision validator table.
#[derive(Default)]
pub struct RevisionValidatorRegistryBuilder {
    entries: BTreeMap<(OwnerId, RevisionKind), RevisionValidatorEntry>,
    owner_counts: BTreeMap<OwnerId, usize>,
}

impl fmt::Debug for RevisionValidatorRegistryBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevisionValidatorRegistryBuilder")
            .field("keys", &self.entries.keys().collect::<Vec<_>>())
            .field("owner_counts", &self.owner_counts)
            .finish()
    }
}

impl RevisionValidatorRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register(
        &mut self,
        stamped_owner: OwnerId,
        kind: RevisionKind,
        validator: Arc<dyn RevisionValidator>,
    ) -> Result<(), RevisionValidatorRegistryError> {
        if self.entries.len() >= MAX_REVISION_VALIDATORS {
            return Err(RevisionValidatorRegistryError::ValidatorLimit {
                max: MAX_REVISION_VALIDATORS,
            });
        }
        let owner_count = self.owner_counts.get(&stamped_owner).copied().unwrap_or(0);
        if owner_count >= MAX_REVISION_VALIDATORS_PER_OWNER {
            return Err(RevisionValidatorRegistryError::OwnerValidatorLimit {
                owner: stamped_owner,
                max: MAX_REVISION_VALIDATORS_PER_OWNER,
            });
        }
        let key = (stamped_owner.clone(), kind.clone());
        if self.entries.contains_key(&key) {
            return Err(RevisionValidatorRegistryError::Duplicate {
                owner: stamped_owner,
                kind,
            });
        }
        self.entries.insert(
            key,
            RevisionValidatorEntry {
                owner: stamped_owner.clone(),
                kind,
                validator,
            },
        );
        self.owner_counts.insert(stamped_owner, owner_count + 1);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn freeze_for_owners(
        self,
        module_owners: &BTreeSet<OwnerId>,
    ) -> Result<RevisionValidatorRegistry, RevisionValidatorRegistryError> {
        let readiness = MechanicalOwnerReadiness::for_owners(module_owners);
        self.freeze_for_owners_with_readiness(module_owners, readiness)
    }

    pub(crate) fn freeze_for_owners_with_readiness(
        self,
        module_owners: &BTreeSet<OwnerId>,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<RevisionValidatorRegistry, RevisionValidatorRegistryError> {
        if let Some(owner) = self
            .entries
            .keys()
            .map(|(owner, _)| owner)
            .find(|owner| !module_owners.contains(*owner))
        {
            return Err(RevisionValidatorRegistryError::UnknownOwner {
                owner: owner.clone(),
            });
        }
        let fingerprint = registry_key_fingerprint(
            "revision_validator_registry.v1",
            self.entries
                .keys()
                .map(|(owner, kind)| json!({ "owner": owner, "kind": kind })),
        )
        .map_err(RevisionValidatorRegistryError::Fingerprint)?;
        Ok(RevisionValidatorRegistry {
            entries: self.entries,
            fingerprint,
            owner_readiness,
        })
    }
}

/// Frozen deterministic revision validator lookup.
pub struct RevisionValidatorRegistry {
    entries: BTreeMap<(OwnerId, RevisionKind), RevisionValidatorEntry>,
    fingerprint: DefinitionDigest,
    owner_readiness: MechanicalOwnerReadiness,
}

impl fmt::Debug for RevisionValidatorRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevisionValidatorRegistry")
            .field("entries", &self.entries)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl RevisionValidatorRegistry {
    pub fn get_for_key(
        &self,
        key: &RevisionKey,
    ) -> Result<&RevisionValidatorEntry, RevisionValidatorLookupError> {
        let entry = self
            .entries
            .get(&(key.owner.clone(), key.kind.clone()))
            .ok_or_else(|| RevisionValidatorLookupError::Missing {
                key: Box::new(key.clone()),
            })?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| RevisionValidatorLookupError::OwnerNotReady {
                key: Box::new(key.clone()),
                source: Box::new(source),
            })?;
        Ok(entry)
    }

    /// Read one exact current generation with owner readiness checked before
    /// and after the immutable provider callback.
    pub(crate) fn current_generation(
        &self,
        context: &RevisionValidatorContext<'_>,
        key: &RevisionKey,
    ) -> Result<Option<u64>, RevisionObservationError> {
        let entry = self.get_for_key(key)?;
        let generation = entry
            .validator()
            .current_generation(context, key)
            .map_err(|source| RevisionObservationError::Provider {
                key: Box::new(key.clone()),
                source: Box::new(source),
            })?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| {
                RevisionObservationError::Lookup(RevisionValidatorLookupError::OwnerNotReady {
                    key: Box::new(key.clone()),
                    source: Box::new(source),
                })
            })?;
        Ok(generation)
    }

    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }
}

/// Revision validator composition failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RevisionValidatorRegistryError {
    #[error("revision validator {owner}:{kind:?} was registered more than once")]
    Duplicate { owner: OwnerId, kind: RevisionKind },
    #[error("revision-validator registry exceeds its hard limit of {max}")]
    ValidatorLimit { max: usize },
    #[error("owner {owner} exceeds its revision-validator limit of {max}")]
    OwnerValidatorLimit { owner: OwnerId, max: usize },
    #[error("revision validator names uncomposed owner {owner}")]
    UnknownOwner { owner: OwnerId },
    #[error("cannot fingerprint revision validators: {0}")]
    Fingerprint(#[source] CanonicalEncodingError),
}

/// Missing exact `(owner, revision kind)` validator.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RevisionValidatorLookupError {
    #[error("no revision validator is registered for {key:?}")]
    Missing { key: Box<RevisionKey> },
    #[error("revision validator owner is not ready for {key:?}: {source}")]
    OwnerNotReady {
        key: Box<RevisionKey>,
        source: Box<MechanicalOwnerReadinessError>,
    },
}

/// Failure while the kernel observes a provider-owned revision coordinate.
#[derive(Debug, Error)]
pub enum RevisionObservationError {
    #[error("revision provider cannot be selected: {0}")]
    Lookup(#[from] RevisionValidatorLookupError),
    #[error("revision provider failed for {key:?}: {source}")]
    Provider {
        key: Box<RevisionKey>,
        source: Box<ActionError>,
    },
}

/// Immutable ordinary-world and typed-family truth available to a final
/// authority check. This mirrors the resource/effect read boundary: validators
/// can inspect current authoritative state but cannot mutate it or perform I/O.
#[derive(Clone, Copy)]
pub struct AuthorityGuardContext<'a> {
    world: &'a World,
    action_state: &'a ActionStateStore,
}

/// A pure request for a provider to issue current private authority evidence.
/// The registry stamps the provider route and validates the returned witness;
/// callers cannot turn this into authority by supplying their own guard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorityWitnessRequest {
    ControlLease {
        provider: OwnerId,
        subject: ObjId,
        channel: ChannelId,
        holder: PrincipalId,
    },
    Contact {
        provider: OwnerId,
        principal: PrincipalId,
        id: ContactId,
        generation: u64,
    },
}

impl AuthorityWitnessRequest {
    #[must_use]
    pub const fn provider(&self) -> &OwnerId {
        match self {
            Self::ControlLease { provider, .. } | Self::Contact { provider, .. } => provider,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> AuthorityGuardKind {
        match self {
            Self::ControlLease { .. } => AuthorityGuardKind::ControlLease,
            Self::Contact { .. } => AuthorityGuardKind::Contact,
        }
    }
}

/// Provider-issued current evidence. Contact resolution additionally carries
/// its server-private truth subject; lease evidence never needs one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorityWitness {
    Guard(AuthorityGuard),
    Contact {
        truth_subject: SubjectKey,
        guard: AuthorityGuard,
    },
}

impl AuthorityWitness {
    #[must_use]
    pub const fn guard(&self) -> &AuthorityGuard {
        match self {
            Self::Guard(guard) | Self::Contact { guard, .. } => guard,
        }
    }

    #[must_use]
    pub const fn truth_subject(&self) -> Option<&SubjectKey> {
        match self {
            Self::Guard(_) => None,
            Self::Contact { truth_subject, .. } => Some(truth_subject),
        }
    }

    fn matches_request(&self, request: &AuthorityWitnessRequest) -> bool {
        match (request, self) {
            (
                AuthorityWitnessRequest::ControlLease {
                    provider,
                    subject,
                    channel,
                    holder,
                },
                Self::Guard(AuthorityGuard::ControlLease {
                    provider: actual_provider,
                    subject: actual_subject,
                    channel: actual_channel,
                    holder: actual_holder,
                    ..
                }),
            ) => {
                actual_provider == provider
                    && actual_subject == subject
                    && actual_channel == channel
                    && actual_holder == holder
            }
            (
                AuthorityWitnessRequest::Contact {
                    provider,
                    principal,
                    id,
                    generation,
                },
                Self::Contact {
                    guard:
                        AuthorityGuard::Contact {
                            provider: actual_provider,
                            principal: actual_principal,
                            id: actual_id,
                            generation: actual_generation,
                        },
                    ..
                },
            ) => {
                actual_provider == provider
                    && actual_principal == principal
                    && actual_id == id
                    && actual_generation == generation
            }
            _ => false,
        }
    }
}

impl fmt::Debug for AuthorityGuardContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorityGuardContext")
            .field("utc_now", &self.world.now())
            .field("action_state_revision", &self.action_state.revision())
            .finish_non_exhaustive()
    }
}

impl<'a> AuthorityGuardContext<'a> {
    pub(crate) const fn new(world: &'a World, action_state: &'a ActionStateStore) -> Self {
        Self {
            world,
            action_state,
        }
    }

    #[must_use]
    pub const fn world(&self) -> &'a World {
        self.world
    }

    #[must_use]
    pub fn module_state<T: Any + Send>(&self) -> Option<&'a T> {
        self.world.module_state::<T>()
    }

    pub fn action_state<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&'a V>, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_state.get(handle, key)
    }

    /// Scan one bounded canonical page of a family this validator owns.
    ///
    /// E7c2 B4: a guard whose subject is not its family's key needs a bounded
    /// walk, not a point lookup — `AuthorityGuard::Reservation { provider, id,
    /// revision }` names the reservation, while the book it lives in is keyed by
    /// its own resource. The read boundary is unchanged: still immutable, still
    /// no I/O, still bounded on records **visited** by the caller's `limit`.
    pub fn scan_action_state<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<crate::action_state::StateFamilyPage<'a, V>, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_state.scan_family(handle, after, limit)
    }
}

/// Pure, object-safe final authority check owned by a module.
pub trait AuthorityGuardValidator: Send + Sync {
    /// `true` means the exact assertion is still current; `false` means stale.
    fn is_current(
        &self,
        context: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError>;

    /// Issue current private evidence for a trusted kernel request. Most
    /// validators only check guards and therefore inherit no witness.
    fn witness(
        &self,
        _context: &AuthorityGuardContext<'_>,
        _request: &AuthorityWitnessRequest,
    ) -> Result<Option<AuthorityWitness>, ActionError> {
        Ok(None)
    }
}

/// Immutable, owner-stamped authority validator entry.
pub struct AuthorityGuardValidatorEntry {
    owner: OwnerId,
    kind: AuthorityGuardKind,
    validator: Arc<dyn AuthorityGuardValidator>,
}

impl fmt::Debug for AuthorityGuardValidatorEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorityGuardValidatorEntry")
            .field("owner", &self.owner)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl AuthorityGuardValidatorEntry {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn kind(&self) -> AuthorityGuardKind {
        self.kind
    }

    #[must_use]
    fn validator(&self) -> &dyn AuthorityGuardValidator {
        self.validator.as_ref()
    }
}

/// Mutable, composition-only authority validator table.
#[derive(Default)]
pub struct AuthorityGuardValidatorRegistryBuilder {
    entries: BTreeMap<(OwnerId, AuthorityGuardKind), AuthorityGuardValidatorEntry>,
    owner_counts: BTreeMap<OwnerId, usize>,
}

impl fmt::Debug for AuthorityGuardValidatorRegistryBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorityGuardValidatorRegistryBuilder")
            .field("keys", &self.entries.keys().collect::<Vec<_>>())
            .field("owner_counts", &self.owner_counts)
            .finish()
    }
}

impl AuthorityGuardValidatorRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register(
        &mut self,
        stamped_owner: OwnerId,
        kind: AuthorityGuardKind,
        validator: Arc<dyn AuthorityGuardValidator>,
    ) -> Result<(), AuthorityGuardValidatorRegistryError> {
        if matches!(
            kind,
            AuthorityGuardKind::ChannelGrant | AuthorityGuardKind::Definition
        ) {
            return Err(AuthorityGuardValidatorRegistryError::ReservedKernelKind { kind });
        }
        if self.entries.len() >= MAX_AUTHORITY_VALIDATORS {
            return Err(AuthorityGuardValidatorRegistryError::ValidatorLimit {
                max: MAX_AUTHORITY_VALIDATORS,
            });
        }
        let owner_count = self.owner_counts.get(&stamped_owner).copied().unwrap_or(0);
        if owner_count >= MAX_AUTHORITY_VALIDATORS_PER_OWNER {
            return Err(AuthorityGuardValidatorRegistryError::OwnerValidatorLimit {
                owner: stamped_owner,
                max: MAX_AUTHORITY_VALIDATORS_PER_OWNER,
            });
        }
        let key = (stamped_owner.clone(), kind);
        if self.entries.contains_key(&key) {
            return Err(AuthorityGuardValidatorRegistryError::Duplicate {
                owner: stamped_owner,
                kind,
            });
        }
        self.entries.insert(
            key,
            AuthorityGuardValidatorEntry {
                owner: stamped_owner.clone(),
                kind,
                validator,
            },
        );
        self.owner_counts.insert(stamped_owner, owner_count + 1);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn freeze_for_owners(
        self,
        module_owners: &BTreeSet<OwnerId>,
    ) -> Result<AuthorityGuardValidatorRegistry, AuthorityGuardValidatorRegistryError> {
        let readiness = MechanicalOwnerReadiness::for_owners(module_owners);
        self.freeze_for_owners_with_readiness(module_owners, readiness)
    }

    pub(crate) fn freeze_for_owners_with_readiness(
        self,
        module_owners: &BTreeSet<OwnerId>,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<AuthorityGuardValidatorRegistry, AuthorityGuardValidatorRegistryError> {
        if let Some(owner) = self
            .entries
            .keys()
            .map(|(owner, _)| owner)
            .find(|owner| !module_owners.contains(*owner))
        {
            return Err(AuthorityGuardValidatorRegistryError::UnknownOwner {
                owner: owner.clone(),
            });
        }
        let fingerprint = registry_key_fingerprint(
            "authority_guard_validator_registry.v1",
            self.entries
                .keys()
                .map(|(owner, kind)| json!({ "owner": owner, "kind": kind })),
        )
        .map_err(AuthorityGuardValidatorRegistryError::Fingerprint)?;
        Ok(AuthorityGuardValidatorRegistry {
            entries: self.entries,
            fingerprint,
            owner_readiness,
        })
    }
}

/// Frozen deterministic authority-validator lookup.
pub struct AuthorityGuardValidatorRegistry {
    entries: BTreeMap<(OwnerId, AuthorityGuardKind), AuthorityGuardValidatorEntry>,
    fingerprint: DefinitionDigest,
    owner_readiness: MechanicalOwnerReadiness,
}

impl fmt::Debug for AuthorityGuardValidatorRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorityGuardValidatorRegistry")
            .field("entries", &self.entries)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl AuthorityGuardValidatorRegistry {
    pub fn get_for_guard(
        &self,
        guard: &AuthorityGuard,
    ) -> Result<&AuthorityGuardValidatorEntry, AuthorityGuardValidatorLookupError> {
        if matches!(
            guard,
            AuthorityGuard::ChannelGrant { .. } | AuthorityGuard::Definition(_)
        ) {
            return Err(AuthorityGuardValidatorLookupError::KernelGuardIsCentral);
        }
        let Some(owner) = guard.provider() else {
            return Err(AuthorityGuardValidatorLookupError::KernelGuardIsCentral);
        };
        let entry = self
            .entries
            .get(&(owner.clone(), guard.kind()))
            .ok_or_else(|| AuthorityGuardValidatorLookupError::Missing {
                guard: Box::new(guard.clone()),
            })?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| AuthorityGuardValidatorLookupError::OwnerNotReady {
                guard: Box::new(guard.clone()),
                source: Box::new(source),
            })?;
        Ok(entry)
    }

    /// Route one witness request through the exact ready owner/kind provider,
    /// then centrally verify both its requested identity and currentness.
    pub fn witness(
        &self,
        context: &AuthorityGuardContext<'_>,
        request: &AuthorityWitnessRequest,
    ) -> Result<Option<AuthorityWitness>, AuthorityWitnessError> {
        let key = (request.provider().clone(), request.kind());
        let entry = self
            .entries
            .get(&key)
            .ok_or_else(|| AuthorityWitnessError::Missing {
                provider: key.0.clone(),
                kind: key.1,
            })?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| AuthorityWitnessError::OwnerNotReady {
                provider: entry.owner.clone(),
                kind: entry.kind,
                source: Box::new(source),
            })?;
        let witness = entry
            .validator()
            .witness(context, request)
            .map_err(|source| AuthorityWitnessError::Provider {
                provider: entry.owner.clone(),
                kind: entry.kind,
                source: Box::new(source),
            })?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| AuthorityWitnessError::OwnerNotReady {
                provider: entry.owner.clone(),
                kind: entry.kind,
                source: Box::new(source),
            })?;
        let Some(witness) = witness else {
            return Ok(None);
        };
        if !witness.matches_request(request) {
            return Err(AuthorityWitnessError::Malformed {
                provider: entry.owner.clone(),
                kind: entry.kind,
            });
        }
        if !entry
            .validator()
            .is_current(context, witness.guard())
            .map_err(|source| AuthorityWitnessError::Provider {
                provider: entry.owner.clone(),
                kind: entry.kind,
                source: Box::new(source),
            })?
        {
            return Err(AuthorityWitnessError::Stale {
                provider: entry.owner.clone(),
                kind: entry.kind,
            });
        }
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| AuthorityWitnessError::OwnerNotReady {
                provider: entry.owner.clone(),
                kind: entry.kind,
                source: Box::new(source),
            })?;
        Ok(Some(witness))
    }

    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }
}

/// Failure while issuing server-private authority evidence.
#[derive(Debug, Error)]
pub enum AuthorityWitnessError {
    #[error("no witness provider is registered for {provider}:{kind:?}")]
    Missing {
        provider: OwnerId,
        kind: AuthorityGuardKind,
    },
    #[error("witness provider {provider}:{kind:?} is not ready: {source}")]
    OwnerNotReady {
        provider: OwnerId,
        kind: AuthorityGuardKind,
        source: Box<MechanicalOwnerReadinessError>,
    },
    #[error("witness provider {provider}:{kind:?} failed: {source}")]
    Provider {
        provider: OwnerId,
        kind: AuthorityGuardKind,
        source: Box<ActionError>,
    },
    #[error("witness provider {provider}:{kind:?} returned mismatched evidence")]
    Malformed {
        provider: OwnerId,
        kind: AuthorityGuardKind,
    },
    #[error("witness provider {provider}:{kind:?} returned evidence that was not current")]
    Stale {
        provider: OwnerId,
        kind: AuthorityGuardKind,
    },
}

/// Authority validator composition failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AuthorityGuardValidatorRegistryError {
    #[error("authority validator {owner}:{kind:?} was registered more than once")]
    Duplicate {
        owner: OwnerId,
        kind: AuthorityGuardKind,
    },
    #[error("{kind:?} authority is validated centrally and cannot register an evaluator")]
    ReservedKernelKind { kind: AuthorityGuardKind },
    #[error("authority-validator registry exceeds its hard limit of {max}")]
    ValidatorLimit { max: usize },
    #[error("owner {owner} exceeds its authority-validator limit of {max}")]
    OwnerValidatorLimit { owner: OwnerId, max: usize },
    #[error("authority validator names uncomposed owner {owner}")]
    UnknownOwner { owner: OwnerId },
    #[error("cannot fingerprint authority validators: {0}")]
    Fingerprint(#[source] CanonicalEncodingError),
}

/// Missing exact `(owner, authority kind)` validator.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AuthorityGuardValidatorLookupError {
    #[error("kernel-owned guards use central validation")]
    KernelGuardIsCentral,
    #[error("no authority validator is registered for {guard:?}")]
    Missing { guard: Box<AuthorityGuard> },
    #[error("authority validator owner is not ready for {guard:?}: {source}")]
    OwnerNotReady {
        guard: Box<AuthorityGuard>,
        source: Box<MechanicalOwnerReadinessError>,
    },
}

fn registry_key_fingerprint(
    domain: &str,
    values: impl Iterator<Item = Value>,
) -> Result<DefinitionDigest, CanonicalEncodingError> {
    DefinitionDigest::for_canonical_value(domain, &Value::Array(values.collect()))
}

/// Server-owned guard bundle. It is intentionally not deserializable.
///
/// ```compile_fail
/// use mush_core::GuardSet;
/// let _: GuardSet = serde_json::from_str("{}").unwrap();
/// ```
///
/// A deadline is intentionally absent from this first slice. C4 must supply a
/// typed `ClockStamp`; substituting an untyped Unix integer here would reopen
/// the clock-domain confusion that contract is meant to prevent.
#[derive(Clone)]
pub struct GuardSet {
    revisions: RevisionSet,
    authority: Vec<AuthorityGuard>,
    owner_readiness: MechanicalOwnerReadiness,
}

impl fmt::Debug for GuardSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuardSet")
            .field("revisions", &self.revisions)
            .field("authority", &self.authority)
            .finish_non_exhaustive()
    }
}

impl PartialEq for GuardSet {
    fn eq(&self, other: &Self) -> bool {
        self.revisions == other.revisions
            && self.authority == other.authority
            && self
                .owner_readiness
                .shares_latch_with(&other.owner_readiness)
    }
}

impl Eq for GuardSet {}

impl GuardSet {
    #[must_use]
    pub const fn revisions(&self) -> &RevisionSet {
        &self.revisions
    }

    #[must_use]
    pub fn authority(&self) -> &[AuthorityGuard] {
        &self.authority
    }

    /// Add kernel-observed revisions selected only after private target
    /// resolution. Providers name coordinates; the kernel supplies values.
    pub(crate) fn with_revision_stamps(
        self,
        stamps: Vec<RevisionStamp>,
    ) -> Result<Self, GuardSetBuildError> {
        if stamps.is_empty() {
            return Ok(self);
        }
        let revisions = RevisionSet::from_stamps(
            self.revisions
                .iter()
                .map(|(key, generation)| RevisionStamp {
                    key: key.clone(),
                    generation: *generation,
                })
                .chain(stamps),
        )
        .map_err(GuardSetBuildError::Revision)?;
        let guards = Self {
            revisions,
            authority: self.authority,
            owner_readiness: self.owner_readiness,
        };
        canonical_json_bytes_with_limit(&guards.canonical_value(), MAX_GUARD_SET_BYTES).map_err(
            |_| GuardSetBuildError::EncodedTooLarge {
                max: MAX_GUARD_SET_BYTES,
            },
        )?;
        Ok(guards)
    }

    /// Restore private prompt-time revisions onto a freshly regenerated
    /// delayed action. A coordinate may also be named by ordinary selected-
    /// target resolution; in that case the older prompt generation replaces
    /// the newly observed value so final revalidation detects intervening
    /// movement. Duplicates within the prompt snapshot remain corruption.
    pub(crate) fn with_prompt_revision_stamps(
        self,
        stamps: Vec<RevisionStamp>,
    ) -> Result<Self, GuardSetBuildError> {
        if stamps.is_empty() {
            return Ok(self);
        }
        let mut prompt = BTreeMap::new();
        for stamp in stamps {
            if prompt.insert(stamp.key.clone(), stamp.generation).is_some() {
                return Err(GuardSetBuildError::Revision(RevisionSetError::Duplicate {
                    key: Box::new(stamp.key),
                }));
            }
        }
        let mut merged = self.revisions.0;
        for (key, generation) in prompt {
            merged.insert(key, generation);
        }
        if merged.len() > MAX_ACTION_REVISIONS {
            return Err(GuardSetBuildError::Revision(RevisionSetError::TooMany {
                max: MAX_ACTION_REVISIONS,
            }));
        }
        let guards = Self {
            revisions: RevisionSet(merged),
            authority: self.authority,
            owner_readiness: self.owner_readiness,
        };
        canonical_json_bytes_with_limit(&guards.canonical_value(), MAX_GUARD_SET_BYTES).map_err(
            |_| GuardSetBuildError::EncodedTooLarge {
                max: MAX_GUARD_SET_BYTES,
            },
        )?;
        Ok(guards)
    }

    /// Bind exact bridge definitions selected by the frozen cross-owner route.
    /// Existing provider-supplied definition guards are retained without
    /// duplication, while every newly selected definition is revalidated
    /// against this composition before it becomes a final guard.
    pub(crate) fn with_definition_guards(
        self,
        references: Vec<DefinitionRef>,
        definitions: &DefinitionRegistry,
    ) -> Result<Self, GuardSetBuildError> {
        if references.is_empty() {
            return Ok(self);
        }
        let mut builder = ServerGuardSetBuilder {
            revisions: self.revisions,
            authority: self.authority.into_iter().collect(),
            owner_readiness: self.owner_readiness,
        };
        for reference in references {
            let guard = AuthorityGuard::Definition(reference.clone());
            if !builder.authority.contains(&guard) {
                builder.add_definition(reference, definitions)?;
            }
        }
        builder.finish()
    }

    /// Add the kernel-selected exact station witnesses for one legal offer.
    ///
    /// Kept crate-private so providers cannot claim that arbitrary projected
    /// station data was selected by the kernel. The provider declares only
    /// public required channels; registry projection chooses one current
    /// station and adds these private guards.
    pub(crate) fn with_channel_grant(
        self,
        grant_owner: &OwnerId,
        grant: &crate::world::ChannelGrant,
        required_channels: &[ChannelId],
    ) -> Result<Self, GuardSetBuildError> {
        self.owner_readiness
            .require_ready(grant_owner)
            .map_err(|source| GuardSetBuildError::GrantOwnerUnavailable {
                owner: grant_owner.clone(),
                source: Box::new(source),
            })?;
        let mut builder = ServerGuardSetBuilder {
            revisions: self.revisions,
            authority: self.authority.into_iter().collect(),
            owner_readiness: self.owner_readiness,
        };
        for channel in required_channels {
            builder.add(AuthorityGuard::ChannelGrant {
                provider: grant_owner.clone(),
                actor: grant.actor,
                subject: grant.subject,
                station: grant.station.clone(),
                channel: channel.clone(),
                priority: grant.priority,
                epoch: grant.epoch,
            })?;
        }
        builder.finish()
    }

    /// Add authority evidence issued by the kernel's subject resolver. Kept
    /// crate-private so an action provider cannot replace the resolver's
    /// private contact/lease witness with self-asserted data.
    pub(crate) fn with_authority_guards(
        self,
        guards: &[AuthorityGuard],
    ) -> Result<Self, GuardSetBuildError> {
        if guards.is_empty() {
            return Ok(self);
        }
        let mut builder = ServerGuardSetBuilder {
            revisions: self.revisions,
            authority: self.authority.into_iter().collect(),
            owner_readiness: self.owner_readiness,
        };
        for guard in guards {
            builder.add(guard.clone())?;
        }
        builder.finish()
    }

    /// Unit-test convenience for guard sets whose validators do not depend on
    /// typed state. Production admission always uses
    /// [`Self::revalidate`].
    #[cfg(test)]
    fn revalidate_unregistered(
        &self,
        world: &World,
        revisions: &RevisionValidatorRegistry,
        authority: &AuthorityGuardValidatorRegistry,
        definitions: &DefinitionRegistry,
    ) -> Result<GuardRevalidation, GuardRevalidationError> {
        let action_state = ActionStateStore::unregistered();
        self.revalidate(world, &action_state, revisions, authority, definitions)
    }

    /// Re-read every registered source against immutable world and typed-state
    /// truth.
    ///
    /// This method is deliberately pure. The world-task coordinator must use
    /// a current result as part of its later atomic transition into
    /// `CommitPending`; this method does not perform that transition itself.
    pub fn revalidate(
        &self,
        world: &World,
        action_state: &ActionStateStore,
        revisions: &RevisionValidatorRegistry,
        authority: &AuthorityGuardValidatorRegistry,
        definitions: &DefinitionRegistry,
    ) -> Result<GuardRevalidation, GuardRevalidationError> {
        self.require_same_composition(revisions, authority, definitions)?;
        let revision_context = RevisionValidatorContext::new(world, action_state);
        let authority_context = AuthorityGuardContext::new(world, action_state);
        let mut stale_revisions = Vec::new();
        for (key, expected) in self.revisions.iter() {
            let entry = revisions
                .get_for_key(key)
                .map_err(GuardRevalidationError::RevisionLookup)?;
            let actual = entry
                .validator()
                .current_generation(&revision_context, key)
                .map_err(|source| GuardRevalidationError::RevisionEvaluator {
                    key: Box::new(key.clone()),
                    source: Box::new(source),
                })?;
            revisions
                .owner_readiness
                .require_ready(&entry.owner)
                .map_err(|source| {
                    GuardRevalidationError::RevisionLookup(
                        RevisionValidatorLookupError::OwnerNotReady {
                            key: Box::new(key.clone()),
                            source: Box::new(source),
                        },
                    )
                })?;
            if actual != Some(*expected) {
                stale_revisions.push(RevisionMismatch {
                    key: key.clone(),
                    expected: *expected,
                    actual,
                });
            }
        }

        let mut stale_authority = Vec::new();
        for guard in &self.authority {
            let is_current = match guard {
                AuthorityGuard::ChannelGrant {
                    provider,
                    actor,
                    subject,
                    station,
                    channel,
                    priority,
                    epoch,
                } => {
                    self.owner_readiness
                        .require_ready(provider)
                        .map_err(|source| GuardRevalidationError::ChannelGrantUnavailable {
                            guard: Box::new(guard.clone()),
                            source: Box::new(source),
                        })?;
                    let current = world
                        .channel_grant(*actor, *subject, provider.as_str(), station.as_str())
                        .is_some_and(|grant| {
                            grant.priority == *priority
                                && grant.epoch == *epoch
                                && grant.channels.contains(channel)
                        });
                    // Readiness may be frozen independently of the world task.
                    // Discard an apparent match if the owner froze during the
                    // projection read.
                    self.owner_readiness
                        .require_ready(provider)
                        .map_err(|source| GuardRevalidationError::ChannelGrantUnavailable {
                            guard: Box::new(guard.clone()),
                            source: Box::new(source),
                        })?;
                    current
                }
                AuthorityGuard::Definition(reference) => match definitions.get_exact(reference) {
                    Ok(_) => true,
                    Err(
                        DefinitionLookupError::Missing { .. }
                        | DefinitionLookupError::DigestMismatch { .. },
                    ) => false,
                    Err(source) => {
                        return Err(GuardRevalidationError::DefinitionUnavailable {
                            reference: Box::new(reference.clone()),
                            source: Box::new(source),
                        });
                    }
                },
                _ => {
                    let entry = authority
                        .get_for_guard(guard)
                        .map_err(GuardRevalidationError::AuthorityLookup)?;
                    let current = entry
                        .validator()
                        .is_current(&authority_context, guard)
                        .map_err(|source| GuardRevalidationError::AuthorityEvaluator {
                            guard: Box::new(guard.clone()),
                            source: Box::new(source),
                        })?;
                    authority
                        .owner_readiness
                        .require_ready(&entry.owner)
                        .map_err(|source| {
                            GuardRevalidationError::AuthorityLookup(
                                AuthorityGuardValidatorLookupError::OwnerNotReady {
                                    guard: Box::new(guard.clone()),
                                    source: Box::new(source),
                                },
                            )
                        })?;
                    current
                }
            };
            if !is_current {
                stale_authority.push(guard.clone());
            }
        }
        Ok(GuardRevalidation {
            stale_revisions,
            stale_authority,
        })
    }

    fn require_same_composition(
        &self,
        revisions: &RevisionValidatorRegistry,
        authority: &AuthorityGuardValidatorRegistry,
        definitions: &DefinitionRegistry,
    ) -> Result<(), GuardRevalidationError> {
        let definition_readiness = definitions.owner_readiness().ok_or(
            GuardRevalidationError::RegistryCompositionMismatch {
                component: "definitions",
            },
        )?;
        for (component, readiness) in [
            ("revision validators", &revisions.owner_readiness),
            ("authority validators", &authority.owner_readiness),
            ("definitions", definition_readiness),
        ] {
            if !self.owner_readiness.shares_latch_with(readiness) {
                return Err(GuardRevalidationError::RegistryCompositionMismatch { component });
            }
        }
        Ok(())
    }

    fn canonical_value(&self) -> Value {
        let revisions = self
            .revisions
            .iter()
            .map(|(key, generation)| json!({ "key": key, "generation": generation }))
            .collect::<Vec<_>>();
        let authority = self
            .authority
            .iter()
            .map(AuthorityGuard::canonical_value)
            .collect::<Vec<_>>();
        json!({ "revisions": revisions, "authority": authority })
    }

    /// Canonical integrity hash for later plan construction. This does not
    /// make the guard set a client credential or a serializable wire type.
    pub fn canonical_hash(&self) -> Result<CanonicalHash, CanonicalHashError> {
        CanonicalHash::for_canonical_value(
            "guard_set.v1",
            1,
            &self.canonical_value(),
            MAX_GUARD_SET_BYTES,
        )
    }
}

/// One observed revision that no longer matches current truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionMismatch {
    pub key: RevisionKey,
    pub expected: u64,
    pub actual: Option<u64>,
}

/// Deterministically ordered final guard-check result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardRevalidation {
    stale_revisions: Vec<RevisionMismatch>,
    stale_authority: Vec<AuthorityGuard>,
}

impl GuardRevalidation {
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.stale_revisions.is_empty() && self.stale_authority.is_empty()
    }

    #[must_use]
    pub fn stale_revisions(&self) -> &[RevisionMismatch] {
        &self.stale_revisions
    }

    #[must_use]
    pub fn stale_authority(&self) -> &[AuthorityGuard] {
        &self.stale_authority
    }
}

/// Evaluator/configuration failure during otherwise pure guard revalidation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GuardRevalidationError {
    #[error("{component} do not belong to the guard set's module composition")]
    RegistryCompositionMismatch { component: &'static str },
    #[error("revision guard cannot be evaluated: {0}")]
    RevisionLookup(#[source] RevisionValidatorLookupError),
    #[error("revision evaluator failed for {key:?}: {source}")]
    RevisionEvaluator {
        key: Box<RevisionKey>,
        source: Box<ActionError>,
    },
    #[error("authority guard cannot be evaluated: {0}")]
    AuthorityLookup(#[source] AuthorityGuardValidatorLookupError),
    #[error("authority evaluator failed for {guard:?}: {source}")]
    AuthorityEvaluator {
        guard: Box<AuthorityGuard>,
        source: Box<ActionError>,
    },
    #[error("definition guard {reference} cannot be evaluated: {source}")]
    DefinitionUnavailable {
        reference: Box<DefinitionRef>,
        source: Box<DefinitionLookupError>,
    },
    #[error("channel-grant guard {guard:?} cannot be evaluated: {source}")]
    ChannelGrantUnavailable {
        guard: Box<AuthorityGuard>,
        source: Box<MechanicalOwnerReadinessError>,
    },
}

/// Builder with an explicitly trusted/server-owned construction name.
#[derive(Debug)]
pub struct ServerGuardSetBuilder {
    revisions: RevisionSet,
    authority: BTreeSet<AuthorityGuard>,
    owner_readiness: MechanicalOwnerReadiness,
}

impl ServerGuardSetBuilder {
    #[must_use]
    pub(crate) fn new(revisions: RevisionSet, owner_readiness: MechanicalOwnerReadiness) -> Self {
        Self {
            revisions,
            authority: BTreeSet::new(),
            owner_readiness,
        }
    }

    pub fn add(&mut self, guard: AuthorityGuard) -> Result<&mut Self, GuardSetBuildError> {
        if matches!(guard, AuthorityGuard::Definition(_)) {
            return Err(GuardSetBuildError::DefinitionRequiresRegistry);
        }
        self.insert_validated(guard)
    }

    fn insert_validated(&mut self, guard: AuthorityGuard) -> Result<&mut Self, GuardSetBuildError> {
        if self.authority.len() >= MAX_AUTHORITY_GUARDS {
            return Err(GuardSetBuildError::TooMany {
                max: MAX_AUTHORITY_GUARDS,
            });
        }
        if !self.authority.insert(guard.clone()) {
            return Err(GuardSetBuildError::Duplicate {
                guard: Box::new(guard),
            });
        }
        Ok(self)
    }

    pub fn add_definition(
        &mut self,
        reference: DefinitionRef,
        definitions: &DefinitionRegistry,
    ) -> Result<&mut Self, GuardSetBuildError> {
        let Some(definition_readiness) = definitions.owner_readiness() else {
            return Err(GuardSetBuildError::RegistryCompositionMismatch);
        };
        if !self.owner_readiness.shares_latch_with(definition_readiness) {
            return Err(GuardSetBuildError::RegistryCompositionMismatch);
        }
        definitions
            .get_exact(&reference)
            .map_err(|error| GuardSetBuildError::Definition(Box::new(error)))?;
        self.insert_validated(AuthorityGuard::Definition(reference))
    }

    pub fn finish(self) -> Result<GuardSet, GuardSetBuildError> {
        let guards = GuardSet {
            revisions: self.revisions,
            authority: self.authority.into_iter().collect(),
            owner_readiness: self.owner_readiness,
        };
        canonical_json_bytes_with_limit(&guards.canonical_value(), MAX_GUARD_SET_BYTES).map_err(
            |_| GuardSetBuildError::EncodedTooLarge {
                max: MAX_GUARD_SET_BYTES,
            },
        )?;
        Ok(guards)
    }
}

/// Invalid server guard construction.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GuardSetBuildError {
    #[error("guard builder is not bound to a composed mechanics registry")]
    RegistryUnavailable,
    #[error("guard definitions do not belong to the builder's module composition")]
    RegistryCompositionMismatch,
    #[error("authority guard set exceeds its hard limit of {max}")]
    TooMany { max: usize },
    #[error("authority guard was supplied more than once: {guard:?}")]
    Duplicate { guard: Box<AuthorityGuard> },
    #[error("revision guard set is invalid: {0}")]
    Revision(#[source] RevisionSetError),
    #[error("definition guard does not name an exact registered definition: {0}")]
    Definition(#[source] Box<DefinitionLookupError>),
    #[error("definition guards must be added through exact registry validation")]
    DefinitionRequiresRegistry,
    #[error("channel-grant publisher {owner} is not a ready composed mechanics owner: {source}")]
    GrantOwnerUnavailable {
        owner: OwnerId,
        source: Box<MechanicalOwnerReadinessError>,
    },
    #[error("canonical guard set exceeds its hard byte limit of {max}")]
    EncodedTooLarge { max: usize },
}

/// Stable pre-commit action failure category.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionErrorCode {
    InvalidRequest,
    Unauthorized,
    Stale,
    Conflict,
    Busy,
    Unavailable,
    BoundsExceeded,
    ProviderRejected,
    Integrity,
}

/// Bounded error metadata. Player projection remains a separate policy step.
#[derive(Clone, Debug, Deserialize, Eq, Error, PartialEq, Serialize)]
#[error("{code:?}: {message}")]
#[serde(try_from = "ActionErrorWire")]
pub struct ActionError {
    code: ActionErrorCode,
    retryable: bool,
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionErrorWire {
    code: ActionErrorCode,
    retryable: bool,
    message: String,
}

impl ActionError {
    pub fn new(
        code: ActionErrorCode,
        retryable: bool,
        message: impl Into<String>,
    ) -> Result<Self, ActionErrorBuildError> {
        let message = message.into();
        if message.len() > MAX_ACTION_ERROR_BYTES {
            return Err(ActionErrorBuildError::TooLong {
                actual: message.len(),
                max: MAX_ACTION_ERROR_BYTES,
            });
        }
        Ok(Self {
            code,
            retryable,
            message,
        })
    }

    #[must_use]
    pub const fn code(&self) -> ActionErrorCode {
        self.code
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl TryFrom<ActionErrorWire> for ActionError {
    type Error = ActionErrorBuildError;

    fn try_from(value: ActionErrorWire) -> Result<Self, Self::Error> {
        Self::new(value.code, value.retryable, value.message)
    }
}

/// Invalid bounded error metadata.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionErrorBuildError {
    #[error("action error message is {actual} bytes; maximum is {max}")]
    TooLong { actual: usize, max: usize },
}

/// Pre-K3 lifecycle. There is deliberately no authoritative `Committed` value.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionLifecycle {
    IngressHeld,
    Validating,
    Prepared,
    CommitPending,
    Rejected,
    Aborted,
}

/// Common identity fields a future K3 authoritative receipt will embed.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "ActionReceiptMetadataWire")]
pub struct ActionReceiptMetadata {
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    action: Option<DefinitionRef>,
    provider: Option<OwnerId>,
    request_hash: Option<CanonicalHash>,
    plan_hash: Option<CanonicalHash>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionReceiptMetadataWire {
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    action: Option<DefinitionRef>,
    provider: Option<OwnerId>,
    request_hash: Option<CanonicalHash>,
    plan_hash: Option<CanonicalHash>,
}

impl ActionReceiptMetadata {
    pub fn ingress(envelope: &IngressEnvelope) -> Self {
        Self {
            request: envelope.request,
            idempotency: envelope.idempotency.clone(),
            ingress_hash: envelope.ingress_hash,
            action: None,
            provider: None,
            request_hash: None,
            plan_hash: None,
        }
    }

    pub fn bind(
        mut self,
        action: DefinitionRef,
        provider: OwnerId,
        invocation: &ActionInvocation,
    ) -> Result<Self, ActionReceiptMetadataError> {
        if invocation.request != self.request || invocation.idempotency != self.idempotency {
            return Err(ActionReceiptMetadataError::InvocationIdentityMismatch);
        }
        if !is_action_owned_by(&provider, &action) {
            return Err(ActionReceiptMetadataError::WrongActionOwner {
                provider,
                action: Box::new(action),
            });
        }
        self.action = Some(action);
        self.provider = Some(provider);
        self.request_hash = Some(invocation.request_hash);
        Ok(self)
    }

    pub fn with_plan_hash(
        mut self,
        plan_hash: CanonicalHash,
    ) -> Result<Self, ActionReceiptMetadataError> {
        if self.request_hash.is_none() {
            return Err(ActionReceiptMetadataError::PlanWithoutBoundRequest);
        }
        self.plan_hash = Some(plan_hash);
        Ok(self)
    }

    #[must_use]
    pub const fn request(&self) -> RequestUid {
        self.request
    }

    #[must_use]
    pub const fn request_hash(&self) -> Option<CanonicalHash> {
        self.request_hash
    }

    #[must_use]
    pub const fn plan_hash(&self) -> Option<CanonicalHash> {
        self.plan_hash
    }

    #[must_use]
    pub const fn action(&self) -> Option<&DefinitionRef> {
        self.action.as_ref()
    }

    #[must_use]
    pub const fn provider(&self) -> Option<&OwnerId> {
        self.provider.as_ref()
    }
}

impl TryFrom<ActionReceiptMetadataWire> for ActionReceiptMetadata {
    type Error = ActionReceiptMetadataError;

    fn try_from(value: ActionReceiptMetadataWire) -> Result<Self, Self::Error> {
        validate_request_key(value.request, &value.idempotency)
            .map_err(|_| ActionReceiptMetadataError::RequestKeyMismatch)?;
        let bound_count = usize::from(value.action.is_some())
            + usize::from(value.provider.is_some())
            + usize::from(value.request_hash.is_some());
        if !matches!(bound_count, 0 | 3) {
            return Err(ActionReceiptMetadataError::PartialBinding);
        }
        if value.plan_hash.is_some() && value.request_hash.is_none() {
            return Err(ActionReceiptMetadataError::PlanWithoutBoundRequest);
        }
        if let (Some(provider), Some(action)) = (&value.provider, &value.action)
            && !is_action_owned_by(provider, action)
        {
            return Err(ActionReceiptMetadataError::WrongActionOwner {
                provider: provider.clone(),
                action: Box::new(action.clone()),
            });
        }
        Ok(Self {
            request: value.request,
            idempotency: value.idempotency,
            ingress_hash: value.ingress_hash,
            action: value.action,
            provider: value.provider,
            request_hash: value.request_hash,
            plan_hash: value.plan_hash,
        })
    }
}

/// Invalid receipt identity metadata.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionReceiptMetadataError {
    #[error("receipt request uid does not match its request-key identity")]
    RequestKeyMismatch,
    #[error("bound invocation does not match receipt ingress identity")]
    InvocationIdentityMismatch,
    #[error("action binding must contain action, provider, and request hash together")]
    PartialBinding,
    #[error("plan hash cannot exist before the semantic request is bound")]
    PlanWithoutBoundRequest,
    #[error("definition {action} is not an action owned by provider {provider}")]
    WrongActionOwner {
        provider: OwnerId,
        action: Box<DefinitionRef>,
    },
}

/// Validated pre-commit lifecycle observation, not an authoritative receipt.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "ActionAttemptMetadataWire")]
pub struct ActionAttemptMetadata {
    lifecycle: ActionLifecycle,
    receipt: ActionReceiptMetadata,
    failure: Option<ActionError>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionAttemptMetadataWire {
    lifecycle: ActionLifecycle,
    receipt: ActionReceiptMetadata,
    failure: Option<ActionError>,
}

impl ActionAttemptMetadata {
    pub fn new(
        lifecycle: ActionLifecycle,
        receipt: ActionReceiptMetadata,
        failure: Option<ActionError>,
    ) -> Result<Self, ActionAttemptMetadataError> {
        let terminal_failure = matches!(
            lifecycle,
            ActionLifecycle::Rejected | ActionLifecycle::Aborted
        );
        if terminal_failure != failure.is_some() {
            return Err(ActionAttemptMetadataError::FailurePhaseMismatch);
        }
        if matches!(
            lifecycle,
            ActionLifecycle::Prepared | ActionLifecycle::CommitPending
        ) && receipt.plan_hash.is_none()
        {
            return Err(ActionAttemptMetadataError::PreparedWithoutPlan);
        }
        Ok(Self {
            lifecycle,
            receipt,
            failure,
        })
    }

    #[must_use]
    pub const fn lifecycle(&self) -> ActionLifecycle {
        self.lifecycle
    }

    #[must_use]
    pub const fn receipt(&self) -> &ActionReceiptMetadata {
        &self.receipt
    }

    #[must_use]
    pub const fn failure(&self) -> Option<&ActionError> {
        self.failure.as_ref()
    }
}

impl TryFrom<ActionAttemptMetadataWire> for ActionAttemptMetadata {
    type Error = ActionAttemptMetadataError;

    fn try_from(value: ActionAttemptMetadataWire) -> Result<Self, Self::Error> {
        Self::new(value.lifecycle, value.receipt, value.failure)
    }
}

/// Invalid pre-commit lifecycle metadata.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionAttemptMetadataError {
    #[error("only rejected or aborted action attempts carry failure metadata")]
    FailurePhaseMismatch,
    #[error("prepared and commit-pending attempts require a plan hash")]
    PreparedWithoutPlan,
}

/// Event identity before K3 binds the causal root to a commit token.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "PreparedEventIdWire")]
pub struct PreparedEventId {
    request: RequestUid,
    ordinal: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedEventIdWire {
    request: RequestUid,
    ordinal: u32,
}

impl PreparedEventId {
    pub fn new(request: RequestUid, ordinal: u32) -> Result<Self, CausalMetadataError> {
        if ordinal >= MAX_SEMANTIC_EVENTS {
            return Err(CausalMetadataError::OrdinalOutOfRange {
                ordinal,
                max_exclusive: MAX_SEMANTIC_EVENTS,
            });
        }
        Ok(Self { request, ordinal })
    }

    #[must_use]
    pub const fn request(&self) -> RequestUid {
        self.request
    }

    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }
}

impl TryFrom<PreparedEventIdWire> for PreparedEventId {
    type Error = CausalMetadataError;

    fn try_from(value: PreparedEventIdWire) -> Result<Self, Self::Error> {
        Self::new(value.request, value.ordinal)
    }
}

/// Causal link within one prepared action, before commit-token binding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "PreparedCausalRefWire")]
pub struct PreparedCausalRef {
    root_request: RequestUid,
    parent: Option<PreparedEventId>,
    ordinal: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedCausalRefWire {
    root_request: RequestUid,
    parent: Option<PreparedEventId>,
    ordinal: u32,
}

impl PreparedCausalRef {
    pub fn new(
        root_request: RequestUid,
        parent: Option<PreparedEventId>,
        ordinal: u32,
    ) -> Result<Self, CausalMetadataError> {
        PreparedEventId::new(root_request, ordinal)?;
        if let Some(parent) = parent {
            if parent.request != root_request {
                return Err(CausalMetadataError::ParentRootMismatch);
            }
            if parent.ordinal >= ordinal {
                return Err(CausalMetadataError::ParentNotEarlier);
            }
        }
        Ok(Self {
            root_request,
            parent,
            ordinal,
        })
    }

    #[must_use]
    pub const fn event_id(self) -> PreparedEventId {
        PreparedEventId {
            request: self.root_request,
            ordinal: self.ordinal,
        }
    }

    #[must_use]
    pub const fn root_request(self) -> RequestUid {
        self.root_request
    }

    #[must_use]
    pub const fn parent(self) -> Option<PreparedEventId> {
        self.parent
    }

    #[must_use]
    pub const fn ordinal(self) -> u32 {
        self.ordinal
    }
}

impl TryFrom<PreparedCausalRefWire> for PreparedCausalRef {
    type Error = CausalMetadataError;

    fn try_from(value: PreparedCausalRefWire) -> Result<Self, Self::Error> {
        Self::new(value.root_request, value.parent, value.ordinal)
    }
}

/// Invalid pre-commit causal identity.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CausalMetadataError {
    #[error("semantic-event ordinal {ordinal} is outside 0..{max_exclusive}")]
    OrdinalOutOfRange { ordinal: u32, max_exclusive: u32 },
    #[error("causal parent belongs to a different root request")]
    ParentRootMismatch,
    #[error("causal parent ordinal must precede the child ordinal")]
    ParentNotEarlier,
}

/// Owner-stamped identity of a same-commit trigger handler.
///
/// It is serializable for canonical plan hashing but intentionally not
/// deserializable or directly constructible: the frozen provider entry stamps
/// the owner through [`PreparedTriggerDraft::new`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerHandlerId {
    owner: OwnerId,
    id: CanonicalId,
}

impl TriggerHandlerId {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn id(&self) -> &CanonicalId {
        &self.id
    }
}

/// Duplicate-suppression identity mandated by C6.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TriggerIdentity {
    pub root_request: RequestUid,
    pub source_event: PreparedEventId,
    pub handler: TriggerHandlerId,
    pub target: SubjectKey,
}

/// Inputs for one bounded same-commit trigger work item.
#[derive(Clone, Debug)]
pub struct PreparedTriggerDraft {
    source_event: PreparedEventId,
    handler: TriggerHandlerId,
    target: SubjectKey,
    phase: u16,
    provider_registration_ordinal: ProviderRegistrationOrdinal,
    payload: Value,
}

impl PreparedTriggerDraft {
    /// Construct work for one exact frozen provider. The provider's owner,
    /// canonical ordering ordinal, and process-local composition latch are
    /// stamped rather than accepted as caller-controlled values.
    #[must_use]
    pub fn new(
        provider: &ActionProviderEntry,
        source_event: PreparedEventId,
        handler: CanonicalId,
        target: SubjectKey,
        phase: u16,
        payload: Value,
    ) -> Self {
        Self {
            source_event,
            handler: TriggerHandlerId {
                owner: provider.owner.clone(),
                id: handler,
            },
            target,
            phase,
            provider_registration_ordinal: provider.registration_ordinal().clone(),
            payload,
        }
    }
}

/// Canonical, immutable trigger work. It carries no mutation authority.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedTrigger {
    identity: TriggerIdentity,
    phase: u16,
    provider_registration_ordinal: ProviderRegistrationOrdinal,
    depth: u8,
    payload: Value,
    encoded_bytes: usize,
    fingerprint: CanonicalHash,
}

impl PreparedTrigger {
    fn new(
        root_request: RequestUid,
        draft: PreparedTriggerDraft,
        depth: u8,
    ) -> Result<Self, CascadeError> {
        if draft.handler.owner != *draft.provider_registration_ordinal.owner() {
            return Err(CascadeError::ProviderOwnerMismatch {
                provider: draft.provider_registration_ordinal.owner().clone(),
                handler: draft.handler.owner.clone(),
            });
        }
        let identity = TriggerIdentity {
            root_request,
            source_event: draft.source_event,
            handler: draft.handler,
            target: draft.target,
        };
        let canonical = json!({
            "identity": {
                "root_request": identity.root_request,
                "source_event": identity.source_event,
                "handler": identity.handler,
                "target": identity.target,
            },
            "phase": draft.phase,
            "provider_registration_ordinal": draft.provider_registration_ordinal.get(),
            "depth": depth,
            "payload": draft.payload,
        });
        let encoded_bytes = canonical_json_bytes_with_limit(&canonical, MAX_CASCADE_BYTES)
            .map_err(|_| CascadeError::BytesExceeded {
                attempted: MAX_CASCADE_BYTES.saturating_add(1),
                max: MAX_CASCADE_BYTES,
            })?
            .len();
        let fingerprint =
            CanonicalHash::for_canonical_value("trigger_work.v1", 1, &canonical, MAX_CASCADE_BYTES)
                .map_err(|_| CascadeError::BytesExceeded {
                    attempted: MAX_CASCADE_BYTES.saturating_add(1),
                    max: MAX_CASCADE_BYTES,
                })?;
        Ok(Self {
            identity,
            phase: draft.phase,
            provider_registration_ordinal: draft.provider_registration_ordinal,
            depth,
            payload: draft.payload,
            encoded_bytes,
            fingerprint,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> &TriggerIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn phase(&self) -> u16 {
        self.phase
    }

    #[must_use]
    pub const fn provider_registration_ordinal(&self) -> &ProviderRegistrationOrdinal {
        &self.provider_registration_ordinal
    }

    #[must_use]
    pub const fn depth(&self) -> u8 {
        self.depth
    }

    #[must_use]
    pub const fn payload(&self) -> &Value {
        &self.payload
    }

    #[must_use]
    pub const fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }

    fn order_key(&self) -> TriggerOrderKey {
        TriggerOrderKey {
            phase: self.phase,
            provider_registration_ordinal: self.provider_registration_ordinal.get(),
            source_event_ordinal: self.identity.source_event.ordinal(),
            target: self.identity.target.clone(),
            handler: self.identity.handler.clone(),
        }
    }
}

/// One dequeued trigger that remains in the originating queue's custody until
/// it is explicitly completed.
///
/// The token is deliberately non-cloneable and bound to one queue instance.
/// Dropping it does not silently turn the work into success: the queue retains
/// an active-work marker, so another pop and final [`CausalTriggerQueue::finish`]
/// both fail closed.
#[derive(Debug)]
#[must_use = "dequeued trigger work must be completed through its originating causal queue"]
pub struct TriggerWork {
    trigger: PreparedTrigger,
    queue_brand: Arc<TriggerQueueBrand>,
}

impl TriggerWork {
    /// The canonical work item carried by this completion token.
    #[must_use]
    pub const fn trigger(&self) -> &PreparedTrigger {
        &self.trigger
    }
}

#[derive(Debug)]
struct TriggerQueueBrand;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveTrigger {
    identity: TriggerIdentity,
    fingerprint: CanonicalHash,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TriggerOrderKey {
    phase: u16,
    provider_registration_ordinal: u32,
    source_event_ordinal: u32,
    target: SubjectKey,
    // A final stable tie-breaker is required when one provider has several
    // handlers for the same phase/source/target.
    handler: TriggerHandlerId,
}

/// Whether enqueueing added work or suppressed an exact duplicate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriggerEnqueue {
    Queued,
    DuplicateSuppressed,
}

/// Pure deterministic queue and aggregate cascade budget.
#[derive(Debug)]
pub struct CausalTriggerQueue {
    root_request: RequestUid,
    owner_readiness: MechanicalOwnerReadiness,
    queue_brand: Arc<TriggerQueueBrand>,
    pending: BTreeMap<TriggerOrderKey, PreparedTrigger>,
    seen: BTreeMap<TriggerIdentity, CanonicalHash>,
    active: Option<ActiveTrigger>,
    event_depths: Vec<u8>,
    trigger_count: u32,
    operation_count: u32,
    encoded_bytes: usize,
    roots_open: bool,
    failure: Option<CascadeError>,
}

impl CausalTriggerQueue {
    #[must_use]
    pub(crate) fn new(root_request: RequestUid, owner_readiness: MechanicalOwnerReadiness) -> Self {
        Self {
            root_request,
            owner_readiness,
            queue_brand: Arc::new(TriggerQueueBrand),
            pending: BTreeMap::new(),
            seen: BTreeMap::new(),
            active: None,
            event_depths: Vec::new(),
            trigger_count: 0,
            operation_count: 0,
            encoded_bytes: 0,
            roots_open: true,
            failure: None,
        }
    }

    /// Allocate an initial semantic event before trigger expansion begins.
    /// Once the first trigger is popped, every further event must be derived
    /// from that queue's active trigger work.
    pub fn allocate_root_event(&mut self) -> Result<PreparedCausalRef, CascadeError> {
        self.run_checked(|queue| {
            if !queue.roots_open {
                return Err(CascadeError::RootEventPhaseClosed);
            }
            queue.allocate_event_inner(None, 0)
        })
    }

    /// Allocate an event caused by this exact queue's active trigger work.
    /// The causal parent and depth come from `source`; callers cannot reset a
    /// recursive chain by supplying `None`, completed work, or an unrelated
    /// shallow event.
    pub fn allocate_derived_event(
        &mut self,
        source: &TriggerWork,
    ) -> Result<PreparedCausalRef, CascadeError> {
        self.run_checked(|queue| {
            queue.roots_open = false;
            if !Arc::ptr_eq(&source.queue_brand, &queue.queue_brand) {
                return Err(CascadeError::ForeignTriggerWork);
            }
            let trigger = source.trigger();
            let Some(active) = &queue.active else {
                return Err(CascadeError::TriggerNotActive {
                    identity: Box::new(trigger.identity.clone()),
                });
            };
            if active.identity != trigger.identity || active.fingerprint != trigger.fingerprint {
                return Err(CascadeError::TriggerNotActive {
                    identity: Box::new(trigger.identity.clone()),
                });
            }
            queue
                .owner_readiness
                .require_ready(trigger.provider_registration_ordinal.owner())
                .map_err(|source_error| CascadeError::ProviderNotReady {
                    owner: trigger.provider_registration_ordinal.owner().clone(),
                    source: Box::new(source_error),
                })?;
            queue.allocate_event_inner(Some(trigger.identity.source_event), trigger.depth)
        })
    }

    fn allocate_event_inner(
        &mut self,
        parent: Option<PreparedEventId>,
        depth: u8,
    ) -> Result<PreparedCausalRef, CascadeError> {
        let next_event_ordinal =
            u32::try_from(self.event_depths.len()).expect("semantic-event hard limit fits u32");
        if next_event_ordinal >= MAX_SEMANTIC_EVENTS {
            return Err(CascadeError::EventLimit {
                max: MAX_SEMANTIC_EVENTS,
            });
        }
        if let Some(parent) = parent {
            if parent.request() != self.root_request {
                return Err(CascadeError::RootMismatch);
            }
            let parent_depth = self
                .event_depths
                .get(parent.ordinal() as usize)
                .copied()
                .ok_or(CascadeError::UnallocatedEvent {
                    ordinal: parent.ordinal(),
                })?;
            let expected_depth =
                parent_depth
                    .checked_add(1)
                    .ok_or(CascadeError::DepthExceeded {
                        actual: u8::MAX,
                        max: MAX_TRIGGER_DEPTH,
                    })?;
            if expected_depth != depth {
                return Err(CascadeError::InvalidDerivedDepth {
                    actual: depth,
                    expected: expected_depth,
                });
            }
        } else if depth != 0 {
            return Err(CascadeError::InvalidDerivedDepth {
                actual: depth,
                expected: 0,
            });
        }
        if depth > MAX_TRIGGER_DEPTH {
            return Err(CascadeError::DepthExceeded {
                actual: depth,
                max: MAX_TRIGGER_DEPTH,
            });
        }
        let causal = PreparedCausalRef::new(self.root_request, parent, next_event_ordinal)?;
        self.event_depths.push(depth);
        Ok(causal)
    }

    /// Queue work once under its complete duplicate key. An exact replay is
    /// suppressed without consuming budget; conflicting bytes under the same
    /// identity abort rather than making insertion order significant.
    pub fn enqueue(&mut self, draft: PreparedTriggerDraft) -> Result<TriggerEnqueue, CascadeError> {
        self.run_checked(|queue| queue.enqueue_inner(draft))
    }

    fn enqueue_inner(
        &mut self,
        draft: PreparedTriggerDraft,
    ) -> Result<TriggerEnqueue, CascadeError> {
        if !self
            .owner_readiness
            .shares_latch_with(&draft.provider_registration_ordinal.owner_readiness)
        {
            return Err(CascadeError::RegistryCompositionMismatch);
        }
        self.owner_readiness
            .require_ready(draft.provider_registration_ordinal.owner())
            .map_err(|source| CascadeError::ProviderNotReady {
                owner: draft.provider_registration_ordinal.owner().clone(),
                source: Box::new(source),
            })?;
        if draft.source_event.request() != self.root_request {
            return Err(CascadeError::RootMismatch);
        }
        let source_depth = self
            .event_depths
            .get(draft.source_event.ordinal() as usize)
            .copied()
            .ok_or(CascadeError::UnallocatedEvent {
                ordinal: draft.source_event.ordinal(),
            })?;
        let depth = source_depth
            .checked_add(1)
            .ok_or(CascadeError::DepthExceeded {
                actual: u8::MAX,
                max: MAX_TRIGGER_DEPTH,
            })?;
        if depth > MAX_TRIGGER_DEPTH {
            return Err(CascadeError::DepthExceeded {
                actual: depth,
                max: MAX_TRIGGER_DEPTH,
            });
        }
        let trigger = PreparedTrigger::new(self.root_request, draft, depth)?;
        if let Some(fingerprint) = self.seen.get(&trigger.identity) {
            return if *fingerprint == trigger.fingerprint {
                Ok(TriggerEnqueue::DuplicateSuppressed)
            } else {
                Err(CascadeError::ConflictingDuplicate {
                    identity: Box::new(trigger.identity),
                })
            };
        }
        if self.trigger_count >= MAX_CASCADE_TRIGGERS {
            return Err(CascadeError::TriggerLimit {
                max: MAX_CASCADE_TRIGGERS,
            });
        }
        let new_bytes = self
            .encoded_bytes
            .checked_add(trigger.encoded_bytes)
            .ok_or(CascadeError::BytesExceeded {
                attempted: usize::MAX,
                max: MAX_CASCADE_BYTES,
            })?;
        if new_bytes > MAX_CASCADE_BYTES {
            return Err(CascadeError::BytesExceeded {
                attempted: new_bytes,
                max: MAX_CASCADE_BYTES,
            });
        }
        let order = trigger.order_key();
        let identity = trigger.identity.clone();
        let fingerprint = trigger.fingerprint;
        if self.pending.contains_key(&order) {
            return Err(CascadeError::OrderingCollision);
        }
        self.pending.insert(order, trigger);
        self.seen.insert(identity, fingerprint);
        self.trigger_count += 1;
        self.encoded_bytes = new_bytes;
        Ok(TriggerEnqueue::Queued)
    }

    /// Charge typed post-state operations before materialization.
    pub fn charge_operations(&mut self, count: u32) -> Result<(), CascadeError> {
        self.run_checked(|queue| queue.charge_operations_inner(count))
    }

    fn charge_operations_inner(&mut self, count: u32) -> Result<(), CascadeError> {
        let total =
            self.operation_count
                .checked_add(count)
                .ok_or(CascadeError::OperationLimit {
                    attempted: u32::MAX,
                    max: MAX_CASCADE_OPERATIONS,
                })?;
        if total > MAX_CASCADE_OPERATIONS {
            return Err(CascadeError::OperationLimit {
                attempted: total,
                max: MAX_CASCADE_OPERATIONS,
            });
        }
        self.operation_count = total;
        Ok(())
    }

    /// Charge canonical plan/event bytes not already represented by queued
    /// trigger payloads.
    pub fn charge_bytes(&mut self, count: usize) -> Result<(), CascadeError> {
        self.run_checked(|queue| queue.charge_bytes_inner(count))
    }

    fn charge_bytes_inner(&mut self, count: usize) -> Result<(), CascadeError> {
        let total = self
            .encoded_bytes
            .checked_add(count)
            .ok_or(CascadeError::BytesExceeded {
                attempted: usize::MAX,
                max: MAX_CASCADE_BYTES,
            })?;
        if total > MAX_CASCADE_BYTES {
            return Err(CascadeError::BytesExceeded {
                attempted: total,
                max: MAX_CASCADE_BYTES,
            });
        }
        self.encoded_bytes = total;
        Ok(())
    }

    /// Dequeue the next deterministic work item while retaining queue custody.
    /// Exactly one [`TriggerWork`] may be active at a time, and it must be
    /// returned through [`Self::complete_trigger`] before another item can be
    /// popped or the cascade can finish successfully.
    pub fn pop_next(&mut self) -> Result<Option<TriggerWork>, CascadeError> {
        self.run_checked(|queue| {
            queue.roots_open = false;
            if let Some(active) = &queue.active {
                return Err(CascadeError::TriggerAlreadyActive {
                    identity: Box::new(active.identity.clone()),
                });
            }
            let Some((_, next)) = queue.pending.first_key_value() else {
                return Ok(None);
            };
            queue
                .owner_readiness
                .require_ready(next.provider_registration_ordinal.owner())
                .map_err(|source| CascadeError::ProviderNotReady {
                    owner: next.provider_registration_ordinal.owner().clone(),
                    source: Box::new(source),
                })?;
            let (_, trigger) = queue
                .pending
                .pop_first()
                .expect("first_key_value established pending work");
            queue.active = Some(ActiveTrigger {
                identity: trigger.identity.clone(),
                fingerprint: trigger.fingerprint,
            });
            Ok(Some(TriggerWork {
                trigger,
                queue_brand: Arc::clone(&queue.queue_brand),
            }))
        })
    }

    /// Mark one dequeued trigger complete after its handler has finished.
    ///
    /// Consuming the queue-bound token prevents accidental double completion.
    /// Owner readiness is rechecked here so a freeze racing handler execution
    /// wins over apparent success.
    pub fn complete_trigger(&mut self, work: TriggerWork) -> Result<(), CascadeError> {
        self.run_checked(|queue| {
            if !Arc::ptr_eq(&work.queue_brand, &queue.queue_brand) {
                return Err(CascadeError::ForeignTriggerWork);
            }
            let trigger = work.trigger;
            let Some(active) = &queue.active else {
                return Err(CascadeError::TriggerNotActive {
                    identity: Box::new(trigger.identity),
                });
            };
            if active.identity != trigger.identity || active.fingerprint != trigger.fingerprint {
                return Err(CascadeError::TriggerNotActive {
                    identity: Box::new(trigger.identity),
                });
            }
            queue
                .owner_readiness
                .require_ready(trigger.provider_registration_ordinal.owner())
                .map_err(|source| CascadeError::ProviderNotReady {
                    owner: trigger.provider_registration_ordinal.owner().clone(),
                    source: Box::new(source),
                })?;
            queue.active = None;
            Ok(())
        })
    }

    /// Consume the queue only after every admitted trigger was processed and
    /// no earlier error was ignored. This is the hand-off future ActionTxn
    /// materialization must require before it can commit cascade products.
    pub fn finish(self) -> Result<(), CascadeError> {
        if let Some(cause) = self.failure {
            return Err(CascadeError::CascadePoisoned {
                cause: Box::new(cause),
            });
        }
        if let Some(active) = self.active {
            return Err(CascadeError::UncompletedTrigger {
                identity: Box::new(active.identity),
            });
        }
        if !self.pending.is_empty() {
            return Err(CascadeError::PendingTriggers {
                count: self.pending.len(),
            });
        }
        Ok(())
    }

    fn run_checked<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, CascadeError>,
    ) -> Result<T, CascadeError> {
        if let Some(cause) = &self.failure {
            return Err(CascadeError::CascadePoisoned {
                cause: Box::new(cause.clone()),
            });
        }
        match operation(self) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.failure = Some(error.clone());
                Err(error)
            }
        }
    }

    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn event_count(&self) -> u32 {
        u32::try_from(self.event_depths.len()).expect("semantic-event hard limit fits u32")
    }

    #[must_use]
    pub const fn trigger_count(&self) -> u32 {
        self.trigger_count
    }

    #[must_use]
    pub const fn operation_count(&self) -> u32 {
        self.operation_count
    }

    #[must_use]
    pub const fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}

/// A deterministic cascade exceeded a bound or contradicted its identity.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CascadeError {
    #[error("trigger queue is not bound to a composed mechanics registry")]
    RegistryUnavailable,
    #[error("trigger provider does not belong to this queue's module composition")]
    RegistryCompositionMismatch,
    #[error("trigger provider {owner} is mechanically unavailable: {source}")]
    ProviderNotReady {
        owner: OwnerId,
        source: Box<MechanicalOwnerReadinessError>,
    },
    #[error("initial semantic events cannot be allocated after trigger expansion begins")]
    RootEventPhaseClosed,
    #[error("trigger work token belongs to a different causal queue")]
    ForeignTriggerWork,
    #[error("causal queue already has trigger work awaiting completion: {identity:?}")]
    TriggerAlreadyActive { identity: Box<TriggerIdentity> },
    #[error("trigger work is not the causal queue's active item: {identity:?}")]
    TriggerNotActive { identity: Box<TriggerIdentity> },
    #[error("derived semantic event depth {actual} does not match required depth {expected}")]
    InvalidDerivedDepth { actual: u8, expected: u8 },
    #[error("trigger/event belongs to a different root request")]
    RootMismatch,
    #[error("trigger depth {actual} exceeds the hard maximum {max}")]
    DepthExceeded { actual: u8, max: u8 },
    #[error("cascade exceeds its hard trigger limit of {max}")]
    TriggerLimit { max: u32 },
    #[error("cascade exceeds its hard semantic-event limit of {max}")]
    EventLimit { max: u32 },
    #[error("semantic event ordinal {ordinal} was not allocated by this cascade")]
    UnallocatedEvent { ordinal: u32 },
    #[error("provider owner {provider} cannot order trigger handler owned by {handler}")]
    ProviderOwnerMismatch { provider: OwnerId, handler: OwnerId },
    #[error("cascade operation count {attempted} exceeds the hard maximum {max}")]
    OperationLimit { attempted: u32, max: u32 },
    #[error("cascade bytes {attempted} exceed the hard maximum {max}")]
    BytesExceeded { attempted: usize, max: usize },
    #[error("same trigger identity was supplied with conflicting canonical bytes: {identity:?}")]
    ConflictingDuplicate { identity: Box<TriggerIdentity> },
    #[error("distinct trigger identities produced the same total ordering key")]
    OrderingCollision,
    #[error("cascade still has {count} pending triggers")]
    PendingTriggers { count: usize },
    #[error("cascade trigger work was not completed: {identity:?}")]
    UncompletedTrigger { identity: Box<TriggerIdentity> },
    #[error("cascade was already poisoned by an earlier failure: {cause}")]
    CascadePoisoned { cause: Box<CascadeError> },
    #[error("invalid prepared causality: {0}")]
    Causal(#[from] CausalMetadataError),
}

/// Object-safe, read-only provider seam admitted during module composition.
///
/// Validation may inspect provider-owned parameter semantics, but it cannot
/// return mutations, authority guards, or a success receipt. Typed mutation
/// is available only through the owner-scoped transaction passed to
/// [`ActionProvider::prepare_invocation`].
pub trait ActionProvider: Send + Sync {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError>;

    /// Project this exact registered action for one server-authenticated
    /// viewer/subject query. The default means that this action is not exposed
    /// through legal-action discovery. Returned data remains internal until
    /// the registry stamps the exact action and signs it.
    fn project_offer(
        &self,
        _action: &DefinitionRef,
        _query: &crate::offer::LegalActionQuery,
        _engine: &crate::engine::Engine,
    ) -> Result<Option<crate::offer::ProviderOfferDraft>, ActionError> {
        Ok(None)
    }

    /// Name provider-owned prompt-time revision coordinates for one projected
    /// delayed-decision offer. The provider supplies keys only; the kernel
    /// reads current generations, proves cross-owner bridges, canonicalizes
    /// ordering, and enforces the delayed-decision bounds. Providers that do
    /// not override this hook declare no extra witnesses.
    fn delayed_decision_witness_keys(
        &self,
        _action: &DefinitionRef,
        _query: &crate::offer::LegalActionQuery,
        _offer: &crate::offer::ProjectedLegalAction,
        _engine: &crate::engine::Engine,
    ) -> Result<crate::decision::DecisionWitnessKeys, ActionError> {
        Ok(crate::decision::DecisionWitnessKeys::default())
    }

    /// Projection-only rank used when an async controller must select at most
    /// 32 offers from the full legal set. Larger values sort first; canonical
    /// action/offer identity is the stable tie-break. Hidden truth must not
    /// influence this callback.
    fn delayed_decision_rank(
        &self,
        _action: &DefinitionRef,
        _query: &crate::offer::LegalActionQuery,
        _offer: &crate::offer::ProjectedLegalAction,
    ) -> Result<i32, ActionError> {
        Ok(0)
    }

    /// Project a durable root outcome for one event-time eligible viewer.
    ///
    /// This callback is deliberately pure: it receives only the exact frozen
    /// action, immutable durable outcome, and authenticated viewer. It cannot
    /// consult current world state, so replay/status projection remains stable
    /// after restart. Returning `None` yields a generic committed receipt.
    fn project_receipt(
        &self,
        _action: &DefinitionRef,
        _outcome: &RulesValue,
        _viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        Ok(None)
    }

    /// Prepare bounded owner-scoped post-images without access to live mutable
    /// world or module state. Existing validation-only providers inherit a
    /// fail-closed empty transaction; only providers that add state mutations
    /// can produce a [`PreparedTxn`].
    fn prepare_invocation(
        &self,
        invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        self.validate_invocation(invocation)
    }

    /// Prepare the semantic result material that surrounds the exact typed
    /// post-images in an authoritative bundle. Providers opt into the K3 lane
    /// explicitly; the default preserves the validation/K2 compatibility
    /// surface while refusing to invent an outcome for it.
    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "this action provider has not migrated to authoritative execution",
        )
        .expect("static provider migration error is bounded"))
    }

    /// Prepare authoritative output with a kernel-owned, one-action
    /// capability for deterministic resolution. Existing non-random
    /// providers remain source-compatible through the default delegation;
    /// providers that need randomness override this method and obtain their
    /// [`ResolutionRng`](crate::resolution::ResolutionRng) only from `context`.
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        _context: &mut crate::resolution::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare_authoritative(invocation, txn)
    }

    /// Prepare an invocation whose projected subject/targets have been
    /// privately resolved by the engine immediately before provider entry.
    ///
    /// Existing providers remain source-compatible and receive the durable
    /// projected invocation. Providers that act on opaque contacts override
    /// this method and use only `truth_subject`/`truth_targets` for mechanical
    /// addressing; those identities never enter the wire invocation.
    fn prepare_resolved_authoritative_with_context(
        &self,
        resolved: &ResolvedActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut crate::resolution::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare_authoritative_with_context(resolved.invocation(), txn, context)
    }

    /// Additional authority selected by this resolved invocation. For example,
    /// a crew member may act through a platform-owned contact audience. Only
    /// the trusted provider chooses that principal; the kernel issues each
    /// witness and retains its guard for the final pre-commit recheck.
    fn resolved_authority_witnesses(
        &self,
        _resolved: &ResolvedActionInvocation,
        _engine: &crate::engine::Engine,
    ) -> Result<Vec<AuthorityWitnessRequest>, ActionError> {
        Ok(Vec::new())
    }

    /// Name exact provider-owned revision coordinates that become known only
    /// after opaque subject/target resolution. The kernel reads and stamps
    /// their generations; providers never supply the observed values.
    fn resolved_revision_keys(
        &self,
        _resolved: &ResolvedActionInvocation,
    ) -> Result<Vec<RevisionKey>, ActionError> {
        Ok(Vec::new())
    }

    /// Expand one provider-owned same-commit trigger against the root
    /// action's still-unprepared scratch transaction. Trigger callbacks have
    /// the same mutation authority as the root provider, but no persistence,
    /// live-world, or asynchronous side-effect access.
    fn prepare_same_commit_trigger(
        &self,
        _invocation: &ActionInvocation,
        _trigger: &PreparedTrigger,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "this action provider does not implement the requested same-commit trigger",
        )
        .expect("static trigger migration error is bounded"))
    }
}

/// Provider-requested work that must finish in the root action's transaction.
///
/// The frozen registry stamps provider identity and ordering when this is
/// admitted to [`CausalTriggerQueue`]; providers can choose only their own
/// handler id, target, phase, and bounded payload.
#[derive(Clone, Debug, PartialEq)]
pub struct SameCommitTrigger {
    handler: CanonicalId,
    target: SubjectKey,
    phase: u16,
    payload: Value,
}

impl SameCommitTrigger {
    #[must_use]
    pub const fn new(handler: CanonicalId, target: SubjectKey, phase: u16, payload: Value) -> Self {
        Self {
            handler,
            target,
            phase,
            payload,
        }
    }

    fn into_prepared(
        self,
        provider: &ActionProviderEntry,
        source_event: PreparedEventId,
    ) -> PreparedTriggerDraft {
        PreparedTriggerDraft::new(
            provider,
            source_event,
            self.handler,
            self.target,
            self.phase,
            self.payload,
        )
    }
}

/// Provider-owned semantic material for one K3 action. The central
/// coordinator supplies request identity, causality, token, plan hash and the
/// exact replay writes; providers cannot override those fields.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoritativeActionOutput {
    outcome: RulesValue,
    resolution: Option<PreparedResolution>,
    definitions: Vec<DefinitionRef>,
    eligible_witnesses: Vec<PrincipalId>,
    durable_outbox: Vec<DurableOutboxIntent>,
    same_commit_triggers: Vec<SameCommitTrigger>,
}

impl AuthoritativeActionOutput {
    #[must_use]
    pub fn new(outcome: RulesValue) -> Self {
        Self {
            outcome,
            resolution: None,
            definitions: Vec::new(),
            eligible_witnesses: Vec::new(),
            durable_outbox: Vec::new(),
            same_commit_triggers: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_definitions(mut self, definitions: Vec<DefinitionRef>) -> Self {
        self.definitions = definitions;
        self
    }

    #[must_use]
    pub fn with_resolution(mut self, resolution: PreparedResolution) -> Self {
        self.resolution = Some(resolution);
        self
    }

    #[must_use]
    pub fn with_eligible_witnesses(mut self, witnesses: Vec<PrincipalId>) -> Self {
        self.eligible_witnesses = witnesses;
        self
    }

    /// Post-finalization delivery intents this one emission wants.
    ///
    /// The ordinals a provider stamps are an **intra-group order hint**: they
    /// order this group against itself and mean nothing across groups. The
    /// coordinator concatenates the root's group ahead of each same-commit
    /// trigger's group in cascade order and renumbers the result `0..n`, so a
    /// trigger may reuse the root's numbers without disturbing it. Two intents
    /// sharing an ordinal inside one group are refused
    /// ([`ActionProviderPrepareError::OutboxOrdinalConflict`]), because there
    /// the provider really did fail to say which comes first.
    #[must_use]
    pub fn with_durable_outbox(mut self, outbox: Vec<DurableOutboxIntent>) -> Self {
        self.durable_outbox = outbox;
        self
    }

    #[must_use]
    pub fn with_same_commit_triggers(mut self, triggers: Vec<SameCommitTrigger>) -> Self {
        self.same_commit_triggers = triggers;
        self
    }

    #[must_use]
    pub const fn outcome(&self) -> &RulesValue {
        &self.outcome
    }

    #[must_use]
    pub const fn resolution(&self) -> Option<&PreparedResolution> {
        self.resolution.as_ref()
    }

    #[must_use]
    pub fn definitions(&self) -> &[DefinitionRef] {
        &self.definitions
    }

    #[must_use]
    pub fn eligible_witnesses(&self) -> &[PrincipalId] {
        &self.eligible_witnesses
    }

    #[must_use]
    pub fn durable_outbox(&self) -> &[DurableOutboxIntent] {
        &self.durable_outbox
    }

    fn into_cascade_parts(self) -> AuthoritativeCascadeParts {
        (
            self.outcome,
            self.resolution,
            self.definitions,
            self.eligible_witnesses,
            self.durable_outbox,
            self.same_commit_triggers,
        )
    }
}

type AuthoritativeCascadeParts = (
    RulesValue,
    Option<PreparedResolution>,
    Vec<DefinitionRef>,
    Vec<PrincipalId>,
    Vec<DurableOutboxIntent>,
    Vec<SameCommitTrigger>,
);

/// Complete semantic products of a fully expanded authoritative action.
/// The transaction remains separate and one-shot so the server can bind the
/// same immutable event graph and post-images to one durable bundle.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedAuthoritativeAction {
    outcome: RulesValue,
    resolution: Option<PreparedResolution>,
    definitions: Vec<DefinitionRef>,
    eligible_witnesses: Vec<PrincipalId>,
    durable_outbox: Vec<DurableOutboxIntent>,
    semantic_events: Vec<AuthoritativeSemanticEvent>,
}

impl PreparedAuthoritativeAction {
    /// The durable outcome this preparation produced.
    ///
    /// E7c2 fold B12: the staging path needs the provider's own receipt line for
    /// the acting principal at stage time, so a first commit and its replay read
    /// the same rather than the first one rendering a raw commit token.
    #[must_use]
    pub const fn outcome(&self) -> &RulesValue {
        &self.outcome
    }

    pub fn into_parts(self) -> PreparedAuthoritativeActionParts {
        (
            self.outcome,
            self.resolution,
            self.definitions,
            self.eligible_witnesses,
            self.durable_outbox,
            self.semantic_events,
        )
    }
}

/// Owned components of a prepared authoritative action.
pub type PreparedAuthoritativeActionParts = (
    RulesValue,
    Option<PreparedResolution>,
    Vec<DefinitionRef>,
    Vec<PrincipalId>,
    Vec<DurableOutboxIntent>,
    Vec<AuthoritativeSemanticEvent>,
);

/// Frozen deterministic provider ordering assigned from canonical registry order.
/// Callers cannot construct or choose this value.
#[derive(Clone)]
pub struct ProviderRegistrationOrdinal {
    owner: OwnerId,
    ordinal: u32,
    owner_readiness: MechanicalOwnerReadiness,
}

impl fmt::Debug for ProviderRegistrationOrdinal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderRegistrationOrdinal")
            .field("owner", &self.owner)
            .field("ordinal", &self.ordinal)
            .finish_non_exhaustive()
    }
}

impl PartialEq for ProviderRegistrationOrdinal {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.ordinal == other.ordinal
            && self
                .owner_readiness
                .shares_latch_with(&other.owner_readiness)
    }
}

impl Eq for ProviderRegistrationOrdinal {}

impl ProviderRegistrationOrdinal {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn get(&self) -> u32 {
        self.ordinal
    }

    pub(crate) const fn owner_readiness(&self) -> &MechanicalOwnerReadiness {
        &self.owner_readiness
    }
}

/// Immutable provider metadata; owner identity is stamped by
/// [`Registrar`](crate::Registrar).
///
/// The executable callback is intentionally not exposed. All invocation must
/// go through [`ActionProviderRegistry::validate_invocation`], which checks
/// owner readiness both before and after provider code runs.
///
/// ```compile_fail
/// fn bypass(entry: &mush_core::ActionProviderEntry) {
///     let _ = entry.provider();
/// }
/// ```
pub struct ActionProviderEntry {
    owner: OwnerId,
    action: DefinitionRef,
    provider: Arc<dyn ActionProvider>,
    registration_ordinal: Option<ProviderRegistrationOrdinal>,
}

impl fmt::Debug for ActionProviderEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionProviderEntry")
            .field("owner", &self.owner)
            .field("action", &self.action)
            .field("registration_ordinal", &self.registration_ordinal)
            .finish_non_exhaustive()
    }
}

impl ActionProviderEntry {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }

    #[must_use]
    fn provider(&self) -> &dyn ActionProvider {
        self.provider.as_ref()
    }

    #[must_use]
    pub fn registration_ordinal(&self) -> &ProviderRegistrationOrdinal {
        self.registration_ordinal
            .as_ref()
            .expect("frozen action-provider entry has an ordinal")
    }
}

/// Mutable action-provider composition table.
#[derive(Default)]
pub struct ActionProviderRegistryBuilder {
    entries: BTreeMap<crate::mechanics::DefinitionLocator, ActionProviderEntry>,
    owner_counts: BTreeMap<OwnerId, usize>,
}

impl fmt::Debug for ActionProviderRegistryBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionProviderRegistryBuilder")
            .field("actions", &self.entries.keys().collect::<Vec<_>>())
            .field("owner_counts", &self.owner_counts)
            .finish()
    }
}

impl ActionProviderRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register(
        &mut self,
        stamped_owner: OwnerId,
        action: DefinitionRef,
        provider: Arc<dyn ActionProvider>,
    ) -> Result<(), ActionProviderRegistryError> {
        if !is_action_owned_by(&stamped_owner, &action) {
            return Err(ActionProviderRegistryError::WrongActionOwner {
                provider: stamped_owner,
                action: Box::new(action),
            });
        }
        if self.entries.len() >= MAX_ACTION_PROVIDERS {
            return Err(ActionProviderRegistryError::ProviderLimit {
                max: MAX_ACTION_PROVIDERS,
            });
        }
        let owner_count = self.owner_counts.get(&stamped_owner).copied().unwrap_or(0);
        if owner_count >= MAX_ACTION_PROVIDERS_PER_OWNER {
            return Err(ActionProviderRegistryError::OwnerProviderLimit {
                owner: stamped_owner,
                max: MAX_ACTION_PROVIDERS_PER_OWNER,
            });
        }
        let locator = action.locator();
        if let Some(existing) = self.entries.get(&locator) {
            return if existing.action.digest() == action.digest() {
                Err(ActionProviderRegistryError::Duplicate {
                    action: Box::new(action),
                })
            } else {
                Err(ActionProviderRegistryError::DigestConflict {
                    attempted: Box::new(action),
                    registered: Box::new(existing.action.clone()),
                })
            };
        }
        self.entries.insert(
            locator,
            ActionProviderEntry {
                owner: stamped_owner.clone(),
                action,
                provider,
                // Reassigned from canonical key order at freeze.
                registration_ordinal: None,
            },
        );
        self.owner_counts.insert(stamped_owner, owner_count + 1);
        Ok(())
    }

    pub(crate) fn freeze_for_owners_with_readiness(
        self,
        definitions: &DefinitionRegistry,
        module_owners: &BTreeSet<OwnerId>,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<ActionProviderRegistry, ActionProviderRegistryError> {
        let definition_readiness = definitions
            .owner_readiness()
            .ok_or(ActionProviderRegistryError::RegistryCompositionMismatch)?;
        if !owner_readiness.shares_latch_with(definition_readiness) {
            return Err(ActionProviderRegistryError::RegistryCompositionMismatch);
        }
        let mut entries = self.entries;
        for entry in entries.values() {
            if !module_owners.contains(&entry.owner) {
                return Err(ActionProviderRegistryError::UnknownOwner {
                    owner: entry.owner.clone(),
                });
            }
            definitions.get_exact(&entry.action).map_err(|source| {
                ActionProviderRegistryError::MissingActionDefinition {
                    action: Box::new(entry.action.clone()),
                    source: Box::new(source),
                }
            })?;
        }
        for (ordinal, entry) in entries.values_mut().enumerate() {
            entry.registration_ordinal = Some(ProviderRegistrationOrdinal {
                owner: entry.owner.clone(),
                ordinal: u32::try_from(ordinal).expect("action provider hard limit fits u32"),
                owner_readiness: owner_readiness.clone(),
            });
        }
        let mut offer_routes = BTreeMap::new();
        for (locator, entry) in &entries {
            let route = crate::offer::OfferRouteId::derive(&entry.action)
                .map_err(ActionProviderRegistryError::OfferRouteHash)?;
            if let Some(existing) = offer_routes.insert(route, locator.clone()) {
                return Err(ActionProviderRegistryError::OfferRouteCollision {
                    first: Box::new(existing),
                    second: Box::new(locator.clone()),
                });
            }
        }
        let fingerprint = provider_fingerprint(entries.values())?;
        Ok(ActionProviderRegistry {
            entries,
            offer_routes,
            fingerprint,
            owner_readiness,
        })
    }
}

/// Immutable exact-action provider lookup.
pub struct ActionProviderRegistry {
    entries: BTreeMap<crate::mechanics::DefinitionLocator, ActionProviderEntry>,
    offer_routes: BTreeMap<crate::offer::OfferRouteId, crate::mechanics::DefinitionLocator>,
    fingerprint: DefinitionDigest,
    owner_readiness: MechanicalOwnerReadiness,
}

impl fmt::Debug for ActionProviderRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionProviderRegistry")
            .field("entries", &self.entries)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

/// Select the one exact station that supplies every channel required by an
/// offer. Higher priority wins, but an equal-priority tie is authority
/// ambiguity and projects no action. In particular, channels can never be
/// assembled across two grants.
fn select_offer_channel_grant<'a>(
    world: &'a World,
    query: &crate::offer::LegalActionQuery,
    required_channels: &[ChannelId],
) -> Option<(OwnerId, &'a crate::world::ChannelGrant)> {
    if required_channels.is_empty() {
        return None;
    }
    let SubjectKey::Object { id: subject } = query.truth_subject() else {
        return None;
    };
    let mut eligible = world
        .command_subjects_of(query.actor())
        .iter()
        .filter(|grant| {
            grant.subject == *subject
                && required_channels
                    .iter()
                    .all(|channel| grant.channels.contains(channel))
        });
    let best = eligible.next()?;
    if eligible.any(|candidate| candidate.priority == best.priority) {
        return None;
    }
    let publisher = OwnerId::parse_canonical(best.owner.as_str()).ok()?;
    Some((publisher, best))
}

impl ActionProviderRegistry {
    pub fn get_exact(
        &self,
        action: &DefinitionRef,
    ) -> Result<&ActionProviderEntry, ActionProviderLookupError> {
        let Some(entry) = self.entries.get(&action.locator()) else {
            return Err(ActionProviderLookupError::Missing {
                requested: Box::new(action.clone()),
            });
        };
        if entry.action.digest() != action.digest() {
            return Err(ActionProviderLookupError::DigestMismatch {
                requested: Box::new(action.clone()),
                registered: Box::new(entry.action.clone()),
            });
        }
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| ActionProviderLookupError::OwnerNotReady {
                requested: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        Ok(entry)
    }

    /// Route-only immutable snapshot used for early idempotency scope
    /// selection before the world task regenerates and verifies an offer.
    #[must_use]
    pub fn offer_route_table(&self) -> crate::offer::OfferRouteTable {
        crate::offer::OfferRouteTable::from_entries(self.offer_routes.iter().filter_map(
            |(route, locator)| {
                self.entries
                    .get(locator)
                    .map(|entry| (*route, entry.action.clone()))
            },
        ))
    }

    /// Clone only the pure receipt-projection capabilities needed outside the
    /// single-writer. Provider callbacks receive no engine or mutable state;
    /// the shared readiness latch still freezes them with their owner.
    #[must_use]
    pub fn receipt_projection_table(&self) -> ReceiptProjectionTable {
        ReceiptProjectionTable {
            entries: self
                .entries
                .iter()
                .map(|(locator, entry)| {
                    (
                        locator.clone(),
                        ReceiptProjectionEntry {
                            owner: entry.owner.clone(),
                            action: entry.action.clone(),
                            provider: Arc::clone(&entry.provider),
                        },
                    )
                })
                .collect(),
            owner_readiness: self.owner_readiness.clone(),
        }
    }

    /// Enumerate providers in canonical action order and stamp every returned
    /// draft with the exact frozen action identity. The callback runs against
    /// immutable engine truth and owner readiness is checked again after it
    /// returns.
    pub fn project_legal_actions(
        &self,
        engine: &crate::engine::Engine,
        query: &crate::offer::LegalActionQuery,
        keys: &crate::offer::OfferKeyRing,
    ) -> Result<crate::offer::ProjectedLegalActionSink, crate::offer::LegalActionProjectionError>
    {
        let mut sink = crate::offer::ProjectedLegalActionSink::new(query.viewer_scope());
        for entry in self.entries.values() {
            let projected = entry
                .provider()
                .project_offer(entry.action(), query, engine)
                .map_err(
                    |source| crate::offer::LegalActionProjectionError::Provider {
                        action: Box::new(entry.action().clone()),
                        source: Box::new(source),
                    },
                )?;
            self.owner_readiness
                .require_ready(entry.owner())
                .map_err(
                    |_| crate::offer::LegalActionProjectionError::ProviderUnavailable {
                        action: Box::new(entry.action().clone()),
                    },
                )?;
            let Some(mut projected) = projected else {
                continue;
            };
            projected.guards = projected
                .guards
                .with_authority_guards(query.authority_guards())
                .map_err(
                    |source| crate::offer::LegalActionProjectionError::SubjectAuthority {
                        action: Box::new(entry.action().clone()),
                        source: Box::new(source),
                    },
                )?;
            if !projected.required_channels.is_empty() {
                let Some((grant_owner, grant)) =
                    select_offer_channel_grant(&engine.world, query, &projected.required_channels)
                else {
                    continue;
                };
                projected.guards = projected
                    .guards
                    .with_channel_grant(&grant_owner, grant, &projected.required_channels)
                    .map_err(
                        |source| crate::offer::LegalActionProjectionError::ChannelGrant {
                            action: Box::new(entry.action().clone()),
                            source: Box::new(source),
                        },
                    )?;
                let validators = engine
                    .registry
                    .authority_guard_validators()
                    .ok_or_else(|| {
                        crate::offer::LegalActionProjectionError::AuthorityRegistryUnavailable {
                            action: Box::new(entry.action().clone()),
                        }
                    })?;
                let context = AuthorityGuardContext::new(&engine.world, engine.action_state());
                let mut lease_guards = Vec::with_capacity(projected.required_channels.len());
                let mut all_leased = true;
                for channel in &projected.required_channels {
                    let request = AuthorityWitnessRequest::ControlLease {
                        provider: grant_owner.clone(),
                        subject: grant.subject,
                        channel: channel.clone(),
                        holder: query.principal().clone(),
                    };
                    match validators.witness(&context, &request) {
                        Ok(Some(witness)) => lease_guards.push(witness.guard().clone()),
                        Ok(None) => {
                            all_leased = false;
                            break;
                        }
                        Err(source) => {
                            return Err(
                                crate::offer::LegalActionProjectionError::ControlLeaseAuthority {
                                    action: Box::new(entry.action().clone()),
                                    source: Box::new(source),
                                },
                            );
                        }
                    }
                }
                if !all_leased {
                    continue;
                }
                projected.guards = projected
                    .guards
                    .with_authority_guards(&lease_guards)
                    .map_err(|source| {
                        crate::offer::LegalActionProjectionError::SubjectAuthority {
                            action: Box::new(entry.action().clone()),
                            source: Box::new(source),
                        }
                    })?;
            }
            let offer = crate::offer::InternalOffer::new(
                crate::offer::InternalOfferDraft {
                    action: entry.action().clone(),
                    route: crate::offer::OfferRouteId::derive(entry.action()).map_err(
                        |source| crate::offer::LegalActionProjectionError::Offer {
                            action: Box::new(entry.action().clone()),
                            source: Box::new(crate::offer::OfferBuildError::Hash(source)),
                        },
                    )?,
                    viewer_scope: query.viewer_scope(),
                    truth_subject: query.truth_subject().clone(),
                    projected_subject: query.projected_subject().clone(),
                    fixed_parameters: projected.fixed_parameters,
                    parameters: projected.parameters,
                    targets: projected.targets,
                    required_channels: projected.required_channels,
                    expires: projected.expires,
                    presentation: projected.presentation,
                    guards: projected.guards,
                },
                keys,
            )
            .map_err(|source| crate::offer::LegalActionProjectionError::Offer {
                action: Box::new(entry.action().clone()),
                source: Box::new(source),
            })?;
            sink.push(offer)
                .map_err(|source| crate::offer::LegalActionProjectionError::Offer {
                    action: Box::new(entry.action().clone()),
                    source: Box::new(source),
                })?;
        }
        Ok(sink)
    }

    /// Validate through the exact ready provider without exposing a callable
    /// trait-object handle that could outlive the readiness check.
    pub fn validate_invocation(
        &self,
        action: &DefinitionRef,
        invocation: &ActionInvocation,
    ) -> Result<(), ActionProviderDispatchError> {
        let entry = self.get_exact(action)?;
        let result = entry.provider().validate_invocation(invocation);
        // A freeze racing provider evaluation wins over either success or a
        // normal provider refusal. The caller must not continue with work from
        // an owner that became mechanically unavailable during evaluation.
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| ActionProviderLookupError::OwnerNotReady {
                requested: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        result.map_err(|source| ActionProviderDispatchError::Provider {
            action: Box::new(action.clone()),
            source: Box::new(source),
        })
    }

    /// Obtain and canonicalize the prompt-time witness plan for one exact
    /// projected offer. Providers name only revision keys. This registry
    /// proves cross-owner bridges, fills projected choices that need no extra
    /// keys, and returns the projection-only rank used by bounded selection.
    pub fn delayed_decision_witness_keys(
        &self,
        action: &DefinitionRef,
        query: &crate::offer::LegalActionQuery,
        offer: &crate::offer::ProjectedLegalAction,
        engine: &crate::engine::Engine,
        definitions: &DefinitionRegistry,
        bridges: &BridgeRegistry,
    ) -> Result<
        (
            crate::decision::DecisionWitnessKeys,
            Vec<DefinitionRef>,
            i32,
        ),
        ActionProviderDispatchError,
    > {
        let entry = self.get_exact(action)?;
        let mut witnesses = entry
            .provider()
            .delayed_decision_witness_keys(action, query, offer, engine)
            .map_err(|source| ActionProviderDispatchError::Provider {
                action: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        let rank = entry
            .provider()
            .delayed_decision_rank(action, query, offer)
            .map_err(|source| ActionProviderDispatchError::Provider {
                action: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| ActionProviderLookupError::OwnerNotReady {
                requested: Box::new(action.clone()),
                source: Box::new(source),
            })?;

        canonicalize_decision_revision_keys(&mut witnesses.common)?;
        if witnesses.targets.len() > crate::decision::MAX_DECISION_TARGET_WITNESSES {
            return Err(ActionProviderDispatchError::DecisionTargetLimit {
                max: crate::decision::MAX_DECISION_TARGET_WITNESSES,
            });
        }
        let mut provided_targets = BTreeMap::new();
        for mut target in witnesses.targets {
            canonicalize_decision_revision_keys(&mut target.revisions)?;
            if provided_targets
                .insert(target.perceived, target.revisions)
                .is_some()
            {
                return Err(ActionProviderDispatchError::DuplicateDecisionTarget);
            }
        }

        let (maximum, mut choices) = match offer.targets() {
            crate::offer::ProjectedTargetSchema::None => {
                if !provided_targets.is_empty() {
                    return Err(ActionProviderDispatchError::DecisionTargetMismatch);
                }
                (0, Vec::new())
            }
            crate::offer::ProjectedTargetSchema::Choices {
                maximum, choices, ..
            } => (*maximum, choices.clone()),
        };
        let choice_set: BTreeSet<_> = choices.iter().cloned().collect();
        if provided_targets
            .keys()
            .any(|target| !choice_set.contains(target))
        {
            return Err(ActionProviderDispatchError::DecisionTargetMismatch);
        }
        choices.sort();
        let targets: Vec<_> = choices
            .into_iter()
            .map(|perceived| crate::decision::DecisionTargetWitnessKeys {
                revisions: provided_targets.remove(&perceived).unwrap_or_default(),
                perceived,
            })
            .collect();

        let mut largest: Vec<usize> = targets
            .iter()
            .map(|target| target.revisions.len())
            .collect();
        largest.sort_unstable_by(|left, right| right.cmp(left));
        if witnesses.common.len() + largest.into_iter().take(maximum).sum::<usize>()
            > MAX_ACTION_REVISIONS
        {
            return Err(ActionProviderDispatchError::DecisionRevisionLimit {
                max: MAX_ACTION_REVISIONS,
            });
        }

        let mut bridge_definitions = BTreeSet::new();
        for key in witnesses
            .common
            .iter()
            .chain(targets.iter().flat_map(|target| &target.revisions))
            .filter(|key| key.owner != entry.owner)
        {
            let route = bridges
                .require_interaction(definitions, &entry.owner, &key.owner, action)
                .map_err(|source| ActionProviderDispatchError::RevisionBridge {
                    key: Box::new(key.clone()),
                    source: Box::new(source),
                })?;
            let InteractionRoute::Bridge(definition) = route else {
                return Err(ActionProviderDispatchError::RevisionBridgeRoute {
                    key: Box::new(key.clone()),
                });
            };
            bridge_definitions.insert(definition);
        }
        Ok((
            crate::decision::DecisionWitnessKeys {
                common: witnesses.common,
                targets,
            },
            bridge_definitions.into_iter().collect(),
            rank,
        ))
    }

    /// Select additional witnesses through the exact ready provider. Issuance
    /// and the final guard-set bound remain kernel responsibilities.
    pub(crate) fn resolved_authority_witnesses(
        &self,
        action: &DefinitionRef,
        resolved: &ResolvedActionInvocation,
        engine: &crate::engine::Engine,
    ) -> Result<Vec<AuthorityWitnessRequest>, ActionProviderDispatchError> {
        let entry = self.get_exact(action)?;
        let requests = entry
            .provider()
            .resolved_authority_witnesses(resolved, engine)
            .map_err(|source| ActionProviderDispatchError::Provider {
                action: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        self.get_exact(action)?; // Recheck readiness after the callback.
        Ok(requests)
    }

    /// Ask one exact provider which revision coordinates are selected by the
    /// privately resolved invocation. Readiness is checked on both sides of
    /// the callback. Cross-owner coordinates require the unique exact bridge
    /// for this action; its definition is returned for final guarding.
    pub(crate) fn resolved_revision_keys(
        &self,
        action: &DefinitionRef,
        resolved: &ResolvedActionInvocation,
        definitions: &DefinitionRegistry,
        bridges: &BridgeRegistry,
    ) -> Result<(Vec<RevisionKey>, Vec<DefinitionRef>), ActionProviderDispatchError> {
        let entry = self.get_exact(action)?;
        let mut keys = entry
            .provider()
            .resolved_revision_keys(resolved)
            .map_err(|source| ActionProviderDispatchError::Provider {
                action: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| ActionProviderLookupError::OwnerNotReady {
                requested: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        if keys.len() > MAX_ACTION_REVISIONS {
            return Err(ActionProviderDispatchError::RevisionLimit {
                max: MAX_ACTION_REVISIONS,
            });
        }
        let mut bridge_definitions = BTreeSet::new();
        for key in keys.iter().filter(|key| key.owner != entry.owner) {
            let route = bridges
                .require_interaction(definitions, &entry.owner, &key.owner, action)
                .map_err(|source| ActionProviderDispatchError::RevisionBridge {
                    key: Box::new(key.clone()),
                    source: Box::new(source),
                })?;
            let InteractionRoute::Bridge(definition) = route else {
                return Err(ActionProviderDispatchError::RevisionBridgeRoute {
                    key: Box::new(key.clone()),
                });
            };
            bridge_definitions.insert(definition);
        }
        keys.sort();
        if keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ActionProviderDispatchError::DuplicateRevisionKey);
        }
        Ok((keys, bridge_definitions.into_iter().collect()))
    }

    /// Run one exact provider inside an owner-scoped logical transaction.
    /// Provider and serializer work finish before the returned one-shot
    /// materialization can cross the persistence boundary.
    pub fn prepare_invocation(
        &self,
        action: &DefinitionRef,
        invocation: &ActionInvocation,
        state: &mut ActionStateStore,
    ) -> Result<PreparedTxn, ActionProviderPrepareError> {
        let entry = self.get_exact(action)?;
        let mut txn = state.transaction_for_provider(entry.registration_ordinal())?;
        entry
            .provider()
            .prepare_invocation(invocation, &mut txn)
            .map_err(|source| ActionProviderPrepareError::Provider {
                action: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| {
                ActionProviderPrepareError::Lookup(ActionProviderLookupError::OwnerNotReady {
                    requested: Box::new(action.clone()),
                    source: Box::new(source),
                })
            })?;
        txn.prepare().map_err(ActionProviderPrepareError::State)
    }

    /// Run one exact provider through its K3 semantic preparation surface and
    /// return the semantic material beside the still-one-shot typed state
    /// transaction.
    pub fn prepare_authoritative_invocation(
        &self,
        action: &DefinitionRef,
        invocation: &ActionInvocation,
        commit_token: CommitToken,
        state: &mut ActionStateStore,
        resolution_keys: Option<&crate::resolution::ResolutionKeyRing>,
    ) -> Result<(PreparedTxn, PreparedAuthoritativeAction), ActionProviderPrepareError> {
        self.prepare_authoritative_invocation_seeded(
            action,
            invocation,
            commit_token,
            state,
            resolution_keys,
            |_| Ok(()),
        )
    }

    /// Run one exact provider with server-private resolved subject/target
    /// identities. The durable bundle still binds the projected invocation;
    /// only provider preparation sees this non-serializable wrapper.
    pub fn prepare_resolved_authoritative_invocation(
        &self,
        action: &DefinitionRef,
        resolved: &ResolvedActionInvocation,
        commit_token: CommitToken,
        state: &mut ActionStateStore,
        resolution_keys: Option<&crate::resolution::ResolutionKeyRing>,
    ) -> Result<(PreparedTxn, PreparedAuthoritativeAction), ActionProviderPrepareError> {
        self.prepare_authoritative_invocation_seeded_inner(
            action,
            resolved.invocation(),
            Some(resolved),
            commit_token,
            state,
            resolution_keys,
            |_| Ok(()),
        )
    }

    pub(crate) fn prepare_authoritative_invocation_seeded<F>(
        &self,
        action: &DefinitionRef,
        invocation: &ActionInvocation,
        commit_token: CommitToken,
        state: &mut ActionStateStore,
        resolution_keys: Option<&crate::resolution::ResolutionKeyRing>,
        seed: F,
    ) -> Result<(PreparedTxn, PreparedAuthoritativeAction), ActionProviderPrepareError>
    where
        F: FnOnce(&mut ActionTxn<'_>) -> Result<(), ActionProviderPrepareError>,
    {
        self.prepare_authoritative_invocation_seeded_inner(
            action,
            invocation,
            None,
            commit_token,
            state,
            resolution_keys,
            seed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_authoritative_invocation_seeded_inner<F>(
        &self,
        action: &DefinitionRef,
        invocation: &ActionInvocation,
        resolved: Option<&ResolvedActionInvocation>,
        commit_token: CommitToken,
        state: &mut ActionStateStore,
        resolution_keys: Option<&crate::resolution::ResolutionKeyRing>,
        seed: F,
    ) -> Result<(PreparedTxn, PreparedAuthoritativeAction), ActionProviderPrepareError>
    where
        F: FnOnce(&mut ActionTxn<'_>) -> Result<(), ActionProviderPrepareError>,
    {
        let entry = self.get_exact(action)?;
        let mut txn = state.transaction_for_provider(entry.registration_ordinal())?;
        let mut queue = CausalTriggerQueue::new(invocation.request(), self.owner_readiness.clone());
        let root_cause = queue.allocate_root_event()?;
        let root_before_operations = txn.operation_count();
        seed(&mut txn)?;
        let mut prepare_context = crate::resolution::PrepareCtx::new(
            invocation.idempotency(),
            commit_token,
            resolution_keys,
        );
        let root_output = match resolved {
            Some(resolved) => entry
                .provider()
                .prepare_resolved_authoritative_with_context(
                    resolved,
                    &mut txn,
                    &mut prepare_context,
                ),
            None => entry.provider().prepare_authoritative_with_context(
                invocation,
                &mut txn,
                &mut prepare_context,
            ),
        }
        .map_err(|source| ActionProviderPrepareError::Provider {
            action: Box::new(action.clone()),
            source: Box::new(source),
        })?;
        validate_resolution_custody(
            prepare_context.resolution_opened(),
            root_output.resolution().is_some(),
        )?;
        charge_new_operations(&mut queue, root_before_operations, &txn)?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| {
                ActionProviderPrepareError::Lookup(ActionProviderLookupError::OwnerNotReady {
                    requested: Box::new(action.clone()),
                    source: Box::new(source),
                })
            })?;
        let (
            outcome,
            resolution,
            mut definitions,
            mut eligible_witnesses,
            mut durable_outbox,
            root_triggers,
        ) = root_output.into_cascade_parts();
        order_outbox_group(&mut durable_outbox, ROOT_OUTBOX_GROUP)?;
        if let Some(prepared) = &resolution {
            if prepared.outcome() != &outcome {
                return Err(ActionProviderPrepareError::ResolutionOutcomeMismatch);
            }
            definitions.extend(prepared.definition_refs().iter().cloned());
            definitions.push(prepared.modifiers().schema().clone());
            definitions.push(prepared.outcome().schema().clone());
        }
        definitions.push(outcome.schema().clone());
        if eligible_witnesses.is_empty() {
            eligible_witnesses.push(invocation.principal().clone());
        }
        let root_event = AuthoritativeSemanticEvent::new(
            root_cause,
            entry.owner.clone(),
            CanonicalId::parse_canonical(action.key().id.as_str())
                .expect("a definition id is also a canonical mechanics id"),
            outcome.clone(),
            eligible_witnesses.clone(),
        )?;
        queue.charge_bytes(root_event.canonical_len()?)?;
        for trigger in root_triggers {
            queue.enqueue(trigger.into_prepared(entry, root_cause.event_id()))?;
        }
        let mut semantic_events = vec![root_event];

        while let Some(work) = queue.pop_next()? {
            let cause = queue.allocate_derived_event(&work)?;
            let before_operations = txn.operation_count();
            let output = entry
                .provider()
                .prepare_same_commit_trigger(invocation, work.trigger(), &mut txn)
                .map_err(|source| ActionProviderPrepareError::Trigger {
                    action: Box::new(action.clone()),
                    handler: work.trigger().identity().handler.id().clone(),
                    source: Box::new(source),
                })?;
            charge_new_operations(&mut queue, before_operations, &txn)?;
            let (
                event_truth,
                event_resolution,
                event_definitions,
                event_witnesses,
                mut event_outbox,
                triggers,
            ) = output.into_cascade_parts();
            if event_resolution.is_some() {
                return Err(ActionProviderPrepareError::TriggerResolutionUnsupported);
            }
            definitions.push(event_truth.schema().clone());
            let event = AuthoritativeSemanticEvent::new(
                cause,
                entry.owner.clone(),
                work.trigger().identity().handler.id().clone(),
                event_truth,
                event_witnesses.clone(),
            )?;
            queue.charge_bytes(event.canonical_len()?)?;
            for trigger in triggers {
                queue.enqueue(trigger.into_prepared(entry, cause.event_id()))?;
            }
            definitions.extend(event_definitions);
            order_outbox_group(
                &mut event_outbox,
                work.trigger().identity().handler.id().as_str(),
            )?;
            durable_outbox.extend(event_outbox);
            semantic_events.push(event);
            queue.complete_trigger(work)?;
        }
        queue.finish()?;
        definitions.sort();
        definitions.dedup();
        eligible_witnesses.sort();
        eligible_witnesses.dedup();
        let durable_outbox = renumber_durable_outbox(durable_outbox)?;
        let prepared = txn.prepare().map_err(ActionProviderPrepareError::State)?;
        Ok((
            prepared,
            PreparedAuthoritativeAction {
                outcome,
                resolution,
                definitions,
                eligible_witnesses,
                durable_outbox,
                semantic_events,
            },
        ))
    }

    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }
}

struct ReceiptProjectionEntry {
    owner: OwnerId,
    action: DefinitionRef,
    provider: Arc<dyn ActionProvider>,
}

/// Frozen, thread-safe table of pure durable-receipt projectors.
pub struct ReceiptProjectionTable {
    entries: BTreeMap<crate::mechanics::DefinitionLocator, ReceiptProjectionEntry>,
    owner_readiness: MechanicalOwnerReadiness,
}

impl fmt::Debug for ReceiptProjectionTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceiptProjectionTable")
            .field("actions", &self.entries.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl ReceiptProjectionTable {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: BTreeMap::new(),
            owner_readiness: MechanicalOwnerReadiness::for_owners(&BTreeSet::new()),
        }
    }

    /// Project one exact durable root outcome without exposing provider
    /// callbacks or allowing a current-state read.
    pub fn project(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ReceiptProjectionError> {
        let Some(entry) = self.entries.get(&action.locator()) else {
            return Err(ReceiptProjectionError::Missing {
                action: Box::new(action.clone()),
            });
        };
        if entry.action.digest() != action.digest() {
            return Err(ReceiptProjectionError::DigestMismatch {
                requested: Box::new(action.clone()),
                registered: Box::new(entry.action.clone()),
            });
        }
        if outcome.schema().key().owner != entry.owner {
            return Err(ReceiptProjectionError::OutcomeOwnerMismatch {
                action: Box::new(action.clone()),
                outcome: Box::new(outcome.schema().clone()),
            });
        }
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| ReceiptProjectionError::OwnerNotReady {
                action: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        let projected = entry
            .provider
            .project_receipt(&entry.action, outcome, viewer)
            .map_err(|source| ReceiptProjectionError::Provider {
                action: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        self.owner_readiness
            .require_ready(&entry.owner)
            .map_err(|source| ReceiptProjectionError::OwnerNotReady {
                action: Box::new(action.clone()),
                source: Box::new(source),
            })?;
        Ok(projected)
    }
}

/// Failure to project a private durable result into its public receipt shape.
#[derive(Debug, Error)]
pub enum ReceiptProjectionError {
    #[error("no receipt projector is registered for {action}")]
    Missing { action: Box<DefinitionRef> },
    #[error("receipt action {requested} differs from registered {registered}")]
    DigestMismatch {
        requested: Box<DefinitionRef>,
        registered: Box<DefinitionRef>,
    },
    #[error("receipt outcome {outcome} is not owned by action provider {action}")]
    OutcomeOwnerMismatch {
        action: Box<DefinitionRef>,
        outcome: Box<DefinitionRef>,
    },
    #[error("receipt projector owner is unavailable for {action}: {source}")]
    OwnerNotReady {
        action: Box<DefinitionRef>,
        source: Box<MechanicalOwnerReadinessError>,
    },
    #[error("receipt projector refused {action}: {source}")]
    Provider {
        action: Box<DefinitionRef>,
        source: Box<ActionError>,
    },
}

fn charge_new_operations(
    queue: &mut CausalTriggerQueue,
    before: usize,
    txn: &ActionTxn<'_>,
) -> Result<(), CascadeError> {
    let added = txn
        .operation_count()
        .checked_sub(before)
        .expect("action transaction operation count is monotonic");
    queue.charge_operations(
        u32::try_from(added).expect("action transaction hard operation limit fits u32"),
    )
}

fn validate_resolution_custody(
    context_opened: bool,
    transcript_supplied: bool,
) -> Result<(), ActionProviderPrepareError> {
    match (context_opened, transcript_supplied) {
        (false, false) | (true, true) => Ok(()),
        (true, false) => Err(ActionProviderPrepareError::UnrecordedResolution),
        (false, true) => Err(ActionProviderPrepareError::ResolutionOutsideContext),
    }
}

/// The name the root provider's own emission group carries in an ordinal
/// conflict; every other group is named by its trigger handler.
const ROOT_OUTBOX_GROUP: &str = "root";

/// Put one provider emission group in the order its own ordinals ask for.
///
/// Ordinals are a hint *inside* a group (see
/// [`AuthoritativeActionOutput::with_durable_outbox`]), so this settles the
/// order the group asked for and refuses the one thing the provider cannot
/// mean: the same ordinal twice, where nothing says which intent leads.
fn order_outbox_group(
    group: &mut [DurableOutboxIntent],
    name: &str,
) -> Result<(), ActionProviderPrepareError> {
    group.sort_by_key(DurableOutboxIntent::ordinal);
    if let Some(pair) = group
        .windows(2)
        .find(|pair| pair[0].ordinal() == pair[1].ordinal())
    {
        return Err(ActionProviderPrepareError::OutboxOrdinalConflict {
            group: name.into(),
            ordinal: pair[0].ordinal(),
        });
    }
    Ok(())
}

/// Renumber the concatenated groups `0..n` so the bundle sees one contiguous
/// run.
///
/// Groups arrive root-first and then in cascade order, each already sorted by
/// its own hint, and this stamps delivery order onto them. It is the identity
/// for the common case — one group already numbered from zero — so the usual
/// bundle's `payload_hash`es and `bundle_hash` are exactly what the provider
/// built. The bound is checked here rather than left to
/// [`AuthoritativeBundle`](crate::authoritative::AuthoritativeBundle)
/// construction so an over-long outbox is refused at its cause.
fn renumber_durable_outbox(
    outbox: Vec<DurableOutboxIntent>,
) -> Result<Vec<DurableOutboxIntent>, ActionProviderPrepareError> {
    if outbox.len() > MAX_DURABLE_OUTBOX_INTENTS {
        return Err(ActionProviderPrepareError::Semantic(
            AuthoritativeBundleError::OutboxLimit {
                actual: outbox.len(),
                max: MAX_DURABLE_OUTBOX_INTENTS,
            },
        ));
    }
    outbox
        .into_iter()
        .enumerate()
        .map(|(index, intent)| {
            let ordinal = u16::try_from(index).map_err(|_| {
                ActionProviderPrepareError::Semantic(AuthoritativeBundleError::OutboxLimit {
                    actual: index.saturating_add(1),
                    max: MAX_DURABLE_OUTBOX_INTENTS,
                })
            })?;
            intent
                .renumbered(ordinal)
                .map_err(ActionProviderPrepareError::Semantic)
        })
        .collect()
}

/// Exact transactional action-preparation failure. Every variant precedes
/// both live installation and persistence submission.
#[derive(Debug, Error)]
pub enum ActionProviderPrepareError {
    #[error("action-provider registry is unavailable before module composition")]
    RegistryUnavailable,
    #[error("action provider cannot be selected: {0}")]
    Lookup(#[from] ActionProviderLookupError),
    #[error("action provider rejected {action}: {source}")]
    Provider {
        action: Box<DefinitionRef>,
        source: Box<ActionError>,
    },
    #[error("same-commit trigger {handler} for {action} was rejected: {source}")]
    Trigger {
        action: Box<DefinitionRef>,
        handler: CanonicalId,
        source: Box<ActionError>,
    },
    #[error("same-commit causal expansion failed: {0}")]
    Cascade(#[from] CascadeError),
    #[error("durable-outbox group {group} claimed ordinal {ordinal} more than once")]
    OutboxOrdinalConflict { group: Box<str>, ordinal: u16 },
    #[error("authoritative semantic event preparation failed: {0}")]
    Semantic(#[from] AuthoritativeBundleError),
    #[error("prepared resolution outcome does not match the authoritative action outcome")]
    ResolutionOutcomeMismatch,
    #[error("provider opened an authoritative RNG but did not return its resolution transcript")]
    UnrecordedResolution,
    #[error("provider returned a resolution transcript without opening its PrepareCtx RNG")]
    ResolutionOutsideContext,
    #[error("same-commit triggers cannot introduce an independent resolution transcript")]
    TriggerResolutionUnsupported,
    #[error("action state preparation failed: {0}")]
    State(#[from] ActionStateError),
    #[error("durable schedule advance was not admissible: {0}")]
    Schedule(#[source] Box<crate::schedule_source::ScheduleAdvanceAdmissionError>),
    #[error("delayed-decision lease was not admissible: {0}")]
    DecisionLease(#[source] Box<crate::decision_source::DecisionSourcePlanError>),
    #[error("delayed-decision lifecycle transition was not admissible: {0}")]
    DecisionLifecycle(#[source] Box<crate::decision_source::DecisionResolvingPlanError>),
    #[error("delayed-decision terminal transition was not admissible: {0}")]
    DecisionTerminal(#[source] Box<crate::decision_source::DecisionTerminalPlanError>),
    #[error("delayed-decision compaction was not admissible: {0}")]
    DecisionCompaction(#[source] Box<crate::decision_source::DecisionCompactionError>),
    #[error("delayed-decision provider attempt was not admissible: {0}")]
    DecisionAttempt(#[source] Box<crate::decision_source::DecisionAttemptPlanError>),
    #[error("delayed-decision fallback transfer was not admissible: {0}")]
    DecisionFallbackTrigger(
        #[source] Box<crate::decision_source::DecisionFallbackTriggerPlanError>,
    ),
    #[error("delayed-decision cancellation was not admissible: {0}")]
    DecisionCancellation(#[source] Box<crate::decision_source::DecisionCancellationPlanError>),
}

/// Exact action-provider dispatch failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionProviderDispatchError {
    #[error("action provider cannot be selected: {0}")]
    Lookup(#[from] ActionProviderLookupError),
    #[error("action provider rejected {action}: {source}")]
    Provider {
        action: Box<DefinitionRef>,
        source: Box<ActionError>,
    },
    #[error("resolved revision-key count exceeds its hard limit of {max}")]
    RevisionLimit { max: usize },
    #[error("resolved revision coordinate {key:?} has no executable bridge: {source}")]
    RevisionBridge {
        key: Box<RevisionKey>,
        source: Box<BridgeLookupError>,
    },
    #[error("resolved revision coordinate {key:?} selected an invalid same-owner bridge route")]
    RevisionBridgeRoute { key: Box<RevisionKey> },
    #[error("resolved revision coordinates contain a duplicate")]
    DuplicateRevisionKey,
    #[error("delayed-decision revision witnesses exceed the selected-action limit of {max}")]
    DecisionRevisionLimit { max: usize },
    #[error("delayed-decision target witnesses exceed their hard limit of {max}")]
    DecisionTargetLimit { max: usize },
    #[error("delayed-decision target witness was supplied more than once")]
    DuplicateDecisionTarget,
    #[error("delayed-decision target witnesses do not belong to the projected choices")]
    DecisionTargetMismatch,
}

fn canonicalize_decision_revision_keys(
    keys: &mut [RevisionKey],
) -> Result<(), ActionProviderDispatchError> {
    if keys.len() > MAX_ACTION_REVISIONS {
        return Err(ActionProviderDispatchError::DecisionRevisionLimit {
            max: MAX_ACTION_REVISIONS,
        });
    }
    keys.sort();
    if keys.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ActionProviderDispatchError::DuplicateRevisionKey);
    }
    Ok(())
}

/// Action-provider composition failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionProviderRegistryError {
    #[error("action providers and definitions do not belong to one module composition")]
    RegistryCompositionMismatch,
    #[error("definition {action} is not an action owned by provider {provider}")]
    WrongActionOwner {
        provider: OwnerId,
        action: Box<DefinitionRef>,
    },
    #[error("action provider was registered more than once for {action}")]
    Duplicate { action: Box<DefinitionRef> },
    #[error("action-provider locator has conflicting digests: {registered} vs {attempted}")]
    DigestConflict {
        registered: Box<DefinitionRef>,
        attempted: Box<DefinitionRef>,
    },
    #[error("action-provider registry exceeds its hard limit of {max}")]
    ProviderLimit { max: usize },
    #[error("owner {owner} exceeds its action-provider limit of {max}")]
    OwnerProviderLimit { owner: OwnerId, max: usize },
    #[error("action provider names uncomposed owner {owner}")]
    UnknownOwner { owner: OwnerId },
    #[error("action provider's exact definition {action} is unavailable: {source}")]
    MissingActionDefinition {
        action: Box<DefinitionRef>,
        source: Box<DefinitionLookupError>,
    },
    #[error("cannot derive an opaque offer route: {0}")]
    OfferRouteHash(#[source] CanonicalHashError),
    #[error("opaque offer route collision between {first:?} and {second:?}")]
    OfferRouteCollision {
        first: Box<crate::mechanics::DefinitionLocator>,
        second: Box<crate::mechanics::DefinitionLocator>,
    },
    #[error("cannot fingerprint action providers: {0}")]
    Fingerprint(#[from] CanonicalEncodingError),
}

/// Exact provider lookup failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionProviderLookupError {
    #[error("exact action provider {requested} is unavailable")]
    Missing { requested: Box<DefinitionRef> },
    #[error(
        "action provider {requested} has the wrong digest; registered identity is {registered}"
    )]
    DigestMismatch {
        requested: Box<DefinitionRef>,
        registered: Box<DefinitionRef>,
    },
    #[error("exact action provider {requested} is mechanically unavailable: {source}")]
    OwnerNotReady {
        requested: Box<DefinitionRef>,
        source: Box<MechanicalOwnerReadinessError>,
    },
}

fn is_action_owned_by(provider: &OwnerId, action: &DefinitionRef) -> bool {
    action.key().owner == *provider && action.key().kind.as_str() == ACTION_DEFINITION_KIND
}

fn provider_fingerprint<'a>(
    entries: impl Iterator<Item = &'a ActionProviderEntry>,
) -> Result<DefinitionDigest, ActionProviderRegistryError> {
    let value = Value::Array(
        entries
            .map(|entry| {
                json!({
                    "action": entry.action,
                    "registration_ordinal": entry.registration_ordinal().get(),
                })
            })
            .collect(),
    );
    DefinitionDigest::for_canonical_value("action_provider_registry.v1", &value)
        .map_err(ActionProviderRegistryError::Fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mechanics::{
        BridgeDescriptor, BridgeRegistryBuilder, DefinitionDescriptor, DefinitionId,
        DefinitionKind, DefinitionRegistryBuilder, IncidentId, RecoveryReason,
    };

    fn owner() -> OwnerId {
        OwnerId::new("toy").unwrap()
    }

    fn principal() -> PrincipalId {
        PrincipalId::Scoped {
            owner: OwnerId::new("core").unwrap(),
            class: CanonicalId::new("account").unwrap(),
            id: IdempotencyKeyText::parse_canonical("alice").unwrap(),
        }
    }

    fn request(byte: u8) -> RequestUid {
        RequestUid::from_bytes([byte; 16])
    }

    fn idempotency(request: RequestUid) -> ScopedIdempotencyKey {
        ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal(),
                owner: owner(),
                namespace: CanonicalId::new("action").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        )
    }

    fn action_definition() -> (DefinitionRegistry, DefinitionRef) {
        let mut builder = DefinitionRegistryBuilder::new();
        let action = builder
            .register(
                owner(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("wave").unwrap(),
                    1,
                    json!({"parameter": "target"}),
                )
                .unwrap(),
            )
            .unwrap();
        (builder.freeze().unwrap(), action)
    }

    fn owner_roster() -> BTreeSet<OwnerId> {
        BTreeSet::from([owner()])
    }

    fn bind_definitions(
        mut definitions: DefinitionRegistry,
        readiness: &MechanicalOwnerReadiness,
    ) -> DefinitionRegistry {
        definitions.bind_owner_readiness(readiness.clone());
        definitions
    }

    fn two_action_definitions() -> (DefinitionRegistry, DefinitionRef, DefinitionRef) {
        let mut builder = DefinitionRegistryBuilder::new();
        let first = builder
            .register(
                owner(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("advance").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let second = builder
            .register(
                owner(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("withdraw").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        (builder.freeze().unwrap(), first, second)
    }

    fn invocation(request: RequestUid) -> ActionInvocation {
        ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal(),
            actor: ObjId(7),
            offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![PerceivedRef::KnownObject {
                id: ObjId(8),
                revision: 3,
            }],
            parameters: json!({"force": 2}),
            idempotency: idempotency(request),
        })
        .unwrap()
    }

    #[test]
    fn request_uid_wire_form_is_fixed_and_strict() {
        let uid = request(0xab);
        assert_eq!(uid.to_string(), "abababababababababababababababab");
        assert_eq!(
            serde_json::from_str::<RequestUid>(&format!("\"{uid}\"")).unwrap(),
            uid
        );
        assert!(
            serde_json::from_str::<RequestUid>("\"ABABABABABABABABABABABABABABABAB\"").is_err()
        );
        assert!(serde_json::from_str::<RequestUid>("\"ab\"").is_err());
    }

    #[test]
    fn ingress_hash_ignores_request_identity_but_checks_stored_hash() {
        let surface = IngressSurfaceVersion::new(CanonicalId::new("ssh.line").unwrap(), 1).unwrap();
        let first = IngressEnvelope::new(
            request(1),
            principal(),
            surface.clone(),
            IngressPayload::line("look").unwrap(),
            idempotency(request(1)),
        )
        .unwrap();
        let second = IngressEnvelope::new(
            request(2),
            principal(),
            surface,
            IngressPayload::line("look").unwrap(),
            idempotency(request(2)),
        )
        .unwrap();
        assert_eq!(first.ingress_hash(), second.ingress_hash());

        let mut wire = serde_json::to_value(first).unwrap();
        wire["ingress_hash"] = Value::String(CanonicalHash::from_bytes([4; 32]).to_hex());
        assert!(serde_json::from_value::<IngressEnvelope>(wire).is_err());
    }

    #[test]
    fn ingress_wire_shape_round_trips_and_rejects_unknown_fields() {
        let payload = IngressPayload::structured(json!({"z": 1, "a": [true]})).unwrap();
        let encoded = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            encoded,
            json!({"kind": "structured", "value": {"a": [true], "z": 1}})
        );
        assert_eq!(
            serde_json::from_value::<IngressPayload>(encoded).unwrap(),
            payload
        );
        assert!(
            serde_json::from_value::<IngressPayload>(
                json!({"kind": "line", "value": "look", "extra": true})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<IdempotencyKey>(
                json!({"kind": "request", "value": request(1), "extra": true})
            )
            .is_err()
        );
    }

    #[test]
    fn request_hash_is_semantic_and_rejects_duplicate_targets() {
        let first = invocation(request(1));
        let second = invocation(request(2));
        assert_eq!(first.request_hash(), second.request_hash());

        let target = PerceivedRef::SelfActor;
        let duplicate = ActionInvocation::new(ActionInvocationDraft {
            request: request(3),
            principal: principal(),
            actor: ObjId(7),
            offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![target.clone(), target],
            parameters: json!({}),
            idempotency: idempotency(request(3)),
        });
        assert_eq!(duplicate.unwrap_err(), ActionMetadataError::DuplicateTarget);
    }

    #[test]
    fn revision_set_serializes_in_one_order_and_rejects_noncanonical_wire() {
        let first = RevisionKey {
            owner: owner(),
            kind: RevisionKind::new(CanonicalId::new("object").unwrap()),
            subject: SubjectKey::Object { id: ObjId(1) },
            facet: None,
        };
        let second = RevisionKey {
            owner: owner(),
            kind: RevisionKind::new(CanonicalId::new("object").unwrap()),
            subject: SubjectKey::Object { id: ObjId(2) },
            facet: None,
        };
        let set = RevisionSet::from_stamps([
            RevisionStamp {
                key: second.clone(),
                generation: 2,
            },
            RevisionStamp {
                key: first.clone(),
                generation: 1,
            },
        ])
        .unwrap();
        let value = serde_json::to_value(&set).unwrap();
        let stamps = value.as_array().unwrap();
        assert_eq!(stamps[0]["key"]["subject"]["id"], json!(1));
        let reversed = Value::Array(stamps.iter().rev().cloned().collect());
        assert!(serde_json::from_value::<RevisionSet>(reversed).is_err());
        assert!(
            RevisionSet::from_stamps([
                RevisionStamp {
                    key: first.clone(),
                    generation: 1,
                },
                RevisionStamp {
                    key: first,
                    generation: 2,
                },
            ])
            .is_err()
        );
    }

    #[test]
    fn guards_are_sorted_duplicate_free_and_definition_exact() {
        let readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let (definitions, action) = action_definition();
        let definitions = bind_definitions(definitions, &readiness);
        let mut builder = ServerGuardSetBuilder::new(RevisionSet::default(), readiness);
        builder
            .add_definition(action.clone(), &definitions)
            .unwrap();
        assert!(matches!(
            builder.add(AuthorityGuard::Definition(action.clone())),
            Err(GuardSetBuildError::DefinitionRequiresRegistry)
        ));
        assert!(matches!(
            builder.add_definition(action, &definitions),
            Err(GuardSetBuildError::Duplicate { .. })
        ));
        let set = builder.finish().unwrap();
        assert_eq!(set.authority().len(), 1);
        assert_ne!(
            set.canonical_hash().unwrap(),
            CanonicalHash::from_bytes([0; 32])
        );
    }

    fn test_channel_grant(
        station: &str,
        priority: u16,
        epoch: u64,
        channels: &[ChannelId],
    ) -> crate::world::ChannelGrant {
        crate::world::ChannelGrant {
            actor: crate::GOD,
            subject: crate::LIMBO,
            station: SmolStr::new(station),
            relation: SmolStr::new_static("operator"),
            owner: SmolStr::new(owner().as_str()),
            priority,
            channels: channels.iter().cloned().collect(),
            epoch,
        }
    }

    fn operated_query() -> crate::offer::LegalActionQuery {
        crate::offer::LegalActionQuery::new(
            PrincipalId::Object { id: crate::GOD },
            crate::GOD,
            SubjectKey::Object { id: crate::LIMBO },
            PerceivedRef::KnownObject {
                id: crate::LIMBO,
                revision: 1,
            },
            1,
            None,
        )
        .unwrap()
    }

    #[test]
    fn channel_offer_selects_one_whole_unambiguous_station_grant() {
        let movement = ChannelId::new("movement").unwrap();
        let weapons = ChannelId::new("weapons").unwrap();
        let required = vec![movement.clone(), weapons.clone()];
        let mut world = World::new_minimal();

        // No cross-seat union: neither specialist grants the complete set.
        world.publish_command_subject(test_channel_grant(
            "driver",
            20,
            4,
            std::slice::from_ref(&movement),
        ));
        world.publish_command_subject(test_channel_grant(
            "gunner",
            20,
            9,
            std::slice::from_ref(&weapons),
        ));
        assert!(select_offer_channel_grant(&world, &operated_query(), &required).is_none());

        world.publish_command_subject(test_channel_grant("commander", 30, 11, &required));
        let (publisher, selected) =
            select_offer_channel_grant(&world, &operated_query(), &required).unwrap();
        assert_eq!(publisher, owner());
        assert_eq!(selected.station, "commander");
        assert_eq!(selected.priority, 30);

        // Equal best priority is an authority ambiguity, not a lexical tie
        // break that silently chooses a seat.
        world.publish_command_subject(test_channel_grant("remote", 30, 12, &required));
        assert!(select_offer_channel_grant(&world, &operated_query(), &required).is_none());
    }

    #[test]
    fn channel_grant_guard_revalidates_station_priority_epoch_and_channel() {
        let channel = ChannelId::new("movement").unwrap();
        let mut world = World::new_minimal();
        let grant = test_channel_grant("cockpit", 40, 7, std::slice::from_ref(&channel));
        world.publish_command_subject(grant.clone());

        let readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let (definitions, action) = action_definition();
        let definitions = bind_definitions(definitions, &readiness);
        let mut builder = ServerGuardSetBuilder::new(RevisionSet::default(), readiness.clone());
        builder.add_definition(action, &definitions).unwrap();
        let guards = builder
            .finish()
            .unwrap()
            .with_channel_grant(&owner(), &grant, std::slice::from_ref(&channel))
            .unwrap();
        let revisions = RevisionValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&owner_roster(), readiness.clone())
            .unwrap();
        let authority = AuthorityGuardValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&owner_roster(), readiness)
            .unwrap();

        assert!(
            guards
                .revalidate_unregistered(&world, &revisions, &authority, &definitions)
                .unwrap()
                .is_current()
        );

        // Even if a buggy publisher forgets to bump the epoch, priority is
        // part of the exact private witness and invalidates queued work.
        world.publish_command_subject(test_channel_grant(
            "cockpit",
            41,
            7,
            std::slice::from_ref(&channel),
        ));
        let stale = guards
            .revalidate_unregistered(&world, &revisions, &authority, &definitions)
            .unwrap();
        assert_eq!(stale.stale_authority().len(), 1);
        assert!(matches!(
            stale.stale_authority()[0],
            AuthorityGuard::ChannelGrant { .. }
        ));
    }

    struct ChannelOfferProvider {
        guards: GuardSet,
        channels: Vec<ChannelId>,
    }

    struct CurrentLeaseAuthority {
        epoch: u64,
    }

    impl AuthorityGuardValidator for CurrentLeaseAuthority {
        fn is_current(
            &self,
            _context: &AuthorityGuardContext<'_>,
            guard: &AuthorityGuard,
        ) -> Result<bool, ActionError> {
            Ok(matches!(
                guard,
                AuthorityGuard::ControlLease { epoch, .. } if *epoch == self.epoch
            ))
        }

        fn witness(
            &self,
            _context: &AuthorityGuardContext<'_>,
            request: &AuthorityWitnessRequest,
        ) -> Result<Option<AuthorityWitness>, ActionError> {
            let AuthorityWitnessRequest::ControlLease {
                provider,
                subject,
                channel,
                holder,
            } = request
            else {
                return Ok(None);
            };
            Ok(Some(AuthorityWitness::Guard(
                AuthorityGuard::ControlLease {
                    provider: provider.clone(),
                    subject: *subject,
                    channel: channel.clone(),
                    holder: holder.clone(),
                    epoch: self.epoch,
                },
            )))
        }
    }

    impl ActionProvider for ChannelOfferProvider {
        fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
            Ok(())
        }

        fn project_offer(
            &self,
            _action: &DefinitionRef,
            _query: &crate::offer::LegalActionQuery,
            _engine: &crate::engine::Engine,
        ) -> Result<Option<crate::offer::ProviderOfferDraft>, ActionError> {
            Ok(Some(crate::offer::ProviderOfferDraft {
                fixed_parameters: json!({}),
                parameters: crate::offer::ProjectedParameterSchema::empty(),
                targets: crate::offer::ProjectedTargetSchema::None,
                required_channels: self.channels.clone(),
                expires: None,
                presentation: crate::offer::ActionPresentation::new(
                    "operate",
                    "Operate",
                    "Use the selected station.",
                )
                .unwrap(),
                guards: self.guards.clone(),
            }))
        }
    }

    #[test]
    fn provider_registry_privately_binds_required_channels_to_selected_grant() {
        let movement = ChannelId::new("movement").unwrap();
        let weapons = ChannelId::new("weapons").unwrap();
        let channels = vec![movement, weapons];
        let control_owner = OwnerId::new("control").unwrap();
        let roster = BTreeSet::from([owner(), control_owner.clone()]);
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let (definitions, action) = action_definition();
        let definitions = bind_definitions(definitions, &readiness);
        let mut guard_builder =
            ServerGuardSetBuilder::new(RevisionSet::default(), readiness.clone());
        guard_builder
            .add_definition(action.clone(), &definitions)
            .unwrap();
        let mut provider_builder = ActionProviderRegistryBuilder::new();
        provider_builder
            .register(
                owner(),
                action.clone(),
                Arc::new(ChannelOfferProvider {
                    guards: guard_builder.finish().unwrap(),
                    channels: channels.clone(),
                }),
            )
            .unwrap();
        let providers = provider_builder
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness.clone())
            .unwrap();
        let keys = crate::offer::OfferKeyRing::new(
            crate::offer::OfferSecret::new(1, [0x44; 32]).unwrap(),
            [],
        )
        .unwrap();
        let mut engine = crate::engine::Engine::new(World::new_minimal(), crate::Registry::new());
        let mut authority_builder = AuthorityGuardValidatorRegistryBuilder::new();
        authority_builder
            .register(
                control_owner.clone(),
                AuthorityGuardKind::ControlLease,
                Arc::new(CurrentLeaseAuthority { epoch: 29 }),
            )
            .unwrap();
        engine.registry.authority_guard_validators = Some(
            authority_builder
                .freeze_for_owners_with_readiness(&roster, readiness.clone())
                .unwrap(),
        );

        // A required-channel offer is absent until one exact grant supplies
        // the complete set.
        assert!(
            providers
                .project_legal_actions(&engine, &operated_query(), &keys)
                .unwrap()
                .is_empty()
        );
        let mut rogue = test_channel_grant("rogue", 99, 1, &channels);
        rogue.owner = SmolStr::new_static("uncomposed");
        engine.world.publish_command_subject(rogue);
        assert!(matches!(
            providers.project_legal_actions(&engine, &operated_query(), &keys),
            Err(crate::offer::LegalActionProjectionError::ChannelGrant { source, .. })
                if matches!(
                    *source,
                    GuardSetBuildError::GrantOwnerUnavailable { .. }
                )
        ));
        engine
            .world
            .retract_command_subjects("uncomposed", Some(crate::GOD));
        let mut grant = test_channel_grant("cockpit", 50, 13, &channels);
        grant.owner = SmolStr::new(control_owner.as_str());
        engine.world.publish_command_subject(grant.clone());
        let offers = providers
            .project_legal_actions(&engine, &operated_query(), &keys)
            .unwrap();
        let projected = offers.projected().into_iter().next().unwrap();
        let public_json = serde_json::to_string(&projected).unwrap();
        assert!(!public_json.contains("cockpit"));
        assert!(!public_json.contains("epoch"));
        assert!(!public_json.contains("priority"));

        let request = request(0x45);
        let resolved = offers
            .resolve(
                &keys,
                crate::offer::OfferExecutionContext {
                    request,
                    principal: PrincipalId::Object { id: crate::GOD },
                    actor: crate::GOD,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal: PrincipalId::Object { id: crate::GOD },
                            owner: owner(),
                            namespace: CanonicalId::new("operate").unwrap(),
                        },
                        1,
                        IdempotencyKey::Request(request),
                    ),
                    viewer_scope: operated_query().viewer_scope(),
                    now: None,
                },
                crate::offer::OfferSubmission {
                    offer: projected.offer(),
                    subject: projected.subject().clone(),
                    targets: Vec::new(),
                    parameters: json!({}),
                },
            )
            .unwrap();
        let (_, _, _, guards) = resolved.into_resolution_parts();
        let grant_guards = guards
            .authority()
            .iter()
            .filter(|guard| {
                matches!(
                    guard,
                    AuthorityGuard::ChannelGrant { provider, .. }
                        if provider.as_str() == control_owner.as_str()
                )
            })
            .count();
        assert_eq!(grant_guards, channels.len());
        let lease_guards = guards
            .authority()
            .iter()
            .filter(|guard| {
                matches!(
                    guard,
                    AuthorityGuard::ControlLease {
                        provider,
                        epoch: 29,
                        ..
                    } if provider.as_str() == control_owner.as_str()
                )
            })
            .count();
        assert_eq!(lease_guards, channels.len());

        let revisions = RevisionValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, readiness.clone())
            .unwrap();
        let authority = engine.registry.authority_guard_validators().unwrap();
        assert!(
            guards
                .revalidate_unregistered(&engine.world, &revisions, authority, &definitions)
                .unwrap()
                .is_current()
        );

        let mut changed_grant = grant.clone();
        changed_grant.epoch += 1;
        engine.world.publish_command_subject(changed_grant);
        let stale = guards
            .revalidate_unregistered(&engine.world, &revisions, authority, &definitions)
            .unwrap();
        assert_eq!(stale.stale_authority().len(), channels.len());
    }

    struct ConstantRevision(Option<u64>);

    impl RevisionValidator for ConstantRevision {
        fn current_generation(
            &self,
            _context: &RevisionValidatorContext<'_>,
            _key: &RevisionKey,
        ) -> Result<Option<u64>, ActionError> {
            Ok(self.0)
        }
    }

    struct ConstantAuthority(bool);

    impl AuthorityGuardValidator for ConstantAuthority {
        fn is_current(
            &self,
            _context: &AuthorityGuardContext<'_>,
            _guard: &AuthorityGuard,
        ) -> Result<bool, ActionError> {
            Ok(self.0)
        }
    }

    fn freeze_test_owner(readiness: &MechanicalOwnerReadiness, code: &str) {
        readiness
            .freeze(
                &owner(),
                IncidentId(90),
                RecoveryReason::IntegrityViolation {
                    code: CanonicalId::new(code).unwrap(),
                },
            )
            .unwrap();
    }

    struct FreezeDuringRevision(MechanicalOwnerReadiness);

    impl RevisionValidator for FreezeDuringRevision {
        fn current_generation(
            &self,
            _context: &RevisionValidatorContext<'_>,
            _key: &RevisionKey,
        ) -> Result<Option<u64>, ActionError> {
            freeze_test_owner(&self.0, "revision_race");
            Ok(Some(1))
        }
    }

    struct FreezeDuringAuthority(MechanicalOwnerReadiness);

    impl AuthorityGuardValidator for FreezeDuringAuthority {
        fn is_current(
            &self,
            _context: &AuthorityGuardContext<'_>,
            _guard: &AuthorityGuard,
        ) -> Result<bool, ActionError> {
            freeze_test_owner(&self.0, "authority_race");
            Ok(true)
        }
    }

    fn revision_key(kind: &str, object: u32) -> RevisionKey {
        RevisionKey {
            owner: owner(),
            kind: RevisionKind::new(CanonicalId::new(kind).unwrap()),
            subject: SubjectKey::Object { id: ObjId(object) },
            facet: None,
        }
    }

    #[test]
    fn validator_registries_are_owner_exact_duplicate_free_and_stably_fingerprinted() {
        let roster = BTreeSet::from([owner()]);
        let alpha = RevisionKind::new(CanonicalId::new("alpha").unwrap());
        let beta = RevisionKind::new(CanonicalId::new("beta").unwrap());

        let mut duplicate = RevisionValidatorRegistryBuilder::new();
        duplicate
            .register(owner(), alpha.clone(), Arc::new(ConstantRevision(Some(1))))
            .unwrap();
        assert!(matches!(
            duplicate.register(owner(), alpha.clone(), Arc::new(ConstantRevision(Some(1)))),
            Err(RevisionValidatorRegistryError::Duplicate { .. })
        ));
        assert!(matches!(
            duplicate.freeze_for_owners(&BTreeSet::new()),
            Err(RevisionValidatorRegistryError::UnknownOwner { .. })
        ));

        let mut forward = RevisionValidatorRegistryBuilder::new();
        forward
            .register(owner(), alpha.clone(), Arc::new(ConstantRevision(Some(1))))
            .unwrap();
        forward
            .register(owner(), beta.clone(), Arc::new(ConstantRevision(Some(1))))
            .unwrap();
        let mut reverse = RevisionValidatorRegistryBuilder::new();
        reverse
            .register(owner(), beta, Arc::new(ConstantRevision(Some(1))))
            .unwrap();
        reverse
            .register(owner(), alpha, Arc::new(ConstantRevision(Some(1))))
            .unwrap();
        assert_eq!(
            forward.freeze_for_owners(&roster).unwrap().fingerprint(),
            reverse.freeze_for_owners(&roster).unwrap().fingerprint()
        );

        let mut authority = AuthorityGuardValidatorRegistryBuilder::new();
        authority
            .register(
                owner(),
                AuthorityGuardKind::Capability,
                Arc::new(ConstantAuthority(true)),
            )
            .unwrap();
        assert!(matches!(
            authority.register(
                owner(),
                AuthorityGuardKind::Capability,
                Arc::new(ConstantAuthority(true)),
            ),
            Err(AuthorityGuardValidatorRegistryError::Duplicate { .. })
        ));
        assert!(matches!(
            authority.freeze_for_owners(&BTreeSet::new()),
            Err(AuthorityGuardValidatorRegistryError::UnknownOwner { .. })
        ));
        assert!(matches!(
            AuthorityGuardValidatorRegistryBuilder::new().register(
                owner(),
                AuthorityGuardKind::Definition,
                Arc::new(ConstantAuthority(true)),
            ),
            Err(AuthorityGuardValidatorRegistryError::ReservedKernelKind {
                kind: AuthorityGuardKind::Definition
            })
        ));
        assert!(matches!(
            AuthorityGuardValidatorRegistryBuilder::new().register(
                owner(),
                AuthorityGuardKind::ChannelGrant,
                Arc::new(ConstantAuthority(true)),
            ),
            Err(AuthorityGuardValidatorRegistryError::ReservedKernelKind {
                kind: AuthorityGuardKind::ChannelGrant
            })
        ));

        let build_authority = |reverse: bool| {
            let mut builder = AuthorityGuardValidatorRegistryBuilder::new();
            let kinds = if reverse {
                [AuthorityGuardKind::Window, AuthorityGuardKind::Capability]
            } else {
                [AuthorityGuardKind::Capability, AuthorityGuardKind::Window]
            };
            for kind in kinds {
                builder
                    .register(owner(), kind, Arc::new(ConstantAuthority(true)))
                    .unwrap();
            }
            builder.freeze_for_owners(&roster).unwrap()
        };
        assert_eq!(
            build_authority(false).fingerprint(),
            build_authority(true).fingerprint()
        );
    }

    #[test]
    fn frozen_owner_disables_all_executable_action_registry_lookups() {
        let (definitions, action) = action_definition();
        let roster = owner_roster();
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let definitions = bind_definitions(definitions, &readiness);

        let revision_key = revision_key("unit", 1);
        let mut revision_builder = RevisionValidatorRegistryBuilder::new();
        revision_builder
            .register(
                owner(),
                revision_key.kind.clone(),
                Arc::new(ConstantRevision(Some(1))),
            )
            .unwrap();
        let revision_validators = revision_builder
            .freeze_for_owners_with_readiness(&roster, readiness.clone())
            .unwrap();

        let authority_guard = AuthorityGuard::Capability {
            provider: owner(),
            actor: ObjId(7),
            capability: CanonicalId::new("pilot").unwrap(),
            generation: 1,
        };
        let mut authority_builder = AuthorityGuardValidatorRegistryBuilder::new();
        authority_builder
            .register(
                owner(),
                AuthorityGuardKind::Capability,
                Arc::new(ConstantAuthority(true)),
            )
            .unwrap();
        let authority_validators = authority_builder
            .freeze_for_owners_with_readiness(&roster, readiness.clone())
            .unwrap();

        let mut provider_builder = ActionProviderRegistryBuilder::new();
        provider_builder
            .register(owner(), action.clone(), Arc::new(Accept))
            .unwrap();
        let action_providers = provider_builder
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness.clone())
            .unwrap();

        assert!(revision_validators.get_for_key(&revision_key).is_ok());
        assert!(authority_validators.get_for_guard(&authority_guard).is_ok());
        assert!(action_providers.get_exact(&action).is_ok());

        readiness
            .freeze(
                &owner(),
                IncidentId(78),
                RecoveryReason::IntegrityViolation {
                    code: CanonicalId::new("test_registry_freeze").unwrap(),
                },
            )
            .unwrap();

        assert!(matches!(
            revision_validators.get_for_key(&revision_key),
            Err(RevisionValidatorLookupError::OwnerNotReady { .. })
        ));
        assert!(matches!(
            authority_validators.get_for_guard(&authority_guard),
            Err(AuthorityGuardValidatorLookupError::OwnerNotReady { .. })
        ));
        assert!(matches!(
            action_providers.get_exact(&action),
            Err(ActionProviderLookupError::OwnerNotReady { .. })
        ));
    }

    #[test]
    fn final_guard_revalidation_reports_stale_state_in_canonical_order() {
        let roster = owner_roster();
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let (planning_definitions, action) = action_definition();
        let planning_definitions = bind_definitions(planning_definitions, &readiness);
        let revision_a = revision_key("unit", 1);
        let revision_b = revision_key("unit", 2);
        let revisions = RevisionSet::from_stamps([
            RevisionStamp {
                key: revision_b.clone(),
                generation: 9,
            },
            RevisionStamp {
                key: revision_a.clone(),
                generation: 7,
            },
        ])
        .unwrap();
        let mut guards = ServerGuardSetBuilder::new(revisions, readiness.clone());
        guards
            .add(AuthorityGuard::Capability {
                provider: owner(),
                actor: ObjId(7),
                capability: CanonicalId::new("pilot").unwrap(),
                generation: 3,
            })
            .unwrap();
        guards
            .add_definition(action, &planning_definitions)
            .unwrap();
        let guards = guards.finish().unwrap();

        let mut revision_builders = RevisionValidatorRegistryBuilder::new();
        revision_builders
            .register(
                owner(),
                RevisionKind::new(CanonicalId::new("unit").unwrap()),
                Arc::new(ConstantRevision(Some(8))),
            )
            .unwrap();
        let revision_validators = revision_builders
            .freeze_for_owners_with_readiness(&roster, readiness.clone())
            .unwrap();
        let mut authority_builders = AuthorityGuardValidatorRegistryBuilder::new();
        authority_builders
            .register(
                owner(),
                AuthorityGuardKind::Capability,
                Arc::new(ConstantAuthority(false)),
            )
            .unwrap();
        let authority_validators = authority_builders
            .freeze_for_owners_with_readiness(&roster, readiness.clone())
            .unwrap();
        let empty_definitions = bind_definitions(
            DefinitionRegistryBuilder::new().freeze().unwrap(),
            &readiness,
        );
        let result = guards
            .revalidate_unregistered(
                &World::new_minimal(),
                &revision_validators,
                &authority_validators,
                &empty_definitions,
            )
            .unwrap();
        assert!(!result.is_current());
        assert_eq!(
            result
                .stale_revisions()
                .iter()
                .map(|stale| stale.key.clone())
                .collect::<Vec<_>>(),
            vec![revision_a, revision_b]
        );
        assert_eq!(result.stale_revisions()[0].actual, Some(8));
        assert_eq!(result.stale_authority().len(), 2);
        assert!(matches!(
            result.stale_authority()[0],
            AuthorityGuard::Capability { .. }
        ));
        assert!(matches!(
            result.stale_authority()[1],
            AuthorityGuard::Definition(_)
        ));
    }

    #[test]
    fn guard_revalidation_fails_closed_without_exact_owner_validators() {
        let other = OwnerId::new("other").unwrap();
        let roster = BTreeSet::from([owner(), other.clone()]);
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: revision_key("unit", 1),
            generation: 1,
        }])
        .unwrap();
        let guards = ServerGuardSetBuilder::new(revisions, readiness.clone())
            .finish()
            .unwrap();
        let empty_revisions = RevisionValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, readiness.clone())
            .unwrap();
        let empty_authority = AuthorityGuardValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, readiness.clone())
            .unwrap();
        let empty_definitions = bind_definitions(
            DefinitionRegistryBuilder::new().freeze().unwrap(),
            &readiness,
        );
        assert!(matches!(
            guards.revalidate_unregistered(
                &World::new_minimal(),
                &empty_revisions,
                &empty_authority,
                &empty_definitions,
            ),
            Err(GuardRevalidationError::RevisionLookup(_))
        ));

        let mut authority_guard = ServerGuardSetBuilder::new(RevisionSet::default(), readiness);
        authority_guard
            .add(AuthorityGuard::Capability {
                provider: other,
                actor: ObjId(1),
                capability: CanonicalId::new("pilot").unwrap(),
                generation: 1,
            })
            .unwrap();
        let authority_guard = authority_guard.finish().unwrap();
        assert!(matches!(
            authority_guard.revalidate_unregistered(
                &World::new_minimal(),
                &empty_revisions,
                &empty_authority,
                &empty_definitions,
            ),
            Err(GuardRevalidationError::AuthorityLookup(_))
        ));
    }

    #[test]
    fn definition_owner_freeze_is_unavailable_not_stale() {
        let (mut definitions, action) = action_definition();
        let roster = BTreeSet::from([owner()]);
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        definitions.bind_owner_readiness(readiness.clone());

        let mut builder = ServerGuardSetBuilder::new(RevisionSet::default(), readiness.clone());
        builder
            .add_definition(action.clone(), &definitions)
            .unwrap();
        let guards = builder.finish().unwrap();

        readiness
            .freeze(
                &owner(),
                IncidentId(77),
                RecoveryReason::IntegrityViolation {
                    code: CanonicalId::new("test_owner_freeze").unwrap(),
                },
            )
            .unwrap();

        let empty_revisions = RevisionValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, readiness.clone())
            .unwrap();
        let empty_authority = AuthorityGuardValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, readiness)
            .unwrap();
        let error = guards
            .revalidate_unregistered(
                &World::new_minimal(),
                &empty_revisions,
                &empty_authority,
                &definitions,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            GuardRevalidationError::DefinitionUnavailable { reference, source }
                if *reference == action
                    && matches!(*source, DefinitionLookupError::OwnerNotReady { .. })
        ));
    }

    #[test]
    fn guard_sets_reject_mixed_or_unbound_registry_compositions() {
        let roster = owner_roster();
        let first_readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let second_readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let (first_definitions, action) = action_definition();
        let first_definitions = bind_definitions(first_definitions, &first_readiness);
        let (second_definitions, _) = action_definition();
        let second_definitions = bind_definitions(second_definitions, &second_readiness);

        let mut builder =
            ServerGuardSetBuilder::new(RevisionSet::default(), first_readiness.clone());
        builder
            .add_definition(action.clone(), &first_definitions)
            .unwrap();
        let guards = builder.finish().unwrap();

        let first_revisions = RevisionValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, first_readiness.clone())
            .unwrap();
        let first_authority = AuthorityGuardValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, first_readiness.clone())
            .unwrap();
        assert!(matches!(
            guards.revalidate_unregistered(
                &World::new_minimal(),
                &first_revisions,
                &first_authority,
                &second_definitions,
            ),
            Err(GuardRevalidationError::RegistryCompositionMismatch {
                component: "definitions"
            })
        ));

        let second_revisions = RevisionValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, second_readiness.clone())
            .unwrap();
        let second_authority = AuthorityGuardValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, second_readiness)
            .unwrap();
        assert!(matches!(
            guards.revalidate_unregistered(
                &World::new_minimal(),
                &second_revisions,
                &second_authority,
                &first_definitions,
            ),
            Err(GuardRevalidationError::RegistryCompositionMismatch {
                component: "revision validators"
            })
        ));

        let mut wrong_builder =
            ServerGuardSetBuilder::new(RevisionSet::default(), first_readiness.clone());
        assert_eq!(
            wrong_builder
                .add_definition(action.clone(), &second_definitions)
                .unwrap_err(),
            GuardSetBuildError::RegistryCompositionMismatch
        );
        let unbound = action_definition().0;
        assert_eq!(
            wrong_builder.add_definition(action, &unbound).unwrap_err(),
            GuardSetBuildError::RegistryCompositionMismatch
        );
    }

    #[test]
    fn guard_validator_results_are_discarded_when_owner_freezes_during_callback() {
        let roster = owner_roster();
        let revision_readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let revision = revision_key("unit", 1);
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: revision.clone(),
            generation: 1,
        }])
        .unwrap();
        let guards = ServerGuardSetBuilder::new(revisions, revision_readiness.clone())
            .finish()
            .unwrap();
        let mut revision_builder = RevisionValidatorRegistryBuilder::new();
        revision_builder
            .register(
                owner(),
                revision.kind.clone(),
                Arc::new(FreezeDuringRevision(revision_readiness.clone())),
            )
            .unwrap();
        let revision_validators = revision_builder
            .freeze_for_owners_with_readiness(&roster, revision_readiness.clone())
            .unwrap();
        let authority_validators = AuthorityGuardValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, revision_readiness.clone())
            .unwrap();
        let definitions = bind_definitions(
            DefinitionRegistryBuilder::new().freeze().unwrap(),
            &revision_readiness,
        );
        assert!(matches!(
            guards.revalidate_unregistered(
                &World::new_minimal(),
                &revision_validators,
                &authority_validators,
                &definitions,
            ),
            Err(GuardRevalidationError::RevisionLookup(
                RevisionValidatorLookupError::OwnerNotReady { .. }
            ))
        ));

        let authority_readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let authority_guard = AuthorityGuard::Capability {
            provider: owner(),
            actor: ObjId(7),
            capability: CanonicalId::new("pilot").unwrap(),
            generation: 1,
        };
        let mut guard_builder =
            ServerGuardSetBuilder::new(RevisionSet::default(), authority_readiness.clone());
        guard_builder.add(authority_guard).unwrap();
        let guards = guard_builder.finish().unwrap();
        let revision_validators = RevisionValidatorRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&roster, authority_readiness.clone())
            .unwrap();
        let mut authority_builder = AuthorityGuardValidatorRegistryBuilder::new();
        authority_builder
            .register(
                owner(),
                AuthorityGuardKind::Capability,
                Arc::new(FreezeDuringAuthority(authority_readiness.clone())),
            )
            .unwrap();
        let authority_validators = authority_builder
            .freeze_for_owners_with_readiness(&roster, authority_readiness.clone())
            .unwrap();
        let definitions = bind_definitions(
            DefinitionRegistryBuilder::new().freeze().unwrap(),
            &authority_readiness,
        );
        assert!(matches!(
            guards.revalidate_unregistered(
                &World::new_minimal(),
                &revision_validators,
                &authority_validators,
                &definitions,
            ),
            Err(GuardRevalidationError::AuthorityLookup(
                AuthorityGuardValidatorLookupError::OwnerNotReady { .. }
            ))
        ));
    }

    #[test]
    fn receipt_and_lifecycle_cannot_claim_unprepared_success() {
        let uid = request(1);
        let envelope = IngressEnvelope::new(
            uid,
            principal(),
            IngressSurfaceVersion::new(CanonicalId::new("https.json").unwrap(), 1).unwrap(),
            IngressPayload::structured(json!({"action": "wave"})).unwrap(),
            idempotency(uid),
        )
        .unwrap();
        let receipt = ActionReceiptMetadata::ingress(&envelope);
        assert_eq!(
            ActionAttemptMetadata::new(ActionLifecycle::Prepared, receipt.clone(), None)
                .unwrap_err(),
            ActionAttemptMetadataError::PreparedWithoutPlan
        );
        let failure = ActionError::new(ActionErrorCode::Busy, true, "retry later").unwrap();
        assert!(
            ActionAttemptMetadata::new(ActionLifecycle::Rejected, receipt, Some(failure)).is_ok()
        );
    }

    #[test]
    fn resolution_custody_requires_context_and_transcript_to_pair() {
        assert!(validate_resolution_custody(false, false).is_ok());
        assert!(validate_resolution_custody(true, true).is_ok());
        assert!(matches!(
            validate_resolution_custody(true, false),
            Err(ActionProviderPrepareError::UnrecordedResolution)
        ));
        assert!(matches!(
            validate_resolution_custody(false, true),
            Err(ActionProviderPrepareError::ResolutionOutsideContext)
        ));
    }

    struct Accept;

    impl ActionProvider for Accept {
        fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
            Ok(())
        }
    }

    struct FreezeDuringAction(MechanicalOwnerReadiness);

    impl ActionProvider for FreezeDuringAction {
        fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
            freeze_test_owner(&self.0, "action_race");
            Ok(())
        }
    }

    struct RevisionKeysProvider(Vec<RevisionKey>);

    impl ActionProvider for RevisionKeysProvider {
        fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
            Ok(())
        }

        fn resolved_revision_keys(
            &self,
            _resolved: &ResolvedActionInvocation,
        ) -> Result<Vec<RevisionKey>, ActionError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn provider_registry_requires_owner_exact_action_and_unique_locator() {
        let (definitions, action) = action_definition();
        let roster = owner_roster();
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let definitions = bind_definitions(definitions, &readiness);
        let mut builder = ActionProviderRegistryBuilder::new();
        builder
            .register(owner(), action.clone(), Arc::new(Accept))
            .unwrap();
        assert!(matches!(
            builder.register(owner(), action.clone(), Arc::new(Accept)),
            Err(ActionProviderRegistryError::Duplicate { .. })
        ));
        let registry = builder
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness)
            .unwrap();
        let entry = registry.get_exact(&action).unwrap();
        assert_eq!(entry.owner(), &owner());
        registry
            .validate_invocation(&action, &invocation(request(1)))
            .unwrap();

        let wrong_digest = DefinitionRef::new(
            action.key().clone(),
            action.version(),
            DefinitionDigest::from_bytes([0xee; 32]),
        )
        .unwrap();
        assert!(matches!(
            registry.get_exact(&wrong_digest),
            Err(ActionProviderLookupError::DigestMismatch { .. })
        ));
    }

    #[test]
    fn provider_freeze_rejects_unknown_owner_and_definition_digest_drift() {
        let (definitions, action) = action_definition();
        let empty_roster = BTreeSet::new();
        let empty_readiness = MechanicalOwnerReadiness::for_owners(&empty_roster);
        let definitions = bind_definitions(definitions, &empty_readiness);
        let mut unknown_owner = ActionProviderRegistryBuilder::new();
        unknown_owner
            .register(owner(), action.clone(), Arc::new(Accept))
            .unwrap();
        assert!(matches!(
            unknown_owner.freeze_for_owners_with_readiness(
                &definitions,
                &empty_roster,
                empty_readiness,
            ),
            Err(ActionProviderRegistryError::UnknownOwner { .. })
        ));

        let (definitions, action) = action_definition();
        let roster = owner_roster();
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let definitions = bind_definitions(definitions, &readiness);
        let wrong_digest = DefinitionRef::new(
            action.key().clone(),
            action.version(),
            DefinitionDigest::from_bytes([0xdd; 32]),
        )
        .unwrap();
        let mut drift = ActionProviderRegistryBuilder::new();
        drift
            .register(owner(), wrong_digest, Arc::new(Accept))
            .unwrap();
        let error = drift
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness)
            .unwrap_err();
        assert!(matches!(
            error,
            ActionProviderRegistryError::MissingActionDefinition { source, .. }
                if matches!(*source, DefinitionLookupError::DigestMismatch { .. })
        ));
    }

    #[test]
    fn provider_result_is_discarded_when_owner_freezes_during_callback() {
        let roster = owner_roster();
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let (definitions, action) = action_definition();
        let definitions = bind_definitions(definitions, &readiness);
        let mut builder = ActionProviderRegistryBuilder::new();
        builder
            .register(
                owner(),
                action.clone(),
                Arc::new(FreezeDuringAction(readiness.clone())),
            )
            .unwrap();
        let registry = builder
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness)
            .unwrap();

        assert!(matches!(
            registry.validate_invocation(&action, &invocation(request(1))),
            Err(ActionProviderDispatchError::Lookup(
                ActionProviderLookupError::OwnerNotReady { .. }
            ))
        ));
    }

    #[test]
    fn resolved_revision_coordinates_are_bounded_unique_and_bridge_gated() {
        fn dispatch(
            keys: Vec<RevisionKey>,
        ) -> Result<Vec<RevisionKey>, ActionProviderDispatchError> {
            let (definitions, action) = action_definition();
            let mut roster = owner_roster();
            roster.extend(keys.iter().map(|key| key.owner.clone()));
            let readiness = MechanicalOwnerReadiness::for_owners(&roster);
            let definitions = bind_definitions(definitions, &readiness);
            let bridges = crate::mechanics::BridgeRegistryBuilder::new()
                .freeze_for_owners_with_readiness(&definitions, &roster, readiness.clone())
                .unwrap();
            let mut builder = ActionProviderRegistryBuilder::new();
            builder
                .register(
                    owner(),
                    action.clone(),
                    Arc::new(RevisionKeysProvider(keys)),
                )
                .unwrap();
            let registry = builder
                .freeze_for_owners_with_readiness(&definitions, &roster, readiness)
                .unwrap();
            let invocation = invocation(request(1));
            let resolved = ResolvedActionInvocation::new(
                invocation,
                SubjectKey::Object { id: ObjId(7) },
                vec![SubjectKey::Object { id: ObjId(8) }],
            )
            .unwrap();
            registry
                .resolved_revision_keys(&action, &resolved, &definitions, &bridges)
                .map(|(keys, _)| keys)
        }

        let duplicate = revision_key("unit", 7);
        assert_eq!(
            dispatch(vec![duplicate.clone(), duplicate]).unwrap_err(),
            ActionProviderDispatchError::DuplicateRevisionKey
        );

        let mut foreign = revision_key("unit", 7);
        foreign.owner = OwnerId::new("foreign").unwrap();
        assert!(matches!(
            dispatch(vec![foreign]),
            Err(ActionProviderDispatchError::RevisionBridge { source, .. })
                if matches!(*source, BridgeLookupError::Missing { .. })
        ));

        let foreign_owner = OwnerId::new("foreign").unwrap();
        let mut definition_builder = DefinitionRegistryBuilder::new();
        let action = definition_builder
            .register(
                owner(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("cross_owner_fire").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let bridge_definition = definition_builder
            .register(
                owner(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("bridge").unwrap(),
                    DefinitionId::new("toy_to_foreign_fire").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let roster = BTreeSet::from([owner(), foreign_owner.clone()]);
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let definitions = bind_definitions(definition_builder.freeze().unwrap(), &readiness);
        let mut bridge_builder = BridgeRegistryBuilder::new();
        bridge_builder
            .register(
                owner(),
                BridgeDescriptor::new(
                    bridge_definition.clone(),
                    owner(),
                    foreign_owner.clone(),
                    action.clone(),
                ),
            )
            .unwrap();
        let bridges = bridge_builder
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness.clone())
            .unwrap();
        let foreign_key = RevisionKey {
            owner: foreign_owner,
            kind: RevisionKind::new(CanonicalId::new("position").unwrap()),
            subject: SubjectKey::Object { id: ObjId(8) },
            facet: None,
        };
        let mut provider_builder = ActionProviderRegistryBuilder::new();
        provider_builder
            .register(
                owner(),
                action.clone(),
                Arc::new(RevisionKeysProvider(vec![foreign_key.clone()])),
            )
            .unwrap();
        let providers = provider_builder
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness)
            .unwrap();
        let resolved = ResolvedActionInvocation::new(
            invocation(request(2)),
            SubjectKey::Object { id: ObjId(7) },
            vec![SubjectKey::Object { id: ObjId(8) }],
        )
        .unwrap();
        let (keys, bridge_definitions) = providers
            .resolved_revision_keys(&action, &resolved, &definitions, &bridges)
            .unwrap();
        assert_eq!(keys, vec![foreign_key]);
        assert_eq!(bridge_definitions, vec![bridge_definition]);

        let excessive = (0..=MAX_ACTION_REVISIONS)
            .map(|object| revision_key("unit", u32::try_from(object).unwrap()))
            .collect();
        assert_eq!(
            dispatch(excessive).unwrap_err(),
            ActionProviderDispatchError::RevisionLimit {
                max: MAX_ACTION_REVISIONS,
            }
        );
    }

    #[test]
    fn provider_fingerprint_is_invariant_under_registration_order() {
        let (definitions, first, second) = two_action_definitions();
        let roster = BTreeSet::from([owner()]);
        let mut forward = ActionProviderRegistryBuilder::new();
        forward
            .register(owner(), first.clone(), Arc::new(Accept))
            .unwrap();
        forward
            .register(owner(), second.clone(), Arc::new(Accept))
            .unwrap();
        let mut reverse = ActionProviderRegistryBuilder::new();
        reverse
            .register(owner(), second.clone(), Arc::new(Accept))
            .unwrap();
        reverse
            .register(owner(), first.clone(), Arc::new(Accept))
            .unwrap();
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let definitions = bind_definitions(definitions, &readiness);
        let forward = forward
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness.clone())
            .unwrap();
        let reverse = reverse
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness)
            .unwrap();
        assert_eq!(forward.fingerprint(), reverse.fingerprint());
        assert_eq!(
            forward.get_exact(&first).unwrap().registration_ordinal(),
            reverse.get_exact(&first).unwrap().registration_ordinal()
        );
        assert!(
            forward
                .get_exact(&first)
                .unwrap()
                .registration_ordinal()
                .get()
                < forward
                    .get_exact(&second)
                    .unwrap()
                    .registration_ordinal()
                    .get()
        );
    }

    #[test]
    fn prepared_causality_is_bounded_and_parented_within_one_request() {
        let root = request(1);
        let parent = PreparedEventId::new(root, 0).unwrap();
        let child = PreparedCausalRef::new(root, Some(parent), 1).unwrap();
        assert_eq!(child.event_id().ordinal(), 1);
        assert_eq!(
            PreparedCausalRef::new(request(2), Some(parent), 1).unwrap_err(),
            CausalMetadataError::ParentRootMismatch
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn prepared_trigger(
        readiness: &MechanicalOwnerReadiness,
        root: RequestUid,
        source_ordinal: u32,
        handler: &str,
        target: u32,
        phase: u16,
        provider_ordinal: u32,
        payload: Value,
    ) -> PreparedTriggerDraft {
        PreparedTriggerDraft {
            source_event: PreparedEventId::new(root, source_ordinal).unwrap(),
            handler: TriggerHandlerId {
                owner: owner(),
                id: CanonicalId::new(handler).unwrap(),
            },
            target: SubjectKey::Object { id: ObjId(target) },
            phase,
            provider_registration_ordinal: ProviderRegistrationOrdinal {
                owner: owner(),
                ordinal: provider_ordinal,
                owner_readiness: readiness.clone(),
            },
            payload,
        }
    }

    #[test]
    fn causal_trigger_order_is_invariant_under_insertion_order() {
        let root = request(1);
        let readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let work = vec![
            prepared_trigger(&readiness, root, 1, "late", 9, 1, 0, json!({"n": 3})),
            prepared_trigger(
                &readiness,
                root,
                0,
                "second_provider",
                4,
                0,
                2,
                json!({"n": 2}),
            ),
            prepared_trigger(
                &readiness,
                root,
                0,
                "first_provider",
                7,
                0,
                1,
                json!({"n": 1}),
            ),
        ];
        let mut forward = CausalTriggerQueue::new(root, readiness.clone());
        forward.allocate_root_event().unwrap();
        forward.allocate_root_event().unwrap();
        for trigger in &work {
            forward.enqueue(trigger.clone()).unwrap();
        }
        let mut reverse = CausalTriggerQueue::new(root, readiness);
        reverse.allocate_root_event().unwrap();
        reverse.allocate_root_event().unwrap();
        for trigger in work.iter().rev() {
            reverse.enqueue(trigger.clone()).unwrap();
        }
        let drain = |queue: &mut CausalTriggerQueue| {
            let mut identities = Vec::new();
            while let Some(work) = queue.pop_next().unwrap() {
                identities.push(work.trigger().identity().clone());
                queue.complete_trigger(work).unwrap();
            }
            identities
        };
        assert_eq!(drain(&mut forward), drain(&mut reverse));
    }

    #[test]
    fn cascade_cannot_finish_after_dequeued_work_is_discarded() {
        let root = request(1);
        let readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let mut queue = CausalTriggerQueue::new(root, readiness.clone());
        queue.allocate_root_event().unwrap();
        queue
            .enqueue(prepared_trigger(
                &readiness,
                root,
                0,
                "work",
                1,
                0,
                0,
                json!({}),
            ))
            .unwrap();

        let discarded = queue.pop_next().unwrap().unwrap();
        let identity = discarded.trigger().identity().clone();
        drop(discarded);

        assert_eq!(
            queue.finish(),
            Err(CascadeError::UncompletedTrigger {
                identity: Box::new(identity),
            })
        );
    }

    #[test]
    fn cascade_completes_only_after_returning_each_work_token() {
        let root = request(1);
        let readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let mut queue = CausalTriggerQueue::new(root, readiness.clone());
        queue.allocate_root_event().unwrap();
        queue
            .enqueue(prepared_trigger(
                &readiness,
                root,
                0,
                "first",
                1,
                0,
                0,
                json!({}),
            ))
            .unwrap();
        queue
            .enqueue(prepared_trigger(
                &readiness,
                root,
                0,
                "second",
                2,
                0,
                0,
                json!({}),
            ))
            .unwrap();

        let first = queue.pop_next().unwrap().unwrap();
        let first_identity = first.trigger().identity().clone();
        assert!(matches!(
            queue.pop_next(),
            Err(CascadeError::TriggerAlreadyActive { identity })
                if *identity == first_identity
        ));
        assert!(matches!(
            queue.complete_trigger(first),
            Err(CascadeError::CascadePoisoned { cause })
                if matches!(*cause, CascadeError::TriggerAlreadyActive { .. })
        ));

        let mut successful = CausalTriggerQueue::new(request(2), readiness.clone());
        successful.allocate_root_event().unwrap();
        successful
            .enqueue(prepared_trigger(
                &readiness,
                request(2),
                0,
                "work",
                1,
                0,
                0,
                json!({}),
            ))
            .unwrap();
        let work = successful.pop_next().unwrap().unwrap();
        successful.complete_trigger(work).unwrap();
        assert!(successful.pop_next().unwrap().is_none());
        successful.finish().unwrap();
    }

    #[test]
    fn cascade_duplicate_suppression_is_exact_and_does_not_consume_budget() {
        let root = request(1);
        let readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let trigger =
            prepared_trigger(&readiness, root, 0, "handler", 7, 0, 1, json!({"value": 1}));
        let mut queue = CausalTriggerQueue::new(root, readiness.clone());
        queue.allocate_root_event().unwrap();
        assert_eq!(
            queue.enqueue(trigger.clone()).unwrap(),
            TriggerEnqueue::Queued
        );
        let bytes = queue.encoded_bytes();
        assert_eq!(
            queue.enqueue(trigger).unwrap(),
            TriggerEnqueue::DuplicateSuppressed
        );
        assert_eq!(queue.trigger_count(), 1);
        assert_eq!(queue.encoded_bytes(), bytes);

        let conflicting =
            prepared_trigger(&readiness, root, 0, "handler", 7, 0, 1, json!({"value": 2}));
        assert!(matches!(
            queue.enqueue(conflicting),
            Err(CascadeError::ConflictingDuplicate { .. })
        ));
        assert_eq!(queue.trigger_count(), 1);
        assert_eq!(queue.pending_len(), 1);
    }

    #[test]
    fn cascade_rejects_cross_composition_tokens_and_stays_poisoned() {
        let root = request(1);
        let first_readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let second_readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let mut queue = CausalTriggerQueue::new(root, first_readiness);
        queue.allocate_root_event().unwrap();

        assert_eq!(
            queue
                .enqueue(prepared_trigger(
                    &second_readiness,
                    root,
                    0,
                    "foreign",
                    1,
                    0,
                    0,
                    json!({}),
                ))
                .unwrap_err(),
            CascadeError::RegistryCompositionMismatch
        );
        assert!(matches!(
            queue.charge_operations(1),
            Err(CascadeError::CascadePoisoned { cause })
                if *cause == CascadeError::RegistryCompositionMismatch
        ));
        assert!(matches!(
            queue.finish(),
            Err(CascadeError::CascadePoisoned { cause })
                if *cause == CascadeError::RegistryCompositionMismatch
        ));
    }

    #[test]
    fn cascade_disallows_root_depth_reset_and_foreign_active_work() {
        let root = request(1);
        let readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let mut queue = CausalTriggerQueue::new(root, readiness.clone());
        queue.allocate_root_event().unwrap();
        queue
            .enqueue(prepared_trigger(
                &readiness,
                root,
                0,
                "work",
                1,
                0,
                0,
                json!({}),
            ))
            .unwrap();
        let work = queue.pop_next().unwrap().unwrap();
        assert_eq!(
            queue.allocate_root_event().unwrap_err(),
            CascadeError::RootEventPhaseClosed
        );
        assert!(matches!(
            queue.allocate_derived_event(&work),
            Err(CascadeError::CascadePoisoned { cause })
                if *cause == CascadeError::RootEventPhaseClosed
        ));

        let mut receiving_queue = CausalTriggerQueue::new(root, readiness.clone());
        receiving_queue.allocate_root_event().unwrap();
        let mut source_queue = CausalTriggerQueue::new(root, readiness.clone());
        source_queue.allocate_root_event().unwrap();
        source_queue
            .enqueue(prepared_trigger(
                &readiness,
                root,
                0,
                "other_queue",
                1,
                0,
                0,
                json!({}),
            ))
            .unwrap();
        let foreign_source = source_queue.pop_next().unwrap().unwrap();
        assert_eq!(
            receiving_queue
                .allocate_derived_event(&foreign_source)
                .unwrap_err(),
            CascadeError::ForeignTriggerWork
        );
    }

    #[test]
    fn cascade_rechecks_provider_readiness_after_admission() {
        let root = request(1);
        let readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let mut queue = CausalTriggerQueue::new(root, readiness.clone());
        queue.allocate_root_event().unwrap();
        queue
            .enqueue(prepared_trigger(
                &readiness,
                root,
                0,
                "work",
                1,
                0,
                0,
                json!({}),
            ))
            .unwrap();
        freeze_test_owner(&readiness, "queue_freeze");
        assert!(matches!(
            queue.pop_next(),
            Err(CascadeError::ProviderNotReady { owner: frozen, .. }) if frozen == owner()
        ));
        assert!(matches!(
            queue.finish(),
            Err(CascadeError::CascadePoisoned { cause })
                if matches!(*cause, CascadeError::ProviderNotReady { .. })
        ));

        let second_root = request(2);
        let second_readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let mut queue = CausalTriggerQueue::new(second_root, second_readiness.clone());
        queue.allocate_root_event().unwrap();
        queue
            .enqueue(prepared_trigger(
                &second_readiness,
                second_root,
                0,
                "popped_work",
                1,
                0,
                0,
                json!({}),
            ))
            .unwrap();
        let work = queue.pop_next().unwrap().unwrap();
        freeze_test_owner(&second_readiness, "queue_post_pop_freeze");
        assert!(matches!(
            queue.allocate_derived_event(&work),
            Err(CascadeError::ProviderNotReady { owner: frozen, .. }) if frozen == owner()
        ));

        let third_root = request(3);
        let third_readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let mut completion_queue = CausalTriggerQueue::new(third_root, third_readiness.clone());
        completion_queue.allocate_root_event().unwrap();
        completion_queue
            .enqueue(prepared_trigger(
                &third_readiness,
                third_root,
                0,
                "completion_freeze",
                1,
                0,
                0,
                json!({}),
            ))
            .unwrap();
        let work = completion_queue.pop_next().unwrap().unwrap();
        freeze_test_owner(&third_readiness, "queue_completion_freeze");
        assert!(matches!(
            completion_queue.complete_trigger(work),
            Err(CascadeError::ProviderNotReady { owner: frozen, .. }) if frozen == owner()
        ));
    }

    #[test]
    fn cascade_enforces_depth_event_operation_trigger_and_byte_limits() {
        let root = request(1);
        let readiness = MechanicalOwnerReadiness::for_owners(&owner_roster());
        let mut depth_queue = CausalTriggerQueue::new(root, readiness.clone());
        let mut parent = depth_queue.allocate_root_event().unwrap().event_id();
        for _ in 0..MAX_TRIGGER_DEPTH {
            depth_queue
                .enqueue(prepared_trigger(
                    &readiness,
                    root,
                    parent.ordinal(),
                    "chain",
                    1,
                    0,
                    0,
                    json!({}),
                ))
                .unwrap();
            let trigger = depth_queue.pop_next().unwrap().unwrap();
            parent = depth_queue
                .allocate_derived_event(&trigger)
                .unwrap()
                .event_id();
            depth_queue.complete_trigger(trigger).unwrap();
        }
        assert!(matches!(
            depth_queue.enqueue(prepared_trigger(
                &readiness,
                root,
                parent.ordinal(),
                "deep",
                1,
                0,
                0,
                json!({}),
            )),
            Err(CascadeError::DepthExceeded { .. })
        ));

        let mut unallocated = CausalTriggerQueue::new(root, readiness.clone());
        assert!(matches!(
            unallocated.enqueue(prepared_trigger(
                &readiness,
                root,
                200,
                "future",
                1,
                0,
                0,
                json!({}),
            )),
            Err(CascadeError::UnallocatedEvent { ordinal: 200 })
        ));

        let mut queue = CausalTriggerQueue::new(root, readiness.clone());
        for _ in 0..MAX_SEMANTIC_EVENTS {
            queue.allocate_root_event().unwrap();
        }
        assert!(matches!(
            queue.allocate_root_event(),
            Err(CascadeError::EventLimit { .. })
        ));

        let mut operation_queue = CausalTriggerQueue::new(root, readiness.clone());
        operation_queue
            .charge_operations(MAX_CASCADE_OPERATIONS)
            .unwrap();
        assert!(matches!(
            operation_queue.charge_operations(1),
            Err(CascadeError::OperationLimit { .. })
        ));

        let mut byte_queue = CausalTriggerQueue::new(root, readiness.clone());
        byte_queue.charge_bytes(MAX_CASCADE_BYTES).unwrap();
        assert!(matches!(
            byte_queue.charge_bytes(1),
            Err(CascadeError::BytesExceeded { .. })
        ));

        let mut trigger_queue = CausalTriggerQueue::new(root, readiness.clone());
        trigger_queue.allocate_root_event().unwrap();
        for target in 0..MAX_CASCADE_TRIGGERS {
            trigger_queue
                .enqueue(prepared_trigger(
                    &readiness,
                    root,
                    0,
                    "bounded",
                    target,
                    0,
                    0,
                    json!(null),
                ))
                .unwrap();
        }
        assert!(matches!(
            trigger_queue.enqueue(prepared_trigger(
                &readiness,
                root,
                0,
                "bounded",
                MAX_CASCADE_TRIGGERS,
                0,
                0,
                json!(null),
            )),
            Err(CascadeError::TriggerLimit { .. })
        ));
    }

    /// One action definition and the outcome definition every group in the
    /// cascade reports, so the bundle's definition set is closed.
    fn outbox_definitions() -> (DefinitionRegistry, DefinitionRef, DefinitionRef) {
        let mut builder = DefinitionRegistryBuilder::new();
        let action = builder
            .register(
                owner(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("wave").unwrap(),
                    1,
                    json!({"parameter": "target"}),
                )
                .unwrap(),
            )
            .unwrap();
        let outcome = builder
            .register(
                owner(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("outcome").unwrap(),
                    DefinitionId::new("waved").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        (builder.freeze().unwrap(), action, outcome)
    }

    /// A provider whose root output and whose one same-commit trigger each
    /// emit their own durable-outbox group with provider-chosen ordinals.
    struct OutboxCascadeProvider {
        outcome: DefinitionRef,
        counter: StateFamilyHandle<u64, u16>,
        root_ordinals: Vec<u16>,
        trigger_ordinals: Vec<u16>,
        emits_trigger: bool,
    }

    /// One trivial typed family and a store that shares the composition's
    /// readiness, so the root provider's transaction is not empty.
    fn outbox_state(
        roster: &BTreeSet<OwnerId>,
        readiness: MechanicalOwnerReadiness,
    ) -> (ActionStateStore, StateFamilyHandle<u64, u16>) {
        let mut builder = crate::action_state::StateFamilyRegistryBuilder::new();
        let counter = builder
            .register::<u64, u16, _, _, _, _>(
                owner(),
                crate::action_state::StateFamilyDescriptor::new(
                    crate::action_state::StateFamilyId::new("outbox_counter").unwrap(),
                    CanonicalId::new("u64.decimal.v1").unwrap(),
                    CanonicalId::new("counter.v1").unwrap(),
                    1,
                    CanonicalId::new("counter.decode_json.v1").unwrap(),
                    CanonicalId::new("counter.range.v1").unwrap(),
                    CanonicalId::new("counter.json.v1").unwrap(),
                )
                .unwrap(),
                |key: &u64| Ok(key.to_string()),
                |value: &Value| {
                    value
                        .as_u64()
                        .and_then(|amount| u16::try_from(amount).ok())
                        .ok_or_else(|| "amount must be a u16".to_owned())
                },
                |_value: &u16| Ok(()),
                |value: &u16| Ok(json!(value)),
            )
            .unwrap();
        let registry = builder
            .freeze_for_owners_with_readiness(roster, readiness)
            .unwrap();
        (ActionStateStore::new(registry), counter)
    }

    fn outbox_intents(ordinals: &[u16], group: &str) -> Vec<DurableOutboxIntent> {
        ordinals
            .iter()
            .map(|ordinal| {
                DurableOutboxIntent::new(
                    *ordinal,
                    owner(),
                    CanonicalId::new("courier").unwrap(),
                    json!({ "group": group, "hint": ordinal }),
                )
                .unwrap()
            })
            .collect()
    }

    impl ActionProvider for OutboxCascadeProvider {
        fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
            Ok(())
        }

        fn prepare_authoritative(
            &self,
            _invocation: &ActionInvocation,
            txn: &mut ActionTxn<'_>,
        ) -> Result<AuthoritativeActionOutput, ActionError> {
            txn.insert(&self.counter, &1, 1).unwrap();
            let outcome = RulesValue::new(self.outcome.clone(), json!({"waved": true})).unwrap();
            let mut output = AuthoritativeActionOutput::new(outcome)
                .with_durable_outbox(outbox_intents(&self.root_ordinals, "root"));
            if self.emits_trigger {
                output = output.with_same_commit_triggers(vec![SameCommitTrigger::new(
                    CanonicalId::new("echo").unwrap(),
                    SubjectKey::Object { id: ObjId(8) },
                    0,
                    json!({}),
                )]);
            }
            Ok(output)
        }

        fn prepare_same_commit_trigger(
            &self,
            _invocation: &ActionInvocation,
            _trigger: &PreparedTrigger,
            _txn: &mut ActionTxn<'_>,
        ) -> Result<AuthoritativeActionOutput, ActionError> {
            let truth = RulesValue::new(self.outcome.clone(), json!({"echoed": true})).unwrap();
            Ok(AuthoritativeActionOutput::new(truth)
                .with_durable_outbox(outbox_intents(&self.trigger_ordinals, "trigger")))
        }
    }

    /// Run the whole authoritative preparation for one cascade: a root group
    /// and, when `trigger_ordinals` is supplied, one same-commit trigger's.
    fn prepare_outbox_cascade(
        root_ordinals: &[u16],
        trigger_ordinals: Option<&[u16]>,
    ) -> Result<PreparedAuthoritativeAction, ActionProviderPrepareError> {
        let roster = owner_roster();
        let readiness = MechanicalOwnerReadiness::for_owners(&roster);
        let (definitions, action, outcome) = outbox_definitions();
        let definitions = bind_definitions(definitions, &readiness);
        let (mut state, counter) = outbox_state(&roster, readiness.clone());
        let mut builder = ActionProviderRegistryBuilder::new();
        builder
            .register(
                owner(),
                action.clone(),
                Arc::new(OutboxCascadeProvider {
                    outcome,
                    counter,
                    root_ordinals: root_ordinals.to_vec(),
                    trigger_ordinals: trigger_ordinals.unwrap_or(&[]).to_vec(),
                    emits_trigger: trigger_ordinals.is_some(),
                }),
            )
            .unwrap();
        let registry = builder
            .freeze_for_owners_with_readiness(&definitions, &roster, readiness)
            .unwrap();
        registry
            .prepare_authoritative_invocation(
                &action,
                &invocation(request(1)),
                CommitToken::from_bytes([2; 16]),
                &mut state,
                None,
            )
            .map(|(_txn, prepared)| prepared)
    }

    /// The group and provider-chosen hint each delivered intent carries,
    /// beside the ordinal the coordinator stamped on it.
    fn outbox_shape(prepared: &PreparedAuthoritativeAction) -> Vec<(String, u64, u16)> {
        prepared
            .durable_outbox
            .iter()
            .map(|intent| {
                (
                    intent.payload()["group"].as_str().unwrap().to_owned(),
                    intent.payload()["hint"].as_u64().unwrap(),
                    intent.ordinal(),
                )
            })
            .collect()
    }

    /// The bundle the server would build from one prepared action, with the
    /// outbox overridden when a test needs the pre-renumbering shape.
    fn outbox_bundle(
        prepared: PreparedAuthoritativeAction,
        outbox: Option<Vec<DurableOutboxIntent>>,
    ) -> Result<crate::authoritative::AuthoritativeBundle, AuthoritativeBundleError> {
        let (_, action, _) = outbox_definitions();
        let request = request(1);
        let (
            outcome,
            _resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = prepared.into_parts();
        // The coordinator, not the provider, contributes the action itself.
        definitions.push(action.clone());
        definitions.sort();
        definitions.dedup();
        crate::authoritative::AuthoritativeBundle::new(
            crate::authoritative::AuthoritativeBundleDraft {
                token: CommitToken::from_bytes([2; 16]),
                request,
                idempotency: idempotency(request),
                ingress_hash: CanonicalHash::from_bytes([3; 32]),
                request_hash: CanonicalHash::from_bytes([4; 32]),
                plan_hash: CanonicalHash::from_bytes([5; 32]),
                action,
                provider: owner(),
                cause: PreparedCausalRef::new(request, None, 0).unwrap(),
                definitions,
                writes: Vec::new(),
                outcome,
                resolution: None,
                semantic_events,
                eligible_witnesses,
                durable_outbox: outbox.unwrap_or(durable_outbox),
            },
        )
    }

    #[test]
    fn a_trigger_reusing_the_roots_outbox_ordinal_still_builds() {
        let prepared = prepare_outbox_cascade(&[0, 1], Some(&[0])).unwrap();
        assert_eq!(
            outbox_shape(&prepared),
            vec![
                ("root".to_owned(), 0, 0),
                ("root".to_owned(), 1, 1),
                ("trigger".to_owned(), 0, 2),
            ]
        );

        // What the pre-renumbering concatenation handed the bundle: the two
        // groups' ordinals globally sorted into 0, 0, 1.
        let mut interleaved = outbox_intents(&[0, 1], "root");
        interleaved.extend(outbox_intents(&[0], "trigger"));
        interleaved.sort_by_key(DurableOutboxIntent::ordinal);
        assert!(matches!(
            outbox_bundle(prepared.clone(), Some(interleaved)),
            Err(AuthoritativeBundleError::OutboxOrdinal {
                expected: 1,
                actual: 0
            })
        ));

        let bundle = outbox_bundle(prepared, None).unwrap();
        assert_eq!(
            bundle
                .durable_outbox()
                .iter()
                .map(DurableOutboxIntent::ordinal)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        // The persisted read is the path that requires canonical order; the
        // renumbered vector satisfies it rather than tripping NonCanonicalOutbox.
        let wire = serde_json::to_value(&bundle).unwrap();
        let restored: crate::authoritative::AuthoritativeBundle =
            serde_json::from_value(wire).unwrap();
        assert_eq!(restored, bundle);
    }

    #[test]
    fn root_and_trigger_ordinals_no_longer_interleave() {
        let prepared = prepare_outbox_cascade(&[0, 2], Some(&[1])).unwrap();
        // The deliberate semantic change: each group delivers whole and the
        // root's first, where the old global sort put the trigger's 1
        // between the root's 0 and 2.
        assert_eq!(
            outbox_shape(&prepared),
            vec![
                ("root".to_owned(), 0, 0),
                ("root".to_owned(), 2, 1),
                ("trigger".to_owned(), 1, 2),
            ]
        );
        outbox_bundle(prepared, None).unwrap();
    }

    #[test]
    fn duplicate_ordinals_inside_one_group_are_refused() {
        let root = prepare_outbox_cascade(&[0, 0], None).unwrap_err();
        assert!(matches!(
            &root,
            ActionProviderPrepareError::OutboxOrdinalConflict { group, ordinal }
                if group.as_ref() == "root" && *ordinal == 0
        ));

        let trigger = prepare_outbox_cascade(&[0], Some(&[3, 3])).unwrap_err();
        assert!(matches!(
            &trigger,
            ActionProviderPrepareError::OutboxOrdinalConflict { group, ordinal }
                if group.as_ref() == "echo" && *ordinal == 3
        ));
    }

    #[test]
    fn renumbering_is_identity_for_already_contiguous_groups() {
        let prepared = prepare_outbox_cascade(&[0, 1], None).unwrap();
        let emitted = outbox_intents(&[0, 1], "root");
        // Byte for byte what the provider built, payload_hash included.
        assert_eq!(prepared.durable_outbox, emitted);
        assert_eq!(
            prepared
                .durable_outbox
                .iter()
                .map(DurableOutboxIntent::payload_hash)
                .collect::<Vec<_>>(),
            emitted
                .iter()
                .map(DurableOutboxIntent::payload_hash)
                .collect::<Vec<_>>()
        );
        let renumbered = outbox_bundle(prepared.clone(), None).unwrap();
        let untouched = outbox_bundle(prepared, Some(emitted)).unwrap();
        assert_eq!(renumbered.bundle_hash(), untouched.bundle_hash());
    }
}
