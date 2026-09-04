//! `[wod.economy]` as the ruleset sees it (E5c, 08d §5.1, D-E5-9).
//!
//! Parsed **host-side** — `ruleset-wod` has no `mush-config` dependency, exactly as
//! `WodConfig` and [`OnboardingConfig`](crate::OnboardingConfig) are — and handed to
//! [`WodRuleset::economy`](crate::WodRuleset::economy) at wiring, where it is frozen
//! for the life of the process. Nothing here is read from a document, and no command
//! writes it: flipping a mode is editing `mushd.toml` and restarting, followed by the
//! one-shot `@wod/economy migrate` reconciliation (§5.1).
//!
//! The default is [`PersonalMode::Off`] with `housing = true`, which is exactly the
//! pre-E5c server: housing works, staff create tenancies, and the WoD layer creates
//! no profile, evaluates nothing, and shows nothing.

use std::collections::BTreeMap;

use mush_core::ObjId;
use smol_str::SmolStr;

/// The instrument exact mode registers (owner answer 2: scale 2).
pub const WOD_USD: &str = "wod.usd";
/// The instrument's minor-unit scale.
pub const WOD_USD_SCALE: u8 = 2;

/// The faucet every income line is minted from (`create_system_obligation`'s caller
/// principal, 08c §4.7 M1).
pub const FAUCET_INCOME: &str = "wod.income";
/// The housing title issuer (08d §2, D-E3-13) — the same principal
/// `HousingModule::with_issuer` and `EconomyModule::with_title_issuers` carry.
pub const FAUCET_HOUSING: &str = "wod.housing";
/// The principal chargen starting funds are minted from.
pub const FAUCET_CHARGEN: &str = "wod.chargen";

/// How much personal money exists (owner answers 1–3, D-E5-2).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum PersonalMode {
    /// No WoD economic profile at all. The platform economy is untouched; nothing is
    /// evaluated and nothing is deleted.
    #[default]
    Off,
    /// A support **level**, not a wallet. No `wod.usd` account exists or is shown.
    Abstract,
    /// A personal `wod.usd` account, configured starting funds, and a faucet-funded
    /// income obligation per period.
    Exact,
}

impl PersonalMode {
    /// The config word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PersonalMode::Off => "off",
            PersonalMode::Abstract => "abstract",
            PersonalMode::Exact => "exact",
        }
    }

    /// Parse a config word (case-insensitive). `None` is a hard boot error at the
    /// host, never a silent fallback.
    #[must_use]
    pub fn parse(s: &str) -> Option<PersonalMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(PersonalMode::Off),
            "abstract" => Some(PersonalMode::Abstract),
            "exact" => Some(PersonalMode::Exact),
            _ => None,
        }
    }

    /// Whether the layer evaluates anything at all.
    #[must_use]
    pub fn is_on(self) -> bool {
        !matches!(self, PersonalMode::Off)
    }
}

/// How much of the career layer a character wants pushed at them (§5.2).
///
/// `Ambient` is the quiet default: the career still supports the lease, still carries
/// strain, still shows up in `+career` — it simply never opens an opportunity prompt.
/// `Engaged` is opting in to being asked. Housing pressure reaches both (it is not an
/// opportunity, it is somebody's home).
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum Engagement {
    /// Quiet: no opportunity or advancement prompts.
    #[default]
    Ambient,
    /// Ask me: the stance's opportunity row applies.
    Engaged,
}

impl Engagement {
    /// The config / display word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Engagement::Ambient => "ambient",
            Engagement::Engaged => "engaged",
        }
    }

    /// Parse a config word (case-insensitive).
    #[must_use]
    pub fn parse(s: &str) -> Option<Engagement> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ambient" | "off" => Some(Engagement::Ambient),
            "engaged" | "on" => Some(Engagement::Engaged),
            _ => None,
        }
    }
}

/// The frozen `[wod.economy]` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WodEconomyConfig {
    /// `off` | `abstract` | `exact`.
    pub personal_mode: PersonalMode,
    /// Whether the WoD layer creates tenancies, and whether the host registers the
    /// housing title issuer.
    pub housing: bool,
    /// Whether careers, stances, opportunities and exposure are evaluated.
    pub careers: bool,
    /// A new profile's engagement.
    pub default_engagement: Engagement,
    /// The **name** of the cadence clock everything here is assessed on (D-E5-14).
    pub cadence: SmolStr,
    /// Exact mode: chargen starting funds, in major units.
    pub starting_funds_major: u64,
    /// Extra rooms that may never be property rooms (D-E5-12), chained into
    /// `HousingModule::with_reserved_rooms` by the host.
    pub reserved_rooms: Vec<ObjId>,
    /// Exact mode: per-period income per Resources dot (`0..=5`), in **major** units.
    /// Authoritative **only** in exact mode (owner answer 3).
    pub income: BTreeMap<u8, u64>,
}

impl Default for WodEconomyConfig {
    fn default() -> WodEconomyConfig {
        WodEconomyConfig {
            personal_mode: PersonalMode::Off,
            housing: true,
            careers: true,
            default_engagement: Engagement::Ambient,
            cadence: SmolStr::new_static("chronicle.period"),
            starting_funds_major: 0,
            reserved_rooms: Vec::new(),
            income: BTreeMap::new(),
        }
    }
}

impl WodEconomyConfig {
    /// The per-period income for a support level, in major units. `None` outside
    /// exact mode, and `None` for a dot the table does not name — an unnamed dot
    /// means "no stipend", never zero-and-therefore-an-obligation.
    #[must_use]
    pub fn income_major(&self, level: u8) -> Option<u64> {
        if self.personal_mode != PersonalMode::Exact {
            return None;
        }
        self.income.get(&level).copied().filter(|n| *n > 0)
    }

    /// Whether this deployment shows or opens a `wod.usd` account.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.personal_mode == PersonalMode::Exact
    }
}
