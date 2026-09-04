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
    CanonicalHash, CanonicalHashError, CommitToken, IdempotencyKey, PreparedCausalRef, PrincipalId,
    RequestUid, ScopedIdempotencyKey,
};
use crate::mechanics::{
    CanonicalId, DefinitionLocator, DefinitionRef, OwnerId, RulesValue,
    canonical_json_bytes_with_limit,
};
use crate::module::MAX_MODULE_KEY_BYTES;
use crate::obj::{Obj, ObjId};

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
    outcome: RulesValue,
    eligible_witnesses: Vec<PrincipalId>,
    durable_outbox: Vec<DurableOutboxIntent>,
    bundle_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeBundleWire {
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

impl AuthoritativeBundle {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn new(draft: AuthoritativeBundleDraft) -> Result<Self, AuthoritativeBundleError> {
        Self::build(draft, false, None)
    }

    fn build(
        mut draft: AuthoritativeBundleDraft,
        require_canonical_order: bool,
        claimed_hash: Option<CanonicalHash>,
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

        let hash_material = bundle_hash_material(&draft);
        let bundle_hash = CanonicalHash::for_canonical_value(
            "authoritative_bundle.v1",
            Self::SCHEMA_VERSION,
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
            schema_version: Self::SCHEMA_VERSION,
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
            outcome: draft.outcome,
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
    pub const fn outcome(&self) -> &RulesValue {
        &self.outcome
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
        if wire.schema_version != Self::SCHEMA_VERSION {
            return Err(D::Error::custom(format!(
                "unsupported authoritative bundle schema {}; expected {}",
                wire.schema_version,
                Self::SCHEMA_VERSION
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
                eligible_witnesses: wire.eligible_witnesses,
                durable_outbox: wire.durable_outbox,
            },
            true,
            Some(wire.bundle_hash),
        )
        .map_err(D::Error::custom)
    }
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

fn bundle_hash_material(draft: &AuthoritativeBundleDraft) -> Value {
    json!({
        "schema_version": AuthoritativeBundle::SCHEMA_VERSION,
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
    })
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
    #[error("persisted bundle replay writes are not in canonical target order")]
    NonCanonicalWrites,
    #[error("bundle writes target {target} more than once")]
    DuplicateWrite { target: Box<str> },
    #[error("bundle replay writes contain {actual} entries; maximum is {max}")]
    ReplayWriteLimit { actual: usize, max: usize },
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
    use crate::IdempotencyScope;
    use crate::mechanics::{
        DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, DefinitionRef,
        DefinitionRegistryBuilder,
    };
    use crate::obj::{GOD, ObjKind};

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
        AuthoritativeBundleDraft {
            token: CommitToken::from_bytes([2; 16]),
            request,
            idempotency,
            ingress_hash: hash(3),
            request_hash: hash(4),
            plan_hash: hash(5),
            provider: action.key().owner.clone(),
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
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
            outcome: RulesValue::new(outcome_definition, json!({ "ok": true })).unwrap(),
            eligible_witnesses: vec![PrincipalId::Object { id: ObjId(9) }, principal],
            durable_outbox: vec![],
        }
    }

    #[test]
    fn bundle_normalizes_once_then_round_trips_strictly() {
        let bundle = AuthoritativeBundle::new(draft()).unwrap();
        assert_eq!(bundle.writes()[0].target(), ReplayTarget::Object(ObjId(7)));
        let encoded = serde_json::to_string(&bundle).unwrap();
        let decoded: AuthoritativeBundle = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, bundle);
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
    }
}
