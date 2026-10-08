//! **The process-wide think pool** (17b cut M4.1; doc 17 §6 "Execution
//! policy", §9 "The pool"; D-UM-22, D-UM-36).
//!
//! A sim task collects its thinks at their milliseconds (`Arena::collect_think`)
//! and, after it has published the pulse, hands the pulse's outstanding
//! thinks here as ONE batch ([`MindPool::submit`]); it awaits the batch's
//! answers ([`BatchHandle::wait`]) at the head of its next pulse — and past a
//! snapshot's due check, before the encode — and writes them
//! (`Arena::deliver_answers`) before it steps again. Think time is off the
//! wake's critical path and outside the wake lens; a saturated pool is seen
//! as late wakes (`pool_wait_us`), never as a different result, because a
//! job is a pure function of its frozen inputs (`CollectedThink`) and the
//! write is at one sim-time moment whichever thread ran the job.
//!
//! **Pure `std`.** `[tacsim.minds].workers` (stock 2) threads
//! (`tacsim-mind-N`), one queue of batches under one mutex with one condvar,
//! served **first-in first-out per batch**: the workers take the head batch's
//! jobs in job order, and a later batch's first job is not taken until the
//! head's last has been; each batch answers its jobs in job order. The
//! arena library never sees this module (it is under `minds/`, outside the
//! `arena/` gate), and the task's only runtime touch is the `spawn_blocking`
//! it awaits a handle through (`arena_task`).
//!
//! **A runner panic voids the match, never a fault** (17b §3's M4 row,
//! finding (11)): the sandbox catches its own panics and labels them
//! (`runner.rs`' label rule); a panic that escapes a runner on a worker is
//! caught only to keep the worker's thread, and the batch answers
//! [`BatchFailed`] naming the job — the task ends the match `Void` on it, as
//! the unwind boundary voids a panic inside `Arena::step` (where the inline
//! path runs the same runner).
//!
//! **One per process** ([`MindPool::shared`]): sized once, from the first
//! task's `[tacsim.minds].workers`; every later task shares it. A private
//! pool ([`MindPool::new`]) is a test's — the gate runs real tasks at one,
//! two and four workers in one process and holds them to the library.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};

use mush_core::ObjId;

use crate::arena::mind::{CollectedThink, ThinkAnswer};

/// **A worker's runner panicked** (17b cut M4.1; this module's head): the
/// job it was on. The batch is lost whole — the task voids the match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a mind runner panicked on the think pool at #{}'s think {think_seq}", unit.0)]
pub struct BatchFailed {
    /// The hull whose think it was.
    pub unit: ObjId,
    /// The think's sequence number.
    pub think_seq: u64,
}

/// **The process-wide think pool** (this module's head).
#[derive(Debug)]
pub struct MindPool {
    shared: Arc<Shared>,
    /// The worker threads actually running.
    workers: usize,
}

/// What the workers and the pool share: the queue and its signal.
#[derive(Debug)]
struct Shared {
    queue: Mutex<Queue>,
    /// Signalled on every submit and on close.
    work: Condvar,
}

/// The queue of batches, head first, and how far into the head the workers
/// are.
#[derive(Debug, Default)]
struct Queue {
    batches: VecDeque<Arc<Batch>>,
    /// The next job of the head batch to take.
    next: usize,
    /// Set by the pool's drop: the workers finish what is queued and exit.
    closed: bool,
}

/// One submitted batch: its jobs, and its progress.
#[derive(Debug)]
struct Batch {
    jobs: Vec<CollectedThink>,
    progress: Mutex<Progress>,
    /// Signalled when the last job is done.
    finished: Condvar,
}

/// A batch's progress: each job's answer in job order, how many are done,
/// and the first panic.
#[derive(Debug, Default)]
struct Progress {
    answers: Vec<Option<ThinkAnswer>>,
    done: usize,
    failed: Option<BatchFailed>,
}

/// **A submitted batch, to await** ([`MindPool::submit`]).
#[derive(Debug)]
pub struct BatchHandle {
    batch: Arc<Batch>,
}

impl BatchHandle {
    /// **Block until every job is done**, answering them in job order — or
    /// the first panic ([`BatchFailed`]), with the batch lost whole. The sim
    /// task calls this through `tokio::task::spawn_blocking`; a test may call
    /// it directly.
    ///
    /// # Errors
    ///
    /// [`BatchFailed`] when a worker's runner panicked on one of the jobs.
    pub fn wait(self) -> Result<Vec<ThinkAnswer>, BatchFailed> {
        let mut progress = self
            .batch
            .progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while progress.done < self.batch.jobs.len() {
            progress = self
                .batch
                .finished
                .wait(progress)
                .unwrap_or_else(PoisonError::into_inner);
        }
        if let Some(failed) = progress.failed {
            return Err(failed);
        }
        Ok(progress
            .answers
            .iter_mut()
            .filter_map(Option::take)
            .collect())
    }

    /// Whether every job is done (a test's read; the task only waits).
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.batch
            .progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .done
            >= self.batch.jobs.len()
    }

    /// How many jobs the batch holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.batch.jobs.len()
    }

    /// Whether the batch holds no job.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.batch.jobs.is_empty()
    }
}

/// The process's one pool ([`MindPool::shared`]).
static SHARED: OnceLock<Arc<MindPool>> = OnceLock::new();

impl MindPool {
    /// **A private pool of `workers` threads** (at least one): a test's, or
    /// what [`MindPool::shared`] builds once. A thread the host refuses to
    /// spawn is logged and the pool runs on the rest; with none at all a
    /// batch is run on the submitting thread ([`MindPool::submit`]), so a
    /// handle is always answered.
    #[must_use]
    pub fn new(workers: u32) -> Arc<MindPool> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            work: Condvar::new(),
        });
        let mut spawned = 0_usize;
        for n in 0..workers.max(1) {
            let mine = Arc::clone(&shared);
            match std::thread::Builder::new()
                .name(format!("tacsim-mind-{n}"))
                .spawn(move || work(&mine))
            {
                Ok(_) => spawned = spawned.saturating_add(1),
                Err(error) => {
                    tracing::error!(worker = n, %error, "a think pool worker could not be spawned");
                }
            }
        }
        Arc::new(MindPool {
            shared,
            workers: spawned,
        })
    }

    /// **The process's one pool**, built on the first call with that call's
    /// `workers` (`[tacsim.minds].workers` as the first task read it) and
    /// shared by every later one, whatever size it names — sized ONCE per
    /// process, as the module head says.
    #[must_use]
    pub fn shared(workers: u32) -> Arc<MindPool> {
        let pool = SHARED.get_or_init(|| MindPool::new(workers));
        if pool.workers() != usize::try_from(workers.max(1)).unwrap_or(usize::MAX) {
            tracing::debug!(
                asked = workers,
                running = pool.workers(),
                "the process's think pool is already sized; the later size is ignored"
            );
        }
        Arc::clone(pool)
    }

    /// How many worker threads run.
    #[must_use]
    pub fn workers(&self) -> usize {
        self.workers
    }

    /// How many batches wait with a job not yet taken (a test's read).
    #[must_use]
    pub fn queued(&self) -> usize {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .batches
            .len()
    }

    /// **Submit one batch** — a pulse's outstanding thinks in collection
    /// order — and answer the handle to await. An empty batch is done at
    /// once and never queued. With no worker thread at all the batch is run
    /// here, on the caller's thread, under the same panic rule.
    #[must_use]
    pub fn submit(&self, jobs: Vec<CollectedThink>) -> BatchHandle {
        let len = jobs.len();
        let batch = Arc::new(Batch {
            jobs,
            progress: Mutex::new(Progress {
                answers: (0..len).map(|_| None).collect(),
                done: 0,
                failed: None,
            }),
            finished: Condvar::new(),
        });
        if len == 0 {
            return BatchHandle { batch };
        }
        if self.workers == 0 {
            for at in 0..len {
                run_job(&batch, at);
            }
            return BatchHandle { batch };
        }
        {
            let mut queue = self
                .shared
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            queue.batches.push_back(Arc::clone(&batch));
        }
        self.shared.work.notify_all();
        BatchHandle { batch }
    }
}

impl Drop for MindPool {
    /// Close the queue: the workers finish every batch still queued, then
    /// exit. A handle on a queued batch is still answered.
    fn drop(&mut self) {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed = true;
        self.shared.work.notify_all();
    }
}

/// **One worker's life**: take the head batch's next job, run it, write its
/// answer; wait when nothing is queued; exit when the queue is closed and
/// empty.
fn work(shared: &Shared) {
    loop {
        let taken = {
            let mut queue = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if let Some(batch) = queue.batches.front().map(Arc::clone) {
                    let at = queue.next;
                    queue.next = queue.next.saturating_add(1);
                    if queue.next >= batch.jobs.len() {
                        queue.batches.pop_front();
                        queue.next = 0;
                    }
                    break Some((batch, at));
                }
                if queue.closed {
                    break None;
                }
                queue = shared
                    .work
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let Some((batch, at)) = taken else {
            return;
        };
        run_job(&batch, at);
    }
}

/// **Run job `at` of `batch` on this thread and write its answer** — or,
/// when the runner panics, the batch's failure (the first one kept). The
/// panic is caught here only so the thread lives on; the batch is lost.
fn run_job(batch: &Batch, at: usize) {
    let Some(job) = batch.jobs.get(at) else {
        return;
    };
    let answered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.think()));
    let mut progress = batch
        .progress
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match answered {
        Ok(answer) => {
            if let Some(slot) = progress.answers.get_mut(at) {
                *slot = Some(answer);
            }
        }
        Err(_) => {
            progress.failed.get_or_insert(BatchFailed {
                unit: job.unit,
                think_seq: job.think_seq,
            });
        }
    }
    progress.done = progress.done.saturating_add(1);
    if progress.done >= batch.jobs.len() {
        batch.finished.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use mush_core::ObjId;

    use super::*;
    use crate::arena::mind::{MindRunner, ThinkJob, ThinkResult, run_batch_inline};
    use crate::arena::motor::MotorCounters;
    use crate::arena::state::{Arena, Controller, Team};
    use crate::arena::test_support::{arena_of, pose, tank};

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);

    /// Red a mind, blue a machine, sealed under the fixture terms.
    fn minded() -> Arena {
        let mut red = tank(RED, Team::Red, pose(1_500, 4_500));
        red.controller = Controller::Mind;
        red.reseal(&crate::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
        arena_of(vec![red, tank(BLUE, Team::Blue, pose(6_500, 4_500))])
    }

    /// **`n` collected thinks of red**, think 0 collected through
    /// `Arena::collect_think` on `runner` and the rest its clones at the
    /// next sequence numbers — distinct jobs on one frozen view.
    fn jobs(runner: Arc<dyn MindRunner>, n: u64) -> Vec<CollectedThink> {
        let mut arena = minded();
        arena.mind_runner = runner;
        let mut counters = MotorCounters::default();
        arena.collect_think(RED, 0, 1_000, &mut counters);
        let first = arena
            .collected
            .first()
            .cloned()
            .expect("the think was collected");
        (0..n)
            .map(|think_seq| CollectedThink {
                think_seq,
                ..first.clone()
            })
            .collect()
    }

    /// A runner that records the order it was called in, as
    /// `(unit, think_seq)`, and answers `think_seq` as its one memory byte.
    #[derive(Debug, Default)]
    struct Ordered {
        calls: Mutex<Vec<(ObjId, u64)>>,
    }

    impl MindRunner for Ordered {
        fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push((job.unit, job.think_seq));
            }
            ThinkResult {
                intents: Vec::new(),
                memory: vec![u8::try_from(job.think_seq).unwrap_or(u8::MAX)],
                fault: None,
                trace: Vec::new(),
            }
        }
    }

    /// A runner that panics on `at` and answers every other think.
    #[derive(Debug)]
    struct Panicking {
        at: u64,
    }

    impl MindRunner for Panicking {
        fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
            if job.think_seq == self.at {
                std::panic::resume_unwind(Box::new("a runner panicked (test)"));
            }
            ThinkResult {
                intents: Vec::new(),
                memory: job.memory.to_vec(),
                fault: None,
                trace: Vec::new(),
            }
        }
    }

    /// **17b cut M4.1: a batch answers its jobs in job order on a pool of
    /// one, two and four workers, equal to the batch run inline**; an empty
    /// batch is done at once and queued nowhere.
    #[test]
    fn m4_a_batch_answers_in_job_order_on_any_pool_size() {
        let runner: Arc<dyn MindRunner> = Arc::new(Ordered::default());
        let thinks = jobs(runner, 7);
        let inline = run_batch_inline(&thinks);
        assert_eq!(inline.len(), 7);
        for workers in [1_u32, 2, 4] {
            let pool = MindPool::new(workers);
            assert_eq!(pool.workers(), usize::try_from(workers).unwrap_or(0));
            let handle = pool.submit(thinks.clone());
            assert_eq!(handle.len(), 7);
            let answers = handle.wait().expect("the batch answers");
            assert_eq!(answers, inline, "job order on {workers} workers");
            assert_eq!(
                answers
                    .iter()
                    .map(|answer| answer.result.memory.clone())
                    .collect::<Vec<_>>(),
                (0..7_u8).map(|n| vec![n]).collect::<Vec<_>>()
            );
            let empty = pool.submit(Vec::new());
            assert!(empty.is_empty());
            assert!(empty.is_done(), "an empty batch is done at once");
            assert_eq!(pool.queued(), 0, "and queued nowhere");
            assert_eq!(empty.wait(), Ok(Vec::new()));
        }
    }

    /// **17b cut M4.1: batches are served first-in first-out** — on one
    /// worker, every job of the first batch runs before any of the second,
    /// in job order, and both handles answer.
    #[test]
    fn m4_batches_are_served_first_in_first_out() {
        let ordered = Arc::new(Ordered::default());
        let runner: Arc<dyn MindRunner> = ordered.clone();
        let first = jobs(Arc::clone(&runner), 3);
        let second: Vec<CollectedThink> = jobs(runner, 2)
            .into_iter()
            .map(|think| CollectedThink {
                think_seq: think.think_seq + 10,
                ..think
            })
            .collect();
        let pool = MindPool::new(1);
        let a = pool.submit(first);
        let b = pool.submit(second);
        let answered_b = b.wait().expect("the second batch answers");
        let answered_a = a.wait().expect("the first batch answers");
        assert_eq!(answered_a.len(), 3);
        assert_eq!(answered_b.len(), 2);
        let calls = ordered
            .calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default();
        assert_eq!(
            calls,
            vec![(RED, 0), (RED, 1), (RED, 2), (RED, 10), (RED, 11)],
            "the first batch whole, then the second, each in job order"
        );
    }

    /// **17b cut M4.1: a runner's panic fails the batch by name and keeps the
    /// worker** (doc 17 §24: a panic voids, never a fault): the batch answers
    /// `BatchFailed` for the job that panicked, and the same pool answers the
    /// next batch whole.
    #[test]
    fn m4_a_panicking_runner_fails_the_batch_and_the_worker_lives_on() {
        let runner: Arc<dyn MindRunner> = Arc::new(Panicking { at: 2 });
        let thinks = jobs(Arc::clone(&runner), 4);
        let pool = MindPool::new(1);
        let failed = pool.submit(thinks.clone()).wait();
        assert_eq!(
            failed,
            Err(BatchFailed {
                unit: RED,
                think_seq: 2
            })
        );
        assert_eq!(
            failed.map_err(|error| error.to_string()),
            Err("a mind runner panicked on the think pool at #4's think 2".to_owned())
        );
        let clean: Vec<CollectedThink> = thinks
            .into_iter()
            .filter(|think| think.think_seq != 2)
            .collect();
        let answers = pool.submit(clean).wait().expect("the worker lived on");
        assert_eq!(answers.len(), 3);
    }

    /// **17b cut M4.1: a pool dropped with a batch queued still answers it**,
    /// and the process's shared pool is one `Arc` however often it is asked
    /// for.
    #[test]
    fn m4_a_dropped_pool_answers_what_it_queued_and_the_shared_pool_is_one() {
        let runner: Arc<dyn MindRunner> = Arc::new(Ordered::default());
        let thinks = jobs(runner, 5);
        let pool = MindPool::new(2);
        let handle = pool.submit(thinks);
        drop(pool);
        assert_eq!(
            handle.wait().map(|answers| answers.len()),
            Ok(5),
            "the workers drain the queue before they exit"
        );
        let shared = MindPool::shared(2);
        assert!(
            Arc::ptr_eq(&shared, &MindPool::shared(4)),
            "sized once per process"
        );
        assert!(shared.workers() >= 1);
    }
}
