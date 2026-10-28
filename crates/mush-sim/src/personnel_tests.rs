#![cfg(test)]

use super::*;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn batch(crossings: Vec<Crossing>) -> TestResult<CrossingBatch> {
    let prepared_at = ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 123,
    };
    let before = DefinitionDigest::for_canonical_value("capture.test.before", &json!({}))?;
    let after = DefinitionDigest::for_canonical_value("capture.test.after", &json!({}))?;
    let plan = DefinitionDigest::for_canonical_value("capture.test.plan", &json!({}))?;
    let capture = capture_digest(&prepared_at, &before, &after, &crossings)?;
    Ok(CrossingBatch {
        format: 1,
        commit: CommitToken::from_bytes([3; 16]),
        operator: ObjId(1),
        root: ObjId(22),
        prepared_at,
        plan_digest: plan,
        physical_digest: combined_digest(&plan, &capture)?,
        before_ancestry_digest: before,
        after_ancestry_digest: after,
        coverage: Coverage::UncertifiedLegacyPaths,
        crossings,
    })
}

fn crossing(character: u32, carrier: u32, direction: CrossingDirection) -> Crossing {
    Crossing {
        character: ObjId(character),
        carrier: ObjId(carrier),
        direction,
    }
}

#[test]
fn personnel_capture_codec_requires_explicit_uncertified_coverage_and_exact_fields() -> TestResult {
    let original = batch(vec![crossing(10, 22, CrossingDirection::Enter)])?;
    let value = encoded(&original)?;
    check_eq!(decode(&value)?, original);
    for field in [
        "coverage",
        "prepared_at",
        "before_ancestry_digest",
        "plan_digest",
    ] {
        let mut changed = value.clone();
        changed.as_object_mut().ok_or("object")?.remove(field);
        check!(decode(&changed).is_err());
    }
    for coverage in [
        json!(null),
        json!("certified"),
        json!("eligible"),
        json!(true),
    ] {
        let mut changed = value.clone();
        changed["coverage"] = coverage;
        check!(decode(&changed).is_err());
    }
    let mut changed = value.clone();
    changed["trusted"] = json!(true);
    check!(decode(&changed).is_err());
    let mut changed = value;
    changed["crossings"][0]["eligible"] = json!(true);
    check!(decode(&changed).is_err());
    check!(decode(&encoded(&batch(Vec::new())?)?).is_ok());
    Ok(())
}

#[test]
fn personnel_capture_digest_covers_clock_ancestry_crossings_and_plan() -> TestResult {
    let value = encoded(&batch(vec![crossing(10, 22, CrossingDirection::Enter)])?)?;
    for changed_value in [
        {
            let mut changed = value.clone();
            changed["prepared_at"]["at"] = json!(124);
            changed
        },
        {
            let mut changed = value.clone();
            changed["prepared_at"]["clock"]["generation"] = json!(2);
            changed
        },
        {
            let mut changed = value.clone();
            changed["crossings"][0]["direction"] = json!("exit");
            changed
        },
        {
            let mut changed = value.clone();
            changed["crossings"][0]["character"] = json!(11);
            changed
        },
        {
            let mut changed = value.clone();
            changed["after_ancestry_digest"] = changed["before_ancestry_digest"].clone();
            changed
        },
        {
            let mut changed = value.clone();
            changed["plan_digest"] = changed["before_ancestry_digest"].clone();
            changed
        },
    ] {
        check!(decode(&changed_value).is_err());
    }
    Ok(())
}

#[test]
fn personnel_capture_counts_crossing_pairs_and_refuses_duplicates_and_non_world_clocks()
-> TestResult {
    // 128 people transferring between carriers consume all 256 pair slots.
    let rows: Vec<_> = (100..228)
        .flat_map(|actor| {
            [
                crossing(actor, 10, CrossingDirection::Exit),
                crossing(actor, 11, CrossingDirection::Enter),
            ]
        })
        .collect();
    let full = batch(rows.clone())?;
    full.validate()?;
    let mut too_many = rows;
    too_many.push(crossing(228, 11, CrossingDirection::Enter));
    check!(
        capture_digest(
            &full.prepared_at,
            &full.before_ancestry_digest,
            &full.after_ancestry_digest,
            &too_many
        )
        .is_err()
    );
    for rows in [
        vec![
            crossing(10, 22, CrossingDirection::Enter),
            crossing(10, 22, CrossingDirection::Enter),
        ],
        vec![
            crossing(10, 22, CrossingDirection::Enter),
            crossing(10, 22, CrossingDirection::Exit),
        ],
        vec![
            crossing(11, 22, CrossingDirection::Enter),
            crossing(10, 22, CrossingDirection::Enter),
        ],
        vec![crossing(22, 22, CrossingDirection::Enter)],
    ] {
        check!(
            capture_digest(
                &full.prepared_at,
                &full.before_ancestry_digest,
                &full.after_ancestry_digest,
                &rows
            )
            .is_err()
        );
    }
    for clock in [ClockRef::Utc, ClockRef::WorldIc { generation: 0 }] {
        let stamp = ClockStamp { clock, at: 123 };
        check!(
            capture_digest(
                &stamp,
                &full.before_ancestry_digest,
                &full.after_ancestry_digest,
                &[]
            )
            .is_err()
        );
    }
    Ok(())
}
