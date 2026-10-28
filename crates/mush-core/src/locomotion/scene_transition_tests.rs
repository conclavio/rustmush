#![cfg(test)]
//! Closed protocol codecs and claims; Store tests exercise the real two-owner ACK.
use super::*;
use crate::authoritative::AuthoritativeLedgerWrite;
use crate::ledger::{LedgerChunk, LedgerEntry};

fn digest(byte: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([byte; 32])
}
fn reference(owner: &str, kind: &str, id: &str) -> DefinitionRef {
    expected(
        owner,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind).unwrap(),
            DefinitionId::new(id).unwrap(),
            1,
            json!({"fixture":true}),
        )
        .unwrap(),
    )
    .unwrap()
}
fn object(id: u32) -> LocomotionObjectDigest {
    LocomotionObjectDigest {
        object: ObjId(id),
        before: digest(id as u8),
        after: digest(id as u8 + 20),
    }
}
fn record() -> Record {
    Record {
        format: 1,
        commit: CommitToken::from_bytes([7; 16]),
        action: expected("sim", action_descriptor().unwrap()).unwrap(),
        participant: reference("sim", "owner_participant", SIM_PARTICIPANT),
        scenes_participant: expected("scenes", scenes_participant_descriptor().unwrap()).unwrap(),
        observer: expected("scenes", observer_descriptor().unwrap()).unwrap(),
        actor: ObjId(3),
        source: Some(ObjId(4)),
        exit: Some(ObjId(5)),
        destination: ObjId(6),
        prepared_at: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 1234,
        },
        at_utc: 77,
        plan_digest: digest(8),
        capture_digest: digest(9),
        scenes_capture_digest: digest(10),
        appends: vec![
            SceneLedgerAppendRef {
                scene: 20,
                sequence: 4,
                event_digest: digest(11),
            },
            SceneLedgerAppendRef {
                scene: 10,
                sequence: 7,
                event_digest: digest(12),
            },
        ],
        scenes_writes_digest: digest(13),
        objects: vec![object(3), object(4), object(6)],
    }
}
fn wire(row: &Record) -> Value {
    serde_json::to_value(row).unwrap()
}
fn write(owner: &str, log: &str, seq: u64, event: Value) -> ReplayWrite {
    ReplayWrite::LedgerChunkSet {
        write: AuthoritativeLedgerWrite::new(
            OwnerId::new(owner).unwrap(),
            log,
            LedgerChunk {
                chunk: 0,
                sealed: false,
                entries: vec![LedgerEntry { seq, value: event }],
            },
        )
        .unwrap(),
    }
}
fn claim(scene: u64, seq: u64, event: &Value) -> SceneLedgerAppendRef {
    SceneLedgerAppendRef {
        scene,
        sequence: seq,
        event_digest: scene_event_digest(event).unwrap(),
    }
}

#[test]
fn joint_receipt_preserves_order_and_has_a_separate_protocol() {
    let row = record();
    assert_eq!(decode(&wire(&row)).unwrap(), row);
    assert_eq!(row.appends[0].scene, 20); // Joined then Left, not sorted by log ID.
    let mut old = row.clone();
    old.action = expected(
        "sim",
        super::super::transition::action_descriptor().unwrap(),
    )
    .unwrap();
    assert!(decode(&wire(&old)).is_err());
    for key in [
        "source",
        "exit",
        "observer",
        "scenes_participant",
        "appends",
        "scenes_capture_digest",
        "scenes_writes_digest",
    ] {
        let mut value = wire(&row);
        value.as_object_mut().unwrap().remove(key);
        assert!(decode(&value).is_err(), "accepted missing {key}");
    }
    let mut value = wire(&row);
    value["extra"] = json!(1);
    assert!(decode(&value).is_err());
    let mut value = wire(&row);
    value["appends"][0]["extra"] = json!(1);
    assert!(decode(&value).is_err());
}

#[test]
fn full_scenes_definition_and_observer_digests_are_required() {
    for field in ["action", "scenes_participant", "observer"] {
        let mut value = wire(&record());
        value[field]["digest"] = json!(digest(66));
        assert!(decode(&value).is_err(), "accepted forged {field} digest");
    }
    let mut row = record();
    row.participant = reference("scenes", "owner_participant", SIM_PARTICIPANT);
    assert!(decode(&wire(&row)).is_err());
    let mut row = record();
    row.scenes_participant = reference("scenes", "owner_participant", SCENES_PARTICIPANT);
    assert!(decode(&wire(&row)).is_err());
}

#[test]
fn empty_scene_observation_and_exit_self_loop_have_no_fake_effects() {
    let mut row = record();
    row.appends.clear();
    assert_eq!(decode(&wire(&row)).unwrap(), row);
    row.destination = row.source.unwrap();
    row.objects.clear();
    assert_eq!(decode(&wire(&row)).unwrap(), row);
    row.appends.push(record().appends.remove(0));
    assert!(decode(&wire(&row)).is_err());
    row.appends.clear();
    row.exit = None;
    assert!(decode(&wire(&row)).is_err());
    assert!(verify_scene_write_claims(&[], &[]).is_ok());
    assert!(verify_scene_write_claims(&[], &[write("scenes", "1", 0, json!({}))]).is_err());
}

#[test]
fn optional_source_and_exit_require_explicit_null_and_exact_object_rows() {
    let mut row = record();
    row.source = None;
    row.exit = None;
    row.objects = vec![object(3), object(6)];
    assert!(decode(&wire(&row)).is_err()); // An unlocated actor cannot leave a second transcript.
    row.appends.truncate(1);
    assert_eq!(decode(&wire(&row)).unwrap(), row);
    for missing in ["source", "exit"] {
        let mut value = wire(&row);
        value.as_object_mut().unwrap().remove(missing);
        assert!(decode(&value).is_err());
    }
    row.objects.push(object(8));
    assert!(decode(&wire(&row)).is_err());
}

#[test]
fn append_bound_and_unique_log_rule_refuse_before_join() {
    let mut row = record();
    row.appends.push(SceneLedgerAppendRef {
        scene: 30,
        sequence: 0,
        event_digest: digest(1),
    });
    assert!(decode(&wire(&row)).is_err());
    let mut row = record();
    row.appends[1].scene = row.appends[0].scene;
    assert!(decode(&wire(&row)).is_err());
    let mut value = wire(&record());
    value["appends"][0]["sequence"] = json!(-1);
    assert!(decode(&value).is_err());
    let mut value = wire(&record());
    value["appends"][0]["event_digest"] = json!("AA".repeat(32));
    assert!(decode(&value).is_err());
}

#[test]
fn claims_bind_real_namespace_log_sequence_event_and_order() {
    let joined = json!({"kind":"joined","at":101,"witnesses":[4,3]});
    let left = json!({"kind":"left","at":101,"witnesses":[8]});
    let claims = vec![claim(20, 4, &joined), claim(10, 7, &left)];
    let writes = vec![
        write("scenes", "20", 4, joined.clone()),
        write("scenes", "10", 7, left.clone()),
    ];
    assert!(verify_scene_write_claims(&claims, &writes).is_ok());
    for wrong in [
        vec![writes[1].clone(), writes[0].clone()],
        vec![write("other", "20", 4, joined.clone()), writes[1].clone()],
        vec![write("scenes", "020", 4, joined.clone()), writes[1].clone()],
        vec![write("scenes", "20", 5, joined.clone()), writes[1].clone()],
        vec![write("scenes", "20", 4, left), writes[1].clone()],
        vec![writes[0].clone()],
    ] {
        assert!(verify_scene_write_claims(&claims, &wrong).is_err());
    }
    let mut changed = joined;
    changed["witnesses"] = json!([3, 4]);
    assert_ne!(
        scene_event_digest(&changed).unwrap(),
        claims[0].event_digest
    );
}

#[test]
fn writes_digest_covers_existing_chunk_bytes_not_just_last_event() {
    let event = json!({"kind":"joined"});
    let claims = vec![claim(1, 1, &event)];
    let build = |earlier| {
        vec![ReplayWrite::LedgerChunkSet {
            write: AuthoritativeLedgerWrite::new(
                OwnerId::new("scenes").unwrap(),
                "1",
                LedgerChunk {
                    chunk: 0,
                    sealed: false,
                    entries: vec![
                        LedgerEntry {
                            seq: 0,
                            value: json!(earlier),
                        },
                        LedgerEntry {
                            seq: 1,
                            value: event.clone(),
                        },
                    ],
                },
            )
            .unwrap(),
        }]
    };
    assert_ne!(
        verify_scene_write_claims(&claims, &build("old")).unwrap(),
        verify_scene_write_claims(&claims, &build("changed")).unwrap()
    );
}

#[test]
fn scene_capture_digest_is_bounded_and_separate_from_event_domain() {
    let value = json!({"actor":3,"from":null,"to":6,"at":77,"events":[]});
    assert_ne!(
        scene_event_digest(&value).unwrap(),
        scene_capture_digest(&value).unwrap()
    );
    assert!(scene_capture_digest(&json!("x".repeat(MAX_SCENE_CAPTURE_BYTES))).is_err());
    let mut row = record();
    row.prepared_at.clock = ClockRef::WorldIc { generation: 0 };
    assert!(decode(&wire(&row)).is_err());
    let mut row = record();
    row.prepared_at.clock = ClockRef::Utc;
    assert!(decode(&wire(&row)).is_err());
}

struct RequestObserver;
impl crate::GameModule for RequestObserver {
    fn name(&self) -> &'static str {
        "sim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.require_current_moved_observer(
            &expected("scenes", observer_descriptor()?).map_err(bad)?,
        )
    }
}
struct ScenesObserver(bool);
impl crate::GameModule for ScenesObserver {
    fn name(&self) -> &'static str {
        "scenes"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if self.0 {
            reg.subscribe_moved(
                OBSERVER,
                1,
                std::sync::Arc::new(|_: &mut crate::Ctx<'_>, _: &crate::WorldEvent| {}),
            )?;
        } else {
            reg.retain_moved_observer(OBSERVER, 1)?;
        }
        Ok(())
    }
}
#[test]
fn dependency_requires_actual_current_callback_in_either_registration_order() {
    for reversed in [false, true] {
        let mut modules: Vec<Box<dyn crate::GameModule>> =
            vec![Box::new(RequestObserver), Box::new(ScenesObserver(true))];
        if reversed {
            modules.reverse();
        }
        assert!(crate::Engine::compose_modules(crate::Registry::new(), modules).is_ok());
    }
    assert!(
        crate::Engine::compose_modules(
            crate::Registry::new(),
            vec![Box::new(RequestObserver), Box::new(ScenesObserver(false))]
        )
        .is_err()
    );
    assert!(
        crate::Engine::compose_modules(crate::Registry::new(), vec![Box::new(RequestObserver)])
            .is_err()
    );
}

struct ReceiptFamily;
impl crate::GameModule for ReceiptFamily {
    fn name(&self) -> &'static str {
        "sim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        register_records(reg)?;
        Ok(())
    }
}
#[test]
fn receipt_audit_grant_is_exact_and_missing_auditor_refuses_composition() {
    let engine = crate::Engine::with_modules(
        World::new_minimal(),
        crate::Registry::new(),
        vec![Box::new(ReceiptFamily), Box::new(ScenesObserver(true))],
    )
    .unwrap();
    let grants = engine
        .registry
        .state_families()
        .unwrap()
        .family_read_grants();
    assert_eq!(
        grants,
        vec![(
            OwnerId::new("sim").unwrap(),
            StateFamilyId::new(FAMILY).unwrap(),
            OwnerId::new("scenes").unwrap()
        )]
    );
    assert!(
        crate::Engine::compose_modules(crate::Registry::new(), vec![Box::new(ReceiptFamily)])
            .is_err()
    );
}
