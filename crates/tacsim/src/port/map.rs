//! **The port map** (doc 18 cut IC3; doc 18 §4.7 "Idempotency"; lead ruling L3):
//! the `PortApplier`'s record of everything it made for one port entry — the
//! domain, the treasury account, and per institution its org, root scope,
//! account and every capital tranche obligation.
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
