//! **The ore bids, the fixed references, the licence block and Occator's raise stripped
//! away** (doc 20 cut SM.5): the v10 shape from the shipped v11, for the suites that
//! rebuild an older lane (the manifest is the caller's).

use serde_json::Value;

/// The four ore goods cut SM.5 bids for at Ceres.
pub const ORE_GOODS: [&str; 4] = ["metal_ore", "carbon_ore", "strategic_ore", "ice_ore"];

/// Remove exactly what cut SM.5 added to the lane — the four ore bids (and Occator's
/// `bids` key when it empties), the programme's `reference_minor`, `ceres_port`'s
/// `licence` block and Occator's `raises` — the v10 shape (idempotent; the manifest is
/// the caller's).
pub fn strip_ore(doc: &mut Value) {
    // Doc 20 cut SM.10: v12's additions first, so every strip chain rebuilds its older lane
    // unchanged.
    super::claims::strip_claims(doc);
    let Some(ceres) = doc["entries"]
        .as_object_mut()
        .and_then(|entries| entries.get_mut("ceres_port"))
    else {
        return;
    };
    let Some(def) = ceres["def"].as_object_mut() else {
        return;
    };
    def.remove("licence");
    if let Some(programme) = def.get_mut("programme").and_then(Value::as_object_mut) {
        programme.remove("reference_minor");
    }
    if let Some(orgs) = def.get_mut("orgs").and_then(Value::as_array_mut) {
        for org in orgs.iter_mut() {
            if org["id"] == "occator"
                && let Some(org) = org.as_object_mut()
            {
                org.remove("raises");
            }
        }
    }
    if let Some(vendors) = def.get_mut("vendors").and_then(Value::as_array_mut) {
        for vendor in vendors.iter_mut() {
            let Some(vendor) = vendor.as_object_mut() else {
                continue;
            };
            let emptied = match vendor.get_mut("bids").and_then(Value::as_array_mut) {
                Some(bids) => {
                    bids.retain(|bid| {
                        !bid["good"]
                            .as_str()
                            .is_some_and(|good| ORE_GOODS.contains(&good))
                    });
                    bids.is_empty()
                }
                None => false,
            };
            if emptied && vendor["org"] == "occator" {
                vendor.remove("bids");
            }
        }
    }
}
