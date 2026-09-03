//! Ruleset-tunable economy limits (E4b1 verify fold, M1).
//!
//! The economy's hard invariants are not configurable — conservation, the durable
//! ack gate, who may spend an org's money. What *is* configurable is the size of the
//! hole a ruleset is allowed to open in the mint: E5's income table funds careers and
//! stipends from a registered faucet, and a faucet-funded obligation is a **standing
//! mint** that the scheduler executes forever with nobody in the loop.
//!
//! So the shape here is deliberately narrow: one cap, in **major** units (the number
//! a game-runner actually thinks in — "50,000 dollars a month", not "5,000,000
//! cents"), with a per-instrument override for worlds that run several currencies at
//! wildly different denominations. The cap is read **live at every settlement**, not
//! captured at creation, so lowering it in the config takes effect on the next sweep
//! and a record edited above it is refused rather than paid.
//!
//! Absent configuration is the default, everywhere: a server with no `[economy]`
//! section gets [`DEFAULT_MINT_OBLIGATION_CAP_MAJOR`] and nothing else changes.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::amount::Amount;
use crate::ids::InstrumentId;
use crate::snapshot::EconomySnapshot;

/// The default per-period ceiling on a faucet-funded obligation, in **major** units.
///
/// 50,000 is chosen to be generous for any legitimate income line and small enough
/// that a typo or a runaway ruleset cannot mint a world-breaking sum before anyone
/// notices — a bound on the blast radius, not a balance decision.
pub const DEFAULT_MINT_OBLIGATION_CAP_MAJOR: u64 = 50_000;

/// Per-instrument overrides of the crate-wide limits.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstrumentCaps {
    /// This instrument's per-period faucet-obligation ceiling, in major units.
    pub mint_obligation_cap_major: u64,
}

impl Default for InstrumentCaps {
    fn default() -> InstrumentCaps {
        InstrumentCaps { mint_obligation_cap_major: DEFAULT_MINT_OBLIGATION_CAP_MAJOR }
    }
}

/// The economy's configurable limits, wired at server start
/// (`EconomyModule::with_config`) and read live thereafter.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EconomyConfig {
    /// The per-period ceiling on a faucet-funded obligation, in major units, for
    /// every instrument without an override.
    pub mint_obligation_cap_major: u64,
    /// Per-instrument overrides, keyed by instrument code (`wod.usd`).
    pub instruments: BTreeMap<String, InstrumentCaps>,
}

impl Default for EconomyConfig {
    fn default() -> EconomyConfig {
        EconomyConfig {
            mint_obligation_cap_major: DEFAULT_MINT_OBLIGATION_CAP_MAJOR,
            instruments: BTreeMap::new(),
        }
    }
}

impl EconomyConfig {
    /// The cap for `instrument`, in the major units the config is written in.
    #[must_use]
    pub fn cap_major(&self, instrument: &InstrumentId) -> u64 {
        self.instruments
            .get(instrument.as_str())
            .map_or(self.mint_obligation_cap_major, |c| c.mint_obligation_cap_major)
    }

    /// The cap for `instrument` as a **minor-unit** [`Amount`], scaled by that
    /// instrument's own `scale`.
    ///
    /// `None` when the instrument is unknown to the snapshot, or when the scaled
    /// value does not fit `i64` — and `None` **refuses**: an unrepresentable cap is
    /// not an unlimited one. A settlement whose cap cannot be computed is a
    /// settlement that does not happen.
    #[must_use]
    pub fn mint_cap(&self, snapshot: &EconomySnapshot, instrument: &InstrumentId) -> Option<Amount> {
        let scale = snapshot.instrument(instrument)?.scale;
        let factor = 10_i128.checked_pow(u32::from(scale))?;
        let minor = i128::from(self.cap_major(instrument)).checked_mul(factor)?;
        Amount::from_widened(minor)
    }
}
