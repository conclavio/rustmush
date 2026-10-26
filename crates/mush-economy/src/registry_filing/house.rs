//! Closed scoped filing identity and live authorization. Data is not authority.
#[cfg(test)]
mod tests;
use super::{RegistryInitialFiling, refused};
use crate::module::EconomyStore;
use crate::{AccountId, Amount};
use mush_core::participant::ParticipantError;
use mush_core::{ObjId, ObjKind, World};
use mush_org::{AuthRequest, CapabilityId, OrgId, OrgScopeId, PrincipalId};
use serde::{Deserialize, Serialize};

pub const HOUSE_FILING_ACTION: &str = "house_initial_filing";
pub const HOUSE_PUBLICATION_CAPABILITY: &str = "roll.mind.publish";
pub(super) const NAMESPACE: &str = "roll.house_filing.v1";

/// Historical attribution, not an authorization token. Only actual history
/// replay establishes that this precise scoped filing was committed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "AttributionWire")]
pub struct HouseFilingAttribution {
    format: u16,
    curator: ObjId,
    organization: OrgId,
    scope: OrgScopeId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttributionWire {
    format: u16,
    curator: ObjId,
    organization: OrgId,
    scope: OrgScopeId,
}
impl TryFrom<AttributionWire> for HouseFilingAttribution {
    type Error = &'static str;
    fn try_from(w: AttributionWire) -> Result<Self, Self::Error> {
        if w.format != 1 || w.curator.0 == 0 {
            return Err("invalid house filing attribution");
        }
        Ok(Self {
            format: 1,
            curator: w.curator,
            organization: w.organization,
            scope: w.scope,
        })
    }
}
impl HouseFilingAttribution {
    pub fn new(
        curator: ObjId,
        organization: OrgId,
        scope: OrgScopeId,
    ) -> Result<Self, ParticipantError> {
        Self::try_from(AttributionWire {
            format: 1,
            curator,
            organization,
            scope,
        })
        .map_err(|_| refused())
    }
    pub const fn curator(&self) -> ObjId {
        self.curator
    }
    pub const fn organization(&self) -> OrgId {
        self.organization
    }
    pub const fn scope(&self) -> OrgScopeId {
        self.scope
    }
}

/// No fee, issuer, plan, grant or actor can be selected by this request.
pub struct RegistryHouseFiling {
    pub shell: ObjId,
    pub payer: AccountId,
    pub organization: OrgId,
    pub scope: OrgScopeId,
}
impl RegistryHouseFiling {
    pub(super) fn into_request(
        self,
        curator: ObjId,
    ) -> Result<(RegistryInitialFiling, HouseFilingAttribution), ParticipantError> {
        let attribution = HouseFilingAttribution::new(curator, self.organization, self.scope)?;
        Ok((
            RegistryInitialFiling {
                shell: self.shell,
                payer: self.payer,
                owner: PrincipalId::Organization(self.organization),
                scope: Some(self.scope),
            },
            attribution,
        ))
    }
}

pub(super) fn check(
    world: &World,
    state: &EconomyStore,
    actor: ObjId,
    request: &RegistryInitialFiling,
    house: &HouseFilingAttribution,
    fee: Amount,
) -> Result<(), ParticipantError> {
    let owner = PrincipalId::Organization(house.organization);
    let payer = state.snapshot.account(request.payer).ok_or_else(refused)?;
    if actor != house.curator
        || request.owner != owner
        || request.scope != Some(house.scope)
        || payer.owner != owner
        || payer.scope != Some(house.scope)
        || payer.status != crate::record::AccountStatus::Active
        || world
            .get(actor)
            .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(mush_core::Flags::GOING))
        || !crate::api::scope_coherent(world, Some(house.scope), &owner)
    {
        return Err(refused());
    }
    // Exact established org authorization, with explicit Allow and Deny
    // precedence. Staff/member/name status supplies no fallback here.
    for (name, amount) in [
        (HOUSE_PUBLICATION_CAPABILITY, None),
        ("asset.title.transfer", None),
        ("economy.account.post", Some(fee.minor())),
    ] {
        let capability = CapabilityId::new(name).ok_or_else(refused)?;
        let req = AuthRequest {
            actor,
            capability,
            scope: house.scope,
            now: world.now(),
            amount,
        };
        if !mush_org::authorize_in_world(world, &req).is_allow() {
            return Err(refused());
        }
    }
    Ok(())
}
