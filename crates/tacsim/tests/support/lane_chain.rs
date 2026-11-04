//! **The lane's newest strip chain** (doc 43 cut TS.8): what lanes v17–v26 added, stripped
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

/// **Doc 39 cut IN.6's seeded part stock, as lane v22 authored it** (Q-39-12): Occator at
/// Ceres four of each escort and rig part, Stickney Yards six of every row — the rows IN.7
/// retires, in their authored order.
pub const SEEDED_PARTS: [(&str, &str, u64); 18] = [
    ("occator", "tacsim.part.weapon.cannon_105", 4),
    ("occator", "tacsim.part.weapon.mg_coax", 4),
    ("occator", "tacsim.part.sensor.visual_mk1", 4),
    ("occator", "tacsim.part.plant.vehicle_mk1", 4),
    ("occator", "tacsim.part.plant.light_mk1", 4),
    ("stickney_yards", "tacsim.part.weapon.cannon_105", 6),
    ("stickney_yards", "tacsim.part.weapon.autocannon_30", 6),
    ("stickney_yards", "tacsim.part.weapon.laser_mk1", 6),
    ("stickney_yards", "tacsim.part.weapon.missile_lr", 6),
    ("stickney_yards", "tacsim.part.weapon.mg_coax", 6),
    ("stickney_yards", "tacsim.part.sensor.visual_mk1", 6),
    ("stickney_yards", "tacsim.part.sensor.thermal_mk1", 6),
    ("stickney_yards", "tacsim.part.sensor.ecm_mk1", 6),
    ("stickney_yards", "tacsim.part.sensor.eccm_mk1", 6),
    ("stickney_yards", "tacsim.part.plant.light_mk1", 6),
    ("stickney_yards", "tacsim.part.plant.vehicle_mk1", 6),
    ("stickney_yards", "tacsim.part.plant.walker_mk1", 6),
    ("stickney_yards", "tacsim.part.plant.hauler_mk1", 6),
];

/// **Doc 39 cut IN.7: the part recipes' lane rows taken out** — Stickney Yards' strategic-ore
/// bid removed and IN.6's seeded part stock put back ([`SEEDED_PARTS`]) — so a strip chain
/// rebuilds the lane before IN.7 (idempotent). A suite that wants the seeded stores runs on
/// this lane.
pub fn strip_part_recipes(doc: &mut Value) {
    let Some(entries) = doc["entries"].as_object_mut() else {
        return;
    };
    for entry in entries.values_mut() {
        let Some(vendors) = entry
            .get_mut("def")
            .and_then(|def| def.get_mut("vendors"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for vendor in vendors {
            let org = vendor["org"].as_str().unwrap_or_default().to_owned();
            if org == "stickney_yards"
                && let Some(vendor) = vendor.as_object_mut()
            {
                vendor.remove("bids");
            }
            let Some(stock) = vendor.get_mut("stock").and_then(Value::as_array_mut) else {
                continue;
            };
            for (seller, good, qty) in SEEDED_PARTS {
                if seller == org && !stock.iter().any(|row| row["good"] == good) {
                    stock.push(serde_json::json!({
                        "good": good, "qty": qty, "grade": 9, "issue": 1
                    }));
                }
            }
        }
    }
}

/// **Doc 39 cut IN.5 (lane v24): the refinery and the works taken out** — Stickney's `refinery`
/// and `works` blocks, Kasei's refine row and its bid's `refresh`, Ceres's `works` block, and the
/// Freeport Works org and vendor (its carbon stock and ask) — so a strip chain rebuilds a lane
/// before v24 (idempotent). The programme's carbon reference goes back from Freeport Works'
/// listing to its fixed 25.00cr.
pub fn strip_industry(doc: &mut Value) {
    strip_part_recipes(doc);
    let Some(entries) = doc["entries"].as_object_mut() else {
        return;
    };
    // `get_mut`, never `IndexMut`: indexing a missing key inserts it.
    if let Some(def) = entries
        .get_mut("stickney_port")
        .and_then(|entry| entry.get_mut("def"))
        .and_then(Value::as_object_mut)
    {
        def.remove("refinery");
        def.remove("works");
        for vendor in def
            .get_mut("vendors")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .filter(|vendor| vendor["org"] == "kasei")
        {
            if let Some(vendor) = vendor.as_object_mut() {
                vendor.remove("mill");
            }
            for bid in vendor
                .get_mut("bids")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
                .filter(|bid| bid["good"] == "metals")
            {
                if let Some(bid) = bid.as_object_mut() {
                    bid.remove("refresh");
                }
            }
        }
    }
    let Some(def) = entries
        .get_mut("ceres_port")
        .and_then(|entry| entry.get_mut("def"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    def.remove("works");
    if let Some(programme) = def.get_mut("programme").and_then(Value::as_object_mut)
        && programme
            .get_mut("reference")
            .and_then(Value::as_object_mut)
            .and_then(|reference| reference.remove("carbon_ore"))
            .is_some()
        && let Some(fixed) = programme
            .get_mut("reference_minor")
            .and_then(Value::as_object_mut)
    {
        fixed.insert("carbon_ore".to_owned(), 2500.into());
    }
    if let Some(orgs) = def.get_mut("orgs").and_then(Value::as_array_mut) {
        orgs.retain(|org| org["id"] != "freeport_works");
    }
    if let Some(vendors) = def.get_mut("vendors").and_then(Value::as_array_mut) {
        vendors.retain(|vendor| vendor["org"] != "freeport_works");
    }
}

/// **Doc 42 cut CR.2 (lane v23): the IC ports' `authority` and `release_fee_cr` taken out**, so a
/// strip chain rebuilds a lane before v23 (idempotent); lane v24's [`strip_industry`] first.
pub fn strip_release_authority(doc: &mut Value) {
    strip_industry(doc);
    let Some(entries) = doc["entries"].as_object_mut() else {
        return;
    };
    // `get_mut`, never `IndexMut`: indexing a missing key inserts it.
    for entry in entries.values_mut() {
        if let Some(def) = entry.get_mut("def").and_then(Value::as_object_mut) {
            def.remove("authority");
            def.remove("release_fee_cr");
        }
    }
}

/// **Doc 39 cut IN.6 (lane v22): the seeded part rows taken out**: every vendor's `stock` and
/// `asks` row of a `tacsim.part.*` good, so a strip chain rebuilds a lane before v22
/// (idempotent); lane v23's [`strip_release_authority`] first.
pub fn strip_part_seeds(doc: &mut Value) {
    strip_release_authority(doc);
    let Some(entries) = doc["entries"].as_object_mut() else {
        return;
    };
    // `get_mut`, never `IndexMut`: indexing a missing key inserts it.
    for entry in entries.values_mut() {
        let Some(vendors) = entry
            .get_mut("def")
            .and_then(|def| def.get_mut("vendors"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for vendor in vendors {
            for side in ["stock", "asks"] {
                if let Some(rows) = vendor.get_mut(side).and_then(Value::as_array_mut) {
                    rows.retain(|row| {
                        !row["good"]
                            .as_str()
                            .is_some_and(|good| good.starts_with("tacsim.part."))
                    });
                }
            }
        }
    }
}

/// **Doc 41 cut RS.1 (lane v21): Occator's tender design taken out** of the vehicles' designs,
/// so a strip chain rebuilds a lane before v21 (idempotent); lane v22's [`strip_part_seeds`]
/// first.
pub fn strip_tender(doc: &mut Value) {
    strip_part_seeds(doc);
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
