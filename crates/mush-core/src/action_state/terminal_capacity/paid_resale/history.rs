//! Immutable birth/accepted-term and independently current holder joins.
use super::*;

const BIRTHS: [(&str, &str); 3] = [
    ("legacy", "roll_mind_paid_purchase"),
    ("entitlement", "roll_mind_entitlement_purchase"),
    ("house", "roll_mind_house_purchase"),
];
pub(super) fn birth_allocation(
    view: &View<'_>,
    operation: CommitToken,
) -> Result<(String, Value), ActionStateError> {
    let mut found = None;
    for (route, family) in BIRTHS {
        if let Some(row) = optional(view, family, operation.to_hex())? {
            if found.is_some() {
                return Err(refused("resale ambiguous immutable birth"));
            }
            let enabled = match route {
                "legacy" => view.registry.original_purchase.is_some(),
                "entitlement" => view.registry.entitlement_purchase.is_some(),
                _ => view.registry.house_purchase.is_some(),
            };
            if !enabled
                || token(field(&row, "operation")?)? != operation
                || text(field(&row, "phase")?, "kind")? != "granted"
            {
                return Err(refused("resale birth not granted by configured protocol"));
            }
            match route {
                "entitlement"
                    if matches!(
                        row.get("protocol").and_then(Value::as_str),
                        Some("original_v4" | "derived_v2")
                    ) =>
                {
                    view.registry
                        .entitlement_purchase
                        .as_ref()
                        .ok_or_else(|| refused("Follow birth protocol absent"))?
                        .require_product(&row)?;
                }
                "house" if row.get("protocol").and_then(Value::as_str) == Some("house_v3") => {
                    view.registry
                        .house_purchase
                        .as_ref()
                        .ok_or_else(|| refused("House Follow birth protocol absent"))?
                        .require_product(&row)?;
                }
                _ => {}
            }
            found = Some((route.to_owned(), row));
        }
    }
    found.ok_or_else(|| refused("resale immutable birth absent"))
}
pub(super) fn binding_history(view: &View<'_>, binding: &Value) -> Result<(), ActionStateError> {
    native_basis(binding)?;
    let b = field(binding, "birth")?;
    let operation = token(field(b, "operation")?)?;
    let (route, allocation) = birth_allocation(view, operation)?;
    if b != &json!({"route":route,"operation":operation,"digest":digest("roll.paid_holder.birth.v1",&allocation)?})
        || ["licence", "charter", "line"]
            .iter()
            .any(|n| binding.get(*n) != allocation.get(*n))
    {
        return Err(refused("resale captured birth differs"));
    }
    let licence = decimal(field(binding, "licence")?, u64::MAX)?;
    let term = field(binding, "term")?;
    let generation = decimal(field(term, "generation")?, u64::MAX)?;
    if required(
        view,
        &key("roll_mind_paid_term", term_key(licence, generation))?,
    )? != *term
    {
        return Err(refused("resale accepted term differs"));
    }
    let term_operation = token(field(term, "operation")?)?;
    let issued = if generation == 1 {
        if term_operation != operation {
            return Err(refused("resale original term birth differs"));
        }
        let family = match route.as_str() {
            "legacy" => "market_mind_purchase_receipt",
            "entitlement" => "market_mind_entitlement_receipt",
            _ => "market_mind_house_receipt",
        };
        let receipt = required(view, &key(family, operation.to_hex())?)?;
        stamp(field(&receipt, "issued_at_ic")?)?
    } else {
        let (family, enabled) = if route == "house" {
            (
                "roll_house_mind_renewal_operation",
                view.registry.house_mind_renewal.is_some(),
            )
        } else {
            (
                "roll_mind_renewal_operation",
                view.registry.mind_renewal.is_some(),
            )
        };
        if !enabled {
            return Err(refused("resale term lacks renewal protocol"));
        }
        let renewal = required(view, &key(family, term_operation.to_hex())?)?;
        let state = field(&renewal, "state")?;
        if text(state, "kind")? != "granted"
            || field(field(&renewal, "binding")?, "licence")? != field(binding, "licence")?
            || field(field(&renewal, "binding")?, "next")? != field(term, "generation")?
        {
            return Err(refused("resale accepted renewal differs"));
        }
        stamp(field(field(state, "terminal")?, "issued_at_ic")?)?
    };
    if field(binding, "term_clock")? != &json!(issued.clock)
        || issued.at != signed(field(term, "window")?, "issued_at")?
    {
        return Err(refused("resale accepted term clock differs"));
    }
    if route == "house" || route == "entitlement" || generation > 1 {
        let family = if route == "house" {
            "roll_mind_house_term_economics"
        } else {
            "roll_mind_paid_term_economics"
        };
        let economics = required(view, &key(family, term_key(licence, generation))?)?;
        let domain = if route == "house" {
            super::super::house::economics_domain(&economics)?
        } else {
            super::super::entitlement::economics_domain(&economics)?
        };
        if field(&economics, "operation")? != field(term, "operation")?
            || field(binding, "economics_digest")? != &json!(digest(domain, &economics)?)
        {
            return Err(refused("resale accepted economics differs"));
        }
        if route != "house"
            && field(binding, "royalty_chain_digest")?
                != field(
                    field(field(&economics, "grant")?, "royalty_chain")?,
                    "digest",
                )?
        {
            return Err(refused("resale accepted royalty chain differs"));
        }
    }
    if route == "house" || (route == "legacy" && generation == 1) {
        let domain = if route == "house" {
            "roll.paid_resale.house_royalty_basis.v1"
        } else {
            "roll.paid_resale.legacy_royalty_basis.v1"
        };
        if field(binding, "royalty_chain_digest")?
            != &json!(digest(domain, &json!({"format":1,"birth":b,"parents":[]}))?)
        {
            return Err(refused("resale explicit parentless basis differs"));
        }
    }
    // Legacy economics is an opaque exact OriginalPurchase projection, proven
    // by the Economy/Roll owners. Never invert rounded payouts into rate terms.
    Ok(())
}
pub(super) fn holder_value(p: &Pair) -> Result<Value, ActionStateError> {
    let binding = field(&p.row, "binding")?;
    let t = field(binding, "term")?;
    let economic = field(&p.row, "economic")?;
    let c = field(field(economic, "intent")?, "consent")?;
    let action = if text(c, "mode")? == "immediate" {
        field(&p.row, "action")?
    } else {
        field(field(field(economic, "terminal")?, "decision")?, "action")?
    };
    Ok(
        json!({"format":1,"licence":field(binding,"licence")?,"generation":p.generation+1,
        "holder":field(field(&p.offer,"terms")?,"buyer")?,"birth":field(binding,"birth")?,"predecessor":field(binding,"ownership")?,
        "operation":terminal(p)?,"action":action,"acceptance":p.operation,"offer":field(&p.row,"offer")?,"offer_digest":field(&p.row,"offer_digest")?,
        "term":{"generation":field(t,"generation")?,"operation":field(t,"operation")?,"digest":digest("roll.paid_resale.term.v1",t)?},
        "accepted_at_ic":field(c,"accepted_at_ic")?,"economic_digest":digest("economy.paid_resale.history.v1",economic)?}),
    )
}
pub(super) fn require_holder(view: &View<'_>, p: &Pair) -> Result<Value, ActionStateError> {
    if p.phase != "settled" {
        return Err(refused("resale holder requires settlement"));
    }
    let actual = required(view, &key(HOLDER, holder_key(p.licence, p.generation + 1))?)?;
    if actual != holder_value(p)?
        || field(field(&p.row, "state")?, "ownership")?
            != &json!({"generation":p.generation+1,"operation":terminal(p)?,"digest":digest("roll.paid_holder.transfer.v1",&actual)?})
    {
        return Err(refused("resale exact holder record differs"));
    }
    Ok(actual)
}
/// This local join is constant depth. The mandatory whole-inventory audit below
/// proves each predecessor once, rather than recursively walking every chain.
pub(super) fn captured_holder(
    view: &View<'_>,
    binding: &Value,
    seller: &Value,
) -> Result<(), ActionStateError> {
    let ownership_ref = field(binding, "ownership")?;
    let generation = ownership(ownership_ref)?;
    let birth_operation = token(field(field(binding, "birth")?, "operation")?)?;
    if generation == 1 {
        let (_, allocation) = birth_allocation(view, birth_operation)?;
        if seller != field(&allocation, "holder")?
            || ownership_ref
                != &json!({"generation":1,"operation":birth_operation,"digest":field(field(binding,"birth")?,"digest")?})
        {
            return Err(refused("resale original holder differs"));
        }
    } else {
        let licence = decimal(field(binding, "licence")?, u64::MAX)?;
        let row = required(view, &key(HOLDER, holder_key(licence, generation))?)?;
        let prior = pair(view, token(field(&row, "acceptance")?)?)?;
        if require_holder(view, &prior)? != row
            || field(&row, "birth")? != field(binding, "birth")?
            || field(&row, "holder")? != seller
            || field(field(&prior.row, "state")?, "ownership")? != ownership_ref
        {
            return Err(refused("resale captured holder history differs"));
        }
    }
    Ok(())
}
pub(super) fn current_pointer(view: &View<'_>, licence: u64) -> Result<Value, ActionStateError> {
    let row = required(view, &key(LICENCE, licence_key(licence))?)?;
    let format = num(&row, "format")?;
    if format == 1 {
        shape(
            &row,
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
    } else if format == 2 {
        shape(
            &row,
            &[
                "format",
                "id",
                "charter",
                "line",
                "holder",
                "term",
                "operation",
                "ownership",
            ],
        )?;
    } else {
        return Err(refused("resale paid pointer format"));
    }
    if decimal(field(&row, "id")?, u64::MAX)? != licence {
        return Err(refused("resale paid pointer key"));
    }
    let operation = token(field(&row, "operation")?)?;
    let (route, allocation) = birth_allocation(view, operation)?;
    for n in ["charter", "line"] {
        if field(&row, n)? != field(&allocation, n)? {
            return Err(refused("resale paid birth identity"));
        }
    }
    let current = decimal(field(&row, "term")?, u64::MAX)?;
    person(field(&row, "holder")?)?;
    if format == 1 {
        if field(&row, "holder")? != field(&allocation, "holder")? {
            return Err(refused("resale unproved original holder"));
        }
    } else {
        if view.registry.paid_resale.is_none() {
            return Err(refused("resale paid pointer without protocol"));
        }
        let r = field(&row, "ownership")?;
        let generation = ownership(r)?;
        if generation < 2 {
            return Err(refused("resale format2 birth holder"));
        }
        let h = required(view, &key(HOLDER, holder_key(licence, generation))?)?;
        let p = pair(view, token(field(&h, "acceptance")?)?)?;
        require_holder(view, &p)?;
        if field(field(&p.row, "state")?, "ownership")? != r
            || field(&h, "holder")? != field(&row, "holder")?
            || field(&h, "birth")?
                != &json!({"route":route,"operation":operation,"digest":digest("roll.paid_holder.birth.v1",&allocation)?})
            || decimal(field(field(&h, "term")?, "generation")?, u64::MAX)? > current
        {
            return Err(refused("resale current holder differs"));
        }
    }
    Ok(row)
}
pub(super) fn require_current(view: &View<'_>, p: &Pair) -> Result<Value, ActionStateError> {
    let current = current_pointer(view, p.licence)?;
    let binding = field(&p.row, "binding")?;
    binding_history(view, binding)?;
    captured_holder(view, binding, field(field(&p.offer, "terms")?, "seller")?)?;
    let expected_ownership = if p.generation == 1 {
        None
    } else {
        Some(field(binding, "ownership")?)
    };
    if field(&current, "holder")? != field(field(&p.offer, "terms")?, "seller")?
        || field(&current, "term")? != field(field(binding, "term")?, "generation")?
        || current.get("ownership") != expected_ownership
        || field(&current, "operation")? != field(field(binding, "birth")?, "operation")?
    {
        return Err(refused("resale current pointer differs from acceptance"));
    }
    Ok(current)
}
pub(super) fn next_pointer(current: &Value, p: &Pair) -> Result<Value, ActionStateError> {
    let mut next = current.clone();
    let row = next
        .as_object_mut()
        .ok_or_else(|| refused("resale paid pointer"))?;
    row.insert("format".into(), json!(2));
    row.insert(
        "holder".into(),
        field(field(&p.offer, "terms")?, "buyer")?.clone(),
    );
    row.insert(
        "ownership".into(),
        field(field(&p.row, "state")?, "ownership")?.clone(),
    );
    Ok(next)
}
pub(super) fn outputs(view: &View<'_>, p: &Pair, transition: bool) -> Result<(), ActionStateError> {
    let receipt_key = key(RECEIPT, p.operation.to_hex())?;
    let future = key(HOLDER, holder_key(p.licence, p.generation + 1))?;
    if p.phase == "funded" {
        if value(view, &receipt_key)?.is_some() || value(view, &future)?.is_some() {
            return Err(refused("resale premature terminal rows"));
        }
        require_current(view, p)?;
    } else {
        if required(view, &receipt_key)? != receipt(p)? {
            return Err(refused("resale terminal receipt differs"));
        }
        if p.phase == "settled" {
            require_holder(view, p)?;
        } else if let Some(row) = value(view, &future)?
            && (transition || field(&row, "acceptance")? == &json!(p.operation))
        {
            return Err(refused("resale refund owns holder output"));
        }
    }
    Ok(())
}
