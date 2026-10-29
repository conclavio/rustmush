//! Doc 17h cut M8d D6: the unwarranted chronicle's pure parts.
#![cfg(test)]
use super::*;
use smol_str::SmolStr;

/// A ring folds past its cursor once: the ungoverned rows are findings in sequence order, a
/// second fold of the same ring finds nothing, a newer row moves the cursor, and the census
/// holds.
#[test]
fn m8d_a_ring_is_chronicled_once_past_its_cursor() {
    let hull = ObjId(12);
    let mut chronicle = Chronicle::default();
    let ring = [(3, 300, true), (1, 100, false), (2, 200, true)];
    let found = chronicle.fold(hull, &ring);
    assert_eq!(
        found,
        vec![Find { seq: 2, at_ic: 200 }, Find { seq: 3, at_ic: 300 }]
    );
    assert_eq!(chronicle.hulls[&12].cursor, 3);
    assert_eq!(chronicle.found(hull, 2), Some(200));
    assert_eq!(
        chronicle.found(hull, 1),
        None,
        "a governed inspection is no finding"
    );
    assert_eq!(chronicle.found(ObjId(13), 2), None);
    assert!(
        chronicle.fold(hull, &ring).is_empty(),
        "the cursor is the replay guard"
    );
    assert_eq!(
        chronicle.fold(hull, &[(4, 400, false)]),
        Vec::new(),
        "a governed newer row moves the cursor and finds nothing"
    );
    assert_eq!(chronicle.hulls[&12].cursor, 4);
    assert!(chronicle.check().is_ok());
    let json = serde_json::to_value(&chronicle).expect("the chronicle serializes");
    let back: Chronicle = serde_json::from_value(json).expect("the chronicle decodes");
    assert_eq!(back, chronicle, "the document round-trips");
}

/// The findings kept per hull are bounded by the ring's size, oldest out first.
#[test]
fn m8d_a_hull_keeps_its_newest_findings() {
    let hull = ObjId(5);
    let mut chronicle = Chronicle::default();
    let rows: Vec<(u64, i64, bool)> = (1..=70).map(|seq| (seq, 0, true)).collect();
    assert_eq!(chronicle.fold(hull, &rows).len(), 70);
    let kept = &chronicle.hulls[&5].finds;
    assert_eq!(kept.len(), FINDS_PER_HULL);
    assert_eq!(kept[0].seq, 7);
    assert!(chronicle.check().is_ok());
}

/// The census refuses what a hand-edited document could carry.
#[test]
fn m8d_the_census_refuses_a_bad_document() {
    let mut chronicle = Chronicle::default();
    chronicle.fold(ObjId(1), &[(1, 0, true), (2, 0, true)]);
    let mut bad = chronicle.clone();
    bad.version = 2;
    assert!(bad.check().is_err());
    let mut bad = chronicle.clone();
    bad.hulls.get_mut(&1).expect("row").cursor = 1;
    assert!(bad.check().is_err(), "a finding past the cursor");
    let mut bad = chronicle.clone();
    bad.hulls.get_mut(&1).expect("row").finds.reverse();
    assert!(bad.check().is_err(), "findings out of order");
    let mut bad = chronicle;
    bad.hulls.insert(0, HullChronicle::default());
    assert!(bad.check().is_err(), "hull zero");
}

/// A slot key names its hull; anything else names none.
#[test]
fn m8d_an_inspection_slot_key_names_its_hull() {
    let key = SmolStr::new(mush_roll::commerce::registry_inspection::inspection_key(
        ObjId(0x2a),
        65,
    ));
    assert_eq!(slot_hull(&key), Some(ObjId(0x2a)));
    for bad in [
        "hull/0000002A/slot/01",
        "hull/00000000/slot/01",
        "hull/2a/slot/01",
        "hull/0000002a/slot/1",
        "hull/0000002a",
        "licence/0000002a/slot/01",
    ] {
        assert_eq!(slot_hull(&SmolStr::new(bad)), None, "{bad}");
    }
}

/// The arming half: a fixture line with `unlawful` on a journalled arming or fire counts; a
/// lawful one does not (unreachable in play until the fight).
#[test]
fn m8d_an_unlawful_arming_line_would_count() {
    use crate::arena::ledger::EventKind;
    let armed = |unlawful| EventKind::Armed {
        unit: ObjId(4),
        weapon: SmolStr::new_static("cannon"),
        slot: 1,
        unlawful,
    };
    assert!(unlawful_on_box(&[armed(false), armed(true)]));
    assert!(!unlawful_on_box(&[armed(false)]));
    assert!(!unlawful_on_box(&[]));
}

/// The public line names nothing: no hull number, no "sortie", inside the floor.
#[test]
fn m8d_the_public_line_names_nobody() {
    assert!(!LINE.chars().any(|c| c.is_ascii_digit()));
    assert!(!LINE.contains("sortie") && !LINE.contains('#'));
    assert!(LINE.chars().count() <= 120);
    assert_eq!(KIND, "minds.unwarranted");
}

/// **K7**: a hull read with nothing past its cursor — every hull the boot pass notes — adds no
/// row; only an inspected hull does.
#[test]
fn m8d_the_boot_pass_adds_no_rows() {
    let mut chronicle = Chronicle::default();
    for hull in 1..=100 {
        assert!(chronicle.fold(ObjId(hull), &[]).is_empty());
    }
    assert!(chronicle.hulls.is_empty(), "no rows for uninspected hulls");
    chronicle.fold(ObjId(7), &[(1, 10, false)]);
    assert_eq!(chronicle.hulls.len(), 1);
    chronicle.fold(ObjId(7), &[(1, 10, false)]);
    assert_eq!(chronicle.hulls.len(), 1);
}

/// **K8, D6's arming half**: the chronicle's predicate counts an unlawful arming line beside a
/// governed inspection, through `unlawful_on_box` (a fixture line; no producer yet).
#[test]
fn m8d_the_chronicle_would_count_an_unlawful_arming_line() {
    use crate::arena::ledger::EventKind;
    let armed = EventKind::Armed {
        unit: ObjId(4),
        weapon: SmolStr::new_static("cannon"),
        slot: 1,
        unlawful: true,
    };
    assert!(finding(false, &[armed]));
    assert!(finding(true, &[]));
    assert!(!finding(false, &[]));
}
