//! Pure gross-based proceeds arithmetic (17f), without payment authority.
//!
//! Inputs are already-frozen rates and recipient accounts. This module does not
//! find owners, establish ancestry, read a World, create contracts or post money.
//! E4 must still validate every actual account/instrument/status, consent and
//! authority before preparing the composed transaction. Account zero is valid
//! in E4 and is not treated as a sentinel here. No source or holder lookup occurs.

use std::collections::BTreeMap;

use mush_economy::{AccountId, Amount, PERMILLE, PayoutShare, Share};

/// At most one royalty cut per ancestor, with a three-link ancestry bound.
pub const MAX_PARENT_CUTS: usize = 3;
/// Aggregate royalty ceiling across the complete parent chain.
pub const MAX_PARENT_PERMILLE: u32 = 300;
/// Registry, venue, three ancestors and one remainder recipient.
pub const MAX_EXACT_PAYEES: usize = 6;

/// One independently floored cut of the original gross minor-unit amount.
/// A repeated account still has each declared cut floored before aggregation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrossCut {
    pub to: AccountId,
    pub permille: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProceedsError {
    #[error("gross proceeds cannot be negative")]
    NegativeGross,
    #[error("too many parent royalty cuts")]
    ParentCount,
    #[error("a gross cut exceeds 1000 permille")]
    Rate,
    #[error("aggregate parent royalties exceed 300 permille")]
    ParentRate,
    #[error("aggregate gross cuts exceed the gross amount")]
    ExcessiveCuts,
    #[error("proceeds arithmetic overflow")]
    Overflow,
}

/// Calculated Exact shares, sorted by account, with no zero rows or implicit remainder.
/// This is arithmetic output, not a prepared posting, receipt or capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactProceeds {
    gross: Amount,
    shares: Vec<PayoutShare>,
}

impl ExactProceeds {
    #[must_use]
    pub const fn gross(&self) -> Amount {
        self.gross
    }

    #[must_use]
    pub fn shares(&self) -> &[PayoutShare] {
        &self.shares
    }

    #[must_use]
    pub fn into_shares(self) -> Vec<PayoutShare> {
        self.shares
    }
}

/// Compute the full deterministic split in one instrument's minor units.
///
/// All cuts use the original gross, including Registry and venue. Their floors
/// are calculated separately, then repeated accounts are merged. The one final
/// recipient receives the entire unassigned amount, including rounding residue.
/// Which account is that recipient (primary owner versus secondary seller) is
/// supplied by the later authorized, frozen commercial terms; it is not inferred.
///
/// Zero gross returns an empty arithmetic split but does not authorize a sale;
/// free owner attestation remains a separate operation. Validation still runs
/// for zero gross, so it cannot hide excessive rates or an over-depth lineage.
pub fn calculate(
    gross: Amount,
    registry: GrossCut,
    venue: Option<GrossCut>,
    parents: &[GrossCut],
    remainder_to: AccountId,
) -> Result<ExactProceeds, ProceedsError> {
    if gross.is_negative() {
        return Err(ProceedsError::NegativeGross);
    }
    if parents.len() > MAX_PARENT_CUTS {
        return Err(ProceedsError::ParentCount);
    }
    let cuts = || {
        std::iter::once(registry)
            .chain(venue)
            .chain(parents.iter().copied())
    };
    let mut total_rate = 0u32;
    for cut in cuts() {
        if u32::from(cut.permille) > PERMILLE {
            return Err(ProceedsError::Rate);
        }
        total_rate = total_rate
            .checked_add(u32::from(cut.permille))
            .ok_or(ProceedsError::Overflow)?;
    }
    let parent_rate = parents.iter().try_fold(0u32, |sum, cut| {
        sum.checked_add(u32::from(cut.permille))
            .ok_or(ProceedsError::Overflow)
    })?;
    if parent_rate > MAX_PARENT_PERMILLE {
        return Err(ProceedsError::ParentRate);
    }
    if total_rate > PERMILLE {
        return Err(ProceedsError::ExcessiveCuts);
    }

    let mut amounts = BTreeMap::new();
    let mut assigned = Amount::ZERO;
    for cut in cuts() {
        let numerator = gross
            .widen()
            .checked_mul(i128::from(cut.permille))
            .ok_or(ProceedsError::Overflow)?;
        let amount = Amount::from_widened(numerator / i128::from(PERMILLE))
            .ok_or(ProceedsError::Overflow)?;
        assigned = assigned
            .checked_add(amount)
            .ok_or(ProceedsError::Overflow)?;
        merge(&mut amounts, cut.to, amount)?;
    }
    let remainder = gross.checked_sub(assigned).ok_or(ProceedsError::Overflow)?;
    merge(&mut amounts, remainder_to, remainder)?;
    let shares = amounts
        .into_iter()
        .map(|(to, amount)| PayoutShare {
            to,
            share: Share::Exact(amount),
        })
        .collect();
    Ok(ExactProceeds { gross, shares })
}

fn merge(
    amounts: &mut BTreeMap<AccountId, Amount>,
    to: AccountId,
    amount: Amount,
) -> Result<(), ProceedsError> {
    if !amount.is_zero() {
        let prior = amounts.get(&to).copied().unwrap_or(Amount::ZERO);
        let merged = prior.checked_add(amount).ok_or(ProceedsError::Overflow)?;
        amounts.insert(to, merged);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
