//! Opt-in fact access bound to a real dispatch and one exact contract surface.
//! Scope is read authority only. It neither creates consent nor settles money.

mod consent;
pub use consent::FactConsentV1;
pub(crate) use consent::{FactAttachment, prepare_attachment, stamp_attachment};

use mush_core::{Ctx, HookReadContext, ObjId, World};
use mush_org::{OrgScopeId, PrincipalId};

use super::{FactState, FactTable};
use crate::{ContractId, ContractState, EconomicContract, EconomicVisibility, ExternalRef};

/// Why the economy is asking an opt-in provider about a restricted reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum FactPurpose {
    /// A real command or producer is attaching or offering this exact surface.
    Attach,
    /// The actual viewer is reading this contract's fact reference or result.
    Read,
    /// The engine is rechecking an actual retained contract, including a sweep.
    Evaluate,
}

/// An economy-created scope. There is no public constructor or wire decoder.
/// A provider must still authorize the fact's subject; being a party to an
/// arbitrary contract does not authorize a guessed licence reference.
#[derive(Clone)]
pub struct ContractFactScope {
    actor: ObjId,
    purpose: FactPurpose,
    proposed: EconomicContract,
    preimage: Option<EconomicContract>,
}

// OwnerRead requests need a bounded wire measurement, not a decoder. Only the
// actual in-memory opaque scope reaches read(); private consent/preimage details
// never become an output schema or caller-constructible authority.
impl serde::Serialize for ContractFactScope {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("ContractFactScope", 5)?;
        s.serialize_field("actor", &self.actor)?;
        s.serialize_field("purpose", &self.purpose)?;
        s.serialize_field("contract", &self.proposed.id)?;
        s.serialize_field("generation", &self.proposed.amendments)?;
        s.serialize_field("consent_hash", &self.consent_hash())?;
        s.end()
    }
}

impl std::fmt::Debug for ContractFactScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContractFactScope")
            .field("contract", &self.proposed.id)
            .field("purpose", &self.purpose)
            .finish_non_exhaustive()
    }
}

impl ContractFactScope {
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    pub const fn purpose(&self) -> FactPurpose {
        self.purpose
    }
    pub const fn contract(&self) -> ContractId {
        self.proposed.id
    }
    pub fn issuer(&self) -> &PrincipalId {
        &self.proposed.issuer
    }
    pub fn parties(&self) -> &[PrincipalId] {
        &self.proposed.consent.parties
    }
    pub fn visibility(&self) -> &EconomicVisibility {
        &self.proposed.consent.visibility
    }
    pub const fn org_scope(&self) -> Option<OrgScopeId> {
        self.proposed.consent.scope
    }
    pub const fn generation(&self) -> u32 {
        self.proposed.amendments
    }
    /// Actual attachment author retained in this exact frozen surface. This is
    /// historical attribution; the provider must still check subject authority.
    pub fn attachment_actor(&self, fact: &ExternalRef) -> Option<ObjId> {
        if self.purpose == FactPurpose::Attach {
            return None;
        }
        let marker = self.proposed.consent.fact_consent.as_ref()?;
        marker.matches(&self.proposed, fact).then(|| marker.actor())
    }
    pub fn consent_hash(&self) -> [u8; 32] {
        crate::consent_hash(&self.proposed.issuer, &self.proposed.consent)
    }

    /// Recheck retained provenance in a registered owner service. The caller
    /// must be the actual dispatch that minted this scope. A copied scope cannot
    /// be replayed after the contract's preimage changed, or by another actor.
    pub fn matches_read(
        &self,
        ctx: &mush_core::read_service::OwnerReadContext<'_>,
        fact: &ExternalRef,
    ) -> bool {
        ctx.actor() == self.actor && self.matches_world(ctx.world(), fact)
    }

    fn matches_world(&self, w: &World, fact: &ExternalRef) -> bool {
        let Some(store) = w.module_state::<crate::EconomyStore>() else {
            return false;
        };
        if store.is_frozen()
            || !self
                .proposed
                .consent
                .conditions
                .iter()
                .any(|c| matches!(c, crate::TypedCondition::Fact { fact: named } if named == fact))
        {
            return false;
        }
        match (&self.preimage, store.snapshot().contract(self.proposed.id)) {
            (Some(expected), Some(current)) => expected == current,
            (None, None) => self.purpose == FactPurpose::Attach,
            _ => false,
        }
    }
}

/// Read-only actual dispatch plus the immutable proposed/retained contract
/// surface. Registered services retain their normal requester/readiness gates.
pub struct ScopedFactContext<'a> {
    read: HookReadContext<'a>,
    scope: ContractFactScope,
}

impl<'a> ScopedFactContext<'a> {
    pub fn world(&self) -> &'a World {
        self.read.world()
    }
    pub fn scope(&self) -> &ContractFactScope {
        &self.scope
    }
    pub fn read_owner<S: mush_core::read_service::OwnerReadService>(
        &self,
        handle: &mush_core::read_service::OwnerReadHandle<S>,
        request: S::Request,
    ) -> Result<S::Output, mush_core::read_service::OwnerReadError> {
        self.read.read_owner(handle, request)
    }

    fn new(ctx: &'a Ctx<'_>, c: &EconomicContract, purpose: FactPurpose) -> Option<Self> {
        let actor = ctx.dispatch_actor()?;
        if ctx.actor != actor {
            return None;
        }
        ctx.reg
            .require_mechanics_owner_ready(&mush_core::mechanics::OwnerId::new("economy").ok()?)
            .ok()?;
        let store = ctx.w.module_state::<crate::EconomyStore>()?;
        if store.is_frozen()
            || c.consent.parties.len() > crate::contract::MAX_PARTIES
            || c.consent.conditions.len() > crate::contract::MAX_CONDITIONS
        {
            return None;
        }
        let preimage = store.snapshot().contract(c.id).cloned();
        if purpose == FactPurpose::Attach {
            if !matches!(c.state, ContractState::Draft | ContractState::Offered)
                || preimage.as_ref().is_some_and(|old| {
                    old.issuer != c.issuer
                        || !(old.amendments == c.amendments
                            || old.amendments.checked_add(1) == Some(c.amendments))
                })
            {
                return None;
            }
        } else if !c.consent_intact()
            || !preimage.as_ref().is_some_and(|actual| {
                actual.consent_intact()
                    && actual.issuer == c.issuer
                    && actual.consent == c.consent
                    && actual.consent_hash == c.consent_hash
                    && actual.amendments == c.amendments
                    && actual.state == c.state
                    && actual.generation_started_at == c.generation_started_at
            })
        {
            return None;
        }
        Some(Self {
            read: ctx.hook_read_context("economy"),
            scope: ContractFactScope {
                actor,
                purpose,
                proposed: c.clone(),
                preimage,
            },
        })
    }
}

/// Live engine adapter. Unlike WorldFacts, restricted references can only be
/// evaluated with the engine's exact current contract and actual dispatch.
pub(crate) struct ContextualWorldFacts<'a, 'w> {
    pub ctx: &'a Ctx<'w>,
    pub providers: FactTable,
}

impl crate::engine::FactEval for ContextualWorldFacts<'_, '_> {
    fn resolve(&self, fact: &ExternalRef) -> Option<bool> {
        super::WorldFacts {
            w: self.ctx.w,
            providers: self.providers.clone(),
        }
        .state(fact)
        .map(|s| s.satisfied)
    }
    fn resolve_in_contract(&self, contract: &EconomicContract, fact: &ExternalRef) -> Option<bool> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.provider() == fact.provider.as_str())?;
        if !provider.requires_scope(fact) {
            return self.resolve(fact);
        }
        let context = ScopedFactContext::new(self.ctx, contract, FactPurpose::Evaluate)?;
        if context.scope.attachment_actor(fact).is_none()
            || !provider.authorize_scope(&context, fact)
        {
            return None;
        }
        provider.resolve_scoped(&context, fact).map(|s| s.satisfied)
    }
}

pub(crate) fn permitted(
    ctx: &Ctx<'_>,
    c: &EconomicContract,
    fact: &ExternalRef,
    purpose: FactPurpose,
) -> bool {
    let Some(store) = ctx.w.module_state::<crate::EconomyStore>() else {
        return false;
    };
    let providers = store.facts();
    let Some(provider) = providers
        .iter()
        .find(|p| p.provider() == fact.provider.as_str())
    else {
        return true;
    };
    if !provider.requires_scope(fact) {
        return true;
    }
    ScopedFactContext::new(ctx, c, purpose).is_some_and(|scope| {
        (purpose == FactPurpose::Attach || scope.scope.attachment_actor(fact).is_some())
            && provider.authorize_scope(&scope, fact)
    })
}

pub(crate) fn validate_attachment(
    ctx: &Ctx<'_>,
    c: &EconomicContract,
) -> Result<(), crate::ContractError> {
    for condition in &c.consent.conditions {
        if let crate::TypedCondition::Fact { fact } = condition
            && !permitted(ctx, c, fact, FactPurpose::Attach)
        {
            return Err(crate::ContractError::NoStanding);
        }
    }
    Ok(())
}

/// Resolve a fact for the actual viewer through an exact retained contract.
/// Missing contract/reference, unauthorized subject and absent truth agree.
pub fn contract_fact(ctx: &Ctx<'_>, id: ContractId, fact: &ExternalRef) -> Option<FactState> {
    if !crate::contract_live::may_view_contract(ctx.w, ctx.actor, id) {
        return None;
    }
    let store = ctx.w.module_state::<crate::EconomyStore>()?;
    let c = store.snapshot().contract(id)?;
    let context = ScopedFactContext::new(ctx, c, FactPurpose::Read)?;
    if !context.scope.matches_world(ctx.w, fact) {
        return None;
    }
    let providers = store.facts();
    let provider = providers
        .iter()
        .find(|p| p.provider() == fact.provider.as_str())?;
    if provider.requires_scope(fact) {
        if context.scope.attachment_actor(fact).is_none()
            || !provider.authorize_scope(&context, fact)
        {
            return None;
        }
        provider.resolve_scoped(&context, fact)
    } else {
        provider.resolve(ctx.w, fact.kind.as_str(), fact.id.as_str(), fact.revision)
    }
}
