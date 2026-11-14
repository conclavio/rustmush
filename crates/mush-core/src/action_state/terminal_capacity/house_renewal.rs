//! The House issuer of the renewal twin (doc 46 CT.3): a licence an organisation issued,
//! renewed by the one validator in `renewal_twin.rs`. This file is what is the House's
//! alone: its identity strings, the one closed purchase audit it needs, the House
//! economics family, and the proof of the exact organisation-issued birth.
use super::*;
use renewal_twin::{House, Issuer, Personal, field, number, optional_family_value, token};

impl Issuer for House {
    const ACTIONS: [&'static str; 5] = [
        "draft_house_mind_renewal",
        "fund_house_mind_renewal",
        "settle_house_mind_renewal",
        "refund_house_mind_renewal",
        "close_house_mind_renewal",
    ];
    const SERVICE_ACTION: &'static str = "close_house_mind_renewal_service";
    const PROTOCOL: &'static str = "house_mind_renewal_v1";
    const SERVICE_PROTOCOL: &'static str = "house_mind_renewal_service_v1";
    const MARKET: &'static str = "market_house_mind_renewal_operation";
    const ROLL: &'static str = "roll_house_mind_renewal_operation";
    const HEAD: &'static str = "roll_house_mind_renewal_head";
    const RECEIPT: &'static str = "market_house_mind_renewal_receipt";
    const ECONOMICS: &'static str = "roll_mind_house_term_economics";
    const OTHER_ISSUER: [&'static str; 2] = [Personal::ROLL, Personal::MARKET];
    const PREDECESSOR_DOMAIN: &'static str = "roll.mind.house_renewal_predecessor.v1";
    const OFFER_DOMAIN: &'static str = "economy.house_mind_renewal_offer.v1";
    const TEMPLATE_DOMAIN: &'static str = "roll.mind.house_renewal_template.v1";
    const CAPACITY_DOMAIN: &'static str = "core.house_mind_renewal.capacity.v1";

    fn protocol(registry: &StateFamilyRegistry) -> Option<&Protocol> {
        registry.house_mind_renewal.as_deref()
    }

    fn birth_audits(registry: &StateFamilyRegistry) -> Result<(), ActionStateError> {
        // House renewal requires its exact organization-issued purchase audit.
        if registry.house_purchase.is_none() {
            return Err(refused("house renewal requires its closed birth audit"));
        }
        Ok(())
    }

    fn economics_domain(economics: &Value) -> Result<&'static str, ActionStateError> {
        house::economics_domain(economics)
    }
    fn grant_domain(grant: &Value) -> Result<&'static str, ActionStateError> {
        house::grant_domain(grant)
    }
    fn grant_version(grant: &Value) -> Result<u64, ActionStateError> {
        house::grant_version(grant).map(u64::from)
    }

    // The House proves its whole birth, Follow or not, in `birth_route`.
    fn follow_birth(_view: &View<'_>, _binding: &Value) -> Result<(), ActionStateError> {
        Ok(())
    }

    // A House grant has no royalty chain to compare. `require_house_birth` refuses an
    // offer that carries one.
    fn royalty_chain_differs(_basis: &Value, _grant: &Value) -> Result<bool, ActionStateError> {
        Ok(false)
    }

    fn birth_route(
        view: &View<'_>,
        binding: &Value,
        basis: &Value,
        terms: &Value,
    ) -> Result<(), ActionStateError> {
        require_house_birth(view, binding, basis, terms)
    }

    fn birth_product(after: &View<'_>, binding: &Value) -> Result<(), ActionStateError> {
        let birth = token(field(binding, "birth_operation")?)?;
        let allocation = optional_family_value(after, "roll_mind_house_purchase", birth)?
            .ok_or_else(|| refused("house renewal birth unavailable"))?;
        after
            .registry
            .house_purchase
            .as_ref()
            .ok_or_else(|| refused("house purchase recipe unavailable"))?
            .require_product(&allocation)
    }
}

// Birth selection is independent of current issuer eligibility. This proves
// only the exact immutable House allocation; live rights remain owner checks.
fn require_house_birth(
    view: &View<'_>,
    binding: &Value,
    basis: &Value,
    terms: &Value,
) -> Result<(), ActionStateError> {
    let operation = token(field(binding, "birth_operation")?)?;
    let allocation = optional_family_value(view, "roll_mind_house_purchase", operation)?
        .ok_or_else(|| refused("house renewal birth allocation missing"))?;
    if view.registry.house_purchase.is_none()
        || optional_family_value(view, "roll_mind_paid_purchase", operation)?.is_some()
        || optional_family_value(view, "roll_mind_entitlement_purchase", operation)?.is_some()
        || optional_family_value(view, "roll_mind_renewal_operation", operation)?.is_some()
        || optional_family_value(view, "market_mind_renewal_operation", operation)?.is_some()
        || optional_family_value(view, House::ROLL, operation)?.is_some()
        || optional_family_value(view, House::MARKET, operation)?.is_some()
        || field(field(&allocation, "phase")?, "kind")? != "granted"
        || house::allocation_version(&allocation)?
            != house::grant_version(field(binding, "grant")?)?
        || field(&allocation, "operation")? != &json!(operation)
        || field(&allocation, "licence")? != field(binding, "licence")?
        || field(&allocation, "quote")? != field(basis, "prior_quote")?
        || field(&allocation, "quote_digest")? != field(basis, "prior_quote_digest")?
        || field(&allocation, "product_digest")? != field(binding, "product_digest")?
    {
        return Err(refused("house renewal birth route differs"));
    }
    if matches!(number(binding, "format")?, 3 | 4) {
        view.registry
            .house_purchase
            .as_ref()
            .ok_or_else(|| refused("house Follow birth protocol absent"))?
            .require_product(&allocation)?;
        if house::grant_version(field(binding, "grant")?)? != 3 {
            return Err(refused("house Follow binding requires Follow acquisition"));
        }
    }
    for name in ["issuer", "charter", "line", "filed", "revision", "terms"] {
        if field(&allocation, name)? != field(binding, name)? {
            return Err(refused("house renewal birth subject differs"));
        }
    }
    // M8c F4: by principal (the binding names the renewing actor as
    // `acting` for an organisation-held licence).
    if matches!(number(binding, "format")?, 1 | 3)
        && !owner_index::same_holder(field(&allocation, "holder")?, field(binding, "holder")?)?
    {
        return Err(refused("house renewal original holder differs"));
    }
    let issuer = field(binding, "issuer")?;
    if issuer.as_object().is_none_or(|o| o.len() != 3)
        || number(issuer, "format")? != 1
        || field(issuer, "organization")? != field(terms, "organization")?
        || field(issuer, "scope")? != field(terms, "scope")?
        || basis.get("royalty_chain_digest").is_some()
        || ["seller", "lineage", "parents"]
            .iter()
            .any(|n| terms.get(*n).is_some())
    {
        return Err(refused("house renewal issuer or native shape differs"));
    }
    // Org and scope zero are actual valid IDs; no synthetic seller is accepted.
    number(issuer, "organization")?;
    number(issuer, "scope")?;
    Ok(())
}

renewal_twin::entry_points!(House);

#[cfg(test)]
pub(super) mod tests;
