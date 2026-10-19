//! Optional source-free transfer of an already issued paid licence.
//! Birth/accepted terms remain immutable; these records do not confer authority.
pub(crate) mod audit;
pub use audit::HistoricalBinding;
pub(crate) mod chain;
pub(crate) mod compound;
mod economics;
pub(crate) mod history;
mod market;
pub use market::PaidResaleMarketWriter;
pub(crate) mod publication;
pub mod reader;
pub(crate) mod service;
pub mod writer;
pub use economics::{ResaleEconomics, ResaleParentRate, ResaleRates};
pub mod protocol;
mod records;
pub use records::{
    PaidHolderRecord, PaidResaleBinding, PaidResaleHead, PaidResaleRecord, PaidResaleRef,
    PaidResaleState, ResaleClaimObservation,
};

#[derive(Clone, Default)]
pub(crate) struct Readiness(std::sync::Arc<std::sync::OnceLock<mush_core::DefinitionRegistry>>);
impl Readiness {
    pub(crate) fn capture(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<(), mush_core::ModError> {
        let definitions = ctx
            .reg
            .definitions()
            .ok_or_else(|| mush_core::ModError::msg("paid resale definitions unavailable"))?;
        self.0
            .set(definitions.clone())
            .map_err(|_| mush_core::ModError::msg("paid resale readiness already captured"))
    }
    pub(crate) fn check(&self) -> Result<(), crate::commerce::records::PublicationError> {
        use crate::commerce::records::PublicationError;
        let definitions = self.0.get().ok_or(PublicationError::Unavailable)?;
        for owner in ["roll", "market", "economy"] {
            definitions
                .require_owner_ready(
                    &mush_core::OwnerId::new(owner).map_err(|_| PublicationError::Invalid)?,
                )
                .map_err(|_| PublicationError::Unavailable)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
