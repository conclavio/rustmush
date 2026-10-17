//! Command/service boundary for private proving. Simulation, source compilation
//! and large artifact encoding stay off-world. Only owner checks and prepared
//! publication run here, then the writer high-water releases a worker lease.
use super::{
    BulkError,
    artifacts::{Artifacts, Key, Part, PreparedBlob},
    commit::CommitLatch,
    executor::{Completion, Executor, FairQueue, Ready},
    invalid,
    model::{BulkConfig, RunId, Scenario, Status},
    owner::{BulkStore, Lease, Receipt},
    preparation::{self, Preparer},
    publication,
    scenarios::{Recipe, ScenarioFactory},
};
use mush_core::{AccountId, Ctx, ObjId, World};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

struct Preparing {
    account: AccountId,
    actor: ObjId,
    nonce: u64,
}
struct Waiting {
    lease: Lease,
    ready: Ready,
    latch: CommitLatch,
}
struct PendingCompletion {
    done: Completion,
    blob: Option<PreparedBlob>,
}
struct Runtime {
    cfg: BulkConfig,
    factory: Option<Arc<ScenarioFactory>>,
    preparer: Preparer,
    executor: Executor,
    next: u64,
    preparing: BTreeMap<u64, Preparing>,
    queue: FairQueue,
    waiting: Vec<Waiting>,
    retry: VecDeque<PendingCompletion>,
    completed_benches: VecDeque<BenchComplete>,
    external_preparing: BTreeSet<AccountId>,
}
#[derive(Clone)]
pub(crate) struct BulkService(Arc<Mutex<Runtime>>);
impl BulkService {
    pub fn new(
        cfg: BulkConfig,
        wake: tokio::sync::mpsc::Sender<crate::arena_task::ArenaOut>,
    ) -> Result<Self, BulkError> {
        Ok(Self(Arc::new(Mutex::new(Runtime {
            preparer: Preparer::new(cfg.queued, wake.clone())?,
            executor: Executor::with_wake(&cfg, Some(wake))?,
            queue: FairQueue::new(&cfg)?,
            cfg,
            factory: None,
            next: 1,
            preparing: BTreeMap::new(),
            waiting: Vec::new(),
            retry: VecDeque::new(),
            completed_benches: VecDeque::new(),
            external_preparing: BTreeSet::new(),
        }))))
    }
}
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
fn account(w: &World, actor: ObjId) -> Result<AccountId, BulkError> {
    w.module_state::<mush_sim::AccountOracle>()
        .and_then(|o| o.account_of(actor))
        .ok_or(BulkError::Missing)
}
fn service(w: &World) -> Result<BulkService, BulkError> {
    w.module_state::<BulkService>()
        .cloned()
        .ok_or(BulkError::Missing)
}
/// All modules have loaded: snapshot actual fielded content/Atlas once, before
/// commands. A server with missing authored ground refuses preparation.
pub(crate) fn ready(ctx: &mut Ctx<'_>) -> Result<(), BulkError> {
    let owner = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .ok_or(BulkError::Missing)?;
    let cfg = owner.config().clone();
    if !cfg.enabled {
        return Ok(());
    }
    let catalog = Arc::clone(&owner.definitions);
    let records = Arc::new(
        mush_atlas::place::records(ctx.w)
            .ok_or(BulkError::Missing)?
            .clone(),
    );
    let service = service(ctx.w)?;
    service
        .0
        .lock()
        .map_err(|_| invalid("proving service unavailable"))?
        .factory = Some(Arc::new(ScenarioFactory::new(catalog, records, cfg)));
    Ok(())
}

struct Words {
    candidate: String,
    opponent: String,
    games: u16,
    scenario: Scenario,
    size: u8,
    seed: u64,
}
const USAGE: &str = "@mind/prove <mind[/profile]> VS <owned mind[/profile]|Standard[/preset]|mirror> [10..100 even] [DUEL|MUSTER|HOLD] [1v1..6v6] [SEED <u64>]";
pub(super) fn tokens(text: &str) -> Result<Vec<String>, BulkError> {
    if text.len() > 1024 {
        return Err(invalid(USAGE));
    }
    let mut left = text;
    let mut words = Vec::new();
    while !left.trim_start().is_empty() {
        left = left.trim_start();
        if left.starts_with('"') {
            let mut escaped = false;
            let mut end = None;
            for (n, c) in left.char_indices().skip(1) {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    end = Some(n + 1);
                    break;
                }
            }
            let end = end.ok_or_else(|| invalid("unterminated quoted mind reference"))?;
            let token = serde_json::from_str::<String>(&left[..end])
                .map_err(|_| invalid("invalid JSON-string mind reference"))?;
            left = &left[end..];
            if left.chars().next().is_some_and(|c| !c.is_whitespace()) {
                return Err(invalid("quoted references must be separate words"));
            }
            if token.chars().any(char::is_control) {
                return Err(invalid("control characters in mind reference"));
            }
            words.push(token);
        } else {
            let end = left.find(char::is_whitespace).unwrap_or(left.len());
            words.push(left[..end].to_owned());
            left = &left[end..];
        }
        if words.len() > 9 {
            return Err(invalid(USAGE));
        }
    }
    Ok(words)
}
fn parse(text: &str) -> Result<Words, BulkError> {
    if text.len() > 1024 {
        return Err(invalid(USAGE));
    }
    let words = tokens(text)?;
    let [candidate, separator, opponent, ..] = words.as_slice() else {
        return Err(invalid(USAGE));
    };
    if !separator.eq_ignore_ascii_case("vs") {
        return Err(invalid(USAGE));
    }
    let mut out = Words {
        candidate: candidate.clone(),
        opponent: opponent.clone(),
        games: 30,
        scenario: Scenario::Muster,
        size: 1,
        seed: 1,
    };
    let mut at = 3;
    if let Some(n) = words.get(at).and_then(|w| w.parse::<u16>().ok()) {
        out.games = n;
        at += 1;
    }
    if let Some(word) = words.get(at) {
        out.scenario = match word.to_ascii_lowercase().as_str() {
            "duel" => {
                at += 1;
                Scenario::Duel
            }
            "muster" => {
                at += 1;
                Scenario::Muster
            }
            "hold" => {
                at += 1;
                Scenario::Hold
            }
            _ => out.scenario,
        };
    }
    if let Some(word) = words.get(at)
        && let Some((left, right)) = word.split_once('v')
    {
        let n = left.parse::<u8>().map_err(|_| invalid(USAGE))?;
        if right.parse::<u8>().ok() != Some(n) {
            return Err(invalid(USAGE));
        }
        out.size = n;
        at += 1;
    }
    if words
        .get(at)
        .is_some_and(|w| w.eq_ignore_ascii_case("seed"))
    {
        out.seed = words
            .get(at + 1)
            .and_then(|w| w.parse().ok())
            .ok_or_else(|| invalid(USAGE))?;
        at += 2;
    }
    if at != words.len()
        || !(10..=100).contains(&out.games)
        || !out.games.is_multiple_of(2)
        || !(1..=6).contains(&out.size)
    {
        return Err(invalid(USAGE));
    }
    Ok(out)
}
pub(crate) fn command(ctx: &mut Ctx<'_>, switch: &str, text: &str) -> Result<(), BulkError> {
    let account = account(ctx.w, ctx.actor)?;
    let now = now_ms();
    if switch == "prove" {
        let words = parse(text)?;
        let service = service(ctx.w)?;
        let mut runtime = service
            .0
            .lock()
            .map_err(|_| invalid("proving service unavailable"))?;
        let store = ctx
            .w
            .module_state::<BulkStore>()
            .ok_or(BulkError::Missing)?;
        if store.active(account).is_some()
            || (runtime.preparing.values().any(|p| p.account == account)
                || runtime.external_preparing.contains(&account))
        {
            return Err(BulkError::Busy);
        }
        if runtime.preparing.len() + runtime.external_preparing.len() + store.active_count()
            >= runtime.cfg.queued + runtime.cfg.proving_workers
        {
            return Err(BulkError::QueueFull);
        }
        let factory = runtime
            .factory
            .clone()
            .ok_or_else(|| invalid("proving is unavailable on this server"))?;
        let roll = ctx
            .w
            .module_state::<mush_roll::RollStore>()
            .ok_or(BulkError::Missing)?;
        let sources = ctx
            .w
            .module_state::<crate::store::TacsimStore>()
            .ok_or(BulkError::Missing)?;
        let recipe = Recipe::owned(
            roll,
            sources,
            ctx.actor,
            &words.candidate,
            &words.opponent,
            words.scenario,
            words.size,
            words.games,
            words.seed,
        )
        .map_err(|_| BulkError::Missing)?;
        let nonce = store.next_nonce(account)?;
        let ticket = runtime.next;
        let next = ticket
            .checked_add(1)
            .ok_or_else(|| invalid("preparation sequence exhausted"))?;
        runtime
            .preparer
            .submit(preparation::Request {
                ticket,
                recipe,
                factory,
            })
            .map_err(|(e, _)| e)?;
        runtime.next = next;
        runtime.preparing.insert(
            ticket,
            Preparing {
                account,
                actor: ctx.actor,
                nonce,
            },
        );
        ctx.out.info(ctx.actor,"Proving preparation queued. The owned run number follows once its frozen start is retained.");
        return Ok(());
    }
    let store = ctx
        .w
        .module_state::<BulkStore>()
        .ok_or(BulkError::Missing)?;
    if switch == "proves" {
        if !text.trim().is_empty() {
            return Err(invalid("@mind/proves"));
        }
        let runs = match store.runs(account, ctx.actor) {
            Ok(r) => r,
            Err(BulkError::Missing) => Vec::new(),
            Err(e) => return Err(e),
        };
        for run in runs.iter().rev().take(50) {
            ctx.out.info(
                ctx.actor,
                format!(
                    "Run {}: {:?}; {} games; {:.3} unit-minutes.",
                    run.id.0,
                    run.status,
                    run.plan.games,
                    run.charged_unit_ms as f64 / 60_000.0
                ),
            );
        }
        if runs.is_empty() {
            ctx.out.info(ctx.actor, "No retained proving runs.");
        }
        return Ok(());
    }
    let mut words = text.split_whitespace();
    let run = RunId(
        words
            .next()
            .and_then(|w| w.parse().ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid("owned run number required"))?,
    );
    let row = store.run(account, ctx.actor, run)?;
    if row.plan.kind != super::model::WorkKind::Proving {
        return Err(BulkError::Missing);
    }
    match switch {
        "proving" => {
            if words.next().is_some() {
                return Err(invalid("@mind/proving <run>"));
            }
            ctx.out.info(
                ctx.actor,
                format!(
                    "Run {}: {:?}; {} paired games; {:.3} unit-minutes charged.",
                    run.0,
                    row.status,
                    row.plan.games,
                    row.charged_unit_ms as f64 / 60_000.0
                ),
            );
        }
        "report" => {
            let page = words
                .next()
                .map(|w| w.parse::<usize>())
                .transpose()
                .map_err(|_| invalid("report page"))?
                .unwrap_or(1);
            if page == 0 || words.next().is_some() {
                return Err(invalid("@mind/report <run> [page]"));
            }
            if row.status != Status::Complete {
                return Err(invalid("that proving run has no completed report"));
            }
            let manifest = ctx
                .w
                .module_state::<Artifacts>()
                .ok_or(BulkError::Missing)?
                .get(Key {
                    account,
                    run,
                    part: Part::Result,
                })?;
            let pages = manifest.summary.len().div_ceil(15).max(1);
            if page > pages {
                return Err(invalid("report page outside retained report"));
            }
            let lines = manifest
                .summary
                .iter()
                .skip((page - 1) * 15)
                .take(15)
                .cloned()
                .collect::<Vec<_>>();
            ctx.out.info(
                ctx.actor,
                format!("Run {} report, page {page}/{pages}.", run.0),
            );
            for line in lines {
                ctx.out.info(ctx.actor, line);
            }
        }
        "cancel" | "forget" => {
            if words.next().is_some() {
                return Err(invalid("one owned run number required"));
            }
            let prepared = if switch == "cancel" {
                store.prepare_cancel(account, ctx.actor, run, now)?
            } else {
                store.prepare_forget(account, ctx.actor, run)?
            };
            publication::publish(ctx, prepared, &mut None, None)?;
            ctx.out.info(ctx.actor,if switch=="cancel"{"Cancellation recorded; any in-flight bounded work must settle before the account is free."}else{"Proving run forgotten."});
            drain(ctx);
        }
        _ => return Err(invalid("unknown proving action")),
    }
    Ok(())
}

/// Called after any egress drain, including a saturated notification channel;
/// wake tokens contain no source, actor identity, artifact or mutable arena.
pub(crate) fn drain(ctx: &mut Ctx<'_>) {
    let Ok(service) = service(ctx.w) else { return };
    let Ok(mut runtime) = service.0.lock() else {
        return;
    };
    let now = now_ms();
    for _ in 0..2 {
        let Some(done) = runtime.preparer.poll() else {
            break;
        };
        let Some(p) = runtime.preparing.remove(&done.ticket) else {
            continue;
        };
        let result = (|| {
            if account(ctx.w, p.actor)? != p.account {
                return Err(BulkError::Missing);
            }
            done.recipe
                .revalidate(
                    ctx.w
                        .module_state::<mush_roll::RollStore>()
                        .ok_or(BulkError::Missing)?,
                    ctx.w
                        .module_state::<crate::store::TacsimStore>()
                        .ok_or(BulkError::Missing)?,
                    p.actor,
                )
                .map_err(|_| BulkError::Missing)?;
            let mut frozen = done.result?;
            let (run, change) = ctx
                .w
                .module_state::<BulkStore>()
                .ok_or(BulkError::Missing)?
                .prepare_admit(p.account, p.actor, p.nonce, frozen.plan().clone(), now)?;
            let mut blob = Some(PreparedBlob::prepare(
                Key {
                    account: p.account,
                    run,
                    part: Part::Start,
                },
                frozen.bytes_mut(),
                Vec::new(),
            )?);
            publication::publish(
                ctx,
                change,
                &mut blob,
                Some((run, done.recipe.references())),
            )?;
            // Capacity was reserved by account admission; no fallible command
            // exit follows publication, even if an internal invariant breaks.
            if runtime.queue.push(frozen.ready(p.account, run)).is_err() {
                tracing::error!("admitted proving continuation exceeded its reserved queue");
            }
            ctx.out.info(
                p.actor,
                format!("Proving run {} retained and queued.", run.0),
            );
            Ok(())
        })();
        if let Err(error) = result {
            ctx.out
                .info(p.actor, format!("Proving preparation refused: {error}"));
        }
    }
    for _ in 0..runtime.cfg.proving_workers {
        let Some(mut pending) = runtime.retry.pop_front().or_else(|| {
            runtime
                .executor
                .poll()
                .map(|done| PendingCompletion { done, blob: None })
        }) else {
            break;
        };
        let done = &mut pending.done;
        let account = done.receipt.account();
        let run = done.receipt.run();
        let result: Result<(), BulkError> = (|| {
            let store = ctx
                .w
                .module_state::<BulkStore>()
                .ok_or(BulkError::Missing)?;
            let actor = store.account_run(account, run)?.character;
            let change = store.prepare_finish(done.receipt.clone(), now)?;
            let status = change.resulting_run(run).ok_or(BulkError::Missing)?.status;
            if pending.blob.is_none()
                && let Some(bytes) = done.artifact.as_mut()
            {
                let summary = bytes.take_summary();
                pending.blob = Some(PreparedBlob::prepare(
                    Key {
                        account,
                        run,
                        part: Part::Result,
                    },
                    bytes,
                    summary,
                )?);
            }
            publication::publish(ctx, change, &mut pending.blob, None)?;
            if let Some(ready) = done.ready.take() {
                if !status.terminal() {
                    if runtime.queue.push(ready).is_err() {
                        tracing::error!("bulk continuation lost its reserved queue slot");
                    }
                } else if ready.bench_picture().is_some() {
                    runtime.completed_benches.push_back(BenchComplete {
                        account,
                        run,
                        status,
                        ready: Some(ready),
                    });
                }
            } else if status.terminal()
                && store_kind(ctx.w, account, run) == Some(super::model::WorkKind::Bench)
            {
                runtime.completed_benches.push_back(BenchComplete {
                    account,
                    run,
                    status,
                    ready: None,
                });
            }
            if status.terminal()
                && store_kind(ctx.w, account, run) == Some(super::model::WorkKind::Proving)
            {
                ctx.out
                    .info(actor, format!("Proving run {} is {:?}.", run.0, status));
            }
            Ok(())
        })();
        if let Err(error) = result {
            tracing::error!(%error,"bulk completion retained pending repair");
            runtime.retry.push_back(pending);
            break;
        }
    }
    let mut retained = Vec::new();
    for waiting in std::mem::take(&mut runtime.waiting) {
        if !waiting.latch.committed() {
            retained.push(waiting);
            continue;
        }
        let cancelled = ctx
            .w
            .module_state::<BulkStore>()
            .and_then(|s| {
                s.account_run(waiting.lease.account(), waiting.lease.run())
                    .ok()
            })
            .is_none_or(|r| r.status == Status::CancelRequested);
        if cancelled {
            runtime.retry.push_back(PendingCompletion {
                done: Completion {
                    artifact: None,
                    receipt: Receipt {
                        actual_unit_ms: 0,
                        result: None,
                        terminal: Some(Status::Cancelled),
                        lease: waiting.lease,
                    },
                    ready: None,
                },
                blob: None,
            });
        } else if let Err((_, lease, ready)) = runtime.executor.submit(waiting.lease, waiting.ready)
        {
            retained.push(Waiting {
                lease,
                ready,
                latch: waiting.latch,
            });
        }
    }
    runtime.waiting = retained;
    for _ in 0..runtime.cfg.queued + runtime.cfg.proving_workers {
        let Some(ready) = runtime.queue.pop() else {
            break;
        };
        let account = ready.account();
        let run = ready.run();
        let store = match ctx.w.module_state::<BulkStore>() {
            Some(s) => s,
            None => break,
        };
        if !store
            .account_run(account, run)
            .is_ok_and(|r| !r.status.terminal())
        {
            if ready.bench_picture().is_some() {
                let status = store
                    .account_run(account, run)
                    .map_or(Status::Interrupted, |r| r.status);
                runtime.completed_benches.push_back(BenchComplete {
                    account,
                    run,
                    status,
                    ready: None,
                });
            }
            continue;
        }
        let change = match store.prepare_grant(account, run, now, ready.next_grant_ms()) {
            Ok(p) => p,
            Err(BulkError::Quota) => {
                let _ = runtime.queue.push(ready);
                continue;
            }
            Err(BulkError::Busy) => {
                let _ = runtime.queue.push(ready);
                break;
            }
            Err(error) => {
                tracing::error!(%error,"bulk grant refused");
                let _ = runtime.queue.push(ready);
                break;
            }
        };
        match publication::publish(ctx, change, &mut None, None) {
            Ok(Some(lease)) => {
                let latch = CommitLatch::default();
                if let Some(registry) = ctx.w.module_state::<crate::arena_bridge::ArenaRegistry>() {
                    registry.tasks().await_bulk_write(latch.clone());
                }
                runtime.waiting.push(Waiting {
                    lease,
                    ready,
                    latch,
                });
            }
            _ => {
                let _ = runtime.queue.push(ready);
                break;
            }
        }
    }
    drop(runtime);
    super::bench_service::drain(ctx);
}

pub(super) struct BenchComplete {
    pub account: AccountId,
    pub run: RunId,
    pub status: Status,
    pub ready: Option<Ready>,
}
fn store_kind(w: &World, account: AccountId, run: RunId) -> Option<super::model::WorkKind> {
    Some(
        w.module_state::<BulkStore>()?
            .account_run(account, run)
            .ok()?
            .plan
            .kind,
    )
}
pub(super) fn hold_preparation(w: &World, account: AccountId) -> Result<(), BulkError> {
    let service = service(w)?;
    let mut runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
    let owner = w.module_state::<BulkStore>().ok_or(BulkError::Missing)?;
    if owner.active(account).is_some()
        || runtime.preparing.values().any(|p| p.account == account)
        || runtime.external_preparing.contains(&account)
    {
        return Err(BulkError::Busy);
    }
    if runtime.preparing.len() + runtime.external_preparing.len() + owner.active_count()
        >= runtime.cfg.queued + runtime.cfg.proving_workers
    {
        return Err(BulkError::QueueFull);
    }
    runtime.external_preparing.insert(account);
    Ok(())
}
pub(super) fn release_preparation(w: &World, account: AccountId) {
    if let Ok(service) = service(w)
        && let Ok(mut runtime) = service.0.lock()
    {
        runtime.external_preparing.remove(&account);
    }
}
pub(super) fn take_bench_completions(w: &World) -> Vec<BenchComplete> {
    service(w)
        .ok()
        .and_then(|service| {
            service
                .0
                .lock()
                .ok()
                .map(|mut runtime| runtime.completed_benches.drain(..).collect())
        })
        .unwrap_or_default()
}
pub(super) fn admit_bench(
    ctx: &mut Ctx<'_>,
    account: AccountId,
    actor: ObjId,
    plan: super::model::Plan,
    mut ready: Ready,
    command: super::replay::ReplayCommand,
) -> Result<RunId, (BulkError, Ready)> {
    let result = (|| {
        let service = service(ctx.w)?;
        let runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
        if runtime.preparing.values().any(|p| p.account == account)
            || runtime.external_preparing.contains(&account)
        {
            return Err(BulkError::Busy);
        }
        let owner = ctx
            .w
            .module_state::<BulkStore>()
            .ok_or(BulkError::Missing)?;
        let nonce = owner.next_nonce(account)?;
        let (run, change) = owner.prepare_admit(account, actor, nonce, plan, now_ms())?;
        ready.stage_bench(command)?;
        if let Err(error) = publication::publish(ctx, change, &mut None, None) {
            ready.abort_bench();
            return Err(error);
        }
        drop(runtime);
        Ok((run, service))
    })();
    match result {
        Err(error) => Err((error, ready)),
        Ok((run, service)) => {
            ready.bind_bench_run(run);
            if let Ok(mut runtime) = service.0.lock()
                && runtime.queue.push(ready).is_err()
            {
                tracing::error!("bench lost its reserved continuation slot");
            }
            Ok(run)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prove_grammar_is_bounded_and_unambiguous() {
        let quoted =
            parse(r#""my patrol@7/line" VS "Standard/Hunter" 30 MUSTER 1v1 SEED 1"#).unwrap();
        assert_eq!(quoted.candidate, "my patrol@7/line");
        assert!(parse(r#""bad\nname" VS mirror"#).is_err());
        assert!(parse(r#""foo"bar VS mirror"#).is_err());
        let w = parse("mine/line VS Standard/Hunter 30 MUSTER 2v2 SEED 9").unwrap();
        assert_eq!((w.games, w.size, w.seed), (30, 2, 9));
        assert!(parse("x VS mirror 11").is_err());
        assert!(parse("x VS mirror 30 DUEL 2v3").is_err());
        assert!(parse("x VS mirror 30 DUEL 7v7").is_err());
        assert!(parse("x VS other/line garbage").is_err());
        assert_eq!(parse("x VS mirror").unwrap().games, 30);
    }
}
