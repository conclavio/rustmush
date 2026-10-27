//! Consequences of an exact paid commission. Durable work and settlement receipts
//! are the proof; transient event tails and editable contract names grant nothing.

use mush_core::{CanonicalId, Ctx, World};
use mush_economy::{ContractId, ContractState, EconomyStore};
use mush_org::{OrgId, PrincipalId};
use sha2::{Digest, Sha256};

use super::SurveyOffer;
use crate::surface::survey::{SurveyRecord, SurveyStandingEffect, SurveyStandingTerms};

pub(super) fn capture(
    world: &World,
    offer: &SurveyOffer,
    sponsor: &PrincipalId,
) -> Result<SurveyStandingTerms, String> {
    let PrincipalId::Organization(beneficiary) = sponsor else {
        return Err("the survey beneficiary is not an institution".into());
    };
    public_organization(world, *beneficiary)?;
    let port = crate::port::coop::programme_port(world)
        .ok_or("the survey institutions are unavailable")?;
    let secondary = offer
        .secondary_effects
        .iter()
        .map(|effect| {
            let key = CanonicalId::new(effect.organization)
                .map_err(|_| "the survey institution is invalid")?;
            let organization = port
                .map
                .orgs
                .get(&key)
                .ok_or("the survey institution is unavailable")?;
            public_organization(world, OrgId(organization.org))?;
            Ok(SurveyStandingEffect {
                organization: OrgId(organization.org),
                delta: effect.delta,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let representative = world
        .module_state::<mush_faction::FactionState>()
        .and_then(|state| {
            state
                .factions
                .values()
                .find(|faction| faction.org == Some(*beneficiary))
        })
        .and_then(|faction| crate::port::relationships::survey_representative(world, faction.id));
    let terms = SurveyStandingTerms {
        beneficiary: *beneficiary,
        delta: offer.standing,
        secondary,
        representative,
    };
    terms.check(sponsor)?;
    Ok(terms)
}

fn public_organization(world: &World, organization: OrgId) -> Result<(), String> {
    let row = world
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| store.state().org(organization))
        .ok_or("the survey institution is unavailable")?;
    if row.hidden || row.status != mush_org::OrgStatus::Active {
        return Err("the survey institution is unavailable".into());
    }
    let factions = world
        .module_state::<mush_faction::FactionState>()
        .ok_or("the survey institution is unavailable")?;
    let mut linked = factions
        .factions
        .values()
        .filter(|faction| faction.org == Some(organization));
    if linked.next().is_some_and(|faction| faction.hidden) || linked.next().is_some() {
        return Err("the survey institution is unavailable".into());
    }
    Ok(())
}

/// Fixed-width domain-separated bytes, independent of names and JSON formatting.
/// The complete 256-bit digest rides inside the economy's accepted fact reference.
fn terms_digest(terms: &SurveyStandingTerms) -> String {
    let mut hash = Sha256::new();
    hash.update(b"tacsim.survey_standing.v1\0");
    hash.update(terms.beneficiary.0.to_le_bytes());
    hash.update(terms.delta.to_le_bytes());
    hash.update(terms.representative.map_or(0, |npc| npc.0).to_le_bytes());
    for effect in &terms.secondary {
        hash.update(effect.organization.0.to_le_bytes());
        hash.update(effect.delta.to_le_bytes());
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut digest = String::with_capacity(64);
    for byte in hash.finalize() {
        digest.push(char::from(HEX[usize::from(byte >> 4)]));
        digest.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    digest
}

pub(super) fn fact_id(row: &SurveyRecord) -> String {
    let base = format!("{}/{}", row.offer.as_str(), row.contract);
    row.standing.as_ref().map_or_else(
        || base.clone(),
        |terms| format!("{base}/{}", terms_digest(terms)),
    )
}

fn organization_name(world: &World, organization: OrgId) -> String {
    world
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| store.state().org(organization))
        .filter(|org| !org.hidden)
        .map_or_else(
            || format!("organization #{}", organization.0),
            |org| org.name.to_string(),
        )
}

fn changes_line(world: &World, terms: &SurveyStandingTerms) -> String {
    let mut changes = vec![format!(
        "{} {:+}",
        organization_name(world, terms.beneficiary),
        terms.delta
    )];
    changes.extend(terms.secondary.iter().map(|effect| {
        format!(
            "{} {:+}",
            organization_name(world, effect.organization),
            effect.delta
        )
    }));
    format!(
        "Standing after completed work is paid: {}.",
        changes.join("; ")
    )
}

pub(super) fn preview_line(world: &World, terms: &SurveyStandingTerms) -> String {
    format!(
        "{} An assigned public survey representative gains +10 trust and +5 disposition toward you after completed work is paid, if they remain assigned.",
        changes_line(world, terms)
    )
}

pub(super) fn line(world: &World, terms: &SurveyStandingTerms) -> String {
    let mut text = changes_line(world, terms);
    if terms.representative.is_some() {
        text.push_str(" Your original public survey representative also gains +10 trust and +5 disposition toward you while they remain assigned.");
    }
    text
}

/// Settled alone includes cancellation. Check completed work, original consent,
/// both acceptances, and the permanent index's exact settlement key. No RAM-tail
/// transaction body is needed, including after reboot and journal rotation.
fn paid(world: &World, row: &SurveyRecord) -> bool {
    if row.standing.is_none()
        || row.check().is_err()
        || row.completed.is_none()
        || row.cancelled.is_some()
        || !super::contract_valid(world, row)
    {
        return false;
    }
    let Some(store) = world.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(contract) = store.snapshot().contract(ContractId(row.contract)) else {
        return false;
    };
    let ContractState::Settled { transaction } = contract.state else {
        return false;
    };
    contract.pending.is_none()
        && contract.acceptances.iter().any(|acceptance| {
            acceptance.party == row.sponsor && acceptance.consent_hash == contract.consent_hash
        })
        && store
            .transaction_ref(transaction)
            .is_some_and(|posted| posted.idempotency == mush_economy::settle_key(contract.id))
}

pub(super) fn reconcile(ctx: &mut Ctx<'_>, row: &SurveyRecord) -> Result<(), String> {
    if !paid(ctx.w, row) {
        return Ok(());
    }
    let Some(terms) = &row.standing else {
        return Ok(());
    };
    let faction = mush_faction::ensure_organization_faction(ctx, terms.beneficiary)?;
    let mut effects = vec![mush_faction::StandingEffect {
        faction,
        delta: terms.delta,
    }];
    for effect in &terms.secondary {
        effects.push(mush_faction::StandingEffect {
            faction: mush_faction::ensure_organization_faction(ctx, effect.organization)?,
            delta: effect.delta,
        });
    }
    let cause = format!("tacsim.survey/{}/{}", row.contract, row.pilot.0);
    let reason = format!(
        "Completed paid survey {} (contract #{}).",
        row.offer.as_str(),
        row.contract
    );
    if mush_faction::record_standing(ctx, &cause, row.pilot, &effects, &reason)? {
        ctx.out.sys_to_actor(row.pilot, "tacsim", format!(
            "Paid survey {} recorded your institutional standing changes. +standing shows the result.",
            row.offer.as_str()
        ));
    }
    // Institutional credit is independent of the representative's availability.
    // A replacement NPC never inherits another representative's receipt.
    if let Some(npc) = terms.representative
        && crate::port::relationships::survey_representative(ctx.w, faction) == Some(npc)
    {
        let _ = mush_npc::record_completed_work_relationship(ctx, npc, row.pilot, &cause, &reason)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
