//! Fixed declaration data. Definition equality includes the actual manifest
//! and two exact participant references; a locator is not a grant.
//!
//! Doc 46 CT.5: the generic owner names a transition by [`Step`]; each family keeps its own
//! `Transition` (its callers and the market name it, and `id` answers the family's action),
//! expanded from `entry_points!` with the family's four forwarders.
use super::super::purchase::PurchaseParticipants;
use super::Issuer;
use mush_core::{
    DefinitionRef, ModError,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
};
use mush_economy::renewal_twin::Issuer as Economy;

/// One of the five ordinary transitions, named apart from its issuer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Draft,
    Fund,
    Grant,
    Refund,
    Close,
}
impl Step {
    pub const ALL: [Self; 5] = [
        Self::Draft,
        Self::Fund,
        Self::Grant,
        Self::Refund,
        Self::Close,
    ];
    /// The issuer's action id of this transition (Economy's constants, the same strings).
    pub const fn id<I: Issuer>(self) -> &'static str {
        match self {
            Self::Draft => <I as Economy>::DRAFT_ACTION,
            Self::Fund => <I as Economy>::FUND_ACTION,
            Self::Grant => <I as Economy>::GRANT_ACTION,
            Self::Refund => <I as Economy>::REFUND_ACTION,
            Self::Close => <I as Economy>::CLOSE_ACTION,
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
    pub(crate) fn select(self, refs: &[DefinitionRef; 5]) -> &DefinitionRef {
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
pub fn descriptor<I: Issuer>(
    transition: Step,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    participants.validate().map_err(error)?;
    Ok(DefinitionDescriptor::new(DefinitionKind::new("action").map_err(error)?,
        DefinitionId::new(transition.id::<I>()).map_err(error)?,1,
        serde_json::json!({"format":1,"protocol":<I as Economy>::PROTOCOL,"transition":transition.name()}))
        .map_err(error)?.with_references(vec![manifest.clone(),participants.economy().clone(),participants.retention().clone()]))
}
pub(crate) fn validate_refs<I: Issuer>(
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
    refs: &[DefinitionRef; 5],
) -> Result<(), ModError> {
    for transition in Step::ALL {
        let expected = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("market").map_err(error)?,
                descriptor::<I>(transition, manifest, participants)?,
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
pub fn service_descriptor<I: Issuer>(
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    participants.validate().map_err(error)?;
    DefinitionDescriptor::new(DefinitionKind::new("action").map_err(error)?,
        DefinitionId::new(<I as Economy>::SERVICE_CLOSE_ACTION).map_err(error)?, 1,
        serde_json::json!({"format":1,"protocol":<I as Economy>::SERVICE_PROTOCOL,"transition":"unfunded_service_close","service_actor":1}))
        .map_err(error).map(|d| d.with_references(vec![manifest.clone(), participants.economy().clone(), participants.retention().clone()]))
}
pub fn service_reference<I: Issuer>(
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("market").map_err(error)?,
            service_descriptor::<I>(manifest, participants)?,
        )
        .map_err(error)
}

/// A family's `protocol` module: its `Transition` and the four functions its callers name,
/// forwarding to the generic ones at the family's issuer.
macro_rules! entry_points {
    ($issuer:ty) => {
        use mush_core::{DefinitionRef, ModError, mechanics::DefinitionDescriptor};
        use $crate::commerce::purchase::PurchaseParticipants;
        use $crate::commerce::renewal_twin::protocol::Step;

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
                self.step().id::<$issuer>()
            }
            #[cfg(test)]
            pub(crate) fn select(self, refs: &[DefinitionRef; 5]) -> &DefinitionRef {
                self.step().select(refs)
            }
            /// The transition named apart from its issuer.
            pub const fn step(self) -> Step {
                match self {
                    Self::Draft => Step::Draft,
                    Self::Fund => Step::Fund,
                    Self::Grant => Step::Grant,
                    Self::Refund => Step::Refund,
                    Self::Close => Step::Close,
                }
            }
        }
        impl From<Transition> for $crate::commerce::renewal_twin::roots::Publication {
            fn from(value: Transition) -> Self {
                Self::Ordinary(value.step())
            }
        }
        pub fn descriptor(
            transition: Transition,
            manifest: &DefinitionRef,
            participants: &PurchaseParticipants,
        ) -> Result<DefinitionDescriptor, ModError> {
            $crate::commerce::renewal_twin::protocol::descriptor::<$issuer>(
                transition.step(),
                manifest,
                participants,
            )
        }
        pub(crate) fn validate_refs(
            manifest: &DefinitionRef,
            participants: &PurchaseParticipants,
            refs: &[DefinitionRef; 5],
        ) -> Result<(), ModError> {
            $crate::commerce::renewal_twin::protocol::validate_refs::<$issuer>(
                manifest,
                participants,
                refs,
            )
        }
        /// Optional sixth action. The original five descriptors remain byte-identical.
        pub fn service_descriptor(
            manifest: &DefinitionRef,
            participants: &PurchaseParticipants,
        ) -> Result<DefinitionDescriptor, ModError> {
            $crate::commerce::renewal_twin::protocol::service_descriptor::<$issuer>(
                manifest,
                participants,
            )
        }
        pub fn service_reference(
            manifest: &DefinitionRef,
            participants: &PurchaseParticipants,
        ) -> Result<DefinitionRef, ModError> {
            $crate::commerce::renewal_twin::protocol::service_reference::<$issuer>(
                manifest,
                participants,
            )
        }
    };
}
pub(crate) use entry_points;
