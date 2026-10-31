//! **The mills stripped away** (doc 20 cut SM.12a): the v13 shape from the shipped v14, for
//! the suites that rebuild an older lane (the manifest is the caller's). `strip_claims` calls
//! it first, so every strip chain rebuilds its older lane unchanged. **Doc 20 cut SM.12b**: it
//! calls `super::drones::strip_yard` first in turn — a yard names a mill row, so the mills are
//! never stripped from under it — so the v13 shape it rebuilds carries no yard either.

use serde_json::Value;

/// Remove exactly what cut SM.12a added to the lane — the `mill` key of every vendor of
/// `ceres_port` (the Exchange's metal-ore mill and Occator's ice-ore mill) — the v13 shape
/// (idempotent; the manifest is the caller's). Doc 20 cut SM.12b: Occator's yard goes first
/// (`super::drones::strip_yard`).
pub fn strip_mill(doc: &mut Value) {
    // Doc 32 cut CF.6c: v16's escort design first, so every strip chain rebuilds its older lane.
    super::drones::strip_designs(doc);
    super::drones::strip_yard(doc);
    let Some(vendors) = doc["entries"]
        .as_object_mut()
        .and_then(|entries| entries.get_mut("ceres_port"))
        .and_then(|ceres| ceres["def"].get_mut("vendors"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for vendor in vendors.iter_mut() {
        if let Some(vendor) = vendor.as_object_mut() {
            vendor.remove("mill");
        }
    }
}
