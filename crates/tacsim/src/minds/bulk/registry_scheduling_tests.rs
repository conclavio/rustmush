#![cfg(test)]
//! Real service/scheduler custody, not a completed certification or server writer proof.
use super::super::registry_tickets::{Ticket, TicketKind};
use super::*;
use mush_core::{
    AuthoritativeSubmit, GuardSet, OfferExecutionContext, OfferKeyRing, OfferSecret,
    OfferSubmission, ScheduledKind,
    action::{PreparedAuthoritativeAction, PreparedCausalRef},
    authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft},
    submission_observation::DrivenSubmission,
};

struct Planned {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
    prepared: (PreparedComposedTxn, PreparedAuthoritativeAction),
    commit: CommitToken,
    ticket_id: TicketId,
    ticket: Arc<Ticket>,
    _driver: Option<Box<dyn std::any::Any + Send>>,
}
struct ScheduledRegistry {
    fixture: Box<Fixture>,
    world: Box<mush_testkit::TestWorld>,
    serial: u128,
}
impl ScheduledRegistry {
    #[inline(never)]
    fn new() -> TestResult<Box<Self>> {
        let fixture = Fixture::new()?;
        let mut engine = registry_boot_tests::real_engine(&fixture)?;
        load(&mut engine, &fixture.disk)?;
        engine.set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x46; 32])?, [])?);
        // Only the ordinary Core timer driver uses this test time. Registry's
        // retry clock and real-day quota still use their unchanged SystemTime.
        engine.set_now(0);
        let mut world = Box::new(mush_testkit::TestWorld::from_engine(*engine));
        let reference = fixture.revision.reference();
        world.run(
            GOD,
            &format!(
                "@registry/rate {}/{}",
                reference.key.mind, reference.key.number
            ),
        );
        Ok(Box::new(Self {
            fixture,
            world,
            serial: 100,
        }))
    }
    fn pulse(&mut self) -> TestResult {
        // This existing fixture command calls production world::drain. Every
        // continuation still enters through the registered opaque scheduler.
        self.world.run(GOD, "@recovery-pulse");
        module_ops(&mut self.fixture.disk, &self.world.take_module_ops())
    }
    fn callbacks(&self) -> TestResult<Vec<i64>> {
        let pending = self.world.engine().sched.pending_entries();
        assert_eq!(pending.omitted(), 0, "bounded actual scheduler inspection");
        Ok(pending
            .entries()
            .iter()
            .filter_map(|(due, kind)| {
                matches!(kind, ScheduledKind::CommandContinuation(_)).then_some(*due)
            })
            .collect())
    }
    fn fire_callbacks(&mut self) -> TestResult {
        if let Some(due) = self.callbacks()?.first().copied() {
            self.world.fire_due(due);
        }
        module_ops(&mut self.fixture.disk, &self.world.take_module_ops())
    }
    fn next(&mut self, transition: Transition) -> TestResult<DrivenSubmission> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.pulse()?;
            self.fire_callbacks()?;
            if let Some(submitted) = self.world.engine_mut().take_observed_authoritative_submit() {
                assert!(submitted.is_observed());
                assert_eq!(submitted.request().actor, GOD);
                assert!(submitted.request().action == self.fixture.action(transition)?);
                return Ok(submitted);
            }
            if Instant::now() >= deadline {
                return Err("real Registry scheduler did not reach expected transition".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn run(&self, id: RunId) -> TestResult<Run> {
        Ok(self
            .world
            .world()
            .module_state::<BulkStore>()
            .ok_or("quota")?
            .run(self.fixture.account, GOD, id)?
            .clone())
    }
    fn active(&self) -> TestResult<RunId> {
        self.world
            .world()
            .module_state::<BulkStore>()
            .ok_or("quota")?
            .active(self.fixture.account)
            .ok_or_else(|| "actual admitted run".into())
    }
    #[inline(never)]
    fn prepare(&mut self, submitted: DrivenSubmission) -> TestResult<Box<Planned>> {
        let (request, driver) = submitted.into_parts();
        let AuthoritativeSubmit {
            actor,
            action,
            subject,
            targets,
            parameters,
            key,
        } = *request;
        assert_eq!(actor, GOD);
        assert!(targets.is_empty());
        assert_eq!(subject, PerceivedRef::SelfActor);
        assert!(parameters == json!({}), "private service selector only");
        self.serial += 1;
        let request = RequestUid::from_bytes(self.serial.to_be_bytes());
        let commit = CommitToken::from_bytes(self.serial.to_be_bytes());
        let principal = PrincipalId::Object { id: actor };
        let engine = self.world.engine_mut();
        let query = engine.legal_action_query(principal.clone(), actor, subject.clone())?;
        let offers = engine.project_legal_actions(&query)?;
        let offer = offers
            .projected_for_action(&action)
            .ok_or("actual Registry offer")?;
        let resolved = engine.resolve_legal_action(
            &offers,
            OfferExecutionContext {
                request,
                principal: principal.clone(),
                actor,
                idempotency: ScopedIdempotencyKey::new(
                    IdempotencyScope {
                        principal,
                        owner: action.key().owner.clone(),
                        namespace: CanonicalId::new(action.key().id.as_str())?,
                    },
                    1,
                    IdempotencyKey::Caller(key),
                ),
                viewer_scope: query.viewer_scope(),
                now: None,
            },
            OfferSubmission {
                offer: offer.offer(),
                subject,
                targets,
                parameters,
            },
        )?;
        let (actual, resolved, guards) = resolved.into_authoritative_parts();
        assert!(actual == action);
        let invocation = resolved.invocation().clone();
        let ticket_id = TicketId::new(
            invocation
                .parameters()
                .get("ticket")
                .and_then(Value::as_u64)
                .ok_or("actual private ticket")?,
        )?;
        let ticket = engine
            .world
            .module_state::<Artifacts>()
            .ok_or("artifacts")?
            .registry
            .get(ticket_id)?;
        let prepared = engine.prepare_registered_composed_resolved_authoritative_action(
            &actual, &resolved, commit,
        )?;
        Ok(Box::new(Planned {
            action: actual,
            invocation,
            guards,
            prepared,
            commit,
            ticket_id,
            ticket,
            _driver: driver,
        }))
    }
    #[inline(never)]
    fn install(&mut self, plan: Box<Planned>) -> TestResult<CommitToken> {
        let Planned {
            action,
            invocation,
            guards,
            prepared,
            commit,
            ticket_id,
            ticket,
            _driver,
        } = *plan;
        self.world
            .world()
            .module_state::<Artifacts>()
            .ok_or("artifacts")?
            .registry
            .check(ticket_id, &ticket)?;
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = prepared.1.into_parts();
        definitions.push(action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: commit,
            request: invocation.request(),
            idempotency: invocation.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: invocation.request_hash(),
            plan_hash: CanonicalHash::from_bytes([2; 32]),
            provider: action.key().owner.clone(),
            action,
            cause: PreparedCausalRef::new(invocation.request(), None, 0)?,
            definitions,
            writes: prepared.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(commit)),
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        })?;
        self.world
            .engine_mut()
            .stage_authoritative_action(prepared.0, guards, bundle)?;
        let (bundle, installation) = self
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("actual staged Registry action")?
            .into_parts();
        for write in bundle.writes() {
            match write {
                ReplayWrite::ModuleJsonSet {
                    namespace,
                    key,
                    value,
                } => {
                    self.fixture
                        .disk
                        .insert((namespace.to_string(), key.to_string()), value.clone());
                }
                ReplayWrite::ModuleDelete { namespace, key } => {
                    self.fixture
                        .disk
                        .remove(&(namespace.to_string(), key.to_string()));
                }
                _ => return Err("unexpected Registry replay write".into()),
            }
        }
        self.world
            .engine_mut()
            .install_composed_action(installation)?;
        let artifacts = self
            .world
            .world()
            .module_state::<Artifacts>()
            .ok_or("artifacts")?;
        let ack = artifacts
            .registry
            .peek_acknowledged()
            .ok_or("actual owner ACK")?;
        assert!(ack.operation == commit && ack.ticket == ticket_id);
        assert!(artifacts.registry.get(ticket_id).is_err());
        Ok(commit)
    }
    fn queue_unresolved_retry(&mut self, plan: &Planned, id: RunId) -> TestResult {
        let before = self.run(id)?;
        assert!(self.callbacks()?.is_empty());
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            self.pulse()?;
            assert!(
                self.world
                    .engine_mut()
                    .take_observed_authoritative_submit()
                    .is_none(),
                "unresolved observation cannot create a second submission"
            );
            if !self.callbacks()?.is_empty() {
                break;
            }
            if Instant::now() >= deadline {
                return Err("ordinary one-second retry did not queue".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            self.callbacks()?.len(),
            1,
            "one normally scheduled older callback"
        );
        self.world
            .world()
            .module_state::<Artifacts>()
            .ok_or("artifacts")?
            .registry
            .check(plan.ticket_id, &plan.ticket)?;
        assert!(
            self.run(id)? == before,
            "queued retry cannot alter quota or flight"
        );
        Ok(())
    }
    fn consume_ack_with_older_callback(&mut self, id: RunId) -> TestResult {
        let before = self.run(id)?;
        self.pulse()?;
        let callbacks = self.callbacks()?;
        assert_eq!(
            callbacks.len(),
            2,
            "old callback plus one progress-earned wake"
        );
        {
            let pending = self.world.engine().sched.pending_entries();
            let mut kinds = pending.entries().iter().filter_map(|(_, kind)| {
                matches!(kind, ScheduledKind::CommandContinuation(_)).then_some(*kind)
            });
            assert!(
                kinds.next() == kinds.next(),
                "same actual opaque actor continuation"
            );
        }
        self.fire_callbacks()?;
        assert!(
            self.callbacks()?.is_empty(),
            "both actual callbacks drain without self-rescheduling"
        );
        assert!(
            self.world
                .engine_mut()
                .take_observed_authoritative_submit()
                .is_none(),
            "stale callback cannot duplicate the acknowledged operation"
        );
        assert!(
            self.run(id)? == before,
            "ACK readback/callback cannot charge or settle twice"
        );
        Ok(())
    }
}

#[test]
fn new_progress_wakes_registered_continuations_without_duplicate_paid_work() -> TestResult {
    let mut h = ScheduledRegistry::new()?;
    let request = h.next(Transition::Admit)?;
    let plan = h.prepare(request)?;
    let job = h.install(plan)?;
    let run = h.active()?;
    let admitted = h.run(run)?;
    assert_eq!(admitted.charged_unit_ms, 0);
    assert!(admitted.flight.is_none());
    let request = h.next(Transition::Grant)?;
    let plan = h.prepare(request)?;
    h.queue_unresolved_retry(&plan, run)?;
    h.install(plan)?;
    let granted = h.run(run)?;
    let flight = granted.flight.as_ref().ok_or("one actual paid flight")?;
    assert_eq!(granted.next_sequence, admitted.next_sequence + 1);
    assert_eq!(granted.charged_unit_ms, flight.allowance_unit_ms);
    h.consume_ack_with_older_callback(run)?;

    // The real worker consumes that one retained lease. No measurement, result
    // or receipt is supplied by the test, and its eight-step bound is unchanged.
    let request = h.next(Transition::Settle)?;
    let plan = h.prepare(request)?;
    let TicketKind::Settle { receipt, .. } = &plan.ticket.kind else {
        return Err("actual worker settlement ticket".into());
    };
    let actual_unit_ms = receipt.actual_unit_ms;
    assert!(actual_unit_ms > 0 && actual_unit_ms <= flight.allowance_unit_ms);
    assert!(
        receipt.terminal.is_none(),
        "one quantum is not certification"
    );
    h.queue_unresolved_retry(&plan, run)?;
    h.install(plan)?;
    let settled = h.run(run)?;
    assert!(settled.flight.is_none());
    assert_eq!(settled.next_sequence, granted.next_sequence);
    assert_eq!(settled.charged_unit_ms, actual_unit_ms);
    // Cancellation is a real player request; it waits for the same fair turn.
    h.world.run(GOD, "@registry/cancel");
    h.consume_ack_with_older_callback(run)?;
    let request = h.next(Transition::Cancel)?;
    let plan = h.prepare(request)?;
    h.install(plan)?;
    h.pulse()?;
    h.fire_callbacks()?;
    h.pulse()?;
    let cancelled = h.run(run)?;
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(cancelled.charged_unit_ms, actual_unit_ms);
    assert_eq!(cancelled.next_sequence, granted.next_sequence);
    assert!(cancelled.flight.is_none());
    assert!(h.callbacks()?.is_empty());
    assert!(
        h.world
            .engine_mut()
            .take_observed_authoritative_submit()
            .is_none()
    );
    let (mut cold, _) = engine(
        &h.fixture.resources,
        h.fixture.accounts.clone(),
        CommerceMode::Active,
    )?;
    load(&mut cold, &h.fixture.disk)?;
    let cold_run = cold
        .world
        .module_state::<BulkStore>()
        .ok_or("cold quota")?
        .run(h.fixture.account, GOD, run)?;
    assert!(
        cold_run == &cancelled,
        "actual complete paid history survives cold audit"
    );
    assert!(cancelled.registry_job == Some(job));
    Ok(())
}
