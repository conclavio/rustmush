//! **The lane's newest strip chain** (doc 43 cut TS.8): what lanes v17–v43 added, stripped
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

/// **Doc 48 unit EX.10 (lane v43): the hauling house taken out** — Ceres's Vesta Carrying
/// Company (its org row and the entry's `hauler` block) and the Deimos reserve's `consume` rows,
/// so a strip chain rebuilds a lane before v43 (idempotent). The chain's head.
pub fn strip_hauling(doc: &mut Value) {
    let Some(entries) = doc.get_mut("entries") else {
        return;
    };
    if let Some(def) = entries
        .get_mut("ceres_port")
        .and_then(|entry| entry.get_mut("def"))
        .and_then(Value::as_object_mut)
    {
        def.remove("hauler");
        if let Some(orgs) = def.get_mut("orgs").and_then(Value::as_array_mut) {
            orgs.retain(|org| org["id"] != "vesta_carriers");
        }
    }
    if let Some(vendors) = entries
        .get_mut("deimos_port")
        .and_then(|entry| entry.get_mut("def"))
        .and_then(|def| def.get_mut("vendors"))
        .and_then(Value::as_array_mut)
    {
        for vendor in vendors
            .iter_mut()
            .filter(|v| v["org"] == "strategic_reserve")
        {
            if let Some(vendor) = vendor.as_object_mut() {
                vendor.remove("consume");
            }
        }
    }
}

/// **Doc 48 unit EX.9 (lane v42): the consumers' rows taken out** — every vendor's `consume`
/// rows (the Habitat Board, Syrtis Bonded and the Selk Townships Board), so a strip chain
/// rebuilds a lane before v42 (idempotent); lane v43's [`strip_hauling`] first.
pub fn strip_consumption(doc: &mut Value) {
    strip_hauling(doc);
    let Some(entries) = doc.get_mut("entries").and_then(Value::as_object_mut) else {
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
        for vendor in vendors.iter_mut().filter_map(Value::as_object_mut) {
            vendor.remove("consume");
        }
    }
}

/// **Doc 48 unit PL.8 (lane v41): the workshops' part bids taken out** — every part bid of
/// Occator's at Ceres, and Stickney Yards' but its light plant's, which goes back to IN.11's 4,000
/// with no refresh in its old place (Q-48-29), so a strip chain rebuilds a lane before v41
/// (idempotent); lane v42's [`strip_consumption`] first.
pub fn strip_parts_market(doc: &mut Value) {
    strip_consumption(doc);
    let light = "tacsim.part.plant.light_mk1";
    for (entry, org) in [
        ("ceres_port", "occator"),
        ("stickney_port", "stickney_yards"),
    ] {
        let Some(bids) = doc
            .get_mut("entries")
            .and_then(|entries| entries.get_mut(entry))
            .and_then(|entry| entry.get_mut("def"))
            .and_then(|def| def.get_mut("vendors"))
            .and_then(Value::as_array_mut)
            .and_then(|vendors| vendors.iter_mut().find(|vendor| vendor["org"] == org))
            .and_then(|vendor| vendor.get_mut("bids"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        let had_light = bids.iter().any(|bid| bid["good"] == light);
        bids.retain(|bid| {
            !bid["good"]
                .as_str()
                .is_some_and(|good| good.starts_with("tacsim.part."))
        });
        if org == "stickney_yards" && had_light {
            let at = 1.min(bids.len());
            bids.insert(
                at,
                serde_json::json!({ "good": light, "price_major": 4000, "intake": 4, "boards": ["yard"] }),
            );
        }
    }
}

/// **Doc 48 unit PL.6 (lane v40): the Ceres yard taken out** — Ceres's `yard` block and
/// Stickney's heavy-yard capability (Q-48-25), so a strip chain rebuilds a lane before v40
/// (idempotent); lane v41's [`strip_parts_market`] first.
pub fn strip_ceres_yard(doc: &mut Value) {
    strip_parts_market(doc);
    let Some(entries) = doc.get_mut("entries") else {
        return;
    };
    if let Some(def) = entries
        .get_mut("ceres_port")
        .and_then(|entry| entry.get_mut("def"))
        .and_then(Value::as_object_mut)
    {
        def.remove("yard");
    }
    if let Some(capabilities) = entries
        .get_mut("stickney_port")
        .and_then(|entry| entry.get_mut("def"))
        .and_then(|def| def.get_mut("yard"))
        .and_then(|yard| yard.get_mut("capabilities"))
        .and_then(Value::as_array_mut)
    {
        capabilities.retain(|word| word != "tacsim.yard.heavy");
    }
}

/// **Doc 48 unit PL.5 (lane v39): Stickney's escrow asks taken out** — Kasei's strategic-ore
/// stock row and ask and Stickney Yards' instrument-board stock row and ask, the asks a bill
/// is bought at with no hold (Q-48-28), so a strip chain rebuilds a lane before v39
/// (idempotent); lane v40's [`strip_ceres_yard`] first.
pub fn strip_escrow_asks(doc: &mut Value) {
    strip_ceres_yard(doc);
    for (org, good) in [
        ("kasei", "strategic_ore"),
        ("stickney_yards", "instrument_boards"),
    ] {
        let Some(vendor) = doc
            .get_mut("entries")
            .and_then(|entries| entries.get_mut("stickney_port"))
            .and_then(|entry| entry.get_mut("def"))
            .and_then(|def| def.get_mut("vendors"))
            .and_then(Value::as_array_mut)
            .and_then(|vendors| vendors.iter_mut().find(|vendor| vendor["org"] == org))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        for rows in ["stock", "asks"] {
            if let Some(rows) = vendor.get_mut(rows).and_then(Value::as_array_mut) {
                rows.retain(|row| row["good"] != good);
            }
        }
    }
}

/// **Doc 48 unit EX.3 (lane v38): the orgs' market roles taken out** — every port org's
/// `roles` row (the acceptors, Q-48-9), so a strip chain rebuilds a lane before v38
/// (idempotent); lane v39's [`strip_escrow_asks`] first.
pub fn strip_roles(doc: &mut Value) {
    strip_escrow_asks(doc);
    for org in doc
        .get_mut("entries")
        .and_then(Value::as_object_mut)
        .into_iter()
        .flat_map(|entries| entries.values_mut())
        .filter_map(|entry| entry.get_mut("def"))
        .filter_map(|def| def.get_mut("orgs"))
        .filter_map(Value::as_array_mut)
        .flatten()
        .filter_map(Value::as_object_mut)
    {
        org.remove("roles");
    }
}

/// **The shipped lane's org roles put back** on a stripped lane (doc 48 unit EX.3): every org
/// row `shipped` gives roles gets them again, matched by entry and org id. For a suite that
/// boots an older lane's shape to exercise today's code on it: the acceptors are lane data now
/// (Q-48-9), so without them no house takes a quote. A digest rebuild never calls it.
#[allow(
    dead_code,
    reason = "mush-server's split_boundary includes this file and rebuilds digests only"
)]
pub fn keep_roles(doc: &mut Value, shipped: &Value) {
    let Some(entries) = shipped.get("entries").and_then(Value::as_object) else {
        return;
    };
    for (key, entry) in entries {
        for row in entry
            .get("def")
            .and_then(|def| def.get("orgs"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(id), Some(roles)) = (row.get("id"), row.get("roles")) else {
                continue;
            };
            if let Some(org) = doc
                .get_mut("entries")
                .and_then(|entries| entries.get_mut(key))
                .and_then(|entry| entry.get_mut("def"))
                .and_then(|def| def.get_mut("orgs"))
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
                .find(|org| org.get("id") == Some(id))
                .and_then(Value::as_object_mut)
            {
                org.insert("roles".to_owned(), roles.clone());
            }
        }
    }
}

/// **Doc 48 unit PL.3 (lane v37): Stickney Yards' munitions taken out** — its seven munitions
/// processes, its alloys bid, and its torpedo, spinal-round and point-defence stock
/// rows and asks — so a strip chain rebuilds a lane before v37 (idempotent); lane v38's
/// [`strip_roles`] first.
pub fn strip_munitions(doc: &mut Value) {
    const SPACE: [&str; 3] = ["torpedo", "spinal_round", "pd_box"];
    strip_roles(doc);
    let Some(vendor) = doc
        .get_mut("entries")
        .and_then(|entries| entries.get_mut("stickney_port"))
        .and_then(|entry| entry.get_mut("def"))
        .and_then(|def| def.get_mut("vendors"))
        .and_then(Value::as_array_mut)
        .and_then(|vendors| {
            vendors
                .iter_mut()
                .find(|vendor| vendor["org"] == "stickney_yards")
        })
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    vendor.remove("mill");
    for rows in ["stock", "asks"] {
        if let Some(rows) = vendor.get_mut(rows).and_then(Value::as_array_mut) {
            rows.retain(|row| !SPACE.iter().any(|good| row["good"] == *good));
        }
    }
    if let Some(bids) = vendor.get_mut("bids").and_then(Value::as_array_mut) {
        bids.retain(|bid| bid["good"] != "structural_alloys");
    }
}

/// **Doc 48 unit PL.2 (lane v36): the house processes taken out** — the Lunar Habitat Board's
/// farm (its provisions stock, ask and mill), Crisium Instruments' fab (its strategic-ore and
/// metals bids and mill) and Titan Concession's farm and carbon mill (its provisions stock and
/// ask, its carbon-ore bid and both mill rows) — so a strip chain rebuilds a lane before v36
/// (idempotent); lane v37's [`strip_munitions`] first.
pub fn strip_producers(doc: &mut Value) {
    strip_munitions(doc);
    for (entry, org) in [
        ("luna_port", "habitat_board"),
        ("luna_port", "crisium_instruments"),
        ("titan_port", "titan_concession"),
    ] {
        let Some(vendor) = doc
            .get_mut("entries")
            .and_then(|entries| entries.get_mut(entry))
            .and_then(|entry| entry.get_mut("def"))
            .and_then(|def| def.get_mut("vendors"))
            .and_then(Value::as_array_mut)
            .and_then(|vendors| vendors.iter_mut().find(|vendor| vendor["org"] == org))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        match org {
            // The farm's whole storefront side: it bought, and listed nothing, before v36.
            "habitat_board" => {
                vendor.remove("stock");
                vendor.remove("asks");
                vendor.remove("mill");
            }
            // The fab's two bids and its process: Crisium only sold boards before v36.
            "crisium_instruments" => {
                vendor.remove("bids");
                vendor.remove("mill");
            }
            _ => {
                vendor.remove("bids");
                vendor.remove("mill");
                for rows in ["stock", "asks"] {
                    if let Some(rows) = vendor.get_mut(rows).and_then(Value::as_array_mut) {
                        rows.retain(|row| row["good"] != "provisions");
                    }
                }
            }
        }
    }
}

/// **Doc 48 unit PL.1 (lane v35): Occator's propellant refine taken out** — its `volatiles`
/// mill row and the Ceres refinery it runs at — so a strip chain rebuilds a lane before v35
/// (idempotent); lane v36's [`strip_producers`] first.
pub fn strip_refine(doc: &mut Value) {
    strip_producers(doc);
    let Some(def) = doc
        .get_mut("entries")
        .and_then(|entries| entries.get_mut("ceres_port"))
        .and_then(|entry| entry.get_mut("def"))
    else {
        return;
    };
    if let Some(def) = def.as_object_mut() {
        def.remove("refinery");
    }
    for vendor in def
        .get_mut("vendors")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter(|vendor| vendor["org"] == "occator")
    {
        if let Some(mills) = vendor.get_mut("mill").and_then(Value::as_array_mut) {
            mills.retain(|mill| mill["ore"] != "volatiles");
        }
    }
}

/// **Doc 47 unit CO.10 (lane v34): Occator's wrecker design taken out** of the vehicles'
/// designs, so a strip chain rebuilds a lane before v34 (idempotent); lane v35's
/// [`strip_refine`] first.
pub fn strip_wrecker(doc: &mut Value) {
    strip_refine(doc);
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
            designs.retain(|design| design.get("word").and_then(Value::as_str) != Some("wrecker"));
        }
    }
}

/// **The owner's IN.11 house-restock ruling (lane v33, 2026-11-05): Stickney Yards' carbon
/// taken out** — the stock row, the ask and the bid at Stickney, so a strip chain rebuilds a
/// lane before v33 (idempotent); lane v34's [`strip_wrecker`] first.
pub fn strip_stickney_carbon(doc: &mut Value) {
    strip_wrecker(doc);
    for vendor in doc
        .get_mut("entries")
        .and_then(|entries| entries.get_mut("stickney_port"))
        .and_then(|entry| entry.get_mut("def"))
        .and_then(|def| def.get_mut("vendors"))
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter(|vendor| vendor["org"] == "stickney_yards")
    {
        for rows in ["stock", "asks", "bids"] {
            if let Some(rows) = vendor.get_mut(rows).and_then(Value::as_array_mut) {
                rows.retain(|row| row["good"] != "carbon_ore");
            }
        }
    }
}

/// **Doc 39 cut IN.11 (lane v32): Freeport Works' strategic ore and the Yards' part bid taken
/// out** — the stock row and the ask at Ceres and the light-plant bid at Stickney, so a strip
/// chain rebuilds a lane before v32 (idempotent), after v33's rows. The programme's
/// strategic-ore reference goes back from that ask to its fixed 300.00cr.
pub fn strip_strategic_ask(doc: &mut Value) {
    strip_stickney_carbon(doc);
    if let Some(def) = doc
        .get_mut("entries")
        .and_then(|entries| entries.get_mut("ceres_port"))
        .and_then(|entry| entry.get_mut("def"))
    {
        if let Some(programme) = def.get_mut("programme").and_then(Value::as_object_mut)
            && programme
                .get_mut("reference")
                .and_then(Value::as_object_mut)
                .and_then(|reference| reference.remove("strategic_ore"))
                .is_some()
            && let Some(fixed) = programme
                .get_mut("reference_minor")
                .and_then(Value::as_object_mut)
        {
            fixed.insert("strategic_ore".to_owned(), 30_000.into());
        }
        for vendor in def
            .get_mut("vendors")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .filter(|vendor| vendor["org"] == "freeport_works")
        {
            for rows in ["stock", "asks"] {
                if let Some(rows) = vendor.get_mut(rows).and_then(Value::as_array_mut) {
                    rows.retain(|row| row["good"] != "strategic_ore");
                }
            }
        }
    }
    // The Yards' light-plant bid at Stickney.
    for vendor in doc
        .get_mut("entries")
        .and_then(|entries| entries.get_mut("stickney_port"))
        .and_then(|entry| entry.get_mut("def"))
        .and_then(|def| def.get_mut("vendors"))
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter(|vendor| vendor["org"] == "stickney_yards")
    {
        if let Some(bids) = vendor.get_mut("bids").and_then(Value::as_array_mut) {
            bids.retain(|row| row["good"] != "tacsim.part.plant.light_mk1");
        }
    }
}

/// **The player-response floor (lane v31; owner rule, 2026-11-04): the licence's grace put
/// back** — the Freeport Registry licence's `grace_ic_days` 28 → 3, so a strip chain rebuilds
/// a lane before v31 (idempotent), after IN.11's rows.
pub fn strip_licence_grace(doc: &mut Value) {
    strip_strategic_ask(doc);
    if let Some(licence) = doc
        .get_mut("entries")
        .and_then(|entries| entries.get_mut("ceres_port"))
        .and_then(|entry| entry.get_mut("def"))
        .and_then(|def| def.get_mut("licence"))
        .and_then(Value::as_object_mut)
    {
        licence.insert("grace_ic_days".into(), Value::from(3));
    }
}

/// **Doc 39 cut IN.10 (lane v29): the Trust's raise taken out** — `coop_trust`'s `raises` at
/// Ceres removed, so a strip chain rebuilds a lane before v29 (idempotent), after the
/// licence's grace.
pub fn strip_trust_raise(doc: &mut Value) {
    strip_licence_grace(doc);
    let Some(orgs) = doc
        .get_mut("entries")
        .and_then(|entries| entries.get_mut("ceres_port"))
        .and_then(|entry| entry.get_mut("def"))
        .and_then(|def| def.get_mut("orgs"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for org in orgs {
        if org.get("id").and_then(Value::as_str) == Some("coop_trust")
            && let Some(org) = org.as_object_mut()
        {
            org.remove("raises");
        }
    }
}

/// **Doc 41 cut RS.6a (lane v28): Tycho Mutual's house taken out** — Ceres's `recovery.house`
/// block and Tycho's 450,000 raise (Q-41-17: 500,000 in all), so a strip chain
/// rebuilds a lane before v28 (idempotent), after IN.10's raise.
pub fn strip_tycho_house(doc: &mut Value) {
    strip_trust_raise(doc);
    if let Some(def) = doc["entries"]
        .as_object_mut()
        .and_then(|entries| entries.get_mut("ceres_port"))
        .and_then(|entry| entry.get_mut("def"))
        .and_then(Value::as_object_mut)
    {
        if let Some(recovery) = def.get_mut("recovery").and_then(Value::as_object_mut) {
            recovery.remove("house");
        }
        for org in def
            .get_mut("orgs")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .filter(|org| org["id"] == "tycho_mutual")
        {
            if let Some(org) = org.as_object_mut() {
                org.remove("raises");
            }
        }
    }
}

/// **Doc 39 cut IN.9 (lane v27): the yard taken out** — Stickney's `yard` block and the Yards'
/// 50,000 raise (IN.7's, folded into v27 on the owner's ruling), so a strip chain rebuilds a
/// lane before v27 (idempotent).
pub fn strip_yard(doc: &mut Value) {
    strip_tycho_house(doc);
    if let Some(def) = doc["entries"]
        .as_object_mut()
        .and_then(|entries| entries.get_mut("stickney_port"))
        .and_then(|entry| entry.get_mut("def"))
        .and_then(Value::as_object_mut)
    {
        def.remove("yard");
        for org in def
            .get_mut("orgs")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .filter(|org| org["id"] == "stickney_yards")
        {
            if let Some(org) = org.as_object_mut() {
                org.remove("raises");
            }
        }
    }
}

/// **Doc 39 cut IN.7: the part recipes' lane rows taken out** — Stickney Yards' strategic-ore
/// bid removed and IN.6's seeded part stock put back ([`SEEDED_PARTS`]) — so a strip chain
/// rebuilds the lane before IN.7 (idempotent). A suite that wants the seeded stores runs on
/// this lane.
pub fn strip_part_recipes(doc: &mut Value) {
    strip_yard(doc);
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
