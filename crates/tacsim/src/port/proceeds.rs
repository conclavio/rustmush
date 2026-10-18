//! **The Co-op's share and the sale line** (doc 18 cut IC10; doc 18 §4.1 "The
//! share", §4.3 "Bids that take real lots", §6; D-18-24, D-18-26).
//!
//! The market's game-neutral proceeds seam (`mush_market::ProceedsPolicy`) asks one
//! question per sale into a standing bid, before any contract — who besides the seller
//! is paid, and what the seller reads — and tells the policy once per settled sale.
//! [`CoopProceeds`] is TacSim's answer:
//!
//! - **Enrolled?** The source is a programme hull's hold ([`governed_hold`]), the
//!   Trust holds a live lease on that hull, and the lease's enrolment names that hold
//!   and stands `Active` or `BuyoutPending`. A live lease whose books cannot be read
//!   refuses ([`SALE_BOOKS_DARK`]); a live lease whose pilot is not the seller refuses
//!   ([`SALE_NOT_PILOT`]) — the policy's own belt behind the gate's
//!   `NOT_YOUR_CHARTER`, because a policy must not trust its caller.
//! - **The reference** (D-18-24; lead rulings (a), 2h-iv): the programme's `reference`
//!   names a listing (`"<port entry>/<org>"`), never a price; its **current** price is
//!   read at contract build, the one instant a read can change the money (an `Exact`
//!   payout must sum to the escrow when the contract is offered). Sold out, withdrawn,
//!   or no reference entry for the good: the goods table's `reference_major`, logged
//!   once and recorded on the ledger entry as the contract-name memo
//!   [`BASELINE_MEMO`]. Never a zero reference ([`REFERENCE_DARK`]).
//! - **The share** ([`share_of`]): 250‰ (the enrolment's frozen permille) of the
//!   margin over reference, floored in minor units — the pilot's favour (decision 9).
//!   Zero below or at reference, and then no payout row at all.
//! - **The counters** fold at settlement (lead ruling (e)), keyed by the pair
//!   `(pilot == seller, vault == source)` — never by the vault alone, since a hull's
//!   hold outlives a charter once cut IC13's recall frees a revoked Dray (reviewer F3).
//!   A **graduated** enrolment folds nothing (cut IC13, lead ruling L10): the hull is
//!   the pilot's and the lay has ended. A Revoked one still folds a programme sale
//!   accepted before its charter ended, which settles on programme terms.
//! - **The residual** (cut IC13): a settlement healed at boot through
//!   `reconcile_contracts` is not folded (the IC10 residual); the counters are the
//!   Trust's bookkeeping only, and the buyout price is the enrolment's frozen
//!   `buyout_minor`, never derived from them.
//!
//! The policy reads `&World` only when it prices a sale, and writes only the
//! enrolment, through the store's single write path, when it is told one settled.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::Amount;
use mush_inventory::record::EndpointRef;
use mush_market::{
    MarketStore, Proceeds, ProceedsCut, ProceedsPolicy, ProceedsRefusal, SaleTerms, SettledSale,
};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::definitions::content::LANE_PACK;
use crate::port::apply::{credit_scale, minor_of};
use crate::port::board::{group_thousands, house_name, label_of, money, unit_of};
use crate::port::coop::{EnrolmentStatus, name_of, programme_port, trust_lease_on};
use crate::port::def::PortDef;
use crate::port::map::{PortMap, PortMapState};
use crate::port::market::{governed_hold, port_of_venue};
use crate::store::{TacsimStore, put_enrolment};

/// The sale line for an enrolled hold (doc 18 §6, adapted; owner item O1). `{qty}`,
/// `{unit}`, `{label}`, `{house}`, `{price}`, `{gross}`, `{share}`, `{net}`,
/// `{signatory}`.
pub const SALE_LINE: &str = "Sold {qty} {unit} of {label} to {house} at {price}/{unit}: {gross}; Co-op share {share}; you receive {net} when the goods are in. {signatory} countersigns.";
/// The sale line for a hold off the programme (no share).
pub const SALE_LINE_NO_SHARE: &str = "Sold {qty} {unit} of {label} to {house} at {price}/{unit}: {gross}; you receive {gross} when the goods are in. {signatory} countersigns.";
/// A live charter whose enrolment cannot be read: refused before any contract.
pub const SALE_BOOKS_DARK: &str =
    "The Co-op's books for your charter cannot be read right now, so nothing was sold.";
/// No usable reference price for `{label}`: refused before any contract.
pub const REFERENCE_DARK: &str =
    "The Co-op's reference price for {label} cannot be read right now, so nothing was sold.";
/// A sale out of an enrolled hold by anyone but its pilot (`{name}`: the hull).
pub const SALE_NOT_PILOT: &str =
    "Only the {name}'s chartered pilot sells out of its hold, so nothing was sold.";
/// The payout's label on the sale contract and the line.
pub const SHARE_LABEL: &str = "Co-op share";
/// The contract-name memo when the reference fell back to the goods table.
pub const BASELINE_MEMO: &str = "baseline ref";

/// **The Co-op's share of one sale** (doc 18 §4.1, D-18-24; cut IC10): `Some((basis,
/// share))` where `basis = max(0, price − reference) × quantity` and `share =
/// floor(basis × permille / 1000)`, all in minor units, checked in `i128`; `None` on
/// overflow (or a result outside `i64`). Floor rounding is the pilot's favour.
#[must_use]
pub fn share_of(
    price: Amount,
    reference: Amount,
    quantity: u64,
    permille: u32,
) -> Option<(Amount, Amount)> {
    let margin = i128::from(price.minor())
        .checked_sub(i128::from(reference.minor()))?
        .max(0);
    let basis = margin.checked_mul(i128::from(quantity))?;
    let share = basis
        .checked_mul(i128::from(permille))?
        .checked_div(1_000)?;
    Some((
        Amount::from_minor(i64::try_from(basis).ok()?),
        Amount::from_minor(i64::try_from(share).ok()?),
    ))
}

/// **TacSim's proceeds policy** (doc 18 §4.1 "The share", D-18-24; cut IC10), wired on
/// the TacSim roster beside the port gate and capacity provider
/// (`with_port_seams_if`).
#[derive(Debug)]
pub struct CoopProceeds;

fn refuse(line: String) -> ProceedsRefusal {
    ProceedsRefusal { line }
}

/// The house behind a bid: its port's def and map, the vendor org id, and the bid
/// authored for the good.
struct House<'a> {
    def: &'a PortDef,
    map: &'a PortMap,
    org: CanonicalId,
}

fn house_of<'a>(w: &'a World, sale: &SaleTerms<'_>) -> Option<House<'a>> {
    let (def, map) = port_of_venue(w, sale.market)?;
    let map = map?;
    let PrincipalId::Organization(house) = sale.house else {
        return None;
    };
    let org = map
        .vendors
        .keys()
        .find(|id| map.orgs.get(*id).is_some_and(|row| row.org == house.0))?
        .clone();
    Some(House { def, map, org })
}

/// The live reference price of `good`, or the goods table's baseline with the memo.
/// `Err` when neither reads (never a zero reference).
fn reference_of(
    w: &World,
    reference: Option<&String>,
    good: &CanonicalId,
    label: &str,
) -> Result<(Amount, Option<SmolStr>), ProceedsRefusal> {
    let live = reference.and_then(|named| {
        let (entry, org) = named.split_once('/')?;
        let entry = CanonicalId::new(entry).ok()?;
        let org = CanonicalId::new(org).ok()?;
        let store = w.module_state::<TacsimStore>()?;
        let lane = CanonicalId::new(LANE_PACK).ok()?;
        let PortMapState::Present(map) = store.port_map(&lane, &entry) else {
            return None;
        };
        let vault = map.vendors.get(&org)?.vault;
        let market = w.module_state::<MarketStore>()?;
        let listing = market
            .listing(market.lot_listing_of(EndpointRef(mush_economy::VaultId(vault)), good)?)?;
        (listing.price.minor() > 0).then_some(listing.price)
    });
    if let Some(price) = live {
        return Ok((price, None));
    }
    let dark = || refuse(REFERENCE_DARK.replace("{label}", label));
    let baseline = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .and_then(|goods| goods.good(good).map(|row| row.reference_major))
        .and_then(|major| minor_of(major, credit_scale(w)))
        .filter(|minor| *minor > 0)
        .ok_or_else(dark)?;
    tracing::warn!(
        %good,
        reference = reference.map_or("", String::as_str),
        "tacsim: a sale's reference listing is gone; the Co-op share is measured against the \
         goods table's baseline"
    );
    Ok((
        Amount::from_minor(baseline),
        Some(SmolStr::new_static(BASELINE_MEMO)),
    ))
}

/// **Whether an enrolment in this status takes the Co-op's share** (doc 18 §4.1, cut
/// IC13, lead ruling L10): `Active` and `BuyoutPending` do; `Graduated` and `Revoked`
/// do not. A graduated hull carries no Trust lease, so [`enrolled`] already answers
/// `Ok(None)` before it reads a status; this arm is the belt (lead 2h).
#[must_use]
pub(crate) const fn takes_share(status: EnrolmentStatus) -> bool {
    match status {
        EnrolmentStatus::Active | EnrolmentStatus::BuyoutPending => true,
        EnrolmentStatus::Graduated | EnrolmentStatus::Revoked => false,
    }
}

/// Whether the sale's source is an enrolled hold, and if so the enrolment's frozen
/// permille. `Err` for a live charter with dark books or another pilot.
fn enrolled(w: &World, sale: &SaleTerms<'_>) -> Result<Option<u32>, ProceedsRefusal> {
    let Some(view) = governed_hold(w, sale.source) else {
        return Ok(None);
    };
    let hull = view.hull();
    let Some(port) = programme_port(w) else {
        return Ok(None);
    };
    let Some(trust) = port.trust() else {
        return Ok(None);
    };
    let Some(lease) = trust_lease_on(w, &trust, hull) else {
        return Ok(None);
    };
    let enrolment = w
        .module_state::<TacsimStore>()
        .and_then(|s| s.enrolment_for_lease(lease.id.0))
        .filter(|e| e.vault == sale.source.vault().0 && takes_share(e.status));
    let Some(enrolment) = enrolment else {
        return Err(refuse(SALE_BOOKS_DARK.to_owned()));
    };
    if enrolment.pilot != sale.seller {
        return Err(refuse(SALE_NOT_PILOT.replace("{name}", &name_of(w, hull))));
    }
    Ok(Some(enrolment.share_permille))
}

impl ProceedsPolicy for CoopProceeds {
    fn proceeds(&self, w: &World, sale: &SaleTerms<'_>) -> Result<Proceeds, ProceedsRefusal> {
        let Some(house) = house_of(w, sale) else {
            // A venue no port authors: the market's own line, no cuts.
            return Ok(Proceeds::default());
        };
        let goods = w.module_state::<TacsimStore>().and_then(TacsimStore::goods);
        let (label, unit) = goods.map_or_else(
            || (sale.commodity.to_string(), String::new()),
            |g| (label_of(g, sale.commodity), unit_of(g, sale.commodity)),
        );
        let vendor = house.def.vendors.iter().find(|v| v.org == house.org);
        let bid = vendor.and_then(|v| v.bids.iter().find(|b| &b.good == sale.commodity));
        let house_label = house_name(house.def, &house.org, bid);
        let signatory = house
            .map
            .vendors
            .get(&house.org)
            .map_or_else(String::new, |row| name_of(w, ObjId(row.signatory)));
        let qty = group_thousands(sale.quantity);
        let fill = |template: &str| {
            template
                .replace("{qty}", &qty)
                .replace("{unit}", &unit)
                .replace("{label}", &label)
                .replace("{house}", &house_label)
                .replace("{price}", &money(w, sale.price))
                .replace("{gross}", &money(w, sale.gross))
                .replace("{signatory}", &signatory)
        };
        let Some(permille) = enrolled(w, sale)? else {
            return Ok(Proceeds {
                line: Some(fill(SALE_LINE_NO_SHARE)),
                ..Proceeds::default()
            });
        };
        let Some(port) = programme_port(w) else {
            return Err(refuse(SALE_BOOKS_DARK.to_owned()));
        };
        // Doc 18 cut IC14 row 66 (documented, no behaviour change): the reference is
        // read here, when the sale's contract is built, and the share is frozen on
        // the quote; the live ask can move before the settlement, so the share may
        // drift from it by at most one draw window. Re-reading it at settlement with a
        // remainder cut is the owner's open question Q-18-50.
        let (reference, memo) = reference_of(
            w,
            port.programme.reference.get(sale.commodity),
            sale.commodity,
            &label,
        )?;
        let Some((basis, share)) = share_of(sale.price, reference, sale.quantity, permille) else {
            return Err(refuse(REFERENCE_DARK.replace("{label}", &label)));
        };
        let mut cuts = Vec::new();
        if share.minor() > 0 {
            let Some(to) = port.trust_account() else {
                return Err(refuse(SALE_BOOKS_DARK.to_owned()));
            };
            cuts.push(ProceedsCut {
                to,
                amount: share,
                label: SmolStr::new_static(SHARE_LABEL),
            });
        }
        let net = Amount::from_minor(sale.gross.minor().saturating_sub(share.minor()));
        let line = fill(SALE_LINE)
            .replace("{share}", &money(w, share))
            .replace("{net}", &money(w, net));
        Ok(Proceeds {
            cuts,
            basis,
            line: Some(line),
            memo,
        })
    }

    fn settled(&self, ctx: &mut Ctx<'_>, sale: &SettledSale<'_>) {
        let PrincipalId::Object(pilot) = sale.seller else {
            return;
        };
        let found = ctx.w.module_state::<TacsimStore>().and_then(|s| {
            s.enrolments()
                .find(|e| e.pilot == *pilot && e.vault == sale.source.vault().0)
                .cloned()
        });
        let Some(mut e) = found else {
            tracing::warn!(
                quote = sale.quote.0,
                contract = sale.contract.0,
                "tacsim: a sale into a bid settled out of a hold with no enrolment for its seller; \
                 nothing folds"
            );
            return;
        };
        // Cut IC13 (lead ruling L10): a graduate's lay has ended; nothing folds.
        if e.status == EnrolmentStatus::Graduated {
            tracing::debug!(
                quote = sale.quote.0,
                pilot = pilot.0,
                "tacsim: a graduate's sale settled; the Co-op's counters do not move"
            );
            return;
        }
        let shares = sale
            .cuts
            .iter()
            .try_fold(0_i64, |sum, cut| sum.checked_add(cut.amount.minor()));
        let folded = shares.and_then(|shares| {
            Some((
                e.sales.checked_add(1)?,
                e.margin_minor.checked_add(sale.basis.minor())?,
                e.shares_minor.checked_add(shares)?,
            ))
        });
        let Some((sales, margin, shares)) = folded else {
            tracing::error!(
                quote = sale.quote.0,
                pilot = pilot.0,
                "tacsim: an enrolment's sale counters would overflow; nothing folds"
            );
            return;
        };
        e.sales = sales;
        e.margin_minor = margin;
        e.shares_minor = shares;
        if let Err(error) = e.check() {
            tracing::error!(
                quote = sale.quote.0,
                pilot = pilot.0,
                %error,
                "tacsim: a folded enrolment broke its own invariants; nothing folds"
            );
            return;
        }
        put_enrolment(ctx, &e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cr(major: i64) -> Amount {
        Amount::from_minor(major * 100)
    }

    /// Doc 18 cut IC13 (lead ruling L10): the share stops at graduation.
    #[test]
    fn ic13_a_graduate_takes_no_share() {
        assert!(takes_share(EnrolmentStatus::Active));
        assert!(takes_share(EnrolmentStatus::BuyoutPending));
        assert!(!takes_share(EnrolmentStatus::Graduated));
        assert!(!takes_share(EnrolmentStatus::Revoked));
    }

    #[test]
    fn ic10_the_share_is_the_floor_of_the_permille_of_the_margin() {
        // At and under reference: nothing.
        assert_eq!(
            share_of(cr(200), cr(200), 20, 250),
            Some((Amount::ZERO, Amount::ZERO))
        );
        assert_eq!(
            share_of(cr(150), cr(200), 20, 250),
            Some((Amount::ZERO, Amount::ZERO))
        );
        // The verify lens: 280 against 200, 20 t, 250‰ → 1,600.00 and 400.00.
        assert_eq!(
            share_of(cr(280), cr(200), 20, 250),
            Some((Amount::from_minor(160_000), Amount::from_minor(40_000)))
        );
        // Floored: a margin of 3 minor at 250‰ is 0.75 → 0.
        assert_eq!(
            share_of(Amount::from_minor(203), Amount::from_minor(200), 1, 250),
            Some((Amount::from_minor(3), Amount::ZERO))
        );
        assert_eq!(
            share_of(Amount::from_minor(207), Amount::from_minor(200), 1, 250),
            Some((Amount::from_minor(7), Amount::from_minor(1)))
        );
        // Overflow out of i64 is None, never a wrapped share.
        assert_eq!(
            share_of(Amount::from_minor(i64::MAX), Amount::ZERO, u64::MAX, 1_000),
            None
        );
    }
}
