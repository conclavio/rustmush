//! Read-only constant-depth joins for an immutable renewal's then-current
//! holder. Whole resale/renewal audits still prove every chain in both directions.
use super::*;

pub(in crate::action_state::terminal_capacity) fn renewal_holder_generation(
    binding: &Value,
) -> Result<u64, ActionStateError> {
    let format = num(binding, "format")?;
    if matches!(format, 1 | 2) && binding.get("revision_policy").is_some() {
        return Err(refused("Pin renewal cannot carry Follow policy"));
    }
    if matches!(format, 3 | 4) {
        super::super::entitlement::binding_revision(binding)?;
    }
    match format {
        1 | 3 if binding.get("holder_basis").is_none() => Ok(1),
        2 | 4 => {
            let basis = field(binding, "holder_basis")?;
            shape(basis, &["acceptance", "ownership"])?;
            token(field(basis, "acceptance")?)?;
            let generation = ownership(field(basis, "ownership")?)?;
            if generation < 2 {
                return Err(refused("renewal holder basis cannot name birth"));
            }
            Ok(generation)
        }
        _ => Err(refused("renewal holder format or omitted basis differs")),
    }
}

pub(in crate::action_state::terminal_capacity) fn renewal_holder(
    view: &View<'_>,
    binding: &Value,
    market: &Value,
) -> Result<(), ActionStateError> {
    let generation = renewal_holder_generation(binding)?;
    let attempt = field(market, "attempt")?;
    let kind = text(attempt, "kind")?;
    match (generation, kind) {
        (1, "automatic") => shape(attempt, &["kind"])?,
        (1, "buyer_retry") => shape(attempt, &["kind", "previous_operation"])?,
        (2.., "holder_automatic") => shape(attempt, &["kind", "basis"])?,
        (2.., "holder_retry") => shape(attempt, &["kind", "basis", "previous_operation"])?,
        _ => return Err(refused("renewal holder attempt differs")),
    }
    if matches!(kind, "buyer_retry" | "holder_retry")
        && token(field(attempt, "previous_operation")?)? == token(field(market, "operation")?)?
    {
        return Err(refused("renewal self retry"));
    }
    let birth = token(field(binding, "birth_operation")?)?;
    let (_, allocation) = history::birth_allocation(view, birth)?;
    if field(&allocation, "licence")? != field(binding, "licence")? {
        return Err(refused("renewal holder birth licence differs"));
    }
    if generation == 1 {
        // M8c F4: by principal: the binding of an organisation-held licence
        // names the renewing actor as `acting`.
        if !crate::action_state::terminal_capacity::owner_index::same_holder(
            field(binding, "holder")?,
            field(&allocation, "holder")?,
        )? {
            return Err(refused("renewal original holder differs"));
        }
        return Ok(());
    }
    if view.registry.paid_resale.is_none() {
        return Err(refused("renewal transferred holder protocol absent"));
    }
    let basis = field(binding, "holder_basis")?;
    if field(attempt, "basis")? != basis {
        return Err(refused("renewal attempt holder basis differs"));
    }
    let p = pair(view, token(field(basis, "acceptance")?)?)?;
    let holder = history::require_holder(view, &p)?;
    history::outputs(view, &p, false)?;
    let captured = field(&p.row, "binding")?;
    let economic = field(&p.row, "economic")?;
    let consent = field(field(economic, "intent")?, "consent")?;
    let terminal_utc = if text(consent, "mode")? == "immediate" {
        signed(consent, "accepted_at_utc")?
    } else {
        signed(
            field(field(economic, "terminal")?, "decision")?,
            "posted_at_utc",
        )?
    };
    if field(binding, "licence")? != field(&holder, "licence")?
        || num(&holder, "generation")? != generation
        || !crate::action_state::terminal_capacity::owner_index::same_holder(
            field(binding, "holder")?,
            field(&holder, "holder")?,
        )?
        || token(field(field(&holder, "birth")?, "operation")?)? != birth
        || field(field(&p.row, "state")?, "ownership")? != field(basis, "ownership")?
        || field(binding, "charter")? != field(captured, "charter")?
        || field(binding, "line")? != field(captured, "line")?
        || decimal(field(field(&holder, "term")?, "generation")?, u64::MAX)?
            > decimal(field(binding, "predecessor")?, u64::MAX)?
        || signed(market, "created_at_utc")? < terminal_utc
        || token(field(market, "operation")?)? == p.operation
        || token(field(market, "operation")?)? == terminal(&p)?
    {
        return Err(refused("renewal transferred holder evidence differs"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn holder_basis_omission_and_exact_generation_domains_remain_distinct() {
        assert_eq!(renewal_holder_generation(&json!({"format":1})).unwrap(), 1);
        assert!(renewal_holder_generation(&json!({"format":1,"holder_basis":null})).is_err());
        assert!(renewal_holder_generation(&json!({"format":2})).is_err());
        assert!(renewal_holder_generation(&json!({"format":2,"holder_basis":null})).is_err());
        for generation in [2, 65537] {
            let row = json!({"format":2,"holder_basis":{"acceptance":CommitToken::from_bytes([1;16]),
                "ownership":{"generation":generation,"operation":CommitToken::from_bytes([2;16]),"digest":DefinitionDigest::from_bytes([3;32])}}});
            assert_eq!(renewal_holder_generation(&row).unwrap(), generation);
            for bad in [0, 1, 65538] {
                let mut broken = row.clone();
                broken["holder_basis"]["ownership"]["generation"] = json!(bad);
                assert!(renewal_holder_generation(&broken).is_err());
            }
            let mut broken = row.clone();
            broken["holder_basis"]["extra"] = Value::Null;
            assert!(renewal_holder_generation(&broken).is_err());
        }
    }
}

pub(in crate::action_state::terminal_capacity) fn renewal_current_holder(
    view: &View<'_>,
    current: &Value,
    binding: &Value,
    pending: bool,
) -> Result<(), ActionStateError> {
    let birth = token(field(binding, "birth_operation")?)?;
    let (_, allocation) = history::birth_allocation(view, birth)?;
    // The original holder remains the original holder even when this immutable
    // renewal was issued to a later buyer. Current pointer proof is independent.
    validate_current_holder(view, current, field(&allocation, "holder")?, birth)?;
    if pending {
        let generation = renewal_holder_generation(binding)?;
        let expected = if generation == 1 {
            None
        } else {
            Some(field(field(binding, "holder_basis")?, "ownership")?)
        };
        if !crate::action_state::terminal_capacity::owner_index::same_holder(
            field(current, "holder")?,
            field(binding, "holder")?,
        )? || num(current, "format")? != if generation == 1 { 1 } else { 2 }
            || current.get("ownership") != expected
        {
            return Err(refused("renewal pending holder changed"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod follow_tests {
    use super::*;
    #[test]
    fn explicit_follow_binding_rejects_null_crossed_line_and_pin_aliases() {
        let valid = json!({"format":3,"line":"1","revision_policy":{"kind":"follow","line":"1"}});
        assert_eq!(renewal_holder_generation(&valid).unwrap(), 1);
        for (field, value) in [
            ("revision_policy", Value::Null),
            ("revision_policy", json!({"kind":"follow","line":"2"})),
            (
                "revision_policy",
                json!({"kind":"pin","filed":"1","revision":{}}),
            ),
            ("format", json!(1)),
            ("holder_basis", Value::Null),
        ] {
            let mut broken = valid.clone();
            broken[field] = value;
            assert!(renewal_holder_generation(&broken).is_err());
        }
        let mut moved = valid;
        moved["format"] = json!(4);
        assert!(renewal_holder_generation(&moved).is_err());
        moved["holder_basis"] = json!({"acceptance":CommitToken::from_bytes([1;16]),"ownership":{"generation":2,"operation":CommitToken::from_bytes([2;16]),"digest":DefinitionDigest::from_bytes([3;32])}});
        assert_eq!(renewal_holder_generation(&moved).unwrap(), 2);
    }
}
