use super::{LaunchDeployment, content::bad};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistry,
    DefinitionRegistryBuilder, ModError, ObjId, OwnerId, World,
};
use mush_org::OrgPredicateProvider;
use mush_roll::commerce::{
    house_catalogue::HouseIssuer, house_purchase::HouseBuyerAdmission, records::PublicationError,
};
use std::{
    any::TypeId,
    sync::{Arc, OnceLock},
};

#[derive(Clone)]
pub(super) struct CopratesAdmission {
    issuer: HouseIssuer,
    faction: mush_faction::FactionId,
    descriptor: DefinitionDescriptor,
    reference: DefinitionRef,
    registry: Arc<OnceLock<DefinitionRegistry>>,
}
impl std::fmt::Debug for CopratesAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopratesAdmission")
            .field("reference", &self.reference)
            .finish_non_exhaustive()
    }
}
impl CopratesAdmission {
    pub(super) fn new(deployment: &LaunchDeployment) -> Result<Self, ModError> {
        let house = &deployment.houses.coprates;
        let issuer = HouseIssuer::new(house.organization, house.scope).map_err(bad)?;
        let descriptor = DefinitionDescriptor::new(
            DefinitionKind::new("buyer_eligibility").map_err(bad)?,
            DefinitionId::new("coprates_house_member").map_err(bad)?,
            1,
            serde_json::json!({"format":1,"issuer":issuer,"faction":deployment.republic,
                "predicate":"member","scope":"fresh_house_purchase_and_renewal","historical_paid_use":false}),
        )
        .map_err(bad)?;
        let reference = DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS).map_err(bad)?, descriptor.clone())
            .map_err(bad)?;
        Ok(Self {
            issuer,
            faction: deployment.republic,
            descriptor,
            reference,
            registry: Arc::default(),
        })
    }
    pub(super) fn descriptor(&self) -> &DefinitionDescriptor {
        &self.descriptor
    }
    pub(super) fn capture(&self, registry: &DefinitionRegistry) -> Result<(), ModError> {
        registry.get_exact(&self.reference).map_err(bad)?;
        registry
            .require_owner_ready(&OwnerId::new(mush_faction::NS).map_err(bad)?)
            .map_err(bad)?;
        self.registry
            .set(registry.clone())
            .map_err(|_| bad("launch admission registry already captured"))
    }
}
impl HouseBuyerAdmission for CopratesAdmission {
    fn check(
        &self,
        world: &World,
        issuer: HouseIssuer,
        buyer: ObjId,
    ) -> Result<(), PublicationError> {
        if issuer != self.issuer {
            return Ok(());
        }
        let registry = self.registry.get().ok_or(PublicationError::Unavailable)?;
        registry
            .get_exact(&self.reference)
            .map_err(|_| PublicationError::Unavailable)?;
        registry
            .require_owner_ready(
                &OwnerId::new(mush_faction::NS).map_err(|_| PublicationError::Invalid)?,
            )
            .map_err(|_| PublicationError::Unavailable)?;
        if world
            .module_state_owner(TypeId::of::<mush_faction::FactionState>())
            .map(|o| o.as_str())
            != Some(mush_faction::NS)
            || world.get(buyer).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
            || mush_faction::FactionStandingProvider.eval(
                world,
                "member",
                &serde_json::json!({"faction":self.faction.0}),
                buyer,
            ) != Some(true)
        {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
}
