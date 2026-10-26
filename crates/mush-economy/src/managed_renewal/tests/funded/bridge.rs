#![cfg(test)]

//! Actual owner images and replay joins used by the production boot helper.
//! These do not replace the paired Roll/Market licence audits.
use super::*;
use crate::managed_renewal::bridge;
use mush_core::ModuleBootData;
use std::collections::HashMap;

fn actual_fold(r: &Rig) -> crate::fold::FoldState {
    let mut fold = crate::fold::FoldState::default();
    let mut entries = r.state.transactions.values().collect::<Vec<_>>();
    entries.sort_by_key(|tx| tx.journal_seq);
    for tx in entries {
        fold.fold_entry(tx.journal_seq, tx);
    }
    fold
}
fn projections(
    r: &Rig,
) -> (
    BTreeMap<ContractId, EconomicContract>,
    BTreeMap<crate::HoldId, crate::record::Hold>,
) {
    (
        r.state
            .snapshot
            .contracts()
            .map(|c| (c.id, c.clone()))
            .collect(),
        r.state
            .snapshot
            .holds()
            .map(|h| (h.id, h.clone()))
            .collect(),
    )
}
fn ordinary() -> mush_testkit::TestWorld {
    mush_testkit::TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(crate::EconomyModule::new()),
    ])
}
#[test]
fn actual_default_boot_proves_absence_without_new_serialized_history() -> TestResult {
    check!(
        EconomyStore::default()
            .require_no_renewal_history()
            .is_err()
    );
    let world = ordinary();
    let state = world
        .world()
        .module_state::<EconomyStore>()
        .ok_or("store")?;
    state.require_no_renewal_history()?;
    check!(
        state.renewal_audit().is_err(),
        "disabled empty boot is not combined renewal activation"
    );
    check!(
        serde_json::to_value(&state.fold)?
            .get("renewal_receipts")
            .is_none()
    );
    let mut frozen = state.clone();
    frozen.frozen = true;
    check!(frozen.require_no_renewal_history().is_err());
    Ok(())
}
#[test]
fn disabled_actual_boot_refuses_retained_offer_before_any_healing_write() -> TestResult {
    let mut r = Rig::new()?;
    let (_, writes) = r.publish()?;
    let kv = documents(&writes)?
        .into_iter()
        .map(|(key, bytes)| Ok((key.into(), serde_json::from_slice(&bytes)?)))
        .collect::<TestResult<Vec<_>>>()?;
    let mut world = ordinary();
    let _ = world.take_module_ops();
    let error = world
        .try_modules_loaded(HashMap::from([(
            "economy".into(),
            ModuleBootData {
                kv,
                ..Default::default()
            },
        )]))
        .err()
        .ok_or("retained renewal unexpectedly booted")?;
    check!(error.to_string().contains("complete recovery refused"));
    check!(world.take_module_ops().is_empty());
    Ok(())
}
#[test]
fn complete_boot_helper_joins_all_five_actual_owner_states() -> TestResult {
    for phase in 0..5 {
        let mut r = Rig::new()?;
        cash(&mut r)?;
        let (offer, writes) = r.publish()?;
        let req = request(&offer);
        let mut docs = documents(&writes)?;
        if phase == 4 {
            let (_, patch, writes) = install::close(
                &r.state,
                &r.context(true, token(7)),
                req.operation,
                UnfundedCloseReason::Declined,
            )?;
            patch.bind(&mut r.state)?.apply();
            docs = documents(&writes)?;
        } else if phase >= 1 {
            let p = prepared(&r, req.clone(), Transition::Fund, 4)?;
            install_money(&mut r, p)?;
            if phase >= 2 {
                let p = prepared(
                    &r,
                    req.clone(),
                    if phase == 2 {
                        Transition::Grant
                    } else {
                        Transition::Refund
                    },
                    5,
                )?;
                install_money(&mut r, p)?;
            }
        }
        let expected = r
            .state
            .snapshot
            .contract(req.contract)
            .ok_or("contract")?
            .clone();
        let (mut contracts, holds) = projections(&r);
        let mut snapshot = EconomySnapshot::new();
        let mut fold = actual_fold(&r);
        let book = bridge::recover(
            &mut fold,
            &mut snapshot,
            &mut contracts,
            &holds,
            &docs,
            true,
        )?;
        check!(
            contracts.is_empty(),
            "renewals must bypass legacy contract healing"
        );
        check!(snapshot.contract(req.contract) == Some(&expected));
        check_eq!(book.operations().len(), 1);
        check_eq!(
            fold.renewal_receipts.receipts()?.count(),
            usize::from((1..4).contains(&phase))
        );
        check!(book.combined_verified());
    }
    Ok(())
}
#[test]
fn boot_helper_rejects_independent_missing_crossed_and_extra_images() -> TestResult {
    let (mut r, req, docs) = funded(true)?;
    let p = prepared(&r, req.clone(), Transition::Grant, 5)?;
    install_money(&mut r, p)?;
    let actual_hold = r
        .state
        .fold
        .renewal_receipts
        .get(req.operation)?
        .funding()
        .hold();
    for case in 0..11 {
        let (mut contracts, mut holds) = projections(&r);
        let mut docs = docs.clone();
        let mut fold = actual_fold(&r);
        match case {
            0 => docs.clear(),
            1 => {
                contracts.remove(&req.contract);
            }
            2 => {
                holds.remove(&actual_hold);
            }
            3 => {
                holds.get_mut(&actual_hold).ok_or("hold")?.amount = Amount::from_minor(999);
            }
            4 => {
                fold.journal_holds.remove(&actual_hold);
            }
            5 => {
                fold.renewal_receipts = receipt::RenewalReceiptAudit::default();
            }
            6 => {
                let mut extra = holds.get(&actual_hold).ok_or("hold")?.clone();
                extra.id = crate::HoldId(999);
                holds.insert(extra.id, extra);
            }
            7 => {
                docs.push(docs.first().ok_or("document")?.clone());
            }
            8 => {
                docs.first_mut().ok_or("document")?.0 = history::key(token(999));
            }
            9 => {
                let mut extra = holds.get(&actual_hold).ok_or("hold")?.clone();
                extra.id = crate::HoldId(999);
                extra.holder = IdempotencyKey::new("unrelated", "orphan");
                fold.journal_holds.insert(extra.id, extra);
            }
            10 => {
                let mut extra = holds.get(&actual_hold).ok_or("hold")?.clone();
                extra.id = crate::HoldId(999);
                extra.holder = IdempotencyKey::new("unrelated", "orphan");
                holds.insert(extra.id, extra);
            }
            _ => unreachable!(),
        }
        let mut snapshot = EconomySnapshot::new();
        check!(
            bridge::recover(
                &mut fold,
                &mut snapshot,
                &mut contracts,
                &holds,
                &docs,
                true
            )
            .is_err(),
            "independent recovery corruption {case}"
        );
        check_eq!(snapshot.contracts().count(), 0);
    }
    Ok(())
}
#[test]
fn disabled_inventory_refuses_every_actual_evidence_source() -> TestResult {
    let (r, _, docs) = funded(false)?;
    for case in 0..5 {
        let (mut contracts, mut holds) = projections(&r);
        let mut fold = actual_fold(&r);
        let mut selected = docs.clone();
        if case != 0 {
            selected.clear();
        }
        if case != 1 {
            contracts.clear();
        }
        if case != 2 {
            holds.clear();
        }
        if case != 3 {
            fold.renewal_receipts = receipt::RenewalReceiptAudit::default();
        }
        if case != 4 {
            fold.journal_holds.clear();
        }
        check!(
            bridge::recover(
                &mut fold,
                &mut EconomySnapshot::new(),
                &mut contracts,
                &holds,
                &selected,
                false
            )
            .is_err(),
            "independent retained evidence {case}"
        );
    }
    Ok(())
}
#[test]
fn renewal_checkpoint_coverage_cannot_disappear_in_disabled_mode() -> TestResult {
    let (r, _, _) = funded(false)?;
    let mut fold = actual_fold(&r);
    fold.renewal_receipts.finish_replay()?;
    let rows = r
        .state
        .transactions
        .values()
        .map(|tx| crate::checkpoint::IdemRow {
            ns: tx.idempotency.namespace.clone(),
            key: tx.idempotency.key.clone(),
            tx: tx.id,
            seq: tx.journal_seq,
            posted_at: tx.posted_at,
            plan_hash: tx.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
        })
        .collect::<Vec<_>>();
    check!(bridge::checkpoint_complete(&fold, &rows, true));
    check!(!bridge::checkpoint_complete(&fold, &rows, false));
    check!(!bridge::checkpoint_complete(&fold, &[], true));
    let empty = crate::fold::FoldState::default();
    check!(!bridge::checkpoint_complete(&empty, &rows, false));
    check!(bridge::checkpoint_complete(&empty, &[], false));
    check!(!bridge::checkpoint_complete(&empty, &[], true));
    Ok(())
}
