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

#[test]
fn ordered_loop_joins_original_positions_and_refuses_a_missing_second_leg_or_changed_native_event()
{
    // Passive historical data only. The production collector obtains its
    // route/digest/claims from the actual owner's scoped Core receipt.
    let mut events = events();
    events.extend([
        (1,json!({"kind":"joined","actor":99,"ic_time":512,"at":1234,"witnesses":[22],"detail":{}})),
        (2,json!({"kind":"left","actor":99,"ic_time":700,"at":1234,"witnesses":[20,21,20],"detail":{}})),
    ]);
    let rows: Vec<_> = events
        .iter()
        .map(|(scene, event)| json!({"scene":scene,"event":event}))
        .collect();
    let value = json!({"format":1,"actor":99,"source":10,"admitted_room":11,"projected_room":10,"at":1234,
        "legs":[{"format":1,"actor":99,"from":10,"to":11,"at":1234,"events":&rows[..2]},
                {"format":1,"actor":99,"from":11,"to":10,"at":1234,"events":&rows[2..]}]});
    let bytes = mush_core::canonical_json_bytes_with_limit(
        &value,
        crate::movement_window_capture::MAX_WINDOW_SCENE_CAPTURE_BYTES,
    )
    .expect("ordered bytes");
    let digest = crate::movement_window_capture::WindowSceneCapture::decode(&bytes)
        .expect("ordered data")
        .digest()
        .expect("digest");
    let mut chunks = BTreeMap::<u64, Vec<LedgerEntry>>::new();
    let mut references = Vec::new();
    for (scene, event) in events {
        let entries = chunks.entry(scene).or_default();
        let sequence = entries.len() as u64;
        references.push(SceneLedgerAppendRef {
            scene,
            sequence,
            event_digest: scene_transition::scene_event_digest(&event).expect("event digest"),
        });
        entries.push(LedgerEntry {
            seq: sequence,
            value: event,
        });
    }
    let docs: Vec<LedgerDoc> = chunks
        .into_iter()
        .map(|(scene, entries)| {
            (
                mush_core::chunk_key(&scene.to_string(), 0),
                serde_json::to_string(&LedgerChunk {
                    chunk: 0,
                    sealed: false,
                    entries,
                })
                .expect("chunk"),
            )
        })
        .collect();
    for fault in 0..5 {
        let mut claims = references.clone();
        let counts = if fault == 3 {
            claims.truncate(2);
            vec![2, 0]
        } else {
            vec![2, 2]
        };
        let projected = if fault == 4 { ObjId(12) } else { ObjId(10) };
        let mut audit = Audit::new();
        audit
            .add_window(route(), projected, counts, digest, &claims)
            .expect("original claims");
        let mut original = docs.clone();
        if fault == 1 || fault == 2 {
            let mut chunk: LedgerChunk = serde_json::from_str(&original[0].1).expect("chunk");
            if fault == 1 {
                chunk.entries.pop();
            } else {
                chunk.entries.last_mut().expect("entry").value["at"] = json!(999);
            }
            original[0].1 = serde_json::to_string(&chunk).expect("chunk");
            assert!(audit.read_history(&original).is_err());
        } else {
            original.reverse();
            audit.read_history(&original).expect("original native rows");
            if fault == 0 {
                audit.finish().expect("exact original ordered capture");
            } else {
                assert!(audit.finish().is_err());
            }
        }
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
