//! Trusted composition. The Economy callback reads the real acknowledged
//! Industry projection; it cannot be supplied by an action parameter.
use super::{CoreRecipeAuthority, IntegrationError as Error, bad, records::*};
use crate::core_production::{CoreOriginMode, OriginPolicy};
use mush_core::{
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, OwnerId, World,
};
use mush_economy::core_integration::{
    CoreFundingSubject, CoreIntegrationConfig, CoreIntegrationError, CoreOfferAttribution,
    CoreOfferAuthority, CorePaymentPolicy,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct CoreFactoryConfig {
    pub(crate) mode: CoreOriginMode,
    pub(crate) payment: CorePaymentPolicy,
    pub(crate) recipes: Arc<dyn CoreRecipeAuthority>,
    pub(crate) economy_owner: DefinitionRef,
    pub(crate) refit: Option<super::super::refit::CoreRefitFactoryConfig>,
}
impl CoreFactoryConfig {
    pub fn new(
        mode: CoreOriginMode,
        payment: CorePaymentPolicy,
        recipes: Arc<dyn CoreRecipeAuthority>,
        economy_owner: DefinitionRef,
    ) -> Result<Self, Error> {
        payment.validate().map_err(bad)?;
        if economy_owner.key().owner.as_str() != "economy"
            || economy_owner.key().kind.as_str() != "owner_participant"
            || economy_owner.key().id.as_str() != "initial_filing"
            || economy_owner.version() != 1
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            mode,
            payment,
            recipes,
            economy_owner,
            refit: None,
        })
    }
    pub fn with_refits(
        mut self,
        config: super::super::refit::CoreRefitFactoryConfig,
    ) -> Result<Self, Error> {
        if config.mode != self.mode || config.payment.factory != self.payment.factory {
            return Err(Error::Invalid);
        }
        // Doc 17h cut M8b-2 fix round 1 (L4): the back room that integrates heirloom and grey
        // dies is the one that ungoverns, so its cores can be fitted where they are made.
        if let Some(u) = &self.payment.ungoverned
            && config.payment.ungovern.as_ref().map(|r| r.factory) != Some(u.factory)
        {
            return Err(Error::Invalid);
        }
        self.refit = Some(config);
        Ok(self)
    }
    pub fn economy_refit_config(
        &self,
        origin: &OriginPolicy,
    ) -> Result<mush_economy::core_refit::CoreRefitConfig, Error> {
        self.refit
            .as_ref()
            .ok_or(Error::Unavailable)?
            .economy_config(self.owner_definition(origin)?)
    }
    pub fn payment(&self) -> &CorePaymentPolicy {
        &self.payment
    }
    pub fn action(&self, name: &str) -> Result<DefinitionRef, Error> {
        self.payment.action_ref(name).map_err(bad)
    }
    pub(crate) fn participant_descriptor(
        &self,
        origin: &OriginPolicy,
    ) -> Result<DefinitionDescriptor, Error> {
        let mut actions = super::ACTIONS
            .iter()
            .map(|name| self.action(name))
            .collect::<Result<Vec<_>, _>>()?;
        if self.payment.delivery.is_some() {
            actions.push(self.action(super::delivery::ACTION)?);
        }
        let origin_actions = [
            origin.action(true).map_err(bad)?,
            origin.action(false).map_err(bad)?,
        ]
        .into_iter()
        .map(|d| {
            DefinitionRegistryBuilder::new()
                .register(OwnerId::new(crate::NS).map_err(bad)?, d)
                .map_err(bad)
        })
        .collect::<Result<Vec<_>, Error>>()?;
        let mut refs = actions.clone();
        refs.extend(origin_actions.iter().cloned());
        refs.push(origin.content.clone());
        refs.sort();
        refs.dedup();
        if let Some(refit) = &self.refit {
            let refit_actions = super::super::refit::ACTIONS
                .iter()
                .map(|a| refit.action(a))
                .collect::<Result<Vec<_>, _>>()?;
            refs.extend(refit_actions.iter().cloned());
            refs.push(refit.tactics_owner.clone());
            refs.sort();
            refs.dedup();
            return DefinitionDescriptor::new(DefinitionKind::new("owner_participant").map_err(bad)?,DefinitionId::new("core_production").map_err(bad)?,3,
                serde_json::json!({"format":3,"protocol":"certified_origin_integration_refit_v1","origin_index":super::super::INDEX_KEY,"origin_actions":origin_actions,"origin_content":origin.content,"origin_max":super::super::MAX_CERTIFICATES,"index":INDEX_KEY,"actions":actions,"payment":self.payment,"max_orders":MAX_ORDERS,"order_bytes":ORDER_BYTES,"refit_index":super::super::refit::records::INDEX_KEY,"refit_policy":refit.payment,"refit_component":refit.component,"refit_actions":refit_actions,"refit_order_bytes":super::super::refit::records::ORDER_BYTES}))
                .map(|d|d.with_references(refs)).map_err(bad);
        }
        DefinitionDescriptor::new(DefinitionKind::new("owner_participant").map_err(bad)?,DefinitionId::new("core_production").map_err(bad)?,2,
   serde_json::json!({"format":2,"protocol":"certified_origin_and_integration_v1","origin_index":super::super::INDEX_KEY,"origin_actions":origin_actions,"origin_content":origin.content,"origin_max":super::super::MAX_CERTIFICATES,"index":INDEX_KEY,"actions":actions,"payment":self.payment,"max_orders":MAX_ORDERS,"order_bytes":ORDER_BYTES}))
   .map(|d|d.with_references(refs)).map_err(bad)
    }
    pub fn owner_definition(&self, origin: &OriginPolicy) -> Result<DefinitionRef, Error> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(bad)?,
                self.participant_descriptor(origin)?,
            )
            .map_err(bad)
    }
    pub fn economy_config(&self, origin: &OriginPolicy) -> Result<CoreIntegrationConfig, Error> {
        CoreIntegrationConfig::new(
            self.payment.clone(),
            self.owner_definition(origin)?,
            Arc::new(AcknowledgedOffers {
                policy: self.payment.digest().map_err(bad)?,
            }),
        )
        .map_err(bad)
    }
}
struct AcknowledgedOffers {
    policy: DefinitionDigest,
}
impl CoreOfferAuthority for AcknowledgedOffers {
    fn acknowledged_offer(
        &self,
        world: &World,
        subject: &CoreFundingSubject,
    ) -> Result<CoreOfferAttribution, CoreIntegrationError> {
        let unavailable = || CoreIntegrationError::Unavailable;
        if world
            .module_state_owner(std::any::TypeId::of::<
                super::super::participant::CoreProductionState,
            >())
            .is_none_or(|o| o.as_str() != crate::NS)
        {
            return Err(unavailable());
        }
        let state = world
            .module_state::<super::super::participant::CoreProductionState>()
            .ok_or_else(unavailable)?;
        let projection = state
            .integration
            .as_ref()
            .filter(|p| p.ready)
            .ok_or_else(unavailable)?;
        if !state.ready || projection.index.is_none() || self.policy != subject.policy {
            return Err(unavailable());
        }
        let offer = projection
            .offers
            .get(&subject.offer)
            .ok_or_else(unavailable)?;
        offer.validate().map_err(|_| unavailable())?;
        if offer.digest().map_err(|_| unavailable())? != subject.offer_digest
            || offer.requester != subject.requester
            || offer.policy.digest()? != self.policy
            || offer.class() != subject.class
            || offer.policy.recipe != subject.recipe
            || projection
                .orders
                .values()
                .any(|o| o.intent.offer == subject.offer)
        {
            return Err(unavailable());
        }
        match &offer.input {
            super::offer::OfferedInput::SupplierStock { certificate, lot } => {
                if !subject.source.is_certified()
                    || certificate.operation != subject.die_certificate
                    || lot.id().0 != subject.die_lot
                    || lot.endpoint().0 != subject.input_vault
                {
                    return Err(unavailable());
                }
            }
            super::offer::OfferedInput::RequesterSupplied {} => {
                if !subject.source.is_certified()
                    || offer.policy.supplier_stock.contains(&subject.input_vault)
                {
                    return Err(unavailable());
                }
            }
            // Doc 17h cut M8b-2 (H2): the ungoverning site's offer funds only a heirloom or
            // grey die the requester brings.
            super::offer::OfferedInput::Uncertified {} => {
                if subject.source.is_certified()
                    || offer.policy.supplier_stock.contains(&subject.input_vault)
                {
                    return Err(unavailable());
                }
            }
        }
        Ok(CoreOfferAttribution {
            operation: offer.operation,
            digest: subject.offer_digest,
            action: offer.action.clone(),
            operator: offer.operator,
            observed_ic: offer.observed_ic.clone(),
            observed_utc: offer.observed_utc,
            expires_utc: offer.expires_utc,
        })
    }
}
