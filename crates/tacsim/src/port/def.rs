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
/// tranche fits under it, and below anything a single drip should ever be.
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
    /// Role word ([`PORT_ROOM_ROLES`]) → `"<area>.<entry>"`.
    pub rooms: BTreeMap<CanonicalId, String>,
    /// A `SystemPrincipalId` (`tacsim.treasury`): the faucet the capital is
    /// minted from.
    pub treasury: String,
    /// The institutions, `1..=`[`MAX_PORT_ORGS`]; authored order is creation
    /// order.
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
        // The institutions.
        if self.orgs.is_empty() || self.orgs.len() > MAX_PORT_ORGS {
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
                if row.qty == 0 || row.issue == 0 || row.grade > 100 {
                    return Err(format!(
                        "vendor {}: stock {}: qty and issue must be at least 1 and grade 0..=100",
                        vendor.org, row.good
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
}
