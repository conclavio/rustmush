//! Native consent projection. Insurer acceptance is the actual closed product
//! response at Bind, proved by the paired producer and current Org treasury
//! authority; it is never attributed to a curator, buyer, staff or GOD account.
use super::{CoverageConsentV1, CoverageMarkerV1, refused};
use crate::contract::managed::ManagedConsent;
use crate::contract::{
    Acceptance, AttestorSet, ConsentSurface, Consideration, ContractRoles, ContractState,
    ContractTerms, Deliverable, EconomicContract, Expiry, PayoutShare, Share, consent_hash,
};
use crate::record::EconomicVisibility;
use crate::{ContractId, HoldId};
use mush_core::participant::ParticipantError;
use mush_org::PrincipalId;

pub(super) fn active(
    c: &CoverageConsentV1,
    id: ContractId,
    hold: HoldId,
) -> Result<EconomicContract, ParticipantError> {
    c.validate()?;
    let t = &c.terms;
    let buyer = PrincipalId::Object(t.buyer);
    let insurer = PrincipalId::Organization(t.insurer);
    let surface = ConsentSurface {
        fact_consent: None,
        managed: Some(ManagedConsent::hull_coverage(CoverageMarkerV1::of(c)?)?),
        parties: vec![buyer.clone(), insurer.clone()],
        terms: ContractTerms {
            consideration: vec![Consideration {
                from: insurer.clone(),
                account: t.treasury,
                amount: t.liability,
            }],
            deliverables: vec![
                Deliverable::Collateral {
                    object: c.basis.hull,
                    from: buyer.clone(),
                },
                Deliverable::TitleTransfer {
                    object: c.basis.hull,
                    from: buyer.clone(),
                    to: insurer.clone(),
                },
            ],
            split: vec![PayoutShare {
                to: t.payer,
                share: Share::Exact(t.liability),
            }],
            remainder_to: t.payer,
            on_expiry: Expiry::Refund,
            outcomes: Vec::new(),
        },
        // Closed IC loss verification is repeated by the paired owner. Generic
        // expiry or settlement cannot manufacture a proof or release this hold.
        conditions: Vec::new(),
        deadline: None,
        visibility: EconomicVisibility::Parties,
        scope: Some(t.scope),
        adjudicator: Some(AttestorSet::Party(PrincipalId::Object(t.adjudicator))),
        evidence: Vec::new(),
    };
    let hash = consent_hash(&insurer, &surface);
    let result = EconomicContract {
        id,
        issuer: insurer.clone(),
        name: "Hull destruction coverage".into(),
        consent: surface,
        consent_hash: hash,
        acceptances: vec![
            Acceptance {
                party: buyer,
                at: c.accepted_at_utc,
                consent_hash: hash,
            },
            Acceptance {
                party: insurer,
                at: c.accepted_at_utc,
                consent_hash: hash,
            },
        ],
        satisfied: Vec::new(),
        escrow: vec![hold],
        state: ContractState::Active,
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
    mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(&result).map_err(|_| refused())?,
        super::MAX_CONTRACT_BYTES,
    )
    .map_err(|_| refused())?;
    Ok(result)
}
