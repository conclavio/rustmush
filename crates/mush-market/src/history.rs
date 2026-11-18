//! **The price history** (doc 48 cut EX.8; Q-48-18, -35; doc 48b G-5).
//!
//! A venue's NPC prices for a good, one row a period (an IC week), kept apart from the trade
//! tape. The tape is a global ring of settled trades and cannot hold 24 IC weeks, so the history
//! is its own bounded series: [`MAX_PRICE_SERIES`] series of at most [`HISTORY_PERIODS`] rows,
//! each `market:history:{venue}:{ruleset}:{code}`, written only when a period closes.
//!
//! The market is game-neutral: it stores what the venue's price rule records and renders it.
//! The rule itself (how stock moves a price) is the ruleset's; TacSim's is
//! `tacsim::port::price`. A row's factor is what the rule carries from one period to the next,
//! so the series is also the rule's memory: a re-posted listing starts from the last factor,
//! never from the authored price.
//!
//! **Expensive to reverse** (flagged in the cut note): the row and series shape, the period
//! (an IC week) and the window (24 periods) are stored.

use mush_core::{CmdError, Ctx, World};
use mush_economy::{Amount, CatalogRef, InstrumentId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::MarketId;
use crate::store::{MarketStore, push_op};

/// One period, IC seconds: an IC week (Q-48-18's ruled step; at 24:1, seven real hours).
pub const PERIOD_IC_SECS: i64 = 7 * 86_400;
/// The window, in periods (Q-48-35): 24 IC weeks, about one real week at 24:1.
pub const HISTORY_PERIODS: usize = 24;
/// The most series the market keeps (one a venue and good).
pub const MAX_PRICE_SERIES: usize = 1_024;
/// The factor's floor and ceiling, permille of the base price (Q-48-18's 50–200% band).
pub const FACTOR_FLOOR: u16 = 500;
/// See [`FACTOR_FLOOR`].
pub const FACTOR_CEILING: u16 = 2_000;

/// The period an IC instant falls in.
#[must_use]
pub const fn period_of(ic_secs: i64) -> i64 {
    ic_secs.div_euclid(PERIOD_IC_SECS)
}

/// Which side of the board a point prices.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceSide {
    /// What the venue's houses sell at.
    Ask,
    /// What they buy at.
    Bid,
}

/// **One side's price in one period.**
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidePoint {
    /// The posted price, per unit, at the period's close.
    pub price: Amount,
    /// The rule's factor, permille of the base price, [`FACTOR_FLOOR`]..=[`FACTOR_CEILING`].
    pub factor_permille: u16,
}

/// **One period's row.**
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricePeriod {
    /// The period ([`period_of`]).
    pub period: i64,
    /// The ask, when a house asks the good here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<SidePoint>,
    /// The bid, when a house bids for it here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bid: Option<SidePoint>,
}

/// **A venue's price series for one good** (`market:history:{venue}:{ruleset}:{code}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "SeriesWire")]
pub struct PriceSeries {
    /// The venue.
    pub venue: MarketId,
    /// The good.
    pub catalog: CatalogRef,
    /// The instrument the prices are in, for the render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<InstrumentId>,
    /// The rows, oldest first, periods strictly rising, at most [`HISTORY_PERIODS`].
    pub periods: Vec<PricePeriod>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeriesWire {
    venue: MarketId,
    catalog: CatalogRef,
    #[serde(default)]
    instrument: Option<InstrumentId>,
    periods: Vec<PricePeriod>,
}

impl TryFrom<SeriesWire> for PriceSeries {
    type Error = String;
    fn try_from(w: SeriesWire) -> Result<Self, String> {
        if w.periods.len() > HISTORY_PERIODS {
            return Err("a price series holds more periods than the window".into());
        }
        if w.periods
            .iter()
            .zip(w.periods.iter().skip(1))
            .any(|(a, b)| a.period >= b.period)
        {
            return Err("a price series' periods do not rise".into());
        }
        let bad = |p: &Option<SidePoint>| {
            p.is_some_and(|p| {
                p.price.minor() <= 0
                    || !(FACTOR_FLOOR..=FACTOR_CEILING).contains(&p.factor_permille)
            })
        };
        if w.periods.iter().any(|p| bad(&p.ask) || bad(&p.bid)) {
            return Err("a price point is out of bounds".into());
        }
        Ok(PriceSeries {
            venue: w.venue,
            catalog: w.catalog,
            instrument: w.instrument,
            periods: w.periods,
        })
    }
}

impl PriceSeries {
    /// The latest point on `side`, with its period.
    #[must_use]
    pub fn latest(&self, side: PriceSide) -> Option<(i64, SidePoint)> {
        self.periods.iter().rev().find_map(|p| {
            match side {
                PriceSide::Ask => p.ask,
                PriceSide::Bid => p.bid,
            }
            .map(|point| (p.period, point))
        })
    }
}

/// The series' document key.
fn history_key(venue: &MarketId, catalog: &CatalogRef) -> SmolStr {
    SmolStr::new(format!(
        "market:history:{}:{}:{}",
        venue.as_str(),
        catalog.ruleset,
        catalog.code
    ))
}

/// **A venue's series for a good**, when one has been recorded.
#[must_use]
pub fn price_series<'w>(
    w: &'w World,
    venue: &MarketId,
    catalog: &CatalogRef,
) -> Option<&'w PriceSeries> {
    w.module_state::<MarketStore>()?
        .history
        .get(&(venue.clone(), catalog.clone()))
}

/// **The latest point on `side`**, with its period: the rule's carried factor.
#[must_use]
pub fn latest_point(
    w: &World,
    venue: &MarketId,
    catalog: &CatalogRef,
    side: PriceSide,
) -> Option<(i64, SidePoint)> {
    price_series(w, venue, catalog)?.latest(side)
}

/// **Record `side`'s point for `period`** (doc 48 cut EX.8): the period's row is written or
/// amended, the window trimmed to the newest [`HISTORY_PERIODS`], and the series' document
/// queued. A period before the series' newest is refused, as is a new series past
/// [`MAX_PRICE_SERIES`] or a point out of bounds; nothing is written on a refusal.
///
/// # Errors
///
/// The refusal, as a sentence for the log.
pub fn record_price(
    ctx: &mut Ctx<'_>,
    (venue, catalog): (&MarketId, &CatalogRef),
    instrument: Option<InstrumentId>,
    period: i64,
    side: PriceSide,
    point: SidePoint,
) -> Result<(), &'static str> {
    if point.price.minor() <= 0 || !(FACTOR_FLOOR..=FACTOR_CEILING).contains(&point.factor_permille)
    {
        return Err("a price point is out of bounds");
    }
    let key = (venue.clone(), catalog.clone());
    let Some(store) = ctx.w.module_state_mut::<MarketStore>() else {
        return Err("no market");
    };
    if !store.history.contains_key(&key) && store.history.len() >= MAX_PRICE_SERIES {
        return Err("the market keeps the most price series it may");
    }
    let series = store.history.entry(key).or_insert_with(|| PriceSeries {
        venue: venue.clone(),
        catalog: catalog.clone(),
        instrument: instrument.clone(),
        periods: Vec::new(),
    });
    match series.periods.last_mut() {
        Some(last) if last.period > period => return Err("that period has closed"),
        Some(last) if last.period == period => match side {
            PriceSide::Ask => last.ask = Some(point),
            PriceSide::Bid => last.bid = Some(point),
        },
        _ => series.periods.push(PricePeriod {
            period,
            ask: (side == PriceSide::Ask).then_some(point),
            bid: (side == PriceSide::Bid).then_some(point),
        }),
    }
    let over = series.periods.len().saturating_sub(HISTORY_PERIODS);
    series.periods.drain(..over);
    if series.instrument.is_none() {
        series.instrument = instrument;
    }
    let doc = series.clone();
    let Ok(value) = serde_json::to_value(&doc) else {
        tracing::error!("market: a price series did not serialize");
        return Err("a price series did not serialize");
    };
    push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: history_key(venue, catalog),
            value,
        },
    );
    Ok(())
}

/// Load one series document (`store::load`).
pub(crate) fn load_series(store: &mut MarketStore, key: &str, value: &serde_json::Value) {
    match serde_json::from_value::<PriceSeries>(value.clone()) {
        Ok(series) if store.history.len() < MAX_PRICE_SERIES => {
            store
                .history
                .insert((series.venue.clone(), series.catalog.clone()), series);
        }
        Ok(_) => tracing::warn!(key, "market: past the price-series cap on load; skipped"),
        Err(error) => tracing::warn!(key, %error, "market: an unreadable price series; skipped"),
    }
}

// ---- +market/history -------------------------------------------------------------------------

/// `+market/history` usage.
pub const HISTORY_USAGE: &str = "Usage: +market/history <catalog>:<code> [market=<venue>]";
/// No venue here and none named.
pub const HISTORY_NO_VENUE: &str = "Name a market with market=<venue>; you are not at one here.";
/// No series yet: `{good}`.
pub const HISTORY_NONE: &str = "No price history is kept here for {good} yet.";
/// The heading: `{good}`, `{n}` periods.
pub const HISTORY_HEAD: &str = "House prices of {good} here, by IC week, newest first ({n}):";
/// One row: `{when}`, `{ask}`, `{bid}`.
pub const HISTORY_ROW: &str = "  {when}: ask {ask}, bid {bid}";
/// The current period.
pub const HISTORY_THIS_WEEK: &str = "this IC week";
/// A past period: `{n}` weeks.
pub const HISTORY_WEEKS_AGO: &str = "{n} IC weeks ago";
/// The previous period.
pub const HISTORY_LAST_WEEK: &str = "last IC week";
/// A side nobody posts.
pub const HISTORY_NO_PRICE: &str = "none";

/// `+market/history <catalog>:<code> [market=<venue>]`.
pub(crate) fn history(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let arg = ctx.args.raw().trim().to_string();
    let mut catalog = None;
    let mut venue = None;
    for word in arg.split_whitespace() {
        if let Some(rest) = word.strip_prefix("market=") {
            venue = Some(MarketId::new(rest).ok_or_else(|| CmdError::msg(HISTORY_USAGE))?);
        } else if catalog.is_none() {
            catalog = Some(
                crate::commands::parse_catalog(word).ok_or_else(|| CmdError::msg(HISTORY_USAGE))?,
            );
        } else {
            return Err(CmdError::msg(HISTORY_USAGE));
        }
    }
    let catalog = catalog.ok_or_else(|| CmdError::msg(HISTORY_USAGE))?;
    let venue = venue
        .or_else(|| crate::commands::gate_venue(ctx))
        .ok_or_else(|| CmdError::msg(HISTORY_NO_VENUE))?;
    let good = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&catalog).map(|e| e.name.to_string()))
        .unwrap_or_else(|| catalog.code.to_string());
    let lines = match price_series(ctx.w, &venue, &catalog) {
        None => vec![HISTORY_NONE.replace("{good}", &good)],
        Some(series) if series.periods.is_empty() => vec![HISTORY_NONE.replace("{good}", &good)],
        Some(series) => history_lines(ctx.w, series, &good),
    };
    let actor = ctx.actor;
    for line in lines {
        ctx.out.sys_to_actor(actor, "market", line);
    }
    Ok(())
}

/// **A series' board lines** (doc 48 cut EX.8): the heading, then a row a period, newest first.
#[must_use]
pub fn history_lines(w: &World, series: &PriceSeries, good: &str) -> Vec<String> {
    let now = period_of(w.world_ic_now().at);
    let money = |p: Option<SidePoint>| {
        p.map_or_else(
            || HISTORY_NO_PRICE.to_string(),
            |p| crate::commands::money(w, series.instrument.as_ref(), p.price),
        )
    };
    let mut out = vec![
        HISTORY_HEAD
            .replace("{good}", good)
            .replace("{n}", &series.periods.len().to_string()),
    ];
    for p in series.periods.iter().rev() {
        let when = match now.saturating_sub(p.period) {
            n if n <= 0 => HISTORY_THIS_WEEK.to_string(),
            1 => HISTORY_LAST_WEEK.to_string(),
            n => HISTORY_WEEKS_AGO.replace("{n}", &n.to_string()),
        };
        out.push(
            HISTORY_ROW
                .replace("{when}", &when)
                .replace("{ask}", &money(p.ask))
                .replace("{bid}", &money(p.bid)),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(minor: i64, factor: u16) -> SidePoint {
        SidePoint {
            price: Amount::from_minor(minor),
            factor_permille: factor,
        }
    }

    /// **Doc 48 cut EX.8**: a stored series' wire refuses more periods than the window, periods
    /// that do not rise, and a point out of the band.
    #[test]
    fn ex8_a_series_wire_holds_its_bounds() {
        let row = |period, factor| PricePeriod {
            period,
            ask: Some(point(100, factor)),
            bid: None,
        };
        let doc = |periods: Vec<PricePeriod>| {
            serde_json::json!({
                "venue": "ceres",
                "catalog": {"ruleset": "tacsim", "code": "metals"},
                "periods": periods,
            })
        };
        let ok: Result<PriceSeries, _> =
            serde_json::from_value(doc(vec![row(1, 1_000), row(2, 1_050)]));
        assert!(ok.is_ok());
        let long: Vec<PricePeriod> = (0..25).map(|p| row(p, 1_000)).collect();
        assert!(
            serde_json::from_value::<PriceSeries>(doc(long)).is_err(),
            "past the window"
        );
        assert!(
            serde_json::from_value::<PriceSeries>(doc(vec![row(2, 1_000), row(2, 1_000)])).is_err(),
            "periods rise"
        );
        assert!(
            serde_json::from_value::<PriceSeries>(doc(vec![row(1, 2_001)])).is_err(),
            "inside the band"
        );
        assert_eq!(period_of(PERIOD_IC_SECS * 3 + 5), 3);
        assert_eq!(period_of(-1), -1);
    }
}
