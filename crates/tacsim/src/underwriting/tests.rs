use super::*;
use mush_core::{ClockRef, ClockStamp};
use mush_economy::{Amount, InstrumentId};

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn money(v: i64) -> Amount {
    Amount::from_minor(v)
}
fn terms() -> Result<CoverageTermsV1, Box<dyn std::error::Error>> {
    Ok(CoverageTermsV1::launch(
        InstrumentId::new("tacsim.cbill").ok_or("instrument")?,
        money(2),
        money(i64::MAX),
        7 * 86_400,
    )?)
}
fn ic(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}

#[test]
fn underwriting_launch_prices_keep_premium_and_reserved_liability_distinct() -> TestResult {
    let terms = terms()?;
    let illustration = terms.illustrate(money(10_000))?;
    let ordinary = illustration.ordinary();
    let qualified = illustration.with_qualified_safe_conduct();
    check_eq!(illustration.insured_value(), money(10_000));
    check_eq!(ordinary.premium(), money(100));
    check_eq!(ordinary.discount(), money(0));
    check_eq!(qualified.base_premium(), money(100));
    check_eq!(qualified.discount(), money(10));
    check_eq!(qualified.premium(), money(90));
    check_eq!(qualified.maximum_payout(), money(8_000));
    check_eq!(ordinary.maximum_payout(), qualified.maximum_payout());
    Ok(())
}

#[test]
fn underwriting_rounding_is_explicit_and_conserves_minor_units() -> TestResult {
    let terms = terms()?;
    for (value, base, discount, payout) in [
        (2, 1, 0, 1),
        (101, 2, 0, 80),
        (1001, 11, 1, 800),
        (10_099, 101, 10, 8079),
    ] {
        let illustration = terms.illustrate(money(value))?;
        let qualified = illustration.with_qualified_safe_conduct();
        check_eq!(qualified.base_premium(), money(base));
        check_eq!(qualified.discount(), money(discount));
        check_eq!(qualified.maximum_payout(), money(payout));
        check_eq!(
            qualified.premium().checked_add(qualified.discount()),
            Some(qualified.base_premium())
        );
        check_eq!(illustration.ordinary().premium(), qualified.base_premium());
    }
    let maximal = terms
        .illustrate(money(i64::MAX))?
        .with_qualified_safe_conduct();
    check_eq!(maximal.base_premium(), money(92_233_720_368_547_759));
    check_eq!(maximal.discount(), money(9_223_372_036_854_775));
    check_eq!(maximal.maximum_payout(), money(7_378_697_629_483_820_645));
    check_eq!(terms.illustrate(money(1)), Err(CoverageError::Value));
    check_eq!(terms.illustrate(money(-1)), Err(CoverageError::Value));
    Ok(())
}

#[test]
fn underwriting_terms_codec_refuses_invalid_mutations_and_preserves_exact_digest() -> TestResult {
    let terms = terms()?;
    let value = serde_json::to_value(&terms)?;
    let encoded = serde_json::to_vec(&terms)?;
    check_eq!(CoverageTermsV1::decode(&encoded)?, terms);
    for (field, replacement) in [
        ("format", serde_json::json!(2)),
        ("minimum_value", serde_json::json!(0)),
        ("maximum_value", serde_json::json!(1)),
        ("premium_permille", serde_json::json!(0)),
        ("discount_permille", serde_json::json!(1000)),
        ("payout_permille", serde_json::json!(1001)),
        ("term_seconds", serde_json::json!(0)),
        ("claim_delay_seconds", serde_json::json!(-1)),
        (
            "term_seconds",
            serde_json::json!(CoverageTermsV1::MAX_TERM_SECONDS + 1),
        ),
        (
            "claim_delay_seconds",
            serde_json::json!(CoverageTermsV1::MAX_CLAIM_DELAY_SECONDS + 1),
        ),
        ("unknown", serde_json::json!(true)),
    ] {
        let mut changed = value.clone();
        changed[field] = replacement;
        check!(serde_json::from_value::<CoverageTermsV1>(changed).is_err());
    }
    let mut changed = value.clone();
    changed["premium_permille"] = serde_json::json!(11);
    let repriced: CoverageTermsV1 = serde_json::from_value(changed)?;
    check_ne!(repriced.digest()?, terms.digest()?);
    let mut changed = value;
    changed["instrument"] = serde_json::json!("another.currency");
    check_ne!(
        serde_json::from_value::<CoverageTermsV1>(changed)?.digest()?,
        terms.digest()?
    );
    check_eq!(
        CoverageTermsV1::decode(&vec![b' '; CoverageTermsV1::MAX_BYTES + 1]),
        Err(CoverageError::Bound)
    );
    check_eq!(CoverageTermsV1::decode(&[]), Err(CoverageError::Bound));
    Ok(())
}

#[test]
fn underwriting_exact_term_and_delay_boundaries_preserve_a_maturing_loss() -> TestResult {
    let terms = terms()?;
    let window = CoverageWindow::new(ic(100), &terms)?;
    check_eq!(window.start(), &ic(100));
    check_eq!(window.end(), &ic(604_900));
    check_eq!(
        window.loss_payment_due(&ic(99)),
        Err(CoverageError::OutsideTerm)
    );
    check_eq!(window.loss_payment_due(&ic(100))?, ic(86_500));
    check_eq!(
        window.loss_payment_due(&ic(604_900)),
        Err(CoverageError::OutsideTerm)
    );
    let loss = ic(604_899);
    check_eq!(window.loss_payment_due(&loss)?, ic(691_299));
    check!(!window.delay_elapsed(&loss, &ic(691_298))?);
    check!(window.delay_elapsed(&loss, &ic(691_299))?);
    check!(!window.delay_elapsed(&loss, &ic(0))?);
    Ok(())
}

#[test]
fn underwriting_never_coerces_clock_generation_or_overflows_deadlines() -> TestResult {
    let terms = terms()?;
    for clock in [ClockRef::Utc, ClockRef::WorldIc { generation: 0 }] {
        check_eq!(
            CoverageWindow::new(ClockStamp { clock, at: 100 }, &terms),
            Err(CoverageError::Clock)
        );
    }
    let window = CoverageWindow::new(ic(100), &terms)?;
    let foreign = ClockStamp {
        clock: ClockRef::WorldIc { generation: 2 },
        at: 100,
    };
    check_eq!(window.loss_payment_due(&foreign), Err(CoverageError::Clock));
    check_eq!(
        window.delay_elapsed(&ic(100), &foreign),
        Err(CoverageError::Clock)
    );
    check_eq!(
        CoverageWindow::new(ic(i64::MAX), &terms),
        Err(CoverageError::Arithmetic)
    );
    let late = CoverageWindow::new(ic(i64::MAX - terms.term_seconds()), &terms)?;
    check_eq!(
        late.loss_payment_due(&ic(i64::MAX - 1)),
        Err(CoverageError::Arithmetic)
    );
    Ok(())
}
