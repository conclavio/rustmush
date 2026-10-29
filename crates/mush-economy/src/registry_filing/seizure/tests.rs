#![cfg(test)]

//! Doc 17h cut M8d2 S1: the closed seizure plan's guard, its receipts and their audit,
//! driven through the real engine (`validate_core_seizure`, `apply_delta`) over a snapshot
//! holding one managed core. The participant's prepare is the composed rig's to drive.
use super::*;
use crate::engine::MapOnlyKeys;
use crate::record::AssetLifecycle;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const CORE: ObjId = ObjId(70);
const HULL: ObjId = ObjId(60);
const HOLDER: ObjId = ObjId(50);
const OPERATOR: ObjId = ObjId(40);
const REGISTRY: OrgId = OrgId(9);

fn issuer() -> SystemPrincipalId {
    SystemPrincipalId::new("roll.registry").expect("an issuer")
}
fn commit(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn birth() -> crate::managed_title::TitleBirth {
    crate::managed_title::TitleBirth::core(commit(1), TransactionId(1))
}
fn core_title() -> EconomicAsset {
    EconomicAsset {
        object: CORE,
        title_owner: PrincipalId::Object(HOLDER),
        scope: None,
        assignment: None,
        maintenance_account: None,
        catalog: None,
        encumbrances: Vec::new(),
        acquisition: None,
        lifecycle: AssetLifecycle::Active,
        last_journal_seq: 3,
        revision: 4,
        stolen: None,
        frozen: false,
        frozen_reason: None,
    }
}
fn snapshot() -> EconomySnapshot {
    let mut snapshot = EconomySnapshot::default();
    snapshot.put_asset(core_title());
    snapshot.managed_titles =
        crate::managed_title::ManagedTitleIndex::Ready(BTreeMap::from([(CORE, birth())]));
    snapshot
}
fn proof(seq: u64) -> InspectionProof {
    InspectionProof {
        key: inspection_key(HULL, seq),
        hull: HULL,
        core: CORE,
        seq,
    }
}
fn claimant() -> PrincipalId {
    PrincipalId::Organization(REGISTRY)
}
fn config() -> SeizureConfig {
    // `plan` reads only the claimant; the action references are the composed rig's.
    let reference = |id: &str| {
        mush_core::DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(ACTION_OWNER).expect("an owner"),
                mush_core::DefinitionDescriptor::new(
                    mush_core::DefinitionKind::new("action").expect("a kind"),
                    mush_core::DefinitionId::new(id).expect("an id"),
                    1,
                    serde_json::json!({"fixture": id}),
                )
                .expect("a descriptor"),
            )
            .expect("registers")
    };
    SeizureConfig {
        seize: reference(SEIZE_ACTION_ID),
        forfeit: reference(FORFEIT_ACTION_ID),
        claimant: REGISTRY,
    }
}
fn policy() -> InitialFilingPolicy {
    InitialFilingPolicy::new(
        issuer(),
        AccountId(2),
        Amount::ZERO,
        CatalogRef {
            ruleset: "roll".into(),
            code: "charter".into(),
        },
    )
    .expect("a policy")
}
fn request(step: SeizureStep, revision: u64) -> RegistryCoreSeizure {
    RegistryCoreSeizure {
        step,
        core: CORE,
        title_revision: revision,
    }
}
fn closed(
    snapshot: &EconomySnapshot,
    commit_n: u8,
    r: &RegistryCoreSeizure,
    shape: Shape,
) -> ClosedSeizurePlan {
    ClosedSeizurePlan {
        plan: plan(
            &config(),
            &policy(),
            (OPERATOR, commit(commit_n), 2_000),
            r,
            &shape,
        ),
        before_title: snapshot.asset(CORE).cloned().expect("the title"),
        birth: birth(),
        claimant: claimant(),
        shape,
    }
}
/// Validate and apply one closed plan through the real engine, as the participant does.
fn post(
    snapshot: &mut EconomySnapshot,
    proof: &ClosedSeizurePlan,
    id: u64,
) -> std::result::Result<EconomicTransaction, crate::EconomyError> {
    let auth = SeizureAuth {
        issuer: PrincipalId::System(issuer()),
        claimant: claimant(),
        core: CORE,
        ops: proof.shape.ops(CORE, &claimant()),
    };
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: 1_000,
        ic_now: 2_000,
        next_hold_id: 1,
        spent: BTreeMap::new(),
        keys: &MapOnlyKeys,
    };
    match crate::engine::validate_core_seizure(proof, snapshot, &env) {
        PlanResult::Post(prepared) => {
            let (delta, tx) =
                crate::engine::apply_delta(snapshot, &prepared, TransactionId(id), id, 1_000);
            delta.apply_to(snapshot);
            Ok(tx)
        }
        PlanResult::Refused(error) => Err(error),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) => {
            Err(crate::EconomyError::Unauthorized)
        }
    }
}
fn seize_shape(seq: u64) -> Shape {
    Shape::Seize {
        reason: inspection_key(HULL, seq),
    }
}

#[test]
fn m8d2_the_inspection_key_and_the_proof_shape() -> Result {
    check_eq!(inspection_key(ObjId(42), 3), "hull/0000002a/inspection/3");
    check!(proof(3).well_formed());
    for bad in [
        InspectionProof {
            key: "hull/0000002a/inspection/3".into(),
            ..proof(3)
        },
        InspectionProof { seq: 0, ..proof(3) },
        InspectionProof {
            hull: CORE,
            ..proof(3)
        },
        InspectionProof {
            key: format!("{} ", inspection_key(HULL, 3)),
            ..proof(3)
        },
    ] {
        check!(!bad.well_formed(), "{bad:?}");
    }
    Ok(())
}

#[test]
fn m8d2_the_closed_plan_refuses_every_mismatch() -> Result {
    let snapshot = snapshot();
    let r = request(SeizureStep::Seize(proof(3)), 4);
    let good = closed(&snapshot, 5, &r, seize_shape(3));
    good.check(good.plan(), &snapshot)?;
    check_eq!(
        good.plan().asset_ops,
        vec![AssetOp::Encumber {
            object: CORE,
            encumbrance: disputed(&claimant(), "hull/0000003c/inspection/3"),
        }],
        "exactly one Encumber, the Registry org's Disputed claim with the inspection key"
    );
    type Change = Box<dyn Fn(&mut EconomyPlan)>;
    let plan_changes: [(&str, Change); 6] = [
        (
            "a second op",
            Box::new(|p| {
                p.asset_ops.push(AssetOp::Unassign { object: CORE });
            }),
        ),
        (
            "a wrong reason",
            Box::new(|p| {
                p.asset_ops = vec![AssetOp::Encumber {
                    object: CORE,
                    encumbrance: disputed(&claimant(), &inspection_key(HULL, 4)),
                }];
            }),
        ),
        (
            "another claimant",
            Box::new(|p| {
                p.asset_ops = vec![AssetOp::Encumber {
                    object: CORE,
                    encumbrance: disputed(
                        &PrincipalId::Organization(OrgId(10)),
                        &inspection_key(HULL, 3),
                    ),
                }];
            }),
        ),
        (
            "a money line",
            Box::new(|p| {
                p.requested_lines.push(RequestedLine {
                    account: AccountId(2),
                    delta: Amount::from_minor(1),
                    memo: "x".into(),
                });
            }),
        ),
        (
            "another namespace",
            Box::new(|p| p.idempotency = IdempotencyKey::new("roll.charter_transfer.v1", "x")),
        ),
        (
            "a transfer instead",
            Box::new(|p| {
                p.asset_ops = vec![AssetOp::TransferTitle {
                    object: CORE,
                    to: claimant(),
                    to_scope: None,
                }];
            }),
        ),
    ];
    for (why, change) in plan_changes {
        let mut plan = good.plan().clone();
        change(&mut plan);
        check!(good.check(&plan, &snapshot).is_err(), "{why}");
    }
    // A wrong before-title image.
    let mut moved = snapshot.clone();
    let mut title = core_title();
    title.revision = 5;
    moved.put_asset(title);
    check!(
        good.check(good.plan(), &moved).is_err(),
        "a wrong title image"
    );
    // A wrong birth: another core birth, a charter birth, no birth.
    for index in [
        BTreeMap::from([(
            CORE,
            crate::managed_title::TitleBirth::core(commit(2), TransactionId(1)),
        )]),
        BTreeMap::from([(
            CORE,
            crate::managed_title::TitleBirth::charter(commit(1), TransactionId(1)),
        )]),
        BTreeMap::new(),
    ] {
        let mut other = snapshot.clone();
        other.managed_titles = crate::managed_title::ManagedTitleIndex::Ready(index);
        check!(good.check(good.plan(), &other).is_err(), "a wrong birth");
    }
    Ok(())
}

#[test]
fn m8d2_seize_then_forfeit_through_the_engine_and_the_audit() -> Result {
    let mut snapshot = snapshot();
    let mut audit = SeizureReceiptAudit::default();
    // Seize.
    let r = request(SeizureStep::Seize(proof(3)), 4);
    let seize = closed(&snapshot, 5, &r, seize_shape(3));
    let tx = post(&mut snapshot, &seize, 7)?;
    let held = snapshot.asset(CORE).cloned().ok_or("the title")?;
    check_eq!(
        held.encumbrances,
        vec![disputed(&claimant(), "hull/0000003c/inspection/3")]
    );
    check_eq!(
        held.title_owner,
        PrincipalId::Object(HOLDER),
        "seizure keeps the title"
    );
    let subject = receipt::subject_of(&tx)?;
    check_eq!((subject.operator, subject.at_ic), (OPERATOR, 2_000));
    check_eq!(subject.title, held);
    audit.observe(&tx);
    // Forfeit: the release of exactly that claim, then the title to the Registry org.
    let shape = Shape::Forfeit {
        reason: inspection_key(HULL, 3),
    };
    let r = request(SeizureStep::Forfeit { seizure: commit(5) }, held.revision);
    let forfeit = closed(&snapshot, 6, &r, shape);
    check_eq!(forfeit.plan().asset_ops.len(), 2);
    let tx = post(&mut snapshot, &forfeit, 8)?;
    let after = snapshot.asset(CORE).cloned().ok_or("the title")?;
    check_eq!(after.title_owner, claimant());
    check!(after.encumbrances.is_empty());
    let receipt = receipt::subject_of(&tx)?;
    check!(matches!(
        &receipt.step,
        SeizureStepReceipt::Forfeit { seizure, .. } if *seizure == commit(5)
    ));
    audit.observe(&tx);
    audit.finish_replay()?;
    check_eq!(audit.receipts()?.len(), 2);
    check!(audit.held(CORE).is_none(), "a forfeit ends the hold");
    audit.verify_titles(&snapshot.managed_titles, &BTreeMap::from([(CORE, after)]))?;
    // The index recovery refuses a receipt on a core the index does not know.
    check!(
        audit
            .verify_titles(
                &crate::managed_title::ManagedTitleIndex::Ready(BTreeMap::new()),
                &BTreeMap::new()
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn m8d2_the_audit_refuses_an_unheld_forfeit_a_second_seizure_and_a_foreign_write() -> Result {
    let mut snapshot = snapshot();
    let r = request(SeizureStep::Seize(proof(3)), 4);
    let seize = closed(&snapshot, 5, &r, seize_shape(3));
    let seize_tx = post(&mut snapshot, &seize, 7)?;
    let held = snapshot.asset(CORE).cloned().ok_or("the title")?;

    // A held core's title is the seizure's post-image at boot.
    let mut audit = SeizureReceiptAudit::default();
    audit.observe(&seize_tx);
    audit.finish_replay()?;
    check_eq!(audit.held(CORE).map(|s| s.commit), Some(commit(5)));
    audit.verify_titles(
        &snapshot.managed_titles,
        &BTreeMap::from([(CORE, held.clone())]),
    )?;
    let mut drifted = held.clone();
    drifted.revision += 1;
    check!(
        audit
            .verify_titles(&snapshot.managed_titles, &BTreeMap::from([(CORE, drifted)]))
            .is_err()
    );

    // The same seizure twice is invalid (a fresh commit of a held core, too).
    let mut twice = SeizureReceiptAudit::default();
    twice.observe(&seize_tx);
    twice.observe(&seize_tx);
    check!(twice.finish_replay().is_err());

    // A forfeit with no live seizure is invalid.
    let mut lone = snapshot.clone();
    let forfeit = closed(
        &lone,
        6,
        &request(SeizureStep::Forfeit { seizure: commit(5) }, held.revision),
        Shape::Forfeit {
            reason: inspection_key(HULL, 3),
        },
    );
    let forfeit_tx = post(&mut lone, &forfeit, 8)?;
    let mut orphan = SeizureReceiptAudit::default();
    orphan.observe(&forfeit_tx);
    check!(orphan.finish_replay().is_err());

    // Any other journal entry touching a held core's title is invalid.
    let mut foreign = SeizureReceiptAudit::default();
    foreign.observe(&seize_tx);
    let mut other = forfeit_tx.clone();
    other.idempotency = IdempotencyKey::new("asset.staff", "x");
    other.cause = EconomicCause::Staff { reason: "x".into() };
    foreign.observe(&other);
    check!(foreign.finish_replay().is_err());
    Ok(())
}

#[test]
fn m8d2_the_checkpoint_must_cover_every_receipt_exactly() -> Result {
    let mut snapshot = snapshot();
    let r = request(SeizureStep::Seize(proof(3)), 4);
    let seize = closed(&snapshot, 5, &r, seize_shape(3));
    let tx = post(&mut snapshot, &seize, 7)?;
    let row = crate::checkpoint::IdemRow {
        ns: receipt::NAMESPACE.into(),
        key: commit(5).to_hex().into(),
        plan_hash: tx.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
        tx: tx.id,
        posted_at: tx.posted_at,
        seq: tx.journal_seq,
    };
    let empty = SeizureReceiptAudit::default();
    check!(empty.checkpoint_complete(&[]));
    check!(
        !empty.checkpoint_complete(std::slice::from_ref(&row)),
        "an omitted inventory"
    );
    let mut audit = SeizureReceiptAudit::default();
    audit.observe(&tx);
    audit.finish_replay()?;
    check!(audit.checkpoint_complete(std::slice::from_ref(&row)));
    check!(!audit.checkpoint_complete(&[]), "a receipt with no key row");
    let mut wrong = row;
    wrong.seq += 1;
    check!(!audit.checkpoint_complete(&[wrong]));
    Ok(())
}

#[test]
fn m8d2_no_generic_path_touches_a_managed_or_held_core() -> Result {
    let mut snapshot = snapshot();
    let release = |reason: &str| {
        let mut plan = EconomyPlan::new(
            IdempotencyKey::new("asset.staff", "release"),
            PrincipalId::Object(OPERATOR),
            EconomicCause::Staff { reason: "x".into() },
            Vec::new(),
        );
        plan.asset_ops.push(AssetOp::Release {
            object: CORE,
            encumbrance: disputed(&claimant(), reason),
        });
        plan
    };
    let key = inspection_key(HULL, 3);
    check!(crate::managed_title::require_unmanaged_mutations(&release(&key), &snapshot).is_err());
    // A held core refuses even an op the managed list otherwise lets through.
    let mut title = core_title();
    title.encumbrances = vec![disputed(&claimant(), &key)];
    snapshot.put_asset(title);
    let mut assign = release(&key);
    assign.asset_ops = vec![AssetOp::Unassign { object: CORE }];
    check!(crate::managed_title::require_unmanaged_mutations(&assign, &snapshot).is_err());
    // S6: a seized core cannot be sold or transferred on any generic path.
    let mut transfer = release(&key);
    transfer.asset_ops = vec![AssetOp::TransferTitle {
        object: CORE,
        to: PrincipalId::Object(OPERATOR),
        to_scope: None,
    }];
    check!(crate::managed_title::require_unmanaged_mutations(&transfer, &snapshot).is_err());
    Ok(())
}

/// **Doc 17h cut M8d2, "impounded in place"**: a seized core reads impounded while its claim
/// stands and after its forfeit to the Registry org; before the seizure, and for a title the
/// managed index does not hold, it does not.
#[test]
fn m8d2_a_seized_core_is_impounded_until_and_after_its_forfeit() -> Result {
    let mut store = EconomyStore {
        snapshot: snapshot(),
        ..EconomyStore::default()
    };
    check!(!store.core_impounded(CORE), "not yet seized");
    check!(!store.core_impounded(HULL), "no title at all");
    let r = request(SeizureStep::Seize(proof(3)), 4);
    let seize = closed(&store.snapshot, 5, &r, seize_shape(3));
    let seize_tx = post(&mut store.snapshot, &seize, 7)?;
    store.fold.seizure_receipts.observe(&seize_tx);
    check!(store.core_impounded(CORE), "held by the seizure");
    let revision = store.snapshot.asset(CORE).ok_or("the title")?.revision;
    let forfeit = closed(
        &store.snapshot,
        6,
        &request(SeizureStep::Forfeit { seizure: commit(5) }, revision),
        Shape::Forfeit {
            reason: inspection_key(HULL, 3),
        },
    );
    let forfeit_tx = post(&mut store.snapshot, &forfeit, 8)?;
    store.fold.seizure_receipts.observe(&forfeit_tx);
    check!(store.fold.seizure_receipts.held(CORE).is_none());
    check!(
        store.core_impounded(CORE),
        "the Registry's after the forfeit"
    );
    // An unmanaged title with the same claim is not impounded (the index decides).
    store.snapshot.managed_titles = crate::managed_title::ManagedTitleIndex::Ready(BTreeMap::new());
    check!(!store.core_impounded(CORE));
    Ok(())
}
