//! The Market owns one Capability validator. Both concrete policy identities
//! are exact and independently checked; there is no arbitrary callback registry.
use super::guard::EntitlementGuard;
use mush_core::action::{
    ActionError, AuthorityGuard, AuthorityGuardContext, AuthorityGuardKind, AuthorityGuardValidator,
};
use mush_core::{ModError, Registrar};
use std::sync::Arc;
struct Capabilities {
    original: Option<crate::mind_trade::PurchaseGuard>,
    entitlement: Option<EntitlementGuard>,
    house: Vec<crate::mind_house::HouseGuard>,
}
impl AuthorityGuardValidator for Capabilities {
    fn is_current(
        &self,
        ctx: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        if let Some(original) = &self.original {
            if original.is_current(ctx, guard)? {
                return Ok(true);
            }
        }
        if let Some(entitlement) = &self.entitlement {
            if entitlement.is_current(ctx, guard)? {
                return Ok(true);
            }
        }
        for house in &self.house {
            if house.is_current(ctx, guard)? {
                return Ok(true);
            }
        }
        if crate::mind_renewal::service_guard_current(ctx, guard) {
            return Ok(true);
        }
        if crate::mind_house_renewal::service_guard_current(ctx, guard) {
            return Ok(true);
        }
        Ok(false)
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    original: Option<&crate::mind_trade::OriginalPurchaseConfig>,
    entitlement: Option<&super::EntitlementConfig>,
    gate: Option<Arc<dyn crate::VenueGate>>,
) -> Result<(), ModError> {
    register_with_house(reg, original, entitlement, None, gate)
}
pub(crate) fn register_with_house(
    reg: &mut Registrar<'_>,
    original: Option<&crate::mind_trade::OriginalPurchaseConfig>,
    entitlement: Option<&super::EntitlementConfig>,
    house: Option<&crate::mind_house::HouseConfig>,
    gate: Option<Arc<dyn crate::VenueGate>>,
) -> Result<(), ModError> {
    if original.is_none() && entitlement.is_none() && house.is_none() {
        return Ok(());
    }
    let gate = gate.ok_or_else(|| super::error("mind purchases require the actual venue gate"))?;
    let original = original
        .map(|c| crate::mind_trade::PurchaseGuard::new(c.quotes.clone(), gate.clone()))
        .transpose()?;
    let entitlement = entitlement
        .map(|c| EntitlementGuard::new(c.clone(), gate.clone()))
        .transpose()?;
    let mut house_guards = Vec::new();
    if let Some(c) = house {
        house_guards.push(crate::mind_house::HouseGuard::new(c.clone(), gate.clone())?);
        if c.has_two_seat_pins() {
            house_guards.push(crate::mind_house::HouseGuard::for_protocol(
                c.clone(),
                gate.clone(),
                mush_roll::commerce::house_purchase::HouseProtocol::HouseV2,
            )?);
        }
    }
    let house = house_guards;
    reg.authority_guard_validator(
        AuthorityGuardKind::Capability,
        Capabilities {
            original,
            entitlement,
            house,
        },
    )
}
