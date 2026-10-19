//! Dedicated worker threads; one bounded substep per dispatch. The world
//! commits each Receipt before releasing the next prepaid lease. A round-robin
//! ready queue prevents a bench RUN from monopolizing a worker.
use super::{
    BulkError, invalid,
    model::{ArtifactBytes, BulkConfig, RunId, Status, WorkKind},
    owner::{Lease, Receipt},
    registry_budget::{RegistryCase, RegistryEnding, RegistryProgress},
};
use mush_core::AccountId;
use std::collections::{BTreeMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::JoinHandle;

/// Implemented only by the frozen proving factory and verified replay adapter
/// in this module tree. No command can substitute an arbitrary source adapter,
/// World callback, shared MindPool or live Arena.
pub(in crate::minds) trait Machine: Send {
    fn registry_case(&self) -> Option<RegistryCase> {
        None
    }
    fn registry_progress(&self) -> Option<RegistryProgress> {
        None
    }
    fn begin_quantum(&mut self) {}
    fn native_substep_ms(&self) -> u32;
    fn next_simulation_ms(&self) -> u64;
    fn step(&mut self, maximum_sim_ms: u64) -> Result<Turn, BulkError>;
    fn bench_start(&mut self, _: super::replay::ReplayCommand) -> Result<(), BulkError> {
        Err(BulkError::Missing)
    }
    fn bench_edit(&mut self, _: super::experiment::Edit) -> Result<(), BulkError> {
        Err(BulkError::Missing)
    }
    fn bench_unit_count(&self) -> Option<u16> {
        None
    }
    fn bench_act_as(&mut self, _callsign: &str) -> Result<(), BulkError> {
        Err(BulkError::Missing)
    }
    fn bench_abort_start(&mut self) {}
    fn bench_picture(&self) -> Option<Arc<super::replay::Picture>> {
        None
    }
}
pub(in crate::minds) struct Turn {
    pub simulation_ms: u64,
    pub result: Option<ArtifactBytes>,
    pub terminal: Option<Status>,
}
/// A prepaid quantum contains several native steps, with an independent phase
/// bound so a stream of non-time tape events cannot monopolize the worker.
/// No simulation is split differently from the captured physics cadence.
fn run_quantum(machine: &mut dyn Machine, maximum: u64) -> Result<Turn, BulkError> {
    machine.begin_quantum();
    let mut used = 0_u64;
    let mut native_steps = 0_u64;
    for _ in 0..32 {
        let next = machine.next_simulation_ms();
        if next > maximum.saturating_sub(used) {
            if used == 0 {
                return Err(BulkError::Quota);
            }
            break;
        }
        let turn = machine.step(maximum - used)?;
        if turn.simulation_ms > maximum - used {
            return Err(invalid("machine exceeded prepaid quantum"));
        }
        used += turn.simulation_ms;
        if turn.simulation_ms > 0 {
            native_steps += 1;
        }
        if turn.terminal.is_some() {
            return Ok(Turn {
                simulation_ms: used,
                ..turn
            });
        }
        if turn.result.is_some() {
            return Err(invalid("nonterminal artifact"));
        }
        // A new case may have a different roster. Its first unit-millisecond
        // requires a new persisted lease; never spend the previous case's rate.
        if machine
            .registry_progress()
            .is_some_and(|p| p.ending != RegistryEnding::Continue)
            || native_steps == super::model::LEASE_STEPS
        {
            break;
        }
    }
    Ok(Turn {
        simulation_ms: used,
        result: None,
        terminal: None,
    })
}
pub(crate) struct Ready {
    account: AccountId,
    run: RunId,
    start: [u8; 32],
    machine: Box<dyn Machine>,
    registry_binding: Option<super::registry_tickets::Binding>,
}
impl Ready {
    pub(super) fn new(
        account: AccountId,
        run: RunId,
        start: [u8; 32],
        machine: Box<dyn Machine>,
    ) -> Self {
        Self {
            account,
            run,
            start,
            machine,
            registry_binding: None,
        }
    }
    pub(super) fn registry(
        binding: super::registry_tickets::Binding,
        start: [u8; 32],
        machine: Box<dyn Machine>,
    ) -> Self {
        Self {
            account: binding.account,
            run: binding.run,
            start,
            machine,
            registry_binding: Some(binding),
        }
    }
    /// Grant one millisecond for non-time phases, then refund their known zero.
    pub fn next_grant_ms(&self) -> u64 {
        u64::from(self.machine.native_substep_ms()) * super::model::LEASE_STEPS
    }
    pub(super) fn stage_bench(
        &mut self,
        command: super::replay::ReplayCommand,
    ) -> Result<(), BulkError> {
        self.machine.bench_start(command)
    }
    /// Only preparation worker adapters call this projection-only operation.
    pub(super) fn bench_edit(&mut self, edit: super::experiment::Edit) -> Result<(), BulkError> {
        self.machine.bench_edit(edit)
    }
    pub(super) fn bench_unit_count(&self) -> Option<u16> {
        self.machine.bench_unit_count()
    }
    pub(super) fn bench_act_as(&mut self, callsign: &str) -> Result<(), BulkError> {
        self.machine.bench_act_as(callsign)
    }
    pub(super) fn abort_bench(&mut self) {
        self.machine.bench_abort_start();
    }
    pub(super) fn bind_bench_run(&mut self, run: RunId) {
        self.run = run;
    }
    /// Current actor/session/source authorization is rechecked before egress.
    pub fn bench_picture(&self) -> Option<Arc<super::replay::Picture>> {
        self.machine.bench_picture()
    }
    pub fn account(&self) -> AccountId {
        self.account
    }
    pub fn run(&self) -> RunId {
        self.run
    }
}
struct Task {
    lease: Lease,
    ready: Ready,
}
pub(crate) struct Completion {
    pub(super) registry_settlement:
        Option<Result<super::registry_tickets::PreparedSettlement, BulkError>>,
    pub artifact: Option<ArtifactBytes>,
    pub receipt: Receipt,
    pub ready: Option<Ready>,
}
/// At most one ready continuation per account. Taking it removes ownership;
/// the worker completion must be persisted and put back at the tail.
pub(crate) struct FairQueue {
    limit: usize,
    order: VecDeque<AccountId>,
    ready: BTreeMap<AccountId, Ready>,
}
impl FairQueue {
    pub(super) fn is_empty(&self) -> bool {
        self.order.is_empty() && self.ready.is_empty()
    }
    pub fn new(cfg: &BulkConfig) -> Result<Self, BulkError> {
        cfg.validate()?;
        Ok(Self {
            limit: cfg.queued + cfg.proving_workers,
            order: VecDeque::new(),
            ready: BTreeMap::new(),
        })
    }
    pub fn push(&mut self, ready: Ready) -> Result<(), (BulkError, Ready)> {
        if self.ready.contains_key(&ready.account) {
            return Err((BulkError::Busy, ready));
        }
        if self.ready.len() >= self.limit {
            return Err((BulkError::QueueFull, ready));
        }
        self.order.push_back(ready.account);
        self.ready.insert(ready.account, ready);
        Ok(())
    }
    pub fn pop(&mut self) -> Option<Ready> {
        let id = self.order.pop_front()?;
        self.ready.remove(&id)
    }
}
pub(crate) struct Executor {
    submitted: AtomicBool,
    sender: Option<SyncSender<Task>>,
    completed: Option<Receiver<Completion>>,
    threads: Vec<JoinHandle<()>>,
}
impl Executor {
    pub fn with_wake(
        cfg: &BulkConfig,
        wake: Option<tokio::sync::mpsc::Sender<crate::arena_task::ArenaOut>>,
    ) -> Result<Self, BulkError> {
        cfg.validate()?;
        // Queue + executing + unread completions are admission-owned. Neither
        // channel can introduce another retained job or source copy.
        let (tx, rx) = mpsc::sync_channel::<Task>(cfg.proving_workers);
        let (done_tx, done_rx) = mpsc::sync_channel::<Completion>(cfg.proving_workers);
        let rx = Arc::new(Mutex::new(rx));
        let mut threads = Vec::new();
        for index in 0..cfg.proving_workers {
            let rx = rx.clone();
            let done = done_tx.clone();
            let wake = wake.clone();
            let thread = std::thread::Builder::new()
                .name(format!("mind-bulk-{index}"))
                .spawn(move || {
                    loop {
                        let task = match rx.lock() {
                            Ok(receiver) => receiver.recv(),
                            Err(_) => return,
                        };
                        let Ok(mut task) = task else { return };
                        let maximum = task.lease.maximum_sim_ms();
                        let worker_day = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()
                            .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
                            .map(super::model::utc_day);
                        // A lease queued across UTC midnight cannot spend yesterday's
                        // reservation. Return it unused; the owner grants today's next.
                        let result = if worker_day != Some(task.lease.utc_day()) {
                            Ok(Ok(Turn {
                                simulation_ms: 0,
                                result: None,
                                terminal: None,
                            }))
                        } else {
                            catch_unwind(AssertUnwindSafe(|| {
                                run_quantum(&mut *task.ready.machine, maximum)
                            }))
                        };
                        #[cfg(test)]
                        if task.lease.kind() == WorkKind::Registry {
                            match &result {
                                Ok(Err(error)) => {
                                    eprintln!("M7_REGISTRY_WORKER_FAILURE error={error}")
                                }
                                Err(_) => eprintln!("M7_REGISTRY_WORKER_FAILURE panic=true"),
                                _ => {}
                            }
                        }
                        let (outcome, unknown_work) = match result {
                            Ok(Ok(turn)) if turn.simulation_ms <= maximum => (turn, false),
                            // A crash/error has unknown actual work. Charge this bounded
                            // grant and stop; retry cannot obtain free unaccounted CPU.
                            _ => (
                                Turn {
                                    simulation_ms: maximum,
                                    result: None,
                                    terminal: Some(Status::Failed),
                                },
                                true,
                            ),
                        };
                        let keep = task.lease.kind() == WorkKind::Registry
                            || outcome.terminal.is_none()
                            || (outcome.terminal == Some(Status::Complete)
                                && task.lease.kind() == WorkKind::Bench);
                        let receipt = Receipt {
                            registry: if unknown_work {
                                None
                            } else if worker_day != Some(task.lease.utc_day()) {
                                task.lease.unchanged_registry()
                            } else {
                                task.ready.machine.registry_progress()
                            },
                            actual_unit_ms: outcome.simulation_ms * task.lease.units(),
                            lease: task.lease,
                            result: outcome
                                .result
                                .as_ref()
                                .map(|value| value.descriptor().clone()),
                            terminal: outcome.terminal,
                        };
                        let (registry_settlement, artifact) =
                            if let Some(binding) = task.ready.registry_binding {
                                match super::registry_tickets::PreparedSettlement::from_worker(
                                    binding,
                                    receipt.clone(),
                                    outcome.result,
                                ) {
                                    Ok(prepared) => (Some(Ok(prepared)), None),
                                    Err((error, artifact)) => (Some(Err(error)), artifact),
                                }
                            } else {
                                (None, outcome.result)
                            };
                        let completion = Completion {
                            registry_settlement,
                            artifact,
                            receipt,
                            ready: keep.then_some(task.ready),
                        };
                        if done.send(completion).is_err() {
                            return;
                        }
                        if let Some(wake) = &wake
                            && matches!(
                                wake.try_send(crate::arena_task::ArenaOut::MindBulkReady),
                                Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
                            )
                        {
                            return;
                        }
                    }
                })
                .map_err(|e| invalid(format!("cannot start proving worker: {e}")))?;
            threads.push(thread);
        }
        Ok(Self {
            submitted: AtomicBool::new(false),
            sender: Some(tx),
            completed: Some(done_rx),
            threads,
        })
    }
    /// A caller must have published Lease from the matching account owner.
    /// Failed submission returns both values: do not manufacture a receipt or
    /// lose the paid quantum; enqueue this exact pair when a worker frees up.
    // Queue refusal must return the exact paid lease and private machine so
    // the world can refund/requeue without releasing ownership prematurely.
    #[allow(clippy::result_large_err)]
    pub fn submit(&self, lease: Lease, ready: Ready) -> Result<(), (BulkError, Lease, Ready)> {
        // A submission attempt permanently closes cold-boot recovery, even if
        // this attempt is refused. Never infer a stopped worker from an empty
        // queue or absence of a completion after a timeout.
        self.submitted.store(true, Ordering::Release);
        if lease.account() != ready.account
            || lease.run() != ready.run
            || lease.start() != ready.start
            || match (
                lease.registry(),
                ready.machine.registry_case(),
                ready.machine.registry_progress(),
            ) {
                (None, None, None) => false,
                (Some((cursor, case)), Some(actual), Some(progress)) => {
                    *case != actual || *cursor != progress.cursor
                }
                _ => true,
            }
        {
            return Err((BulkError::Missing, lease, ready));
        }
        let Some(sender) = &self.sender else {
            return Err((BulkError::QueueFull, lease, ready));
        };
        match sender.try_send(Task { lease, ready }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(t) | TrySendError::Disconnected(t)) => {
                Err((BulkError::QueueFull, t.lease, t.ready))
            }
        }
    }
    pub fn poll(&self) -> Option<Completion> {
        self.completed.as_ref()?.try_recv().ok()
    }
    pub(super) fn never_submitted(&self) -> bool {
        !self.submitted.load(Ordering::Acquire)
    }
}
impl Drop for Executor {
    fn drop(&mut self) {
        self.sender.take();
        self.completed.take();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Counter {
        phases: usize,
        limit: Option<usize>,
        timed: bool,
    }
    impl Machine for Counter {
        fn native_substep_ms(&self) -> u32 {
            250
        }
        fn next_simulation_ms(&self) -> u64 {
            if self.timed { 250 } else { 0 }
        }
        fn step(&mut self, _: u64) -> Result<Turn, BulkError> {
            self.phases += 1;
            Ok(Turn {
                simulation_ms: self.next_simulation_ms(),
                result: None,
                terminal: (self.limit == Some(self.phases)).then_some(Status::Cancelled),
            })
        }
    }
    #[test]
    fn prepaid_quantum_yields_after_eight_native_steps_and_reports_unused_work() {
        let mut machine = Counter {
            phases: 0,
            limit: None,
            timed: true,
        };
        let turn = run_quantum(&mut machine, 2_000).expect("bounded work");
        assert_eq!((machine.phases, turn.simulation_ms), (8, 2_000));
        assert_eq!(turn.terminal, None);
        machine.limit = Some(10);
        let turn = run_quantum(&mut machine, 2_000).expect("early cancel");
        assert_eq!(turn.simulation_ms, 500);
        assert_eq!(turn.terminal, Some(Status::Cancelled));
    }
    #[test]
    fn non_time_phases_have_a_separate_fairness_bound() {
        let mut machine = Counter {
            phases: 0,
            limit: None,
            timed: false,
        };
        let turn = run_quantum(&mut machine, 2_000).expect("bounded phases");
        assert_eq!((machine.phases, turn.simulation_ms), (32, 0));
    }
    struct Idle;
    impl Machine for Idle {
        fn native_substep_ms(&self) -> u32 {
            250
        }
        fn next_simulation_ms(&self) -> u64 {
            0
        }
        fn step(&mut self, _: u64) -> Result<Turn, BulkError> {
            Ok(Turn {
                simulation_ms: 0,
                result: None,
                terminal: None,
            })
        }
    }
    fn ready(name: &str) -> Ready {
        Ready::new(
            AccountId::mint(name, 1).expect("account"),
            RunId(1),
            [1; 32],
            Box::new(Idle),
        )
    }
    #[test]
    fn continuations_yield_to_other_accounts() {
        let mut q = FairQueue::new(&BulkConfig::default()).expect("queue");
        let a = ready("a");
        let aid = a.account;
        let b = ready("b");
        let bid = b.account;
        assert!(q.push(a).is_ok());
        assert!(q.push(b).is_ok());
        let first = q.pop().expect("first");
        assert_eq!(first.account, aid);
        assert!(q.push(first).is_ok());
        assert_eq!(q.pop().expect("other").account, bid);
        assert_eq!(q.pop().expect("continued").account, aid)
    }
    #[test]
    fn duplicate_account_and_queue_bound() {
        let cfg = BulkConfig {
            queued: 1,
            ..Default::default()
        };
        let mut q = FairQueue::new(&cfg).expect("queue");
        assert!(q.push(ready("a")).is_ok());
        assert!(matches!(q.push(ready("a")), Err((BulkError::Busy, _))));
        assert!(q.push(ready("b")).is_ok());
        assert!(matches!(q.push(ready("c")), Err((BulkError::QueueFull, _))))
    }
}
