#![cfg(test)]
//! Strict receipt codec coverage; production coordinator tests verify the
//! acknowledged Sim/Core pair and boot-time exact registered participant join.
use super::*;

fn reference(owner: &str, kind: &str, id: &str, version: u32) -> DefinitionRef {
    crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(owner).unwrap(),
            DefinitionDescriptor::new(
                DefinitionKind::new(kind).unwrap(),
                DefinitionId::new(id).unwrap(),
                version,
                json!({"fixture":"locomotion_codec"}),
            )
            .unwrap(),
        )
        .unwrap()
}

fn object(id: u32) -> LocomotionObjectDigest {
    LocomotionObjectDigest {
        object: ObjId(id),
        before: DefinitionDigest::from_bytes([id as u8; 32]),
        after: DefinitionDigest::from_bytes([id as u8 + 20; 32]),
    }
}

fn record() -> Record {
    Record {
        format: 1,
        commit: CommitToken::from_bytes([7; 16]),
        action: crate::DefinitionRegistryBuilder::new()
            .register(OwnerId::new("sim").unwrap(), action_descriptor().unwrap())
            .unwrap(),
        participant: reference("sim", "owner_participant", PARTICIPANT, 1),
        actor: ObjId(3),
        source: Some(ObjId(4)),
        exit: Some(ObjId(5)),
        destination: ObjId(6),
        prepared_at: ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 1234,
        },
        at_utc: 77,
        plan_digest: DefinitionDigest::from_bytes([8; 32]),
        capture_digest: DefinitionDigest::from_bytes([9; 32]),
        objects: vec![object(3), object(4), object(6)],
    }
}

fn wire(row: &Record) -> Value {
    serde_json::to_value(row).unwrap()
}

#[test]
fn canonical_receipt_round_trips_with_exact_sorted_rows() {
    let row = record();
    assert_eq!(decode(&wire(&row)).unwrap(), row);
    let mut wrong = row.clone();
    wrong.objects.swap(0, 1);
    assert!(decode(&wire(&wrong)).is_err());
    let mut wrong = row.clone();
    wrong.objects.pop();
    assert!(decode(&wire(&wrong)).is_err());
    let mut wrong = row.clone();
    wrong.objects.push(object(7));
    assert!(decode(&wire(&wrong)).is_err());
    let mut wrong = row.clone();
    wrong.objects[1] = object(3);
    assert!(decode(&wire(&wrong)).is_err());
    let mut wrong = row;
    wrong.objects[2] = object(7);
    assert!(decode(&wire(&wrong)).is_err());
}

#[test]
fn absent_source_home_requires_explicit_nulls_and_exact_two_rows() {
    let mut row = record();
    row.source = None;
    row.exit = None;
    row.objects = vec![object(3), object(6)];
    assert_eq!(decode(&wire(&row)).unwrap(), row);
    for missing in ["source", "exit"] {
        let mut value = wire(&row);
        value.as_object_mut().unwrap().remove(missing);
        assert!(decode(&value).is_err(), "missing {missing} was normalized");
    }
    row.objects.insert(1, object(4));
    assert!(decode(&wire(&row)).is_err());
}

#[test]
fn ordinary_home_and_self_loop_exit_are_distinct_shapes() {
    let mut home = record();
    home.exit = None;
    assert_eq!(decode(&wire(&home)).unwrap(), home);
    let mut loop_exit = record();
    loop_exit.destination = loop_exit.source.unwrap();
    loop_exit.objects.clear();
    assert_eq!(decode(&wire(&loop_exit)).unwrap(), loop_exit);
    let mut wrong = loop_exit.clone();
    wrong.objects.push(object(3));
    assert!(decode(&wire(&wrong)).is_err());
    loop_exit.exit = None; // AlreadyHome never produces a moving admission.
    assert!(decode(&wire(&loop_exit)).is_err());
}

#[test]
fn action_and_participant_identity_shapes_are_strict() {
    let mut row = record();
    for action in [
        reference("sim", "action", ACTION, 1), // correct key, wrong digest
        reference("sim", "action", ACTION, 2),
        reference("other", "action", ACTION, 1),
        reference("sim", "action", "other", 1),
    ] {
        row.action = action;
        assert!(decode(&wire(&row)).is_err());
    }
    for participant in [
        reference("other", "owner_participant", PARTICIPANT, 1),
        reference("sim", "action", PARTICIPANT, 1),
        reference("sim", "owner_participant", "other", 1),
        reference("sim", "owner_participant", PARTICIPANT, 2),
    ] {
        let mut row = record();
        row.participant = participant;
        assert!(decode(&wire(&row)).is_err());
    }
    let mut value = wire(&record());
    value["action"]["version"] = json!(0);
    assert!(decode(&value).is_err());
}

#[test]
fn unknown_fields_missing_required_values_and_wrong_types_refuse() {
    for key in [
        "format",
        "commit",
        "actor",
        "prepared_at",
        "plan_digest",
        "objects",
    ] {
        let mut value = wire(&record());
        value.as_object_mut().unwrap().remove(key);
        assert!(decode(&value).is_err(), "accepted missing {key}");
    }
    let mut value = wire(&record());
    value["extra"] = json!(true);
    assert!(decode(&value).is_err());
    let mut value = wire(&record());
    value["objects"][0]["extra"] = json!(true);
    assert!(decode(&value).is_err());
    let mut value = wire(&record());
    value["actor"] = json!("3");
    assert!(decode(&value).is_err());
    let mut value = wire(&record());
    value["format"] = json!(2);
    assert!(decode(&value).is_err());
}

#[test]
fn all_digest_positions_require_canonical_lower_hex() {
    for invalid in ["ab".to_owned(), "AB".repeat(32), "zz".repeat(32)] {
        for field in ["plan_digest", "capture_digest"] {
            let mut value = wire(&record());
            value[field] = json!(invalid);
            assert!(decode(&value).is_err());
        }
        for field in ["before", "after"] {
            let mut value = wire(&record());
            value["objects"][0][field] = json!(invalid);
            assert!(decode(&value).is_err());
        }
        for field in ["action", "participant"] {
            let mut value = wire(&record());
            value[field]["digest"] = json!(invalid);
            assert!(decode(&value).is_err());
        }
    }
}

#[test]
fn clock_must_be_exact_nonzero_world_ic_but_signed_time_is_valid() {
    let mut row = record();
    row.prepared_at.at = -1234;
    row.at_utc = -77;
    assert_eq!(decode(&wire(&row)).unwrap(), row);
    for clock in [
        ClockRef::Utc,
        ClockRef::WorldIc { generation: 0 },
        ClockRef::SceneTurn {
            scene: 1,
            generation: 1,
        },
    ] {
        let mut row = record();
        row.prepared_at.clock = clock;
        assert!(decode(&wire(&row)).is_err());
    }
    let mut value = wire(&record());
    value["prepared_at"]["clock"]["extra"] = json!(1);
    assert!(decode(&value).is_err());
    let mut value = wire(&record());
    value["prepared_at"]["at"] = json!("1234");
    assert!(decode(&value).is_err());
    let mut value = wire(&record());
    value["prepared_at"]["clock"]["generation"] = json!(-1);
    assert!(decode(&value).is_err());
}

#[test]
fn impossible_actor_exit_relationships_and_unchanged_rows_refuse() {
    let mut row = record();
    row.source = Some(row.actor);
    row.objects = vec![object(3), object(6)];
    assert!(decode(&wire(&row)).is_err());
    let mut row = record();
    row.source = None;
    row.objects = vec![object(3), object(6)];
    assert!(decode(&wire(&row)).is_err());
    for exit in [ObjId(3), ObjId(4), ObjId(6)] {
        let mut row = record();
        row.exit = Some(exit);
        assert!(decode(&wire(&row)).is_err());
    }
    let mut row = record();
    row.destination = row.actor;
    row.objects = vec![object(3), object(4)];
    assert!(decode(&wire(&row)).is_err());
    let mut row = record();
    row.objects[0].after = row.objects[0].before;
    assert!(decode(&wire(&row)).is_err());
}
