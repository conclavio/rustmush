//! One bounded automatic lane using the real registered GOD continuation.
//! Timers schedule a command; all reads/submissions occur with Core's actual
//! actor stamp. Preparation errors never become measured rejection or recall.
use super::*;
use mush_roll::commerce::{
    FiledRevisionId,
    rating::{
        execution::RatingExecution,
        reconformance::reader::{
            ReconformanceReadRequest, ReconformanceReadResult, ReconformanceReader,
        },
        registration::ReconformanceDefinitions,
    },
};
use std::collections::VecDeque;
#[derive(Clone)]
pub(super) struct Configuration {
    pub(super) reader: OwnerReadHandle<ReconformanceReader>,
    pub(super) actions: [mush_core::DefinitionRef; 5],
    continuation: ContinuationHandle,
}
impl Configuration {
    pub(super) fn register(
        reg: &mut Registrar<'_>,
        manifest: &mush_core::DefinitionRef,
    ) -> Result<Self, ModError> {
        Ok(Self {
            reader: ReconformanceReader::request(reg, manifest)?,
            actions: ReconformanceDefinitions::new(manifest.clone())?
                .actions()
                .clone(),
            continuation: reg.command_continuation(COMMAND, &["reconform"])?,
        })
    }
}
#[derive(Default)]
pub(super) struct Queue {
    after: Option<String>,
    filed: VecDeque<FiledRevisionId>,
    pub(super) preparing: bool,
    pub(super) next_ms: i64,
}
pub(super) fn schedule(ctx: &mut Ctx<'_>, runtime: &mut Runtime) {
    let Some(config) = ctx
        .w
        .module_state::<super::Configuration>()
        .and_then(|c| c.reconformance.clone())
    else {
        return;
    };
    let now = world::now_ms();
    if runtime.registry.reconformance.preparing
        || runtime.registry.entries.contains_key(&mush_core::GOD)
        || ctx.authoritative_pending(mush_core::GOD)
        || now < runtime.registry.reconformance.next_ms
    {
        return;
    }
    runtime.registry.reconformance.next_ms = now.saturating_add(RETRY_MS);
    // Queue refusal delays discovery. It cannot fabricate an observation.
    if let Err(error) = ctx.schedule_continuation(&config.continuation, mush_core::GOD, "", 0) {
        tracing::debug!(%error,"Registry reconformance scheduling retained");
    }
}
pub(super) fn continue_scan(ctx: &mut Ctx<'_>) -> Result<(), BulkError> {
    if ctx.actor != mush_core::GOD || !ctx.args.raw().trim().is_empty() {
        return Err(BulkError::Missing);
    }
    let config = ctx
        .w
        .module_state::<super::Configuration>()
        .cloned()
        .ok_or(BulkError::Missing)?;
    let service_config = config.reconformance.as_ref().ok_or(BulkError::Missing)?;
    let service = world::service(ctx.w)?;
    let mut runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
    if runtime.registry.reconformance.preparing
        || runtime.registry.entries.contains_key(&mush_core::GOD)
        || ctx.authoritative_pending(mush_core::GOD)
    {
        return Ok(());
    }
    if runtime.registry.reconformance.filed.is_empty() {
        let after = runtime.registry.reconformance.after.clone();
        let result = ctx
            .read_owner(
                &service_config.reader,
                ReconformanceReadRequest::Scan { after },
            )
            .map_err(|_| BulkError::Stale)?;
        let ReconformanceReadResult::Page { filed, next } = result else {
            return Err(BulkError::Stale);
        };
        if filed.len() > mush_roll::commerce::rating::reconformance::reader::SCAN_ROWS {
            return Err(BulkError::Stale);
        }
        runtime.registry.reconformance.after = next;
        runtime.registry.reconformance.filed = filed.into();
        if runtime.registry.reconformance.filed.is_empty() {
            runtime.registry.reconformance.next_ms = world::now_ms().saturating_add(60_000);
            return Ok(());
        }
    }
    // One exact candidate proof per dispatch, including ineligible rows. This
    // bounds examined work and eventually revisits temporary refusals.
    let Some(filed) = runtime.registry.reconformance.filed.pop_front() else {
        return Ok(());
    };
    let Ok(ReconformanceReadResult::Candidate { candidate }) = ctx.read_owner(
        &service_config.reader,
        ReconformanceReadRequest::Candidate { filed },
    ) else {
        return Ok(());
    };
    let account = world::account(ctx.w, mush_core::GOD)?;
    let store = ctx
        .w
        .module_state::<BulkStore>()
        .ok_or(BulkError::Missing)?;
    runtime.ensure_admission(store, account)?;
    let nonce = store.next_nonce(account)?;
    let source = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .ok_or(BulkError::Missing)?
        .read_source(candidate.revision().source())
        .map_err(|_| BulkError::Missing)?;
    let ticket = runtime.next;
    let next = ticket
        .checked_add(1)
        .ok_or_else(|| invalid("Registry preparation sequence"))?;
    runtime
        .preparer
        .submit(preparation::Request::Reconformance {
            ticket,
            account,
            nonce,
            candidate,
            source: Arc::from(source),
            factory: config.factory,
        })
        .map_err(|(e, _)| e)?;
    runtime.next = next;
    runtime.preparing.insert(
        ticket,
        Preparing {
            account,
            actor: mush_core::GOD,
            nonce,
        },
    );
    runtime.registry.reconformance.preparing = true;
    Ok(())
}
pub(super) fn execution(
    ctx: &Ctx<'_>,
    config: &super::Configuration,
    job: mush_core::CommitToken,
    reconformance: bool,
) -> Result<RatingExecution, BulkError> {
    let execution = if reconformance {
        let service = config.reconformance.as_ref().ok_or(BulkError::Missing)?;
        let result = ctx
            .read_owner(&service.reader, ReconformanceReadRequest::Job { job })
            .map_err(|_| BulkError::Stale)?;
        let ReconformanceReadResult::Job { execution, .. } = result else {
            return Err(BulkError::Stale);
        };
        execution
    } else {
        ctx.read_owner(&config.reader, RegistryReadRequest::new(job))
            .map_err(|_| BulkError::Stale)?
            .execution()
            .clone()
    };
    if execution.admission().reconformance.is_some() != reconformance {
        return Err(BulkError::Stale);
    }
    Ok(execution)
}
