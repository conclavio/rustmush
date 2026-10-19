#![cfg(test)]

use mush_economy::{MAX_SHARES, apply_outcome};

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn cut(account: u64, permille: u16) -> GrossCut {
    GrossCut {
        to: AccountId(account),
        permille,
    }
}

fn rows(value: &ExactProceeds) -> Result<Vec<(u64, i64)>, Box<dyn std::error::Error>> {
    value
        .shares()
        .iter()
        .map(|row| match row.share {
            Share::Exact(amount) => Ok((row.to.0, amount.minor())),
            Share::Permille(_) => Err("proceeds must emit Exact shares".into()),
        })
        .collect()
}

#[test]
fn gross_cuts_and_rounding_residue_feed_real_e4_exact_outcome() -> TestResult {
    let result = calculate(
        Amount::from_minor(1_019),
        cut(9, 50),
        Some(cut(2, 50)),
        &[cut(8, 125), cut(3, 100), cut(7, 75)],
        AccountId(1),
    )?;
    assert_eq!(
        rows(&result)?,
        vec![(1, 615), (2, 50), (3, 101), (7, 76), (8, 127), (9, 50)]
    );
    let e4 = apply_outcome(result.gross(), result.shares())?;
    assert_eq!(
        e4.iter().map(|(_, value)| value.widen()).sum::<i128>(),
        1_019
    );
    assert_eq!(e4.len(), MAX_EXACT_PAYEES);
    assert!(e4.len() <= MAX_SHARES);
    Ok(())
}

#[test]
fn one_minor_unit_and_zero_gross_have_no_zero_payout_rows() -> TestResult {
    let parents = [cut(3, 100), cut(4, 100), cut(5, 100)];
    let one = calculate(
        Amount::from_minor(1),
        cut(1, 50),
        Some(cut(2, 50)),
        &parents,
        AccountId(6),
    )?;
    assert_eq!(rows(&one)?, vec![(6, 1)]);
    let zero = calculate(
        Amount::ZERO,
        cut(1, 50),
        Some(cut(2, 50)),
        &parents,
        AccountId(6),
    )?;
    assert!(zero.shares().is_empty());
    assert!(apply_outcome(zero.gross(), zero.shares())?.is_empty());
    Ok(())
}

#[test]
fn repeated_recipients_merge_only_after_each_declared_cut_is_floored() -> TestResult {
    // floor(19*50/1000) + floor(19*50/1000) is zero; flooring the
    // prematurely merged 100-permille royalty would incorrectly pay one.
    let result = calculate(
        Amount::from_minor(19),
        cut(0, 50),
        None,
        &[cut(7, 50), cut(7, 50)],
        AccountId(3),
    )?;
    assert_eq!(rows(&result)?, vec![(3, 19)]);
    let same = calculate(
        Amount::from_minor(1_019),
        cut(7, 50),
        Some(cut(7, 50)),
        &[cut(7, 100), cut(7, 100), cut(7, 100)],
        AccountId(7),
    )?;
    assert_eq!(rows(&same)?, vec![(7, 1_019)]);
    Ok(())
}

#[test]
fn real_economy_account_zero_and_large_ids_keep_their_identity() -> TestResult {
    let result = calculate(
        Amount::from_minor(100),
        cut(0, 50),
        None,
        &[],
        AccountId(u64::MAX),
    )?;
    assert_eq!(rows(&result)?, vec![(0, 5), (u64::MAX, 95)]);
    assert_eq!(apply_outcome(result.gross(), result.shares())?.len(), 2);
    Ok(())
}

#[test]
fn parent_order_does_not_change_the_canonical_merged_split() -> TestResult {
    let a = calculate(
        Amount::from_minor(123_457),
        cut(0, 50),
        Some(cut(4, 50)),
        &[cut(7, 25), cut(6, 175), cut(7, 100)],
        AccountId(9),
    )?;
    let b = calculate(
        Amount::from_minor(123_457),
        cut(0, 50),
        Some(cut(4, 50)),
        &[cut(7, 100), cut(7, 25), cut(6, 175)],
        AccountId(9),
    )?;
    assert_eq!(a, b);
    Ok(())
}

#[test]
fn supplied_primary_or_secondary_remainder_is_never_inferred_or_redirected() -> TestResult {
    let primary = calculate(
        Amount::from_minor(100),
        cut(0, 50),
        None,
        &[cut(3, 100)],
        AccountId(7),
    )?;
    let secondary = calculate(
        Amount::from_minor(100),
        cut(0, 50),
        None,
        &[cut(3, 100)],
        AccountId(8),
    )?;
    assert_eq!(rows(&primary)?, vec![(0, 5), (3, 10), (7, 85)]);
    assert_eq!(rows(&secondary)?, vec![(0, 5), (3, 10), (8, 85)]);
    Ok(())
}

#[test]
fn invalid_rates_depth_and_negative_gross_refuse_even_with_zero_amount() {
    assert_eq!(
        calculate(Amount::from_minor(-1), cut(0, 50), None, &[], AccountId(1)),
        Err(ProceedsError::NegativeGross)
    );
    assert_eq!(
        calculate(
            Amount::from_minor(i64::MIN),
            cut(0, 50),
            None,
            &[],
            AccountId(1)
        ),
        Err(ProceedsError::NegativeGross)
    );
    assert_eq!(
        calculate(Amount::ZERO, cut(0, 1_001), None, &[], AccountId(1)),
        Err(ProceedsError::Rate)
    );
    assert_eq!(
        calculate(
            Amount::ZERO,
            cut(0, 50),
            Some(cut(2, u16::MAX)),
            &[],
            AccountId(1)
        ),
        Err(ProceedsError::Rate)
    );
    assert_eq!(
        calculate(Amount::ZERO, cut(0, 50), None, &[cut(2, 301)], AccountId(1)),
        Err(ProceedsError::ParentRate)
    );
    assert_eq!(
        calculate(
            Amount::ZERO,
            cut(0, 50),
            None,
            &[cut(2, 0); 4],
            AccountId(1)
        ),
        Err(ProceedsError::ParentCount)
    );
    assert_eq!(
        calculate(
            Amount::ZERO,
            cut(0, 800),
            Some(cut(2, 201)),
            &[],
            AccountId(1)
        ),
        Err(ProceedsError::ExcessiveCuts)
    );
}

#[test]
fn maximum_amount_uses_widened_checked_math_and_conserves_every_minor_unit() -> TestResult {
    let gross = Amount::from_minor(i64::MAX);
    let result = calculate(
        gross,
        cut(0, 50),
        Some(cut(1, 50)),
        &[cut(2, 100), cut(3, 100), cut(4, 100)],
        AccountId(5),
    )?;
    let paid = apply_outcome(gross, result.shares())?;
    assert_eq!(
        paid.iter().map(|(_, amount)| amount.widen()).sum::<i128>(),
        gross.widen()
    );
    assert!(paid.iter().all(|(_, amount)| amount.minor() > 0));
    let all = calculate(gross, cut(0, 700), None, &[cut(1, 300)], AccountId(2))?;
    assert_eq!(
        apply_outcome(gross, all.shares())?
            .iter()
            .map(|(_, a)| a.widen())
            .sum::<i128>(),
        gross.widen()
    );
    Ok(())
}
