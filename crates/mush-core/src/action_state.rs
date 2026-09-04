//! Engine-owned typed state families and bounded logical action transactions.
//!
//! State-family handles are capabilities minted only while composing the
//! concrete [`GameModule`](crate::GameModule) that owns them.  An
//! [`ActionTxn`] can read live state but accumulates validated, canonically
//! encoded post-images in private scratch storage.  Preparing the transaction
//! reserves the store and produces a one-shot [`PreparedTxn`]; live state is
//! changed only when its [`PreparedInstallation`] is installed after the exact
//! persistence acknowledgement.

use std::any::{Any, TypeId, type_name};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;
use std::ops::Bound;
use std::sync::Arc;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use smol_str::SmolStr;
use thiserror::Error;

use crate::action::{MAX_CASCADE_OPERATIONS, ProviderRegistrationOrdinal};
use crate::authoritative::{AuthoritativeKey, ReplayWrite};
use crate::mechanics::{
    CanonicalEncodingError, CanonicalId, CanonicalIdError, DefinitionDigest,
    MechanicalOwnerReadiness, MechanicalOwnerReadinessError, OwnerId,
    canonical_json_bytes_with_limit,
};
use crate::module::{MAX_MODULE_KEY_BYTES, ModuleOp, ModuleWrite};

/// Maximum state families in one composed deployment.
pub const MAX_STATE_FAMILIES: usize = 4_096;
/// Maximum state families declared by one owner.
pub const MAX_STATE_FAMILIES_PER_OWNER: usize = 1_024;
/// Hard maximum encoded key bytes for one state-family record.
pub const MAX_STATE_KEY_BYTES: usize = 1_024;
/// Hard maximum canonical bytes in one state-family record.
pub const MAX_ACTION_STATE_RECORD_BYTES: usize = 256 * 1024;
/// Hard maximum canonical bytes in one logical action bundle.
pub const MAX_ACTION_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
/// Hard maximum distinct record mutations in one logical action.
pub const MAX_ACTION_MUTATION_OPERATIONS: usize = MAX_CASCADE_OPERATIONS as usize;
/// Hard maximum records accepted during one typed-state boot hydration.
pub const MAX_ACTION_STATE_BOOT_RECORDS: usize = 1_000_000;
/// Hard maximum canonical record bytes retained during one boot hydration.
pub const MAX_ACTION_STATE_BOOT_BYTES: usize = 512 * 1024 * 1024;
/// Maximum typed records returned by one synchronous family scan.
pub const MAX_STATE_FAMILY_SCAN_RECORDS: usize = 1_024;

/// Reserved module-key root owned exclusively by the action-state coordinator.
/// Ordinary module boot, migrations, and legacy writes must never consume it.
pub const ACTION_STATE_PERSISTENCE_ROOT: &str = "action-state/";
const STATE_PERSISTENCE_PREFIX: &str = "action-state/v1";
const MAX_STATE_CALLBACK_ERROR_BYTES: usize = 512;

/// Whether a module key belongs to the coordinator-reserved action-state tree.
/// All versions are reserved so an older binary cannot hand a future encoding
/// to an ordinary module or its eager migration callbacks.
#[must_use]
pub fn is_action_state_persistence_key(key: &str) -> bool {
    key.starts_with(ACTION_STATE_PERSISTENCE_ROOT)
}

/// Owner-local canonical identity of an authoritative state family.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StateFamilyId(CanonicalId);

impl StateFamilyId {
    /// Canonicalize trusted registration input.
    pub fn new(value: impl AsRef<str>) -> Result<Self, CanonicalIdError> {
        CanonicalId::new(value).map(Self)
    }

    /// Parse persisted input without silently normalizing it.
    pub fn parse_canonical(value: impl AsRef<str>) -> Result<Self, CanonicalIdError> {
        CanonicalId::parse_canonical(value).map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for StateFamilyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("StateFamilyId")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for StateFamilyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for StateFamilyId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StateFamilyId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse_canonical(value).map_err(D::Error::custom)
    }
}

/// Stable, reviewable description of a typed state family.
///
/// Callback implementation details are deliberately represented by canonical
/// contract IDs. Changing a key codec, schema, decoder, validator, or
/// serializer must change its corresponding ID or schema version and therefore
/// changes the deployment fingerprint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateFamilyDescriptor {
    id: StateFamilyId,
    key_codec: CanonicalId,
    value_schema: CanonicalId,
    schema_version: u16,
    decoder: CanonicalId,
    validator: CanonicalId,
    serializer: CanonicalId,
    max_key_bytes: usize,
    max_record_bytes: usize,
}

impl StateFamilyDescriptor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: StateFamilyId,
        key_codec: CanonicalId,
        value_schema: CanonicalId,
        schema_version: u16,
        decoder: CanonicalId,
        validator: CanonicalId,
        serializer: CanonicalId,
    ) -> Result<Self, StateFamilyDescriptorError> {
        if schema_version == 0 {
            return Err(StateFamilyDescriptorError::ZeroSchemaVersion);
        }
        Ok(Self {
            id,
            key_codec,
            value_schema,
            schema_version,
            decoder,
            validator,
            serializer,
            max_key_bytes: MAX_STATE_KEY_BYTES,
            max_record_bytes: MAX_ACTION_STATE_RECORD_BYTES,
        })
    }

    /// Tighten this family's per-key and per-record limits.
    pub fn with_limits(
        mut self,
        max_key_bytes: usize,
        max_record_bytes: usize,
    ) -> Result<Self, StateFamilyDescriptorError> {
        if max_key_bytes == 0 || max_key_bytes > MAX_STATE_KEY_BYTES {
            return Err(StateFamilyDescriptorError::InvalidKeyLimit {
                attempted: max_key_bytes,
                max: MAX_STATE_KEY_BYTES,
            });
        }
        if max_record_bytes == 0 || max_record_bytes > MAX_ACTION_STATE_RECORD_BYTES {
            return Err(StateFamilyDescriptorError::InvalidRecordLimit {
                attempted: max_record_bytes,
                max: MAX_ACTION_STATE_RECORD_BYTES,
            });
        }
        self.max_key_bytes = max_key_bytes;
        self.max_record_bytes = max_record_bytes;
        Ok(self)
    }

    #[must_use]
    pub const fn id(&self) -> &StateFamilyId {
        &self.id
    }

    #[must_use]
    pub const fn key_codec(&self) -> &CanonicalId {
        &self.key_codec
    }

    #[must_use]
    pub const fn value_schema(&self) -> &CanonicalId {
        &self.value_schema
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn decoder(&self) -> &CanonicalId {
        &self.decoder
    }

    #[must_use]
    pub const fn validator(&self) -> &CanonicalId {
        &self.validator
    }

    #[must_use]
    pub const fn serializer(&self) -> &CanonicalId {
        &self.serializer
    }

    #[must_use]
    pub const fn max_key_bytes(&self) -> usize {
        self.max_key_bytes
    }

    #[must_use]
    pub const fn max_record_bytes(&self) -> usize {
        self.max_record_bytes
    }

    pub(crate) fn canonical_value(&self) -> Value {
        json!({
            "id": self.id,
            "key_codec": self.key_codec,
            "value_schema": self.value_schema,
            "schema_version": self.schema_version,
            "decoder": self.decoder,
            "validator": self.validator,
            "serializer": self.serializer,
            "max_key_bytes": self.max_key_bytes,
            "max_record_bytes": self.max_record_bytes,
        })
    }
}

/// Invalid state-family metadata or attempted limit widening.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StateFamilyDescriptorError {
    #[error("state-family schema version must be non-zero")]
    ZeroSchemaVersion,
    #[error("state-family key limit {attempted} is outside 1..={max}")]
    InvalidKeyLimit { attempted: usize, max: usize },
    #[error("state-family record limit {attempted} is outside 1..={max}")]
    InvalidRecordLimit { attempted: usize, max: usize },
}

/// Typed key codec registered for a state family.
pub trait StateKeyCodec<K>: Send + Sync {
    fn encode_key(&self, key: &K) -> Result<String, String>;
}

impl<K, F> StateKeyCodec<K> for F
where
    F: Fn(&K) -> Result<String, String> + Send + Sync,
{
    fn encode_key(&self, key: &K) -> Result<String, String> {
        self(key)
    }
}

/// Pure typed invariant validator registered for a state family.
pub trait StateValueValidator<V>: Send + Sync {
    fn validate_value(&self, value: &V) -> Result<(), String>;
}

/// Strict persisted-JSON-to-typed-value decoder registered for a state family.
/// A decoder may not silently retain unknown meaning: hydration reserializes
/// its result and requires canonical equality with the persisted envelope.
pub trait StateValueDecoder<V>: Send + Sync {
    fn decode_value(&self, value: &Value) -> Result<V, String>;
}

impl<V, F> StateValueDecoder<V> for F
where
    F: Fn(&Value) -> Result<V, String> + Send + Sync,
{
    fn decode_value(&self, value: &Value) -> Result<V, String> {
        self(value)
    }
}

impl<V, F> StateValueValidator<V> for F
where
    F: Fn(&V) -> Result<(), String> + Send + Sync,
{
    fn validate_value(&self, value: &V) -> Result<(), String> {
        self(value)
    }
}

/// Canonical typed-to-JSON projection registered for a state family.
pub trait StateValueSerializer<V>: Send + Sync {
    fn serialize_value(&self, value: &V) -> Result<Value, String>;
}

impl<V, F> StateValueSerializer<V> for F
where
    F: Fn(&V) -> Result<Value, String> + Send + Sync,
{
    fn serialize_value(&self, value: &V) -> Result<Value, String> {
        self(value)
    }
}

trait ErasedStateFamily: Send + Sync {
    fn owner(&self) -> &OwnerId;
    fn descriptor(&self) -> &StateFamilyDescriptor;
    fn key_type_id(&self) -> TypeId;
    fn value_type_id(&self) -> TypeId;
    fn key_type_name(&self) -> &'static str;
    fn value_type_name(&self) -> &'static str;
    fn decode_and_project(
        &self,
        key: &StateKey,
        value: &Value,
    ) -> Result<(Box<dyn ErasedStateValue>, Value), ActionStateError>;
    fn as_any(&self) -> &dyn Any;
}

struct TypedStateFamily<K, V> {
    owner: OwnerId,
    descriptor: StateFamilyDescriptor,
    key_codec: Arc<dyn StateKeyCodec<K>>,
    decoder: Arc<dyn StateValueDecoder<V>>,
    validator: Arc<dyn StateValueValidator<V>>,
    serializer: Arc<dyn StateValueSerializer<V>>,
}

impl<K: 'static, V: Send + Sync + 'static> ErasedStateFamily for TypedStateFamily<K, V> {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn descriptor(&self) -> &StateFamilyDescriptor {
        &self.descriptor
    }

    fn key_type_id(&self) -> TypeId {
        TypeId::of::<K>()
    }

    fn value_type_id(&self) -> TypeId {
        TypeId::of::<V>()
    }

    fn key_type_name(&self) -> &'static str {
        type_name::<K>()
    }

    fn value_type_name(&self) -> &'static str {
        type_name::<V>()
    }

    fn decode_and_project(
        &self,
        key: &StateKey,
        value: &Value,
    ) -> Result<(Box<dyn ErasedStateValue>, Value), ActionStateError> {
        let decoded = self.decoder.decode_value(value).map_err(|message| {
            ActionStateError::DecodeRejected {
                key: key.clone(),
                message: bounded_callback_message(message),
            }
        })?;
        self.validator.validate_value(&decoded).map_err(|message| {
            ActionStateError::ValueRejected {
                key: key.clone(),
                message: bounded_callback_message(message),
            }
        })?;
        let projected = self
            .serializer
            .serialize_value(&decoded)
            .map_err(|message| ActionStateError::SerializationRejected {
                key: key.clone(),
                message: bounded_callback_message(message),
            })?;
        Ok((Box::new(decoded), projected))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Opaque owner-stamped capability for one typed state family.
///
/// It has no public constructor and is not serializable. A family with the
/// same textual IDs in a different module composition is a different
/// capability and is rejected by the store.
pub struct StateFamilyHandle<K, V> {
    family: Arc<dyn ErasedStateFamily>,
    marker: PhantomData<fn(&K) -> &V>,
}

impl<K, V> Clone for StateFamilyHandle<K, V> {
    fn clone(&self) -> Self {
        Self {
            family: Arc::clone(&self.family),
            marker: PhantomData,
        }
    }
}

impl<K, V> fmt::Debug for StateFamilyHandle<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateFamilyHandle")
            .field("owner", self.family.owner())
            .field("descriptor", self.family.descriptor())
            .field("key_type", &self.family.key_type_name())
            .field("value_type", &self.family.value_type_name())
            .finish_non_exhaustive()
    }
}

impl<K: 'static, V: 'static> StateFamilyHandle<K, V> {
    #[must_use]
    pub fn owner(&self) -> &OwnerId {
        self.family.owner()
    }

    #[must_use]
    pub fn descriptor(&self) -> &StateFamilyDescriptor {
        self.family.descriptor()
    }

    fn typed(&self) -> &TypedStateFamily<K, V> {
        self.family
            .as_ref()
            .as_any()
            .downcast_ref::<TypedStateFamily<K, V>>()
            .expect("opaque state-family handle retains its exact typed registration")
    }
}

/// Read-only, owner-scoped view of engine-owned typed state supplied to an
/// ordinary module context.
///
/// The backing store and owner stamp are private. A module can use only an
/// opaque family handle minted for its own owner; it cannot mutate state or
/// borrow another owner's family through this surface.
pub struct ModuleActionState<'a> {
    store: Option<&'a ActionStateStore>,
    owner: Option<OwnerId>,
}

impl fmt::Debug for ModuleActionState<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModuleActionState")
            .field("available", &self.store.is_some())
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl<'a> ModuleActionState<'a> {
    /// An unavailable reader for manually constructed compatibility contexts.
    /// Engine-dispatched module contexts always receive a live reader.
    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            store: None,
            owner: None,
        }
    }

    pub(crate) fn for_module(store: Option<&'a ActionStateStore>, module: Option<&str>) -> Self {
        Self {
            store,
            owner: module.and_then(|name| OwnerId::parse_canonical(name).ok()),
        }
    }

    #[must_use]
    pub const fn is_available(&self) -> bool {
        self.store.is_some() && self.owner.is_some()
    }

    /// Read one exact record from a family owned by this context's module.
    pub fn get<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&'a V>, ModuleActionStateReadError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let store = self.store.ok_or(ModuleActionStateReadError::Unavailable)?;
        self.require_owner(handle.owner())?;
        store.get(handle, key).map_err(Into::into)
    }

    /// Scan one bounded canonical page from a family owned by this context's
    /// module. The cursor is the previous page's opaque encoded key.
    pub fn scan_family<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<StateFamilyPage<'a, V>, ModuleActionStateReadError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let store = self.store.ok_or(ModuleActionStateReadError::Unavailable)?;
        self.require_owner(handle.owner())?;
        store.scan_family(handle, after, limit).map_err(Into::into)
    }

    fn require_owner(&self, family: &OwnerId) -> Result<(), ModuleActionStateReadError> {
        let context = self
            .owner
            .as_ref()
            .ok_or(ModuleActionStateReadError::UnownedContext)?;
        if context != family {
            return Err(ModuleActionStateReadError::ForeignOwner {
                context: context.clone(),
                family: family.clone(),
            });
        }
        Ok(())
    }

    pub(crate) const fn store(&self) -> Option<&'a ActionStateStore> {
        self.store
    }
}

/// Refusal from an ordinary module's read-only typed-state capability.
#[derive(Debug, Error)]
pub enum ModuleActionStateReadError {
    #[error("typed action state is unavailable in this compatibility context")]
    Unavailable,
    #[error("core context has no module owner for typed action-state reads")]
    UnownedContext,
    #[error("module {context} cannot read typed state owned by {family}")]
    ForeignOwner { context: OwnerId, family: OwnerId },
    #[error("typed action-state read failed: {0}")]
    State(#[from] ActionStateError),
}

type StateFamilyLocator = (OwnerId, StateFamilyId);

/// Mutable state-family composition table.
#[derive(Default)]
pub struct StateFamilyRegistryBuilder {
    entries: BTreeMap<StateFamilyLocator, Arc<dyn ErasedStateFamily>>,
    owner_counts: BTreeMap<OwnerId, usize>,
}

impl fmt::Debug for StateFamilyRegistryBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateFamilyRegistryBuilder")
            .field("families", &self.entries.keys().collect::<Vec<_>>())
            .field("owner_counts", &self.owner_counts)
            .finish()
    }
}

impl StateFamilyRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register<K, V, KC, VD, VV, VS>(
        &mut self,
        owner: OwnerId,
        descriptor: StateFamilyDescriptor,
        key_codec: KC,
        decoder: VD,
        validator: VV,
        serializer: VS,
    ) -> Result<StateFamilyHandle<K, V>, StateFamilyRegistryError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
        KC: StateKeyCodec<K> + 'static,
        VD: StateValueDecoder<V> + 'static,
        VV: StateValueValidator<V> + 'static,
        VS: StateValueSerializer<V> + 'static,
    {
        if self.entries.len() >= MAX_STATE_FAMILIES {
            return Err(StateFamilyRegistryError::FamilyLimit {
                max: MAX_STATE_FAMILIES,
            });
        }
        let owner_count = self.owner_counts.get(&owner).copied().unwrap_or(0);
        if owner_count >= MAX_STATE_FAMILIES_PER_OWNER {
            return Err(StateFamilyRegistryError::OwnerFamilyLimit {
                owner,
                max: MAX_STATE_FAMILIES_PER_OWNER,
            });
        }
        let locator = (owner.clone(), descriptor.id.clone());
        if let Some(existing) = self.entries.get(&locator) {
            return Err(StateFamilyRegistryError::Duplicate {
                owner,
                family: descriptor.id.clone(),
                registered_key_type: existing.key_type_name(),
                registered_value_type: existing.value_type_name(),
                attempted_key_type: type_name::<K>(),
                attempted_value_type: type_name::<V>(),
            });
        }
        let typed = Arc::new(TypedStateFamily::<K, V> {
            owner: owner.clone(),
            descriptor,
            key_codec: Arc::new(key_codec),
            decoder: Arc::new(decoder),
            validator: Arc::new(validator),
            serializer: Arc::new(serializer),
        });
        let family: Arc<dyn ErasedStateFamily> = typed;
        self.entries.insert(locator, Arc::clone(&family));
        self.owner_counts.insert(owner, owner_count + 1);
        Ok(StateFamilyHandle {
            family,
            marker: PhantomData,
        })
    }

    pub(crate) fn freeze_for_owners_with_readiness(
        self,
        module_owners: &BTreeSet<OwnerId>,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<StateFamilyRegistry, StateFamilyRegistryError> {
        for family in self.entries.values() {
            if !module_owners.contains(family.owner()) {
                return Err(StateFamilyRegistryError::UnknownOwner {
                    owner: family.owner().clone(),
                });
            }
        }
        let fingerprint = state_family_fingerprint(self.entries.values())?;
        Ok(StateFamilyRegistry {
            entries: self.entries,
            fingerprint,
            owner_readiness,
        })
    }
}

/// Frozen state-family registration table.
#[derive(Clone)]
pub struct StateFamilyRegistry {
    entries: BTreeMap<StateFamilyLocator, Arc<dyn ErasedStateFamily>>,
    fingerprint: DefinitionDigest,
    owner_readiness: MechanicalOwnerReadiness,
}

impl fmt::Debug for StateFamilyRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateFamilyRegistry")
            .field("families", &self.entries.keys().collect::<Vec<_>>())
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl StateFamilyRegistry {
    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn get_for_handle<K: 'static, V: 'static>(
        &self,
        handle: &StateFamilyHandle<K, V>,
    ) -> Result<&Arc<dyn ErasedStateFamily>, ActionStateError> {
        let locator = (handle.owner().clone(), handle.descriptor().id.clone());
        let registered =
            self.entries
                .get(&locator)
                .ok_or_else(|| ActionStateError::UnknownFamily {
                    owner: handle.owner().clone(),
                    family: handle.descriptor().id.clone(),
                })?;
        if !Arc::ptr_eq(registered, &handle.family) {
            return Err(ActionStateError::ForeignHandle {
                owner: handle.owner().clone(),
                family: handle.descriptor().id.clone(),
            });
        }
        debug_assert_eq!(registered.key_type_id(), TypeId::of::<K>());
        debug_assert_eq!(registered.value_type_id(), TypeId::of::<V>());
        self.owner_readiness
            .require_ready(handle.owner())
            .map_err(ActionStateError::OwnerNotReady)?;
        Ok(registered)
    }

    pub(crate) fn validate_handle<K: 'static, V: 'static>(
        &self,
        handle: &StateFamilyHandle<K, V>,
    ) -> Result<(), ActionStateError> {
        self.get_for_handle(handle).map(|_| ())
    }

    pub(crate) fn shares_readiness_with(&self, readiness: &MechanicalOwnerReadiness) -> bool {
        self.owner_readiness.shares_latch_with(readiness)
    }
}

fn state_family_fingerprint<'a>(
    entries: impl Iterator<Item = &'a Arc<dyn ErasedStateFamily>>,
) -> Result<DefinitionDigest, CanonicalEncodingError> {
    DefinitionDigest::for_canonical_value(
        "state_family_registry.v1",
        &Value::Array(
            entries
                .map(|family| {
                    json!({
                        "owner": family.owner(),
                        "descriptor": family.descriptor().canonical_value(),
                    })
                })
                .collect(),
        ),
    )
}

/// State-family registration failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StateFamilyRegistryError {
    #[error("state-family registry exceeds its hard limit of {max}")]
    FamilyLimit { max: usize },
    #[error("owner {owner} exceeds its state-family limit of {max}")]
    OwnerFamilyLimit { owner: OwnerId, max: usize },
    #[error(
        "state family {owner}/{family} is already registered as {registered_key_type} -> {registered_value_type}; attempted {attempted_key_type} -> {attempted_value_type}"
    )]
    Duplicate {
        owner: OwnerId,
        family: StateFamilyId,
        registered_key_type: &'static str,
        registered_value_type: &'static str,
        attempted_key_type: &'static str,
        attempted_value_type: &'static str,
    },
    #[error("state family names uncomposed owner {owner}")]
    UnknownOwner { owner: OwnerId },
    #[error("state-family fingerprint is not canonical: {0}")]
    Fingerprint(#[from] CanonicalEncodingError),
}

/// Exact record identity inside the engine-owned action store.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct StateKey {
    owner: OwnerId,
    family: StateFamilyId,
    encoded: SmolStr,
}

impl StateKey {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn family(&self) -> &StateFamilyId {
        &self.family
    }

    #[must_use]
    pub fn encoded(&self) -> &str {
        &self.encoded
    }

    fn persistence_key(&self) -> SmolStr {
        SmolStr::new(format!(
            "{STATE_PERSISTENCE_PREFIX}/{}/{}",
            self.family, self.encoded
        ))
    }
}

/// One persisted action-state document discovered by the server before
/// ordinary module boot. The namespace supplies the owner identity; `key`
/// remains the complete reserved module key so path and envelope can be
/// cross-checked before any typed value becomes live.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionStateBootRecord {
    pub namespace: SmolStr,
    pub key: SmolStr,
    pub value: Value,
}

impl ActionStateBootRecord {
    #[must_use]
    pub fn new(namespace: impl Into<SmolStr>, key: impl Into<SmolStr>, value: Value) -> Self {
        Self {
            namespace: namespace.into(),
            key: key.into(),
            value,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedStateEnvelope {
    family: PersistedStateFamily,
    key: String,
    schema: PersistedStateSchema,
    value: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedStateFamily {
    owner: OwnerId,
    id: StateFamilyId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedStateSchema {
    id: CanonicalId,
    version: u16,
}

/// Immutable bounded canonical JSON bytes retained with a prepared/live value.
#[derive(Clone, Eq, PartialEq)]
pub struct CanonicalStateBytes(Arc<[u8]>);

impl CanonicalStateBytes {
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for CanonicalStateBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CanonicalStateBytes")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

trait ErasedStateValue: Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Send + Sync + 'static> ErasedStateValue for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct StoredStateRecord {
    value: Box<dyn ErasedStateValue>,
    canonical: CanonicalStateBytes,
}

impl fmt::Debug for StoredStateRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredStateRecord")
            .field("canonical", &self.canonical)
            .finish_non_exhaustive()
    }
}

/// Engine-owned live authoritative state. It has no unrestricted mutable map
/// accessor; all gameplay mutation goes through [`ActionTxn`].
pub struct ActionStateStore {
    registry: StateFamilyRegistry,
    records: BTreeMap<StateKey, StoredStateRecord>,
    identity: Arc<()>,
    revision: u64,
    next_reservation: u64,
    pending_reservation: Option<u64>,
}

/// One immutable entry in a bounded typed-family scan.
#[derive(Clone, Copy, Debug)]
pub struct StateFamilyScanEntry<'a, V> {
    encoded_key: &'a str,
    value: &'a V,
}

impl<'a, V> StateFamilyScanEntry<'a, V> {
    #[must_use]
    pub const fn encoded_key(&self) -> &'a str {
        self.encoded_key
    }

    #[must_use]
    pub const fn value(&self) -> &'a V {
        self.value
    }
}

/// Canonically ordered bounded page from one exact typed family.
#[derive(Debug)]
pub struct StateFamilyPage<'a, V> {
    entries: Vec<StateFamilyScanEntry<'a, V>>,
    next_after: Option<SmolStr>,
}

impl<'a, V> StateFamilyPage<'a, V> {
    #[must_use]
    pub fn entries(&self) -> &[StateFamilyScanEntry<'a, V>] {
        &self.entries
    }

    /// Encoded key to pass as `after` for the next page, or `None` when this
    /// page reached the end of the family.
    #[must_use]
    pub const fn next_after(&self) -> Option<&SmolStr> {
        self.next_after.as_ref()
    }
}

impl fmt::Debug for ActionStateStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionStateStore")
            .field("families", &self.registry.len())
            .field("records", &self.records.len())
            .field("revision", &self.revision)
            .field("pending_reservation", &self.pending_reservation)
            .finish()
    }
}

impl ActionStateStore {
    #[must_use]
    pub(crate) fn unregistered() -> Self {
        let owners = BTreeSet::new();
        let readiness = MechanicalOwnerReadiness::for_owners(&owners);
        let registry = StateFamilyRegistryBuilder::new()
            .freeze_for_owners_with_readiness(&owners, readiness)
            .expect("empty state-family registry is canonical");
        Self::new(registry)
    }

    #[must_use]
    pub(crate) fn new(registry: StateFamilyRegistry) -> Self {
        Self {
            registry,
            records: BTreeMap::new(),
            identity: Arc::new(()),
            revision: 0,
            next_reservation: 0,
            pending_reservation: None,
        }
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[must_use]
    pub const fn has_prepared(&self) -> bool {
        self.pending_reservation.is_some()
    }

    /// Atomically hydrate every discovered persisted record through its exact
    /// registered decoder. The live map is replaced only after the complete
    /// input has passed path/envelope/schema/bounds checks, typed validation,
    /// and canonical decode→serialize equality.
    ///
    /// The process-local revision and reservation counters intentionally start
    /// at zero after boot; K3 will place their durable successors in the exact
    /// authoritative bundle alongside idempotency and outcome records.
    pub fn hydrate(
        &mut self,
        records: impl IntoIterator<Item = ActionStateBootRecord>,
    ) -> Result<(), ActionStateError> {
        if !self.records.is_empty() || self.pending_reservation.is_some() || self.revision != 0 {
            return Err(ActionStateError::HydrationStoreNotEmpty);
        }
        let mut hydrated = BTreeMap::new();
        let mut total_bytes = 0usize;
        for record in records {
            if hydrated.len() >= MAX_ACTION_STATE_BOOT_RECORDS {
                return Err(ActionStateError::HydrationRecordLimit {
                    max: MAX_ACTION_STATE_BOOT_RECORDS,
                });
            }
            let state_key = self.hydration_key(&record)?;
            let locator = (state_key.owner.clone(), state_key.family.clone());
            let family = self.registry.entries.get(&locator).ok_or_else(|| {
                ActionStateError::UnknownFamily {
                    owner: state_key.owner.clone(),
                    family: state_key.family.clone(),
                }
            })?;
            // Bound the untrusted persisted document before cloning it into a
            // typed envelope or invoking any module-supplied decoder.
            let actual = canonical_json_bytes_with_limit(
                &record.value,
                family.descriptor().max_record_bytes(),
            )
            .map_err(|source| ActionStateError::CanonicalPostImage {
                key: state_key.clone(),
                source,
            })?;
            total_bytes = total_bytes
                .checked_add(actual.len())
                .ok_or(ActionStateError::ArithmeticOverflow)?;
            if total_bytes > MAX_ACTION_STATE_BOOT_BYTES {
                return Err(ActionStateError::HydrationBytesExceeded {
                    attempted: total_bytes,
                    max: MAX_ACTION_STATE_BOOT_BYTES,
                });
            }
            let envelope: PersistedStateEnvelope = serde_json::from_value(record.value.clone())
                .map_err(|error| ActionStateError::HydrationEnvelope {
                    namespace: record.namespace.clone(),
                    persistence_key: record.key.clone(),
                    message: bounded_callback_message(error.to_string()),
                })?;
            require_hydration_identity(
                &state_key,
                "family.owner",
                state_key.owner.as_str(),
                envelope.family.owner.as_str(),
            )?;
            require_hydration_identity(
                &state_key,
                "family.id",
                state_key.family.as_str(),
                envelope.family.id.as_str(),
            )?;
            require_hydration_identity(&state_key, "key", state_key.encoded(), &envelope.key)?;
            require_hydration_identity(
                &state_key,
                "schema.id",
                family.descriptor().value_schema().as_str(),
                envelope.schema.id.as_str(),
            )?;
            if envelope.schema.version != family.descriptor().schema_version() {
                return Err(ActionStateError::HydrationIdentityMismatch {
                    key: state_key,
                    field: "schema.version",
                    expected: family
                        .descriptor()
                        .schema_version()
                        .to_string()
                        .into_boxed_str(),
                    actual: envelope.schema.version.to_string().into_boxed_str(),
                });
            }
            let (value, projected) = family.decode_and_project(&state_key, &envelope.value)?;
            let expected_document = json!({
                "family": {
                    "owner": &state_key.owner,
                    "id": &state_key.family,
                },
                "key": &state_key.encoded,
                "schema": {
                    "id": family.descriptor().value_schema(),
                    "version": family.descriptor().schema_version(),
                },
                "value": projected,
            });
            let expected = canonical_json_bytes_with_limit(
                &expected_document,
                family.descriptor().max_record_bytes(),
            )
            .map_err(|source| ActionStateError::CanonicalPostImage {
                key: state_key.clone(),
                source,
            })?;
            if actual != expected {
                return Err(ActionStateError::HydrationCanonicalMismatch { key: state_key });
            }
            let canonical = CanonicalStateBytes(Arc::from(expected));
            if hydrated
                .insert(state_key.clone(), StoredStateRecord { value, canonical })
                .is_some()
            {
                return Err(ActionStateError::DuplicateHydratedRecord { key: state_key });
            }
        }
        self.records = hydrated;
        Ok(())
    }

    fn hydration_key(&self, record: &ActionStateBootRecord) -> Result<StateKey, ActionStateError> {
        let owner = OwnerId::parse_canonical(record.namespace.as_str()).map_err(|error| {
            ActionStateError::HydrationPath {
                namespace: record.namespace.clone(),
                persistence_key: record.key.clone(),
                message: bounded_callback_message(error.to_string()),
            }
        })?;
        let prefix = format!("{STATE_PERSISTENCE_PREFIX}/");
        let tail =
            record
                .key
                .strip_prefix(&prefix)
                .ok_or_else(|| ActionStateError::HydrationPath {
                    namespace: record.namespace.clone(),
                    persistence_key: record.key.clone(),
                    message: "unsupported action-state persistence version".to_owned(),
                })?;
        let (family_text, encoded) =
            tail.split_once('/')
                .ok_or_else(|| ActionStateError::HydrationPath {
                    namespace: record.namespace.clone(),
                    persistence_key: record.key.clone(),
                    message: "action-state path must contain family and encoded key".to_owned(),
                })?;
        let family = StateFamilyId::parse_canonical(family_text).map_err(|error| {
            ActionStateError::HydrationPath {
                namespace: record.namespace.clone(),
                persistence_key: record.key.clone(),
                message: bounded_callback_message(error.to_string()),
            }
        })?;
        let locator = (owner.clone(), family.clone());
        let registered =
            self.registry
                .entries
                .get(&locator)
                .ok_or_else(|| ActionStateError::UnknownFamily {
                    owner: owner.clone(),
                    family: family.clone(),
                })?;
        validate_encoded_state_key(
            owner,
            family,
            encoded,
            registered.descriptor().max_key_bytes(),
        )
    }

    /// Read one live value through its exact composition-bound capability.
    pub fn get<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&V>, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let state_key = self.state_key(handle, key)?;
        self.records
            .get(&state_key)
            .map(|record| downcast_value::<V>(record.value.as_ref(), &state_key))
            .transpose()
    }

    /// Scan one exact typed family in canonical encoded-key order.
    ///
    /// The cursor is exclusive and opaque to callers: use only a
    /// [`StateFamilyPage::next_after`] value returned by an earlier call.
    /// Paging bounds both allocation and synchronous module work even when
    /// the complete hydrated store contains many records.
    pub fn scan_family<'a, K, V>(
        &'a self,
        handle: &StateFamilyHandle<K, V>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<StateFamilyPage<'a, V>, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.registry.get_for_handle(handle)?;
        if limit == 0 || limit > MAX_STATE_FAMILY_SCAN_RECORDS {
            return Err(ActionStateError::InvalidScanLimit {
                actual: limit,
                max: MAX_STATE_FAMILY_SCAN_RECORDS,
            });
        }
        if after.is_some_and(|cursor| cursor.len() > handle.descriptor().max_key_bytes()) {
            return Err(ActionStateError::ScanCursorTooLong {
                actual: after.map_or(0, str::len),
                max: handle.descriptor().max_key_bytes(),
            });
        }
        let start = StateKey {
            owner: handle.owner().clone(),
            family: handle.descriptor().id.clone(),
            encoded: SmolStr::new(after.unwrap_or("")),
        };
        let lower = if after.is_some() {
            Bound::Excluded(start)
        } else {
            Bound::Included(start)
        };
        let mut entries = Vec::with_capacity(limit);
        let mut next_after = None;
        for (key, stored) in self.records.range((lower, Bound::Unbounded)) {
            if key.owner() != handle.owner() || key.family() != &handle.descriptor().id {
                break;
            }
            if entries.len() == limit {
                next_after = entries
                    .last()
                    .map(|entry: &StateFamilyScanEntry<'_, V>| SmolStr::new(entry.encoded_key));
                break;
            }
            entries.push(StateFamilyScanEntry {
                encoded_key: key.encoded(),
                value: downcast_value::<V>(stored.value.as_ref(), key)?,
            });
        }
        Ok(StateFamilyPage {
            entries,
            next_after,
        })
    }

    /// Exact resolution-secret versions pinned by unresolved durable action
    /// windows. The server uses this bounded set to refuse a key rotation
    /// that would prune material still needed after restart.
    #[must_use]
    pub fn unresolved_window_resolution_secret_versions(&self) -> BTreeSet<u32> {
        self.records
            .values()
            .filter_map(|stored| {
                stored
                    .value
                    .as_ref()
                    .as_any()
                    .downcast_ref::<crate::action_window::ActionWindowRecord>()
            })
            .filter(|record| {
                matches!(
                    record.phase(),
                    crate::action_window::ActionWindowPhase::Open
                        | crate::action_window::ActionWindowPhase::Closed
                )
            })
            .filter_map(|record| record.window().resolution_secret_version())
            .collect()
    }

    /// Return the exact canonical live post-image for diagnostics/tests.
    pub fn canonical<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&CanonicalStateBytes>, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let state_key = self.state_key(handle, key)?;
        Ok(self.records.get(&state_key).map(|record| &record.canonical))
    }

    /// Begin a transaction bound to the owner proven by `handle`.
    pub fn transaction<'a, K, V>(
        &'a mut self,
        handle: &StateFamilyHandle<K, V>,
    ) -> Result<ActionTxn<'a>, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.registry.get_for_handle(handle)?;
        self.begin_for_owner(handle.owner().clone(), None)
    }

    pub(crate) fn transaction_for_provider(
        &mut self,
        provider: &ProviderRegistrationOrdinal,
    ) -> Result<ActionTxn<'_>, ActionStateError> {
        if !self
            .registry
            .shares_readiness_with(provider.owner_readiness())
        {
            return Err(ActionStateError::RegistryCompositionMismatch);
        }
        self.begin_for_owner(provider.owner().clone(), Some(provider.get()))
    }

    fn begin_for_owner(
        &mut self,
        owner: OwnerId,
        provider_ordinal: Option<u32>,
    ) -> Result<ActionTxn<'_>, ActionStateError> {
        self.registry
            .owner_readiness
            .require_ready(&owner)
            .map_err(ActionStateError::OwnerNotReady)?;
        if let Some(reservation) = self.pending_reservation {
            return Err(ActionStateError::CommitPending { reservation });
        }
        let limits = ActionTxnLimits::default();
        Ok(ActionTxn {
            store: self,
            owner,
            provider_ordinal,
            mutations: BTreeMap::new(),
            operation_count: 0,
            canonical_bytes: 0,
            limits,
            poison: None,
        })
    }

    fn state_key<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<StateKey, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.registry.get_for_handle(handle)?;
        encode_state_key(handle, key)
    }

    /// Install already-validated images after the exact persistence receipt.
    ///
    /// This operation is logically infallible: every identity, bound, type,
    /// and pre-state check happened before [`PreparedInstallation`] was
    /// created. A mismatch can therefore only be an engine wiring bug and is
    /// asserted rather than converted into a post-ack gameplay failure.
    pub fn install(&mut self, installation: PreparedInstallation) {
        self.assert_installation(&installation);
        for (key, mutation) in installation.mutations {
            match mutation {
                InstallMutation::Put { value, canonical } => {
                    self.records
                        .insert(key, StoredStateRecord { value, canonical });
                }
                InstallMutation::Delete => {
                    let removed = self.records.remove(&key);
                    debug_assert!(removed.is_some(), "prepared delete retained its pre-state");
                }
            }
        }
        self.revision = installation.new_revision;
        self.pending_reservation = None;
    }

    /// Drop prepared post-images after a definitive persistence refusal.
    pub fn abort(&mut self, installation: PreparedInstallation) {
        self.assert_installation(&installation);
        self.pending_reservation = None;
    }

    fn assert_installation(&self, installation: &PreparedInstallation) {
        assert!(
            Arc::ptr_eq(&self.identity, &installation.store_identity),
            "prepared installation belongs to a different action state store"
        );
        assert_eq!(
            self.pending_reservation,
            Some(installation.reservation),
            "prepared installation does not own the store reservation"
        );
        assert_eq!(
            self.revision, installation.base_revision,
            "prepared installation pre-state changed while commit was pending"
        );
    }
}

/// Caller-tightenable transaction limits. Hard ceilings can never be raised.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionTxnLimits {
    pub operations: usize,
    pub canonical_bytes: usize,
}

impl Default for ActionTxnLimits {
    fn default() -> Self {
        Self {
            operations: MAX_ACTION_MUTATION_OPERATIONS,
            canonical_bytes: MAX_ACTION_BUNDLE_BYTES,
        }
    }
}

impl ActionTxnLimits {
    #[must_use]
    pub const fn tightened(operations: usize, canonical_bytes: usize) -> Self {
        Self {
            operations: if operations < MAX_ACTION_MUTATION_OPERATIONS {
                operations
            } else {
                MAX_ACTION_MUTATION_OPERATIONS
            },
            canonical_bytes: if canonical_bytes < MAX_ACTION_BUNDLE_BYTES {
                canonical_bytes
            } else {
                MAX_ACTION_BUNDLE_BYTES
            },
        }
    }
}

enum TxnMutation {
    Put {
        value: Box<dyn ErasedStateValue>,
        canonical: CanonicalStateBytes,
        document: Value,
    },
    Delete,
}

/// Scratch state for one owner. It borrows the live store mutably, making
/// concurrent preparation impossible even before a prepared reservation is
/// installed.
pub struct ActionTxn<'a> {
    store: &'a mut ActionStateStore,
    owner: OwnerId,
    provider_ordinal: Option<u32>,
    mutations: BTreeMap<StateKey, TxnMutation>,
    operation_count: usize,
    canonical_bytes: usize,
    limits: ActionTxnLimits,
    poison: Option<String>,
}

impl fmt::Debug for ActionTxn<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionTxn")
            .field("owner", &self.owner)
            .field("provider_ordinal", &self.provider_ordinal)
            .field("mutations", &self.mutations.len())
            .field("operation_count", &self.operation_count)
            .field("canonical_bytes", &self.canonical_bytes)
            .field("limits", &self.limits)
            .field("poisoned", &self.poison.is_some())
            .finish()
    }
}

impl ActionTxn<'_> {
    /// Tighten per-action limits before adding mutations.
    pub fn set_limits(&mut self, limits: ActionTxnLimits) -> Result<(), ActionStateError> {
        let limits = ActionTxnLimits::tightened(limits.operations, limits.canonical_bytes);
        if self.operation_count > limits.operations || self.canonical_bytes > limits.canonical_bytes
        {
            let error = ActionStateError::LimitsAlreadyExceeded;
            self.poison(&error);
            return Err(error);
        }
        self.limits = limits;
        Ok(())
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn operation_count(&self) -> usize {
        self.operation_count
    }

    #[must_use]
    pub const fn canonical_bytes(&self) -> usize {
        self.canonical_bytes
    }

    /// Read the transaction's post-image when present, otherwise live state.
    pub fn get<K, V>(
        &mut self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&V>, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let state_key = match self.checked_key(handle, key) {
            Ok(key) => key,
            Err(error) => {
                self.poison(&error);
                return Err(error);
            }
        };
        match self.mutations.get(&state_key) {
            Some(TxnMutation::Put { value, .. }) => {
                downcast_value::<V>(value.as_ref(), &state_key).map(Some)
            }
            Some(TxnMutation::Delete) => Ok(None),
            None => self
                .store
                .records
                .get(&state_key)
                .map(|record| downcast_value::<V>(record.value.as_ref(), &state_key))
                .transpose(),
        }
    }

    /// Insert a record that must not exist in the live pre-state.
    pub fn insert<K, V>(
        &mut self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
        value: V,
    ) -> Result<(), ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let result = (|| {
            let state_key = self.checked_key(handle, key)?;
            self.require_fresh_mutation(&state_key)?;
            if self.store.records.contains_key(&state_key) {
                return Err(ActionStateError::AlreadyExists { key: state_key });
            }
            self.put_checked(handle, state_key, value)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    /// Replace a record that must exist in the live pre-state.
    pub fn replace<K, V>(
        &mut self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
        value: V,
    ) -> Result<(), ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let result = (|| {
            let state_key = self.checked_key(handle, key)?;
            self.require_fresh_mutation(&state_key)?;
            if !self.store.records.contains_key(&state_key) {
                return Err(ActionStateError::Missing { key: state_key });
            }
            self.put_checked(handle, state_key, value)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    /// Remove a record that must exist in the live pre-state.
    pub fn remove<K, V>(
        &mut self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<(), ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let result = (|| {
            let state_key = self.checked_key(handle, key)?;
            self.require_fresh_mutation(&state_key)?;
            if !self.store.records.contains_key(&state_key) {
                return Err(ActionStateError::Missing { key: state_key });
            }
            let charge = state_key.encoded.len();
            self.preflight_mutation(charge)?;
            self.mutations.insert(state_key, TxnMutation::Delete);
            self.operation_count += 1;
            self.canonical_bytes += charge;
            Ok(())
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    fn checked_key<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<StateKey, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.store.registry.get_for_handle(handle)?;
        if handle.owner() != &self.owner {
            return Err(ActionStateError::OwnerMismatch {
                transaction: self.owner.clone(),
                family: handle.owner().clone(),
            });
        }
        encode_state_key(handle, key)
    }

    fn require_fresh_mutation(&self, key: &StateKey) -> Result<(), ActionStateError> {
        if self.mutations.contains_key(key) {
            return Err(ActionStateError::DuplicateMutation { key: key.clone() });
        }
        Ok(())
    }

    fn put_checked<K, V>(
        &mut self,
        handle: &StateFamilyHandle<K, V>,
        state_key: StateKey,
        value: V,
    ) -> Result<(), ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let family = handle.typed();
        family.validator.validate_value(&value).map_err(|message| {
            ActionStateError::ValueRejected {
                key: state_key.clone(),
                message: bounded_callback_message(message),
            }
        })?;
        let projected = family
            .serializer
            .serialize_value(&value)
            .map_err(|message| ActionStateError::SerializationRejected {
                key: state_key.clone(),
                message: bounded_callback_message(message),
            })?;
        let document = json!({
            "family": {
                "owner": family.owner,
                "id": family.descriptor.id,
            },
            "key": state_key.encoded,
            "schema": {
                "id": family.descriptor.value_schema,
                "version": family.descriptor.schema_version,
            },
            "value": projected,
        });
        let canonical =
            canonical_json_bytes_with_limit(&document, family.descriptor.max_record_bytes)
                .map_err(|source| ActionStateError::CanonicalPostImage {
                    key: state_key.clone(),
                    source,
                })?;
        self.preflight_mutation(canonical.len())?;
        self.operation_count += 1;
        self.canonical_bytes += canonical.len();
        self.mutations.insert(
            state_key,
            TxnMutation::Put {
                value: Box::new(value),
                canonical: CanonicalStateBytes(Arc::from(canonical)),
                document,
            },
        );
        Ok(())
    }

    fn preflight_mutation(&self, bytes: usize) -> Result<(), ActionStateError> {
        let attempted_operations = self
            .operation_count
            .checked_add(1)
            .ok_or(ActionStateError::ArithmeticOverflow)?;
        if attempted_operations > self.limits.operations {
            return Err(ActionStateError::OperationLimit {
                attempted: attempted_operations,
                max: self.limits.operations,
            });
        }
        let attempted_bytes = self
            .canonical_bytes
            .checked_add(bytes)
            .ok_or(ActionStateError::ArithmeticOverflow)?;
        if attempted_bytes > self.limits.canonical_bytes {
            return Err(ActionStateError::BundleBytesExceeded {
                attempted: attempted_bytes,
                max: self.limits.canonical_bytes,
            });
        }
        Ok(())
    }

    fn poison(&mut self, error: &ActionStateError) {
        if self.poison.is_none() {
            self.poison = Some(bounded_callback_message(error.to_string()));
        }
    }

    /// Consume validated scratch state and reserve the store for one exact
    /// persistence result. No serializer or provider callback can run after
    /// this returns.
    pub fn prepare(self) -> Result<PreparedTxn, ActionStateError> {
        if let Some(cause) = self.poison {
            return Err(ActionStateError::TransactionPoisoned { cause });
        }
        if self.mutations.is_empty() {
            return Err(ActionStateError::EmptyTransaction);
        }
        self.store
            .registry
            .owner_readiness
            .require_ready(&self.owner)
            .map_err(ActionStateError::OwnerNotReady)?;
        let reservation = self
            .store
            .next_reservation
            .checked_add(1)
            .ok_or(ActionStateError::ReservationExhausted)?;
        let new_revision = self
            .store
            .revision
            .checked_add(1)
            .ok_or(ActionStateError::RevisionExhausted)?;
        let base_revision = self.store.revision;
        let store_identity = Arc::clone(&self.store.identity);
        let owner = self.owner;
        let provider_ordinal = self.provider_ordinal;
        let operation_count = self.operation_count;
        let canonical_bytes = self.canonical_bytes;
        let mut records = Vec::with_capacity(self.mutations.len());
        for (key, mutation) in self.mutations {
            let persistence_key = key.persistence_key();
            debug_assert!(persistence_key.len() <= MAX_MODULE_KEY_BYTES);
            let (canonical_post_image, module_op, installation) = match mutation {
                TxnMutation::Put {
                    value,
                    canonical,
                    document,
                } => (
                    Some(canonical.clone()),
                    ModuleOp {
                        ns: SmolStr::new(key.owner.as_str()),
                        op: ModuleWrite::JsonSet {
                            key: persistence_key,
                            value: document,
                        },
                    },
                    InstallMutation::Put { value, canonical },
                ),
                TxnMutation::Delete => (
                    None,
                    ModuleOp {
                        ns: SmolStr::new(key.owner.as_str()),
                        op: ModuleWrite::Del {
                            key: persistence_key,
                        },
                    },
                    InstallMutation::Delete,
                ),
            };
            records.push(PreparedRecord {
                key,
                canonical_post_image,
                module_op,
                installation,
            });
        }
        self.store.next_reservation = reservation;
        self.store.pending_reservation = Some(reservation);
        Ok(PreparedTxn {
            owner,
            provider_ordinal,
            operation_count,
            canonical_bytes,
            reservation,
            base_revision,
            new_revision,
            store_identity,
            records,
        })
    }
}

fn encode_state_key<K, V>(
    handle: &StateFamilyHandle<K, V>,
    key: &K,
) -> Result<StateKey, ActionStateError>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let family = handle.typed();
    let encoded =
        family
            .key_codec
            .encode_key(key)
            .map_err(|message| ActionStateError::KeyRejected {
                owner: family.owner.clone(),
                family: family.descriptor.id.clone(),
                message: bounded_callback_message(message),
            })?;
    validate_encoded_state_key(
        family.owner.clone(),
        family.descriptor.id.clone(),
        &encoded,
        family.descriptor.max_key_bytes,
    )
}

fn validate_encoded_state_key(
    owner: OwnerId,
    family: StateFamilyId,
    encoded: &str,
    max_key_bytes: usize,
) -> Result<StateKey, ActionStateError> {
    if encoded.is_empty() {
        return Err(ActionStateError::EmptyKey { owner, family });
    }
    if encoded.len() > max_key_bytes {
        return Err(ActionStateError::KeyTooLong {
            owner,
            family,
            actual: encoded.len(),
            max: max_key_bytes,
        });
    }
    if let Some((offset, byte)) = encoded.bytes().enumerate().find(|(_, byte)| {
        !matches!(
            byte,
            b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b':' | b'/' | b'-' | b'~'
        )
    }) {
        return Err(ActionStateError::InvalidKeyByte {
            owner,
            family,
            offset,
            byte,
        });
    }
    let state_key = StateKey {
        owner,
        family,
        encoded: SmolStr::new(encoded),
    };
    if state_key.persistence_key().len() > MAX_MODULE_KEY_BYTES {
        return Err(ActionStateError::PersistenceKeyTooLong {
            key: state_key,
            max: MAX_MODULE_KEY_BYTES,
        });
    }
    Ok(state_key)
}

fn require_hydration_identity(
    key: &StateKey,
    field: &'static str,
    expected: &str,
    actual: &str,
) -> Result<(), ActionStateError> {
    if expected == actual {
        Ok(())
    } else {
        Err(ActionStateError::HydrationIdentityMismatch {
            key: key.clone(),
            field,
            expected: expected.to_owned().into_boxed_str(),
            actual: actual.to_owned().into_boxed_str(),
        })
    }
}

fn downcast_value<'a, V: 'static>(
    value: &'a dyn ErasedStateValue,
    key: &StateKey,
) -> Result<&'a V, ActionStateError> {
    value
        .as_any()
        .downcast_ref::<V>()
        .ok_or_else(|| ActionStateError::StoredTypeMismatch {
            key: key.clone(),
            requested: type_name::<V>(),
        })
}

fn bounded_callback_message(mut message: String) -> String {
    if message.len() <= MAX_STATE_CALLBACK_ERROR_BYTES {
        return message;
    }
    let mut boundary = MAX_STATE_CALLBACK_ERROR_BYTES;
    while !message.is_char_boundary(boundary) {
        boundary -= 1;
    }
    message.truncate(boundary);
    message
}

/// One fully validated persistence/install pair inside a prepared transaction.
pub struct PreparedRecord {
    key: StateKey,
    canonical_post_image: Option<CanonicalStateBytes>,
    module_op: ModuleOp,
    installation: InstallMutation,
}

impl fmt::Debug for PreparedRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedRecord")
            .field("key", &self.key)
            .field("canonical_post_image", &self.canonical_post_image)
            .field("module_op", &self.module_op)
            .finish_non_exhaustive()
    }
}

impl PreparedRecord {
    #[must_use]
    pub const fn key(&self) -> &StateKey {
        &self.key
    }

    #[must_use]
    pub const fn canonical_post_image(&self) -> Option<&CanonicalStateBytes> {
        self.canonical_post_image.as_ref()
    }

    #[must_use]
    pub const fn module_op(&self) -> &ModuleOp {
        &self.module_op
    }
}

/// One-shot transaction materialization. It intentionally does not implement
/// `Clone` or serialization.
#[must_use = "a prepared transaction must be staged for persistence or explicitly aborted"]
pub struct PreparedTxn {
    owner: OwnerId,
    provider_ordinal: Option<u32>,
    operation_count: usize,
    canonical_bytes: usize,
    reservation: u64,
    base_revision: u64,
    new_revision: u64,
    store_identity: Arc<()>,
    records: Vec<PreparedRecord>,
}

impl fmt::Debug for PreparedTxn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedTxn")
            .field("owner", &self.owner)
            .field("provider_ordinal", &self.provider_ordinal)
            .field("operation_count", &self.operation_count)
            .field("canonical_bytes", &self.canonical_bytes)
            .field("reservation", &self.reservation)
            .field("base_revision", &self.base_revision)
            .field("new_revision", &self.new_revision)
            .field("records", &self.records)
            .finish_non_exhaustive()
    }
}

impl PreparedTxn {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn provider_ordinal(&self) -> Option<u32> {
        self.provider_ordinal
    }

    #[must_use]
    pub const fn operation_count(&self) -> usize {
        self.operation_count
    }

    #[must_use]
    pub const fn canonical_bytes(&self) -> usize {
        self.canonical_bytes
    }

    #[must_use]
    pub fn records(&self) -> &[PreparedRecord] {
        &self.records
    }

    /// Deterministic K3 replay post-images corresponding to this typed
    /// transaction. A complete authoritative bundle may be built around this
    /// write set before the transaction is consumed for staging.
    #[must_use]
    pub fn authoritative_replay_writes(&self) -> Vec<ReplayWrite> {
        self.records
            .iter()
            .map(|record| match &record.module_op.op {
                ModuleWrite::JsonSet { key, value } => ReplayWrite::ModuleJsonSet {
                    namespace: record.key.owner.clone(),
                    key: AuthoritativeKey::parse_canonical(key).expect(
                        "prepared action-state keys already satisfy authoritative-key constraints",
                    ),
                    value: value.clone(),
                },
                ModuleWrite::Del { key } => ReplayWrite::ModuleDelete {
                    namespace: record.key.owner.clone(),
                    key: AuthoritativeKey::parse_canonical(key).expect(
                        "prepared action-state keys already satisfy authoritative-key constraints",
                    ),
                },
                ModuleWrite::StreamAppend { .. } => {
                    unreachable!("typed action-state transactions never produce stream appends")
                }
            })
            .collect()
    }

    /// Split store writes from the non-cloneable install images. Both were
    /// derived from the same validated typed values before this call.
    pub fn into_parts(self) -> (Vec<ModuleOp>, PreparedInstallation) {
        let mut module_ops = Vec::with_capacity(self.records.len());
        let mut mutations = BTreeMap::new();
        for record in self.records {
            module_ops.push(record.module_op);
            let previous = mutations.insert(record.key, record.installation);
            debug_assert!(previous.is_none());
        }
        (
            module_ops,
            PreparedInstallation {
                reservation: self.reservation,
                base_revision: self.base_revision,
                new_revision: self.new_revision,
                store_identity: self.store_identity,
                mutations,
            },
        )
    }
}

enum InstallMutation {
    Put {
        value: Box<dyn ErasedStateValue>,
        canonical: CanonicalStateBytes,
    },
    Delete,
}

/// Private typed RAM post-images held across the exact persistence receipt.
/// It is deliberately non-cloneable and has no public field access.
#[must_use = "a prepared installation must be installed after exact acknowledgement or aborted"]
pub struct PreparedInstallation {
    reservation: u64,
    base_revision: u64,
    new_revision: u64,
    store_identity: Arc<()>,
    mutations: BTreeMap<StateKey, InstallMutation>,
}

impl fmt::Debug for PreparedInstallation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedInstallation")
            .field("reservation", &self.reservation)
            .field("base_revision", &self.base_revision)
            .field("new_revision", &self.new_revision)
            .field("mutations", &self.mutations.len())
            .finish_non_exhaustive()
    }
}

/// Checked action-state refusal. Every variant occurs before live mutation.
#[derive(Debug, Error)]
pub enum ActionStateError {
    #[error("state family {owner}/{family} is not registered")]
    UnknownFamily {
        owner: OwnerId,
        family: StateFamilyId,
    },
    #[error("state-family handle {owner}/{family} belongs to another composition")]
    ForeignHandle {
        owner: OwnerId,
        family: StateFamilyId,
    },
    #[error("action state and provider registries belong to different compositions")]
    RegistryCompositionMismatch,
    #[error("state-family owner is unavailable: {0}")]
    OwnerNotReady(#[source] MechanicalOwnerReadinessError),
    #[error("transaction owner {transaction} cannot access state owned by {family}")]
    OwnerMismatch {
        transaction: OwnerId,
        family: OwnerId,
    },
    #[error("state key codec rejected {owner}/{family}: {message}")]
    KeyRejected {
        owner: OwnerId,
        family: StateFamilyId,
        message: String,
    },
    #[error("state key for {owner}/{family} is empty")]
    EmptyKey {
        owner: OwnerId,
        family: StateFamilyId,
    },
    #[error("state key for {owner}/{family} is {actual} bytes; maximum is {max}")]
    KeyTooLong {
        owner: OwnerId,
        family: StateFamilyId,
        actual: usize,
        max: usize,
    },
    #[error("state key for {owner}/{family} contains invalid byte {byte:#04x} at {offset}")]
    InvalidKeyByte {
        owner: OwnerId,
        family: StateFamilyId,
        offset: usize,
        byte: u8,
    },
    #[error("materialized persistence key for {key:?} exceeds {max} bytes")]
    PersistenceKeyTooLong { key: StateKey, max: usize },
    #[error("state value for {key:?} was rejected: {message}")]
    ValueRejected { key: StateKey, message: String },
    #[error("persisted state decoder for {key:?} rejected the value: {message}")]
    DecodeRejected { key: StateKey, message: String },
    #[error("state serializer for {key:?} refused the value: {message}")]
    SerializationRejected { key: StateKey, message: String },
    #[error("canonical post-image for {key:?} is invalid: {source}")]
    CanonicalPostImage {
        key: StateKey,
        #[source]
        source: CanonicalEncodingError,
    },
    #[error("state record already exists: {key:?}")]
    AlreadyExists { key: StateKey },
    #[error("state record does not exist: {key:?}")]
    Missing { key: StateKey },
    #[error("state record was mutated more than once in one transaction: {key:?}")]
    DuplicateMutation { key: StateKey },
    #[error("stored value for {key:?} does not match requested type {requested}")]
    StoredTypeMismatch {
        key: StateKey,
        requested: &'static str,
    },
    #[error("action state store already holds prepared reservation {reservation}")]
    CommitPending { reservation: u64 },
    #[error("transaction operation count {attempted} exceeds maximum {max}")]
    OperationLimit { attempted: usize, max: usize },
    #[error("transaction canonical bytes {attempted} exceed maximum {max}")]
    BundleBytesExceeded { attempted: usize, max: usize },
    #[error("new limits are already below accumulated transaction work")]
    LimitsAlreadyExceeded,
    #[error("transaction was poisoned by an earlier ignored refusal: {cause}")]
    TransactionPoisoned { cause: String },
    #[error("transaction count/byte arithmetic overflowed")]
    ArithmeticOverflow,
    #[error("an empty logical action cannot reserve persistence")]
    EmptyTransaction,
    #[error("action-state reservation sequence is exhausted")]
    ReservationExhausted,
    #[error("action-state revision sequence is exhausted")]
    RevisionExhausted,
    #[error("state-family scan limit {actual} must be in 1..={max}")]
    InvalidScanLimit { actual: usize, max: usize },
    #[error("state-family scan cursor is {actual} bytes; maximum is {max}")]
    ScanCursorTooLong { actual: usize, max: usize },
    #[error("action state can only hydrate into a fresh, unreserved store")]
    HydrationStoreNotEmpty,
    #[error("action-state hydration exceeds the maximum of {max} records")]
    HydrationRecordLimit { max: usize },
    #[error("action-state hydration bytes {attempted} exceed maximum {max}")]
    HydrationBytesExceeded { attempted: usize, max: usize },
    #[error(
        "invalid action-state envelope in namespace {namespace}, key {persistence_key}: {message}"
    )]
    HydrationEnvelope {
        namespace: SmolStr,
        persistence_key: SmolStr,
        message: String,
    },
    #[error("invalid action-state path in namespace {namespace}, key {persistence_key}: {message}")]
    HydrationPath {
        namespace: SmolStr,
        persistence_key: SmolStr,
        message: String,
    },
    #[error(
        "persisted state identity {field} for {key:?} differs: expected {expected}, got {actual}"
    )]
    HydrationIdentityMismatch {
        key: StateKey,
        field: &'static str,
        expected: Box<str>,
        actual: Box<str>,
    },
    #[error("persisted state for {key:?} is not the canonical registered representation")]
    HydrationCanonicalMismatch { key: StateKey },
    #[error("persisted action-state record is duplicated: {key:?}")]
    DuplicateHydratedRecord { key: StateKey },
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{Engine, GameModule, GuardSet, ModError, Registrar, Registry, RevisionSet, World};

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct CounterState {
        amount: u16,
    }

    type CounterHandle = StateFamilyHandle<u64, CounterState>;

    struct CounterModule {
        owner: &'static str,
        captured: Arc<Mutex<Option<CounterHandle>>>,
        serializer_calls: Arc<AtomicUsize>,
    }

    impl GameModule for CounterModule {
        fn name(&self) -> &'static str {
            self.owner
        }

        fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), ModError> {
            let descriptor = StateFamilyDescriptor::new(
                StateFamilyId::new("counter").expect("static family id"),
                CanonicalId::new("u64.decimal.v1").expect("static codec id"),
                CanonicalId::new("counter.v1").expect("static schema id"),
                1,
                CanonicalId::new("counter.decode_json.v1").expect("static decoder id"),
                CanonicalId::new("counter.range.v1").expect("static validator id"),
                CanonicalId::new("counter.json.v1").expect("static serializer id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?;
            let calls = Arc::clone(&self.serializer_calls);
            let handle = registrar.state_family(
                descriptor,
                |key: &u64| Ok(key.to_string()),
                |value: &Value| {
                    let amount = value
                        .get("amount")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| "amount must be an unsigned integer".to_owned())?;
                    Ok(CounterState {
                        amount: u16::try_from(amount)
                            .map_err(|_| "amount exceeds u16".to_owned())?,
                    })
                },
                |value: &CounterState| {
                    if value.amount <= 3 {
                        Ok(())
                    } else {
                        Err("counter must remain within 0..=3".to_owned())
                    }
                },
                move |value: &CounterState| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(json!({ "amount": value.amount }))
                },
            )?;
            *self.captured.lock().expect("capture mutex poisoned") = Some(handle);
            Ok(())
        }
    }

    fn fixture(owner: &'static str) -> (Engine, CounterHandle, Arc<AtomicUsize>) {
        let captured = Arc::new(Mutex::new(None));
        let serializer_calls = Arc::new(AtomicUsize::new(0));
        let module = CounterModule {
            owner,
            captured: Arc::clone(&captured),
            serializer_calls: Arc::clone(&serializer_calls),
        };
        let engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(module)],
        )
        .expect("state-family module composes");
        let handle = captured
            .lock()
            .expect("capture mutex poisoned")
            .clone()
            .expect("registration captured handle");
        (engine, handle, serializer_calls)
    }

    fn install_direct(engine: &mut Engine, prepared: PreparedTxn) -> Vec<ModuleOp> {
        let (ops, installation) = prepared.into_parts();
        engine.install_prepared_action(installation);
        ops
    }

    fn empty_guards(engine: &Engine) -> GuardSet {
        engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .unwrap()
            .finish()
            .unwrap()
    }

    fn boot_records(ops: Vec<ModuleOp>) -> Vec<ActionStateBootRecord> {
        ops.into_iter()
            .map(|op| match op.op {
                ModuleWrite::JsonSet { key, value } => {
                    ActionStateBootRecord::new(op.ns, key, value)
                }
                other => panic!("expected state JsonSet, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn persisted_state_round_trips_into_a_fresh_composition() {
        let (mut first, first_handle, _) = fixture("state_test");
        let mut txn = first.action_transaction(&first_handle).unwrap();
        txn.insert(&first_handle, &7, CounterState { amount: 2 })
            .unwrap();
        txn.insert(&first_handle, &3, CounterState { amount: 1 })
            .unwrap();
        let prepared = txn.prepare().unwrap();
        let records = boot_records(install_direct(&mut first, prepared));

        let (mut restarted, restarted_handle, calls) = fixture("state_test");
        restarted.hydrate_action_state(records).unwrap();

        assert_eq!(restarted.action_state().len(), 2);
        assert_eq!(
            restarted.action_state().get(&restarted_handle, &3).unwrap(),
            Some(&CounterState { amount: 1 })
        );
        assert_eq!(
            restarted.action_state().get(&restarted_handle, &7).unwrap(),
            Some(&CounterState { amount: 2 })
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(restarted.action_state().revision(), 0);
        assert!(!restarted.action_state().has_prepared());
    }

    #[test]
    fn typed_family_scan_is_canonical_bounded_and_exclusively_pageable() {
        let (mut engine, handle, _) = fixture("state_test");
        let mut txn = engine.action_transaction(&handle).unwrap();
        for key in [3, 1, 2] {
            txn.insert(&handle, &key, CounterState { amount: key as u16 })
                .unwrap();
        }
        let prepared = txn.prepare().unwrap();
        install_direct(&mut engine, prepared);

        let first = engine.action_state().scan_family(&handle, None, 2).unwrap();
        assert_eq!(
            first
                .entries()
                .iter()
                .map(StateFamilyScanEntry::encoded_key)
                .collect::<Vec<_>>(),
            vec!["1", "2"]
        );
        assert_eq!(
            first
                .entries()
                .iter()
                .map(|entry| entry.value().amount)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        let cursor = first.next_after().cloned().expect("another page");
        let second = engine
            .action_state()
            .scan_family(&handle, Some(cursor.as_str()), 2)
            .unwrap();
        assert_eq!(
            second
                .entries()
                .iter()
                .map(StateFamilyScanEntry::encoded_key)
                .collect::<Vec<_>>(),
            vec!["3"]
        );
        assert!(second.next_after().is_none());
        assert!(matches!(
            engine.action_state().scan_family(&handle, None, 0),
            Err(ActionStateError::InvalidScanLimit { .. })
        ));
    }

    #[test]
    fn one_bad_record_leaves_the_entire_hydration_uninstalled() {
        let (mut first, handle, _) = fixture("state_test");
        let mut txn = first.action_transaction(&handle).unwrap();
        txn.insert(&handle, &1, CounterState { amount: 1 }).unwrap();
        txn.insert(&handle, &2, CounterState { amount: 2 }).unwrap();
        let prepared = txn.prepare().unwrap();
        let mut records = boot_records(install_direct(&mut first, prepared));
        records[1].value["schema"]["version"] = json!(9);

        let (mut restarted, restarted_handle, _) = fixture("state_test");
        assert!(matches!(
            restarted.hydrate_action_state(records),
            Err(ActionStateError::HydrationIdentityMismatch {
                field: "schema.version",
                ..
            })
        ));
        assert!(restarted.action_state().is_empty());
        assert!(
            restarted
                .action_state()
                .get(&restarted_handle, &1)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn hydration_rejects_noncanonical_decoder_normalization() {
        let (mut first, handle, _) = fixture("state_test");
        let mut txn = first.action_transaction(&handle).unwrap();
        txn.insert(&handle, &1, CounterState { amount: 1 }).unwrap();
        let prepared = txn.prepare().unwrap();
        let mut records = boot_records(install_direct(&mut first, prepared));
        records[0].value["value"]["unregistered"] = json!(true);

        let (mut restarted, _, _) = fixture("state_test");
        assert!(matches!(
            restarted.hydrate_action_state(records),
            Err(ActionStateError::HydrationCanonicalMismatch { .. })
        ));
        assert!(restarted.action_state().is_empty());
    }

    #[test]
    fn hydration_rejects_an_unregistered_family_without_partial_state() {
        let (mut first, handle, _) = fixture("state_test");
        let mut txn = first.action_transaction(&handle).unwrap();
        txn.insert(&handle, &1, CounterState { amount: 1 }).unwrap();
        let prepared = txn.prepare().unwrap();
        let mut records = boot_records(install_direct(&mut first, prepared));
        records[0].key = "action-state/v1/missing/1".into();

        let (mut restarted, _, _) = fixture("state_test");
        assert!(matches!(
            restarted.hydrate_action_state(records),
            Err(ActionStateError::UnknownFamily { .. })
        ));
        assert!(restarted.action_state().is_empty());
    }

    #[test]
    fn prepare_is_non_mutating_and_abort_restores_capacity() {
        let (mut engine, handle, _) = fixture("state_test");
        let mut txn = engine.action_transaction(&handle).unwrap();
        txn.insert(&handle, &7, CounterState { amount: 3 }).unwrap();
        let prepared = txn.prepare().unwrap();

        assert!(engine.action_state().get(&handle, &7).unwrap().is_none());
        assert!(engine.action_state().has_prepared());
        assert_eq!(prepared.operation_count(), 1);
        assert_eq!(prepared.records().len(), 1);
        assert!(matches!(
            engine.action_transaction(&handle),
            Err(ActionStateError::CommitPending { .. })
        ));

        let (_, installation) = prepared.into_parts();
        engine.abort_prepared_action(installation);
        assert!(!engine.action_state().has_prepared());
        assert!(engine.action_state().get(&handle, &7).unwrap().is_none());
    }

    #[test]
    fn install_uses_the_already_serialized_typed_post_image_once() {
        let (mut engine, handle, calls) = fixture("state_test");
        let mut txn = engine.action_transaction(&handle).unwrap();
        txn.insert(&handle, &2, CounterState { amount: 3 }).unwrap();
        let prepared = txn.prepare().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let canonical = prepared.records()[0]
            .canonical_post_image()
            .unwrap()
            .as_slice()
            .to_vec();
        let ops = install_direct(&mut engine, prepared);

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            engine.action_state().get(&handle, &2).unwrap(),
            Some(&CounterState { amount: 3 })
        );
        assert_eq!(
            engine
                .action_state()
                .canonical(&handle, &2)
                .unwrap()
                .unwrap()
                .as_slice(),
            canonical
        );
        assert!(matches!(
            &ops[0].op,
            ModuleWrite::JsonSet { key, value }
                if key == "action-state/v1/counter/2"
                    && value["value"]["amount"] == 3
        ));
    }

    #[test]
    fn ignored_validation_error_poisons_finalization_without_live_mutation() {
        let (mut engine, handle, _) = fixture("state_test");
        let mut txn = engine.action_transaction(&handle).unwrap();
        let error = txn
            .insert(&handle, &1, CounterState { amount: 4 })
            .unwrap_err();
        assert!(matches!(error, ActionStateError::ValueRejected { .. }));
        txn.insert(&handle, &2, CounterState { amount: 1 })
            .expect("later scratch mutation may be diagnosed but cannot unpoison");
        assert!(matches!(
            txn.prepare(),
            Err(ActionStateError::TransactionPoisoned { .. })
        ));
        assert!(engine.action_state().is_empty());
        assert!(!engine.action_state().has_prepared());
    }

    #[test]
    fn foreign_composition_handle_is_rejected() {
        let (first, first_handle, _) = fixture("state_test");
        let (_second, second_handle, _) = fixture("state_test");
        assert!(matches!(
            first.action_state().get(&second_handle, &1),
            Err(ActionStateError::ForeignHandle { .. })
        ));
        assert!(
            first
                .action_state()
                .get(&first_handle, &1)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn cross_owner_handle_attempt_poisons_transaction() {
        let (mut first, first_handle, _) = fixture("state_test");
        let (_other, other_handle, _) = fixture("other_state");

        // The foreign-composition brand wins before owner scope; neither path
        // can disclose or mutate another owner's state.
        let mut txn = first.action_transaction(&first_handle).unwrap();
        assert!(txn.get(&other_handle, &1).is_err());
        txn.insert(&first_handle, &1, CounterState { amount: 1 })
            .unwrap();
        assert!(matches!(
            txn.prepare(),
            Err(ActionStateError::TransactionPoisoned { .. })
        ));
        assert!(first.action_state().is_empty());
    }

    #[test]
    fn hard_operation_bound_rejects_1025th_and_changes_nothing() {
        let (mut engine, handle, _) = fixture("state_test");
        let mut txn = engine.action_transaction(&handle).unwrap();
        for key in 0..MAX_ACTION_MUTATION_OPERATIONS as u64 {
            txn.insert(&handle, &key, CounterState { amount: 1 })
                .unwrap();
        }
        assert_eq!(txn.operation_count(), MAX_ACTION_MUTATION_OPERATIONS);
        assert!(matches!(
            txn.insert(
                &handle,
                &(MAX_ACTION_MUTATION_OPERATIONS as u64),
                CounterState { amount: 1 }
            ),
            Err(ActionStateError::OperationLimit { .. })
        ));
        assert!(matches!(
            txn.prepare(),
            Err(ActionStateError::TransactionPoisoned { .. })
        ));
        assert!(engine.action_state().is_empty());
    }

    #[test]
    fn stable_key_order_is_independent_of_insertion_order() {
        let (mut first, first_handle, _) = fixture("state_test");
        let (mut second, second_handle, _) = fixture("state_test");
        let first_keys = [9, 1, 4];
        let second_keys = [4, 9, 1];

        let mut first_txn = first.action_transaction(&first_handle).unwrap();
        for key in first_keys {
            first_txn
                .insert(&first_handle, &key, CounterState { amount: 1 })
                .unwrap();
        }
        let first_prepared = first_txn.prepare().unwrap();
        let first_order: Vec<_> = first_prepared
            .records()
            .iter()
            .map(|record| record.key().encoded().to_owned())
            .collect();

        let mut second_txn = second.action_transaction(&second_handle).unwrap();
        for key in second_keys {
            second_txn
                .insert(&second_handle, &key, CounterState { amount: 1 })
                .unwrap();
        }
        let second_prepared = second_txn.prepare().unwrap();
        let second_order: Vec<_> = second_prepared
            .records()
            .iter()
            .map(|record| record.key().encoded().to_owned())
            .collect();
        assert_eq!(first_order, vec!["1", "4", "9"]);
        assert_eq!(second_order, first_order);

        let (_, first_installation) = first_prepared.into_parts();
        first.abort_prepared_action(first_installation);
        let (_, second_installation) = second_prepared.into_parts();
        second.abort_prepared_action(second_installation);
    }

    #[test]
    fn staging_refuses_legacy_dirty_world_and_clears_reservation() {
        let (mut engine, handle, _) = fixture("state_test");
        let mut txn = engine.action_transaction(&handle).unwrap();
        txn.insert(&handle, &1, CounterState { amount: 1 }).unwrap();
        let prepared = txn.prepare().unwrap();
        let guards = empty_guards(&engine);

        assert_eq!(
            engine.stage_prepared_action(prepared, guards),
            Err(crate::StagePreparedActionError::DirtyWorld)
        );
        assert!(!engine.action_state().has_prepared());
        assert!(engine.action_state().is_empty());
    }
}
