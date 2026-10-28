//! Bounded, composition-bound cross-owner reads. A service is registered by its
//! actual owner and called only from an attributed command/query or authoritative
//! preparation context. No mutable World, typed writer, persistence hook or
//! caller-made owner lens escapes.

use std::any::{Any, TypeId};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Write};
use std::marker::PhantomData;
use std::sync::Arc;

use crate::action::PrincipalId;
use crate::action_state::{ModuleActionState, StateFamilyHandle, StateFamilyPage, TypedStateLens};
use crate::mechanics::{
    DefinitionDigest, DefinitionRef, DefinitionRegistry, MechanicalOwnerReadiness, OwnerId,
};
use crate::module::StateInit;
use crate::{ObjId, World};
use serde::Serialize;

// Declaration capacity for the full composed roster: the lengths of two heap
// `Vec`s checked at `register`/`request`, outside the `owner_read_services.v1`
// fingerprint and no memory bound, independent of each invocation's unchanged
// read, row and encoding budgets below. 48 since 17g C2e, 64 since the scoped
// passenger facts (the format-2 Active launch composes 33 services and 35
// grants: the passenger facts' 34 plus TacSim's grant of the market read).
pub const MAX_OWNER_READ_SERVICES: usize = 64;
pub const MAX_OWNER_READ_GRANTS: usize = 64;
pub const MAX_OWNER_READ_REQUEST_BYTES: usize = 16 * 1024;
pub const MAX_OWNER_READ_RESULT_BYTES: usize = 1024 * 1024;
pub const MAX_OWNER_READS: u16 = 256;
pub const MAX_OWNER_READ_ROWS: u16 = 512;
pub const MAX_OWNER_READ_PAGE: usize = 64;
/// Per authoritative preparation, including its same-commit triggers. Failed
/// or excess reads refuse the complete preparation; proofs are never evicted.
pub const MAX_OWNER_PREIMAGE_READS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OwnerReadError {
    #[error("owner read service is unavailable")]
    Unavailable,
    #[error("owner read registration or caller is invalid")]
    Authority,
    #[error("owner read exceeded its bounded envelope")]
    Limit,
    #[error("owner read refused the requested subject")]
    Refused,
    #[error("owner read state failed an integrity check")]
    Integrity,
}

/// A bounded request/result contract, encoded with the same serde JSON codec
/// the service declares. Limits are checked before a request enters owner code
/// and before a result leaves it. Values never become transport registrations.
pub trait OwnerReadService: Send + Sync + 'static {
    type State: Any + Send;
    type Request: Serialize + Send + 'static;
    type Output: Serialize + Send + 'static;
    fn read(
        &self,
        state: &Self::State,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError>;
}

/// Actual calling identity, minted by core from dispatch context. A request may
/// name another subject, but can never replace this requester or its principal.
#[derive(Clone)]
pub(crate) struct OwnerReadCaller {
    module: OwnerId,
    actor: ObjId,
    principal: PrincipalId,
}
impl OwnerReadCaller {
    pub(crate) fn module(&self) -> &OwnerId {
        &self.module
    }
    pub(crate) const fn actor(&self) -> ObjId {
        self.actor
    }
    pub(crate) fn object(module: &str, actor: ObjId) -> Result<Self, OwnerReadError> {
        Ok(Self {
            module: OwnerId::parse_canonical(module).map_err(|_| OwnerReadError::Authority)?,
            actor,
            principal: PrincipalId::Object { id: actor },
        })
    }
}

/// One private owner-stamped read lens. Every get/scan consumes the same small
/// budget. The result is owned and 'static, so no borrowed typed row escapes.
pub struct OwnerReadContext<'a> {
    world: &'a World,
    typed: ModuleActionState<'a>,
    caller: &'a OwnerReadCaller,
    reads: Cell<u16>,
    rows: Cell<u16>,
    exhausted: Cell<bool>,
}
impl<'a> OwnerReadContext<'a> {
    pub const fn world(&self) -> &'a World {
        self.world
    }
    pub const fn actor(&self) -> ObjId {
        self.caller.actor
    }
    pub const fn principal(&self) -> &PrincipalId {
        &self.caller.principal
    }
    pub const fn requester(&self) -> &OwnerId {
        &self.caller.module
    }
    fn charge(&self, rows: usize) -> Result<(), OwnerReadError> {
        if self.exhausted.get() {
            return Err(OwnerReadError::Limit);
        }
        self.exhausted.set(true);
        let reads = self
            .reads
            .get()
            .checked_add(1)
            .ok_or(OwnerReadError::Limit)?;
        let rows = self
            .rows
            .get()
            .checked_add(u16::try_from(rows).map_err(|_| OwnerReadError::Limit)?)
            .ok_or(OwnerReadError::Limit)?;
        if reads > MAX_OWNER_READS || rows > MAX_OWNER_READ_ROWS {
            return Err(OwnerReadError::Limit);
        }
        self.reads.set(reads);
        self.rows.set(rows);
        self.exhausted.set(false);
        Ok(())
    }
    pub fn action_state<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&'a V>, OwnerReadError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.charge(1)?;
        self.typed
            .get(handle, key)
            .map_err(|_| OwnerReadError::Integrity)
    }
    /// Read a bounded group of exact keys from one actual typed family.
    ///
    /// Like a bounded scan, this consumes one read operation and charges every
    /// requested row before reading. Missing and repeated keys still count.
    /// Results preserve key order; an owner/handle failure returns no partial
    /// batch. No scan cursor, caller-supplied owner or detached lens is created.
    pub fn action_states<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        keys: &[K],
    ) -> Result<Vec<Option<&'a V>>, OwnerReadError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        if keys.is_empty() || keys.len() > MAX_OWNER_READ_PAGE {
            self.exhausted.set(true);
            return Err(OwnerReadError::Limit);
        }
        self.charge(keys.len())?;
        keys.iter()
            .map(|key| {
                self.typed
                    .get(handle, key)
                    .map_err(|_| OwnerReadError::Integrity)
            })
            .collect()
    }

    pub fn scan_action_state<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<StateFamilyPage<'a, V>, OwnerReadError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        if limit == 0 || limit > MAX_OWNER_READ_PAGE {
            self.exhausted.set(true);
            return Err(OwnerReadError::Limit);
        }
        self.charge(limit)?;
        self.typed
            .scan_family(handle, after, limit)
            .map_err(|_| OwnerReadError::Integrity)
    }
}

pub struct OwnerReadHandle<S: OwnerReadService> {
    composition: Arc<()>,
    requester: OwnerId,
    definition: DefinitionRef,
    marker: PhantomData<fn() -> S>,
}
impl<S: OwnerReadService> Clone for OwnerReadHandle<S> {
    fn clone(&self) -> Self {
        Self {
            composition: Arc::clone(&self.composition),
            requester: self.requester.clone(),
            definition: self.definition.clone(),
            marker: PhantomData,
        }
    }
}
impl<S: OwnerReadService> fmt::Debug for OwnerReadHandle<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnerReadHandle")
            .field("service", &self.definition)
            .field("requester", &self.requester)
            .finish_non_exhaustive()
    }
}
struct Registration {
    owner: OwnerId,
    definition: DefinitionRef,
    allowed: BTreeSet<OwnerId>,
    dependencies: BTreeSet<OwnerId>,
    service_type: TypeId,
    state_type: TypeId,
    state_name: &'static str,
    service: Box<dyn Any + Send + Sync>,
}
struct Grant {
    requester: OwnerId,
    definition: DefinitionRef,
    service_type: TypeId,
}
#[derive(Default)]
pub(crate) struct OwnerReadRegistryBuilder {
    composition: Arc<()>,
    registrations: Vec<Registration>,
    grants: Vec<Grant>,
}
impl OwnerReadRegistryBuilder {
    pub(crate) fn register<S: OwnerReadService>(
        &mut self,
        owner: OwnerId,
        definition: DefinitionRef,
        allowed: BTreeSet<OwnerId>,
        dependencies: BTreeSet<OwnerId>,
        service: S,
    ) -> Result<(), OwnerReadError> {
        if self.registrations.len() >= MAX_OWNER_READ_SERVICES
            || allowed.len() > MAX_OWNER_READ_GRANTS
            || dependencies.len() > MAX_OWNER_READ_GRANTS
        {
            return Err(OwnerReadError::Limit);
        }
        if definition.key().owner != owner
            || definition.key().kind.as_str() != "owner_read"
            || self
                .registrations
                .iter()
                .any(|r| r.definition == definition || r.service_type == TypeId::of::<S>())
        {
            return Err(OwnerReadError::Authority);
        }
        self.registrations.push(Registration {
            owner,
            definition,
            allowed,
            dependencies,
            service_type: TypeId::of::<S>(),
            state_type: TypeId::of::<S::State>(),
            state_name: std::any::type_name::<S::State>(),
            service: Box::new(service),
        });
        Ok(())
    }
    pub(crate) fn request<S: OwnerReadService>(
        &mut self,
        requester: OwnerId,
        definition: DefinitionRef,
    ) -> Result<OwnerReadHandle<S>, OwnerReadError> {
        if self.grants.len() >= MAX_OWNER_READ_GRANTS {
            return Err(OwnerReadError::Limit);
        }
        if self
            .grants
            .iter()
            .any(|g| g.requester == requester && g.definition == definition)
        {
            return Err(OwnerReadError::Authority);
        }
        self.grants.push(Grant {
            requester: requester.clone(),
            definition: definition.clone(),
            service_type: TypeId::of::<S>(),
        });
        Ok(OwnerReadHandle {
            composition: Arc::clone(&self.composition),
            requester,
            definition,
            marker: PhantomData,
        })
    }
    pub(crate) fn freeze(
        mut self,
        definitions: &DefinitionRegistry,
        owners: &BTreeSet<OwnerId>,
        slots: &[StateInit],
        readiness: MechanicalOwnerReadiness,
    ) -> Result<OwnerReadRegistry, OwnerReadError> {
        for r in &self.registrations {
            definitions
                .get_exact(&r.definition)
                .map_err(|_| OwnerReadError::Authority)?;
            if !owners.contains(&r.owner)
                || !r.allowed.is_subset(owners)
                || !r.dependencies.is_subset(owners)
                || !slots
                    .iter()
                    .any(|s| s.type_id() == r.state_type && s.module().as_str() == r.owner.as_str())
            {
                return Err(OwnerReadError::Authority);
            }
        }
        for g in &self.grants {
            if !owners.contains(&g.requester)
                || !self.registrations.iter().any(|r| {
                    r.service_type == g.service_type
                        && r.definition == g.definition
                        && r.allowed.contains(&g.requester)
                })
            {
                return Err(OwnerReadError::Authority);
            }
        }
        self.registrations
            .sort_by(|a, b| a.definition.cmp(&b.definition));
        self.grants
            .sort_by(|a, b| (&a.requester, &a.definition).cmp(&(&b.requester, &b.definition)));
        let fingerprint = DefinitionDigest::for_canonical_value("owner_read_services.v1", &serde_json::json!({
            "services":self.registrations.iter().map(|r|serde_json::json!({"owner":r.owner,"definition":r.definition,"allowed":r.allowed,"dependencies":r.dependencies,"state":r.state_name})).collect::<Vec<_>>(),
            "grants":self.grants.iter().map(|g|serde_json::json!({"requester":g.requester,"definition":g.definition})).collect::<Vec<_>>(),
            "limits":{"request":MAX_OWNER_READ_REQUEST_BYTES,"result":MAX_OWNER_READ_RESULT_BYTES,"reads":MAX_OWNER_READS,"rows":MAX_OWNER_READ_ROWS,"page":MAX_OWNER_READ_PAGE,"preimage_calls":MAX_OWNER_PREIMAGE_READS}
        })).map_err(|_|OwnerReadError::Limit)?;
        Ok(OwnerReadRegistry {
            composition: self.composition,
            registrations: self.registrations,
            grants: self.grants,
            readiness,
            fingerprint,
        })
    }
}
pub(crate) struct OwnerReadRegistry {
    composition: Arc<()>,
    registrations: Vec<Registration>,
    grants: Vec<Grant>,
    readiness: MechanicalOwnerReadiness,
    fingerprint: DefinitionDigest,
}
impl OwnerReadRegistry {
    pub(crate) fn is_empty(&self) -> bool {
        self.registrations.is_empty() && self.grants.is_empty()
    }
    /// `(services, grants)` registered in this frozen composition.
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.registrations.len(), self.grants.len())
    }
    pub(crate) const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }
    pub(crate) fn call<S: OwnerReadService>(
        &self,
        world: &World,
        lens: TypedStateLens<'_>,
        caller: &OwnerReadCaller,
        handle: &OwnerReadHandle<S>,
        request: S::Request,
    ) -> Result<S::Output, OwnerReadError> {
        self.call_observed(world, lens, caller, handle, request)
            .map(|(value, _)| value)
    }

    fn call_observed<S: OwnerReadService>(
        &self,
        world: &World,
        lens: TypedStateLens<'_>,
        caller: &OwnerReadCaller,
        handle: &OwnerReadHandle<S>,
        request: S::Request,
    ) -> Result<(S::Output, ReadObservation), OwnerReadError> {
        if lens.store().is_none() {
            return Err(OwnerReadError::Unavailable);
        }
        if !Arc::ptr_eq(&self.composition, &handle.composition)
            || caller.module != handle.requester
            || world.get(caller.actor).is_none()
            || caller.principal != (PrincipalId::Object { id: caller.actor })
            || !self.grants.iter().any(|g| {
                g.requester == handle.requester
                    && g.definition == handle.definition
                    && g.service_type == TypeId::of::<S>()
            })
        {
            return Err(OwnerReadError::Authority);
        }
        let r = self
            .registrations
            .iter()
            .find(|r| r.service_type == TypeId::of::<S>() && r.definition == handle.definition)
            .ok_or(OwnerReadError::Authority)?;
        let ready = || {
            self.readiness
                .require_ready(&r.owner)
                .map_err(|_| OwnerReadError::Unavailable)?;
            self.readiness
                .require_ready(&caller.module)
                .map_err(|_| OwnerReadError::Unavailable)?;
            for owner in &r.dependencies {
                self.readiness
                    .require_ready(owner)
                    .map_err(|_| OwnerReadError::Unavailable)?;
            }
            Ok(())
        };
        ready()?;
        if world
            .module_state_owner(r.state_type)
            .is_none_or(|owner| owner.as_str() != r.owner.as_str())
        {
            return Err(OwnerReadError::Integrity);
        }
        let (capture, state) = world
            .participant_slot(r.state_type)
            .ok_or(OwnerReadError::Unavailable)?;
        let state = state
            .downcast_ref::<S::State>()
            .ok_or(OwnerReadError::Integrity)?;
        let service = r
            .service
            .downcast_ref::<S>()
            .ok_or(OwnerReadError::Integrity)?;
        encoded_size(&request, MAX_OWNER_READ_REQUEST_BYTES)?;
        // Serialize belongs to module code too. It may change a shared owner
        // readiness latch, so it is inside the same pre-read fence as read().
        ready()?;
        if world.participant_captured_slot(&capture).is_none() {
            return Err(OwnerReadError::Integrity);
        }
        let context = OwnerReadContext {
            world,
            typed: ModuleActionState::for_module(lens, Some(r.owner.as_str())),
            caller,
            reads: Cell::new(0),
            rows: Cell::new(0),
            exhausted: Cell::new(false),
        };
        let result = service.read(state, &context, request)?;
        if context.exhausted.get() {
            return Err(OwnerReadError::Limit);
        }
        // Complete every module callback before the final readiness/slot check.
        encoded_size(&result, MAX_OWNER_READ_RESULT_BYTES)?;
        ready()?;
        if world.participant_captured_slot(&capture).is_none() {
            return Err(OwnerReadError::Integrity);
        }
        Ok((
            result,
            ReadObservation {
                definition: r.definition.clone(),
                owner: r.owner.clone(),
                requester: caller.module.clone(),
                dependencies: r.dependencies.clone(),
                capture,
                readiness: self.readiness.clone(),
            },
        ))
    }
}

struct ReadObservation {
    definition: DefinitionRef,
    owner: OwnerId,
    requester: OwnerId,
    dependencies: BTreeSet<OwnerId>,
    capture: crate::module::ErasedSlotCapture,
    readiness: MechanicalOwnerReadiness,
}

/// Core-owned proof of which registered services were actually called. No
/// provider constructor, serialization, callback, or write authority.
#[derive(Default)]
pub(crate) struct ReadObservations(Vec<ReadObservation>);
impl fmt::Debug for ReadObservations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ReadObservations")
            .field(&self.0.len())
            .finish()
    }
}
impl ReadObservations {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn definitions(&self) -> impl Iterator<Item = &DefinitionRef> {
        self.0.iter().map(|r| &r.definition)
    }
    pub(crate) fn recheck(
        &self,
        world: &World,
    ) -> Result<(), crate::participant::ParticipantError> {
        use crate::participant::ParticipantError;
        self.require_ready()?;
        for r in &self.0 {
            if world
                .module_state_owner(r.capture.type_id())
                .is_none_or(|owner| owner.as_str() != r.owner.as_str())
                || world.participant_captured_slot(&r.capture).is_none()
            {
                return Err(ParticipantError::Slot);
            }
        }
        Ok(())
    }

    /// Callback-free final latch check while owner slots are mutably bound.
    /// The map borrow keeps the already-checked slot incarnations fixed.
    pub(crate) fn require_ready(&self) -> Result<(), crate::participant::ParticipantError> {
        use crate::participant::ParticipantError;
        for r in &self.0 {
            for owner in std::iter::once(&r.owner)
                .chain(std::iter::once(&r.requester))
                .chain(r.dependencies.iter())
            {
                r.readiness
                    .require_ready(owner)
                    .map_err(|_| ParticipantError::NotReady)?;
            }
        }
        Ok(())
    }
}

/// Attributed by the actual provider dispatch, before any module callback.
pub(crate) struct PrepareReadContext<'a> {
    registry: &'a OwnerReadRegistry,
    world: &'a World,
    provider: &'a crate::action::ActionProviderEntry,
    caller: OwnerReadCaller,
    base: &'a crate::action_state::ActionReadBase,
    observations: &'a mut ReadObservations,
}
impl<'a> PrepareReadContext<'a> {
    pub(crate) fn new(
        registry: &'a OwnerReadRegistry,
        world: &'a World,
        provider: &'a crate::action::ActionProviderEntry,
        invocation: &crate::action::ActionInvocation,
        base: &'a crate::action_state::ActionReadBase,
        observations: &'a mut ReadObservations,
    ) -> Self {
        Self {
            registry,
            world,
            provider,
            caller: OwnerReadCaller {
                module: provider.owner().clone(),
                actor: invocation.actor(),
                principal: invocation.principal().clone(),
            },
            base,
            observations,
        }
    }

    pub(crate) fn read<S: OwnerReadService>(
        &mut self,
        txn: &crate::action_state::ActionTxn<'_>,
        handle: &OwnerReadHandle<S>,
        request: S::Request,
    ) -> Result<S::Output, OwnerReadError> {
        if self.observations.0.len() >= MAX_OWNER_PREIMAGE_READS {
            return Err(OwnerReadError::Limit);
        }
        let lens = txn.read_preimage(self.base, self.provider.registration_ordinal())?;
        let (output, observation) =
            self.registry
                .call_observed(self.world, lens, &self.caller, handle, request)?;
        self.observations.0.push(observation);
        Ok(output)
    }
}

fn encoded_size<T: Serialize>(value: &T, max: usize) -> Result<usize, OwnerReadError> {
    struct Counter {
        size: usize,
        max: usize,
    }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let size = self
                .size
                .checked_add(bytes.len())
                .filter(|n| *n <= self.max)
                .ok_or_else(|| io::Error::other("owner read envelope limit"))?;
            self.size = size;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut out = Counter { size: 0, max };
    serde_json::to_writer(&mut out, value).map_err(|_| OwnerReadError::Limit)?;
    Ok(out.size)
}

#[cfg(test)]
mod tests;

/// Test-only: a read lens over `owner`'s live typed state for `caller`, for
/// the unit tests of other core modules that read through the owner-read lens
/// (17g C2a's `ShellReservations::open_request`).
#[cfg(test)]
impl<'a> OwnerReadContext<'a> {
    pub(crate) fn for_test(
        world: &'a World,
        lens: TypedStateLens<'a>,
        owner: &'a str,
        caller: &'a OwnerReadCaller,
    ) -> Self {
        Self {
            world,
            typed: ModuleActionState::for_module(lens, Some(owner)),
            caller,
            reads: Cell::new(0),
            rows: Cell::new(0),
            exhausted: Cell::new(false),
        }
    }
}
