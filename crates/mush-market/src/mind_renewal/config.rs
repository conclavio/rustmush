//! Explicit personal Pin renewal composition. Prices and licence terms always
//! come from the immutable acquired template, never this host fee policy.
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use mush_economy::managed_entitlement::GrossRecipientCut;
use mush_roll::commerce::{
    purchase::PurchaseParticipants,
    renewal::{
        protocol::{self, Transition},
        writer::RenewalWriter,
    },
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct RenewalPolicy {
    pub registry: GrossRecipientCut,
    pub venue: Option<GrossRecipientCut>,
    pub offer_ic_seconds: i64,
}
impl RenewalPolicy {
    pub fn validate(&self) -> Result<(), ModError> {
        let cut = |c: &GrossRecipientCut| {
            c.permille <= 1000 && (c.permille == 0) == c.recipient.is_none()
        };
        if !cut(&self.registry)
            || self.venue.as_ref().is_some_and(|c| !cut(c))
            || u32::from(self.registry.permille)
                + u32::from(self.venue.as_ref().map_or(0, |c| c.permille))
                > 1000
            || !(1..=mush_economy::managed_renewal::MAX_RENEWAL_OFFER_IC_SECONDS)
                .contains(&self.offer_ic_seconds)
        {
            return Err(error("invalid renewal policy"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct RenewalConfig {
    pub(super) manifest: DefinitionRef,
    pub(super) participants: PurchaseParticipants,
    pub(super) writer: RenewalWriter,
    pub(super) policy: RenewalPolicy,
    pub(super) service: Option<super::service::ServiceConfig>,
}
pub(super) fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(super) fn descriptor(
    kind: &str,
    id: &str,
    payload: serde_json::Value,
) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(error)?,
        DefinitionId::new(id).map_err(error)?,
        1,
        payload,
    )
    .map_err(error)
}
impl RenewalConfig {
    pub fn new(
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        writer: RenewalWriter,
        policy: RenewalPolicy,
    ) -> Result<Self, ModError> {
        policy.validate()?;
        let value = Self {
            manifest,
            participants,
            writer,
            policy,
            service: None,
        };
        value.actions()?;
        Ok(value)
    }
    /// Explicit service opt-in. This never authorizes automatic acceptance.
    pub fn with_service(mut self) -> Result<Self, ModError> {
        if self.service.is_some() {
            return Err(error("renewal service already configured"));
        }
        self.service = Some(super::service::ServiceConfig::default());
        self.service_action()?;
        Ok(self)
    }
    pub fn service_action(&self) -> Result<Option<DefinitionRef>, ModError> {
        self.service
            .as_ref()
            .map(|_| protocol::service_reference(&self.manifest, &self.participants))
            .transpose()
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
    pub fn participants(&self) -> &PurchaseParticipants {
        &self.participants
    }
    pub fn writer(&self) -> RenewalWriter {
        self.writer.clone()
    }
    pub fn actions(&self) -> Result<[DefinitionRef; 5], ModError> {
        let action = |t| {
            DefinitionRegistryBuilder::new()
                .register(
                    OwnerId::new(crate::NS).map_err(error)?,
                    protocol::descriptor(t, &self.manifest, &self.participants)?,
                )
                .map_err(error)
        };
        let refs = [
            action(Transition::Draft)?,
            action(Transition::Fund)?,
            action(Transition::Grant)?,
            action(Transition::Refund)?,
            action(Transition::Close)?,
        ];
        Ok(refs)
    }
    pub(super) fn policy_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        Ok(descriptor(
            "renewal_policy",
            "personal",
            serde_json::to_value(&self.policy).map_err(error)?,
        )?
        .with_references(vec![self.manifest.clone()]))
    }
    pub(super) fn policy_ref(&self) -> Result<DefinitionRef, ModError> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                self.policy_descriptor()?,
            )
            .map_err(error)
    }
    pub(super) fn outcome_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        descriptor(
            "outcome",
            "mind_renewal_recorded",
            serde_json::json!({"format":1,"fields":["kind","operation","commit"]}),
        )
    }
    pub(crate) fn declarations(&self) -> Result<Vec<DefinitionDescriptor>, ModError> {
        let mut all = vec![self.outcome_descriptor()?];
        for t in [
            Transition::Draft,
            Transition::Fund,
            Transition::Grant,
            Transition::Refund,
            Transition::Close,
        ] {
            all.push(protocol::descriptor(t, &self.manifest, &self.participants)?);
        }
        if self.service.is_some() {
            all.push(protocol::service_descriptor(
                &self.manifest,
                &self.participants,
            )?);
        }
        Ok(all)
    }
}
