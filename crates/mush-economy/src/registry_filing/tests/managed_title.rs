use super::*;
use crate::{AllowAll, Encumbrance, InstrumentId, MapOnlyKeys};

fn unrestricted() -> ValidateEnv<'static> {
    ValidateEnv {
        auth: &AllowAll,
        custody: &NoCustody,
        facts: &NoFacts,
        now: 0,
        ic_now: 0,
        next_hold_id: 0,
        spent: BTreeMap::new(),
        keys: &MapOnlyKeys,
    }
}
fn plan(op: AssetOp) -> EconomyPlan {
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new("title_guard_test", "attempt"),
        PrincipalId::Object(GOD),
        EconomicCause::Ruleset {
            namespace: "ordinary".into(),
            correlation: "ordinary".into(),
        },
        vec![],
    );
    plan.asset_ops.push(op);
    plan
}

#[test]
fn actual_filed_charter_refuses_every_generic_title_and_collateral_escape() -> TestResult {
    let (world, mut state, adapter, actor, shell) = fixture(Amount::from_minor(100))?;
    let (_, patch, _) =
        adapter.prepare_images(&state, &world, actor, commit(), request(actor, shell))?;
    check!(!state.snapshot.managed_titles.contains(shell));
    Arc::new(*adapter).bind(&mut state, patch.into())?.apply();
    check!(state.snapshot.managed_titles.contains(shell));
    let before = state.snapshot.clone();
    let mut operations = vec![
        AssetOp::TransferTitle {
            object: shell,
            to: PrincipalId::Object(GOD),
            to_scope: None,
        },
        AssetOp::Title {
            object: shell,
            owner: PrincipalId::Object(GOD),
            scope: None,
            catalog: None,
        },
        AssetOp::SetCatalog {
            object: shell,
            catalog: None,
        },
        AssetOp::Abandon { object: shell },
        AssetOp::WriteOff {
            object: shell,
            reason: "erase policy".into(),
        },
    ];
    for encumbrance in [
        Encumbrance::Lien {
            creditor: PrincipalId::Object(GOD),
            amount: Amount::from_minor(1),
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
        },
        Encumbrance::Collateral {
            contract: crate::ContractId(9),
        },
        Encumbrance::Lease {
            lessor: PrincipalId::Object(actor),
            contract: crate::ContractId(9),
        },
    ] {
        operations.push(AssetOp::Encumber {
            object: shell,
            encumbrance,
        });
    }
    for operation in operations {
        check!(matches!(
            crate::validate(&plan(operation), &state.snapshot, &unrestricted()),
            PlanResult::Refused(crate::EconomyError::Unauthorized)
        ));
    }
    check_eq!(state.snapshot, before);
    check_eq!(state.journal.next_seq(JOURNAL_LOG), 1);
    Ok(())
}

#[test]
fn birth_identity_outlives_mutable_title_fields_and_catalogue_text_is_not_authority() -> TestResult
{
    let (world, mut state, adapter, actor, shell) = fixture(Amount::from_minor(100))?;
    let (_, patch, _) =
        adapter.prepare_images(&state, &world, actor, commit(), request(actor, shell))?;
    Arc::new(*adapter).bind(&mut state, patch.into())?.apply();
    let mut changed = state.snapshot.asset(shell).ok_or("title")?.clone();
    changed.catalog = None;
    changed.lifecycle = crate::AssetLifecycle::WrittenOff;
    changed.title_owner = PrincipalId::Object(GOD);
    state.snapshot.put_asset(changed.clone());
    check!(matches!(
        crate::validate(
            &plan(AssetOp::Title {
                object: shell,
                owner: PrincipalId::Object(actor),
                scope: None,
                catalog: None,
            }),
            &state.snapshot,
            &unrestricted()
        ),
        PlanResult::Refused(crate::EconomyError::Unauthorized)
    ));

    let ordinary = ObjId(shell.0 + 1);
    changed.object = ordinary;
    changed.lifecycle = crate::AssetLifecycle::Active;
    changed.catalog = Some(CatalogRef {
        ruleset: "roll".into(),
        code: "mind_charter".into(),
    });
    state.snapshot.put_asset(changed);
    check!(matches!(
        crate::validate(
            &plan(AssetOp::TransferTitle {
                object: ordinary,
                to: PrincipalId::Object(actor),
                to_scope: None,
            }),
            &state.snapshot,
            &unrestricted()
        ),
        PlanResult::Post(_)
    ));
    Ok(())
}

#[test]
fn managed_title_projection_requires_complete_actual_receipt_replay() -> TestResult {
    let (world, mut state, adapter, actor, shell) = fixture(Amount::from_minor(100))?;
    let (output, patch, _) =
        adapter.prepare_images(&state, &world, actor, commit(), request(actor, shell))?;
    Arc::new(*adapter).bind(&mut state, patch.into())?.apply();
    let mut audit = ReceiptAudit::default();
    audit.observe(
        state
            .transaction(output.transaction())
            .ok_or("transaction")?,
    );
    check!(crate::managed_title::ManagedTitleIndex::from_filings(&audit).is_err());
    audit.finish_replay()?;
    let rebuilt = crate::managed_title::ManagedTitleIndex::from_filings(&audit)?;
    check_eq!(rebuilt, state.snapshot.managed_titles);
    let decoded: ReceiptAudit = serde_json::from_slice(&serde_json::to_vec(&audit)?)?;
    check!(crate::managed_title::ManagedTitleIndex::from_filings(&decoded).is_err());
    Ok(())
}
