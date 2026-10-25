//! **The `port` pack entry** (doc 18 cut IC3; doc 18 §4.7 "The `port` entry",
//! D-18-14, D-18-20, D-18-25, D-18-47).
//!
//! One `port` entry binds one station of the corridor to the world: the lane
//! place whose room hosts the port's passive domain, the station neighborhood
//! every room reference lives in, the berth gallery the domain presents in, the
//! rooms each later cut finds its desk in, the treasury that capitalizes the
//! port's institutions, the institutions themselves, the First Captain programme
//! (Ceres only) and the routes out to every other port. It is **seed-only**: a
//! port instantiates (a domain, orgs, accounts, obligations), so a rules pack
//! may not declare one (D-18-20; [`crate::pack::PackError::WrongKindEntry`]).
//!
//! What reads each field, and when:
//!
//! | field | read by |
//! |---|---|
//! | `location`, `gallery` | the `PortApplier`'s domain (IC3, part a) |
//! | `treasury`, `orgs` | the `PortApplier`'s treasury, orgs and capital (IC3, parts b–c) |
//! | `area`, `rooms` | the readiness report (IC3); the desks of IC6–IC8 |
//! | `programme` | the readiness report (IC3); `+coop/enrol` (IC6), the hulls (IC4), buyout and dormancy (IC13) |
//! | `vendors` | the `PortApplier`'s parts d–e (IC8): vendors, storefront vaults, the clerk, grants, plan rows, boards, Wanted bids; the stock arm |
//! | `routes` | validated here; the passage planner's route list (IC9, D-18-47) |
//! | `venue` | the routes key; the market venue (IC7/IC8) |
//! | `faction` (per org) | nothing until the faction cut (carried and validated, L16) |
//!
//! Every number is an integer and every bound is checked twice: by
//! [`PortDef::check`] (the entry alone) and by [`check_ports`] (the rules that
//! span the pack), both from `RulesPack::from_content`, so a pack that fails
//! one refuses at load naming the port entry and the rule, never at first use.
//!
//! **The reference split rule** (lead ruling L2): every `<pack>.<entry>` string
//! in a port entry — the gallery, each room, the programme's office, a vendor's
//! storefront and signatory — is `"<area>." + entry`. Pack ids and entry ids
//! both contain dots, so the port's own `area` is what makes the split
//! unambiguous, and a reference outside that area is refused ([`room_key`]).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::CanonicalId;
use serde::{Deserialize, Serialize};

use crate::pack::{MAX_LABEL_BYTES, PackEntry};

/// The role words a port's `rooms` map may use (doc 18 §4.7; 18a §0.2). Ceres
/// authors the first six and `spine`, Stickney `customs` to `yard` and `spine`;
/// a later cut that needs another desk adds its word here.
pub const PORT_ROOM_ROLES: [&str; 12] = [
    "arrivals",
    "registry",
    "clearing",
    "exchange",
    "chandlery",
    "dockmaster",
    "spine",
    "customs",
    "harbourmaster",
    "receiving",
    "commissary",
    "yard",
];

/// The most institutions one port entry may author.
pub const MAX_PORT_ORGS: usize = 16;
/// The most routes one port entry may list for one destination (D-18-47: a
/// route list, read by the passage planner of cut IC9).
pub const MAX_PORT_ROUTES: usize = 8;
/// The most vendors one port entry may author (applied from cut IC8).
pub const MAX_PORT_VENDORS: usize = 16;
/// The most stock, ask or bid rows one vendor may author.
pub const MAX_VENDOR_ROWS: usize = 16;
/// The most mill rows one vendor may author (doc 20 cut SM.12a).
pub const MAX_VENDOR_MILLS: usize = 4;
/// The most tonnes of ore (and so of product) one mill ratio may name (doc 20
/// cut SM.12a): `1 <= product <= ore <= MAX_MILL_RATIO`.
pub const MAX_MILL_RATIO: u32 = 100;
/// The most capital one institution may be authored with, in major units.
pub const MAX_PORT_CAPITAL_MAJOR: u64 = 100_000_000;
/// **The largest one capital tranche may be**, in major units (D-18-14): an
/// institution's capital is minted by one-shot treasury obligations of at most
/// this much each ([`tranches`]).
pub const MAX_TRANCHE_MAJOR: u64 = 800_000;
/// **The `tacsim.credit` mint cap the game contributes**, in major units
/// (D-18-14; doc 18 §4.7 "Config flips"): above [`MAX_TRANCHE_MAJOR`], so every
/// tranche fits under it, and above any single drip the game issues.
pub const PORT_MINT_CAP_MAJOR: u64 = 1_000_000;
/// The largest hull pool one programme may author.
pub const MAX_PROGRAMME_POOL: u32 = 16;
/// The longest registry prefix one programme may author, in bytes.
pub const MAX_REGISTRY_PREFIX_BYTES: usize = 16;
/// The longest a programme's dormancy window may be, in real days.
pub const MAX_DORMANCY_REAL_DAYS: u32 = 365;
/// The most refit rows one programme may author (doc 20 cut SM.3a).
pub const MAX_PROGRAMME_REFITS: usize = 4;
/// The refit word that always means the programme's own unit, and is never
/// authored (doc 20 cut SM.3a).
pub const STANDARD_REFIT: &str = "standard";
/// The largest rig pool one lessor may author (doc 20 cut SM.3a).
pub const MAX_VEHICLE_POOL: u32 = 16;
/// The longest rig lease term, IC days (doc 20 cut SM.3a).
pub const MAX_VEHICLE_TERM_IC_DAYS: u32 = 365;
/// The most tools one rig pool may name (doc 20 cut SM.3a).
pub const MAX_VEHICLE_TOOLS: usize = 8;

/// **One `port` entry** (doc 18 §4.7, D-18-20). Strict and integer-only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortDef {
    /// A `location` entry of this pack: the room the port's passive domain is
    /// hosted on (the lane's place room).
    pub location: CanonicalId,
    /// The port's venue word (`ceres`, `stickney`): the key every other port's
    /// `routes` names this one by, and the market venue later (IC7/IC8).
    pub venue: CanonicalId,
    /// The station neighborhood pack every room reference lives in.
    pub area: CanonicalId,
    /// `"<area>.<entry>"`: the room the domain presents in; it must carry the
    /// `port.dock` marker.
    pub gallery: String,
    /// Role word ([`PORT_ROOM_ROLES`]) → `"<area>.<entry>"`. A site authors
    /// none (doc 20 cut SM.2: its pad is its gallery); skipped when empty, and
    /// every station authors some, so no station re-encodes differently.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rooms: BTreeMap<CanonicalId, String>,
    /// A `SystemPrincipalId` (`tacsim.treasury`): the faucet the capital is
    /// minted from. Defaults to `tacsim.treasury` (doc 20 cut SM.2: a site
    /// authors none) and is always written.
    #[serde(default = "default_treasury")]
    pub treasury: String,
    /// The institutions, `1..=`[`MAX_PORT_ORGS`]; authored order is creation
    /// order. A site authors none (doc 20 cut SM.2, §3.1's zero-orgs rule).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub orgs: Vec<PortOrgDef>,
    /// The First Captain programme, where the port runs one (Ceres).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub programme: Option<ProgrammeDef>,
    /// The vendors (signatory required), applied by the `PortApplier`'s parts
    /// d–e from cut IC8: storefront vaults, market vendors, the clerk, its
    /// grants, the stock plan rows, the boards and the standing bids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vendors: Vec<VendorDef>,
    /// Destination venue → a non-empty list of route ids of this pack
    /// (D-18-47).
    pub routes: BTreeMap<CanonicalId, Vec<CanonicalId>>,
    /// **The reach premium** (doc 19 §6, cut A.4; D-19-14): how this port's
    /// demand-side bids rise with its reach — the least passage from a supplying
    /// port, now against the lane's epoch. Absent, the port posts its authored
    /// prices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub premium: Option<ReachPremiumDef>,
    /// **The emergency Wanted bids** (doc 19 §6, cut A.4b; D-19-14): the
    /// life-support goods this port bids for at the premium's cap when its reach
    /// crosses the ratio. Requires `premium`. Absent, the port posts none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emergency: Option<EmergencyDef>,
    /// **Doc 20 cut SM.2 (§3.1, D-20-2)** — this port is a mining site's pad:
    /// its location has role [`crate::pack::SITE_ROLE`], it authors no orgs,
    /// rooms, programme, vendors, premium or emergency, and its sixteen blocks'
    /// tenure, prospectivity and worked share are here. Absent for a station.
    /// Boxed since doc 20 cut SM.12b (its `yard` tipped a port entry over clippy's
    /// `large_enum_variant` for every `PackEntry`, as `vehicles` and `licence` below).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<Box<SiteDef>>,
    /// **Doc 20 cut SM.3a (§4.3, Q-20-6)** — a lessor's pool of terrain
    /// vehicles, laid up by the applier (part h): Occator's rigs at Ceres. At
    /// most one port of a lane authors it, and that port runs the programme
    /// ([`check_ports`]). Absent everywhere else. Boxed, so a port entry does not
    /// grow every `PackEntry` (clippy's `large_enum_variant`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicles: Option<Box<VehiclesDef>>,
    /// **Doc 20 cut SM.5 (§9.1, D-20-12)** — the prospecting licence this port's
    /// org issues: the Freeport Registry's at Ceres. At most one port of a lane
    /// authors it, and every site's `issuer` is its org ([`check_ports`]); the
    /// instrument is [`crate::economy::LICENCE_FREEPORT`]. Absent everywhere else. Boxed,
    /// so a port entry does not grow every `PackEntry` (clippy's `large_enum_variant`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<Box<LicenceDef>>,
}

/// The most a prospecting licence's fee may be, major units (doc 20 cut SM.5).
pub const MAX_LICENCE_FEE_MAJOR: u64 = 1_000_000;
/// The longest a prospecting licence's term may be, IC days (doc 20 cut SM.5).
pub const MAX_LICENCE_TERM_IC_DAYS: u32 = 365;

/// **Doc 20 cut SM.5 (§9.1, §11's Tenure row, §12.3)**: the prospecting
/// licence's terms, which are levers and so lane data (the [`VehiclesDef`]
/// precedent). `+licence/buy` (`crate::port::licence`) reads them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenceDef {
    /// One of this entry's orgs: the issuer, paid the fee (`freeport_registry`).
    pub org: CanonicalId,
    /// The fee for one term, major units, `1..=`[`MAX_LICENCE_FEE_MAJOR`].
    pub fee_major: u64,
    /// One term, IC days, `1..=`[`MAX_LICENCE_TERM_IC_DAYS`].
    pub term_ic_days: u32,
    /// The grace after a term ends, IC days, at most the term.
    pub grace_ic_days: u32,
    /// **Doc 20 cut SM.10 (§9.3, lead ruling 3)**: a mining claim's stake fee,
    /// major units, `1..=`[`MAX_LICENCE_FEE_MAJOR`] — paid to the same org as
    /// the licence's fee, beside it (a public E4 contract into the Registry's
    /// treasury, the licence's shape). A stake pays the stake and one term.
    pub claim_stake_major: u64,
    /// **Doc 20 cut SM.10 (§9.3, lead ruling 3)**: a mining claim's term fee,
    /// major units, `1..=`[`MAX_LICENCE_FEE_MAJOR`]: paid at the stake, at each
    /// renewal, and refunded to a contest's loser.
    pub claim_term_major: u64,
}

/// **Doc 20 cut SM.3a (§4.3, Q-20-6)**: a lessor's pool of terrain vehicles,
/// laid up by the applier (part h).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehiclesDef {
    /// The pool's id (`occator.rig_lease`).
    pub id: CanonicalId,
    /// One of this entry's orgs: the lessor and the title holder.
    pub org: CanonicalId,
    /// The unit every rig is bound to (`tacsim.unit.tv_prospector`).
    pub unit: CanonicalId,
    /// The `SystemPrincipalId` that titles them (`tacsim.registry`).
    pub issuer: String,
    /// A role word of this entry's `rooms`: where `+tv/lease` is signed.
    pub counter: CanonicalId,
    /// The name of the room the applier makes for the laid-up rigs.
    pub shed: String,
    /// How many rigs, `1..=`[`MAX_VEHICLE_POOL`].
    pub pool: u32,
    /// `[A-Z0-9-]`, 1..=16 bytes; marks are `registry_mark(prefix, n)`.
    pub registry_prefix: String,
    /// One term's fee, major units.
    pub fee_major: u64,
    /// The deposit held while leased, major units.
    pub deposit_major: u64,
    /// One term, IC days, `1..=`[`MAX_VEHICLE_TERM_IC_DAYS`].
    pub term_ic_days: u32,
    /// 1..=[`MAX_VEHICLE_TOOLS`] distinct rows of
    /// [`crate::definitions::tools::tools_table`]: what every rig of the pool
    /// carries.
    pub tools: Vec<CanonicalId>,
}

/// The treasury a port that authors none capitalizes from (doc 20 cut SM.2).
fn default_treasury() -> String {
    "tacsim.treasury".to_owned()
}

/// Blocks on a site: 4 × 4 ([`crate::surface::ground::SITE_BLOCKS`] is the 4
/// along one side; doc 20 §3.1).
pub const SITE_BLOCK_COUNT: usize = 16;

/// **The sixteen block letters** (doc 20 §3.1; the SM.2 brief's decision 11),
/// read row by row from the **north-west**: `A`–`D` the northern row west to
/// east … `M`–`P` the southern; the pad sits where `F`, `G`, `J` and `K` meet.
pub const BLOCK_LETTERS: [char; SITE_BLOCK_COUNT] = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P',
];

/// **A site's tenure word** (doc 20 §3.1, §10): commons (anyone licensed works
/// it, nobody stakes it), open (staked by the first claimant; SM.10) or held
/// (an org's, named by the block's `holder`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tenure {
    /// Worked by anyone licensed, staked by nobody.
    Commons,
    /// Unclaimed; workable under a licence until staked (Q-20-4 as amended).
    Open,
    /// An org's.
    Held,
}

impl Tenure {
    /// The word a player reads.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Tenure::Commons => "commons",
            Tenure::Open => "open",
            Tenure::Held => "held",
        }
    }
}

/// **A block's zone** (doc 20 §9.5, D-20-14): every phase-1 block is core;
/// `frontier` arrives with the fight (§17) and does not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    /// The Registry's patrolled ground.
    Core,
}

/// The tenure every block of a site takes unless its row says otherwise.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteTenureDef {
    /// `commons` or `open`; a held block names its holder in its own row.
    pub default: Tenure,
}

/// **One block's exceptions** (doc 20 §3.1): every field optional, the
/// defaults the site's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDef {
    /// The block's tenure, when not the site's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenure: Option<Tenure>,
    /// The holding org (an org id of some port entry of the pack), exactly when
    /// `tenure` is `held`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<CanonicalId>,
    /// A class shift on this block's fields, `-3..=3` (SM.1's `MAX_SHIFT`; §3.1's
    /// "authored rich").
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub prospectivity: i8,
    /// The share of every pocket drawn into the spoil at opening, `0..=1000`
    /// (SM.6 acts on it; §3.4's pre-worked commons).
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    pub worked_permille: u16,
    /// The block's zone, when not the site's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<Zone>,
    /// **Doc 20 cut SM.10 (§9.6, lead ruling 10)**: another issuer's paper over
    /// this open block (the Pallas Syndicate's at Occator Rim). The site clerk
    /// seeds it as that issuer's `Contested` claims; it bars nothing, and the
    /// Registrar suspends it at the first renewal of a worked player claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<BlockPaperDef>,
}

/// **A block's paper** (doc 20 §9.6; cut SM.10): who files it and which of the
/// site's [`SiteDef::papers`] rows is its adjudication contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockPaperDef {
    /// The paper's issuer: an org of some port of the pack, never the site's
    /// own issuer.
    pub issuer: CanonicalId,
    /// A key of the site's `papers`.
    pub contract: CanonicalId,
}

/// **One paper's adjudication contract** (doc 20 §9.6; cut SM.10, OC2): the
/// bond the contract names (never escrowed). Its deadline is not lane data: it
/// is the seeded term (`surface::tenure::SEEDED_TERM_IC_SECS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaperDef {
    /// The bond, major units, `1..=`[`MAX_LICENCE_FEE_MAJOR`].
    pub bond_major: u64,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_i8(value: &i8) -> bool {
    *value == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_u16(value: &u16) -> bool {
    *value == 0
}

/// **A site block** (doc 20 §3.1, D-20-2; cut SM.2): the site's name, the
/// Registry that rules it, its zone, its default tenure and per-block
/// exceptions. The geology table is its **body's** (the lane's `BodyDef`), never
/// the site's (the SM.2 brief's decision 3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteDef {
    /// The site's player-facing name; equal to its location's label (a pack
    /// rule), so [`crate::port::coop::port_name`] names a site with no pack in hand.
    pub name: String,
    /// The org whose Registry rules the site (an org of some port entry of this pack).
    pub issuer: CanonicalId,
    /// The site's zone; every phase-1 block is core (§9.5, D-20-14).
    pub zone: Zone,
    /// The tenure every block takes unless its row says otherwise.
    pub tenure: SiteTenureDef,
    /// Block letter (`A`..=`P`) → its exceptions; absent letters take the defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blocks: BTreeMap<String, BlockDef>,
    /// **Doc 20 cut SM.10 (§9.6)**: the papers filed over this site's blocks,
    /// row id → its adjudication contract's terms; every row is named by some
    /// block's `paper`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub papers: BTreeMap<CanonicalId, PaperDef>,
    /// **Doc 20 cut SM.12b (§12.3; the lead's rulings 1–2, 5)**: the house's yard at this site,
    /// laid up by the applier (part i, `crate::port::yard::lay_up`). Absent everywhere but
    /// Occator Rim. Boxed, so a site block does not grow every port entry; skipped when absent,
    /// so a site from before the cut re-encodes byte-identically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yard: Option<Box<YardDef>>,
}

/// The longest a yard's re-arm interval may be, IC seconds: a week, the claims' and the fill
/// plan's scale (doc 20 cut SM.12b).
pub const MAX_YARD_REARM_IC_SECS: i64 = 7 * 86_400;

/// **A house's yard at a site** (doc 20 cut SM.12b; doc 20 §12.3 "Occator's drones", §9.6, §4.3,
/// D-20-11; the lead's rulings 1, 2 and 5): a hull of `unit` settled for good on the site's pad
/// (named `name`, marked `registry_prefix` + `01`), its hold a vault of the house, its bay holding
/// `rigs` rigs of the house's own pool (`pool`, marked `rig_prefix` + `01`…), and a `DARK`
/// dispatcher in the site's host room that operates them for the house, filing SM.11 fill plans
/// over `blocks` for `ore` and delivering the yard hold's ore in kind to the house's mill.
///
/// **The numbers and their basis** (O-8, O-9, O-14, ruled by the owner on 2026-10-24 as defaults
/// kept provisionally — "keep the defaults … I'll tune later"; every figure the yard turns on is
/// here or named from here): **rigs 2** is the Dray-P's bay (`dray_p.json`'s `rigs` bay,
/// `maximum_items` 2); **the hold is the unit's**, 86 t / 103.2 m³ under `stow.ore`, and ice ore
/// binds by mass, so one landed hold a plan is 86 t of ice — 34.4 t of water at the mill's 5 : 2,
/// a little over two IC days of its 40 t; **re-arm 3,600 IC s** is one IC hour (SM.10's
/// `CLAIM_ORDER_TTL_IC_SECS` scale; 150 real s at world rate 24), so at most one filing a rig an
/// IC hour; **blocks F and G** are §9.6/§12.3's "two worked by its drones" (fair-or-better ice
/// under the reference key `KEY_1`: F 271 t, G 256 t); **the delivery bound** is the house mill's
/// own `MillDef::per_ic_day_t` (SM.12a's number, none of this cut's).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YardDef {
    /// The house: an org of some port of the pack (`occator`).
    pub org: CanonicalId,
    /// The hull's name, label-shaped ("Occator Yard").
    pub name: String,
    /// The hull's unit, with a bay and an ore hold (`tacsim.unit.dray_p`).
    pub unit: CanonicalId,
    /// The hull's mark prefix, separator included (`OWP-YD-`).
    pub registry_prefix: String,
    /// The pack's one rig pool, whose org is `org` (`occator.rig_lease`).
    pub pool: CanonicalId,
    /// The rigs' mark prefix (`OWP-YT-`).
    pub rig_prefix: String,
    /// How many rigs, `1..=`[`crate::definitions::logistics::MAX_BAY_ITEMS`], at most the unit's
    /// bay (the apply's preflight).
    pub rigs: u32,
    /// The ore its plans mine and it delivers (`ice_ore`).
    pub ore: CanonicalId,
    /// The held blocks it works, 1 to 4 distinct letters (`F`, `G`).
    pub blocks: Vec<char>,
    /// The re-arm interval after a plan ends, IC seconds, `1..=`[`MAX_YARD_REARM_IC_SECS`].
    pub rearm_ic_secs: i64,
}

/// Whether a registry prefix is 1 to [`MAX_REGISTRY_PREFIX_BYTES`] bytes of `A-Z`, `0-9` and `-`.
fn prefix_ok(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix.len() <= MAX_REGISTRY_PREFIX_BYTES
        && prefix
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
}

impl YardDef {
    /// **The yard's own bounds against its site** (doc 20 cut SM.12b): a label-shaped name; two
    /// registry prefixes, each 1 to 16 bytes of `A-Z`, `0-9` and `-`, and not the same; `rigs`
    /// 1 to 4; the re-arm interval in bounds; 1 to 4 distinct block letters, each held by the
    /// yard's own org at `site`.
    ///
    /// # Errors
    ///
    /// The first rule broken, as its detail.
    pub fn check(&self, site: &SiteDef) -> Result<(), String> {
        let max_rigs = crate::definitions::logistics::MAX_BAY_ITEMS;
        if !label_ok(&self.name) {
            return Err("site: yard: the name is not a label".to_owned());
        }
        if !prefix_ok(&self.registry_prefix) || !prefix_ok(&self.rig_prefix) {
            return Err(format!(
                "site: yard: a registry prefix is 1 to {MAX_REGISTRY_PREFIX_BYTES} bytes of A-Z, \
                 0-9 and -"
            ));
        }
        if self.registry_prefix == self.rig_prefix {
            return Err("site: yard: the hull and its rigs need two prefixes".to_owned());
        }
        if self.rigs == 0 || self.rigs > max_rigs {
            return Err(format!("site: yard: rigs is 1 to {max_rigs}"));
        }
        if self.rearm_ic_secs < 1 || self.rearm_ic_secs > MAX_YARD_REARM_IC_SECS {
            return Err(format!(
                "site: yard: rearm_ic_secs is 1 to {MAX_YARD_REARM_IC_SECS}"
            ));
        }
        let distinct: BTreeSet<char> = self.blocks.iter().copied().collect();
        if self.blocks.is_empty()
            || self.blocks.len() > 4
            || distinct.len() != self.blocks.len()
            || self.blocks.iter().any(|l| !BLOCK_LETTERS.contains(l))
        {
            return Err("site: yard: blocks names 1 to 4 distinct letters A to P".to_owned());
        }
        for letter in &self.blocks {
            let terms = site.block(*letter);
            if terms.tenure != Tenure::Held || terms.holder != Some(&self.org) {
                return Err(format!(
                    "site: yard: block {letter} is not held by {}",
                    self.org
                ));
            }
        }
        Ok(())
    }
}

/// One block as the site reads it: its tenure, holder, prospectivity, worked
/// share and zone, the row's or the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockTerms<'a> {
    /// The tenure.
    pub tenure: Tenure,
    /// The holder, for a held block.
    pub holder: Option<&'a CanonicalId>,
    /// The class shift.
    pub prospectivity: i8,
    /// The worked share, permille.
    pub worked_permille: u16,
    /// The zone.
    pub zone: Zone,
}

impl SiteDef {
    /// Block `letter`'s terms: its row's, or the site's defaults.
    #[must_use]
    pub fn block(&self, letter: char) -> BlockTerms<'_> {
        let row = self.blocks.get(letter.to_string().as_str());
        BlockTerms {
            tenure: row
                .and_then(|row| row.tenure)
                .unwrap_or(self.tenure.default),
            holder: row.and_then(|row| row.holder.as_ref()),
            prospectivity: row.map_or(0, |row| row.prospectivity),
            worked_permille: row.map_or(0, |row| row.worked_permille),
            zone: row.and_then(|row| row.zone).unwrap_or(self.zone),
        }
    }

    /// **The sixteen shifts in `SiteGeologySpec`'s order** (decision 11): letter
    /// index `i` (0 for `A` … 15 for `P`, row by row from the north-west) is block
    /// `bx = i % 4`, `by = 3 − i / 4` from the south-west, slot `by * 4 + bx`.
    #[must_use]
    pub fn shifts(&self) -> [i8; SITE_BLOCK_COUNT] {
        let mut shifts = [0_i8; SITE_BLOCK_COUNT];
        for (i, letter) in BLOCK_LETTERS.into_iter().enumerate() {
            let (bx, by) = (i % 4, 3_usize.saturating_sub(i / 4));
            if let Some(slot) = shifts.get_mut(by * 4 + bx) {
                *slot = self.block(letter).prospectivity;
            }
        }
        shifts
    }

    /// **The site block's own bounds** (doc 20 §3.1; cut SM.2): a label-shaped
    /// name; every `blocks` key one letter `A`..=`P`; the default tenure
    /// `commons` or `open`; a row `held` exactly when it names a holder;
    /// prospectivity `-3..=3`; worked share `0..=1000`; (doc 20 cut SM.12b) a yard's own
    /// bounds against the site ([`YardDef::check`]).
    ///
    /// # Errors
    ///
    /// The first rule broken, as its detail.
    pub fn check(&self) -> Result<(), String> {
        if !label_ok(&self.name) {
            return Err(format!(
                "site: name must be non-blank and at most {MAX_LABEL_BYTES} bytes"
            ));
        }
        if self.tenure.default == Tenure::Held {
            return Err(
                "site: the default tenure is commons or open; a held block names its holder"
                    .to_owned(),
            );
        }
        for (letter, row) in &self.blocks {
            let one = {
                let mut chars = letter.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if BLOCK_LETTERS.contains(&c) => Some(c),
                    _ => None,
                }
            };
            let Some(letter) = one else {
                return Err(format!("site: block {letter} is not a block letter A to P"));
            };
            if (row.tenure == Some(Tenure::Held)) != row.holder.is_some() {
                return Err(format!(
                    "site: block {letter}: a held block names its holder, and only a held block does"
                ));
            }
            let max = crate::surface::geology::MAX_SHIFT;
            if !(-max..=max).contains(&row.prospectivity) {
                return Err(format!(
                    "site: block {letter}: prospectivity is -{max} to {max}, not {}",
                    row.prospectivity
                ));
            }
            if row.worked_permille > 1_000 {
                return Err(format!(
                    "site: block {letter}: worked_permille is 0 to 1000, not {}",
                    row.worked_permille
                ));
            }
            // Doc 20 cut SM.10 (§9.6): a paper stands only over open ground,
            // and names one of the site's papers rows.
            if let Some(paper) = &row.paper {
                if self.block(letter).tenure != Tenure::Open {
                    return Err(format!(
                        "site: block {letter}: a paper stands only over an open block"
                    ));
                }
                if !self.papers.contains_key(&paper.contract) {
                    return Err(format!(
                        "site: block {letter}: paper contract {} is not a row of the site's papers",
                        paper.contract
                    ));
                }
            }
        }
        // Doc 20 cut SM.10: every papers row is named, its bond in bounds.
        for (id, paper) in &self.papers {
            if !self.blocks.values().any(|row| {
                row.paper
                    .as_ref()
                    .is_some_and(|block_paper| &block_paper.contract == id)
            }) {
                return Err(format!("site: papers row {id} is named by no block"));
            }
            if paper.bond_major == 0 || paper.bond_major > MAX_LICENCE_FEE_MAJOR {
                return Err(format!(
                    "site: papers row {id}: bond_major must be 1..={MAX_LICENCE_FEE_MAJOR}, not {}",
                    paper.bond_major
                ));
            }
        }
        // Doc 20 cut SM.12b (§12.3): the yard's own bounds; its pack-wide rules are
        // `check_ports`', its unit facts the apply preflight's.
        if let Some(yard) = &self.yard {
            yard.check(self)?;
        }
        Ok(())
    }
}

/// The most a premium's slope or cap may be, permille (ten times the price).
pub const MAX_PREMIUM_PERMILLE: u32 = 10_000;

/// The most an emergency ratio may be, permille (a hundred times the median).
pub const MAX_EMERGENCY_RATIO_PERMILLE: u32 = 100_000;

/// The most goods an emergency block may name.
pub const MAX_EMERGENCY_GOODS: usize = 16;

/// **A port's emergency bids** (doc 19 §6, cut A.4b; D-19-14; Q-19-5's default
/// ratio 1,500 ‰): when the port's reach for one of `goods` is at or past
/// `ratio_permille` of its synodic median, `org`'s vendor posts a Wanted bid for
/// it at the premium's cap over the good's reference (the port's highest authored
/// price for it, else the goods table's), `intake` units deep; withdrawn when the
/// reach falls back. A good no other port supplies has no reach and no emergency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmergencyDef {
    /// The vendor (one of `vendors[].org`) whose storefront posts the bids; it
    /// may not ask or bid any of `goods` itself.
    pub org: CanonicalId,
    /// `reach_now / reach_median` in permille at or past which the bids post;
    /// `1000..=MAX_EMERGENCY_RATIO_PERMILLE`.
    pub ratio_permille: u32,
    /// The life-support goods, unique, `1..=MAX_EMERGENCY_GOODS`.
    pub goods: Vec<CanonicalId>,
    /// Each bid's intake in the good's quote units,
    /// `1..=mush_market::MAX_QUANTITY`.
    pub intake: u64,
}

/// **A port's reach premium** (doc 19 §6; D-19-14; Q-19-5's defaults 300 / 500):
/// `premium‰ = min(cap, k · (reach_now / reach_epoch − 1))`, never below zero,
/// applied to a bid's authored price when the bid is posted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReachPremiumDef {
    /// The slope, permille of price per unit of excess reach ratio;
    /// `0..=MAX_PREMIUM_PERMILLE`.
    pub k_permille: u32,
    /// The ceiling, permille of price; `0..=MAX_PREMIUM_PERMILLE`.
    pub cap_permille: u32,
}

/// **One institution of a port** (doc 18 §4.1, D-18-27): a real `mush-org`
/// org with a root scope and one scoped `tacsim.credit` account, capitalized
/// once from the treasury.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOrgDef {
    /// The id the port map and the programme's references name it by; unique
    /// across every port entry of the pack (one legal person per id).
    pub id: CanonicalId,
    /// The org's name in the world.
    pub name: String,
    /// Its capital, major units, `1..=`[`MAX_PORT_CAPITAL_MAJOR`].
    pub capital_major: u64,
    /// The polity it answers to (`freeport`, `concord`, `republic`). Carried
    /// and validated, read by nothing until the faction cut (lead ruling L16).
    pub faction: CanonicalId,
    /// **Doc 20 cut SM.5 (§7.3, §12.3)**: capital raised after the base, each
    /// raise minted as its own appended tranches ([`capital_schedule`]). Append-only
    /// across versions: a raise once issued is never edited, dropped or reordered
    /// (the applier refuses it as drift). At most [`MAX_CAPITAL_RAISES`]; the
    /// capital with every raise stays within [`MAX_PORT_CAPITAL_MAJOR`]. Occator's
    /// 250,000 at v11 takes it from 50,000 to 300,000.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raises: Vec<u64>,
}

/// The most capital raises one institution may author (doc 20 cut SM.5).
pub const MAX_CAPITAL_RAISES: usize = 8;

impl PortOrgDef {
    /// The capital with every raise, major units; `None` on overflow.
    #[must_use]
    pub fn capital_total(&self) -> Option<u64> {
        self.raises
            .iter()
            .try_fold(self.capital_major, |total, raise| total.checked_add(*raise))
    }
}

/// **The First Captain programme** (doc 18 §4.1, R2, D-18-25).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgrammeDef {
    /// The programme's id (`coop.first_captain`).
    pub id: CanonicalId,
    /// The org that runs it: one of this entry's orgs (the Trust).
    pub org: CanonicalId,
    /// `"<area>.<entry>"`: the desk it enrols at.
    pub office: String,
    /// The balance the programme org keeps above every stake, major units.
    pub floor_major: u64,
    /// The hull class it leases (`tacsim.unit.dray`).
    pub unit: CanonicalId,
    /// A `SystemPrincipalId`: the registry that titles its hulls.
    pub issuer: String,
    /// How many hulls it holds, `1..=`[`MAX_PROGRAMME_POOL`].
    pub pool: u32,
    /// The registry number prefix of its hulls (`[A-Z0-9-]`, 1..=16 bytes).
    pub registry_prefix: String,
    /// One name per hull, `pool` of them, unique.
    pub hull_names: Vec<String>,
    /// The stake it lends each captain, major units.
    pub stake_major: u64,
    /// Its share of the trading margin, per mille, `0..=1000`.
    pub share_permille: u32,
    /// Good → `"<port entry>/<org>"`: the listing whose posted ask is the
    /// reference price (D-18-24: it names a listing, never a price).
    pub reference: BTreeMap<CanonicalId, String>,
    /// **Doc 20 cut SM.5 (§7.4, Q-20-5)**: good → a fixed reference price in
    /// minor units, for a good **no port of the lane asks** (ore at half its bid),
    /// read before any listing by `CoopProceeds`. A good is in at most one of the
    /// two maps ([`check_ports`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reference_minor: BTreeMap<CanonicalId, u64>,
    /// The buyout price of a hull, major units.
    pub buyout_major: u64,
    /// How long a lease may lie idle before it is released, real days.
    pub dormancy_real_days: u32,
    /// How many enrolments one account may hold.
    pub per_account: u32,
    /// **Doc 20 cut SM.3a (§4.3, D-20-7)**: refit word → the unit `+coop/refit
    /// <word>` re-binds a chartered hull to; `standard` is always `unit` and is
    /// never authored. Each refit unit is `refit_compatible` with `unit`
    /// (checked at the apply preflight, where the catalog is).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refits: BTreeMap<CanonicalId, CanonicalId>,
}

/// **One vendor** (doc 18 §4.7, §4.3 "Vendors and buyers"; applied by the
/// `PortApplier`'s parts d–e from cut IC8): an org of the entry that sells its
/// finite stock from one storefront vault and posts its standing bids.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VendorDef {
    /// The org that trades: one of this entry's orgs.
    pub org: CanonicalId,
    /// `<area>.<entry>` of an `item` in the port's `spine` room: the container
    /// its storefront vault is backed by.
    pub storefront: String,
    /// **The port's stock clerk** (doc 18 cut IC8, lead ruling L2; D-18-26 "a
    /// per-port clerk actor"): every vendor of one port names the same agent
    /// id, and no two ports share one. The applier makes one clerk Thing per
    /// port under it.
    pub agent: CanonicalId,
    /// `"<area>.<entry>"`: the Warrant signatory, an npc of the area.
    /// **Required** — no serde default.
    pub signatory: String,
    /// The finite receipted stock minted once (R4).
    #[serde(default)]
    pub stock: Vec<StockRowDef>,
    /// The posted asks.
    #[serde(default)]
    pub asks: Vec<AskDef>,
    /// The Wanted bids.
    #[serde(default)]
    pub bids: Vec<BidDef>,
    /// **Doc 20 cut SM.12a (§7.3 "The mill"; doc 18 §5's P5)**: the house mills an
    /// ore it bids for into a good it asks, `batch_t` at a time, at most
    /// `per_ic_day_t` of ore an IC day (`crate::port::mill`). A mill's `ratio` and
    /// `into` are fixed for the life of its ledger (D9). Skipped when empty, so a
    /// vendor without a mill re-encodes byte-identically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mill: Vec<MillDef>,
}

/// **A mill's ratio** (doc 20 cut SM.12a; §7.3's "5 : 2"): `ore` tonnes of ore
/// make `product` tonnes of product, `1 <= product <= ore <= MAX_MILL_RATIO`
/// ([`MAX_MILL_RATIO`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MillRatio {
    /// Tonnes of ore a ratio consumes.
    pub ore: u32,
    /// Tonnes of product a ratio makes.
    pub product: u32,
}

/// **One mill of a vendor** (doc 20 cut SM.12a; doc 20 §7.3 "The mill", §12.3's
/// "Levers, all data", D-20-10, Q-20-1): on the port tick's stock arm the port's
/// stock clerk mills `batch_t` tonnes of the house's storefront `ore` into
/// `batch_t / ratio.ore × ratio.product` tonnes of `into` at a time, at most
/// `per_ic_day_t` tonnes of ore an IC day, and restocks the house's drawn ask of
/// `into` by what it made (`crate::port::mill`).
///
/// **The numbers are the lane's, and their basis is here** (doc 20a §6, O-1; the
/// owner took them provisionally on 2026-10-24 — "put in some reasonable defaults
/// and figure it out as we go" — so tune them in the lane's two `mill` rows and
/// nowhere else):
///
/// - **The ratio, 5 : 2**, is §7.3's ore-goods table: buy-and-mill never loses a
///   house money (a tonne of concentrate costs 2.5 t of metal ore at 70.00cr/t,
///   175.00cr, under its 200.00cr ask; a tonne of water 2.5 t of ice ore at
///   35.00cr/t, 87.50cr, under its 100.00cr ask — §11's Prices row).
/// - **The rate, 40 t of ore an IC day**, equals the intake drip's `per_ic_day`
///   (Q-20-11), so the mill keeps pace with what the drip lets the house buy and
///   is never a second cap on the ore economy. At world rate 24 that is up to
///   960 t of each bulk ore a real day milled into 384 t of product, against
///   §12.3's draw of about 80 t of concentrate and 320 t of water a real day:
///   4.8× and 1.2× headroom. Water holds only if Occator mills at least 33.3 t of
///   ice an IC day (about 800 t a real day, some 11.4 fair miner-hours at 70 t an
///   hour, unless SM.12b's drones feed it). With the mill at the drip's rate up to
///   about 259 t can sit unmilled at full drip (the bid's standing 240 t plus a
///   remainder under one batch).
/// - **The resale is bounded by the trade run's draw** (80 t × 200 = 16,000 cr a
///   real day at the Exchange against 67,200 cr of full metal-ore drip), so the
///   mill does not refund the Exchange's capital; Occator nearly breaks even
///   (320 × 100 = 32,000 against 33,600). At full drip the Exchange's intakes close
///   on capital in about a real month whether the mill runs or not (C8).
/// - **The batch, 20 t (four ratios)**, mills a day's drip in two acts, lets one
///   sold hopper (12 t, §11's Hopper row) wait for the next sale, and keeps a mill
///   to at most two batches an IC day.
///
/// `ratio` and `into` are fixed for the life of the mill's ledger (D9, O-5's
/// default): the ledger is keyed by the ore, so a changed ratio or product is
/// refused at tick time and paged; `batch_t` and `per_ic_day_t` change freely.
///
/// **Removing a row** (O-7's default): the mill stops, and the units it milled and
/// the clerk had not listed make the drift check page and the gated Listings line of
/// its good read Flagged until staff run `@market/vendor withdraw <listing>` on that
/// ask; the stock arm's list step then re-lists the whole pool. A sell-out of that
/// ask to 0, or its lapse the sweep deletes, clears the page the same way: the list
/// step re-lists whenever the ask has no listing. **Re-adding a removed row** (the
/// authoring rule; SM.12a's correctness F3): that list step sets only the authored
/// mills' counters, so any fresh listing of that good's ask since the removal (a
/// staff withdraw, a sell-out to 0, or a lapse the sweep deleted) re-lists the whole
/// pool and leaves the removed mill's `mill_listed` counter stale; the same ore names
/// the same ledger again (the mill id is pack, entry, org and ore), so the re-added
/// row's owed units are units already listed — the drift check pages and the Listings
/// line reads Flagged until a second withdraw. Re-add a row only together with a
/// withdraw of its good's ask, or when that ask has not been listed afresh since the
/// removal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MillDef {
    /// The ore milled: one of this vendor's `bids`.
    pub ore: CanonicalId,
    /// The product made: one of this vendor's `asks`, of the ore's family.
    pub into: CanonicalId,
    /// Tonnes of ore to tonnes of product.
    pub ratio: MillRatio,
    /// Tonnes of ore one batch mills: a whole number of ratios, at most
    /// `per_ic_day_t`.
    pub batch_t: u64,
    /// The most ore the mill takes in one IC day, tonnes, at most
    /// `mush_market::MAX_QUANTITY`.
    pub per_ic_day_t: u64,
}

/// One stock row of a vendor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockRowDef {
    /// The good.
    pub good: CanonicalId,
    /// How much, `>= 1`.
    pub qty: u64,
    /// Its grade, `0..=100`.
    pub grade: u8,
    /// Its issue number, `>= 1`.
    pub issue: u32,
}

/// One posted ask of a vendor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskDef {
    /// The good.
    pub good: CanonicalId,
    /// The price, major units per quote unit, `>= 1`.
    pub price_major: u64,
    /// The room roles (keys of the entry's `rooms`, never `spine`) whose look
    /// renders this line on its board (doc 18 cut IC8, lead ruling L15: 18a's
    /// room-by-room Items entries). Empty: no board shows it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boards: Vec<CanonicalId>,
}

/// One Wanted bid of a vendor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BidDef {
    /// The good.
    pub good: CanonicalId,
    /// The price, major units per quote unit, `>= 1`.
    pub price_major: u64,
    /// How much it takes, `>= 1`.
    pub intake: u64,
    /// The name the bid posts under, when not the good's.
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub as_name: Option<String>,
    /// The room roles whose look renders this bid on its board, as
    /// [`AskDef::boards`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boards: Vec<CanonicalId>,
    /// **Doc 20 cut SM.5 (§7.3, Q-20-11)**: an intake the stock clerk refreshes by
    /// a drip each IC day, to a cap (the ore bids at Ceres). Present, the bid is
    /// the drip's (`crate::port::stock`) and never the lapsed-bid re-post's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<RefreshDef>,
}

/// **An intake's daily refresh** (doc 20 §7.3, Q-20-11; cut SM.5): at most once
/// an IC day the clerk re-posts the bid at `min(cap, unfilled + per_ic_day)`,
/// within what the house can fund.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshDef {
    /// Units added to the unfilled intake each IC day, `1..=cap`.
    pub per_ic_day: u64,
    /// The most the intake ever stands at, `per_ic_day..=mush_market::MAX_QUANTITY`
    /// and at least the bid's opening `intake`.
    pub cap: u64,
}

/// **An institution's capital schedule with its raises** (doc 20 cut SM.5; §7.3,
/// D-18-14): the base's [`tranches`], then each raise's own [`tranches`] in
/// authored order, so a raise appends tranches and never re-cuts an issued one,
/// and every tranche stays at most [`MAX_TRANCHE_MAJOR`] under the mint cap.
/// `capital_schedule(50_000, &[250_000])` is `[50000, 250000]`.
#[must_use]
pub fn capital_schedule(base_major: u64, raises: &[u64]) -> Vec<u64> {
    let mut out = tranches(base_major);
    for raise in raises {
        out.extend(tranches(*raise));
    }
    out
}

/// **An institution's capital as tranches** (D-18-14; doc 18 §4.3): greedy
/// chunks of [`MAX_TRANCHE_MAJOR`], the remainder last, none for zero — so
/// 3,100,000 is `[800000, 800000, 800000, 700000]` and 50,000 is `[50000]`.
#[must_use]
pub fn tranches(capital_major: u64) -> Vec<u64> {
    let mut out = Vec::new();
    let mut left = capital_major;
    while left > 0 {
        let take = left.min(MAX_TRANCHE_MAJOR);
        out.push(take);
        left -= take;
    }
    out
}

/// **The one split rule for a `<pack>.<entry>` reference in a port entry**
/// (lead ruling L2): the reference must be `"<area>." + entry` with `entry` a
/// canonical id; the key is `RecordKey::new(area, entry)` — the `RoomGeo.key`
/// (or npc key) the referenced entry was made under.
///
/// # Errors
///
/// The detail naming the reference, when it is outside the port's area or its
/// entry part is not a canonical id.
pub fn room_key(area: &CanonicalId, reference: &str) -> Result<mush_atlas::RecordKey, String> {
    let prefix = format!("{area}.");
    let Some(rest) = reference.strip_prefix(prefix.as_str()) else {
        return Err(format!("`{reference}` is not in the port's area `{area}`"));
    };
    let entry = CanonicalId::parse_canonical(rest)
        .map_err(|_| format!("`{reference}` does not name an entry of `{area}`"))?;
    Ok(mush_atlas::RecordKey::new(area.clone(), entry))
}

/// A label: non-blank and at most [`MAX_LABEL_BYTES`].
fn label_ok(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= MAX_LABEL_BYTES
}

/// A `SystemPrincipalId` in its canonical form (no case folding on the wire:
/// what the entry says is what the economy is asked for).
fn principal_ok(text: &str) -> bool {
    mush_org::SystemPrincipalId::new(text).is_some_and(|p| p.as_str() == text)
}

impl PortDef {
    /// **The entry's own bounds** (doc 18 §4.7). The detail names the field and
    /// the rule; the caller (`RulesPack::from_content`) wraps it with the entry
    /// id, so the entry is mentioned once.
    ///
    /// # Errors
    ///
    /// The first rule broken, as its detail.
    pub fn check(&self) -> Result<(), String> {
        // Doc 19 cut A.4: the premium's bounds.
        if let Some(premium) = &self.premium
            && (premium.k_permille > MAX_PREMIUM_PERMILLE
                || premium.cap_permille > MAX_PREMIUM_PERMILLE)
        {
            return Err(format!(
                "premium: k_permille and cap_permille are 0..={MAX_PREMIUM_PERMILLE}, not {} / {}",
                premium.k_permille, premium.cap_permille
            ));
        }
        // Doc 20 cut SM.2 (§3.1: "a validator rule letting a site entry author zero
        // orgs and only a pad room"): a site is a pad and its block terms, nothing
        // else; its gallery and routes are checked below as every port's are.
        if let Some(site) = &self.site {
            if !self.orgs.is_empty()
                || !self.rooms.is_empty()
                || !self.vendors.is_empty()
                || self.programme.is_some()
                || self.premium.is_some()
                || self.emergency.is_some()
                || self.vehicles.is_some()
                || self.licence.is_some()
            {
                // Doc 20 cut SM.3a: nor a rig pool (rigs are leased at a station);
                // cut SM.5: nor a licence (the Registry issues it at its station).
                return Err(
                    "site: a site authors no orgs, rooms, programme, vendors, premium or \
                     emergency, and no vehicles or licence block; its pad is its gallery"
                        .to_owned(),
                );
            }
            site.check()?;
        } else if self.orgs.is_empty() || self.orgs.len() > MAX_PORT_ORGS {
            // The institutions.
            return Err(format!(
                "orgs: a port authors 1..={MAX_PORT_ORGS} orgs, not {}",
                self.orgs.len()
            ));
        }
        let mut ids: BTreeSet<&CanonicalId> = BTreeSet::new();
        for org in &self.orgs {
            if !ids.insert(&org.id) {
                return Err(format!("org {}: the id is authored twice", org.id));
            }
            if !label_ok(&org.name) {
                return Err(format!(
                    "org {}: name must be non-blank and at most {MAX_LABEL_BYTES} bytes",
                    org.id
                ));
            }
            if org.capital_major == 0 || org.capital_major > MAX_PORT_CAPITAL_MAJOR {
                return Err(format!(
                    "org {}: capital_major must be 1..={MAX_PORT_CAPITAL_MAJOR}, not {}",
                    org.id, org.capital_major
                ));
            }
            // Doc 20 cut SM.5 (§7.3, §12.3): the appended raises.
            if !org.raises.is_empty()
                && (org.raises.len() > MAX_CAPITAL_RAISES
                    || org
                        .raises
                        .iter()
                        .any(|raise| *raise == 0 || *raise > MAX_PORT_CAPITAL_MAJOR)
                    || org
                        .capital_total()
                        .is_none_or(|total| total > MAX_PORT_CAPITAL_MAJOR))
            {
                return Err(format!(
                    "org {}: raises: at most {MAX_CAPITAL_RAISES}, each 1..={MAX_PORT_CAPITAL_MAJOR}, \
                     and the capital with every raise at most {MAX_PORT_CAPITAL_MAJOR}",
                    org.id
                ));
            }
        }
        // The rooms, by the one split rule.
        room_key(&self.area, &self.gallery).map_err(|detail| format!("gallery: {detail}"))?;
        for (role, reference) in &self.rooms {
            if !PORT_ROOM_ROLES.contains(&role.as_str()) {
                return Err(format!(
                    "rooms: `{role}` is not a port room role ({})",
                    PORT_ROOM_ROLES.join(", ")
                ));
            }
            room_key(&self.area, reference).map_err(|detail| format!("rooms.{role}: {detail}"))?;
        }
        if !principal_ok(&self.treasury) {
            return Err(format!(
                "treasury: `{}` is not a system principal id",
                self.treasury
            ));
        }
        // The routes (D-18-47).
        if self.routes.is_empty() {
            return Err("routes: a port lists the routes to at least one other port".to_owned());
        }
        for (venue, list) in &self.routes {
            if list.is_empty() || list.len() > MAX_PORT_ROUTES {
                return Err(format!(
                    "routes.{venue}: a destination lists 1..={MAX_PORT_ROUTES} routes, not {}",
                    list.len()
                ));
            }
            let mut seen: BTreeSet<&CanonicalId> = BTreeSet::new();
            if let Some(twice) = list.iter().find(|id| !seen.insert(*id)) {
                return Err(format!("routes.{venue}: route {twice} is listed twice"));
            }
        }
        if let Some(programme) = &self.programme {
            self.check_programme(programme)?;
        }
        // The vendors' schema (applied from IC8).
        if self.vendors.len() > MAX_PORT_VENDORS {
            return Err(format!(
                "vendors: a port authors at most {MAX_PORT_VENDORS} vendors, not {}",
                self.vendors.len()
            ));
        }
        // Doc 18 cut IC14 row 29 (lead ruling L24): the stock clerk carries at most
        // `MAX_CLERK_ROWS` rows for a port, so a port may author no more across its
        // vendors (a row past it would never be minted, and the clerk's arm stalled on
        // it). The caller names the entry.
        let stock_rows: usize = self.vendors.iter().map(|v| v.stock.len()).sum();
        if stock_rows > crate::port::stock::MAX_CLERK_ROWS {
            return Err(format!(
                "vendors: a port stocks at most {} rows across its vendors, not {stock_rows}",
                crate::port::stock::MAX_CLERK_ROWS
            ));
        }
        for vendor in &self.vendors {
            if !ids.contains(&vendor.org) {
                return Err(format!(
                    "vendors: org {} is not one of this port's orgs",
                    vendor.org
                ));
            }
            room_key(&self.area, &vendor.storefront)
                .map_err(|detail| format!("vendor {}: storefront: {detail}", vendor.org))?;
            if vendor.signatory.trim().is_empty() {
                return Err(format!(
                    "vendor {}: signatory must be non-blank",
                    vendor.org
                ));
            }
            room_key(&self.area, &vendor.signatory)
                .map_err(|detail| format!("vendor {}: signatory: {detail}", vendor.org))?;
            for (field, len) in [
                ("stock", vendor.stock.len()),
                ("asks", vendor.asks.len()),
                ("bids", vendor.bids.len()),
            ] {
                if len > MAX_VENDOR_ROWS {
                    return Err(format!(
                        "vendor {}: {field} holds at most {MAX_VENDOR_ROWS} rows, not {len}",
                        vendor.org
                    ));
                }
            }
            for row in &vendor.stock {
                // Doc 18 cut IC14 row 28: a quantity past the market's own cap would list
                // min(pool, cap) and read as drift for ever, so it is refused here.
                if row.qty == 0
                    || row.qty > mush_market::MAX_QUANTITY
                    || row.issue == 0
                    || row.grade > 100
                {
                    return Err(format!(
                        "vendor {}: stock {}: qty must be 1..={} and issue at least 1 and grade \
                         0..=100",
                        vendor.org,
                        row.good,
                        mush_market::MAX_QUANTITY
                    ));
                }
            }
            if let Some(ask) = vendor.asks.iter().find(|ask| ask.price_major == 0) {
                return Err(format!(
                    "vendor {}: ask {}: price_major must be at least 1",
                    vendor.org, ask.good
                ));
            }
            if let Some(bid) = vendor
                .bids
                .iter()
                .find(|bid| bid.price_major == 0 || bid.intake == 0)
            {
                return Err(format!(
                    "vendor {}: bid {}: price_major and intake must be at least 1",
                    vendor.org, bid.good
                ));
            }
            // Doc 18 cut IC8 (fixer F3): a bid's intake is its posted remaining, which
            // the market bounds; refused here, at load, so the applier never makes a
            // vendor's clerk, vault and grant and then fails at `post_wanted`.
            if let Some(bid) = vendor
                .bids
                .iter()
                .find(|bid| bid.intake > mush_market::MAX_QUANTITY)
            {
                return Err(format!(
                    "vendor {}: bid {}: intake must be at most {}",
                    vendor.org,
                    bid.good,
                    mush_market::MAX_QUANTITY
                ));
            }
            self.check_vendor_lines(vendor)?;
        }
        // Doc 18 cut IC8 (lead ruling L2; D-18-26): one stock clerk per port.
        if let Some(first) = self.vendors.first()
            && self
                .vendors
                .iter()
                .any(|vendor| vendor.agent != first.agent)
        {
            return Err(
                "vendors: every vendor of a port names the port's one stock clerk".to_owned(),
            );
        }
        // Doc 19 cut A.4b: the emergency block, after the vendors' own rules so a
        // vendor's fault is named as the vendor's.
        if let Some(emergency) = &self.emergency {
            self.check_emergency(emergency)?;
        }
        // Doc 20 cut SM.3a: the rig pool.
        if let Some(vehicles) = &self.vehicles {
            self.check_vehicles(vehicles)?;
        }
        // Doc 20 cut SM.5: the licence block.
        if let Some(licence) = &self.licence {
            self.check_licence(licence)?;
        }
        Ok(())
    }

    /// **The licence block's bounds** (doc 20 cut SM.5; §9.1): its org one of the
    /// entry's, the fee and the term in bounds, the grace at most the term. Each
    /// sentence starts `licence: `.
    fn check_licence(&self, licence: &LicenceDef) -> Result<(), String> {
        if !self.orgs.iter().any(|org| org.id == licence.org) {
            return Err(format!(
                "licence: org {} is not one of this port's orgs",
                licence.org
            ));
        }
        if licence.fee_major == 0 || licence.fee_major > MAX_LICENCE_FEE_MAJOR {
            return Err(format!(
                "licence: fee_major must be 1..={MAX_LICENCE_FEE_MAJOR}, not {}",
                licence.fee_major
            ));
        }
        if licence.term_ic_days == 0 || licence.term_ic_days > MAX_LICENCE_TERM_IC_DAYS {
            return Err(format!(
                "licence: term_ic_days must be 1..={MAX_LICENCE_TERM_IC_DAYS}, not {}",
                licence.term_ic_days
            ));
        }
        if licence.grace_ic_days > licence.term_ic_days {
            return Err(format!(
                "licence: grace_ic_days must be at most the term ({}), not {}",
                licence.term_ic_days, licence.grace_ic_days
            ));
        }
        // Doc 20 cut SM.10 (§9.3, lead ruling 3): the claim fees beside it.
        for (field, value) in [
            ("claim_stake_major", licence.claim_stake_major),
            ("claim_term_major", licence.claim_term_major),
        ] {
            if value == 0 || value > MAX_LICENCE_FEE_MAJOR {
                return Err(format!(
                    "licence: {field} must be 1..={MAX_LICENCE_FEE_MAJOR}, not {value}"
                ));
            }
        }
        Ok(())
    }

    /// **The rig pool's bounds** (doc 20 cut SM.3a; §4.3, Q-20-6): its org one
    /// of the entry's, its counter a key of `rooms`, the issuer a system
    /// principal, the shed a label, the pool, prefix and term in bounds, the
    /// money positive, and every tool a row of the compiled tools table, none
    /// twice. Each sentence starts `vehicles {id}: `.
    fn check_vehicles(&self, vehicles: &VehiclesDef) -> Result<(), String> {
        let vid = &vehicles.id;
        if !self.orgs.iter().any(|org| org.id == vehicles.org) {
            return Err(format!(
                "vehicles {vid}: org {} is not one of this port's orgs",
                vehicles.org
            ));
        }
        if !self.rooms.contains_key(&vehicles.counter) {
            return Err(format!(
                "vehicles {vid}: counter `{}` is not one of this port's rooms",
                vehicles.counter
            ));
        }
        if !principal_ok(&vehicles.issuer) {
            return Err(format!(
                "vehicles {vid}: issuer `{}` is not a system principal id",
                vehicles.issuer
            ));
        }
        if !label_ok(&vehicles.shed) {
            return Err(format!(
                "vehicles {vid}: shed must be non-blank and at most {MAX_LABEL_BYTES} bytes"
            ));
        }
        if vehicles.pool == 0 || vehicles.pool > MAX_VEHICLE_POOL {
            return Err(format!(
                "vehicles {vid}: pool must be 1..={MAX_VEHICLE_POOL}, not {}",
                vehicles.pool
            ));
        }
        let prefix = &vehicles.registry_prefix;
        if prefix.is_empty()
            || prefix.len() > MAX_REGISTRY_PREFIX_BYTES
            || !prefix
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(format!(
                "vehicles {vid}: registry_prefix must be 1..={MAX_REGISTRY_PREFIX_BYTES} bytes \
                 of [A-Z0-9-], not `{prefix}`"
            ));
        }
        for (field, major) in [
            ("fee_major", vehicles.fee_major),
            ("deposit_major", vehicles.deposit_major),
        ] {
            if major == 0 || major > MAX_PORT_CAPITAL_MAJOR {
                return Err(format!(
                    "vehicles {vid}: {field} must be 1..={MAX_PORT_CAPITAL_MAJOR}, not {major}"
                ));
            }
        }
        if vehicles.term_ic_days == 0 || vehicles.term_ic_days > MAX_VEHICLE_TERM_IC_DAYS {
            return Err(format!(
                "vehicles {vid}: term_ic_days must be 1..={MAX_VEHICLE_TERM_IC_DAYS}, not {}",
                vehicles.term_ic_days
            ));
        }
        if vehicles.tools.is_empty() || vehicles.tools.len() > MAX_VEHICLE_TOOLS {
            return Err(format!(
                "vehicles {vid}: tools names 1..={MAX_VEHICLE_TOOLS} tools, not {}",
                vehicles.tools.len()
            ));
        }
        let table = crate::definitions::tools::tools_table()
            .map_err(|error| format!("vehicles {vid}: the tools table does not load: {error}"))?;
        let mut seen: BTreeSet<&CanonicalId> = BTreeSet::new();
        for tool in &vehicles.tools {
            if !table.contains_key(tool) {
                return Err(format!("vehicles {vid}: tool {tool} is not a shipped tool"));
            }
            if !seen.insert(tool) {
                return Err(format!("vehicles {vid}: tool {tool} is named twice"));
            }
        }
        Ok(())
    }

    /// **The emergency block's bounds** (doc 19 cut A.4b): a premium to take the
    /// cap from; the ratio in `1000..=MAX_EMERGENCY_RATIO_PERMILLE`; one to
    /// `MAX_EMERGENCY_GOODS` goods, none twice; an intake the market buys; an
    /// `org` that is one of the port's vendors and neither asks nor bids any of
    /// the goods (its storefront would hold two lines for one good).
    fn check_emergency(&self, emergency: &EmergencyDef) -> Result<(), String> {
        if self.premium.is_none() {
            return Err("emergency: needs a premium block to take its cap from".to_owned());
        }
        if emergency.ratio_permille < 1_000
            || emergency.ratio_permille > MAX_EMERGENCY_RATIO_PERMILLE
        {
            return Err(format!(
                "emergency: ratio_permille is 1000..={MAX_EMERGENCY_RATIO_PERMILLE}, not {}",
                emergency.ratio_permille
            ));
        }
        if emergency.goods.is_empty() || emergency.goods.len() > MAX_EMERGENCY_GOODS {
            return Err(format!(
                "emergency: 1..={MAX_EMERGENCY_GOODS} goods, not {}",
                emergency.goods.len()
            ));
        }
        let mut seen: BTreeSet<&CanonicalId> = BTreeSet::new();
        if let Some(good) = emergency.goods.iter().find(|good| !seen.insert(good)) {
            return Err(format!("emergency: {good} is named twice"));
        }
        if emergency.intake == 0 || emergency.intake > mush_market::MAX_QUANTITY {
            return Err(format!(
                "emergency: intake is 1..={}, not {}",
                mush_market::MAX_QUANTITY,
                emergency.intake
            ));
        }
        let Some(vendor) = self
            .vendors
            .iter()
            .find(|vendor| vendor.org == emergency.org)
        else {
            return Err(format!(
                "emergency: org {} is not one of this port's vendors",
                emergency.org
            ));
        };
        let lines = vendor
            .asks
            .iter()
            .map(|ask| ("asks", &ask.good))
            .chain(vendor.bids.iter().map(|bid| ("bids", &bid.good)));
        for (side, good) in lines {
            if emergency.goods.contains(good) {
                return Err(format!(
                    "emergency: org {} already {side} {good}; its storefront cannot hold the \
                     emergency bid too",
                    emergency.org
                ));
            }
        }
        Ok(())
    }

    /// **A vendor's mill rows** (doc 20 cut SM.12a; §7.3 "The mill"), checked in
    /// this order: at most [`MAX_VENDOR_MILLS`] rows; no ore and no product named
    /// twice; the ore one of the vendor's bids and the product one of its asks (so
    /// `ore != into`, a good never being both asked and bid); `1 <= product <= ore
    /// <= MAX_MILL_RATIO`; `batch_t` a whole number of ratios with `1 <= batch_t <=
    /// per_ic_day_t <= mush_market::MAX_QUANTITY`. The family rule (a mill's ore
    /// and product are one family's goods) is [`check_ports`]', where the goods
    /// table is loaded.
    fn check_vendor_mills(
        &self,
        vendor: &VendorDef,
        asks: &BTreeSet<&CanonicalId>,
        bids: &BTreeSet<&CanonicalId>,
    ) -> Result<(), String> {
        let org = &vendor.org;
        if vendor.mill.len() > MAX_VENDOR_MILLS {
            return Err(format!(
                "vendor {org}: at most {MAX_VENDOR_MILLS} mill rows"
            ));
        }
        let mut ores: BTreeSet<&CanonicalId> = BTreeSet::new();
        if let Some(mill) = vendor.mill.iter().find(|mill| !ores.insert(&mill.ore)) {
            return Err(format!("vendor {org}: mill {} is authored twice", mill.ore));
        }
        let mut intos: BTreeSet<&CanonicalId> = BTreeSet::new();
        if let Some(mill) = vendor.mill.iter().find(|mill| !intos.insert(&mill.into)) {
            return Err(format!(
                "vendor {org}: mill into {} is authored twice",
                mill.into
            ));
        }
        for mill in &vendor.mill {
            let ore = &mill.ore;
            if !bids.contains(ore) {
                return Err(format!(
                    "vendor {org}: mill {ore}: the ore must be a good this vendor bids"
                ));
            }
            if !asks.contains(&mill.into) {
                return Err(format!(
                    "vendor {org}: mill {ore}: {} must be a good this vendor asks",
                    mill.into
                ));
            }
            let ratio = mill.ratio;
            if ratio.product == 0 || ratio.product > ratio.ore || ratio.ore > MAX_MILL_RATIO {
                return Err(format!(
                    "vendor {org}: mill {ore}: the ratio is 1..={MAX_MILL_RATIO} tonnes of ore to \
                     1..=that many of product"
                ));
            }
            if mill.batch_t == 0
                || mill.batch_t % u64::from(ratio.ore) != 0
                || mill.batch_t > mill.per_ic_day_t
                || mill.per_ic_day_t > mush_market::MAX_QUANTITY
            {
                return Err(format!(
                    "vendor {org}: mill {ore}: batch_t is a whole number of ratios and at most \
                     per_ic_day_t, which is at most {}",
                    mush_market::MAX_QUANTITY
                ));
            }
        }
        Ok(())
    }

    /// **One vendor's lines** (doc 18 cut IC8; §4.3 "Vendors and buyers"):
    /// stock, ask and bid goods each unique within the vendor; every ask drawn
    /// from a stock row of the same vendor; no good both asked and bid; (doc 20
    /// cut SM.5) a refreshed intake's drip and cap; (doc 20 cut SM.12a) the mill
    /// rows (`check_vendor_mills`); every `boards` role a key of `rooms` other
    /// than `spine`, none repeated in one line.
    fn check_vendor_lines(&self, vendor: &VendorDef) -> Result<(), String> {
        let org = &vendor.org;
        let mut stock: BTreeSet<&CanonicalId> = BTreeSet::new();
        if let Some(row) = vendor.stock.iter().find(|row| !stock.insert(&row.good)) {
            return Err(format!(
                "vendor {org}: stock {} is authored twice",
                row.good
            ));
        }
        let mut asks: BTreeSet<&CanonicalId> = BTreeSet::new();
        if let Some(ask) = vendor.asks.iter().find(|ask| !asks.insert(&ask.good)) {
            return Err(format!("vendor {org}: ask {} is authored twice", ask.good));
        }
        let mut bids: BTreeSet<&CanonicalId> = BTreeSet::new();
        if let Some(bid) = vendor.bids.iter().find(|bid| !bids.insert(&bid.good)) {
            return Err(format!("vendor {org}: bid {} is authored twice", bid.good));
        }
        if let Some(ask) = vendor.asks.iter().find(|ask| !stock.contains(&ask.good)) {
            return Err(format!(
                "vendor {org}: ask {} has no stock row to draw from",
                ask.good
            ));
        }
        if let Some(good) = asks.intersection(&bids).next() {
            return Err(format!("vendor {org}: {good} is both asked and bid"));
        }
        // Doc 20 cut SM.5 (§7.3, Q-20-11): a refreshed intake's drip and cap.
        for bid in &vendor.bids {
            let Some(refresh) = &bid.refresh else {
                continue;
            };
            if refresh.per_ic_day == 0
                || refresh.per_ic_day > refresh.cap
                || refresh.cap > mush_market::MAX_QUANTITY
                || bid.intake > refresh.cap
            {
                return Err(format!(
                    "vendor {org}: bid {}: refresh: per_ic_day and cap are 1..={}, per_ic_day is \
                     at most cap, and the intake is at most the cap",
                    bid.good,
                    mush_market::MAX_QUANTITY
                ));
            }
        }
        self.check_vendor_mills(vendor, &asks, &bids)?;
        let lines = vendor
            .asks
            .iter()
            .map(|ask| ("ask", &ask.good, &ask.boards))
            .chain(
                vendor
                    .bids
                    .iter()
                    .map(|bid| ("bid", &bid.good, &bid.boards)),
            );
        for (side, good, boards) in lines {
            let mut seen: BTreeSet<&CanonicalId> = BTreeSet::new();
            for role in boards {
                if role.as_str() == "spine" || !self.rooms.contains_key(role) {
                    return Err(format!(
                        "vendor {org}: {side} {good}: board `{role}` is not one of the port's rooms"
                    ));
                }
                if !seen.insert(role) {
                    return Err(format!(
                        "vendor {org}: {side} {good}: board `{role}` is named twice"
                    ));
                }
            }
        }
        Ok(())
    }

    /// The programme's bounds, D-18-25's floor arithmetic among them.
    fn check_programme(&self, programme: &ProgrammeDef) -> Result<(), String> {
        let pid = &programme.id;
        let Some(org) = self.orgs.iter().find(|org| org.id == programme.org) else {
            return Err(format!(
                "programme {pid}: org {} is not one of this port's orgs",
                programme.org
            ));
        };
        room_key(&self.area, &programme.office)
            .map_err(|detail| format!("programme {pid}: office: {detail}"))?;
        if !principal_ok(&programme.issuer) {
            return Err(format!(
                "programme {pid}: issuer `{}` is not a system principal id",
                programme.issuer
            ));
        }
        if programme.floor_major == 0 || programme.stake_major == 0 || programme.buyout_major == 0 {
            return Err(format!(
                "programme {pid}: floor_major, stake_major and buyout_major must be at least 1"
            ));
        }
        if programme.pool == 0 || programme.pool > MAX_PROGRAMME_POOL {
            return Err(format!(
                "programme {pid}: pool must be 1..={MAX_PROGRAMME_POOL}, not {}",
                programme.pool
            ));
        }
        let pool = usize::try_from(programme.pool).unwrap_or(usize::MAX);
        if programme.hull_names.len() != pool {
            return Err(format!(
                "programme {pid}: hull_names must name {} hulls, one per pool slot, not {}",
                programme.pool,
                programme.hull_names.len()
            ));
        }
        let mut names: BTreeSet<&str> = BTreeSet::new();
        for name in &programme.hull_names {
            if !label_ok(name) {
                return Err(format!(
                    "programme {pid}: a hull name must be non-blank and at most {MAX_LABEL_BYTES} bytes"
                ));
            }
            if !names.insert(name.as_str()) {
                return Err(format!(
                    "programme {pid}: hull name `{name}` is authored twice"
                ));
            }
        }
        let prefix = &programme.registry_prefix;
        if prefix.is_empty()
            || prefix.len() > MAX_REGISTRY_PREFIX_BYTES
            || !prefix
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(format!(
                "programme {pid}: registry_prefix must be 1..={MAX_REGISTRY_PREFIX_BYTES} bytes \
                 of [A-Z0-9-], not `{prefix}`"
            ));
        }
        if programme.share_permille > 1_000 {
            return Err(format!(
                "programme {pid}: share_permille must be 0..=1000, not {}",
                programme.share_permille
            ));
        }
        if programme.dormancy_real_days == 0
            || programme.dormancy_real_days > MAX_DORMANCY_REAL_DAYS
        {
            return Err(format!(
                "programme {pid}: dormancy_real_days must be 1..={MAX_DORMANCY_REAL_DAYS}, not {}",
                programme.dormancy_real_days
            ));
        }
        // Doc 20 cut SM.3a (§4.3, D-20-7): the refit rows. `refit_compatible` is
        // the apply preflight's, where the catalog is.
        if programme.refits.len() > MAX_PROGRAMME_REFITS {
            return Err(format!(
                "programme {pid}: refits holds at most {MAX_PROGRAMME_REFITS} rows, not {}",
                programme.refits.len()
            ));
        }
        for (word, unit) in &programme.refits {
            if word.as_str() == STANDARD_REFIT {
                return Err(format!(
                    "programme {pid}: refit `{STANDARD_REFIT}` is the programme's own unit and \
                     is never authored"
                ));
            }
            if *unit == programme.unit {
                return Err(format!(
                    "programme {pid}: refit `{word}` names the programme's own unit {unit}"
                ));
            }
        }
        // Doc 18 cut IC6 (D-18-25, lead ruling L5): one `tacsim:coop:{account}`
        // document per gateway account holds one enrolment, so the programme can
        // promise exactly one.
        if programme.per_account != 1 {
            return Err(format!(
                "programme {pid}: per_account must be 1; this server keeps one enrolment per \
                 gateway account (D-18-25)"
            ));
        }
        // D-18-25: the floor plus a stake for every hull fits inside the
        // programme org's capital (50,000 + 8 × 25,000 = 250,000 at v2).
        let needed = programme
            .stake_major
            .checked_mul(u64::from(programme.pool))
            .and_then(|stakes| stakes.checked_add(programme.floor_major));
        // Doc 20 cut SM.5: the capital with its raises (the Trust raises nothing).
        let capital = org.capital_total().unwrap_or(org.capital_major);
        match needed {
            Some(needed) if needed <= capital => Ok(()),
            Some(needed) => Err(format!(
                "programme {pid}: floor_major + pool × stake_major is {needed}, more than org \
                 {}'s capital_major {}",
                org.id, capital
            )),
            None => Err(format!(
                "programme {pid}: floor_major + pool × stake_major overflows"
            )),
        }
    }
}

/// **The yards' pack-wide rules** (doc 20 cut SM.12b; [`check_ports`]' last site rule): for each
/// site's yard, entry order — its org an org of the pack; its pool the pack's one `vehicles` block
/// and that block's org the yard's (the rigs' unit and tools are read from the programme port's
/// pool whatever pool a rig came from, and the one-lessor rule makes that pool the only one); its
/// ore an ore good (`family_of_word`, `ore_good`); exactly one vendor of the house with a mill row
/// of that ore (where the yard delivers); neither prefix any programme's, the pool's or another
/// yard's; one yard an org.
fn check_yards(
    ports: &[(&CanonicalId, &PortDef)],
    orgs: &BTreeMap<&CanonicalId, &CanonicalId>,
) -> Result<(), (CanonicalId, String)> {
    let pool_block = ports.iter().find_map(|(_, def)| def.vehicles.as_deref());
    let mut seen: Vec<(&CanonicalId, &YardDef)> = Vec::new();
    for (id, def) in ports {
        let fail = |detail: String| Err(((*id).clone(), detail));
        let Some(yard) = def.site.as_ref().and_then(|site| site.yard.as_deref()) else {
            continue;
        };
        let org = &yard.org;
        if !orgs.contains_key(org) {
            return fail(format!("site: yard: {org} is not an org of this pack"));
        }
        if pool_block.is_none_or(|pool| pool.id != yard.pool || pool.org != *org) {
            return fail(format!(
                "site: yard: pool {} is not the pack's rig pool of {org}",
                yard.pool
            ));
        }
        let ore_good = crate::surface::geology::family_of_word(yard.ore.as_str())
            .map(crate::surface::opening::ore_good);
        if ore_good != Some(yard.ore.as_str()) {
            return fail(format!("site: yard: {} is not an ore good", yard.ore));
        }
        let mills = ports
            .iter()
            .flat_map(|(_, port)| port.vendors.iter())
            .filter(|vendor| {
                vendor.org == *org && vendor.mill.iter().any(|mill| mill.ore == yard.ore)
            })
            .count();
        if mills != 1 {
            return fail(format!(
                "site: yard: {org} mills no {}, so the yard has nowhere to deliver",
                yard.ore
            ));
        }
        let mut taken: Vec<&str> = ports
            .iter()
            .filter_map(|(_, port)| port.programme.as_ref())
            .map(|programme| programme.registry_prefix.as_str())
            .collect();
        taken.extend(pool_block.map(|pool| pool.registry_prefix.as_str()));
        for (_, other) in &seen {
            taken.push(other.registry_prefix.as_str());
            taken.push(other.rig_prefix.as_str());
        }
        for prefix in [&yard.registry_prefix, &yard.rig_prefix] {
            if taken.contains(&prefix.as_str()) {
                return fail(format!("site: yard: prefix {prefix} is another pool's"));
            }
        }
        if let Some((other, _)) = seen.iter().find(|(_, other)| other.org == *org) {
            return fail(format!("site: yard: {org} already keeps a yard at {other}"));
        }
        seen.push((id, yard));
    }
    Ok(())
}

/// **The rules that span every `port` entry of one pack** (doc 18 §4.7,
/// D-18-47). The detail is returned beside the offending port entry's id; the
/// caller wraps both into one refusal.
///
/// - `location` is a `location` entry of the pack;
/// - venues are unique, and so are locations, across port entries;
/// - org ids are unique **across** port entries (one legal person per id);
/// - every `routes` key is the venue of **another** port entry, and every
///   listed id is a `route` entry from this port's location to that port's;
/// - a programme's `reference` names a trade good of the goods table (one
///   with a family and no `consumable`) and an `<entry>/<org>` of this pack;
///   in an entry that authors vendors (doc 18 cut IC8, lead ruling L23), that
///   org is a vendor of that entry whose asks include the good;
/// - (cut IC8) no stock clerk (`vendors[].agent`) serves two port entries;
/// - (cut IC8) every stock, ask and bid good is a row of the goods table;
/// - (doc 20 cut SM.12a) a mill's ore and product are one family's goods;
/// - (doc 20 cut SM.5) a programme's `reference_minor` names a trade good, at
///   least 1 minor unit, that is not also in `reference` and that **no vendor of
///   any port entry of the pack asks** (§7.4: a fixed reference only for a good
///   no port asks);
/// - (doc 20 cut SM.5) at most one port entry authors a `licence` block, and
///   when one does every site's `issuer` is its org (D-20-12, phase 1);
/// - (doc 20 cut SM.12b) a site's yard names an org of the pack, the pack's one rig
///   pool of that org, an ore good its house mills at exactly one vendor, and
///   prefixes no programme, pool or other yard uses; one yard an org (`check_yards`).
///
/// # Errors
///
/// `(port entry id, detail)` for the first rule broken.
pub fn check_ports(
    entries: &BTreeMap<CanonicalId, PackEntry>,
) -> Result<(), (CanonicalId, String)> {
    let ports: Vec<(&CanonicalId, &PortDef)> = entries
        .iter()
        .filter_map(|(id, entry)| match entry {
            PackEntry::Port(def) => Some((id, def)),
            _ => None,
        })
        .collect();
    if ports.is_empty() {
        return Ok(());
    }
    let mut venues: BTreeMap<&CanonicalId, &CanonicalId> = BTreeMap::new();
    let mut locations: BTreeSet<&CanonicalId> = BTreeSet::new();
    let mut orgs: BTreeMap<&CanonicalId, &CanonicalId> = BTreeMap::new();
    let mut agents: BTreeMap<&CanonicalId, &CanonicalId> = BTreeMap::new();
    // Doc 20 cut SM.3a (reviewer F11): one rig lessor a lane.
    let mut lessor: Option<&CanonicalId> = None;
    // Doc 20 cut SM.5: the one licence block's port and org.
    let mut licensor: Option<(&CanonicalId, &CanonicalId)> = None;
    for (id, def) in &ports {
        let fail = |detail: String| Err(((*id).clone(), detail));
        // Doc 20 cut SM.3a (reviewer F11): at most one port of the pack authors a
        // rig pool, and that port runs the programme the rigs are leased into —
        // so the vehicles port **is** the programme port. The one-lessor rule is
        // checked first (entries iterate in id order).
        if let Some(vehicles) = &def.vehicles {
            if let Some(other) = lessor {
                return fail(format!(
                    "vehicles: port {id} also authors a vehicles block (port {other}'s); a lane \
                     has one rig lessor"
                ));
            }
            if def.programme.is_none() {
                return fail(format!(
                    "vehicles {}: port {id} authors no programme; rigs are leased into the \
                     programme's hulls",
                    vehicles.id
                ));
            }
            lessor = Some(*id);
        }
        // Doc 20 cut SM.5 (§9.1, D-20-12): one licence issuer a lane in phase 1.
        if let Some(licence) = &def.licence {
            if let Some((other, _)) = licensor {
                return fail(format!(
                    "licence: port {id} also authors a licence block (port {other}'s); a lane \
                     has one licence issuer in phase 1"
                ));
            }
            licensor = Some((*id, &licence.org));
        }
        // Doc 18 cut IC8 (lead ruling L2): a stock clerk is one port's.
        if let Some(vendor) = def.vendors.first()
            && let Some(other) = agents.insert(&vendor.agent, id)
        {
            return fail(format!(
                "vendors: agent {} is also port {other}'s stock clerk",
                vendor.agent
            ));
        }
        if !matches!(entries.get(&def.location), Some(PackEntry::Location(_))) {
            return fail(format!(
                "location {} is not a location entry of this pack",
                def.location
            ));
        }
        if let Some(other) = venues.insert(&def.venue, id) {
            return fail(format!("venue {} is also port {other}'s", def.venue));
        }
        if !locations.insert(&def.location) {
            return fail(format!(
                "location {} is another port's location",
                def.location
            ));
        }
        for org in &def.orgs {
            if let Some(other) = orgs.insert(&org.id, id) {
                return fail(format!("org {} is also authored by port {other}", org.id));
            }
        }
    }
    // Doc 20 cut SM.2 (§3.1, D-20-2): a site's place, body and Registry.
    for (id, def) in &ports {
        let fail = |detail: String| Err(((*id).clone(), detail));
        let Some(PackEntry::Location(place)) = entries.get(&def.location) else {
            continue;
        };
        let is_site_place = place.role.as_str() == crate::pack::SITE_ROLE;
        let Some(site) = &def.site else {
            if is_site_place {
                return fail(format!(
                    "location {} has role {}, which only a site's port may stand on",
                    def.location,
                    crate::pack::SITE_ROLE
                ));
            }
            continue;
        };
        if !is_site_place {
            return fail(format!(
                "site: location {} has role {}, not {}",
                def.location,
                place.role,
                crate::pack::SITE_ROLE
            ));
        }
        let Some(body) = &place.body else {
            return fail(format!("site: location {} stands on no body", def.location));
        };
        if place.lat_mdeg.is_none() || place.lon_mdeg.is_none() {
            return fail(format!(
                "site: location {} authors no lat_mdeg and lon_mdeg",
                def.location
            ));
        }
        let Some(PackEntry::Body(figures)) = entries.get(body) else {
            return fail(format!(
                "site: body {body} is not a body entry of this pack"
            ));
        };
        for (field, missing) in [
            ("surface_mg", figures.surface_mg.is_none()),
            ("radius_km", figures.radius_km.is_none()),
            ("geology", figures.geology.is_none()),
        ] {
            if missing {
                return fail(format!("site: body {body} authors no {field}"));
            }
        }
        if site.name != place.label {
            return fail(format!(
                "site: name `{}` is not its location's label `{}`",
                site.name, place.label
            ));
        }
        if !orgs.contains_key(&site.issuer) {
            return fail(format!(
                "site: issuer {} is not an org of any port of this pack",
                site.issuer
            ));
        }
        // Doc 20 cut SM.5 (D-20-12): phase 1 has one instrument, so every site is
        // the licence issuer's.
        if let Some((_, org)) = licensor
            && site.issuer != *org
        {
            return fail(format!(
                "site: issuer {} is not the lane's licence issuer {org}",
                site.issuer
            ));
        }
        for (letter, row) in &site.blocks {
            if let Some(holder) = &row.holder
                && !orgs.contains_key(holder)
            {
                return fail(format!(
                    "site: block {letter}: holder {holder} is not an org of any port of this pack"
                ));
            }
            // Doc 20 cut SM.10 (§9.6): a paper is another issuer's.
            if let Some(paper) = &row.paper {
                if !orgs.contains_key(&paper.issuer) {
                    return fail(format!(
                        "site: block {letter}: paper issuer {} is not an org of any port of this \
                         pack",
                        paper.issuer
                    ));
                }
                if paper.issuer == site.issuer {
                    return fail(format!(
                        "site: block {letter}: a paper's issuer is never the site's own issuer {}",
                        site.issuer
                    ));
                }
            }
        }
    }
    // Doc 20 cut SM.12b (§12.3; the lead's rulings 1–2, 5): a yard's house, its pool, its ore,
    // its house's one mill of that ore, and prefixes no other pool, programme or yard uses.
    check_yards(&ports, &orgs)?;
    for (id, def) in &ports {
        let fail = |detail: String| Err(((*id).clone(), detail));
        for (venue, list) in &def.routes {
            let Some((_, there)) = ports
                .iter()
                .find(|(other, port)| other != id && &port.venue == venue)
            else {
                return fail(format!(
                    "routes.{venue}: {venue} is not the venue of another port of this pack"
                ));
            };
            for route in list {
                let Some(PackEntry::Route(entry)) = entries.get(route) else {
                    return fail(format!(
                        "routes.{venue}: {route} is not a route entry of this pack"
                    ));
                };
                if entry.from != def.location || entry.to != there.location {
                    return fail(format!(
                        "routes.{venue}: route {route} runs {} to {}, not {} to {}",
                        entry.from, entry.to, def.location, there.location
                    ));
                }
            }
        }
        let references = def.programme.as_ref().filter(|programme| {
            !programme.reference.is_empty() || !programme.reference_minor.is_empty()
        });
        if references.is_none() && def.vendors.is_empty() {
            continue;
        }
        // One load of the goods table for both rules below.
        let goods = crate::definitions::goods::goods_table()
            .map_err(|error| ((*id).clone(), format!("the goods table: {error}")))?;
        // Doc 18 cut IC8: every stock, ask and bid good is a row of the goods table.
        for vendor in &def.vendors {
            let named = vendor
                .stock
                .iter()
                .map(|row| &row.good)
                .chain(vendor.asks.iter().map(|ask| &ask.good))
                .chain(vendor.bids.iter().map(|bid| &bid.good));
            for good in named {
                if goods.good(good).is_none() {
                    return fail(format!(
                        "vendor {}: {good} is not a good of the goods table",
                        vendor.org
                    ));
                }
            }
            // Doc 20 cut SM.12a (§7.3 "The mill"): a mill turns an ore into its own
            // family's good (metal ore into metal concentrate, ice ore into water).
            for mill in &vendor.mill {
                let family = |good: &CanonicalId| {
                    goods
                        .good(good)
                        .and_then(|row| row.family.as_ref().cloned())
                };
                let one_family = match (family(&mill.ore), family(&mill.into)) {
                    (Some(ore), Some(into)) => ore == into,
                    _ => false,
                };
                if !one_family {
                    return fail(format!(
                        "vendor {}: mill {}: {} and {} are not one family's goods",
                        vendor.org, mill.ore, mill.ore, mill.into
                    ));
                }
            }
        }
        // Doc 19 cut A.4b: every emergency good is a row of the goods table.
        if let Some(emergency) = &def.emergency
            && let Some(good) = emergency
                .goods
                .iter()
                .find(|good| goods.good(good).is_none())
        {
            return fail(format!(
                "emergency: {good} is not a good of the goods table"
            ));
        }
        if let Some(programme) = references {
            for (good, listing) in &programme.reference {
                let trade = goods
                    .good(good)
                    .is_some_and(|row| row.family.is_some() && row.consumable.is_none());
                if !trade {
                    return fail(format!(
                        "programme {}: reference {good} is not a trade good of the goods table",
                        programme.id
                    ));
                }
                let named = listing.split_once('/').and_then(|(entry, org)| {
                    let entry = CanonicalId::parse_canonical(entry).ok()?;
                    let org = CanonicalId::parse_canonical(org).ok()?;
                    match entries.get(&entry) {
                        Some(PackEntry::Port(port)) => port
                            .orgs
                            .iter()
                            .any(|row| row.id == org)
                            .then_some((port, org)),
                        _ => None,
                    }
                });
                let Some((port, org)) = named else {
                    return fail(format!(
                        "programme {}: reference {good} names `{listing}`, which is not \
                         `<port entry>/<org>` of this pack",
                        programme.id
                    ));
                };
                // IC3 L8's deferred rule, landed by doc 18 cut IC8 and scoped to an
                // entry that authors vendors (lead ruling L23): the published v2
                // (references, no vendors) keeps loading as a latched or seeded pack.
                if !def.vendors.is_empty() {
                    let asks = port.vendors.iter().any(|vendor| {
                        vendor.org == org && vendor.asks.iter().any(|ask| &ask.good == good)
                    });
                    if !asks {
                        return fail(format!(
                            "programme {}: reference {good} names `{listing}`, which posts no \
                             ask for it",
                            programme.id
                        ));
                    }
                }
            }
            // Doc 20 cut SM.5 (§7.4, Q-20-5, amending D-18-24 for a good no port
            // asks): a fixed reference, in minor units. The live listing stays the
            // rule for every good some port asks, so a fixed one is refused there.
            let pid = &programme.id;
            for (good, minor) in &programme.reference_minor {
                let trade = goods
                    .good(good)
                    .is_some_and(|row| row.family.is_some() && row.consumable.is_none());
                if !trade {
                    return fail(format!(
                        "programme {pid}: reference_minor {good} is not a trade good of the goods \
                         table"
                    ));
                }
                if *minor == 0 {
                    return fail(format!(
                        "programme {pid}: reference_minor {good} must be at least 1"
                    ));
                }
                if programme.reference.contains_key(good) {
                    return fail(format!(
                        "programme {pid}: {good} has both a reference listing and a fixed reference"
                    ));
                }
                let asker = ports.iter().find(|(_, port)| {
                    port.vendors
                        .iter()
                        .any(|vendor| vendor.asks.iter().any(|ask| &ask.good == good))
                });
                if let Some((entry, _)) = asker {
                    return fail(format!(
                        "programme {pid}: reference_minor {good}: a fixed reference is only for a \
                         good no port of this lane asks, and port {entry} asks it"
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(word: &str) -> CanonicalId {
        CanonicalId::new(word).unwrap_or_else(|_| CanonicalId::new("x").expect("x"))
    }

    fn org(id: &str, capital: u64) -> PortOrgDef {
        PortOrgDef {
            id: cid(id),
            name: format!("The {id}"),
            capital_major: capital,
            faction: cid("freeport"),
            raises: Vec::new(),
        }
    }

    fn port() -> PortDef {
        PortDef {
            location: cid("here"),
            venue: cid("ceres"),
            area: cid("neighborhood.ceres_freeport"),
            gallery: "neighborhood.ceres_freeport.room.c01_berth_gallery".to_owned(),
            rooms: BTreeMap::from([(
                cid("registry"),
                "neighborhood.ceres_freeport.room.c05_registry".to_owned(),
            )]),
            treasury: "tacsim.treasury".to_owned(),
            orgs: vec![org("coop", 3_100_000), org("trust", 250_000)],
            programme: Some(ProgrammeDef {
                id: cid("coop.first_captain"),
                org: cid("trust"),
                office: "neighborhood.ceres_freeport.room.c05_registry".to_owned(),
                floor_major: 50_000,
                unit: cid("tacsim.unit.dray"),
                issuer: "tacsim.registry".to_owned(),
                pool: 2,
                registry_prefix: "FPC-D-04".to_owned(),
                hull_names: vec!["Slack Water".to_owned(), "Tally Ho".to_owned()],
                stake_major: 25_000,
                share_permille: 250,
                reference: BTreeMap::new(),
                reference_minor: BTreeMap::new(),
                buyout_major: 90_000,
                dormancy_real_days: 7,
                per_account: 1,
                refits: BTreeMap::new(),
            }),
            vendors: Vec::new(),
            routes: BTreeMap::from([(cid("stickney"), vec![cid("out")])]),
            premium: None,
            emergency: None,
            site: None,
            vehicles: None,
            licence: None,
        }
    }

    fn refused(def: &PortDef) -> String {
        def.check().err().unwrap_or_else(|| "accepted".to_owned())
    }

    /// D-18-14: greedy chunks of 800,000, the remainder last, none for zero.
    #[test]
    fn ic3_tranches_are_greedy_and_bounded() {
        assert_eq!(
            tranches(3_100_000),
            vec![800_000, 800_000, 800_000, 700_000]
        );
        assert_eq!(tranches(4_000_000), vec![800_000; 5]);
        assert_eq!(tranches(50_000), vec![50_000]);
        assert_eq!(tranches(800_000), vec![800_000]);
        assert_eq!(tranches(800_001), vec![800_000, 1]);
        assert!(tranches(0).is_empty());
    }

    /// L2: one split rule, the port's own area.
    #[test]
    fn ic3_room_key_splits_on_the_area() {
        let area = cid("neighborhood.ceres_freeport");
        let key =
            room_key(&area, "neighborhood.ceres_freeport.room.c05_registry").expect("in the area");
        assert_eq!(key.pack, area);
        assert_eq!(key.entry.as_str(), "room.c05_registry");
        let outside = room_key(&area, "neighborhood.stickney_yards.room.p01_berth_gallery")
            .expect_err("outside the area");
        assert!(outside.contains("is not in the port's area"), "{outside}");
        assert!(room_key(&area, "neighborhood.ceres_freeport.").is_err());
        assert!(room_key(&area, "neighborhood.ceres_freeport.Room C05").is_err());
    }

    /// One refusal per bound family; the authored shape is accepted.
    #[test]
    fn ic3_port_check_refuses_each_bound() {
        assert_eq!(refused(&port()), "accepted");

        let mut def = port();
        def.orgs.clear();
        assert!(refused(&def).starts_with("orgs:"), "{}", refused(&def));

        let mut def = port();
        def.orgs.push(org("coop", 1));
        assert!(refused(&def).contains("authored twice"));

        let mut def = port();
        def.orgs[0].capital_major = 0;
        assert_eq!(
            refused(&def),
            "org coop: capital_major must be 1..=100000000, not 0"
        );

        let mut def = port();
        def.orgs[0].name = " ".to_owned();
        assert!(refused(&def).contains("name must be non-blank"));

        let mut def = port();
        def.gallery = "elsewhere.room.x".to_owned();
        assert!(refused(&def).starts_with("gallery:"));

        let mut def = port();
        def.rooms.insert(cid("bar"), def.gallery.clone());
        assert!(refused(&def).contains("is not a port room role"));

        let mut def = port();
        def.treasury = "Not A Principal!".to_owned();
        assert!(refused(&def).starts_with("treasury:"));

        let mut def = port();
        def.routes.clear();
        assert!(refused(&def).starts_with("routes:"));

        let mut def = port();
        def.routes.insert(cid("stickney"), Vec::new());
        assert!(refused(&def).starts_with("routes.stickney:"));

        let mut def = port();
        def.routes
            .insert(cid("stickney"), vec![cid("out"), cid("out")]);
        assert!(refused(&def).contains("listed twice"));

        let programme = |edit: &dyn Fn(&mut ProgrammeDef)| {
            let mut def = port();
            if let Some(p) = def.programme.as_mut() {
                edit(p);
            }
            refused(&def)
        };
        assert!(programme(&|p| p.org = cid("nobody")).contains("is not one of this port's orgs"));
        assert!(programme(&|p| p.pool = 0).contains("pool must be"));
        assert!(
            programme(&|p| {
                p.hull_names.pop();
            })
            .contains("hull_names")
        );
        assert!(
            programme(&|p| p.hull_names[1] = "Slack Water".to_owned()).contains("authored twice")
        );
        assert!(programme(&|p| p.registry_prefix = "fpc".to_owned()).contains("registry_prefix"));
        assert!(programme(&|p| p.share_permille = 1_001).contains("share_permille"));
        assert!(programme(&|p| p.dormancy_real_days = 366).contains("dormancy_real_days"));
        assert!(programme(&|p| p.per_account = 0).contains("per_account"));
        // Doc 18 cut IC6: exactly one, never more.
        assert!(programme(&|p| p.per_account = 2).contains("per_account must be 1"));
        assert!(programme(&|p| p.stake_major = 0).contains("at least 1"));
        assert!(programme(&|p| p.issuer = String::new()).contains("issuer"));
        assert!(
            programme(&|p| p.stake_major = 200_000).contains("floor_major + pool × stake_major")
        );

        let vendor = VendorDef {
            org: cid("coop"),
            storefront: "neighborhood.ceres_freeport.room.c07_exchange".to_owned(),
            agent: cid("npc.someone"),
            signatory: "neighborhood.ceres_freeport.npc.someone".to_owned(),
            stock: Vec::new(),
            asks: vec![AskDef {
                good: cid("metals"),
                price_major: 0,
                boards: Vec::new(),
            }],
            bids: Vec::new(),
            mill: Vec::new(),
        };
        let mut def = port();
        def.vendors.push(vendor.clone());
        assert!(refused(&def).contains("price_major must be at least 1"));
        let mut def = port();
        def.vendors.push(VendorDef {
            org: cid("nobody"),
            ..vendor
        });
        assert!(refused(&def).contains("is not one of this port's orgs"));
    }

    /// Doc 18 cut IC14 rows 28 and 29 (lead ruling L24): a stock row's quantity is
    /// bounded by the market's own cap, and a port authors no more stock rows across its
    /// vendors than its clerk carries.
    #[test]
    fn ic14_stock_rows_are_bounded_by_the_market_and_the_clerk() {
        let row = |good: &str, qty: u64| StockRowDef {
            good: cid(good),
            qty,
            grade: 0,
            issue: 1,
        };
        let vendor = |org: &str, stock: Vec<StockRowDef>| VendorDef {
            org: cid(org),
            storefront: "neighborhood.ceres_freeport.room.c07_exchange".to_owned(),
            agent: cid("npc.someone"),
            signatory: "neighborhood.ceres_freeport.npc.someone".to_owned(),
            stock,
            asks: Vec::new(),
            bids: Vec::new(),
            mill: Vec::new(),
        };
        let mut def = port();
        def.vendors.push(vendor(
            "coop",
            vec![row("metals", mush_market::MAX_QUANTITY + 1)],
        ));
        assert_eq!(
            refused(&def),
            format!(
                "vendor coop: stock metals: qty must be 1..={} and issue at least 1 and grade \
                 0..=100",
                mush_market::MAX_QUANTITY
            )
        );
        let mut def = port();
        def.vendors.push(vendor(
            "coop",
            vec![row("metals", mush_market::MAX_QUANTITY)],
        ));
        assert!(
            !refused(&def).contains("qty must be"),
            "the cap itself is admitted: {}",
            refused(&def)
        );
        // Five vendors of thirteen rows each: 65, one past the clerk's 64.
        let mut def = port();
        for n in 0..5 {
            let id = format!("org{n}");
            def.orgs.push(org(&id, 1_000));
            let rows = (0..13).map(|g| row(&format!("good{g}"), 1)).collect();
            def.vendors.push(vendor(&id, rows));
        }
        assert_eq!(
            refused(&def),
            format!(
                "vendors: a port stocks at most {} rows across its vendors, not 65",
                crate::port::stock::MAX_CLERK_ROWS
            )
        );
    }

    /// A site's port: a pad gallery, one route, the site block.
    fn site_port() -> PortDef {
        PortDef {
            location: cid("ahuna_flank"),
            venue: cid("ahuna"),
            area: cid("neighborhood.ahuna_flank"),
            gallery: "neighborhood.ahuna_flank.room.pad".to_owned(),
            rooms: BTreeMap::new(),
            treasury: default_treasury(),
            orgs: Vec::new(),
            programme: None,
            vendors: Vec::new(),
            routes: BTreeMap::from([(cid("ceres"), vec![cid("ahuna_ceres")])]),
            premium: None,
            emergency: None,
            site: Some(Box::new(SiteDef {
                name: "Ahuna Flank".to_owned(),
                issuer: cid("freeport_registry"),
                zone: Zone::Core,
                tenure: SiteTenureDef {
                    default: Tenure::Open,
                },
                blocks: BTreeMap::new(),
                papers: BTreeMap::new(),
                yard: None,
            })),
            vehicles: None,
            licence: None,
        }
    }

    /// **Doc 20 cut SM.2 (§3.1)**: the site block's own bounds — zero orgs and no
    /// rooms are legal for a site and only for a site; each other rule refuses by
    /// name; the letters map onto SM.1's south-west-indexed shifts from the
    /// north-west.
    #[test]
    fn sm2_the_site_block_keeps_its_own_bounds() {
        assert_eq!(refused(&site_port()), "accepted");
        let site = |edit: &dyn Fn(&mut SiteDef)| {
            let mut def = site_port();
            if let Some(site) = def.site.as_mut() {
                edit(site);
            }
            refused(&def)
        };
        let mut def = site_port();
        def.orgs.push(org("coop", 1));
        assert!(refused(&def).starts_with("site: a site authors no orgs"));
        let mut def = site_port();
        def.rooms.insert(cid("registry"), def.gallery.clone());
        assert!(refused(&def).starts_with("site: a site authors no orgs"));
        let mut def = site_port();
        def.site = None;
        assert!(
            refused(&def).starts_with("orgs:"),
            "a station still authors orgs"
        );
        assert!(site(&|s| s.tenure.default = Tenure::Held).starts_with("site: the default tenure"));
        let row = |tenure: Option<Tenure>, holder: Option<&str>| BlockDef {
            tenure,
            holder: holder.map(cid),
            ..BlockDef::default()
        };
        assert_eq!(
            site(&|s| {
                s.blocks
                    .insert("F".to_owned(), row(Some(Tenure::Held), None));
            }),
            "site: block F: a held block names its holder, and only a held block does"
        );
        assert!(
            site(&|s| {
                s.blocks
                    .insert("F".to_owned(), row(Some(Tenure::Open), Some("occator")));
            })
            .contains("a held block names its holder")
        );
        assert_eq!(
            site(&|s| {
                s.blocks
                    .insert("F".to_owned(), row(Some(Tenure::Held), Some("occator")));
            }),
            "accepted"
        );
        assert!(
            site(&|s| {
                s.blocks.insert("Q".to_owned(), BlockDef::default());
            })
            .contains("is not a block letter")
        );
        assert!(
            site(&|s| {
                s.blocks.insert("AB".to_owned(), BlockDef::default());
            })
            .contains("is not a block letter")
        );
        let shifted = |p: i8| {
            site(&|s| {
                s.blocks.insert(
                    "A".to_owned(),
                    BlockDef {
                        prospectivity: p,
                        ..BlockDef::default()
                    },
                );
            })
        };
        assert_eq!(shifted(3), "accepted");
        assert_eq!(shifted(-3), "accepted");
        assert!(shifted(4).contains("prospectivity is -3 to 3, not 4"));
        let worked = |w: u16| {
            site(&|s| {
                s.blocks.insert(
                    "A".to_owned(),
                    BlockDef {
                        worked_permille: w,
                        ..BlockDef::default()
                    },
                );
            })
        };
        assert_eq!(worked(1_000), "accepted");
        assert!(worked(1_001).contains("worked_permille is 0 to 1000, not 1001"));
        assert!(site(&|s| s.name = " ".to_owned()).starts_with("site: name"));
        // `frontier` does not parse (§9.5); `core` does.
        assert!(serde_json::from_str::<Zone>("\"frontier\"").is_err());
        assert_eq!(
            serde_json::from_str::<Zone>("\"core\"").ok(),
            Some(Zone::Core)
        );

        // The letters, from the north-west, onto SM.1's south-west slots: A is
        // the north-west block (bx 0, by 3 → slot 12), D the north-east (slot 15),
        // M the south-west (slot 0), P the south-east (slot 3).
        let mut def = site_port();
        if let Some(site) = def.site.as_mut() {
            for (letter, p) in [("A", 1), ("D", 2), ("M", -1), ("P", -2), ("F", 3)] {
                site.blocks.insert(
                    letter.to_owned(),
                    BlockDef {
                        prospectivity: p,
                        ..BlockDef::default()
                    },
                );
            }
        }
        let shifts = def.site.as_deref().map(SiteDef::shifts).unwrap_or_default();
        let mut want = [0_i8; 16];
        want[12] = 1;
        want[15] = 2;
        want[0] = -1;
        want[3] = -2;
        want[9] = 3; // F: i 5 → bx 1, by 2 → slot 9
        assert_eq!(shifts, want);
        // The serde defaults: a site's entry re-encodes without rooms or orgs, with
        // the treasury.
        let json = serde_json::to_value(site_port()).unwrap_or_default();
        assert!(json.get("rooms").is_none() && json.get("orgs").is_none());
        assert_eq!(json["treasury"], "tacsim.treasury");
    }

    /// A vendor of `org` asking `asks` (each with a stock row) and bidding `bids`.
    fn sm5_vendor(org: &str, agent: &str, asks: &[&str], bids: Vec<BidDef>) -> VendorDef {
        VendorDef {
            org: cid(org),
            storefront: "neighborhood.ceres_freeport.room.c07_exchange".to_owned(),
            agent: cid(agent),
            signatory: "neighborhood.ceres_freeport.npc.someone".to_owned(),
            stock: asks
                .iter()
                .map(|good| StockRowDef {
                    good: cid(good),
                    qty: 10,
                    grade: 50,
                    issue: 1,
                })
                .collect(),
            asks: asks
                .iter()
                .map(|good| AskDef {
                    good: cid(good),
                    price_major: 100,
                    boards: Vec::new(),
                })
                .collect(),
            bids,
            mill: Vec::new(),
        }
    }

    fn sm5_bid(good: &str, intake: u64, refresh: Option<(u64, u64)>) -> BidDef {
        BidDef {
            good: cid(good),
            price_major: 70,
            intake,
            as_name: None,
            boards: Vec::new(),
            refresh: refresh.map(|(per_ic_day, cap)| RefreshDef { per_ic_day, cap }),
        }
    }

    /// **Doc 20 cut SM.5 (§7.3, Q-20-11)**: each bound of a refresh row refused with
    /// its sentence; the lane's four rows accepted.
    #[test]
    fn sm5_a_refresh_row_is_bounded() {
        let with = |bid: BidDef| {
            let mut def = port();
            def.vendors
                .push(sm5_vendor("coop", "npc.someone", &[], vec![bid]));
            refused(&def)
        };
        let sentence = |good: &str| {
            format!(
                "vendor coop: bid {good}: refresh: per_ic_day and cap are 1..={}, per_ic_day is \
                 at most cap, and the intake is at most the cap",
                mush_market::MAX_QUANTITY
            )
        };
        assert_eq!(
            with(sm5_bid("metal_ore", 240, Some((0, 240)))),
            sentence("metal_ore")
        );
        assert_eq!(
            with(sm5_bid("metal_ore", 240, Some((241, 240)))),
            sentence("metal_ore")
        );
        assert_eq!(
            with(sm5_bid("metal_ore", 241, Some((40, 240)))),
            sentence("metal_ore")
        );
        assert_eq!(
            with(sm5_bid(
                "metal_ore",
                1,
                Some((1, mush_market::MAX_QUANTITY + 1))
            )),
            sentence("metal_ore")
        );
        // The lane's four rows.
        for (good, intake, per, cap) in [
            ("metal_ore", 240, 40, 240),
            ("carbon_ore", 120, 20, 120),
            ("strategic_ore", 12, 2, 12),
            ("ice_ore", 240, 40, 240),
        ] {
            assert_eq!(
                with(sm5_bid(good, intake, Some((per, cap)))),
                "accepted",
                "{good}"
            );
        }
        // The bounds themselves are admitted.
        assert_eq!(with(sm5_bid("metal_ore", 1, Some((1, 1)))), "accepted");
        // The wire: `refresh` is skipped when absent and strict when present.
        let bare = serde_json::to_value(sm5_bid("metals", 1, None)).unwrap_or_default();
        assert!(bare.get("refresh").is_none());
        assert!(
            serde_json::from_value::<RefreshDef>(
                serde_json::json!({"per_ic_day": 1, "cap": 1, "x": 1})
            )
            .is_err()
        );
    }

    /// **Doc 20 cut SM.5 (§7.3; lead ruling 3)**: a raise appends its own tranches to
    /// the base's, every tranche at most 800,000.
    #[test]
    fn sm5_the_capital_schedule_appends_each_raise() {
        assert_eq!(capital_schedule(50_000, &[250_000]), vec![50_000, 250_000]);
        let mut want = vec![800_000; 5];
        want.extend([800_000, 100_000]);
        assert_eq!(capital_schedule(4_000_000, &[900_000]), want);
        assert_eq!(capital_schedule(50_000, &[]), vec![50_000]);
        assert_eq!(
            capital_schedule(50_000, &[250_000, 1]),
            vec![50_000, 250_000, 1]
        );
        let mut occator = org("occator", 50_000);
        occator.raises = vec![250_000];
        assert_eq!(occator.capital_total(), Some(300_000));
        // The bounds.
        let sentence = format!(
            "org occator: raises: at most {MAX_CAPITAL_RAISES}, each 1..={MAX_PORT_CAPITAL_MAJOR}, \
             and the capital with every raise at most {MAX_PORT_CAPITAL_MAJOR}"
        );
        let with = |raises: Vec<u64>| {
            let mut def = port();
            let mut occator = org("occator", 50_000);
            occator.raises = raises;
            def.orgs.push(occator);
            refused(&def)
        };
        assert_eq!(with(vec![250_000]), "accepted");
        assert_eq!(with(vec![0]), sentence);
        assert_eq!(with(vec![MAX_PORT_CAPITAL_MAJOR + 1]), sentence);
        assert_eq!(
            with(vec![MAX_PORT_CAPITAL_MAJOR]),
            sentence,
            "the total passes the bound"
        );
        assert_eq!(with(vec![1; MAX_CAPITAL_RAISES]), "accepted");
        assert_eq!(with(vec![1; MAX_CAPITAL_RAISES + 1]), sentence);
        // The wire: `raises` is skipped when empty.
        let bare = serde_json::to_value(org("occator", 50_000)).unwrap_or_default();
        assert!(bare.get("raises").is_none());
    }

    /// **Doc 20 cut SM.5 (§9.1)**: the licence block's bounds, and a site authors none.
    #[test]
    fn sm5_a_licence_block_is_bounded() {
        let licence = LicenceDef {
            org: cid("trust"),
            fee_major: 2_500,
            term_ic_days: 168,
            grace_ic_days: 3,
            claim_stake_major: 2_000,
            claim_term_major: 2_000,
        };
        let with = |edit: &dyn Fn(&mut LicenceDef)| {
            let mut def = port();
            let mut block = licence.clone();
            edit(&mut block);
            def.licence = Some(Box::new(block));
            refused(&def)
        };
        assert_eq!(with(&|_| {}), "accepted");
        assert_eq!(
            with(&|l| l.org = cid("nobody")),
            "licence: org nobody is not one of this port's orgs"
        );
        assert_eq!(
            with(&|l| l.fee_major = 0),
            format!("licence: fee_major must be 1..={MAX_LICENCE_FEE_MAJOR}, not 0")
        );
        assert!(
            with(&|l| l.fee_major = MAX_LICENCE_FEE_MAJOR + 1).starts_with("licence: fee_major")
        );
        assert_eq!(
            with(&|l| l.term_ic_days = 0),
            format!("licence: term_ic_days must be 1..={MAX_LICENCE_TERM_IC_DAYS}, not 0")
        );
        assert!(
            with(&|l| l.term_ic_days = MAX_LICENCE_TERM_IC_DAYS + 1)
                .starts_with("licence: term_ic_days")
        );
        assert_eq!(
            with(&|l| l.grace_ic_days = 169),
            "licence: grace_ic_days must be at most the term (168), not 169"
        );
        assert_eq!(with(&|l| l.grace_ic_days = 168), "accepted");
        let mut site = site_port();
        site.licence = Some(Box::new(licence.clone()));
        assert_eq!(
            refused(&site),
            "site: a site authors no orgs, rooms, programme, vendors, premium or emergency, and \
             no vehicles or licence block; its pad is its gallery"
        );
        assert!(
            serde_json::from_value::<LicenceDef>(serde_json::json!({
                "org": "x", "fee_major": 1, "term_ic_days": 1, "grace_ic_days": 0, "extra": 1
            }))
            .is_err()
        );
    }

    /// A pack entry from its JSON.
    fn sm5_entry(value: serde_json::Value) -> PackEntry {
        serde_json::from_value(value).unwrap_or_else(|error| panic!("an entry: {error}"))
    }

    /// Two ports, `ceres_port` (the programme's, at the `here` location) and
    /// `stickney_port` (a vendor asking `metals`, at `there`), routed to each other.
    fn sm5_two_ports(edit: &dyn Fn(&mut ProgrammeDef)) -> BTreeMap<CanonicalId, PackEntry> {
        let mut ceres = port();
        if let Some(programme) = ceres.programme.as_mut() {
            edit(programme);
        }
        let mut stickney = port();
        stickney.programme = None;
        stickney.venue = cid("stickney");
        stickney.location = cid("there");
        stickney.orgs = vec![org("kasei", 1_000)];
        stickney.routes = BTreeMap::from([(cid("ceres"), vec![cid("back")])]);
        stickney.vendors.push(sm5_vendor(
            "kasei",
            "npc.kasei_clerk",
            &["metals"],
            Vec::new(),
        ));
        let place = |label: &str| {
            sm5_entry(serde_json::json!({
                "kind": "location", "def": { "label": label, "role": "port" }
            }))
        };
        let route = |from: &str, to: &str| {
            sm5_entry(serde_json::json!({
                "kind": "route",
                "def": { "from": from, "to": to, "legs": [ { "to": to, "base_s": 1 } ],
                         "drive_class": "coast_hauler", "propellant": 0 }
            }))
        };
        BTreeMap::from([
            (cid("here"), place("Here")),
            (cid("there"), place("There")),
            (cid("out"), route("here", "there")),
            (cid("back"), route("there", "here")),
            (
                cid("ceres_port"),
                sm5_entry(serde_json::json!({ "kind": "port", "def": ceres })),
            ),
            (
                cid("stickney_port"),
                sm5_entry(serde_json::json!({ "kind": "port", "def": stickney })),
            ),
        ])
    }

    /// **Doc 20 cut SM.5 (§7.4, Q-20-5)**: a fixed reference is only for a good no port
    /// of the lane asks; never also a reference listing; never 0; always a trade good.
    #[test]
    fn sm5_a_fixed_reference_is_only_for_a_good_no_port_asks() {
        let check = |edit: &dyn Fn(&mut ProgrammeDef)| {
            check_ports(&sm5_two_ports(edit)).err().map_or_else(
                || "accepted".to_owned(),
                |(entry, detail)| format!("{entry}: {detail}"),
            )
        };
        assert_eq!(check(&|_| {}), "accepted");
        assert_eq!(
            check(&|p| {
                p.reference_minor.insert(cid("metal_ore"), 3_500);
            }),
            "accepted"
        );
        assert_eq!(
            check(&|p| {
                p.reference_minor.insert(cid("metals"), 10_000);
            }),
            "ceres_port: programme coop.first_captain: reference_minor metals: a fixed \
             reference is only for a good no port of this lane asks, and port stickney_port asks it"
        );
        assert_eq!(
            check(&|p| {
                p.reference
                    .insert(cid("metal_ore"), "ceres_port/coop".to_owned());
                p.reference_minor.insert(cid("metal_ore"), 3_500);
            }),
            "ceres_port: programme coop.first_captain: metal_ore has both a reference listing \
             and a fixed reference"
        );
        assert_eq!(
            check(&|p| {
                p.reference_minor.insert(cid("metal_ore"), 0);
            }),
            "ceres_port: programme coop.first_captain: reference_minor metal_ore must be at least 1"
        );
        assert_eq!(
            check(&|p| {
                p.reference_minor.insert(cid("provisions"), 100);
            }),
            "ceres_port: programme coop.first_captain: reference_minor provisions is not a trade \
             good of the goods table"
        );
    }

    /// A mill row of `ore` into `into`, `ore_t : product_t`, `batch_t` a batch and
    /// `per_day_t` an IC day.
    fn sm12a_mill(
        ore: &str,
        into: &str,
        ratio: (u32, u32),
        batch_t: u64,
        per_day_t: u64,
    ) -> MillDef {
        MillDef {
            ore: cid(ore),
            into: cid(into),
            ratio: MillRatio {
                ore: ratio.0,
                product: ratio.1,
            },
            batch_t,
            per_ic_day_t: per_day_t,
        }
    }

    /// The Exchange's shape: asks `metals` and `volatiles`, bids `metal_ore` and
    /// `ice_ore`, mills the lane's metal-ore row.
    fn sm12a_vendor(mills: Vec<MillDef>) -> VendorDef {
        let mut vendor = sm5_vendor(
            "coop",
            "npc.someone",
            &["metals", "volatiles"],
            vec![
                sm5_bid("metal_ore", 240, None),
                sm5_bid("ice_ore", 240, None),
            ],
        );
        vendor.mill = mills;
        vendor
    }

    /// **Doc 20 cut SM.12a (§7.3 "The mill")**: each mill rule refuses at load with
    /// its sentence, whole; the lane's two rows are accepted; the family rule is
    /// `check_ports`'.
    #[test]
    fn sm12a_each_mill_rule_refuses_with_its_sentence() {
        let with = |mills: Vec<MillDef>| {
            let mut def = port();
            def.vendors.push(sm12a_vendor(mills));
            refused(&def)
        };
        let lane = || sm12a_mill("metal_ore", "metals", (5, 2), 20, 40);
        assert_eq!(with(vec![lane()]), "accepted");
        assert_eq!(
            with(vec![
                lane(),
                sm12a_mill("ice_ore", "volatiles", (5, 2), 20, 40)
            ]),
            "accepted"
        );
        assert_eq!(with(Vec::new()), "accepted");
        // At most four rows.
        assert_eq!(
            with(vec![lane(); MAX_VENDOR_MILLS + 1]),
            "vendor coop: at most 4 mill rows"
        );
        // An ore named twice; a product named twice.
        assert_eq!(
            with(vec![
                lane(),
                sm12a_mill("metal_ore", "volatiles", (5, 2), 20, 40)
            ]),
            "vendor coop: mill metal_ore is authored twice"
        );
        assert_eq!(
            with(vec![
                lane(),
                sm12a_mill("ice_ore", "metals", (5, 2), 20, 40)
            ]),
            "vendor coop: mill into metals is authored twice"
        );
        // The ore a bid; the product an ask.
        assert_eq!(
            with(vec![sm12a_mill("carbon_ore", "metals", (5, 2), 20, 40)]),
            "vendor coop: mill carbon_ore: the ore must be a good this vendor bids"
        );
        assert_eq!(
            with(vec![sm12a_mill("metal_ore", "propellant", (5, 2), 20, 40)]),
            "vendor coop: mill metal_ore: propellant must be a good this vendor asks"
        );
        // The ratio.
        let ratio = "vendor coop: mill metal_ore: the ratio is 1..=100 tonnes of ore to 1..=that \
                     many of product";
        for bad in [(5, 0), (2, 5), (101, 2), (0, 0)] {
            assert_eq!(
                with(vec![sm12a_mill("metal_ore", "metals", bad, 20, 40)]),
                ratio,
                "{bad:?}"
            );
        }
        assert_eq!(
            with(vec![sm12a_mill(
                "metal_ore",
                "metals",
                (100, 100),
                100,
                100
            )]),
            "accepted"
        );
        // The batch and the rate.
        let batch = "vendor coop: mill metal_ore: batch_t is a whole number of ratios and at most \
                     per_ic_day_t, which is at most 100000";
        for (batch_t, per_day_t) in [
            (0, 40),
            (21, 40),
            (45, 40),
            (20, mush_market::MAX_QUANTITY + 1),
        ] {
            assert_eq!(
                with(vec![sm12a_mill(
                    "metal_ore",
                    "metals",
                    (5, 2),
                    batch_t,
                    per_day_t
                )]),
                batch,
                "{batch_t} / {per_day_t}"
            );
        }
        assert_eq!(
            with(vec![sm12a_mill(
                "metal_ore",
                "metals",
                (5, 2),
                40,
                mush_market::MAX_QUANTITY
            )]),
            "accepted"
        );

        // The family rule, through `check_ports` on the two-port map: Stickney's
        // vendor mills metal ore into `volatiles`, another family's good.
        let family = |mill: MillDef| {
            let mut entries = sm5_two_ports(&|_| {});
            if let Some(PackEntry::Port(stickney)) = entries.get_mut(&cid("stickney_port"))
                && let Some(vendor) = stickney.vendors.first_mut()
            {
                let mut milling = sm12a_vendor(vec![mill]);
                milling.org = vendor.org.clone();
                milling.agent = vendor.agent.clone();
                *vendor = milling;
            }
            check_ports(&entries).err().map_or_else(
                || "accepted".to_owned(),
                |(entry, detail)| format!("{entry}: {detail}"),
            )
        };
        assert_eq!(family(lane()), "accepted");
        assert_eq!(
            family(sm12a_mill("metal_ore", "volatiles", (5, 2), 20, 40)),
            "stickney_port: vendor kasei: mill metal_ore: metal_ore and volatiles are not one \
             family's goods"
        );
        assert_eq!(
            family(sm12a_mill("ice_ore", "metals", (5, 2), 20, 40)),
            "stickney_port: vendor kasei: mill ice_ore: ice_ore and metals are not one family's \
             goods"
        );
    }

    /// **Doc 20 cut SM.12a**: a vendor with no mill rows re-encodes with no `mill`
    /// key, byte-identically; a mill row is strict at each level.
    #[test]
    fn sm12a_a_vendor_without_a_mill_re_encodes_byte_identically() {
        let bare = sm5_vendor(
            "coop",
            "npc.someone",
            &["metals"],
            vec![sm5_bid("metal_ore", 240, Some((40, 240)))],
        );
        let json = serde_json::to_value(&bare).unwrap_or_default();
        assert!(json.get("mill").is_none(), "{json}");
        let text = serde_json::to_string(&bare).unwrap_or_default();
        let back: VendorDef = serde_json::from_str(&text).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(back, bare);
        assert_eq!(serde_json::to_string(&back).unwrap_or_default(), text);
        // With a mill row, the key is present and round-trips.
        let milling = sm12a_vendor(vec![sm12a_mill("metal_ore", "metals", (5, 2), 20, 40)]);
        let json = serde_json::to_value(&milling).unwrap_or_default();
        assert_eq!(
            json["mill"],
            serde_json::json!([{ "ore": "metal_ore", "into": "metals",
                "ratio": { "ore": 5, "product": 2 }, "batch_t": 20, "per_ic_day_t": 40 }])
        );
        assert_eq!(
            serde_json::from_value::<VendorDef>(json).ok(),
            Some(milling)
        );
        // Strict at each level.
        assert!(
            serde_json::from_value::<MillDef>(serde_json::json!({
                "ore": "metal_ore", "into": "metals", "ratio": { "ore": 5, "product": 2 },
                "batch_t": 20, "per_ic_day_t": 40, "x": 1
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<MillRatio>(serde_json::json!({
                "ore": 5, "product": 2, "x": 1
            }))
            .is_err()
        );
    }

    /// The lane document's game entries (the atlas's own kinds, `region`, are not tacsim's).
    fn sm12b_entries(doc: &serde_json::Value) -> BTreeMap<CanonicalId, PackEntry> {
        doc["entries"]
            .as_object()
            .unwrap_or_else(|| panic!("the lane has entries"))
            .iter()
            .filter(|(_, entry)| entry["kind"] != "region")
            .map(|(id, entry)| {
                (
                    cid(id),
                    serde_json::from_value(entry.clone())
                        .unwrap_or_else(|error| panic!("entry {id} decodes: {error}")),
                )
            })
            .collect()
    }

    /// The shipped lane's entries after `edit`, refused as the loader formats a port refusal
    /// (`port entry {id}: {detail}`): every port's own bounds first (`PortDef::check`, entry
    /// order), then the pack-wide rules (`check_ports`) — `crate::pack`'s order.
    fn sm12b_lane(edit: &dyn Fn(&mut serde_json::Value)) -> String {
        let mut doc: serde_json::Value =
            serde_json::from_str(crate::definitions::content::LANE_JSON)
                .unwrap_or_else(|error| panic!("the lane is JSON: {error}"));
        edit(&mut doc);
        let entries = sm12b_entries(&doc);
        for (id, entry) in &entries {
            if let PackEntry::Port(def) = entry
                && let Err(detail) = def.check()
            {
                return format!("port entry {id}: {detail}");
            }
        }
        match check_ports(&entries) {
            Ok(()) => "accepted".to_owned(),
            Err((id, detail)) => format!("port entry {id}: {detail}"),
        }
    }

    /// An edit of the shipped yard's `key` to `value`.
    fn sm12b_yard(key: &'static str, value: serde_json::Value) -> impl Fn(&mut serde_json::Value) {
        move |doc: &mut serde_json::Value| {
            doc["entries"]["occator_port"]["def"]["site"]["yard"][key] = value.clone();
        }
    }

    /// **Doc 20 cut SM.12b (§12.3)**: each yard rule refuses at load with its sentence, whole, as
    /// the loader formats it; the shipped yard is accepted.
    #[test]
    #[allow(clippy::too_many_lines)] // one case a rule
    fn sm12b_each_yard_rule_refuses_with_its_sentence() {
        use serde_json::json;
        let at = |detail: &str| format!("port entry occator_port: site: yard: {detail}");
        assert_eq!(sm12b_lane(&|_| {}), "accepted");
        // SiteDef::check's rules.
        assert_eq!(
            sm12b_lane(&sm12b_yard("name", json!(" "))),
            at("the name is not a label")
        );
        let prefix = at(&format!(
            "a registry prefix is 1 to {MAX_REGISTRY_PREFIX_BYTES} bytes of A-Z, 0-9 and -"
        ));
        for (key, bad) in [
            ("registry_prefix", "owp-yd-"),
            ("registry_prefix", ""),
            ("registry_prefix", "OWP-YD-0123456789"),
            ("rig_prefix", "OWP YT"),
        ] {
            assert_eq!(
                sm12b_lane(&sm12b_yard(key, json!(bad))),
                prefix,
                "{key} {bad}"
            );
        }
        assert_eq!(
            sm12b_lane(&sm12b_yard("rig_prefix", json!("OWP-YD-"))),
            at("the hull and its rigs need two prefixes")
        );
        for rigs in [0, MAX_BAY_ITEMS_FOR_TEST + 1] {
            assert_eq!(
                sm12b_lane(&sm12b_yard("rigs", json!(rigs))),
                at(&format!("rigs is 1 to {MAX_BAY_ITEMS_FOR_TEST}")),
                "{rigs}"
            );
        }
        for rearm in [0, MAX_YARD_REARM_IC_SECS + 1] {
            assert_eq!(
                sm12b_lane(&sm12b_yard("rearm_ic_secs", json!(rearm))),
                at("rearm_ic_secs is 1 to 604800"),
                "{rearm}"
            );
        }
        assert_eq!(
            sm12b_lane(&sm12b_yard("rearm_ic_secs", json!(MAX_YARD_REARM_IC_SECS))),
            "accepted"
        );
        for blocks in [
            json!([]),
            json!(["F", "F"]),
            json!(["F", "G", "J", "K", "F"]),
            json!(["Q"]),
        ] {
            assert_eq!(
                sm12b_lane(&sm12b_yard("blocks", blocks.clone())),
                at("blocks names 1 to 4 distinct letters A to P"),
                "{blocks}"
            );
        }
        assert_eq!(
            sm12b_lane(&sm12b_yard("blocks", json!(["F", "G", "J", "K"]))),
            "accepted"
        );
        // Open (papered) ground is not the house's.
        assert_eq!(
            sm12b_lane(&sm12b_yard("blocks", json!(["F", "A"]))),
            at("block A is not held by occator")
        );
        // A block held by another org (the Pallas Syndicate) is not held by the yard's org.
        assert_eq!(
            sm12b_lane(&|doc| {
                doc["entries"]["occator_port"]["def"]["site"]["blocks"]["B"] =
                    json!({ "tenure": "held", "holder": "pallas_syndicate" });
                doc["entries"]["occator_port"]["def"]["site"]["yard"]["blocks"] = json!(["B"]);
            }),
            at("block B is not held by occator")
        );
        // check_ports' rules.
        assert_eq!(
            sm12b_lane(&sm12b_yard("pool", json!("occator.other"))),
            at("pool occator.other is not the pack's rig pool of occator")
        );
        assert_eq!(
            sm12b_lane(&|doc| {
                doc["entries"]["ceres_port"]["def"]["vehicles"]["org"] = json!("coop");
            }),
            at("pool occator.rig_lease is not the pack's rig pool of occator")
        );
        assert_eq!(
            sm12b_lane(&sm12b_yard("ore", json!("volatiles"))),
            at("volatiles is not an ore good")
        );
        assert_eq!(
            sm12b_lane(&sm12b_yard("ore", json!("metal_ore"))),
            at("occator mills no metal_ore, so the yard has nowhere to deliver")
        );
        assert_eq!(
            sm12b_lane(&|doc| {
                if let Some(vendors) = doc["entries"]["ceres_port"]["def"]["vendors"].as_array_mut()
                {
                    for vendor in vendors.iter_mut() {
                        if vendor["org"] == "occator"
                            && let Some(vendor) = vendor.as_object_mut()
                        {
                            vendor.remove("mill");
                        }
                    }
                }
            }),
            at("occator mills no ice_ore, so the yard has nowhere to deliver")
        );
        for (key, taken) in [
            ("registry_prefix", "OWP-TV-"),
            ("rig_prefix", "FPC-D-04"),
            ("rig_prefix", "CF-H-"),
        ] {
            assert_eq!(
                sm12b_lane(&sm12b_yard(key, json!(taken))),
                at(&format!("prefix {taken} is another pool's")),
                "{key} {taken}"
            );
        }
        // A second yard for the same house, at Juling Shadow (entry order: it is the first).
        let second = |hull: &'static str, rigs: &'static str| {
            move |doc: &mut serde_json::Value| {
                let site = &mut doc["entries"]["juling_port"]["def"]["site"];
                site["blocks"] = json!({ "A": { "tenure": "held", "holder": "occator" } });
                let mut yard = doc["entries"]["occator_port"]["def"]["site"]["yard"].clone();
                yard["registry_prefix"] = json!(hull);
                yard["rig_prefix"] = json!(rigs);
                yard["blocks"] = json!(["A"]);
                doc["entries"]["juling_port"]["def"]["site"]["yard"] = yard;
            }
        };
        assert_eq!(
            sm12b_lane(&second("OWP-YE-", "OWP-YU-")),
            at("occator already keeps a yard at juling_port")
        );
        assert_eq!(
            sm12b_lane(&second("OWP-YD-", "OWP-YU-")),
            at("prefix OWP-YD- is another pool's")
        );
        // The org rule, unreachable through a whole pack (a held block's holder is an org of the
        // pack by the SM.2 rule, checked first), pinned through `check_yards` itself.
        let mut doc: serde_json::Value =
            serde_json::from_str(crate::definitions::content::LANE_JSON)
                .unwrap_or_else(|error| panic!("the lane is JSON: {error}"));
        doc["entries"]["occator_port"]["def"]["site"]["yard"]["org"] = json!("nobody");
        let entries = sm12b_entries(&doc);
        let ports: Vec<(&CanonicalId, &PortDef)> = entries
            .iter()
            .filter_map(|(id, entry)| match entry {
                PackEntry::Port(def) => Some((id, def)),
                _ => None,
            })
            .collect();
        let orgs: BTreeMap<&CanonicalId, &CanonicalId> = ports
            .iter()
            .flat_map(|(id, def)| def.orgs.iter().map(move |org| (&org.id, *id)))
            .collect();
        assert_eq!(
            check_yards(&ports, &orgs)
                .err()
                .map(|(id, detail)| format!("port entry {id}: {detail}")),
            Some(at("nobody is not an org of this pack"))
        );
    }

    /// `crate::definitions::logistics::MAX_BAY_ITEMS`, as the rigs sentence spells it.
    const MAX_BAY_ITEMS_FOR_TEST: u32 = crate::definitions::logistics::MAX_BAY_ITEMS;

    /// **Doc 20 cut SM.12b**: a site with no yard re-encodes with no `yard` key, byte-identically
    /// (a v14 site); a yard block round-trips and is strict.
    #[test]
    fn sm12b_a_site_without_a_yard_re_encodes_byte_identically() {
        let def = site_port();
        let site = def
            .site
            .as_deref()
            .cloned()
            .unwrap_or_else(|| panic!("a site"));
        let json = serde_json::to_value(&site).unwrap_or_default();
        assert!(json.get("yard").is_none(), "{json}");
        let text = serde_json::to_string(&site).unwrap_or_default();
        let back: SiteDef = serde_json::from_str(&text).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(back, site);
        assert_eq!(serde_json::to_string(&back).unwrap_or_default(), text);
        // The shipped Occator Rim decodes its yard, and re-encodes it.
        let doc: serde_json::Value = serde_json::from_str(crate::definitions::content::LANE_JSON)
            .unwrap_or_else(|error| panic!("the lane is JSON: {error}"));
        let wire = doc["entries"]["occator_port"]["def"]["site"]["yard"].clone();
        let yard: YardDef = serde_json::from_value(wire.clone())
            .unwrap_or_else(|error| panic!("the yard decodes: {error}"));
        assert_eq!(yard.blocks, vec!['F', 'G']);
        assert_eq!((yard.rigs, yard.rearm_ic_secs), (2, 3_600));
        assert_eq!(serde_json::to_value(&yard).unwrap_or_default(), wire);
        let mut strict = wire;
        strict["x"] = serde_json::json!(1);
        assert!(serde_json::from_value::<YardDef>(strict).is_err());
    }
}
