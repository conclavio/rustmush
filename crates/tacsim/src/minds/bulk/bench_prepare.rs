//! Bounded preparation and expression worker. Compilation, source verification,
//! replay opening and EVAL never run in a world command or live mind pool.
use super::{
    BulkError,
    artifacts::Read,
    bench::ReferenceId,
    eval::{EvalAnswer, EvalPicture},
    invalid,
    model::{BulkConfig, RunId},
    replay::{Opened, ReferenceLease},
    tape_cache::TapeCache,
};
use crate::{arena_task::MatchId, match_::mind_tape::TapeCheckout};
use mush_core::{AccountId, ObjId};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
pub(super) enum Work {
    Edit {
        ready: super::executor::Ready,
        edit: Box<super::experiment::Edit>,
    },
    Select {
        ready: super::executor::Ready,
        callsign: String,
    },
    History {
        checkout: TapeCheckout,
        id: MatchId,
        actor: ObjId,
        callsign: String,
    },
    Proving {
        start: Box<Read>,
        result: Box<Read>,
        run: RunId,
        game: usize,
        actor: ObjId,
    },
    Eval {
        picture: EvalPicture,
        expressions: Vec<String>,
        cfg: BulkConfig,
    },
}
pub(super) enum ResultValue {
    Edited(super::executor::Ready, Option<BulkError>),
    Selected(super::executor::Ready, Option<BulkError>),
    Open(Opened),
    Eval(Vec<Result<EvalAnswer, BulkError>>),
}
pub(super) struct Request {
    pub ticket: u64,
    pub account: AccountId,
    pub work: Work,
}
pub(super) struct Completion {
    pub ticket: u64,
    pub result: Result<ResultValue, BulkError>,
}
pub(super) struct Worker {
    sender: Option<SyncSender<Request>>,
    done: Option<Receiver<Completion>>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}
impl Worker {
    pub fn new(
        limit: usize,
        wake: tokio::sync::mpsc::Sender<crate::arena_task::ArenaOut>,
    ) -> Result<Self, BulkError> {
        let (sender, requests) = mpsc::sync_channel::<Request>(limit);
        let (finished, done) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let halted = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("mind-bench-prepare".into())
            .spawn(move || {
                let cache = TapeCache::default();
                while let Ok(request) = requests.recv() {
                    if halted.load(Ordering::Acquire) {
                        return;
                    }
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        match request.work {
                            Work::Edit { mut ready, edit } => {
                                let error = ready.bench_edit(*edit).err();
                                Ok(ResultValue::Edited(ready, error))
                            }
                            Work::Select {
                                mut ready,
                                callsign,
                            } => {
                                let error = ready.bench_act_as(&callsign).err();
                                Ok(ResultValue::Selected(ready, error))
                            }
                            Work::History {
                                checkout,
                                id,
                                actor,
                                callsign,
                            } => {
                                let lease = cache.verify(checkout).map_err(|_| {
                                    invalid("historical reference verification failed")
                                })?;
                                super::replay::open_reference(
                                    request.account,
                                    ReferenceLease::historical(lease, actor),
                                    ReferenceId::Match(id),
                                    Some(&callsign),
                                )
                                .map(ResultValue::Open)
                            }
                            Work::Proving {
                                start,
                                result,
                                run,
                                game,
                                actor,
                            } => {
                                let lease = ReferenceLease::proving(*start, *result, game, actor)?;
                                super::replay::open_reference(
                                    request.account,
                                    lease,
                                    ReferenceId::Proving(run),
                                    None,
                                )
                                .map(ResultValue::Open)
                            }
                            Work::Eval {
                                picture,
                                expressions,
                                cfg,
                            } => {
                                if expressions.len() > 8 {
                                    return Err(invalid("expression count"));
                                }
                                Ok(ResultValue::Eval(
                                    expressions
                                        .iter()
                                        .map(|text| super::eval::evaluate(&cfg, text, &picture))
                                        .collect(),
                                ))
                            }
                        }
                    }))
                    .unwrap_or_else(|_| Err(invalid("bench preparation failed")));
                    if finished
                        .send(Completion {
                            ticket: request.ticket,
                            result,
                        })
                        .is_err()
                    {
                        return;
                    }
                    if matches!(
                        wake.try_send(crate::arena_task::ArenaOut::MindBulkReady),
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
                    ) {
                        return;
                    }
                }
            })
            .map_err(|_| invalid("bench worker unavailable"))?;
        Ok(Self {
            sender: Some(sender),
            done: Some(done),
            thread: Some(thread),
            stop,
        })
    }
    // The refusal path returns the exact lease-bearing work for rollback;
    // dropping it here would release state before the service updates its roots.
    #[allow(clippy::result_large_err)]
    pub fn submit(&self, request: Request) -> Result<(), (BulkError, Request)> {
        let Some(sender) = &self.sender else {
            return Err((BulkError::QueueFull, request));
        };
        sender.try_send(request).map_err(|e| match e {
            TrySendError::Full(r) | TrySendError::Disconnected(r) => (BulkError::QueueFull, r),
        })
    }
    pub fn poll(&self) -> Option<Completion> {
        self.done.as_ref()?.try_recv().ok()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.sender.take();
        self.done.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
