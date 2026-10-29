#![cfg(test)]

use super::*;

#[test]
fn journey_fingerprints_bound_selected_roots_and_refuse_any_held_record() {
    let mut book = JourneyBook::default();
    assert_eq!(
        fingerprint(&book, &BTreeSet::new()),
        Err(OwnerReadError::Limit)
    );
    let mut roots: BTreeSet<_> = (0..512).map(ObjId).collect();
    let initial = fingerprint(&book, &roots).expect("512 selected roots");
    book.next_id = 10;
    book.cursor = 42;
    assert_eq!(
        fingerprint(&book, &roots).expect("unconsulted allocator"),
        initial
    );
    book.held.insert("cjourney:broken".into(), "repair".into());
    assert_eq!(fingerprint(&book, &roots), Err(OwnerReadError::Refused));
    book.held.clear();
    roots.insert(ObjId(512));
    assert_eq!(fingerprint(&book, &roots), Err(OwnerReadError::Limit));
}
