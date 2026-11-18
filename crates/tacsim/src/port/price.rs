//! **The reach premium** (doc 19 §6, cut A.4; D-19-14; doc 18 Q-18-39): the
//! owner's "Pallas might be willing to pay more for emergency supplies when
//! awkward", as a demand signal tied to **reach**.
//!
//! A port's **reach** for a good at an instant is the least passage, in seconds,
//! from any port that asks (sells) that good to this one — the lane's hull class
//! planned empty with one soul over the supplier's listed routes, at full
//! throttle. Against the port's **synodic median** — the median of that reach
//! sampled over the next synodic period of the port's body and the supplier's
//! (the whole season, so the median is where a season spends half its days) — a
//! demand-side bid takes a premium:
//!
//! ```text
//! premium‰ = min(cap, k · (reach_now / reach_median − 1))     (never below zero)
//! price    = authored × (1000 + premium‰) / 1000               (whole major units, floor)
//! ```
//!
//! `k` and `cap` are the port entry's `premium` block (lane data, Q-19-5's
//! defaults 300 and 500 ‰); a port without one posts its authored prices. The
//! premium is read when a bid is **posted** — at apply and at every re-post of a
//! lapsed bid by the stock clerk — so the board re-prices on the bid's own
//! cadence (its TTL), each write signed by the vendor's officer as every bid is
//! (11 §2: nothing automated signs; the schema refuses an unsigned listing); the
//! doc's once-per-IC-day re-write in place is not built, and the median is
//! computed at each posting rather than cached per boot (at most
//! [`MAX_SCAN_SAMPLES`] plans per posting; under the constant sky, one). Quotes
//! stay frozen; the programme's reference reads the moved ask (D-18-24). Nothing
//! is minted: a dearer bid draws on the org's finite capital (doc 10 7A), and a
//! house that cannot fund a tonne posts nothing, as before.
//!
//! The emergency Wanted bids of doc 19 §6 (life-support goods at the cap when
//! reach crosses a ratio) are **not** in this cut (A.4b).

use mush_core::{CanonicalId, World};

use crate::definitions::GoodsTable;
use crate::port::def::{BidDef, PortDef};
use crate::port::passage::{DEFAULT_THROTTLE_PERMILLE, RoutePolicy, choose_route};
use crate::port::readiness::report_ports;
use crate::sky::Sky;
use crate::store::TacsimStore;

/// One IC day, the scan's unit.
pub const DAY_S: i64 = 86_400;
/// The longest scan, IC days: a synodic period past it (Jupiter–Saturn's 20
/// years; two bodies on the same orbit, which never close) is cut here.
pub const MAX_SCAN_DAYS: u32 = 2_000;
/// The most samples a scan takes: a longer period is sampled every
/// `ceil(days / MAX_SCAN_SAMPLES)` days (Ceres–Mars's 1,162-day season every
/// three days).
pub const MAX_SCAN_SAMPLES: u32 = 400;

/// A port's reach for a good, now and its synodic median, seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reach {
    /// The least passage from a supplier now.
    pub now_s: i64,
    /// The median of the same over the next synodic period.
    pub median_s: i64,
}

/// **The premium, permille** (D-19-14): `min(cap, k · (now / median − 1))`,
/// never below zero, in checked integer arithmetic (the ratio in permille first).
#[must_use]
pub fn premium_permille(k_permille: u32, cap_permille: u32, reach: Reach) -> u32 {
    if reach.median_s <= 0 || reach.now_s <= reach.median_s {
        return 0;
    }
    let excess_permille =
        i128::from(reach.now_s - reach.median_s) * 1_000 / i128::from(reach.median_s);
    let premium = excess_permille * i128::from(k_permille) / 1_000;
    let capped = premium.clamp(0, i128::from(cap_permille));
    u32::try_from(capped).unwrap_or(cap_permille)
}

/// **A bid's price under a premium**, major units: `authored × (1000 + premium)
/// / 1000`, whole units, floor; the authored price when the premium is zero.
#[must_use]
pub fn priced(authored_major: u64, premium_permille: u32) -> u64 {
    let scaled = u128::from(authored_major) * (1_000 + u128::from(premium_permille)) / 1_000;
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

/// **The synodic period of two orbits**, days: `1 / |1/a − 1/b|`; `None` when
/// the periods are equal (the bodies never close) or either is not positive.
#[must_use]
pub fn synodic_days(period_a_days: f64, period_b_days: f64) -> Option<f64> {
    if period_a_days <= 0.0 || period_b_days <= 0.0 {
        return None;
    }
    let closure = (1.0 / period_a_days - 1.0 / period_b_days).abs();
    if closure <= f64::EPSILON {
        return None;
    }
    Some(1.0 / closure)
}

/// **The scan's length**, IC days, for a port's body and a supplier's: their
/// synodic period at `at` rounded up, `1..=MAX_SCAN_DAYS`; the cap when the
/// period is undefined or either body is unknown to the sky.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn scan_days(sky: &Sky, port_body: &CanonicalId, supplier_body: &CanonicalId, at: i64) -> u32 {
    let (Some(a), Some(b)) = (
        sky.period_days(port_body, at),
        sky.period_days(supplier_body, at),
    ) else {
        return MAX_SCAN_DAYS;
    };
    match synodic_days(a, b) {
        Some(days) if days.is_finite() && days < f64::from(MAX_SCAN_DAYS) => {
            (days.ceil() as u32).max(1)
        }
        _ => MAX_SCAN_DAYS,
    }
}

/// The lower median of `samples` (the middle of the sorted values; the lower of
/// the two middles when even); `None` when empty.
#[must_use]
pub fn median_of(samples: &[i64]) -> Option<i64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    Some(sorted[(sorted.len() - 1) / 2])
}

/// **A port's reach for a good** (the module doc): over every other lane port
/// that asks `good` and lists a route to `def`'s venue, the least `choose_route`
/// for the lane's Dray empty with one soul, now, and the median of the same
/// sampled over the next synodic period (the longest over the suppliers, capped;
/// one sample under the constant sky). `None` when the port or its suppliers
/// have no body, no supplier is listed, or no passage plans now.
#[must_use]
pub fn reach_for(w: &World, def: &PortDef, good: &CanonicalId) -> Option<Reach> {
    let store = w.module_state::<TacsimStore>()?;
    let (pack, ports) = report_ports(store, w);
    let pack = pack?;
    let goods = store.goods()?;
    let ephemeris = &store.config().time.ephemeris;
    let sky = Sky::from_pack(pack, ephemeris);
    let dray = CanonicalId::new(crate::definitions::content::DRAY).ok()?;
    let (_, spec) = crate::port::burn::hull_spec_of(store.definitions(), goods, &dray).ok()?;
    let drive = spec.drive.clone().at_throttle(DEFAULT_THROTTLE_PERMILLE);
    let payload = u64::from(spec.allowance_kg);
    let now = w.world_ic_now().at;
    let body_of = |location: &CanonicalId| match pack.entry(location) {
        Some(crate::pack::PackEntry::Location(place)) => place.body.clone(),
        _ => None,
    };
    let port_body = body_of(&def.location)?;
    // The suppliers: their routes here and their bodies.
    let mut suppliers: Vec<(&[CanonicalId], CanonicalId)> = Vec::new();
    for (_, supplier) in ports {
        if supplier.venue == def.venue {
            continue;
        }
        let asks = supplier
            .vendors
            .iter()
            .any(|vendor| vendor.asks.iter().any(|ask| &ask.good == good));
        if !asks {
            continue;
        }
        let Some(routes) = supplier.routes.get(&def.venue).filter(|r| !r.is_empty()) else {
            continue;
        };
        let Some(body) = body_of(&supplier.location) else {
            continue;
        };
        suppliers.push((routes.as_slice(), body));
    }
    if suppliers.is_empty() {
        return None;
    }
    // The least passage over the suppliers at an instant.
    let reach_at = |depart_ic: i64| -> Option<i64> {
        suppliers
            .iter()
            .filter_map(|(routes, _)| {
                choose_route(
                    &sky,
                    pack,
                    routes,
                    &drive,
                    spec.dry_kg,
                    payload,
                    1,
                    depart_ic,
                    RoutePolicy::Fastest,
                    None,
                )
                .ok()
                .map(|plan| plan.total_s)
            })
            .min()
    };
    let now_s = reach_at(now)?;
    // The scan: the longest synodic period over the suppliers, at most
    // MAX_SCAN_SAMPLES samples; one under the constant sky.
    let days = if ephemeris.rate == 0 {
        1
    } else {
        suppliers
            .iter()
            .map(|(_, body)| scan_days(&sky, &port_body, body, now))
            .max()
            .unwrap_or(1)
    };
    let step_days = days.div_ceil(MAX_SCAN_SAMPLES).max(1);
    let mut samples = vec![now_s];
    let mut day = step_days;
    while day < days {
        if let Some(reach) = reach_at(now.saturating_add(i64::from(day) * DAY_S)) {
            samples.push(reach);
        }
        day += step_days;
    }
    let median_s = median_of(&samples)?;
    Some(Reach { now_s, median_s })
}

/// **A bid's price as the port posts it**, major units (D-19-14): the authored
/// price under the port's premium for the bid's good, or the authored price when
/// the port has no premium block or no reach.
#[must_use]
pub fn bid_price_major(w: &World, def: &PortDef, bid: &BidDef) -> u64 {
    // Doc 48 cut EX.8 (Q-48-18): the authored price under the stock rule's factor first, then the
    // reach premium on top.
    let base = factored(
        bid.price_major,
        factor_of(w, def, &bid.good, mush_market::PriceSide::Bid),
    );
    let Some(policy) = def.premium.as_ref() else {
        return base;
    };
    let Some(reach) = reach_for(w, def, &bid.good) else {
        return base;
    };
    priced(
        base,
        premium_permille(policy.k_permille, policy.cap_permille, reach),
    )
}

// ---- doc 19 cut A.4b: the emergency bids ----------------------------------------

/// How often the stock clerk takes a port's emergency reading, real seconds: once
/// an hour — one IC day at the shipped world rate of 24.
pub const EMERGENCY_CHECK_SECS: i64 = 3_600;

/// **Whether reach crosses the emergency ratio** (doc 19 §6): `now / median ≥
/// ratio`, in permille and integer arithmetic; never with a median of zero.
#[must_use]
pub fn emergency_crossed(reach: Reach, ratio_permille: u32) -> bool {
    reach.median_s > 0
        && reach.now_s > 0
        && i128::from(reach.now_s) * 1_000
            >= i128::from(reach.median_s) * i128::from(ratio_permille)
}

/// The reach ratio `now / median`, permille, for the chart; `None` without a
/// positive median.
#[must_use]
pub fn reach_ratio_permille(reach: Reach) -> Option<u64> {
    if reach.median_s <= 0 || reach.now_s < 0 {
        return None;
    }
    u64::try_from(i128::from(reach.now_s) * 1_000 / i128::from(reach.median_s)).ok()
}

/// **The reference an emergency bid is priced from**: the port's highest authored
/// ask or bid for the good among its vendors (the port's own board knows the
/// good's worth), else the goods table's `reference_major`; `None` when neither
/// is a positive figure.
#[must_use]
pub fn emergency_reference_major(
    def: &PortDef,
    goods: &GoodsTable,
    good: &CanonicalId,
) -> Option<u64> {
    let authored = def
        .vendors
        .iter()
        .flat_map(|vendor| {
            vendor
                .asks
                .iter()
                .map(|ask| (&ask.good, ask.price_major))
                .chain(vendor.bids.iter().map(|bid| (&bid.good, bid.price_major)))
        })
        .filter(|(named, price)| *named == good && *price > 0)
        .map(|(_, price)| price)
        .max();
    authored.or_else(|| {
        goods
            .good(good)
            .map(|row| row.reference_major)
            .filter(|&price| price > 0)
    })
}

/// One emergency good's reading, for the clerk and the chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmergencyReading {
    /// The good.
    pub good: CanonicalId,
    /// Its reach, when a port supplies it.
    pub reach: Option<Reach>,
    /// Whether the reach crosses the port's ratio.
    pub crossed: bool,
    /// What the emergency bid posts at, major units: the reference under the
    /// premium's cap; `None` without a reference.
    pub price_major: Option<u64>,
}

/// **The readings for every emergency good of `def`**, authored order; empty
/// when the port has no emergency block or no premium to take the cap from.
#[must_use]
pub fn emergency_readings(w: &World, def: &PortDef) -> Vec<EmergencyReading> {
    let (Some(emergency), Some(premium)) = (&def.emergency, &def.premium) else {
        return Vec::new();
    };
    let goods = w.module_state::<TacsimStore>().and_then(TacsimStore::goods);
    emergency
        .goods
        .iter()
        .map(|good| {
            let reach = reach_for(w, def, good);
            let crossed =
                reach.is_some_and(|reach| emergency_crossed(reach, emergency.ratio_permille));
            let price_major = goods
                .and_then(|goods| emergency_reference_major(def, goods, good))
                .map(|reference| priced(reference, premium.cap_permille));
            EmergencyReading {
                good: good.clone(),
                reach,
                crossed,
                price_major,
            }
        })
        .collect()
}

// ---- doc 48 cut EX.8: the NPC price rule ----------------------------------------------------
//
// Q-48-18 (owner ruling, 2026-11-15): an NPC house's ask and bid move with its stock against an
// authored target, by at most 5% an IC week, within 50% to 200% of the authored price, with the
// reach premium on top. Q-48-39: the throttle (a house in debt) is the one exception: its asks
// rise 10% an IC week inside the same band, and it posts no new bids.
//
// - **The factor** is permille of the authored price, carried period to period in the market's
//   price history (`mush_market::history`), so a re-posted listing starts from it.
// - **An ask's target** is its authored stock, the stock rows' quantity of the good: a storefront
//   holding the target asks the authored price; empty, it wants 200%; at twice the target, 50%.
// - **A bid's target** is what its house holds of the good against its authored intake: holding
//   none it bids the authored price, and as its stores fill to the intake the bid falls to 50%.
//   What the bid still wants is not the signal: a bid re-posted within its funds wants less when
//   its house is short of money, and a falling price would then let it buy more.
// - **The step** moves the factor toward the target by at most 5% of the current price an IC week.

/// The step, permille of the current price, an IC week (Q-48-18).
pub const STEP_PERMILLE: u32 = 50;
/// The throttle's step, permille of the current price, an IC week (Q-48-34, -39).
pub const THROTTLE_PERMILLE: u32 = 100;
/// The authored price's factor.
pub const PAR_PERMILLE: u16 = 1_000;

/// The market's catalogue reference for a port good (`tacsim:<good>`).
#[must_use]
pub fn catalog_of_good(good: &CanonicalId) -> mush_economy::CatalogRef {
    mush_economy::CatalogRef {
        ruleset: smol_str::SmolStr::new_static(crate::NS),
        code: smol_str::SmolStr::new(good.as_str()),
    }
}

fn band(permille: i128) -> u16 {
    let clamped = permille.clamp(
        i128::from(mush_market::FACTOR_FLOOR),
        i128::from(mush_market::FACTOR_CEILING),
    );
    u16::try_from(clamped).unwrap_or(PAR_PERMILLE)
}

/// **An ask's target factor**: `1000 × (2T − pool) / T`, in the band; par with no target.
#[must_use]
pub fn ask_target_permille(pool: u64, target: u64) -> u16 {
    if target == 0 {
        return PAR_PERMILLE;
    }
    let t = i128::from(target);
    band(1_000 * (2 * t - i128::from(pool)) / t)
}

/// **A bid's target factor**: `1000 − 500 × held / intake`, in the band; par with no intake. A
/// house holding none of the good bids par, and one whose stores hold its whole intake bids half.
#[must_use]
pub fn bid_target_permille(held: u64, intake: u64) -> u16 {
    if intake == 0 {
        return PAR_PERMILLE;
    }
    band(1_000 - 500 * i128::from(held.min(intake)) / i128::from(intake))
}

/// **One week's step** from `current` toward `desired`: at most [`STEP_PERMILLE`] of the current
/// factor (at least one permille, so a step is never stuck), in the band. Throttled, the factor
/// rises [`THROTTLE_PERMILLE`] of itself whatever the target, to the ceiling.
#[must_use]
pub fn step_permille(current: u16, desired: u16, throttled: bool) -> u16 {
    let cur = i128::from(current);
    if throttled {
        return band(cur + (cur * i128::from(THROTTLE_PERMILLE) / 1_000).max(1));
    }
    let most = (cur * i128::from(STEP_PERMILLE) / 1_000).max(1);
    let delta = (i128::from(desired) - cur).clamp(-most, most);
    band(cur + delta)
}

/// `base_major × factor / 1000`, whole units, floor, at least one.
#[must_use]
pub fn factored(base_major: u64, factor: u16) -> u64 {
    let scaled = u128::from(base_major) * u128::from(factor) / 1_000;
    u64::try_from(scaled).unwrap_or(u64::MAX).max(1)
}

/// The factor a port's `side` of `good` carries: the latest recorded, par before any.
#[must_use]
pub fn factor_of(
    w: &World,
    def: &PortDef,
    good: &CanonicalId,
    side: mush_market::PriceSide,
) -> u16 {
    mush_market::MarketId::new(def.venue.as_str())
        .and_then(|venue| mush_market::latest_point(w, &venue, &catalog_of_good(good), side))
        .map_or(PAR_PERMILLE, |(_, point)| point.factor_permille)
}

/// **An ask's price as the port posts it**, major units: the authored price under its factor.
#[must_use]
pub fn ask_price_major(w: &World, def: &PortDef, good: &CanonicalId, authored_major: u64) -> u64 {
    factored(
        authored_major,
        factor_of(w, def, good, mush_market::PriceSide::Ask),
    )
}

/// **Whether a house is throttled** (Q-48-39): it owes an obligation it has gone delinquent on.
/// EX.4's player-owed class and the economy's own obligations both count; running costs (Q-48-38)
/// are not built, so a delinquency is today's one sign a house cannot pay.
#[must_use]
pub fn throttled(w: &World, org: mush_org::OrgId) -> bool {
    let debtor = mush_org::PrincipalId::Organization(org);
    w.module_state::<mush_economy::EconomyStore>()
        .is_some_and(|e| {
            e.snapshot().obligations().any(|o| {
                o.debtor == debtor && o.status == mush_economy::ObligationStatus::Delinquent
            })
        })
}

/// The minor units of `major` on this world's credit.
fn minor(w: &World, major: u64) -> Option<mush_economy::Amount> {
    let scale = crate::port::apply::credit_scale(w);
    crate::port::apply::minor_of(major, scale).map(mush_economy::Amount::from_minor)
}

/// **The week's step for one port** (doc 48 cut EX.8): every NPC ask and bid of the port whose
/// latest recorded period is before this IC week is stepped toward its stock's target, recorded in
/// the market's price history, and its standing listing re-priced in place. A row stepped this
/// week is left alone, so the step runs once an IC week whatever the fire rate. Writes only market
/// documents; submits nothing.
pub(crate) fn weekly_step(ctx: &mut mush_core::Ctx<'_>, map: &crate::port::PortMap, def: &PortDef) {
    let Some(venue) = mush_market::MarketId::new(def.venue.as_str()) else {
        return;
    };
    let period = mush_market::period_of(ctx.w.world_ic_now().at);
    let instrument = mush_economy::InstrumentId::new(crate::economy::CREDIT);
    let mut asks: Vec<(CanonicalId, u16, u64, Option<mush_market::ListingId>)> = Vec::new();
    let mut bids: Vec<(CanonicalId, u16, u64, Option<mush_market::ListingId>)> = Vec::new();
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        let hold = map
            .orgs
            .get(&vendor.org)
            .is_some_and(|o| throttled(ctx.w, mush_org::OrgId(o.org)));
        for ask in vendor
            .asks
            .iter()
            .filter(|ask| !crate::port_tick::is_part(&ask.good))
        {
            let catalog = catalog_of_good(&ask.good);
            let latest =
                mush_market::latest_point(ctx.w, &venue, &catalog, mush_market::PriceSide::Ask);
            if latest.is_some_and(|(p, _)| p >= period) {
                continue;
            }
            let target: u64 = vendor
                .stock
                .iter()
                .filter(|s| s.good == ask.good)
                .map(|s| s.qty)
                .sum();
            let endpoint = mush_inventory::EndpointRef(mush_economy::VaultId(row.vault));
            let Ok(pool) = crate::port_tick::read_pool(ctx, endpoint, &ask.good) else {
                continue;
            };
            let current = latest.map_or(PAR_PERMILLE, |(_, p)| p.factor_permille);
            let next = step_permille(current, ask_target_permille(pool, target), hold);
            let listing = ctx
                .w
                .module_state::<mush_market::MarketStore>()
                .and_then(|s| crate::port::stock::ask_reading(s, row.vault, &ask.good).listing)
                .map(|(id, _)| id);
            // A row not yet listed and never stepped has not started: the clerk is still
            // seeding it, and an empty pool then is not scarcity.
            if listing.is_none() && latest.is_none() {
                continue;
            }
            asks.push((
                ask.good.clone(),
                next,
                factored(ask.price_major, next),
                listing.filter(|_| next != current),
            ));
        }
        if hold {
            continue;
        }
        for bid in &vendor.bids {
            let catalog = catalog_of_good(&bid.good);
            let latest =
                mush_market::latest_point(ctx.w, &venue, &catalog, mush_market::PriceSide::Bid);
            if latest.is_some_and(|(p, _)| p >= period) {
                continue;
            }
            let listing = row
                .bids
                .get(&bid.good)
                .map(|r| mush_market::ListingId(r.listing))
                .filter(|id| {
                    ctx.w
                        .module_state::<mush_market::MarketStore>()
                        .is_some_and(|s| s.listing(*id).is_some())
                });
            // A bid not standing has no price to move; its re-post starts from the carried factor.
            if listing.is_none() {
                continue;
            }
            let endpoint = mush_inventory::EndpointRef(mush_economy::VaultId(row.vault));
            let Ok(held) = crate::port_tick::read_pool(ctx, endpoint, &bid.good) else {
                continue;
            };
            let intake = bid.intake;
            let current = latest.map_or(PAR_PERMILLE, |(_, p)| p.factor_permille);
            let next = step_permille(current, bid_target_permille(held, intake), false);
            let premium = def
                .premium
                .as_ref()
                .zip(reach_for(ctx.w, def, &bid.good))
                .map_or(0, |(policy, reach)| {
                    premium_permille(policy.k_permille, policy.cap_permille, reach)
                });
            let price = priced(factored(bid.price_major, next), premium);
            // An unchanged factor leaves the standing bid as posted (its reach premium is read
            // at posting, as before); only a step re-prices it.
            bids.push((
                bid.good.clone(),
                next,
                price,
                listing.filter(|_| next != current),
            ));
        }
    }
    for (side, rows) in [
        (mush_market::PriceSide::Ask, asks),
        (mush_market::PriceSide::Bid, bids),
    ] {
        for (good, factor, major, listing) in rows {
            let Some(price) = minor(ctx.w, major) else {
                continue;
            };
            let point = mush_market::SidePoint {
                price,
                factor_permille: factor,
            };
            if let Err(why) = mush_market::record_price(
                ctx,
                (&venue, &catalog_of_good(&good)),
                instrument.clone(),
                period,
                side,
                point,
            ) {
                tracing::warn!(venue = %venue.as_str(), good = %good, why, "tacsim: a price point was not recorded");
                continue;
            }
            if let Some(listing) = listing
                && let Err(error) = mush_market::reprice_listing(ctx, listing, price)
            {
                tracing::warn!(
                    listing = listing.0,
                    ?error,
                    "tacsim: a house listing was not re-priced"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 48 cut EX.8**: an ask at its target asks par; empty it wants the ceiling; at twice
    /// the target the floor. A bid bids par while its house holds none and falls as its stores fill.
    #[test]
    fn ex8_the_targets_follow_stock_within_the_band() {
        assert_eq!(ask_target_permille(120, 120), 1_000);
        assert_eq!(ask_target_permille(0, 120), 2_000);
        assert_eq!(ask_target_permille(80, 120), 1_333);
        assert_eq!(ask_target_permille(240, 120), 500);
        assert_eq!(ask_target_permille(1_000, 120), 500, "the floor");
        assert_eq!(ask_target_permille(7, 0), 1_000, "no target, par");
        assert_eq!(bid_target_permille(0, 240), 1_000, "holding none, par");
        assert_eq!(bid_target_permille(120, 240), 750);
        assert_eq!(bid_target_permille(240, 240), 500, "the stores full");
        assert_eq!(bid_target_permille(480, 240), 500, "the floor");
    }

    /// **Doc 48 cut EX.8**: at most 5% of the current factor a week toward the target, never
    /// past it; throttled, 10% up whatever the target; always inside 500..=2000.
    #[test]
    fn ex8_a_step_is_five_percent_and_the_throttle_ten() {
        assert_eq!(step_permille(1_000, 2_000, false), 1_050);
        assert_eq!(step_permille(1_050, 2_000, false), 1_102);
        assert_eq!(step_permille(1_000, 500, false), 950);
        assert_eq!(
            step_permille(1_000, 1_020, false),
            1_020,
            "never past the target"
        );
        assert_eq!(step_permille(1_000, 1_000, false), 1_000);
        assert_eq!(step_permille(1_990, 2_000, false), 2_000);
        assert_eq!(step_permille(1_000, 500, true), 1_100, "the throttle rises");
        assert_eq!(step_permille(1_950, 500, true), 2_000, "to the ceiling");
        assert_eq!(factored(1_500, 1_333), 1_999);
        assert_eq!(factored(1, 500), 1, "at least one");
    }

    #[test]
    fn a4b_the_emergency_crosses_at_the_ratio() {
        let at = Reach {
            now_s: 1_500,
            median_s: 1_000,
        };
        assert!(emergency_crossed(at, 1_500), "at the ratio counts");
        assert!(!emergency_crossed(at, 1_501));
        assert!(emergency_crossed(at, 1_000));
        let same = Reach {
            now_s: 1_000,
            median_s: 1_000,
        };
        assert!(
            emergency_crossed(same, 1_000),
            "a ratio of 1000 is always on"
        );
        assert!(!emergency_crossed(same, 1_001));
        let broken = Reach {
            now_s: 5,
            median_s: 0,
        };
        assert!(!emergency_crossed(broken, 1_000));
        assert_eq!(reach_ratio_permille(at), Some(1_500));
        assert_eq!(reach_ratio_permille(broken), None);
    }

    #[test]
    fn a4_the_premium_is_the_capped_excess_over_the_median_reach() {
        let same = Reach {
            now_s: 1_000,
            median_s: 1_000,
        };
        assert_eq!(premium_permille(300, 500, same), 0);
        let better = Reach {
            now_s: 800,
            median_s: 1_000,
        };
        assert_eq!(
            premium_permille(300, 500, better),
            0,
            "a good season pays no less"
        );
        let worse = Reach {
            now_s: 1_800,
            median_s: 1_000,
        };
        assert_eq!(
            premium_permille(300, 500, worse),
            240,
            "80 % farther at k 300 is +24 %"
        );
        let far = Reach {
            now_s: 4_000,
            median_s: 1_000,
        };
        assert_eq!(premium_permille(300, 500, far), 500, "the cap");
        assert_eq!(premium_permille(0, 500, far), 0, "k 0 is no policy");
        let broken = Reach {
            now_s: 5,
            median_s: 0,
        };
        assert_eq!(premium_permille(300, 500, broken), 0);
    }

    #[test]
    fn a4_the_priced_bid_is_whole_major_units_floor() {
        assert_eq!(priced(280, 0), 280);
        assert_eq!(priced(280, 240), 347, "280 × 1.24 = 347.2");
        assert_eq!(priced(520, 500), 780);
        assert_eq!(priced(u64::MAX, 500), u64::MAX, "saturates");
    }

    #[test]
    fn a4_the_synodic_period_and_the_median() {
        let synodic = synodic_days(687.0, 1_681.6).expect("Mars and Ceres close");
        assert!((synodic - 1_161.5).abs() < 2.0, "{synodic}");
        assert_eq!(
            synodic_days(100.0, 100.0),
            None,
            "the same orbit never closes"
        );
        assert_eq!(synodic_days(0.0, 100.0), None);
        assert_eq!(median_of(&[]), None);
        assert_eq!(median_of(&[3, 1, 2]), Some(2));
        assert_eq!(median_of(&[4, 1, 3, 2]), Some(2), "the lower median");
        assert_eq!(median_of(&[7]), Some(7));
    }
}
