//! Fixed declaration data. Definition equality includes the actual manifest
//! and two exact participant references; a locator is not a grant.
use super::super::purchase::PurchaseParticipants;
use mush_core::{
    DefinitionRef, ModError,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Draft,
    Fund,
    Grant,
    Refund,
    Close,
}
impl Transition {
    pub const ALL: [Self; 5] = [
        Self::Draft,
        Self::Fund,
        Self::Grant,
        Self::Refund,
        Self::Close,
    ];
    pub const fn id(self) -> &'static str {
        match self {
            Self::Draft => "draft_house_mind_renewal",
            Self::Fund => "fund_house_mind_renewal",
            Self::Grant => "settle_house_mind_renewal",
            Self::Refund => "refund_house_mind_renewal",
            Self::Close => "close_house_mind_renewal",
        }
    }
    pub(crate) const fn fresh(self) -> bool {
        matches!(self, Self::Draft | Self::Fund | Self::Grant)
    }
    const fn name(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Fund => "fund",
            Self::Grant => "grant",
            Self::Refund => "refund",
            Self::Close => "unfunded_close",
        }
    }
    pub(crate) fn select<'a>(self, refs: &'a [DefinitionRef; 5]) -> &'a DefinitionRef {
        let [draft, fund, grant, refund, close] = refs;
        match self {
            Self::Draft => draft,
            Self::Fund => fund,
            Self::Grant => grant,
            Self::Refund => refund,
            Self::Close => close,
        }
    }
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub fn descriptor(
    transition: Transition,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    participants.validate().map_err(error)?;
    Ok(DefinitionDescriptor::new(DefinitionKind::new("action").map_err(error)?,
        DefinitionId::new(transition.id()).map_err(error)?,1,
        serde_json::json!({"format":1,"protocol":"house_mind_renewal_v1","transition":transition.name()}))
        .map_err(error)?.with_references(vec![manifest.clone(),participants.economy().clone(),participants.retention().clone()]))
}
pub(crate) fn validate_refs(
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
    refs: &[DefinitionRef; 5],
) -> Result<(), ModError> {
    for transition in Transition::ALL {
        let expected = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("market").map_err(error)?,
                descriptor(transition, manifest, participants)?,
            )
            .map_err(error)?;
        if transition.select(refs) != &expected {
            return Err(ModError::msg(
                "renewal action differs from the fixed protocol",
            ));
        }
    }
    Ok(())
}

/// Optional sixth action. The original five descriptors remain byte-identical.
pub fn service_descriptor(
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    participants.validate().map_err(error)?;
    DefinitionDescriptor::new(DefinitionKind::new("action").map_err(error)?,
        DefinitionId::new(mush_economy::managed_house_renewal::SERVICE_CLOSE_ACTION).map_err(error)?, 1,
        serde_json::json!({"format":1,"protocol":"house_mind_renewal_service_v1","transition":"unfunded_service_close","service_actor":1}))
        .map_err(error).map(|d| d.with_references(vec![manifest.clone(), participants.economy().clone(), participants.retention().clone()]))
}
pub fn service_reference(
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("market").map_err(error)?,
            service_descriptor(manifest, participants)?,
        )
        .map_err(error)
}
