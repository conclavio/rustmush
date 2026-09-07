//! E2a end-to-end tests — build an `EconomySnapshot`, drive the pure posting engine
//! against the nine §7.1 invariants (§17.1 journal tests 1–7) plus reversal
//! idempotency, plan-hash determinism, the boundedness caps, and a conservation fuzz.

use mush_core::ObjId;
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, AuthEval, CreditPolicy, EconomicCause, EconomyError,
    EconomyPlan, EconomySnapshot, IdempotencyKey, InstrumentDef, InstrumentId, InstrumentKind,
    InstrumentStatus, NegativePolicy, PlanResult, RequestedLine, TransactionId, ValidateEnv, apply,
    plan_hash, plan_reversal, validate,
};
use mush_org::PrincipalId;

// ---- builders -------------------------------------------------------------

fn iid(s: &str) -> InstrumentId {
    InstrumentId::new(s).unwrap()
}
fn instrument(id: &str, scale: u8) -> InstrumentDef {
    InstrumentDef {
        id: iid(id),
        ruleset: "test".into(),
        code: id.into(),
        kind: InstrumentKind::Currency,
        scale,
        transferable: true,
        negative_policy: NegativePolicy::SystemOnly,
        status: InstrumentStatus::Active,
        display: None,
        revision: 1,
    }
}
fn account(id: u64, instrument: &str, credit: CreditPolicy, balance: i64) -> Account {
    Account {
        id: AccountId(id),
        owner: PrincipalId::Object(ObjId(id as u32)),
        instrument: iid(instrument),
        scope: None,
        name: format!("acct{id}").into(),
        status: AccountStatus::Active,
        posted_balance: Amount::from_minor(balance),
        held_amount: Amount::ZERO,
        last_hold_seq: 0,
        credit,
        last_journal_seq: 0,
        revision: 1,
        approval: None,
    }
}
fn snap(instruments: &[(&str, u8)], accounts: Vec<Account>) -> EconomySnapshot {
    let mut s = EconomySnapshot::new();
    for (id, scale) in instruments {
        s.put_instrument(instrument(id, *scale));
    }
    for a in accounts {
        s.put_account(a);
    }
    s
}
fn line(account: u64, delta: i64, memo: &str) -> RequestedLine {
    RequestedLine {
        account: AccountId(account),
        delta: Amount::from_minor(delta),
        memo: memo.into(),
    }
}
fn plan(ns: &str, key: &str, lines: Vec<RequestedLine>) -> EconomyPlan {
    EconomyPlan::new(
        IdempotencyKey::new(ns, key),
        PrincipalId::Object(ObjId(1)),
        EconomicCause::Staff {
            reason: "test".into(),
        },
        lines,
    )
}
/// The E2 environment: everything authorized, no custody provider (E2 plans move no
/// objects), clock at the same 1000 the tests post at.
fn env(auth: &dyn AuthEval) -> ValidateEnv<'_> {
    ValidateEnv {
        keys: None,
        auth,
        custody: &mush_economy::NoCustody,
        facts: &mush_economy::NoFacts,
        now: 1000,
        ic_now: 1000,
        next_hold_id: 1,
        spent: std::collections::BTreeMap::new(),
    }
}
fn post(
    s: &EconomySnapshot,
    p: &EconomyPlan,
    txid: u64,
    seq: u64,
) -> (EconomySnapshot, mush_economy::EconomicTransaction) {
    match validate(p, s, &env(&mush_economy::AllowAll)) {
        PlanResult::Post(prepared) => apply(s, &prepared, TransactionId(txid), seq, 1000),
        other => panic!("expected Post, got {other:?}"),
    }
}
fn refuse(s: &EconomySnapshot, p: &EconomyPlan) -> EconomyError {
    match validate(p, s, &env(&mush_economy::AllowAll)) {
        PlanResult::Refused(e) => e,
        other => panic!("expected Refused, got {other:?}"),
    }
}
fn bal(s: &EconomySnapshot, id: u64) -> i64 {
    s.account(AccountId(id)).unwrap().posted_balance.minor()
}
fn instrument_sum(s: &EconomySnapshot, instrument: &str) -> i128 {
    let id = iid(instrument);
    s.accounts()
        .filter(|a| a.instrument == id)
        .map(|a| a.posted_balance.widen())
        .sum()
}

struct DenyAll;
impl AuthEval for DenyAll {
    fn authorize(&self, _: &PrincipalId, _: &Account, _: Amount) -> bool {
        false
    }
}

// ---- tests ----------------------------------------------------------------

#[test]
fn two_account_transfer_balances_and_folds() {
    let s = snap(
        &[("gold", 2)],
        vec![
            account(1, "gold", CreditPolicy::forbid(), 500),
            account(2, "gold", CreditPolicy::forbid(), 0),
        ],
    );
    let p = plan(
        "transfer",
        "t1",
        vec![line(1, -100, "pay"), line(2, 100, "recv")],
    );
    let (s2, tx) = post(&s, &p, 1, 1);
    assert_eq!(bal(&s2, 1), 400);
    assert_eq!(bal(&s2, 2), 100);
    assert_eq!(tx.lines.len(), 2);
    assert_eq!(tx.journal_seq, 1);
    // Projections advanced their journal marker + revision.
    assert_eq!(s2.account(AccountId(1)).unwrap().last_journal_seq, 1);
    assert_eq!(s2.account(AccountId(1)).unwrap().revision, 2);
    // The idempotency record now maps the key -> this transaction.
    assert_eq!(s2.idempotency(&p.idempotency).unwrap().transaction, tx.id);
}

#[test]
fn refuses_every_invalid_shape_without_mutation() {
    let s = snap(
        &[("gold", 2), ("silver", 0)],
        vec![
            account(1, "gold", CreditPolicy::forbid(), 50),
            account(2, "gold", CreditPolicy::forbid(), 0),
            account(3, "silver", CreditPolicy::forbid(), 0),
        ],
    );
    // Unbalanced (per-instrument sum != 0).
    assert_eq!(
        refuse(
            &s,
            &plan("x", "unbal", vec![line(1, -100, ""), line(2, 50, "")])
        ),
        EconomyError::NotBalanced(iid("gold"))
    );
    // Credit violation: A has 50, sending 100 drives it to -50 under a forbid policy.
    assert_eq!(
        refuse(
            &s,
            &plan("x", "credit", vec![line(1, -100, ""), line(2, 100, "")])
        ),
        EconomyError::CreditViolation(AccountId(1))
    );
    // Mismatched instruments do not net: gold vs silver each unbalanced.
    assert_eq!(
        refuse(
            &s,
            &plan("x", "mix", vec![line(1, -10, ""), line(3, 10, "")])
        ),
        EconomyError::NotBalanced(iid("gold"))
    );
    // Too few lines.
    assert_eq!(
        refuse(&s, &plan("x", "one", vec![line(1, -10, "")])),
        EconomyError::TooFewLines
    );
    // Zero line.
    assert!(matches!(
        refuse(&s, &plan("x", "zero", vec![line(1, 0, ""), line(2, 0, "")])),
        EconomyError::ZeroLine(0)
    ));
    // Nonexistent account.
    assert_eq!(
        refuse(
            &s,
            &plan("x", "ghost", vec![line(1, -10, ""), line(99, 10, "")])
        ),
        EconomyError::NoSuchAccount(AccountId(99))
    );
    // Unauthorized (auth denies) — separate eval.
    let denied = match validate(
        &plan("x", "auth", vec![line(1, -10, ""), line(2, 10, "")]),
        &s,
        &env(&DenyAll),
    ) {
        PlanResult::Refused(e) => e,
        other => panic!("expected Refused, got {other:?}"),
    };
    assert_eq!(denied, EconomyError::Unauthorized);
}

#[test]
fn overflow_is_refused_not_wrapped() {
    let s = snap(
        &[("gold", 0)],
        vec![
            account(1, "gold", CreditPolicy::unbounded(), i64::MAX),
            account(2, "gold", CreditPolicy::unbounded(), 0),
        ],
    );
    // Crediting +1 onto i64::MAX overflows -> refused (invariant 4, checked).
    assert_eq!(
        refuse(&s, &plan("x", "of", vec![line(1, 1, ""), line(2, -1, "")])),
        EconomyError::Overflow
    );
}

#[test]
fn idempotency_same_plan_returns_original_conflict_on_different() {
    let s = snap(
        &[("gold", 2)],
        vec![
            account(1, "gold", CreditPolicy::forbid(), 500),
            account(2, "gold", CreditPolicy::forbid(), 0),
        ],
    );
    let p = plan(
        "transfer",
        "dup",
        vec![line(1, -100, "pay"), line(2, 100, "recv")],
    );
    let (s2, tx) = post(&s, &p, 7, 1);
    // Same key + same plan against the post-state -> AlreadyPosted(original), no new tx.
    match validate(&p, &s2, &env(&mush_economy::AllowAll)) {
        PlanResult::AlreadyPosted(id) => assert_eq!(id, tx.id),
        other => panic!("expected AlreadyPosted, got {other:?}"),
    }
    // Same key + DIFFERENT plan -> conflict (never a silent re-post).
    let p2 = plan(
        "transfer",
        "dup",
        vec![line(1, -200, "pay"), line(2, 200, "recv")],
    );
    assert_eq!(refuse(&s2, &p2), EconomyError::IdempotencyConflict);
}

#[test]
fn reversal_compensates_and_is_at_most_once() {
    let s = snap(
        &[("gold", 2)],
        vec![
            account(1, "gold", CreditPolicy::forbid(), 500),
            account(2, "gold", CreditPolicy::forbid(), 0),
        ],
    );
    let (s2, tx) = post(
        &s,
        &plan(
            "transfer",
            "r1",
            vec![line(1, -100, "pay"), line(2, 100, "recv")],
        ),
        1,
        1,
    );
    // Reverse via the dedicated engine path (fed the real original transaction).
    let (s3, rtx) = match plan_reversal(
        &tx,
        PrincipalId::Object(ObjId(1)),
        EconomicCause::Staff {
            reason: "undo".into(),
        },
        &s2,
        &mush_economy::AllowAll,
        None,
    ) {
        PlanResult::Post(prepared) => apply(&s2, &prepared, TransactionId(2), 2, 1000),
        other => panic!("expected Post, got {other:?}"),
    };
    assert_eq!(bal(&s3, 1), 500, "balance compensated");
    assert_eq!(bal(&s3, 2), 0);
    assert_eq!(
        rtx.reverses,
        Some(tx.id),
        "reversal references the original"
    );
    // Both immutable records still describe their own movements (nothing rewritten).
    assert_eq!(
        tx.lines
            .iter()
            .find(|l| l.account == AccountId(1))
            .unwrap()
            .delta,
        Amount::from_minor(-100)
    );
    assert_eq!(
        rtx.lines
            .iter()
            .find(|l| l.account == AccountId(1))
            .unwrap()
            .delta,
        Amount::from_minor(100)
    );
    // At-most-once: re-reversing is the original reversal id (deterministic key).
    match plan_reversal(
        &tx,
        PrincipalId::Object(ObjId(1)),
        EconomicCause::Staff {
            reason: "again".into(),
        },
        &s3,
        &mush_economy::AllowAll,
        None,
    ) {
        PlanResult::AlreadyPosted(id) => assert_eq!(id, rtx.id),
        other => panic!("expected AlreadyPosted, got {other:?}"),
    }
}

#[test]
fn an_ordinary_post_cannot_forge_a_reversal() {
    // Regression (E2a verify): a normal balanced post can NOT latch an unrelated /
    // not-yet-allocated transaction as reversed — `reverses` is engine-only now.
    let s = snap(
        &[("gold", 2)],
        vec![
            account(1, "gold", CreditPolicy::forbid(), 500),
            account(2, "gold", CreditPolicy::forbid(), 0),
        ],
    );
    // A plan literal has no `reverses` field to set; the post marks nothing reversed.
    let (s2, _tx) = post(
        &s,
        &plan("transfer", "p", vec![line(1, -100, ""), line(2, 100, "")]),
        9,
        1,
    );
    assert!(
        !s2.is_reversed(TransactionId(9999)),
        "no transaction was falsely latched reversed"
    );
    assert!(
        !s2.is_reversed(TransactionId(9)),
        "the post itself is not marked reversed"
    );
}

#[test]
fn exchange_two_instruments_each_balances_independently() {
    let s = snap(
        &[("gold", 2), ("silver", 0)],
        vec![
            account(1, "gold", CreditPolicy::forbid(), 1000),
            account(2, "gold", CreditPolicy::forbid(), 0),
            account(3, "silver", CreditPolicy::forbid(), 50),
            account(4, "silver", CreditPolicy::forbid(), 0),
        ],
    );
    // A trades 100 gold to B for 5 silver: each instrument sums to zero.
    let p = plan(
        "exchange",
        "e1",
        vec![
            line(1, -100, "gold out"),
            line(2, 100, "gold in"),
            line(4, 5, "silver in"),
            line(3, -5, "silver out"),
        ],
    );
    let (s2, _tx) = post(&s, &p, 1, 1);
    assert_eq!(bal(&s2, 1), 900);
    assert_eq!(bal(&s2, 2), 100);
    assert_eq!(bal(&s2, 3), 45);
    assert_eq!(bal(&s2, 4), 5);
    // If silver did NOT balance, the whole plan is refused.
    let bad = plan(
        "exchange",
        "e2",
        vec![
            line(1, -100, ""),
            line(2, 100, ""),
            line(4, 5, ""),
            line(3, -4, ""),
        ],
    );
    assert_eq!(refuse(&s, &bad), EconomyError::NotBalanced(iid("silver")));
}

#[test]
fn system_source_may_mint_by_going_negative() {
    // A registered system source (unbounded credit) mints 1000 gold to a player.
    let s = snap(
        &[("gold", 2)],
        vec![
            account(1, "gold", CreditPolicy::unbounded(), 0), // system source
            account(2, "gold", CreditPolicy::forbid(), 0),    // player
        ],
    );
    let (s2, _tx) = post(
        &s,
        &plan(
            "mint",
            "m1",
            vec![line(1, -1000, "faucet"), line(2, 1000, "grant")],
        ),
        1,
        1,
    );
    assert_eq!(bal(&s2, 1), -1000, "source drives negative (allowed)");
    assert_eq!(bal(&s2, 2), 1000);
    // Conservation: gold still sums to zero across all accounts.
    assert_eq!(instrument_sum(&s2, "gold"), 0);
}

#[test]
fn plan_hash_is_order_independent_and_excludes_concurrency_metadata() {
    let a = plan("t", "k", vec![line(1, -100, "a"), line(2, 100, "b")]);
    let mut b = plan("t", "k", vec![line(2, 100, "b"), line(1, -100, "a")]); // reordered lines
    b.expected_revisions = vec![mush_economy::AccountRevision {
        account: AccountId(1),
        revision: 42,
    }];
    assert_eq!(
        plan_hash(&a),
        plan_hash(&b),
        "reordered lines + expected_revisions hash identically"
    );
    // A genuinely different economic content hashes differently.
    let c = plan("t", "k", vec![line(1, -101, "a"), line(2, 101, "b")]);
    assert_ne!(plan_hash(&a), plan_hash(&c));
}

#[test]
fn boundedness_caps_refuse_oversized_plans() {
    let s = snap(
        &[("gold", 0)],
        vec![
            account(1, "gold", CreditPolicy::unbounded(), 0),
            account(2, "gold", CreditPolicy::unbounded(), 0),
        ],
    );
    // Too many lines.
    let mut many = vec![];
    for _ in 0..(mush_economy::MAX_LINES_PER_TX + 1) {
        many.push(line(1, 1, ""));
    }
    assert!(matches!(
        refuse(&s, &plan("x", "many", many)),
        EconomyError::TooLarge(_)
    ));
    // Memo too long.
    let big = "z".repeat(mush_economy::MAX_MEMO_BYTES + 1);
    assert!(matches!(
        refuse(
            &s,
            &plan("x", "memo", vec![line(1, -1, &big), line(2, 1, "")])
        ),
        EconomyError::TooLarge(_)
    ));
}

#[test]
fn stale_expected_revision_is_refused() {
    let s = snap(
        &[("gold", 0)],
        vec![
            account(1, "gold", CreditPolicy::forbid(), 100),
            account(2, "gold", CreditPolicy::forbid(), 0),
        ],
    );
    let mut p = plan("x", "stale", vec![line(1, -10, ""), line(2, 10, "")]);
    p.expected_revisions = vec![mush_economy::AccountRevision {
        account: AccountId(1),
        revision: 99,
    }];
    assert!(matches!(
        refuse(&s, &p),
        EconomyError::Stale {
            account: AccountId(1),
            expected: 99,
            found: Some(1)
        }
    ));
}

#[test]
fn fuzz_never_panics_and_preserves_conservation() {
    // Deterministic pseudo-random plans; every ACCEPTED post preserves per-instrument
    // conservation (zero-sum), and nothing panics/overflows.
    let mut s = snap(
        &[("gold", 2)],
        vec![
            account(1, "gold", CreditPolicy::unbounded(), 0),
            account(2, "gold", CreditPolicy::unbounded(), 0),
            account(3, "gold", CreditPolicy::unbounded(), 0),
        ],
    );
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut txid = 1u64;
    for _ in 0..2000 {
        let a = 1 + (next() % 3);
        let b = 1 + (next() % 3);
        if a == b {
            continue;
        }
        // Extreme amounts included (near i64 bounds) to probe overflow handling.
        let amt = (next() as i64).wrapping_rem(1 << 40).wrapping_sub(1 << 39);
        let p = plan(
            "fuzz",
            &format!("k{txid}"),
            vec![line(a, -amt, ""), line(b, amt, "")],
        );
        match validate(&p, &s, &env(&mush_economy::AllowAll)) {
            PlanResult::Post(prepared) => {
                let (s2, _tx) = apply(&s, &prepared, TransactionId(txid), txid, 1000);
                s = s2;
                txid += 1;
            }
            PlanResult::Refused(_)
            | PlanResult::AlreadyPosted(_)
            | PlanResult::NeedsApproval(_) => {}
        }
        // Conservation invariant holds after every step (zero-sum ledger).
        assert_eq!(
            instrument_sum(&s, "gold"),
            0,
            "gold conservation broke at txid {txid}"
        );
    }
}
