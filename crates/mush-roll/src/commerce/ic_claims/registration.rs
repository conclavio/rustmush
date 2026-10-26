//! Trusted closed type selection; no callback, data grant or readiness mint.
use super::super::{
    families::CommerceFamilies,
    rating::{RatingCompatibility, RatingFamilies},
    records::PublicationError,
};
use super::{ClaimInventory, IcClaimConfig, IcSealOutput, IcSealParticipant, IcWriter};
use crate::RollStore;
use mush_core::{ModError, ModuleCtx, Registrar};
use std::marker::PhantomData;

pub(crate) trait Installer: Send + Sync + std::fmt::Debug {
    fn config(&self) -> &IcClaimConfig;
    fn register(
        &self,
        reg: &mut Registrar<'_>,
        f: CommerceFamilies,
        r: RatingFamilies,
        c: RatingCompatibility,
    ) -> Result<(), ModError>;
    fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        roll: &RollStore,
        f: &CommerceFamilies,
        r: &RatingFamilies,
    ) -> Result<ClaimInventory, PublicationError>;
}
pub(crate) struct Selected<S> {
    config: IcClaimConfig,
    writer: IcWriter,
    marker: PhantomData<fn() -> S>,
}
impl<S> std::fmt::Debug for Selected<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IcSelectedOwners")
            .field("config", &self.config)
            .finish()
    }
}
impl<S> Selected<S> {
    pub(crate) fn new(config: IcClaimConfig, writer: IcWriter) -> Self {
        Self {
            config,
            writer,
            marker: PhantomData,
        }
    }
}
impl<S: IcSealParticipant> Installer for Selected<S>
where
    S::Output: IcSealOutput,
{
    fn config(&self) -> &IcClaimConfig {
        &self.config
    }
    fn register(
        &self,
        reg: &mut Registrar<'_>,
        f: CommerceFamilies,
        r: RatingFamilies,
        c: RatingCompatibility,
    ) -> Result<(), ModError> {
        super::reader::register(reg, f.clone(), r, self.config.manifest(), c)?;
        self.writer.install(reg, f, self.config.clone())
    }
    fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        roll: &RollStore,
        f: &CommerceFamilies,
        r: &RatingFamilies,
    ) -> Result<ClaimInventory, PublicationError> {
        let rows = super::audit::rows(ctx, f)?;
        let inventory = super::audit::local(ctx, roll, f, r, &rows)?;
        super::audit::physical(ctx.w, &rows)?;
        // Mandatory even when all typed rows are absent: no retained seal may
        // be dropped merely because its companion projections disappeared.
        S::audit_claims(ctx.w, &rows, inventory.next_transition())
            .map_err(|_| PublicationError::Mismatch)?;
        Ok(inventory)
    }
}
