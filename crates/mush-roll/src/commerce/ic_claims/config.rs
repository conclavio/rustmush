//! Exact configuration, not a writer or a readiness capability.
use mush_core::mechanics::DefinitionRegistryBuilder;
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, ModError, OwnerId,
};

use super::{IcParticipants, IcProtocol, Transition};

fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

#[derive(Clone, Debug)]
pub struct IcClaimConfig {
    pub(crate) protocol: IcProtocol,
}
impl IcClaimConfig {
    /// Trusted host configuration must supply the actual Tacsim seal descriptor.
    /// Every other participant is the existing concrete fixed owner type.
    pub fn new(manifest: DefinitionRef, seal: DefinitionRef) -> Result<Self, ModError> {
        let reference = |owner: &str, d: DefinitionDescriptor| {
            DefinitionRegistryBuilder::new()
                .register(OwnerId::new(owner).map_err(bad)?, d)
                .map_err(bad)
        };
        // This is the frozen public declaration shared with Core's fixed IC
        // verifier. The actual registered concrete adapter is still mandatory.
        let expected = reference(
            "tacsim",
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant").map_err(bad)?,
                DefinitionId::new("ic_sortie").map_err(bad)?,
                1,
                serde_json::json!({"format":1,"protocol":"ic_sortie_v1","max_open":1024,
                "max_history":8192,"max_record_bytes":1048576,"max_total_bytes":67108864}),
            )
            .map_err(bad)?,
        )?;
        if seal != expected {
            return Err(bad("IC seal descriptor differs"));
        }
        let protocol = IcProtocol {
            manifest,
            participants: IcParticipants {
                retention: super::super::authored_retirement::retention_reference()?,
                seal,
                custody: reference(
                    "tactics",
                    mush_tactics::ic_custody::participant_descriptor()?,
                )?,
                exclusion: reference("sim", mush_sim::ic_exclusion::participant_descriptor()?)?,
            },
        };
        protocol.validate().map_err(bad)?;
        Ok(Self { protocol })
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.protocol.manifest
    }
    pub fn participants(&self) -> [DefinitionRef; 4] {
        self.protocol.participants.ordered()
    }
    pub fn actions(&self) -> Result<[DefinitionRef; 2], ModError> {
        Ok([
            self.protocol.action(Transition::Open).map_err(bad)?,
            self.protocol.action(Transition::Close).map_err(bad)?,
        ])
    }
    pub fn action_descriptors(&self) -> Result<[DefinitionDescriptor; 2], ModError> {
        Ok([
            self.protocol.descriptor(Transition::Open).map_err(bad)?,
            self.protocol.descriptor(Transition::Close).map_err(bad)?,
        ])
    }
}
