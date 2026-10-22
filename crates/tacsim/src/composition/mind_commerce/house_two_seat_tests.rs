//! Real registration only: this does not manufacture any paid term or seat.
use super::*;
use mush_roll::commerce::house_purchase::HouseProtocol;

fn selected() -> Result<MindCommerce, ModError> {
    prepare(CommerceMode::Active)?
        .with_house_filings()?
        .with_house_minds(house_policy()?)?
        .with_two_seat_house_pins(house_policy()?)
}

#[test]
fn house_two_seat_registration_is_explicit_and_keeps_both_full_protocols() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_two_seat_house_pins(house_policy()?)
            .is_err()
    );
    check!(
        prepare(CommerceMode::Active)?
            .with_two_seat_house_pins(house_policy()?)
            .is_err()
    );
    let old = prepare(CommerceMode::Active)?
        .with_house_filings()?
        .with_house_minds(house_policy()?)?;
    let old_config = old.state.house_purchases.as_ref().ok_or("House config")?;
    let old_refs = old_config.owner_actions(HouseProtocol::HouseV1)?;
    let old_quote = old_config.quote_action(HouseProtocol::HouseV1)?;
    let old_engine = house_engine(&old, true)?;
    let definitions = old_engine.registry.definitions().ok_or("definitions")?;
    check!(
        !definitions
            .iter()
            .any(|d| d.reference().key().owner.as_str() == "market"
                && d.reference().key().id.as_str() == "purchase_house_mind"
                && d.reference().version() == 2)
    );
    let both = selected()?;
    let house = both.state.house_purchases.as_ref().ok_or("House config")?;
    check_eq!(house.owner_actions(HouseProtocol::HouseV1)?, old_refs);
    check_eq!(house.quote_action(HouseProtocol::HouseV1)?, old_quote);
    let v2 = house.owner_actions(HouseProtocol::HouseV2)?;
    let v2_quote = house.quote_action(HouseProtocol::HouseV2)?;
    let actual = house_engine(&both, true)?;
    let definitions = actual.registry.definitions().ok_or("definitions")?;
    for reference in old_refs.iter().chain(&v2).chain([&old_quote, &v2_quote]) {
        check_eq!(
            definitions
                .iter()
                .filter(|d| d.reference() == reference)
                .count(),
            1
        );
    }
    for participant in [
        house.participants().economy(),
        house.participants().retention(),
    ] {
        check_eq!(
            definitions
                .iter()
                .filter(|d| d.reference() == participant)
                .count(),
            1
        );
    }
    // A failed composition may install private writer handles before refusing;
    // each independent boot therefore gets its own checked configuration.
    check!(house_engine(&selected()?, false).is_err());
    check!(both.with_two_seat_house_pins(house_policy()?).is_err());
    Ok(())
}

#[test]
fn house_two_seat_renewals_capture_both_policies_before_registration() -> TestResult {
    let configured = selected()?
        .with_house_mind_renewals(house_renewal_policy()?)?
        .with_house_mind_renewal_service()?;
    let actual = house_engine(&configured, true)?;
    let definitions = actual.registry.definitions().ok_or("definitions")?;
    let house = configured
        .state
        .house_purchases
        .as_ref()
        .ok_or("House config")?;
    for protocol in [HouseProtocol::HouseV1, HouseProtocol::HouseV2] {
        check!(
            definitions
                .get_exact(&house.policy_ref_for_protocol(protocol)?)
                .is_ok()
        );
        for reference in house.owner_actions(protocol)? {
            check!(definitions.get_exact(&reference).is_ok());
        }
    }
    for reference in configured
        .state
        .house_renewals
        .as_ref()
        .ok_or("renewals")?
        .actions()?
    {
        check!(definitions.get_exact(&reference).is_ok());
    }
    check!(has(
        &actual,
        "market",
        "action",
        "close_house_mind_renewal_service"
    ));
    check!(
        prepare(CommerceMode::Active)?
            .with_house_filings()?
            .with_house_minds(house_policy()?)?
            .with_house_mind_renewals(house_renewal_policy()?)?
            .with_two_seat_house_pins(house_policy()?)
            .is_err()
    );
    Ok(())
}
