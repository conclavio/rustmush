use super::*;
use mush_core::ObjId;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
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
/// **One chain good a die consumes when it is issued** (doc 48 unit PL.7; Q-48-27, Q-48-48):
/// a pooled commodity and its units, taken from the importer's depot.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainLine {
    pub commodity: mush_core::CanonicalId,
    pub units: u64,
}
/// The most chain lines one class's die consumes.
pub const MAX_CHAIN_LINES: usize = 4;
/// The most units one chain line consumes.
pub const MAX_CHAIN_UNITS: u64 = 1_000;
/// The die lots' own commodity, which no chain line may name.
pub(crate) const DIE_COMMODITY: &str = "tacsim.electronics";

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
    /// **The chain goods a die of each class consumes when it is issued** (doc 48 unit PL.7),
    /// Class I, II and III in order, debited from [`Self::depot`] in the issue's bundle. Every
    /// certificate retains its policy, so the goods a die consumed are read back from it
    /// ([`super::DieCertificate::consumed`]). Empty for every class when the composition names
    /// none, and then not written, so such a policy keeps its bytes and digest.
    #[serde(default, skip_serializing_if = "no_chain")]
    pub chain: [Vec<ChainLine>; 3],
}
fn no_chain(chain: &[Vec<ChainLine>; 3]) -> bool {
    chain.iter().all(Vec::is_empty)
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
        // Each class's lines: bounded, in strictly ascending commodity order (so one good is
        // named once and the bytes are canonical), never the die's own commodity.
        for lines in &self.chain {
            if lines.len() > MAX_CHAIN_LINES
                || lines.iter().any(|l| {
                    !(1..=MAX_CHAIN_UNITS).contains(&l.units)
                        || l.commodity.as_str() == DIE_COMMODITY
                })
                || lines.windows(2).any(|w| w[0].commodity >= w[1].commodity)
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    /// The chain goods a die of `class` consumes at issue.
    #[must_use]
    pub fn chain_of(&self, class: super::DieClass) -> &[ChainLine] {
        &self.chain[class.index()]
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
    /// Doc 17h cut M8a fix round 1 (F6): the one other owner granted a read of the retained
    /// die certificates (tacsim's provenance read), when the composition names one.
    pub(crate) certificate_reader: Option<String>,
    /// Doc 31 UD.4b: further Industry owner participants composed beside the
    /// Origin's (the bay refit's), admitted by the same exact validator.
    pub(crate) further_participants: Vec<mush_core::DefinitionDescriptor>,
}
impl CoreOriginConfig {
    pub fn new(policy: OriginPolicy, mode: CoreOriginMode) -> Result<Self, ModError> {
        policy.validate().map_err(mod_error)?;
        Ok(Self {
            policy,
            mode,
            certificate_reader: None,
            further_participants: Vec::new(),
        })
    }
    pub fn policy(&self) -> &OriginPolicy {
        &self.policy
    }
    /// **Grant `reader` a read of the retained die certificates** (doc 17h cut M8a fix
    /// round 1, F6): a read-only grant on `industry_core_die_certificates`, published as the
    /// [`super::DieCertificates`] slot. The reader must be composed beside this owner, or the
    /// registries refuse the grant when they freeze. A read grants nothing.
    #[must_use]
    pub fn with_certificate_reader(mut self, reader: &str) -> Self {
        self.certificate_reader = Some(reader.to_owned());
        self
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
