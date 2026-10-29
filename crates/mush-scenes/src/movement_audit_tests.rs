#![cfg(test)]

use super::*;
use mush_core::ledger::{LedgerChunk, LedgerEntry};

fn route() -> Route {
    Route {
        actor: ObjId(99),
        from: Some(ObjId(10)),
        to: ObjId(11),
        at: 1234,
    }
}

fn events() -> Vec<(u64, Value)> {
    vec![
        (
            2,
            json!({"kind":"joined","actor":99,"ic_time":700,"at":1234,
        "witnesses":[20,21,20],"detail":{}}),
        ),
        (
            1,
            json!({"kind":"left","actor":99,"ic_time":512,"at":1234,
        "witnesses":[22],"detail":{}}),
        ),
    ]
}

fn claims(events: &[(u64, Value)]) -> (DefinitionDigest, Vec<SceneLedgerAppendRef>) {
    let rows: Vec<_> = events
        .iter()
        .map(|(scene, event)| json!({"scene":scene,"event":event}))
        .collect();
    let capture = json!({"format":1,"actor":99,"from":10,"to":11,"at":1234,"events":rows});
    let digest = scene_transition::scene_capture_digest(&capture).expect("capture digest");
    let refs = events
        .iter()
        .map(|(scene, event)| SceneLedgerAppendRef {
            scene: *scene,
            sequence: 0,
            event_digest: scene_transition::scene_event_digest(event).expect("event digest"),
        })
        .collect();
    (digest, refs)
}

fn docs(events: &[(u64, Value)]) -> Vec<LedgerDoc> {
    events
        .iter()
        .map(|(scene, event)| {
            let chunk = LedgerChunk {
                chunk: 0,
                sealed: false,
                entries: vec![
                    LedgerEntry {
                        seq: 0,
                        value: event.clone(),
                    },
                    // Boot must locate the original sequence, rather than compare
                    // the receipt to an unrelated newer lifecycle event.
                    LedgerEntry {
                        seq: 1,
                        value: json!({"kind":"ended","at":9999}),
                    },
                ],
            };
            (
                format!("log:{scene}:0").into(),
                serde_json::to_string(&chunk).expect("chunk"),
            )
        })
        .collect()
}

fn audit(events: &[(u64, Value)]) -> Audit {
    let (digest, refs) = claims(events);
    let mut audit = Audit::new();
    audit.add(route(), digest, &refs).expect("receipt evidence");
    audit
}

#[test]
fn original_sequence_join_survives_later_history_without_live_scene_state() {
    let events = events();
    let mut audit = audit(&events);
    audit
        .read_history(&docs(&events))
        .expect("original events still present");
    audit
        .finish()
        .expect("same captured route and original times");
}

#[test]
fn missing_original_event_or_changed_utc_refuses() {
    let events = events();
    let mut missing = audit(&events);
    assert!(missing.read_history(&docs(&events)[1..]).is_err());
    let mut corrupt = events.clone();
    corrupt[0].1["at"] = json!(9999);
    let mut changed = audit(&events);
    assert!(changed.read_history(&docs(&corrupt)).is_err());
}

#[test]
fn receipt_capture_digest_and_event_order_are_both_checked() {
    let events = events();
    let mut reversed = events.clone();
    reversed.reverse();
    let mut wrong_order = audit(&reversed);
    wrong_order
        .read_history(&docs(&events))
        .expect("matching individual event hashes");
    assert!(wrong_order.finish().is_err());
    let (_, refs) = claims(&events);
    let mut wrong_digest = Audit::new();
    wrong_digest
        .add(route(), DefinitionDigest::from_bytes([0; 32]), &refs)
        .expect("bounded claims");
    wrong_digest
        .read_history(&docs(&events))
        .expect("events match");
    assert!(wrong_digest.finish().is_err());
}

#[test]
fn duplicate_claims_and_budget_exhaustion_refuse_before_new_proofs() {
    let events = events();
    let (digest, refs) = claims(&events);
    let mut duplicate = audit(&events);
    assert!(duplicate.add(route(), digest, &refs).is_err());
    let mut bounded = Audit::new();
    bounded.remaining = 1;
    assert!(bounded.add(route(), digest, &refs).is_err());
    assert!(bounded.proofs.is_empty());
    assert!(bounded.expected.is_empty());
}

#[test]
fn empty_capture_is_authenticated_without_inventing_a_transcript_row() {
    let mut empty = audit(&[]);
    empty.check_empty_captures().expect("typed empty capture");
    empty
        .read_history(&[])
        .expect("no claimed ledger positions");
    empty.finish().expect("exact empty capture");
    let mut changed = audit(&[]);
    changed.proofs[0].digest = DefinitionDigest::from_bytes([1; 32]);
    assert!(changed.check_empty_captures().is_err());
}
