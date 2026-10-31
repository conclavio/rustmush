//! **Occator's yard stripped away** (doc 20 cut SM.12b): the v14 shape from the shipped v15, for
//! the suites that rebuild an older lane (the manifest is the caller's). `strip_mill` calls it
//! first — a yard names a mill row, so the mills cannot be stripped from under it — so every
//! strip chain rebuilds its older lane unchanged.

use serde_json::Value;

/// Remove exactly what doc 32 cut CF.6c added to the lane — Occator's pool's further designs
/// (`ceres_port.vehicles.designs`) — the v15 shape (idempotent; the manifest is the caller's).
pub fn strip_designs(doc: &mut Value) {
    if let Some(vehicles) = doc["entries"]
        .as_object_mut()
        .and_then(|entries| entries.get_mut("ceres_port"))
        .and_then(|ceres| ceres["def"].get_mut("vehicles"))
        .and_then(Value::as_object_mut)
    {
        vehicles.remove("designs");
    }
}

/// Remove exactly what cut SM.12b added to the lane — `occator_port`'s `site.yard` (the Occator
/// Yard, its two rigs and its dispatcher's blocks) — the v14 shape (idempotent; the manifest is
/// the caller's).
pub fn strip_yard(doc: &mut Value) {
    if let Some(site) = doc["entries"]
        .as_object_mut()
        .and_then(|entries| entries.get_mut("occator_port"))
        .and_then(|occator| occator["def"].get_mut("site"))
        .and_then(Value::as_object_mut)
    {
        site.remove("yard");
    }
}

/// **Whether `submit` is the house's** (doc 20 cut SM.12b): a yard dispatcher's own submission —
/// its drones' plan steps (acts and finishes) and its queued unloads. The house works Occator Rim
/// in every keyed world, so a suite counting one player's starts or finishes at a fire counts
/// the players' only; the house's are landed like any other.
#[must_use]
pub fn is_house(tw: &mush_testkit::TestWorld, submit: &mush_core::AuthoritativeSubmit) -> bool {
    tacsim::port::yard::dispatchers(tw.world()).contains(&submit.actor)
}
