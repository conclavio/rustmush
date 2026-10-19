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
    let Some(policy) = def.premium.as_ref() else {
        return bid.price_major;
    };
    let Some(reach) = reach_for(w, def, &bid.good) else {
        return bid.price_major;
    };
    priced(
        bid.price_major,
        premium_permille(policy.k_permille, policy.cap_permille, reach),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
