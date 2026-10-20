//! Trusted composition selects the actual owner once. This is not a recovered
//! health bit, a caller-selectable callback or a decoded qualification.
use super::{
    protocol::{ConductHistoryRow, ConductQuotaParticipant},
    *,
};
use crate::commerce::rating::{RatingCompatibility, RatingFamilies};
use mush_core::{DefinitionRef, ModuleCtx, World};
use std::{marker::PhantomData, sync::Arc};

pub(crate) trait Native: Send + Sync {
    fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        rows: &[ConductHistoryRow<'_>],
    ) -> Result<(), PublicationError>;
    fn check(&self, world: &World, row: ConductHistoryRow<'_>) -> Result<(), PublicationError>;
}
struct Actual<Q>(PhantomData<fn() -> Q>);
impl<Q: ConductQuotaParticipant> Native for Actual<Q> {
    fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        rows: &[ConductHistoryRow<'_>],
    ) -> Result<(), PublicationError> {
        Q::audit_history(ctx, rows)
    }
    fn check(&self, world: &World, row: ConductHistoryRow<'_>) -> Result<(), PublicationError> {
        Q::check_history(world, row)
    }
}
pub(crate) fn actual<Q: ConductQuotaParticipant>() -> Arc<dyn Native> {
    Arc::new(Actual::<Q>(PhantomData))
}

#[derive(Clone)]
pub(crate) struct Configuration {
    pub(crate) native: Arc<dyn Native>,
    pub(crate) ratings: RatingFamilies,
    pub(crate) expected: RatingCompatibility,
    pub(crate) manifest: DefinitionRef,
}
#[derive(Clone, Default)]
pub(crate) struct State {
    configuration: Option<Configuration>,
}
impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConductState")
            .field("configured", &self.configuration.is_some())
            .finish()
    }
}
impl State {
    pub(crate) fn configure(
        &mut self,
        configuration: Configuration,
    ) -> Result<(), PublicationError> {
        if self.configuration.is_some() {
            return Err(PublicationError::Mismatch);
        }
        self.configuration = Some(configuration);
        Ok(())
    }
    pub(crate) fn configuration(&self) -> Result<&Configuration, PublicationError> {
        self.configuration
            .as_ref()
            .ok_or(PublicationError::Unavailable)
    }
}
