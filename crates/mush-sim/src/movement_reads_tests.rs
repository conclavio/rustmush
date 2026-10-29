#![cfg(test)]

use super::*;

#[test]
fn deployment_projection_fingerprint_bounds_marks_and_text_before_encoding() {
    let mut test =
        mush_testkit::TestWorld::with_modules(vec![Box::new(crate::SimulationModule::new())]);
    let actual = |w: &mush_core::World| {
        deployment_fingerprint(
            w.module_state::<crate::DeployedUnits>()
                .expect("registered index"),
        )
    };
    let empty = actual(test.world()).expect("empty index");
    for unit in 0..4096 {
        crate::deployed::mark(
            test.world_mut(),
            ObjId(unit),
            crate::DeployedMark::new(1, "arena"),
        );
    }
    let full = actual(test.world()).expect("4096 marks");
    assert_ne!(full, empty);
    crate::deployed::mark(
        test.world_mut(),
        ObjId(4096),
        crate::DeployedMark::new(1, "arena"),
    );
    assert_eq!(actual(test.world()), Err(OwnerReadError::Limit));
    for unit in 0..=4096 {
        crate::deployed::clear(test.world_mut(), ObjId(unit));
    }
    crate::deployed::mark(
        test.world_mut(),
        ObjId(1),
        crate::DeployedMark::new(1, "x".repeat(1025)),
    );
    assert_eq!(actual(test.world()), Err(OwnerReadError::Limit));
    crate::deployed::clear(test.world_mut(), ObjId(1));
    for unit in 0..64 {
        crate::deployed::mark(
            test.world_mut(),
            ObjId(unit),
            crate::DeployedMark::new(1, "x".repeat(1024)),
        );
    }
    assert!(actual(test.world()).is_ok());
    crate::deployed::mark(
        test.world_mut(),
        ObjId(64),
        crate::DeployedMark::new(1, "x"),
    );
    assert_eq!(actual(test.world()), Err(OwnerReadError::Limit));
}

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
