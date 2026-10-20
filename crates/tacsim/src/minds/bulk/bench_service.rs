//! Connection-bound private bench orchestration. Reads/verification/compilation
//! are bounded independently; simulations enter the common durable quota lane.
use super::{
    BulkError,
    artifacts::{Artifacts, Key, Part, Read},
    bench::{Benches, Connection, ReferenceId, SessionId},
    bench_prepare::{self, ResultValue, Work, Worker},
    executor::Ready,
    invalid,
    model::{BulkConfig, Plan, RunId, Status, WorkKind},
    owner::BulkStore,
    publication,
    replay::{Picture, ReplayCommand},
    world,
};
use crate::{arena_task::MatchId, match_::mind_tape};
use mush_core::output::ConnId;
use mush_core::{AccountId, CmdError, Ctx, StoreReadResult, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use std::{
    collections::{BTreeMap, btree_map},
    sync::{Arc, Mutex},
};

pub(crate) const FEED: &str = "mind.bench";
struct Input {
    start: Option<Read>,
    result: Option<Read>,
    on_result: bool,
    correlation: u64,
    game: usize,
}
enum Load {
    Work(Box<Work>),
    Read(Box<Input>),
}
enum Phase {
    Reading(Box<Input>),
    Preparing(u64),
    Idle(Ready),
    Running(RunId),
    Evaluating { ticket: u64, ready: Ready },
    Stopped,
}
struct Entry {
    connection: Connection,
    conn: ConnId,
    touched: i64,
    bytes: u64,
    pin: Option<RunId>,
    source_root: Option<mush_roll::ReferenceKey>,
    pending_root: Option<mush_roll::ReferenceKey>,
    source_revision: Option<mush_roll::RevisionRef>,
    pending_revision: Option<mush_roll::RevisionRef>,
    pending_replace: bool,
    fork_bytes: u64,
    reference: ReferenceId,
    session: Option<SessionId>,
    phase: Phase,
    plan: Option<Plan>,
    picture: Option<Arc<Picture>>,
    closing: bool,
    watches: Vec<String>,
}
struct Runtime {
    cfg: BulkConfig,
    worker: Worker,
    next: u64,
    benches: Benches,
    entries: BTreeMap<AccountId, Entry>,
}
#[derive(Clone)]
pub(crate) struct BenchService(Arc<Mutex<Runtime>>);
impl BenchService {
    pub fn new(
        cfg: BulkConfig,
        wake: tokio::sync::mpsc::Sender<crate::arena_task::ArenaOut>,
    ) -> Result<Self, BulkError> {
        Ok(Self(Arc::new(Mutex::new(Runtime {
            worker: Worker::new(cfg.queued, wake)?,
            benches: Benches::new(cfg.clone())?,
            cfg,
            next: 1,
            entries: BTreeMap::new(),
        }))))
    }
}
fn service(w: &World) -> Result<BenchService, BulkError> {
    w.module_state::<BenchService>()
        .cloned()
        .ok_or(BulkError::Missing)
}
fn current(ctx: &Ctx<'_>) -> Result<(Connection, ConnId), BulkError> {
    let conn = ctx.w.dispatching_conn().ok_or(BulkError::Missing)?;
    let (actor, lease) = ctx
        .w
        .connection_binding(conn)
        .filter(|(a, _)| *a == ctx.actor)
        .ok_or(BulkError::Missing)?;
    let account = ctx
        .w
        .module_state::<mush_sim::AccountOracle>()
        .and_then(|o| o.account_of(actor))
        .ok_or(BulkError::Missing)?;
    Ok((
        Connection {
            account,
            character: actor,
            lease,
        },
        conn,
    ))
}
fn authorized(w: &World, row: &Entry) -> bool {
    w.connection_binding(row.conn) == Some((row.connection.character, row.connection.lease))
        && w.module_state::<mush_sim::AccountOracle>()
            .and_then(|o| o.account_of(row.connection.character))
            == Some(row.connection.account)
        && source_access(w, row)
}
fn source_access(w: &World, row: &Entry) -> bool {
    let owner = w.module_state::<mush_roll::RollStore>();
    if let Some(picture) = &row.picture {
        let Some(origin) = &picture.access_origin else {
            return false;
        };
        if !crate::match_::draft_holdings::current_draft_source_access(owner, origin) {
            return false;
        }
    }
    row.source_revision
        .iter()
        .chain(row.pending_revision.iter())
        .all(|reference| {
            owner
                .and_then(|store| store.revision(reference.key.character, reference.key).ok())
                .is_some_and(|revision| {
                    revision.reference() == *reference && revision.require_original_access().is_ok()
                })
        })
}
fn say(ctx: &mut Ctx<'_>, conn: ConnId, text: impl Into<String>) {
    ctx.out
        .to_conn(conn, OutputEvent::Sys(RichText::plain(text.into())));
}
fn emit(ctx: &mut Ctx<'_>, row: &Entry) -> Result<(), BulkError> {
    if !authorized(ctx.w, row) {
        return Err(BulkError::Missing);
    }
    let data = serde_json::json!({"schema":{"id":FEED,"version":1},"state":if row.closing{"closing"}else{match row.phase{Phase::Reading(_)|Phase::Preparing(_)=>"loading",Phase::Running(_)=>"running",Phase::Evaluating{..}=>"evaluating",Phase::Idle(_)=>"ready",Phase::Stopped=>"closed"}},"picture":row.picture.as_deref(),"watches":row.watches});
    super::encoding::encode(&data, 256 * 1024)?;
    ctx.out.to_conn(
        row.conn,
        OutputEvent::Panel {
            feed: FEED.into(),
            kind: PanelFrameKind::Keyframe,
            data,
        },
    );
    Ok(())
}
fn ticket(runtime: &mut Runtime) -> Result<u64, BulkError> {
    let n = runtime.next;
    runtime.next = n
        .checked_add(1)
        .ok_or_else(|| invalid("bench ticket allocator"))?;
    Ok(n)
}
fn pin(
    ctx: &mut Ctx<'_>,
    connection: Connection,
    run: RunId,
    value: bool,
) -> Result<(), BulkError> {
    let change = ctx
        .w
        .module_state::<BulkStore>()
        .ok_or(BulkError::Missing)?
        .prepare_pin(connection.account, connection.character, run, value)?;
    publication::publish(ctx, change, &mut None, None)?;
    Ok(())
}
fn clean(ctx: &mut Ctx<'_>, runtime: &mut Runtime, account: AccountId) -> Result<(), BulkError> {
    let btree_map::Entry::Occupied(entry) = runtime.entries.entry(account) else {
        return Ok(());
    };
    let row = entry.get();
    if matches!(
        row.phase,
        Phase::Preparing(_) | Phase::Evaluating { .. } | Phase::Running(_)
    ) {
        return Ok(());
    }
    let releases = row
        .source_root
        .iter()
        .chain(row.pending_root.iter())
        .cloned()
        .collect();
    if let Some(run) = row.pin {
        let change = ctx
            .w
            .module_state::<BulkStore>()
            .ok_or(BulkError::Missing)?
            .prepare_pin(row.connection.account, row.connection.character, run, false)?;
        publication::release_bench(ctx, change, releases)?;
    } else {
        super::bench_roots::change(ctx, None, releases)?;
    }
    let row = entry.remove();
    runtime.benches.close(row.connection);
    world::release_preparation(ctx.w, account);
    Ok(())
}
fn close(ctx: &mut Ctx<'_>, runtime: &mut Runtime, account: AccountId) -> Result<(), BulkError> {
    if let Some(row) = runtime.entries.get_mut(&account) {
        row.closing = true;
        if let Phase::Running(run) = row.phase {
            let owner = ctx
                .w
                .module_state::<BulkStore>()
                .ok_or(BulkError::Missing)?;
            if owner.account_run(account, run)?.status == Status::CancelRequested {
                return Ok(());
            }
            let change =
                owner.prepare_cancel(account, row.connection.character, run, world::now_ms())?;
            publication::publish(ctx, change, &mut None, None)?;
        }
    }
    clean(ctx, runtime, account)
}
fn take_current(
    entries: &mut BTreeMap<AccountId, Entry>,
    connection: Connection,
) -> Result<Entry, BulkError> {
    match entries.entry(connection.account) {
        btree_map::Entry::Occupied(entry) if entry.get().connection == connection => {
            Ok(entry.remove())
        }
        _ => Err(BulkError::Missing),
    }
}
fn take_idle(row: &mut Entry) -> Result<Ready, BulkError> {
    match std::mem::replace(&mut row.phase, Phase::Stopped) {
        Phase::Idle(ready) => Ok(ready),
        previous => {
            row.phase = previous;
            Err(BulkError::Busy)
        }
    }
}
fn evaluate(
    ctx: &Ctx<'_>,
    runtime: &mut Runtime,
    row: &mut Entry,
    expressions: Vec<String>,
) -> Result<(), BulkError> {
    if !matches!(row.phase, Phase::Idle(_)) {
        return Err(BulkError::Busy);
    }
    let p = row.picture.as_ref().ok_or(BulkError::Busy)?;
    let picture = super::eval::EvalPicture {
        view: p.view.clone(),
        card: p.card.clone().ok_or(BulkError::Missing)?,
        memory: p.memory.clone().ok_or(BulkError::Missing)?,
    };
    let id = ticket(runtime)?;
    let account = row.connection.account;
    let ready = take_idle(row)?;
    if let Err(error) = world::hold_preparation(ctx.w, account) {
        row.phase = Phase::Idle(ready);
        return Err(error);
    }
    if let Err((error, _)) = runtime.worker.submit(bench_prepare::Request {
        ticket: id,
        account,
        work: Work::Eval {
            picture,
            expressions,
            cfg: runtime.cfg.clone(),
        },
    }) {
        world::release_preparation(ctx.w, account);
        row.phase = Phase::Idle(ready);
        return Err(error);
    }
    row.phase = Phase::Evaluating { ticket: id, ready };
    Ok(())
}
/// The only proving-blob read admission: exact durable owner and complete
/// result, before any read request or artifact payload is exposed to a worker.
fn completed_proving(
    owner: &BulkStore,
    account: AccountId,
    character: mush_core::ObjId,
    run: RunId,
    game: usize,
) -> Result<&super::model::Run, BulkError> {
    let row = owner.run(account, character, run)?;
    if row.status != Status::Complete
        || row.plan.kind != WorkKind::Proving
        || game >= usize::from(row.plan.games)
    {
        return Err(BulkError::Missing);
    }
    Ok(row)
}

fn read_next(ctx: &mut Ctx<'_>, input: &mut Input) -> Result<(), BulkError> {
    let reader = if input.on_result {
        input.result.as_ref()
    } else {
        input.start.as_ref()
    }
    .ok_or(BulkError::Missing)?;
    input.correlation = ctx
        .read_store(reader.keys())
        .map_err(|_| invalid("bench artifact read unavailable"))?;
    Ok(())
}
fn time(word: &str) -> Result<i64, BulkError> {
    let fields = word.split(':').collect::<Vec<_>>();
    let seconds = match fields.as_slice() {
        [s] => s.parse::<u64>().ok(),
        [m, s] => m.parse::<u64>().ok().and_then(|m| {
            s.parse::<u64>()
                .ok()
                .filter(|s| *s < 60)
                .and_then(|s| m.checked_mul(60)?.checked_add(s))
        }),
        _ => None,
    }
    .ok_or_else(|| invalid("BENCH time is seconds or minutes:seconds"))?;
    i64::try_from(
        seconds
            .checked_mul(1000)
            .ok_or_else(|| invalid("bench time"))?,
    )
    .map_err(|_| invalid("bench time"))
}
pub(crate) fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let text = ctx.args.raw().trim().to_owned();
    perform(ctx, &text).map_err(|e| CmdError::msg(e.to_string()))
}
fn perform(ctx: &mut Ctx<'_>, text: &str) -> Result<(), BulkError> {
    if text.len() > 4096 {
        return Err(invalid("BENCH request bound"));
    }
    let (connection, conn) = current(ctx)?;
    let account = connection.account;
    let now = world::now_ms();
    let service = service(ctx.w)?;
    let mut runtime = service.0.lock().map_err(|_| BulkError::Busy)?;
    let (verb, tail) = text
        .split_once(char::is_whitespace)
        .map_or((text, ""), |(a, b)| (a, b.trim()));
    if verb.eq_ignore_ascii_case("load") {
        if runtime.entries.contains_key(&account) {
            return Err(BulkError::Busy);
        }
        if runtime.entries.len() >= runtime.cfg.bench_sessions {
            return Err(BulkError::Retention);
        }
        let parts = tail.split_whitespace().collect::<Vec<_>>();
        let (reference, pin_run, bytes, load) = if let [kind, id, callsign] = parts.as_slice() {
            if !kind.eq_ignore_ascii_case("match") || callsign.len() > 32 {
                return Err(invalid("BENCH LOAD MATCH <match> <callsign>"));
            }
            let id = MatchId(id.parse().map_err(|_| invalid("match number"))?);
            let checkout = mind_tape::checkout(ctx.w, id, connection.character)
                .map_err(|_| BulkError::Missing)?;
            (
                ReferenceId::Match(id),
                None,
                checkout.encoded_bytes(),
                Load::Work(Box::new(Work::History {
                    checkout,
                    id,
                    actor: connection.character,
                    callsign: (*callsign).into(),
                })),
            )
        } else if let [pair] = parts.as_slice() {
            let (run, game) = pair
                .split_once('/')
                .ok_or_else(|| invalid("BENCH LOAD <run>/<game>"))?;
            let run = RunId(run.parse().map_err(|_| invalid("run number"))?);
            let game = game
                .parse::<usize>()
                .ok()
                .and_then(|g| g.checked_sub(1))
                .ok_or_else(|| invalid("one-based game number"))?;
            let owner = ctx
                .w
                .module_state::<BulkStore>()
                .ok_or(BulkError::Missing)?;
            completed_proving(owner, account, connection.character, run, game)?;
            let artifacts = ctx
                .w
                .module_state::<Artifacts>()
                .ok_or(BulkError::Missing)?;
            let start = artifacts
                .get(Key {
                    account,
                    run,
                    part: Part::Start,
                })?
                .clone();
            let result = artifacts
                .get(Key {
                    account,
                    run,
                    part: Part::Result,
                })?
                .clone();
            let bytes = start.artifact.bytes.saturating_add(result.artifact.bytes);
            (
                ReferenceId::Proving(run),
                Some(run),
                bytes,
                Load::Read(Box::new(Input {
                    start: Some(Read::new(start)?),
                    result: Some(Read::new(result)?),
                    on_result: false,
                    correlation: 0,
                    game,
                })),
            )
        } else {
            return Err(invalid(
                "BENCH LOAD <run>/<game> or BENCH LOAD MATCH <match> <callsign>",
            ));
        };
        if runtime
            .entries
            .values()
            .map(|e| e.bytes)
            .sum::<u64>()
            .saturating_add(bytes)
            > runtime.cfg.bench_state_bytes
        {
            return Err(BulkError::Retention);
        }
        let id = ticket(&mut runtime)?;
        world::hold_preparation(ctx.w, account)?;
        if let Some(run) = pin_run
            && let Err(e) = pin(ctx, connection, run, true)
        {
            world::release_preparation(ctx.w, account);
            return Err(e);
        }
        let mut entry = Entry {
            connection,
            conn,
            touched: now,
            bytes,
            pin: pin_run,
            source_root: None,
            pending_root: None,
            source_revision: None,
            pending_revision: None,
            pending_replace: false,
            fork_bytes: 0,
            reference,
            session: None,
            phase: Phase::Stopped,
            plan: None,
            picture: None,
            closing: false,
            watches: Vec::new(),
        };
        let result = match load {
            Load::Work(work) => runtime
                .worker
                .submit(bench_prepare::Request {
                    ticket: id,
                    account,
                    work: *work,
                })
                .map(|()| {
                    entry.phase = Phase::Preparing(id);
                })
                .map_err(|(e, _)| e),
            Load::Read(mut input) => {
                read_next(ctx, &mut input).map(|()| entry.phase = Phase::Reading(input))
            }
        };
        runtime.entries.insert(account, entry);
        if let Err(error) = result {
            let _ = clean(ctx, &mut runtime, account);
            return Err(error);
        }
        say(
            ctx,
            conn,
            "Bench loading; reference verification is queued.",
        );
    } else {
        let mut row = take_current(&mut runtime.entries, connection)?;
        if row.closing
            || !authorized(ctx.w, &row)
            || now.saturating_sub(row.touched) >= runtime.cfg.bench_idle_ms as i64
        {
            runtime.entries.insert(account, row);
            return Err(BulkError::Missing);
        }
        row.touched = now;
        if row.session.is_some()
            && let Err(error) = runtime.benches.selected(connection, now)
        {
            runtime.entries.insert(account, row);
            return Err(error);
        }
        let result = (|| {
            match verb.to_ascii_lowercase().as_str() {
                "close" => {
                    if !tail.is_empty() {
                        return Err(invalid("BENCH CLOSE"));
                    }
                    row.closing = true;
                }
                "pic" => {
                    if !tail.is_empty() {
                        return Err(invalid("BENCH PIC"));
                    }
                    emit(ctx, &row)?;
                    if let Some(p) = &row.picture {
                        say(
                            ctx,
                            conn,
                            format!("Bench picture at {} ms, event {}.", p.at_ms, p.event),
                        );
                    }
                }
                "trace" => {
                    let count = if tail.is_empty() {
                        8
                    } else {
                        tail.parse::<usize>()
                            .ok()
                            .filter(|n| (1..=64).contains(n))
                            .ok_or_else(|| invalid("BENCH TRACE [1..64]"))?
                    };
                    let p = row.picture.as_ref().ok_or(BulkError::Busy)?;
                    if p.memory.is_none() {
                        return Err(BulkError::Missing);
                    }
                    for trace in p.traces.iter().rev().take(count).rev() {
                        say(
                            ctx,
                            conn,
                            format!("Think {}: {:?}.", trace.think_seq, trace.outcome),
                        );
                    }
                    emit(ctx, &row)?;
                }
                "watch" => {
                    if tail.eq_ignore_ascii_case("off") {
                        row.watches.clear();
                    } else if tail.is_empty() {
                        for w in &row.watches {
                            say(ctx, conn, format!("WATCH {w}"));
                        }
                    } else {
                        if row.picture.as_ref().is_none_or(|p| p.memory.is_none()) {
                            return Err(BulkError::Missing);
                        }
                        if row.watches.len() >= 8 || tail.len() > super::eval::MAX_EXPRESSION {
                            return Err(invalid("eight bounded WATCH expressions maximum"));
                        }
                        row.watches.push(tail.into());
                    }
                    say(
                        ctx,
                        conn,
                        format!("Bench has {} watch expressions.", row.watches.len()),
                    );
                }
                "eval" => {
                    if tail.is_empty() || tail.len() > super::eval::MAX_EXPRESSION {
                        return Err(invalid("BENCH EVAL <expression>"));
                    }
                    evaluate(ctx, &mut runtime, &mut row, vec![tail.into()])?;
                    say(ctx, conn, "Bench evaluation queued.");
                }
                "act" => {
                    let words = tail.split_whitespace().collect::<Vec<_>>();
                    let [kind, callsign] = words.as_slice() else {
                        return Err(invalid("BENCH ACT AS <callsign>"));
                    };
                    if !kind.eq_ignore_ascii_case("as") || callsign.len() > 32 {
                        return Err(invalid("BENCH ACT AS <callsign>"));
                    }
                    if !matches!(row.reference, ReferenceId::Proving(_)) {
                        return Err(BulkError::Missing);
                    }
                    if !matches!(row.phase, Phase::Idle(_)) {
                        return Err(BulkError::Busy);
                    }
                    let id = ticket(&mut runtime)?;
                    let ready = take_idle(&mut row)?;
                    if let Err(error) = world::hold_preparation(ctx.w, account) {
                        row.phase = Phase::Idle(ready);
                        return Err(error);
                    }
                    match runtime.worker.submit(bench_prepare::Request {
                        ticket: id,
                        account,
                        work: Work::Select {
                            ready,
                            callsign: (*callsign).into(),
                        },
                    }) {
                        Ok(()) => {
                            row.phase = Phase::Preparing(id);
                            say(ctx, conn, "Bench selection queued.");
                        }
                        Err((error, request)) => {
                            if let Work::Select { ready, .. } = request.work {
                                row.phase = Phase::Idle(ready);
                            }
                            world::release_preparation(ctx.w, account);
                            return Err(error);
                        }
                    }
                }
                "diff" | "swap" | "possess" | "release" | "do" | "reference" => {
                    if !matches!(row.phase, Phase::Idle(_)) {
                        return Err(BulkError::Busy);
                    }
                    let id = ticket(&mut runtime)?;
                    let command = verb.to_ascii_lowercase();
                    let (edit, revision, replace) = match command.as_str() {
                        "diff" | "swap" => {
                            let words = world::tokens(tail)?;
                            let [reference] = words.as_slice() else {
                                return Err(invalid("BENCH DIFF|SWAP <owned mind/profile>"));
                            };
                            if command == "diff" && reference.eq_ignore_ascii_case("off") {
                                (super::experiment::Edit::DiffOff, None, true)
                            } else {
                                let roll = ctx
                                    .w
                                    .module_state::<mush_roll::RollStore>()
                                    .ok_or(BulkError::Missing)?;
                                let sources = ctx
                                    .w
                                    .module_state::<crate::store::TacsimStore>()
                                    .ok_or(BulkError::Missing)?;
                                let candidate = super::scenarios::Candidate::resolve(
                                    roll,
                                    sources,
                                    connection.character,
                                    reference,
                                )
                                .map_err(|_| BulkError::Missing)?;
                                candidate
                                    .revalidate(roll, sources, connection.character)
                                    .map_err(|_| BulkError::Missing)?;
                                let revision = candidate.reference();
                                (
                                    if command == "diff" {
                                        super::experiment::Edit::Diff(candidate)
                                    } else {
                                        super::experiment::Edit::Swap {
                                            candidate,
                                            serial: id,
                                        }
                                    },
                                    revision,
                                    true,
                                )
                            }
                        }
                        "possess" if tail.is_empty() => {
                            (super::experiment::Edit::Possess { serial: id }, None, false)
                        }
                        "release" if tail.is_empty() => {
                            (super::experiment::Edit::Release, None, false)
                        }
                        "reference" if tail.is_empty() => {
                            (super::experiment::Edit::Reference, None, true)
                        }
                        "do" if !tail.is_empty() => {
                            (super::experiment::Edit::Do(tail.into()), None, false)
                        }
                        _ => {
                            return Err(invalid(
                                "BENCH POSSESS|RELEASE|REFERENCE, or BENCH DO <station command>",
                            ));
                        }
                    };
                    let reserve =
                        if command == "swap" || (row.fork_bytes == 0 && command == "possess") {
                            super::branch::RESERVATION_BYTES
                        } else {
                            0
                        };
                    if runtime
                        .entries
                        .values()
                        .map(|r| r.bytes)
                        .sum::<u64>()
                        .saturating_add(row.bytes)
                        .saturating_add(reserve)
                        > runtime.cfg.bench_state_bytes
                    {
                        return Err(BulkError::Retention);
                    }
                    let ready = take_idle(&mut row)?;
                    if let Err(error) = world::hold_preparation(ctx.w, account) {
                        row.phase = Phase::Idle(ready);
                        return Err(error);
                    }
                    let root = revision.map(|revision| {
                        (
                            super::bench_roots::key(account, connection.lease, id),
                            revision,
                        )
                    });
                    if let Err(error) = super::bench_roots::change(ctx, root.clone(), Vec::new()) {
                        world::release_preparation(ctx.w, account);
                        row.phase = Phase::Idle(ready);
                        return Err(error);
                    }
                    row.pending_root = root.map(|(key, _)| key);
                    row.pending_revision = revision;
                    row.pending_replace = replace;
                    row.bytes += reserve;
                    row.fork_bytes += reserve;
                    match runtime.worker.submit(bench_prepare::Request {
                        ticket: id,
                        account,
                        work: Work::Edit {
                            ready,
                            edit: Box::new(edit),
                        },
                    }) {
                        Ok(()) => {
                            row.phase = Phase::Preparing(id);
                            say(ctx, conn, "Bench experiment preparation queued.");
                        }
                        Err((error, request)) => {
                            if let Work::Edit { ready, .. } = request.work {
                                row.phase = Phase::Idle(ready);
                            }
                            world::release_preparation(ctx.w, account);
                            row.bytes -= reserve;
                            row.fork_bytes -= reserve;
                            super::bench_roots::change(
                                ctx,
                                None,
                                row.pending_root.iter().cloned().collect(),
                            )?;
                            row.pending_root = None;
                            row.pending_revision = None;
                            row.pending_replace = false;
                            return Err(error);
                        }
                    }
                }
                "step" | "back" | "run" => {
                    let p = row.picture.as_ref().ok_or(BulkError::Busy)?;
                    let plan = row.plan.clone().ok_or(BulkError::Busy)?;
                    let command = if verb.eq_ignore_ascii_case("run") {
                        let t = tail
                            .strip_prefix("TO ")
                            .or_else(|| tail.strip_prefix("to "))
                            .ok_or_else(|| invalid("BENCH RUN TO <minutes:seconds>"))?;
                        ReplayCommand::RunTo(time(t)?)
                    } else {
                        let n = if tail.is_empty() {
                            1
                        } else {
                            tail.parse::<u32>()
                                .ok()
                                .filter(|n| (1..=1000).contains(n))
                                .ok_or_else(|| invalid("BENCH STEP|BACK [1..1000]"))?
                        };
                        let delta = i64::from(n) * i64::from(plan.native_substep_ms);
                        if verb.eq_ignore_ascii_case("back") {
                            ReplayCommand::BackTo(p.at_ms.saturating_sub(delta).max(0))
                        } else {
                            ReplayCommand::RunTo(
                                p.at_ms.saturating_add(delta).min(i64::from(plan.limit_ms)),
                            )
                        }
                    };
                    let ready = take_idle(&mut row)?;
                    match world::admit_bench(
                        ctx,
                        account,
                        connection.character,
                        plan,
                        ready,
                        command,
                    ) {
                        Ok(run) => {
                            row.phase = Phase::Running(run);
                            say(ctx, conn, format!("Bench work {} queued.", run.0));
                        }
                        Err((error, ready)) => {
                            row.phase = Phase::Idle(ready);
                            return Err(error);
                        }
                    }
                }
                _ => {
                    return Err(invalid(
                        "BENCH LOAD, STEP, RUN TO, BACK, PIC, TRACE, WATCH, EVAL, ACT AS, DIFF, SWAP, POSSESS, RELEASE, DO, REFERENCE or CLOSE",
                    ));
                }
            }
            Ok(())
        })();
        let closing = row.closing;
        runtime.entries.insert(account, row);
        result?;
        if closing {
            close(ctx, &mut runtime, account)?;
            say(
                ctx,
                conn,
                "Bench closed; in-flight work must settle before its pins release.",
            );
        }
    }
    drop(runtime);
    world::drain(ctx);
    Ok(())
}

/// Multiplex this before the existing ReportRead; unknown correlations belong
/// to the older reader. Every continuation remains bound to its initiating conn.
pub(crate) fn on_read(ctx: &mut Ctx<'_>, correlation: u64, result: StoreReadResult) -> bool {
    let Ok(service) = service(ctx.w) else {
        return false;
    };
    let Ok(mut runtime) = service.0.lock() else {
        return false;
    };
    let found = runtime.entries.iter().find_map(|(a, e)| match &e.phase {
        Phase::Reading(i) if i.correlation == correlation => Some(*a),
        _ => None,
    });
    let Some((account, mut row)) =
        found.and_then(|account| runtime.entries.remove(&account).map(|row| (account, row)))
    else {
        return false;
    };
    let next = (|| {
        if row.closing || !authorized(ctx.w, &row) || ctx.actor != row.connection.character {
            return Err(BulkError::Missing);
        }
        let Phase::Reading(input) = &mut row.phase else {
            return Err(BulkError::Stale);
        };
        let StoreReadResult::Docs { docs, absent } = result else {
            return Err(invalid("bench artifact read failed"));
        };
        if !absent.is_empty() {
            return Err(invalid("bench artifact chunk missing"));
        }
        let reader = if input.on_result {
            input.result.as_mut()
        } else {
            input.start.as_mut()
        }
        .ok_or(BulkError::Missing)?;
        if !reader.accept(&docs)? {
            return read_next(ctx, input);
        }
        if !input.on_result {
            input.on_result = true;
            return read_next(ctx, input);
        }
        let ReferenceId::Proving(run) = row.reference else {
            return Err(BulkError::Missing);
        };
        let work = Work::Proving {
            start: Box::new(input.start.take().ok_or(BulkError::Missing)?),
            result: Box::new(input.result.take().ok_or(BulkError::Missing)?),
            run,
            game: input.game,
            actor: row.connection.character,
        };
        let id = ticket(&mut runtime)?;
        runtime
            .worker
            .submit(bench_prepare::Request {
                ticket: id,
                account,
                work,
            })
            .map_err(|(e, _)| e)?;
        row.phase = Phase::Preparing(id);
        Ok(())
    })();
    if let Err(error) = next {
        if authorized(ctx.w, &row) {
            say(ctx, row.conn, format!("Bench load refused: {error}"));
        }
        row.closing = true;
        row.phase = Phase::Stopped;
    }
    let closing = row.closing;
    runtime.entries.insert(account, row);
    if closing {
        let _ = clean(ctx, &mut runtime, account);
    }
    true
}

pub(crate) fn drain(ctx: &mut Ctx<'_>) {
    let Ok(service) = service(ctx.w) else {
        return;
    };
    let Ok(mut runtime) = service.0.lock() else {
        return;
    };
    let now = world::now_ms();
    if let Some(done) = runtime.worker.poll() {
        let found = runtime.entries.iter().find_map(|(a, e)| match e.phase {
            Phase::Preparing(t) | Phase::Evaluating { ticket: t, .. } if t == done.ticket => {
                Some(*a)
            }
            _ => None,
        });
        if let Some((account, mut row)) =
            found.and_then(|account| runtime.entries.remove(&account).map(|row| (account, row)))
        {
            world::release_preparation(ctx.w, account);
            let valid = !row.closing
                && authorized(ctx.w, &row)
                && now.saturating_sub(row.touched) < runtime.cfg.bench_idle_ms as i64;
            let result = (|| {
                if !valid {
                    return Err(BulkError::Missing);
                }
                match done.result? {
                    ResultValue::Open(opened) => {
                        let prepared = runtime.benches.prepare_open(
                            row.connection,
                            opened.reference,
                            opened.selected,
                            now,
                        )?;
                        runtime.benches.check_open(&prepared)?;
                        row.session = Some(runtime.benches.apply_open(prepared));
                        row.bytes = opened.plan.start.bytes;
                        row.picture = opened.ready.bench_picture();
                        row.plan = Some(opened.plan);
                        row.phase = Phase::Idle(opened.ready);
                        say(ctx, row.conn, "Bench ready; verified reference at 0 ms.");
                        emit(ctx, &row)?;
                    }
                    ResultValue::Selected(ready, error) => {
                        let picture = ready.bench_picture().ok_or(BulkError::Missing)?;
                        row.picture = Some(picture.clone());
                        if !source_access(ctx.w, &row) {
                            return Err(BulkError::Missing);
                        }
                        if let Some(error) = error {
                            say(ctx, row.conn, error.to_string());
                        } else {
                            runtime.benches.act_as(row.connection, picture.unit, now)?;
                            say(
                                ctx,
                                row.conn,
                                "Bench selected private hull; reference timeline unchanged.",
                            );
                        }
                        row.picture = Some(picture);
                        row.phase = Phase::Idle(ready);
                        emit(ctx, &row)?;
                    }
                    ResultValue::Edited(ready, error) => {
                        if let Some(error) = error {
                            super::bench_roots::change(
                                ctx,
                                None,
                                row.pending_root.iter().cloned().collect(),
                            )?;
                            row.pending_root = None;
                            row.pending_revision = None;
                            say(ctx, row.conn, error.to_string());
                        } else {
                            if row.pending_replace {
                                super::bench_roots::change(
                                    ctx,
                                    None,
                                    row.source_root.iter().cloned().collect(),
                                )?;
                                row.source_root = row.pending_root.take();
                                row.source_revision = row.pending_revision.take();
                            }
                            if let Some(plan) = row.plan.as_mut() {
                                plan.unit_count =
                                    ready.bench_unit_count().ok_or(BulkError::Missing)?;
                            }
                        }
                        row.pending_replace = false;
                        row.picture = ready.bench_picture();
                        let retained = if row.picture.as_ref().is_some_and(|p| {
                            matches!(p.timeline, super::branch::Timeline::Fork { .. })
                        }) {
                            super::branch::RESERVATION_BYTES
                        } else {
                            0
                        };
                        row.bytes = row
                            .bytes
                            .saturating_sub(row.fork_bytes.saturating_sub(retained));
                        row.fork_bytes = retained;

                        if let Some(p) = &row.picture {
                            say(
                                ctx,
                                row.conn,
                                match &p.timeline {
                                    super::branch::Timeline::Reference => {
                                        "Bench reference selected; reference timeline unchanged."
                                            .to_owned()
                                    }
                                    super::branch::Timeline::Fork {
                                        serial,
                                        parent_event,
                                        ..
                                    } => format!(
                                        "Bench fork {serial}, from reference event {parent_event}; {}.",
                                        if p.possessed { "possessed" } else { "released" }
                                    ),
                                },
                            );
                        }
                        row.phase = Phase::Idle(ready);
                        emit(ctx, &row)?;
                    }
                    ResultValue::Eval(results) => {
                        let Phase::Evaluating { ready, .. } =
                            std::mem::replace(&mut row.phase, Phase::Stopped)
                        else {
                            return Err(BulkError::Stale);
                        };
                        row.phase = Phase::Idle(ready);
                        for result in results {
                            match result {
                                Ok(answer) => say(
                                    ctx,
                                    row.conn,
                                    format!(
                                        "EVAL {} ({} ops + {} fees).",
                                        answer.text, answer.operations, answer.fees
                                    ),
                                ),
                                Err(error) => say(ctx, row.conn, error.to_string()),
                            }
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                if valid {
                    say(ctx, row.conn, format!("Bench refused: {error}"));
                }
                row.closing = true;
                row.phase = Phase::Stopped;
            }
            runtime.entries.insert(account, row);
        }
    }
    for done in world::take_bench_completions(ctx.w) {
        let Some(mut row) = runtime.entries.remove(&done.account) else {
            continue;
        };
        if !matches!(row.phase,Phase::Running(run)if run==done.run) {
            runtime.entries.insert(done.account, row);
            continue;
        }
        if !row.closing
            && authorized(ctx.w, &row)
            && now.saturating_sub(row.touched) < runtime.cfg.bench_idle_ms as i64
            && done.status == Status::Complete
        {
            if let Some(ready) = done.ready {
                row.picture = ready.bench_picture();
                row.phase = Phase::Idle(ready);
                say(
                    ctx,
                    row.conn,
                    if row
                        .picture
                        .as_ref()
                        .is_some_and(|p| matches!(p.timeline, super::branch::Timeline::Fork { .. }))
                    {
                        "Bench fork step complete; reference timeline unchanged."
                    } else {
                        "Bench step complete; reference hashes matched."
                    },
                );
                let _ = emit(ctx, &row);
                if !row.watches.is_empty() {
                    let expressions = row.watches.clone();
                    if let Err(error) = evaluate(ctx, &mut runtime, &mut row, expressions) {
                        say(ctx, row.conn, format!("WATCH evaluation deferred: {error}"));
                    }
                }
            } else {
                row.closing = true;
                row.phase = Phase::Stopped;
            }
        } else {
            if authorized(ctx.w, &row) {
                say(
                    ctx,
                    row.conn,
                    format!("Bench work stopped: {:?}.", done.status),
                );
            }
            row.closing = true;
            row.phase = Phase::Stopped;
        }
        runtime.entries.insert(done.account, row);
    }
    let closed = runtime
        .entries
        .iter()
        .filter(|(_, e)| {
            e.closing
                || !authorized(ctx.w, e)
                || now.saturating_sub(e.touched) >= runtime.cfg.bench_idle_ms as i64
        })
        .map(|(a, _)| *a)
        .collect::<Vec<_>>();
    for account in closed {
        if let Err(error) = close(ctx, &mut runtime, account) {
            tracing::error!(%error,"bench cleanup retained pending owner repair");
        }
    }
}

pub(crate) struct Reads;
impl mush_core::StoreReadCompletion for Reads {
    fn on_read(&self, ctx: &mut Ctx<'_>, correlation: u64, result: StoreReadResult) {
        let is_bench = service(ctx.w)
            .ok()
            .and_then(|service| {
                service.0.lock().ok().map(|runtime| {
                    runtime
                        .entries
                        .values()
                        .any(|e| matches!(&e.phase,Phase::Reading(i)if i.correlation==correlation))
                })
            })
            .unwrap_or(false);
        if is_bench {
            on_read(ctx, correlation, result);
        } else {
            mush_core::StoreReadCompletion::on_read(
                &crate::match_::report::ReportRead,
                ctx,
                correlation,
                result,
            );
        }
    }
}
#[cfg(test)]
mod tests {
    mod lineage;
    use super::*;
    use mush_core::ObjId;

    struct CleanupAccounts(AccountId);
    impl mush_core::AccountIdentity for CleanupAccounts {
        fn account_of(&self, actor: ObjId) -> Option<AccountId> {
            (actor == mush_core::GOD).then_some(self.0)
        }
    }
    struct CleanupModule {
        owner: BulkStore,
        service: BenchService,
        account: AccountId,
        root: mush_roll::ReferenceKey,
    }
    impl mush_core::GameModule for CleanupModule {
        fn name(&self) -> &'static str {
            crate::store::NS
        }
        fn register(
            &mut self,
            reg: &mut mush_core::Registrar<'_>,
        ) -> Result<(), mush_core::ModError> {
            // Use the production module's persistence lane for these handlers.
            reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
            reg.state(self.owner.clone());
            reg.state(Artifacts::default());
            reg.state(self.service.clone());
            reg.state(mush_sim::AccountOracle::new(Arc::new(CleanupAccounts(
                self.account,
            ))));
            reg.command(
                mush_core::CommandSpec {
                    name: "bench".into(),
                    shape: mush_core::ArgShape::Raw,
                    ..Default::default()
                },
                Arc::new(command),
            )?;
            let root = self.root.clone();
            reg.command(
                mush_core::CommandSpec {
                    name: "publish-then-refuse".into(),
                    shape: mush_core::ArgShape::Raw,
                    ..Default::default()
                },
                Arc::new(move |ctx: &mut Ctx<'_>| {
                    let (connection, _) = current(ctx).map_err(|e| CmdError::msg(e.to_string()))?;
                    pin(ctx, connection, RunId(1), true)
                        .map_err(|e| CmdError::msg(e.to_string()))?;
                    super::super::bench_roots::change(ctx, None, vec![root.clone()])
                        .map_err(|e| CmdError::msg(e.to_string()))?;
                    // Model a later read/queue refusal through the real command
                    // boundary, which ordinarily truncates all deferred writes.
                    Err(CmdError::msg("later queue refused"))
                }),
            )?;
            Ok(())
        }
    }
    fn cleanup_rig(
        quarantined: bool,
        pinned: bool,
    ) -> (
        mush_core::Engine,
        BenchService,
        mush_roll::ReferenceKey,
        Vec<(smol_str::SmolStr, serde_json::Value)>,
    ) {
        use super::super::model::{Account, Artifact, Opponent, Run, Scenario};
        use mush_core::{Effects, Engine, GOD, Inbound, ModuleBootData, ModuleWrite, Outbox};
        let account = AccountId::mint("bench-cleanup", 1).unwrap();
        let root = super::super::bench_roots::key(account, 1, 1);
        let mut roll = mush_roll::RollStore::default();
        let working = mush_roll::WorkingCopy {
            source: "fn think(view, card) {}".into(),
            envelope: mush_roll::Envelope {
                schema: serde_json::json!({"sections": []}),
                presets: serde_json::json!({}),
                profiles: serde_json::json!({"Default": {}}),
                rules: serde_json::json!({}),
                declared_class: "I".into(),
                mind_abi: crate::minds::MIND_ABI.into(),
            },
        };
        let mut operations = Vec::new();
        let change = roll.prepare_create(GOD, "retained", working).unwrap();
        roll.publish_with(change, |ops| operations.extend_from_slice(ops))
            .unwrap();
        let saved = roll
            .prepare_save(
                GOD,
                "retained",
                roll.mind(GOD, "retained").unwrap().base(),
                "",
                None,
                &crate::minds::authoring::RevisionRules,
                &crate::TacsimStore::default(),
            )
            .unwrap();
        let (change, _, revision) = saved.into_parts();
        roll.publish_with(change, |ops| operations.extend_from_slice(ops))
            .unwrap();
        let change = roll
            .prepare_reference(crate::store::NS, root.clone(), vec![revision])
            .unwrap();
        roll.publish_with(change, |ops| operations.extend_from_slice(ops))
            .unwrap();
        let mut documents = BTreeMap::new();
        for operation in operations {
            if let ModuleWrite::JsonSet { key, value } = operation.op {
                documents.insert(key, value);
            } else {
                panic!("fixture creates immutable records only");
            }
        }
        let documents = documents.into_iter().collect::<Vec<_>>();
        let mut record = Account::empty(account, 0);
        record.request_watermark = 1;
        record.next_run = 2;
        record.runs.insert(
            RunId(1),
            Run {
                conduct_job: None,
                registry_job: None,
                registry: None,
                conduct: None,
                id: RunId(1),
                character: GOD,
                nonce: 1,
                created_ms: 0,
                finished_ms: Some(1),
                plan: Plan {
                    registry: None,
                    conduct: None,
                    kind: WorkKind::Proving,
                    scenario: Scenario::Duel,
                    opponent: Opponent::Mirror,
                    per_side: 1,
                    unit_count: 2,
                    games: 10,
                    seeds: (0..5).collect(),
                    limit_ms: 60_000,
                    native_substep_ms: 250,
                    start: Artifact {
                        sha256: [1; 32],
                        bytes: 100,
                    },
                    result_reservation_bytes: 1_000,
                },
                status: Status::Complete,
                charged_unit_ms: 0,
                next_sequence: 1,
                flight: None,
                result: Some(Artifact {
                    sha256: [2; 32],
                    bytes: 100,
                }),
                pins: u8::from(pinned),
            },
        );
        let mut owner = BulkStore::new(BulkConfig::default()).unwrap();
        owner
            .load(account, &serde_json::to_vec(&record).unwrap())
            .unwrap();
        if quarantined {
            assert_eq!(
                owner.load(account, b"unreadable"),
                Err(BulkError::Quarantined)
            );
        }
        let (wake, _rx) = tokio::sync::mpsc::channel(1);
        let service = BenchService::new(BulkConfig::default(), wake).unwrap();
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![
                Box::new(CleanupModule {
                    owner,
                    service: service.clone(),
                    account,
                    root: root.clone(),
                }),
                Box::new(mush_roll::RollModule::with_writers([crate::store::NS])),
            ],
        )
        .unwrap();
        engine
            .modules_loaded(
                std::collections::HashMap::from([(
                    mush_roll::NS.into(),
                    ModuleBootData {
                        kv: documents.clone(),
                        ..Default::default()
                    },
                )]),
                &mut Outbox::new(),
                &mut Effects::new(),
            )
            .unwrap();
        engine.step(
            Inbound::ConnUp {
                conn: 7,
                actor: GOD,
                caps_structured: true,
                from_linkdead: false,
            },
            &mut Outbox::new(),
            &mut Effects::new(),
        );
        let (_, lease) = engine.world.connection_binding(7).unwrap();
        let mut row = running();
        row.connection = Connection {
            account,
            character: GOD,
            lease,
        };
        row.phase = Phase::Stopped;
        row.touched = world::now_ms();
        row.source_root = Some(root.clone());
        service.0.lock().unwrap().entries.insert(account, row);
        assert!(engine.take_module_ops().is_empty());
        (engine, service, root, documents)
    }
    fn cleanup_command(engine: &mut mush_core::Engine, line: &str) -> mush_core::Outbox {
        let mut out = mush_core::Outbox::new();
        engine.step(
            mush_core::Inbound::Cmd {
                conn: 7,
                actor: mush_core::GOD,
                line: line.into(),
            },
            &mut out,
            &mut mush_core::Effects::new(),
        );
        out
    }
    fn replay_cleanup_writes(
        initial: Vec<(smol_str::SmolStr, serde_json::Value)>,
        operations: &[mush_core::ModuleOp],
    ) -> (mush_roll::RollStore, Option<BulkStore>) {
        let mut roll = initial.into_iter().collect::<BTreeMap<_, _>>();
        let mut quota = None;
        for operation in operations {
            match (&*operation.ns, &operation.op) {
                (mush_roll::NS, mush_core::ModuleWrite::JsonSet { key, value }) => {
                    roll.insert(key.clone(), value.clone());
                }
                (mush_roll::NS, mush_core::ModuleWrite::Del { key }) => {
                    roll.remove(key);
                }
                (crate::store::NS, mush_core::ModuleWrite::JsonSet { value, .. }) => {
                    let record: super::super::model::Account =
                        serde_json::from_value(value.clone()).unwrap();
                    let mut owner = BulkStore::new(BulkConfig::default()).unwrap();
                    owner
                        .load(record.id, &serde_json::to_vec(value).unwrap())
                        .unwrap();
                    quota = Some(owner);
                }
                _ => panic!("unexpected cleanup write"),
            }
        }
        (
            mush_roll::RollStore::load(&mush_core::ModuleBootData {
                kv: roll.into_iter().collect(),
                ..Default::default()
            })
            .unwrap(),
            quota,
        )
    }
    #[test]
    fn command_cleanup_refusal_preserves_roots_and_session_before_first_owner_write() {
        let (mut engine, service, root, _) = cleanup_rig(true, true);
        let out = cleanup_command(&mut engine, "BENCH CLOSE");
        assert!(format!("{:?}", out.events()).contains("unreadable"));
        assert!(engine.take_module_ops().is_empty());
        assert_eq!(
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .unwrap()
                .reference_keys(crate::store::NS, mush_roll::ReferenceKind::OpenSeal),
            vec![root.clone()]
        );
        let runtime = service.0.lock().unwrap();
        let row = runtime.entries.values().next().unwrap();
        assert_eq!(row.source_root.as_ref(), Some(&root));
        assert_eq!(row.pin, Some(RunId(1)));
        assert!(row.closing);
        assert!(matches!(row.phase, Phase::Stopped));
    }
    #[test]
    fn command_cleanup_publishes_root_release_and_unpin_together() {
        let (mut engine, service, _, documents) = cleanup_rig(false, true);
        let account = service
            .0
            .lock()
            .unwrap()
            .entries
            .keys()
            .next()
            .copied()
            .unwrap();
        cleanup_command(&mut engine, "BENCH CLOSE");
        assert!(service.0.lock().unwrap().entries.is_empty());
        assert!(
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .unwrap()
                .reference_keys(crate::store::NS, mush_roll::ReferenceKind::OpenSeal)
                .is_empty()
        );
        assert_eq!(
            engine
                .world
                .module_state::<BulkStore>()
                .unwrap()
                .run(account, mush_core::GOD, RunId(1))
                .unwrap()
                .pins,
            0
        );
        let operations = engine.take_module_ops();
        assert_eq!(
            operations.len(),
            2,
            "one root deletion and one account image"
        );
        let (roll, quota) = replay_cleanup_writes(documents, &operations);
        assert!(
            roll.reference_keys(crate::store::NS, mush_roll::ReferenceKind::OpenSeal)
                .is_empty()
        );
        assert_eq!(
            quota
                .unwrap()
                .run(account, mush_core::GOD, RunId(1))
                .unwrap()
                .pins,
            0
        );
    }
    #[test]
    fn later_command_refusal_keeps_each_complete_owner_publication_durable() {
        let (mut engine, service, _, documents) = cleanup_rig(false, false);
        let account = service
            .0
            .lock()
            .unwrap()
            .entries
            .keys()
            .next()
            .copied()
            .unwrap();
        let out = cleanup_command(&mut engine, "publish-then-refuse");
        assert!(format!("{:?}", out.events()).contains("later queue refused"));
        let operations = engine.take_module_ops();
        assert_eq!(
            operations.len(),
            2,
            "both complete publications survive run_handler truncation"
        );
        let (roll, quota) = replay_cleanup_writes(documents, &operations);
        assert_eq!(
            roll.reference_keys(crate::store::NS, mush_roll::ReferenceKind::OpenSeal),
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .unwrap()
                .reference_keys(crate::store::NS, mush_roll::ReferenceKind::OpenSeal)
        );
        assert_eq!(
            quota
                .unwrap()
                .run(account, mush_core::GOD, RunId(1))
                .unwrap()
                .pins,
            1
        );
        assert_eq!(
            engine
                .world
                .module_state::<BulkStore>()
                .unwrap()
                .run(account, mush_core::GOD, RunId(1))
                .unwrap()
                .pins,
            1
        );
    }
    fn connection() -> Connection {
        Connection {
            account: AccountId::mint("bench", 1).unwrap(),
            character: ObjId(2),
            lease: 9,
        }
    }
    fn running() -> Entry {
        Entry {
            connection: connection(),
            conn: 7,
            touched: 0,
            bytes: 100,
            pin: Some(RunId(1)),
            source_root: None,
            pending_root: None,
            source_revision: None,
            pending_revision: None,
            pending_replace: false,
            fork_bytes: 0,
            reference: ReferenceId::Proving(RunId(1)),
            session: None,
            phase: Phase::Running(RunId(3)),
            plan: None,
            picture: None,
            closing: false,
            watches: Vec::new(),
        }
    }
    #[test]
    fn another_character_or_reused_connection_cannot_remove_private_session() {
        let c = connection();
        let mut entries = BTreeMap::from([(c.account, running())]);
        assert!(
            take_current(
                &mut entries,
                Connection {
                    character: ObjId(3),
                    ..c
                }
            )
            .is_err()
        );
        assert!(take_current(&mut entries, Connection { lease: 10, ..c }).is_err());
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[&c.account].phase,
            Phase::Running(RunId(3))
        ));
    }
    #[test]
    fn busy_request_preserves_in_flight_state_and_pin() {
        let mut row = running();
        assert!(matches!(take_idle(&mut row), Err(BulkError::Busy)));
        assert!(matches!(row.phase, Phase::Running(RunId(3))));
        assert_eq!(row.pin, Some(RunId(1)));
    }
    #[test]
    fn proving_blob_reads_require_completed_owned_proving_and_in_range_game() {
        use super::super::model::{Account, Artifact, Opponent, Run, Scenario};
        let account = AccountId::mint("bench-read-owner", 1).expect("account");
        let other = AccountId::mint("bench-read-other", 1).expect("other account");
        let actor = ObjId(42);
        let id = RunId(1);
        for kind in [WorkKind::Proving, WorkKind::Bench] {
            for status in [
                Status::Queued,
                Status::Running,
                Status::CancelRequested,
                Status::Complete,
                Status::Cancelled,
                Status::Interrupted,
                Status::Failed,
            ] {
                let mut record = Account::empty(account, 0);
                record.request_watermark = 1;
                record.next_run = 2;
                record.active = (!status.terminal()).then_some(id);
                let games = if kind == WorkKind::Proving { 10 } else { 1 };
                record.runs.insert(
                    id,
                    Run {
                        conduct_job: None,
                        registry_job: None,
                        registry: None,
                        conduct: None,
                        id,
                        character: actor,
                        nonce: 1,
                        created_ms: 0,
                        finished_ms: status.terminal().then_some(1),
                        plan: Plan {
                            registry: None,
                            conduct: None,
                            kind,
                            scenario: Scenario::Duel,
                            opponent: Opponent::Mirror,
                            per_side: 1,
                            unit_count: 2,
                            games,
                            seeds: if kind == WorkKind::Proving {
                                (0..5).collect()
                            } else {
                                vec![0]
                            },
                            limit_ms: 60_000,
                            native_substep_ms: 250,
                            start: Artifact {
                                sha256: [1; 32],
                                bytes: 100,
                            },
                            result_reservation_bytes: 1_000,
                        },
                        status,
                        charged_unit_ms: 0,
                        next_sequence: 1,
                        flight: None,
                        result: (status == Status::Complete).then_some(Artifact {
                            sha256: [2; 32],
                            bytes: 100,
                        }),
                        pins: 0,
                    },
                );
                let mut owner = BulkStore::new(BulkConfig::default()).expect("owner");
                owner
                    .load(account, &serde_json::to_vec(&record).expect("record"))
                    .expect("valid durable state");
                let permitted = status == Status::Complete && kind == WorkKind::Proving;
                for game in [0, usize::from(games - 1)] {
                    assert_eq!(
                        completed_proving(&owner, account, actor, id, game).is_ok(),
                        permitted,
                        "{kind:?}/{status:?}/game {game}"
                    );
                }
                assert!(completed_proving(&owner, account, actor, id, usize::from(games)).is_err());
                assert!(completed_proving(&owner, account, ObjId(43), id, 0).is_err());
                assert!(completed_proving(&owner, other, actor, id, 0).is_err());
                assert!(completed_proving(&owner, account, actor, RunId(2), 0).is_err());
            }
        }
    }

    #[test]
    fn times_are_bounded_without_rounding_or_overflow() {
        assert_eq!(time("1:30"), Ok(90_000));
        assert_eq!(time("3"), Ok(3_000));
        assert!(time("0:60").is_err());
        assert!(time("-1").is_err());
        assert!(time("18446744073709551615:59").is_err());
    }
}
