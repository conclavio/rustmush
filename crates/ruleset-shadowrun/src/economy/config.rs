//! `[shadowrun.economy]` as the ruleset sees it (E6a, 08f §9, answer 16).
//!
//! Parsed **host-side** — `ruleset-shadowrun` has no `mush-config` dependency, the
//! same arrangement `[wod.economy]` has (E5c, D-E5-9) — and handed to
//! [`ShadowrunRuleset::economy`](crate::ShadowrunRuleset::economy) at wiring, where
//! it is frozen for the life of the process. Nothing here is read from a document
//! and no command writes it: flipping a switch is editing `mushd.toml` and
//! restarting, followed by the one-shot `@sr/migrate commit` reconciliation.
//!
//! The default is **`enabled = false`**, which is exactly the pre-E6a server: SR5
//! rolls dice, `nuyen` is a sheet number, and the economic layer registers its verbs
//! but creates nothing, evaluates nothing and shows nothing (E5c deviation 1's
//! shape).
//!
//! ## The partial-switch table (08f §9, G9)
//!
//! Every switch says what it *stops*, and no switch ever touches an account, a
//! balance or a live contract:
//!
//! | switch | `false` stops | never touches |
//! |---|---|---|
//! | `enabled` | creates nothing, evaluates nothing | existing accounts, balances, live contracts and obligations |
//! | `player_run_offers` | `+run/post` from a personal account | listings by orgs or staff; live runs |
//! | `organization_run_offers` | `scope=` posts | live org runs — they settle |
//! | `lifestyle` | no lifestyle obligation is created; existing ones are cancelled by `@sr/migrate`, never by boot | accounts, balances, arrears already posted |
//! | `chargen_shopping` | parsed, inert until E6c | — |
//! | `fence_rate_*` | parsed, inert until E6b | — |

use std::collections::BTreeMap;

use smol_str::SmolStr;

/// The instrument E6a registers (answer 1, D-E6-1: scale 0 — a nuyen has no cents).
pub const SR_NUYEN: &str = "sr.nuyen";
/// The instrument's minor-unit scale.
pub const SR_NUYEN_SCALE: u8 = 0;
/// The instrument's rendered symbol (E2-C16's [`InstrumentDisplay`], prefix).
///
/// [`InstrumentDisplay`]: mush_economy::InstrumentDisplay
pub const SR_NUYEN_SYMBOL: &str = "\u{a5}";

/// The faucet a starting runner's priority allotment is minted from (answer 17).
pub const FAUCET_CHARGEN: &str = "sr.chargen";
/// The faucet `@sr/migrate commit` mints an existing sheet's balance from (answer 18).
pub const FAUCET_MIGRATION: &str = "sr.migration";
/// The sink lifestyle upkeep is paid into (D-E6-3).
pub const SINK_LIFESTYLE: &str = "sr.lifestyle";

/// The cadence clock lifestyle is assessed on, by name (§9; a **declared** cadence
/// since SP1 — the host contributes the stanza, the economy creates or adopts it).
pub const SR_MONTH: &str = "sr.month";
/// `sr.month`'s declared period: thirty IC days (§11 q1).
pub const SR_MONTH_PERIOD_SECS: i64 = 30 * 86_400;
/// `sr.month`'s declared clock-level catch-up cap (§11 q1) — it *forgives* on the
/// clock. Not to be confused with [`LIFESTYLE_CATCH_UP`], which decides *attempts*.
pub const SR_MONTH_CATCH_UP: u8 = 3;

/// The lifestyle schedule's own `maximum_catch_up` (§9, M11, D-E6-23): one fire is
/// one attempt, so a downtime backlog can never be the *second* miss.
pub const LIFESTYLE_CATCH_UP: u8 = 1;
/// How many missed periods a lifestyle obligation tolerates before it is delinquent.
///
/// One: the first miss is `Grace` (a private notice, no downgrade), the second is
/// `Delinquent` and the record is replaced one tier down (§3.2 "Life").
pub const LIFESTYLE_GRACE_PERIODS: u8 = 1;

/// The most open run listings one principal may hold (§9).
pub const MAX_OPEN_RUNS_PER_PRINCIPAL: usize = 4;
/// The most open listings the whole board may hold (§9).
pub const BOARD_CAP: usize = 128;
/// The most listings one `+run/list` page renders (§9).
pub const BOARD_PAGE: usize = 20;
/// How long a listing lives — **the contract's own `consent.deadline` on `RealUtc`**
/// (§9, M8): nothing else expires an `Offered` contract holding escrow.
pub const BOARD_TTL_SECS: i64 = 14 * 86_400;
/// The most outcome rows one run may carry (§9; inside E4's `MAX_OUTCOMES` 8).
pub const MAX_RUN_OUTCOMES: usize = 5;
/// The longest run name a listing stores.
pub const MAX_RUN_NAME_BYTES: usize = 48;
/// The most runners one run may seat (E4's `MAX_PARTIES` bounds the contract; this
/// bounds the board's own `slots`, and the party's own cap bounds the roster).
pub const MAX_RUN_SLOTS: u8 = 8;
/// The most parties that may be waiting on one listing's answer at a time.
pub const MAX_RUN_APPLICANTS: usize = 8;

/// The lifestyle tiers, low to high (`crates/ruleset-shadowrun/src/data.rs` —
/// `LIFESTYLES`), as the economy layer orders them.
///
/// The order is the *downgrade ladder*: a second consecutive miss moves one step
/// left, and `street` is the floor (D-E6-3).
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
#[serde(rename_all = "snake_case")]
pub enum LifestyleTier {
    /// No lifestyle at all — the floor, and always free.
    #[default]
    Street,
    /// Squatter.
    Squatter,
    /// Low.
    Low,
    /// Middle.
    Middle,
    /// High.
    High,
    /// Luxury.
    Luxury,
}

impl LifestyleTier {
    /// Every tier, low to high.
    pub const ALL: [LifestyleTier; 6] = [
        LifestyleTier::Street,
        LifestyleTier::Squatter,
        LifestyleTier::Low,
        LifestyleTier::Middle,
        LifestyleTier::High,
        LifestyleTier::Luxury,
    ];

    /// The sheet / config word (`data::LIFESTYLES`' id).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            LifestyleTier::Street => "street",
            LifestyleTier::Squatter => "squatter",
            LifestyleTier::Low => "low",
            LifestyleTier::Middle => "middle",
            LifestyleTier::High => "high",
            LifestyleTier::Luxury => "luxury",
        }
    }

    /// Parse a config / sheet / command word, case-insensitively.
    #[must_use]
    pub fn parse(s: &str) -> Option<LifestyleTier> {
        let s = s.trim().to_ascii_lowercase();
        LifestyleTier::ALL.into_iter().find(|t| t.label() == s)
    }

    /// One step down the ladder; `street` is the floor and returns itself.
    #[must_use]
    pub fn downgraded(self) -> LifestyleTier {
        match self {
            LifestyleTier::Street | LifestyleTier::Squatter => LifestyleTier::Street,
            LifestyleTier::Low => LifestyleTier::Squatter,
            LifestyleTier::Middle => LifestyleTier::Low,
            LifestyleTier::High => LifestyleTier::Middle,
            LifestyleTier::Luxury => LifestyleTier::High,
        }
    }
}

/// The default SR5 monthly lifestyle costs, in **major** units (nuyen).
///
/// `street` is free, which is what makes it the floor: a downgrade always terminates
/// at a tier that creates no obligation.
#[must_use]
pub fn default_lifestyle_costs() -> BTreeMap<LifestyleTier, u64> {
    BTreeMap::from([
        (LifestyleTier::Street, 0),
        (LifestyleTier::Squatter, 500),
        (LifestyleTier::Low, 2_000),
        (LifestyleTier::Middle, 5_000),
        (LifestyleTier::High, 10_000),
        (LifestyleTier::Luxury, 100_000),
    ])
}

/// The frozen `[shadowrun.economy]` block (answer 16, §9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SrEconomyConfig {
    /// Whether the layer exists at all. `false` registers the verbs and creates
    /// nothing (E5c deviation 1).
    pub enabled: bool,
    /// The instrument code (`sr.nuyen`).
    pub instrument: SmolStr,
    /// The **name** of the cadence clock lifestyle is assessed on (`sr.month`).
    pub lifestyle_clock: SmolStr,
    /// Whether lifestyle obligations are created at all.
    pub lifestyle: bool,
    /// Whether `+run/post` accepts a personal payer.
    pub player_run_offers: bool,
    /// Whether `+run/post … scope=<org>` is accepted.
    pub organization_run_offers: bool,
    /// Parsed, inert until E6c (§11 q9).
    pub chargen_shopping: bool,
    /// Parsed, inert until E6c.
    pub catalog_paths: Vec<String>,
    /// Per-tier monthly cost, in **major** units.
    pub lifestyle_costs: BTreeMap<LifestyleTier, u64>,
    /// Parsed, inert until E6c (`QUOTE_TTL_LIVE`).
    pub quote_ttl_live: i64,
    /// Parsed, inert until E6c (`QUOTE_TTL_REMOTE`).
    pub quote_ttl_remote: i64,
    /// Permille of the catalog baseline an immediate fence pays — parsed, inert
    /// until E6b (§11 q6).
    pub fence_rate_immediate: u32,
    /// Permille a consignment fence pays — parsed, inert until E6b.
    pub fence_rate_consign: u32,
    /// The consignment delay in seconds — parsed, inert until E6b.
    pub fence_delay_secs: i64,
    /// The per-principal open-listing cap (§9's `MAX_OPEN_RUNS_PER_PRINCIPAL`).
    pub active_offer_cap: usize,
    /// How long a listing lives, in seconds — the contract's own deadline (M8).
    pub board_ttl_secs: i64,
    /// The whole board's open-listing cap.
    pub board_cap: usize,
    /// Parsed and **unapplied** until shopping ships (answer 17).
    pub chargen_carryover_cap: u64,
}

impl Default for SrEconomyConfig {
    fn default() -> SrEconomyConfig {
        SrEconomyConfig {
            enabled: false,
            instrument: SmolStr::new_static(SR_NUYEN),
            lifestyle_clock: SmolStr::new_static(SR_MONTH),
            lifestyle: true,
            player_run_offers: true,
            organization_run_offers: true,
            chargen_shopping: false,
            catalog_paths: Vec::new(),
            lifestyle_costs: default_lifestyle_costs(),
            quote_ttl_live: 3_600,
            quote_ttl_remote: 24 * 3_600,
            fence_rate_immediate: 300,
            fence_rate_consign: 600,
            fence_delay_secs: 48 * 3_600,
            active_offer_cap: MAX_OPEN_RUNS_PER_PRINCIPAL,
            board_ttl_secs: BOARD_TTL_SECS,
            board_cap: BOARD_CAP,
            chargen_carryover_cap: 5_000,
        }
    }
}

impl SrEconomyConfig {
    /// This tier's per-period cost in **major** units. `None` for a tier the table
    /// does not name and for a zero cost alike — both mean "no obligation", which is
    /// exactly what a zero-cost tier is (§3.2 "Life": a zero-cost tier creates none).
    #[must_use]
    pub fn cost_major(&self, tier: LifestyleTier) -> Option<u64> {
        self.lifestyle_costs.get(&tier).copied().filter(|n| *n > 0)
    }

    /// Whether the lifestyle half of the layer evaluates anything.
    #[must_use]
    pub fn lifestyle_on(&self) -> bool {
        self.enabled && self.lifestyle
    }

    /// The per-principal open-listing cap, clamped into a sane band so a config typo
    /// cannot lift E4's own `MAX_OPEN_CONTRACTS_PER_ISSUER`.
    #[must_use]
    pub fn open_runs_cap(&self) -> usize {
        self.active_offer_cap
            .clamp(1, mush_economy::MAX_OPEN_CONTRACTS_PER_ISSUER)
    }

    /// The board TTL, clamped to a positive number of seconds.
    #[must_use]
    pub fn board_ttl(&self) -> i64 {
        self.board_ttl_secs.clamp(60, 365 * 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_round_trip_and_downgrade_terminates() {
        for t in LifestyleTier::ALL {
            assert_eq!(LifestyleTier::parse(t.label()), Some(t));
        }
        assert_eq!(LifestyleTier::Luxury.downgraded(), LifestyleTier::High);
        assert_eq!(LifestyleTier::Squatter.downgraded(), LifestyleTier::Street);
        assert_eq!(LifestyleTier::Street.downgraded(), LifestyleTier::Street);
    }

    #[test]
    fn a_zero_cost_tier_has_no_cost() {
        let cfg = SrEconomyConfig::default();
        assert_eq!(cfg.cost_major(LifestyleTier::Street), None);
        assert_eq!(cfg.cost_major(LifestyleTier::Middle), Some(5_000));
    }

    #[test]
    fn the_default_layer_is_off_and_the_caps_are_bounded() {
        let cfg = SrEconomyConfig::default();
        assert!(!cfg.enabled);
        assert!(!cfg.lifestyle_on());
        assert_eq!(cfg.open_runs_cap(), MAX_OPEN_RUNS_PER_PRINCIPAL);
        assert_eq!(cfg.board_ttl(), BOARD_TTL_SECS);
        let wild = SrEconomyConfig {
            active_offer_cap: 9_999,
            board_ttl_secs: -1,
            ..SrEconomyConfig::default()
        };
        assert_eq!(
            wild.open_runs_cap(),
            mush_economy::MAX_OPEN_CONTRACTS_PER_ISSUER
        );
        assert_eq!(wild.board_ttl(), 60);
    }
}
