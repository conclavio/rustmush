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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<SiteDef>,
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
    /// prospectivity `-3..=3`; worked share `0..=1000`.
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
    /// The buyout price of a hull, major units.
    pub buyout_major: u64,
    /// How long a lease may lie idle before it is released, real days.
    pub dormancy_real_days: u32,
    /// How many enrolments one account may hold.
    pub per_account: u32,
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
            {
                return Err(
                    "site: a site authors no orgs, rooms, programme, vendors, premium or \
                     emergency; its pad is its gallery"
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

    /// **One vendor's lines** (doc 18 cut IC8; §4.3 "Vendors and buyers"):
    /// stock, ask and bid goods each unique within the vendor; every ask drawn
    /// from a stock row of the same vendor; no good both asked and bid; every
    /// `boards` role a key of `rooms` other than `spine`, none repeated in one
    /// line.
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
        match needed {
            Some(needed) if needed <= org.capital_major => Ok(()),
            Some(needed) => Err(format!(
                "programme {pid}: floor_major + pool × stake_major is {needed}, more than org \
                 {}'s capital_major {}",
                org.id, org.capital_major
            )),
            None => Err(format!(
                "programme {pid}: floor_major + pool × stake_major overflows"
            )),
        }
    }
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
/// - (cut IC8) every stock, ask and bid good is a row of the goods table.
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
    for (id, def) in &ports {
        let fail = |detail: String| Err(((*id).clone(), detail));
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
        for (letter, row) in &site.blocks {
            if let Some(holder) = &row.holder
                && !orgs.contains_key(holder)
            {
                return fail(format!(
                    "site: block {letter}: holder {holder} is not an org of any port of this pack"
                ));
            }
        }
    }
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
        let references = def
            .programme
            .as_ref()
            .filter(|programme| !programme.reference.is_empty());
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
                buyout_major: 90_000,
                dormancy_real_days: 7,
                per_account: 1,
            }),
            vendors: Vec::new(),
            routes: BTreeMap::from([(cid("stickney"), vec![cid("out")])]),
            premium: None,
            emergency: None,
            site: None,
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
            site: Some(SiteDef {
                name: "Ahuna Flank".to_owned(),
                issuer: cid("freeport_registry"),
                zone: Zone::Core,
                tenure: SiteTenureDef {
                    default: Tenure::Open,
                },
                blocks: BTreeMap::new(),
            }),
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
        let shifts = def.site.as_ref().map(SiteDef::shifts).unwrap_or_default();
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
}
