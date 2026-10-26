//! Exact historical action reconstruction; references are data, never grants.

use mush_core::mechanics::{DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId};
use mush_core::{DefinitionDescriptor, DefinitionRef};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::records::PublicationError;

fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Invalid
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transition {
    Open,
    Close,
}
impl Transition {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Close => "close",
        }
    }
    pub(crate) const fn action_id(self) -> &'static str {
        match self {
            Self::Open => "open_ic_mind",
            Self::Close => "close_ic_mind",
        }
    }
}

/// Ordered actual owner dependencies. No handles or public constructors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IcParticipants {
    pub(crate) retention: DefinitionRef,
    pub(crate) seal: DefinitionRef,
    pub(crate) custody: DefinitionRef,
    pub(crate) exclusion: DefinitionRef,
}
impl IcParticipants {
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        for (reference, (owner, id)) in self.ordered().iter().zip([
            ("roll", "commercial_roots"),
            ("tacsim", "ic_sortie"),
            ("tactics", "ic_custody"),
            ("sim", "ic_exclusion"),
        ]) {
            if reference.key().owner.as_str() != owner
                || reference.key().kind.as_str() != "owner_participant"
                || reference.key().id.as_str() != id
                || reference.version() != 1
            {
                return Err(PublicationError::Invalid);
            }
        }
        Ok(())
    }
    pub(crate) fn ordered(&self) -> [DefinitionRef; 4] {
        [
            self.retention.clone(),
            self.seal.clone(),
            self.custody.clone(),
            self.exclusion.clone(),
        ]
    }
}

/// Captured exact dependencies reconstruct old Open effects after configuration
/// changes. Current Close still requires its actual registered owner handle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IcProtocol {
    pub(crate) manifest: DefinitionRef,
    pub(crate) participants: IcParticipants,
}
impl IcProtocol {
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        self.participants.validate()?;
        let reference = &self.manifest;
        if reference.key().owner.as_str() != "tacsim"
            || reference.key().kind.as_str() != "registry_manifest"
            || reference.key().id.as_str() != "combat"
            || reference.version() != 1
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    pub(crate) fn descriptor(
        &self,
        transition: Transition,
    ) -> Result<DefinitionDescriptor, PublicationError> {
        self.validate()?;
        let mut references = vec![self.manifest.clone()];
        references.extend(self.participants.ordered());
        DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(bad)?,
            DefinitionId::new(transition.action_id()).map_err(bad)?,
            1,
            json!({"format":1,"protocol":"ic_mind_custody_v1","transition":transition.name()}),
        )
        .map(|descriptor| descriptor.with_references(references))
        .map_err(bad)
    }
    pub(crate) fn action(&self, transition: Transition) -> Result<DefinitionRef, PublicationError> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tacsim").map_err(bad)?,
                self.descriptor(transition)?,
            )
            .map_err(bad)
    }
}
