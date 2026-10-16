//! **Doc 18 cut IC2 — the goods table's validator and the capacity function**
//! (doc 18 §4.3 "The validator" and "The capacity function", §4.2
//! "Compartments"; D-18-28 as replaced by R7; doc 18 §3-R corrections (3) and
//! (4) are the arithmetic).
//!
//! Every test reads the shipped `goods_table()` and the Dray's three
//! compartments as the fielded catalog holds them, through
//! `Compartment::from_stowage`. The quantities below test the limits — mass and
//! packed volume binding apart and together — and assert nothing about which
//! load is best (D-18-39: IC8's LP test derives the optimum from the posted
//! prices).

use std::collections::BTreeMap;

use mush_core::CanonicalId;
use tacsim::definitions::content::DRAY;
use tacsim::definitions::goods::{GOODS_JSON, GOODS_PACK, goods_table};
use tacsim::definitions::{Catalog, GoodsTable, LogisticsDef, StowageDef};
use tacsim::port::capacity::{
    Admission, CapacityError, CapacityRefusal, Compartment, admits, format_m3, format_tonnes, load,
    max_more,
};

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn mass() -> CanonicalId {
    cid("si.mass_kg")
}

fn volume() -> CanonicalId {
    cid("si.volume_l")
}

fn table() -> GoodsTable {
    goods_table().expect("the shipped goods table loads")
}

/// The Dray's logistics, as the fielded catalog holds them.
fn dray() -> LogisticsDef {
    Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds")
        .specs
        .get(&cid(DRAY))
        .and_then(|spec| spec.logistics.clone())
        .expect("the Dray's logistics")
}

fn compartment(id: &str) -> Compartment {
    let logistics = dray();
    let def = logistics
        .stowage
        .get(&cid(id))
        .unwrap_or_else(|| panic!("the {id} compartment"));
    Compartment::from_stowage(&cid(id), def)
}

fn contents(rows: &[(&str, u64)]) -> BTreeMap<CanonicalId, u64> {
    rows.iter().map(|(good, qty)| (cid(good), *qty)).collect()
}

fn loaded(kg: u64, litres: u64) -> BTreeMap<CanonicalId, u64> {
    BTreeMap::from([(mass(), kg), (volume(), litres)])
}

/// The goods table with one row's JSON changed, decoded (or refused) by `load`.
fn edited(change: impl Fn(&mut serde_json::Value)) -> Result<GoodsTable, String> {
    let mut doc: serde_json::Value = serde_json::from_str(GOODS_JSON).expect("goods.json");
    change(&mut doc);
    GoodsTable::load("goods.json", &doc.to_string()).map_err(|error| error.to_string())
}

/// The row of `id` in the goods document.
fn row<'a>(doc: &'a mut serde_json::Value, id: &str) -> &'a mut serde_json::Value {
    doc.get_mut("goods")
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|rows| {
            rows.iter_mut()
                .find(|row| row.get("id").and_then(serde_json::Value::as_str) == Some(id))
        })
        .unwrap_or_else(|| panic!("the {id} row"))
}

// ---------------------------------------------------------------------------
// The validator
// ---------------------------------------------------------------------------

/// **A trade good names its family** (doc 18 §4.3).
#[test]
fn ic2_goods_refuses_a_family_less_trade_row() {
    let refused = edited(|doc| row(doc, "metals")["family"] = serde_json::Value::Null)
        .expect_err("a family-less trade row is refused");
    assert!(
        refused.contains("metals: a trade good names its family"),
        "{refused}"
    );
}

/// **A bulk row without packed volume loads, and is refused against the hull**
/// (doc 18 §4.3 "The validator": every good a compartment accepts carries every
/// measure it limits).
#[test]
fn ic2_goods_refuses_a_bulk_row_without_volume() {
    let table = edited(|doc| {
        if let Some(measures) = row(doc, "volatiles")
            .get_mut("measures")
            .and_then(serde_json::Value::as_object_mut)
        {
            measures.remove("si.volume_l");
        }
    })
    .expect("a row without volume is a legal row");
    let refused = table
        .check_stowage(DRAY, &dray())
        .err()
        .map(|error| error.to_string())
        .expect("the Dray's hold limits packed volume");
    assert!(
        refused.contains(
            "volatiles is stowed in cargo, which limits si.volume_l, and declares no si.volume_l"
        ),
        "{refused}"
    );
}

/// **A row names only registered measures.**
#[test]
fn ic2_goods_refuses_an_unregistered_measure() {
    let refused = edited(|doc| {
        if let Some(measures) = row(doc, "metals")
            .get_mut("measures")
            .and_then(serde_json::Value::as_object_mut)
        {
            measures.insert("si.reefer_slots".to_owned(), serde_json::json!(1));
        }
    })
    .expect_err("an unregistered measure is refused");
    assert!(
        refused.contains("measure `si.reefer_slots` is not registered"),
        "{refused}"
    );
}

/// **The locker's two limits agree on crew-days** (doc 18 §4.3: 480 / 4 =
/// 1,200 / 10 = 120); a locker at 480 kg and 1,300 L does not.
#[test]
fn ic2_the_locker_limits_agree_on_crew_days() {
    let table = table();
    table
        .check_stowage(DRAY, &dray())
        .expect("the shipped locker agrees");
    let mut skewed = dray();
    skewed.stowage.insert(
        cid("provisions"),
        StowageDef {
            accepts: [cid("stow.provisions")].into_iter().collect(),
            limits: BTreeMap::from([(mass(), 480), (volume(), 1_300)]),
        },
    );
    let refused = table
        .check_stowage(DRAY, &skewed)
        .err()
        .map(|error| error.to_string())
        .expect("a skewed locker is refused");
    assert!(
        refused.contains(
            "compartment `provisions` holds 120 units of provisions by si.mass_kg and 130 by \
             si.volume_l; the limits agree on the unit count"
        ),
        "{refused}"
    );
}

/// **The market rows derive from the table** (lead ruling R12): six entries,
/// each one the market's own check admits, coded by the good's id, lot-
/// represented, under pack `tacsim.goods`, its baseline the reference in minor
/// units.
#[test]
fn ic2_the_catalog_rows_derive_from_the_goods_table() {
    let table = table();
    let entries = table.catalog_entries().expect("the rows derive");
    assert_eq!(entries.len(), 6);
    for (entry, good) in entries.iter().zip(&table.goods) {
        mush_market::check_entry(entry).unwrap_or_else(|error| panic!("{}: {error:?}", good.id));
        assert_eq!(entry.catalog.code.as_str(), good.id.as_str());
        assert_eq!(entry.catalog.ruleset.as_str(), "tacsim");
        assert_eq!(entry.pack.as_str(), GOODS_PACK);
        assert_eq!(entry.definition_version, table.version);
        assert_eq!(entry.name.as_str(), good.label);
        assert_eq!(entry.representation, mush_market::Representation::Lot);
        assert!(entry.stackable);
        let reference = i64::try_from(good.reference_major).expect("in range");
        assert_eq!(
            entry.baseline.map(|amount| amount.minor()),
            Some(reference * 100),
            "{}",
            good.id
        );
        assert!(
            entry
                .tags
                .iter()
                .any(|tag| tag.as_str() == good.class.as_str())
        );
    }
}

// ---------------------------------------------------------------------------
// The capacity function
// ---------------------------------------------------------------------------

/// **Packed volume binds on ice and the fittings, mass on the dense goods**
/// (doc 18 §3-R corrections (3)/(4)): from an empty hold.
#[test]
fn ic2_volume_binds_on_ice_and_mass_on_alloys() {
    let (goods, cargo, empty) = (table(), compartment("cargo"), contents(&[]));
    let more = |good: &str| max_more(&goods, &cargo, &empty, &cid(good)).expect("a known good");
    assert_eq!(more("volatiles"), (85, vec![volume()]));
    assert_eq!(more("structural_alloys"), (100, vec![mass()]));
    assert_eq!(more("metals"), (100, vec![mass()]));
    assert_eq!(more("industrial_parts"), (52, vec![volume()]));
}

/// **86 t of water ice is refused on packed volume** (doc 18 §4.3: 120.4 m³
/// against 120 m³; 85 t fits).
#[test]
fn ic2_86_t_of_ice_is_refused_on_packed_volume() {
    let (goods, cargo) = (table(), compartment("cargo"));
    assert_eq!(
        admits(&goods, &cargo, &contents(&[]), (&cid("volatiles"), 86)),
        Admission::Refused(CapacityRefusal {
            compartment: cid("cargo"),
            bound: vec![volume()],
            fits: 85,
            room: loaded(100_000, 120_000),
        })
    );
}

/// **19 t of concentrate and 81 t of ice is refused on packed volume alone**:
/// the mass is exactly the rating and not in `bound`.
#[test]
fn ic2_19_plus_81_is_refused_on_packed_volume_alone() {
    let (goods, cargo) = (table(), compartment("cargo"));
    let aboard = contents(&[("metals", 19)]);
    let Admission::Refused(refusal) = admits(&goods, &cargo, &aboard, (&cid("volatiles"), 81))
    else {
        panic!("19 + 81 fitted");
    };
    assert_eq!(
        refusal.bound,
        vec![volume()],
        "mass at exactly 100,000 kg binds nothing"
    );
    assert_eq!(refusal.fits, 80);
    assert_eq!(refusal.room, loaded(81_000, 112_400));
}

/// **101 t of alloy stock is refused on mass** (30.3 m³; 100 t fits).
#[test]
fn ic2_101_t_of_alloys_is_refused_on_mass() {
    let (goods, cargo) = (table(), compartment("cargo"));
    let Admission::Refused(refusal) = admits(
        &goods,
        &cargo,
        &contents(&[]),
        (&cid("structural_alloys"), 101),
    ) else {
        panic!("101 t fitted");
    };
    assert_eq!(refusal.bound, vec![mass()]);
    assert_eq!(refusal.fits, 100);
}

/// **20 t of concentrate and 81 t of ice is refused on both**, and 80 more is
/// where both bind.
#[test]
fn ic2_20_plus_81_is_refused_on_both() {
    let (goods, cargo) = (table(), compartment("cargo"));
    let aboard = contents(&[("metals", 20)]);
    let Admission::Refused(refusal) = admits(&goods, &cargo, &aboard, (&cid("volatiles"), 81))
    else {
        panic!("20 + 81 fitted");
    };
    assert_eq!(refusal.bound, vec![mass(), volume()], "mass before volume");
    assert_eq!(
        max_more(&goods, &cargo, &aboard, &cid("volatiles")).expect("a known good"),
        (80, vec![mass(), volume()])
    );
}

/// **Two loads that meet both limits exactly** are admitted at 100,000 kg and
/// 120,000 L, after which no bulk good fits. (Limits, not a recommended mix.)
#[test]
fn ic2_a_balanced_load_meets_both_limits_exactly() {
    let (goods, cargo) = (table(), compartment("cargo"));
    for (first, second) in [
        (("metals", 20), ("volatiles", 80)),
        (("structural_alloys", 55), ("industrial_parts", 45)),
    ] {
        let aboard = contents(&[first]);
        assert_eq!(
            admits(&goods, &cargo, &aboard, (&cid(second.0), second.1)),
            Admission::Admitted(loaded(100_000, 120_000)),
            "{first:?} + {second:?}"
        );
        let full = contents(&[first, second]);
        assert_eq!(
            load(&goods, &cargo, &full).expect("a load"),
            loaded(100_000, 120_000)
        );
        for good in [
            "metals",
            "volatiles",
            "structural_alloys",
            "industrial_parts",
        ] {
            assert_eq!(
                max_more(&goods, &cargo, &full, &cid(good))
                    .expect("a known good")
                    .0,
                0,
                "{good} after {first:?} + {second:?}"
            );
        }
    }
}

/// **No more ice fits by packed volume** once 19 t of concentrate and 80 t of ice
/// are aboard (1 t of room by mass, 0.4 m³ by volume).
#[test]
fn ic2_no_more_ice_fits_by_packed_volume() {
    let (goods, cargo) = (table(), compartment("cargo"));
    let aboard = contents(&[("metals", 19)]);
    assert!(matches!(
        admits(&goods, &cargo, &aboard, (&cid("volatiles"), 80)),
        Admission::Admitted(_)
    ));
    let full = contents(&[("metals", 19), ("volatiles", 80)]);
    assert_eq!(
        max_more(&goods, &cargo, &full, &cid("volatiles")).expect("a known good"),
        (0, vec![volume()])
    );
}

/// **The tank limits mass alone; the locker binds on both at once.**
#[test]
fn ic2_the_tank_and_the_locker() {
    let goods = table();
    let (tank, locker, empty) = (
        compartment("propellant"),
        compartment("provisions"),
        contents(&[]),
    );
    assert!(matches!(
        admits(&goods, &tank, &empty, (&cid("propellant"), 80)),
        Admission::Admitted(_)
    ));
    let Admission::Refused(refusal) = admits(&goods, &tank, &empty, (&cid("propellant"), 81))
    else {
        panic!("81 t of reaction mass fitted the tank");
    };
    assert_eq!(refusal.bound, vec![mass()], "the tank limits mass only");
    assert!(matches!(
        admits(&goods, &locker, &empty, (&cid("provisions"), 120)),
        Admission::Admitted(_)
    ));
    let Admission::Refused(refusal) = admits(&goods, &locker, &empty, (&cid("provisions"), 121))
    else {
        panic!("121 crew-days fitted the locker");
    };
    assert_eq!(refusal.bound, vec![mass(), volume()]);
    assert_eq!(refusal.fits, 120);
}

/// **A malformed question is an error, never a refusal.**
#[test]
fn ic2_capacity_errors_are_not_refusals() {
    let (goods, cargo, empty) = (table(), compartment("cargo"), contents(&[]));
    assert_eq!(
        admits(&goods, &cargo, &empty, (&cid("propellant"), 1)),
        Admission::Error(CapacityError::NotAccepted {
            good: cid("propellant"),
            compartment: cid("cargo"),
        })
    );
    assert_eq!(
        admits(&goods, &cargo, &empty, (&cid("munitions"), 1)),
        Admission::Error(CapacityError::UnknownGood {
            good: cid("munitions"),
        })
    );
    let thin = edited(|doc| {
        if let Some(measures) = row(doc, "volatiles")
            .get_mut("measures")
            .and_then(serde_json::Value::as_object_mut)
        {
            measures.remove("si.volume_l");
        }
    })
    .expect("a row without volume loads");
    assert_eq!(
        admits(&thin, &cargo, &empty, (&cid("volatiles"), 1)),
        Admission::Error(CapacityError::MissingMeasure {
            good: cid("volatiles"),
            measure: volume(),
        })
    );
    assert_eq!(
        admits(&goods, &cargo, &empty, (&cid("metals"), u64::MAX)),
        Admission::Error(CapacityError::Overflow)
    );
}

/// **Cubic metres to a tenth, truncated; tonnes whole, truncated** (doc 18 §4.3
/// UI rule).
#[test]
fn ic2_the_m3_formatter_truncates() {
    assert_eq!(format_m3(120_400), "120.4");
    assert_eq!(format_m3(118_700), "118.7");
    assert_eq!(format_m3(30_399), "30.3");
    assert_eq!(format_m3(99), "0.0");
    assert_eq!(format_m3(0), "0.0");
    assert_eq!(format_tonnes(100_999), "100");
}
