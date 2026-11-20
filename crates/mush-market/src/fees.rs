//! **Doc 48 cut EX.4 — the exchange's fees** (Q-48-8, provisional; Q-48-54's "charged once a
//! listing").
//!
//! A person's listing pays three things, all game-neutral here:
//!
//! - **A listing fee** of [`LISTING_FEE_PERMILLE`] of what it lists (price × quantity, or the
//!   whole price of a titled object), at least the venue's floor, paid to the venue's operator
//!   **once**, at posting (`charge_listing_fee`). A lot hold that renews is not a new listing.
//! - **A sale cut** of [`SALE_CUT_PERMILLE`] of a sale, to the venue's operator, and **a levy** of
//!   [`LEVY_PERMILLE`] to the venue's sink, each an exact payout row of the sale contract beside
//!   the seller's, which is the price less every other row ([`sale_payouts`]). The standing
//!   acceptance holds a sale to exactly these rows.
//!
//! Who the operator and the sink are is the game's answer ([`VenueFees`], composition-frozen
//! like every seam here). **With none installed, or a venue it does not answer for, nothing is
//! charged**: every listing and sale is exactly as before EX.4. A vendor's (a house's) listings
//! pay none of these, nor does a personal listing the game says its house posts
//! ([`VenueFees::house_lists`]: TacSim's buy-back desk).

use mush_core::{Ctx, World};
use mush_economy::{AccountId, Amount, PayoutShare, Share};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::api::MarketError;
use crate::ids::{ListingId, MarketId};

/// The listing fee, per mille of what is listed (Q-48-8: 1%, provisional).
pub const LISTING_FEE_PERMILLE: i64 = 10;
/// The sale cut to the venue's operator, per mille of a sale (Q-48-8: 3%, provisional).
pub const SALE_CUT_PERMILLE: i64 = 30;
/// The levy to the venue's sink, per mille of a sale (Q-48-8: 1%, provisional).
pub const LEVY_PERMILLE: i64 = 10;

/// A listing that cannot pay its fee: `{fee}`.
pub const FEE_SHORT: &str =
    "Listing it costs a fee of {fee}, which your account cannot cover; nothing was listed.";
/// **Who a venue's fees are paid to** (doc 48 cut EX.4): the operator's account (the listing fee
/// and the sale cut), the sink's (the levy; a system account the money census counts as a
/// sink), and the least listing fee, in the venue's currency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VenueFeeAccounts {
    /// The venue's operator: the listing fee and the sale cut.
    pub operator: AccountId,
    /// The operator's name, for the line a seller reads.
    pub operator_name: String,
    /// The levy's sink.
    pub sink: AccountId,
    /// The least listing fee.
    pub listing_floor: Amount,
}

/// **The game's fee seam** (doc 48 cut EX.4): who a venue's fees are paid to. Composition-frozen
/// ([`crate::MarketModule::with_venue_fees`]); unwired, or `None` for a venue, is no fees.
pub trait VenueFees: Send + Sync + std::fmt::Debug {
    /// The accounts `venue`'s fees go to, in its own currency; `None` charges nothing there.
    fn accounts(&self, w: &World, venue: &MarketId) -> Option<VenueFeeAccounts>;

    /// **Whether `payer` lists at `venue` as the venue's own house**, though through a personal
    /// listing (a house's listings pay none of these): its listings then pay no listing fee.
    /// Unanswered, nobody does.
    fn house_lists(&self, _w: &World, _venue: &MarketId, _payer: &PrincipalId) -> bool {
        false
    }
}

/// The accounts `venue` charges, when the composition installed a seam that answers for it.
#[must_use]
pub fn venue_accounts(w: &World, venue: &MarketId) -> Option<VenueFeeAccounts> {
    crate::module::venue_fees(w)?.accounts(w, venue)
}

/// **The accounts `payer`'s listing fee at `venue` is paid to**: `None` when the venue charges
/// nothing there or `payer` lists as its house ([`VenueFees::house_lists`]).
#[must_use]
pub fn listing_fee_accounts(
    w: &World,
    venue: &MarketId,
    payer: &PrincipalId,
) -> Option<VenueFeeAccounts> {
    let seam = crate::module::venue_fees(w)?;
    if seam.house_lists(w, venue, payer) {
        return None;
    }
    seam.accounts(w, venue)
}

/// `permille` of `amount`, floored, in minor units; zero on overflow or a non-positive amount.
#[must_use]
pub fn per_mille(amount: Amount, permille: i64) -> Amount {
    let share = i128::from(amount.minor().max(0)) * i128::from(permille) / 1_000;
    Amount::from_minor(i64::try_from(share).unwrap_or(0))
}

/// **The listing fee on `listed`** (what a listing lists) at a venue whose floor is `floor`.
#[must_use]
pub fn listing_fee(listed: Amount, floor: Amount) -> Amount {
    per_mille(listed, LISTING_FEE_PERMILLE).max(floor)
}

/// **Pay a listing's fee, once** (doc 48 cut EX.4): `fee` from `from` (the seller's own account)
/// to the venue's operator, under the actor's own authority and the key
/// `market.listing_fee/<listing>`, so a retried posting cannot charge twice. Call it as the
/// posting's last fallible step. `Ok(None)` when the venue charges nothing.
///
/// # Errors
///
/// [`FEE_SHORT`] when the account cannot cover it, and the post's own refusal otherwise.
pub(crate) fn charge_listing_fee(
    ctx: &mut Ctx<'_>,
    payer: &PrincipalId,
    from: AccountId,
    venue: &MarketId,
    listing: ListingId,
    listed: Amount,
) -> Result<Option<(Amount, String)>, MarketError> {
    let Some(accounts) = listing_fee_accounts(ctx.w, venue, payer) else {
        return Ok(None);
    };
    let fee = listing_fee(listed, accounts.listing_floor);
    if fee.minor() <= 0 {
        return Ok(None);
    }
    let rendered = render(ctx.w, from, fee);
    let mut plan = mush_economy::EconomyPlan::new(
        mush_economy::IdempotencyKey::new(
            SmolStr::new_static("market.listing_fee"),
            SmolStr::new(listing.0.to_string()),
        ),
        payer.clone(),
        mush_economy::EconomicCause::Command {
            serial: ctx.w.next_correlation_id(),
        },
        fee_lines(from, accounts.operator, fee),
    );
    plan.description = format!("market listing #{listing} fee");
    match mush_economy::post(ctx, &plan) {
        Ok(mush_economy::PostOutcome::Posted(_) | mush_economy::PostOutcome::AlreadyPosted(_)) => {
            Ok(Some((fee, accounts.operator_name)))
        }
        Ok(mush_economy::PostOutcome::NeedsApproval(_)) => {
            Err(MarketError::bad(FEE_SHORT.replace("{fee}", &rendered)))
        }
        Err(
            mush_economy::EconomyError::InsufficientAvailable(_)
            | mush_economy::EconomyError::CreditViolation(_),
        ) => Err(MarketError::bad(FEE_SHORT.replace("{fee}", &rendered))),
        Err(error) => {
            tracing::info!(listing = listing.0, %error, "market: a listing fee was refused");
            Err(MarketError::bad(FEE_SHORT.replace("{fee}", &rendered)))
        }
    }
}

/// `amount` in `account`'s currency.
fn render(w: &World, account: AccountId, amount: Amount) -> String {
    let instrument = w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().account(account).map(|a| a.instrument.clone()));
    crate::commands::money(w, instrument.as_ref(), amount)
}

/// **`amount` rendered in `account`'s currency**, for a fee line.
#[must_use]
pub fn rendered(w: &World, account: AccountId, amount: Amount) -> String {
    render(w, account, amount)
}

/// The two lines that move `fee` from `from` to `to`.
pub(crate) fn fee_lines(
    from: AccountId,
    to: AccountId,
    fee: Amount,
) -> Vec<mush_economy::RequestedLine> {
    vec![
        mush_economy::RequestedLine {
            account: from,
            delta: Amount::from_minor(fee.minor().saturating_neg()),
            memo: "listing fee".into(),
        },
        mush_economy::RequestedLine {
            account: to,
            delta: fee,
            memo: "listing fee".into(),
        },
    ]
}

/// **The fee rows of one personal sale** (doc 48 cut EX.4): the operator's cut and the sink's
/// levy on `gross`, each only when it is more than nothing; none at a venue that charges none.
#[must_use]
pub fn sale_fee_rows(w: &World, venue: &MarketId, gross: Amount) -> Vec<(AccountId, Amount)> {
    let Some(accounts) = venue_accounts(w, venue) else {
        return Vec::new();
    };
    [
        (accounts.operator, per_mille(gross, SALE_CUT_PERMILLE)),
        (accounts.sink, per_mille(gross, LEVY_PERMILLE)),
    ]
    .into_iter()
    .filter(|(_, amount)| amount.minor() > 0)
    .collect()
}

/// **A personal sale's payouts** (doc 48 cut EX.4): `seller` is paid `gross` less every other
/// row (the venue's cut and levy, then `others`, a proceeds policy's cuts), each an exact
/// payout; `None` when the rows would come to more than `gross`. The builder writes these and
/// the standing acceptance requires exactly these, in this order.
#[must_use]
pub fn sale_payouts(
    w: &World,
    venue: &MarketId,
    seller: AccountId,
    gross: Amount,
    others: &[(AccountId, Amount)],
) -> Option<Vec<PayoutShare>> {
    let mut rows = sale_fee_rows(w, venue, gross);
    rows.extend(others.iter().copied().filter(|(_, a)| a.minor() > 0));
    let taken = rows
        .iter()
        .try_fold(0_i64, |sum, (_, a)| sum.checked_add(a.minor()))?;
    let net = gross.minor().checked_sub(taken).filter(|n| *n >= 0)?;
    let mut out = vec![PayoutShare {
        to: seller,
        share: Share::Exact(Amount::from_minor(net)),
    }];
    out.extend(rows.into_iter().map(|(to, amount)| PayoutShare {
        to,
        share: Share::Exact(amount),
    }));
    Some(out)
}

/// **The venue's rows on a sale into a person's bid** (doc 48e, exchange G4): its cut and levy
/// ([`sale_fee_rows`]), none when the bidder bids as the venue's own house
/// ([`VenueFees::house_lists`], TacSim's buy-back desk), as a house's listings pay none.
#[must_use]
pub fn bid_fee_rows(
    w: &World,
    listing: &crate::Listing,
    gross: Amount,
) -> Vec<(AccountId, Amount)> {
    let house = crate::module::venue_fees(w)
        .is_some_and(|seam| seam.house_lists(w, &listing.market, &listing.seller));
    if house {
        return Vec::new();
    }
    sale_fee_rows(w, &listing.market, gross)
}

/// **The payouts of a sale into a bid** (doc 48e, exchange G4 and G26): the seller's account
/// `gross` less every other row (when that is more than nothing), then `fees` (the venue's cut
/// and levy, on a person's bid only) and the proceeds policy's `cuts`, each an exact payout.
/// `None` when the rows come to more than `gross`. The sale's builder writes these, and a
/// personal bidder's standing acceptance requires exactly these.
#[must_use]
pub fn bid_sale_payouts(
    seller: AccountId,
    gross: Amount,
    fees: &[(AccountId, Amount)],
    cuts: &[crate::record::ProceedsCut],
) -> Option<Vec<PayoutShare>> {
    let rows: Vec<(AccountId, Amount)> = fees
        .iter()
        .copied()
        .chain(cuts.iter().map(|c| (c.to, c.amount)))
        .collect();
    let taken = rows
        .iter()
        .try_fold(0_i64, |sum, (_, a)| sum.checked_add(a.minor()))?;
    let net = gross.minor().checked_sub(taken).filter(|n| *n >= 0)?;
    let mut out = Vec::with_capacity(rows.len().saturating_add(1));
    if net > 0 {
        out.push(PayoutShare {
            to: seller,
            share: Share::Exact(Amount::from_minor(net)),
        });
    }
    out.extend(rows.into_iter().map(|(to, amount)| PayoutShare {
        to,
        share: Share::Exact(amount),
    }));
    Some(out)
}

/// **A proceeds policy's cuts on a person's lot ask** (doc 48 cut EX.4; Q-48-56, doc 48b G-7):
/// the policy (TacSim's Co-op share) is asked about an ask from a person's lot exactly as about
/// a sale into a bid, reading `w` only, so the builder and the standing acceptance ask the same
/// question and get the same rows. None for an object, a vendor, or no policy.
///
/// # Errors
///
/// The policy's own refusal; a split the market cannot settle.
pub fn ask_policy_cuts(
    w: &World,
    listing: &crate::Listing,
    buyer: &PrincipalId,
    quantity: u64,
    price: Amount,
) -> Result<Vec<(AccountId, Amount)>, MarketError> {
    Ok(ask_policy_proceeds(w, listing, buyer, quantity, price)?
        .map(|p| p.cuts.into_iter().map(|c| (c.to, c.amount)).collect())
        .unwrap_or_default())
}

/// **The proceeds policy's whole answer on a person's lot ask** ([`ask_policy_cuts`]' rows with
/// their labels and the basis they were measured on; doc 48e, exchange G5): frozen on the
/// ask's quote at the accept, so the policy is told of the sale when it settles, as of a sale
/// into a bid. `None` where [`ask_policy_cuts`] has no rows to ask for.
///
/// # Errors
///
/// As [`ask_policy_cuts`].
pub fn ask_policy_proceeds(
    w: &World,
    listing: &crate::Listing,
    buyer: &PrincipalId,
    quantity: u64,
    price: Amount,
) -> Result<Option<crate::module::Proceeds>, MarketError> {
    let (
        crate::Supply::Lot {
            endpoint,
            commodity,
            ..
        },
        PrincipalId::Object(seller),
    ) = (&listing.source, &listing.seller)
    else {
        return Ok(None);
    };
    if !listing.is_personal() {
        return Ok(None);
    }
    let Some(policy) = crate::module::proceeds_policy(w) else {
        return Ok(None);
    };
    let gross = gross_of(price, quantity)?;
    let terms = crate::module::SaleTerms {
        seller: *seller,
        source: endpoint.0,
        house: buyer,
        market: &listing.market,
        listing: listing.id,
        commodity: &commodity.0,
        quantity,
        price,
        gross,
    };
    let p = policy
        .proceeds(w, &terms)
        .map_err(|r| MarketError::bad(r.line))?;
    let instrument = listing.account.and_then(|a| instrument_of(w, a));
    let mut sum: i64 = 0;
    for cut in &p.cuts {
        sum = sum
            .checked_add(cut.amount.minor())
            .ok_or_else(|| MarketError::bad(crate::trade::UNSETTLEABLE))?;
        if cut.check().is_err() || instrument.is_none() || instrument_of(w, cut.to) != instrument {
            return Err(MarketError::bad(crate::trade::UNSETTLEABLE));
        }
    }
    if p.cuts.len() > crate::record::MAX_PROCEEDS_CUTS || sum > gross.minor() || p.basis.minor() < 0
    {
        return Err(MarketError::bad(crate::trade::UNSETTLEABLE));
    }
    Ok(Some(p))
}

/// **A person's ask's payouts** (doc 48 cut EX.4): [`sale_payouts`] over the policy's cuts, the
/// one answer the builder writes and the standing acceptance requires.
///
/// # Errors
///
/// As [`ask_policy_cuts`], and a split whose rows come to more than the sale.
pub fn personal_ask_payouts(
    w: &World,
    listing: &crate::Listing,
    buyer: &PrincipalId,
    quantity: u64,
    price: Amount,
    seller_account: AccountId,
) -> Result<Vec<PayoutShare>, MarketError> {
    let gross = gross_of(price, quantity)?;
    let others = ask_policy_cuts(w, listing, buyer, quantity, price)?;
    sale_payouts(w, &listing.market, seller_account, gross, &others)
        .ok_or_else(|| MarketError::bad(crate::trade::UNSETTLEABLE))
}

fn gross_of(price: Amount, quantity: u64) -> Result<Amount, MarketError> {
    i64::try_from(quantity)
        .ok()
        .and_then(|q| price.minor().checked_mul(q))
        .map(Amount::from_minor)
        .ok_or_else(|| MarketError::bad(crate::trade::UNSETTLEABLE))
}

fn instrument_of(w: &World, account: AccountId) -> Option<mush_economy::InstrumentId> {
    w.module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().account(account).map(|a| a.instrument.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ex4_the_rates_floor_and_the_listing_fee_has_its_floor() {
        assert_eq!(
            per_mille(Amount::from_minor(100_000), SALE_CUT_PERMILLE).minor(),
            3_000
        );
        assert_eq!(
            per_mille(Amount::from_minor(100_000), LEVY_PERMILLE).minor(),
            1_000
        );
        assert_eq!(per_mille(Amount::from_minor(99), LEVY_PERMILLE).minor(), 0);
        assert_eq!(per_mille(Amount::from_minor(-5), LEVY_PERMILLE).minor(), 0);
        let floor = Amount::from_minor(1_000);
        assert_eq!(
            listing_fee(Amount::from_minor(50_000), floor).minor(),
            1_000,
            "the floor"
        );
        assert_eq!(
            listing_fee(Amount::from_minor(500_000), floor).minor(),
            5_000,
            "1%"
        );
    }

    #[test]
    fn ex4_the_fee_sentence_fits_120_columns() {
        let filled = FEE_SHORT.replace("{fee}", "1,234,567.89cr");
        assert!(filled.chars().count() < 120, "{filled}");
    }
}
