//! Tacsim's captured wire uses Roll's one exact closed descriptor recipe.
use super::CoverageError;
use mush_core::{DefinitionDescriptor, DefinitionRef};
use mush_roll::commerce::coverage::CoverageProtocol;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Protocol {
    pub(super) format: u16,
    pub(super) manifest: DefinitionRef,
    pub(super) participants: [DefinitionRef; 2],
}
impl Protocol {
    pub(super) fn roll(&self) -> CoverageProtocol {
        CoverageProtocol {
            format: self.format,
            manifest: self.manifest.clone(),
            participants: self.participants.clone(),
        }
    }
    pub(super) fn validate(&self) -> Result<(), CoverageError> {
        self.roll().validate().map_err(|_| CoverageError::Product)
    }
    pub(super) fn descriptor(&self, index: usize) -> Result<DefinitionDescriptor, CoverageError> {
        self.roll()
            .descriptor(index)
            .map_err(|_| CoverageError::Product)
    }
    pub(super) fn action(&self, index: usize) -> Result<DefinitionRef, CoverageError> {
        self.roll()
            .action(index)
            .map_err(|_| CoverageError::Product)
    }
    pub(super) fn actions(&self) -> Result<[DefinitionRef; 5], CoverageError> {
        self.roll().actions().map_err(|_| CoverageError::Product)
    }
}
