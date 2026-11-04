//! **The lane's newest strip chain** (doc 43 cut TS.8): what lanes v17–v21 added, stripped
//! newest first, so a suite that rebuilds an older lane from the shipped one calls one function
//! and a lane bump extends one chain.
//!
//! The one copy: tacsim's suites reach it through `support` (`sites::strip_sites` and
//! `drones` call [`strip_wide_sites`]), and mush-server's `split_boundary` binary
//! includes this file by path for the relay registry's lane rebuilds, which pin the published
//! digests. It needs nothing but `serde_json`, so it stays includable from another crate.

use serde_json::Value;

/// **Doc 32 cut S.1: the 512 site's rows** — Urvara Reach's place, its two routes and its port.
pub const WIDE_SITE_ROWS: [&str; 4] = [
    "urvara_reach",
    "ceres_urvara",
    "urvara_ceres",
    "urvara_port",
];

/// **Doc 36 cut GC.7: the munitions goods the lane's vendors stock.**
pub const MUNITIONS_GOODS: [&str; 4] =
    ["ammo_ap_30", "ammo_ball_762", "ammo_he_105", "ammo_missile"];

/// **Doc 41 cut RS.1 (lane v21): Occator's tender design taken out** of the vehicles' designs,
/// so a strip chain rebuilds a lane before v21 (idempotent).
pub fn strip_tender(doc: &mut Value) {
    for entry in doc["entries"]
        .as_object_mut()
        .into_iter()
        .flat_map(|entries| entries.values_mut())
    {
        // `get_mut`, never `IndexMut`: indexing would write `null` into every entry it misses.
        if let Some(designs) = entry
            .get_mut("def")
            .and_then(|def| def.get_mut("vehicles"))
            .and_then(|vehicles| vehicles.get_mut("designs"))
            .and_then(Value::as_array_mut)
        {
            designs.retain(|design| design.get("word").and_then(Value::as_str) != Some("tender"));
        }
    }
}

/// **Doc 39 cut IN.1 (lane v20): the ladder's three prices put back** — Kasei's alloy ask 450 →
/// 400, Stickney Yards' fittings ask 520 → 400, the Co-op's fittings bid 600 → 560 — so a strip
/// chain rebuilds a lane before v20 (idempotent); lane v21's [`strip_tender`] first.
pub fn strip_ladder(doc: &mut Value) {
    strip_tender(doc);
    for (port, org, side, good, was) in [
        ("stickney_port", "kasei", "asks", "structural_alloys", 400),
        (
            "stickney_port",
            "stickney_yards",
            "asks",
            "industrial_parts",
            400,
        ),
        ("ceres_port", "coop", "bids", "industrial_parts", 560),
    ] {
        for vendor in doc["entries"][port]["def"]["vendors"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .filter(|vendor| vendor["org"] == org)
        {
            for row in vendor[side]
                .as_array_mut()
                .into_iter()
                .flatten()
                .filter(|row| row["good"] == good)
            {
                row["price_major"] = was.into();
            }
        }
    }
}

/// Remove what doc 36 cut GC.7 added to the lane (lane v18): Ceres's and Stickney's `workshop`
/// blocks and every vendor's munitions stock and asks — the lane v17 shape but its manifest
/// (idempotent). Cut GC.13's `medical` blocks (lane v19) go with them, and since lane v20
/// doc 39 cut IN.1's ladder ([`strip_ladder`]) first and cut IN.4's Ceres `recovery` row (the
/// workshops' `cost` goes with their blocks).
pub fn strip_workshops(doc: &mut Value) {
    strip_ladder(doc);
    let Some(entries) = doc["entries"].as_object_mut() else {
        return;
    };
    for port in ["ceres_port", "stickney_port"] {
        let Some(def) = entries
            .get_mut(port)
            .and_then(|entry| entry["def"].as_object_mut())
        else {
            continue;
        };
        def.remove("workshop");
        def.remove("medical");
        def.remove("recovery");
        for vendor in def
            .get_mut("vendors")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            for rows in ["stock", "asks"] {
                if let Some(rows) = vendor[rows].as_array_mut() {
                    rows.retain(|row| {
                        !MUNITIONS_GOODS.contains(&row["good"].as_str().unwrap_or_default())
                    });
                }
            }
        }
    }
    if let Some(def) = entries
        .get_mut("kirkwood_depot")
        .and_then(|entry| entry["def"].as_object_mut())
    {
        def.remove("kirkwood_picket");
        def.remove("clusters");
    }
    if let Some(legs) = entries
        .get_mut("ceres_stickney")
        .and_then(|entry| entry["def"]["legs"].as_array_mut())
        && let Some(leg0) = legs.first_mut().and_then(Value::as_object_mut)
    {
        leg0.remove("hazard");
        leg0.remove("interdiction");
    }
}

/// Remove what doc 32 cut S.1 added to the lane (lane v17): Urvara Reach's rows and Ceres's
/// `urvara` routes key — the lane v16 shape but its manifest. Since lane v18, doc 36 cut GC.7's
/// rows first ([`strip_workshops`]), so every strip chain rebuilds its older lane.
pub fn strip_wide_sites(doc: &mut Value) {
    strip_workshops(doc);
    let entries = doc["entries"].as_object_mut().expect("the lane's entries");
    for id in WIDE_SITE_ROWS {
        entries.remove(id);
    }
    if let Some(routes) = entries
        .get_mut("ceres_port")
        .and_then(|ceres| ceres["def"]["routes"].as_object_mut())
    {
        routes.remove("urvara");
    }
}
