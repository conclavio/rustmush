#![cfg(test)]

//! Real changed-policy re-conformance from the parent's acknowledged filed and
//! paid snapshot. No certificate, worker result or recall authority is fabricated.
//! A fixture driver delivers the actual registered continuation commands promptly;
//! it does not change timer state, pending identity or bounded paid work.
use super::*;
use mush_core::submission_observation::{
    SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
};
use mush_core::{AuthoritativeSubmit, OfferExecutionContext, OfferSubmission};
use mush_roll::commerce::{
    FiledRevisionId,
    rating::{RatingPage, RatingReceipt, nonconformance::RatingNonconformance},
    records::FiledRecord,
};
use std::sync::atomic::{AtomicBool, Ordering};
const START: u128 = 38_000_000;
const HEAD: &str = "roll_registry_reconformance_heads";
const CAUSE: &str = "roll_registry_measured_recalls";
struct Proof(Arc<AtomicBool>);
impl SubmissionDispositionReader for Proof {
    fn no_intent(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
struct Runtime;
impl SubmissionRuntime for Runtime {
    fn observe(&self, _: &AuthoritativeSubmit) -> SubmissionRuntimeState {
        let value = Arc::new(AtomicBool::new(false));
        SubmissionRuntimeState::new(Arc::new(Proof(value.clone())), Box::new(value))
    }
}
fn disk(w: &PlayerWorld) -> Disk {
    w.image
        .iter()
        .flat_map(|(ns, data)| {
            // Keep the `log:` chunks boot normalisation moved into `ledger`.
            let kv = data
                .kv
                .iter()
                .map(move |(k, v)| ((ns.to_string(), k.to_string()), v.clone()));
            let ledger = data.ledger.iter().map(move |(key, text)| {
                (
                    (ns.to_string(), key.to_string()),
                    serde_json::from_str::<serde_json::Value>(text).expect("a ledger chunk is JSON"),
                )
            });
            kv.chain(ledger)
        })
        .collect()
}
fn branch(
    f: &PurchaseFixture,
    saved: &Durable,
    cfg: crate::TacsimConfig,
) -> TestResult<(PurchaseFixture, PlayerWorld)> {
    let resources = Resources::reconformance_config(cfg)?;
    let composed = PurchaseFixture::boot(resources, f.accounts.clone(), saved)?;
    let mut world = PlayerWorld::boot(&composed, saved)?;
    world.engine.set_submission_runtime(Arc::new(Runtime))?;
    world.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x92; 32])?,
        [],
    )?);
    Ok((composed, world))
}
fn resolved(
    w: &PlayerWorld,
    request: &AuthoritativeSubmit,
    n: u128,
) -> TestResult<mush_core::ResolvedAuthoritativeOffer> {
    let principal = PrincipalId::Object { id: request.actor };
    let query =
        w.engine
            .legal_action_query(principal.clone(), request.actor, request.subject.clone())?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&request.action)
        .ok_or("actual private Registry offer")?;
    assert_eq!(request.actor, GOD);
    assert_eq!(request.subject, PerceivedRef::SelfActor);
    assert!(request.targets.is_empty());
    assert_eq!(
        request.parameters,
        json!({}),
        "fixed selector has no public authority fields"
    );
    Ok(w.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: RequestUid::from_bytes(n.to_be_bytes()),
            principal: principal.clone(),
            actor: GOD,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: request.action.key().owner.clone(),
                    namespace: CanonicalId::new(request.action.key().id.as_str())?,
                },
                1,
                IdempotencyKey::Caller(request.key.clone()),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: offer.offer(),
            subject: request.subject.clone(),
            targets: request.targets.clone(),
            parameters: request.parameters.clone(),
        },
    )?)
}
fn land(w: &mut PlayerWorld, request: &AuthoritativeSubmit, n: u128) -> TestResult<Value> {
    let (action, resolved, _) = resolved(w, request, n)?.into_authoritative_parts();
    let params = resolved.invocation().parameters().clone();
    let old = disk(w);
    let (prepared, _) = w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(&action, &resolved, token(n))?;
    assert!(
        disk(w) == old,
        "preparation did not publish the measured result"
    );
    let (writes, installation) = prepared.into_parts();
    // The existing exact replay converter retains every module/ledger/object kind.
    let operations = charter_transfer_fixture::module_writes(&writes)?;
    mush_testkit::fold_module_ops_into(&mut w.image, operations);
    w.engine.install_composed_action(installation)?;
    w.commit()?;
    Ok(params)
}
fn pulse(w: &mut PlayerWorld) -> TestResult {
    w.say(GOD, "@reconformance-pulse")?;
    w.say(GOD, "@registry/reconform")?;
    if let Some(id) = crate::minds::bulk::registry_service::fixture_entry(&w.engine.world) {
        w.say(GOD, &format!("@registry/continue {id}"))?;
    }
    // This is the real registered timer/continuation queue, not a direct recall call.
    if let Some(next) = w.engine.sched.next_due() {
        w.engine
            .fire_due(next, &mut Outbox::new(), &mut mush_core::Effects::new());
        w.commit()?;
    }
    Ok(())
}
fn head(w: &PlayerWorld, filed: FiledRevisionId, compat: &DefinitionRef) -> Option<Value> {
    rows(&disk(w), "roll", HEAD).into_iter().find(|v| {
        v.get("filed") == Some(&json!(filed))
            && v.get("compatibility") == Some(&json!(compat.digest()))
    })
}
fn job(w: &PlayerWorld, head: &Value) -> TestResult<RatingJob> {
    let id = head.get("job").ok_or("actual head job")?;
    Ok(serde_json::from_value(
        rows(&disk(w), "roll", "rating_jobs")
            .into_iter()
            .find(|v| v.get("job") == Some(id))
            .ok_or("actual Registry job")?,
    )?)
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ending {
    Cancelled,
    Certified,
    Recalled,
}
async fn run(
    w: &mut PlayerWorld,
    filed: FiledRevisionId,
    manifest: &DefinitionRef,
    n: u128,
    ending: Ending,
) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(7200);
    let mut serial = 0_u128;
    let mut admission = None;
    let mut retained_unknown = false;
    let mut retained_retry = false;
    let mut recall_seen = false;
    loop {
        pulse(w)?;
        if let Some(driven) = w.engine.take_observed_authoritative_submit() {
            assert!(driven.is_observed());
            let (request, proof) = driven.into_parts();
            let absence = *proof
                .ok_or("runtime observation")?
                .downcast::<Arc<AtomicBool>>()
                .map_err(|_| "observation type")?;
            let id = request.action.key().id.as_str();
            assert!(
                id.starts_with("reconformance_") || id == "recall_nonconforming_mind",
                "unrelated service submission"
            );
            if id == "reconformance_admit" && !retained_unknown {
                let before = disk(w);
                // Unknown is not an absence proof and cannot rotate a private ticket.
                for _ in 0..3 {
                    pulse(w)?;
                    assert!(w.engine.take_observed_authoritative_submit().is_none());
                }
                assert!(disk(w) == before, "Unknown cannot admit or recall");
                // Admission NoIntent intentionally discards an unadmitted machine.
                // A real ACK ends this Unknown attempt; only admitted transitions
                // below exercise the existing same-ticket retry contract.
                let params = land(w, &request, n + serial)?;
                serial += 1;
                admission = Some(params);
                if ending == Ending::Cancelled {
                    w.say(GOD, "@registry/cancel")?;
                }
                retained_unknown = true;
            } else if id.starts_with("reconformance_")
                && id != "reconformance_admit"
                && !retained_retry
            {
                let old_key = request.key.clone();
                let before = disk(w);
                absence.store(true, Ordering::Release);
                for _ in 0..20 {
                    pulse(w)?;
                    if let Some(retry) = w.engine.take_observed_authoritative_submit() {
                        let (retry, _) = retry.into_parts();
                        assert_eq!(retry.key, old_key);
                        assert_eq!(retry.action, request.action);
                        assert!(
                            disk(w) == before,
                            "NoIntent does not publish a retained transition"
                        );
                        land(w, &retry, n + serial)?;
                        serial += 1;
                        retained_retry = true;
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                assert!(
                    retained_retry,
                    "admitted NoIntent retries the retained selector"
                );
            } else {
                if id == "recall_nonconforming_mind" {
                    assert!(
                        ending == Ending::Recalled,
                        "an unmeasured outcome did not authorize recall"
                    );
                    let h = head(w, filed, manifest).ok_or("actual measured head")?;
                    assert!(matches!(
                        job(w, &h)?.state(),
                        RatingJobState::Nonconforming { .. }
                    ));
                    assert!(rows(&disk(w), "roll", CAUSE).is_empty());
                    recall_seen = true;
                }
                let params = land(w, &request, n + serial)?;
                serial += 1;
                if id == "reconformance_admit" {
                    admission = Some(params);
                }
            }
        }
        if let Some(h) = head(w, filed, manifest) {
            let current = job(w, &h)?;
            match current.state() {
                RatingJobState::Completed { receipt } if ending == Ending::Certified => {
                    let actual: RatingReceipt = serde_json::from_value(
                        rows(&disk(w), "roll", "rating_receipts")
                            .into_iter()
                            .find(|v| v.pointer("/publication/job") == Some(&json!(receipt.job())))
                            .ok_or("actual successful receipt")?,
                    )?;
                    assert_eq!(actual.reference()?, *receipt);
                    assert_eq!(actual.compatibility().digest(), manifest.digest());
                    assert!(rows(&disk(w), "roll", CAUSE).is_empty());
                    break;
                }
                RatingJobState::Nonconforming { receipt } if ending == Ending::Recalled => {
                    let actual: RatingNonconformance = serde_json::from_value(
                        rows(&disk(w), "roll", "rating_nonconformance")
                            .into_iter()
                            .find(|v| v.pointer("/publication/job") == Some(&json!(receipt.job())))
                            .ok_or("actual measured rejection")?,
                    )?;
                    assert_eq!(actual.reference()?, *receipt);
                    if recall_seen
                        && rows(&disk(w), "roll", CAUSE)
                            .iter()
                            .any(|v| v.get("filed") == Some(&json!(filed)))
                    {
                        break;
                    }
                }
                RatingJobState::Cancelled {} if ending == Ending::Cancelled => {
                    assert!(rows(&disk(w), "roll", CAUSE).is_empty());
                    assert!(rows(&disk(w), "roll", "rating_nonconformance").is_empty());
                    break;
                }
                RatingJobState::Failed { .. }
                | RatingJobState::Cancelled {}
                | RatingJobState::Interrupted {} => {
                    return Err("real re-conformance did not finish measured work".into());
                }
                _ => {}
            }
        }
        if Instant::now() >= deadline {
            return Err("bounded actual re-conformance did not complete".into());
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(admission.is_some());
    assert!(retained_retry);
    Ok(())
}
async fn scenario(
    f: &PurchaseFixture,
    saved: &Durable,
    expensive: &PurchaseFixture,
    expensive_saved: &Durable,
) -> TestResult {
    let mut all_tasks = Vec::new();
    let outcome = async {
        let filed: FiledRecord =
            serde_json::from_value(one(rows(&saved.disk, "roll", "roll_mind_filed"))?)?;
        let id = filed.content().id();
        let old = rows(&saved.disk, "roll", "rating_receipts");
        let paid = rows(&saved.disk, "roll", "roll_mind_paid_term");
        let mut cfg = f.resources.config();
        cfg.minds.bench_eval_ops = cfg.minds.bench_eval_ops.checked_add(1).ok_or("policy")?;
        let (configured, mut w) = branch(f, saved, cfg.clone())?;
        all_tasks.push(w.tasks.clone());
        let manifest = configured.resources.reference()?;
        assert_ne!(manifest, f.resources.reference()?);
        // Birth evidence alone is historical, so a current paid selection refuses until
        // an actual completed current-policy head is published.
        let team = w.say(BUYER, "SIM TEAM red")?;
        assert!(PlayerWorld::private_text(&team, BUYER).contains("red slot 1"));
        w.say(BUYER, "SIM AI 1")?;
        w.say(BUYER, "SIM UNIT heavy_mk1")?;
        let events = w.say(
            BUYER,
            &format!("SIM MIND filed:{} PROFILE Default", id.get()),
        )?;
        assert!(!PlayerWorld::private_text(&events, BUYER).contains("Your slot will carry"));
        let tasks = w.tasks.clone();
        drop(w);
        tasks.shutdown(Duration::from_secs(5)).await;
        let (_, mut w) = branch(f, saved, cfg.clone())?;
        all_tasks.push(w.tasks.clone());
        run(&mut w, id, &manifest, START, Ending::Cancelled).await?;
        let cancelled = Durable::capture(&w.engine, &disk(&w));
        let cancelled_head = head(&w, id, &manifest).ok_or("cancelled current head")?;
        let cancelled_job = job(&w, &cancelled_head)?.job();
        let tasks = w.tasks.clone();
        drop(w);
        tasks.shutdown(Duration::from_secs(5)).await;
        let mut w = PlayerWorld::boot(&configured, &cancelled)?;
        all_tasks.push(w.tasks.clone());
        w.engine.set_submission_runtime(Arc::new(Runtime))?;
        w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x92; 32])?,
            [],
        )?);
        run(&mut w, id, &manifest, START + 100_000, Ending::Certified).await?;
        let current_head = head(&w, id, &manifest).ok_or("successful current head")?;
        let execution = rows(&disk(&w), "roll", "rating_executions")
            .into_iter()
            .find(|v| v.get("job") == current_head.get("job"))
            .ok_or("actual successful execution")?;
        assert_eq!(
            execution.pointer("/admission/reconformance/previous_attempt"),
            Some(&json!(cancelled_job))
        );
        let result = Durable::capture(&w.engine, &disk(&w));
        w.select()?;
        for value in &old {
            assert!(rows(&result.disk, "roll", "rating_receipts").contains(value));
        }
        assert!(rows(&result.disk, "roll", "roll_mind_paid_term") == paid);
        // A fresh sale must freeze the actual newly current certificate, while
        // the earlier paid grant retains its immutable birth proof.
        let mut sale = configured.reboot(&result)?;
        sale.engine.world.move_to(GOD, ROOM)?;
        sale.engine.world.move_to(BUYER, ROOM)?;
        // Complete the fixture's physical-arrival snapshot before the actual
        // venue-gated purchase preparation; no dirty-world failure is hidden.
        sale = sale.reboot(&sale.snapshot())?;
        let n = START + 500_000;
        sale.perform(
            &sale.config.quotes.listing_action()?,
            GOD,
            n,
            json!({"owner_licence":"1"}),
        )?;
        sale.perform(
            &sale.config.quotes.quote_action()?,
            BUYER,
            n + 1,
            json!({"listing":token(n)}),
        )?;
        let [fund, grant, _] = sale.config.purchases.actions()?;
        sale.perform(&fund, BUYER, n + 2, json!({"quote":token(n+1)}))?;
        sale.perform(&grant, BUYER, n + 3, json!({"operation":token(n+2)}))?;
        let new_allocation = sale
            .rows("roll", "roll_mind_paid_purchase")
            .into_iter()
            .find(|v| v.get("operation") == Some(&json!(token(n + 2))))
            .ok_or("actual new grant")?;
        assert_eq!(
            new_allocation.pointer("/rating/job"),
            current_head.get("job")
        );
        assert_ne!(
            new_allocation.get("rating"),
            serde_json::to_value(&filed)?.get("rating")
        );
        let cold_sale = sale.reboot(&sale.snapshot())?;
        assert!(
            rows(&cold_sale.disk, "roll", "roll_mind_paid_term")
                .iter()
                .any(|v| paid.contains(v))
        );
        let mut positive = PlayerWorld::boot(&configured, &result)?;
        all_tasks.push(positive.tasks.clone());
        positive.select()?;
        let tasks = positive.tasks.clone();
        drop(positive);
        tasks.shutdown(Duration::from_secs(5)).await;
        let tasks = w.tasks.clone();
        drop(w);
        tasks.shutdown(Duration::from_secs(5)).await;
        for family in [HEAD, "rating_executions"] {
            let mut corrupt = result.clone();
            assert!(remove_family(&mut corrupt, "roll", family) > 0);
            assert!(
                PlayerWorld::boot(&configured, &corrupt).is_err(),
                "missing re-conformance history refused"
            );
        }
        // The second source first earned its own real certificate and paid grant.
        // Its retained actual nonkernel meter proves that 100 operations is smaller
        // than the accepted workload; the policy is valid and the source is unchanged.
        let filed: FiledRecord =
            serde_json::from_value(one(rows(&expensive_saved.disk, "roll", "roll_mind_filed"))?)?;
        let id = filed.content().id();
        let paid = rows(&expensive_saved.disk, "roll", "roll_mind_paid_term");
        cfg = expensive.resources.config();
        cfg.minds.classes = serde_json::from_value(
            json!({"0":{"ops":100},"I":{"ops":100},"II":{"ops":100},"III":{"ops":100}}),
        )?;
        let (rejected_config, mut w) = branch(expensive, expensive_saved, cfg)?;
        all_tasks.push(w.tasks.clone());
        let manifest = rejected_config.resources.reference()?;
        run(&mut w, id, &manifest, START + 1_000_000, Ending::Recalled).await?;
        let rejected = Durable::capture(&w.engine, &disk(&w));
        assert!(rows(&rejected.disk, "roll", "roll_mind_paid_term") == paid);
        let causes = rows(&rejected.disk, "roll", CAUSE);
        assert_eq!(causes.len(), 1);
        let status = one(rows(&rejected.disk, "roll", "roll_mind_status"))?;
        assert_eq!(
            status.get("recalled_by"),
            causes.first().and_then(|v| v.get("operation"))
        );
        let cold = PlayerWorld::boot(&rejected_config, &rejected)?;
        all_tasks.push(cold.tasks.clone());
        let tasks = cold.tasks.clone();
        drop(cold);
        tasks.shutdown(Duration::from_secs(5)).await;
        for family in [CAUSE, "rating_nonconformance", HEAD] {
            let mut corrupt = rejected.clone();
            assert!(remove_family(&mut corrupt, "roll", family) > 0);
            assert!(PlayerWorld::boot(&rejected_config, &corrupt).is_err());
        }
        let tasks = w.tasks.clone();
        drop(w);
        tasks.shutdown(Duration::from_secs(5)).await;
        Ok(())
    }
    .await;
    for tasks in all_tasks {
        tasks.shutdown(Duration::from_secs(5)).await;
    }
    outcome
}
fn expensive_filed() -> TestResult<(PurchaseFixture, Durable)> {
    const SOURCE: &str = "fn think(v,c) { for i in 0..40 { this.work=t_ms()+i; } stock::rules(); speed(0); this.landed=t_ms(); }";
    let mut registry = Fixture::with_source(SOURCE, "I")?;
    let rating = complete_registry(&mut registry)?;
    let mut measured_above = false;
    for value in rows(&registry.disk, "roll", "rating_pages") {
        let page: RatingPage = serde_json::from_value(value)?;
        measured_above |= page.content().rows().iter().any(|row| {
            !matches!(
                row.key().kind(),
                mush_roll::commerce::rating::CaseKind::Kernel { .. }
            ) && row.meter().max_ops() > 100
        });
    }
    assert!(
        measured_above,
        "the actual completed source must exceed the later legal cap"
    );
    let mut world = World::new_minimal();
    assert_eq!(
        world.create(
            ObjKind::Player,
            "Actual reconformance buyer",
            GOD,
            Some(ROOM)
        )?,
        BUYER
    );
    world.set_now(1000);
    {
        let mut accounts = registry.accounts.values.lock().map_err(module_error)?;
        registry.accounts.epoch.invalidate();
        accounts.insert(BUYER, AccountId::mint("reconformance-buyer", 2)?);
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
    f.perform(&reserve, GOD, START + 2_000_000, json!({}))?;
    let revision = registry.revision.reference();
    let filing = f.filing.clone();
    f.perform(&filing,GOD,START+2_000_001,json!({"shell_request":RequestUid::from_bytes((START+2_000_000).to_be_bytes()),"mind":revision.key.mind,"revision":revision.key.number,"revision_digest":revision.digest,"rating_job":rating.job(),"rating_id":rating.id(),"rating_digest":rating.receipt_digest(),"payer":0,"charter_name":"Measured budget candidate"}))?;
    f.perform(
        &f.config.quotes.listing_action()?,
        GOD,
        START + 2_000_002,
        json!({"owner_licence":"1"}),
    )?;
    f.perform(
        &f.config.quotes.quote_action()?,
        BUYER,
        START + 2_000_003,
        json!({"listing":token(START+2_000_002)}),
    )?;
    let [fund, grant, _] = f.config.purchases.actions()?;
    f.perform(
        &fund,
        BUYER,
        START + 2_000_004,
        json!({"quote":token(START+2_000_003)}),
    )?;
    f.perform(
        &grant,
        BUYER,
        START + 2_000_005,
        json!({"operation":token(START+2_000_004)}),
    )?;
    let saved = f.snapshot();
    Ok((f, saved))
}
pub(super) fn exercise(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let (expensive, expensive_saved) = expensive_filed()?;
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?
        .block_on(super::fork_tests::boxed_story_phase(
            "reconformance.scenario",
            || scenario(f, saved, &expensive, &expensive_saved),
        )?)
}
