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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, ThreadId};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use smol_str::SmolStr;
use thiserror::Error;

mod terminal_capacity;

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
/// Maximum other owners one owner may delegate its typed families to.
///
/// E7c3 B1: the delegation is the narrow bridge E7 cut 2 handed to cut 3 —
/// "an `ActionTxn` is owner-stamped, so a `mush-industry` provider cannot write
/// `inventory`'s families directly". It is deliberately tiny: a delegation is a
/// composition-time statement by the **owning** module that one named grantee
/// may write its families inside the grantee's own bundle, so a two-owner
/// post-image (a deposit's pool debited in the same commit that mints the
/// extractor's lot) is one bundle rather than two. It is never implicit,
/// never transitive and never symmetric.
pub const MAX_STATE_FAMILY_DELEGATES: usize = 4;
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

/// Why a [`TypedStateLens`] carries no live [`ActionStateStore`].
///
/// A context that cannot read typed state is not a bug — the off-loop script
/// snapshot, the local REPL and hand-built compatibility contexts genuinely
/// have no store to read. What *was* a bug is not being able to tell those
/// apart from a production call that lost its store on the way down, because
/// both spelled themselves `None` and both silently answered "typed state is
/// unavailable". The variants below are that missing sentence: every detached
/// lens says which of the four it is, at the point it is built.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DetachedTypedState {
    /// The off-loop Rhai runtime, reading a snapshot of the hook slice taken
    /// before the world task moved on (`mush-script`). There is no live store
    /// on that thread by construction.
    ScriptSnapshot,
    /// The single-player local REPL reading outside a dispatched command.
    Repl,
    /// A hand-built read context on a path the engine does not dispatch —
    /// [`HookReadContext::without_typed_state`](crate::HookReadContext::without_typed_state)
    /// and the store-less public wrappers that still shadow their
    /// typed-state-carrying partners.
    CompatibilityCtx,
    /// A test that deliberately calls a perception or veto predicate with no
    /// store, to pin the store-less answer.
    Test,
}

impl DetachedTypedState {
    /// The stable lower-case name used in diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ScriptSnapshot => "script-snapshot",
            Self::Repl => "repl",
            Self::CompatibilityCtx => "compatibility-ctx",
            Self::Test => "test",
        }
    }
}

impl fmt::Display for DetachedTypedState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// How one call reaches engine-owned typed state: the live authoritative
/// store, or a named reason there is none.
///
/// This is the parameter every perception, veto, dispatch and render entry
/// point in the engine takes. It replaces the `Option<&ActionStateStore>`
/// those calls used to thread, and — more importantly — it replaces the
/// *pairs* of names that grew around that `Option`: a plain `can_perceive`
/// that passed `None`, and a longer twin beside it that passed the store. The
/// plain name was the wrong default and the easy one to reach, so a caller
/// that should have carried the store could quietly get the store-less answer
/// — the wrong perception verdict, not a degraded one. With one name taking
/// one lens, a store-less call has to say so. No twin is left: the pin at
/// `crates/mush-contract-tests/typed-state-lens.toml` is what keeps it that
/// way.
#[derive(Copy, Clone)]
pub enum TypedStateLens<'a> {
    /// The live authoritative snapshot the world task owns.
    Live(&'a ActionStateStore),
    /// No store, and why.
    Detached(DetachedTypedState),
}

impl fmt::Debug for TypedStateLens<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Live(_) => formatter.write_str("TypedStateLens::Live"),
            Self::Detached(reason) => formatter
                .debug_tuple("TypedStateLens::Detached")
                .field(&reason.as_str())
                .finish(),
        }
    }
}

impl<'a> TypedStateLens<'a> {
    /// The live store, or `None` for a detached lens.
    #[must_use]
    pub const fn store(self) -> Option<&'a ActionStateStore> {
        match self {
            Self::Live(store) => Some(store),
            Self::Detached(_) => None,
        }
    }

    /// Whether this lens carries the live authoritative store.
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Live(_))
    }

    /// Why there is no store, or `None` when there is one.
    #[must_use]
    pub const fn detached(self) -> Option<DetachedTypedState> {
        match self {
            Self::Live(_) => None,
            Self::Detached(reason) => Some(reason),
        }
    }
}

/// Read-only, owner-scoped view of engine-owned typed state supplied to an
/// ordinary module context.
///
/// The backing store and owner stamp are private. A module can use only an
/// opaque family handle minted for its own owner or explicitly delegated
/// to it at composition; this surface cannot mutate state.
pub struct ModuleActionState<'a> {
    lens: TypedStateLens<'a>,
    owner: Option<OwnerId>,
    dispatch: Option<crate::read_service::OwnerReadCaller>,
}

impl fmt::Debug for ModuleActionState<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModuleActionState")
            .field("lens", &self.lens)
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
            lens: TypedStateLens::Detached(DetachedTypedState::CompatibilityCtx),
            owner: None,
            dispatch: None,
        }
    }

    pub(crate) fn for_module(lens: TypedStateLens<'a>, module: Option<&str>) -> Self {
        Self {
            lens,
            owner: module.and_then(|name| OwnerId::parse_canonical(name).ok()),
            dispatch: None,
        }
    }

    /// Captured by actual core dispatch, independent of the compatibility
    /// context's public actor/module fields. No external constructor can mint it.
    pub(crate) fn for_dispatch(
        lens: TypedStateLens<'a>,
        module: Option<&str>,
        actor: crate::ObjId,
    ) -> Self {
        let mut state = Self::for_module(lens, module);
        state.dispatch =
            module.and_then(|name| crate::read_service::OwnerReadCaller::object(name, actor).ok());
        state
    }

    pub(crate) fn dispatch_caller(&self) -> Option<&crate::read_service::OwnerReadCaller> {
        self.dispatch.as_ref()
    }

    /// The lens this reader was built from, so a module holding a
    /// [`Ctx`](crate::Ctx) can hand its own live typed state to an engine
    /// predicate instead of reaching for a store-less name.
    #[must_use]
    pub const fn lens(&self) -> TypedStateLens<'a> {
        self.lens
    }

    #[must_use]
    pub const fn is_available(&self) -> bool {
        self.lens.store().is_some() && self.owner.is_some()
    }

    /// Whether a prepared action owns the underlying typed store (doc 18 cut
    /// IC14, row 10): staged, or in the writer's custody, with its post-images
    /// not yet installed ([`ActionStateStore::has_prepared`]). One action is
    /// prepared at a time, so `true` means *some* actor's commit is in flight,
    /// not necessarily the asker's. `false` for an unavailable reader (a
    /// manually built context sees no store and so no commit; the same honest
    /// blind spot [`crate::Outbox::authoritative_pending`] has outside a note).
    #[must_use]
    pub fn commit_pending(&self) -> bool {
        self.lens
            .store()
            .is_some_and(ActionStateStore::has_prepared)
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
        let store = self
            .lens
            .store()
            .ok_or(ModuleActionStateReadError::Unavailable)?;
        self.require_owner(handle.owner(), &handle.descriptor().id)?;
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
        let store = self
            .lens
            .store()
            .ok_or(ModuleActionStateReadError::Unavailable)?;
        self.require_owner(handle.owner(), &handle.descriptor().id)?;
        store.scan_family(handle, after, limit).map_err(Into::into)
    }

    fn require_owner(
        &self,
        family: &OwnerId,
        id: &StateFamilyId,
    ) -> Result<(), ModuleActionStateReadError> {
        let context = self
            .owner
            .as_ref()
            .ok_or(ModuleActionStateReadError::UnownedContext)?;
        if context == family {
            return Ok(());
        }
        // E7c3 B1: the same composition-declared delegation the transaction
        // gate consults. A grantee that may write a family may also read it —
        // the reverse would leave `mush-industry` able to debit a deposit's
        // pool and unable to tell an operator what is left in it.
        if self
            .lens
            .store()
            .is_some_and(|store| store.registry.delegates_family(family, id, context))
        {
            return Ok(());
        }
        Err(ModuleActionStateReadError::ForeignOwner {
            context: context.clone(),
            family: family.clone(),
        })
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
    /// `(family owner, grantee)` — see [`MAX_STATE_FAMILY_DELEGATES`].
    delegations: BTreeSet<(OwnerId, OwnerId)>,
    /// Exact `(family owner, family id, grantee)` grants.
    family_delegations: BTreeSet<(OwnerId, StateFamilyId, OwnerId)>,
    registry_rating: Option<terminal_capacity::Protocol>,
    original_purchase: Option<terminal_capacity::PurchaseProtocol>,
    entitlement_purchase: Option<terminal_capacity::EntitlementProtocol>,
    house_purchase: Option<terminal_capacity::HouseProtocol>,
    charter_pending: Option<terminal_capacity::CharterPendingProtocol>,
    mind_renewal: Option<terminal_capacity::RenewalProtocol>,
    ic_mind_claim: Option<terminal_capacity::IcClaimProtocol>,
    core_integration: Option<terminal_capacity::CoreIntegrationProtocol>,
}

impl fmt::Debug for StateFamilyRegistryBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateFamilyRegistryBuilder")
            .field("families", &self.entries.keys().collect::<Vec<_>>())
            .field("owner_counts", &self.owner_counts)
            .field("delegations", &self.delegations)
            .field("family_delegations", &self.family_delegations)
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

    /// Delegate write and read authority over `owner`'s typed families to one
    /// other composed `grantee`.
    ///
    /// Declared by the **owning** module during its own registration, never by
    /// the grantee: a module cannot help itself to another's families. The
    /// grantee must be a composed owner at freeze time.
    pub(crate) fn delegate(
        &mut self,
        owner: OwnerId,
        grantee: OwnerId,
    ) -> Result<(), StateFamilyRegistryError> {
        self.check_delegate(&owner, &grantee)?;
        self.delegations.insert((owner, grantee));
        Ok(())
    }

    /// Delegate one already-registered family. An opaque handle from another
    /// composition, or one owned by another module, cannot create a grant.
    pub(crate) fn delegate_family<K: 'static, V: 'static>(
        &mut self,
        owner: OwnerId,
        handle: &StateFamilyHandle<K, V>,
        grantee: OwnerId,
    ) -> Result<(), StateFamilyRegistryError> {
        if handle.owner() != &owner {
            return Err(StateFamilyRegistryError::DelegationOwnerMismatch {
                caller: owner,
                family_owner: handle.owner().clone(),
            });
        }
        let locator = (owner.clone(), handle.descriptor().id.clone());
        let Some(registered) = self.entries.get(&locator) else {
            return Err(StateFamilyRegistryError::UnknownDelegatedFamily {
                owner,
                family: handle.descriptor().id.clone(),
            });
        };
        if !Arc::ptr_eq(registered, &handle.family) {
            return Err(StateFamilyRegistryError::ForeignDelegatedFamily {
                owner,
                family: handle.descriptor().id.clone(),
            });
        }
        self.check_delegate(&owner, &grantee)?;
        self.family_delegations
            .insert((owner, handle.descriptor().id.clone(), grantee));
        Ok(())
    }

    fn check_delegate(
        &self,
        owner: &OwnerId,
        grantee: &OwnerId,
    ) -> Result<(), StateFamilyRegistryError> {
        if owner == grantee {
            return Err(StateFamilyRegistryError::SelfDelegation {
                owner: owner.clone(),
            });
        }
        // A mixture of scoped and legacy grants cannot bypass the existing
        // distinct-grantee cap. Repeating a grant consumes no additional slot.
        let grantees: BTreeSet<_> = self
            .delegations
            .iter()
            .filter(|(from, _)| from == owner)
            .map(|(_, to)| to)
            .chain(
                self.family_delegations
                    .iter()
                    .filter(|(from, _, _)| from == owner)
                    .map(|(_, _, to)| to),
            )
            .collect();
        if grantees.len() >= MAX_STATE_FAMILY_DELEGATES && !grantees.contains(grantee) {
            return Err(StateFamilyRegistryError::OwnerDelegateLimit {
                owner: owner.clone(),
                max: MAX_STATE_FAMILY_DELEGATES,
            });
        }
        Ok(())
    }

    pub(crate) fn registry_rating_capacity(
        &mut self,
        owner: &OwnerId,
        manifest: crate::DefinitionRef,
        actions: [crate::DefinitionRef; 5],
    ) -> Result<(), ActionStateError> {
        if self.registry_rating.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate Registry protocol",
            ));
        }
        self.registry_rating = Some(terminal_capacity::Protocol::new(owner, manifest, actions)?);
        Ok(())
    }

    pub(crate) fn registry_reconformance_capacity(
        &mut self,
        owner: &OwnerId,
        actions: [crate::DefinitionRef; 5],
    ) -> Result<(), ActionStateError> {
        self.registry_rating
            .as_mut()
            .ok_or(ActionStateError::TerminalCapacity(
                "Registry rating declaration required",
            ))?
            .with_reconformance(owner, actions)
    }

    pub(crate) fn original_mind_purchase_capacity(
        &mut self,
        owner: &OwnerId,
        manifest: crate::DefinitionRef,
        participants: [crate::DefinitionRef; 2],
        actions: [crate::DefinitionRef; 3],
    ) -> Result<(), ActionStateError> {
        if self.original_purchase.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate purchase protocol",
            ));
        }
        self.original_purchase = Some(terminal_capacity::PurchaseProtocol::new(
            owner,
            manifest,
            participants,
            actions,
        )?);
        Ok(())
    }

    pub(crate) fn charter_pending_capacity(
        &mut self,
        owner: &OwnerId,
        participants: [crate::DefinitionRef; 2],
        actions: [crate::DefinitionRef; 4],
        registry_revocation: Option<crate::DefinitionRef>,
    ) -> Result<(), ActionStateError> {
        if self.charter_pending.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate pending charter protocol",
            ));
        }
        self.charter_pending = Some(terminal_capacity::CharterPendingProtocol::new(
            owner,
            participants,
            actions,
            registry_revocation,
        )?);
        Ok(())
    }

    pub(crate) fn house_mind_purchase_capacity(
        &mut self,
        owner: &OwnerId,
        manifest: crate::DefinitionRef,
        participants: [crate::DefinitionRef; 2],
        actions: [crate::DefinitionRef; 3],
    ) -> Result<(), ActionStateError> {
        if self.house_purchase.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate purchase protocol",
            ));
        }
        self.house_purchase = Some(terminal_capacity::HouseProtocol::new(
            owner,
            manifest,
            participants,
            actions,
        )?);
        Ok(())
    }

    pub(crate) fn mind_entitlement_purchase_capacity(
        &mut self,
        owner: &OwnerId,
        manifest: crate::DefinitionRef,
        participants: [crate::DefinitionRef; 2],
        original_actions: [crate::DefinitionRef; 3],
        derived_actions: [crate::DefinitionRef; 3],
    ) -> Result<(), ActionStateError> {
        if self.entitlement_purchase.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate entitlement purchase protocol",
            ));
        }
        self.entitlement_purchase = Some(terminal_capacity::EntitlementProtocol::new(
            owner,
            manifest,
            participants,
            original_actions,
            derived_actions,
        )?);
        Ok(())
    }

    pub(crate) fn core_integration_capacity(
        &mut self,
        owner: &OwnerId,
        participants: [crate::DefinitionRef; 2],
        actions: [crate::DefinitionRef; 5],
    ) -> Result<(), ActionStateError> {
        if self.core_integration.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate core integration protocol",
            ));
        }
        self.core_integration = Some(terminal_capacity::CoreIntegrationProtocol::new(
            owner,
            participants,
            actions,
        )?);
        Ok(())
    }

    pub(crate) fn core_integration_delivery_capacity(
        &mut self,
        owner: &OwnerId,
        participants: [crate::DefinitionRef; 2],
        actions: [crate::DefinitionRef; 6],
    ) -> Result<(), ActionStateError> {
        if self.core_integration.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate core integration protocol",
            ));
        }
        self.core_integration = Some(terminal_capacity::CoreIntegrationProtocol::with_delivery(
            owner,
            participants,
            actions,
        )?);
        Ok(())
    }

    pub(crate) fn ic_mind_claim_capacity(
        &mut self,
        owner: &OwnerId,
        manifest: crate::DefinitionRef,
        participants: [crate::DefinitionRef; 4],
        actions: [crate::DefinitionRef; 2],
    ) -> Result<(), ActionStateError> {
        if self.ic_mind_claim.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate IC claim protocol",
            ));
        }
        self.ic_mind_claim = Some(terminal_capacity::IcClaimProtocol::new(
            owner,
            manifest,
            participants,
            actions,
        )?);
        Ok(())
    }

    pub(crate) fn mind_renewal_capacity(
        &mut self,
        owner: &OwnerId,
        manifest: crate::DefinitionRef,
        participants: [crate::DefinitionRef; 2],
        actions: [crate::DefinitionRef; 5],
    ) -> Result<(), ActionStateError> {
        if self.mind_renewal.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate renewal protocol",
            ));
        }
        self.mind_renewal = Some(terminal_capacity::RenewalProtocol::new(
            owner,
            manifest,
            participants,
            actions,
        )?);
        Ok(())
    }

    pub(crate) fn mind_renewal_capacity_with_service(
        &mut self,
        owner: &OwnerId,
        manifest: crate::DefinitionRef,
        participants: [crate::DefinitionRef; 2],
        actions: [crate::DefinitionRef; 5],
        service_close: crate::DefinitionRef,
    ) -> Result<(), ActionStateError> {
        if self.mind_renewal.is_some() {
            return Err(ActionStateError::TerminalCapacity(
                "duplicate renewal protocol",
            ));
        }
        self.mind_renewal = Some(terminal_capacity::RenewalProtocol::with_service(
            owner,
            manifest,
            participants,
            actions,
            service_close,
        )?);
        Ok(())
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
        for (owner, grantee) in &self.delegations {
            for named in [owner, grantee] {
                if !module_owners.contains(named) {
                    return Err(StateFamilyRegistryError::UnknownOwner {
                        owner: named.clone(),
                    });
                }
            }
        }
        for (owner, _, grantee) in &self.family_delegations {
            for named in [owner, grantee] {
                if !module_owners.contains(named) {
                    return Err(StateFamilyRegistryError::UnknownOwner {
                        owner: named.clone(),
                    });
                }
            }
        }
        let fingerprint = state_family_fingerprint(
            self.entries.values(),
            &self.delegations,
            &self.family_delegations,
            self.registry_rating.as_ref(),
            self.original_purchase.as_ref(),
            self.entitlement_purchase.as_ref(),
            self.mind_renewal.as_ref(),
            self.ic_mind_claim.as_ref(),
            self.core_integration.as_ref(),
            self.house_purchase.as_ref(),
            self.charter_pending.as_ref(),
        )?;
        let registry = StateFamilyRegistry {
            entries: self.entries,
            delegations: self.delegations,
            family_delegations: self.family_delegations,
            registry_rating: self.registry_rating,
            original_purchase: self.original_purchase,
            entitlement_purchase: self.entitlement_purchase,
            mind_renewal: self.mind_renewal,
            ic_mind_claim: self.ic_mind_claim,
            core_integration: self.core_integration,
            house_purchase: self.house_purchase,
            charter_pending: self.charter_pending,
            fingerprint,
            owner_readiness,
        };
        if let Some(protocol) = &registry.registry_rating {
            protocol
                .validate_families(&registry)
                .map_err(|e| StateFamilyRegistryError::RegistryRating(e.to_string()))?;
        }
        if let Some(protocol) = &registry.original_purchase {
            protocol
                .validate_families(&registry)
                .map_err(|e| StateFamilyRegistryError::OriginalPurchase(e.to_string()))?;
        }
        if let Some(protocol) = &registry.entitlement_purchase {
            protocol
                .validate_families(&registry)
                .map_err(|e| StateFamilyRegistryError::EntitlementPurchase(e.to_string()))?;
        }
        if let Some(protocol) = &registry.charter_pending {
            protocol
                .validate_families(&registry)
                .map_err(|e| StateFamilyRegistryError::CharterPending(e.to_string()))?;
        }
        if let Some(protocol) = &registry.house_purchase {
            protocol
                .validate_families(&registry)
                .map_err(|e| StateFamilyRegistryError::HousePurchase(e.to_string()))?;
        }
        if let Some(protocol) = &registry.mind_renewal {
            protocol
                .validate_families(&registry)
                .map_err(|e| StateFamilyRegistryError::MindRenewal(e.to_string()))?;
        }
        if let Some(protocol) = &registry.ic_mind_claim {
            protocol
                .validate_families(&registry)
                .map_err(|e| StateFamilyRegistryError::IcMindClaim(e.to_string()))?;
        }
        if let Some(protocol) = &registry.core_integration {
            protocol
                .validate_families(&registry)
                .map_err(|e| StateFamilyRegistryError::CoreIntegration(e.to_string()))?;
        }
        Ok(registry)
    }
}

/// Frozen state-family registration table.
#[derive(Clone)]
pub struct StateFamilyRegistry {
    entries: BTreeMap<StateFamilyLocator, Arc<dyn ErasedStateFamily>>,
    delegations: BTreeSet<(OwnerId, OwnerId)>,
    /// Exact `(family owner, family id, grantee)` grants.
    family_delegations: BTreeSet<(OwnerId, StateFamilyId, OwnerId)>,
    registry_rating: Option<terminal_capacity::Protocol>,
    original_purchase: Option<terminal_capacity::PurchaseProtocol>,
    entitlement_purchase: Option<terminal_capacity::EntitlementProtocol>,
    house_purchase: Option<terminal_capacity::HouseProtocol>,
    charter_pending: Option<terminal_capacity::CharterPendingProtocol>,
    mind_renewal: Option<terminal_capacity::RenewalProtocol>,
    ic_mind_claim: Option<terminal_capacity::IcClaimProtocol>,
    core_integration: Option<terminal_capacity::CoreIntegrationProtocol>,
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
    pub(crate) fn validate_registry_rating_composition(
        &self,
        definitions: &crate::mechanics::DefinitionRegistry,
        providers: &crate::action::ActionProviderRegistry,
        participants: &mut crate::participant::ParticipantRegistry,
    ) -> Result<(), ActionStateError> {
        if let Some(protocol) = &self.registry_rating {
            protocol.validate_composition(definitions, providers, participants)?;
        }
        if let Some(protocol) = &self.original_purchase {
            protocol.validate_composition(definitions, providers, participants)?;
        }
        if let Some(protocol) = &self.entitlement_purchase {
            protocol.validate_composition(definitions, providers, participants)?;
        }
        if let Some(protocol) = &self.charter_pending {
            protocol.validate_composition(definitions, providers, participants)?;
        }
        if let Some(protocol) = &self.house_purchase {
            protocol.validate_composition(definitions, providers, participants)?;
        }
        if let Some(protocol) = &self.mind_renewal {
            protocol.validate_composition(definitions, providers, participants)?;
        }
        if let Some(protocol) = &self.core_integration {
            protocol.validate_composition(definitions, providers, participants)?;
        }
        if let Some(protocol) = &self.ic_mind_claim {
            protocol.validate_composition(definitions, providers, participants)?;
            protocol
                .enable_participant_limits(participants)
                .map_err(|_| {
                    ActionStateError::TerminalCapacity("IC image admission configuration")
                })?;
        }
        Ok(())
    }

    /// Whether `grantee` was delegated authority over `family_owner`'s typed
    /// families at composition. Never transitive, never symmetric.
    #[must_use]
    pub fn delegates(&self, family_owner: &OwnerId, grantee: &OwnerId) -> bool {
        self.delegations
            .contains(&(family_owner.clone(), grantee.clone()))
    }

    /// Whether an explicit scoped or legacy owner-wide grant permits access
    /// to this exact family. This does not replace opaque-handle validation.
    #[must_use]
    pub fn delegates_family(
        &self,
        family_owner: &OwnerId,
        family: &StateFamilyId,
        grantee: &OwnerId,
    ) -> bool {
        self.delegates(family_owner, grantee)
            || self.family_delegations.contains(&(
                family_owner.clone(),
                family.clone(),
                grantee.clone(),
            ))
    }

    /// Every explicit `(family owner, family id, grantee)` scoped grant,
    /// in canonical order. Legacy owner-wide grants remain in `delegations`.
    #[must_use]
    pub fn family_delegations(&self) -> Vec<(OwnerId, StateFamilyId, OwnerId)> {
        self.family_delegations.iter().cloned().collect()
    }

    /// Every legacy owner-wide `(family owner, grantee)` grant declared.
    #[must_use]
    pub fn delegations(&self) -> Vec<(OwnerId, OwnerId)> {
        self.delegations.iter().cloned().collect()
    }

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
    delegations: &BTreeSet<(OwnerId, OwnerId)>,
    family_delegations: &BTreeSet<(OwnerId, StateFamilyId, OwnerId)>,
    registry_rating: Option<&terminal_capacity::Protocol>,
    original_purchase: Option<&terminal_capacity::PurchaseProtocol>,
    entitlement_purchase: Option<&terminal_capacity::EntitlementProtocol>,
    mind_renewal: Option<&terminal_capacity::RenewalProtocol>,
    ic_mind_claim: Option<&terminal_capacity::IcClaimProtocol>,
    core_integration: Option<&terminal_capacity::CoreIntegrationProtocol>,
    house_purchase: Option<&terminal_capacity::HouseProtocol>,
    charter_pending: Option<&terminal_capacity::CharterPendingProtocol>,
) -> Result<DefinitionDigest, CanonicalEncodingError> {
    let mut value = serde_json::Map::from_iter([
        (
            "families".to_owned(),
            Value::Array(
                entries
                    .map(|family| {
                        json!({
                            "owner": family.owner(),
                            "descriptor": family.descriptor().canonical_value(),
                        })
                    })
                    .collect(),
            ),
        ),
        (
            "delegations".to_owned(),
            Value::Array(
                delegations
                    .iter()
                    .map(|(owner, grantee)| json!({ "owner": owner, "grantee": grantee }))
                    .collect(),
            ),
        ),
    ]);
    // Preserve the exact existing domain and canonical bytes when no scoped
    // grants exist. A new scoped grant changes the frozen deployment identity.
    let domain = if family_delegations.is_empty() {
        "state_family_registry.v1"
    } else {
        value.insert(
            "family_delegations".to_owned(),
            Value::Array(
                family_delegations
                    .iter()
                    .map(|(owner, family, grantee)| {
                        json!({ "owner": owner, "family": family, "grantee": grantee })
                    })
                    .collect(),
            ),
        );
        "state_family_registry.v2"
    };
    let domain = if let Some(protocol) = registry_rating {
        value.insert("registry_rating".to_owned(), json!(protocol));
        "state_family_registry.v3"
    } else {
        domain
    };
    let domain = if let Some(protocol) = original_purchase {
        value.insert("original_purchase".to_owned(), json!(protocol));
        "state_family_registry.v4"
    } else {
        domain
    };
    let domain = if let Some(protocol) = entitlement_purchase {
        value.insert("entitlement_purchase".to_owned(), json!(protocol));
        "state_family_registry.v5"
    } else {
        domain
    };
    let domain = if let Some(protocol) = mind_renewal {
        value.insert("mind_renewal".to_owned(), json!(protocol));
        if protocol.has_service() {
            "state_family_registry.v7"
        } else {
            "state_family_registry.v6"
        }
    } else {
        domain
    };
    let domain = if let Some(protocol) = ic_mind_claim {
        value.insert("ic_mind_claim".to_owned(), json!(protocol));
        "state_family_registry.v8"
    } else {
        domain
    };
    let domain = if let Some(protocol) = core_integration {
        value.insert("core_integration".to_owned(), json!(protocol));
        "state_family_registry.v9"
    } else {
        domain
    };
    let domain = if let Some(protocol) = house_purchase {
        value.insert("house_purchase".to_owned(), json!(protocol));
        "state_family_registry.v10"
    } else {
        domain
    };
    let domain = if let Some(protocol) = charter_pending {
        value.insert("charter_pending".to_owned(), json!(protocol));
        "state_family_registry.v11"
    } else {
        domain
    };
    DefinitionDigest::for_canonical_value(domain, &Value::Object(value))
}

/// State-family registration failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StateFamilyRegistryError {
    #[error("Pending charter capacity composition: {0}")]
    CharterPending(String),
    #[error("House purchase capacity composition: {0}")]
    HousePurchase(String),
    #[error("Core integration capacity composition: {0}")]
    CoreIntegration(String),
    #[error("IC mind claim capacity composition: {0}")]
    IcMindClaim(String),
    #[error("Mind renewal capacity composition: {0}")]
    MindRenewal(String),
    #[error("Entitlement purchase capacity composition: {0}")]
    EntitlementPurchase(String),
    #[error("Original purchase capacity composition: {0}")]
    OriginalPurchase(String),
    #[error("Registry rating capacity composition: {0}")]
    RegistryRating(String),
    #[error("state-family registry exceeds its hard limit of {max}")]
    FamilyLimit { max: usize },
    #[error("owner {owner} exceeds its state-family limit of {max}")]
    OwnerFamilyLimit { owner: OwnerId, max: usize },
    #[error("module {caller} cannot delegate a state family owned by {family_owner}")]
    DelegationOwnerMismatch {
        caller: OwnerId,
        family_owner: OwnerId,
    },
    #[error("delegated state family {owner}/{family} is not registered")]
    UnknownDelegatedFamily {
        owner: OwnerId,
        family: StateFamilyId,
    },
    #[error("delegated state family {owner}/{family} belongs to another composition")]
    ForeignDelegatedFamily {
        owner: OwnerId,
        family: StateFamilyId,
    },
    #[error("owner {owner} cannot delegate its state families to itself")]
    SelfDelegation { owner: OwnerId },
    #[error("owner {owner} exceeds its state-family delegate limit of {max}")]
    OwnerDelegateLimit { owner: OwnerId, max: usize },
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

/// The one commit reservation a store can hold, shared by pointer with the
/// [`PreparedTxn`]/[`PreparedInstallation`] minted from it.
///
/// It is one allocation doing two jobs that used to be two: it *is* the store
/// identity [`ActionStateStore::check_installation`] proves with
/// `Arc::ptr_eq`, and it carries the outstanding reservation itself, so the
/// prepared value can release what it reserved when it dies unstaged
/// (core-action-8). `pending == 0` means unreserved; reservations start at 1
/// because [`ActionTxn::prepare`] increments before it stores, so zero is a
/// sentinel no live reservation can collide with.
struct ReservationSlot {
    pending: AtomicU64,
    dropped: AtomicU64,
    lost: AtomicBool,
    lost_installations: AtomicU64,
}

impl ReservationSlot {
    fn new() -> Self {
        Self {
            pending: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            lost: AtomicBool::new(false),
            lost_installations: AtomicU64::new(0),
        }
    }

    fn pending(&self) -> Option<u64> {
        match self.pending.load(Ordering::Relaxed) {
            0 => None,
            reservation => Some(reservation),
        }
    }
}

/// How far the reservation this guard holds has travelled. Only the first
/// stage releases on drop: a [`PreparedTxn`] that dies never staged anything
/// durable, so its reservation is pure leaked bookkeeping. Once
/// [`PreparedTxn::into_parts`] has handed the module ops out, a durable write
/// may already be in flight, and silently freeing the slot would let the next
/// transaction prepare against RAM the acknowledged write was never installed
/// into — so the installation stage stays fail-closed and wedged, and a drop
/// there is a loss witness: `lost` latches, `lost_installations` counts, and
/// the store refuses every later transaction with
/// [`ActionStateError::InstallationLost`] (WS03-S11).
enum ReservationStage {
    Prepared,
    Installation,
    Settled,
}

/// The store-side reservation, owned by whichever prepared value currently
/// stands for the pending commit.
///
/// Its `Drop` is the whole point of core-action-8: it runs on the world task,
/// so it may never panic (docs/PLAN.md), and it is loud rather than silent —
/// a leaked reservation is a coordinator bug, not a gameplay outcome, and the
/// counter plus the `tracing::error!` are how it stops being invisible.
struct ReservationGuard {
    slot: Arc<ReservationSlot>,
    reservation: u64,
    /// The thread that armed the guard — the single world-owning task. A drop
    /// anywhere else cannot be ordered against the store, so it poisons
    /// instead of releasing.
    armed_on: ThreadId,
    stage: ReservationStage,
}

impl ReservationGuard {
    fn arm(slot: Arc<ReservationSlot>, reservation: u64) -> Self {
        Self {
            slot,
            reservation,
            armed_on: thread::current().id(),
            stage: ReservationStage::Prepared,
        }
    }

    /// Clear the slot for a commit that reached its definitive end — an
    /// install or an abort. Disarms, so the guard's own `Drop` stays quiet.
    fn settle(&mut self) {
        self.slot.pending.store(0, Ordering::Relaxed);
        self.stage = ReservationStage::Settled;
    }
}

impl Drop for ReservationGuard {
    fn drop(&mut self) {
        match self.stage {
            // `install`/`abort` reached a definitive answer and cleared the
            // slot themselves.
            ReservationStage::Settled => return,
            // `into_parts` already handed the module ops to the store, so a
            // durable write may have been acknowledged for post-images that
            // are now gone. Releasing the slot here would let the next
            // transaction prepare against RAM that acknowledged write was
            // never installed into: RAM and Redis would diverge with no
            // witness and no recovery. So the reservation stays held, the
            // loss latches, and the world task fails stop on it.
            ReservationStage::Installation => {
                self.slot.lost.store(true, Ordering::Release);
                self.slot.lost_installations.fetch_add(1, Ordering::Relaxed);
                tracing::error!(
                    reservation = self.reservation,
                    "prepared action-state installation was dropped without an install or an \
                     abort; its acknowledged post-images are lost and the store reservation is \
                     poisoned rather than released"
                );
                return;
            }
            ReservationStage::Prepared => {}
        }
        if thread::current().id() != self.armed_on {
            self.slot.lost.store(true, Ordering::Release);
            tracing::error!(
                reservation = self.reservation,
                "prepared action-state transaction was dropped off the world task; the reservation is poisoned rather than released"
            );
            return;
        }
        self.slot.pending.store(0, Ordering::Relaxed);
        self.slot.dropped.fetch_add(1, Ordering::Relaxed);
        tracing::error!(
            reservation = self.reservation,
            "prepared action-state transaction was dropped without being staged or aborted; its store reservation was released"
        );
    }
}

/// Engine-owned live authoritative state. It has no unrestricted mutable map
/// accessor; all gameplay mutation goes through [`ActionTxn`].
pub struct ActionStateStore {
    registry: StateFamilyRegistry,
    records: BTreeMap<StateKey, StoredStateRecord>,
    live_canonical_bytes: usize,
    capacity: Arc<terminal_capacity::CapacityState>,
    capacity_usage: terminal_capacity::Usage,
    capacity_verifiers: terminal_capacity::Verifiers,
    slot: Arc<ReservationSlot>,
    revision: u64,
    next_reservation: u64,
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
            .field("pending_reservation", &self.pending_reservation())
            .field("dropped_reservations", &self.dropped_reservations())
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
        let capacity_verifiers = terminal_capacity::Verifiers::for_registry(&registry);
        Self {
            registry,
            records: BTreeMap::new(),
            live_canonical_bytes: 0,
            capacity: Arc::default(),
            capacity_usage: terminal_capacity::Usage::default(),
            capacity_verifiers,
            slot: Arc::new(ReservationSlot::new()),
            revision: 0,
            next_reservation: 0,
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

    /// Whether a prepared commit currently owns this store.
    ///
    /// WS03-S2: no longer `const`. The reservation moved into an
    /// `Arc<ReservationSlot>` so a dropped [`PreparedTxn`] can release it from
    /// its own `Drop`, and an `AtomicU64` cannot be read in a `const fn`.
    #[must_use]
    pub fn has_prepared(&self) -> bool {
        self.slot.pending().is_some()
    }

    fn pending_reservation(&self) -> Option<u64> {
        self.slot.pending()
    }

    /// How many prepared transactions died on this store without ever being
    /// staged or aborted. Every count is a coordinator bug that used to wedge
    /// the store with [`ActionStateError::CommitPending`] forever
    /// (core-action-8); releasing keeps the writer usable, and this is the
    /// number that says it happened at all.
    #[must_use]
    pub fn dropped_reservations(&self) -> u64 {
        self.slot.dropped.load(Ordering::Relaxed)
    }

    /// Whether this store's reservation is poisoned: a prepared value died
    /// somewhere the release could not be ordered or could not be justified —
    /// off the world task that armed it, or after
    /// [`PreparedTxn::into_parts`] handed the durable half out. The
    /// reservation stays held and every later transaction refuses with
    /// [`ActionStateError::InstallationLost`].
    #[must_use]
    pub fn reservation_lost(&self) -> bool {
        self.slot.lost.load(Ordering::Acquire)
    }

    /// How many prepared installations died between [`PreparedTxn::into_parts`]
    /// and a definitive persistence answer.
    ///
    /// Each one is a lost write-through commit: the module ops had already
    /// left for the store, so the durable half may have been acknowledged
    /// while the typed RAM post-images went out with the dropped value. That
    /// is silent RAM/Redis divergence, which is why the store poisons instead
    /// of releasing — this is the number the boot/health report and the world
    /// task's supervision check read to turn the poison into a fail-stop.
    #[must_use]
    pub fn lost_installations(&self) -> u64 {
        self.slot.lost_installations.load(Ordering::Relaxed)
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
        if !self.records.is_empty()
            || !self.capacity.is_empty()
            || self.pending_reservation().is_some()
            || self.revision != 0
        {
            return Err(ActionStateError::HydrationStoreNotEmpty);
        }
        let mut hydrated = BTreeMap::new();
        let mut capacity_records = BTreeMap::new();
        let mut total_bytes = 0usize;
        for record in records {
            if hydrated.len() + capacity_records.len() >= MAX_ACTION_STATE_BOOT_RECORDS {
                return Err(ActionStateError::HydrationRecordLimit {
                    max: MAX_ACTION_STATE_BOOT_RECORDS,
                });
            }
            if terminal_capacity::is_capacity_key(&record.key) {
                let stored = terminal_capacity::decode(&record)?;
                total_bytes = total_bytes
                    .checked_add(stored.len())
                    .ok_or(ActionStateError::ArithmeticOverflow)?;
                if total_bytes > MAX_ACTION_STATE_BOOT_BYTES {
                    return Err(ActionStateError::HydrationBytesExceeded {
                        attempted: total_bytes,
                        max: MAX_ACTION_STATE_BOOT_BYTES,
                    });
                }
                if capacity_records
                    .insert(stored.id(), Arc::new(stored))
                    .is_some()
                {
                    return Err(ActionStateError::TerminalCapacity("duplicate metadata"));
                }
                continue;
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
        let (capacity, usage) = terminal_capacity::hydrate(
            &self.registry,
            &self.capacity_verifiers,
            capacity_records,
            &hydrated,
            total_bytes,
        )?;
        let typed_bytes = total_bytes
            .checked_sub(capacity.metadata_bytes())
            .ok_or(ActionStateError::ArithmeticOverflow)?;
        self.records = hydrated;
        self.live_canonical_bytes = typed_bytes;
        self.capacity = Arc::new(capacity);
        self.capacity_usage = usage;
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
        // The poison outranks the ordinary wedge. `CommitPending` says "wait
        // for the commit in flight"; this says "there is no commit to wait
        // for, and what it would have installed is gone" — a caller that
        // retried on it would be preparing against a revision the store may
        // never have received.
        if self.reservation_lost() {
            return Err(ActionStateError::InstallationLost {
                reservation: self.pending_reservation().unwrap_or_default(),
            });
        }
        if let Some(reservation) = self.pending_reservation() {
            return Err(ActionStateError::CommitPending { reservation });
        }
        let limits = ActionTxnLimits::default();
        Ok(ActionTxn {
            store: self,
            owner,
            provider_ordinal,
            mutations: BTreeMap::new(),
            capacity_changes: terminal_capacity::Changes::default(),
            registered_identity: None,
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
    /// Every identity, bound, type and pre-state check happened before
    /// [`PreparedInstallation`] was created, so a refusal here can only be an
    /// engine wiring bug — never a gameplay outcome. It is still returned
    /// rather than asserted: this runs on the single world-owning task, which
    /// must never panic (docs/PLAN.md), so a wiring bug has to freeze
    /// persistence instead of taking the writer down with it.
    ///
    /// The whole check happens before the first mutation, so a refusal leaves
    /// `records`, `revision` and the reservation exactly as they were.
    pub fn install(&mut self, installation: PreparedInstallation) -> Result<(), ActionStateError> {
        self.bind_installation(installation)?
            .apply_images()
            .complete();
        Ok(())
    }

    /// Bind the complete typed pre-state before any image is installed. The
    /// exclusive borrow prevents it moving between this check and application.
    /// This stays private until the complete registered-participant protocol
    /// can bind every owner before applying any of them.
    pub(crate) fn bind_installation(
        &mut self,
        installation: PreparedInstallation,
    ) -> Result<BoundTypedInstall<'_>, ActionStateError> {
        self.check_installation(&installation)?;
        Ok(BoundTypedInstall {
            store: self,
            installation,
        })
    }

    /// Drop prepared post-images after a definitive persistence refusal.
    ///
    /// Refuses on the same three identity checks as [`Self::install`], and for
    /// the same reason: an abort that does not match its store is a wiring
    /// bug, and the single writer may not panic over one.
    pub fn abort(&mut self, installation: PreparedInstallation) -> Result<(), ActionStateError> {
        self.check_installation(&installation)?;
        let PreparedInstallation { mut guard, .. } = installation;
        guard.settle();
        Ok(())
    }

    /// The three things a prepared installation must prove about the store it
    /// is being applied to. Pure: it never touches live state, so every caller
    /// can run it first and stay all-or-nothing.
    fn check_installation(
        &self,
        installation: &PreparedInstallation,
    ) -> Result<(), ActionStateError> {
        if !Arc::ptr_eq(&self.slot, &installation.guard.slot) {
            return Err(ActionStateError::ForeignStoreIdentity);
        }
        let pending = self.pending_reservation();
        if pending != Some(installation.guard.reservation) {
            return Err(ActionStateError::ReservationMismatch {
                expected: installation.guard.reservation,
                found: pending,
            });
        }
        if self.revision != installation.base_revision {
            return Err(ActionStateError::BaseRevisionMoved {
                expected: installation.base_revision,
                found: self.revision,
            });
        }
        Ok(())
    }
}

/// A validated installation exclusively bound to its exact typed store.
/// Dropping it before application preserves all images and drops the existing
/// installation-stage guard, which retains the reservation and latches loss.
#[must_use = "apply and complete the bound installation; dropping it fails closed"]
pub(crate) struct BoundTypedInstall<'a> {
    store: &'a mut ActionStateStore,
    installation: PreparedInstallation,
}

impl<'a> BoundTypedInstall<'a> {
    /// Move only already-prepared images. No serializer, provider, external
    /// owner lookup or fallible operation runs after the first mutation.
    pub(crate) fn apply_images(self) -> UnsettledTypedInstall<'a> {
        let Self {
            store,
            installation,
        } = self;
        let PreparedInstallation {
            new_revision,
            live_bytes,
            capacity,
            capacity_usage,
            mutations,
            guard,
            ..
        } = installation;
        for (key, mutation) in mutations {
            match mutation {
                InstallMutation::Put { value, canonical } => {
                    store
                        .records
                        .insert(key, StoredStateRecord { value, canonical });
                }
                InstallMutation::Delete => {
                    let removed = store.records.remove(&key);
                    debug_assert!(removed.is_some(), "prepared delete retained its pre-state");
                }
            }
        }
        store.live_canonical_bytes = live_bytes;
        store.capacity = capacity;
        store.capacity_usage = capacity_usage;
        store.revision = new_revision;
        UnsettledTypedInstall {
            _store: store,
            guard,
        }
    }
}

/// The typed images are live, but the reservation stays held until the final
/// consuming completion. Keep the exclusive store borrow across that interval.
/// Dropping this is fail-stop, not a rollback or permission to start another
/// transaction; exact durable images must repair the owner on recovery.
#[must_use = "complete only after every bound image is installed"]
pub(crate) struct UnsettledTypedInstall<'a> {
    _store: &'a mut ActionStateStore,
    guard: ReservationGuard,
}

impl UnsettledTypedInstall<'_> {
    pub(crate) fn complete(mut self) {
        self.guard.settle();
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

/// Exact committed base captured by core before any provider callback. Keeping
/// the reservation allocation alive prevents a replacement store from matching.
pub(crate) struct ActionReadBase {
    slot: Arc<ReservationSlot>,
    revision: u64,
}

/// Scratch state for one owner. It borrows the live store mutably, making
/// concurrent preparation impossible even before a prepared reservation is
/// installed.
pub struct ActionTxn<'a> {
    store: &'a mut ActionStateStore,
    owner: OwnerId,
    provider_ordinal: Option<u32>,
    mutations: BTreeMap<StateKey, TxnMutation>,
    capacity_changes: terminal_capacity::Changes,
    registered_identity: Option<terminal_capacity::RegisteredTxnIdentity>,
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
    pub(crate) fn bind_registered_invocation(
        &mut self,
        provider: &crate::action::ActionProviderEntry,
        invocation: &crate::action::ActionInvocation,
        commit: crate::CommitToken,
    ) {
        self.registered_identity = Some(terminal_capacity::RegisteredTxnIdentity::new(
            provider, invocation, commit,
        ));
    }

    pub(crate) fn registry_rating_completion(
        &mut self,
        base: &ActionReadBase,
        provider: &ProviderRegistrationOrdinal,
        commit: crate::CommitToken,
        terminal: Option<crate::CommitToken>,
    ) -> Result<(), ActionStateError> {
        let result = (|| {
            self.read_preimage(base, provider).map_err(|_| {
                ActionStateError::TerminalCapacity("Registry transaction base differs")
            })?;
            terminal_capacity::registry_change(self, commit, terminal)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    pub(crate) fn original_mind_purchase_completion(
        &mut self,
        base: &ActionReadBase,
        provider: &ProviderRegistrationOrdinal,
        commit: crate::CommitToken,
        terminal: Option<crate::CommitToken>,
    ) -> Result<(), ActionStateError> {
        let result = (|| {
            self.read_preimage(base, provider).map_err(|_| {
                ActionStateError::TerminalCapacity("purchase transaction base differs")
            })?;
            terminal_capacity::purchase_change(self, commit, terminal)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    pub(crate) fn mind_entitlement_purchase_completion(
        &mut self,
        base: &ActionReadBase,
        provider: &ProviderRegistrationOrdinal,
        commit: crate::CommitToken,
        terminal: Option<crate::CommitToken>,
    ) -> Result<(), ActionStateError> {
        let result = (|| {
            self.read_preimage(base, provider).map_err(|_| {
                ActionStateError::TerminalCapacity("entitlement transaction base differs")
            })?;
            terminal_capacity::entitlement_change(self, commit, terminal)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }
    pub(crate) fn charter_pending_completion(
        &mut self,
        base: &ActionReadBase,
        provider: &ProviderRegistrationOrdinal,
        commit: crate::CommitToken,
        terminal: Option<crate::CommitToken>,
    ) -> Result<(), ActionStateError> {
        let result = (|| {
            self.read_preimage(base, provider).map_err(|_| {
                ActionStateError::TerminalCapacity("pending charter transaction base differs")
            })?;
            terminal_capacity::charter_pending_change(self, commit, terminal)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    pub(crate) fn house_mind_purchase_completion(
        &mut self,
        base: &ActionReadBase,
        provider: &ProviderRegistrationOrdinal,
        commit: crate::CommitToken,
        terminal: Option<crate::CommitToken>,
    ) -> Result<(), ActionStateError> {
        let result = (|| {
            self.read_preimage(base, provider).map_err(|_| {
                ActionStateError::TerminalCapacity("house transaction base differs")
            })?;
            terminal_capacity::house_change(self, commit, terminal)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    pub(crate) fn require_ic_owner_images(
        &self,
        images: &crate::participant::OwnerImages,
    ) -> Result<(), crate::participant::ParticipantError> {
        if let (Some(protocol), Some(identity)) = (
            &self.store.registry.ic_mind_claim,
            &self.registered_identity,
        ) {
            protocol.require_owner_images(identity, images)?;
        }
        Ok(())
    }

    pub(crate) fn core_integration_completion(
        &mut self,
        base: &ActionReadBase,
        provider: &ProviderRegistrationOrdinal,
        commit: crate::CommitToken,
        order: u64,
        admission: bool,
    ) -> Result<(), ActionStateError> {
        let result = (|| {
            self.read_preimage(base, provider).map_err(|_| {
                ActionStateError::TerminalCapacity("core integration transaction base differs")
            })?;
            terminal_capacity::core_integration_change(self, commit, order, admission)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    pub(crate) fn ic_mind_claim_completion(
        &mut self,
        base: &ActionReadBase,
        provider: &ProviderRegistrationOrdinal,
        commit: crate::CommitToken,
        open: Option<crate::CommitToken>,
    ) -> Result<(), ActionStateError> {
        let result = (|| {
            self.read_preimage(base, provider).map_err(|_| {
                ActionStateError::TerminalCapacity("IC claim transaction base differs")
            })?;
            terminal_capacity::ic_claim_change(self, commit, open)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    pub(crate) fn mind_renewal_completion(
        &mut self,
        base: &ActionReadBase,
        provider: &ProviderRegistrationOrdinal,
        commit: crate::CommitToken,
        operation: Option<crate::CommitToken>,
    ) -> Result<(), ActionStateError> {
        let result = (|| {
            self.read_preimage(base, provider).map_err(|_| {
                ActionStateError::TerminalCapacity("renewal transaction base differs")
            })?;
            terminal_capacity::renewal_change(self, commit, operation)
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
    }

    pub(crate) fn read_base(&self) -> ActionReadBase {
        ActionReadBase {
            slot: Arc::clone(&self.store.slot),
            revision: self.store.revision,
        }
    }

    /// A temporary committed-only lens. Staged mutations are deliberately not
    /// part of this snapshot; its borrow cannot outlive the transaction borrow.
    pub(crate) fn read_preimage(
        &self,
        base: &ActionReadBase,
        provider: &ProviderRegistrationOrdinal,
    ) -> Result<TypedStateLens<'_>, crate::read_service::OwnerReadError> {
        if self.poison.is_some()
            || !Arc::ptr_eq(&base.slot, &self.store.slot)
            || base.revision != self.store.revision
            || self.owner != *provider.owner()
            || self.provider_ordinal != Some(provider.get())
            || !self
                .store
                .registry
                .shares_readiness_with(provider.owner_readiness())
        {
            return Err(crate::read_service::OwnerReadError::Authority);
        }
        Ok(TypedStateLens::Live(&*self.store))
    }

    pub(crate) fn poison_owner_read(&mut self) {
        self.poison
            .get_or_insert_with(|| "owner pre-image read refused".into());
    }

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

    /// Check that an exact registered family has neither committed records nor
    /// staged mutations. This is a bounded bootstrap precondition, not a scan
    /// or a way to erase existing rows and then describe them as uninitialized.
    /// The transaction's existing global revision fence guards this read.
    pub fn family_is_empty<K, V>(
        &mut self,
        handle: &StateFamilyHandle<K, V>,
    ) -> Result<bool, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let result = (|| {
            self.store.registry.get_for_handle(handle)?;
            if handle.owner() != &self.owner
                && !self.store.registry.delegates_family(
                    handle.owner(),
                    &handle.descriptor().id,
                    &self.owner,
                )
            {
                return Err(ActionStateError::OwnerMismatch {
                    transaction: self.owner.clone(),
                    family: handle.owner().clone(),
                });
            }
            let first = StateKey {
                owner: handle.owner().clone(),
                family: handle.descriptor().id.clone(),
                encoded: SmolStr::new(""),
            };
            let same = |key: &StateKey| {
                key.owner() == handle.owner() && key.family() == &handle.descriptor().id
            };
            Ok(!self
                .store
                .records
                .range(first.clone()..)
                .next()
                .is_some_and(|(key, _)| same(key))
                && !self
                    .mutations
                    .range(first..)
                    .next()
                    .is_some_and(|(key, _)| same(key)))
        })();
        if let Err(error) = &result {
            self.poison(error);
        }
        result
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
        // E7c3 B1: a family this transaction's owner does not own is writable
        // only when the family's **own** owner delegated it at composition
        // (exact-family or legacy owner-wide grant). The persisted namespace and
        // the replay write stay the family owner's either way
        // (`PreparedTxn::authoritative_replay_writes` keys each write by
        // `record.key.owner`), so a delegated write is one bundle carrying two
        // owners' post-images, never a namespace transplant.
        if handle.owner() != &self.owner
            && !self.store.registry.delegates_family(
                handle.owner(),
                &handle.descriptor().id,
                &self.owner,
            )
        {
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
        // A live action must remain bootable under the same global record
        // and byte limits. Charge exact final canonical images, across every
        // owner/family; replacements and deletes release their own old bytes.
        let mut live_bytes = self.store.live_canonical_bytes;
        let mut live_records = self.store.records.len();
        for (key, mutation) in &self.mutations {
            if let Some(previous) = self.store.records.get(key) {
                live_bytes = live_bytes
                    .checked_sub(previous.canonical.len())
                    .ok_or(ActionStateError::ArithmeticOverflow)?;
                live_records = live_records
                    .checked_sub(1)
                    .ok_or(ActionStateError::ArithmeticOverflow)?;
            }
            if let TxnMutation::Put { canonical, .. } = mutation {
                live_bytes = live_bytes
                    .checked_add(canonical.len())
                    .ok_or(ActionStateError::ArithmeticOverflow)?;
                live_records = live_records
                    .checked_add(1)
                    .ok_or(ActionStateError::ArithmeticOverflow)?;
            }
        }
        if live_records > MAX_ACTION_STATE_BOOT_RECORDS {
            return Err(ActionStateError::LiveRecordLimit {
                attempted: live_records,
                max: MAX_ACTION_STATE_BOOT_RECORDS,
            });
        }
        if live_bytes > MAX_ACTION_STATE_BOOT_BYTES {
            return Err(ActionStateError::LiveBytesExceeded {
                attempted: live_bytes,
                max: MAX_ACTION_STATE_BOOT_BYTES,
            });
        }
        let capacity = terminal_capacity::prepare(
            self.store,
            &self.owner,
            &self.mutations,
            self.capacity_changes,
            self.registered_identity.as_ref(),
            live_bytes,
            live_records,
        )?;
        let operation_count = self
            .operation_count
            .checked_add(capacity.writes.len())
            .ok_or(ActionStateError::ArithmeticOverflow)?;
        let canonical_bytes = self
            .canonical_bytes
            .checked_add(capacity.write_bytes)
            .ok_or(ActionStateError::ArithmeticOverflow)?;
        if operation_count > self.limits.operations {
            return Err(ActionStateError::OperationLimit {
                attempted: operation_count,
                max: self.limits.operations,
            });
        }
        if canonical_bytes > self.limits.canonical_bytes {
            return Err(ActionStateError::BundleBytesExceeded {
                attempted: canonical_bytes,
                max: self.limits.canonical_bytes,
            });
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
        let owner = self.owner;
        let provider_ordinal = self.provider_ordinal;
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
        let capacity_replay = if capacity.writes.is_empty() {
            None
        } else {
            let mut writes = typed_replay_writes(&records);
            writes.extend(capacity.writes.iter().map(|write| write.replay.clone()));
            crate::authoritative::canonicalize_replay_writes(&mut writes, false)
                .map_err(|_| ActionStateError::TerminalCapacity("physical write union"))?;
            Some(writes)
        };
        self.store.next_reservation = reservation;
        self.store
            .slot
            .pending
            .store(reservation, Ordering::Relaxed);
        Ok(PreparedTxn {
            owner,
            provider_ordinal,
            operation_count,
            canonical_bytes,
            base_revision,
            new_revision,
            live_bytes,
            capacity,
            capacity_replay,
            guard: ReservationGuard::arm(Arc::clone(&self.store.slot), reservation),
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
///
/// It owns the store's commit reservation through `ReservationGuard`, so a
/// coordinator that gives up between `prepare` and staging hands the slot back
/// on the way out instead of wedging every later transaction with
/// [`ActionStateError::CommitPending`] (core-action-8).
#[must_use = "a prepared transaction must be staged for persistence or explicitly aborted"]
pub struct PreparedTxn {
    owner: OwnerId,
    provider_ordinal: Option<u32>,
    operation_count: usize,
    canonical_bytes: usize,
    base_revision: u64,
    new_revision: u64,
    live_bytes: usize,
    capacity: terminal_capacity::PreparedCapacity,
    capacity_replay: Option<Vec<ReplayWrite>>,
    guard: ReservationGuard,
    records: Vec<PreparedRecord>,
}

impl fmt::Debug for PreparedTxn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedTxn")
            .field("owner", &self.owner)
            .field("provider_ordinal", &self.provider_ordinal)
            .field("operation_count", &self.operation_count)
            .field("canonical_bytes", &self.canonical_bytes)
            .field("reservation", &self.guard.reservation)
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
        self.capacity_replay
            .clone()
            .unwrap_or_else(|| typed_replay_writes(&self.records))
    }

    /// Split store writes from the non-cloneable install images. Both were
    /// derived from the same validated typed values before this call.
    pub fn into_parts(self) -> (Vec<ModuleOp>, PreparedInstallation) {
        let PreparedTxn {
            base_revision,
            new_revision,
            live_bytes,
            capacity,
            mut guard,
            records,
            ..
        } = self;
        let mut module_ops = Vec::with_capacity(records.len());
        let mut mutations = BTreeMap::new();
        for record in records {
            module_ops.push(record.module_op);
            let previous = mutations.insert(record.key, record.installation);
            debug_assert!(previous.is_none());
        }
        module_ops.extend(capacity.writes.into_iter().map(|write| write.module_op));
        // The module ops are about to leave for the store, so from here a
        // durable write may already be in flight: the guard stops releasing on
        // drop and the reservation stays held, fail-closed, until an install or
        // an abort settles it.
        guard.stage = ReservationStage::Installation;
        (
            module_ops,
            PreparedInstallation {
                base_revision,
                new_revision,
                live_bytes,
                capacity: capacity.state,
                capacity_usage: capacity.usage,
                guard,
                mutations,
            },
        )
    }
}

fn typed_replay_writes(records: &[PreparedRecord]) -> Vec<ReplayWrite> {
    records
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

enum InstallMutation {
    Put {
        value: Box<dyn ErasedStateValue>,
        canonical: CanonicalStateBytes,
    },
    Delete,
}

/// Private typed RAM post-images held across the exact persistence receipt.
/// It is deliberately non-cloneable and has no public field access.
///
/// Unlike [`PreparedTxn`], it may not be dropped its way out of trouble: its
/// module ops already left for the store, so dying here loses the RAM half of
/// an ack-gated write-through commit. Dropping it therefore poisons the
/// store's reservation and counts a loss
/// ([`ActionStateStore::lost_installations`]) instead of freeing the slot; the
/// world task reads that counter and fails stop (core-action-8).
#[must_use = "a prepared installation must be installed after exact acknowledgement or aborted"]
pub struct PreparedInstallation {
    base_revision: u64,
    new_revision: u64,
    live_bytes: usize,
    capacity: Arc<terminal_capacity::CapacityState>,
    capacity_usage: terminal_capacity::Usage,
    guard: ReservationGuard,
    mutations: BTreeMap<StateKey, InstallMutation>,
}

impl fmt::Debug for PreparedInstallation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedInstallation")
            .field("reservation", &self.guard.reservation)
            .field("base_revision", &self.base_revision)
            .field("new_revision", &self.new_revision)
            .field("mutations", &self.mutations.len())
            .finish_non_exhaustive()
    }
}

impl PreparedInstallation {
    /// The typed records this installation will touch, grouped by owner and
    /// family in key order — what
    /// [`Engine::install_prepared_action_announcing`](crate::Engine::install_prepared_action_announcing)
    /// turns into one
    /// [`WorldEvent::ActionStateChanged`](crate::WorldEvent::ActionStateChanged)
    /// per family once the images are live (interface plan I13). Encoded keys
    /// only: the owning module decodes its own codec, and no value crosses.
    #[must_use]
    pub fn touched_families(&self) -> Vec<(OwnerId, StateFamilyId, Vec<SmolStr>)> {
        let mut out: Vec<(OwnerId, StateFamilyId, Vec<SmolStr>)> = Vec::new();
        for key in self.mutations.keys() {
            match out.last_mut() {
                Some((owner, family, keys)) if owner == key.owner() && family == key.family() => {
                    keys.push(SmolStr::new(key.encoded()));
                }
                _ => out.push((
                    key.owner().clone(),
                    key.family().clone(),
                    vec![SmolStr::new(key.encoded())],
                )),
            }
        }
        out
    }
}

/// Checked action-state refusal. Every variant occurs before live mutation.
#[derive(Debug, Error)]
pub enum ActionStateError {
    #[error("terminal capacity refused: {0}")]
    TerminalCapacity(&'static str),
    #[error("live typed records {attempted} exceed the global recoverable limit {max}")]
    LiveRecordLimit { attempted: usize, max: usize },
    #[error("live typed bytes {attempted} exceed the global recoverable limit {max}")]
    LiveBytesExceeded { attempted: usize, max: usize },
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
    /// A prepared installation reached a store it was not minted from. Only a
    /// wiring bug produces this; it used to be an `assert!` on the single
    /// world-owning task.
    #[error("prepared installation belongs to a different action state store")]
    ForeignStoreIdentity,
    /// The installation names a reservation this store does not currently
    /// hold — a double install, or an installation applied out of order.
    #[error("prepared installation owns reservation {expected}, but the store holds {found:?}")]
    ReservationMismatch { expected: u64, found: Option<u64> },
    /// Live state moved underneath a pending commit: the post-images were
    /// derived from a revision the store has already left.
    #[error(
        "prepared installation was prepared against revision {expected}; the store is at {found}"
    )]
    BaseRevisionMoved { expected: u64, found: u64 },
    /// A prepared value died without ever reaching `install` or `abort`, in a
    /// place the store cannot make safe: after
    /// [`PreparedTxn::into_parts`] handed the durable half out, or off the
    /// world task that armed the reservation. The reservation stays held, so
    /// this is terminal for the store rather than a retryable refusal — the
    /// world task turns it into a fail-stop, not a per-command error.
    #[error(
        "action state store lost prepared reservation {reservation}; its acknowledged write may \
         never have been installed, so RAM and the store may have diverged"
    )]
    InstallationLost { reservation: u64 },
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

    /// A module that owns no family and exists only to be a **grantee** of one:
    /// the delegation is declared by the *owning* module, never by this one.
    struct GranteeModule {
        owner: &'static str,
    }

    impl GameModule for GranteeModule {
        fn name(&self) -> &'static str {
            self.owner
        }

        fn register(&mut self, _registrar: &mut Registrar<'_>) -> Result<(), ModError> {
            Ok(())
        }
    }

    /// A `CounterModule` that delegates its families to one named grantee.
    struct DelegatingCounterModule {
        inner: CounterModule,
        grantee: &'static str,
    }

    impl GameModule for DelegatingCounterModule {
        fn name(&self) -> &'static str {
            self.inner.owner
        }

        fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), ModError> {
            registrar.delegate_state_families(self.grantee)?;
            self.inner.register(registrar)
        }
    }

    fn delegated_fixture(owner: &'static str, grantee: &'static str) -> (Engine, CounterHandle) {
        let captured = Arc::new(Mutex::new(None));
        let module = DelegatingCounterModule {
            inner: CounterModule {
                owner,
                captured: Arc::clone(&captured),
                serializer_calls: Arc::new(AtomicUsize::new(0)),
            },
            grantee,
        };
        let engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(module), Box::new(GranteeModule { owner: grantee })],
        )
        .expect("the delegating pair composes");
        let handle = captured
            .lock()
            .expect("capture mutex poisoned")
            .clone()
            .expect("registration captured handle");
        (engine, handle)
    }

    /// E7c3 B1: the cross-owner write E7 cut 2's handoff named as cut 3's.
    /// A **declared** delegation lets one grantee's transaction write the
    /// owner's family, and the write still persists under the FAMILY owner's
    /// namespace — so one bundle carries two owners' post-images and no bundle
    /// format changed.
    #[test]
    fn a_declared_delegation_admits_one_grantee_and_keeps_the_family_owners_namespace() {
        let (mut engine, handle) = delegated_fixture("state_test", "other_state");
        let families = engine
            .registry
            .state_families()
            .expect("the composed registry");
        let owner = OwnerId::new("state_test").expect("owner");
        let grantee = OwnerId::new("other_state").expect("grantee");
        assert!(families.delegates(&owner, &grantee));
        // Never symmetric, and never implicit for anybody else.
        assert!(!families.delegates(&grantee, &owner));
        assert!(!families.delegates(&owner, &OwnerId::new("nobody").expect("owner")));
        assert_eq!(families.delegations(), vec![(owner, grantee.clone())]);

        // The *write* half is proved where a real grantee exists — E7 cut 3's
        // `e7c3_d_the_bundle_carries_two_owners_post_images` prepares an
        // extraction bundle whose replay writes carry exactly the two
        // namespaces `{industry, inventory}`, and `mush-server`'s
        // `e7c3_wiring_the_quantity_delegation_is_declared_and_narrow` pins that
        // the production composition holds exactly this one pair. What this
        // test owns is the registry rule itself.
        let _ = &handle;
        let _ = &mut engine;
    }

    /// And an undeclared one still refuses: the fence is widened only by the
    /// owner's own composition-time statement.
    #[test]
    fn an_undeclared_grantee_still_cannot_write_another_owners_family() {
        let (mut first, first_handle, _) = fixture("state_test");
        let (_second, second_handle, _) = fixture("other_state");
        let mut txn = first.action_transaction(&first_handle).unwrap();
        assert!(txn.get(&second_handle, &1).is_err());
        assert!(
            matches!(
                txn.prepare(),
                Err(ActionStateError::TransactionPoisoned { .. })
            ),
            "an undelegated cross-owner attempt still poisons the transaction"
        );
    }

    /// The three refusals the delegation itself makes.
    #[test]
    fn a_delegation_refuses_itself_an_uncomposed_grantee_and_its_own_cap() {
        let mut builder = StateFamilyRegistryBuilder::new();
        let owner = OwnerId::new("state_test").expect("owner");
        assert!(matches!(
            builder.delegate(owner.clone(), owner.clone()),
            Err(StateFamilyRegistryError::SelfDelegation { .. })
        ));
        for index in 0..MAX_STATE_FAMILY_DELEGATES {
            let grantee = OwnerId::new(format!("grantee{index}")).expect("grantee");
            builder
                .delegate(owner.clone(), grantee)
                .expect("inside the cap");
        }
        assert!(matches!(
            builder.delegate(owner.clone(), OwnerId::new("onemore").expect("grantee")),
            Err(StateFamilyRegistryError::OwnerDelegateLimit { .. })
        ));
        // An uncomposed grantee is refused at freeze, not silently ignored.
        let mut builder = StateFamilyRegistryBuilder::new();
        builder
            .delegate(owner.clone(), OwnerId::new("ghost").expect("grantee"))
            .expect("declared");
        let owners = BTreeSet::from([owner]);
        let readiness = MechanicalOwnerReadiness::for_owners(&owners);
        assert!(matches!(
            builder.freeze_for_owners_with_readiness(&owners, readiness),
            Err(StateFamilyRegistryError::UnknownOwner { .. })
        ));
    }

    fn install_direct(engine: &mut Engine, prepared: PreparedTxn) -> Vec<ModuleOp> {
        let (ops, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
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

    /// One complete K3 bundle around an already-prepared transaction, so a
    /// staging refusal can be provoked at the lane's only entry point.
    fn state_test_bundle(prepared: &PreparedTxn, owner: &str) -> crate::AuthoritativeBundle {
        let owner = crate::OwnerId::new(owner).expect("static owner id");
        let mut definitions = crate::DefinitionRegistryBuilder::new();
        let action = definitions
            .register(
                owner.clone(),
                crate::DefinitionDescriptor::new(
                    crate::DefinitionKind::new("action").unwrap(),
                    crate::DefinitionId::new("bump_counter").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let outcome_definition = definitions
            .register(
                owner.clone(),
                crate::DefinitionDescriptor::new(
                    crate::DefinitionKind::new("outcome").unwrap(),
                    crate::DefinitionId::new("counter_bumped").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let request = crate::RequestUid::from_bytes([0x11; 16]);
        let principal = crate::PrincipalId::Object { id: crate::GOD };
        let cause = crate::PreparedCausalRef::new(request, None, 0).unwrap();
        crate::AuthoritativeBundle::new(crate::AuthoritativeBundleDraft {
            token: crate::CommitToken::from_bytes([0x12; 16]),
            request,
            idempotency: crate::ScopedIdempotencyKey::new(
                crate::IdempotencyScope {
                    principal: principal.clone(),
                    owner: owner.clone(),
                    namespace: CanonicalId::new("bump_counter").unwrap(),
                },
                0,
                crate::IdempotencyKey::Request(request),
            ),
            ingress_hash: crate::CanonicalHash::from_bytes([0x13; 32]),
            request_hash: crate::CanonicalHash::from_bytes([0x14; 32]),
            plan_hash: crate::CanonicalHash::from_bytes([0x15; 32]),
            action: action.clone(),
            provider: owner,
            cause,
            definitions: vec![action.clone(), outcome_definition.clone()],
            writes: prepared.authoritative_replay_writes(),
            outcome: crate::RulesValue::new(
                outcome_definition.clone(),
                serde_json::json!({ "accepted": true }),
            )
            .unwrap(),
            resolution: None,
            semantic_events: vec![
                crate::AuthoritativeSemanticEvent::root(
                    cause,
                    &action,
                    crate::RulesValue::new(
                        outcome_definition,
                        serde_json::json!({ "accepted": true }),
                    )
                    .unwrap(),
                    vec![principal.clone()],
                )
                .unwrap(),
            ],
            eligible_witnesses: vec![principal],
            durable_outbox: vec![],
        })
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
    fn family_empty_bootstrap_guard_rejects_staged_or_live_rows_even_when_deleted() {
        let (mut engine, handle, _) = fixture("state_test");
        let mut txn = engine.action_transaction(&handle).unwrap();
        assert!(txn.family_is_empty(&handle).unwrap());
        txn.insert(&handle, &1, CounterState { amount: 1 }).unwrap();
        assert!(!txn.family_is_empty(&handle).unwrap());
        let prepared = txn.prepare().unwrap();
        install_direct(&mut engine, prepared);
        let mut txn = engine.action_transaction(&handle).unwrap();
        assert!(!txn.family_is_empty(&handle).unwrap());
        txn.remove(&handle, &1).unwrap();
        assert!(
            !txn.family_is_empty(&handle).unwrap(),
            "deleting history is not first initialization"
        );
        drop(txn);
        assert_eq!(
            engine.action_state().get(&handle, &1).unwrap(),
            Some(&CounterState { amount: 1 })
        );
    }

    #[test]
    fn family_empty_refuses_foreign_registry_handle_and_poison_cannot_be_caught() {
        let (mut engine, handle, _) = fixture("state_test");
        let (_, foreign, _) = fixture("state_test");
        let mut txn = engine.action_transaction(&handle).unwrap();
        assert!(txn.family_is_empty(&foreign).is_err());
        assert!(txn.prepare().is_err());
        assert!(engine.action_state().is_empty());
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
        engine
            .abort_prepared_action(installation)
            .expect("the fixture aborts");
        assert!(!engine.action_state().has_prepared());
        assert!(engine.action_state().get(&handle, &7).unwrap().is_none());
    }

    #[test]
    fn live_byte_accounting_tracks_actual_replace_delete_and_hydration_images() {
        let (mut store, handle) = bare_store();
        let mut txn = store.transaction(&handle).unwrap();
        txn.insert(&handle, &7, CounterState { amount: 1 }).unwrap();
        let (first_ops, first) = txn.prepare().unwrap().into_parts();
        assert_eq!(store.live_canonical_bytes, 0, "prepare is not an install");
        store.install(first).unwrap();
        let first_bytes = store.canonical(&handle, &7).unwrap().unwrap().len();
        assert_eq!(store.live_canonical_bytes, first_bytes);

        let mut txn = store.transaction(&handle).unwrap();
        txn.replace(&handle, &7, CounterState { amount: 65_535 })
            .unwrap();
        let (replacement_ops, replacement) = txn.prepare().unwrap().into_parts();
        store.install(replacement).unwrap();
        let replacement_bytes = store.canonical(&handle, &7).unwrap().unwrap().len();
        assert!(replacement_bytes > first_bytes);
        assert_eq!(store.live_canonical_bytes, replacement_bytes);

        let (mut restarted, restarted_handle) = bare_store();
        restarted.hydrate(boot_records(replacement_ops)).unwrap();
        assert_eq!(restarted.live_canonical_bytes, replacement_bytes);
        assert_eq!(
            restarted.get(&restarted_handle, &7).unwrap(),
            store.get(&handle, &7).unwrap()
        );
        let (mut original, _) = bare_store();
        original.hydrate(boot_records(first_ops)).unwrap();
        assert_eq!(original.live_canonical_bytes, first_bytes);

        let mut txn = store.transaction(&handle).unwrap();
        txn.remove(&handle, &7).unwrap();
        let (_, deletion) = txn.prepare().unwrap().into_parts();
        store.install(deletion).unwrap();
        assert_eq!(store.live_canonical_bytes, 0);
        assert!(store.is_empty());
    }

    #[test]
    fn live_byte_limit_refuses_before_reservation_and_replacement_reuses_existing_charge() {
        let (mut store, handle) = bare_store();
        let seeded = prepare_counter(&mut store, &handle, 1, 1);
        store.install(seeded).unwrap();
        // Exercise the exact aggregate boundary without allocating 512 MiB.
        // The preceding test proves this counter from real installed/replayed
        // bytes; here unseen unchanged families account for the remaining charge.
        store.live_canonical_bytes = MAX_ACTION_STATE_BOOT_BYTES;
        let before = live_state(&store);
        let mut txn = store.transaction(&handle).unwrap();
        txn.insert(&handle, &2, CounterState { amount: 1 }).unwrap();
        assert!(matches!(
            txn.prepare(),
            Err(ActionStateError::LiveBytesExceeded { .. })
        ));
        assert_eq!(live_state(&store), before);
        assert!(!store.has_prepared());

        let mut txn = store.transaction(&handle).unwrap();
        txn.replace(&handle, &1, CounterState { amount: 2 })
            .unwrap();
        let (_, equal_size) = txn.prepare().unwrap().into_parts();
        store.install(equal_size).unwrap();
        assert_eq!(store.live_canonical_bytes, MAX_ACTION_STATE_BOOT_BYTES);
        let removed = store.canonical(&handle, &1).unwrap().unwrap().len();
        let mut txn = store.transaction(&handle).unwrap();
        txn.remove(&handle, &1).unwrap();
        let (_, deletion) = txn.prepare().unwrap().into_parts();
        store.install(deletion).unwrap();
        assert_eq!(
            store.live_canonical_bytes,
            MAX_ACTION_STATE_BOOT_BYTES - removed
        );
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
        first
            .abort_prepared_action(first_installation)
            .expect("the fixture aborts");
        let (_, second_installation) = second_prepared.into_parts();
        second
            .abort_prepared_action(second_installation)
            .expect("the fixture aborts");
    }

    #[test]
    fn staging_refuses_legacy_dirty_world_and_clears_reservation() {
        let (mut engine, handle, _) = fixture("state_test");
        let mut txn = engine.action_transaction(&handle).unwrap();
        txn.insert(&handle, &1, CounterState { amount: 1 }).unwrap();
        let prepared = txn.prepare().unwrap();
        let guards = empty_guards(&engine);

        let bundle = state_test_bundle(&prepared, "state_test");

        assert!(
            engine.world.has_dirty(),
            "the fixture's seeded world is the legacy dirtiness under test"
        );
        assert_eq!(
            engine.stage_authoritative_action(prepared, guards, bundle),
            Err(crate::StagePreparedActionError::DirtyWorld)
        );
        assert!(!engine.action_state().has_prepared());
        assert!(engine.action_state().is_empty());
    }
    /// A store with one registered family and no engine around it, so a test
    /// can move the store's own pre-state under a pending commit.
    fn bare_store() -> (ActionStateStore, CounterHandle) {
        let owner = OwnerId::new("state_test").expect("static owner id");
        let owners = BTreeSet::from([owner.clone()]);
        let readiness = MechanicalOwnerReadiness::for_owners(&owners);
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new("counter").expect("static family id"),
            CanonicalId::new("u64.decimal.v1").expect("static codec id"),
            CanonicalId::new("counter.v1").expect("static schema id"),
            1,
            CanonicalId::new("counter.decode_json.v1").expect("static decoder id"),
            CanonicalId::new("counter.range.v1").expect("static validator id"),
            CanonicalId::new("counter.json.v1").expect("static serializer id"),
        )
        .expect("static state-family descriptor");
        let mut builder = StateFamilyRegistryBuilder::new();
        let handle = builder
            .register::<u64, CounterState, _, _, _, _>(
                owner,
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
                |_value: &CounterState| Ok(()),
                |value: &CounterState| Ok(json!({ "amount": value.amount })),
            )
            .expect("the counter family registers");
        let registry = builder
            .freeze_for_owners_with_readiness(&owners, readiness)
            .expect("single-owner registry is canonical");
        (ActionStateStore::new(registry), handle)
    }

    /// Everything a refused installation must leave untouched: the revision,
    /// the outstanding reservation, and the live records byte for byte.
    type LiveState = (u64, Option<u64>, Vec<(StateKey, Vec<u8>)>);

    fn live_state(store: &ActionStateStore) -> LiveState {
        (
            store.revision,
            store.pending_reservation(),
            store
                .records
                .iter()
                .map(|(key, record)| (key.clone(), record.canonical.as_slice().to_vec()))
                .collect(),
        )
    }

    fn prepare_counter(
        store: &mut ActionStateStore,
        handle: &CounterHandle,
        key: u64,
        amount: u16,
    ) -> PreparedInstallation {
        let mut txn = store.transaction(handle).expect("the store is unreserved");
        txn.insert(handle, &key, CounterState { amount })
            .expect("the counter value is valid");
        txn.prepare()
            .expect("the transaction prepares")
            .into_parts()
            .1
    }

    #[test]
    fn bound_typed_install_keeps_the_reservation_until_consuming_completion() {
        let (mut store, handle) = bare_store();
        let installation = prepare_counter(&mut store, &handle, 9, 3);
        let before = live_state(&store);
        let bound = store
            .bind_installation(installation)
            .expect("exact binding");
        assert_eq!(live_state(bound.store), before, "bind changes no image");

        let completion = bound.apply_images();
        assert_eq!(completion._store.revision(), 1);
        assert_eq!(
            completion._store.get(&handle, &9).expect("record"),
            Some(&CounterState { amount: 3 })
        );
        assert_eq!(completion._store.pending_reservation(), Some(1));
        assert_eq!(completion._store.lost_installations(), 0);
        assert!(!completion._store.reservation_lost());
        completion.complete();

        assert_eq!(store.pending_reservation(), None);
        assert_eq!(store.lost_installations(), 0);
        assert!(!store.reservation_lost());
        let next = prepare_counter(&mut store, &handle, 10, 2);
        store
            .install(next)
            .expect("ordinary install still completes");
        assert_eq!(store.revision(), 2);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn dropping_bound_typed_install_preserves_images_and_latches_installation_loss() {
        let (mut store, handle) = bare_store();
        let seeded = prepare_counter(&mut store, &handle, 1, 1);
        store.install(seeded).expect("seed");
        let installation = prepare_counter(&mut store, &handle, 2, 2);
        let before = live_state(&store);
        let bound = store
            .bind_installation(installation)
            .expect("exact binding");
        // A later owner bind could refuse here. Dropping the earlier bound
        // value must not apply it or release an acknowledged reservation.
        drop(bound);
        assert_eq!(live_state(&store), before);
        assert!(store.reservation_lost());
        assert_eq!(store.lost_installations(), 1);
        assert_eq!(store.dropped_reservations(), 0);
        assert!(matches!(
            store.transaction(&handle).err(),
            Some(ActionStateError::InstallationLost { reservation: 2 })
        ));
    }

    #[test]
    fn dropping_unsettled_typed_images_never_reopens_the_store() {
        let (mut store, handle) = bare_store();
        let installation = prepare_counter(&mut store, &handle, 9, 3);
        let completion = store
            .bind_installation(installation)
            .expect("exact binding")
            .apply_images();
        assert_eq!(completion._store.revision(), 1);
        assert_eq!(
            completion._store.get(&handle, &9).expect("record"),
            Some(&CounterState { amount: 3 })
        );
        let images = live_state(completion._store);
        // Images already changed, so this is fail-stop, not rollback. The
        // existing installation guard must remain armed across this interval.
        drop(completion);
        assert_eq!(live_state(&store), images);
        assert_eq!(store.pending_reservation(), Some(1));
        assert!(store.reservation_lost());
        assert_eq!(store.lost_installations(), 1);
        assert_eq!(store.dropped_reservations(), 0);
        assert!(matches!(
            store.transaction(&handle).err(),
            Some(ActionStateError::InstallationLost { reservation: 1 })
        ));
    }

    /// core-action-7 (a): an installation minted against one store and applied
    /// to another used to `assert!` on `Arc::ptr_eq`. The world task is the
    /// single writer and may never panic (docs/PLAN.md), so the mismatch is a
    /// typed refusal now — and the target store is byte-identical afterwards.
    #[test]
    fn an_installation_from_a_foreign_store_is_refused_not_asserted() {
        let (mut first, first_handle) = bare_store();
        let (mut second, second_handle) = bare_store();

        // Give the target store live state worth clobbering.
        let seeded = prepare_counter(&mut second, &second_handle, 9, 1);
        second.install(seeded).expect("the store installs its own");
        let before = live_state(&second);

        let foreign = prepare_counter(&mut first, &first_handle, 9, 3);
        assert!(matches!(
            second.install(foreign),
            Err(ActionStateError::ForeignStoreIdentity)
        ));
        assert_eq!(live_state(&second), before);
    }

    /// The abort half refuses on the same proof, and leaves the target store's
    /// own outstanding reservation alone rather than clearing it.
    #[test]
    fn aborting_a_foreign_installation_does_not_clear_the_local_reservation() {
        let (mut first, first_handle) = bare_store();
        let (mut second, second_handle) = bare_store();
        let local = prepare_counter(&mut second, &second_handle, 9, 1);
        let before = live_state(&second);

        let foreign = prepare_counter(&mut first, &first_handle, 9, 3);
        assert!(matches!(
            second.abort(foreign),
            Err(ActionStateError::ForeignStoreIdentity)
        ));
        assert_eq!(live_state(&second), before);
        assert!(second.has_prepared());
        second
            .abort(local)
            .expect("the local reservation still releases");
        assert!(!second.has_prepared());
    }

    /// core-action-7 (b): the pre-state moved while the commit was pending.
    /// The post-images were derived from a revision the store has already
    /// left, so the install refuses and changes nothing on its way out.
    #[test]
    fn an_installation_whose_base_revision_moved_is_refused_not_asserted() {
        let (mut store, handle) = bare_store();
        let installation = prepare_counter(&mut store, &handle, 9, 1);
        // What the wiring bug looks like from the store's side: live state
        // advanced underneath a commit that was still in flight.
        store.revision = 4;
        let before = live_state(&store);

        assert!(matches!(
            store.install(installation),
            Err(ActionStateError::BaseRevisionMoved {
                expected: 0,
                found: 4
            })
        ));
        assert_eq!(live_state(&store), before);
    }

    /// core-action-7 (c): the second half of a double install. The first one
    /// cleared `pending_reservation`, so the replay owns nothing — and its
    /// delete never reaches `records`, which is the all-or-nothing ordering
    /// the check exists for. The replay is hand-built because
    /// `PreparedInstallation` deliberately cannot be cloned; a real double
    /// install delivers exactly this shape a second time.
    #[test]
    fn a_second_install_of_one_reservation_is_refused_not_asserted() {
        let (mut store, handle) = bare_store();
        let installation = prepare_counter(&mut store, &handle, 9, 1);
        let reservation = installation.guard.reservation;
        let base_revision = installation.base_revision;
        let new_revision = installation.new_revision;
        let slot = Arc::clone(&installation.guard.slot);
        store
            .install(installation)
            .expect("the first install lands");

        let live_key = store
            .records
            .keys()
            .next()
            .expect("the first install left one record")
            .clone();
        let mut guard = ReservationGuard::arm(slot, reservation);
        guard.stage = ReservationStage::Installation;
        let replay = PreparedInstallation {
            base_revision,
            new_revision,
            live_bytes: 0, // The forged replay would delete the sole record.
            capacity: Arc::default(),
            capacity_usage: terminal_capacity::Usage::default(),
            guard,
            mutations: BTreeMap::from([(live_key, InstallMutation::Delete)]),
        };
        let before = live_state(&store);
        assert!(matches!(
            store.install(replay),
            Err(ActionStateError::ReservationMismatch {
                expected: 1,
                found: None
            })
        ));
        assert_eq!(live_state(&store), before);
    }
    /// What a coordinator early return actually looks like: it prepares, then
    /// bails on something else before the transaction is ever staged, and the
    /// `PreparedTxn` dies on the stack.
    fn coordinator_early_return(
        store: &mut ActionStateStore,
        handle: &CounterHandle,
        key: u64,
    ) -> Result<(), ActionStateError> {
        let mut txn = store.transaction(handle)?;
        txn.insert(handle, &key, CounterState { amount: 7 })?;
        let _prepared = txn.prepare()?;
        // The give-up. `_prepared` is still alive here and is dropped on the
        // way out — the exact shape core-action-8 is about.
        Err(ActionStateError::EmptyTransaction)
    }

    /// core-action-8: `pending_reservation` used to be cleared only by
    /// `install` and `abort`, so a coordinator that dropped its `PreparedTxn`
    /// wedged the store — every later `begin_for_owner` refused with
    /// `CommitPending` for the life of the process. The guard's `Drop` hands
    /// the slot back.
    #[test]
    fn a_dropped_prepared_txn_releases_the_reservation() {
        let (mut store, handle) = bare_store();

        assert!(matches!(
            coordinator_early_return(&mut store, &handle, 9),
            Err(ActionStateError::EmptyTransaction)
        ));
        assert!(!store.has_prepared());

        // The wedge: this used to be Err(CommitPending { reservation: 1 }).
        let recovered = prepare_counter(&mut store, &handle, 9, 3);
        store
            .install(recovered)
            .expect("the store commits after the leak");
        assert_eq!(store.revision, 1);
        assert_eq!(store.len(), 1);
    }

    /// The release is bookkeeping only: nothing durable was staged for a
    /// `PreparedTxn`, so the post-images die with it and live state is exactly
    /// where the failed coordinator found it.
    #[test]
    fn a_dropped_prepared_txn_leaves_ram_and_revision_untouched() {
        let (mut store, handle) = bare_store();
        let seeded = prepare_counter(&mut store, &handle, 1, 1);
        store.install(seeded).expect("the seed installs");
        let before = live_state(&store);

        assert!(matches!(
            coordinator_early_return(&mut store, &handle, 2),
            Err(ActionStateError::EmptyTransaction)
        ));

        assert_eq!(live_state(&store), before);
        assert!(!store.reservation_lost());
        // The prepared post-image never reached the live map.
        assert_eq!(store.len(), 1);
    }

    /// A released reservation is still a bug, so it is counted rather than
    /// swallowed.
    #[test]
    fn the_drop_counter_reports_the_leak() {
        let (mut store, handle) = bare_store();
        assert_eq!(store.dropped_reservations(), 0);

        assert!(matches!(
            coordinator_early_return(&mut store, &handle, 4),
            Err(ActionStateError::EmptyTransaction)
        ));
        assert_eq!(store.dropped_reservations(), 1);

        // A clean commit is not a leak.
        let clean = prepare_counter(&mut store, &handle, 4, 2);
        store.install(clean).expect("the clean commit installs");
        assert_eq!(store.dropped_reservations(), 1);
    }

    /// `PreparedTxn` is `Send`, and a drop off the world task cannot be
    /// ordered against the store it would be releasing. So it poisons instead:
    /// the reservation stays held, the store stays deliberately wedged, and
    /// the leak counter does not move. WS03-S11 owns what the engine does with
    /// the `lost` flag.
    #[test]
    fn a_prepared_txn_dropped_off_the_world_task_poisons_instead_of_releasing() {
        let (mut store, handle) = bare_store();
        let prepared = {
            let mut txn = store.transaction(&handle).expect("the store is unreserved");
            txn.insert(&handle, &5, CounterState { amount: 1 })
                .expect("the counter value is valid");
            txn.prepare().expect("the transaction prepares")
        };

        std::thread::spawn(move || drop(prepared))
            .join()
            .expect("the dropping thread does not panic");

        assert!(store.reservation_lost());
        assert!(store.has_prepared());
        assert_eq!(store.dropped_reservations(), 0);
        // WS03-S2 left this as `CommitPending` and said WS03-S11 owned what
        // the engine does with the `lost` flag. It does this: the poison is
        // its own refusal, because "wait for the commit in flight" is a lie
        // when there is no commit left to wait for. The off-thread drop staged
        // nothing durable, so it is not counted as a lost installation.
        assert_eq!(store.lost_installations(), 0);
        assert!(matches!(
            store.transaction(&handle).err(),
            Some(ActionStateError::InstallationLost { reservation: 1 })
        ));
    }

    /// Regression for the identity half of the refactor: the store identity
    /// used to be its own `Arc<()>` and is now the reservation slot itself, so
    /// an installation minted elsewhere must still fail `Arc::ptr_eq` — and
    /// must not release the target store's own reservation on its way out.
    #[test]
    fn install_after_prepare_still_asserts_ownership() {
        let (mut first, first_handle) = bare_store();
        let (mut second, second_handle) = bare_store();
        let local = prepare_counter(&mut second, &second_handle, 3, 1);

        let foreign = prepare_counter(&mut first, &first_handle, 3, 2);
        assert!(matches!(
            second.install(foreign),
            Err(ActionStateError::ForeignStoreIdentity)
        ));
        assert_eq!(second.pending_reservation(), Some(1));
        assert_eq!(second.dropped_reservations(), 0);

        second.abort(local).expect("the local reservation releases");
        assert!(!second.has_prepared());
    }

    /// core-action-8, the installation half. `into_parts` hands the module ops
    /// to the store and keeps the typed RAM post-images, so from that moment a
    /// durable write may already be acknowledged. A `PreparedInstallation`
    /// dropped there has lost the RAM half of an ack-gated write-through
    /// commit: freeing the reservation would let the next transaction prepare
    /// against a revision the acknowledged write was never installed into, and
    /// RAM and Redis would diverge with nothing to say it happened. So the
    /// store fails stop instead — and the refusal is its own variant, because
    /// `CommitPending` would invite exactly the retry that must not happen.
    #[test]
    fn a_dropped_prepared_installation_fails_stop_instead_of_diverging() {
        let (mut store, handle) = bare_store();
        let seeded = prepare_counter(&mut store, &handle, 1, 1);
        store.install(seeded).expect("the seed installs");
        let (revision_before, _, records_before) = live_state(&store);

        let (module_ops, installation) = {
            let mut txn = store.transaction(&handle).expect("the store is unreserved");
            txn.insert(&handle, &2, CounterState { amount: 5 })
                .expect("the counter value is valid");
            txn.prepare()
                .expect("the transaction prepares")
                .into_parts()
        };
        // The durable half is out of the coordinator's hands; the RAM half
        // dies with the value.
        assert_eq!(module_ops.len(), 1);
        drop(installation);

        assert!(store.reservation_lost());
        assert_eq!(store.lost_installations(), 1);
        // Not Ok, and not the retryable wedge.
        assert!(matches!(
            store.transaction(&handle).err(),
            Some(ActionStateError::InstallationLost { reservation: 2 })
        ));
        // The post-image never went live and nothing rolled forward — but the
        // reservation is deliberately still held, so `live_state` is compared
        // without it.
        let (revision_after, pending_after, records_after) = live_state(&store);
        assert_eq!(revision_after, revision_before);
        assert_eq!(records_after, records_before);
        assert_eq!(pending_after, Some(2));
        assert_eq!(store.revision(), 1);
        assert_eq!(store.len(), 1);
    }

    /// The two drops are different failures and are counted separately: a
    /// dropped `PreparedTxn` staged nothing and hands its slot back, a dropped
    /// `PreparedInstallation` staged everything and poisons it.
    #[test]
    fn a_dropped_installation_is_distinguishable_from_a_dropped_txn() {
        let (mut leaked, leaked_handle) = bare_store();
        assert!(matches!(
            coordinator_early_return(&mut leaked, &leaked_handle, 7),
            Err(ActionStateError::EmptyTransaction)
        ));
        assert_eq!(leaked.dropped_reservations(), 1);
        assert_eq!(leaked.lost_installations(), 0);
        assert!(!leaked.reservation_lost());
        assert!(!leaked.has_prepared());

        let (mut lost, lost_handle) = bare_store();
        drop(prepare_counter(&mut lost, &lost_handle, 7, 1));
        assert_eq!(lost.dropped_reservations(), 0);
        assert_eq!(lost.lost_installations(), 1);
        assert!(lost.reservation_lost());
        assert!(lost.has_prepared());
    }

    /// The witness is only for the deaths nobody answered for. Both ordinary
    /// endings settle the slot, leave `lost` clear, and let the next
    /// transaction begin.
    #[test]
    fn install_and_abort_still_clear_the_slot() {
        let (mut store, handle) = bare_store();

        let installed = prepare_counter(&mut store, &handle, 3, 1);
        store.install(installed).expect("the store installs");
        assert!(!store.reservation_lost());
        assert_eq!(store.lost_installations(), 0);
        assert!(!store.has_prepared());

        let aborted = prepare_counter(&mut store, &handle, 4, 2);
        store.abort(aborted).expect("the store aborts");
        assert!(!store.reservation_lost());
        assert_eq!(store.lost_installations(), 0);
        assert!(!store.has_prepared());

        // The proof that neither ending poisoned anything: prepare still works.
        let again = prepare_counter(&mut store, &handle, 5, 3);
        store.install(again).expect("the store still commits");
        assert_eq!(store.revision(), 2);
        assert_eq!(store.len(), 2);
    }

    /// Doc 18 cut IC14 row 10: a module's read-only lens answers whether a
    /// prepared transaction owns the store — `true` while one is held,
    /// `false` once it is aborted, and `false` for an unavailable reader.
    #[test]
    fn ic14_commit_pending_reads_a_held_prepared_transaction() {
        let (mut store, handle) = bare_store();
        let lens = |store: &ActionStateStore| {
            ModuleActionState::for_module(TypedStateLens::Live(store), Some("state_test"))
                .commit_pending()
        };
        assert!(!lens(&store), "nothing prepared");
        let held = prepare_counter(&mut store, &handle, 9, 1);
        assert!(lens(&store), "a prepared transaction owns the store");
        store.abort(held).expect("the store aborts");
        assert!(!lens(&store), "released at abort");
        assert!(!ModuleActionState::unavailable().commit_pending());
    }
}

#[cfg(test)]
mod scoped_delegation_tests;
