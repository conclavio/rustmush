#![cfg(test)]

use super::*;
use mush_core::{CanonicalId, ClockRef};
use serde_json::json;

fn key(id: &str) -> RecordKey {
    RecordKey::new(
        CanonicalId::new("test").unwrap(),
        CanonicalId::new(id).unwrap(),
    )
}
fn row() -> Mutation {
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
#[test]
fn strict_mutation_codec_preserves_original_context_and_refuses_unknown_profiles_and_limits() {
    let row = row();
    let value = row.value().unwrap();
    assert_eq!(Mutation::decode(&value).unwrap().value().unwrap(), value);
    for field in ["format", "actor", "at_utc", "source", "known"] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(Mutation::decode(&missing).is_err());
    }
    let mut unknown = value.clone();
    unknown["source"]["unexpected"] = json!(true);
    assert!(Mutation::decode(&unknown).is_err());
    let mut wrong = row.clone();
    wrong.format = 2;
    assert!(wrong.value().is_err());
    let mut wrong = row.clone();
    wrong.known.places = vec![key("same"); MAX_KNOWN_PLACES + 1];
    assert!(wrong.value().is_err());
    let mut wrong = row.clone();
    wrong.known.places.push(key("one"));
    assert!(wrong.value().is_err());
    let mut wrong = value.clone();
    wrong["known"]["places"][0]["pack"] = json!("x".repeat(65));
    assert!(Mutation::decode(&wrong).is_err());
    let mut huge = value;
    huge["extra"] = json!("x".repeat(MAX_BYTES + 1));
    assert!(Mutation::decode(&huge).is_err());
}
#[test]
fn legacy_projection_keeps_its_existing_ordered_first_512_deduplication() {
    let mut row = row();
    row.source = Source::Legacy;
    row.known.places = vec![key("one"), key("one"), key("two")];
    Mutation::decode(&row.value().unwrap()).unwrap();
    assert_eq!(normalized(&row.known), vec![key("one"), key("two")]);
    assert_ne!(
        known_digest(None).unwrap(),
        known_digest(Some(&[])).unwrap()
    );
}
#[test]
fn configured_legacy_learning_uses_the_real_ledger_and_refuses_oversize_before_any_append() {
    let mut history = MovementHistory::default();
    assert!(
        history
            .legacy(ObjId(90), 100, &row().known)
            .unwrap()
            .is_none()
    );
    assert_eq!(history.ledger.next_seq(LOG), 0);
    history.enabled = true;
    let write = history
        .legacy(ObjId(90), 100, &row().known)
        .unwrap()
        .unwrap();
    assert!(
        matches!(write, mush_core::ModuleWrite::JsonSet { ref key, .. } if key.starts_with("log:movement_knowledge:"))
    );
    assert_eq!(history.ledger.next_seq(LOG), 1);
    let oversized = KnownDoc {
        places: vec![key("one"); MAX_KNOWN_PLACES + 1],
    };
    assert!(history.legacy(ObjId(90), 101, &oversized).is_err());
    assert_eq!(history.ledger.next_seq(LOG), 1);
}
