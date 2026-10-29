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

#[test]
fn focus_fingerprints_bound_the_same_selected_objects() {
    let focus = FocusState::default();
    assert_eq!(
        focus_fingerprint(&focus, &BTreeSet::new()),
        Err(OwnerReadError::Limit)
    );
    let mut roots: BTreeSet<_> = (0..512).map(ObjId).collect();
    assert!(focus_fingerprint(&focus, &roots).is_ok());
    roots.insert(ObjId(512));
    assert_eq!(
        focus_fingerprint(&focus, &roots),
        Err(OwnerReadError::Limit)
    );
}

#[test]
fn ic_fingerprints_require_loaded_owner_state_and_bound_selected_objects() {
    let roots = BTreeSet::from([ObjId(1)]);
    assert_eq!(
        ic_fingerprint(&crate::ic_exclusion::IcExclusions::default(), &roots),
        Err(OwnerReadError::Integrity)
    );
    let test =
        mush_testkit::TestWorld::with_modules(vec![Box::new(crate::SimulationModule::new())]);
    let actual = test
        .world()
        .module_state::<crate::ic_exclusion::IcExclusions>()
        .expect("registered and loaded owner");
    let mut roots: BTreeSet<_> = (0..512).map(ObjId).collect();
    assert!(ic_fingerprint(actual, &roots).is_ok());
    roots.insert(ObjId(512));
    assert_eq!(ic_fingerprint(actual, &roots), Err(OwnerReadError::Limit));
}
