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
//!
//! SP1 (D-LBK-7 as decided, 08h §2 Q6) adds the second knob: `[economy.cadences]`,
//! the bounded list of **declared** cadence clocks the economy registers at
//! composition as `ClockRef::Registered { owner: economy, id: cadence.<name> }`. The
//! list is validated whole ([`EconomyConfig::check_declared_cadences`]) — a bad name
//! or a 33rd clock refuses to start rather than registering a clock nobody can read.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CanonicalIdError};
use serde::Deserialize;
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::clock::{CadenceSource, DEFAULT_CATCH_UP, MAX_CATCH_UP_CAP};
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
        InstrumentCaps {
            mint_obligation_cap_major: DEFAULT_MINT_OBLIGATION_CAP_MAJOR,
        }
    }
}

/// The most cadence clocks `[economy.cadences]` may declare — the world's own
/// cadence-table bound ([`crate::MAX_CADENCES`]), so a fully declared world still
/// fits and a 33rd declaration is refused at configuration, not at boot.
pub const MAX_DECLARED_CADENCES: usize = crate::api::MAX_CADENCES;

/// The source a **declared** cadence advances from (SP1, D-LBK-7).
///
/// Deliberately narrower than [`CadenceSource`]: a manual clock is a human's to
/// step, and a configuration file has no hand to step it with, so `manual` is not a
/// word here — a manual campaign clock stays a runtime cadence.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclaredCadenceClock {
    /// The world's in-character clock (`clock = "world_ic"`, the default).
    #[default]
    WorldIc,
    /// Wall-clock seconds (`clock = "real_utc"`).
    RealUtc,
}

impl DeclaredCadenceClock {
    /// Parse the configuration word (`world_ic` | `real_utc`, case-insensitive).
    #[must_use]
    pub fn parse(word: &str) -> Option<DeclaredCadenceClock> {
        match word.trim().to_ascii_lowercase().as_str() {
            "world_ic" => Some(DeclaredCadenceClock::WorldIc),
            "real_utc" => Some(DeclaredCadenceClock::RealUtc),
            _ => None,
        }
    }

    /// The configuration word.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            DeclaredCadenceClock::WorldIc => "world_ic",
            DeclaredCadenceClock::RealUtc => "real_utc",
        }
    }

    /// The [`CadenceSource`] a cadence created from this declaration advances from.
    #[must_use]
    pub const fn source(self) -> CadenceSource {
        match self {
            DeclaredCadenceClock::WorldIc => CadenceSource::WorldIc,
            DeclaredCadenceClock::RealUtc => CadenceSource::RealUtc,
        }
    }

    /// The declaration word for a persisted source, `None` for a manual clock (which
    /// no declaration can express — see the type's note).
    #[must_use]
    pub const fn of_source(source: CadenceSource) -> Option<DeclaredCadenceClock> {
        match source {
            CadenceSource::WorldIc => Some(DeclaredCadenceClock::WorldIc),
            CadenceSource::RealUtc => Some(DeclaredCadenceClock::RealUtc),
            CadenceSource::Manual => None,
        }
    }
}

fn default_declared_catch_up() -> u8 {
    DEFAULT_CATCH_UP
}

/// One `[economy.cadences.<name>]` stanza, typed (SP1, D-LBK-7).
///
/// The defaults are [`crate::create_cadence`]'s: `catch_up` is
/// [`DEFAULT_CATCH_UP`] when omitted. `period_secs` has no default — a clock with
/// no period is a typo, not a clock — and is validated positive by
/// [`EconomyConfig::check_declared_cadences`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredCadence {
    /// What the clock advances from.
    #[serde(default)]
    pub clock: DeclaredCadenceClock,
    /// The period length in `clock` units (seconds), `> 0`.
    pub period_secs: i64,
    /// How many periods one advance may cover, `1..=`[`MAX_CATCH_UP_CAP`].
    #[serde(default = "default_declared_catch_up")]
    pub catch_up: u8,
}

/// Why `[economy.cadences]` was refused whole.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeclaredCadenceError {
    /// More than [`MAX_DECLARED_CADENCES`] stanzas.
    #[error("[economy.cadences] declares {count} clocks; the most a world carries is {max}")]
    TooMany { count: usize, max: usize },
    /// A name that is not a cadence name (1..=[`crate::MAX_CADENCE_NAME_BYTES`]
    /// bytes) **and** a canonical clock id (`[a-z0-9_.]`, no empty segment).
    #[error(
        "[economy.cadences.{name:?}]: {reason} (a declared cadence is named in lowercase \
         [a-z0-9_.] with no empty segment, at most {max} bytes)"
    )]
    Name {
        name: String,
        reason: String,
        max: usize,
    },
    /// A non-positive period.
    #[error("[economy.cadences.{name:?}].period_secs must be a positive number of seconds")]
    Period { name: String },
    /// A catch-up outside `1..=`[`MAX_CATCH_UP_CAP`].
    #[error("[economy.cadences.{name:?}].catch_up must be 1..={max}")]
    CatchUp { name: String, max: u8 },
}

/// The registered clock id for a declared cadence: `cadence.<name>`, parsed
/// **canonically** (no silent lowercasing — a configuration name must already be
/// the id it registers under).
///
/// # Errors
///
/// The name is not canonical-id shaped.
pub fn declared_cadence_clock_id(name: &str) -> Result<CanonicalId, CanonicalIdError> {
    CanonicalId::parse_canonical(format!("cadence.{name}"))
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
    /// The **declared** cadence clocks (SP1, D-LBK-7), keyed by cadence name. Each is
    /// registered at composition under `cadence.<name>` and adopted or created at
    /// boot; see [`crate::EconomyModule`]. Validated whole by
    /// [`EconomyConfig::check_declared_cadences`], which composition runs.
    pub cadences: BTreeMap<SmolStr, DeclaredCadence>,
    /// How long a **live** contract may go with no condition progress before the
    /// sweep pages a human, in seconds (E6a step 1, DL-08; `[economy]
    /// stale_contract_age_secs`). Default [`crate::STALE_CONTRACT_AGE_SECS`] (7 days).
    /// A contract whose own deadline has not lapsed is never paged for staleness —
    /// its window is the parties' stated patience, and a 14-day run is not stuck on
    /// day 7. A non-positive value pages nothing.
    pub stale_contract_age_secs: i64,
    /// How many of the newest journal transactions stay in RAM as full records
    /// (doc 12 §5.1, J2; `[economy] tx_tail_max`). Everything older is reachable
    /// by id through the journal-derived index and by page through the store-read
    /// seam; nothing is forgotten, only its body leaves RAM. Default
    /// [`crate::DEFAULT_TX_TAIL_MAX`]; the sweep evicts down to it, never the post
    /// path. Zero means "the default".
    pub tx_tail_max: usize,
}

impl Default for EconomyConfig {
    fn default() -> EconomyConfig {
        EconomyConfig {
            mint_obligation_cap_major: DEFAULT_MINT_OBLIGATION_CAP_MAJOR,
            instruments: BTreeMap::new(),
            cadences: BTreeMap::new(),
            stale_contract_age_secs: crate::contract::STALE_CONTRACT_AGE_SECS,
            tx_tail_max: crate::module::DEFAULT_TX_TAIL_MAX,
        }
    }
}

impl EconomyConfig {
    /// Validate `[economy.cadences]` whole: the count, every name (a cadence name
    /// that is also a canonical clock id), every period and every catch-up.
    ///
    /// The host runs this while converting its raw config (so a bad stanza refuses to
    /// start), and `EconomyModule::register` runs it again before registering a
    /// single provider (so no other composition path can slip one past it).
    ///
    /// # Errors
    ///
    /// The first stanza that is not acceptable, in name order.
    pub fn check_declared_cadences(&self) -> Result<(), DeclaredCadenceError> {
        if self.cadences.len() > MAX_DECLARED_CADENCES {
            return Err(DeclaredCadenceError::TooMany {
                count: self.cadences.len(),
                max: MAX_DECLARED_CADENCES,
            });
        }
        for (name, decl) in &self.cadences {
            let bad = |reason: String| DeclaredCadenceError::Name {
                name: name.to_string(),
                reason,
                max: crate::api::MAX_CADENCE_NAME_BYTES,
            };
            if name.trim() != name.as_str() {
                return Err(bad("the name carries surrounding whitespace".to_string()));
            }
            if name.is_empty() || name.len() > crate::api::MAX_CADENCE_NAME_BYTES {
                return Err(bad(format!("the name is {} bytes long", name.len())));
            }
            declared_cadence_clock_id(name)
                .map_err(|e| bad(format!("the name is not a canonical clock id: {e}")))?;
            if decl.period_secs <= 0 {
                return Err(DeclaredCadenceError::Period {
                    name: name.to_string(),
                });
            }
            if !(1..=MAX_CATCH_UP_CAP).contains(&decl.catch_up) {
                return Err(DeclaredCadenceError::CatchUp {
                    name: name.to_string(),
                    max: MAX_CATCH_UP_CAP,
                });
            }
        }
        Ok(())
    }

    /// The declaration for a cadence **name**, matched the way the cadence table
    /// matches names (case-insensitively) — `None` for a runtime cadence.
    #[must_use]
    pub fn declared_cadence(&self, name: &str) -> Option<(&SmolStr, &DeclaredCadence)> {
        self.cadences
            .iter()
            .find(|(declared, _)| declared.eq_ignore_ascii_case(name))
    }

    /// The cap for `instrument`, in the major units the config is written in.
    #[must_use]
    pub fn cap_major(&self, instrument: &InstrumentId) -> u64 {
        self.instruments
            .get(instrument.as_str())
            .map_or(self.mint_obligation_cap_major, |c| {
                c.mint_obligation_cap_major
            })
    }

    /// The cap for `instrument` as a **minor-unit** [`Amount`], scaled by that
    /// instrument's own `scale`.
    ///
    /// `None` when the instrument is unknown to the snapshot, or when the scaled
    /// value does not fit `i64` — and `None` **refuses**: an unrepresentable cap is
    /// not an unlimited one. A settlement whose cap cannot be computed is a
    /// settlement that does not happen.
    #[must_use]
    pub fn mint_cap(
        &self,
        snapshot: &EconomySnapshot,
        instrument: &InstrumentId,
    ) -> Option<Amount> {
        let scale = snapshot.instrument(instrument)?.scale;
        let factor = 10_i128.checked_pow(u32::from(scale))?;
        let minor = i128::from(self.cap_major(instrument)).checked_mul(factor)?;
        Amount::from_widened(minor)
    }
}
