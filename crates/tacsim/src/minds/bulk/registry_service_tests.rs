#![cfg(test)]
//! These tests prove dispatch, custody and fair paid-lease mechanics. The
//! structural quota plan below is not evidence of a Registry certification.
use super::super::{
    executor::{FairQueue, Turn},
    model::{Artifact, ArtifactBytes, BulkConfig, Opponent, Plan, Scenario, Status, WorkKind},
    owner::{Prepared, Receipt},
    registry_budget::{
        RegistryBudget, RegistryCase, RegistryCursor, RegistryEnding, RegistryProgress,
    },
    registry_participant::QuotaParticipant,
    registry_tickets::{PreparedSettlement, Tickets},
};
use super::*;
use mush_core::{
    CommitToken, DefinitionDigest, GameModule, LegalActionQuery, PrincipalId, SubjectKey,
};
use mush_roll::commerce::rating::{
    CaseKey, CaseKind, ProbeName, RatingClass, protocol::RegistryQuotaParticipant,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

// This test runtime supplies only the read half of the core observation. Its
// proof latches test consumer custody; actual server proof production is covered
// by mush-server's submission_disposition tests, not invented here.
#[derive(Clone, Default)]
struct Observations(Arc<Mutex<Vec<SubmissionObservation>>>);
#[derive(Clone, Default)]
struct Proofs(Arc<Mutex<Vec<Arc<std::sync::atomic::AtomicBool>>>>);
struct Proof(Arc<std::sync::atomic::AtomicBool>);
impl mush_core::submission_observation::SubmissionDispositionReader for Proof {
    fn no_intent(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
impl mush_core::submission_observation::SubmissionRuntime for Proofs {
    fn observe(
        &self,
        _: &mush_core::AuthoritativeSubmit,
    ) -> mush_core::submission_observation::SubmissionRuntimeState {
        let proof = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Ok(mut proofs) = self.0.lock() {
            proofs.push(proof.clone());
        }
        mush_core::submission_observation::SubmissionRuntimeState::new(
            Arc::new(Proof(proof)),
            Box::new(()),
        )
    }
}
struct ObserveFixture(Observations);
impl GameModule for ObserveFixture {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let descriptor = mush_core::DefinitionDescriptor::new(
            mush_core::DefinitionKind::new("observation_test")
                .map_err(|e| ModError::msg(e.to_string()))?,
            mush_core::DefinitionId::new("selector").map_err(|e| ModError::msg(e.to_string()))?,
            1,
            serde_json::json!({"test":"consumer custody only"}),
        )
        .map_err(|e| ModError::msg(e.to_string()))?;
        reg.definition_validator(
            descriptor.kind().clone(),
            crate::definitions::exact_validator(vec![descriptor.clone()]),
        )?;
        let action = reg.definition(descriptor)?;
        let observations = self.0.clone();
        reg.classify_commands(CommandCompatibility::Authoritative);
        reg.command(
            CommandSpec {
                name: "@observe-attempt".into(),
                switches: vec!["refuse".into()],
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let observation = ctx.submit_authoritative_observed(
                    action.clone(),
                    PerceivedRef::SelfActor,
                    vec![],
                    serde_json::json!({}),
                    IdempotencyKeyText::parse_canonical("registry-consumer")
                        .map_err(|e| CmdError::msg(e.to_string()))?,
                )?;
                observations
                    .0
                    .lock()
                    .map_err(|_| CmdError::msg("capture"))?
                    .push(observation);
                if ctx.switches.has("refuse") {
                    return Err(CmdError::msg("unsubmitted fixture"));
                }
                Ok(())
            }),
        )
    }
}
struct ObservedHarness {
    world: mush_testkit::TestWorld,
    observations: Observations,
    proofs: Proofs,
}
impl ObservedHarness {
    fn new() -> TestResult<Self> {
        let observations = Observations::default();
        let proofs = Proofs::default();
        let mut world = mush_testkit::TestWorld::with_modules(vec![Box::new(ObserveFixture(
            observations.clone(),
        ))]);
        world
            .engine_mut()
            .set_submission_runtime(Arc::new(proofs.clone()))?;
        Ok(Self {
            world,
            observations,
            proofs,
        })
    }
    fn queue(&mut self, refuse: bool) -> TestResult<SubmissionObservation> {
        self.world.run(
            mush_core::GOD,
            if refuse {
                "@observe-attempt/refuse"
            } else {
                "@observe-attempt"
            },
        );
        self.observations
            .0
            .lock()
            .map_err(|_| "capture")?
            .pop()
            .ok_or_else(|| "actual observation".into())
    }
    fn drive(&mut self) -> TestResult<Arc<std::sync::atomic::AtomicBool>> {
        let driven = self
            .world
            .engine_mut()
            .take_observed_authoritative_submit()
            .ok_or("actual queued request")?;
        drop(driven);
        self.proofs
            .0
            .lock()
            .map_err(|_| "proofs")?
            .last()
            .cloned()
            .ok_or_else(|| "runtime observation".into())
    }
}

fn admission_ticket(tickets: &mut Tickets, binding: Binding) -> TestResult<TicketId> {
    use mush_roll::commerce::rating::{RatingCompatibility, RatingSubject};
    let hash = DefinitionDigest::from_bytes([1; 32]);
    let revision = serde_json::from_value(serde_json::json!({"format":1,
        "revision":{"key":{"character":binding.character.0,"mind":1,"number":1},"digest":hash},"source":hash}))?;
    let subject = RatingSubject::new(revision, hash, hash, hash, hash, RatingClass::I);
    let start = ArtifactBytes::new(br#"{"fixture":"retained admission custody"}"#.to_vec())?;
    let mut plan = plan()?;
    plan.start = start.descriptor().clone();
    plan.registry = Some(RegistryBudget::prepare(
        subject.digest()?,
        hash,
        hash,
        vec![RegistryCase {
            key: CaseKey::new(
                RatingClass::I,
                CaseKind::Probe {
                    pair: 0,
                    name: ProbeName::EmptyArmed,
                },
            )?,
            units: 2,
            native_ms: 250,
            maximum_ms: 10_000,
        }],
    )?);
    Ok(
        tickets.admit(super::super::registry_tickets::AdmissionData {
            account: binding.account,
            character: binding.character,
            nonce: 1,
            subject,
            compatibility: RatingCompatibility::combat_v1(hash),
            plan,
            start,
        })?,
    )
}

#[test]
fn exact_admission_no_intent_releases_ticket_machine_and_shared_hold_only_once() -> TestResult {
    let mut harness = ObservedHarness::new()?;
    let binding = binding(7)?;
    let mut tickets = Tickets::default();
    let ticket = admission_ticket(&mut tickets, binding)?;
    let exact = tickets.get(ticket)?;
    let drops = Arc::new(AtomicUsize::new(0));
    let mut entry = entry(binding, ticket, drops.clone());
    let mut preparing = std::collections::BTreeSet::from([binding.account]);
    entry.pending.as_mut().ok_or("pending")?.attempt = Attempt::Observed(harness.queue(false)?);
    for _ in 0..3 {
        check!(!consume_attempt(&mut entry, &mut tickets, &mut preparing)?);
        tickets.check(ticket, &exact)?;
        check!(preparing.contains(&binding.account));
        check_eq!(drops.load(Ordering::SeqCst), 0);
    }
    let proof = harness.drive()?;
    check!(
        !consume_attempt(&mut entry, &mut tickets, &mut preparing)?,
        "driver drop is unresolved"
    );
    check!(matches!(
        entry.pending.as_ref().ok_or("pending")?.attempt,
        Attempt::Observed(_)
    ));
    check!(
        tickets
            .discard_unadmitted(ticket, ObjId(8), binding.account)
            .is_err()
    );
    tickets.check(ticket, &exact)?;
    proof.store(true, Ordering::Release);
    entry.actor = ObjId(8);
    check!(consume_attempt(&mut entry, &mut tickets, &mut preparing).is_err());
    check!(matches!(
        entry.pending.as_ref().ok_or("retained proof")?.attempt,
        Attempt::NoIntent
    ));
    tickets.check(ticket, &exact)?;
    check_eq!(drops.load(Ordering::SeqCst), 0);
    check!(preparing.contains(&binding.account));
    entry.actor = binding.character;
    check!(consume_attempt(&mut entry, &mut tickets, &mut preparing)?);
    check!(entry.pending.is_none());
    check!(matches!(entry.phase, Phase::Finished));
    check!(preparing.is_empty());
    check!(tickets.get(ticket).is_err());
    check_eq!(drops.load(Ordering::SeqCst), 1);
    check!(!consume_attempt(&mut entry, &mut tickets, &mut preparing)?);
    check_eq!(drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn not_dispatched_retries_but_new_unresolved_attempt_keeps_custody() -> TestResult {
    let mut harness = ObservedHarness::new()?;
    let binding = binding(7)?;
    let mut tickets = Tickets::default();
    let ticket = admission_ticket(&mut tickets, binding)?;
    let exact = tickets.get(ticket)?;
    let drops = Arc::new(AtomicUsize::new(0));
    let mut entry = entry(binding, ticket, drops.clone());
    let mut preparing = std::collections::BTreeSet::from([binding.account]);
    entry.pending.as_mut().ok_or("pending")?.attempt = Attempt::Observed(harness.queue(true)?);
    check!(!consume_attempt(&mut entry, &mut tickets, &mut preparing)?);
    check!(matches!(
        entry.pending.as_ref().ok_or("pending")?.attempt,
        Attempt::Unsubmitted
    ));
    entry.pending.as_mut().ok_or("pending")?.attempt = Attempt::Observed(harness.queue(false)?);
    let _unresolved = harness.drive()?;
    for _ in 0..3 {
        check!(!consume_attempt(&mut entry, &mut tickets, &mut preparing)?);
    }
    tickets.check(ticket, &exact)?;
    check!(matches!(
        entry.pending.as_ref().ok_or("pending")?.attempt,
        Attempt::Observed(_)
    ));
    check_eq!(drops.load(Ordering::SeqCst), 0);
    check!(preparing.contains(&binding.account));
    Ok(())
}

#[test]
fn admitted_transition_no_intent_preserves_exact_machine_ticket_and_paid_receipt() -> TestResult {
    for transition in [Transition::Grant, Transition::Settle, Transition::Cancel] {
        let mut harness = ObservedHarness::new()?;
        let binding = binding(7)?;
        let mut quota = BulkStore::new(BulkConfig::default())?;
        admit(&mut quota, binding)?;
        let mut tickets = Tickets::default();
        let ticket = match transition {
            Transition::Grant => tickets.grant(binding, 2_000)?,
            Transition::Cancel => tickets.cancel(binding)?,
            Transition::Settle => {
                let operation = CommitToken::from_bytes([44; 16]);
                let grant = quota.prepare_grant(binding.account, binding.run, 0, 2_000)?;
                install(&mut quota, grant, operation)?;
                let lease = quota
                    .take_registry_lease(operation)
                    .ok_or("actual paid lease")?;
                let receipt = Receipt {
                    actual_unit_ms: 1_000 * lease.units(),
                    lease,
                    result: None,
                    terminal: None,
                    conduct: None,
                    registry: Some(RegistryProgress {
                        cursor: RegistryCursor {
                            index: 0,
                            simulated_ms: 1_000,
                        },
                        ending: RegistryEnding::Continue,
                    }),
                };
                quota.prepare_finish(receipt.clone(), 0)?;
                tickets.settle(binding, receipt, None, None)?
            }
            _ => return Err("fixture phase".into()),
        };
        let exact = tickets.get(ticket)?;
        let before = serde_json::to_value(quota.account_run(binding.account, binding.run)?)?;
        let drops = Arc::new(AtomicUsize::new(0));
        let ready = Some(Ready::registry(
            binding,
            [1; 32],
            Box::new(Kept(drops.clone())),
        ));
        let phase = match transition {
            Transition::Grant => Phase::Grant(ready),
            Transition::Settle => Phase::Settle(ready),
            Transition::Cancel => Phase::Cancel(ready),
            _ => return Err("phase".into()),
        };
        let mut entry = Entry {
            reconformance: false,
            id: ticket.get(),
            account: binding.account,
            actor: binding.character,
            binding: Some(binding),
            pending: Some(Pending {
                ticket,
                transition,
                ack: None,
                attempt: Attempt::Observed(harness.queue(false)?),
            }),
            phase,
            cancel: false,
            schedule_after: 0,
        };
        let proof = harness.drive()?;
        let mut preparing = std::collections::BTreeSet::new();
        check!(!consume_attempt(&mut entry, &mut tickets, &mut preparing)?);
        proof.store(true, Ordering::Release);
        check!(!consume_attempt(&mut entry, &mut tickets, &mut preparing)?);
        check!(matches!(
            entry.pending.as_ref().ok_or("pending")?.attempt,
            Attempt::Unsubmitted
        ));
        tickets.check(ticket, &exact)?;
        check!(
            tickets
                .discard_unadmitted(ticket, binding.character, binding.account)
                .is_err()
        );
        check_eq!(
            serde_json::to_value(quota.account_run(binding.account, binding.run)?)?,
            before
        );
        check_eq!(drops.load(Ordering::SeqCst), 0);
        entry.pending.as_mut().ok_or("pending")?.attempt = Attempt::Observed(harness.queue(false)?);
        let _new_unresolved = harness.drive()?;
        check!(!consume_attempt(&mut entry, &mut tickets, &mut preparing)?);
        check!(
            matches!(
                entry.pending.as_ref().ok_or("pending")?.attempt,
                Attempt::Observed(_)
            ),
            "old NoIntent cannot complete the replacement attempt"
        );
        check_eq!(drops.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct Kept(Arc<AtomicUsize>);
impl Drop for Kept {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Machine for Kept {
    fn native_substep_ms(&self) -> u32 {
        250
    }
    fn next_simulation_ms(&self) -> u64 {
        250
    }
    fn step(&mut self, _: u64) -> Result<Turn, BulkError> {
        Err(BulkError::Missing)
    }
}
fn binding(actor: u32) -> TestResult<Binding> {
    Ok(Binding {
        job: CommitToken::from_bytes([u8::try_from(actor)?; 16]),
        account: AccountId::mint("registry-service", i64::from(actor))?,
        character: ObjId(actor),
        run: RunId(1),
    })
}
fn entry(binding: Binding, ticket: TicketId, drops: Arc<AtomicUsize>) -> Entry {
    Entry {
        reconformance: false,
        id: ticket.get(),
        account: binding.account,
        actor: binding.character,
        binding: None,
        pending: Some(Pending {
            ticket,
            transition: Transition::Admit,
            ack: None,
            attempt: Attempt::Unsubmitted,
        }),
        phase: Phase::Admission {
            machine: Some(Box::new(Kept(drops))),
            start: [1; 32],
        },
        cancel: false,
        schedule_after: 0,
    }
}
fn plan() -> TestResult<Plan> {
    let hash = DefinitionDigest::from_bytes([1; 32]);
    Ok(Plan {
        kind: WorkKind::Registry,
        conduct: None,
        registry: Some(RegistryBudget::prepare(
            hash,
            hash,
            hash,
            vec![RegistryCase {
                key: CaseKey::new(
                    RatingClass::I,
                    CaseKind::Probe {
                        pair: 0,
                        name: ProbeName::EmptyArmed,
                    },
                )?,
                units: 2,
                native_ms: 250,
                maximum_ms: 10_000,
            }],
        )?),
        scenario: Scenario::Duel,
        opponent: Opponent::Doctrine,
        per_side: 0,
        unit_count: 0,
        games: 0,
        seeds: vec![],
        limit_ms: 0,
        native_substep_ms: 0,
        start: Artifact {
            sha256: [1; 32],
            bytes: 100,
        },
        result_reservation_bytes: 8192,
    })
}
fn install(store: &mut BulkStore, change: Prepared, operation: CommitToken) -> TestResult {
    store.check_registry(&change, operation)?;
    store.apply_registry_checked(change, operation);
    Ok(())
}
fn admit(store: &mut BulkStore, binding: Binding) -> TestResult {
    let (run, change) = store.prepare_registry_admit(
        binding.account,
        binding.character,
        1,
        plan()?,
        binding.job,
        0,
    )?;
    check_eq!(run, binding.run);
    install(store, change, binding.job)
}

#[test]
fn absent_or_wrong_ack_never_releases_machine_and_retries_the_same_actor_selector() -> TestResult {
    let binding = binding(7)?;
    let drops = Arc::new(AtomicUsize::new(0));
    let ticket = TicketId::new(17)?;
    let mut lane = Lane::default();
    lane.entries
        .insert(binding.character, entry(binding, ticket, drops.clone()));
    for now in [0, 1_000, 60_000, 86_400_000] {
        let entry = lane.entries.get_mut(&binding.character).ok_or("entry")?;
        check!(
            entry.continuation(now, true).is_none(),
            "pending core custody suppresses resubmit"
        );
        let (actor, argument) = entry
            .continuation(now, false)
            .ok_or("registered actor continuation")?;
        check_eq!(actor, binding.character);
        check_eq!(argument, "17");
        check!(
            entry.continuation(now, false).is_none(),
            "bounded retry cadence"
        );
        check!(matches!(
            entry.phase,
            Phase::Admission {
                machine: Some(_),
                ..
            }
        ));
    }
    let ack = Acknowledged {
        ticket,
        operation: binding.job,
        job: binding.job,
        transition: Transition::Admit,
    };
    check!(!lane.accept_ack(Acknowledged {
        ticket: TicketId::new(18)?,
        ..ack
    }));
    check!(!lane.accept_ack(Acknowledged {
        transition: Transition::Grant,
        ..ack
    }));
    let now = 86_400_001;
    check!(
        lane.entries
            .get_mut(&binding.character)
            .ok_or("entry")?
            .continuation(now, false)
            .is_none(),
        "wrong acknowledgements do not reset unresolved retry backoff"
    );
    check!(lane.accept_ack(ack));
    let entry = lane.entries.get_mut(&binding.character).ok_or("entry")?;
    check!(
        entry.continuation(now, true).is_none(),
        "fresh ACK keeps Core custody gate"
    );
    check_eq!(
        entry.continuation(now, false),
        Some((binding.character, "17".into()))
    );
    check!(!lane.accept_ack(ack), "an Ack is consumed only once");
    check!(
        lane.entries
            .get_mut(&binding.character)
            .ok_or("entry")?
            .continuation(now, false)
            .is_none(),
        "duplicate ACK cannot queue another callback"
    );
    check!(
        lane.entries
            .get_mut(&binding.character)
            .ok_or("entry")?
            .continuation(now + RETRY_MS - 1, false)
            .is_none(),
        "same unresolved state retains 1000ms backoff"
    );
    check!(
        matches!(
            lane.entries.get(&binding.character).ok_or("entry")?.phase,
            Phase::Admission {
                machine: Some(_),
                ..
            }
        ),
        "Ack alone cannot bypass typed owner readback"
    );
    check_eq!(drops.load(Ordering::SeqCst), 0);
    drop(lane);
    check_eq!(drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn two_accounts_keep_shared_fair_turn_across_real_busy_grant_and_paid_settlement() -> TestResult {
    let cfg = BulkConfig {
        proving_workers: 1,
        queued: 2,
        ..BulkConfig::default()
    };
    let mut store = BulkStore::new(cfg.clone())?;
    // Reverse actor order deliberately: B must not lose to actor-map ordering.
    let a = binding(7)?;
    let b = binding(3)?;
    admit(&mut store, a)?;
    admit(&mut store, b)?;
    let drops = Arc::new(AtomicUsize::new(0));
    let mut lane = Lane::default();
    let mut queue = FairQueue::new(&cfg)?;
    for (binding, ticket) in [(a, 17), (b, 18)] {
        let mut entry = entry(binding, TicketId::new(ticket)?, drops.clone());
        // Admission readback has already supplied the bound Ready in this
        // focused scheduler fixture; it does not manufacture a certificate.
        entry.phase = Phase::Queued;
        entry.pending = None;
        entry.binding = Some(binding);
        entry.schedule_after = 10_000;
        lane.entries.insert(binding.character, entry);
        queue
            .push(Ready::registry(
                binding,
                [1; 32],
                Box::new(Kept(drops.clone())),
            ))
            .map_err(|(e, _)| e)?;
    }
    lane.take_fair_turn(queue.pop().ok_or("A fair head")?)
        .map_err(|_| "take A")?;
    check_eq!(lane.pending_turn, Some(a.character));
    let head = lane.entries.get_mut(&a.character).ok_or("A entry")?;
    check!(
        head.continuation(1, true).is_none(),
        "new turn keeps Core custody gate"
    );
    check_eq!(
        head.continuation(1, false),
        Some((a.character, "17".into()))
    );
    check!(head.continuation(1, false).is_none());
    let op_a = CommitToken::from_bytes([41; 16]);
    let change = store.prepare_grant(a.account, a.run, 0, 2_000)?;
    install(&mut store, change, op_a)?;
    let lease_a = store
        .take_registry_lease(op_a)
        .ok_or("actual acknowledged A lease")?;
    // This is the production continuation's point after exact Ack/readback.
    lane.pending_turn = None;
    let mut ready_a = match std::mem::replace(
        &mut lane.entries.get_mut(&a.character).ok_or("A entry")?.phase,
        Phase::Working,
    ) {
        Phase::Grant(ready) => ready,
        _ => return Err("A granted phase".into()),
    };
    lane.take_fair_turn(queue.pop().ok_or("B fair head")?)
        .map_err(|_| "take B")?;
    check_eq!(lane.pending_turn, Some(b.character));
    check_eq!(
        lane.entries
            .get_mut(&b.character)
            .ok_or("B entry")?
            .continuation(2, false),
        Some((b.character, "18".into()))
    );
    let mut harness = ObservedHarness::new()?;
    let mut tickets = Tickets::default();
    let ticket = tickets.grant(b, 2_000)?;
    lane.entries.get_mut(&b.character).ok_or("B entry")?.pending = Some(Pending {
        ticket,
        transition: Transition::Grant,
        ack: None,
        attempt: Attempt::Unsubmitted,
    });
    for _ in 0..3 {
        check!(matches!(
            store.prepare_grant(b.account, b.run, 0, 2_000),
            Err(BulkError::Busy)
        ));
        let entry = lane.entries.get_mut(&b.character).ok_or("B entry")?;
        check!(matches!(
            entry.pending.as_ref().ok_or("B pending")?.attempt,
            Attempt::Unsubmitted
        ));
        entry.pending.as_mut().ok_or("B pending")?.attempt =
            Attempt::Observed(harness.queue(false)?);
        let proof = harness.drive()?;
        check!(!consume_attempt(
            entry,
            &mut tickets,
            &mut std::collections::BTreeSet::new()
        )?);
        // The server proof producer is covered separately; this drives the
        // same consumer after each actual quota Busy refusal in this fixture.
        proof.store(true, Ordering::Release);
        check!(!consume_attempt(
            entry,
            &mut tickets,
            &mut std::collections::BTreeSet::new()
        )?);
        check_eq!(
            lane.pending_turn,
            Some(b.character),
            "Busy does not lose the fair head"
        );
    }
    let receipt = Receipt {
        conduct: None,
        registry: Some(RegistryProgress {
            cursor: RegistryCursor {
                index: 0,
                simulated_ms: 1_000,
            },
            ending: RegistryEnding::Continue,
        }),
        actual_unit_ms: 1_000 * lease_a.units(),
        lease: lease_a,
        result: None,
        terminal: None,
    };
    let settled = store.prepare_finish(receipt, 0)?;
    install(&mut store, settled, CommitToken::from_bytes([42; 16]))?;
    lane.entries.get_mut(&a.character).ok_or("A entry")?.phase = Phase::Queued;
    queue
        .push(ready_a.take().ok_or("A real retained machine")?)
        .map_err(|(e, _)| e)?;
    let next_a = queue.pop().ok_or("A returned queue tail")?;
    let previous_deadline = lane
        .entries
        .get(&a.character)
        .ok_or("A entry")?
        .schedule_after;
    let next_a = lane
        .take_fair_turn(next_a)
        .err()
        .ok_or("B must still own turn")?;
    check_eq!(
        lane.entries
            .get(&a.character)
            .ok_or("A entry")?
            .schedule_after,
        previous_deadline,
        "refused fair turn cannot reset scheduling"
    );
    queue.push(next_a).map_err(|(e, _)| e)?;
    let change = store.prepare_grant(b.account, b.run, 0, 2_000)?;
    let op_b = CommitToken::from_bytes([43; 16]);
    install(&mut store, change, op_b)?;
    check_eq!(
        store
            .take_registry_lease(op_b)
            .ok_or("B paid lease")?
            .account(),
        b.account
    );
    lane.pending_turn = None;
    lane.take_fair_turn(queue.pop().ok_or("A follows B")?)
        .map_err(|_| "next A")?;
    check_eq!(lane.pending_turn, Some(a.character));
    check_eq!(
        lane.entries
            .get_mut(&a.character)
            .ok_or("A entry")?
            .continuation(3, false),
        Some((a.character, "17".into()))
    );
    Ok(())
}

#[test]
fn terminal_owner_apply_keeps_account_and_global_capacity_until_machine_readback() -> TestResult {
    let cfg = BulkConfig {
        proving_workers: 1,
        queued: 1,
        ..BulkConfig::default()
    };
    let (wake, _receiver) = tokio::sync::mpsc::channel(1);
    let service = world::BulkService::new(cfg.clone(), wake)?;
    let mut runtime = service.0.lock().map_err(|_| "runtime poisoned")?;
    let mut store = BulkStore::new(cfg)?;
    let a = binding(7)?;
    let b = binding(3)?;
    let c = binding(5)?;
    admit(&mut store, a)?;
    let drops = Arc::new(AtomicUsize::new(0));
    let mut retained = entry(a, TicketId::new(17)?, drops.clone());
    retained.binding = Some(a);
    // The real service has retained the admitted Machine. This focused test
    // exercises its shared admission boundary, not synthetic typed readback.
    runtime.registry.entries.insert(a.character, retained);
    runtime.external_preparing.insert(a.account);
    check_eq!(store.active_count(), 1);
    check_eq!(
        runtime.ensure_admission(&store, b.account),
        Ok(()),
        "admission overlap consumes one account slot, not three"
    );
    runtime.preparing.insert(
        2,
        Preparing {
            account: b.account,
            actor: b.character,
            nonce: 1,
        },
    );
    check_eq!(
        runtime.ensure_admission(&store, c.account),
        Err(BulkError::QueueFull)
    );
    runtime.preparing.remove(&2);
    runtime.external_preparing.remove(&a.account);
    let cancelled = store.prepare_cancel(a.account, a.character, a.run, 1)?;
    install(&mut store, cancelled, CommitToken::from_bytes([44; 16]))?;
    check!(store.account_run(a.account, a.run)?.status.terminal());
    // With one queued slot, B can enter only after A's actual quota image is
    // terminal. A's retained Machine must still occupy the other service slot.
    admit(&mut store, b)?;
    check_eq!(
        store.active_count(),
        1,
        "real terminal owner image freed its active row"
    );
    check_eq!(
        runtime.ensure_admission(&store, a.account),
        Err(BulkError::Busy),
        "every character of this account is held until the Machine is released"
    );
    check_eq!(
        runtime.ensure_admission(&store, c.account),
        Err(BulkError::QueueFull),
        "terminal readback delay still consumes the global slot"
    );
    check_eq!(drops.load(Ordering::SeqCst), 0);
    // Model only the release boundary: production removes this entry after
    // exact terminal Ack plus Roll/execution/BulkStore joins all succeed.
    runtime.registry.entries.remove(&a.character);
    check_eq!(drops.load(Ordering::SeqCst), 1);
    check_eq!(runtime.ensure_admission(&store, a.account), Ok(()));
    check_eq!(runtime.ensure_admission(&store, c.account), Ok(()));
    // Even broken in-process joins must retain their bounded repair custody.
    // This Ready has no Entry and its actual quota row is already terminal.
    runtime.registry.stranded_ready.push(Ready::registry(
        a,
        [1; 32],
        Box::new(Kept(drops.clone())),
    ));
    check_eq!(
        runtime.ensure_admission(&store, a.account),
        Err(BulkError::Busy)
    );
    check_eq!(
        runtime.ensure_admission(&store, c.account),
        Err(BulkError::QueueFull)
    );
    check_eq!(drops.load(Ordering::SeqCst), 1);
    runtime.registry.stranded_ready.clear();
    check_eq!(drops.load(Ordering::SeqCst), 2);
    check_eq!(runtime.ensure_admission(&store, c.account), Ok(()));
    Ok(())
}

#[test]
fn worker_evidence_and_receipt_survive_codec_and_ticket_capacity_refusals() -> TestResult {
    let binding = binding(7)?;
    let mut store = BulkStore::new(BulkConfig::default())?;
    admit(&mut store, binding)?;
    let granted = store.prepare_grant(binding.account, binding.run, 0, 2_000)?;
    let operation = CommitToken::from_bytes([44; 16]);
    install(&mut store, granted, operation)?;
    let lease = store.take_registry_lease(operation).ok_or("paid lease")?;
    let bytes = ArtifactBytes::new(br#"{"not":"a measured result"}"#.to_vec())?;
    let bad = Receipt {
        registry: None,
        conduct: None,
        actual_unit_ms: lease.maximum_sim_ms() * lease.units(),
        lease: lease.clone(),
        result: Some(bytes.descriptor().clone()),
        terminal: Some(Status::Complete),
    };
    let (_, retained) = PreparedSettlement::from_worker(binding, bad, Some(bytes.clone()))
        .err()
        .ok_or("malformed result refusal")?;
    check_eq!(retained.ok_or("retained raw bytes")?.bytes(), bytes.bytes());
    let receipt = Receipt {
        registry: None,
        conduct: None,
        actual_unit_ms: lease.maximum_sim_ms() * lease.units(),
        lease,
        result: None,
        terminal: Some(Status::Failed),
    };
    // The actual quota owner accepts conservative full charging for unknown work.
    store.prepare_finish(receipt.clone(), 0)?;
    let prepared = PreparedSettlement::from_worker(binding, receipt, None).map_err(|(e, _)| e)?;
    let mut tickets = Tickets::default();
    let mut count = 0;
    while count < 137 {
        match tickets.cancel(binding) {
            Ok(_) => count += 1,
            Err(BulkError::QueueFull) => break,
            Err(e) => return Err(e.into()),
        }
    }
    check_eq!(count, 136);
    let (error, retained) = tickets
        .insert_settlement(prepared)
        .err()
        .ok_or("ticket bound")?;
    check_eq!(error, BulkError::QueueFull);
    // Free exactly one acknowledged slot; the same opaque completion inserts.
    let first = TicketId::new(1)?;
    let captured = tickets.get(first)?;
    tickets.acknowledge_checked(
        &captured,
        Acknowledged {
            ticket: first,
            operation,
            job: binding.job,
            transition: Transition::Cancel,
        },
    );
    tickets.take_acknowledged().ok_or("consumed ack")?;
    let id = tickets.insert_settlement(retained).map_err(|(e, _)| e)?;
    let row = tickets.get(id)?;
    let super::super::registry_tickets::TicketKind::Settle { receipt, .. } = &row.kind else {
        return Err("same settlement".into());
    };
    check_eq!(
        (receipt.account(), receipt.run(), receipt.terminal),
        (binding.account, binding.run, Some(Status::Failed))
    );
    Ok(())
}

#[derive(Clone, Default)]
struct Dispatches(Arc<Mutex<Vec<ObjId>>>);
struct EditingContext(Arc<AtomicUsize>);
impl mush_core::module::InputContext for EditingContext {
    fn prompt(&self, _: &mush_core::World) -> mush_proto::PromptSpec {
        mush_proto::PromptSpec {
            mode: mush_proto::InputMode::Normal,
            text: None,
        }
    }
    fn feed(&mut self, _: &mut Ctx<'_>, _: &str) -> mush_core::module::ContextOutcome {
        self.0.fetch_add(1, Ordering::AcqRel);
        mush_core::module::ContextOutcome::Consumed
    }
}
struct DispatchFixture;
impl GameModule for DispatchFixture {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(Artifacts::default());
        reg.state(BulkStore::new(BulkConfig::default()).map_err(|e| ModError::msg(e.to_string()))?);
        reg.state(Dispatches::default());
        reg.classify_commands(CommandCompatibility::Authoritative);
        reg.command(
            CommandSpec {
                name: COMMAND.into(),
                switches: vec!["continue".into()],
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut mush_core::Ctx<'_>| {
                if !ctx.switches.has("continue") || ctx.args.raw().trim() != "17" {
                    return Err(CmdError::msg("fixture selector"));
                }
                ctx.w
                    .module_state::<Dispatches>()
                    .ok_or_else(|| CmdError::msg("dispatch capture"))?
                    .0
                    .lock()
                    .map_err(|_| CmdError::msg("capture poisoned"))?
                    .push(ctx.actor);
                Ok(())
            }),
        )?;
        let continuation = reg.command_continuation(COMMAND, &["continue"])?;
        reg.command(
            CommandSpec {
                name: "@schedule-registry".into(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut mush_core::Ctx<'_>| {
                let binding = binding(ctx.actor.0).map_err(|e| CmdError::msg(e.to_string()))?;
                let mut entry = entry(
                    binding,
                    TicketId::new(17).map_err(|e| CmdError::msg(e.to_string()))?,
                    Arc::new(AtomicUsize::new(0)),
                );
                if let Some((actor, argument)) = entry.continuation(0, false) {
                    ctx.schedule_continuation(&continuation, actor, &argument, 0)?;
                }
                Ok(())
            }),
        )
    }
}

#[test]
fn real_scheduler_preserves_the_actor_and_projection_refuses_foreign_principal() -> TestResult {
    let mut world = mush_testkit::TestWorld::with_modules(vec![Box::new(DispatchFixture)]);
    let actor = world.spawn_player("Registry owner");
    let other = world.spawn_player("Other owner");
    world.set_now(1_000);
    world.run(actor, "@schedule-registry");
    let observed = world
        .world()
        .module_state::<Dispatches>()
        .ok_or("capture")?
        .clone();
    check!(observed.0.lock().map_err(|_| "poisoned")?.is_empty());
    let editor = Arc::new(AtomicUsize::new(0));
    // This can change after scheduling: checking only at enqueue would not
    // protect the ticket from entering a newly opened editor at timer fire.
    world.world_mut().push_input_context(
        actor,
        Some("tacsim".into()),
        Box::new(EditingContext(editor.clone())),
    );
    world.fire_due(1_000_000);
    check_eq!(*observed.0.lock().map_err(|_| "poisoned")?, vec![actor]);
    check_eq!(editor.load(Ordering::Acquire), 0);
    check_eq!(world.world().input_depth(actor), 1);
    world.run(actor, "ordinary editor input");
    check_eq!(editor.load(Ordering::Acquire), 1);
    world
        .world_mut()
        .pop_input_context(actor)
        .ok_or("editor retained")?;
    let binding = Binding {
        character: actor,
        ..binding(7)?
    };
    // Existing-run selectors classify the retained real quota plan.
    admit(
        world
            .world_mut()
            .module_state_mut::<BulkStore>()
            .ok_or("quota owner")?,
        binding,
    )?;
    world
        .world_mut()
        .module_state_mut::<Artifacts>()
        .ok_or("artifacts")?
        .registry
        .cancel(binding)?;
    let query = |principal, actor, subject| {
        LegalActionQuery::new(principal, actor, subject, PerceivedRef::SelfActor, 1, None)
    };
    let own = query(
        PrincipalId::Object { id: actor },
        actor,
        SubjectKey::Object { id: actor },
    )?;
    check!(QuotaParticipant::project_selector(world.engine(), &own, Transition::Cancel)?.is_some());
    check!(QuotaParticipant::project_selector(world.engine(), &own, Transition::Grant)?.is_none());
    let forged = query(
        PrincipalId::Object { id: other },
        actor,
        SubjectKey::Object { id: actor },
    )?;
    check!(
        QuotaParticipant::project_selector(world.engine(), &forged, Transition::Cancel)?.is_none()
    );
    let foreign = query(
        PrincipalId::Object { id: other },
        other,
        SubjectKey::Object { id: other },
    )?;
    check!(
        QuotaParticipant::project_selector(world.engine(), &foreign, Transition::Cancel)?.is_none()
    );
    let wrong_subject = query(
        PrincipalId::Object { id: actor },
        actor,
        SubjectKey::Object { id: other },
    )?;
    check!(
        QuotaParticipant::project_selector(world.engine(), &wrong_subject, Transition::Cancel)?
            .is_none()
    );
    Ok(())
}
