#![cfg(test)]

use super::*;
use crate::mechanics::{DefinitionRegistryBuilder, OwnerId};
use crate::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, GOD, IdempotencyKeyText, PerceivedRef,
};
use std::sync::atomic::AtomicBool;

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn request() -> Result<Box<AuthoritativeSubmit>, Box<dyn std::error::Error>> {
    let action = DefinitionRegistryBuilder::new().register(
        OwnerId::new("probe")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new("observe")?,
            1,
            serde_json::json!({}),
        )?,
    )?;
    Ok(Box::new(AuthoritativeSubmit {
        actor: GOD,
        action,
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: serde_json::json!({}),
        key: IdempotencyKeyText::parse_canonical("attempt")?,
    }))
}
struct Reader(Arc<AtomicBool>);
impl SubmissionDispositionReader for Reader {
    fn no_intent(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
struct Runtime(Arc<AtomicBool>);
impl SubmissionRuntime for Runtime {
    fn observe(&self, _: &AuthoritativeSubmit) -> SubmissionRuntimeState {
        SubmissionRuntimeState::new(Arc::new(Reader(self.0.clone())), Box::new(()))
    }
}

#[test]
fn last_undriven_copy_reports_only_not_dispatched_once() -> TestResult {
    let (request, mut observed) = ObservedSubmission::new(request()?);
    let copy = request.clone();
    drop(request);
    assert_eq!(observed.take_terminal(), None);
    drop(copy);
    assert_eq!(
        observed.take_terminal(),
        Some(SubmissionDisposition::NotDispatched)
    );
    assert_eq!(observed.take_terminal(), None);
    Ok(())
}

#[test]
fn only_one_clone_enters_runtime_and_drop_after_entry_stays_unresolved() -> TestResult {
    let (request, mut observed) = ObservedSubmission::new(request()?);
    let copy = request.clone();
    let driven = QueuedSubmission::Observed(request)
        .drive(None, Arc::new(()))
        .ok_or("first handoff")?;
    assert!(
        QueuedSubmission::Observed(copy)
            .drive(None, Arc::new(()))
            .is_none()
    );
    drop(driven);
    assert_eq!(observed.take_terminal(), None);
    Ok(())
}

#[test]
fn completion_and_consumption_belong_to_one_exact_attempt() -> TestResult {
    let first = Arc::new(AtomicBool::new(false));
    let second = Arc::new(AtomicBool::new(false));
    let (a, mut old) = ObservedSubmission::new(request()?);
    let (b, mut current) = ObservedSubmission::new(request()?);
    let _a = QueuedSubmission::Observed(a)
        .drive(Some(&Runtime(first.clone())), Arc::new(()))
        .ok_or("old handoff")?;
    let _b = QueuedSubmission::Observed(b)
        .drive(Some(&Runtime(second.clone())), Arc::new(()))
        .ok_or("new handoff")?;
    first.store(true, Ordering::Release);
    assert_eq!(old.take_terminal(), Some(SubmissionDisposition::NoIntent));
    assert_eq!(old.take_terminal(), None);
    assert_eq!(
        current.take_terminal(),
        None,
        "old refusal cannot complete an identical new selector"
    );
    second.store(true, Ordering::Release);
    assert_eq!(
        current.take_terminal(),
        Some(SubmissionDisposition::NoIntent)
    );
    Ok(())
}
