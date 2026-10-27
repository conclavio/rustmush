//! Registered, immutable preparation of legacy-owner images for one commit.
//!
//! Registration is available only inside the actual module-composition callback.
//! A handle is bound to that composition and one exact action provider. Neither
//! a request nor an owner image is a client payload or a persistence callback.

use std::any::{Any, TypeId};
use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::World;
use crate::action::{ActionInvocation, ActionProviderEntry, ActionProviderRegistry, CommitToken};
use crate::authoritative::{
    MAX_AUTHORITATIVE_BUNDLE_BYTES, MAX_AUTHORITATIVE_REPLAY_WRITES, ReplayWrite,
};
use crate::mechanics::{
    DefinitionDigest, DefinitionRef, DefinitionRegistry, MechanicalOwnerReadiness, OwnerId,
};
use crate::module::{ErasedSlotCapture, ModuleStateMap, StateInit};

/// The full optional roster has thirteen adapters: nine commerce, hardware and IC
/// adapters, underwriting publication, Conduct quota/artifact adapters, and
/// TacSim's surface black box (doc 23 cut S.1).
/// Registration headroom does not expand any invocation's images or modules;
/// adapters sharing an actual state slot still require disjoint full action refs.
pub const MAX_OWNER_PARTICIPANTS: usize = 16;
/// An invocation binds at most three distinct slots belonging to two modules.
pub const MAX_OWNER_PARTICIPANT_IMAGES: usize = 3;
pub const MAX_OWNER_PARTICIPANT_MODULES: usize = 2;
/// Registration requests are bounded independently of each invocation's images.
/// The same ceiling bounds two things at composition: the number of
/// participant requests declared (`ParticipantRegistryBuilder::request`) and
/// the size of each registration's `allowed_actions` set
/// (`ParticipantRegistryBuilder::register`).
/// - 17f set it at 128, when the full optional commerce roster used 60 exact
///   grants and IC added eight.
/// - 17g C1c raised it to 256: the named launch's format-2 Active composition
///   (the House chain, Conduct, governed hardware and the whole player market)
///   overran 128. It declared 156 requests when C1c landed (format 1: 91), a
///   measurement no test pins, so a later owner moves that margin silently
///   until a boot hits the ceiling.
///
/// This declaration ceiling leaves bounded room for later owner actions;
/// ordinary invocations remain three slots/two modules, and only the validated
/// fixed IC protocol may prepare its exact four slots/four modules.
pub const MAX_OWNER_PARTICIPANT_REQUESTS: usize = 256;

/// A refusal before publication, or an integrity failure at post-receipt bind.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ParticipantError {
    #[error("owner participants are unavailable on this preparation path")]
    Unavailable,
    #[error("owner participant registration is invalid")]
    Registration,
    #[error("owner participant registration exceeds the composition bound")]
    RegistrationLimit,
    #[error("owner participant handle does not belong to this exact provider/composition")]
    Authority,
    #[error("owner participant or requester is not ready")]
    NotReady,
    #[error("a commit may prepare an owner slot only once")]
    DuplicateSlot,
    #[error("owner participant slot is missing, replaced or has a different type")]
    Slot,
    #[error("owner participant request or patch has a different registered type")]
    Type,
    #[error("owner participant write is outside its registered exact families")]
    WriteFamily,
    #[error("owner participant images exceed the authoritative write/byte bound")]
    ImageLimit,
    #[error("owner participant image encoding failed")]
    Encoding,
    #[error("owner participant refused preparation: {0}")]
    Refused(Box<str>),
}

impl ParticipantError {
    /// Keep an owner's diagnostic bounded; source text is never appropriate here.
    pub fn refused(message: &str) -> Self {
        let end = message
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|i| *i <= 256)
            .last()
            .unwrap_or(0);
        let bounded = if message.len() <= 256 {
            message
        } else {
            message.get(..end).unwrap_or("owner refused")
        };
        Self::Refused(bounded.into())
    }
}

/// Opaque identity of one actual authenticated owner preparation. This is only
/// comparable by Core; it grants no owner permission and records no freshness.
/// Capturing it never permits constructing or altering an invocation.
pub struct OwnerPreparationBinding {
    invocation: ActionInvocation,
    action: DefinitionRef,
    commit: CommitToken,
}

impl OwnerPreparationBinding {
    fn matches(
        &self,
        action: &DefinitionRef,
        invocation: &ActionInvocation,
        commit: CommitToken,
    ) -> bool {
        self.action == *action && self.invocation == *invocation && self.commit == commit
    }
}

/// Read access during one synchronous owner preparation/recheck. Current
/// authority must be explicitly rechecked: borrowing World does not track reads.
pub struct OwnerPrepareContext<'a> {
    world: &'a World,
    invocation: &'a ActionInvocation,
    action: &'a DefinitionRef,
    commit_token: CommitToken,
}

impl<'a> OwnerPrepareContext<'a> {
    /// Capture the complete actual invocation, full action reference and commit.
    /// A consumer can only ask Core to compare this with another actual context.
    #[must_use]
    pub fn preparation_binding(&self) -> OwnerPreparationBinding {
        OwnerPreparationBinding {
            invocation: self.invocation.clone(),
            action: self.action.clone(),
            commit: self.commit_token,
        }
    }

    #[must_use]
    pub fn matches_preparation(&self, binding: &OwnerPreparationBinding) -> bool {
        binding.matches(self.action, self.invocation, self.commit_token)
    }

    #[must_use]
    pub const fn world(&self) -> &'a World {
        self.world
    }
    #[must_use]
    pub const fn invocation(&self) -> &'a ActionInvocation {
        self.invocation
    }
    #[must_use]
    pub const fn action(&self) -> &'a DefinitionRef {
        self.action
    }
    #[must_use]
    pub const fn commit_token(&self) -> CommitToken {
        self.commit_token
    }
}

/// Trusted owner implementation. Preparation/recheck cannot publish, and bind
/// may only validate and borrow. Dropping a bound value must leave State intact.
pub trait OwnerParticipant: Send + Sync + 'static {
    type State: Any + Send;
    type Request: Send + 'static;
    type Output: Send + 'static;
    type Patch: Send + 'static;
    type Bound<'a>: BoundParticipant + 'a
    where
        Self: 'a;

    fn prepare(
        &self,
        state: &Self::State,
        context: &OwnerPrepareContext<'_>,
        request: Self::Request,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError>;
    fn recheck(
        &self,
        state: &Self::State,
        context: &OwnerPrepareContext<'_>,
        patch: &Self::Patch,
    ) -> Result<(), ParticipantError>;
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut Self::State,
        patch: Self::Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError>;

    /// Parse exact keys and kinds owned by this adapter. Prefix-only approval is
    /// not an exact codec. Core independently checks namespace and forbids objects.
    fn accepts_write(&self, write: &ReplayWrite) -> bool;
}

/// No error/serialization/lookup may occur after the first participant applies.
pub trait BoundParticipant {
    fn apply(self);
}

/// An owner may build images, but only its registered adapter can give them to
/// the collector. Callers cannot replace the output, patch or replay writes.
pub struct OwnerPreparation<O, P> {
    output: O,
    patch: P,
    writes: Vec<ReplayWrite>,
}

impl<O, P> OwnerPreparation<O, P> {
    #[must_use]
    pub fn new(output: O, patch: P, writes: Vec<ReplayWrite>) -> Self {
        Self {
            output,
            patch,
            writes,
        }
    }
}

/// Opaque registration-time capability for one exact invoking action.
pub struct ParticipantHandle<P: OwnerParticipant> {
    composition: Arc<()>,
    action: DefinitionRef,
    requester: OwnerId,
    marker: PhantomData<fn() -> P>,
}

impl<P: OwnerParticipant> Clone for ParticipantHandle<P> {
    fn clone(&self) -> Self {
        Self {
            composition: Arc::clone(&self.composition),
            action: self.action.clone(),
            requester: self.requester.clone(),
            marker: PhantomData,
        }
    }
}

impl<P: OwnerParticipant> fmt::Debug for ParticipantHandle<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParticipantHandle")
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

type ErasedValue = Box<dyn Any + Send>;
struct ErasedPreparation {
    output: ErasedValue,
    patch: ErasedValue,
    writes: Vec<ReplayWrite>,
}

trait ErasedAdapter: Send + Sync {
    fn prepare(
        &self,
        state: &(dyn Any + Send),
        context: &OwnerPrepareContext<'_>,
        request: ErasedValue,
    ) -> Result<ErasedPreparation, ParticipantError>;
    fn recheck(
        &self,
        state: &(dyn Any + Send),
        context: &OwnerPrepareContext<'_>,
        patch: &(dyn Any + Send),
    ) -> Result<(), ParticipantError>;
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut (dyn Any + Send),
        patch: ErasedValue,
    ) -> Result<Box<dyn ErasedBound + 'a>, ParticipantError>;
    fn accepts_write(&self, write: &ReplayWrite) -> bool;
}

struct Registered<P>(Arc<P>);
impl<P: OwnerParticipant> ErasedAdapter for Registered<P> {
    fn prepare(
        &self,
        state: &(dyn Any + Send),
        context: &OwnerPrepareContext<'_>,
        request: ErasedValue,
    ) -> Result<ErasedPreparation, ParticipantError> {
        let state = state
            .downcast_ref::<P::State>()
            .ok_or(ParticipantError::Type)?;
        let request = request
            .downcast::<P::Request>()
            .map_err(|_| ParticipantError::Type)?;
        let prepared = self.0.prepare(state, context, *request)?;
        Ok(ErasedPreparation {
            output: Box::new(prepared.output),
            patch: Box::new(prepared.patch),
            writes: prepared.writes,
        })
    }
    fn recheck(
        &self,
        state: &(dyn Any + Send),
        context: &OwnerPrepareContext<'_>,
        patch: &(dyn Any + Send),
    ) -> Result<(), ParticipantError> {
        self.0.recheck(
            state
                .downcast_ref::<P::State>()
                .ok_or(ParticipantError::Type)?,
            context,
            patch
                .downcast_ref::<P::Patch>()
                .ok_or(ParticipantError::Type)?,
        )
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut (dyn Any + Send),
        patch: ErasedValue,
    ) -> Result<Box<dyn ErasedBound + 'a>, ParticipantError> {
        let state = state
            .downcast_mut::<P::State>()
            .ok_or(ParticipantError::Type)?;
        let patch = patch
            .downcast::<P::Patch>()
            .map_err(|_| ParticipantError::Type)?;
        let bound = Arc::clone(&self.0).bind(state, *patch)?;
        Ok(Box::new(BoundValue(bound)))
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        self.0.accepts_write(write)
    }
}

pub(crate) trait ErasedBound {
    fn apply(self: Box<Self>);
}
struct BoundValue<B>(B);
impl<B: BoundParticipant> ErasedBound for BoundValue<B> {
    fn apply(self: Box<Self>) {
        self.0.apply();
    }
}

struct Registration {
    adapter_type: TypeId,
    state_type: TypeId,
    state_name: &'static str,
    owner: OwnerId,
    definition: DefinitionRef,
    allowed_actions: BTreeSet<DefinitionRef>,
    adapter: Arc<dyn ErasedAdapter>,
}
struct RequestRegistration {
    adapter_type: TypeId,
    requester: OwnerId,
    action: DefinitionRef,
}

/// Only Registrar creates registrations/requests; the builder is core-private.
#[derive(Default)]
pub(crate) struct ParticipantRegistryBuilder {
    composition: Arc<()>,
    registrations: Vec<Registration>,
    requests: Vec<RequestRegistration>,
}

impl ParticipantRegistryBuilder {
    pub(crate) fn register<P: OwnerParticipant>(
        &mut self,
        owner: OwnerId,
        definition: DefinitionRef,
        allowed_actions: BTreeSet<DefinitionRef>,
        adapter: P,
    ) -> Result<(), ParticipantError> {
        if self.registrations.len() >= MAX_OWNER_PARTICIPANTS
            || allowed_actions.len() > MAX_OWNER_PARTICIPANT_REQUESTS
        {
            return Err(ParticipantError::RegistrationLimit);
        }
        if definition.key().owner != owner
            || allowed_actions.is_empty()
            || self.registrations.iter().any(|r| {
                r.adapter_type == TypeId::of::<P>()
                    || r.definition == definition
                    || (r.state_type == TypeId::of::<P::State>()
                        && (r.owner != owner || !r.allowed_actions.is_disjoint(&allowed_actions)))
            })
        {
            return Err(ParticipantError::Registration);
        }
        self.registrations.push(Registration {
            adapter_type: TypeId::of::<P>(),
            state_type: TypeId::of::<P::State>(),
            state_name: std::any::type_name::<P::State>(),
            owner,
            definition,
            allowed_actions,
            adapter: Arc::new(Registered(Arc::new(adapter))),
        });
        Ok(())
    }

    pub(crate) fn request<P: OwnerParticipant>(
        &mut self,
        requester: OwnerId,
        action: DefinitionRef,
    ) -> Result<ParticipantHandle<P>, ParticipantError> {
        if self.requests.len() >= MAX_OWNER_PARTICIPANT_REQUESTS {
            return Err(ParticipantError::RegistrationLimit);
        }
        if action.key().owner != requester
            || self
                .requests
                .iter()
                .any(|r| r.adapter_type == TypeId::of::<P>() && r.action == action)
        {
            return Err(ParticipantError::Registration);
        }
        self.requests.push(RequestRegistration {
            adapter_type: TypeId::of::<P>(),
            requester: requester.clone(),
            action: action.clone(),
        });
        Ok(ParticipantHandle {
            composition: Arc::clone(&self.composition),
            action,
            requester,
            marker: PhantomData,
        })
    }

    pub(crate) fn freeze(
        mut self,
        definitions: &DefinitionRegistry,
        providers: &ActionProviderRegistry,
        slots: &[StateInit],
        readiness: MechanicalOwnerReadiness,
    ) -> Result<ParticipantRegistry, ParticipantError> {
        for r in &self.registrations {
            definitions
                .get_exact(&r.definition)
                .map_err(|_| ParticipantError::Registration)?;
            if !slots
                .iter()
                .any(|s| s.type_id() == r.state_type && s.module().as_str() == r.owner.as_str())
            {
                return Err(ParticipantError::Registration);
            }
            for action in &r.allowed_actions {
                providers
                    .get_exact(action)
                    .map_err(|_| ParticipantError::Registration)?;
            }
        }
        for request in &self.requests {
            let r = self
                .registrations
                .iter()
                .find(|r| r.adapter_type == request.adapter_type)
                .ok_or(ParticipantError::Registration)?;
            let provider = providers
                .get_exact(&request.action)
                .map_err(|_| ParticipantError::Registration)?;
            if provider.owner() != &request.requester
                || !r.allowed_actions.contains(&request.action)
            {
                return Err(ParticipantError::Authority);
            }
        }
        self.registrations
            .sort_by(|a, b| a.definition.cmp(&b.definition));
        let registrations = &self.registrations;
        self.requests.sort_by_key(|q| {
            (
                q.requester.clone(),
                q.action.clone(),
                registrations
                    .iter()
                    .find(|r| r.adapter_type == q.adapter_type)
                    .map(|r| r.definition.clone()),
            )
        });
        let fingerprint = DefinitionDigest::for_canonical_value("owner_participants.v1", &serde_json::json!({
            "registrations":self.registrations.iter().map(|r|serde_json::json!({
                "owner":r.owner,"definition":r.definition,"state":r.state_name,"callers":r.allowed_actions
            })).collect::<Vec<_>>(),
            "requests":self.requests.iter().map(|q| {
                let definition=self.registrations.iter().find(|r|r.adapter_type==q.adapter_type).map(|r|&r.definition);
                serde_json::json!({"requester":q.requester,"action":q.action,"participant":definition})
            }).collect::<Vec<_>>()
        })).map_err(|_| ParticipantError::Encoding)?;
        Ok(ParticipantRegistry {
            composition: self.composition,
            registrations: self.registrations,
            requests: self.requests,
            readiness,
            fingerprint,
            ic_claim: None,
        })
    }
}

pub(crate) struct ParticipantRegistry {
    composition: Arc<()>,
    registrations: Vec<Registration>,
    requests: Vec<RequestRegistration>,
    readiness: MechanicalOwnerReadiness,
    fingerprint: DefinitionDigest,
    ic_claim: Option<([DefinitionRef; 2], [DefinitionRef; 4])>,
}

impl ParticipantRegistry {
    pub(crate) fn enable_ic_claim(
        &mut self,
        actions: [DefinitionRef; 2],
        participants: [DefinitionRef; 4],
    ) -> Result<(), ParticipantError> {
        if self.ic_claim.is_some() {
            return Err(ParticipantError::Registration);
        }
        self.fingerprint = DefinitionDigest::for_canonical_value(
            "owner_participants.v2",
            &serde_json::json!({
                "base": self.fingerprint, "ic_actions": actions, "ic_participants": participants
            }),
        )
        .map_err(|_| ParticipantError::Encoding)?;
        self.ic_claim = Some((actions, participants));
        Ok(())
    }
    /// Validate the actual registered adapter and granted request, not merely a
    /// participant-shaped definition. Used only by the closed core protocol.
    pub(crate) fn grants_exact(
        &self,
        definition: &DefinitionRef,
        requester: &OwnerId,
        action: &DefinitionRef,
    ) -> bool {
        self.registrations.iter().any(|registration| {
            registration.definition == *definition
                && registration.allowed_actions.contains(action)
                && self.requests.iter().any(|request| {
                    request.adapter_type == registration.adapter_type
                        && request.requester == *requester
                        && request.action == *action
                })
        })
    }
}

pub(crate) struct ParticipantPreparation<'a> {
    registry: &'a ParticipantRegistry,
    provider: &'a ActionProviderEntry,
    world: &'a World,
    images: &'a mut OwnerImages,
    shell: &'a mut Option<crate::core_output::image::BuiltinObjects>,
    invocation: &'a ActionInvocation,
}
impl<'a> ParticipantPreparation<'a> {
    pub(crate) fn matches_preparation(
        &self,
        binding: &OwnerPreparationBinding,
        commit: CommitToken,
    ) -> bool {
        binding.matches(self.provider.action(), self.invocation, commit)
    }

    pub(crate) fn world(&self) -> &World {
        self.world
    }
    pub(crate) fn new(
        registry: &'a ParticipantRegistry,
        provider: &'a ActionProviderEntry,
        world: &'a World,
        images: &'a mut OwnerImages,
        shell: &'a mut Option<crate::core_output::image::BuiltinObjects>,
        invocation: &'a ActionInvocation,
    ) -> Self {
        Self {
            registry,
            provider,
            world,
            images,
            shell,
            invocation,
        }
    }
    pub(crate) fn prepare<P: OwnerParticipant>(
        &mut self,
        handle: &ParticipantHandle<P>,
        commit_token: CommitToken,
        request: P::Request,
    ) -> Result<P::Output, ParticipantError> {
        if self
            .shell
            .as_ref()
            .is_some_and(|shell| shell.creates_object())
        {
            return Err(ParticipantError::Authority);
        }
        let context = OwnerPrepareContext {
            world: self.world,
            invocation: self.invocation,
            action: self.provider.action(),
            commit_token,
        };
        self.registry
            .prepare(handle, self.provider, &context, self.images, request)
    }

    pub(crate) fn initialize_core_outputs<P: crate::core_output::CoreOutputPolicy>(
        &mut self,
        service: &crate::core_output::CoreOutputReservations<P>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        commit: CommitToken,
        facts: crate::core_output::OutputPolicyFacts,
    ) -> Result<(), ParticipantError> {
        if self.shell.is_some() || !self.images.is_empty() {
            return Err(ParticipantError::Authority);
        }
        let image = service.initialize(
            self.world,
            self.invocation,
            self.provider.action(),
            commit,
            txn,
            facts,
        )?;
        *self.shell = Some(crate::core_output::image::BuiltinObjects::CoreOutput(
            Box::new(image),
        ));
        Ok(())
    }
    pub(crate) fn reserve_core_output<P: crate::core_output::CoreOutputPolicy>(
        &mut self,
        service: &crate::core_output::CoreOutputReservations<P>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        commit: CommitToken,
        order: u64,
        facts: crate::core_output::OutputPolicyFacts,
    ) -> Result<crate::core_output::ReservedCoreOutput, ParticipantError> {
        if self.shell.is_some() || !self.images.is_empty() {
            return Err(ParticipantError::Authority);
        }
        let (output, image) = service.reserve(
            self.world,
            self.invocation,
            self.provider.action(),
            commit,
            txn,
            order,
            facts,
        )?;
        *self.shell = Some(crate::core_output::image::BuiltinObjects::CoreOutput(
            Box::new(image),
        ));
        Ok(output)
    }
    pub(crate) fn consume_core_output<P: crate::core_output::CoreOutputPolicy>(
        &mut self,
        service: &crate::core_output::CoreOutputReservations<P>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        commit: CommitToken,
        order: u64,
        facts: crate::core_output::OutputPolicyFacts,
    ) -> Result<crate::core_output::ReservedCoreOutput, ParticipantError> {
        if self.shell.is_some() {
            return Err(ParticipantError::Authority);
        }
        let (output, image) = service.consume(
            self.world,
            self.invocation,
            self.provider.action(),
            commit,
            txn,
            order,
            facts,
        )?;
        *self.shell = Some(crate::core_output::image::BuiltinObjects::CoreOutput(
            Box::new(image),
        ));
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn deliver_core_output<
        P: crate::core_output::CoreOutputPolicy,
        D: crate::core_delivery::CoreDeliveryPolicy,
    >(
        &mut self,
        service: &crate::core_delivery::CoreOutputDeliveries<P, D>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        commit: CommitToken,
        order: u64,
        facts: crate::core_delivery::DeliveryFacts,
        hooks: std::sync::Arc<[crate::module::HookReg]>,
        base: &crate::action_state::ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<crate::core_delivery::DeliveredCoreOutput, ParticipantError> {
        if self.shell.is_some() {
            return Err(ParticipantError::Authority);
        }
        let (output, image) = service.prepare(
            self.world,
            self.invocation,
            self.provider.action(),
            commit,
            txn,
            order,
            facts,
            hooks,
            base,
            provider,
        )?;
        *self.shell = Some(crate::core_output::image::BuiltinObjects::Delivery(
            Box::new(image),
        ));
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn exchange_core_component<P: crate::core_exchange::CoreExchangePolicy>(
        &mut self,
        service: &crate::core_exchange::CoreComponentExchanges<P>,
        txn: &mut crate::action_state::ActionTxn<'_>,
        commit: CommitToken,
        order: u64,
        facts: crate::core_exchange::ExchangeFacts,
        hooks: std::sync::Arc<[crate::module::HookReg]>,
        base: &crate::action_state::ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<crate::core_exchange::ExchangedCoreComponent, ParticipantError> {
        if self.shell.is_some() {
            return Err(ParticipantError::Authority);
        }
        self.images.require_exact_exchange(
            service.participants(),
            self.provider.action(),
            commit,
        )?;
        if service.participants().iter().any(|p| {
            !self
                .registry
                .grants_exact(p, self.provider.owner(), self.provider.action())
        }) {
            return Err(ParticipantError::Authority);
        }
        let (output, image) = service.prepare(
            self.world,
            self.invocation,
            self.provider.action(),
            commit,
            txn,
            order,
            facts,
            hooks,
            base,
            provider,
        )?;
        *self.shell = Some(crate::core_output::image::BuiltinObjects::Exchange(
            Box::new(image),
        ));
        Ok(output)
    }

    pub(crate) fn reserve_shell(
        &mut self,
        service: &crate::reserved_shell::ShellReservations,
        txn: &mut crate::action_state::ActionTxn<'_>,
        commit: CommitToken,
    ) -> Result<(crate::reserved_shell::ShellKey, crate::ObjId), ParticipantError> {
        if self.shell.is_some() || !self.images.is_empty() {
            return Err(ParticipantError::Authority);
        }
        let (key, object, shell) = service.prepare_reserve(
            self.world,
            self.invocation,
            self.provider.action(),
            commit,
            txn,
        )?;
        *self.shell = Some(shell.into());
        Ok((key, object))
    }

    pub(crate) fn initialize_shells(
        &mut self,
        service: &crate::reserved_shell::ShellReservations,
        txn: &mut crate::action_state::ActionTxn<'_>,
        commit: CommitToken,
    ) -> Result<(), ParticipantError> {
        if self.shell.is_some() || !self.images.is_empty() {
            return Err(ParticipantError::Authority);
        }
        *self.shell = Some(
            service
                .initialize(
                    self.world,
                    self.invocation,
                    self.provider.action(),
                    commit,
                    txn,
                )?
                .into(),
        );
        Ok(())
    }

    pub(crate) fn resolve_shell(
        &mut self,
        service: &crate::reserved_shell::ShellReservations,
        txn: &mut crate::action_state::ActionTxn<'_>,
        key: crate::reserved_shell::ShellKey,
    ) -> Result<crate::reserved_shell::ReservedShell, ParticipantError> {
        service.resolve(
            self.world,
            self.invocation,
            self.provider.action(),
            txn,
            key,
        )
    }

    pub(crate) fn consume_shell(
        &mut self,
        service: &crate::reserved_shell::ShellReservations,
        txn: &mut crate::action_state::ActionTxn<'_>,
        shell: crate::reserved_shell::ReservedShell,
        commit: CommitToken,
    ) -> Result<(), ParticipantError> {
        if self.shell.is_some() {
            return Err(ParticipantError::Authority);
        }
        *self.shell = Some(
            service
                .consume(
                    self.world,
                    self.invocation,
                    self.provider.action(),
                    txn,
                    shell,
                    commit,
                )?
                .into(),
        );
        Ok(())
    }
}

impl ParticipantRegistry {
    pub(crate) fn is_empty(&self) -> bool {
        self.registrations.is_empty() && self.requests.is_empty()
    }
    pub(crate) const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }
    pub(crate) fn prepare<P: OwnerParticipant>(
        &self,
        handle: &ParticipantHandle<P>,
        provider: &ActionProviderEntry,
        context: &OwnerPrepareContext<'_>,
        images: &mut OwnerImages,
        request: P::Request,
    ) -> Result<P::Output, ParticipantError> {
        if !Arc::ptr_eq(&handle.composition, &self.composition)
            || handle.action != *provider.action()
            || handle.requester != *provider.owner()
            || !provider
                .registration_ordinal()
                .owner_readiness()
                .shares_latch_with(&self.readiness)
            || !self
                .requests
                .iter()
                .any(|q| q.adapter_type == TypeId::of::<P>() && q.action == handle.action)
        {
            return Err(ParticipantError::Authority);
        }
        let r = self
            .registrations
            .iter()
            .find(|r| r.adapter_type == TypeId::of::<P>())
            .ok_or(ParticipantError::Registration)?;
        if images
            .0
            .iter()
            .any(|image| image.capture.type_id() == r.state_type)
        {
            return Err(ParticipantError::DuplicateSlot);
        }
        let ic = self
            .ic_claim
            .as_ref()
            .filter(|(actions, _)| actions.contains(provider.action()));
        if let Some((_, participants)) = ic
            && (!participants.contains(&r.definition)
                || images.0.iter().any(|image| {
                    !participants.contains(&image.definition) || image.action != *provider.action()
                }))
        {
            return Err(ParticipantError::Authority);
        }
        let (max_images, max_modules) = if ic.is_some() {
            (4, 4)
        } else {
            (MAX_OWNER_PARTICIPANT_IMAGES, MAX_OWNER_PARTICIPANT_MODULES)
        };
        if images.0.len() >= max_images
            || images
                .0
                .iter()
                .map(|image| &image.owner)
                .chain(std::iter::once(&r.owner))
                .collect::<BTreeSet<_>>()
                .len()
                > max_modules
        {
            return Err(ParticipantError::ImageLimit);
        }
        self.readiness
            .require_ready(&r.owner)
            .map_err(|_| ParticipantError::NotReady)?;
        self.readiness
            .require_ready(provider.owner())
            .map_err(|_| ParticipantError::NotReady)?;
        let (capture, state) = context
            .world
            .participant_slot(r.state_type)
            .ok_or(ParticipantError::Slot)?;
        let prepared = r.adapter.prepare(state, context, Box::new(request))?;
        if prepared.writes.is_empty() || prepared.writes.len() > MAX_AUTHORITATIVE_REPLAY_WRITES {
            return Err(ParticipantError::ImageLimit);
        }
        for write in &prepared.writes {
            let namespace = match write {
                ReplayWrite::ModuleJsonSet { namespace, .. }
                | ReplayWrite::ModuleDelete { namespace, .. } => namespace,
                ReplayWrite::LedgerChunkSet { write } => write.namespace(),
                _ => return Err(ParticipantError::WriteFamily),
            };
            if namespace != &r.owner || !r.adapter.accepts_write(write) {
                return Err(ParticipantError::WriteFamily);
            }
        }
        let length = crate::mechanics::canonical_json_bytes_with_limit(
            &serde_json::to_value(&prepared.writes).map_err(|_| ParticipantError::Encoding)?,
            MAX_AUTHORITATIVE_BUNDLE_BYTES,
        )
        .map_err(|_| ParticipantError::Encoding)?
        .len();
        if length > MAX_AUTHORITATIVE_BUNDLE_BYTES {
            return Err(ParticipantError::ImageLimit);
        }
        let output = prepared
            .output
            .downcast::<P::Output>()
            .map_err(|_| ParticipantError::Type)?;
        // An owner may fail readiness while its pure callback executes.
        self.readiness
            .require_ready(&r.owner)
            .map_err(|_| ParticipantError::NotReady)?;
        self.readiness
            .require_ready(provider.owner())
            .map_err(|_| ParticipantError::NotReady)?;
        images.0.push(OwnerImage {
            capture,
            owner: r.owner.clone(),
            definition: r.definition.clone(),
            action: context.action.clone(),
            invocation: context.invocation.clone(),
            commit_token: context.commit_token,
            adapter: Arc::clone(&r.adapter),
            patch: prepared.patch,
            writes: prepared.writes,
            readiness: self.readiness.clone(),
        });
        Ok(*output)
    }
}

struct OwnerImage {
    capture: ErasedSlotCapture,
    owner: OwnerId,
    definition: DefinitionRef,
    action: DefinitionRef,
    invocation: ActionInvocation,
    commit_token: CommitToken,
    adapter: Arc<dyn ErasedAdapter>,
    patch: ErasedValue,
    writes: Vec<ReplayWrite>,
    readiness: MechanicalOwnerReadiness,
}

/// A collector never escapes by itself: the typed reservation is its one owner.
#[derive(Default)]
pub(crate) struct OwnerImages(Vec<OwnerImage>);

impl fmt::Debug for OwnerImages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|image| &image.definition))
            .finish()
    }
}

impl OwnerImages {
    pub(crate) fn require_exact_exchange(
        &self,
        participants: &[DefinitionRef; 2],
        action: &DefinitionRef,
        commit: CommitToken,
    ) -> Result<(), ParticipantError> {
        if self.0.len() != 2
            || self
                .0
                .iter()
                .any(|image| image.action != *action || image.commit_token != commit)
            || participants.iter().any(|reference| {
                self.0
                    .iter()
                    .filter(|image| &image.definition == reference)
                    .count()
                    != 1
            })
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
    pub(crate) fn require_exact_conduct(
        &self,
        participants: &[DefinitionRef; 3],
        action: &DefinitionRef,
        commit: CommitToken,
    ) -> Result<(), ParticipantError> {
        if self.0.len() != 3
            || self
                .0
                .iter()
                .any(|image| image.action != *action || image.commit_token != commit)
            || participants.iter().any(|reference| {
                self.0
                    .iter()
                    .filter(|image| &image.definition == reference)
                    .count()
                    != 1
            })
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
    pub(crate) fn require_exact_ic(
        &self,
        participants: &[DefinitionRef; 4],
        action: &DefinitionRef,
        commit: CommitToken,
    ) -> Result<(), ParticipantError> {
        if self.0.len() != 4
            || self
                .0
                .iter()
                .any(|image| image.action != *action || image.commit_token != commit)
            || participants.iter().any(|reference| {
                self.0
                    .iter()
                    .filter(|image| &image.definition == reference)
                    .count()
                    != 1
            })
        {
            return Err(ParticipantError::Authority);
        }
        Ok(())
    }
    pub(crate) fn order(&mut self) {
        self.0.sort_by(|a, b| a.definition.cmp(&b.definition));
    }
    pub(crate) fn definitions(&self) -> impl Iterator<Item = &DefinitionRef> {
        self.0.iter().map(|image| &image.definition)
    }
    pub(crate) fn writes(&self) -> impl Iterator<Item = &ReplayWrite> {
        self.0.iter().flat_map(|image| &image.writes)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn recheck(&self, world: &World) -> Result<(), ParticipantError> {
        for image in &self.0 {
            let require_ready = || {
                image
                    .readiness
                    .require_ready(&image.owner)
                    .and_then(|()| image.readiness.require_ready(&image.action.key().owner))
                    .map_err(|_| ParticipantError::NotReady)
            };
            require_ready()?;
            let state = world
                .participant_captured_slot(&image.capture)
                .ok_or(ParticipantError::Slot)?;
            let context = OwnerPrepareContext {
                world,
                invocation: &image.invocation,
                action: &image.action,
                commit_token: image.commit_token,
            };
            image
                .adapter
                .recheck(state, &context, image.patch.as_ref())?;
            // The adapter may discover integrity drift while checking. Its
            // one-way latch and the requester's latch must still be ready at
            // handoff, even when the callback itself returned success.
            require_ready()?;
        }
        Ok(())
    }
    /// Every slot and owner binds before any apply. A failed later bind drops
    /// every earlier binding without changing it. No reference crosses an await.
    pub(crate) fn bind(
        self,
        slots: &mut ModuleStateMap,
    ) -> Result<BoundOwners<'_>, ParticipantError> {
        let readiness: Vec<_> = self
            .0
            .iter()
            .map(|image| {
                (
                    image.readiness.clone(),
                    image.owner.clone(),
                    image.action.key().owner.clone(),
                )
            })
            .collect();
        let require_ready = || {
            for (latch, owner, requester) in &readiness {
                latch
                    .require_ready(owner)
                    .and_then(|()| latch.require_ready(requester))
                    .map_err(|_| ParticipantError::NotReady)?;
            }
            Ok::<(), ParticipantError>(())
        };
        require_ready()?;
        let mut rows = self.0.into_iter();
        let first = rows.next();
        let second = rows.next();
        let third = rows.next();
        let fourth = rows.next();
        if rows.next().is_some() {
            return Err(ParticipantError::ImageLimit);
        }
        let bound = match (first, second, third, fourth) {
            (None, None, None, None) => Ok(BoundOwners::None),
            (Some(a), None, None, None) => {
                a.readiness
                    .require_ready(&a.owner)
                    .map_err(|_| ParticipantError::NotReady)?;
                let state = slots
                    .get_erased_captured_mut(&a.capture)
                    .map_err(|_| ParticipantError::Slot)?;
                Ok(BoundOwners::One(a.adapter.bind(state, a.patch)?))
            }
            (Some(a), Some(b), None, None) => {
                a.readiness
                    .require_ready(&a.owner)
                    .map_err(|_| ParticipantError::NotReady)?;
                b.readiness
                    .require_ready(&b.owner)
                    .map_err(|_| ParticipantError::NotReady)?;
                let (left, right) = slots
                    .get_erased_captured_pair_mut(&a.capture, &b.capture)
                    .map_err(|_| ParticipantError::Slot)?;
                let first = a.adapter.bind(left, a.patch)?;
                let second = b.adapter.bind(right, b.patch)?;
                Ok(BoundOwners::Two(first, second))
            }
            (Some(a), Some(b), Some(c), None) => {
                for image in [&a, &b, &c] {
                    image
                        .readiness
                        .require_ready(&image.owner)
                        .map_err(|_| ParticipantError::NotReady)?;
                }
                let (left, middle, right) = slots
                    .get_erased_captured_triple_mut(&a.capture, &b.capture, &c.capture)
                    .map_err(|_| ParticipantError::Slot)?;
                let first = a.adapter.bind(left, a.patch)?;
                let second = b.adapter.bind(middle, b.patch)?;
                let third = c.adapter.bind(right, c.patch)?;
                Ok(BoundOwners::Three(first, second, third))
            }
            (Some(a), Some(b), Some(c), Some(d)) => {
                let (one, two, three, four) = slots
                    .get_erased_captured_four_mut(&a.capture, &b.capture, &c.capture, &d.capture)
                    .map_err(|_| ParticipantError::Slot)?;
                let a = a.adapter.bind(one, a.patch)?;
                let b = b.adapter.bind(two, b.patch)?;
                let c = c.adapter.bind(three, c.patch)?;
                let d = d.adapter.bind(four, d.patch)?;
                Ok(BoundOwners::Four(a, b, c, d))
            }
            _ => Err(ParticipantError::Type),
        }?;
        // An integrity-discovering bind may close a latch while returning a
        // value. That value must be dropped inert, never applied after closure.
        require_ready()?;
        Ok(bound)
    }
}

pub(crate) enum BoundOwners<'a> {
    Four(
        Box<dyn ErasedBound + 'a>,
        Box<dyn ErasedBound + 'a>,
        Box<dyn ErasedBound + 'a>,
        Box<dyn ErasedBound + 'a>,
    ),
    None,
    One(Box<dyn ErasedBound + 'a>),
    Two(Box<dyn ErasedBound + 'a>, Box<dyn ErasedBound + 'a>),
    Three(
        Box<dyn ErasedBound + 'a>,
        Box<dyn ErasedBound + 'a>,
        Box<dyn ErasedBound + 'a>,
    ),
}
impl BoundOwners<'_> {
    pub(crate) fn apply(self) {
        match self {
            Self::Four(a, b, c, d) => {
                a.apply();
                b.apply();
                c.apply();
                d.apply();
            }
            Self::None => {}
            Self::One(a) => a.apply(),
            Self::Two(a, b) => {
                a.apply();
                b.apply();
            }
            Self::Three(a, b, c) => {
                a.apply();
                b.apply();
                c.apply();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{
        ActionError, ActionInvocationDraft, ActionOfferId, ActionProvider,
        ActionProviderRegistryBuilder, CanonicalHash, IdempotencyKey, IdempotencyScope,
        PerceivedRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
    };
    use crate::mechanics::{
        CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    };
    use crate::{ObjId, Registrar, Registry};
    use serde_json::json;

    #[derive(Clone, Debug, PartialEq)]
    struct Cell<const N: u8>(u64);
    struct Adapter<const N: u8>;
    struct Patch {
        before: u64,
        after: u64,
        refuse: bool,
    }
    struct Bound<'a, const N: u8> {
        cell: &'a mut Cell<N>,
        after: u64,
    }
    impl<const N: u8> BoundParticipant for Bound<'_, N> {
        fn apply(self) {
            self.cell.0 = self.after;
        }
    }
    fn owner<const N: u8>() -> OwnerId {
        OwnerId::new(if N == 1 { "left" } else { "right" }).unwrap()
    }
    impl<const N: u8> OwnerParticipant for Adapter<N> {
        type State = Cell<N>;
        type Request = Patch;
        type Output = u64;
        type Patch = Patch;
        type Bound<'a> = Bound<'a, N>;
        fn prepare(
            &self,
            state: &Cell<N>,
            _: &OwnerPrepareContext<'_>,
            patch: Patch,
        ) -> Result<OwnerPreparation<u64, Patch>, ParticipantError> {
            if state.0 != patch.before {
                return Err(ParticipantError::Slot);
            }
            let writes = vec![ReplayWrite::ModuleJsonSet {
                namespace: owner::<N>(),
                key: crate::AuthoritativeKey::parse_canonical("root").unwrap(),
                value: json!(patch.after),
            }];
            Ok(OwnerPreparation::new(patch.after, patch, writes))
        }
        fn recheck(
            &self,
            state: &Cell<N>,
            _: &OwnerPrepareContext<'_>,
            patch: &Patch,
        ) -> Result<(), ParticipantError> {
            if state.0 == patch.before {
                Ok(())
            } else {
                Err(ParticipantError::Slot)
            }
        }
        fn bind<'a>(
            self: Arc<Self>,
            state: &'a mut Cell<N>,
            patch: Patch,
        ) -> Result<Bound<'a, N>, ParticipantError> {
            if state.0 != patch.before || patch.refuse {
                return Err(ParticipantError::Slot);
            }
            Ok(Bound {
                cell: state,
                after: patch.after,
            })
        }
        fn accepts_write(&self, write: &ReplayWrite) -> bool {
            matches!(write, ReplayWrite::ModuleJsonSet { key, value, .. }
                if key.as_str() == "root" && value.is_u64())
        }
    }
    struct Provider;
    impl ActionProvider for Provider {
        fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
            Ok(())
        }
    }

    #[test]
    fn composition_accepts_256_distinct_requests_and_refuses_257() {
        assert_eq!(MAX_OWNER_PARTICIPANT_REQUESTS, 256);
        let caller = OwnerId::new("writer").unwrap();
        let mut definitions = DefinitionRegistryBuilder::new();
        let mut builder = ParticipantRegistryBuilder::default();
        for index in 0..=MAX_OWNER_PARTICIPANT_REQUESTS {
            let action = definitions
                .register(
                    caller.clone(),
                    DefinitionDescriptor::new(
                        DefinitionKind::new("action").unwrap(),
                        DefinitionId::new(format!("action_{index}")).unwrap(),
                        1,
                        json!({}),
                    )
                    .unwrap(),
                )
                .unwrap();
            let result = builder.request::<Adapter<1>>(caller.clone(), action);
            if index < MAX_OWNER_PARTICIPANT_REQUESTS {
                assert!(result.is_ok());
            } else {
                assert!(matches!(result, Err(ParticipantError::RegistrationLimit)));
            }
        }
        assert_eq!(builder.requests.len(), MAX_OWNER_PARTICIPANT_REQUESTS);
    }

    /// 17g C1c: the ceiling also bounds one registration's `allowed_actions`.
    /// A set of 257 refuses by the limit; the same participant with 256 of
    /// them registers.
    #[test]
    fn registration_accepts_256_allowed_actions_and_refuses_257() {
        let caller = OwnerId::new("writer").unwrap();
        let mut definitions = DefinitionRegistryBuilder::new();
        let mut define = |owner: OwnerId, kind: &str, id: String| {
            definitions
                .register(
                    owner,
                    DefinitionDescriptor::new(
                        DefinitionKind::new(kind).unwrap(),
                        DefinitionId::new(id).unwrap(),
                        1,
                        json!({}),
                    )
                    .unwrap(),
                )
                .unwrap()
        };
        let participant = define(owner::<1>(), "participant", "images".into());
        let actions: Vec<DefinitionRef> = (0..=MAX_OWNER_PARTICIPANT_REQUESTS)
            .map(|index| define(caller.clone(), "action", format!("action_{index}")))
            .collect();
        let over: BTreeSet<DefinitionRef> = actions.iter().cloned().collect();
        assert_eq!(over.len(), MAX_OWNER_PARTICIPANT_REQUESTS + 1);
        let mut builder = ParticipantRegistryBuilder::default();
        assert!(matches!(
            builder.register(owner::<1>(), participant.clone(), over, Adapter::<1>),
            Err(ParticipantError::RegistrationLimit)
        ));
        let at: BTreeSet<DefinitionRef> = actions
            .into_iter()
            .take(MAX_OWNER_PARTICIPANT_REQUESTS)
            .collect();
        assert!(
            builder
                .register(owner::<1>(), participant, at, Adapter::<1>)
                .is_ok()
        );
        assert_eq!(builder.registrations.len(), 1);
    }
    struct Fixture {
        registry: ParticipantRegistry,
        providers: ActionProviderRegistry,
        world: World,
        left: ParticipantHandle<Adapter<1>>,
        right: ParticipantHandle<Adapter<2>>,
        action: DefinitionRef,
        invocation: ActionInvocation,
    }
    fn fixture() -> Fixture {
        let caller = OwnerId::new("writer").unwrap();
        let roster = BTreeSet::from([caller.clone(), owner::<1>(), owner::<2>()]);
        let ready = MechanicalOwnerReadiness::for_owners(&roster);
        let mut definitions = DefinitionRegistryBuilder::new();
        let mut define = |owner, kind, id| {
            definitions
                .register(
                    owner,
                    DefinitionDescriptor::new(
                        DefinitionKind::new(kind).unwrap(),
                        DefinitionId::new(id).unwrap(),
                        1,
                        json!({}),
                    )
                    .unwrap(),
                )
                .unwrap()
        };
        let action = define(caller.clone(), "action", "commit");
        let left_def = define(owner::<1>(), "participant", "images");
        let right_def = define(owner::<2>(), "participant", "images");
        let mut definitions = definitions.freeze().unwrap();
        definitions.bind_owner_readiness(ready.clone());
        let mut providers = ActionProviderRegistryBuilder::new();
        providers
            .register(caller.clone(), action.clone(), Arc::new(Provider))
            .unwrap();
        let providers = providers
            .freeze_for_owners_with_readiness(&definitions, &roster, ready.clone())
            .unwrap();
        let mut states = Registry::new();
        Registrar::new(&mut states, "left").state(Cell::<1>(10));
        Registrar::new(&mut states, "right").state(Cell::<2>(20));
        let inits = states.take_state_inits();
        let mut builder = ParticipantRegistryBuilder::default();
        let left = builder
            .request::<Adapter<1>>(caller.clone(), action.clone())
            .unwrap();
        let right = builder
            .request::<Adapter<2>>(caller.clone(), action.clone())
            .unwrap();
        builder
            .register(
                owner::<1>(),
                left_def,
                BTreeSet::from([action.clone()]),
                Adapter::<1>,
            )
            .unwrap();
        builder
            .register(
                owner::<2>(),
                right_def,
                BTreeSet::from([action.clone()]),
                Adapter::<2>,
            )
            .unwrap();
        let registry = builder
            .freeze(&definitions, &providers, &inits, ready)
            .unwrap();
        let mut world = World::new_minimal();
        for init in inits {
            world.module_state_apply(init);
        }
        let request = RequestUid::from_bytes([1; 16]);
        let principal = PrincipalId::Object { id: ObjId(7) };
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: ObjId(7),
            offered_action: ActionOfferId(CanonicalHash::from_bytes([3; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: caller,
                    namespace: CanonicalId::new("filing").unwrap(),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap();
        Fixture {
            registry,
            providers,
            world,
            left,
            right,
            action,
            invocation,
        }
    }
    impl Fixture {
        fn prepare<P: OwnerParticipant>(
            &self,
            handle: &ParticipantHandle<P>,
            request: P::Request,
            images: &mut OwnerImages,
        ) -> Result<P::Output, ParticipantError> {
            let entry = self.providers.get_exact(&self.action).unwrap();
            let context = OwnerPrepareContext {
                world: &self.world,
                invocation: &self.invocation,
                action: &self.action,
                commit_token: CommitToken::from_bytes([9; 16]),
            };
            self.registry
                .prepare(handle, entry, &context, images, request)
        }
    }

    // The adapter's own result can be successful while an integrity check
    // closes a shared readiness latch. Recheck must test both owners again.
    struct FreezeDuringRecheck {
        inner: Arc<dyn ErasedAdapter>,
        readiness: MechanicalOwnerReadiness,
        target: OwnerId,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl ErasedAdapter for FreezeDuringRecheck {
        fn prepare(
            &self,
            state: &(dyn Any + Send),
            context: &OwnerPrepareContext<'_>,
            request: ErasedValue,
        ) -> Result<ErasedPreparation, ParticipantError> {
            self.inner.prepare(state, context, request)
        }
        fn recheck(
            &self,
            state: &(dyn Any + Send),
            context: &OwnerPrepareContext<'_>,
            patch: &(dyn Any + Send),
        ) -> Result<(), ParticipantError> {
            self.inner.recheck(state, context, patch)?;
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.readiness
                .freeze(
                    &self.target,
                    crate::mechanics::IncidentId(1),
                    crate::mechanics::RecoveryReason::IntegrityViolation {
                        code: CanonicalId::new("test_recheck_integrity").unwrap(),
                    },
                )
                .unwrap();
            Ok(())
        }
        fn bind<'a>(
            self: Arc<Self>,
            state: &'a mut (dyn Any + Send),
            patch: ErasedValue,
        ) -> Result<Box<dyn ErasedBound + 'a>, ParticipantError> {
            Arc::clone(&self.inner).bind(state, patch)
        }
        fn accepts_write(&self, write: &ReplayWrite) -> bool {
            self.inner.accepts_write(write)
        }
    }

    #[test]
    fn a_successful_recheck_that_freezes_owner_or_requester_cannot_pass_handoff() {
        for target in [owner::<1>(), OwnerId::new("writer").unwrap()] {
            let f = fixture();
            let mut images = OwnerImages::default();
            f.prepare(
                &f.left,
                Patch {
                    before: 10,
                    after: 11,
                    refuse: false,
                },
                &mut images,
            )
            .unwrap();
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let image = images.0.first_mut().unwrap();
            image.readiness.require_ready(&target).unwrap();
            image.adapter = Arc::new(FreezeDuringRecheck {
                inner: Arc::clone(&image.adapter),
                readiness: image.readiness.clone(),
                target,
                calls: Arc::clone(&calls),
            });
            assert!(matches!(
                images.recheck(&f.world),
                Err(ParticipantError::NotReady)
            ));
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            // Once closed, the pre-callback guard refuses without invoking it.
            assert!(matches!(
                images.recheck(&f.world),
                Err(ParticipantError::NotReady)
            ));
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(f.world.module_state::<Cell<1>>(), Some(&Cell(10)));
            assert_eq!(f.world.module_state::<Cell<2>>(), Some(&Cell(20)));
        }
    }
    #[test]
    fn a_foreign_composition_handle_and_duplicate_slot_never_add_an_image() {
        let first = fixture();
        let foreign = fixture();
        let mut images = OwnerImages::default();
        assert!(matches!(
            first.prepare(
                &foreign.left,
                Patch {
                    before: 10,
                    after: 11,
                    refuse: false
                },
                &mut images
            ),
            Err(ParticipantError::Authority)
        ));
        assert!(images.is_empty());
        first
            .prepare(
                &first.left,
                Patch {
                    before: 10,
                    after: 11,
                    refuse: false,
                },
                &mut images,
            )
            .unwrap();
        assert!(matches!(
            first.prepare(
                &first.left,
                Patch {
                    before: 10,
                    after: 12,
                    refuse: false
                },
                &mut images
            ),
            Err(ParticipantError::DuplicateSlot)
        ));
        assert_eq!(images.0.len(), 1);
        assert_eq!(first.world.module_state::<Cell<1>>(), Some(&Cell(10)));
    }
    #[test]
    fn the_last_owner_bind_refusal_keeps_every_owner_unchanged() {
        let mut f = fixture();
        let mut images = OwnerImages::default();
        f.prepare(
            &f.left,
            Patch {
                before: 10,
                after: 11,
                refuse: false,
            },
            &mut images,
        )
        .unwrap();
        f.prepare(
            &f.right,
            Patch {
                before: 20,
                after: 21,
                refuse: true,
            },
            &mut images,
        )
        .unwrap();
        images.order();
        assert!(matches!(
            images.bind(f.world.composed_install_parts_mut().0),
            Err(ParticipantError::Slot)
        ));
        assert_eq!(f.world.module_state::<Cell<1>>(), Some(&Cell(10)));
        assert_eq!(f.world.module_state::<Cell<2>>(), Some(&Cell(20)));
    }
    #[test]
    fn replacement_is_not_a_current_slot_even_with_identical_contents() {
        let mut f = fixture();
        let mut images = OwnerImages::default();
        f.prepare(
            &f.left,
            Patch {
                before: 10,
                after: 11,
                refuse: false,
            },
            &mut images,
        )
        .unwrap();
        f.world.module_state_insert(Cell::<1>(10));
        assert!(matches!(
            images.recheck(&f.world),
            Err(ParticipantError::Slot)
        ));
        assert!(matches!(
            images.bind(f.world.composed_install_parts_mut().0),
            Err(ParticipantError::Slot)
        ));
        assert_eq!(f.world.module_state::<Cell<1>>(), Some(&Cell(10)));
    }
    #[test]
    fn both_bindings_are_read_only_until_the_consuming_apply() {
        let mut f = fixture();
        let mut images = OwnerImages::default();
        f.prepare(
            &f.right,
            Patch {
                before: 20,
                after: 22,
                refuse: false,
            },
            &mut images,
        )
        .unwrap();
        f.prepare(
            &f.left,
            Patch {
                before: 10,
                after: 12,
                refuse: false,
            },
            &mut images,
        )
        .unwrap();
        images.order();
        drop(images.bind(f.world.composed_install_parts_mut().0).unwrap());
        assert_eq!(f.world.module_state::<Cell<1>>(), Some(&Cell(10)));
        assert_eq!(f.world.module_state::<Cell<2>>(), Some(&Cell(20)));
        let mut images = OwnerImages::default();
        f.prepare(
            &f.left,
            Patch {
                before: 10,
                after: 12,
                refuse: false,
            },
            &mut images,
        )
        .unwrap();
        f.prepare(
            &f.right,
            Patch {
                before: 20,
                after: 22,
                refuse: false,
            },
            &mut images,
        )
        .unwrap();
        images
            .bind(f.world.composed_install_parts_mut().0)
            .unwrap()
            .apply();
        assert_eq!(f.world.module_state::<Cell<1>>(), Some(&Cell(12)));
        assert_eq!(f.world.module_state::<Cell<2>>(), Some(&Cell(22)));
    }
}

#[cfg(test)]
mod ic_tests;
