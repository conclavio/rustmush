#![cfg(test)]
//! Ledger mechanics only; these fixtures grant no owner or physical proof.

use super::*;
use serde_json::json;

fn entries() -> Vec<(String, serde_json::Value)> {
    vec![
        ("a".into(), json!("joined")),
        ("b".into(), json!("left")),
        ("a".into(), json!("left")),
        ("b".into(), json!("joined")),
    ]
}

fn append(ledger: &mut Ledger, id: &str, value: serde_json::Value) {
    let plan = ledger.prepare_append(id, value).unwrap();
    ledger.apply_prepared(plan).unwrap();
}

#[test]
fn repeated_logs_keep_original_order_and_only_final_chunk_writes() {
    let mut ledger = Ledger::new();
    append(&mut ledger, "a", json!("prior"));
    append(&mut ledger, "unrelated", json!("x".repeat(8192)));
    let before = ledger.clone();
    let plan = ledger.prepare_sequence_bounded(entries(), 4096).unwrap();
    assert_eq!(plan.expected.len(), 2);
    assert!(!plan.tails.contains_key("unrelated"));
    assert_eq!(
        plan.positions().collect::<Vec<_>>(),
        vec![("a", 1), ("b", 0), ("a", 2), ("b", 1)]
    );
    let writes = plan
        .authoritative_replay_writes(OwnerId::new("scenes").unwrap())
        .unwrap();
    assert_eq!(writes.len(), 2);
    for write in &writes {
        let ReplayWrite::LedgerChunkSet { write } = write else {
            panic!("ledger write");
        };
        assert_eq!(
            write.chunk().entries.len(),
            if write.log_id() == "a" { 3 } else { 2 }
        );
    }
    drop(ledger.bind_prepared_sequence(plan).unwrap());
    assert_eq!(ledger, before);
    let plan = ledger.prepare_sequence_bounded(entries(), 4096).unwrap();
    let installed = ledger.bind_prepared_sequence(plan).unwrap().apply();
    assert_eq!(installed.len(), 2);
    assert_eq!(ledger.next_seq("a"), 3);
    assert_eq!(ledger.next_seq("b"), 2);
    assert_eq!(ledger.logs["unrelated"], before.logs["unrelated"]);
    let docs: Vec<_> = installed
        .iter()
        .map(|write| match write {
            ModuleWrite::JsonSet { key, value } => {
                (key.clone(), serde_json::to_string(value).unwrap())
            }
            _ => panic!("chunk document"),
        })
        .collect();
    let mut restored = Ledger::new();
    restored.restore_from_kv(&docs).unwrap();
    assert_eq!(restored.logs["a"], ledger.logs["a"]);
    assert_eq!(restored.logs["b"], ledger.logs["b"]);
}

#[test]
fn repeated_log_rollover_retains_every_sealed_chunk_and_exact_positions() {
    let mut ledger = Ledger::with_chunk_target(0);
    let plan = ledger
        .prepare_sequence_bounded((0..4).map(|n| ("one".into(), json!(n))).collect(), 4096)
        .unwrap();
    assert_eq!(
        plan.positions().collect::<Vec<_>>(),
        vec![("one", 0), ("one", 1), ("one", 2), ("one", 3)]
    );
    let writes = plan
        .authoritative_replay_writes(OwnerId::new("scenes").unwrap())
        .unwrap();
    assert_eq!(writes.len(), 4);
    for (n, write) in writes.iter().enumerate() {
        let ReplayWrite::LedgerChunkSet { write } = write else {
            panic!("ledger write");
        };
        assert_eq!(write.chunk().chunk, n as u64);
        assert!(write.chunk().sealed);
        assert_eq!(write.chunk().entries[0].seq, n as u64);
        assert_eq!(write.chunk().entries[0].value, json!(n));
    }
    let installed = ledger.bind_prepared_sequence(plan).unwrap().apply();
    let docs: Vec<_> = installed
        .iter()
        .map(|write| match write {
            ModuleWrite::JsonSet { key, value } => {
                (key.clone(), serde_json::to_string(value).unwrap())
            }
            _ => panic!("chunk document"),
        })
        .collect();
    let mut restored = Ledger::with_chunk_target(0);
    restored.restore_from_kv(&docs).unwrap();
    assert_eq!(restored, ledger);
    assert_eq!(ledger.next_seq("one"), 4);
}

#[test]
fn a_changed_second_tail_or_seal_policy_refuses_every_selected_image() {
    for policy in [false, true] {
        let mut ledger = Ledger::new();
        let plan = ledger.prepare_sequence_bounded(entries(), 4096).unwrap();
        if policy {
            ledger.chunk_target_bytes = 0;
        } else {
            append(&mut ledger, "b", json!("late"));
        }
        let before = ledger.clone();
        assert!(matches!(
            ledger.bind_prepared_sequence(plan),
            Err(LedgerSequenceError::StalePrestate)
        ));
        assert_eq!(ledger, before);
        assert_eq!(ledger.next_seq("a"), 0);
    }
}

#[test]
fn late_malformed_or_oversized_selected_tail_is_bounded_before_comparison() {
    for deep in [false, true] {
        let mut ledger = Ledger::new();
        append(&mut ledger, "b", json!("original"));
        let plan = ledger.prepare_sequence_bounded(entries(), 4096).unwrap();
        let mut changed = json!("x".repeat(8192));
        if deep {
            changed = json!(null);
            for _ in 0..crate::MAX_DOC_DEPTH + 8 {
                changed = json!([changed]);
            }
        }
        ledger.logs.get_mut("b").unwrap().open_entries[0].value = changed;
        assert_eq!(
            ledger.check_prepared_sequence(&plan),
            Err(LedgerSequenceError::InputLimit)
        );
        assert!(matches!(
            ledger.bind_prepared_sequence(plan),
            Err(LedgerSequenceError::InputLimit)
        ));
        assert_eq!(ledger.next_seq("a"), 0);
        assert_eq!(ledger.next_seq("b"), 1);
    }
}

#[test]
fn limits_and_bad_late_inputs_never_publish_a_prepared_prefix() {
    let mut ledger = Ledger::new();
    append(&mut ledger, "unrelated", json!("x".repeat(8192)));
    let before = ledger.clone();
    assert!(matches!(
        ledger.prepare_sequence_bounded(vec![("a".into(), json!(0)); 5], 4096),
        Err(LedgerSequenceError::TooMany)
    ));
    let mut bad_id = entries();
    bad_id[3].0.clear();
    assert!(matches!(
        ledger.prepare_sequence_bounded(bad_id, 4096),
        Err(LedgerSequenceError::Append(LedgerAppendError::EmptyLogId))
    ));
    let mut deep = json!(null);
    for _ in 0..crate::MAX_DOC_DEPTH + 8 {
        deep = json!([deep]);
    }
    let mut bad_value = entries();
    bad_value[3].1 = deep;
    assert!(matches!(
        ledger.prepare_sequence_bounded(bad_value, 4096),
        Err(LedgerSequenceError::Append(LedgerAppendError::InputLimit))
    ));
    assert_eq!(ledger, before);
    let empty = ledger.prepare_sequence_bounded(vec![], 0).unwrap();
    assert_eq!(empty.positions().count(), 0);
    assert!(
        empty
            .authoritative_replay_writes(OwnerId::new("scenes").unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(
        ledger
            .bind_prepared_sequence(empty)
            .unwrap()
            .apply()
            .is_empty()
    );
    assert_eq!(ledger, before);
}
