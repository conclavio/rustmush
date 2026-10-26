use std::collections::BTreeSet;

use mush_core::{ClockRef, ClockStamp, ObjId};
use serde_json::{Value, json};

use super::*;
use crate::{Envelope, RevisionKey};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn at(value: i64) -> Result<WorldIcSeconds, ValueError> {
    WorldIcSeconds::from_stamp(&ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: value,
    })
}

fn revision() -> Result<Revision, crate::RollError> {
    let source = "private-source-marker";
    let envelope = Envelope {
        schema: json!({}),
        presets: json!({}),
        profiles: json!({"line":{"threshold":10}}),
        rules: json!({}),
        declared_class: "I".into(),
        mind_abi: "test=1".into(),
    };
    Ok(Revision {
        format: crate::FORMAT,
        provenance: None,
        reference: RevisionRef {
            key: RevisionKey {
                character: ObjId(7),
                mind: 3,
                number: 2,
            },
            digest: envelope.digest(source)?,
        },
        source: crate::source_digest(source),
        envelope,
        notes: "private-annotation-marker".into(),
    })
}

fn replace(value: &mut Value, key: &str, new: Value) -> TestResult {
    value
        .as_object_mut()
        .ok_or("expected object fixture")?
        .insert(key.into(), new);
    Ok(())
}

#[test]
fn identities_keep_all_bits_and_distinct_key_domains() -> TestResult {
    let raw = u64::MAX;
    let charter = CharterId::from_object(ObjId(u32::MAX))?;
    let line = LineId::new(raw)?;
    let filed = FiledRevisionId::new(raw)?;
    let licence = LicenceId::new(raw)?;
    let term = TermGeneration::new(raw)?;
    let encoded = "\"18446744073709551615\"";
    check_eq!(serde_json::to_string(&charter)?, "\"4294967295\"");
    check_eq!(
        serde_json::from_str::<CharterId>("\"4294967295\"")?,
        charter
    );
    check_eq!(charter.object(), ObjId(u32::MAX));
    check!(serde_json::from_str::<CharterId>("\"4294967296\"").is_err());
    check_eq!(serde_json::from_str::<LineId>(encoded)?, line);
    check_eq!(serde_json::from_str::<FiledRevisionId>(encoded)?, filed);
    check_eq!(serde_json::from_str::<LicenceId>(encoded)?, licence);
    check_eq!(serde_json::from_str::<TermGeneration>(encoded)?, term);
    let keys = [
        CharterId::new(1)?.key(),
        LineId::new(1)?.key(),
        FiledRevisionId::new(1)?.key(),
        LicenceId::new(1)?.key(),
        TermGeneration::new(1)?.key(),
    ];
    check_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 5);
    check!(keys.iter().all(|key| key.len() <= 24));
    check_eq!(CharterId::new(1)?.key(), "charter/0000000000000001");
    check_eq!(line.get(), raw);
    check_eq!(filed.get(), raw);
    check_eq!(licence.get(), raw);
    check_eq!(term.get(), raw);
    Ok(())
}

#[test]
fn identity_decoders_reject_zero_aliases_overflow_and_numeric_json() {
    for encoded in [
        "\"\"",
        "\"0\"",
        "\"01\"",
        "\"+1\"",
        "\"-1\"",
        "\" 1\"",
        "\"1 \"",
        "\"١\"",
        "\"1.0\"",
        "\"18446744073709551616\"",
        "\"111111111111111111111\"",
        "1",
        "null",
        "[]",
        "{}",
    ] {
        assert!(
            serde_json::from_str::<CharterId>(encoded).is_err(),
            "{encoded}"
        );
        assert!(
            serde_json::from_str::<LineId>(encoded).is_err(),
            "{encoded}"
        );
        assert!(
            serde_json::from_str::<FiledRevisionId>(encoded).is_err(),
            "{encoded}"
        );
        assert!(
            serde_json::from_str::<LicenceId>(encoded).is_err(),
            "{encoded}"
        );
        assert!(
            serde_json::from_str::<TermGeneration>(encoded).is_err(),
            "{encoded}"
        );
    }
    assert_eq!(CharterId::new(0), Err(ValueError::Identifier));
    assert_eq!(
        CharterId::from_object(ObjId(0)),
        Err(ValueError::Identifier)
    );
    assert_eq!(LineId::new(0), Err(ValueError::Identifier));
    assert_eq!(FiledRevisionId::new(0), Err(ValueError::Identifier));
    assert_eq!(LicenceId::new(0), Err(ValueError::Identifier));
    assert_eq!(TermGeneration::new(0), Err(ValueError::Identifier));
}

#[test]
fn revision_identity_has_no_source_envelope_annotations_or_holder() -> TestResult {
    let original = revision()?;
    let identity = RevisionIdentity::capture(&original)?;
    let text = serde_json::to_string(&identity)?;
    check_eq!(serde_json::from_str::<RevisionIdentity>(&text)?, identity);
    for forbidden in [
        "private-source-marker",
        "private-annotation-marker",
        "schema",
        "profiles",
        "holder",
    ] {
        check!(!text.contains(forbidden), "{forbidden}");
    }
    check_eq!(identity.revision(), original.reference());
    check_eq!(identity.source(), original.source());
    let mut annotations = original.clone();
    annotations.notes = "different private notes".into();
    check!(identity.matches(&annotations)?);
    let mut different_record = original.clone();
    different_record.reference.key.number += 1;
    check!(!identity.matches(&different_record)?);
    let mut different_bytes = original.clone();
    different_bytes.source = crate::source_digest("other bytes");
    check!(!identity.matches(&different_bytes)?);
    let mut different_envelope = original;
    different_envelope.reference.digest = crate::source_digest("other revision identity");
    check!(!identity.matches(&different_envelope)?);
    Ok(())
}

#[test]
fn revision_decode_refuses_bad_versions_zero_keys_and_unknown_fields() -> TestResult {
    let original = revision()?;
    let identity = RevisionIdentity::capture(&original)?;
    for (key, value) in [
        ("format", json!(0)),
        ("format", json!(2)),
        ("holder", json!(7)),
    ] {
        let mut wire = serde_json::to_value(identity)?;
        replace(&mut wire, key, value)?;
        check!(serde_json::from_value::<RevisionIdentity>(wire).is_err());
    }
    for (mind, number) in [(0, 1), (1, 0)] {
        let mut invalid = original.clone();
        invalid.reference.key.mind = mind;
        invalid.reference.key.number = number;
        check_eq!(
            RevisionIdentity::capture(&invalid),
            Err(ValueError::Revision)
        );
        let wire = json!({"format":1,"revision":invalid.reference(),"source":invalid.source()});
        check!(serde_json::from_value::<RevisionIdentity>(wire).is_err());
    }
    let text = serde_json::to_string(&identity)?;
    let duplicate = text.replacen("\"format\":1", "\"format\":1,\"format\":1", 1);
    check!(serde_json::from_str::<RevisionIdentity>(&duplicate).is_err());
    let mut absent = serde_json::to_value(identity)?;
    absent
        .as_object_mut()
        .ok_or("expected object")?
        .remove("source");
    check!(serde_json::from_value::<RevisionIdentity>(absent).is_err());
    Ok(())
}

#[test]
fn paid_capacity_is_bounded_but_owned_dispatch_has_no_count() -> TestResult {
    check_eq!(PaidSeats::new(0), Err(ValueError::Seats));
    check_eq!(PaidSeats::new(MAX_PAID_SEATS + 1), Err(ValueError::Seats));
    let seats = PaidSeats::new(MAX_PAID_SEATS)?;
    check_eq!(seats.get(), MAX_PAID_SEATS);
    let paid = Capacity::PaidSeats { seats };
    check_eq!(
        serde_json::to_value(paid)?,
        json!({"kind":"paid_seats","seats":1024})
    );
    check_eq!(
        serde_json::from_value::<Capacity>(serde_json::to_value(paid)?)?,
        paid
    );
    let owned = Capacity::OwnedDispatch {};
    check_eq!(
        serde_json::to_value(owned)?,
        json!({"kind":"owned_dispatch"})
    );
    check_eq!(
        serde_json::from_value::<Capacity>(serde_json::to_value(owned)?)?,
        owned
    );
    Ok(())
}

#[test]
fn capacity_codec_does_not_accept_extra_counts_unknown_arms_or_bypassed_bounds() {
    for value in [
        json!({"kind":"owned_dispatch","seats":1}),
        json!({"kind":"paid_seats","seats":0}),
        json!({"kind":"paid_seats","seats":1025}),
        json!({"kind":"paid_seats","seats":65536}),
        json!({"kind":"paid_seats","seats":-1}),
        json!({"kind":"paid_seats","seats":"1"}),
        json!({"kind":"paid_seats","seats":1,"holder":7}),
        json!({"kind":"paid_seats"}),
        json!({"kind":"unlimited"}),
    ] {
        assert!(
            serde_json::from_value::<Capacity>(value.clone()).is_err(),
            "{value}"
        );
    }
    assert!(
        serde_json::from_str::<Capacity>("{\"kind\":\"paid_seats\",\"seats\":1,\"seats\":2}")
            .is_err()
    );
}

#[test]
fn clock_kind_is_checked_without_persisting_rate_generation_as_a_term() -> TestResult {
    let first = at(500)?;
    let next_generation = WorldIcSeconds::from_stamp(&ClockStamp {
        clock: ClockRef::WorldIc { generation: 2 },
        at: 500,
    })?;
    check_eq!(first, next_generation);
    check_eq!(first.get(), 500);
    for clock in [
        ClockRef::Utc,
        ClockRef::WorldIc { generation: 0 },
        ClockRef::SceneTurn {
            scene: 1,
            generation: 1,
        },
    ] {
        check_eq!(
            WorldIcSeconds::from_stamp(&ClockStamp { clock, at: 500 }),
            Err(ValueError::Clock)
        );
    }
    Ok(())
}

#[test]
fn temporal_edges_are_exact_and_never_extend_an_interval() -> TestResult {
    let term = TermWindow::new(at(0)?, at(0)?, at(10)?, at(13)?)?;
    for (now, expected) in [
        (-1, TermPhase::NotEffective),
        (0, TermPhase::Active),
        (9, TermPhase::Active),
        (10, TermPhase::Grace),
        (12, TermPhase::Grace),
        (13, TermPhase::Lapsed),
        (14, TermPhase::Lapsed),
    ] {
        check_eq!(term.phase_at(at(now)?), expected, "{now}");
    }
    let no_grace = TermWindow::new(at(0)?, at(0)?, at(10)?, at(10)?)?;
    check_eq!(no_grace.phase_at(at(10)?), TermPhase::Lapsed);
    Ok(())
}

#[test]
fn grace_issue_preserves_actual_issue_and_original_eligibility_origin() -> TestResult {
    let term = TermWindow::new(at(0)?, at(11)?, at(10)?, at(13)?)?;
    check_eq!(term.eligibility_start().get(), 0);
    check_eq!(term.issued_at().get(), 11);
    check_eq!(term.term_end().get(), 10);
    check_eq!(term.grace_end().get(), 13);
    check_eq!(term.phase_at(at(9)?), TermPhase::NotEffective);
    check_eq!(term.phase_at(at(10)?), TermPhase::NotEffective);
    check_eq!(term.phase_at(at(11)?), TermPhase::Grace);
    // Late issue must not hide an original term exceeding the audit limit (the
    // owner bound, 17g C0: a window type carries owner and paid terms alike).
    check!(
        TermWindow::new(
            at(0)?,
            at(MAX_OWNER_TERM_SECONDS + 2)?,
            at(MAX_OWNER_TERM_SECONDS + 1)?,
            at(MAX_OWNER_TERM_SECONDS + 3)?
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn a_future_interval_and_an_unissued_interval_are_both_ineffective() -> TestResult {
    let issued_early = TermWindow::new(at(10)?, at(5)?, at(20)?, at(23)?)?;
    check_eq!(issued_early.phase_at(at(9)?), TermPhase::NotEffective);
    check_eq!(issued_early.phase_at(at(10)?), TermPhase::Active);
    let issued_late = TermWindow::new(at(0)?, at(5)?, at(20)?, at(23)?)?;
    check_eq!(issued_late.phase_at(at(4)?), TermPhase::NotEffective);
    check_eq!(issued_late.phase_at(at(5)?), TermPhase::Active);
    Ok(())
}

#[test]
fn term_and_grace_limits_have_no_extra_second_or_empty_eligible_tail() -> TestResult {
    // The window's own bound is the owner bound (17g C0); a paid record holds
    // its window to `MAX_TERM_SECONDS` itself (`paid/tests.rs`).
    let end = MAX_OWNER_TERM_SECONDS;
    let grace = end + MAX_GRACE_SECONDS;
    check!(TermWindow::new(at(0)?, at(0)?, at(end)?, at(grace)?).is_ok());
    for (start, issue, end, grace) in [
        (0, 0, MAX_OWNER_TERM_SECONDS + 1, MAX_OWNER_TERM_SECONDS + 1),
        (0, 0, 1, 1 + MAX_GRACE_SECONDS + 1),
        (1, 1, 1, 2),
        (2, 2, 1, 3),
        (0, 0, 2, 1),
        (0, 3, 2, 3),
        (0, 4, 2, 3),
    ] {
        check_eq!(
            TermWindow::new(at(start)?, at(issue)?, at(end)?, at(grace)?),
            Err(ValueError::Term)
        );
    }
    Ok(())
}

#[test]
fn timestamp_extremes_use_checked_arithmetic_and_allow_valid_negative_dates() -> TestResult {
    let lower = TermWindow::new(
        at(i64::MIN)?,
        at(i64::MIN)?,
        at(i64::MIN + 1)?,
        at(i64::MIN + 1)?,
    )?;
    check_eq!(lower.phase_at(at(i64::MIN)?), TermPhase::Active);
    let upper = TermWindow::new(
        at(i64::MAX - 1)?,
        at(i64::MAX - 1)?,
        at(i64::MAX)?,
        at(i64::MAX)?,
    )?;
    check_eq!(upper.phase_at(at(i64::MAX)?), TermPhase::Lapsed);
    check_eq!(
        TermWindow::new(at(i64::MIN)?, at(0)?, at(i64::MAX)?, at(i64::MAX)?),
        Err(ValueError::Term)
    );
    check_eq!(
        TermWindow::new(
            at(i64::MIN)?,
            at(i64::MIN)?,
            at(i64::MIN + 1)?,
            at(i64::MAX)?
        ),
        Err(ValueError::Term)
    );
    Ok(())
}

#[test]
fn term_codec_rechecks_invariants_and_rejects_unknown_missing_duplicate_fields() -> TestResult {
    let term = TermWindow::new(at(0)?, at(11)?, at(10)?, at(13)?)?;
    let text = serde_json::to_string(&term)?;
    check_eq!(serde_json::from_str::<TermWindow>(&text)?, term);
    for (key, value) in [
        ("format", json!(0)),
        ("format", json!(2)),
        ("allow_parent_loss", json!(true)),
        ("term_end", json!(0)),
        ("issued_at", json!(13)),
        ("eligibility_start", json!("0")),
        ("grace_end", json!(MAX_GRACE_SECONDS + 11)),
    ] {
        let mut wire = serde_json::to_value(term)?;
        replace(&mut wire, key, value)?;
        check!(serde_json::from_value::<TermWindow>(wire).is_err(), "{key}");
    }
    let duplicate = text.replacen("\"format\":1", "\"format\":1,\"format\":1", 1);
    check!(serde_json::from_str::<TermWindow>(&duplicate).is_err());
    let mut absent = serde_json::to_value(term)?;
    absent
        .as_object_mut()
        .ok_or("expected object")?
        .remove("eligibility_start");
    check!(serde_json::from_value::<TermWindow>(absent).is_err());
    Ok(())
}
