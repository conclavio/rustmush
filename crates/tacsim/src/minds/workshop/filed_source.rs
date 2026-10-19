//! Synchronous connection-bound paid source reads. Each request resolves the
//! current owner service; source observations/bytes are never queued as jobs.
use mush_core::read_service::OwnerReadHandle;
use mush_core::{Ctx, DefinitionRef, ModError, Registrar};
use mush_roll::RollError;
use mush_roll::commerce::LicenceId;
use mush_roll::commerce::paid_source_reader::{PaidSourceReader, PaidSourceRequest};

#[derive(Clone)]
enum SourceAccessSeam {
    Original(OwnerReadHandle<PaidSourceReader>),
    Entitlement(
        OwnerReadHandle<mush_roll::commerce::entitlement::customer::EntitlementSourceReader>,
    ),
}
pub(crate) fn register(reg: &mut Registrar<'_>, manifest: &DefinitionRef) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("paid source consumer belongs to Tacsim"));
    }
    let reader = PaidSourceReader::request(reg, manifest)?;
    reg.state(SourceAccessSeam::Original(reader));
    Ok(())
}
pub(crate) fn register_entitlements(
    reg: &mut Registrar<'_>,
    manifest: &DefinitionRef,
) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("paid source consumer belongs to Tacsim"));
    }
    let reader = mush_roll::commerce::entitlement::customer::EntitlementSourceReader::request(
        reg, manifest,
    )?;
    reg.state(SourceAccessSeam::Entitlement(reader));
    Ok(())
}
#[derive(Clone)]
struct DependentSourceSeam {
    reader: OwnerReadHandle<mush_roll::commerce::dependent_source::DependentSourceReader>,
}
pub(crate) fn register_dependent(
    reg: &mut Registrar<'_>,
    manifest: &DefinitionRef,
) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("dependent source belongs to Tacsim"));
    }
    let reader =
        mush_roll::commerce::dependent_source::DependentSourceReader::request(reg, manifest)?;
    reg.state(DependentSourceSeam { reader });
    Ok(())
}
pub(super) struct SourceLines {
    pub licence: String,
    pub source_sha256: String,
    pub first: u32,
    pub lines: Vec<String>,
}
fn licence(text: &str) -> Result<LicenceId, RollError> {
    let raw = text.parse::<u64>().map_err(|_| RollError::Missing)?;
    if raw.to_string() != text {
        return Err(RollError::Missing);
    }
    LicenceId::new(raw).map_err(|_| RollError::Missing)
}
pub(super) fn read(
    ctx: &Ctx<'_>,
    selector: &str,
    first: u32,
    last: u32,
) -> Result<SourceLines, RollError> {
    let licence = licence(selector)?;
    if first == 0 || last < first || last - first >= 200 {
        return Err(RollError::Missing);
    }
    if let Some(seam) = ctx.w.module_state::<DependentSourceSeam>() {
        let result = ctx
            .hook_read_context(crate::NS)
            .read_owner(
                &seam.reader,
                mush_roll::commerce::dependent_source::DependentSourceRequest::own(licence),
            )
            .map_err(|_| RollError::Missing)?;
        if let Some(access) = result.into_owner() {
            if access.actor() != ctx.actor || access.licence() != licence {
                return Err(RollError::Missing);
            }
            let sources = ctx
                .w
                .module_state::<crate::TacsimStore>()
                .ok_or(RollError::Missing)?;
            let source =
                crate::match_::filed_holdings::decode_dependent_source_access(&access, sources)
                    .map_err(|_| RollError::Missing)?;
            let lines = super::source_lines(
                &source,
                usize::try_from(first).map_err(|_| RollError::Missing)?,
                usize::try_from(last).map_err(|_| RollError::Missing)?,
            )?;
            return Ok(SourceLines {
                licence: licence.get().to_string(),
                source_sha256: access.identity().source().to_hex(),
                first,
                lines,
            });
        }
        // Only an actual paid-namespace result can reach the unchanged paid
        // reader. A dependent-owner failure is returned above without fallback.
    }
    let seam = ctx
        .w
        .module_state::<SourceAccessSeam>()
        .ok_or(RollError::Missing)?;
    // The captured dispatch actor is authoritative. The request contains no
    // alternate holder, caller, form, account or source selector.
    let context = ctx.hook_read_context(crate::NS);
    let access = match seam {
        SourceAccessSeam::Original(reader) => {
            context.read_owner(reader, PaidSourceRequest::own(licence))
        }
        SourceAccessSeam::Entitlement(reader) => context.read_owner(
            reader,
            mush_roll::commerce::entitlement::customer::EntitlementSourceRequest::own(licence),
        ),
    }
    .map_err(|_| RollError::Missing)?;
    if access.actor() != ctx.actor || access.licence() != licence {
        return Err(RollError::Missing);
    }
    let sources = ctx
        .w
        .module_state::<crate::TacsimStore>()
        .ok_or(RollError::Missing)?;
    let source = crate::match_::filed_holdings::decode_source_access(&access, sources)
        .map_err(|_| RollError::Missing)?;
    let lines = super::source_lines(
        &source,
        usize::try_from(first).map_err(|_| RollError::Missing)?,
        usize::try_from(last).map_err(|_| RollError::Missing)?,
    )?;
    Ok(SourceLines {
        licence: licence.get().to_string(),
        source_sha256: access.identity().source().to_hex(),
        first,
        lines,
    })
}

/// Line/SSH output is only to the actual dispatching physical connection. It
/// never enters the actor/session resume feed or an asynchronous author job.
pub(super) fn line(
    ctx: &mut Ctx<'_>,
    selector: &str,
    first: usize,
    last: usize,
) -> Result<(), RollError> {
    let conn = ctx.w.dispatching_conn().ok_or(RollError::Missing)?;
    let (actor, lease) = ctx
        .w
        .connection_binding(conn)
        .filter(|(actor, _)| *actor == ctx.actor)
        .ok_or(RollError::Missing)?;
    let value = read(
        ctx,
        selector,
        u32::try_from(first).map_err(|_| RollError::Missing)?,
        u32::try_from(last).map_err(|_| RollError::Missing)?,
    )?;
    if ctx.w.connection_binding(conn) != Some((actor, lease)) {
        return Err(RollError::Missing);
    }
    ctx.out.to_conn(
        conn,
        mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(format!(
            "Filed source filed:{}; SHA256 {}.",
            value.licence, value.source_sha256
        ))),
    );
    for line in value.lines {
        ctx.out.to_conn(
            conn,
            mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(line)),
        );
    }
    Ok(())
}
