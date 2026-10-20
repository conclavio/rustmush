//! Complete native envelope sizing. Constructed values below are explicit
//! synthetic sizing data, never an owner output or historical authority.
use super::*;
use crate::record::{CatalogRef, HoldPurpose};
use mush_core::{ClockRef, ClockStamp, DefinitionDigest, ObjId};
use mush_org::SystemPrincipalId;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn sized_intent() -> Result<FundingIntent> {
    let mut i = FundingIntent {
        consent: super::super::tests::maximum_consent()?,
        issuer: SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
        catalog: CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
        contract: ContractId(u64::MAX),
        hold: HoldId(u64::MAX),
    };
    // The catalogue strings are native configuration, not fixed-width IDs.
    // Push the real complete correlation to its accepted boundary instead of
    // claiming small fixture names establish headroom for arbitrary metadata.
    let mut low = 0;
    let mut high = super::super::MAX_PENDING_CORRELATION_BYTES + 1;
    while low + 1 < high {
        let middle = (low + high) / 2;
        let mut candidate = i.clone();
        candidate.catalog.code = "c".repeat(middle).into();
        if candidate.validate().is_ok() {
            low = middle;
        } else {
            high = middle;
        }
    }
    i.catalog.code = "c".repeat(low).into();
    i.validate()?;
    let proof = Proof::Fund {
        format: 1,
        intent: Box::new(i.clone()),
    };
    assert_eq!(
        plans::encode(&proof)?.len(),
        super::super::MAX_PENDING_CORRELATION_BYTES
    );
    let mut too_big = i.clone();
    too_big.catalog.code = "c".repeat(low + 1).into();
    assert!(too_big.validate().is_err());
    Ok(i)
}
fn funded(i: FundingIntent) -> Result<CharterPendingHistory> {
    let c = &i.consent;
    let hold = Hold {
        id: i.hold,
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
    let row = CharterPendingHistory {
        funding: Box::new(CharterPendingFundingReceiptV1 {
            transaction: TransactionId(u64::MAX - 1),
            journal_seq: u64::MAX - 1,
            plan_hash: crate::hash::plan_hash(&plans::funding(&i)?),
            hold,
        }),
        intent: i,
        terminal: None,
    };
    row.validate()?;
    row.require_terminal_room()?;
    Ok(row)
}
fn action(id: &str) -> Result<mush_core::DefinitionRef> {
    use mush_core::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    };
    let mut value = serde_json::to_value(DefinitionRegistryBuilder::new().register(
        OwnerId::new("roll")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new(id)?,
            1,
            serde_json::json!({"sizing":true}),
        )?,
    )?)?;
    *value.get_mut("digest").ok_or("action digest")? =
        serde_json::to_value(DefinitionDigest::from_bytes([255; 32]))?;
    Ok(serde_json::from_value(value)?)
}
#[test]
fn exact_correlation_boundary_preserves_complete_terminal_history_contract_and_replay_room()
-> Result {
    let before = funded(sized_intent()?)?;
    let consent = before.consent();
    for n in 0..5 {
        let mut d = super::super::CharterPendingDecisionV1 {
            format: 1,
            operation: token(251),
            action: action(super::super::SETTLE_ACTION)?,
            actor: ObjId(u32::MAX),
            at_ic: ClockStamp {
                clock: consent.accepted_at_ic.clock.clone(),
                at: i64::MAX,
            },
            posted_at_utc: i64::MAX,
            reason: super::super::CharterPendingDecisionKindV1::DeadlineReached {
                next_transition: u64::MAX,
                open_owned: u32::MAX,
            },
        };
        match n {
            0 => (),
            1 => {
                d.reason = super::super::CharterPendingDecisionKindV1::LastClaimClosed {
                    next_transition: u64::MAX,
                }
            }
            2 => {
                d.action = action("registry_term_revocation")?;
                d.reason = super::super::CharterPendingDecisionKindV1::RegistryRevoked {
                    sanction: d.operation,
                    licence: u64::MAX,
                    generation: u64::MAX,
                    term_digest: consent.terms.owner_term.digest,
                };
            }
            3 => {
                d.action = action(super::super::REFUND_ACTION)?;
                d.reason = super::super::CharterPendingDecisionKindV1::ClockChanged;
                d.at_ic.clock = ClockRef::WorldIc {
                    generation: u64::MAX - 1,
                };
                d.at_ic.at = i64::MIN;
            }
            _ => {
                d.action = action(super::super::REFUND_ACTION)?;
                d.reason = super::super::CharterPendingDecisionKindV1::RecipientUnavailable;
            }
        }
        d.matches_consent(consent)?;
        let mut row = before.clone();
        let mut hold = before.funding.hold.clone();
        hold.status = if d.reason.settles() {
            HoldStatus::Consumed {
                transaction: TransactionId(u64::MAX),
                amount: consent.terms.price,
            }
        } else {
            HoldStatus::Released
        };
        hold.revision = 2;
        hold.last_journal_seq = u64::MAX;
        let title = if d.reason.settles() {
            Some(title_after(
                &before.intent,
                TransactionId(u64::MAX),
                u64::MAX,
                d.posted_at_utc,
            )?)
        } else {
            None
        };
        row.terminal = Some(Box::new(CharterPendingTerminalReceiptV1 {
            plan_hash: crate::hash::plan_hash(&plans::terminal(&before.intent, &d)?),
            decision: d.clone(),
            transaction: TransactionId(u64::MAX),
            journal_seq: u64::MAX,
            hold,
            title,
        }));
        row.validate()?;
        let raw = bounded(&row, super::super::MAX_PENDING_HISTORY_BYTES)?;
        assert!(CharterPendingHistory::decode(&raw)? == row);
        let contract = row.contract()?;
        let write =
            crate::registry_filing::json_write(crate::api::contract_key(contract.id), &contract)?;
        // The ReplayWrite wrapper and maximum decimal key are measured as
        // well as the document; a document-only fit is not this assertion.
        assert!(serde_json::to_vec(&write)?.len() <= super::super::MAX_PENDING_CONTRACT_BYTES);
        assert!(bounded(&d, 2048)?.len() <= 2048);
        assert!(
            plans::encode(&Proof::Terminal {
                format: 1,
                funding: consent.operation,
                decision: Box::new(d)
            })?
            .len()
                <= super::super::MAX_PENDING_CORRELATION_BYTES
        );
    }
    Ok(())
}
