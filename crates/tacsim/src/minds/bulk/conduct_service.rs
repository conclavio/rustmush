//! Actual GOD-dispatched private worker continuation. Unknown submissions keep
//! the same ticket and machine. Only native ACK plus complete typed/native
//! readback releases a lease or retires a continuation.
use super::{
    BulkError,
    artifacts::Artifacts,
    conduct_tickets::{Acknowledged, Binding, StoppedBinding},
    executor::{Completion, Machine, Ready},
    invalid,
    owner::{BulkStore, Lease},
    preparation,
    world::{self, Preparing, Runtime},
};
use crate::minds::registry::conduct::factory::{Factory, PreparedConduct};
use mush_core::{
    AccountId, ArgShape, CmdError, CommandCompatibility, CommandSpec, ContinuationHandle, Ctx,
    DefinitionRef, GOD, ModError, Registrar,
    action::{IdempotencyKeyText, PerceivedRef},
    read_service::OwnerReadHandle,
    submission_observation::{SubmissionDisposition, SubmissionObservation},
};
use mush_roll::{
    SourceAdapter,
    commerce::{
        FiledRevisionId,
        conduct::{
            ConductRole, ConductTrialProof, ConductWorkRead, ConductWorkReader, ConductWorkRequest,
            ConductWorkState,
            protocol::{TicketId, Transition},
        },
    },
};
use std::{collections::VecDeque, sync::Arc};

const COMMAND: &str = "@conduct";
#[derive(Clone)]
struct Configuration {
    factory: Arc<Factory>,
    reader: OwnerReadHandle<ConductWorkReader>,
    actions: [DefinitionRef; 5],
    continuation: ContinuationHandle,
}
pub(super) fn configured(w: &mush_core::World) -> bool {
    w.module_state::<Configuration>().is_some()
}
pub(crate) fn finalize(ctx: &mush_core::ModuleCtx<'_>, enabled: bool) -> Result<(), ModError> {
    let artifacts = ctx
        .w
        .module_state::<Artifacts>()
        .ok_or_else(|| ModError::msg("Conduct native artifact owner unavailable"))?;
    if enabled != configured(ctx.w) {
        return Err(ModError::msg("Conduct actual configuration differs"));
    }
    if enabled {
        artifacts.conduct_history.require()
    } else {
        artifacts.conduct_history.require_disabled()
    }
    .map_err(|_| ModError::msg("Conduct native history disagrees"))?;
    Ok(())
}
pub(in crate::minds) fn register(
    reg: &mut Registrar<'_>,
    factory: Arc<Factory>,
    manifest: &DefinitionRef,
    actions: [DefinitionRef; 5],
) -> Result<(), ModError> {
    let reader = ConductWorkReader::request(reg, manifest)?;
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(CommandSpec{name:COMMAND.into(),switches:vec!["run".into(),"cancel".into(),"retry".into(),"continue".into(),"status".into()],shape:ArgShape::Raw,
        help:"@conduct/run <filed revision id> <profile> <escort|transit>; @conduct/cancel; @conduct/retry; @conduct/status <operation>".into(),..CommandSpec::default()},Arc::new(command))?;
    let continuation = reg.command_continuation(COMMAND, &["continue"])?;
    reg.state(Configuration {
        factory,
        reader,
        actions,
        continuation,
    });
    Ok(())
}
enum Attempt {
    Unsubmitted,
    Observed(SubmissionObservation),
    NoIntent,
}
impl Attempt {
    fn poll(&mut self) {
        if let Self::Observed(observation) = self {
            match observation.take_terminal() {
                Some(SubmissionDisposition::NotDispatched) => *self = Self::Unsubmitted,
                Some(SubmissionDisposition::NoIntent) => *self = Self::NoIntent,
                None => {}
            }
        }
    }
}
struct Pending {
    ticket: TicketId,
    phase: Transition,
    ack: Option<Acknowledged>,
    attempt: Attempt,
}
impl Pending {
    fn new(ticket: TicketId, phase: Transition) -> Self {
        Self {
            ticket,
            phase,
            ack: None,
            attempt: Attempt::Unsubmitted,
        }
    }
}
enum Phase {
    Admission {
        machine: Option<Box<dyn Machine>>,
        start: [u8; 32],
    },
    Ready(Option<Ready>),
    Queued,
    Grant(Option<Ready>),
    Waiting {
        ready: Option<Ready>,
        lease: Option<Lease>,
    },
    Working,
    Returned(Option<Box<Completion>>),
    Settle(Option<Ready>),
    Cancel(Option<Ready>),
    Recover(Option<StoppedBinding>),
    Finished,
}
struct Entry {
    id: u64,
    account: AccountId,
    binding: Option<Binding>,
    pending: Option<Pending>,
    phase: Phase,
    cancel: bool,
    cancelled: bool,
    next_ms: i64,
}
#[derive(Default)]
pub(super) struct Lane {
    loaded: bool,
    entry: Option<Entry>,
    stopped: VecDeque<StoppedBinding>,
    pending_turn: bool,
    stranded: Vec<Ready>,
}
impl Lane {
    pub(super) fn pristine(&self) -> bool {
        !self.loaded
            && self.entry.is_none()
            && self.stopped.is_empty()
            && !self.pending_turn
            && self.stranded.is_empty()
    }
    pub(super) fn accounts(&self) -> impl Iterator<Item = AccountId> + '_ {
        self.entry
            .iter()
            .map(|e| e.account)
            .chain(self.stopped.iter().map(|s| s.binding().account))
            .chain(self.stranded.iter().map(Ready::account))
    }
    pub(super) fn recovering(&self) -> bool {
        !self.stopped.is_empty()
            || self
                .entry
                .as_ref()
                .is_some_and(|e| matches!(e.phase, Phase::Recover(_)))
    }
    pub(super) fn has_pending_turn(&self) -> bool {
        self.pending_turn
    }
    pub(super) fn capture(
        &mut self,
        cold: &world::ColdHydration,
        store: &BulkStore,
        artifacts: &Artifacts,
    ) -> Result<(), BulkError> {
        if !self.pristine() {
            return Err(invalid("Conduct stopped service is not fresh"));
        }
        artifacts.conduct_history.require()?;
        for (account, run) in store.conduct_runs()?.filter(|(_, r)| !r.status.terminal()) {
            if self.stopped.len() >= 136 {
                return Err(invalid("Conduct stopped inventory bound"));
            }
            self.stopped
                .push_back(StoppedBinding::capture(cold, account, run)?);
        }
        self.loaded = true;
        Ok(())
    }
}
fn config(ctx: &Ctx<'_>) -> Result<Configuration, BulkError> {
    ctx.w
        .module_state::<Configuration>()
        .cloned()
        .ok_or(BulkError::Missing)
}
fn live(ctx: &Ctx<'_>) -> Result<(), BulkError> {
    if ctx.actor != GOD
        || ctx.w.get(GOD).is_none_or(|o| {
            o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
        })
    {
        return Err(BulkError::Missing);
    }
    Ok(())
}
fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let result = (|| {
        live(ctx)?;
        if ctx.switches.has("run") {
            start(ctx)
        } else if ctx.switches.has("cancel") {
            control(ctx, false)
        } else if ctx.switches.has("retry") {
            control(ctx, true)
        } else if ctx.switches.has("continue") {
            continue_owned(ctx)
        } else if ctx.switches.has("status") {
            status(ctx)
        } else {
            Err(BulkError::Missing)
        }
    })();
    if ctx.switches.has("continue") {
        if result.is_err() {
            tracing::debug!("Conduct continuation retained");
        }
        Ok(())
    } else {
        result.map_err(|_| CmdError::msg("The owned Conduct trial is unavailable."))
    }
}
fn status(ctx: &mut Ctx<'_>) -> Result<(), BulkError> {
    let operation =
        mush_core::CommitToken::from_hex(ctx.args.raw().trim()).map_err(|_| BulkError::Missing)?;
    let cfg = config(ctx)?;
    let state = readback(ctx, &cfg, operation)?;
    let qualified = current_proof(ctx, operation).is_ok();
    ctx.out.info(
        GOD,
        &format!(
            "Conduct {}: {:?}; current qualification {}.",
            operation,
            state.execution().observation().phase,
            if qualified { "passed" } else { "unavailable" }
        ),
    );
    Ok(())
}
/// Reads the actual registered owner; callers cannot supply or decode a proof.
pub(in crate::minds) fn current_proof(
    ctx: &mut Ctx<'_>,
    operation: mush_core::CommitToken,
) -> Result<ConductTrialProof, BulkError> {
    live(ctx)?;
    let cfg = config(ctx)?;
    match ctx
        .read_owner(&cfg.reader, ConductWorkRequest::CurrentProof { operation })
        .map_err(|_| BulkError::Missing)?
    {
        ConductWorkRead::Proof { proof } => Ok(proof),
        _ => Err(BulkError::Missing),
    }
}
fn start(ctx: &mut Ctx<'_>) -> Result<(), BulkError> {
    let words = ctx.args.raw().split_whitespace().collect::<Vec<_>>();
    let [filed, profile, role] = words.as_slice() else {
        return Err(BulkError::Missing);
    };
    let filed =
        serde_json::from_value::<FiledRevisionId>(serde_json::Value::String((*filed).into()))
            .map_err(|_| BulkError::Missing)?;
    let profile = (*profile).to_owned();
    let role = match *role {
        "escort" => ConductRole::Escort,
        "transit" => ConductRole::Transit,
        _ => return Err(BulkError::Missing),
    };
    let cfg = config(ctx)?;
    let account = world::account(ctx.w, GOD)?;
    let service = world::service(ctx.w)?;
    let mut runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
    let store = ctx
        .w
        .module_state::<BulkStore>()
        .ok_or(BulkError::Missing)?;
    if !runtime.conduct.loaded || runtime.conduct.entry.is_some() {
        return Err(BulkError::Busy);
    }
    runtime.ensure_admission(store, account)?;
    let nonce = store.next_nonce(account)?;
    let ConductWorkRead::Candidate { candidate } = ctx
        .read_owner(
            &cfg.reader,
            ConductWorkRequest::Candidate {
                filed,
                profile,
                role,
            },
        )
        .map_err(|_| BulkError::Missing)?
    else {
        return Err(BulkError::Stale);
    };
    let source = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .ok_or(BulkError::Missing)?
        .read_source(candidate.revision().source())
        .map_err(|_| BulkError::Missing)?;
    let ticket = runtime.next;
    let next = ticket.checked_add(1).ok_or(BulkError::Busy)?;
    runtime
        .preparer
        .submit(preparation::Request::Conduct {
            ticket,
            account,
            nonce,
            candidate: Box::new(candidate),
            source: Arc::from(source),
            factory: cfg.factory,
        })
        .map_err(|(e, _)| e)?;
    runtime.next = next;
    runtime.preparing.insert(
        ticket,
        Preparing {
            account,
            actor: GOD,
            nonce,
        },
    );
    ctx.out.info(GOD, "Conduct trial preparation queued.");
    Ok(())
}
fn control(ctx: &mut Ctx<'_>, retry: bool) -> Result<(), BulkError> {
    if !ctx.args.raw().trim().is_empty() {
        return Err(BulkError::Missing);
    }
    let service = world::service(ctx.w)?;
    let mut runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
    let entry = runtime.conduct.entry.as_mut().ok_or(BulkError::Missing)?;
    if retry {
        let pending = entry.pending.as_mut().ok_or(BulkError::Missing)?;
        pending.attempt.poll();
        if !matches!(pending.attempt, Attempt::NoIntent) {
            return Err(BulkError::Busy);
        }
        pending.attempt = Attempt::Unsubmitted;
    } else {
        entry.cancel = true;
    }
    entry.next_ms = 0;
    Ok(())
}
pub(super) fn prepared(
    ctx: &mut Ctx<'_>,
    runtime: &mut Runtime,
    preparing: Preparing,
    result: Result<PreparedConduct, BulkError>,
) {
    let result = (|| {
        let PreparedConduct { data, machine } = result?;
        if preparing.actor != GOD
            || data.account != preparing.account
            || data.character != GOD
            || data.nonce != preparing.nonce
            || world::account(ctx.w, GOD)? != preparing.account
            || runtime.conduct.entry.is_some()
        {
            return Err(BulkError::Stale);
        }
        let start = data.plan.start.sha256;
        let ticket = ctx
            .w
            .module_state_mut::<Artifacts>()
            .ok_or(BulkError::Missing)?
            .conduct
            .admit(data)?;
        runtime.external_preparing.insert(preparing.account);
        runtime.conduct.entry = Some(Entry {
            id: ticket.get(),
            account: preparing.account,
            binding: None,
            pending: Some(Pending::new(ticket, Transition::Admit)),
            phase: Phase::Admission {
                machine: Some(machine),
                start,
            },
            cancel: false,
            cancelled: false,
            next_ms: 0,
        });
        Ok(())
    })();
    if result.is_err() {
        ctx.out.info(GOD, "Conduct trial preparation refused.");
    }
}
pub(super) fn acknowledged(ctx: &mut Ctx<'_>, runtime: &mut Runtime) {
    let Some(artifacts) = ctx.w.module_state_mut::<Artifacts>() else {
        return;
    };
    let Some(ack) = artifacts.conduct.peek_acknowledged() else {
        return;
    };
    let Some(pending) = runtime
        .conduct
        .entry
        .as_mut()
        .and_then(|e| e.pending.as_mut())
        .filter(|p| p.ticket == ack.ticket && p.phase == ack.transition && p.ack.is_none())
    else {
        tracing::error!("Conduct ACK has no retained continuation");
        return;
    };
    pending.ack = Some(ack);
    artifacts.conduct.take_acknowledged();
}
fn readback(
    ctx: &mut Ctx<'_>,
    cfg: &Configuration,
    job: mush_core::CommitToken,
) -> Result<ConductWorkState, BulkError> {
    match ctx
        .read_owner(&cfg.reader, ConductWorkRequest::Job { operation: job })
        .map_err(|_| BulkError::Stale)?
    {
        ConductWorkRead::Job { state } => Ok(state),
        _ => Err(BulkError::Stale),
    }
}
fn continue_owned(ctx: &mut Ctx<'_>) -> Result<(), BulkError> {
    let id = ctx
        .args
        .raw()
        .trim()
        .parse::<u64>()
        .map_err(|_| BulkError::Missing)?;
    let cfg = config(ctx)?;
    let service = world::service(ctx.w)?;
    let mut runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
    let Some(mut entry) = runtime.conduct.entry.take() else {
        return Err(BulkError::Missing);
    };
    let result = if entry.id == id {
        advance(ctx, &cfg, &mut runtime, &mut entry)
    } else {
        Err(BulkError::Missing)
    };
    if !matches!(entry.phase, Phase::Finished) {
        runtime.conduct.entry = Some(entry);
    }
    result
}
fn advance(
    ctx: &mut Ctx<'_>,
    cfg: &Configuration,
    runtime: &mut Runtime,
    entry: &mut Entry,
) -> Result<(), BulkError> {
    if let Some(ack) = entry.pending.as_ref().and_then(|p| p.ack) {
        let state = readback(ctx, cfg, ack.job)?;
        let execution = state.execution();
        let admission = state.job().admission();
        if execution.updated_by() != ack.operation
            || execution.job() != ack.job
            || admission.character() != GOD
        {
            return Err(BulkError::Stale);
        }
        let binding = Binding {
            job: ack.job,
            account: entry.account,
            character: GOD,
            run: super::model::RunId(admission.run()),
        };
        if entry.binding.is_some_and(|old| old != binding) {
            return Err(BulkError::Stale);
        }
        let actual = ctx
            .w
            .module_state::<BulkStore>()
            .ok_or(BulkError::Missing)?
            .account_run(entry.account, binding.run)?;
        super::conduct_projection::require_before(
            entry.account,
            binding.job,
            actual,
            admission,
            execution.observation(),
        )?;
        match (&mut entry.phase, ack.transition) {
            (Phase::Admission { machine, start }, Transition::Admit) => {
                if execution.observation().phase
                    != mush_roll::commerce::rating::execution::ExecutionPhase::Queued
                {
                    return Err(BulkError::Stale);
                }
                entry.phase = Phase::Ready(Some(Ready::conduct(
                    binding,
                    *start,
                    machine.take().ok_or(BulkError::Missing)?,
                )));
                runtime.external_preparing.remove(&entry.account);
            }
            (Phase::Grant(ready), Transition::Grant) => {
                if ready.is_none() || !runtime.conduct.pending_turn {
                    return Err(BulkError::Stale);
                }
                let lease = ctx
                    .w
                    .module_state_mut::<BulkStore>()
                    .ok_or(BulkError::Missing)?
                    .take_conduct_lease(ack.operation)
                    .ok_or(BulkError::Missing)?;
                entry.phase = Phase::Waiting {
                    ready: ready.take(),
                    lease: Some(lease),
                };
                runtime.conduct.pending_turn = false;
            }
            (Phase::Cancel(ready), Transition::Cancel) => {
                if !runtime.conduct.pending_turn || !execution.observation().phase.terminal() {
                    return Err(BulkError::Stale);
                }
                let _ = ready.take();
                runtime.conduct.pending_turn = false;
                entry.phase = Phase::Finished;
            }
            (Phase::Waiting { .. } | Phase::Working | Phase::Returned(_), Transition::Cancel) => {
                if execution.observation().phase
                    != mush_roll::commerce::rating::execution::ExecutionPhase::CancelRequested
                {
                    return Err(BulkError::Stale);
                }
                entry.cancelled = true;
            }
            (Phase::Settle(ready), Transition::Settle) => {
                if execution.observation().phase.terminal() {
                    entry.phase = Phase::Finished;
                } else {
                    entry.phase = Phase::Ready(ready.take());
                }
            }
            (Phase::Recover(_), Transition::Recover) => {
                if execution.observation().phase
                    != mush_roll::commerce::rating::execution::ExecutionPhase::Interrupted
                    || state.result().is_none()
                {
                    return Err(BulkError::Stale);
                }
                entry.phase = Phase::Finished;
            }
            _ => return Err(BulkError::Stale),
        }
        entry.binding = Some(binding);
        entry.pending = None;
        if matches!(entry.phase, Phase::Finished) {
            ctx.out.info(
                GOD,
                "Conduct trial reached its acknowledged terminal result.",
            );
            return Ok(());
        }
    }
    if entry.pending.is_none() {
        if entry.cancel
            && !entry.cancelled
            && matches!(
                entry.phase,
                Phase::Waiting { .. } | Phase::Working | Phase::Returned(_)
            )
        {
            let ticket = ctx
                .w
                .module_state_mut::<Artifacts>()
                .ok_or(BulkError::Missing)?
                .conduct
                .cancel(entry.binding.ok_or(BulkError::Missing)?)?;
            entry.pending = Some(Pending::new(ticket, Transition::Cancel));
        } else {
            match &mut entry.phase {
                Phase::Grant(ready) => {
                    let value = ready.as_ref().ok_or(BulkError::Missing)?;
                    let binding = entry.binding.ok_or(BulkError::Missing)?;
                    let tickets = &mut ctx
                        .w
                        .module_state_mut::<Artifacts>()
                        .ok_or(BulkError::Missing)?
                        .conduct;
                    let (ticket, phase) = if entry.cancel {
                        (tickets.cancel(binding)?, Transition::Cancel)
                    } else {
                        (
                            tickets.grant(binding, value.next_grant_ms())?,
                            Transition::Grant,
                        )
                    };
                    if entry.cancel {
                        entry.phase = Phase::Cancel(ready.take());
                    }
                    entry.pending = Some(Pending::new(ticket, phase));
                }
                Phase::Returned(done) => {
                    let done = done.as_mut().ok_or(BulkError::Missing)?;
                    let prepared = match done.conduct_settlement.take() {
                        Some(Ok(v)) => v,
                        Some(Err(e)) => {
                            done.conduct_settlement = Some(Err(e.clone()));
                            return Err(e);
                        }
                        None => return Err(BulkError::Missing),
                    };
                    let Some(artifacts) = ctx.w.module_state_mut::<Artifacts>() else {
                        done.conduct_settlement = Some(Ok(prepared));
                        return Err(BulkError::Missing);
                    };
                    let ticket = match artifacts.conduct.insert_settlement(prepared) {
                        Ok(t) => t,
                        Err((e, p)) => {
                            done.conduct_settlement = Some(Ok(p));
                            return Err(e);
                        }
                    };
                    entry.phase = Phase::Settle(done.ready.take());
                    entry.pending = Some(Pending::new(ticket, Transition::Settle));
                }
                Phase::Recover(stopped) => {
                    let binding = stopped.as_ref().ok_or(BulkError::Missing)?.binding();
                    let state = readback(ctx, cfg, binding.job)?;
                    if state.execution().observation().phase.terminal() {
                        return Err(BulkError::Stale);
                    }
                    let ticket = ctx
                        .w
                        .module_state_mut::<Artifacts>()
                        .ok_or(BulkError::Missing)?
                        .conduct
                        .recover(stopped.as_ref().ok_or(BulkError::Missing)?)?;
                    stopped.take();
                    entry.pending = Some(Pending::new(ticket, Transition::Recover));
                }
                _ => {}
            }
        }
    }
    if let Some(pending) = &mut entry.pending {
        pending.attempt.poll();
        if pending.ack.is_some()
            || !matches!(pending.attempt, Attempt::Unsubmitted)
            || ctx.authoritative_pending(GOD)
        {
            return Ok(());
        }
        let action = cfg
            .actions
            .iter()
            .find(|a| a.key().id.as_str() == pending.phase.action_id())
            .ok_or(BulkError::Missing)?
            .clone();
        let selected = ctx
            .w
            .module_state::<Artifacts>()
            .ok_or(BulkError::Missing)?
            .conduct
            .get(pending.ticket)?;
        let key = IdempotencyKeyText::parse_canonical(selected.key(pending.ticket))
            .map_err(|_| BulkError::Missing)?;
        let observation = ctx
            .submit_authoritative_observed(
                action,
                PerceivedRef::SelfActor,
                vec![],
                serde_json::json!({}),
                key,
            )
            .map_err(|_| BulkError::Stale)?;
        pending.attempt = Attempt::Observed(observation);
    }
    Ok(())
}
pub(super) fn completed(_ctx: &mut Ctx<'_>, runtime: &mut Runtime, done: Completion) {
    if let Some(entry) = runtime.conduct.entry.as_mut().filter(|e| {
        e.account == done.receipt.account()
            && e.binding.is_some_and(|b| b.run == done.receipt.run())
            && matches!(e.phase, Phase::Working)
    }) {
        entry.phase = Phase::Returned(Some(Box::new(done)));
        return;
    }
    tracing::error!("Conduct completion has no retained continuation");
    runtime
        .retry
        .push_back(super::world::PendingCompletion { done, blob: None });
}
pub(super) fn grant(_ctx: &mut Ctx<'_>, runtime: &mut Runtime, ready: Ready) {
    if !runtime.conduct.pending_turn
        && let Some(entry) = runtime.conduct.entry.as_mut().filter(|e| {
            e.account == ready.account()
                && e.binding.is_some_and(|b| b.run == ready.run())
                && matches!(e.phase, Phase::Queued)
        })
    {
        runtime.conduct.pending_turn = true;
        entry.phase = Phase::Grant(Some(ready));
        return;
    }
    if let Err((_, ready)) = runtime.queue.push(ready) {
        runtime.conduct.stranded.push(ready);
    }
}
pub(super) fn dispatch(ctx: &mut Ctx<'_>, runtime: &mut Runtime) {
    let Some(cfg) = ctx.w.module_state::<Configuration>().cloned() else {
        return;
    };
    if runtime.conduct.entry.is_none()
        && let Some(next) = runtime.next.checked_add(1)
        && let Some(stopped) = runtime.conduct.stopped.pop_front()
    {
        let binding = stopped.binding();
        let id = runtime.next;
        runtime.next = next;
        runtime.conduct.entry = Some(Entry {
            id,
            account: binding.account,
            binding: Some(binding),
            pending: None,
            phase: Phase::Recover(Some(stopped)),
            cancel: false,
            cancelled: false,
            next_ms: 0,
        });
    }
    let Some(entry) = runtime.conduct.entry.as_mut() else {
        return;
    };
    match &mut entry.phase {
        Phase::Ready(ready) => {
            if let Some(value) = ready.take() {
                match runtime.queue.push(value) {
                    Ok(()) => entry.phase = Phase::Queued,
                    Err((_, value)) => *ready = Some(value),
                }
            }
        }
        Phase::Waiting { ready, lease } => {
            if ready.is_some()
                && lease.is_some()
                && let (Some(value), Some(paid)) = (ready.take(), lease.take())
            {
                match runtime.executor.submit(paid, value) {
                    Ok(()) => entry.phase = Phase::Working,
                    Err((_, paid, value)) => {
                        *ready = Some(value);
                        *lease = Some(paid);
                    }
                }
            }
        }
        _ => {}
    }
    let now = world::now_ms();
    if (entry.pending.is_some()
        || matches!(
            entry.phase,
            Phase::Grant(_) | Phase::Returned(_) | Phase::Recover(_)
        )
        || entry.cancel
            && !entry.cancelled
            && matches!(entry.phase, Phase::Waiting { .. } | Phase::Working))
        && now >= entry.next_ms
        && !ctx.authoritative_pending(GOD)
    {
        entry.next_ms = now.saturating_add(1000);
        if ctx
            .schedule_continuation(&cfg.continuation, GOD, &entry.id.to_string(), 0)
            .is_err()
        {
            tracing::debug!("Conduct continuation scheduling retained");
        }
    }
}
#[cfg(test)]
pub(in crate::minds) fn fixture_entry(w: &mush_core::World) -> Option<u64> {
    let service = world::service(w).ok()?;
    let runtime = service.0.lock().ok()?;
    runtime.conduct.entry.as_ref().map(|e| e.id)
}
