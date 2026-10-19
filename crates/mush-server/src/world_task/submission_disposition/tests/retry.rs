//! Actual queue/coordinator retry tests. The forwarding store faults only the
//! read-only preflight; all permits, staged images and durable facts are real.

use super::*;
use crate::world_task::{COMMAND_PREFLIGHT_RETRY_DELAY, command_submit_ready};
use mush_core::submission_observation::{SubmissionRuntime, SubmissionRuntimeState};
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

mod store;
use store::TransientPreflightStore;

struct CountedRuntime(Arc<AtomicUsize>);
impl SubmissionRuntime for CountedRuntime {
    fn observe(&self, request: &mush_core::AuthoritativeSubmit) -> SubmissionRuntimeState {
        self.0.fetch_add(1, Ordering::SeqCst);
        Runtime.observe(request)
    }
}
fn counted(engine: &mut Engine) -> TestResult<(CommandSubmitDriver, Arc<AtomicUsize>)> {
    let calls = Arc::new(AtomicUsize::new(0));
    engine.set_submission_runtime(Arc::new(CountedRuntime(calls.clone())))?;
    let mut driver = CommandSubmitDriver::new(1);
    driver.runtime_installed = true;
    Ok((driver, calls))
}
fn selector(
    actor: mush_core::ObjId,
    amount: i64,
) -> TestResult<Box<mush_core::AuthoritativeSubmit>> {
    Ok(Box::new(mush_core::AuthoritativeSubmit {
        actor,
        action: action()?,
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: serde_json::json!({"amount":amount}),
        key: IdempotencyKeyText::parse_canonical("observed.bump")?,
    }))
}

#[tokio::test(start_paused = true)]
async fn transient_retry_retains_one_observation_and_waits_before_actual_success() -> TestResult {
    let (mut engine, captures) = fixture()?;
    let (mut driver, calls) = counted(&mut engine)?;
    let store = TransientPreflightStore::new(1);
    let mut observation = one(submit(&mut engine, &captures, "@observed 3")?)?;
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::DeferredByPreflight
    ));
    assert_eq!(store.lookups(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(engine.has_authoritative_submit_for(GOD));
    assert!(!engine.has_staged_action());
    assert_eq!(observation.take_terminal(), None);

    // Neither a later plain submission nor another observed command replaces
    // the retained selector. Its queue slot counts exactly once.
    assert!(
        engine
            .queue_authoritative_submit(selector(GOD, 99)?)
            .is_err()
    );
    let mut later_output = Outbox::new();
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: "@observed 99".into(),
        },
        &mut later_output,
        &mut Effects::new(),
    );
    let later = std::mem::take(&mut *captures.lock().map_err(|_| "capture poisoned")?);
    for mut later in later {
        assert_eq!(
            later.take_terminal(),
            Some(SubmissionDisposition::NotDispatched)
        );
    }
    assert!(
        later_output
            .events()
            .iter()
            .any(|(_, event)| mush_testkit::text_of(event).contains("still committing")),
        "actual same-actor backpressure"
    );
    assert_eq!(engine.queued_authoritative_actors().count(), 1);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::DeferredByPreflight
    ));
    assert_eq!(store.lookups(), 1, "direct calls cannot bypass the timer");
    assert!(
        tokio::time::timeout(
            COMMAND_PREFLIGHT_RETRY_DELAY - Duration::from_millis(1),
            command_submit_ready(driver.retry_after)
        )
        .await
        .is_err()
    );
    assert_eq!(store.lookups(), 1);
    command_submit_ready(driver.retry_after).await;
    let CommandSubmitDrive::Staged {
        staged,
        disposition: Some(proof),
        actor,
    } = driver.drive_one(&mut engine, &store).await?
    else {
        return Err("healed original attempt must stage".into());
    };
    assert_eq!(actor, GOD);
    assert_eq!(staged.summary.as_deref(), Some("The counter stands at 3."));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "retry did not replace its reader or private Attempt"
    );
    assert_eq!(store.lookups(), 2);
    assert!(!engine.has_authoritative_submit());
    assert!(driver.retry_after.is_none());
    let (bundle, installation) = engine
        .take_staged_action()?
        .ok_or("staged image")?
        .into_parts();
    assert_eq!(
        (bundle.token(), bundle.bundle_hash()),
        (staged.token, staged.bundle_hash)
    );
    store.reserve_authoritative_intent(*bundle.clone()).await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    engine.install_composed_action(installation)?;
    drop(proof);
    assert_eq!(
        observation.take_terminal(),
        None,
        "successful application is not absence"
    );
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::Idle
    ));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn healed_vacant_refusal_completes_original_observation_and_releases_queue_slot() -> TestResult
{
    let (mut engine, captures) = fixture()?;
    let (mut driver, calls) = counted(&mut engine)?;
    let store = TransientPreflightStore::new(2);
    let mut observation = one(submit(&mut engine, &captures, "@observed 0")?)?;
    for expected in 1..=2 {
        command_submit_ready(driver.retry_after).await;
        assert!(matches!(
            driver.drive_one(&mut engine, &store).await?,
            CommandSubmitDrive::DeferredByPreflight
        ));
        assert_eq!(store.lookups(), expected);
        assert_eq!(observation.take_terminal(), None);
        assert_eq!(engine.queued_authoritative_actors().count(), 1);
    }
    command_submit_ready(driver.retry_after).await;
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::Refused {
            actor: GOD,
            integrity: false,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        observation.take_terminal(),
        Some(SubmissionDisposition::NoIntent)
    );
    assert_eq!(observation.take_terminal(), None);
    assert!(!engine.has_authoritative_submit());
    assert!(!engine.has_staged_action());
    engine
        .queue_authoritative_submit(selector(GOD, 4)?)
        .map_err(|_| "released actor slot")?;
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::Staged {
            disposition: None,
            ..
        }
    ));
    let (_, installation) = engine
        .take_staged_action()?
        .ok_or("new plain image")?
        .into_parts();
    engine.abort_prepared_action(installation)?;
    Ok(())
}

#[test]
fn retry_rejects_foreign_engine_and_shares_original_queue_budget() -> TestResult {
    let (mut origin, captures) = fixture()?;
    let (mut foreign, _) = fixture()?;
    let (_, calls) = counted(&mut origin)?;
    let mut observation = one(submit(&mut origin, &captures, "@observed 3")?)?;
    let driven = origin
        .take_observed_authoritative_submit()
        .ok_or("original handoff")?;
    let expected = driven.request().clone();
    let driven = foreign
        .requeue_observed_authoritative_submit(driven)
        .err()
        .ok_or("foreign Engine must refuse")?;
    assert_eq!(driven.request(), &expected);
    assert!(!foreign.has_authoritative_submit());
    assert_eq!(observation.take_terminal(), None);
    origin
        .requeue_observed_authoritative_submit(driven)
        .map_err(|_| "origin restoration")?;
    for i in 1..mush_core::engine::AUTHORITATIVE_SUBMIT_QUEUE_MAX {
        origin
            .queue_authoritative_submit(selector(mush_core::ObjId(100 + u32::try_from(i)?), 0)?)
            .map_err(|_| "bounded other actor")?;
    }
    assert_eq!(
        origin.queued_authoritative_actors().count(),
        mush_core::engine::AUTHORITATIVE_SUBMIT_QUEUE_MAX
    );
    assert!(
        origin
            .queue_authoritative_submit(selector(mush_core::ObjId(9999), 0)?)
            .is_err()
    );
    let mut original = None;
    while let Some(driven) = origin.take_observed_authoritative_submit() {
        if driven.is_observed() {
            original = Some(driven);
        }
    }
    let driven = original.ok_or("same retained handoff")?;
    assert_eq!(driven.request(), &expected);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    origin
        .requeue_observed_authoritative_submit(driven)
        .map_err(|_| "restored after other actors drain")?;
    assert_eq!(observation.take_terminal(), None);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn retry_cursor_reaches_third_actor_despite_replenished_second_actor() -> TestResult {
    let captures = Captures::default();
    let mut world = mush_testkit::TestWorld::with_modules(vec![Box::new(Module(captures.clone()))]);
    let bystander = world.spawn_player("bystander");
    let third = world.spawn_player("third");
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x61; 32])?, [])?);
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let (mut driver, calls) = counted(&mut engine)?;
    let store = TransientPreflightStore::new(1);
    let mut first = one(submit(&mut engine, &captures, "@observed 3")?)?;
    engine.step(
        Inbound::Cmd {
            conn: 2,
            actor: bystander,
            line: "@observed 0".into(),
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    let mut other = one(std::mem::take(
        &mut *captures.lock().map_err(|_| "capture poisoned")?,
    ))?;
    engine.step(
        Inbound::Cmd {
            conn: 3,
            actor: third,
            line: "@observed 0".into(),
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    let mut last = one(std::mem::take(
        &mut *captures.lock().map_err(|_| "capture poisoned")?,
    ))?;
    assert!(GOD < bystander && bystander < third);
    assert_eq!(engine.queued_authoritative_actors().count(), 3);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::DeferredByPreflight
    ));
    command_submit_ready(driver.retry_after).await;
    assert!(
        matches!(driver.drive_one(&mut engine, &store).await?, CommandSubmitDrive::Refused { actor, integrity: false, .. } if actor == bystander)
    );
    assert_eq!(other.take_terminal(), Some(SubmissionDisposition::NoIntent));
    assert!(engine.has_authoritative_submit_for(GOD));
    // Replenishing B must not reset the active rotation to A -> B forever.
    engine.step(
        Inbound::Cmd {
            conn: 2,
            actor: bystander,
            line: "@observed 0".into(),
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    let mut replenished = one(std::mem::take(
        &mut *captures.lock().map_err(|_| "capture poisoned")?,
    ))?;
    assert!(
        matches!(driver.drive_one(&mut engine, &store).await?, CommandSubmitDrive::Refused { actor, integrity: false, .. } if actor == third)
    );
    assert_eq!(last.take_terminal(), Some(SubmissionDisposition::NoIntent));
    assert_eq!(replenished.take_terminal(), None);
    store.arm(1);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::DeferredByPreflight
    ));
    assert_eq!(first.take_terminal(), None);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "three actors, not four attempts"
    );
    command_submit_ready(driver.retry_after).await;
    assert!(
        matches!(driver.drive_one(&mut engine, &store).await?, CommandSubmitDrive::Refused { actor, integrity: false, .. } if actor == bystander)
    );
    assert_eq!(
        replenished.take_terminal(),
        Some(SubmissionDisposition::NoIntent)
    );
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::Staged { actor: GOD, .. }
    ));
    let (_, installation) = engine
        .take_staged_action()?
        .ok_or("original staged image")?
        .into_parts();
    engine.abort_prepared_action(installation)?;
    // The retry is gone. Later ordinary work resumes the existing key order.
    engine
        .queue_authoritative_submit(selector(third, 0)?)
        .map_err(|_| "third plain")?;
    engine
        .queue_authoritative_submit(selector(bystander, 0)?)
        .map_err(|_| "bystander plain")?;
    assert_eq!(
        engine
            .take_observed_authoritative_submit()
            .ok_or("plain ordering")?
            .request()
            .actor,
        bystander
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn retry_stops_at_actual_pending_or_finalized_without_requeue_or_absence() -> TestResult {
    for finalized in [false, true] {
        let store = TransientPreflightStore::new(0);
        let (mut first, captures) = fixture()?;
        let _observation = one(submit(&mut first, &captures, "@observed 3")?)?;
        assert!(matches!(
            CommandSubmitDriver::new(1)
                .drive_one(&mut first, &store)
                .await?,
            CommandSubmitDrive::Staged { .. }
        ));
        let (bundle, installation) = first
            .take_staged_action()?
            .ok_or("first staged image")?
            .into_parts();
        store.reserve_authoritative_intent(*bundle.clone()).await?;
        if finalized {
            store
                .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
                .await?;
        }
        drop(installation); // Explicit lost local image; second Engine reads durable truth.
        let (mut engine, captures) = fixture()?;
        let (mut driver, calls) = counted(&mut engine)?;
        let mut observation = one(submit(&mut engine, &captures, "@observed 3")?)?;
        store.arm(1);
        assert!(matches!(
            driver.drive_one(&mut engine, &store).await?,
            CommandSubmitDrive::DeferredByPreflight
        ));
        command_submit_ready(driver.retry_after).await;
        assert!(matches!(
            (finalized, driver.drive_one(&mut engine, &store).await?),
            (false, CommandSubmitDrive::AlreadyOwned { .. })
                | (true, CommandSubmitDrive::AlreadyFinished { .. })
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(observation.take_terminal(), None);
        assert!(!engine.has_authoritative_submit());
        assert!(driver.retry_after.is_none());
        assert!(!engine.has_staged_action());
    }
    Ok(())
}

#[tokio::test]
async fn plain_transient_keeps_existing_refusal_and_cannot_enter_observed_retry() -> TestResult {
    let (mut engine, _) = fixture()?;
    engine
        .queue_authoritative_submit(selector(GOD, 3)?)
        .map_err(|_| "plain queue")?;
    let driven = engine
        .take_observed_authoritative_submit()
        .ok_or("plain handoff")?;
    assert!(!driven.is_observed());
    let driven = engine
        .requeue_observed_authoritative_submit(driven)
        .err()
        .ok_or("plain retry refusal")?;
    engine
        .queue_authoritative_submit(driven.into_parts().0)
        .map_err(|_| "plain requeue")?;
    let mut driver = CommandSubmitDriver::new(1);
    let store = TransientPreflightStore::new(1);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::Refused {
            integrity: false,
            ..
        }
    ));
    assert!(!engine.has_authoritative_submit());
    assert!(driver.retry_after.is_none());
    Ok(())
}
