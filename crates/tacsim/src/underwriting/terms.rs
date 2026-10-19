use super::CoverageError;
use mush_core::DefinitionDigest;
use mush_economy::{Amount, InstrumentId};
use serde::{Deserialize, Serialize};

/// A price illustration has no actor, quote identity, acceptance or activation
/// method. Both alternatives remain conditional on actual owner admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoverageIllustration {
    insured_value: Amount,
    ordinary: CoverageAmounts,
    safe_conduct: CoverageAmounts,
}
impl CoverageIllustration {
    pub const fn insured_value(&self) -> Amount {
        self.insured_value
    }
    pub const fn ordinary(&self) -> CoverageAmounts {
        self.ordinary
    }
    pub const fn with_qualified_safe_conduct(&self) -> CoverageAmounts {
        self.safe_conduct
    }
}

/// Arithmetic only. Liability must be reserved from the insurer's existing
/// available balance before the premium can become income; a price is no hold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoverageAmounts {
    base_premium: Amount,
    discount: Amount,
    premium: Amount,
    maximum_payout: Amount,
}
impl CoverageAmounts {
    pub const fn base_premium(&self) -> Amount {
        self.base_premium
    }
    pub const fn discount(&self) -> Amount {
        self.discount
    }
    pub const fn premium(&self) -> Amount {
        self.premium
    }
    pub const fn maximum_payout(&self) -> Amount {
        self.maximum_payout
    }
}

/// Exact immutable financial terms, separate from the insurer, approved source,
/// covered zone and exclusion policy that the product owner must also bind.
/// A later product revision cannot mutate a policy's captured terms.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "TermsWire")]
pub struct CoverageTermsV1 {
    format: u16,
    instrument: InstrumentId,
    minimum_value: Amount,
    maximum_value: Amount,
    premium_permille: u16,
    discount_permille: u16,
    payout_permille: u16,
    term_seconds: i64,
    claim_delay_seconds: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TermsWire {
    format: u16,
    instrument: InstrumentId,
    minimum_value: Amount,
    maximum_value: Amount,
    premium_permille: u16,
    discount_permille: u16,
    payout_permille: u16,
    term_seconds: i64,
    claim_delay_seconds: i64,
}

impl TryFrom<TermsWire> for CoverageTermsV1 {
    type Error = CoverageError;
    fn try_from(w: TermsWire) -> Result<Self, Self::Error> {
        let terms = Self {
            format: w.format,
            instrument: w.instrument,
            minimum_value: w.minimum_value,
            maximum_value: w.maximum_value,
            premium_permille: w.premium_permille,
            discount_permille: w.discount_permille,
            payout_permille: w.payout_permille,
            term_seconds: w.term_seconds,
            claim_delay_seconds: w.claim_delay_seconds,
        };
        terms.validate()?;
        Ok(terms)
    }
}

impl CoverageTermsV1 {
    pub const MAX_BYTES: usize = 4096;
    pub const MAX_TERM_SECONDS: i64 = 90 * 86_400;
    pub const MAX_CLAIM_DELAY_SECONDS: i64 = 30 * 86_400;

    /// Provisional M7 tuning. The actual product still chooses its registered
    /// instrument, valuation bounds and duration, and binds a funded insurer.
    pub fn launch(
        instrument: InstrumentId,
        minimum_value: Amount,
        maximum_value: Amount,
        term_seconds: i64,
    ) -> Result<Self, CoverageError> {
        Self::new(
            instrument,
            minimum_value,
            maximum_value,
            10,
            100,
            800,
            term_seconds,
            86_400,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        minimum_value: Amount,
        maximum_value: Amount,
        premium_permille: u16,
        discount_permille: u16,
        payout_permille: u16,
        term_seconds: i64,
        claim_delay_seconds: i64,
    ) -> Result<Self, CoverageError> {
        Self::try_from(TermsWire {
            format: 1,
            instrument,
            minimum_value,
            maximum_value,
            premium_permille,
            discount_permille,
            payout_permille,
            term_seconds,
            claim_delay_seconds,
        })
    }

    fn validate(&self) -> Result<(), CoverageError> {
        if self.format != 1
            || self.instrument.as_str().len() > 128
            || self.minimum_value.minor() <= 0
            || self.maximum_value < self.minimum_value
            || !(1..=1000).contains(&self.premium_permille)
            || self.discount_permille >= 1000
            || !(1..=1000).contains(&self.payout_permille)
            || !(1..=Self::MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=Self::MAX_CLAIM_DELAY_SECONDS).contains(&self.claim_delay_seconds)
        {
            return Err(CoverageError::Terms);
        }
        // All admitted values must quote a positive premium and payout. These
        // monotone calculations at both bounds also reject overflow up front.
        self.illustrate(self.minimum_value)?;
        self.illustrate(self.maximum_value)?;
        self.encoded()?;
        Ok(())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoverageError> {
        if bytes.is_empty() || bytes.len() > Self::MAX_BYTES {
            return Err(CoverageError::Bound);
        }
        serde_json::from_slice(bytes).map_err(|_| CoverageError::Terms)
    }

    fn encoded(&self) -> Result<serde_json::Value, CoverageError> {
        let value = serde_json::to_value(self).map_err(|_| CoverageError::Terms)?;
        mush_core::canonical_json_bytes_with_limit(&value, Self::MAX_BYTES)
            .map_err(|_| CoverageError::Bound)?;
        Ok(value)
    }

    pub fn digest(&self) -> Result<DefinitionDigest, CoverageError> {
        DefinitionDigest::for_canonical_value("tacsim.coverage.terms.v1", &self.encoded()?)
            .map_err(|_| CoverageError::Terms)
    }

    /// Premium rounds upward to the next minor unit; discount and payout round
    /// downward. There are no floating-point amounts, saturation or implied FX.
    /// The returned qualified price is conditional, never an eligibility claim.
    pub fn illustrate(&self, insured_value: Amount) -> Result<CoverageIllustration, CoverageError> {
        if insured_value < self.minimum_value || insured_value > self.maximum_value {
            return Err(CoverageError::Value);
        }
        let base = fraction(insured_value, self.premium_permille, 999)?;
        let discount = fraction(base, self.discount_permille, 0)?;
        let payout = fraction(insured_value, self.payout_permille, 0)?;
        let premium = base
            .checked_sub(discount)
            .ok_or(CoverageError::Arithmetic)?;
        if base.minor() <= 0 || premium.minor() <= 0 || payout.minor() <= 0 {
            return Err(CoverageError::Value);
        }
        Ok(CoverageIllustration {
            insured_value,
            ordinary: CoverageAmounts {
                base_premium: base,
                discount: Amount::ZERO,
                premium: base,
                maximum_payout: payout,
            },
            safe_conduct: CoverageAmounts {
                base_premium: base,
                discount,
                premium,
                maximum_payout: payout,
            },
        })
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub const fn minimum_value(&self) -> Amount {
        self.minimum_value
    }
    pub const fn maximum_value(&self) -> Amount {
        self.maximum_value
    }
    pub const fn term_seconds(&self) -> i64 {
        self.term_seconds
    }
    pub const fn claim_delay_seconds(&self) -> i64 {
        self.claim_delay_seconds
    }
}

fn fraction(amount: Amount, permille: u16, rounding: i128) -> Result<Amount, CoverageError> {
    let value = amount
        .widen()
        .checked_mul(i128::from(permille))
        .and_then(|v| v.checked_add(rounding))
        .ok_or(CoverageError::Arithmetic)?
        / 1000;
    Amount::from_widened(value).ok_or(CoverageError::Arithmetic)
}
