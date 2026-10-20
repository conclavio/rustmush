//! Maximum complete envelopes built from explicit sizing data. These rows are
//! never supplied to an owner as evidence or used as a purchase/holder grant.
use super::*;
use crate::record::{HoldEffect, JournalLine};
use crate::{Amount, InstrumentId};
use mush_core::{ClockRef, CommitToken, DefinitionRef, ObjId};
use mush_org::SystemPrincipalId;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn maximum_intent() -> Result<Intent> {
    let mut c = super::super::tests::sample()?;
    c.terms.price = Amount::from_minor(i64::MAX);
    c.terms.basis.licence = u64::MAX;
    c.terms.basis.line = u64::MAX;
    c.terms.basis.term.generation = u64::MAX;
    c.terms.basis.holder.generation = 65536;
    c.claims.next_transition = u64::MAX - 1;
    c.claims.open_paid = 32768;
    let mut low = 1;
    let mut high = MAX_SUBJECT_BYTES + 1;
    while low + 1 < high {
        let middle = (low + high) / 2;
        let mut v = c.clone();
        v.terms.instrument = InstrumentId::new("i".repeat(middle)).ok_or("instrument")?;
        if v.validate().is_ok() {
            low = middle
        } else {
            high = middle
        }
    }
    c.terms.instrument = InstrumentId::new("i".repeat(low)).ok_or("instrument")?;
    c.validate()?;
    assert_eq!(
        subject::bounded(&c, MAX_SUBJECT_BYTES)?.len(),
        MAX_SUBJECT_BYTES
    );
    let mut larger = c.clone();
    larger.terms.instrument = InstrumentId::new("i".repeat(low + 1)).ok_or("instrument")?;
    assert!(larger.validate().is_err());
    let mut i = Intent {
        consent: c,
        issuer: SystemPrincipalId::new("issuer").ok_or("issuer")?,
        contract: ContractId(u64::MAX),
        hold: Some(HoldId(u64::MAX)),
    };
    low = 1;
    high = MAX_CORRELATION_BYTES + 1;
    while low + 1 < high {
        let middle = (low + high) / 2;
        let mut v = i.clone();
        v.issuer = SystemPrincipalId::new("s".repeat(middle)).ok_or("issuer")?;
        if v.validate().is_ok() {
            low = middle
        } else {
            high = middle
        }
    }
    i.issuer = SystemPrincipalId::new("s".repeat(low)).ok_or("issuer")?;
    i.validate()?;
    assert_eq!(
        plans::encode(&plans::Proof::Fund {
            format: 1,
            intent: i.clone()
        })?
        .len(),
        MAX_CORRELATION_BYTES
    );
    let mut larger = i.clone();
    larger.issuer = SystemPrincipalId::new("s".repeat(low + 1)).ok_or("issuer")?;
    assert!(larger.validate().is_err());
    Ok(i)
}
fn before(i: Intent) -> Result<PaidResaleHistoryV1> {
    let c = &i.consent;
    let hold = Hold {
        id: i.hold.ok_or("hold")?,
        account: c.payer,
        amount: c.terms.price,
        beneficiary: Some(crate::api::contract_principal()),
        purpose: HoldPurpose::Escrow {
            contract: i.contract,
        },
        placed_by: PrincipalId::Object(c.terms.buyer),
        holder: IdempotencyKey::new(NAMESPACE, format!("{}/fund", c.operation.to_hex())),
        expires: None,
        status: HoldStatus::Held,
        last_journal_seq: u64::MAX - 1,
        revision: 1,
    };
    let row = PaidResaleHistoryV1 {
        acceptance: Box::new(PaidResaleAcceptanceReceiptV1 {
            transaction: TransactionId(u64::MAX - 1),
            journal_seq: u64::MAX - 1,
            plan_hash: crate::hash::plan_hash(&plans::acceptance(&i)?),
            hold: Some(hold),
        }),
        intent: i,
        terminal: None,
    };
    row.validate()?;
    row.require_terminal_room()?;
    Ok(row)
}
fn action(old: &DefinitionRef, id: &str) -> Result<DefinitionRef> {
    let mut v = serde_json::to_value(old)?;
    v["key"]["id"] = serde_json::json!(id);
    v["digest"] = serde_json::to_value(DefinitionDigest::from_bytes([255; 32]))?;
    Ok(serde_json::from_value(v)?)
}
#[test]
fn resale_exact_subject_and_correlation_bounds_leave_complete_terminal_replay_room() -> Result {
    let row = before(maximum_intent()?)?;
    for variant in 0..5 {
        let c = row.consent();
        let mut d = PaidResaleDecisionV1 {
            format: 1,
            operation: CommitToken::from_bytes([250; 16]),
            action: action(&c.effect, SETTLE)?,
            actor: ObjId(u32::MAX),
            at_ic: mush_core::ClockStamp {
                clock: c.accepted_at_ic.clock.clone(),
                at: i64::MAX,
            },
            posted_at_utc: i64::MAX,
            reason: PaidResaleDecisionKindV1::DeadlineReached {
                next_transition: u64::MAX,
                open_paid: 32768,
            },
        };
        match variant {
            0 => (),
            1 => {
                d.reason = PaidResaleDecisionKindV1::LastClaimClosed {
                    next_transition: u64::MAX,
                }
            }
            2 => {
                d.action = action(&c.effect, REFUND)?;
                d.reason = PaidResaleDecisionKindV1::ClockChanged {};
                d.at_ic.clock = ClockRef::WorldIc {
                    generation: u64::MAX,
                };
                d.at_ic.at = i64::MIN;
            }
            3 => {
                d.action = action(&c.effect, REFUND)?;
                d.reason = PaidResaleDecisionKindV1::RecipientUnavailable {};
            }
            _ => {
                d.action = action(&c.effect, REFUND)?;
                d.reason = PaidResaleDecisionKindV1::RegistryRevoked {
                    sanction: CommitToken::from_bytes([249; 16]),
                    licence: c.terms.basis.licence,
                    generation: c.terms.basis.term.generation,
                    term_digest: c.terms.basis.term.digest,
                };
                d.at_ic.at = c.terms.basis.term.window.grace_end - 1;
            }
        }
        d.matches_consent(c)?;
        let plan = plans::terminal(row.intent(), &d)?;
        let mut hold = row.acceptance.hold.clone().ok_or("hold")?;
        hold.status = if d.reason.settles() {
            HoldStatus::Consumed {
                transaction: TransactionId(u64::MAX),
                amount: c.terms.price,
            }
        } else {
            HoldStatus::Released
        };
        hold.revision = 2;
        hold.last_journal_seq = u64::MAX;
        let tx = EconomicTransaction {
            id: TransactionId(u64::MAX),
            journal_seq: u64::MAX,
            idempotency: plan.idempotency.clone(),
            cause: plan.cause.clone(),
            actor: plan.actor.clone(),
            posted_at: d.posted_at_utc,
            description: plan.description.clone(),
            lines: plan
                .requested_lines
                .iter()
                .map(|l| JournalLine {
                    account: l.account,
                    instrument: c.terms.instrument.clone(),
                    delta: l.delta,
                    memo: l.memo.clone(),
                })
                .collect(),
            reverses: None,
            visibility: plan.visibility.clone(),
            plan_hash: crate::hash::plan_hash(&plan),
            asset_effects: vec![],
            custody_effects: vec![],
            consumed_approval: None,
            hold_effects: vec![HoldEffect {
                kind: if d.reason.settles() {
                    HoldEffectKind::Captured
                } else {
                    HoldEffectKind::Released
                },
                hold,
            }],
        };
        let after = with_terminal(&row, &tx, d)?;
        let encoded = subject::bounded(&after, MAX_HISTORY_BYTES)?;
        assert!(PaidResaleHistoryV1::decode(&encoded)? == after);
        let contract = after.contract()?;
        let write =
            crate::registry_filing::json_write(crate::api::contract_key(contract.id), &contract)?;
        assert!(subject::bounded(&write, MAX_CONTRACT_BYTES)?.len() <= MAX_CONTRACT_BYTES);
        assert!(
            subject::bounded(&tx, crate::engine::MAX_ENTRY_BYTES)?.len()
                <= crate::engine::MAX_ENTRY_BYTES
        );
        let ledger = mush_core::ledger::Ledger::new();
        let append =
            ledger.prepare_append(crate::module::JOURNAL_LOG, serde_json::to_value(&tx)?)?;
        let replay =
            append.authoritative_replay_write(mush_core::mechanics::OwnerId::new("economy")?)?;
        assert!(
            subject::bounded(&replay, crate::engine::MAX_ENTRY_BYTES)?.len()
                <= crate::engine::MAX_ENTRY_BYTES
        );
    }
    Ok(())
}
