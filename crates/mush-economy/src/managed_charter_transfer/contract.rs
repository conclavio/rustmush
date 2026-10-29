//! The real buyer acceptance and sole hold are reconstructed exactly. Seller
//! consent is the separately acknowledged Roll offer; it is never forged here.
use super::{CharterPendingConsentV1, CharterPendingMarkerV1, refused};
use crate::contract::managed::ManagedConsent;
use crate::contract::{
    Acceptance, ConsentSurface, Consideration, ContractRoles, ContractState, ContractTerms,
    EconomicContract, Expiry, PayoutShare, Share, consent_hash,
};
use crate::record::EconomicVisibility;
use crate::{ContractId, HoldId};
use mush_core::participant::ParticipantError;
use mush_org::PrincipalId;

pub(super) fn funded(
    accepted: &CharterPendingConsentV1,
    id: ContractId,
    hold: HoldId,
) -> Result<EconomicContract, ParticipantError> {
    accepted.validate()?;
    let t = &accepted.terms;
    let buyer = PrincipalId::Object(t.buyer);
    let surface = ConsentSurface {
        fact_consent: None,
        managed: Some(ManagedConsent::charter_pending(
            CharterPendingMarkerV1::of(accepted)?,
        )?),
        parties: vec![buyer.clone()],
        terms: ContractTerms {
            consideration: vec![Consideration {
                from: buyer.clone(),
                account: accepted.payer,
                amount: t.price,
            }],
            deliverables: Vec::new(),
            split: vec![PayoutShare {
                to: t.seller_account,
                share: Share::Exact(t.price),
            }],
            remainder_to: t.seller_account,
            on_expiry: Expiry::Refund,
            outcomes: Vec::new(),
        },
        conditions: Vec::new(),
        // The captured WorldIC deadline is a positive settlement trigger owned
        // by this protocol. It is deliberately not generic contract expiry.
        deadline: None,
        visibility: EconomicVisibility::Parties,
        scope: None,
        adjudicator: None,
        evidence: Vec::new(),
    };
    let hash = consent_hash(&buyer, &surface);
    let c = EconomicContract {
        id,
        issuer: buyer.clone(),
        name: "Pending mind charter transfer".into(),
        consent: surface,
        consent_hash: hash,
        acceptances: vec![Acceptance {
            party: buyer,
            at: accepted.accepted_at_utc,
            consent_hash: hash,
        }],
        satisfied: Vec::new(),
        escrow: vec![hold],
        state: ContractState::ReadyToSettle,
        settle_attempts: 0,
        created_at: accepted.accepted_at_utc,
        amendments: 0,
        generation_started_at: accepted.accepted_at_utc,
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
    let raw = serde_json::to_vec(&c).map_err(|_| refused())?;
    if raw.len() > super::MAX_PENDING_CONTRACT_BYTES {
        return Err(refused());
    }
    Ok(c)
}
