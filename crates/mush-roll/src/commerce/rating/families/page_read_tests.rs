//! Structural maximum-page evidence through the actual registered owner reader.
//! This tests bounded complete readback, not measured Registry certification.
use super::*;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    DefinitionValidationContext, OwnerId,
};
use mush_core::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use mush_core::{
    CmdError, CommandCompatibility, CommandSpec, Ctx, Engine, GOD, GameModule, Inbound, Outbox,
    Registry, World,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Answers = Arc<Mutex<Vec<Result<[RatingRef; 2], OwnerReadError>>>>;
type Handles = Arc<Mutex<Option<RatingFamilies>>>;
fn hash(byte: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([byte; 32])
}

fn complete(number: u8) -> TestResult<test_support::CompleteRating> {
    let identity = serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":1,"mind":number,"number":1},"digest":hash(number)},
        "source":hash(number)}))?;
    let base = test_support::complete_rating(
        RatingSubject::new(identity, hash(3), hash(4), hash(5), hash(6), RatingClass::I),
        RatingCompatibility::combat_v1(hash(7)),
        hash(8),
        CommitToken::from_bytes([number; 16]),
        ClockStamp {
            clock: mush_core::ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    )?;
    let template = base
        .pages
        .first()
        .ok_or("fixture page")?
        .content
        .rows()
        .first()
        .ok_or("fixture case")?
        .clone();
    let names = [
        ProbeName::EmptyArmed,
        ProbeName::EmptyUnarmed,
        ProbeName::Hostile450Armed,
        ProbeName::Hostile450Unarmed,
        ProbeName::HurtTwoArmed,
        ProbeName::HurtTwoUnarmed,
        ProbeName::OrderLossArmed,
        ProbeName::OrderLossUnarmed,
        ProbeName::DensityArmed,
        ProbeName::DensityUnarmed,
        ProbeName::FordMobile,
        ProbeName::FordDisabled,
    ];
    let kernels = [
        KernelName::MemoryKeysAt,
        KernelName::MemoryKeysOver,
        KernelName::MemoryValueAt,
        KernelName::MemoryValueOver,
        KernelName::RulesSlotsAt,
        KernelName::RulesSlotsOver,
        KernelName::MeterFinalFee,
        KernelName::MeterCallback,
    ];
    let mut rows = Vec::new();
    let mut classes = Vec::new();
    for class in [RatingClass::I, RatingClass::II, RatingClass::III] {
        let start = rows.len();
        for variant in 0..12 {
            for layout in [TrialLayout::Duel, TrialLayout::Muster, TrialLayout::Hold] {
                for seed in 1..=2 {
                    for swapped in [false, true] {
                        let mut row = template.clone();
                        row.key = CaseKey::new(
                            class,
                            CaseKind::Trial {
                                variant,
                                layout,
                                seed,
                                swapped,
                            },
                        )?;
                        row.terminal =
                            Some(TrialEnd::new(1000, 42, hash(9), TrialOutcome::Timeout)?);
                        rows.push(row);
                    }
                }
            }
        }
        for pair in 0..36 {
            for name in names {
                let mut row = template.clone();
                row.key = CaseKey::new(class, CaseKind::Probe { pair, name })?;
                rows.push(row);
            }
        }
        let mut cycle = template.clone();
        cycle.key = CaseKey::new(class, CaseKind::ProfileCycle {})?;
        if class != RatingClass::III {
            cycle.outcome = CaseOutcome::BudgetExceeded {};
            cycle.meter =
                MeterSummary::new(1, 100, 20, Some(Peak::new(100, 20, 0, 1)?), 1, 0, 0, 0)?;
        }
        rows.push(cycle);
        if class == RatingClass::III {
            for name in kernels {
                let mut row = template.clone();
                row.key = CaseKey::new(class, CaseKind::Kernel { name })?;
                row.coverage = Coverage::new(Applicability::EngineBoundary, 1, 1, hash(9))?;
                row.outcome = CaseOutcome::Kernel {
                    expected: KernelOutcome::Passed,
                    actual: KernelOutcome::Passed,
                };
                rows.push(row);
            }
        }
        classes.push(ClassSummary::from_cases(
            class,
            &rows
                .get(start..)
                .ok_or("class rows")?
                .iter()
                .collect::<Vec<_>>(),
        )?);
    }
    // MAX_PAGES117 is the representational bound. A complete certificate must
    // end each failed class on its non-kernel budget failure; its eight later
    // kernels cannot follow that failure. The full valid shape is1739/116.
    assert_eq!(rows.len(), MAX_CASES - 2 * kernels.len());
    let contents = rows
        .chunks(MAX_PAGE_ROWS)
        .enumerate()
        .map(|(i, rows)| {
            PageContent::new(
                base.receipt.subject().digest()?,
                base.receipt.compatibility().digest(),
                u16::try_from(i).map_err(|_| RatingError::Bound)?,
                rows.to_vec(),
            )
        })
        .collect::<Result<Vec<_>, RatingError>>()?;
    assert_eq!(contents.len(), 116);
    let result = RatingResultRef::from_pages(&contents)?;
    let mut content = base.receipt.content.clone();
    content.classes = Rows::new(classes)?;
    content.minimum = RatingClass::III;
    content.result = result.clone();
    let receipt = RatingReceipt::new(content, base.receipt.publication.clone())?;
    let pages = contents
        .into_iter()
        .map(|content| {
            RatingPage::new(
                receipt.publication().job(),
                receipt.id(),
                result.id(),
                content,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    receipt.verify_pages(&pages)?;
    let job = RatingJob::new(
        receipt.publication().job(),
        receipt.subject().clone(),
        receipt.compatibility().clone(),
        receipt.plan(),
    );
    let mut admission = base.execution.admission().clone();
    admission.cases = u16::try_from(MAX_CASES)?;
    admission.maximum_pages = u16::try_from(MAX_PAGES)?;
    use execution::{ExecutionObservation, ExecutionPhase, MeasuredRating, RatingExecution};
    let queued = ExecutionObservation {
        phase: ExecutionPhase::Queued,
        next_sequence: 1,
        cursor: Default::default(),
        charged_unit_ms: 0,
        flight: None,
        result: None,
    };
    let execution = RatingExecution::admit(&job, admission, queued.clone())?;
    let job = job.transition(RatingJobState::Running { lease_sequence: 1 })?;
    let running = ExecutionObservation {
        phase: ExecutionPhase::Running,
        next_sequence: 2,
        ..queued.clone()
    };
    let execution = execution.observed(
        &queued,
        running.clone(),
        &job,
        CommitToken::from_bytes([201; 16]),
    )?;
    let job = job.transition(RatingJobState::Completed {
        receipt: receipt.reference()?,
    })?;
    let completed = ExecutionObservation {
        phase: ExecutionPhase::Complete,
        result: Some(MeasuredRating::from_receipt(&receipt, &pages)?.artifact()?),
        ..running.clone()
    };
    let execution = execution.observed(
        &running,
        completed,
        &job,
        CommitToken::from_bytes([202; 16]),
    )?;
    execution.verify_completion(&job, &receipt, &pages)?;
    Ok(test_support::CompleteRating {
        completion: RatingCompletion::for_execution(&receipt, &execution)?,
        status: RatingStatus::active(&receipt)?,
        receipt,
        pages,
        job,
        execution,
    })
}

fn descriptor() -> TestResult<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read")?,
        DefinitionId::new("complete_pair_test")?,
        1,
        json!({"format":1}),
    )?)
}
fn definition() -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, descriptor()?)?)
}
#[derive(Clone)]
struct Slot;
struct PairReader {
    ratings: RatingFamilies,
}
impl OwnerReadService for PairReader {
    type State = Slot;
    type Request = [RatingRef; 2];
    type Output = [RatingRef; 2];
    fn read(
        &self,
        _: &Slot,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        // Real additional row reads model the owner/title/history joins; the
        // old per-page path exceeded256 calls here (40 + 2*(116+6)).
        for _ in 0..40 {
            ctx.action_state(&self.ratings.jobs, &request[0].job())?
                .ok_or(OwnerReadError::Refused)?;
        }
        for reference in &request {
            self.ratings
                .complete_active_receipt(ctx, reference, &RatingCompatibility::combat_v1(hash(7)))
                .map_err(|_| OwnerReadError::Integrity)?;
        }
        Ok(request)
    }
}
struct Owner {
    handles: Handles,
}
impl GameModule for Owner {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let error = |e: Box<dyn std::error::Error>| ModError::msg(e.to_string());
        let expected = definition().map_err(error)?;
        reg.definition_validator(
            DefinitionKind::new("owner_read").map_err(|e| ModError::msg(e.to_string()))?,
            move |ctx: &DefinitionValidationContext<'_>| {
                let entry = ctx.definition();
                if entry.reference() == &expected
                    && entry.payload() == &json!({"format":1})
                    && entry.references().is_empty()
                    && !entry.is_current()
                {
                    Ok(())
                } else {
                    Err("exact complete-pair fixture definition".into())
                }
            },
        )?;
        reg.state(Slot);
        let ratings = RatingFamilies::register(reg)?;
        *self
            .handles
            .lock()
            .map_err(|_| ModError::msg("handles poisoned"))? = Some(ratings.clone());
        let reference = reg.definition(descriptor().map_err(error)?)?;
        reg.owner_read_service(
            reference,
            BTreeSet::from([OwnerId::new("consumer").map_err(|e| ModError::msg(e.to_string()))?]),
            BTreeSet::new(),
            PairReader { ratings },
        )
    }
}
struct Consumer {
    request: [RatingRef; 2],
    answers: Answers,
}
impl GameModule for Consumer {
    fn name(&self) -> &'static str {
        "consumer"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(CommandCompatibility::ReadOnly);
        let handle = reg.request_owner_read::<PairReader>(
            definition().map_err(|e| ModError::msg(e.to_string()))?,
        )?;
        let request = self.request;
        let answers = self.answers.clone();
        reg.command(
            CommandSpec {
                name: "readpair".into(),
                ..Default::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let answer = ctx.read_owner(&handle, request);
                answers
                    .lock()
                    .map_err(|_| CmdError::msg("answers poisoned"))?
                    .push(answer);
                Ok(())
            }),
        )
    }
}
struct Fixture {
    engine: Engine,
    handles: RatingFamilies,
    answers: Answers,
    evidence: [test_support::CompleteRating; 2],
}
impl Fixture {
    fn new() -> TestResult<Self> {
        let evidence = [complete(10)?, complete(11)?];
        let request = [
            evidence[0].receipt.reference()?,
            evidence[1].receipt.reference()?,
        ];
        let handles: Handles = Arc::new(Mutex::new(None));
        let answers: Answers = Arc::new(Mutex::new(Vec::new()));
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(Owner {
                    handles: handles.clone(),
                }),
                Box::new(Consumer {
                    request,
                    answers: answers.clone(),
                }),
            ],
        )?;
        let handles = handles
            .lock()
            .map_err(|_| "handles poisoned")?
            .clone()
            .ok_or("handles")?;
        for row in &evidence {
            let mut txn = engine.action_transaction(&handles.receipts)?;
            let job = row.job.job();
            txn.insert(&handles.receipts, &job, row.receipt.clone())?;
            txn.insert(&handles.jobs, &job, row.job.clone())?;
            txn.insert(&handles.executions, &job, row.execution.clone())?;
            txn.insert(&handles.completions, &job, row.completion.clone())?;
            txn.insert(&handles.status, &job, row.status.clone())?;
            for page in &row.pages {
                txn.insert(&handles.pages, &page.key(), page.clone())?;
            }
            let (_, install) = txn.prepare()?.into_parts();
            engine.install_prepared_action(install)?;
        }
        Ok(Self {
            engine,
            handles,
            answers,
            evidence,
        })
    }
    fn read(&mut self) -> TestResult<Result<[RatingRef; 2], OwnerReadError>> {
        self.engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "readpair".into(),
            },
            &mut Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        );
        Ok(self
            .answers
            .lock()
            .map_err(|_| "answers poisoned")?
            .pop()
            .ok_or("actual read")?)
    }
}
#[test]
fn two_full_complete_receipts_and_actual_join_reads_fit_existing_owner_limits() -> TestResult {
    let mut f = Fixture::new()?;
    let before = f.engine.action_state().len();
    let expected = [
        f.evidence[0].receipt.reference()?,
        f.evidence[1].receipt.reference()?,
    ];
    assert_eq!(f.read()?, Ok(expected));
    assert_eq!(f.engine.action_state().len(), before);
    assert!(!f.engine.action_state().has_prepared());
    Ok(())
}
#[test]
fn batching_preserves_last_page_attempt_binding_and_independent_current_status() -> TestResult {
    for mode in 0..3 {
        let mut f = Fixture::new()?;
        assert!(f.read()?.is_ok());
        let page = f.evidence[1].pages.last().ok_or("last page")?;
        let mut txn = f.engine.action_transaction(&f.handles.receipts)?;
        match mode {
            0 => txn.remove(&f.handles.pages, &page.key())?,
            1 => txn.replace(
                &f.handles.pages,
                &page.key(),
                f.evidence[0]
                    .pages
                    .last()
                    .ok_or("crossed attempt page")?
                    .clone(),
            )?,
            _ => {
                let status = RatingStatus {
                    standing: RatingStanding::Recalled {
                        operation: CommitToken::from_bytes([220; 16]),
                    },
                    ..f.evidence[0].status.clone()
                };
                txn.replace(&f.handles.status, &f.evidence[0].job.job(), status)?;
            }
        }
        let (_, install) = txn.prepare()?.into_parts();
        f.engine.install_prepared_action(install)?;
        assert_eq!(f.read()?, Err(OwnerReadError::Integrity));
    }
    Ok(())
}
