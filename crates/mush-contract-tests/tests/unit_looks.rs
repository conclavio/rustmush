//! **Every unit type has a look, and every look fits its units** (doc 22, the
//! unit kit, D-22-6).
//!
//! A unit is two halves kept apart (D-22-1): its rules, the tacsim rules packs
//! under `crates/tacsim/content/units/`, and its look, the files `mush-pixel`
//! ships under `crates/mush-pixel/looks/`. Neither crate sees the other, so
//! this test holds them together: every shipped unit type is drawn by a look
//! (or is named below as drawing `unknown`, with the reason); every unit a look
//! names exists; every section of every unit it draws is in its doll; every
//! doll part names a section — and a facet of it — each of those units has;
//! every weapon short name, and every sprite muzzle, names a weapon component
//! of one of them. The reserved `unknown` look (an unclassified contact's)
//! names no unit.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mush_pixel::look::{self, Look};
use serde_json::Value;

/// Unit types shipped without a look, and why: they draw as `unknown` and
/// their pilots get the section rows with no doll (D-22-6), never a failure.
const LOOKLESS: &[(&str, &str)] = &[
    (
        "tacsim.unit.halcyon",
        "a non-fieldable capital relief hull; ship and relay status use section rows, with no arena sprite",
    ),
    (
        "tacsim.unit.relay_station",
        "a fixed, non-fieldable relay installation; structural status uses section rows, with no arena sprite",
    ),
    (
        "tacsim.unit.dray",
        "a hauler, never in the arena: its look comes with its own cut",
    ),
    (
        "tacsim.unit.dray_p",
        "a hauler, never in the arena: its look comes with its own cut",
    ),
    (
        "tacsim.unit.tv_prospector",
        "a mining terrain vehicle, never in the arena: its look comes with its own cut",
    ),
    (
        "tacsim.unit.conduct_carrier",
        "the unarmed Safe Conduct proving carrier (doc 17f M7.6), loaded only for the Registry's Conduct trials: its look comes with its own cut",
    ),
];

/// One shipped unit type's rules, as far as a look needs them.
struct Unit {
    /// Each section and its facets.
    sections: BTreeMap<String, Vec<String>>,
    /// Its weapon component ids.
    weapons: Vec<String>,
}

fn shipped_units() -> BTreeMap<String, Unit> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tacsim/content/units");
    let mut units = BTreeMap::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the unit packs")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for path in files {
        let text = std::fs::read_to_string(&path).expect("a unit pack");
        let pack: Value = serde_json::from_str(&text).expect("JSON");
        let entries = pack["entries"].as_object().expect("entries");
        for (id, entry) in entries {
            if entry["kind"] != "unit" {
                continue;
            }
            let def = &entry["def"];
            let sections = def["sections"]
                .as_array()
                .expect("sections")
                .iter()
                .map(|s| {
                    let facets = s["facets"]
                        .as_array()
                        .map(|fs| {
                            fs.iter()
                                .filter_map(|f| f["id"].as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    (s["id"].as_str().expect("a section id").to_string(), facets)
                })
                .collect();
            let weapons = def["components"]
                .as_array()
                .map(|cs| {
                    cs.iter()
                        .filter(|c| c["role"]["kind"] == "weapon")
                        .filter_map(|c| c["id"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            units.insert(id.clone(), Unit { sections, weapons });
        }
    }
    units
}

#[test]
fn every_shipped_look_parses_and_draws_distinct_units() {
    let mut drawn: BTreeMap<String, &str> = BTreeMap::new();
    for (name, text) in look::shipped_sources() {
        let look = Look::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        for unit in &look.units {
            if let Some(other) = drawn.insert(unit.clone(), name) {
                panic!("{unit} is drawn by both {other} and {name}");
            }
        }
    }
}

#[test]
fn every_unit_type_has_a_look_and_every_look_fits_its_units() {
    let units = shipped_units();
    assert!(units.len() >= 14, "the shipped unit types: {}", units.len());
    for id in units.keys() {
        let lookless = LOOKLESS.iter().any(|(u, _)| u == id);
        let has = look::for_unit(id).is_some();
        assert!(
            has != lookless,
            "{id}: {}",
            if has {
                "has a look and is listed as lookless"
            } else {
                "has no look; give it one, or list it in LOOKLESS with the reason"
            }
        );
    }
    for (u, _) in LOOKLESS {
        assert!(
            units.contains_key(*u),
            "LOOKLESS names {u}, which is not shipped"
        );
    }
    for look in look::shipped() {
        // The reserved `unknown` look draws contacts not yet classified, and
        // names no unit type.
        if look.units.iter().any(|u| u == look::UNKNOWN) {
            assert_eq!(
                look.units,
                [look::UNKNOWN],
                "the unknown look draws nothing else"
            );
            assert!(look.doll.is_none(), "an unknown contact has no doll");
            continue;
        }
        let mine: Vec<&Unit> = look
            .units
            .iter()
            .map(|u| {
                units.get(u).unwrap_or_else(|| {
                    panic!("look {} names {u}, which is not shipped", look.label)
                })
            })
            .collect();
        for (name, unit) in look.units.iter().zip(&mine) {
            if let Some(doll) = &look.doll {
                let drawn = doll.sections();
                for section in unit.sections.keys() {
                    assert!(
                        drawn.contains(&section.as_str()),
                        "{name}: section {section} is not in {}'s doll",
                        look.label
                    );
                }
                for part in &doll.parts {
                    let facets = unit.sections.get(&part.section).unwrap_or_else(|| {
                        panic!(
                            "{name} has no section {} ({}'s doll)",
                            part.section, look.label
                        )
                    });
                    if let Some(facet) = &part.facet {
                        assert!(
                            facets.contains(facet),
                            "{name}: section {} has no facet {facet} ({}'s doll)",
                            part.section,
                            look.label
                        );
                    }
                }
            }
        }
        let muzzles = look
            .sprites
            .iter()
            .flat_map(|sheet| sheet.facings.values())
            .flat_map(|facing| facing.muzzles.keys());
        for weapon in look.weapons.keys().chain(muzzles) {
            assert!(
                mine.iter().any(|u| u.weapons.contains(weapon)),
                "{}: weapon {weapon} is fitted to none of its units",
                look.label
            );
        }
    }
}
