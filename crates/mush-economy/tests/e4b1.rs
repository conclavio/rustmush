//! E4b1 acceptance (08c §4.5, scenarios 13–15): **holds and obligations live**.
//!
//! The verify axis of this cut is *duplicate fires, restart, time jumps*, so the
//! rhythms here are:
//!
//! - **the same instant twice.** Every scheduled act has a deterministic idempotency
//!   key, so firing the sweep twice at one instant must post exactly once — the exit
//!   criterion's "duplicate scheduler events".
//! - **fold and reboot.** `fold_kv` + `reboot` replays the real boot: a projection
//!   that merely lags heals, a doc that disagrees with the journal at its own claimed
//!   sequence freezes the economy, a missing hold doc is rebuilt silently, and an
//!   unjournaled one is never loaded.
//! - **time moves wrongly.** Ten periods of downtime, an IC epoch jump, a starved
//!   post budget, and a clock that runs backwards each have one designed answer, and
//!   none of them is "charge the backlog".
//! - **nothing moved.** Every refusal is asserted against balances and the journal,
//!   not just against the message.
//!
//! Everything runs through the real `Engine` via `TestWorld`, like `e3b.rs`/`e3b2.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mush_core::{Audience, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId, ObjKind, World};
use mush_economy::{
    AccountId, AccountStatus, CustodyError, CustodyProvider, CustodyRef, EconomicCause,
    EconomyPlan, EconomySnapshot, EconomyStore, EscalationKind, EscalationSubject, ForgiveCause,
    HoldId, HoldStatus, IdempotencyKey, ObligationEvent, ObligationId, ObligationStatus,
    RequestedLine, SuspendReason,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "economy";

// ---- rig ------------------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events.iter().map(|(_, e)| text_of(e)).collect::<Vec<_>>().join("\n")
}
fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}
fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(has_error(&ev), "`{line}` should have been refused, got: {}", texts(&ev));
    texts(&ev)
}
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world().module_state::<EconomyStore>().expect("economy store")
}
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    store(tw).snapshot()
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").posted_balance.minor()
}
fn held(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").held_amount.minor()
}
fn journal_len(tw: &TestWorld) -> usize {
    store(tw).transactions().count()
}
fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut().create(ObjKind::Thing, name, GOD, Some(LIMBO)).expect("create")
}
fn account_of(tw: &TestWorld, owner: ObjId) -> AccountId {
    snap(tw)
        .accounts()
        .find(|a| a.owner == mush_org::PrincipalId::Object(owner))
        .expect("account of owner")
        .id
}

/// One sweep fire at wall-clock second `t`.
///
/// The core arms the tick at construction and re-arms it one period after every
/// fire, so a strictly increasing `t` (at least `SWEEP_PERIOD_SECS` apart) yields
/// exactly one fire per call — which is what makes "fire twice at the same instant"
/// an explicit act below rather than an accident.
fn sweep_at(tw: &mut TestWorld, t: i64) -> Vec<(Audience, OutputEvent)> {
    tw.set_now(t);
    tw.fire_due(t.saturating_mul(1000))
}

fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    merge(&mut kv, drain(tw));
    kv
}
fn drain(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != NS {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}
fn merge(base: &mut Vec<(SmolStr, serde_json::Value)>, extra: Vec<(SmolStr, serde_json::Value)>) {
    for (k, v) in extra {
        base.retain(|(bk, _)| *bk != k);
        base.push((k, v));
    }
}
/// Reboot the economy **in place**, so the world (and its objects) survive.
fn reboot(tw: &mut TestWorld, kv: Vec<(SmolStr, serde_json::Value)>) {
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData { kv, streams: HashMap::new() },
    )]));
}
fn patch(kv: &mut [(SmolStr, serde_json::Value)], key: &str, f: impl Fn(&mut serde_json::Value)) {
    let mut seen = false;
    for (k, v) in kv.iter_mut() {
        if k == key {
            f(v);
            seen = true;
        }
    }
    assert!(seen, "no such persisted key to patch: {key}");
}
fn value_of(kv: &[(SmolStr, serde_json::Value)], key: &str) -> serde_json::Value {
    kv.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_else(|| panic!("no {key}"))
}
fn drop_key(kv: &mut Vec<(SmolStr, serde_json::Value)>, key: &str) {
    let before = kv.len();
    kv.retain(|(k, _)| k != key);
    assert!(kv.len() < before, "no such persisted key to drop: {key}");
}

/// Two players, an instrument, a faucet, and funded personal wallets.
struct Rig {
    pat: ObjId,
    quinn: ObjId,
    pat_acct: AccountId,
    quinn_acct: AccountId,
    faucet: AccountId,
}

fn rig(tw: &mut TestWorld, pat_funds: i64) -> Rig {
    let pat = tw.spawn_player("Pat");
    let quinn = tw.spawn_player("Quinn");
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, &format!("@account/open #{}=gold", pat.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", quinn.0));
    let faucet = snap(tw)
        .accounts()
        .find(|a| matches!(a.owner, mush_org::PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let pat_acct = account_of(tw, pat);
    let quinn_acct = account_of(tw, quinn);
    if pat_funds > 0 {
        ok(tw, GOD, &format!("@econ/mint {faucet}={pat_acct} {pat_funds}"));
    }
    Rig { pat, quinn, pat_acct, quinn_acct, faucet }
}

fn only_hold(tw: &TestWorld) -> HoldId {
    let ids: Vec<HoldId> = snap(tw).holds().map(|h| h.id).collect();
    assert_eq!(ids.len(), 1, "expected exactly one hold, got {ids:?}");
    ids[0]
}
fn only_obligation(tw: &TestWorld) -> ObligationId {
    let ids: Vec<ObligationId> = snap(tw).obligations().map(|o| o.id).collect();
    assert_eq!(ids.len(), 1, "expected exactly one obligation, got {ids:?}");
    ids[0]
}
fn obligation(tw: &TestWorld, id: ObligationId) -> mush_economy::Obligation {
    snap(tw).obligation(id).expect("obligation").clone()
}

// ======================================================================
// Scenario 13 — the available balance, through the live seam
// ======================================================================

#[test]
fn two_holds_racing_one_available_balance_refuse_the_second() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);

    ok(&mut tw, GOD, &format!("@hold/place {}=60 deposit", r.pat_acct));
    assert_eq!(held(&tw, r.pat_acct), 60, "the reservation is not projected");
    assert_eq!(bal(&tw, r.pat_acct), 100, "a reservation moved money");

    let seen = err(&mut tw, GOD, &format!("@hold/place {}=60 second", r.pat_acct));
    assert!(seen.contains("insufficient available"), "wrong refusal: {seen}");
    assert_eq!(held(&tw, r.pat_acct), 60, "the refused placement still reserved");
    assert_eq!(snap(&tw).holds().count(), 1, "the refused placement minted a hold");
}

#[test]
fn a_payment_that_would_dip_into_held_funds_is_refused() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 deposit", r.pat_acct));
    let before = journal_len(&tw);

    let seen = err(&mut tw, r.pat, &format!("@econ/pay {}={} 50", r.pat_acct, r.quinn_acct));
    assert!(seen.contains("insufficient available"), "wrong refusal: {seen}");
    assert_eq!(bal(&tw, r.pat_acct), 100, "the refused payment moved money");
    assert_eq!(bal(&tw, r.quinn_acct), 0);
    assert_eq!(journal_len(&tw), before, "the refused payment journaled something");

    // Exactly the uncommitted part still spends.
    ok(&mut tw, r.pat, &format!("@econ/pay {}={} 40", r.pat_acct, r.quinn_acct));
    assert_eq!(bal(&tw, r.pat_acct), 60);
    assert_eq!(held(&tw, r.pat_acct), 60);
}

#[test]
fn the_beneficiary_captures_and_the_owners_own_wallet_rights_do_not() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 for=#{} escrow", r.pat_acct, r.quinn.0));
    let hold = only_hold(&tw);

    // The payer owns the account and may spend it — and still cannot spend the
    // reservation standing between them and their own money (the E4a fold, finding 1).
    err(&mut tw, r.pat, &format!("@hold/capture {hold}={} 60", r.pat_acct));
    assert_eq!(bal(&tw, r.quinn_acct), 0);

    ok(&mut tw, r.quinn, &format!("@hold/capture {hold}={} 40", r.quinn_acct));
    assert_eq!(bal(&tw, r.pat_acct), 60, "the capture debited the wrong amount");
    assert_eq!(bal(&tw, r.quinn_acct), 40);
    // The uncaptured remainder is released by the same effect.
    assert_eq!(held(&tw, r.pat_acct), 0, "the remainder was not released");
    assert!(matches!(
        snap(&tw).hold(hold).expect("hold").status,
        HoldStatus::Consumed { .. }
    ));
}

#[test]
fn reverse_is_refused_on_a_capture_carrying_transaction() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 for=#{} escrow", r.pat_acct, r.quinn.0));
    let hold = only_hold(&tw);
    ok(&mut tw, r.quinn, &format!("@hold/capture {hold}={} 60", r.quinn_acct));
    let txid = store(&tw)
        .transactions()
        .filter(|t| !t.hold_effects.is_empty())
        .map(|t| t.id)
        .max()
        .expect("the capture transaction");

    // Invariant 28: unwinding a capture by reversing its money would leave the hold
    // `Consumed` while the payee's credit vanished.
    let seen = err(&mut tw, GOD, &format!("@econ/reverse {txid}"));
    assert!(seen.contains("cannot be reversed"), "wrong refusal: {seen}");
    assert_eq!(bal(&tw, r.quinn_acct), 60, "the refused reversal moved money");
}

#[test]
fn account_close_is_refused_while_a_hold_or_an_obligation_lives() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=100 deposit", r.pat_acct));
    let hold = only_hold(&tw);

    // Invariant 29: a closed account takes no posting, so closing one that still
    // carries escrow would trap it forever.
    err(&mut tw, GOD, &format!("@account/close {}", r.pat_acct));
    assert_eq!(
        snap(&tw).account(r.pat_acct).expect("account").status,
        AccountStatus::Active,
        "the account closed over a live reservation"
    );

    ok(&mut tw, GOD, &format!("@hold/release {hold}"));
    ok(&mut tw, GOD, &format!("@econ/pay {}={} 100", r.pat_acct, r.quinn_acct));
    ok(&mut tw, GOD, &format!("@obligation/create {}={} 10 every=100", r.pat_acct, r.quinn_acct));
    let seen = err(&mut tw, GOD, &format!("@account/close {}", r.pat_acct));
    assert!(seen.contains("Obligation"), "wrong refusal: {seen}");

    let id = only_obligation(&tw);
    ok(&mut tw, GOD, &format!("@obligation/cancel {id}"));
    ok(&mut tw, GOD, &format!("@account/close {}", r.pat_acct));
}

// ======================================================================
// Scenario 14 — reservations across a reboot, and duplicate fires
// ======================================================================

#[test]
fn capture_release_and_expiry_all_survive_a_reboot_identically() {
    for act in ["capture", "release", "expire"] {
        let mut tw = boot();
        let r = rig(&mut tw, 300);
        ok(&mut tw, GOD, &format!("@hold/place {}=60 for=#{} expires=10 a", r.pat_acct, r.quinn.0));
        ok(&mut tw, GOD, &format!("@hold/place {}=90 for=#{} b", r.pat_acct, r.quinn.0));
        // The hold allocator's snapshot floor starts at 1 (E4a `next_hold_floor`), so the
        // first reservation in a fresh economy is #1.
        let first = HoldId(1);
        match act {
            "capture" => ok(&mut tw, r.quinn, &format!("@hold/capture {first}={} 25", r.quinn_acct)),
            "release" => ok(&mut tw, GOD, &format!("@hold/release {first}")),
            _ => {
                let ev = sweep_at(&mut tw, 100);
                assert!(!has_error(&ev), "sweep errored: {}", texts(&ev));
                String::new()
            }
        };
        let before_held = held(&tw, r.pat_acct);
        let before_bal = bal(&tw, r.pat_acct);
        let before_status = snap(&tw).hold(first).expect("hold").status.clone();

        let kv = fold_kv(&mut tw);
        reboot(&mut tw, kv);

        assert!(!store(&tw).is_frozen(), "{act}: a clean reboot froze the economy");
        assert_eq!(held(&tw, r.pat_acct), before_held, "{act}: held_amount changed across reboot");
        assert_eq!(bal(&tw, r.pat_acct), before_bal, "{act}: balance changed across reboot");
        assert_eq!(
            snap(&tw).hold(first).expect("hold").status,
            before_status,
            "{act}: hold state changed across reboot"
        );
        assert_eq!(held(&tw, r.pat_acct), 90, "{act}: the second reservation was lost");
    }
}

#[test]
fn firing_the_sweep_twice_at_one_instant_expires_a_hold_once() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 expires=10 a", r.pat_acct));
    let hold = only_hold(&tw);
    let before = journal_len(&tw);

    tw.set_now(100);
    tw.fire_due(100_000);
    // The SAME instant, a second scheduler delivery — the exit criterion's duplicate
    // scheduler event. The deterministic key `hold/{id}/expire` resolves it.
    tw.fire_due(130_000);

    assert_eq!(journal_len(&tw), before + 1, "the duplicate fire posted a second expiry");
    assert_eq!(snap(&tw).hold(hold).expect("hold").status, HoldStatus::Expired);
    assert_eq!(held(&tw, r.pat_acct), 0, "expiry did not free the reservation");
    assert_eq!(bal(&tw, r.pat_acct), 100, "expiry moved money");
}

#[test]
fn a_hold_on_a_manual_clock_is_never_aged_out_by_the_sweep() {
    // A `Manual` deadline resolves to "never passed" for the sweep: a manually-clocked
    // reservation is released by hand, never by a scheduler reading someone else's
    // clock. Built through the store because no command offers a manual deadline.
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 expires=ic:1000000 a", r.pat_acct));
    let hold = only_hold(&tw);
    sweep_at(&mut tw, 1_000);
    assert_eq!(
        snap(&tw).hold(hold).expect("hold").status,
        HoldStatus::Held,
        "a deadline in the future was aged out"
    );
}

// ======================================================================
// The boot quartet for `hold:{id}` (§4.2)
// ======================================================================

#[test]
fn a_lagging_hold_doc_heals_silently() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 a", r.pat_acct));
    let hold = only_hold(&tw);
    let mut kv = fold_kv(&mut tw);
    let stale = value_of(&kv, "hold:1");
    ok(&mut tw, GOD, &format!("@hold/release {hold}"));
    merge(&mut kv, drain(&mut tw));
    // The release is in the journal; its doc write dropped.
    patch(&mut kv, "hold:1", |v| *v = stale.clone());
    reboot(&mut tw, kv);

    assert!(!store(&tw).is_frozen(), "a lagging projection froze the economy");
    assert_eq!(
        snap(&tw).hold(hold).expect("hold").status,
        HoldStatus::Released,
        "the lagging doc was not healed to the journal"
    );
    assert_eq!(held(&tw, r.pat_acct), 0);
}

#[test]
fn a_drifted_hold_doc_freezes_the_economy() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 a", r.pat_acct));
    let mut kv = fold_kv(&mut tw);
    // The doc claims the sequence it really appears at, and disagrees there.
    patch(&mut kv, "hold:1", |v| {
        v["amount"] = serde_json::json!(11);
    });
    reboot(&mut tw, kv);

    assert!(store(&tw).is_frozen(), "drift at the claimed sequence did not freeze");
    // Healed to the journal, and every account frozen with it.
    assert_eq!(snap(&tw).hold(HoldId(1)).expect("hold").amount.minor(), 60);
    assert_eq!(snap(&tw).account(r.pat_acct).expect("account").status, AccountStatus::Frozen);
}

#[test]
fn a_missing_hold_doc_is_rebuilt_silently_and_an_unjournaled_one_is_not_loaded() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 a", r.pat_acct));
    let mut kv = fold_kv(&mut tw);
    drop_key(&mut kv, "hold:1");
    // A hand-written doc with no journal history is not a reservation.
    let ghost = value_of(&kv, "account:1"); // any well-formed json; it must NOT parse as a hold
    let _ = ghost;
    kv.push((
        SmolStr::new("hold:77"),
        serde_json::json!({
            "id": 77,
            "account": r.pat_acct.0,
            "amount": 500,
            "beneficiary": null,
            "purpose": { "Manual": { "reason": "forged" } },
            "placed_by": { "Object": GOD.0 },
            "holder": { "namespace": "forged", "key": "1" },
            "expires": null,
            "status": "Held",
            "last_journal_seq": 1,
            "revision": 1
        }),
    ));
    reboot(&mut tw, kv);

    assert!(!store(&tw).is_frozen(), "a lost projection froze the economy");
    assert_eq!(
        snap(&tw).hold(HoldId(1)).expect("the hold was not rebuilt").amount.minor(),
        60,
        "a missing hold doc must rebuild from the journal"
    );
    assert!(snap(&tw).hold(HoldId(77)).is_none(), "an unjournaled hold doc was loaded");
    assert_eq!(held(&tw, r.pat_acct), 60, "the forged doc reached held_amount");
}

#[test]
fn held_amount_drift_is_detected_at_its_own_sequence_anchor() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 a", r.pat_acct));
    let acct_key = format!("account:{}", r.pat_acct.0);

    // A placement is EFFECT-ONLY: it never moves `last_journal_seq`, so the drift
    // check has to be asked at `last_hold_seq` (D11). Disagree there.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &acct_key, |v| {
        v["held_amount"] = serde_json::json!(5);
    });
    reboot(&mut tw, kv);
    assert!(store(&tw).is_frozen(), "held_amount drift at last_hold_seq did not freeze");
    assert_eq!(held(&tw, r.pat_acct), 60, "the drifted projection was not healed");
}

#[test]
fn a_lagging_held_amount_heals_without_freezing() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    ok(&mut tw, GOD, &format!("@hold/place {}=60 a", r.pat_acct));
    let acct_key = format!("account:{}", r.pat_acct.0);
    let mut kv = fold_kv(&mut tw);
    let stale = value_of(&kv, &acct_key);
    ok(&mut tw, GOD, &format!("@hold/place {}=20 b", r.pat_acct));
    merge(&mut kv, drain(&mut tw));
    // The second placement is journaled; the account doc write dropped.
    patch(&mut kv, &acct_key, |v| *v = stale.clone());
    reboot(&mut tw, kv);

    assert!(!store(&tw).is_frozen(), "a benign lag froze the economy");
    assert_eq!(held(&tw, r.pat_acct), 80, "the lagging projection was not healed forward");
}

#[test]
fn the_hold_allocator_floor_survives_garbage_collection() {
    let mut tw = boot();
    let r = rig(&mut tw, 300);
    ok(&mut tw, GOD, &format!("@hold/place {}=10 a", r.pat_acct));
    ok(&mut tw, GOD, &format!("@hold/place {}=10 b", r.pat_acct));
    // Retire the HIGHEST id, so a floor taken from the surviving docs would drop back.
    ok(&mut tw, GOD, &format!("@hold/release {}", HoldId(2)));

    // Past the retention window, the sweep collects it.
    sweep_at(&mut tw, 8 * 86_400);
    assert!(snap(&tw).hold(HoldId(2)).is_none(), "a terminal hold was not collected");
    assert_eq!(store(&tw).peek_next_hold(), 3, "the allocator floor dropped after GC");

    // And the floor is journal-derived, so it survives the reboot that loses the doc.
    let kv = fold_kv(&mut tw);
    assert!(!kv.iter().any(|(k, _)| k == "hold:2"), "the collected doc was not deleted");
    reboot(&mut tw, kv);
    assert_eq!(store(&tw).peek_next_hold(), 3, "the reboot floor came from the docs");

    ok(&mut tw, GOD, &format!("@hold/place {}=10 c", r.pat_acct));
    assert!(snap(&tw).hold(HoldId(3)).is_some(), "the new reservation re-minted a collected id");
}

// ======================================================================
// Hidden parties (§4.5)
// ======================================================================

#[test]
fn a_third_party_cannot_tell_a_hold_or_an_obligation_from_a_nonexistent_one() {
    let mut tw = boot();
    let r = rig(&mut tw, 100);
    let nosy = tw.spawn_player("Nosy");
    ok(&mut tw, GOD, &format!("@hold/place {}=60 for=#{} escrow", r.pat_acct, r.quinn.0));
    let hold = only_hold(&tw);
    ok(&mut tw, GOD, &format!("@obligation/create {}={} 10 every=100", r.pat_acct, r.quinn_acct));
    let obligation_id = only_obligation(&tw);

    let a = err(&mut tw, nosy, &format!("@hold/show {hold}"));
    let b = err(&mut tw, nosy, "@hold/show 4242");
    assert_eq!(a, b, "an existing hold reads differently from a nonexistent one");
    let a = err(&mut tw, nosy, &format!("@hold/release {hold}"));
    assert_eq!(a, b, "a refusal to release disclosed the hold");
    let a = err(&mut tw, nosy, &format!("@obligation/show {obligation_id}"));
    let b = err(&mut tw, nosy, "@obligation/show 4242");
    assert_eq!(a, b, "an existing obligation reads differently from a nonexistent one");

    // The list is empty rather than a count of what is hidden.
    let seen = ok(&mut tw, nosy, "@hold/list");
    assert!(seen.contains("No holds."), "the list leaked: {seen}");
    let seen = ok(&mut tw, nosy, "@obligation/list");
    assert!(seen.contains("No obligations."), "the list leaked: {seen}");

    // `@hold/list account=` follows E3's list discipline: identical text for a
    // nonexistent account and one the actor may not view.
    let a = err(&mut tw, nosy, &format!("@hold/list account={}", r.pat_acct));
    let b = err(&mut tw, nosy, "@hold/list account=4242");
    assert_eq!(a, b, "an account the actor may not view read differently from a missing one");
    // ...and empty, not an error, for a viewable account with no reservations.
    let seen = ok(&mut tw, r.quinn, &format!("@hold/list account={}", r.quinn_acct));
    assert!(seen.contains("No holds."), "a viewable empty account errored: {seen}");

    // The parties themselves see their own records.
    ok(&mut tw, r.quinn, &format!("@hold/show {hold}"));
    ok(&mut tw, r.pat, &format!("@obligation/show {obligation_id}"));
}

// ======================================================================
// Scenario 15 — obligations on a clock
// ======================================================================

/// `pat` owes `quinn` `amount` every 100 seconds, first due at t+100.
fn owe(tw: &mut TestWorld, r: &Rig, amount: i64, extra: &str) -> ObligationId {
    ok(
        tw,
        GOD,
        &format!(
            "@obligation/create {}={} {amount} every=100 start=100 {extra}",
            r.pat_acct, r.quinn_acct
        ),
    );
    only_obligation(tw)
}

#[test]
fn an_obligation_settles_on_schedule_and_says_nothing() {
    let mut tw = boot();
    let r = rig(&mut tw, 1000);
    let id = owe(&mut tw, &r, 100, "");

    let ev = sweep_at(&mut tw, 100);
    assert_eq!(bal(&tw, r.pat_acct), 900, "the first period did not settle");
    assert_eq!(bal(&tw, r.quinn_acct), 100);
    assert!(
        !texts(&ev).contains("Obligation"),
        "routine settlement spoke: {}",
        texts(&ev)
    );

    let ev = sweep_at(&mut tw, 200);
    assert_eq!(bal(&tw, r.pat_acct), 800, "the second period did not settle");
    assert!(!texts(&ev).contains("Obligation"), "routine settlement spoke");

    let o = obligation(&tw, id);
    assert_eq!(o.settled_total, 2);
    assert_eq!(o.periods_settled, 2);
    assert_eq!(o.status, ObligationStatus::Active);
}

#[test]
fn firing_twice_at_one_instant_posts_once_per_period() {
    let mut tw = boot();
    let r = rig(&mut tw, 1000);
    let id = owe(&mut tw, &r, 100, "");
    let before = journal_len(&tw);

    tw.set_now(100);
    tw.fire_due(100_000);
    tw.fire_due(130_000);

    assert_eq!(journal_len(&tw), before + 1, "the duplicate fire posted twice");
    assert_eq!(bal(&tw, r.pat_acct), 900, "the duplicate fire charged twice");
    assert_eq!(obligation(&tw, id).periods_settled, 1);
}

#[test]
fn a_short_account_falls_into_grace_with_a_notice_and_then_delinquency() {
    let mut tw = boot();
    let r = rig(&mut tw, 150);
    let id = owe(&mut tw, &r, 100, "");

    sweep_at(&mut tw, 100);
    assert_eq!(bal(&tw, r.pat_acct), 50, "the first period did not settle");

    let ev = sweep_at(&mut tw, 200);
    assert!(matches!(obligation(&tw, id).status, ObligationStatus::Grace { missed: 1, .. }));
    let said = texts(&ev);
    assert!(said.contains("missed a period"), "grace was silent: {said}");
    assert!(!said.contains("durab"), "a sweep notice claimed durability: {said}");
    assert_eq!(bal(&tw, r.pat_acct), 50, "a missed period moved money");

    let ev = sweep_at(&mut tw, 300);
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Delinquent);
    assert!(texts(&ev).contains("delinquent"), "delinquency was silent");

    // Out of the due pool: later fires attempt nothing at all.
    let before = journal_len(&tw);
    sweep_at(&mut tw, 400);
    sweep_at(&mut tw, 500);
    assert_eq!(journal_len(&tw), before, "a delinquent obligation kept being attempted");
    let o = obligation(&tw, id);
    assert_eq!(o.settled_total, 1);
    assert_eq!(o.missed_total, 2);
}

#[test]
fn reinstate_returns_a_delinquent_obligation_re_anchored_after_now() {
    let mut tw = boot();
    let r = rig(&mut tw, 150);
    let id = owe(&mut tw, &r, 100, "");
    sweep_at(&mut tw, 100);
    sweep_at(&mut tw, 200);
    sweep_at(&mut tw, 300);
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Delinquent);

    // The debtor may not reinstate their own debt; the creditor (or staff) may.
    err(&mut tw, r.pat, &format!("@obligation/reinstate {id}"));
    ok(&mut tw, GOD, &format!("@econ/mint {}={} 1000", r.faucet, r.pat_acct));
    ok(&mut tw, r.quinn, &format!("@obligation/reinstate {id}"));

    let o = obligation(&tw, id);
    assert_eq!(o.status, ObligationStatus::Active);
    assert!(o.schedule.next_due > 300, "the schedule was not re-anchored past now");
    let closed = o.periods_settled;

    // Reinstating charges nothing for the time it spent delinquent.
    let before = bal(&tw, r.pat_acct);
    sweep_at(&mut tw, 350);
    assert_eq!(bal(&tw, r.pat_acct), before, "reinstating charged the delinquent gap");
    sweep_at(&mut tw, 450);
    assert_eq!(bal(&tw, r.pat_acct), before - 100, "the next period did not settle");
    assert_eq!(obligation(&tw, id).periods_settled, closed + 1);
}

#[test]
fn ten_periods_of_downtime_settle_three_and_forgive_seven() {
    let mut tw = boot();
    let r = rig(&mut tw, 10_000);
    let id = owe(&mut tw, &r, 100, "catchup=3");

    // No sweep has run at all, and 1000 seconds have passed: ten periods are due.
    let ev = sweep_at(&mut tw, 1_000);
    let o = obligation(&tw, id);
    assert_eq!(o.settled_total, 3, "more than the catch-up cap was charged");
    assert_eq!(o.forgiven_total, 7, "the excess was not forgiven");
    assert_eq!(bal(&tw, r.pat_acct), 10_000 - 300, "a downtime backlog was charged");
    assert_eq!(o.status, ObligationStatus::Active, "forgiveness is not a wait state");
    assert_eq!(o.periods_settled, 10, "the period index did not advance past the forgiven span");

    let forgiven: Vec<&ObligationEvent> = o
        .history
        .iter()
        .filter(|e| matches!(e, ObligationEvent::Forgiven { .. }))
        .collect();
    assert_eq!(forgiven.len(), 1, "the excess was forgiven in pieces: {forgiven:?}");
    assert!(matches!(
        forgiven[0],
        ObligationEvent::Forgiven { periods: 7, cause: ForgiveCause::Downtime, .. }
    ));
    let said = texts(&ev);
    assert_eq!(said.matches("were forgiven").count(), 2, "not exactly one notice per party: {said}");
    assert!(said.contains("downtime"), "the notice did not carry the cause: {said}");

    // Back on schedule, one period at a time.
    let before = bal(&tw, r.pat_acct);
    sweep_at(&mut tw, 1_100);
    assert_eq!(bal(&tw, r.pat_acct), before - 100, "the schedule did not resume");
}

#[test]
fn an_ic_epoch_jump_is_forgiven_as_a_time_jump() {
    let mut tw = boot();
    let r = rig(&mut tw, 10_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 100 every=ic:100 start=100 catchup=3",
            r.pat_acct, r.quinn_acct
        ),
    );
    let id = only_obligation(&tw);
    // One ordinary fire, so the classifier has a recent stamp to compare against.
    sweep_at(&mut tw, 100);
    assert_eq!(obligation(&tw, id).settled_total, 1);

    // The IC clock jumps far beyond what the ratio allows — an epoch change.
    tw.world_mut()
        .set_game_clock(mush_core::GameClock { ic_ratio: 1, epoch_offset: 100_000 });
    sweep_at(&mut tw, 130);

    let o = obligation(&tw, id);
    assert!(o.forgiven_total > 0, "the jump was not forgiven");
    assert!(
        o.history.iter().any(|e| matches!(
            e,
            ObligationEvent::Forgiven { cause: ForgiveCause::TimeJump, .. }
        )),
        "the jump was misclassified: {:?}",
        o.history
    );
    assert!(
        bal(&tw, r.pat_acct) >= 10_000 - 400,
        "an IC jump charged a backlog: {}",
        bal(&tw, r.pat_acct)
    );
}

#[test]
fn a_backwards_clock_charges_nothing() {
    let mut tw = boot();
    tw.set_now(1_000);
    let r = rig(&mut tw, 1_000);
    let id = owe(&mut tw, &r, 100, "");
    let before = bal(&tw, r.pat_acct);
    let entries = journal_len(&tw);

    // The wall clock is rewound behind the schedule's own anchor.
    tw.set_now(500);
    tw.fire_due(1_030_000);

    assert_eq!(bal(&tw, r.pat_acct), before, "a rewound clock charged something");
    assert_eq!(journal_len(&tw), entries, "a rewound clock journaled something");
    assert_eq!(obligation(&tw, id).periods_settled, 0);
}

#[test]
fn repeated_overload_forgiveness_escalates_the_second_time() {
    let mut tw = boot();
    let r = rig(&mut tw, 10_000);
    // A first fire, so the next ones are close enough together that a backlog cannot
    // be downtime: sweeps DID run, this obligation simply never got budget.
    sweep_at(&mut tw, 100);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 1 every=1 start=1 catchup=3",
            r.pat_acct, r.quinn_acct
        ),
    );
    let id = only_obligation(&tw);

    sweep_at(&mut tw, 130);
    let o = obligation(&tw, id);
    assert!(
        o.history.iter().any(|e| matches!(
            e,
            ObligationEvent::Forgiven { cause: ForgiveCause::Overload, .. }
        )),
        "the first starvation was misclassified: {:?}",
        o.history
    );
    assert_eq!(o.status, ObligationStatus::Active, "one overload escalated");
    assert_eq!(store(&tw).escalations().count(), 0, "one overload paged a human");

    sweep_at(&mut tw, 160);
    let o = obligation(&tw, id);
    assert!(
        matches!(o.status, ObligationStatus::Suspended { reason: SuspendReason::Escalated(_) }),
        "a repeated overload did not suspend the subject: {:?}",
        o.status
    );
    let alert = store(&tw).escalations().next().expect("an escalation").clone();
    assert_eq!(alert.kind, EscalationKind::RepeatedOverload);
    assert_eq!(alert.subject, EscalationSubject::Obligation(id));
    assert!(!alert.recommended.is_empty(), "the alert carries no recommended command");

    // ...and only that obligation is stopped.
    assert!(!store(&tw).is_frozen(), "a local problem stopped the world");
}

#[test]
fn a_restart_between_the_post_and_the_doc_write_converges() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let id = owe(&mut tw, &r, 100, "");
    let mut kv = fold_kv(&mut tw);
    let stale = value_of(&kv, "obligation:0");

    sweep_at(&mut tw, 100);
    assert_eq!(bal(&tw, r.pat_acct), 900, "the period did not settle");
    merge(&mut kv, drain(&mut tw));
    // The money is journaled; the obligation's own doc write dropped, so the record
    // still believes period 0 is open.
    patch(&mut kv, "obligation:0", |v| *v = stale.clone());
    reboot(&mut tw, kv);
    assert!(!store(&tw).is_frozen(), "a lagging container froze the economy");
    assert_eq!(bal(&tw, r.pat_acct), 900, "the journal lost the settlement");
    assert_eq!(obligation(&tw, id).periods_settled, 0, "the doc was not the stale one");

    // The next fire re-derives the SAME deterministic key, resolves to
    // `AlreadyPosted`, and closes the period without charging it twice.
    sweep_at(&mut tw, 130);
    assert_eq!(bal(&tw, r.pat_acct), 900, "the restart charged the period twice");
    assert_eq!(obligation(&tw, id).periods_settled, 1, "the period never closed");
    assert_eq!(obligation(&tw, id).settled_total, 1);
}

#[test]
fn a_manual_clock_obligation_moves_only_when_someone_steps_it() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 100 every=manual:10 start=10",
            r.pat_acct, r.quinn_acct
        ),
    );
    let id = only_obligation(&tw);

    sweep_at(&mut tw, 1_000);
    assert_eq!(bal(&tw, r.pat_acct), 1_000, "the sweep advanced a manual clock");

    // The debtor cannot charge themselves less by refusing to step it, and cannot
    // step it either: advancing a manual clock is the act of charging.
    err(&mut tw, r.pat, &format!("@obligation/advance {id}"));
    ok(&mut tw, r.quinn, &format!("@obligation/advance {id}"));
    sweep_at(&mut tw, 1_100);
    assert_eq!(bal(&tw, r.pat_acct), 900, "the stepped period did not settle");
    assert_eq!(obligation(&tw, id).settled_total, 1);
}

// ======================================================================
// The escalation ladder (§4.6)
// ======================================================================

/// Freeze the whole economy the way a recovery boot does: the latch is durable in
/// `meta`, and only `@econ/unfreeze` clears it.
fn freeze(tw: &mut TestWorld, kv: &mut [(SmolStr, serde_json::Value)]) {
    patch(kv, "meta", |v| {
        v["frozen"] = serde_json::json!(true);
    });
    reboot(tw, kv.to_vec());
    assert!(store(tw).is_frozen(), "the durable freeze latch did not survive the reboot");
}

#[test]
fn the_whole_economy_freeze_stops_the_sweep_and_opens_one_record() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let id = owe(&mut tw, &r, 100, "");
    let mut kv = fold_kv(&mut tw);
    freeze(&mut tw, &mut kv);
    let entries = journal_len(&tw);

    sweep_at(&mut tw, 100);
    assert_eq!(journal_len(&tw), entries, "the sweep worked while frozen");
    assert_eq!(obligation(&tw, id).periods_settled, 0, "the sweep settled while frozen");
    assert_eq!(store(&tw).last_sweep(), (0, 0), "a sweep that did nothing stamped anyway");

    let alerts: Vec<_> = store(&tw).escalations().cloned().collect();
    assert_eq!(alerts.len(), 1, "expected exactly one record: {alerts:?}");
    assert_eq!(alerts[0].kind, EscalationKind::EconomyFrozen);
    assert_eq!(alerts[0].subject, EscalationSubject::Economy);
    // §4.6: the freeze itself is the level-3 action; the record it opens is an
    // ordinary level-2 page carrying the diagnosis and the recovery command.
    assert_eq!(alerts[0].level, 2);
    assert!(
        alerts[0].recommended.iter().any(|c| c.contains("@econ/unfreeze")),
        "the record does not carry the recovery command: {:?}",
        alerts[0].recommended
    );

    // Later fires do not mint a second record for the same open subject.
    sweep_at(&mut tw, 200);
    sweep_at(&mut tw, 300);
    assert_eq!(store(&tw).escalations().count(), 1, "the freeze minted an alert storm");

    let seen = ok(&mut tw, GOD, "@econ/alerts");
    assert!(seen.contains("economy frozen"), "the alert did not list: {seen}");
    // Acknowledging the diagnosis is not the same act as clearing the freeze.
    let seen = ok(&mut tw, GOD, "@econ/ack 0");
    assert!(seen.contains("does not clear"), "ack implied it unfroze: {seen}");
    assert!(store(&tw).is_frozen(), "@econ/ack cleared the whole-economy freeze");
}

#[test]
fn the_escalation_allocator_floor_survives_a_lost_leaf_doc() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let _ = owe(&mut tw, &r, 100, "");
    let mut kv = fold_kv(&mut tw);
    freeze(&mut tw, &mut kv);
    sweep_at(&mut tw, 100);
    assert!(store(&tw).escalation(mush_economy::EscalationId(0)).is_some());

    // The leaf doc is lost — but its id was minted, and an id that was minted must
    // never be minted again onto a different alert, or an acknowledgement would
    // resume the wrong subject.
    merge(&mut kv, drain(&mut tw));
    drop_key(&mut kv, "escalation:0");
    reboot(&mut tw, kv);
    assert!(store(&tw).escalation(mush_economy::EscalationId(0)).is_none());

    sweep_at(&mut tw, 200);
    let ids: Vec<u64> = store(&tw).escalations().map(|e| e.id.0).collect();
    assert_eq!(ids, vec![1], "a collected escalation id was re-minted: {ids:?}");
}

#[test]
fn a_frozen_creditor_account_suspends_only_that_obligation_and_pages_with_the_fix() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    // A second, healthy obligation, so "stop only the subject" is a real assertion.
    let landlord = thing(&mut tw, "escrow desk");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", landlord.0));
    let landlord_acct = account_of(&tw, landlord);
    ok(
        &mut tw,
        GOD,
        &format!("@obligation/create {}={} 50 every=100 start=100", r.pat_acct, landlord_acct),
    );
    let broken = ObligationId(0);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 10 every=100 start=100",
            r.pat_acct, r.quinn_acct
        ),
    );
    let healthy = ObligationId(1);

    // The creditor object is destroyed: the backstop freezes its account, and the
    // creditor principal itself is gone.
    ok(&mut tw, GOD, &format!("@destroy #{}", landlord.0));
    assert_eq!(
        snap(&tw).account(landlord_acct).expect("account").status,
        AccountStatus::Frozen
    );

    let ev = sweep_at(&mut tw, 100);
    let o = obligation(&tw, broken);
    let alert_id = match &o.status {
        ObligationStatus::Suspended { reason: SuspendReason::Escalated(id) } => *id,
        other => panic!("the broken obligation was not suspended: {other:?}"),
    };
    assert!(!store(&tw).is_frozen(), "one broken creditor stopped the world");
    assert_eq!(
        obligation(&tw, healthy).settled_total,
        1,
        "the healthy obligation was stopped too"
    );
    assert!(texts(&ev).contains("suspended"), "the parties were not told: {}", texts(&ev));

    let alert = store(&tw).escalation(alert_id).expect("the alert").clone();
    assert_eq!(alert.kind, EscalationKind::SettlementRefused);
    assert_eq!(alert.subject, EscalationSubject::Obligation(broken));
    assert!(
        alert.recommended.iter().any(|c| c.contains("@obligation/repoint") && c.contains("credit")),
        "the alert does not name the fix: {:?}",
        alert.recommended
    );
    let listed = ok(&mut tw, GOD, &format!("@econ/alerts {alert_id}"));
    assert!(listed.contains("@obligation/repoint"), "the alert did not print its fix: {listed}");

    // The sweep does not keep attempting it (it is out of the due pool)...
    let entries = journal_len(&tw);
    sweep_at(&mut tw, 200);
    assert_eq!(
        journal_len(&tw),
        entries + 1,
        "only the healthy obligation should have posted"
    );
    assert_eq!(store(&tw).escalations().count(), 1, "the suspended obligation kept paging");

    // ...and the recommended command plus an acknowledgement puts it back.
    ok(&mut tw, GOD, &format!("@obligation/repoint {broken}=credit {}", r.quinn_acct));
    ok(&mut tw, GOD, &format!("@econ/ack {alert_id}"));
    assert_eq!(obligation(&tw, broken).status, ObligationStatus::Active);
    let before = bal(&tw, r.pat_acct);
    sweep_at(&mut tw, 300);
    assert!(bal(&tw, r.pat_acct) < before, "the resumed obligation did not settle");
    // The periods it missed while suspended are forgiven, never charged.
    let o = obligation(&tw, broken);
    assert!(
        o.settled_total <= 1 + u32::from(o.schedule.maximum_catch_up),
        "the resumed obligation charged past its catch-up cap: {o:?}"
    );
}

#[test]
fn boot_suspends_an_obligation_whose_party_is_gone_and_pages_with_the_fix() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let landlord = thing(&mut tw, "escrow desk");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", landlord.0));
    let landlord_acct = account_of(&tw, landlord);
    ok(
        &mut tw,
        GOD,
        &format!("@obligation/create {}={} 50 every=100 start=100", r.pat_acct, landlord_acct),
    );
    let id = only_obligation(&tw);
    ok(&mut tw, GOD, &format!("@destroy #{}", landlord.0));

    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);

    assert!(!store(&tw).is_frozen(), "boot froze the world over one broken record");
    let o = obligation(&tw, id);
    assert!(
        matches!(o.status, ObligationStatus::Suspended { reason: SuspendReason::Escalated(_) }),
        "boot did not suspend an unsettleable obligation: {:?}",
        o.status
    );
    // Never deleted: the record is the only thing that remembers the debt.
    assert!(snap(&tw).obligation(id).is_some());
    let alert = store(&tw).escalations().next().expect("boot opened no alert").clone();
    assert_eq!(alert.kind, EscalationKind::BootReconciliation);
    assert!(alert.recommended.iter().any(|c| c.contains("@obligation/repoint")));
}

// ======================================================================
// §12 — the K install hazard: the install is infallible, and the assertion is
// what fires if anything ever re-enters the posting path after the world moves
// ======================================================================

/// A custody provider that re-enters [`mush_economy::post`] from inside the world
/// move — the exact interleaving K's two-phase split left latent (§12).
#[derive(Debug, Default)]
struct Reenter(Mutex<Option<(AccountId, AccountId)>>);

impl CustodyProvider for Reenter {
    fn provider(&self) -> &str {
        "reenter"
    }
    fn handles(&self, backing: &CustodyRef) -> bool {
        matches!(backing, CustodyRef::Container(_))
    }
    fn claims(&self, _w: &World, _obj: ObjId) -> bool {
        false
    }
    fn backing_valid(&self, _w: &World, _backing: &CustodyRef) -> Option<bool> {
        Some(true)
    }
    fn contents(&self, _w: &World, _backing: &CustodyRef) -> Option<Vec<ObjId>> {
        Some(Vec::new())
    }
    fn holds(&self, _w: &World, _backing: &CustodyRef, _obj: ObjId) -> Option<bool> {
        Some(true)
    }
    fn move_into(
        &self,
        ctx: &mut mush_core::Ctx<'_>,
        _actor: ObjId,
        _obj: ObjId,
        _backing: &CustodyRef,
    ) -> Result<(), CustodyError> {
        let accounts = *self.0.lock().expect("lock");
        if let Some((from, to)) = accounts {
            let plan = EconomyPlan::new(
                IdempotencyKey::new(SmolStr::new_static("reenter"), SmolStr::new_static("1")),
                mush_org::PrincipalId::Object(GOD),
                EconomicCause::Staff { reason: "re-entry".into() },
                vec![
                    RequestedLine {
                        account: from,
                        delta: mush_economy::Amount::from_minor(-1),
                        memo: SmolStr::default(),
                    },
                    RequestedLine {
                        account: to,
                        delta: mush_economy::Amount::from_minor(1),
                        memo: SmolStr::default(),
                    },
                ],
            );
            let _ = mush_economy::post(ctx, &plan);
        }
        Ok(())
    }
    fn move_out(
        &self,
        _ctx: &mut mush_core::Ctx<'_>,
        _actor: ObjId,
        _obj: ObjId,
        _backing: &CustodyRef,
        _to: ObjId,
    ) -> Result<(), CustodyError> {
        Ok(())
    }
}

#[test]
#[should_panic(expected = "moved between prepare and install")]
fn a_provider_that_re_enters_post_trips_the_install_invariant() {
    let stub = Arc::new(Reenter::default());
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![stub.clone()])),
    ]);
    let r = rig(&mut tw, 1_000);
    *stub.0.lock().expect("lock") = Some((r.faucet, r.quinn_acct));
    let chest = thing(&mut tw, "chest");
    let widget = thing(&mut tw, "widget");
    ok(&mut tw, GOD, &format!("@vault/create box=container:#{}", chest.0));
    ok(&mut tw, GOD, &format!("@asset/title #{}=#{}", widget.0, r.pat.0));

    // The custody move is the last fallible act of a post; the install after it
    // cannot fail, so an allocator that moved underneath it is a PROGRAMMING error
    // and must surface as one — not as a `JournalUnavailable` desync after the world
    // has already moved.
    tw.run(
        GOD,
        &format!(
            "@econ/pay {}={} 10 asset=#{} to_vault=0",
            r.pat_acct, r.quinn_acct, widget.0
        ),
    );
}

#[test]
fn an_ordinary_custody_post_still_installs_cleanly() {
    // The other half of the assertion's reachability argument: the shipped providers
    // do not re-enter, so the very same path posts normally.
    let stub = Arc::new(Reenter::default());
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![stub.clone()])),
    ]);
    let r = rig(&mut tw, 1_000);
    let chest = thing(&mut tw, "chest");
    let widget = thing(&mut tw, "widget");
    ok(&mut tw, GOD, &format!("@vault/create box=container:#{}", chest.0));
    ok(&mut tw, GOD, &format!("@asset/title #{}=#{}", widget.0, r.pat.0));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@econ/pay {}={} 10 asset=#{} to_vault=0",
            r.pat_acct, r.quinn_acct, widget.0
        ),
    );
    assert_eq!(bal(&tw, r.quinn_acct), 10);
}

// ======================================================================
// The sweep's own discipline
// ======================================================================

#[test]
fn the_sweep_is_a_no_op_on_an_empty_economy_and_stamps_itself() {
    let mut tw = boot();
    let ev = sweep_at(&mut tw, 100);
    assert!(!has_error(&ev), "an empty sweep complained: {}", texts(&ev));
    assert_eq!(store(&tw).last_sweep(), (100, 100), "the sweep did not stamp itself");
    // And the stamps are durable, so the classifier's evidence survives a restart.
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert_eq!(store(&tw).last_sweep(), (100, 100), "the sweep stamps were not persisted");
}

#[test]
fn a_suspended_obligation_leaves_the_due_pool_and_resume_forgives_the_gap() {
    let mut tw = boot();
    let r = rig(&mut tw, 1_000);
    let id = owe(&mut tw, &r, 100, "");
    sweep_at(&mut tw, 100);
    assert_eq!(bal(&tw, r.pat_acct), 900);

    ok(&mut tw, GOD, &format!("@obligation/suspend {id}=audit"));
    let entries = journal_len(&tw);
    sweep_at(&mut tw, 200);
    sweep_at(&mut tw, 300);
    sweep_at(&mut tw, 400);
    assert_eq!(journal_len(&tw), entries, "a suspended obligation kept being attempted");

    ok(&mut tw, GOD, &format!("@obligation/resume {id}"));
    let ev = sweep_at(&mut tw, 500);
    // Three periods went by while it was suspended; with the default catch-up of 3
    // they are attempted, not charged as a lump — and anything past the cap is
    // forgiven, never charged (doc 08 §2).
    let o = obligation(&tw, id);
    assert!(o.settled_total <= 4, "resuming charged the whole suspended gap");
    assert!(!has_error(&ev), "the resumed sweep errored: {}", texts(&ev));
}
