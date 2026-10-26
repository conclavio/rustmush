//! Real legacy command parsing; no provider truth or scope is fabricated.
use super::*;
#[test]
fn fact_condition_revision_preserves_legacy_none_and_full_u64() {
    let mut tw = boot_with(vec![Arc::new(StubFacts { table: vec![] })], vec![], vec![]);
    let r = run_rig(&mut tw, 1000);
    ok(&mut tw, r.johnson, "@contract/draft Versioned");
    let id = only_contract(&tw);
    for revision in [None, Some(0), Some(1), Some(u64::MAX)] {
        let suffix = revision
            .map(|n| format!(" revision={n}"))
            .unwrap_or_default();
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/condition {id}=fact test resolved 7{suffix}"),
        );
        let c = snap(&tw).contract(id).expect("actual draft");
        let Some(mush_economy::TypedCondition::Fact { fact }) = c.consent.conditions.last() else {
            panic!("actual fact condition")
        };
        assert_eq!(fact.revision, revision);
        assert_eq!(fact.id.as_str(), "7");
    }
}
#[test]
fn fact_condition_revision_refuses_noncanonical_overflow_and_extra_tokens_before_write() {
    let mut tw = boot_with(vec![Arc::new(StubFacts { table: vec![] })], vec![], vec![]);
    let r = run_rig(&mut tw, 1000);
    ok(&mut tw, r.johnson, "@contract/draft Versioned");
    let id = only_contract(&tw);
    let before = snap(&tw).contract(id).expect("draft").clone();
    for suffix in [
        "revision=",
        "revision=01",
        "revision=+1",
        "revision=-1",
        "revision=18446744073709551616",
        "revision=1 ignored",
        "revision=1 revision=2",
        "ignored",
        "1",
    ] {
        err(
            &mut tw,
            r.johnson,
            &format!("@contract/condition {id}=fact test resolved 7 {suffix}"),
        );
        assert!(
            snap(&tw).contract(id) == Some(&before),
            "invalid revision preserved exact draft"
        );
    }
}
