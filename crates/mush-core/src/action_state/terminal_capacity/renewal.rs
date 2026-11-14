//! The personal issuer of the renewal twin (doc 46 CT.3): a licence a player sold, renewed
//! by the one validator in `renewal_twin.rs`. This file is what is the personal issuer's
//! alone: its identity strings, the two closed purchase audits it shares its birth records
//! with, the personal economics family, and the royalty chain only a personal grant has.
use super::*;
use renewal_twin::{House, Issuer, Personal, field};

impl Issuer for Personal {
    const ACTIONS: [&'static str; 5] = [
        "draft_mind_renewal",
        "fund_mind_renewal",
        "settle_mind_renewal",
        "refund_mind_renewal",
        "close_mind_renewal",
    ];
    const SERVICE_ACTION: &'static str = "close_mind_renewal_service";
    const PROTOCOL: &'static str = "personal_mind_renewal_v1";
    const SERVICE_PROTOCOL: &'static str = "personal_mind_renewal_service_v1";
    const MARKET: &'static str = "market_mind_renewal_operation";
    const ROLL: &'static str = "roll_mind_renewal_operation";
    const HEAD: &'static str = "roll_mind_renewal_head";
    const RECEIPT: &'static str = "market_mind_renewal_receipt";
    const ECONOMICS: &'static str = "roll_mind_paid_term_economics";
    const OTHER_ISSUER: [&'static str; 2] = [House::ROLL, House::MARKET];
    const PREDECESSOR_DOMAIN: &'static str = "roll.mind.renewal_predecessor.v1";
    const OFFER_DOMAIN: &'static str = "economy.mind_renewal_offer.v1";
    const TEMPLATE_DOMAIN: &'static str = "roll.mind.renewal_template.v1";
    const CAPACITY_DOMAIN: &'static str = "core.mind_renewal.capacity.v1";

    fn protocol(registry: &StateFamilyRegistry) -> Option<&Protocol> {
        registry.mind_renewal.as_deref()
    }

    fn birth_audits(registry: &StateFamilyRegistry) -> Result<(), ActionStateError> {
        // Renewal shares immutable birth records with both closed issuance paths.
        if registry.original_purchase.is_none() || registry.entitlement_purchase.is_none() {
            return Err(refused("renewal requires both closed purchase audits"));
        }
        Ok(())
    }

    fn economics_domain(economics: &Value) -> Result<&'static str, ActionStateError> {
        entitlement::economics_domain(economics)
    }
    fn grant_domain(grant: &Value) -> Result<&'static str, ActionStateError> {
        entitlement::grant_domain(grant)
    }
    fn grant_version(grant: &Value) -> Result<u64, ActionStateError> {
        entitlement::grant_version(grant)
    }

    fn follow_birth(view: &View<'_>, binding: &Value) -> Result<(), ActionStateError> {
        entitlement::require_follow_birth(view, binding)
    }

    fn royalty_chain_differs(basis: &Value, grant: &Value) -> Result<bool, ActionStateError> {
        Ok(field(basis, "royalty_chain_digest")?
            != field(field(grant, "royalty_chain")?, "digest")?)
    }

    // Nothing more to prove: `follow_birth` and the holder join both issuers run
    // (`paid_resale::renewal_holder`) have held a personal licence to its birth.
    fn birth_route(
        _view: &View<'_>,
        _binding: &Value,
        _basis: &Value,
        _terms: &Value,
    ) -> Result<(), ActionStateError> {
        Ok(())
    }

    // A personal Follow birth is held to its recipe in `follow_birth`, at every read.
    fn birth_product(_after: &View<'_>, _binding: &Value) -> Result<(), ActionStateError> {
        Ok(())
    }
}

renewal_twin::entry_points!(Personal);

#[cfg(test)]
pub(super) mod tests;
