//! Actual quota-owner transitions. These tests neither certify a source nor
//! substitute for the composed artifact/typed-owner publication tests.
use super::*;
use crate::minds::bulk::conduct_budget::{ConductBudget, TRIALS};
use mush_core::DefinitionDigest;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn plan() -> TestResult<Plan> {
    let digest = DefinitionDigest::for_canonical_value(
        "conduct.quota.fixture.v1",
        &serde_json::json!({"fixture":1}),
    )?;
    let cases = (0..TRIALS)
        .map(|index| ConductCase {
            index: index as u8,
            units: if index < 2 { 3 } else { 4 },
            native_ms: 250,
            maximum_ms: 120_000,
        })
        .collect();
    Ok(Plan {
        registry: None,
        conduct: Some(ConductBudget::prepare(digest, digest, digest, cases)?),
        kind: WorkKind::Conduct,
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
            bytes: 1024,
        },
        result_reservation_bytes: 256 * 1024,
    })
}
fn store() -> TestResult<(BulkStore, AccountId, ObjId)> {
    Ok((
        BulkStore::new(BulkConfig::default())?,
        AccountId::mint("conduct.quota.fixture", 1)?,
        ObjId(7),
    ))
}
fn apply(store: &mut BulkStore, change: Prepared) -> TestResult<Option<Lease>> {
    let operation = CommitToken::from_bytes(u128::from(change.expected).to_be_bytes());
    store.check_conduct(&change, operation)?;
    store.apply_conduct_checked(change, operation);
    Ok(store.take_conduct_lease(operation))
}
fn admit(store: &mut BulkStore, id: AccountId, actor: ObjId) -> TestResult<RunId> {
    let (run, change) =
        store.prepare_conduct_admit(id, actor, 1, plan()?, CommitToken::from_bytes([6; 16]), 0)?;
    apply(store, change)?;
    Ok(run)
}
fn settle(store: &mut BulkStore, lease: Lease, elapsed: u64, ending: ConductEnding) -> TestResult {
    let run = store.account_run(lease.account, lease.run)?;
    let cursor = run.plan.conduct.as_ref().ok_or("budget")?.advance(
        run.conduct.ok_or("cursor")?,
        elapsed,
        ending,
    )?;
    let receipt = Receipt {
        registry: None,
        conduct: Some(ConductProgress { cursor, ending }),
        actual_unit_ms: elapsed * lease.units,
        lease,
        result: None,
        terminal: None,
    };
    let change = store.prepare_finish(receipt, 1)?;
    apply(store, change)?;
    Ok(())
}

#[test]
fn conduct_admission_is_distinct_and_reserves_the_actual_complete_owner_document() -> TestResult {
    let (store, id, actor) = store()?;
    check!(store.prepare_admit(id, actor, 1, plan()?, 0).is_err());
    check!(
        store
            .prepare_registry_admit(id, actor, 1, plan()?, CommitToken::from_bytes([1; 16]), 0)
            .is_err()
    );
    let (run, change) =
        store.prepare_conduct_admit(id, actor, 1, plan()?, CommitToken::from_bytes([2; 16]), 0)?;
    check!(change.changes_conduct());
    check!(!change.changes_registry());
    check!(
        store
            .check_registry(&change, CommitToken::from_bytes([2; 16]))
            .is_err()
    );
    check_eq!(change.resulting_run(run).ok_or("run")?.registry_job, None);
    let ceiling = change
        .account
        .registry_document_ceiling()?
        .ok_or("ceiling")?;
    check!(change.bytes().len() < ceiling && ceiling <= MAX_ACCOUNT_DOCUMENT);
    let row = change.resulting_run(run).ok_or("run")?;
    check_eq!(row.charge()?, (3, 250, 120250));
    check_eq!(row.plan.maximum_unit_ms(), 120250 * (2 * 3 + 58 * 4));
    Ok(())
}
#[test]
fn actual_quota_yields_at_both_repetitions_then_charges_the_new_roster() -> TestResult {
    let (mut store, id, actor) = store()?;
    let run = admit(&mut store, id, actor)?;
    for trial in 0..2 {
        for step in 0..60 {
            let change = store.prepare_grant(id, run, 0, 2000)?;
            let lease = apply(&mut store, change)?.ok_or("lease")?;
            check_eq!(lease.units(), 3);
            settle(
                &mut store,
                lease,
                2000,
                if step == 59 {
                    ConductEnding::CaseComplete
                } else {
                    ConductEnding::Continue
                },
            )?;
        }
        check_eq!(
            store.account_run(id, run)?.conduct,
            Some(ConductCursor {
                index: trial + 1,
                simulated_ms: 0
            })
        );
    }
    check_eq!(store.account_run(id, run)?.charged_unit_ms, 720000);
    let change = store.prepare_grant(id, run, 0, 2000)?;
    let lease = apply(&mut store, change)?.ok_or("new rate")?;
    check_eq!((lease.units(), lease.allowance), (4, 8000));
    check_eq!(store.account_run(id, run)?.charged_unit_ms, 728000);
    Ok(())
}
#[test]
fn skipped_case_partial_completion_and_unknown_refunds_do_not_mutate_the_owner() -> TestResult {
    let (mut store, id, actor) = store()?;
    let run = admit(&mut store, id, actor)?;
    let change = store.prepare_grant(id, run, 0, 2000)?;
    let lease = apply(&mut store, change)?.ok_or("lease")?;
    let before = store.account(id)?.clone();
    let mut receipt = Receipt {
        registry: None,
        conduct: Some(ConductProgress {
            cursor: ConductCursor {
                index: 1,
                simulated_ms: 0,
            },
            ending: ConductEnding::CaseComplete,
        }),
        actual_unit_ms: 750,
        lease: lease.clone(),
        result: None,
        terminal: None,
    };
    check!(store.prepare_finish(receipt.clone(), 1).is_err());
    receipt.conduct = Some(ConductProgress {
        cursor: ConductCursor {
            index: 0,
            simulated_ms: 250,
        },
        ending: ConductEnding::Complete,
    });
    receipt.result = Some(Artifact {
        sha256: [2; 32],
        bytes: 1,
    });
    receipt.terminal = Some(Status::Complete);
    check!(store.prepare_finish(receipt.clone(), 1).is_err());
    receipt.conduct = None;
    receipt.result = None;
    receipt.terminal = Some(Status::Failed);
    check!(store.prepare_finish(receipt.clone(), 1).is_err());
    check_eq!(store.account(id)?, &before);
    receipt.actual_unit_ms = lease.allowance;
    let change = store.prepare_finish(receipt, 1)?;
    apply(&mut store, change)?;
    let row = store.account_run(id, run)?;
    check_eq!((row.status, row.charged_unit_ms), (Status::Failed, 6000));
    check!(row.result.is_none());
    Ok(())
}
#[test]
fn quota_day_handoff_and_stale_capsules_keep_exact_paid_flight() -> TestResult {
    let (mut store, id, actor) = store()?;
    let run = admit(&mut store, id, actor)?;
    let change = store.prepare_grant(id, run, 0, 2000)?;
    let stale = store.prepare_cancel(id, actor, run, 0)?;
    let lease = apply(&mut store, change)?.ok_or("lease")?;
    check!(
        store
            .check_conduct(&stale, CommitToken::from_bytes([9; 16]))
            .is_err()
    );
    check!(store.prepare_grant(id, run, DAY_MS, 2000).is_err());
    let receipt = Receipt {
        registry: None,
        conduct: lease.unchanged_conduct(),
        actual_unit_ms: 0,
        lease,
        result: None,
        terminal: None,
    };
    let change = store.prepare_finish(receipt, DAY_MS)?;
    apply(&mut store, change)?;
    check_eq!(store.account(id)?.spent_unit_ms, 0);
    let change = store.prepare_grant(id, run, DAY_MS, 2000)?;
    let lease = apply(&mut store, change)?.ok_or("new day")?;
    check_eq!(lease.utc_day(), 1);
    Ok(())
}
#[test]
fn completed_or_interrupted_conduct_history_cannot_be_trimmed_or_forgotten() -> TestResult {
    let (mut store, id, actor) = store()?;
    let run = admit(&mut store, id, actor)?;
    let change = store.prepare_cancel(id, actor, run, 1)?;
    apply(&mut store, change)?;
    check!(store.prepare_forget(id, actor, run).is_err());
    let legacy = store.prepare_legacy_recover(id, 2)?;
    check!(!legacy.changes_conduct());
    check_eq!(legacy.resulting_run(run), Some(store.account_run(id, run)?));
    store.cfg.completed_per_character = 1;
    // Proving retention limits must never deadlock Conduct admission, and
    // Conduct history remains retained and untrimmable across subsequent runs.
    let (second, admitted) =
        store.prepare_conduct_admit(id, actor, 2, plan()?, CommitToken::from_bytes([8; 16]), 2)?;
    check!(admitted.evicted().is_empty());
    apply(&mut store, admitted)?;
    check!(store.account_run(id, run).is_ok());
    check!(store.account_run(id, second).is_ok());
    check_eq!(store.conduct_runs()?.count(), 2);
    Ok(())
}
#[test]
fn actual_cold_quota_rejects_missing_identity_partial_complete_and_mismatched_profile() -> TestResult
{
    let (mut store, id, actor) = store()?;
    let run = admit(&mut store, id, actor)?;
    let actual = serde_json::to_value(store.account(id)?)?;
    for which in 0..4 {
        let mut wire = actual.clone();
        let row = wire
            .get_mut("runs")
            .and_then(|v| v.get_mut(run.0.to_string()))
            .ok_or("run")?;
        match which {
            0 => {
                row.as_object_mut().ok_or("object")?.remove("conduct_job");
            }
            1 => {
                row["conduct"]["index"] = serde_json::json!(60);
            }
            2 => {
                row["status"] = serde_json::json!("complete");
                row["finished_ms"] = serde_json::json!(1);
                row["result"] = serde_json::json!({"sha256":vec![1u8;32],"bytes":1});
                wire["active"] = serde_json::Value::Null;
                wire["reserved_unit_ms"] = serde_json::json!(0);
            }
            _ => {
                row["plan"]["kind"] = serde_json::json!("registry");
            }
        }
        let mut cold = BulkStore::new(BulkConfig::default())?;
        check_eq!(
            cold.load(id, &serde_json::to_vec(&wire)?),
            Err(BulkError::Quarantined)
        );
    }
    let mut cold = BulkStore::new(BulkConfig::default())?;
    cold.load(id, &serde_json::to_vec(&actual)?)?;
    check_eq!(cold.account_run(id, run)?, store.account_run(id, run)?);
    Ok(())
}
#[test]
fn old_plan_bytes_omit_every_conduct_field() -> TestResult {
    let mut p = plan()?;
    p.conduct = None;
    p.kind = WorkKind::Proving;
    p.per_side = 1;
    p.unit_count = 2;
    p.games = 10;
    p.seeds = (1..=5).collect();
    p.limit_ms = 1000;
    p.native_substep_ms = 250;
    p.validate()?;
    let actual = serde_json::to_value(&p)?;
    check!(actual.get("conduct").is_none());
    let decoded: Plan = serde_json::from_value(actual.clone())?;
    check_eq!(serde_json::to_value(decoded)?, actual);
    let (store, id, actor) = store()?;
    let (_, change) = store.prepare_admit(id, actor, 1, p, 0)?;
    let text = std::str::from_utf8(change.bytes())?;
    check!(!text.contains("conduct"));
    Ok(())
}

#[test]
fn p2_tsc_7_conduct_archival_separates_per_run_docs_and_roundtrips_fail_closed() -> TestResult {
    let cfg = BulkConfig {
        completed_per_character: 1,
        ..BulkConfig::default()
    };
    let mut store = BulkStore::new(cfg.clone())?;
    let id = AccountId::mint("p2-tsc-7-conduct-archive", 1)?;
    let actor = mush_core::GOD;
    let mut archived_docs: Vec<(RunId, Vec<u8>)> = Vec::new();
    let mut run_ids = Vec::new();

    // Run 8 sequential Conduct trials under completed_per_character = 1.
    for i in 1..=8_u8 {
        let job = CommitToken::from_bytes([i + 10; 16]);
        let (run, admitted) =
            store.prepare_conduct_admit(id, actor, u64::from(i), plan()?, job, i64::from(i))?;
        check!(admitted.evicted().is_empty());
        check!(admitted.changes_conduct());
        for (acct, archived_run) in admitted.archived_conduct() {
            let doc = ArchivedConductRun {
                format: CONDUCT_ARCHIVE_FORMAT,
                account: *acct,
                run: Box::new(archived_run.clone()),
            };
            archived_docs.push((archived_run.id, serde_json::to_vec(&doc)?));
        }
        apply(&mut store, admitted)?;
        let cancelled = store.prepare_cancel(id, actor, run, i64::from(i) + 50)?;
        apply(&mut store, cancelled)?;
        run_ids.push(run);
    }

    // Old terminal Conduct runs archived into separate KV docs while all 8
    // remain visible through conduct_runs() and run()/account_run().
    check!(!archived_docs.is_empty());
    check!(store.account(id)?.runs.len() <= 5);
    check_eq!(store.conduct_runs()?.count(), 8);
    for run in &run_ids {
        check!(store.run(id, actor, *run).is_ok());
        check!(store.account_run(id, *run).is_ok());
    }

    // Reloading Account + per-run archived Conduct docs restores all 8 runs.
    let account_bytes = serde_json::to_vec(store.account(id)?)?;
    let mut reboot = BulkStore::new(cfg.clone())?;
    reboot.load(id, &account_bytes)?;
    for (run_id, bytes) in &archived_docs {
        reboot.load_archived_conduct(id, *run_id, bytes)?;
    }
    check_eq!(reboot.conduct_runs()?.count(), 8);
    for run in &run_ids {
        check_eq!(reboot.account_run(id, *run)?, store.account_run(id, *run)?);
    }

    // Tampered Conduct archive doc fails closed and quarantines conduct_runs().
    let mut bad = BulkStore::new(cfg)?;
    bad.load(id, &account_bytes)?;
    check_eq!(
        bad.load_archived_conduct(id, archived_docs[0].0, b"{}"),
        Err(BulkError::Quarantined)
    );
    check!(bad.conduct_runs().is_err());
    Ok(())
}
