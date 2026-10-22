#![cfg(test)]

//! One actual shipped-source certificate, registered filing and Conduct owner
//! run. The submission-disposition runtime is a labeled fixture boundary; all
//! qualification evidence comes from the real worker and installed owners.
use super::*;
use crate::minds::bulk::conduct_service;
use mush_core::{CmdError, Ctx};
use mush_roll::commerce::conduct::{
    ConductTrialProof,
    records::{ConductExecution, ConductJob, ConductResult},
};

const FIRST: u128 = 150_000_000;
const PROGRAM: &str = include_str!("../../../content/minds/house/safe_conduct.rhai");
const ENVELOPE: &str = include_str!("../../../content/minds/house/safe_conduct.envelope.json");

pub(super) fn certified_safe_conduct() -> TestResult<(Box<Fixture>, RatingRef)> {
    let mut fixture = Fixture::with_program(PROGRAM, serde_json::from_str(ENVELOPE)?)?;
    let rating = complete_registry(&mut fixture)?;
    assert_eq!(fixture.revision.envelope().declared_class, "II");
    Ok((fixture, rating))
}

/// One genuine parent reused by qualified coverage; no replacement certificate,
/// fake grant, source setter, or prepared-result constructor is involved.
pub(super) fn filed_parent() -> TestResult<(Box<Fixture>, PurchaseFixture, RatingRef)> {
    let (mut registry, rating) = certified_safe_conduct()?;
    let mut world = World::new_minimal();
    assert_eq!(
        world.create(ObjKind::Player, "Conduct buyer", GOD, Some(ROOM))?,
        BUYER
    );
    world.set_now(1000);
    {
        let mut values = registry.accounts.values.lock().map_err(module_error)?;
        registry.accounts.epoch.invalidate();
        values.insert(BUYER, AccountId::mint("actual-conduct-buyer", 2)?);
    }
    seed_wallets(&mut registry.disk)?;
    let initial = Durable {
        disk: registry.disk.clone(),
        objects: world.iter().cloned().collect(),
        next_id: world.next_id(),
        now: world.now(),
    };
    let mut f = PurchaseFixture::boot(
        registry.resources.clone(),
        registry.accounts.clone(),
        &initial,
    )?;
    f.fund_buyer_cash()?;
    let reserve = reference(
        "roll",
        "action",
        mush_core::reserved_shell::RESERVE_ACTION,
        json!({"format":1,"purpose":"roll.initial_filing","object":"inert"}),
    )?;
    f.perform(&reserve, GOD, FIRST, json!({}))?;
    let revision = registry.revision.reference();
    let filing = f.filing.clone();
    f.perform(
        &filing,
        GOD,
        FIRST + 1,
        json!({
        "shell_request":RequestUid::from_bytes(FIRST.to_be_bytes()),
        "mind":revision.key.mind,"revision":revision.key.number,"revision_digest":revision.digest,
        "rating_job":rating.job(),"rating_id":rating.id(),"rating_digest":rating.receipt_digest(),
        "payer":0,"charter_name":"Actual Safe Conduct"}),
    )?;
    let list = f.config.quotes.listing_action()?;
    f.perform(&list, GOD, FIRST + 2, json!({"owner_licence":"1"}))?;
    let quote = f.config.quotes.quote_action()?;
    f.perform(&quote, BUYER, FIRST + 3, json!({"listing":token(FIRST+2)}))?;
    let [fund, grant, _] = f.config.purchases.actions()?;
    f.perform(&fund, BUYER, FIRST + 4, json!({"quote":token(FIRST+3)}))?;
    f.perform(
        &grant,
        BUYER,
        FIRST + 5,
        json!({"operation":token(FIRST+4)}),
    )?;
    assert_eq!(
        rows(&f.disk, "roll", "roll_mind_personal_paid_licence").len(),
        1
    );
    Ok((registry, f, rating))
}

pub(super) fn boot(f: &PurchaseFixture, base: &IcWorld, saved: &Durable) -> TestResult<IcWorld> {
    IcWorld::boot_source_conduct(
        f.into(),
        saved,
        base.base,
        base.field,
        base.hulls,
        true,
        base.pending_charters,
        base.paid_resales,
        base.registry_revocation.clone(),
        base.coverage_products.clone(),
        false,
        base.transfer_facts.clone(),
        true,
    )
}

// This invokes the actual registered Tacsim owner context. The closure has no
// state setter; proof creation still belongs solely to the real Roll reader.
pub(super) fn context<T: Send + 'static>(
    w: &mut IcWorld,
    call: impl for<'a> FnOnce(&mut Ctx<'a>) -> Result<T, crate::minds::bulk::BulkError> + Send + 'static,
) -> TestResult<T> {
    let answer = Arc::new(Mutex::new(None));
    let result = answer.clone();
    *w.coverage_calls.lock().map_err(module_error)? = Some(Box::new(move |ctx| {
        *result.lock().map_err(|_| CmdError::msg("Conduct answer"))? = Some(call(ctx));
        Ok(())
    }));
    w.engine.step(
        Inbound::Cmd {
            conn: CONN,
            actor: GOD,
            line: "@coverage-host-fixture".into(),
        },
        &mut Outbox::new(),
        &mut mush_core::Effects::new(),
    );
    let value = answer
        .lock()
        .map_err(module_error)?
        .take()
        .ok_or("actual Conduct owner context")??;
    Ok(value)
}
pub(super) fn proof(w: &mut IcWorld, operation: CommitToken) -> TestResult<ConductTrialProof> {
    context(w, move |ctx| conduct_service::current_proof(ctx, operation))
}

fn install_submission(
    w: &mut IcWorld,
    submission: Box<mush_core::AuthoritativeSubmit>,
    n: u128,
) -> TestResult {
    assert_eq!(submission.actor, GOD);
    assert_eq!(submission.subject, PerceivedRef::SelfActor);
    assert!(submission.targets.is_empty());
    assert_eq!(submission.action.key().owner.as_str(), "roll");
    let action = submission.action;
    let principal = PrincipalId::Object { id: GOD };
    let query = w
        .engine
        .legal_action_query(principal.clone(), GOD, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let projected = offers
        .projected_for_action(&action)
        .ok_or("actual Conduct signed offer")?;
    let request = RequestUid::from_bytes(n.to_be_bytes());
    let resolved = w.engine.resolve_legal_action(
        &offers,
        mush_core::OfferExecutionContext {
            request,
            principal: principal.clone(),
            actor: GOD,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("roll")?,
                    namespace: CanonicalId::new("conduct_fixture")?,
                },
                1,
                IdempotencyKey::Caller(submission.key),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: submission.parameters,
        },
    )?;
    let (exact, resolved, _) = resolved.into_authoritative_parts();
    assert_eq!(exact, action);
    let invocation = resolved.invocation().clone();
    let (p, o) = w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(&action, &resolved, token(n))?;
    w.stage_action(&action, n, p, o, &invocation)?;
    let held = w
        .engine
        .take_staged_action()?
        .ok_or("actual Conduct custody")?;
    let (bundle, installation) = held.into_parts();
    retain_writes(&mut w.disk, bundle.writes())?;
    w.engine.install_composed_action(installation)?;
    Ok(())
}

fn pulse(w: &mut IcWorld) -> TestResult {
    context(w, |ctx| {
        crate::minds::bulk::world::drain(ctx);
        Ok(())
    })?;
    if let Some(id) = conduct_service::fixture_entry(&w.engine.world) {
        w.command(&format!("@conduct/continue {id}"))?;
    }
    Ok(())
}
fn next_submission(
    w: &mut IcWorld,
) -> TestResult<mush_core::submission_observation::DrivenSubmission> {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        pulse(w)?;
        if let Some(submitted) = w.engine.take_observed_authoritative_submit() {
            return Ok(submitted);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err("actual Conduct continuation produced no submission".into())
}
fn execution(w: &IcWorld, operation: CommitToken) -> TestResult<ConductExecution> {
    let value = rows(&w.disk, "roll", ConductExecution::FAMILY)
        .into_iter()
        .find(|v| v.get("job") == Some(&json!(operation)))
        .ok_or("actual Conduct execution")?;
    Ok(serde_json::from_value(value)?)
}
fn disposition_retry(
    w: &mut IcWorld,
    submitted: mush_core::submission_observation::DrivenSubmission,
) -> TestResult<Box<mush_core::AuthoritativeSubmit>> {
    let (original, runtime) = submitted.into_parts();
    let absence = *runtime
        .ok_or("actual Conduct observation")?
        .downcast::<Arc<std::sync::atomic::AtomicBool>>()
        .map_err(|_| "Conduct observation type")?;
    let disk = w.disk.clone();
    for _ in 0..2 {
        pulse(w)?;
        assert!(w.engine.take_observed_authoritative_submit().is_none());
    }
    assert!(
        w.command("@conduct/retry").is_err(),
        "Unknown cannot authorize replacement"
    );
    assert!(disk == w.disk, "Unknown changed durable Conduct evidence");
    absence.store(true, std::sync::atomic::Ordering::Release);
    pulse(w)?;
    assert!(
        w.engine.take_observed_authoritative_submit().is_none(),
        "NoIntent requires explicit retry"
    );
    w.command("@conduct/retry")?;
    let (retry, _runtime) = next_submission(w)?.into_parts();
    assert_eq!(retry.key, original.key);
    assert_eq!(retry.action, original.action);
    assert_eq!(retry.parameters, original.parameters);
    assert_eq!(retry.actor, original.actor);
    assert!(
        disk == w.disk,
        "retry created evidence before actual installation"
    );
    Ok(retry)
}

fn cancellation_and_cold_recovery(f: &PurchaseFixture, base: &IcWorld) -> TestResult {
    use mush_roll::commerce::conduct::protocol::Transition;
    use mush_roll::commerce::rating::execution::ExecutionPhase;
    let mut queued = boot(f, base, &base.snapshot())?;
    queued.engine.set_submission_runtime(Arc::new(Runtime))?;
    queued.command("@conduct/run 1 Transit transit")?;
    let submitted = next_submission(&mut queued)?;
    let request = disposition_retry(&mut queued, submitted)?;
    assert_eq!(
        request.action.key().id.as_str(),
        Transition::Admit.action_id()
    );
    let job = token(FIRST + 20);
    install_submission(&mut queued, request, FIRST + 20)?;
    let stopped = queued.snapshot();
    assert_eq!(
        execution(&queued, job)?.observation().phase,
        ExecutionPhase::Queued
    );
    queued.command("@conduct/cancel")?;
    let (request, _) = next_submission(&mut queued)?.into_parts();
    assert_eq!(
        request.action.key().id.as_str(),
        Transition::Cancel.action_id()
    );
    install_submission(&mut queued, request, FIRST + 21)?;
    pulse(&mut queued)?;
    assert_eq!(
        execution(&queued, job)?.observation().phase,
        ExecutionPhase::Cancelled
    );
    assert!(proof(&mut queued, job).is_err());
    let result: ConductResult =
        serde_json::from_value(one(rows(&queued.disk, "roll", ConductResult::FAMILY))?)?;
    assert!(result.measurement().is_none());
    let mut cancelled_cold = boot(f, &queued, &queued.snapshot())?;
    assert!(proof(&mut cancelled_cold, job).is_err());
    // This independent boot comes from a real admitted nonterminal checkpoint.
    // Only the actual hydration path can mint its stopped recovery binding.
    let mut recovery = boot(f, base, &stopped)?;
    recovery.engine.set_submission_runtime(Arc::new(Runtime))?;
    let (request, _) = next_submission(&mut recovery)?.into_parts();
    assert_eq!(
        request.action.key().id.as_str(),
        Transition::Recover.action_id()
    );
    install_submission(&mut recovery, request, FIRST + 22)?;
    pulse(&mut recovery)?;
    assert_eq!(
        execution(&recovery, job)?.observation().phase,
        ExecutionPhase::Interrupted
    );
    assert!(proof(&mut recovery, job).is_err());
    let result: ConductResult =
        serde_json::from_value(one(rows(&recovery.disk, "roll", ConductResult::FAMILY))?)?;
    assert!(result.measurement().is_none());
    Ok(())
}

/// Drive the actual command, off-thread preparer, shared fair queue, paid
/// executor and signed five-action service to its complete retained result.
/// The server disposition read-half is modeled; it never supplies owner data.
pub(super) fn complete(
    w: &mut IcWorld,
    profile: &str,
    role: &str,
    first: u128,
) -> TestResult<ConductTrialProof> {
    let before = rows(&w.disk, "roll", ConductJob::FAMILY).len();
    w.engine.set_submission_runtime(Arc::new(Runtime))?;
    w.command(&format!("@conduct/run 1 {profile} {role}"))?;
    let deadline = Instant::now() + Duration::from_secs(7200);
    let mut n = first;
    let mut operation = None;
    while Instant::now() < deadline {
        context(w, |ctx| {
            crate::minds::bulk::world::drain(ctx);
            Ok(())
        })?;
        if let Some(id) = conduct_service::fixture_entry(&w.engine.world) {
            w.command(&format!("@conduct/continue {id}"))?;
        }
        if let Some(submitted) = w.engine.take_observed_authoritative_submit() {
            let (request, _runtime_proof) = submitted.into_parts();
            if request.action.key().id.as_str()
                == mush_roll::commerce::conduct::protocol::Transition::Admit.action_id()
            {
                operation = Some(token(n));
            }
            install_submission(w, request, n)?;
            n = n.checked_add(1).ok_or("Conduct operation sequence")?;
        }
        if let Some(op) = operation {
            if let Some(value) = rows(&w.disk, "roll", ConductExecution::FAMILY)
                .into_iter()
                .find(|v| v.get("job") == Some(&json!(op)))
            {
                let execution: ConductExecution = serde_json::from_value(value)?;
                if execution.observation().phase.terminal() {
                    context(w, |ctx| {
                        crate::minds::bulk::world::drain(ctx);
                        Ok(())
                    })?;
                    if let Some(id) = conduct_service::fixture_entry(&w.engine.world) {
                        w.command(&format!("@conduct/continue {id}"))?;
                    }
                    let proof = proof(w, op)?;
                    assert_eq!(proof.profile(), profile);
                    assert_eq!(rows(&w.disk, "roll", ConductJob::FAMILY).len(), before + 1);
                    return Ok(proof);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err("actual Conduct service did not reach a complete result within its hang guard".into())
}

#[test]
#[ignore = "required serialized M7 Conduct gate: one actual shipped-source certificate plus full native role trial"]
fn genuine_safe_conduct_service_result_and_complete_cold_inverse() -> TestResult {
    let (_registry, f, _rating) = filed_parent()?;
    let base = IcWorld::new(&f, &f.snapshot())?;
    cancellation_and_cold_recovery(&f, &base)?;
    let mut w = boot(&f, &base, &base.snapshot())?;
    let retained = complete(&mut w, "Transit", "transit", FIRST + 100)?;
    let saved = w.snapshot();
    let mut cold = boot(&f, &w, &saved)?;
    assert_eq!(
        proof(&mut cold, retained.operation())?.digest(),
        retained.digest()
    );
    let jobs = rows(&saved.disk, "roll", ConductJob::FAMILY);
    let job: ConductJob = serde_json::from_value(one(jobs)?)?;
    assert_eq!(job.operation(), retained.operation());
    let result: ConductResult =
        serde_json::from_value(one(rows(&saved.disk, "roll", ConductResult::FAMILY))?)?;
    assert!(result.measurement().is_some_and(|m| m.passed()));
    super::coverage::qualified::exercise(&f, &w, &saved, &retained)?;
    for remove in [
        ConductJob::FAMILY,
        "roll_conduct_trial_execution",
        ConductResult::FAMILY,
    ] {
        let mut broken = saved.clone();
        broken.disk.retain(|_, v| {
            v.get("family")
                .and_then(|f| f.get("id"))
                .and_then(Value::as_str)
                != Some(remove)
        });
        assert!(
            boot(&f, &w, &broken).is_err(),
            "missing Conduct typed half accepted"
        );
    }
    let mut orphan = saved.clone();
    let mut removed_roots = 0;
    orphan.disk.retain(|(owner, key), value| {
        let typed = value
            .get("family")
            .and_then(|f| f.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|f| f.starts_with("roll_conduct_"));
        let conduct_root = owner == "roll"
            && value
                .pointer("/key/record")
                .and_then(Value::as_str)
                .is_some_and(|r| r.starts_with("v1/conduct_job/"));
        if conduct_root {
            removed_roots += 1;
        }
        !typed
            && !conduct_root
            && !(owner == "tacsim"
                && (key.starts_with("tacsim:conduct:")
                    || key.starts_with("tacsim:mindbulk:account:")))
    });
    assert_eq!(
        removed_roots, 1,
        "actual Conduct root removed for independent physical inverse"
    );
    assert!(
        orphan
            .disk
            .keys()
            .any(|(_, k)| k.contains(":conduct_start"))
    );
    assert!(
        boot(&f, &w, &orphan).is_err(),
        "retained orphan Conduct blobs became empty history"
    );
    Ok(())
}
