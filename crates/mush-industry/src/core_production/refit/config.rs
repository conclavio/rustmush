//! Trusted finite family and real acknowledged owner callbacks.
use super::{Error, RefitRecipeAuthority, bad, records::*};
use crate::core_production::{
    CoreOriginMode, authority::Definitions, participant::CoreProductionState,
};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    OwnerId, World,
};
use mush_economy::core_refit::{
    CoreRefitConfig, RefitError, RefitOfferAttribution, RefitOrderAttribution, RefitOwnerAuthority,
    RefitPaymentPolicy, RefitSubject,
};
use mush_tactics::ComponentId;
use std::sync::Arc;
#[derive(Clone)]
pub struct CoreRefitFactoryConfig {
    pub(crate) mode: CoreOriginMode,
    pub(crate) payment: RefitPaymentPolicy,
    pub(crate) component: ComponentId,
    pub(crate) recipes: Arc<dyn RefitRecipeAuthority>,
    pub(crate) tactics_owner: DefinitionRef,
    pub(crate) definitions: Definitions,
}
impl CoreRefitFactoryConfig {
    pub fn new(
        mode: CoreOriginMode,
        payment: RefitPaymentPolicy,
        component: ComponentId,
        recipes: Arc<dyn RefitRecipeAuthority>,
        tactics_owner: DefinitionRef,
    ) -> Result<Self, Error> {
        payment.validate().map_err(bad)?;
        if tactics_owner.key().owner.as_str() != "tactics"
            || tactics_owner.key().kind.as_str() != "owner_participant"
            || tactics_owner.key().id.as_str() != "component_replacement"
            || tactics_owner.version() != 1
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            mode,
            payment,
            component,
            recipes,
            tactics_owner,
            definitions: Definitions::default(),
        })
    }
    pub fn payment(&self) -> &RefitPaymentPolicy {
        &self.payment
    }
    pub fn component(&self) -> &ComponentId {
        &self.component
    }
    pub fn replacement_policy(&self) -> super::RefitReplacementPolicy {
        super::RefitReplacementPolicy {
            config: self.clone(),
            definitions: self.definitions.clone(),
        }
    }
    pub(crate) fn action_descriptor(&self, name: &str) -> Result<DefinitionDescriptor, Error> {
        if name != super::INITIALIZE && name != super::ADVANCE {
            return self.payment.action_descriptor(name).map_err(bad);
        }
        let mut refs = vec![self.payment.recipe.clone()];
        refs.extend(self.payment.fitted_units.iter().cloned());
        refs.sort();
        refs.dedup();
        DefinitionDescriptor::new(DefinitionKind::new("action").map_err(bad)?,DefinitionId::new(name).map_err(bad)?,1,
            serde_json::json!({"format":1,"protocol":"certified_core_refit_v1","policy":self.payment,"component":self.component,"kind":name,"cadence":super::CADENCE,"maximum_work_units":super::MAX_WORK_UNITS}))
            .map(|d|d.with_references(refs)).map_err(bad)
    }
    pub fn action(&self, name: &str) -> Result<DefinitionRef, Error> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(bad)?,
                self.action_descriptor(name)?,
            )
            .map_err(bad)
    }
    pub(crate) fn economy_config(&self, owner: DefinitionRef) -> Result<CoreRefitConfig, Error> {
        CoreRefitConfig::new(
            self.payment.clone(),
            owner,
            Arc::new(Acknowledged {
                config: self.clone(),
            }),
        )
        .map_err(bad)
    }
}
struct Acknowledged {
    config: CoreRefitFactoryConfig,
}
fn state(world: &World) -> Result<&CoreProductionState, RefitError> {
    if world
        .module_state_owner(std::any::TypeId::of::<CoreProductionState>())
        .is_none_or(|o| o.as_str() != crate::NS)
    {
        return Err(RefitError::Unavailable);
    }
    world
        .module_state::<CoreProductionState>()
        .filter(|s| {
            s.ready
                && s.refit
                    .as_ref()
                    .is_some_and(|p| p.ready && p.index.is_some())
        })
        .ok_or(RefitError::Unavailable)
}
impl RefitOwnerAuthority for Acknowledged {
    fn acknowledged_offer(
        &self,
        world: &World,
        subject: &RefitSubject,
    ) -> Result<RefitOfferAttribution, RefitError> {
        let p = state(world)?
            .refit
            .as_ref()
            .ok_or(RefitError::Unavailable)?;
        let o = p
            .offers
            .get(&subject.offer)
            .ok_or(RefitError::Unavailable)?;
        o.validate().map_err(|_| RefitError::Integrity)?;
        if o.policy != self.config.payment
            || o.digest().map_err(|_| RefitError::Integrity)? != subject.offer_digest
            || o.requester != subject.requester
            || o.incoming != subject.incoming
            || o.core_acceptance != subject.core_acceptance
            || o.hull != subject.hull
            || o.unit_revision != subject.unit_revision
            || o.displaced.installed.asset != subject.displaced
            || o.class() != subject.class
            || o.policy
                .target(o.class())
                .map_err(|_| RefitError::Integrity)?
                != &subject.target
            || p.orders
                .values()
                .any(|v| v.intent.offer.operation == o.operation)
        {
            return Err(RefitError::Unavailable);
        }
        Ok(RefitOfferAttribution {
            operation: o.operation,
            digest: subject.offer_digest,
            action: o.action.clone(),
            operator: o.operator,
            observed_ic: o.observed_ic.clone(),
            observed_utc: o.observed_utc,
            expires_utc: o.expires_utc,
        })
    }
    fn acknowledged_ready_order(
        &self,
        world: &World,
        subject: &RefitSubject,
    ) -> Result<RefitOrderAttribution, RefitError> {
        let state = state(world)?;
        let p = state.refit.as_ref().ok_or(RefitError::Unavailable)?;
        let row = p
            .orders
            .get(&crate::workorder::WorkOrderId(subject.order))
            .ok_or(RefitError::Unavailable)?;
        if row.intent.subject().map_err(|_| RefitError::Integrity)? != *subject
            || !row.progress.production().is_complete()
        {
            return Err(RefitError::Unavailable);
        }
        let registry = self
            .config
            .definitions
            .0
            .get()
            .ok_or(RefitError::Unavailable)?;
        let (_, journal) = super::policies::replacement(world, registry, &self.config, &row.intent)
            .map_err(|_| RefitError::Unavailable)?;
        let replacement = ReplacementEvidence::capture(&journal)
            .digest()
            .map_err(|_| RefitError::Integrity)?;
        let required = row.intent.offer.recipe.work_units();
        let due_at = mush_core::ClockStamp {
            clock: subject.accepted_at.clock.clone(),
            at: subject
                .accepted_at
                .at
                .checked_add(
                    i64::try_from(required)
                        .map_err(|_| RefitError::Invalid)?
                        .checked_mul(super::CADENCE)
                        .ok_or(RefitError::Invalid)?,
                )
                .ok_or(RefitError::Invalid)?,
        };
        Ok(RefitOrderAttribution {
            acceptance: row.accepted,
            subject: subject.digest()?,
            owner_generation: state.generation,
            due_at,
            required_work: required,
            completed_work: required,
            replacement,
        })
    }
}
