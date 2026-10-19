//! One bounded off-world preparation lane. It expands production terrain/seals
//! and owns all large serialization before the world publishes an admission.
use super::{
    BulkError, invalid,
    proving::FrozenProving,
    scenarios::{Recipe, ScenarioFactory},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
pub(super) enum Request {
    Proving {
        ticket: u64,
        recipe: Recipe,
        factory: Arc<ScenarioFactory>,
    },
    Registry {
        ticket: u64,
        account: mush_core::AccountId,
        actor: mush_core::ObjId,
        nonce: u64,
        revision: mush_roll::Revision,
        source: Arc<str>,
        factory: Arc<crate::minds::registry::factory::Factory>,
    },
}
pub(super) enum Completion {
    Proving {
        ticket: u64,
        recipe: Recipe,
        result: Result<FrozenProving, BulkError>,
    },
    Registry {
        ticket: u64,
        result: Result<crate::minds::registry::factory::PreparedRegistry, BulkError>,
    },
}
impl Completion {
    pub fn ticket(&self) -> u64 {
        match self {
            Self::Proving { ticket, .. } | Self::Registry { ticket, .. } => *ticket,
        }
    }
}
pub(super) struct Preparer {
    submitted: AtomicBool,
    sender: Option<SyncSender<Request>>,
    done: Option<Receiver<Completion>>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}
impl Preparer {
    pub fn new(
        limit: usize,
        wake: tokio::sync::mpsc::Sender<crate::arena_task::ArenaOut>,
    ) -> Result<Self, BulkError> {
        let (sender, receiver) = mpsc::sync_channel::<Request>(limit);
        let (finished, done) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("mind-bulk-prepare".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    if stopped.load(Ordering::Acquire) {
                        return;
                    }
                    let done = match request {
                        Request::Proving {
                            ticket,
                            recipe,
                            factory,
                        } => {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    factory.prepare(&recipe)
                                }))
                                .unwrap_or_else(|_| Err(invalid("proving preparation failed")));
                            Completion::Proving {
                                ticket,
                                recipe,
                                result,
                            }
                        }
                        Request::Registry {
                            ticket,
                            account,
                            actor,
                            nonce,
                            revision,
                            source,
                            factory,
                        } => {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    factory.prepare(account, actor, nonce, revision, source)
                                }))
                                .map_err(|_| invalid("Registry preparation failed"))
                                .and_then(|r| {
                                    r.map_err(|_| invalid("Registry preparation refused"))
                                });
                            Completion::Registry { ticket, result }
                        }
                    };
                    if finished.send(done).is_err() {
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
            .map_err(|e| invalid(format!("cannot start proving preparation: {e}")))?;
        Ok(Self {
            submitted: AtomicBool::new(false),
            sender: Some(sender),
            done: Some(done),
            thread: Some(thread),
            stop,
        })
    }
    // Refusal returns the same owner-bound request; its retained sources
    // remain with the caller until rollback, rather than being silently dropped.
    #[allow(clippy::result_large_err)]
    pub fn submit(&self, request: Request) -> Result<(), (BulkError, Request)> {
        self.submitted.store(true, Ordering::Release);
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
    pub(super) fn never_submitted(&self) -> bool {
        !self.submitted.load(Ordering::Acquire)
    }
}
impl Drop for Preparer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.sender.take();
        self.done.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
