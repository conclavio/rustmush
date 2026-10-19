#![cfg(test)]

use super::super::{
    ActionCompletion, CommandSubmitDrive, CommandSubmitDriver, PendingCommit, PendingReceipt,
};
use super::*;

mod paid_purchase;
mod retry;
use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, DefinitionDescriptor, DefinitionId,
    DefinitionKind, DefinitionRef, Effects, GOD, GameModule, IdempotencyKeyText, Inbound, ModError,
    Outbox, PerceivedRef, Registrar,
    mechanics::{DefinitionRegistryBuilder, OwnerId},
    offer::{OfferKeyRing, OfferSecret},
    submission_observation::{SubmissionDisposition, SubmissionObservation},
};
use mush_store::{AuthoritativeStore, BundleStatus, InMemoryStore};
use std::sync::Mutex;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Captures = Arc<Mutex<Vec<SubmissionObservation>>>;

fn action() -> TestResult<DefinitionRef> {
    let owner = OwnerId::new(mush_contract_tests::b1_seam::B1_OWNER)?;
    let mut definitions = DefinitionRegistryBuilder::new();
    let outcome = definitions.register(
        owner.clone(),
        DefinitionDescriptor::new(
            DefinitionKind::new("outcome")?,
            DefinitionId::new(mush_contract_tests::b1_seam::B1_OUTCOME)?,
            1,
            serde_json::json!({"fields":["actor","count"]}),
        )?
        .as_current(),
    )?;
    Ok(definitions.register(
        owner,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new(mush_contract_tests::b1_seam::B1_ACTION)?,
            1,
            serde_json::json!({"subject":"self_actor","targets":0}),
        )?
        .as_current()
        .with_references(vec![outcome]),
    )?)
}
struct Module(Captures);
impl GameModule for Module {
    fn name(&self) -> &'static str {
        mush_contract_tests::b1_seam::B1_OWNER
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        mush_contract_tests::b1_seam::B1SeamModule.register(reg)?;
        reg.classify_commands(CommandCompatibility::Authoritative);
        let action = action().map_err(|e| ModError::msg(e.to_string()))?;
        let captures = self.0.clone();
        reg.command(
            CommandSpec {
                name: "@observed".into(),
                shape: ArgShape::Raw,
                switches: vec!["mutate".into(), "twice".into(), "error".into()],
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let amount = ctx
                    .args
                    .raw()
                    .trim()
                    .parse::<i64>()
                    .map_err(|e| CmdError::msg(e.to_string()))?;
                if ctx.switches.has("mutate") {
                    ctx.actor = mush_core::ObjId(999);
                    ctx.module = Some("forged".into());
                }
                for _ in 0..if ctx.switches.has("twice") { 2 } else { 1 } {
                    let observation = ctx.submit_authoritative_observed(
                        action.clone(),
                        PerceivedRef::SelfActor,
                        vec![],
                        serde_json::json!({"amount":amount}),
                        IdempotencyKeyText::parse_canonical("observed.bump")
                            .map_err(|e| CmdError::msg(e.to_string()))?,
                    )?;
                    captures
                        .lock()
                        .map_err(|_| CmdError::msg("capture poisoned"))?
                        .push(observation);
                }
                if ctx.switches.has("error") {
                    return Err(CmdError::msg("deliberate command refusal"));
                }
                Ok(())
            }),
        )
    }
}
fn fixture() -> TestResult<(Engine, Captures)> {
    let captures = Captures::default();
    let mut world = mush_testkit::TestWorld::with_modules(vec![Box::new(Module(captures.clone()))]);
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x61; 32])?, [])?);
    let _ = world.drain_write_batch();
    Ok((world.into_engine(), captures))
}
fn submit(
    engine: &mut Engine,
    captures: &Captures,
    line: &str,
) -> TestResult<Vec<SubmissionObservation>> {
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: line.into(),
        },
        &mut out,
        &mut effects,
    );
    let captured = std::mem::take(&mut *captures.lock().map_err(|_| "capture poisoned")?);
    if captured.is_empty() {
        let lines: Vec<_> = out
            .events()
            .iter()
            .map(|(_, event)| mush_testkit::text_of(event))
            .collect();
        return Err(format!(
            "no observation for {line:?}; actual dispatch output: {lines:?}; deferred={}",
            out.deferred().len()
        )
        .into());
    }
    Ok(captured)
}
fn one(values: Vec<SubmissionObservation>) -> TestResult<SubmissionObservation> {
    let [value]: [SubmissionObservation; 1] = values.try_into().map_err(|_| "one observation")?;
    Ok(value)
}
async fn stage(
    engine: &mut Engine,
    store: &InMemoryStore,
    driver: &mut CommandSubmitDriver,
) -> TestResult<(
    Box<mush_core::AuthoritativeBundle>,
    mush_core::composed::ComposedInstallation,
    StagedAttempt,
)> {
    let CommandSubmitDrive::Staged {
        staged,
        disposition: Some(disposition),
        ..
    } = driver.drive_one(engine, store).await?
    else {
        return Err("actual observed provider did not stage".into());
    };
    let (bundle, installation) = engine
        .take_staged_action()?
        .ok_or("actual staged installation")?
        .into_parts();
    assert_eq!(
        (bundle.token(), bundle.bundle_hash()),
        (staged.token, staged.bundle_hash)
    );
    Ok((bundle, installation, disposition))
}

fn pending(
    bundle: &mush_core::AuthoritativeBundle,
    installation: mush_core::composed::ComposedInstallation,
    disposition: StagedAttempt,
    receiver: tokio::sync::oneshot::Receiver<
        Result<mush_store::AuthoritativeReceipt, AuthoritativeCommitFailure>,
    >,
) -> PendingCommit {
    PendingCommit {
        token: super::super::CommitToken::Authoritative(bundle.token()),
        receipt: PendingReceipt::Authoritative {
            expected_token: bundle.token(),
            expected_hash: bundle.bundle_hash(),
            ownership: None,
            receiver,
        },
        sends: Vec::new(),
        effects: Vec::new(),
        failure_actor: None,
        create: None,
        action: Some(ActionCompletion {
            staged: StagedAuthoritativeAction {
                token: bundle.token(),
                bundle_hash: bundle.bundle_hash(),
                summary: None,
            },
            disposition: Some(disposition),
            owned: None,
            finished: None,
            ownership_valid: false,
            post_commit_decision: None,
            post_commit_decision_response: None,
            post_commit_decision_terminal: None,
        }),
        prepared_installation: Some(installation),
        barriers: Vec::new(),
        coalesced_reattach: None,
        exit_after_ack: false,
    }
}

fn complete(
    engine: &mut Engine,
    pending: PendingCommit,
    result: Result<(), AuthoritativeCommitFailure>,
    store: Arc<dyn mush_store::ObjectStore>,
) -> bool {
    let (gateway_tx, _gateway_rx) = tokio::sync::mpsc::channel(16);
    let gateway: super::super::Gateway = Arc::new(crate::gateway_sink::SocketSink::new(
        gateway_tx,
        Arc::new(mush_config::metrics::Metrics::new()),
        Arc::new(tokio::sync::Notify::new()),
        16,
        90,
    ));
    let mut health = super::super::StoreHealth::new();
    let integrity = super::super::PersistenceIntegrityLatch::new();
    let (world_tx, _world_rx) = tokio::sync::mpsc::channel(1);
    super::super::complete_pending_commit(
        pending,
        result,
        engine,
        &gateway,
        &mut health,
        &integrity,
        &world_tx.downgrade(),
        None,
        &store,
    );
    integrity.is_latched()
}

#[tokio::test]
async fn actual_vacant_refusal_is_consumed_before_retry_and_dispatch_actor_is_immutable()
-> TestResult {
    let (mut engine, captures) = fixture()?;
    let store = InMemoryStore::new();
    let mut driver = CommandSubmitDriver::new(1);
    let mut rejected = one(submit(&mut engine, &captures, "@observed 0")?)?;
    assert_eq!(rejected.take_terminal(), None);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await?,
        CommandSubmitDrive::Refused {
            integrity: false,
            ..
        }
    ));
    assert_eq!(
        rejected.take_terminal(),
        Some(SubmissionDisposition::NoIntent)
    );
    let mut accepted = one(submit(&mut engine, &captures, "@observed/mutate 3")?)?;
    let (bundle, installation, proof) = stage(&mut engine, &store, &mut driver).await?;
    assert_eq!(
        bundle.idempotency().scope().principal,
        mush_core::PrincipalId::Object { id: GOD }
    );
    assert_eq!(rejected.take_terminal(), None);
    assert_eq!(accepted.take_terminal(), None);
    engine.abort_prepared_action(installation)?;
    drop(proof);
    assert_eq!(
        accepted.take_terminal(),
        None,
        "an arbitrary local abort is not durable vacancy proof"
    );
    Ok(())
}

#[tokio::test]
async fn real_fanout_and_handler_error_report_retry_only() -> TestResult {
    for line in ["@observed/twice 3", "@observed/error 3"] {
        let (mut engine, captures) = fixture()?;
        let observations = submit(&mut engine, &captures, line)?;
        assert!(!observations.is_empty());
        assert!(!engine.has_authoritative_submit());
        for mut observed in observations {
            assert_eq!(
                observed.take_terminal(),
                Some(SubmissionDisposition::NotDispatched)
            );
            assert_eq!(observed.take_terminal(), None);
        }
    }
    Ok(())
}

#[tokio::test]
async fn real_pending_and_finalized_retry_never_publish_no_intent() -> TestResult {
    let store = InMemoryStore::new();
    let (mut original, captures) = fixture()?;
    let mut first = one(submit(&mut original, &captures, "@observed 3")?)?;
    let (bundle, installation, proof) =
        stage(&mut original, &store, &mut CommandSubmitDriver::new(1)).await?;
    store.reserve_authoritative_intent(*bundle.clone()).await?;
    drop(installation);
    drop(proof);
    assert_eq!(
        first.take_terminal(),
        None,
        "lost post-intent installation stays unresolved"
    );
    for finalized in [false, true] {
        if finalized {
            store
                .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
                .await?;
        }
        let (mut retry, captures) = fixture()?;
        let mut observation = one(submit(&mut retry, &captures, "@observed 3")?)?;
        let result = CommandSubmitDriver::new(1)
            .drive_one(&mut retry, &store)
            .await?;
        assert!(matches!(
            (finalized, result),
            (false, CommandSubmitDrive::AlreadyOwned { .. })
                | (true, CommandSubmitDrive::AlreadyFinished { .. })
        ));
        assert_eq!(observation.take_terminal(), None);
        assert!(!retry.has_staged_action());
    }
    Ok(())
}

#[tokio::test]
async fn actual_writer_absent_refusal_completes_only_after_exact_installation_abort() -> TestResult
{
    let store = Arc::new(InMemoryStore::new());
    let (mut engine, captures) = fixture()?;
    let mut observation = one(submit(&mut engine, &captures, "@observed 3")?)?;
    let (bundle, installation, proof) =
        stage(&mut engine, &store, &mut CommandSubmitDriver::new(1)).await?;
    let identity = Some((bundle.token(), bundle.bundle_hash()));
    let (mut other, other_captures) = fixture()?;
    let _other_observation = one(submit(&mut other, &other_captures, "@observed 4")?)?;
    let (occupied, other_installation, other_proof) =
        stage(&mut other, &store, &mut CommandSubmitDriver::new(1)).await?;
    store.reserve_authoritative_intent(*occupied).await?;
    drop(other_installation);
    drop(other_proof);
    let (signals, _signal_rx) = super::super::writer_signal_channel();
    let (writer, join) = crate::persist::spawn_store_writer(
        store.clone(),
        signals,
        Arc::new(mush_config::metrics::Metrics::new()),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        crate::quarantine::QuarantineSpool::in_memory(),
        super::super::PersistenceIntegrityLatch::default(),
    );
    let (ownership, owned) = tokio::sync::oneshot::channel();
    let (notify, finished) = tokio::sync::oneshot::channel();
    let mut pending = pending(&bundle, installation, proof, finished);
    writer
        .send(crate::persist::StoreJob::Authoritative {
            bundle,
            ownership,
            notify,
        })
        .await
        .map_err(|_| "writer admission")?;
    let failure = pending
        .receipt
        .receive()
        .await
        .err()
        .ok_or("actual writer refusal")?;
    assert!(matches!(&failure, AuthoritativeCommitFailure::Refused(_)));
    assert!(matches!(
        owned.await?,
        Err(AuthoritativeCommitFailure::Refused(_))
    ));
    assert_eq!(
        observation.take_terminal(),
        None,
        "durable Absent alone has not aborted the local image"
    );
    assert!(!complete(&mut engine, pending, Err(failure), store.clone()));
    assert_eq!(
        observation.take_terminal(),
        Some(SubmissionDisposition::NoIntent)
    );
    assert_eq!(observation.take_terminal(), None);
    let (token, hash) = identity.ok_or("identity")?;
    assert_eq!(
        store.authoritative_status(token, hash).await?,
        BundleStatus::Absent
    );
    drop(writer);
    join.await?;
    Ok(())
}

#[tokio::test]
async fn dropped_exact_receipt_freezes_without_releasing_observed_custody() -> TestResult {
    let store = Arc::new(InMemoryStore::new());
    let (mut engine, captures) = fixture()?;
    let mut observation = one(submit(&mut engine, &captures, "@observed 3")?)?;
    let (bundle, installation, proof) =
        stage(&mut engine, &store, &mut CommandSubmitDriver::new(1)).await?;
    store.reserve_authoritative_intent(*bundle.clone()).await?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut pending = pending(&bundle, installation, proof, receiver);
    drop(sender);
    let failure = pending
        .receipt
        .receive()
        .await
        .err()
        .ok_or("dropped exact receipt")?;
    assert!(matches!(&failure, AuthoritativeCommitFailure::Integrity(_)));
    assert!(complete(&mut engine, pending, Err(failure), store.clone()));
    assert_eq!(observation.take_terminal(), None);
    assert!(
        engine.action_state().has_prepared(),
        "unknown installation cannot be reused"
    );
    assert!(engine.action_state().reservation_lost());
    assert_eq!(
        store
            .authoritative_status(bundle.token(), bundle.bundle_hash())
            .await?,
        BundleStatus::PendingExact
    );
    Ok(())
}

#[tokio::test]
async fn crossed_refusal_identity_cannot_complete_or_release_the_installation() -> TestResult {
    let store = InMemoryStore::new();
    let (mut engine, captures) = fixture()?;
    let mut observation = one(submit(&mut engine, &captures, "@observed 3")?)?;
    let (bundle, installation, proof) =
        stage(&mut engine, &store, &mut CommandSubmitDriver::new(1)).await?;
    let failure = AuthoritativeCommitFailure::Refused("test wrong exact identity".into());
    assert!(
        proof
            .abort_refused(
                &mut engine,
                installation,
                &failure,
                Some((bundle.token(), CanonicalHash::from_bytes([0x73; 32])))
            )
            .is_err()
    );
    assert_eq!(observation.take_terminal(), None);
    assert!(engine.action_state().has_prepared());
    assert!(engine.action_state().reservation_lost());
    Ok(())
}
