//! **Doc 20 cut SM.10's shared test pieces**: what cut SM.10 added to the lane stripped away (the
//! v11 shape from the shipped v12, so every older-lane strip chain rebuilds its lane unchanged),
//! the fixed gateway-account oracle (`plumbing.rs`'s `MapOracle`, lifted here), and the three
//! geology tables the claim suites' deposit-count rig installs (reviewer F10).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mush_core::{AccountId, AccountIdentity, ObjId};
use mush_testkit::TestWorld;
use serde_json::Value;

/// Remove exactly what cut SM.10 added to the lane — the Pallas Syndicate org, the licence
/// block's two claim fees, Occator Rim's `papers` and its blocks' `paper` — the v11 shape
/// (idempotent; the manifest is the caller's).
pub fn strip_claims(doc: &mut Value) {
    let Some(entries) = doc["entries"].as_object_mut() else {
        return;
    };
    // The shipped lane is now v13. Historical v11 fixtures must also remove
    // the later, additive Fleet programme without changing the old digest.
    if let Some(def) = entries
        .get_mut("luna_port")
        .and_then(|entry| entry["def"].as_object_mut())
    {
        def.remove("programme");
        if let Some(orgs) = def.get_mut("orgs").and_then(Value::as_array_mut) {
            orgs.retain(|org| org["id"] != "concord_fleet");
        }
    }
    if let Some(def) = entries
        .get_mut("ceres_port")
        .and_then(|entry| entry["def"].as_object_mut())
    {
        if let Some(orgs) = def.get_mut("orgs").and_then(Value::as_array_mut) {
            orgs.retain(|org| org["id"] != "pallas_syndicate");
        }
        if let Some(licence) = def.get_mut("licence").and_then(Value::as_object_mut) {
            licence.remove("claim_stake_major");
            licence.remove("claim_term_major");
        }
    }
    if let Some(site) = entries
        .get_mut("occator_port")
        .and_then(|entry| entry["def"]["site"].as_object_mut())
    {
        site.remove("papers");
        if let Some(blocks) = site.get_mut("blocks").and_then(Value::as_object_mut) {
            for row in blocks.values_mut() {
                if let Some(row) = row.as_object_mut() {
                    row.remove("paper");
                }
            }
        }
    }
}

/// **A fixed gateway-account oracle** (`plumbing.rs`'s, lifted): characters mapped to gateway
/// accounts by the test, so two characters can share one account (an alt).
#[derive(Default)]
pub struct MapOracle(pub Mutex<BTreeMap<ObjId, AccountId>>);

impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.lock().ok()?.get(&character).copied()
    }
}

/// Install `oracle` as the world's account oracle (boot-hydrated by the host from the store,
/// which a `TestWorld` has none of — the `match_lifecycle.rs` precedent).
pub fn install_oracle(tw: &mut TestWorld, oracle: &Arc<MapOracle>) {
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(oracle.clone());
}

/// Every family rich everywhere: four opened deposits a block (`deposits.rs`'s `ALL_RICH`).
pub const ALL_RICH: &str = r#"{
  "province": {
    "metals": { "rich": 1000 }, "volatiles": { "rich": 1000 },
    "carbon_feedstock": { "rich": 1000 }, "strategic_elements": { "rich": 1000 }
  },
  "field": {
    "barren": { "barren": 1000 }, "lean": { "lean": 1000 },
    "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
  },
  "pocket": {
    "lean": { "lean": 1000 }, "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
  }
}"#;

/// Metals rich everywhere, the other three families' province barren: one opened deposit a
/// block.
pub const METALS_ONLY: &str = r#"{
  "province": {
    "metals": { "rich": 1000 }, "volatiles": { "barren": 1000 },
    "carbon_feedstock": { "barren": 1000 }, "strategic_elements": { "barren": 1000 }
  },
  "field": {
    "barren": { "barren": 1000 }, "lean": { "lean": 1000 },
    "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
  },
  "pocket": {
    "lean": { "lean": 1000 }, "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
  }
}"#;

/// Every family's province barren, so every field row draws barren: no opened deposit at all.
pub const ALL_BARREN: &str = r#"{
  "province": {
    "metals": { "barren": 1000 }, "volatiles": { "barren": 1000 },
    "carbon_feedstock": { "barren": 1000 }, "strategic_elements": { "barren": 1000 }
  },
  "field": {
    "barren": { "barren": 1000 }, "lean": { "lean": 1000 },
    "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
  },
  "pocket": {
    "lean": { "lean": 1000 }, "fair": { "fair": 1000 }, "rich": { "rich": 1000 }
  }
}"#;
