use super::*;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionDigest, ObjId};
use mush_economy::VaultId;
use mush_inventory::Grade;
use mush_org::{OrgId, OrgScopeId};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreOriginMode {
    Initialize,
    Active,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginAgent {
    pub actor: ObjId,
    pub stock: VaultId,
}
/// Trusted composition data, retained in each receipt for historical reconstruction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginPolicy {
    pub importer: OrgId,
    pub scope: OrgScopeId,
    pub depot: VaultId,
    pub agents: [OriginAgent; 2],
    pub content: DefinitionRef,
    pub grades: [Grade; 3],
    pub monthly_quotas: [u16; 3],
}
impl OriginPolicy {
    pub fn validate(&self) -> Result<(), Error> {
        if self
            .agents
            .iter()
            .any(|a| a.actor.0 == 0 || a.stock == self.depot)
            || self.agents[0].actor == self.agents[1].actor
            || self.agents[0].stock == self.agents[1].stock
            || self.monthly_quotas.iter().any(|n| !(1..=1024).contains(n))
            || self.content.key().kind.as_str() != "content"
            || self.content.key().id.as_str() != "core_die_stock"
            || self.content.version() != 1
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "industry.core_origin_policy.v1",
            &encode(self).map_err(bad)?,
        )
        .map_err(bad)
    }
    pub(crate) fn action(&self, initialize: bool) -> Result<DefinitionDescriptor, ModError> {
        self.validate().map_err(mod_error)?;
        descriptor("action",if initialize{INITIALIZE_ACTION}else{ISSUE_ACTION},json!({"format":1,"protocol":"certified_origin_stock_v1","policy":self,"quantity":1,"initialize":initialize}))
   .map(|d|d.with_references(vec![self.content.clone()]))
    }
    pub(crate) fn action_ref(&self, initialize: bool) -> Result<DefinitionRef, Error> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(bad)?,
                self.action(initialize).map_err(bad)?,
            )
            .map_err(bad)
    }
    pub(crate) fn agent(&self, actor: ObjId) -> Result<&OriginAgent, Error> {
        self.agents
            .iter()
            .find(|a| a.actor == actor)
            .ok_or(Error::Unavailable)
    }
}
#[derive(Clone, Debug)]
pub struct CoreOriginConfig {
    pub(crate) policy: OriginPolicy,
    pub(crate) mode: CoreOriginMode,
}
impl CoreOriginConfig {
    pub fn new(policy: OriginPolicy, mode: CoreOriginMode) -> Result<Self, ModError> {
        policy.validate().map_err(mod_error)?;
        Ok(Self { policy, mode })
    }
    pub fn policy(&self) -> &OriginPolicy {
        &self.policy
    }
}
pub fn origin_content_descriptor(grades: [Grade; 3]) -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "content",
        "core_die_stock",
        json!({"format":1,"commodity":"tacsim.electronics","classes":[{"class":"i","label":"die.c1","grade":grades[0]},{"class":"ii","label":"die.c2","grade":grades[1]},{"class":"iii","label":"die.c3","grade":grades[2]}],"quantity":1,"certification":"industry_origin_receipt_v1"}),
    )
}
pub(crate) fn descriptor(
    kind: &str,
    name: &str,
    payload: Value,
) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(mod_error)?,
        DefinitionId::new(name).map_err(mod_error)?,
        1,
        payload,
    )
    .map_err(mod_error)
}
