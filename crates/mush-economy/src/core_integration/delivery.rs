//! Current receiving facts from actual complete E4 history. This function is a
//! read only check, not an authorization constructor or a custody mutation.
use super::{
    CoreClass, CoreDeliveryPromise, CoreIntegrationError as Error, CorePaymentHistory,
    CoreTerminalResult,
};
use crate::{
    AccountStatus, AssetLifecycle, EconomyStore, InstrumentId, InstrumentKind, InstrumentStatus,
};
use mush_core::{CommitToken, Flags, ObjKind, World};
use mush_org::PrincipalId;
use std::{any::TypeId, collections::BTreeSet};

/// Resolve the actual acknowledged completed order and current personal title
/// and Class III receiving eligibility. The caller must independently bind its
/// trusted action/readiness and exact Industry order to this borrowed history.
/// Former supplier/operator/recipe eligibility is deliberately not consulted.
pub fn require_core_delivery(
    world: &World,
    acceptance: CommitToken,
) -> Result<&CorePaymentHistory, Error> {
    if world
        .module_state_owner(TypeId::of::<EconomyStore>())
        .is_none_or(|o| o.as_str() != "economy")
    {
        return Err(Error::Unavailable);
    }
    let state = world
        .module_state::<EconomyStore>()
        .ok_or(Error::Unavailable)?;
    let history = state.core_payment_history(acceptance)?;
    if history.policy().delivery != Some(CoreDeliveryPromise::PersonalCustodyV1) {
        return Err(Error::Unavailable);
    }
    let terminal = history.terminal().ok_or(Error::Unavailable)?;
    let CoreTerminalResult::Completed {
        output,
        title: birth,
    } = terminal.result()
    else {
        return Err(Error::Unavailable);
    };
    let requester = history.subject().requester;
    let snapshot = state.snapshot();
    let title = snapshot.asset(output.object).ok_or(Error::Unavailable)?;
    let (object, managed_birth) = history.managed_title_birth().ok_or(Error::Integrity)?;
    if object != output.object
        || birth.object != object
        || snapshot.managed_titles.core(object) != Some(&managed_birth)
        || title.title_owner != PrincipalId::Object(requester)
        || title.scope.is_some()
        || title.lifecycle != AssetLifecycle::Active
        || title.frozen
        || title.stolen.is_some()
        || !title.encumbrances.is_empty()
        || world
            .get(requester)
            .is_none_or(|p| p.kind != ObjKind::Player || p.flags.contains(Flags::GOING))
    {
        return Err(Error::Unavailable);
    }
    // This first delivery moves only ordinary personal containment. No vault
    // endpoint or nested vault ancestry can be smuggled through a CarryAttempt.
    for start in [object, history.policy().output_container, requester] {
        let mut at = Some(start);
        let mut seen = BTreeSet::new();
        while let Some(id) = at {
            if seen.len() >= mush_core::MAX_CONTAINMENT_DEPTH
                || !seen.insert(id)
                || snapshot.is_vault_backing(id)
            {
                return Err(Error::Unavailable);
            }
            let row = world
                .get(id)
                .filter(|o| !o.flags.contains(Flags::GOING))
                .ok_or(Error::Unavailable)?;
            at = row.location;
        }
    }
    if history.subject().class == CoreClass::III {
        let id = InstrumentId::new(super::subject::CORE3_ENTITLEMENT).ok_or(Error::Invalid)?;
        let entitlement = snapshot.instrument(&id).ok_or(Error::Unavailable)?;
        let principal = PrincipalId::Object(requester);
        if entitlement.kind != InstrumentKind::Entitlement
            || entitlement.scale != 0
            || entitlement.transferable
            || entitlement.status != InstrumentStatus::Active
            || !snapshot.accounts_of(&principal).any(|a| {
                a.instrument == id
                    && a.scope.is_none()
                    && a.status == AccountStatus::Active
                    && a.posted_balance.minor() > 0
            })
        {
            return Err(Error::Unavailable);
        }
    }
    Ok(history)
}
