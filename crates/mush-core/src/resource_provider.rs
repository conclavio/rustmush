//! Composition-frozen read adapters for ruleset-owned resources and effects.
//!
//! The kernel owns exact routing, definition binding, readiness checks, and
//! validation of returned coordinates. Providers expose existing owner truth
//! through immutable world and typed-state reads; mutation remains exclusively
//! behind registered action providers and authoritative bundles.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::action_state::{ActionStateError, ActionStateStore, StateFamilyHandle};
use crate::mechanics::{
    CanonicalEncodingError, CanonicalId, DefinitionDigest, DefinitionLookupError, DefinitionRef,
    DefinitionRegistry, MechanicalOwnerReadiness, MechanicalOwnerReadinessError, OwnerId,
};
use crate::resource::{BoundedScalar, EffectInstance, EffectInstanceId, EffectTarget, ResourceRef};
use crate::world::World;

/// Maximum resource and effect adapters in one deployment.
pub const MAX_RESOURCE_EFFECT_PROVIDERS: usize = 4_096;
/// Maximum resource and effect adapters owned by one module.
pub const MAX_RESOURCE_EFFECT_PROVIDERS_PER_OWNER: usize = 1_024;

/// Stable address of one resource adapter. The frozen entry supplies its
/// exact selected definition version.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceProviderKey {
    owner: OwnerId,
    id: CanonicalId,
}

impl ResourceProviderKey {
    #[must_use]
    pub const fn new(owner: OwnerId, id: CanonicalId) -> Self {
        Self { owner, id }
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn id(&self) -> &CanonicalId {
        &self.id
    }

    fn from_resource(resource: &ResourceRef) -> Self {
        Self::new(resource.owner.clone(), resource.id.clone())
    }
}

/// Immutable truth exposed during a synchronous resource/effect read.
#[derive(Clone, Copy)]
pub struct ResourceProviderContext<'a> {
    world: &'a World,
    action_state: &'a ActionStateStore,
}

impl fmt::Debug for ResourceProviderContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceProviderContext")
            .field("utc_now", &self.world.now())
            .field("action_state_revision", &self.action_state.revision())
            .finish_non_exhaustive()
    }
}

impl<'a> ResourceProviderContext<'a> {
    pub(crate) const fn new(world: &'a World, action_state: &'a ActionStateStore) -> Self {
        Self {
            world,
            action_state,
        }
    }

    /// Read immutable ordinary world truth. This deliberately does not expose
    /// a mutable world or a transaction.
    #[must_use]
    pub const fn world(&self) -> &'a World {
        self.world
    }

    /// Read immutable module state registered in the ordinary world context.
    #[must_use]
    pub fn module_state<T: Any + Send>(&self) -> Option<&'a T> {
        self.world.module_state::<T>()
    }

    /// Read one authoritative typed-family value through its
    /// composition-branded handle.
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

/// Pure synchronous adapter for one selected ruleset-owned resource.
pub trait ResourceProvider: Send + Sync {
    fn read(
        &self,
        context: &ResourceProviderContext<'_>,
        resource: &ResourceRef,
    ) -> Result<Option<BoundedScalar>, ResourceProviderFailure>;
}

impl<F> ResourceProvider for F
where
    F: for<'a> Fn(
            &ResourceProviderContext<'a>,
            &ResourceRef,
        ) -> Result<Option<BoundedScalar>, ResourceProviderFailure>
        + Send
        + Sync,
{
    fn read(
        &self,
        context: &ResourceProviderContext<'_>,
        resource: &ResourceRef,
    ) -> Result<Option<BoundedScalar>, ResourceProviderFailure> {
        self(context, resource)
    }
}

/// Pure synchronous adapter for instances of one exact effect definition.
pub trait EffectProvider: Send + Sync {
    fn read(
        &self,
        context: &ResourceProviderContext<'_>,
        target: &EffectTarget,
        instance: EffectInstanceId,
    ) -> Result<Option<EffectInstance>, ResourceProviderFailure>;
}

impl<F> EffectProvider for F
where
    F: for<'a> Fn(
            &ResourceProviderContext<'a>,
            &EffectTarget,
            EffectInstanceId,
        ) -> Result<Option<EffectInstance>, ResourceProviderFailure>
        + Send
        + Sync,
{
    fn read(
        &self,
        context: &ResourceProviderContext<'_>,
        target: &EffectTarget,
        instance: EffectInstanceId,
    ) -> Result<Option<EffectInstance>, ResourceProviderFailure> {
        self(context, target, instance)
    }
}

/// Bounded failure vocabulary for read adapters. Domain-specific refusal is
/// kept out of this interface because projected errors remain action policy.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ResourceProviderFailure {
    #[error("provider state is temporarily unavailable")]
    Unavailable,
    #[error("provider does not support the requested subject shape")]
    UnsupportedSubject,
    #[error("provider state failed an integrity check")]
    Integrity,
}

/// A kernel-labelled resource result. The provider returns only the scalar,
/// so it cannot substitute another resource or definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceSnapshot {
    definition: DefinitionRef,
    resource: ResourceRef,
    value: BoundedScalar,
}

impl ResourceSnapshot {
    #[must_use]
    pub const fn definition(&self) -> &DefinitionRef {
        &self.definition
    }

    #[must_use]
    pub const fn resource(&self) -> &ResourceRef {
        &self.resource
    }

    #[must_use]
    pub const fn value(&self) -> BoundedScalar {
        self.value
    }
}

struct ResourceProviderEntry {
    key: ResourceProviderKey,
    definition: DefinitionRef,
    provider: Arc<dyn ResourceProvider>,
}

impl fmt::Debug for ResourceProviderEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceProviderEntry")
            .field("key", &self.key)
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

struct EffectProviderEntry {
    owner: OwnerId,
    definition: DefinitionRef,
    provider: Arc<dyn EffectProvider>,
}

impl fmt::Debug for EffectProviderEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EffectProviderEntry")
            .field("owner", &self.owner)
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

/// Mutable composition-only resource/effect provider table.
#[derive(Default)]
pub struct ResourceEffectProviderRegistryBuilder {
    resources: BTreeMap<ResourceProviderKey, ResourceProviderEntry>,
    effects: BTreeMap<DefinitionRef, EffectProviderEntry>,
    owner_counts: BTreeMap<OwnerId, usize>,
}

impl fmt::Debug for ResourceEffectProviderRegistryBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceEffectProviderRegistryBuilder")
            .field("resources", &self.resources.keys().collect::<Vec<_>>())
            .field("effects", &self.effects.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ResourceEffectProviderRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register_resource(
        &mut self,
        owner: OwnerId,
        definition: DefinitionRef,
        provider: Arc<dyn ResourceProvider>,
    ) -> Result<(), ResourceEffectProviderRegistryError> {
        self.require_capacity(&owner)?;
        let key = ResourceProviderKey::new(
            owner.clone(),
            CanonicalId::new(definition.key().id.as_str()).expect("definition ids are canonical"),
        );
        if self.resources.contains_key(&key) {
            return Err(ResourceEffectProviderRegistryError::DuplicateResource { key });
        }
        self.resources.insert(
            key.clone(),
            ResourceProviderEntry {
                key,
                definition,
                provider,
            },
        );
        self.increment_owner(owner);
        Ok(())
    }

    pub(crate) fn register_effect(
        &mut self,
        owner: OwnerId,
        definition: DefinitionRef,
        provider: Arc<dyn EffectProvider>,
    ) -> Result<(), ResourceEffectProviderRegistryError> {
        self.require_capacity(&owner)?;
        if self.effects.contains_key(&definition) {
            return Err(ResourceEffectProviderRegistryError::DuplicateEffect { definition });
        }
        self.effects.insert(
            definition.clone(),
            EffectProviderEntry {
                owner: owner.clone(),
                definition,
                provider,
            },
        );
        self.increment_owner(owner);
        Ok(())
    }

    fn require_capacity(&self, owner: &OwnerId) -> Result<(), ResourceEffectProviderRegistryError> {
        if self.len() >= MAX_RESOURCE_EFFECT_PROVIDERS {
            return Err(ResourceEffectProviderRegistryError::ProviderLimit {
                max: MAX_RESOURCE_EFFECT_PROVIDERS,
            });
        }
        if self.owner_counts.get(owner).copied().unwrap_or(0)
            >= MAX_RESOURCE_EFFECT_PROVIDERS_PER_OWNER
        {
            return Err(ResourceEffectProviderRegistryError::OwnerProviderLimit {
                owner: owner.clone(),
                max: MAX_RESOURCE_EFFECT_PROVIDERS_PER_OWNER,
            });
        }
        Ok(())
    }

    fn increment_owner(&mut self, owner: OwnerId) {
        *self.owner_counts.entry(owner).or_default() += 1;
    }

    fn len(&self) -> usize {
        self.resources.len() + self.effects.len()
    }

    pub(crate) fn freeze_for_composition(
        self,
        definitions: &DefinitionRegistry,
        module_owners: &BTreeSet<OwnerId>,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<ResourceEffectProviderRegistry, ResourceEffectProviderRegistryError> {
        for entry in self.resources.values() {
            validate_provider_definition(
                definitions,
                module_owners,
                &entry.definition,
                &entry.key.owner,
                "resource",
            )?;
            if entry.definition.key().id.as_str() != entry.key.id.as_str() {
                return Err(ResourceEffectProviderRegistryError::ResourceIdMismatch {
                    key: entry.key.clone(),
                    definition: Box::new(entry.definition.clone()),
                });
            }
        }
        for entry in self.effects.values() {
            validate_provider_definition(
                definitions,
                module_owners,
                &entry.definition,
                &entry.owner,
                "effect",
            )?;
        }
        let fingerprint = DefinitionDigest::for_canonical_value(
            "resource_effect_provider_registry.v1",
            &serde_json::json!({
                "resources": self.resources.values().map(|entry| serde_json::json!({
                    "key": entry.key,
                    "definition": entry.definition,
                })).collect::<Vec<_>>(),
                "effects": self.effects.keys().collect::<Vec<_>>(),
            }),
        )?;
        Ok(ResourceEffectProviderRegistry {
            resources: self.resources,
            effects: self.effects,
            fingerprint,
            owner_readiness,
        })
    }
}

fn validate_provider_definition(
    definitions: &DefinitionRegistry,
    module_owners: &BTreeSet<OwnerId>,
    definition: &DefinitionRef,
    registered_owner: &OwnerId,
    expected_kind: &'static str,
) -> Result<(), ResourceEffectProviderRegistryError> {
    if !module_owners.contains(registered_owner) {
        return Err(ResourceEffectProviderRegistryError::UnknownOwner {
            owner: registered_owner.clone(),
        });
    }
    definitions.get_exact(definition).map_err(|source| {
        ResourceEffectProviderRegistryError::Definition {
            definition: Box::new(definition.clone()),
            source: Box::new(source),
        }
    })?;
    if definition.key().owner != *registered_owner {
        return Err(ResourceEffectProviderRegistryError::OwnerMismatch {
            registered: registered_owner.clone(),
            definition: Box::new(definition.clone()),
        });
    }
    if definition.key().kind.as_str() != expected_kind {
        return Err(ResourceEffectProviderRegistryError::DefinitionKind {
            definition: Box::new(definition.clone()),
            expected: expected_kind,
        });
    }
    Ok(())
}

/// Frozen owner-stamped resource/effect provider table.
pub struct ResourceEffectProviderRegistry {
    resources: BTreeMap<ResourceProviderKey, ResourceProviderEntry>,
    effects: BTreeMap<DefinitionRef, EffectProviderEntry>,
    fingerprint: DefinitionDigest,
    owner_readiness: MechanicalOwnerReadiness,
}

impl fmt::Debug for ResourceEffectProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceEffectProviderRegistry")
            .field("resources", &self.resources.keys().collect::<Vec<_>>())
            .field("effects", &self.effects.keys().collect::<Vec<_>>())
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl ResourceEffectProviderRegistry {
    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.resources.len() + self.effects.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty() && self.effects.is_empty()
    }

    pub(crate) fn read_resource(
        &self,
        world: &World,
        action_state: &ActionStateStore,
        resource: &ResourceRef,
    ) -> Result<Option<ResourceSnapshot>, ResourceReadError> {
        let key = ResourceProviderKey::from_resource(resource);
        let entry = self
            .resources
            .get(&key)
            .ok_or_else(|| ResourceReadError::ProviderMissing {
                resource: Box::new(resource.clone()),
            })?;
        self.owner_readiness.require_ready(key.owner())?;
        let context = ResourceProviderContext::new(world, action_state);
        let value = entry.provider.read(&context, resource).map_err(|source| {
            ResourceReadError::Provider {
                resource: Box::new(resource.clone()),
                source,
            }
        })?;
        self.owner_readiness.require_ready(key.owner())?;
        Ok(value.map(|value| ResourceSnapshot {
            definition: entry.definition.clone(),
            resource: resource.clone(),
            value,
        }))
    }

    pub(crate) fn read_effect(
        &self,
        world: &World,
        action_state: &ActionStateStore,
        definition: &DefinitionRef,
        target: &EffectTarget,
        instance: EffectInstanceId,
    ) -> Result<Option<EffectInstance>, EffectReadError> {
        let entry =
            self.effects
                .get(definition)
                .ok_or_else(|| EffectReadError::ProviderMissing {
                    definition: Box::new(definition.clone()),
                })?;
        let owner = &entry.owner;
        self.owner_readiness.require_ready(owner)?;
        let context = ResourceProviderContext::new(world, action_state);
        let effect = entry
            .provider
            .read(&context, target, instance)
            .map_err(|source| EffectReadError::Provider {
                definition: Box::new(definition.clone()),
                source,
            })?;
        self.owner_readiness.require_ready(owner)?;
        let Some(effect) = effect else {
            return Ok(None);
        };
        let mismatch = if effect.id() != instance {
            Some(EffectProviderResultMismatch::Instance)
        } else if effect.definition() != definition {
            Some(EffectProviderResultMismatch::Definition)
        } else if effect.owner() != owner {
            Some(EffectProviderResultMismatch::Owner)
        } else if effect.target() != target {
            Some(EffectProviderResultMismatch::Target)
        } else {
            None
        };
        if let Some(mismatch) = mismatch {
            return Err(EffectReadError::ResultMismatch {
                definition: Box::new(definition.clone()),
                instance,
                mismatch,
            });
        }
        Ok(Some(effect))
    }
}

/// Composition failure for resource/effect adapters.
#[derive(Debug, Error)]
pub enum ResourceEffectProviderRegistryError {
    #[error("resource/effect provider registry exceeds its hard limit of {max}")]
    ProviderLimit { max: usize },
    #[error("owner {owner} exceeds its resource/effect provider limit of {max}")]
    OwnerProviderLimit { owner: OwnerId, max: usize },
    #[error("resource provider is registered more than once: {key:?}")]
    DuplicateResource { key: ResourceProviderKey },
    #[error("effect provider is registered more than once: {definition}")]
    DuplicateEffect { definition: DefinitionRef },
    #[error("resource/effect provider names uncomposed owner {owner}")]
    UnknownOwner { owner: OwnerId },
    #[error("provider definition {definition} is unavailable: {source}")]
    Definition {
        definition: Box<DefinitionRef>,
        source: Box<DefinitionLookupError>,
    },
    #[error("provider registered by {registered} cannot own definition {definition}")]
    OwnerMismatch {
        registered: OwnerId,
        definition: Box<DefinitionRef>,
    },
    #[error("provider definition {definition} must have kind {expected}")]
    DefinitionKind {
        definition: Box<DefinitionRef>,
        expected: &'static str,
    },
    #[error("resource provider key {key:?} disagrees with definition {definition}")]
    ResourceIdMismatch {
        key: ResourceProviderKey,
        definition: Box<DefinitionRef>,
    },
    #[error("resource/effect provider registry fingerprint is not canonical: {0}")]
    Fingerprint(#[from] CanonicalEncodingError),
}

/// Failure to read one ruleset-owned resource.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResourceReadError {
    #[error("resource provider registry is unavailable")]
    RegistryUnavailable,
    #[error("no provider is registered for resource {resource:?}")]
    ProviderMissing { resource: Box<ResourceRef> },
    #[error("resource provider failed for {resource:?}: {source}")]
    Provider {
        resource: Box<ResourceRef>,
        source: ResourceProviderFailure,
    },
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

/// Coordinate a malformed effect provider attempted to substitute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectProviderResultMismatch {
    Instance,
    Definition,
    Owner,
    Target,
}

/// Failure to read one exact effect instance.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EffectReadError {
    #[error("effect provider registry is unavailable")]
    RegistryUnavailable,
    #[error("no provider is registered for effect definition {definition}")]
    ProviderMissing { definition: Box<DefinitionRef> },
    #[error("effect provider failed for definition {definition}: {source}")]
    Provider {
        definition: Box<DefinitionRef>,
        source: ResourceProviderFailure,
    },
    #[error(
        "effect provider returned mismatched {mismatch:?} for {definition} instance {instance:?}"
    )]
    ResultMismatch {
        definition: Box<DefinitionRef>,
        instance: EffectInstanceId,
        mismatch: EffectProviderResultMismatch,
    },
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}
