//! Closed optional pending personal charter escrow. Actual named-buyer funding
//! and private service or Registry terminals share acknowledged owner custody.
//! Default composition remains disabled; no automatic buyer acceptance.
pub(crate) mod audit;
pub(crate) mod compound;
pub(crate) mod current;
pub(crate) mod history;
pub(crate) mod index;
pub(crate) mod protocol;
mod provider;
pub(crate) mod publication;
pub(crate) mod records;
pub(crate) mod service;

pub(crate) use super::charter_transfer::Readiness;
use super::records::PublicationError;
use mush_core::{ModError, ObjId};
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}

/// Explicit trusted composition. Funding is always the named buyer's action;
/// the service receives authority only for its retained exact terminal intent.
#[derive(Clone, Debug)]
pub struct PendingCharterTransferConfig {
    pub(crate) service_actor: ObjId,
    pub(crate) settle_seconds: i64,
}
impl PendingCharterTransferConfig {
    pub fn new(service_actor: ObjId, settle_seconds: i64) -> Result<Self, ModError> {
        if service_actor.0 == 0 || !(1..=259200).contains(&settle_seconds) {
            return Err(ModError::msg(
                "pending charter service configuration differs",
            ));
        }
        Ok(Self {
            service_actor,
            settle_seconds,
        })
    }
    pub fn actions(&self) -> Result<[mush_core::DefinitionRef; 4], ModError> {
        Ok([
            protocol::offer_action()?,
            protocol::fund_action()?,
            protocol::settle_action()?,
            protocol::refund_action()?,
        ])
    }
    pub fn economy_config(
        &self,
        issuer: mush_org::SystemPrincipalId,
        catalog: mush_economy::CatalogRef,
        registry_revocation: Option<mush_core::DefinitionRef>,
    ) -> Result<mush_economy::managed_charter_transfer::CharterPendingConfig, ModError> {
        mush_economy::managed_charter_transfer::CharterPendingConfig::new(
            issuer,
            catalog,
            self.service_actor,
            [
                protocol::fund_action()?,
                protocol::settle_action()?,
                protocol::refund_action()?,
            ],
            registry_revocation,
        )
        .map_err(|_| ModError::msg("pending charter native configuration differs"))
    }
}

#[cfg(test)]
mod tests;

pub(crate) use provider::register;
