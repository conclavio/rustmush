#![cfg(test)]

//! Actual B2-saved fork -> Registry providers/owners/worker. The first parent
//! certificate, paid grant, fork and save have already run through their real
//! producers. This file constructs no grant, result row or parent capability.
use super::*;
#[path = "dependent_owner_integration_tests.rs"]
mod dependent_owner_integration_tests;
#[path = "entitlement_integration_tests.rs"]
mod entitlement_integration_tests;
use crate::minds::bulk::{executor::Machine, world as bulk_world};
use mush_roll::commerce::{
    paid::PaidForm,
    rating::{RatingReceipt, RatingRef},
    records::{RevisionPolicy, canonical_duplicate_digest},
};

struct Input {
    ticket: TicketId,
    reference: mush_roll::RevisionRef,
    account: AccountId,
    start: [u8; 32],
    machine: Box<dyn Machine>,
}
fn artifacts(world: &mut PlayerWorld) -> TestResult<&mut Artifacts> {
    world
        .engine
        .world
        .module_state_mut::<Artifacts>()
        .ok_or_else(|| "actual artifacts".into())
}
fn quota(world: &PlayerWorld) -> TestResult<&BulkStore> {
    world
        .engine
        .world
        .module_state::<BulkStore>()
        .ok_or_else(|| "actual quota".into())
}
fn source(world: &PlayerWorld) -> TestResult<Revision> {
    let mind = roll(world)?.mind(BUYER, "child")?;
    Ok(roll(world)?
        .revision(BUYER, mind.latest().ok_or("actual saved child")?)?
        .clone())
}
fn input(world: &mut PlayerWorld) -> TestResult<Input> {
    let revision = source(world)?;
    assert!(revision.require_original_access().is_err());
    let account = bulk_world::account(&world.engine.world, BUYER)?;
    let nonce = quota(world)?.next_nonce(account)?;
    let text = mush_roll::SourceAdapter::read_source(
        world
            .engine
            .world
            .module_state::<TacsimStore>()
            .ok_or("actual sources")?,
        revision.source(),
    )?;
    let prepared =
        world
            .factory
            .prepare(account, BUYER, nonce, revision.clone(), Arc::from(text))?;
    let start = prepared.data.start.descriptor().sha256;
    let ticket = artifacts(world)?.registry.admit(prepared.data)?;
    Ok(Input {
        ticket,
        reference: revision.reference(),
        account,
        start,
        machine: prepared.machine,
    })
}
fn action(world: &PlayerWorld, transition: Transition) -> TestResult<DefinitionRef> {
    world
        .engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|e| e.reference())
        .find(|r| {
            r.key().owner.as_str() == "roll"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == transition.action_id()
        })
        .cloned()
        .ok_or_else(|| "actual Registry action".into())
}
fn admit_request(input: &Input, number: u128, actor: ObjId) -> TestResult<ActionInvocation> {
    crate::minds::bulk::registry_integration_tests::original_purchase::request(
        number,
        actor,
        "roll",
        json!({"ticket":input.ticket.get(),"mind":input.reference.key.mind,
        "revision":input.reference.key.number,"revision_digest":input.reference.digest}),
    )
}
fn prepare_admit(
    world: &mut PlayerWorld,
    input: &Input,
    number: u128,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    let action = action(world, Transition::Admit)?;
    Ok(world
        .engine
        .prepare_registered_composed_authoritative_action(
            &action,
            &admit_request(input, number, BUYER)?,
            token(number),
        )?)
}
fn prepare_transition(
    world: &mut PlayerWorld,
    binding: Binding,
    ticket: TicketId,
    number: u128,
    transition: Transition,
) -> TestResult<PreparedComposedTxn> {
    let action = action(world, transition)?;
    Ok(world
        .engine
        .prepare_registered_composed_authoritative_action(
            &action,
            &crate::minds::bulk::registry_integration_tests::original_purchase::request(
                number,
                BUYER,
                "roll",
                json!({"ticket":ticket.get(),"job":binding.job}),
            )?,
            token(number),
        )?
        .0)
}
fn ack(
    world: &mut PlayerWorld,
    prepared: PreparedComposedTxn,
    ticket: TicketId,
    job: CommitToken,
    number: u128,
    transition: Transition,
) -> TestResult {
    let (writes, installation) = prepared.into_parts();
    // The same exact complete union is retained before the fixture sees ACK.
    let operations = writes
        .into_iter()
        .map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => Ok(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: key.to_string().into(),
                    value,
                },
            }),
            ReplayWrite::ModuleDelete { namespace, key } => Ok(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::Del {
                    key: key.to_string().into(),
                },
            }),
            _ => Err("Registry unexpectedly wrote outside its exact module families"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    mush_testkit::fold_module_ops_into(&mut world.image, operations);
    world.engine.install_composed_action(installation)?;
    world.commit()?;
    let ack = artifacts(world)?
        .registry
        .take_acknowledged()
        .ok_or("actual Registry ACK")?;
    assert_eq!(
        (ack.ticket, ack.job, ack.operation, ack.transition),
        (ticket, job, token(number), transition)
    );
    assert!(artifacts(world)?.registry.get(ticket).is_err());
    Ok(())
}
fn run(world: &PlayerWorld, binding: Binding) -> TestResult<Run> {
    Ok(quota(world)?
        .run(binding.account, binding.character, binding.run)?
        .clone())
}
fn typed<T: serde::de::DeserializeOwned>(
    world: &PlayerWorld,
    family: &str,
    job: CommitToken,
) -> TestResult<T> {
    let candidates = rows(&disk(world), "roll", family);
    let row = candidates
        .into_iter()
        .find(|row| {
            row.get("job") == Some(&json!(job))
                || row.pointer("/publication/job") == Some(&json!(job))
        })
        .ok_or("actual job-keyed row")?;
    Ok(serde_json::from_value(row)?)
}
fn stage_admit(
    world: &mut PlayerWorld,
    input: &Input,
    number: u128,
    prepared: PreparedComposedTxn,
    output: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let action = action(world, Transition::Admit)?;
    let inv = admit_request(input, number, BUYER)?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        output.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(number),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([83; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([84; 32]),
        action,
        provider: OwnerId::new("roll")?,
        cause: mush_core::action::PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: prepared.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(number))),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?;
    let guards = world
        .engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())?
        .finish()?;
    let _ = world.engine.world.take_dirty();
    world
        .engine
        .stage_authoritative_action(prepared, guards, bundle)?;
    Ok(())
}
fn expiry(world: &PlayerWorld) -> TestResult<i64> {
    let term: PaidTermRecord =
        serde_json::from_value(one(rows(&disk(world), "roll", "roll_mind_paid_term"))?)?;
    Ok(term.window().grace_end().get())
}
async fn public_start(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_marked(f, saved)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let revision = source(&world)?.reference();
        let before = disk(&world);
        let events = world.say(
            BUYER,
            &format!(
                "@registry/rate {}/{}",
                revision.key.mind, revision.key.number
            ),
        )?;
        assert!(PlayerWorld::private_text(&events, BUYER).contains("Registry preparation queued."));
        let service = bulk_world::service(&world.engine.world)?;
        let runtime = service.0.lock().map_err(module_error)?;
        assert_eq!(
            runtime.preparing.len(),
            1,
            "actual public command entered the shared preparation worker"
        );
        drop(runtime);
        assert_eq!(
            disk(&world),
            before,
            "public preparation has no quota/typed publication"
        );
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}

async fn freshness(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    for mode in ["default", "before", "stage", "take", "clock", "foreign"] {
        let mut world = if mode == "default" {
            PlayerWorld::boot_forks(f, saved, true)?
        } else {
            PlayerWorld::boot_marked(f, saved)?
        };
        let tasks = world.tasks.clone();
        let result: TestResult = async {
            let input = input(&mut world)?;
            let number = 5_100_000;
            let before = disk(&world);
            if mode == "before" {
                world.engine.world.set_now(expiry(&world)?);
            }
            if matches!(mode, "default" | "before") {
                let command = format!(
                    "@registry/rate {}/{}",
                    input.reference.key.mind, input.reference.key.number
                );
                let events = world.say(BUYER, &command)?;
                assert!(!PlayerWorld::private_text(&events, BUYER).contains("preparation queued"));
                let service = bulk_world::service(&world.engine.world)?;
                assert!(service.0.lock().map_err(module_error)?.preparing.is_empty());
                assert!(prepare_admit(&mut world, &input, number).is_err());
            } else if mode == "foreign" {
                let inv = admit_request(&input, number, GOD)?;
                let action = action(&world, Transition::Admit)?;
                assert!(
                    world
                        .engine
                        .prepare_registered_composed_authoritative_action(
                            &action,
                            &inv,
                            token(number)
                        )
                        .is_err()
                );
            } else {
                let (prepared, output) = prepare_admit(&mut world, &input, number)?;
                assert_eq!(disk(&world), before);
                assert!(quota(&world)?.active(input.account).is_none());
                if mode == "take" {
                    stage_admit(&mut world, &input, number, prepared, output)?;
                    world.engine.world.set_now(expiry(&world)?);
                    assert!(world.engine.take_staged_action().is_err());
                } else {
                    if mode == "clock" {
                        let stamp = world.engine.world.world_ic_now();
                        let epoch = world.engine.world.next_world_timeline_epoch(2)?;
                        world.engine.world.append_world_timeline_epoch(epoch)?;
                        assert_eq!(world.engine.world.world_ic_now().at, stamp.at);
                        assert_ne!(world.engine.world.world_ic_now().clock, stamp.clock);
                    } else {
                        world.engine.world.set_now(expiry(&world)?);
                    }
                    assert!(stage_admit(&mut world, &input, number, prepared, output).is_err());
                }
            }
            assert_eq!(disk(&world), before, "{mode}: no quota/root/typed write");
            assert!(quota(&world)?.active(input.account).is_none());
            assert!(artifacts(&mut world)?.registry.get(input.ticket).is_ok());
            assert!(!world.engine.action_state().reservation_lost());
            Ok(())
        }
        .await;
        tasks.shutdown(Duration::from_secs(5)).await;
        result?;
    }
    Ok(())
}
fn admit(world: &mut PlayerWorld, number: u128) -> TestResult<(Binding, Ready)> {
    let input = input(world)?;
    let (prepared, _) = prepare_admit(world, &input, number)?;
    assert!(quota(world)?.active(input.account).is_none());
    ack(
        world,
        prepared,
        input.ticket,
        token(number),
        number,
        Transition::Admit,
    )?;
    let binding = Binding {
        job: token(number),
        account: input.account,
        character: BUYER,
        run: quota(world)?.active(input.account).ok_or("actual run")?,
    };
    let job: RatingJob = typed(world, "rating_jobs", binding.job)?;
    assert_eq!(job.subject().revision().revision(), input.reference);
    assert!(source(world)?.require_original_access().is_err());
    Ok((
        binding,
        Ready::registry(binding, input.start, input.machine),
    ))
}
/// Make a genuinely saved distinct author source after the existing B2
/// same-source roundtrip assertions. The comment changes no runtime policy,
/// schema, profiles, rules, budgets or acceptance criteria.
async fn distinct_saved(f: &PurchaseFixture, saved: &Durable) -> TestResult<Durable> {
    let mut world = PlayerWorld::boot_marked(f, saved)?;
    let tasks = world.tasks.clone();
    let result: TestResult<Durable> = async {
        let before = source(&world)?;
        let parent: PaidTermRecord =
            serde_json::from_value(one(rows(&disk(&world), "roll", "roll_mind_paid_term"))?)?;
        let RevisionPolicy::Pin {
            revision: parent_identity,
            ..
        } = parent.policy()
        else {
            return Err("actual parent Pin".into());
        };
        let reference = parent_identity.revision();
        let parent_revision = roll(&world)?.revision(reference.key.character, reference.key)?;
        let parent_duplicate = canonical_duplicate_digest(parent_revision)?;
        assert_eq!(canonical_duplicate_digest(&before)?, parent_duplicate);
        let selector = one(rows(&disk(&world), "roll", "roll_mind_draft_parent"))?;
        let events = super::line(
            &mut world,
            "@mind/insert child 1=// independently authored child annotation",
        )?;
        assert!(PlayerWorld::private_text(&events, BUYER).contains("Working source updated."));
        let edited = roll(&world)?.mind(BUYER, "child")?.clone();
        let job = super::submit(&mut world, true, true)?;
        super::ready(&world, job).await?;
        assert_eq!(
            roll(&world)?.mind(BUYER, "child")?,
            &edited,
            "worker cannot publish"
        );
        let events = super::drain(&mut world, job).await?;
        match super::terminal(&events)? {
            MindResponse::Saved { revision, .. } => {
                assert_eq!(source(&world)?.reference().key.number, revision);
            }
            other => {
                return Err(format!("distinct source was not actually saved: {other:?}").into());
            }
        }
        let after = source(&world)?;
        assert_ne!(after.reference(), before.reference());
        assert_ne!(after.source(), before.source());
        assert_ne!(canonical_duplicate_digest(&after)?, parent_duplicate);
        assert_eq!(after.envelope(), before.envelope());
        assert_eq!(
            serde_json::to_value(&after)?.get("provenance"),
            serde_json::to_value(&before)?.get("provenance")
        );
        assert_eq!(
            one(rows(&disk(&world), "roll", "roll_mind_draft_parent"))?,
            selector
        );
        assert!(after.require_original_access().is_err());
        let expected = mush_roll::RevisionAdapter::normalize_source(
            &crate::minds::authoring::RevisionRules,
            &format!("// independently authored child annotation\n{SOURCE}"),
        )?;
        assert_eq!(after.source(), mush_roll::source_digest(&expected));
        let actual = mush_roll::SourceAdapter::read_source(
            world
                .engine
                .world
                .module_state::<TacsimStore>()
                .ok_or("actual source owner")?,
            after.source(),
        )?;
        assert_eq!(actual, expected);
        assert!(world.engine.world.now() < expiry(&world)?);
        Ok(capture(&world, &disk(&world)))
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}

async fn complete_eligible(
    f: &PurchaseFixture,
    saved: &Durable,
) -> TestResult<(Durable, RatingRef)> {
    let mut world = PlayerWorld::boot_marked(f, saved)?;
    let tasks = world.tasks.clone();
    let result: TestResult<(Durable, RatingRef)> = async {
        let (binding, mut ready) = admit(&mut world, 5_200_000)?;
        let subject: RatingJob = typed(&world, "rating_jobs", binding.job)?;
        // Keep this one full genuine certificate eligible for the dependent
        // filing child. Lapse settlement uses a separate actual paid quantum.
        assert!(world.engine.world.now() < expiry(&world)?);
        let executor = Executor::with_wake(&BulkConfig::default(), None)?;
        let deadline = Instant::now() + Duration::from_secs(7_200);
        let mut number = 5_200_001_u128;
        let mut measured = 0_u64;
        let mut quanta = 0_u64;
        loop {
            if Instant::now() >= deadline {
                return Err("actual marked Registry liveness guard".into());
            }
            let ticket = artifacts(&mut world)?
                .registry
                .grant(binding, ready.next_grant_ms())?;
            let prepared =
                prepare_transition(&mut world, binding, ticket, number, Transition::Grant)?;
            assert!(run(&world, binding)?.flight.is_none());
            ack(
                &mut world,
                prepared,
                ticket,
                binding.job,
                number,
                Transition::Grant,
            )?;
            let lease = world
                .engine
                .world
                .module_state_mut::<BulkStore>()
                .ok_or("quota")?
                .take_registry_lease(token(number))
                .ok_or("actual paid lease")?;
            number += 1;
            executor
                .submit(lease, ready)
                .map_err(|(error, _, _)| error)?;
            let mut done = loop {
                if let Some(done) = executor.poll() {
                    break done;
                }
                if Instant::now() >= deadline {
                    return Err("actual marked worker liveness guard".into());
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            };
            measured = measured
                .checked_add(done.receipt.actual_unit_ms)
                .ok_or("measured charge overflow")?;
            let next = done
                .ready
                .take()
                .ok_or("Machine retained through actual ACK")?;
            let settlement = done
                .registry_settlement
                .take()
                .ok_or("actual measured settlement")??;
            let ticket = artifacts(&mut world)?
                .registry
                .insert_settlement(settlement)
                .map_err(|(error, _)| error)?;
            let before = run(&world, binding)?;
            let prepared =
                prepare_transition(&mut world, binding, ticket, number, Transition::Settle)?;
            assert_eq!(run(&world, binding)?, before);
            ack(
                &mut world,
                prepared,
                ticket,
                binding.job,
                number,
                Transition::Settle,
            )?;
            number += 1;
            quanta += 1;
            let after = run(&world, binding)?;
            assert_eq!(after.charged_unit_ms, measured);
            if after.status == Status::Complete {
                drop(next);
                break;
            }
            if after.status.terminal() {
                return Err(format!(
                    "actual marked Registry ended {:?} after {quanta} quanta",
                    after.status
                )
                .into());
            }
            ready = next;
        }
        let receipt: RatingReceipt = typed(&world, "rating_receipts", binding.job)?;
        assert_eq!(receipt.subject(), subject.subject());
        assert_eq!(receipt.reference()?.job(), binding.job);
        assert_eq!(
            receipt.subject().revision().revision(),
            source(&world)?.reference()
        );
        assert!(
            source(&world)?.require_original_access().is_err(),
            "rating never erases lineage"
        );
        // Retain the exact eligible snapshot without rewriting its clock,
        // receipt or immutable parent window. This is the later child's input.
        assert!(world.engine.world.now() < expiry(&world)?);
        let certified = capture(&world, &disk(&world));
        let reference = receipt.reference()?;
        // A separate later world state proves expiry is not boot corruption.
        world.engine.world.set_now(expiry(&world)?);
        let terminal = capture(&world, &disk(&world));
        let cold = PlayerWorld::boot_marked(f, &terminal)?;
        assert_eq!(run(&cold, binding)?.status, Status::Complete);
        assert_eq!(
            typed::<RatingReceipt>(&cold, "rating_receipts", binding.job)?,
            receipt
        );
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        // A fresh new job remains refused. This is not a cached rating ACL.
        let input = input(&mut world)?;
        let before = disk(&world);
        assert!(prepare_admit(&mut world, &input, number).is_err());
        assert_eq!(disk(&world), before);
        eprintln!(
            "M7_MARKED_REGISTRY_CERTIFICATE job={} quanta={quanta} unit_ms={measured}",
            binding.job
        );
        Ok((certified, reference))
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
async fn recover_after_lapse(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_marked(f, saved)?;
    let (binding, ready) = admit(&mut world, 5_300_000)?;
    let original: RatingJob = typed(&world, "rating_jobs", binding.job)?;
    world.engine.world.set_now(expiry(&world)?);
    // A real granted lease and measured answer settle after fresh parent
    // eligibility disappears. Both Grant and Settle retain the owned subject.
    let ticket = artifacts(&mut world)?
        .registry
        .grant(binding, ready.next_grant_ms())?;
    let before = run(&world, binding)?;
    let prepared = prepare_transition(&mut world, binding, ticket, 5_300_001, Transition::Grant)?;
    assert_eq!(run(&world, binding)?, before);
    ack(
        &mut world,
        prepared,
        ticket,
        binding.job,
        5_300_001,
        Transition::Grant,
    )?;
    let lease = world
        .engine
        .world
        .module_state_mut::<BulkStore>()
        .ok_or("quota")?
        .take_registry_lease(token(5_300_001))
        .ok_or("actual expired-parent lease")?;
    let executor = Executor::with_wake(&BulkConfig::default(), None)?;
    executor
        .submit(lease, ready)
        .map_err(|(error, _, _)| error)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut done = loop {
        if let Some(done) = executor.poll() {
            break done;
        }
        if Instant::now() >= deadline {
            return Err("actual expired-parent quantum liveness guard".into());
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    let charged = done.receipt.actual_unit_ms;
    assert!(charged > 0, "a real native quantum was measured");
    let ready = done
        .ready
        .take()
        .ok_or("actual continuation retained through ACK")?;
    let settlement = done
        .registry_settlement
        .take()
        .ok_or("actual measured answer")??;
    let ticket = artifacts(&mut world)?
        .registry
        .insert_settlement(settlement)
        .map_err(|(error, _)| error)?;
    let before = run(&world, binding)?;
    let prepared = prepare_transition(&mut world, binding, ticket, 5_300_002, Transition::Settle)?;
    assert_eq!(run(&world, binding)?, before);
    ack(
        &mut world,
        prepared,
        ticket,
        binding.job,
        5_300_002,
        Transition::Settle,
    )?;
    let paid = run(&world, binding)?;
    assert_eq!(paid.charged_unit_ms, charged);
    assert_eq!(paid.status, Status::Running);
    assert!(paid.flight.is_none() && paid.result.is_none());
    let retained = capture(&world, &disk(&world));
    drop(ready);
    world.tasks.shutdown(Duration::from_secs(5)).await;
    drop(world);
    // Real Tacsim hydration alone captures StoppedBinding on its never-run
    // service. No timeout or caller-created worker receipt proves stoppage.
    let mut cold = PlayerWorld::boot_marked(f, &retained)?;
    let tasks = cold.tasks.clone();
    let result: TestResult = async {
        let service = bulk_world::service(&cold.engine.world)?;
        let stopped = service
            .0
            .lock()
            .map_err(module_error)?
            .recovery
            .take_available(|b| b == binding)
            .ok_or("actual cold stopped binding")?;
        let ticket = artifacts(&mut cold)?.registry.recover_stopped(&stopped)?;
        let prepared =
            prepare_transition(&mut cold, binding, ticket, 5_300_003, Transition::Recover)?;
        ack(
            &mut cold,
            prepared,
            ticket,
            binding.job,
            5_300_003,
            Transition::Recover,
        )?;
        let after: RatingJob = typed(&cold, "rating_jobs", binding.job)?;
        assert_eq!(after.subject(), original.subject());
        assert!(matches!(after.state(), RatingJobState::Interrupted {}));
        assert_eq!(run(&cold, binding)?.status, Status::Interrupted);
        assert_eq!(
            run(&cold, binding)?.charged_unit_ms,
            charged,
            "recovery preserves settled measured charge"
        );
        assert!(run(&cold, binding)?.result.is_none());
        let again = PlayerWorld::boot_marked(f, &capture(&cold, &disk(&cold)))?;
        assert_eq!(run(&again, binding)?.status, Status::Interrupted);
        again.tasks.shutdown(Duration::from_secs(5)).await;
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
pub(super) async fn exercise(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    story_phase("marked_registry.public_start", || public_start(f, saved))?.await?;
    story_phase("marked_registry.freshness", || freshness(f, saved))?.await?;
    story_phase("marked_registry.recovery", || recover_after_lapse(f, saved))?.await?;
    let parent: PaidTermRecord =
        serde_json::from_value(one(rows(&saved.disk, "roll", "roll_mind_paid_term"))?)?;
    assert!(matches!(
        parent.terms().form(),
        PaidForm::Open {} | PaidForm::OpenResale {}
    ));
    // The existing Open branch proves minimum private-filing permission. D
    // separately needs a genuinely acquired OpenResale parent; its immutable
    // acquisition cannot be upgraded or forged from the Open certificate.
    // Each form runs one full certificate, reused by all downstream scenarios.
    {
        let distinct = story_phase("marked_registry.distinct_saved", || {
            distinct_saved(f, saved)
        })?
        .await?;
        let (certified, reference) = story_phase("marked_registry.certificate", || {
            complete_eligible(f, &distinct)
        })?
        .await?;
        let receipt: RatingReceipt = serde_json::from_value(
            rows(&certified.disk, "roll", "rating_receipts")
                .into_iter()
                .find(|row| row.pointer("/publication/job") == Some(&json!(reference.job())))
                .ok_or("actual retained eligible marked certificate")?,
        )?;
        assert_eq!(receipt.reference()?, reference);
        if matches!(parent.terms().form(), PaidForm::Open {}) {
            story_phase("marked_registry.dependent", || {
                dependent_owner_integration_tests::exercise(f, &certified, &reference)
            })?
            .await?;
        } else {
            story_phase("marked_registry.entitlement", || {
                entitlement_integration_tests::exercise(f, &certified, &reference)
            })?
            .await?;
        }
    }
    Ok(())
}
