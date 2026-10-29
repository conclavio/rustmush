#![cfg(test)]

use super::*;
use crate::record::RecordKey;
use crate::travel::KnownDoc;
use mush_core::{CanonicalId, ClockRef};

fn key(id: &str) -> RecordKey {
    RecordKey::new(
        CanonicalId::new("test").unwrap(),
        CanonicalId::new(id).unwrap(),
    )
}
fn mutation() -> Mutation {
    Mutation {
        format: 1,
        actor: ObjId(90),
        at_utc: 100,
        source: Source::Prepared {
            commit: CommitToken::from_bytes([1; 16]),
            prepared_at: ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 100,
            },
            capture_digest: DefinitionDigest::from_bytes([2; 32]),
        },
        known: KnownDoc {
            places: vec![key("one")],
        },
    }
}
fn expected(mutation: &Mutation) -> Expected {
    let Source::Prepared {
        commit,
        prepared_at,
        capture_digest,
    } = &mutation.source
    else {
        panic!("prepared fixture");
    };
    Expected {
        actor: mutation.actor,
        before: Some(known_digest(None).unwrap()),
        after: Some(known_digest(Some(&mutation.known.places)).unwrap()),
        event: Some(EventClaim {
            sequence: 0,
            digest: mutation.digest().unwrap(),
        }),
        commit: *commit,
        stamp: prepared_at.clone(),
        at_utc: mutation.at_utc,
        capture: *capture_digest,
    }
}
fn audit(row: &Mutation) -> Audit {
    let mut audit = Audit::new();
    audit.pending.insert(0, vec![expected(row)]);
    audit
}
fn travel(known: Vec<RecordKey>) -> TravelState {
    let mut state = TravelState::default();
    state.known.insert(ObjId(90), known);
    state
}
#[test]
fn later_legacy_overwrite_advances_original_history_without_reclassifying_the_prepared_event() {
    let original = mutation();
    let mut audit = audit(&original);
    audit.boundary(0, Some(&original)).unwrap();
    let mut later = original.clone();
    later.source = Source::Legacy;
    later.at_utc = 200;
    later.known.places.push(key("two"));
    audit.boundary(1, Some(&later)).unwrap();
    audit.boundary(2, None).unwrap();
    audit.finish(&travel(later.known.places)).unwrap();
    let mut audit = self::audit(&original);
    audit.boundary(0, Some(&original)).unwrap();
    assert!(audit.finish(&travel(vec![key("one"), key("two")])).is_err());
}
#[test]
fn missing_or_orphaned_prepared_event_changed_clock_and_double_claim_refuse() {
    let original = mutation();
    assert!(audit(&original).boundary(0, None).is_err());
    assert!(Audit::new().boundary(0, Some(&original)).is_err());
    let mut changed = original.clone();
    changed.at_utc += 1;
    assert!(audit(&original).boundary(0, Some(&changed)).is_err());
    let mut changed = original.clone();
    changed.source = Source::Legacy;
    assert!(audit(&original).boundary(0, Some(&changed)).is_err());
    let mut doubled = audit(&original);
    doubled
        .pending
        .get_mut(&0)
        .unwrap()
        .push(expected(&original));
    assert!(doubled.boundary(0, Some(&original)).is_err());
}
#[test]
fn no_op_observations_bind_the_original_knowledge_boundary_and_absence_is_not_empty() {
    let original = mutation();
    let mut audit = audit(&original);
    audit.boundary(0, Some(&original)).unwrap();
    let mut no_op = expected(&original);
    no_op.before = Some(known_digest(Some(&original.known.places)).unwrap());
    no_op.after = None;
    no_op.event = None;
    audit.pending.insert(1, vec![no_op]);
    audit.boundary(1, None).unwrap();
    audit
        .finish(&travel(original.known.places.clone()))
        .unwrap();
    let mut audit = self::audit(&original);
    audit
        .pending
        .get_mut(&0)
        .unwrap()
        .first_mut()
        .unwrap()
        .before = Some(known_digest(Some(&[])).unwrap());
    audit
        .observe(original.actor, known_digest(None).unwrap())
        .unwrap();
    assert!(audit.boundary(0, Some(&original)).is_err());
}
