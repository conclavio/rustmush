use super::{audit, families::Families, metadata::Publications, registration::Registration};
use mush_core::{ModError, ModuleBootData, ModuleCtx};
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn load(
    ctx: &mut ModuleCtx<'_>,
    data: &ModuleBootData,
    registration: Option<&Registration>,
) -> Result<(), ModError> {
    // Raw native attribution is checked before ordinary Tacsim recovery can
    // skip/heal unknown documents. Final typed inverse remains mandatory.
    let state = Publications::recover(
        data.kv.iter().map(|(k, v)| (k.to_string(), v.clone())),
        registration.is_some(),
    )?;
    if let Some(r) = registration {
        r.loaded(ctx)?;
    }
    *ctx.w
        .module_state_mut::<Publications>()
        .ok_or_else(|| bad("coverage metadata owner unavailable"))? = state;
    Ok(())
}
pub(crate) fn finalize(
    ctx: &mut ModuleCtx<'_>,
    families: &Families,
    registration: Option<&Registration>,
) -> Result<(), ModError> {
    audit::all(
        ctx,
        families,
        registration.map(Registration::writer),
        registration.map(Registration::service),
    )
    .map_err(bad)?;
    ctx.w
        .module_state_mut::<Publications>()
        .ok_or_else(|| bad("coverage metadata owner unavailable"))?
        .loaded = true;
    Ok(())
}
