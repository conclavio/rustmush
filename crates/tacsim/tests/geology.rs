//! **Doc 20 cut SM.1 acceptance — the ground** (doc 20 §3.2, §5, §11's Geology and Instruments
//! rows, D-20-2, D-20-4, Q-20-10, Q-20-12, §16; the SM.1 brief's lead rulings 1–6 and decisions
//! 1–15).
//!
//! The keyed geology function, its table and lattice, the reading rule, the site geology table
//! and composite cache, and the key's boot: the first keyed boot pins the fingerprint and every
//! later boot compares it; every disabled reason reads on the readiness line and never gates the
//! trade run's desk; `@geology` answers staff with the truth and nobody else.
//!
//! The tables here are **test data only** (lead ruling 1): they reproduce doc 20 §3.2's sentences
//! for Ceres and Vesta; SM.2 authors the lane's. The engine tests compose the production roster
//! (`tacsim::composition`, the shape `mush_server::boot::game_modules` builds; `tests/hulls.rs`'s
//! rig) and hand the key in before the module boot pass, as `build_engine` does.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use mush_atlas::geom::{Point2, Rect2};
use mush_atlas::record::{BackgroundRecipe, RegionKind, TileOwner};
use mush_atlas::window::{CompositeInput, WindowFrame, capture};
use mush_atlas::{
    Access, Cell, Cells, Footprint, PointCm, RecordKey, Records, RectCm, RegionRec, RoomGeo,
    RoomKind, SurfaceRef, TerrainClass, TileRec,
};
use mush_core::{CanonicalId, GOD, GeologyMac, ModuleBootData, ModuleWrite, ObjId};
use mush_testkit::{TestWorld, fold_module_ops};
use smol_str::SmolStr;
use tacsim::port::{Binding, LineState, ReadinessLine};
use tacsim::surface::geology::{
    BodyGeology, FieldGround, GeologyError, GeologyTable, GeologyTableWire, Lattice, OreFamily,
    POCKETS_PER_FIELD_SIDE, PROVINCE_COLUMNS, PROVINCE_ROWS, Word, field_ground, pocket_at,
    province_at,
};
use tacsim::surface::ground::{
    SITE_CACHE_CAPACITY, SiteGeologySpec, SiteGround, SiteGroundCache, SiteGroundKey,
    build_site_ground, site_geology, site_ground_sync, site_revision,
};
use tacsim::surface::reading::{
    FieldReading, Footing, READING_RULE_VERSION, read_r0, read_r1, read_r2,
};
use tacsim::surface::{GEOLOGY_PIN_KEY, GeologyBoot, GeologyDisabled, GeologyStatus};

#[path = "support/geology_keyed.rs"]
mod geology_keyed;
use geology_keyed::{
    CERES_RADIUS_KM, CERES_TABLE, KEY_1, KEY_2, VESTA_RADIUS_KM, VESTA_TABLE, boot_keyed, mac,
    secret,
};

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn wire(json: &str) -> GeologyTableWire {
    serde_json::from_str(json).expect("the test table parses")
}

fn table(json: &str) -> GeologyTable {
    GeologyTable::from_wire(&wire(json)).expect("the test table checks")
}

fn ceres() -> BodyGeology {
    BodyGeology::new(cid("ceres"), CERES_RADIUS_KM, table(CERES_TABLE)).expect("ceres")
}

fn vesta() -> BodyGeology {
    BodyGeology::new(cid("vesta"), VESTA_RADIUS_KM, table(VESTA_TABLE)).expect("vesta")
}

fn by<T: Copy>(values: [T; 4], family: OreFamily) -> T {
    values
        .get(usize::from(family.code()))
        .copied()
        .expect("four families")
}

fn base16(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// 1. The table, the lattice and the provinces
// ---------------------------------------------------------------------------

/// The wire refuses every table it cannot honour, each with its own variant (an unknown key at
/// any level is refused by the strict wire itself); the two test tables round-trip.
#[test]
fn sm1_the_table_wire_refuses_what_it_cannot_honour() {
    for json in [CERES_TABLE, VESTA_TABLE] {
        let w = wire(json);
        let t = GeologyTable::from_wire(&w).expect("checks");
        assert_eq!(t.wire(), w, "the wire round-trips");
        assert_eq!(GeologyTable::from_wire(&t.wire()), Ok(t));
    }
    let edit = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut value: serde_json::Value = serde_json::from_str(CERES_TABLE).expect("json");
        f(&mut value);
        value
    };
    let check = |value: serde_json::Value| {
        GeologyTable::from_wire(&serde_json::from_value(value).expect("the wire parses"))
    };
    // A row summing to 999.
    let short = edit(&|v| v["province"]["metals"]["fair"] = 399.into());
    assert_eq!(
        check(short),
        Err(GeologyError::RowSum {
            row: "province metals".to_owned(),
            sum: 999
        })
    );
    // A missing family.
    let no_family = edit(&|v| {
        v["province"]
            .as_object_mut()
            .expect("map")
            .remove("strategic_elements");
    });
    assert_eq!(
        check(no_family),
        Err(GeologyError::MissingProvinceRow("strategic_elements"))
    );
    // A missing field row.
    let no_field = edit(&|v| {
        v["field"].as_object_mut().expect("map").remove("fair");
    });
    assert_eq!(check(no_field), Err(GeologyError::MissingFieldRow("fair")));
    // A barren province's field row weighting ore (decision 13: a province barren in a family
    // has only barren fields in it, for any table — not only under the shift guard).
    let ore_in_barren = edit(&|v| {
        v["field"]["barren"] = serde_json::json!({ "barren": 950, "lean": 50 });
    });
    assert_eq!(
        check(ore_in_barren),
        Err(GeologyError::BarrenProvinceField("lean"))
    );
    // A missing pocket row.
    let no_pocket = edit(&|v| {
        v["pocket"].as_object_mut().expect("map").remove("rich");
    });
    assert_eq!(
        check(no_pocket),
        Err(GeologyError::MissingPocketRow("rich"))
    );
    // A barren pocket row, and a barren weight inside a pocket row.
    let barren_row = edit(&|v| v["pocket"]["barren"] = serde_json::json!({ "lean": 1000 }));
    assert_eq!(check(barren_row), Err(GeologyError::BarrenPocketRow));
    let barren_class = edit(&|v| {
        v["pocket"]["lean"] = serde_json::json!({ "barren": 300, "lean": 700 });
    });
    assert_eq!(
        check(barren_class),
        Err(GeologyError::BarrenPocketClass("lean"))
    );
    // An unknown key at each level is refused by the strict wire.
    for unknown in [
        edit(&|v| v["seam"] = 1.into()),
        edit(&|v| v["province"]["iron"] = serde_json::json!({ "lean": 1000 })),
        edit(&|v| v["field"]["epic"] = serde_json::json!({ "lean": 1000 })),
        edit(&|v| v["pocket"]["lean"]["epic"] = 1.into()),
    ] {
        assert!(
            serde_json::from_value::<GeologyTableWire>(unknown).is_err(),
            "an unknown key is refused"
        );
    }
    assert_eq!(
        BodyGeology::new(cid("ceres"), 0, table(CERES_TABLE)),
        Err(GeologyError::Radius(0))
    );
}

/// The lattice is pinned: Ceres 73,824 × 36,912 pockets, Vesta 41,304 × 20,640; x wraps, a pole
/// is off it.
#[test]
fn sm1_the_lattice_is_pinned() {
    assert_eq!(
        ceres().lattice(),
        Lattice {
            columns: 73_824,
            rows: 36_912
        }
    );
    assert_eq!(
        vesta().lattice(),
        Lattice {
            columns: 41_304,
            rows: 20_640
        }
    );
    assert_eq!(
        POCKETS_PER_FIELD_SIDE, 8,
        "a field is 8 x 8 pockets of 40 m"
    );
    let k = mac(KEY_1);
    for body in [ceres(), vesta()] {
        let lattice = body.lattice();
        assert_eq!(lattice.columns % 8, 0);
        assert_eq!(lattice.rows % 16, 0);
        let fields = lattice.columns / 8;
        let here = field_ground(&k, &body, (17, -3), 0).expect("on the lattice");
        let wrapped = field_ground(&k, &body, (17 + fields, -3), 0).expect("wrapped");
        let back = field_ground(&k, &body, (17 - fields, -3), 0).expect("wrapped west");
        assert_eq!(here, wrapped, "x wraps: the seam is invisible");
        assert_eq!(here, back);
        let (_, pocket) = pocket_at(&k, &body, (5, 0)).expect("a pocket");
        let (_, pocket_wrapped) = pocket_at(&k, &body, (5 + lattice.columns, 0)).expect("wrapped");
        assert_eq!(pocket, pocket_wrapped);
        assert_eq!(
            pocket_at(&k, &body, (0, lattice.rows / 2)),
            Err(GeologyError::OffLattice),
            "y = rows/2 is off the lattice"
        );
        assert!(pocket_at(&k, &body, (0, lattice.rows / 2 - 1)).is_ok());
        assert!(pocket_at(&k, &body, (0, -lattice.rows / 2)).is_ok());
        assert_eq!(
            pocket_at(&k, &body, (0, -lattice.rows / 2 - 1)),
            Err(GeologyError::OffLattice)
        );
        assert_eq!(
            field_ground(&k, &body, (0, 0), 4),
            Err(GeologyError::Shift(4))
        );
    }
}

/// Over all 1,568 provinces of each body under the fixed key, the province words say what doc 20
/// §3.2 says of Ceres and Vesta.
#[test]
fn sm1_the_province_words_say_what_section_3_2_says() {
    let k = mac(KEY_1);
    let all = |body: &BodyGeology| -> Vec<[Word; 4]> {
        (0..PROVINCE_COLUMNS)
            .flat_map(|c| (0..PROVINCE_ROWS).map(move |r| (c, r)))
            .map(|p| province_at(&k, body, p).expect("a province").words)
            .collect()
    };
    let ceres = all(&ceres());
    let vesta = all(&vesta());
    assert_eq!(ceres.len(), 1_568);
    assert_eq!(vesta.len(), 1_568);
    let count = |rows: &[[Word; 4]], family: OreFamily, pred: &dyn Fn(Word) -> bool| {
        rows.iter().filter(|w| pred(by(**w, family))).count()
    };
    // Ceres: volatiles fair or rich in ≥ 90 % and never barren; metals only lean or fair;
    // carbon always lean; strategic barren in ≥ 90 % and lean at least once.
    assert!(
        count(&ceres, OreFamily::Volatiles, &|w| matches!(
            w,
            Word::Fair | Word::Rich
        )) * 10
            >= 1_568 * 9
    );
    assert_eq!(
        count(&ceres, OreFamily::Volatiles, &|w| w == Word::Barren),
        0
    );
    assert_eq!(
        count(&ceres, OreFamily::Metals, &|w| matches!(
            w,
            Word::Lean | Word::Fair
        )),
        1_568
    );
    assert_eq!(
        count(&ceres, OreFamily::CarbonFeedstock, &|w| w == Word::Lean),
        1_568
    );
    assert!(count(&ceres, OreFamily::StrategicElements, &|w| w == Word::Barren) * 10 >= 1_568 * 9);
    assert!(count(&ceres, OreFamily::StrategicElements, &|w| w == Word::Lean) >= 1);
    // Vesta: metals only fair or rich; strategic only lean or fair; volatiles always lean;
    // carbon always barren.
    assert_eq!(
        count(&vesta, OreFamily::Metals, &|w| matches!(
            w,
            Word::Fair | Word::Rich
        )),
        1_568
    );
    assert_eq!(
        count(&vesta, OreFamily::StrategicElements, &|w| matches!(
            w,
            Word::Lean | Word::Fair
        )),
        1_568
    );
    assert_eq!(
        count(&vesta, OreFamily::Volatiles, &|w| w == Word::Lean),
        1_568
    );
    assert_eq!(
        count(&vesta, OreFamily::CarbonFeedstock, &|w| w == Word::Barren),
        1_568
    );
}

// ---------------------------------------------------------------------------
// 2. The acceptance property: a million pockets
// ---------------------------------------------------------------------------

/// Everything the honest-summary property says of one field, asserted.
fn assert_field_honest(field: &FieldGround) {
    assert_eq!(field.pockets.len(), 64);
    for family in OreFamily::ALL {
        let ore = field
            .pockets
            .iter()
            .filter(|p| by(p.classes, family) != Word::Barren)
            .count();
        let count = by(field.counts, family);
        assert_eq!(ore, usize::from(count), "{family:?} in {:?}", field.field);
        assert!(count <= 16);
        assert_eq!(Word::of_count(count), Some(by(field.words, family)));
        // Decision 13: a province-barren family stays barren under any shift.
        if by(field.province.words, family) == Word::Barren {
            assert_eq!(by(field.words, family), Word::Barren, "{family:?}");
            assert_eq!(count, 0);
        }
    }
    for pocket in &field.pockets {
        assert!(pocket.families() <= 2, "at most two families a pocket");
        for family in OreFamily::ALL {
            let class = by(pocket.classes, family);
            let tonnes = by(pocket.tonnes, family);
            assert_eq!(class == Word::Barren, tonnes == 0, "barren ⇔ 0 t");
            let (lo, hi) = class.tonnes_range();
            assert!((lo..=hi).contains(&tonnes), "{class:?} {tonnes} t");
        }
    }
}

/// **The acceptance property** (doc 20 §13's SM.1 cell; lead ruling 5): ≥ 10⁶ pocket
/// evaluations, every pocket of every sampled field, fields grouped into 2 × 2 blocks. Block `k`
/// of 3,907 is on Ceres for even `k` and Vesta for odd; its south-west field is
/// `(2·389k, 2·((97k) mod R) − R)` with `R = rows / 16` (a stride that visits every longitude
/// band and both hemispheres), under the shift `k mod 7 − 3` (all seven appear).
#[test]
fn sm1_a_million_pockets_every_field_word_is_honest() {
    let started = Instant::now();
    let k = mac(KEY_1);
    let bodies = [ceres(), vesta()];
    let mut pockets = 0_usize;
    let mut shifts = BTreeSet::new();
    let mut hemispheres = BTreeSet::new();
    for block in 0..3_907_i64 {
        let body = &bodies[usize::try_from(block % 2).expect("0 or 1")];
        let half = body.lattice().rows / 16;
        let sw = (2 * (block * 389), 2 * ((block * 97) % half) - half);
        let shift = i8::try_from(block % 7).unwrap_or(0) - 3;
        shifts.insert(shift);
        hemispheres.insert(sw.1 >= 0);
        let mut block_counts = [0_u32; 4];
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let field = field_ground(&k, body, (sw.0 + dx, sw.1 + dy), shift).expect("a field");
            assert_eq!(field.shift, shift);
            assert_field_honest(&field);
            pockets += field.pockets.len();
            for (total, count) in block_counts.iter_mut().zip(field.counts) {
                *total += u32::from(count);
            }
        }
        for total in block_counts {
            assert!(total <= 64, "at most 64 ore pockets a block a family");
        }
    }
    assert!(pockets >= 1_000_000, "{pockets} pocket evaluations");
    assert_eq!(shifts.len(), 7);
    assert_eq!(hemispheres.len(), 2);
    eprintln!(
        "sm1_a_million_pockets_every_field_word_is_honest: {pockets} pockets in {:.2?}",
        started.elapsed()
    );
}

/// The same key gives the same ground; another key, another.
#[test]
fn sm1_the_same_key_gives_the_same_ground_and_another_key_another() {
    let (one, two) = (mac(KEY_1), mac(KEY_2));
    let body = ceres();
    let a = field_ground(&one, &body, (150, -43), 1).expect("a field");
    let b = field_ground(&one, &body, (150, -43), 1).expect("a field");
    assert_eq!(a, b);
    let differs = (0..64_i64)
        .filter(|i| {
            let field = (i * 113, i * 7 - 200);
            field_ground(&one, &body, field, 0).expect("one")
                != field_ground(&two, &body, field, 0).expect("two")
        })
        .count();
    assert!(differs >= 1, "another key is another ground");
}

/// The reading rule quantises without lying, over 10,000 fields.
#[test]
fn sm1_the_reading_rule_quantises_without_lying() {
    assert_eq!(READING_RULE_VERSION, 1);
    let k = mac(KEY_1);
    let body = ceres();
    let mut bands = BTreeSet::new();
    for i in 0..10_000_i64 {
        let field =
            field_ground(&k, &body, (i * 37, (i * 13) % 4_000 - 2_000), 0).expect("a field");
        let rock = read_r1(&k, &body, &field, Footing::Rock);
        let regolith = read_r1(&k, &body, &field, Footing::Regolith);
        let again = read_r1(&k, &body, &field, Footing::Regolith);
        let ice = read_r1(&k, &body, &field, Footing::Ice);
        assert_eq!(regolith, again, "a repeated scan never narrows");
        for family in OreFamily::ALL {
            let truth = by(field.words, family);
            assert_eq!(by(rock, family), FieldReading::Exact(truth));
            assert_eq!(by(ice, family), FieldReading::NoReturn);
            let FieldReading::Band(low, high) = by(regolith, family) else {
                panic!("regolith reads a band");
            };
            assert_eq!(high.code(), low.code() + 1, "two adjacent words");
            assert!(low == truth || high == truth, "the band holds the truth");
            match truth {
                Word::Barren => assert_eq!((low, high), (Word::Barren, Word::Lean)),
                Word::Rich => assert_eq!((low, high), (Word::Fair, Word::Rich)),
                Word::Lean | Word::Fair => {
                    bands.insert(low == truth);
                }
            }
            let (lo, hi) = by(regolith, family).pockets().expect("a range");
            assert!((lo..=hi).contains(&by(field.counts, family)));
        }
        assert_eq!(read_r0(&field.province), field.province.words);
        for pocket in &field.pockets {
            assert_eq!(read_r2(pocket), pocket.classes);
        }
    }
    assert_eq!(
        bands.len(),
        2,
        "the band's side is not the truth's position"
    );
    assert_eq!(
        FieldReading::Band(Word::Fair, Word::Rich).words(),
        "fair–rich"
    );
    assert_eq!(FieldReading::Exact(Word::Fair).words(), "fair");
    assert_eq!(FieldReading::NoReturn.words(), "no return");
    assert_eq!(Footing::of(TerrainClass::Regolith), Some(Footing::Regolith));
    assert_eq!(Footing::of(TerrainClass::Paved), None);
}

// ---------------------------------------------------------------------------
// 3. The site
// ---------------------------------------------------------------------------

/// The pin test's region: surface on ceres, 2,560 m square, regolith with ice and rock.
fn site_region() -> RegionRec {
    RegionRec {
        key: RecordKey::new(cid("sm1"), cid("site")),
        name: "SM.1 test site".to_owned(),
        kind: RegionKind::Surface {
            body: "ceres".to_owned(),
        },
        frame: None,
        parent: None,
        extent: Rect2(Point2(0.0, 0.0), Point2(2_560.0, 2_560.0)),
        datum_cm: 0,
        background: BackgroundRecipe {
            surface: TerrainClass::Regolith,
            mix: vec![(TerrainClass::Ice, 20), (TerrainClass::Rock, 15)],
            relief_cm: 150,
            relief_period_cm: 1_000,
            water_cm: None,
            seed: 2026,
        },
        ground: Vec::new(),
        ground_resolution_cm: 1_000,
        theme: None,
        revision: 1,
    }
}

fn site_frame(region: &RegionRec) -> WindowFrame {
    WindowFrame {
        region: region.key.clone(),
        surfaces: vec![SurfaceRef::Ground],
        bbox: RectCm {
            min: PointCm { x: 0, y: 0 },
            max: PointCm {
                x: 256_000,
                y: 256_000,
            },
        },
        cell_cm: 1_000,
    }
}

/// The pinned spec: block 0 +1, block 15 −1, the rest 0.
fn site_spec() -> SiteGeologySpec {
    let mut shifts = [0_i8; 16];
    shifts[0] = 1;
    shifts[15] = -1;
    SiteGeologySpec {
        body: cid("ceres"),
        origin: (8_000, -1_600),
        shifts,
    }
}

fn site_input() -> CompositeInput {
    let region = site_region();
    let frame = site_frame(&region);
    let mut records = Records::default();
    records.regions.insert(region.key.clone(), region);
    capture(&records, &frame).expect("the capture")
}

fn site_ground() -> SiteGround {
    build_site_ground(&site_input(), &site_spec(), &ceres(), &mac(KEY_1), 1).expect("the ground")
}

/// **The acceptance pin**: a fixed key gives pinned digests of a 256 × 256 site.
#[test]
fn sm1_a_fixed_key_pins_a_whole_site() {
    let ground = site_ground();
    let surface = ground
        .composite
        .surface(&SurfaceRef::Ground)
        .expect("the ground surface");
    assert_eq!((surface.cells.width(), surface.cells.height()), (256, 256));
    let geology = &ground.geology;
    let totals = geology.totals();
    let digest = geology.digest();
    assert_eq!(
        (digest.as_str(), ground.composite.digest.as_str(), totals),
        (
            "67bf968c2b5fbe28d1ee2eb55227b88620efb73a194b8bf194e8f46697459eeb",
            "c6a2ff3e4b3d5b2ed937effe04ed3b82598f90ca2d072f13c05a6af7c803eff6",
            [(326, 2_979), (412, 3_945), (172, 1_325), (0, 0)]
        ),
        "the whole-site pins (totals as (ore pockets, tonnes) for metals, volatiles, carbon \
         feedstock, strategic elements: {totals:?}; geology {digest}; composite {})",
        ground.composite.digest
    );
    // Every block's per-family count ≤ 64 and every field's word the honest summary of the
    // SITE TABLE's own pockets in it (counted here, not taken from the field record).
    for by_ in 0..4_i64 {
        for bx in 0..4_i64 {
            let mut block = [0_u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (fx, fy) = (bx * 2 + dx, by_ * 2 + dy);
                let (words, counts) = geology.field(fx, fy).expect("a site field");
                let mut ore = [0_u8; 4];
                for ly in 0..8_i64 {
                    for lx in 0..8_i64 {
                        let pocket = geology
                            .pocket(fx * 8 + lx, fy * 8 + ly)
                            .expect("a site pocket");
                        assert!(pocket.families() <= 2, "at most two families a pocket");
                        for (family, slot) in OreFamily::ALL.into_iter().zip(ore.iter_mut()) {
                            let class = by(pocket.classes, family);
                            let tonnes = by(pocket.tonnes, family);
                            assert_eq!(class == Word::Barren, tonnes == 0, "barren ⇔ 0 t");
                            let (lo, hi) = class.tonnes_range();
                            assert!((lo..=hi).contains(&tonnes), "{class:?} {tonnes} t");
                            if class != Word::Barren {
                                *slot += 1;
                            }
                        }
                    }
                }
                assert_eq!(
                    ore, counts,
                    "field {fx},{fy}: its count is its site pockets'"
                );
                for family in OreFamily::ALL {
                    assert_eq!(Word::of_count(by(counts, family)), Some(by(words, family)));
                }
                for (total, count) in block.iter_mut().zip(counts) {
                    *total += u32::from(count);
                }
            }
            assert!(block.iter().all(|t| *t <= 64));
        }
    }
}

/// The site table is the body function: unshifted blocks equal `pocket_at` on the body lattice,
/// block 0's fields equal `field_ground(.., +1)`, and a province-barren family stays barren
/// under the +1 block (decision 13).
#[test]
fn sm1_the_site_table_is_the_body_function() {
    let ground = site_ground();
    let geology = &ground.geology;
    let k = mac(KEY_1);
    let body = ceres();
    let spec = site_spec();
    let field_origin = (spec.origin.0 / 8, spec.origin.1 / 8);
    let mut barren_pairs = 0;
    for fy in 0..8_i64 {
        for fx in 0..8_i64 {
            let block = usize::try_from((fy / 2) * 4 + fx / 2).expect("a block");
            let shift = spec.shifts[block];
            let truth = field_ground(&k, &body, (field_origin.0 + fx, field_origin.1 + fy), shift)
                .expect("a field");
            assert_eq!(
                geology.field(fx, fy),
                Some((truth.words, truth.counts)),
                "field {fx},{fy}"
            );
            for ly in 0..8_i64 {
                for lx in 0..8_i64 {
                    let (px, py) = (fx * 8 + lx, fy * 8 + ly);
                    let site = geology.pocket(px, py).expect("a site pocket");
                    assert_eq!(Some(site), truth.pocket(lx, ly));
                    if shift == 0 {
                        let (_, body_pocket) =
                            pocket_at(&k, &body, (spec.origin.0 + px, spec.origin.1 + py))
                                .expect("a body pocket");
                        assert_eq!(site, body_pocket, "pocket {px},{py}");
                    }
                }
            }
            if block == 0 {
                for family in OreFamily::ALL {
                    if by(truth.province.words, family) == Word::Barren {
                        barren_pairs += 1;
                        assert_eq!(by(truth.words, family), Word::Barren);
                        assert_eq!(by(truth.counts, family), 0);
                        assert!(
                            truth
                                .pockets
                                .iter()
                                .all(|p| by(p.classes, family) == Word::Barren)
                        );
                    }
                }
            }
        }
    }
    assert!(
        barren_pairs >= 1,
        "the pinned origin's province is barren in some family under the +1 block"
    );
}

/// The site key moves with every input `composite()` or `site_geology()` reads.
#[test]
fn sm1_the_site_key_moves_with_every_input() {
    let base = site_input();
    let spec = site_spec();
    let region = base.region.key.clone();
    let room = RoomGeo {
        room: ObjId(42),
        key: RecordKey::new(cid("sm1"), cid("room")),
        neighborhood: RecordKey::new(cid("sm1"), cid("nb")),
        site: None,
        surface: SurfaceRef::Ground,
        footprint: Footprint::Rect(Rect2(Point2(10.0, 10.0), Point2(20.0, 20.0))),
        kind: RoomKind::Outdoor,
        access: Access::Public,
        revision: 1,
    };
    let tile = TileRec {
        key: RecordKey::new(cid("sm1"), cid("site.tile.1")),
        region,
        surface: SurfaceRef::Ground,
        owner: TileOwner::Region,
        bbox: Rect2(Point2(0.0, 0.0), Point2(10.0, 10.0)),
        resolution_cm: 1_000,
        cells: Cells::filled(
            1,
            1,
            Cell {
                class: TerrainClass::Rock,
                elev_cm: 0,
                cover: 0,
                room: 0,
            },
            16,
        )
        .expect("one cell"),
        rooms: Vec::new(),
        revision: 1,
    };
    let mut with_room = base.clone();
    with_room.rooms.push(room.clone());
    let mut moved_room = base.clone();
    moved_room.rooms.push(RoomGeo {
        footprint: Footprint::Rect(Rect2(Point2(30.0, 10.0), Point2(40.0, 20.0))),
        ..room
    });
    let mut shrunk = base.clone();
    shrunk.frame.bbox.max.x -= 1_000;
    let mut with_tile = base.clone();
    with_tile.tiles.push(tile.clone());
    let mut bumped_tile = base.clone();
    bumped_tile.tiles.push(TileRec {
        revision: 2,
        ..tile
    });
    let mut moved_origin = spec.clone();
    moved_origin.origin.0 += 8;
    let mut reshifted = spec.clone();
    reshifted.shifts[5] = 2;

    let rev = |input: &CompositeInput, spec: &SiteGeologySpec| {
        site_revision(input, spec).expect("a revision")
    };
    let first = rev(&base, &spec);
    assert_eq!(first, rev(&base, &spec), "the unchanged input twice");
    let all = [
        first,
        rev(&with_room, &spec),
        rev(&moved_room, &spec),
        rev(&shrunk, &spec),
        rev(&with_tile, &spec),
        rev(&bumped_tile, &spec),
        rev(&base, &moved_origin),
        rev(&base, &reshifted),
    ];
    let distinct: BTreeSet<u64> = all.iter().copied().collect();
    assert_eq!(distinct.len(), all.len(), "{all:?}");
}

/// The cache holds sixteen and evicts the least recent.
#[test]
fn sm1_the_site_cache_holds_sixteen_and_evicts_the_least_recent() {
    let ground = site_ground();
    let region = RecordKey::new(cid("sm1"), cid("site"));
    let key = |revision: u64, key_version: u32| SiteGroundKey {
        region: region.clone(),
        revision,
        key_version,
    };
    let mut cache = SiteGroundCache::default();
    let first = cache.insert(key(0, 1), ground.clone());
    cache.insert(key(1, 1), ground.clone());
    for revision in 2..16 {
        cache.insert(key(revision, 1), ground.clone());
    }
    assert_eq!(cache.len(), SITE_CACHE_CAPACITY);
    // A hit returns the same Arc, and touches key 0 so it outlives key 1.
    let hit = cache.get(&key(0, 1)).expect("a hit");
    assert!(Arc::ptr_eq(&hit, &first));
    cache.insert(key(16, 1), ground.clone());
    assert_eq!(cache.len(), 16);
    assert!(cache.get(&key(1, 1)).is_none(), "the least recent is gone");
    assert!(cache.get(&key(0, 1)).is_some(), "the touched one survives");
    assert!(cache.get(&key(16, 1)).is_some());
    assert!(
        cache.get(&key(0, 2)).is_none(),
        "another key version is another entry"
    );
}

/// **The revision is cached beside the ground** (doc 20 cut SM.4a's pre-commit round, item 1;
/// the SM.4a brief's K14 STOP): an input no entry was built over reads its full
/// `site_revision`; once an entry is inserted with its capture, the same input (and spec, under
/// the same key version) reads **the entry's** revision — inserted here under a marker no hash
/// gives, so the hit proves the warm path neither serialises nor hashes — and the warm peek is
/// that entry's `Arc`; another key version, another spec or a changed input reads its own full
/// revision (a new one) and no warm hit; an entry inserted without its capture is found by its
/// key alone.
#[test]
fn sm4a_the_site_cache_keeps_the_revision_beside_the_ground() {
    let input = site_input();
    let spec = site_spec();
    let ground = site_ground();
    let full = site_revision(&input, &spec).expect("a revision");
    let mut cache = SiteGroundCache::default();
    assert_eq!(
        cache.revision_of(&input, &spec, 1),
        Ok(full),
        "not seen yet"
    );
    assert!(cache.peek_captured(&input, &spec, 1).is_none());
    let marker = full ^ 0x5a5a_5a5a;
    let key = SiteGroundKey {
        region: input.frame.region.clone(),
        revision: marker,
        key_version: 1,
    };
    let held = cache.insert_captured(key.clone(), ground.clone(), input.clone(), spec.clone());
    assert_eq!(
        cache.revision_of(&input, &spec, 1),
        Ok(marker),
        "the unchanged input reads the cached revision, never a re-hash"
    );
    let hit = cache
        .peek_captured(&input, &spec, 1)
        .expect("the warm peek hits");
    assert!(Arc::ptr_eq(&hit, &held));
    assert_eq!(cache.revision_of(&input.clone(), &spec, 1), Ok(marker));
    // Another key version or spec: not this entry.
    assert_eq!(cache.revision_of(&input, &spec, 2), Ok(full));
    assert!(cache.peek_captured(&input, &spec, 2).is_none());
    let mut moved = spec.clone();
    moved.origin.0 += 8;
    assert_eq!(
        cache.revision_of(&input, &moved, 1),
        site_revision(&input, &moved)
    );
    assert!(cache.peek_captured(&input, &moved, 1).is_none());
    // A changed input (the window shrunk, a region record bumped): a new revision, no hit.
    let mut shrunk = input.clone();
    shrunk.frame.bbox.max.x -= 1_000;
    let mut bumped = input.clone();
    bumped.region.revision += 1;
    for changed in [shrunk, bumped] {
        let new = site_revision(&changed, &spec).expect("a revision");
        assert_ne!(new, full);
        assert_eq!(cache.revision_of(&changed, &spec, 1), Ok(new));
        assert!(cache.peek_captured(&changed, &spec, 1).is_none());
    }
    // An entry inserted without its capture: only its key finds it.
    let mut plain = SiteGroundCache::default();
    let plain_key = SiteGroundKey {
        revision: full,
        ..key
    };
    plain.insert(plain_key.clone(), ground);
    assert!(plain.peek_captured(&input, &spec, 1).is_none());
    assert_eq!(plain.revision_of(&input, &spec, 1), Ok(full));
    assert!(plain.peek(&plain_key).is_some());
}

/// The site table refuses what is not a site: an origin off a field corner, a shift past ±3, a
/// spec for another body, rows past either pole (the boundaries exact); and the build refuses a
/// window that is not 256 × 256 cells of 10 m.
#[test]
fn sm1_the_site_table_refuses_what_is_not_a_site() {
    let k = mac(KEY_1);
    let body = ceres();
    let north = body.lattice().rows / 2;
    let south = -north;
    let with = |f: &dyn Fn(&mut SiteGeologySpec)| {
        let mut spec = site_spec();
        f(&mut spec);
        site_geology(&k, 1, &body, &spec)
    };
    assert!(with(&|_| ()).is_ok(), "the pinned spec builds");
    assert_eq!(
        with(&|s| s.origin = (8_001, -1_600)).err(),
        Some(GeologyError::Origin)
    );
    assert_eq!(
        with(&|s| s.origin = (8_000, -1_596)).err(),
        Some(GeologyError::Origin)
    );
    assert_eq!(
        with(&|s| s.shifts[3] = 4).err(),
        Some(GeologyError::Shift(4))
    );
    assert_eq!(
        with(&|s| s.shifts[12] = -4).err(),
        Some(GeologyError::Shift(-4))
    );
    assert_eq!(
        with(&|s| s.body = cid("vesta")).err(),
        Some(GeologyError::BodyMismatch {
            spec: "vesta".to_owned(),
            body: "ceres".to_owned()
        })
    );
    // The top row of the site is origin + 63: on the lattice up to north − 1.
    assert!(with(&|s| s.origin = (8_000, north - 64)).is_ok());
    assert_eq!(
        with(&|s| s.origin = (8_000, north - 56)).err(),
        Some(GeologyError::OffLattice)
    );
    assert!(with(&|s| s.origin = (8_000, south)).is_ok());
    assert_eq!(
        with(&|s| s.origin = (8_000, south - 8)).err(),
        Some(GeologyError::OffLattice)
    );

    // A window one cell short is not a site's ground.
    let region = site_region();
    let mut frame = site_frame(&region);
    frame.bbox.max.x = 255_000;
    let mut records = Records::default();
    records.regions.insert(region.key.clone(), region);
    let short = capture(&records, &frame).expect("the capture");
    assert_eq!(
        build_site_ground(&short, &site_spec(), &body, &k, 1).err(),
        Some("a site's ground is 256 by 256 cells of 10 m".to_owned())
    );
}

// ---------------------------------------------------------------------------
// 4. The key at the composition root
// ---------------------------------------------------------------------------

/// The readiness report's geology line.
fn geology_line(tw: &TestWorld) -> ReadinessLine {
    let report = tacsim::port::readiness(tw.world());
    let line = report
        .lines
        .iter()
        .find(|line| line.binding == Binding::Geology)
        .cloned()
        .expect("a geology line");
    // Reported, never gated: whatever the gate says, it never names this line.
    if let Err(refused) = tacsim::port::enrol_gate(&report) {
        assert_ne!(
            refused.binding,
            Binding::Geology,
            "the geology line never gates"
        );
    }
    line
}

/// The `tacsim:geology` writes in a batch of module ops.
fn pin_writes(ops: &[mush_core::ModuleOp]) -> Vec<serde_json::Value> {
    ops.iter()
        .filter(|op| op.ns == tacsim::NS)
        .filter_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, value } if key == GEOLOGY_PIN_KEY => Some(value.clone()),
            _ => None,
        })
        .collect()
}

fn prefix(fingerprint: &[u8; 32]) -> String {
    base16(fingerprint).chars().take(12).collect()
}

/// **The first keyed boot pins the key and every later boot compares it** (Q-20-12; §13's
/// "a wrong fingerprint disables the layer with the reason on the readiness line").
#[test]
fn sm1_the_first_boot_pins_the_key_and_the_next_compares_it() {
    let one = secret(KEY_1);
    let two = secret(KEY_2);
    let mut tw = boot_keyed(Some(GeologyBoot::Key(one.clone())), HashMap::new(), true);
    let ops = tw.take_module_ops();
    let pins = pin_writes(&ops);
    assert_eq!(
        pins,
        vec![serde_json::json!({
            "fingerprint": base16(&one.fingerprint()),
            "key_version": 1,
        })],
        "exactly one pin, the key's fingerprint"
    );
    let line = geology_line(&tw);
    assert_eq!(line.state, LineState::Ready);
    assert_eq!(
        line.text,
        format!(
            "geology: key v1, fingerprint {}, pinned; reading rule v1.",
            prefix(&one.fingerprint())
        )
    );
    assert!(tacsim::surface::live(tw.world()).is_ok());
    // A key handed in after the boot pass is refused and changes nothing.
    let before = tacsim::surface::key::status(tw.world()).cloned();
    assert!(!tacsim::surface::install_geology(
        tw.world_mut(),
        GeologyBoot::Key(two.clone())
    ));
    assert_eq!(tacsim::surface::key::status(tw.world()).cloned(), before);

    // A reboot over the folded store with the same key: Ready, and no second pin.
    let folded = fold_module_ops(ops);
    let mut again = boot_keyed(Some(GeologyBoot::Key(one.clone())), folded.clone(), true);
    assert!(
        pin_writes(&again.take_module_ops()).is_empty(),
        "no rewrite"
    );
    assert_eq!(geology_line(&again).state, LineState::Ready);

    // A third boot with another key: Flagged with the mismatch, nothing written.
    let mut wrong = boot_keyed(Some(GeologyBoot::Key(two.clone())), folded, true);
    assert!(
        pin_writes(&wrong.take_module_ops()).is_empty(),
        "no rewrite"
    );
    let line = geology_line(&wrong);
    assert_eq!(line.state, LineState::Flagged);
    assert_eq!(
        line.text,
        format!(
            "geology: disabled: the key's fingerprint {} is not the pinned {}; restore the key \
             directory this world first booted with.",
            prefix(&two.fingerprint()),
            prefix(&one.fingerprint())
        )
    );
    assert!(matches!(
        tacsim::surface::live(wrong.world()),
        Err(GeologyDisabled::Mismatch { .. })
    ));
}

/// Every disabled reason reads on the readiness line (Flagged), and none gates the desk.
#[test]
fn sm1_every_disabled_reason_reads_on_the_readiness_line() {
    let flagged = |tw: &TestWorld, reason: &str| {
        let line = geology_line(tw);
        assert_eq!(
            (line.state, line.text.as_str()),
            (
                LineState::Flagged,
                format!("geology: disabled: {reason}.").as_str()
            )
        );
    };
    let keyless = boot_keyed(None, HashMap::new(), true);
    flagged(&keyless, "no [tacsim.geology].key_dir is configured");

    let unreadable = boot_keyed(
        Some(GeologyBoot::Unreadable("x".to_owned())),
        HashMap::new(),
        true,
    );
    flagged(&unreadable, "the key directory did not load (x)");

    let mut store: HashMap<SmolStr, ModuleBootData> = HashMap::new();
    store.entry(SmolStr::new(tacsim::NS)).or_default().kv.push((
        SmolStr::new(GEOLOGY_PIN_KEY),
        serde_json::json!({ "fingerprint": "zz" }),
    ));
    let mut corrupt = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), store, true);
    flagged(
        &corrupt,
        "tacsim:geology cannot be read; staff repair the document",
    );
    assert!(
        pin_writes(&corrupt.take_module_ops()).is_empty(),
        "the boot never rewrites an unreadable pin"
    );
    assert_eq!(
        tacsim::surface::key::status(corrupt.world()),
        Some(&GeologyStatus::Disabled(GeologyDisabled::PinUnreadable))
    );
    // A record that PARSES but is not a pin is unreadable too, never a mismatch: the key's own
    // fingerprint in upper case (not 64 lowercase base-16 characters), and the right fingerprint
    // at version 0.
    let fingerprint = base16(&secret(KEY_1).fingerprint());
    assert_ne!(
        fingerprint.to_uppercase(),
        fingerprint,
        "the digest has a letter"
    );
    for body in [
        serde_json::json!({ "fingerprint": fingerprint.to_uppercase(), "key_version": 1 }),
        serde_json::json!({ "fingerprint": fingerprint, "key_version": 0 }),
    ] {
        let mut store: HashMap<SmolStr, ModuleBootData> = HashMap::new();
        store
            .entry(SmolStr::new(tacsim::NS))
            .or_default()
            .kv
            .push((SmolStr::new(GEOLOGY_PIN_KEY), body.clone()));
        let mut malformed = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), store, true);
        flagged(
            &malformed,
            "tacsim:geology cannot be read; staff repair the document",
        );
        assert!(
            pin_writes(&malformed.take_module_ops()).is_empty(),
            "the boot never rewrites a malformed pin: {body}"
        );
    }

    let off = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), HashMap::new(), false);
    flagged(&off, "the layer is off");
}

fn said(
    tw: &TestWorld,
    events: &[(mush_core::Audience, mush_proto::OutputEvent)],
    who: ObjId,
) -> String {
    tw.seen_by(events, who).join("\n")
}

/// What `@geology` must print at `pocket` under key v1, built HERE from `pocket_at`'s values with
/// the brief's labels, separators and plural rule written out — never by the handler's own
/// formatter, so a regression in it cannot hide behind itself.
fn expected_lines(k: &GeologyMac, body: &BodyGeology, pocket: (i64, i64)) -> [String; 4] {
    const LABELS: [(OreFamily, &str); 4] = [
        (OreFamily::Metals, "metals"),
        (OreFamily::Volatiles, "volatiles"),
        (OreFamily::CarbonFeedstock, "carbon feedstock"),
        (OreFamily::StrategicElements, "strategic elements"),
    ];
    let word = |w: Word| match w {
        Word::Barren => "barren",
        Word::Lean => "lean",
        Word::Fair => "fair",
        Word::Rich => "rich",
    };
    let (field, row) = pocket_at(k, body, pocket).expect("a pocket");
    let province: Vec<String> = LABELS
        .iter()
        .map(|(family, label)| format!("{label} {}", word(by(field.province.words, *family))))
        .collect();
    let fields: Vec<String> = LABELS
        .iter()
        .map(|(family, label)| {
            let count = by(field.counts, *family);
            let noun = if count == 1 { "pocket" } else { "pockets" };
            format!(
                "{label} {} ({count} {noun})",
                word(by(field.words, *family))
            )
        })
        .collect();
    let ore: Vec<String> = LABELS
        .iter()
        .filter(|(family, _)| by(row.classes, *family) != Word::Barren)
        .map(|(family, label)| {
            format!(
                "{label} {} {} t",
                word(by(row.classes, *family)),
                by(row.tonnes, *family)
            )
        })
        .collect();
    let pocket_words = if ore.is_empty() {
        "barren".to_owned()
    } else {
        ore.join("; ")
    };
    [
        format!(
            "Geology of {} at pocket {},{} (key v1; reading rule v1):",
            body.body, pocket.0, pocket.1
        ),
        format!(
            "  province {},{}: {}.",
            field.province.province.0,
            field.province.province.1,
            province.join(", ")
        ),
        format!(
            "  field {},{}: {}.",
            field.field.0,
            field.field.1,
            fields.join(", ")
        ),
        format!("  pocket {},{}: {pocket_words}.", pocket.0, pocket.1),
    ]
}

/// `@geology` answers staff with the truth and refuses everyone else; it writes nothing.
#[test]
fn sm1_geology_answers_staff_with_the_truth_and_refuses_everyone_else() {
    let mut tw = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), HashMap::new(), true);
    tacsim::surface::install_body_geology(tw.world_mut(), ceres()).expect("ceres installs");
    let _ = tw.take_module_ops();

    let k = mac(KEY_1);
    let body = ceres();
    let lines = expected_lines(&k, &body, (1_200, -340));
    assert_eq!(
        lines[0],
        "Geology of ceres at pocket 1200,-340 (key v1; reading rule v1):"
    );
    assert!(lines[2].starts_with("  field 150,-43: "));
    // The handler's own formatter agrees with the test's.
    let (field, row) = pocket_at(&k, &body, (1_200, -340)).expect("a pocket");
    assert_eq!(
        tacsim::surface::commands::geology_lines(&cid("ceres"), (1_200, -340), 1, &field, &row),
        lines
    );
    let events = tw.run(GOD, "@geology ceres 1200,-340");
    assert_eq!(said(&tw, &events, GOD), lines.join("\n"));

    // A pocket with no ore reads `barren`, and a field with one ore pocket of a family reads
    // "(1 pocket)": each found by scanning under the fixed key, then asked of the verb.
    let barren = (0..4_096_i64)
        .map(|i| (1_200 + i % 64, -340 + i / 64))
        .find(|p| pocket_at(&k, &body, *p).expect("a pocket").1.families() == 0)
        .expect("a barren pocket near 1200,-340");
    let lines = expected_lines(&k, &body, barren);
    assert_eq!(
        lines[3],
        format!("  pocket {},{}: barren.", barren.0, barren.1)
    );
    let events = tw.run(GOD, &format!("@geology ceres {},{}", barren.0, barren.1));
    assert_eq!(said(&tw, &events, GOD), lines.join("\n"));
    let single = (0..4_096_i64)
        .map(|i| ((150 + i % 64) * 8, (-43 + i / 64) * 8))
        .find(|p| {
            let (field, _) = pocket_at(&k, &body, *p).expect("a pocket");
            field.counts.contains(&1)
        })
        .expect("a field with one ore pocket of a family");
    let lines = expected_lines(&k, &body, single);
    assert!(
        lines[2].contains(" (1 pocket)") && !lines[2].contains("(1 pockets)"),
        "{}",
        lines[2]
    );
    let events = tw.run(GOD, &format!("@geology ceres {},{}", single.0, single.1));
    assert_eq!(said(&tw, &events, GOD), lines.join("\n"));
    let double = (0..4_096_i64)
        .map(|i| (1_200 + i % 64, -340 + i / 64))
        .find(|p| pocket_at(&k, &body, *p).expect("a pocket").1.families() == 2)
        .expect("a two-family pocket near 1200,-340");
    let lines = expected_lines(&k, &body, double);
    assert_eq!(lines[3].matches("; ").count(), 1, "{}", lines[3]);
    let events = tw.run(GOD, &format!("@geology ceres {},{}", double.0, double.1));
    assert_eq!(said(&tw, &events, GOD), lines.join("\n"));

    let lines = expected_lines(&k, &body, (1_200, -340));
    let events = tw.run(GOD, "@geology ceres 1200,-340 field");
    assert_eq!(
        said(&tw, &events, GOD),
        format!("{}\n{}", lines[0], lines[2])
    );

    let pat = tw.spawn_player("Pat");
    let events = tw.run(pat, "@geology ceres 1200,-340");
    assert_eq!(said(&tw, &events, pat), "Permission denied.");

    // Doc 20 cut SM.4b: the usage names the site form too.
    let usage = "Usage: @geology <body> <x>,<y> [province|field|pocket]  or  @geology <site> <pocket>|<field> [province|field|pocket]";
    for bad in ["@geology ceres 1200", "@geology ceres 1200,-340 seam"] {
        let events = tw.run(GOD, bad);
        assert_eq!(said(&tw, &events, GOD), usage, "{bad}");
    }
    // Doc 20 cut SM.2: the boot pass installs the lane's Ceres and Vesta tables, so the
    // body with no table is Pallas (surface figures, no geology).
    let events = tw.run(GOD, "@geology pallas 0,0");
    assert_eq!(
        said(&tw, &events, GOD),
        "No geology table is loaded for pallas."
    );
    let events = tw.run(GOD, "@geology ceres 0,18456");
    assert_eq!(
        said(&tw, &events, GOD),
        "That point is off ceres: y runs from -18456 to 18455."
    );
    assert!(
        tw.take_module_ops().iter().all(|op| op.ns != tacsim::NS),
        "no command writes a document"
    );

    let mut keyless = boot_keyed(None, HashMap::new(), true);
    tacsim::surface::install_body_geology(keyless.world_mut(), ceres()).expect("ceres installs");
    let events = keyless.run(GOD, "@geology ceres 1200,-340");
    assert_eq!(
        said(&keyless, &events, GOD),
        "Geology is disabled: no [tacsim.geology].key_dir is configured."
    );
}

/// Ahuna Flank's spec from the lane and the world's installed Ceres table (`knowing.rs`'s
/// `spec_of`).
fn ahuna_spec(tw: &TestWorld) -> (SiteGeologySpec, std::sync::Arc<BodyGeology>) {
    use tacsim::pack::PackEntry;
    let pack = tacsim::pack::load_pack(tacsim::definitions::content::LANE_JSON).expect("the lane");
    let def = match pack.entry(&cid("ahuna_port")) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("ahuna_port is a port entry: {other:?}"),
    };
    let location = match pack.entry(&def.location) {
        Some(PackEntry::Location(location)) => location.clone(),
        other => panic!("{} is a location: {other:?}", def.location),
    };
    let body = tacsim::surface::body_geology(tw.world(), location.body.as_ref().expect("a body"))
        .expect("the body's table is installed");
    let spec =
        tacsim::port::site::geology_spec(def.site.as_ref().expect("a site"), &location, &body)
            .expect("a spec");
    (spec, body)
}

/// **`@geology <site> <addr>` reads a site address** (doc 20 cut SM.4b; the brief's 3.12): on a
/// keyed world with the lane's sites, `@geology ahuna flank C117` prints the header naming the
/// site, the address, the lattice pocket, the key, the reading rule and the block's authored
/// shift, then the province, field and pocket lines of that lattice pocket **under the shift** —
/// built here from `field_ground` at the site's own field with the block's shift, never from the
/// handler's formatter. `C-NW` prints the header, the province and the field; a level word
/// narrows; `pocket` with a field address and an unknown site refuse; a non-staff actor is
/// refused; nothing is written.
#[test]
fn sm4b_geology_reads_a_site_address() {
    let mut tw = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), HashMap::new(), true);
    let _ = tw.take_module_ops();
    let k = mac(KEY_1);
    let (spec, body) = ahuna_spec(&tw);
    // C117 is site pocket (36, 56): field (4, 7), block C (bx 2, by 3), shift index 14.
    let (px, py) = (36_i64, 56_i64);
    let (fx, fy) = (px / 8, py / 8);
    let shift = spec.shifts[usize::try_from((fy / 2) * 4 + fx / 2).expect("a block")];
    let ground = field_ground(
        &k,
        &body,
        (spec.origin.0 / 8 + fx, spec.origin.1 / 8 + fy),
        shift,
    )
    .expect("the field's ground");
    let row = ground.pocket(px % 8, py % 8).expect("the pocket");
    let lattice = (spec.origin.0 + px, spec.origin.1 + py);
    let words = |words: [Word; 4], counts: Option<[u8; 4]>| -> String {
        OreFamily::ALL
            .iter()
            .map(|family| {
                let at = usize::from(family.code());
                let label = family.label();
                match counts {
                    Some(counts) => {
                        let n = counts[at];
                        let noun = if n == 1 { "pocket" } else { "pockets" };
                        format!("{label} {} ({n} {noun})", words[at].name())
                    }
                    None => format!("{label} {}", words[at].name()),
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let pocket_words = {
        let ore: Vec<String> = OreFamily::ALL
            .iter()
            .filter_map(|family| {
                let at = usize::from(family.code());
                (row.classes[at] != Word::Barren).then(|| {
                    format!(
                        "{} {} {} t",
                        family.label(),
                        row.classes[at].name(),
                        row.tonnes[at]
                    )
                })
            })
            .collect();
        if ore.is_empty() {
            "barren".to_owned()
        } else {
            ore.join("; ")
        }
    };
    let header = format!(
        "Geology of Ahuna Flank C117 — pocket {},{} of ceres (key v1; reading rule v1; block C \
         shift {shift}):",
        lattice.0, lattice.1
    );
    let province = format!(
        "  province {},{}: {}.",
        ground.province.province.0,
        ground.province.province.1,
        words(ground.province.words, None)
    );
    let field = format!(
        "  field {},{}: {}.",
        ground.field.0,
        ground.field.1,
        words(ground.words, Some(ground.counts))
    );
    let pocket = format!("  pocket {},{}: {pocket_words}.", lattice.0, lattice.1);
    let events = tw.run(GOD, "@geology ahuna flank C117");
    assert_eq!(
        said(&tw, &events, GOD),
        [
            header.clone(),
            province.clone(),
            field.clone(),
            pocket.clone()
        ]
        .join("\n")
    );
    let events = tw.run(GOD, "@geology ahuna_port c117 pocket");
    assert_eq!(said(&tw, &events, GOD), [header, pocket].join("\n"));
    // The field form: header, province and field.
    let events = tw.run(GOD, "@geology Ahuna Flank C-NW");
    let field_header = format!(
        "Geology of Ahuna Flank C-NW — field {},{} of ceres (key v1; reading rule v1; block C \
         shift {shift}):",
        ground.field.0, ground.field.1
    );
    assert_eq!(
        said(&tw, &events, GOD),
        [field_header, province, field].join("\n")
    );
    let usage = "Usage: @geology <body> <x>,<y> [province|field|pocket]  or  @geology <site> <pocket>|<field> [province|field|pocket]";
    let events = tw.run(GOD, "@geology ahuna flank C-NW pocket");
    assert_eq!(said(&tw, &events, GOD), usage);
    let events = tw.run(GOD, "@geology nowhere at all C117");
    assert_eq!(
        said(&tw, &events, GOD),
        "There is no mining site called nowhere at all."
    );
    let pat = tw.spawn_player("Pat");
    let events = tw.run(pat, "@geology ahuna flank C117");
    assert_eq!(said(&tw, &events, pat), "Permission denied.");
    assert!(
        tw.take_module_ops().iter().all(|op| op.ns != tacsim::NS),
        "no command writes a document"
    );
}

/// The geology state's site cache size.
fn cached_sites(tw: &TestWorld) -> usize {
    tw.world()
        .module_state::<tacsim::surface::GeologyState>()
        .expect("the geology slot")
        .sites()
        .len()
}

/// Put the pin test's region into the composed atlas's records (where `capture_site` reads).
fn seed_site_region(tw: &mut TestWorld) -> WindowFrame {
    let region = site_region();
    let frame = site_frame(&region);
    tw.world_mut()
        .module_state_mut::<mush_atlas::AtlasState>()
        .expect("the atlas composes")
        .records
        .regions
        .insert(region.key.clone(), region);
    frame
}

/// **The synchronous site door** (lead ruling 4, decision 6): it captures from the composed
/// atlas, keys on (region, revision, key version), builds the very ground the pure functions
/// pin, and a second call is a hit (the same `Arc`); replacing the body's table drops its cached
/// sites, so the next call builds the new table's ground; it refuses while the layer is
/// disabled (the reason), with no table for the body, for an unknown region and for a window
/// that is not a site.
#[test]
fn sm1_the_site_door_builds_once_and_refuses_what_it_cannot_build() {
    let mut tw = boot_keyed(Some(GeologyBoot::Key(secret(KEY_1))), HashMap::new(), true);
    tacsim::surface::install_body_geology(tw.world_mut(), ceres()).expect("ceres installs");
    let frame = seed_site_region(&mut tw);
    let spec = site_spec();

    let first = site_ground_sync(tw.world_mut(), &frame, &spec).expect("the site builds");
    assert_eq!(
        *first,
        site_ground(),
        "the door builds what the pure build pins"
    );
    assert_eq!(
        first.geology.digest(),
        "67bf968c2b5fbe28d1ee2eb55227b88620efb73a194b8bf194e8f46697459eeb"
    );
    assert_eq!(cached_sites(&tw), 1);
    let again = site_ground_sync(tw.world_mut(), &frame, &spec).expect("a hit");
    assert!(Arc::ptr_eq(&first, &again), "the second call is a hit");
    assert_eq!(cached_sites(&tw), 1);
    // Another spec over the same region is another entry.
    let mut moved = spec.clone();
    moved.origin.0 += 8;
    let other = site_ground_sync(tw.world_mut(), &frame, &moved).expect("another site");
    assert!(!Arc::ptr_eq(&first, &other));
    assert_eq!(cached_sites(&tw), 2);

    // Replacing Ceres's table drops its cached sites; the next call builds the new ground.
    let replaced =
        BodyGeology::new(cid("ceres"), CERES_RADIUS_KM, table(VESTA_TABLE)).expect("a table");
    tacsim::surface::install_body_geology(tw.world_mut(), replaced.clone()).expect("replaced");
    assert_eq!(cached_sites(&tw), 0, "a replaced table forgets its sites");
    let rebuilt = site_ground_sync(tw.world_mut(), &frame, &spec).expect("rebuilt");
    assert!(!Arc::ptr_eq(&first, &rebuilt));
    assert_eq!(
        *rebuilt,
        build_site_ground(&site_input(), &spec, &replaced, &mac(KEY_1), 1).expect("the ground")
    );
    assert_ne!(rebuilt.geology.digest(), first.geology.digest());

    // The refusals.
    // Doc 20 cut SM.2: Vesta's table is the lane's now; Pallas has none.
    let mut pallas_spec = spec.clone();
    pallas_spec.body = cid("pallas");
    assert_eq!(
        site_ground_sync(tw.world_mut(), &frame, &pallas_spec).err(),
        Some("No geology table is loaded for pallas.".to_owned())
    );
    let mut unknown = frame.clone();
    unknown.region = RecordKey::new(cid("sm1"), cid("nowhere"));
    assert!(site_ground_sync(tw.world_mut(), &unknown, &spec).is_err());
    let mut short = frame.clone();
    short.bbox.max.y = 255_000;
    assert_eq!(
        site_ground_sync(tw.world_mut(), &short, &spec).err(),
        Some("a site's ground is 256 by 256 cells of 10 m".to_owned())
    );
    assert_eq!(cached_sites(&tw), 1, "a refusal caches nothing");

    // At most 32 body tables: a 33rd body is refused, re-installing one is not.
    // Doc 20 cut SM.2: counted from what the boot pass installed (the lane's tables).
    let standing = tacsim::surface::installed_bodies(tw.world()).len();
    for i in 0..(tacsim::surface::MAX_BODY_GEOLOGIES - standing) {
        let id = cid(&format!("body{i}"));
        tacsim::surface::install_body_geology(
            tw.world_mut(),
            BodyGeology::new(id, 100, table(CERES_TABLE)).expect("a body"),
        )
        .expect("under the cap");
    }
    assert_eq!(
        tacsim::surface::install_body_geology(
            tw.world_mut(),
            BodyGeology::new(cid("toomany"), 100, table(CERES_TABLE)).expect("a body"),
        ),
        Err("a world holds at most 32 body geology tables".to_owned())
    );
    assert!(tacsim::surface::install_body_geology(tw.world_mut(), ceres()).is_ok());

    // A keyless world refuses with the reason, and caches nothing.
    let mut keyless = boot_keyed(None, HashMap::new(), true);
    tacsim::surface::install_body_geology(keyless.world_mut(), ceres()).expect("ceres installs");
    let frame = seed_site_region(&mut keyless);
    assert_eq!(
        site_ground_sync(keyless.world_mut(), &frame, &spec).err(),
        Some("no [tacsim.geology].key_dir is configured".to_owned())
    );
    assert_eq!(cached_sites(&keyless), 0);
}

/// **G1 — reading rule v1, pinned literally** (doc 20 cut SM.4a; the brief's lead ruling 4 and
/// its reviewer finding F7; SM.1's residual T8). Under `KEY_1` and **the fixed test table**
/// `CERES_TABLE` (through `ceres()`, never the lane's: a later lane version may edit Ceres's
/// geology and must not move this pin; `sites.rs`'s `sm2_the_lane_tables_are_section_3_2s`
/// separately pins that today's lane equals it), sixteen fields of the body's own ground (shift
/// 0) spread over the lattice — one every 577 field columns (about one province in three across
/// the 56-column chart) and every 120 field rows north and south alternately, so the provinces
/// and their survey words differ — print their true words, `read_r1` from regolith (words and
/// the pocket counts admitted), from rock and from ice; eight pockets with ore in the first
/// eight print `read_r2`'s classes; the four words' pocket and tonne ranges and
/// `READING_RULE_VERSION` are pinned. Since the cut's pre-commit round (item 9) the pin also
/// holds the province level — each field's province and its `read_r0` survey words, literally —
/// the footing's sharpness order (ice, regolith, rock) and the three classes that give one, and
/// the R1 legs as rules over the sixteen fields: rock reads the true word exactly, ice no
/// return, regolith two adjacent words that never exclude the truth (barren–lean and fair–rich at
/// the edges). **A change to any of these literals is a new
/// `READING_RULE_VERSION` (and a re-read of every chart row), never a re-bless.**
#[test]
fn sm4a_reading_rule_v1_is_pinned_literally() {
    assert_eq!(READING_RULE_VERSION, 1);
    let body = ceres();
    let mac = mac(KEY_1);
    let fields: [((i64, i64), &str); 16] = [
        (
            (0, 0),
            "barren,rich,lean,barren; regolith barren–lean (0-4), fair–rich (5-16), barren–lean (0-4), barren–lean (0-4); rock barren (0-0), rich (9-16), lean (1-4), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (577, 120),
            "fair,lean,barren,barren; regolith lean–fair (1-8), lean–fair (1-8), barren–lean (0-4), barren–lean (0-4); rock fair (5-8), lean (1-4), barren (0-0), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (1154, -240),
            "barren,fair,lean,barren; regolith barren–lean (0-4), fair–rich (5-16), lean–fair (1-8), barren–lean (0-4); rock barren (0-0), fair (5-8), lean (1-4), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (1731, 360),
            "lean,rich,barren,barren; regolith barren–lean (0-4), fair–rich (5-16), barren–lean (0-4), barren–lean (0-4); rock lean (1-4), rich (9-16), barren (0-0), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (2308, -480),
            "lean,lean,barren,barren; regolith lean–fair (1-8), lean–fair (1-8), barren–lean (0-4), barren–lean (0-4); rock lean (1-4), lean (1-4), barren (0-0), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (2885, 600),
            "lean,lean,fair,barren; regolith lean–fair (1-8), lean–fair (1-8), fair–rich (5-16), barren–lean (0-4); rock lean (1-4), lean (1-4), fair (5-8), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (3462, -720),
            "barren,lean,lean,barren; regolith barren–lean (0-4), lean–fair (1-8), lean–fair (1-8), barren–lean (0-4); rock barren (0-0), lean (1-4), lean (1-4), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (4039, 840),
            "barren,lean,lean,barren; regolith barren–lean (0-4), barren–lean (0-4), lean–fair (1-8), barren–lean (0-4); rock barren (0-0), lean (1-4), lean (1-4), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (4616, -960),
            "fair,rich,barren,barren; regolith lean–fair (1-8), fair–rich (5-16), barren–lean (0-4), barren–lean (0-4); rock fair (5-8), rich (9-16), barren (0-0), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (5193, 1080),
            "barren,fair,lean,barren; regolith barren–lean (0-4), fair–rich (5-16), barren–lean (0-4), barren–lean (0-4); rock barren (0-0), fair (5-8), lean (1-4), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (5770, -1200),
            "lean,rich,barren,barren; regolith lean–fair (1-8), fair–rich (5-16), barren–lean (0-4), barren–lean (0-4); rock lean (1-4), rich (9-16), barren (0-0), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (6347, 1320),
            "lean,lean,lean,barren; regolith lean–fair (1-8), barren–lean (0-4), barren–lean (0-4), barren–lean (0-4); rock lean (1-4), lean (1-4), lean (1-4), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (6924, -1440),
            "fair,lean,lean,barren; regolith fair–rich (5-16), lean–fair (1-8), barren–lean (0-4), barren–lean (0-4); rock fair (5-8), lean (1-4), lean (1-4), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (7501, 1560),
            "barren,fair,lean,barren; regolith barren–lean (0-4), fair–rich (5-16), barren–lean (0-4), barren–lean (0-4); rock barren (0-0), fair (5-8), lean (1-4), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (8078, -1680),
            "fair,fair,barren,barren; regolith lean–fair (1-8), lean–fair (1-8), barren–lean (0-4), barren–lean (0-4); rock fair (5-8), fair (5-8), barren (0-0), barren (0-0); ice no return, no return, no return, no return",
        ),
        (
            (8655, 1800),
            "lean,lean,lean,barren; regolith lean–fair (1-8), lean–fair (1-8), barren–lean (0-4), barren–lean (0-4); rock lean (1-4), lean (1-4), lean (1-4), barren (0-0); ice no return, no return, no return, no return",
        ),
    ];
    for (field, pinned) in fields {
        assert_eq!(g1_describe(&mac, &body, field), pinned, "{field:?}");
    }
    // The pre-commit round's item 9: every leg the literals carry, asserted as a rule too, so a
    // quantiser edit that happened to keep these sixteen strings still fails here. Footing sets
    // sharpness (ice 0, regolith 1, rock 2; the three classes a scan is taken from); rock reads
    // each family's true word exactly; ice no return; regolith a band of two adjacent words that
    // never excludes the truth, barren and rich at their own edges.
    assert_eq!(
        [Footing::Ice, Footing::Regolith, Footing::Rock].map(Footing::sharpness),
        [0, 1, 2]
    );
    assert_eq!(
        [
            TerrainClass::Rock,
            TerrainClass::Regolith,
            TerrainClass::Ice
        ]
        .map(Footing::of),
        [
            Some(Footing::Rock),
            Some(Footing::Regolith),
            Some(Footing::Ice)
        ]
    );
    let mut provinces = Vec::new();
    for (field, _) in fields {
        let ground = field_ground(&mac, &body, field, 0).expect("the field's ground");
        let rock = read_r1(&mac, &body, &ground, Footing::Rock);
        let regolith = read_r1(&mac, &body, &ground, Footing::Regolith);
        let ice = read_r1(&mac, &body, &ground, Footing::Ice);
        for (n, truth) in ground.words.into_iter().enumerate() {
            assert_eq!(rock[n], FieldReading::Exact(truth), "{field:?} rock");
            assert_eq!(ice[n], FieldReading::NoReturn, "{field:?} ice");
            let FieldReading::Band(low, high) = regolith[n] else {
                panic!("{field:?}: a regolith reading is a band: {:?}", regolith[n]);
            };
            assert_eq!(low.shifted(1), high, "{field:?}: two adjacent words");
            assert!(
                low == truth || high == truth,
                "{field:?}: the band holds the truth"
            );
            match truth {
                Word::Barren => assert_eq!((low, high), (Word::Barren, Word::Lean)),
                Word::Rich => assert_eq!((low, high), (Word::Fair, Word::Rich)),
                Word::Lean | Word::Fair => {}
            }
        }
        // The province level (R0, the Registry's survey): the field's province and its words.
        let province = &ground.province;
        provinces.push(format!(
            "{:?} {}",
            province.province,
            read_r0(province).map(Word::name).join(",")
        ));
    }
    assert_eq!(
        provinces,
        vec![
            "(0, 14) fair,rich,lean,barren",
            "(3, 14) fair,rich,lean,barren",
            "(7, 12) fair,rich,lean,barren",
            "(10, 16) lean,fair,lean,barren",
            "(14, 11) lean,fair,lean,barren",
            "(17, 17) lean,rich,lean,barren",
            "(21, 9) lean,fair,lean,barren",
            "(24, 19) lean,rich,lean,barren",
            "(28, 8) fair,rich,lean,barren",
            "(31, 20) lean,fair,lean,barren",
            "(35, 6) lean,rich,lean,barren",
            "(38, 22) fair,rich,lean,barren",
            "(42, 5) fair,fair,lean,barren",
            "(45, 23) lean,fair,lean,barren",
            "(49, 3) lean,fair,lean,barren",
            "(52, 24) fair,lean,lean,barren",
        ]
    );
    let pockets: [((i64, i64), [Word; 4]); 8] = [
        (
            (0, 1),
            [Word::Barren, Word::Rich, Word::Barren, Word::Barren],
        ),
        (
            (4617, 960),
            [Word::Fair, Word::Barren, Word::Barren, Word::Barren],
        ),
        (
            (9236, -1919),
            [Word::Barren, Word::Barren, Word::Lean, Word::Barren],
        ),
        (
            (13848, 2880),
            [Word::Barren, Word::Fair, Word::Barren, Word::Barren],
        ),
        (
            (18466, -3839),
            [Word::Barren, Word::Rich, Word::Barren, Word::Barren],
        ),
        (
            (23080, 4800),
            [Word::Barren, Word::Rich, Word::Barren, Word::Barren],
        ),
        (
            (27697, -5760),
            [Word::Barren, Word::Lean, Word::Barren, Word::Barren],
        ),
        (
            (32312, 6720),
            [Word::Barren, Word::Fair, Word::Barren, Word::Barren],
        ),
    ];
    for (pocket, pinned) in pockets {
        let (_, row) = pocket_at(&mac, &body, pocket).expect("the pocket");
        assert_eq!(read_r2(&row), pinned, "{pocket:?}");
    }
    let ranges: Vec<String> = Word::ALL
        .iter()
        .map(|w| {
            let (plo, phi) = w.pocket_range();
            let (tlo, thi) = w.tonnes_range();
            format!("{} {plo}-{phi} pockets {tlo}-{thi} t", w.name())
        })
        .collect();
    assert_eq!(
        ranges,
        vec![
            "barren 0-0 pockets 0-0 t",
            "lean 1-4 pockets 3-6 t",
            "fair 5-8 pockets 6-12 t",
            "rich 9-16 pockets 12-24 t",
        ]
    );
}

/// One field's G1 description: its true words, then `read_r1` from regolith (words and the
/// pocket counts admitted), from rock and from ice.
fn g1_describe(mac: &GeologyMac, body: &BodyGeology, field: (i64, i64)) -> String {
    let ground = field_ground(mac, body, field, 0).expect("the field's ground");
    let words = |w: [Word; 4]| w.map(Word::name).join(",");
    let reads = |footing: Footing| {
        read_r1(mac, body, &ground, footing)
            .iter()
            .map(|r| match r.pockets() {
                Some((lo, hi)) => format!("{} ({lo}-{hi})", r.words()),
                None => r.words(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{}; regolith {}; rock {}; ice {}",
        words(ground.words),
        reads(Footing::Regolith),
        reads(Footing::Rock),
        reads(Footing::Ice)
    )
}
