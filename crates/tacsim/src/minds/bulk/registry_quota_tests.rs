use super::*;
use crate::minds::bulk::registry_budget::RegistryBudget;
use mush_core::DefinitionDigest;
use mush_roll::commerce::rating::{CaseKey, CaseKind, ProbeName, RatingClass, TrialLayout};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn a_readable_near_cap_account_cannot_consume_the_registry_terminal_reserve() -> TestResult {
    let id = AccountId::mint("registry-near-cap", 1)?;
    let actor = ObjId(7);
    let store = BulkStore::new(BulkConfig::default())?;
    let (_, admitted) = store.prepare_registry_admit(
        id,
        actor,
        1,
        plan()?,
        CommitToken::from_bytes([29; 16]),
        0,
    )?;
    let opening = admitted.account;
    let mut historical = opening
        .runs
        .values()
        .next()
        .ok_or("active fixture")?
        .clone();
    historical.registry_job = None;
    historical.registry = None;
    historical.status = Status::Cancelled;
    historical.finished_ms = Some(0);
    historical.plan.registry = None;
    historical.plan.kind = WorkKind::Proving;
    historical.plan.per_side = 1;
    historical.plan.unit_count = 2;
    historical.plan.games = 100;
    historical.plan.seeds = (1..=50).collect();
    historical.plan.limit_ms = 10_000;
    historical.plan.native_substep_ms = 250;
    let account_with = |count: usize| -> Result<Account, Box<dyn std::error::Error>> {
        let mut account = opening.clone();
        for index in 0..count {
            let mut row = historical.clone();
            row.id = RunId(u64::try_from(index)? + 2);
            row.nonce = row.id.0;
            row.character = ObjId(7 + u32::try_from(index / 100)?);
            account.runs.insert(row.id, row);
        }
        account.request_watermark = u64::try_from(count)? + 1;
        account.next_run = account.request_watermark + 1;
        Ok(account)
    };
    let (mut low, mut high) = (0usize, 6300usize);
    assert!(serde_json::to_vec(&account_with(high)?)?.len() > MAX_ACCOUNT_DOCUMENT);
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        if serde_json::to_vec(&account_with(middle)?)?.len() <= MAX_ACCOUNT_DOCUMENT {
            low = middle;
        } else {
            high = middle;
        }
    }
    let mut account = account_with(low)?;
    let mut bytes = serde_json::to_vec(&account)?.len();
    // Valid hash byte values let this fixture approach the real encoded limit
    // without adding an unrecognized padding field or violating record bounds.
    'fill: for row in account
        .runs
        .values_mut()
        .filter(|row| row.status.terminal())
    {
        for byte in &mut row.plan.start.sha256 {
            if bytes + 2 > MAX_ACCOUNT_DOCUMENT - 32 {
                break 'fill;
            }
            *byte = 255;
            bytes += 2; // fixture byte1 has one decimal digit;255 has three
        }
    }
    account.validate(&store.cfg)?;
    let encoded = serde_json::to_vec(&account)?;
    assert_eq!(encoded.len(), bytes);
    assert!(encoded.len() <= MAX_ACCOUNT_DOCUMENT);
    assert!(
        account
            .registry_document_ceiling()?
            .ok_or("reserved ceiling")?
            > MAX_ACCOUNT_DOCUMENT
    );
    assert!(store.prepared(account, Vec::new(), None).is_err());
    assert!(
        store.accounts.is_empty(),
        "refused preparation installs no owner image"
    );
    let mut boot = BulkStore::new(BulkConfig::default())?;
    assert_eq!(boot.load(id, &encoded), Err(BulkError::Quarantined));
    assert!(boot.accounts.is_empty());
    Ok(())
}

#[test]
fn registry_job_binding_and_terminal_growth_are_owned_by_the_actual_quota_store() -> TestResult {
    let id = AccountId::mint("registry-capacity", 1)?;
    let actor = ObjId(7);
    let job = CommitToken::from_bytes([19; 16]);
    let mut store = BulkStore::new(BulkConfig::default())?;
    assert!(store.prepare_admit(id, actor, 1, plan()?, 0).is_err());
    let (run, admitted) = store.prepare_registry_admit(id, actor, 1, plan()?, job, 0)?;
    assert!(admitted.changes_registry());
    let ceiling = admitted
        .account
        .registry_document_ceiling()?
        .ok_or("reserved ceiling")?;
    assert!(ceiling > admitted.bytes().len());
    assert_eq!(
        admitted.resulting_run(run).ok_or("run")?.registry_job,
        Some(job)
    );
    let original = admitted.bytes().to_vec();
    let mut missing: serde_json::Value = serde_json::from_slice(&original)?;
    missing
        .get_mut("runs")
        .and_then(|v| v.get_mut(run.0.to_string()))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("stored run")?
        .remove("registry_job");
    let mut boot = BulkStore::new(BulkConfig::default())?;
    assert!(boot.load(id, &serde_json::to_vec(&missing)?).is_err());
    boot.load(id, &original)?;
    assert_eq!(boot.account_run(id, run)?.registry_job, Some(job));
    publish(&mut store, admitted)?;
    let granted = store.prepare_grant(id, run, 0, 2000)?;
    assert!(granted.changes_registry());
    assert!(granted.bytes().len() <= ceiling);
    let lease = publish(&mut store, granted)?.ok_or("paid lease")?;
    let cancelled = store.prepare_cancel(id, actor, run, 1)?;
    assert!(cancelled.bytes().len() <= ceiling);
    publish(&mut store, cancelled)?;
    let finished = store.prepare_finish(
        Receipt {
            registry: None,
            actual_unit_ms: lease.allowance,
            lease,
            result: None,
            terminal: Some(Status::Failed),
        },
        i64::MAX,
    )?;
    assert!(finished.changes_registry());
    assert!(finished.bytes().len() <= ceiling);
    assert_eq!(finished.account.registry_document_ceiling()?, None);
    assert_eq!(
        finished
            .resulting_run(run)
            .ok_or("terminal run")?
            .registry_job,
        Some(job)
    );
    Ok(())
}

#[test]
fn ordinary_pin_writes_cannot_change_or_release_an_active_registry_document_ceiling() -> TestResult
{
    let id = AccountId::mint("registry-ordinary-capacity", 1)?;
    let actor = ObjId(7);
    let mut store = BulkStore::new(BulkConfig::default())?;
    let mut proving = plan()?;
    proving.registry = None;
    proving.kind = WorkKind::Proving;
    proving.per_side = 1;
    proving.unit_count = 2;
    proving.games = 10;
    proving.seeds = vec![1, 2, 3, 4, 5];
    proving.limit_ms = 10_000;
    proving.native_substep_ms = 250;
    let (old, p) = store.prepare_admit(id, actor, 1, proving, 0)?;
    assert!(!p.changes_registry());
    assert!(
        serde_json::to_value(p.resulting_run(old).ok_or("M6 run")?)?
            .get("registry_job")
            .is_none()
    );
    publish(&mut store, p)?;
    let p = store.prepare_cancel(id, actor, old, 1)?;
    publish(&mut store, p)?;
    let (run, p) = store.prepare_registry_admit(
        id,
        actor,
        2,
        plan()?,
        CommitToken::from_bytes([20; 16]),
        2,
    )?;
    publish(&mut store, p)?;
    let before = store
        .account(id)?
        .registry_document_ceiling()?
        .ok_or("reserved bytes")?;
    let pinned = store.prepare_pin(id, actor, old, true)?;
    assert!(!pinned.changes_registry());
    assert!(
        pinned
            .account
            .registry_document_ceiling()?
            .ok_or("preserved reserve")?
            >= before
    );
    assert_eq!(
        pinned.resulting_run(run).ok_or("active run")?.registry_job,
        Some(CommitToken::from_bytes([20; 16]))
    );
    Ok(())
}
fn plan() -> Result<Plan, Box<dyn std::error::Error>> {
    let hash = DefinitionDigest::from_bytes([1; 32]);
    let cases = vec![
        RegistryCase {
            key: CaseKey::new(
                RatingClass::I,
                CaseKind::Trial {
                    variant: 0,
                    layout: TrialLayout::Duel,
                    seed: 1,
                    swapped: false,
                },
            )?,
            units: 2,
            native_ms: 250,
            maximum_ms: 10_000,
        },
        RegistryCase {
            key: CaseKey::new(
                RatingClass::I,
                CaseKind::Probe {
                    pair: 0,
                    name: ProbeName::DensityArmed,
                },
            )?,
            units: 33,
            native_ms: 250,
            maximum_ms: 10_000,
        },
    ];
    Ok(Plan {
        registry: Some(RegistryBudget::prepare(hash, hash, hash, cases)?),
        kind: WorkKind::Registry,
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
fn publish(store: &mut BulkStore, change: Prepared) -> Result<Option<Lease>, BulkError> {
    store.check(&change)?;
    Ok(store.apply_checked(change))
}

#[test]
fn registry_leases_charge_the_current_roster_and_refund_only_exact_known_work() -> TestResult {
    let id = AccountId::mint("registry-quota", 1)?;
    let actor = ObjId(7);
    let mut store = BulkStore::new(BulkConfig::default())?;
    let (run, change) =
        store.prepare_registry_admit(id, actor, 1, plan()?, CommitToken::from_bytes([1; 16]), 0)?;
    publish(&mut store, change)?;
    let change = store.prepare_grant(id, run, 0, 2000)?;
    let lease = publish(&mut store, change)?.ok_or("lease")?;
    assert_eq!(lease.units(), 2);
    assert_eq!(lease.allowance, 4000);
    let receipt = Receipt {
        registry: Some(RegistryProgress {
            cursor: RegistryCursor {
                index: 1,
                simulated_ms: 0,
            },
            ending: RegistryEnding::CaseComplete,
        }),
        lease,
        actual_unit_ms: 500,
        result: None,
        terminal: None,
    };
    let change = store.prepare_finish(receipt, 1)?;
    publish(&mut store, change)?;
    assert_eq!(store.account(id)?.spent_unit_ms, 500);
    let change = store.prepare_grant(id, run, 1, 2000)?;
    let lease = publish(&mut store, change)?.ok_or("second lease")?;
    assert_eq!(lease.units(), 33);
    assert_eq!(lease.allowance, 66_000);
    let before = store.account(id)?.clone();
    let wrong = Receipt {
        registry: Some(RegistryProgress {
            cursor: RegistryCursor {
                index: 1,
                simulated_ms: 251,
            },
            ending: RegistryEnding::Continue,
        }),
        lease: lease.clone(),
        actual_unit_ms: 8250,
        result: None,
        terminal: None,
    };
    assert!(store.prepare_finish(wrong, 2).is_err());
    assert_eq!(
        *store.account(id)?,
        before,
        "refusal changes no quota or cursor"
    );
    let receipt = Receipt {
        registry: Some(RegistryProgress {
            cursor: RegistryCursor {
                index: 1,
                simulated_ms: 250,
            },
            ending: RegistryEnding::Continue,
        }),
        lease,
        actual_unit_ms: 8250,
        result: None,
        terminal: None,
    };
    let change = store.prepare_finish(receipt.clone(), 2)?;
    publish(&mut store, change)?;
    assert_eq!(store.account(id)?.spent_unit_ms, 8750);
    assert!(
        store.prepare_finish(receipt, 2).is_err(),
        "paid receipt is consumed once"
    );
    Ok(())
}

#[test]
fn registry_cancellation_keeps_dispatched_charge_until_worker_exit_and_restart_is_conservative()
-> TestResult {
    let id = AccountId::mint("registry-cancel", 1)?;
    let actor = ObjId(7);
    let mut store = BulkStore::new(BulkConfig::default())?;
    let (run, change) =
        store.prepare_registry_admit(id, actor, 1, plan()?, CommitToken::from_bytes([1; 16]), 0)?;
    publish(&mut store, change)?;
    let change = store.prepare_grant(id, run, 0, 2000)?;
    let lease = publish(&mut store, change)?.ok_or("lease")?;
    let change = store.prepare_cancel(id, actor, run, 1)?;
    publish(&mut store, change)?;
    assert!(store.account_run(id, run)?.flight.is_some());
    assert_eq!(store.account(id)?.spent_unit_ms, 4000);
    let receipt = Receipt {
        registry: None,
        actual_unit_ms: lease.allowance,
        lease,
        result: None,
        terminal: Some(Status::Failed),
    };
    let change = store.prepare_finish(receipt, 2)?;
    publish(&mut store, change)?;
    assert_eq!(store.account_run(id, run)?.status, Status::Cancelled);
    assert_eq!(store.account(id)?.spent_unit_ms, 4000);
    assert_eq!(store.account(id)?.reserved_unit_ms, 0);
    let (other, change) =
        store.prepare_registry_admit(id, actor, 2, plan()?, CommitToken::from_bytes([2; 16]), 3)?;
    publish(&mut store, change)?;
    let change = store.prepare_grant(id, other, 3, 2000)?;
    publish(&mut store, change)?;
    let before = store.account(id)?.spent_unit_ms;
    let change = store.prepare_recover(id, 4)?;
    publish(&mut store, change)?;
    assert_eq!(store.account(id)?.spent_unit_ms, before);
    assert_eq!(store.account_run(id, other)?.status, Status::Interrupted);
    Ok(())
}

#[test]
fn registry_case_transition_cannot_skip_a_same_class_case_or_relabel_partial_coverage_certified()
-> TestResult {
    let budget = plan()?.registry.ok_or("budget")?;
    let start = RegistryCursor::default();
    assert!(
        budget
            .advance(start, 250, RegistryEnding::Certified)
            .is_err()
    );
    assert!(
        budget
            .advance(start, 250, RegistryEnding::ClassBudget)
            .is_err(),
        "no higher class was admitted"
    );
    assert!(
        budget
            .advance(start, 10_251, RegistryEnding::Continue)
            .is_err()
    );
    let next = budget.advance(start, 250, RegistryEnding::CaseComplete)?;
    assert_eq!(
        next,
        RegistryCursor {
            index: 1,
            simulated_ms: 0
        }
    );
    Ok(())
}

#[test]
fn unknown_registry_failure_cannot_refund_and_known_failure_keeps_exact_progress() -> TestResult {
    let id = AccountId::mint("registry-failure", 1)?;
    let actor = ObjId(7);
    let mut store = BulkStore::new(BulkConfig::default())?;
    let (run, change) =
        store.prepare_registry_admit(id, actor, 1, plan()?, CommitToken::from_bytes([1; 16]), 0)?;
    publish(&mut store, change)?;
    let change = store.prepare_grant(id, run, 0, 2000)?;
    let lease = publish(&mut store, change)?.ok_or("lease")?;
    let before = store.account(id)?.clone();
    for actual_unit_ms in [0, 500, lease.allowance - 1] {
        let unknown = Receipt {
            registry: None,
            lease: lease.clone(),
            actual_unit_ms,
            result: None,
            terminal: Some(Status::Failed),
        };
        assert!(store.prepare_finish(unknown, 1).is_err());
        assert_eq!(
            *store.account(id)?,
            before,
            "unknown work cannot release a precharge"
        );
    }
    let known = Receipt {
        registry: Some(RegistryProgress {
            cursor: RegistryCursor {
                index: 0,
                simulated_ms: 250,
            },
            ending: RegistryEnding::Continue,
        }),
        lease,
        actual_unit_ms: 500,
        result: None,
        terminal: Some(Status::Failed),
    };
    let change = store.prepare_finish(known, 1)?;
    publish(&mut store, change)?;
    let account = store.account(id)?;
    assert_eq!(account.spent_unit_ms, 500);
    assert_eq!(account.reserved_unit_ms, 0);
    let row = store.account_run(id, run)?;
    assert_eq!(row.status, Status::Failed);
    assert_eq!(
        row.registry,
        Some(RegistryCursor {
            index: 0,
            simulated_ms: 250
        })
    );
    assert!(row.flight.is_none());
    Ok(())
}

#[test]
fn registry_cancel_wins_over_a_measured_completion_without_losing_its_charge() -> TestResult {
    let id = AccountId::mint("registry-cancel-complete", 1)?;
    let actor = ObjId(7);
    let job = CommitToken::from_bytes([31; 16]);
    let mut store = BulkStore::new(BulkConfig::default())?;
    let mut admitted_plan = plan()?;
    let budget = admitted_plan.registry.as_ref().ok_or("Registry budget")?;
    let first = budget.case(RegistryCursor::default())?.clone();
    admitted_plan.registry = Some(RegistryBudget::prepare(
        budget.subject,
        budget.compatibility,
        budget.schedule,
        vec![first],
    )?);
    let (run, admission) = store.prepare_registry_admit(id, actor, 1, admitted_plan, job, 0)?;
    publish(&mut store, admission)?;
    let granted = store.prepare_grant(id, run, 0, 2000)?;
    let lease = publish(&mut store, granted)?.ok_or("paid lease")?;
    let cancelling = store.prepare_cancel(id, actor, run, 1)?;
    publish(&mut store, cancelling)?;
    assert_eq!(store.account_run(id, run)?.status, Status::CancelRequested);
    let observed = RegistryProgress {
        cursor: RegistryCursor {
            index: 0,
            simulated_ms: 250,
        },
        ending: RegistryEnding::Certified,
    };
    let receipt = Receipt {
        lease,
        registry: Some(observed),
        actual_unit_ms: 500,
        result: Some(Artifact {
            sha256: [32; 32],
            bytes: 100,
        }),
        terminal: Some(Status::Complete),
    };
    let finished = store.prepare_finish(receipt, 2)?;
    publish(&mut store, finished)?;
    let row = store.account_run(id, run)?;
    assert_eq!(row.status, Status::Cancelled);
    assert_eq!(row.charged_unit_ms, 500);
    assert_eq!(row.registry, Some(observed.cursor));
    assert!(row.result.is_none());
    assert!(row.flight.is_none());
    let exact = crate::minds::bulk::registry_projection::observation(row)?;
    assert_eq!(
        exact.phase,
        mush_roll::commerce::rating::execution::ExecutionPhase::Cancelled
    );
    assert!(exact.result.is_none());
    let account = store.account(id)?;
    assert_eq!(account.spent_unit_ms, 500);
    assert_eq!(account.reserved_unit_ms, 0);
    assert!(account.active.is_none());
    Ok(())
}

#[test]
fn maximum_class_rejection_is_new_admission_only_and_old_budget_bytes_stay_exact() -> TestResult {
    let hash = DefinitionDigest::from_bytes([8; 32]);
    let case = RegistryCase {
        key: CaseKey::new(
            RatingClass::III,
            CaseKind::Probe {
                pair: 0,
                name: ProbeName::EmptyArmed,
            },
        )?,
        units: 1,
        native_ms: 250,
        maximum_ms: 1000,
    };
    let budget = RegistryBudget::prepare(hash, hash, hash, vec![case.clone()])?;
    let from = RegistryCursor::default();
    let next = budget.advance(from, 250, RegistryEnding::Rejected)?;
    assert_eq!(
        next,
        RegistryCursor {
            index: 0,
            simulated_ms: 250
        }
    );
    let mut old = serde_json::to_value(&budget)?;
    old.as_object_mut()
        .ok_or("budget")?
        .remove("measured_rejection");
    let historical: RegistryBudget = serde_json::from_value(old.clone())?;
    assert_eq!(serde_json::to_value(&historical)?, old);
    assert!(
        historical
            .advance(from, 250, RegistryEnding::Rejected)
            .is_err()
    );
    assert!(
        historical
            .advance(from, 250, RegistryEnding::Certified)
            .is_ok()
    );
    assert_ne!(historical.digest()?, budget.digest()?);
    let mut lower = case.clone();
    lower.key = CaseKey::new(
        RatingClass::II,
        CaseKind::Probe {
            pair: 0,
            name: ProbeName::EmptyArmed,
        },
    )?;
    let lower = RegistryBudget::prepare(hash, hash, hash, vec![lower])?;
    assert!(lower.advance(from, 250, RegistryEnding::Rejected).is_err());
    let mut kernel = case;
    kernel.key = CaseKey::new(
        RatingClass::III,
        CaseKind::Kernel {
            name: mush_roll::commerce::rating::KernelName::MemoryKeysAt,
        },
    )?;
    let kernel = RegistryBudget::prepare(hash, hash, hash, vec![kernel])?;
    assert!(kernel.advance(from, 250, RegistryEnding::Rejected).is_err());
    Ok(())
}

#[test]
fn partial_maximum_class_rejection_finishes_and_recovers_actual_quota_without_certifying()
-> TestResult {
    let id = AccountId::mint("registry-rejection-terminal", 1)?;
    let actor = ObjId(7);
    let job = CommitToken::from_bytes([41; 16]);
    let hash = DefinitionDigest::from_bytes([42; 32]);
    let cases = (0..2)
        .map(|pair| {
            Ok(RegistryCase {
                key: CaseKey::new(
                    RatingClass::III,
                    CaseKind::Probe {
                        pair,
                        name: ProbeName::EmptyArmed,
                    },
                )?,
                units: 1,
                native_ms: 250,
                maximum_ms: 1000,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let budget = RegistryBudget::prepare(hash, hash, hash, cases)?;
    let cursor = RegistryCursor {
        index: 0,
        simulated_ms: 250,
    };
    assert!(!budget.can_certify(cursor)?, "partial class cannot certify");
    assert!(budget.can_reject(cursor)?);
    let mut admitted_plan = plan()?;
    admitted_plan.registry = Some(budget);
    let mut store = BulkStore::new(BulkConfig::default())?;
    let (run, admitted) = store.prepare_registry_admit(id, actor, 1, admitted_plan, job, 0)?;
    publish(&mut store, admitted)?;
    let granted = store.prepare_grant(id, run, 0, 2000)?;
    let lease = publish(&mut store, granted)?.ok_or("actual paid lease")?;
    let before = store.account_run(id, run)?.clone();
    let finished = store.prepare_finish(
        Receipt {
            registry: Some(RegistryProgress {
                cursor,
                ending: RegistryEnding::Rejected,
            }),
            lease,
            actual_unit_ms: 250,
            result: Some(Artifact {
                sha256: [43; 32],
                bytes: 100,
            }),
            terminal: Some(Status::Complete),
        },
        1,
    )?;
    assert_eq!(store.account_run(id, run)?, &before, "prepare stays inert");
    let bytes = finished.bytes().to_vec();
    publish(&mut store, finished)?;
    let row = store.account_run(id, run)?;
    assert_eq!(row.status, Status::Complete);
    assert_eq!(row.registry, Some(cursor));
    assert_eq!(row.charged_unit_ms, 250);
    assert!(row.flight.is_none());
    let observed = crate::minds::bulk::registry_projection::observation(row)?;
    assert_eq!(
        observed.phase,
        mush_roll::commerce::rating::execution::ExecutionPhase::Complete
    );
    assert!(observed.result.is_some());
    assert_eq!(store.account(id)?.reserved_unit_ms, 0);
    let mut recovered = BulkStore::new(BulkConfig::default())?;
    recovered.load(id, &bytes)?;
    assert_eq!(recovered.account_run(id, run)?, row);
    // Historical admissions never promised rejection. Removing the opt-in
    // must still reject this partial-class terminal state during native load.
    let mut old: serde_json::Value = serde_json::from_slice(&bytes)?;
    old["runs"][run.0.to_string()]["plan"]["registry"]
        .as_object_mut()
        .ok_or("budget")?
        .remove("measured_rejection");
    let mut denied = BulkStore::new(BulkConfig::default())?;
    assert!(denied.load(id, &serde_json::to_vec(&old)?).is_err());
    assert!(denied.accounts.is_empty());
    Ok(())
}
