//! Actor-dispatched Registry continuations on the one shared bulk service.
//! Neither a tick nor a worker replaces the dispatch actor. The ordinary
//! registered continuation re-enters a ticket-only command; core supplies its read stamp and
//! the existing signed-offer submission path owns the complete transaction.

use std::{collections::BTreeMap, sync::Arc};

use mush_core::action::{IdempotencyKeyText, PerceivedRef};
use mush_core::read_service::OwnerReadHandle;
use mush_core::submission_observation::{SubmissionDisposition, SubmissionObservation};
use mush_core::{
    AccountId, ArgShape, CmdError, CommandCompatibility, CommandSpec, ContinuationHandle, Ctx,
    ModError, ObjId, Registrar,
};
use mush_roll::commerce::rating::{
    protocol::{TicketId, Transition},
    registration::RegistryDefinitions,
    service_read::{RegistryReadRequest, RegistryReader},
};
use mush_roll::{RevisionKey, SourceAdapter};

use super::{
    BulkError,
    artifacts::Artifacts,
    executor::{Completion, Machine, Ready},
    invalid,
    model::RunId,
    owner::{BulkStore, Lease},
    preparation,
    registry_tickets::{Acknowledged, Binding},
    world::{self, Preparing, Runtime},
};
use crate::minds::registry::factory::{Factory, PreparedRegistry};

#[path = "registry_service/reconformance.rs"]
mod reconformance;
const COMMAND: &str = "@registry";
const RETRY_MS: i64 = 1_000;
#[cfg(test)]
#[path = "registry_service_tests.rs"]
mod tests;
#[derive(Clone)]
struct Configuration {
    factory: Arc<Factory>,
    reader: OwnerReadHandle<RegistryReader>,
    actions: [mush_core::DefinitionRef; 5],
    continuation: ContinuationHandle,
    reconformance: Option<reconformance::Configuration>,
}
pub(super) fn configured(world: &mush_core::World) -> bool {
    world.module_state::<Configuration>().is_some()
}
pub(in crate::minds) fn register_profiles(
    reg: &mut Registrar<'_>,
    factory: Arc<Factory>,
    manifest: &mush_core::DefinitionRef,
    reconformance: bool,
) -> Result<(), ModError> {
    let definitions = RegistryDefinitions::new(manifest.clone())?;
    let reader = RegistryReader::request(reg, manifest)?;
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: COMMAND.into(),
            // 17g C2d: `recall` and `revoke`, the Registry operators'
            // commands (`market_commands::registry`), in both lists.
            switches: if reconformance {
                vec![
                    "rate".into(),
                    "cancel".into(),
                    "continue".into(),
                    "reconform".into(),
                    "recall".into(),
                    "revoke".into(),
                ]
            } else {
                vec![
                    "rate".into(),
                    "cancel".into(),
                    "continue".into(),
                    "recall".into(),
                    "revoke".into(),
                ]
            },
            shape: ArgShape::Raw,
            help: HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(command),
    )?;
    let continuation = reg.command_continuation(COMMAND, &["continue"])?;
    let reconformance = if reconformance {
        Some(reconformance::Configuration::register(reg, manifest)?)
    } else {
        None
    };
    reg.state(Configuration {
        factory,
        reader,
        actions: definitions.actions().clone(),
        continuation,
        reconformance,
    });
    Ok(())
}

#[derive(Clone)]
struct MarkedConfiguration(OwnerReadHandle<mush_roll::commerce::marked_rating::MarkedRatingReader>);
/// Only the explicit Active authored-fork composition requests this exact reader.
/// The separately configured Registry service and Roll producer still validate
/// their genuine owners/manifest at freeze and admission.
pub(crate) fn register_marked(
    reg: &mut Registrar<'_>,
    manifest: &mush_core::DefinitionRef,
) -> Result<(), ModError> {
    let reader = mush_roll::commerce::marked_rating::MarkedRatingReader::request(reg, manifest)?;
    reg.state(MarkedConfiguration(reader));
    Ok(())
}

struct Pending {
    ticket: TicketId,
    transition: Transition,
    ack: Option<Acknowledged>,
    attempt: Attempt,
}
enum Attempt {
    Unsubmitted,
    Observed(SubmissionObservation),
    NoIntent,
}
impl Attempt {
    fn poll(&mut self) {
        let Self::Observed(observation) = self else {
            return;
        };
        match observation.take_terminal() {
            Some(SubmissionDisposition::NotDispatched) => *self = Self::Unsubmitted,
            Some(SubmissionDisposition::NoIntent) => *self = Self::NoIntent,
            None => {}
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
    Recover(Option<super::registry_recovery::StoppedBinding>),
    Unrecoverable {
        _stopped: super::registry_recovery::StoppedBinding,
        _reason: super::registry_recovery::Unavailable,
    },
    Finished,
}
struct Entry {
    reconformance: bool,
    id: u64,
    account: AccountId,
    actor: ObjId,
    binding: Option<Binding>,
    pending: Option<Pending>,
    phase: Phase,
    cancel: bool,
    schedule_after: i64,
}
#[derive(Default)]
pub(super) struct Lane {
    reconformance: reconformance::Queue,
    entries: BTreeMap<ObjId, Entry>,
    // The shared FairQueue has lent this turn to one signed publication. Keep
    // every later account in that queue until this grant/cancel is acknowledged.
    pending_turn: Option<ObjId>,
    // Only a broken in-process phase join can reach this lane. Each retained
    // value came out of the same bounded queue and still owns its admitted job.
    // Never discard a Machine merely because that invariant failed.
    stranded_ready: Vec<Ready>,
}
impl Lane {
    pub(super) fn accounts(&self) -> impl Iterator<Item = AccountId> + '_ {
        self.entries
            .values()
            .map(|entry| entry.account)
            .chain(self.stranded_ready.iter().map(Ready::account))
    }
    pub(super) fn has_pending_turn(&self) -> bool {
        self.pending_turn.is_some()
    }
    // A refusal hands the caller's own `Ready` back, unboxed, for its retry.
    #[allow(clippy::result_large_err)]
    fn take_fair_turn(&mut self, ready: Ready) -> Result<(), Ready> {
        if self.pending_turn.is_some() {
            return Err(ready);
        }
        let Some(entry) = self.entries.values_mut().find(|e| {
            e.account == ready.account() && e.binding.is_some_and(|b| b.run == ready.run())
        }) else {
            return Err(ready);
        };
        if !matches!(entry.phase, Phase::Queued) {
            return Err(ready);
        }
        self.pending_turn = Some(entry.actor);
        entry.phase = Phase::Grant(Some(ready));
        entry.schedule_after = 0;
        Ok(())
    }
    fn accept_ack(&mut self, ack: Acknowledged) -> bool {
        let Some((pending, schedule_after)) = self.entries.values_mut().find_map(|entry| {
            entry
                .pending
                .as_mut()
                .filter(|p| {
                    p.ticket == ack.ticket && p.transition == ack.transition && p.ack.is_none()
                })
                .map(|pending| (pending, &mut entry.schedule_after))
        }) else {
            return false;
        };
        pending.ack = Some(ack);
        *schedule_after = 0;
        true
    }
}
impl Entry {
    // A newly lent fair turn, first matching ACK or retained worker completion
    // makes progress eligible immediately. Polling the same unresolved state
    // still uses this bounded retry deadline and never replaces its observation.
    fn continuation(&mut self, now: i64, authoritative_pending: bool) -> Option<(ObjId, String)> {
        let needs_actor = self.pending.is_some()
            || matches!(
                self.phase,
                Phase::Grant(_) | Phase::Returned(_) | Phase::Recover(_)
            );
        if !needs_actor || now < self.schedule_after || authoritative_pending {
            return None;
        }
        self.schedule_after = now.saturating_add(RETRY_MS);
        Some((self.actor, self.id.to_string()))
    }
}

fn consume_attempt(
    entry: &mut Entry,
    tickets: &mut super::registry_tickets::Tickets,
    preparing: &mut std::collections::BTreeSet<AccountId>,
) -> Result<bool, BulkError> {
    let Some(pending) = &mut entry.pending else {
        return Ok(false);
    };
    // A genuine owner acknowledgement must still pass the normal typed and
    // private-quota readback. It cannot be discarded by a transport outcome.
    if pending.ack.is_some() {
        return Ok(false);
    }
    pending.attempt.poll();
    if !matches!(pending.attempt, Attempt::NoIntent) {
        return Ok(false);
    }
    if matches!(entry.phase, Phase::Admission { .. }) {
        if pending.transition != Transition::Admit || entry.binding.is_some() {
            return Err(BulkError::Stale);
        }
        // Only a never-admitted job can release its Machine. The ticket is
        // unchanged since this exact attempt; the server proved this bundle
        // absent and aborted any local image, not key-wide absence. There is
        // exactly one observed attempt and no forgotten unresolved predecessor.
        // Failure retains both proof and custody.
        tickets.discard_unadmitted(pending.ticket, entry.actor, entry.account)?;
        preparing.remove(&entry.account);
        entry.pending = None;
        entry.phase = Phase::Finished;
        return Ok(true);
    }
    if !matches!(
        (&entry.phase, pending.transition),
        (Phase::Grant(_), Transition::Grant)
            | (Phase::Settle(_), Transition::Settle)
            | (Phase::Cancel(_), Transition::Cancel)
            | (Phase::Recover(_), Transition::Recover)
    ) || entry.binding.is_none()
    {
        return Err(BulkError::Stale);
    }
    let ticket = tickets.get(pending.ticket)?;
    if ticket.transition() != pending.transition
        || ticket.binding() != entry.binding
        || ticket.character() != entry.actor
        || ticket.account() != entry.account
    {
        return Err(BulkError::Stale);
    }
    // Admitted work retains its paid lease/result and fair head. Only this
    // exact ticket may retry; no receipt or Machine is dropped or rebuilt.
    pending.attempt = Attempt::Unsubmitted;
    Ok(false)
}

/// Whether the Registry service still holds a job for `actor`. A rating's
/// receipt can be read a pulse before its finished job is released, and
/// `@registry/rate` refuses a second job for the same character until then.
#[cfg(test)]
pub(crate) fn holds(w: &mush_core::World, actor: ObjId) -> bool {
    super::world::service(w)
        .ok()
        .and_then(|service| {
            service
                .0
                .lock()
                .ok()
                .map(|runtime| runtime.registry.entries.contains_key(&actor))
        })
        .unwrap_or(false)
}

/// 17g C2a (reviewer F2): whether the Registry already holds a rating job for
/// `actor` itself, still preparing (a Registry `Preparing` row queued by this
/// character) or retained in its lane. A proving or Conduct preparation, or a
/// job of another character on the same login, is not this character's
/// rating, though admission refuses it alike.
pub(super) fn holds_rating(runtime: &world::Runtime, actor: ObjId) -> bool {
    runtime.registry.entries.contains_key(&actor)
        || runtime
            .preparing
            .values()
            .any(|p| p.kind == super::model::WorkKind::Registry && p.actor == actor)
}
/// 17g C2a: `holds_rating` on the live service, for `@mind/rate`, which
/// answers `RATING_BUSY` only for this case and the service's unavailable
/// line for every other `BulkError::Busy`. An absent or poisoned service
/// holds nothing.
pub(in crate::minds) fn rating_held(w: &mush_core::World, actor: ObjId) -> bool {
    super::world::service(w)
        .ok()
        .and_then(|service| {
            service
                .0
                .lock()
                .ok()
                .map(|runtime| holds_rating(&runtime, actor))
        })
        .unwrap_or(false)
}

/// `@registry`'s help (17g C2d adds the operators' recall and revocation).
/// The brief's one sentence is 136 columns, so it breaks where its second
/// clause begins, keeping both lines inside the 120-column floor.
pub(crate) const HELP: &str = "@registry/rate <owned mind number>/<saved revision>; @registry/cancel\n@registry/recall <filed> and @registry/revoke <licence> (Registry operators)";

fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // 17g C2d: the operators' commands answer for themselves (the driver's
    // line, `REGISTRY_OFF` or the help), never through this service's own
    // "unavailable" mapping below.
    if ctx.switches.has("recall") {
        return crate::minds::market_commands::registry::registry(
            ctx,
            crate::minds::market_commands::registry::RegistryCommand::Recall,
            HELP,
        );
    }
    if ctx.switches.has("revoke") {
        return crate::minds::market_commands::registry::registry(
            ctx,
            crate::minds::market_commands::registry::RegistryCommand::Revoke,
            HELP,
        );
    }
    let result = if ctx.switches.has("reconform") {
        reconformance::continue_scan(ctx)
    } else if ctx.switches.has("rate") {
        start(ctx)
    } else if ctx.switches.has("cancel") {
        cancel(ctx)
    } else if ctx.switches.has("continue") {
        continue_owned(ctx)
    } else {
        Err(BulkError::Missing)
    };
    // Internal continuation retries do not publish failure/success guesses or
    // expose private ticket/phase details to a player. A missing acknowledgement
    // is not evidence that the writer refused, including after a timeout.
    if ctx.switches.has("continue") || ctx.switches.has("reconform") {
        if let Err(error) = result {
            tracing::debug!(%error, "Registry continuation retained");
        }
        Ok(())
    } else {
        result.map_err(|_| CmdError::msg("The owned Registry computation is unavailable."))
    }
}
fn start(ctx: &mut Ctx<'_>) -> Result<(), BulkError> {
    let (mind, number) = ctx
        .args
        .raw()
        .trim()
        .split_once('/')
        .ok_or(BulkError::Missing)?;
    let mind = mind
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or(BulkError::Missing)?;
    let number = number
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or(BulkError::Missing)?;
    start_revision(ctx, mind, number)
}
/// 17g C2a (C2 plan §2.2): queue a Registry rating of the actor's own saved
/// revision `number` of library mind `mind`. `@registry/rate` parses its
/// `<mind>/<revision>` argument and calls this; `@mind/rate` resolves the
/// author's `<name>@<rev>` and calls the same body. Success prints
/// `Registry preparation queued.`. `BulkError::Busy` is not only "this
/// character already has a Registry job" (`holds_rating`): admission also
/// answers Busy while bulk recovery runs, while the login account has any
/// other bulk run, preparation or retained job (an `@mind/prove` run, the
/// alt's rating), on a poisoned service lock and on a full preparer queue.
/// `@mind/rate` tells the first case apart with `rating_held`.
pub(in crate::minds) fn start_revision(
    ctx: &mut Ctx<'_>,
    mind: u64,
    number: u64,
) -> Result<(), BulkError> {
    let config = ctx
        .w
        .module_state::<Configuration>()
        .cloned()
        .ok_or(BulkError::Missing)?;
    let account = world::account(ctx.w, ctx.actor)?;
    let service = world::service(ctx.w)?;
    let mut runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
    let store = ctx
        .w
        .module_state::<BulkStore>()
        .ok_or(BulkError::Missing)?;
    if holds_rating(&runtime, ctx.actor) {
        return Err(BulkError::Busy);
    }
    runtime.ensure_admission(store, account)?;
    let revision = ctx
        .w
        .module_state::<mush_roll::RollStore>()
        .ok_or(BulkError::Missing)?
        .revision(
            ctx.actor,
            RevisionKey {
                character: ctx.actor,
                mind,
                number,
            },
        )
        .map_err(|_| BulkError::Missing)?
        .clone();
    if revision.require_original_access().is_err() {
        let marked = ctx
            .w
            .module_state::<MarkedConfiguration>()
            .ok_or(BulkError::Missing)?;
        let access = ctx
            .read_owner(
                &marked.0,
                mush_roll::commerce::marked_rating::MarkedRatingRequest::new(revision.reference()),
            )
            .map_err(|_| BulkError::Missing)?;
        if access.actor() != ctx.actor || access.revision() != &revision {
            return Err(BulkError::Missing);
        }
    }
    let source = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .ok_or(BulkError::Missing)?
        .read_source(revision.source())
        .map_err(|_| BulkError::Missing)?;
    let nonce = store.next_nonce(account)?;
    let ticket = runtime.next;
    let next = ticket
        .checked_add(1)
        .ok_or_else(|| invalid("Registry preparation sequence"))?;
    runtime
        .preparer
        .submit(preparation::Request::Registry {
            ticket,
            account,
            actor: ctx.actor,
            nonce,
            revision: Box::new(revision),
            source: Arc::from(source),
            factory: config.factory,
        })
        .map_err(|(error, _)| error)?;
    runtime.next = next;
    runtime.preparing.insert(
        ticket,
        Preparing {
            account,
            actor: ctx.actor,
            nonce,
            kind: super::model::WorkKind::Registry,
        },
    );
    ctx.out.info(ctx.actor, "Registry preparation queued.");
    Ok(())
}
/// 17g C2a (C2 plan §2.2): the actor's own finished Registry rating of exactly
/// `revision`, for `@mind/file` and `@mind/update`. Among the actor's bulk
/// account's retained Registry runs for this character that finished with an
/// admitted job (newest first), each job is read through the Registry's own
/// own-job read; the first whose subject is this revision and whose state is
/// `Completed` gives its rating reference. No rating index is added: a run
/// the account has since trimmed means "rate it again" (`Ok(None)`).
pub(in crate::minds) fn completed_rating(
    ctx: &Ctx<'_>,
    revision: &mush_roll::Revision,
) -> Result<Option<mush_roll::commerce::rating::RatingRef>, BulkError> {
    let config = ctx
        .w
        .module_state::<Configuration>()
        .ok_or(BulkError::Missing)?;
    let identity =
        mush_roll::commerce::RevisionIdentity::capture(revision).map_err(|_| BulkError::Missing)?;
    let account = world::account(ctx.w, ctx.actor)?;
    let store = ctx
        .w
        .module_state::<BulkStore>()
        .ok_or(BulkError::Missing)?;
    // An account with no bulk record has rated nothing yet.
    let Ok(runs) = store.runs(account, ctx.actor) else {
        return Ok(None);
    };
    let mut jobs: Vec<_> = runs
        .into_iter()
        .filter(|run| run.plan.kind == super::model::WorkKind::Registry)
        .filter_map(|run| Some((run.finished_ms?, run.id, run.registry_job?)))
        .collect();
    jobs.sort_by_key(|(finished, run, _)| std::cmp::Reverse((*finished, *run)));
    for (_, _, job) in jobs {
        let Ok(read) = ctx.read_owner(&config.reader, RegistryReadRequest::new(job)) else {
            continue;
        };
        if read.job().subject().revision() != identity {
            continue;
        }
        if let mush_roll::commerce::rating::RatingJobState::Completed { receipt } =
            read.job().state()
        {
            return Ok(Some(*receipt));
        }
    }
    Ok(None)
}
fn cancel(ctx: &mut Ctx<'_>) -> Result<(), BulkError> {
    if !ctx.args.raw().trim().is_empty() {
        return Err(BulkError::Missing);
    }
    let service = world::service(ctx.w)?;
    let mut runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
    let entry = runtime
        .registry
        .entries
        .get_mut(&ctx.actor)
        .ok_or(BulkError::Missing)?;
    if world::account(ctx.w, ctx.actor)? != entry.account {
        return Err(BulkError::Missing);
    }
    // The next real bounded worker receipt must settle before an idle cancel.
    // This is merely an unacknowledged request; no durable success is emitted.
    entry.cancel = true;
    Ok(())
}
pub(super) fn prepared(
    ctx: &mut Ctx<'_>,
    runtime: &mut Runtime,
    preparing: Preparing,
    result: Result<PreparedRegistry, BulkError>,
) {
    let automatic = preparing.actor == mush_core::GOD && runtime.registry.reconformance.preparing;
    if automatic {
        runtime.registry.reconformance.preparing = false;
    }
    let result = (|| {
        if world::account(ctx.w, preparing.actor)? != preparing.account
            || runtime.registry.entries.contains_key(&preparing.actor)
        {
            return Err(BulkError::Missing);
        }
        let PreparedRegistry { data, machine } = result?;
        if data.account != preparing.account
            || data.character != preparing.actor
            || data.nonce != preparing.nonce
        {
            return Err(BulkError::Missing);
        }
        let reconformance = data
            .plan
            .registry
            .as_ref()
            .is_some_and(|b| b.reconformance.is_some());
        if reconformance != automatic {
            return Err(BulkError::Stale);
        }
        let start = data.plan.start.sha256;
        let tickets = &mut ctx
            .w
            .module_state_mut::<Artifacts>()
            .ok_or(BulkError::Missing)?
            .registry;
        let ticket = tickets.admit(data)?;
        // No selector has been submitted before this point. A preparation
        // refusal can drop its unadmitted machine safely; accepted custody cannot.
        runtime.external_preparing.insert(preparing.account);
        runtime.registry.entries.insert(
            preparing.actor,
            Entry {
                reconformance,
                id: ticket.get(),
                account: preparing.account,
                actor: preparing.actor,
                binding: None,
                pending: Some(Pending {
                    ticket,
                    transition: Transition::Admit,
                    ack: None,
                    attempt: Attempt::Unsubmitted,
                }),
                phase: Phase::Admission {
                    machine: Some(machine),
                    start,
                },
                cancel: false,
                schedule_after: 0,
            },
        );
        Ok(())
    })();
    if let Err(error) = result {
        if automatic {
            runtime.registry.reconformance.next_ms = world::now_ms().saturating_add(60_000);
            tracing::debug!(%error,"Registry re-conformance preparation retained for later scan");
            return;
        }
        ctx.out.info(
            preparing.actor,
            format!("Registry preparation refused: {error}"),
        );
    }
}
pub(super) fn acknowledged(ctx: &mut Ctx<'_>, runtime: &mut Runtime) {
    let Some(artifacts) = ctx.w.module_state_mut::<Artifacts>() else {
        return;
    };
    for _ in 0..runtime.cfg.queued + runtime.cfg.proving_workers {
        let Some(ack) = artifacts.registry.peek_acknowledged() else {
            break;
        };
        if !runtime.registry.accept_ack(ack) {
            tracing::error!("Registry acknowledgement has no retained continuation");
            break;
        }
        artifacts.registry.take_acknowledged();
    }
}
fn continue_owned(ctx: &mut Ctx<'_>) -> Result<(), BulkError> {
    let id = ctx
        .args
        .raw()
        .trim()
        .parse::<u64>()
        .map_err(|_| BulkError::Missing)?;
    let config = ctx
        .w
        .module_state::<Configuration>()
        .cloned()
        .ok_or(BulkError::Missing)?;
    let service = world::service(ctx.w)?;
    let mut runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
    let Some(mut entry) = runtime.registry.entries.remove(&ctx.actor) else {
        return Err(BulkError::Missing);
    };
    let result = if entry.id != id {
        Err(BulkError::Missing)
    } else {
        advance(ctx, &config, &mut runtime, &mut entry)
    };
    if !matches!(entry.phase, Phase::Finished) {
        runtime.registry.entries.insert(entry.actor, entry);
    }
    result
}
fn advance(
    ctx: &mut Ctx<'_>,
    config: &Configuration,
    runtime: &mut Runtime,
    entry: &mut Entry,
) -> Result<(), BulkError> {
    if entry.actor != ctx.actor {
        return Err(BulkError::Missing);
    }
    if let Some(ack) = entry.pending.as_ref().and_then(|p| p.ack) {
        let execution = reconformance::execution(ctx, config, ack.job, entry.reconformance)?;
        if execution.updated_by() != ack.operation
            || execution.job() != ack.job
            || execution.admission().character != entry.actor
            || execution.admission().reconformance.is_some() != entry.reconformance
        {
            return Err(BulkError::Stale);
        }
        let binding = Binding {
            job: ack.job,
            account: entry.account,
            character: entry.actor,
            run: RunId(execution.admission().run),
        };
        if entry.binding.is_some_and(|old| old != binding) {
            return Err(BulkError::Stale);
        }
        // Readback is joined to the actual private quota image as well as the
        // acknowledged typed image; a token alone is never reconstructed evidence.
        let actual = ctx
            .w
            .module_state::<BulkStore>()
            .ok_or(BulkError::Missing)?
            .account_run(entry.account, binding.run)?;
        if actual.registry_job != Some(binding.job) || actual.character != entry.actor {
            return Err(BulkError::Stale);
        }
        super::registry_projection::require_before(
            entry.account,
            binding.job,
            actual,
            execution.admission(),
            execution.observation(),
        )?;
        match (&mut entry.phase, ack.transition) {
            (Phase::Admission { machine, start }, Transition::Admit) => {
                if execution.observation().phase
                    != mush_roll::commerce::rating::execution::ExecutionPhase::Queued
                {
                    return Err(BulkError::Stale);
                }
                let machine = machine.take().ok_or(BulkError::Missing)?;
                entry.phase = Phase::Ready(Some(Ready::registry(binding, *start, machine)));
                runtime.external_preparing.remove(&entry.account);
                if !entry.reconformance {
                    ctx.out
                        .info(entry.actor, "Registry work retained and queued.");
                }
            }
            (Phase::Grant(ready), Transition::Grant) => {
                if ready.is_none() || runtime.registry.pending_turn != Some(entry.actor) {
                    return Err(BulkError::Missing);
                }
                let lease = ctx
                    .w
                    .module_state_mut::<BulkStore>()
                    .ok_or(BulkError::Missing)?
                    .take_registry_lease(ack.operation)
                    .ok_or(BulkError::Missing)?;
                entry.phase = Phase::Waiting {
                    ready: ready.take(),
                    lease: Some(lease),
                };
                runtime.registry.pending_turn = None;
            }
            (Phase::Settle(ready), Transition::Settle)
            | (Phase::Cancel(ready), Transition::Cancel) => {
                if ack.transition == Transition::Cancel {
                    if runtime.registry.pending_turn != Some(entry.actor) {
                        return Err(BulkError::Stale);
                    }
                    runtime.registry.pending_turn = None;
                }
                if execution.observation().phase.terminal() {
                    entry.phase = Phase::Finished;
                    if !entry.reconformance {
                        ctx.out.info(entry.actor, "Registry work finished.");
                    }
                } else {
                    entry.phase = Phase::Ready(ready.take());
                }
            }
            (Phase::Recover(_), Transition::Recover) => {
                if execution.observation().phase
                    != mush_roll::commerce::rating::execution::ExecutionPhase::Interrupted
                    || execution.observation().result.is_some()
                {
                    return Err(BulkError::Stale);
                }
                entry.phase = Phase::Finished;
            }
            _ => return Err(BulkError::Stale),
        }
        #[cfg(test)]
        super::registry_timing::acknowledged(ack.job, ack.transition, execution.observation());
        entry.binding = Some(binding);
        entry.pending = None;
    }
    if consume_attempt(
        entry,
        &mut ctx
            .w
            .module_state_mut::<Artifacts>()
            .ok_or(BulkError::Missing)?
            .registry,
        &mut runtime.external_preparing,
    )? {
        if !entry.reconformance {
            ctx.out
                .info(entry.actor, "Registry preparation was not admitted.");
        }
        return Ok(());
    }
    if entry.pending.is_none() {
        let binding = entry.binding;
        match &mut entry.phase {
            Phase::Recover(stopped) => {
                let stopped = stopped.as_ref().ok_or(BulkError::Missing)?;
                let binding = stopped.binding();
                if entry.binding != Some(binding)
                    || binding.character != entry.actor
                    || binding.account != entry.account
                {
                    return Err(BulkError::Stale);
                }
                // The final Roll/source/commerce audit and current registered
                // owner readiness are checked by this actual actor-stamped
                // reader before any recovery ticket can exist. Preserve old
                // compatibility; the closed Recover provider fences history.
                let execution =
                    reconformance::execution(ctx, config, binding.job, entry.reconformance)?;
                if execution.observation().phase.terminal() {
                    return Err(BulkError::Stale);
                }
                let actual = ctx
                    .w
                    .module_state::<BulkStore>()
                    .ok_or(BulkError::Missing)?
                    .account_run(binding.account, binding.run)?;
                super::registry_projection::require_before(
                    binding.account,
                    binding.job,
                    actual,
                    execution.admission(),
                    execution.observation(),
                )?;
                let ticket = ctx
                    .w
                    .module_state_mut::<Artifacts>()
                    .ok_or(BulkError::Missing)?
                    .registry
                    .recover_stopped(stopped)?;
                entry.phase = Phase::Recover(None);
                entry.pending = Some(Pending {
                    ticket,
                    transition: Transition::Recover,
                    ack: None,
                    attempt: Attempt::Unsubmitted,
                });
            }
            Phase::Grant(ready) => {
                let binding = binding.ok_or(BulkError::Missing)?;
                let ready_ref = ready.as_ref().ok_or(BulkError::Missing)?;
                let tickets = &mut ctx
                    .w
                    .module_state_mut::<Artifacts>()
                    .ok_or(BulkError::Missing)?
                    .registry;
                let transition = if entry.cancel {
                    Transition::Cancel
                } else {
                    Transition::Grant
                };
                let ticket = if entry.cancel {
                    tickets.cancel(binding)?
                } else {
                    tickets.grant(binding, ready_ref.next_grant_ms())?
                };
                if entry.cancel {
                    entry.phase = Phase::Cancel(ready.take());
                }
                entry.pending = Some(Pending {
                    ticket,
                    transition,
                    ack: None,
                    attempt: Attempt::Unsubmitted,
                });
            }
            Phase::Returned(done) => {
                let done_ref = done.as_mut().ok_or(BulkError::Missing)?;
                let prepared = match done_ref.registry_settlement.take() {
                    Some(Ok(value)) => value,
                    Some(Err(error)) => {
                        done_ref.registry_settlement = Some(Err(error.clone()));
                        return Err(error);
                    }
                    None => return Err(BulkError::Missing),
                };
                let Some(artifacts) = ctx.w.module_state_mut::<Artifacts>() else {
                    done_ref.registry_settlement = Some(Ok(prepared));
                    return Err(BulkError::Missing);
                };
                let ticket = match artifacts.registry.insert_settlement(prepared) {
                    Ok(ticket) => ticket,
                    Err((error, prepared)) => {
                        done_ref.registry_settlement = Some(Ok(prepared));
                        return Err(error);
                    }
                };
                entry.phase = Phase::Settle(done_ref.ready.take());
                entry.pending = Some(Pending {
                    ticket,
                    transition: Transition::Settle,
                    ack: None,
                    attempt: Attempt::Unsubmitted,
                });
            }
            _ => {}
        }
    }
    if let Some(pending) = &mut entry.pending {
        if pending.ack.is_some()
            || !matches!(pending.attempt, Attempt::Unsubmitted)
            || ctx.authoritative_pending(entry.actor)
        {
            return Ok(());
        }
        let actions = if entry.reconformance {
            &config
                .reconformance
                .as_ref()
                .ok_or(BulkError::Missing)?
                .actions
        } else {
            &config.actions
        };
        let action_id = if entry.reconformance {
            pending.transition.reconformance_action_id()
        } else {
            pending.transition.action_id()
        };
        let action = actions
            .iter()
            .find(|a| a.key().id.as_str() == action_id)
            .ok_or(BulkError::Missing)?
            .clone();
        let key = IdempotencyKeyText::parse_canonical(format!(
            "registry/{}/{}",
            entry.id,
            pending.ticket.get()
        ))
        .map_err(|_| BulkError::Missing)?;
        // No measurements, identities, actor override, or writes are submitted.
        // The exact ticket's self-only offer supplies all fixed parameters.
        #[cfg(test)]
        super::registry_timing::submit(ctx.w, entry.binding, pending.transition);
        let observation = ctx
            .submit_authoritative_observed(
                action,
                PerceivedRef::SelfActor,
                vec![],
                serde_json::json!({}),
                key,
            )
            .map_err(|_| BulkError::Stale)?;
        // One bounded observation per pending ticket. An unresolved attempt
        // is never replaced: a later refusal could not prove every earlier
        // attempt absent. Actual Ack/readback above can still complete it.
        pending.attempt = Attempt::Observed(observation);
    }
    Ok(())
}
pub(super) fn completed(_ctx: &mut Ctx<'_>, runtime: &mut Runtime, done: Completion) {
    let account = done.receipt.account();
    let run = done.receipt.run();
    if let Some(entry) = runtime
        .registry
        .entries
        .values_mut()
        .find(|e| e.account == account && e.binding.is_some_and(|b| b.run == run))
        && matches!(entry.phase, Phase::Working)
    {
        entry.phase = Phase::Returned(Some(Box::new(done)));
        entry.schedule_after = 0;
        return;
    }
    // This can only indicate corrupted in-process custody. Keep the real paid
    // receipt in the existing bounded repair queue rather than publish legacy.
    tracing::error!("Registry worker completion lacks its retained continuation");
    runtime
        .retry
        .push_back(super::world::PendingCompletion { done, blob: None });
}
pub(super) fn grant(_ctx: &mut Ctx<'_>, runtime: &mut Runtime, ready: Ready) {
    let Err(ready) = runtime.registry.take_fair_turn(ready) else {
        return;
    };
    if let Err((_, ready)) = runtime.queue.push(ready) {
        runtime.registry.stranded_ready.push(ready);
        tracing::error!("Registry continuation exceeded its reserved queue");
    }
}
pub(super) fn dispatch(ctx: &mut Ctx<'_>, runtime: &mut Runtime) {
    // One bounded cold binding enters the ordinary actor-dispatched lane per
    // maintenance/wakeup. Same-actor old accounts serialize; no source or worker
    // is reconstructed. A missing/Unknown acknowledgment retains this entry.
    if configured(ctx.w)
        && runtime.registry.entries.len() < super::registry_recovery::MAX_STOPPED_JOBS
        && let Some(next) = runtime.next.checked_add(1)
    {
        let id = runtime.next;
        if let Some(stopped) = runtime
            .recovery
            .take_available(|binding| !runtime.registry.entries.contains_key(&binding.character))
        {
            let binding = stopped.binding();
            runtime.next = next;
            let phase = if let Some(reason) = super::registry_recovery::unavailable(ctx.w, binding)
            {
                tracing::warn!(job = %binding.job, ?reason, "Registry stopped job retained; owner repair required");
                Phase::Unrecoverable {
                    _stopped: stopped,
                    _reason: reason,
                }
            } else {
                Phase::Recover(Some(stopped))
            };
            let reconformance = ctx
                .w
                .module_state::<BulkStore>()
                .and_then(|s| s.run(binding.account, binding.character, binding.run).ok())
                .and_then(|r| r.plan.registry.as_ref())
                .is_some_and(|b| b.reconformance.is_some());
            runtime.registry.entries.insert(
                binding.character,
                Entry {
                    reconformance,
                    id,
                    account: binding.account,
                    actor: binding.character,
                    binding: Some(binding),
                    pending: None,
                    phase,
                    cancel: false,
                    schedule_after: 0,
                },
            );
        }
    }
    let continuation = ctx
        .w
        .module_state::<Configuration>()
        .map(|config| config.continuation.clone());
    let now = world::now_ms();
    for entry in runtime.registry.entries.values_mut() {
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
                // Both values are present together; no submission consumes only
                // one half. The executor returns both on backpressure.
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
        if let Some(handle) = &continuation
            && let Some((actor, argument)) =
                entry.continuation(now, ctx.authoritative_pending(entry.actor))
        {
            // The exact registered command preserves actor/module attribution
            // without feeding a private ticket into the actor's input context.
            // Refusal only delays work; no observation or custody is discarded.
            if let Err(error) = ctx.schedule_continuation(handle, actor, &argument, 0) {
                tracing::debug!(%error, "Registry continuation scheduling retained");
            }
        }
    }
    reconformance::schedule(ctx, runtime);
}

/// Read-only fixture delivery hint. The fixture still invokes the real registered
/// command as GOD and resolves every private offer through actual Core ingress.
/// No worker result, ticket, permit, timer state or owner image is synthesized.
#[cfg(test)]
pub(in crate::minds) fn fixture_entry(world: &mush_core::World) -> Option<u64> {
    let service = world::service(world).ok()?;
    let runtime = service.0.lock().ok()?;
    runtime
        .registry
        .entries
        .get(&mush_core::GOD)
        .map(|entry| entry.id)
}
