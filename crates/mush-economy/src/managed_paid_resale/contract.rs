//! Exact native buyer acceptance. A separate acknowledged seller offer is
//! mandatory in Roll/Market; no seller acceptance is invented here.
use super::plans::Intent;
use super::*;
use crate::TransactionId;
use crate::contract::managed::ManagedConsent;
use crate::contract::{
    Acceptance, ConsentSurface, Consideration, ContractRoles, ContractState, ContractTerms,
    EconomicContract, Expiry, PayoutShare, Share, consent_hash,
};
use crate::record::EconomicVisibility;
use mush_org::PrincipalId;

pub(super) fn accepted(
    i: &Intent,
    tx: TransactionId,
) -> Result<EconomicContract, ParticipantError> {
    i.validate()?;
    let c = &i.consent;
    let buyer = PrincipalId::Object(c.terms.buyer);
    let surface = ConsentSurface {
        fact_consent: None,
        managed: Some(ManagedConsent::paid_resale(PaidResaleMarkerV1::of(c)?)?),
        parties: vec![buyer.clone()],
        terms: ContractTerms {
            consideration: vec![Consideration {
                from: buyer.clone(),
                account: c.payer,
                amount: c.terms.price,
            }],
            deliverables: Vec::new(),
            split: c
                .terms
                .payouts(c.payer)?
                .into_iter()
                .map(|p| PayoutShare {
                    to: p.account,
                    share: Share::Exact(p.amount),
                })
                .collect(),
            remainder_to: c.terms.remainder.account,
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
    let hash = consent_hash(&buyer, &surface);
    let row = EconomicContract {
        id: i.contract,
        issuer: buyer.clone(),
        name: "Paid mind licence resale".into(),
        consent: surface,
        consent_hash: hash,
        acceptances: vec![Acceptance {
            party: buyer,
            at: c.accepted_at_utc,
            consent_hash: hash,
        }],
        satisfied: Vec::new(),
        escrow: i.hold.into_iter().collect(),
        state: if c.mode == PaidResaleModeV1::Immediate {
            ContractState::Settled { transaction: tx }
        } else {
            ContractState::ReadyToSettle
        },
        settle_attempts: 0,
        created_at: c.accepted_at_utc,
        amendments: 0,
        generation_started_at: c.accepted_at_utc,
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
    subject::bounded(&row, MAX_CONTRACT_BYTES)?;
    Ok(row)
}
