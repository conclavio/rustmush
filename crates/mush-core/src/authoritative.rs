//! Canonical K3 authoritative-bundle identities and replay materialization.
//!
//! These types contain only deterministic post-images. They deliberately do
//! not expose stream appends, increments, server-time reads, or generated
//! store IDs. Persistence backends can therefore replay an accepted intent
//! until every target matches without rerunning game logic.

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use smol_str::SmolStr;
use thiserror::Error;

use crate::action::{
    CanonicalHash, CanonicalHashError, CommitToken, IdempotencyKey, MAX_SEMANTIC_EVENTS,
    PreparedCausalRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
};
use crate::ledger::{LedgerChunk, MAX_LEDGER_LOG_ID_BYTES, MAX_SAFE_CHUNK_BYTES, chunk_key};
use crate::mechanics::{
    CanonicalId, DefinitionLocator, DefinitionRef, OwnerId, RulesValue,
    canonical_json_bytes_with_limit,
};
use crate::module::MAX_MODULE_KEY_BYTES;
use crate::obj::{Obj, ObjId};
use crate::resolution::ResolutionRecord;

/// Hard maximum exact replay writes in one authoritative bundle.
pub const MAX_AUTHORITATIVE_REPLAY_WRITES: usize = 1_024;
/// Hard maximum exact definitions named by one authoritative bundle.
pub const MAX_AUTHORITATIVE_DEFINITIONS: usize = 256;
/// Hard maximum event-time witness principals retained by one bundle.
pub const MAX_AUTHORITATIVE_WITNESSES: usize = 1_024;
/// Hard maximum durable outbox intents attached to one bundle.
pub const MAX_DURABLE_OUTBOX_INTENTS: usize = 256;
/// Hard maximum canonical bytes in one durable outbox payload.
pub const MAX_DURABLE_OUTBOX_PAYLOAD_BYTES: usize = 256 * 1_024;
/// Hard maximum canonical bytes in the complete authoritative bundle.
pub const MAX_AUTHORITATIVE_BUNDLE_BYTES: usize = 16 * 1_024 * 1_024;

const ACTION_DEFINITION_KIND: &str = "action";

/// One immutable semantic fact prepared inside an authoritative action.
///
/// Its durable identity is the enclosing bundle's commit token plus the
/// ordinal carried by [`Self::cause`]. Keeping the pre-commit causal identity
/// here lets the K2 executor build and hash the complete event graph before
/// persistence assigns success.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "AuthoritativeSemanticEventWire", deny_unknown_fields)]
pub struct AuthoritativeSemanticEvent {
    cause: PreparedCausalRef,
    owner: OwnerId,
    kind: CanonicalId,
    truth: RulesValue,
    eligible_witnesses: Vec<PrincipalId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeSemanticEventWire {
    cause: PreparedCausalRef,
    owner: OwnerId,
    kind: CanonicalId,
    truth: RulesValue,
    eligible_witnesses: Vec<PrincipalId>,
}

impl AuthoritativeSemanticEvent {
    pub fn root(
        cause: PreparedCausalRef,
        action: &DefinitionRef,
        truth: RulesValue,
        eligible_witnesses: Vec<PrincipalId>,
    ) -> Result<Self, AuthoritativeBundleError> {
        Self::new(
            cause,
            action.key().owner.clone(),
            CanonicalId::parse_canonical(action.key().id.as_str())
                .expect("a definition id is also a canonical mechanics id"),
            truth,
            eligible_witnesses,
        )
    }

    pub fn new(
        cause: PreparedCausalRef,
        owner: OwnerId,
        kind: CanonicalId,
        truth: RulesValue,
        mut eligible_witnesses: Vec<PrincipalId>,
    ) -> Result<Self, AuthoritativeBundleError> {
        eligible_witnesses.sort();
        eligible_witnesses.dedup();
        if eligible_witnesses.len() > MAX_AUTHORITATIVE_WITNESSES {
            return Err(AuthoritativeBundleError::EventWitnessLimit {
                ordinal: cause.ordinal(),
                actual: eligible_witnesses.len(),
                max: MAX_AUTHORITATIVE_WITNESSES,
            });
        }
        let event = Self {
            cause,
            owner,
            kind,
            truth,
            eligible_witnesses,
        };
        event.canonical_len()?;
        Ok(event)
    }

    #[must_use]
    pub const fn cause(&self) -> PreparedCausalRef {
        self.cause
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn kind(&self) -> &CanonicalId {
        &self.kind
    }

    #[must_use]
    pub const fn truth(&self) -> &RulesValue {
        &self.truth
    }

    #[must_use]
    pub fn eligible_witnesses(&self) -> &[PrincipalId] {
        &self.eligible_witnesses
    }

    pub(crate) fn canonical_len(&self) -> Result<usize, AuthoritativeBundleError> {
        canonical_json_bytes_with_limit(
            &json!({
                "cause": self.cause,
                "owner": self.owner,
                "kind": self.kind,
                "truth": self.truth,
                "eligible_witnesses": self.eligible_witnesses,
            }),
            crate::action::MAX_CASCADE_BYTES,
        )
        .map(|bytes| bytes.len())
        .map_err(|_| AuthoritativeBundleError::SemanticEventTooLarge {
            ordinal: self.cause.ordinal(),
            max: crate::action::MAX_CASCADE_BYTES,
        })
    }
}

impl TryFrom<AuthoritativeSemanticEventWire> for AuthoritativeSemanticEvent {
    type Error = AuthoritativeBundleError;

    fn try_from(value: AuthoritativeSemanticEventWire) -> Result<Self, Self::Error> {
        let original_witnesses = value.eligible_witnesses.clone();
        let event = Self::new(
            value.cause,
            value.owner,
            value.kind,
            value.truth,
            value.eligible_witnesses,
        )?;
        if event.eligible_witnesses != original_witnesses {
            return Err(AuthoritativeBundleError::NonCanonicalEventWitnesses {
                ordinal: event.cause.ordinal(),
            });
        }
        Ok(event)
    }
}

/// Canonical bounded key below an owner-stamped module namespace.
///
/// New authoritative prefixes use this stricter vocabulary even though the
/// legacy module store accepts a wider set of historical keys.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthoritativeKey(SmolStr);

impl AuthoritativeKey {
    pub fn parse_canonical(value: impl AsRef<str>) -> Result<Self, AuthoritativeKeyError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(AuthoritativeKeyError::Empty);
        }
        if value.len() > MAX_MODULE_KEY_BYTES {
            return Err(AuthoritativeKeyError::TooLong {
                actual: value.len(),
                max: MAX_MODULE_KEY_BYTES,
            });
        }
        if let Some((offset, byte)) = value.bytes().enumerate().find(|(_, byte)| {
            !matches!(
                byte,
                b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b':' | b'/' | b'-' | b'~'
            )
        }) {
            return Err(AuthoritativeKeyError::InvalidByte { offset, byte });
        }
        Ok(Self(SmolStr::new(value)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for AuthoritativeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("AuthoritativeKey")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for AuthoritativeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for AuthoritativeKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AuthoritativeKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse_canonical(value).map_err(D::Error::custom)
    }
}

/// Invalid authoritative persistence key.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AuthoritativeKeyError {
    #[error("authoritative persistence key is empty")]
    Empty,
    #[error("authoritative persistence key is {actual} bytes; maximum is {max}")]
    TooLong { actual: usize, max: usize },
    #[error("authoritative persistence key contains invalid byte {byte:#04x} at byte {offset}")]
    InvalidByte { offset: usize, byte: u8 },
}

/// One exact replayable Ledger chunk post-image.
///
/// The physical module key is derived from `log_id` and the chunk's index;
/// callers cannot supply an alias. A sealed chunk carries a domain-separated
/// hash of its exact owner/location/post-image in the same immutable value.
/// Unsealed tails remain replayable whole-document post-images but do not
/// claim immutable-seal identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "AuthoritativeLedgerWriteWire", deny_unknown_fields)]
pub struct AuthoritativeLedgerWrite {
    namespace: OwnerId,
    log_id: SmolStr,
    chunk: LedgerChunk,
    #[serde(skip_serializing_if = "Option::is_none")]
    seal: Option<CanonicalHash>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeLedgerWriteWire {
    namespace: OwnerId,
    log_id: SmolStr,
    chunk: LedgerChunk,
    seal: Option<CanonicalHash>,
}

impl AuthoritativeLedgerWrite {
    pub fn new(
        namespace: OwnerId,
        log_id: impl AsRef<str>,
        chunk: LedgerChunk,
    ) -> Result<Self, AuthoritativeLedgerWriteError> {
        let log_id = log_id.as_ref();
        if log_id.is_empty() {
            return Err(AuthoritativeLedgerWriteError::EmptyLogId);
        }
        if log_id.len() > MAX_LEDGER_LOG_ID_BYTES {
            return Err(AuthoritativeLedgerWriteError::LogIdTooLong {
                actual: log_id.len(),
                max: MAX_LEDGER_LOG_ID_BYTES,
            });
        }
        let key = AuthoritativeKey::parse_canonical(chunk_key(log_id, chunk.chunk))
            .map_err(AuthoritativeLedgerWriteError::Key)?;
        if chunk.entries.is_empty()
            || chunk
                .entries
                .windows(2)
                .any(|pair| pair[0].seq.checked_add(1) != Some(pair[1].seq))
        {
            return Err(AuthoritativeLedgerWriteError::NonContiguousChunk);
        }
        canonical_json_bytes_with_limit(&serde_json::to_value(&chunk)?, MAX_SAFE_CHUNK_BYTES)
            .map_err(|_| AuthoritativeLedgerWriteError::ChunkTooLarge {
                max: MAX_SAFE_CHUNK_BYTES,
            })?;
        let log_id = SmolStr::new(log_id);
        let seal = chunk.sealed.then(|| {
            CanonicalHash::for_canonical_value(
                "authoritative_ledger_seal.v1",
                1,
                &json!({
                    "namespace": namespace,
                    "key": key,
                    "chunk": chunk,
                }),
                MAX_SAFE_CHUNK_BYTES + MAX_LEDGER_LOG_ID_BYTES + 1_024,
            )
        });
        Ok(Self {
            namespace,
            log_id,
            chunk,
            seal: seal.transpose()?,
        })
    }

    #[must_use]
    pub const fn namespace(&self) -> &OwnerId {
        &self.namespace
    }

    #[must_use]
    pub fn log_id(&self) -> &str {
        self.log_id.as_str()
    }

    #[must_use]
    pub const fn chunk(&self) -> &LedgerChunk {
        &self.chunk
    }

    #[must_use]
    pub const fn seal(&self) -> Option<CanonicalHash> {
        self.seal
    }

    #[must_use]
    pub fn key(&self) -> AuthoritativeKey {
        AuthoritativeKey::parse_canonical(chunk_key(self.log_id(), self.chunk.chunk))
            .expect("validated authoritative Ledger keys remain canonical")
    }
}

impl TryFrom<AuthoritativeLedgerWriteWire> for AuthoritativeLedgerWrite {
    type Error = AuthoritativeLedgerWriteError;

    fn try_from(value: AuthoritativeLedgerWriteWire) -> Result<Self, Self::Error> {
        let expected = Self::new(value.namespace, value.log_id, value.chunk)?;
        if expected.seal != value.seal {
            return Err(AuthoritativeLedgerWriteError::SealMismatch);
        }
        Ok(expected)
    }
}

#[derive(Debug, Error)]
pub enum AuthoritativeLedgerWriteError {
    #[error("authoritative Ledger log id is empty")]
    EmptyLogId,
    #[error("authoritative Ledger log id is {actual} bytes; maximum is {max}")]
    LogIdTooLong { actual: usize, max: usize },
    #[error("authoritative Ledger key is invalid: {0}")]
    Key(#[source] AuthoritativeKeyError),
    #[error("authoritative Ledger chunk entries are empty or non-contiguous")]
    NonContiguousChunk,
    #[error("authoritative Ledger chunk exceeds {max} canonical bytes")]
    ChunkTooLarge { max: usize },
    #[error("authoritative Ledger seal does not match its exact owner/location/post-image")]
    SealMismatch,
    #[error("authoritative Ledger seal hash failed: {0}")]
    Hash(#[from] CanonicalHashError),
    #[error("authoritative Ledger chunk could not be encoded: {0}")]
    Encoding(#[from] serde_json::Error),
}

/// One exact idempotently replayable persistence post-image.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplayWrite {
    ObjectSet {
        object: Obj,
    },
    ObjectDelete {
        id: ObjId,
    },
    ObjectAllocatorSet {
        next_id: u32,
    },
    ModuleJsonSet {
        namespace: OwnerId,
        key: AuthoritativeKey,
        value: Value,
    },
    ModuleDelete {
        namespace: OwnerId,
        key: AuthoritativeKey,
    },
    LedgerChunkSet {
        write: AuthoritativeLedgerWrite,
    },
}

impl ReplayWrite {
    fn target(&self) -> ReplayTarget {
        match self {
            Self::ObjectSet { object } => ReplayTarget::Object(object.id),
            Self::ObjectDelete { id } => ReplayTarget::Object(*id),
            Self::ObjectAllocatorSet { .. } => ReplayTarget::ObjectAllocator,
            Self::ModuleJsonSet { namespace, key, .. } | Self::ModuleDelete { namespace, key } => {
                ReplayTarget::Module(namespace.clone(), key.clone())
            }
            Self::LedgerChunkSet { write } => {
                ReplayTarget::Module(write.namespace().clone(), write.key())
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ReplayTarget {
    Object(ObjId),
    ObjectAllocator,
    Module(OwnerId, AuthoritativeKey),
}

/// One stable post-finalization delivery intent.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "DurableOutboxIntentWire", deny_unknown_fields)]
pub struct DurableOutboxIntent {
    ordinal: u16,
    owner: OwnerId,
    adapter: CanonicalId,
    payload: Value,
    payload_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableOutboxIntentWire {
    ordinal: u16,
    owner: OwnerId,
    adapter: CanonicalId,
    payload: Value,
    payload_hash: CanonicalHash,
}

impl DurableOutboxIntent {
    pub fn new(
        ordinal: u16,
        owner: OwnerId,
        adapter: CanonicalId,
        payload: Value,
    ) -> Result<Self, AuthoritativeBundleError> {
        canonical_json_bytes_with_limit(&payload, MAX_DURABLE_OUTBOX_PAYLOAD_BYTES).map_err(
            |_| AuthoritativeBundleError::OutboxPayloadTooLarge {
                ordinal,
                max: MAX_DURABLE_OUTBOX_PAYLOAD_BYTES,
            },
        )?;
        let payload_hash = CanonicalHash::for_canonical_value(
            "durable_outbox_payload.v1",
            1,
            &json!({
                "ordinal": ordinal,
                "owner": owner,
                "adapter": adapter,
                "payload": payload,
            }),
            MAX_DURABLE_OUTBOX_PAYLOAD_BYTES + 1_024,
        )?;
        Ok(Self {
            ordinal,
            owner,
            adapter,
            payload,
            payload_hash,
        })
    }

    #[must_use]
    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn adapter(&self) -> &CanonicalId {
        &self.adapter
    }

    #[must_use]
    pub const fn payload(&self) -> &Value {
        &self.payload
    }

    #[must_use]
    pub const fn payload_hash(&self) -> CanonicalHash {
        self.payload_hash
    }

    pub fn stable_id(&self, parent: CommitToken) -> Result<CanonicalHash, CanonicalHashError> {
        CanonicalHash::for_canonical_value(
            "durable_outbox_item.v1",
            1,
            &json!({ "parent": parent, "ordinal": self.ordinal }),
            256,
        )
    }
}

impl TryFrom<DurableOutboxIntentWire> for DurableOutboxIntent {
    type Error = AuthoritativeBundleError;

    fn try_from(value: DurableOutboxIntentWire) -> Result<Self, Self::Error> {
        let intent = Self::new(value.ordinal, value.owner, value.adapter, value.payload)?;
        if intent.payload_hash != value.payload_hash {
            return Err(AuthoritativeBundleError::HashMismatch {
                field: "durable_outbox.payload_hash",
            });
        }
        Ok(intent)
    }
}

/// Constructor input for one complete authoritative intent.
#[derive(Clone, Debug)]
pub struct AuthoritativeBundleDraft {
    pub token: CommitToken,
    pub request: RequestUid,
    pub idempotency: ScopedIdempotencyKey,
    pub ingress_hash: CanonicalHash,
    pub request_hash: CanonicalHash,
    pub plan_hash: CanonicalHash,
    pub action: DefinitionRef,
    pub provider: OwnerId,
    pub cause: PreparedCausalRef,
    pub definitions: Vec<DefinitionRef>,
    pub writes: Vec<ReplayWrite>,
    pub outcome: RulesValue,
    pub resolution: Option<ResolutionRecord>,
    pub semantic_events: Vec<AuthoritativeSemanticEvent>,
    pub eligible_witnesses: Vec<PrincipalId>,
    pub durable_outbox: Vec<DurableOutboxIntent>,
}

/// Canonical, bounded and self-hashed durable action intent.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AuthoritativeBundle {
    schema_version: u16,
    token: CommitToken,
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    request_hash: CanonicalHash,
    plan_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    cause: PreparedCausalRef,
    definitions: Vec<DefinitionRef>,
    writes: Vec<ReplayWrite>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ledger_seals: Option<Vec<CanonicalHash>>,
    outcome: RulesValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<ResolutionRecord>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    semantic_events: Vec<AuthoritativeSemanticEvent>,
    eligible_witnesses: Vec<PrincipalId>,
    durable_outbox: Vec<DurableOutboxIntent>,
    bundle_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeBundleWireV2 {
    schema_version: u16,
    token: CommitToken,
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    request_hash: CanonicalHash,
    plan_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    cause: PreparedCausalRef,
    definitions: Vec<DefinitionRef>,
    writes: Vec<ReplayWrite>,
    outcome: RulesValue,
    semantic_events: Vec<AuthoritativeSemanticEvent>,
    eligible_witnesses: Vec<PrincipalId>,
    durable_outbox: Vec<DurableOutboxIntent>,
    bundle_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeBundleWireV3 {
    schema_version: u16,
    token: CommitToken,
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    request_hash: CanonicalHash,
    plan_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    cause: PreparedCausalRef,
    definitions: Vec<DefinitionRef>,
    writes: Vec<ReplayWrite>,
    outcome: RulesValue,
    resolution: ResolutionRecord,
    semantic_events: Vec<AuthoritativeSemanticEvent>,
    eligible_witnesses: Vec<PrincipalId>,
    durable_outbox: Vec<DurableOutboxIntent>,
    bundle_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeBundleWireV4 {
    schema_version: u16,
    token: CommitToken,
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    request_hash: CanonicalHash,
    plan_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    cause: PreparedCausalRef,
    definitions: Vec<DefinitionRef>,
    writes: Vec<ReplayWrite>,
    ledger_seals: Vec<CanonicalHash>,
    outcome: RulesValue,
    resolution: Option<ResolutionRecord>,
    semantic_events: Vec<AuthoritativeSemanticEvent>,
    eligible_witnesses: Vec<PrincipalId>,
    durable_outbox: Vec<DurableOutboxIntent>,
    bundle_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeBundleWireV1 {
    schema_version: u16,
    token: CommitToken,
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    request_hash: CanonicalHash,
    plan_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    cause: PreparedCausalRef,
    definitions: Vec<DefinitionRef>,
    writes: Vec<ReplayWrite>,
    outcome: RulesValue,
    eligible_witnesses: Vec<PrincipalId>,
    durable_outbox: Vec<DurableOutboxIntent>,
    bundle_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AuthoritativeBundleWire {
    V4(Box<AuthoritativeBundleWireV4>),
    V3(Box<AuthoritativeBundleWireV3>),
    V2(Box<AuthoritativeBundleWireV2>),
    V1(Box<AuthoritativeBundleWireV1>),
}

impl AuthoritativeBundle {
    pub const SCHEMA_VERSION: u16 = 4;
    const RESOLUTION_SCHEMA_VERSION: u16 = 3;
    const SEMANTIC_EVENT_SCHEMA_VERSION: u16 = 2;
    const LEGACY_SCHEMA_VERSION: u16 = 1;

    pub fn new(draft: AuthoritativeBundleDraft) -> Result<Self, AuthoritativeBundleError> {
        Self::build(draft, Self::SCHEMA_VERSION, false, None, None)
    }

    fn build(
        mut draft: AuthoritativeBundleDraft,
        schema_version: u16,
        require_canonical_order: bool,
        claimed_hash: Option<CanonicalHash>,
        claimed_ledger_seals: Option<Vec<CanonicalHash>>,
    ) -> Result<Self, AuthoritativeBundleError> {
        validate_identity(&draft)?;

        let original_definitions = require_canonical_order.then(|| draft.definitions.clone());
        draft.definitions.sort();
        if draft.definitions.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(AuthoritativeBundleError::DuplicateDefinition);
        }
        if let Some(pair) = draft
            .definitions
            .windows(2)
            .find(|pair| pair[0].locator() == pair[1].locator())
        {
            return Err(AuthoritativeBundleError::ConflictingDefinition {
                locator: Box::new(pair[0].locator()),
            });
        }
        if original_definitions
            .as_ref()
            .is_some_and(|original| *original != draft.definitions)
        {
            return Err(AuthoritativeBundleError::NonCanonicalDefinitions);
        }
        if draft.definitions.len() > MAX_AUTHORITATIVE_DEFINITIONS {
            return Err(AuthoritativeBundleError::DefinitionLimit {
                actual: draft.definitions.len(),
                max: MAX_AUTHORITATIVE_DEFINITIONS,
            });
        }
        for required in [&draft.action, draft.outcome.schema()] {
            if draft.definitions.binary_search(required).is_err() {
                return Err(AuthoritativeBundleError::MissingDefinition {
                    definition: Box::new(required.clone()),
                });
            }
        }

        let original_writes = require_canonical_order.then(|| draft.writes.clone());
        draft.writes.sort_by_key(ReplayWrite::target);
        if original_writes
            .as_ref()
            .is_some_and(|original| *original != draft.writes)
        {
            return Err(AuthoritativeBundleError::NonCanonicalWrites);
        }
        for pair in draft.writes.windows(2) {
            if pair[0].target() == pair[1].target() {
                return Err(AuthoritativeBundleError::DuplicateWrite {
                    target: format!("{:?}", pair[0].target()).into_boxed_str(),
                });
            }
        }
        if draft.writes.len() > MAX_AUTHORITATIVE_REPLAY_WRITES {
            return Err(AuthoritativeBundleError::ReplayWriteLimit {
                actual: draft.writes.len(),
                max: MAX_AUTHORITATIVE_REPLAY_WRITES,
            });
        }
        validate_allocator(&draft.writes)?;
        if schema_version < Self::SCHEMA_VERSION
            && draft
                .writes
                .iter()
                .any(|write| matches!(write, ReplayWrite::LedgerChunkSet { .. }))
        {
            return Err(AuthoritativeBundleError::LegacyLedgerWrite);
        }
        let ledger_seals: Vec<_> = draft
            .writes
            .iter()
            .filter_map(|write| match write {
                ReplayWrite::LedgerChunkSet { write } => write.seal(),
                _ => None,
            })
            .collect();
        if claimed_ledger_seals
            .as_ref()
            .is_some_and(|claimed| claimed != &ledger_seals)
        {
            return Err(AuthoritativeBundleError::LedgerSealSetMismatch);
        }

        let original_witnesses = require_canonical_order.then(|| draft.eligible_witnesses.clone());
        draft.eligible_witnesses.sort();
        draft.eligible_witnesses.dedup();
        if original_witnesses
            .as_ref()
            .is_some_and(|original| *original != draft.eligible_witnesses)
        {
            return Err(AuthoritativeBundleError::NonCanonicalWitnesses);
        }
        if draft.eligible_witnesses.len() > MAX_AUTHORITATIVE_WITNESSES {
            return Err(AuthoritativeBundleError::WitnessLimit {
                actual: draft.eligible_witnesses.len(),
                max: MAX_AUTHORITATIVE_WITNESSES,
            });
        }
        validate_semantic_events(&draft, schema_version)?;
        validate_resolution(&draft, schema_version)?;

        let original_outbox = require_canonical_order.then(|| draft.durable_outbox.clone());
        draft
            .durable_outbox
            .sort_by_key(DurableOutboxIntent::ordinal);
        if original_outbox
            .as_ref()
            .is_some_and(|original| *original != draft.durable_outbox)
        {
            return Err(AuthoritativeBundleError::NonCanonicalOutbox);
        }
        if draft.durable_outbox.len() > MAX_DURABLE_OUTBOX_INTENTS {
            return Err(AuthoritativeBundleError::OutboxLimit {
                actual: draft.durable_outbox.len(),
                max: MAX_DURABLE_OUTBOX_INTENTS,
            });
        }
        for (expected, intent) in draft.durable_outbox.iter().enumerate() {
            if usize::from(intent.ordinal) != expected {
                return Err(AuthoritativeBundleError::OutboxOrdinal {
                    expected,
                    actual: intent.ordinal,
                });
            }
        }

        let hash_material = bundle_hash_material(&draft, schema_version);
        let hash_domain = if schema_version == Self::LEGACY_SCHEMA_VERSION {
            "authoritative_bundle.v1"
        } else {
            "authoritative_bundle.v2"
        };
        let bundle_hash = CanonicalHash::for_canonical_value(
            hash_domain,
            schema_version,
            &hash_material,
            MAX_AUTHORITATIVE_BUNDLE_BYTES,
        )?;
        if claimed_hash.is_some_and(|claimed| claimed != bundle_hash) {
            return Err(AuthoritativeBundleError::HashMismatch {
                field: "bundle_hash",
            });
        }
        let mut bounded_material = hash_material;
        bounded_material
            .as_object_mut()
            .expect("bundle hash material is always an object")
            .insert("bundle_hash".to_owned(), json!(bundle_hash));
        canonical_json_bytes_with_limit(&bounded_material, MAX_AUTHORITATIVE_BUNDLE_BYTES)
            .map_err(|_| AuthoritativeBundleError::BundleTooLarge {
                max: MAX_AUTHORITATIVE_BUNDLE_BYTES,
            })?;
        Ok(Self {
            schema_version,
            token: draft.token,
            request: draft.request,
            idempotency: draft.idempotency,
            ingress_hash: draft.ingress_hash,
            request_hash: draft.request_hash,
            plan_hash: draft.plan_hash,
            action: draft.action,
            provider: draft.provider,
            cause: draft.cause,
            definitions: draft.definitions,
            writes: draft.writes,
            ledger_seals: (schema_version >= Self::SCHEMA_VERSION).then_some(ledger_seals),
            outcome: draft.outcome,
            resolution: draft.resolution,
            semantic_events: draft.semantic_events,
            eligible_witnesses: draft.eligible_witnesses,
            durable_outbox: draft.durable_outbox,
            bundle_hash,
        })
    }

    #[must_use]
    pub const fn token(&self) -> CommitToken {
        self.token
    }

    #[must_use]
    pub const fn request(&self) -> RequestUid {
        self.request
    }

    #[must_use]
    pub const fn idempotency(&self) -> &ScopedIdempotencyKey {
        &self.idempotency
    }

    #[must_use]
    pub const fn ingress_hash(&self) -> CanonicalHash {
        self.ingress_hash
    }

    #[must_use]
    pub const fn request_hash(&self) -> CanonicalHash {
        self.request_hash
    }

    #[must_use]
    pub const fn plan_hash(&self) -> CanonicalHash {
        self.plan_hash
    }

    #[must_use]
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }

    #[must_use]
    pub const fn provider(&self) -> &OwnerId {
        &self.provider
    }

    #[must_use]
    pub const fn cause(&self) -> PreparedCausalRef {
        self.cause
    }

    #[must_use]
    pub fn definitions(&self) -> &[DefinitionRef] {
        &self.definitions
    }

    #[must_use]
    pub const fn bundle_hash(&self) -> CanonicalHash {
        self.bundle_hash
    }

    #[must_use]
    pub fn writes(&self) -> &[ReplayWrite] {
        &self.writes
    }

    #[must_use]
    pub fn ledger_seals(&self) -> &[CanonicalHash] {
        self.ledger_seals.as_deref().unwrap_or_default()
    }

    #[must_use]
    pub const fn outcome(&self) -> &RulesValue {
        &self.outcome
    }

    #[must_use]
    pub const fn resolution(&self) -> Option<&ResolutionRecord> {
        self.resolution.as_ref()
    }

    #[must_use]
    pub fn semantic_events(&self) -> &[AuthoritativeSemanticEvent] {
        &self.semantic_events
    }

    #[must_use]
    pub fn eligible_witnesses(&self) -> &[PrincipalId] {
        &self.eligible_witnesses
    }

    #[must_use]
    pub fn durable_outbox(&self) -> &[DurableOutboxIntent] {
        &self.durable_outbox
    }
}

impl<'de> Deserialize<'de> for AuthoritativeBundle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AuthoritativeBundleWire::deserialize(deserializer)?;
        match wire {
            AuthoritativeBundleWire::V4(wire) => {
                let wire = *wire;
                if wire.schema_version != Self::SCHEMA_VERSION {
                    return Err(D::Error::custom(format!(
                        "unsupported authoritative bundle schema version {}",
                        wire.schema_version
                    )));
                }
                Self::build(
                    AuthoritativeBundleDraft {
                        token: wire.token,
                        request: wire.request,
                        idempotency: wire.idempotency,
                        ingress_hash: wire.ingress_hash,
                        request_hash: wire.request_hash,
                        plan_hash: wire.plan_hash,
                        action: wire.action,
                        provider: wire.provider,
                        cause: wire.cause,
                        definitions: wire.definitions,
                        writes: wire.writes,
                        outcome: wire.outcome,
                        resolution: wire.resolution,
                        semantic_events: wire.semantic_events,
                        eligible_witnesses: wire.eligible_witnesses,
                        durable_outbox: wire.durable_outbox,
                    },
                    wire.schema_version,
                    true,
                    Some(wire.bundle_hash),
                    Some(wire.ledger_seals),
                )
            }
            AuthoritativeBundleWire::V3(wire) => {
                let wire = *wire;
                if wire.schema_version != Self::RESOLUTION_SCHEMA_VERSION {
                    return Err(D::Error::custom(format!(
                        "unsupported authoritative bundle schema version {}",
                        wire.schema_version
                    )));
                }
                Self::build(
                    AuthoritativeBundleDraft {
                        token: wire.token,
                        request: wire.request,
                        idempotency: wire.idempotency,
                        ingress_hash: wire.ingress_hash,
                        request_hash: wire.request_hash,
                        plan_hash: wire.plan_hash,
                        action: wire.action,
                        provider: wire.provider,
                        cause: wire.cause,
                        definitions: wire.definitions,
                        writes: wire.writes,
                        outcome: wire.outcome,
                        resolution: Some(wire.resolution),
                        semantic_events: wire.semantic_events,
                        eligible_witnesses: wire.eligible_witnesses,
                        durable_outbox: wire.durable_outbox,
                    },
                    wire.schema_version,
                    true,
                    Some(wire.bundle_hash),
                    None,
                )
            }
            AuthoritativeBundleWire::V1(wire) => {
                let wire = *wire;
                if wire.schema_version != Self::LEGACY_SCHEMA_VERSION {
                    return Err(D::Error::custom(format!(
                        "unsupported authoritative bundle schema version {}",
                        wire.schema_version
                    )));
                }
                Self::build(
                    AuthoritativeBundleDraft {
                        token: wire.token,
                        request: wire.request,
                        idempotency: wire.idempotency,
                        ingress_hash: wire.ingress_hash,
                        request_hash: wire.request_hash,
                        plan_hash: wire.plan_hash,
                        action: wire.action,
                        provider: wire.provider,
                        cause: wire.cause,
                        definitions: wire.definitions,
                        writes: wire.writes,
                        outcome: wire.outcome,
                        resolution: None,
                        semantic_events: Vec::new(),
                        eligible_witnesses: wire.eligible_witnesses,
                        durable_outbox: wire.durable_outbox,
                    },
                    wire.schema_version,
                    true,
                    Some(wire.bundle_hash),
                    None,
                )
            }
            AuthoritativeBundleWire::V2(wire) => {
                let wire = *wire;
                if wire.schema_version != Self::SEMANTIC_EVENT_SCHEMA_VERSION {
                    return Err(D::Error::custom(format!(
                        "unsupported authoritative bundle schema version {}",
                        wire.schema_version
                    )));
                }
                Self::build(
                    AuthoritativeBundleDraft {
                        token: wire.token,
                        request: wire.request,
                        idempotency: wire.idempotency,
                        ingress_hash: wire.ingress_hash,
                        request_hash: wire.request_hash,
                        plan_hash: wire.plan_hash,
                        action: wire.action,
                        provider: wire.provider,
                        cause: wire.cause,
                        definitions: wire.definitions,
                        writes: wire.writes,
                        outcome: wire.outcome,
                        resolution: None,
                        semantic_events: wire.semantic_events,
                        eligible_witnesses: wire.eligible_witnesses,
                        durable_outbox: wire.durable_outbox,
                    },
                    wire.schema_version,
                    true,
                    Some(wire.bundle_hash),
                    None,
                )
            }
        }
        .map_err(D::Error::custom)
    }
}

fn validate_resolution(
    draft: &AuthoritativeBundleDraft,
    schema_version: u16,
) -> Result<(), AuthoritativeBundleError> {
    let Some(resolution) = &draft.resolution else {
        if schema_version == AuthoritativeBundle::RESOLUTION_SCHEMA_VERSION {
            return Err(AuthoritativeBundleError::MissingResolution);
        }
        return Ok(());
    };
    if schema_version < AuthoritativeBundle::RESOLUTION_SCHEMA_VERSION {
        return Err(AuthoritativeBundleError::LegacyResolution);
    }
    if resolution.action() != draft.token {
        return Err(AuthoritativeBundleError::ResolutionTokenMismatch);
    }
    if resolution.outcome() != &draft.outcome {
        return Err(AuthoritativeBundleError::ResolutionOutcomeMismatch);
    }
    for definition in resolution.definition_refs().iter().chain([
        resolution.modifiers().schema(),
        resolution.outcome().schema(),
    ]) {
        if draft.definitions.binary_search(definition).is_err() {
            return Err(AuthoritativeBundleError::MissingDefinition {
                definition: Box::new(definition.clone()),
            });
        }
    }
    Ok(())
}

fn validate_semantic_events(
    draft: &AuthoritativeBundleDraft,
    schema_version: u16,
) -> Result<(), AuthoritativeBundleError> {
    if schema_version == AuthoritativeBundle::LEGACY_SCHEMA_VERSION {
        if !draft.semantic_events.is_empty() {
            return Err(AuthoritativeBundleError::LegacySemanticEvents);
        }
        return Ok(());
    }
    if draft.semantic_events.is_empty() {
        return Err(AuthoritativeBundleError::MissingRootSemanticEvent);
    }
    if draft.semantic_events.len() > MAX_SEMANTIC_EVENTS as usize {
        return Err(AuthoritativeBundleError::SemanticEventLimit {
            actual: draft.semantic_events.len(),
            max: MAX_SEMANTIC_EVENTS,
        });
    }
    for (expected, event) in draft.semantic_events.iter().enumerate() {
        let expected = u32::try_from(expected).expect("semantic event hard limit fits u32");
        if event.cause.ordinal() != expected {
            return Err(AuthoritativeBundleError::SemanticEventOrdinal {
                expected,
                actual: event.cause.ordinal(),
            });
        }
        if event.cause.root_request() != draft.request {
            return Err(AuthoritativeBundleError::SemanticEventRequestMismatch {
                ordinal: event.cause.ordinal(),
            });
        }
        if event.owner != draft.provider {
            return Err(AuthoritativeBundleError::SemanticEventOwnerMismatch {
                ordinal: event.cause.ordinal(),
            });
        }
        if draft
            .definitions
            .binary_search(event.truth.schema())
            .is_err()
        {
            return Err(AuthoritativeBundleError::MissingDefinition {
                definition: Box::new(event.truth.schema().clone()),
            });
        }
    }
    let root = &draft.semantic_events[0];
    if root.cause != draft.cause
        || root.truth != draft.outcome
        || root.eligible_witnesses != draft.eligible_witnesses
    {
        return Err(AuthoritativeBundleError::RootSemanticEventMismatch);
    }
    Ok(())
}

fn validate_identity(draft: &AuthoritativeBundleDraft) -> Result<(), AuthoritativeBundleError> {
    if let IdempotencyKey::Request(request) = draft.idempotency.key()
        && *request != draft.request
    {
        return Err(AuthoritativeBundleError::RequestKeyMismatch);
    }
    if draft.cause.root_request() != draft.request {
        return Err(AuthoritativeBundleError::CauseRequestMismatch);
    }
    if draft.idempotency.scope().owner != draft.provider {
        return Err(AuthoritativeBundleError::IdempotencyOwnerMismatch);
    }
    if draft.action.key().owner != draft.provider
        || draft.action.key().kind.as_str() != ACTION_DEFINITION_KIND
    {
        return Err(AuthoritativeBundleError::WrongActionProvider);
    }
    Ok(())
}

fn validate_allocator(writes: &[ReplayWrite]) -> Result<(), AuthoritativeBundleError> {
    let allocator = writes.iter().find_map(|write| match write {
        ReplayWrite::ObjectAllocatorSet { next_id } => Some(*next_id),
        _ => None,
    });
    let max_set_id = writes.iter().filter_map(|write| match write {
        ReplayWrite::ObjectSet { object } => Some(object.id.0),
        _ => None,
    });
    if let (Some(next_id), Some(max_id)) = (allocator, max_set_id.max())
        && next_id <= max_id
    {
        return Err(AuthoritativeBundleError::AllocatorBehindObject { next_id, max_id });
    }
    Ok(())
}

fn bundle_hash_material(draft: &AuthoritativeBundleDraft, schema_version: u16) -> Value {
    let mut material = json!({
        "schema_version": schema_version,
        "token": draft.token,
        "request": draft.request,
        "idempotency": draft.idempotency,
        "ingress_hash": draft.ingress_hash,
        "request_hash": draft.request_hash,
        "plan_hash": draft.plan_hash,
        "action": draft.action,
        "provider": draft.provider,
        "cause": draft.cause,
        "definitions": draft.definitions,
        "writes": draft.writes,
        "outcome": draft.outcome,
        "eligible_witnesses": draft.eligible_witnesses,
        "durable_outbox": draft.durable_outbox,
    });
    if schema_version >= AuthoritativeBundle::SEMANTIC_EVENT_SCHEMA_VERSION {
        material
            .as_object_mut()
            .expect("bundle hash material is an object")
            .insert("semantic_events".to_owned(), json!(draft.semantic_events));
    }
    if schema_version >= AuthoritativeBundle::RESOLUTION_SCHEMA_VERSION {
        material
            .as_object_mut()
            .expect("bundle hash material is always an object")
            .insert("resolution".to_owned(), json!(draft.resolution));
    }
    if schema_version >= AuthoritativeBundle::SCHEMA_VERSION {
        let ledger_seals: Vec<_> = draft
            .writes
            .iter()
            .filter_map(|write| match write {
                ReplayWrite::LedgerChunkSet { write } => write.seal(),
                _ => None,
            })
            .collect();
        material
            .as_object_mut()
            .expect("bundle hash material is always an object")
            .insert("ledger_seals".to_owned(), json!(ledger_seals));
    }
    material
}

/// Rejected K3 bundle construction or persisted wire input.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AuthoritativeBundleError {
    #[error("request-key idempotency identity does not match the bundle request")]
    RequestKeyMismatch,
    #[error("the causal root does not match the bundle request")]
    CauseRequestMismatch,
    #[error("the idempotency scope owner does not match the bundle provider")]
    IdempotencyOwnerMismatch,
    #[error("the primary action is not owned by the named provider")]
    WrongActionProvider,
    #[error("the bundle contains a duplicate exact definition")]
    DuplicateDefinition,
    #[error("the bundle assigns conflicting digests to definition {locator:?}")]
    ConflictingDefinition { locator: Box<DefinitionLocator> },
    #[error("persisted bundle definitions are not in canonical order")]
    NonCanonicalDefinitions,
    #[error("bundle definitions contain {actual} entries; maximum is {max}")]
    DefinitionLimit { actual: usize, max: usize },
    #[error("bundle omits required exact definition {definition}")]
    MissingDefinition { definition: Box<DefinitionRef> },
    #[error("authoritative bundle v1 cannot contain semantic events")]
    LegacySemanticEvents,
    #[error("authoritative bundle before v3 cannot contain a resolution record")]
    LegacyResolution,
    #[error("authoritative bundle v3 omits its resolution record")]
    MissingResolution,
    #[error("resolution record names another commit token")]
    ResolutionTokenMismatch,
    #[error("resolution record outcome does not match the bundle outcome")]
    ResolutionOutcomeMismatch,
    #[error("authoritative bundle omits its root semantic event")]
    MissingRootSemanticEvent,
    #[error("bundle semantic events contain {actual} entries; maximum is {max}")]
    SemanticEventLimit { actual: usize, max: u32 },
    #[error("semantic event ordinal is {actual}; expected contiguous ordinal {expected}")]
    SemanticEventOrdinal { expected: u32, actual: u32 },
    #[error("semantic event {ordinal} belongs to a different root request")]
    SemanticEventRequestMismatch { ordinal: u32 },
    #[error("semantic event {ordinal} is not owned by the bundle provider")]
    SemanticEventOwnerMismatch { ordinal: u32 },
    #[error("the root semantic event does not match bundle cause/outcome")]
    RootSemanticEventMismatch,
    #[error("semantic event {ordinal} witnesses are not canonical and duplicate-free")]
    NonCanonicalEventWitnesses { ordinal: u32 },
    #[error("semantic event {ordinal} has {actual} witnesses; maximum is {max}")]
    EventWitnessLimit {
        ordinal: u32,
        actual: usize,
        max: usize,
    },
    #[error("semantic event {ordinal} exceeds {max} canonical bytes")]
    SemanticEventTooLarge { ordinal: u32, max: usize },
    #[error("persisted bundle replay writes are not in canonical target order")]
    NonCanonicalWrites,
    #[error("bundle writes target {target} more than once")]
    DuplicateWrite { target: Box<str> },
    #[error("bundle replay writes contain {actual} entries; maximum is {max}")]
    ReplayWriteLimit { actual: usize, max: usize },
    #[error("authoritative bundle before v4 cannot contain Ledger replay writes")]
    LegacyLedgerWrite,
    #[error("persisted authoritative Ledger seal set does not match its replay writes")]
    LedgerSealSetMismatch,
    #[error("object allocator next_id {next_id} does not exceed created object id {max_id}")]
    AllocatorBehindObject { next_id: u32, max_id: u32 },
    #[error("persisted bundle witnesses are not strictly canonical and duplicate-free")]
    NonCanonicalWitnesses,
    #[error("bundle witnesses contain {actual} entries; maximum is {max}")]
    WitnessLimit { actual: usize, max: usize },
    #[error("persisted bundle outbox intents are not in canonical ordinal order")]
    NonCanonicalOutbox,
    #[error("bundle outbox contains {actual} entries; maximum is {max}")]
    OutboxLimit { actual: usize, max: usize },
    #[error("durable outbox ordinal is {actual}; expected contiguous ordinal {expected}")]
    OutboxOrdinal { expected: usize, actual: u16 },
    #[error("durable outbox payload {ordinal} exceeds {max} canonical bytes")]
    OutboxPayloadTooLarge { ordinal: u16, max: usize },
    #[error("stored {field} does not match canonical bundle content")]
    HashMismatch { field: &'static str },
    #[error("authoritative bundle exceeds {max} canonical bytes")]
    BundleTooLarge { max: usize },
    #[error("canonical authoritative hash failed: {0}")]
    Hash(#[from] CanonicalHashError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mechanics::{
        DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, DefinitionRef,
        DefinitionRegistryBuilder,
    };
    use crate::obj::{GOD, ObjKind};
    use crate::{IdempotencyScope, ResolutionKeyRing, ResolutionSecret};

    fn hash(byte: u8) -> CanonicalHash {
        CanonicalHash::from_bytes([byte; 32])
    }

    fn definitions() -> (DefinitionRef, DefinitionRef) {
        let owner = OwnerId::new("toy").unwrap();
        let mut builder = DefinitionRegistryBuilder::new();
        let action = builder
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("brace").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let outcome = builder
            .register(
                owner,
                DefinitionDescriptor::new(
                    DefinitionKind::new("outcome").unwrap(),
                    DefinitionId::new("braced").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        (action, outcome)
    }

    fn draft() -> AuthoritativeBundleDraft {
        let request = RequestUid::from_bytes([1; 16]);
        let principal = PrincipalId::Object { id: GOD };
        let idempotency = ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: OwnerId::new("toy").unwrap(),
                namespace: CanonicalId::new("brace").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        );
        let (action, outcome_definition) = definitions();
        let object = Obj::new(ObjId(7), ObjKind::Thing, "guard", GOD, 0);
        let cause = PreparedCausalRef::new(request, None, 0).unwrap();
        let outcome = RulesValue::new(outcome_definition.clone(), json!({ "ok": true })).unwrap();
        let witnesses = vec![PrincipalId::Object { id: ObjId(9) }, principal.clone()];
        let root_event = AuthoritativeSemanticEvent::new(
            cause,
            action.key().owner.clone(),
            CanonicalId::new("brace").unwrap(),
            outcome.clone(),
            witnesses.clone(),
        )
        .unwrap();
        AuthoritativeBundleDraft {
            token: CommitToken::from_bytes([2; 16]),
            request,
            idempotency,
            ingress_hash: hash(3),
            request_hash: hash(4),
            plan_hash: hash(5),
            provider: action.key().owner.clone(),
            cause,
            definitions: vec![outcome_definition.clone(), action.clone()],
            writes: vec![
                ReplayWrite::ModuleDelete {
                    namespace: OwnerId::new("toy").unwrap(),
                    key: AuthoritativeKey::parse_canonical("old").unwrap(),
                },
                ReplayWrite::ObjectAllocatorSet { next_id: 8 },
                ReplayWrite::ObjectSet { object },
            ],
            action,
            outcome,
            resolution: None,
            semantic_events: vec![root_event],
            eligible_witnesses: witnesses,
            durable_outbox: vec![],
        }
    }

    fn resolved_draft() -> AuthoritativeBundleDraft {
        let mut draft = draft();
        let domain = CanonicalId::new("brace_resolution").unwrap();
        let mut rng = ResolutionKeyRing::new(ResolutionSecret::new(1, [0x61; 32]).unwrap(), [])
            .unwrap()
            .start(&draft.idempotency, &domain, draft.request_hash)
            .unwrap();
        rng.draw_bounded_u64(&CanonicalId::new("effect_duration").unwrap(), 3)
            .unwrap();
        draft.resolution = Some(
            rng.into_prepared(
                draft.definitions.clone(),
                RulesValue::new(draft.outcome.schema().clone(), json!({ "focus_spent": 1 }))
                    .unwrap(),
                draft.outcome.clone(),
            )
            .unwrap()
            .bind(draft.token),
        );
        draft
    }

    #[test]
    fn bundle_normalizes_once_then_round_trips_strictly() {
        let bundle = AuthoritativeBundle::new(draft()).unwrap();
        assert_eq!(bundle.schema_version, AuthoritativeBundle::SCHEMA_VERSION);
        assert_eq!(bundle.writes()[0].target(), ReplayTarget::Object(ObjId(7)));
        let encoded = serde_json::to_string(&bundle).unwrap();
        let decoded: AuthoritativeBundle = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, bundle);
    }

    #[test]
    fn resolution_record_is_commit_outcome_and_definition_bound() {
        let draft = resolved_draft();
        let domain = CanonicalId::new("brace_resolution").unwrap();
        let bundle = AuthoritativeBundle::new(draft.clone()).unwrap();
        assert_eq!(bundle.schema_version, AuthoritativeBundle::SCHEMA_VERSION);
        assert!(bundle.resolution().is_some());
        let encoded = serde_json::to_value(&bundle).unwrap();
        assert_eq!(encoded["schema_version"], 4);
        assert!(encoded.get("resolution").is_some());
        assert_eq!(
            serde_json::from_value::<AuthoritativeBundle>(encoded).unwrap(),
            bundle
        );

        let mut wrong_token = draft;
        let mut rng = ResolutionKeyRing::new(ResolutionSecret::new(1, [0x61; 32]).unwrap(), [])
            .unwrap()
            .start(&wrong_token.idempotency, &domain, wrong_token.request_hash)
            .unwrap();
        rng.draw_u32(&CanonicalId::new("effect_duration").unwrap())
            .unwrap();
        wrong_token.resolution = Some(
            rng.into_prepared(
                wrong_token.definitions.clone(),
                RulesValue::new(
                    wrong_token.outcome.schema().clone(),
                    json!({ "focus_spent": 1 }),
                )
                .unwrap(),
                wrong_token.outcome.clone(),
            )
            .unwrap()
            .bind(CommitToken::from_bytes([0x99; 16])),
        );
        assert_eq!(
            AuthoritativeBundle::new(wrong_token).unwrap_err(),
            AuthoritativeBundleError::ResolutionTokenMismatch
        );
    }

    #[test]
    fn legacy_v1_bundle_round_trips_without_inventing_events() {
        let mut legacy = draft();
        legacy.semantic_events.clear();
        let bundle = AuthoritativeBundle::build(
            legacy,
            AuthoritativeBundle::LEGACY_SCHEMA_VERSION,
            false,
            None,
            None,
        )
        .unwrap();
        let encoded = serde_json::to_value(&bundle).unwrap();
        assert_eq!(encoded["schema_version"], 1);
        assert!(encoded.get("semantic_events").is_none());
        let decoded: AuthoritativeBundle = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, bundle);
        assert!(decoded.semantic_events().is_empty());
    }

    #[test]
    fn legacy_v2_and_v3_bundles_remain_strictly_readable() {
        let v2 = AuthoritativeBundle::build(
            draft(),
            AuthoritativeBundle::SEMANTIC_EVENT_SCHEMA_VERSION,
            false,
            None,
            None,
        )
        .unwrap();
        let encoded_v2 = serde_json::to_value(&v2).unwrap();
        assert_eq!(encoded_v2["schema_version"], 2);
        assert!(encoded_v2.get("resolution").is_none());
        assert!(encoded_v2.get("ledger_seals").is_none());
        assert_eq!(
            serde_json::from_value::<AuthoritativeBundle>(encoded_v2).unwrap(),
            v2
        );

        let v3 = AuthoritativeBundle::build(
            resolved_draft(),
            AuthoritativeBundle::RESOLUTION_SCHEMA_VERSION,
            false,
            None,
            None,
        )
        .unwrap();
        let encoded_v3 = serde_json::to_value(&v3).unwrap();
        assert_eq!(encoded_v3["schema_version"], 3);
        assert!(encoded_v3.get("resolution").is_some());
        assert!(encoded_v3.get("ledger_seals").is_none());
        assert_eq!(
            serde_json::from_value::<AuthoritativeBundle>(encoded_v3).unwrap(),
            v3
        );
    }

    #[test]
    fn semantic_event_graph_is_contiguous_and_root_bound() {
        let mut missing = draft();
        missing.semantic_events.clear();
        assert_eq!(
            AuthoritativeBundle::new(missing).unwrap_err(),
            AuthoritativeBundleError::MissingRootSemanticEvent
        );

        let mut skipped = draft();
        let root = skipped.semantic_events[0].clone();
        skipped.semantic_events.push(
            AuthoritativeSemanticEvent::new(
                PreparedCausalRef::new(skipped.request, Some(root.cause().event_id()), 2).unwrap(),
                root.owner().clone(),
                CanonicalId::new("derived").unwrap(),
                root.truth().clone(),
                root.eligible_witnesses().to_vec(),
            )
            .unwrap(),
        );
        assert_eq!(
            AuthoritativeBundle::new(skipped).unwrap_err(),
            AuthoritativeBundleError::SemanticEventOrdinal {
                expected: 1,
                actual: 2,
            }
        );
    }

    #[test]
    fn duplicate_targets_and_allocator_regression_are_refused() {
        let mut wrong_scope = draft();
        wrong_scope.idempotency = ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: PrincipalId::Object { id: GOD },
                owner: OwnerId::new("other").unwrap(),
                namespace: CanonicalId::new("brace").unwrap(),
            },
            0,
            IdempotencyKey::Request(wrong_scope.request),
        );
        assert_eq!(
            AuthoritativeBundle::new(wrong_scope).unwrap_err(),
            AuthoritativeBundleError::IdempotencyOwnerMismatch
        );

        let mut duplicate = draft();
        duplicate
            .writes
            .push(ReplayWrite::ObjectDelete { id: ObjId(7) });
        assert!(matches!(
            AuthoritativeBundle::new(duplicate),
            Err(AuthoritativeBundleError::DuplicateWrite { .. })
        ));

        let mut allocator = draft();
        allocator
            .writes
            .retain(|write| !matches!(write, ReplayWrite::ObjectAllocatorSet { .. }));
        allocator
            .writes
            .push(ReplayWrite::ObjectAllocatorSet { next_id: 7 });
        assert_eq!(
            AuthoritativeBundle::new(allocator).unwrap_err(),
            AuthoritativeBundleError::AllocatorBehindObject {
                next_id: 7,
                max_id: 7,
            }
        );

        let mut conflicting_definition = draft();
        let existing = conflicting_definition.action.clone();
        conflicting_definition.definitions.push(
            DefinitionRef::new(
                existing.key().clone(),
                existing.version(),
                DefinitionDigest::from_bytes([99; 32]),
            )
            .unwrap(),
        );
        assert!(matches!(
            AuthoritativeBundle::new(conflicting_definition),
            Err(AuthoritativeBundleError::ConflictingDefinition { .. })
        ));
    }

    #[test]
    fn ledger_write_embeds_exact_seal_and_collides_on_its_physical_module_key() {
        let owner = OwnerId::new("toy").unwrap();
        let mut ledger = crate::ledger::Ledger::with_chunk_target(0);
        let prepared = ledger
            .prepare_append("economy", json!({ "amount": 7 }))
            .unwrap();
        let ledger_write = prepared.authoritative_replay_write(owner.clone()).unwrap();
        let ReplayWrite::LedgerChunkSet { write } = &ledger_write else {
            panic!("prepared Ledger append returned another replay kind")
        };
        let seal = write.seal().expect("target-zero append seals");
        assert_eq!(write.key().as_str(), "log:economy:0");

        let mut with_ledger = draft();
        with_ledger.writes.push(ledger_write.clone());
        let bundle = AuthoritativeBundle::new(with_ledger).unwrap();
        assert_eq!(bundle.ledger_seals(), &[seal]);
        let encoded = serde_json::to_value(&bundle).unwrap();
        assert_eq!(
            serde_json::from_value::<AuthoritativeBundle>(encoded.clone()).unwrap(),
            bundle
        );

        let mut tampered = encoded.clone();
        tampered["ledger_seals"][0] = serde_json::to_value(hash(0xee)).unwrap();
        assert!(serde_json::from_value::<AuthoritativeBundle>(tampered).is_err());

        let mut tampered_write = encoded;
        let ledger_wire = tampered_write["writes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|value| value["kind"] == "ledger_chunk_set")
            .expect("serialized Ledger replay write");
        ledger_wire["write"]["seal"] = serde_json::to_value(hash(0xef)).unwrap();
        assert!(serde_json::from_value::<AuthoritativeBundle>(tampered_write).is_err());

        let mut collision = draft();
        collision.writes.extend([
            ledger_write,
            ReplayWrite::ModuleJsonSet {
                namespace: owner,
                key: AuthoritativeKey::parse_canonical("log:economy:0").unwrap(),
                value: json!({ "not": "the ledger" }),
            },
        ]);
        assert!(matches!(
            AuthoritativeBundle::new(collision),
            Err(AuthoritativeBundleError::DuplicateWrite { .. })
        ));

        let (seq, legacy_write) = ledger.apply_prepared(prepared).unwrap();
        assert_eq!(seq, 0);
        assert!(matches!(
            legacy_write,
            crate::module::ModuleWrite::JsonSet { .. }
        ));

        let open_ledger = crate::ledger::Ledger::with_chunk_target(MAX_SAFE_CHUNK_BYTES);
        let open_prepared = open_ledger
            .prepare_append("open", json!({ "amount": 8 }))
            .unwrap();
        let open_replay = open_prepared
            .authoritative_replay_write(OwnerId::new("toy").unwrap())
            .unwrap();
        let ReplayWrite::LedgerChunkSet { write } = &open_replay else {
            panic!("prepared open Ledger append returned another replay kind")
        };
        assert!(write.seal().is_none());
        let mut open_draft = draft();
        open_draft.writes.push(open_replay);
        let open_bundle = AuthoritativeBundle::new(open_draft).unwrap();
        assert!(open_bundle.ledger_seals().is_empty());
        assert_eq!(
            serde_json::to_value(open_bundle).unwrap()["ledger_seals"],
            json!([])
        );
    }

    #[test]
    fn persisted_hash_and_order_tampering_fail_closed() {
        let bundle = AuthoritativeBundle::new(draft()).unwrap();
        let mut value = serde_json::to_value(bundle).unwrap();
        value["plan_hash"] = serde_json::to_value(hash(99)).unwrap();
        assert!(serde_json::from_value::<AuthoritativeBundle>(value).is_err());

        let bundle = AuthoritativeBundle::new(draft()).unwrap();
        let mut value = serde_json::to_value(bundle).unwrap();
        value["definitions"].as_array_mut().unwrap().reverse();
        assert!(serde_json::from_value::<AuthoritativeBundle>(value).is_err());
    }

    #[test]
    fn durable_outbox_identity_is_parent_and_ordinal_stable() {
        let first = DurableOutboxIntent::new(
            0,
            OwnerId::new("toy").unwrap(),
            CanonicalId::new("discord").unwrap(),
            json!({ "content": "braced" }),
        )
        .unwrap();
        let parent = CommitToken::from_bytes([7; 16]);
        assert_eq!(
            first.stable_id(parent).unwrap(),
            first.stable_id(parent).unwrap()
        );
        assert_ne!(
            first.stable_id(parent).unwrap(),
            first.stable_id(CommitToken::from_bytes([8; 16])).unwrap()
        );
    }

    #[test]
    fn key_and_bundle_wire_are_strict() {
        assert!(AuthoritativeKey::parse_canonical("action-state/v1/family/1").is_ok());
        assert!(AuthoritativeKey::parse_canonical("Upper").is_err());
        let mut value = serde_json::to_value(AuthoritativeBundle::new(draft()).unwrap()).unwrap();
        value["unknown"] = json!(true);
        assert!(serde_json::from_value::<AuthoritativeBundle>(value).is_err());

        let encoded = serde_json::to_string(&AuthoritativeBundle::new(draft()).unwrap()).unwrap();
        let duplicated = encoded.replacen(
            "{\"schema_version\":4",
            "{\"schema_version\":4,\"schema_version\":4",
            1,
        );
        assert!(serde_json::from_str::<AuthoritativeBundle>(&duplicated).is_err());
    }
}
