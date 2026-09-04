//! Composition-frozen providers for module-owned typed clocks.
//!
//! The kernel constructs the returned [`ClockStamp`], so a provider supplies
//! only the scalar value for the exact requested generation and cannot relabel
//! work onto another clock. Providers receive immutable world and typed action
//! state through a narrow context.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::action_state::{ActionStateError, ActionStateStore, StateFamilyHandle};
use crate::clock::{ClockRef, ClockStamp};
use crate::mechanics::{
    CanonicalEncodingError, CanonicalId, DefinitionDigest, MechanicalOwnerReadiness,
    MechanicalOwnerReadinessError, OwnerId,
};
use crate::world::World;

/// Maximum registered module clocks in one deployment.
pub const MAX_CLOCK_PROVIDERS: usize = 4_096;
/// Maximum registered clocks owned by one module.
pub const MAX_CLOCK_PROVIDERS_PER_OWNER: usize = 1_024;

/// Structurally distinct module-owned clock classes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockProviderKind {
    DomainTick,
    Registered,
}

/// Exact owner-stamped provider coordinate.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClockProviderKey {
    owner: OwnerId,
    kind: ClockProviderKind,
    id: CanonicalId,
}

impl ClockProviderKey {
    #[must_use]
    pub const fn new(owner: OwnerId, kind: ClockProviderKind, id: CanonicalId) -> Self {
        Self { owner, kind, id }
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn kind(&self) -> ClockProviderKind {
        self.kind
    }

    #[must_use]
    pub const fn id(&self) -> &CanonicalId {
        &self.id
    }

    fn from_clock(clock: &ClockRef) -> Option<(Self, u64)> {
        match clock {
            ClockRef::DomainTick {
                owner,
                id,
                generation,
            } => Some((
                Self::new(owner.clone(), ClockProviderKind::DomainTick, id.clone()),
                *generation,
            )),
            ClockRef::Registered {
                owner,
                id,
                generation,
            } => Some((
                Self::new(owner.clone(), ClockProviderKind::Registered, id.clone()),
                *generation,
            )),
            ClockRef::Utc | ClockRef::WorldIc { .. } | ClockRef::SceneTurn { .. } => None,
        }
    }
}

/// Immutable truth exposed during a synchronous clock read.
#[derive(Clone, Copy)]
pub struct ClockProviderContext<'a> {
    world: &'a World,
    action_state: &'a ActionStateStore,
}

impl fmt::Debug for ClockProviderContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClockProviderContext")
            .field("utc_now", &self.world.now())
            .field("action_state_revision", &self.action_state.revision())
            .finish_non_exhaustive()
    }
}

impl<'a> ClockProviderContext<'a> {
    pub(crate) const fn new(world: &'a World, action_state: &'a ActionStateStore) -> Self {
        Self {
            world,
            action_state,
        }
    }

    #[must_use]
    pub fn utc_now(&self) -> i64 {
        self.world.now()
    }

    #[must_use]
    pub fn world_ic_now(&self) -> ClockStamp {
        self.world.world_ic_now()
    }

    /// Read immutable module state registered in the ordinary world context.
    #[must_use]
    pub fn module_state<T: Any + Send>(&self) -> Option<&'a T> {
        self.world.module_state::<T>()
    }

    /// Read one authoritative typed-family value through its composition-
    /// branded handle.
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

/// Pure synchronous provider for one registered clock identity.
pub trait ClockProvider: Send + Sync {
    /// Return the scalar current value for the exact requested generation.
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure>;
}

/// Pure synchronous provider for one owner's dynamic domain-clock namespace.
///
/// The provider is registered once during composition. Runtime domain ids are
/// carried by [`ClockRef::DomainTick`] and passed into the provider; promoting
/// a domain therefore never mutates the frozen registry.
pub trait DomainClockProvider: Send + Sync {
    /// Return the scalar current value for one exact domain and generation.
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        domain: &CanonicalId,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure>;
}

/// Pure synchronous provider for the one first-class scene-clock namespace.
///
/// [`ClockRef::SceneTurn`] intentionally carries no module owner: durable
/// scene ids are world-global coordinates. Composition therefore permits
/// exactly one owner-stamped adapter and still subjects every read to that
/// owner's readiness barrier.
pub trait SceneClockProvider: Send + Sync {
    /// Return the scalar current value for one exact scene and generation.
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        scene: u64,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure>;
}

impl<F> SceneClockProvider for F
where
    F: for<'a> Fn(&ClockProviderContext<'a>, u64, u64) -> Result<i64, ClockProviderFailure>
        + Send
        + Sync,
{
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        scene: u64,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure> {
        self(context, scene, generation)
    }
}

impl<F> DomainClockProvider for F
where
    F: for<'a> Fn(
            &ClockProviderContext<'a>,
            &CanonicalId,
            u64,
        ) -> Result<i64, ClockProviderFailure>
        + Send
        + Sync,
{
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        domain: &CanonicalId,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure> {
        self(context, domain, generation)
    }
}

impl<F> ClockProvider for F
where
    F: for<'a> Fn(&ClockProviderContext<'a>, u64) -> Result<i64, ClockProviderFailure>
        + Send
        + Sync,
{
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure> {
        self(context, generation)
    }
}

/// Bounded provider-level refusal.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ClockProviderFailure {
    #[error("clock is temporarily unavailable")]
    Unavailable,
    #[error("clock generation {requested} is stale; current generation is {current}")]
    StaleGeneration { requested: u64, current: u64 },
    #[error("clock arithmetic overflow")]
    Overflow,
    #[error("clock provider detected invalid state")]
    Integrity,
}

struct ClockProviderEntry {
    key: ClockProviderKey,
    provider: Arc<dyn ClockProvider>,
}

struct SceneClockProviderEntry {
    owner: OwnerId,
    provider: Arc<dyn SceneClockProvider>,
}

struct DomainClockProviderEntry {
    owner: OwnerId,
    provider: Arc<dyn DomainClockProvider>,
}

impl fmt::Debug for DomainClockProviderEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DomainClockProviderEntry")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for SceneClockProviderEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SceneClockProviderEntry")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ClockProviderEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClockProviderEntry")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// Mutable composition-only clock-provider table.
#[derive(Default)]
pub struct ClockProviderRegistryBuilder {
    entries: BTreeMap<ClockProviderKey, ClockProviderEntry>,
    domains: BTreeMap<OwnerId, DomainClockProviderEntry>,
    scene: Option<SceneClockProviderEntry>,
    owner_counts: BTreeMap<OwnerId, usize>,
}

impl fmt::Debug for ClockProviderRegistryBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClockProviderRegistryBuilder")
            .field("keys", &self.entries.keys().collect::<Vec<_>>())
            .field("domain_owners", &self.domains.keys().collect::<Vec<_>>())
            .field(
                "scene_owner",
                &self.scene.as_ref().map(|entry| &entry.owner),
            )
            .finish()
    }
}

impl ClockProviderRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register(
        &mut self,
        owner: OwnerId,
        kind: ClockProviderKind,
        id: CanonicalId,
        provider: Arc<dyn ClockProvider>,
    ) -> Result<(), ClockProviderRegistryError> {
        if self.len() >= MAX_CLOCK_PROVIDERS {
            return Err(ClockProviderRegistryError::ProviderLimit {
                max: MAX_CLOCK_PROVIDERS,
            });
        }
        let owner_count = self.owner_counts.get(&owner).copied().unwrap_or(0);
        if owner_count >= MAX_CLOCK_PROVIDERS_PER_OWNER {
            return Err(ClockProviderRegistryError::OwnerProviderLimit {
                owner,
                max: MAX_CLOCK_PROVIDERS_PER_OWNER,
            });
        }
        let key = ClockProviderKey::new(owner.clone(), kind, id);
        if self.entries.contains_key(&key) {
            return Err(ClockProviderRegistryError::Duplicate { key });
        }
        self.entries
            .insert(key.clone(), ClockProviderEntry { key, provider });
        self.owner_counts.insert(owner, owner_count + 1);
        Ok(())
    }

    pub(crate) fn register_scene(
        &mut self,
        owner: OwnerId,
        provider: Arc<dyn SceneClockProvider>,
    ) -> Result<(), ClockProviderRegistryError> {
        if self.len() >= MAX_CLOCK_PROVIDERS {
            return Err(ClockProviderRegistryError::ProviderLimit {
                max: MAX_CLOCK_PROVIDERS,
            });
        }
        let owner_count = self.owner_counts.get(&owner).copied().unwrap_or(0);
        if owner_count >= MAX_CLOCK_PROVIDERS_PER_OWNER {
            return Err(ClockProviderRegistryError::OwnerProviderLimit {
                owner,
                max: MAX_CLOCK_PROVIDERS_PER_OWNER,
            });
        }
        if let Some(existing) = &self.scene {
            return Err(ClockProviderRegistryError::DuplicateScene {
                existing_owner: existing.owner.clone(),
                requested_owner: owner,
            });
        }
        self.scene = Some(SceneClockProviderEntry {
            owner: owner.clone(),
            provider,
        });
        self.owner_counts.insert(owner, owner_count + 1);
        Ok(())
    }

    pub(crate) fn register_domain(
        &mut self,
        owner: OwnerId,
        provider: Arc<dyn DomainClockProvider>,
    ) -> Result<(), ClockProviderRegistryError> {
        if self.len() >= MAX_CLOCK_PROVIDERS {
            return Err(ClockProviderRegistryError::ProviderLimit {
                max: MAX_CLOCK_PROVIDERS,
            });
        }
        let owner_count = self.owner_counts.get(&owner).copied().unwrap_or(0);
        if owner_count >= MAX_CLOCK_PROVIDERS_PER_OWNER {
            return Err(ClockProviderRegistryError::OwnerProviderLimit {
                owner,
                max: MAX_CLOCK_PROVIDERS_PER_OWNER,
            });
        }
        if self.domains.contains_key(&owner) {
            return Err(ClockProviderRegistryError::DuplicateDomain { owner });
        }
        self.domains.insert(
            owner.clone(),
            DomainClockProviderEntry {
                owner: owner.clone(),
                provider,
            },
        );
        self.owner_counts.insert(owner, owner_count + 1);
        Ok(())
    }

    fn len(&self) -> usize {
        self.entries.len() + self.domains.len() + usize::from(self.scene.is_some())
    }

    pub(crate) fn freeze_for_owners_with_readiness(
        self,
        module_owners: &BTreeSet<OwnerId>,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<ClockProviderRegistry, ClockProviderRegistryError> {
        if let Some(key) = self
            .entries
            .keys()
            .find(|key| !module_owners.contains(key.owner()))
        {
            return Err(ClockProviderRegistryError::UnknownOwner { key: key.clone() });
        }
        if let Some(scene) = &self.scene
            && !module_owners.contains(&scene.owner)
        {
            return Err(ClockProviderRegistryError::UnknownSceneOwner {
                owner: scene.owner.clone(),
            });
        }
        if let Some(owner) = self
            .domains
            .keys()
            .find(|owner| !module_owners.contains(*owner))
        {
            return Err(ClockProviderRegistryError::UnknownDomainOwner {
                owner: owner.clone(),
            });
        }
        let fingerprint = DefinitionDigest::for_canonical_value(
            "clock_provider_registry.v3",
            &serde_json::json!({
                "registered": self.entries.keys().collect::<Vec<_>>(),
                "domain_owners": self.domains.keys().collect::<Vec<_>>(),
                "scene_owner": self.scene.as_ref().map(|entry| &entry.owner),
            }),
        )?;
        Ok(ClockProviderRegistry {
            entries: self.entries,
            domains: self.domains,
            scene: self.scene,
            fingerprint,
            owner_readiness,
        })
    }
}

/// Frozen owner-stamped clock-provider table.
pub struct ClockProviderRegistry {
    entries: BTreeMap<ClockProviderKey, ClockProviderEntry>,
    domains: BTreeMap<OwnerId, DomainClockProviderEntry>,
    scene: Option<SceneClockProviderEntry>,
    fingerprint: DefinitionDigest,
    owner_readiness: MechanicalOwnerReadiness,
}

impl fmt::Debug for ClockProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClockProviderRegistry")
            .field("keys", &self.entries.keys().collect::<Vec<_>>())
            .field("domain_owners", &self.domains.keys().collect::<Vec<_>>())
            .field(
                "scene_owner",
                &self.scene.as_ref().map(|entry| &entry.owner),
            )
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl ClockProviderRegistry {
    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len() + self.domains.len() + usize::from(self.scene.is_some())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.domains.is_empty() && self.scene.is_none()
    }

    pub(crate) fn read(
        &self,
        world: &World,
        action_state: &ActionStateStore,
        clock: &ClockRef,
    ) -> Result<ClockStamp, ClockReadError> {
        let (key, generation) = ClockProviderKey::from_clock(clock).ok_or_else(|| {
            ClockReadError::UnsupportedClock {
                clock: clock.clone(),
            }
        })?;
        if generation == 0 {
            return Err(ClockReadError::ZeroGeneration {
                clock: clock.clone(),
            });
        }
        let entry = self
            .entries
            .get(&key)
            .ok_or_else(|| ClockReadError::ProviderMissing { key: key.clone() })?;
        self.owner_readiness.require_ready(key.owner())?;
        let context = ClockProviderContext::new(world, action_state);
        let at = entry.provider.now(&context, generation).map_err(|source| {
            ClockReadError::Provider {
                key: key.clone(),
                source,
            }
        })?;
        self.owner_readiness.require_ready(key.owner())?;
        Ok(ClockStamp {
            clock: clock.clone(),
            at,
        })
    }

    pub(crate) fn read_scene(
        &self,
        world: &World,
        action_state: &ActionStateStore,
        clock: &ClockRef,
    ) -> Result<ClockStamp, ClockReadError> {
        let ClockRef::SceneTurn { scene, generation } = clock else {
            return Err(ClockReadError::UnsupportedClock {
                clock: clock.clone(),
            });
        };
        if *generation == 0 {
            return Err(ClockReadError::ZeroGeneration {
                clock: clock.clone(),
            });
        }
        let entry = self
            .scene
            .as_ref()
            .ok_or(ClockReadError::SceneProviderMissing)?;
        self.owner_readiness.require_ready(&entry.owner)?;
        let context = ClockProviderContext::new(world, action_state);
        let at = entry
            .provider
            .now(&context, *scene, *generation)
            .map_err(|source| ClockReadError::SceneProvider {
                owner: entry.owner.clone(),
                source,
            })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(ClockStamp {
            clock: clock.clone(),
            at,
        })
    }

    pub(crate) fn read_domain(
        &self,
        world: &World,
        action_state: &ActionStateStore,
        clock: &ClockRef,
    ) -> Result<ClockStamp, ClockReadError> {
        let ClockRef::DomainTick {
            owner,
            id,
            generation,
        } = clock
        else {
            return Err(ClockReadError::UnsupportedClock {
                clock: clock.clone(),
            });
        };
        if *generation == 0 {
            return Err(ClockReadError::ZeroGeneration {
                clock: clock.clone(),
            });
        }
        let entry =
            self.domains
                .get(owner)
                .ok_or_else(|| ClockReadError::DomainProviderMissing {
                    owner: owner.clone(),
                })?;
        self.owner_readiness.require_ready(owner)?;
        let context = ClockProviderContext::new(world, action_state);
        let at = entry
            .provider
            .now(&context, id, *generation)
            .map_err(|source| ClockReadError::DomainProvider {
                owner: owner.clone(),
                id: id.clone(),
                source,
            })?;
        self.owner_readiness.require_ready(owner)?;
        Ok(ClockStamp {
            clock: clock.clone(),
            at,
        })
    }
}

/// Composition failure for the registered clock table.
#[derive(Debug, Error)]
pub enum ClockProviderRegistryError {
    #[error("clock provider registry exceeds its hard limit of {max}")]
    ProviderLimit { max: usize },
    #[error("clock owner {owner} exceeds its provider limit of {max}")]
    OwnerProviderLimit { owner: OwnerId, max: usize },
    #[error("clock provider is registered more than once: {key:?}")]
    Duplicate { key: ClockProviderKey },
    #[error(
        "scene clock provider is already owned by {existing_owner}; {requested_owner} cannot replace it"
    )]
    DuplicateScene {
        existing_owner: OwnerId,
        requested_owner: OwnerId,
    },
    #[error("domain clock provider is already registered for owner {owner}")]
    DuplicateDomain { owner: OwnerId },
    #[error("clock provider names an uncomposed owner: {key:?}")]
    UnknownOwner { key: ClockProviderKey },
    #[error("scene clock provider names an uncomposed owner: {owner}")]
    UnknownSceneOwner { owner: OwnerId },
    #[error("domain clock provider names an uncomposed owner: {owner}")]
    UnknownDomainOwner { owner: OwnerId },
    #[error("clock provider registry fingerprint is not canonical: {0}")]
    Fingerprint(#[from] CanonicalEncodingError),
}

/// Failure to read one exact typed clock.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ClockReadError {
    #[error("clock provider registry is unavailable")]
    RegistryUnavailable,
    #[error("clock cannot be read through the module provider registry: {clock:?}")]
    UnsupportedClock { clock: ClockRef },
    #[error("clock generation zero is reserved: {clock:?}")]
    ZeroGeneration { clock: ClockRef },
    #[error("no provider is registered for {key:?}")]
    ProviderMissing { key: ClockProviderKey },
    #[error("no first-class scene clock provider is registered")]
    SceneProviderMissing,
    #[error("no domain clock provider is registered for owner {owner}")]
    DomainProviderMissing { owner: OwnerId },
    #[error("scene clock provider owned by {owner} failed: {source}")]
    SceneProvider {
        owner: OwnerId,
        source: ClockProviderFailure,
    },
    #[error("domain clock provider {owner}/{id} failed: {source}")]
    DomainProvider {
        owner: OwnerId,
        id: CanonicalId,
        source: ClockProviderFailure,
    },
    #[error("clock provider {key:?} failed: {source}")]
    Provider {
        key: ClockProviderKey,
        source: ClockProviderFailure,
    },
    #[error("requested clock generation {requested} is stale; current generation is {current}")]
    StaleGeneration { requested: u64, current: u64 },
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}
