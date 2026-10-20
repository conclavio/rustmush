//! **Occator's rigs and the refit row stripped away** (doc 20 cut SM.3a): the v9 shape from the
//! shipped v10, for the suites that rebuild an older lane (the manifest is the caller's).

use serde_json::Value;

/// Remove everything cut SM.3a added to the lane — `ceres_port`'s `vehicles` block and its
/// programme's `refits` row — the v9 shape (idempotent; the manifest is the caller's).
pub fn strip_vehicles(doc: &mut Value) {
    let Some(ceres) = doc["entries"]
        .as_object_mut()
        .and_then(|entries| entries.get_mut("ceres_port"))
    else {
        return;
    };
    if let Some(def) = ceres["def"].as_object_mut() {
        def.remove("vehicles");
    }
    if let Some(programme) = ceres["def"]["programme"].as_object_mut() {
        programme.remove("refits");
    }
}
