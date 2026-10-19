//! Only an actual current measured rejection can produce this permanent cause.
//! The separate service intent is not a substitute for manual Org authority.
pub(crate) mod history;
pub(crate) mod protocol;
mod provider;
pub(crate) mod publication;
pub(crate) mod records;
mod service;
use crate::commerce::{
    families::CommerceFamilies,
    rating::RatingFamilies,
    records::{PublicationError, Record},
};
use mush_core::mechanics::{DefinitionRegistry, OwnerId};
use mush_core::{ModError, ModuleCtx, Registrar, World};
use std::sync::{Arc, OnceLock};
fn bad<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
#[derive(Clone, Default)]
pub(crate) struct Readiness(Arc<OnceLock<DefinitionRegistry>>);
impl Readiness {
    pub(crate) fn capture(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        self.0
            .set(
                ctx.reg
                    .definitions()
                    .ok_or_else(|| error("Registry definitions"))?
                    .clone(),
            )
            .map_err(|_| error("Registry recall already captured"))
    }
    fn check(&self, w: &World) -> Result<(), PublicationError> {
        let defs = self.0.get().ok_or(PublicationError::Unavailable)?;
        for owner in [crate::NS, "tacsim"] {
            defs.require_owner_ready(&OwnerId::new(owner).map_err(bad)?)
                .map_err(|_| PublicationError::Unavailable)?;
        }
        if w.module_state_owner(std::any::TypeId::of::<crate::RollStore>())
            .is_none_or(|o| o.as_str() != crate::NS)
        {
            return Err(PublicationError::Unavailable);
        }
        w.module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(|_| PublicationError::Unavailable)
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    f: CommerceFamilies,
    ratings: RatingFamilies,
    expected: super::super::RatingCompatibility,
    manifest: mush_core::DefinitionRef,
    definitions: [mush_core::DefinitionRef; 2],
    reader: mush_core::read_service::OwnerReadHandle<super::reader::ReconformanceReader>,
) -> Result<Readiness, ModError> {
    let ready = Readiness::default();
    let runtime = service::Runtime::default();
    provider::register(
        reg,
        f.clone(),
        definitions.clone(),
        manifest,
        reader,
        ready.clone(),
        runtime.clone(),
    )?;
    let [action, _] = definitions;
    service::register(reg, runtime, ready.clone(), f, ratings, expected, action)?;
    Ok(ready)
}
