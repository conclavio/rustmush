//! Pure request/publication checks plus real typed-family registration. These
//! tests do not create an E4 audit view or claim a successful renewal action.
use super::*;
use crate::mind_renewal::{RenewalConfig, RenewalPolicy};
use crate::mind_renewal::{
    capacity,
    families::{self, Families},
    provider::{self, Request},
};
use mush_core::{Engine, GameModule, ModError, Registrar, Registry, World};
use std::sync::{Arc, Mutex};

fn config(row: &MindRenewalOperationV1) -> TestResult<RenewalConfig> {
    let [economy, retention] = row.participants();
    Ok(RenewalConfig::new(
        row.manifest().clone(),
        PurchaseParticipants::new(economy.clone(), retention.clone())?,
        mush_roll::commerce::renewal::writer::RenewalWriter::new(),
        RenewalPolicy {
            registry: GrossRecipientCut {
                permille: 0,
                recipient: None,
            },
            venue: None,
            offer_ic_seconds: 100,
        },
    )?)
}
#[test]
fn renewal_requests_accept_only_one_self_selector_and_no_money_or_authority_fields() -> TestResult {
    check!(matches!(
        provider::decode(Transition::Draft, &json!({"licence":7}))?,
        Request::Draft(_)
    ));
    check!(matches!(
        provider::decode(Transition::Draft, &json!({"previous_operation":token(1)}))?,
        Request::Retry(_)
    ));
    for v in [
        json!({}),
        json!({"licence":0}),
        json!({"licence":7,"previous_operation":token(1)}),
        json!({"licence":7,"payer":10}),
        json!({"licence":7,"price":1}),
        json!({"licence":7,"actor":1}),
        json!({"previous_operation":token(1),"receipt":{}}),
        json!({"licence":"7"}),
        json!({"licence":u64::MAX}),
        json!({"licence":-1}),
        json!({"licence":null}),
        json!({"licence":7,"previous_operation":null}),
        json!({"licence":null,"previous_operation":token(1)}),
    ] {
        check!(
            provider::decode(Transition::Draft, &v).is_err(),
            "invalid Draft selector accepted"
        );
    }
    for t in [Transition::Fund, Transition::Grant, Transition::Refund] {
        check!(provider::decode(t, &json!({"operation":token(1)})).is_ok());
        check!(provider::decode(t, &json!({"operation":token(1),"contract":9})).is_err());
        check!(provider::decode(t, &json!({"operation":token(1),"outcome":"granted"})).is_err());
    }
    check!(
        provider::decode(
            Transition::Close,
            &json!({"operation":token(1),"reason":"declined"})
        )
        .is_ok()
    );
    check!(
        provider::decode(
            Transition::Close,
            &json!({"operation":token(1),"reason":"expired"})
        )
        .is_ok()
    );
    for v in [
        json!({"operation":token(1)}),
        json!({"operation":token(1),"reason":"refunded"}),
        json!({"operation":token(1),"reason":"expired","at":0}),
    ] {
        check!(provider::decode(Transition::Close, &v).is_err());
    }
    Ok(())
}
#[test]
fn renewal_projection_has_only_bounded_selectors_and_no_implicit_acceptance() -> TestResult {
    use mush_core::ProjectedParameterKind;
    for t in Transition::ALL {
        let parameters = crate::mind_renewal::actions::parameters(t)?;
        mush_core::ProjectedParameterSchema::new(parameters.clone())?;
        check!(parameters.iter().all(|p| matches!(
            p.id.as_str(),
            "licence" | "previous_operation" | "operation" | "reason"
        )));
        if t == Transition::Draft {
            check_eq!(parameters.len(), 2);
            check!(parameters.iter().all(|p| !p.required));
        } else {
            check!(parameters.iter().all(|p| p.required));
        }
        for p in parameters {
            match p.kind {
                ProjectedParameterKind::Text { maximum_bytes } => check_eq!(maximum_bytes, 32),
                ProjectedParameterKind::Integer { minimum, maximum } => {
                    check_eq!((minimum, maximum), (1, i64::MAX))
                }
                ProjectedParameterKind::Choice { choices } => {
                    check_eq!(choices, vec!["declined", "expired"])
                }
                _ => return Err("unexpected renewal input kind".into()),
            }
        }
    }
    Ok(())
}
#[test]
fn renewal_host_policy_bounds_fees_and_deadline_without_repricing_the_template() -> TestResult {
    let row = fixture()?;
    let c = config(&row)?;
    let refs = c.actions()?;
    for (t, r) in Transition::ALL.into_iter().zip(refs.iter()) {
        check_eq!(*r, row.expected_action(t)?);
    }
    let mut later = c.clone();
    later.policy.offer_ic_seconds = 259200;
    later.policy.registry = GrossRecipientCut {
        permille: 1000,
        recipient: Some(Recipient {
            account: AccountId(14),
            owner: mush_org::PrincipalId::Object(ObjId(6)),
        }),
    };
    later.policy.validate()?;
    check_eq!(
        later.actions()?,
        refs,
        "fee policy must not reinterpret retained action identity"
    );
    check_ne!(later.policy_ref()?, c.policy_ref()?);
    for seconds in [0, -1, 259201] {
        let mut p = c.policy.clone();
        p.offer_ic_seconds = seconds;
        check!(p.validate().is_err());
    }
    let mut p = c.policy.clone();
    p.registry.permille = 1;
    check!(p.validate().is_err());
    let mut p = later.policy.clone();
    p.venue = Some(GrossRecipientCut {
        permille: 1,
        recipient: p.registry.recipient.clone(),
    });
    check!(p.validate().is_err());
    check_eq!(row.offer().terms.price, Amount::from_minor(500));
    Ok(())
}
#[inline(never)]
fn publication_cases() -> TestResult<Box<(MindRenewalOperationV1, MindRenewalOperationV1)>> {
    let row = fixture()?;
    let c = config(&row)?;
    let roll = row.roll_projection()?;
    let captured = MindRenewalOperationV1::captured(&c, row.offer().clone(), &roll)?;
    check_eq!(captured, row);
    let fund = funded(&row)?;
    let next = row.progressed(&fund.roll_projection()?)?;
    check_eq!(next, fund);
    let mut wrong = serde_json::to_value(fund.roll_projection()?)?;
    put(&mut wrong, &["contract"], json!(999))?;
    put(&mut wrong, &["state", "funding", "contract"], json!(999))?;
    let wrong: RenewalOperationV1 = serde_json::from_value(wrong)?;
    check!(row.progressed(&wrong).is_err());
    let mut offer = row.offer().clone();
    offer.terms.expires_at.at += 1;
    check!(MindRenewalOperationV1::captured(&c, offer, &roll).is_err());
    check!(row.receipt().is_err());
    Ok(Box::new((row, fund)))
}

#[test]
fn renewal_publication_keeps_exact_offer_and_rejects_crossed_closed_writer_data() -> TestResult {
    let (row, fund) = *publication_cases()?;
    let ends: [fn(
        &MindRenewalOperationV1,
        &MindRenewalOperationV1,
    ) -> TestResult<MindRenewalOperationV1>; 4] = [
        |_, fund| terminal(fund, true),
        |_, fund| terminal(fund, false),
        |row, _| closed(row, false),
        |row, _| closed(row, true),
    ];
    for make in ends {
        let end = make(&row, &fund)?;
        let receipt = end.receipt()?;
        receipt.matches_operation(&end)?;
        check!(receipt.matches_operation(&fund).is_err());
    }
    Ok(())
}

struct FamilyModule(Arc<Mutex<Option<Families>>>);
impl GameModule for FamilyModule {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let f = Families {
            operations: families::family(reg)?,
            receipts: families::family(reg)?,
        };
        *self
            .0
            .lock()
            .map_err(|_| ModError::msg("test family lock"))? = Some(f);
        Ok(())
    }
}
#[inline(never)]
fn composed(modules: Vec<Box<dyn GameModule>>) -> Result<Box<Engine>, ModError> {
    Engine::with_modules(World::new_minimal(), Registry::new(), modules).map(Box::new)
}
fn registered() -> TestResult<(Box<Engine>, Families)> {
    let slot = Arc::new(Mutex::new(None));
    let engine = composed(vec![Box::new(FamilyModule(slot.clone()))])?;
    let family = slot
        .lock()
        .map_err(|_| "test family lock")?
        .take()
        .ok_or("families")?;
    Ok((engine, family))
}
fn complete<R: serde::Serialize + 'static>(
    family: &mush_core::StateFamilyHandle<CommitToken, R>,
    operation: CommitToken,
    row: &R,
) -> TestResult<usize> {
    let d = family.descriptor();
    let value = json!({"family":{"owner":family.owner(),"id":d.id()},"key":operation.to_hex(),
        "schema":{"id":d.value_schema(),"version":d.schema_version()},"value":row});
    Ok(mush_core::canonical_json_bytes_with_limit(&value, d.max_record_bytes())?.len())
}
#[test]
fn renewal_admission_reserves_complete_registered_operation_and_receipt_envelopes() -> TestResult {
    let (engine, f) = registered()?;
    let row = fixture()?;
    check_eq!(f.operations.owner().as_str(), "market");
    check_eq!(f.operations.descriptor().max_record_bytes(), 32768);
    check_eq!(f.receipts.descriptor().max_record_bytes(), 8192);
    capacity::check(&f, &row)?;
    let fund = funded(&row)?;
    let states: [fn(
        &MindRenewalOperationV1,
        &MindRenewalOperationV1,
    ) -> TestResult<MindRenewalOperationV1>; 6] = [
        |row, _| Ok(row.clone()),
        |_, fund| Ok(fund.clone()),
        |_, fund| terminal(fund, true),
        |_, fund| terminal(fund, false),
        |row, _| closed(row, false),
        |row, _| closed(row, true),
    ];
    for make in states {
        let state = make(&row, &fund)?;
        check!(complete(&f.operations, state.operation(), &state)? <= 32768);
        if !state.state().pending() {
            check!(complete(&f.receipts, state.operation(), &state.receipt()?)? <= 8192);
        }
    }
    check!(engine.action_state().is_empty());
    check!(!engine.action_state().has_prepared());
    check!(!engine.action_state().reservation_lost());
    Ok(())
}
#[test]
fn renewal_config_does_not_allow_missing_existing_purchase_composition() -> TestResult {
    composed(vec![Box::new(crate::MarketModule::new())])?;
    let c = config(&fixture()?)?;
    let module = crate::MarketModule::new().with_mind_renewals(c);
    let refusal = composed(vec![Box::new(module)])
        .err()
        .ok_or("missing renewal prerequisites were accepted")?;
    check!(
        refusal
            .to_string()
            .contains("renewals require entitlement composition")
    );
    Ok(())
}

#[test]
fn renewal_fresh_preparation_starts_only_at_exact_predecessor_end() -> TestResult {
    let end = WorldIcSeconds::from_stamp(&stamp(100))?;
    check!(provider::require_due(&stamp(99), end).is_err());
    provider::require_due(&stamp(100), end)?;
    provider::require_due(&stamp(101), end)?;
    check!(
        provider::require_due(
            &ClockStamp {
                clock: ClockRef::WorldIc { generation: 0 },
                at: 100
            },
            end
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn renewal_personal_account_requires_one_actual_matching_snapshot_row() -> TestResult {
    use mush_economy::{Account, AccountStatus, CreditPolicy, EconomySnapshot};
    let instrument = InstrumentId::new("renewal.test").ok_or("instrument")?;
    let actor = ObjId(20);
    let account = |id| Account {
        id: AccountId(id),
        owner: mush_org::PrincipalId::Object(actor),
        instrument: instrument.clone(),
        scope: None,
        name: "renewal fixture".into(),
        status: AccountStatus::Active,
        posted_balance: Amount::ZERO,
        held_amount: Amount::ZERO,
        credit: CreditPolicy::forbid(),
        last_journal_seq: 0,
        revision: 1,
        approval: None,
        last_hold_seq: 0,
    };
    let mut snapshot = EconomySnapshot::new();
    check!(crate::mind_renewal::offer::personal(&snapshot, &instrument, actor).is_err());
    snapshot.put_account(account(1));
    check_eq!(
        crate::mind_renewal::offer::personal(&snapshot, &instrument, actor)?.id,
        AccountId(1)
    );
    snapshot.put_account(account(2));
    check!(
        crate::mind_renewal::offer::personal(&snapshot, &instrument, actor).is_err(),
        "two eligible real rows must not select the first account"
    );
    for status in [
        AccountStatus::Frozen,
        AccountStatus::Closing,
        AccountStatus::Closed,
    ] {
        snapshot.put_account(Account {
            status,
            ..account(2)
        });
        check_eq!(
            crate::mind_renewal::offer::personal(&snapshot, &instrument, actor)?.id,
            AccountId(1)
        );
    }
    snapshot.put_account(Account {
        scope: Some(mush_org::OrgScopeId(3)),
        ..account(2)
    });
    check_eq!(
        crate::mind_renewal::offer::personal(&snapshot, &instrument, actor)?.id,
        AccountId(1)
    );
    snapshot.put_account(Account {
        instrument: InstrumentId::new("other.test").ok_or("instrument")?,
        ..account(2)
    });
    check_eq!(
        crate::mind_renewal::offer::personal(&snapshot, &instrument, actor)?.id,
        AccountId(1)
    );
    snapshot.put_account(Account {
        owner: mush_org::PrincipalId::Object(ObjId(21)),
        ..account(2)
    });
    check_eq!(
        crate::mind_renewal::offer::personal(&snapshot, &instrument, actor)?.id,
        AccountId(1)
    );
    // Reindexing that actual second row back to the buyer restores ambiguity.
    snapshot.put_account(account(2));
    check!(crate::mind_renewal::offer::personal(&snapshot, &instrument, actor).is_err());
    check_eq!(snapshot.accounts().count(), 2);
    check_eq!(
        snapshot
            .accounts_of(&mush_org::PrincipalId::Object(actor))
            .count(),
        2
    );
    Ok(())
}
