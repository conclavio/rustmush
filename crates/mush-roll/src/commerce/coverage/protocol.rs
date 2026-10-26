//! Exact closed action references. Captured references reconstruct history;
//! registration and current owners separately establish actual authority.
use crate::commerce::records::PublicationError;
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    OwnerId,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const ACTIONS: [&str; 5] = [
    "publish_hull_coverage",
    "quote_hull_coverage",
    "bind_hull_coverage",
    "claim_hull_coverage",
    "release_hull_coverage",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageProtocol {
    pub format: u16,
    pub manifest: DefinitionRef,
    pub participants: [DefinitionRef; 2],
}
impl CoverageProtocol {
    pub fn validate(&self) -> Result<(), PublicationError> {
        let [economy, roots] = &self.participants;
        if self.format != 1
            || self.manifest.key().owner.as_str() != "tacsim"
            || self.manifest.key().kind.as_str() != "registry_manifest"
            || self.manifest.key().id.as_str() != "combat"
            || self.manifest.version() != 1
            || economy.key().owner.as_str() != "economy"
            || economy.key().kind.as_str() != "owner_participant"
            || economy.key().id.as_str() != "initial_filing"
            || economy.version() != 1
        {
            return Err(PublicationError::Invalid);
        }
        let exact = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("roll").map_err(|_| PublicationError::Invalid)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant")
                        .map_err(|_| PublicationError::Invalid)?,
                    DefinitionId::new("commercial_roots").map_err(|_| PublicationError::Invalid)?,
                    1,
                    json!({"format":1,"protocol":"commercial_retention_v1"}),
                )
                .map_err(|_| PublicationError::Invalid)?,
            )
            .map_err(|_| PublicationError::Invalid)?;
        if roots != &exact {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    pub fn publication_participant_descriptor() -> Result<DefinitionDescriptor, PublicationError> {
        DefinitionDescriptor::new(DefinitionKind::new("owner_participant").map_err(|_|PublicationError::Invalid)?,
            DefinitionId::new("coverage_metadata").map_err(|_|PublicationError::Invalid)?,1,
            json!({"format":1,"protocol":"hull_coverage_publication_owner_v1","authority":"actual_scoped_org","records":"immutable_publication","monetary":false}))
            .map_err(|_|PublicationError::Invalid)
    }
    pub fn descriptor(&self, index: usize) -> Result<DefinitionDescriptor, PublicationError> {
        self.validate()?;
        let id = ACTIONS.get(index).ok_or(PublicationError::Invalid)?;
        let payload = match index {
            0 => {
                json!({"format":1,"protocol":"hull_coverage_publication_v1","authority":"actual_scoped_org_publisher","product":"immutable_exact_approvals","acceptance":"native_org_product_response"})
            }
            1 => {
                json!({"format":1,"protocol":"hull_coverage_quote_v1","authority":"actual_personal_dispatcher","pricing":"exact_publication_and_private_role_proof","window":"duration_only","activation":"acknowledged_ic_and_withheld_departure"})
            }
            2 => {
                json!({"format":1,"protocol":"hull_coverage_bind_v1","authority":"buyer_and_private_withheld_departure","start":"actual_world_ic","premium":"same_ack","liability":"pre_premium_funded_hold","retention":"same_ack","future_terminal_reserved":true})
            }
            3 => {
                json!({"format":1,"protocol":"hull_coverage_claim_v1","authority":"actual_private_host_loss","loss":"recorded_ic_destruction","window":"captured_original_clock_interval","delay":"captured_loss_plus_delay","wreck_title":"same_ack_insurer"})
            }
            4 => {
                json!({"format":1,"protocol":"hull_coverage_release_v1","authority":"actual_private_close_and_no_covered_loss","causes":["clean_close","cancelled","expired_clean"],"premium":"retained","collateral":"released_same_ack"})
            }
            _ => return Err(PublicationError::Invalid),
        };
        let mut references = if index == 0 {
            vec![
                self.manifest.clone(),
                DefinitionRegistryBuilder::new()
                    .register(
                        OwnerId::new("tacsim").map_err(|_| PublicationError::Invalid)?,
                        Self::publication_participant_descriptor()?,
                    )
                    .map_err(|_| PublicationError::Invalid)?,
            ]
        } else {
            vec![self.manifest.clone(), self.participants[1].clone()]
        };
        if index >= 2 {
            references.push(self.participants[0].clone());
        }
        references.sort();
        DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(|_| PublicationError::Invalid)?,
            DefinitionId::new(id).map_err(|_| PublicationError::Invalid)?,
            1,
            payload,
        )
        .map(|d| d.with_references(references))
        .map_err(|_| PublicationError::Invalid)
    }
    pub fn action(&self, index: usize) -> Result<DefinitionRef, PublicationError> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tacsim").map_err(|_| PublicationError::Invalid)?,
                self.descriptor(index)?,
            )
            .map_err(|_| PublicationError::Invalid)
    }
    pub fn actions(&self) -> Result<[DefinitionRef; 5], PublicationError> {
        Ok([
            self.action(0)?,
            self.action(1)?,
            self.action(2)?,
            self.action(3)?,
            self.action(4)?,
        ])
    }
}
