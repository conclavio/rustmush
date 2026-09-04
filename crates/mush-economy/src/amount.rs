//! [`Amount`] — a signed **minor-unit** money value with checked-only arithmetic
//! (E2a, review point R-R3).
//!
//! The whole point of the newtype is that there is **no** `Add`/`Sub`/`Neg` impl:
//! every arithmetic operation is a `checked_*` returning `Option`, so a caller can
//! never silently write `Amount(a.0 + b.0)` and get wrapping arithmetic on money.
//! Saturating arithmetic is forbidden (§6.1) — it would fabricate or destroy value.
//! The inner `i64` is private; construction is explicit ([`Amount::from_minor`]).
//!
//! Authoritative values are signed `i64` minor units, so `Amount` serializes
//! **transparently as that i64** (the journal doc form). The wire form a JS client
//! sees — a decimal string plus a formatted display — is produced only at the
//! render boundary ([`Amount::to_decimal_string`]), never by leaking the raw i64.
//! Validation that must not overflow sums in `i128` via [`Amount::widen`]; a posted
//! line and a projected balance must still fit `i64`.

use serde::{Deserialize, Serialize};

/// A signed money value in an instrument's minor units. Checked arithmetic only.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Amount(i64);

/// The largest instrument decimal scale [`Amount::to_decimal_string`] formats. Real
/// currencies are ≤ ~18 places; E2b's instrument registration should reject a larger
/// scale. Beyond this the render falls back to the raw minor value rather than
/// overflow (`10^scale` leaves `u128` at scale 39).
pub const MAX_SCALE: u8 = 18;

impl Amount {
    /// Zero.
    pub const ZERO: Amount = Amount(0);

    /// Construct from signed minor units (the only constructor).
    #[must_use]
    pub const fn from_minor(minor: i64) -> Amount {
        Amount(minor)
    }

    /// The raw signed minor-unit value. Use this only at a boundary that needs the
    /// integer (persistence is handled by serde; the wire form is
    /// [`Amount::to_decimal_string`]).
    #[must_use]
    pub const fn minor(self) -> i64 {
        self.0
    }

    /// Widen to `i128` for overflow-free intermediate summation in validation.
    #[must_use]
    pub const fn widen(self) -> i128 {
        self.0 as i128
    }

    /// Whether this is exactly zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Whether this is strictly negative.
    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// `-1`, `0`, or `1`.
    #[must_use]
    pub const fn signum(self) -> i64 {
        self.0.signum()
    }

    /// Checked addition; `None` on `i64` overflow (never wraps or saturates).
    #[must_use]
    pub fn checked_add(self, rhs: Amount) -> Option<Amount> {
        self.0.checked_add(rhs.0).map(Amount)
    }

    /// Checked subtraction; `None` on overflow.
    #[must_use]
    pub fn checked_sub(self, rhs: Amount) -> Option<Amount> {
        self.0.checked_sub(rhs.0).map(Amount)
    }

    /// Checked negation; `None` for `i64::MIN` (which has no positive counterpart).
    #[must_use]
    pub fn checked_neg(self) -> Option<Amount> {
        self.0.checked_neg().map(Amount)
    }

    /// Narrow an `i128` intermediate back to an `Amount`; `None` if it does not fit
    /// `i64` (a posted line / projected balance must fit `i64`, §6.1).
    #[must_use]
    pub fn from_widened(v: i128) -> Option<Amount> {
        i64::try_from(v).ok().map(Amount)
    }

    /// The wire form: minor units as a decimal string with `scale` fractional
    /// digits, so a JS client never rounds a large `i64` through an f64. `scale` 0
    /// (indivisible) yields the bare integer. This is the ONLY way an amount should
    /// reach a client (§16 / R-R3).
    #[must_use]
    pub fn to_decimal_string(self, scale: u8) -> String {
        if scale == 0 {
            return self.0.to_string();
        }
        // `10^scale` leaves u128 at scale 39 (and is divisible by 2^128 at scale
        // 128, a divide-by-zero after wrapping). Guard with checked_pow and fall back
        // to the raw minor value — precision-safe, never a panic or garbage divisor.
        let Some(div) = 10u128.checked_pow(u32::from(scale)) else {
            return self.0.to_string();
        };
        let neg = self.0 < 0;
        // Use the absolute value in u128 space so i64::MIN does not overflow negation.
        let abs = (self.0 as i128).unsigned_abs();
        let whole = abs / div;
        let frac = abs % div;
        format!(
            "{}{}.{:0width$}",
            if neg { "-" } else { "" },
            whole,
            frac,
            width = usize::from(scale),
        )
    }
}

impl std::fmt::Debug for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Amount({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_arithmetic_never_wraps() {
        assert_eq!(
            Amount::from_minor(2).checked_add(Amount::from_minor(3)),
            Some(Amount::from_minor(5))
        );
        assert_eq!(
            Amount::from_minor(i64::MAX).checked_add(Amount::from_minor(1)),
            None
        );
        assert_eq!(Amount::from_minor(i64::MIN).checked_neg(), None);
        assert_eq!(
            Amount::from_minor(5).checked_sub(Amount::from_minor(8)),
            Some(Amount::from_minor(-3))
        );
    }

    #[test]
    fn serializes_transparently_as_i64() {
        assert_eq!(
            serde_json::to_string(&Amount::from_minor(-4200)).unwrap(),
            "-4200"
        );
        assert_eq!(
            serde_json::from_str::<Amount>("999").unwrap(),
            Amount::from_minor(999)
        );
    }

    #[test]
    fn decimal_wire_form_keeps_precision() {
        assert_eq!(Amount::from_minor(123456).to_decimal_string(2), "1234.56");
        assert_eq!(Amount::from_minor(-5).to_decimal_string(2), "-0.05");
        assert_eq!(Amount::from_minor(1000).to_decimal_string(0), "1000");
        // i64::MIN must not panic through negation.
        assert!(
            Amount::from_minor(i64::MIN)
                .to_decimal_string(2)
                .starts_with('-')
        );
    }

    #[test]
    fn absurd_scale_falls_back_instead_of_overflowing() {
        // scale >= 39 overflows 10^scale in u128; the render must fall back to the
        // raw minor value, never panic or produce a garbage/zero divisor.
        assert_eq!(Amount::from_minor(42).to_decimal_string(39), "42");
        assert_eq!(Amount::from_minor(-7).to_decimal_string(200), "-7");
    }
}
