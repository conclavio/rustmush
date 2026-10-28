//! Fixed four-target pending charter completion. Native escrow, title, claims
//! and service authority remain actual owner obligations.
use super::*;
use crate::action::{ActionProviderRegistry, PrincipalId};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};
mod protocol;
mod transition;
mod wire;
pub(in crate::action_state) use protocol::Protocol;
pub(in crate::action_state) use transition::charter_pending_change;
pub(super) use transition::{audit_boot, extra_terminal_key, validate_changes};
use wire::*;
const OFFER: &str = "roll_mind_charter_pending_offers";
const PENDING: &str = "roll_mind_charter_pending";
const HEAD: &str = "roll_mind_charter_pending_head";
const TRANSFER: &str = "roll_mind_charter_transfers";
const OWNERSHIP: &str = "roll_mind_charter_ownership";
const SANCTION: &str = "roll_registry_term_revocations";
const STATUS: &str = "roll_registry_revoked_terms";
const FAMILIES: [(&str, usize); 5] = [
    (OFFER, 8192),
    (PENDING, 40960),
    (HEAD, 4096),
    (TRANSFER, 40960),
    (OWNERSHIP, 4096),
];
const ACTIONS: [&str; 4] = [
    "offer_mind_charter_transfer",
    "fund_mind_charter_transfer",
    "settle_mind_charter_transfer",
    "refund_mind_charter_transfer",
];
fn target(
    family: &str,
    encoded: String,
    ceiling: usize,
    mode: SlotMode,
) -> Result<Target, ActionStateError> {
    Ok(Target {
        key: key(family, encoded)?,
        schema: CanonicalId::new(format!("{family}.value.v1"))
            .map_err(|_| refused("pending schema"))?,
        schema_version: 1,
        ceiling,
        mode,
    })
}
fn targets(p: &Pair) -> Result<Vec<Target>, ActionStateError> {
    let mut targets = vec![
        target(
            PENDING,
            p.operation.to_hex(),
            40960,
            SlotMode::UpdateExisting,
        )?,
        target(HEAD, charter_key(p.charter), 4096, SlotMode::UpdateExisting)?,
        target(
            TRANSFER,
            transfer_key(p.charter, p.generation + 1),
            40960,
            SlotMode::Create,
        )?,
        target(
            OWNERSHIP,
            charter_key(p.charter),
            4096,
            if p.generation == 1 {
                SlotMode::Create
            } else {
                SlotMode::UpdateExisting
            },
        )?,
    ];
    targets.sort_by(|a, b| {
        (&a.key.owner, &a.key.family, &a.key.encoded).cmp(&(
            &b.key.owner,
            &b.key.family,
            &b.key.encoded,
        ))
    });
    Ok(targets)
}
pub(super) fn verify(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    let p = pair(view, reservation.funding)?;
    let protocol = view
        .registry
        .charter_pending
        .as_ref()
        .ok_or_else(|| refused("pending protocol unavailable"))?;
    if reservation.format != 1
        || reservation.effect != protocol.actions[1]
        || reservation.sponsor != key(PENDING, p.operation.to_hex())?
        || reservation.targets != targets(&p)?
        || reservation.protocol_fingerprint != Some(fingerprint(&p)?)
        || (matches!(phase, Phase::Funding | Phase::Retained) && p.phase != "funded")
        || (matches!(phase, Phase::Terminal) && p.phase == "funded")
    {
        return Err(refused("pending reservation differs"));
    }
    head_matches(view, &p)?;
    if p.phase == "funded" {
        require_current_ownership(view, &p)?;
        if value(
            view,
            &key(TRANSFER, transfer_key(p.charter, p.generation + 1))?,
        )?
        .is_some()
        {
            return Err(refused("pending future ownership already exists"));
        }
    }
    if p.phase == "settled" {
        require_transfer(view, &p)?;
        if matches!(phase, Phase::Terminal) {
            require_new_ownership(view, &p)?;
        }
    }
    Ok(())
}
#[cfg(test)]
pub(super) mod tests;
