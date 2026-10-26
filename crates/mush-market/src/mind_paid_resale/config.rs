//! Explicit composition only. The configured service has no authority without
//! a retained private intent and the exact registered action guards.
use mush_core::{DefinitionRef, ModError, ObjId};
use mush_org::SystemPrincipalId;
use mush_roll::commerce::{
    paid_resale::{protocol, writer::PaidResaleWriter},
    purchase::PurchaseParticipants,
};

#[derive(Clone, Debug)]
pub struct PaidResaleConfig {
    pub(super) issuer: SystemPrincipalId,
    pub(super) service: ObjId,
    pub(super) offer_seconds: i64,
    pub(super) participants: PurchaseParticipants,
    pub(super) writer: PaidResaleWriter,
    pub(super) revocation: Option<DefinitionRef>,
}
impl PaidResaleConfig {
    pub fn new(
        issuer: SystemPrincipalId,
        service: ObjId,
        offer_seconds: i64,
        participants: PurchaseParticipants,
        writer: PaidResaleWriter,
        revocation: Option<DefinitionRef>,
    ) -> Result<Self, ModError> {
        if !(1..=mush_economy::managed_paid_resale::MAX_OFFER_SECONDS).contains(&offer_seconds) {
            return Err(ModError::msg("paid resale offer duration differs"));
        }
        let value = Self {
            issuer,
            service,
            offer_seconds,
            participants,
            writer,
            revocation,
        };
        value.economy()?;
        Ok(value)
    }
    pub fn with_registry_revocation(mut self, action: DefinitionRef) -> Result<Self, ModError> {
        if self.revocation.is_some() {
            return Err(ModError::msg("paid resale revocation already configured"));
        }
        self.revocation = Some(action);
        self.economy()?;
        Ok(self)
    }
    pub fn actions(&self) -> Result<[DefinitionRef; 5], ModError> {
        protocol::actions()
    }
    pub fn participants(&self) -> &PurchaseParticipants {
        &self.participants
    }
    pub fn writer(&self) -> PaidResaleWriter {
        self.writer.clone()
    }
    pub fn economy(&self) -> Result<mush_economy::managed_paid_resale::PaidResaleConfig, ModError> {
        mush_economy::managed_paid_resale::PaidResaleConfig::new(
            self.issuer.clone(),
            self.service,
            self.actions()?,
            self.revocation.clone(),
        )
        .map_err(|e| ModError::msg(e.to_string()))
    }
    pub(crate) fn declarations(&self) -> Result<Vec<mush_core::DefinitionDescriptor>, ModError> {
        protocol::definitions()
    }
}
