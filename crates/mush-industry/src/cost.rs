//! **Doc 39 cut IN.4 — the facility cost model** (§4.4; Q-39-9, Q-39-10, Q-39-24, Q-39-32;
//! the owner's Q-38-12 and Q-38-15).
//!
//! A [`FacilityCostDef`] is authored beside a facility's slots and stored on its
//! [`FacilityRecord`](crate::FacilityRecord)'s policy half. It turns capital, lifetime, upkeep
//! and staff into a **daily cost**; the daily cost over the slots and the 24 IC hours of a day
//! is the **slot-hour**, the price of one work unit (`PRODUCTION_CADENCE_SECS`, one IC hour) in
//! one slot. Billing is by work unit, never by whole slot-day (Q-39-32):
//!
//! ```text
//! daily_cost    = capital / lifetime_days + capital × upkeep_permille / 1000 / 365 + staff × wage
//! slot_hour     = daily_cost / slots / 24
//! fee(order)    = work_units × slot_hour × (1 + margin_permille / 1000)
//! floor         = the fee at margin 0 (Q-38-12's "before labour and margin")
//! ```
//!
//! Every figure is minor units (centicredits) and integer: each of the daily cost's two capital
//! terms rounds up to the minor unit, and a fee or a floor rounds up to the **whole credit**, the
//! grain a quote prints. The mills are outside the model (Q-39-32): a mill is a house process
//! whose margin is its ratio, so a mill row carries no cost.

use serde::{Deserialize, Serialize};

/// Minor units in one credit (the credit currency's scale 2).
pub const MINOR_PER_CREDIT: u64 = 100;
/// IC seconds in one IC hour: one work unit.
pub const SECS_PER_WORK_UNIT: u64 = 3_600;
/// Work units in one IC day.
pub const WORK_UNITS_PER_DAY: u64 = 24;
/// Days in a year of the lifetime and upkeep terms.
pub const DAYS_PER_YEAR: u64 = 365;

/// The largest capital one facility may author, credits.
pub const MAX_CAPITAL_MAJOR: u64 = 10_000_000_000;
/// The longest lifetime, years.
pub const MAX_LIFETIME_YEARS: u32 = 200;
/// The most staff one facility may author.
pub const MAX_STAFF: u32 = 10_000;
/// The largest daily wage a head, credits.
pub const MAX_WAGE_MAJOR: u64 = 100_000;
/// The largest margin, per mille (5×).
pub const MAX_MARGIN_PERMILLE: u32 = 5_000;

/// **A facility's cost inputs** (§4.4): lane data, provisional under the owner's economy rule
/// of 2026-10-24.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FacilityCostDef {
    /// What the facility cost to build, credits.
    pub capital_major: u64,
    /// Years the capital is written off over.
    pub lifetime_years: u32,
    /// Upkeep a year, per mille of the capital.
    pub upkeep_permille: u32,
    /// Heads on the payroll.
    pub staff: u32,
    /// One head's wage an IC day, credits.
    pub wage_major: u64,
    /// The operator's margin on the fee, per mille.
    pub margin_permille: u32,
}

impl FacilityCostDef {
    /// The first field out of bounds with its value and its allowed range, if any.
    fn out_of_bounds(&self) -> Option<(&'static str, u64, u64, u64)> {
        let rows: [(&'static str, u64, u64, u64); 6] = [
            ("capital_major", self.capital_major, 1, MAX_CAPITAL_MAJOR),
            (
                "lifetime_years",
                u64::from(self.lifetime_years),
                1,
                u64::from(MAX_LIFETIME_YEARS),
            ),
            ("upkeep_permille", u64::from(self.upkeep_permille), 0, 1_000),
            ("staff", u64::from(self.staff), 0, u64::from(MAX_STAFF)),
            ("wage_major", self.wage_major, 0, MAX_WAGE_MAJOR),
            (
                "margin_permille",
                u64::from(self.margin_permille),
                0,
                u64::from(MAX_MARGIN_PERMILLE),
            ),
        ];
        rows.into_iter()
            .find(|(_, value, min, max)| !(*min..=*max).contains(value))
    }

    /// **The bounds** an authored row must meet.
    ///
    /// # Errors
    ///
    /// The sentence naming the field out of bounds.
    pub fn check(&self) -> Result<(), String> {
        match self.out_of_bounds() {
            Some((field, value, min, max)) => {
                Err(format!("cost: {field} must be {min}..={max}, not {value}"))
            }
            None => Ok(()),
        }
    }

    /// The first field out of bounds, if any: [`Self::check`]'s answer without the sentence.
    #[must_use]
    pub fn bad_field(&self) -> Option<&'static str> {
        self.out_of_bounds().map(|(field, ..)| field)
    }

    /// **The daily cost**, minor units: the capital's write-off and upkeep (each rounded up to
    /// the minor unit) plus the payroll.
    #[must_use]
    pub fn daily_cost_minor(&self) -> u64 {
        let capital = u128::from(self.capital_major) * u128::from(MINOR_PER_CREDIT);
        let lifetime_days = u128::from(self.lifetime_years) * u128::from(DAYS_PER_YEAR);
        let write_off = capital.div_ceil(lifetime_days.max(1));
        let upkeep = (capital * u128::from(self.upkeep_permille))
            .div_ceil(1_000 * u128::from(DAYS_PER_YEAR));
        let payroll =
            u128::from(self.staff) * u128::from(self.wage_major) * u128::from(MINOR_PER_CREDIT);
        u64::try_from(write_off + upkeep + payroll).unwrap_or(u64::MAX)
    }

    /// **The slot-hour**, minor units, exact as a fraction: `(numerator, denominator)` with the
    /// daily cost over `slots × 24`. Callers multiply before they divide.
    #[must_use]
    pub fn slot_hour_fraction(&self, slots: u32) -> (u128, u128) {
        (
            u128::from(self.daily_cost_minor()),
            u128::from(slots.max(1)) * u128::from(WORK_UNITS_PER_DAY),
        )
    }

    /// **The slot-hour rounded up to the minor unit**, for printing.
    #[must_use]
    pub fn slot_hour_minor(&self, slots: u32) -> u64 {
        let (num, den) = self.slot_hour_fraction(slots);
        u64::try_from(num.div_ceil(den)).unwrap_or(u64::MAX)
    }

    /// **The fee for `work_units`** in one of `slots` slots at `margin_permille`, minor units,
    /// rounded up to the whole credit.
    #[must_use]
    pub fn fee_minor_at(&self, work_units: u64, slots: u32, margin_permille: u32) -> u64 {
        let (num, den) = self.slot_hour_fraction(slots);
        let raw = (u128::from(work_units) * num * (1_000 + u128::from(margin_permille)))
            .div_ceil(den * 1_000);
        whole_credit(raw)
    }

    /// **`fee(order)`** (§4.4): `work_units` at the authored margin.
    #[must_use]
    pub fn fee_minor(&self, work_units: u64, slots: u32) -> u64 {
        self.fee_minor_at(work_units, slots, self.margin_permille)
    }

    /// **The floor for `secs` of one slot's time** (margin 0), minor units, rounded up to the
    /// whole credit: what the operator must not charge under (Q-39-9).
    #[must_use]
    pub fn floor_minor_for_secs(&self, secs: u64, slots: u32) -> u64 {
        let (num, den) = self.slot_hour_fraction(slots);
        let raw = (u128::from(secs) * num).div_ceil(den * u128::from(SECS_PER_WORK_UNIT));
        whole_credit(raw)
    }

    /// **The lowest multiplier, per mille, at which `hour_minor` of labour an hour clears the
    /// slot-hour** (Q-39-9's floor, the validator's clamp). `hour_minor` is the schedule's own
    /// labour hour (the workshop tariff's 100 cr).
    #[must_use]
    pub fn floor_multiplier_permille(&self, slots: u32, hour_minor: u64) -> u32 {
        let (num, den) = self.slot_hour_fraction(slots);
        let permille = (num * 1_000).div_ceil(den * u128::from(hour_minor.max(1)));
        u32::try_from(permille).unwrap_or(u32::MAX)
    }
}

/// `minor` rounded up to the whole credit, saturating.
fn whole_credit(minor: u128) -> u64 {
    let credit = u128::from(MINOR_PER_CREDIT);
    u64::try_from(minor.div_ceil(credit) * credit).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn works() -> FacilityCostDef {
        FacilityCostDef {
            capital_major: 3_000_000,
            lifetime_years: 25,
            upkeep_permille: 20,
            staff: 8,
            wage_major: 150,
            margin_permille: 250,
        }
    }

    /// §4.4's table: the yard ≈ 3,300, a works ≈ 1,700, a workshop ≈ 1,250, the recovery
    /// ≈ 2,100 a day (the doc rounds; these are the exact figures).
    #[test]
    fn the_daily_costs_are_section_4_4s() {
        let yard = FacilityCostDef {
            capital_major: 10_000_000,
            lifetime_years: 30,
            staff: 12,
            ..works()
        };
        assert_eq!(yard.daily_cost_minor(), 326_120);
        assert_eq!(works().daily_cost_minor(), 169_316);
        let workshop = FacilityCostDef {
            capital_major: 1_500_000,
            lifetime_years: 20,
            upkeep_permille: 30,
            staff: 6,
            ..works()
        };
        assert_eq!(workshop.daily_cost_minor(), 122_877);
        let recovery = FacilityCostDef {
            capital_major: 4_000_000,
            lifetime_years: 15,
            upkeep_permille: 40,
            staff: 6,
            ..works()
        };
        assert_eq!(recovery.daily_cost_minor(), 206_896);
    }

    /// The customer's fittings order at the Stickney works: six work units in one of two slots
    /// at 25 % is 264.56 cr, billed 265 (§4.4).
    #[test]
    fn a_six_unit_fittings_order_is_265_credits() {
        assert_eq!(works().fee_minor(6, 2), 26_500);
        assert_eq!(works().fee_minor_at(6, 2, 0), 21_200);
    }

    /// The workshop's berth-hour floors: 256 ‰ of the tariff's hour at Ceres (2 berths), 128 at
    /// Stickney (4).
    #[test]
    fn the_floor_multiplier_is_the_berth_hour_over_the_tariffs_hour() {
        let workshop = FacilityCostDef {
            capital_major: 1_500_000,
            lifetime_years: 20,
            upkeep_permille: 30,
            staff: 6,
            ..works()
        };
        assert_eq!(workshop.floor_multiplier_permille(2, 10_000), 256);
        assert_eq!(workshop.floor_multiplier_permille(4, 10_000), 128);
        assert_eq!(workshop.slot_hour_minor(2), 2_560);
        assert_eq!(workshop.floor_minor_for_secs(3_600, 2), 2_600);
    }

    #[test]
    fn the_bounds_refuse_a_zero_lifetime() {
        assert!(works().check().is_ok());
        let bad = FacilityCostDef {
            lifetime_years: 0,
            ..works()
        };
        assert!(bad.check().unwrap_err().contains("lifetime_years"));
    }
}
