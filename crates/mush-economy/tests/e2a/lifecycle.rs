//! Doc 43 cut TS.5: the generated lifecycle over the pure engine.
//!
//! A proptest state machine: random histories of payments, holds (place,
//! capture, release, expire), titles (title, transfer, assign, unassign) and
//! liens (encumber, release), each step validated against the snapshot the
//! previous ones left and applied when accepted. After EVERY step the engine's
//! invariants are checked as the model:
//!
//! - conservation: the instrument's balances sum to what they started at
//!   (invariants 1–4, 10);
//! - the held projection: an account's `held_amount` is the sum of its `Held`
//!   holds, never negative (E4a §3.2);
//! - available ≥ the floor on every forbid-credit account (invariant 23);
//! - at most `MAX_ACTIVE_HOLDS_PER_ACCOUNT` live holds (invariant 26);
//! - a hold leaves `Held` once and never comes back; a capture spends at most
//!   what was held (invariant 24);
//! - an `Expire` never posts for a hold without a deadline (invariant 24);
//! - an assignee is never the title owner, and a title that moved carries no
//!   assignment, no maintenance payer and none of its old liens (invariants
//!   12, 13);
//! - a posted plan, submitted again, is `AlreadyPosted` its first transaction.
//!
//! It replaces `fuzz_never_panics_and_preserves_conservation`: one instrument,
//! two-line transfers and one fixed seed with no shrinking. A failure here
//! shrinks to its shortest history; keep a minimised one as a named test.

use std::collections::BTreeMap;

use mush_core::ObjId;
use mush_economy::{
    Amount, AssetOp, CreditPolicy, EconomicTransaction, EconomyPlan, EconomySnapshot, Encumbrance,
    HoldId, HoldOp, HoldPurpose, HoldStatus, MAX_ACTIVE_HOLDS_PER_ACCOUNT, PlanResult,
    TransactionId, apply, validate,
};
use mush_org::PrincipalId;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

use super::{account, env, iid, line, plan, snap};

/// Three people with forbid-credit accounts, and a system account that may go
/// negative (the faucet the money starts from).
const PEOPLE: u8 = 3;
const START: i64 = 1_000;
const OBJECTS: u8 = 4;

fn who(n: u8) -> PrincipalId {
    PrincipalId::Object(ObjId(u32::from(n)))
}

fn object(n: u8) -> ObjId {
    ObjId(100 + u32::from(n))
}

fn start() -> EconomySnapshot {
    let mut accounts = vec![account(
        0,
        "gold",
        CreditPolicy::unbounded(),
        -START * i64::from(PEOPLE),
    )];
    for n in 1..=PEOPLE {
        accounts.push(account(u64::from(n), "gold", CreditPolicy::forbid(), START));
    }
    snap(&[("gold", 2)], accounts)
}

/// One step of a history. Indices are taken modulo what exists, so every
/// generated history is well-formed and shrinks to a shorter one.
#[derive(Clone, Debug)]
enum Step {
    Pay {
        from: u8,
        to: u8,
        amount: i64,
    },
    Place {
        account: u8,
        amount: i64,
    },
    Capture {
        hold: u8,
        amount: i64,
        to: u8,
    },
    ReleaseHold {
        hold: u8,
    },
    ExpireHold {
        hold: u8,
    },
    Title {
        object: u8,
        owner: u8,
    },
    Transfer {
        object: u8,
        to: u8,
        release_liens: bool,
    },
    Assign {
        object: u8,
        to: u8,
    },
    Unassign {
        object: u8,
    },
    Lien {
        object: u8,
        creditor: u8,
        amount: i64,
    },
    ReleaseLien {
        object: u8,
        which: u8,
    },
    Resubmit {
        which: u8,
    },
}

/// Amounts weighted to the edges: zero, one, the whole balance, past it.
fn amount() -> impl Strategy<Value = i64> {
    prop_oneof![
        Just(0),
        Just(1),
        Just(START),
        Just(START + 1),
        1i64..=START,
        -5i64..0,
    ]
}

fn person() -> impl Strategy<Value = u8> {
    1..=PEOPLE
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (person(), person(), amount()).prop_map(|(from, to, amount)| Step::Pay {
            from,
            to,
            amount
        }),
        (person(), amount()).prop_map(|(account, amount)| Step::Place { account, amount }),
        (any::<u8>(), amount(), person()).prop_map(|(hold, amount, to)| Step::Capture {
            hold,
            amount,
            to
        }),
        any::<u8>().prop_map(|hold| Step::ReleaseHold { hold }),
        any::<u8>().prop_map(|hold| Step::ExpireHold { hold }),
        (0..OBJECTS, person()).prop_map(|(object, owner)| Step::Title { object, owner }),
        (0..OBJECTS, person(), any::<bool>()).prop_map(|(object, to, release_liens)| {
            Step::Transfer {
                object,
                to,
                release_liens,
            }
        }),
        (0..OBJECTS, person()).prop_map(|(object, to)| Step::Assign { object, to }),
        (0..OBJECTS).prop_map(|object| Step::Unassign { object }),
        (0..OBJECTS, person(), amount()).prop_map(|(object, creditor, amount)| Step::Lien {
            object,
            creditor,
            amount
        }),
        (0..OBJECTS, any::<u8>()).prop_map(|(object, which)| Step::ReleaseLien { object, which }),
        any::<u8>().prop_map(|which| Step::Resubmit { which }),
    ]
}

/// The hold a step names: the `n`th hold ever placed, or one that does not
/// exist when there are none.
fn nth_hold(s: &EconomySnapshot, n: u8) -> HoldId {
    let ids: Vec<HoldId> = s.holds().map(|h| h.id).collect();
    if ids.is_empty() {
        HoldId(1)
    } else {
        ids[usize::from(n) % ids.len()]
    }
}

/// The plan a step submits, keyed `k<i>`; `None` for a resubmission.
fn plan_for(s: &EconomySnapshot, i: usize, step: &Step) -> Option<EconomyPlan> {
    let key = format!("k{i}");
    let mut p = plan("life", &key, Vec::new());
    match *step {
        Step::Pay { from, to, amount } => {
            p.requested_lines = vec![
                line(u64::from(from), -amount, ""),
                line(u64::from(to), amount, ""),
            ];
        }
        Step::Place { account, amount } => {
            p.hold_ops = vec![HoldOp::Place {
                account: mush_economy::AccountId(u64::from(account)),
                amount: Amount::from_minor(amount),
                beneficiary: None,
                purpose: HoldPurpose::Manual {
                    reason: "life".into(),
                },
                expires: None,
            }];
        }
        Step::Capture { hold, amount, to } => {
            let id = nth_hold(s, hold);
            let from = s.hold(id).map_or(1, |h| h.account.0);
            p.requested_lines = vec![line(from, -amount, ""), line(u64::from(to), amount, "")];
            p.hold_ops = vec![HoldOp::Capture {
                hold: id,
                amount: Amount::from_minor(amount),
            }];
        }
        Step::ReleaseHold { hold } => {
            p.hold_ops = vec![HoldOp::Release {
                hold: nth_hold(s, hold),
            }]
        }
        Step::ExpireHold { hold } => {
            p.hold_ops = vec![HoldOp::Expire {
                hold: nth_hold(s, hold),
            }]
        }
        Step::Title { object: o, owner } => {
            p.asset_ops = vec![AssetOp::Title {
                object: object(o),
                owner: who(owner),
                scope: None,
                catalog: None,
            }];
        }
        Step::Transfer {
            object: o,
            to,
            release_liens,
        } => {
            if release_liens && let Some(asset) = s.asset(object(o)) {
                for encumbrance in &asset.encumbrances {
                    p.asset_ops.push(AssetOp::Release {
                        object: object(o),
                        encumbrance: encumbrance.clone(),
                    });
                }
            }
            p.asset_ops.push(AssetOp::TransferTitle {
                object: object(o),
                to: who(to),
                to_scope: None,
            });
        }
        Step::Assign { object: o, to } => {
            p.asset_ops = vec![AssetOp::Assign {
                object: object(o),
                to: who(to),
                purpose: "use".into(),
                valid_until: None,
            }];
        }
        Step::Unassign { object: o } => p.asset_ops = vec![AssetOp::Unassign { object: object(o) }],
        Step::Lien {
            object: o,
            creditor,
            amount,
        } => {
            p.asset_ops = vec![AssetOp::Encumber {
                object: object(o),
                encumbrance: Encumbrance::Lien {
                    creditor: who(creditor),
                    amount: Amount::from_minor(amount),
                    instrument: iid("gold"),
                },
            }];
        }
        Step::ReleaseLien { object: o, which } => {
            let held = s
                .asset(object(o))
                .map(|a| a.encumbrances.clone())
                .unwrap_or_default();
            let encumbrance = if held.is_empty() {
                Encumbrance::Lien {
                    creditor: who(1),
                    amount: Amount::from_minor(1),
                    instrument: iid("gold"),
                }
            } else {
                held[usize::from(which) % held.len()].clone()
            };
            p.asset_ops = vec![AssetOp::Release {
                object: object(o),
                encumbrance,
            }];
        }
        Step::Resubmit { .. } => return None,
    }
    Some(p)
}

/// What a history produced: the final snapshot, and which step kinds posted.
struct Outcome {
    posted: BTreeMap<&'static str, usize>,
}

fn kind(step: &Step) -> &'static str {
    match step {
        Step::Pay { .. } => "pay",
        Step::Place { .. } => "place",
        Step::Capture { .. } => "capture",
        Step::ReleaseHold { .. } => "release_hold",
        Step::ExpireHold { .. } => "expire_hold",
        Step::Title { .. } => "title",
        Step::Transfer { .. } => "transfer",
        Step::Assign { .. } => "assign",
        Step::Unassign { .. } => "unassign",
        Step::Lien { .. } => "lien",
        Step::ReleaseLien { .. } => "release_lien",
        Step::Resubmit { .. } => "resubmit",
    }
}

/// Run a history, checking the model after every step. `Err` names the step
/// and the invariant that broke.
fn run(history: &[Step]) -> Result<Outcome, String> {
    let mut s = start();
    let total: i64 = s.accounts().map(|a| a.posted_balance.minor()).sum();
    let mut posted_plans: Vec<(EconomyPlan, TransactionId)> = Vec::new();
    let mut posted = BTreeMap::new();
    let mut txid = 1u64;
    for (i, step) in history.iter().enumerate() {
        let before = s.clone();
        let (p, resubmitted) = match (plan_for(&s, i, step), step) {
            (Some(p), _) => (p, None),
            (None, Step::Resubmit { which }) if !posted_plans.is_empty() => {
                let (p, tx) = posted_plans[usize::from(*which) % posted_plans.len()].clone();
                (p, Some(tx))
            }
            (None, _) => continue,
        };
        let mut e = env(&mush_economy::AllowAll);
        e.next_hold_id = s.next_hold_floor();
        let result = validate(&p, &s, &e);
        if let Some(tx) = resubmitted {
            match result {
                PlanResult::AlreadyPosted(id) if id == tx => {}
                other => {
                    return Err(format!(
                        "step {i}: a resubmitted plan answered {other:?}, not AlreadyPosted({tx:?})"
                    ));
                }
            }
            *posted.entry("resubmit").or_insert(0) += 1;
            continue;
        }
        let PlanResult::Post(prepared) = result else {
            continue;
        };
        if matches!(step, Step::ExpireHold { .. }) {
            return Err(format!(
                "step {i}: an Expire posted for a hold with no deadline"
            ));
        }
        let (next, tx): (EconomySnapshot, EconomicTransaction) =
            apply(&s, &prepared, TransactionId(txid), txid, 1000);
        s = next;
        *posted.entry(kind(step)).or_insert(0) += 1;
        posted_plans.push((p.clone(), TransactionId(txid)));
        txid += 1;
        let _ = tx;
        check(&before, &s, total).map_err(|e| format!("step {i} ({step:?}): {e}"))?;
        // The same plan again is a replay of the first post.
        let mut e = env(&mush_economy::AllowAll);
        e.next_hold_id = s.next_hold_floor();
        match validate(&p, &s, &e) {
            PlanResult::AlreadyPosted(id) if id == TransactionId(txid - 1) => {}
            other => return Err(format!("step {i}: resubmitted at once answered {other:?}")),
        }
    }
    Ok(Outcome { posted })
}

/// The model: every invariant over the post-state, and every transition
/// invariant over (before, after).
fn check(before: &EconomySnapshot, s: &EconomySnapshot, total: i64) -> Result<(), String> {
    let sum: i64 = s.accounts().map(|a| a.posted_balance.minor()).sum();
    if sum != total {
        return Err(format!(
            "conservation: the balances sum to {sum}, not {total}"
        ));
    }
    for a in s.accounts() {
        let held: i64 = s
            .holds_of(a.id)
            .filter(|h| h.status == HoldStatus::Held)
            .map(|h| h.amount.minor())
            .sum();
        if a.held_amount.minor() != held || held < 0 {
            return Err(format!(
                "held projection: account {:?} says {} held, its holds sum to {held}",
                a.id,
                a.held_amount.minor()
            ));
        }
        if a.credit == CreditPolicy::forbid() && a.posted_balance.minor() - held < 0 {
            return Err(format!(
                "invariant 23: account {:?} has {} posted and {held} held",
                a.id,
                a.posted_balance.minor()
            ));
        }
        if s.active_holds_on(a.id) > MAX_ACTIVE_HOLDS_PER_ACCOUNT {
            return Err(format!(
                "invariant 26: account {:?} carries too many holds",
                a.id
            ));
        }
    }
    for h in before.holds() {
        let Some(now) = s.hold(h.id) else {
            return Err(format!("hold {:?} vanished", h.id));
        };
        if h.status != HoldStatus::Held && now.status != h.status {
            return Err(format!(
                "invariant 24: hold {:?} left {:?} for {:?}",
                h.id, h.status, now.status
            ));
        }
        if let HoldStatus::Consumed { amount, .. } = now.status
            && amount.minor() > h.amount.minor()
        {
            return Err(format!(
                "invariant 24: hold {:?} captured {} of {}",
                h.id,
                amount.minor(),
                h.amount.minor()
            ));
        }
    }
    for asset in s.assets() {
        if let Some(assignment) = &asset.assignment
            && assignment.assigned_to == asset.title_owner
        {
            return Err(format!(
                "invariant 13: {:?} is assigned to its own owner",
                asset.object
            ));
        }
        if let Some(old) = before.asset(asset.object)
            && old.title_owner != asset.title_owner
        {
            if asset.assignment.is_some() || asset.maintenance_account.is_some() {
                return Err(format!(
                    "invariant 13: {:?} moved title and kept its assignment",
                    asset.object
                ));
            }
            if old
                .encumbrances
                .iter()
                .any(|e| asset.encumbrances.contains(e))
            {
                return Err(format!(
                    "invariant 12: {:?} moved title with a lien on it",
                    asset.object
                ));
            }
        }
    }
    Ok(())
}

fn history() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(step(), 1..48)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Every generated history keeps every invariant after every step.
    #[test]
    fn every_history_keeps_the_invariants(history in history()) {
        if let Err(e) = run(&history) {
            prop_assert!(false, "{}", e);
        }
    }
}

/// The generator reaches every kind of post, so the property above is not
/// vacuous: across a fixed sample of histories, each step kind posted at
/// least once (an `Expire` never may, and is checked never to).
#[test]
fn the_histories_reach_every_kind_of_post() {
    let mut runner = TestRunner::deterministic();
    let strategy = history();
    let mut posted: BTreeMap<&'static str, usize> = BTreeMap::new();
    for _ in 0..256 {
        let h = strategy.new_tree(&mut runner).expect("draws").current();
        let outcome = run(&h).expect("the property holds");
        for (k, n) in outcome.posted {
            *posted.entry(k).or_insert(0) += n;
        }
    }
    for k in [
        "pay",
        "place",
        "capture",
        "release_hold",
        "title",
        "transfer",
        "assign",
        "unassign",
        "lien",
        "release_lien",
        "resubmit",
    ] {
        assert!(
            posted.get(k).copied().unwrap_or(0) > 0,
            "no {k} ever posted: {posted:?}"
        );
    }
    assert_eq!(posted.get("expire_hold"), None);
}
