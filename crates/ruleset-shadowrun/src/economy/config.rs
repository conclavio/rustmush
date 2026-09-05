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
/// The most **condition kinds** one board notice names (E6a verify fold, D8/R6).
///
/// E6a's notice is the constant `["attest"]`; E6b adds `before`/`after`/`fact`. Eight
/// is E4's own `MAX_CONDITIONS`-shaped ceiling and far past any table that will exist.
pub const MAX_NOTICE_KINDS: usize = 8;
/// The most UTF-8 bytes in one notice kind token (E6a verify fold, D8/R6).
pub const MAX_NOTICE_KIND_BYTES: usize = 32;
/// The longest window a persisted notice may claim, in seconds (E6a verify fold, R6).
///
/// One year — the same ceiling [`SrEconomyConfig::board_ttl`] clamps to — so a
/// hand-written document cannot render a listing that closes in the year 300 million.
pub const MAX_RUN_WINDOW_SECS: i64 = 365 * 86_400;
/// The most UTF-8 bytes in a persisted mint's idempotency key (E6a verify fold, D8).
///
/// The keys this ruleset writes are `sr.chargen/{obj}` and `sr.migration/{obj}` — well
/// under 32 bytes — and 128 leaves room for a namespace change without touching
/// the wire.
pub const MAX_MINT_KEY_BYTES: usize = 128;
// ---- E6b: B9 — the fence -------------------------------------------------

/// The longest catalog code the fence baseline table keys on (E6b, OPEN 1).
pub const MAX_CATALOG_CODE_BYTES: usize = 64;
/// The shortest consignment delay the offer may carry (answer 15: 24–72 h).
pub const FENCE_DELAY_MIN_SECS: i64 = 24 * 3_600;
/// The longest one (answer 15).
pub const FENCE_DELAY_MAX_SECS: i64 = 72 * 3_600;
/// How long a fence quote stands before it is refused deterministically
/// (scenario 29). It is on `RealUtc`, and the offer says so.
pub const FENCE_QUOTE_TTL_SECS: i64 = 3_600;
/// The most price factors one quote's basis carries (a bounded record).
pub const MAX_PRICE_FACTORS: usize = 8;
/// The most UTF-8 bytes in one price factor's label.
pub const MAX_PRICE_FACTOR_BYTES: usize = 32;
/// The most fences one server registers, and the most quotes one seller may hold
/// open at once — both bounded, both refused rather than grown.
pub const MAX_FENCES: usize = 64;
/// The most open quotes one seller may hold.
pub const MAX_OPEN_QUOTES_PER_SELLER: usize = 8;
/// The most open quotes the whole store holds (E6b verify fold, D6): the per-seller
/// cap bounds one character, and this bounds the roster.
pub const MAX_QUOTES: usize = 1_024;

/// How many **lapsed, contract-less** quotes one `+fence/quote` clears as it counts
/// (E6b independent verify fold, D4).
///
/// The count of a seller's open quotes is a walk over their own index, so the sweep
/// rides a walk that already had to happen; bounding it keeps the command's cost
/// flat whatever a seller has accumulated, and the next call clears the rest.
pub const MAX_LAPSED_SWEEP_PER_CALL: usize = 32;
/// The most listings or quotes one `+fence/list` page renders.
pub const FENCE_PAGE: usize = 20;
/// The most fence or quote records one `+fence/*` read may **visit** (E6b verify
/// fold, D3/D10; D-LBK-5's rule): a bounded read bounds the records it *touches*,
/// never the ones it *matches*, or a selective filter over a long tail is an
/// unbounded scan wearing a page cap.
pub const FENCE_PAGE_VISIT: usize = 256;
/// The most vaults `owner_home`'s search may visit when routing a deposit home
/// (E6b verify fold, D11).
pub const MAX_HOME_VAULT_VISIT: usize = 256;

/// The most listing documents one board command may **visit** (E6a verify fold, D1).
///
/// D-LBK-5's rule: a bounded read bounds the records it *touches*, never the ones it
/// *matches*, or a selective filter over a long tail is an unbounded scan wearing a
/// page cap. `+run/list` walks at most this many live listings from its cursor and
/// returns at most [`BOARD_PAGE`] of them, with a continuation cursor when it stopped
/// early.
pub const BOARD_PAGE_VISIT: usize = 256;
/// How many just-closed listing ids the board remembers for `+run/show`
/// (E6a verify fold, D1).
///
/// A closed listing's document is deleted — a post/withdraw loop must not mint
/// permanent documents — but somebody who was looking at run #7 when it closed
/// deserves "closed" rather than "No such run.". A 32-deep ring of ids is that grace
/// period, and it costs nothing: an id, not a record.
pub const RECENT_CLOSED_RING: usize = 32;

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
    /// The consignment delay in seconds — clamped to [`FENCE_DELAY_MIN_SECS`] ..=
    /// [`FENCE_DELAY_MAX_SECS`] by [`SrEconomyConfig::fence_delay`] (answer 15).
    pub fence_delay_secs: i64,
    /// The **fence price baseline**, in major units, keyed on `CatalogRef.code`
    /// (E6b, OPEN 1). A code this table does not name cannot be quoted.
    pub fence_baseline: BTreeMap<SmolStr, u64>,
    /// The organization ids whose contract acceptance may be performed
    /// programmatically (E6b, DL-11 / M15) — composition data, ids only.
    pub vendor_orgs: Vec<u64>,
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
            fence_baseline: BTreeMap::new(),
            vendor_orgs: Vec::new(),
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
    /// The consignment delay actually offered: the configured value clamped to
    /// answer 15's 24–72 hours, so a mis-set `mushd.toml` can never offer a fence
    /// deal that matures in a minute or in a decade.
    #[must_use]
    pub fn fence_delay(&self) -> i64 {
        self.fence_delay_secs
            .clamp(FENCE_DELAY_MIN_SECS, FENCE_DELAY_MAX_SECS)
    }

    /// Whether the wiring named this organization as a programmatic vendor (DL-11).
    #[must_use]
    pub fn is_vendor_org(&self, org: mush_org::OrgId) -> bool {
        self.vendor_orgs.contains(&org.0)
    }

    /// The source-controlled baseline for a catalog code, in **minor** units
    /// (E6b, OPEN 1). `None` means the object cannot be quoted at all.
    #[must_use]
    pub fn baseline_minor(&self, code: &str) -> Option<i64> {
        self.fence_baseline
            .get(code)
            .copied()
            .filter(|n| *n > 0)
            .and_then(|n| i64::try_from(n).ok())
    }

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
