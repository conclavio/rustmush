//! **The port map** (doc 18 cut IC3; doc 18 §4.7 "Idempotency"; lead ruling L3):
//! the `PortApplier`'s record of everything it made for one port entry — the
//! domain, the treasury account, per institution its org, root scope,
//! account and every capital tranche obligation, (cut IC4) per programme
//! hull the Thing, its rooms and its exits, and (cut IC8) the stock clerk,
//! per vendor its storefront, vault, market vendor, signatory, the clerk's
//! grant, its stock plan rows and its standing bids, and the board rooms.
//!
//! One whole typed document per port entry, `tacsim:port:{pack}/{entry}`,
//! **version-agnostic**: lane v2 writes it and lane v3 (cut IC8) reuses it. It is
//! persisted **immediately after every act** inside the apply's one batch, the
//! `ArenaRoster` pattern (`crate::match_::centre::record_roster`), so an apply
//! that stops part way leaves a map naming every act it made. It is the **sole
//! dedup**: the economy does not dedup obligations, so a re-apply that could not
//! read the map would mint the capital twice.
//!
//! Its key bars re-creation when its body is unreadable (the `tacsim:seed` key
//! rule, `crate::store`'s table): the entry's next apply refuses before any write,
//! naming the key, and `@tacsim/port status` reports the map unreadable. Repair is
//! a staff act on the document.

use std::collections::BTreeMap;

use mush_core::CanonicalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The prefix of every port map document.
pub const PORT_MAP_PREFIX: &str = "tacsim:port:";

/// The map's schema version.
pub const PORT_MAP_SCHEMA: u16 = 1;

/// **`tacsim:port:{pack}/{entry}`** — the one key of one port entry's map. A
/// canonical id holds no `/`, so the tail splits on its single `/`.
#[must_use]
pub fn port_map_key(pack: &CanonicalId, entry: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("{PORT_MAP_PREFIX}{pack}/{entry}"))
}

/// Split a port map key's tail (`{pack}/{entry}`) into its two ids. `None` when
/// either half is unusable, which is what makes an unreadable key a loud skip.
#[must_use]
pub(crate) fn split_key_tail(tail: &str) -> Option<(CanonicalId, CanonicalId)> {
    let (pack, entry) = tail.split_once('/')?;
    if entry.contains('/') {
        return None;
    }
    Some((
        CanonicalId::parse_canonical(pack).ok()?,
        CanonicalId::parse_canonical(entry).ok()?,
    ))
}

/// **One port entry's map** (doc 18 §4.7; L3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortMap {
    /// [`PORT_MAP_SCHEMA`].
    pub schema: u16,
    /// The pack the entry belongs to (the lane).
    pub pack: CanonicalId,
    /// The port entry.
    pub entry: CanonicalId,
    /// The passive domain (part a), `DomainId.0`.
    pub domain: Option<u64>,
    /// The domain's host: the location room, `ObjId.0`.
    pub host: Option<u32>,
    /// The domain's presentation: the gallery, `ObjId.0`.
    pub presentation: Option<u32>,
    /// The treasury account (part b), `AccountId.0`.
    pub treasury: Option<u64>,
    /// Per authored org id, what part c made for it.
    pub orgs: BTreeMap<CanonicalId, PortOrgRow>,
    /// The programme's hulls (part f, cut IC4), in pool order. Skipped when
    /// empty, so a map with no hulls encodes byte-identically to cut IC3's and
    /// [`PORT_MAP_SCHEMA`] stays 1 (lead ruling L9).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hulls: Vec<HullRow>,
    /// **The port's stock clerk** (part e, cut IC8; lead ruling L2), once made.
    /// Skipped when absent, so a map without vendors encodes as before and
    /// [`PORT_MAP_SCHEMA`] stays 1 (the IC4 L9 precedent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clerk: Option<ClerkRow>,
    /// Per authored vendor org id, what part e made for it (cut IC8). Doc 18's
    /// `content.stock/<pack>/<entry>/<org>/<good>/<issue>` plan rows are this
    /// map's `vendors[org].stock[i]` (IC3 L3's one-document rule).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vendors: BTreeMap<CanonicalId, VendorRow>,
    /// Room role → the room's `ObjId.0`, for every role an ask or bid of the
    /// port names in its `boards` (cut IC8, lead ruling L15): the board hook
    /// finds its rooms here, never by a scan at look time.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub boards: BTreeMap<CanonicalId, u32>,
}

/// **The port's stock clerk** (doc 18 cut IC8; D-18-26): the Thing the
/// `tacsim.port` tick acts as when it mints, dissolves and lists the port's
/// stock. It never speaks and owns nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClerkRow {
    /// The authored agent id (`vendors[].agent`).
    pub agent: CanonicalId,
    /// The clerk Thing, `ObjId.0`.
    pub thing: u32,
}

/// **One vendor's row** (doc 18 §4.7 part e; cut IC8): the objects and records
/// part e made for it. Objects only; the owners keep the truth (the vault in
/// the economy, the vendor in the market, the grant in the org).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VendorRow {
    /// The storefront item Thing, `ObjId.0`.
    pub storefront: u32,
    /// The storefront vault, `VaultId.0`.
    pub vault: u64,
    /// The market vendor, `VendorId.0`, once registered.
    pub vendor: Option<u64>,
    /// The signatory npc, `ObjId.0`.
    pub signatory: u32,
    /// The clerk's `vault.withdraw` grant in this org's root scope,
    /// `GrantId.0`, once submitted.
    pub grant: Option<u64>,
    /// The stock plan rows, in authored order.
    pub stock: Vec<StockRow>,
    /// Good → the standing bid posted for it.
    pub bids: BTreeMap<CanonicalId, BidRow>,
}

/// **One stock plan row** (doc 18 §4.3 "Finite seeded stock"; cut IC8): the
/// authored row as recorded at first apply. A port update never re-issues or
/// re-sizes it (Q-18-19; lead ruling L6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockRow {
    /// The good.
    pub good: CanonicalId,
    /// The issue number.
    pub issue: u32,
    /// The quantity to mint.
    pub qty: u64,
    /// The grade to mint at.
    pub grade: u8,
    /// The `StockIssueId` key text (64 lowercase hexadecimal digits).
    pub id: String,
    /// `StockIssueId::lot_id().0`.
    pub lot: u64,
}

/// **One standing bid** (doc 18 §4.3 "Bids that take real lots"; cut IC8):
/// the Wanted listing posted for it and the authored figures it was posted
/// with, which an update compares the pack against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BidRow {
    /// The Wanted listing, `ListingId.0`.
    pub listing: u64,
    /// The authored price, major units.
    pub price_major: u64,
    /// The authored intake.
    pub intake: u64,
    /// The posted listing's real-time (`RealUtc`) deadline, unix seconds: what tells
    /// a bid that is gone because it lapsed (re-posted by the next apply when its
    /// intake is unchanged) from one gone before it (exhausted — "not buying").
    pub expires: i64,
}

impl PortMap {
    /// An empty map for `(pack, entry)`: nothing made yet.
    #[must_use]
    pub fn new(pack: CanonicalId, entry: CanonicalId) -> PortMap {
        PortMap {
            schema: PORT_MAP_SCHEMA,
            pack,
            entry,
            domain: None,
            host: None,
            presentation: None,
            treasury: None,
            orgs: BTreeMap::new(),
            hulls: Vec::new(),
            clerk: None,
            vendors: BTreeMap::new(),
            boards: BTreeMap::new(),
        }
    }

    /// This map's document key.
    #[must_use]
    pub fn key(&self) -> SmolStr {
        port_map_key(&self.pack, &self.entry)
    }
}

/// **One institution's row** (part c): the org and its root scope (created in
/// one `CreateOrg`), the capital it was created to receive, its scoped account
/// once opened, and each capital tranche once its obligation exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOrgRow {
    /// `OrgId.0`.
    pub org: u64,
    /// `OrgScopeId.0`: the root scope the account hangs off.
    pub root_scope: u64,
    /// **The authored `capital_major` the row was created for**, recorded with
    /// the org (L4, D-18-27): the one number a later apply compares the pack
    /// against. Equal, the missing tranches of `tranches(capital_major)` are
    /// completed (an apply that stopped part way); different, the entry is
    /// refused as drift — never topped up, even when the recorded tranches are
    /// a prefix of the new list (4,000,000 → 4,800,000 is five rows of 800,000
    /// either way).
    pub capital_major: u64,
    /// `AccountId.0`: the org's scoped `tacsim.credit` account.
    pub account: Option<u64>,
    /// The tranche obligations made so far, in tranche order.
    pub capital: Vec<TrancheRow>,
}

/// **One capital tranche** (D-18-14): its index in `tranches(capital_major)`,
/// the one-shot obligation that mints it, and its amount in minor units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrancheRow {
    /// The tranche's index, from 0.
    pub n: u32,
    /// `ObligationId.0`.
    pub obligation: u64,
    /// The amount, minor units of `tacsim.credit`.
    pub amount_minor: i64,
}

/// **One programme hull** (doc 18 §4.7 part f; cut IC4): the objects the
/// applier made for it, in the order made. Title, unit, spaces, members,
/// compartment and placement are their owners' records and are read there,
/// never copied here (lead ruling L9: one writer per authority). The hull's
/// interior space and store are `rooms[interior.space]` and
/// `rooms[interior.store]` (doc 18 §4.7's `hulls{hull, space, store, rooms,
/// exits}`, derived rather than duplicated).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HullRow {
    /// The pool index, from 0 (`registry_mark(prefix, n)`).
    pub n: u32,
    /// The registry mark (`FPC-D-0401`), the Thing's alias; never re-issued.
    pub mark: String,
    /// The hull Thing, `ObjId.0`.
    pub hull: u32,
    /// Interior room id → `ObjId.0`, every room made so far (all seven when
    /// complete).
    pub rooms: BTreeMap<CanonicalId, u32>,
    /// The exits made so far, `ObjId.0`, in template order: pair i is `2i`
    /// (names[0]) and `2i+1` (names[1]).
    pub exits: Vec<u32>,
}

/// **What the store holds under one port entry's key** — the three answers the
/// applier and the report need to tell apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortMapState<'a> {
    /// No document under the key: nothing was made.
    Absent,
    /// A readable map.
    Present(&'a PortMap),
    /// A document under the key whose body did not read: **never re-created**.
    Unreadable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ic3_the_key_round_trips_through_its_tail() {
        let pack = CanonicalId::new("tacsim.lane.ceres_phobos").expect("an id");
        let entry = CanonicalId::new("ceres_port").expect("an id");
        let key = port_map_key(&pack, &entry);
        assert_eq!(key, "tacsim:port:tacsim.lane.ceres_phobos/ceres_port");
        let tail = key.strip_prefix(PORT_MAP_PREFIX).expect("the prefix");
        assert_eq!(split_key_tail(tail), Some((pack, entry)));
        assert_eq!(split_key_tail("a/b/c"), None);
        assert_eq!(split_key_tail("nokey"), None);
    }
}
