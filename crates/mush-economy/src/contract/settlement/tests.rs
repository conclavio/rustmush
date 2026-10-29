#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;

use super::*;
use crate::Amount;
use crate::contract::{
    Acceptance, ConsentSurface, Consideration, ContractRoles, ContractState, ContractTerms, Expiry,
    OutcomeTerms, PayoutShare, Share, TypedCondition, consent_hash, settlement_plan,
};
use crate::engine::{
    AuthEval, FactEval, KeyGate, KeyVerdict, MapOnlyKeys, NoCustody, NoFacts, apply, validate,
};
use crate::ids::{AccountId, ContractId, HoldId, InstrumentId, TransactionId};
use crate::plan::{EconomyError, PlanResult};
use crate::record::{
    Account, AccountStatus, CreditPolicy, EconomicVisibility, ExternalRef, Hold, HoldPurpose,
    HoldStatus, IdempotencyKey, InstrumentDef, InstrumentKind, InstrumentStatus, NegativePolicy,
};
use mush_core::ObjId;
use mush_org::PrincipalId;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const NOW: i64 = 10_000;

struct NoAuthority;
impl AuthEval for NoAuthority {
    fn authorize(&self, _: &PrincipalId, _: &Account, _: Amount) -> bool {
        false
    }
}

fn who(id: u32) -> PrincipalId {
    PrincipalId::Object(ObjId(id))
}

fn env<'a>(
    facts: &'a dyn FactEval,
    auth: &'a dyn AuthEval,
    keys: &'a dyn KeyGate,
) -> ValidateEnv<'a> {
    ValidateEnv {
        auth,
        custody: &NoCustody,
        facts,
        now: NOW,
        ic_now: NOW * 10,
        next_hold_id: 2,
        spent: BTreeMap::new(),
        keys,
    }
}

fn reoffer(contract: &mut EconomicContract) {
    contract.consent_hash = consent_hash(&contract.issuer, &contract.consent);
    contract.acceptances = contract
        .consent
        .parties
        .iter()
        .map(|party| Acceptance {
            party: party.clone(),
            at: NOW,
            consent_hash: contract.consent_hash,
        })
        .collect();
}

fn fixture() -> Result<(EconomySnapshot, EconomicContract), Box<dyn std::error::Error>> {
    let instrument = InstrumentId::new("settlement-test").ok_or("invalid fixture instrument")?;
    let mut snapshot = EconomySnapshot::new();
    snapshot.put_instrument(InstrumentDef {
        id: instrument.clone(),
        ruleset: "test".into(),
        code: "cr".into(),
        kind: InstrumentKind::Currency,
        scale: 0,
        transferable: true,
        negative_policy: NegativePolicy::SystemOnly,
        status: InstrumentStatus::Active,
        display: None,
        revision: 1,
    });
    for id in [1_u32, 2, 3] {
        snapshot.put_account(Account {
            id: AccountId(u64::from(id)),
            owner: who(id),
            instrument: instrument.clone(),
            scope: None,
            name: format!("account-{id}").into(),
            status: AccountStatus::Active,
            posted_balance: Amount::from_minor(if id == 1 { 1_000 } else { 0 }),
            held_amount: Amount::from_minor(if id == 1 { 100 } else { 0 }),
            credit: CreditPolicy::forbid(),
            last_journal_seq: 0,
            last_hold_seq: 0,
            revision: 1,
            approval: None,
        });
    }
    snapshot.put_hold(Hold {
        id: HoldId(1),
        account: AccountId(1),
        amount: Amount::from_minor(100),
        beneficiary: None,
        purpose: HoldPurpose::Escrow {
            contract: ContractId(1),
        },
        placed_by: who(1),
        holder: IdempotencyKey::new("contract", "1/accept/1"),
        expires: None,
        status: HoldStatus::Held,
        last_journal_seq: 0,
        revision: 1,
    });
    let consent = ConsentSurface {
        fact_consent: None,
        managed: None,
        parties: vec![who(1), who(2), who(3)],
        terms: ContractTerms {
            consideration: vec![Consideration {
                from: who(1),
                account: AccountId(1),
                amount: Amount::from_minor(100),
            }],
            deliverables: Vec::new(),
            split: vec![PayoutShare {
                to: AccountId(2),
                share: Share::Permille(1_000),
            }],
            remainder_to: AccountId(2),
            on_expiry: Expiry::Refund,
            outcomes: Vec::new(),
        },
        conditions: Vec::new(),
        deadline: None,
        visibility: EconomicVisibility::Parties,
        scope: None,
        adjudicator: None,
        evidence: Vec::new(),
    };
    let mut contract = EconomicContract {
        id: ContractId(1),
        issuer: who(1),
        name: "prepared-settlement".into(),
        consent,
        consent_hash: [0; 32],
        acceptances: Vec::new(),
        satisfied: Vec::new(),
        escrow: vec![HoldId(1)],
        state: ContractState::ReadyToSettle,
        settle_attempts: 0,
        created_at: NOW,
        amendments: 0,
        generation_started_at: NOW,
        disputed_at: None,
        canon_published: None,
        roster_locked: true,
        deposits: Vec::new(),
        agreements: Vec::new(),
        pending: None,
        roles: ContractRoles::default(),
        revision: 1,
        real_deadline_extension_secs: 0,
        paused_real_remaining_secs: None,
    };
    reoffer(&mut contract);
    snapshot.put_contract(contract.clone());
    Ok((snapshot, contract))
}

fn validate_preparation(
    prepared: &SettlementPreparation<'_>,
    snapshot: &EconomySnapshot,
    keys: &dyn KeyGate,
) -> PlanResult {
    validate(
        prepared.plan(),
        snapshot,
        &env(&NoFacts, prepared.authorization(), keys),
    )
}

#[test]
fn m7_settlement_preparation_is_inert_and_preserves_legacy_plan_and_receipt() -> TestResult {
    let (snapshot, contract) = fixture()?;
    let before_snapshot = snapshot.clone();
    let before_contract = contract.clone();
    let environment = env(&NoFacts, &NoAuthority, &MapOnlyKeys);
    let legacy_plan = settlement_plan(&contract, &snapshot, &environment)?;
    let prepared = prepare_settlement(&contract, &snapshot, &environment)?;
    // EconomyPlan deliberately is not a wire record. Its derived equality
    // compares every semantic field, including all staleness expectations.
    check_eq!(prepared.plan(), &legacy_plan);
    let PlanResult::Post(prepared_post) = validate_preparation(&prepared, &snapshot, &MapOnlyKeys)
    else {
        return Err("prepared settlement must post".into());
    };
    let legacy_auth = ContractAuth::new(&contract, &snapshot);
    let PlanResult::Post(legacy_post) = validate(
        &legacy_plan,
        &snapshot,
        &env(&NoFacts, &legacy_auth, &MapOnlyKeys),
    ) else {
        return Err("legacy settlement must post".into());
    };
    check_eq!(prepared_post, legacy_post);
    check_eq!(
        apply(&snapshot, &prepared_post, TransactionId(9), 7, NOW),
        apply(&snapshot, &legacy_post, TransactionId(9), 7, NOW),
        "actual post-images and immutable journal receipt must agree"
    );
    check!(matches!(
        validate(prepared.plan(), &snapshot, &environment),
        PlanResult::Refused(_)
    ));
    drop(prepared);
    check_eq!(snapshot, before_snapshot);
    check_eq!(contract, before_contract);
    Ok(())
}

#[test]
fn m7_settlement_preparation_rejects_changed_consent_and_old_acceptances() -> TestResult {
    let (snapshot, mut contract) = fixture()?;
    let environment = env(&NoFacts, &NoAuthority, &MapOnlyKeys);
    check!(prepare_settlement(&contract, &snapshot, &environment).is_ok());
    contract.consent.visibility = EconomicVisibility::Public;
    check!(matches!(
        prepare_settlement(&contract, &snapshot, &environment),
        Err(ContractError::ConsentMismatch)
    ));
    contract.consent_hash = consent_hash(&contract.issuer, &contract.consent);
    check!(matches!(
        prepare_settlement(&contract, &snapshot, &environment),
        Err(ContractError::NotAccepted)
    ));
    reoffer(&mut contract);
    check!(prepare_settlement(&contract, &snapshot, &environment).is_ok());
    check_eq!(
        snapshot.hold(HoldId(1)).ok_or("missing hold")?.status,
        HoldStatus::Held
    );
    Ok(())
}

#[test]
fn m7_settlement_preparation_refuses_foreign_and_spent_escrow() -> TestResult {
    let (snapshot, contract) = fixture()?;
    let original = snapshot.hold(HoldId(1)).ok_or("missing hold")?.clone();
    let environment = env(&NoFacts, &NoAuthority, &MapOnlyKeys);
    let mut foreign = snapshot.clone();
    foreign.put_hold(Hold {
        purpose: HoldPurpose::Escrow {
            contract: ContractId(9),
        },
        ..original.clone()
    });
    check!(matches!(
        prepare_settlement(&contract, &foreign, &environment),
        Err(ContractError::EscrowIncomplete)
    ));
    let mut spent = snapshot.clone();
    spent.put_hold(Hold {
        status: HoldStatus::Released,
        ..original
    });
    check!(matches!(
        prepare_settlement(&contract, &spent, &environment),
        Err(ContractError::HoldNotHeld(HoldId(1)))
    ));
    check_eq!(
        snapshot
            .account(AccountId(1))
            .ok_or("missing payer")?
            .posted_balance,
        Amount::from_minor(1_000)
    );
    Ok(())
}

#[test]
fn m7_settlement_preparation_keeps_the_actual_engine_hold_revision_fence() -> TestResult {
    let (mut snapshot, contract) = fixture()?;
    let environment = env(&NoFacts, &NoAuthority, &MapOnlyKeys);
    let prepared = prepare_settlement(&contract, &snapshot, &environment)?;
    let mut changed = snapshot.hold(HoldId(1)).ok_or("missing hold")?.clone();
    changed.revision += 1;
    snapshot.put_hold(changed);
    check!(matches!(
        validate_preparation(&prepared, &snapshot, &MapOnlyKeys),
        PlanResult::Refused(EconomyError::StaleHold {
            hold: HoldId(1),
            ..
        })
    ));
    let fresh = prepare_settlement(&contract, &snapshot, &environment)?;
    check!(validate_preparation(&fresh, &snapshot, &MapOnlyKeys).is_post());
    check_eq!(
        snapshot.hold(HoldId(1)).ok_or("missing hold")?.status,
        HoldStatus::Held
    );
    Ok(())
}

struct OutcomeFacts {
    success: Option<bool>,
    failure: Option<bool>,
    reads: Cell<usize>,
}
impl OutcomeFacts {
    fn new(success: Option<bool>, failure: Option<bool>) -> Self {
        Self {
            success,
            failure,
            reads: Cell::new(0),
        }
    }
}
impl FactEval for OutcomeFacts {
    fn resolve(&self, fact: &ExternalRef) -> Option<bool> {
        self.reads.set(self.reads.get() + 1);
        match fact.id.as_str() {
            "success" => self.success,
            "failure" => self.failure,
            _ => None,
        }
    }
}

fn outcomes(contract: &mut EconomicContract) {
    contract.consent.conditions = ["success", "failure"]
        .into_iter()
        .map(|id| TypedCondition::Fact {
            fact: ExternalRef {
                provider: "test".into(),
                kind: "result".into(),
                id: id.into(),
                revision: Some(1),
            },
        })
        .collect();
    contract.satisfied = vec![None, None];
    contract.consent.terms.outcomes = [("success", 0, AccountId(2)), ("failure", 1, AccountId(3))]
        .into_iter()
        .map(|(name, condition, to)| OutcomeTerms {
            name: name.into(),
            when: vec![condition],
            payouts: vec![PayoutShare {
                to,
                share: Share::Permille(1_000),
            }],
            expense_treatment: Default::default(),
            asset_consequences: Vec::new(),
        })
        .collect();
    reoffer(contract);
}

#[test]
fn m7_settlement_preparation_rechecks_non_outcome_conditions() -> TestResult {
    let (snapshot, mut contract) = fixture()?;
    contract.consent.conditions = vec![TypedCondition::Fact {
        fact: ExternalRef {
            provider: "test".into(),
            kind: "gating".into(),
            id: "success".into(),
            revision: Some(1),
        },
    }];
    contract.satisfied = vec![None];
    reoffer(&mut contract);
    let present = OutcomeFacts::new(Some(true), None);
    check!(
        prepare_settlement(
            &contract,
            &snapshot,
            &env(&present, &NoAuthority, &MapOnlyKeys)
        )
        .is_ok()
    );
    for answer in [Some(false), None] {
        let absent = OutcomeFacts::new(answer, None);
        check!(matches!(
            prepare_settlement(
                &contract,
                &snapshot,
                &env(&absent, &NoAuthority, &MapOnlyKeys)
            ),
            Err(ContractError::ConditionFailed(0))
        ));
    }
    check_eq!(contract.state, ContractState::ReadyToSettle);
    Ok(())
}

#[test]
fn m7_settlement_preparation_binds_plan_and_auth_to_one_current_outcome() -> TestResult {
    let (snapshot, mut contract) = fixture()?;
    outcomes(&mut contract);
    let success = OutcomeFacts::new(Some(true), Some(false));
    let first = prepare_settlement(
        &contract,
        &snapshot,
        &env(&success, &NoAuthority, &MapOnlyKeys),
    )?;
    check_eq!(success.reads.get(), 2, "each outcome fact is resolved once");
    check!(validate_preparation(&first, &snapshot, &MapOnlyKeys).is_post());
    let failure = OutcomeFacts::new(Some(false), Some(true));
    let second = prepare_settlement(
        &contract,
        &snapshot,
        &env(&failure, &NoAuthority, &MapOnlyKeys),
    )?;
    check_eq!(failure.reads.get(), 2);
    check!(validate_preparation(&second, &snapshot, &MapOnlyKeys).is_post());
    check_ne!(first.plan().requested_lines, second.plan().requested_lines);
    check!(matches!(
        validate(
            first.plan(),
            &snapshot,
            &env(&NoFacts, second.authorization(), &MapOnlyKeys)
        ),
        PlanResult::Refused(_)
    ));
    check!(matches!(
        prepare_settlement(
            &contract,
            &snapshot,
            &env(
                &OutcomeFacts::new(Some(true), Some(true)),
                &NoAuthority,
                &MapOnlyKeys
            )
        ),
        Err(ContractError::AmbiguousOutcome)
    ));
    check!(matches!(
        prepare_settlement(
            &contract,
            &snapshot,
            &env(&OutcomeFacts::new(None, None), &NoAuthority, &MapOnlyKeys)
        ),
        Err(ContractError::NoOutcome)
    ));
    Ok(())
}

#[test]
fn m7_settlement_preparation_preserves_unknown_key_and_exact_receipt_replay() -> TestResult {
    struct UnknownKey;
    impl KeyGate for UnknownKey {
        fn verdict<'s>(&'s self, _: &'s EconomySnapshot, _: &IdempotencyKey) -> KeyVerdict<'s> {
            KeyVerdict::Unknown
        }
    }
    let (snapshot, contract) = fixture()?;
    let prepared = prepare_settlement(
        &contract,
        &snapshot,
        &env(&NoFacts, &NoAuthority, &MapOnlyKeys),
    )?;
    check_eq!(
        validate_preparation(&prepared, &snapshot, &UnknownKey),
        PlanResult::Refused(EconomyError::RetryShortly)
    );
    let PlanResult::Post(engine_prepared) =
        validate_preparation(&prepared, &snapshot, &MapOnlyKeys)
    else {
        return Err("exact contract authorization must validate".into());
    };
    let (after, transaction) = apply(&snapshot, &engine_prepared, TransactionId(7), 12, NOW);
    check_eq!(transaction.id, TransactionId(7));
    check_eq!(
        validate_preparation(&prepared, &after, &MapOnlyKeys),
        PlanResult::AlreadyPosted(TransactionId(7))
    );
    check_eq!(
        after
            .account(AccountId(1))
            .ok_or("missing payer")?
            .posted_balance,
        Amount::from_minor(900)
    );
    check_eq!(
        after
            .account(AccountId(2))
            .ok_or("missing payee")?
            .posted_balance,
        Amount::from_minor(100)
    );
    check_eq!(
        contract.state,
        ContractState::ReadyToSettle,
        "only the live post result may publish the terminal contract row"
    );
    Ok(())
}

#[test]
fn m7_settlement_preparation_keeps_live_refusal_precedence() -> TestResult {
    let (snapshot, mut contract) = fixture()?;
    outcomes(&mut contract);
    // The live path historically reports an ambiguous result first, even if a
    // second fault would separately make the surface unaccepted.
    contract.consent.visibility = EconomicVisibility::Public;
    let facts = OutcomeFacts::new(Some(true), Some(true));
    check!(matches!(
        prepare_settlement(
            &contract,
            &snapshot,
            &env(&facts, &NoAuthority, &MapOnlyKeys)
        ),
        Err(ContractError::AmbiguousOutcome)
    ));
    check!(matches!(
        settlement_plan(
            &contract,
            &snapshot,
            &env(&facts, &NoAuthority, &MapOnlyKeys)
        ),
        Err(ContractError::ConsentMismatch)
    ));
    Ok(())
}

fn managed_marker() -> Result<crate::contract::managed::ManagedConsent, Box<dyn std::error::Error>>
{
    use mush_core::{CommitToken, DefinitionDigest};
    let digest =
        DefinitionDigest::for_canonical_value("managed.contract.test", &serde_json::json!({}))?;
    Ok(serde_json::from_value(serde_json::json!({
        "kind": "original_personal",
        "operation": CommitToken::from_bytes([1; 16]),
        "quote": CommitToken::from_bytes([2; 16]),
        "effect": { "key": { "owner": "market", "kind": "action", "id": "purchase_original_mind" },
            "version": 1, "digest": digest },
        "quote_digest": digest,
        "product_digest": digest,
    }))?)
}

#[test]
fn managed_consent_is_exact_and_none_preserves_legacy_wire() -> TestResult {
    let (_, mut contract) = fixture()?;
    let legacy = serde_json::to_value(&contract)?;
    check!(legacy["consent"].get("managed").is_none());
    let legacy_hash = consent_hash(&contract.issuer, &contract.consent);
    let marker = managed_marker()?;
    contract.consent.managed = Some(marker.clone());
    let managed_hash = consent_hash(&contract.issuer, &contract.consent);
    check_ne!(managed_hash, legacy_hash);
    let encoded = serde_json::to_vec(&contract)?;
    check_eq!(
        serde_json::from_slice::<EconomicContract>(&encoded)?,
        contract
    );
    for field in ["operation", "quote", "quote_digest", "product_digest"] {
        let mut wire = serde_json::to_value(&marker)?;
        let replacement = if field.ends_with("digest") {
            serde_json::json!(mush_core::DefinitionDigest::for_canonical_value(
                "managed.other",
                &serde_json::json!({})
            )?)
        } else {
            serde_json::json!(mush_core::CommitToken::from_bytes([3; 16]))
        };
        wire[field] = replacement;
        contract.consent.managed = Some(serde_json::from_value(wire)?);
        check_ne!(
            consent_hash(&contract.issuer, &contract.consent),
            managed_hash
        );
    }
    for field in ["actor", "validated", "authority"] {
        let mut wire = serde_json::to_value(&marker)?;
        wire[field] = serde_json::json!(true);
        check!(serde_json::from_value::<crate::contract::managed::ManagedConsent>(wire).is_err());
    }
    let encoded_marker = serde_json::to_string(&marker)?;
    let duplicate = encoded_marker.replacen("{", "{\"kind\":\"original_personal\",", 1);
    check!(serde_json::from_str::<crate::contract::managed::ManagedConsent>(&duplicate).is_err());
    let mut wrong = serde_json::to_value(&marker)?;
    wrong["effect"]["key"]["owner"] = serde_json::json!("roll");
    check!(serde_json::from_value::<crate::contract::managed::ManagedConsent>(wrong).is_err());
    contract.consent.managed = None;
    check_eq!(serde_json::to_value(&contract)?, legacy);
    check_eq!(
        consent_hash(&contract.issuer, &contract.consent),
        legacy_hash
    );
    Ok(())
}

#[test]
fn managed_contract_auth_refuses_actual_marker_and_marker_stripped_clone() -> TestResult {
    use crate::contract::Deliverable;
    let (mut snapshot, mut contract) = fixture()?;
    contract
        .consent
        .terms
        .deliverables
        .push(Deliverable::TitleTransfer {
            object: ObjId(90),
            from: who(1),
            to: who(2),
        });
    reoffer(&mut contract);
    let ordinary = contract.clone();
    let hold = snapshot.hold(HoldId(1)).ok_or("missing hold")?;
    let capture = crate::plan::HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::from_minor(100),
    };
    let release = crate::plan::HoldOp::Release { hold: HoldId(1) };
    check!(
        crate::contract::ContractAuth::new(&ordinary, &snapshot).authorize_hold(
            &who(1),
            hold,
            &capture
        )
    );
    check!(
        crate::contract::ContractAuth::new(&ordinary, &snapshot).authorize_title_receipt(
            &who(1),
            &who(2),
            None
        )
    );
    contract.consent.managed = Some(managed_marker()?);
    reoffer(&mut contract);
    snapshot.put_contract(contract.clone());
    let before = snapshot.clone();
    let hold = snapshot.hold(HoldId(1)).ok_or("missing hold")?;
    for supplied in [&contract, &ordinary] {
        let settle = crate::contract::ContractAuth::new(supplied, &snapshot);
        check!(!settle.authorize_hold(&who(1), hold, &capture));
        check!(!settle.authorize_title_receipt(&who(1), &who(2), None));
        check!(
            !crate::contract::ContractAuth::settling(supplied, &snapshot, None).authorize_hold(
                &who(1),
                hold,
                &capture
            )
        );
        check!(
            !crate::contract::ContractAuth::forfeiting(supplied, &snapshot, AccountId(2))
                .authorize_hold(&who(1), hold, &capture)
        );
        check!(
            !crate::contract::ContractAuth::unwinding(supplied, &snapshot).authorize_hold(
                &who(1),
                hold,
                &release
            )
        );
    }
    let mut changed_id = ordinary;
    changed_id.id = ContractId(99);
    check!(
        !crate::contract::ContractAuth::new(&changed_id, &snapshot).authorize_hold(
            &who(1),
            hold,
            &capture
        )
    );
    check_eq!(snapshot, before);
    Ok(())
}

#[test]
fn managed_actual_escrow_refuses_fresh_key_and_known_retry_before_key_gate() -> TestResult {
    use crate::engine::AllowAll;
    struct NeverKeys;
    impl KeyGate for NeverKeys {
        fn verdict(&self, _: &EconomySnapshot, _: &IdempotencyKey) -> KeyVerdict<'_> {
            panic!("managed refusal must precede idempotency lookup")
        }
    }
    let (mut snapshot, mut contract) = fixture()?;
    contract.consent.managed = Some(managed_marker()?);
    reoffer(&mut contract);
    snapshot.put_contract(contract.clone());
    let before = snapshot.clone();
    for op in [
        crate::plan::HoldOp::Release { hold: HoldId(1) },
        crate::plan::HoldOp::Expire { hold: HoldId(1) },
        crate::plan::HoldOp::Capture {
            hold: HoldId(1),
            amount: Amount::from_minor(100),
        },
        crate::plan::HoldOp::Place {
            account: AccountId(1),
            amount: Amount::from_minor(1),
            beneficiary: None,
            purpose: HoldPurpose::Escrow {
                contract: contract.id,
            },
            expires: None,
        },
    ] {
        let mut plan = crate::plan::EconomyPlan::new(
            IdempotencyKey::new("fresh", "other"),
            who(1),
            crate::record::EconomicCause::Command { serial: 1 },
            Vec::new(),
        );
        plan.hold_ops.push(op);
        check!(matches!(
            validate(&plan, &snapshot, &env(&NoFacts, &AllowAll, &NeverKeys)),
            PlanResult::Refused(EconomyError::ManagedContract)
        ));
    }
    check_eq!(snapshot, before);
    Ok(())
}

#[test]
fn managed_actual_asset_and_custody_references_refuse_even_with_unrelated_plan_identity()
-> TestResult {
    use crate::engine::AllowAll;
    use crate::ids::VaultId;
    use crate::plan::{AssetOp, CustodyOp, EconomyPlan};
    use crate::record::{AssetLifecycle, EconomicAsset, EconomicCause, Encumbrance};

    let (mut snapshot, mut contract) = fixture()?;
    contract.consent.managed = Some(managed_marker()?);
    contract
        .consent
        .conditions
        .push(TypedCondition::DepositInto {
            object: ObjId(90),
            vault: VaultId(1),
        });
    reoffer(&mut contract);
    snapshot.put_contract(contract.clone());
    snapshot.put_asset(EconomicAsset {
        object: ObjId(91),
        title_owner: who(1),
        scope: None,
        assignment: None,
        maintenance_account: None,
        catalog: None,
        encumbrances: vec![Encumbrance::Collateral {
            contract: contract.id,
        }],
        acquisition: None,
        lifecycle: AssetLifecycle::Active,
        last_journal_seq: 0,
        revision: 1,
        stolen: None,
        frozen: false,
        frozen_reason: None,
    });
    let before = snapshot.clone();
    let plain = EconomyPlan::new(
        IdempotencyKey::new("unrelated", "identity"),
        who(1),
        EconomicCause::Command { serial: 1 },
        Vec::new(),
    );
    for op in [
        AssetOp::Unassign { object: ObjId(91) },
        AssetOp::Unassign { object: ObjId(90) },
        AssetOp::Encumber {
            object: ObjId(92),
            encumbrance: Encumbrance::Collateral {
                contract: contract.id,
            },
        },
    ] {
        let mut plan = plain.clone();
        plan.asset_ops.push(op);
        check!(matches!(
            validate(&plan, &snapshot, &env(&NoFacts, &AllowAll, &MapOnlyKeys)),
            PlanResult::Refused(EconomyError::ManagedContract)
        ));
    }
    for op in [
        CustodyOp::Deposit {
            object: ObjId(90),
            vault: VaultId(99),
        },
        CustodyOp::Withdraw {
            object: ObjId(90),
            vault: VaultId(99),
            to: ObjId(1),
        },
    ] {
        let mut plan = plain.clone();
        plan.custody_ops.push(op);
        check!(matches!(
            validate(&plan, &snapshot, &env(&NoFacts, &AllowAll, &MapOnlyKeys)),
            PlanResult::Refused(EconomyError::ManagedContract)
        ));
    }
    check_eq!(snapshot, before);
    Ok(())
}

struct ManagedProbe;
impl mush_core::GameModule for ManagedProbe {
    fn name(&self) -> &'static str {
        "managedprobe"
    }
    fn register(&mut self, reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        reg.command(
            mush_core::CommandSpec {
                name: "@managed-probe".into(),
                switches: [
                    "accept",
                    "cloneaccept",
                    "clonewrite",
                    "clonecompanion",
                    "settle",
                    "expire",
                    "canon",
                ]
                .into_iter()
                .map(smol_str::SmolStr::new)
                .collect(),
                ..Default::default()
            },
            std::sync::Arc::new(|ctx: &mut mush_core::Ctx<'_>| {
                let mut contract = ctx
                    .w
                    .module_state::<crate::EconomyStore>()
                    .and_then(|store| store.snapshot().contract(ContractId(1)))
                    .cloned()
                    .ok_or_else(|| mush_core::CmdError::msg("missing fixture"))?;
                if ctx.switches.has("cloneaccept")
                    || ctx.switches.has("clonewrite")
                    || ctx.switches.has("clonecompanion")
                {
                    contract.consent.managed = None;
                    contract.consent_hash = consent_hash(&contract.issuer, &contract.consent);
                }
                if ctx.switches.has("accept") || ctx.switches.has("cloneaccept") {
                    crate::contract_live::accept_core_as(ctx, contract, who(1), who(1))
                        .map_err(|error| mush_core::CmdError::msg(error.to_string()))?;
                } else if ctx.switches.has("clonewrite") {
                    crate::api::put_contract(ctx, contract)
                        .map_err(|error| mush_core::CmdError::msg(error.to_string()))?;
                } else if ctx.switches.has("clonecompanion") {
                    let plan = crate::plan::EconomyPlan::new(
                        IdempotencyKey::new("probe", "companion"),
                        who(1),
                        crate::record::EconomicCause::Command { serial: 1 },
                        Vec::new(),
                    );
                    crate::api::post_with_companions(
                        ctx,
                        &plan,
                        crate::api::PostAuth::World,
                        vec![contract],
                    )
                    .map_err(|error| mush_core::CmdError::msg(error.to_string()))?;
                } else if ctx.switches.has("settle") {
                    crate::contract_live::promote_and_settle(ctx, contract.id, true);
                } else if ctx.switches.has("expire") {
                    crate::contract_live::expire(ctx, contract.id);
                } else if ctx.switches.has("canon") {
                    crate::api::mark_canon_published(ctx, contract.id, "1");
                }
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn managed_world(
    snapshot: EconomySnapshot,
) -> Result<mush_testkit::TestWorld, Box<dyn std::error::Error>> {
    let capabilities = crate::CAPABILITIES
        .iter()
        .copied()
        .map(mush_org::CapabilityId::new)
        .collect::<Option<std::collections::BTreeSet<_>>>()
        .ok_or("invalid capability fixture")?;
    let mut world = mush_testkit::TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new().with_capabilities(capabilities)),
        Box::new(crate::EconomyModule::with_custody(Vec::new())),
        Box::new(ManagedProbe),
    ]);
    world
        .world_mut()
        .module_state_mut::<crate::EconomyStore>()
        .ok_or("missing real registered EconomyStore")?
        .snapshot = snapshot;
    let _ = world.take_module_ops();
    Ok(world)
}

#[test]
fn managed_actual_command_and_programmatic_paths_leave_free_acceptance_unchanged() -> TestResult {
    let (mut snapshot, mut contract) = fixture()?;
    contract.state = ContractState::Offered;
    contract.consent.terms.consideration.clear();
    contract.escrow.clear();
    contract.acceptances.clear();
    contract.consent.managed = Some(managed_marker()?);
    contract.consent_hash = consent_hash(&contract.issuer, &contract.consent);
    snapshot.put_contract(contract);
    let before = snapshot.clone();
    let mut world = managed_world(snapshot)?;
    for line in [
        "@contract/accept 1",
        "@contract/cancel 1",
        "@contract/amend 1",
        "@contract/roster 1=open",
        "@contract/offer 1",
        "@managed-probe/accept",
        "@managed-probe/cloneaccept",
        "@managed-probe/clonewrite",
        "@managed-probe/clonecompanion",
    ] {
        let events = world.run(mush_core::GOD, line);
        let text = events
            .iter()
            .map(|(_, event)| mush_testkit::text_of(event))
            .collect::<Vec<_>>()
            .join("\n");
        check!(
            text.contains("complete entitlement operation"),
            "{line}: {text}"
        );
        let store = world
            .world()
            .module_state::<crate::EconomyStore>()
            .ok_or("missing store")?;
        check_eq!(store.snapshot(), &before, "{line}");
        check_eq!(store.transactions().count(), 0, "{line}");
        check!(
            world.take_module_ops().is_empty(),
            "{line}: queued writes on refusal"
        );
    }
    let events = world.run(mush_core::GOD, "@contract/show 1");
    let text = events
        .iter()
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect::<Vec<_>>()
        .join("\n");
    check!(
        !text.contains("complete entitlement operation"),
        "read-only visibility remains: {text}"
    );
    check!(
        text.contains("prepared-settlement"),
        "read-only record still visible: {text}"
    );
    Ok(())
}

#[test]
fn managed_automatic_settlement_expiry_and_canon_do_not_repair_half_an_operation() -> TestResult {
    for state in [
        ContractState::Active,
        ContractState::ReadyToSettle,
        ContractState::Disputed,
        ContractState::Settled {
            transaction: TransactionId(99),
        },
    ] {
        let (mut snapshot, mut contract) = fixture()?;
        contract.state = state;
        contract.consent.managed = Some(managed_marker()?);
        reoffer(&mut contract);
        snapshot.put_contract(contract);
        let mut due = snapshot.hold(HoldId(1)).ok_or("missing escrow")?.clone();
        due.expires = Some(crate::clock::EconomicDeadline::real(NOW));
        snapshot.put_hold(due);
        let before = snapshot.clone();
        let mut world = managed_world(snapshot)?;
        for line in [
            "@managed-probe/settle",
            "@managed-probe/expire",
            "@managed-probe/canon",
        ] {
            world.run(mush_core::GOD, line);
            let store = world
                .world()
                .module_state::<crate::EconomyStore>()
                .ok_or("missing store")?;
            check_eq!(store.snapshot(), &before, "{line}");
            check_eq!(store.transactions().count(), 0, "{line}");
            check!(
                world.take_module_ops().is_empty(),
                "{line}: queued half-operation writes"
            );
        }
        world.set_now(NOW + crate::contract::DRAFT_TTL_SECS + 1);
        world.fire_due((NOW + crate::contract::DRAFT_TTL_SECS + 1).saturating_mul(1000));
        check_eq!(
            world
                .world()
                .module_state::<crate::EconomyStore>()
                .ok_or("missing store")?
                .snapshot(),
            &before
        );
        check_eq!(
            world
                .world()
                .module_state::<crate::EconomyStore>()
                .ok_or("missing store")?
                .expiry_refusals(HoldId(1)),
            0,
            "legacy expiry cannot accumulate repair advice for managed escrow"
        );
    }
    Ok(())
}

#[test]
fn managed_boot_requires_full_operation_recovery_before_legacy_healing() -> TestResult {
    for state in [
        ContractState::Draft,
        ContractState::Offered,
        ContractState::Active,
        ContractState::Settled {
            transaction: TransactionId(99),
        },
    ] {
        let (_, mut contract) = fixture()?;
        contract.state = state;
        contract.consent.managed = Some(managed_marker()?);
        reoffer(&mut contract);
        let mut world = managed_world(EconomySnapshot::new())?;
        let boot = std::collections::HashMap::from([(
            smol_str::SmolStr::new("economy"),
            mush_core::ModuleBootData {
                kv: vec![("contract:1".into(), serde_json::to_value(&contract)?)],
                ..Default::default()
            },
        )]);
        let error = world
            .try_modules_loaded(boot)
            .expect_err("managed boot cannot silently use legacy reconciliation");
        check!(
            error.to_string().contains("complete entitlement recovery"),
            "{error}"
        );
        check!(world.take_module_ops().is_empty());
    }
    Ok(())
}
