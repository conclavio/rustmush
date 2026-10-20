//! Current scoped authority for the fee-only House update. Captured attribution
//! never substitutes for current Org authorization or the composed readiness latch.
use super::*;
use mush_org::{AuthRequest, CapabilityId, OrgId, OrgScopeId, OrgStore};
use std::{
    any::TypeId,
    sync::{Arc, OnceLock},
};

/// Same-slot input. The native owner derives actor, fee and issuer itself.
pub struct RegistryHouseCompatibleUpdate {
    pub charter: ObjId,
    pub title_revision: u64,
    pub payer: AccountId,
    pub organization: OrgId,
    pub scope: OrgScopeId,
}

#[derive(Clone)]
pub(crate) struct HouseUpdateConfig {
    pub(crate) action: DefinitionRef,
    pub(crate) definitions: Arc<OnceLock<mush_core::DefinitionRegistry>>,
}
impl std::fmt::Debug for HouseUpdateConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HouseUpdateConfig")
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}
impl HouseUpdateConfig {
    pub(crate) fn new(action: DefinitionRef) -> Self {
        Self {
            action,
            definitions: Arc::default(),
        }
    }
    fn ready(&self, world: &World) -> Result<(), ParticipantError> {
        let definitions = self.definitions.get().ok_or_else(refused)?;
        for owner in ["economy", "org"] {
            definitions
                .require_owner_ready(&OwnerId::new(owner).map_err(|_| refused())?)
                .map_err(|_| refused())?;
        }
        definitions.get_exact(&self.action).map_err(|_| refused())?;
        if world
            .module_state_owner(TypeId::of::<OrgStore>())
            .is_none_or(|o| o.as_str() != "org")
            || world
                .module_state_owner(TypeId::of::<EconomyStore>())
                .is_none_or(|o| o.as_str() != "economy")
        {
            return Err(refused());
        }
        Ok(())
    }
}

pub(super) enum UpdateAuthority {
    Personal,
    House {
        config: HouseUpdateConfig,
        attribution: HouseFilingAttribution,
    },
}
impl UpdateAuthority {
    pub(super) fn house(&self) -> Option<&HouseFilingAttribution> {
        match self {
            Self::Personal => None,
            Self::House { attribution, .. } => Some(attribution),
        }
    }
    pub(super) fn owner(&self, actor: ObjId) -> PrincipalId {
        self.house().map_or(PrincipalId::Object(actor), |h| {
            PrincipalId::Organization(h.organization())
        })
    }
    pub(super) fn scope(&self) -> Option<OrgScopeId> {
        self.house().map(HouseFilingAttribution::scope)
    }
    pub(super) fn require(
        &self,
        world: &World,
        state: &EconomyStore,
        actor: ObjId,
        request: &RegistryCompatibleUpdate,
        fee: Amount,
    ) -> Result<(), ParticipantError> {
        let Self::House {
            config,
            attribution,
        } = self
        else {
            return Ok(());
        };
        config.ready(world)?;
        let owner = self.owner(actor);
        let payer = state.snapshot.account(request.payer).ok_or_else(refused)?;
        if actor != attribution.curator()
            || payer.owner != owner
            || payer.scope != self.scope()
            || !crate::api::scope_coherent(world, self.scope(), &owner)
        {
            return Err(refused());
        }
        // No asset-transfer capability is requested: this plan never changes
        // title. Current explicit publication and actual fee debit are required.
        for (name, amount) in [
            (super::super::house::HOUSE_PUBLICATION_CAPABILITY, None),
            ("economy.account.post", Some(fee.minor())),
        ] {
            let request = AuthRequest {
                actor,
                capability: CapabilityId::new(name).ok_or_else(refused)?,
                scope: attribution.scope(),
                now: world.now(),
                amount,
            };
            if !mush_org::authorize_in_world(world, &request).is_allow() {
                return Err(refused());
            }
        }
        Ok(())
    }
}
