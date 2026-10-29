//! Closed existing paid-licence resale accounting. Native consent, money,
//! current-holder/property and claim authority remain actual owner obligations.
use super::*;
use crate::action::{ActionProviderRegistry, PrincipalId};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};
mod capacity;
mod history;
mod renewal_holder;
pub(super) use renewal_holder::{
    renewal_current_holder, renewal_holder, renewal_holder_generation,
};
mod native;
mod protocol;
mod transition;
mod wire;
pub(in crate::action_state) use capacity::paid_resale_change;
pub(super) use capacity::verify;
pub(in crate::action_state) use protocol::Protocol;
pub(super) use transition::{
    audit_boot, compound_terminal_owner, extra_terminal_key, validate_changes,
};
use wire::*;
const OFFER: &str = "market_mind_paid_resale_offers";
const ROLL: &str = "roll_mind_paid_resales";
const MARKET: &str = "market_mind_paid_resales";
const HEAD: &str = "roll_mind_paid_resale_heads";
const HOLDER: &str = "roll_mind_paid_holders";
const LICENCE: &str = "roll_mind_personal_paid_licence";
const RECEIPT: &str = "market_mind_paid_resale_receipts";
const SANCTION: &str = "roll_registry_term_revocations";
const STATUS: &str = "roll_registry_revoked_terms";
const FAMILIES: [(&str, &str, usize); 7] = [
    ("market", OFFER, 16384),
    ("roll", ROLL, 40960),
    ("market", MARKET, 40960),
    ("roll", HEAD, 4096),
    ("roll", HOLDER, 32768),
    ("roll", LICENCE, 4096),
    ("market", RECEIPT, 8192),
];
const ACTIONS: [&str; 5] = [
    "offer_paid_mind_resale",
    "accept_paid_mind_resale",
    "fund_paid_mind_resale",
    "settle_paid_mind_resale",
    "refund_paid_mind_resale",
];

/// Purchase audits still prove the original holder in the immutable allocation.
/// Only this configured, exact current transfer chain explains another holder.
pub(super) fn validate_current_holder(
    view: &View<'_>,
    current: &Value,
    original_holder: &Value,
    birth: CommitToken,
) -> Result<(), ActionStateError> {
    if token(field(current, "operation")?)? != birth {
        return Err(refused("resale immutable birth changed"));
    }
    if num(current, "format")? == 1 {
        shape(
            current,
            &[
                "format",
                "id",
                "charter",
                "line",
                "holder",
                "term",
                "operation",
            ],
        )?;
        // M8c F4: by principal (a renewal re-stamps an organisation-held
        // pointer's `acting` with the renewing actor).
        if !super::owner_index::same_holder(field(current, "holder")?, original_holder)? {
            return Err(refused("resale original holder differs"));
        }
        return Ok(());
    }
    let id = decimal(field(current, "id")?, u64::MAX)?;
    let (_, allocation) = history::birth_allocation(view, birth)?;
    if !super::owner_index::same_holder(field(&allocation, "holder")?, original_holder)?
        || history::current_pointer(view, id)? != *current
    {
        return Err(refused("resale current holder proof differs"));
    }
    Ok(())
}

/// Narrow shared-family routing. The resale transition validator independently
/// requires every fixed paired postimage and checks the complete prior pointer.
pub(super) fn owns_licence_change(
    view: &View<'_>,
    candidate: &StateKey,
    identity: Option<&RegisteredTxnIdentity>,
    changes: &Changes,
) -> Result<bool, ActionStateError> {
    if candidate.owner.as_str() != "roll" || candidate.family.as_str() != LICENCE {
        return Ok(false);
    }
    let Some(protocol) = &view.registry.paid_resale else {
        return Ok(false);
    };
    let Some(identity) = identity else {
        return Ok(false);
    };
    let operation = match protocol.action_index(identity).ok() {
        Some(1) => identity.commit,
        Some(3) => {
            let id = changes
                .terminal
                .as_ref()
                .ok_or_else(|| refused("resale holder update lacks reservation"))?;
            if id.0.as_str() != "market" {
                return Err(refused("resale terminal namespace"));
            }
            token(&json!(id.1))?
        }
        _ => return Ok(false),
    };
    let p = pair(view, operation)?;
    if p.phase != "settled"
        || terminal(&p)? != identity.commit
        || *candidate != key(LICENCE, licence_key(p.licence))?.resolve(view.registry)?
    {
        return Err(refused("resale holder mutation crosses operation"));
    }
    history::require_holder(view, &p)?;
    Ok(true)
}

/// Fresh money/term admission cannot cross a retained resale escrow. Historical
/// birth/current reads and terminal cleanup deliberately do not use this gate.
pub(super) fn require_no_pending(view: &View<'_>, licence: u64) -> Result<(), ActionStateError> {
    let Some(head) = optional(view, HEAD, licence_key(licence))? else {
        return Ok(());
    };
    let id = token(field(field(&head, "current")?, "operation")?)?;
    let p = pair(view, id)?;
    head_matches(view, &p)?;
    if p.licence != licence || p.phase == "funded" {
        return Err(refused("renewal blocked by pending resale"));
    }
    Ok(())
}
