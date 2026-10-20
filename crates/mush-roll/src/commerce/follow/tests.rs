//! Locator arithmetic only. Structural references do not certify or file a
//! revision. The real paired publication/cold story is in Tacsim's update child.
use super::*;
use mush_core::{CommitToken, DefinitionDigest};
use serde_json::json;

fn compatibility(n: u8) -> RatingCompatibility {
    RatingCompatibility::combat_v1(DefinitionDigest::from_bytes([n; 32]))
}
fn entry() -> Entry {
    let revision = crate::commerce::publication_tests::saved().3;
    let rating: RatingRef = serde_json::from_value(json!({
        "job":CommitToken::from_bytes([2;16]),
        "id":DefinitionDigest::from_bytes([3;32]),
        "schema":1,"receipt_digest":DefinitionDigest::from_bytes([4;32]),
    }))
    .unwrap();
    Entry {
        filed: FiledRevisionId::new(1).unwrap(),
        charter: CharterId::new(20).unwrap(),
        line: LineId::new(1).unwrap(),
        sequence: 1,
        at: 100,
        identity: RevisionIdentity::capture(&revision).unwrap(),
        birth_rating: rating,
        blocked: false,
        certificates: BTreeMap::from([(
            compatibility(5).digest(),
            Some(Certificate {
                reference: rating,
                minimum: RatingClass::I,
            }),
        )]),
    }
}
fn line(entry: &Entry) -> LineRecord {
    LineRecord {
        format: if entry.sequence == 1 { 1 } else { 2 },
        id: entry.line,
        charter: entry.charter,
        schema_digest: DefinitionDigest::from_bytes([6; 32]),
        head: entry.filed,
        next_sequence: entry.sequence + 1,
        operation: CommitToken::from_bytes([2; 16]),
    }
}
fn insert(index: &mut Index, entry: Entry) {
    let previous = index.heads.get(&entry.line).map(|head| head.filed);
    let prepared = index
        .prepare(&Change(Kind::Filing { entry, previous }))
        .unwrap();
    index.apply(prepared);
}
fn block(index: &mut Index, filed: FiledRevisionId) {
    let prepared = index.prepare(&Change(Kind::Block(filed))).unwrap();
    index.apply(prepared);
}

#[test]
fn follow_uses_newest_eligible_sequence_at_the_exact_historical_ic_boundary() {
    let first = entry();
    let mut second = first.clone();
    second.filed = FiledRevisionId::new(2).unwrap();
    second.sequence = 2;
    second.at = 200;
    let mut index = Index::default();
    insert(&mut index, first.clone());
    insert(&mut index, second.clone());
    let line = line(&second);
    assert!(
        index
            .select(&line, &compatibility(5), WorldIcSeconds(99))
            .is_err()
    );
    assert_eq!(
        index
            .select(&line, &compatibility(5), WorldIcSeconds(199))
            .unwrap(),
        first.filed
    );
    assert_eq!(
        index
            .select(&line, &compatibility(5), WorldIcSeconds(200))
            .unwrap(),
        second.filed
    );
    // A clock-generation change has no invented ordering for old Filed data:
    // an earlier absolute IC observation makes the future revision ineligible.
    assert_eq!(
        index
            .select(&line, &compatibility(5), WorldIcSeconds(100))
            .unwrap(),
        first.filed
    );
    block(&mut index, second.filed);
    assert_eq!(
        index
            .select(&line, &compatibility(5), WorldIcSeconds(200))
            .unwrap(),
        first.filed
    );
    block(&mut index, first.filed);
    assert!(
        index
            .select(&line, &compatibility(5), WorldIcSeconds(200))
            .is_err()
    );
}

#[test]
fn follow_does_not_spend_owner_reads_walking_a_long_withdrawn_suffix() {
    let first = entry();
    let mut index = Index::default();
    let mut latest = first.clone();
    for sequence in 1..=1024 {
        latest = first.clone();
        latest.filed = FiledRevisionId::new(sequence).unwrap();
        latest.sequence = sequence;
        latest.at = 100 + sequence as i64;
        insert(&mut index, latest.clone());
        if sequence > 1 {
            block(&mut index, latest.filed);
        }
    }
    assert_eq!(index.entries.len(), 1024);
    assert_eq!(
        index
            .eligible
            .get(&(first.line, compatibility(5).digest(), RatingClass::I))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        index
            .select(&line(&latest), &compatibility(5), WorldIcSeconds(2000))
            .unwrap(),
        first.filed
    );
    let mut crossed = line(&latest);
    crossed.head = first.filed;
    assert!(
        index
            .select(&crossed, &compatibility(5), WorldIcSeconds(2000))
            .is_err(),
        "a crossed current head never falls back"
    );
}

#[test]
fn follow_current_plan_pending_overrides_birth_and_new_completion_does_not_unwithdraw() {
    let first = entry();
    let mut index = Index::default();
    insert(&mut index, first.clone());
    let change = |compatibility: DefinitionDigest, rating| {
        Change(Kind::Certificate {
            filed: first.filed,
            identity: first.identity,
            birth_rating: first.birth_rating,
            compatibility,
            rating,
        })
    };
    let pending = index
        .prepare(&change(compatibility(5).digest(), None))
        .unwrap();
    assert_eq!(
        index
            .select(&line(&first), &compatibility(5), WorldIcSeconds(100))
            .unwrap(),
        first.filed,
        "preparing a delta does not publish it"
    );
    index.apply(pending);
    assert!(
        index
            .select(&line(&first), &compatibility(5), WorldIcSeconds(100))
            .is_err()
    );
    let complete = index
        .prepare(&change(
            compatibility(6).digest(),
            Some(Certificate {
                reference: first.birth_rating,
                minimum: RatingClass::I,
            }),
        ))
        .unwrap();
    index.apply(complete);
    assert_eq!(
        index
            .select(&line(&first), &compatibility(6), WorldIcSeconds(100))
            .unwrap(),
        first.filed
    );
    assert!(
        index
            .select(&line(&first), &compatibility(5), WorldIcSeconds(100))
            .is_err()
    );
    block(&mut index, first.filed);
    let complete = index
        .prepare(&change(
            compatibility(5).digest(),
            Some(Certificate {
                reference: first.birth_rating,
                minimum: RatingClass::I,
            }),
        ))
        .unwrap();
    index.apply(complete);
    assert!(
        index
            .select(&line(&first), &compatibility(5), WorldIcSeconds(100))
            .is_err()
    );
    assert!(
        index
            .select(&line(&first), &compatibility(6), WorldIcSeconds(100))
            .is_err()
    );
}

#[test]
fn follow_requires_contiguous_history_and_never_treats_a_broken_link_as_ineligible() {
    let first = entry();
    let mut index = Index::default();
    insert(&mut index, first.clone());
    for (sequence, at, previous) in [
        (3, 100, Some(first.filed)),
        (2, 99, Some(first.filed)),
        (2, 100, None),
        (2, 100, Some(FiledRevisionId::new(9).unwrap())),
    ] {
        let mut next = first.clone();
        next.filed = FiledRevisionId::new(2).unwrap();
        next.sequence = sequence;
        next.at = at;
        assert!(
            index
                .prepare(&Change(Kind::Filing {
                    entry: next,
                    previous
                }))
                .is_err()
        );
    }
    assert!(
        index
            .prepare(&Change(Kind::Block(FiledRevisionId::new(9).unwrap())))
            .is_err()
    );
    let mut crossed = first.clone();
    crossed.filed = FiledRevisionId::new(2).unwrap();
    crossed.sequence = 2;
    crossed.charter = CharterId::new(21).unwrap();
    assert!(
        index
            .prepare(&Change(Kind::Filing {
                entry: crossed,
                previous: Some(first.filed)
            }))
            .is_err()
    );
    let mut crossed_line = line(&first);
    crossed_line.charter = CharterId::new(21).unwrap();
    assert!(
        index
            .select(&crossed_line, &compatibility(5), WorldIcSeconds(100))
            .is_err()
    );
    assert_eq!(index.entries.len(), 1);
}

#[test]
fn follow_entry_and_current_certificate_inventory_keep_existing_family_bounds() {
    let first = entry();
    let mut index = Index::default();
    for sequence in 1..=crate::MAX_RECORDS as u64 {
        let mut next = first.clone();
        next.sequence = sequence;
        next.filed = FiledRevisionId::new(sequence).unwrap();
        insert(&mut index, next);
    }
    let mut extra = first.clone();
    extra.sequence = crate::MAX_RECORDS as u64 + 1;
    extra.filed = FiledRevisionId::new(extra.sequence).unwrap();
    assert!(
        index
            .prepare(&Change(Kind::Filing {
                entry: extra,
                previous: index.heads.get(&first.line).map(|h| h.filed)
            }))
            .is_err()
    );
    // Isolate arithmetic at the independent bounded head-family ceiling. This
    // is not a valid boot index and makes no authority/readiness claim.
    index.certificate_count = crate::MAX_RECORDS * 2;
    assert!(
        index
            .prepare(&Change(Kind::Certificate {
                filed: first.filed,
                identity: first.identity,
                birth_rating: first.birth_rating,
                compatibility: compatibility(9).digest(),
                rating: Some(Certificate {
                    reference: first.birth_rating,
                    minimum: RatingClass::I
                })
            }))
            .is_err()
    );
    let prepared = index
        .prepare(&Change(Kind::Certificate {
            filed: first.filed,
            identity: first.identity,
            birth_rating: first.birth_rating,
            compatibility: compatibility(5).digest(),
            rating: None,
        }))
        .unwrap();
    index.apply(prepared);
    assert_eq!(index.certificate_count, crate::MAX_RECORDS * 2);
}

#[test]
fn follow_class_ceiling_skips_a_long_expensive_suffix_without_scanning() {
    let first = entry();
    let mut index = Index::default();
    insert(&mut index, first.clone());
    let mut last = first.clone();
    for sequence in 2..=1024 {
        last = first.clone();
        last.filed = FiledRevisionId::new(sequence).unwrap();
        last.sequence = sequence;
        last.at = 100 + sequence as i64;
        last.certificates
            .get_mut(&compatibility(5).digest())
            .unwrap()
            .as_mut()
            .unwrap()
            .minimum = RatingClass::III;
        insert(&mut index, last.clone());
    }
    assert_eq!(index.eligible.len(), 2);
    let line = line(&last);
    for ceiling in [RatingClass::I, RatingClass::II] {
        assert_eq!(
            index
                .select_for_class(&line, &compatibility(5), WorldIcSeconds(2000), ceiling)
                .unwrap(),
            first.filed
        );
    }
    assert_eq!(
        index
            .select(&line, &compatibility(5), WorldIcSeconds(2000))
            .unwrap(),
        last.filed
    );
    block(&mut index, first.filed);
    assert!(
        index
            .select_for_class(
                &line,
                &compatibility(5),
                WorldIcSeconds(2000),
                RatingClass::II
            )
            .is_err()
    );
}

#[test]
fn follow_reconformance_moves_the_candidate_between_class_partitions_only_on_apply() {
    let first = entry();
    let mut index = Index::default();
    insert(&mut index, first.clone());
    let mut second = first.clone();
    second.filed = FiledRevisionId::new(2).unwrap();
    second.sequence = 2;
    second.at = 101;
    insert(&mut index, second.clone());
    let line = line(&second);
    let change = |minimum| {
        Change(Kind::Certificate {
            filed: second.filed,
            identity: second.identity,
            birth_rating: second.birth_rating,
            compatibility: compatibility(5).digest(),
            rating: Some(Certificate {
                reference: second.birth_rating,
                minimum,
            }),
        })
    };
    let prepared = index.prepare(&change(RatingClass::II)).unwrap();
    assert_eq!(
        index
            .select_for_class(
                &line,
                &compatibility(5),
                WorldIcSeconds(101),
                RatingClass::I
            )
            .unwrap(),
        second.filed
    );
    index.apply(prepared);
    assert_eq!(
        index
            .select_for_class(
                &line,
                &compatibility(5),
                WorldIcSeconds(101),
                RatingClass::I
            )
            .unwrap(),
        first.filed
    );
    assert_eq!(
        index
            .select_for_class(
                &line,
                &compatibility(5),
                WorldIcSeconds(101),
                RatingClass::II
            )
            .unwrap(),
        second.filed
    );
    let prepared = index.prepare(&change(RatingClass::I)).unwrap();
    index.apply(prepared);
    assert!(!index.eligible.contains_key(&(
        first.line,
        compatibility(5).digest(),
        RatingClass::II
    )));
    assert_eq!(
        index
            .select_for_class(
                &line,
                &compatibility(5),
                WorldIcSeconds(100),
                RatingClass::I
            )
            .unwrap(),
        first.filed
    );
    assert_eq!(
        index
            .select_for_class(
                &line,
                &compatibility(5),
                WorldIcSeconds(101),
                RatingClass::I
            )
            .unwrap(),
        second.filed
    );
}
