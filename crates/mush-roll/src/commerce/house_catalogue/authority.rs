use mush_core::read_service::OwnerReadError;
use mush_core::{Flags, ObjId, ObjKind, World};
use mush_economy::{AssetLifecycle, EconomicAsset, EconomyStore};
use mush_org::{
    AuthRequest, CapabilityId, OrgId, OrgScopeId, OrgStatus, OrgStore, PrincipalId, ScopeStatus,
};
use serde::{Deserialize, Serialize};
use std::any::TypeId;

/// Exact issuer data. Deserialization alone confers no rights. Each current
/// observation proves the real filing, native title, organization and scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "IssuerWire")]
pub struct HouseIssuer {
    format: u16,
    organization: OrgId,
    scope: OrgScopeId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IssuerWire {
    format: u16,
    organization: OrgId,
    scope: OrgScopeId,
}
impl TryFrom<IssuerWire> for HouseIssuer {
    type Error = &'static str;
    fn try_from(w: IssuerWire) -> Result<Self, Self::Error> {
        if w.format != 1 {
            return Err("invalid house issuer");
        }
        Ok(Self {
            format: 1,
            organization: w.organization,
            scope: w.scope,
        })
    }
}
impl HouseIssuer {
    pub fn new(organization: OrgId, scope: OrgScopeId) -> Result<Self, OwnerReadError> {
        Self::try_from(IssuerWire {
            format: 1,
            organization,
            scope,
        })
        .map_err(|_| OwnerReadError::Refused)
    }
    pub const fn organization(&self) -> OrgId {
        self.organization
    }
    pub const fn scope(&self) -> OrgScopeId {
        self.scope
    }
    pub fn principal(&self) -> PrincipalId {
        PrincipalId::Organization(self.organization)
    }
    pub(super) fn live(&self, world: &World) -> Result<(u64, u64), OwnerReadError> {
        if world
            .module_state_owner(TypeId::of::<OrgStore>())
            .is_none_or(|o| o.as_str() != "org")
        {
            return Err(OwnerReadError::Unavailable);
        }
        let store = world
            .module_state::<OrgStore>()
            .ok_or(OwnerReadError::Unavailable)?;
        let org = store
            .state()
            .org(self.organization)
            .ok_or(OwnerReadError::Refused)?;
        let scope = store
            .state()
            .scope(self.scope)
            .ok_or(OwnerReadError::Refused)?;
        if org.status != OrgStatus::Active
            || scope.org != org.id
            || scope.status != ScopeStatus::Active
        {
            return Err(OwnerReadError::Refused);
        }
        Ok((org.revision, scope.revision))
    }
    pub(super) fn publication(&self, world: &World, actor: ObjId) -> Result<(), OwnerReadError> {
        self.live(world)?;
        player(world, actor)?;
        let capability =
            CapabilityId::new(mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY)
                .ok_or(OwnerReadError::Integrity)?;
        let request = AuthRequest {
            actor,
            capability,
            scope: self.scope,
            now: world.now(),
            amount: None,
        };
        if !mush_org::authorize_in_world(world, &request).is_allow() {
            return Err(OwnerReadError::Authority);
        }
        Ok(())
    }
    pub(super) fn title<'a>(
        &self,
        world: &'a World,
        charter: super::super::CharterId,
    ) -> Result<&'a EconomicAsset, OwnerReadError> {
        self.live(world)?;
        if world
            .module_state_owner(TypeId::of::<EconomyStore>())
            .is_none_or(|o| o.as_str() != "economy")
        {
            return Err(OwnerReadError::Unavailable);
        }
        let economy = world
            .module_state::<EconomyStore>()
            .ok_or(OwnerReadError::Unavailable)?;
        if economy.is_frozen()
            || world
                .get(charter.object())
                .is_none_or(|o| o.flags.contains(Flags::GOING))
        {
            return Err(OwnerReadError::Unavailable);
        }
        let title = economy
            .snapshot()
            .asset(charter.object())
            .ok_or(OwnerReadError::Refused)?;
        if title.object != charter.object()
            || title.revision == 0
            || title.title_owner != self.principal()
            || title.scope != Some(self.scope)
            || title.lifecycle != AssetLifecycle::Active
            || title.frozen
            || title.stolen.is_some()
        {
            return Err(OwnerReadError::Refused);
        }
        Ok(title)
    }
}
pub(super) fn player(world: &World, actor: ObjId) -> Result<(), OwnerReadError> {
    if actor.0 == 0
        || world
            .get(actor)
            .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
    {
        return Err(OwnerReadError::Refused);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_zero_org_and_scope_identifiers_remain_data_and_wire_is_closed() {
        let zero = HouseIssuer::new(OrgId(0), OrgScopeId(0)).expect("actual first Org IDs");
        let value = serde_json::to_value(zero).expect("data");
        assert_eq!(
            serde_json::from_value::<HouseIssuer>(value.clone()).expect("round trip"),
            zero
        );
        let mut unknown = value.clone();
        unknown["authority"] = serde_json::json!(true);
        assert!(serde_json::from_value::<HouseIssuer>(unknown).is_err());
        let mut version = value;
        version["format"] = serde_json::json!(2);
        assert!(serde_json::from_value::<HouseIssuer>(version).is_err());
    }
}
