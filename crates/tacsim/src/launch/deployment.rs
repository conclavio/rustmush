//! Strict host coordinates. Decoding config creates no world object or right.
use mush_core::{ModError, ObjId};
use mush_economy::{
    AccountId, VaultId, core_integration::ScopedCoreAccount, managed_entitlement::Recipient,
};
use mush_faction::FactionId;
use mush_industry::core_production::OriginAgent;
use mush_inventory::Grade;
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use serde::{Deserialize, Serialize};

pub const MAX_DEPLOYMENT_BYTES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    Initialize,
    Active,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HouseDeployment {
    pub organization: OrgId,
    pub scope: OrgScopeId,
    pub treasury: AccountId,
    pub publisher: ObjId,
    pub source_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedHouses {
    pub meridiani: HouseDeployment,
    pub coprates: HouseDeployment,
    pub tycho: HouseDeployment,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchVenue {
    pub name: mush_market::MarketId,
    pub room: ObjId,
    pub recipient: Recipient,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareCoordinates {
    pub importer: OrgId,
    pub stock_scope: OrgScopeId,
    pub depot: VaultId,
    pub agents: [OriginAgent; 2],
    pub grades: [Grade; 3],
    pub supplier_account: AccountId,
    pub integrator: ScopedCoreAccount,
    pub factory: ObjId,
    pub output_container: ObjId,
}
impl HardwareCoordinates {
    pub(super) fn deployment(
        &self,
        issuer: &SystemPrincipalId,
    ) -> crate::hardware::HardwareDeployment {
        crate::hardware::HardwareDeployment {
            importer: self.importer,
            stock_scope: self.stock_scope,
            depot: self.depot,
            agents: self.agents.clone(),
            grades: self.grades,
            supplier_account: self.supplier_account,
            integrator: self.integrator.clone(),
            issuer: issuer.clone(),
            factory: self.factory,
            output_container: self.output_container,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchDeployment {
    pub format: u16,
    pub mode: LaunchMode,
    pub registry_issuer: SystemPrincipalId,
    pub registry: Recipient,
    pub venue: LaunchVenue,
    pub houses: NamedHouses,
    pub republic: FactionId,
    pub hardware: HardwareCoordinates,
}
impl LaunchDeployment {
    pub fn decode(bytes: &[u8]) -> Result<Self, ModError> {
        if bytes.is_empty() || bytes.len() > MAX_DEPLOYMENT_BYTES {
            return Err(ModError::msg(
                "launch deployment exceeds its bounded document",
            ));
        }
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|_| ModError::msg("invalid launch deployment document"))?;
        value.validate()?;
        Ok(value)
    }
    pub(super) fn validate(&self) -> Result<(), ModError> {
        let houses = [
            &self.houses.meridiani,
            &self.houses.coprates,
            &self.houses.tycho,
        ];
        if self.format != 1 || self.registry.account == self.venue.recipient.account {
            return Err(ModError::msg("launch deployment identity differs"));
        }
        for (index, house) in houses.iter().enumerate() {
            if mush_roll::canonical_name(&house.source_name).is_err()
                || house.source_name.contains('=')
                || houses.iter().take(index).any(|other| {
                    other.organization == house.organization
                        || other.treasury == house.treasury
                        || (other.publisher == house.publisher
                            && other.source_name.eq_ignore_ascii_case(&house.source_name))
                })
            {
                return Err(ModError::msg(
                    "named launch houses must have distinct exact identities",
                ));
            }
        }
        Ok(())
    }
}
