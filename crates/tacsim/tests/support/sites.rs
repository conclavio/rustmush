//! **The mining sites stripped away** (doc 20 cut SM.2): helpers that rebuild the v8 lane shape
//! from the shipped v9 for the suites that rebuild an older lane — the six site ports, places and
//! twelve routes, Ceres's six site routes keys, the Freeport Registry org, the eight bodies'
//! surface figures and tables, and the Freeport's coordinates.

use serde_json::Value;

/// The six site port entries, in the lane's site order.
pub const SITE_PORTS: [&str; 6] = [
    "commons_port",
    "kerwan_port",
    "ahuna_port",
    "juling_port",
    "occator_port",
    "rheasilvia_port",
];
/// The six site places, in the same order.
pub const SITE_PLACES: [&str; 6] = [
    "freeport_commons",
    "kerwan_floor",
    "ahuna_flank",
    "juling_shadow",
    "occator_rim",
    "rheasilvia_commons",
];
/// The six venues, in the same order.
pub const SITE_VENUES: [&str; 6] = [
    "commons",
    "kerwan",
    "ahuna",
    "juling",
    "occator",
    "rheasilvia",
];
/// The twelve site routes, out and back per site.
pub const SITE_ROUTES: [&str; 12] = [
    "ceres_commons",
    "commons_ceres",
    "ceres_kerwan",
    "kerwan_ceres",
    "ceres_ahuna",
    "ahuna_ceres",
    "ceres_juling",
    "juling_ceres",
    "ceres_occator",
    "occator_ceres",
    "ceres_rheasilvia",
    "rheasilvia_ceres",
];
/// The bodies cut SM.2 gave surface figures.
pub const SURFACE_BODIES: [&str; 8] = [
    "ceres", "vesta", "pallas", "luna", "mars", "titan", "phobos", "deimos",
];

/// Remove everything cut SM.2 added to the lane — the v8 shape (the manifest is the caller's) —
/// and, since lane v10, cut SM.3a's blocks (`super::vehicles::strip_vehicles`, first).
pub fn strip_sites(doc: &mut Value) {
    super::vehicles::strip_vehicles(doc);
    let entries = doc["entries"].as_object_mut().expect("the lane's entries");
    for id in SITE_PORTS
        .iter()
        .chain(SITE_PLACES.iter())
        .chain(SITE_ROUTES.iter())
    {
        entries.remove(*id);
    }
    for body in SURFACE_BODIES {
        if let Some(def) = entries
            .get_mut(body)
            .and_then(|entry| entry["def"].as_object_mut())
        {
            def.remove("surface_mg");
            def.remove("radius_km");
            def.remove("geology");
        }
    }
    if let Some(def) = entries
        .get_mut("ceres_freeport")
        .and_then(|entry| entry["def"].as_object_mut())
    {
        def.remove("lat_mdeg");
        def.remove("lon_mdeg");
    }
    if let Some(ceres) = entries.get_mut("ceres_port") {
        if let Some(routes) = ceres["def"]["routes"].as_object_mut() {
            for venue in SITE_VENUES {
                routes.remove(venue);
            }
        }
        if let Some(orgs) = ceres["def"]["orgs"].as_array_mut() {
            orgs.retain(|org| org["id"] != "freeport_registry");
        }
    }
}
