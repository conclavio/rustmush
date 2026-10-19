//! **The moon ports stripped away** (doc 19 cut A.8, Q17): helpers that rebuild an
//! older lane shape from the shipped v8 for the suites that pin the v7 supplier
//! graph — `price.rs`'s Stickney variant (Ceres as Stickney's only supplier),
//! `ports.rs`'s lane v1 (the places kept, so the placed rooms keep their ids) and
//! `commerce.rs`'s published v2 rebuild (its digest pinned byte for byte).

use serde_json::Value;

/// The three moon port entries.
pub const MOON_PORTS: [&str; 3] = ["luna_port", "deimos_port", "titan_port"];
/// The six direct routes cut A.8 listed.
pub const MOON_ROUTES: [&str; 6] = [
    "ceres_aitken",
    "aitken_voltaire",
    "aitken_ligeia",
    "ligeia_aitken",
    "ligeia_voltaire",
    "voltaire_stickney",
];
/// The three moon places.
pub const MOON_PLACES: [&str; 3] = ["aitken_highport", "voltaire_anchorage", "ligeia_terminal"];

/// Remove everything cut A.8 added to the lane's trade — the three ports, the six
/// routes, Ceres's `luna` routes key and the programme's two reference rows for
/// goods v2 — leaving the three places (and the bodies, which cut A.7 added).
pub fn strip_moon_trade(doc: &mut Value) {
    let entries = doc["entries"].as_object_mut().expect("the lane's entries");
    for id in MOON_PORTS.iter().chain(MOON_ROUTES.iter()) {
        entries.remove(*id);
    }
    if let Some(ceres) = entries.get_mut("ceres_port") {
        if let Some(routes) = ceres["def"]["routes"].as_object_mut() {
            routes.remove("luna");
        }
        if let Some(reference) = ceres["def"]["programme"]["reference"].as_object_mut() {
            reference.remove("instrument_boards");
            reference.remove("carbon_feedstock");
        }
    }
}

/// Remove the three moon places too (the v2 rebuild; a v1 keeps them).
pub fn strip_moon_places(doc: &mut Value) {
    let entries = doc["entries"].as_object_mut().expect("the lane's entries");
    for id in MOON_PLACES {
        entries.remove(id);
    }
}
