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
pub(crate) struct Request {
    pub ticket: u64,
    pub recipe: Recipe,
    pub factory: Arc<ScenarioFactory>,
}
pub(crate) struct Completion {
    pub ticket: u64,
    pub recipe: Recipe,
    pub result: Result<FrozenProving, BulkError>,
}
pub(crate) struct Preparer {
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
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        request.factory.prepare(&request.recipe)
                    }))
                    .unwrap_or_else(|_| Err(invalid("proving preparation failed")));
                    if finished
                        .send(Completion {
                            ticket: request.ticket,
                            recipe: request.recipe,
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
            .map_err(|e| invalid(format!("cannot start proving preparation: {e}")))?;
        Ok(Self {
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
